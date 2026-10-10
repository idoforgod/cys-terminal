#!/usr/bin/env python3
"""부서 한정 키 모델 v2 백엔드 단위 테스트 — stdlib unittest만 (신규 의존성 0).

대상(DESIGN-dept-qualified-keys-v2 §7): 동번호 2부서 diff 무간섭·patch 정확 타깃 · 소켓 캐리
라우팅 3분기 · CMD_KEY_RE 음성 · heat 지연 승격 부팅 순서 재현 · apply_usage 본부 스코프 ·
despawn/spawn fallback 정식 키 · 귀속 사다리 4분기 · slug 정규화·충돌 · route_event 본부 키 ·
dept 필드 부재 폴백 · 리플레이 관용 · fleet 키==이벤트 키 정합. 음성 케이스 포함.
"""
import contextlib
import io
import json
import os
import secrets
import tempfile
import threading
import time
import unittest

BIN = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "bin")
if BIN not in __import__("sys").path:
    __import__("sys").path.insert(0, BIN)

import javis_hud_bridge as B  # noqa: E402
import javis_event as E       # noqa: E402


def build_fleet(depts):
    """depts: [(dept_slug|None, socket, department_label, [(role, ref, sid), ...])] → fleet dict."""
    out = []
    for slug, socket, label, surfaces in depts:
        d = {"department": label, "socket": socket,
             "surfaces": [{"role": r, "surface_ref": ref, "surface_id": sid}
                          for (r, ref, sid) in surfaces]}
        if slug is not None:
            d["dept"] = slug
        out.append(d)
    return {"departments": out}


def merged_world(depts):
    """build_fleet → merge_fleet 로 주석(_full_key·socket)이 부여된 World."""
    w = B.World()
    w.merge_fleet(build_fleet(depts), None)
    return w


# 동번호 2부서(둘 다 surface:5) + 본부에 유일 surface:8 — 충돌 재현 공용 fixture
DUP = [("main", None, "본부 · CEO",
        [("master", "surface:5", 5), ("cso", "surface:8", 8)]),
       ("dept-1", "/tmp/d1.sock", "1부서",
        [("worker", "surface:5", 5)])]


class KeyHelpers(unittest.TestCase):
    def test_full_key_and_none(self):
        self.assertEqual(B.full_key("dept-1", "surface:5"), "dept-1@surface:5")
        self.assertIsNone(B.full_key("dept-1", None))

    def test_node_key_prefers_annotation_then_bare(self):
        self.assertEqual(B.node_key({"_full_key": "main@surface:5",
                                     "surface_ref": "surface:5"}), "main@surface:5")
        self.assertEqual(B.node_key({"surface_ref": "surface:9"}), "surface:9")  # 미주석 폴백

    def test_normalize_slug_charset(self):
        self.assertTrue(B.SLUG_RE.match(B.normalize_slug("Team A!")))
        self.assertEqual(B.normalize_slug("Team.A"), "team-a")   # 대문자화·. → -
        self.assertEqual(B.normalize_slug(""), "dept")           # 빈값 → dept (fail-open 금지)
        self.assertEqual(B.normalize_slug("MAIN"), "main")


class SlugNormalizationInMerge(unittest.TestCase):
    def test_collision_gets_suffix(self):
        # "Team A" → team-a, "team.a" → team-a 충돌 → 두 번째는 team-a-2
        w = merged_world([("Team A", None, "A", [("r1", "surface:1", 1)]),
                          ("team.a", "/tmp/x.sock", "B", [("r2", "surface:2", 2)])])
        slugs = [d["_slug"] for d in w.departments]
        self.assertEqual(slugs, ["team-a", "team-a-2"])
        self.assertTrue(all(B.SLUG_RE.match(s) for s in slugs))


