#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_orchestra_todo_path.py — 위임 티켓의 todo 경로 단일 매핑 + 장부 id 배선 회귀 (0.14.30 A4 #10 · PREP #10).

## 무엇을 막는가
(A) **todo 경로 이원화**: 티켓이 박는 todo 파일명과 데몬 `cys todo-path` 가 산출하는 파일명이 갈리면,
    워커가 만든 파일이 자기 팩에서 남의 것으로 배제되거나 진행% 집계에서 사라진다. 복수 워커 레인
    (dept-1 의 worker-2~5)에서 실제로 문제가 되는 형상이라, 두 생산자가 **같은 규칙 함수 하나**를
    쓰는지 고정한다(문자열 사본이 생기면 언젠가 갈린다).
(B) **플레이스홀더 방치**: E1·P3 블록의 `<id>`·`<task-id>` 를 워커가 치환하지 않으면 done 게이트가
    대상 불일치로 거부한다 — 그 실패는 티켓을 읽을 때가 아니라 **완료 보고 직전**에 드러나 재작업이
    된다. 발부자가 아는 값은 발부 시점에 박고(--task-id), 모르면 그 사실을 stderr 로 고지한다.

## 이 파일이 못박는 것
  A1 역할→파일명 규칙(대문자·하이픈→언더스코어)이 단일 함수 `todo_file_name` 하나다.
  A2 티켓의 todo 경로가 그 함수 산출과 정확히 같다(worker-2·reviewer-gemini·cso-1).
  A3 티켓 경로는 **발부 시점 절대경로**다(워커 셸에서 늦게 전개되는 문자열이 아니다 — S14 회귀).
  A4 (라이브 cys 가용 시) `cys todo-path --role worker-2` 의 basename 이 같은 함수 산출과 일치.
     cys 부재·데몬 미응답이면 **정직하게 skip**(측정 불능을 통과로 적지 않는다).
  B1 --task-id 지정 시 E1 set-status·증거 경로·probe --task 가 실 id 로 치환되고 플레이스홀더 0.
  B2 --task-id 미지정 시 티켓이 **byte-동일**(하위호환)이고 stderr 에 고지 1줄.
  B3 부적격 id(공백·경로 문자)는 exit 2 로 거부(문자열이 명령문에 그대로 보간되므로 위생 필수).

