#!/usr/bin/env python3
"""홈페이지(www.cysinsight.com) 배포 — 자산 업로드 + 페이지 버전 범프 + 구버전 정리.

★설계 근거 (2026-07-29 실측):
  · 배포처는 Hostinger 공유호스팅(LiteSpeed). 자격은 `~/.cys/hostinger-ftp.env`(600).
    프로토콜은 **FTPS(명시적 AUTH SSL, 포트 21)** 이고, 인증서가 `*.hstgr.io` 라
    IP 접속 시 SAN 불일치 → `--ftp-ssl -k` 가 필요하다.
  · FTP 루트가 곧 docroot 다(`/index.html` · `/downloads/`). `domains/...` 접두 불요.
  · ★**로컬 `cys-homepage` 리포는 라이브보다 뒤처져 있다**(실측: 로컬 0.13.18 vs 라이브 0.14.4,
    미커밋 511건). 그래서 **로컬에서 빌드해 올리면 라이브가 퇴행한다.**
    → 이 스크립트는 **라이브 페이지를 받아 외과적으로 버전·용량 토큰만 치환**한다.
  · 용량 표기 관례는 **MiB 버림**이다(실측: 231,644,695B → 라이브 220MB · 반올림이면 221).
  · 자산 보존 정책은 **최신 1개**(v0.14.1 인계 기록: "0.14.0 자산 rm").
  · ★**메인페이지 macOS(Safari) 안내 문단은 업로드 때마다 삭제한다**(2026-08-24 오너 지시).
    같은 외과적 방식 — class 토큰 `dl-hero__macnote` 를 가진 <p> 하나만 도려낸다.
    바로 아래 형제인 윈도우 안내(`dl-hero__winnote`)는 릴리스 체크리스트 ⓐ 필수 잔존 항목이라
    **반드시 살려둔다**. 세목은 아래 strip_main_macnote() 위 주석 참조.

  · ★**라이브 페이지 수신은 fail-closed 다**(2026-08-27). 이 스크립트가 받은 HTML 이 곧
    업로드될 본문이라, 부분 본문을 받으면 **잘린 홈페이지가 그대로 발행된다.** 그래서
    `--fail`·종료코드·Content-Length 대조로 수신을 검증하고, 유계 재시도(3회)를 소진하고도
    불확실하면 **치환·업로드로 진행하지 않고 중단**한다. 세목은 아래 '수신 검증 계약' 절.

기본은 dry-run. `--apply` 를 줘야 실제로 올린다.

사용:
  python3 scripts/deploy-homepage.py 0.14.5 <자산디렉터리>           # 계획만
  python3 scripts/deploy-homepage.py 0.14.5 <자산디렉터리> --apply   # 집행
  python3 scripts/deploy-homepage.py --self-test                     # 삭제 로직 셀프테스트(무접촉)

종료코드: 0 = 정상(dry-run 포함) · 1 = 중단(수신 미판정 · 원격 확정 오류 · fail-closed 위반 ·
          업로드 실패) · 2 = 사용법

동반 회귀 테스트: `python3 scripts/tests/test_deploy_homepage_fetch.py`
  (합성 페이크 curl · 네트워크 불요 · ★부분 본문 수신 시 업로드가 **호출되지 않는지** 음성 대조)
"""
import os
import re
import subprocess
import sys
import tempfile
import time

ENV = os.path.expanduser("~/.cys/hostinger-ftp.env")
SITE = "https://www.cysinsight.com"
UA = "Mozilla/5.0"


def load_env():
    if not os.path.exists(ENV):
        raise SystemExit("::error::자격 파일 없음: %s" % ENV)
    out = {}
    for line in open(ENV, encoding="utf-8"):
        line = line.strip()
        if line.startswith("export ") and "=" in line:
            k, v = line[len("export "):].split("=", 1)
            out[k.strip()] = v.strip().strip('"').strip("'")
    for k in ("FTP_HOST", "FTP_USER", "FTP_PASS"):
        if not out.get(k):
            raise SystemExit("::error::%s 미설정" % k)
    return out


def curl(args, **kw):
    return subprocess.run(["curl", "-s", "--max-time", "600"] + args,
                          capture_output=True, **kw)


def ftp_base(e):
    return "ftp://%s/" % e["FTP_HOST"]


def ftp_auth(e):
    return ["-k", "--ftp-ssl", "-u", "%s:%s" % (e["FTP_USER"], e["FTP_PASS"])]


def ftp_list(e, path):
    r = curl(ftp_auth(e) + [ftp_base(e) + path])
    return r.stdout.decode("utf-8", "replace").splitlines()


