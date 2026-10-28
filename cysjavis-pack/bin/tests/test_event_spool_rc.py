#!/usr/bin/env python3
"""javis_event.py emit --spool must not report success when the spool append failed (0.14.56 EMIT-FORK E4).

At 9da2418d `cmd_emit` caught the OSError of `_spool_append` and returned EXIT_OK, so every caller that reads the
exit code of a `--spool` emit (javis_report_gate.emit, javis_approval_queue._emit) saw a delivery that never
happened. The wire line is still printed (the spool stays a best-effort carrier); only the exit code changes.

The fault is portable (Windows too): HUD_STATE_DIR points at an existing REGULAR FILE, so creating the spool
directory fails. The specimen first proves the fault bites (nothing written), and carries a positive control
(a writable spool dir gives exit 0 and exactly one spool line) so a test that always sees a failure cannot pass.
"""
import json
import os
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
SCRIPT = os.path.join(os.path.dirname(HERE), "javis_event.py")
sys.path.insert(0, os.path.dirname(HERE))
import javis_event as E  # noqa: E402


def _run(hud_state_dir, spool=True):
    env = {k: v for k, v in os.environ.items() if k != "HUD_STATE_DIR"}
    env["HUD_STATE_DIR"] = hud_state_dir
    argv = [sys.executable, "-I", "-B", SCRIPT, "emit", "task.unblocked", "--payload", '{"task":"T"}']
    if spool:
        argv.append("--spool")
    return subprocess.run(argv, capture_output=True, text=True, timeout=30, env=env)


def _tree(root):
    out = {}
    for dp, dns, fns in os.walk(root):
        for n in dns + fns:
            p = os.path.join(dp, n)
            out[os.path.relpath(p, root)] = os.path.getsize(p) if os.path.isfile(p) else "dir"
    return out


class SpoolFailureIsNotSuccess(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="evt-spool-rc-")
        self.blocker = os.path.join(self.tmp, "not-a-dir")
        with open(self.blocker, "w", encoding="utf-8") as f:
            f.write("x")

    def tearDown(self):
        import shutil
        shutil.rmtree(self.tmp, ignore_errors=True)

    def test_failed_spool_append_returns_spool_failed(self):
        before = _tree(self.tmp)
        r = _run(self.blocker)
        # the fault bites: nothing was written anywhere under the root
        self.assertEqual(_tree(self.tmp), before, "the injected fault did not prevent the spool write")
        self.assertEqual(r.returncode, E.EXIT_SPOOL_FAILED, (r.returncode, r.stderr))
        self.assertIn("spool append failed", r.stderr)
        # the wire line is still emitted: the spool is a carrier, not a gate on the wire
        self.assertIn("[EVT v2] task.unblocked", r.stdout)

    def test_writable_spool_returns_ok_and_writes_one_line(self):  # positive control
        d = os.path.join(self.tmp, "hud")
        r = _run(d)
        self.assertEqual(r.returncode, E.EXIT_OK, r.stderr)
        with open(os.path.join(d, "evt_spool.jsonl"), encoding="utf-8") as f:
            lines = f.read().splitlines()
        self.assertEqual(len(lines), 1)
        self.assertEqual(json.loads(lines[0])["type"], "task.unblocked")

    def test_without_spool_flag_the_blocked_dir_is_irrelevant(self):
        r = _run(self.blocker, spool=False)
        self.assertEqual(r.returncode, E.EXIT_OK, r.stderr)

    def test_exit_code_is_distinct(self):
        codes = {E.EXIT_OK, E.EXIT_USAGE, E.EXIT_INVALID, E.EXIT_DEGRADED}
        self.assertNotIn(E.EXIT_SPOOL_FAILED, codes)


if __name__ == "__main__":
    unittest.main()
