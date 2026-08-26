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
종료코드 — ★판정은 **세 분류**이고 첫 줄이 어느 것인지 말한다(2026-08-27 개정):
  0 = 전건 통과
  1 = **적색** — 원격이 틀렸다. 두 종류가 여기 모인다(첫 줄이 어느 쪽인지 이름을 댄다):
        · 자산 무결성 실패 — 완전히 수신한 바이트가 SHA256SUMS 와 다르다 / 미등재
        · 원격 확정 오류   — 4xx(429 제외). 원격이 "그건 없다/못 준다"고 **확정 답**을 했다
  2 = 사용법
  3 = **미판정(DOWNLOAD_FAILED)** — 이 도구가 그 항목을 **판정하지 못했다**. 수신이 완결되지
      않았거나(전송 중단·부분 본문) 원격이 일시적으로 대답을 못 했다(429·5xx)이며, 유계 재시도
      상한(MAX_ATTEMPTS)을 소진한 상태다. ★적색도 초록도 아니다 — 이 도구에는 둘을 가릴 근거가 없다.
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
# ★수리 원칙 — 판정을 **세 종류로 쪼갠다** (2026-08-27 2라운드 개정):
#     · 자산 무결성 실패  = 완전 수신한 바이트가 SUMS 와 다르다/미등재  → FAIL(exit 1)
#     · 원격 확정 오류    = 4xx(429 제외). 원격의 **확정 답**이다        → FAIL(exit 1)
#     · 미판정            = 전송 미완결(중단·부분 본문) 또는 429·5xx 소진 → DOWNLOAD_FAILED(exit 3)
#   이유: 게이트가 무작위 적색을 내면 사람은 진짜 적색까지 "또 그거겠지"로 흘린다.
#   exit·첫 줄이 갈려야 "자산이 틀렸나 · 원격이 없다고 했나 · 내가 못 봤나"를 즉시 안다.
#
# ★2026-08-27 MF1 수리 — 1라운드 구현은 `status >= 400` 을 통째로 '원격이 틀렸다'로 접고
#   재시도를 한 번도 하지 않았다. 그 정당화("4xx 는 원격의 상태다 — 재시도해도 같다")는
#   **4xx 에만** 성립한다. 503·502·504·429 는 '릴리스가 틀렸다'가 아니라 '지금 대답을 못 한다'다.
#   호스팅/CDN 블립 1회로 릴리스가 BLOCK 되는 것은 이 파일이 죽이겠다고 선언한 위양성
#   클래스 그 자체다. 이제 429·5xx 는 다운로드 실패와 **같은 유계 재시도**를 타고, 소진되면
#   적색이 아니라 **미판정**으로 접힌다.
#
# ★2026-08-27 MF2 수리 — 미판정의 문면에서 **단언을 제거**했다. 구 문면은 "이것은 자산 적색이
#   아니다"라고 단언했는데, 이 도구는 그렇게 말할 근거가 없다: 오리진이 Content-Length 8192 를
#   선언하고 3000B 만 보내고 끊는 **실제 부분 자산**과, 경로에서 전송이 끊긴 경우는 수신 측에서
#   **같은 증상**이다. 방향만 바뀐 근거 없는 주장은 여전히 근거 없는 주장이다. 그래서 미판정은
#   "판정하지 못했다"만 말하고, 재시도가 매번 같은 지점에서 끝났다면 그 **관측 사실만** 덧붙인다
#   (해석·원인 지목 금지 — 판단은 사람 몫).
MAX_ATTEMPTS = 3                 # ★유계 — 최초 1회 + 재시도 2회. 이 레포의 재시도 규율
                                 #   (`retry-loop-needs-stop-condition`): 상한 없는 재시도 금지.
RETRY_BACKOFF_S = (2.0, 5.0)     # 지수 백오프 · len ≥ MAX_ATTEMPTS-1 (부족하면 마지막 값 반복)
PAGE_TIMEOUT = 120               # 페이지·HEAD 1회 상한(초) — 최악 3×120s
ASSET_TIMEOUT = 600              # 자산 본문 1회 상한(초) — 최악 3×600s = 30분(유계)

