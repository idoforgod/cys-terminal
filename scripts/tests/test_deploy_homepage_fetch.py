"""deploy-homepage.py 수신 검증의 밀폐 unittest — 네트워크·FTP 불요, 전부 페이크.

★왜 이 파일이 필요한가 (2026-08-27 · 검증기보다 **높은** 심각도)
  `scripts/verify-release-remote.py` 의 위양성은 "게이트가 헛되이 빨개진다"였다. 여기는
  방향이 반대이고 더 나쁘다: 이 파일의 `fetch()` 는 **라이브 페이지를 받아 그 위에서
  버전·용량 토큰을 치환한 뒤 그대로 업로드한다**(main() → strip_main_macnote → ftp_put).
  구 구현은 `curl -s`(--fail 없음)로 받아 **종료코드도 수신 바이트수도 보지 않았다** —
  전송이 도중에 끊기면 **잘린 홈페이지가 그대로 발행된다.** 배포는 비가역이다.

★이 파일의 핵심은 **음성 대조**다: 부분 본문을 받았을 때 `ftp_put` 이 **한 번도 호출되지
  않는지**를 단언한다. "중단했다고 보고했지만 실은 올렸다"를 구조적으로 봉한다.

★2026-08-27 3라운드(MF-A) 확장 — 같은 무방비가 **FTPS 목록 수신**에도 있었다(`ftp_list` 가
  rc 를 보지 않고 `[]` 를 돌려줘 '실패'와 '비어 있음'이 같아짐 → 구자산이 한 건도 지워지지
  않은 채 '구버전 없음' + exit 0). 그 축의 핀은 `FtpListFailClosedTests`·`FtpListContractTests`.

★설계 — `test_verify_release_remote.py` 의 페이크 curl 관례를 그대로 따른다. 실제로 -o/-D
  파일을 쓰고 rc·%{http_code} 를 돌려주는 호출가능 객체를 주입하고, 계획에 없는 URL 을
  부르면 즉시 AssertionError(=라이브 접촉 의심). 쓰기 경로(ftp_put·ftp_delete·sftp_put)와
  자격(load_env)은 스파이로 치환하고, **읽기 경로인 LIST 는 한 칸 아래(`dh.curl`)에서**
  페이크로 막아 실 `ftp_list` 코드를 그대로 태운다(수리 대상을 건너뛰지 않기 위해).
  결과적으로 **어떤 경우에도 원격에 나가지 않는다.**
  경로는 전부 tempfile — 개인 경로·실 홈 디렉터리 금지.

사용: python3 scripts/tests/test_deploy_homepage_fetch.py
"""

import contextlib
import importlib.util
import io
import os
import tempfile
import unittest

# 하이픈 파일명은 import 문으로 못 부른다 — importlib 로 직접 적재한다(레인 관례).
_DH_PATH = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "deploy-homepage.py")
_spec = importlib.util.spec_from_file_location("deploy_homepage", _DH_PATH)
dh = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(dh)

# ★버전 픽스처는 반드시 `0.x.y` 꼴이어야 한다 — main() 의 이전버전 탐지가
#   `re.findall(r"0\.\d+\.\d+", …)` 이라 다른 꼴이면 prev=None 이 되어 용량 토큰 경로
#   (=이 파일이 검사하려는 구자산 HEAD 분기)가 통째로 건너뛰어진다.
VER = "0.99.9"
PREV = "0.99.8"
FOUR = ["cys_%s_aarch64.dmg" % VER, "cys_%s_x64.dmg" % VER,
        "cys_%s_x64-setup.exe" % VER, "cys_%s_x64-setup.zip" % VER]
MAIN_URL = dh.SITE + "/"
DL_URL = dh.SITE + "/downloads/"

# 라이브 메인페이지를 흉내낸 합성 본문 — macOS 안내 <p> 와 그 형제인 윈도우 안내를 포함한다
# (실측 구조: macOS 문단이 winnote 를 함께 달고 있다).
MAIN_HTML = (
    "<html><body>\n"
    '<p class="dl-hero__winnote dl-hero__macnote">참고 — macOS(Safari) 설치: '
    "App Translocation …</p>\n"
    '<p class="dl-hero__winnote">참고 — 윈도우 설치파일: SmartScreen …</p>\n'
    + "".join('<a href="/downloads/%s">받기</a>\n' % f.replace(VER, PREV) for f in FOUR)
    + "<span>220MB</span>\n"
    + ("<b>v%s</b>\n" % PREV) * 5
    + "</body></html>\n")
