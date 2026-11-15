#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_preflight_c84_repair_sweep.py — C84(0.15.0 「스스로 해결」 M10 · T6): 점검 때 데몬에게 미승인 변경 훑기를 청한다.

계약(설계 §4-10 · C7 F9): 판정·되돌리기는 데몬이 한다 — 이 점검은 `cys repair sweep --json` 으로 청하고(데몬을 깨우지
않는다), 데몬이 한 번만 돌려주는 알림을 보고할 뿐이다. 보고 모드에서는 청하지 않는다(무변경). 데몬 응답이 없으면 SKIP
(점검을 막지 않는다). `cys` 는 띄우지 않는다 — `_cys_local_json` 을 갈아 끼운 순수 검체.

    python3 cysjavis-pack/bin/tests/test_preflight_c84_repair_sweep.py
"""
import inspect
import json
import os
import shutil
import sys
import tempfile
import unittest
from unittest import mock

TESTS_DIR = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(TESTS_DIR)
sys.path.insert(0, BIN)

import javis_preflight as pf  # noqa: E402

PASS, WARN, SKIP = pf.PASS, pf.WARN, pf.SKIP


class C84RepairSweep(unittest.TestCase):
    def run_check(self, mode, reply):
        calls = []

        def fake(args, timeout=None):
            calls.append((list(args), timeout))
            return reply

        orig = pf._cys_local_json
        pf._cys_local_json = fake
        try:
            p = pf.Preflight(fix=(mode == "fix"), skips=[], mode=mode)
            p.c84_repair_sweep()
            return p.results, calls
        finally:
            pf._cys_local_json = orig

    def test_fix_mode_requests_the_sweep_and_reports_new_notices(self):
        # GK3 (REWRITTEN — old expectation: exactly ONE call, `repair sweep --json`): the notices are
        # reported, THEN their keys are confirmed in a second call (the daemon marks nothing itself)
        rows, calls = self.run_check("fix", {"tick": "started", "notices": [
            {"key": "f:hooks/inject-context.sh:ab:kept", "rel": "hooks/inject-context.sh",
             "note": "승인 없이 바뀌었지만 자체 검사를 통과 — 그대로 둠", "kind": "kept"}]})
        self.assertEqual(calls, [(["repair", "sweep", "--json"], pf.REPAIR_SWEEP_TIMEOUT_S),
                                 (["repair", "sweep", "--json", "--confirm", "f:hooks/inject-context.sh:ab:kept"],
                                  pf.REPAIR_SWEEP_TIMEOUT_S)])
        self.assertEqual(len(rows), 1)
        self.assertEqual(rows[0]["id"], "C84.repair-sweep")
        self.assertEqual(rows[0]["status"], WARN)
        self.assertIn("hooks/inject-context.sh", rows[0]["detail"])

    def test_nothing_is_confirmed_without_notices(self):
        # GK3: no notice taken → no confirmation call
        _, calls = self.run_check("fix", {"tick": "started", "notices": []})
        self.assertEqual(len(calls), 1)

    def test_no_notice_is_pass(self):
        rows, _ = self.run_check("fix", {"tick": "busy", "notices": []})
        self.assertEqual(rows[0]["status"], PASS)
        self.assertIn("busy", rows[0]["detail"])

    def test_report_mode_requests_nothing(self):
        for mode in ("report", "dry", "safe"):
            rows, calls = self.run_check(mode, {"tick": "started"})
            self.assertEqual(calls, [], mode)
            self.assertEqual(rows[0]["status"], SKIP, mode)

    def test_no_daemon_is_skip_never_a_failure(self):
        rows, calls = self.run_check("fix", None)
        self.assertEqual(len(calls), 1)
        self.assertEqual(rows[0]["status"], SKIP)
        self.assertNotIn("unmeasured", rows[0])

    def test_the_sweep_writes_no_settings_file(self):
        # S5-MUT part 1 · H-10-A1-32-01 (RULINGS JU): the sweep is a REQUEST to the daemon — the check itself writes no
        #   Claude settings file (a write there would bypass the settings writer's lock). The tests above fake the CLI and
        #   look only at the call list, so an unlocked read-modify-write of ~/.claude/settings.json added to the check
        #   stayed green. Here: a private HOME holding a settings file; after the check (with and without notices) the
        #   settings folder holds exactly the same bytes.
        home = tempfile.mkdtemp(prefix="c84-home-")
        self.addCleanup(shutil.rmtree, home, True)
        cfg = os.path.join(home, ".claude")
        os.makedirs(cfg)
        with open(os.path.join(cfg, "settings.json"), "w", encoding="utf-8") as f:
            json.dump({"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "echo mine"}]}]}, "theme": "auto"},
                      f, indent=2)

        def snapshot():
            out = {}
            for d, _, files in os.walk(cfg):
                for n in files:
                    with open(os.path.join(d, n), "rb") as f:
                        out[os.path.relpath(os.path.join(d, n), cfg)] = f.read()
            return out

        before = snapshot()
        env = {k: v for k, v in os.environ.items() if not k.startswith("CLAUDE")}
        env["HOME"] = home
        with mock.patch.dict(os.environ, env, clear=True):
            self.assertEqual(os.path.expanduser("~"), home)
            for reply in ({"tick": "started", "notices": []},
                          {"tick": "started", "notices": [{"key": "k1", "rel": "hooks/x.sh", "note": "n", "kind": "kept"}]}):
                rows, calls = self.run_check("fix", reply)
                self.assertTrue(calls, "the sweep was requested")
                self.assertEqual(snapshot(), before, "the check wrote into the Claude settings folder")

    def test_registered_once_before_the_last_fixed_slots(self):
        src = inspect.getsource(pf.Preflight.run)
        self.assertEqual(src.count("self.c84_repair_sweep,"), 1)
        self.assertLess(src.index("self.c84_repair_sweep,"), src.index("self.c62_pack_heal_ledger,"))

    def test_the_cli_call_never_starts_a_daemon(self):
        # `_cys_local_json` runs `cys` with CYS_NO_AUTOSTART=1 (and the repair CLI turns autostart off itself)
        self.assertIn('"CYS_NO_AUTOSTART": "1"', inspect.getsource(pf._cys_local_json))


if __name__ == "__main__":
    unittest.main()
