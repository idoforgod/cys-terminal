#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_refl_stagnation_budget_ledger.py — 성찰 R4 N3·N18: 라운드 종결 집행과 구 장부 이주.

무엇을 막는가
  ① N3 — `round_stop_reason` 은 `stopped_budget` 을 `stopped_stagnation` **앞에서** 반환하는데
     (정본 우선순위: 헌장의 하드 상한이 정체보다 강하다) `stagnation_gate` 는 `stopped_stagnation`
     하나만 집행했다. 그래서 R10 에 상한이 차는 순간 사유가 budget 으로 바뀌며 **종결 게이트가
     통째로 무장 해제**된다 — R11·R12 가 rc 0 으로 열리고 `stagnation_stop` 은 0건인데 도구는
     "무한 루프 금지" 를 출력한다(실측). 게이트가 무는 구간이 R3~R10 뿐이고 그 뒤는 영구 자유였다.
  ② N18 — 구 장부(0.14.30 · `task-id` 주석 없음)의 신원 비교가 `disp`(장부에서 `.strip()` 된 값)
     대 `str(task)`(원문) 였다. 후행 공백 표기로 만든 장부에 **같은 표기로** append 해도 영원히
     거부됐고, 안내는 "장부 표기 그대로 `--task` 를 줘라"(이미 한 일을 하라는 말)였다. 이 검사는
     다른 무엇보다 먼저 돌고 override 경로가 없어 **진행 중이던 라운드 장부가 업그레이드 순간
     append 불능**이 된다(자율 루프 정지 방향).

출력: PASS/FAIL 행 · 실패 시 exit 1 · 전부 통과 시 종료 토큰 REFL-STAGNATION-BUDGET-OK.
실행: python3 cysjavis-pack/bin/tests/test_refl_stagnation_budget_ledger.py
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile

SELF = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(SELF)
sys.path.insert(0, BIN)
PY = sys.executable
ORCH = os.path.join(BIN, "javis_orchestra.py")

fails = []


