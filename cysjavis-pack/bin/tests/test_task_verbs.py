#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_task_verbs.py — P2 settle-gate pass audit row and P3 lift verb (0.14.57 post-release queue).

Sealed by itself, so it needs no env from the caller: every case makes its own tmp JAVIS_ROOT and tmp
CYS_PROBE_RUNS, drops inherited CYS_*, JAVIS_*, AITERM_* and CLAUDE_CONFIG_DIR, and points HOME at tmp.
★JAVIS_ROOT alone does not seal this tool: the audit sink is <pack>/state/probe_runs.jsonl, outside
JAVIS_ROOT, and falls back to the live pack when CYS_PROBE_RUNS is unset.

    python3 -B cysjavis-pack/bin/tests/test_task_verbs.py -v

Each `[control]` line prints what was measured.
"""
import importlib.util
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
import unittest
from unittest import mock

HERE = os.path.dirname(os.path.abspath(__file__))
TASK = os.path.join(os.path.dirname(HERE), "javis_task.py")
PY = sys.executable
PASS_ROW = "settle-gate-pass"


def control(msg):
    print("[control] " + msg, flush=True)


class Base(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="task-verbs-")
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.root = os.path.join(self.tmp, "root")
        os.makedirs(self.root)
        self.runs = os.path.join(self.tmp, "probe_runs.jsonl")

    def env(self, **extra):
        env = {k: v for k, v in os.environ.items()
               if not k.startswith(("CYS_", "JAVIS_", "AITERM_")) and k != "CLAUDE_CONFIG_DIR"}
        env.update({
            "HOME": self.tmp,
            "JAVIS_ROOT": self.root,
            "CYS_PROBE_RUNS": self.runs,
            "CYS_NO_AUTOSTART": "1",
            "CYS_ACTPROBE_CALLER": "test-task-verbs",
            "JAVIS_VERIFY_GATE": "warn",
            "JAVIS_TASK_LIVENESS": "alive",
            "JAVIS_SETTLE_PROBE": "9:1;9:1",   # owner idle 9s, TODO mtime still: the gate passes
            "JAVIS_SETTLE_SEC": "0",
            "CYS_TASK_EVIDENCE_ARTIFACT_GATE": "off",
        })
        env.update(extra)
        return env

    def t(self, *argv, **extra):
        r = subprocess.run([PY, "-B", TASK] + list(argv), capture_output=True, text=True,
                           env=self.env(**extra), timeout=60)
        return r.returncode, r.stdout, r.stderr

    def burst(self, argvs, **extra):
        procs = [subprocess.Popen([PY, "-B", TASK] + list(a), stdout=subprocess.PIPE,
                                  stderr=subprocess.PIPE, text=True, env=self.env(**extra))
                 for a in argvs]
        out = []
        for p in procs:
            so, se = p.communicate(timeout=60)
            out.append((p.returncode, so, se))
        return out

    def path(self, tid):
        return os.path.join(self.root, "_round", "tasks", tid + ".json")

    def rec(self, tid):
        with open(self.path(tid), encoding="utf-8") as f:
            return json.load(f)

    def raw(self, tid):
        with open(self.path(tid), "rb") as f:
            return f.read()

    def rows(self, tid=None, kind=None):
        if not os.path.exists(self.runs):
            return []
        with open(self.runs, encoding="utf-8") as f:
            rows = [json.loads(x) for x in f if x.strip()]
        return [r for r in rows if r.get("probe") == "task-audit"
                and (tid is None or r.get("target") == tid)
                and (kind is None or r.get("audit") == kind)]

    def load_module(self):
        with mock.patch.dict(os.environ, self.env(), clear=True):
            spec = importlib.util.spec_from_file_location("javis_task_under_test", TASK)
            mod = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(mod)
        return mod

    def call(self, mod, *argv):
        with mock.patch.dict(os.environ, self.env(), clear=True):
            return mod.main(list(argv))

    def checked_out(self, tid):
        self.assertEqual(self.t("create", tid, "--id", tid)[0], 0)
        self.assertEqual(self.t("checkout", tid, "--owner", "w1")[0], 0)


class P2SettleGatePassRow(Base):
    EV = ("--evidence", "unit test evidence")

    def test_pass_gives_one_row(self):
        self.checked_out("A")
        rc, _, se = self.t("set-status", "A", "done", *self.EV)
        rows = self.rows("A")
        control("P2 pass: rc=%s status=%s owner=%s rows=%s" % (
            rc, self.rec("A")["status"], self.rec("A")["owner"], [r["audit"] for r in rows]))
        self.assertEqual(rc, 0, se)
        self.assertEqual([r["audit"] for r in rows], [PASS_ROW])
        self.assertEqual(rows[0]["caller"], "test-task-verbs")

    def test_row_is_written_after_the_transition_is_durable(self):
        self.checked_out("A")
        mod = self.load_module()
        seen = []
        real = mod._append_audit

        def spy(tid, kind):
            seen.append((kind, self.rec(tid)["status"], self.rec(tid)["owner"]))
            real(tid, kind)
        mod._append_audit = spy
        rc = self.call(mod, "set-status", "A", "done", *self.EV)
        control("P2 durable: rc=%s at the moment of the audit call, disk read %s" % (rc, seen))
        self.assertEqual(rc, 0)
        self.assertEqual(seen, [(PASS_ROW, "done", None)])

    def test_failed_write_gives_no_row(self):
        self.checked_out("A")
        mod = self.load_module()

        def boom(path, obj):
            raise OSError("disk full, injected")
        mod._write_json_atomic = boom
        with self.assertRaises(OSError):
            self.call(mod, "set-status", "A", "done", *self.EV)
        control("P2 write fails: status=%s rows=%s" % (self.rec("A")["status"], self.rows("A")))
        self.assertEqual(self.rec("A")["status"], "in_progress")
        self.assertEqual(self.rows("A"), [])

    def test_owner_unset_gives_no_row(self):
        self.t("create", "A", "--id", "A")
        self.t("set-status", "A", "in_progress")
        rc, _, se = self.t("set-status", "A", "done", *self.EV)
        control("P2 owner-unset: rc=%s status=%s rows=%s" % (rc, self.rec("A")["status"], self.rows("A")))
        self.assertEqual(rc, 0, se)
        self.assertEqual(self.rec("A")["status"], "done")
        self.assertEqual(self.rows("A"), [])

    def test_override_is_recorded_and_distinguishable(self):
        self.checked_out("OVR")
        rc, _, se = self.t("set-status", "OVR", "done", "--settled-override", "owner declares own done",
                           *self.EV)
        self.assertEqual(rc, 0, se)
        self.checked_out("PASS")
        self.t("set-status", "PASS", "done", *self.EV)
        self.t("create", "NONE", "--id", "NONE")
        self.t("set-status", "NONE", "in_progress")
        self.t("set-status", "NONE", "done", *self.EV)
        shape = {}
        for tid in ("OVR", "PASS", "NONE"):
            shape[tid] = (len(self.rows(tid, PASS_ROW)), len(self.rec(tid).get("settle_overrides", [])))
        control("P2 three shapes (pass rows, settle_overrides): %s" % shape)
        self.assertEqual(shape, {"OVR": (0, 1), "PASS": (1, 0), "NONE": (0, 0)})
        self.assertEqual(self.rec("OVR")["settle_overrides"][0]["reason"], "owner declares own done")

    def test_denied_gives_no_row(self):
        self.checked_out("SETTLE")
        rc, _, _ = self.t("set-status", "SETTLE", "done", *self.EV, JAVIS_SETTLE_PROBE="1:1")
        control("P2 denied, gate says not settled: rc=%s status=%s rows=%s" % (
            rc, self.rec("SETTLE")["status"], self.rows("SETTLE")))
        self.assertEqual(rc, 4)
        self.assertEqual(self.rows("SETTLE"), [])

        self.checked_out("NOEV")
        rc, _, _ = self.t("set-status", "NOEV", "done")
        control("P2 denied, no evidence: rc=%s rows=%s" % (rc, self.rows("NOEV")))
        self.assertEqual(rc, 5)
        self.assertEqual(self.rows("NOEV"), [])

        # The gate itself passes here and the transition table then denies: a row placed at the gate would lie.
        self.checked_out("TRANS")
        self.assertEqual(self.t("set-status", "TRANS", "blocked")[0], 0)
        rc, _, _ = self.t("set-status", "TRANS", "done", *self.EV)
        control("P2 denied after the gate passed, blocked to done: rc=%s status=%s rows=%s" % (
            rc, self.rec("TRANS")["status"], self.rows("TRANS")))
        self.assertEqual(rc, 2)
        self.assertEqual(self.rows("TRANS"), [])

    def test_row_names_the_settle_gate_not_the_evidence_gate(self):
        self.checked_out("A")
        rc, _, se = self.t("set-status", "A", "done", CYS_TASK_EVIDENCE_GATE="off")
        kinds = sorted(r["audit"] for r in self.rows("A"))
        control("P2 evidence gate off, settle gate passed: rc=%s rows=%s" % (rc, kinds))
        self.assertEqual(rc, 0, se)
        self.assertEqual(kinds, ["gate-off", PASS_ROW])


class P3Lift(Base):
    WHO = ("--by", "head", "--why", "hold reviewed, cleared")

    def blocked(self, tid, *blockers, status=None):
        argv = ["create", tid, "--id", tid, "--blocked-by"] + list(blockers)
        if status:
            argv += ["--status", status]
        self.assertEqual(self.t(*argv)[0], 0)

    def ready(self, tid):
        rc, so, _ = self.t("ready", tid)
        return rc, json.loads(so)

    def test_measured_disagreement_is_cured(self):
        self.t("create", "B", "--id", "B")
        self.blocked("A", "B")
        before = self.rec("A")
        ready_rc, ready = self.ready("A")
        co_rc = self.t("checkout", "A", "--owner", "w1")[0]
        control("P3 before: status=%s blocked_by=%s ready_rc=%s ready=%s checkout_rc=%s" % (
            before["status"], before["blocked_by"], ready_rc, ready["ready"], co_rc))
        self.assertEqual((before["status"], before["blocked_by"]), ("todo", ["B"]))
        self.assertEqual((ready_rc, ready["ready"], co_rc), (4, False, 4))

        rc, so, se = self.t("lift", "A", "--blocker", "B", *self.WHO)
        after = self.rec("A")
        ready_rc, ready = self.ready("A")
        co_rc = self.t("checkout", "A", "--owner", "w1")[0]
        control("P3 after: lift_rc=%s status=%s blocked_by=%s ready_rc=%s ready=%s checkout_rc=%s history=%s" % (
            rc, after["status"], after["blocked_by"], ready_rc, ready["ready"], co_rc, after["lift_history"]))
        self.assertEqual(rc, 0, se)
        self.assertEqual((after["status"], after["blocked_by"]), ("todo", []))
        self.assertEqual((ready_rc, ready["ready"], co_rc), (0, True, 0))
        self.assertEqual(json.loads(so)["ready"], True)
        hist = after["lift_history"]
        self.assertEqual(len(hist), 1)
        self.assertEqual((hist[0]["by"], hist[0]["why"], hist[0]["blockers"]),
                         ("head", "hold reviewed, cleared", ["B"]))
        self.assertTrue(hist[0]["at"])

    def test_both_fields_go_in_one_write(self):
        self.blocked("A", "B", status="blocked")
        mod = self.load_module()
        writes = []
        real = mod._write_json_atomic

        def spy(path, obj):
            writes.append((obj["status"], list(obj["blocked_by"])))
            real(path, obj)
        mod._write_json_atomic = spy
        rc = self.call(mod, "lift", "A", "--blocker", "B", *self.WHO)
        control("P3 one write: rc=%s writes=%s" % (rc, writes))
        self.assertEqual(rc, 0)
        self.assertEqual(writes, [("todo", [])])

    def test_lifting_one_of_two_keeps_the_other(self):
        self.blocked("A", "B", "C")
        rc, so, se = self.t("lift", "A", "--blocker", "B", *self.WHO)
        one = self.rec("A")
        ready_rc, ready = self.ready("A")
        control("P3 one of two: lift_rc=%s status=%s blocked_by=%s ready=%s unresolved=%s" % (
            rc, one["status"], one["blocked_by"], ready["ready"], ready["unresolved"]))
        self.assertEqual(rc, 0, se)
        self.assertEqual(one["blocked_by"], ["C"])
        self.assertEqual(one["status"], "blocked")   # todo here would be the disagreement again
        self.assertEqual((ready_rc, ready["unresolved"]), (4, ["C"]))
        self.assertEqual(json.loads(so)["ready"], ready["ready"])

        self.t("lift", "A", "--blocker", "C", *self.WHO)
        two = self.rec("A")
        control("P3 second lift: status=%s blocked_by=%s history=%s" % (
            two["status"], two["blocked_by"], len(two["lift_history"])))
        self.assertEqual((two["status"], two["blocked_by"], len(two["lift_history"])), ("todo", [], 2))
        self.assertEqual(self.ready("A")[0], 0)

    def test_remaining_blocker_already_done_reads_ready(self):
        self.checked_out("C")
        self.t("set-status", "C", "done", "--evidence", "unit test evidence")
        self.blocked("A", "B", "C", status="blocked")
        self.t("lift", "A", "--blocker", "B", *self.WHO)
        rec = self.rec("A")
        rc, ready = self.ready("A")
        control("P3 leftover blocker is done: status=%s blocked_by=%s ready=%s" % (
            rec["status"], rec["blocked_by"], ready["ready"]))
        self.assertEqual((rec["status"], rec["blocked_by"], rc), ("todo", ["C"], 0))

    def test_status_of_work_in_hand_is_left_alone(self):
        self.checked_out("B")
        self.t("set-status", "B", "done", "--evidence", "unit test evidence")
        self.blocked("A", "B")
        self.assertEqual(self.t("checkout", "A", "--owner", "w1")[0], 0)
        self.t("lift", "A", "--blocker", "B", *self.WHO)
        self.blocked("Q", "ZZ", status="backlog")
        self.t("lift", "Q", "--blocker", "ZZ", *self.WHO)
        got = (self.rec("A")["status"], self.rec("Q")["status"])
        control("P3 status kept: in_progress and backlog read %s" % (got,))
        self.assertEqual(got, ("in_progress", "backlog"))

    def test_dangling_blocker_can_be_lifted(self):
        self.blocked("A", "NO-SUCH-TASK")
        rc, _, se = self.t("lift", "A", "--blocker", "NO-SUCH-TASK", *self.WHO)
        control("P3 blocker with no task file: rc=%s blocked_by=%s" % (rc, self.rec("A")["blocked_by"]))
        self.assertEqual(rc, 0, se)
        self.assertEqual(self.rec("A")["blocked_by"], [])

    def test_refusals_change_nothing(self):
        self.blocked("A", "B")
        self.checked_out("D")
        self.t("set-status", "D", "done", "--evidence", "unit test evidence")
        before = self.raw("A")
        cases = [
            ("missing id", 3, ("lift", "NOPE", "--blocker", "B") + self.WHO),
            ("unknown blocker", 2, ("lift", "A", "--blocker", "X") + self.WHO),
            ("one unknown of two", 2, ("lift", "A", "--blocker", "B", "--blocker", "X") + self.WHO),
            ("empty --by", 2, ("lift", "A", "--blocker", "B", "--by", "", "--why", "w")),
            ("blank --by", 2, ("lift", "A", "--blocker", "B", "--by", "   ", "--why", "w")),
            ("empty --why", 2, ("lift", "A", "--blocker", "B", "--by", "w", "--why", "")),
            ("blank --why", 2, ("lift", "A", "--blocker", "B", "--by", "w", "--why", " \t ")),
            ("no --by", 2, ("lift", "A", "--blocker", "B", "--why", "w")),
            ("no --why", 2, ("lift", "A", "--blocker", "B", "--by", "w")),
            ("no --blocker", 2, ("lift", "A") + self.WHO),
            ("no id", 2, ("lift", "--blocker", "B") + self.WHO),
            ("traversal blocker", 2, ("lift", "A", "--blocker", "../x") + self.WHO),
        ]
        for name, want, argv in cases:
            rc = self.t(*argv)[0]
            control("P3 refused, %s: rc=%s want=%s record unchanged=%s" % (
                name, rc, want, self.raw("A") == before))
            self.assertEqual(rc, want, name)
            self.assertEqual(self.raw("A"), before, name)
        self.assertFalse(os.path.exists(self.path("NOPE")))

        self.blocked("T", "B")
        self.t("set-status", "T", "in_progress")
        self.t("set-status", "T", "cancelled")
        closed = self.raw("T")
        rc = self.t("lift", "T", "--blocker", "B", *self.WHO)[0]
        control("P3 refused, terminal task: rc=%s record unchanged=%s" % (rc, self.raw("T") == closed))
        self.assertEqual(rc, 9)
        self.assertEqual(self.raw("T"), closed)

    def test_concurrent_lifts_of_different_blockers_lose_nothing(self):
        n = 16
        for round_no in range(3):
            tid = "C%d" % round_no
            blockers = ["B%02d" % i for i in range(n)]
            self.blocked(tid, *blockers)
            res = self.burst([("lift", tid, "--blocker", b) + self.WHO for b in blockers])
            rec = self.rec(tid)
            lifted = sorted(b for h in rec["lift_history"] for b in h["blockers"])
            rcs = sorted(set(r[0] for r in res))
            leftover = [x for x in os.listdir(os.path.dirname(self.path(tid))) if ".wlock" in x]
            control("P3 concurrent, %d lifts: rcs=%s blocked_by=%s history=%d status=%s wlock_left=%s" % (
                n, rcs, rec["blocked_by"], len(rec["lift_history"]), rec["status"], leftover))
            self.assertEqual(rcs, [0], [r[2] for r in res if r[0]])
            self.assertEqual(rec["blocked_by"], [])
            self.assertEqual(lifted, blockers)
            self.assertEqual(rec["status"], "todo")
            self.assertEqual(leftover, [])

    def test_concurrent_lifts_of_the_same_blocker_have_one_winner(self):
        self.blocked("A", "B", "C")
        res = self.burst([("lift", "A", "--blocker", "B") + self.WHO for _ in range(8)])
        rcs = sorted(r[0] for r in res)
        rec = self.rec("A")
        control("P3 concurrent, same blocker 8 times: rcs=%s blocked_by=%s history=%d" % (
            rcs, rec["blocked_by"], len(rec["lift_history"])))
        self.assertEqual(rcs, [0] + [2] * 7)
        self.assertEqual(rec["blocked_by"], ["C"])
        self.assertEqual(len(rec["lift_history"]), 1)

    def test_concurrent_lift_and_set_status_keep_both(self):
        for i in range(6):
            tid = "R%d" % i
            self.checked_out("B" + tid)
            self.t("set-status", "B" + tid, "done", "--evidence", "unit test evidence")
            self.blocked(tid, "B" + tid, "C" + tid)
            self.t("lift", tid, "--blocker", "C" + tid, *self.WHO)
            self.assertEqual(self.t("checkout", tid, "--owner", "w1")[0], 0)
            self.assertEqual(self.rec(tid)["status"], "in_progress")
            res = self.burst([("lift", tid, "--blocker", "B" + tid) + self.WHO,
                              ("set-status", tid, "in_review")])
            rec = self.rec(tid)
            self.assertEqual([r[0] for r in res], [0, 0], [r[2] for r in res])
            self.assertEqual((rec["status"], rec["blocked_by"], len(rec["lift_history"])),
                             ("in_review", [], 2))
        control("P3 concurrent with set-status, 6 rounds: both writes kept every round")

    def test_lift_refuses_rather_than_run_without_the_lock(self):
        self.blocked("A", "B")
        before = self.raw("A")
        lock = os.path.join(os.path.dirname(self.path("A")), "A.wlock")
        os.mkdir(lock)
        self.addCleanup(lambda: os.path.isdir(lock) and os.rmdir(lock))
        t0 = time.time()
        rc, _, se = self.t("lift", "A", "--blocker", "B", *self.WHO)
        control("P3 wlock held elsewhere: rc=%s after %.1fs, record unchanged=%s, stderr=%s" % (
            rc, time.time() - t0, self.raw("A") == before, se.strip()))
        self.assertEqual(rc, 4)
        self.assertEqual(self.raw("A"), before)


if __name__ == "__main__":
    unittest.main(verbosity=2)
