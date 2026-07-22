#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_report_gate.py — javis_report_gate.py + javis_gate_check.py 회귀.

DESIGN §C1 필수 케이스 10종을 Gate 코어에 대역 Runner를 주입해 핀한다(서버·데몬 기동 0).
외부 명령(javis_report/event/wakeup·cys)은 전부 FakeRunner로 대체 — 호출 여부·인자를 기록해
"배달 체인 완결(enqueue+drain)"·"emit 거부 폴백"·"fail-open 직송" 등 부작용을 검증한다.

실행: python3 test_report_gate.py   (unittest·표준 러너 — CI가 파일 직접 실행하는 관례 준거)
"""
import json
import os
import subprocess
import sys
import tempfile
import unittest

BIN = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))          # cysjavis-pack/bin
sys.path.insert(0, BIN)
import javis_report_gate as G                                             # noqa: E402


def report(nodes=None, live_nodes=None, idle_nodes=None, feed=None,
           status_available=True, **extra):
    """javis_report.py --json 형태의 report 픽스처."""
    r = {
        "overall_pct": 0, "overall_done": 0, "overall_total": 0,
        "nodes": nodes or [], "live_nodes": live_nodes or [],
        "idle_nodes": idle_nodes or [], "feed_pending": feed,
        "paused": None, "status_available": status_available,
    }
    r.update(extra)
    return r


class FakeRunner:
    def __init__(self, report_ok=True, rep=None, err=None, emit_rc=0,
                 drain_delivered=1, collect_raises=False, enqueue_rc=0):
        self.report_ok, self.rep, self.err = report_ok, rep, err
        self.emit_rc, self.drain_delivered = emit_rc, drain_delivered
        self.collect_raises = collect_raises
        self.enqueue_rc = enqueue_rc            # enqueue 실패 주입(T21 원자성 검증용)
        self.emits, self.enqueues, self.drains, self.sends = [], [], [], []
        self.collect_calls = 0

    def collect_report(self):
        self.collect_calls += 1
        if self.collect_raises:
            raise RuntimeError("주입된 내부 오류")
        return self.report_ok, self.rep, self.err

    def emit(self, evt_type, fields, surface="auto"):
        self.emits.append((evt_type, fields))
        return self.emit_rc, "", ""

    def enqueue(self, to, task, reason, idem, payload=None):
        self.enqueues.append((to, task, reason, idem))
        return self.enqueue_rc

    def drain(self, target):
        self.drains.append(target)
        return 0, self.drain_delivered

    def send_queued(self, to, body):
        self.sends.append((to, body))
        return 0


class Clock:
    """주입 가능한 시계 — GAP·briefing 테스트용."""
    def __init__(self, epoch):
        self.epoch = epoch

    def now_epoch(self):
        return self.epoch

    def now_iso(self):
        return "2026-07-18T%02d:00:00+0900" % (int(self.epoch // 3600) % 24)


def gate(state_dir, runner, clock=None, stall_cycles=2, quiet_cycles=3, edge_cooldown=None):
    clk = clock or Clock(1_000_000.0)
    kw = {} if edge_cooldown is None else {"edge_cooldown": edge_cooldown}
    return G.Gate(state_dir, runner, cycle_minutes=5, stall_cycles=stall_cycles,
                  quiet_cycles=quiet_cycles,
                  now_epoch_fn=clk.now_epoch, now_iso_fn=clk.now_iso, **kw)


def ledger_entries(state_dir):
    path = os.path.join(state_dir, "ledger.jsonl")
    with open(path, encoding="utf-8") as f:
        return [json.loads(l) for l in f if l.strip()]


class GateCore(unittest.TestCase):

    # ── ① BASELINE(스냅샷 부재) ──
    def test_baseline_records_no_delivery(self):
        with tempfile.TemporaryDirectory() as t:
            r = FakeRunner(rep=report(nodes=[{"node": "worker", "done": 1, "total": 3, "pct": 33}]))
            gate(t, r).run()
            e = ledger_entries(t)[-1]
            self.assertEqual(e["verdict"], "BASELINE")
            self.assertEqual(r.enqueues, [])
            self.assertEqual(r.emits, [])
            self.assertTrue(os.path.isfile(os.path.join(t, "last_snapshot.json")))

    # ── ② 무변화 → NOCHG / QUIET ──
    def test_no_change_in_progress_is_nochg(self):
        with tempfile.TemporaryDirectory() as t:
            rep = report(nodes=[{"node": "worker", "done": 1, "total": 3, "pct": 33}],
                         live_nodes=[{"role": "worker", "agent_alive": True, "idle_secs": 10,
                                      "context_pct": 20}])
            r = FakeRunner(rep=rep)
            gate(t, r).run()                                  # baseline
            gate(t, r).run()                                  # 2nd = no change
            e = ledger_entries(t)[-1]
            self.assertEqual(e["verdict"], "NOCHG")
            self.assertEqual(r.enqueues, [])
            self.assertEqual(r.emits, [])

    def test_no_change_all_idle_no_work_is_quiet(self):
        with tempfile.TemporaryDirectory() as t:
            rep = report(nodes=[{"node": "worker", "done": 3, "total": 3, "pct": 100}],
                         live_nodes=[{"role": "worker", "agent_alive": True, "idle_secs": 600}],
                         idle_nodes=[{"role": "worker", "idle_secs": 600}])   # done이라 idle WARN 아님
            r = FakeRunner(rep=rep)
            gate(t, r).run()
            gate(t, r).run()
            e = ledger_entries(t)[-1]
            self.assertEqual(e["verdict"], "QUIET")
            self.assertEqual(r.enqueues, [])

    # ── ③ 경고 주입 → WARN + 배달(enqueue+drain) 호출 ──
    def test_warning_triggers_wake_and_drain(self):
        with tempfile.TemporaryDirectory() as t:
            rep = report(nodes=[{"node": "worker", "done": 1, "total": 5, "pct": 20}],
                         idle_nodes=[{"role": "worker", "idle_secs": 600}])
            r = FakeRunner(rep=rep)
            gate(t, r).run()                                  # baseline (no delivery)
            gate(t, r).run()
            e = ledger_entries(t)[-1]
            self.assertEqual(e["verdict"], "WARN")
            self.assertEqual(len(r.enqueues), 1)
            self.assertEqual(r.enqueues[0][0], "master")
            self.assertEqual(r.drains, ["master"])            # 배달 체인 완결(drain 필수)
            self.assertEqual(e["delivered"], "wake")

    def test_multi_idle_nodes_separate_per_node_wake_keys(self):
        # master 승인 2026-07-18: idle 노드별로 task/idem 분리 → 큐 병합 최대화.
        # [v5 엣지] 무배정 리뷰어는 active→idle '전이' 시 엣지 1회. baseline에 이미 idle이면 disarm
        #   초기화라 발화하지 않으므로(§3.2·D7), baseline은 active·2주기에 idle 진입으로 셋업한다.
        with tempfile.TemporaryDirectory() as t:
            base = report(live_nodes=[{"role": "reviewer-codex", "agent_alive": True, "idle_secs": 10},
                                      {"role": "reviewer-gemini", "agent_alive": True, "idle_secs": 10}])
            gate(t, FakeRunner(rep=base)).run()               # baseline: 리뷰어 active(idle_nodes 없음)
            rep = report(live_nodes=[{"role": "reviewer-codex", "agent_alive": True, "idle_secs": 600},
                                     {"role": "reviewer-gemini", "agent_alive": True, "idle_secs": 700}],
                         idle_nodes=[{"role": "reviewer-codex", "idle_secs": 600},
                                     {"role": "reviewer-gemini", "idle_secs": 700}])
            r = FakeRunner(rep=rep)
            gate(t, r).run()                                  # 무배정 idle 전이 → 엣지 각 1회
            tasks = sorted(task for _to, task, _reason, _idem in r.enqueues)
            idems = sorted(idem for _to, _task, _reason, idem in r.enqueues)
            self.assertEqual(tasks, ["gate-idle-reviewer-codex", "gate-idle-reviewer-gemini"])
            self.assertEqual(idems, ["gate-idle-reviewer-codex", "gate-idle-reviewer-gemini"])
            self.assertEqual(r.drains, ["master"])            # 여러 enqueue·drain 1회(WARN 주기)

    def test_feed_pending_warns_with_approval_evt(self):
        with tempfile.TemporaryDirectory() as t:
            rep = report(feed=2)
            r = FakeRunner(rep=rep)
            gate(t, r).run()
            gate(t, r).run()
            e = ledger_entries(t)[-1]
            self.assertEqual(e["verdict"], "WARN")
            self.assertIn("approval.needed", [t for t, _ in r.emits])

    # ── ④ 태스크별 stall(6주기·노드 idle) + busy 시 보류 ──
    def test_per_task_stall_promotes_when_idle(self):
        with tempfile.TemporaryDirectory() as t:
            rep = report(nodes=[{"node": "worker", "done": 1, "total": 5, "pct": 20}],
                         live_nodes=[{"role": "worker", "agent_alive": True, "idle_secs": 600}])
            r = FakeRunner(rep=rep)
            g = lambda: gate(t, r, stall_cycles=2).run()      # noqa: E731
            g()                                               # baseline
            g()                                               # count=0
            g()                                               # count=1
            g()                                               # count=2 → stall
            e = ledger_entries(t)[-1]
            self.assertEqual(e["verdict"], "WARN")
            self.assertIn("agent.silent", [t for t, _ in r.emits])
            self.assertTrue(any("stall:worker" in x for x in e["reasons"]))

    def test_stall_held_when_node_busy(self):
        with tempfile.TemporaryDirectory() as t:
            rep = report(nodes=[{"node": "worker", "done": 1, "total": 5, "pct": 20}],
                         live_nodes=[{"role": "worker", "agent_alive": True, "idle_secs": 30}])
            r = FakeRunner(rep=rep)
            for _ in range(6):
                gate(t, r, stall_cycles=2).run()
            e = ledger_entries(t)[-1]
            self.assertEqual(e["verdict"], "NOCHG")           # busy → 승격 보류
            self.assertFalse(any("stall" in x for x in e["reasons"]))

    # ── ⑤ GAP re-baseline ──
    def test_gap_rebaselines_without_wake(self):
        with tempfile.TemporaryDirectory() as t:
            rep = report(nodes=[{"node": "worker", "done": 1, "total": 5, "pct": 20}],
                         idle_nodes=[{"role": "worker", "idle_secs": 600}])
            clk = Clock(1_000_000.0)
            gate(t, FakeRunner(rep=rep), clock=clk).run()     # baseline at t0
            clk.epoch = 1_000_000.0 + 16 * 60                 # +16분 > 3주기(15분)
            r2 = FakeRunner(rep=rep)
            gate(t, r2, clock=clk).run()
            e = ledger_entries(t)[-1]
            self.assertEqual(e["verdict"], "GAP")
            self.assertEqual(r2.enqueues, [])                 # wake 금지
            self.assertEqual(r2.drains, [])

    # ── ⑥ fail-open(내부 예외 주입 → 직송 호출) ──
    def test_fail_open_direct_sends_and_records(self):
        with tempfile.TemporaryDirectory() as t:
            r = FakeRunner(collect_raises=True)
            rc = gate(t, r).run()
            self.assertEqual(rc, 0)                            # 죽지 않는다
            self.assertEqual(len(r.sends), 1)
            self.assertEqual(r.sends[0][0], "master")
            e = ledger_entries(t)[-1]
            self.assertEqual(e["verdict"], "FAILOPEN")

    def test_fail_open_streak_note_after_three(self):
        with tempfile.TemporaryDirectory() as t:
            r = FakeRunner(collect_raises=True)
            for _ in range(3):
                gate(t, r).run()
            self.assertIn("게이트 자체 수리 필요", r.sends[-1][1])

    # ── P1-1: state_dir 접근 불가(권한/락 OSError)도 최상위 fail-open으로 exit 0 + 직송 ──
    @unittest.skipIf(os.geteuid() == 0, "root는 파일권한 무시 — chmod 555 재현 불가")
    def test_fail_open_when_state_dir_unwritable(self):
        with tempfile.TemporaryDirectory() as t:
            sd = os.path.join(t, "state")
            os.makedirs(sd)
            r = FakeRunner(rep=report(nodes=[{"node": "worker", "done": 1, "total": 3, "pct": 33}]))
            os.chmod(sd, 0o555)                               # 읽기·실행만 — 락 mkdir 불가
            try:
                rc = gate(sd, r).run()
            finally:
                os.chmod(sd, 0o755)                           # 정리 위해 복구
            self.assertEqual(rc, 0)                            # 죽지 않는다(exit 1 금지)
            self.assertEqual(len(r.sends), 1)                  # master 직송 시도
            self.assertEqual(r.sends[0][0], "master")
            self.assertIn("state 기록 불가", r.sends[0][1])

    # ── ⑦ 블랙리스트 정규화(타임스탬프만 다른 입력 = 무변화) ──
    def test_blacklist_normalization_timestamp_only_no_change(self):
        with tempfile.TemporaryDirectory() as t:
            base = report(nodes=[{"node": "worker", "done": 1, "total": 5, "pct": 20}],
                          live_nodes=[{"role": "worker", "agent_alive": True, "idle_secs": 10}])
            gate(t, FakeRunner(rep=base), Clock(1000.0)).run()          # baseline
            # idle_secs(블랙리스트)·ts만 변화 → 정규화 후 동일 → 무변화
            drift = report(nodes=[{"node": "worker", "done": 1, "total": 5, "pct": 20}],
                           live_nodes=[{"role": "worker", "agent_alive": True, "idle_secs": 99}],
                           ts="2026-07-18T09:05:00+0900")
            r = FakeRunner(rep=drift)
            gate(t, r, Clock(1100.0)).run()
            e = ledger_entries(t)[-1]
            self.assertEqual(e["verdict"], "NOCHG")
            self.assertEqual(e["delta_fields"], [])
            self.assertEqual(r.emits, [])

    # ── ⑧ 미지 신규 필드 = 변화로 감지 ──
    def test_unknown_new_field_detected_as_delta(self):
        with tempfile.TemporaryDirectory() as t:
            base = report(nodes=[{"node": "worker", "done": 1, "total": 5, "pct": 20}],
                          live_nodes=[{"role": "worker", "agent_alive": True, "idle_secs": 10}])
            gate(t, FakeRunner(rep=base)).run()               # baseline
            grown = report(nodes=[{"node": "worker", "done": 1, "total": 5, "pct": 20}],
                           live_nodes=[{"role": "worker", "agent_alive": True, "idle_secs": 10}],
                           brand_new_field={"x": 1})          # 화이트리스트 아님 → diff 대상
            r = FakeRunner(rep=grown)
            gate(t, r).run()
            e = ledger_entries(t)[-1]
            self.assertEqual(e["verdict"], "DELTA")
            self.assertIn("brand_new_field", e["delta_fields"])
            self.assertIn("task_progress", [t for t, _ in r.emits])

    # ── ⑨ emit 거부 폴백(deny-by-default) ──
    def test_emit_reject_recorded_no_silent_loss(self):
        with tempfile.TemporaryDirectory() as t:
            base = report(nodes=[{"node": "worker", "done": 1, "total": 5, "pct": 20}])
            gate(t, FakeRunner(rep=base)).run()
            grown = report(nodes=[{"node": "worker", "done": 2, "total": 5, "pct": 40}])
            r = FakeRunner(rep=grown, emit_rc=6)              # 6 = deny-by-default 거부
            gate(t, r).run()
            e = ledger_entries(t)[-1]
            self.assertEqual(e["verdict"], "DELTA")
            self.assertEqual(e["delivered"], "none")          # emit 실패 → 배달 없음
            self.assertTrue(any("evt_reject:task_progress(6)" in x for x in e["reasons"]))
            self.assertEqual(r.enqueues, [])                  # DELTA는 WARN급 아님 → 폴백 wake 안 함

    # ── P2-3: schema_version 부착(counters·snapshot·ledger) ──
    def test_schema_version_on_state_files(self):
        with tempfile.TemporaryDirectory() as t:
            r = FakeRunner(rep=report(nodes=[{"node": "worker", "done": 1, "total": 3, "pct": 33}]))
            gate(t, r).run()
            counters = json.load(open(os.path.join(t, "counters.json"), encoding="utf-8"))
            snap = json.load(open(os.path.join(t, "last_snapshot.json"), encoding="utf-8"))
            self.assertEqual(counters.get("schema_version"), 2)   # counters v2(idle_edge·park_notified)
            self.assertEqual(snap.get("schema_version"), 1)       # snapshot·ledger는 v1 불변
            self.assertIn("data", snap)                       # 스냅샷 본문은 래퍼 안(diff 오탐 방지)
            self.assertEqual(ledger_entries(t)[-1]["schema_version"], 1)

    # ── T12: counters v1 로드 → v2 자연 마이그레이션(추가 전용) ──
    def test_counters_v1_migrates_to_v2(self):
        with tempfile.TemporaryDirectory() as t:
            rep = report(nodes=[{"node": "worker", "done": 1, "total": 3, "pct": 33}],
                         live_nodes=[{"role": "worker", "agent_alive": True, "idle_secs": 10}])
            # v1 counters(구 스키마 — idle_edge/park_notified 부재) + 스냅샷 선기록
            gate(t, FakeRunner(rep=rep)).run()                # baseline → snapshot 생성
            with open(os.path.join(t, "counters.json"), "w", encoding="utf-8") as f:
                json.dump({"nodes": {}, "consecutive_nochg": 4, "schema_version": 1}, f)
            gate(t, FakeRunner(rep=rep)).run()                # v1 로드 → v2 기록
            counters = json.load(open(os.path.join(t, "counters.json"), encoding="utf-8"))
            self.assertEqual(counters.get("schema_version"), 2)
            self.assertIn("idle_edge", counters)              # 신규 필드 자연 마이그레이션

    def test_wrapped_snapshot_roundtrips_no_false_delta(self):
        # 래핑 스냅샷 로드→재정규화→diff가 schema_version 때문에 오탐 DELTA를 내지 않아야 한다.
        with tempfile.TemporaryDirectory() as t:
            rep = report(nodes=[{"node": "worker", "done": 1, "total": 3, "pct": 33}],
                         live_nodes=[{"role": "worker", "agent_alive": True, "idle_secs": 10}])
            r = FakeRunner(rep=rep)
            gate(t, r).run()                                  # baseline (wrapped snapshot)
            gate(t, r).run()                                  # 무변화
            e = ledger_entries(t)[-1]
            self.assertEqual(e["verdict"], "NOCHG")
            self.assertEqual(e["delta_fields"], [])

    # ── P2-4: ledger tail-read + 5MB 로테이션 ──
    def test_ledger_rotation_at_threshold(self):
        with tempfile.TemporaryDirectory() as t:
            saved = G.LEDGER_MAX_BYTES
            G.LEDGER_MAX_BYTES = 200
            try:
                for _ in range(20):
                    G.ledger_append(t, {"ts": "x", "verdict": "NOCHG", "pad": "y" * 40})
            finally:
                G.LEDGER_MAX_BYTES = saved
            self.assertTrue(os.path.isfile(os.path.join(t, "ledger.jsonl")))
            self.assertTrue(os.path.isfile(os.path.join(t, "ledger.jsonl.1")))  # 1세대 보관

    def test_last_ledger_tail_read_returns_last(self):
        with tempfile.TemporaryDirectory() as t:
            for i in range(5):
                G.ledger_append(t, {"ts": "x", "verdict": "NOCHG", "seq": i})
            last = G.last_ledger(t)
            self.assertEqual(last["seq"], 4)                  # 마지막 줄

    # ── ⑩ 동시 실행 락 ──
    def test_concurrent_lock_skips(self):
        with tempfile.TemporaryDirectory() as t:
            os.makedirs(t, exist_ok=True)
            os.mkdir(os.path.join(t, "lock"))                 # 락 선점(비-stale)
            r = FakeRunner(rep=report())
            g = gate(t, r)
            rc = g.run()
            self.assertEqual(rc, 0)
            e = ledger_entries(t)[-1]
            self.assertEqual(e["verdict"], "SKIPPED_CONCURRENT")
            self.assertEqual(r.enqueues, [])

    # ── 최종 stdout 판정 요약 1줄(schedule.command_done 텔레메트리) ──
    def test_summary_line_emitted(self):
        with tempfile.TemporaryDirectory() as t:
            script = os.path.join(BIN, "javis_report_gate.py")
            env = dict(os.environ, CYS_REPORT_GATE_DIR=t)
            # collect 실패 유도(pack_bin의 javis_report가 없는 임시 pack) → WARN 경로·요약 출력
            env["CYS_PACK_DIR"] = tempfile.mkdtemp()
            p = subprocess.run([sys.executable, script, "run", "--shadow"],
                               capture_output=True, text=True, env=env)
            self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
            self.assertTrue(p.stdout.strip().splitlines()[-1].startswith("verdict="),
                            p.stdout)


def _counters(state_dir):
    return json.load(open(os.path.join(state_dir, "counters.json"), encoding="utf-8"))


class IdleEdge(unittest.TestCase):
    """무배정 idle 엣지 1회 wake(DESIGN §3.2 v2.2) — T1~T7·T15·T21~T23."""

    ACTIVE = {"role": "worker", "agent_alive": True, "idle_secs": 10}
    IDLE = {"role": "worker", "agent_alive": True, "idle_secs": 600}

    def _idle_rep(self):
        return report(live_nodes=[dict(self.IDLE)], idle_nodes=[{"role": "worker", "idle_secs": 600}])

    def _active_rep(self):
        return report(live_nodes=[dict(self.ACTIVE)])

    # ── T1: 무배정 idle 진입 → wake 1회 + armed=False ──
    def test_t1_unassigned_idle_edge_fires_once_and_disarms(self):
        with tempfile.TemporaryDirectory() as t:
            gate(t, FakeRunner(rep=self._active_rep())).run()          # baseline active
            r = FakeRunner(rep=self._idle_rep())
            gate(t, r).run()                                           # idle 전이 → 엣지 발화
            e = ledger_entries(t)[-1]
            self.assertEqual(e["verdict"], "WARN")
            self.assertEqual([task for _to, task, _r, _i in r.enqueues], ["gate-idle-worker"])
            self.assertTrue(any("idle_edge:worker" in x for x in e["reasons"]))
            self.assertFalse(_counters(t)["idle_edge"]["worker"]["armed"])

    # ── T2: 동일 상태 지속 → 추가 wake 0 + QUIET 누적 ──
    def test_t2_persisting_idle_no_extra_wake_quiet_accumulates(self):
        with tempfile.TemporaryDirectory() as t:
            gate(t, FakeRunner(rep=self._active_rep())).run()
            r = FakeRunner(rep=self._idle_rep())
            gate(t, r).run()                                           # 엣지 1
            master_wakes = lambda: [e for e in r.enqueues if e[0] == "master"]
            self.assertEqual(len(master_wakes()), 1)
            for _ in range(3):
                gate(t, r).run()
            self.assertEqual(len(master_wakes()), 1)                   # 추가 master idle wake 0
            e = ledger_entries(t)[-1]
            self.assertEqual(e["verdict"], "QUIET")
            self.assertGreaterEqual(e["consecutive_quiet"], 1)

    # ── T3: 활동 재개 → re-arm → 쿨다운 경과 후 재진입 시 다시 1회 ──
    def test_t3_reactivate_rearms_and_refires_after_cooldown(self):
        with tempfile.TemporaryDirectory() as t:
            clk = Clock(1_000_000.0)
            gate(t, FakeRunner(rep=self._active_rep()), clock=clk, edge_cooldown=60).run()
            clk.epoch += 300
            r1 = FakeRunner(rep=self._idle_rep())
            gate(t, r1, clock=clk, edge_cooldown=60).run()             # 발화 #1
            self.assertEqual(len(r1.enqueues), 1)
            clk.epoch += 300
            gate(t, FakeRunner(rep=self._active_rep()), clock=clk, edge_cooldown=60).run()  # 활동 재개→재무장
            clk.epoch += 300
            r2 = FakeRunner(rep=self._idle_rep())
            gate(t, r2, clock=clk, edge_cooldown=60).run()             # 쿨다운 경과 재진입 → 발화 #2
            self.assertEqual(len(r2.enqueues), 1)

    # ── T4: pending-todo idle → 레벨 WARN 유지(현행 동등) ──
    def test_t4_pending_todo_idle_level_warn_persists(self):
        with tempfile.TemporaryDirectory() as t:
            rep = report(nodes=[{"node": "worker", "done": 1, "total": 5, "pct": 20}],
                         live_nodes=[dict(self.IDLE)],
                         idle_nodes=[{"role": "worker", "idle_secs": 600}])
            r = FakeRunner(rep=rep)
            gate(t, r).run()                                           # baseline
            for _ in range(3):
                gate(t, r).run()
            warns = [e for e in ledger_entries(t) if e["verdict"] == "WARN"]
            self.assertGreaterEqual(len(warns), 3)                     # 레벨: 매 주기 발화
            self.assertTrue(all(any("idle_5min:worker" in x for x in e["reasons"]) for e in warns))

    # ── T5: done==total idle → 무발화(현행 동등) ──
    def test_t5_done_total_idle_no_fire(self):
        with tempfile.TemporaryDirectory() as t:
            rep = report(nodes=[{"node": "worker", "done": 3, "total": 3, "pct": 100}],
                         live_nodes=[dict(self.IDLE)],
                         idle_nodes=[{"role": "worker", "idle_secs": 600}])
            r = FakeRunner(rep=rep)
            gate(t, r).run()
            gate(t, r).run()
            self.assertEqual(r.enqueues, [])
            self.assertEqual(ledger_entries(t)[-1]["verdict"], "QUIET")

    # ── T6: BASELINE에 이미 idle이던 role → disarmed(파도 없음) ──
    def test_t6_baseline_idle_disarmed_no_flood(self):
        with tempfile.TemporaryDirectory() as t:
            r = FakeRunner(rep=self._idle_rep())
            gate(t, r).run()                                           # baseline: 이미 idle → disarm
            gate(t, r).run()                                           # 차기: disarmed → 발화 0
            self.assertEqual(r.enqueues, [])
            self.assertFalse(_counters(t)["idle_edge"]["worker"]["armed"])

    # ── T7: GAP 후 → disarmed + 사망 미발화(조기 반환) ──
    def test_t7_gap_disarms_and_no_death_fire(self):
        with tempfile.TemporaryDirectory() as t:
            clk = Clock(1_000_000.0)
            gate(t, FakeRunner(rep=self._active_rep()), clock=clk).run()   # baseline
            clk.epoch += 16 * 60                                       # +16분 > 3주기 = GAP
            r = FakeRunner(rep=self._idle_rep())
            gate(t, r, clock=clk).run()
            e = ledger_entries(t)[-1]
            self.assertEqual(e["verdict"], "GAP")
            self.assertEqual(r.enqueues, [])                           # GAP wake 금지·엣지 disarm
            self.assertFalse(_counters(t)["idle_edge"]["worker"]["armed"])

    # ── T15: 검증기 자체 검증(네거티브의 네거티브) — disarm 무력화 시 재발화 검출력 ──
    def test_t15_disarm_regression_is_detectable(self):
        with tempfile.TemporaryDirectory() as t:
            gate(t, FakeRunner(rep=self._active_rep())).run()
            gate(t, FakeRunner(rep=self._idle_rep())).run()           # 발화 → disarm
            cpath = os.path.join(t, "counters.json")
            c = json.load(open(cpath, encoding="utf-8"))
            c["idle_edge"]["worker"] = {"armed": True, "last_fired": 0}   # 고의 파손: disarm 무력화 모사
            json.dump(c, open(cpath, "w", encoding="utf-8"))
            r = FakeRunner(rep=self._idle_rep())
            gate(t, r).run()
            self.assertEqual(len(r.enqueues), 1)                      # 재발화 = 회귀 검출 가능(정상은 0)

    # ── T21: enqueue 실패(rc≠0) → armed 유지·재시도 / 성공 시 disarm ──
    def test_t21_enqueue_failure_keeps_armed_retries(self):
        with tempfile.TemporaryDirectory() as t:
            gate(t, FakeRunner(rep=self._active_rep())).run()
            r_fail = FakeRunner(rep=self._idle_rep(), enqueue_rc=1)
            gate(t, r_fail).run()                                     # enqueue 실패
            self.assertEqual(len(r_fail.enqueues), 1)                 # 시도는 함
            self.assertNotIn("worker", _counters(t).get("idle_edge", {}))  # disarm 안 됨(default armed 유지)
            r_ok = FakeRunner(rep=self._idle_rep())
            gate(t, r_ok).run()                                       # 다음 주기 재시도
            self.assertEqual(len(r_ok.enqueues), 1)                   # 재발화
            self.assertFalse(_counters(t)["idle_edge"]["worker"]["armed"])  # 성공 → disarm

    # ── T22: 진동 노드 → wake ≤1회/쿨다운창 ──
    def test_t22_oscillating_node_wake_capped_per_cooldown(self):
        with tempfile.TemporaryDirectory() as t:
            clk = Clock(1_000_000.0)
            gate(t, FakeRunner(rep=self._active_rep()), clock=clk, edge_cooldown=1000).run()
            total = 0
            for i in range(10):
                clk.epoch += 100                                      # 100s 토글 · 쿨다운 1000s
                r = FakeRunner(rep=(self._idle_rep() if i % 2 == 0 else self._active_rep()))
                gate(t, r, clock=clk, edge_cooldown=1000).run()
                total += len(r.enqueues)
            self.assertGreaterEqual(total, 1)
            self.assertLessEqual(total, 2)                            # 진동 5회에도 쿨다운창당 ≤1

    # ── FIX-4: idle-신규 wake_body에 "⚠" 마커(설계 §4 정합) ──
    def test_fix4_edge_wake_body_has_warning_marker(self):
        with tempfile.TemporaryDirectory() as t:
            gate(t, FakeRunner(rep=self._active_rep())).run()
            r = FakeRunner(rep=self._idle_rep())
            gate(t, r).run()
            bodies = [reason for _to, _task, reason, _idem in r.enqueues]   # enqueue 3번째=wake_body
            self.assertTrue(any("⚠" in b and "idle-신규" in b for b in bodies), bodies)

    # ── T23: counters 파손 복원 주기 → idle_edge 초기화 → 재-파도 0(Sim O-1) ──
    def test_t23_counters_corruption_restore_no_edge_flood(self):
        roles = ("a", "b", "c", "d")
        idle_rep = report(live_nodes=[{"role": r, "agent_alive": True, "idle_secs": 600} for r in roles],
                          idle_nodes=[{"role": r, "idle_secs": 600} for r in roles])
        with tempfile.TemporaryDirectory() as t:
            gate(t, FakeRunner(rep=idle_rep)).run()                   # baseline → 4노드 disarm
            gate(t, FakeRunner(rep=idle_rep)).run()                   # QUIET
            with open(os.path.join(t, "counters.json"), "w", encoding="utf-8") as f:
                f.write("{corrupt json")                              # counters 파손
            r = FakeRunner(rep=idle_rep)
            gate(t, r).run()                                          # 복원 주기 → init_idle_edge
            self.assertEqual(r.enqueues, [])                          # 재-파도 0(미적용 시 4발화)


class ParkEdge(unittest.TestCase):
    """park 후보 통보 엣지화 + 배달 완결 + in_progress 재무장(DESIGN §3.4 v2.2·D4·D5)."""

    def _quiet_rep(self, status=True):
        return report(nodes=[{"node": "worker", "done": 3, "total": 3, "pct": 100}],
                      live_nodes=[{"role": "worker", "agent_alive": True, "idle_secs": 600}],
                      idle_nodes=[{"role": "worker", "idle_secs": 600}],
                      status_available=status)

    def _parks(self, state_dir):
        return sum(1 for e in ledger_entries(state_dir) if "park_candidate" in (e.get("reasons") or []))

    # ── T10: QUIET 임계 도달 → park 1회 + drain("cso") 호출 + notified / 이후 0회 ──
    def test_t10_park_fires_once_with_cso_drain(self):
        with tempfile.TemporaryDirectory() as t:
            r = FakeRunner(rep=self._quiet_rep())
            for _ in range(6):                                        # quiet_cycles=3 → 도중 park 1회
                gate(t, r).run()
            self.assertEqual(len([e for e in r.enqueues if e[0] == "cso"]), 1)   # park 1회
            self.assertIn("cso", r.drains)                            # 배달 완결 drain(cso)
            self.assertTrue(_counters(t)["park_notified"])
            self.assertEqual(self._parks(t), 1)

    # ── T11: notified 후 status 토글 DELTA 진동 → 재통보 0(유지) ──
    def test_t11_park_notified_held_through_status_toggle_delta(self):
        with tempfile.TemporaryDirectory() as t:
            r = FakeRunner(rep=self._quiet_rep())
            for _ in range(6):
                gate(t, r).run()                                      # park 1회
            self.assertEqual(self._parks(t), 1)
            for i in range(8):                                        # status 토글 DELTA(in_progress 없음)
                r.rep = self._quiet_rep(status=(i % 2 == 0))
                gate(t, r).run()
            self.assertTrue(_counters(t)["park_notified"])            # DELTA 진동에 해제 안 됨
            r.rep = self._quiet_rep()
            for _ in range(5):                                        # 정상 quiet 재누적
                gate(t, r).run()
            self.assertEqual(self._parks(t), 1)                       # 재통보 0

    # ── T16: master 포함 전-플릿 idle → QUIET 도달 → park 1회(도달성 인과) ──
    def test_t16_full_fleet_idle_reaches_quiet_and_parks(self):
        with tempfile.TemporaryDirectory() as t:
            rep = report(nodes=[{"node": "master", "done": 1, "total": 1, "pct": 100},
                                {"node": "worker", "done": 2, "total": 2, "pct": 100}],
                         live_nodes=[{"role": "master", "agent_alive": True, "idle_secs": 600},
                                     {"role": "worker", "agent_alive": True, "idle_secs": 600}],
                         idle_nodes=[{"role": "master", "idle_secs": 600},
                                     {"role": "worker", "idle_secs": 600}])
            r = FakeRunner(rep=rep)
            for _ in range(6):
                gate(t, r).run()
            self.assertEqual(ledger_entries(t)[-1]["verdict"], "QUIET")
            self.assertEqual(len([e for e in r.enqueues if e[0] == "cso"]), 1)

    # ── T24: park_notified 재무장 — in_progress → 해제 / status DELTA → 유지 (D-FIX-1 이중) ──
    def test_t24_park_rearm_on_in_progress_not_on_status_delta(self):
        with tempfile.TemporaryDirectory() as t:
            r = FakeRunner(rep=self._quiet_rep())
            for _ in range(6):
                gate(t, r).run()                                      # park #1 → notified True
            self.assertTrue(_counters(t)["park_notified"])
            # (A) status 토글 DELTA(in_progress 없음) → 유지
            r.rep = self._quiet_rep(status=False)
            gate(t, r).run()
            self.assertTrue(_counters(t)["park_notified"])
            # (B) task 배정(in_progress 비공백) → 재무장 해제
            r.rep = report(nodes=[{"node": "worker", "done": 1, "total": 3, "pct": 33}],
                           live_nodes=[{"role": "worker", "agent_alive": True, "idle_secs": 600}])
            gate(t, r).run()
            self.assertFalse(_counters(t)["park_notified"])
            # (C) 작업 완료 후 재-조용 → 2차 park 재발화(1회성 사망 아님·S1-4)
            r.rep = self._quiet_rep()
            for _ in range(5):
                gate(t, r).run()
            self.assertEqual(self._parks(t), 2)


class DeathEdge(unittest.TestCase):
    """사망 엣지 WARN — 시딩/확정 분리·"fired" sentinel·부활 cleanup 독립(DESIGN §3.3 v2.2)."""

    ALIVE = report(live_nodes=[{"role": "worker", "agent_alive": True, "idle_secs": 10}])
    DEAD = report(live_nodes=[{"role": "worker", "agent_alive": False}])
    GONE = report(live_nodes=[])

    def _master_tasks(self, r):
        return [task for _to, task, _r, _i in r.enqueues if _to == "master"]

    # ── T8: alive→dead 전이 → death WARN 1회, 차기 주기 0회 ──
    def test_t8_death_transition_fires_once(self):
        with tempfile.TemporaryDirectory() as t:
            gate(t, FakeRunner(rep=self.ALIVE)).run()                 # baseline alive
            r1 = FakeRunner(rep=self.DEAD); gate(t, r1).run()         # 시딩(무발화)
            self.assertEqual(self._master_tasks(r1), [])
            r2 = FakeRunner(rep=self.DEAD); gate(t, r2).run()         # 확정 발화
            e = ledger_entries(t)[-1]
            self.assertEqual(e["verdict"], "WARN")
            self.assertIn("gate-death-worker", self._master_tasks(r2))
            self.assertTrue(any("death:worker" in x for x in e["reasons"]))
            r3 = FakeRunner(rep=self.DEAD); gate(t, r3).run()         # "fired" → 재발화 0
            self.assertEqual(self._master_tasks(r3), [])

    # ── T9: surface 소멸(gone) → death WARN 1회 ──
    def test_t9_surface_disappearance_fires_death(self):
        with tempfile.TemporaryDirectory() as t:
            gate(t, FakeRunner(rep=self.ALIVE)).run()
            gate(t, FakeRunner(rep=self.GONE)).run()                  # 시딩
            r = FakeRunner(rep=self.GONE); gate(t, r).run()           # 확정
            e = ledger_entries(t)[-1]
            self.assertEqual(e["verdict"], "WARN")
            self.assertIn("gate-death-worker", self._master_tasks(r))

    # ── T17: 사망 라이프사이클 — 시딩→확정 1회→"fired" 유지→부활 소거 ──
    def test_t17_death_lifecycle_seed_confirm_fired_resurrect(self):
        with tempfile.TemporaryDirectory() as t:
            gate(t, FakeRunner(rep=self.ALIVE)).run()                 # baseline
            gate(t, FakeRunner(rep=self.DEAD)).run()                  # 시딩
            r_fire = FakeRunner(rep=self.DEAD); gate(t, r_fire).run() # 확정 1회
            self.assertIn("gate-death-worker", self._master_tasks(r_fire))
            self.assertEqual(_counters(t)["death_pending"]["worker"], "fired")
            r_hold = FakeRunner(rep=self.DEAD); gate(t, r_hold).run() # "fired" 유지 → 0회
            self.assertEqual(self._master_tasks(r_hold), [])
            gate(t, FakeRunner(rep=self.ALIVE)).run()                 # 부활 → 상태 소거
            self.assertNotIn("worker", _counters(t).get("death_pending", {}))

    # ── T18: 사망→부활 cleanup(발화 독립)→fresh armed→idle 진입 시 엣지 1회 ──
    def test_t18_resurrection_fresh_armed_then_edge(self):
        with tempfile.TemporaryDirectory() as t:
            gate(t, FakeRunner(rep=self.ALIVE)).run()                 # baseline
            idle_rep = report(live_nodes=[{"role": "worker", "agent_alive": True, "idle_secs": 600}],
                              idle_nodes=[{"role": "worker", "idle_secs": 600}])
            gate(t, FakeRunner(rep=idle_rep)).run()                   # 엣지 1회 → disarm
            gate(t, FakeRunner(rep=self.DEAD)).run()                  # 시딩
            gate(t, FakeRunner(rep=self.DEAD)).run()                  # 확정 발화
            gate(t, FakeRunner(rep=self.ALIVE)).run()                 # 부활 → fresh armed·death_pending 소거
            self.assertTrue(_counters(t)["idle_edge"]["worker"]["armed"])
            r = FakeRunner(rep=idle_rep); gate(t, r).run()            # 재-idle 진입 → 엣지 1회
            self.assertIn("gate-idle-worker", [task for _to, task, _r, _i in r.enqueues])

    # ── T19: 검증기 자체 검증 — "fired" sentinel 무력화 시 재발화 검출력 ──
    def test_t19_death_refire_regression_is_detectable(self):
        with tempfile.TemporaryDirectory() as t:
            gate(t, FakeRunner(rep=self.ALIVE)).run()
            gate(t, FakeRunner(rep=self.DEAD)).run()                  # 시딩
            gate(t, FakeRunner(rep=self.DEAD)).run()                  # 확정 → "fired"
            cpath = os.path.join(t, "counters.json")
            c = json.load(open(cpath, encoding="utf-8"))
            c["death_pending"]["worker"] = 1                          # 고의 파손: sentinel 되돌림
            json.dump(c, open(cpath, "w", encoding="utf-8"))
            r = FakeRunner(rep=self.DEAD); gate(t, r).run()
            self.assertIn("gate-death-worker", self._master_tasks(r))  # 재발화 = 회귀 검출 가능

    # ── FIX-3: death role 키 소문자 정규화(idle_edge·death_pending 키 공간 통일) ──
    def test_fix3_death_role_keys_lowercased(self):
        with tempfile.TemporaryDirectory() as t:
            alive = report(live_nodes=[{"role": "Worker", "agent_alive": True, "idle_secs": 10}])
            dead = report(live_nodes=[{"role": "Worker", "agent_alive": False}])
            gate(t, FakeRunner(rep=alive)).run()                     # baseline
            gate(t, FakeRunner(rep=dead)).run()                      # 시딩
            dp = _counters(t)["death_pending"]
            self.assertIn("worker", dp)                              # 소문자 키
            self.assertNotIn("Worker", dp)                           # 대문자 이중 키 없음

    # ── T20: GAP 주기 중 사망 → 무발화(위양성 차단 트레이드오프 명문화) ──
    def test_t20_death_during_gap_no_fire_by_design(self):
        with tempfile.TemporaryDirectory() as t:
            clk = Clock(1_000_000.0)
            gate(t, FakeRunner(rep=self.ALIVE), clock=clk).run()      # baseline
            clk.epoch += 16 * 60                                      # GAP
            r = FakeRunner(rep=self.DEAD); gate(t, r, clock=clk).run()
            e = ledger_entries(t)[-1]
            self.assertEqual(e["verdict"], "GAP")
            self.assertEqual(self._master_tasks(r), [])               # 사망 무발화(의도)
            self.assertEqual(_counters(t).get("death_pending", {}), {})  # 시딩도 안 됨


class ShadowChecker(unittest.TestCase):
    """javis_gate_check.py — 독립 키워드 규칙 검사기(producer≠evaluator)."""
    CHECK = os.path.join(BIN, "javis_gate_check.py")

    def _run(self, ledger, push_dir, window="300"):
        return subprocess.run([sys.executable, self.CHECK, "--ledger", ledger,
                               "--push-dir", push_dir, "--window", window],
                              capture_output=True, text=True)

    def test_suppressed_with_warning_keyword_is_violation(self):
        with tempfile.TemporaryDirectory() as t:
            ledger = os.path.join(t, "ledger.jsonl")
            with open(ledger, "w", encoding="utf-8") as f:
                f.write(json.dumps({"ts": "x", "ts_epoch": 1000.0, "verdict": "NOCHG"}) + "\n")
            pd = os.path.join(t, "push")
            os.makedirs(pd)
            body = os.path.join(pd, "push1.txt")
            with open(body, "w", encoding="utf-8") as f:
                f.write("주인님께 보고\n  • ⚠ idle 5분+ 노드: worker\n")
            os.utime(body, (1000.0, 1000.0))                  # 억제 시점과 동일 창
            p = self._run(ledger, pd)
            self.assertEqual(p.returncode, 1, p.stdout)       # 오억제 발견
            self.assertIn("오억제 발견 1건", p.stdout)
            self.assertIn("push1.txt", p.stdout)

    def test_clean_suppression_passes(self):
        with tempfile.TemporaryDirectory() as t:
            ledger = os.path.join(t, "ledger.jsonl")
            with open(ledger, "w", encoding="utf-8") as f:
                f.write(json.dumps({"ts": "x", "ts_epoch": 1000.0, "verdict": "NOCHG"}) + "\n")
            pd = os.path.join(t, "push")
            os.makedirs(pd)
            body = os.path.join(pd, "push1.txt")
            with open(body, "w", encoding="utf-8") as f:
                f.write("주인님께 보고\n  • 전체 진행: 40% (2/5 완료)\n")   # 경고 키워드 없음
            os.utime(body, (1000.0, 1000.0))
            p = self._run(ledger, pd)
            self.assertEqual(p.returncode, 0, p.stdout)


class LaunchdMinimalEnv(unittest.TestCase):
    """★fire_command는 launchd 데몬 최소 env를 상속(CYS_PACK_DIR·CYS_SOCKET 부재, PATH에
    /usr/local/bin 없을 수 있음, HOME은 존재). 경로/바이너리 해석이 그 env에서도 성립하는지 핀."""

    def _with_env(self, env, fn):
        saved = dict(os.environ)
        try:
            os.environ.clear()
            os.environ.update(env)
            return fn()
        finally:
            os.environ.clear()
            os.environ.update(saved)

    def test_pack_bin_resolves_without_cys_pack_dir(self):
        # CYS_PACK_DIR 부재 → __file__ 형제(javis_report.py 동거) 디렉터리로 해석.
        home = os.path.expanduser("~")
        got = self._with_env({"HOME": home}, G.default_pack_bin)
        self.assertTrue(os.path.isfile(os.path.join(got, "javis_report.py")),
                        "pack_bin=%s 에 javis_report.py 없음" % got)

    def test_cys_bin_absolute_fallback_when_path_lacks_cys(self):
        # PATH 비움·CYS_BIN 부재 → which 실패 → 절대경로 후보 또는 최후 'cys'. crash 없이 문자열.
        got = self._with_env({"HOME": os.path.expanduser("~"), "PATH": "/nonexistent"},
                             G.resolve_cys_bin)
        self.assertIsInstance(got, str)
        self.assertTrue(got == "cys" or os.path.isabs(got), got)

    def test_cys_bin_env_wins(self):
        got = self._with_env({"HOME": os.path.expanduser("~"), "CYS_BIN": "/custom/cys"},
                             G.resolve_cys_bin)
        self.assertEqual(got, "/custom/cys")

    def test_gate_runs_under_minimal_env(self):
        # 최소 env + 존재하지 않는 pack_bin → collect 실패(WARN 경로) → exit 0(fail-open 계약).
        with tempfile.TemporaryDirectory() as t:
            r = FakeRunner(report_ok=False, err="수집 실패(최소 env)")
            rc = gate(t, r).run()
            self.assertEqual(rc, 0)
            self.assertEqual(ledger_entries(t)[-1]["verdict"], "WARN")


class ForeignDaemonGuard(unittest.TestCase):
    """외부 데몬 가드 — socket-pack 부정합(부서 데몬이 본사 팩 로드) 시 SKIP(핫픽스)."""

    def _with_env(self, env, fn):
        saved = dict(os.environ)
        try:
            os.environ.clear()
            os.environ.update(env)
            return fn()
        finally:
            os.environ.clear()
            os.environ.update(saved)

    def test_hq_pack_with_dept_socket_skips_no_state_touch(self):
        with tempfile.TemporaryDirectory() as t:
            env = {"HOME": os.path.expanduser("~"),
                   "CYS_PACK_DIR": os.path.expanduser("~/.cys/pack"),
                   "CYS_SOCKET": "/tmp/cys-dept-1/cys.sock"}   # 본사 팩 + dept 소켓 = 오염
            r = FakeRunner(rep=report(nodes=[{"node": "worker", "done": 1, "total": 3, "pct": 33}]))
            rc = self._with_env(env, lambda: gate(t, r).run())
            self.assertEqual(rc, 0)
            e = ledger_entries(t)[-1]
            self.assertEqual(e["verdict"], "SKIPPED_FOREIGN_DAEMON")
            self.assertIn("cys-dept-1", e["reasons"][0])        # socket 값 포함
            self.assertFalse(os.path.isfile(os.path.join(t, "counters.json")))  # 카운터 무접촉
            self.assertEqual(r.collect_calls, 0)                # 수집·stall 무접촉
            self.assertEqual(r.enqueues, [])                    # 배달 무접촉

    def test_hq_pack_socket_unset_proceeds(self):
        with tempfile.TemporaryDirectory() as t:
            env = {"HOME": os.path.expanduser("~"),
                   "CYS_PACK_DIR": os.path.expanduser("~/.cys/pack")}   # 소켓 unset = 정합
            r = FakeRunner(rep=report(nodes=[{"node": "worker", "done": 1, "total": 3, "pct": 33}]))
            self._with_env(env, lambda: gate(t, r).run())
            self.assertEqual(ledger_entries(t)[-1]["verdict"], "BASELINE")  # 정상 진행

    def test_dept_pack_matching_socket_proceeds(self):
        with tempfile.TemporaryDirectory() as t:
            deptpack = os.path.join(t, "pack-dept-1")
            os.makedirs(deptpack)
            sd = os.path.join(t, "state")
            env = {"HOME": os.path.expanduser("~"),
                   "CYS_PACK_DIR": deptpack,
                   "CYS_SOCKET": "/tmp/cys-dept-1/cys.sock"}    # 부서 팩 + 해당 dept 소켓 = 정합
            r = FakeRunner(rep=report(nodes=[{"node": "worker", "done": 1, "total": 3, "pct": 33}]))
            self._with_env(env, lambda: gate(sd, r).run())
            self.assertEqual(ledger_entries(sd)[-1]["verdict"], "BASELINE")  # 미래 부서 게이트 호환


class C16ReportScheduleGate(unittest.TestCase):
    """C16이 델타게이트 잡을 5분 보고 체계로 인정하는지(마이그레이션 되돌림 방지) 회귀."""

    import importlib
    P = importlib.import_module("javis_preflight")

    def _c16(self, tmp, jobs, fix=False):
        with open(os.path.join(tmp, "schedule.json"), "w", encoding="utf-8") as f:
            json.dump({"jobs": jobs}, f, ensure_ascii=False)
        saved = dict(os.environ)
        os.environ["CYS_PACK_DIR"] = tmp
        try:
            pf = self.P.Preflight(fix=fix, skips=set(), mode=("fix" if fix else "report"))
            pf.c16_report_schedule()
            with open(os.path.join(tmp, "schedule.json"), encoding="utf-8") as f:
                after = json.load(f)
            return pf.results[-1], after
        finally:
            os.environ.clear()
            os.environ.update(saved)

    _GATE_CMD = ('CYS_REPORT_GATE_DIR="$HOME/.cys/state/report_gate" python3 '
                 '"${CYS_PACK_DIR:-$HOME/.cys/pack}/bin/javis_report_gate.py" run')
    GATE_JOB = {"id": "owner-progress-gate-5min", "every_minutes": 5, "action": "command",
                "command": _GATE_CMD, "if_absent": "skip"}                 # 상태격리 배선된 게이트 잡
    UNWIRED_GATE_JOB = {"id": "owner-progress-gate-5min", "every_minutes": 5, "action": "command",
                        "command": 'python3 "${CYS_PACK_DIR:-$HOME/.cys/pack}/bin/javis_report_gate.py" run',
                        "if_absent": "skip"}                               # 미배선(구 잡)
    PUSH_JOB = {"id": "owner-progress-report-5min", "every_minutes": 5, "action": "push",
                "to": "master", "text_command": "python3 x", "if_absent": "skip"}

    def test_gate_job_only_passes_and_fix_is_noop(self):
        with tempfile.TemporaryDirectory() as t:
            res, _ = self._c16(t, [dict(self.GATE_JOB)])
            self.assertEqual(res["status"], "PASS", res)
            # --fix: 배선된 게이트 잡 존재 → 재추가·재삽입 없음(마이그레이션 보존)
            res2, after = self._c16(t, [dict(self.GATE_JOB)], fix=True)
            self.assertEqual(res2["status"], "PASS")
            ids = [j["id"] for j in after["jobs"]]
            self.assertEqual(ids, ["owner-progress-gate-5min"])   # 구 push 잡 재생성 안 됨

    def test_no_report_job_fails_and_fix_adds_gate_job(self):
        # reviewer1 P1: --fix는 구 push 잡이 아니라 게이트 잡을 추가해야 한다.
        with tempfile.TemporaryDirectory() as t:
            res, _ = self._c16(t, [])
            self.assertEqual(res["status"], "FAIL", res)
            res2, after = self._c16(t, [], fix=True)
            self.assertEqual(res2["status"], "FIXED")
            added = [j for j in after["jobs"] if j["id"] == "owner-progress-gate-5min"]
            self.assertEqual(len(added), 1, after)
            self.assertEqual(added[0]["action"], "command")
            self.assertIn("javis_report_gate.py", added[0]["command"])
            self.assertIn("CYS_REPORT_GATE_DIR", added[0]["command"])   # 상태격리 배선 포함
            # 구 push 보고 잡은 부활하지 않는다(제거 대상).
            self.assertFalse(any(j["id"] == "owner-progress-report-5min" for j in after["jobs"]))

    def test_legacy_push_job_still_passes(self):
        with tempfile.TemporaryDirectory() as t:
            res, _ = self._c16(t, [dict(self.PUSH_JOB)])
            self.assertEqual(res["status"], "PASS", res)   # 하위호환

    # ── T14: 미배선 게이트 잡 → FAIL(report) / --fix prepend 배선 / 멱등(2회차 PASS) ──
    def test_t14_unwired_gate_migrates_prepend_idempotent(self):
        with tempfile.TemporaryDirectory() as t:
            res, _ = self._c16(t, [dict(self.UNWIRED_GATE_JOB)])
            self.assertEqual(res["status"], "FAIL", res)              # 미배선 → 배선 요구
            res2, after = self._c16(t, [dict(self.UNWIRED_GATE_JOB)], fix=True)
            self.assertEqual(res2["status"], "FIXED", res2)
            cmd = after["jobs"][0]["command"]
            self.assertTrue(cmd.startswith('CYS_REPORT_GATE_DIR='))    # 프리픽스 삽입
            self.assertIn("javis_report_gate.py", cmd)                # 기존 토큰 보존
            self.assertTrue(cmd.rstrip().endswith("run"))
            res3, after2 = self._c16(t, after["jobs"], fix=True)      # 멱등
            self.assertEqual(res3["status"], "PASS", res3)
            self.assertEqual(after2["jobs"][0]["command"].count("CYS_REPORT_GATE_DIR"), 1)

    # ── T14+: --shadow·후행 토큰 보존(재생성 금지 — 삽입만) ──
    def test_t14plus_shadow_and_trailing_tokens_preserved(self):
        with tempfile.TemporaryDirectory() as t:
            job = {"id": "owner-progress-gate-5min", "every_minutes": 5, "action": "command",
                   "command": 'python3 "${CYS_PACK_DIR:-$HOME/.cys/pack}/bin/javis_report_gate.py" run --shadow',
                   "if_absent": "skip"}
            res, after = self._c16(t, [job], fix=True)
            self.assertEqual(res["status"], "FIXED", res)
            cmd = after["jobs"][0]["command"]
            self.assertTrue(cmd.startswith('CYS_REPORT_GATE_DIR='))
            self.assertTrue(cmd.rstrip().endswith("run --shadow"))    # 후행 토큰 보존

    # ── FIX-2: 본사값 bake된 dept command → dept 파생값으로 값 교정 재작성 + 멱등 ──
    _HQ_CMD = ('CYS_REPORT_GATE_DIR="$HOME/.cys/state/report_gate" python3 '
               '"${CYS_PACK_DIR:-$HOME/.cys/pack}/bin/javis_report_gate.py" run')

    def test_fix2_hq_value_in_dept_pack_corrected_to_dept_derived(self):
        with tempfile.TemporaryDirectory() as base:
            deptpack = os.path.join(base, "pack-dept-dept-1")         # dept 팩(파생값=report_gate-dept-1)
            os.makedirs(deptpack)
            job = {"id": "owner-progress-gate-5min", "every_minutes": 5, "action": "command",
                   "command": self._HQ_CMD, "if_absent": "skip"}       # 본사값 bake(잘못 든 상황)
            # report 모드: 값 부정합 → FAIL
            res0, _ = self._c16(deptpack, [dict(job)])
            self.assertEqual(res0["status"], "FAIL", res0)
            self.assertIn("값 부정합", res0["detail"])
            # --fix: dept 파생값으로 값 교정
            res, after = self._c16(deptpack, [dict(job)], fix=True)
            self.assertEqual(res["status"], "FIXED", res)
            cmd = after["jobs"][0]["command"]
            self.assertTrue(cmd.startswith('CYS_REPORT_GATE_DIR="$HOME/.cys/state/report_gate-dept-1"'), cmd)
            self.assertEqual(cmd.count("CYS_REPORT_GATE_DIR"), 1)      # 이중 프리픽스 없음
            self.assertIn("javis_report_gate.py", cmd)                # 후행 토큰 보존
            # 멱등: 2회차 → PASS
            res2, _ = self._c16(deptpack, after["jobs"], fix=True)
            self.assertEqual(res2["status"], "PASS", res2)

    # ── FIX-5: 비선두 오염 command → 전역 스트립·기대값 1개 선두 재삽입·타 env 보존·멱등 ──
    def test_fix5_non_leading_pollution_globally_stripped(self):
        with tempfile.TemporaryDirectory() as base:
            deptpack = os.path.join(base, "pack-dept-dept-1")
            os.makedirs(deptpack)
            # CYS_REPORT_GATE_DIR가 선두가 아니고(JAVIS_ROOT 뒤) 본사값으로 오염된 command
            polluted = ('JAVIS_ROOT="/x" CYS_REPORT_GATE_DIR="$HOME/.cys/state/report_gate" python3 '
                        '"${CYS_PACK_DIR:-$HOME/.cys/pack}/bin/javis_report_gate.py" run --shadow')
            job = {"id": "owner-progress-gate-5min", "every_minutes": 5, "action": "command",
                   "command": polluted, "if_absent": "skip"}
            res, after = self._c16(deptpack, [dict(job)], fix=True)
            self.assertEqual(res["status"], "FIXED", res)
            cmd = after["jobs"][0]["command"]
            self.assertEqual(cmd.count("CYS_REPORT_GATE_DIR"), 1)                 # 정확히 1개
            self.assertTrue(cmd.startswith('CYS_REPORT_GATE_DIR="$HOME/.cys/state/report_gate-dept-1"'), cmd)  # 선두·dept값
            self.assertIn('JAVIS_ROOT="/x"', cmd)                                 # 타 env 프리픽스 보존
            self.assertTrue(cmd.rstrip().endswith("run --shadow"))               # 후행 토큰 보존
            res2, _ = self._c16(deptpack, after["jobs"], fix=True)               # 멱등
            self.assertEqual(res2["status"], "PASS", res2)

    # ── FIX-5: 선두 정합 + 후미 오염(이중 env) → startswith false-PASS 봉쇄 ──
    def test_fix5_leading_correct_plus_trailing_pollution_not_false_pass(self):
        with tempfile.TemporaryDirectory() as base:
            deptpack = os.path.join(base, "pack-dept-dept-1")
            os.makedirs(deptpack)
            # 선두는 올바른 dept값이나 뒤에 본사값이 또 붙은 이중 env(셸 last-wins 오값 위험)
            dbl = ('CYS_REPORT_GATE_DIR="$HOME/.cys/state/report_gate-dept-1" '
                   'CYS_REPORT_GATE_DIR="$HOME/.cys/state/report_gate" python3 '
                   '"${CYS_PACK_DIR:-$HOME/.cys/pack}/bin/javis_report_gate.py" run')
            job = {"id": "owner-progress-gate-5min", "every_minutes": 5, "action": "command",
                   "command": dbl, "if_absent": "skip"}
            # report 모드: 선두 정합이어도 occ==2 → 정합 아님 → FAIL(false-PASS 봉쇄)
            res0, _ = self._c16(deptpack, [dict(job)])
            self.assertEqual(res0["status"], "FAIL", res0)
            res, after = self._c16(deptpack, [dict(job)], fix=True)
            self.assertEqual(res["status"], "FIXED", res)
            cmd = after["jobs"][0]["command"]
            self.assertEqual(cmd.count("CYS_REPORT_GATE_DIR"), 1)                 # 이중 제거
            self.assertTrue(cmd.startswith('CYS_REPORT_GATE_DIR="$HOME/.cys/state/report_gate-dept-1"'), cmd)


class C71GateGuardBehavior(unittest.TestCase):
    """c71 게이트 가드 행동 회귀 — 가드 포함 게이트에 대해 PASS(DESIGN §3.7)."""

    import importlib
    P = importlib.import_module("javis_preflight")

    @unittest.skipUnless(os.path.isdir(os.path.expanduser("~/.cys/pack")),
                         "본사 팩(~/.cys/pack) 부재 — hq realpath 대조 불가")
    def test_c71_passes_on_guarded_worktree_gate(self):
        saved = dict(os.environ)
        os.environ["CYS_PACK_DIR"] = os.path.dirname(BIN)      # worktree 팩(가드 포함 게이트)
        os.environ.pop("CYS_SOCKET", None)
        os.environ.pop("CYS_REPORT_GATE_DIR", None)
        try:
            pf = self.P.Preflight(fix=False, skips=set(), mode="report")
            pf.c71_gate_guard_behavior()
            res = pf.results[-1]
        finally:
            os.environ.clear()
            os.environ.update(saved)
        self.assertEqual(res["status"], "PASS", res)           # F=SKIP·L=not-SKIP

    def _run_c71_with_pack(self, pack_dir):
        saved = dict(os.environ)
        os.environ["CYS_PACK_DIR"] = pack_dir
        os.environ.pop("CYS_SOCKET", None)
        os.environ.pop("CYS_REPORT_GATE_DIR", None)
        try:
            pf = self.P.Preflight(fix=False, skips=set(), mode="report")
            pf.c71_gate_guard_behavior()
            return pf.results[-1]
        finally:
            os.environ.clear()
            os.environ.update(saved)

    # ── FIX-1: dept 팩 컨텍스트 → L-케이스가 정합 소켓 토큰으로 not-SKIP → PASS(위양 FAIL 봉쇄) ──
    @unittest.skipUnless(os.path.isdir(os.path.expanduser("~/.cys/pack")),
                         "본사 팩(~/.cys/pack) 부재 — hq realpath 대조 불가")
    def test_c71_passes_on_dept_pack_context(self):
        import shutil
        with tempfile.TemporaryDirectory() as td:
            deptpack = os.path.join(td, "pack-dept-dept-1")            # dept 팩 basename 모사
            os.makedirs(os.path.join(deptpack, "bin"))
            shutil.copyfile(os.path.join(BIN, "javis_report_gate.py"),  # 가드 포함 게이트 사본
                            os.path.join(deptpack, "bin", "javis_report_gate.py"))
            res = self._run_c71_with_pack(deptpack)
            self.assertEqual(res["status"], "PASS", res)               # F=SKIP·L(dept 정합)=not-SKIP

    # ── FIX-1: 가드 제거본 → 케이스 F가 SKIP 안 냄 → FAIL(회귀 검출 유지) ──
    @unittest.skipUnless(os.path.isdir(os.path.expanduser("~/.cys/pack")),
                         "본사 팩(~/.cys/pack) 부재 — hq realpath 대조 불가")
    def test_c71_fails_when_guard_removed(self):
        with tempfile.TemporaryDirectory() as td:
            pk = os.path.join(td, "pack-noguard")
            os.makedirs(os.path.join(pk, "bin"))
            src = open(os.path.join(BIN, "javis_report_gate.py"), encoding="utf-8").read()
            stripped = src.replace("foreign = foreign_daemon_verdict()", "foreign = None", 1)
            self.assertIn("foreign = None", stripped)                  # 가드 무력화 확인
            with open(os.path.join(pk, "bin", "javis_report_gate.py"), "w", encoding="utf-8") as f:
                f.write(stripped)
            res = self._run_c71_with_pack(pk)
            self.assertEqual(res["status"], "FAIL", res)              # 회귀 검출
            self.assertIn("가드 회귀", res["detail"])


def _write_standby(state_dir, roles, declared_epoch=1_000_000, ttl_secs=21_600):
    """owner_standby.json 마커 기록(정규화 매칭·억제 검증용)."""
    nodes = {r: {"declared_epoch": declared_epoch, "ttl_secs": ttl_secs} for r in roles}
    with open(os.path.join(state_dir, "owner_standby.json"), "w", encoding="utf-8") as f:
        json.dump({"nodes": nodes}, f)


class OwnerStandby(unittest.TestCase):
    """owner-directed-standby 억제 — idle/stall만 억제·정규화 매칭(하이픈↔언더스코어)·
    ★death는 standby 중에도 경보(억제 금지·master 기결). 마커 없으면 회귀 0(기존 65 테스트 green)."""

    def _tasks(self, r):
        return [task for _to, task, _r, _i in r.enqueues]

    # ── 정규화 매칭: 언더스코어 마커가 하이픈 role의 idle을 억제(SF 결함 봉인) ──
    def test_underscore_marker_suppresses_hyphen_role_idle(self):
        with tempfile.TemporaryDirectory() as t:
            _write_standby(t, ["reviewer_codex"])                  # 마커=언더스코어
            rep = report(nodes=[{"node": "reviewer-codex", "done": 1, "total": 5, "pct": 20}],
                         idle_nodes=[{"role": "reviewer-codex", "idle_secs": 600}])  # role=하이픈
            r = FakeRunner(rep=rep)
            gate(t, r).run()                                       # baseline
            gate(t, r).run()                                       # idle → 억제
            self.assertNotIn("gate-idle-reviewer-codex", self._tasks(r))
            e = ledger_entries(t)[-1]
            self.assertTrue(any("suppressed(owner-standby)" in x and "idle:reviewer-codex" in x
                                for x in e["reasons"]), e["reasons"])

    # ── 정규화 매칭 역방향: 하이픈 마커가 언더스코어 role의 idle을 억제(무구분) ──
    def test_hyphen_marker_suppresses_underscore_role_idle(self):
        with tempfile.TemporaryDirectory() as t:
            _write_standby(t, ["reviewer-codex"])                  # 마커=하이픈
            rep = report(nodes=[{"node": "reviewer_codex", "done": 1, "total": 5, "pct": 20}],
                         idle_nodes=[{"role": "reviewer_codex", "idle_secs": 600}])  # role=언더스코어
            r = FakeRunner(rep=rep)
            gate(t, r).run(); gate(t, r).run()
            self.assertNotIn("gate-idle-reviewer_codex", self._tasks(r))

    # ── 5노드 전원 억제: 마커(혼합 구분자)로 하이픈 리뷰어 포함 5노드 idle 전원 억제('worker만 억제' 봉인) ──
    def test_five_nodes_all_suppressed(self):
        with tempfile.TemporaryDirectory() as t:
            roles = ["cso", "worker", "reviewer-gemini", "reviewer-codex", "master"]
            _write_standby(t, ["cso", "worker", "reviewer_gemini", "reviewer_codex", "master"])
            rep = report(nodes=[{"node": h, "done": 1, "total": 5, "pct": 20} for h in roles],
                         idle_nodes=[{"role": h, "idle_secs": 600} for h in roles])
            r = FakeRunner(rep=rep)
            gate(t, r).run(); gate(t, r).run()
            for h in roles:
                self.assertNotIn("gate-idle-%s" % h, self._tasks(r),
                                 "%s idle 억제 실패(정규화 결함 재현)" % h)

    # ── 무배정 idle_edge 억제: standby role의 무배정 idle 엣지 미발화 ──
    def test_suppresses_unassigned_idle_edge(self):
        with tempfile.TemporaryDirectory() as t:
            _write_standby(t, ["worker"])
            active = report(live_nodes=[{"role": "worker", "agent_alive": True, "idle_secs": 10}])
            idle = report(live_nodes=[{"role": "worker", "agent_alive": True, "idle_secs": 600}],
                          idle_nodes=[{"role": "worker", "idle_secs": 600}])
            gate(t, FakeRunner(rep=active)).run()                  # baseline active
            r = FakeRunner(rep=idle)
            gate(t, r).run()                                       # 무배정 idle → 엣지 억제
            self.assertNotIn("gate-idle-worker", self._tasks(r))
            e = ledger_entries(t)[-1]
            self.assertTrue(any("idle:worker" in x for x in e["reasons"]), e["reasons"])

    # ── ★death는 standby 중에도 WARN 발화(억제 금지·master 기결·별도 결함 미유입) ──
    def test_death_fires_during_standby(self):
        with tempfile.TemporaryDirectory() as t:
            _write_standby(t, ["worker"])
            alive = report(live_nodes=[{"role": "worker", "agent_alive": True, "idle_secs": 10}])
            dead = report(live_nodes=[{"role": "worker", "agent_alive": False}])
            gate(t, FakeRunner(rep=alive)).run()                   # baseline alive
            gate(t, FakeRunner(rep=dead)).run()                    # 시딩
            r = FakeRunner(rep=dead)
            gate(t, r).run()                                       # 확정 → death WARN(standby 무관)
            e = ledger_entries(t)[-1]
            self.assertEqual(e["verdict"], "WARN")
            self.assertIn("gate-death-worker",
                          [task for _to, task, _r, _i in r.enqueues if _to == "master"])

    # ── 만료 마커는 억제 안 함(TTL 경과 → idle WARN 정상 발화) ──
    def test_expired_marker_does_not_suppress(self):
        with tempfile.TemporaryDirectory() as t:
            _write_standby(t, ["worker"], declared_epoch=1_000_000 - 30_000, ttl_secs=21_600)  # 만료
            rep = report(nodes=[{"node": "worker", "done": 1, "total": 5, "pct": 20}],
                         idle_nodes=[{"role": "worker", "idle_secs": 600}])
            r = FakeRunner(rep=rep)
            gate(t, r).run(); gate(t, r).run()
            self.assertIn("gate-idle-worker", self._tasks(r))      # 만료 → 억제 안 됨

    # ── ①codex(missing): 미래 epoch/비유한(Inf/NaN)/손상 declared_epoch·ttl 마커는 억제 안 함(fail-open) ──
    def test_epoch_validation_fails_open(self):
        with tempfile.TemporaryDirectory() as t:
            # 미래(now+100k·유예 300s 초과) → 만료 판정 무력화 위험 → fail-open 제외
            _write_standby(t, ["worker"], declared_epoch=1_000_000 + 100_000, ttl_secs=21_600)
            self.assertEqual(G.load_owner_standby(t, 1_000_000), frozenset(), "미래 epoch fail-open")
            # 비유한 declared_epoch(Inf/NaN) → fail-open
            _write_standby(t, ["worker"], declared_epoch=float("inf"), ttl_secs=21_600)
            self.assertEqual(G.load_owner_standby(t, 1_000_000), frozenset(), "Inf epoch fail-open")
            _write_standby(t, ["worker"], declared_epoch=float("nan"), ttl_secs=21_600)
            self.assertEqual(G.load_owner_standby(t, 1_000_000), frozenset(), "NaN epoch fail-open")
            # 비유한 ttl도 만료 판정을 무력화 → fail-open
            _write_standby(t, ["worker"], declared_epoch=1_000_000, ttl_secs=float("inf"))
            self.assertEqual(G.load_owner_standby(t, 1_000_000), frozenset(), "Inf ttl fail-open")
            # 대조군1: 유예 내 미세 미래(now+100s)는 정상 유효(시계 스큐 흡수)
            _write_standby(t, ["worker"], declared_epoch=1_000_000 + 100, ttl_secs=21_600)
            self.assertIn("worker", G.load_owner_standby(t, 1_000_000), "유예 내 미세 미래는 유효")
            # 대조군2: 정상 유한 마커는 유효(fail-open이 정상 억제까지 죽이지 않음)
            _write_standby(t, ["worker"], declared_epoch=1_000_000, ttl_secs=21_600)
            self.assertIn("worker", G.load_owner_standby(t, 1_000_000))

    # ── ①codex(잔여 P1): 거대 정수 epoch/ttl은 항목만 fail-open(예외가 게이트 밖 탈출 금지) ──
    def test_huge_int_marker_fails_open_isolated(self):
        # math.isfinite(10**400) → OverflowError. 예외가 load_owner_standby를 벗어나면 게이트
        # 전체가 무너져 경보 전멸 — 손상 항목만 무효화하고 정상 항목은 살아야 한다.
        huge = 10 ** 400
        with tempfile.TemporaryDirectory() as t:
            # 거대 epoch 손상 마커 + 정상 마커 공존
            nodes = {"worker": {"declared_epoch": huge, "ttl_secs": 21_600},
                     "cso": {"declared_epoch": 1_000_000, "ttl_secs": 21_600}}
            with open(os.path.join(t, "owner_standby.json"), "w", encoding="utf-8") as f:
                json.dump({"nodes": nodes}, f)
            got = G.load_owner_standby(t, 1_000_000)   # 예외 던지면 여기서 에러(=결함)
            self.assertNotIn("worker", got, "거대 epoch 항목은 fail-open 제외")
            self.assertIn("cso", got, "예외가 게이트를 무너뜨리지 않고 정상 항목은 유효")
            # 거대 ttl도 동일(isfinite(huge ttl) OverflowError)
            nodes2 = {"worker": {"declared_epoch": 1_000_000, "ttl_secs": huge},
                      "cso": {"declared_epoch": 1_000_000, "ttl_secs": 21_600}}
            with open(os.path.join(t, "owner_standby.json"), "w", encoding="utf-8") as f:
                json.dump({"nodes": nodes2}, f)
            got2 = G.load_owner_standby(t, 1_000_000)
            self.assertNotIn("worker", got2, "거대 ttl 항목은 fail-open 제외")
            self.assertIn("cso", got2, "정상 항목 유효")

    # ── ②codex(P2): 정규화 충돌(서로 다른 raw가 같은 canonical)은 그 키 fail-open 제외 ──
    def test_canonical_collision_fails_open(self):
        with tempfile.TemporaryDirectory() as t:
            # reviewer-codex 와 reviewer_codex 는 서로 다른 raw지만 canonical=reviewer_codex 로 병합
            _write_standby(t, ["reviewer-codex", "reviewer_codex"])
            self.assertNotIn("reviewer_codex", G.load_owner_standby(t, 1_000_000),
                             "충돌 canonical 키는 fail-open 제외(어느 노드 억제인지 모호)")
            # 단일 마커의 하이픈↔언더스코어 무구분 매칭은 충돌 아님 → 유지(본래 목적 보존)
            _write_standby(t, ["reviewer-codex"])
            self.assertIn("reviewer_codex", G.load_owner_standby(t, 1_000_000),
                          "단일 마커 정규화 매칭은 유지(충돌과 구분)")

    # ── ②codex(missing): standby 중 stall 카운터 누적·승격만 억제 → 만료 시 승격+카운터 유지 ──
    def test_stall_accumulates_during_standby_then_promotes(self):
        rep = report(nodes=[{"node": "worker", "done": 1, "total": 5, "pct": 20}])
        counters = {}
        # standby 중 3주기: 카운터는 증가하되 승격만 억제(stall_cycles=2·node live 미기재→idle None→승격 가능)
        for _ in range(3):
            sup = []
            stalls = G.build_stall_warnings(counters, rep, 5, 2, "t", frozenset({"worker"}), sup)
            self.assertEqual(stalls, [], "standby 중 stall 승격 억제")
        self.assertGreaterEqual(counters["nodes"]["worker"]["count"], 2,
                                "카운터는 standby 중에도 누적(억제 대상은 승격뿐)")
        acc = counters["nodes"]["worker"]["count"]
        # 만료(standby 해제) → 누적 카운터로 즉시 승격
        stalls = G.build_stall_warnings(counters, rep, 5, 2, "t", frozenset(), None)
        self.assertTrue(any(s["task"] == "gate-stall-worker" for s in stalls),
                        "만료 시 누적 카운터로 stall 승격")
        self.assertGreater(counters["nodes"]["worker"]["count"], acc,
                           "승격 후에도 카운터는 계속(리셋 아님)")

    # ── ②codex(missing): idle_edge armed 유지(standby 중 disarm 없음) → 만료 후 정상 쿨다운 재발화 ──
    def test_idle_edge_armed_retained_refires_after_standby(self):
        rep = report(idle_nodes=[{"role": "worker", "idle_secs": 600}])  # 무배정 idle → 엣지 경로
        counters = {"idle_edge": {}}
        # standby: 엣지 진입 전 억제(continue) → armed 비트 미변경(extract_warnings 순수)
        sup = []
        w = G.extract_warnings(rep, counters, 1_000_000, 900, frozenset({"worker"}), sup)
        self.assertFalse(any(x["reason"].startswith("idle_edge:") for x in w),
                         "standby 중 idle 엣지 억제")
        self.assertTrue(any("idle:worker" in s for s in sup), "억제 감사 기록")
        self.assertEqual(counters["idle_edge"], {}, "armed 비트 불변(disarm 없음)")
        # 만료(standby 해제) → armed True(기본)+쿨다운 통과 → 엣지 재발화
        w2 = G.extract_warnings(rep, counters, 1_000_000, 900, frozenset(), None)
        self.assertTrue(any(x["reason"] == "idle_edge:worker" for x in w2),
                        "만료 후 정상 쿨다운으로 엣지 재발화")


if __name__ == "__main__":
    unittest.main()
