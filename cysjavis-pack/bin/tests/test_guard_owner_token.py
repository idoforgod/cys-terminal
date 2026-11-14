#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_guard_owner_token.py — guard.sh 가 규칙 문서 쓰기를 **주인 지시 표**로만 더 허용하는가 (0.15.0 M13).

무엇을 재는가(C10 10.5 · O-C10-4 · RULINGS AT9):
  1  표가 없으면(데몬이 거부) 규칙 문서 Write/Edit 은 배포본 그대로 차단된다(같은 deny 문장).
  2  데몬이 허용(rule-doc-allowed 종료 코드 0)하면 그 Write/Edit 만 통과하고, 조회는
     `cys --socket <s> repair rule-doc-allowed --path <파일>` 하나이며 CYS_NO_AUTOSTART=1 로 간다.
  3  소켓 없음 · 옛 데몬(93) · 거부(95) · 시간 초과 → 오늘의 동작(차단). 시간 초과는 한도를 **양쪽에서 끼워** 판정한다
     (RULINGS EW1 · FB1). 스텁(가짜 cys — 테스트를 돌리는 python 을 절대경로 shebang 으로 쓰는 작은 스크립트)은 첫
     동작으로 자기 단조 시계 t0 를 찍고, 막기 모드에서 20ms 마다 "waiting <Δ> <절대시각>" 을 덧붙여 쓰며 끝내 답하지
     않는다(time.monotonic 은 macOS·Linux 에서 시스템 전역 시계라 테스트와 스텁이 같은 시계를 읽는다).
     · 하한 = 스텁 자신의 구간(t0 → 마지막 하트비트의 Δ). 스텁은 조회가 시작된 뒤에 뜨고 죽은 뒤에는 쓰지 못하므로
       이 값은 한도보다 **길 수 없다** → "한도는 적어도 X" 만 이것으로 단언한다.
     · 상한 = 테스트가 훅을 부르기 직전에 읽은 시각 → 훅이 돌아온 시각. 조회는 호출 뒤에 시작되고 가드는 스텁을
       죽인 뒤에야 돌아오므로 이 값은 한도보다 **짧을 수 없다** → "한도는 많아야 X" 는 이것으로 단언한다. FB1 이 이름
       붙인 호출 → 스텁의 마지막 하트비트(절대시각)도 같은 문턱으로 단언한다 — 다만 그 값은 스텁이 마지막 하트비트와
       자기 죽음 사이를 기록하지 못하는 만큼(스케줄에서 밀린 꼬리) 한도보다 짧아질 수 있어 단독으로는 상한이 아니다.
     기동 지연은 이제 맞는 한도를 시끄러운 실패로 만들 수 있을 뿐, 틀린 한도를 통과시키지 못한다.
     통과 = rc 2 + "start" + 하트비트 2개 이상 + "answered" 없음 + 그 프로세스가 끝났음 + 스텁의 절대시각이 모두
     [호출, 반환] 안(이 호출의 기록 · 같은 시계) + 하한 + 상한. 가드의 거부 문장은 증거가 아니고, 훅 전체 시간은
     상한으로만 쓴다(하한으로는 결코 쓰지 않는다). 즉시 실패하는 막기는 하트비트를 남기지 못하므로 실패한다(회귀 3e).
  7  가드의 테스트 예산 손잡이가 실제로 쓰는 조회 한도(RULINGS EW2 · ER1 · EM2 · FB1 — 제품 변경 없이). 경우마다
     쓰는 쪽과 그쪽이 잡아내는 틀린 구현:
       7a 예산 5초 → D=1.5초 뒤 "answered Δ≥D" 와 허용 — 하한(0.6초 한도가 그대로면 답 전에 끊는다)
       7b 예산 60초(상한 위) → D=3초 뒤 허용 — 하한(1초 넘는 예산이 한도를 올리지 않으면 끊는다)
       7c 예산 60초 → 상한 21초(20초 상한이 안 걸려 60초면 못 맞춘다) · 하한 19.5초(상한이 20초보다 낮으면)
       7d 작은 예산 0.3초 → 상한 0.6초(작은 예산을 무시해 0.6초 한도가 남으면 결코 못 맞춘다) · 하한 0.05초
          (조회가 시간을 받지 못하면)
       7e 변수 없음 → 하한 0.35초(기본 0.6초가 줄면) · 상한 1.0초(기본이 1초 — 합산 예산 — 이상으로 늘면)
       7f 테스트 모드 끔 + 변수 5초 → 상한 1.0초(변수를 따라 5초가 되면) · 하한 0.35초(기본이 줄면)
       3c 예산 5초(시간 초과) → 하한 4.5초(0.6초 한도가 남으면) · 상한 6초(합산 예산이 안 걸려 20초 상한까지
          가는 등 한도가 6초 이상이면)
  9  자기 점검(RULINGS EW2 · FB1 음성 대조): 판정 함수에 지어낸 틀린 기록을 먹인다 — (i) 한도가 0.1초로 줄었는데 무관한
     기동 지연으로 훅 전체가 0.8초 걸린 기록, (ii) 막기가 즉시 실패했는데 무관한 지연으로 훅 전체가 5.5초 걸린
     기록, (iii) D 전에 답한 기록, (iv) 한도가 너무 긴 기록, (v)(vi) codex 의 반례 — 작은 예산을 무시해 0.6초 한도가
     남았는데 스텁 기동이 0.15초 / 0.5초 늦은 기록(084da8bd 의 스텁 단독 창 [0.05, 0.50] 은 받아들였다), (vii) 스텁이
     마지막 하트비트 뒤 스케줄되지 못해 꼬리가 기록되지 않은 기록(호출 → 마지막 하트비트만으로는 통과한다) — 모두
     판정이 **실패**해야 한다. (viii)(ix) 같은 반례를 실제 가드로: 0.6초 한도(변수 없음 = 작은 예산을 무시했을 때
     남는 값)에 STUB_START_DELAY 0.15초 / 0.5초를 주고 7d 의 판정에 넣으면 상한에서 실패해야 한다.
  4  Bash 로 규칙 문서를 쓰는 경로(tee·sed -i·리다이렉트)는 표가 있어도 막힌다.
  5  corpus G 의 대표 호출(외부 발행·외부 전송·영구 삭제)은 표가 있든 없든 막힌다(AT9).
  6  규칙 문서가 아닌 쓰기는 조회 없이 통과한다(평시 동작 무변화).
