"""Pure output and actual-workflow-body controls; no Rust build or native PTY.

Run with: python3 scripts/tests/test_unix_pty_results.py
Like test_ci_branch_cysd_step, run the extracted shell bodies with fake Cargo.
The helper itself is real, and complete Cargo output/exit codes are preserved.
"""

import importlib.util
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

from test_ci_branch_cysd_step import extract_step


ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location(
    "verify_unix_pty_results", ROOT / "scripts/verify_unix_pty_results.py")
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)
WORKFLOW = Path(os.environ.get("CI_BRANCH_YML_UNDER_TEST",
                               ROOT / ".github/workflows/ci-branch.yml"))
STEPS = tuple("Unix PTY allocator tests (" + platform + ", blocking)"
              for platform in ("macOS", "Linux"))


def output(states=None, extra=()):
    # Reverse completion order intentionally: default libtest is parallel.
    states = list(states if states is not None else ((pin, "ok") for pin in gate.REQUIRED))
    states.extend(extra)
    totals = {kind: sum(status.startswith(kind) for _, status in states)
              for kind in ("ok", "FAILED", "ignored")}
    lines = ["   Compiling portable-pty", "", "running %d tests" % len(states)]
    lines += ["test %s ... %s" % row for row in reversed(states)]
    lines += ["", "test result: %s. %d passed; %d failed; %d ignored; 0 measured; "
              "0 filtered out; finished in 0.01s" % (
                  "FAILED" if totals["FAILED"] else "ok", totals["ok"],
                  totals["FAILED"], totals["ignored"]), ""]
    return "\n".join(lines)


def changed(pin, status):
    return [(name, status if name == pin else "ok") for name in gate.REQUIRED]


class OutputControls(unittest.TestCase):
    def test_all_seven_ok_in_parallel_order_with_other_tests(self):
        self.assertEqual(gate.verify_output(output(extra=[("other::one", "ok"),
                                                          ("other::two", "ok")])), [])

    def test_each_ignored_pin_refuses_even_with_successful_summary(self):
        for pin in gate.REQUIRED:
            with self.subTest(pin=pin):
                self.assertTrue(gate.verify_output(output(changed(pin, "ignored, disabled"))))

    def test_each_absent_pin_refuses_with_truthful_smaller_summary(self):
        for pin in gate.REQUIRED:
            with self.subTest(pin=pin):
                self.assertTrue(gate.verify_output(output([(name, "ok") for name in gate.REQUIRED
                                                           if name != pin])))

    def test_each_failed_pin_refuses(self):
        for pin in gate.REQUIRED:
            with self.subTest(pin=pin):
                self.assertTrue(gate.verify_output(output(changed(pin, "FAILED"))))

    def test_list_only_cannot_pass(self):
        listing = "\n".join(pin + ": test" for pin in gate.REQUIRED) + "\n7 tests, 0 benchmarks\n"
        self.assertTrue(gate.verify_output(listing))

    def test_bad_or_missing_summary_and_duplicate_result_refuse(self):
        good = output()
        variants = [good.replace("test result: ok.", "test result: FAILED."),
                    good.replace("7 passed", "8 passed"),
                    good[:good.index("test result:")],
                    output(extra=[(gate.REQUIRED[0], "ok")])]
        for text in variants:
            with self.subTest(text=text):
                self.assertTrue(gate.verify_output(text))


class WorkflowControls(unittest.TestCase):
    def run_body(self, step, text, cargo_rc=0, tee_rc=0):
        with tempfile.TemporaryDirectory(prefix="pty-execution-gate-") as temporary:
            folder = Path(temporary)
            binary = folder / "bin"
            binary.mkdir()
            os.symlink(sys.executable, binary / "python3")
            cargo = binary / "cargo"
            cargo.write_text("#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$F2_ARGV\"\n"
                             "case \" $* \" in\n*' --list '*) cat \"$F2_LIST\"; exit 0;;\n"
                             "*) cat \"$F2_RUN\"; exit \"$F2_RC\";;\nesac\n")
            cargo.chmod(0o755)
            if tee_rc:
                tee = binary / "tee"
                tee.write_text('#!/bin/sh\n/usr/bin/tee "$@"\nexit ' + str(tee_rc) + '\n')
                tee.chmod(0o755)
            (folder / "list").write_text("".join(pin + ": test\n" for pin in gate.REQUIRED))
            (folder / "run").write_text(text)
            script = folder / "step.sh"
            script.write_text(step["run"])
            env = {"PATH": str(binary) + ":/usr/bin:/bin", "RUNNER_TEMP": str(folder),
                   "LANG": "C", "LC_ALL": "C", "PYTHONDONTWRITEBYTECODE": "1",
                   "F2_ARGV": str(folder / "argv"), "F2_LIST": str(folder / "list"),
                   "F2_RUN": str(folder / "run"), "F2_RC": str(cargo_rc)}
            result = subprocess.run([shutil.which("bash") or "/bin/bash", "--noprofile",
                                     "--norc", "-e", "-o", "pipefail", str(script)],
                                    cwd=ROOT, env=env, capture_output=True, timeout=20)
            log = folder / "portable-pty-unix-run.log"
            return result, log.read_text() if log.exists() else None, (folder / "argv").read_text()

    def test_same_blocking_body_on_both_platforms(self):
        steps = [extract_step(str(WORKFLOW), name) for name in STEPS]
        self.assertEqual(steps[0]["run"], steps[1]["run"])
        for step in steps:
            self.assertNotIn("if", step)
            self.assertNotIn("continue-on-error", step)
            self.assertEqual(step["shell"], "bash")

    def test_actual_bodies_require_executed_pins(self):
        pin = gate.REQUIRED[0]
        cases = [(output(), True), (output(changed(pin, "ignored")), False),
                 (output([(name, "ok") for name in gate.REQUIRED if name != pin]), False),
                 (output(changed(pin, "FAILED")), False),
                 ("\n".join(name + ": test" for name in gate.REQUIRED), False)]
        for name in STEPS:
            step = extract_step(str(WORKFLOW), name)
            for text, accepted in cases:
                with self.subTest(step=name, accepted=accepted, text=text):
                    result, captured, args = self.run_body(step, text)
                    self.assertEqual(result.returncode == 0, accepted,
                                     result.stdout.decode(errors="replace") + result.stderr.decode(errors="replace"))
                    self.assertEqual(captured, text)
                    self.assertEqual(args.splitlines(), [
                        "test --manifest-path vendor/portable-pty/Cargo.toml --lib --locked -- --list",
                        "test --manifest-path vendor/portable-pty/Cargo.toml --lib --locked -- --format pretty --color never"])

    def test_cargo_and_tee_failures_cannot_be_masked_by_ok_results(self):
        for name in STEPS:
            step = extract_step(str(WORKFLOW), name)
            for cargo_rc, tee_rc, expected in [(101, 0, 101), (0, 23, 23), (101, 23, 101)]:
                with self.subTest(step=name, cargo_rc=cargo_rc, tee_rc=tee_rc):
                    result, captured, _ = self.run_body(step, output(), cargo_rc, tee_rc)
                    self.assertEqual(result.returncode, expected)
                    self.assertEqual(captured, output())


if __name__ == "__main__":
    unittest.main()
