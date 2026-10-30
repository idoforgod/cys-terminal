#!/usr/bin/env python3
"""0.14.56 phoenix `plan` specimens — shared rig (not a test; the test_phoenix_plan_*.py files import it).

Why a rig: the plan reads the LIVE state dir when no socket is given (that is the only place the generation
snapshot is read, javis_phoenix._snapshot_roster_entries), and LIVE_STATE is computed from HOME (LOCALAPPDATA on
Windows) when the module is imported. So the rig points HOME and LOCALAPPDATA at a throw-away root BEFORE it
loads the module, asserts the module's LIVE_STATE is under that root, and only then writes world files there.
A fake daemon sits behind the module's own `cys()` seam (status --json, list); any other daemon call is
recorded and refused. Worlds follow the DESIGN-r2 rig (round/_evidence/0.14.56-PHX-PLAN/r2/rig/r2_rig.py).
"""
import argparse
import contextlib
import importlib.util
import io
import json
import os
import shutil
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
PH = os.path.normpath(os.path.join(HERE, "..", "javis_phoenix.py"))
PREFIX = "phoenix-plan/1 "
GEN = "20261028T000000Z"
CORRUPT = "{\"entries\": [ {\"role\": \"mas"     # a truncated write
ROLES = ["master", "worker-1", "worker-2"]


def surf(ref, role, exited=False, seat="occupied"):
    return {"surface_ref": ref, "role": role, "exited": exited, "agent_alive": not exited, "seat": seat}


# worker-2 is dead and has no surface at all (cold boot): only the files decide whether it is a target.
LIVE_W2_GONE = [surf("surface:1", "master"), surf("surface:2", "worker-1")]
LIVE_ALL = LIVE_W2_GONE + [surf("surface:3", "worker-2")]


def topo(roles, tombs=(), rev=1, marker=True):
    t = {"entries": [{"role": r, "agent": "claude", "session_id": "sid-" + r, "cwd": "/tmp"} for r in roles],
         "tombstones": list(tombs), "updated_at": 1}
    if marker:
        t["schema_version"] = 1
        t["tombstones_rev"] = rev
    return t


def desired(roles, tombs=(), rev=1, epoch="sa:1000", extra=None):
    d = {"roster": {r: {"role": r, "agent": "claude", "session_id": "sid-" + r} for r in roles},
         "tombstones": sorted(tombs), "tombstones_rev": rev, "daemon_epoch": epoch, "updated_at": 1}
    if extra:
        d.update(extra)
    return d


SELF_ID = {"build_id": "b1", "embedded_pack_hash": "h1", "protocol_version": "1"}

_FAKE_CLI = r'''#!%s -B
import json, os, sys
a = sys.argv[1:]
if a[:1] == ["--socket"]:
    a = a[2:]
with open(os.environ["FAKE_CYS_LOG"], "a") as f:
    f.write(json.dumps(a) + "\n")
with open(os.environ["FAKE_CYS_LOG"] + ".env", "a") as f:   # what phoenix's child saw (S5 R1-3 specimen)
    f.write(json.dumps(os.environ.get("PHOENIX_RESULT_FILE")) + "\n")
w = json.load(open(os.environ["FAKE_CYS_WORLD"]))
if a[:1] == ["phoenix-identity"]:
    if w.get("self") is None:
        sys.exit(2)
    print(json.dumps(w["self"])); sys.exit(0)
if a[:2] == ["status", "--json"]:
    print(json.dumps({"daemon": dict(w["daemon"], started_at=1000), "surfaces": w["surfaces"]})); sys.exit(0)
if a[:1] == ["list"]:
    sys.exit(0)
sys.stderr.write("fake cys: refused %%r\n" %% (a,)); sys.exit(2)
'''


class _R:
    def __init__(self, rc, out="", err=""):
        self.returncode, self.stdout, self.stderr = rc, out, err


def _write(path, value):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as f:
        f.write(value if isinstance(value, str) else json.dumps(value))


