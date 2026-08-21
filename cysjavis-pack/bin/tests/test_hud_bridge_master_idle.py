#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_hud_bridge_master_idle.py — G2 결함 8 W3-B: master.idle HUD 소비 회귀 핀.

이 스위트가 지키는 것
  ① **master.idle → fx kind "idle"** — v2 데몬의 정보성 침묵 신호가 HUD 프레임에 도달한다
     (role additive·payload 필드명은 idle_secs — pane.idle 의 idle_seconds 와 다름)
  ② **코얼레싱 등재(120s)** — ALERT_COALESCE 미등재면 5s 틱마다 fx 폭주가 가능했다.
     같은 (name,surface) 윈도 내 재발화 억제 + 윈도 경과 후 재허용
  ③ **pane.idle 상호 불간섭** — fx kind 는 "idle" 을 공유(확인된 의도)하되 코얼레스 키는
     (이벤트명, surface)라 서로를 억제하지 않는다
  ④ **master.deadman v2 소비 무변경** — v2 additive payload(axis·role·inputs…)에서도
     기존 kind "deadman"·top-level idle_secs 소비가 그대로다(스큐 안전 ADR-2 동형)
  ⑤ **백로그 억제** — 콜드스타트에 과거 master.idle 이 fx 로 폭주하지 않는다(§7 관례)

