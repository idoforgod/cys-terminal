#!/usr/bin/env python3
"""javis_org.dispatch_task must DELIVER a new department's first ticket (0.14.56 EMIT-FORK E3).

At 9da2418d dispatch_task ran `javis_orchestra.py task-prompt`, kept the exit code and discarded the generated
ticket: the department's first ticket reached nobody, and apply reported success.

The fix sends the ticket to the department's socket (from depts.json by mission_key) with `cys send --queued`,
and only reports success when the department's delivery ledger shows the daemon's receipt for that ticket
(positive control each run - the recipient holds it - never an exit code).

The fakes: a `python3` shim stands in for task-prompt (prints a ticket), a fake `cys` records its argv and, in
the deliver mode, writes the queue row + receipt row the daemon writes (shapes from src/bin/cysd/delivery.rs).
"""
import importlib.util
import json
import os
import shutil
import stat
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(HERE)

FAKE_CYS = r'''#!/bin/sh
# fake cys: record argv; in deliver mode append the daemon's queue + receipt rows for `send --queued`
printf '%s\n' "$*" >> "$FAKE_LOG"
case " $* " in
  *" send "*)
    f=""; prev=""
    for a in "$@"; do [ "$prev" = "--file" ] && f="$a"; prev="$a"; done
    if [ -n "$f" ] && [ "$FAKE_MODE" != "silent" ]; then
      head -c 120 "$f" | tr '\n' ' ' > "$FAKE_LOG.preview"
      python3 - "$FAKE_LEDGER" "$FAKE_LOG.preview" "$FAKE_MODE" <<'PY'
import json, sys, time
led, prev, mode = sys.argv[1], open(sys.argv[2], encoding="utf-8").read(), sys.argv[3]
with open(led, "a", encoding="utf-8") as f:
    f.write(json.dumps({"origin": "queue", "queue_entry_id": "qfake.1", "preview": prev, "delivered_at": None}) + "\n")
    if mode == "deliver":
        f.write(json.dumps({"origin": "queue_receipt", "kind": "receipt", "queue_entry_id": "qfake.1",
                            "delivered_at": time.time()}) + "\n")
    elif mode == "expire":
        f.write(json.dumps({"origin": "queue_tombstone", "kind": "tombstone", "queue_entry_id": "qfake.1"}) + "\n")
PY
    fi
    echo "QUEUED (depth 1)"; exit 0 ;;
esac
exit 0
'''

FAKE_PY = r'''#!/bin/sh
# fake python3 for the task-prompt call only; everything else goes to the real interpreter
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
        self.bin = os.path.join(self.root, "bin")
        os.makedirs(self.bin)
        _exe(os.path.join(self.bin, "cys"), FAKE_CYS)
        _exe(os.path.join(self.bin, "python3"), FAKE_PY)
        self.state = os.path.join(self.root, "state")
        os.makedirs(self.state)
        self.sock = os.path.join(self.root, "dept", "cys.sock")
        self.depts = os.path.join(self.root, "depts.json")
        with open(self.depts, "w", encoding="utf-8") as f:
            json.dump({"depts": {"dept-9": {"mission_key": "future-research", "socket": self.sock}}}, f)
        self.log = os.path.join(self.root, "cys.log")
        self.saved = dict(os.environ)
        os.environ.update({"PATH": self.bin + os.pathsep + self.saved.get("PATH", ""), "REAL_PY": sys.executable,
                           "FAKE_LOG": self.log, "CYS_STATE_DIR": self.state, "CYS_DEPTS_JSON": self.depts,
                           "JAVIS_ORG_DISPATCH_WAIT_S": "2"})
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
        return [l for l in open(self.log, encoding="utf-8").read().splitlines() if " send " in " %s " % l]

    def test_delivered_ticket_reports_success(self):
        os.environ["FAKE_MODE"] = "deliver"
        rc, out = self.org.dispatch_task(self.task)
        sends = self._sends()
        self.assertEqual(len(sends), 1, "the ticket was not sent to the department")
        self.assertIn("--socket %s" % self.sock, sends[0])
        self.assertIn("--queued", sends[0])
        self.assertIn("--to worker", sends[0])
        self.assertEqual(rc, 0, out)
        self.assertIn("delivered", out)

    def test_queued_but_no_receipt_is_not_success(self):  # the positive control must be able to fail
        os.environ["FAKE_MODE"] = "queued_only"
        rc, out = self.org.dispatch_task(self.task)
        self.assertEqual(len(self._sends()), 1)
        self.assertNotEqual(rc, 0, out)
        self.assertIn("queued_not_delivered", out)

    def test_expired_item_is_not_success(self):
        os.environ["FAKE_MODE"] = "expire"
        rc, out = self.org.dispatch_task(self.task)
        self.assertNotEqual(rc, 0, out)
        self.assertIn("expired", out)

    def test_unresolved_department_sends_nothing(self):
        os.environ["FAKE_MODE"] = "deliver"
        rc, out = self.org.dispatch_task(dict(self.task, dept="no-such-mission"))
        self.assertNotEqual(rc, 0)
        self.assertIn("dept_unresolved", out)
        self.assertEqual(self._sends(), [])


if __name__ == "__main__":
    unittest.main()
