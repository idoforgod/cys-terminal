#!/usr/bin/env python3
"""0.14.56 pre-tag review PKG-3A r1 - tools lane specimens (HEAD-RULING-PKG3A-R1 sections 1-2, head answer to the w53 plan).

M3  javis_org child waits are bounded. Read-only / idempotent calls (cys status, cys queue list, cys send, the task-prompt
    child, the purge verifier) are killed at their bound: a queue read that times out is UNKNOWN (None), never "absent";
    a send that times out goes through the existing item-state check. The two MUTATING calls (cys-dept create, cys-dept
    down) are never killed: on timeout the child is left running, the result names its pid, nothing is retried, and a
    down that did not finish stops the destroy before any isolation.
#9  task-roots.json is written with flush+fsync and its temp file is removed when the write fails.
#12 every text=True child in javis_org.py, javis_actprobe.py and javis_report_gate.py decodes as utf-8 with
    errors='replace' (the Windows ANSI code page is not used); the report_gate task-list child runs with -B.
    macOS stand-in for a Korean Windows code page: a child python under LC_ALL=ko_KR.CP949 PYTHONUTF8=0 (its preferred
    encoding is then CP949, and UTF-8 Korean bytes raise UnicodeDecodeError in a strict decode). Not measured on Windows.
#13 the durable first-ticket copy is opened with O_BINARY (where the platform has it) and newline='\\n'. macOS has no
    O_BINARY: the production path runs against a stand-in `os` that has one and records the flags. Not measured on Windows.
#14 phoenix _atomic_write_json removes its temp file on failure and accepts a bare relative path.
#15 the delivery-ledger reader keeps only JSON objects; a valid non-object line no longer crashes the receipt check.

Run: python3 test_pkg3a_tools_lane.py   (POSIX only - the fakes are sh shims)
"""
import ast
import importlib.util
import json
import os
import shutil
import signal
import stat
import subprocess
import sys
import tempfile
import threading
import time
import types
import unittest
from unittest import mock

sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(HERE)
if BIN not in sys.path:
    sys.path.insert(0, BIN)

HANG_S = 20          # how long a sleeping fake holds its caller
JOIN_S = 10          # a bounded call must return well inside this (its bound is patched to 1 s)
KOREAN = "첫 프로젝트 티켓 — 한글 미리보기"


def _exe(path, text):
    with open(path, "w", encoding="utf-8") as f:
        f.write(text)
    os.chmod(path, os.stat(path).st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)


def _load(name, filename):
    spec = importlib.util.spec_from_file_location(name, os.path.join(BIN, filename))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def _alive(pid):
    try:
        os.kill(pid, 0)
        return True
    except OSError:
        return False


def _bounded(fn, *a, **k):
    """Run fn on a thread; (finished, result_or_exc, seconds). An unbounded wait shows as finished=False."""
    box = {}

    def run():
        try:
            box["r"] = fn(*a, **k)
        except BaseException as e:   # noqa: BLE001 - the specimen reports it
            box["e"] = e
    t0 = time.monotonic()
    th = threading.Thread(target=run, daemon=True)
    th.start()
    th.join(JOIN_S)
    return (not th.is_alive()), box.get("e", box.get("r")), time.monotonic() - t0


FAKE_CYS = r'''#!/bin/sh
printf '%s\n' "$*" >> "$FAKE_LOG"
exec "$REAL_PY" -B "$FAKE_IMPL" "$@"
'''

