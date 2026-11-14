#!/usr/bin/env python3
"""Prepare the "following version" build inputs IN THE CI JOB'S WORKSPACE ONLY (RULINGS EQ2 D4; never committed).

The measurement's second transition (candidate -> next) exchanges cys.exe and cysd.exe built from the same commit with:
  * the package version raised by one patch level (Cargo.toml [package] and the matching Cargo.lock entry) - the pack
    version and VERSIONINFO follow it (src/pack.rs uses CARGO_PKG_VERSION; build.rs sets VERSIONINFO from it);
  * one marker line appended to every pack file the scenario may edit (matrix.json "scenario"), so that the vendor bytes
    of those files differ between the candidate and the next version. Written with LF only.
Usage: python tnv_prepare.py --root <checkout> --record tnv-prepare.json
"""

import argparse
import json
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))


def bump_patch(v):
    m = re.fullmatch(r"(\d+)\.(\d+)\.(\d+)", v.strip())
    if not m:
        raise ValueError("not a plain x.y.z version: %r" % v)
    return "%s.%s.%d" % (m.group(1), m.group(2), int(m.group(3)) + 1)


def set_package_version(cargo_toml, new):
    m = re.search(r'(?ms)^(\[package\].*?^version\s*=\s*")([^"]+)(")', cargo_toml)
    if not m:
        raise ValueError("no [package] version in Cargo.toml")
    return cargo_toml[:m.start(2)] + new + cargo_toml[m.end(2):], m.group(2)


def set_lock_version(lock, name, old, new):
    pat = re.compile(r'(\[\[package\]\]\nname = "%s"\nversion = ")%s(")' % (re.escape(name), re.escape(old)))
    out, n = pat.subn(lambda mm: mm.group(1) + new + mm.group(2), lock)
    return out, n


def append_marker(data, line):
    nl = b"" if data.endswith(b"\n") else b"\n"
    return data.replace(b"\r\n", b"\n") + nl + line.encode("utf-8") + b"\n"


def main(argv=None):
    ap = argparse.ArgumentParser()
    ap.add_argument("--root", required=True)
    ap.add_argument("--record", required=True)
    a = ap.parse_args(argv)
    with open(os.path.join(HERE, "matrix.json"), encoding="utf-8") as f:
        sc = json.load(f)["scenario"]
    root = os.path.abspath(a.root)
    cargo_p = os.path.join(root, "Cargo.toml")
    with open(cargo_p, encoding="utf-8", newline="") as f:
        cargo = f.read()
    name = re.search(r'(?ms)^\[package\].*?^name\s*=\s*"([^"]+)"', cargo).group(1)
    cargo2, old = set_package_version(cargo, "0.0.0")
    new = bump_patch(old)
    cargo2, _ = set_package_version(cargo, new)
    with open(cargo_p, "w", encoding="utf-8", newline="") as f:
        f.write(cargo2)
    lock_p = os.path.join(root, "Cargo.lock")
    lock_n = 0
    if os.path.isfile(lock_p):
        with open(lock_p, encoding="utf-8", newline="") as f:
            lock = f.read()
        lock2, lock_n = set_lock_version(lock, name, old, new)
        with open(lock_p, "w", encoding="utf-8", newline="") as f:
            f.write(lock2)
    marked, skipped = [], []
    files = (sc["listed_changed"] + sc["listed_unchanged"] + sc["general_changed_py"] + sc["general_changed_sh"]
             + sc["general_changed_md"])
    for rel in files:
        p = os.path.join(root, "cysjavis-pack", *rel.split("/"))
        if not os.path.isfile(p):
            skipped.append(rel)
            continue
        ext = rel.rsplit(".", 1)[-1]
        line = sc["next_version_marker"].get("md" if ext == "md" else ("py" if ext == "py" else "sh"))
        with open(p, "rb") as f:
            data = f.read()
        with open(p, "wb") as f:
            f.write(append_marker(data, line))
        marked.append(rel)
    rec = {"package": name, "candidate_version": old, "tnv_version": new, "lock_entries_changed": lock_n,
           "marked_files": marked, "skipped_files": skipped,
           "note": "workspace-only inputs of the following-version build (RULINGS EQ2 D4); never committed"}
    with open(a.record, "w", encoding="utf-8", newline="\n") as f:
        json.dump(rec, f, indent=1)
    print(json.dumps(rec))
    return 0 if lock_n in (0, 1) else 2


if __name__ == "__main__":
    sys.exit(main())
