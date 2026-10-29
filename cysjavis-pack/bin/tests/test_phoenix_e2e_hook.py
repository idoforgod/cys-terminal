#!/usr/bin/env python3
"""0.14.56 PHX S6 — the pre-restore hook end to end on a REAL isolated cysd (DESIGN r1 §11 AT1-AT5, r2 §3 · §7).

Each case wipes the isolated harness state, writes (or not) the allow file under a rig CYS_LOCAL_DIR, boots a real cysd
with auto-restore enabled, waits until phoenix-restore.log shows the case's end marker, tears the daemon down, and reads
the log.
  at1_marker        allow file `default` line -> the user's script runs as the hook; its output is in the restore log
                    under `===== phoenix hook <path> sha256=<16> =====`, then `phoenix hook end: exit=0`.
  at2_unset         no allow file -> no hook line, no hook.run.lock (POSIX-meaningful: only unix makes that lock; at1
                    asserts it exists after a hook run), the authority ran (C1).
  at3_broken        allow file -> a script with a syntax error -> hook exit 1 -> the authority still runs after it.
  at4_lock_held     (POSIX) no allow file; this test holds restore.run.lock -> the authority steps back (LEASE_HELD) ->
                    footer -> after D the lock is still held -> the recheck stops (N8 path).
  at5_holder_killed (POSIX) the holder is a child that dies inside D -> the recheck plans (no N8 stop) and ends in a
                    gated Empty or exactly one more attempt (no role is seeded: a real harness would launch its agent).
Binaries: PHOENIX_HARNESS_CYSD / <repo>/target/debug / PATH (as test_phoenix_e2e_replacement.py). None found => SKIP (exit 0);
PHOENIX_HARNESS_CYSD set but missing => FAIL.
Rig hygiene: the harness lives under $HOME/.cys/state-harness, so HOME must be a scratch dir. A stub
$HOME/_round/SESSION_STATE.md is written so no upward search from the rig reaches a real lane's _round; if $HOME/_round
already exists and is not this test's stub, the test refuses instead of touching it: FAIL in gate mode
(PHOENIX_HARNESS_CYSD set by the caller), SKIP otherwise. A $HOME/_round this test
created is removed at exit (stub file, then the directory only if empty).

Run: python3 cysjavis-pack/bin/tests/test_phoenix_e2e_hook.py   (0 = all PASS or SKIP)
"""
import atexit
import hashlib
import importlib.util
import json
import os
import shutil
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.normpath(os.path.join(HERE, "..", "..", ".."))
DBG = os.path.join(REPO, "target", "debug")
STUB = "# rig stub (test_phoenix_e2e_hook.py) - not a lane state file\n"
EXE = ".exe" if os.name == "nt" else ""


def _find(name):
    env = os.environ.get("PHOENIX_HARNESS_" + name.upper())
    if env:
        if not os.path.exists(env):
            print("FAIL PHOENIX_HARNESS_%s=%s does not exist" % (name.upper(), env))
            sys.exit(1)
        return env
    cand = os.path.join(DBG, name + EXE)
    if os.path.exists(cand):
        return cand
    return shutil.which(name)


CYSD, CYS = _find("cysd"), _find("cys")
if not (CYSD and CYS):
    print("SKIP: cysd/cys not found (build first) - exit 0")
    sys.exit(0)

HOME = os.path.realpath(os.path.expanduser("~"))
ROUND = os.path.join(HOME, "_round")
STATE_STUB = os.path.join(ROUND, "SESSION_STATE.md")
if os.path.exists(ROUND) and not (os.path.isfile(STATE_STUB) and open(STATE_STUB, encoding="utf-8").read() == STUB):
    why = "%s exists and is not this test's stub - run with a scratch HOME (the glob runner sets one)" % ROUND
    if os.environ.get("PHOENIX_HARNESS_CYSD"):  # gate mode: a skip here would pass the gate without running it
        print("FAIL " + why)
        sys.exit(1)
    print("SKIP: " + why)
    sys.exit(0)
if not os.path.exists(ROUND):
    def _undo_round():
        for undo in (lambda: os.remove(STATE_STUB), lambda: os.rmdir(ROUND)):
            try:
                undo()
            except OSError:
                pass
    atexit.register(_undo_round)  # also on a module-level exit below (guard_isolation refusing)
os.makedirs(ROUND, exist_ok=True)
with open(STATE_STUB, "w", encoding="utf-8") as f:
    f.write(STUB)