FAKE_IMPL = r'''
import json, os, sys, time
a = sys.argv[1:]
mode = os.environ.get("FAKE_MODE", "")
qf = os.environ["FAKE_QUEUE"]
q = json.load(open(qf, encoding="utf-8")) if os.path.exists(qf) else {"pending": [], "expired": []}
def out(obj):
    sys.stdout.buffer.write(json.dumps(obj, ensure_ascii=False).encode("utf-8") + b"\n")   # raw UTF-8, like cys
if "queue" in a and "list" in a:
    if mode == "sleep_queue":
        time.sleep(float(os.environ["HANG_S"]))
    out([{"id": i, "preview": p} for i, p in q["expired" if "--expired" in a else "pending"]])
    sys.exit(0)
if "status" in a and "--json" in a:
    if mode == "sleep_status":
        time.sleep(float(os.environ["HANG_S"]))
    out({"surfaces": [{"role": "worker", "agent_alive": True, "idle_secs": 1, "title": os.environ.get("KOREAN", "")}]})
    sys.exit(0)
if "read-screen" in a:
    out({"text": os.environ.get("KOREAN", "") + "\n> ", "composer": None})
    sys.exit(0)
if "send" in a:
    f = a[a.index("--file") + 1]
    prev = open(f, encoding="utf-8").read().replace("\n", " ")[:80]
    if mode == "sleep_send_before":
        time.sleep(float(os.environ["HANG_S"]))
        sys.exit(0)
    q["pending"].append(["qfake.1", prev]); json.dump(q, open(qf, "w", encoding="utf-8"))
    if mode == "sleep_send_after":
        time.sleep(float(os.environ["HANG_S"]))
    print("QUEUED (depth 1)")
    sys.exit(0)
sys.exit(3)
'''

FAKE_PY = r'''#!/bin/sh
case "$1" in
  *javis_orchestra.py) if [ "$FAKE_MODE" = sleep_tp ]; then exec sleep "$HANG_S"; fi; echo "[ticket] first project"; exit 0 ;;
esac
exec "$REAL_PY" "$@"
'''

FAKE_CYS_DEPT = r'''#!/bin/sh
echo $$ > "$FAKE_PIDFILE"
echo "$$ $*" >> "$FAKE_PIDFILE.calls"
echo "fake cys-dept $*"
trap 'kill $! 2>/dev/null; exit 143' TERM
sleep "$HANG_S" &
wait $!
echo "fake cys-dept done"
exit 0
'''


@unittest.skipIf(os.name == "nt", "the fakes are POSIX sh shims")
class _Org(unittest.TestCase):
    def setUp(self):
        self.root = tempfile.mkdtemp(prefix="pkg3a-tools-")
        self.addCleanup(shutil.rmtree, self.root, True)
        self.bin = os.path.join(self.root, "bin")
        os.makedirs(self.bin)
        _exe(os.path.join(self.bin, "cys"), FAKE_CYS)
        _exe(os.path.join(self.bin, "python3"), FAKE_PY)
        _exe(os.path.join(self.bin, "cys-dept"), FAKE_CYS_DEPT)
        with open(os.path.join(self.root, "fake_impl.py"), "w", encoding="utf-8") as f:
            f.write(FAKE_IMPL)
        self.home = os.path.join(self.root, "home")
        os.makedirs(self.home)
        self.state = os.path.join(self.root, "state")
        os.makedirs(self.state)
        self.sock = os.path.join(self.root, "dept", "cys.sock")
        self.depts = os.path.join(self.root, "depts.json")
        with open(self.depts, "w", encoding="utf-8") as f:
            json.dump({"depts": {"dept-9": {"mission_key": "future-research", "socket": self.sock}}}, f)
        self.pidfile = os.path.join(self.root, "cys-dept.pid")
        self.saved = dict(os.environ)
        self.addCleanup(self._restore_env)
        os.environ.update({"PATH": self.bin + os.pathsep + self.saved.get("PATH", ""), "REAL_PY": sys.executable,
                           "FAKE_LOG": os.path.join(self.root, "cys.log"),
                           "FAKE_IMPL": os.path.join(self.root, "fake_impl.py"),
                           "FAKE_QUEUE": os.path.join(self.root, "queue.json"), "CYS_STATE_DIR": self.state,
                           "CYS_DEPTS_JSON": self.depts, "HANG_S": str(HANG_S), "FAKE_PIDFILE": self.pidfile,
                           "HOME": self.home, "KOREAN": KOREAN})
        self.org = _load("javis_org_pkg3a", "javis_org.py")
        self.org.require_cso = lambda: None
        for name in ("CYS_CALL_TIMEOUT_S", "TASK_PROMPT_TIMEOUT_S", "PURGE_VERIFY_TIMEOUT_S",
                     "DEPT_CREATE_TIMEOUT_S", "DEPT_DOWN_TIMEOUT_S"):
            setattr(self.org, name, 1)   # absent on the unfixed tree: the setattr is then a no-op and the wait is unbounded
        self.task = {"dept": "future-research", "task": "T1", "scope": "s", "to": "worker"}
        self.addCleanup(self._kill_fake_dept)

    def _restore_env(self):
        os.environ.clear()
        os.environ.update(self.saved)

    def _kill_fake_dept(self):
        try:
            with open(self.pidfile) as f:
                pid = int(f.read().strip())
        except (OSError, ValueError):
            return
        if _alive(pid):
            os.kill(pid, signal.SIGTERM)

    def _fake_pid(self):
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            try:
                with open(self.pidfile) as f:
                    return int(f.read().strip())
            except (OSError, ValueError):
                time.sleep(0.05)
        self.fail("the fake cys-dept never wrote its pid")


