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
import ast
import contextlib
import glob
import importlib.util
import io
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
BIN = os.path.dirname(HERE)
TASK = os.path.join(BIN, "javis_task.py")
PY = sys.executable
PASS_ROW = "settle-gate-pass"
REMEDY = ("wait out the 30 s stale window · try ONCE · if it still returns 7, report a lock anomaly · "
          "never --settled-override · never immediate repeated retry")
# A release that removes the ownership lock, then dies at its record write.
RELEASE_DIES = """
import importlib.util, sys
spec = importlib.util.spec_from_file_location("jt", sys.argv[1])
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)
def boom(path, obj, **kw):
    raise OSError("injected")
m._write_json_atomic = boom
sys.exit(m.main(["release", sys.argv[2], "--owner", sys.argv[3]]))
"""
OWNER_CLAIM = ("살아있는", "소유자", "게이트", "live owner", "gate")
# Runs one real verb, stopping just before its record write until the test says go. Nothing is mocked but the pause.
HOLDER = """
import importlib.util, os, sys, time
spec = importlib.util.spec_from_file_location("jt", sys.argv[1])
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)
real = m._write_json_atomic
def barrier(path, obj, **kw):
    open(sys.argv[2], "w").close()
    while not os.path.exists(sys.argv[3]):
        time.sleep(0.01)
    real(path, obj, **kw)
m._write_json_atomic = barrier
sys.exit(m.main(sys.argv[4:]))
"""


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

        def spy(tid, kind, **fields):
            seen.append((kind, self.rec(tid)["status"], self.rec(tid)["owner"]))
            real(tid, kind, **fields)
        mod._append_audit = spy
        rc = self.call(mod, "set-status", "A", "done", *self.EV)
        control("P2 durable: rc=%s at the moment of the audit call, disk read %s" % (rc, seen))
        self.assertEqual(rc, 0)
        self.assertEqual(seen, [(PASS_ROW, "done", None)])

    def test_failed_write_gives_no_row(self):
        self.checked_out("A")
        mod = self.load_module()

        def boom(path, obj, **kw):
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

        def spy(path, obj, **kw):
            writes.append((obj["status"], list(obj["blocked_by"])))
            real(path, obj, **kw)
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
        self.assertEqual(rc, 7)
        self.assertEqual(self.raw("A"), before)
        self.assertIn(REMEDY, se)
        self.assertEqual([w for w in OWNER_CLAIM if w in se.replace("--settled-override", "")], [])


def _is_task_write(node):
    return isinstance(node, ast.Call) and getattr(node.func, "id", "") == "_write_task"


def raw_task_writes(path):
    """Functions that reach the record file without _write_task and its check."""
    with open(path, encoding="utf-8") as f:
        tree = ast.parse(f.read())
    out = []
    for fn in ast.walk(tree):
        if isinstance(fn, ast.FunctionDef) and "test" not in fn.name:
            out += [fn.name for n in ast.walk(fn) if isinstance(n, ast.Call)
                    and getattr(n.func, "id", "") == "_write_json_atomic" and n.args
                    and getattr(getattr(n.args[0], "func", None), "id", "") == "_task_path"]
    return out


def writer_census(path):
    """(functions that write a task record, those writes that sit outside `with _wlock`), found by predicate."""
    with open(path, encoding="utf-8") as f:
        tree = ast.parse(f.read())
    writers, unlocked = set(), []
    for fn in ast.walk(tree):
        if not isinstance(fn, ast.FunctionDef) or "test" in fn.name:
            continue
        writes = [n for n in ast.walk(fn) if _is_task_write(n)]
        if not writes:
            continue
        writers.add(fn.name)
        guarded = set()
        for w in ast.walk(fn):
            if isinstance(w, ast.With) and any(getattr(getattr(i.context_expr, "func", None), "id", "") == "_wlock"
                                               for i in w.items):
                guarded.update(id(n) for n in ast.walk(w))
        unlocked += [fn.name for n in writes if id(n) not in guarded]
    return writers, unlocked


