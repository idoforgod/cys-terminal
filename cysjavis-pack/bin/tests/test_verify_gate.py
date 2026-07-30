#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_verify_gate.py — T1 verify_spec 게이트·brief_lint 통합 격리 검증 (standalone).

발효 태그 체계(run_bootstrap_health `specimen`/`LANDED_WAVES`)에는 **등록하지 않는다** —
러너 편입은 Wave 착지 규약이 별도다(T1 브리프 산출물 7 · 발효 태그 체계 무접촉).
본 파일은 standalone 실행 전용:

    JAVIS_ROOT=<scratch> CYS_PROBE_RUNS=<scratch>/probe_runs.jsonl \
    HUD_STATE_DIR=<scratch>/hud CYS_NO_AUTOSTART=1 \
    python3 cysjavis-pack/bin/tests/test_verify_gate.py

★격리 hard assert(안전 계약 G1 — T1 브리프): JAVIS_ROOT·CYS_PROBE_RUNS 미설정 시 **실행 거부**.
  javis_task/wakeup 의 `ROOT = env or os.getcwd()` cwd 폴백이 라이브 `_round/`·pack state 를
  오염시키는 경로를 실행 전에 결정론 차단한다. 모든 케이스는 JAVIS_ROOT 하위 임시 루트에서만
  쓰고, cys 는 PATH 셔임으로만 존재한다(라이브 cys CLI 호출 0).