class M3ReadOnlyBounds(_Org):
    """M3 read-only / idempotent calls: a sleeping fake must not hold the caller past the bound."""

    def test_queue_list_timeout_is_unknown_not_absent(self):
        os.environ["FAKE_MODE"] = "sleep_queue"
        done, r, secs = _bounded(self.org._queue_has, self.sock, "abcdef0123")
        self.assertTrue(done, "_queue_has waited on a stuck `cys queue list` with no bound")
        self.assertIsNone(r, "a timed-out queue read must be None (UNKNOWN), got %r" % (r,))

    def test_ticket_state_with_a_stuck_queue_read_is_unknown(self):
        os.environ["FAKE_MODE"] = "sleep_queue"
        done, r, _ = _bounded(self.org._ticket_state, self.sock, "abcdef0123")
        self.assertTrue(done, "_ticket_state waited on a stuck queue read")
        self.assertEqual(r, "unknown")

    def test_dept_status_is_bounded(self):
        os.environ["FAKE_MODE"] = "sleep_status"
        done, r, _ = _bounded(self.org.dept_status, self.sock)
        self.assertTrue(done, "dept_status waited on a stuck `cys status` with no bound")
        self.assertIsNone(r)

    def test_send_that_hangs_before_the_hand_off_is_bounded_and_settled_by_the_item_state(self):
        os.environ["FAKE_MODE"] = "sleep_send_before"
        done, r, _ = _bounded(self.org._deliver_first_ticket, self.task, "ticket body", "")
        self.assertTrue(done, "the first-ticket send waited on a stuck `cys send` with no bound")
        rc, out = r
        self.assertNotEqual(rc, 0, out)
        self.assertIn("send_failed", out)

    def test_send_that_hangs_after_the_hand_off_reads_the_queued_item(self):
        os.environ["FAKE_MODE"] = "sleep_send_after"
        done, r, _ = _bounded(self.org._deliver_first_ticket, self.task, "ticket body", "")
        self.assertTrue(done, "the first-ticket send waited on a stuck `cys send` with no bound")
        rc, out = r
        self.assertEqual(rc, 0, out)
        self.assertIn("the item is pending", out)

    def test_task_prompt_child_is_bounded(self):
        os.environ["FAKE_MODE"] = "sleep_tp"
        done, r, _ = _bounded(self.org.dispatch_task, self.task)
        self.assertTrue(done, "dispatch_task waited on a stuck task-prompt child with no bound")
        rc, out = r
        self.assertNotEqual(rc, 0, out)


