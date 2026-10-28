#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_report_gate.py — javis_report_gate.py + javis_gate_check.py 회귀.

DESIGN §C1 필수 케이스 10종을 Gate 코어에 대역 Runner를 주입해 핀한다(서버·데몬 기동 0).
외부 명령(javis_report/event/wakeup·cys)은 전부 FakeRunner로 대체 — 호출 여부·인자를 기록해
"배달 체인 완결(enqueue+drain)"·"emit 거부 폴백" 등 부작용을 검증한다.

★W5(T-0147-2 wakeup 홍수 해소) 계약 전환 — 아래 단언들은 **의도적으로** 갱신됐다:
  · idle·feed·collect·내부오류는 **더 이상 master push 를 내지 않는다**(층2 채널 정책표).
    정보는 ledger·badge(·EVT)로 남고 stdin 주입만 사라진다 — 채널 이동이지 침묵이 아니다.
  · fail-open 직송 2경로(I2)는 제거됐다. state 기록 불능은 stdout `gate_signal=state_unwritable`
    토큰(=state **외부** oracle)으로만 나간다.
  · push 는 stall 확증(§2-C)·시스템 데드락(P3)·노드 사망(deadman 소비)에서만 승격된다.
검체 전량(§1-B N1~C3)은 `tests/run_bootstrap_health.py` W5 그룹이 소유한다. 여기는 코어 회귀다.

