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
    rows = re.findall(r"^test (" + re.escape(TEST_PREFIX) + r"[A-Za-z0-9_:]+) \.\.\. (ok|FAILED|ignored)\b[^\r\n]*", text, re.M)
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
