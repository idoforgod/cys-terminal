"""Hermetic controls for scripts/check_wallclock_premise.py - the wall-clock premise gate (0.14.56 SLEEP-PREMISE-FIX).

Why this exists: the gate is a port of the 0.14.56-R2 census classifier. A gate that is only ever seen passing cannot be
told apart from one that always passes (this repo's rule; see the version-check negative control in release.yml), so
every branch is measured here on planted trees: a new premise fails (exit 1) and names the test, a GUARD shape and an
accepted annotation pass (exit 0), a fixed-but-still-baselined test fails as stale (exit 1), and a tree it cannot judge
is exit 2, never a pass. Each planted failure has a passing control that differs only in the planted construct, so a
red result is caused by that construct and not by something else in the tree.

The trees are temp dirs (no network, no git). The last case runs the real repo tree against the committed baseline.

    python3 scripts/tests/test_check_wallclock_premise.py
"""
import os
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
# Mutation testing: WALLCLOCK_PREMISE_GATE_UNDER_TEST=<mutant path> runs the same cases against that script instead.
SCRIPT = os.environ.get("WALLCLOCK_PREMISE_GATE_UNDER_TEST") or os.path.normpath(os.path.join(HERE, "..", "check_wallclock_premise.py"))
HEADER = "# test baseline\n"

RS = "src/bin/cysd/planted_tests.rs"
PY = "cysjavis-pack/tests/test_planted.py"
BENIGN = "src/bin/cysd/benign_tests.rs"

# A timing token that is not a premise (CLOCK-ONLY), so every tree has hits and the population is never empty.
BENIGN_SRC = """\
#[cfg(test)]
mod tests {
    #[test]
    fn reads_the_clock_only() {
        let started = std::time::Instant::now();
        let _ = started;
        assert_eq!(1 + 1, 2);
    }
}
"""

# P1: a bare sleep assumed long enough for another thread to act, then an assert that only holds under that assumption.
P1_ASSERT = "assert!(flag.load(Ordering::SeqCst));"
P1_SRC = """\
#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    #[test]
    fn planted_sleep_then_assert() {
        let flag = Arc::new(AtomicBool::new(false));
        let f2 = flag.clone();
        std::thread::spawn(move || f2.store(true, Ordering::SeqCst));
        std::thread::sleep(Duration::from_millis(50));
        ASSERT_LINE
    }
}
"""
P1_ID = RS + "::tests::planted_sleep_then_assert"

# The same test with the premise removed: the thread is joined (a hand-shake), only a clock read remains.
P1_FIXED_SRC = """\
#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    #[test]
    fn planted_sleep_then_assert() {
        let started = std::time::Instant::now();
        let flag = Arc::new(AtomicBool::new(false));
        let f2 = flag.clone();
        std::thread::spawn(move || f2.store(true, Ordering::SeqCst)).join().unwrap();
        assert!(flag.load(Ordering::SeqCst));
        let _ = started;
    }
}
"""

P2_SRC = """\
import threading
import time
import unittest


class Planted(unittest.TestCase):
    def test_planted_sleep_then_assert(self):
        box = []
        threading.Thread(target=box.append, args=(1,)).start()
        time.sleep(0.2)
        self.assertEqual(box, [1])
"""
P2_ID = PY + "::Planted.test_planted_sleep_then_assert"

# P3: the test has no timing token of its own; its helper waits 300 ms on another thread (PREMISE-DEADLINE, < 1 s).
P3_SRC = """\
#[cfg(test)]
mod tests {
    use std::time::Duration;

    fn settle(rx: &std::sync::mpsc::Receiver<u32>) -> u32 {
        rx.recv_timeout(Duration::from_millis(300)).expect("worker answered")
    }

    #[test]
    fn planted_calls_a_premise_helper() {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || tx.send(7).unwrap());
        CALL
    }
}
"""
P3_CALL = "assert_eq!(settle(&rx), 7);"
P3_NO_CALL = "assert_eq!(rx.recv().unwrap(), 7);"
P3_ID = RS + "::tests::planted_calls_a_premise_helper"

