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
import textwrap
import unittest
from unittest.mock import patch

BIN = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, BIN)
import javis_hud_bridge as HB
from test_report_gate import G, FakeRunner, badges, gate, ledger_entries, report


def node(measured=78, reported=87, status_age_secs=10, **extra):
    return dict(role="worker", usage_ctx_pct=measured, context_pct=reported,
                status_age_secs=status_age_secs, agent_alive=True, idle_secs=10, **extra)


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
                measurement = json.load(f)
            row = measurement["ctx_divergence"][0]
            self.assertEqual(measurement["ctx_divergence_stats"],
                             {"compared": 1, "skipped_stale": 0, "skipped_missing": 0})
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
                measurement = json.load(f)
            self.assertEqual(measurement["ctx_divergence"], [])
            self.assertEqual(measurement["ctx_divergence_stats"]["compared"], 1)
            self.assertNotIn("gate-ctx-divergence-worker", badges(t))

    def test_freshness_and_missing_counters(self):
        """신선·경계는 비교, 나이 미상·만료는 stale, 축 결측은 missing으로 구별."""
        for age in (99999, G.CTX_SELF_REPORT_MAX_AGE_S + 1, None, "10", True,
                    float("nan"), float("inf"), float("-inf")):
            with self.subTest(age=age):
                stats = {}
                self.assertEqual(G.ctx_divergence([node(status_age_secs=age)], stats), [])
                self.assertEqual(stats, {"compared": 0, "skipped_stale": 1,
                                         "skipped_missing": 0})
        missing_age = node()
        del missing_age["status_age_secs"]
        stats = {}
        rows = G.ctx_divergence([node(), node(status_age_secs=G.CTX_SELF_REPORT_MAX_AGE_S),
                                 node(reported=80), missing_age,
                                 node(measured=None), node(reported="90")], stats)
        self.assertEqual(len(rows), 2)
        self.assertEqual(stats, {"compared": 3, "skipped_stale": 1, "skipped_missing": 2})
        self.assertEqual(divergence_warnings([missing_age, node(status_age_secs=99999)]), [])

    def test_stale_reports_allow_quiet_park_and_delta(self):
        """낡음·age 결측은 WARN을 고정하지 않아 quiet 주차와 DELTA 라우팅이 살아 있다."""
        for age in (99999, None):
            with self.subTest(age=age), tempfile.TemporaryDirectory() as t, \
                    patch.object(G, "foreign_daemon_verdict", return_value=None), \
                    contextlib.redirect_stdout(io.StringIO()):
                n = node(measured=20, reported=95, status_age_secs=age)
                if age is None:
                    del n["status_age_secs"]
                n["idle_secs"] = 600
                runner = FakeRunner(rep=report(live_nodes=[n]))
                g = gate(t, runner, quiet_cycles=3)
                for _ in range(4):
                    self.assertEqual(g.run(), 0)
                entry = ledger_entries(t)[-1]
                self.assertEqual(entry["verdict"], "QUIET")
                self.assertEqual(entry["consecutive_quiet"], 3)
                self.assertIn("master-park", [e[1] for e in runner.enqueues])
                with open(g.measure_path, encoding="utf-8") as f:
                    measurement = json.load(f)
                self.assertEqual(measurement["ctx_divergence"], [])
                self.assertEqual(measurement["ctx_divergence_stats"],
                                 {"compared": 0, "skipped_stale": 1, "skipped_missing": 0})
                runner.rep["overall_done"] = 1
                with patch.object(g, "_route_delta", wraps=g._route_delta) as route:
                    self.assertEqual(g.run(), 0)
                    route.assert_called_once()
                self.assertEqual(ledger_entries(t)[-1]["verdict"], "DELTA")

    def test_gate_env_override(self):
        """게이트도 CYS_CTX_DIVERGENCE_PCT 환경값을 소비한다."""
        code = ("import javis_report_gate as g; "
                "print(g.CTX_DIVERGENCE_ALERT_PCT); "
                "print(g.ctx_divergence([{'usage_ctx_pct':78,'context_pct':87,'status_age_secs':10}]))")
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

    def test_gate_invalid_env_import_and_reasons(self):
        """무효 env도 import·Gate rc=0, 기본 8·대장·badge 유지, WARN/push 강제 없음."""
        code = textwrap.dedent("""\
            import contextlib, io, json, os, sys, tempfile
            from unittest.mock import patch
            import javis_report_gate as g
            sys.path.insert(0, os.path.join(g._SELF_DIR, 'tests'))
            from test_report_gate import Clock, FakeRunner, badges, gate, ledger_entries, report
            with tempfile.TemporaryDirectory() as t, \\
                    patch.object(g, 'foreign_daemon_verdict', return_value=None), \\
                    contextlib.redirect_stdout(io.StringIO()):
                runner = FakeRunner(rep=report(live_nodes=[{'role':'worker',
                    'agent_alive':True, 'idle_secs':600}]))
                clk = Clock(1000000)
                instance = gate(t, runner, clock=clk)
                exits, marks = [], []
                for phase in ('baseline', 'quiet', 'gap', 'collect'):
                    if phase == 'gap':
                        clk.epoch += 16 * 60
                    if phase == 'collect':
                        runner.report_ok, runner.err = False, 'injected'
                    exits.append(instance.run())
                    marks.append('ctx_divergence_env_invalid' in badges(t))
                entries = ledger_entries(t)
            print(json.dumps({'threshold': g.CTX_DIVERGENCE_ALERT_PCT, 'exits': exits,
                              'entries': entries, 'badges': marks,
                              'pushes': runner.enqueues + runner.drains + runner.sends,
                              'divergence': g.ctx_divergence([{'usage_ctx_pct':78,
                                  'context_pct':87, 'status_age_secs':10}])}))
        """)
        for value in ("abc", "", "nan", "inf", "-inf", "-1", "0", "1e309"):
            with self.subTest(value=value):
                env = dict(os.environ, CYS_CTX_DIVERGENCE_PCT=value)
                p = subprocess.run([sys.executable, "-c", code], cwd=BIN, env=env,
                                   capture_output=True, text=True, timeout=15)
                self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
                result = json.loads(p.stdout)
                self.assertEqual(result["threshold"], 8.0)
                self.assertEqual(result["exits"], [0] * 4)
                self.assertEqual([e["verdict"] for e in result["entries"]],
                                 ["BASELINE", "QUIET", "GAP", "WARN"])
                for entry in result["entries"]:
                    self.assertIn("ctx_divergence_env_invalid", entry["reasons"])
                self.assertEqual(result["badges"], [True] * 4)
                self.assertEqual(result["pushes"], [])
                self.assertEqual(len(result["divergence"]), 1)


