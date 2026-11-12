#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_report_gate_open_defects.py — 「스스로 해결」(0.15.0 · 설계 §4-4 · §6-4 (3)(7) · RULINGS S2 · track T3).

접수는 장부의 한 줄이고 마스터 큐로 가지 않는다. 5분 보고 관문이 한 차례에 많아야 **한 줄**
("열린 결함 N건 — 워커에게 맡길 것")을 master 에게 넣는다. 같은 열린 결함 집합이면 멱등 키가 같아 깨우기
큐가 억제한다(관문 한 차례에 알림 1건 이하 · 반복 없음). `cys` 부재·실패·shadow 는 아무것도 보내지 않는다.

라이브 무접촉: 게이트는 시험 대역 러너로 돌고, `cys` 는 임시 폴더의 가짜 스크립트다. 깨우기 큐는 임시
JAVIS_ROOT 에만 쓴다.

실행: CYS_PACK_DIR="$(mktemp -d)" python3 cysjavis-pack/bin/tests/test_report_gate_open_defects.py
"""
import json
import os
import subprocess
import sys
import tempfile
import unittest

sys.dont_write_bytecode = True
TESTS = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(TESTS)
sys.path.insert(0, BIN)
sys.path.insert(0, TESTS)
import javis_report_gate as G  # noqa: E402
from test_report_gate import FakeRunner, Clock  # noqa: E402  (sibling fixtures)


def make_gate(state_dir, runner, source):
    clk = Clock(1_000_000.0)
    return G.Gate(state_dir, runner, cycle_minutes=5, now_epoch_fn=clk.now_epoch,
                  now_iso_fn=clk.now_iso, defect_source=source)


def master_wakes(runner):
    return [e for e in runner.enqueues if e[1] == G.OPEN_DEFECT_TASK]


class OpenDefectLine(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="gate-defects-")

    def run_once(self, source, shadow=False):
        r = FakeRunner(report_ok=False, err="collector absent")   # the gate's own path stays minimal
        rc = make_gate(os.path.join(self.tmp, "gate"), r, source).run(shadow=shadow)
        self.assertEqual(rc, 0)
        return r

    def test_one_line_to_master_per_run(self):
        r = self.run_once(lambda: (True, "base", ["aa", "bb"]))
        wakes = master_wakes(r)
        self.assertEqual(len(wakes), 1, r.enqueues)
        to, task, body, idem, severity = wakes[0]
        self.assertEqual(to, "master")
        self.assertEqual(body, "열린 결함 2건 — 워커에게 맡길 것")
        self.assertTrue(idem.startswith("repair-open-defects-"))
        self.assertEqual(severity, G.SEV_INFO)
        self.assertIn("master", r.drains)

    def test_same_open_set_same_key_new_defect_new_key(self):
        k1 = master_wakes(self.run_once(lambda: (True, "base", ["bb", "aa"])))[0][3]
        k2 = master_wakes(self.run_once(lambda: (True, "base", ["aa", "bb"])))[0][3]
        k3 = master_wakes(self.run_once(lambda: (True, "base", ["aa", "bb", "cc"])))[0][3]
        self.assertEqual(k1, k2, "the same open set must coalesce to one wake")
        self.assertNotEqual(k1, k3)

    def test_nothing_when_empty_failing_off_or_shadow(self):
        for source in (lambda: (True, "base", []), lambda: (False, None, []), None):
            self.assertEqual(master_wakes(self.run_once(source)), [])

        def boom():
            raise RuntimeError("source exploded")
        self.assertEqual(master_wakes(self.run_once(boom)), [], "a failing source must not stop the gate")
        self.assertEqual(master_wakes(self.run_once(lambda: (True, "base", ["aa"]), shadow=True)), [])

    def test_wake_queue_suppresses_the_same_key(self):
        """The coalescing is the wake queue's own (javis_wakeup.py): the same key twice → one pending."""
        root = tempfile.mkdtemp(prefix="wk-root-")
        os.makedirs(os.path.join(root, "_round"), exist_ok=True)
        body, idem = G.open_defect_wake(["aa", "bb"])
        script = os.path.join(BIN, "javis_wakeup.py")
        env = dict(os.environ, JAVIS_ROOT=root)
        for _ in range(3):
            p = subprocess.run([sys.executable, script, "enqueue", "--to", "master", "--task", G.OPEN_DEFECT_TASK,
                                "--reason", body, "--idempotency-key", idem],
                               capture_output=True, text=True, env=env, timeout=30)
            self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
        pending = os.listdir(os.path.join(root, "_round", "wakeups", "pending"))
        self.assertEqual(len([f for f in pending if f.endswith(".json")]), 1, pending)


class CollectOpenDefects(unittest.TestCase):
    """`cys repair list --json` parsing with a fake `cys` (bounded, CYS_NO_AUTOSTART=1, distinct signatures — G4)."""

    def fake_cys(self, body, rc=0):
        d = tempfile.mkdtemp(prefix="fake-cys-")
        envlog = os.path.join(d, "env.log")
        p = os.path.join(d, "cys")
        with open(p, "w", encoding="utf-8") as f:
            f.write("#!/bin/sh\nprintf '%%s\\n' \"$CYS_NO_AUTOSTART\" > '%s'\ncat <<'JSON'\n%s\nJSON\nexit %d\n" % (envlog, body, rc))
        os.chmod(p, 0o755)
        return p, envlog

    @unittest.skipIf(os.name == "nt", "shell fake")
    def test_counts_distinct_open_signatures_across_lanes(self):
        doc = {"lane": "base", "defects": [
            {"signature": "s1", "open": True, "lanes": ["base"]},
            {"signature": "s2", "open": False, "lanes": ["base"]},
            {"signature": "s3", "open": True, "lanes": ["dept-1"]},
            {"signature": "s4", "open": True, "lanes": ["dept-1", "base"]},
        ]}
        cys, envlog = self.fake_cys(json.dumps(doc))
        ok, lane, sigs = G.collect_open_defects(cys)
        self.assertTrue(ok)
        self.assertEqual(lane, "base")
        self.assertEqual(sigs, ["s1", "s3", "s4"], "G4: distinct open signatures across lanes, closed ones excluded")
        with open(envlog, encoding="utf-8") as f:
            self.assertEqual(f.read().strip(), "1", "CYS_NO_AUTOSTART=1 must be passed")

    @unittest.skipIf(os.name == "nt", "shell fake")
    def test_failures_send_nothing(self):
        for body, rc in (("not json", 0), ('{"lane": "base"}', 0), ('{"lane":"base","defects":[]}', 93)):
            cys, _ = self.fake_cys(body, rc)
            self.assertEqual(G.collect_open_defects(cys), (False, None, []), body)
        self.assertEqual(G.collect_open_defects("/nonexistent/cys"), (False, None, []))
        self.assertIsNone(G.open_defect_wake([]))


if __name__ == "__main__":
    unittest.main()
