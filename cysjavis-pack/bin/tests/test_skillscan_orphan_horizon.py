#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""P4 C-14 (B-490): how many skillscan workers outlive the preflight deadline.

The row asks for a number, not a repair. What the record said and what the source
shows differ in one way that decides the design: the product RE-MEASURES survivors
only on POSIX. The Windows arm of `Preflight._run_skillscan_all` appends the string
"Windows owned-tree observation UNMEASURED" and nothing else, so `cleanup["state"]`
stays "unknown" there by construction. A specimen therefore cannot read an orphan
count out of the product on Windows - it has to count for itself, which is what this
file does.

How it observes (and why not by process name):
  * MARKER FILES, never process enumeration. Each stand-in worker writes a marker when
    it finishes its unit of work, so "an orphan survived" is "a marker appeared after
    `_run_skillscan_all` returned" - no name matching, no ppid chain, no reparenting
    assumption. The trap is written in the production source: a spawned worker's command
    line is `python -c ...`, NOT javis_skillscan.py, so a name-based detector counts 0
    and reports "no survivors" falsely.
  * A STARTUP DENOMINATOR **that excludes the control**. Each worker writes a `started`
    marker with its pid BEFORE it does any work, so the number of workers that actually
    started is known. Without it, a worker that hangs forever or whose completion write
    fails is silently absent and "no marker" reads as "no orphan". ★(r1 m3) The denominator
    is required to be the INTENDED scanner population and is counted apart from the
    control: the earlier form only asserted `started > 0`, which the control alone
    satisfies, so a run where NO scanner worker started still passed its own premise. Such
    a run is now `incomplete`, which is a different verdict from "no orphans".
  * A BOUNDED OBSERVATION HORIZON. After the production call returns, the cell watches
    for a stated number of seconds. A worker with no completion marker at the deadline is
    `unknown`, never 0.
  * A DELIBERATELY SURVIVING CONTROL, spawned by this cell OUTSIDE the scanner's process
    group so the production kill cannot reach it. It writes a `started` marker like any
    worker and its completion marker DURING the horizon, so it travels the same detector
    path and must come back classified as a late completion. A run that reports no late
    completion while the control survived has a broken detector, and every survivor verdict
    from it is void - which is a different verdict from "no orphans". The control is counted
    separately from the scanner's own workers, since by construction it IS a survivor.
  * STATES, NOT ONE NUMBER, and ★(r1 m3) the states separate SURVIVAL from COMPLETION:
      completed_before_return  - its unit finished before the production call came back.
                                 Whether it outlived the DEADLINE (as against the return)
                                 is not separable from here, and is not claimed.
      survived_then_completed  - its completion marker appeared AFTER the production call
                                 returned. That is a CONFIRMED survivor of the deadline:
                                 the unit was still running when the bound fired.
      alive_at_horizon         - no completion marker and the process is still alive at the
                                 end of the horizon. Also a confirmed survivor.
      killed_before_completing - no completion marker and the process is gone.
      unknown                  - `_alive` could not answer (no owned-tree observation on
                                 this arm, or an OSError). Counted and printed, NEVER
                                 folded into a measured survivor count.
    The earlier form labelled EVERY completion marker seen during the horizon "exited" and
    then computed survivors from alive+unknown only, so four workers that demonstrably
    outlived the deadline and finished afterwards were reported as 0 orphans (r1 m3).
    ★A completion marker proves a UNIT FINISHED. It does not prove process exit, so the
    process is probed separately and the two facts are reported separately.

What it drives: the real `Preflight._run_skillscan_all`, with the real
`SKILLSCAN_SCAN_TIMEOUT_S`. The bound is deliberately NOT shortened - the production
source says in as many words that a specimen which shortens it stops burning the real
path. The cell therefore costs about the timeout plus the cleanup and horizon windows.

What it does NOT do: it asserts no particular survivor count. The outcome is a number on
stdout, whatever it is, so the lane that runs it records it. It touches no checklist path
(C-03 owns that; if the checklist kills preflight first this count is not measurable at
all, so a red here before C-03 lands is not attributable to the orphan question).

