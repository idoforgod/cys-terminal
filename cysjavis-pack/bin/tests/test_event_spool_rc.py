#!/usr/bin/env python3
"""javis_event.py: the spool path and the failure contract of `emit --spool` (0.14.56 EMIT-FORK E4 + X-8 CARRIER).

E4: at 9da2418d `cmd_emit` caught the OSError of `_spool_append` and returned EXIT_OK, so every caller that reads the
exit code of a `--spool` emit (javis_report_gate.emit, javis_approval_queue._emit) saw a delivery that never
happened. The wire line is still printed (the spool stays a best-effort carrier); only the exit code changes.

X-8 (invariant A, decisions Q1 = A1 and Q3): the spool path is javis_spool_path.spool_path_for_pack(<the pack of
javis_event.py>) = <pack>/state/evt_spool.jsonl - a function of the pack directory ONLY. HUD_STATE_DIR no longer moves
it. The leaf is loaded lazily and by explicit path (a bare import fails under `python -I`); a missing or broken leaf is
a spool failure (wire line printed, exit 5, `spool leaf unavailable: <type>`), never a traceback. `spool-path` prints
exactly that path and creates nothing.

Every specimen runs from a RIG PACK: a temp dir holding a copy of this bin/ (cp -R), so the producer's own pack is the
rig and the spool lands in <rig>/state/evt_spool.jsonl through the same function. The E4 fault stays portable (Windows
too): a REGULAR FILE at <rig>/state makes creating the spool directory fail. Positive controls: a writable rig gives
exit 0 and exactly one spool line; the bare import succeeds without -I; a module loaded by path WITHOUT the seal does
write __pycache__ (an interpreter that never writes bytecode makes the no-bytecode check a skip that says UNMEASURED,
never a pass).
"""
import importlib.util
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest

sys.dont_write_bytecode = True  # this test loads bin modules in-process: it writes no bytecode into the repo bin

HERE = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(HERE)
PACK = os.path.dirname(BIN)
LEAF = "javis_spool_path.py"
WIRE = b"[EVT v2] task.unblocked"
sys.path.insert(0, BIN)
import javis_event as E  # noqa: E402


def _make_rig(parent):
    """A RIG PACK under `parent`: <parent>/pack/bin is a copy of this bin/. Returns the rig's pack dir."""
    rig = os.path.join(parent, "pack")
    shutil.copytree(BIN, os.path.join(rig, "bin"), ignore=shutil.ignore_patterns("__pycache__"))
    return rig


def _rig_spool(pack):
    return os.path.join(pack, "state", "evt_spool.jsonl")  # the contract, written out (not the leaf's own value)


def _env(**extra):
    env = {k: v for k, v in os.environ.items() if k != "HUD_STATE_DIR"}
    env.update(extra)
    return env


def _run(pack, *args, env=None, seal_flag=True):
    """javis_event.py of `pack` under -I (and -B unless seal_flag is False). Bytes out (stdout is compared exactly)."""
    argv = [sys.executable, "-I"] + (["-B"] if seal_flag else []) + [os.path.join(pack, "bin", "javis_event.py")]
    return subprocess.run(argv + list(args), capture_output=True, timeout=30, env=env if env is not None else _env())


def _emit(pack, spool=True, **kw):
    args = ["emit", "task.unblocked", "--payload", '{"task":"T"}'] + (["--spool"] if spool else [])
    return _run(pack, *args, **kw)


def _txt(b):
    return b.decode("utf-8", "replace")


def _load_leaf(bin_dir):
    """The leaf loaded by explicit path - the same way its callers load it."""
    spec = importlib.util.spec_from_file_location("spool_leaf_under_test", os.path.join(bin_dir, LEAF))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def _lines(path):
    with open(path, encoding="utf-8") as f:
        return [json.loads(x) for x in f.read().splitlines() if x.strip()]


def _tree(root):
    out = {}
    for dp, dns, fns in os.walk(root):
        for n in dns + fns:
            p = os.path.join(dp, n)
            out[os.path.relpath(p, root)] = os.path.getsize(p) if os.path.isfile(p) else "dir"
    return out


def _pycache_dirs(root):
    return sorted(os.path.relpath(dp, root) for dp, dns, _f in os.walk(root) if os.path.basename(dp) == "__pycache__")


class RigCase(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="evt-spool-rc-")
        self.pack = _make_rig(self.tmp)
        self.rig_bin = os.path.join(self.pack, "bin")

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def block_state_dir(self):
        """The E4 fault: a regular file where the spool directory <rig>/state must be created."""
        with open(os.path.join(self.pack, "state"), "w", encoding="utf-8") as f:
            f.write("x")