실행: python3 test_report_gate.py   (unittest·표준 러너 — CI가 파일 직접 실행하는 관례 준거)
"""
import calendar
import json
import os
import subprocess
import sys
import tempfile
import textwrap
import time
import unittest

BIN =os.path.dirname(os.path.dirname(os.path.abspath(__file__)))          # cysjavis-pack/bin
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
                 drain_delivered=1, collect_raises=False, events=None,
                 ack_ok=True, tasks=None, enqueue_rc=0):
        self.report_ok, self.rep, self.err = report_ok, rep, err
        self.emit_rc, self.drain_delivered = emit_rc, drain_delivered
        self.collect_raises = collect_raises
        self.events, self.ack_ok, self.tasks = events or [], ack_ok, tasks
        self.enqueue_rc = enqueue_rc
        self.emits, self.enqueues, self.drains, self.sends = [], [], [], []
        self.polls = []
        self._wid = 0

    def collect_report(self):
        if self.collect_raises:
            raise RuntimeError("주입된 내부 오류")
        # Modern collector fixture: each successful collection has identity and a fresh sample.
        if self.rep is not None and self.report_ok:
            self._sample = getattr(self, "_sample", 0) + 300
            self.rep["sampled_at"] = self._sample
            for i, row in enumerate(self.rep.get("live_nodes") or []):
                row.setdefault("surface_id", i + 1)
                row.setdefault("created_at", 1)
        return self.report_ok, self.rep, self.err

    def emit(self, evt_type, fields, surface="auto"):
        self.emits.append((evt_type, fields))
        return self.emit_rc, "", ""

    def enqueue(self, to, task, reason, idem, payload=None, severity=None):
        self.enqueues.append((to, task, reason, idem, severity))
        self._wid += 1
        return self.enqueue_rc, ("W-%010x" % self._wid)

    def drain(self, target):
        self.drains.append(target)
        return 0, self.drain_delivered

    # ── W5: 데몬 이벤트 1회 폴링(queue.delivered 영수증 · master.deadman 사망 확증) ──
    def poll_events(self, after_seq, names, timeout=0):
        self.polls.append((after_seq, tuple(names)))
        if not self.ack_ok:
            return False, [], after_seq
        evs = [e for e in self.events if e.get("name") in names]
        return True, evs, after_seq + len(evs)

    # ── W5: P3 데드락 술어의 티켓 원장 증거 ──
    def task_snapshot(self):
        if self.tasks is None:
            return False, None, "no tasks"
        return True, self.tasks, None

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


def gate(state_dir, runner, clock=None, stall_cycles=2, quiet_cycles=3):
    clk = clock or Clock(1_000_000.0)
    return G.Gate(state_dir, runner, cycle_minutes=5, stall_cycles=stall_cycles,
                  quiet_cycles=quiet_cycles,
                  now_epoch_fn=clk.now_epoch, now_iso_fn=clk.now_iso)


def ledger_entries(state_dir):
    path = os.path.join(state_dir, "ledger.jsonl")
    with open(path, encoding="utf-8") as f:
        return [json.loads(l) for l in f if l.strip()]


def badges(state_dir):
    """A8 배지 파일(데몬 alerts.rs `node_liveness` 소비 계약) — {key: badge}."""
    with open(os.path.join(state_dir, "badges.json"), encoding="utf-8") as f:
        return {b["key"]: b for b in json.load(f)["badges"]}


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

    # ── ③ 경고 주입 → WARN + **push 강등**(W5 층2: idle 은 ledger+evt+badge) ──
    def test_idle_warning_records_but_never_pushes(self):
        with tempfile.TemporaryDirectory() as t:
            rep = report(nodes=[{"node": "worker", "done": 1, "total": 5, "pct": 20}],
                         idle_nodes=[{"role": "worker", "idle_secs": 600}])
            r = FakeRunner(rep=rep)
            gate(t, r).run()                                  # baseline (no delivery)
            gate(t, r).run()
            e = ledger_entries(t)[-1]
            self.assertEqual(e["verdict"], "WARN")
            self.assertEqual(r.enqueues, [], "idle 이 master stdin 을 다시 잠식한다(W5 층2 위반)")
            self.assertEqual(r.drains, [])
            self.assertEqual(e["delivered"], "none")
            self.assertTrue(any("idle_5min:worker" in x for x in e["reasons"]))
            self.assertIn("agent.silent", [t for t, _ in r.emits])   # EVT 채널은 유지
            self.assertIn("gate-idle-worker", badges(t))             # badge 채널도 유지

    def test_multi_idle_nodes_separate_per_node_keys(self):
        # master 승인 2026-07-18: idle 노드별로 task/idem 분리(키 분리는 유지 — 채널만 강등).
        # ★BASELINE 은 현재 idle 인 role 을 **disarmed** 로 시드한다(idle-standby-v5 D7 — 업그레이드
        #   순간의 재발화 파도 방지). 그래서 엣지는 baseline **이후의 idle 진입**에서만 뜬다.
        busy = report(nodes=[{"node": "worker", "done": 1, "total": 5, "pct": 20}],
                      live_nodes=[{"role": "reviewer-codex", "agent_alive": True, "idle_secs": 5},
                                  {"role": "reviewer-gemini", "agent_alive": True, "idle_secs": 5}])
        idle = report(nodes=[{"node": "worker", "done": 1, "total": 5, "pct": 20}],
                      live_nodes=[{"role": "reviewer-codex", "agent_alive": True, "idle_secs": 600},
                                  {"role": "reviewer-gemini", "agent_alive": True, "idle_secs": 700}],
                      idle_nodes=[{"role": "reviewer-codex", "idle_secs": 600},
                                  {"role": "reviewer-gemini", "idle_secs": 700}])
        with tempfile.TemporaryDirectory() as t:
            gate(t, FakeRunner(rep=busy)).run()               # baseline (idle 아님)
            r = FakeRunner(rep=idle)
            gate(t, r).run()                                  # idle 진입 → 엣지 2건
            keys = badges(t)
            self.assertIn("gate-idle-reviewer-codex", keys)
            self.assertIn("gate-idle-reviewer-gemini", keys)
            self.assertEqual(r.enqueues, [])                  # push 0
            # 라벨 `worker` 는 live role 로 조인되지 않는다 → 스키마 결함으로 **노출**(은닉 금지)
            self.assertIn("gate-label-worker", keys)

    def test_idle_edge_fires_once_then_suppressed(self):
        """A3′ — 무배정 idle 은 엣지 1회. 종전 레벨 트리거가 매 주기 재발화하던 갈래."""
        busy = report(live_nodes=[{"role": "reviewer-codex", "agent_alive": True, "idle_secs": 5}])
        idle = report(live_nodes=[{"role": "reviewer-codex", "agent_alive": True, "idle_secs": 600}],
                      idle_nodes=[{"role": "reviewer-codex", "idle_secs": 600}])
        with tempfile.TemporaryDirectory() as t:
            gate(t, FakeRunner(rep=busy)).run()               # baseline (idle 아님)
            r = FakeRunner(rep=idle)
            gate(t, r).run()                                  # 엣지 1회
            first = ledger_entries(t)[-1]
            gate(t, r).run()                                  # 같은 조건 지속 → 재발화 금지
            second = ledger_entries(t)[-1]
            self.assertTrue(any("idle_edge:reviewer-codex" in x for x in first["reasons"]),
                            first["reasons"])
            self.assertFalse(any("idle_edge:reviewer-codex" in x for x in second["reasons"]),
                             "엣지가 레벨로 퇴화했다(매 주기 재발화)")

    def test_baseline_seeds_current_idle_as_disarmed(self):
        """D7 — 재설치·업그레이드 직후 전 노드가 동시 엣지 발화하는 '파도'를 막는다."""
        idle = report(live_nodes=[{"role": "reviewer-codex", "agent_alive": True, "idle_secs": 600}],
                      idle_nodes=[{"role": "reviewer-codex", "idle_secs": 600}])
        with tempfile.TemporaryDirectory() as t:
            r = FakeRunner(rep=idle)
            gate(t, r).run()                                  # baseline: idle 상태로 시작
            gate(t, r).run()
            e = ledger_entries(t)[-1]
            self.assertFalse(any("idle_edge" in x for x in e["reasons"]),
                             "BASELINE 직후 엣지 파도 발생: %r" % e["reasons"])

    def test_feed_pending_is_ledger_and_badge_only(self):
        # 설계 층2 표: gate-feed = ledger+badge(EVT·push 없음 — GUI Feed 탭이 이미 소비자다).
        with tempfile.TemporaryDirectory() as t:
            rep = report(feed=2)
            r = FakeRunner(rep=rep)
            gate(t, r).run()
            gate(t, r).run()
            e = ledger_entries(t)[-1]
            self.assertEqual(e["verdict"], "WARN")
            self.assertEqual(r.enqueues, [])
            self.assertIn("gate-feed", badges(t))

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

    # ── ⑥ fail-open(내부 예외 주입) — ★W5 I2: 직송 제거·대장+배지로 수렴 ──
    def test_fail_open_records_without_direct_send(self):
        with tempfile.TemporaryDirectory() as t:
            r = FakeRunner(collect_raises=True)
            rc = gate(t, r).run()
            self.assertEqual(rc, 0)                            # 죽지 않는다
            self.assertEqual(r.sends, [], "fail-open 직송(I2)이 살아있다 — 라우터 우회 발행자")
            self.assertEqual(r.enqueues, [])
            e = ledger_entries(t)[-1]
            self.assertEqual(e["verdict"], "FAILOPEN")
            self.assertIn("gate-internal-error", badges(t))     # 침묵 금지: 배지로 노출

    def test_fail_open_streak_note_after_three(self):
        with tempfile.TemporaryDirectory() as t:
            r = FakeRunner(collect_raises=True)
            for _ in range(3):
                gate(t, r).run()
            self.assertIn("게이트 자체 수리 필요",
                          badges(t)["gate-internal-error"]["message"])

    # ── N6b: state 기록 불능 → **state 외부 oracle**(stdout gate_signal 토큰)만 나간다 ──
    @unittest.skipIf(os.geteuid() == 0, "root는 파일권한 무시 — chmod 555 재현 불가")
    def test_state_unwritable_emits_gate_signal_not_direct_send(self):
        import contextlib
        import io
        with tempfile.TemporaryDirectory() as t:
            sd = os.path.join(t, "state")
            os.makedirs(sd)
            r = FakeRunner(rep=report(nodes=[{"node": "worker", "done": 1, "total": 3, "pct": 33}]))
            os.chmod(sd, 0o555)                               # 읽기·실행만 — 락 mkdir 불가
            out = io.StringIO()
            try:
                with contextlib.redirect_stdout(out):
                    rc = gate(sd, r).run()
            finally:
                os.chmod(sd, 0o755)                           # 정리 위해 복구
            self.assertEqual(rc, 0)                            # 죽지 않는다(exit 1 금지)
            self.assertEqual(r.sends, [], "state 불능 직송(I2)이 살아있다")
            self.assertIn("gate_signal=state_unwritable", out.getvalue())

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
            self.assertEqual(counters.get("schema_version"), 1)
            self.assertEqual(snap.get("schema_version"), 1)
            self.assertIn("data", snap)                       # 스냅샷 본문은 래퍼 안(diff 오탐 방지)
            self.assertEqual(ledger_entries(t)[-1]["schema_version"], 1)

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

    RAW_CMD = "python3 \"${CYS_PACK_DIR:-$HOME/.cys/pack}/bin/javis_report_gate.py\" run"
    GATE_JOB = {"id": "owner-progress-gate-5min", "every_minutes": 5, "action": "command",
                "command": "CYS_REPORT_GATE_DIR=\"/tmp/lane\" " + RAW_CMD,
                "if_absent": "skip"}
    UNWIRED_JOB = {"id": "owner-progress-gate-5min", "every_minutes": 5, "action": "command",
                   "command": RAW_CMD + " --shadow", "if_absent": "skip"}
    PUSH_JOB = {"id": "owner-progress-report-5min", "every_minutes": 5, "action": "push",
                "to": "master", "text_command": "python3 x", "if_absent": "skip"}

    def test_gate_job_only_passes_and_fix_is_noop(self):
        with tempfile.TemporaryDirectory() as t:
            res, _ = self._c16(t, [dict(self.GATE_JOB)])
            self.assertEqual(res["status"], "PASS", res)
            # --fix: 게이트 잡 존재 → 재추가 없음(마이그레이션 보존)
            res2, after = self._c16(t, [dict(self.GATE_JOB)], fix=True)
            self.assertEqual(res2["status"], "PASS")
            ids = [j["id"] for j in after["jobs"]]
            self.assertEqual(ids, ["owner-progress-gate-5min"])   # 구 push 잡 재생성 안 됨

    # ── W5 B7: 레인 배선 마이그레이션(토큰 보존 삽입만) ──
    def test_unwired_gate_job_fails_and_fix_inserts_lane_env_preserving_args(self):
        with tempfile.TemporaryDirectory() as t:
            res, _ = self._c16(t, [dict(self.UNWIRED_JOB)])
            self.assertEqual(res["status"], "FAIL", res)          # 다중 데몬 state 공유 위험
            res2, after = self._c16(t, [dict(self.UNWIRED_JOB)], fix=True)
            self.assertEqual(res2["status"], "FIXED", res2)
            cmd = after["jobs"][0]["command"]
            self.assertTrue(cmd.startswith("CYS_REPORT_GATE_DIR="), cmd)
            self.assertIn("--shadow", cmd, "기존 인자가 재생성으로 소실됐다(토큰 보존 위반)")
            self.assertIn("javis_report_gate.py", cmd)
            # 멱등: 재실행은 무동작(PASS)
            res3, after3 = self._c16(t, after["jobs"], fix=True)
            self.assertEqual(res3["status"], "PASS", res3)
            self.assertEqual(after3["jobs"][0]["command"], cmd)

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
            # 구 push 보고 잡은 부활하지 않는다(제거 대상).
            self.assertFalse(any(j["id"] == "owner-progress-report-5min" for j in after["jobs"]))

    def test_legacy_push_job_still_passes(self):
        with tempfile.TemporaryDirectory() as t:
            res, _ = self._c16(t, [dict(self.PUSH_JOB)])
            self.assertEqual(res["status"], "PASS", res)   # 하위호환


class PushTargetRouting(unittest.TestCase):
    """push 수신자 라우팅 회귀 핀 — 2026-08-01 실사고(surface:241) 수리분 잠금.

    사고: `death` 가 표 층위에서 master 직송이라 **죽은 master 에게 자기 부고**가 갔다.
    좌석이 비어 배달이 성립하지 않으니 `queue.delivered` 영수증도 영영 없고, critical 은
    at-least-once 라 seen 이 `inflight` 로 남아 SEEN_TTL_SECS(1800s)마다 재발화한다.
    실측: 11.5h 동안 death push 23회 · queue depth 36 · 재발화 간격 중앙값 정확히 1800s.

    수리는 **두 겹**이고 두 겹 다 여기서 핀한다(한 겹만 되돌려져도 사고가 재현되므로):
      ① 표(PUSH_TARGET) → 세 트리거 전부 cso (규범 "시스템·자원 사안의 1차 수신자는 CSO")
      ② `avoid` 자기참조 차단 — 표가 언제 다시 바뀌어도 사망 당사자에게는 보내지 않는 독립 장치.
         ②는 다시 두 갈래다: (가) want 가족 == avoid 가족  (나) **CSO 부재 폴백**(cso_absent).
         (나)는 적대검증 A4 잔존결함이었다 — want='cso' 인데 죽은 것이 master 면 가족이 달라
         (가)를 통과하고, 폴백이 그대로 master 를 돌려줬다.
    """

    def setUp(self):
        # _push_target 은 상태 무의존 순수 라우팅 — 인스턴스 없이도 되지만 실제 바인딩을 쓴다.
        self.g = G.Gate.__new__(G.Gate)

    # ── ⓐ 표 자체(재정 ①) ──
    def test_push_target_table_routes_all_three_triggers_to_cso(self):
        self.assertEqual(
            G.PUSH_TARGET,
            {"deadlock": "cso", "stall_confirmed": "cso", "death": "cso"},
            "표가 바뀌었다 — master 직송 복귀는 자기부고 사고(surface:241)의 1차 원인이다")

    # ── ⓑ 정상 라우팅(무회귀) ──
    def test_death_routes_to_cso_when_cso_alive(self):
        self.assertEqual(self.g._push_target("death", ["cso", "worker"], None), ("cso", ""))

    def test_death_with_dead_master_still_routes_to_live_cso(self):
        # 죽은 것이 master 여도 CSO 가 살아 있으면 정상 수신자는 CSO 다(억제가 아니다).
        self.assertEqual(self.g._push_target("death", ["cso", "worker"], "master"), ("cso", ""))

    # ── ⓒ 자기참조 억제 — want 가족 == avoid 가족 ──
    def test_death_of_cso_itself_is_suppressed(self):
        self.assertEqual(self.g._push_target("death", ["cso"], "cso"),
                         (None, "self_target_no_alt:cso"))

    # ── ⓓ ★A4 잔존결함 — cso_absent 폴백도 avoid 를 봐야 한다 ──
    def test_cso_absent_fallback_never_targets_the_dead_master(self):
        """죽은 master + CSO 부재 → master 폴백 금지(=None). 이 핀이 A4 재현 케이스다."""
        for live in ([], ["worker"], ["worker", "reviewer-gemini"]):
            got = self.g._push_target("death", live, "master")
            self.assertIsNone(
                got[0],
                "죽은 master 에게 자기 부고를 폴백 배달했다(live=%r) → %r" % (live, got))
            self.assertEqual(got[1], "self_target_no_alt:master")

    def test_cso_absent_fallback_to_master_preserved_when_master_alive(self):
        """무회귀 대칭축: avoid 가 없으면 CSO 부재 폴백은 **그대로 master** 여야 한다.
        (ⓓ를 '항상 억제'로 과잉 수리하면 CSO 부재 레인의 경보가 통째로 사라진다.)"""
        for trig in ("death", "stall_confirmed", "deadlock"):
            self.assertEqual(self.g._push_target(trig, ["worker"], None),
                             ("master", "cso_absent"), trig)

    def test_stall_and_deadlock_routing_unchanged(self):
        for trig in ("stall_confirmed", "deadlock"):
            self.assertEqual(self.g._push_target(trig, ["cso", "worker"], None), ("cso", ""), trig)

    # ── ⓔ 엔드투엔드 — 억제 갈래가 seen 을 되돌리고 사유를 남기는가(자가치유 보존) ──
    def test_suppressed_push_unlinks_seen_and_records_reason(self):
        """None 갈래에서 seen 이 지워져야 레벨 트리거가 좌석 복구 시 자연 재시도한다.
        seen 이 남으면 정확히 사고 당시의 병리(inflight 고착 → TTL 재발화)로 되돌아간다."""
        with tempfile.TemporaryDirectory() as t:
            r = FakeRunner()
            g = gate(t, r)
            w = {"trigger": "death", "severity": G.SEV_CRIT, "idem": "death:master",
                 "task": "gate-death", "cooldown": 0, "avoid_role": "master",
                 "wake_body": "master 좌석 사망 확증", "stamp": {}}
            reasons, counters = [], {}
            target = g._push(w, counters, 1_000_000.0, reasons, ["worker"])   # CSO 부재

            self.assertIsNone(target, "죽은 master 에게 배달됐다")
            self.assertEqual(r.enqueues, [], "억제 갈래인데 큐에 실렸다")
            self.assertIn("push_suppressed_self_target:self_target_no_alt:master", reasons)
            key = G.seen_key("death", "death:master", G.SEV_CRIT)
            self.assertFalse(os.path.exists(G.seen_path(t, key)),
                             "seen 이 남았다 — TTL(1800s)마다 재발화하는 사고 형태로 회귀한다")

    def test_normal_push_still_enqueues_and_keeps_seen(self):
        """ⓔ의 대조군 — 정상 갈래는 큐에 싣고 seen 을 유지한다(억제가 전면화되지 않았음)."""
        with tempfile.TemporaryDirectory() as t:
            r = FakeRunner()
            g = gate(t, r)
            w = {"trigger": "death", "severity": G.SEV_CRIT, "idem": "death:worker",
                 "task": "gate-death", "cooldown": 0, "avoid_role": "worker",
                 "wake_body": "worker 좌석 사망 확증", "stamp": {}}
            reasons, counters = [], {}
            target = g._push(w, counters, 1_000_000.0, reasons, ["cso", "master"])

            self.assertEqual(target, "cso")
            self.assertEqual(len(r.enqueues), 1, r.enqueues)
            self.assertEqual(r.enqueues[0][0], "cso")
            key = G.seen_key("death", "death:worker", G.SEV_CRIT)
            self.assertTrue(os.path.exists(G.seen_path(t, key)))


class QueueExpiryTermination(unittest.TestCase):
    """★(0.14.31 · WP-5 리뷰 R1 · codex major) **만료는 재시도 사슬을 끊는다.**

    데몬이 TTL(기본 6h)로 큐 항목을 뺐다는 사실(`queue.expired` · `entry_ids` 에코)을 종전 게이트는
    구독하지도, 넘겨받아도 해석하지도 않았다. 그래서 만료된 critical wakeup 이 `inflight` 로 남아
    seen TTL 마다 재enqueue 됐고, 그 재enqueue 가 또 만료되며 만료 통지까지 반복 생산했다
    (적체가 스스로를 먹여 살린다). 지금은 **종결**로 처리하되 배달로는 세지 않는다.
    """

    def _pend(self, t, wid, key):
        G.seen_claim(t, key, G.SEV_CRIT, 1_000_000.0)
        G.seen_mark(t, key, 1_000_000.0, state=G.SEEN_STATE_INFLIGHT, wakeup_id=wid)

    def test_expired_event_is_subscribed(self):
        with tempfile.TemporaryDirectory() as t:
            r = FakeRunner()
            g = gate(t, r)
            g._poll_once()
            self.assertTrue(r.polls, "이벤트 폴링을 하지 않았다")
            self.assertIn("queue.expired", r.polls[0][1],
                          "만료 사실을 구독하지 않는다 — 종결 신호가 도착조차 하지 않는다")

    def test_expired_wakeup_is_terminally_disarmed_without_counting_as_delivered(self):
        with tempfile.TemporaryDirectory() as t:
            r = FakeRunner()
            key = G.seen_key("stall_confirmed", "s:worker", G.SEV_CRIT)
            self._pend(t, "W-0000000001", key)
            r.events = [{"name": "queue.expired",
                         "payload": {"entry_ids": ["W-0000000001"],
                                     "queue_entry_id": "q1", "surface_ref": "surface:9"}}]
            g = gate(t, r)
            g._poll_once()
            counters = {}
            g._reconcile_inflight(counters, 1_000_100.0)
            with open(G.seen_path(t, key), encoding="utf-8") as fh:
                rec = json.load(fh)
            self.assertEqual(rec["state"], G.SEEN_STATE_EXPIRED,
                             "만료가 inflight 를 풀지 않았다 — TTL 마다 영구 재enqueue 된다")
            self.assertNotEqual(rec["state"], G.SEEN_STATE_DELIVERED,
                                "만료를 배달로 셌다(배달되지 않은 일을 완료로 기록)")
            # ★성찰 R4 N9 — 이 자리의 종전 단언은 "엣지가 무장 해제됐는가" 였다. 그것이 막으려던
            #   것은 **같은 주기의 재발화**인데, 그 상한은 seen-store 가 이미 준다(state=expired 는
            #   TTL 까지 재선점을 막는다). 엣지까지 내리면 대가가 셋이었다: 재통보 간격이 seen TTL
            #   1800s → 쿨다운 7200s 로 4배 · `cooldown` 기본값 0 인 트리거는 **영구 침묵**(재무장은
            #   '이번 주기에 없는 키' 에만 일어나는데 조건이 지속되면 키는 계속 있다) · 만료가
            #   배지/이벤트로 남지 않아 "critical 이 미배달로 폐기됐다" 를 함대가 관측 못 함.
            #   그래서 핀을 **실제로 지키려는 성질**로 바꾼다.
            self.assertTrue(counters.get("push_edge", {}).get(key, {}).get("armed", True),
                            "만료가 엣지 무장까지 풀었다 — 쿨다운 0 트리거가 영구 침묵한다(N9)")
            claimed, _r = G.seen_claim(t, key, G.SEV_CRIT, 1_000_100.0)
            self.assertFalse(claimed,
                             "만료 직후 같은 주기에 재선점됐다 — seen TTL 상한이 무너졌다")
            claimed, _r = G.seen_claim(t, key, G.SEV_CRIT, 1_000_000.0 + G.SEEN_TTL_SECS + 1)
            self.assertTrue(claimed, "seen TTL 이 지났는데 재통보가 열리지 않는다(영구 침묵)")
            self.assertEqual(counters.get("expired_undelivered"), 1,
                             "미배달 폐기가 계수로 남지 않았다(조용한 유실)")

    def test_delivered_receipt_still_marks_delivered(self):
        """대조군 — 영수증 경로는 종전 그대로 `delivered` 다(만료 처리가 전면화되지 않았다)."""
        with tempfile.TemporaryDirectory() as t:
            r = FakeRunner()
            key = G.seen_key("stall_confirmed", "s:worker2", G.SEV_CRIT)
            self._pend(t, "W-0000000002", key)
            r.events = [{"name": "queue.delivered",
                         "payload": {"entry_ids": ["W-0000000002"]}}]
            g = gate(t, r)
            g._poll_once()
            g._reconcile_inflight({}, 1_000_100.0)
            with open(G.seen_path(t, key), encoding="utf-8") as fh:
                rec = json.load(fh)
            self.assertEqual(rec["state"], G.SEEN_STATE_DELIVERED)

    def test_unrelated_expiry_leaves_other_work_inflight(self):
        """다른 항목의 만료가 내 wakeup 을 종결시키지 않는다(id 대조가 실제로 걸린다)."""
        with tempfile.TemporaryDirectory() as t:
            r = FakeRunner()
            key = G.seen_key("stall_confirmed", "s:worker3", G.SEV_CRIT)
            self._pend(t, "W-0000000003", key)
            r.events = [{"name": "queue.expired", "payload": {"entry_ids": ["W-0000009999"]}}]
            g = gate(t, r)
            g._poll_once()
            g._reconcile_inflight({}, 1_000_100.0)
            with open(G.seen_path(t, key), encoding="utf-8") as fh:
                rec = json.load(fh)
            self.assertEqual(rec["state"], G.SEEN_STATE_INFLIGHT)


class QueueDropTermination(unittest.TestCase):
    """★(0.14.31 · 성찰 Q6) **폐기도 종결이다** — 좌석 종료·자력 종료·clear·만료 축출.

    종전에는 `queue.dropped` 를 구독하지도 않았고 페이로드에 W-id 에코(`entry_ids`)도 없었다.
    그래서 폐기 4사유 중 `expired_evicted` 만(그것도 나란히 나가는 `queue.expired` 덕에) 종결이
    도달했고, 나머지 셋의 W-id 는 영원히 `inflight` 로 남아 seen TTL 마다 **낡은 wakeup_id 그대로**
    재enqueue 됐다(wakeup 홍수 — M7 이 없애려던 병리의 재개방).
    """

    def _pend(self, t, wid, key):
        G.seen_claim(t, key, G.SEV_CRIT, 1_000_000.0)
        G.seen_mark(t, key, 1_000_000.0, state=G.SEEN_STATE_INFLIGHT, wakeup_id=wid)

    def test_dropped_event_is_subscribed(self):
        with tempfile.TemporaryDirectory() as t:
            r = FakeRunner()
            g = gate(t, r)
            g._poll_once()
            self.assertTrue(r.polls, "이벤트 폴링을 하지 않았다")
            self.assertIn("queue.dropped", r.polls[0][1],
                          "폐기 사실을 구독하지 않는다 — 종결 신호가 도착조차 하지 않는다")

    def test_dropped_wakeup_is_terminally_disarmed_without_counting_as_delivered(self):
        for reason in ("surface_closed", "process_exited", "cleared", "expired_evicted"):
            with self.subTest(reason=reason), tempfile.TemporaryDirectory() as t:
                r = FakeRunner()
                key = G.seen_key("stall_confirmed", "s:w-%s" % reason, G.SEV_CRIT)
                self._pend(t, "W-000000dead", key)
                r.events = [{"name": "queue.dropped",
                             "payload": {"reason": reason,
                                         "entry_ids": ["W-000000dead"],
                                         "queue_entry_ids": ["q7"], "count": 1}}]
                g = gate(t, r)
                g._poll_once()
                counters = {}
                g._reconcile_inflight(counters, 1_000_100.0)
                with open(G.seen_path(t, key), encoding="utf-8") as fh:
                    rec = json.load(fh)
                self.assertEqual(rec["state"], G.SEEN_STATE_DROPPED,
                                 "폐기가 inflight 를 풀지 않았다 — seen TTL 마다 영구 재enqueue")
                self.assertNotEqual(rec["state"], G.SEEN_STATE_DELIVERED,
                                    "폐기를 배달로 셌다(배달되지 않은 일을 완료로 기록)")
                self.assertFalse(
                    counters.get("push_edge", {}).get(key, {}).get("armed", True),
                    "엣지가 무장 해제되지 않아 같은 주기에 다시 발화한다")

    def test_unrelated_drop_leaves_other_work_inflight(self):
        """다른 항목의 폐기가 내 wakeup 을 종결시키지 않는다(조인 키가 실제로 걸린다)."""
        with tempfile.TemporaryDirectory() as t:
            r = FakeRunner()
            key = G.seen_key("stall_confirmed", "s:worker4", G.SEV_CRIT)
            self._pend(t, "W-0000000004", key)
            r.events = [{"name": "queue.dropped",
                         "payload": {"reason": "cleared", "entry_ids": ["W-0000009999"]}}]
            g = gate(t, r)
            g._poll_once()
            g._reconcile_inflight({}, 1_000_100.0)
            with open(G.seen_path(t, key), encoding="utf-8") as fh:
                rec = json.load(fh)
            self.assertEqual(rec["state"], G.SEEN_STATE_INFLIGHT)

    def test_queue_entry_ids_are_not_joined_as_wakeup_ids(self):
        """`queue_entry_ids`(큐 항목 id)를 조인 키로 쓰면 두 체계가 섞인다 — 그 경로가 없다."""
        with tempfile.TemporaryDirectory() as t:
            r = FakeRunner()
            key = G.seen_key("stall_confirmed", "s:worker5", G.SEV_CRIT)
            self._pend(t, "W-0000000005", key)
            r.events = [{"name": "queue.dropped",
                         "payload": {"reason": "cleared",
                                     "queue_entry_ids": ["W-0000000005"],
                                     "entry_ids": []}}]
            g = gate(t, r)
            g._poll_once()
            g._reconcile_inflight({}, 1_000_100.0)
            with open(G.seen_path(t, key), encoding="utf-8") as fh:
                rec = json.load(fh)
            self.assertEqual(rec["state"], G.SEEN_STATE_INFLIGHT,
                             "queue_entry_ids 로 조인했다 — 두 id 체계 혼선")


class QueueHoldRefresh(unittest.TestCase):
    """★(0.14.31 · 성찰 Q7 · codex 설계 검토 #9) **보존·이동은 종결이 아니다 — TTL 창을 되감는다.**

    역할 좌석이 죽으면 데몬은 미배달 항목을 폐기하지 않고 보존소로 옮긴다(`queue.parked`) — 같은
    role 의 새 좌석에 `queue.rehomed` 로 되돌아온다. 그 사이 소비자의 inflight 가 seen TTL(30분)을
    넘기면 `seen_claim` 이 만료로 보고 같은 사건을 다시 enqueue 했다(원본이 살아 있는데 중복).
    """

    def _pend(self, t, wid, key, first_ts=1_000_000.0):
        G.seen_claim(t, key, G.SEV_CRIT, first_ts)
        G.seen_mark(t, key, first_ts, state=G.SEEN_STATE_INFLIGHT, wakeup_id=wid)

    def test_hold_events_are_subscribed(self):
        with tempfile.TemporaryDirectory() as t:
            r = FakeRunner()
            g = gate(t, r)
            g._poll_once()
            for name in ("queue.parked", "queue.rehomed"):
                self.assertIn(name, r.polls[0][1], "%s 를 구독하지 않는다" % name)

    def test_held_wakeup_stays_inflight_with_a_refreshed_ttl_window(self):
        for name in ("queue.parked", "queue.rehomed"):
            with self.subTest(event=name), tempfile.TemporaryDirectory() as t:
                r = FakeRunner()
                key = G.seen_key("stall_confirmed", "s:w-hold", G.SEV_CRIT)
                self._pend(t, "W-00000held1", key, first_ts=1_000_000.0)
                r.events = [{"name": name,
                             "payload": {"role": "worker", "entry_ids": ["W-00000held1"],
                                         "queue_entry_ids": ["q9"], "count": 1}}]
                g = gate(t, r)
                g._poll_once()
                counters = {}
                now = 1_000_000.0 + G.SEEN_TTL_SECS - 10  # TTL 만료 10초 전
                g._reconcile_inflight(counters, now)
                with open(G.seen_path(t, key), encoding="utf-8") as fh:
                    rec = json.load(fh)
                self.assertEqual(rec["state"], G.SEEN_STATE_INFLIGHT, "보존을 종결로 읽었다")
                self.assertEqual(rec["first_ts"], now, "TTL 창을 되감지 않았다 — 30분 뒤 재enqueue")
                # 되감긴 창 안에서는 재선점이 거부된다(= 같은 사건의 재enqueue 0).
                claimed, _ = G.seen_claim(t, key, G.SEV_CRIT, 1_000_000.0 + G.SEEN_TTL_SECS + 5)
                self.assertFalse(claimed, "원본이 보존 중인데 같은 사건을 다시 선점했다(중복)")

    def test_unrelated_hold_leaves_other_work_untouched(self):
        with tempfile.TemporaryDirectory() as t:
            r = FakeRunner()
            key = G.seen_key("stall_confirmed", "s:w-hold2", G.SEV_CRIT)
            self._pend(t, "W-00000held2", key, first_ts=1_000_000.0)
            r.events = [{"name": "queue.parked",
                         "payload": {"entry_ids": ["W-00000other"]}}]
            g = gate(t, r)
            g._poll_once()
            g._reconcile_inflight({}, 1_000_100.0)
            with open(G.seen_path(t, key), encoding="utf-8") as fh:
                rec = json.load(fh)
            self.assertEqual(rec["first_ts"], 1_000_000.0, "무관한 보존이 내 TTL 창을 건드렸다")


class P7InstrumentTests(unittest.TestCase):
    def sample(self, idle=2100, sample=1, role='master', sid=7, created=10):
        return dict(role=role, idle_secs=idle, sampled_at=sample, surface_id=sid,
                    created_at=created, source='daemon.status.idle_secs',
                    status_age_secs=1001, usage_ctx_tokens=100)

    def rep(self, *samples, label='master'):
        return report(nodes=[dict(node=label, done=1, total=2)], role_measurements=list(samples))

    def tick(self, counters, samples, epoch=10000, label='master'):
        return G.build_stall_warnings(counters, self.rep(*samples, label=label), 5, 6, 'ts', epoch)

    def matured(self):
        counters={}
        for i in range(7):
            warnings=self.tick(counters,[self.sample(2100+i*300,i+1)],10000+i*300)
        self.assertEqual(len(warnings),1)
        return counters,warnings

    def test_p7_row7_does_not_fire(self):
        c={'nodes':{'master':dict(sig='1/2',count=120,last_stall_fired=0)}}
        self.assertEqual(self.tick(c,[self.sample(560)]),[])

    def test_p7_identity_duplicates_both_orders_and_repeat(self):
        a=self.sample();b=self.sample(sid=8)
        for rows in ([a,b],[b,a]):
            m=G.measurements(self.rep(*rows))['master']
            self.assertIsNone(m['idle_secs'])
            self.assertEqual(m['source'],'ambiguous')
        self.assertEqual(G.measurements(self.rep(a,a))['master']['idle_secs'],2100)
        b=dict(a,idle_secs=2200)
        self.assertIsNone(G.measurements(self.rep(a,b))['master']['idle_secs'])

    def test_p7_identity_types_before_duplicate_equality(self):
        for invalid in (True, 1.0):
            for ids in ((invalid, 1), (1, invalid)):
                with self.subTest(ids=ids):
                    ms = G.measurements(self.rep(*(self.sample(sid=sid) for sid in ids)))
                    self.assertIsNone(ms['master']['idle_secs'])
                    self.assertEqual(G.update_idle_tracking({}, ms)['master']['consecutive_idle_cycles'], 0)

    def test_p7_replayed_timestamps_do_not_advance(self):
        for stamps in ([1000, 1001] * 5, [1000, 1001, 1002, 1000, 1001, 1002]):
            with self.subTest(stamps=stamps):
                c = {}
                high = -1
                for stamp in stamps:
                    before = c.get('idle_tracking_v1', {}).get('master', {}).get('consecutive_idle_cycles', 0)
                    self.assertEqual(self.tick(c, [self.sample(sample=stamp)]), [])
                    now = c['idle_tracking_v1']['master']
                    if stamp <= high:
                        self.assertLessEqual(now['consecutive_idle_cycles'], before)
                    high = max(high, stamp)
                    self.assertEqual(now['sample'][2], high)
        c = {}
        for stamp in range(1000, 1007):
            warnings = self.tick(c, [self.sample(sample=stamp)])
        self.assertEqual(c['idle_tracking_v1']['master']['consecutive_idle_cycles'], 6)
        self.assertEqual(len(warnings), 1)

    def test_p7_invalid_and_legacy_measurements(self):
        for v in (None,True,-1,'491'):
            self.assertIsNone(G.measurements(self.rep(self.sample(v)))['master']['idle_secs'])
        self.assertEqual(G.measurements(self.rep(self.sample(0)))['master']['idle_secs'],0)
        c={};legacy=self.sample();legacy.pop('surface_id')
        for i in range(9):
            legacy['sampled_at']=i
            self.assertEqual(self.tick(c,[legacy]),[])
        self.assertIn('master',c['idle_measurement_missing'])

    def test_p7_monotonic_cycles_reset_before_cooldown(self):
        c,w=self.matured()
        self.assertEqual(c['idle_tracking_v1']['master']['consecutive_idle_cycles'],6)
        self.tick(c,[self.sample(0,8)],12100)
        self.assertEqual(c['idle_tracking_v1']['master']['consecutive_idle_cycles'],0)
        self.assertEqual(self.tick(c,[self.sample(560,9)],15000),[])
        self.tick(c,[self.sample(400,10)],15300)
        self.assertEqual(c['idle_tracking_v1']['master']['consecutive_idle_cycles'],0)

    def test_p7_persistence_duplicate_and_identity(self):
        c,_=self.matured();c=json.loads(json.dumps(c))
        self.tick(c,[self.sample(3900,7)],15000)
        self.assertEqual(c['idle_tracking_v1']['master']['consecutive_idle_cycles'],6)
        self.tick(c,[self.sample(5000,8,created=11)],15300)
        self.assertEqual(c['idle_tracking_v1']['master']['consecutive_idle_cycles'],0)

    def test_p7_family_minimum_and_attribution(self):
        c={}
        for i in range(7):
            a=self.sample(2100+i*300,i+1,'reviewer-a',7)
            b=self.sample(2400+i*300,i+1,'reviewer-b',8)
            b['source']='source-b'
            w=self.tick(c,[a,b],10000+i*300,label='reviewer')
        self.assertEqual(w[0]['stamp']['measure_source'],'source-b')
        self.assertEqual(w[0]['evt_fields']['measurement_kind'],'todo_signature')
        b.update(idle_secs=560,sampled_at=8)
        a.update(sampled_at=8)
        self.assertEqual(self.tick(c,[a,b],16000,label='reviewer'),[])

    def test_p7_missing_visible_without_stall(self):
        c={};self.tick(c,[self.sample(None)])
        w=G.build_measure_warnings([],c.get('idle_measurement_missing'))
        self.assertEqual(len(w),1)
        self.assertIsNone(w[0]['evt_type'])
        self.assertEqual(w[0]['trigger'],'measure')

    def test_p7_gate_keeps_missing_diagnostic_without_stall(self):
        with tempfile.TemporaryDirectory() as t:
            r=FakeRunner(rep=self.rep(self.sample(None)),tasks=[])
            gate(t,r).run();gate(t,r).run()
            self.assertIn("gate-measure-master",badges(t))
            self.assertFalse(any(name=="agent.silent" for name,_ in r.emits))

    def test_p7_typed_event_preserves_legacy_formatter(self):
        import javis_event as E
        fields=dict(agent='master',silent_minutes=35,level='critical')
        legacy=E.speak('agent.silent',fields)
        typed=E.speak('agent.silent',dict(fields,measurement_kind='todo_signature',stall_cycles=6))
        self.assertNotEqual(typed,legacy)
        self.assertNotIn('응답이 없습니다',typed)
        self.assertIn('6',typed)

    def test_p7_invalid_persisted_sample_starts_at_zero(self):
        for old in (None, [], [7,10], [8,10,1], [7,10,True], [7,10,float("inf")]):
            c={"idle_tracking_v1":{"master":{"binding":[7,10],"prev_idle_secs":2100,
                "consecutive_idle_cycles":6,"sample":old}}}
            self.tick(c,[self.sample(2400,2)])
            self.assertEqual(c["idle_tracking_v1"]["master"]["consecutive_idle_cycles"],0)

    def test_p7_boolean_old_binding_never_aliases_surface_one(self):
        for binding,sample in [([True,1],[1,1,1]),([1,1],[True,1,1]),([1,True],[1,1,1]),([1,1],[1,True,1])]:
            c={'idle_tracking_v1':{'master':{'binding':binding,'sample':sample,
                'prev_idle_secs':2100,'consecutive_idle_cycles':6}}}
            self.tick(c,[self.sample(2400,2,sid=1,created=1)])
            self.assertEqual(c['idle_tracking_v1']['master']['consecutive_idle_cycles'],0)

    def test_p7_fast_distinct_polls_need_measured_duration(self):
        c={}
        for i in range(8):
            self.assertEqual(self.tick(c,[self.sample(560,i+1)],10000+i),[])
        self.assertGreaterEqual(c['idle_tracking_v1']['master']['consecutive_idle_cycles'],6)

    def test_p7_new_family_member_cannot_borrow_a_streak(self):
        c={}
        for i in range(7):
            self.tick(c,[self.sample(2400,i+1,'reviewer-a',7)],10000+i*300,label='reviewer')
        a=self.sample(2400,8,'reviewer-a',7);b=self.sample(2400,8,'reviewer-b',8)
        self.assertEqual(self.tick(c,[a,b],16000,label='reviewer'),[])

    def test_p7_producer_keeps_identity_and_rejects_bool(self):
        import javis_report as R
        m=R._measurement_from_node(self.sample(),123)
        self.assertEqual((m['surface_id'],m['created_at']),(7,10))
        self.assertIsNone(R._measurement_from_node(self.sample(True),123)['idle_secs'])


class ParseIsoEpochOffsetTest(unittest.TestCase):
    """0.14.56 #2 — `_parse_iso_epoch` must honour the stamp's UTC offset.

    The old body was `time.mktime(time.strptime(s, "...%z"))`: strptime parses the offset, mktime reads the
    fields as HOST-LOCAL time and drops it, so a stamp written in another timezone lands off by
    (stamp offset - host offset). Every cell runs in a child interpreter whose host offset is set through the
    TZ environment variable (POSIX strings, no tz database needed: KST-9 = UTC+9, ICT-7 = UTC+7, MST7 =
    UTC-7, UTC0), so this process's own clock is never touched. Each child first reports its own offset and
    the cell asserts it — a child that ignored TZ would make every cell vacuous.
    The instant is 2026-10-28T12:00:00Z written in each form."""

    INSTANT = calendar.timegm((2026, 10, 28, 12, 0, 0))
    HOSTS = {"KST-9": 32400, "ICT-7": 25200, "MST7": -25200, "UTC0": 0}
    OFFSET_FORMS = {
        "+0900": "2026-10-28T21:00:00+0900",
        "+09:00": "2026-10-28T21:00:00+09:00",
        "+0700": "2026-10-28T19:00:00+0700",
        "+0000": "2026-10-28T12:00:00+0000",
        "Z": "2026-10-28T12:00:00Z",
        "-0700": "2026-10-28T05:00:00-0700",
    }
    CHILD = textwrap.dedent("""
        import json, sys, time
        sys.path.insert(0, sys.argv[1])
        import javis_report_gate as G
        job = json.loads(sys.argv[2])
        out = {"gmtoff": time.localtime(job["at"]).tm_gmtoff,
               "parsed": {k: G._parse_iso_epoch(v) for k, v in job.get("parse", {}).items()}}
        if "deadlock" in job:
            d = job["deadlock"]
            tickets = [{"id": "T-1", "status": "todo", "owner": None,
                        "created_at": d["stamp"], "updated_at": d["stamp"]}]
            rep = {"live_nodes": [{"role": "master", "status_age_secs": 3600}]}
            dl, reason = G.build_deadlock_warning(rep, tickets, d["now"])
            out["deadlock"] = {"fired": dl is not None, "reason": reason}
        print(json.dumps(out))
    """)

    def child(self, tz, job):
        job = dict(job, at=self.INSTANT)
        env = dict(os.environ, TZ=tz)
        r = subprocess.run([sys.executable, "-B", "-c", self.CHILD, BIN, json.dumps(job)],
                           capture_output=True, text=True, env=env, timeout=60)
        self.assertEqual(r.returncode, 0, r.stderr[-800:])
        out = json.loads(r.stdout)
        self.assertEqual(out["gmtoff"], self.HOSTS[tz],
                         "child ignored TZ=%s (offset %r) - the cell would measure nothing" % (tz, out["gmtoff"]))
        return out

    def test_offset_bearing_forms_are_honoured_at_every_host_offset(self):
        for tz in self.HOSTS:
            got = self.child(tz, {"parse": self.OFFSET_FORMS})["parsed"]
            for form, value in got.items():
                self.assertIsNotNone(value, "host %s: %s did not parse" % (tz, form))
                self.assertEqual(value - self.INSTANT, 0,
                                 "host %s: %s read %+.2f h off (offset dropped)"
                                 % (tz, form, (value - self.INSTANT) / 3600.0))

    def test_deadlock_silent_direction_stamp_offset_above_host(self):
        # Host back at +0700, newest ticket stamped at +0900 40 minutes ago. Read with the offset dropped it
        # lands 2 h in the future, the age goes negative and the alarm stays silent past its 30-minute line.
        d = {"stamp": "2026-10-28T20:20:00+0900", "now": self.INSTANT}
        got = self.child("ICT-7", {"deadlock": d})["deadlock"]
        self.assertTrue(got["fired"], "40 min of ticket inactivity did not fire: %r" % got)

    def test_deadlock_noisy_direction_stamp_offset_below_host(self):
        # Host at +0900, newest ticket stamped at -0700 10 minutes ago. Read with the offset dropped it lands
        # 16 h in the past and the alarm fires on fresh activity.
        d = {"stamp": "2026-10-28T04:50:00-0700", "now": self.INSTANT}
        got = self.child("KST-9", {"deadlock": d})["deadlock"]
        self.assertFalse(got["fired"], "10 min of inactivity fired the deadlock alarm: %r" % got)
        self.assertEqual(got["reason"], "deadlock_no:recent_ticket_activity(600s)")

    def test_zone_less_stamp_is_host_local_wall_time(self):
        # Declared policy P-A (head decision 2026-10-28): a stamp without an offset is the host's local wall
        # time at read time. No shipped writer emits one; this pins the rule instead of letting it drift.
        wall = "2026-10-28T21:00:00"
        for tz, off in self.HOSTS.items():
            got = self.child(tz, {"parse": {"zl": wall}})["parsed"]["zl"]
            self.assertEqual(got, calendar.timegm((2026, 10, 28, 21, 0, 0)) - off,
                             "host %s: zone-less %s is not host-local" % (tz, wall))

    def test_fractional_seconds_stay_unparsed(self):
        # Declared: no writer emits fractional seconds; the parser returns None for them (unchanged).
        forms = {"f_off": "2026-10-28T21:00:00.123+0900", "f_z": "2026-10-28T12:00:00.123Z",
                 "f_zl": "2026-10-28T21:00:00.123"}
        got = self.child("KST-9", {"parse": forms})["parsed"]
        self.assertEqual(got, {k: None for k in forms})

    def test_numbers_and_empty_pass_through(self):
        self.assertEqual(G._parse_iso_epoch(1793018388), 1793018388.0)
        self.assertEqual(G._parse_iso_epoch(1793018388.5), 1793018388.5)
        self.assertIsNone(G._parse_iso_epoch(""))
        self.assertIsNone(G._parse_iso_epoch(None))


# ── 0.14.56 PACKTOOLS Phase 4 — deadlock ticket root: D-d · D-b · SHADOW (CEO ruling 4eeea43f Part B) ──
#   The ticket path below is the REAL one (G.Runner task methods + a javis_task.py subprocess); only the
#   report, event, wakeup and daemon calls are faked. HOME, the three override names and the publish seam are
#   pinned per test, so nothing reads or writes a live ~/.cys/state or a live pack.
class _TaskPathRunner(G.Runner):
    def __init__(self, pack_bin, state_dir, rep):
        super().__init__(pack_bin, state_dir)
        self.rep = rep
        self.emits, self.enqueues, self.drains, self.sends = [], [], [], []
        self._wid = 0

    def collect_report(self):
        self._sample = getattr(self, "_sample", 0) + 300
        self.rep["sampled_at"] = self._sample
        for i, row in enumerate(self.rep.get("live_nodes") or []):
            row.setdefault("surface_id", i + 1)
            row.setdefault("created_at", 1)
        return True, self.rep, None

    def emit(self, evt_type, fields, surface="auto"):
        self.emits.append((evt_type, fields))
        return 0, "", ""

    def enqueue(self, to, task, reason, idem, payload=None, severity=None):
        self.enqueues.append((to, task, reason, idem, severity))
        self._wid += 1
        return 0, "W-%010x" % self._wid

    def drain(self, target):
        self.drains.append(target)
        return 0, 1

    def poll_events(self, after_seq, names, timeout=0):
        return True, [], after_seq

    def send_queued(self, to, body):
        self.sends.append((to, body))
        return 0


class DeadlockRootShadowTest(unittest.TestCase):
    SCOPE = "pack-dept-dept-T"

    def setUp(self):
        from unittest import mock
        self._tmp = tempfile.TemporaryDirectory()
        self.tmp = self._tmp.name
        self.pack = os.path.join(self.tmp, self.SCOPE)
        os.makedirs(self.pack)
        os.symlink(BIN, os.path.join(self.pack, "bin"))
        self.pack_bin = os.path.join(self.pack, "bin")
        self.registry = os.path.join(self.pack, "state", "task-roots.json")
        self.ws = os.path.join(self.tmp, "ws")
        self.tasks_dir = os.path.join(self.ws, "_round", "tasks")
        #   A dept-shaped pack name needs a matching CYS_SOCKET or the gate skips the cycle as a foreign
        #   daemon (foreign_daemon_verdict). Nothing connects to it: every daemon-facing method is faked.
        env = {"HOME": os.path.join(self.tmp, "home"), "CYS_PACK_DIR": self.pack,
               "CYS_SOCKET": os.path.join(self.tmp, "cys-dept-dept-T", "cys.sock")}
        self._env = mock.patch.dict(os.environ, env)
        self._env.start()
        for k in ("JAVIS_ROOT", "JAVIS_TASK_ROOT", "CYS_TASK_ROOT", "JAVIS_TASK_PUBLISH_EPHEMERAL",
                  "CYS_SURFACE_ID", "AITERM_SURFACE_ID", "CYS_ROLE"):
            os.environ.pop(k, None)
        self.stale = report(live_nodes=[
            {"role": "master", "idle_secs": 60, "agent_alive": True, "status_age_secs": 3600},
            {"role": "cso", "idle_secs": 60, "agent_alive": True, "status_age_secs": 3600}])
        self.clock = Clock(time.time() + 7200)       # tickets written now are 2 h old to the gate

    def tearDown(self):
        for d, _, _ in os.walk(self.tmp):
            try:
                os.chmod(d, 0o755)
            except OSError:
                pass
        self._env.stop()
        self._tmp.cleanup()

    # ── helpers ──
    def jt(self, *argv, publish=True, root=None):
        env = dict(os.environ, JAVIS_ROOT=root or self.ws, CYS_PACK_DIR=self.pack, CYS_NO_AUTOSTART="1",
                   JAVIS_TASK_LIVENESS="alive", CYS_PROBE_RUNS=os.path.join(self.tmp, "probe_runs.jsonl"))
        if publish:
            env["JAVIS_TASK_PUBLISH_EPHEMERAL"] = "1"
        return subprocess.run([sys.executable, "-B", os.path.join(BIN, "javis_task.py")] + list(argv),
                              capture_output=True, text=True, env=env, timeout=60)

    def write_ticket(self, tasks_dir, tid, **extra):
        os.makedirs(tasks_dir, exist_ok=True)
        t = {"id": tid, "title": tid, "status": "todo", "owner": None,
             "created_at": "2026-10-28T00:00:00+0900", "updated_at": "2026-10-28T00:00:00+0900"}
        t.update(extra)
        with open(os.path.join(tasks_dir, tid + ".json"), "w", encoding="utf-8") as f:
            json.dump(t, f)

    def write_registry(self, *roots):
        os.makedirs(os.path.dirname(self.registry), exist_ok=True)
        with open(self.registry, "w", encoding="utf-8") as f:
            json.dump({"v": 1, "roots": {r: {"first_seen": 1, "last_write": 1, "role": None}
                                         for r in roots}}, f)

    def run_gate(self, runner=None):
        r = runner or _TaskPathRunner(self.pack_bin, os.path.join(self.tmp, "gate"), self.stale)
        sd = os.path.join(self.tmp, "gate")
        for _ in range(2):                              # run 1 is BASELINE (no classification)
            gate(sd, r, clock=self.clock).run()
            self.clock.epoch += 300
        return r, ledger_entries(sd)[-1]

    def shadow(self, entry, source=None):
        recs = entry.get("deadlock_shadow")
        self.assertIsInstance(recs, list, "ledger row has no deadlock_shadow list: %r" % sorted(entry))
        return [x for x in recs if source is None or x.get("source") == source]

    def assert_never_paged(self, r, entry):
        self.assertEqual([e for e in r.enqueues if e[1] == "gate-deadlock"], [], "shadow enqueued a page")
        self.assertEqual([e for e in r.emits if e[0] == "agent.silent"], [], "shadow emitted agent.silent")
        self.assertFalse(any(str(x).startswith("deadlock:") for x in entry.get("reasons") or []),
                         "the deadlock warn was routed (its reason reached the ledger)")

    # ── D-d: an absent or unreadable source is an INSTRUMENT-FAIL, never a clean zero ──
    def test_dd_registry_absent_is_instrument_fail(self):
        r, e = self.run_gate()
        recs = self.shadow(e, "registry")
        self.assertEqual(len(recs), 1, recs)
        self.assertEqual(recs[0]["verdict"], "instrument_fail")
        self.assertIn("registry_absent", recs[0]["reason"])
        self.assertNotIn("deadlock_no:no_unassigned_ticket", e.get("reasons") or [])

    def test_dd_published_root_absent_is_instrument_fail(self):
        self.write_registry(self.tasks_dir)                # published, then gone
        r, e = self.run_gate()
        recs = self.shadow(e, "registry")
        self.assertEqual([(x["root"], x["verdict"]) for x in recs], [(self.tasks_dir, "instrument_fail")])
        self.assertIn("root_absent", recs[0]["reason"])

    def test_dd_unreadable_root_is_instrument_fail(self):
        self.write_ticket(self.tasks_dir, "T-1", scope=self.SCOPE)
        self.write_registry(self.tasks_dir)
        os.chmod(self.tasks_dir, 0)
        r, e = self.run_gate()
        recs = self.shadow(e, "registry")
        self.assertEqual(recs[0]["verdict"], "instrument_fail")
        self.assertIn("root_unreadable", recs[0]["reason"])

    def test_dd_unreadable_registry_is_instrument_fail(self):
        os.makedirs(os.path.dirname(self.registry))
        with open(self.registry, "w", encoding="utf-8") as f:
            f.write("{not json")
        r, e = self.run_gate()
        recs = self.shadow(e, "registry")
        self.assertEqual(recs[0]["verdict"], "instrument_fail")
        self.assertIn("registry_unreadable", recs[0]["reason"])

    # ── D-b: the writer stamps scope and publishes its root; the gate reads only the registry ──
    def test_db_create_stamps_scope_and_publishes_root(self):
        p = self.jt("create", "x", "--id", "T-1")
        self.assertEqual(p.returncode, 0, p.stderr)
        with open(os.path.join(self.tasks_dir, "T-1.json"), encoding="utf-8") as f:
            self.assertEqual(json.load(f).get("scope"), self.SCOPE)
        with open(self.registry, encoding="utf-8") as f:
            reg = json.load(f)
        self.assertEqual(reg.get("v"), 1)
        self.assertEqual(sorted(reg.get("roots") or {}), [os.path.abspath(self.tasks_dir)])

    def test_db_set_status_publishes_root(self):
        self.assertEqual(self.jt("create", "x", "--id", "T-1").returncode, 0)
        if os.path.exists(self.registry):                   # create's publish is pinned elsewhere
            os.remove(self.registry)
        p = self.jt("set-status", "T-1", "backlog")
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertTrue(os.path.isfile(self.registry), "set-status did not publish its root")
        with open(self.registry, encoding="utf-8") as f:
            self.assertIn(os.path.abspath(self.tasks_dir), json.load(f)["roots"])

    def test_db_ephemeral_root_not_published_without_seam(self):
        # Guard (passes on the old bytes too): a test root under a temp prefix must not plant a root that is
        # gone tomorrow, or every later gate cycle reads a permanent INSTRUMENT-FAIL.
        p = self.jt("create", "x", "--id", "T-1", publish=False)
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertFalse(os.path.exists(self.registry), "an ephemeral root was published")

    def test_db_own_scope_unassigned_would_fire_without_paging(self):
        self.assertEqual(self.jt("create", "x", "--id", "T-1").returncode, 0)
        r, e = self.run_gate()
        recs = self.shadow(e, "registry")
        self.assertEqual(len(recs), 1, recs)
        self.assertEqual(recs[0]["root"], os.path.abspath(self.tasks_dir))
        self.assertEqual((recs[0]["verdict"], recs[0]["reason"]), ("would_fire", "deadlock_yes"))
        self.assertEqual((recs[0]["attributed"], recs[0]["unattributed"], recs[0]["foreign"]), (1, 0, 0))
        self.assert_never_paged(r, e)
        self.assertIn("deadlock_shadow:would_fire=1,no=0,unmeasured=0,instrument_fail=0", e["reasons"])

    def test_db_unscoped_legacy_row_is_unattributed_and_not_fired(self):
        self.write_ticket(self.tasks_dir, "T-L")             # pre-0.14.56 row: no scope key
        self.write_registry(os.path.abspath(self.tasks_dir))
        r, e = self.run_gate()
        rec = self.shadow(e, "registry")[0]
        self.assertEqual((rec["verdict"], rec["reason"]), ("no", "deadlock_no:no_unassigned_ticket"))
        self.assertEqual((rec["attributed"], rec["unattributed"], rec["foreign"]), (0, 1, 0))
        self.assert_never_paged(r, e)

    def test_db_foreign_scope_row_is_not_fired(self):
        self.write_ticket(self.tasks_dir, "T-F", scope="pack-dept-dept-OTHER")
        self.write_registry(os.path.abspath(self.tasks_dir))
        r, e = self.run_gate()
        rec = self.shadow(e, "registry")[0]
        self.assertEqual(rec["verdict"], "no")
        self.assertEqual((rec["attributed"], rec["unattributed"], rec["foreign"]), (0, 0, 1))

    def test_db_override_env_is_named_in_the_record(self):
        self.write_ticket(self.tasks_dir, "T-1", scope=self.SCOPE)
        os.environ["JAVIS_TASK_ROOT"] = self.ws
        r, e = self.run_gate()
        recs = self.shadow(e, "override:JAVIS_TASK_ROOT")
        self.assertEqual([(x["root"], x["verdict"]) for x in recs], [(self.tasks_dir, "would_fire")])
        self.assertEqual(self.shadow(e, "registry"), [], "the registry was read beside an explicit override")

    # ── B-2: one record per root, each with its own stop reason ──
    def test_b2_two_roots_one_record_each(self):
        gone = os.path.join(self.tmp, "gone", "_round", "tasks")
        self.assertEqual(self.jt("create", "x", "--id", "T-1").returncode, 0)
        self.assertTrue(os.path.isfile(self.registry), "create did not publish its root")
        with open(self.registry, encoding="utf-8") as f:
            reg = json.load(f)
        reg["roots"][gone] = {"first_seen": 1, "last_write": 1, "role": None}
        with open(self.registry, "w", encoding="utf-8") as f:
            json.dump(reg, f)
        r, e = self.run_gate()
        got = sorted((x["root"], x["verdict"]) for x in self.shadow(e, "registry"))
        self.assertEqual(got, sorted([(gone, "instrument_fail"), (os.path.abspath(self.tasks_dir), "would_fire")]))
        self.assert_never_paged(r, e)

    # ── D-c: an in-memory positive control every cycle; a control that does not fire blinds the verdict ──
    def test_dc_control_record_every_run(self):
        r, e = self.run_gate()
        ctl = self.shadow(e, "control")
        self.assertEqual([(x["verdict"], x["reason"]) for x in ctl], [("control_fired", "deadlock_yes")])
        self.assertTrue(G.deadlock_positive_control(self.clock.epoch)[0])

    def test_dc_failed_control_turns_every_root_into_instrument_fail(self):
        from unittest import mock
        self.write_ticket(self.tasks_dir, "T-O", scope=self.SCOPE, owner="worker")   # a clean "no"
        self.write_registry(os.path.abspath(self.tasks_dir))
        with mock.patch.object(G, "deadlock_positive_control", create=True,
                               return_value=(False, "deadlock_no:injected")):
            r, e = self.run_gate()
        rec = self.shadow(e, "registry")[0]
        self.assertEqual(rec["verdict"], "instrument_fail")
        self.assertTrue(rec["reason"].startswith("deadlock_instrument_fail:control"), rec)
        self.assertIn("deadlock_no:no_unassigned_ticket", rec["reason"], "the per-root stop reason was lost")

    # ── legacy runner (no task_roots): the old single source still answers, shadow only ──
    def test_legacy_task_snapshot_runner_is_shadow_only(self):
        old = "2023-11-01T00:00:00+0000"
        fr = FakeRunner(rep=self.stale, tasks=[{"id": "T-1", "status": "todo", "owner": None,
                                                "created_at": old, "updated_at": old}])
        r, e = self.run_gate(fr)
        recs = self.shadow(e, "legacy:task_snapshot")
        self.assertEqual([(x["verdict"], x["reason"]) for x in recs], [("would_fire", "deadlock_yes")])
        self.assert_never_paged(r, e)


if __name__ == "__main__":
    unittest.main()
