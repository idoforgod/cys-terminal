#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_guard_owner_token.py — guard.sh 가 규칙 문서 쓰기를 **주인 지시 표**로만 더 허용하는가 (0.15.0 M13).

무엇을 재는가(C10 10.5 · O-C10-4 · RULINGS AT9 · FV1):
  1  표가 없으면(데몬이 거부) 규칙 문서 Write/Edit 은 배포본 그대로 차단된다(같은 deny 문장).
  2  데몬이 허용(rule-doc-allowed 종료 코드 0)하면 그 Write/Edit 만 통과하고, 조회는
     `cys --socket <s> repair rule-doc-allowed --path <파일>` 하나이며 CYS_NO_AUTOSTART=1 로 간다.
  3  소켓 없음 · 옛 데몬(93) · 거부(95) · 시간 초과 → 오늘의 동작(차단). 시간 초과는 **경로**로만 판정한다(RULINGS
     FV1 — 숫자 한계 없음): 스텁(가짜 cys — 테스트를 돌리는 python 을 절대경로 shebang 으로 쓰는 작은 스크립트)은
     막기 모드에서 "start pid=<pid>" 를 쓰고 20ms 마다 "waiting" 을 덧붙이며 끝내 답하지 않는다. 통과 = 차단(rc 2) +
     "start" + 하트비트 2개 이상 + "answered" 없음 + 그 프로세스가 끝났음. 걸린 시간은 어디서도 판정하지 않는다.
     즉시 실패하는 막기는 하트비트를 남기지 못하므로 실패한다(회귀 3e · 자기 점검 9).
  7  가드의 테스트 예산 손잡이가 실제로 쓰는 한도(RULINGS FV1): GUARD_TEST_REPORT_LIMITS=1 이면 가드가 오류 스트림에
     한 줄 "GUARD-LIMITS budget_s=<합산 예산> probe_cap_s=<조회 한도>" 를 쓴다. 다섯 설정을 그 **값**으로 정확히 단언:
     예산 5초 → 5.000/5.000(1초 넘는 예산을 한도가 따른다) · 60초 → 20.000/20.000(상한) · 0.3초 → 0.300/0.300(낮춤) ·
     변수 없음 → 1.000/0.600(기본) · 테스트 모드 끔 + 변수 5초 → 1.000/0.600(변수 무시).
  8  보고 변수가 없으면 훅의 출력은 오늘과 같다: 같은 입력을 변수 없이/있이 돌려 rc·stdout 이 같고, 변수가 있을 때의
     stderr 에서 그 한 줄만 빼면 변수가 없을 때의 stderr 와 바이트 단위로 같으며, 변수가 없으면 그 줄이 없다.
  9  자기 점검(음성 대조): 판정 함수에 틀린 기록을 먹인다 — 하트비트 없는 기록, 답한 기록, 아직 살아 있는 프로세스의
     기록, 한도 줄이 없거나 두 줄이거나 작은 예산을 무시한 값인 출력 — 모두 판정이 **실패**해야 한다.
  4  Bash 로 규칙 문서를 쓰는 경로(tee·sed -i·리다이렉트)는 표가 있어도 막힌다.
  5  corpus G 의 대표 호출(외부 발행·외부 전송·영구 삭제)은 표가 있든 없든 막힌다(AT9).
  6  규칙 문서가 아닌 쓰기는 조회 없이 통과한다(평시 동작 무변화).
 11  수리 장부(RULINGS HP1 · T4e): 데몬의 수리 장부(이 훅이 놓인 팩의 부모 아래 `local/repairs`)를 좌석이 파일 도구나
     이 가드가 이미 알아보는 셸 쓰기 형태로 쓰면 역할과 상관없이 한 문장으로 막힌다 — 기록 · 레인 줄 · 작업 기록 ·
     주인 지시 표 · grant.json · 일시정지 표시 · 차례 번호 · 묘비 · 보관소 · 점검 작업 상태 · 결함 신고 · 뿌리의 파일.
     역할(master · worker · 리뷰어 두 자리 · CSO) 마다 Write 와 셸 리다이렉트를 시험하고, 다른 쓰기 형태(Edit ·
     MultiEdit · NotebookEdit · tee · cp · sed -i · >> · cd 뒤의 상대 경로 · 심링크 너머)도 시험한다. 좌석이 쓰는
     자리는 수리의 작업 사본 `R-<번호>/staging/`(U3) 하나뿐이라 그곳은 통과하고, 장부를 읽기만 하는 명령도 통과한다.
     장부의 자리는 훅의 환경 변수(CYS_LOCAL_DIR · CYS_PACK_DIR · HOME)로 옮겨지지 않는다.
