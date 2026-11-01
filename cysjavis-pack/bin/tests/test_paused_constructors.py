#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_paused_constructors.py — the kill-switch path list, built in lockstep (0.14.57 P1 part 4).

The tools that build the two-path list keep no shared import, because packs can tear mid-update and a kill-switch path
must still run. So this test finds them by a predicate over pack/bin and asserts they agree. ⛔It does not name the
tools: an enumeration always lags, and the docstring that once listed them proved it.

    python3 -B cysjavis-pack/bin/tests/test_paused_constructors.py -v

Each `[control]` line prints what was measured.
"""
import ast
import glob
import json
import os
import shutil
import subprocess
import sys
import tempfile
import textwrap
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(HERE)
PY = sys.executable
FILE = "AUTOPILOT_PAUSED"
SHAPE = [["<pack>", "PAUSED_BASENAME"], ["os.path.expanduser('~')", "'_round'", "PAUSED_BASENAME"]]
PACK_EXPR = {"pack_dir()", "PACK_DIR"}
DELEGATE_CALLS = {"paused_evidence", "paused_paths"}


def control(msg):
    print("[control] " + msg, flush=True)


def is_paused_join(node):
    if not (isinstance(node, ast.Call) and node.args):
        return False
    if getattr(node.func, "attr", getattr(node.func, "id", "")) != "join":
        return False
    last = node.args[-1]
    if isinstance(last, ast.Name):
        return last.id == "PAUSED_BASENAME"
    if isinstance(last, ast.Attribute):
        return last.attr == "PAUSED_BASENAME"
    return isinstance(last, ast.Constant) and FILE in str(last.value)


def own_nodes(fn):
    """Nodes of fn, leaving out functions nested inside it."""
    stack = list(ast.iter_child_nodes(fn))
    while stack:
        n = stack.pop()
        yield n
        if not isinstance(n, (ast.FunctionDef, ast.AsyncFunctionDef)):
            stack.extend(ast.iter_child_nodes(n))


def shape_of(join):
    return ["<pack>" if ast.unparse(a) in PACK_EXPR else ast.unparse(a) for a in join.args]


def census(bin_dir):
    """Return (constructors, delegates, strays), each a sorted list. Self-test code is out: it plants files."""
    constructors, delegates, strays = [], [], []
    for path in sorted(glob.glob(os.path.join(bin_dir, "*.py"))):
        mod = os.path.basename(path)[:-3]
        with open(path, encoding="utf-8") as f:
            tree = ast.parse(f.read())
        for fn in ast.walk(tree):
            if not isinstance(fn, ast.FunctionDef) or "test" in fn.name:
                continue
            nodes = list(own_nodes(fn))
            joins = [n for n in nodes if is_paused_join(n)]
            if not joins:
                continue
            lists = [n for n in nodes if isinstance(n, ast.List) and any(is_paused_join(e) for e in n.elts)]
            called = {getattr(n.func, "attr", getattr(n.func, "id", "")) for n in nodes if isinstance(n, ast.Call)}
            if lists:
                constructors.append((mod, fn.name, [shape_of(e) for e in lists[0].elts]))
            elif called & DELEGATE_CALLS:
                delegates.append((mod, fn.name))
            else:
                strays.append((mod, fn.name))
    return constructors, delegates, strays


def disagreements(constructors):
    return [c[:2] for c in constructors if c[2] != SHAPE]


class Predicate(unittest.TestCase):
    def test_every_constructor_found_by_the_predicate_has_the_one_shape(self):
        constructors, delegates, strays = census(BIN)
        control("P1 derived constructors: %s" % [c[:2] for c in constructors])
        control("P1 derived delegates: %s, strays: %s" % (delegates, strays))
        self.assertGreaterEqual(len(constructors), 2, "the predicate found almost nothing, so it proves nothing")
        self.assertEqual(disagreements(constructors), [])
        self.assertEqual(strays, [])

    def synth(self, source):
        d = tempfile.mkdtemp(prefix="paused-census-")
        self.addCleanup(shutil.rmtree, d, True)
        shutil.copy(os.path.join(BIN, "javis_wakeup.py"), d)
        with open(os.path.join(d, "javis_new_tool.py"), "w", encoding="utf-8") as f:
            f.write(textwrap.dedent(source))
        return d

    def test_positive_control_a_new_tool_on_the_old_formula_fails(self):
        d = self.synth("""
            import os
            def paused_paths():
                return [os.path.join(pack_dir(), PAUSED_BASENAME),
                        os.path.join(PROJECT, "_round", PAUSED_BASENAME)]
            """)
        bad = disagreements(census(d)[0])
        control("P1 predicate control, a fifth tool on the old formula: caught=%s" % bad)
        self.assertEqual(bad, [("javis_new_tool", "paused_paths")])

    def test_positive_control_a_third_entry_fails(self):
        d = self.synth("""
            import os
            def kill_files():
                return [os.path.join(pack_dir(), PAUSED_BASENAME),
                        os.path.join(os.path.expanduser("~"), "_round", PAUSED_BASENAME),
                        os.path.join(PROJECT, "_round", PAUSED_BASENAME)]
            """)
        bad = disagreements(census(d)[0])
        control("P1 predicate control, a third entry under another name: caught=%s" % bad)
        self.assertEqual(bad, [("javis_new_tool", "kill_files")])

    def test_positive_control_paths_built_without_a_list_are_strays(self):
        d = self.synth("""
            import os
            def am_i_stopped():
                a = os.path.join(ROOT, "_round", "AUTOPILOT_PAUSED")
                return os.path.exists(a)
            """)
        strays = census(d)[2]
        control("P1 predicate control, a literal path with no list: stray=%s" % strays)
        self.assertEqual(strays, [("javis_new_tool", "am_i_stopped")])

    def test_an_empty_bin_finds_nothing_and_the_guard_would_fail(self):
        d = tempfile.mkdtemp(prefix="paused-census-")
        self.addCleanup(shutil.rmtree, d, True)
        self.assertEqual(census(d), ([], [], []))


class DenyText(unittest.TestCase):
    def test_guard_deny_messages_name_the_files_really_read(self):
        with open(os.path.join(os.path.dirname(BIN), "hooks", "guard.sh"), encoding="utf-8") as f:
            text = f.read()
        old, new = text.count("<프로젝트루트>/_round/" + FILE), text.count("~/_round/" + FILE)
        control("P1R guard.sh deny text: old path=%d, ~/_round path=%d" % (old, new))
        self.assertEqual(old, 0)
        self.assertGreaterEqual(new, 1)


class Lane(unittest.TestCase):
    """A non-base lane after its export: PROJECT, JAVIS_ROOT and cwd all sit in the dept's pack, not in $HOME."""
    DEPT = "dept-7"

    def setUp(self):
        self.home = tempfile.mkdtemp(prefix="paused-lane-")
        self.addCleanup(shutil.rmtree, self.home, True)
        self.pack = os.path.join(self.home, ".cys", "pack-dept-" + self.DEPT)
        self.other = os.path.join(self.home, ".cys", "pack-dept-dept-8")
        os.makedirs(os.path.join(self.pack, "_round"))
        os.makedirs(self.other)
        os.makedirs(os.path.join(self.home, "_round"))
        self.local = os.path.join(self.home, "AppData", "Local")
        os.makedirs(self.local)
        self.fleet = os.path.join(self.home, "_round", FILE)
        self.lane_file = os.path.join(self.pack, FILE)

    def env(self):
        env = {k: v for k, v in os.environ.items()
               if not k.startswith(("CYS_", "JAVIS_", "AITERM_")) and k != "CLAUDE_CONFIG_DIR"}
        env.update({"HOME": self.home, "USERPROFILE": self.home, "LOCALAPPDATA": self.local, "CYS_PACK_DIR": self.pack, "CYS_PROJECT_ROOT": self.pack, "JAVIS_ROOT": self.pack,
                    "CYS_SOCKET": os.path.join(self.home, ".local", "state", "cys-dept-" + self.DEPT, "cys.sock"),
                    "CYS_NO_AUTOSTART": "1", "PATH": "/usr/bin:/bin"})
        return env

    def ask(self, mod, fn):
        code = ("import importlib, json, sys\n"
                "sys.path.insert(0, sys.argv[1])\n"
                "m = importlib.import_module(sys.argv[2])\n"
                "print('@@' + json.dumps(getattr(m, sys.argv[3])()))\n")
        r = subprocess.run([PY, "-B", "-c", code, BIN, mod, fn], capture_output=True, text=True, cwd=self.pack,
                           env=self.env(), timeout=60)
        self.assertEqual(r.returncode, 0, r.stderr[-400:])
        return json.loads([x for x in r.stdout.splitlines() if x.startswith("@@")][-1][2:])

    def hits(self, mod, fn):
        paths = self.ask(mod, fn)
        rig = os.path.realpath(self.home) + os.sep
        self.assertEqual([p for p in paths if not os.path.realpath(p).startswith(rig)], [], "outside the rig")
        return [p for p in paths if os.path.exists(p)]

    def test_fleet_file_pauses_a_non_base_lane_per_constructor(self):
        constructors = census(BIN)[0]
        self.assertGreaterEqual(len(constructors), 2)
        with open(self.fleet, "w", encoding="utf-8") as f:
            f.write("")
        for mod, fn, _shape in constructors:
            got = self.hits(mod, fn)
            control("P1 fleet file, non-base lane, %s.%s: hits=%s" % (mod, fn, [os.path.relpath(p, self.home) for p in got]))
            self.assertEqual(got, [self.fleet], mod)

    def test_lane_file_pauses_its_lane_and_no_other(self):
        constructors = census(BIN)[0]
        with open(self.lane_file, "w", encoding="utf-8") as f:
            f.write("")
        with_other = dict(self.env(), CYS_PACK_DIR=self.other)
        for mod, fn, _shape in constructors:
            mine = self.hits(mod, fn)
            r = subprocess.run([PY, "-B", "-c",
                                "import importlib, json, os, sys\nsys.path.insert(0, sys.argv[1])\n"
                                "m = importlib.import_module(sys.argv[2])\n"
                                "print('@@' + json.dumps([p for p in getattr(m, sys.argv[3])() if os.path.exists(p)]))\n",
                                BIN, mod, fn], capture_output=True, text=True, cwd=self.pack, env=with_other, timeout=60)
            self.assertEqual(r.returncode, 0, r.stderr[-400:])
            theirs = json.loads([x for x in r.stdout.splitlines() if x.startswith("@@")][-1][2:])
            control("P1 lane file, %s: this lane hits=%d, another lane hits=%d" % (mod, len(mine), len(theirs)))
            self.assertEqual((mine, theirs), ([self.lane_file], []), mod)

    def test_absent_files_pause_nothing(self):
        for mod, fn, _shape in census(BIN)[0]:
            got = self.hits(mod, fn)
            control("P1 no files, %s: hits=%d" % (mod, len(got)))
            self.assertEqual(got, [], mod)

    def test_a_project_file_outside_home_no_longer_pauses(self):
        """The consequence of the literal entry: <project>/_round/AUTOPILOT_PAUSED is not a kill-switch path any more."""
        with open(os.path.join(self.pack, "_round", FILE), "w", encoding="utf-8") as f:
            f.write("")
        for mod, fn, _shape in census(BIN)[0]:
            got = self.hits(mod, fn)
            control("P1 file at <project>/_round, %s: hits=%d" % (mod, len(got)))
            self.assertEqual(got, [], mod)

    def test_delegates_only_add_triggers(self):
        delegates = census(BIN)[1]
        self.assertGreaterEqual(len(delegates), 1)
        for mod, fn in delegates:
            paused, _hits = self.ask(mod, fn)
            self.assertIs(paused, False, mod)
            with open(self.fleet, "w", encoding="utf-8") as f:
                f.write("")
            fleet_paused, _hits = self.ask(mod, fn)
            os.remove(self.fleet)
            with open(os.path.join(self.pack, "_round", FILE), "w", encoding="utf-8") as f:
                f.write("")
            root_paused, _hits = self.ask(mod, fn)
            os.remove(os.path.join(self.pack, "_round", FILE))
            control("P1 delegate %s.%s: no files=%s, fleet file=%s, its own root's file=%s" % (
                mod, fn, paused, fleet_paused, root_paused))
            self.assertEqual((fleet_paused, root_paused), (True, True), mod)

    def test_wakeup_refuses_delivery_in_a_non_base_lane(self):
        with open(self.fleet, "w", encoding="utf-8") as f:
            f.write("")
        r = subprocess.run([PY, "-B", os.path.join(BIN, "javis_wakeup.py"), "drain", "--deliver"], capture_output=True,
                           text=True, cwd=self.pack, env=self.env(), timeout=60)
        control("P1 real wakeup drain under the fleet file: rc=%s" % r.returncode)
        self.assertEqual(r.returncode, 4, r.stderr[-300:])


if __name__ == "__main__":
    unittest.main(verbosity=2)
