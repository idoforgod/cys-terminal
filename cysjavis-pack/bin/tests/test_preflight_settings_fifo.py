#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_preflight_settings_fifo.py — `settings.json` 자리의 **writer 없는 FIFO** 에서 C28 판독기가
유계로 접히는지(성찰 P17 · 2026-09-10).

## 무엇을 막는가
`.claude.json` 에 적용한 FIFO 정지 하드닝(`_read_json_tolerant` = O_NONBLOCK + fstat 정규 재확인)이
**같은 커밋에서 신설된 `settings.json` 판독기 2개**(`_event_hook_scope_ok`·`_event_hook_present_any`)
에는 적용되지 않았다. 두 함수는 C28 이 대상 프로필마다 부르고, C28 은 부트 체인이 자동으로 도는
유일한 검사다 — 그 자리에서 영구 정지하면 뒤 체크가 통째로 날아간다(관측 0 · rc 124).

## 이 파일이 못박는 것
  1) FIFO 를 `settings.json` 자리에 둔 상태에서 판독기 4종이 **유계 시간 안에** 판정을 낸다.
  2) 판정 방향은 종전과 같다(등록/잔존 = False · 범위 축 = True `판독 불가는 이 축의 사실이 아니다`).
  3) 음성 대조(검출력): 같은 FIFO 에 **무가드 `json.load(open(...))`** 는 실제로 막힌다 —
     이 테스트가 무엇을 잡는지 매 실행 스스로 증명한다.
POSIX 전용 축이라 `os.mkfifo` 가 없는 플랫폼(Windows)은 SKIP 한다(측정 불가를 통과로 접지 않는다).

실행: python3 bin/tests/test_preflight_settings_fifo.py
출력: PASS/FAIL 행 · 실패 시 exit 1 · 전부 통과 시 종료 토큰 SETTINGS-FIFO-OK.
"""
import json
import os
import shutil
import sys
import tempfile
import threading

TESTS_DIR = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(TESTS_DIR)
sys.path.insert(0, BIN)

import javis_preflight as pf  # noqa: E402

BUDGET_S = 10.0          # '유계' 의 상한 — 무가드 open 은 여기서 절대 못 돌아온다(writer 0)
fails = []


def check(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
    if not cond:
        fails.append(name)


def bounded(fn, *a):
    """fn(*a) 를 데몬 스레드로 돌려 BUDGET_S 안에 끝나면 (True, 값) · 아니면 (False, None).
    데몬 스레드라 막혀도 인터프리터 종료를 막지 않는다(FIFO 는 프로세스 종료로 풀린다)."""
    box = {}

    def _w():
        try:
            box["v"] = fn(*a)
        except BaseException as e:            # noqa: BLE001 — 예외도 '끝났다' 이다(정지가 아니다)
            box["v"] = ("EXC", type(e).__name__)
    t = threading.Thread(target=_w, daemon=True)
    t.start()
    t.join(BUDGET_S)
    return (not t.is_alive()), box.get("v")


if not hasattr(os, "mkfifo"):
    print("SKIP settings.json FIFO 축 — 이 플랫폼에 os.mkfifo 가 없다(측정 불가 · 통과 아님)")
    print("SETTINGS-FIFO-OK")
    sys.exit(0)

d = tempfile.mkdtemp(prefix="fifo-settings-")
try:
    fifo = os.path.join(d, "settings.json")
    os.mkfifo(fifo)

    # ── 3(먼저) 음성 대조: 무가드 판독은 실제로 막힌다 ────────────────────────────
    done_raw, _ = bounded(lambda p: json.load(open(p, encoding="utf-8")), fifo)
    check("3 검출력: 무가드 json.load(open(FIFO)) 는 %ds 안에 돌아오지 않는다(종전 형상)" % BUDGET_S,
          not done_raw)

    # ── 1·2 판독기 4종은 유계로 접히고 방향은 종전과 같다 ────────────────────────
    cases = [
        ("_event_hook_registered", lambda: pf.Preflight._event_hook_registered(
            fifo, "PreToolUse", "role-capability-gate.sh", 15), False),
        ("_event_hook_scope_ok", lambda: pf.Preflight._event_hook_scope_ok(
            fifo, "PreToolUse", "role-capability-gate.sh", None), True),
        ("_event_hook_present_any", lambda: pf.Preflight._event_hook_present_any(
            fifo, "PreToolUse", "role-capability-gate.sh"), False),
        ("_guard_wired", lambda: pf.Preflight._guard_wired(fifo), False),
    ]
    for name, fn, want in cases:
        done, got = bounded(fn)
        check("1 %s 는 FIFO 에서 %ds 안에 판정한다" % (name, BUDGET_S), done, "got=%r" % (got,))
        check("2 %s 의 판정 방향은 종전과 같다(%r)" % (name, want), done and got is want,
              "got=%r" % (got,))
finally:
    shutil.rmtree(d, ignore_errors=True)

print("\n=== FAIL %d ===" % len(fails))
if fails:
    print("FAIL: %s" % fails, file=sys.stderr)
    sys.exit(1)
print("SETTINGS-FIFO-OK")