DL_HTML = "<html><body><section>다운로드 페이지 %s</section></body></html>\n" % PREV


@contextlib.contextmanager
def captured():
    out, err = io.StringIO(), io.StringIO()
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
        yield out, err


class Resp(object):
    """페이크 응답 1건. clen=None → len(body) · clen=False → 헤더 생략."""

    def __init__(self, http=200, body=b"", rc=None, clen=None, stderr=""):
        self.http = http
        self.body = body
        self.rc = rc
        self.clen = clen
        self.stderr = stderr

    def resolved_rc(self):
        if self.rc is not None:
            return self.rc
        # 실측: HTTP/2 4xx 는 --fail 아래서 rc 22 가 아니라 56 으로 나오기도 한다.
        # 구현이 rc 가 아니라 %{http_code} 로 가르는 이유를 페이크도 재현한다.
        return 56 if self.http >= 400 else 0


class FakeCurl(object):
    """argv 를 해석해 실제 파일을 쓰는 페이크 curl. plan = {url: [Resp, ...]}.

    HEAD 는 `"HEAD " + url` 키로 따로 계획한다. 큐가 마르면 마지막 응답을 반복한다
    (=재시도해도 계속 같은 답을 주는 원격).
    """

    def __init__(self, plan):
        self.plan = plan
        self.calls = []

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

        if r.http >= 400:          # --fail 은 4xx/5xx 를 -o 로 흘리지 않는다
            payload = b""
        elif head:
            payload = htxt.encode()
        else:
            payload = r.body
        with open(dest, "wb") as fh:
            fh.write(payload)

        return r.resolved_rc(), "%d %d" % (r.http, len(payload)), r.stderr


class NoSleep(object):
    def __init__(self):
        self.slept = []

    def __call__(self, sec):
        self.slept.append(sec)


class FtpResp(object):
    """페이크 FTPS 응답 1건 — `subprocess.run` 반환값의 필요한 면만 흉내낸다."""

    def __init__(self, rc=0, stdout=b"", stderr=b""):
        self.returncode = rc
        self.stdout = stdout
        self.stderr = stderr


class FakeFtpCurl(object):
    """`dh.curl` 치환 — FTPS LIST 를 계획대로 돌려준다(★실 ftp_list 코드를 그대로 태운다).

    ★ftp_list 를 통째로 스텁하지 않는 이유: 이 파일이 검사하려는 결함이 **ftp_list 안**에
      있다(rc 미검사 → 실패와 빈 목록의 동일 취급). 스텁하면 수리 대상을 건너뛴 채 초록이
      나온다. 그래서 주입 지점을 한 칸 아래인 `curl` 로 내린다.
    큐가 마르면 마지막 응답을 반복한다(=재시도해도 같은 답을 주는 원격).
    """

    def __init__(self, responses):
        self.queue = list(responses)
        self.calls = 0

    def __call__(self, args, **kw):
        self.calls += 1
        return self.queue.pop(0) if len(self.queue) > 1 else self.queue[0]


def list_body(names, terminated=True):
    """FTPS LIST 본문 합성. LIST 의 각 줄은 CRLF 로 끝난다 — `terminated=False` 면
    마지막 줄이 개행 없이 끊긴 **부분 목록**(잘린 이름이 DELE 대상이 되는 형태)."""
    out = "".join("-rw-r--r-- 1 u u 1 Jan 1 00:00 %s\r\n" % n for n in names)
    if not terminated:
        out = out.rstrip("\r\n") + "-set"      # 이름 중간에서 끊긴 꼴
    return out.encode()


OLD_NAMES = [f.replace(VER, PREV) for f in FOUR]


class FtpSpy(object):
    """★음성 대조의 눈 — 업로드·삭제가 **호출되었는지**만 기록한다(실행하지 않는다)."""

    def __init__(self, delete_ok=True):
        self.puts = []
        self.deletes = []
        self.delete_ok = delete_ok      # False = DELE 가 실패하는 원격

    def put(self, e, local, remote):
        self.puts.append(remote)
        return True

    def delete(self, e, remote):
        self.deletes.append(remote)
        return self.delete_ok


