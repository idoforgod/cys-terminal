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
종료코드: 0 = 전건 통과 · 1 = 하나라도 실패(발행 미완)
"""
import hashlib
import re
import subprocess
import sys

SITE = "https://www.cysinsight.com"
UA = "Mozilla/5.0"
results = []

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


def check(name, ok, detail=""):
    results.append(ok)
    print(("PASS " if ok else "FAIL ") + name + (" | " + detail if detail else ""))


def get(url, head=False):
    a = ["curl", "-s", "-A", UA, "--max-time", "120"]
    if head:
        a.append("-I")
    return subprocess.run(a + [url], capture_output=True).stdout.decode("utf-8", "replace")


def clen(url):
    m = re.search(r"(?i)content-length:\s*(\d+)", get(url, head=True))
    return int(m.group(1)) if m else None


def code(url):
    return subprocess.run(["curl", "-s", "-o", "/dev/null", "-w", "%{http_code}", "-A", UA,
                           "-I", "--max-time", "120", url], capture_output=True).stdout.decode()


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
        return 2
    ver = argv[1]
    prev = argv[2] if len(argv) > 2 else None
    four = ["cys_%s_aarch64.dmg" % ver, "cys_%s_x64.dmg" % ver,
            "cys_%s_x64-setup.exe" % ver, "cys_%s_x64-setup.zip" % ver]

    main_html = get(SITE + "/")
    if not main_html:
        check("메인 페이지 수신", False, "빈 응답")
        return 1

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
    bad = []
    for f in four:
        L = clen("%s/downloads/%s" % (SITE, f))
        if L is None:
            bad.append("%s: 자산 부재" % f)
            continue
        tok = "%dMB" % (L // 1024 // 1024)
        if tok not in main_html:
            bad.append("%s: 표기 %s 없음(실제 %d B)" % (f, tok, L))
    check("③ 용량 4토큰 실자산 대조", not bad, "; ".join(bad) if bad else "4종 일치")

    # ④ 링크 4종 200 (정적 href + JS setAttribute 양쪽)
    urls = set(re.findall(r'href="([^"]*(?:dmg|setup\.exe|setup\.zip))"', main_html))
    urls |= set(re.findall(r"""setAttribute\(\s*['"]href['"]\s*,\s*['"]([^'"]*(?:dmg|setup\.exe|setup\.zip))['"]""", main_html))
    full = sorted((u if u.startswith("http") else SITE + u.lstrip(".")) for u in urls)
    codes = {u: code(u) for u in full}
    bad = [u for u, c in codes.items() if c != "200"]
    check("④ 다운로드 링크 4종 HEAD 200", len(full) == 4 and not bad,
          "링크 %d개 · 비200 %s" % (len(full), bad or "없음"))

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
    dl_html = get(SITE + "/downloads/")
    if not dl_html:
        check("⑤ Defender 안내 섹션 마커 (다운로드 페이지)", False, "/downloads/ 빈 응답")
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
    sums = get("%s/downloads/SHA256SUMS.txt" % SITE)
    lines = [l for l in sums.splitlines() if l.strip()]
    newn = sum(1 for l in lines if ("cys_%s_" % ver) in l)
    oldn = sum(1 for l in lines if re.search(r"cys_\d+\.\d+\.\d+_", l) and ("cys_%s_" % ver) not in l)
    ok6 = bool(lines) and newn >= 4 and oldn == 0
    detail = "총 %d줄 · 신버전 %d · 구버전 %d" % (len(lines), newn, oldn)
    # 실자산 바이트 해시 대조 — 표기만 갱신되고 바이트가 구버전인 사고 차단
    if ok6:
        want = {}
        for l in lines:
            p = l.split()
            if len(p) == 2:
                want[p[1]] = p[0]
        mismatch = []
        for f in four:
            if f not in want:
                mismatch.append("%s 미등재" % f)
                continue
            blob = subprocess.run(["curl", "-s", "-A", UA, "--max-time", "600",
                                   "%s/downloads/%s" % (SITE, f)], capture_output=True).stdout
            if hashlib.sha256(blob).hexdigest() != want[f]:
                mismatch.append("%s 해시 불일치" % f)
        ok6 = not mismatch
        detail += " · 실자산 대조 " + (", ".join(mismatch) if mismatch else "4종 일치")
    check("⑥ SHA256SUMS.txt 전수·실자산 대조", ok6, detail)

    # ⑦ 메인페이지 macOS(Safari) 안내 삭제 확인 (오너 지시 2026-08-24)
    # 이미 받아둔 main_html 을 재사용한다 — 추가 왕복 없음.
    ok7, detail7 = macnote_verdict(main_macnote_counts(main_html))
    check("⑦ 메인 macOS 안내 삭제 · 윈도우 안내 잔존", ok7, detail7)

    npass = sum(1 for r in results if r)
    print("\n=== %d/%d PASS ===" % (npass, len(results)))
    return 0 if npass == len(results) else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
