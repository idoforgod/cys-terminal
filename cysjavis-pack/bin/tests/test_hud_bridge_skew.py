#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_hud_bridge_skew.py — HUD 브리지(C4)의 선언 판정 표시 + **배송 스큐 안전** 회귀.

이 스위트가 지키는 것
  ① 신버전 데몬이 싣는 `verdict` 를 읽어 **미선언/고아를 구분 표시**한다(설계 §4-5 C4)
  ② ★**스큐 안전(ADR-2)** — 구버전 데몬 payload(=`verdict` 없음)에서는 프레임·월드가
     **개정 전과 완전히 동일**하다. 팩은 pack 채널로, 데몬은 앱 릴리스로 배송돼 스큐가
     **정상 상태**이므로 양측은 서로를 전제하면 안 된다
  ③ **온보딩 방어(설계 §6)** — 미선언은 경고가 아니라 정보다. 라벨만 달고 진행률(done/total)은
     사용자에게서 빼앗지 않는다

실행: python3 test_hud_bridge_skew.py   (unittest·파일 직접 실행 — 저장소 관례 준거)
"""
import os
import sys
import unittest

SELF = os.path.dirname(os.path.abspath(__file__))                        # …/bin/tests
BIN = os.path.dirname(SELF)                                              # cysjavis-pack/bin
sys.path.insert(0, BIN)
import javis_hud_bridge as HB                                            # noqa: E402


def status(todo):
    """`cys status --json` 최소 모사 — merge_fleet 이 읽는 필드만."""
    return {"daemon": {"version": "test", "latest_seq": 7}, "paused": False, "todo": todo}


def event(path, done, total, verdict=None):
    """데몬 todo.updated 이벤트 1건. verdict=None = **구버전 데몬**(필드 자체가 없다)."""
    payload = {"path": path, "done": done, "total": total}
    if verdict is not None:
        payload["verdict"] = verdict
    return {"name": "todo.updated", "timestamp": HB.time.time(), "surface_id": None,
            "payload": payload}


def route(world, ev):
    return HB.route_event(ev, world, HB.Coalescer(), slug="main")[0]


P_WORKER = "/x/_round/WORKER_TODO.md"
P_PLAIN = "/x/_round/PLAIN_TODO.md"
P_ORPHAN = "/x/_round/CSO_TODO.md"


class SkewSafety(unittest.TestCase):
    """구버전 데몬 ↔ 신버전 팩 — 개정 전과 동일 동작(불변식: 상호 전제 금지)."""

    def test_old_daemon_status_snapshot_is_untouched(self):
        # 구버전 데몬의 org.status todo 항목에는 verdict 가 없다.
        old = {P_WORKER: {"done": 1, "total": 2, "age_secs": 3}}
        w = HB.World()
        w.merge_fleet(None, status(old))
        self.assertEqual(w.todo[P_WORKER], {"done": 1, "total": 2, "age_secs": 3})
        self.assertNotIn("flag", w.todo[P_WORKER])
        # 원본 dict 를 그대로 돌려준다(사본 생성조차 없다 = 완전 동일 경로).
        self.assertIs(w.todo[P_WORKER], old[P_WORKER])

    def test_old_daemon_event_frame_is_byte_identical(self):
        w = HB.World()
        frames = route(w, event(P_WORKER, 1, 2))
        self.assertEqual(frames, [{"t": "fx", "kind": "todo", "path": P_WORKER,
                                   "done": 1, "total": 2}],
                         "구버전 payload 에서 프레임 모양이 달라지면 스큐가 곧 회귀다")
        self.assertEqual(w.todo[P_WORKER], {"done": 1, "total": 2, "age_secs": 0})

    def test_old_daemon_event_does_not_erase_known_verdict(self):
        # 신버전 스냅샷으로 라벨이 붙은 뒤 구버전 이벤트가 도착해도 라벨을 지우지 않는다.
        w = HB.World()
        w.merge_fleet(None, status({P_PLAIN: {"done": 0, "total": 3, "age_secs": 1,
                                              "verdict": "unclaimed"}}))
        self.assertEqual(w.todo[P_PLAIN]["flag"], "미선언")
        route(w, event(P_PLAIN, 1, 3))          # verdict 없는 구버전 이벤트
        self.assertEqual(w.todo[P_PLAIN]["flag"], "미선언")
        self.assertEqual((w.todo[P_PLAIN]["done"], w.todo[P_PLAIN]["total"]), (1, 3))


class VerdictDisplay(unittest.TestCase):
    """신버전 데몬 — 미선언·고아를 구분 표시한다."""

    def test_status_snapshot_labels_unclaimed_and_orphan(self):
        w = HB.World()
        w.merge_fleet(None, status({
            P_WORKER: {"done": 1, "total": 2, "age_secs": 0, "verdict": "counted"},
            P_PLAIN: {"done": 2, "total": 5, "age_secs": 0, "verdict": "unclaimed"},
            P_ORPHAN: {"done": 0, "total": 4, "age_secs": 0, "verdict": "orphan-scope"},
        }))
        self.assertNotIn("flag", w.todo[P_WORKER], "정상 파일에 라벨을 달면 소음이 된다")
        self.assertEqual(w.todo[P_PLAIN]["flag"], "미선언")
        self.assertEqual(w.todo[P_ORPHAN]["flag"], "고아")
        # 두 상태는 서로 구분돼야 한다 — 불리언 하나로는 나를 수 없는 이유.
        self.assertNotEqual(w.todo[P_PLAIN]["flag"], w.todo[P_ORPHAN]["flag"])

    def test_event_frame_carries_flag(self):
        w = HB.World()
        self.assertEqual(route(w, event(P_PLAIN, 0, 1, "unclaimed"))[0]["flag"], "미선언")
        self.assertEqual(route(w, event(P_ORPHAN, 0, 1, "orphan-scope"))[0]["flag"], "고아")
        self.assertNotIn("flag", route(w, event(P_WORKER, 0, 1, "counted"))[0])

    def test_flag_is_released_when_file_becomes_counted(self):
        # 사용자가 선언 한 줄을 넣으면 라벨이 사라져야 한다(고쳤는데 계속 표시되면 신뢰가 깨진다).
        w = HB.World()
        route(w, event(P_PLAIN, 0, 1, "unclaimed"))
        self.assertEqual(w.todo[P_PLAIN]["flag"], "미선언")
        route(w, event(P_PLAIN, 0, 1, "counted"))
        self.assertNotIn("flag", w.todo[P_PLAIN])

    def test_unknown_verdict_is_silent(self):
        # 미래 데몬이 모르는 판정을 실어도 라벨을 지어내지 않는다(전방 호환).
        w = HB.World()
        frames = route(w, event(P_WORKER, 1, 2, "some-future-verdict"))
        self.assertNotIn("flag", frames[0])
        self.assertIsNone(HB.todo_flag({"verdict": "some-future-verdict"}))

    def test_snapshot_exposes_flag_to_hud_clients(self):
        # 최종 소비자(HUD 프레임)까지 라벨이 도달하는지 — 중간에서 끊기면 표시가 안 된다.
        w = HB.World()
        w.merge_fleet(None, status({
            P_WORKER: {"done": 1, "total": 2, "age_secs": 0, "verdict": "counted"},
            P_ORPHAN: {"done": 0, "total": 4, "age_secs": 0, "verdict": "orphan-scope"},
        }))
        snap = w.snapshot()
        self.assertNotIn("flag", snap["todo"]["worker"])
        self.assertEqual(snap["todo"]["cso"]["flag"], "고아")


class DeclaredOwnerLabel(unittest.TestCase):
    """★W14 S16 — HUD 라벨의 진실은 **선언 `owner`** 다(파일명 추론 D3의 마지막 생존지).

    데몬은 `todo.updated`에 owner 를 실으면서 `org.status`에는 싣지 않았고, 브리지는 **어느
    쪽도 읽지 않았다**. 그래서 선언이 없애려던 파일명→역할 추론이 HUD 에 그대로 살아 있었다.
    """

    def test_declared_owner_wins_over_filename(self):
        # 재현: 선언 owner=cso 인데 파일명은 WORKER_TODO.md → 종전 HUD 라벨은 'worker' 였다.
        w = HB.World()
        w.merge_fleet(None, status({P_WORKER: {"done": 1, "total": 3, "age_secs": 0,
                                               "verdict": "counted", "owner": "cso"}}))
        self.assertEqual(list(w.snapshot()["todo"]), ["cso"])
        # 이벤트 경로도 같은 진실을 쓴다(스냅샷과 이벤트가 갈리면 새로고침에 라벨이 뒤집힌다).
        w2 = HB.World()
        ev = event(P_WORKER, 1, 3, "counted")
        ev["payload"]["owner"] = "cso"
        route(w2, ev)
        self.assertEqual(list(w2.snapshot()["todo"]), ["cso"])

    def test_owner_label_is_normalized_to_role_space(self):
        # 라벨은 조인 키다 — 한 글자만 어긋나도 조인이 조용히 전패한다.
        for raw in ("REVIEWER_GEMINI", "Reviewer-Gemini", "reviewer_gemini", " reviewer-gemini "):
            self.assertEqual(HB.normalize_role_label(raw), "reviewer-gemini", raw)

    def test_hyphen_and_digit_filenames_are_not_dropped(self):
        """★재현 — 종전 정규식 `[A-Z_]+` 는 숫자·하이픈 파일명을 **통째로 탈락**시켰다.
        둘 다 실재 형태다: `WORKER_2_TODO.md` 는 `cys todo-path` 가 role `worker-2` 에 대해
        실제로 만드는 이름이고, 하이픈판은 손기동 산출물로 실측된다."""
        w = HB.World()
        w.merge_fleet(None, status({
            "/x/_round/REVIEWER-GEMINI_TODO.md": {"done": 0, "total": 2, "age_secs": 0},
            "/x/_round/WORKER_2_TODO.md": {"done": 0, "total": 5, "age_secs": 0},
            "/x/_round/MASTER_TODO.md": {"done": 1, "total": 1, "age_secs": 0},
        }))
        self.assertEqual(sorted(w.snapshot()["todo"]),
                         ["master", "reviewer-gemini", "worker-2"])
        # 라벨공간 4벌 동치 — 어떤 표기로 와도 같은 키로 수렴한다.
        for p in ("/x/WORKER_2_TODO.md", "/x/worker-2_TODO.md",
                  "/x/Worker_2_TODO.md", "/x/WORKER-2_TODO.md"):
            self.assertEqual(HB.todo_label(p, {}), "worker-2", p)

    def test_completed_file_does_not_shadow_pending_one(self):
        """★재현 — 같은 라벨 dict 덮어쓰기로 **완료 5/5 가 미완 0/2 를 덮었다**.
        `javis_report` 의 정본 선출에는 '미완 우선' 키가 있는데 HUD 에 미이식이었다."""
        w = HB.World()
        w.merge_fleet(None, status({
            "/a/_round/WORKER_TODO.md": {"done": 0, "total": 2, "age_secs": 10},
            "/b/_round/WORKER_TODO.md": {"done": 5, "total": 5, "age_secs": 99},
        }))
        self.assertEqual(w.snapshot()["todo"]["worker"]["total"], 2,
                         "완료된 파일이 미완 파일을 덮었다(살아있는 작업이 화면에서 사라진다)")
        # 선언 owner 보유 파일이 미선언 파일에 밀리지 않는다(정렬 1순위).
        w2 = HB.World()
        w2.merge_fleet(None, status({
            "/a/_round/WORKER_TODO.md": {"done": 0, "total": 9, "age_secs": 1},
            "/b/_round/OTHER_TODO.md": {"done": 0, "total": 3, "age_secs": 5,
                                        "owner": "worker"},
        }))
        self.assertEqual(w2.snapshot()["todo"]["worker"]["total"], 3)

    def test_old_daemon_without_owner_falls_back_to_filename(self):
        """스큐 안전(ADR-2) — 구버전 데몬(owner 필드 없음)에서는 종전 동작 그대로다."""
        w = HB.World()
        w.merge_fleet(None, status({P_ORPHAN: {"done": 2, "total": 4, "age_secs": 3}}))
        self.assertEqual(list(w.snapshot()["todo"]), ["cso"])
        # 구버전 이벤트 한 건이 이미 알고 있는 owner 를 지우지 않는다.
        w2 = HB.World()
        ev = event(P_WORKER, 1, 3, "counted")
        ev["payload"]["owner"] = "cso"
        route(w2, ev)
        route(w2, event(P_WORKER, 2, 3))          # 구버전 payload(verdict·owner 없음)
        self.assertEqual(list(w2.snapshot()["todo"]), ["cso"])


class OnboardingDefense(unittest.TestCase):
    """설계 §6 온보딩 방어 — 미선언은 경고가 아니라 정보다."""

    def test_unclaimed_keeps_its_progress(self):
        # ②진행률을 사용자에게서 빼앗지 않는다: done/total 이 그대로 보인다.
        w = HB.World()
        w.merge_fleet(None, status({P_PLAIN: {"done": 3, "total": 4, "age_secs": 0,
                                              "verdict": "unclaimed"}}))
        self.assertEqual((w.todo[P_PLAIN]["done"], w.todo[P_PLAIN]["total"]), (3, 4))

    def test_label_is_informational_not_warning(self):
        # ①경고(⚠)가 아니라 정보다 — 라벨 문자열에 경고 기호를 쓰지 않는다.
        for flag in HB.TODO_FLAG_BY_VERDICT.values():
            self.assertNotIn("⚠", flag)
            self.assertNotIn("!", flag)

    def test_no_todo_no_section(self):
        # ③todo 가 0개면 아무것도 만들어내지 않는다(신규 설치 무소음).
        w = HB.World()
        w.merge_fleet(None, status({}))
        self.assertEqual(w.todo, {})
        self.assertEqual(w.snapshot()["todo"], {})


if __name__ == "__main__":
    unittest.main(verbosity=2)