밀폐: 가짜 `cys` 스텁(PATH 선두) · 임시 디렉터리의 진짜 unix 소켓 파일 · GUARD_TEST_MODE 로 자율주행 깃발을
임시 경로에 둔다 · HOME 임시. 데몬·라이브 팩 무접촉.
시간(RULINGS FV1 · FZ3): 어떤 경과 시간도 한도와 비교하지 않는다 — 한도에 대한 판정은 가드가 밝힌 값만 읽는다.
다만 실제 경로를 보는 경우는 스텁이 남은 예산 안에 떠야 한다: 허용·거부 경우는 20초 안에 답해야 하고, 시간 초과
경우(3c)는 8초 안에 떠서 하트비트를 남겨야 한다. 그보다 늦게 뜰 만큼 부하가 크면 맞는 가드에서도 그 경우가 실패한다.
정리(RULINGS FZ2): 훅은 자기 세션에서 돈다 — 그 프로세스 그룹에 셸·parser·parser 가 띄운 스텁이 모두 있다. 훅이
120초 안에 끝나지 않으면 감시가 그 그룹 전체를 끝내고 거둔다(정리일 뿐 한도에 대해 아무것도 판정하지 않는다).
대조 9vii: 막고 있는 스텁을 남긴 채 멈춘 훅을 그 정리로 끝내면 스텁도 사라진다. 10: 테스트가 띄운 스텁은 끝에 모두
사라졌다.
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

SELF = os.path.dirname(os.path.abspath(__file__))
PACK = os.path.dirname(os.path.dirname(SELF))
GUARD = os.path.join(PACK, "hooks", "guard.sh")
BASH = shutil.which("bash") or "/bin/bash"
fails = []


