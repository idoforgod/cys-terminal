"""Hermetic controls for scripts/check_min_binary_floor.py — the min_binary three-site equality gate
(0.15.0 「스스로 해결」 T2c · M11 카 · CONTRACTS O-C8-5 (ii) · testplan H-11ka-N1-01, H-11ka-N2-01, G-1-26).

A gate that is only ever seen passing cannot be told apart from one that always passes, so every finding is planted
here on a temp copy of the five real input files (the three sites, security-floor.txt, Cargo.toml) and must turn the gate red (exit 1) naming its site, while the
unmodified copy and the real tree stay green (exit 0). Raising all three sites together (the release commit) stays
green, and a 0.15.0+ program version with a floor below 0.15.0 is red (RULINGS AE3). A missing input is exit 2, never a
pass. Two guard rails of design §6-11 카 are covered: the min_binary
verifier step of pack-release.yml (N1, mutations: removed / `if:` / `continue-on-error` / no longer runs the dry run)
and security-floor.txt (N2: pinned byte for byte to its v0.14.56 content, and the floor never below it).

Every "other" value a control writes (a raised floor, a raised program version, a floor below the security floor) is
DERIVED from the copied tree and asserted to differ from the tree's own value (RULINGS EU1, 2026-11-14), so the controls
hold at every version - at the 0.15.0 version commit nine of them wrote the literal 0.15.0 over 0.15.0 and failed their
own no-op guard. The one literal kept is the AE3 boundary 0.15.0 (AE3_BOUNDARY), the rule's own constant.

No network, no git: the copies are plain files in a temp dir.

    python3 scripts/tests/test_check_min_binary_floor.py
"""
import hashlib
import os
import re
import shutil
import subprocess
import sys
import tempfile
import unittest

sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, "..", ".."))
# Mutation testing: MIN_BINARY_FLOOR_GATE_UNDER_TEST=<mutant path> runs the same cases against that script instead.
SCRIPT = os.environ.get("MIN_BINARY_FLOOR_GATE_UNDER_TEST") or os.path.join(ROOT, "scripts", "check_min_binary_floor.py")
RELEASE_YML = ".github/workflows/release.yml"
PACK_YML = ".github/workflows/pack-release.yml"
RELEASE_MD = "docs/RELEASE.md"
FLOOR_TXT = "security-floor.txt"
CARGO_TOML = "Cargo.toml"
INPUTS = (RELEASE_YML, PACK_YML, RELEASE_MD, FLOOR_TXT, CARGO_TOML)
# security-floor.txt at v0.14.56 (git blob 36d365788464a5db20c672d3a16a30fba7b8fa28). Design §6-11 카: release 1 does
# not touch the security floor file. Changing it deliberately later means changing this pin in the same commit.
SECURITY_FLOOR_SHA256_V0_14_56 = "2ae797a7907ddd8434699f3bb8b62ef8689f5f1ded9c1127d464fd3416feb42f"
# RULINGS AE3: "program version >= 0.15.0 => floor >= 0.15.0" (= check_min_binary_floor.SELF_REPAIR_RELEASE). Kept as a
# literal on purpose: it is the boundary the rule names, not a stand-in for "some other version" (RULINGS EU1).
AE3_BOUNDARY = "0.15.0"


def vtuple(v):
    return tuple(int(x) for x in v.split("."))


def vstr(t):
    return ".".join(str(n) for n in t)


def above(v):
    """The next patch version - always above v, so always different from it (0.15.0 -> 0.15.1, 0.0.0 -> 0.0.1)."""
    a, b, c = vtuple(v)
    return vstr((a, b, c + 1))


def below(v):
    """A version below v: its last non-zero component lowered by one (0.12.48 -> 0.12.47, 0.13.0 -> 0.12.0,
    1.0.0 -> 0.0.0). Nothing is below 0.0.0 -> AssertionError: the control cannot be built, a loud failure."""
    nums = list(vtuple(v))
    for i in range(len(nums) - 1, -1, -1):
        if nums[i] > 0:
            nums[i] -= 1
            return vstr(nums)
    raise AssertionError("premise broken: there is no version below %s" % v)