os.environ["PHOENIX_HARNESS_CYSD"] = CYSD
os.environ["PATH"] = os.path.dirname(CYSD) + os.pathsep + os.environ.get("PATH", "")
os.environ["PHOENIX_CYS"] = CYS
os.environ["CYS_NO_AUTORESTORE"] = "0"                          # this test verifies the daemon's own auto-restore
os.environ["CYS_AUTORESTORE_LEASE_RECHECK_DELAY_MS"] = "5000"     # D at its production floor (clamp [5 000, 300 000])
for k in ("CYS_PHOENIX_SCRIPT", "CYS_PHOENIX_HOOK_STALL_S", "CYS_PHOENIX_HOOK_TIMEOUT_S", "PHOENIX_RESULT_FILE"):
    os.environ.pop(k, None)

spec = importlib.util.spec_from_file_location("h", os.path.join(HERE, "..", "javis_phoenix_harness.py"))
h = importlib.util.module_from_spec(spec)
spec.loader.exec_module(h)
h.CYS, h.CYSD = CYS, CYSD
h.guard_isolation()

LOG = os.path.join(h.HARN_DIR, "phoenix-restore.log")
PHX = os.path.join(h.HARN_DIR, "phoenix")
LOCAL = os.path.join(HOME, "e2e-hook-local")
SCRIPTS = os.path.join(HOME, "e2e-hook-scripts")
results = []


def check(name, cond, detail=""):
    results.append(bool(cond))
    print(("PASS " if cond else "FAIL ") + name + (" | " + str(detail)[:300] if detail else ""))


def wipe():
    h.teardown(verbose=False)
    shutil.rmtree(PHX, ignore_errors=True)
    shutil.rmtree(os.path.join(h.HARN_DIR, "phoenix-embed"), ignore_errors=True)
    for f in ("topology.json", "desired_roster.json", "phoenix-restore.log"):
        try:
            os.remove(os.path.join(h.HARN_DIR, f))
        except OSError:
            pass
    shutil.rmtree(LOCAL, ignore_errors=True)
    os.makedirs(LOCAL)
    os.environ["CYS_LOCAL_DIR"] = LOCAL


def allow(name, body):
    """Write the user's script and a `default` allow line for its exact bytes → (path, sha256)."""
    os.makedirs(SCRIPTS, exist_ok=True)
    p = os.path.join(SCRIPTS, name)
    with open(p, "wb") as f:
        f.write(body.encode("utf-8"))
    sha = hashlib.sha256(body.encode("utf-8")).hexdigest()
    with open(os.path.join(LOCAL, "phoenix-allow.sha256"), "w", encoding="utf-8") as f:
        f.write("default %s  %s\n" % (sha, p))
    return p, sha


def read_log():
    try:
        with open(LOG, encoding="utf-8", errors="replace") as f:
            return f.read()
    except OSError:
        return ""


def boot(done, wait=60.0, during=None):
    """Kill any harness daemon, boot a fresh one, poll the restore log until done(log) (the event), tear down.
    `during(log)` runs on every poll (at5 kills the holder there). The wait is a hang guard, not the verdict."""
    h.teardown(verbose=False)
    pid = h.start_daemon(wait=15.0)
    t0 = time.monotonic()
    body = ""
    while time.monotonic() - t0 < wait:  # wallclock: guard hang — every case ends on a log line the daemon writes
        time.sleep(0.25)
        body = read_log()
        if during:
            during(body)
        if done(body):
            break
    h.teardown(verbose=False)
    return pid, read_log()


def after(body, first, then):
    i = body.find(first)
    return i >= 0 and body.find(then, i + len(first)) >= 0


AUTH = "===== phoenix auto-restore @"
RESULT = '"phoenix_restore"'


def at1_marker():
    wipe()
    path, sha = allow("at1_hook.py", "print('AT1-HOOK-MARKER', flush=True)\n")
    header = "===== phoenix hook %s sha256=%s =====" % (path, sha[:16])
    end = "===== phoenix hook end: exit=0 ====="
    pid, body = boot(lambda b: end in b and ("내장 복원을 건너뛴다" in b or after(b, end, RESULT)))
    check("at1 daemon booted", pid is not None)
    check("at1 the hook ran under its header with its output in the restore log", after(body, header, "AT1-HOOK-MARKER"),
          body[-400:])
    check("at1 the hook ended exit 0 and a post-hook plan followed", after(body, end, "===== phoenix plan (post-hook)"), body[-300:])
    check("at1 the pre-hook plan carried W/X/U", "===== phoenix plan (pre-hook · wall=" in body)
    if os.name != "nt":  # the stdin lock carrier is unix-only (Windows hooks get a null stdin and no hook.run.lock)
        check("at1 the hook run left hook.run.lock where at2 looks (positive control for at2's absence check)",
              os.path.exists(os.path.join(PHX, "hook.run.lock")))


def at2_unset():
    wipe()
    pid, body = boot(lambda b: RESULT in b)
    check("at2 daemon booted", pid is not None)
    check("at2 no allow file -> no hook line", "phoenix hook" not in body, body[-300:])
    check("at2 no allow file -> no hook.run.lock", not os.path.exists(os.path.join(PHX, "hook.run.lock")))
    check("at2 the authority ran (result line present)", AUTH in body and RESULT in body, body[-300:])


