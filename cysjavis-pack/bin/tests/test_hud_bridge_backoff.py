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
  ⑦ ★(0.14.43 · R1F-PK · S3 minor 1) **이벤트 한 건의 처리 예외가 구독 스레드를 죽이지 못한다** — `_reader` 가 줄 하나를 처리하다 예외를
     만나도 그 이벤트만 건너뛰고 다음 줄을 처리한다(종전엔 예외 하나로 스레드가 끝나고 `reconcile_targets` 는 스레드 생존을 보지 않아 다시
     띄우지도 않았다). 의도된 탈출 경로(종료 신호 `stop` · KeyboardInterrupt/SystemExit)는 삼키지 않는다. 삼킨 사실은 stderr 한 줄(같은
     예외 형 연속은 첫 1회만).

실행: python3 test_hud_bridge_backoff.py   (unittest·파일 직접 실행 — 저장소 관례 준거)
"""
import contextlib
import io
import json
import os
import shutil
import sys
import tempfile
import types
import unittest
from unittest import mock

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


# ── ⑦ 이벤트 한 건의 예외 격리 — _reader 가 줄 단위로 예외를 가두는가 (R1F-PK · S3 minor 1) ──


class _EventProc:
    """`cys events` 자식 스텁 — 지정한 줄들을 내고 EOF. terminate 호출 수를 센다."""

    def __init__(self, lines):
        self.stdout = iter(lines)
        self.terminated = 0

    def terminate(self):
        self.terminated += 1

    def wait(self, timeout=None):
        return 0

    def poll(self):
        return 0


class _Hub:
    """HUB 대역 — publish 된 프레임을 기록하고, 선택적으로 훅을 부른다."""

    def __init__(self, on_publish=None):
        self.frames = []
        self._on = on_publish

    def publish(self, frame):
        self.frames.append(frame)
        if self._on:
            self._on(frame)


def ev_line(seq, name="probe", **payload):
    """데몬 `events` 한 줄(JSON) — `type:"event"` 만 처리 대상이다."""
    return json.dumps({"type": "event", "seq": seq, "name": name, "timestamp": HB.time.time(),
                       "surface_id": 3, "payload": payload}) + "\n"


class ReaderEventFaultIsolation(unittest.TestCase):
    """`SubscriptionSupervisor._reader` 의 이벤트 단위 예외 격리 — 즉사형 자식 스텁(줄 N개 뒤 EOF)으로 루프를 1회 돈다.

    실제 sleep 0초: `_RecordingStop` 이 첫 대기 기록에서 스스로 set 되어 루프를 결정론으로 끝낸다. archive_fx 는 디스크(`state/`)를 쓰므로
    기록형 대역으로 바꾼다(라이브·저장소 무접촉).
    """

    def run_reader(self, lines, route=None, hub=None, stop=None, stderr=None):
        """→ (hub, world, proc, stderr_text). `route` 가 있으면 route_event 를 그것으로 바꾼다."""
        hub = hub or _Hub()
        world = types.SimpleNamespace(seq=0)
        state_dir = tempfile.mkdtemp(prefix="hud-evt-fault-")
        self.addCleanup(shutil.rmtree, state_dir, True)
        sup = HB.SubscriptionSupervisor(world=world, hub=hub, coal=HB.Coalescer(), poke=types.SimpleNamespace(set=lambda: None),
                                        state_dir=state_dir)
        stop = stop or _RecordingStop(n=1)
        proc = _EventProc(lines)
        err = stderr if stderr is not None else io.StringIO()
        archived = []
        with contextlib.ExitStack() as st:
            st.enter_context(mock.patch.object(HB.subprocess, "Popen", lambda *a, **k: proc))
            st.enter_context(mock.patch.object(HB, "archive_fx", lambda ts, fr: archived.append(fr)))
            if route is not None:
                st.enter_context(mock.patch.object(HB, "route_event", route))
            st.enter_context(contextlib.redirect_stderr(err))
            sup._reader("wiring-test", None, stop, [None])
        self.archived = archived
        return hub, world, proc, (err.getvalue() if hasattr(err, "getvalue") else "")

    @staticmethod
    def evt_logs(text):
        return [l for l in text.splitlines() if "이벤트 처리 예외" in l]

    def test_exception_in_one_event_does_not_kill_the_subscription(self):
        # 둘째 이벤트(seq=2)의 처리가 예외 — 첫째·셋째·넷째는 처리된다(종전엔 둘째에서 스레드가 끝났다).
        def route(ev, world, coal, slug="main", now=None):
            if ev["seq"] == 2:
                raise RuntimeError("route boom")
            return [{"t": "fx", "kind": "probe", "n": ev["seq"]}], False
        hub, world, proc, err = self.run_reader([ev_line(1), ev_line(2), ev_line(3), ev_line(4)], route=route)
        self.assertEqual([f["n"] for f in hub.frames], [1, 3, 4], "예외 이벤트만 건너뛰고 나머지는 처리돼야 한다")
        self.assertEqual(world.seq, 4, "예외 뒤 이벤트의 seq 반영이 끊겼다")
        self.assertEqual([f["n"] for f in self.archived], [1, 3, 4], "fx 보관(archive_fx)도 건너뛴 이벤트만 빠진다")
        logs = self.evt_logs(err)
        self.assertEqual(len(logs), 1, "삼킨 사실은 stderr 한 줄이어야 한다: %r" % err)
        self.assertTrue(logs[0].startswith("[hud-bridge] "), logs[0])
        self.assertIn("sub=wiring-test", logs[0])
        self.assertIn("RuntimeError: route boom", logs[0])
        self.assertEqual(proc.terminated, 1, "자식 정리(finally)는 종전대로 1회")

    def test_exceptions_at_every_stage_are_isolated(self):
        # 처리 단계 어디서 나도 같다 — publish 단계(프레임 직렬화 등) · seq 반영 단계(seq 가 문자열). route_event 단계는 위 검체가 이미 본다.
        def route(ev, world, coal, slug="main", now=None):
            return [{"t": "fx", "kind": "probe", "n": ev["seq"]}], False
        hub = _Hub(on_publish=lambda fr: (_ for _ in ()).throw(KeyError("publish boom")) if fr["n"] == 1 else None)
        h, w, p, err = self.run_reader([ev_line(1), ev_line(2)], route=route, hub=hub)
        self.assertEqual([f["n"] for f in h.frames], [1, 2], "publish 예외 뒤 다음 이벤트가 처리돼야 한다")
        # seq 가 비교 불가 형(문자열)이면 max() 가 TypeError — 그 줄만 버려진다.
        bad = json.dumps({"type": "event", "seq": "x", "name": "probe", "payload": {}}) + "\n"
        h2, w2, p2, err2 = self.run_reader([bad, ev_line(5)], route=route)
        self.assertEqual([f["n"] for f in h2.frames], [5])
        self.assertEqual(w2.seq, 5)
        self.assertIn("TypeError", err2)

    def test_non_dict_json_lines_are_skipped_and_5000_digit_from_is_external(self):
        # JSON 이지만 객체가 아닌 줄(리스트·숫자·문자열·null)은 `ev.get` 에서 AttributeError — 종전엔 스레드가 죽었다. 실제 route_event 로
        # 이어서 5000자리 `from` 이벤트(S3 minor 1 의 원 재현)가 '외부'(None) 프레임으로 나오는지까지 본다.
        long_from = ev_line(9, name="surface.input_injected", **{"from": "9" * 5000, "bytes": 42})
        hub, world, proc, err = self.run_reader(["[1, 2]\n", "5\n", '"x"\n', "null\n", "{broken\n", "\n", long_from])
        self.assertEqual(hub.frames, [{"t": "fx", "kind": "doc", "to": "wiring-test@surface:3", "from": None, "bytes": 42}])
        self.assertEqual(world.seq, 9)
        logs = self.evt_logs(err)
        self.assertEqual(len(logs), 1, "같은 예외 형(AttributeError)의 연속은 첫 1회만 로그: %r" % err)
        self.assertIn("AttributeError", logs[0])

    def test_log_is_deduped_per_consecutive_exception_type(self):
        def route(ev, world, coal, slug="main", now=None):
            kind = ev["payload"].get("k")
            if kind == "rt":
                raise RuntimeError("a")
            if kind == "key":
                raise KeyError("b")
            return [{"t": "fx", "kind": "probe", "n": ev["seq"]}], False
        lines = [ev_line(1, k="rt"), ev_line(2, k="rt"), ev_line(3, k="key"), ev_line(4, k="rt"), ev_line(5)]
        hub, world, proc, err = self.run_reader(lines, route=route)
        logs = self.evt_logs(err)
        self.assertEqual([("RuntimeError" in l, "KeyError" in l) for l in logs], [(True, False), (False, True), (True, False)],
                         "연속 같은 형은 1회 · 형이 바뀌면 다시 1회: %r" % logs)
        self.assertEqual([f["n"] for f in hub.frames], [5])
        self.assertTrue(all("\n" not in l for l in logs))

    def test_log_line_is_single_line_even_for_multiline_messages(self):
        def route(ev, world, coal, slug="main", now=None):
            raise ValueError("줄1\n줄2\r\n" + "x" * 500)
        hub, world, proc, err = self.run_reader([ev_line(1), ev_line(2)], route=route)
        logs = self.evt_logs(err)
        self.assertEqual(len(logs), 1, err)
        self.assertLess(len(logs[0]), 400, "로그 한 줄이 한없이 길어지면 안 된다(메시지 120자 절단)")
        self.assertIn("ValueError: 줄1 줄2 ", logs[0])

    def test_unprintable_exception_and_broken_stderr_do_not_kill_the_reader(self):
        class Nasty(Exception):
            def __str__(self):
                raise RuntimeError("__str__ boom")

        class BrokenErr(io.StringIO):
            # 이벤트 예외 로그 쓰기만 실패시킨다 — 자식 종료 뒤의 재수립 로그(기존 코드 · 이번 판 범위 밖)는 건드리지 않는다.
            def write(self, s):
                if "이벤트 처리 예외" in s:
                    raise OSError("stderr closed")
                return super().write(s)

        def route(ev, world, coal, slug="main", now=None):
            if ev["seq"] == 1:
                raise Nasty()
            return [{"t": "fx", "kind": "probe", "n": ev["seq"]}], False
        # (a) 예외 객체의 str() 이 또 예외 — 로그를 못 남겨도 구독은 산다.
        hub, world, proc, err = self.run_reader([ev_line(1), ev_line(2)], route=route)
        self.assertEqual([f["n"] for f in hub.frames], [2])
        # (b) stderr 쓰기가 실패 — 그것이 구독을 죽이지 못한다.
        hub2, world2, proc2, _e = self.run_reader([ev_line(1), ev_line(2)], route=route, stderr=BrokenErr())
        self.assertEqual([f["n"] for f in hub2.frames], [2])

    def test_stop_signal_is_still_an_exit_path(self):
        # 의도된 탈출 경로 1 — 종료 신호(stop). 첫 이벤트를 처리하는 중 stop 이 서면 다음 줄은 처리하지 않고 빠진다(예외가 아니라 break).
        stop = _RecordingStop(n=99)
        hub = _Hub(on_publish=lambda fr: stop.set())

        def route(ev, world, coal, slug="main", now=None):
            return [{"t": "fx", "kind": "probe", "n": ev["seq"]}], False
        h, w, proc, err = self.run_reader([ev_line(1), ev_line(2), ev_line(3)], route=route, hub=hub, stop=stop)
        self.assertEqual([f["n"] for f in h.frames], [1], "stop 뒤의 이벤트를 처리했다 — 종료 신호가 삼켜졌다")
        # for 문은 다음 줄(seq=2)을 **받은 뒤** stop 을 보고 빠진다 → 읽지 않은 줄은 seq=3 하나. 계속 읽었다면(continue) 0 이다.
        self.assertEqual(len(list(proc.stdout)), 1, "stop 이 선 뒤에도 줄을 계속 읽었다 — 즉시 빠져야 한다(break · reap 이 자식을 죽여 깨우는 계약)")
        self.assertEqual(stop.waits, [], "stop 이 선 뒤 재수립 대기로 가면 안 된다")
        self.assertEqual(proc.terminated, 1)
        self.assertEqual(self.evt_logs(err), [])

    def test_base_exceptions_are_not_swallowed(self):
        # 의도된 탈출 경로 2 — KeyboardInterrupt·SystemExit(BaseException)은 `except Exception` 에 걸리지 않고 그대로 나간다(자식 정리 finally 는 돈다).
        for exc in (KeyboardInterrupt, SystemExit):
            def route(ev, world, coal, slug="main", now=None, _exc=exc):
                raise _exc()
            with self.subTest(exc=exc.__name__):
                state_dir = tempfile.mkdtemp(prefix="hud-evt-base-")
                self.addCleanup(shutil.rmtree, state_dir, True)
                sup = HB.SubscriptionSupervisor(world=types.SimpleNamespace(seq=0), hub=_Hub(), coal=HB.Coalescer(),
                                                poke=types.SimpleNamespace(set=lambda: None), state_dir=state_dir)
                proc = _EventProc([ev_line(1), ev_line(2)])
                with mock.patch.object(HB.subprocess, "Popen", lambda *a, **k: proc), \
                        mock.patch.object(HB, "archive_fx", lambda ts, fr: None), \
                        mock.patch.object(HB, "route_event", route), \
                        contextlib.redirect_stderr(io.StringIO()):
                    with self.assertRaises(exc):
                        sup._reader("wiring-test", None, _RecordingStop(n=1), [None])
                self.assertEqual(proc.terminated, 1, "BaseException 이 나가도 자식 정리(finally)는 돌아야 한다")

    def test_json_decode_failures_stay_silent(self):
        # 기존 동작 보존 — JSON 이 아닌 줄은 로그 없이 건너뛴다(예외 로그가 노이즈로 늘지 않는다).
        hub, world, proc, err = self.run_reader(["not json\n", "{\n", "\n"] + [ev_line(7)],
                                                route=lambda ev, world, coal, slug="main", now=None: ([{"t": "fx", "n": ev["seq"]}], False))
        self.assertEqual([f["n"] for f in hub.frames], [7])
        self.assertEqual(self.evt_logs(err), [])


if __name__ == "__main__":
    unittest.main(verbosity=2)
