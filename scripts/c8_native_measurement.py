#!/usr/bin/env python3
"""C8 native-fixture measurement marker only. It does not decide general test coverage."""
import json
import re

PREFIX = "C8_NATIVE_UNMEASURED_V1 "
GROUP = "C8_NATIVE_TRANSITIONS"
TEST_PREFIX = "native_publication::native_transition_tests::"

def classify_native_markers(text):
    records = []
    malformed = []
    for match in re.finditer(re.escape(PREFIX) + r"([^\r\n]*)", text):
        try:
            value = json.loads(match.group(1))
            if not isinstance(value, dict) or set(value) != {"state", "group", "test", "reason"}:
                raise ValueError("unexpected marker fields")
            if value["state"] != "UNMEASURED" or value["group"] != GROUP:
                raise ValueError("unexpected marker state/group")
            if not isinstance(value["test"], str) or not value["test"].startswith(TEST_PREFIX):
                raise ValueError("missing named native fixture")
            if value["reason"] != "python3_unavailable":
                raise ValueError("unknown marker reason")
            records.append(value)
        except (ValueError, TypeError, json.JSONDecodeError) as error:
            malformed.append({"offset": match.start(), "reason": str(error), "raw": match.group(1)})
    # A truncated/unknown-version marker also fails closed, including no trailing space.
    mentions = len(re.findall(r"C8_NATIVE_UNMEASURED", text))
    if mentions != len(records) + len(malformed):
        malformed.append({"reason": "unparsed or unknown-version native marker", "mentions": mentions})
    blocked = bool(records or malformed)
    return {
        "marker_contract": "C8_NATIVE_UNMEASURED_V1",
        "measurement_state": "UNMEASURED" if blocked else "NO_UNMEASURED_MARKER",
        "blocks_c8_gate": blocked,
        "native_unmeasured": records,
        "marker_errors": malformed,
    }


def main(argv=None):
    import argparse
    from pathlib import Path
    parser = argparse.ArgumentParser(description="Fail CI on C8 UNMEASURED markers; marker absence is not coverage.")
    parser.add_argument("logs", nargs="+", type=Path)
    args = parser.parse_args(argv)
    reports = []
    unreadable = False
    blocked = False
    for path in args.logs:
        try:
            text = path.read_bytes().decode("utf-8", "replace")
        except OSError as error:
            unreadable = True
            reports.append({"log": str(path), "measurement_state": "UNREADABLE", "error": str(error)})
            continue
        result = classify_native_markers(text)
        blocked = blocked or result["blocks_c8_gate"]
        reports.append({"log": str(path), **result})
    print(json.dumps({"logs": reports}, sort_keys=True))
    return 2 if unreadable else 3 if blocked else 0


if __name__ == "__main__":
    raise SystemExit(main())