def sftp_put(e, local, remote):
    """FTPS 450 폴백 — Hostinger 는 해시밀도 높은 파일(pack-manifest.json)을 FTP 에서
    "Link to file server lost"(450)로 차단한다(v0.13.3·v0.14.7 실측). SFTP(65002·같은 계정)가
    정공법. ★docroot 차이: FTP 루트=docroot 직행 / SFTP 루트=계정 홈 → `domains/.../public_html/`
    접두 필요. 암호는 expect 로 구동(셸 보간 금지 — $env 로만)."""
    sftp_root = "domains/cysinsight.com/public_html/"
    with tempfile.TemporaryDirectory() as td:
        batch = os.path.join(td, "b")
        with open(batch, "w") as fh:      # ★닫고 나서 expect 에 넘긴다(덜 쓰인 배치 방지)
            fh.write("put %s %s%s\n" % (local, sftp_root, remote))
        exp = os.path.join(td, "e")
        with open(exp, "w") as fh:
            fh.write(
                'set timeout 300\n'
                'spawn sftp -P %s -o BatchMode=no -o StrictHostKeyChecking=accept-new '
                '-b %s %s@%s\n'
                'expect {\n'
                '  -re "(?i)password" { send -- "$env(CYS_SFTP_PASS)\\r"; exp_continue }\n'
                '  eof {}\n'
                '}\n'
                'catch wait result\n'
                'exit [lindex $result 3]\n'
                % (e.get("FTP_SFTP_PORT", "65002"), batch, e["FTP_USER"], e["FTP_HOST"]))
        env = dict(os.environ, CYS_SFTP_PASS=e["FTP_PASS"])
        r = subprocess.run(["/usr/bin/expect", exp], capture_output=True, env=env)
        return r.returncode == 0


def ftp_put(e, local, remote):
    r = curl(ftp_auth(e) + ["-T", local, ftp_base(e) + remote])
    if r.returncode == 0:
        return True
    # FTPS 실패(450 등) → SFTP 폴백(정공법 — 함수 docstring 참조)
    return sftp_put(e, local, remote)


def ftp_delete(e, remote):
    d = os.path.dirname(remote)
    r = curl(ftp_auth(e) + ["-Q", "-DELE %s" % os.path.basename(remote),
                            ftp_base(e) + (d + "/" if d else "")])
    return r.returncode == 0


# ── ★수신 검증 계약 (2026-08-27 · '잘린 홈페이지 발행' 사고 봉합) ──────────────────
# ★배경: 이 파일의 fetch() 는 라이브 페이지를 `curl -s`(--fail 없음)로 받아 **종료코드도
#   수신 바이트수도 보지 않고** 그 문자열 위에서 버전·용량 토큰을 치환한 뒤 **그대로
#   업로드**했다(main() → strip_main_macnote → ftp_put). 전송이 도중에 끊기면 **잘린
#   홈페이지가 그대로 발행된다.** 같은 무방비가 구자산 HEAD(용량 토큰 재계산)에도 있어,
#   잘린 헤더의 Content-Length 가 페이지에 **틀린 용량**으로 박힐 수 있었다.
#
# ★원칙 — 배포는 **비가역**이다. 그래서 검증기(`scripts/verify-release-remote.py`)보다
#   한 칸 더 엄격하다:
#     · 수신 검증(rc · --fail · Content-Length 대조) 없이는 **치환·업로드로 진행하지 않는다**
#     · 유계 재시도를 소진하고도 불확실하면 **중단**하고 사유를 loud 로 남긴다(조용한 진행 금지)
#     · 429·5xx = '원격이 지금 대답을 못 한다' → 재시도 대상
#       4xx(429 제외) = 원격의 **확정 답** → 재시도하지 않는다(같은 답이 올 뿐)
#   판정 3분류(자산/원격 확정 오류/미판정)는 검증기와 같은 어휘를 쓴다 — 두 도구의 문면이
#   갈리면 사람이 같은 사건을 다르게 읽는다.
MAX_ATTEMPTS = 3                 # ★유계 — 최초 1회 + 재시도 2회 (`retry-loop-needs-stop-condition`)
RETRY_BACKOFF_S = (2.0, 5.0)     # len ≥ MAX_ATTEMPTS-1 (부족하면 마지막 값 반복)
HTTP_TIMEOUT = 120               # 1회 상한(초) — 페이지·HEAD 용

_CL_RE = re.compile(r"(?im)^\s*content-length:\s*(\d+)\s*$")


class HttpResult(object):
    """HTTP 수신 1건의 결과.

      · err != None → **미판정**: 수신이 완결되지 못했거나 원격이 일시적으로 대답을 못 했다.
                      ★이 상태에서 본문을 쓰면 안 된다.
      · err == None → 수신 완결 + 원격의 확정 답. 그 다음 판정은 status/text 로 한다.
    """

    __slots__ = ("err", "status", "text", "nbytes", "clen", "tries")

    def __init__(self, err=None, status=0, text="", nbytes=0, clen=None):
        self.err = err
        self.status = status
        self.text = text
        self.nbytes = nbytes
        self.clen = clen
        self.tries = 1


def is_transient_status(status):
    """유계 재시도 대상인가 — **429 와 5xx**('지금 대답을 못 한다'). 4xx 는 확정 답이다."""
    return status == 429 or status >= 500


