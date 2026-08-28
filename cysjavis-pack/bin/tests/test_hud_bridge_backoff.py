#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_hud_bridge_backoff.py — 구독 자식 재수립 지수 백오프(W2) 회귀.

이 스위트가 지키는 것 (IMPL-SPEC W2 — replay_gap 무한 재스폰 스톰 차단)
  ① 급속 재종료는 지수 증가: 2 → 4 → 8 → … → 상한 60s (고정 2초 재스폰 스톰 금지)
  ② 상한 불변식 — 어떤 이력에서도 cap 을 넘지 않는다
  ③ 안정 생존(>= 30s) 후 종료는 정상 회전 — 백오프가 바닥(2s)으로 초기화
     (데몬 rotate 등 정상 재수립을 스톰으로 벌하지 않는다)
  ④ 첫 재수립·경계값·비정상 입력(음수 생존)에서도 대기가 0/음수로 새지 않는다
  ⑤ 정책 상수 자체 핀 — 값이 조용히 0/역전되면 스톰 축이 되살아난다

실행: python3 test_hud_bridge_backoff.py   (unittest·파일 직접 실행 — 저장소 관례 준거)
"""
import os
import sys
import unittest

SELF = os.path.dirname(os.path.abspath(__file__))                        # …/bin/tests
BIN = os.path.dirname(SELF)                                              # cysjavis-pack/bin
sys.path.insert(0, BIN)
import javis_hud_bridge as HB                                            # noqa: E402


class ExponentialBackoff(unittest.TestCase):
    """급속 재종료 = 지수 증가 (①·②)."""

    def test_rapid_exit_sequence_doubles_to_cap(self):
        # 즉사 반복(구 CLI replay_gap 종료 시나리오): 2 → 4 → 8 → 16 → 32 → 60(cap) → 60 …
        seq, prev = [], None
        for _ in range(8):
            prev = HB.next_sub_backoff(prev, alive_secs=0.1)
            seq.append(prev)
        self.assertEqual(seq, [2.0, 4.0, 8.0, 16.0, 32.0, 60.0, 60.0, 60.0])

    def test_cap_is_never_exceeded(self):
        prev = None
        for _ in range(50):
            prev = HB.next_sub_backoff(prev, alive_secs=0.0)
            self.assertLessEqual(prev, HB.SUB_BACKOFF_CAP_SECS)
            self.assertGreaterEqual(prev, HB.SUB_BACKOFF_SECS)

    def test_monotonic_nondecreasing_under_rapid_exits(self):
        prev = HB.next_sub_backoff(None, 0.0)
        for _ in range(20):
            nxt = HB.next_sub_backoff(prev, 1.0)
            self.assertGreaterEqual(nxt, prev)
            prev = nxt


class StableReset(unittest.TestCase):
    """안정 생존 후 초기화 (③) — 스톰 차단이 정상 회전까지 벌하지 않게."""

    def test_stable_life_resets_to_base(self):
        self.assertEqual(HB.next_sub_backoff(60.0, alive_secs=3600.0),
                         HB.SUB_BACKOFF_SECS)

    def test_reset_boundary_is_inclusive(self):
        # 정확히 stable 경계 = 초기화 (>= 계약) · 경계 바로 밑은 여전히 급속(지수 유지)
        self.assertEqual(
            HB.next_sub_backoff(32.0, alive_secs=HB.SUB_STABLE_RESET_SECS),
            HB.SUB_BACKOFF_SECS)
        self.assertEqual(
            HB.next_sub_backoff(32.0, alive_secs=HB.SUB_STABLE_RESET_SECS - 0.001),
            60.0)   # min(32×2, cap 60)

    def test_after_reset_storm_restarts_from_base(self):
        # 안정 → 초기화(2) → 즉사 재개면 4 부터 다시 지수 — 초기화가 면죄부가 아니다
        b = HB.next_sub_backoff(60.0, 999.0)
        self.assertEqual(b, 2.0)
        self.assertEqual(HB.next_sub_backoff(b, 0.5), 4.0)


class EdgeInputs(unittest.TestCase):
    """④·⑤ 첫 재수립·비정상 입력·정책 상수 방어."""

    def test_first_respawn_starts_at_base(self):
        # 첫 재수립은 생존 시간과 무관하게 바닥 — 종전(고정 2s)과 대기 동일 = 하위호환
        self.assertEqual(HB.next_sub_backoff(None, 0.0), HB.SUB_BACKOFF_SECS)
        self.assertEqual(HB.next_sub_backoff(None, 9999.0), HB.SUB_BACKOFF_SECS)

    def test_negative_alive_is_rapid_not_crash(self):
        # 시계 이상(음수 생존)도 급속으로 취급 — 예외·0초 대기로 새지 않는다
        self.assertEqual(HB.next_sub_backoff(2.0, -5.0), 4.0)

    def test_policy_constants_are_sane(self):
        self.assertGreater(HB.SUB_BACKOFF_SECS, 0)
        self.assertGreater(HB.SUB_BACKOFF_CAP_SECS, HB.SUB_BACKOFF_SECS)
        self.assertGreater(HB.SUB_STABLE_RESET_SECS, 0)


if __name__ == "__main__":
    unittest.main(verbosity=2)
