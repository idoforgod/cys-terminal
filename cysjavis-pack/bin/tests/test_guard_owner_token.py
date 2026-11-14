#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_guard_owner_token.py — guard.sh 가 규칙 문서 쓰기를 **주인 지시 표**로만 더 허용하는가 (0.15.0 M13).

무엇을 재는가(C10 10.5 · O-C10-4 · RULINGS AT9):
  1  표가 없으면(데몬이 거부) 규칙 문서 Write/Edit 은 배포본 그대로 차단된다(같은 deny 문장).
  2  데몬이 허용(rule-doc-allowed 종료 코드 0)하면 그 Write/Edit 만 통과하고, 조회는
     `cys --socket <s> repair rule-doc-allowed --path <파일>` 하나이며 CYS_NO_AUTOSTART=1 로 간다.
  3  소켓 없음 · 옛 데몬(93) · 거부(95) · 시간 초과 → 오늘의 동작(차단). 시간 초과는 스텁이 **스스로 잰** 기다림으로
     판정한다(RULINGS EW1): 스텁(가짜 cys — 테스트를 돌리는 python 을 절대경로 shebang 으로 쓰는 작은 스크립트)은 첫
     동작으로 자기 단조 시계 t0 를 찍고, 막기 모드에서 20ms 마다 "waiting Δ" 를 덧붙여 쓰며 끝내 답하지 않는다.
     통과 = "start" + 하트비트 + "answered" 없음 + 마지막 하트비트 Δ 가 기대 한도의 [−하한 관용, +상한 관용] 안 +
     그 프로세스가 끝났음. 가드의 거부 문장이나 훅 전체의 걸린 시간은 증거가 아니다. 즉시 실패하는 막기는
     하트비트를 남기지 못하므로 실패한다(회귀 3e · 자기 점검 9).
  7  가드의 테스트 예산 손잡이가 실제로 쓰는 조회 한도(RULINGS EW2 · ER1 · EM2 — 제품 변경 없이, 스텁이 잰 값으로):
     예산 5초 → D=1.5초 뒤 "answered Δ≥D" 와 허용 · 상한 위 60초 → D=3초 뒤 허용, 그리고 막기 모드의 마지막
     하트비트가 20초 근처(상한) · 작은 예산 0.3초 → 마지막 하트비트가 0.3초 근처 · 변수 없음 → 0.6초 근처 ·
     테스트 모드 끔 + 변수 5초 → 0.6초 근처(변수 무시).
  9  자기 점검(RULINGS EW2 음성 대조): 판정 함수에 지어낸 틀린 기록을 먹인다 — (i) 한도가 0.1초로 줄었는데 무관한
     기동 지연으로 훅 전체가 0.8초 걸린 기록, (ii) 막기가 즉시 실패했는데 무관한 지연으로 훅 전체가 6초 걸린
     기록, (iii) D 전에 답한 기록, (iv) 한도가 너무 긴 기록 — 모두 판정이 **실패**해야 한다.
  4  Bash 로 규칙 문서를 쓰는 경로(tee·sed -i·리다이렉트)는 표가 있어도 막힌다.
  5  corpus G 의 대표 호출(외부 발행·외부 전송·영구 삭제)은 표가 있든 없든 막힌다(AT9).
  6  규칙 문서가 아닌 쓰기는 조회 없이 통과한다(평시 동작 무변화).
