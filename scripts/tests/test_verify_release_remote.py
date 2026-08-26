"""verify-release-remote.py 의 밀폐 unittest — 네트워크·실자산 불요, 페이크 curl 만 사용.

★왜 이 테스트가 필요한가 (2026-08-25 위양성 BLOCK 사고)
  원격 검증기는 자산을 `curl -s`(--fail 없음)로 받아 **종료코드도 수신 바이트수도 보지 않고**
  곧바로 `hashlib.sha256(blob)` 를 걸었다. 전송이 도중에 끊기면 **부분 본문**이 그대로 해시돼
  "해시 불일치"로 찍힌다 — 이 코드는 원리적으로 **'자산이 썩었다'와 '내 다운로드가 실패했다'를
  구분할 수 없었다.**
  실관측: v0.14.27 2차 실행에서 `cys_0.14.27_aarch64.dmg` 해시 불일치 · EXITCODE=1 로 위양성
  BLOCK. 같은 자산 7회 재실측(직접 GET 3 + 스크립트 curl 인자 복제 3 + A/B cmp) 전건
  269437340B · b5907506f9… 일치 = 자산 무결. 1·3차 실행은 PASS.
  게이트가 무작위 적색을 내면 사람은 진짜 적색까지 "또 그거겠지"로 흘린다 — **그 클래스를 끊는
  판정 분리(FAIL=exit 1 vs DOWNLOAD_FAILED=exit 3)를 여기 못박는다.**

★설계
  실 curl 대신 **페이크 curl**(argv 를 해석해 실제로 -o/-D 파일을 쓰고 rc·%{http_code} 를
  돌려주는 호출가능 객체)을 주입한다. 주입 지점은 `fetch(_runner=…, _sleep=…)` 의 테스트 전용
  키워드 인자와, 종단 시험에서의 `vr._run_curl` 치환이다(test_release_postprocess_gate.py 의
  페이크 게이트 관례와 같다). 계획에 없는 URL 을 부르면 즉시 AssertionError — 이 파일은
  **어떤 경우에도 네트워크에 나가지 않는다.**
  경로는 전부 tempfile — 개인 경로·실 홈 디렉터리 금지(test_release_verify.py 관례).

  ★음성 대조가 이 파일의 핵심이다: 수리가 **진짜 해시 불일치를 삼키지 않는지**(exit 1 유지)를
  ⓒ군이 지킨다. 그게 없으면 이 수리는 게이트를 무디게 만드는 개악과 구별되지 않는다.

사용: python3 scripts/tests/test_verify_release_remote.py
"""

import contextlib
import hashlib
import importlib.util
import io
import os
import unittest

# ★하이픈 파일명(`verify-release-remote.py`)은 `import` 문으로 못 부른다 — importlib 로 직접
#   적재한다(test_release_verify.py·test_release_postprocess_gate.py 와 같은 이유·같은 관례).
_VR_PATH = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "verify-release-remote.py")
_spec = importlib.util.spec_from_file_location("verify_release_remote", _VR_PATH)
vr = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(vr)

VER = "9.9.9"
PREV = "9.9.8"
FOUR = ["cys_%s_aarch64.dmg" % VER, "cys_%s_x64.dmg" % VER,
        "cys_%s_x64-setup.exe" % VER, "cys_%s_x64-setup.zip" % VER]
ASSET_URL = "%s/downloads/%s" % (vr.SITE, FOUR[0])

# 실물 dmg 를 흉내내는 합성 본문(수 KB) — 실물 대조는 스모크로 따로 한다
# (`python3 scripts/verify-release-remote.py 0.14.27 0.14.26`).
FULL_BODY = b"CYSDMG" + b"\xa5" * 4096
PARTIAL_BODY = FULL_BODY[:1500]          # ★전송이 끊긴 부분 본문
OTHER_BODY = b"CYSDMG" + b"\x5a" * 4096  # 길이는 같고 내용만 다르다 = **진짜** 해시 불일치


def sha(b):
    return hashlib.sha256(b).hexdigest()


@contextlib.contextmanager
def quiet():
    """재시도 로그가 **검증 대상이 아닌** 곳에서만 stdout 을 삼킨다.
    (로그 자체를 단언하는 곳은 test_09·test_16 이며 거기서는 삼키지 않는다.)"""
    buf = io.StringIO()
    with contextlib.redirect_stdout(buf):
        yield buf