EXIT_OK = 0
EXIT_FAIL = 1                    # 적색 — 원격이 틀렸다(자산 무결성 실패 · 원격 확정 오류 4xx)
EXIT_USAGE = 2
EXIT_DOWNLOAD_FAILED = 3         # 미판정 — 이 도구가 판정하지 못했다. ★적색도 초록도 아니다

# ── 적색의 하위 분류(첫 줄에 이름이 찍힌다) ──
KIND_INTEGRITY = "자산 무결성 실패"       # 완전 수신 바이트 ≠ SUMS · 미등재
KIND_REMOTE = "원격 확정 오류(4xx)"       # 404 등 — 원격이 "없다"고 확정 답을 했다
KIND_CONTENT = "페이지 내용 불일치"       # 버전·용량·마커 등 문자열 판정 실패
KIND_UNDETERMINED = "미판정"

_CL_RE = re.compile(r"(?im)^\s*content-length:\s*(\d+)\s*$")


def is_transient_status(status):
    """유계 재시도 대상인 HTTP 상태인가 — **429 와 5xx**.

    ★근거: 이 둘은 "릴리스가 틀렸다"가 아니라 "지금 대답을 못 한다"다(rate limit·게이트웨이·
      과부하·유지보수). 반면 4xx(429 제외)는 원격의 **확정 답**이라 재시도해도 같다.
      경계를 여기 한 곳에만 둔다 — 호출부가 각자 상태코드를 해석하지 않게 하기 위해서다.
    """
    return status == 429 or status >= 500


class Fetched(object):
    """한 URL 전송의 결과 — **판정 불가와 확정 답을 다른 필드로 분리한다.**

      · err  != None → **미판정 재료**: 수신이 완결되지 못했거나(중단·부분 본문) 원격이
                       일시적으로 대답을 못 했다(429·5xx). 유계 재시도를 타고, 소진되면
                       DOWNLOAD_FAILED(exit 3). ★이 도구는 원인을 판정하지 않는다.
      · err  == None → 원격이 **확정 답**을 줬고 수신도 완결됐다. 그 다음 판정은
                       status/text/digest 로 한다(4xx · 해시 불일치 = 적색).
    """

    __slots__ = ("err", "status", "text", "digest", "nbytes", "clen", "tries", "retries",
                 "attempts_log")

    def __init__(self, err=None, status=0, text="", digest=None, nbytes=0, clen=None):
        self.err = err
        self.status = status
        self.text = text
        self.digest = digest
        self.nbytes = nbytes
        self.clen = clen
        self.tries = 1
        self.retries = []       # 재시도 사유 로그 — ★침묵 금지(있었으면 반드시 남는다)
        self.attempts_log = []  # 시도별 **관측 사실**(HTTP·수신 바이트) — 해석 없음

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
        nbytes = os.path.getsize(dest) if os.path.exists(dest) else 0
        clen = _content_length(header_text)
        # ★분기 1 — 응답은 왔고 상태가 4xx/5xx 다. **여기서 둘로 가른다**(2026-08-27 MF1 수리).
        #   --fail 이 이걸 비0 rc 로 접지만 **rc 값은 믿을 수 없다**: 실측(2026-08-25)에서
        #   HTTP/2 404 는 rc 22 가 아니라 **rc 56**으로 나왔다. 그래서 rc 가 아니라
        #   `%{http_code}` 로 가른다. (rc 검사는 분기 2 가 맡는다.)
        if status >= 400:
            if is_transient_status(status):
                # 429·5xx = '지금 대답을 못 한다'. 재시도가 다른 답을 줄 수 있으므로
                # 다운로드 실패와 **같은 유계 재시도**에 태운다(소진되면 미판정).
                return Fetched(err="원격 일시 불응답(HTTP %d)" % status,
                               status=status, nbytes=nbytes, clen=clen)
            # 4xx(429 제외) = 원격의 **확정 답**. 재시도해도 같다 → 즉시 적색 재료.
            return Fetched(status=status, text=header_text if head else "",
                           nbytes=nbytes, clen=clen)
        # ★분기 2 — returncode 검사(구현이 통째로 빠뜨렸던 축). 비0 = 수신 미완결.
        if rc != 0:
            return Fetched(err="curl 종료코드 %d%s" % (rc, _tail(serr)), status=status,
                           nbytes=nbytes, clen=clen)
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