def raised(v):
    """How the next release commit raises a value: to the AE3 boundary while below it, else one patch up
    (0.14.52 -> 0.15.0, 0.14.56 -> 0.15.0, 0.15.0 -> 0.15.1, 1.0.0 -> 1.0.1, 0.0.0 -> 0.15.0). Always >= the boundary
    and always different from v. At a tree below the boundary this is exactly the value the controls used to write."""
    return AE3_BOUNDARY if vtuple(v) < vtuple(AE3_BOUNDARY) else above(v)


def set_yaml_value(text, key, value):
    """Replace the value of the single non-comment `KEY: '…'` line (value-agnostic, so the controls survive the bump)."""
    pat = re.compile(r"^(\s*" + re.escape(key) + r"\s*:\s*)'[^']*'(.*)$")
    out, n = [], 0
    for line in text.split("\n"):
        m = None if line.lstrip().startswith("#") else pat.match(line)
        if m:
            line, n = m.group(1) + "'" + value + "'" + m.group(2), n + 1
        out.append(line)
    assert n == 1, "fixture: expected one %s assignment, found %d" % (key, n)
    return "\n".join(out)


def set_md_value(text, value):
    out, n = [], 0
    for line in text.split("\n"):
        if "`$PACK_MIN_BINARY`" in line and "현행" in line:
            line, k = re.subn(r"(현행\s+)\d+\.\d+\.\d+", lambda m: m.group(1) + value, line)
            n += k
        out.append(line)
    assert n == 1, "fixture: expected one '현행 X.Y.Z' literal, found %d" % n
    return "\n".join(out)


def set_cargo_version(text, value):
    """The [package] version of Cargo.toml (the first `version = "…"` line at column 0)."""
    new, n = re.subn(r'(?m)^version = "\d+\.\d+\.\d+"', 'version = "%s"' % value, text, count=1)
    assert n == 1, "fixture: no [package] version line"
    return new


def get_yaml_value(text, key):
    """The value of the single non-comment `KEY: '…'` line (the reader twin of set_yaml_value)."""
    pat = re.compile(r"^\s*" + re.escape(key) + r"\s*:\s*'([^']*)'")
    vals = [m.group(1) for line in text.split("\n")
            if not line.lstrip().startswith("#") for m in [pat.match(line)] if m]
    assert len(vals) == 1, "fixture: expected one %s assignment, found %d" % (key, len(vals))
    return vals[0]


def get_md_value(text):
    vals = []
    for line in text.split("\n"):
        if "`$PACK_MIN_BINARY`" in line and "현행" in line:
            vals += re.findall(r"현행\s+(\d+\.\d+\.\d+)", line)
    assert len(vals) == 1, "fixture: expected one '현행 X.Y.Z' literal, found %d" % len(vals)
    return vals[0]


def get_cargo_version(text):
    m = re.search(r'(?m)^version = "(\d+\.\d+\.\d+)"', text)
    assert m, "fixture: no [package] version line"
    return m.group(1)


def raise_floor(case, value):
    case.edit(RELEASE_YML, lambda t: set_yaml_value(t, "PACK_MIN_BINARY", value))
    case.edit(PACK_YML, lambda t: set_yaml_value(t, "PACK_MIN_BINARY_OVERRIDE", value))
    case.edit(RELEASE_MD, lambda t: set_md_value(t, value))