def healthy_plan():
    plan = {MAIN_URL: [Resp(body=MAIN_HTML.encode())],
            DL_URL: [Resp(body=DL_HTML.encode())]}
    for f in FOUR:                      # 구자산 HEAD(용량 토큰 재계산) — 건강한 200
        plan["HEAD %s/downloads/%s" % (dh.SITE, f.replace(VER, PREV))] = [Resp(clen=230686720)]
    return plan


class DeployFetchGateTests(unittest.TestCase):

    def setUp(self):
        self._real = (dh._run_http_curl, dh.load_env, dh.curl,
                      dh.ftp_put, dh.ftp_delete, dh.sftp_put, dh.RETRY_BACKOFF_S)
        dh.RETRY_BACKOFF_S = (0.0, 0.0)                 # 테스트는 대기하지 않는다
        dh.load_env = lambda: {"FTP_HOST": "h.invalid", "FTP_USER": "u", "FTP_PASS": "p"}
        # 라이브 downloads/ 목록 — 구버전 자산 4종이 남아 있는 상태(정리 대상)
        self.ftp = FakeFtpCurl([FtpResp(stdout=list_body(OLD_NAMES))])
        dh.curl = self.ftp
        self.spy = FtpSpy()
        dh.ftp_put = self.spy.put
        dh.ftp_delete = self.spy.delete
        dh.sftp_put = lambda e, local, remote: self.fail("sftp_put 이 호출됐다(우회 업로드 경로)")
        self.assets = tempfile.mkdtemp(prefix="cys-deploy-test-")
        for f in FOUR:
            with open(os.path.join(self.assets, f), "wb") as fh:
                fh.write(b"A" * 4096)

    def tearDown(self):
        (dh._run_http_curl, dh.load_env, dh.curl,
         dh.ftp_put, dh.ftp_delete, dh.sftp_put, dh.RETRY_BACKOFF_S) = self._real
        for f in os.listdir(self.assets):
            os.remove(os.path.join(self.assets, f))
        os.rmdir(self.assets)

    def _run(self, plan, apply_=True, ftp=None):
        fake = FakeCurl(plan)
        dh._run_http_curl = fake
        if ftp is not None:
            self.ftp = ftp
            dh.curl = ftp
        argv = ["deploy-homepage.py", VER, self.assets] + (["--apply"] if apply_ else [])
        with captured() as (out, err):
            rc = dh.main(argv)
        return rc, out.getvalue() + err.getvalue(), fake

    # ── ⓐ 기준선 — 건강한 사이트에서는 업로드가 실제로 일어난다 ────────────────
    def test_01_healthy_site_uploads(self):
        """이게 깨지면 아래 '업로드 안 함' 단언들은 무조건 통과하는 헛검사가 된다."""
        rc, out, _ = self._run(healthy_plan())
        self.assertEqual(rc, 0, out)
        self.assertIn("index.html", self.spy.puts, out)
        self.assertIn("downloads/index.html", self.spy.puts, out)

    # ── ⓑ ★음성 대조 — 부분 본문이면 업로드가 **호출되지 않는다** ──────────────
    def test_02_partial_main_page_blocks_upload(self):
        """★핵심 핀 — 메인 HTML 이 잘려 오면(rc 18) 치환·업로드로 진행하지 않는다.
        구 구현은 이 부분 본문을 그대로 치환해 **잘린 홈페이지를 발행**했다."""
        plan = healthy_plan()
        body = MAIN_HTML.encode()
        plan[MAIN_URL] = [Resp(body=body[:60], rc=18, clen=len(body))]
        rc, out, fake = self._run(plan)
        self.assertEqual(rc, 1, out)
        self.assertEqual(self.spy.puts, [], "★부분 본문을 받고도 업로드했다: %s" % self.spy.puts)
        self.assertEqual(self.spy.deletes, [], "구버전 자산 삭제까지 진행했다")
        self.assertIn("fail-closed", out)
        self.assertEqual(fake.urls_called(MAIN_URL), dh.MAX_ATTEMPTS)   # 유계 재시도

    def test_03_silent_truncation_rc0_blocks_upload(self):
        """rc 0 인데 본문만 짧은 가장 음험한 형태 — Content-Length 대조가 잡는다."""
        plan = healthy_plan()
        body = MAIN_HTML.encode()
        plan[MAIN_URL] = [Resp(body=body[:60], rc=0, clen=len(body))]
        rc, out, _ = self._run(plan)
        self.assertEqual(rc, 1, out)
        self.assertEqual(self.spy.puts, [])
        self.assertIn("부분 본문", out)

    def test_04_partial_downloads_page_blocks_upload(self):
        """/downloads/ 도 업로드 대상이다 — 같은 규율을 탄다."""
        plan = healthy_plan()
        body = DL_HTML.encode()
        plan[DL_URL] = [Resp(body=body[:20], rc=18, clen=len(body))]
        rc, out, _ = self._run(plan)
        self.assertEqual(rc, 1, out)
        self.assertEqual(self.spy.puts, [])

    def test_05_http_error_page_is_not_uploaded_as_body(self):
        """--fail 이 없던 구현은 4xx 오류 페이지 본문을 '라이브 HTML' 로 알고 올릴 수 있었다."""
        plan = healthy_plan()
        plan[MAIN_URL] = [Resp(http=403)]
        rc, out, _ = self._run(plan)
        self.assertEqual(rc, 1, out)
        self.assertEqual(self.spy.puts, [])
        self.assertIn("403", out)

    # ── ⓒ 5xx 는 재시도 · 소진하면 중단(초록으로 눙치지 않는다) ─────────────────
    def test_06_transient_503_is_retried_then_blocks(self):
        plan = healthy_plan()
        plan[MAIN_URL] = [Resp(http=503)]
        rc, out, fake = self._run(plan)
        self.assertEqual(rc, 1, out)
        self.assertEqual(fake.urls_called(MAIN_URL), dh.MAX_ATTEMPTS,
                         "503 을 재시도하지 않았다")
        self.assertEqual(self.spy.puts, [])

    def test_07_503_then_200_recovers_and_uploads(self):
        """블립 1회로 배포가 죽지는 않는다 — 회복하면 정상 진행한다."""
        plan = healthy_plan()
        plan[MAIN_URL] = [Resp(http=503), Resp(body=MAIN_HTML.encode())]
        rc, out, fake = self._run(plan)
        self.assertEqual(rc, 0, out)
        self.assertIn("index.html", self.spy.puts)
        self.assertEqual(fake.urls_called(MAIN_URL), 2)
        self.assertIn("재시도 1/2", out)                 # 침묵 금지

    def test_08_4xx_is_not_retried(self):
        plan = healthy_plan()
        plan[MAIN_URL] = [Resp(http=404)]
        rc, out, fake = self._run(plan)
        self.assertEqual(rc, 1, out)
        self.assertEqual(fake.urls_called(MAIN_URL), 1, "404 를 재시도했다")

    # ── ⓓ 구자산 HEAD — 미판정이면 중단 · 확정 404 면 생략(발행은 계속) ─────────
    def test_09_old_asset_head_undetermined_blocks_upload(self):
        """구자산 크기를 확인하지 못한 채 용량 토큰을 남기고 발행하지 않는다(fail-closed)."""
        plan = healthy_plan()
        plan["HEAD %s/downloads/%s" % (dh.SITE, FOUR[0].replace(VER, PREV))] = [Resp(http=502)]
        rc, out, _ = self._run(plan)
        self.assertEqual(rc, 1, out)
        self.assertEqual(self.spy.puts, [], "미판정 상태로 업로드했다")
        self.assertIn("미판정", out)

    def test_10_old_asset_absent_404_skips_token_and_proceeds(self):
        """구자산이 이미 정리됐다는 **확정 답**(404)은 비교 대상 없음 = 생략이고, 배포는 계속된다."""
        plan = healthy_plan()
        for f in FOUR:
            plan["HEAD %s/downloads/%s" % (dh.SITE, f.replace(VER, PREV))] = [Resp(http=404)]
        rc, out, _ = self._run(plan)
        self.assertEqual(rc, 0, out)
        self.assertIn("치환 생략", out)
        self.assertIn("index.html", self.spy.puts)

    # ── ⓔ dry-run 은 무해하다 ───────────────────────────────────────────────
    def test_11_dry_run_never_uploads(self):
        rc, out, _ = self._run(healthy_plan(), apply_=False)
        self.assertEqual(rc, 0, out)
        self.assertEqual(self.spy.puts, [], "dry-run 이 업로드했다")
        self.assertEqual(self.spy.deletes, [])
        self.assertIn("dry-run", out)

    # ── ⓕ 삭제 단계는 여전히 fail-closed(회귀 방지) ────────────────────────────
    def test_12_macnote_edit_failure_blocks_upload(self):
        """윈도우 안내가 없는 페이지 = 삭제 시 winnote 0 → 중단. 업로드는 없다."""
        plan = healthy_plan()
        broken = ('<html><body>\n<p class="dl-hero__macnote">macOS 안내 %s</p>\n</body></html>\n'
                  % PREV)
        plan[MAIN_URL] = [Resp(body=broken.encode())]
        rc, out, _ = self._run(plan)
        self.assertEqual(rc, 1, out)
        self.assertEqual(self.spy.puts, [])


