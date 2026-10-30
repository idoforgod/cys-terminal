#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_inject_context_owned_cwd.py - 0.14.56 PKG-3A hooks lane (review r1 M1, #3, #4, #5, #7).

What it pins (issue numbers = verdict 0.14.56_PRETAG_cross-vendor_PKG-3A_pack_bin_hooks-r1-reviewer1.json):
  M1  A project directory is the SessionStart hook's working directory. A `python -c` puts '' (= that
      directory) first on sys.path, so a project file named like a stdlib module (queue.py, json.py,
      subprocess.py ...) either crashed the restore or ran project code inside the hook. Three -c sites:
        M1-1 the owned bootstrap in _lib.sh cys_timeout_run --owned (the leaf imports json/queue/...)
        M1-2 the ingress JSON parse in inject-context.sh --phase ingress (import json)
        M1-3 the legacy cys_timeout_run fallback in _lib.sh (import subprocess; macOS has no timeout(1))
      Each site is driven on its own, then the whole hook. Every shadow module writes a marker file and
      raises: a marker means project code ran; a missing restore means the hook was broken by it.
  #3  role phase fails before it emits -> an env lead seat (master) still gets the lead restore note.
  #4  ingress fault AFTER source/cwd were recorded -> restore continues with one warning line.
  #5  leaf cannot run (missing, or an interpreter that cannot start) -> degraded shell-only restore
      (soul headings + SESSION_STATE head) with one line saying it is degraded and unbounded;
      malformed hook JSON -> restore as source=startup with one warning line (v0.14.55 default).
  #7  a SIGTERM to the controller does not leave cys-ic-rec.* directories behind.