Run: CYS_PACK_DIR="$(mktemp -d)" python3 bin/tests/test_skillscan_orphan_horizon.py
"""
import json
import os
import subprocess
import sys
import tempfile
import time
import unittest

SELF = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(SELF)
sys.path.insert(0, BIN)
import javis_preflight as P  # noqa: E402

WORKERS = 4
HORIZON_S = 20.0          # how long after the production call returns we keep watching
CONTROL_SLEEP_S = 28.0    # fires after the call returns (timeout 20 + cleanup <= 5) and inside the horizon
CONTROL_KEY = "control"   # its marker index, kept out of the scanner's own population

# The stand-in scanner. Shaped like the real one where it matters: a process pool whose
# workers outlive the parent's deadline. Written to a temp dir at run time, so it is not a
# tracked pack file and carries none of the pack's window/detach policy.
STAND_IN = r'''
import os, sys, time
from concurrent.futures import ProcessPoolExecutor

# _run_skillscan_all fixes argv to ["all", "--json"], so the inputs come through the
# environment, which Popen inherits from the test process.
MARKS = os.environ["C14_MARKS"]
SLEEP = float(os.environ["C14_SLEEP"])

def unit(i):
    with open(os.path.join(MARKS, "started-%d.txt" % i), "w") as fh:
        fh.write("%d\n" % os.getpid())
    time.sleep(SLEEP)
    with open(os.path.join(MARKS, "done-%d.txt" % i), "w") as fh:
        fh.write("%d %.3f\n" % (os.getpid(), time.monotonic()))
    return i

if __name__ == "__main__":
    with ProcessPoolExecutor(max_workers=__W__) as ex:
        list(ex.map(unit, range(__W__)))
    print("{}")
'''.replace("__W__", str(WORKERS))


def _alive(pid):
    """True / False / None(unknown). None is a real answer and must not be read as False."""
    if os.name != "posix":
        return None                      # no owned-tree observation on this arm (see the note)
    try:
        os.kill(pid, 0)
        return True
    except ProcessLookupError:
        return False
    except PermissionError:
        return True                      # exists, not ours to signal
    except OSError:
        return None


def classify(pids, done_before, done_after, alive, control_key=CONTROL_KEY,
             want_workers=WORKERS):
    """Pure classifier: marker sets + an aliveness oracle -> per-unit states and a summary.

    ★(r1 m3) It is a FUNCTION so that the escaped-worker and no-worker controls can drive it
    directly, without paying the production timeout again. Nothing here enumerates processes
    by name and nothing here decides a count from `unknown`.

      pids        {unit key: pid} from the `started-` markers (the control key included)
      done_before completion markers present when the production call RETURNED
      done_after  completion markers that appeared after that, inside the horizon
      alive       pid -> True / False / None(unknown)
    """
    states = {}
    exit_probe = {}
    for key, pid in pids.items():
        mark = "done-%s.txt" % key
        if mark in done_before:
            states[key] = "completed_before_return"
        elif mark in done_after:
            # the unit finished late => the process outlived the deadline. Whether it has
            # since exited is a SEPARATE question, probed and reported separately.
            states[key] = "survived_then_completed"
            exit_probe[key] = alive(pid)
        else:
            states[key] = {True: "alive_at_horizon", False: "killed_before_completing",
                           None: "unknown"}[alive(pid)]
    scanner = sorted(k for k in states if k != control_key)
    counts = {}
    for k in scanner:
        counts[states[k]] = counts.get(states[k], 0) + 1
    confirmed = counts.get("survived_then_completed", 0) + counts.get("alive_at_horizon", 0)
    unknown = counts.get("unknown", 0)
    population_ok = (len(scanner) == want_workers)
    summary = {
        "scanner_workers_started": len(scanner),
        "scanner_workers_intended": want_workers,
        "scanner_population_complete": population_ok,
        "states": counts,
        "confirmed_survivors_past_the_deadline": confirmed,
        "unknown": unknown,
        # the count is only a measurement when nothing is unknown AND the population is whole
        "orphans_surviving_the_deadline": confirmed,
        "orphan_count_is_complete": bool(population_ok and unknown == 0),
        "still_alive_after_a_late_completion": {k: v for k, v in exit_probe.items()},
    }
    return states, summary


class SkillscanOrphanHorizon(unittest.TestCase):
    def test_orphans_after_the_preflight_deadline_are_counted_by_state(self):
        with tempfile.TemporaryDirectory(prefix="c14-") as tmp:
            marks = os.path.join(tmp, "marks")
            os.makedirs(marks)
            tool = os.path.join(tmp, "stand_in_scanner.py")
            with open(tool, "w", encoding="utf-8") as fh:
                fh.write(STAND_IN)

            timeout_s = P.Preflight.SKILLSCAN_SCAN_TIMEOUT_S
            self.assertGreater(timeout_s, 0, "premise: the production bound is a positive number")
            # Workers must still be working when the deadline fires, or nothing is being measured.
            worker_sleep = timeout_s * 2.0

            # The detector control: outside the scanner's process group, so the production kill
            # cannot reach it. It writes a late marker during the horizon.
            control_mark = os.path.join(marks, "done-%s.txt" % CONTROL_KEY)
            control = subprocess.Popen(
                [sys.executable, "-c",
                 "import os,sys,time\n"
                 "marks, key, nap = sys.argv[1], sys.argv[2], float(sys.argv[3])\n"
                 "open(os.path.join(marks, 'started-' + key + '.txt'), 'w').write(str(os.getpid()))\n"
                 "time.sleep(nap)\n"
                 "open(os.path.join(marks, 'done-' + key + '.txt'), 'w').write('%d late' % os.getpid())\n",
                 marks, CONTROL_KEY, str(CONTROL_SLEEP_S)],
                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, stdin=subprocess.DEVNULL)
            try:
                os.environ["C14_MARKS"] = marks
                os.environ["C14_SLEEP"] = str(worker_sleep)
                t0 = time.monotonic()
                rc, out, err, timed_out, cleanup = P.Preflight._run_skillscan_all(tool, timeout_s)
                t_return = time.monotonic()
                self.assertTrue(timed_out, "premise: the stand-in must still be running at the deadline")
                # ★(r1 m3) the completion markers that already exist AT THE RETURN are a
                #   different fact from the ones that appear afterwards, and the earlier form
                #   conflated them: it listed the directory only after the return, so a unit
                #   that finished before the deadline read as a "late completion" and a unit
                #   that finished after it read as "exited". Snapshot the boundary.
                done_before = set(f for f in os.listdir(marks) if f.startswith("done-"))
                started = sorted(f for f in os.listdir(marks) if f.startswith("started-"))
                pids = {}
                for f in started:
                    with open(os.path.join(marks, f), encoding="utf-8") as fh:
                        pids[f[len("started-"):-len(".txt")]] = int(fh.read().strip())

                # Bounded observation horizon: only markers NOT already present at the return.
                deadline = t_return + HORIZON_S
                done_after = set()
                while time.monotonic() < deadline:
                    for f in os.listdir(marks):
                        if f.startswith("done-") and f not in done_before:
                            done_after.add(f)
                    if os.path.exists(control_mark) and len(done_before | done_after) >= len(pids):
                        break
                    time.sleep(0.25)

                states, summary = classify(pids, done_before, done_after, _alive)
                control_state = states.get(CONTROL_KEY)
                control_marker_present = os.path.exists(control_mark)
                report = dict(summary)
                report.update({
                    "row": "C-14 (B-490)",
                    "platform": os.name,
                    "production_timeout_s": timeout_s,
                    "production_cleanup_state": cleanup.get("state"),
                    "production_cleanup_reason": cleanup.get("reason"),
                    "distinct_worker_pids": len(set(p for k, p in pids.items() if k != CONTROL_KEY)),
                    "completions_present_at_return": sorted(done_before),
                    "completions_after_return": sorted(done_after),
                    "detector_control_marker_present": control_marker_present,
                    "detector_control_state": control_state,
                    "horizon_s": HORIZON_S,
                    "call_to_return_s": round(t_return - t0, 2),
                })
                print("C-14-REPORT " + json.dumps(report, ensure_ascii=False, sort_keys=True))

                # The instrument's own integrity, which is the only thing this cell asserts.
                self.assertTrue(
                    control_marker_present,
                    "the deliberately surviving control never wrote its marker: the horizon is too "
                    "short or the control was killed, so ANY survivor verdict from this run is void")
                # ★the control is a confirmed survivor BY CONSTRUCTION, so the classifier must
                #   place it there. If it ever lands in completed_before_return or killed_*, the
                #   detector or the boundary snapshot is broken and the run measures nothing.
                self.assertEqual(
                    control_state, "survived_then_completed",
                    "the control survived the deadline and completed during the horizon, and the "
                    "classifier called it %r: the detector is broken, so this run measures nothing"
                    % (control_state,))
                # ★(r1 m3) the denominator is the SCANNER's population, counted apart from the
                #   control. `started > 0` was satisfied by the control alone.
                self.assertTrue(
                    summary["scanner_population_complete"],
                    "only %d of the intended %d scanner workers started: this run is INCOMPLETE, "
                    "which is not the same verdict as 'no orphans'"
                    % (summary["scanner_workers_started"], WORKERS))
                self.assertEqual(sum(summary["states"].values()), summary["scanner_workers_started"],
                                 "every started scanner worker must land in exactly one state")
                self.assertEqual(len(pids), summary["scanner_workers_started"] + 1,
                                 "the started markers must split into the scanner's workers and the control")
            finally:
                os.environ.pop("C14_MARKS", None)
                os.environ.pop("C14_SLEEP", None)
                try:
                    control.kill()
                except Exception:
                    pass
                try:
                    control.wait(timeout=5)
                except Exception:
                    pass

    # ── controls on the INSTRUMENT (r1 m3). They pay no production timeout: they drive the
    #    classifier directly, which is why it was extracted.
    def test_control_an_escaped_worker_is_counted_as_a_survivor_not_as_an_exit(self):
        """★the case the shipped instrument got wrong: four workers outlive the deadline and
        finish afterwards. The old form labelled every completion marker "exited" and summed
        survivors from alive+unknown, so it printed 0 orphans while four had escaped."""
        pids = {"0": 101, "1": 102, "2": 103, "3": 104, CONTROL_KEY: 199}
        done_before = set()                                   # nothing had finished at the return
        done_after = set("done-%s.txt" % k for k in ("0", "1", "2", "3", CONTROL_KEY))
        states, summary = classify(pids, done_before, done_after, lambda pid: False)
        self.assertEqual(states["0"], "survived_then_completed")
        self.assertEqual(states[CONTROL_KEY], "survived_then_completed")
        self.assertNotIn("exited", set(states.values()))
        self.assertEqual(summary["states"].get("survived_then_completed"), 4)
        self.assertEqual(summary["confirmed_survivors_past_the_deadline"], 4,
                         "four workers completed AFTER the production call returned, so four "
                         "outlived the deadline; a count of 0 here is the m3 defect")
        self.assertEqual(summary["orphans_surviving_the_deadline"], 4)
        self.assertTrue(summary["orphan_count_is_complete"])
        # the completion marker says the UNIT finished; process exit is a separate fact
        self.assertEqual(summary["still_alive_after_a_late_completion"]["0"], False)
        # and a worker that finished BEFORE the return is not a survivor of the return
        states2, summary2 = classify(pids, done_after, set(), lambda pid: False)
        self.assertEqual(states2["0"], "completed_before_return")
        self.assertEqual(summary2["confirmed_survivors_past_the_deadline"], 0)
        # one still alive with no marker at all is a survivor too, and `unknown` is not a count
        states3, summary3 = classify({"0": 101, CONTROL_KEY: 199}, set(),
                                     set(["done-%s.txt" % CONTROL_KEY]),
                                     lambda pid: True, want_workers=1)
        self.assertEqual(states3["0"], "alive_at_horizon")
        self.assertEqual(summary3["confirmed_survivors_past_the_deadline"], 1)
        states4, summary4 = classify({"0": 101, CONTROL_KEY: 199}, set(),
                                     set(["done-%s.txt" % CONTROL_KEY]),
                                     lambda pid: None, want_workers=1)
        self.assertEqual(states4["0"], "unknown")
        self.assertEqual(summary4["confirmed_survivors_past_the_deadline"], 0)
        self.assertEqual(summary4["unknown"], 1)
        self.assertFalse(summary4["orphan_count_is_complete"],
                         "an unknown must make the count incomplete, never a measured 0")

    def test_control_a_run_where_no_scanner_worker_started_is_incomplete_not_zero(self):
        """★the premise the shipped instrument could not fail: only the control started.
        `started > 0` was true, every reconciliation held, and the run reported 0 orphans."""
        pids = {CONTROL_KEY: 199}
        states, summary = classify(pids, set(), set(["done-%s.txt" % CONTROL_KEY]),
                                   lambda pid: False)
        self.assertEqual(states[CONTROL_KEY], "survived_then_completed")
        self.assertEqual(summary["scanner_workers_started"], 0)
        self.assertEqual(summary["scanner_workers_intended"], WORKERS)
        self.assertFalse(summary["scanner_population_complete"],
                         "a run with no scanner worker must be INCOMPLETE")
        self.assertFalse(summary["orphan_count_is_complete"])
        self.assertEqual(summary["states"], {},
                         "there is no scanner population to classify, and that is the finding")
        # a partial population is incomplete too
        _, partial = classify({"0": 101, CONTROL_KEY: 199}, set(),
                              set(["done-%s.txt" % CONTROL_KEY]), lambda pid: False)
        self.assertFalse(partial["scanner_population_complete"])



if __name__ == "__main__":
    unittest.main(verbosity=2)