# ──────────────────────────────────────────────────────────────────────────────
# ⓖ ★MF-A — FTPS 목록: '받지 못했다'와 '비어 있다'는 다른 사건이다 (2026-08-27 3라운드)
#
#   구현은 `curl -s`(--fail 없음)로 LIST 를 받아 **rc 를 보지 않고** `.splitlines()` 했다.
#   실행 실측: 호스트 해소 실패에서 `curl rc=6` 인데 `ftp_list() → []`.
#   그 목록이 구동하는 것은 둘 — `old_versions` 보고와 **구버전 자산 삭제 루프**.
#   귀결: 목록 수신이 실패하면 구자산이 **한 건도 삭제되지 않은 채** '구버전 없음' 으로 찍히고
#   `✅ 홈페이지 배포 완료` + exit 0 이 난다. v0.13.17 '무증상 구버전 잔존' 클래스이고,
#   `verify-release-remote.py` 는 페이지 문자열만 보므로 **서버에 남은 구자산을 못 잡는다**.
#
#   ★음성 대조가 이 군의 핵심 — test_17 이 "건강하면 실제로 지운다"를 잡아둔다. 그게 없으면
#     "아무것도 안 지우고 조용히 초록"이 아래 단언들을 전부 통과한다.
# ──────────────────────────────────────────────────────────────────────────────
def _legacy_ftp_list(resp):
    """★구현 재현(테스트 전용) — rc 를 보지 않고 stdout 만 쪼개던 식.
    실패(rc≠0 · stdout 빈문자)와 '정말 비어 있음'이 **같은 `[]`** 로 접힌다."""
    return resp.stdout.decode("utf-8", "replace").splitlines()


