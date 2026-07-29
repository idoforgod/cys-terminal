#!/usr/bin/env python3
"""원격 발행 검증 — `docs/RELEASE.md` 의 메인 페이지 검증 6항목을 기계로 돌린다.

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
  ⑤ Windows Defender/SmartScreen 안내 섹션 잔존 (오너 지시 ⓐ·ⓒ)
  ⑥ SHA256SUMS.txt — 신버전 전수·구버전 0줄 + **실자산 바이트 해시 대조** (오너 지시 ⓑ)

사용: python3 scripts/verify-release-remote.py 0.14.5 [이전버전]
      (이전버전 생략 시 ① 은 건너뛴다)
종료코드: 0 = 전건 통과 · 1 = 하나라도 실패(발행 미완)
"""
import hashlib
import re
import subprocess
import sys

SITE = "https://www.cysinsight.com"
UA = "Mozilla/5.0"
results = []


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


def main(argv):
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

    # ⑤ Defender 안내 잔존
    n = len(re.findall(r"(?i)smartscreen|defender|추가 정보|알 수 없는 게시자", main_html))
    check("⑤ Defender/SmartScreen 안내 잔존", n >= 1, "marker %d" % n)

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

    npass = sum(1 for r in results if r)
    print("\n=== %d/%d PASS ===" % (npass, len(results)))
    return 0 if npass == len(results) else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