def foreign_writers(bin_dir):
    """Modules other than javis_task that open a task record for writing."""
    out = []
    for path in sorted(glob.glob(os.path.join(bin_dir, "*.py"))):
        if os.path.basename(path) == "javis_task.py":
            continue
        with open(path, encoding="utf-8") as f:
            tree = ast.parse(f.read())
        for n in ast.walk(tree):
            if not isinstance(n, ast.Call) or not n.args:
                continue
            name = getattr(n.func, "id", getattr(n.func, "attr", ""))
            if name != "open" and "write" not in name:
                continue
            text = ast.unparse(n.args[0])
            mode = ast.unparse(n.args[1]) if name == "open" and len(n.args) > 1 else ""
            if "TASKS_DIR" in text and "'%s.json'" in text and (name != "open" or set(mode) & set("wax+")):
                out.append((os.path.basename(path), n.lineno))
    return out


class Barriers(Base):
    WHO = ("--by", "head", "--why", "hold reviewed, cleared")

    def holder(self, *argv, name="h"):
        at, go = os.path.join(self.tmp, name + "-at-write"), os.path.join(self.tmp, name + "-go")
        self.addCleanup(lambda: open(go, "w").close())
        p = subprocess.Popen([PY, "-B", "-c", HOLDER, TASK, at, go] + list(argv), stdout=subprocess.PIPE,
                             stderr=subprocess.PIPE, text=True, env=self.env())
        deadline = time.time() + 30
        while not os.path.exists(at):
            self.assertLess(time.time(), deadline, "the holder never reached its write")  # wallclock: guard 30 s only bounds a hang; the barrier file decides
            time.sleep(0.01)  # wallclock: guard polling a barrier file, not a duration the test rests on
        return p, go


class WriterProtocol(Barriers):
    """Finding 1: every writer of a task record refuses past its wait. No unlocked path is left."""
    PEERS = {
        "cmd_create": (("create", "RACE", "--id", "RACE"), {}),
        "cmd_checkout": (("checkout", "RACE", "--owner", "w2"), {"JAVIS_TASK_LIVENESS": "dead"}),
        "cmd_release": (("release", "RACE", "--owner", "w1"), {}),
        "cmd_set_status": (("set-status", "RACE", "in_review"), {}),
        "cmd_set_verify_spec": (("set-verify-spec", "RACE", "--demote-calibrated"), {}),
        "cmd_lift": (("lift", "RACE", "--blocker", "B", "--by", "peer", "--why", "second"), {}),
    }

    def test_writers_are_derived_and_every_write_is_locked(self):
        writers, unlocked = writer_census(TASK)
        control("P3R writers by predicate: %s, writes outside the lock: %s, other modules writing a record: %s" % (
            sorted(writers), unlocked, foreign_writers(BIN)))
        self.assertEqual(unlocked, [])
        self.assertEqual(foreign_writers(BIN), [])
        self.assertEqual(raw_task_writes(TASK), ["_write_task"])
        self.assertEqual(writers, set(self.PEERS), "a new writer needs a row in the barrier test below")

    def test_positive_control_an_unlocked_writer_and_a_foreign_writer_are_caught(self):
        d = tempfile.mkdtemp(prefix="writers-")
        self.addCleanup(shutil.rmtree, d, True)
        tool = os.path.join(d, "javis_new_tool.py")
        with open(tool, "w", encoding="utf-8") as f:
            f.write("def cmd_new(a):\n    _write_task(a.id, {})\n"
                    "def poke(i):\n    open(os.path.join(TASKS_DIR, '%s.json' % i), 'w')\n")
        got = (writer_census(tool)[1], foreign_writers(d))
        control("P3R predicate control: unlocked=%s foreign=%s" % got)
        self.assertEqual(got[0], ["cmd_new"])
        self.assertEqual(got[1], [("javis_new_tool.py", 4)])

    def test_every_writer_past_its_wait_refuses_and_lift_keeps_its_write(self):
        self.checked_out("B")
        self.t("set-status", "B", "done", "--evidence", "unit test evidence")
        self.t("create", "RACE", "--id", "RACE", "--blocked-by", "B")
        self.assertEqual(self.t("checkout", "RACE", "--owner", "w1")[0], 0)
        lock = os.path.join(self.root, "_round", "tasks", "RACE.lock", "owner.json")
        with open(lock, "rb") as f:
            lock_before = f.read()
        before = self.raw("RACE")
        holder, go = self.holder("lift", "RACE", "--blocker", "B", *self.WHO)
        procs = {}
        for name, (argv, extra) in sorted(self.PEERS.items()):
            procs[name] = subprocess.Popen([PY, "-B", TASK] + list(argv), stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                           text=True, env=self.env(**extra))
        peers = {name: (p.communicate(timeout=60)[1], p.returncode) for name, p in procs.items()}
        with open(lock, "rb") as f:
            lock_mid = f.read()
        same = self.raw("RACE") == before and lock_mid == lock_before
        control("P3R peers past their wait while lift holds the lock: %s, record and ownership lock unchanged=%s" % (
            {k: v[1] for k, v in peers.items()}, same))
        self.assertEqual({k: v[1] for k, v in peers.items()}, {k: 7 for k in self.PEERS}, peers)
        self.assertTrue(same)
        for name, (err, _rc) in peers.items():
            self.assertIn(REMEDY, err, name)
            self.assertNotIn("무락", err, name)
        open(go, "w").close()
        so, se = holder.communicate(timeout=60)
        rec = self.rec("RACE")
        control("P3R lift resumes: rc=%s status=%s blocked_by=%s history=%d" % (
            holder.returncode, rec["status"], rec["blocked_by"], len(rec["lift_history"])))
        self.assertEqual(holder.returncode, 0, se)
        self.assertEqual((rec["status"], rec["blocked_by"], rec["owner"]), ("in_progress", [], "w1"))
        self.assertEqual(len(rec["lift_history"]), 1)