class DeptFieldFallback(unittest.TestCase):
    def test_absent_dept_field_normalizes_display_name_with_warning(self):
        w = B.World()
        fleet = {"departments": [{"department": "main-hq",   # dept 필드 없음
                                  "surfaces": [{"role": "m", "surface_ref": "surface:5",
                                                "surface_id": 5}]}]}
        buf = io.StringIO()
        with contextlib.redirect_stderr(buf):
            w.merge_fleet(fleet, None)
        self.assertTrue(w._dept_fallback_warned)
        self.assertIn("dept 필드 부재", buf.getvalue())
        self.assertEqual(B.node_key(w.departments[0]["surfaces"][0]), "main-hq@surface:5")

    def test_warning_only_once(self):
        w = B.World()
        fleet = {"departments": [{"department": "hq",
                                  "surfaces": [{"role": "m", "surface_ref": "surface:1",
                                                "surface_id": 1}]}]}
        buf = io.StringIO()
        with contextlib.redirect_stderr(buf):
            w.merge_fleet(fleet, None)
            w.merge_fleet(fleet, None)   # 2회차 — 경고 재출력 없음
        self.assertEqual(buf.getvalue().count("dept 필드 부재"), 1)


class DiffIsolation(unittest.TestCase):
    """동번호 2부서 diff 무간섭 — prev_nodes 가 정식 키로 분리돼 상호 덮어쓰지 않는다."""

    def test_same_ref_two_depts_distinct_keys(self):
        w = merged_world(DUP)
        keys = {B.node_key(s) for d in w.departments for s in d.get("surfaces", [])}
        self.assertIn("main@surface:5", keys)
        self.assertIn("dept-1@surface:5", keys)   # 충돌 없이 공존

    def test_patch_targets_only_changed_node(self):
        w = merged_world(DUP)   # 1차 병합 → prev_nodes 채움
        # dept-1 의 surface:5 만 task 변경 → 그 정식 키 patch 1건만
        fleet2 = build_fleet(DUP)
        fleet2["departments"][1]["surfaces"][0]["status"] = {"task": "빌드", "state": "working",
                                                             "age_secs": 1}
        patches, structural = w.merge_fleet(fleet2, None)
        self.assertFalse(structural)   # 키 집합·shape 불변 → 구조 변화 아님
        changed = [p["key"] for p in patches]
        self.assertEqual(changed, ["dept-1@surface:5"])   # 본부 surface:5 는 무간섭


class SocketCarry(unittest.TestCase):
    """M1 소켓 캐리 3분기 — 부서→socket 경로 · 본부→None · 미지 키→fail-closed."""

    def test_dept_socket_present(self):
        w = merged_world(DUP)
        self.assertEqual(w.socket_for("dept-1@surface:5"), (True, "/tmp/d1.sock"))

    def test_hq_socket_none(self):
        w = merged_world(DUP)
        self.assertEqual(w.socket_for("main@surface:8"), (True, None))

    def test_unknown_key_fail_closed(self):
        w = merged_world(DUP)
        self.assertEqual(w.socket_for("ghost@surface:99"), (False, None))


class CmdKeyRegex(unittest.TestCase):
    """CMD_KEY_RE 음성 — 정식 키만 통과, bare·비정합 slug·# 구분자 거부."""

    def test_formal_key_accepted(self):
        self.assertTrue(B.CMD_KEY_RE.match("dept-1@surface:5"))
        self.assertTrue(B.CMD_KEY_RE.match("main@surface:11"))

    def test_bare_key_rejected(self):   # 구 v1 bare 키 거부
        self.assertIsNone(B.CMD_KEY_RE.match("surface:5"))

    def test_uppercase_slug_rejected(self):
        self.assertIsNone(B.CMD_KEY_RE.match("Dept@surface:5"))

    def test_hash_separator_rejected(self):   # v2.0 # → @ 교체 회귀 방지
        self.assertIsNone(B.CMD_KEY_RE.match("dept-1#surface:5"))

    def test_gate_command_rejects_bare_and_accepts_formal(self):
        origins = B.allowed_origins(8642)
        tok = secrets.token_hex(8)
        known = {"dept-1@surface:5"}
        ok, err, _ = B.gate_command({"key": "surface:5", "text": "hi"},
                                    tok, tok, None, origins, known)
        self.assertFalse(ok)
        self.assertEqual(err, "bad_key_format")
        ok, err, cleaned = B.gate_command({"key": "dept-1@surface:5", "text": "hi"},
                                          tok, tok, None, origins, known)
        self.assertTrue(ok)
        self.assertEqual(cleaned["key"], "dept-1@surface:5")


