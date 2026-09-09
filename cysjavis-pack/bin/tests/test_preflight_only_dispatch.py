#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_preflight_only_dispatch.py — `preflight --only` 표적 재측정의 **디스패치 컷**과 실패 방향
(성찰 P5 · 2026-09-10).

## 무엇을 막는가
종전 `--only` 는 **보고 계층에서만** 걸렀다: `run()` 은 81개 체크를 전량 순회하고 `add()`/`skipped()`
가 표적 밖 행을 버렸다. 그래서
  ⓐ `--only C28.self-corection`(오타) → `preflight: READY … 검사 0`, **rc 0**
  ⓑ `--only NOPE --json` → `{"ok": true, "checks": []}`
즉 **아무것도 재지 않은 실행이 스스로 READY 를 선언**했다(실패 방향이 초록 · C58 이 '쌍 0 → SKIP'
으로 구현한 §3-3 원칙의 역전). 게다가 `--help` 는 "이 검사만 실행" 이라 적는데 실제로는 "이 행만
기록" 이라, 조기 반환으로 부작용을 내는 체크가 `--fix --only` 에서 **행 없이 부작용만** 남길 수 있었다.

## 이 파일이 못박는 것
  1) 미존재 id → rc 2 · `ok:false` · `error:"only-no-match"` · `checks:[]`(READY 아님)
  2) 가족은 존재하는데 id 가 오타 → rc 2(디스패치는 지나가지만 **행 0** 이 사용법 오류로 접힌다)
  3) 정상 id 1개 → rc 0 · 그 행 **하나만**
  4) 컷이 **디스패치**에 있다 — `_dispatch_only` 가 전체 목록에서 그 체크만 남긴다(보고 필터 아님)
  5) 가족 토큰(`--only C03`)도 표적이다(동적 id `C03.pin.<파일>` 재측정용)
음성 대조(검출력): 종전 술어(= 보고 계층 필터만)를 그대로 재현해 **같은 입력에서 rc 0·행 0** 이
나오는지 확인한다 — 이 테스트가 무엇을 잡는지 매 실행 스스로 증명한다.