class Resp(object):
    """페이크 응답 1건.

    rc      : curl 종료코드(None = http 로 자동 결정 — 4xx/5xx 는 --fail 이 비0 으로 접는다)
    body    : -o 파일에 실제로 쓸 바이트
    clen    : Content-Length 헤더 값. None = len(body) · False = 헤더 자체를 생략
    """

    def __init__(self, http=200, body=b"", rc=None, clen=None, stderr=""):
        self.http = http
        self.body = body
        self.rc = rc
        self.clen = clen
        self.stderr = stderr

    def resolved_rc(self):
        if self.rc is not None:
            return self.rc
        # 실측(2026-08-25): HTTP/2 404 는 --fail 아래서 rc 22 가 아니라 **56**으로 나온다.
        # 구현이 rc 가 아니라 %{http_code} 로 가르는 이유를 페이크도 그대로 재현한다.
        return 56 if self.http >= 400 else 0


class FakeCurl(object):
    """argv 를 해석해 실제 파일을 쓰는 페이크 curl. plan = {url: [Resp, ...]}.

    ★HEAD 는 `"HEAD " + url` 키로 따로 계획한다 — 실제로 HEAD 는 성공하는데 본문 GET 만
      끊기는 경우(=이번 사고의 형태)를 재현하려면 두 메서드가 갈려야 한다. HEAD 키가
      없으면 GET 큐로 떨어진다.
    호출마다 큐에서 하나씩 소비하고, 큐가 마르면 마지막 응답을 반복한다
    (=재시도해도 계속 실패하는 원격을 재현).
    """

    def __init__(self, plan):
        self.plan = plan
        self.calls = []          # 실행된 argv 전부 — 재시도 횟수 검사에 쓴다

    def urls_called(self, url, head=False):
        return sum(1 for a in self.calls if a[-1] == url and (("-I" in a) == head))

    def __call__(self, argv):
        self.calls.append(list(argv))
        url = argv[-1]
        dest = argv[argv.index("-o") + 1]
        hdr = argv[argv.index("-D") + 1]
        head = "-I" in argv

        key = ("HEAD " + url) if (head and ("HEAD " + url) in self.plan) else url
        assert key in self.plan, "계획에 없는 URL 호출(=라이브 접촉 의심): %s" % key
        q = self.plan[key]
        r = q.pop(0) if len(q) > 1 else q[0]

        clen = len(r.body) if r.clen is None else r.clen
        htxt = "HTTP/2 %d \r\n" % r.http
        if clen is not False:
            htxt += "content-length: %d\r\n" % clen
        htxt += "\r\n"
        with open(hdr, "w") as fh:
            fh.write(htxt)

        # 실 curl 재현: `--fail` 은 4xx/5xx 응답을 -o 로 **아예 흘리지 않는다**(HEAD 포함).
        # 성공한 `-I` 는 헤더를 -o 로 흘린다.
        if r.http >= 400:
            payload = b""
        elif head:
            payload = htxt.encode()
        else:
            payload = r.body
        with open(dest, "wb") as fh:
            fh.write(payload)

        return r.resolved_rc(), "%d %d" % (r.http, len(payload)), r.stderr


class NoSleep(object):
    """유계 재시도의 대기를 삼키고 호출 횟수만 센다."""

    def __init__(self):
        self.slept = []

    def __call__(self, sec):
        self.slept.append(sec)


# ──────────────────────────────────────────────────────────────────────────────
# ⓐ 부분 본문 → **다운로드 실패**로 분류 (해시 불일치 아님)
# ──────────────────────────────────────────────────────────────────────────────
class PartialBodyTests(unittest.TestCase):

    def test_01_truncated_transfer_rc_nonzero_is_download_failure(self):
        """전송 중단(rc 18 · 부분 본문) → err 로 분류되고 **해시는 아예 계산되지 않는다**.

        ★구판 재현: 구판은 rc 를 안 봤으므로 이 부분 본문을 그대로 sha256 해 '해시 불일치'로
          찍었다. 아래 assertIsNone(digest) 가 그 경로를 구조적으로 봉한다.
        """
        fake = FakeCurl({ASSET_URL: [Resp(body=PARTIAL_BODY, rc=18, clen=len(FULL_BODY))]})
        s = NoSleep()
        with quiet():
            f = vr.fetch(ASSET_URL, want_text=False, want_digest=True, _runner=fake, _sleep=s)
        self.assertFalse(f.download_ok, "부분 본문이 성공으로 통과했다")
        self.assertIn("curl 종료코드 18", f.err)
        self.assertIsNone(f.digest, "다운로드 실패한 본문의 해시를 계산했다 — 위양성 경로가 살아있다")
        self.assertNotEqual(sha(PARTIAL_BODY), sha(FULL_BODY))  # 픽스처 자기정합

    def test_02_short_body_with_rc0_caught_by_content_length(self):
        """rc 0 인데 본문만 짧은 경우(가장 음험한 형태) → Content-Length 대조가 잡는다."""
        fake = FakeCurl({ASSET_URL: [Resp(body=PARTIAL_BODY, rc=0, clen=len(FULL_BODY))]})
        with quiet():
            f = vr.fetch(ASSET_URL, want_text=False, want_digest=True,
                         _runner=fake, _sleep=NoSleep())
        self.assertFalse(f.download_ok)
        self.assertIn("부분 본문", f.err)
        self.assertIsNone(f.digest)