밀폐: 가짜 `cys` 스텁(PATH 선두) · 임시 디렉터리의 진짜 unix 소켓 파일 · GUARD_TEST_MODE 로 자율주행 깃발을
임시 경로에 둔다 · HOME 임시. 데몬·라이브 팩 무접촉.
시간 관용도(RULINGS EW1 · EW3): 한도에 대한 모든 판정은 스텁이 자기 시계로 잰 Δ 만 읽는다. 하한 관용은 스텁
자신의 기동(t0 이전 · 가드의 시계로는 이미 흐른 시간)과 예산이 걸린 경우 가드 자신의 전처리(합산 예산 안에서
조회 전에 쓴 시간), 그리고 하트비트 간격(20ms)을 흡수한다 — 그만큼 마지막 하트비트가 한도보다 이르게 찍히기
때문이다. 상한 관용은 가드가 스케줄에서 밀려 시간 초과를 늦게 처리하는 만큼을 흡수한다(스텁은 죽은 뒤에 기록할
수 없다). 허용·거부 경우는 큰 예산(20초)을 쓴다 — 관용도를 키웠을 뿐 조회는 여전히 남은 예산 안에 끝나야 한다.
극단적 부하에서는 한 경우가 크게 실패하고 다시 돌린다 — 조용히 통과하지 않는다.
`STUB_START_DELAY=<초>` 는 스텁의 t0 이전에 느린 기동을 주입한다(부하 재현).
출력: PASS/FAIL 행 · 실패 시 exit 1 · 전부 통과 시 GUARD-OWNER-TOKEN-OK.
"""
import json
import os
import shutil
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


# The fake `cys` (RULINGS EW1): a small python script run by the test's own interpreter through an
# absolute shebang (no command found through PATH). It takes its own monotonic t0 as its first act and
# records every claim about the limit itself:
#   STUB_MODE=answer-after (STUB_D=<s>): waits until its own delta >= D, writes "answered <delta>", exits 0
#   STUB_MODE=block: appends "waiting <delta>" every 20 ms (flushed each time) and never answers
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
put(ev, "start pid=%d" % os.getpid())
mode = os.environ.get("STUB_MODE", "")
if mode == "answer-after":
    d = float(os.environ["STUB_D"])
    while time.monotonic() - t0 < d:
        time.sleep(min(0.01, max(0.0, d - (time.monotonic() - t0))))
    put(ev, "answered %.4f" % (time.monotonic() - t0))
    sys.exit(0)
if mode == "block":
    while True:
        put(ev, "waiting %.4f" % (time.monotonic() - t0))
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
# tolerances around an expected limit, applied to the stub's LAST heartbeat (EW1, see the header):
# below = the stub's own start-up + the guard's prelude inside the budget + one heartbeat interval;
# above = the guard handling its time-out late when it is descheduled
TOL_SHORT = (0.25, 0.20)   # the 0.3 s and 0.6 s limits
TOL_LONG = (0.50, 1.00)    # the 5 s and 20 s limits


def parse_evidence(lines):
    """The stub's own record: (started, pid, heartbeats [delta...], answered delta or None)."""
    started, pid, beats, answered = False, None, [], None
    for ln in lines:
        parts = ln.split()
        try:
            if parts[0] == "start":
                started = True
                pid = int(parts[1].split("=", 1)[1])
            elif parts[0] == "waiting":
                beats.append(float(parts[1]))
            elif parts[0] == "answered":
                answered = float(parts[1])
        except (IndexError, ValueError):
            pass
    return started, pid, beats, answered


def allow_ok(rc, ev, d):
    """ALLOW: the guard allowed AND the stub recorded "answered" with its own delta >= D."""
    return rc == 0 and ev[3] is not None and ev[3] >= d