class ApplyUsageScope(unittest.TestCase):
    """C3 apply_usage 본부 한정 — 순서 의존 없이 동번호 부서 오귀속 차단."""

    def test_main_scope_ignores_dept_order(self):
        # 부서를 본부보다 먼저 배치해도 usage 는 본부(main) surface_id==7 로만 귀속
        depts = [("dept-1", "/tmp/d1.sock", "1부서", [("worker", "surface:7", 7)]),
                 ("main", None, "본부", [("master", "surface:7", 7)])]
        w = merged_world(depts)
        k = w.apply_usage(7, {"ctx_pct": 55})
        self.assertEqual(k, "main@surface:7")
        # 본부 노드에만 usage 반영, 부서 노드는 무변
        main_s = [s for d in w.departments if d["_slug"] == "main"
                  for s in d["surfaces"]][0]
        self.assertEqual(main_s["usage"]["ctx_pct"], 55)
        dept_s = [s for d in w.departments if d["_slug"] == "dept-1"
                  for s in d["surfaces"]][0]
        self.assertNotIn("usage", dept_s)

    def test_no_main_returns_none(self):
        w = merged_world([("dept-1", "/tmp/d1.sock", "1부서", [("worker", "surface:7", 7)])])
        self.assertIsNone(w.apply_usage(7, {"ctx_pct": 10}))


class RouteEventKeys(unittest.TestCase):
    """route_event 본부 정식 키(main@surface:N) — 단일 구독 반영."""

    def test_tool_hook_key_is_main_qualified(self):
        w = merged_world(DUP)
        coal = B.Coalescer()
        now = time.time()
        ev = {"name": "agent.hook.PreToolUse", "surface_id": 5, "timestamp": now,
              "payload": {"tool_name": "Bash", "role": "master"}}
        frames, poke = B.route_event(ev, w, coal, now=now)
        fx = [f for f in frames if f["t"] == "fx"]
        self.assertEqual(fx[0]["key"], "main@surface:5")

    def test_despawn_fallback_is_main_qualified(self):
        # C4: surface_id 부재 → payload.surface_ref 재조립 → main@ 정식 키
        w = merged_world(DUP)
        coal = B.Coalescer()
        now = time.time()
        ev = {"name": "surface.exited", "surface_id": None, "timestamp": now,
              "payload": {"surface_ref": "surface:9"}}
        frames, poke = B.route_event(ev, w, coal, now=now)
        self.assertTrue(poke)
        fx = [f for f in frames if f["t"] == "fx"][0]
        self.assertEqual(fx["kind"], "despawn")
        self.assertEqual(fx["key"], "main@surface:9")

    def test_spawn_with_sid_is_main_qualified(self):
        w = merged_world(DUP)
        coal = B.Coalescer()
        now = time.time()
        ev = {"name": "surface.created", "surface_id": 12, "timestamp": now, "payload": {}}
        frames, _ = B.route_event(ev, w, coal, now=now)
        fx = [f for f in frames if f["t"] == "fx"][0]
        self.assertEqual(fx["kind"], "spawn")
        self.assertEqual(fx["key"], "main@surface:12")


class AttributionLadderV2(unittest.TestCase):
    """귀속 사다리 4분기 — 정식 키·bare 유일·role 유일·미귀속."""

    def test_formal_key_in_snapshot_wins(self):
        w = merged_world(DUP)
        self.assertEqual(B.attribute_spool(
            {"key": "dept-1@surface:5", "payload": {}}, w), "dept-1@surface:5")

    def test_formal_key_absent_falls_back_to_role(self):
        w = merged_world(DUP)
        # 스냅샷에 없는 정식 키 → 신뢰 안 함 → agent=cso 유일 role 폴백(본부 surface:8)
        self.assertEqual(B.attribute_spool(
            {"key": "ghost@surface:99", "payload": {"agent": "cso"}}, w), "main@surface:8")

    def test_bare_unique_promotes_to_full_key(self):
        w = merged_world(DUP)
        # surface:8 은 본부에만 유일 → 정식 키로 승격
        self.assertEqual(B.attribute_spool({"key": "surface:8", "payload": {}}, w),
                         "main@surface:8")

    def test_bare_ambiguous_no_role_is_none(self):
        w = merged_world(DUP)
        # surface:5 는 2부서 중복 + agent 미매칭 → 미귀속
        self.assertIsNone(B.attribute_spool(
            {"key": "surface:5", "payload": {"agent": "ghost"}}, w))

    def test_progress_attributes_to_full_key(self):
        w = merged_world(DUP)
        coal = B.Coalescer()
        entry = {"ts": time.time(), "type": "task_progress", "key": "dept-1@surface:5",
                 "payload": {"task": "T", "stage": "build", "pct": 30}}
        B.route_spool(entry, w, coal)
        self.assertIn("dept-1@surface:5", w.progress)       # 정식 키로 귀속
        self.assertNotIn("main@surface:5", w.progress)      # 본부 동번호는 무간섭