class M3MutatingCallsAreNeverKilled(_Org):
    """M3 head answer 2): cys-dept create/down are left running on timeout; the result names the pid; no retry."""

    def test_create_timeout_leaves_the_child_running_and_names_its_pid(self):
        done, r, secs = _bounded(self.org.create_dept, "dept-9")
        self.assertTrue(done, "create_dept waited on a stuck `cys-dept create` with no bound")
        rc, out = r
        pid = self._fake_pid()
        self.assertNotEqual(rc, 0, out)
        self.assertIn("pid %d" % pid, out)
        self.assertIn("outcome unknown", out)
        self.assertTrue(_alive(pid), "the create child was killed - a half-built department is worse than waiting")
        with open(self.pidfile + ".calls", encoding="utf-8") as f:
            calls = f.read().splitlines()
        self.assertEqual(len(calls), 1, "cys-dept create must run once and never be retried: %r" % calls)

    def test_create_that_finishes_inside_the_bound_keeps_its_output(self):
        os.environ["HANG_S"] = "0"
        self.org.DEPT_CREATE_TIMEOUT_S = 10
        rc, out = self.org.create_dept("dept-9")
        self.assertEqual(rc, 0, out)
        self.assertIn("fake cys-dept create dept-9", out)
        self.assertIn("fake cys-dept done", out)

    def test_down_timeout_leaves_the_child_running_and_stops_before_isolation(self):
        pack = os.path.join(self.home, ".cys", "pack-dept-dept-9")
        os.makedirs(pack)
        os.environ["CYS_DEPT_BIN"] = os.path.join(self.bin, "cys-dept")
        err = []
        with mock.patch.object(self.org.sys, "stderr", types.SimpleNamespace(write=err.append, flush=lambda: None)):
            done, r, _ = _bounded(self.org.destroy_dept, "dept-9", "future-research", purge=True)
        self.assertTrue(done, "destroy_dept waited on a stuck `cys-dept down` with no bound")
        pid = self._fake_pid()
        self.assertTrue(_alive(pid), "the down child was killed - a half-stopped department is worse than waiting")
        self.assertTrue(os.path.isdir(pack), "the pack was isolated although the down did not finish")
        text = "".join(err)
        self.assertIn("pid %d" % pid, text)
        self.assertIn("outcome unknown", text)
        self.assertTrue(any(a[0] == "down" and a[1] != 0 for a in r), r)


class M3Census(unittest.TestCase):
    """Every subprocess call in javis_org.py is bounded (a timeout= keyword, or a Popen whose wait has a timeout)."""

    def test_every_run_call_has_a_timeout(self):
        with open(os.path.join(BIN, "javis_org.py"), encoding="utf-8") as f:
            src = f.read()
        missing, runs = [], 0
        for node in ast.walk(ast.parse(src)):
            if isinstance(node, ast.Call) and isinstance(node.func, ast.Attribute) \
                    and isinstance(node.func.value, ast.Name) and node.func.value.id == "subprocess" \
                    and node.func.attr in ("run", "call", "check_call", "check_output"):
                runs += 1
                if not any(k.arg == "timeout" for k in node.keywords):
                    missing.append(node.lineno)
        self.assertGreaterEqual(runs, 5, "census vacuous: too few subprocess.run calls found")
        self.assertEqual(missing, [], "subprocess calls without timeout= at javis_org.py lines %s" % missing)


ENC_FILES = ("javis_org.py", "javis_actprobe.py", "javis_report_gate.py")


