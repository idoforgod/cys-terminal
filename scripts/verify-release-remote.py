#!/usr/bin/env python3
"""원격 발행 검증 — `docs/RELEASE.md` 의 메인·다운로드 페이지 검증 6항목을 기계로 돌린다.

★배경: 이 6항목은 지금까지 **100% 수동 게이트**였다(`verify-release-remote.sh`·
`release-assemble.py` 는 이 레인에 존재하지 않는다 — 실측). 수동이라 v0.13.17 에서
메인 밴드 누락이 **404 가 아니라 무증상 구버전 배포**로 나갔다. 이 스크립트가 그 공백을 닫는다.

검증 항목
  ① 구버전 문자열 0
  ② 신버전 문자열 9 (밴드 전건 반영)
  ③ 용량 4토큰 — 실자산 content-length 로 재계산해 페이지 표기와 대조 (MiB 버림)
  ④ 다운로드 링크 4종 HEAD 200
     ★정적 `href="…"` 만 보면 **JS 로 설정되는 zip 링크를 놓친다**(실측: 라이브 zip 은
       `w.setAttribute('href','/downloads/…zip')` 로 붙는다). 그래서 정적/동적 양쪽을 본다.
  ⑤ Windows Defender 안내 섹션 잔존 — **다운로드 페이지**(`/downloads/`)의 섹션 마커
     `data-cys-release-marker="windows-defender-guidance-v2"` 를 **정확히 1건** 단언 (오너 지시 ⓐ·ⓒ)
     ★루트(`/`)가 아니다 — 오너 체크리스트 ⓐ 가 검사 대상을 "다운로드 페이지"로 못박는다.
     ★낱말 grep(smartscreen/defender/…)은 **제거하지 않고 보조 축으로 AND** 유지한다
       (마커 껍데기만 남고 카피가 비는 사고 + 기존 루트 밴드 감시 축 보존).
  ⑥ SHA256SUMS.txt — 신버전 전수·구버전 0줄 + **실자산 바이트 해시 대조** (오너 지시 ⓑ)
  ⑦ 메인페이지 macOS(Safari) 안내 삭제 확인 (2026-08-24 오너 지시) — 루트(`/`)에서
     ⓐ`dl-hero__macnote` 0건 ⓑ`dl-hero__winnote` **정확히 1건**(윈도우 안내 생존)
     ⓒ`App Translocation` 0건. 셋을 **AND** 로 본다.
     ★ⓑ가 핵심 안전장치다 — 삭제 대상 macOS 문단이 class 를 `"dl-hero__winnote dl-hero__macnote"`
       로 **둘 다** 달고 있어(라이브 실측), winnote 로 매칭한 구현은 바로 아래 형제인 진짜 윈도우
       안내까지 지운다. 그 회귀는 ⓐ·ⓒ만으로는 통과해 버리므로 ⓑ 로 0건을 즉시 잡는다.
     ★⑤(다운로드 페이지 Defender 섹션)와 **다른 페이지·다른 축**이다 — 어느 쪽도 약화시키지 않는다.

사용: python3 scripts/verify-release-remote.py 0.14.5 [이전버전]
      (이전버전 생략 시 ① 은 건너뛴다)
      python3 scripts/verify-release-remote.py --self-test   # ⑦ 집계 로직 셀프테스트(무접촉)
종료코드: 0 = 전건 통과 · 1 = **원격이 틀렸다**(발행 미완 · 자산 적색) · 2 = 사용법
          3 = **DOWNLOAD_FAILED — 게이트 자신의 다운로드가 실패했다**(원격 판정 불가 · 재실행하라)
          ★1 과 3 의 구분이 이 스크립트의 계약이다 — 아래 '전송 판정 계약' 절 참조.

동반 회귀 테스트: `python3 scripts/tests/test_verify_release_remote.py` (합성 페이크 curl · 네트워크 불요)
"""
import hashlib
import os
import re
import subprocess
import sys
import tempfile
import time

SITE = "https://www.cysinsight.com"
UA = "Mozilla/5.0"
results = []            # (이름, "PASS"|"FAIL"|"DOWNLOAD_FAILED")
download_failures = []  # ★게이트 자신의 전송 실패 — '원격이 틀렸다'와 **다른 종류**다

