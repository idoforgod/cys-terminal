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
  * A STARTUP DENOMINATOR. Each worker writes a `started` marker with its pid BEFORE it
    does any work, so the number of workers that actually started is known. Without it,
    a worker that hangs forever or whose completion write fails is silently absent and
    "no marker" reads as "no orphan".
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
  * STATES, NOT ONE NUMBER: exited / killed / alive / unknown, with `unknown` counted and
    printed. Incomplete coverage stays `unknown`, which is the honest form of this answer.

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
                started = sorted(f for f in os.listdir(marks) if f.startswith("started-"))
                self.assertGreater(len(started), 0,
                                   "premise: no worker ever started, so there is no denominator")
                pids = {}
                for f in started:
                    with open(os.path.join(marks, f), encoding="utf-8") as fh:
                        pids[f[len("started-"):-len(".txt")]] = int(fh.read().strip())

                # Bounded observation horizon.
                deadline = t_return + HORIZON_S
                late = set()
                while time.monotonic() < deadline:
                    for f in os.listdir(marks):
                        if f.startswith("done-"):
                            late.add(f)
                    if os.path.exists(control_mark) and len(late) >= len(pids):
                        break
                    time.sleep(0.25)

                # The control is a survivor by construction; it validates the detector and is
                # never mixed into the scanner's own population.
                control_states = {}
                states = {}
                for key, pid in pids.items():
                    if ("done-%s.txt" % key) in late:
                        states[key] = "exited"      # finished its unit - late or not
                    else:
                        a = _alive(pid)
                        states[key] = {True: "alive", False: "killed", None: "unknown"}[a]
                for key in list(states):
                    if key == CONTROL_KEY:
                        control_states[key] = states.pop(key)

                control_marker_present = os.path.exists(control_mark)
                control_seen_by_detector = ("done-%s.txt" % CONTROL_KEY) in late
                counts = {}
                for v in states.values():
                    counts[v] = counts.get(v, 0) + 1
                report = {
                    "row": "C-14 (B-490)",
                    "platform": os.name,
                    "production_timeout_s": timeout_s,
                    "production_cleanup_state": cleanup.get("state"),
                    "production_cleanup_reason": cleanup.get("reason"),
                    "workers_started": len(pids),
                    "states": counts,
                    "late_completion_markers": sorted(late),
                    "orphans_surviving_the_deadline": counts.get("alive", 0) + counts.get("unknown", 0),
                    "detector_control_marker_present": control_marker_present,
                    "detector_control_seen_as_late_completion": control_seen_by_detector,
                    "detector_control_state": control_states.get(CONTROL_KEY),
                    "horizon_s": HORIZON_S,
                    "call_to_return_s": round(t_return - t0, 2),
                }
                print("C-14-REPORT " + json.dumps(report, ensure_ascii=False, sort_keys=True))

                # The instrument's own integrity, which is the only thing this cell asserts.
                self.assertTrue(
                    control_marker_present,
                    "the deliberately surviving control never wrote its marker: the horizon is too "
                    "short or the control was killed, so ANY survivor verdict from this run is void")
                self.assertTrue(
                    control_seen_by_detector,
                    "the control survived and wrote a late completion marker, and the detector did "
                    "NOT report it: the detector is broken, so this run measures nothing")
                self.assertEqual(len(states), sum(counts.values()),
                                 "every started worker must land in exactly one state")
                self.assertEqual(len(pids), len(states) + len(control_states),
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


if __name__ == "__main__":
    unittest.main(verbosity=2)