def timeout_ok(rc, ev, expected, tol, pid_gone):
    """TIME-OUT: started, heartbeats, NO answer, the last heartbeat (the measured limit) within
    [expected - tol[0], expected + tol[1]], the process gone. The refusal itself is only rc 2 - the
    guard's sentence and the whole-hook time are not evidence."""
    started, pid, beats, answered = ev
    return (rc == 2 and started and pid is not None and len(beats) >= 2 and answered is None
            and expected - tol[0] <= beats[-1] <= expected + tol[1] and pid_gone)


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
            t0 = time.monotonic()
            p = subprocess.run([BASH, GUARD], input=json.dumps({"tool_name": tool, "tool_input": tool_input}),
                               capture_output=True, text=True, env=env, timeout=60)
            calls = open(log).read().splitlines() if os.path.exists(log) else []
            return p, calls, time.monotonic() - t0

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

        def drive(mode, budget_ms, d=None, test_mode=True):
            base = ev_path()
            extra = {"STUB_EVIDENCE": base, "STUB_MODE": mode}
            if d is not None:
                extra["STUB_D"] = "%g" % d
            p, _, el = run({"file_path": soul, "content": "x"}, rc=0, budget_ms=budget_ms,
                           test_mode=test_mode, extra=extra)
            lines = open(base).read().splitlines() if os.path.exists(base) else []
            ev = parse_evidence(lines)
            return p, ev, gone(ev[1]), el

        def show(ev):
            started, pid, beats, answered = ev
            return "start=%s pid=%s heartbeats=%d last=%s answered=%s" % (
                started, pid, len(beats), ("%.3f" % beats[-1]) if beats else None, answered)

        def last(ev):
            return ("%.3f" % ev[2][-1]) if ev[2] else "없음"

        # 3c — the daemon's answer never comes: the stub's LAST heartbeat is the limit it measured
        p, ev, pid_gone, el = drive("block", TIMEOUT_BUDGET_MS)
        ok = timeout_ok(p.returncode, ev, 5.0, TOL_LONG, pid_gone)
        check("3c 시간 초과 → 차단: 스텁이 잰 마지막 하트비트 %s초(기대 5초 −%.2f/+%.2f) · 답 없음 · 그 프로세스는 끝났다"
              % (last(ev), TOL_LONG[0], TOL_LONG[1]), ok,
              "" if ok else "%s rc=%d hook=%.2fs" % (show(ev), p.returncode, el))
        # 3e — a blocking operation that fails at once leaves no heartbeat: the time-out judgment FAILS
        p, ev, pid_gone, el = drive("block-fails", TIMEOUT_BUDGET_MS)
        bad = timeout_ok(p.returncode, ev, 5.0, TOL_LONG, pid_gone)
        check("3e 막기가 즉시 실패한 스텁에서는 시간 초과 판정이 통과하지 않는다(스텁의 하트비트 %d개)" % len(ev[2]), not bad,
              "" if not bad else "%s rc=%d" % (show(ev), p.returncode))

        # 7 — the knob's effective per-query limit, from the stub's own measurement (RULINGS EW2)
        def allow_case(label, budget_ms, d):
            p, ev, _, el = drive("answer-after", budget_ms, d=d)
            ok = allow_ok(p.returncode, ev, d)
            check("%s — 스텁이 잰 답 시점 Δ=%s초 ≥ D=%g초, 가드 허용" % (
                label, ("%.3f" % ev[3]) if ev[3] is not None else "없음", d), ok,
                "" if ok else "%s rc=%d hook=%.2fs" % (show(ev), p.returncode, el))

        def limit_case(label, budget_ms, expected, tol, test_mode=True):
            p, ev, pid_gone, el = drive("block", budget_ms, test_mode=test_mode)
            ok = timeout_ok(p.returncode, ev, expected, tol, pid_gone)
            check("%s — 스텁이 잰 마지막 하트비트 %s초(기대 %g초 −%.2f/+%.2f)" % (
                label, last(ev), expected, tol[0], tol[1]), ok,
                "" if ok else "%s rc=%d hook=%.2fs" % (show(ev), p.returncode, el))

        allow_case("7a 예산 5초 → 한도가 예산을 따랐다", "5000", 1.5)
        allow_case("7b 예산 60초(상한 위) → 1초 넘는 예산은 한도를 올린다", "60000", 3.0)
        limit_case("7c 예산 60초 → 상한 20초에서 끝났다", "60000", 20.0, TOL_LONG)
        limit_case("7d 작은 예산 0.3초 → 한도를 낮추기만 한다", "300", 0.3, TOL_SHORT)
        limit_case("7e 변수 없음 → 기본 한도 0.6초", None, 0.6, TOL_SHORT)
        limit_case("7f 테스트 모드 끔 + 변수 5초 → 변수 무시, 0.6초", "5000", 0.6, TOL_SHORT, test_mode=False)

        # 9 — self-checks of the predicates (RULINGS EW2 negative controls, fabricated wrong records)
        lowered = parse_evidence(["start pid=1"] + ["waiting %.3f" % (0.02 * k) for k in range(1, 6)])
        check("9i 한도가 0.1초로 줄어든 기록(훅 전체 0.8초여도) → 판정 실패", not timeout_ok(2, lowered, 0.6, TOL_SHORT, True))
        no_beat = parse_evidence(["start pid=1"])
        check("9ii 막기가 즉시 실패한 기록(훅 전체 6초여도) → 판정 실패", not timeout_ok(2, no_beat, 5.0, TOL_LONG, True))
        early = parse_evidence(["start pid=1", "answered 0.0100"])
        check("9iii D 전에 답한 기록 → 허용 판정 실패", not allow_ok(0, early, 1.5))
        too_long = parse_evidence(["start pid=1"] + ["waiting %.3f" % (0.1 * k) for k in range(1, 60)])
        check("9iv 한도가 너무 긴 기록(5.9초 · 기대 0.6초) → 판정 실패", not timeout_ok(2, too_long, 0.6, TOL_SHORT, True))

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