class SpoolFailureIsNotSuccess(RigCase):
    def test_failed_spool_append_returns_spool_failed(self):
        self.block_state_dir()
        before = _tree(self.tmp)
        r = _emit(self.pack)
        # the fault bites: nothing was written anywhere under the root
        self.assertEqual(_tree(self.tmp), before, "the injected fault did not prevent the spool write")
        self.assertEqual(r.returncode, E.EXIT_SPOOL_FAILED, (r.returncode, _txt(r.stderr)))
        self.assertIn("spool append failed", _txt(r.stderr))
        # the wire line is still emitted: the spool is a carrier, not a gate on the wire
        self.assertIn(WIRE, r.stdout)

    def test_writable_spool_returns_ok_and_writes_one_line(self):  # positive control
        r = _emit(self.pack)
        self.assertEqual(r.returncode, E.EXIT_OK, _txt(r.stderr))
        lines = _lines(_rig_spool(self.pack))
        self.assertEqual(len(lines), 1)
        self.assertEqual(lines[0]["type"], "task.unblocked")

    def test_without_spool_flag_the_blocked_dir_is_irrelevant(self):
        self.block_state_dir()
        r = _emit(self.pack, spool=False)
        self.assertEqual(r.returncode, E.EXIT_OK, _txt(r.stderr))

    def test_exit_code_is_distinct(self):
        codes = {E.EXIT_OK, E.EXIT_USAGE, E.EXIT_INVALID, E.EXIT_DEGRADED}
        self.assertNotIn(E.EXIT_SPOOL_FAILED, codes)


class SpoolPathIsAFunctionOfThePack(RigCase):
    """X-8 (a), (b): `spool-path` == the leaf's value for that pack; no environment variable moves the spool."""

    def _assert_prints_leaf_value(self, pack):
        expected = os.path.join(pack, "state", "evt_spool.jsonl")
        existed = os.path.lexists(expected), os.path.lexists(os.path.dirname(expected))
        r = _run(pack, "spool-path")
        self.assertEqual(r.returncode, E.EXIT_OK, (r.returncode, _txt(r.stderr)))
        self.assertEqual(r.stderr, b"")
        leaf_value = _load_leaf(os.path.join(pack, "bin")).spool_path_for_pack(pack)
        self.assertEqual(leaf_value, expected)
        # exactly one line, exactly the leaf's value (UTF-8 bytes, the reader's decode is utf-8 + surrogateescape)
        self.assertEqual(r.stdout, leaf_value.encode("utf-8", "surrogateescape") + b"\n")
        # read-only: neither the spool nor its directory came into being
        self.assertEqual((os.path.lexists(expected), os.path.lexists(os.path.dirname(expected))), existed)

    def test_spool_path_prints_the_leaf_value_for_the_repo_pack(self):
        self._assert_prints_leaf_value(PACK)

    def test_spool_path_prints_the_leaf_value_for_a_rig_pack(self):
        self._assert_prints_leaf_value(self.pack)
        self.assertFalse(os.path.lexists(os.path.join(self.pack, "state")), "spool-path created the state dir")

    def test_hud_state_dir_does_not_move_the_spool(self):
        elsewhere = os.path.join(self.tmp, "elsewhere")
        os.makedirs(elsewhere)
        env = _env(HUD_STATE_DIR=elsewhere)
        r = _emit(self.pack, env=env)
        self.assertEqual(r.returncode, E.EXIT_OK, _txt(r.stderr))
        self.assertTrue(os.path.isfile(_rig_spool(self.pack)),
                        "HUD_STATE_DIR moved the spool out of <rig>/state (found in elsewhere: %s)" % os.listdir(elsewhere))
        self.assertEqual([x["type"] for x in _lines(_rig_spool(self.pack))], ["task.unblocked"])
        self.assertEqual(os.listdir(elsewhere), [], "the producer wrote into HUD_STATE_DIR")
        r = _run(self.pack, "spool-path", env=env)
        self.assertEqual(r.returncode, E.EXIT_OK, _txt(r.stderr))
        self.assertEqual(r.stdout, _rig_spool(self.pack).encode("utf-8", "surrogateescape") + b"\n")


