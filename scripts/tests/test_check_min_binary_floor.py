"""Hermetic controls for scripts/check_min_binary_floor.py — the min_binary three-site equality gate
(0.15.0 「스스로 해결」 T2c · M11 카 · CONTRACTS O-C8-5 (ii) · testplan H-11ka-N1-01, H-11ka-N2-01, G-1-26).

A gate that is only ever seen passing cannot be told apart from one that always passes, so every finding is planted
here on a temp copy of the four real input files and must turn the gate red (exit 1) naming its site, while the
unmodified copy and the real tree stay green (exit 0). Raising all three sites together (the release commit) stays
green. A missing input is exit 2, never a pass. Two guard rails of design §6-11 카 are covered: the min_binary
verifier step of pack-release.yml (N1, mutations: removed / `if:` / `continue-on-error` / no longer runs the dry run)
and security-floor.txt (N2: pinned byte for byte to its v0.14.56 content, and the floor never below it).

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
INPUTS = (RELEASE_YML, PACK_YML, RELEASE_MD, FLOOR_TXT)
# security-floor.txt at v0.14.56 (git blob 36d365788464a5db20c672d3a16a30fba7b8fa28). Design §6-11 카: release 1 does
# not touch the security floor file. Changing it deliberately later means changing this pin in the same commit.
SECURITY_FLOOR_SHA256_V0_14_56 = "2ae797a7907ddd8434699f3bb8b62ef8689f5f1ded9c1127d464fd3416feb42f"


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


class FloorGate(unittest.TestCase):
    def setUp(self):
        self.root = tempfile.mkdtemp(prefix="min-binary-floor-")
        for rel in INPUTS:
            dst = os.path.join(self.root, rel)
            os.makedirs(os.path.dirname(dst) or self.root, exist_ok=True)
            shutil.copyfile(os.path.join(ROOT, rel), dst)

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
        # the release commit of 0.15.0 (design §4-11 카, testplan G-1-26): one value in all three sites
        self.edit(RELEASE_YML, lambda t: set_yaml_value(t, "PACK_MIN_BINARY", "0.15.0"))
        self.edit(PACK_YML, lambda t: set_yaml_value(t, "PACK_MIN_BINARY_OVERRIDE", "0.15.0"))
        self.edit(RELEASE_MD, lambda t: set_md_value(t, "0.15.0"))
        rc, out = self.run_gate()
        self.assertEqual(rc, 0, out)
        self.assertIn("0.15.0 in all three sites", out)

    def test_a_commented_assignment_is_ignored(self):
        self.edit(RELEASE_YML, lambda t: t.replace("    steps:", "      # PACK_MIN_BINARY: '9.9.9'\n    steps:", 1))
        rc, out = self.run_gate()
        self.assertEqual(rc, 0, out)

    # -- equality: each site alone --
    def test_release_yml_raised_alone_is_red(self):
        self.edit(RELEASE_YML, lambda t: set_yaml_value(t, "PACK_MIN_BINARY", "0.15.0"))
        self.red("the three min_binary sites differ", "release.yml", "0.15.0")

    def test_pack_release_override_raised_alone_is_red(self):
        self.edit(PACK_YML, lambda t: set_yaml_value(t, "PACK_MIN_BINARY_OVERRIDE", "0.15.0"))
        self.red("the three min_binary sites differ", "pack-release.yml", "0.15.0")

    def test_release_md_literal_raised_alone_is_red(self):
        self.edit(RELEASE_MD, lambda t: set_md_value(t, "0.15.0"))
        self.red("the three min_binary sites differ", "docs/RELEASE.md")

    def test_two_raised_one_forgotten_is_red(self):
        self.edit(RELEASE_YML, lambda t: set_yaml_value(t, "PACK_MIN_BINARY", "0.15.0"))
        self.edit(RELEASE_MD, lambda t: set_md_value(t, "0.15.0"))
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
        self.edit(RELEASE_YML, lambda t: set_yaml_value(t, "PACK_MIN_BINARY", "0.12.47"))
        self.edit(PACK_YML, lambda t: set_yaml_value(t, "PACK_MIN_BINARY_OVERRIDE", "0.12.47"))
        self.edit(RELEASE_MD, lambda t: set_md_value(t, "0.12.47"))
        self.red("N2", "below the security floor")

    def test_n2_security_floor_file_is_unchanged_from_v0_14_56(self):
        with open(os.path.join(ROOT, FLOOR_TXT), "rb") as f:
            got = hashlib.sha256(f.read()).hexdigest()
        self.assertEqual(got, SECURITY_FLOOR_SHA256_V0_14_56,
                         "security-floor.txt changed — design §6-11 카: release 1 does not touch the security floor file")

    # -- cannot judge --
    def test_missing_input_is_exit_2_not_a_pass(self):
        os.remove(os.path.join(self.root, RELEASE_MD))
        rc, out = self.run_gate()
        self.assertEqual(rc, 2, out)
        self.assertIn("cannot judge", out)


if __name__ == "__main__":
    unittest.main()
