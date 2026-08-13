#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_scrub.py — javis_scrub 비밀 마스킹 회귀 잠금(보안).

배경: `_normalize` 가 U+2028(LINE SEPARATOR)·U+2029(PARAGRAPH SEPARATOR)·
bidi isolate(U+2066..2069) 를 제거하지 못해, 이 문자를 비밀 토큰 사이에 끼워
넣으면 sk-/ghp_/AKIA 등 접두 패턴이 깨져 마스킹이 0건이 되고 원문(비밀 전체)이
그대로 통과하던 우회를 실측 재현·수리했다. 본 테스트는 그 우회 재발을 막고,
동시에 정상 텍스트(한국어·코드·URL·경로·개행)가 훼손되지 않음을 잠근다.

모든 비가시 문자는 chr() 코드포인트로만 표기(소스 왕복 훼손 방지).
부작용 0 · 표준 라이브러리만. 직접 실행 시 전건 통과면 exit 0.
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import javis_scrub as s  # noqa: E402

_fail = []


def check(name, cond, detail=""):
    mark = "OK  " if cond else "FAIL"
    print("[%s] %s%s" % (mark, name, ("" if cond else "  <<< " + repr(detail))))
    if not cond:
        _fail.append(name)


# ── ① 비가시 분리문자로 쪼갠 비밀은 반드시 마스킹된다(우회 차단) ────────────────
SECRET = "sk-ABCDEFGHIJKLMNOP"          # 접두 sk- + 16자 → 고신뢰 패턴
GH = "ghp_" + "A" * 36
SPLITTERS = {
    "U+2028 LINE SEP": chr(0x2028),
    "U+2029 PARA SEP": chr(0x2029),
    "U+2060 WORD JOINER": chr(0x2060),
    "U+2066 LRI": chr(0x2066),
    "U+2069 PDI": chr(0x2069),
    "U+200B ZWSP": chr(0x200B),
    "U+FEFF BOM": chr(0xFEFF),
    "U+00AD SOFT HYPHEN": chr(0x00AD),
    "U+202E RLO": chr(0x202E),
    "U+0007 BEL(Cc)": chr(0x0007),
}
for label, ch in SPLITTERS.items():
    inp = "sk-ABCD" + ch + "EFGHIJKLMNOP"          # sk- 토큰 중간에 분리문자 삽입
    out, n = s.scrub(inp)
    check("우회차단 sk- via %s" % label, n >= 1 and SECRET not in out, (n, out))
    inp2 = "leak ghp_" + "A" * 18 + ch + "A" * 18   # ghp_ 토큰 중간에 삽입
    out2, n2 = s.scrub(inp2)
    check("우회차단 ghp_ via %s" % label, n2 >= 1 and GH not in out2, (n2, out2))

# 대조군: 접두 붙은 온전한 비밀은 종전대로 마스킹
for name, secret, needle in [
    ("sk- contiguous", SECRET, SECRET),
    ("ghp_ contiguous", GH, GH),
    ("AKIA", "AKIA" + "0123456789AB", "AKIA0123456789AB"),
    ("key=val", "api_key=SUPERSECRETVALUE123", "SUPERSECRETVALUE123"),
]:
    out, n = s.scrub(secret)
    check("정상 마스킹 %s" % name, n >= 1 and needle not in out, (n, out))


# ── ② 정상 텍스트 무훼손(비밀 없음 → 원문 바이트 그대로, 개행·탭 보존) ─────────
NORMAL = {
    "한국어 문장": "주인님께 보고드립니다. 오늘 작업은 정상 완료되었습니다.",
    "멀티라인 개행탭": "line1\n\tindented line2\r\nline3",
    "파일경로": "/Users/x/proj/cysjavis-pack/bin/javis_scrub.py:42",
    "URL": "https://example.com/path?a=1&b=2#frag",
    "코드 스니펫": "def f(x):\n    return x_key + api  # not a secret literal",
}
for name, text in NORMAL.items():
    out, n = s.scrub(text)
    check("무훼손 %s (n=0)" % name, n == 0, (n, out))
    check("무훼손 %s (원문 동일)" % name, out == text, out)

# 개행/탭이 '마스킹이 발생한' 경우에도 보존되는지(구조 파괴 금지) — 종전엔 \n\t가 삭제됨
multiline_secret = "머리말 줄1\n\tapi_key=SUPERSECRETVALUE123\n꼬리말 줄3"
out, n = s.scrub(multiline_secret)
check("마스킹 시 개행보존", n >= 1 and "\n" in out and "\t" in out
      and "SUPERSECRETVALUE123" not in out, (n, out))
check("마스킹 시 구조보존(머리·꼬리말)", "머리말 줄1" in out and "꼬리말 줄3" in out, out)

# 가시적 공백(스페이스)로 분리된 것은 비밀이 아니다(오탐 금지) — 원문 유지
vis = "sk-ABCD" + chr(0x20) + "EFGHIJKLMNOP"      # ← 일반 스페이스 U+0020
out, n = s.scrub(vis)
check("가시 스페이스는 비밀 아님(오탐0)", n == 0 and out == vis, (n, out))


# ── ③ scrub_obj 재귀·민감키 계약 유지 ─────────────────────────────────────
obj = {
    "note": "sk-ABCD" + chr(0x2028) + "EFGHIJKLMNOP",
    "token": "opaque-value-xyz",
    "nested": ["ghp_" + "A" * 18 + chr(0x2029) + "A" * 18, {"password": "p"}],
    "task_key": "T1-normal-key",
}
red = s.scrub_obj(obj)
check("scrub_obj: 분리문자 비밀 마스킹", SECRET not in red["note"], red["note"])
check("scrub_obj: 민감키(token) 값 마스킹", red["token"] == s.MASK.strip(), red["token"])
check("scrub_obj: 중첩 리스트 비밀 마스킹", GH not in red["nested"][0], red["nested"][0])
check("scrub_obj: 정상키(task_key) 보존", red["task_key"] == "T1-normal-key", red["task_key"])


if _fail:
    print("\n%d FAIL: %s" % (len(_fail), _fail))
    sys.exit(1)
print("\njavis_scrub 회귀 테스트 전건 통과")