class StaleReclaim(Barriers):
    """Round 3: a holder the 30 s reclaim took the lock from must not write, and must not remove the next holder's lock."""

    def setUp(self):
        super().setUp()
        self.checked_out("B")
        self.t("set-status", "B", "done", "--evidence", "unit test evidence")
        self.t("create", "RACE", "--id", "RACE", "--blocked-by", "B")
        self.assertEqual(self.t("checkout", "RACE", "--owner", "w1")[0], 0)
        self.lock = os.path.join(self.root, "_round", "tasks", "RACE.wlock")

    def age_the_lock(self):
        old = time.time() - 31
        os.utime(self.lock, (old, old))

    def token(self):
        with open(os.path.join(self.lock, "token"), encoding="ascii") as f:
            return f.read()

    def test_a_reclaimed_holder_refuses_and_writes_nothing(self):
        holder, go = self.holder("lift", "RACE", "--blocker", "B", *self.WHO)
        self.age_the_lock()
        rc = self.t("set-status", "RACE", "in_review")[0]
        after_peer = self.raw("RACE")
        open(go, "w").close()
        _, err = holder.communicate(timeout=60)
        rec = self.rec("RACE")
        control("P3R3 lift reclaimed at 31 s, peer rc=%s, lift resumes rc=%s, record unchanged=%s, blocked_by=%s, "
                "history=%d, status=%s" % (rc, holder.returncode, self.raw("RACE") == after_peer, rec["blocked_by"],
                                           len(rec.get("lift_history", [])), rec["status"]))
        self.assertEqual((rc, holder.returncode), (0, 7), err)
        self.assertIn(REMEDY, err)
        self.assertEqual(self.raw("RACE"), after_peer)
        self.assertEqual((rec["status"], rec["blocked_by"]), ("in_review", ["B"]))
        self.assertFalse(os.path.exists(self.lock))
        tasks = os.path.join(self.root, "_round", "tasks")
        self.assertEqual(glob.glob(os.path.join(tasks, "*.tmp.*")) + glob.glob(os.path.join(tasks, "*.stale.*")), [])

    def test_an_old_holder_cannot_remove_the_new_instance(self):
        old, old_go = self.holder("lift", "RACE", "--blocker", "B", *self.WHO, name="old")
        first = self.token()
        self.age_the_lock()
        new, new_go = self.holder("set-status", "RACE", "in_review", name="new")
        second = self.token()
        open(old_go, "w").close()
        old.communicate(timeout=60)
        kept = os.path.isdir(self.lock) and self.token() == second
        control("P3R3 old holder exits while the new one holds: old rc=%s, tokens differ=%s, new instance intact=%s" % (
            old.returncode, first != second, kept))
        self.assertEqual(old.returncode, 7)
        self.assertNotEqual(first, second)
        self.assertTrue(kept)
        open(new_go, "w").close()
        new.communicate(timeout=60)
        self.assertEqual(new.returncode, 0)
        self.assertEqual(self.rec("RACE")["status"], "in_review")
        self.assertFalse(os.path.exists(self.lock))

    def test_a_lock_with_no_token_is_still_reclaimed(self):
        os.mkdir(self.lock)
        self.age_the_lock()
        rc = self.t("set-status", "RACE", "in_review")[0]
        control("P3R3 aged lock from a holder that died before its token: rc=%s" % rc)
        self.assertEqual(rc, 0)
        self.assertFalse(os.path.exists(self.lock))


