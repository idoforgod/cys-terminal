"""release-postprocess.py 의 **수신 규율**(timeout · 유계 재시도) 밀폐 unittest.

★왜 이 파일이 필요한가 (2026-08-27 MF-C · 3라운드 적대 리뷰 지적)
  후처리기는 GitHub 릴리스에서 전 자산을 받아 zip·SHA256SUMS 를 만드는 **발행 경로의 손절차**다.
  그 수신부(`api()`·`download()`)는 `urllib.request.urlopen(req)` 를 **timeout 없이·재시도 없이**
  불렀다. 절단 축은 이미 fail-closed 였지만(1단계 크기 대조 → `::error::` + return 1) 남은 둘:
    · 일시 5xx·429 가 잡히지 않고 **트레이스백으로 죽는다** — main() 의 except 는 릴리스 조회
      한 곳(404 폴백)만 감싼다. CI 직후 GitHub 블립 한 번이면 후처리 전체가 죽는다.
    · 멈춘 연결은 timeout 부재로 **무기한 대기**한다(소켓 기본값 None) = 릴리스 절차 hang.

★이 파일이 지키는 것 — 세 축을 **동시에** 못박는다:
    ⓐ 모든 호출에 timeout 이 실제로 전달된다(문자열 핀 + 실행 핀)
    ⓑ 재시도는 **유계**이고 **429·5xx·연결 오류/타임아웃만** 탄다(4xx 는 확정 답 = 1회)
    ⓒ ★음성 대조 — **크기 대조 fail-closed 는 그대로 살아 있다**. 재시도가 그 게이트를 무디게
      만들면(크기 불일치를 재시도로 눙치면) 이 수리는 개악과 구별되지 않는다.

★설계 — 네트워크는 한 줄도 쓰지 않는다. `api(_opener=…)`·`download(_opener=…)` 의 테스트 전용
  키워드로 페이크 opener 를 주입하고, 대기는 `_sleep` 으로 삼킨다(레인 관례:
  test_verify_release_remote.py · test_deploy_homepage_fetch.py 의 페이크 curl 과 동형).
  경로는 전부 tempfile — 개인 경로·실 홈 디렉터리 금지.

사용: python3 scripts/tests/test_release_postprocess_http.py
"""

import contextlib
import importlib.util
import io
import os
import re
import socket
import tempfile
import unittest
import urllib.error

# 하이픈 파일명은 import 문으로 못 부른다 — importlib 로 직접 적재한다(레인 관례).
_RP_PATH = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "release-postprocess.py")
_spec = importlib.util.spec_from_file_location("release_postprocess_http", _RP_PATH)
rp = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(rp)

V = "0.14.19"
TAG = "v" + V
ASSET_URL = "https://api.github.com/repos/x/releases/assets/1"
BODY = b"CYSDMG" + b"\xa5" * 2048


@contextlib.contextmanager
def captured():
    out, err = io.StringIO(), io.StringIO()
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
        yield out, err


class NoSleep(object):
    def __init__(self):
        self.slept = []

    def __call__(self, sec):
        self.slept.append(sec)


class FakeResp(object):
    """urlopen 반환값 흉내 — 컨텍스트 매니저 + 청크 read."""

    def __init__(self, body=b""):
        self.body = body
        self.pos = 0

    def read(self, n=None):
        if n is None:
            chunk, self.pos = self.body[self.pos:], len(self.body)
            return chunk
        chunk = self.body[self.pos:self.pos + n]
        self.pos += len(chunk)
        return chunk

    def __enter__(self):
        return self

    def __exit__(self, *a):
        return False


def http_error(code):
    return urllib.error.HTTPError(ASSET_URL, code, "synthetic", {}, None)


class FakeOpener(object):
    """계획된 응답/예외를 순서대로 돌려준다. 큐가 마르면 마지막 것을 반복
    (=재시도해도 계속 같은 답을 주는 원격). 호출마다 (url, timeout) 을 기록한다."""

    def __init__(self, plan):
        self.plan = list(plan)
        self.calls = []

    def __call__(self, req, timeout=None):
        self.calls.append((getattr(req, "full_url", str(req)), timeout))
        item = self.plan.pop(0) if len(self.plan) > 1 else self.plan[0]
        if isinstance(item, BaseException):
            raise item
        if callable(item):
            raise item()
        return FakeResp(item)


