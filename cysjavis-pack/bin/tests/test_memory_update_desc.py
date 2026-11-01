#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_memory_update_desc.py — `update --body` may not leave the index description behind (CEO A8).

Seats read the INDEX, so a body changed under an old description is a fact recall cannot see. The tool knew
(`index_updated: false`) and said so on stdout, where nobody reads it. The gate fails closed: --body needs --desc, or
--keep-desc saying the description is deliberately unchanged. --keep-desc exists so a typo fix is not blocked, and it
leaves a durable row, because stdout is the channel that hid this defect.

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
        self.assertEqual({k: rows[0][k] for k in ("file", "flag", "role", "surface")},
                         {"file": CARD, "flag": "keep-desc", "role": "rig-role", "surface": "42"})
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
        self.assert_refused(3, r, before, "--keep-desc")
        self.assertIn(OLD_BODY, self.card())
        control("row cannot be written: rc 3, card and index byte-identical")

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


if __name__ == "__main__":
    unittest.main()
