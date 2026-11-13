#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_guard_owner_token.py — guard.sh 가 규칙 문서 쓰기를 **주인 지시 표**로만 더 허용하는가 (0.15.0 M13).

무엇을 재는가(C10 10.5 · O-C10-4 · RULINGS AT9):
  1  표가 없으면(데몬이 거부) 규칙 문서 Write/Edit 은 배포본 그대로 차단된다(같은 deny 문장).
  2  데몬이 허용(rule-doc-allowed 종료 코드 0)하면 그 Write/Edit 만 통과하고, 조회는
     `cys --socket <s> repair rule-doc-allowed --path <파일>` 하나이며 CYS_NO_AUTOSTART=1 로 간다.
  3  소켓 없음 · 옛 데몬(93) · 거부(95) · 시간 초과 → 오늘의 동작(차단). 시간 초과도 합산 예산 안에서 끝난다.
  4  Bash 로 규칙 문서를 쓰는 경로(tee·sed -i·리다이렉트)는 표가 있어도 막힌다.
  5  corpus G 의 대표 호출(외부 발행·외부 전송·영구 삭제)은 표가 있든 없든 막힌다(AT9).
  6  규칙 문서가 아닌 쓰기는 조회 없이 통과한다(평시 동작 무변화).
밀폐: 가짜 `cys` 스텁(PATH 선두) · 임시 디렉터리의 진짜 unix 소켓 파일 · GUARD_TEST_MODE 로 자율주행 깃발을
임시 경로에 둔다 · HOME 임시. 데몬·라이브 팩 무접촉.
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
printf '%s|NO_AUTOSTART=%s\n' "$*" "${CYS_NO_AUTOSTART:-}" >> "$STUB_LOG"
case "$*" in
  *"repair rule-doc-allowed"*)
    [ -n "${STUB_SLEEP:-}" ] && exec sleep "$STUB_SLEEP"
    exit "${STUB_RULEDOC_RC:-95}" ;;
esac
exit 1
"""


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

        def run(tool_input, tool="Write", rc=None, sock_path=sock, sleep=None, active=False):
            env = {k: v for k, v in os.environ.items()
                   if not k.startswith(("CYS_", "JAVIS_", "AITERM_", "AUTOPILOT_", "GUARD_"))}
            env.update({
                "PATH": stubdir + os.pathsep + env.get("PATH", "/usr/bin:/bin"),
                "HOME": home,
                "CYS_SOCKET": sock_path,
                "GUARD_TEST_MODE": "1",
                "AUTOPILOT_PAUSED_FILE": os.path.join(flags, "AUTOPILOT_PAUSED"),
                "AUTOPILOT_ACTIVE_FILE": os.path.join(flags, "AUTOPILOT_ACTIVE"),
                "STUB_LOG": log,
            })
            if rc is not None:
                env["STUB_RULEDOC_RC"] = str(rc)
            if sleep is not None:
                env["STUB_SLEEP"] = str(sleep)
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
        p, _, el = run({"file_path": soul, "content": "x"}, rc=0, sleep=5)
        check("3c 시간 초과 → 차단(합산 예산 안)", p.returncode == 2 and el < 4.5, "%.2fs rc=%d" % (el, p.returncode))
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