class ClaimBinding(Base):
    """Finding 2: done completes only the claim its gates checked."""
    EV = ("--evidence", "unit test evidence")

    def lock_owner(self, tid):
        with open(os.path.join(self.root, "_round", "tasks", tid + ".lock", "owner.json"), encoding="utf-8") as f:
            return json.load(f)["owner_id"]

    def after(self, mod, name, *steps):
        """Run real CLI steps once, right after the real function `name` returns."""
        real = getattr(mod, name)
        done = []

        def hooked(*a, **k):
            res = real(*a, **k)
            if not done:
                done.append(1)
                for step in steps:
                    self.assertEqual(self.t(*step)[0], 0, step)
            return res
        setattr(mod, name, hooked)

    def test_gate_passed_for_w1_then_w2_took_it_refuses(self):
        self.checked_out("A")
        mod = self.load_module()
        self.after(mod, "_completion_settled", ("release", "A", "--owner", "w1"), ("checkout", "A", "--owner", "w2"))
        rc = self.call(mod, "set-status", "A", "done", *self.EV)
        rec = self.rec("A")
        fresh = self.load_module()
        with mock.patch.dict(os.environ, self.env(JAVIS_SETTLE_PROBE="0:1"), clear=True):
            w2 = fresh._completion_settled("w2")
        control("P2R G6 passed for w1, then w2 took the task: rc=%s status=%s owner=%s gen=%s lock=%s rows=%s" % (
            rc, rec["status"], rec["owner"], rec["claim_gen"], self.lock_owner("A"), self.rows("A")))
        self.assertEqual(rc, 9)
        self.assertEqual((rec["status"], rec["owner"], rec["claim_gen"]), ("in_progress", "w2", 2))
        self.assertEqual(self.lock_owner("A"), "w2")
        self.assertEqual(self.rows("A"), [])
        control("P2R control, the same gate asked about w2: %s" % (w2,))
        self.assertIs(w2[0], False)

    def test_same_name_aba_refuses(self):
        self.checked_out("A")
        mod = self.load_module()
        self.after(mod, "_completion_settled", ("release", "A", "--owner", "w1"), ("checkout", "A", "--owner", "w1"))
        rc = self.call(mod, "set-status", "A", "done", *self.EV)
        rec = self.rec("A")
        control("P2R same-name ABA, w1 released and rechecked out: rc=%s status=%s owner=%s gen=%s rows=%s" % (
            rc, rec["status"], rec["owner"], rec["claim_gen"], self.rows("A")))
        self.assertEqual(rc, 9)
        self.assertEqual((rec["status"], rec["owner"], rec["claim_gen"]), ("in_progress", "w1", 2))
        self.assertEqual(self.rows("A"), [])

    def test_released_with_no_new_owner_refuses_without_claiming_a_live_owner(self):
        self.checked_out("A")
        mod = self.load_module()
        self.after(mod, "_completion_settled", ("release", "A", "--owner", "w1"))
        err = io.StringIO()
        with contextlib.redirect_stderr(err):
            rc = self.call(mod, "set-status", "A", "done", *self.EV)
        rec = self.rec("A")
        control("P2R w1 released, nobody holds it: rc=%s status=%s owner=%s stderr=%s" % (
            rc, rec["status"], rec["owner"], err.getvalue().strip()))
        self.assertEqual((rc, rec["status"], rec["owner"]), (9, "todo", None))
        self.assertNotIn("살아있는", err.getvalue())

    def lock_path(self, tid):
        return os.path.join(self.root, "_round", "tasks", tid + ".lock")

    def test_a_fresh_lock_after_an_interrupted_release_is_a_new_claim(self):
        """The review's schedule: the record still says w1#1, the lock is gone, w1 checks out again."""
        self.checked_out("A")
        mod = self.load_module()
        real = mod._completion_settled

        def hooked(owner):
            res = real(owner)
            r = subprocess.run([PY, "-B", "-c", RELEASE_DIES, TASK, "A", "w1"], capture_output=True, text=True,
                               env=self.env())
            self.assertNotEqual(r.returncode, 0)
            self.assertFalse(os.path.exists(self.lock_path("A")))
            self.assertEqual(self.rec("A")["claim_gen"], 1)
            self.assertEqual(self.t("checkout", "A", "--owner", "w1")[0], 0)
            return res
        mod._completion_settled = hooked
        rc = self.call(mod, "set-status", "A", "done", *self.EV)
        rec = self.rec("A")
        control("P2R3 release died after removing the lock, w1 checked out afresh: rc=%s status=%s gen=%s lock=%s rows=%s" % (
            rc, rec["status"], rec["claim_gen"], self.lock_owner("A"), self.rows("A")))
        self.assertEqual((rc, rec["status"], rec["claim_gen"]), (9, "in_progress", 2))
        self.assertEqual(self.lock_owner("A"), "w1")
        self.assertEqual(self.rows("A"), [])

    def test_a_record_with_an_owner_and_no_lock_gets_a_new_generation(self):
        self.checked_out("A")
        shutil.rmtree(self.lock_path("A"))
        self.assertEqual(self.t("checkout", "A", "--owner", "w1")[0], 0)
        control("P2R3 owner in the record, no lock, checkout: gen=%s" % self.rec("A")["claim_gen"])
        self.assertEqual(self.rec("A")["claim_gen"], 2)

    def test_terminal_status_and_owner_land_in_one_write(self):
        self.checked_out("A")
        mod = self.load_module()
        writes = []
        real = mod._write_json_atomic

        def spy(path, obj, **kw):
            if path.endswith("/A.json"):
                writes.append((obj["status"], obj["owner"]))
            real(path, obj, **kw)
        mod._write_json_atomic = spy
        rc = self.call(mod, "set-status", "A", "done", *self.EV)
        control("P2R3 done: rc=%s record writes=%s" % (rc, writes))
        self.assertEqual((rc, writes), (0, [("done", None)]))

    def test_reentrant_checkout_is_the_same_claim_and_still_passes(self):
        self.checked_out("A")
        mod = self.load_module()
        self.after(mod, "_completion_settled", ("checkout", "A", "--owner", "w1"))
        rc = self.call(mod, "set-status", "A", "done", *self.EV)
        rows = self.rows("A")
        control("P2R same owner re-enters checkout, no new claim: rc=%s status=%s row=%s" % (
            rc, self.rec("A")["status"], [(r["audit"], r["owner"], r["claim_gen"]) for r in rows]))
        self.assertEqual(rc, 0)
        self.assertEqual([(r["audit"], r["owner"], r["claim_gen"]) for r in rows], [(PASS_ROW, "w1", 1)])

    def test_a_task_claimed_after_an_owner_unset_read_refuses(self):
        self.t("create", "A", "--id", "A")
        self.t("set-status", "A", "in_progress")
        mod = self.load_module()
        self.after(mod, "_radio_done_gate", ("checkout", "A", "--owner", "w2"))
        rc = self.call(mod, "set-status", "A", "done", *self.EV)
        rec = self.rec("A")
        control("P2R no owner when read, w2 claimed before commit: rc=%s status=%s owner=%s lock=%s" % (
            rc, rec["status"], rec["owner"], self.lock_owner("A")))
        self.assertEqual((rc, rec["status"], rec["owner"], self.lock_owner("A")), (9, "in_progress", "w2", "w2"))

    def test_an_override_is_bound_too(self):
        self.checked_out("A")
        mod = self.load_module()
        self.after(mod, "_radio_done_gate", ("release", "A", "--owner", "w1"), ("checkout", "A", "--owner", "w2"))
        rc = self.call(mod, "set-status", "A", "done", "--settled-override", "owner declares", *self.EV)
        rec = self.rec("A")
        control("P2R override, then w2 took it: rc=%s status=%s settle_overrides=%d" % (
            rc, rec["status"], len(rec.get("settle_overrides", []))))
        self.assertEqual((rc, rec["status"], rec.get("settle_overrides", [])), (9, "in_progress", []))

    def test_a_record_from_before_claim_gen_still_completes(self):
        self.checked_out("A")
        rec = self.rec("A")
        del rec["claim_gen"]
        with open(self.path("A"), "w", encoding="utf-8") as f:
            json.dump(rec, f)
        rc, _, se = self.t("set-status", "A", "done", *self.EV)
        rows = self.rows("A")
        control("P2R record with no claim_gen: rc=%s row gen=%s" % (rc, [r["claim_gen"] for r in rows]))
        self.assertEqual(rc, 0, se)
        self.assertEqual([r["claim_gen"] for r in rows], [None])

    def test_checkout_gives_each_claim_a_new_generation(self):
        self.checked_out("A")
        one = self.rec("A")["claim_gen"]
        self.t("checkout", "A", "--owner", "w1")
        same = self.rec("A")["claim_gen"]
        self.t("release", "A", "--owner", "w1")
        self.t("checkout", "A", "--owner", "w1")
        two = self.rec("A")["claim_gen"]
        control("P2R claim_gen: first=%s re-entry=%s after release and recheckout=%s" % (one, same, two))
        self.assertEqual((one, same, two), (1, 1, 2))