# ──────────────────────────────────────────────────────────────────────────────
# ⓑ Content-Length 불일치 감지
# ──────────────────────────────────────────────────────────────────────────────
class ContentLengthTests(unittest.TestCase):

    def test_03_mismatch_reports_both_numbers(self):
        """수신 바이트수와 선언값을 **둘 다** 문면에 남긴다(사람이 즉시 판별하도록)."""
        fake = FakeCurl({ASSET_URL: [Resp(body=b"x" * 100, rc=0, clen=999)]})
        with quiet():
            f = vr.fetch(ASSET_URL, want_text=False, _runner=fake, _sleep=NoSleep())
        self.assertFalse(f.download_ok)
        self.assertIn("100B", f.err)
        self.assertIn("999B", f.err)

    def test_04_exact_match_passes(self):
        fake = FakeCurl({ASSET_URL: [Resp(body=FULL_BODY)]})
        f = vr.fetch(ASSET_URL, want_text=False, want_digest=True,
                     _runner=fake, _sleep=NoSleep())
        self.assertTrue(f.download_ok, f.err)
        self.assertEqual(f.nbytes, len(FULL_BODY))
        self.assertEqual(f.clen, len(FULL_BODY))
        self.assertEqual(f.digest, sha(FULL_BODY))

    def test_05_absent_header_does_not_fabricate_a_failure(self):
        """Content-Length 를 서버가 안 주면(청크 전송 등) 그 축만 없는 것이다 —
        rc 0 + 본문 수신이면 **새 위양성 클래스를 만들지 않는다**."""
        fake = FakeCurl({ASSET_URL: [Resp(body=FULL_BODY, clen=False)]})
        f = vr.fetch(ASSET_URL, want_text=False, want_digest=True,
                     _runner=fake, _sleep=NoSleep())
        self.assertTrue(f.download_ok, f.err)
        self.assertIsNone(f.clen)
        self.assertEqual(f.digest, sha(FULL_BODY))


# ──────────────────────────────────────────────────────────────────────────────
# ⓓ 재시도는 유계 · 침묵 금지
# ──────────────────────────────────────────────────────────────────────────────
class BoundedRetryTests(unittest.TestCase):

    def test_06_retry_is_bounded_by_max_attempts(self):
        """계속 실패해도 정확히 MAX_ATTEMPTS 회에서 멈춘다(무한 루프 불가 · 재시도 규율)."""
        fake = FakeCurl({ASSET_URL: [Resp(body=PARTIAL_BODY, rc=18, clen=len(FULL_BODY))]})
        s = NoSleep()
        with quiet():
            f = vr.fetch(ASSET_URL, want_text=False, _runner=fake, _sleep=s)
        self.assertEqual(fake.urls_called(ASSET_URL), vr.MAX_ATTEMPTS)
        self.assertEqual(len(s.slept), vr.MAX_ATTEMPTS - 1)
        self.assertEqual(f.tries, vr.MAX_ATTEMPTS)
        self.assertIn("시도 %d회 소진" % vr.MAX_ATTEMPTS, f.err)

    def test_07_backoff_is_exponential_and_bounded(self):
        s = NoSleep()
        with quiet():
            vr.fetch(ASSET_URL, want_text=False, _sleep=s,
                     _runner=FakeCurl({ASSET_URL: [Resp(body=b"", rc=28)]}))
        self.assertEqual(s.slept, [2.0, 5.0])
        self.assertTrue(all(b > 0 for b in vr.RETRY_BACKOFF_S))
        self.assertGreaterEqual(len(vr.RETRY_BACKOFF_S), vr.MAX_ATTEMPTS - 1,
                                "백오프 표가 시도 횟수보다 짧다")

    def test_08_attempts_one_means_no_retry(self):
        fake = FakeCurl({ASSET_URL: [Resp(body=b"", rc=28)]})
        with quiet():
            vr.fetch(ASSET_URL, want_text=False, attempts=1, _runner=fake, _sleep=NoSleep())
        self.assertEqual(fake.urls_called(ASSET_URL), 1)

    def test_09_recovery_is_logged_not_silent(self):
        """1차 실패 → 2차 성공. 성공했어도 **재시도가 있었다는 사실이 남는다**(침묵 금지)."""
        fake = FakeCurl({ASSET_URL: [Resp(body=PARTIAL_BODY, rc=18, clen=len(FULL_BODY)),
                                     Resp(body=FULL_BODY)]})
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            f = vr.fetch(ASSET_URL, want_text=False, want_digest=True,
                         _runner=fake, _sleep=NoSleep())
        self.assertTrue(f.download_ok, f.err)
        self.assertEqual(f.tries, 2)
        self.assertEqual(len(f.retries), 1)
        self.assertIn("재시도 1/2", buf.getvalue())
        self.assertEqual(f.digest, sha(FULL_BODY))

    def test_10_http_error_is_not_retried(self):
        """4xx(429 제외)는 원격의 **확정 답**이다 — 재시도해도 같으므로 **한 번만** 부르고
        적색 재료로 넘긴다."""
        fake = FakeCurl({ASSET_URL: [Resp(http=404)]})
        f = vr.fetch(ASSET_URL, want_text=False, _runner=fake, _sleep=NoSleep())
        self.assertEqual(fake.urls_called(ASSET_URL), 1, "HTTP 404 를 재시도했다")
        self.assertTrue(f.download_ok, "404 를 '다운로드 실패'로 오분류했다")
        self.assertEqual(f.status, 404)
        self.assertFalse(f.http_ok)


