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
from unittest import mock

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
if "status" in a and "--json" in a:
    print(json.dumps({"surfaces": [{"role": "worker", "agent_alive": True, "idle_secs": 1}]}))
    sys.exit(0)
if "send" in a:
    f = a[a.index("--file") + 1]
    prev = open(f, encoding="utf-8").read().replace("\n", " ")[:80]
    qid = "qfake.%d" % (len(q["pending"]) + len(q["expired"]) + 1)
    now = time.time()
    # the queued text is the file text minus one trailing newline (measured on a real cysd: EMIT-FORK-R2 MEASURE-digest.md)
    txt = open(f, encoding="utf-8").read()
    txt = txt[:-1] if txt.endswith("\n") else txt
    if mode == "digest_tail":
        # the ticket is the NON-head item of a 2-item queue digest (governance.rs: head id on the row, the rest only in
        # digest_parts with sha256 = digest_text(item text); the receipt carries merged_ids)
        sys.path.insert(0, os.environ["FAKE_BIN"]); import javis_mission
        head = "qhead.%d" % (len(q["pending"]) + 1)
        with open(led, "a", encoding="utf-8") as fh:
            fh.write(json.dumps({"origin": "queue", "queue_entry_id": head, "ts_epoch": now, "delivered_at": None,
                                 "preview": "[큐 다이제스트 2건 · 발신 cli:send] [1/2] another sender item",
                                 "digest_items": 2, "digest_parts": [{"queue_entry_id": head, "sha256": "0" * 64},
                                 {"queue_entry_id": qid, "sha256": javis_mission.delivery_digest(txt)}]}) + "\n")
            fh.write(json.dumps({"origin": "queue_receipt", "kind": "receipt", "queue_entry_id": head, "ts_epoch": now,
                                 "merged_ids": [head, qid], "body_sha256": "1" * 64, "delivered_at": now}) + "\n")
    elif mode == "rotated_in_dot1":
        with open(led + ".1", "a", encoding="utf-8") as fh:
            fh.write(json.dumps({"origin": "queue", "queue_entry_id": qid, "preview": prev[:64], "ts_epoch": now, "delivered_at": None}) + "\n")
            fh.write(json.dumps({"origin": "queue_receipt", "kind": "receipt", "queue_entry_id": qid, "ts_epoch": now, "delivered_at": now}) + "\n")
        with open(led, "a", encoding="utf-8") as fh:
            fh.write(json.dumps({"origin": "boot", "ts_epoch": now + 1}) + "\n")
    elif mode == "rotated_away":
        # delivered, then the ledger rotated twice: neither generation still covers the send
        with open(led + ".1", "w", encoding="utf-8") as fh:
            fh.write(json.dumps({"origin": "boot", "ts_epoch": now + 100}) + "\n")
        with open(led, "w", encoding="utf-8") as fh:
            fh.write(json.dumps({"origin": "boot", "ts_epoch": now + 200}) + "\n")
    elif mode == "send_then_die":
        q["pending"].append([qid, prev]); json.dump(q, open(qf, "w"))
        sys.exit(137)   # the daemon accepted it, then the client died before the caller saw rc 0
    if mode in ("digest_tail", "rotated_in_dot1", "rotated_away"):
        json.dump(q, open(qf, "w")); print("QUEUED (depth 1)"); sys.exit(0)
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
                           "CYS_DEPTS_JSON": self.depts, "JAVIS_ORG_DISPATCH_WAIT_S": "3", "FAKE_BIN": BIN})
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

    # (a) P-1: the caller's turn never sits on a delivery wait. Event check, no clock: the receiver never delivers
    # (pending), and after the send dispatch_task makes no further observation - no cys call (queue list, status),
    # no delivery-ledger or queue read (the module's readers, or any open() in the module), no sleep. Waiting for the
    # receipt needs at least one of them. One ordered stream: the fake cys appends its argv to FAKE_LOG, the wrappers
    # below append "PY:<name>" to the same file. Not seen: a fixed busy-wait that observes nothing.
    def test_apply_does_not_wait_for_a_slow_receiver(self):
        os.environ["FAKE_MODE"] = "pending"

        def mark(name):
            with open(self.log, "a", encoding="utf-8") as f:
                f.write("PY:%s\n" % name)

        for name in ("_ticket_receipt", "_ledger_rows", "_ledger_size", "_queue_has", "_ticket_state"):
            def wrapped(*a, _real=getattr(self.org, name), _name=name, **k):
                mark(_name)
                return _real(*a, **k)
            setattr(self.org, name, wrapped)

        def recorded_open(file, *a, **k):   # module-scoped: shadows the builtin for javis_org only
            mark("open:%s" % os.path.basename(str(file)))
            return open(file, *a, **k)
        self.org.open = recorded_open
        with mock.patch.object(self.org.time, "sleep", lambda _s: mark("sleep")):
            rc, out = self.org.dispatch_task(self.task)
        with open(self.log, encoding="utf-8") as f:
            events = f.read().splitlines()
        sent = [i for i, e in enumerate(events) if e.startswith("--socket") and " send " in e]
        self.assertEqual(len(sent), 1, events)
        self.assertEqual(events[sent[0] + 1:], [], "dispatch observed delivery after the send (a wait)")
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

    # ── R2 (r1 review gemini1 M1-M3, m1; gemini5 m1, m2) ────────────────────────────────────────────────────────
    def _keyed_copies(self):
        import javis_lane
        d = os.path.join(javis_lane.state_dir(), "org-dispatch")
        return sorted(n for n in os.listdir(d)) if os.path.isdir(d) else []

    def test_r2_m1_non_head_item_of_a_merged_digest_is_delivered(self):
        os.environ["FAKE_MODE"] = "digest_tail"
        self.org.dispatch_task(self.task)
        self.assertEqual(self.org.first_ticket_state_of(self.task), "delivered")
        rc, out = self.org.dispatch_task(self.task)
        self.assertEqual(len(self._sends()), 1, "a ticket delivered inside a queue digest was sent again: " + out)
        self.assertIn("delivered (existing item, not resent)", out)

    def test_r2_m1_receipt_in_the_rotated_generation_counts(self):
        os.environ["FAKE_MODE"] = "rotated_in_dot1"
        self.org.dispatch_task(self.task)
        self.assertEqual(self.org.first_ticket_state_of(self.task), "delivered")

    def test_r2_m1_rotation_that_no_longer_covers_the_send_is_unknown_not_lost(self):
        os.environ["FAKE_MODE"] = "rotated_away"
        self.org.dispatch_task(self.task)
        self.assertEqual(self.org.first_ticket_state_of(self.task), "unknown")
        rc, out = self.org.dispatch_task(self.task)
        self.assertEqual(len(self._sends()), 1, "rotation read as lost and re-sent: " + out)
        self.assertNotEqual(rc, 0, out)

    def test_r2_m1_really_lost_ticket_without_rotation_is_still_resent(self):
        os.environ["FAKE_MODE"] = "pending"
        self.org.dispatch_task(self.task)
        os.remove(os.environ["FAKE_QUEUE"])          # the item vanished: in no list, no receipt, no rotation
        os.makedirs(os.path.dirname(os.environ["FAKE_LEDGER"]), exist_ok=True)
        with open(os.environ["FAKE_LEDGER"], "a", encoding="utf-8") as fh:   # the daemon restarted after the send
            fh.write(json.dumps({"origin": "boot", "ts_epoch": time.time() + 50}) + "\n")
        self.assertEqual(self.org.first_ticket_state_of(self.task), "lost")
        rc, out = self.org.dispatch_task(self.task)
        self.assertEqual(len(self._sends()), 2, out)
        self.assertIn("RESEND of lost item", out)

    def test_r2_m1_without_the_digest_absence_is_unknown_not_lost(self):
        from unittest import mock
        os.environ["FAKE_MODE"] = "pending"
        self.org.dispatch_task(self.task)
        os.remove(os.environ["FAKE_QUEUE"])
        os.makedirs(os.path.dirname(os.environ["FAKE_LEDGER"]), exist_ok=True)
        with open(os.environ["FAKE_LEDGER"], "a", encoding="utf-8") as fh:
            fh.write(json.dumps({"origin": "boot", "ts_epoch": time.time() + 50}) + "\n")
        with mock.patch.dict(sys.modules, {"javis_mission": None}):   # the digest mirror cannot be imported
            self.assertEqual(self.org.first_ticket_state_of(self.task), "unknown")
            rc, out = self.org.dispatch_task(self.task)
        self.assertEqual(len(self._sends()), 1, "absence without a digest was read as lost: " + out)
        self.assertNotEqual(rc, 0, out)

    # ── r2 review gemini1 minor (5-8 c): an unreadable OLD generation must not mask the CURRENT ledger or the queue ──
    def _unreadable_dot1(self):
        dot1 = os.path.splitext(os.environ["FAKE_LEDGER"])[0] + ".jsonl.1"
        os.makedirs(os.path.dirname(dot1), exist_ok=True)
        with open(dot1, "w", encoding="utf-8") as fh:
            fh.write(json.dumps({"origin": "boot", "ts_epoch": time.time() - 3600}) + "\n")
        os.chmod(dot1, 0)
        with self.assertRaises(OSError):         # positive control: the generation really cannot be read
            open(dot1, "rb").close()
        return dot1

    def test_r2fold_unreadable_dot1_does_not_mask_a_receipt_in_the_current_ledger(self):
        os.environ["FAKE_MODE"] = "deliver"
        self.org.dispatch_task(self.task)
        self._unreadable_dot1()
        self.assertEqual(self.org.first_ticket_state_of(self.task), "delivered")
        rc, out = self.org.dispatch_task(self.task)
        self.assertEqual((rc, len(self._sends())), (0, 1), out)
        self.assertIn("delivered (existing item, not resent)", out)

    def test_r2fold_unreadable_dot1_with_the_item_pending_reads_pending(self):
        os.environ["FAKE_MODE"] = "pending"
        self.org.dispatch_task(self.task)
        self._unreadable_dot1()
        self.assertEqual(self.org.first_ticket_state_of(self.task), "pending")

    def test_r2fold_unreadable_dot1_and_nothing_else_settles_it_stays_unknown(self):
        os.environ["FAKE_MODE"] = "pending"
        self.org.dispatch_task(self.task)
        os.remove(os.environ["FAKE_QUEUE"])
        self._unreadable_dot1()
        self.assertEqual(self.org.first_ticket_state_of(self.task), "unknown")
        rc, out = self.org.dispatch_task(self.task)
        self.assertEqual(len(self._sends()), 1, out)
        self.assertNotEqual(rc, 0, out)

    def test_r2fold_unreadable_dot1_but_the_current_ledger_covers_the_send_settles_lost(self):
        os.makedirs(os.path.dirname(os.environ["FAKE_LEDGER"]), exist_ok=True)
        with open(os.environ["FAKE_LEDGER"], "a", encoding="utf-8") as fh:   # current generation starts before the send
            fh.write(json.dumps({"origin": "boot", "ts_epoch": time.time() - 600}) + "\n")
        os.environ["FAKE_MODE"] = "pending"
        self.org.dispatch_task(self.task)
        os.remove(os.environ["FAKE_QUEUE"])
        self._unreadable_dot1()
        self.assertEqual(self.org.first_ticket_state_of(self.task), "lost")
        rc, out = self.org.dispatch_task(self.task)
        self.assertEqual(len(self._sends()), 2, out)
        self.assertIn("RESEND of lost item", out)

    def test_r2fold_unreadable_current_ledger_is_unknown_without_a_receipt(self):
        os.environ["FAKE_MODE"] = "pending"
        self.org.dispatch_task(self.task)
        os.remove(os.environ["FAKE_QUEUE"])
        dot1 = os.path.splitext(os.environ["FAKE_LEDGER"])[0] + ".jsonl.1"
        os.makedirs(os.path.dirname(dot1), exist_ok=True)
        with open(dot1, "w", encoding="utf-8") as fh:
            fh.write(json.dumps({"origin": "boot", "ts_epoch": time.time() - 3600}) + "\n")
        with open(os.environ["FAKE_LEDGER"], "w", encoding="utf-8") as fh:
            fh.write(json.dumps({"origin": "boot", "ts_epoch": time.time()}) + "\n")
        os.chmod(os.environ["FAKE_LEDGER"], 0)
        self.assertEqual(self.org.first_ticket_state_of(self.task), "unknown")

    def test_r2_m2_same_second_resend_orders_by_sequence_not_by_nonce(self):
        from unittest import mock
        nonces = iter([b"\xff" * 5, b"\x00" * 5, b"\x11" * 5])
        with mock.patch.object(self.org.os, "urandom", lambda n: next(nonces)), \
                mock.patch.object(self.org.time, "strftime", lambda *a, **k: "20261029T000000"), \
                mock.patch.object(self.org.time, "time_ns", lambda: 1793200000000000000):
            os.environ["FAKE_MODE"] = "expire"
            self.org.dispatch_task(self.task)
            os.environ["FAKE_MODE"] = "pending"
            rc, out = self.org.dispatch_task(self.task)
            self.assertIn("RESEND of expired item", out)
            self.assertEqual(self.org.first_ticket_state_of(self.task), "pending", self._keyed_copies())
            rc, out = self.org.dispatch_task(self.task)
        self.assertEqual(len(self._sends()), 2, "the newest copy lost to an older one by nonce order: " + out)

    def test_r2_m2_send_accepted_then_client_died_is_not_sent_twice(self):
        os.environ["FAKE_MODE"] = "send_then_die"
        rc1, out1 = self.org.dispatch_task(self.task)
        os.environ["FAKE_MODE"] = "pending"
        rc2, out2 = self.org.dispatch_task(self.task)
        self.assertEqual(len(self._sends()), 1, "an item the daemon accepted was queued twice: %s || %s" % (out1, out2))
        self.assertIn("pending (existing item, not resent)", out2)

    def test_r2_m3_apply_surfaces_the_dispatch_lines(self):
        import contextlib, io
        self.org.require_cso = lambda: None
        self.org.apply_plan = lambda m: [("dispatch_task", self.task)]
        self.org.validate_manifest = lambda m: []
        man = os.path.join(self.root, "manifest.json")
        with open(man, "w", encoding="utf-8") as f:
            json.dump({"departments": [], "tasks": [self.task]}, f)
        os.environ["FAKE_MODE"] = "expire"
        err, outb = io.StringIO(), io.StringIO()
        with contextlib.redirect_stderr(err), contextlib.redirect_stdout(outb):
            self.org.cmd_apply(man)
            os.environ["FAKE_MODE"] = "pending"
            self.org.cmd_apply(man)
        self.assertIn("[dispatch] queued", err.getvalue())
        self.assertIn("RESEND of expired item", err.getvalue())
        last = json.loads(outb.getvalue().strip().splitlines()[-1])
        self.assertTrue(any("RESEND of expired item" in x for x in last.get("dispatch", [])), last)

    def test_r2_m1_to_none_means_worker(self):
        os.environ["FAKE_MODE"] = "pending"
        rc, out = self.org.dispatch_task(dict(self.task, to=None))
        self.assertEqual(rc, 0, out)
        self.assertIn("--to worker", self._sends()[0])
        self.assertEqual(self.org._first_ticket_key(dict(self.task, to=None)), self.org._first_ticket_key(self.task))
        self.assertEqual(self.org._first_ticket_key(dict(self.task, to="")), self.org._first_ticket_key(self.task))

    def test_r2_g5_cmd_status_pins_incomplete_then_ok(self):
        import contextlib, io
        def status_of(task):
            man = os.path.join(self.root, "m-%s.json" % task["task"])
            with open(man, "w", encoding="utf-8") as f:
                json.dump({"departments": [{"key": "future-research"}], "tasks": [task]}, f)
            buf = io.StringIO()
            with contextlib.redirect_stdout(buf):
                rc = self.org.cmd_status(man)
            return rc, json.loads(buf.getvalue().strip().splitlines()[-1])
        os.environ["FAKE_MODE"] = "pending"
        self.org.dispatch_task(self.task)
        rc, d = status_of(self.task)
        self.assertEqual((rc, d["status"]), (1, "incomplete"), d)
        os.environ["FAKE_MODE"] = "deliver"
        t2 = dict(self.task, task="T2")
        self.org.dispatch_task(t2)
        rc, d = status_of(t2)
        self.assertEqual((rc, d["status"]), (0, "ok"), d)


if __name__ == "__main__":
    unittest.main()
