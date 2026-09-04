#!/usr/bin/env python3
# -*- coding: utf-8 -*-
r"""test_pack_syntax_warnings.py — 팩 python 전수 `SyntaxWarning` 0 (0.14.30 · gemini Phase0 R1 minor).

무엇을 막는가: 정규식 리터럴(`\s`·`\[`)을 **일반 문자열**에 쓰면 CPython 이 매 import 마다
`SyntaxWarning: "\s" is an invalid escape sequence` 를 stderr 로 낸다. 이 팩은 훅·부트 체인이
python 을 초당 여러 번 부르므로 그 경고가 **부트 로그를 통째로 오염**시키고, 진짜 진단 한 줄이
경고 더미에 묻힌다. 게다가 파이썬은 이 경고를 장래 **에러로 승격**한다고 예고했다 — 지금 고치지
않으면 어느 릴리스에서 팩 전체가 import 불능이 된다.

수리 방식은 raw 문자열(모듈 독스트링 앞에 `r` 접두)이다. 부수 효과가 하나 더 있다: 종전에는 독스트링 안의
`` `\n` `` 이 **실제 개행 문자**로 치환돼 문서가 조용히 깨져 있었다 — raw 화가 그 의미까지 복원한다.

판정: `bin/*.py` · `bin/tests/*.py` · `scripts/*.py` 를 **컴파일**해 SyntaxWarning 을 수집한다
(import 가 아니라 compile 이라 부작용 0 · 서브프로세스 0).
★음성 대조: 비-raw 문자열에 `\s` 를 넣은 합성 소스가 실제로 경고를 내는지 확인한다 — 계측기가
살아 있지 않으면 "0건"은 아무것도 뜻하지 않는다.
출력: PASS/FAIL 행 · 실패 시 exit 1 · 종료 토큰 PACK-SYNTAX-WARNINGS-OK.
실행 규약(CI 동형): CYS_PACK_DIR="$(mktemp -d)" python3 bin/tests/test_pack_syntax_warnings.py
"""
import glob
import io
import os
import sys
import warnings

SELF = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(SELF)
REPO = os.path.dirname(os.path.dirname(BIN))
fails = []


def check(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
    if not cond:
        fails.append(name)


def warns(src, name):
    """(SyntaxWarning 목록) — compile 만 한다(실행·import 0)."""
    with warnings.catch_warnings(record=True) as w:
        warnings.simplefilter("always")
        try:
            compile(src, name, "exec")
        except SyntaxError as e:
            return ["SyntaxError: %s" % e]
        return ["%s:%s %s" % (name, x.lineno, x.message) for x in w
                if issubclass(x.category, SyntaxWarning)]


targets = sorted(glob.glob(os.path.join(BIN, "*.py"))
                 + glob.glob(os.path.join(SELF, "*.py"))
                 + glob.glob(os.path.join(REPO, "scripts", "*.py")))
check("0 스캔 대상 실재(계측 불능 방지)", len(targets) >= 60, "대상 %d개" % len(targets))

found = []
for p in targets:
    found += warns(io.open(p, encoding="utf-8").read(), os.path.relpath(p, REPO))
check("1 팩 python 전수 SyntaxWarning 0", not found, "잔여 %d건: %s" % (len(found), found[:5]))

# ★음성 대조 — 계측기가 실제로 경고를 잡는가(0건이 '재지 않아서'가 아님을 증명)
mut = warns('"""doc with \\s regex"""\n', "<synthetic>")
check("2 음성 대조: 비-raw 문자열의 \\s 는 실제로 잡힌다", len(mut) == 1, repr(mut))
mut_ok = warns('r"""doc with \\s regex"""\n', "<synthetic-raw>")
check("3 raw 문자열은 경고가 없다(수리 방식의 타당성)", not mut_ok, repr(mut_ok))

if fails:
    print("\n%d FAIL: %s" % (len(fails), ", ".join(fails)))
    sys.exit(1)
print("\nALL PASS")
print("PACK-SYNTAX-WARNINGS-OK")
sys.exit(0)