# ──────────────────────────────────────────────────────────────────────────────
# ⓖ ★MF1 — 429·5xx 는 '원격이 틀렸다'가 아니라 '지금 대답을 못 한다'다 (2026-08-27 2라운드)
#
#   1라운드 구현은 `status >= 400` 을 통째로 접어 **시도 1회**로 exit 1 BLOCK 을 냈다.
#   호스팅/CDN 블립 1회 = 릴리스 BLOCK — 이 파일이 죽이겠다고 선언한 위양성 클래스 그 자체다.
# ──────────────────────────────────────────────────────────────────────────────
class TransientStatusTests(unittest.TestCase):

    def test_20_transient_classifier_boundary(self):
        """경계를 한 곳(is_transient_status)에만 두고 그 경계를 문자로 못박는다."""
        for s in (429, 500, 502, 503, 504, 599):
            self.assertTrue(vr.is_transient_status(s), "HTTP %d 를 확정 오류로 접었다" % s)
        for s in (400, 401, 403, 404, 410, 418, 428, 430, 451):
            self.assertFalse(vr.is_transient_status(s), "HTTP %d 를 일시 장애로 봤다" % s)

    def test_21_503_is_retried_max_attempts(self):
        """★MF1 핀 (a) — 503 은 정확히 MAX_ATTEMPTS 회 호출된다(시도 1회로 끝나지 않는다)."""
        fake = FakeCurl({ASSET_URL: [Resp(http=503)]})
        s = NoSleep()
        with quiet():
            f = vr.fetch(ASSET_URL, want_text=False, _runner=fake, _sleep=s)
        self.assertEqual(fake.urls_called(ASSET_URL), vr.MAX_ATTEMPTS,
                         "503 을 재시도하지 않았다 — 블립 1회로 릴리스를 BLOCK 한다")
        self.assertEqual(len(s.slept), vr.MAX_ATTEMPTS - 1)
        self.assertFalse(f.download_ok, "503 소진을 '확정 답'으로 접었다")
        self.assertIn("원격 일시 불응답(HTTP 503)", f.err)
        self.assertIn("시도 %d회 소진" % vr.MAX_ATTEMPTS, f.err)

    def test_22_429_is_retried(self):
        """429 는 4xx 지만 rate limit 이다 — 확정 답이 아니므로 재시도한다."""
        fake = FakeCurl({ASSET_URL: [Resp(http=429)]})
        with quiet():
            f = vr.fetch(ASSET_URL, want_text=False, _runner=fake, _sleep=NoSleep())
        self.assertEqual(fake.urls_called(ASSET_URL), vr.MAX_ATTEMPTS)
        self.assertFalse(f.download_ok)

    def test_23_transient_then_200_recovers(self):
        """★MF1 핀 (b) 전반 — 첫 시도 503 → 재시도 200 이면 **회복**이다(재시도 사실은 남는다)."""
        fake = FakeCurl({ASSET_URL: [Resp(http=503), Resp(body=FULL_BODY)]})
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            f = vr.fetch(ASSET_URL, want_text=False, want_digest=True,
                         _runner=fake, _sleep=NoSleep())
        self.assertTrue(f.download_ok, f.err)
        self.assertEqual(f.digest, sha(FULL_BODY))
        self.assertEqual(f.tries, 2)
        self.assertIn("HTTP 503", buf.getvalue())      # 침묵 금지 — 사유가 남는다


