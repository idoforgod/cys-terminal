#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_guard_owner_token.py — guard.sh 가 규칙 문서 쓰기를 **주인 지시 표**로만 더 허용하는가 (0.15.0 M13).

무엇을 재는가(C10 10.5 · O-C10-4 · RULINGS AT9):
  1  표가 없으면(데몬이 거부) 규칙 문서 Write/Edit 은 배포본 그대로 차단된다(같은 deny 문장).
  2  데몬이 허용(rule-doc-allowed 종료 코드 0)하면 그 Write/Edit 만 통과하고, 조회는
     `cys --socket <s> repair rule-doc-allowed --path <파일>` 하나이며 CYS_NO_AUTOSTART=1 로 간다.
  3  소켓 없음 · 옛 데몬(93) · 거부(95) · 시간 초과 → 오늘의 동작(차단). 시간 초과는 PATH 의 명령에 기대지 않고
     막혀 있는 스텁(셸 내장 `read` 가 아무도 쓰지 않는 FIFO 를 읽는다)으로 재며, 통과 조건은 넷 다다(RULINGS ER2):
     가드의 거부 문장 · 거부가 한도보다 **이르지 않음**(단조 시계 ≥) · 스텁이 기다리고 있었다는 증거(pid · waiting
     표지) · 그 프로세스가 끝났음. 막기가 즉시 실패하는 스텁에서는 이 판정이 **실패**해야 한다(회귀 3e).
  7  가드의 테스트 예산 손잡이가 실제로 쓰는 조회 한도(RULINGS ER1 · EM2 — 제품 변경 없이): 스텁이 **스스로
     지키는 하한 지연 D** 뒤에야 "허용"으로 답한다. 허용 = 한도 ≥ D(가드가 D 이상 기다렸다 · 단조 ≥ D),
     거부 = 스텁이 아직 기다리는 중에 끝났다는 증거 + 가드가 자기 한도 이상 기다린 뒤였다(단조 ≥). 다섯 설정:
     예산 5초(허용 D=1.5) · 상한 넘는 60초(허용 D=3 · 거부 D=21 — 상한 20초) · 작은 예산 0.4초(거부 D=0.55 —
     낮추기만) · 변수 없음(거부 D=0.9 — 기본 0.6초) · 테스트 모드 끔 + 변수 5초(거부 D=0.9 — 변수 무시).
  4  Bash 로 규칙 문서를 쓰는 경로(tee·sed -i·리다이렉트)는 표가 있어도 막힌다.
  5  corpus G 의 대표 호출(외부 발행·외부 전송·영구 삭제)은 표가 있든 없든 막힌다(AT9).
  6  규칙 문서가 아닌 쓰기는 조회 없이 통과한다(평시 동작 무변화).
밀폐: 가짜 `cys` 스텁(PATH 선두) · 임시 디렉터리의 진짜 unix 소켓 파일 · GUARD_TEST_MODE 로 자율주행 깃발을
임시 경로에 둔다 · HOME 임시. 데몬·라이브 팩 무접촉.
시간 관용도(RULINGS EH1 · EA2 · EM1 · ER3): 관용도를 키웠을 뿐이며 조회는 여전히 남은 예산 안에 끝나야 한다.
허용·거부 경우는 가드의 테스트 예산을 크게(`GUARD_TEST_GATE_BUDGET_MS=20000`) 주고, 시간 초과 경우는 5초 예산을
쓴다. 스텁이 그 안에 기동해 답하거나 증거를 남기지 못할 만큼 극단적인 부하에서는 테스트가 **크게 실패**하고
다시 돌린다 — 조용히 통과하지 않는다. 7 의 작은 예산(0.4초)·기본 한도(0.6초) 경우는 그 짧은 한도 안에 조회가
시작돼야 하므로 부하에 가장 민감하다(실패하면 증거 부족으로 크게 실패한다). `STUB_START_DELAY=<초>` 로 스텁의
느린 기동을 주입할 수 있다(부하 재현). 스텁의 지연은 절대경로 `/bin/sleep`(먼저 있는지 확인), 막기는 셸 내장
`read` — PATH 의 명령에 기대지 않는다.
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


STUB = r"""#!/bin/sh
# fake cys: logs argv + CYS_NO_AUTOSTART, answers rule-doc-allowed with $STUB_RULEDOC_RC
[ -n "${STUB_START_DELAY:-}" ] && /bin/sleep "$STUB_START_DELAY"   # injected slow start-up (load)
printf '%s|NO_AUTOSTART=%s\n' "$*" "${CYS_NO_AUTOSTART:-}" >> "$STUB_LOG"
case "$*" in
  *"repair rule-doc-allowed"*)
    [ -n "${STUB_EVIDENCE:-}" ] && echo $$ > "$STUB_EVIDENCE"      # evidence: the pid that waits
    if [ -n "${STUB_BLOCK_FIFO:-}" ]; then
      : > "$STUB_EVIDENCE.waiting"
      read _ < "$STUB_BLOCK_FIFO"   # shell builtin on a FIFO nobody writes: blocks until killed
      exit 97                       # reached only if the block failed at once: never an answer
    fi
    if [ -n "${STUB_ALLOW_AFTER:-}" ]; then
      : > "$STUB_EVIDENCE.waiting"
      /bin/sleep "$STUB_ALLOW_AFTER" || exit 98   # the lower bound D, enforced by the stub itself
      : > "$STUB_EVIDENCE.answered"
      exit 0                        # "allowed" — never before D
    fi
    exit "${STUB_RULEDOC_RC:-95}" ;;
esac
exit 1
"""

