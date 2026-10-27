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
  * TWO AXES, NOT ONE NUMBER, reported separately and combined only per worker:
      completion  before_return | after_return | none   - when the WORK UNIT finished
      liveness    alive | exited | unknown              - what the PROCESS is at the horizon
    A worker is a confirmed survivor of the deadline if EITHER axis says so: a completion
    AFTER the production return means the unit was still running when the bound fired, and a
    process still alive after the return and its cleanup outlived both. `unknown` is reported
    only where neither axis could answer, and is never folded into a measured count.
    Two fixes live here, both from review rounds, both of which produced a false 0:
      (r1 m3) the old form labelled EVERY completion marker seen during the horizon "exited"
      and summed survivors from alive+unknown only, so four workers that outlived the
      deadline and finished afterwards were reported as 0 orphans.
      (r2 m3) the r1 form then classified `done_before` WITHOUT probing liveness at all, so
      four workers whose units had finished but whose PROCESSES were all still alive at the
      horizon were again reported as 0, with complete = true. ★Liveness is now probed for
      EVERY started worker, `completed_before_return` included.
    ★A completion marker proves a UNIT FINISHED. It never proves process exit.

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

    ★(r1 m3) It is a FUNCTION so that the controls can drive it directly, without paying the
    production timeout again. Nothing here enumerates processes by name and nothing here
    decides a count from `unknown`.

    ★(r2 m3) LIVENESS IS PROBED FOR EVERY STARTED WORKER, including the ones whose unit had
    already finished at the return. The r1 form classified `done_before` WITHOUT calling
    `alive(pid)` at all, and the reviewer's counterexample is exact: four `done_before`
    markers, a late control and an oracle that answers True for every pid gave
    survivors = 0 and complete = true while all four worker PROCESSES were alive at the
    horizon. A pool worker can finish its unit and stay a process - that is what a pool is.

      pids        {unit key: pid} from the `started-` markers (the control key included)
      done_before completion markers present when the production call RETURNED
      done_after  completion markers that appeared after that, inside the horizon
      alive       pid -> True / False / None(unknown)

    Two axes, kept SEPARATE in the output and combined only in the per-key state:
      completion  before_return | after_return | none      (when the UNIT finished)
      liveness    alive | exited | unknown                  (what the PROCESS is now)
    A worker is a confirmed survivor of the deadline if EITHER axis says so:
      * completion == after_return  - the unit was still running when the bound fired, so the
        process outlived it, whatever the process is doing now; or
      * liveness == alive           - the process is alive after the production call returned
        and after its cleanup, so it outlived both.
    `unknown` is only reported where neither axis could answer - an unknown liveness with a
    late completion is still a confirmed survivor.
    """
    states = {}
    completion = {}
    liveness = {}
    for key, pid in pids.items():
        mark = "done-%s.txt" % key
        if mark in done_before:
            completion[key] = "before_return"
        elif mark in done_after:
            completion[key] = "after_return"
        else:
            completion[key] = "none"
        liveness[key] = {True: "alive", False: "exited", None: "unknown"}[alive(pid)]
        states[key] = completion[key] + "+" + liveness[key]

    scanner = sorted(k for k in states if k != control_key)

    def survivor(k):
        return completion[k] == "after_return" or liveness[k] == "alive"

    def unresolved(k):
        # neither axis could place it: no late completion AND no answer about the process
        return completion[k] != "after_return" and liveness[k] == "unknown"

    counts, comp_counts, live_counts = {}, {}, {}
    for k in scanner:
        counts[states[k]] = counts.get(states[k], 0) + 1
        comp_counts[completion[k]] = comp_counts.get(completion[k], 0) + 1
        live_counts[liveness[k]] = live_counts.get(liveness[k], 0) + 1
    confirmed = sum(1 for k in scanner if survivor(k))
    unknown = sum(1 for k in scanner if unresolved(k))
    population_ok = (len(scanner) == want_workers)
    summary = {
        "scanner_workers_started": len(scanner),
        "scanner_workers_intended": want_workers,
        "scanner_population_complete": population_ok,
        "states": counts,
        "completion_timing": comp_counts,
        "process_liveness": live_counts,
        "confirmed_survivors_past_the_deadline": confirmed,
        "survivor_keys": sorted(k for k in scanner if survivor(k)),
        "unknown": unknown,
        # the count is only a measurement when nothing is unresolved AND the population is whole
        "orphans_surviving_the_deadline": confirmed,
        "orphan_count_is_complete": bool(population_ok and unknown == 0),
        "liveness_probed_for_every_started_worker": sorted(liveness) == sorted(pids),
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
                #   place it there. If its completion axis is ever anything but after_return, the
                #   detector or the boundary snapshot is broken and the run measures nothing.
                self.assertTrue(
                    control_state.startswith("after_return+"),
                    "the control survived the deadline and completed during the horizon, and the "
                    "classifier called it %r: the detector is broken, so this run measures nothing"
                    % (control_state,))
                # ★(r2 m3) the instrument must have asked about every started worker's process
                self.assertTrue(summary["liveness_probed_for_every_started_worker"],
                                "liveness was not probed for every started worker")
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
        self.assertEqual(states["0"], "after_return+exited")
        self.assertEqual(states[CONTROL_KEY], "after_return+exited")
        self.assertEqual(summary["completion_timing"], {"after_return": 4})
        self.assertEqual(summary["states"].get("after_return+exited"), 4)
        self.assertEqual(summary["confirmed_survivors_past_the_deadline"], 4,
                         "four workers completed AFTER the production call returned, so four "
                         "outlived the deadline; a count of 0 here is the m3 defect")
        self.assertEqual(summary["orphans_surviving_the_deadline"], 4)
        self.assertTrue(summary["orphan_count_is_complete"])
        # ★the two axes are reported separately, and a late completion counts as a survivor
        #   even though the process has since exited - the unit outlived the deadline.
        self.assertEqual(summary["process_liveness"], {"exited": 4})
        self.assertEqual(summary["survivor_keys"], ["0", "1", "2", "3"])
        # and a worker that finished BEFORE the return is not a survivor of the return
        states2, summary2 = classify(pids, done_after, set(), lambda pid: False)
        self.assertEqual(states2["0"], "before_return+exited")
        self.assertEqual(summary2["confirmed_survivors_past_the_deadline"], 0)
        # one still alive with no marker at all is a survivor too, and `unknown` is not a count
        states3, summary3 = classify({"0": 101, CONTROL_KEY: 199}, set(),
                                     set(["done-%s.txt" % CONTROL_KEY]),
                                     lambda pid: True, want_workers=1)
        self.assertEqual(states3["0"], "none+alive")
        self.assertEqual(summary3["confirmed_survivors_past_the_deadline"], 1)
        states4, summary4 = classify({"0": 101, CONTROL_KEY: 199}, set(),
                                     set(["done-%s.txt" % CONTROL_KEY]),
                                     lambda pid: None, want_workers=1)
        self.assertEqual(states4["0"], "none+unknown")
        self.assertEqual(summary4["confirmed_survivors_past_the_deadline"], 0)
        self.assertEqual(summary4["unknown"], 1)
        self.assertFalse(summary4["orphan_count_is_complete"],
                         "an unknown must make the count incomplete, never a measured 0")

    def test_control_all_four_completed_before_return_but_every_process_is_alive(self):
        """★(r2 m3 · codex) THE reviewer's counterexample, executed. Four units finished before
        the production call returned, a late control, and an aliveness oracle that answers True
        for every pid. The r1 classifier never asked `alive()` about a `done_before` worker, so
        it reported survivors = 0 with complete = true while all four processes were alive at
        the horizon. A pool worker finishing its unit is not a dead process."""
        pids = {"0": 101, "1": 102, "2": 103, "3": 104, CONTROL_KEY: 199}
        done_before = set("done-%s.txt" % k for k in ("0", "1", "2", "3"))
        done_after = set(["done-%s.txt" % CONTROL_KEY])
        states, summary = classify(pids, done_before, done_after, lambda pid: True)
        # the completion axis is honest: their units DID finish before the return
        self.assertEqual(summary["completion_timing"], {"before_return": 4})
        # and the liveness axis is the one the r1 form never asked for
        self.assertEqual(summary["process_liveness"], {"alive": 4})
        self.assertEqual(states["0"], "before_return+alive")
        self.assertEqual(summary["confirmed_survivors_past_the_deadline"], 4,
                         "all four worker PROCESSES are alive at the horizon, so all four "
                         "outlived the production deadline; 0 here is the r2 m3 defect")
        self.assertEqual(summary["orphans_surviving_the_deadline"], 4)
        self.assertEqual(summary["survivor_keys"], ["0", "1", "2", "3"])
        # it is a MEASURED 4, not an unknown: both axes answered for every worker
        self.assertEqual(summary["unknown"], 0)
        self.assertTrue(summary["orphan_count_is_complete"])
        self.assertTrue(summary["liveness_probed_for_every_started_worker"])
        # the mirror case must still read 0: units finished early AND the processes are gone
        gone_states, gone = classify(pids, done_before, done_after, lambda pid: False)
        self.assertEqual(gone_states["0"], "before_return+exited")
        self.assertEqual(gone["confirmed_survivors_past_the_deadline"], 0)
        self.assertTrue(gone["orphan_count_is_complete"])
        # ... and an unprobeable process with an early completion stays UNKNOWN, never a 0
        murky_states, murky = classify(pids, done_before, done_after, lambda pid: None)
        self.assertEqual(murky_states["0"], "before_return+unknown")
        self.assertEqual(murky["unknown"], 4)
        self.assertEqual(murky["confirmed_survivors_past_the_deadline"], 0)
        self.assertFalse(murky["orphan_count_is_complete"])

    def test_control_a_run_where_no_scanner_worker_started_is_incomplete_not_zero(self):
        """★the premise the shipped instrument could not fail: only the control started.
        `started > 0` was true, every reconciliation held, and the run reported 0 orphans."""
        pids = {CONTROL_KEY: 199}
        states, summary = classify(pids, set(), set(["done-%s.txt" % CONTROL_KEY]),
                                   lambda pid: False)
        self.assertEqual(states[CONTROL_KEY], "after_return+exited")
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