# ──────────────────────────────────────────────────────────────────────────────
# ⓗ ★MF2 — 미판정은 자산 상태를 **어느 방향으로도** 단언하지 않는다 (2026-08-27 2라운드)
#
#   1라운드 전: "자산이 썩었다"고 단언 → 1라운드 후: "자산 문제가 아니다"라고 단언.
#   근거 없는 주장이 방향만 바꿔 살아남았다. 이 도구는 오리진에 박힌 부분 자산과 경로에서
#   끊긴 전송을 **가릴 근거가 없다**(수신 측에서 같은 증상). 말할 수 있는 건 "판정 못 했다"뿐.
# ──────────────────────────────────────────────────────────────────────────────
class UndeterminedWordingTests(unittest.TestCase):

    def test_24_repeated_same_point_failure_is_reported_as_observation(self):
        """오리진이 CL 8192 를 선언하고 3000B 만 보내고 끊는 경우 — 매 시도 같은 지점에서
        끝난다. 그 **관측 사실**을 그대로 남기되 원인은 지목하지 않는다."""
        fake = FakeCurl({ASSET_URL: [Resp(body=b"z" * 3000, rc=0, clen=8192)]})
        with quiet():
            f = vr.fetch(ASSET_URL, want_text=False, want_digest=True,
                         _runner=fake, _sleep=NoSleep())
        self.assertFalse(f.download_ok)
        self.assertIn("★관측", f.err)
        self.assertIn("수신 3000B", f.err)                       # 사실
        self.assertIn("원인을 판정하지 않는다", f.err)            # 해석 금지 명시
        self.assertEqual(len(f.attempts_log), vr.MAX_ATTEMPTS)   # 시도별 관측이 다 남는다
        self.assertIsNone(f.digest)

    def test_25_varying_failure_point_gets_no_sameness_claim(self):
        """매 시도 다른 지점에서 끝났으면 '같은 지점' 이라는 **사실 주장을 하지 않되**,
        시도별 값은 그대로 남긴다(주장 금지 ≠ 침묵)."""
        fake = FakeCurl({ASSET_URL: [Resp(body=b"z" * 100, rc=0, clen=8192),
                                     Resp(body=b"z" * 900, rc=0, clen=8192),
                                     Resp(body=b"z" * 2500, rc=0, clen=8192)]})
        with quiet():
            f = vr.fetch(ASSET_URL, want_text=False, _runner=fake, _sleep=NoSleep())
        self.assertFalse(f.download_ok)
        self.assertNotIn("같은 지점", f.err)        # 없는 규칙성을 주장하지 않는다
        self.assertIn("시도별", f.err)              # 그래도 사실은 남는다
        for n in ("100B", "900B", "2500B"):
            self.assertIn(n, f.err)


class CurlArgvContractTests(unittest.TestCase):
    """수리의 뼈대가 되는 curl 인자 4종을 문자열로 못박는다(누가 지워도 여기서 걸린다)."""

    def test_11_required_flags_present(self):
        a = vr.curl_argv("https://example.invalid/x", 120, False, "/tmp/b", "/tmp/h")
        for flag in ("--fail", "--show-error", "-D", "-o", "-w", "--max-time"):
            self.assertIn(flag, a, "%s 가 빠졌다" % flag)
        self.assertIn("%{http_code} %{size_download}", a)
        self.assertEqual(a[-1], "https://example.invalid/x")
        self.assertNotIn("-I", a)

    def test_12_head_adds_dash_capital_i(self):
        self.assertIn("-I", vr.curl_argv("https://example.invalid/x", 120, True, "/tmp/b", "/tmp/h"))


