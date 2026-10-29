#!/usr/bin/env python3
"""오피스 디테일 v1.1 백엔드 단위 테스트 — stdlib unittest만 (신규 의존성 0).

대상: route_spool 4계열·귀속 규칙·날짜 롤오버·tail_spool 오프셋/truncate·칸반 스캔·
verdict md 파싱·히트 버킷 산술. 음성 케이스 포함(깨진 JSON·미지 type·spool 부재).
"""
import importlib.util
import json
import os
import shutil
import tempfile
import time
import unittest

BIN = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "bin")
if BIN not in __import__("sys").path:
    __import__("sys").path.insert(0, BIN)

import javis_hud_bridge as B  # noqa: E402


def rig_event_module(tmp):
    """(0.14.56 X-8 CARRIER · A1, DESIGN-r3 §3) A rig pack: a COPY of bin/ at <tmp>/pack with its javis_event loaded from
    there by explicit path - so the producer's own pack is the rig and `--spool` lands in <tmp>/pack/state/evt_spool.jsonl.
    HUD_STATE_DIR no longer moves the spool (it is the bridge's private state dir only)."""
    pack = os.path.join(tmp, "pack")
    shutil.copytree(BIN, os.path.join(pack, "bin"), ignore=shutil.ignore_patterns("tests", "__pycache__"))
    spec = importlib.util.spec_from_file_location("javis_event_rig", os.path.join(pack, "bin", "javis_event.py"))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod, os.path.join(pack, "state", "evt_spool.jsonl")


def make_world(roles=None):
    """지정 role→surface_ref 매핑으로 fleet가 채워진 World."""
    w = B.World()
    surfaces = []
    for role, ref in (roles or {}).items():
        surfaces.append({"role": role, "surface_ref": ref,
                         "surface_id": int(ref.split(":")[-1])})
    w.departments = [{"department": "본부", "surfaces": surfaces}]
    return w


class RouteSpoolProgress(unittest.TestCase):
    def test_progress_attributes_and_fx(self):
        w = make_world({"worker": "surface:12"})
        coal = B.Coalescer()
        entry = {"ts": time.time(), "type": "task_progress", "key": "surface:12",
                 "payload": {"task": "T1", "stage": "build", "pct": 40, "detail": "…"}}
        frames, poke = B.route_spool(entry, w, coal)
        self.assertFalse(poke)
        self.assertEqual(w.progress["surface:12"]["pct"], 40)
        self.assertEqual(w.progress["surface:12"]["stage"], "build")
        self.assertEqual([f for f in frames if f["kind"] == "progress"][0]["pct"], 40)

    def test_progress_non_increasing_pct_ignored(self):
        w = make_world({"worker": "surface:12"})
        coal = B.Coalescer()
        base = {"ts": time.time(), "type": "task_progress", "key": "surface:12"}
        B.route_spool({**base, "payload": {"task": "T1", "stage": "s", "pct": 50}}, w, coal)
        # 동일 (task,stage)에 pct 미증가 → 상태 유지·fx 없음
        frames, _ = B.route_spool(
            {**base, "payload": {"task": "T1", "stage": "s", "pct": 40}}, w, coal)
        self.assertEqual(w.progress["surface:12"]["pct"], 50)  # 40으로 후퇴 안 함
        self.assertEqual([f for f in frames if f.get("kind") == "progress"], [])

    def test_progress_increasing_pct_updates(self):
        w = make_world({"worker": "surface:12"})
        coal = B.Coalescer()
        base = {"ts": time.time(), "type": "task_progress", "key": "surface:12"}
        B.route_spool({**base, "payload": {"task": "T1", "stage": "s", "pct": 50}}, w, coal)
        frames, _ = B.route_spool(
            {**base, "payload": {"task": "T1", "stage": "s", "pct": 70}}, w, coal)
        self.assertEqual(w.progress["surface:12"]["pct"], 70)
        self.assertTrue(any(f.get("kind") == "progress" for f in frames))