def _repeat_note(seen):
    """유계 재시도를 소진했을 때 **시도별 관측 사실**을 덧붙인다.

    ★해석 금지 (MF2) — 매 시도가 같은 지점에서 끝났다면 그건 사람이 알아야 할 사실이지만,
      원인이 오리진(실제로 박힌 부분 자산)인지 경로(전송 절단)인지 이 도구는 **가릴 근거가
      없다**(수신 측에서 두 증상은 같다). 그래서 원인을 지목하지 않고 관측만 넘긴다.
    ★침묵 금지 — 지점이 매번 달랐다면 '같다'고 주장하지 않되, 시도별 값은 그대로 남긴다.
    """
    if len(seen) < 2:
        return ""
    facts = "; ".join("%d회차 HTTP %s · 수신 %dB" % (n + 1, s or "?", b)
                      for n, (s, b, _) in enumerate(seen))
    if len({(s, b) for s, b, _ in seen}) == 1:
        status, nbytes, _ = seen[0]
        return (" ★관측: 시도 %d회가 모두 같은 지점에서 끝났다(HTTP %s · 수신 %dB) — "
                "사실만 기록한다(이 도구는 원인을 판정하지 않는다)"
                % (len(seen), status or "?", nbytes))
    return (" ★관측: 시도별 [%s] — 사실만 기록한다(이 도구는 원인을 판정하지 않는다)" % facts)


def fetch(url, timeout=PAGE_TIMEOUT, head=False, want_text=True, want_digest=False,
          attempts=MAX_ATTEMPTS, _runner=None, _sleep=None):
    """유계 재시도로 URL 을 받는다 → Fetched.

    ★재시도 대상 (2026-08-27 MF1 개정):
        · 수신 미완결(전송 중단·부분 본문)  → 재시도
        · **429·5xx**(원격이 지금 대답을 못 함) → 재시도  ← 구현이 빠뜨렸던 축
        · 4xx(429 제외) = 원격의 확정 답     → 재시도 **안 한다**(같은 답이 올 뿐)
      해시 불일치도 재시도 대상이 아니다 — 같은 결과에 시간만 쓴다(호출부가 결정한다).
    ★상한은 `range(attempts)` 가 구조적으로 강제한다(무한 루프 불가 · 재시도 규율).
    ★재시도가 있었으면 즉시 로그로 남기고 결과에도 남긴다(침묵 금지).
    """
    runner = _runner or _run_curl
    sleeper = _sleep or time.sleep
    attempts = max(1, int(attempts))
    retries = []
    seen = []                                      # 시도별 관측 사실(해석 없음)
    r = None
    for i in range(attempts):                      # ← 유계
        if i:
            delay = RETRY_BACKOFF_S[min(i - 1, len(RETRY_BACKOFF_S) - 1)]
            note = ("재시도 %d/%d (%.0fs 후) ← %s" % (i, attempts - 1, delay, r.err))
            retries.append(note)
            print("  ↻ %s | %s" % (note, url))
            sleeper(delay)
        r = _attempt(url, timeout, head, want_text, want_digest, runner)
        seen.append((r.status, r.nbytes, r.err))
        if r.download_ok:
            break
    r.tries = len(retries) + 1
    r.retries = retries
    r.attempts_log = ["시도 %d: HTTP %s · 수신 %dB%s"
                      % (n + 1, s or "?", b, (" · " + e) if e else "")
                      for n, (s, b, e) in enumerate(seen)]
    if not r.download_ok:
        r.err = "%s [시도 %d회 소진]%s" % (r.err, r.tries, _repeat_note(seen))
    return r