밀폐: 가짜 `cys` 스텁(PATH 선두) · 임시 디렉터리의 진짜 unix 소켓 파일 · GUARD_TEST_MODE 로 자율주행 깃발을
임시 경로에 둔다 · HOME 임시. 데몬·라이브 팩 무접촉.
시간 관용도(RULINGS EW1 · EW3 · FB1): 하한의 여유(기대 한도 − 하한)는 스텁 자신의 기동(t0 이전 · 가드의 시계로는
이미 흐른 시간)과 예산이 걸린 경우 가드 자신의 전처리(합산 예산 안에서 조회 전에 쓴 시간), 하트비트 간격(20ms)을
흡수한다. 상한의 여유(상한 − 기대 한도)는 bash·인터프리터 기동, 가드의 전처리·거부 출력·종료, 가드가 스케줄에서
밀려 시간 초과를 늦게 처리하는 만큼을 흡수한다. 상한은 그것이 잡아내는 틀린 한도 이하로 잡았으므로 그 틀린 한도는
상한을 결코 맞추지 못한다. 허용·거부 경우는 큰 예산(20초)을 쓴다 — 관용도를 키웠을 뿐 조회는 여전히 남은 예산
안에 끝나야 한다. 극단적 부하에서는 맞는 한도가 한 경우에서 크게 실패하고 다시 돌린다 — 틀린 한도가 조용히
통과하지는 않는다.
`STUB_START_DELAY=<초>` 는 스텁의 t0 이전에 느린 기동을 주입한다(부하 재현).
출력: PASS/FAIL 행 · 실패 시 exit 1 · 전부 통과 시 GUARD-OWNER-TOKEN-OK.
"""
import json
import os
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time
from collections import namedtuple

SELF = os.path.dirname(os.path.abspath(__file__))
PACK = os.path.dirname(os.path.dirname(SELF))
GUARD = os.path.join(PACK, "hooks", "guard.sh")
BASH = shutil.which("bash") or "/bin/bash"
fails = []


def check(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
    if not cond:
        fails.append(name)


# The fake `cys` (RULINGS EW1): a small python script run by the test's own interpreter through an
# absolute shebang (no command found through PATH). It takes its own monotonic t0 as its first act and
# records every claim about the limit itself; beside each delta it writes the absolute monotonic reading
# (time.monotonic is system-wide on macOS and Linux, so the test reads the same clock - RULINGS FB1):
#   start: "start pid=<pid> at=<t0>"
#   STUB_MODE=answer-after (STUB_D=<s>): waits until its own delta >= D, writes "answered <delta> <at>", exits 0
#   STUB_MODE=block: appends "waiting <delta> <at>" every 20 ms (flushed each time) and never answers
#   STUB_MODE=block-fails: the blocking operation fails at once (a negative control) - exits 97
STUB_BODY = r"""
import os, sys, time
_d = os.environ.get("STUB_START_DELAY")
if _d:
    time.sleep(float(_d))                    # injected slow start-up, BEFORE t0
