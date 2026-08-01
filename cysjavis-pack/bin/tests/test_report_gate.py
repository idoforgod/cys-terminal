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


if __name__ == "__main__":
    unittest.main()