class RouteSpoolRun(unittest.TestCase):
    def test_run_lifecycle_counters(self):
        w = make_world({"worker": "surface:5"})
        coal = B.Coalescer()
        k = "surface:5"
        for typ, pay in [("run.queued", {"agent": "worker", "task": "A"}),
                         ("run.queued", {"agent": "worker", "task": "B"}),
                         ("run.started", {"agent": "worker", "task": "A"}),
                         ("run.succeeded", {"agent": "worker", "task": "A", "summary": "ok"}),
                         ("run.failed", {"agent": "worker", "task": "B", "summary": "x"})]:
            B.route_spool({"ts": time.time(), "type": typ, "key": k, "payload": pay}, w, coal)
        r = w.run[k]
        self.assertEqual(r["queued"], 1)          # 2 enqueued − 1 started
        self.assertIsNone(r["active"])            # succeeded 후 해제
        self.assertEqual(r["done_today"], 1)
        self.assertEqual(r["failed_today"], 1)

    def test_run_started_sets_active(self):
        w = make_world({"worker": "surface:5"})
        coal = B.Coalescer()
        B.route_spool({"ts": time.time(), "type": "run.started", "key": "surface:5",
                       "payload": {"agent": "worker", "task": "Z"}}, w, coal)
        self.assertEqual(w.run["surface:5"]["active"]["task"], "Z")

    def test_run_date_rollover_resets_today(self):
        w = make_world({"worker": "surface:5"})
        coal = B.Coalescer()
        t0 = time.mktime((2026, 7, 11, 10, 0, 0, 0, 0, -1))
        B.route_spool({"ts": t0, "type": "run.succeeded", "key": "surface:5",
                       "payload": {"agent": "worker", "task": "A", "summary": "s"}},
                      w, coal, now=t0)
        self.assertEqual(w.run["surface:5"]["done_today"], 1)
        t1 = time.mktime((2026, 7, 12, 10, 0, 0, 0, 0, -1))  # 다음 날
        B.route_spool({"ts": t1, "type": "run.succeeded", "key": "surface:5",
                       "payload": {"agent": "worker", "task": "B", "summary": "s"}},
                      w, coal, now=t1)
        self.assertEqual(w.run["surface:5"]["done_today"], 1)  # 롤오버 후 리셋되어 다시 1


def world_with_surfaces(pairs):
    """(role, ref) 목록으로 부서별 1 surface씩 배치 — 부서 간 ref 중복 재현용."""
    w = B.World()
    w.departments = [{"department": "d%d" % i, "surfaces": [
        {"role": role, "surface_ref": ref, "surface_id": i}]}
        for i, (role, ref) in enumerate(pairs)]
    return w


class Attribution(unittest.TestCase):
    def test_explicit_unique_key_wins(self):
        # 전역 유일 key는 role보다 우선
        w = make_world({"worker": "surface:1", "boss": "surface:99"})
        self.assertEqual(B.attribute_spool(
            {"key": "surface:99", "payload": {"agent": "worker"}}, w), "surface:99")

    def test_agent_unique_role_match(self):
        w = make_world({"worker": "surface:7"})
        self.assertEqual(B.attribute_spool({"payload": {"agent": "worker"}}, w), "surface:7")

    def test_agent_ambiguous_or_absent_is_none(self):
        w = make_world({"worker": "surface:7", "reviewer": "surface:8"})
        self.assertIsNone(B.attribute_spool({"payload": {"agent": "cso"}}, w))  # 매칭 role 없음
        self.assertIsNone(B.attribute_spool({"payload": {}}, w))               # agent 없음

    def test_blocked_keeps_null_key_when_unattributed(self):
        w = make_world({"worker": "surface:7"})
        coal = B.Coalescer()
        B.route_spool({"ts": time.time(), "type": "task.blocked",
                       "payload": {"task": "X", "blocked_by": ["Y"], "agent": "ghost"}},
                      w, coal)
        self.assertEqual(w.blocked[0]["task"], "X")
        self.assertIsNone(w.blocked[0]["key"])   # 미귀속 → key:null 로 남김