# ⑤ 전용 상수 — 다운로드 페이지 Windows Defender 안내 섹션의 지문.
# 라이브 실측(2026-08-17):
#   /downloads/ → windows-defender-guidance-v2 1건 · macos-install-guidance-v1 1건
#   /          → data-cys-release-marker 0건
DEFENDER_MARKER = 'data-cys-release-marker="windows-defender-guidance-v2"'
DEFENDER_MARKER_EXPECT = 1
GUIDANCE_WORDS = r"(?i)smartscreen|defender|추가 정보|알 수 없는 게시자"
# ★마커 속성 자체를 낱말 집계에서 **빼기 위한** 정규식. 이유는 아래 ⑤ 주석의 항진명제 절 참조.
#   값이 무엇이든(`…-v2`·`macos-install-guidance-v1`·앞으로 생길 마커) 전부 지운다 — 마커
#   문자열에 감시 낱말이 섞이는 사고를 이름 규칙에 의존하지 않고 구조적으로 차단한다.
MARKER_ATTR_RE = re.compile(r'data-cys-release-marker="[^"]*"')

# ⑦ 전용 상수 — 메인페이지 macOS(Safari) 안내 문단 삭제 확인 (2026-08-24 오너 지시).
# 라이브 실측(2026-08-24 · 읽기 전용 GET, 삭제 **전** 상태):
#   /  →  dl-hero__macnote 1 · dl-hero__winnote 2 · "App Translocation" 1
# 삭제 후 기대치는 아래 EXPECT 상수 셋이다(0 / 1 / 0).
MACNOTE_CLASS = "dl-hero__macnote"
WINNOTE_CLASS = "dl-hero__winnote"
TRANSLOCATION_TOKEN = "App Translocation"
MACNOTE_EXPECT = 0
WINNOTE_EXPECT = 1          # ★0 도 실패다 — 윈도우 안내 소실 회귀를 여기서 잡는다
TRANSLOCATION_EXPECT = 0


def main_macnote_counts(html):
    """⑦ 3축 집계 — 순수함수(네트워크 무접촉)라 --self-test 가 합성 표본으로 시험할 수 있다."""
    return {"macnote": html.count(MACNOTE_CLASS),
            "winnote": html.count(WINNOTE_CLASS),
            "translocation": html.count(TRANSLOCATION_TOKEN)}


def macnote_verdict(counts):
    """집계 → (통과여부, 사람이 읽을 상세). 세 축 AND."""
    ok = (counts["macnote"] == MACNOTE_EXPECT
          and counts["winnote"] == WINNOTE_EXPECT
          and counts["translocation"] == TRANSLOCATION_EXPECT)
    detail = ("macnote %d(기대 %d) · winnote %d(기대 %d·윈도우 안내 생존) · '%s' %d(기대 %d)"
              % (counts["macnote"], MACNOTE_EXPECT, counts["winnote"], WINNOTE_EXPECT,
                 TRANSLOCATION_TOKEN, counts["translocation"], TRANSLOCATION_EXPECT))
    if counts["winnote"] < WINNOTE_EXPECT:
        detail += " ★윈도우 안내가 사라졌다 — winnote 매칭 회귀 의심"
    return ok, detail


# ── 전송 판정 계약 (2026-08-25 위양성 BLOCK 사고 수리) ────────────────────────
# ★사고: v0.14.27 검증 2차 실행에서 `cys_0.14.27_aarch64.dmg` 가 "해시 불일치"로 찍혀 EXITCODE=1
#   BLOCK 이 났다. 같은 자산을 7회 재실측(직접 GET 3 + 이 스크립트의 curl 인자 복제 3 + A/B cmp)
#   한 결과 전건 269437340B · b5907506f9… 일치 = **자산은 무결**이었고 1·3차 실행은 PASS 였다.
#
# ★근인: 구현이 자산을 `curl -s`(--fail 없음)로 받아 **종료코드도 수신 바이트수도 보지 않고**
#   곧바로 `hashlib.sha256(blob)` 를 걸었다. 전송이 도중에 끊기면 **부분 본문**이 그대로 해시돼
#   "해시 불일치"로 찍힌다 — 이 코드는 원리적으로 **'자산이 썩었다'와 '내 다운로드가 실패했다'를
#   구분할 수 없었다.** 같은 무방비가 페이지 GET·HEAD 경로에도 있었다(부분 HTML → 문자열 집계
#   오답 → ①②③④⑤⑦ 전 항목 위양성).
#
# ★수리 원칙 — 판정을 **두 종류로 쪼갠다**:
#     · 다운로드 실패 = 게이트 자신의 문제 → 유계 재시도 → 그래도 실패면 DOWNLOAD_FAILED(exit 3)
#     · 원격 내용 불일치(해시 불일치·404·문자열 오집계) = 자산 적색 → FAIL(exit 1)
#   이유: 게이트가 무작위 적색을 내면 사람은 진짜 적색까지 "또 그거겠지"로 흘린다.
#   exit·문면이 갈려야 "게이트가 틀렸나 자산이 틀렸나"를 즉시 안다.
MAX_ATTEMPTS = 3                 # ★유계 — 최초 1회 + 재시도 2회. 이 레포의 재시도 규율
                                 #   (`retry-loop-needs-stop-condition`): 상한 없는 재시도 금지.