class Enc12Census(unittest.TestCase):
    """#12: every text=True child in the three files passes encoding='utf-8' and errors='replace'."""

    def test_every_text_child_decodes_utf8_with_replace(self):
        bad, seen = [], 0
        for fn in ENC_FILES:
            with open(os.path.join(BIN, fn), encoding="utf-8") as f:
                src = f.read()
            for node in ast.walk(ast.parse(src)):
                if not isinstance(node, ast.Call):
                    continue
                kw = {k.arg: k.value for k in node.keywords if k.arg}
                t = kw.get("text") or kw.get("universal_newlines")
                if not (isinstance(t, ast.Constant) and t.value is True):
                    continue
                seen += 1
                enc, errs = kw.get("encoding"), kw.get("errors")
                if not (isinstance(enc, ast.Constant) and enc.value == "utf-8"
                        and isinstance(errs, ast.Constant) and errs.value == "replace"):
                    bad.append("%s:%d" % (fn, node.lineno))
        self.assertGreaterEqual(seen, 18, "census vacuous: %d text-mode calls found (18 at 2b7cd363)" % seen)
        self.assertEqual(bad, [], "text=True without encoding='utf-8', errors='replace': %s" % bad)

    def test_report_gate_task_list_child_runs_with_dash_b(self):
        rg = _load("javis_report_gate_pkg3a", "javis_report_gate.py")
        tmp = tempfile.mkdtemp(prefix="pkg3a-rg-")
        self.addCleanup(shutil.rmtree, tmp, True)
        with open(os.path.join(tmp, "javis_task.py"), "w", encoding="utf-8") as f:
            f.write("print('[]')\n")
        seen = []
        real = rg.subprocess.run

        def spy(argv, *a, **k):
            seen.append(list(argv))
            return real(argv, *a, **k)
        with mock.patch.object(rg.subprocess, "run", spy):
            ok, rows, err = rg.Runner(tmp, tmp, timeout=20)._task_list_at(tmp)
        self.assertTrue(ok, err)
        self.assertEqual(len(seen), 1)
        self.assertEqual(seen[0][1], "-B", "the task-list child must run with -B: %r" % seen[0])


CP949_CHILD = r'''
import importlib.util, json, os, sys
sys.dont_write_bytecode = True
BIN, which = sys.argv[1], sys.argv[2]
def load(name, fn):
    spec = importlib.util.spec_from_file_location(name, os.path.join(BIN, fn))
    m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m); return m
import locale
res = {"enc": locale.getpreferredencoding(False)}
try:
    if which == "org_status":
        org = load("o", "javis_org.py")
        st = org.dept_status(os.environ["SOCK"])
        res["value"] = (st or {}).get("surfaces", [{}])[0].get("title")
    elif which == "org_queue":
        org = load("o", "javis_org.py")
        res["value"] = org._queue_has(os.environ["SOCK"], os.environ["NONCE"])
    elif which == "actprobe":
        ap = load("a", "javis_actprobe.py")
        res["value"] = ap._read_screen_live("surface:1")[0]
    elif which == "report_gate":
        rg = load("r", "javis_report_gate.py")
        ok, rows, err = rg.Runner(os.environ["RG_BIN"], os.environ["RG_BIN"], timeout=20)._task_list_at(os.environ["RG_BIN"])
        res["value"] = rows[0]["title"] if ok and rows else err
except Exception as e:
    res["error"] = "%s: %s" % (type(e).__name__, str(e)[:120])
sys.stdout.write(json.dumps(res, ensure_ascii=True) + "\n")
'''