커버(T1 완료 기준 매핑): ①3모드 ②grandfather ③동일 거부 10회→pending 1건+drain 흔적
+A4 마커 억제/해제 ④--brief hard·경고 훅 비차단 ⑤기존 exit 0·2·3·4·5·8·9 회귀
⑥set-verify-spec 동시성 ⑦P1b probe 영수증 성공 경로(U16) ⑧(B7) 마커 자동생성 경로
⑨시스템 클래스 템플릿.
"""
import json
import os
import subprocess
import sys
import tempfile
import threading
import time
import unittest


def _hard_assert_isolation():
    missing = [k for k in ("JAVIS_ROOT", "CYS_PROBE_RUNS") if not os.environ.get(k)]
    if missing:
        sys.stderr.write(
            "REFUSE(hard assert): 격리 env 미설정 %s — cwd 폴백의 라이브 오염 차단(G1). "
            "JAVIS_ROOT=<scratch> CYS_PROBE_RUNS=<scratch>/probe_runs.jsonl 로 재실행하라.\n"
            % missing)
        sys.exit(2)


_hard_assert_isolation()

TESTS_DIR = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(TESTS_DIR)
HOOKS = os.path.join(os.path.dirname(BIN), "hooks")
TASK = os.path.join(BIN, "javis_task.py")
ACTPROBE = os.path.join(BIN, "javis_actprobe.py")
BRIEF_HOOK = os.path.join(HOOKS, "brief-lint-warn.sh")
PY = sys.executable or "python3"
BASE = os.environ["JAVIS_ROOT"]

EXIT_VERIFY = 6


def _iso(epoch):
    return time.strftime("%Y-%m-%dT%H:%M:%S%z", time.localtime(epoch))


class T1GateCase(unittest.TestCase):
    def setUp(self):
        os.makedirs(BASE, exist_ok=True)
        self.root = tempfile.mkdtemp(prefix="t1-gate-", dir=BASE)
        self.tasks = os.path.join(self.root, "_round", "tasks")
        # cys PATH 셔임 — 라이브 cys 호출 0 보증 + 배달 시도 로그(exit 1 = 배달실패 경로)
        self.shim_dir = os.path.join(self.root, "shim")
        os.makedirs(self.shim_dir, exist_ok=True)
        self.shim_log = os.path.join(self.shim_dir, "cys.log")
        shim = os.path.join(self.shim_dir, "cys")
        with open(shim, "w", encoding="utf-8") as f:
            f.write('#!/bin/sh\necho "$@" >> "%s"\nexit 1\n' % self.shim_log)
        os.chmod(shim, 0o755)

    def env(self, **extra):
        env = dict(os.environ)
        env.update({
            "JAVIS_ROOT": self.root,
            "JAVIS_TASK_LIVENESS": "alive",
            "JAVIS_WAKEUP_LIVENESS": "alive",
            "JAVIS_FASTFAIL_MAX": "99",
            "JAVIS_VERIFY_GATE": "warn",
            "CYS_NO_AUTOSTART": "1",
            "CYS_PROBE_RUNS": os.path.join(self.root, "probe_runs.jsonl"),
            "CYS_ACTPROBE_CALLER": "t1-test",
            "PATH": self.shim_dir + os.pathsep + os.environ.get("PATH", ""),
        })
        env.update(extra)
        return env

    def task(self, argv, **extra):
        r = subprocess.run([PY, TASK] + argv, capture_output=True, text=True,
                           env=self.env(**extra), timeout=120)
        return r.returncode, r.stdout, r.stderr

    def read_task(self, tid):
        with open(os.path.join(self.tasks, tid + ".json"), encoding="utf-8") as f:
            return json.load(f)

    def write_marker(self, epoch):
        os.makedirs(self.tasks, exist_ok=True)
        with open(os.path.join(self.tasks, ".verify-gate-activated"), "w",
                  encoding="utf-8") as f:
            f.write(_iso(epoch) + "\n")

    def backdate(self, tid, epoch):
        p = os.path.join(self.tasks, tid + ".json")
        with open(p, encoding="utf-8") as f:
            t = json.load(f)
        t["created_at"] = _iso(epoch)
        with open(p, "w", encoding="utf-8") as f:
            json.dump(t, f, ensure_ascii=False)

    # ── ① 3모드 매트릭스 ──
    def test_01_mode_matrix(self):
        self.write_marker(time.time() - 3600)  # strict 승격 1h 전 집행 상태 조성
        rc, _, _ = self.task(["create", "T1", "--id", "T1"])
        self.assertEqual(rc, 0)
        rc, _, err = self.task(["checkout", "T1", "--owner", "w1"],
                               JAVIS_VERIFY_GATE="strict")
        self.assertEqual(rc, EXIT_VERIFY, err)
        self.assertIn("verify-spec missing(6): 재시도 금지 — master가 verify_spec 작성 후 재위임. "
                      "통지 큐 등재됨(태스크당 1회 병합·미배달 시 후속 drain 편승).", err)
        rc, _, err = self.task(["checkout", "T1", "--owner", "w1"],
                               JAVIS_VERIFY_GATE="warn")
        self.assertEqual(rc, 0, err)
        self.assertIn("verify-gate WARN", err)
        rc, _, _ = self.task(["release", "T1", "--owner", "w1"])
        self.assertEqual(rc, 0)
        rc, _, err = self.task(["checkout", "T1", "--owner", "w1"],
                               JAVIS_VERIFY_GATE="off")
        self.assertEqual(rc, 0, err)
        self.assertNotIn("verify-gate", err)

    # ── ② grandfather ──
    def test_02_grandfather(self):
        self.write_marker(time.time() - 3600)
        rc, _, _ = self.task(["create", "Tgf", "--id", "Tgf"])
        self.assertEqual(rc, 0)
        self.backdate("Tgf", time.time() - 7200)
        rc, _, err = self.task(["checkout", "Tgf", "--owner", "w1"],
                               JAVIS_VERIFY_GATE="strict")
        self.assertEqual(rc, 0, err)
        self.assertIn("verify-gate WARN", err)
        self.assertIn("grandfathered", err)
        self.assertIs(self.read_task("Tgf").get("grandfathered"), True)

    # ── ③ 동일 거부 10회 → pending 정확 1건(D17 — drain 전 스냅샷) + drain 시도 흔적
    #    + A4: enqueue 는 1회차만(<id>.verify-notified 마커 억제)·set-verify-spec 이 마커 해제 ──
    def test_03_reject_notification(self):
        self.write_marker(time.time() - 3600)
        rc, _, _ = self.task(["create", "Trej", "--id", "Trej"])
        self.assertEqual(rc, 0)
        for _ in range(10):
            rc, _, _ = self.task(["checkout", "Trej", "--owner", "w1"],
                                 JAVIS_VERIFY_GATE="strict")
            self.assertEqual(rc, EXIT_VERIFY)
        pend_dir = os.path.join(self.root, "_round", "wakeups", "pending")
        pend = [n for n in os.listdir(pend_dir) if n.endswith(".json")]
        self.assertEqual(len(pend), 1, "pending %s" % pend)
        rec = json.load(open(os.path.join(pend_dir, pend[0]), encoding="utf-8"))
        self.assertEqual(rec["target"], "master")
        self.assertEqual(rec["task_key"], "verify-spec-missing-Trej")
        self.assertEqual(rec["idempotency_keys"], ["Trej"])
        # A4 마커: 1회차 enqueue 성공 시 생성 — 이후 9회는 enqueue 자체 생략(재발화 억제)
        mark = os.path.join(self.tasks, "Trej.verify-notified")
        self.assertTrue(os.path.isfile(mark), "A4 통지 마커 미생성")
        # same-run drain 배달 시도 흔적 — 1회차 drain 의 cys send 정확 1회(종전 10회 → 억제 증거)
        with open(self.shim_log, encoding="utf-8") as f:
            sends = [ln for ln in f.read().splitlines() if "send" in ln]
        self.assertEqual(len(sends), 1, sends)
        with open(os.path.join(self.root, "_round", "wakeups", "queue.jsonl"),
                  encoding="utf-8") as f:
            ledger = f.read()
        self.assertIn("deliver_failed", ledger)
        queued = [ln for ln in ledger.splitlines()
                  if '"queued"' in ln and "verify-spec-missing-Trej" in ln]
        self.assertEqual(len(queued), 1, "A4 마커 억제 실패 — enqueue %d회" % len(queued))
        # set-verify-spec 성공 → 마커 삭제(재위임 사이클 재개) → spec 보유 checkout 통과
        spec = json.dumps({"mode": "command", "cmd": "python3 -c 0",
                           "pass_rule": {"kind": "exit_map", "pass_exits": [0]}})
        rc, _, err = self.task(["set-verify-spec", "Trej", "--json", spec])
        self.assertEqual(rc, 0, err)
        self.assertFalse(os.path.isfile(mark), "A4 set-verify-spec 후 마커 미삭제")
        rc, _, err = self.task(["checkout", "Trej", "--owner", "w1"],
                               JAVIS_VERIFY_GATE="strict")
        self.assertEqual(rc, 0, err)

    # ── ④ --brief hard + 경고 훅 비차단 ──
    def test_04_brief_paths(self):
        bad = os.path.join(self.root, "bad-brief.md")
        with open(bad, "w", encoding="utf-8") as f:
            f.write("# task\n구현만 서술\n")
        ok = os.path.join(self.root, "ok-brief.md")
        with open(ok, "w", encoding="utf-8") as f:
            f.write("# task\n구현\n# verify\n- python3 x.py self-test → exit 0\n")
        rc, _, _ = self.task(["create", "Tb", "--id", "Tb"])
        self.assertEqual(rc, 0)
        rc, _, err = self.task(["checkout", "Tb", "--owner", "w1", "--brief", bad])
        self.assertEqual(rc, EXIT_VERIFY, err)
        self.assertIn("verify[]", err)
        rc, _, err = self.task(["checkout", "Tb", "--owner", "w1", "--brief", ok])
        self.assertEqual(rc, 0, err)
        # 경고 훅: 브리프 미동봉 Agent 호출 — 비차단 exit 0(조건 05③)
        p = subprocess.run(["sh", BRIEF_HOOK], input=json.dumps(
            {"tool_name": "Agent", "tool_input": {"prompt": "브리프 없는 인세션 위임"},
             "session_id": "t1"}), capture_output=True, text=True,
            env=self.env(), timeout=60)
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertIn("additionalContext", p.stdout)
        # malformed stdin 도 비차단
        p = subprocess.run(["sh", BRIEF_HOOK], input="not json",
                           capture_output=True, text=True, env=self.env(), timeout=60)
        self.assertEqual(p.returncode, 0, p.stderr)

    # ── ⑤ 기존 exit 회귀(0·2·3·4·5·8·9 — T0 §3-2 표 재현) ──
    def test_05_exit_regression(self):
        rc, _, _ = self.task(["create", "Te", "--id", "Te"])
        self.assertEqual(rc, 0)                                    # 0 정상
        rc, _, _ = self.task(["create", "dup", "--id", "Te"])
        self.assertEqual(rc, 8)                                    # 8 duplicate
        rc, _, _ = self.task(["show", "nope-404"])
        self.assertEqual(rc, 3)                                    # 3 not found
        rc, _, _ = self.task(["checkout", "../evil", "--owner", "w"])
        self.assertEqual(rc, 2)                                    # 2 usage(G10)
        rc, _, _ = self.task(["create", "Tblk", "--id", "Tblk", "--blocked-by", "Te"])
        self.assertEqual(rc, 0)
        rc, _, _ = self.task(["checkout", "Tblk", "--owner", "w1"])
        self.assertEqual(rc, 4)                                    # 4 blocked
        rc, _, _ = self.task(["checkout", "Te", "--owner", "w1"])
        self.assertEqual(rc, 0)
        rc, _, _ = self.task(["checkout", "Te", "--owner", "w2"])
        self.assertEqual(rc, 9)                                    # 9 conflict(산 소유자)
        rc, _, _ = self.task(["set-status", "Te", "done", "--evidence",
                              "텍스트만으로 done 시도 12345", "--settled-override", "t"])
        self.assertEqual(rc, 5)                                    # 5 no evidence(artifact 부재)

    # ── ⑥ set-verify-spec ∥ set-status 동시성 — lost-update 0 ──
    def test_06_concurrency(self):
        spec = json.dumps({"mode": "command", "cmd": "python3 -c 0",
                           "pass_rule": {"kind": "exit_map", "pass_exits": [0]}})
        for it in range(3):
            tid = "Tc%d" % it
            rc, _, _ = self.task(["create", tid, "--id", tid])
            self.assertEqual(rc, 0)
            rc, _, _ = self.task(["checkout", tid, "--owner", "w1"])
            self.assertEqual(rc, 0)
            res = {}
            t1 = threading.Thread(
                target=lambda: res.setdefault("sv", self.task(
                    ["set-verify-spec", tid, "--json", spec])))
            t2 = threading.Thread(
                target=lambda: res.setdefault("ss", self.task(
                    ["set-status", tid, "in_review"])))
            t1.start(); t2.start(); t1.join(60); t2.join(60)
            self.assertEqual(res["sv"][0], 0, res["sv"][2])
            self.assertEqual(res["ss"][0], 0, res["ss"][2])
            final = self.read_task(tid)
            self.assertEqual(final.get("status"), "in_review", final)
            self.assertEqual((final.get("verify_spec") or {}).get("mode"), "command", final)

    # ── ⑦ P1b probe 영수증 성공 경로(U16 — --runs-path 격리·라이브 무접촉) ──
    def test_07_probe_receipt_success(self):
        runs = os.path.join(self.root, "probe_runs.jsonl")
        rc, _, _ = self.task(["create", "Tpr", "--id", "Tpr"])
        self.assertEqual(rc, 0)
        rc, _, _ = self.task(["checkout", "Tpr", "--owner", "w1"])
        self.assertEqual(rc, 0)
        art = os.path.join(self.root, "out.txt")
        with open(art, "w", encoding="utf-8") as f:
            f.write("verified artifact\n")
        p = subprocess.run([PY, ACTPROBE, "artifact", "--path", art, "--task", "Tpr",
                            "--runs-path", runs, "--caller", "t1-test"],
                           capture_output=True, text=True, env=self.env(), timeout=60)
        self.assertEqual(p.returncode, 0, p.stdout + p.stderr)  # PASS 영수증 기록
        rc, out, err = self.task(["set-status", "Tpr", "done",
                                  "--evidence", "probe:artifact 실측 → %s" % art,
                                  "--evidence-artifact", art,
                                  "--settled-override", "t1-test"])
        self.assertEqual(rc, 0, err)  # 영수증 대조(최근성·exit0·target/task) 통과 = done

    # ── ⑧(B7) 마커 자동생성 경로: 마커 부재 strict 최초 checkout = 승격 집행 —
    #    grandfather WARN 통과+마커 생성 → 직후 신규 create checkout → exit 6 ──
    def test_08_marker_autocreate(self):
        rc, _, _ = self.task(["create", "Told", "--id", "Told"])
        self.assertEqual(rc, 0)
        time.sleep(1.1)  # created_at(초 해상도) < 마커 생성 시각 엄밀 보장
        rc, _, err = self.task(["checkout", "Told", "--owner", "w1"],
                               JAVIS_VERIFY_GATE="strict")
        self.assertEqual(rc, 0, err)
        self.assertIn("grandfathered", err)
        self.assertTrue(os.path.isfile(
            os.path.join(self.tasks, ".verify-gate-activated")), "B7 마커 자동생성 안 됨")
        rc, _, _ = self.task(["create", "Tnew", "--id", "Tnew"])
        self.assertEqual(rc, 0)
        rc, _, err = self.task(["checkout", "Tnew", "--owner", "w2"],
                               JAVIS_VERIFY_GATE="strict")
        self.assertEqual(rc, EXIT_VERIFY, err)

    # ── ⑨ 시스템 클래스 → 템플릿 자동 부여 ──
    def test_09_system_template(self):
        for kind in ("recovery", "healing", "watchdog"):
            tid = "Ts-" + kind
            rc, _, _ = self.task(["create", tid, "--id", tid, "--origin-kind", kind])
            self.assertEqual(rc, 0)
            spec = self.read_task(tid).get("verify_spec")
            self.assertIsNotNone(spec, tid)
            self.assertEqual(spec.get("template"), "system-readonly-v1")
            self.assertEqual(spec.get("template_origin"), kind)
            rc, _, err = self.task(["checkout", tid, "--owner", "w1"],
                                   JAVIS_VERIFY_GATE="strict")
            self.assertEqual(rc, 0, err)  # 치유 티켓이 게이트에 걸리지 않음(조건 16②)


if __name__ == "__main__":
    unittest.main(verbosity=2)