def check(name, ok, detail="", kind=KIND_CONTENT):
    """PASS/FAIL 을 기록한다. `kind` 는 **적색의 하위 분류**이고 첫 줄 요약에 이름이 찍힌다."""
    results.append((name, "PASS" if ok else "FAIL", "" if ok else kind))
    print(("PASS " if ok else "FAIL " + "[%s] " % kind) + name
          + (" | " + detail if detail else ""))


def download_failed(name, reason, detail=""):
    """★적색도 초록도 아니다 — 이 도구가 그 항목을 **판정하지 못했다**(미판정).

    ★문면 규율 (2026-08-27 MF2): 여기서 자산의 상태를 **어느 방향으로도 단언하지 않는다.**
      오리진에 실제로 박힌 부분 자산과 경로에서 끊긴 전송은 수신 측에서 같은 증상이고,
      이 도구에는 둘을 가릴 근거가 없다. 말할 수 있는 것은 "확인하지 못했다"뿐이다.
    """
    results.append((name, "DOWNLOAD_FAILED", KIND_UNDETERMINED))
    download_failures.append("%s: %s" % (name, reason))
    print("DOWNLOAD_FAILED [미판정] " + name
          + " | 이 도구는 이 항목을 판정하지 못했다(수신 미완결 · 원격 내용 미확인): " + reason
          + ((" | " + detail) if detail else ""))


def http_detail(f):
    return "HTTP %d · 수신 %dB" % (f.status, f.nbytes)


