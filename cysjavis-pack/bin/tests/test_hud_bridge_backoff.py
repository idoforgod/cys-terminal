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
  ⑧ ★(0.14.49 · T-49-3) **구독 스레드 사망 수리** — 윈도우(로케일이 cp949·cp1252)에서 한글 이벤트 한 줄이 읽기 단계(`for line in proc.stdout`)에서
     UnicodeDecodeError 를 일으켜 리더 스레드가 영구히 끝나던 결함. 진짜 파이프로 한글·손상 바이트를 읽는 시험, 읽기 단계 예외의 지수 재수립,
     끝난 스레드의 한정 재시작(2→60s 지수 · 600s 살아야 초기화), `/health` 의 `subs`, 다섯 자식 읽기 자리의 인코딩 명시(AST 점검).
     손상(UTF-8 아닌) 바이트가 든 줄·스냅샷은 **버린다**(설계 v2 D2 — 라우팅하지 않음). 종전의 "errors=replace 로 라우팅" 문면은 폐기됐다.

실행: python3 test_hud_bridge_backoff.py   (unittest·파일 직접 실행 — 저장소 관례 준거)
"""
import ast
import contextlib
import io
import json
import locale
import os
import shutil
import sys
import tempfile
import threading
import time
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
            world=types.SimpleNamespace(seq=0), hub=None, coal=None, poke=types.SimpleNamespace(set=lambda: None),   # 0.14.49: 재수립마다 poke.set() 을 부른다
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


# ── ⑧ 0.14.49 · 구독 스레드 사망 수리 (T-49-3 · 설계 DESIGN-49-v2 §7) ──────────────────────────────────────────────

_REAL_POPEN = HB.subprocess.Popen
_FAKE_CYS = "FAKE-CYS"      # HB.CYS 자리에 세우는 표지 — 아래 Popen 대역이 `python 가짜cys.py` 로 바꿔 끼운다(윈도우에서도 shebang 불필요)

# 가짜 `cys` — 진짜 자식 프로세스이고 진짜 파이프로 바이트를 쓴다(스텁 이터레이터가 아니다: 디코딩은 Popen 의 텍스트 래퍼가 한다).
_FAKE_CYS_SRC = r"""
import json, os, sys, time
w = sys.stdout.buffer
a = sys.argv[1:]
if "events" in a:
    for h in json.loads(os.environ.get("T49_LINES", "[]")):
        w.write(bytes.fromhex(h) + b"\n"); w.flush(); time.sleep(0.05)
    tail = os.environ.get("T49_TAIL", "")
    if tail:
        w.write(bytes.fromhex(tail)); w.flush()
    time.sleep(float(os.environ.get("T49_HOLD", "20")))
elif "read-screen" in a:
    w.write("한글 화면 줄 ready\n".encode("utf-8"))
else:
    w.write(bytes.fromhex(os.environ.get("T49_JSON", "7b7d")) + b"\n")