# N1: a 60 s deadline on a child thread's natural completion; the sleep only paces the loop (GUARD).
N1_SRC = """\
#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    #[test]
    fn child_finishes_within_a_hang_guard() {
        let done = Arc::new(AtomicBool::new(false));
        let d2 = done.clone();
        let child = std::thread::spawn(move || d2.store(true, Ordering::SeqCst));
        let deadline = Instant::now() + Duration::from_secs(60);
        while !done.load(Ordering::SeqCst) {
            assert!(Instant::now() < deadline, "child hung for 60 s");
            std::thread::sleep(Duration::from_millis(20));
        }
        child.join().unwrap();
    }
}
"""
N1_ID = RS + "::tests::child_finishes_within_a_hang_guard"

# The census rule for the same loop on a child PROCESS: try_wait names OS-teardown state, so it is PREMISE-DEADLINE
# (census §6 audit item A54 disagreed and said GUARD; the census classifier, which this gate ports, says DEADLINE).
N1_TRY_WAIT_SRC = """\
#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    #[test]
    fn child_process_finishes_within_a_hang_guard() {
        let mut child = std::process::Command::new("true").spawn().expect("spawn");
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            if child.try_wait().expect("try_wait").is_some() {
                break;
            }
            assert!(Instant::now() < deadline, "child hung for 60 s");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}
"""


def line_no(src, needle):
    for i, l in enumerate(src.split("\n"), 1):
        if needle in l:
            return i
    raise AssertionError("needle not in source: %r" % needle)