Faults are injected without touching product code: BASH_ENV is read by every non-interactive bash, so a
file that matches its own process arguments can end exactly one phase (`--phase role`).
Hermetic: stub ps/lsof/cys on PATH, temporary pack/home/TMPDIR, CYS_* and CLAUDE_CONFIG_DIR removed.
POSIX only (bash hook). Output: PASS/FAIL lines, exit 1 on any failure, final token
INJECT-CONTEXT-OWNED-CWD-OK. Run as CI does: CYS_PACK_DIR="$(mktemp -d)" python3 bin/tests/<this file>.
"""
import json
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import time

SELF = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.normpath(os.path.join(SELF, ".."))
HOOKS = os.path.normpath(os.path.join(SELF, "..", "..", "hooks"))
HOOK = os.path.join(HOOKS, "inject-context.sh")
LIB = os.path.join(HOOKS, "_lib.sh")
LEAF = os.path.join(BIN, "_owned_process.py")
BASH = shutil.which("bash") or "/bin/bash"
PY = sys.executable
# Stdlib names the hook's -c sites and the leaf import after start-up (runpy and its imports included).
SHADOWS = ("json", "queue", "runpy", "subprocess", "threading", "signal", "argparse", "ctypes",
           "pkgutil", "selectors", "tempfile", "shutil")
LEAD_CLEAR = "▶ 작업 계속(source=clear): 위 작업기억 이어서 진행."
fails = []


def check(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" - " + detail) if detail else ""))
    if not cond:
        fails.append(name)


def _write(path, body, mode=None):
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write(body)
    if mode:
        os.chmod(path, mode)


def fixture(tag, shadows=SHADOWS, leaf=True, chk_sleep=0):
    root = tempfile.mkdtemp(prefix="pkg3a-%s-" % tag)
    fx = {k: os.path.join(root, k) for k in ("stubbin", "home", "work", "ran", "tmpd", "pack")}
    fx["root"] = root
    for d in fx.values():
        os.makedirs(d, exist_ok=True)
    _write(os.path.join(fx["stubbin"], "ps"), "#!/bin/sh\necho '9100 claude /x/claude --x'\n", 0o755)
    _write(os.path.join(fx["stubbin"], "lsof"), "#!/bin/sh\nprintf 'n%s\\n'\n" % fx["work"], 0o755)
    _write(os.path.join(fx["stubbin"], "cys"),
           "#!/bin/sh\ncase \"$1\" in surface-role) exit 3 ;; esac\nexit 0\n", 0o755)
    _write(os.path.join(fx["pack"], "soul.md"), "# soul\n## SOUL-HEAD-A\nbody\n## SOUL-HEAD-B\n")
    os.makedirs(os.path.join(fx["work"], "_round"))
    _write(os.path.join(fx["work"], "_round", "SESSION_STATE.md"), "# S\nHUB-SS-MARKER\n")
    for name in shadows:
        _write(os.path.join(fx["work"], name + ".py"),
               "import os\nopen(os.path.join(%r, %r), 'w').close()\n"
               "raise ImportError('project file %s.py shadowed the stdlib')\n" % (fx["ran"], name, name))
    if shadows:
        os.makedirs(os.path.join(fx["work"], "multiprocessing"))
        _write(os.path.join(fx["work"], "multiprocessing", "__init__.py"),
               "import os\nopen(os.path.join(%r, 'multiprocessing'), 'w').close()\nraise ImportError('shadow')\n"
               % fx["ran"])
    if leaf:
        os.makedirs(os.path.join(fx["pack"], "bin"))
        fx["chk_start"] = os.path.join(root, "chk.started")
        _write(os.path.join(fx["pack"], "bin", "javis_checklist.py"),
               "import sys, time\nsys.path.insert(0, %r)\nfrom _owned_process import managed_main\n"
               "def main():\n    open(%r, 'w').close()\n    time.sleep(%r)\n    print('CHECKLIST-RAN')\n    return 0\n"
               "sys.exit(managed_main(main))\n" % (BIN, fx["chk_start"], chk_sleep))
        fx["hook"] = HOOK
    else:
        # A pack whose hooks have no owned leaf beside them and none under CYS_PACK_DIR/bin either.
        os.makedirs(os.path.join(fx["pack"], "hooks"))
        for name in ("inject-context.sh", "_lib.sh", "inject_gate.py"):
            shutil.copy2(os.path.join(HOOKS, name), os.path.join(fx["pack"], "hooks", name))
        fx["hook"] = os.path.join(fx["pack"], "hooks", "inject-context.sh")
    return fx


def env_for(fx, role=None, extra=None, path=None):
    env = {k: v for k, v in os.environ.items()
           if not k.startswith("CYS_") and k not in ("CLAUDE_CONFIG_DIR", "BASH_ENV", "PYTHONPATH",
                                                     "PYTHONSAFEPATH", "PYTHONHOME", "_CYS_LIB_DEFINE_ONLY")}
    env.update({"PATH": path or (fx["stubbin"] + os.pathsep + "/usr/bin" + os.pathsep + "/bin"),
                "CYS_PACK_DIR": fx["pack"], "CYS_ROOT": fx["root"], "HOME": fx["home"],
                "TMPDIR": fx["tmpd"], "CYS_SURFACE_ID": "7", "CYS_PY": PY})
    if role:
        env["CYS_ROLE"] = role
    env.update(extra or {})
    return env


def run_hook(fx, source="startup", role=None, payload=None, extra=None, timeout=150):
    if payload is None:
        payload = json.dumps({"source": source, "cwd": fx["work"]})
    return subprocess.run([BASH, fx["hook"]], input=payload, cwd=fx["work"], capture_output=True, text=True,
                          encoding="utf-8", errors="replace", env=env_for(fx, role, extra), timeout=timeout)


def ran(fx):
    return sorted(os.listdir(fx["ran"]))


def records(raw):
    fields = raw.split(b"\0")
    return {fields[i].decode("utf-8", "replace"): fields[i + 1].decode("utf-8", "replace")
            for i in range(0, len(fields) - 1, 2)}


def bash_env(fx, body):
    path = os.path.join(fx["root"], "bashenv.sh")
    _write(path, body)
    return {"BASH_ENV": path}


# BASH_ENV is read before the positional parameters are set (measured, bash 3.2 and later), so the phase is
# taken from the process arguments; /bin/ps because PATH starts with the stub ps.
PHASE_IS = 'case "$(/bin/ps -o args= -p $$)" in *" --phase %s")'


def leftovers(fx):
    return sorted(n for n in os.listdir(fx["tmpd"]) if n.startswith("cys-ic-rec"))


def main():
    # ---- M1-1: the owned bootstrap itself (cys_timeout_run --owned) ----
    fx = fixture("m1a")
    r = subprocess.run([BASH, "-c", '. "$1"; cys_timeout_run --owned 30 "$2" --records -- /bin/echo LEAF-OK',
                        "x", LIB, LEAF], cwd=fx["work"], capture_output=True,
                       env=env_for(fx, extra={"_CYS_LIB_DEFINE_ONLY": "1"}), timeout=60)
    check("M1-1 owned bootstrap: no project module ran", ran(fx) == [], "ran=%s" % ran(fx))
    check("M1-1 owned bootstrap: the child ran and completed",
          b"LEAF-OK" in r.stdout and records(r.stdout.split(b"LEAF-OK\n", 1)[-1]).get("_OWNED_RETURNED") == "1",
          "rc=%s err=%r" % (r.returncode, r.stderr[-300:]))

    # ---- M1-2: the ingress JSON parse (--phase ingress) ----
    fx = fixture("m1b")
    r = subprocess.run([BASH, HOOK, "--phase", "ingress"], cwd=fx["work"], capture_output=True,
                       env=env_for(fx, extra={"INPUT": json.dumps({"source": "clear", "cwd": fx["work"]})}),
                       timeout=60)
    rec = records(r.stdout)
    check("M1-2 ingress parse: no project module ran", ran(fx) == [], "ran=%s" % ran(fx))
    check("M1-2 ingress parse: source and cwd parsed", rec.get("SOURCE") == "clear" and rec.get("CWD") == fx["work"],
          "rc=%s rec=%s" % (r.returncode, {k: rec.get(k) for k in ("SOURCE", "CWD")}))

    # ---- M1-3: the legacy fallback (no timeout/gtimeout on PATH: the CYS_PY branch) ----
    fx = fixture("m1c")
    no_timeout = all(shutil.which(t, path=fx["stubbin"]) is None for t in ("timeout", "gtimeout"))
    r = subprocess.run([BASH, "-c", '. "$1"; cys_timeout_run 20 /bin/echo LEGACY-OK', "x", LIB],
                       cwd=fx["work"], capture_output=True, text=True,
                       env=env_for(fx, extra={"_CYS_LIB_DEFINE_ONLY": "1"}, path=fx["stubbin"]), timeout=60)
    check("M1-3 legacy fallback: PATH has no timeout/gtimeout (the -c branch is the one measured)", no_timeout)
    check("M1-3 legacy fallback: no project module ran", ran(fx) == [], "ran=%s" % ran(fx))
    check("M1-3 legacy fallback: command ran, rc 0", r.returncode == 0 and "LEGACY-OK" in r.stdout,
          "rc=%s out=%r err=%r" % (r.returncode, r.stdout[-200:], r.stderr[-300:]))

    # ---- M1 whole hook: unknown role (role lookup goes through cys_timeout_run) and master on clear ----
    fx = fixture("m1d")
    r = run_hook(fx, "startup")
    check("M1 hook startup/unknown: no project module ran", ran(fx) == [], "ran=%s" % ran(fx))
    check("M1 hook startup/unknown: soul headings and state restored",
          "SOUL-HEAD-A" in r.stdout and "HUB-SS-MARKER" in r.stdout and "restoration input incomplete" not in r.stdout,
          "rc=%s out=%r" % (r.returncode, r.stdout[-400:]))
    fx = fixture("m1e", shadows=("queue", "json"))
    r = run_hook(fx, "clear", role="master")
    check("M1 hook clear/master (queue.py + json.py): no project module ran", ran(fx) == [], "ran=%s" % ran(fx))
    check("M1 hook clear/master: state and lead note restored",
          "HUB-SS-MARKER" in r.stdout and LEAD_CLEAR in r.stdout and "CHECKLIST-RAN" in r.stdout,
          "rc=%s out=%r" % (r.returncode, r.stdout[-400:]))

    # ---- #3: role phase ends before it emits; env lead keeps the lead note ----
    fx = fixture("i3", shadows=())
    r = run_hook(fx, "clear", role="master",
                 extra=bash_env(fx, PHASE_IS % "role" + ' exit 7 ;; esac\n'))
    check("#3 fault injected (role phase reported incomplete)", "restoration phase role incomplete" in r.stdout,
          "out=%r" % r.stdout[-300:])
    check("#3 env lead seat gets the lead note, not the start-gate note",
          LEAD_CLEAR in r.stdout and "착수 게이트" not in r.stdout, "out=%r" % r.stdout[-400:])

    # ---- #4: ingress fault after SOURCE/CWD were recorded ----
    fx = fixture("i4", shadows=())
    r = run_hook(fx, "startup", role="master",
                 extra=bash_env(fx, PHASE_IS % "ingress" + ' trap "exit 9" EXIT ;; esac\n'))
    check("#4 restore continues after an ingress fault that came after SOURCE",
          "SOUL-HEAD-A" in r.stdout and "HUB-SS-MARKER" in r.stdout and "restoration input incomplete" not in r.stdout,
          "out=%r" % r.stdout[-400:])
    check("#4 the fault is one warning line", r.stdout.count("ingress fault after source") == 1,
          "out=%r" % r.stdout[-400:])

    # ---- #5: leaf cannot run ----
    fx = fixture("i5a", shadows=(), leaf=False)
    r = run_hook(fx, "startup", role="master")
    out = r.stdout
    check("#5a leaf missing: degraded restore says degraded and unbounded in one line",
          sum(1 for ln in out.splitlines() if "degraded" in ln and "unbounded" in ln) == 1, "out=%r" % out[-500:])
    check("#5a leaf missing: soul headings and state head restored",
          "SOUL-HEAD-A" in out and "HUB-SS-MARKER" in out and "restoration input incomplete" not in out,
          "out=%r" % out[-500:])
    fx = fixture("i5b", shadows=())
    fake = os.path.join(fx["root"], "fakepy")
    _write(fake, "#!/bin/sh\nexit 1\n", 0o755)
    r = run_hook(fx, "clear", role="master", extra={"CYS_PY": fake})
    out = r.stdout
    check("#5b interpreter cannot start: degraded restore (one line, state head, lead note)",
          sum(1 for ln in out.splitlines() if "degraded" in ln and "unbounded" in ln) == 1
          and "HUB-SS-MARKER" in out and LEAD_CLEAR in out, "out=%r" % out[-500:])
    fx = fixture("i5d", shadows=(), leaf=False)
    dept = os.path.join(fx["root"], "pack-dept-t1")
    os.makedirs(os.path.join(dept, "round"))
    _write(os.path.join(dept, "round", "SESSION_STATE.md"), "# S\nDEPT-SS-MARKER\n다음 액션: r09 착수\n")
    r = run_hook(fx, "clear", role="worker-1", extra={"CYS_PACK_DIR": dept})
    check("#5d degraded restore keeps U13: a dept member seat gets the pointer, not the head's SESSION_STATE",
          "DEPT-SS-MARKER" not in r.stdout and "master 소관" in r.stdout and "착수 게이트" in r.stdout
          and "degraded" in r.stdout, "out=%r" % r.stdout[-500:])
    fx = fixture("i5c", shadows=())
    r = run_hook(fx, payload='{"source": "clear", ', role="master")
    check("#5c malformed hook JSON: restored as startup with one warning line",
          "SOUL-HEAD-A" in r.stdout and r.stdout.count("not valid JSON") == 1
          and "restoration input incomplete" not in r.stdout, "out=%r" % r.stdout[-400:])

    # ---- #7: SIGTERM to the controller while a phase runs ----
    fx = fixture("i7", shadows=(), chk_sleep=4)
    p = subprocess.Popen([BASH, fx["hook"]], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                         cwd=fx["work"], env=env_for(fx, "master"), start_new_session=True)
    p.stdin.write(json.dumps({"source": "clear", "cwd": fx["work"]}).encode())
    p.stdin.close()
    deadline = time.time() + 90
    while time.time() < deadline and not os.path.exists(fx["chk_start"]):
        time.sleep(0.05)
    started = os.path.exists(fx["chk_start"])
    os.kill(p.pid, signal.SIGTERM)
    try:
        p.wait(timeout=60)
    except subprocess.TimeoutExpired:
        pass
    time.sleep(6)   # let any orphaned leaf finish its phase before looking
    try:
        os.killpg(p.pid, signal.SIGKILL)
    except OSError:
        pass
    p.stdout.close(); p.stderr.close()
    check("#7 SIGTERM reached the controller during the checklist phase", started)
    check("#7 no cys-ic-rec directory left behind", leftovers(fx) == [], "left=%s" % leftovers(fx))
    fx = fixture("i7n", shadows=())
    run_hook(fx, "clear", role="master")
    check("#7 normal run leaves no cys-ic-rec directory", leftovers(fx) == [], "left=%s" % leftovers(fx))

    if fails:
        print("FAILED: %d - %s" % (len(fails), ", ".join(fails)))
        return 1
    print("INJECT-CONTEXT-OWNED-CWD-OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
