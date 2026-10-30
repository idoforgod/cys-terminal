#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_pkg3a_owned_tools.py - 0.14.56 PKG-3A hooks lane, tool minors (#23 #24 #25 #26 #27).

Issue numbers = verdict 0.14.56_PRETAG_cross-vendor_PKG-3A_pack_bin_hooks-r1-reviewer1.json; ruling HEAD-RULING-PKG3A-R1.md.
  #23 skillscan pool: a worker that never returns ends the wait at POOL_SCAN_DEADLINE_S (TimeoutError -> the
      existing bounded stop and serial fallback) instead of f.result() waiting forever.
  #24 the owned-run reason reaches the reader: checklist 'preflight: 실행불가 (<reason>)' and the C60 line.
  #25 actprobe falls back to a plain read-screen only on the CLI's own rejection of --composer (rc 2 +
      "unexpected argument '--composer'"); any other non-zero is _ReadError (INDET).
  #26 a missing _owned_process.py does not stop preflight / checklist / skillscan from starting (loud stderr
      line); C60 names a scanner that is not preflight's own sibling when its run ends incomplete (option A).
  #27 managed_main: an error raised by the tool's main() is the tool's own error (traceback, rc 1, DONE),
      not 'owned lifecycle unavailable'.
Hermetic: temporary packs and HOME, no daemon, stub cys. Output PASS/FAIL lines, exit 1 on any failure,
final token PKG3A-OWNED-TOOLS-OK. POSIX (killpg, sh stubs).
"""
import json
import os
import shutil
import signal
import subprocess
import sys
import tempfile

SELF = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.normpath(os.path.join(SELF, ".."))
PY = sys.executable
sys.path.insert(0, BIN)
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


def _env(home, **extra):
    env = {k: v for k, v in os.environ.items() if not k.startswith("CYS_") and k != "CLAUDE_CONFIG_DIR"}
    env.update({"HOME": home, "TMPDIR": home, "PYTHONDONTWRITEBYTECODE": "1"})
    env.update(extra)
    return env


def _run(cmd, env, timeout):
    """Own session so a hung child and its pool workers can be ended; (rc|'timeout', stdout, stderr)."""
    p = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env, start_new_session=True)
    try:
        out, err = p.communicate(timeout=timeout)
        return p.returncode, out.decode("utf-8", "replace"), err.decode("utf-8", "replace")
    except subprocess.TimeoutExpired:
        os.killpg(p.pid, signal.SIGKILL)
        out, err = p.communicate()
        return "timeout", out.decode("utf-8", "replace"), err.decode("utf-8", "replace")


def t23():
    tmp = tempfile.mkdtemp(prefix="pkg3a-23-")
    drv = os.path.join(tmp, "drv.py")
    _write(drv, "import os, sys, time\nsys.path.insert(0, %r)\nimport javis_skillscan as S\n"
                "def blocked(s):\n    time.sleep(3600)\n"
                "S.scan_skill = blocked\nS.POOL_SCAN_DEADLINE_S = 2.0\n"
                "if __name__ == '__main__':\n    t0 = time.monotonic()\n    try:\n"
                "        S._scan_with_pool(['a', 'b'])\n        print('RESULT returned')\n"
                "    except Exception as e:\n        print('RESULT %%s %%.1f' %% (type(e).__name__, time.monotonic() - t0))\n"
                "    sys.stdout.flush(); os._exit(0)\n" % BIN)
    rc, out, err = _run([PY, "-B", drv], _env(tmp), 45)
    line = [ln for ln in out.splitlines() if ln.startswith("RESULT")]
    ok = bool(line) and line[0].split()[1] == "TimeoutError" and float(line[0].split()[2]) < 20
    check("#23 a blocked pool worker ends the wait at the scan deadline (TimeoutError)", ok,
          "rc=%s out=%r err=%r" % (rc, out[-200:], err[-300:]))


def t24_checklist():
    import javis_checklist as C
    saved = C.run_preflight
    C.run_preflight = lambda cmd, timeout=0: (None, "incomplete: group absence not verified before cleanup deadline")
    try:
        line = C._preflight_line(["x"], 1)
    finally:
        C.run_preflight = saved
    check("#24 checklist line carries the owned-run reason", "group absence not verified" in line, repr(line))


def _c60(patch_body):
    tmp = tempfile.mkdtemp(prefix="pkg3a-c60-")
    pack, root = os.path.join(tmp, "pack"), os.path.join(tmp, "root")
    for d in (os.path.join(pack, "bin"), os.path.join(pack, "hooks"), os.path.join(pack, "skills"),
              os.path.join(root, "_round", "mcp_approved")):
        os.makedirs(d, exist_ok=True)
    _write(os.path.join(pack, "hooks", "inject-context.sh"), "# inject_gate.py\n")
    _write(os.path.join(pack, "hooks", "inject_gate.py"), "pass\n")
    _write(os.path.join(pack, "bin", "javis_memory.py"), "_skillscan = object()\n")
    _write(os.path.join(pack, "bin", "javis_skillscan.py"), "import sys\nsys.exit(0)\n", 0o755)
    drv = os.path.join(tmp, "drive.py")
    _write(drv, "import json, sys\nsys.path.insert(0, %r)\nimport javis_preflight as P\n%s\n"
                "p = P.Preflight(False, [], mode='report')\np.c60_gate_wiring()\n"
                "print(json.dumps(p.results, ensure_ascii=False, default=str))\n" % (BIN, patch_body))
    rc, out, err = _run([PY, "-B", drv], _env(tmp, CYS_PACK_DIR=pack, JAVIS_ROOT=root), 120)
    try:
        rows = json.loads(out.strip().split("\n")[-1])
        return [r for r in rows if r["id"] == "C60.gate-wiring"][0]["detail"]
    except Exception:
        return "UNREADABLE rc=%s out=%r err=%r" % (rc, out[-200:], err[-300:])


def t24_c60_and_26b():
    detail = _c60("P.run_owned = lambda *a, **k: {'complete': False, 'returncode': None, 'timed_out': False,\n"
                  "    'cleanup': 'unknown', 'reason': 'group absence not verified before cleanup deadline',\n"
                  "    'stdout': '', 'stderr': ''}")
    check("#24 C60 line carries the owned-run reason", "group absence not verified" in detail, detail[-300:])
    check("#26b C60 names a scanner that is not preflight's own sibling", "not preflight's own" in detail,
          detail[-300:])


def t25():
    import javis_actprobe as A
    tmp = tempfile.mkdtemp(prefix="pkg3a-25-")
    stub = os.path.join(tmp, "cys")
    saved = A.CYS_BIN
    A.CYS_BIN = stub
    try:
        _write(stub, "#!/bin/sh\ncase \"$*\" in *--composer*) echo 'daemon unreachable' >&2; exit 1 ;; esac\n"
                     "echo SCREEN-TEXT\n", 0o755)
        try:
            got = A._read_screen_live("surface:1")
            res = "returned %r" % (got,)
        except A._ReadError as e:
            res = "ReadError %s" % e
        check("#25 a non-parser failure of --composer is INDET, not a silent plain read",
              res.startswith("ReadError"), res)
        # Positive control: the CLI's own rejection (cys 0.14.54 text) still falls back to the plain read.
        _write(stub, "#!/bin/sh\ncase \"$*\" in *--composer*) printf \"error: unexpected argument '--composer' found\\n"
                     "\\nUsage: cys read-screen --surface <SURFACE>\\n\" >&2; exit 2 ;; esac\necho SCREEN-TEXT\n", 0o755)
        try:
            got = A._read_screen_live("surface:1")
        except A._ReadError as e:
            got = ("ReadError", str(e))
        check("#25 an old CLI that rejects --composer still falls back to the plain read",
              got == ("SCREEN-TEXT\n", None), repr(got))
    finally:
        A.CYS_BIN = saved


def t26a():
    tmp = tempfile.mkdtemp(prefix="pkg3a-26-")
    rig = os.path.join(tmp, "bin")
    shutil.copytree(BIN, rig, ignore=shutil.ignore_patterns("tests", "__pycache__", "_owned_process.py"))
    empty = os.path.join(tmp, "skills")
    os.makedirs(empty)
    env = _env(tmp, CYS_PACK_DIR=os.path.join(tmp, "pack"))
    for name, args in (("javis_preflight.py", ["--help"]), ("javis_skillscan.py", ["all", "--roots", empty, "--json"]),
                       ("javis_checklist.py", ["--state", os.path.join(tmp, "none.md")])):
        rc, out, err = _run([PY, "-B", os.path.join(rig, name)] + args, env, 120)
        check("#26a %s starts without _owned_process.py and says so" % name,
              rc in (0, 1) and "owned-process leaf unavailable" in err and "Traceback" not in err,
              "rc=%s err=%r" % (rc, err[-300:]))


def t27():
    from _owned_process import run_owned
    tmp = tempfile.mkdtemp(prefix="pkg3a-27-")
    child = os.path.join(tmp, "child.py")
    _write(child, "import sys\nsys.path.insert(0, %r)\nfrom _owned_process import managed_main\n"
                  "def main():\n    raise PermissionError('listdir denied')\n"
                  "sys.exit(managed_main(main))\n" % BIN)
    r = run_owned([PY, "-B", child], 30, force=2)
    check("#27 an error from main() is the tool's own (complete, rc 1, traceback), not a lifecycle failure",
          r["complete"] and r["returncode"] == 1 and "PermissionError" in r["stderr"]
          and "owned lifecycle unavailable" not in r["stderr"],
          "complete=%s rc=%s reason=%r err=%r" % (r["complete"], r["returncode"], r["reason"], r["stderr"][-200:]))


def main():
    for t in (t23, t24_checklist, t24_c60_and_26b, t25, t26a, t27):
        try:
            t()
        except Exception as e:
            check(t.__name__ + " ran", False, "%s: %s" % (type(e).__name__, e))
    if fails:
        print("FAILED: %d - %s" % (len(fails), ", ".join(fails)))
        return 1
    print("PKG3A-OWNED-TOOLS-OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