class WallclockPremiseGate(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="wallclock-gate-")
        self.addCleanup(shutil.rmtree, self.tmp, ignore_errors=True)
        self.root = os.path.join(self.tmp, "repo")
        self.baseline = os.path.join(self.tmp, "baseline.txt")
        self.write(BENIGN, BENIGN_SRC)

    def write(self, rel, text):
        path = os.path.join(self.root, rel)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w", encoding="utf-8", newline="\n") as f:
            f.write(text)

    def set_baseline(self, *ids):
        with open(self.baseline, "w", encoding="utf-8", newline="\n") as f:
            f.write(HEADER + "".join(i + "\n" for i in ids))

    def run_gate(self, *extra, root=True):
        argv = [sys.executable, SCRIPT] + (["--root", self.root, "--baseline", self.baseline] if root else []) + list(extra)
        r = subprocess.run(argv, capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=600)
        out = r.stdout + r.stderr
        self.assertTrue(r.stdout.rstrip("\n").split("\n")[-1].startswith("summary: "), "the last line must be the summary:\n" + out)
        return r.returncode, out

    def new_lines(self, out):
        return [l for l in out.split("\n") if l.startswith("NEW ")]

    # -- the clean control every planted case is measured against --
    def test_clean_tree_passes_with_exit_0(self):
        self.set_baseline()
        rc, out = self.run_gate()
        self.assertEqual(rc, 0, out)
        self.assertIn("premise tests 0 (baselined 0, new 0); stale baseline entries 0", out)
        self.assertIn("PASS (exit 0)", out)

    # -- exit 1: a new premise test, named, with file:line, class and the quoted deciding line --
    def test_p1_rust_sleep_then_assert_fails_and_is_named(self):
        src = P1_SRC.replace("ASSERT_LINE", P1_ASSERT)
        self.write(RS, src)
        self.set_baseline()
        rc, out = self.run_gate()
        self.assertEqual(rc, 1, out)
        new = self.new_lines(out)
        self.assertEqual(len(new), 1, out)
        self.assertIn(P1_ID, new[0])
        self.assertIn("%s:%d" % (RS, line_no(src, P1_ASSERT)), new[0], "file:line must point at the deciding assert")
        self.assertIn("PREMISE-SLEEP", new[0])
        self.assertIn('"%s"' % P1_ASSERT, new[0], "the deciding line must be quoted")
        self.assertIn("bare sleep at :%d (50.0 ms)" % line_no(src, "thread::sleep"), new[0])
        self.assertIn("premise tests 1 (baselined 0, new 1)", out)
        # control: the same tree with that one id baselined passes, so the failure above was this test and nothing else
        self.set_baseline(P1_ID)
        rc, out = self.run_gate()
        self.assertEqual(rc, 0, out)
        self.assertIn("premise tests 1 (baselined 1, new 0)", out)

    def test_p2_python_sleep_then_assert_fails_and_is_named(self):
        self.write(PY, P2_SRC)
        self.set_baseline()
        rc, out = self.run_gate()
        self.assertEqual(rc, 1, out)
        new = self.new_lines(out)
        self.assertEqual(len(new), 1, out)
        self.assertIn(P2_ID, new[0])
        self.assertIn("%s:%d" % (PY, line_no(P2_SRC, "self.assertEqual")), new[0])
        self.assertIn("PREMISE-SLEEP", new[0])
        self.assertIn("bare sleep at :%d (200.0 ms)" % line_no(P2_SRC, "time.sleep"), new[0])
        # control: without the sleep the same test is not a premise
        self.write(PY, P2_SRC.replace("        time.sleep(0.2)\n", ""))
        rc, out = self.run_gate()
        self.assertEqual(rc, 0, out)

    def test_p3_rust_test_inheriting_a_premise_helper_fails(self):
        src = P3_SRC.replace("CALL", P3_CALL)
        self.write(RS, src)
        self.set_baseline()
        rc, out = self.run_gate()
        self.assertEqual(rc, 1, out)
        new = self.new_lines(out)
        self.assertEqual(len(new), 1, out)
        self.assertIn(P3_ID, new[0])
        self.assertIn("INHERITED:PREMISE-DEADLINE via %s::tests::settle" % RS, new[0])
        self.assertIn("%s:%d" % (RS, line_no(src, "recv_timeout")), new[0], "file:line must point at the helper's deciding line")
        self.assertIn("bound 300 ms < 1 s", new[0])
        # control: the helper alone (no test calls it) fails nothing - the failure is the inheritance through the call
        self.write(RS, P3_SRC.replace("CALL", P3_NO_CALL))
        rc, out = self.run_gate()
        self.assertEqual(rc, 0, out)
        self.assertNotIn("NEW ", out)

    # -- exit 0: a GUARD shape is not a premise --
    def test_n1_sixty_second_guard_on_a_child_thread_passes(self):
        self.write(RS, N1_SRC)
        self.set_baseline()
        rc, out = self.run_gate()
        self.assertEqual(rc, 0, out)
        self.assertNotIn(N1_ID, out)
        self.assertIn("premise tests 0 (baselined 0, new 0)", out)
        # positive control on the same loop: shrink the bound to 500 ms and the guard becomes a premise (< 1 s)
        self.write(RS, N1_SRC.replace("Duration::from_secs(60)", "Duration::from_millis(500)"))
        rc, out = self.run_gate()
        self.assertEqual(rc, 1, out)
        self.assertIn(N1_ID, out)
        self.assertIn("PREMISE-DEADLINE", out)

    def test_n1_try_wait_on_a_child_process_is_a_premise_by_the_census_rule(self):
        self.write(RS, N1_TRY_WAIT_SRC)
        self.set_baseline()
        rc, out = self.run_gate()
        self.assertEqual(rc, 1, out)
        self.assertIn("awaits OS-teardown state", out)

    # -- the escape annotation --
    def test_n2_annotation_with_a_reason_is_accepted_printed_and_counted(self):
        reason = "the store happens-before the join in the fixture; the sleep is not load-bearing"
        src = P1_SRC.replace("ASSERT_LINE", P1_ASSERT + "  // wallclock: guard " + reason)
        self.write(RS, src)
        self.set_baseline()
        rc, out = self.run_gate()
        self.assertEqual(rc, 0, out)
        acc = [l for l in out.split("\n") if l.startswith("ACCEPTED ")]
        self.assertEqual(len(acc), 1, out)
        self.assertIn("%s:%d" % (RS, line_no(src, P1_ASSERT)), acc[0])
        self.assertIn(P1_ID, acc[0])
        self.assertIn(reason, acc[0])
        self.assertIn("accepted annotations 1", out)
        # an accepted test must not also sit in the baseline: one exemption per test
        self.set_baseline(P1_ID)
        rc, out = self.run_gate()
        self.assertEqual(rc, 1, out)
        self.assertIn("STALE", out)
        self.assertIn("accepted by an annotation", out)

    def test_n2_annotation_without_a_reason_is_rejected(self):
        for suffix in ("  // wallclock: guard", "  // wallclock: guard   "):
            with self.subTest(suffix=suffix):
                src = P1_SRC.replace("ASSERT_LINE", P1_ASSERT + suffix)
                self.write(RS, src)
                self.set_baseline()
                rc, out = self.run_gate()
                self.assertEqual(rc, 1, out)
                new = self.new_lines(out)
                self.assertEqual(len(new), 1, out)
                self.assertIn(P1_ID, new[0])
                self.assertIn("no reason: not accepted", new[0])
                self.assertIn("accepted annotations 0; rejected annotations 1", out)

    def test_n2_annotation_off_the_deciding_line_is_not_accepted(self):
        src = P1_SRC.replace("ASSERT_LINE", P1_ASSERT).replace(
            "std::thread::sleep(Duration::from_millis(50));", "std::thread::sleep(Duration::from_millis(50)); // wallclock: guard on the sleep line")
        self.write(RS, src)
        self.set_baseline()
        rc, out = self.run_gate()
        self.assertEqual(rc, 1, out)
        self.assertIn(P1_ID, out)
        self.assertIn("accepted annotations 0", out)

    # -- exit 1: stale baseline entries (the ratchet) --
    def test_s_baselined_test_whose_premise_is_removed_is_stale(self):
        self.write(RS, P1_SRC.replace("ASSERT_LINE", P1_ASSERT))
        self.set_baseline(P1_ID)
        rc, out = self.run_gate()
        self.assertEqual(rc, 0, out)   # control: baselined premise passes
        self.write(RS, P1_FIXED_SRC)
        rc, out = self.run_gate()
        self.assertEqual(rc, 1, out)
        stale = [l for l in out.split("\n") if l.startswith("STALE ")]
        self.assertEqual(len(stale), 1, out)
        self.assertIn(P1_ID, stale[0])
        self.assertIn("baseline.txt:2", stale[0], "file:line must point at the baseline entry")
        self.assertIn("no longer a premise (now CLOCK-ONLY", stale[0])
        self.assertIn("remove from baseline", stale[0])
        self.assertIn("stale baseline entries 1", out)

    def test_s_baselined_test_that_no_longer_exists_is_stale(self):
        self.set_baseline(P1_ID)
        rc, out = self.run_gate()
        self.assertEqual(rc, 1, out)
        self.assertIn("STALE ", out)
        self.assertIn("no such test any more", out)

    def test_write_baseline_round_trip(self):
        self.write(RS, P1_SRC.replace("ASSERT_LINE", P1_ASSERT))
        self.write(PY, P2_SRC)
        out_file = os.path.join(self.tmp, "written.txt")
        rc, out = self.run_gate("--write-baseline", out_file)
        self.assertEqual(rc, 0, out)
        with open(out_file, encoding="utf-8") as f:
            lines = f.read().split("\n")
        self.assertTrue(lines[0].startswith("#"))
        self.assertTrue(any("--write-baseline" in l for l in lines if l.startswith("#")), "the header must say how to regenerate")
        self.assertEqual([l for l in lines if l and not l.startswith("#")], sorted([P1_ID, P2_ID]))
        shutil.copy(out_file, self.baseline)
        rc, out = self.run_gate()
        self.assertEqual(rc, 0, out)

    # -- exit 2: cannot judge (never a pass) --
    def test_e_missing_baseline_is_exit_2(self):
        rc, out = self.run_gate()
        self.assertEqual(rc, 2, out)
        self.assertIn("cannot judge", out)
        self.assertIn("baseline missing or unreadable", out)
        self.assertNotIn("PASS", out)

    def test_e_unparsable_source_is_exit_2(self):
        self.set_baseline()
        self.write(PY, "def test_broken(:\n    pass\n")
        rc, out = self.run_gate()
        self.assertEqual(rc, 2, out)
        self.assertIn("unparsable source", out)

    def test_e_empty_population_is_exit_2(self):
        self.set_baseline()
        os.remove(os.path.join(self.root, BENIGN))
        rc, out = self.run_gate()
        self.assertEqual(rc, 2, out)
        self.assertIn("empty population", out)

    # -- the real tree against the committed baseline --
    def test_t_real_repo_tree_with_the_committed_baseline_passes(self):
        rc, out = self.run_gate(root=False)
        self.assertEqual(rc, 0, out)
        self.assertIn("PASS (exit 0)", out)
        self.assertIn("new 0); stale baseline entries 0", out)


if __name__ == "__main__":
    unittest.main()