RETRY_BACKOFF_S = (2.0, 5.0)     # 지수 백오프 · len ≥ MAX_ATTEMPTS-1 (부족하면 마지막 값 반복)
PAGE_TIMEOUT = 120               # 페이지·HEAD 1회 상한(초) — 최악 3×120s
ASSET_TIMEOUT = 600              # 자산 본문 1회 상한(초) — 최악 3×600s = 30분(유계)

EXIT_OK = 0
EXIT_FAIL = 1                    # 원격이 틀렸다(발행 미완)
EXIT_USAGE = 2
EXIT_DOWNLOAD_FAILED = 3         # 게이트가 못 받았다(판정 불가) — ★자산 적색 아님

_CL_RE = re.compile(r"(?im)^\s*content-length:\s*(\d+)\s*$")


class Fetched(object):
    """한 URL 전송의 결과 — **두 실패를 다른 필드로 분리한다.**

      · err  != None → **다운로드 실패**(전송 자체가 끝나지 못했다) = 게이트 문제
      · err  == None → 전송은 완결됐다. 그 다음 판정은 status/text/digest 로 한다
                       (status ≥ 400 · 해시 불일치 = **원격 내용 문제** = 적색)
    """

    __slots__ = ("err", "status", "text", "digest", "nbytes", "clen", "tries", "retries")

    def __init__(self, err=None, status=0, text="", digest=None, nbytes=0, clen=None):
        self.err = err
        self.status = status
        self.text = text
        self.digest = digest
        self.nbytes = nbytes
        self.clen = clen
        self.tries = 1
        self.retries = []     # 재시도 사유 로그 — ★침묵 금지(있었으면 반드시 남는다)

    @property
    def download_ok(self):
        return self.err is None

    @property
    def http_ok(self):
        return self.err is None and 200 <= self.status < 300


def _run_curl(argv):
    """실 curl 실행기. 반환 (returncode, -w 출력, stderr)."""
    p = subprocess.run(argv, capture_output=True)
    return (p.returncode,
            p.stdout.decode("utf-8", "replace"),
            p.stderr.decode("utf-8", "replace"))


def curl_argv(url, timeout, head, dest, hdr):
    """전송 1회의 인자. ★`--fail`·`--show-error`·`-D`·`-w` 넷이 계약이다(테스트가 못박는다).

      --fail        : 4xx/5xx 본문(=오류 페이지)을 성공으로 착각해 해시하지 않는다
      --show-error  : `-s` 로 눌린 오류 문면을 되살린다(침묵 금지)
      -D <hdr>      : Content-Length 대조용 헤더 원본
      -w %{http_code} %{size_download} : rc 에 의존하지 않는 상태·바이트수 관측
    """
    a = ["curl", "-s", "--show-error", "--fail", "-A", UA, "--max-time", str(int(timeout)),
         "-D", hdr, "-o", dest, "-w", "%{http_code} %{size_download}"]
    if head:
        a.append("-I")
    return a + [url]


def _content_length(header_text):
    """헤더 덤프의 **마지막** Content-Length(리다이렉트 체인 대비). 없으면 None."""
    m = _CL_RE.findall(header_text or "")
    return int(m[-1]) if m else None


def _http_code(w_out):
    try:
        return int((w_out or "").split()[0])
    except (IndexError, ValueError):
        return 0


def _read_text(path):
    try:
        with open(path, "rb") as fh:
            return fh.read().decode("utf-8", "replace")
    except OSError:
        return ""


