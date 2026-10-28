#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_orchestra_todo_path.py — 위임 티켓의 todo 경로·생존 게이트가 **수신 좌석**에 묶이는가 + 장부 id 배선 회귀
(0.14.30 A4 #10 · PREP #10 · 0.14.56 #6).

## 무엇을 막는가 — 두 형식(CEO ruling 2026-10-28 22:40 `ebe8081218a26eb5` §1)
(N) **헌장이 부르는 형식(`--surface` 없음)은 0.14.55(9da2418d) 그대로다.** 열 개 레인의 헌장이 이 형식으로
    부른다. 이 경로가 rc 1 로 답하면 헌장을 그대로 따른 좌석이 막히고 위임 자체가 멈춘다 — 그래서 티켓의 todo 세
    줄(발부자 절대경로 · 선언 지시 · 선언 한 줄)과 생존 게이트(그 역할의 산 좌석 아무나)를 **바이트 그대로** 둔다.
    남는 결함(번호 붙은 워커·일회용 좌석이 `--to worker` 티켓을 받으면 `WORKER_TODO.md` 를 가리킨다)은 기록된
    잔여이고, 헌장 줄이 바뀌기 전까지 그 좌석들에는 `--surface` 를 명시해 보낸다(CEO 임시 완화).
(A) **`--surface N` 형식은 수신 좌석이 자기 todo 를 산출한다**(0.14.56 #6-a): 티켓은 파일을 이름 붙이지 않고
    수신 좌석이 `cys todo-path`·`--emit-decl` 로 자기 경로·선언을 받는다. S14 불변식(경로와 선언 scope 는
    **같은 바인딩**)은 수신 좌석의 한 생산자가 둘 다 만드는 더 강한 형태로 유지된다.
(W) **`--surface` 형식의 우회 줄**(CEO 조건 ⒝): codex 셸은 CYS_* 를 지워 `cys todo-path` 가
    `CYS_SURFACE_ID 없음` 으로 거부된다. 티켓은 그 거부를 **이름으로** 말하고 읽기 전용 우회를 싣는다 —
    역할은 발부 시점에 이 데몬의 **그 surface 행**에서 읽은 값이다(추측 0). 역할을 읽지 못했으면(이 데몬에
    없는 surface · 생존 게이트 생략 · 형식 밖 역할) 우회를 싣지 않고 "master 에 보고" 를 이름 붙여 말한다.
(B) **플레이스홀더 방치**: E1·P3 블록의 `<id>`·`<task-id>` 를 워커가 치환하지 않으면 done 게이트가
    대상 불일치로 거부한다. 발부자가 아는 값은 발부 시점에 박고(--task-id), 모르면 stderr 로 고지한다.
(C) **수신 surface 를 잰 생존 게이트**(0.14.56 #6-b · `--surface` 형식에서만): 그 행을 `live_roles` 와 같은
    술어로 잰다. 이 데몬에 없는 surface 는 거부하지 않고 'liveness UNVERIFIED' 를 stderr 와 티켓 머리에 적는다.

## 이 파일이 못박는 것
  N1 골든 픽스처 두 개(fixtures/task-prompt-noflag-{worker,worker-2}.txt · 9da2418d 코드가 만든 바이트)의 sha256 이
     핀과 같다(누가 픽스처를 새 코드로 재생성하면 여기서 붉어진다).
  N2 `--surface` 없는 티켓이 그 골든과 **바이트 동일**(고정 입력: 팩 /nonexistent-golden/pack-dept-dept-G ·
     날짜 2026-10-28 · FALLBACK_RULES). 의도한 티켓 변경은 두 골든을 새 코드로 재생성하고 커밋에 그렇게 적는다.
  N3 양성 대조(같은 실행): 골든의 1바이트를 뒤집으면 비교기가 그 오프셋을 정확히 짚는다(처음·가운데·끝).
  N4 Control Center 일회용 좌석 경로(`--to worker --no-survival-gate`, `--surface` 없음)도 0.14.55 형식이다
     (발부자 절대경로 + 선언 한 줄 · `cys todo-path` 없음) — 기록된 잔여 결함이 여기 산다.
  A1 `--surface` 형식의 todo 두 줄이 `cys todo-path` 를 지시하고 `_TODO.md`·`${CYS_PACK_DIR`·발부자 팩 경로를
     싣지 않는다(worker · worker-2 · reviewer-gemini · worker-fresh-<epoch>).
  A2 그 두 줄은 수신자 역할과 무관하게 바이트 동일 — 발부자가 수신자별 파일을 고르지 않는다.
  A3 `--surface` 형식에 발부자 선언 없음 · 선언은 `cys todo-path --emit-decl` · 위치·은퇴 계약 유지.
  A4 역할을 읽지 못한 `--surface` 티켓에는 발부자 팩 이름(부서 팩 · G4 밖 이름)이 새지 않는다.
  A6 (pane 안일 때만) 생산자 `cys todo-path --role` 이 두 워커에 서로 다른 파일을 낸다. pane 밖은 정직하게 skip.
  W1 이 데몬의 산 surface 로 보낸 티켓의 우회 줄: `CYS_SURFACE_ID 없음` 을 이름으로 말하고
     `CYS_PACK_DIR=<발부자 팩> cys todo-path --role <그 행의 역할>`·`--emit-decl` 을 싣는다.
  W2 이 데몬에 없는 surface → 우회 명령 없음 · "역할을 읽지 못했다 … master 에 보고" 를 이름으로.
  W3 `--surface` + `--no-survival-gate` → 역할을 읽지 않았으므로 W2 와 같다(추측 0).
  W4 행의 역할이 형식 밖(kebab 아님) → 명령에 보간하지 않고 W2 와 같다.
  B1 --task-id 지정 시 E1 set-status·증거 경로·probe --task 가 실 id 로 치환되고 플레이스홀더 0.
  B2 --task-id 미지정 시 티켓이 **byte-동일**(하위호환)이고 stderr 에 고지 1줄.
  B3 부적격 id(공백·경로 문자)는 exit 2 로 거부(문자열이 명령문에 그대로 보간되므로 위생 필수).
  C1 --surface 의 행이 죽었으면 같은 역할의 다른 산 좌석이 있어도 거부(exit 1) — 역할이 아니라 그 행을 잰다.
  C2 --surface 의 행이 살아 있으면 역할이 `worker` 가 아니어도(역할 `worker` 부재) 발부한다.
  C3 ★`--surface` 없이 역할 `worker` 의 산 좌석이 둘이어도 **발부한다**(exit 0 · 0.14.55 게이트 · CEO ⒞ 의
     실패 방향: 헌장 경로는 rc 1 로 답하지 않는다).
  C4 역할의 산 좌석이 하나면 발부(exit 0).
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
import hashlib
import io
import os
import shlex
import shutil
import subprocess
import sys
import time
import types
from unittest import mock

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
    try:
        return orch.build_task_ticket("T", "S", "C", to_role, rules=orch.FALLBACK_RULES, **kw)
    except Exception as e:                  # noqa: BLE001 — 한 칸의 예외가 뒤 칸을 지우지 않게 그 칸의 실패로 남긴다
        return "<build_task_ticket raised %s: %s>" % (type(e).__name__, str(e)[:80])


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


# ── N. `--surface` 없는 형식 = 0.14.55(9da2418d) 바이트 그대로 (CEO ruling ebe8081218a26eb5 §1 ⒜) ─────
GOLDEN_PACK = "/nonexistent-golden/pack-dept-dept-G"
GOLDEN = {"worker": ("task-prompt-noflag-worker.txt",
                     "fa0f875eb2ae6ec3ca32018f2843fbe13d0ac8afd6f8cd3217b31be2752ecd8a"),
          "worker-2": ("task-prompt-noflag-worker-2.txt",
                       "dd25356bd074a8c86e89ff6ec96e4d26493049549f612066a4c875278791491a")}
_real_strftime = time.strftime


def _fixed_date(fmt, *a):
    return "2026-10-28" if fmt == "%Y-%m-%d" else _real_strftime(fmt, *a)


def golden_mismatch(actual, expected):
    """첫 불일치 바이트 오프셋 — 같으면 None. 길이만 다르면 짧은 쪽 길이."""
    if actual == expected:
        return None
    for i in range(min(len(actual), len(expected))):
        if actual[i] != expected[i]:
            return i
    return min(len(actual), len(expected))


if os.sep != "/":
    print("SKIP N1-N3 골든 — 픽스처는 POSIX 경로 결합의 바이트다(이 플랫폼 os.sep=%r) · 측정 불능(통과 아님)" % os.sep)
else:
    with pack_env(GOLDEN_PACK), mock.patch("time.strftime", side_effect=_fixed_date):
        for role, (fname, pin) in GOLDEN.items():
            with open(os.path.join(SELF, "fixtures", fname), "rb") as f:
                exp = f.read()
            got_sha = hashlib.sha256(exp).hexdigest()
            check("N1 골든 픽스처 고정(%s · sha256 핀)" % role, got_sha == pin, got_sha)
            cur = orch.build_task_ticket("T-PIN", "S-PIN", "C-PIN", role,
                                         rules=orch.FALLBACK_RULES).encode("utf-8")
            off = golden_mismatch(cur, exp)
            check("N2 --surface 없는 티켓이 9da2418d 골든과 바이트 동일(%s)" % role, off is None,
                  "첫 불일치 바이트 %s: 현재=%r 골든=%r" % (off, cur[off:off + 60] if off is not None else b"",
                                                   exp[off:off + 60] if off is not None else b""))
            for pos in (0, len(exp) // 2, len(exp) - 1):
                bad = bytearray(exp)
                bad[pos] ^= 0x01
                check("N3 양성 대조 — 1바이트 뒤집기(@%d)를 비교기가 그 오프셋으로 거부(%s)" % (pos, role),
                      golden_mismatch(bytes(bad), exp) == pos)

env = dict(os.environ)
env["CYS_PACK_DIR"] = env.get("CYS_PACK_DIR") or "/tmp"
r_cc = subprocess.run([PY, ORCH, "task-prompt", "--task", "T", "--scope", "S", "--success", "C",
                       "--to", "worker", "--no-survival-gate", "--output-format", "F"],
                      capture_output=True, text=True, encoding="utf-8", env=env, timeout=60)
cc_path = os.path.join(os.path.abspath(env["CYS_PACK_DIR"]), "round", "WORKER_TODO.md")
check("N4 Control Center 일회용 좌석 경로(`--surface` 없음) = 0.14.55 형식(발부자 절대경로 + 선언 · cys todo-path 없음)",
      r_cc.returncode == 0 and ('"%s"' % cc_path) in r_cc.stdout
      and "<!-- javis:todo v1 owner=worker scope=" in r_cc.stdout and "`cys todo-path`" not in r_cc.stdout,
      "rc=%s path=%s" % (r_cc.returncode, cc_path))

# ── A. `--surface` 형식: todo 경로·선언은 수신 좌석이 산출한다 ─────────────────────────────
FRESH = "worker-fresh-1793190000"           # schedule.rs 가 만드는 일회용 좌석 역할 꼴
issuer_pack = os.path.abspath(orch.pack_dir())


def todo_main(t):
    """`--surface` 형식의 todo 두 줄(`todo 영속:`·`todo 선언:`) — 우회 줄은 뺀다."""
    return "\n".join(ln for ln in (todo_block(t) or "").splitlines()
                     if ln.startswith("todo 영속:") or ln.startswith("todo 선언:"))


mains = {}
for role in ("worker", "worker-2", "reviewer-gemini", FRESH):
    t = ticket(role, recipient={"surface": 7, "role": role})
    b = todo_block(t)
    mains[role] = todo_main(t)
    check("A1 todo 블록 존재(%s)" % role, b is not None, repr(t[-600:]))
    m = mains[role]
    check("A1 수신 좌석 산출 지시 `cys todo-path`(%s)" % role, "`cys todo-path`" in m, m[:200])
    check("A1 발부자가 고른 todo 파일 없음(%s)" % role, "_TODO.md" not in (b or ""), (b or "")[:200])
    check("A1 todo 두 줄에 발부자 팩 경로 없음(%s)" % role, issuer_pack not in m, issuer_pack)
    check("A1 늦은 셸 전개 없음(%s)" % role, "${CYS_PACK_DIR" not in (b or ""))

check("A2 todo 두 줄이 수신자 역할과 무관하게 바이트 동일(발부자가 수신자별 파일을 고르지 않는다)",
      mains["worker"] != "" and mains["worker"] == mains["worker-2"] == mains[FRESH] == mains["reviewer-gemini"],
      "worker=%r worker-2=%r" % (mains["worker"][:120], mains["worker-2"][:120]))

t0 = ticket("worker", recipient={"surface": 7, "role": "worker"})
check("A3 발부자 선언 없음(`<!-- javis:todo` 0)", "<!-- javis:todo" not in t0)
check("A3 발부자 scope 없음(`scope=` 0)", "scope=" not in t0)
check("A3 선언은 수신 좌석 생성기로(`cys todo-path --emit-decl`)", "cys todo-path --emit-decl" in t0)
check("A3 위치 계약 유지(첫 체크박스보다 위)", "첫 체크박스보다 위" in t0)
check("A3 은퇴 계약 유지(status=retired)", "status=retired" in t0)

for pk in ("/tmp/pack-dept-dept-9", "/tmp/자비스"):
    with pack_env(pk):
        tk = ticket("worker", recipient={"surface": 7, "role": None, "why": "test"})
    leaf = os.path.basename(pk)
    check("A4 역할을 읽지 못한 --surface 티켓에 발부자 팩 이름이 새지 않음(%s)" % leaf,
          leaf not in tk and pk not in tk)

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

# C3 — ★`--surface` 없이 역할 worker 의 산 좌석 둘: 0.14.55 게이트 그대로 발부(CEO ruling ebe80812 §1 · 실패 방향:
#   헌장이 부르는 경로는 rc 1 로 답하지 않는다 — 막히면 헌장을 그대로 따른 좌석이 값을 치른다)
rc, so, se, _ = run_tp([row(126, "worker"), row(150, "worker")], to="worker")
check("C3 --surface 없음 · 역할 worker 의 산 좌석 둘 → 발부(exit 0 · 0.14.55 형식 · 거부 문구 없음)",
      rc == 0 and "[작업 위임" in so and "WORKER_TODO.md" in so and "산 좌석이" not in se,
      "rc=%s se=%r" % (rc, se[-240:]))

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

# ── W. `--surface` 형식의 우회 줄 — 이름 붙은 거부 + 읽기 전용 우회(CEO ⒝ · 추측 0) ─────────────────────────
REFUSAL = "CYS_SURFACE_ID 없음"
rc, so, se, _ = run_tp([row(126, "worker"), row(133, "worker-7")], to="worker", surface="133")
want_path = "CYS_PACK_DIR=%s cys todo-path --role worker-7" % shlex.quote(issuer_pack)
check("W1 산 surface:133(worker-7) → 거부 이름(`%s`)과 우회 `%s`·`--emit-decl` 을 싣는다" % (REFUSAL, want_path),
      rc == 0 and REFUSAL in so and want_path in so and (want_path + " --emit-decl") in so
      and "--role worker " not in so and "--role worker`" not in so,
      "rc=%s tail=%r" % (rc, so[-900:]))

UNKNOWN = "역할을 읽지 못했다"
for name, (rows, surface, no_gate) in {
        "W2 이 데몬에 없는 surface": ([row(126, "worker")], "999", False),
        "W3 --surface + --no-survival-gate": (None, "133", True),
        "W4 형식 밖 역할(kebab 아님)": ([row(133, "Worker 7;rm -rf")], "133", False)}.items():
    rc, so, se, _ = run_tp(rows, to="worker", surface=surface, no_gate=no_gate)
    check("%s → 우회 명령 없음 · '%s … master 에 보고' 를 이름으로" % (name, UNKNOWN),
          rc == 0 and UNKNOWN in so and "master 에 보고" in so and "todo-path --role" not in so
          and "Worker 7;rm" not in so,
          "rc=%s tail=%r" % (rc, so[-700:]))

print("\n=== %d/%d PASS ===" % (checked[0] - len(fails), checked[0]))
if fails:
    print("FAIL: %s" % fails, file=sys.stderr)
    sys.exit(1)
print("ORCHESTRA-TODO-PATH-OK")