class HeatMigration(unittest.TestCase):
    """M2 heat 지연 승격 — 부팅 순서 재현: 빈 fleet 보존 → 첫 merge 후 유일 승격."""

    def test_load_splits_bare_and_full(self):
        tmp = tempfile.mkdtemp()
        path = os.path.join(tmp, "presence_heat.json")
        with open(path, "w") as f:
            json.dump({"v": 2, "hour": 3, "acc": {
                "surface:5": {"active": [1] * 24, "total": [2] * 24},          # bare → pending
                "main@surface:8": {"active": [0] * 24, "total": [1] * 24}}},   # 정식 → 즉시
                f)
        orig = B.PRESENCE_HEAT_PATH
        B.PRESENCE_HEAT_PATH = path
        try:
            w = B.World()
            B.load_heat(w)
        finally:
            B.PRESENCE_HEAT_PATH = orig
        self.assertIn("main@surface:8", w.heat_acc)     # 정식 키 즉시 복원
        self.assertIn("surface:5", w.pending_heat)      # bare 키 보존
        self.assertNotIn("surface:5", w.heat_acc)
        self.assertEqual(w.heat_hour, 3)

    def test_boot_order_promotes_on_first_merge(self):
        w = B.World()
        w.pending_heat = {"surface:8": {"active": [3] * 24, "total": [4] * 24}}
        # 빈 fleet(부팅 직후) — 승격 불가·보존 유지
        w.merge_fleet(None, None)
        self.assertFalse(w.heat_migrated)
        self.assertIn("surface:8", w.pending_heat)
        # 첫 실 fleet 병합 → surface:8 유일 → main@surface:8 로 승격
        w.merge_fleet(build_fleet(DUP), None)
        self.assertTrue(w.heat_migrated)
        self.assertEqual(w.pending_heat, {})
        self.assertIn("main@surface:8", w.heat_acc)
        self.assertEqual(w.heat_acc["main@surface:8"]["active"][0], 3)

    def test_ambiguous_pending_discarded(self):
        w = B.World()
        w.pending_heat = {"surface:5": {"active": [1] * 24, "total": [1] * 24}}
        w.merge_fleet(build_fleet(DUP), None)   # surface:5 = 2부서 중복 → 승격 실패·폐기
        self.assertTrue(w.heat_migrated)
        self.assertNotIn("main@surface:5", w.heat_acc)
        self.assertNotIn("dept-1@surface:5", w.heat_acc)
        self.assertEqual(w.pending_heat, {})


class ReplayTolerance(unittest.TestCase):
    """리플레이 관용 — read_history 는 아카이브 프레임을 원형 보존(프론트가 suffix 매칭)."""

    def test_archive_roundtrip_preserves_bare_key(self):
        tmp = tempfile.mkdtemp()
        orig_arch, orig_state = B.ARCHIVE_PATH, B.STATE_DIR
        B.ARCHIVE_PATH = os.path.join(tmp, "fx_archive.jsonl")
        B.STATE_DIR = tmp
        try:
            now = time.time()
            B.archive_fx(now, {"t": "fx", "kind": "progress", "key": "surface:5", "pct": 40})
            hist = B.read_history(0)
        finally:
            B.ARCHIVE_PATH, B.STATE_DIR = orig_arch, orig_state
        self.assertEqual(len(hist), 1)
        self.assertEqual(hist[0]["fx"]["key"], "surface:5")   # 원형 보존(무재작성)