class FtpListFailClosedTests(unittest.TestCase):

    setUp = DeployFetchGateTests.setUp
    tearDown = DeployFetchGateTests.tearDown
    _run = DeployFetchGateTests._run

    # ── 음성 대조: 건강하면 실제로 지운다 ────────────────────────────────────
    def test_17_healthy_cleanup_actually_deletes(self):
        """★기준선 — 이게 깨지면 아래 '안 지웠다' 단언들은 헛검사가 된다."""
        rc, out, _ = self._run(healthy_plan())
        self.assertEqual(rc, 0, out)
        self.assertEqual(self.spy.deletes, ["downloads/" + n for n in OLD_NAMES], out)
        self.assertIn("구버전 정리 완료", out)

    # ── 업로드 **전** 목록 실패 = 착수하지 않는다 ─────────────────────────────
    def test_18_list_failure_before_upload_blocks_everything(self):
        """★핵심 핀 — rc 6(호스트 해소 실패)을 '목록이 비었다'로 읽지 않는다.
        구판은 여기서 업로드까지 마치고 '구버전 없음' + exit 0 을 냈다."""
        dead = FtpResp(rc=6, stderr=b"curl: (6) Could not resolve host: h.invalid\n")
        self.assertEqual(_legacy_ftp_list(dead), [], "구판 재현이 틀렸다")   # 구판은 []
        rc, out, _ = self._run(healthy_plan(), ftp=FakeFtpCurl([dead]))
        self.assertEqual(rc, 1, out)
        self.assertEqual(self.spy.puts, [], "목록도 못 받고 업로드했다")
        self.assertEqual(self.spy.deletes, [])
        self.assertIn("fail-closed", out)
        self.assertIn("종료코드 6", out)                  # 침묵 금지 — 사유가 남는다
        # ★거짓 안심 문면 금지 — 구판은 여기서 "라이브 downloads/ 항목 0 · 구버전 없음" 을
        #   찍고 그대로 발행했다. 그 보고 줄 자체가 나오면 안 된다.
        self.assertNotIn("라이브 downloads/ 항목", out)
        self.assertNotIn("배포 완료", out)
        self.assertEqual(self.ftp.calls, dh.MAX_ATTEMPTS)  # 유계 재시도

    def test_19_genuinely_empty_listing_is_not_a_failure(self):
        """★반대편 — rc 0 이고 목록이 정말 비었으면 그건 **정상**이다(지울 것이 없다).
        수리가 '비었으면 무조건 중단'으로 흐르면 첫 배포·정리 직후 재실행이 죽는다."""
        rc, out, _ = self._run(healthy_plan(), ftp=FakeFtpCurl([FtpResp(stdout=b"")]))
        self.assertEqual(rc, 0, out)
        self.assertIn("index.html", self.spy.puts)
        self.assertEqual(self.spy.deletes, [], "빈 목록인데 뭔가를 지웠다")
        self.assertIn("구버전 정리 완료(대상 0건", out)

    def test_20_partial_listing_blocks_and_never_deletes_truncated_name(self):
        """부분 목록 — 잘린 마지막 줄(`…_x64-set`)을 DELE 대상으로 삼으면 **진짜 파일은
        살아남고** 없는 이름만 지우려다 만다. 그 전에 멈춘다."""
        cut = FtpResp(stdout=list_body(OLD_NAMES, terminated=False))
        rc, out, _ = self._run(healthy_plan(), ftp=FakeFtpCurl([cut]))
        self.assertEqual(rc, 1, out)
        self.assertEqual(self.spy.puts, [])
        self.assertEqual(self.spy.deletes, [])
        self.assertIn("부분 목록", out)
        self.assertNotIn("-set\n", "".join(self.spy.deletes) + "\n")

    # ── 업로드 **후**(정리 단계) 실패 = 롤백 없이 loud + 비영 ──────────────────
    def test_21_cleanup_list_failure_is_loud_nonzero_without_rollback(self):
        """★비가역 이후의 실패 — 업로드·페이지 발행은 **되돌리지 않는다**(되돌리면 홈페이지가
        더 나빠진다). 대신 '정리 미완'을 명시하고 비영 종료한다. 조용한 성공 선언 금지."""
        ftp = FakeFtpCurl([FtpResp(stdout=list_body(OLD_NAMES)),      # 계획 단계: 정상
                           FtpResp(rc=7, stderr=b"curl: (7) Failed to connect\n")])
        rc, out, _ = self._run(healthy_plan(), ftp=ftp)
        self.assertEqual(rc, 1, out)
        self.assertIn("index.html", self.spy.puts, "발행이 일어나지 않았다(픽스처 오류)")
        self.assertEqual(self.spy.deletes, [], "목록도 못 받고 삭제를 시작했다")
        self.assertIn("정리는 미완", out)
        self.assertIn("되돌리지 않는다", out)
        self.assertNotIn("✅", out)                    # ★'완료' 단언 금지

    def test_22_delete_failure_is_reported_and_nonzero(self):
        """DELE 가 실패하면 구자산이 남는다 — ✗ 만 찍고 exit 0 으로 끝내지 않는다."""
        self.spy.delete_ok = False
        rc, out, _ = self._run(healthy_plan())
        self.assertEqual(rc, 1, out)
        self.assertEqual(len(self.spy.deletes), len(OLD_NAMES))   # 시도는 전건 했다
        self.assertIn("삭제 실패", out)
        self.assertIn("정리는 미완", out)
        self.assertNotIn("✅", out)


