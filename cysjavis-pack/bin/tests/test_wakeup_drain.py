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
        """★B6②: 종전 단언은 마지막 `pending_count()==0` 하나뿐이라, **enqueue 가 아예 안 됐어도**
        통과했다(0을 0으로 확인). 1건 선주입을 명시 확인하고, 임계 직전까지 그 1건이
        **유지**되는지(조기 폐기 없음)까지 못박는다."""
        self.enqueue()
        self.assertEqual(self.pending_count(), 1, "전제: 1건이 선주입돼야 한다")
        self.fake_send(1, "boom")
        for i in (1, 2):
            self.drain()
            self.assertEqual(self.pending_count(), 1,
                             f"{i}회 실패(임계 3 미만)인데 pending 이 조기 폐기됐다")
            self.assertEqual(self.W._load_failcount().get("master"), i, "연속 실패 계수 오류")
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
        # ★B1: 배달에 데몬 멱등키(=task_key)를 실어 C4 제자리 병합을 태운다.
        #      (부수효과: 이 경로가 keyless 통계를 오염시키던 문제도 해소)
        cmd = self.calls[-1]
        self.assertIn("--idempotency-key", cmd, "배달에 데몬 멱등키가 실리지 않았다")
        self.assertEqual(cmd[cmd.index("--idempotency-key") + 1], "t1",
                         "멱등키는 task_key 여야 같은 주제 wakeup 이 제자리 병합된다")


    # ── B1: 구 데몬 호환 폴백 ──
    def test_idem_key_unsupported_detector(self):
        W = self.W
        self.assertTrue(W.idem_key_unsupported(
            "error: unexpected argument '--idempotency-key' found"))
        self.assertTrue(W.idem_key_unsupported("", "unrecognized option --idempotency-key"))
        self.assertFalse(W.idem_key_unsupported("send failed: surface not found", ""),
                         "무관 실패에 폴백이 오발동하면 배달이 두 번 나간다")
        self.assertFalse(W.idem_key_unsupported(None, None))

    def test_old_daemon_falls_back_to_keyless_send_once(self):
        """구 CLI/구 데몬이 플래그를 모르면 **플래그 없이 1회만** 재전송한다 —
        배달이 병합 최적화보다 우선이고, 재시도 폭풍은 만들지 않는다."""
        self.enqueue()
        seq = []

        def _run(cmd, **kw):
            self.calls.append(cmd)
            seq.append(list(cmd))
            if "--idempotency-key" in cmd:
                return FakeCompleted(2, "", "error: unexpected argument '--idempotency-key' found")
            return FakeCompleted(0, "", "")

        self.W.subprocess.run = _run
        rc, out, err = self.drain()
        self.assertEqual(json.loads(out.strip().splitlines()[-1]),
                         {"delivered": 1, "skipped": 0}, "폴백 배달이 성공으로 집계되지 않았다")
        self.assertEqual(len(seq), 2, "폴백은 정확히 1회여야 한다: %s" % seq)
        self.assertIn("--idempotency-key", seq[0])
        self.assertNotIn("--idempotency-key", seq[1])
        self.assertEqual(self.pending_count(), 0)
        self.assertEqual(self.W._load_failcount(), {}, "폴백 성공이 실패로 계수됐다")


if __name__ == "__main__":
    unittest.main(verbosity=2)
