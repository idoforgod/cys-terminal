#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_preflight_c84_repair_sweep.py — C84(0.15.0 「스스로 해결」 M10 · T6): 점검 때 데몬에게 미승인 변경 훑기를 청한다.

계약(설계 §4-10 · C7 F9): 판정·되돌리기는 데몬이 한다 — 이 점검은 `cys repair sweep --json` 으로 청하고(데몬을 깨우지
않는다), 데몬이 한 번만 돌려주는 알림을 보고할 뿐이다. 보고 모드에서는 청하지 않는다(무변경). 데몬 응답이 없으면 SKIP
(점검을 막지 않는다). `cys` 는 띄우지 않는다 — `_cys_local_json` 을 갈아 끼운 순수 검체.

    python3 cysjavis-pack/bin/tests/test_preflight_c84_repair_sweep.py
"""
import inspect
import os
import sys
import unittest

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
        rows, calls = self.run_check("fix", {"tick": "started", "notices": [
            {"rel": "hooks/inject-context.sh", "note": "승인 없이 바뀌었지만 자체 검사를 통과 — 그대로 둠", "kind": "kept"}]})
        self.assertEqual(calls, [(["repair", "sweep", "--json"], pf.REPAIR_SWEEP_TIMEOUT_S)])
        self.assertEqual(len(rows), 1)
        self.assertEqual(rows[0]["id"], "C84.repair-sweep")
        self.assertEqual(rows[0]["status"], WARN)
        self.assertIn("hooks/inject-context.sh", rows[0]["detail"])

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

    def test_registered_once_before_the_last_fixed_slots(self):
        src = inspect.getsource(pf.Preflight.run)
        self.assertEqual(src.count("self.c84_repair_sweep,"), 1)
        self.assertLess(src.index("self.c84_repair_sweep,"), src.index("self.c62_pack_heal_ledger,"))

    def test_the_cli_call_never_starts_a_daemon(self):
        # `_cys_local_json` runs `cys` with CYS_NO_AUTOSTART=1 (and the repair CLI turns autostart off itself)
        self.assertIn('"CYS_NO_AUTOSTART": "1"', inspect.getsource(pf._cys_local_json))


if __name__ == "__main__":
    unittest.main()