class FleetKeyEqualsEventKey(unittest.TestCase):
    """e2e 정합 — 본부 fleet 노드 키 == 이벤트 키(main@surface:N)."""

    def test_hq_fleet_key_matches_event_key(self):
        w = merged_world([("main", None, "본부", [("master", "surface:5", 5)])])
        snap = w.snapshot()
        self.assertEqual(snap["v"], 2)
        node = snap["departments"][0]["nodes"][0]
        self.assertEqual(node["key"], "main@surface:5")
        coal = B.Coalescer()
        now = time.time()
        ev = {"name": "agent.hook.PreToolUse", "surface_id": 5, "timestamp": now,
              "payload": {"tool_name": "Bash", "role": "master"}}
        frames, _ = B.route_event(ev, w, coal, now=now)
        fx = [f for f in frames if f["t"] == "fx"][0]
        self.assertEqual(fx["key"], node["key"])   # fleet 키 == 이벤트 키

    def test_node_view_has_dept_label(self):
        w = merged_world([("dept-1", "/tmp/d1.sock", "1부서 데스크", [("worker", "surface:5", 5)])])
        node = w.snapshot()["departments"][0]["nodes"][0]
        self.assertEqual(node["dept_label"], "1부서 데스크")   # 표시 전용 라벨


class EventSurfaceValidation(unittest.TestCase):
    """§4d javis_event --surface 검증 확장 — bare·정식 키 허용, 비정합 거부."""

    def _emit(self, surface):
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            rc = E.main(["emit", "task.unblocked", "--payload", '{"task":"T"}',
                         "--surface", surface])
        return rc

    def test_bare_key_accepted(self):
        self.assertEqual(self._emit("surface:12"), E.EXIT_OK)

    def test_formal_key_accepted(self):
        self.assertEqual(self._emit("dept-1@surface:12"), E.EXIT_OK)

    def test_hash_separator_rejected(self):
        self.assertEqual(self._emit("dept-1#surface:12"), E.EXIT_INVALID)

    def test_uppercase_slug_rejected(self):
        self.assertEqual(self._emit("Dept@surface:12"), E.EXIT_INVALID)

    def test_garbage_rejected(self):
        self.assertEqual(self._emit("surface:notanumber"), E.EXIT_INVALID)


# ============================ Phase 2 (멀티 구독·sid 맵 정식화·emitter auto) ============

class ReconcileTargets(unittest.TestCase):
    """P2-1 reconcile_targets 순수 로직 — spawn·reap·상한 절단."""

    def test_spawn_all_from_empty(self):
        to_spawn, to_reap = B.reconcile_targets({"main": None, "dept-1": "s1"}, {})
        self.assertEqual(to_spawn, {"main": None, "dept-1": "s1"})
        self.assertEqual(to_reap, set())

    def test_reap_removed_target(self):
        to_spawn, to_reap = B.reconcile_targets({"main": None}, {"main": None, "dept-1": "s1"})
        self.assertEqual(to_spawn, {})
        self.assertEqual(to_reap, {"dept-1"})

    def test_spawn_only_new(self):
        to_spawn, to_reap = B.reconcile_targets(
            {"main": None, "dept-1": "s1", "dept-2": "s2"}, {"main": None, "dept-1": "s1"})
        self.assertEqual(to_spawn, {"dept-2": "s2"})
        self.assertEqual(to_reap, set())

    def test_cap_enforced_main_priority(self):
        desired = {"main": None}
        for i in range(20):
            desired["dept-%02d" % i] = "s%d" % i
        to_spawn, _ = B.reconcile_targets(desired, {}, cap=12)
        self.assertEqual(len(to_spawn), 12)   # 상한 절단
        self.assertIn("main", to_spawn)       # main 우선 보존