class FloorGate(unittest.TestCase):
    def setUp(self):
        self.root = tempfile.mkdtemp(prefix="min-binary-floor-")
        for rel in INPUTS:
            dst = os.path.join(self.root, rel)
            os.makedirs(os.path.dirname(dst) or self.root, exist_ok=True)
            shutil.copyfile(os.path.join(ROOT, rel), dst)
        # the tree's own values - every "other" value below is derived from these (RULINGS EU1)
        self.program = get_cargo_version(self.read(CARGO_TOML))
        floors = {get_yaml_value(self.read(RELEASE_YML), "PACK_MIN_BINARY"),
                  get_yaml_value(self.read(PACK_YML), "PACK_MIN_BINARY_OVERRIDE"),
                  get_md_value(self.read(RELEASE_MD))}
        self.assertEqual(len(floors), 1, "premise broken: the tree's three floor sites already differ: %s" % sorted(floors))
        self.floor = floors.pop()
        self.security_floor = [l.strip() for l in self.read(FLOOR_TXT).split("\n")
                               if l.strip() and not l.strip().startswith("#")][0]

    def read(self, rel):
        with open(os.path.join(self.root, rel), encoding="utf-8") as f:
            return f.read()

    def differs(self, derived, current, what):
        """RULINGS EU1: a derived "other" value that equals the tree's value would make the mutation a silent no-op."""
        self.assertNotEqual(derived, current, "premise broken: the derived %s %s equals the tree's own value" % (what, derived))
        return derived

    def forgotten_floor(self):
        """A floor below the AE3 boundary for the site(s) a release commit forgot: the tree's own floor while it is below
        the boundary (nothing to plant), else the security floor - the lowest legal floor, so N2 stays quiet."""
        if vtuple(self.floor) < vtuple(AE3_BOUNDARY):
            return self.floor
        low = self.differs(self.security_floor, self.floor, "forgotten floor")
        self.assertLess(vtuple(low), vtuple(AE3_BOUNDARY),
                        "premise broken: the security floor %s is not below the AE3 boundary %s" % (low, AE3_BOUNDARY))
        return low

    def ensure_floor(self, value):
        """All three sites at VALUE - the AE3 boundary literal, which the tree may already hold: write only the sites
        that differ, then read every site back (a missed anchor is a loud failure, not a silent pass)."""
        if value != self.floor:
            raise_floor(self, value)
        self.assertEqual(get_yaml_value(self.read(RELEASE_YML), "PACK_MIN_BINARY"), value)
        self.assertEqual(get_yaml_value(self.read(PACK_YML), "PACK_MIN_BINARY_OVERRIDE"), value)
        self.assertEqual(get_md_value(self.read(RELEASE_MD)), value)

    def tearDown(self):
        shutil.rmtree(self.root, ignore_errors=True)

    def edit(self, rel, fn):
        p = os.path.join(self.root, rel)
        with open(p, encoding="utf-8") as f:
            text = f.read()
        new = fn(text)
        self.assertNotEqual(new, text, "fixture edit changed nothing in %s" % rel)
        with open(p, "w", encoding="utf-8") as f:
            f.write(new)

    def run_gate(self, root=None):
        r = subprocess.run([sys.executable, SCRIPT, "--root", root or self.root],
                           capture_output=True, text=True, timeout=60)
        return r.returncode, r.stdout + r.stderr

    def red(self, *needles):
        rc, out = self.run_gate()
        self.assertEqual(rc, 1, out)
        for n in needles:
            self.assertIn(n, out)
        return out

    # -- green controls --
    def test_real_repository_tree_passes(self):
        rc, out = self.run_gate(root=ROOT)
        self.assertEqual(rc, 0, out)
        self.assertIn("in all three sites", out)

    def test_unmodified_copy_passes(self):
        rc, out = self.run_gate()
        self.assertEqual(rc, 0, out)
        self.assertIn("OK (exit 0)", out)

    def test_all_three_raised_together_passes(self):
        # the release commit (design §4-11 카, testplan G-1-26): one raised value in all three sites
        new = self.differs(raised(self.floor), self.floor, "floor")
        self.edit(RELEASE_YML, lambda t: set_yaml_value(t, "PACK_MIN_BINARY", new))
        self.edit(PACK_YML, lambda t: set_yaml_value(t, "PACK_MIN_BINARY_OVERRIDE", new))
        self.edit(RELEASE_MD, lambda t: set_md_value(t, new))
        rc, out = self.run_gate()
        self.assertEqual(rc, 0, out)
        self.assertIn("%s in all three sites" % new, out)

    def test_a_commented_assignment_is_ignored(self):
        self.edit(RELEASE_YML, lambda t: t.replace("    steps:", "      # PACK_MIN_BINARY: '9.9.9'\n    steps:", 1))
        rc, out = self.run_gate()
        self.assertEqual(rc, 0, out)

    # -- equality: each site alone --
    def test_release_yml_raised_alone_is_red(self):
        new = self.differs(raised(self.floor), self.floor, "floor")
        self.edit(RELEASE_YML, lambda t: set_yaml_value(t, "PACK_MIN_BINARY", new))
        self.red("the three min_binary sites differ", "release.yml", new)

    def test_pack_release_override_raised_alone_is_red(self):
        new = self.differs(raised(self.floor), self.floor, "floor")
        self.edit(PACK_YML, lambda t: set_yaml_value(t, "PACK_MIN_BINARY_OVERRIDE", new))
        self.red("the three min_binary sites differ", "pack-release.yml", new)

    def test_release_md_literal_raised_alone_is_red(self):
        new = self.differs(raised(self.floor), self.floor, "floor")
        self.edit(RELEASE_MD, lambda t: set_md_value(t, new))
        self.red("the three min_binary sites differ", "docs/RELEASE.md")

    def test_two_raised_one_forgotten_is_red(self):
        new = self.differs(raised(self.floor), self.floor, "floor")
        self.edit(RELEASE_YML, lambda t: set_yaml_value(t, "PACK_MIN_BINARY", new))
        self.edit(RELEASE_MD, lambda t: set_md_value(t, new))
        self.red("the three min_binary sites differ", "PACK_MIN_BINARY_OVERRIDE")

    # -- shape of each site --
    def test_empty_pack_only_override_is_red(self):
        self.edit(PACK_YML, lambda t: set_yaml_value(t, "PACK_MIN_BINARY_OVERRIDE", ""))
        self.red("PACK_MIN_BINARY_OVERRIDE", "is not X.Y.Z")

    def test_a_second_assignment_is_red(self):
        self.edit(RELEASE_YML, lambda t: t.replace("    steps:", "      PACK_MIN_BINARY: '0.14.52'\n    steps:", 1))
        self.red("expected exactly one non-comment `PACK_MIN_BINARY:` assignment, found 2")

    def test_missing_assignment_is_red(self):
        self.edit(PACK_YML, lambda t: re.sub(r"(?m)^(\s*)PACK_MIN_BINARY_OVERRIDE:", r"\1# PACK_MIN_BINARY_OVERRIDE:", t))
        self.red("found 0")

    def test_release_md_literal_missing_is_red(self):
        self.edit(RELEASE_MD, lambda t: re.sub(r"현행 (\d+\.\d+\.\d+)", r"\1", t))
        self.red("docs/RELEASE.md", "found 0")

    # -- the values must reach the signed manifest --
    def test_release_lane_no_longer_signs_with_the_value_is_red(self):
        self.edit(RELEASE_YML, lambda t: t.replace('--min-binary-version "$PACK_MIN_BINARY"', '--min-binary-version "0.12.48"'))
        self.red('--min-binary-version "$PACK_MIN_BINARY"')

    def test_pack_only_lane_ignores_the_override_is_red(self):
        self.edit(PACK_YML, lambda t: t.replace('MB="$PACK_MIN_BINARY_OVERRIDE"', 'MB="0.12.48"'))
        self.red("no longer takes PACK_MIN_BINARY_OVERRIDE into MIN_BINARY")

    # -- N1: the min_binary verifier (hard-floor validator) of pack-release.yml --
    def test_n1_verifier_step_removed_is_red(self):
        self.edit(PACK_YML, lambda t: t.replace("- name: Verify with min_binary verifier (해법②)", "- name: Verify (renamed)"))
        self.red("N1", "is missing")

    def test_n1_verifier_step_made_conditional_is_red(self):
        self.edit(PACK_YML, lambda t: t.replace("      - name: Verify with min_binary verifier (해법②)\n",
                                                 "      - name: Verify with min_binary verifier (해법②)\n        if: false\n"))
        self.red("N1", "conditional")

    def test_n1_condition_written_before_the_name_is_red(self):
        self.edit(PACK_YML, lambda t: t.replace("      - name: Verify with min_binary verifier (해법②)\n",
                                                 "      - if: false\n        name: Verify with min_binary verifier (해법②)\n"))
        self.red("N1", "conditional")

    def test_n1_verifier_continue_on_error_is_red(self):
        self.edit(PACK_YML, lambda t: t.replace("      - name: Verify with min_binary verifier (해법②)\n",
                                                 "      - name: Verify with min_binary verifier (해법②)\n        continue-on-error: true\n"))
        self.red("N1", "continue-on-error")

    def test_n1_verifier_no_longer_runs_the_dry_run_is_red(self):
        self.edit(PACK_YML, lambda t: t.replace('"$BIN" pack-update --from . --dry-run', '"$BIN" --version'))
        self.red("N1", "no longer runs")

    # -- N2: the security floor --
    def test_n2_floor_below_the_security_floor_is_red(self):
        low = self.differs(below(self.security_floor), self.floor, "floor")   # 0.12.47 while security-floor.txt = 0.12.48
        self.edit(RELEASE_YML, lambda t: set_yaml_value(t, "PACK_MIN_BINARY", low))
        self.edit(PACK_YML, lambda t: set_yaml_value(t, "PACK_MIN_BINARY_OVERRIDE", low))
        self.edit(RELEASE_MD, lambda t: set_md_value(t, low))
        self.red("N2", "below the security floor")

    def test_n2_security_floor_file_is_unchanged_from_v0_14_56(self):
        with open(os.path.join(ROOT, FLOOR_TXT), "rb") as f:
            got = hashlib.sha256(f.read()).hexdigest()
        self.assertEqual(got, SECURITY_FLOOR_SHA256_V0_14_56,
                         "security-floor.txt changed — design §6-11 카: release 1 does not touch the security floor file")

    # -- AE3: a 0.15.0+ program version needs a 0.15.0+ floor (RULINGS AE3) --
    def test_ae3_program_version_raised_without_the_floor_is_red(self):
        # the release commit that bumps the version but forgets the three floor sites
        program = self.differs(raised(self.program), self.program, "program version")   # >= the AE3 boundary
        low = self.forgotten_floor()
        self.edit(CARGO_TOML, lambda t: set_cargo_version(t, program))
        if low != self.floor:   # the tree already carries a raised floor: plant the forgotten one
            raise_floor(self, low)
        self.red("AE3", "Cargo.toml version is %s" % program, "below %s" % AE3_BOUNDARY)

    def test_ae3_release_commit_with_version_and_floor_raised_passes(self):
        program = self.differs(raised(self.program), self.program, "program version")
        floor = self.differs(raised(self.floor), self.floor, "floor")
        self.edit(CARGO_TOML, lambda t: set_cargo_version(t, program))
        raise_floor(self, floor)
        rc, out = self.run_gate()
        self.assertEqual(rc, 0, out)

    def test_ae3_a_later_program_with_the_0_15_0_floor_passes(self):
        # the floor exactly AT the boundary (literal by intent) stays green under a program later than the boundary
        program = self.differs(above(max(self.program, AE3_BOUNDARY, key=vtuple)), self.program, "program version")
        self.edit(CARGO_TOML, lambda t: set_cargo_version(t, program))
        self.ensure_floor(AE3_BOUNDARY)
        rc, out = self.run_gate()
        self.assertEqual(rc, 0, out)

    def test_ae3_floor_raised_in_two_sites_only_names_the_low_one(self):
        program = self.differs(raised(self.program), self.program, "program version")
        high = self.differs(raised(self.floor), self.floor, "floor")
        low = self.forgotten_floor()
        self.edit(CARGO_TOML, lambda t: set_cargo_version(t, program))
        self.edit(RELEASE_YML, lambda t: set_yaml_value(t, "PACK_MIN_BINARY", high))
        self.edit(RELEASE_MD, lambda t: set_md_value(t, high))
        if low != self.floor:   # the tree already carries a raised floor: plant the forgotten one
            self.edit(PACK_YML, lambda t: set_yaml_value(t, "PACK_MIN_BINARY_OVERRIDE", low))
        out = self.red("AE3", "the three min_binary sites differ")
        self.assertIn("PACK_MIN_BINARY_OVERRIDE = ", out.split("AE3", 1)[1])

    def test_ae3_missing_program_version_is_exit_2(self):
        self.edit(CARGO_TOML, lambda t: re.sub(r'(?m)^version = "[^"]*"\n', "", t, count=1))
        rc, out = self.run_gate()
        self.assertEqual(rc, 2, out)
        self.assertIn("Cargo.toml", out)

    # -- cannot judge --
    def test_missing_input_is_exit_2_not_a_pass(self):
        os.remove(os.path.join(self.root, RELEASE_MD))
        rc, out = self.run_gate()
        self.assertEqual(rc, 2, out)
        self.assertIn("cannot judge", out)


if __name__ == "__main__":
    unittest.main()