def _sha256_file(path):
    """스트리밍 해시 — 269MB 자산을 메모리에 통째로 올리지 않는다."""
    h = hashlib.sha256()
    with open(path, "rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def _attempt(url, timeout, head, want_text, want_digest, runner):
    """전송 1회. 다운로드 실패는 err 로, 원격 상태는 status 로 나온다."""
    td = tempfile.mkdtemp(prefix="cys-verify-remote-")
    try:
        dest, hdr = os.path.join(td, "body"), os.path.join(td, "hdr")
        rc, w_out, serr = runner(curl_argv(url, timeout, head, dest, hdr))
        status = _http_code(w_out)
        header_text = _read_text(hdr)
        # ★분기 1 — 응답은 왔고 4xx/5xx 다 → **원격 내용 문제**(재시도해도 같다).
        #   --fail 이 이걸 비0 rc 로 접지만 **rc 값은 믿을 수 없다**: 실측(2026-08-25)에서
        #   HTTP/2 404 는 rc 22 가 아니라 **rc 56**으로 나왔다. 그래서 rc 가 아니라
        #   `%{http_code}` 로 가른다. (rc 검사는 분기 2 가 맡는다.)
        if status >= 400:
            return Fetched(status=status, text=header_text if head else "")
        # ★분기 2 — returncode 검사(구현이 통째로 빠뜨렸던 축). 비0 = 다운로드 실패.
        if rc != 0:
            return Fetched(err="curl 종료코드 %d%s" % (rc, _tail(serr)), status=status)
        nbytes = os.path.getsize(dest) if os.path.exists(dest) else 0
        clen = _content_length(header_text)
        if head:
            # HEAD 는 본문이 없다 — 헤더가 산출물이고 Content-Length 는 '선언된 크기'다.
            return Fetched(status=status, text=header_text, clen=clen)
        # ★분기 3 — 수신 바이트수 vs Content-Length 대조. 불일치 = 부분 본문 = 다운로드 실패.
        #   (rc 검사만으로는 못 잡는 조기 종료가 여기서 걸린다.)
        if clen is not None and clen != nbytes:
            return Fetched(err="수신 %dB ≠ Content-Length %dB (부분 본문)" % (nbytes, clen),
                           status=status, nbytes=nbytes, clen=clen)
        return Fetched(status=status,
                       text=_read_text(dest) if want_text else "",
                       digest=_sha256_file(dest) if want_digest else None,
                       nbytes=nbytes, clen=clen)
    finally:
        _rmtree(td)


def _rmtree(path):
    try:
        for name in os.listdir(path):
            try:
                os.remove(os.path.join(path, name))
            except OSError:
                pass
        os.rmdir(path)
    except OSError:
        pass


def _tail(serr):
    line = ([l for l in (serr or "").splitlines() if l.strip()] or [""])[-1].strip()
    return (" · " + line[:200]) if line else ""


def fetch(url, timeout=PAGE_TIMEOUT, head=False, want_text=True, want_digest=False,
          attempts=MAX_ATTEMPTS, _runner=None, _sleep=None):
    """유계 재시도로 URL 을 받는다 → Fetched.

    ★재시도 대상은 **다운로드 실패뿐**이다(전송 중단·부분 본문). HTTP 4xx/5xx 는
      원격의 상태이므로 재시도하지 않는다 — 같은 답이 올 뿐이고, 적색을 늦출 뿐이다.
    ★상한은 `range(attempts)` 가 구조적으로 강제한다(무한 루프 불가 · 재시도 규율).
    ★재시도가 있었으면 즉시 로그로 남기고 결과에도 남긴다(침묵 금지).
    """
    runner = _runner or _run_curl
    sleeper = _sleep or time.sleep
    attempts = max(1, int(attempts))
    retries = []
    r = None
    for i in range(attempts):                      # ← 유계
        if i:
            delay = RETRY_BACKOFF_S[min(i - 1, len(RETRY_BACKOFF_S) - 1)]
            note = ("재시도 %d/%d (%.0fs 후) ← %s" % (i, attempts - 1, delay, r.err))
            retries.append(note)
            print("  ↻ %s | %s" % (note, url))
            sleeper(delay)
        r = _attempt(url, timeout, head, want_text, want_digest, runner)
        if r.download_ok:
            break
    r.tries = len(retries) + 1
    r.retries = retries
    if not r.download_ok:
        r.err = "%s [시도 %d회 소진]" % (r.err, r.tries)
    return r


def check(name, ok, detail=""):
    results.append((name, "PASS" if ok else "FAIL"))
    print(("PASS " if ok else "FAIL ") + name + (" | " + detail if detail else ""))


def download_failed(name, reason, detail=""):
    """★적색이 아니다 — 게이트가 못 받아서 **판정하지 못했다**는 제3의 결과."""
    results.append((name, "DOWNLOAD_FAILED"))
    download_failures.append("%s: %s" % (name, reason))
    print("DOWNLOAD_FAILED " + name + " | 게이트 다운로드 실패(원격 무결성 판정 불가): " + reason
          + ((" | " + detail) if detail else ""))


def http_detail(f):
    return "HTTP %d · 수신 %dB" % (f.status, f.nbytes)


def verdict():
    """세 결과를 **다른 종료코드**로 낸다 — 사람이 문면만 보고 책임 소재를 안다.

      · FAIL 하나라도 있음        → 1 (원격이 틀렸다 · 확정 적색이 판정을 지배한다)
      · FAIL 0 · DOWNLOAD_FAILED  → 3 (게이트가 못 받았다 · **판정 불가** — 적색 아님)
      · 전건 PASS                 → 0
    """
    npass = sum(1 for _, s in results if s == "PASS")
    nfail = sum(1 for _, s in results if s == "FAIL")
    ndl = sum(1 for _, s in results if s == "DOWNLOAD_FAILED")
    line = "\n=== %d/%d PASS ===" % (npass, len(results))
    if ndl:
        line += " (DOWNLOAD_FAILED %d건 — 판정 불가)" % ndl
    print(line)
    if ndl:
        print("\n!! DOWNLOAD_FAILED %d건 — 이것은 **자산 적색이 아니다**. 게이트 자신의 다운로드가"
              " 유계 재시도 %d회를 소진하고 실패했다(전송 중단·부분 본문). 해당 항목의 원격"
              " 무결성은 이번 실행으로 **판정되지 않았다** — 재실행하라." % (ndl, MAX_ATTEMPTS))
        for d in download_failures:
            print("   · " + d)
    if nfail:
        return EXIT_FAIL
    if ndl:
        return EXIT_DOWNLOAD_FAILED
    return EXIT_OK


def self_test():
    """⑦ 판정 로직을 합성 표본으로 시험한다(라이브 무접촉)."""
    tally = {"pass": 0, "fail": 0}

    def ok(name, cond, detail=""):
        tally["pass" if cond else "fail"] += 1
        print(("PASS " if cond else "FAIL ") + name + (" | " + detail if detail else ""))

    mac_p = ('<p class="dl-hero__winnote dl-hero__macnote">참고 — macOS(Safari) 설치: '
             'App Translocation 때문에 …</p>\n')
    win_p = '<p class="dl-hero__winnote">참고 — 윈도우 설치파일: SmartScreen …</p>\n'

    # ⓐ 삭제 완료 상태 = 통과
    v, d = macnote_verdict(main_macnote_counts("<div>\n" + win_p + "</div>"))
    ok("⑦ⓐ 삭제 완료 페이지는 통과", v, d)

    # ⓑ 삭제 전(macnote 잔존) = 실패
    v, d = macnote_verdict(main_macnote_counts("<div>\n" + mac_p + win_p + "</div>"))
    ok("⑦ⓑ 삭제 안 된 페이지는 실패", not v, d)

    # ⓒ ★회귀: winnote 로 매칭해 둘 다 지운 페이지 = 실패 (macnote 0·translocation 0 이라
    #    ⓐ·ⓒ축만 보면 통과해버린다 — winnote 축이 유일한 검출기다)
    v, d = macnote_verdict(main_macnote_counts("<div>\n</div>"))
    ok("⑦ⓒ 윈도우 안내까지 지운 회귀는 실패", not v, d)

    # ⓓ 윈도우 안내 중복(2건) = 실패
    v, d = macnote_verdict(main_macnote_counts("<div>\n" + win_p + win_p + "</div>"))
    ok("⑦ⓓ winnote 2건은 실패", not v, d)

    total = tally["pass"] + tally["fail"]
    print("\n=== self-test %d/%d PASS (실패 %d건) ===" % (tally["pass"], total, tally["fail"]))
    return 0 if tally["fail"] == 0 else 1


def main(argv):
    if "--self-test" in argv[1:]:
        return self_test()
    if len(argv) < 2:
        print(__doc__.strip(), file=sys.stderr)
        return EXIT_USAGE
    ver = argv[1]
    prev = argv[2] if len(argv) > 2 else None
    four = ["cys_%s_aarch64.dmg" % ver, "cys_%s_x64.dmg" % ver,
            "cys_%s_x64-setup.exe" % ver, "cys_%s_x64-setup.zip" % ver]

    page = fetch(SITE + "/")
    if not page.download_ok:
        # ★메인 HTML 이 부분 본문이면 ①②③④⑤⑦ 이 전부 오답을 낸다 — 그 위에서 판정하지 않는다.
        download_failed("메인 페이지 수신", page.err)
        return verdict()
    if not page.http_ok or not page.text:
        check("메인 페이지 수신", False, http_detail(page))
        return verdict()
    main_html = page.text

    # ① 구버전 0
    if prev:
        n = main_html.count(prev)
        check("① 구버전 문자열 0 (%s)" % prev, n == 0, "발견 %d개" % n)
    else:
        print("SKIP ① 구버전 미지정")

    # ② 신버전 9
    n = main_html.count(ver)
    check("② 신버전 문자열 9 (%s)" % ver, n == 9, "발견 %d개" % n)

    # ③ 용량 4토큰
    # ★HEAD 도 같은 분리 규율을 탄다 — '자산 부재(404)'와 '내 HEAD 가 실패했다'는 다른 사건이다.
    #   구 구현은 둘 다 "자산 부재"로 적었다(전자만 적색인데 후자까지 적색이 됐다).
    bad, dl_bad = [], []
    for f in four:
        h = fetch("%s/downloads/%s" % (SITE, f), head=True)
        if not h.download_ok:
            dl_bad.append("%s: %s" % (f, h.err))
            continue
        if h.status == 404:
            bad.append("%s: 자산 부재(HTTP 404)" % f)
            continue
        if not h.http_ok:
            bad.append("%s: HTTP %d" % (f, h.status))
            continue
        if h.clen is None:
            bad.append("%s: Content-Length 헤더 없음(HTTP %d)" % (f, h.status))
            continue
        tok = "%dMB" % (h.clen // 1024 // 1024)
        if tok not in main_html:
            bad.append("%s: 표기 %s 없음(실제 %d B)" % (f, tok, h.clen))
    if bad:
        check("③ 용량 4토큰 실자산 대조", False,
              "; ".join(bad) + ("; [+다운로드 실패 " + "; ".join(dl_bad) + "]" if dl_bad else ""))
    elif dl_bad:
        download_failed("③ 용량 4토큰 실자산 대조", "; ".join(dl_bad))
    else:
        check("③ 용량 4토큰 실자산 대조", True, "4종 일치")

    # ④ 링크 4종 200 (정적 href + JS setAttribute 양쪽)
    urls = set(re.findall(r'href="([^"]*(?:dmg|setup\.exe|setup\.zip))"', main_html))
    urls |= set(re.findall(r"""setAttribute\(\s*['"]href['"]\s*,\s*['"]([^'"]*(?:dmg|setup\.exe|setup\.zip))['"]""", main_html))
    full = sorted((u if u.startswith("http") else SITE + u.lstrip(".")) for u in urls)
    bad, dl_bad = [], []
    for u in full:
        h = fetch(u, head=True)
        if not h.download_ok:
            dl_bad.append("%s: %s" % (u, h.err))
        elif h.status != 200:
            bad.append("%s(HTTP %d)" % (u, h.status))
    if len(full) != 4 or bad:
        check("④ 다운로드 링크 4종 HEAD 200", False,
              "링크 %d개 · 비200 %s" % (len(full), bad or "없음")
              + ("; [+다운로드 실패 " + "; ".join(dl_bad) + "]" if dl_bad else ""))
    elif dl_bad:
        download_failed("④ 다운로드 링크 4종 HEAD 200", "; ".join(dl_bad),
                        "링크 %d개(수집은 정상)" % len(full))
    else:
        check("④ 다운로드 링크 4종 HEAD 200", True, "링크 4개 · 비200 없음")

    # ⑤ Defender 안내 섹션 잔존 — 다운로드 페이지의 섹션 마커가 정본 (오너 지시 ⓐ·ⓒ)
    #
    # ★왜 루트(`/`)가 아니라 다운로드 페이지(`/downloads/`)인가 — 오너 체크리스트 정본 문언:
    #   「ⓐ**다운로드 페이지** Defender 안내 섹션 잔존 grep 확인 ⓑSHA256SUMS 전 자산 갱신·누락 0
    #    ⓒ**원격 검증(verify-release-remote)에 안내 섹션 출현 포함**」
    #   ★출처(리포 내 정본): docs/RELEASE.md 의 발행 후 검증 ⑤ 항목. 종전 이 인용은 저장소
    #     어디에도 원문이 없어 주석만 읽는 사람이 출처에 도달할 수 없었다(적대적 리뷰 지적).
    #     2026-08-18 에 RELEASE.md ⑤ 를 이 구현에 맞춰 갱신하면서 그 문언을 문서에 심었다.
    #   안내 섹션의 실체는 /downloads/ 의 <section data-cys-release-marker="…-v2"> 이고,
    #   루트에는 밴드 카피의 낱말만 흩어져 있다(실측: 루트 마커 0건 · 다운로드 1건).
    #   구 구현은 루트만 봤으므로 ⓐ 가 지목한 페이지를 **한 번도 받지 않았다** = ⓒ 미구현.
    #
    # ★왜 낱말 grep 이 아니라 마커인가 — 낱말 grep 은 섹션이 통째로 사라져도 페이지 다른 곳에
    #   'Defender' 한 낱말만 남아 있으면 통과한다(무증상 통과). 마커는 섹션 그 자체의 지문이라
    #   섹션이 빠지면 즉시 0이 된다. 개수까지 단언해 중복 삽입(마커 2건)도 잡는다.
    #
    # ★낱말 grep 은 제거하지 않고 **보조 축으로 AND** 한다 — 회귀 감시 축이 줄면 안 되므로
    #   (a) 다운로드 낱말 ≥1: 마커 <section> 껍데기만 남고 본문 카피가 비는 사고 차단
    #   (b) 루트   낱말 ≥1: 기존 감시 축(버전 범프 일괄 치환이 밴드 카피를 통째로 갈아끼워
    #                        루트 안내가 조용히 사라지는 사고 — RELEASE.md ⑤) 그대로 보존
    #
    # ★2026-08-18 교정 — (a) 는 **항진명제였다**(적대적 리뷰 지적 · 실측 반증됨).
    #   마커 문자열 `data-cys-release-marker="windows-defender-guidance-v2"` 안에 'defender'
    #   가 들어 있어, GUIDANCE_WORDS 가 마커 자신에게 걸렸다. 즉 mk==1 인 한 dlw>=1 이
    #   **구조적으로 보장**돼 (a) 는 절대 실패할 수 없었다 — 주석이 막는다고 쓴 바로 그 사고
    #   (마커 껍데기만 남고 본문 카피가 빈 <section>)가 그대로 통과했다.
    #   고침: 낱말을 세기 **전에** 마커 속성을 전부 지운다. 그러면 낱말은 오직 **본문 카피**에서만
    #   나온다. 라이브 실측(2026-08-18 · 읽기 전용 GET): /downloads/ 낱말 원본 5 → 마커 제거 후
    #   **4**(전부 본문 'Defender'), 마커 1건 — 즉 이 교정으로 현행 페이지 판정은 안 바뀌고
    #   (여전히 PASS) 항진명제만 사라진다. 루트(`/`)는 마커가 0건이라(실측) 제거 전후 8로 동일하다.
    dlp = fetch(SITE + "/downloads/")
    dl_html = dlp.text if dlp.download_ok else ""
    if not dlp.download_ok:
        download_failed("⑤ Defender 안내 섹션 마커 (다운로드 페이지)", dlp.err)
    elif not dl_html:
        check("⑤ Defender 안내 섹션 마커 (다운로드 페이지)", False,
              "/downloads/ 빈 응답 · " + http_detail(dlp))
    else:
        mk = dl_html.count(DEFENDER_MARKER)
        # 마커 속성을 지운 **본문**에서만 낱말을 센다(항진명제 차단).
        dlw = len(re.findall(GUIDANCE_WORDS, MARKER_ATTR_RE.sub("", dl_html)))
        rootw = len(re.findall(GUIDANCE_WORDS, MARKER_ATTR_RE.sub("", main_html)))
        ok5 = mk == DEFENDER_MARKER_EXPECT and dlw >= 1 and rootw >= 1
        check("⑤ Defender 안내 섹션 마커 (다운로드 페이지)", ok5,
              "마커 %d(기대 %d) · 다운로드 본문 낱말 %d(보조·마커 제외) · 루트 낱말 %d(보조)"
              % (mk, DEFENDER_MARKER_EXPECT, dlw, rootw))

    # ⑥ SHA256SUMS.txt
    sf = fetch("%s/downloads/SHA256SUMS.txt" % SITE)
    sums = sf.text if sf.download_ok else ""
    lines = [l for l in sums.splitlines() if l.strip()]
    newn = sum(1 for l in lines if ("cys_%s_" % ver) in l)
    oldn = sum(1 for l in lines if re.search(r"cys_\d+\.\d+\.\d+_", l) and ("cys_%s_" % ver) not in l)
    ok6 = bool(lines) and newn >= 4 and oldn == 0
    detail = "총 %d줄 · 신버전 %d · 구버전 %d" % (len(lines), newn, oldn)
    # 실자산 바이트 해시 대조 — 표기만 갱신되고 바이트가 구버전인 사고 차단
    #
    # ★2026-08-25 수리(위양성 BLOCK) — 여기가 사고 지점이다. 구현:
    #     blob = subprocess.run(["curl","-s",…], capture_output=True).stdout
    #     if hashlib.sha256(blob).hexdigest() != want[f]: mismatch.append("해시 불일치")
    #   returncode 도 수신 바이트수도 보지 않으니 **끊긴 전송의 부분 본문이 그대로 해시**돼
    #   "해시 불일치"(=자산 적색)로 찍혔다. 이제 `fetch()` 가 전송 완결성(rc·Content-Length)을
    #   먼저 단언하므로, **여기 도달한 해시 불일치는 완전한 본문끼리의 불일치**다 = 진짜 적색.
    #   두 결과를 다른 통에 담는다: mismatch(적색·exit 1) vs dl_bad(판정 불가·exit 3).
    if not sf.download_ok:
        # SUMS 본문을 못 받았다 = 대조표가 없다 → 적색이 아니라 **판정 불가**(⑦ 은 계속 본다).
        download_failed("⑥ SHA256SUMS.txt 전수·실자산 대조", sf.err)
    elif ok6:
        want = {}
        for l in lines:
            p = l.split()
            if len(p) == 2:
                want[p[1]] = p[0]
        mismatch, dl_bad = [], []
        for f in four:
            if f not in want:
                mismatch.append("%s 미등재" % f)
                continue
            a = fetch("%s/downloads/%s" % (SITE, f), timeout=ASSET_TIMEOUT,
                      want_text=False, want_digest=True)
            if not a.download_ok:
                dl_bad.append("%s: %s" % (f, a.err))
                continue
            if not a.http_ok:
                mismatch.append("%s 자산 부재/오류(HTTP %d)" % (f, a.status))
                continue
            if a.digest != want[f]:
                # ★적색 — 완전한 본문(rc 0 · 수신 %dB = Content-Length)끼리의 불일치다.
                mismatch.append("%s 해시 불일치(완전 수신 %dB · 기대 %s… ≠ 실제 %s…)"
                                % (f, a.nbytes, want[f][:12], (a.digest or "")[:12]))
            elif a.retries:
                detail += " · [%s 재시도 %d회 후 성공]" % (f, len(a.retries))
        if mismatch:
            # 적색이 하나라도 확정되면 그게 판정이다(다운로드 실패는 곁들여 적는다).
            check("⑥ SHA256SUMS.txt 전수·실자산 대조", False,
                  detail + " · 실자산 대조 " + ", ".join(mismatch)
                  + ("; [+다운로드 실패 " + "; ".join(dl_bad) + "]" if dl_bad else ""))
        elif dl_bad:
            download_failed("⑥ SHA256SUMS.txt 전수·실자산 대조", "; ".join(dl_bad),
                            detail + " · 나머지 자산은 해시 일치")
        else:
            check("⑥ SHA256SUMS.txt 전수·실자산 대조", True, detail + " · 실자산 대조 4종 일치")
    else:
        check("⑥ SHA256SUMS.txt 전수·실자산 대조", False, detail)

    # ⑦ 메인페이지 macOS(Safari) 안내 삭제 확인 (오너 지시 2026-08-24)
    # 이미 받아둔 main_html 을 재사용한다 — 추가 왕복 없음.
    ok7, detail7 = macnote_verdict(main_macnote_counts(main_html))
    check("⑦ 메인 macOS 안내 삭제 · 윈도우 안내 잔존", ok7, detail7)

    return verdict()


if __name__ == "__main__":
    sys.exit(main(sys.argv))