"""


def _ev(name, cat, payload, seq):
    o = {"_flen": 0, "_pv": 1, "type": "event", "seq": seq, "name": name, "category": cat, "timestamp": 1791564757.7,
         "surface_id": None, "payload": payload}
    return json.dumps(o, sort_keys=True, ensure_ascii=False, separators=(",", ":")).encode("utf-8")


# 윈도우 로그에서 재구성한 틀(키 순서·머리·한글 본문) — 0.14.48 조사에서 쓴 세 줄과 같은 꼴이다. 뒤따르는 AFTER(557)가 "다음 이벤트가 처리되는가"의 표지.
F_ACK = (b'{"_flen":0,"_pv":1,"heartbeat_interval_seconds":15,"latest_seq":553,"ok":true,"resume":{"after_seq":null,"gap":false,'
         b'"latest_seq":553,"next_seq":554,"oldest_seq":513},"type":"ack"}')
F_ASCII = _ev("surface.created", "surface", {"cmd": "cmd.exe", "cwd": "C:\\Users\\runner", "pid": 6084, "role": None, "surface_ref": "surface:5"}, 554)
_TICK_OUT = json.dumps({"result": "skip", "roles": [{"role": "worker", "pass": False, "reason": "대상 유휴: role=worker surface 부재; 미해결 발화(clear 가드)·측정 유효: "
                        "phase=부재(구 데몬 — fail-closed) fire_id=None executed=False · 측정 ok=False reason=x ctx_pct=None"}],
                        "sweep": {}, "gate_exit": 3}, ensure_ascii=False) + "\n"
F_TICK = _ev("schedule.command_done", "schedule", {"job_id": "cycle-autopilot-tick", "exit": 0, "stdout_tail": _TICK_OUT}, 555)
F_WARN = _ev("schedule.warning", "schedule", {"kind": "retired-seed-text", "job_id": "fleet-adoption-cost-digest", "detail":
             "[cysd] schedule: 잡 'fleet-adoption-cost-digest' 는 은퇴한 시드 문구다 — 서명 승인이 없으면 실행하지 않는다(오류로 세지 않는다). 지우려면: cys schedule remove x"}, 556)
F_FEED = _ev("feed.item.created", "feed", {"request_id": "x", "kind": "ceo-notice", "title": "CEO 승격 보류(부트 필요)", "body":
             "부서가 생성되었으나 base master가 아직 부트되지 않았습니다. base에서 '너는 마스터다' 또는 '마스터 시작' 버튼으로 부트를 완료하면 명령 팔레트의 'CEO 승격 진행'으로 승인할 수 있습니다.",
             "wait": False, "tier": "d", "auto_route": False}, 558)
F_AFTER = _ev("pane.idle", "pane", {"idle_secs": 303, "surface_ref": "surface:3"}, 557)
F_BAD1 = b'{"type":"event","seq":600,"name":"x","payload":{"t":"\xff\xfe raw"}}'                       # 문자열 안의 UTF-8 아닌 바이트
_i = next(i for i, b in enumerate(F_TICK) if b >= 0x80)
F_BAD2 = F_TICK[:_i + 1]                                                                             # 여러 바이트 글자의 머리 바이트에서 잘린 줄
F_FFFD = '{"type":"event","seq":601,"name":"x","payload":{"t":"ok \ufffd ok 한글"}}'.encode("utf-8")  # 진짜 U+FFFD 가 든 유효한 줄


def _kor(text):
    return sum(1 for c in text if "\uac00" <= c <= "\ud7a3")


class _Rig:
    """가짜 cys 자식 + 진짜 파이프 + 진짜 SubscriptionSupervisor. emu 가 있으면 "인코딩을 이름 붙이지 않은 text=True 호출"만 그 인코딩을 기본으로 받는다
    (로케일 기본값이 cp949·cp1252 인 윈도우를 모사 — 이름을 붙인 호출은 건드리지 않으므로 수리된 코드는 이 대역의 도움을 받지 못한다)."""

    def __init__(self, tc, emu=None, lines=(), tail=b"", hold=20, json_bytes=None):
        self.tc, self.emu = tc, emu
        self.dir = tempfile.mkdtemp(prefix="hud-t49-")
        tc.addCleanup(shutil.rmtree, self.dir, True)
        self.script = os.path.join(self.dir, "fakecys.py")
        with open(self.script, "w", encoding="utf-8") as f:
            f.write(_FAKE_CYS_SRC)
        self.seen, self.err, self.texc = [], io.StringIO(), []
        self.world = types.SimpleNamespace(seq=0, dept_targets=lambda: {}, daemon={})
        self.poke = threading.Event()
        self.sup = HB.SubscriptionSupervisor(self.world, _Hub(), HB.Coalescer(), self.poke, self.dir)
        rig = self

        class FakePopen(_REAL_POPEN):
            def __init__(p, args, *a, **k):
                if isinstance(args, (list, tuple)) and args and args[0] == _FAKE_CYS:
                    args = [sys.executable, rig.script] + list(args[1:])
                if rig.emu and (k.get("text") or k.get("universal_newlines")) and "encoding" not in k:
                    k["encoding"] = rig.emu
                super().__init__(args, *a, **k)

        env = {"T49_LINES": json.dumps([l.hex() for l in lines]), "T49_TAIL": tail.hex(), "T49_HOLD": str(hold)}
        if json_bytes is not None:
            env["T49_JSON"] = json_bytes.hex()
        self.stack = contextlib.ExitStack()
        st = self.stack
        st.enter_context(mock.patch.object(HB, "CYS", _FAKE_CYS))
        st.enter_context(mock.patch.object(HB.subprocess, "Popen", FakePopen))
        st.enter_context(mock.patch.object(HB, "route_event", self._route))
        st.enter_context(mock.patch.object(HB, "archive_fx", lambda ts, fr: None))
        st.enter_context(mock.patch.dict(os.environ, env))
        st.enter_context(mock.patch.object(threading, "excepthook", lambda a: self.texc.append("%s: %s" % (a.exc_type.__name__, str(a.exc_value)[:140]))))
        st.enter_context(contextlib.redirect_stderr(self.err))
        tc.addCleanup(self.close)

    def _route(self, ev, world, coal, slug="main", now=None):
        p = json.dumps(ev.get("payload"), ensure_ascii=False)
        self.seen.append({"seq": ev.get("seq"), "kor": _kor(p), "fffd": p.count("\ufffd"),
                          "escapes": sum(1 for c in p if "\udc80" <= c <= "\udcff")})
        return [], False

    def thread(self):
        return self.sup.subs["main"]["thread"]

    def seqs(self):
        return [e["seq"] for e in self.seen]

    def logs(self, needle=None):
        ls = self.err.getvalue().splitlines()
        return [l for l in ls if needle in l] if needle else ls

    def stats(self):
        return dict((getattr(self.sup, "stats", {}) or {}).get("main") or {})

    def run(self, until=None, secs=8.0):
        """리더 스레드를 띄우고 until() 이 참이 되거나 스레드가 끝나거나 secs 가 지날 때까지 기다린다(죽은 스레드를 헛기다리지 않는다)."""
        self.sup._spawn("main", None)
        end = time.monotonic() + secs
        until = until or (lambda: 557 in self.seqs())
        while time.monotonic() < end and not until() and self.thread().is_alive():
            time.sleep(0.02)
        time.sleep(0.15)        # until 직후 같은 읽기 흐름의 나머지 줄·로그가 도착할 틈
        return self

    def close(self):
        for slug in list(self.sup.subs):
            proc = self.sup.subs[slug]["proc"][0]
            self.sup._reap(slug)
            try:
                if proc is not None:
                    proc.kill()
            except Exception:
                pass
        self.stack.close()


class HudBridgeEncodingOnRealPipe(unittest.TestCase):
    """★윈도우 CI 걸음이 번들 python3.exe 로 이 클래스를 돈다. 진짜 자식·진짜 파이프로 한글·손상 바이트를 읽는다(0.14.48 에서는 붉다)."""

    EMUS = ("cp949", "cp1252")

    def test_korean_events_are_routed_and_the_next_event_arrives(self):
        for emu in self.EMUS:
            for name, frame, seq in (("tick", F_TICK, 555), ("warn", F_WARN, 556), ("feed", F_FEED, 558)):
                with self.subTest(default_encoding=emu, frame=name):
                    rig = _Rig(self, emu=emu, lines=[F_ACK, F_ASCII, frame, F_AFTER]).run()
                    try:
                        self.assertTrue(rig.thread().is_alive(), "한글 이벤트 한 줄에 리더 스레드가 끝났다: %s" % rig.texc[:1])
                        self.assertIn(seq, rig.seqs(), "한글 이벤트가 라우팅되지 않았다")
                        row = [e for e in rig.seen if e["seq"] == seq][0]
                        self.assertGreater(row["kor"], 0, "한글이 사라졌다")
                        self.assertEqual((row["fffd"], row["escapes"]), (0, 0), "한글이 깨졌다(대체문자·이스케이프)")
                        self.assertIn(557, rig.seqs(), "한글 이벤트 **다음** 이벤트가 처리되지 않았다")
                    finally:
                        rig.close()

    def test_ascii_positive_control(self):
        for emu in self.EMUS:
            with self.subTest(default_encoding=emu):
                rig = _Rig(self, emu=emu, lines=[F_ACK, F_ASCII, F_AFTER]).run()
                try:
                    self.assertTrue(rig.thread().is_alive())
                    self.assertEqual([s for s in rig.seqs() if s in (554, 557)], [554, 557])
                finally:
                    rig.close()

    def test_native_locale_condition(self):
        # 이 기계의 진짜 기본값 — UTF-8 기계에서는 조건이 성립하지 않으니 조용히 통과하지 않고 그 사실을 찍는다.
        enc = (locale.getpreferredencoding(False) or "?")
        utf8 = sys.flags.utf8_mode or enc.lower().replace("-", "") == "utf8"
        print("[T-49-3] native default encoding seen: %s utf8_mode=%s%s" % (enc, sys.flags.utf8_mode,
              "  -> condition not established (UTF-8 machine); the emulated rows above are the proof here" if utf8 else ""), file=sys.__stderr__)
        rig = _Rig(self, emu=None, lines=[F_ACK, F_TICK, F_AFTER]).run()
        try:
            self.assertTrue(rig.thread().is_alive(), "native: %s %s" % (enc, rig.texc[:1]))
            self.assertIn(555, rig.seqs())
            self.assertIn(557, rig.seqs())
        finally:
            rig.close()

    def test_bytes_that_are_not_utf8_are_dropped_counted_and_logged_once(self):
        for emu in (None, "cp1252"):
            with self.subTest(default_encoding=emu):
                rig = _Rig(self, emu=emu, lines=[F_ACK, F_BAD1, F_BAD2, F_AFTER]).run()
                try:
                    self.assertTrue(rig.thread().is_alive(), "손상 바이트 줄에 스레드가 끝났다: %s" % rig.texc[:1])
                    self.assertNotIn(600, rig.seqs(), "손상 줄이 라우팅됐다(설계 v2 D2: 버린다)")
                    self.assertTrue(all(e["escapes"] == 0 and e["fffd"] == 0 for e in rig.seen), rig.seen)
                    self.assertIn(557, rig.seqs(), "손상 줄 다음 이벤트가 처리되지 않았다")
                    self.assertEqual(rig.stats().get("damaged"), 2)
                    self.assertEqual(len(rig.logs("non-UTF-8")), 1, rig.logs())
                    self.assertTrue(rig.poke.is_set(), "손상 줄은 스냅샷을 요청한다")
                finally:
                    rig.close()

    def test_a_valid_line_with_a_real_u_fffd_is_routed_unchanged(self):
        for emu in (None, "cp1252"):
            with self.subTest(default_encoding=emu):
                rig = _Rig(self, emu=emu, lines=[F_ACK, F_FFFD, F_AFTER]).run()
                try:
                    row = [e for e in rig.seen if e["seq"] == 601]
                    self.assertEqual(len(row), 1, "정상 줄(진짜 U+FFFD 포함)이 버려졌다")
                    self.assertEqual((row[0]["fffd"], row[0]["kor"], row[0]["escapes"]), (1, 2, 0))
                    self.assertEqual(rig.stats().get("damaged", 0), 0)
                finally:
                    rig.close()

    def test_snapshot_korean_is_parsed_and_damaged_snapshot_returns_none(self):
        good = '{"daemon":{"latest_seq":5},"label":"한글 부서 이름"}'.encode("utf-8")
        bad = b'{"daemon":{"latest_seq":5},"label":"\xff\xfe"}'
        for emu in self.EMUS:
            with self.subTest(default_encoding=emu, body="korean"):
                rig = _Rig(self, emu=emu, json_bytes=good)
                try:
                    r = HB.run_json(["status", "--json"])
                    self.assertIsNotNone(r, "한글이 든 스냅샷을 못 읽었다")
                    self.assertEqual(_kor(r["label"]), 6)
                finally:
                    rig.close()
            with self.subTest(default_encoding=emu, body="damaged"):
                rig = _Rig(self, emu=emu, json_bytes=bad)
                try:
                    before = HB._STATS.get("damaged_snapshots", 0)
                    self.assertIsNone(HB.run_json(["status", "--json"]), "손상 바이트 스냅샷을 받아들였다")
                    self.assertEqual(HB._STATS.get("damaged_snapshots", 0), before + 1)
                finally:
                    rig.close()

    def test_screen_text_keeps_its_korean(self):
        for emu in self.EMUS:
            with self.subTest(default_encoding=emu):
                rig = _Rig(self, emu=emu)
                try:
                    rows = HB.peek_surface("main@surface:1")
                    self.assertTrue(rows and "한글" in " ".join(rows), rows)
                finally:
                    rig.close()


class _RaisingProc:
    """읽기 단계가 터지는 자식 — 첫 줄 뒤 `for line in proc.stdout` 의 다음 걸음이 OSError."""
    spawned = 0

    def __init__(self, *a, **k):
        type(self).spawned += 1

        class Out:
            n = 0

            def __iter__(o):
                return o

            def __next__(o):
                o.n += 1
                if o.n == 1:
                    return ev_line(1)
                raise OSError("model: pipe read failed")
        self.stdout = Out()

    def terminate(self):
        pass

    def wait(self, timeout=None):
        return 1

    def poll(self):
        return 1


class ReaderReadStepError(unittest.TestCase):
    """(b1) 읽기 단계 예외는 그 자식만 끝내고 기존 지수 백오프로 다시 세운다 — 스레드를 죽이지 않는다."""

    def test_read_error_goes_through_the_shipped_backoff_and_keeps_retrying(self):
        pokes, err = [], io.StringIO()
        _RaisingProc.spawned = 0
        sup = HB.SubscriptionSupervisor(world=types.SimpleNamespace(seq=0), hub=_Hub(), coal=HB.Coalescer(),
                                        poke=types.SimpleNamespace(set=lambda: pokes.append(1)), state_dir=tempfile.mkdtemp(prefix="hud-t49-rd-"))
        self.addCleanup(shutil.rmtree, sup.state_dir, True)
        stop = _RecordingStop(n=8)
        raised = None
        with mock.patch.object(HB.subprocess, "Popen", _RaisingProc), \
                mock.patch.object(HB, "route_event", lambda *a, **k: ([], False)), \
                mock.patch.object(HB, "archive_fx", lambda ts, fr: None), \
                contextlib.redirect_stderr(err):
            try:
                sup._reader("rd", None, stop, [None])
            except Exception as e:                                  # noqa: BLE001
                raised = "%s: %s" % (type(e).__name__, e)
        self.assertIsNone(raised, "읽기 단계 예외가 _reader 밖으로 나갔다(스레드가 끝난다)")
        self.assertEqual(stop.waits, [2.0, 4.0, 8.0, 16.0, 32.0, 60.0, 60.0, 60.0])
        self.assertEqual(_RaisingProc.spawned, 8, "한 번 재시도하고 멎었다 — 계속 재시도해야 한다")
        self.assertEqual(len([l for l in err.getvalue().splitlines() if "events read error" in l]), 1, err.getvalue())
        self.assertTrue(pokes, "다시 세울 때는 스냅샷을 요청한다")


class ThreadRevival(unittest.TestCase):
    """(b2) 끝난 스레드의 한정 재시작. 주입 시계로 1시간(1800 번의 reconcile) — 실제 sleep 없음(스레드 종료는 join 으로 결정론)."""

    PASSES, STEP = 1800, 2.0

    def simulate(self, life):
        """life: 스레드가 사는 모의 초(0=즉사 · None=살아서 막혀 있음)."""
        clock = {"t": 1000.0}
        starts, live = [], []                               # live: [(시작 모의시각, 문, 스레드)]
        world = types.SimpleNamespace(seq=0, dept_targets=lambda: {}, daemon={})
        sup = HB.SubscriptionSupervisor(world, _Hub(), HB.Coalescer(), threading.Event(), tempfile.mkdtemp(prefix="hud-t49-rev-"))
        self.addCleanup(shutil.rmtree, sup.state_dir, True)

        def reader(slug, socket, stop, holder):
            g = threading.Event()
            starts.append(clock["t"])
            live.append([clock["t"], g, None])
            if life == 0:
                raise RuntimeError("model: reader dies at once")
            g.wait(30)
            if life is not None and not stop.is_set():
                raise RuntimeError("model: reader dies after %ss" % life)
        sup._reader = reader
        same_pass, logs = 0, io.StringIO()
        with mock.patch.object(threading, "excepthook", lambda a: None), contextlib.redirect_stderr(logs):
            for i in range(self.PASSES):
                t = clock["t"]
                for row in live:                            # 살 만큼 산 스레드를 끝내고 끝났음을 확인한다
                    th = row[2]
                    if th is not None and th.is_alive() and life is not None and t - row[0] >= life:
                        row[1].set()
                        th.join(2)
                    elif th is not None and th.is_alive() and life == 0:
                        th.join(2)
                n0 = len(starts)
                dead_before = "main" in sup.subs and not sup.subs["main"]["thread"].is_alive()
                sup.reconcile_once(now=t)
                if "main" in sup.subs and live and live[-1][2] is None:
                    live[-1][2] = sup.subs["main"]["thread"]
                    if life == 0:
                        live[-1][2].join(2)
                if len(starts) != n0 and i > 0 and not dead_before:
                    same_pass += 1
                clock["t"] += self.STEP
            for row in live:
                row[1].set()
            for row in live:                                # 남은 스레드가 훅이 살아 있는 동안 끝나게 한다(끝난 뒤 역추적 소음 방지)
                if row[2] is not None:
                    row[2].join(2)
        rel = [round(s - 1000.0) for s in starts]
        gaps = [b - a for a, b in zip(rel, rel[1:])]
        return {"rel": rel, "gaps": gaps, "same_pass": same_pass, "log": logs.getvalue(), "end": self.PASSES * self.STEP}

    def check_continued_progress(self, life, r):
        """C2: 지속 진행 점검은 **생애별**로 정의한다 — 연속한 시작 사이는 `life + 64` 초(= 생애 + 상한 60 + 한 번의 reconcile 주기 2 + 사망 감지 한 걸음 2)를
        넘지 않고, 마지막 시작 뒤 남은 시간도 같은 값을 넘지 않는다. 생애 601 은 주기가 약 604 초라 마지막 300 초가 비어도 이 점검은 참이고,
        한 번 재시도하고 멎은 경우는(남은 시간 ≈ 3598) 어떤 생애에서도 거짓이다."""
        limit = life + 64
        self.assertGreater(len(r["rel"]), 3, "재시도가 이어지지 않았다: %s" % r["rel"][:8])
        self.assertTrue(all(g <= limit for g in r["gaps"]), "시작 사이가 %s 초를 넘었다: %s" % (limit, r["gaps"][:12]))
        self.assertLessEqual(r["end"] - r["rel"][-1], limit, "마지막 시작 뒤 %s 초 넘게 시작이 없다 — 재시도가 멎었다" % limit)

    def test_dies_at_once_restarts_continue_at_the_cap_and_are_bounded(self):
        r = self.simulate(0)
        self.check_continued_progress(0, r)
        self.assertLessEqual(len(r["rel"]), 62)
        late = r["gaps"][len(r["gaps"]) // 2:]
        self.assertGreaterEqual(min(late), 60, "정상 상태에서 62초보다 촘촘히 재시작했다: %s" % late[:10])
        self.assertEqual(r["rel"][:6], [0, 2, 8, 18, 36, 70], "초기 지수 단계(2→4→8→16→32→60) 이탈")
        self.assertEqual(r["same_pass"], 0, "죽음을 처음 본 걸음에서 재시작했다")
        self.assertGreaterEqual(len([l for l in r["log"].splitlines() if "reader thread ended" in l]), len(r["rel"]) - 1, "스레드 사망마다 한 줄")

    def test_lives_30_seconds_pacing_does_not_reset_at_the_child_rule(self):
        r = self.simulate(30)
        self.check_continued_progress(30, r)
        self.assertLessEqual(len(r["rel"]), 62, "30초 사는 스레드가 시간당 %d 번 재시작 — 30초 규칙으로 초기화되고 있다" % len(r["rel"]))
        late = r["gaps"][len(r["gaps"]) // 2:]
        self.assertGreaterEqual(min(late), 60, late[:10])
        self.assertEqual(r["same_pass"], 0)

    def test_lives_601_seconds_resets_pacing_and_continues(self):
        r = self.simulate(601)
        self.check_continued_progress(601, r)
        self.assertGreaterEqual(min(r["gaps"]), 601, "601초 사는 스레드가 자기 수명보다 빨리 재시작됐다: %s" % r["gaps"])
        self.assertEqual(r["same_pass"], 0)

    def test_pacing_resets_to_base_only_after_a_life_of_600_seconds(self):
        """IMPL-R1 M1: 600초 초기화를 **직접** 재는 시험 — 대기를 60초까지 올린 뒤, 599초 산 스레드의 다음 대기는 60초(초기화 없음), 601초 산 스레드의
        다음 대기는 2초(초기화). SUB_REVIVE_STABLE_SECS 를 꺼 버리면(10**9) 601초 칸이 60초가 되어 붉다(생애+64 진행 점검만으로는 못 잡던 변이)."""
        world = types.SimpleNamespace(seq=0, dept_targets=lambda: {}, daemon={})
        sup = HB.SubscriptionSupervisor(world, _Hub(), HB.Coalescer(), threading.Event(), tempfile.mkdtemp(prefix="hud-t49-r6-"))
        self.addCleanup(shutil.rmtree, sup.state_dir, True)
        sup._reader = lambda slug, socket, stop, holder: None            # 시작하자마자 끝나는 스레드
        th = threading.Thread(target=lambda: None)
        th.start()
        th.join()
        sup.subs["main"] = {"stop": threading.Event(), "proc": [None], "thread": th, "socket": None, "born": 1000.0}
        now = 1000.0
        for _ in range(8):                                               # 즉사 반복으로 대기를 2→4→…→60 까지 올린다
            sup.revive_dead(now)
            now = sup.revive["main"]["next_at"]
            sup.revive_dead(now)                                         # 이 회차에 다시 세운다
            sup.subs["main"]["thread"].join(2)
        self.assertEqual(sup.revive["main"]["backoff"], 60.0, "준비 단계: 대기가 상한까지 올라가야 한다")
        born = sup.subs["main"]["born"]
        for life, want in ((599.0, 60.0), (601.0, 2.0)):
            with self.subTest(life=life):
                snap = dict(sup.revive["main"])
                sup.revive["main"].update(next_at=None, backoff=60.0)    # 같은 출발점(대기 60초)에서 각 생애를 잰다
                sup.subs["main"]["born"] = born
                sup.revive_dead(born + life)                             # 죽음을 처음 보는 회차: 다음 대기만 정한다
                self.assertEqual(sup.revive["main"]["backoff"], want)
                self.assertEqual(sup.revive["main"]["next_at"], born + life + want)
                sup.revive["main"].update(snap)

    def test_healthy_blocked_reader_is_never_restarted(self):
        r = self.simulate(None)
        self.assertEqual(len(r["rel"]), 1, "살아서 막혀 있는 리더가 다시 시작됐다(리더 중복 = 이벤트 폭주): %s" % r["rel"])

    def test_a_reaped_subscription_is_never_revived(self):
        world = types.SimpleNamespace(seq=0, dept_targets=lambda: {}, daemon={})
        sup = HB.SubscriptionSupervisor(world, _Hub(), HB.Coalescer(), threading.Event(), tempfile.mkdtemp(prefix="hud-t49-reap-"))
        self.addCleanup(shutil.rmtree, sup.state_dir, True)
        n = {"s": 0}

        def reader(slug, socket, stop, holder):
            n["s"] += 1
            raise RuntimeError("model: dies at once")
        sup._reader = reader
        with mock.patch.object(threading, "excepthook", lambda a: None), contextlib.redirect_stderr(io.StringIO()):
            sup.reconcile_once(now=1000.0)
            sup.subs["main"]["thread"].join(2)
            sup._reap("main")
            for i in range(100):
                sup.revive_dead(1002.0 + 2 * i)
        self.assertEqual(n["s"], 1)
        self.assertNotIn("main", sup.subs)
        self.assertFalse(getattr(sup, "revive", None), "reap 이 재시작 장부를 비우지 않았다")


class SilentReaderObservation(unittest.TestCase):
    """(e) 살아 있지만 조용한 리더 — 관측하고 한 줄 남길 뿐 죽이지 않는다(D1)."""

    def run_case(self, lines, tail=b""):
        self.assertTrue(hasattr(HB.SubscriptionSupervisor, "note_behind"), "note_behind 가 없다")
        rig = _Rig(self, emu="cp1252", lines=lines, tail=tail)
        rig.run(until=(lambda: bool(lines) and (lines[-1] == F_ASCII and 554 in rig.seqs())) if lines else (lambda: False), secs=1.2)
        return rig

    def check(self, rig, partial):
        sup, t0 = rig.sup, time.monotonic()
        last = rig.stats().get("last_seq", 0)
        rig.world.daemon = {"latest_seq": last + 40}
        r1, r2, r3 = sup.note_behind(t0 + 60), sup.note_behind(t0 + 130), sup.note_behind(t0 + 400)
        sup.revive_dead(t0 + 400)
        lv = sup.liveness(t0 + 400)
        rig.world.daemon = {"latest_seq": last}
        r4 = sup.note_behind(t0 + 500)
        self.assertTrue(rig.thread().is_alive(), "조용한 리더가 죽었다/재시작됐다")
        self.assertEqual(rig.stats().get("thread_restarts", 0), 0)
        self.assertIs(r1, False, "120초 전에 표시됐다")
        self.assertIs(r2, True)
        self.assertIs(r3, True)
        self.assertEqual(len(rig.logs("behind the daemon")), 1, "에피소드당 한 줄이 아니다: %s" % rig.logs())
        self.assertIs(r4, False, "따라잡았는데 표시가 안 풀렸다")
        idle = lv["items"]["main"]["idle_s"]
        if partial:
            self.assertIsNone(idle, "줄이 하나도 없었는데 idle_s 가 숫자다")
        else:
            self.assertGreaterEqual(idle, 398)

    def test_reader_silent_after_its_lines(self):
        rig = self.run_case([F_ACK, F_ASCII])
        try:
            self.check(rig, partial=False)
        finally:
            rig.close()

    def test_reader_with_half_a_line_then_silence(self):
        rig = self.run_case([], tail=b'{"type":"event","seq":70,"name":"x","pay')
        try:
            self.check(rig, partial=True)
        finally:
            rig.close()


class HealthSubs(unittest.TestCase):
    """(d) `/health` 의 `subs` — 한정된 객체 · 감독자 배선 · 실패해도 응답."""

    def setUp(self):
        self._had = hasattr(HB, "_SUPERVISOR")
        self._old = getattr(HB, "_SUPERVISOR", None)
        self.addCleanup(self._restore)

    def _restore(self):
        if self._had:
            HB._SUPERVISOR = self._old
        elif hasattr(HB, "_SUPERVISOR"):
            del HB._SUPERVISOR

    def sup_with(self, n, blk, hot=None):
        s = HB.SubscriptionSupervisor(types.SimpleNamespace(seq=0, dept_targets=lambda: {}, daemon={}), _Hub(), HB.Coalescer(),
                                      threading.Event(), tempfile.mkdtemp(prefix="hud-t49-h-"))
        self.addCleanup(shutil.rmtree, s.state_dir, True)
        s._reader = lambda slug, socket, stop, holder: blk.wait(20)
        for i in range(n):
            s._spawn("main" if i == 0 else "dept-with-a-rather-long-slug-name-%04d" % i, None)
        if hot:
            st = s._st(hot)
            st.update(read_errors=10 ** 9, respawns=10 ** 9, thread_restarts=10 ** 9, damaged=10 ** 9, last_seq=10 ** 12)
        return s

    def test_old_keys_stay_and_subs_is_null_without_a_supervisor(self):
        HB._SUPERVISOR = None
        j = json.loads(HB.health_body())
        for k in ("ok", "pid", "boot_id", "pack_version", "assets", "timeouts"):
            self.assertIn(k, j)
        self.assertIn("subs", j)
        self.assertIsNone(j["subs"])

    def test_subs_is_bounded_at_any_subscription_count(self):
        blk = threading.Event()
        self.addCleanup(blk.set)
        sizes = {}
        for n in (1, 12, 64, 500):
            hot = "dept-with-a-rather-long-slug-name-%04d" % (n - 1) if n > 1 else None
            HB._SUPERVISOR = self.sup_with(n, blk, hot)
            body = HB.health_body()
            sizes[n] = len(body)
            j = json.loads(body)["subs"]
            self.assertEqual(j["total"], n)
            self.assertEqual(len(j["items"]), min(n, HB.SUBS_HEALTH_MAX))
            self.assertEqual(j["omitted"], n - len(j["items"]))
            self.assertIn("damaged_snapshots", j)
            if hot:
                self.assertIn(hot[:48], j["items"], "오류가 많은 구독이 한정된 목록에서 밀려났다")
            if n == 1:
                self.assertEqual(sorted(j["items"]["main"]), ["alive", "damaged", "idle_s", "last_seq", "read_errors", "respawns", "thread_restarts"])
        self.assertLess(max(sizes.values()), 8192, sizes)

    def test_health_answers_when_the_snapshot_raises(self):
        class Boom:
            def liveness(self):
                raise RuntimeError("model")
        HB._SUPERVISOR = Boom()
        self.assertIsNone(json.loads(HB.health_body())["subs"])


class DamagedLinePokeIsRateLimited(unittest.TestCase):
    """성찰 1회차 발견: 손상 줄마다 poke 하면 계속 손상된 파이프가 fleet_loop 의 스냅샷(자식 2개 기동)을 연달아 일으킨다 — 줄 수가 아니라 시간으로 한정한다."""

    def test_two_hundred_damaged_lines_poke_at_most_once_per_reconcile_period(self):
        pokes = []
        sup = HB.SubscriptionSupervisor(world=types.SimpleNamespace(seq=0), hub=_Hub(), coal=HB.Coalescer(),
                                        poke=types.SimpleNamespace(set=lambda: pokes.append(1)), state_dir=tempfile.mkdtemp(prefix="hud-t49-pk-"))
        self.addCleanup(shutil.rmtree, sup.state_dir, True)
        bad = '{"type":"event","seq":9,"payload":{"t":"\udcff\udcfe"}}\n'
        proc = _EventProc([bad] * 200 + [ev_line(1)])
        with mock.patch.object(HB.subprocess, "Popen", lambda *a, **k: proc), \
                mock.patch.object(HB, "route_event", lambda *a, **k: ([], False)), \
                mock.patch.object(HB, "archive_fx", lambda ts, fr: None), \
                contextlib.redirect_stderr(io.StringIO()):
            sup._reader("pk", None, _RecordingStop(n=1), [None])
        self.assertEqual(sup.stats["pk"]["damaged"], 200)
        # 손상 줄 쪽 poke 는 한 번, 그 밖에 끝난 자식을 다시 세울 때의 poke 가 한 번 — 200 번이 아니다.
        self.assertLessEqual(len(pokes), 2, "손상 줄마다 poke 했다: %d" % len(pokes))


class SourceCensus(unittest.TestCase):
    """다섯 자식 읽기 자리가 모두 인코딩을 이름 붙였는가(AST) — 호출 4·5(`/cmd`)는 이 점검만이 지킨다."""

    @classmethod
    def setUpClass(cls):
        with open(HB.__file__, encoding="utf-8") as f:
            cls.tree = ast.parse(f.read())

    def sites(self):
        found = {}
        for fn in ast.walk(self.tree):
            if not isinstance(fn, (ast.FunctionDef, ast.AsyncFunctionDef)):
                continue
            for node in ast.walk(fn):
                if isinstance(node, ast.Call) and isinstance(node.func, ast.Attribute) and node.func.attr in ("run", "Popen", "check_output") \
                        and isinstance(node.func.value, ast.Name) and node.func.value.id == "subprocess":
                    kw = {k.arg: k.value for k in node.keywords if k.arg}
                    if "text" in kw or "universal_newlines" in kw:
                        found[(node.lineno, node.col_offset)] = (fn.name, getattr(kw.get("encoding"), "value", None), getattr(kw.get("errors"), "value", None))
        return sorted((k[0], v) for k, v in found.items())

    def test_five_text_mode_child_reads_name_utf8_and_the_right_errors(self):
        rows = self.sites()
        self.assertEqual(len(rows), 5, rows)
        for line, (fn, enc, errs) in rows:
            self.assertEqual(enc, "utf-8", "javis_hud_bridge.py:%d (%s) 가 인코딩을 이름 붙이지 않았다" % (line, fn))
            want = "surrogateescape" if fn in ("_reader", "run_json") else "replace"
            self.assertEqual(errs, want, "javis_hud_bridge.py:%d (%s) errors=%r (기대 %r) — 라우팅·JSON 으로 가는 읽기는 surrogateescape, 화면에만 보이는 읽기는 replace" % (line, fn, errs, want))

    def test_main_hands_the_supervisor_to_health_before_its_thread_starts(self):
        main = next(n for n in self.tree.body if isinstance(n, ast.FunctionDef) and n.name == "main")
        assign = [n for n in ast.walk(main) if isinstance(n, ast.Assign) and any(isinstance(t, ast.Name) and t.id == "_SUPERVISOR" for t in n.targets)]
        self.assertEqual(len(assign), 1, "main() 이 _SUPERVISOR 를 정확히 한 번 대입해야 한다")
        self.assertTrue(any(isinstance(n, ast.Global) and "_SUPERVISOR" in n.names for n in ast.walk(main)))
        starts = [n.lineno for n in ast.walk(main) if isinstance(n, ast.Call) and any(
            k.arg == "target" and isinstance(k.value, ast.Attribute) and k.value.attr == "run" and isinstance(k.value.value, ast.Name) and k.value.value.id == "sup"
            for k in n.keywords)]
        self.assertEqual(len(starts), 1)
        self.assertLess(assign[0].lineno, starts[0], "감독자 스레드가 뜬 뒤에 대입했다 — /health 가 반쯤 만들어진 객체를 볼 수 있다")
        hb = next(n for n in self.tree.body if isinstance(n, ast.FunctionDef) and n.name == "health_body")
        self.assertTrue(any(isinstance(n, ast.Attribute) and n.attr == "liveness" for n in ast.walk(hb)))


if __name__ == "__main__":
    unittest.main(verbosity=2)