@unittest.skipIf(os.name == "nt", "a macOS/Linux locale stands in for a Korean Windows code page")
class Enc12Cp949StandIn(_Org):
    """#12 behaviour: under a CP949 preferred encoding, UTF-8 Korean child output must still decode correctly."""

    def setUp(self):
        super().setUp()
        r = subprocess.run(["locale", "-a"], capture_output=True, timeout=10)
        if b"ko_KR.CP949" not in r.stdout:
            self.skipTest("ko_KR.CP949 locale absent on this host - stand-in unavailable (not measured)")
        self.child = os.path.join(self.root, "cp949_child.py")
        with open(self.child, "w", encoding="utf-8") as f:
            f.write(CP949_CHILD)
        rgbin = os.path.join(self.root, "rgbin")
        os.makedirs(rgbin)
        with open(os.path.join(rgbin, "javis_task.py"), "w", encoding="utf-8") as f:
            f.write("import json, sys\nsys.stdout.buffer.write(json.dumps([{'title': %r}], ensure_ascii=False)"
                    ".encode('utf-8'))\n" % KOREAN)
        self.rgbin = rgbin
        with open(os.environ["FAKE_QUEUE"], "w", encoding="utf-8") as f:
            json.dump({"pending": [["q1", "[org-dispatch 0a1b2c3d4e] %s" % KOREAN]], "expired": []}, f,
                      ensure_ascii=False)

    def run_child(self, which):
        env = {k: v for k, v in os.environ.items() if not k.startswith(("LC_", "LANG", "PYTHONUTF8", "PYTHONIOENCODING"))}
        env.update({"LC_ALL": "ko_KR.CP949", "PYTHONUTF8": "0", "SOCK": self.sock, "NONCE": "0a1b2c3d4e",
                    "RG_BIN": self.rgbin, "CYS_BIN": os.path.join(self.bin, "cys")})
        r = subprocess.run([sys.executable, "-B", self.child, BIN, which], capture_output=True, env=env, timeout=60)
        self.assertEqual(r.returncode, 0, r.stderr.decode("utf-8", "replace"))
        res = json.loads(r.stdout.decode("ascii"))
        self.assertEqual(res["enc"].upper().replace("-", ""), "CP949", "the stand-in locale did not take: %r" % res)
        return res

    def test_org_dept_status(self):
        res = self.run_child("org_status")
        self.assertNotIn("error", res, res)
        self.assertEqual(res["value"], KOREAN)

    def test_org_queue_has(self):
        res = self.run_child("org_queue")
        self.assertNotIn("error", res, res)
        self.assertIs(res["value"], True)

    def test_actprobe_read_screen(self):
        res = self.run_child("actprobe")
        self.assertNotIn("error", res, res)
        self.assertTrue(res["value"].startswith(KOREAN), res)

    def test_report_gate_task_list(self):
        res = self.run_child("report_gate")
        self.assertNotIn("error", res, res)
        self.assertEqual(res["value"], KOREAN)


class _OsStandIn(object):
    """A stand-in `os` for javis_org: it has an O_BINARY bit (as Windows does) and records open flags / fdopen keywords.
    The real open is called without that bit (macOS would reject or misread it)."""
    O_BINARY = 0x8000

    def __init__(self, real):
        self._real, self.opened, self.fdopened = real, [], []

    def __getattr__(self, name):
        return getattr(self._real, name)

    def open(self, path, flags, *a):
        self.opened.append((path, flags))
        return self._real.open(path, flags & ~self.O_BINARY, *a)

    def fdopen(self, fd, *a, **k):
        self.fdopened.append((a, dict(k)))
        return self._real.fdopen(fd, *a, **k)


class Minor13DurableCopyIsBinaryLf(_Org):
    def test_copy_is_opened_binary_with_lf_newlines(self):
        os.environ["FAKE_MODE"] = "pending"
        standin = _OsStandIn(os)
        with mock.patch.object(self.org, "os", standin):
            rc, out = self.org._deliver_first_ticket(self.task, "line one\nline two\n", "")
        self.assertEqual(rc, 0, out)
        copies = [(p, fl) for p, fl in standin.opened if p.endswith(".txt") and "org-dispatch" in p]
        self.assertEqual(len(copies), 1, standin.opened)
        self.assertTrue(copies[0][1] & _OsStandIn.O_BINARY, "the durable copy is not opened with O_BINARY")
        self.assertEqual(len(standin.fdopened), 1, standin.fdopened)
        self.assertEqual(standin.fdopened[0][1].get("newline"), "\n", standin.fdopened[0])
        with open(copies[0][0], "rb") as f:
            self.assertNotIn(b"\r\n", f.read())


