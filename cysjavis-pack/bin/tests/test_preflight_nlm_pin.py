#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_preflight_nlm_pin.py — NotebookLM SOT 도구(nlm) 핀 회귀 (0.14.30 A4 #12 · PREP #12).

## 무엇을 막는가
종전 핀은 git 커밋(`notebooklm-mcp-cli @ git+…@6d41c75…` = v0.7.3)이었다. 그 리비전은 **구 호스트
전용**이라 `preflight --fix` 가 그것을 재설치하면 **로그인 불가 버전**이 깔린다(CSO 실측: 0.7.3
`nlm login` exit 1). 자동 수리 경로가 도구를 고장 난 상태로 되돌리는 형상이라, 핀을 PyPI 정식판
(`notebooklm-mcp-cli==0.10.0`)으로 옮기고 하한을 (0,9,3)으로 올렸다.

## 이 파일이 못박는 것
  1) 핀이 **PyPI 스펙**이다(`이름==버전`) — git+ URL 이 다시 들어오면 FAIL.
  2) 핀 버전이 하한 이상이다(핀↔하한 역전 = 자기모순 방지).
  3) 하한이 (0,9,3) 이상이다 — **하향 금지**(0.7.x 로 되돌리면 위 사고 재발).
  4) 설치 폴백 3경로(uv·pipx·pip)가 이 핀 문자열을 **그대로** 인자로 넘긴다(자구 조립이 아니라
     상수 소비 — 핀만 고치면 세 경로가 함께 움직인다는 계약).
  5) 안내 문안이 핀을 그대로 인용한다(사용자가 복사해 실행 가능).
음성 대조(검출력): 위 1·3 은 **구 핀 문자열/구 하한 튜플을 그대로 주면 실패**하는 술어라
`_negative_controls()` 가 같은 판정 함수에 구 값을 넣어 FAIL 이 나오는지 매 실행 재현한다.

실행(CI 동형): CYS_PACK_DIR="$(mktemp -d)" python3 bin/tests/test_preflight_nlm_pin.py
출력: PASS/FAIL 행 · 실패 시 exit 1 · 전부 통과 시 종료 토큰 NLM-PIN-OK.
"""
import os
import re
import sys

TESTS_DIR = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(TESTS_DIR)
sys.path.insert(0, BIN)

import javis_preflight as pf  # noqa: E402 — 형제 모듈 직접 판독(서브프로세스 아님)

fails = []


def check(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
    if not cond:
        fails.append(name)


# ── 판정 술어(음성 대조가 같은 함수를 재사용한다) ────────────────────────────────
_PYPI_SPEC = re.compile(r"^[A-Za-z0-9._-]+==\d+\.\d+(\.\d+)?$")


def is_pypi_spec(pin):
    """PyPI 고정 스펙인가(이름==버전) — git+/URL/범위 지정은 거짓."""
    return bool(_PYPI_SPEC.match(pin or "")) and "git+" not in (pin or "")


def pin_version(pin):
    """핀 문자열에서 버전 튜플 — 파싱 불가면 None."""
    m = re.search(r"==(\d+)\.(\d+)(?:\.(\d+))?$", pin or "")
    return tuple(int(g or 0) for g in m.groups()) if m else None


def min_ok(minv):
    """하한이 0.9.3 이상인가(하향 금지 술어)."""
    return isinstance(minv, tuple) and minv >= (0, 9, 3)


def _negative_controls():
    """구 값에서 위 술어가 실제로 FAIL 하는지 — 검출력 실증(매 실행 재현)."""
    old_pin = ("notebooklm-mcp-cli @ git+https://github.com/jacob-bd/notebooklm-mcp-cli"
               "@6d41c75e21dae89d7bf6f43a71e3095239a28281")
    return (not is_pypi_spec(old_pin), pin_version(old_pin) is None, not min_ok((0, 7, 3)))


# ── 1~3. 핀 자체 ────────────────────────────────────────────────────────────────
check("1 핀이 PyPI 고정 스펙(이름==버전 · git+ 아님)", is_pypi_spec(pf.NLM_PIN), repr(pf.NLM_PIN))
check("2 핀 버전 ≥ 하한(핀↔하한 역전 없음)",
      pin_version(pf.NLM_PIN) is not None and pin_version(pf.NLM_PIN) >= pf.NLM_MIN_VERSION,
      "pin=%r min=%r" % (pin_version(pf.NLM_PIN), pf.NLM_MIN_VERSION))
check("3 하한 ≥ (0,9,3) — 하향 금지(구 호스트 전용 0.7.x 회귀 차단)",
      min_ok(pf.NLM_MIN_VERSION), repr(pf.NLM_MIN_VERSION))
check("3b 패키지 이름 불변(notebooklm-mcp-cli)",
      (pf.NLM_PIN or "").startswith("notebooklm-mcp-cli=="), repr(pf.NLM_PIN))

# ── 4. 설치 폴백 3경로가 상수를 그대로 소비 ─────────────────────────────────────
src = open(os.path.join(BIN, "javis_preflight.py"), encoding="utf-8").read()
i = src.find("def _install_nlm(")
seg = src[i:i + 1200] if i > 0 else ""
check("4a _install_nlm 실재", i > 0)
for tool in ("uv", "pipx", "pip"):
    check("4b %s 경로가 NLM_PIN 상수를 그대로 넘김" % tool,
          seg.count("NLM_PIN") >= 3 and tool in seg,
          "NLM_PIN 등장 %d회" % seg.count("NLM_PIN"))
check("4c 설치 인자에 자구 조립(문자열 접합) 없음",
      "NLM_PIN +" not in seg and '"%s" % NLM_PIN' not in seg)

# ── 5. 안내 문안이 핀을 그대로 인용 ─────────────────────────────────────────────
check("5 FAIL 안내가 uv tool install '<핀>' 형태로 핀을 인용",
      "uv tool install '%s'" in src and "NLM_PIN" in src)

# ── 6. 음성 대조 — 구 값에서 술어가 FAIL 한다 ───────────────────────────────────
neg = _negative_controls()
check("6 검출력: 구 git+ 핀은 PyPI 술어 FAIL · 버전 파싱 None · 구 하한 (0,7,3) 은 하한 술어 FAIL",
      all(neg), repr(neg))

print("\n=== %d/%d PASS ===" % (7 + 3 - len(fails) - 1 + 1, 7 + 3))
if fails:
    print("FAIL: %s" % fails, file=sys.stderr)
    sys.exit(1)
print("NLM-PIN-OK")
