#!/usr/bin/env python3
"""javis_org dispatch_task delivers a new department's first ticket (0.14.56 EMIT-FORK E3 + follow-up P-1/P-2).

At 9da2418d dispatch_task generated the first ticket with task-prompt and discarded it. E3 (c72262e5) sent it and
waited in the caller's turn for the delivery receipt. The follow-up:
  P-1  apply never waits: it sends and reports "queued"; `javis_org status` reports each first ticket's state and is
       ok only when every one is DELIVERED (the daemon's receipt in the department's delivery ledger);
  P-2  an idempotency key sha256(dept, task, scope, to) in the durable copy's name: an earlier item that is PENDING or
       DELIVERED is not re-sent; one that EXPIRED (or is in no list) is re-sent and the output says RESEND; UNKNOWN
       state is not re-sent.

Fakes: a `python3` shim stands in for task-prompt; a fake `cys` records argv and keeps a fake department queue
(pending / expired, previews like `cys queue list --json`) and writes the ledger rows the daemon writes on delivery
(queue row + receipt; shapes from src/bin/cysd/delivery.rs and governance.rs).
"""
import importlib.util
import json
import os
import shutil
import stat
import sys
import tempfile
import time
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(HERE)

FAKE_CYS = r'''#!/bin/sh
printf '%s\n' "$*" >> "$FAKE_LOG"
exec "$REAL_PY" "$FAKE_IMPL" "$@"
'''

FAKE_IMPL = r'''
import json, os, sys, time
a = sys.argv[1:]
mode, led, qf = os.environ.get("FAKE_MODE", ""), os.environ["FAKE_LEDGER"], os.environ["FAKE_QUEUE"]
q = json.load(open(qf)) if os.path.exists(qf) else {"pending": [], "expired": []}
if "queue" in a and "list" in a:
    if mode == "qfail":
        sys.exit(1)
    print(json.dumps([{"id": i, "preview": p} for i, p in q["expired" if "--expired" in a else "pending"]]))
    sys.exit(0)
if "send" in a:
    f = a[a.index("--file") + 1]
    prev = open(f, encoding="utf-8").read().replace("\n", " ")[:80]
    qid = "qfake.%d" % (len(q["pending"]) + len(q["expired"]) + 1)
    if mode == "deliver":
        with open(led, "a", encoding="utf-8") as fh:
            fh.write(json.dumps({"origin": "queue", "queue_entry_id": qid, "preview": prev[:64], "delivered_at": None}) + "\n")
            fh.write(json.dumps({"origin": "queue_receipt", "kind": "receipt", "queue_entry_id": qid, "delivered_at": time.time()}) + "\n")
    elif mode == "expire":
        q["expired"].append([qid, prev])
    else:
        q["pending"].append([qid, prev])
    json.dump(q, open(qf, "w"))
    print("QUEUED (depth 1)")
'''

FAKE_PY = r'''#!/bin/sh
case "$1" in
  *javis_orchestra.py) echo "[ticket] first project for the new dept"; echo "line two"; exit 0 ;;
esac
exec "$REAL_PY" "$@"
'''


def _exe(path, text):
    with open(path, "w", encoding="utf-8") as f:
        f.write(text)
    os.chmod(path, os.stat(path).st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)


