#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_wakeup_drain.py — javis_wakeup.py drain 의 배달 결과 분기 회귀 (C7 · DESIGN §4-C7).

핵심 계약(Sim1 S1-2): 데몬의 `queue_softcap_exceeded` 는 **실패가 아니라 종결**이다.
  - pending 제거 · 원장 skipped(why=`dead-lettered(softcap)`) · **fast-fail 카운터 미증가**
  - 그 외 실패는 기존 G13(연속 3회 fast-fail) 그대로

cys 는 호출하지 않는다 — subprocess.run 을 대역으로 갈아끼워 밀폐 검증한다.

실행: python3 test_wakeup_drain.py
"""
import io
import json
import os
import sys
import tempfile
import unittest
from contextlib import redirect_stdout, redirect_stderr

BIN = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))          # cysjavis-pack/bin
sys.path.insert(0, BIN)


class Args:
    def __init__(self, deliver=True, target=None):
        self.deliver, self.target = deliver, target


class FakeCompleted:
    def __init__(self, rc, stdout="", stderr=""):
        self.returncode, self.stdout, self.stderr = rc, stdout, stderr


class DrainTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self._prev_root = os.environ.get("JAVIS_ROOT")
        os.environ["JAVIS_ROOT"] = self.tmp.name
        os.environ["JAVIS_WAKEUP_LIVENESS"] = "alive"        # zombie 가드 통과
        for mod in list(sys.modules):
            if mod == "javis_wakeup":
                del sys.modules[mod]
        import javis_wakeup                                   # noqa: E402  (경로 env 반영 후 로드)
        self.W = javis_wakeup
        self._orig_run = javis_wakeup.subprocess.run
        self.calls = []

    def tearDown(self):
        self.W.subprocess.run = self._orig_run
        os.environ.pop("JAVIS_WAKEUP_LIVENESS", None)
        if self._prev_root is None:
            os.environ.pop("JAVIS_ROOT", None)
        else:
            os.environ["JAVIS_ROOT"] = self._prev_root
        self.tmp.cleanup()

    # ── 헬퍼 ──
    def enqueue(self, task="t1", target="master"):
        a = type("A", (), {"to": target, "task": task, "reason": "r",
                           "payload": None, "idempotency_key": None})()
        with redirect_stdout(io.StringIO()):
            self.W.cmd_enqueue(a)

    def fake_send(self, rc, stderr=""):
        def _run(cmd, **kw):
            self.calls.append(cmd)
            return FakeCompleted(rc, "", stderr)
        self.W.subprocess.run = _run

    def drain(self):
        out, err = io.StringIO(), io.StringIO()
        with redirect_stdout(out), redirect_stderr(err):
            rc = self.W.cmd_drain(Args())
        return rc, out.getvalue(), err.getvalue()

    def ledger(self):
        with open(self.W.LEDGER, encoding="utf-8") as f:
            return [json.loads(ln) for ln in f if ln.strip()]

    def pending_count(self):
        d = self.W.PENDING_DIR
        return len([n for n in os.listdir(d) if n.endswith(".json")]) if os.path.isdir(d) else 0

    # ── 판별 함수 ──
    def test_softcap_detector(self):
        self.assertTrue(self.W.is_softcap_rejection("error: queue_softcap_exceeded"))
        self.assertTrue(self.W.is_softcap_rejection("", "queue_softcap_exceeded (dead-letter 기록)"))
        self.assertFalse(self.W.is_softcap_rejection("surface not found", ""))
        self.assertFalse(self.W.is_softcap_rejection(None, None))

    # ── 소프트캡 = 종결 ──
    def test_softcap_is_terminal_not_failure(self):
        self.enqueue()
        self.fake_send(1, "error: queue_softcap_exceeded — 재전송하지 마라")
        rc, out, err = self.drain()
        self.assertEqual(rc, self.W.EXIT_OK)
        self.assertEqual(json.loads(out.strip().splitlines()[-1]),
                         {"delivered": 0, "skipped": 1})
        self.assertEqual(self.pending_count(), 0, "소프트캡 종결인데 pending 잔존")
        why = [e.get("why") for e in self.ledger() if e["event"] == "skipped"]
        self.assertEqual(why, ["dead-lettered(softcap)"])
        self.assertEqual(self.W._load_failcount(), {},
                         "소프트캡이 fast-fail 카운터를 오염시켰다")

    # ── 그 외 실패 = 기존 G13 ──
    def test_generic_failure_keeps_pending_and_bumps_counter(self):
        self.enqueue()
        self.fake_send(1, "send failed: surface not found")
        rc, out, err = self.drain()
        self.assertEqual(self.pending_count(), 1, "일반 실패인데 pending 삭제됨")
        self.assertEqual(self.W._load_failcount().get("master"), 1)
        self.assertIn("deliver failed", err)
        self.assertIn("surface not found", err)

    def test_generic_failure_fast_fails_at_threshold(self):
        self.enqueue()
        self.fake_send(1, "boom")
        for _ in range(3):
            self.drain()
        self.assertEqual(self.pending_count(), 0, "3회 연속 실패인데 fast-fail 미발동")
        whys = [e.get("why") for e in self.ledger() if e["event"] == "skipped"]
        self.assertTrue(any("fast-fail" in (w or "") for w in whys), whys)
        self.assertEqual(self.W._load_failcount().get("master"), 0, "fast-fail 후 카운터 미리셋")

    # ── 성공 경로 무회귀 ──
    def test_success_delivers_and_resets_counter(self):
        self.enqueue()
        self.fake_send(1, "boom")
        self.drain()
        self.assertEqual(self.W._load_failcount().get("master"), 1)
        self.fake_send(0)
        rc, out, err = self.drain()
        self.assertEqual(json.loads(out.strip().splitlines()[-1]),
                         {"delivered": 1, "skipped": 0})
        self.assertEqual(self.pending_count(), 0)
        self.assertEqual(self.W._load_failcount().get("master"), 0)
        self.assertIn("--queued", self.calls[-1], "배달 명령이 --queued 단일경로가 아니다")


if __name__ == "__main__":
    unittest.main(verbosity=2)
