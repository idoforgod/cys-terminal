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
  ⑥ ★배선 통합(R2 라운드2): _reader 루프가 next_sub_backoff 를 **실제로 소비**하는가 —
     순수 함수 핀(①~⑤)만으로는 호출줄을 `backoff = SUB_BACKOFF_SECS`(수리 전 코드 그대로)
     로 되돌려도 초록이었다(M7 변이 실측: 9/9 OK). 즉사 Popen 스텁 + 기록형 stop.wait 로
     루프가 실제로 잔 대기 수열 [2,4,8]·안정 후 [.,.,2,4] 리셋을 잰다(오너 앵커 ① 재스폰
     스톰의 배선층 재발 차단).

실행: python3 test_hud_bridge_backoff.py   (unittest·파일 직접 실행 — 저장소 관례 준거)
"""
import os
import sys
import tempfile
import types
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


# ── ⑥ 배선 통합 — _reader 루프가 지수 백오프를 실제로 소비하는가 (R2 라운드2) ──


class _InstantDeadProc:
    """즉사하는 `cys events` 자식 스텁 — stdout 즉시 EOF(구 CLI replay_gap 종료 모사)."""

    def __init__(self, *args, **kwargs):
        self.stdout = iter(())          # 라인 0개 = 즉시 EOF

    def terminate(self):
        pass

    def wait(self, timeout=None):
        return 1

    def poll(self):
        return 1


class _RecordingStop:
    """threading.Event 대역 — `stop.wait(backoff)` 로 넘어온 대기값을 기록하고,
    n 회째 기록에서 스스로 set 되어 루프를 결정론으로 종료시킨다(실제 sleep 0초)."""

    def __init__(self, n):
        self.waits = []
        self._n = n
        self._set = False

    def is_set(self):
        return self._set

    def set(self):
        self._set = True

    def wait(self, timeout=None):
        self.waits.append(timeout)
        if len(self.waits) >= self._n:
            self._set = True
        return self._set


class _FakeClock:
    """time.monotonic 대역 — 미리 짠 값 수열을 차례로 돌려준다(호출 2회/반복: born·alive)."""

    def __init__(self, alive_secs_per_iter):
        vals, t = [], 0.0
        for alive in alive_secs_per_iter:
            vals.append(t)              # born = monotonic()
            vals.append(t + alive)      # alive = monotonic() - born
            t += alive + 100.0
        self._vals = iter(vals)

    def __call__(self):
        return next(self._vals)


class ReaderLoopWiring(unittest.TestCase):
    """_reader(구독 리더 루프)의 대기 수열을 직접 잰다 — M7 변이(호출줄을 고정 2s 로 되돌림)
    는 여기서 [2,2,2] 가 되어 즉시 붉는다."""

    def _run_reader(self, alive_secs_per_iter):
        sup = HB.SubscriptionSupervisor(
            world=types.SimpleNamespace(seq=0), hub=None, coal=None, poke=None,
            state_dir=tempfile.mkdtemp(prefix="hud-backoff-wiring-"))
        stop = _RecordingStop(n=len(alive_secs_per_iter))
        orig_popen, orig_mono = HB.subprocess.Popen, HB.time.monotonic
        HB.subprocess.Popen = _InstantDeadProc
        HB.time.monotonic = _FakeClock(alive_secs_per_iter)
        try:
            sup._reader("wiring-test", None, stop, [None])
        finally:
            HB.subprocess.Popen = orig_popen
            HB.time.monotonic = orig_mono
        return stop.waits

    def test_three_rapid_exits_sleep_exponentially(self):
        # 즉사 3연속 → 루프가 실제로 잔 대기 = [2, 4, 8] (수리 전 배선은 [2, 2, 2]).
        self.assertEqual(self._run_reader([0.1, 0.1, 0.1]),
                         [2.0, 4.0, 8.0])

    def test_stable_life_resets_the_wired_backoff(self):
        # 즉사 2회 → 안정 생존 1회(≥ stable) → 즉사 재개: [2, 4, 2(리셋), 4].
        stable = HB.SUB_STABLE_RESET_SECS + 1.0
        self.assertEqual(self._run_reader([0.1, 0.1, stable, 0.1]),
                         [2.0, 4.0, 2.0, 4.0])


if __name__ == "__main__":
    unittest.main(verbosity=2)