class CheckoutUnderLock(Base):
    """Round 3: eligibility comes from the locked snapshot, before ownership is claimed."""
    EV = ("--evidence", "unit test evidence")

    def lock_path(self, tid):
        return os.path.join(self.root, "_round", "tasks", tid + ".lock")

    def between_reads(self, mod, *steps):
        """Run after checkout's first checks and before it enters the wlock."""
        real = mod._verify_gate_check

        def hooked(task):
            for step in steps:
                if callable(step):
                    step()
                else:
                    self.assertEqual(self.t(*step)[0], 0, step)
            return real(task)
        mod._verify_gate_check = hooked

    def test_a_task_that_went_done_between_the_reads_is_not_reopened(self):
        self.t("create", "A", "--id", "A")
        mod = self.load_module()
        self.between_reads(mod, ("set-status", "A", "in_progress"), ("set-status", "A", "done", *self.EV))
        rc = self.call(mod, "checkout", "A", "--owner", "late")
        rec = self.rec("A")
        control("P3R3 went done between checkout's reads: rc=%s status=%s owner=%s lock=%s" % (
            rc, rec["status"], rec["owner"], os.path.exists(self.lock_path("A"))))
        self.assertEqual((rc, rec["status"], rec["owner"]), (9, "done", None))
        self.assertFalse(os.path.exists(self.lock_path("A")))

    def test_a_blocker_that_came_back_between_the_reads_refuses(self):
        self.t("create", "B", "--id", "B")
        self.t("checkout", "B", "--owner", "w1")
        self.t("set-status", "B", "done", *self.EV)
        self.t("create", "A", "--id", "A", "--blocked-by", "B")
        before = self.raw("A")
        mod = self.load_module()
        self.between_reads(mod, lambda: os.remove(self.path("B")))
        rc = self.call(mod, "checkout", "A", "--owner", "late")
        control("P3R3 blocker unresolved at the locked read: rc=%s record unchanged=%s lock=%s" % (
            rc, self.raw("A") == before, os.path.exists(self.lock_path("A"))))
        self.assertEqual(rc, 4)
        self.assertEqual(self.raw("A"), before)
        self.assertFalse(os.path.exists(self.lock_path("A")))

    def test_a_lost_lock_stops_ownership_changing(self):
        self.t("create", "A", "--id", "A")
        mod = self.load_module()

        def lost(task_id):
            raise mod._WlockBusy(task_id)
        mod._wlock_confirm = lost
        co = self.call(mod, "checkout", "A", "--owner", "w1")
        made = os.path.exists(self.lock_path("A"))
        self.t("checkout", "A", "--owner", "w1")
        with open(os.path.join(self.lock_path("A"), "owner.json"), "rb") as f:
            before = f.read()
        rel = self.call(mod, "release", "A", "--owner", "w1")
        with open(os.path.join(self.lock_path("A"), "owner.json"), "rb") as f:
            kept = f.read() == before
        control("P3R3 lock lost before ownership moves: checkout rc=%s lock made=%s, release rc=%s lock kept=%s" % (
            co, made, rel, kept))
        self.assertEqual((co, made, rel, kept), (7, False, 7, True))

    def test_a_refused_write_gives_a_fresh_ownership_lock_back(self):
        self.t("create", "A", "--id", "A")
        before = self.raw("A")
        mod = self.load_module()

        def lost(task_id, task):
            raise mod._WlockBusy(task_id)
        mod._write_task = lost
        rc = self.call(mod, "checkout", "A", "--owner", "w1")
        control("P3R3 checkout lost the lock at its write: rc=%s record unchanged=%s ownership lock=%s" % (
            rc, self.raw("A") == before, os.path.exists(self.lock_path("A"))))
        self.assertEqual(rc, 7)
        self.assertEqual(self.raw("A"), before)
        self.assertFalse(os.path.exists(self.lock_path("A")))


def has_remedy(text):
    return REMEDY in text


class Remedy(unittest.TestCase):
    """The code's contract is its remedy. It must stay in the readers' copy, verbatim."""
    PACK = os.path.dirname(BIN)

    def read(self, rel):
        with open(os.path.join(self.PACK, rel), encoding="utf-8") as f:
            return f.read()

    def test_the_readers_copies_carry_the_remedy_verbatim(self):
        docs = ("directives/WORKER_DIRECTIVE.md", "round/TOOL_RESULT_VOCAB.md", "skills/vibecoding/SKILL.md",
                "bin/javis_task.py")
        got = {d: has_remedy(self.read(d)) for d in docs}
        control("P3R remedy verbatim: %s" % got)
        self.assertEqual(set(got.values()), {True}, got)

    def test_positive_control_a_reworded_remedy_fails(self):
        self.assertFalse(has_remedy(self.read("directives/WORKER_DIRECTIVE.md").replace("try ONCE", "retry")))


if __name__ == "__main__":
    unittest.main(verbosity=2)