def http_curl_argv(url, head, dest, hdr):
    """수신 검증의 뼈대 — `--fail`·`--show-error`·`-D`·`-w` 넷이 계약이다.

      --fail        : 4xx/5xx 오류 페이지를 '본문'으로 착각해 업로드 재료로 쓰지 않는다
      --show-error  : `-s` 로 눌린 오류 문면을 되살린다(침묵 금지)
      -D <hdr>      : Content-Length 대조용 헤더 원본
      -w %{http_code} %{size_download} : rc 에 의존하지 않는 상태·바이트수 관측
    """
    a = ["curl", "-s", "--show-error", "--fail", "-A", UA,
         "--max-time", str(HTTP_TIMEOUT), "-D", hdr, "-o", dest,
         "-w", "%{http_code} %{size_download}"]
    if head:
        a.append("-I")
    return a + [url]


def _run_http_curl(argv):
    p = subprocess.run(argv, capture_output=True)
    return (p.returncode,
            p.stdout.decode("utf-8", "replace"),
            p.stderr.decode("utf-8", "replace"))


def _read_bytes(path):
    try:
        with open(path, "rb") as fh:
            return fh.read()
    except OSError:
        return b""


def _http_attempt(url, head, runner):
    with tempfile.TemporaryDirectory() as td:
        dest, hdr = os.path.join(td, "body"), os.path.join(td, "hdr")
        rc, w_out, serr = runner(http_curl_argv(url, head, dest, hdr))
        try:
            status = int((w_out or "").split()[0])
        except (IndexError, ValueError):
            status = 0
        header_text = _read_bytes(hdr).decode("utf-8", "replace")
        m = _CL_RE.findall(header_text)
        clen = int(m[-1]) if m else None            # 리다이렉트 체인 대비 — **마지막** 값
        blob = _read_bytes(dest)
        nbytes = len(blob)
        if status >= 400:
            if is_transient_status(status):
                return HttpResult(err="원격 일시 불응답(HTTP %d)" % status,
                                  status=status, nbytes=nbytes, clen=clen)
            return HttpResult(status=status, nbytes=nbytes, clen=clen)
        if rc != 0:
            tail = ([l for l in (serr or "").splitlines() if l.strip()] or [""])[-1].strip()
            return HttpResult(err="curl 종료코드 %d%s" % (rc, (" · " + tail[:200]) if tail else ""),
                              status=status, nbytes=nbytes, clen=clen)
        if head:
            # HEAD 는 본문이 없다 — 헤더가 산출물이고 Content-Length 는 '선언된 크기'다.
            return HttpResult(status=status, text=header_text, nbytes=nbytes, clen=clen)
        # ★수신 바이트수 vs 선언값 대조 — rc 만으로는 못 잡는 조기 종료가 여기서 걸린다.
        if clen is not None and clen != nbytes:
            return HttpResult(err="수신 %dB ≠ Content-Length %dB (부분 본문)" % (nbytes, clen),
                              status=status, nbytes=nbytes, clen=clen)
        return HttpResult(status=status, text=blob.decode("utf-8", "replace"),
                          nbytes=nbytes, clen=clen)


def http_fetch(url, head=False, attempts=MAX_ATTEMPTS, _runner=None, _sleep=None):
    """유계 재시도로 URL 을 받는다 → HttpResult. ★상한은 `range(attempts)` 가 강제한다."""
    runner = _runner or _run_http_curl
    sleeper = _sleep or time.sleep
    attempts = max(1, int(attempts))
    tries = 0
    r = None
    for i in range(attempts):                      # ← 유계
        if i:
            delay = RETRY_BACKOFF_S[min(i - 1, len(RETRY_BACKOFF_S) - 1)]
            print("  ↻ 재시도 %d/%d (%.0fs 후) ← %s | %s" % (i, attempts - 1, delay, r.err, url))
            sleeper(delay)
        r = _http_attempt(url, head, runner)
        tries = i + 1
        if not r.err:
            break
    r.tries = tries
    if r.err:
        r.err = "%s [시도 %d회 소진]" % (r.err, tries)
    return r


def fetch(url, _runner=None, _sleep=None):
    """라이브 페이지 GET — **fail-closed**. 반환 `(text, err)`.

    ★err != None 이면 그 본문 위에서 **아무것도 하지 마라**(치환도 업로드도). 이 함수의
      반환값이 업로드될 HTML 의 출처이기 때문에, 여기서 통과시킨 부분 본문은 곧바로
      **잘린 홈페이지 발행**이 된다.
    """
    r = http_fetch(url, _runner=_runner, _sleep=_sleep)
    if r.err:
        return "", "미판정 — " + r.err
    if not (200 <= r.status < 300):
        return "", "원격 확정 오류(HTTP %d)" % r.status
    if not r.text:
        return "", "빈 응답(HTTP %d · 수신 %dB)" % (r.status, r.nbytes)
    return r.text, None