# ──────────────────────────────────────────────────────────────────────────────
# 종단(end-to-end) — 판정 분리가 **종료코드로** 드러나는가
# ──────────────────────────────────────────────────────────────────────────────
def build_site(asset_bodies, sums_bodies=None):
    """합성 사이트 하나. sums_bodies 를 따로 주면 'SUMS 는 A 를 적었는데 실물은 B' 를 만든다."""
    sums_bodies = sums_bodies or asset_bodies
    links = "".join('<a href="/downloads/%s">다운로드</a>\n' % f for f in FOUR)  # ver 4회
    main = ("<html><body>\n" + links
            + '<p class="dl-hero__winnote">참고 — 윈도우 설치파일: SmartScreen 안내</p>\n'
            + "<span>0MB</span>\n"
            + ("<b>v%s</b>\n" % VER) * 5          # ver 5회 → 합계 9회
            + "</body></html>\n")
    assert main.count(VER) == 9, "픽스처 자기정합 실패: 신버전 토큰 %d개" % main.count(VER)
    assert main.count(PREV) == 0

    dl = ("<html><body>\n"
          '<section data-cys-release-marker="windows-defender-guidance-v2">\n'
          "  Windows Defender SmartScreen 이 뜨면 '추가 정보' 를 누르세요.\n"
          "</section>\n</body></html>\n")

    sums = "".join("%s  %s\n" % (sha(sums_bodies[f]), f) for f in FOUR)

    plan = {vr.SITE + "/": [Resp(body=main.encode())],
            vr.SITE + "/downloads/": [Resp(body=dl.encode())],
            "%s/downloads/SHA256SUMS.txt" % vr.SITE: [Resp(body=sums.encode())]}
    for f in FOUR:
        u = "%s/downloads/%s" % (vr.SITE, f)
        plan[u] = [Resp(body=asset_bodies[f])]
        # HEAD(③④)는 건강한 200 — 본문 GET 만 따로 망가뜨릴 수 있게 분리해 둔다.
        plan["HEAD " + u] = [Resp(clen=len(asset_bodies[f]))]
    return plan


