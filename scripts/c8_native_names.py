#!/usr/bin/env python3
"""Separate full macOS native-group name reconciliation; not marker parsing or a C8 verdict."""
import argparse
from collections import Counter
import json
from pathlib import Path
import re
from c8_native_measurement import TEST_PREFIX


def reconcile_native_names(text, expected):
    if not isinstance(expected, list) or not expected or len(expected) != len(set(expected)):
        raise ValueError("expected native test set must be nonempty and unique")
    if any(not isinstance(name, str) or not name.startswith(TEST_PREFIX) for name in expected):
        raise ValueError("unexpected native test name")
    text = re.sub(r"\x1b\[[0-9;]*m", "", text)
    test = r"test (" + re.escape(TEST_PREFIX) + r"[A-Za-z0-9_:]+) \.\.\."
    same_line, started = re.compile(test + r" (ok|FAILED|ignored)\b"), re.compile(test + r"(?: |$)")
    outcome_alone = re.compile(r"ok|FAILED|ignored|ignored, .+")
    lines = text.split("\n")
    rows = []
    for at, line in enumerate(lines):
        row = same_line.match(line)
        if row:
            rows.append(row.groups())
            continue
        start = started.match(line)
        if not start:
            continue
        # Output libtest did not capture ended the `test X ... ` line, so X's outcome came alone on a later line. Read it
        # only before the next `test ` line (a new test or the summary) and only if exactly one is there; else X is missing.
        outcomes = []
        for later in lines[at + 1:]:
            if later.startswith("test "):
                break
            if outcome_alone.fullmatch(later.rstrip()):
                outcomes.append(later.rstrip().split(",")[0])
        if len(outcomes) == 1:
            rows.append((start.group(1), outcomes[0]))
    counts = Counter(name for name, _ in rows)
    expected_set = set(expected)
    missing = sorted(expected_set - counts.keys())
    unexpected = sorted(counts.keys() - expected_set)
    duplicates = sorted(name for name, count in counts.items() if count != 1)
    nonpassing = [{"test": name, "status": status} for name, status in rows if status != "ok"]
    return {"native_group_state": "INCOMPLETE" if missing or unexpected or duplicates or nonpassing else "COMPLETE",
            "expected": len(expected), "observed_rows": len(rows), "missing": missing,
            "unexpected": unexpected, "duplicates": duplicates, "nonpassing": nonpassing}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("log", type=Path)
    parser.add_argument("--expected", type=Path, default=Path(__file__).with_name("c8_native_tests.json"))
    args = parser.parse_args(argv)
    try:
        text = args.log.read_bytes().decode("utf-8", "replace")
        result = reconcile_native_names(text, json.loads(args.expected.read_text(encoding="utf-8")))
    except (OSError, ValueError, TypeError) as error:
        print(json.dumps({"native_group_state": "UNREADABLE", "error": str(error)}))
        return 2
    print(json.dumps(result, sort_keys=True))
    return 0 if result["native_group_state"] == "COMPLETE" else 4


if __name__ == "__main__":
    raise SystemExit(main())
