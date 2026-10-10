#!/usr/bin/env python3
"""B-070 (0.14.50 · T-50-B2) — the H-SECRET-1 `--all` scan limit is a measured rule, and it still bites.

What is pinned here:
  1. RULE   — `SECRET_SCAN_ALL_TIMEOUT_S == ceil(max(saved --all wall times) x SECRET_SCAN_ALL_FACTOR)`;
              the table below is the measured input (laneB/B070-REPORT.md section 1, every saved CI span).
              The runner's `SECRET_SCAN_ALL_OBSERVED_MAX_S` must equal the table maximum, so the constant
              cannot drift to a bare bigger number without this file changing too.
  2. WIRING — inside `h_secret_1` the `--all` call passes `timeout=SECRET_SCAN_ALL_TIMEOUT_S` and every other
              scan passes `timeout=SECRET_SCAN_ONE_FILE_TIMEOUT_S`; no numeric timeout literal is left there.
  3. BITE   — a deliberately HUNG scanner still ends the specimen as a failure (TimeoutExpired), both on the
              `--all` call and on a one-file probe call. The limit is scaled down in a child process (1 s)
              and the child has an outer guard (OUTER_GUARD_S): if the limit were not applied, the child
              would hang and this test FAILS on the guard instead of hanging the suite.

stdlib only · no network · temp dirs only · user HOME untouched.
"""
import ast
import math
import os
import subprocess
import sys
import tempfile
import textwrap
import unittest

sys.dont_write_bytecode = True

TESTS_DIR = os.path.dirname(os.path.abspath(__file__))
RUNNER = os.path.join(TESTS_DIR, "run_bootstrap_health.py")
PY = sys.executable or "python3"

# Measured table: `secret-scan.sh --all` wall time (seconds), one row per saved CI span
# ('##[group]Run bash scripts/secret-scan.sh --all' -> 'clean (mode=--all, N 파일)').
# (runner image, seconds, files, source under _evidence/, line of the Run line)
ALL_SCAN_TABLE = (
    ("windows-2025-vs2026", 210.5, 1001, "impl-3problems-20260921/release/R1b-07b-windows-build-job-PREV-0.14.39.log", 99),
    ("windows-2025-vs2026", 295.7, 1001, "impl-3problems-20260921/release/R1b-07-windows-build-job.log", 99),
    ("windows-2025-vs2026", 277.6, 1082, "impl-0.14.43-20261003/REL/R1-12-windows-build-55d3d2d6-job.log", 98),
    ("windows-2025-vs2026", 276.3, 1115, "dept5-M1-item2-20261008/F-03-release-run-windows-job.log", 135),
    ("windows-2025-vs2026", 294.8, 1118, "dept5-M1-item1-20261008/INSTR-ci/P3-job-113288549568.log", 98),
    ("windows-2025-vs2026", 289.2, 1118, "dept5-M1-item1-20261008/INSTR-ci/attempt2/P7-job-113308756051.log", 98),
    ("windows-2025-vs2026", 309.0, 1118, "dept5-M1-item1-20261008/INSTR-release/release-attempt1/R2-job-113524459866.log", 135),
    ("windows-2025-vs2026", 228.8, 1121, "dept5-impl-0.14.48-20261009/RELEASE/R1-ci/run-37930930704-job-113821168836.log", 98),
    ("windows-2025-vs2026", 298.2, 1121, "dept5-impl-0.14.48-20261009/RELEASE/R1-ci2/run-37944643366-job-113867659497.log", 98),
    ("windows-2025-vs2026", 307.5, 1121, "dept5-impl-0.14.48-20261009/RELEASE/R2-release/run-37951952464-job-113892711563.log", 135),
    ("windows-2025-vs2026", 297.3, None, "dept5-impl-0.14.49-20261010/ci/windows-build-job-114202929219.log", 98),
    ("windows-2025-vs2026", 296.9, 1122, "dept5-impl-0.14.49-20261010/ci2/windows-build-job-114211312989.log", 98),
    ("windows-2025-vs2026", 311.6, 1122, "dept5-impl-0.14.49-20261010/RELEASE/S2-release-job-114221181479.log", 135),
    ("macos-26-arm64", 41.4, 1118, "dept5-M1-item1-20261008/INSTR-release/release-attempt1/R2-job-113524459648.log", 132),
    ("macos-26-arm64", 55.9, 1118, "dept5-M1-item1-20261008/INSTR-release/release-attempt1/R2-job-113524459855.log", 195),
    ("macos-26-arm64", 48.3, 1118, "dept5-M1-item1-20261008/INSTR-release/release-attempt2/R2-job-113537722297.log", 195),
    ("macos-26-arm64", 32.0, 1121, "dept5-impl-0.14.48-20261009/RELEASE/R2-release/run-37951952464-job-113892711184.log", 195),
    ("macos-26-arm64", 52.6, 1121, "dept5-impl-0.14.48-20261009/RELEASE/R2-release/run-37951952464-job-113892711462.log", 132),
    ("macos-26-arm64", 35.1, 1122, "dept5-impl-0.14.49-20261010/RELEASE/S2-release-job-114221181475.log", 195),
    ("macos-26-arm64", 47.7, 1122, "dept5-impl-0.14.49-20261010/RELEASE/S2-release-job-114221181396.log", 132),
    ("ubuntu-24.04", 11.8, 1001, "impl-3problems-20260921/release/R2-07-pack-artifacts-job.log", 153),
    ("ubuntu-24.04", 15.9, 1001, "impl-3problems-20260921/release/R2b-05-pack-artifacts-job.log", 153),
    ("ubuntu-24.04", 18.7, 1118, "dept5-M1-item1-20261008/INSTR-release/release-attempt2/R2-job-113561721231.log", 152),
    ("ubuntu-24.04", 17.5, 1121, "dept5-impl-0.14.48-20261009/RELEASE/R2-release/run-37951952464-job-113916371815.log", 152),
)
OUTER_GUARD_S = 25          # the child must end long before this; reaching it = the limit did not bite