class Rig:
    def __init__(self, prefix="phx-plan-"):
        self.root = os.path.realpath(tempfile.mkdtemp(prefix=prefix))
        self.home = os.path.join(self.root, "home")
        os.makedirs(self.home)
        os.environ["HOME"] = self.home
        os.environ["LOCALAPPDATA"] = os.path.join(self.home, "AppData", "Local")
        os.environ.pop("PHOENIX_FORBID_LIVE", None)
        if os.name == "nt":
            self.state = os.path.join(os.environ["LOCALAPPDATA"], "cys")
        else:
            self.state = os.path.join(self.home, ".local", "state", "cys")
        self.phx = os.path.join(self.state, "phoenix")
        self.gen_root = os.path.join(self.home, ".cys", "state-generations")
        self.surfaces, self.calls, self.emits = [], [], []
        self.status_ok, self.list_out = True, ""
        self.reap_outcome = "deferred"

    def load(self, path=PH, name="javis_phoenix_plan_rig"):
        spec = importlib.util.spec_from_file_location(name, path)
        m = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(m)
        if os.path.realpath(m.LIVE_STATE) != os.path.realpath(self.state) or \
                not os.path.realpath(m.LIVE_STATE).startswith(self.root + os.sep):
            raise SystemExit("rig refused: module LIVE_STATE %r is not the rig state %r" % (m.LIVE_STATE, self.state))
        m.cys = self.fake_cys
        m._emit_evt = lambda evt_type, **fields: self.emits.append([evt_type, fields.get("summary", "")]) or True
        m._CYS_IDENTITY = "verified"      # in-process: main()'s identity contract is not run
        return m

    def world(self, desired=None, topo=None, snapshot=None, live=(), intents=None, files=None):
        for d in (self.state, self.gen_root):
            if os.path.realpath(d).startswith(self.root + os.sep):
                shutil.rmtree(d, ignore_errors=True)
        os.makedirs(self.phx)
        if topo is not None:
            _write(os.path.join(self.state, "topology.json"), topo)
        if snapshot is not None:
            _write(os.path.join(self.gen_root, GEN, "topology.json"), snapshot)
        if desired is not None:
            _write(os.path.join(self.phx, "desired_roster.json"), desired)
        if intents is not None:
            _write(os.path.join(self.phx, "tombstone-intents.jsonl"), intents)
        for rel, value in (files or {}).items():
            _write(os.path.join(self.state, rel), value)
        self.surfaces = [dict(s) for s in live]
        self.calls, self.emits = [], []
        self.status_ok, self.list_out = True, ""
        self.reap_outcome = "deferred"

    def fake_cys(self, *args, socket=None, timeout=25):
        a = [str(x) for x in args]
        self.calls.append(a)
        if a[:2] == ["status", "--json"]:
            if not self.status_ok:
                return _R(1, "", "rig: status refused")
            return _R(0, json.dumps({"daemon": {"started_at": 1000}, "surfaces": self.surfaces}))
        if a[:1] == ["list"]:
            return _R(0, self.list_out)
        if a[:1] == ["reap-due"]:
            if self.reap_outcome == "reaped":       # CENSUS W2: the daemon reaps the exited surface first
                self.surfaces = [s for s in self.surfaces if s.get("surface_ref") != a[1]]
            return _R(0, json.dumps({"surface_id": int(a[1].split(":")[1]), "outcome": self.reap_outcome}))
        return _R(2, "", "rig: unexpected cys call %r" % (a,))

    def plan(self, m, compare=None):
        """In-process `plan` → (exit code | "absent", parsed framed JSON | None, stdout text).
        "absent" = the module has no plan entry (the pin) — every caller's check then fails."""
        fn = getattr(m, "cmd_plan", None)
        if fn is None:
            return "absent", None, ""
        buf, code = io.StringIO(), None
        with contextlib.redirect_stdout(buf):
            try:
                fn(argparse.Namespace(socket=None, compare_plan=compare))
            except SystemExit as e:
                code = e.code
        text = buf.getvalue()
        framed = [ln for ln in text.split("\n") if ln.startswith(PREFIX)]
        data = json.loads(framed[0][len(PREFIX):]) if len(framed) == 1 else None
        return code, data, text

    # ── child-process runs through the real main(): a fake `cys` executable answers the identity probe and the
    #    status/list reads from a world file and logs every call (POSIX: it is a shebang script) ──
    def fake_cli(self):
        bindir = os.path.join(self.root, "bin")
        os.makedirs(bindir, exist_ok=True)
        fake = os.path.join(bindir, "cys")
        with open(fake, "w", encoding="utf-8") as f:
            f.write(_FAKE_CLI % sys.executable)
        os.chmod(fake, 0o755)
        return fake

    def run_cli(self, fake, args, world, extra_env=None, timeout=120, script=PH):
        """→ (rc, framed lines, parsed framed JSON | None, stdout, stderr, cys calls)."""
        import subprocess
        wpath = os.path.join(self.root, "world.json")
        with open(wpath, "w", encoding="utf-8") as f:
            json.dump(world, f)
        log = os.path.join(self.root, "cys-calls.log")
        if os.path.exists(log):
            os.remove(log)
        env = {"HOME": self.home, "PATH": "/usr/bin:/bin", "LANG": "en_US.UTF-8", "PHOENIX_CYS": fake,
               "FAKE_CYS_WORLD": wpath, "FAKE_CYS_LOG": log, "PHOENIX_IDENTITY_RETRY_TRIES": "0",
               "TMPDIR": os.path.join(self.root, "tmp")}
        os.makedirs(env["TMPDIR"], exist_ok=True)
        env.update(extra_env or {})
        p = subprocess.run([sys.executable, "-B", script] + list(args), env=env, stdout=subprocess.PIPE,
                           stderr=subprocess.PIPE, timeout=timeout)
        out = p.stdout.decode("utf-8", "replace")
        framed = [ln for ln in out.split("\n") if ln.startswith(PREFIX)]
        data = None
        if len(framed) == 1:
            try:
                data = json.loads(framed[0][len(PREFIX):])
            except ValueError:
                data = None
        calls = [json.loads(ln) for ln in open(log, encoding="utf-8")] if os.path.exists(log) else []
        return p.returncode, framed, data, out, p.stderr.decode("utf-8", "replace"), calls

    def save(self, name, text):
        p = os.path.join(self.root, "artifacts", name)
        _write(p, text)
        return p

    def close(self):
        shutil.rmtree(self.root, ignore_errors=True)


class Checks:
    def __init__(self):
        self.results = []

    def __call__(self, name, cond, detail=""):
        self.results.append(bool(cond))
        print(("PASS " if cond else "FAIL ") + name + (" | " + str(detail) if detail != "" else ""))
        return bool(cond)

    def finish(self):
        npass = sum(self.results)
        print("\n=== %d/%d PASS ===" % (npass, len(self.results)))
        return 0 if self.results and npass == len(self.results) else 1