class Minor15LedgerRowsAreObjects(_Org):
    def test_non_object_lines_are_skipped_and_the_receipt_is_found(self):
        led = os.path.join(self.root, "delivery.jsonl")
        rows = [[1, 2], "a string", 7, None,
                {"origin": "queue", "queue_entry_id": "q1", "preview": "[org-dispatch 0a1b2c3d4e] x", "delivered_at": None},
                {"origin": "queue_receipt", "kind": "receipt", "queue_entry_id": "q1", "delivered_at": time.time()}]
        with open(led, "w", encoding="utf-8") as f:
            for r in rows:
                f.write(json.dumps(r) + "\n")
        self.assertTrue(all(isinstance(r, dict) for r in self.org._ledger_rows(led)))
        self.assertEqual(self.org._ticket_receipt(led, "0a1b2c3d4e"), "delivered")


class Minor9RegistryWrite(unittest.TestCase):
    def setUp(self):
        self.root = tempfile.mkdtemp(prefix="pkg3a-task-")
        self.addCleanup(shutil.rmtree, self.root, True)
        self.pack = os.path.join(self.root, "pack-dept-dept-T")
        os.makedirs(os.path.join(self.pack, "state"))
        saved = dict(os.environ)
        self.addCleanup(lambda: (os.environ.clear(), os.environ.update(saved)))
        os.environ.update({"JAVIS_ROOT": os.path.join(self.root, "ws"), "CYS_PACK_DIR": self.pack,
                           "JAVIS_TASK_PUBLISH_EPHEMERAL": "1"})
        self.task = _load("javis_task_pkg3a", "javis_task.py")
        self.reg_dir = os.path.join(self.pack, "state")

    def test_registry_write_is_fsynced(self):
        calls = []
        real = os.fsync
        with mock.patch.object(self.task.os, "fsync", lambda fd: (calls.append(fd), real(fd))):
            self.task._publish_root()
        self.assertTrue(os.path.exists(os.path.join(self.reg_dir, "task-roots.json")))
        self.assertGreaterEqual(len(calls), 1, "task-roots.json was replaced without fsync")

    def test_failed_write_leaves_no_temp_file(self):
        def boom(*a, **k):
            raise OSError(28, "No space left on device (specimen)")
        with mock.patch.object(self.task.json, "dump", boom):
            self.task._publish_root()
        left = [n for n in os.listdir(self.reg_dir) if ".tmp." in n]
        self.assertEqual(left, [], "the registry temp file leaked: %s" % left)


class Minor14PhoenixAtomicWrite(unittest.TestCase):
    def setUp(self):
        self.root = tempfile.mkdtemp(prefix="pkg3a-phx-")
        self.addCleanup(shutil.rmtree, self.root, True)
        self.phx = _load("javis_phoenix_pkg3a", "javis_phoenix.py")

    def test_failed_dump_leaves_no_temp_file(self):
        path = os.path.join(self.root, "result.json")
        with self.assertRaises(TypeError):
            self.phx._atomic_write_json(path, {"bad": object()})
        self.assertEqual([n for n in os.listdir(self.root) if n.startswith(".tmp-")], [])
        self.assertFalse(os.path.exists(path))

    def test_bare_relative_path_is_written(self):
        cwd = os.getcwd()
        self.addCleanup(os.chdir, cwd)
        os.chdir(self.root)
        self.phx._atomic_write_json("result.json", {"phoenix_restore": "VERIFIED", "exit": 0})
        with open(os.path.join(self.root, "result.json"), encoding="utf-8") as f:
            self.assertEqual(json.load(f), {"phoenix_restore": "VERIFIED", "exit": 0})
        self.assertEqual([n for n in os.listdir(self.root) if n.startswith(".tmp-")], [])


if __name__ == "__main__":
    unittest.main(verbosity=2)