class EndToEndVerdictTests(unittest.TestCase):
    """★이 군이 계약의 본체다 — 같은 증상(해시가 안 맞음)이 **원인에 따라 다른 exit** 로 갈린다."""

    def setUp(self):
        self._real_runner = vr._run_curl
        self._real_backoff = vr.RETRY_BACKOFF_S
        vr.RETRY_BACKOFF_S = (0.0, 0.0)       # 테스트는 대기하지 않는다
        vr.results[:] = []
        vr.download_failures[:] = []

    def tearDown(self):
        vr._run_curl = self._real_runner
        vr.RETRY_BACKOFF_S = self._real_backoff
        vr.results[:] = []
        vr.download_failures[:] = []

    def _run(self, plan):
        fake = FakeCurl(plan)
        vr._run_curl = fake
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            rc = vr.main(["verify-release-remote.py", VER, PREV])
        return rc, buf.getvalue(), fake

    def test_13_healthy_site_passes(self):
        """기준선 — 이게 깨지면 아래 실패 단언들은 '무조건 빨간 검사'를 오독한 것이다."""
        rc, out, _ = self._run(build_site({f: FULL_BODY for f in FOUR}))
        self.assertEqual(rc, vr.EXIT_OK, out)
        self.assertIn("7/7 PASS", out)
        self.assertNotIn("DOWNLOAD_FAILED", out)

    def test_14_truncated_asset_is_exit3_not_asset_red(self):
        """★사고 재현 — 부분 본문의 해시는 SUMS 와 다르다. 구판은 여기서 '해시 불일치'
        exit 1(위양성 BLOCK)을 냈다. 수리판은 **DOWNLOAD_FAILED exit 3**이어야 한다."""
        plan = build_site({f: FULL_BODY for f in FOUR})
        plan[ASSET_URL] = [Resp(body=PARTIAL_BODY, rc=18, clen=len(FULL_BODY))]
        rc, out, fake = self._run(plan)
        self.assertEqual(rc, vr.EXIT_DOWNLOAD_FAILED, out)
        self.assertIn("DOWNLOAD_FAILED", out)
        self.assertNotIn("해시 불일치", out)      # ★자산을 적색으로 부르지 않는다
        self.assertIn("미판정", out)               # ★첫 줄이 분류를 이름으로 댄다
        self.assertIn("판정되지 않았다", out)
        # ★MF2 (2026-08-27) — 1라운드가 남겼던 **반대 방향의 단언**이 사라졌는지 못박는다.
        #   구 문면: "이것은 **자산 적색이 아니다**" ← 이 도구가 말할 근거가 없는 주장이다.
        self.assertNotIn("자산 적색이 아니다", out)
        self.assertIn("어느 쪽으로도 읽지 마라", out)   # 무판정임을 명시
        self.assertEqual(fake.urls_called(ASSET_URL), vr.MAX_ATTEMPTS)  # 유계 재시도

    def test_14b_origin_serves_partial_asset_is_undetermined_not_green(self):
        """★MF2 본체 — 오리진이 CL 8192 를 선언하고 3000B 만 보내고 끊는 경우(=실제로 오리진에
        박힌 부분 자산). 이 도구는 그것과 '경로에서 끊긴 전송'을 가릴 수 없다.
        요구: **초록이 아니고**(exit != 0) **적색도 아니며**(exit != 1) 관측 사실이 그대로 남는다."""
        plan = build_site({f: FULL_BODY for f in FOUR})
        plan[ASSET_URL] = [Resp(body=b"z" * 3000, rc=0, clen=8192)]
        rc, out, _ = self._run(plan)
        self.assertNotEqual(rc, vr.EXIT_OK, "부분 자산을 초록으로 통과시켰다\n" + out)
        self.assertNotEqual(rc, vr.EXIT_FAIL, "판정 근거 없이 적색으로 단언했다\n" + out)
        self.assertEqual(rc, vr.EXIT_DOWNLOAD_FAILED, out)
        self.assertIn("★관측", out)               # 사실은 그대로 보고한다
        self.assertIn("수신 3000B", out)
        self.assertNotIn("해시 불일치", out)

    def test_15_true_hash_mismatch_is_still_exit1_red(self):
        """★음성 대조 — 완전히 수신됐는데 바이트가 다르면 **여전히 자산 적색(exit 1)**이다.
        수리가 진짜 결함을 삼키면 게이트는 무의미해진다."""
        bodies = {f: FULL_BODY for f in FOUR}
        bodies[FOUR[0]] = OTHER_BODY            # 길이는 같고 내용만 다르다
        rc, out, _ = self._run(build_site(bodies, sums_bodies={f: FULL_BODY for f in FOUR}))
        self.assertEqual(rc, vr.EXIT_FAIL, out)
        self.assertIn("해시 불일치", out)
        self.assertIn("완전 수신", out)          # 완결성을 확인한 뒤의 불일치임을 문면이 말한다
        self.assertNotIn("DOWNLOAD_FAILED", out)

    def test_16_retry_then_success_still_passes(self):
        """1차 끊김 → 2차 완전 수신 = 자산은 무결하다. 게이트는 초록이고 재시도 사실은 남는다."""
        plan = build_site({f: FULL_BODY for f in FOUR})
        plan[ASSET_URL] = [Resp(body=PARTIAL_BODY, rc=18, clen=len(FULL_BODY)),
                           Resp(body=FULL_BODY)]
        rc, out, fake = self._run(plan)
        self.assertEqual(rc, vr.EXIT_OK, out)
        self.assertIn("7/7 PASS", out)
        self.assertIn("재시도 1/2", out)                 # 침묵 금지
        self.assertEqual(fake.urls_called(ASSET_URL), 2)

    def test_17_missing_asset_404_is_red_not_download_failed(self):
        """자산이 실제로 없으면(404) 그건 **발행 결함**이다 — exit 3 으로 눙치지 않는다."""
        plan = build_site({f: FULL_BODY for f in FOUR})
        plan[ASSET_URL] = [Resp(http=404)]
        rc, out, _ = self._run(plan)
        self.assertEqual(rc, vr.EXIT_FAIL, out)
        self.assertIn("404", out)

    def test_18_page_truncation_does_not_produce_string_count_red(self):
        """★같은 결함 패턴의 페이지 판(전수 수색 산물) — 메인 HTML 이 잘려 오면 ①②③④⑤⑦ 이
        전부 오답을 낸다. 그 위에서 판정하지 않고 DOWNLOAD_FAILED 로 멈춘다."""
        plan = build_site({f: FULL_BODY for f in FOUR})
        full = plan[vr.SITE + "/"][0].body
        plan[vr.SITE + "/"] = [Resp(body=full[:40], rc=18, clen=len(full))]
        rc, out, _ = self._run(plan)
        self.assertEqual(rc, vr.EXIT_DOWNLOAD_FAILED, out)
        self.assertIn("메인 페이지 수신", out)
        self.assertNotIn("신버전 문자열 9", out)   # 잘린 본문 위에서 세지 않았다

    def test_19_exit_codes_are_distinct(self):
        """네 종료코드가 서로 다른 값이어야 문면 없이도 구분된다."""
        codes = [vr.EXIT_OK, vr.EXIT_FAIL, vr.EXIT_USAGE, vr.EXIT_DOWNLOAD_FAILED]
        self.assertEqual(len(set(codes)), 4, codes)

    # ── ★MF1 종단 핀 (b) — 503 은 exit 로 갈린다 ──────────────────────────────
    def test_26_persistent_503_is_exit3_not_red(self):
        """★MF1 핀 (b) 후반 — 503 이 계속되면 **미판정 exit 3**이다.
        1라운드 구현은 여기서 '자산 부재/오류(HTTP 503)' 문면으로 **exit 1 BLOCK** 을 냈고,
        사람은 그 문면을 읽고 발행 결함으로 오독했다."""
        plan = build_site({f: FULL_BODY for f in FOUR})
        plan[ASSET_URL] = [Resp(http=503)]
        rc, out, fake = self._run(plan)
        self.assertEqual(rc, vr.EXIT_DOWNLOAD_FAILED, out)
        self.assertEqual(fake.urls_called(ASSET_URL), vr.MAX_ATTEMPTS, out)
        self.assertNotIn("자산 부재", out)          # ★발행 결함으로 오독시키지 않는다
        self.assertIn("HTTP 503", out)              # 사실은 남는다

    def test_27_503_then_200_is_green(self):
        """★MF1 핀 (b) 전반 종단 — 첫 시도 503, 재시도 200 이면 **exit 0**이다.
        호스팅/CDN 블립 1회로 릴리스가 BLOCK 되지 않는다."""
        plan = build_site({f: FULL_BODY for f in FOUR})
        plan[ASSET_URL] = [Resp(http=503), Resp(body=FULL_BODY)]
        rc, out, fake = self._run(plan)
        self.assertEqual(rc, vr.EXIT_OK, out)
        self.assertIn("7/7 PASS", out)
        self.assertIn("재시도 %d회 후 성공" % 1, out)   # 침묵 금지
        self.assertEqual(fake.urls_called(ASSET_URL), 2)

    def test_28_head_503_is_undetermined_not_red(self):
        """HEAD 경로(③④)도 같은 규율을 탄다 — 5xx 는 '자산 부재'가 아니다."""
        plan = build_site({f: FULL_BODY for f in FOUR})
        plan["HEAD " + ASSET_URL] = [Resp(http=503)]
        rc, out, fake = self._run(plan)
        self.assertEqual(rc, vr.EXIT_DOWNLOAD_FAILED, out)
        # ③(용량)·④(링크) 가 같은 URL 을 각각 HEAD 한다 → 2 × MAX_ATTEMPTS.
        self.assertEqual(fake.urls_called(ASSET_URL, head=True), 2 * vr.MAX_ATTEMPTS, out)
        self.assertNotIn("자산 부재", out)

    def test_29_verdict_headline_names_the_category(self):
        """★3분류 계약 — 사람이 **첫 줄 + exit** 만 보고 어느 것인지 알아야 한다."""
        # ⓐ 자산 무결성 실패
        bodies = {f: FULL_BODY for f in FOUR}
        bodies[FOUR[0]] = OTHER_BODY
        rc, out, _ = self._run(build_site(bodies, sums_bodies={f: FULL_BODY for f in FOUR}))
        self.assertEqual(rc, vr.EXIT_FAIL)
        self.assertIn("판정: 적색 — %s" % vr.KIND_INTEGRITY, out)

        # ⓑ 원격 확정 오류(4xx)
        self.setUp()
        plan = build_site({f: FULL_BODY for f in FOUR})
        plan[ASSET_URL] = [Resp(http=404)]
        rc, out, _ = self._run(plan)
        self.assertEqual(rc, vr.EXIT_FAIL)
        self.assertIn("판정: 적색 — %s" % vr.KIND_REMOTE, out)

        # ⓒ 미판정
        self.setUp()
        plan = build_site({f: FULL_BODY for f in FOUR})
        plan[ASSET_URL] = [Resp(http=503)]
        rc, out, _ = self._run(plan)
        self.assertEqual(rc, vr.EXIT_DOWNLOAD_FAILED)
        self.assertIn("판정: %s" % vr.KIND_UNDETERMINED, out)

        # ⓓ 초록
        self.setUp()
        rc, out, _ = self._run(build_site({f: FULL_BODY for f in FOUR}))
        self.assertEqual(rc, vr.EXIT_OK)
        self.assertIn("판정: 전건 통과", out)


if __name__ == "__main__":
    unittest.main(verbosity=2)