def check(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
    if not cond:
        fails.append(name)


import javis_orchestra as ORC          # noqa: E402


def pack_env(pack):
    env = dict(os.environ)
    env["CYS_PACK_DIR"] = pack
    for k in ("CYS_ROUND_DIR", "JAVIS_PACK_DIR", "AITERM_PACK_DIR", "AITERM_JARVIS_DIR"):
        env.pop(k, None)
    return env


def round_log(env, task, rnd, extra=()):
    return subprocess.run([PY, ORCH, "round-log", "--task", task, "--round", str(rnd),
                           "--evaluator", "worker", "--verdict", "ACCEPT"] + list(extra),
                          capture_output=True, text=True, env=env)


def events(pack, task):
    p = ORC.round_events_path(task)          # CYS_PACK_DIR 는 아래에서 세운다
    if not os.path.isfile(p):
        return []
    out = []
    for ln in open(p, encoding="utf-8"):
        ln = ln.strip()
        if ln:
            try:
                out.append(json.loads(ln))
            except ValueError:
                pass
    return out


# ── ① N3: R10 이후 종결 집행 ─────────────────────────────────────────────────
root = tempfile.mkdtemp()
try:
    pack = os.path.join(root, "pack")
    os.makedirs(os.path.join(pack, "round"))
    env = pack_env(pack)
    task = "REFL-N3-BUDGET"
    _saved = os.environ.get("CYS_PACK_DIR")
    os.environ["CYS_PACK_DIR"] = pack
    for k in ("CYS_ROUND_DIR", "JAVIS_PACK_DIR", "AITERM_PACK_DIR", "AITERM_JARVIS_DIR"):
        os.environ.pop(k, None)
    ledger = ORC.round_path(task)

    # R1~R10 을 장부에 직접 세운다(합격 4자 전원 승인은 없다 → accepted 아님).
    body = [ORC.ledger_header_text(task).rstrip("\n")]
    for r in range(1, ORC.MAX_ROUNDS + 1):
        body.append("| %d | worker | 기록 | ACCEPT |" % r)
    open(ledger, "w", encoding="utf-8").write("\n".join(body) + "\n")

    rows = ORC.parse_rounds(ledger)
    check("0a 전제: R1~R10 이 파싱된다", len(rows) == ORC.MAX_ROUNDS, repr(len(rows)))
    reason, why = ORC.round_stop_reason(rows, {})
    check("0b 전제: R10 의 stop_reason 은 stopped_budget", reason == "stopped_budget",
          repr((reason, why)))

    r11 = round_log(env, task, 11)
    check("1a R11 이 rc≠0 으로 거부된다", r11.returncode != 0,
          "rc=%s out=%r err=%r" % (r11.returncode, r11.stdout[-200:], r11.stderr[-300:]))
    check("1b 거부는 정체 종결 exit(3)", r11.returncode == ORC.ROUND_LOG_EXIT_STAGNATION,
          str(r11.returncode))
    check("1c 사유가 stopped_budget 으로 표기된다",
          "stopped_budget" in (r11.stderr or ""), (r11.stderr or "")[-300:])
    evs = events(pack, task)
    stops = [e for e in evs if e.get("event") == "stagnation_stop"]
    check("1d stagnation_stop 이 정확히 1건", len(stops) == 1, repr(evs))
    check("1e 장부에 R11 행이 남지 않았다", "| 11 |" not in open(ledger, encoding="utf-8").read())

    # R12 도 계속 막힌다(끈끈한 종결) — 종전에는 여기가 영구 자유였다.
    r12 = round_log(env, task, 12)
    check("1f R12 도 막힌다", r12.returncode == ORC.ROUND_LOG_EXIT_STAGNATION,
          "rc=%s %r" % (r12.returncode, (r12.stderr or "")[-200:]))
    check("1g 종결은 중복 기록되지 않는다",
          len([e for e in events(pack, task) if e.get("event") == "stagnation_stop"]) == 1)

    # --override 로만 열린다.
    r11b = round_log(env, task, 11, ["--override", "오너 승인 — 상한 연장"])
    check("1h --override 로 열린다", r11b.returncode in (0, 1),
          "rc=%s %r" % (r11b.returncode, (r11b.stderr or "")[-300:]))
    check("1i override 가 사이드카에 기록된다",
          any(e.get("event") == "override" for e in events(pack, task)))
    check("1j R11 행이 남았다", "| 11 |" in open(ledger, encoding="utf-8").read())

    # ★음성 대조 — 상한 **전**(R2)은 종전대로 열려 있어야 한다(게이트를 넓히지 않았다는 증거).
    task2 = "REFL-N3-OPEN"
    ledger2 = ORC.round_path(task2)
    open(ledger2, "w", encoding="utf-8").write(
        ORC.ledger_header_text(task2).rstrip("\n") + "\n| 1 | worker | 기록 | ACCEPT |\n")
    r2 = round_log(env, task2, 2)
    check("1k 음성 대조: 상한 전 라운드는 그대로 열린다", r2.returncode in (0, 1),
          "rc=%s %r" % (r2.returncode, (r2.stderr or "")[-300:]))
finally:
    if _saved is None:
        os.environ.pop("CYS_PACK_DIR", None)
    else:
        os.environ["CYS_PACK_DIR"] = _saved
    shutil.rmtree(root, ignore_errors=True)


# ── ② N18: 구 장부 신원 비교·이주 ────────────────────────────────────────────
root = tempfile.mkdtemp()
try:
    pack = os.path.join(root, "pack")
    os.makedirs(os.path.join(pack, "round"))
    env = pack_env(pack)
    _saved = os.environ.get("CYS_PACK_DIR")
    os.environ["CYS_PACK_DIR"] = pack
    for k in ("CYS_ROUND_DIR", "JAVIS_PACK_DIR", "AITERM_PACK_DIR", "AITERM_JARVIS_DIR"):
        os.environ.pop(k, None)

    # 0.14.30 형상: 헤더에 **원문 그대로** · `task-id` 주석 없음.
    task = "WP6 정체 "                            # ★후행 공백 표기(실측 재현)
    ledger = ORC.round_path(task)
    open(ledger, "w", encoding="utf-8").write(
        "# ORCHESTRATION 라운드 장부 — %s\n\n"
        "| 라운드 | 평가자 | 기록값 | 판정 |\n|---|---|---|---|\n"
        "| 1 | worker | 기록 | ACCEPT |\n" % task)
    disp, tid = ORC.ledger_identity(ledger)
    check("2a 전제: 구 장부에 task-id 가 없다", tid is None and disp == "WP6 정체", repr((disp, tid)))
    check("2b 같은 표기의 append 가 거부되지 않는다",
          ORC.ledger_owner_mismatch(ledger, task) is None,
          repr(ORC.ledger_owner_mismatch(ledger, task)))

    r = round_log(env, task, 2)
    check("2c round-log 가 실제로 통과한다", r.returncode in (0, 1),
          "rc=%s %r" % (r.returncode, (r.stderr or "")[-300:]))
    _d2, tid2 = ORC.ledger_identity(ledger)
    check("2d 첫 성공 기록에서 task-id 가 백필된다", tid2 == ORC.task_id(task), repr(tid2))
    check("2e 백필 뒤에도 행이 보존된다",
          open(ledger, encoding="utf-8").read().count("| 1 | worker") == 1)
    check("2f 백필 뒤 신원은 정확 비교로 넘어간다",
          ORC.ledger_owner_mismatch(ledger, task) is None
          and ORC.ledger_owner_mismatch(ledger, "WP6 다른것") is not None)

    # ★음성 대조 — **진짜 다른 task** 는 여전히 거부되고, 안내에 이주 방법이 있다.
    task_b = "WP6 완전히 다른 작업"
    ledger_b = ORC.round_path("WP6 완전히 다른 작업")
    other = "# ORCHESTRATION 라운드 장부 — WP6 남의 작업\n\n| 라운드 | 평가자 | 기록값 | 판정 |\n"
    open(ledger_b, "w", encoding="utf-8").write(other)
    msg = ORC.ledger_owner_mismatch(ledger_b, task_b)
    check("2g 진짜 다른 task 는 거부된다", msg is not None, repr(msg))
    check("2h 거부 문면에 이주 안내가 있다",
          msg is not None and "task-id:" in msg and "헤더 줄을 직접 고쳐 이주" in msg, repr(msg))

    # 개행 표기(헤더에 담을 수 없는 형상) — 거부하되 안내가 실행 가능한 지시를 준다.
    nl_task = "WP6\n정체"
    nl_ledger = ORC.round_path(nl_task)
    open(nl_ledger, "w", encoding="utf-8").write(
        "# ORCHESTRATION 라운드 장부 — WP6\n정체\n\n| 라운드 | 평가자 | 기록값 | 판정 |\n")
    nmsg = ORC.ledger_owner_mismatch(nl_ledger, nl_task)
    check("2i 개행 표기 구 장부는 이주 안내와 함께 거부",
          nmsg is not None and ORC.task_id(nl_task) in nmsg, repr(nmsg))
    # 안내대로 task-id 를 심으면 통과한다(안내가 실제로 통하는가 — 문면의 진위 검증).
    body = open(nl_ledger, encoding="utf-8").read().split("\n")
    body.insert(1, "<!-- task-id: %s -->" % ORC.task_id(nl_task))
    open(nl_ledger, "w", encoding="utf-8").write("\n".join(body))
    check("2j 안내대로 이주하면 통과한다", ORC.ledger_owner_mismatch(nl_ledger, nl_task) is None,
          repr(ORC.ledger_owner_mismatch(nl_ledger, nl_task)))
finally:
    if _saved is None:
        os.environ.pop("CYS_PACK_DIR", None)
    else:
        os.environ["CYS_PACK_DIR"] = _saved
    shutil.rmtree(root, ignore_errors=True)

print("REFL-STAGNATION-BUDGET-OK" if not fails else "FAILED: %s" % ", ".join(fails))
sys.exit(1 if fails else 0)