def check(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
    if not cond:
        fails.append(name)


# The fake `cys`: a small python script run by the test's own interpreter through an absolute shebang (no
# command found through PATH). It records what it did; nothing about time is judged from it (RULINGS FV1):
#   STUB_MODE=block: writes "start pid=<pid>", then appends "waiting" every 20 ms (flushed each time) and never answers
#   STUB_MODE=block-fails: the blocking operation fails at once (a negative control) - exits 97
#   otherwise: exits STUB_RULEDOC_RC (default 95)
STUB_BODY = r"""
import os, sys, time


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
put(ev, "start pid=%d" % os.getpid())
mode = os.environ.get("STUB_MODE", "")
if mode == "block":
    while True:
        put(ev, "waiting")
        time.sleep(0.02)
if mode == "block-fails":
    sys.exit(97)
sys.exit(int(os.environ.get("STUB_RULEDOC_RC", "95")))
"""

# no elapsed time is compared with a limit (RULINGS FV1 / FZ3); the path cases assume the stub STARTS inside the
# remaining budget: the allow / deny cases answer within 20 s, the time-out case starts and leaves heartbeats within 8 s
BIG_BUDGET_MS = "20000"
TIMEOUT_BUDGET_MS = "8000"
# RULINGS FZ2: a hook still running after this long is ended with its whole process group — clean-up only
WATCHDOG_S = 120
LIMITS_VAR = "GUARD_TEST_REPORT_LIMITS"


def parse_evidence(lines):
    """The stub's own record: (started, pid, heartbeats, answered)."""
    started, pid, beats, answered = False, None, 0, False
    for ln in lines:
        parts = ln.split()
        if not parts:
            continue
        if parts[0] == "start":
            started = True
            try:
                pid = int(parts[1].split("=", 1)[1])
            except (IndexError, ValueError):
                pid = None
        elif parts[0] == "waiting":
            beats += 1
        elif parts[0] == "answered":
            answered = True
    return started, pid, beats, answered


def timeout_path_ok(rc, ev, pid_gone):
    """TIME-OUT, judged by its PATH only (no numeric bound): refused (rc 2), the stub started, left at least two
    heartbeats (it was still waiting), never answered, and its process is gone."""
    started, pid, beats, answered = ev
    return rc == 2 and started and pid is not None and beats >= 2 and not answered and pid_gone


def limit_lines(stderr):
    return [ln for ln in stderr.splitlines() if ln.startswith("GUARD-LIMITS ")]


def limits_ok(stderr, budget, cap):
    """The guard stated EXACTLY ONE limits line, with exactly these values."""
    return limit_lines(stderr) == ["GUARD-LIMITS budget_s=%s probe_cap_s=%s" % (budget, cap)]


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
                budget_ms=BIG_BUDGET_MS, test_mode=True, extra=None, guard=None, cwd=None):
            env = hook_env(rc=rc, sock_path=sock_path, active=active, budget_ms=budget_ms, test_mode=test_mode,
                           extra=extra)
            call = {"tool_name": tool, "tool_input": tool_input}
            if cwd:
                call["cwd"] = cwd
            payload = json.dumps(call)
            proc = start_hook(env, guard)
            try:
                out, err = proc.communicate(payload, timeout=WATCHDOG_S)
            except subprocess.TimeoutExpired:
                end_group(proc)                       # clean-up only; the case then fails on its rc
                out, err = proc.communicate()
            calls = open(log).read().splitlines() if os.path.exists(log) else []
            return subprocess.CompletedProcess(proc.args, proc.returncode, out, err), calls

        def hook_env(rc=None, sock_path=sock, active=False, budget_ms=BIG_BUDGET_MS, test_mode=True, extra=None):
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
            return env

        def start_hook(env, guard=None):
            """The hook in a session of its own (RULINGS FZ2): its process group holds the shell, the parser and any
            stub the parser starts."""
            return subprocess.Popen([BASH, guard or GUARD], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                    stderr=subprocess.PIPE, text=True, env=env, start_new_session=True)

        def end_group(proc):
            """Clean-up only — it decides nothing about the limit (RULINGS FZ2): terminate the hook's whole process
            group (the shell, the parser, any stub) and reap the shell."""
            try:
                os.killpg(proc.pid, signal.SIGKILL)
            except (ProcessLookupError, PermissionError):
                pass
            proc.wait()

        # 1 — no token: the released deny, same sentence
        p, calls = run({"file_path": soul, "content": "x"}, rc=95)
        check("1a 표 없음(데몬 거부) → 규칙 문서 Write 차단", p.returncode == 2, repr(p.stderr[-300:]))
        check("1b 배포본과 같은 deny 문장", "헌법파일(soul.md) Write 차단" in p.stderr, repr(p.stderr[-300:]))
        check("1c 조회는 rule-doc-allowed 한 번", len(calls) == 1 and "repair rule-doc-allowed --path %s" % soul in calls[0],
              repr(calls))
        # 2 — the daemon allows: only that write passes; the query carries the socket and no-autostart
        p, calls = run({"file_path": soul, "content": "x"}, rc=0)
        check("2a 데몬 허용 → 통과(exit 0)", p.returncode == 0, repr(p.stderr[-300:]))
        # (the parser's own stderr is not forwarded by the device — guard.sh sends it to a temp file)
        check("2b 허용이면 deny JSON 없음(stdout 비어 있음)", p.stdout.strip() == "", repr(p.stdout[-200:]))
        check("2c --socket 명시 + CYS_NO_AUTOSTART=1",
              len(calls) == 1 and calls[0].startswith("--socket %s repair rule-doc-allowed" % sock)
              and calls[0].endswith("NO_AUTOSTART=1"), repr(calls))
        p, _ = run({"file_path": directive, "old_string": "a", "new_string": "b"}, tool="Edit", rc=0)
        check("2d Edit 도 같은 규칙(허용)", p.returncode == 0, repr(p.stderr[-200:]))
        p, _ = run({"file_path": directive, "old_string": "a", "new_string": "b"}, tool="Edit", rc=95)
        check("2e Edit 거부면 차단", p.returncode == 2)
        p, _ = run({"file_path": soul, "content": "x"}, rc=0, active=True)
        check("2f 자율주행(STRICT) 중에도 표는 데몬의 판정 그대로(허용)", p.returncode == 0, repr(p.stderr[-200:]))
        p, _ = run({"file_path": soul, "content": "x"}, rc=95, active=True)
        check("2g 자율주행 중 거부면 차단", p.returncode == 2)
        # 3 — every failure keeps today's behaviour
        p, calls = run({"file_path": soul, "content": "x"}, rc=0, sock_path=os.path.join(root, "none.sock"))
        check("3a 소켓 없음 → 조회 없이 차단", p.returncode == 2 and not calls, repr((p.returncode, calls)))
        p, _ = run({"file_path": soul, "content": "x"}, rc=93)
        check("3b 옛 데몬(93) → 차단", p.returncode == 2)
        # evidence helpers: the stub's own record
        seq = [0]
        stub_pids = []

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

        def drive(mode, budget_ms):
            base = ev_path()
            p, _ = run({"file_path": soul, "content": "x"}, rc=0, budget_ms=budget_ms,
                       extra={"STUB_EVIDENCE": base, "STUB_MODE": mode})
            lines = open(base).read().splitlines() if os.path.exists(base) else []
            ev = parse_evidence(lines)
            if ev[1] is not None:
                stub_pids.append(ev[1])
            return p, ev, gone(ev[1])

        def gone_soon(pid, within=5.0):
            """A killed orphan may stay a zombie until it is reaped by the system: look again for a few seconds
            (clean-up verification of an event that is certain — nothing about the limit)."""
            end = time.monotonic() + within
            while not gone(pid) and time.monotonic() < end:
                time.sleep(0.05)
            return gone(pid)

        def show(p, ev):
            return "rc=%d start=%s pid=%s heartbeats=%d answered=%s" % ((p.returncode,) + tuple(ev))

        # 3c — the daemon's answer never comes: refused, by the time-out path (RULINGS FV1 — no numeric bound)
        p, ev, pid_gone = drive("block", TIMEOUT_BUDGET_MS)
        ok = timeout_path_ok(p.returncode, ev, pid_gone)
        check("3c 시간 초과 → 차단: 스텁이 시작했고(start) 하트비트 %d개를 남기며 기다리는 중이었고 답하지 않았으며 그 "
              "프로세스는 끝났다(걸린 시간은 판정하지 않는다)" % ev[2], ok, "" if ok else show(p, ev))
        # 3e — a blocking operation that fails at once leaves no heartbeat: the time-out judgment FAILS
        p, ev, pid_gone = drive("block-fails", TIMEOUT_BUDGET_MS)
        bad = timeout_path_ok(p.returncode, ev, pid_gone)
        check("3e 막기가 즉시 실패한 스텁에서는 시간 초과 판정이 통과하지 않는다(스텁의 하트비트 %d개)" % ev[2], not bad,
              "" if not bad else show(p, ev))

        # 7 — the limits the guard will use, as it states them (RULINGS FV1): exact values, no duration
        def limits_case(label, budget_ms, test_mode, budget, cap):
            p, _ = run({"file_path": soul, "content": "x"}, rc=0, budget_ms=budget_ms, test_mode=test_mode,
                       extra={LIMITS_VAR: "1"})
            ok = limits_ok(p.stderr, budget, cap)
            check("%s — 가드가 밝힌 한도: 합산 예산 %s초 · 조회 한도 %s초" % (label, budget, cap), ok,
                  "" if ok else repr(limit_lines(p.stderr)) + " " + repr(p.stderr[-300:]))

        limits_case("7a 예산 5초 → 1초 넘는 예산을 한도가 따른다", "5000", True, "5.000", "5.000")
        limits_case("7b 예산 60초 → 상한 20초", "60000", True, "20.000", "20.000")
        limits_case("7c 작은 예산 0.3초 → 한도를 낮춘다", "300", True, "0.300", "0.300")
        limits_case("7d 변수 없음 → 기본(합산 1초 · 조회 0.6초)", None, True, "1.000", "0.600")
        limits_case("7e 테스트 모드 끔 + 변수 5초 → 변수 무시, 기본", "5000", False, "1.000", "0.600")

        # 8 — without the report variable the hook's output is today's: the variable adds that one line, nothing else
        samples = [
            ("규칙 문서 Write 허용", {"file_path": soul, "content": "x"}, "Write", 0),
            ("규칙 문서 Write 거부", {"file_path": soul, "content": "x"}, "Write", 95),
            ("corpus G Bash", {"command": "git push origin main"}, "Bash", 0),
            ("규칙 문서가 아닌 쓰기", {"file_path": os.path.join(pack, "bin", "b.py"), "content": "x"}, "Write", 95),
        ]
        for label, ti, tool, rc in samples:
            plain, _ = run(ti, tool=tool, rc=rc)
            told, _ = run(ti, tool=tool, rc=rc, extra={LIMITS_VAR: "1"})
            rest = "".join(ln for ln in told.stderr.splitlines(True) if not ln.startswith("GUARD-LIMITS "))
            same = (plain.returncode == told.returncode and plain.stdout == told.stdout and rest == plain.stderr
                    and not limit_lines(plain.stderr) and len(limit_lines(told.stderr)) == 1)
            check("8 보고 변수 없음 = 오늘의 출력(%s): rc·stdout 같음 · stderr 는 한 줄만 다름" % label, same,
                  "" if same else repr((plain.returncode, told.returncode, plain.stdout[-120:], told.stdout[-120:],
                                        plain.stderr[-200:], told.stderr[-200:])))

        # 9 — self-checks of the predicates (negative controls): fabricated wrong records must FAIL
        check("9i 하트비트 없는 기록 → 시간 초과 판정 실패",
              not timeout_path_ok(2, parse_evidence(["start pid=1"]), True))
        check("9ii 답한 기록 → 시간 초과 판정 실패",
              not timeout_path_ok(2, parse_evidence(["start pid=1", "waiting", "waiting", "answered"]), True))
        check("9iii 프로세스가 아직 살아 있는 기록 → 시간 초과 판정 실패",
              not timeout_path_ok(2, parse_evidence(["start pid=1", "waiting", "waiting"]), False))
        check("9iv 한도 줄이 없는 출력 → 한도 판정 실패", not limits_ok("AUTOPILOT GUARD DENY: x\n", "0.300", "0.300"))
        check("9v 작은 예산을 무시한 값(1.000/0.600) → 0.3초 판정 실패",
              not limits_ok("GUARD-LIMITS budget_s=1.000 probe_cap_s=0.600\n", "0.300", "0.300"))
        check("9vi 한도 줄이 두 줄인 출력 → 한도 판정 실패",
              not limits_ok("GUARD-LIMITS budget_s=0.300 probe_cap_s=0.300\n" * 2, "0.300", "0.300"))

        # 9vii — RULINGS FZ2: a hook stuck with a stub still blocking is ended by the clean-up, and the stub is gone
        base = ev_path()
        env = hook_env(rc=0, budget_ms=BIG_BUDGET_MS, extra={"STUB_EVIDENCE": base, "STUB_MODE": "block"})
        proc = start_hook(env)
        payload = json.dumps({"tool_name": "Write", "tool_input": {"file_path": soul, "content": "x"}})
        ev = parse_evidence([])
        end = time.monotonic() + WATCHDOG_S          # wait for the stub to start (clean-up bound only)
        while time.monotonic() < end:
            try:
                proc.communicate(payload, timeout=0.2)
                break                                 # the hook ended by itself: the control is void (fails below)
            except subprocess.TimeoutExpired:
                payload = None
            ev = parse_evidence(open(base).read().splitlines() if os.path.exists(base) else [])
            if ev[1] is not None and ev[2] >= 1:
                break
        alive_before = ev[1] is not None and not gone(ev[1]) and proc.poll() is None
        end_group(proc)
        # judged BEFORE draining the pipes: a parser left alive keeps the hook's stderr open, and draining would wait
        # until the guard's own time-out (about 20 s) had ended the stub — the 5 s look is well inside that
        stub_gone = ev[1] is not None and gone_soon(ev[1])
        proc.communicate()
        if ev[1] is not None:
            stub_pids.append(ev[1])
        cleaned = alive_before and stub_gone and proc.returncode == -signal.SIGKILL
        check("9vii 막고 있는 스텁(pid %s · 하트비트 %d개)을 남긴 채 멈춘 훅을 정리(프로세스 그룹 종료)하면 그 스텁도 "
              "사라진다" % (ev[1], ev[2]), cleaned,
              "" if cleaned else "alive_before=%s hook rc=%s stub gone within 5 s of the clean-up=%s" % (
                  alive_before, proc.returncode, stub_gone))

        # 4 — Bash writers of rule documents stay denied with a token
        for cmd in ("tee %s < /dev/null" % soul, "sed -i 's/a/b/' %s" % soul, "echo x > %s" % soul):
            p, _ = run({"command": cmd}, tool="Bash", rc=0)
            check("4 Bash 규칙 문서 쓰기는 표가 있어도 차단: %s" % cmd.split()[0], p.returncode == 2,
                  repr(p.stderr[-200:]))
        # 5 — corpus-G representatives (AT9): denied with and without a token
        for cmd in ("git push origin main", "scp a.txt host:/tmp/", "rm -rf /home/x/project",
                    "curl -X POST -d x https://example.invalid", "gh release create v1"):
            for rc in (0, 95):
                p, _ = run({"command": cmd}, tool="Bash", rc=rc)
                check("5 corpus G 차단 유지(rc=%d): %s" % (rc, cmd.split()[0]), p.returncode == 2,
                      repr(p.stderr[-200:]))
        # 6 — non-rule writes: no query, allowed as before
        p, calls = run({"file_path": os.path.join(pack, "bin", "a.py"), "content": "x"}, rc=95)
        check("6 규칙 문서가 아닌 쓰기 — 조회 없이 통과", p.returncode == 0 and not calls, repr((p.returncode, calls)))
        # 11 — RULINGS HP1 (T4e): no seat writes the daemon's repair ledger with a plain tool
        ledger = os.path.join(os.path.dirname(PACK), "local", "repairs")   # the root of THIS hook's pack
        sha = "a" * 64
        stores = [
            ("기록", "R-7/record.json", "cys repair begin"),
            ("레인 줄", "R-7/lanes/base.json", "cys repair begin"),
            ("작업 기록", "R-7/ops/op-1.json", "cys repair begin"),
            ("주인 지시 표", "owner-tokens/%s.json" % sha, "cys repair owner-token ask --path"),
            ("자동 수리 허락", "grant.json", "cys repair grant ask"),
            ("일시정지 표시", "grant-pause/base.json", "cys repair grant ask"),
            ("차례 번호", "grant-seq/1.json", "cys repair grant ask"),
            ("묘비", "tombstones/base.json", "cys repair commit"),
            ("보관소", "displaced/R-7/20261115T000000Z/sealed/guard.sh", "cys repair revert"),
            ("점검 작업 상태", "lanes/base.job.json", "cys repair sweep"),
            ("결함 신고", "intake/base/abcd.json", "`report` 동사"),
            ("뿌리의 파일", "lanes-at-upgrade.json", "cys repair begin"),
        ]
        roles = ("master", "worker", "reviewer-codex", "reviewer-gemini", "cso")

        def ledger_refused(p, verb):
            return (p.returncode == 2 and "수리 장부" in p.stderr and "cys repair" in p.stderr and verb in p.stderr)

        for label, rel, verb in stores:
            target = os.path.join(ledger, rel)
            bad = []
            for role in roles:
                for tool, ti in (("Write", {"file_path": target, "content": "{}"}),
                                 ("Bash", {"command": "echo '{}' > %s" % target})):
                    p, calls = run(ti, tool=tool, rc=0, extra={"CYS_ROLE": role})
                    if not ledger_refused(p, verb) or calls:
                        bad.append("%s/%s rc=%d %r" % (role, tool, p.returncode, p.stderr[-160:]))
            check("11 장부의 %s(%s) — 역할 %d개 × Write·셸 리다이렉트 모두 한 문장으로 차단, 할 일의 동사(%s) 안내"
                  % (label, rel, len(roles), verb), not bad, "; ".join(bad[:3]))
        token = os.path.join(ledger, "owner-tokens", "%s.json" % sha)
        os.makedirs(os.path.join(root, "src"), exist_ok=True)
        link = os.path.join(root, "ledger-link")
        os.symlink(ledger, link)
        for label, tool, ti in (
                ("Edit", "Edit", {"file_path": token, "old_string": "a", "new_string": "b"}),
                ("MultiEdit", "MultiEdit", {"file_path": token, "edits": []}),
                ("NotebookEdit", "NotebookEdit", {"notebook_path": token, "new_source": "x"}),
                ("tee", "Bash", {"command": "printf x | tee %s" % token}),
                ("cp", "Bash", {"command": "cp %s %s" % (os.path.join(root, "src", "t.json"), token)}),
                ("sed -i", "Bash", {"command": "sed -i 's/a/b/' %s" % token}),
                ("덧붙이기 >>", "Bash", {"command": "echo x >> %s" % token}),
                ("붙여 쓴 >경로", "Bash", {"command": "echo x >%s" % token}),
                ("cd 뒤 상대 경로", "Bash", {"command": "cd %s && echo x > b.json" % os.path.dirname(token)}),
                ("심링크 너머 Write", "Write", {"file_path": os.path.join(link, "grant.json"), "content": "{}"}),
                ("심링크 너머 리다이렉트", "Bash", {"command": "echo x > %s" % os.path.join(link, "owner-tokens", "x.json")})):
            p, _ = run(ti, tool=tool, rc=0)
            ok = ledger_refused(p, "cys repair")
            check("11 다른 쓰기 형태도 차단: %s" % label, ok, "" if ok else "rc=%d %r" % (p.returncode, p.stderr[-200:]))
        staging = os.path.join(ledger, "R-7", "staging", "hooks", "guard.sh")
        for label, tool, ti in (
                ("작업 사본 Write", "Write", {"file_path": staging, "content": "x"}),
                ("작업 사본 cp", "Bash", {"command": "cp %s %s" % (os.path.join(root, "src", "g.sh"), staging)}),
                ("작업 사본 sed -i", "Bash", {"command": "sed -i 's/a/b/' %s" % staging}),
                ("장부 읽기 cat", "Bash", {"command": "cat %s" % os.path.join(ledger, "grant.json")}),
                ("장부 읽기 ls", "Bash", {"command": "ls -la %s" % ledger})):
            p, _ = run(ti, tool=tool, rc=0)
            check("11 통과: %s" % label, p.returncode == 0,
                  "" if p.returncode == 0 else "rc=%d %r" % (p.returncode, p.stderr[-200:]))
        # the root is THIS hook's — the hook's environment does not move it
        elsewhere = os.path.join(root, "elsewhere")
        moved = {"CYS_LOCAL_DIR": elsewhere, "CYS_PACK_DIR": os.path.join(elsewhere, "pack"),
                 "HOME": os.path.join(elsewhere, "home")}
        p, _ = run({"file_path": os.path.join(ledger, "grant.json"), "content": "{}"}, rc=0, extra=moved)
        ok = ledger_refused(p, "cys repair grant ask")
        check("11 훅의 환경 변수(CYS_LOCAL_DIR · CYS_PACK_DIR · HOME)를 옮겨도 데몬의 장부는 그대로 막힌다", ok,
              "" if ok else "rc=%d %r" % (p.returncode, p.stderr[-200:]))
        p, _ = run({"file_path": os.path.join(elsewhere, "repairs", "grant.json"), "content": "{}"}, rc=0, extra=moved)
        check("11 환경 변수가 가리키는 다른 자리는 장부가 아니다(막는 자리는 훅이 놓인 팩에서 정해진다)",
              p.returncode == 0, "" if p.returncode == 0 else "rc=%d %r" % (p.returncode, p.stderr[-200:]))

        # 12 — RULINGS ID1 (T4e round 2): the place of a word is judged as the shell will reach it — every relative word
        #      also from each directory the command itself changes to (cd, pushd), and a root whose spelling holds a space
        tok = os.path.join(ledger, "owner-tokens", "%s.json" % sha)
        src = os.path.join(root, "src", "t.json")
        for label, cmd in (
                ("cd 작업 사본 뒤 ../../ 로 빠져나온 cp", "cd %s && cp %s ../../owner-tokens/%s.json"
                 % (os.path.join(ledger, "R-1", "staging"), src, sha)),
                ("pushd 작업 사본 뒤 ../../grant.json", "pushd %s && cp %s ../../grant.json"
                 % (os.path.join(ledger, "R-1", "staging"), src)),
                ("cd 두 번 이어서", "cd %s && cd staging && cp %s ../../owner-tokens/x.json"
                 % (os.path.join(ledger, "R-1"), src)),
                ("cd 장부 뒤 상대 리다이렉트", "cd %s; echo on > grant.json" % ledger)):
            p, _ = run({"command": cmd}, tool="Bash", rc=0)
            ok = ledger_refused(p, "cys repair")
            check("12 %s → 차단" % label, ok, "" if ok else "rc=%d %r" % (p.returncode, p.stderr[-200:]))
        # a root whose spelling holds a space: this guard copied into "<tmp>/space home/pack/hooks/"
        spack = os.path.join(root, "space home", "pack")
        os.makedirs(os.path.join(spack, "hooks"))
        for f in ("guard.sh", "_lib.sh"):
            shutil.copy(os.path.join(PACK, "hooks", f), os.path.join(spack, "hooks", f))
        sguard = os.path.join(spack, "hooks", "guard.sh")
        sl = os.path.join(root, "space home", "local", "repairs")
        esc = sl.replace(" ", "\\ ")
        for label, tool, ti in (
                ("Write", "Write", {"file_path": os.path.join(sl, "grant.json"), "content": "{}"}),
                ("리다이렉트", "Bash", {"command": 'echo on > "%s/grant.json"' % sl}),
                ("덧붙이기", "Bash", {"command": 'echo on >> "%s/grant.json"' % sl}),
                ("cp 안으로", "Bash", {"command": 'cp %s "%s/owner-tokens/%s.json"' % (src, sl, sha)}),
                ("tee", "Bash", {"command": 'printf x | tee "%s/grant.json"' % sl}),
                ("mv 안으로", "Bash", {"command": 'mv %s "%s/tombstones/base.json"' % (src, sl)}),
                ("sed -i", "Bash", {"command": "sed -i s/a/b/ '%s/grant.json'" % sl}),
                ("역슬래시로 띄운 공백", "Bash", {"command": "echo on > %s/grant.json" % esc}),
                ("cd 뒤 ../../", "Bash", {"command": 'cd "%s/R-1/staging" && cp %s ../../grant.json' % (sl, src)})):
            p, _ = run(ti, tool=tool, rc=0, guard=sguard)
            ok = ledger_refused(p, "cys repair")
            check("12 공백이 든 장부 자리(%s) → 차단" % label, ok, "" if ok else "rc=%d %r" % (p.returncode, p.stderr[-200:]))
        p, _ = run({"command": 'cp %s "%s/R-1/staging/x.md"' % (src, sl)}, tool="Bash", rc=0, guard=sguard)
        check("12 공백이 든 장부 자리의 작업 사본 → 통과", p.returncode == 0,
              "" if p.returncode == 0 else "rc=%d %r" % (p.returncode, p.stderr[-200:]))

        # 13 — RULINGS ID3 (T4e round 2): a read is not a write — a redirection counts only when ITS target is a ledger place
        #      outside the working copy; a writer counts only for the operands of its own segment; for cp / mv / install / ln
        #      only the destination is written (mv also writes its source)
        out = os.path.join(root, "out")
        os.makedirs(out, exist_ok=True)
        tmpf = os.path.join(out, "t.json")
        for label, cmd, guard in (
                ("ls 2>/dev/null", "ls %s 2>/dev/null" % os.path.join(ledger, "R-3"), None),
                ("cat 2>&1 | head", "cat %s 2>&1 | head -5" % os.path.join(ledger, "grant.json"), None),
                ("grep -r 2>/dev/null | wc", "grep -r tok %s 2>/dev/null | wc -l" % os.path.join(ledger, "owner-tokens"), None),
                ("head > 임시 파일", "head -n 3 %s > %s" % (os.path.join(ledger, "tombstones", "base.json"),
                                                        os.path.join(out, "head.txt")), None),
                ("cat > 임시 파일", "cat %s > %s" % (os.path.join(ledger, "grant.json"), os.path.join(out, "copy.json")), None),
                ("밖으로 cp", "cp %s %s" % (os.path.join(ledger, "grant.json"), os.path.join(out, "g.json")), None),
                ("밖으로 cp -r", "cp -r %s %s" % (os.path.join(ledger, "R-3"), os.path.join(out, "r3")), None),
                ("따옴표 글 안의 언급", 'echo "the ledger %s is written by the daemon" > %s'
                 % (os.path.join(ledger, "grant.json"), os.path.join(out, "note.txt")), None),
                ("cd 뒤 ls 2>/dev/null", "cd %s && ls -la 2>/dev/null" % ledger, None),
                ("cd 뒤 cat 2>&1 | head", "cd %s && cat record.json 2>&1 | head" % os.path.join(ledger, "R-3"), None),
                ("공백 자리 cat 2>/dev/null", 'cat "%s/grant.json" 2>/dev/null' % sl, sguard),
                ("공백 자리 밖으로 cp", 'cp "%s/grant.json" %s' % (sl, os.path.join(out, "s.json")), sguard)):
            p, _ = run({"command": cmd}, tool="Bash", rc=0, guard=guard)
            check("13 읽기는 쓰기가 아니다 — 통과: %s" % label, p.returncode == 0,
                  "" if p.returncode == 0 else "rc=%d %r" % (p.returncode, p.stderr[-200:]))
        for label, plain, spaced, after_cd in (
                (">", "echo on > %s/grant.json" % ledger, 'echo on > "%s/grant.json"' % sl,
                 "cd %s && echo on > grant.json" % ledger),
                (">>", "echo on >> %s/grant.json" % ledger, 'echo on >> "%s/grant.json"' % sl,
                 "cd %s && echo on >> grant.json" % ledger),
                ("안으로 cp", "cp %s %s/owner-tokens/x.json" % (tmpf, ledger), 'cp %s "%s/owner-tokens/x.json"' % (tmpf, sl),
                 "cd %s/owner-tokens && cp %s x.json" % (ledger, tmpf)),
                ("cp -t 안으로", "cp -t %s/owner-tokens %s" % (ledger, tmpf), 'cp -t "%s/owner-tokens" %s' % (sl, tmpf),
                 "cd %s && cp -t owner-tokens %s" % (ledger, tmpf)),
                ("tee 안으로", "echo x | tee %s/grant.json" % ledger, 'echo x | tee "%s/grant.json"' % sl,
                 "cd %s && echo x | tee grant.json" % ledger),
                ("mv 밖으로(원본을 지운다)", "mv %s/grant.json %s" % (ledger, tmpf), 'mv "%s/grant.json" %s' % (sl, tmpf),
                 "cd %s && mv grant.json %s" % (ledger, tmpf)),
                ("읽고 장부 안으로 리다이렉트", "cat %s/grant.json > %s/grant-copy.json" % (ledger, ledger),
                 'cat "%s/grant.json" > "%s/grant-copy.json"' % (sl, sl),
                 "cd %s && cat grant.json > grant-copy.json" % ledger)):
            bad = []
            for how, cmd, guard in (("곧은 자리", plain, None), ("공백 자리", spaced, sguard), ("cd 뒤", after_cd, None)):
                p, _ = run({"command": cmd}, tool="Bash", rc=0, guard=guard)
                if not ledger_refused(p, "cys repair"):
                    bad.append("%s rc=%d %r" % (how, p.returncode, p.stderr[-160:]))
            check("13 쓰기는 막힌다: %s (곧은 자리 · 공백 자리 · cd 뒤)" % label, not bad, "; ".join(bad))

        # 10 — RULINGS FZ2: every stub this test started is gone at its end
        left = [pid for pid in stub_pids if not gone_soon(pid)]
        check("10 테스트가 띄운 스텁 %d개가 끝에 모두 사라졌다" % len(stub_pids), not left, "" if not left else repr(left))
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
