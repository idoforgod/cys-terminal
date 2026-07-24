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

# Authenticode-only signing inputs. The owner holds no Authenticode certificate;
# the documented distribution model is unsigned exe + zip with Windows Defender
# guidance on the download page. Setting the ALLOW_UNSIGNED_WINDOWS='1' repository
# variable is the explicit, auditable opt-out that drops exactly these inputs while
# every other required input (updater key, Browser Runtime minisign trust) stays
# fail-closed. Unset/other values keep the byte-identical fail-closed default.
UNSIGNED_WINDOWS_SIGNING_INPUTS = (
    "WINDOWS_CERTIFICATE_B64",
    "WINDOWS_CERTIFICATE_PASSWORD",
    "WINDOWS_EXPECTED_PUBLISHER",
)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("target", choices=("macos", "windows"))
    parser.add_argument("--contract", type=Path, default=DEFAULT_CONTRACT)
    args = parser.parse_args()

    contract = json.loads(args.contract.read_text(encoding="utf-8"))[args.target]
    required = list(contract["required"])
    unsigned_windows = (
        args.target == "windows" and os.environ.get("ALLOW_UNSIGNED_WINDOWS") == "1"
    )
    if unsigned_windows:
        required = [
            name for name in required if name not in UNSIGNED_WINDOWS_SIGNING_INPUTS
        ]
    missing = [name for name in required if not os.environ.get(name)]
    invalid_urls = [
        name
        for name in contract.get("https_url", [])
        if os.environ.get(name) and not os.environ[name].startswith("https://")
    ]
    if missing or invalid_urls:
        if missing:
            print(f"missing release inputs: {', '.join(missing)}", file=sys.stderr)
        if invalid_urls:
            print(
                f"release URL inputs must use https: {', '.join(invalid_urls)}",
                file=sys.stderr,
            )
        return 1
    if unsigned_windows:
        print("UNSIGNED WINDOWS RELEASE (explicit opt-out): Authenticode signing inputs not required")
    print(f"{args.target} release credential contract satisfied (values redacted)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