class AttributionUniqueness(unittest.TestCase):
    """전역 유일 게이트 — surface_ref는 부서 데몬별이라 충돌 가능(오귀속 봉쇄)."""

    def test_unique_ref_attributes(self):
        w = world_with_surfaces([("worker", "surface:5"), ("cso", "surface:9")])
        self.assertEqual(w.ref_count("surface:5"), 1)
        self.assertEqual(B.attribute_spool(
            {"key": "surface:5", "payload": {}}, w), "surface:5")

    def test_duplicate_ref_falls_back_to_role_success(self):
        # surface:5 가 두 부서에 중복 → key 게이트 실패 → agent=gemini 유일 role 폴백
        w = world_with_surfaces([("worker", "surface:5"), ("cso", "surface:5"),
                                 ("gemini", "surface:8")])
        self.assertEqual(w.ref_count("surface:5"), 2)
        self.assertEqual(B.attribute_spool(
            {"key": "surface:5", "payload": {"agent": "gemini"}}, w), "surface:8")

    def test_duplicate_ref_role_fail_is_none(self):
        # 중복 ref + role 폴백도 실패 → 미귀속(None) (오정보보다 정직한 공백)
        w = world_with_surfaces([("worker", "surface:5"), ("cso", "surface:5")])
        self.assertIsNone(B.attribute_spool(
            {"key": "surface:5", "payload": {"agent": "ghost"}}, w))

    def test_absent_ref_falls_back_to_role(self):
        # fleet에 없는 ref(n=0)도 유일 아님 → role 폴백
        w = make_world({"worker": "surface:1"})
        self.assertEqual(B.attribute_spool(
            {"key": "surface:404", "payload": {"agent": "worker"}}, w), "surface:1")

    def test_duplicate_ref_blocked_leaves_no_wrong_key(self):
        # 중복 ref blocked → 오귀속 대신 role 폴백 실패 시 key:null
        w = world_with_surfaces([("worker", "surface:5"), ("cso", "surface:5")])
        coal = B.Coalescer()
        B.route_spool({"ts": time.time(), "type": "task.blocked", "key": "surface:5",
                       "payload": {"task": "T", "blocked_by": ["d"], "agent": "ghost"}}, w, coal)
        self.assertIsNone(w.blocked[0]["key"])


class RouteSpoolBlocked(unittest.TestCase):
    def test_block_then_unblock(self):
        w = make_world({"worker": "surface:3"})
        coal = B.Coalescer()
        B.route_spool({"ts": time.time(), "type": "task.blocked", "key": "surface:3",
                       "payload": {"task": "T", "blocked_by": ["dep1"]}}, w, coal)
        self.assertEqual(len(w.blocked), 1)
        B.route_spool({"ts": time.time(), "type": "task.blocked", "key": "surface:3",
                       "payload": {"task": "T", "blocked_by": ["dep1", "dep2"]}}, w, coal)
        self.assertEqual(len(w.blocked), 1)      # 동일 task 중복 갱신
        self.assertEqual(w.blocked[0]["blocked_by"], ["dep1", "dep2"])
        frames, _ = B.route_spool({"ts": time.time(), "type": "task.unblocked",
                                   "key": "surface:3", "payload": {"task": "T"}}, w, coal)
        self.assertEqual(w.blocked, [])
        self.assertTrue(any(f.get("kind") == "unblocked" for f in frames))

    def test_unknown_type_ignored(self):
        w = make_world()
        coal = B.Coalescer()
        frames, poke = B.route_spool(
            {"ts": time.time(), "type": "totally.bogus", "payload": {}}, w, coal)
        self.assertEqual(frames, [])
        self.assertFalse(poke)

    def test_backlog_suppresses_fx_keeps_state(self):
        w = make_world({"worker": "surface:2"})
        coal = B.Coalescer()
        old = time.time() - (B.BACKLOG_FX_SECS + 60)
        frames, _ = B.route_spool(
            {"ts": old, "type": "task.blocked", "key": "surface:2",
             "payload": {"task": "T", "blocked_by": ["d"]}}, w, coal, now=time.time())
        self.assertEqual(len(w.blocked), 1)      # 상태 반영
        self.assertEqual([f for f in frames if f.get("t") == "fx"], [])  # 연출 억제