실행: python3 test_hud_bridge_master_idle.py   (unittest·파일 직접 실행 — 저장소 관례 준거)
"""
import os
import sys
import unittest

SELF = os.path.dirname(os.path.abspath(__file__))                        # …/bin/tests
BIN = os.path.dirname(SELF)                                              # cysjavis-pack/bin
sys.path.insert(0, BIN)
import javis_hud_bridge as HB                                            # noqa: E402


def master_idle_event(sid=3, role="master", idle_secs=930, ts=None):
    """데몬 v2 master.idle 이벤트 — governance.rs 발행 payload 실스키마 절취."""
    return {"name": "master.idle", "timestamp": HB.time.time() if ts is None else ts,
            "surface_id": sid,
            "payload": {"role": role, "surface_ref": f"surface:{sid}", "axis": "silence",
                        "idle_secs": idle_secs, "threshold_secs": 900,
                        "debounce_secs": 300, "process_alive": True, "agent_alive": True,
                        "last_output_epoch": 0.0, "severity": "info"}}


def deadman_v2_event(sid=3):
    """데몬 v2 master.deadman — additive 확장 payload(axis·inputs·thresholds…) 실스키마 절취."""
    return {"name": "master.deadman", "timestamp": HB.time.time(), "surface_id": sid,
            "payload": {"reason": "agent process dead", "axis": "agent_dead",
                        "role": "master", "surface_ref": f"surface:{sid}",
                        "idle_secs": 42,
                        "inputs": {"pid": 111, "seat_state": "empty",
                                   "agent_meta": "claude", "agent_alive": False,
                                   "status_age_secs": None},
                        "thresholds": {"confirm_ticks": 3, "tick_secs": 5,
                                       "grace_secs": 60, "debounce_secs": 300},
                        "misses": 3, "last_ok_epoch": 0.0}}


def route(world, ev, coal=None):
    return HB.route_event(ev, world, coal or HB.Coalescer(), slug="main")[0]


class MasterIdleFrame(unittest.TestCase):
    """① master.idle → fx kind "idle" (role·idle_secs 탑재)."""

    def test_master_idle_emits_idle_fx_with_role(self):
        frames = route(HB.World(), master_idle_event())
        self.assertEqual(frames, [{"t": "fx", "kind": "idle", "key": "main@surface:3",
                                   "role": "master", "idle_secs": 930}],
                         "master.idle 프레임 모양이 달라지면 HUD 소비 계약 회귀다")

    def test_payload_field_is_idle_secs_not_idle_seconds(self):
        # 데몬은 idle_secs 를 싣는다 — pane.idle 의 idle_seconds 를 읽으면 None 이 샌다.
        ev = master_idle_event(idle_secs=77)
        self.assertEqual(route(HB.World(), ev)[0]["idle_secs"], 77)

    def test_role_generalization_passthrough(self):
        # CYS_ROLE_DEADMAN_ROLES 확대(role 일반화 opt-in) 시 role 이 그대로 흐른다.
        self.assertEqual(route(HB.World(), master_idle_event(role="cso"))[0]["role"], "cso")


class MasterIdleCoalesce(unittest.TestCase):
    """② ALERT_COALESCE 등재 — 윈도 내 억제·경과 후 재허용."""

    def test_registered_with_pane_idle_parity_window(self):
        self.assertEqual(HB.ALERT_COALESCE.get("master.idle"), 120.0,
                         "미등재면 fx 예산만이 상한 — 틱마다 idle fx 폭주 가능")

    def test_second_event_within_window_is_suppressed(self):
        w, coal = HB.World(), HB.Coalescer()
        self.assertEqual(len(route(w, master_idle_event(), coal)), 1)
        self.assertEqual(route(w, master_idle_event(), coal), [],
                         "윈도(120s) 내 같은 surface 재발화가 프레임을 만들면 코얼레스 회귀")

    def test_window_expiry_reallows(self):
        # Coalescer.allow 는 now 주입이 가능해 윈도 경계를 결정론으로 핀할 수 있다.
        coal = HB.Coalescer()
        self.assertTrue(coal.allow("master.idle", "main@surface:3", now=1000.0))
        self.assertFalse(coal.allow("master.idle", "main@surface:3", now=1119.9))
        self.assertTrue(coal.allow("master.idle", "main@surface:3", now=1120.1))

    def test_distinct_surfaces_do_not_suppress_each_other(self):
        w, coal = HB.World(), HB.Coalescer()
        self.assertEqual(len(route(w, master_idle_event(sid=3), coal)), 1)
        self.assertEqual(len(route(w, master_idle_event(sid=4), coal)), 1,
                         "코얼레스 키는 (name,surface) — 타 surface 를 억제하면 안 된다")


class PaneIdleIsolation(unittest.TestCase):
    """③ pane.idle 상호 불간섭 — kind 공유는 의도, 코얼레스는 분리."""

    def test_master_idle_does_not_suppress_pane_idle(self):
        w, coal = HB.World(), HB.Coalescer()
        self.assertEqual(len(route(w, master_idle_event(sid=3), coal)), 1)
        pane = {"name": "pane.idle", "timestamp": HB.time.time(), "surface_id": 3,
                "payload": {"idle_seconds": 120}}
        frames = route(w, pane, coal)
        self.assertEqual(len(frames), 1,
                         "master.idle 직후 같은 surface 의 pane.idle 이 억제되면 키 혼입 회귀")
        # pane.idle 프레임은 개정 전과 완전 동일(additive 원칙 — role 키 자체가 없다).
        self.assertEqual(frames[0], {"t": "fx", "kind": "idle", "key": "main@surface:3",
                                     "idle_secs": 120})


class DeadmanV2Unchanged(unittest.TestCase):
    """④ master.deadman 소비 무변경 — v2 additive payload 에서 종전 프레임 그대로."""

    def test_v2_payload_yields_same_deadman_frame(self):
        frames = route(HB.World(), deadman_v2_event())
        self.assertEqual(frames, [{"t": "fx", "kind": "deadman", "key": "main@surface:3",
                                   "idle_secs": 42}],
                         "v2 additive 확장이 deadman 프레임 모양을 바꾸면 스큐 회귀(ADR-2)")


class BacklogGate(unittest.TestCase):
    """⑤ 백로그 억제 — 과거 master.idle 은 fx 를 만들지 않는다(콜드스타트 폭주 방지)."""

    def test_backlog_master_idle_emits_no_fx(self):
        old = master_idle_event(ts=HB.time.time() - (HB.BACKLOG_FX_SECS + 5))
        self.assertEqual(route(HB.World(), old), [])


if __name__ == "__main__":
    unittest.main(verbosity=2)