실행(CI 동형): CYS_PACK_DIR="$(mktemp -d)" python3 bin/tests/test_orchestra_todo_path.py
출력: PASS/FAIL 행 · 실패 시 exit 1 · 전부 통과 시 종료 토큰 ORCHESTRA-TODO-PATH-OK.
"""
import os
import shutil
import subprocess
import sys

SELF = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(SELF)
ORCH = os.path.join(BIN, "javis_orchestra.py")
PY = sys.executable or "python3"
sys.path.insert(0, BIN)

import javis_orchestra as orch  # noqa: E402 — 순수 규칙 함수 직접 핀

fails = []


def check(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
    if not cond:
        fails.append(name)


def ticket(to_role, **kw):
    return orch.build_task_ticket("T", "S", "C", to_role, rules=orch.FALLBACK_RULES, **kw)


# ── A. todo 경로 단일 매핑 ─────────────────────────────────────────────────────
check("A1 규칙 함수 단일 존재(todo_file_name)", callable(getattr(orch, "todo_file_name", None)))
for role, want in (("worker", "WORKER_TODO.md"), ("worker-2", "WORKER_2_TODO.md"),
                   ("reviewer-gemini", "REVIEWER_GEMINI_TODO.md"), ("cso-1", "CSO_1_TODO.md")):
    check("A1b 규칙: %s → %s" % (role, want), orch.todo_file_name(role) == want,
          repr(orch.todo_file_name(role)))

pack_root, _scope = orch._pack_identity()
for role in ("worker", "worker-2", "reviewer-gemini"):
    want_path = os.path.join(pack_root, "round", orch.todo_file_name(role))
    check("A2 티켓 todo 경로 = 규칙 함수 산출(%s)" % role, want_path in ticket(role), want_path)

check("A3 티켓 경로가 발부 시점 절대경로(늦은 셸 전개 아님 — S14 회귀)",
      "${CYS_PACK_DIR" not in ticket("worker").split("todo 영속:")[1].split("\n")[0])

# A4 — 라이브 데몬 대조(가용할 때만 · 측정 불능은 skip 으로 정직하게 적는다)
cys = shutil.which("cys")
if cys and os.environ.get("CYS_SURFACE_ID"):
    r = subprocess.run([cys, "todo-path", "--role", "worker-2"], capture_output=True,
                       text=True, encoding="utf-8", timeout=30)
    if r.returncode == 0 and (r.stdout or "").strip():
        check("A4 데몬 todo-path basename = 규칙 함수 산출(생산자 2벌 일치)",
              os.path.basename(r.stdout.strip()) == orch.todo_file_name("worker-2"),
              "daemon=%r rule=%r" % (os.path.basename(r.stdout.strip()),
                                     orch.todo_file_name("worker-2")))
    else:
        print("SKIP A4 데몬 todo-path 조회 실패(rc=%s) — 측정 불능(통과 아님)" % r.returncode)
else:
    print("SKIP A4 cys 부재 또는 pane 밖 실행 — 측정 불능(통과 아님)")

# ── B. --task-id 배선 ──────────────────────────────────────────────────────────
t_id = ticket("worker", probes=["submit"], task_id="t-abc.1")
t_no = ticket("worker", probes=["submit"])
check("B1a E1 set-status 가 실 id 로 치환", "set-status t-abc.1 done" in t_id)
check("B1b 증거 경로가 실 id 로 치환", "_round/evidence/t-abc.1/" in t_id)
check("B1c probe --task 가 실 id 로 치환", "--task t-abc.1" in t_id)
check("B1d 플레이스홀더 0(치환 누락 시 done 게이트가 거부한다)",
      "<id>" not in t_id and "<task-id>" not in t_id)
check("B2a 미지정 티켓은 종전 플레이스홀더 유지(하위호환)",
      "set-status <id> done" in t_no and "<task-id>" in t_no)
check("B2b task_id=None 은 기본값과 byte-동일",
      ticket("worker", probes=["submit"], task_id=None) == t_no)

env = dict(os.environ)
env["CYS_PACK_DIR"] = env.get("CYS_PACK_DIR") or "/tmp"
r_no = subprocess.run([PY, ORCH, "task-prompt", "--task", "T", "--scope", "S",
                       "--to", "worker", "--no-survival-gate"],
                      capture_output=True, text=True, encoding="utf-8", env=env, timeout=60)
check("B2c 미지정 시 stderr 고지 1줄(stdout 무오염)",
      r_no.returncode == 0 and "task id 미지정" in r_no.stderr
      and "task id 미지정" not in r_no.stdout, repr(r_no.stderr[:120]))
r_id = subprocess.run([PY, ORCH, "task-prompt", "--task", "T", "--scope", "S",
                       "--to", "worker", "--no-survival-gate", "--task-id", "t-1"],
                      capture_output=True, text=True, encoding="utf-8", env=env, timeout=60)
check("B2d 지정 시 고지 없음", r_id.returncode == 0 and "task id 미지정" not in r_id.stderr,
      repr(r_id.stderr[:120]))
r_bad = subprocess.run([PY, ORCH, "task-prompt", "--task", "T", "--scope", "S",
                        "--to", "worker", "--no-survival-gate", "--task-id", "../etc/passwd"],
                       capture_output=True, text=True, encoding="utf-8", env=env, timeout=60)
check("B3 부적격 id 거부(exit 2 · 명령문 보간 위생)",
      r_bad.returncode == 2 and "--task-id" in r_bad.stderr, "rc=%s" % r_bad.returncode)

total = 17
print("\n=== %d/%d PASS ===" % (total - len(fails), total))
if fails:
    print("FAIL: %s" % fails, file=sys.stderr)
    sys.exit(1)
print("ORCHESTRA-TODO-PATH-OK")