class TailSpool(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp()
        self.path = os.path.join(self.tmp, "evt_spool.jsonl")

    def _write(self, *entries):
        with open(self.path, "a") as f:
            for e in entries:
                f.write(json.dumps(e, ensure_ascii=False) + "\n")

    def test_absent_file_is_harmless(self):
        entries, off = B.tail_spool(os.path.join(self.tmp, "nope.jsonl"), 0)
        self.assertEqual(entries, [])
        self.assertEqual(off, 0)

    def test_reads_only_new_lines(self):
        self._write({"type": "a", "payload": {}}, {"type": "b", "payload": {}})
        entries, off = B.tail_spool(self.path, 0)
        self.assertEqual([e["type"] for e in entries], ["a", "b"])
        self._write({"type": "c", "payload": {}})
        entries2, off2 = B.tail_spool(self.path, off)
        self.assertEqual([e["type"] for e in entries2], ["c"])
        self.assertGreater(off2, off)

    def test_broken_json_line_skipped(self):
        with open(self.path, "w") as f:
            f.write('{"type":"ok","payload":{}}\n')
            f.write('this is not json\n')
            f.write('{"type":"ok2","payload":{}}\n')
        entries, _ = B.tail_spool(self.path, 0)
        self.assertEqual([e["type"] for e in entries], ["ok", "ok2"])

    def test_truncate_resets_offset(self):
        self._write({"type": "a", "payload": {}}, {"type": "b", "payload": {}})
        _, off = B.tail_spool(self.path, 0)
        with open(self.path, "w") as f:   # truncate + 새 줄
            f.write(json.dumps({"type": "fresh", "payload": {}}) + "\n")
        entries, _ = B.tail_spool(self.path, off)   # off > 새 파일 크기 → 0부터
        self.assertEqual([e["type"] for e in entries], ["fresh"])

    def test_partial_last_line_deferred(self):
        with open(self.path, "w") as f:
            f.write('{"type":"whole","payload":{}}\n{"type":"partial"')  # 개행 없음
        entries, off = B.tail_spool(self.path, 0)
        self.assertEqual([e["type"] for e in entries], ["whole"])
        with open(self.path, "a") as f:   # 나머지 완결
            f.write(',"payload":{}}\n')
        entries2, _ = B.tail_spool(self.path, off)
        self.assertEqual([e["type"] for e in entries2], ["partial"])


class EmitSpool(unittest.TestCase):
    def test_emit_spool_appends_entry(self):
        tmp = tempfile.mkdtemp()
        old = os.environ.pop("HUD_STATE_DIR", None)   # (0.14.56) a rig pack, not the env, places the spool
        try:
            E, spool = rig_event_module(tmp)
            E._spool_append("task.blocked", {"task": "T", "blocked_by": ["d"]}, "surface:4")
            with open(spool) as f:
                e = json.loads(f.readline())
            self.assertEqual(e["type"], "task.blocked")
            self.assertEqual(e["key"], "surface:4")
            self.assertEqual(e["payload"]["task"], "T")
            # tail_spool 이 emit 산출을 그대로 소비 가능한지 왕복 확인
            entries, _ = B.tail_spool(spool, 0)
            self.assertEqual(entries[0]["type"], "task.blocked")
        finally:
            if old is not None:
                os.environ["HUD_STATE_DIR"] = old
            shutil.rmtree(tmp, True)


class KanbanScan(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp()

    def _task(self, name, **fields):
        with open(os.path.join(self.tmp, name), "w") as f:
            json.dump(fields, f)

    def test_status_normalization(self):
        self._task("a.json", id="a", title="A", status="in_progress", owner="worker",
                   blocked_by=[])
        self._task("b.json", id="b", title="B", status="done", owner=None, blocked_by=[])
        self._task("c.json", id="c", title="C", status="pending", owner=None, blocked_by=[])
        self._task("d.json", id="d", title="D", status="in_progress", owner=None,
                   blocked_by=["a"])   # 미완료+선행 → blocked
        kb = B.scan_kanban(self.tmp)
        by = {t["id"]: t["status"] for t in kb["tasks"]}
        self.assertEqual(by, {"a": "doing", "b": "done", "c": "todo", "d": "blocked"})

    def test_non_json_and_broken_skipped(self):
        self._task("ok.json", id="ok", title="OK", status="done", blocked_by=[])
        with open(os.path.join(self.tmp, "broken.json"), "w") as f:
            f.write("{ not json")
        os.mkdir(os.path.join(self.tmp, "sub.lock"))   # 디렉터리(실물 channels-c0.lock 형)
        kb = B.scan_kanban(self.tmp)
        self.assertEqual([t["id"] for t in kb["tasks"]], ["ok"])

    def test_absent_dir_harmless(self):
        kb = B.scan_kanban(os.path.join(self.tmp, "nope"))
        self.assertEqual(kb["tasks"], [])


class VerdictParse(unittest.TestCase):
    def test_markdown_equals_form(self):   # 실물: **verdict = ACCEPT**
        p = B.parse_verdict("**verdict = ACCEPT** (…)", "CSO_VERDICT_OPP21.md")
        self.assertEqual(p["verdict"], "ACCEPT")
        self.assertEqual(p["reviewer"], "CSO")
        self.assertEqual(p["target"], "OPP21")

    def test_json_colon_form(self):        # 실물: "verdict": "REVISE"
        p = B.parse_verdict('{\n  "verdict": "REVISE"\n}', "MACRT_T6B_VERDICT_codex.md")
        self.assertEqual(p["verdict"], "REVISE")
        self.assertEqual(p["reviewer"], "codex")
        self.assertEqual(p["target"], "MACRT_T6B")

    def test_reviewer_from_agy_filename(self):
        p = B.parse_verdict('"verdict": "ACCEPT"', "REVIEWER_AGY_VERDICT_CLI_PATH.md")
        self.assertEqual(p["reviewer"], "agy")
        self.assertEqual(p["target"], "CLI_PATH")

    def test_no_verdict_returns_none(self):
        self.assertIsNone(B.parse_verdict("no verdict token here", "X_VERDICT.md"))

    def test_scan_verdicts_keeps_latest(self):
        tmp = tempfile.mkdtemp()
        for i in range(12):
            fp = os.path.join(tmp, "CSO_VERDICT_P%02d.md" % i)
            with open(fp, "w") as f:
                f.write("verdict = ACCEPT")
            os.utime(fp, (1000 + i, 1000 + i))   # mtime 오름차순
        rv = B.scan_verdicts(tmp, keep=10)
        self.assertEqual(len(rv["items"]), 10)
        self.assertEqual(rv["items"][0]["target"], "P11")   # 최신 우선


class VerdictFx(unittest.TestCase):
    def test_fresh_verdict_emits_fx(self):
        now = time.time()
        items = [{"reviewer": "agy", "verdict": "ACCEPT", "target": "T", "ts": now - 5}]
        frames = B.verdict_fx(items, set(), now=now)
        self.assertEqual(len(frames), 1)
        self.assertEqual(frames[0]["kind"], "verdict")
        self.assertEqual(frames[0]["verdict"], "ACCEPT")

    def test_backlog_verdict_suppresses_fx_but_registers_seen(self):
        now = time.time()
        old_ts = now - (B.BACKLOG_FX_SECS + 60)
        items = [{"reviewer": "codex", "verdict": "REVISE", "target": "T", "ts": old_ts}]
        seen = set()
        frames = B.verdict_fx(items, seen, now=now)
        self.assertEqual(frames, [])                 # 과거 → fx 억제
        self.assertEqual(len(seen), 1)               # seen 등록은 됨 (이후 중복 방지)

    def test_restart_backlog_does_not_flood(self):
        # (재)기동 시 빈 seen에 10건 과거 verdict → fx 폭주 없어야 (결함 회귀 방지)
        now = time.time()
        items = [{"reviewer": "CSO", "verdict": "ACCEPT", "target": "P%d" % i,
                  "ts": now - (B.BACKLOG_FX_SECS + 100)} for i in range(10)]
        frames = B.verdict_fx(items, set(), now=now)
        self.assertEqual(frames, [])

    def test_seen_dedup_prevents_repeat(self):
        now = time.time()
        items = [{"reviewer": "agy", "verdict": "ACCEPT", "target": "T", "ts": now - 1}]
        seen = set()
        self.assertEqual(len(B.verdict_fx(items, seen, now=now)), 1)
        self.assertEqual(B.verdict_fx(items, seen, now=now), [])   # 동일 항목 재방출 없음

    def test_mixed_fresh_and_old(self):
        now = time.time()
        items = [{"reviewer": "agy", "verdict": "ACCEPT", "target": "new", "ts": now - 3},
                 {"reviewer": "agy", "verdict": "BLOCK", "target": "old",
                  "ts": now - (B.BACKLOG_FX_SECS + 30)}]
        frames = B.verdict_fx(items, set(), now=now)
        self.assertEqual([f["target"] for f in frames], ["new"])   # 신선만


class HeatBuckets(unittest.TestCase):
    def test_ratio_arithmetic(self):
        w = make_world({"worker": "surface:1"})
        # active 판정을 강제하기 위해 최근 hook 주입 (P2-3: hooks 는 정식 키로 키잉 — 미주석 시 bare)
        now = time.time()
        w.hooks["surface:1"].append(now)
        for _ in range(3):
            w.accumulate_heat(now)
        hour = time.localtime(now).tm_hour
        acc = w.heat_acc["surface:1"]
        self.assertEqual(acc["total"][hour], 3)
        self.assertEqual(acc["active"][hour], 3)      # 매 틱 active
        board = w.board_snapshot(now)
        self.assertEqual(board["heat"]["surface:1"][hour], 1.0)

    def test_inactive_ratio_zero(self):
        w = make_world({"worker": "surface:1"})
        w.departments[0]["surfaces"][0]["idle_secs"] = 5000   # sleeping
        now = time.time()
        w.accumulate_heat(now)
        hour = time.localtime(now).tm_hour
        self.assertEqual(w.board_snapshot(now)["heat"]["surface:1"][hour], 0.0)


class CostBestEffort(unittest.TestCase):
    def test_absent_db_returns_null(self):
        out = B.read_cost_today("/nonexistent/path/x.db")
        self.assertEqual(out, {"usd": None, "tokens": None})

    def test_schema_without_cost_columns_returns_null(self):
        import sqlite3
        tmp = tempfile.mkdtemp()
        db = os.path.join(tmp, "t.db")
        con = sqlite3.connect(db)
        con.execute("CREATE TABLE lines(id INTEGER, ts REAL, line TEXT)")
        con.execute("INSERT INTO lines VALUES (1, ?, 'x')", (time.time(),))
        con.commit(); con.close()
        out = B.read_cost_today(db)     # 비용/토큰 컬럼 없음 → null (죽지 않음)
        self.assertEqual(out, {"usd": None, "tokens": None})


class TopFrame(unittest.TestCase):
    def test_patch_top_only_on_change(self):
        w = make_world({"worker": "surface:1"})
        coal = B.Coalescer()
        B.route_spool({"ts": time.time(), "type": "task.blocked", "key": "surface:1",
                       "payload": {"task": "T", "blocked_by": ["d"]}}, w, coal)
        fr = w.top_frame("blocked")
        self.assertIsNotNone(fr)
        self.assertEqual(fr["field"], "blocked")
        self.assertIsNone(w.top_frame("blocked"))   # 변화 없음 → None

    def test_board_dedup_and_reemit_on_change(self):
        # ③ prev_top 락 내부화 회귀 — board도 1회만·변경 시 재방출
        w = make_world({"worker": "surface:1"})
        now = time.time()
        w.hooks["surface:1"].append(now)   # P2-3: hooks 정식 키 키잉(미주석 시 bare)
        w.accumulate_heat(now)
        self.assertIsNotNone(w.top_frame("board", now=now))
        self.assertIsNone(w.top_frame("board", now=now))          # 동일 → None
        w.cost_cache = {"ts": now, "value": {"usd": 1.5, "tokens": 100}}
        self.assertIsNotNone(w.top_frame("board", now=now))       # 값 변경 → 재방출


class SpoolRotate(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp()
        self.path = os.path.join(self.tmp, "evt_spool.jsonl")
        self._orig = B.SPOOL_ROTATE_BYTES

    def tearDown(self):
        B.SPOOL_ROTATE_BYTES = self._orig   # 전역 상한 원복

    def test_below_threshold_noop(self):
        with open(self.path, "w") as f:
            f.write('{"type":"a","payload":{}}\n')
        size = os.path.getsize(self.path)
        rotated, leftover, off = B.maybe_rotate_spool(self.path, size)  # 기본 4MB 미만
        self.assertFalse(rotated)
        self.assertEqual(leftover, [])
        self.assertEqual(off, size)
        self.assertTrue(os.path.exists(self.path))   # 파일 유지

    def test_drain_rotation_resets_offset_and_keeps_the_archive(self):
        # (0.14.56 X-8 CARRIER · contract change, DESIGN-r3 §3) the pin deleted the rotated spool (.old removed); now the
        # spool is renamed to evt_spool.<UTC>.jsonl beside it and kept with every byte (CEO E5: never delete a spool).
        B.SPOOL_ROTATE_BYTES = 5
        with open(self.path, "w") as f:
            f.write('{"type":"a","payload":{}}\n')
        size = os.path.getsize(self.path)          # > 5, 완전 드레인(offset==size)
        rotated, leftover, off = B.maybe_rotate_spool(self.path, size)
        self.assertTrue(rotated)
        self.assertEqual(off, 0)                    # offset 리셋
        self.assertEqual(leftover, [])              # 잔여 없음
        self.assertFalse(os.path.exists(self.path))            # 원본 rename됨(방출자가 새로 생성)
        self.assertFalse(os.path.exists(self.path + ".old"))   # .old 는 만들지 않는다
        kept = [n for n in os.listdir(self.tmp) if n != "evt_spool.jsonl"]
        self.assertEqual(len(kept), 1, kept)
        self.assertRegex(kept[0], r"^evt_spool\.\d{8}T\d{6}Z\.jsonl$")   # 보관본이 남는다
        with open(os.path.join(self.tmp, kept[0])) as f:
            self.assertEqual(f.read(), '{"type":"a","payload":{}}\n')   # 바이트 그대로

    def test_landing_write_not_lost(self):
        # 마지막 tail~rename 창에 착지한 완결 줄을 offset 이후 내용으로 재현 → 무유실 회수
        B.SPOOL_ROTATE_BYTES = 5
        drained = '{"type":"a","payload":{}}\n'        # 이미 소비한 부분
        landing = '{"type":"late","payload":{}}\n'     # 창에서 착지한 부분
        with open(self.path, "w") as f:
            f.write(drained + landing)
        offset = len(drained.encode())                 # tail이 여기까지 소비했다고 가정
        rotated, leftover, off = B.maybe_rotate_spool(self.path, offset)
        self.assertTrue(rotated)
        self.assertEqual(off, 0)
        self.assertEqual([e["type"] for e in leftover], ["late"])   # 착지분 회수
        self.assertFalse(os.path.exists(self.path + ".old"))

    def test_no_rotation_when_far_below_threshold(self):
        B.SPOOL_ROTATE_BYTES = 10 ** 9
        with open(self.path, "w") as f:
            f.write('{"type":"a","payload":{}}\n')
        rotated, _, _ = B.maybe_rotate_spool(self.path, os.path.getsize(self.path))
        self.assertFalse(rotated)


class CoalescerShared(unittest.TestCase):
    def test_single_instance_budget_capped(self):
        c = B.Coalescer(budget=5)
        allowed = sum(1 for _ in range(20) if c.allow("x.event", "surface:1", now=100.0))
        self.assertEqual(allowed, 5)   # 고정 now → 리필 0 → 정확히 예산만 (2배 아님)

    def test_thread_safe_no_overshoot(self):
        import threading as _t
        c = B.Coalescer(budget=50)
        results = []
        rlock = _t.Lock()

        def worker():
            local = sum(1 for _ in range(200) if c.allow("x.event", "surface:1", now=100.0))
            with rlock:
                results.append(local)

        threads = [_t.Thread(target=worker) for _ in range(8)]
        for t in threads:
            t.start()
        for t in threads:
            t.join()
        self.assertEqual(sum(results), 50)   # 락 + 고정 now → 총 허용 == 예산 (초과·경합 손실 0)


class SnapshotShape(unittest.TestCase):
    def test_contract_fields_present(self):
        w = make_world({"worker": "surface:1"})
        snap = w.snapshot()
        for field in ("blocked", "kanban", "review", "board"):
            self.assertIn(field, snap)
        self.assertIn("heat", snap["board"])
        self.assertIn("cost_today", snap["board"])
        node = snap["departments"][0]["nodes"][0]
        self.assertIn("progress", node)
        self.assertIn("run", node)
        self.assertIn("dept_label", node)   # v2: 표시 전용 라벨 필드 추가
        self.assertEqual(snap["v"], 2)   # v2 부서 한정 키 계약


if __name__ == "__main__":
    unittest.main()