# RULINGS EH1 / ER3: the allow / deny cases get a large budget (increased tolerance — the query must
# still finish within the remaining budget); the guard's test knob raises its per-query cap with it,
# up to the guard's ceiling (20 s)
BIG_BUDGET_MS = "20000"
# the time-out case: generous enough for the stub's start-up (EM1), then the stub never answers
TIMEOUT_BUDGET_MS = "5000"
TIMEOUT_LIMIT_S = 5.0
DENY_SOUL = "헌법파일(soul.md) Write 차단"   # the guard's refusal on this path (it has no separate time-out wording)


def main():
    root = tempfile.mkdtemp(prefix="gtok-", dir="/tmp")
    srv = None
    try:
        stubdir = os.path.join(root, "bin")
        os.makedirs(stubdir)
        stub = os.path.join(stubdir, "cys")
        with open(stub, "w") as f:
            f.write(STUB)
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
        # evidence helpers (EM1 / ER1 / ER2)
        seq = [0]

        def ev_path():
            seq[0] += 1
            return os.path.join(root, "ev%d" % seq[0])

        def evidence(base):
            try:
                pid = int(open(base).read().strip())
            except (OSError, ValueError):
                pid = None
            return pid, os.path.exists(base + ".waiting"), os.path.exists(base + ".answered")

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

        # 3c — the daemon's answer never comes (RULINGS ER2 / EM1). Passes only with ALL of: the guard's
        # refusal sentence, the refusal NO EARLIER than the limit (monotonic >=), the stub's evidence
        # that it was waiting, and that process gone afterwards.
        def timeout_case(fifo):
            base = ev_path()
            p, calls, el = run({"file_path": soul, "content": "x"}, rc=0, budget_ms=TIMEOUT_BUDGET_MS,
                               extra={"STUB_EVIDENCE": base, "STUB_BLOCK_FIFO": fifo})
            pid, waiting, answered = evidence(base)
            ok = (p.returncode == 2 and DENY_SOUL in p.stderr and el >= TIMEOUT_LIMIT_S
                  and pid is not None and waiting and not answered and gone(pid)
                  and len(calls) == 1 and "repair rule-doc-allowed" in calls[0])
            return ok, pid, (p.returncode, round(el, 3), pid, waiting, answered, gone(pid), calls)

        fifo = os.path.join(root, "never-written.fifo")
        os.mkfifo(fifo)
        ok, pid, detail = timeout_case(fifo)
        check("3c 시간 초과 → 차단: 거부 문장 · %.1f초 이상 기다린 뒤 · 기다리던 스텁(pid=%s)의 증거 · 그 프로세스는 끝났다"
              % (TIMEOUT_LIMIT_S, pid), ok, "" if ok else repr(detail))
        # 3e — regression of the time-out judgment itself: a block that fails at once (its FIFO is
        # missing) must make the time-out case FAIL — refused early, nothing waited
        bad_ok, _, bad_detail = timeout_case(os.path.join(root, "missing.fifo"))
        check("3e 막기가 즉시 실패하는 스텁에서는 시간 초과 판정이 통과하지 않는다", not bad_ok, repr(bad_detail))

        # 7 — the knob's effective per-query limit, observed without a product change (RULINGS ER1): the
        # stub answers "allowed" only after a delay D it enforces itself. ALLOW proves limit >= D (and the
        # guard waited >= D); REFUSAL counts only with the stub's evidence that it was still waiting when
        # ended and the guard having waited at least its limit (monotonic >=).
        have_sleep = os.access("/bin/sleep", os.X_OK)
        check("7- 스텁의 하한 지연에 쓰는 /bin/sleep 이 있다(PATH 에 기대지 않는다)", have_sleep)

        def limit_case(label, budget_ms, d, expect, limit_s=None, test_mode=True):
            base = ev_path()
            p, _, el = run({"file_path": soul, "content": "x"}, rc=0, budget_ms=budget_ms, test_mode=test_mode,
                           extra={"STUB_EVIDENCE": base, "STUB_ALLOW_AFTER": "%g" % d})
            pid, waiting, answered = evidence(base)
            if expect == "allow":
                ok = p.returncode == 0 and answered and el >= d
            else:
                ok = (p.returncode == 2 and DENY_SOUL in p.stderr and pid is not None and waiting
                      and not answered and gone(pid) and el >= limit_s)
            check(label, ok, "" if ok else repr((p.returncode, round(el, 3), pid, waiting, answered)))

        if have_sleep:
            limit_case("7a 예산 5초 → D=1.5초 뒤의 허용을 기다렸다(한도가 예산을 따랐다 · ≥1.5초)", "5000", 1.5, "allow")
            limit_case("7b 예산 60초(상한 위) → D=3초 뒤의 허용을 기다렸다(1초 넘는 예산은 한도를 올린다)", "60000", 3.0, "allow")
            limit_case("7c 예산 60초 → D=21초를 기다리지 않고 20초 이상 기다린 뒤 거부(상한 20초 · 스텁은 기다리던 중)",
                       "60000", 21.0, "refuse", limit_s=20.0)
            limit_case("7d 작은 예산 0.4초 → D=0.55초 전에 거부(0.4초 이상 · 낮추기만 한다)", "400", 0.55, "refuse",
                       limit_s=0.4)
            limit_case("7e 변수 없음 → D=0.9초 전에 거부(기본 한도 0.6초 이상 기다린 뒤)", None, 0.9, "refuse",
                       limit_s=0.6)
            limit_case("7f 테스트 모드 끔 + 변수 5초 → D=0.9초 전에 거부(변수 무시 · 0.6초 이상 기다린 뒤)", "5000", 0.9,
                       "refuse", limit_s=0.6, test_mode=False)
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