def verdict():
    """세 판정을 **다른 종료코드 + 첫 줄의 이름**으로 낸다.

      · FAIL 하나라도 있음        → 1 적색 (하위 분류를 첫 줄이 이름으로 댄다:
                                     자산 무결성 실패 / 원격 확정 오류(4xx) / 페이지 내용 불일치)
      · FAIL 0 · DOWNLOAD_FAILED  → 3 미판정 (**적색도 초록도 아니다**)
      · 전건 PASS                 → 0
    """
    npass = sum(1 for r in results if r[1] == "PASS")
    fails = [(r[0], r[2]) for r in results if r[1] == "FAIL"]
    dls = [r[0] for r in results if r[1] == "DOWNLOAD_FAILED"]
    tally = "%d/%d PASS" % (npass, len(results))

    # ★첫 줄 하나로 "어느 분류인가 · 어떤 exit 인가"가 끝나야 한다.
    if fails:
        head = "적색 — %s (exit %d)" % (" + ".join(sorted({k for _, k in fails})), EXIT_FAIL)
    elif dls:
        head = "미판정 — 이 실행은 원격을 판정하지 못했다 (exit %d)" % EXIT_DOWNLOAD_FAILED
    else:
        head = "전건 통과 (exit %d)" % EXIT_OK
    print("\n=== 판정: %s · %s ===" % (head, tally)
          + ((" (미판정 %d건)" % len(dls)) if dls and fails else ""))

    if fails:
        for name, k in fails:
            print("   · [%s] %s" % (k, name))
    if dls:
        print("\n!! 미판정(DOWNLOAD_FAILED) %d건 — 아래 항목은 이번 실행에서 **판정되지 않았다**."
              % len(dls))
        print("   수신이 완결되지 않았거나(전송 중단·부분 본문) 원격이 일시적으로 대답을 못 했고"
              "(HTTP 429·5xx), 유계 재시도 %d회를 소진했다." % MAX_ATTEMPTS)
        print("   ★이 결과에서 자산의 상태는 **어느 쪽으로도 읽지 마라** — 이 도구에는 '오리진에"
              " 박힌 부분 자산'과 '경로에서 끊긴 전송'을 가릴 근거가 없다(수신 측에서 같은 증상).")
        print("   조치: 그대로 재실행하라. 반복되면 아래 관측 사실을 사람이 판단하라.")
        for d in download_failures:
            print("   · " + d)
    if fails:
        return EXIT_FAIL
    if dls:
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
        # 여기 도달하는 상태는 4xx(429 제외)뿐이다 — 429·5xx 는 위 분기에서 미판정으로 접힌다.
        check("메인 페이지 수신", False, http_detail(page),
              kind=KIND_REMOTE if page.status >= 400 else KIND_CONTENT)
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
    remote_bad, content_bad, dl_bad = [], [], []
    for f in four:
        h = fetch("%s/downloads/%s" % (SITE, f), head=True)
        if not h.download_ok:
            dl_bad.append("%s: %s" % (f, h.err))
            continue
        if h.status == 404:
            remote_bad.append("%s: 자산 부재(HTTP 404 · 원격 확정 답)" % f)
            continue
        if not h.http_ok:
            remote_bad.append("%s: 원격 확정 오류(HTTP %d)" % (f, h.status))
            continue
        if h.clen is None:
            content_bad.append("%s: Content-Length 헤더 없음(HTTP %d)" % (f, h.status))
            continue
        tok = "%dMB" % (h.clen // 1024 // 1024)
        if tok not in main_html:
            content_bad.append("%s: 표기 %s 없음(실제 %d B)" % (f, tok, h.clen))
    bad = remote_bad + content_bad
    if bad:
        check("③ 용량 4토큰 실자산 대조", False,
              "; ".join(bad) + ("; [+미판정 " + "; ".join(dl_bad) + "]" if dl_bad else ""),
              kind=KIND_CONTENT if content_bad else KIND_REMOTE)
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
              + ("; [+미판정 " + "; ".join(dl_bad) + "]" if dl_bad else ""),
              kind=KIND_REMOTE if (bad and len(full) == 4) else KIND_CONTENT)
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
        # SUMS 본문을 못 받았다 = 대조표가 없다 → 적색이 아니라 **미판정**(⑦ 은 계속 본다).
        download_failed("⑥ SHA256SUMS.txt 전수·실자산 대조", sf.err)
    elif ok6:
        want = {}
        for l in lines:
            p = l.split()
            if len(p) == 2:
                want[p[1]] = p[0]
        mismatch, remote_err, dl_bad, ok_notes = [], [], [], []
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
                # 여기 도달하는 상태는 4xx(429 제외)뿐 — 429·5xx 는 위에서 미판정으로 접힌다.
                remote_err.append("%s 원격 확정 오류(HTTP %d)" % (f, a.status))
                continue
            if a.digest != want[f]:
                # ★적색 — 완전한 본문(rc 0 · 수신 바이트수 = Content-Length)끼리의 불일치다.
                #   ★해시 불일치는 재시도 대상이 아니다: 같은 바이트를 다시 받아 같은 결과를
                #     내고 시간만 쓴다(재시도 규율).
                mismatch.append("%s 해시 불일치(완전 수신 %dB · 기대 %s… ≠ 실제 %s…)"
                                % (f, a.nbytes, want[f][:12], (a.digest or "")[:12]))
            elif a.retries:
                ok_notes.append("%s 재시도 %d회 후 성공" % (f, len(a.retries)))
        if ok_notes:
            detail += " · [" + "; ".join(ok_notes) + "]"     # 침묵 금지
        red = mismatch + remote_err
        if red:
            # 적색이 하나라도 확정되면 그게 판정이다(미판정 항목은 곁들여 적는다).
            check("⑥ SHA256SUMS.txt 전수·실자산 대조", False,
                  detail + " · 실자산 대조 " + ", ".join(red)
                  + ("; [+미판정 " + "; ".join(dl_bad) + "]" if dl_bad else ""),
                  kind=KIND_INTEGRITY if mismatch else KIND_REMOTE)
        elif dl_bad:
            # ★문면 규율(MF2): 여기서 "자산은 정상"이라고 말하지 않는다. 말할 수 있는 것은
            #   **다른** 자산들이 일치했다는 사실과, 이 자산은 확인하지 못했다는 사실뿐이다.
            download_failed("⑥ SHA256SUMS.txt 전수·실자산 대조", "; ".join(dl_bad),
                            detail + " · (대조에 성공한 나머지 자산은 해시 일치 — 위 항목은 미확인)")
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