def _load_runner():
    import importlib.util
    spec = importlib.util.spec_from_file_location("rbh_b070", RUNNER)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def _h_secret_1_ast():
    tree = ast.parse(open(RUNNER, encoding="utf-8").read())
    for node in ast.walk(tree):
        if isinstance(node, ast.FunctionDef) and node.name == "h_secret_1":
            return node
    raise AssertionError("h_secret_1 not found in the runner")


class T1Rule(unittest.TestCase):
    def test_1_limit_is_the_rule_over_the_table(self):
        rbh = _load_runner()
        table_max = max(r[1] for r in ALL_SCAN_TABLE)
        self.assertEqual(rbh.SECRET_SCAN_ALL_OBSERVED_MAX_S, table_max,
                         "the runner's observed maximum must be the table maximum")
        self.assertEqual(rbh.SECRET_SCAN_ALL_FACTOR, 2.0)
        self.assertEqual(rbh.SECRET_SCAN_ALL_TIMEOUT_S, int(math.ceil(table_max * 2.0)))

    def test_1_limit_clears_every_observed_run_by_the_factor(self):
        rbh = _load_runner()
        for os_name, secs, _n, src, line in ALL_SCAN_TABLE:
            self.assertGreaterEqual(rbh.SECRET_SCAN_ALL_TIMEOUT_S, 2.0 * secs, "%s %s:%d" % (os_name, src, line))

    def test_1_limit_is_bounded(self):
        # The protection: a hang must end well inside the release job's 50-minute limit (release.yml:61),
        # leaving room for the build that follows. Pinned: the limit is under 15 minutes.
        rbh = _load_runner()
        self.assertLess(rbh.SECRET_SCAN_ALL_TIMEOUT_S, 15 * 60)

    def test_1_one_file_limit_unchanged(self):
        rbh = _load_runner()
        self.assertEqual(rbh.SECRET_SCAN_ONE_FILE_TIMEOUT_S, 300)

    def test_1_limit_is_written_as_the_rule(self):
        # A bare literal - even one equal to today's value - would not follow a new table. The module-level
        # assignment must be computed from the two named inputs.
        tree = ast.parse(open(RUNNER, encoding="utf-8").read())
        hits = [n for n in tree.body if isinstance(n, ast.Assign)
                and any(isinstance(t, ast.Name) and t.id == "SECRET_SCAN_ALL_TIMEOUT_S" for t in n.targets)]
        self.assertEqual(len(hits), 1, "exactly one module-level assignment expected")
        names = {n.id for n in ast.walk(hits[0].value) if isinstance(n, ast.Name)}
        self.assertTrue({"SECRET_SCAN_ALL_OBSERVED_MAX_S", "SECRET_SCAN_ALL_FACTOR"} <= names,
                        "the limit is not computed from the rule inputs: names=%r" % sorted(names))