# ──────────────────────────────────────────────────────────────────────────────
# ⓐ timeout — 실제로 전달되는가
# ──────────────────────────────────────────────────────────────────────────────
class TimeoutContractTests(unittest.TestCase):

    def test_01_api_passes_timeout(self):
        op = FakeOpener([b'{"ok":1}'])
        self.assertEqual(rp.api("/x", "T", _opener=op), {"ok": 1})
        self.assertEqual(op.calls[0][1], rp.API_TIMEOUT)

    def test_02_download_passes_timeout(self):
        op = FakeOpener([BODY])
        with tempfile.TemporaryDirectory() as td:
            dest = os.path.join(td, "a.bin")
            rp.download(ASSET_URL, dest, "T", _opener=op)
            with open(dest, "rb") as fh:
                self.assertEqual(fh.read(), BODY)
        self.assertEqual(op.calls[0][1], rp.DOWNLOAD_TIMEOUT)

    def test_03_no_bare_urlopen_left_in_source(self):
        """★문자열 핀 — `urlopen(...)` 호출은 **전부** timeout 을 실어야 한다.
        누가 새 호출을 추가하면서 timeout 을 빠뜨리면 여기서 걸린다(무기한 대기 = 절차 hang)."""
        with open(_RP_PATH, encoding="utf-8") as fh:
            lines = fh.read().splitlines()
        calls = []
        for n, line in enumerate(lines, 1):
            code = line.split("#", 1)[0]        # 주석은 뺀다(구판 코드를 인용한 설명 주석이 있다)
            calls += [(n, a) for a in re.findall(r"urlopen\(([^)]*)\)", code)]
        self.assertTrue(calls, "urlopen 호출을 하나도 못 찾았다 — 핀이 헛돌고 있다")
        for n, c in calls:
            self.assertIn("timeout", c,
                          "timeout 없는 urlopen 호출 (%s:%d): urlopen(%s)"
                          % (os.path.basename(_RP_PATH), n, c))

    def test_04_timeouts_are_positive_and_ordered(self):
        for name in ("API_TIMEOUT", "DOWNLOAD_TIMEOUT", "UPLOAD_TIMEOUT"):
            self.assertGreater(getattr(rp, name), 0, name)
        # 자산 본문은 API 응답보다 오래 걸린다 — 상한이 뒤집히면 큰 DMG 가 상시 죽는다.
        self.assertGreater(rp.DOWNLOAD_TIMEOUT, rp.API_TIMEOUT)


# ──────────────────────────────────────────────────────────────────────────────
# ⓑ 유계 재시도 — 무엇을 재시도하고 무엇을 재시도하지 않는가
# ──────────────────────────────────────────────────────────────────────────────
class BoundedRetryTests(unittest.TestCase):

    def test_05_transient_classifier_boundary(self):
        for s in (429, 500, 502, 503, 504):
            self.assertTrue(rp.is_transient_status(s), "HTTP %d 를 확정 오류로 접었다" % s)
        for s in (400, 401, 403, 404, 410, 422, 428, 451):
            self.assertFalse(rp.is_transient_status(s), "HTTP %d 를 일시 장애로 봤다" % s)

    def test_06_5xx_is_retried_then_exits_loud(self):
        """★핵심 핀 — 구판은 여기서 **트레이스백으로 죽었다**(재시도 0회)."""
        op, s = FakeOpener([http_error(503)]), NoSleep()
        with captured() as (out, _e):
            with self.assertRaises(SystemExit) as ctx:
                rp.api("/x", "T", _opener=op, _sleep=s)
        self.assertEqual(len(op.calls), rp.MAX_ATTEMPTS, "503 을 재시도하지 않았다")
        self.assertEqual(len(s.slept), rp.MAX_ATTEMPTS - 1)
        self.assertIn("::error::", str(ctx.exception))
        self.assertIn("HTTP 503", str(ctx.exception))
        self.assertIn("재시도 1/2", out.getvalue())          # 침묵 금지

    def test_07_429_is_retried(self):
        op = FakeOpener([http_error(429)])
        with captured():
            with self.assertRaises(SystemExit):
                rp.api("/x", "T", _opener=op, _sleep=NoSleep())
        self.assertEqual(len(op.calls), rp.MAX_ATTEMPTS)

    def test_08_4xx_is_not_retried_and_propagates(self):
        """404 는 원격의 **확정 답**이다 — 1회로 끝나고 예외 그대로 올라간다.
        (main() 의 draft 폴백이 이 404 를 잡아 목록 조회로 넘어간다 — 그 경로를 지킨다.)"""
        op = FakeOpener([http_error(404)])
        with self.assertRaises(urllib.error.HTTPError) as ctx:
            rp.api("/x", "T", _opener=op, _sleep=NoSleep())
        self.assertEqual(ctx.exception.code, 404)
        self.assertEqual(len(op.calls), 1, "404 를 재시도했다")

    def test_09_connection_errors_are_retried(self):
        for exc in (urllib.error.URLError("dns"), socket.timeout("stalled"),
                    ConnectionResetError("reset")):
            op = FakeOpener([exc])
            with captured():
                with self.assertRaises(SystemExit):
                    rp.api("/x", "T", _opener=op, _sleep=NoSleep())
            self.assertEqual(len(op.calls), rp.MAX_ATTEMPTS,
                             "%s 를 재시도하지 않았다" % type(exc).__name__)

    def test_10_recovery_after_blip_is_logged_not_silent(self):
        op = FakeOpener([http_error(503), b'{"ok":2}'])
        with captured() as (out, _e):
            self.assertEqual(rp.api("/x", "T", _opener=op, _sleep=NoSleep()), {"ok": 2})
        self.assertEqual(len(op.calls), 2)
        self.assertIn("HTTP 503", out.getvalue())            # 사유가 남는다

    def test_11_download_retry_restarts_from_scratch(self):
        """자산 다운로드도 같은 규율. ★재시도는 **처음부터** 받는다 — 부분 파일이 남으면
        1단계 크기 대조가 엉뚱한 값을 본다."""
        op = FakeOpener([socket.timeout("stalled"), BODY])
        with tempfile.TemporaryDirectory() as td:
            dest = os.path.join(td, "a.bin")
            with captured():
                rp.download(ASSET_URL, dest, "T", _opener=op, _sleep=NoSleep())
            with open(dest, "rb") as fh:
                got = fh.read()
        self.assertEqual(got, BODY, "재시도 후 본문이 온전하지 않다")
        self.assertEqual(len(op.calls), 2)

    def test_12_backoff_table_covers_attempts(self):
        self.assertGreaterEqual(len(rp.RETRY_BACKOFF_S), rp.MAX_ATTEMPTS - 1,
                                "백오프 표가 시도 횟수보다 짧다")
        self.assertTrue(all(b > 0 for b in rp.RETRY_BACKOFF_S))

    def test_13_mutating_calls_are_not_retried(self):
        """★DELETE·POST 는 멱등하지 않다 — 서버가 처리했는데 응답만 유실된 경우 재시도는
        두 번째 호출의 404 를 '확정 오류'로 올려 **없는 실패를 만든다**. timeout 만 건다."""
        op = FakeOpener([http_error(503)])
        with self.assertRaises(urllib.error.HTTPError):
            rp.api("/x", "T", method="DELETE", _opener=op, _sleep=NoSleep())
        self.assertEqual(len(op.calls), 1, "DELETE 를 재시도했다")
        self.assertEqual(op.calls[0][1], rp.API_TIMEOUT)     # timeout 은 걸린다


