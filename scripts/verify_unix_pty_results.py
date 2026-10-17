"""Require executed Unix PTY pins in one captured, default-parallel libtest run.

The workflow supplies --format pretty --color never without --nocapture or a
thread-count override. Listing output and a successful Cargo exit are not proof
that these pins ran. Cargo and tee exit codes remain separate workflow gates.
"""

import argparse
from collections import Counter
from pathlib import Path
import re
import sys


REQUIRED = tuple("unix::pty_open_tests::" + name for name in (
    "unix_allocation_guard_covers_native_open_and_both_cloexec_without_spawn_or_await",
    "allocation_lock_stays_behind_the_unix_module_boundary",
    "static_allocation_lock_excludes_a_second_thread",
    "unit_allocation_lock_is_reusable_after_unwind",
    "held_allocator_returns_busy_at_deadline_without_interrupting_holder",
    "acquired_after_deadline_is_released_without_granting_allocation",
    "expired_deadline_refuses_even_a_free_allocator_and_production_budget_is_five_seconds",
))
RUNNING = re.compile(r"running ([0-9]+) tests?")
RESULT = re.compile(r"test (\S+) \.\.\. (ok|FAILED|ignored(?:, .*)?)")
SUMMARY = re.compile(
    r"test result: (ok|FAILED)\. ([0-9]+) passed; ([0-9]+) failed; "
    r"([0-9]+) ignored; ([0-9]+) measured; ([0-9]+) filtered out; "
    r"finished in [0-9]+(?:\.[0-9]+)?s"
)


def verify_output(text):
    """Return refusal reasons; accept any parallel completion order, never sort IDs away."""
    lines = text.splitlines()
    starts = [(i, RUNNING.fullmatch(line)) for i, line in enumerate(lines)
              if RUNNING.fullmatch(line)]
    ends = [(i, SUMMARY.fullmatch(line)) for i, line in enumerate(lines)
            if SUMMARY.fullmatch(line)]
    if len(starts) != 1 or len(ends) != 1:
        return ["expected exactly one running-tests header and one libtest summary"]
    start, count = starts[0]
    end, summary = ends[0]
    if start >= end:
        return ["libtest summary precedes its running-tests header"]

    records = []
    for line in lines[start + 1:end]:
        match = RESULT.fullmatch(line)
        if match:
            records.append(match.groups())
    names = Counter(name for name, _ in records)
    states = Counter("ignored" if status.startswith("ignored") else status
                     for _, status in records)
    errors = []
    for pin in REQUIRED:
        matches = [status for name, status in records if name == pin]
        if matches != ["ok"]:
            errors.append("required executed result must occur once as ok: " + pin
                          + " (observed " + repr(matches) + ")")
    if any(number != 1 for number in names.values()):
        errors.append("duplicate test result identity")
    passed, failed, ignored, measured, _filtered = map(int, summary.groups()[1:])
    if summary.group(1) != "ok" or failed != 0:
        errors.append("libtest summary is not successful")
    if (passed, failed, ignored, measured) != (
            states["ok"], states["FAILED"], states["ignored"], 0):
        errors.append("libtest summary does not reconcile with terminal test results")
    if int(count.group(1)) != len(records):
        errors.append("running-tests count does not reconcile with terminal results")
    return errors


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("log", type=Path)
    args = parser.parse_args(argv)
    try:
        errors = verify_output(args.log.read_text(encoding="utf-8", errors="replace"))
    except OSError as error:
        errors = ["cannot read current vendor run log: " + str(error)]
    if errors:
        for error in errors:
            print("Unix PTY execution gate: " + error, file=sys.stderr)
        return 1
    print("Unix PTY execution gate: all 7 required tests executed ok; summary reconciled")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