class LeafFailureIsASpoolFailure(RigCase):
    """X-8 (c): a missing or broken leaf -> wire line, exit 5, `spool leaf unavailable: <type>`; nothing created."""

    def test_missing_leaf_gives_wire_line_and_exit_5(self):
        leaf = os.path.join(self.rig_bin, LEAF)
        had_leaf = os.path.isfile(leaf)
        if had_leaf:
            os.remove(leaf)
        r = _emit(self.pack)
        self.assertEqual(r.returncode, E.EXIT_SPOOL_FAILED, (r.returncode, _txt(r.stderr)))
        self.assertIn(WIRE, r.stdout)
        self.assertIn("spool leaf unavailable: FileNotFoundError", _txt(r.stderr))
        self.assertNotIn("Traceback", _txt(r.stderr))
        self.assertFalse(os.path.lexists(os.path.join(self.pack, "state")), "a failed leaf still created <rig>/state")
        r = _run(self.pack, "spool-path")
        self.assertEqual(r.returncode, E.EXIT_SPOOL_FAILED, (r.returncode, _txt(r.stderr)))
        self.assertEqual(r.stdout, b"")
        self.assertEqual(_txt(r.stderr).strip(), "spool leaf unavailable: FileNotFoundError")
        self.assertFalse(os.path.lexists(os.path.join(self.pack, "state")), "spool-path created <rig>/state")
        self.assertTrue(had_leaf, "the rig had no %s to remove - the rig is not a copy of this bin/" % LEAF)

    def test_broken_leaf_gives_wire_line_and_exit_5(self):
        with open(os.path.join(self.rig_bin, LEAF), "w", encoding="utf-8") as f:
            f.write('raise RuntimeError("broken leaf")\n')
        r = _emit(self.pack)
        self.assertEqual(r.returncode, E.EXIT_SPOOL_FAILED, (r.returncode, _txt(r.stderr)))
        self.assertIn(WIRE, r.stdout)
        self.assertIn("spool leaf unavailable: RuntimeError", _txt(r.stderr))
        self.assertNotIn("Traceback", _txt(r.stderr))
        r = _run(self.pack, "spool-path")
        self.assertEqual((r.returncode, r.stdout), (E.EXIT_SPOOL_FAILED, b""), _txt(r.stderr))
        self.assertEqual(_txt(r.stderr).strip(), "spool leaf unavailable: RuntimeError")


PROBE = r'''
import importlib.util, json, os
try:
    import javis_spool_path  # noqa: F401 - the forbidden bare import
    bare = "imported"
except ImportError as e:
    bare = type(e).__name__
spec = importlib.util.spec_from_file_location(
    "leaf", os.path.join(os.path.dirname(os.path.abspath(__file__)), "javis_spool_path.py"))
leaf = importlib.util.module_from_spec(spec)
spec.loader.exec_module(leaf)
print(json.dumps({"bare": bare, "path": leaf.spool_path_for_pack("P")}))
'''


class LeafLoadsOnlyByExplicitPath(RigCase):
    """X-8 (d): a script in bin/ cannot bare-import the leaf under -I; the explicit-path load works."""

    def _probe(self, flags):
        probe = os.path.join(self.rig_bin, "_leaf_probe.py")
        with open(probe, "w", encoding="utf-8") as f:
            f.write(PROBE)
        r = subprocess.run([sys.executable] + flags + [probe], capture_output=True, text=True, timeout=30, env=_env())
        self.assertEqual(r.returncode, 0, r.stderr)
        return json.loads(r.stdout)

    def test_bare_import_fails_under_isolated_mode_explicit_path_works(self):
        got = self._probe(["-I", "-B"])
        self.assertEqual(got, {"bare": "ModuleNotFoundError", "path": os.path.join("P", "state", "evt_spool.jsonl")})

    def test_bare_import_works_without_isolated_mode(self):  # control: it is -I that removes the script dir
        got = self._probe(["-E", "-s", "-B"])
        self.assertEqual(got, {"bare": "imported", "path": os.path.join("P", "state", "evt_spool.jsonl")})


class NoBytecodeIntoTheRigBin(RigCase):
    """X-8 (e): the lazy loader runs after javis_event's own `sys.dont_write_bytecode = True` - no -B given here."""

    def test_spool_path_and_emit_write_no_pycache(self):
        # positive control FIRST, in its own dir: the same kind of load (a bin module by explicit path, -I, no -B)
        # WITHOUT the seal must write __pycache__ - otherwise this interpreter cannot show the seal at all
        # (measured: Apple's /usr/bin/python3 3.9.6 has sys.flags.dont_write_bytecode == 1 even under `env -i ... -I`)
        ctl = os.path.join(self.tmp, "control-bin")
        os.makedirs(ctl)
        shutil.copy2(os.path.join(self.rig_bin, "javis_scrub.py"), ctl)
        code = ("import importlib.util,os;b=%r;s=importlib.util.spec_from_file_location("
                "'scrub',os.path.join(b,'javis_scrub.py'));m=importlib.util.module_from_spec(s);"
                "s.loader.exec_module(m)" % ctl)
        c = subprocess.run([sys.executable, "-I", "-c", code], capture_output=True, text=True, timeout=30, env=_env())
        self.assertEqual(c.returncode, 0, c.stderr)
        if _pycache_dirs(ctl) != ["__pycache__"]:
            self.skipTest("UNMEASURED on %s: it writes no bytecode even unsealed under -I (the control wrote no "
                          "__pycache__), so the seal cannot be observed with this interpreter" % sys.executable)
        r = _run(self.pack, "spool-path", seal_flag=False)
        self.assertEqual(r.returncode, E.EXIT_OK, (r.returncode, _txt(r.stderr)))
        r = _emit(self.pack, seal_flag=False)
        self.assertEqual(r.returncode, E.EXIT_OK, (r.returncode, _txt(r.stderr)))
        self.assertEqual(len(_lines(_rig_spool(self.pack))), 1)
        self.assertEqual(_pycache_dirs(self.rig_bin), [], "a __pycache__ was written into the rig bin")


if __name__ == "__main__":
    unittest.main()