class ActprobeDivergence(unittest.TestCase):
    def probe(self, measured, reported, expected_rc, env_threshold=None, cli_threshold=None,
              reason_code=None):
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
                cmd += ["--threshold=" + str(cli_threshold)]
            p = subprocess.run(cmd, env=env, capture_output=True, text=True, timeout=15)
            self.assertEqual(p.returncode, expected_rc, p.stdout + p.stderr)
            output = json.loads(p.stdout)
            with open(runs_path, encoding="utf-8") as f:
                receipts = [json.loads(line) for line in f]
            self.assertEqual(len(receipts), 1)
            expected = {"measured": measured, "reported": reported, "usage_source": "statusline",
                        "diff": abs(reported - measured)
                        if measured is not None and reported is not None else None}
            if reason_code:
                expected = dict.fromkeys(expected)
            for rec in (receipts[0], output):
                self.assertEqual(rec["exit"], expected_rc)
                self.assertEqual({key: rec[key] for key in expected}, expected)
                if reason_code:
                    self.assertEqual(rec["reason_code"], reason_code)
                    self.assertEqual(rec["anomalies"][0][0], reason_code)
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

    def test_invalid_env_exit_3_with_receipt(self):
        """env abc·빈 값·nan·inf·음수 → exit 3 + threshold_env_invalid 영수증."""
        for value in ("abc", "", "nan", "inf", "-inf", "-1", "1e309"):
            with self.subTest(value=value):
                self.probe(78, 95, 3, env_threshold=value, reason_code="threshold_env_invalid")

    def test_invalid_cli_exit_3_with_receipt(self):
        """CLI 무효 임계도 파서 종료 전에 사라지지 않고 exit 3·영수증을 남긴다."""
        for value in ("abc", "", "nan", "inf", "-inf", "-1", "1e309"):
            with self.subTest(value=value):
                self.probe(78, 95, 3, cli_threshold=value, reason_code="threshold_env_invalid")
        self.probe(78, 78, 0, cli_threshold=0)
        self.probe(78, 79, 2, env_threshold=0)


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