class SupervisorReconcile(unittest.TestCase):
    """P2-1 SubscriptionSupervisor.reconcile_once — 부서 추가/소멸 fixture (프로세스 미기동)."""

    def _fake_sup(self, world):
        sup = B.SubscriptionSupervisor(world, None, None, threading.Event(), tempfile.mkdtemp())
        spawned, reaped = [], []
        # 0.14.49: reconcile_once 가 끝난 스레드 재시작(revive_dead)을 부르므로 가짜 구독도 진짜 구독의 키(thread·stop)를 갖는다 — 살아 있는 스레드로 모사.
        alive = type("AliveThread", (), {"is_alive": staticmethod(lambda: True)})()
        sup._spawn = lambda slug, sock: (spawned.append((slug, sock)),
                                         sup.subs.__setitem__(slug, {"socket": sock, "thread": alive, "stop": threading.Event()}))
        sup._reap = lambda slug: (reaped.append(slug), sup.subs.pop(slug, None))
        return sup, spawned, reaped

    def test_initial_spawns_main_and_depts(self):
        sup, spawned, reaped = self._fake_sup(merged_world(DUP))
        sup.reconcile_once()
        self.assertEqual(dict(spawned), {"main": None, "dept-1": "/tmp/d1.sock"})
        self.assertEqual(reaped, [])

    def test_new_dept_spawned_only(self):
        w = merged_world(DUP)
        sup, spawned, reaped = self._fake_sup(w)
        sup.reconcile_once()
        spawned.clear()
        w.merge_fleet(build_fleet(DUP + [("dept-2", "/tmp/d2.sock", "2부서",
                                          [("worker", "surface:3", 3)])]), None)
        sup.reconcile_once()
        self.assertEqual(spawned, [("dept-2", "/tmp/d2.sock")])   # 신규 부서만
        self.assertEqual(reaped, [])

    def test_removed_dept_reaped(self):
        w = merged_world(DUP)
        sup, spawned, reaped = self._fake_sup(w)
        sup.reconcile_once()
        w.merge_fleet(build_fleet([("main", None, "본부", [("master", "surface:5", 5)])]), None)
        sup.reconcile_once()   # dept-1 소멸
        self.assertEqual(reaped, ["dept-1"])


class RouteEventSlugContext(unittest.TestCase):
    """P2-2 route_event slug 문맥 — 키 f'{slug}@surface:N', 기본 main 하위호환."""

    def _hook(self, w, slug):
        coal = B.Coalescer()
        now = time.time()
        ev = {"name": "agent.hook.PreToolUse", "surface_id": 5, "timestamp": now,
              "payload": {"tool_name": "Bash"}}
        frames, _ = B.route_event(ev, w, coal, slug, now=now)
        return [f for f in frames if f["t"] == "fx"][0]

    def test_dept_slug_key(self):
        self.assertEqual(self._hook(merged_world(DUP), "dept-1")["key"], "dept-1@surface:5")

    def test_default_slug_is_main(self):
        w = merged_world(DUP)
        coal = B.Coalescer()
        now = time.time()
        ev = {"name": "agent.hook.PreToolUse", "surface_id": 5, "timestamp": now,
              "payload": {"tool_name": "Bash"}}
        frames, _ = B.route_event(ev, w, coal, now=now)   # slug 생략 → main
        self.assertEqual([f for f in frames if f["t"] == "fx"][0]["key"], "main@surface:5")


class HookIsolation(unittest.TestCase):
    """P2-3 동번호 hook 무충돌 — 두 데몬 sid 5, 본부에만 hook → 본부 노드만 active."""

    def test_same_sid_hook_marks_only_own_dept(self):
        w = merged_world(DUP)   # main@surface:5 + dept-1@surface:5
        coal = B.Coalescer()
        now = time.time()
        ev = {"name": "agent.hook.PreToolUse", "surface_id": 5, "timestamp": now,
              "payload": {"tool_name": "Bash"}}
        B.route_event(ev, w, coal, "main", now=now)   # 본부 구독 경유
        nodes = {n["key"]: n for dep in w.snapshot()["departments"] for n in dep["nodes"]}
        self.assertEqual(nodes["main@surface:5"]["presence"], "active")
        self.assertNotEqual(nodes["dept-1@surface:5"]["presence"], "active")   # 오염 없음


class LineRateIsolation(unittest.TestCase):
    """P2-3 line_rate 격리(잠복 결함 2호) — 동번호 부서가 서로 activity 를 덮지 않는다."""

    def test_same_sid_line_rate_isolated(self):
        depts = [("main", None, "본부", [("master", "surface:5", 5)]),
                 ("dept-1", "/tmp/d1.sock", "1부서", [("worker", "surface:5", 5)])]
        w = B.World()
        f1 = build_fleet(depts)
        f1["departments"][0]["surfaces"][0]["line_count"] = 100
        f1["departments"][1]["surfaces"][0]["line_count"] = 100
        w.merge_fleet(f1, None)          # baseline (prev 없음 → rate 미산출)
        time.sleep(0.01)                 # dt>0 보장
        f2 = build_fleet(depts)
        f2["departments"][0]["surfaces"][0]["line_count"] = 1000   # 본부만 급증
        f2["departments"][1]["surfaces"][0]["line_count"] = 100    # 부서는 정지
        w.merge_fleet(f2, None)
        self.assertGreater(w.line_rate["main@surface:5"], 0.0)     # 본부 rate 보존
        self.assertEqual(w.line_rate["dept-1@surface:5"], 0.0)     # 부서에 덮이지 않음


