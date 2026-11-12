#!/usr/bin/env python3
"""min_binary floor gate — 0.15.0 「스스로 해결」 M11 카 · CONTRACTS C8 8.13 / O-C8-5 (ii) · design §4-11 카, §6-11 카.

The pack's minimum program version (`min_binary_version`, the floor below which an installed program refuses a
pack-only update — src/bin/cys.rs version_gates) is written in three places that must move together in ONE commit:
  1. .github/workflows/release.yml       job env `PACK_MIN_BINARY`           (binary lane; signs the pack manifest)
  2. .github/workflows/pack-release.yml  job env `PACK_MIN_BINARY_OVERRIDE`  (pack-only lane)
  3. docs/RELEASE.md                     the literal "현행 X.Y.Z" on the pack-only step-2 line that names `$PACK_MIN_BINARY`
Until 0.15.0 only comments tied them together; one site left behind is silent (an old program installs the new pack
under the old rules — design §4-11 카; the S26 incident class in docs/RELEASE.md). This gate fails when they differ.

It also lints what the design says must stay untouched (§6-11 카 "하한 검증기, 보안 하한 파일은 건드리지 않음"):
  N1  pack-release.yml still runs its min_binary verifier step (`Verify with min_binary verifier`) unconditionally
      (no `if:`, no `continue-on-error: true`) and that step still runs `pack-update --from . --dry-run` with the
      `$MIN_BINARY` program; the policy step still takes the override into MIN_BINARY; release.yml still signs with
      `--min-binary-version "$PACK_MIN_BINARY"`.
  N2  the floor is not below security-floor.txt (docs/RELEASE.md 불변 규칙: "--min-binary-version 0.12.48 이상 필수").
      The file itself is pinned byte for byte by scripts/tests/test_check_min_binary_floor.py.
  AE3 (RULINGS AE3) once the program version (Cargo.toml [package] version) is 0.15.0 or later, the floor must be
      0.15.0 or later too: the 0.15.0 pack may only install under a program that has `cys repair` (design §4-11 카).
      Green at today's values (0.14.56 / 0.14.52); red at a release commit that bumps the version but forgets
      the three floor sites.

The VALUE is not chosen here: the release commit raises all three sites together; the gate is green at any equal
value that satisfies AE3 (0.14.52 today, 0.15.0 in the release commit). Text only — no network, no git, no YAML
library (stdlib only).

    python3 scripts/check_min_binary_floor.py [--root DIR]
Exit: 0 = consistent · 1 = inconsistent (every finding named, as ::error:: lines) · 2 = cannot judge (file missing)
"""
import argparse
import os
import re
import sys

VER = r"\d+\.\d+\.\d+"
RELEASE_YML = ".github/workflows/release.yml"
PACK_YML = ".github/workflows/pack-release.yml"
RELEASE_MD = "docs/RELEASE.md"
FLOOR_TXT = "security-floor.txt"
CARGO_TOML = "Cargo.toml"
SELF_REPAIR_RELEASE = (0, 15, 0)   # RULINGS AE3: from this program version on, the floor is at least this
VERIFIER_STEP = "Verify with min_binary verifier (해법②)"
COMPUTE_STEP = "Compute min_binary (policy)"
EXIT_OK, EXIT_BAD, EXIT_UNJUDGEABLE = 0, 1, 2


class Unjudgeable(Exception):
    pass


def read(root, rel):
    try:
        with open(os.path.join(root, rel), encoding="utf-8") as f:
            return f.read()
    except (OSError, UnicodeDecodeError) as e:
        raise Unjudgeable("%s: %s" % (rel, e))


def is_comment_or_blank(line):
    s = line.strip()
    return not s or s.startswith("#")


def code_lines(text):
    """(1-based line number, line) for every line that is not blank and not a YAML comment."""
    return [(i, l) for i, l in enumerate(text.split("\n"), 1) if not is_comment_or_blank(l)]


def assignments(text, key):
    """Every non-comment `KEY: value` line → (line number, unquoted value). `KEY_OTHER:` never matches `KEY`."""
    pat = re.compile(r"^\s*" + re.escape(key) + r"\s*:\s*(.*?)\s*(?:#.*)?$")
    out = []
    for i, l in code_lines(text):
        m = pat.match(l)
        if m:
            raw = m.group(1)
            if len(raw) >= 2 and raw[0] == raw[-1] and raw[0] in "'\"":
                raw = raw[1:-1]
            out.append((i, raw))
    return out


