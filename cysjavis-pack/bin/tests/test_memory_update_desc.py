#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_memory_update_desc.py — `update --body` may not leave the index description behind (CEO A8).

Seats read the INDEX, so a body changed under an old description is a fact recall cannot see. The tool knew
(`index_updated: false`) and said so on stdout, where nobody reads it. The gate fails closed: --body needs --desc, or
--keep-desc saying the description is deliberately unchanged. --keep-desc exists so a typo fix is not blocked, and it
leaves a durable row, because stdout is the channel that hid this defect. The row is written before the card, so it
records an attempt, not a completion.

Round 2: the card was read before the lock, so a keep-desc update could restore a description a peer had just written.

Every case builds its environment from nothing and asserts each axis by name before any operation.

    python3 -B cysjavis-pack/bin/tests/test_memory_update_desc.py -v

Each `[control]` line prints what was measured.
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from rig_axes import AXES, assert_sealed, breaches  # noqa: E402

BIN = os.path.dirname(HERE)
TOOL = os.path.join(BIN, "javis_memory.py")
PY = sys.executable
LOG = ".update-log.jsonl"
NAME = "alpha-rule"
CARD = "feedback_%s.md" % NAME
OLD_DESC = "알파 규칙의 옛 설명"
OLD_BODY = "옛 본문"
NEW_DESC = "알파 규칙의 새 설명"
NEW_BODY = "새 본문 — 사실이 바뀌었다"
PEER_DESC = "동료가 방금 쓴 설명"
# Runs one real verb. Nothing is mocked but the named pause or fault. argv: tool, mark, go, where, then the tool's argv.
HARNESS = """
import importlib.util, os, sys, time
spec = importlib.util.spec_from_file_location("jm", sys.argv[1])
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)
mark, go, where = sys.argv[2:5]
sys.argv = ["javis_memory.py"] + sys.argv[5:]

def pause():
    open(mark, "w").close()
    limit = time.time() + 30
    while not os.path.exists(go):
        if time.time() > limit:
            os._exit(70)
        time.sleep(0.01)

real_enter, real_write = m.FileLock.__enter__, m._write_text_atomic
real_sleep = time.sleep

if where == "observe-lock-wait":
    # Keep the real exclusive-file lock and its retries. Freeze only its clock
    # so scheduler delay in the test cannot expire/reclaim the held lock.
    held_clock = time.time()
    m.time.time = lambda: held_clock

    def observe_sleep(delay):
        if sys._getframe(1).f_code is real_enter.__code__:
            # This call is reached only after the real exclusive open collided.
            open(mark + ".waiting", "w").close()
        return real_sleep(delay)

    m.time.sleep = observe_sleep

def enter(self):
    with open(mark + ".enters", "a") as f:
        f.write("1")
    if where == "before-lock":
        pause()
    return real_enter(self)

def write(path, text):
    if os.path.basename(path) != m.INDEX_FILE:
        if where == "before-card-write":
            pause()
        if where == "card-write-fails":
            raise OSError(5, "injected card write failure")
    return real_write(path, text)

def fsync(fd):
    raise OSError(5, "injected fsync failure")

m.FileLock.__enter__ = enter
m._write_text_atomic = write
if where == "fsync-fails":
    m.os.fsync = fsync
sys.exit(m.main())
"""


def control(msg):
    print("[control] " + msg, flush=True)