def mib(n):
    """용량 표기 = MiB **버림**(반올림 아님). 실측: 231,644,695B → 라이브 표기 220MB
    (반올림이면 221). x64/exe 는 두 방식이 같아 구분되지 않으므로 aarch64 가 판별자다."""
    return int(n // 1024 // 1024)


# ── ★메인페이지 전용 · macOS(Safari) 안내 문단 삭제 (2026-08-24 오너 지시) ──
# 지시: 「홈페이지 업로드 시 www.cysinsight.com **메인페이지**에서 macOS(Safari) 안내 문단을 삭제」.
#
# ★대상 특정 — class 토큰 `dl-hero__macnote` 를 가진 <p> 하나.
#   라이브 실측(2026-08-24 · 읽기 전용 GET, 배포 아님):
#     /  →  dl-hero__macnote 1건 · dl-hero__winnote 2건 · "App Translocation" 1건
#
# ★★함정(반드시 읽어라) — 그 macOS 문단의 class 는 `"dl-hero__winnote dl-hero__macnote"` 로
#   **winnote 를 함께 달고** 있고, 바로 다음 형제가 진짜 윈도우 안내
#   `<p class="dl-hero__winnote">참고 — 윈도우 설치파일: … SmartScreen …</p>` 다.
#   그래서 winnote 로 매칭하면 **둘 다** 지워져 릴리스 체크리스트 ⓐ(윈도우/Defender 안내 잔존)를
#   위반한다. 오직 macnote 토큰으로만 특정한다. (--self-test 케이스 ⓒ 가 이 회귀를 적색으로 잡는다.)
#
# ★범위 — 지시는 **메인페이지 한정**이다. `/downloads/` 의
#   data-cys-release-marker="macos-install-guidance-v1" · "…windows-defender-guidance-v2" 섹션은
#   건드리지 않는다(이 함수는 메인 HTML 에만 적용하고 dl_html 은 통과시킨다).
#
# ★fail-closed — "지웠다고 보고하고 실제로는 안 지움"(조용한 no-op)이 최악이다. 그래서
#   ① macnote 2건 이상 = 오류 중단  ② 삭제 후 macnote 0건 재확인  ③ winnote 정확히 1건 잔존 단언.
# ★멱등 — 다음 릴리스의 라이브에는 macnote 가 이미 없다. 그때 죽으면 배포가 막히므로
#   "이미 없음"은 **정상 통과**로 다루되, 그 경우에도 winnote 잔존 1건은 똑같이 단언한다.
#   두 상태(removed / already-absent)는 로그 문구로 명확히 구분한다.
MACNOTE_CLASS = "dl-hero__macnote"
WINNOTE_CLASS = "dl-hero__winnote"
WINNOTE_EXPECT_AFTER = 1
TRANSLOCATION_TOKEN = "App Translocation"

_P_OPEN_RE = re.compile(r"<p\b[^>]*>", re.IGNORECASE)
_CLASS_ATTR_RE = re.compile(r"""\bclass\s*=\s*(?:"([^"]*)"|'([^']*)')""", re.IGNORECASE)


class MacnoteEditError(Exception):
    """메인페이지 macOS 안내 삭제 단계의 fail-closed 위반 — 배포를 중단시킨다."""


def _tag_class_tokens(tag):
    """여는 태그에서 class 토큰 목록을 뽑는다. 부분일치(`dl-hero__macnote-extra`·
    `x-dl-hero__macnote`)를 막으려고 정규식 \\b 가 아니라 **공백 분해 후 완전일치**로 판정한다."""
    m = _CLASS_ATTR_RE.search(tag)
    if not m:
        return []
    return (m.group(1) if m.group(1) is not None else m.group(2)).split()


def find_class_paragraphs(html, cls):
    """class 토큰 `cls` 를 가진 <p>…</p> 의 (시작, 끝) 목록. HTML 은 <p> 중첩을 허용하지 않으므로
    여는 태그 뒤 **최초 </p>** 가 자기 닫힘이다(내부 <code> 는 영향 없다)."""
    spans = []
    for m in _P_OPEN_RE.finditer(html):
        if cls not in _tag_class_tokens(m.group(0)):
            continue
        close = html.find("</p>", m.end())
        if close < 0:
            raise MacnoteEditError("class=%s 인 <p>(offset %d)의 </p> 를 찾지 못했다" % (cls, m.start()))
        spans.append((m.start(), close + len("</p>")))
    return spans


def assert_winnote_survives(html, where):
    """★윈도우 안내 생존 단언 — 릴리스 체크리스트 ⓐ 의 필수 잔존 항목이다.
    문자열 출현수와 <p> 요소수를 **둘 다** 정확히 1로 단언한다(0=오탈락 · 2+=macOS 문단 미삭제)."""
    n_str = html.count(WINNOTE_CLASS)
    n_el = len(find_class_paragraphs(html, WINNOTE_CLASS))
    if n_str != WINNOTE_EXPECT_AFTER or n_el != WINNOTE_EXPECT_AFTER:
        raise MacnoteEditError(
            "[%s] 윈도우 안내 잔존 단언 실패 — %s 문자열 %d개·<p> %d개 (기대 각 %d개). "
            "winnote 로 매칭해 윈도우 안내까지 지운 회귀를 의심하라."
            % (where, WINNOTE_CLASS, n_str, n_el, WINNOTE_EXPECT_AFTER))
    return n_el


def strip_main_macnote(html):
    """메인페이지 HTML 에서 macOS(Safari) 안내 <p> 를 제거해 (새 HTML, 보고dict) 를 돌려준다.

    ★순수함수 — 네트워크·파일·전역 상태를 만지지 않는다(그래서 --self-test 가 합성 표본으로
      전 분기를 시험할 수 있다). 위반 시 MacnoteEditError 를 던져 배포를 **중단**시킨다."""
    win_before = html.count(WINNOTE_CLASS)
    spans = find_class_paragraphs(html, MACNOTE_CLASS)

    if len(spans) > 1:
        raise MacnoteEditError(
            "%s <p> 가 %d개다(기대 0개 또는 1개) — 페이지 구조가 바뀌었다. 임의 삭제 대신 중단한다."
            % (MACNOTE_CLASS, len(spans)))

    if not spans:
        # ── 멱등 경로: 이미 지워진 라이브. 정상 통과 — 단, 잔존 단언은 동일하게 건다.
        n_win = assert_winnote_survives(html, "이미 없음(멱등)")
        return html, {"state": "already-absent", "removed": 0, "removed_len": 0,
                      "preview": "", "winnote_before": win_before, "winnote_after": n_win,
                      "translocation_after": html.count(TRANSLOCATION_TOKEN)}

    start, end = spans[0]
    removed = html[start:end]
    # 빈 줄이 남지 않도록 줄머리 들여쓰기와 뒤따르는 개행 하나까지 흡수한다.
    ls = start
    while ls > 0 and html[ls - 1] in " \t":
        ls -= 1
    if ls == 0 or html[ls - 1] == "\n":
        start = ls
    te = end
    while te < len(html) and html[te] in " \t":
        te += 1
    if te < len(html) and html[te] == "\n":
        end = te + 1

    new = html[:start] + html[end:]

    # ── fail-closed 사후 검증 ── (조용한 no-op 차단)
    left = len(find_class_paragraphs(new, MACNOTE_CLASS))
    if left != 0:
        raise MacnoteEditError("삭제 후에도 %s <p> 가 %d개 남았다 — 삭제가 실제로 일어나지 않았다"
                               % (MACNOTE_CLASS, left))
    if len(new) >= len(html):
        raise MacnoteEditError("삭제 후 HTML 이 줄지 않았다(%d → %d) — 조용한 no-op"
                               % (len(html), len(new)))
    n_win = assert_winnote_survives(new, "삭제 후")

    preview = " ".join(removed.split())[:40]
    return new, {"state": "removed", "removed": 1, "removed_len": len(removed),
                 "preview": preview, "winnote_before": win_before, "winnote_after": n_win,
                 "translocation_after": new.count(TRANSLOCATION_TOKEN)}


def print_macnote_report(rep):
    """dry-run·집행 공통 — 사람이 읽을 수 있게 '무엇을 지울 것인지'를 찍는다."""
    if rep["state"] == "removed":
        print("메인페이지 macOS(Safari) 안내 삭제: ✓ 제거 대상 1개 (%d자)" % rep["removed_len"])
        print("  지울 내용 첫 40자: %s…" % rep["preview"])
        print("  잔존 단언: %s %d개 → %d개 (윈도우 안내 생존 · 기대 %d) · '%s' 잔여 %d개"
              % (WINNOTE_CLASS, rep["winnote_before"], rep["winnote_after"],
                 WINNOTE_EXPECT_AFTER, TRANSLOCATION_TOKEN, rep["translocation_after"]))
    else:
        print("메인페이지 macOS(Safari) 안내 삭제: — 이미 없음(멱등 통과 · 삭제할 것이 없다)")
        print("  잔존 단언: %s %d개 (윈도우 안내 생존 · 기대 %d) · '%s' 잔여 %d개"
              % (WINNOTE_CLASS, rep["winnote_after"], WINNOTE_EXPECT_AFTER,
                 TRANSLOCATION_TOKEN, rep["translocation_after"]))


# ── 셀프테스트 — 합성 표본으로 삭제 로직 전 분기를 시험한다(네트워크 무접촉) ──
# 이 레인의 기존 관례는 `scripts/tests/test_*.py` 이지만, 이번 작업 반경이 이 파일 하나로
# 못박혀 있어 `--self-test` 플래그로 동봉한다(실행: python3 scripts/deploy-homepage.py --self-test).
_S_MAC_AND_WIN = (
    '<div class="dl-hero__copy">\n'
    '        <ul class="dl-hero__bullets"><li>역할별 편성</li></ul>\n'
    '        <p class="dl-hero__winnote dl-hero__macnote">참고 — macOS(Safari) 설치: Safari로 받은 '
    'DMG는 macOS 보안 기능(App Translocation) 때문에 첫 실행 시 "손상됨" 안내가 나타날 수 있습니다. '
    '<code>xattr -d com.apple.quarantine /Applications/cys.app</code>를 실행한 뒤 다시 여세요.</p>\n'
    '        <p class="dl-hero__winnote">참고 — 윈도우 설치파일: SmartScreen "알 수 없는 게시자" '
    '경고가 뜰 수 있습니다("추가 정보 → 실행"으로 진행).</p>\n'
    '      </div>\n')
_S_WIN_ONLY = (
    '<div class="dl-hero__copy">\n'
    '        <ul class="dl-hero__bullets"><li>역할별 편성</li></ul>\n'
    '        <p class="dl-hero__winnote">참고 — 윈도우 설치파일: SmartScreen "알 수 없는 게시자" '
    '경고가 뜰 수 있습니다("추가 정보 → 실행"으로 진행).</p>\n'
    '      </div>\n')
_S_TWO_MAC = _S_MAC_AND_WIN.replace(
    '      </div>\n',
    '        <p class="dl-hero__macnote">참고 — macOS 중복 문단(이상 상태).</p>\n      </div>\n')
_S_MAC_NO_WIN = (
    '<div class="dl-hero__copy">\n'
    '        <p class="dl-hero__macnote">참고 — macOS(Safari) 설치: App Translocation …</p>\n'
    '      </div>\n')
_S_DECOY = _S_WIN_ONLY.replace(
    '        <p class="dl-hero__winnote">',
    '        <p class="dl-hero__macnote-extra x-dl-hero__macnote">유사 클래스 미끼 문단</p>\n'
    '        <p class="dl-hero__winnote">')


def _buggy_strip_by_winnote(html):
    """★회귀 재현용(테스트 전용) — 실수로 winnote 로 매칭하는 구현. 라이브에서는 macOS 문단이
    winnote 를 함께 달고 있어 **윈도우 안내까지** 지운다. 잔존 단언이 이걸 잡아야 한다."""
    out = html
    for s, e in reversed(find_class_paragraphs(html, WINNOTE_CLASS)):
        out = out[:s] + out[e:]
    return out


def self_test():
    tally = {"pass": 0, "fail": 0}

    def ok(name, cond, detail=""):
        tally["pass" if cond else "fail"] += 1
        print(("PASS " if cond else "FAIL ") + name + (" | " + detail if detail else ""))

    # ⓐ 정상 — 둘 다 있음 → macnote 만 삭제, winnote 생존
    try:
        new, rep = strip_main_macnote(_S_MAC_AND_WIN)
        ok("ⓐ 정상: macnote 만 삭제·winnote 생존",
           rep["state"] == "removed" and rep["removed"] == 1
           and new.count(MACNOTE_CLASS) == 0 and new.count(WINNOTE_CLASS) == 1
           and "윈도우 설치파일" in new and TRANSLOCATION_TOKEN not in new
           and "역할별 편성" in new,
           "제거 %d자 · winnote %d→%d · '\\n\\n' 잔여 %d"
           % (rep["removed_len"], rep["winnote_before"], rep["winnote_after"], new.count("\n\n")))
    except MacnoteEditError as ex:
        ok("ⓐ 정상: macnote 만 삭제·winnote 생존", False, "예기치 못한 오류: %s" % ex)

    # ⓑ 멱등 — 이미 삭제된 라이브에 재실행
    try:
        new, rep = strip_main_macnote(_S_WIN_ONLY)
        ok("ⓑ 멱등: 이미 없음은 정상 통과",
           rep["state"] == "already-absent" and rep["removed"] == 0
           and new == _S_WIN_ONLY and rep["winnote_after"] == 1,
           "state=%s · HTML 무변경=%s" % (rep["state"], new == _S_WIN_ONLY))
    except MacnoteEditError as ex:
        ok("ⓑ 멱등: 이미 없음은 정상 통과", False, "예기치 못한 오류: %s" % ex)

    # ⓒ 회귀 — winnote 로 매칭해 윈도우 안내를 지우면 **반드시 적색**
    for label, sample in (("이미 삭제된 페이지", _S_WIN_ONLY), ("라이브 구조", _S_MAC_AND_WIN)):
        broken = _buggy_strip_by_winnote(sample)
        try:
            assert_winnote_survives(broken, "회귀 시험")
            ok("ⓒ 회귀 적색(%s): winnote 삭제를 단언이 잡는다" % label, False,
               "★단언이 통과해버렸다 — 윈도우 안내가 조용히 사라진다")
        except MacnoteEditError as ex:
            ok("ⓒ 회귀 적색(%s): winnote 삭제를 단언이 잡는다" % label, True,
               "적색 확인 → %s" % ex)

    # ⓓ 이상 상태 — macnote 2개면 오류 중단
    try:
        strip_main_macnote(_S_TWO_MAC)
        ok("ⓓ 이상: macnote 2개는 오류 중단", False, "★오류 없이 통과해버렸다")
    except MacnoteEditError as ex:
        ok("ⓓ 이상: macnote 2개는 오류 중단", True, str(ex))

    # ⓔ fail-closed 보강 — 삭제하면 winnote 가 0이 되는 페이지는 중단(윈도우 안내 소실 차단)
    try:
        strip_main_macnote(_S_MAC_NO_WIN)
        ok("ⓔ fail-closed: winnote 0 되면 중단", False, "★오류 없이 통과해버렸다")
    except MacnoteEditError as ex:
        ok("ⓔ fail-closed: winnote 0 되면 중단", True, str(ex))

    # ⓕ 부분일치 방어 — `dl-hero__macnote-extra`·`x-dl-hero__macnote` 는 대상이 아니다
    try:
        new, rep = strip_main_macnote(_S_DECOY)
        ok("ⓕ 부분일치 방어: 유사 클래스는 삭제하지 않는다",
           rep["state"] == "already-absent" and "유사 클래스 미끼 문단" in new,
           "state=%s · 미끼 생존=%s" % (rep["state"], "유사 클래스 미끼 문단" in new))
    except MacnoteEditError as ex:
        ok("ⓕ 부분일치 방어: 유사 클래스는 삭제하지 않는다", False, "예기치 못한 오류: %s" % ex)

    total = tally["pass"] + tally["fail"]
    print("\n=== self-test %d/%d PASS (실패 %d건) ===" % (tally["pass"], total, tally["fail"]))
    return 0 if tally["fail"] == 0 else 1


def main(argv):
    if "--self-test" in argv[1:]:
        return self_test()
    if len(argv) < 3:
        print(__doc__.strip(), file=sys.stderr)
        return 2
    ver = argv[1]
    assets = argv[2]
    apply_ = "--apply" in argv[3:]
    e = load_env()

    four = ["cys_%s_aarch64.dmg" % ver, "cys_%s_x64.dmg" % ver,
            "cys_%s_x64-setup.exe" % ver, "cys_%s_x64-setup.zip" % ver]
    upload = [f for f in sorted(os.listdir(assets)) if os.path.isfile(os.path.join(assets, f))]
    missing = [f for f in four if f not in upload]
    if missing:
        print("::error::배포 4종 누락: %s" % ", ".join(missing), file=sys.stderr)
        return 1

    # ── 현재 라이브 상태 ──
    live = ftp_list(e, "downloads/")
    old_versions = sorted({m.group(1) for l in live for m in [re.search(r"cys_(\d+\.\d+\.\d+)_", l)] if m}
                          - {ver})
    print("라이브 downloads/ 항목 %d · 구버전 %s" % (len(live), old_versions or "없음"))

    # ── 페이지 범프 계획 ──
    # ★fail-closed 관문 (2026-08-27) — 여기서 받은 HTML 이 곧 **업로드될 본문**이다.
    #   수신이 검증되지 않은 문자열 위에서는 치환도 업로드도 하지 않는다. 유계 재시도를
    #   소진하고도 불확실하면 조용히 넘어가지 않고 **중단**한다(배포는 비가역).
    main_html, err_main = fetch(SITE + "/")
    dl_html, err_dl = fetch(SITE + "/downloads/")
    if err_main or err_dl:
        print("::error::라이브 페이지 수신 검증 실패 — 치환·업로드로 진행하지 않는다(fail-closed). "
              "잘린 본문을 올리면 홈페이지가 잘린 채 발행된다.", file=sys.stderr)
        if err_main:
            print("::error::  GET %s/ → %s" % (SITE, err_main), file=sys.stderr)
        if err_dl:
            print("::error::  GET %s/downloads/ → %s" % (SITE, err_dl), file=sys.stderr)
        return 1

    # ── ★메인페이지 전용 삭제 단계 (오너 지시 2026-08-24) ──
    # 버전 범프보다 **먼저** 돌린다 — 이후 계산(버전 토큰 집계·용량 토큰 치환)이 전부
    # "실제로 올라갈 HTML" 위에서 이뤄지도록 하기 위해서다. dl_html 은 손대지 않는다
    # (지시는 메인페이지 한정 · /downloads/ 안내 섹션 2종은 그대로 둔다).
    try:
        main_html, macrep = strip_main_macnote(main_html)
    except MacnoteEditError as ex:
        print("::error::메인페이지 macOS 안내 삭제 중단(fail-closed): %s" % ex, file=sys.stderr)
        return 1
    print_macnote_report(macrep)
    # 이전 버전 = 페이지에 **가장 많이** 등장하는 토큰(밴드 9곳). 문자열 정렬은
    # 잔존 토큰(예: 0.12.97 한 곳)을 집어 오답을 낸다 — dry-run 으로 실측 확인.
    counts = {}
    for v in re.findall(r"0\.\d+\.\d+", main_html):
        if v != ver:
            counts[v] = counts.get(v, 0) + 1
    prev = max(counts, key=counts.get) if counts else None
    print("메인 페이지: 현재 %s 토큰 %d개 → %s 로 범프" % (prev, main_html.count(prev or "\0"), ver))

    sizes = {f: os.path.getsize(os.path.join(assets, f)) for f in four}
    for f in four:
        print("  %-32s %12d B = %d MB(MiB 버림)" % (f, sizes[f], mib(sizes[f])))

    new_main = main_html.replace(prev, ver) if prev else main_html
    new_dl = dl_html.replace(prev, ver) if prev else dl_html
    # 용량 토큰 — MiB 버림. 순서는 aarch64 → x64 → exe → zip 로 페이지에 등장한다고 가정하지 않고
    # **구 자산의 실제 MiB → 신 자산 MiB** 로 1:1 치환한다(오탐 방지).
    if prev:
        for f in four:
            oldf = f.replace(ver, prev)
            # ★같은 수신 검증 규율을 탄다 — 잘린 헤더의 Content-Length 로 **틀린 용량**을
            #   페이지에 박는 경로를 봉한다. 3분류로 갈라 처리한다:
            h = http_fetch("%s/downloads/%s" % (SITE, oldf), head=True)
            if h.err:
                # 미판정 — 구자산 크기를 확인하지 못했다. 이대로 두면 페이지에 **구 용량이
                # 남은 채** 발행되고, 그건 조용한 오배포다. 배포는 비가역이므로 중단한다.
                print("::error::구자산 크기 미판정 — 중단(fail-closed): %s | %s" % (oldf, h.err),
                      file=sys.stderr)
                print("::error::  용량 토큰을 확인 없이 남긴 채 발행하지 않는다. 재실행하라.",
                      file=sys.stderr)
                return 1
            if not (200 <= h.status < 300):
                # 원격 확정 오류 — 구자산이 이미 정리됐다는 확정 답이다(비교 대상 없음 = 생략).
                print("  ⚠ 구자산 없음/원격 확정 오류(HTTP %d) — 용량 토큰 치환 생략: %s"
                      % (h.status, oldf))
                continue
            if not h.clen:
                print("  ⚠ 구자산 Content-Length 없음/0 — 용량 토큰 치환 생략: %s" % oldf)
                continue
            o, n = mib(h.clen), mib(sizes[f])
            if o == n:
                continue
            tok = "%dMB" % o
            if tok not in new_main:
                print("  ⚠ 페이지에 %s 토큰이 없다 — 치환 생략(%s)" % (tok, f))
                continue
            new_main = new_main.replace(tok, "%dMB" % n)
            print("  용량 토큰 %s → %dMB (%s)" % (tok, n, f))

    if not apply_:
        print("\n[dry-run] 아무것도 올리지 않았다. 집행은 --apply")
        print("  올릴 자산: %d개" % len(upload))
        print("  삭제 대상 구버전: %s" % (old_versions or "없음"))
        return 0

    # ── 집행 ──
    print("\n── 자산 업로드 ──")
    for f in upload:
        ok = ftp_put(e, os.path.join(assets, f), "downloads/" + f)
        print("  %s %s (%d bytes)" % ("✓" if ok else "✗", f, os.path.getsize(os.path.join(assets, f))))
        if not ok:
            print("::error::업로드 실패: %s" % f, file=sys.stderr)
            return 1

    print("── 페이지 업로드 ──")
    with tempfile.TemporaryDirectory() as td:
        mp, dp = os.path.join(td, "index.html"), os.path.join(td, "dl.html")
        # ★반드시 닫고 나서 올린다 — 흘려 쓴 핸들(open(...).write(...))은 flush 시점이
        #   인터프리터 GC 에 달려 있어, **덜 쓰인 파일을 업로드**할 여지를 남긴다.
        #   이 파일이 봉하려는 결함(부분 본문 발행)과 같은 클래스다.
        with open(mp, "w", encoding="utf-8") as fh:
            fh.write(new_main)
        with open(dp, "w", encoding="utf-8") as fh:
            fh.write(new_dl)
        for local, remote in ((mp, "index.html"), (dp, "downloads/index.html")):
            ok = ftp_put(e, local, remote)
            print("  %s %s" % ("✓" if ok else "✗", remote))
            if not ok:
                return 1

    print("── 구버전 자산 정리 (최신 1개 정책) ──")
    for l in live:
        name = l.split()[-1] if l.split() else ""
        m = re.search(r"cys_(\d+\.\d+\.\d+)_", name)
        if m and m.group(1) != ver:
            print("  %s rm %s" % ("✓" if ftp_delete(e, "downloads/" + name) else "✗", name))
    print("\n✅ 홈페이지 배포 완료")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
