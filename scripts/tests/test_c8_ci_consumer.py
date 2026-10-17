"""Execute actual workflow reader bodies over isolated logs; never invoke real cargo."""
import importlib.util
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
import unittest
sys.dont_write_bytecode = True
from test_ci_branch_cysd_step import extract_step

ROOT = Path(__file__).resolve().parents[2]
BASH = os.environ.get("CYSD_STEP_BASH") or shutil.which("bash") or "/bin/bash"
MARKER_STEPS = [
    ("ci-branch.yml", "C8 UNMEASURED marker gate (branch macOS)", ["cysd-mac.log"]),
    ("release.yml", "C8 UNMEASURED marker gate (release native macOS)", ["c8-release-cysd.log"]),
    ("windows-health.yml", "C8 UNMEASURED marker gate (Windows full and hwmon)", ["cysd-win/cargo.log", "hwmon-win/cargo.log"]),
]
NAME_STEPS = [
    ("ci-branch.yml", "C8 native test names (branch macOS full group)", "cysd-mac.log"),
    ("release.yml", "C8 native test names (release macOS full group)", "c8-release-cysd.log"),
]

def import_script(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / "scripts" / (name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module

def marker():
    return "C8_NATIVE_UNMEASURED_V1 " + json.dumps({"state":"UNMEASURED", "group":"C8_NATIVE_TRANSITIONS",
        "test":"native_publication::native_transition_tests::np_fixture", "reason":"python3_unavailable"}) + "\n"

def workflow_run_bodies(path):
    lines = path.read_text(encoding="utf-8").splitlines()
    result = []
    for i, line in enumerate(lines):
        match = re.match(r"^( *)(?:run):\s*(.*)$", line)
        if not match:
            continue
        value = match.group(2)
        if value in ("|", ">"):
            indent = len(match.group(1))
            body = []
            for following in lines[i + 1:]:
                if following.strip() and len(following) - len(following.lstrip(" ")) <= indent:
                    break
                body.append(following.strip())
            result.append("\n".join(body))
        else:
            result.append(value)
    return result


def invokes_cysd_tests(line):
    # Current workflows use direct cargo commands; this also recognizes an implicit
    # all-target/default-package test route rather than demanding '--bin cysd'.
    try:
        tokens = shlex.split(line, comments=True)
    except ValueError:
        return False
    if not tokens or tokens[0] in ("echo", "printf") or "cargo" not in tokens:
        return False
    start = tokens.index("cargo") + 1
    if start < len(tokens) and tokens[start].startswith("+"):
        start += 1
    if start >= len(tokens) or tokens[start] != "test":
        return False
    args = tokens[start + 1:]
    if "--list" in args or "--no-run" in args:
        return False
    selectors = args[:args.index("--")] if "--" in args else args
    expanded = []
    for token in selectors:
        if token.startswith(("--bin=", "--package=")):
            expanded.extend(token.split("=", 1))
        else:
            expanded.append(token)
    selectors = expanded
    for flag in ("-p", "--package"):
        if flag in selectors and selectors[selectors.index(flag) + 1] == "cys-app":
            return False
    if any(flag in selectors for flag in ("--bins", "--all-targets", "--tests")):
        return True
    bins = [selectors[i + 1] for i, token in enumerate(selectors[:-1]) if token == "--bin"]
    if bins:
        return "cysd" in bins or any("$" in name for name in bins)
    if any(flag in selectors for flag in ("--lib", "--test", "--example", "--doc")):
        return False
    return True


class ReaderFixture(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="c8-ci-reader-")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.rt = self.root / "runner-temp"
        self.rt.mkdir()
        (self.root / "scripts").mkdir()
        for name in ["c8_native_measurement.py", "c8_native_names.py", "c8_native_tests.json"]:
            shutil.copyfile(ROOT / "scripts" / name, self.root / "scripts" / name)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        for name in ["python", "python3"]:
            script = self.bin / name
            script.write_text("#!/bin/sh\nexec " + shlex.quote(sys.executable) + " -B \"$@\"\n", encoding="utf-8")
            script.chmod(0o755)
        cargo = self.bin / "cargo"
        cargo.write_text("#!/bin/sh\necho unexpected-cargo-in-reader >&2\nexit 97\n", encoding="utf-8")
        cargo.chmod(0o755)
        self.env = os.environ.copy()
        self.env.update(PATH=str(self.bin) + os.pathsep + self.env.get("PATH", ""), RUNNER_TEMP=str(self.rt), PYTHONDONTWRITEBYTECODE="1")

    def log(self, relative, text):
        path = self.rt / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(text.encode("utf-8"))
        return path

    def run_body(self, workflow, step_name):
        step = extract_step(str(ROOT / ".github" / "workflows" / workflow), step_name)
        self.assertNotIn("continue-on-error", step, "marker/name gate must remain blocking")
        self.assertIn("always()", step.get("if", ""), "reader must survive cargo failure/continue-on-error")
        if workflow == "release.yml":
            self.assertEqual(step["if"], "always() && matrix.target == 'aarch64-apple-darwin'")
        else:
            self.assertEqual(step["if"], "always()")
        self.assertEqual(step.get("shell"), "bash")
        script = self.root / "reader.sh"
        script.write_text(step["run"], encoding="utf-8")
        return subprocess.run([BASH, "--noprofile", "--norc", "-e", "-o", "pipefail", str(script)],
                              cwd=self.root, env=self.env, capture_output=True, timeout=30)

class C8MarkerCliTests(ReaderFixture):
    def run_cli(self, paths):
        return subprocess.run([sys.executable, "-B", str(self.root / "scripts/c8_native_measurement.py"), *map(str, paths)],
                              cwd=self.root, env=self.env, capture_output=True, timeout=30)

    def test_same_import_api_remains_marker_only(self):
        module = import_script("c8_native_measurement")
        clean = module.classify_native_markers("test result: FAILED. 0 passed; 1 failed;\n")
        self.assertEqual(clean["measurement_state"], "NO_UNMEASURED_MARKER")
        self.assertFalse(clean["blocks_c8_gate"], "marker parser must not replace cargo's rc/coverage meaning")
        self.assertTrue(module.classify_native_markers(marker())["blocks_c8_gate"])

    def test_cli_clean_missing_unreadable_and_every_marker_shape(self):
        clean = self.log("clean.log", "test result: ok. 1 passed; 0 failed; 0 ignored;\n")
        self.assertEqual(self.run_cli([clean]).returncode, 0)
        self.assertEqual(self.run_cli([self.rt / "missing.log"]).returncode, 2)
        self.assertEqual(self.run_cli([self.rt]).returncode, 2, "directory is not a readable test log")
        for text in [marker(), marker().replace("python3_unavailable", "unknown_reason"),
                     marker().replace("UNMEASURED\"", "MEASURED\""),
                     "C8_NATIVE_UNMEASURED_V1 {bad-json}\n", "C8_NATIVE_UNMEASURED_V1", "C8_NATIVE_UNMEASURED_V2 {}\n"]:
            with self.subTest(text=text):
                bad = self.log("bad.log", text + "test result: ok. 1 passed; 0 failed;\n")
                self.assertEqual(self.run_cli([bad]).returncode, 3, "passing summary must not absorb a marker")
        a = self.log("a.log", marker())
        b = self.log("b.log", "clean\n")
        result = self.run_cli([a, b])
        self.assertEqual(result.returncode, 3)
        self.assertEqual(len(json.loads(result.stdout)["logs"]), 2, "all Windows leg logs must be read")

class C8WorkflowReaderTests(ReaderFixture):
    def test_actual_cysd_execution_inventory_and_implicit_full_routes(self):
        counts = {}
        for path in (ROOT / ".github/workflows").glob("*.yml"):
            commands = [line for body in workflow_run_bodies(path) for line in body.splitlines() if invokes_cysd_tests(line)]
            if commands:
                counts[path.name] = len(commands)
        self.assertEqual(counts, {"ci-branch.yml": 1, "release.yml": 1, "windows-health.yml": 2},
                         "cysd execution inventory changed: audit and wire every actual leg")
        for command in ["cargo test", "cargo test --workspace", "cargo test --all-targets", "cargo +stable test --bins"]:
            self.assertTrue(invokes_cysd_tests(command), command)
        for command in ["# cargo test", "echo cargo test", "cargo test --lib", "cargo test --bin cys", "cargo test -p cys-app --bins", "cargo test --bin cysd -- --list"]:
            self.assertFalse(invokes_cysd_tests(command), command)

    def test_actual_marker_step_bodies_clean_marked_and_missing_logs(self):
        for workflow, name, logs in MARKER_STEPS:
            with self.subTest(workflow=workflow, case="clean"):
                for log in logs:
                    self.log(log, "test result: ok. 1 passed; 0 failed; 0 ignored;\n")
                self.assertEqual(self.run_body(workflow, name).returncode, 0)
            for marked in logs:
                with self.subTest(workflow=workflow, case="marker", marked=marked):
                    for log in logs:
                        self.log(log, marker() if log == marked else "clean\n")
                    self.assertEqual(self.run_body(workflow, name).returncode, 3, "actual reader swallowed marker failure")
            for missing in logs:
                with self.subTest(workflow=workflow, case="missing", missing=missing):
                    for log in logs:
                        self.log(log, "clean\n")
                    (self.rt / missing).unlink()
                    self.assertEqual(self.run_body(workflow, name).returncode, 2, "actual reader accepted a missing log")

    def test_actual_marker_step_bodies_reject_unreadable_log_objects(self):
        for workflow, name, logs in MARKER_STEPS:
            for unreadable in logs:
                with self.subTest(workflow=workflow, unreadable=unreadable):
                    for log in logs:
                        self.log(log, "clean\n")
                    path = self.rt / unreadable
                    path.unlink()
                    path.mkdir()
                    try:
                        self.assertEqual(self.run_body(workflow, name).returncode, 2,
                                         "actual reader accepted an unreadable log object")
                    finally:
                        path.rmdir()

    def test_windows_reader_runs_after_both_nonblocking_diagnostics_and_has_no_name_gate(self):
        source = (ROOT / ".github/workflows/windows-health.yml").read_text(encoding="utf-8")
        gate = source.index("- name: C8 UNMEASURED marker gate (Windows full and hwmon)")
        self.assertGreater(gate, source.index("- name: hwmon-win 결과 판독"))
        for name in ["cargo test --bin cysd 감독자 축 (Windows 실기 · IG-31 · 비차단 A10 1단계)",
                     "cargo test --bin cysd hwmon 검체 (Windows 실기 · 비차단 · 위 전량 스텝과 별개)"]:
            start = source.index("- name: " + name)
            header = source[start:source.index("        run: |", start)]
            self.assertIn("continue-on-error: true", header, "existing cargo policy changed")
        self.assertNotIn("scripts/c8_native_names.py", source, "Windows/cfg-disabled and hwmon-filtered legs must not demand native group")

    def test_release_cargo_capture_keeps_its_exit_and_stops_following_tests(self):
        step = extract_step(str(ROOT / ".github/workflows/release.yml"), "phoenix host-level 로직 게이트 (macOS 네이티브)")
        phoenix = self.root / "cysjavis-pack/bin/tests"
        phoenix.mkdir(parents=True)
        (phoenix / "test_phoenix_fixture.py").write_text("# isolated successful prerequisite\n", encoding="utf-8")
        cargo = self.bin / "cargo"
        cargo.write_text("#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$FAKE_CALLS\"\n"
                         "case \"$*\" in 'test --bin cysd '*) printf '%s\\n' \"$FAKE_LOG\"; exit \"$FAKE_RC\";; esac\nexit 0\n", encoding="utf-8")
        cargo.chmod(0o755)
        script = self.root / "release-test.sh"
        script.write_text(step["run"], encoding="utf-8")
        for rc in [0, 101, 137]:
            calls = self.root / "calls"
            calls.unlink(missing_ok=True)
            env = self.env.copy()
            env.update(FAKE_RC=str(rc), FAKE_LOG="actual fake cysd output", FAKE_CALLS=str(calls), TMPDIR=str(self.rt))
            result = subprocess.run([BASH, "--noprofile", "--norc", "-e", "-o", "pipefail", str(script)],
                                    cwd=self.root, env=env, capture_output=True, timeout=30)
            self.assertEqual(result.returncode, rc, "release capture changed cargo rc")
            self.assertEqual((self.rt / "c8-release-cysd.log").read_text(), "actual fake cysd output\n")
            invoked = calls.read_text().splitlines()
            self.assertEqual(any(x.startswith("test --lib") for x in invoked), rc == 0)

class C8NativeNameTests(ReaderFixture):
    def expected(self):
        return json.loads((ROOT / "scripts/c8_native_tests.json").read_text(encoding="utf-8"))

    def test_frozen_names_match_actual_native_test_functions(self):
        source = (ROOT / "src/bin/cysd/native_publication_transition_tests.rs").read_text(encoding="utf-8")
        found = re.findall(r"(?m)^#\[test\]\s*\nfn ([A-Za-z0-9_]+)\(", source)
        expected = ["native_publication::native_transition_tests::" + name for name in found]
        self.assertTrue(expected)
        self.assertEqual(sorted(self.expected()), sorted(expected), "native group manifest/source drift")

    def test_actual_name_steps_are_separate_and_reject_missing_ignored_duplicate_names(self):
        names = self.expected()
        complete = "".join("test " + name + " ... ok\n" for name in names)
        for workflow, step, log in NAME_STEPS:
            for label, body, rc in [("complete", complete, 0),
                                    ("clean-without-coverage", "clean log\n", 4),
                                    ("missing", complete.replace("test " + names[0] + " ... ok\n", "", 1), 4),
                                    ("ignored", complete.replace(" ... ok", " ... ignored", 1), 4),
                                    ("failed", complete.replace(" ... ok", " ... FAILED", 1), 4),
                                    ("duplicate", complete + "test " + names[0] + " ... ok\n", 4)]:
                with self.subTest(workflow=workflow, case=label):
                    self.log(log, body)
                    self.assertEqual(self.run_body(workflow, step).returncode, rc)

if __name__ == "__main__":
    unittest.main()