def step_block(text, name):
    """The non-comment lines of the workflow step whose `name:` is exactly NAME (the whole list item, so a key
    such as `if:` written BEFORE `name:` in the same item is included), or None if there is no such step."""
    lines = text.split("\n")
    for i, l in enumerate(lines):
        m = re.match(r"^(\s*)(-\s+)?name:\s*(.+?)\s*$", l)
        if not m or m.group(3).strip("'\"") != name:
            continue
        if m.group(2):
            start, dash = i, len(m.group(1))
        else:
            key_ind, start, dash = len(m.group(1)), None, None
            for j in range(i - 1, -1, -1):
                if is_comment_or_blank(lines[j]):
                    continue
                mm = re.match(r"^(\s*)-\s+\S", lines[j])
                ind = len(lines[j]) - len(lines[j].lstrip())
                if mm and len(mm.group(1)) + 2 == key_ind:
                    start, dash = j, len(mm.group(1))
                    break
                if ind < key_ind:
                    break
            if start is None:
                continue
        end = len(lines)
        for j in range(start + 1, len(lines)):
            if is_comment_or_blank(lines[j]):
                continue
            if len(lines[j]) - len(lines[j].lstrip()) <= dash:
                end = j
                break
        return "\n".join(l for l in lines[start:end] if not is_comment_or_blank(l))
    return None


def vtuple(v):
    return tuple(int(x) for x in v.split("."))


def cargo_package_version(text):
    """`version = "X.Y.Z"` of the [package] table of Cargo.toml (the product-version SOT, version-check.sh)."""
    section = None
    for line in text.split("\n"):
        s = line.strip()
        if s.startswith("["):
            section = s
            continue
        if section == "[package]":
            m = re.match(r'version\s*=\s*"(' + VER + r')"\s*(?:#.*)?$', s)
            if m:
                return m.group(1)
    raise Unjudgeable("%s: no [package] version = \"X.Y.Z\"" % CARGO_TOML)