실행: CYS_PACK_DIR="$(mktemp -d)" python3 bin/tests/test_preflight_only_dispatch.py
출력: PASS/FAIL 행 · 실패 시 exit 1 · 전부 통과 시 종료 토큰 ONLY-DISPATCH-OK.
"""
import json
import os
import subprocess
import sys
import tempfile

TESTS_DIR = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(TESTS_DIR)
sys.path.insert(0, BIN)

import javis_preflight as pf  # noqa: E402

PREFLIGHT = os.path.join(BIN, "javis_preflight.py")
fails = []


def check(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
    if not cond:
        fails.append(name)


def run_only(*ids, **kw):
    """격리 팩에서 `preflight --only …` 실행 → (rc, stdout)."""
    env = dict(os.environ)
    env["CYS_PACK_DIR"] = kw.get("pack") or tempfile.mkdtemp(prefix="only-pack-")
    env["CYS_NO_AUTOSTART"] = "1"
    cmd = [sys.executable, PREFLIGHT]
    for i in ids:
        cmd += ["--only", i]
    if kw.get("json", True):
        cmd.append("--json")
    p = subprocess.run(cmd, capture_output=True, text=True, env=env, timeout=180)
    return p.returncode, p.stdout


# ── 1. 미존재 id → 사용법 오류(rc 2) ────────────────────────────────────────────
rc, out = run_only("NOPE")
try:
    doc = json.loads(out)
except ValueError:
    doc = {}
check("1 미존재 id --only NOPE → rc 2 · ok:false · checks 0",
      rc == 2 and doc.get("ok") is False and doc.get("checks") == []
      and doc.get("error") == "only-no-match", "rc=%s doc=%s" % (rc, list(doc)[:6]))

# ── 2. 가족은 맞고 id 가 오타 → 행 0 이 rc 2 로 접힌다 ──────────────────────────
rc2, out2 = run_only("C28.self-corection")
try:
    doc2 = json.loads(out2)
except ValueError:
    doc2 = {}
check("2 오타 id(C28.self-corection) → rc 2 · ok:false(종전엔 READY rc 0)",
      rc2 == 2 and doc2.get("ok") is False and doc2.get("checks") == [],
      "rc=%s" % rc2)

# ── 3. 정상 id 1개 → rc 0 · 그 행 하나만 ────────────────────────────────────────
rc3, out3 = run_only("C14.preflight-self")
try:
    doc3 = json.loads(out3)
except ValueError:
    doc3 = {}
rows = doc3.get("checks") or []
# rc 는 **그 한 행의 판정**을 그대로 따른다(격리 팩에선 FAIL 일 수 있다 — 사용법 오류 2 와는 다른 값).
_expect_rc = 1 if any(r.get("status") == pf.FAIL for r in rows) else 0
check("3 정상 id 1행 통과 — 행 1 · id 일치 · rc 는 그 행의 판정(사용법 오류 2 아님)",
      rc3 == _expect_rc and rc3 != 2 and len(rows) == 1
      and rows[0]["id"] == "C14.preflight-self",
      "rc=%s rows=%s" % (rc3, [(r.get("id"), r.get("status")) for r in rows]))


# ── 4. 컷이 디스패치에 있다(보고 필터가 아니라) ─────────────────────────────────
p_all = pf.Preflight(fix=False, skips=[], mode="report")
p_one = pf.Preflight(fix=False, skips=[], mode="report", only=["C14.preflight-self"])
all_checks = [getattr(p_all, n) for n in dir(p_all)
              if n.startswith("c") and callable(getattr(p_all, n, None))
              and pf.Preflight._check_family(getattr(p_all, n))]
cut = p_one._dispatch_only(all_checks)
check("4 _dispatch_only 는 그 가족의 체크만 남긴다(전량 순회 0)",
      len(all_checks) > 50 and [c.__name__ for c in cut] == ["c14_self"],
      "%d → %s" % (len(all_checks), [c.__name__ for c in cut]))

raised = False
try:
    p_bad = pf.Preflight(fix=False, skips=[], mode="report", only=["ZZ9.nope"])
    p_bad._dispatch_only(all_checks)
except pf.OnlyUsageError:
    raised = True
check("4b 미존재 가족은 OnlyUsageError(디스패치에서 중단 — 부작용 0)", raised)

# ── 5. 가족 토큰도 표적(동적 id 재측정) ─────────────────────────────────────────
p_fam = pf.Preflight(fix=False, skips=[], mode="report", only=["C03"])
check("5 가족 토큰 --only C03 은 C03.* 를 표적으로 본다",
      p_fam._only_match("C03.pin.master") and p_fam._only_match("C03.boot-contract")
      and not p_fam._only_match("C28.self-correction"))

check("5b 체크 이름 → 가족 매핑(접미 문자 보존)",
      pf.Preflight._check_family(p_all.c11b_cys_dept_path) == "C11b"
      and pf.Preflight._check_family(p_all.c03_content_pins) == "C03"
      and pf.Preflight._cid_family("C03.pin.master") == "C03")


# ── 6. 음성 대조 — 종전 술어(보고 계층 필터만)는 같은 입력에서 rc 0·행 0 ────────
def _legacy_only_run(only, cids):
    """종전 구현 재현: 전량 순회 + 보고 계층에서만 정확 id 필터 → (rows, rc)."""
    rows = [c for c in cids if c in only]
    return rows, (0 if not [r for r in rows if r == "FAIL"] else 1)

legacy_rows, legacy_rc = _legacy_only_run({"C28.self-corection"}, ["C28.self-correction"])
check("6 검출력: 종전 술어는 같은 오타에서 행 0 · rc 0(= READY) 였다",
      legacy_rows == [] and legacy_rc == 0)

print("\n=== %d checks · FAIL %d ===" % (8, len(fails)))
if fails:
    print("FAIL: %s" % fails, file=sys.stderr)
    sys.exit(1)
print("ONLY-DISPATCH-OK")
