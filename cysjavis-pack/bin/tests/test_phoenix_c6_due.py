#!/usr/bin/env python3
"""S5 C6 contract: scripted commands only; no daemon or live endpoint."""
import importlib.util
import contextlib
import json
import tempfile
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest import mock

spec = importlib.util.spec_from_file_location("phoenix_s5", Path(__file__).resolve().parents[1] / "javis_phoenix.py")
P = importlib.util.module_from_spec(spec)
spec.loader.exec_module(P)


class C6DueTests(unittest.TestCase):
    def run_c6(self, replies):
        calls = []
        def run(*args, **kwargs):
            calls.append((args, kwargs))
            self.assertLessEqual(len(calls), len(replies), "S5 no extra retry")
            rc, out, err = replies[len(calls) - 1]
            return SimpleNamespace(returncode=rc, stdout=out, stderr=err)
        live = {"worker-s5": [{"surface": "surface:9", "exited": True, "pid": None}]}
        with mock.patch.object(P, "live_role_surfaces", return_value=live), mock.patch.object(P, "cys", side_effect=run):
            result = P.c6_reap_stale_surfaces("/isolated/s5.sock")
        self.assertEqual(calls[0][0], ("reap-due", "surface:9"), "S5 due verb")
        self.assertEqual(calls[0][1], {"socket": "/isolated/s5.sock", "timeout": 12})
        return result, calls

    def test_four_outcomes(self):
        for outcome in ("reaped", "deferred", "gone"):
            with self.subTest(outcome=outcome):
                body = {"surface_id": 9, "outcome": outcome}
                if outcome == "deferred":
                    body["reason"] = "grace_not_elapsed"
                result, calls = self.run_c6([(0, json.dumps(body), "")])
                self.assertEqual(result["reaped"], ["surface:9"] if outcome == "reaped" else [])
                self.assertEqual(len(result["reap_deferred"]), int(outcome == "deferred"))
                self.assertEqual(result["gone"], ["surface:9"] if outcome == "gone" else [])
                self.assertEqual(result["reap_failed"], [])
                self.assertEqual(len(calls), 1)

    def test_errors_never_fallback(self):
        for rc, out, err in [(1, "", e) for e in ("not_found: gone", "invalid_params", "close_denied", "transport error")] + [(124, "", "TIMEOUT"), (0, "not json", ""), (0, '{"outcome":"unknown"}', "")]:
            with self.subTest(error=err or out):
                result, calls = self.run_c6([(rc, out, err)])
                self.assertEqual(len(calls), 1, "S5 no generic fallback")
                self.assertEqual(len(result["reap_failed"]), 1)
                self.assertEqual(result["reaped"], [])

    def test_capability_fallback_once(self):
        for first in [(1, "", "method_not_found: surface.reap_due"), (2, "", "unrecognized subcommand 'reap-due'")]:
            for legacy_rc in (0, 1):
                with self.subTest(first=first, legacy_rc=legacy_rc):
                    result, calls = self.run_c6([first, (legacy_rc, "", "close_denied" if legacy_rc else "")])
                    self.assertEqual(calls[1][0], ("close-surface", "surface:9", "--reap"))
                    self.assertEqual(len(calls), 2)
                    self.assertEqual(result["reaped"], ["surface:9"] if legacy_rc == 0 else [])
                    self.assertEqual(len(result["reap_failed"]), int(legacy_rc != 0))

    def test_malformed_or_wrong_target_never_counts_reaped(self):
        for body in [{"outcome":"reaped"}, {"outcome":"gone", "surface_id":True},
                     {"outcome":"reaped", "surface_id":"9"}, {"outcome":"reaped", "surface_id":10},
                     {"outcome":"deferred", "surface_id":9}, []]:
            with self.subTest(body=body):
                result, calls = self.run_c6([(0, json.dumps(body), "")])
                self.assertEqual(result["reaped"], [])
                self.assertEqual(result["gone"], [])
                self.assertEqual(result["reap_deferred"], [])
                self.assertEqual(len(result["reap_failed"]), 1)
                self.assertEqual(len(calls), 1)

    def test_restore_continues_and_keeps_c6_journal_and_early_results(self):
        self.run_restore_cases(windows=False)

    def test_windows_restore_state_stays_in_fixture(self):
        self.run_restore_cases(windows=True)

    def run_restore_cases(self, windows):
        for disposition in ("deferred", "failed", "tombstoned", "unknown", "corrupt"):
            with self.subTest(disposition=disposition), tempfile.TemporaryDirectory(prefix="s5-ph-") as td, contextlib.ExitStack() as stack:
                sock = str(Path(td) / "d.sock")
                stack.enter_context(mock.patch.object(P, "IS_WINDOWS", windows))
                stack.enter_context(mock.patch.dict(P.os.environ, {"LOCALAPPDATA": str(Path(td) / "appdata")}))
                state_dir = Path(P.state_dir_for(sock)).resolve()
                self.assertTrue(state_dir.is_relative_to(Path(td).resolve()),
                                "S5 fixture state must stay inside its temp root before any write")
                live = {"worker-s5": [{"surface":"surface:9", "exited":True}]}
                replies = SimpleNamespace(returncode=1, stdout="", stderr="transport error") if disposition == "failed" else SimpleNamespace(returncode=0, stdout=json.dumps({"surface_id":9,"outcome":"deferred","reason":"grace_not_elapsed"}), stderr="")
                spawned = []
                def spawn(socket, role, observed, **kwargs):
                    spawned.append(role)
                    live[role].append({"surface":"surface:10", "exited":False, "seat":"occupied"})
                    return "surface:10", "fixture replacement"
                patches = {"CYS":"/nonexistent-s5-cys", "cys":mock.Mock(return_value=replies),
                           "get_boot_epoch":lambda socket: "s5", "live_role_surfaces":lambda socket: live,
                           "_LAST_LIVENESS_KNOWN":disposition != "unknown",
                           "observe_and_persist_roster":lambda socket: ({"worker-s5":{"role":"worker-s5","session_id":"s5-session"}}, {"worker-s5"} if disposition == "tombstoned" else set()),
                           "spawn_surrogate":spawn, "stage_ready":lambda *a: (True,"fixture"),
                           "stage_observe_session":lambda *a, **k: ("s5-session","fixture"),
                           "stage_reinject":lambda *a: (True,"fixture"), "stage_g2_ack":lambda *a: (True,"fixture"),
                           "_emit_evt":lambda *a, **k: None,
                           "_recovered_provenance":lambda p: {"source":"fixture","ts":1} if disposition == "corrupt" else None}
                for name, value in patches.items():
                    stack.enter_context(mock.patch.object(P, name, value))
                result = P.run_restore(sock, ticket="s5-first", stub=True, no_breaker=True, print_result=False)
                self.assertEqual(spawned, ["worker-s5"] if disposition in ("deferred","failed") else [], "S5 deferred must not abort restoration")
                expected_key = "c6_reap_failed" if disposition == "failed" else "c6_reap_deferred"
                self.assertEqual(len(result[expected_key]), 1)
                journal = json.loads(Path(P.journal_path(sock,"s5-first")).read_text())
                self.assertEqual(journal[expected_key], result[expected_key])
                self.assertTrue(any(e["stage"] == "s0_c6" and e["status"] == "observed" for e in journal["events"]))
                if disposition in ("unknown", "corrupt"):
                    self.assertEqual(result["phoenix_restore"], "DEGRADED")
                    self.assertNotEqual(P.restore_exit_code(result), 0)
                if disposition in ("deferred", "failed"):
                    P.run_restore(sock, ticket="s5-second", stub=True, no_breaker=True, print_result=False)
                    self.assertEqual(spawned, ["worker-s5"], "S5 replacement exactly once")
                    live["worker-s5"] = [live["worker-s5"][-1]]
                    empty = P.run_restore(sock, ticket="s5-first", stub=True, no_breaker=True, print_result=False)
                    self.assertEqual(empty[expected_key], [])
                    journal = json.loads(Path(P.journal_path(sock,"s5-first")).read_text())
                    self.assertEqual(journal[expected_key], [], "S5 reused ticket must clear stale C6 observations")


if __name__ == "__main__":
    unittest.main()
