#!/usr/bin/env python3
"""Apply Tauri's exact in-place bundle-type marker before release hashing."""

from __future__ import annotations

import argparse
import os
import sys
import tempfile
from pathlib import Path


UNKNOWN = b"__TAURI_BUNDLE_TYPE_VAR_UNK"
BUNDLE_TYPES = {"nsis": b"__TAURI_BUNDLE_TYPE_VAR_NSS"}


class PatchError(ValueError):
    pass


def patch_bundle_type(executable: Path, bundle_type: str) -> None:
    if executable.is_symlink() or not executable.is_file():
        raise PatchError(f"executable is not a regular file: {executable}")
    replacement = BUNDLE_TYPES[bundle_type]
    if len(replacement) != len(UNKNOWN):
        raise PatchError("Tauri bundle marker length mismatch")

    original = executable.read_bytes()
    occurrences = original.count(UNKNOWN)
    if occurrences != 1:
        raise PatchError(
            f"expected exactly one Tauri bundle marker, found {occurrences}"
        )
    patched = original.replace(UNKNOWN, replacement, 1)

    temporary_name: str | None = None
    try:
        with tempfile.NamedTemporaryFile(
            mode="wb",
            prefix=f".{executable.name}.",
            suffix=".partial",
            dir=executable.parent,
            delete=False,
        ) as temporary:
            temporary_name = temporary.name
            temporary.write(patched)
            temporary.flush()
            os.fsync(temporary.fileno())
        os.chmod(temporary_name, executable.stat().st_mode)
        os.replace(temporary_name, executable)
        temporary_name = None
    finally:
        if temporary_name is not None:
            Path(temporary_name).unlink(missing_ok=True)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--executable", required=True, type=Path)
    parser.add_argument("--bundle-type", required=True, choices=sorted(BUNDLE_TYPES))
    args = parser.parse_args()
    try:
        patch_bundle_type(args.executable, args.bundle_type)
    except (OSError, PatchError) as error:
        print(f"Tauri bundle marker patch failed: {error}", file=sys.stderr)
        return 1
    print(f"Tauri bundle marker fixed before hashing: {args.bundle_type}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