class T2Wiring(unittest.TestCase):
    def test_2_all_call_uses_the_all_limit_and_others_the_one_file_limit(self):
        fn = _h_secret_1_ast()
        scan_calls, run_calls = [], []
        for node in ast.walk(fn):
            if isinstance(node, ast.Call) and isinstance(node.func, ast.Name):
                if node.func.id == "_scan":
                    scan_calls.append(node)
                elif node.func.id == "_run":
                    run_calls.append(node)
        all_calls = [c for c in scan_calls
                     if c.args and isinstance(c.args[0], ast.Constant) and c.args[0].value == "--all"]
        self.assertEqual(len(all_calls), 1, "exactly one --all scan expected")
        kw = {k.arg: k.value for k in all_calls[0].keywords}
        self.assertIn("timeout", kw, "--all must pass its own timeout")
        self.assertIsInstance(kw["timeout"], ast.Name)
        self.assertEqual(kw["timeout"].id, "SECRET_SCAN_ALL_TIMEOUT_S")
        self.assertEqual(len(run_calls), 1, "one _run call inside _scan expected")
        rkw = {k.arg: k.value for k in run_calls[0].keywords}
        self.assertIsInstance(rkw.get("timeout"), ast.Name, "the _run timeout must be a name, not a literal")

    def test_2_scan_default_is_the_one_file_limit(self):
        fn = _h_secret_1_ast()
        inner = [n for n in ast.walk(fn) if isinstance(n, ast.FunctionDef) and n.name == "_scan"]
        self.assertEqual(len(inner), 1)
        a = inner[0].args
        names = [x.arg for x in a.kwonlyargs]
        self.assertIn("timeout", names)
        d = a.kw_defaults[names.index("timeout")]
        self.assertIsInstance(d, ast.Name)
        self.assertEqual(d.id, "SECRET_SCAN_ONE_FILE_TIMEOUT_S")

    def test_2_no_numeric_timeout_literal_in_h_secret_1(self):
        fn = _h_secret_1_ast()
        for node in ast.walk(fn):
            if isinstance(node, ast.keyword) and node.arg == "timeout":
                self.assertNotIsInstance(node.value, ast.Constant,
                                         "numeric timeout literal at line %d" % node.value.lineno)


# The child: load the runner, point REPO_DIR at a fake checkout whose scanner HANGS, scale the
# limit down to 1 s, run the specimen, print the exception class. `exec sleep` so that the kill
# on timeout takes the sleeping process itself (no orphan left).
_CHILD = textwrap.dedent(r'''
    import importlib.util, os, sys, time
    sys.dont_write_bytecode = True
    spec = importlib.util.spec_from_file_location("rbh_child", sys.argv[1])
    m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m)
    m.REPO_DIR = sys.argv[2]
    which = sys.argv[3]
    if which == "all":
        m.SECRET_SCAN_ALL_TIMEOUT_S = 1
    else:
        m.SECRET_SCAN_ONE_FILE_TIMEOUT_S = 1
    t = time.monotonic()
    try:
        m.h_secret_1()
        print("RESULT no-exception")
    except Exception as e:
        print("RESULT %s %.1f" % (type(e).__name__, time.monotonic() - t))
''')


def _fake_repo(root, mode):
    open(os.path.join(root, "Cargo.toml"), "w").close()
    os.makedirs(os.path.join(root, "scripts"))
    p = os.path.join(root, "scripts", "secret-scan.sh")
    with open(p, "w", encoding="utf-8", newline="\n") as f:
        if mode == "all":           # the --all call hangs
            f.write("#!/bin/bash\nexec sleep 60\n")
        else:                       # --all is clean and fast, every one-file call hangs
            f.write('#!/bin/bash\nif [ "$1" = "--all" ]; then echo "x clean (mode=--all, 900 파일)"; exit 0; fi\n'
                    "exec sleep 60\n")
    os.chmod(p, 0o755)


class T3HungScanStillFails(unittest.TestCase):
    def _run(self, mode):
        with tempfile.TemporaryDirectory(prefix="b070-hung-") as root:
            _fake_repo(root, mode)
            try:
                r = subprocess.run([PY, "-c", _CHILD, RUNNER, root, mode], capture_output=True, text=True,
                                   encoding="utf-8", errors="replace", timeout=OUTER_GUARD_S)
            except subprocess.TimeoutExpired:
                self.fail("hung scan (%s) was NOT cut by the limit: child still running after %d s"
                          % (mode, OUTER_GUARD_S))
            line = [l for l in r.stdout.splitlines() if l.startswith("RESULT ")]
            self.assertTrue(line, "no RESULT line: rc=%s err=%s" % (r.returncode, r.stderr[-600:]))
            parts = line[-1].split()
            self.assertEqual(parts[1], "TimeoutExpired", line[-1])
            self.assertLess(float(parts[2]), 10.0, line[-1])

    def test_3a_hung_all_scan_fails(self):
        self._run("all")

    def test_3b_hung_probe_scan_fails(self):
        self._run("probe")


if __name__ == "__main__":
    if os.name == "nt":
        # POSIX shell fixture (bash + exec sleep); the Windows runner meets the real limit in H-SECRET-1 itself.
        print("SKIP: POSIX-only fixture")
        sys.exit(0)
    unittest.main(verbosity=2)