class Base(unittest.TestCase):
    def setUp(self):
        self.tmp = os.path.realpath(tempfile.mkdtemp(prefix="memory-update-"))
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.addCleanup(os.chdir, os.getcwd())
        os.chdir(self.tmp)
        self.home = os.path.join(self.tmp, "home")
        self.mdir = os.path.join(self.tmp, "memory")
        self.pack = os.path.join(self.tmp, "pack")
        for d in (self.home, self.mdir, self.pack, os.path.join(self.tmp, "root"), os.path.join(self.tmp, "appdata"),
                  os.path.join(self.tmp, "state")):
            os.makedirs(d)
        self.env = {
            "PATH": "/usr/bin:/bin",
            "LANG": "C.UTF-8",
            "PYTHONUTF8": "1",
            "HOME": self.home,
            "USERPROFILE": self.home,
            "LOCALAPPDATA": os.path.join(self.tmp, "appdata"),
            "CYS_SOCKET": os.path.join(self.tmp, "no.sock"),
            "CYS_STATE_DIR": os.path.join(self.tmp, "state"),
            "CYS_PACK_DIR": self.pack,
            "JAVIS_ROOT": os.path.join(self.tmp, "root"),
            "CYS_PROBE_RUNS": os.path.join(self.tmp, "probe_runs.jsonl"),
        }
        with open(os.path.join(self.mdir, "MEMORY.md"), "w", encoding="utf-8") as f:
            f.write("# MEMORY.md — rig\n\n```markdown\n- [예시](type_example.md) — 코드펜스 예시\n```\n\n")
        r = self.run_tool("add", "--type", "feedback", "--name", NAME, "--desc", OLD_DESC, "--body", OLD_BODY)
        self.assertEqual(r.returncode, 0, r.stderr)

    def run_tool(self, *argv, env=None):
        full = dict(self.env if env is None else env)
        assert_sealed(self, full, self.tmp)
        self.assertEqual(os.path.realpath(os.getcwd()), self.tmp)
        self.assertEqual(os.path.commonpath([self.tmp, self.mdir]), self.tmp)
        return subprocess.run([PY, "-B", TOOL, "--dir", self.mdir] + list(argv), cwd=self.tmp, env=full,
                              capture_output=True, text=True, timeout=60)

    def harness(self, where, *argv, env=None):
        """Start one real verb under HARNESS. Returns (process, mark, go)."""
        full = dict(self.env if env is None else env)
        assert_sealed(self, full, self.tmp)
        self.assertEqual(os.path.realpath(os.getcwd()), self.tmp)
        mark, go = os.path.join(self.tmp, "mark"), os.path.join(self.tmp, "go")
        p = subprocess.Popen([PY, "-B", "-c", HARNESS, TOOL, mark, go, where, "--dir", self.mdir] + list(argv),
                             cwd=self.tmp, env=full, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        self.addCleanup(p.kill)
        self.addCleanup(p.stdout.close)
        self.addCleanup(p.stderr.close)
        return p, mark, go

    def wait_for(self, path, proc):
        for _ in range(3000):
            if os.path.exists(path):
                return
            self.assertIsNone(proc.poll(), "the verb ended before reaching its pause")
            time.sleep(0.01)
        self.fail("never reached %s" % path)

    def snapshot(self):
        """Every byte in the memory directory, by name, so a stray temp, lock or log file also counts as a write."""
        out = {}
        for n in sorted(os.listdir(self.mdir)):
            p = os.path.join(self.mdir, n)
            out[n] = open(p, "rb").read() if os.path.isfile(p) else None
        return out

    def card(self):
        return open(os.path.join(self.mdir, CARD), encoding="utf-8").read()

    def index(self):
        return open(os.path.join(self.mdir, "MEMORY.md"), encoding="utf-8").read()

    def rows(self):
        p = os.path.join(self.mdir, LOG)
        if not os.path.exists(p):
            return []
        with open(p, encoding="utf-8") as f:
            return [json.loads(line) for line in f]

    def assert_refused(self, rc, r, before, *names):
        self.assertEqual(r.returncode, rc, r.stdout + r.stderr)
        self.assertEqual(self.snapshot(), before)
        self.assertEqual(r.stdout, "")
        msg = json.loads(r.stderr.strip().splitlines()[-1])["error"]
        for n in names:
            self.assertIn(n, msg)
        self.assertIn("쓰지 않았다", msg)
        return msg


class Fixture(Base):
    def test_every_axis_is_inside_the_rig_and_named(self):
        self.assertEqual(os.path.realpath(os.getcwd()), self.tmp)
        assert_sealed(self, self.env, self.tmp)
        control("axes asserted by name: %s" % " ".join(AXES))

    def test_the_axis_check_can_say_no(self):
        for axis, bad in (("HOME", "/"), ("CYS_PACK_DIR", "/nonexistent-outside")):
            with self.subTest(axis=axis):
                self.assertEqual(breaches(dict(self.env, **{axis: bad}), self.tmp, self.tmp), {axis: bad})
        self.assertEqual(breaches(self.env, "/", self.tmp), {"cwd": "/"})
        missing = {k: v for k, v in self.env.items() if k != "CYS_PACK_DIR"}
        self.assertEqual(breaches(missing, self.tmp, self.tmp), {"CYS_PACK_DIR": "unset"})


class Gate(Base):
    def test_body_alone_is_refused_and_nothing_is_written(self):
        before = self.snapshot()
        r = self.run_tool("update", "--name", NAME, "--body", NEW_BODY)
        msg = self.assert_refused(2, r, before, "--desc", "--keep-desc")
        self.assertIn(OLD_BODY, self.card())
        self.assertNotIn(NEW_BODY, self.card())
        control("body alone: rc %d, %d files byte-identical, says: %s" % (r.returncode, len(before), msg))

    def test_desc_with_body_passes_and_moves_both_halves(self):
        r = self.run_tool("update", "--name", NAME, "--desc", NEW_DESC, "--body", NEW_BODY)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(json.loads(r.stdout), {"updated": CARD, "fields": ["description", "body"],
                                                "index_updated": True})
        self.assertIn("description: %s\n" % NEW_DESC, self.card())
        self.assertIn(NEW_BODY, self.card())
        self.assertNotIn(OLD_BODY, self.card())
        self.assertIn(NEW_DESC, self.index())
        self.assertNotIn(OLD_DESC, self.index())
        self.assertEqual(self.rows(), [])
        v = self.run_tool("verify", "--desc-strict")
        self.assertEqual(v.returncode, 0, v.stdout)
        control("--desc with --body: rc 0, card and index both moved, verify --desc-strict rc 0, no opt-out row")

    def test_keep_desc_passes_and_records_that_it_was_used(self):
        index_before = self.index()
        env = dict(self.env, CYS_ROLE="rig-role", CYS_SURFACE_ID="42")
        r = self.run_tool("update", "--name", NAME, "--body", NEW_BODY, "--keep-desc", env=env)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(json.loads(r.stdout), {"updated": CARD, "fields": ["body"], "index_updated": False,
                                                "keep_desc": True})
        self.assertIn(NEW_BODY, self.card())
        self.assertIn("description: %s\n" % OLD_DESC, self.card())
        self.assertEqual(self.index(), index_before)
        rows = self.rows()
        self.assertEqual(len(rows), 1)
        self.assertEqual({k: rows[0][k] for k in ("file", "event", "role", "surface")},
                         {"file": CARD, "event": "keep-desc-attempt", "role": "rig-role", "surface": "42"})
        self.assertRegex(rows[0]["at"], r"^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ$")
        control("--keep-desc: rc 0, index byte-identical, durable row %s" % json.dumps(rows[0]))

    def test_each_use_of_the_opt_out_leaves_its_own_row(self):
        for i in range(3):
            r = self.run_tool("update", "--name", NAME, "--body", "오타 수정 %d" % i, "--keep-desc")
            self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(len(self.rows()), 3)

    def test_an_unknown_seat_is_recorded_as_unknown_and_not_guessed(self):
        r = self.run_tool("update", "--name", NAME, "--body", NEW_BODY, "--keep-desc")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIsNone(self.rows()[0]["role"])
        self.assertIsNone(self.rows()[0]["surface"])

    def test_desc_alone_behaves_as_before(self):
        before_card, before_index = self.card(), self.index()
        r = self.run_tool("update", "--name", NAME, "--desc", NEW_DESC)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(r.stdout, '{"updated": "%s", "fields": ["description"], "index_updated": true}\n' % CARD)
        self.assertEqual(self.card(), before_card.replace(OLD_DESC, NEW_DESC))
        self.assertEqual(self.index(), before_index.replace(OLD_DESC, NEW_DESC))
        self.assertEqual(self.rows(), [])
        self.assertNotIn(LOG, self.snapshot())
        control("--desc alone: rc 0, exact stdout, exact bytes of card and index, no row")

    def test_both_at_once_is_refused(self):
        before = self.snapshot()
        r = self.run_tool("update", "--name", NAME, "--desc", NEW_DESC, "--body", NEW_BODY, "--keep-desc")
        self.assert_refused(2, r, before, "--desc", "--keep-desc")

    def test_keep_desc_alone_has_nothing_to_do(self):
        before = self.snapshot()
        r = self.run_tool("update", "--name", NAME, "--keep-desc")
        self.assertEqual(r.returncode, 2, r.stdout + r.stderr)
        self.assertEqual(self.snapshot(), before)

    def test_an_empty_body_stays_refused_with_the_opt_out(self):
        before = self.snapshot()
        r = self.run_tool("update", "--name", NAME, "--body", "   ", "--keep-desc")
        self.assertEqual(r.returncode, 2, r.stdout + r.stderr)
        self.assertEqual(self.snapshot(), before)

    def test_abbreviating_desc_still_works(self):
        # The opt-out is not named --desc-*, which would have made --des ambiguous and changed existing behaviour.
        r = self.run_tool("update", "--name", NAME, "--des", NEW_DESC)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn(NEW_DESC, self.index())


class Record(Base):
    def test_no_write_when_the_row_cannot_be_written(self):
        os.makedirs(os.path.join(self.mdir, LOG))
        before = self.snapshot()
        r = self.run_tool("update", "--name", NAME, "--body", NEW_BODY, "--keep-desc")
        self.assertEqual(r.returncode, 3, r.stdout + r.stderr)
        self.assertEqual(self.snapshot(), before)
        self.assertIn(OLD_BODY, self.card())
        control("row cannot be written: rc 3, card and index byte-identical")

    def test_a_row_that_landed_before_fsync_failed_is_not_denied(self):
        card, index = self.card(), self.index()
        p, _, _ = self.harness("fsync-fails", "update", "--name", NAME, "--body", NEW_BODY, "--keep-desc")
        out, err = p.communicate(timeout=30)
        self.assertEqual(p.returncode, 3, out + err)
        self.assertEqual(out, "")
        self.assertEqual((self.card(), self.index()), (card, index))
        rows = self.rows()
        self.assertEqual(len(rows), 1, "the injected fault did not land after the append")
        msg = json.loads(err.strip().splitlines()[-1])["error"]
        self.assertNotIn("아무것도 쓰지", msg)
        self.assertIn("카드와 색인은 갱신하지 않았다", msg)
        self.assertIn("남았을 수 있다", msg)
        control("fsync fails after the append: rc 3, card and index unchanged, 1 row present, says: %s" % msg)

    def test_a_row_records_an_attempt_not_a_completion(self):
        card, index = self.card(), self.index()
        p, _, _ = self.harness("card-write-fails", "update", "--name", NAME, "--body", NEW_BODY, "--keep-desc")
        out, err = p.communicate(timeout=30)
        self.assertNotEqual(p.returncode, 0, out + err)
        self.assertEqual(out, "")
        self.assertEqual((self.card(), self.index()), (card, index))
        rows = self.rows()
        self.assertEqual(len(rows), 1)
        self.assertEqual(rows[0]["event"], "keep-desc-attempt")
        control("card write fails after the row: rc %s, no result printed, card unchanged, row reads %s"
                % (p.returncode, rows[0]["event"]))

    def test_readers_of_the_directory_do_not_take_the_log_for_a_card(self):
        r = self.run_tool("update", "--name", NAME, "--body", NEW_BODY, "--keep-desc")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn(LOG, self.snapshot())
        v = json.loads(self.run_tool("verify", "--json", "--desc-strict").stdout)
        self.assertEqual((v["ok"], v["files"], v["problems"], v["desc_problems"]), (True, 1, [], []))
        recent = json.loads(self.run_tool("recent", "--json").stdout)
        self.assertEqual([i["file"] for i in recent["items"]], [CARD])
        health = json.loads(self.run_tool("health", "--json").stdout)
        self.assertEqual(health["files"], 1)
        graph = json.loads(self.run_tool("graph", "--json").stdout)
        self.assertEqual(graph["node_count"], 1)
        control("with a log present: verify files 1, recent 1, health 1, graph 1, none is the log")


class Concurrency(Base):
    def peer(self, *argv):
        r = self.run_tool(*argv)
        self.assertEqual(r.returncode, 0, r.stderr)

    def test_keep_desc_cannot_restore_a_description_a_peer_just_wrote(self):
        # The review's schedule: A pauses just before the lock, B updates the description and returns, A goes on.
        a, mark, go = self.harness("before-lock", "update", "--name", NAME, "--body", NEW_BODY, "--keep-desc")
        self.wait_for(mark, a)
        self.peer("update", "--name", NAME, "--desc", PEER_DESC)
        open(go, "w").close()
        out, err = a.communicate(timeout=30)
        self.assertEqual(a.returncode, 0, out + err)
        self.assertIn("description: %s\n" % PEER_DESC, self.card())
        self.assertIn(NEW_BODY, self.card())
        self.assertIn(PEER_DESC, self.index())
        v = self.run_tool("verify", "--desc-strict")
        self.assertEqual(v.returncode, 0, v.stdout)
        control("review schedule: card holds the peer's description and A's body, index agrees, verify --desc-strict rc 0")

    def test_a_peer_waits_while_a_holder_is_between_read_and_write(self):
        a, mark, go = self.harness("before-card-write", "update", "--name", NAME, "--body", NEW_BODY, "--keep-desc")
        self.wait_for(mark, a)
        b, peer_mark, _ = self.harness("observe-lock-wait", "update", "--name", NAME, "--desc", PEER_DESC)
        self.wait_for(peer_mark + ".waiting", b)
        self.assertFalse(os.path.exists(go), "holder was released before the contention witness")
        self.assertIn(OLD_BODY, self.card())
        self.assertIn(OLD_DESC, self.index())
        open(go, "w").close()
        out, err = a.communicate(timeout=30)
        self.assertEqual(a.returncode, 0, out + err)
        out, err = b.communicate(timeout=30)
        self.assertEqual(b.returncode, 0, out + err)
        self.assertIn("description: %s\n" % PEER_DESC, self.card())
        self.assertIn(NEW_BODY, self.card())
        self.assertIn(PEER_DESC, self.index())
        self.assertEqual(self.run_tool("verify", "--desc-strict").returncode, 0)
        control("holder between read and write: real lock-contention wait witnessed before release; "
                "then both landed, verify --desc-strict rc 0")

    def test_one_update_takes_the_lock_once(self):
        for i, argv in enumerate((["--body", NEW_BODY, "--keep-desc"], ["--desc", NEW_DESC, "--body", NEW_BODY],
                                  ["--desc", PEER_DESC])):
            p, mark, _ = self.harness("count", "update", "--name", NAME, *argv)
            out, err = p.communicate(timeout=30)
            self.assertEqual(p.returncode, 0, out + err)
            with open(mark + ".enters") as f:
                self.assertEqual(f.read(), "1", "read and write must sit under one lock: %r" % (argv,))
            os.unlink(mark + ".enters")
        control("each update form entered the lock exactly once")


if __name__ == "__main__":
    unittest.main()