def at3_broken():
    wipe()
    path, _sha = allow("at3_hook.py", "def (:\n")
    end = "===== phoenix hook end: exit=1 ====="
    pid, body = boot(lambda b: after(b, end, RESULT))
    check("at3 daemon booted", pid is not None)
    check("at3 the broken script ran as the hook and ended exit 1", end in body and "SyntaxError" in body, body[-400:])
    check("at3 the authority ran after it", after(body, end, AUTH) and after(body, end, RESULT), body[-300:])


def _hold(lockpath):
    """A child that holds restore.run.lock until killed; returns the Popen once it reports 'held'."""
    os.makedirs(os.path.dirname(lockpath), exist_ok=True)
    code = ("import fcntl, sys, time\nf = open(sys.argv[1], 'a+')\nfcntl.flock(f.fileno(), fcntl.LOCK_EX)\n"
            "print('held', flush=True)\ntime.sleep(300)\n")
    p = subprocess.Popen([sys.executable, "-I", "-B", "-c", code, lockpath, HOME], stdout=subprocess.PIPE,
                         stdin=subprocess.DEVNULL)
    line = p.stdout.readline().decode().strip()
    if line != "held":
        p.kill()
        p.wait()
        raise SystemExit("FAIL holder did not take the lock: %r" % line)
    return p


FOOTER = "LEASE_HELD: 다른 복원이 진행 중이라 이번 실행은 물러났습니다"


def at4_lock_held():
    wipe()
    holder = _hold(os.path.join(PHX, "restore.run.lock"))
    try:
        stop = "[cysd] recheck: restore lock held"
        pid, body = boot(lambda b: stop in b)
    finally:
        holder.kill()
        holder.wait()
    check("at4 daemon booted", pid is not None)
    check("at4 the authority stepped back and the footer says LEASE_HELD + exit=0", "exit=0 (" + FOOTER in body, body[-400:])
    check("at4 after D the lock was still held -> the recheck stopped (no plan, no second attempt)",
          after(body, FOOTER, "recheck: restore lock held") and "phoenix plan (recheck)" not in body, body[-300:])


def at5_holder_killed():
    """The holder dies inside D -> the recheck plans instead of stopping at N8. No dead role is seeded on purpose: in a
    real harness phoenix would launch that role's agent seat. "Plan Targets -> exactly one attempt" is the Rust specimen
    phoenix_s4_tests::lease_recheck_restores_after_holder_dies (fake phoenix)."""
    wipe()
    holder = _hold(os.path.join(PHX, "restore.run.lock"))
    state = {"killed": False}

    def kill_holder_on_footer(b):
        if not state["killed"] and FOOTER in b:
            holder.kill()
            holder.wait()
            state["killed"] = True

    recheck = "===== phoenix plan (recheck)"
    gated = "[cysd] recheck: 다른 복원이 끝났고 남은 대상이 없습니다"
    try:
        pid, body = boot(lambda b: gated in b or (after(b, recheck, AUTH) and after(b[b.find(recheck):], AUTH, RESULT)),
                         during=kill_holder_on_footer)
    finally:
        if not state["killed"]:
            holder.kill()
            holder.wait()
    tail = body[body.find(FOOTER):] if FOOTER in body else ""
    attempts = tail.count(AUTH)
    check("at5 daemon booted", pid is not None)
    check("at5 the first attempt stepped back (LEASE_HELD footer) and the holder was killed inside D",
          FOOTER in body and state["killed"], body[-300:])
    check("at5 the recheck planned (no N8 stop)", recheck in tail and "recheck: restore lock held" not in tail, tail[-300:])
    check("at5 then either a gated Empty stop with no attempt, or exactly one more attempt",
          (gated in tail and attempts == 0) or (gated not in tail and attempts == 1), "gated=%s attempts=%d" % (gated in tail, attempts))


def main():
    h.teardown(verbose=False)
    h._fresh_harness()
    at1_marker()
    at2_unset()
    at3_broken()
    if os.name == "nt":
        print("SKIP at4_lock_held / at5_holder_killed on Windows (POSIX flock holder) - covered by lease_recheck_tests")
    else:
        at4_lock_held()
        at5_holder_killed()
    n = sum(results)
    print("\n=== %d/%d PASS ===" % (n, len(results)))
    return 0 if n == len(results) else 1


if __name__ == "__main__":
    try:
        sys.exit(main())
    finally:
        try:
            h.teardown(verbose=False)
        except Exception:
            pass
        os.environ.pop("CYS_LOCAL_DIR", None)
        shutil.rmtree(LOCAL, ignore_errors=True)
        shutil.rmtree(SCRIPTS, ignore_errors=True)