def check(root):
    """→ (sites {label: value}, problems [str]). Raises Unjudgeable when an input cannot be read."""
    rel_yml, pack_yml = read(root, RELEASE_YML), read(root, PACK_YML)
    md, floor_txt = read(root, RELEASE_MD), read(root, FLOOR_TXT)
    program = cargo_package_version(read(root, CARGO_TOML))
    sites, problems = {}, []

    for path, text, key in ((RELEASE_YML, rel_yml, "PACK_MIN_BINARY"),
                            (PACK_YML, pack_yml, "PACK_MIN_BINARY_OVERRIDE")):
        found = assignments(text, key)
        if len(found) != 1:
            problems.append("%s: expected exactly one non-comment `%s:` assignment, found %d (lines %s)"
                            % (path, key, len(found), ",".join(str(i) for i, _ in found) or "-"))
            continue
        line, val = found[0]
        if not re.fullmatch(VER, val):
            problems.append("%s:%d: `%s` = %r is not X.Y.Z (an empty pack-only override would sign the pack with the "
                            "policy value of scripts/min-binary-policy.sh, not the release floor)" % (path, line, key, val))
            continue
        sites["%s:%d %s" % (path, line, key)] = val

    md_lines = [(i, l) for i, l in enumerate(md.split("\n"), 1) if "`$PACK_MIN_BINARY`" in l and "현행" in l]
    if len(md_lines) != 1:
        problems.append("%s: expected exactly one line naming `$PACK_MIN_BINARY` with the literal '현행 X.Y.Z', found %d"
                        % (RELEASE_MD, len(md_lines)))
    else:
        i, l = md_lines[0]
        vals = re.findall(r"현행\s+(" + VER + r")", l)
        if len(vals) != 1:
            problems.append("%s:%d: expected one '현행 X.Y.Z' on the `$PACK_MIN_BINARY` line, found %d"
                            % (RELEASE_MD, i, len(vals)))
        else:
            sites["%s:%d 현행" % (RELEASE_MD, i)] = vals[0]

    distinct = sorted(set(sites.values()), key=vtuple)
    if len(sites) == 3 and len(distinct) != 1:
        problems.append("the three min_binary sites differ: %s — raise all three in ONE commit "
                        "(docs/RELEASE.md 불변 규칙; design §4-11 카)"
                        % "; ".join("%s = %s" % kv for kv in sites.items()))

    # the values must still reach the signed manifest (otherwise equal values prove nothing)
    if not any('--min-binary-version "$PACK_MIN_BINARY"' in l for _, l in code_lines(rel_yml)):
        problems.append('%s: no non-comment `--min-binary-version "$PACK_MIN_BINARY"` — the binary lane no longer '
                        'signs the pack with this value' % RELEASE_YML)
    compute = step_block(pack_yml, COMPUTE_STEP)
    if compute is None or 'MB="$PACK_MIN_BINARY_OVERRIDE"' not in compute or "MIN_BINARY=" not in compute:
        problems.append("%s: step '%s' no longer takes PACK_MIN_BINARY_OVERRIDE into MIN_BINARY" % (PACK_YML, COMPUTE_STEP))

    # N1 — the min_binary verifier (hard-floor validator) stays and stays unconditional
    ver = step_block(pack_yml, VERIFIER_STEP)
    if ver is None:
        problems.append("N1 %s: the min_binary verifier step '%s' is missing (design §6-11 카: 하한 검증기는 "
                        "건드리지 않는다)" % (PACK_YML, VERIFIER_STEP))
    else:
        if re.search(r"^\s*(-\s+)?if\s*:", ver, re.M):
            problems.append("N1 %s: the min_binary verifier step is conditional (`if:`) — it must run on every "
                            "pack-only release" % PACK_YML)
        if re.search(r"^\s*(-\s+)?continue-on-error\s*:\s*true\b", ver, re.M):
            problems.append("N1 %s: the min_binary verifier step has `continue-on-error: true` — a red verifier "
                            "would no longer stop the release" % PACK_YML)
        if "pack-update --from . --dry-run" not in ver or "$MIN_BINARY" not in ver:
            problems.append("N1 %s: the min_binary verifier step no longer runs `pack-update --from . --dry-run` "
                            "with the $MIN_BINARY program" % PACK_YML)

    # N2 — never below the security floor
    floor_lines = [l.strip() for l in floor_txt.split("\n") if l.strip() and not l.strip().startswith("#")]
    floor = floor_lines[0] if floor_lines else ""
    if not re.fullmatch(VER, floor):
        problems.append("N2 %s: first non-comment line %r is not X.Y.Z" % (FLOOR_TXT, floor))
    elif len(distinct) == 1 and vtuple(distinct[0]) < vtuple(floor):
        problems.append("N2: min_binary %s is below the security floor %s (%s; docs/RELEASE.md 불변 규칙)"
                        % (distinct[0], floor, FLOOR_TXT))

    # AE3 — a 0.15.0+ program version with a floor below 0.15.0 = the release commit forgot the bump
    if vtuple(program) >= SELF_REPAIR_RELEASE:
        low = ["%s = %s" % kv for kv in sites.items() if vtuple(kv[1]) < SELF_REPAIR_RELEASE]
        if low:
            problems.append("AE3: %s version is %s but the min_binary floor is below %d.%d.%d (%s) — the 0.15.0 pack "
                            "must require a program that has `cys repair`; raise all three sites in the release "
                            "commit (design §4-11 카, RULINGS AE3)"
                            % ((CARGO_TOML, program) + SELF_REPAIR_RELEASE + ("; ".join(low),)))
    return sites, problems


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    ap.add_argument("--root", default=os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")),
                    help="repository root (default: this script's repository)")
    a = ap.parse_args(argv)
    try:
        sites, problems = check(a.root)
    except Unjudgeable as e:
        print("::error::min_binary floor gate cannot judge — %s (exit %d; not a pass)" % (e, EXIT_UNJUDGEABLE))
        return EXIT_UNJUDGEABLE
    for p in problems:
        print("::error::" + p)
    if problems:
        print("min_binary floor gate: RED — %d finding(s) (exit %d)" % (len(problems), EXIT_BAD))
        return EXIT_BAD
    value = next(iter(sites.values()))
    print("min_binary floor gate: OK (exit 0) — %s in all three sites (%s); verifier step present and unconditional; "
          "not below the security floor" % (value, ", ".join(sites)))
    return EXIT_OK


if __name__ == "__main__":
    sys.exit(main())