class ApplyUsageSlugScope(unittest.TestCase):
    """P2-2 apply_usage slug 스코프 — 부서 구독 usage 는 그 부서 노드에만 귀속."""

    def test_usage_attributes_to_subscription_slug(self):
        w = merged_world(DUP)   # main surface:5·8, dept-1 surface:5
        coal = B.Coalescer()
        ev = {"name": "usage.updated", "surface_id": 5, "timestamp": time.time(),
              "payload": {"ctx_pct": 42}}
        frames, _ = B.route_event(ev, w, coal, "dept-1")
        fx = [f for f in frames if f["kind"] == "usage"][0]
        self.assertEqual(fx["key"], "dept-1@surface:5")   # 본부 surface:5 아님
        main5 = [s for d in w.departments if d["_slug"] == "main"
                 for s in d["surfaces"] if s["surface_id"] == 5][0]
        self.assertNotIn("usage", main5)   # 본부 노드 무반영


class EventSurfaceAuto(unittest.TestCase):
    """P2-4 --surface auto 해석 3분기 — main·부서매칭·미귀속(fail-open 금지)."""

    def setUp(self):
        self.tmp = tempfile.mkdtemp()
        depts = os.path.join(self.tmp, "depts.json")
        with open(depts, "w") as f:
            json.dump({"depts": {"dept-1": {"socket": "/tmp/d1.sock"}}}, f)
        self._saved = {k: os.environ.get(k)
                       for k in ("CYS_SOCKET", "CYS_DEPTS_JSON", "HUD_STATE_DIR")}
        os.environ["CYS_DEPTS_JSON"] = depts
        os.environ.pop("CYS_SOCKET", None)
        self._orig_ref = E._resolve_surface_ref
        E._resolve_surface_ref = lambda: "surface:7"   # cys identify 스텁

    def tearDown(self):
        E._resolve_surface_ref = self._orig_ref
        for k, v in self._saved.items():
            if v is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = v

    def test_main_when_no_socket(self):
        self.assertEqual(E.resolve_auto_surface(), "main@surface:7")

    def test_dept_when_socket_matches(self):
        os.environ["CYS_SOCKET"] = "/tmp/d1.sock"
        self.assertEqual(E.resolve_auto_surface(), "dept-1@surface:7")

    def test_unmatched_socket_is_none(self):
        os.environ["CYS_SOCKET"] = "/tmp/unknown.sock"
        self.assertIsNone(E.resolve_auto_surface())   # 미귀속 폴백

    def test_identify_fail_is_none(self):
        E._resolve_surface_ref = lambda: None
        self.assertIsNone(E.resolve_auto_surface())

    def test_emit_auto_spools_resolved_key(self):
        os.environ["CYS_SOCKET"] = "/tmp/d1.sock"
        sd = tempfile.mkdtemp()
        os.environ["HUD_STATE_DIR"] = sd
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            rc = E.main(["emit", "task.unblocked", "--payload", '{"task":"T"}',
                         "--surface", "auto", "--spool"])
        self.assertEqual(rc, 0)
        with open(os.path.join(sd, "evt_spool.jsonl")) as f:
            e = json.loads(f.readline())
        self.assertEqual(e["key"], "dept-1@surface:7")

    def test_emit_auto_unresolved_omits_key(self):
        os.environ["CYS_SOCKET"] = "/tmp/unknown.sock"   # 미매칭 → None
        sd = tempfile.mkdtemp()
        os.environ["HUD_STATE_DIR"] = sd
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            rc = E.main(["emit", "task.unblocked", "--payload", '{"task":"T"}',
                         "--surface", "auto", "--spool"])
        self.assertEqual(rc, 0)
        with open(os.path.join(sd, "evt_spool.jsonl")) as f:
            e = json.loads(f.readline())
        self.assertNotIn("key", e)   # 미귀속 — bare/오귀속 키 미기록


if __name__ == "__main__":
    unittest.main()