@unittest.skipIf(os.name == "nt", "the fakes are POSIX sh shims; the Windows lanes do not run this file")
class DispatchDelivers(unittest.TestCase):
    def setUp(self):
        self.root = tempfile.mkdtemp(prefix="org-dispatch-")
        b = os.path.join(self.root, "bin")
        os.makedirs(b)
        _exe(os.path.join(b, "cys"), FAKE_CYS)
        _exe(os.path.join(b, "python3"), FAKE_PY)
        with open(os.path.join(self.root, "fake_impl.py"), "w", encoding="utf-8") as f:
            f.write(FAKE_IMPL)
        self.state = os.path.join(self.root, "state")
        os.makedirs(self.state)
        self.sock = os.path.join(self.root, "dept", "cys.sock")
        self.depts = os.path.join(self.root, "depts.json")
        with open(self.depts, "w", encoding="utf-8") as f:
            json.dump({"depts": {"dept-9": {"mission_key": "future-research", "socket": self.sock}}}, f)
        self.log = os.path.join(self.root, "cys.log")
        self.saved = dict(os.environ)
        os.environ.update({"PATH": b + os.pathsep + self.saved.get("PATH", ""), "REAL_PY": sys.executable,
                           "FAKE_LOG": self.log, "FAKE_IMPL": os.path.join(self.root, "fake_impl.py"),
                           "FAKE_QUEUE": os.path.join(self.root, "queue.json"), "CYS_STATE_DIR": self.state,
                           "CYS_DEPTS_JSON": self.depts, "JAVIS_ORG_DISPATCH_WAIT_S": "3"})
        sys.path.insert(0, BIN)
        import javis_lane
        os.environ["FAKE_LEDGER"] = javis_lane.lane_state_path("delivery", self.sock)
        spec = importlib.util.spec_from_file_location("javis_org_under_test", os.path.join(BIN, "javis_org.py"))
        self.org = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.org)
        self.task = {"dept": "future-research", "task": "T1", "scope": "s", "to": "worker"}

    def tearDown(self):
        os.environ.clear()
        os.environ.update(self.saved)
        shutil.rmtree(self.root, ignore_errors=True)

    def _sends(self):
        if not os.path.exists(self.log):
            return []
        return [l for l in open(self.log, encoding="utf-8").read().splitlines() if l.startswith("--socket") and " send " in l]

    # (a) P-1: the caller's turn never sits on a delivery wait
    def test_apply_does_not_wait_for_a_slow_receiver(self):
        os.environ["FAKE_MODE"] = "pending"
        t0 = time.monotonic()
        rc, out = self.org.dispatch_task(self.task)
        dt = time.monotonic() - t0
        self.assertLess(dt, 1.5, "dispatch waited %.1f s for the receiver" % dt)
        self.assertEqual(rc, 0, out)
        self.assertIn("queued", out)
        self.assertEqual(len(self._sends()), 1)
        s = self._sends()[0]
        self.assertIn("--socket %s" % self.sock, s)
        self.assertIn("--queued", s)
        self.assertIn("--to worker", s)

    # (b) P-2: the documented skill re-run (apply once more) must not deliver the first ticket twice
    def test_rerun_while_pending_sends_once(self):
        os.environ["FAKE_MODE"] = "pending"
        self.org.dispatch_task(self.task)
        rc, out = self.org.dispatch_task(self.task)
        self.assertEqual(len(self._sends()), 1, "the re-run queued the first ticket a second time")
        self.assertEqual(rc, 0, out)
        self.assertIn("pending (existing item, not resent)", out)

    def test_rerun_after_delivery_sends_once(self):
        os.environ["FAKE_MODE"] = "deliver"
        self.org.dispatch_task(self.task)
        rc, out = self.org.dispatch_task(self.task)
        self.assertEqual(len(self._sends()), 1)
        self.assertIn("delivered (existing item, not resent)", out)

    def test_rerun_after_expiry_resends_and_says_so(self):
        os.environ["FAKE_MODE"] = "expire"
        self.org.dispatch_task(self.task)
        rc, out = self.org.dispatch_task(self.task)
        self.assertEqual(len(self._sends()), 2, "an expired first ticket must be re-sendable")
        self.assertEqual(rc, 0, out)
        self.assertIn("RESEND of expired item", out)

    def test_unknown_state_is_not_resent(self):
        os.environ["FAKE_MODE"] = "pending"
        self.org.dispatch_task(self.task)
        os.environ["FAKE_MODE"] = "qfail"
        rc, out = self.org.dispatch_task(self.task)
        self.assertEqual(len(self._sends()), 1)
        self.assertNotEqual(rc, 0, out)
        self.assertIn("unknown state", out)

    # P-1: the success claim lives in status
    def test_status_reports_first_ticket_state(self):
        os.environ["FAKE_MODE"] = "pending"
        self.org.dispatch_task(self.task)
        self.assertEqual(self.org.first_ticket_state_of(self.task), "pending")
        os.environ["FAKE_MODE"] = "deliver"
        t2 = dict(self.task, task="T2")
        self.org.dispatch_task(t2)
        self.assertEqual(self.org.first_ticket_state_of(t2), "delivered")
        self.assertEqual(self.org.first_ticket_state_of(dict(self.task, task="never-sent")), "absent")

    def test_unresolved_department_sends_nothing(self):
        os.environ["FAKE_MODE"] = "deliver"
        rc, out = self.org.dispatch_task(dict(self.task, dept="no-such-mission"))
        self.assertNotEqual(rc, 0)
        self.assertIn("dept_unresolved", out)
        self.assertEqual(self._sends(), [])


if __name__ == "__main__":
    unittest.main()
