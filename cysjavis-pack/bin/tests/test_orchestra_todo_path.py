#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_orchestra_todo_path.py — 위임 티켓의 todo 경로·생존 게이트가 **수신 좌석**에 묶이는가 + 장부 id 배선 회귀
(0.14.30 A4 #10 · PREP #10 · 0.14.56 #6).

## 무엇을 막는가
(A) **발부자가 고른 todo 파일**(0.14.56 #6-a): 종전 티켓은 `--to`(기본 `worker`)에서 파일명을 만들어
    발부자 팩의 절대경로로 박았다. 수신 좌석의 역할이 `--to` 와 다르면(번호 붙은 워커 · Control Center 의
    `worker-fresh-<epoch>` 일회용 좌석 · 다른 레인의 좌석) 그 좌석은 **남의 파일**(예: 비서실 `worker` 의
    WORKER_TODO.md)에 쓰라는 지시를 받았고, 두 번째 선언이 들어가면 그 파일은 미선언으로 떨어진다.
    ⇒ 티켓은 파일을 이름 붙이지 않고 수신 좌석이 `cys todo-path` 로 자기 경로·선언을 받게 한다.
    ★S14 불변식(경로와 선언 scope 는 **같은 바인딩**에서 나온다)은 더 강한 형태로 유지된다: 이제 둘 다
    수신 좌석의 한 프로세스(`cys todo-path`)가 만든다 — 발부자·수신자 사이의 이원 바인딩 자체가 없다.
(B) **플레이스홀더 방치**: E1·P3 블록의 `<id>`·`<task-id>` 를 워커가 치환하지 않으면 done 게이트가
    대상 불일치로 거부한다. 발부자가 아는 값은 발부 시점에 박고(--task-id), 모르면 stderr 로 고지한다.
(C) **역할을 잰 생존 게이트**(0.14.56 #6-b): 종전 게이트는 `--to` 역할을 가진 아무 산 좌석이나 보면
    통과했다 — 실제 수신 surface 는 입력조차 아니었다(CEO 실측: surface:133 으로 보내는 중에 stderr 가
    `worker`(s126) 를 보고했다). ⇒ `--surface N` 이면 **그 행**을 `live_roles` 와 같은 술어로 재고,
    역할만 주면 그 역할을 가진 산 좌석이 둘 이상일 때 거부하며 후보를 댄다. 이 데몬에 없는 surface 는
    거부하지 않고 'liveness UNVERIFIED' 를 stderr 와 티켓 머리에 함께 적는다(CEO 는 레인을 건너 발부한다).

## 이 파일이 못박는 것
  A1 티켓 todo 블록이 `cys todo-path` 를 지시하고 `_TODO.md` 파일·발부자 팩 경로·`${CYS_PACK_DIR` 를 싣지 않는다
     (worker · worker-2 · reviewer-gemini · worker-fresh-<epoch>).
  A2 두 워커(worker · worker-2) 티켓의 todo 블록이 바이트 동일 — 발부자가 수신자별 파일을 고르지 않는다.
  A3 발부자 선언이 없다(`<!-- javis:todo`·`scope=` 0) · 선언은 `cys todo-path --emit-decl` 로 받으라는 지시 ·
     위치(첫 체크박스보다 위)·은퇴(status=retired) 계약은 그대로.
  A4 발부자 팩 이름(부서 팩 · G4 밖 이름)이 티켓에 새지 않는다.
  A5 Control Center 일회용 좌석 경로(make_ticket 과 같은 인자 · `--to worker --no-survival-gate`)도 같다.
  A6 (pane 안일 때만) 생산자 `cys todo-path --role` 이 두 워커에 서로 다른 파일을 낸다. pane 밖은 정직하게 skip.
  B1 --task-id 지정 시 E1 set-status·증거 경로·probe --task 가 실 id 로 치환되고 플레이스홀더 0.
  B2 --task-id 미지정 시 티켓이 **byte-동일**(하위호환)이고 stderr 에 고지 1줄.
  B3 부적격 id(공백·경로 문자)는 exit 2 로 거부(문자열이 명령문에 그대로 보간되므로 위생 필수).
  C1 --surface 의 행이 죽었으면 같은 역할의 다른 산 좌석이 있어도 거부(exit 1) — 역할이 아니라 그 행을 잰다.
  C2 --surface 의 행이 살아 있으면 역할이 `worker` 가 아니어도(역할 `worker` 부재) 발부한다.
  C3 --surface 없이 역할만 주고 그 역할의 산 좌석이 둘이면 거부(exit 1)하며 둘 다 이름 댄다.
  C4 역할의 산 좌석이 하나면 종전대로 발부(exit 0).
  C5 이 데몬에 없는 surface 는 거부하지 않고 'liveness UNVERIFIED (recipient not on this daemon)' 를 stderr 와
     티켓 머리에 함께 싣는다.
  C6 idle 경고는 --surface 의 그 행을 읽는다(같은 역할의 다른 행이 아니다).
  C7 --no-survival-gate 는 status 를 부르지 않는다(종전 그대로).
  C8 --surface 값 위생(N 또는 surface:N 만) — 위반 exit 2.
  C9 CLI 가 --surface 를 받는다.
  C10 live_roles 술어는 cmd_check·self-test 용으로 종전 그대로.

실행(CI 동형): CYS_PACK_DIR="$(mktemp -d)" python3 bin/tests/test_orchestra_todo_path.py
출력: PASS/FAIL 행 · 실패 시 exit 1 · 전부 통과 시 종료 토큰 ORCHESTRA-TODO-PATH-OK.
"""
import contextlib
import io
import os
import shutil
import subprocess
import sys
import types

SELF = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(SELF)
ORCH = os.path.join(BIN, "javis_orchestra.py")
PY = sys.executable or "python3"
sys.path.insert(0, BIN)

import javis_orchestra as orch  # noqa: E402 — 순수 함수·명령 함수 직접 핀

fails = []
checked = [0]


def check(name, cond, detail=""):
    checked[0] += 1
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
    if not cond:
        fails.append(name)


def ticket(to_role, **kw):
    return orch.build_task_ticket("T", "S", "C", to_role, rules=orch.FALLBACK_RULES, **kw)


def todo_block(t):
    """`todo 영속:` 줄부터 `보고 채널:` 직전까지 — 티켓의 todo 지시 전체."""
    lines = t.splitlines()
    starts = [i for i, ln in enumerate(lines) if ln.startswith("todo 영속:")]
    ends = [i for i, ln in enumerate(lines) if ln.startswith("보고 채널:")]
    if len(starts) != 1 or len(ends) != 1 or ends[0] <= starts[0]:
        return None
    return "\n".join(lines[starts[0]:ends[0]])


@contextlib.contextmanager
def pack_env(value):
    old = os.environ.get("CYS_PACK_DIR")
    os.environ["CYS_PACK_DIR"] = value
    try:
        yield
    finally:
        if old is None:
            os.environ.pop("CYS_PACK_DIR", None)
        else:
            os.environ["CYS_PACK_DIR"] = old


# ── A. todo 경로·선언은 수신 좌석이 산출한다 ─────────────────────────────────────
FRESH = "worker-fresh-1793190000"           # schedule.rs 가 만드는 일회용 좌석 역할 꼴
issuer_pack = os.path.abspath(orch.pack_dir())
blocks = {}
for role in ("worker", "worker-2", "reviewer-gemini", FRESH):
    t = ticket(role)
    b = todo_block(t)
    blocks[role] = b
    check("A1 todo 블록 존재(%s)" % role, b is not None, repr(t[-600:]))
    b = b or ""
    check("A1 수신 좌석 산출 지시 `cys todo-path`(%s)" % role, "`cys todo-path`" in b, b[:200])
    check("A1 발부자가 고른 todo 파일 없음(%s)" % role, "_TODO.md" not in b, b[:200])
    check("A1 발부자 팩 경로 없음(%s)" % role, issuer_pack not in t, issuer_pack)
    check("A1 늦은 셸 전개 없음(%s)" % role, "${CYS_PACK_DIR" not in b)

check("A2 두 워커 티켓의 todo 블록 바이트 동일(발부자가 수신자별 파일을 고르지 않는다)",
      blocks["worker"] is not None and blocks["worker"] == blocks["worker-2"] == blocks[FRESH],
      "worker=%r worker-2=%r" % ((blocks["worker"] or "")[:120], (blocks["worker-2"] or "")[:120]))

t0 = ticket("worker")
check("A3 발부자 선언 없음(`<!-- javis:todo` 0)", "<!-- javis:todo" not in t0)
check("A3 발부자 scope 없음(`scope=` 0)", "scope=" not in t0)
check("A3 선언은 수신 좌석 생성기로(`cys todo-path --emit-decl`)", "cys todo-path --emit-decl" in t0)
check("A3 위치 계약 유지(첫 체크박스보다 위)", "첫 체크박스보다 위" in t0)
check("A3 은퇴 계약 유지(status=retired)", "status=retired" in t0)

for pk in ("/tmp/pack-dept-dept-9", "/tmp/자비스"):
    with pack_env(pk):
        tk = ticket("worker")
    leaf = os.path.basename(pk)
    check("A4 발부자 팩 이름이 티켓에 새지 않음(%s)" % leaf, leaf not in tk and pk not in tk)

env = dict(os.environ)
env["CYS_PACK_DIR"] = env.get("CYS_PACK_DIR") or "/tmp"
r_cc = subprocess.run([PY, ORCH, "task-prompt", "--task", "T", "--scope", "S", "--success", "C",
                       "--to", "worker", "--no-survival-gate", "--output-format", "F"],
                      capture_output=True, text=True, encoding="utf-8", env=env, timeout=60)
b_cc = todo_block(r_cc.stdout) or ""
check("A5 Control Center 일회용 좌석 경로(make_ticket 동형 인자) — cys todo-path 지시 · WORKER_TODO.md 없음",
      r_cc.returncode == 0 and "`cys todo-path`" in b_cc and "_TODO.md" not in r_cc.stdout
      and os.path.join(env["CYS_PACK_DIR"], "round") not in r_cc.stdout,
      "rc=%s block=%r" % (r_cc.returncode, b_cc[:160]))

# A6 — 생산자(데몬 `cys todo-path`)가 두 워커를 가른다 · pane 밖·cys 부재는 측정 불능(통과 아님)
cys = shutil.which("cys")
if cys and os.environ.get("CYS_SURFACE_ID"):
    got = {}
    for role in ("worker", "worker-2"):
        r = subprocess.run([cys, "todo-path", "--role", role], capture_output=True,
                           text=True, encoding="utf-8", timeout=30)
        got[role] = os.path.basename(r.stdout.strip()) if r.returncode == 0 else None
    check("A6 생산자 todo-path 가 두 워커에 서로 다른 파일(WORKER_TODO.md ≠ WORKER_2_TODO.md)",
          got == {"worker": "WORKER_TODO.md", "worker-2": "WORKER_2_TODO.md"}, repr(got))
else:
    print("SKIP A6 cys 부재 또는 pane 밖 실행 — 측정 불능(통과 아님)")

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

# ── C. 생존 게이트는 수신 surface 를 잰다 ───────────────────────────────────────
UNVERIFIED = "liveness UNVERIFIED (recipient not on this daemon)"


def row(sid, role, alive=True, idle=0, exited=False, status_age=None):
    st = {"state": "working", "age_secs": status_age} if status_age is not None else {}
    return {"surface_id": sid, "surface_ref": "surface:%d" % sid, "role": role,
            "agent_alive": alive, "exited": exited, "idle_secs": idle, "status": st}


def run_tp(rows, to="worker", surface=None, no_gate=False):
    """cmd_task_prompt 를 가짜 status 로 프로세스 안에서 돌린다 → (rc, stdout, stderr, status 호출 수)."""
    calls = [0]

    def fake_status():
        calls[0] += 1
        if rows is None:
            raise AssertionError("status must not be read under --no-survival-gate")
        return {"surfaces": rows}

    args = types.SimpleNamespace(task="T", scope="S", success="C", to=to, surface=surface,
                                 no_survival_gate=no_gate)
    real = orch.cys_status
    orch.cys_status = fake_status
    out, err = io.StringIO(), io.StringIO()
    try:
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            rc = orch.cmd_task_prompt(args)
    except Exception as e:                  # noqa: BLE001 — 한 칸의 예외가 뒤 칸을 지우지 않게 그 칸의 실패로 남긴다
        rc = "EXC:%s:%s" % (type(e).__name__, str(e)[:80])
    finally:
        orch.cys_status = real
    return rc, out.getvalue(), err.getvalue(), calls[0]


# C1 — CEO 실측 형상: 실제 수신자(surface:133, worker-7)가 죽었는데 같은 역할의 다른 좌석과 `worker` 는 살아 있다
rc, so, se, _ = run_tp([row(126, "worker"), row(133, "worker-7", alive=False, status_age=9999),
                        row(140, "worker-7")], to="worker", surface="133")
check("C1 죽은 수신 surface 는 같은 역할의 산 좌석이 있어도 거부(exit 1 · 티켓 미출력)",
      rc == 1 and so == "" and "surface:133" in se, "rc=%s se=%r" % (rc, se[-200:]))

# C2 — 수신 surface 가 살아 있고 역할이 worker 가 아니다(역할 worker 는 아예 없다)
rc, so, se, _ = run_tp([row(133, "worker-7")], to="worker", surface="surface:133")
check("C2 산 수신 surface(역할 worker-7)는 역할 worker 부재여도 발부",
      rc == 0 and "[작업 위임" in so and UNVERIFIED not in so, "rc=%s se=%r" % (rc, se[-200:]))

# C3 — 두 워커: 역할만 주면 어느 좌석인지 모른다 → 거부하며 둘 다 이름 댄다
rc, so, se, _ = run_tp([row(126, "worker"), row(150, "worker")], to="worker")
check("C3 역할 worker 의 산 좌석 둘 → 거부(exit 1)·후보 surface:126·surface:150 명시",
      rc == 1 and so == "" and "surface:126" in se and "surface:150" in se, "rc=%s se=%r" % (rc, se[-240:]))

# C4 — 역할의 산 좌석이 하나면 종전대로
rc, so, se, _ = run_tp([row(126, "worker"), row(150, "worker", alive=False, exited=True)], to="worker")
check("C4 역할의 산 좌석 하나 → 발부(exit 0)", rc == 0 and "[작업 위임" in so, "rc=%s se=%r" % (rc, se[-200:]))

# C5 — 이 데몬에 없는 surface: 거부하지 않고 UNVERIFIED 를 stderr 와 티켓 머리에
rc, so, se, _ = run_tp([row(126, "worker")], to="worker", surface="999")
head = "\n".join(so.splitlines()[:3])
check("C5 교차 데몬 surface → 발부(exit 0)·stderr 와 티켓 머리에 UNVERIFIED",
      rc == 0 and UNVERIFIED in se and UNVERIFIED in head, "rc=%s head=%r se=%r" % (rc, head, se[-200:]))
check("C5b 같은 데몬의 수신자에는 UNVERIFIED 없음(byte-동일 머리)",
      UNVERIFIED not in run_tp([row(126, "worker")], to="worker")[1])

# C6 — idle 경고는 그 행을 읽는다(같은 역할의 먼저 나오는 행이 아니다)
idle_thr = int(os.environ.get("CYS_IDLE_SECONDS", "300"))
rc, so, se, _ = run_tp([row(120, "worker-7", idle=0), row(133, "worker-7", idle=idle_thr + 699)],
                       to="worker", surface="133")
check("C6 idle 경고가 수신 surface:133 의 idle 을 읽는다",
      rc == 0 and ("idle %d초" % (idle_thr + 699)) in se and "surface:133" in se, "se=%r" % se[-240:])

# C7 — --no-survival-gate 는 status 를 읽지 않는다(Control Center 일회용 경로 · 종전 그대로)
rc, so, se, calls = run_tp(None, to="worker", surface=None, no_gate=True)
check("C7 --no-survival-gate → status 미호출·발부", rc == 0 and calls == 0 and "[작업 위임" in so,
      "rc=%s calls=%s" % (rc, calls))

# C8 — --surface 값 위생
rc, so, se, _ = run_tp([row(126, "worker")], to="worker", surface="133;rm")
check("C8 --surface 위생 위반 → exit 2", rc == 2 and so == "", "rc=%s" % rc)

# C9 — CLI 가 --surface 를 받는다(생존 게이트는 --no-survival-gate 로 생략 — 파서 계약만)
r_sf = subprocess.run([PY, ORCH, "task-prompt", "--task", "T", "--scope", "S", "--surface", "surface:12",
                       "--no-survival-gate"],
                      capture_output=True, text=True, encoding="utf-8", env=env, timeout=60)
check("C9 CLI --surface 인자 수용", r_sf.returncode == 0 and "[작업 위임" in r_sf.stdout,
      "rc=%s se=%r" % (r_sf.returncode, r_sf.stderr[-200:]))

# C10 — live_roles 술어(cmd_check·self-test 의 소비)는 종전 그대로
check("C10 live_roles 종전 술어",
      orch.live_roles({"surfaces": [row(1, "cso"), row(2, "worker", alive=False),
                                    row(3, "worker-7", alive=False, status_age=10),
                                    row(4, "worker-9", exited=True)]})
      == {"cso": True, "worker-7": True})

print("\n=== %d/%d PASS ===" % (checked[0] - len(fails), checked[0]))
if fails:
    print("FAIL: %s" % fails, file=sys.stderr)
    sys.exit(1)
print("ORCHESTRA-TODO-PATH-OK")
