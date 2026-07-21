#!/usr/bin/env python3
"""Fail closed on missing release inputs without ever printing their values."""

from __future__ import annotations

import argparse
import json
import os
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_CONTRACT = ROOT / "release" / "credential-contract.json"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("target", choices=("macos", "windows"))
    parser.add_argument("--contract", type=Path, default=DEFAULT_CONTRACT)
    args = parser.parse_args()

    contract = json.loads(args.contract.read_text(encoding="utf-8"))[args.target]
    missing = [name for name in contract["required"] if not os.environ.get(name)]
    invalid_urls = [
        name
        for name in contract.get("https_url", [])
        if os.environ.get(name) and not os.environ[name].startswith("https://")
    ]
    invalid_exact = [
        name
        for name, expected in contract.get("exact", {}).items()
        if os.environ.get(name) and os.environ[name] != expected
    ]
    if missing or invalid_urls or invalid_exact:
        if missing:
            print(f"missing release inputs: {', '.join(missing)}", file=sys.stderr)
        if invalid_urls:
            print(
                f"release URL inputs must use https: {', '.join(invalid_urls)}",
                file=sys.stderr,
            )
        if invalid_exact:
            print(
                f"release inputs do not match the required policy: {', '.join(invalid_exact)}",
                file=sys.stderr,
            )
        return 1
    print(f"{args.target} release credential contract satisfied (values redacted)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