class FtpListContractTests(unittest.TestCase):
    """ftp_list 자체의 계약 — 종단 없이 반환값만 본다."""

    def _e(self):
        return {"FTP_HOST": "h.invalid", "FTP_USER": "u", "FTP_PASS": "p"}

    def test_23_empty_and_failure_are_different_values(self):
        """★계약의 본체 — `([], None)` 과 `(None, err)` 은 **다른 값**이다."""
        with captured():
            lines, err = dh.ftp_list(self._e(), "downloads/",
                                     _runner=lambda a, **k: FtpResp(stdout=b""), _sleep=NoSleep())
        self.assertEqual((lines, err), ([], None))
        with captured():
            lines, err = dh.ftp_list(self._e(), "downloads/",
                                     _runner=lambda a, **k: FtpResp(rc=6), _sleep=NoSleep())
        self.assertIsNone(lines, "실패를 '빈 목록'으로 돌려줬다")
        self.assertIn("종료코드 6", err)

    def test_24_list_retry_is_bounded(self):
        fake = FakeFtpCurl([FtpResp(rc=7)])
        s = NoSleep()
        with captured():
            lines, err = dh.ftp_list(self._e(), "downloads/", _runner=fake, _sleep=s)
        self.assertIsNone(lines)
        self.assertEqual(fake.calls, dh.MAX_ATTEMPTS)
        self.assertEqual(len(s.slept), dh.MAX_ATTEMPTS - 1)
        self.assertIn("시도 %d회 소진" % dh.MAX_ATTEMPTS, err)

    def test_25_list_recovers_on_retry(self):
        fake = FakeFtpCurl([FtpResp(rc=7), FtpResp(stdout=list_body(OLD_NAMES))])
        with captured() as (out, _err):
            lines, err = dh.ftp_list(self._e(), "downloads/", _runner=fake, _sleep=NoSleep())
        self.assertIsNone(err)
        self.assertEqual(len(lines), len(OLD_NAMES))
        self.assertIn("LIST 재시도 1/2", out.getvalue())      # 침묵 금지


