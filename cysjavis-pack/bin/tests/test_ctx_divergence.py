#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""WP6-3: 괴리 임계·영수증·보고 게이트·HUD 신선도 회귀(실데몬 호출 0).

test_report_gate의 FakeRunner/시계를 재사용한다. actprobe는 --status-file,
--caller, --runs-path를 모두 주입하여 cys status/identify와 라이브 원장을 피한다.
실행: python3 cysjavis-pack/bin/tests/test_ctx_divergence.py
"""
import contextlib
import io
import json
import os
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

BIN = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, BIN)
import javis_hud_bridge as HB
from test_report_gate import G, FakeRunner, badges, gate, ledger_entries, report


def node(measured=78, reported=87, **extra):
    return dict(role="worker", usage_ctx_pct=measured, context_pct=reported,
                agent_alive=True, idle_secs=10, **extra)


def divergence_warnings(nodes):
    return [w for w in G.extract_warnings(report(live_nodes=nodes))
            if w["trigger"] == "ctx_divergence"]


class ReportDivergence(unittest.TestCase):
    def setUp(self):
        self.threshold = patch.object(G, "CTX_DIVERGENCE_ALERT_PCT", 8.0)
        self.threshold.start()
        self.addCleanup(self.threshold.stop)

    def test_nine_points_alert(self):
        """9pt 괴리가 기본 임계(8)에서 잡힌다."""
        self.assertEqual(len(divergence_warnings([node()])), 1)
        self.assertEqual(G.ctx_divergence([node()])[0]["role"], "worker")

    def test_old_default_missed_incident(self):
        """옛 기본값 15였다면 놓쳤을 사례임을 박제."""
        self.assertTrue(9.0 <= 87 - 78 <= 15.0)
        with patch.object(G, "CTX_DIVERGENCE_ALERT_PCT", 15.0):
            self.assertEqual(divergence_warnings([node()]), [])

    def test_missing_measured_is_not_comparable(self):
        """실측 없음(agy)은 괴리 판정 대상이 아니다."""
        self.assertEqual(divergence_warnings([
            {"role": "gemini", "usage_ctx_pct": None, "context_pct": 90}]), [])

    def test_signed_message(self):
        """부호를 문구에 남긴다: 자기보고 95 vs 실측 78 = +17."""
        warning = divergence_warnings([node(reported=95)])[0]
        self.assertIn("자기보고 95 vs 실측 78 = +17", warning["wake_body"])
        self.assertIn("+17", warning["reason"])

    def test_negative_sign_and_exact_threshold(self):
        """음수 괴리도 경보, 정확히 8pt와 2pt는 경보 없음."""
        warning = divergence_warnings([node(reported=69)])[0]
        self.assertIn("자기보고 69 vs 실측 78 = -9", warning["wake_body"])
        self.assertEqual(divergence_warnings([node(reported=86), node(reported=80)]), [])

    def test_both_axes_require_numbers(self):
        """어느 축이든 결측·비수치면 경보 없음, 실제 0은 숫자."""
        for bad in (None, "90", True, float("nan"), float("inf")):
            with self.subTest(bad=bad):
                # NaN/Infinity는 정상 JSON 밖의 주입값: 괴리 술어의 방어를 직접 잰다.
                self.assertEqual(G.ctx_divergence([node(measured=bad)]), [])
                self.assertEqual(G.ctx_divergence([node(reported=bad)]), [])
        self.assertEqual(divergence_warnings([{"context_pct": 90}]), [])
        self.assertEqual(divergence_warnings([{"usage_ctx_pct": 90}]), [])
        self.assertEqual(len(divergence_warnings([node(measured=0, reported=9)])), 1)

    def test_cycle_persists_measurement_and_signed_badge(self):
        """합성 보고 주기: JSON 필드·대장·배지 기록, 회복 시 해소, push 0."""
        with tempfile.TemporaryDirectory() as t, \
                patch.object(G, "foreign_daemon_verdict", return_value=None), \
                contextlib.redirect_stdout(io.StringIO()):
            runner = FakeRunner(rep=report(live_nodes=[node()]))
            g = gate(t, runner)
            self.assertEqual(g.run(), 0)  # BASELINE도 관측 JSON을 쓴다.
            with open(g.measure_path, encoding="utf-8") as f:
                row = json.load(f)["ctx_divergence"][0]
            self.assertEqual(row, {"role": "worker", "diff": 9.0, "signed_diff": 9.0,
                                   "measured": 78, "reported": 87})
            self.assertEqual(g.run(), 0)
            self.assertEqual(ledger_entries(t)[-1]["verdict"], "WARN")
            self.assertTrue(any("ctx_divergence:" in r and "+9" in r
                                for r in ledger_entries(t)[-1]["reasons"]))
            self.assertIn("+9", badges(t)["gate-ctx-divergence-worker"]["message"])
            self.assertEqual(runner.enqueues + runner.drains + runner.sends + runner.emits, [])
            runner.rep = report(live_nodes=[node(reported=80)])
            self.assertEqual(g.run(), 0)
            with open(g.measure_path, encoding="utf-8") as f:
                self.assertEqual(json.load(f)["ctx_divergence"], [])
            self.assertNotIn("gate-ctx-divergence-worker", badges(t))

    def test_gate_env_override(self):
        """게이트도 CYS_CTX_DIVERGENCE_PCT 환경값을 소비한다."""
        code = ("import javis_report_gate as g; "
                "print(g.CTX_DIVERGENCE_ALERT_PCT); "
                "print(g.ctx_divergence([{'usage_ctx_pct':78,'context_pct':87}]))")
        for env_value, expected in ((None, "8.0"), ("15", "15.0")):
            env = dict(os.environ)
            env.pop("CYS_CTX_DIVERGENCE_PCT", None)
            if env_value is not None:
                env["CYS_CTX_DIVERGENCE_PCT"] = env_value
            p = subprocess.run([sys.executable, "-c", code], cwd=BIN, env=env,
                               capture_output=True, text=True, timeout=15)
            self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
            self.assertEqual(p.stdout.splitlines()[0], expected)
            self.assertEqual(p.stdout.splitlines()[1] == "[]", env_value == "15")


class ActprobeDivergence(unittest.TestCase):
    def probe(self, measured, reported, expected_rc, env_threshold=None, cli_threshold=None):
        with tempfile.TemporaryDirectory() as t:
            status_path = os.path.join(t, "status.json")
            runs_path = os.path.join(t, "probe_runs.jsonl")
            with open(status_path, "w", encoding="utf-8") as f:
                json.dump({"surfaces": [{"surface_ref": "s1",
                           "usage": {"ctx_pct": measured, "source": "statusline"},
                           "status": {"context_pct": reported}}]}, f)
            env = dict(os.environ, CYS_BIN=os.path.join(t, "must-not-call-cys"))
            env.pop("CYS_CTX_DIVERGENCE_PCT", None)
            if env_threshold is not None:
                env["CYS_CTX_DIVERGENCE_PCT"] = str(env_threshold)
            cmd = [sys.executable, os.path.join(BIN, "javis_actprobe.py"),
                   "ctx-compare", "--surface", "s1", "--status-file", status_path,
                   "--caller", "test-ctx-divergence", "--runs-path", runs_path, "--json"]
            if cli_threshold is not None:
                cmd += ["--threshold", str(cli_threshold)]
            p = subprocess.run(cmd, env=env, capture_output=True, text=True, timeout=15)
            self.assertEqual(p.returncode, expected_rc, p.stdout + p.stderr)
            output = json.loads(p.stdout)
            with open(runs_path, encoding="utf-8") as f:
                receipts = [json.loads(line) for line in f]
            self.assertEqual(len(receipts), 1)
            expected = {"measured": measured, "reported": reported, "usage_source": "statusline",
                        "diff": abs(reported - measured)
                        if measured is not None and reported is not None else None}
            for rec in (receipts[0], output):
                self.assertEqual(rec["exit"], expected_rc)
                self.assertEqual({key: rec[key] for key in expected}, expected)
            self.assertEqual(receipts[0]["caller"], "test-ctx-divergence")

    def test_nine_points_exit_2(self):
        """actprobe --status-file measured 78 / reported 87 → rc=2, 영수증 4필드."""
        self.probe(78, 87, 2)

    def test_two_points_exit_0(self):
        """actprobe --status-file measured 78 / reported 80 → rc=0, 영수증 4필드."""
        self.probe(78, 80, 0)

    def test_missing_measured_exit_3(self):
        """actprobe --status-file measured null → rc=3, diff null 보존."""
        self.probe(None, 90, 3)

    def test_missing_reported_exit_3(self):
        """actprobe 자기보고 결측 → rc=3, 측정값 보존."""
        self.probe(78, None, 3)

    def test_exact_threshold_and_overrides(self):
        """actprobe 정확히 8pt는 rc=0, env 임계 및 CLI 우선 덮어쓰기."""
        self.probe(78, 86, 0)
        self.probe(78, 87, 0, env_threshold=15)
        self.probe(78, 87, 2, env_threshold=15, cli_threshold=8)


class HudContextFreshness(unittest.TestCase):
    def test_stale_report_is_none(self):
        """HUD 낡은 자기보고(age 301) → None."""
        self.assertIsNone(HB.pick_ctx({"status": {"context_pct": 95, "age_secs": 301}}))

    def test_fresh_report_is_value(self):
        """HUD 신선한 자기보고(age 10) → 값."""
        self.assertEqual(HB.pick_ctx({"status": {"context_pct": 95, "age_secs": 10}}), 95)

    def test_boundary_missing_age_and_measured_priority(self):
        """HUD age 300 포함, age 결측은 None, 실측 우선 유지."""
        self.assertEqual(HB.pick_ctx({"status": {"context_pct": 95, "age_secs": 300}}), 95)
        for age in (None, "10"):
            self.assertIsNone(HB.pick_ctx({"status": {"context_pct": 95, "age_secs": age}}))
        self.assertIsNone(HB.pick_ctx({"status": {"context_pct": 95}}))
        self.assertEqual(HB.pick_ctx({"usage": {"ctx_pct": 0},
                                     "status": {"context_pct": 95, "age_secs": 301}}), 0)


class PassResult(unittest.TextTestResult):
    def addSuccess(self, test):
        super().addSuccess(test)
        self.stream.writeln("PASS " + (test.shortDescription() or test.id()))


if __name__ == "__main__":
    suite = unittest.defaultTestLoader.loadTestsFromModule(sys.modules[__name__])
    result = unittest.TextTestRunner(verbosity=0, resultclass=PassResult).run(suite)
    print("=== %d/%d PASS ===" % (result.testsRun - len(result.failures) - len(result.errors),
                                 result.testsRun))
    sys.exit(0 if result.wasSuccessful() else 1)