# ──────────────────────────────────────────────────────────────────────────────
# ⓒ ★음성 대조 — 크기 대조 fail-closed 는 **그대로** 살아 있다
#
#   재시도를 넣으면서 절단 축의 게이트까지 무디게 만들면(크기 불일치를 재시도로 눙기면)
#   이 수리는 개악과 구별되지 않는다. main() 1단계가 여전히 **한 번 보고 즉시 적색**인지 본다.
# ──────────────────────────────────────────────────────────────────────────────
class SizeMismatchFailClosedPins(unittest.TestCase):

    def setUp(self):
        self._real = (rp.token, rp.api, rp.download, rp.gatekeeper_gate, rp.BACKUP_ROOT)
        self._tmp = tempfile.TemporaryDirectory()
        rp.BACKUP_ROOT = self._tmp.name
        rp.token = lambda: "T"
        rp.gatekeeper_gate = lambda *a, **k: 0
        self.rel = {"draft": True, "upload_url": "https://u/{?name}",
                    "assets": [{"name": "cys_%s_aarch64.dmg" % V, "size": len(BODY),
                                "url": ASSET_URL}]}
        rp.api = lambda *a, **k: self.rel
        self.downloads = []

    def tearDown(self):
        (rp.token, rp.api, rp.download, rp.gatekeeper_gate, rp.BACKUP_ROOT) = self._real
        self._tmp.cleanup()

    def test_14_size_mismatch_is_red_and_never_retried(self):
        """★핀 — 선언 크기와 다른 바이트가 오면 **즉시 return 1**이고 재다운로드하지 않는다."""
        def short(url, dest, tok, **kw):
            self.downloads.append(url)
            with open(dest, "wb") as fh:
                fh.write(BODY[:10])          # 선언보다 짧다
        rp.download = short
        with captured() as (out, err):
            rc = rp.main(["release-postprocess.py", TAG])
        text = out.getvalue() + err.getvalue()
        self.assertEqual(rc, 1, text)
        self.assertIn("크기 불일치", text)
        self.assertEqual(len(self.downloads), 1,
                         "크기 불일치를 재시도했다 — 같은 바이트에 시간만 쓴다")
        self.assertNotIn("자기 검증 통과", text)
        self.assertNotIn("✅", text)

    def test_15_matching_size_proceeds_past_step1(self):
        """음성 대조의 반대편 — 크기가 맞으면 1단계는 통과한다(그 뒤 exe 부재로 멈춘다).
        이게 없으면 test_14 는 '무조건 빨간 검사'와 구별되지 않는다."""
        def full(url, dest, tok, **kw):
            self.downloads.append(url)
            with open(dest, "wb") as fh:
                fh.write(BODY)
        rp.download = full
        with captured() as (out, err):
            rc = rp.main(["release-postprocess.py", TAG])
        text = out.getvalue() + err.getvalue()
        self.assertEqual(rc, 1, text)                 # exe 가 없어서 2단계에서 멈춘다
        self.assertNotIn("크기 불일치", text)          # ★1단계는 통과했다
        self.assertIn("setup.exe", text)


if __name__ == "__main__":
    unittest.main(verbosity=2)