t0 = time.monotonic()


def put(path, line):
    if path:
        with open(path, "a") as f:
            f.write(line + "\n")
            f.flush()


args = " ".join(sys.argv[1:])
put(os.environ.get("STUB_LOG"), "%s|NO_AUTOSTART=%s" % (args, os.environ.get("CYS_NO_AUTOSTART", "")))
if "repair rule-doc-allowed" not in args:
    sys.exit(1)
ev = os.environ.get("STUB_EVIDENCE")
put(ev, "start pid=%d at=%.6f" % (os.getpid(), t0))
mode = os.environ.get("STUB_MODE", "")
if mode == "answer-after":
    d = float(os.environ["STUB_D"])
    while time.monotonic() - t0 < d:
        time.sleep(min(0.01, max(0.0, d - (time.monotonic() - t0))))
    now = time.monotonic()
    put(ev, "answered %.4f %.6f" % (now - t0, now))
    sys.exit(0)
if mode == "block":
    while True:
        now = time.monotonic()
        put(ev, "waiting %.4f %.6f" % (now - t0, now))
        time.sleep(0.02)
if mode == "block-fails":
    sys.exit(97)
sys.exit(int(os.environ.get("STUB_RULEDOC_RC", "95")))
"""

# RULINGS EH1 / ER3: the allow / deny cases get a large budget (increased tolerance - the query must
# still finish within the remaining budget); the guard's test knob raises its per-query cap with it,
# up to the guard's ceiling (20 s)
BIG_BUDGET_MS = "20000"
TIMEOUT_BUDGET_MS = "5000"

# RULINGS FB1 - every time-out case brackets the limit from both sides (see the header):
#   (label, budget ms or None, test mode, LOWER bound, what it catches, UPPER bound, what it catches)
# LOWER = the stub's own interval (t0 -> last heartbeat) >= lo; UPPER = invocation -> the hook's return < hi
# (and invocation -> the stub's last heartbeat < hi). Each upper bound lies at or below the wrong limit it
# catches, so that wrong limit can never meet it.
C3C = ("3c 시간 초과(예산 5초) → 차단", TIMEOUT_BUDGET_MS, True,
       4.5, "0.6초 한도가 남으면 못 맞춘다", 6.0, "합산 예산이 안 걸려 20초 상한까지 가는 등 6초 이상이면 못 맞춘다")
C7C = ("7c 예산 60초 → 상한 20초에서 끝났다", "60000", True,
       19.5, "상한이 20초보다 낮으면 못 맞춘다", 21.0, "20초 상한이 안 걸려 60초면 못 맞춘다")
C7D = ("7d 작은 예산 0.3초 → 한도를 낮춘다", "300", True,
       0.05, "조회가 시간을 받지 못하면 못 맞춘다", 0.6, "작은 예산을 무시해 0.6초가 남으면 결코 못 맞춘다")
C7E = ("7e 변수 없음 → 기본 한도 0.6초", None, True,
       0.35, "기본이 줄면 못 맞춘다", 1.0, "기본이 합산 예산 1초 이상으로 늘면 못 맞춘다")
C7F = ("7f 테스트 모드 끔 + 변수 5초 → 변수 무시", "5000", False,
       0.35, "기본이 줄면 못 맞춘다", 1.0, "변수를 따라 5초가 되면 못 맞춘다")

Ev = namedtuple("Ev", "started pid beats answered at_start beats_at answered_at")


def parse_evidence(lines):
    """The stub's own record: started, pid, heartbeat deltas, answered delta or None, and the absolute
    monotonic reading of each (start, heartbeats, answer)."""
    started, pid, beats, answered, at_start, beats_at, answered_at = False, None, [], None, None, [], None
    for ln in lines:
        parts = ln.split()
        try:
            if parts[0] == "start":
                started = True
                pid = int(parts[1].split("=", 1)[1])
                at_start = float(parts[2].split("=", 1)[1])
            elif parts[0] == "waiting":
                beats.append(float(parts[1]))
                beats_at.append(float(parts[2]))
            elif parts[0] == "answered":
                answered = float(parts[1])
                answered_at = float(parts[2])
        except (IndexError, ValueError):
            pass
    return Ev(started, pid, beats, answered, at_start, beats_at, answered_at)


def in_window(ev, t_call, t_ret):
    """Every absolute reading of the stub lies inside [the test's invocation, the hook's return]: the
    record belongs to this invocation and both sides read the same clock."""
    pts = [ev.at_start] + ev.beats_at + ([ev.answered_at] if ev.answered_at is not None else [])
    return ev.at_start is not None and all(t_call <= x <= t_ret for x in pts)


def allow_ok(rc, ev, d, t_call, t_ret):
    """ALLOW, a LOWER bound (the limit is at least D): the guard allowed AND the stub recorded "answered"
    with its own delta >= D, inside this invocation."""
    return rc == 0 and ev.answered is not None and ev.answered >= d and in_window(ev, t_call, t_ret)


def timed_out(rc, ev, pid_gone, t_call, t_ret):
    """The shape of a time-out: refused (rc 2 - the guard's sentence is not evidence), started, at least
    two heartbeats, NO answer, the process gone, the record inside this invocation."""
    return (rc == 2 and ev.started and ev.pid is not None and len(ev.beats) >= 2 and ev.answered is None
            and pid_gone and in_window(ev, t_call, t_ret))


def limit_at_least(ev, lo):
    """LOWER bound: the stub's own interval (t0 -> last heartbeat). The stub starts after the query began
    and cannot write after its kill, so this is never LONGER than the limit."""
    return bool(ev.beats) and ev.beats[-1] >= lo


def limit_at_most(ev, t_call, t_ret, hi):
    """UPPER bound: invocation -> the hook's return. The query begins after the invocation and the guard
    returns only after it has killed the stub, so this is never SHORTER than the limit. Invocation -> the
    stub's last heartbeat (RULINGS FB1, same clock) must be below hi too; on its own it can fall short of the
    limit by the stub's unrecorded tail (between its last heartbeat and its kill)."""
    return bool(ev.beats_at) and ev.beats_at[-1] - t_call < hi and t_ret - t_call < hi


def timeout_ok(rc, ev, pid_gone, t_call, t_ret, lo, hi):
    """TIME-OUT with the limit bracketed: the shape, the lower bound and the upper bound."""
    return (timed_out(rc, ev, pid_gone, t_call, t_ret) and limit_at_least(ev, lo)
            and limit_at_most(ev, t_call, t_ret, hi))


def stub_only_window_084da8bd(ev):
    """The 7d rule of 084da8bd (the stub's last delta within [0.05, 0.50]) - kept only to show that the
    controls 9v-9vii are codex's case: it accepted them."""
    return bool(ev.beats) and 0.05 <= ev.beats[-1] <= 0.50


def main():
    root = tempfile.mkdtemp(prefix="gtok-", dir="/tmp")
    srv = None
    try:
        stubdir = os.path.join(root, "bin")
        os.makedirs(stubdir)
        stub = os.path.join(stubdir, "cys")
        with open(stub, "w") as f:
            f.write("#!%s -IS\n" % sys.executable + STUB_BODY)
        os.chmod(stub, 0o755)
        sock = os.path.join(root, "s.sock")
        srv = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        srv.bind(sock)
        log = os.path.join(root, "stub.log")
        home = os.path.join(root, "home")
        os.makedirs(home)
        flags = os.path.join(root, "flags")
        os.makedirs(flags)
        pack = os.path.join(root, "pack")
        soul = os.path.join(pack, "soul.md")
        directive = os.path.join(pack, "directives", "WORKER_DIRECTIVE.md")

        def run(tool_input, tool="Write", rc=None, sock_path=sock, active=False,
                budget_ms=BIG_BUDGET_MS, test_mode=True, extra=None):
            env = {k: v for k, v in os.environ.items()
                   if not k.startswith(("CYS_", "JAVIS_", "AITERM_", "AUTOPILOT_", "GUARD_"))}
            env.update({
                "PATH": stubdir + os.pathsep + env.get("PATH", "/usr/bin:/bin"),
                "HOME": home,
                "CYS_SOCKET": sock_path,
                "AUTOPILOT_PAUSED_FILE": os.path.join(flags, "AUTOPILOT_PAUSED"),
                "AUTOPILOT_ACTIVE_FILE": os.path.join(flags, "AUTOPILOT_ACTIVE"),
                "STUB_LOG": log,
            })
            if test_mode:
                env["GUARD_TEST_MODE"] = "1"
            if budget_ms is not None:
                env["GUARD_TEST_GATE_BUDGET_MS"] = budget_ms
            if rc is not None:
                env["STUB_RULEDOC_RC"] = str(rc)
            env.update(extra or {})
            act = os.path.join(flags, "AUTOPILOT_ACTIVE")
            if active:
                open(act, "w").close()
            elif os.path.exists(act):
                os.remove(act)
            if os.path.exists(log):
                os.remove(log)
            payload = json.dumps({"tool_name": tool, "tool_input": tool_input})
            t_call = time.monotonic()          # read immediately before the hook starts (RULINGS FB1)
            proc = subprocess.Popen([BASH, GUARD], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                    stderr=subprocess.PIPE, text=True, env=env, start_new_session=True)
            try:
                out, err = proc.communicate(payload, timeout=60)
            except subprocess.TimeoutExpired:
                # a limit far too long: stop this hook's own session (its exact id) and fail loudly below
                os.killpg(proc.pid, signal.SIGKILL)
                out, err = proc.communicate()
            t_ret = time.monotonic()
            p = subprocess.CompletedProcess(proc.args, proc.returncode, out, err)
            calls = open(log).read().splitlines() if os.path.exists(log) else []
            return p, calls, (t_call, t_ret)

        # 1 — no token: the released deny, same sentence
        p, calls, _ = run({"file_path": soul, "content": "x"}, rc=95)
        check("1a 표 없음(데몬 거부) → 규칙 문서 Write 차단", p.returncode == 2, repr(p.stderr[-300:]))
        check("1b 배포본과 같은 deny 문장", "헌법파일(soul.md) Write 차단" in p.stderr, repr(p.stderr[-300:]))
        check("1c 조회는 rule-doc-allowed 한 번", len(calls) == 1 and "repair rule-doc-allowed --path %s" % soul in calls[0],
              repr(calls))
        # 2 — the daemon allows: only that write passes; the query carries the socket and no-autostart
        p, calls, _ = run({"file_path": soul, "content": "x"}, rc=0)
        check("2a 데몬 허용 → 통과(exit 0)", p.returncode == 0, repr(p.stderr[-300:]))
        # (the parser's own stderr is not forwarded by the device — guard.sh sends it to a temp file)
        check("2b 허용이면 deny JSON 없음(stdout 비어 있음)", p.stdout.strip() == "", repr(p.stdout[-200:]))
        check("2c --socket 명시 + CYS_NO_AUTOSTART=1",
              len(calls) == 1 and calls[0].startswith("--socket %s repair rule-doc-allowed" % sock)
              and calls[0].endswith("NO_AUTOSTART=1"), repr(calls))
        p, _, _ = run({"file_path": directive, "old_string": "a", "new_string": "b"}, tool="Edit", rc=0)
        check("2d Edit 도 같은 규칙(허용)", p.returncode == 0, repr(p.stderr[-200:]))
        p, _, _ = run({"file_path": directive, "old_string": "a", "new_string": "b"}, tool="Edit", rc=95)
        check("2e Edit 거부면 차단", p.returncode == 2)
        p, _, _ = run({"file_path": soul, "content": "x"}, rc=0, active=True)
        check("2f 자율주행(STRICT) 중에도 표는 데몬의 판정 그대로(허용)", p.returncode == 0, repr(p.stderr[-200:]))
        p, _, _ = run({"file_path": soul, "content": "x"}, rc=95, active=True)
        check("2g 자율주행 중 거부면 차단", p.returncode == 2)
        # 3 — every failure keeps today's behaviour
        p, calls, _ = run({"file_path": soul, "content": "x"}, rc=0, sock_path=os.path.join(root, "none.sock"))
        check("3a 소켓 없음 → 조회 없이 차단", p.returncode == 2 and not calls, repr((p.returncode, calls)))
        p, _, _ = run({"file_path": soul, "content": "x"}, rc=93)
        check("3b 옛 데몬(93) → 차단", p.returncode == 2)
        # evidence helpers (EW1): the stub's own record
        seq = [0]

        def ev_path():
            seq[0] += 1
            return os.path.join(root, "ev%d" % seq[0])

        def gone(pid):
            if pid is None:
                return False
            try:
                os.kill(pid, 0)
            except ProcessLookupError:
                return True
            except PermissionError:
                return False
            return False

        def drive(mode, budget_ms, d=None, test_mode=True, extra_env=None):
            base = ev_path()
            extra = {"STUB_EVIDENCE": base, "STUB_MODE": mode}
            if d is not None:
                extra["STUB_D"] = "%g" % d
            extra.update(extra_env or {})
            p, _, tw = run({"file_path": soul, "content": "x"}, rc=0, budget_ms=budget_ms,
                           test_mode=test_mode, extra=extra)
            lines = open(base).read().splitlines() if os.path.exists(base) else []
            ev = parse_evidence(lines)
            return p, ev, gone(ev.pid), tw

        def since(t_call, t):
            return ("%.3f" % (t - t_call)) if t is not None else "없음"

        def show(p, ev, tw):
            return "rc=%s start=%s pid=%s heartbeats=%d last Δ=%s answered=%s 호출→마지막 하트비트=%s 호출→반환=%.3f" % (
                p.returncode, ev.started, ev.pid, len(ev.beats), ("%.3f" % ev.beats[-1]) if ev.beats else None,
                ev.answered, since(tw[0], ev.beats_at[-1] if ev.beats_at else None), tw[1] - tw[0])

        def last(ev):
            return ("%.3f" % ev.beats[-1]) if ev.beats else "없음"

        def limit_case(case, extra_env=None):
            label, budget_ms, test_mode, lo, lo_catches, hi, hi_catches = case
            p, ev, pid_gone, tw = drive("block", budget_ms, test_mode=test_mode, extra_env=extra_env)
            ok = timeout_ok(p.returncode, ev, pid_gone, tw[0], tw[1], lo, hi)
            check("%s — 하한: 스텁 자신의 Δ %s초 ≥ %g초(%s) · 상한: 호출→마지막 하트비트 %s초 · 호출→반환 %.3f초"
                  " < %g초(%s) · 답 없음 · 그 프로세스는 끝났다"
                  % (label, last(ev), lo, lo_catches, since(tw[0], ev.beats_at[-1] if ev.beats_at else None),
                     tw[1] - tw[0], hi, hi_catches), ok, "" if ok else show(p, ev, tw))

        # 3c — the daemon's answer never comes: the limit bracketed from both sides (RULINGS FB1)
        limit_case(C3C)
        # 3e — a blocking operation that fails at once leaves no heartbeat: the time-out judgment FAILS
        p, ev, pid_gone, tw = drive("block-fails", TIMEOUT_BUDGET_MS)
        bad = timeout_ok(p.returncode, ev, pid_gone, tw[0], tw[1], C3C[3], C3C[5])
        check("3e 막기가 즉시 실패한 스텁에서는 시간 초과 판정이 통과하지 않는다(스텁의 하트비트 %d개)" % len(ev.beats), not bad,
              "" if not bad else show(p, ev, tw))

        # 7 — the knob's effective per-query limit (RULINGS EW2 · FB1)
        def allow_case(label, budget_ms, d, catches):
            p, ev, _, tw = drive("answer-after", budget_ms, d=d)
            ok = allow_ok(p.returncode, ev, d, tw[0], tw[1])
            check("%s — 하한: 스텁이 잰 답 시점 Δ=%s초 ≥ D=%g초, 가드 허용(%s)" % (
                label, ("%.3f" % ev.answered) if ev.answered is not None else "없음", d, catches), ok,
                "" if ok else show(p, ev, tw))

        allow_case("7a 예산 5초 → 한도가 예산을 따랐다", "5000", 1.5, "0.6초 한도가 그대로면 답 전에 끊는다")
        allow_case("7b 예산 60초(상한 위) → 1초 넘는 예산은 한도를 올린다", "60000", 3.0,
                   "1초 넘는 예산이 한도를 올리지 않으면 끊는다")
        limit_case(C7C)
        limit_case(C7D)
        limit_case(C7E)
        limit_case(C7F)

        # 9 — self-checks of the predicates (RULINGS EW2 · FB1 negative controls): fabricated wrong records
        def fake(stub_t0, deltas, answered=None):
            lines = ["start pid=1 at=%.6f" % stub_t0]
            lines += ["waiting %.4f %.6f" % (x, stub_t0 + x) for x in deltas]
            if answered is not None:
                lines.append("answered %.4f %.6f" % (answered, stub_t0 + answered))
            return parse_evidence(lines)

        def every(step, upto):
            return [round(step * k, 4) for k in range(int(upto / step + 1e-9) + 1)]

        lo7e, hi7e = C7E[3], C7E[5]
        lo7d, hi7d = C7D[3], C7D[5]
        lowered = fake(0.05, every(0.02, 0.10)[1:])            # invocation 0, hook returned at 0.8
        check("9i 한도가 0.1초로 줄어든 기록(훅 전체 0.8초여도) → 판정 실패",
              not timeout_ok(2, lowered, True, 0.0, 0.8, lo7e, hi7e))
        no_beat = fake(0.05, [])
        check("9ii 막기가 즉시 실패한 기록(훅 전체 5.5초여도) → 판정 실패",
              not timeout_ok(2, no_beat, True, 0.0, 5.5, C3C[3], C3C[5]))
        early = fake(0.05, [], answered=0.01)
        check("9iii D 전에 답한 기록 → 허용 판정 실패", not allow_ok(0, early, 1.5, 0.0, 0.1))
        too_long = fake(0.05, every(0.1, 5.9)[1:])
        check("9iv 한도가 너무 긴 기록(5.9초 · 기대 0.6초) → 판정 실패",
              not timeout_ok(2, too_long, True, 0.0, 6.0, lo7e, hi7e))
        # codex's case: a small budget ignored (limit 0.6 s; query at 0.05, killed at 0.65, hook returned at
        # 0.67) behind the stub's slow start-up
        for k, startup, deltas in (("v", 0.15, every(0.02, 0.44)), ("vi", 0.5, every(0.02, 0.08))):
            ev = fake(0.05 + startup, deltas)
            check("9%s 작은 예산 무시(한도 0.6초) + 스텁 기동 %g초 지연(마지막 Δ %.2f초) → 084da8bd 의 스텁 단독 창은"
                  " 받아들였고, 지금 판정은 상한에서 실패" % (k, startup, deltas[-1]),
                  stub_only_window_084da8bd(ev) and limit_at_least(ev, lo7d)
                  and not limit_at_most(ev, 0.0, 0.67, hi7d) and not timeout_ok(2, ev, True, 0.0, 0.67, lo7d, hi7d))
        # the stub's tail goes unrecorded: last heartbeat at 0.455, descheduled until its kill at 0.605
        tail = fake(0.155, every(0.02, 0.30))
        check("9vii 스텁의 꼬리가 기록되지 않은 기록(호출→마지막 하트비트 0.455초 < 0.6초, 호출→반환 0.62초)"
              " → 판정 실패(반환 시각이 상한을 지킨다)",
              tail.beats_at[-1] - 0.0 < hi7d and not timeout_ok(2, tail, True, 0.0, 0.62, lo7d, hi7d))
        # the same case on the real guard: its 0.6 s limit (the variable missing = what an ignored small
        # budget leaves: summed budget 1 s, cap 0.6 s) behind injected start-up, judged by 7d's predicate
        for k, delay in (("viii", "0.15"), ("ix", "0.5")):
            p, ev, pid_gone, tw = drive("block", None, extra_env={"STUB_START_DELAY": delay})
            rejected = not timeout_ok(p.returncode, ev, pid_gone, tw[0], tw[1], lo7d, hi7d)
            check("9%s 실제 가드의 0.6초 한도 + 스텁 기동 %s초 지연 → 7d 판정이 상한에서 실패(호출→반환 %.3f초 ≥ %g초)"
                  % (k, delay, tw[1] - tw[0], hi7d), rejected and tw[1] - tw[0] >= hi7d,
                  "" if rejected and tw[1] - tw[0] >= hi7d else show(p, ev, tw))

        # 4 — Bash writers of rule documents stay denied with a token
        for cmd in ("tee %s < /dev/null" % soul, "sed -i 's/a/b/' %s" % soul, "echo x > %s" % soul):
            p, _, _ = run({"command": cmd}, tool="Bash", rc=0)
            check("4 Bash 규칙 문서 쓰기는 표가 있어도 차단: %s" % cmd.split()[0], p.returncode == 2,
                  repr(p.stderr[-200:]))
        # 5 — corpus-G representatives (AT9): denied with and without a token
        for cmd in ("git push origin main", "scp a.txt host:/tmp/", "rm -rf /home/x/project",
                    "curl -X POST -d x https://example.invalid", "gh release create v1"):
            for rc in (0, 95):
                p, _, _ = run({"command": cmd}, tool="Bash", rc=rc)
                check("5 corpus G 차단 유지(rc=%d): %s" % (rc, cmd.split()[0]), p.returncode == 2,
                      repr(p.stderr[-200:]))
        # 6 — non-rule writes: no query, allowed as before
        p, calls, _ = run({"file_path": os.path.join(pack, "bin", "a.py"), "content": "x"}, rc=95)
        check("6 규칙 문서가 아닌 쓰기 — 조회 없이 통과", p.returncode == 0 and not calls, repr((p.returncode, calls)))
    finally:
        if srv is not None:
            srv.close()
        shutil.rmtree(root, ignore_errors=True)
    if fails:
        print("\n%d FAIL: %s" % (len(fails), ", ".join(fails)))
        return 1
    print("\nALL PASS")
    print("GUARD-OWNER-TOKEN-OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