class FetchContractTests(unittest.TestCase):
    """수신 검증의 뼈대를 문자로 못박는다(누가 지워도 여기서 걸린다)."""

    def test_13_required_curl_flags(self):
        a = dh.http_curl_argv("https://example.invalid/x", False, "/tmp/b", "/tmp/h")
        for flag in ("--fail", "--show-error", "-D", "-o", "-w", "--max-time"):
            self.assertIn(flag, a, "%s 가 빠졌다" % flag)
        self.assertIn("%{http_code} %{size_download}", a)
        self.assertEqual(a[-1], "https://example.invalid/x")
        self.assertNotIn("-I", a)
        self.assertIn("-I", dh.http_curl_argv("https://example.invalid/x", True, "/tmp/b", "/tmp/h"))

    def test_14_transient_classifier_boundary(self):
        for s in (429, 500, 502, 503, 504):
            self.assertTrue(dh.is_transient_status(s))
        for s in (400, 401, 403, 404, 410, 428, 451):
            self.assertFalse(dh.is_transient_status(s))

    def test_15_retry_is_bounded(self):
        url = "https://example.invalid/p"
        fake = FakeCurl({url: [Resp(body=b"x" * 10, rc=18, clen=999)]})
        s = NoSleep()
        with captured():
            r = dh.http_fetch(url, _runner=fake, _sleep=s)
        self.assertEqual(fake.urls_called(url), dh.MAX_ATTEMPTS)
        self.assertEqual(len(s.slept), dh.MAX_ATTEMPTS - 1)
        self.assertIn("시도 %d회 소진" % dh.MAX_ATTEMPTS, r.err)
        self.assertGreaterEqual(len(dh.RETRY_BACKOFF_S), dh.MAX_ATTEMPTS - 1,
                                "백오프 표가 시도 횟수보다 짧다")

    def test_16_fetch_returns_error_not_partial_text(self):
        """★fetch() 는 부분 본문을 **본문으로 돌려주지 않는다** — 호출부가 못 쓰게 한다."""
        url = "https://example.invalid/p"
        fake = FakeCurl({url: [Resp(body=b"<html>jjj", rc=0, clen=9999)]})
        with captured():
            text, err = dh.fetch(url, _runner=fake, _sleep=NoSleep())
        self.assertEqual(text, "", "부분 본문이 호출부로 새어 나갔다")
        self.assertIsNotNone(err)
        self.assertIn("부분 본문", err)


if __name__ == "__main__":
    unittest.main(verbosity=2)
