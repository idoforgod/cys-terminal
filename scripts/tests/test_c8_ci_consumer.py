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

# Producer steps whose cargo log a blocking C8 marker gate reads: (workflow, step name) -> log under RUNNER_TEMP.
# Any other cysd test run must use filters that cannot select a C8 native test (native_selection below).
GATED_CYSD_LEGS = {
    ("ci-branch.yml", "cargo test --bin cysd (데몬 단위 검체 · 브랜치 레인 편입 · macOS)"): "cysd-mac.log",
    ("release.yml", "phoenix host-level 로직 게이트 (macOS 네이티브)"): "c8-release-cysd.log",
    ("windows-health.yml", "cargo test --bin cysd 감독자 축 (Windows 실기 · IG-31 · 비차단 A10 1단계)"): "cysd-win/cargo.log",
    ("windows-health.yml", "cargo test --bin cysd hwmon 검체 (Windows 실기 · 비차단 · 위 전량 스텝과 별개)"): "hwmon-win/cargo.log",
}
# Fail-closed flag sets: a flag missing here makes a run count as able to select a native test.
# Add only flags that cannot change WHICH tests run (never a filter-, target- or package-widening flag).
CARGO_VALUE_FLAGS = {"--bin", "-p", "--package", "--manifest-path", "--target", "--target-dir", "--features", "-F",
                     "--profile", "-j", "--jobs", "--color", "--config", "-Z", "--message-format", "--exclude",
                     "--test", "--example", "--bench"}
CARGO_SWITCHES = {"--offline", "--locked", "--frozen", "--release", "--no-fail-fast", "--all-features",
                  "--no-default-features", "--workspace", "--all", "-q", "--quiet", "-v", "-vv", "--verbose", "--bins",
                  "--lib", "--tests", "--all-targets", "--doc", "--examples", "--benches", "--ignore-rust-version"}
LIBTEST_VALUE_FLAGS = {"--skip", "--test-threads", "--format", "--logfile", "--color", "-Z", "--shuffle-seed"}
LIBTEST_SWITCHES = {"--nocapture", "--no-capture", "--exact", "--ignored", "--include-ignored", "-q", "--quiet",
                    "--show-output", "--test", "--bench", "--force-run-in-process", "--report-time", "--ensure-time",
                    "--shuffle", "--exclude-should-panic"}
SHELL_PREFIX_WORDS = {"!", "{", "}", "if", "then", "elif", "else", "do", "while", "until", "time"}
# Subcommands that are not refused. Add only subcommands that cannot execute tests; one that can (llvm-cov, tarpaulin,
# xtask, ...) must be classified in cargo_test_args instead, as test/t and nextest are (R-C8MIN-1 issue 5).
KNOWN_CARGO_SUBCOMMANDS = {"test", "t", "nextest", "build", "b", "check", "c", "clean", "run", "r", "fmt", "clippy", "doc",
                           "d", "metadata", "fetch", "tree", "install", "update", "generate-lockfile", "pkgid",
                           "locate-project", "package", "publish", "rustc", "rustdoc", "bench", "fix", "vendor",
                           "version", "--version", "-V", "help"}

# Second net (R-C8MIN-1 issue 2): a wrapper carries a whole command in one quoted word, which the cargo word search
# never sees. A command naming a wrapper with a later word that contains a cargo word (cargo, cargo.exe, cargo-nextest,
# $CARGO; not Cargo.toml) is refused. Quoted prose and data elsewhere are left alone: on 3339eea6, 14 quoted words outside
# echo/printf commands mention cargo (Cargo.toml included), none runs, none is in a wrapper command.
SHELL_WRAPPERS = {"sh", "bash", "zsh", "dash", "ksh", "eval", "pwsh", "powershell", "cmd"}
CARGO_IN_TEXT = re.compile(r"(?i)(?<![A-Za-z0-9_.-])cargo(?:\.exe|-nextest)?(?![A-Za-z0-9_.-])")
# uses: values in the workflows today (census 2026-10-17, version ignored). Add only an action that cannot run tests
# (checkout, setup, cache, upload); tauri-apps/tauri-action builds the app and runs no test. Anything else - another
# action, a local ./ action, a reusable workflow - is refused.
ALLOWED_ACTIONS = {"actions/cache", "actions/checkout", "actions/setup-python", "actions/upload-artifact",
                   "dtolnay/rust-toolchain", "oven-sh/setup-bun", "swatinem/rust-cache", "tauri-apps/tauri-action"}

def basename_word(token):
    return re.split(r"[\\/]", token)[-1].lower().removesuffix(".exe")

def is_cargo_word(token):
    """cargo, cargo.exe, a path ending in either, or $CARGO / ${CARGO}."""
    return re.fullmatch(r"\$?\{?cargo(\.exe)?\}?", re.split(r"[\\/]", token)[-1].lower()) is not None

def native_test_functions(source):
    """Test function names, also when other attribute lines sit between #[test] and fn (R-C8CI-1 issue 3)."""
    return re.findall(r"(?m)^[ \t]*#\[(?:tokio::)?test(?:\([^)\n]*\))?\]\s*(?:#\[[^\n]*\]\s*)*"
                      r"(?:async\s+)?fn\s+([A-Za-z0-9_]+)\s*\(", source)

def workflow_step_runs(path):
    """(owning step name, run body) for every run: key; the step is found by indentation, block scalars skipped."""
    lines = path.read_text(encoding="utf-8").splitlines()
    indent = lambda text: len(text) - len(text.lstrip(" "))
    unquote = lambda v: v[1:-1].replace("''", "'") if len(v) > 1 and v[0] == v[-1] and v[0] in "'\"" else v
    items, result, i = [], [], 0          # items: [dash column, key column, step name] of open sequence entries
    while i < len(lines):
        line = lines[i]
        i += 1
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        match = re.match(r"^( *)(- +)?([A-Za-z0-9_-]+):(?:\s+(.*?))?\s*$", line)
        if not match:
            continue
        column = indent(line)
        items = [item for item in items if item[0] < column]
        key_column = column + len(match.group(2) or "")
        if match.group(2):
            items.append([column, key_column, None])
        key, value = match.group(3), (match.group(4) or "")
        owner = next((item for item in reversed(items) if item[1] == key_column), None)
        if re.fullmatch(r"[|>][0-9+-]*(\s+#.*)?", value):
            body = []
            while i < len(lines) and (not lines[i].strip() or indent(lines[i]) > key_column):
                body.append(lines[i].strip())
                i += 1
            value = (" " if value.startswith(">") else "\n").join(body)
        else:
            value = unquote(value)
        if key == "name" and owner is not None:
            owner[2] = value
        elif key == "run":
            result.append((owner[2] if owner else None, value))
    return result

def logical_lines(body):
    """Shell lines with backslash-newline continuations joined."""
    result, pending = [], ""
    for line in body.splitlines():
        stripped = line.rstrip()
        if (len(stripped) - len(stripped.rstrip("\\"))) % 2:
            pending += stripped[:-1] + " "
            continue
        result.append(pending + line)
        pending = ""
    if pending:
        result.append(pending)
    return result

def simple_commands(line):
    """Token lists of the simple commands in one shell line: split on ; & | ( ), redirections removed."""
    lexer = shlex.shlex(line, posix=True, punctuation_chars=True)
    lexer.whitespace_split = True
    lexer.commenters = "#"
    commands, current, tokens = [], [], list(lexer)
    index = 0
    while index < len(tokens):
        token = tokens[index]
        index += 1
        if token and set(token) <= set(";&|()"):
            commands.append(current)
            current = []
        elif token and set(token) <= set("<>&|") and ("<" in token or ">" in token):
            if current and current[-1].isdigit():
                current.pop()                 # the file descriptor of '2>&1'
            index += 1                        # the redirection target
        else:
            current.append(token)
    commands.append(current)
    return [command for command in commands if command]

def cargo_test_args(tokens):
    """Arguments after 'cargo [+toolchain] test|t' in one simple command, else None ('nextest run' -> ['--nextest'])."""
    while tokens and (tokens[0] in SHELL_PREFIX_WORDS or re.match(r"^[A-Za-z_][A-Za-z0-9_]*=", tokens[0])):
        tokens = tokens[1:]
    cargo = next((i for i, token in enumerate(tokens) if is_cargo_word(token)), None)
    if not tokens or tokens[0] in ("echo", "printf", ":") or cargo is None:
        return None
    start = cargo + 1
    if start < len(tokens) and tokens[start].startswith("+"):
        start += 1
    if tokens[start:start + 2] == ["nextest", "run"]:
        return ["--nextest"] + tokens[start + 2:]
    if start >= len(tokens) or tokens[start] not in ("test", "t"):
        return None
    return tokens[start + 1:]

def selects_cysd(args):
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
        if flag in selectors[:-1] and selectors[selectors.index(flag) + 1] == "cys-app":
            return False
    if any(flag in selectors for flag in ("--bins", "--all-targets", "--tests")):
        return True
    bins = [selectors[i + 1] for i, token in enumerate(selectors[:-1]) if token == "--bin"]
    if bins:
        return "cysd" in bins or any("$" in name for name in bins)
    if any(flag in selectors for flag in ("--lib", "--test", "--example", "--doc")):
        return False
    return True

def line_commands(line):
    try:
        return simple_commands(line)
    except ValueError:                        # unbalanced quoting: read words anyway rather than skip the line
        text = re.sub(r"(^|\s)#.*$", "", line.replace("'", " ").replace('"', " "))
        return [part.split() for part in re.split(r"[;&|()]", text)]

def cysd_test_runs(body):
    """(logical line, cargo args) for every simple command in a run body that executes cysd tests."""
    runs = []
    for line in logical_lines(body):
        for tokens in line_commands(line):
            args = cargo_test_args(tokens)
            if args is not None and selects_cysd(args):
                runs.append((line.strip(), args))
    return runs

# Limits of unread_cargo_routes (R-C8MIN-1 issues 1-2), printed with its assertion; the ci-branch registry text for
# this test carries the same sentences.
UNREAD_ROUTE_LIMITS = ("Not covered: cargo started by a script, heredoc or other program a run line calls; audit those "
                       "by hand. Also unread: a cmd.exe ^ continuation (its first line reads as a test-name filter). "
                       "Refused, not read: cargo inside an sh, bash, zsh, dash, ksh, eval, pwsh, powershell or cmd "
                       "command, the cargo-nextest binary, and any action or reusable workflow not on ALLOWED_ACTIONS.")

def unread_cargo_routes(root):
    """Routes the inventory cannot read, refused so that a new one is audited (R-C8INV-1 issue 2).
    What it cannot see: UNREAD_ROUTE_LIMITS."""
    workflows = root / ".github/workflows"
    problems = ["workflow file not named *.yml: " + p.name for p in sorted(workflows.iterdir()) if p.is_file() and p.suffix != ".yml"]
    if (root / ".github/actions").exists():
        problems.append(".github/actions exists: composite action steps are not read")
    for path in sorted(workflows.glob("*.yml")):
        for line in path.read_text(encoding="utf-8").splitlines():
            used = re.match(r"^\s*(?:-\s+)?uses:\s*['\"]?([^\s'\"]+)", line)
            if used and "cargo" in used.group(1).lower():
                problems.append("%s uses a cargo action: %s" % (path.name, used.group(1)))
            elif used and used.group(1).split("@")[0].lower() not in ALLOWED_ACTIONS:
                problems.append("%s uses an action or reusable workflow not on the allow-list: %s" % (path.name, used.group(1)))
        for step, body in workflow_step_runs(path):
            for line in logical_lines(body):
                for tokens in line_commands(line):
                    if not tokens or tokens[0] in ("echo", "printf", ":"):
                        continue
                    wrapper = next((i for i, token in enumerate(tokens) if basename_word(token) in SHELL_WRAPPERS), None)
                    if wrapper is not None and any(CARGO_IN_TEXT.search(token) for token in tokens[wrapper + 1:]):
                        problems.append("%s step %r: cargo inside a wrapper command (%s) is not read: %s"
                                        % (path.name, step, tokens[wrapper], line.strip()))
                        continue
                    if any(basename_word(token) == "cargo-nextest" for token in tokens):
                        problems.append("%s step %r: the cargo-nextest binary is not read: %s" % (path.name, step, line.strip()))
                        continue
                    cargo = next((i for i, token in enumerate(tokens) if is_cargo_word(token)), None)
                    if cargo is None:
                        continue
                    rest = tokens[cargo + 1:]
                    rest = rest[1:] if rest[:1] and rest[0].startswith("+") else rest
                    if rest and rest[0] not in KNOWN_CARGO_SUBCOMMANDS:
                        problems.append("%s step %r: cargo subcommand %r is not classifiable: %s"
                                        % (path.name, step, rest[0], line.strip()))
    return problems

def invokes_cysd_tests(line):
    # Current workflows use direct cargo commands; this also recognizes an implicit
    # all-target/default-package test route rather than demanding '--bin cysd'.
    return bool(cysd_test_runs(line))

def native_selection(args, names):
    """(True, why) when a cysd test run can select a C8 native test, else (False, why). Unknown syntax can select."""
    if args[:1] == ["--nextest"]:
        return True, "cargo nextest filters are not libtest filters"
    split = args.index("--") if "--" in args else len(args)
    filters, exact = [], False
    for part, values, switches in ((args[:split], CARGO_VALUE_FLAGS, CARGO_SWITCHES),
                                   (args[split + 1:], LIBTEST_VALUE_FLAGS, LIBTEST_SWITCHES)):
        tokens = iter(part)
        for token in tokens:
            if not token.startswith("-") or token == "-":
                filters.append(token)
                continue
            flag = token.split("=", 1)[0]
            if flag in values:
                if "=" not in token and next(tokens, None) is None:
                    return True, "flag %s has no value" % flag
            elif flag in switches and "=" not in token:
                exact = exact or (switches is LIBTEST_SWITCHES and flag == "--exact")
            else:
                return True, "unclassified flag %s" % token
    if not filters:
        return True, "no test-name filter: the whole suite runs"
    for name_filter in filters:
        if re.search(r"[$`*?\[{~\\]", name_filter):
            return True, "filter %r is not a literal" % name_filter
        hits = [name for name in names if (name_filter == name if exact else name_filter in name)]
        if hits:
            return True, "filter %r selects %s" % (name_filter, hits[0])
    return False, "%s filter(s) %s select none of the %d C8 native tests" % ("exact" if exact else "substring",
                                                                             filters, len(names))


# Exact lines of the real windows-health job log on 345b996d (2026-10-19, evidence windows-health-job.log; its line numbers),
# GitHub's timestamp prefix removed. The runner showed `test X ... ` and the panic header as two lines 36 microseconds
# apart; the step's tee writes the raw stream, so the INLINE part joins two real lines.
WIN_LOG_STEP9 = [
    # 2298-2306: a helper thread's expected panic inside a passing test
    'test accounts::tests::b3_units::b3_account_in_use_table ... ok',
    'test accounts::tests::b3_units::b3_alert_eligible_table ... ok',
    'test accounts::tests::b3_units::b3_alert_rows_carry_age_in_use_reset_and_held ... ok',
    'test accounts::tests::b3_units::b3_fired_alert_detail_carries_observation_meta ... ok',
    'test accounts::tests::b3_units::b3_local_json_rows_carry_in_use_rate_observed_at_and_alert_eligible ... ',
    "thread '<unnamed>' (8324) panicked at src\\bin\\cysd\\accounts.rs:4126:17:",
    'surfaces 락 오염(검체)',
    'note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace',
    'ok',
    # 2389-2395: a failing test, header on its own line (as the runner showed it)
    'test activity_tests::exited_seat_serializes_unknown ... ',
    "thread 'activity_tests::exited_seat_serializes_unknown' (12296) panicked at src\\bin\\cysd\\activity_tests.rs:658:5:",
    'assertion `left == right` failed: precondition: {"consecutive_rejections":0,"display_epoch":"81171d9d1779e730","family":"claude","grammar_revision":null,"last_positive_age_secs":null,"observation_seq":4,"observed_age_secs":0.0,"raw":"unknown","reason":"odd output generation","retained":false,"stable_age_secs":3.0,"state":"unknown"}',
    '  left: String("unknown")',
    ' right: "working"',
    'FAILED',
    'test activity_tests::families_do_not_cross_match ... ok',
    # 2397-2401: an ignored-with-reason line, then an expected panic in a passing test
    'test activity_tests::hold_copy_micro_benchmark ... ignored, micro-benchmark; run with --release and CYS_HOLD_BENCH_OUT',
    'test activity_tests::identity_guard_overlapping_writers_and_unwinding_writer ... ',
    "thread 'activity_tests::identity_guard_overlapping_writers_and_unwinding_writer' (12952) panicked at src\\bin\\cysd\\activity_tests.rs:635:9:",
    'writer unwinds mid-mutation',
    'ok',
    # 2406+2407 joined, then 2408-2411: the INLINE form, header on the `test X ... ` line
    "test activity_tests::live_quiescing_window_is_reset_pending ... thread 'activity_tests::live_quiescing_window_is_reset_pending' (13464) panicked at src\\bin\\cysd\\activity_tests.rs:658:5:",
    'assertion `left == right` failed: precondition: {"consecutive_rejections":0,"display_epoch":"a230a40c851eebb3","family":"claude","grammar_revision":"synthetic-test-1","last_positive_age_secs":null,"observation_seq":4,"observed_age_secs":0.0,"raw":"unknown","reason":"odd output generation","retained":false,"stable_age_secs":3.0,"state":"unknown"}',
    '  left: String("unknown")',
    ' right: "working"',
    'FAILED',
    # 5309-5313, 5375-5376: libtest's names list (two of 63) and its real summary
    'failures:',
    '',
    'failures:',
    '    activity_tests::exited_seat_serializes_unknown',
    '    activity_tests::live_quiescing_window_is_reset_pending',
    '',
    'test result: FAILED. 2428 passed; 63 failed; 59 ignored; 0 measured; 2 filtered out; finished in 1044.27s',
]
# 7107-7123: captured-output blocks (a later cargo invocation in the same job, without --nocapture)
WIN_LOG_BLOCKS = [
    'failures:',
    '',
    '---- activity_tests::collect_copies_bounded_rows_without_growing_its_buffers stdout ----',
    '',
    "thread 'activity_tests::collect_copies_bounded_rows_without_growing_its_buffers' (16616) panicked at src\\bin\\cysd\\activity_tests.rs:404:5:",
    'assertion failed: matches!(collect(&s, &mut frame), Collected::Frame)',
    'note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace',
    '',
    '---- activity_tests::epoch_changes_with_identity_and_sequence_is_monotonic_within_it stdout ----',
    '',
    "thread 'activity_tests::epoch_changes_with_identity_and_sequence_is_monotonic_within_it' (27696) panicked at src\\bin\\cysd\\activity_tests.rs:658:5:",
    'assertion `left == right` failed: precondition: {"consecutive_rejections":0,"display_epoch":"c65a509e9d52f58c","family":"claude","grammar_revision":"synthetic-test-1","last_positive_age_secs":null,"observation_seq":4,"observed_age_secs":0.0,"raw":"unknown","reason":"odd output generation","retained":false,"stable_age_secs":3.0,"state":"unknown"}',
    '  left: String("unknown")',
    ' right: "working"',
    '',
    '---- activity_tests::exited_seat_serializes_unknown stdout ----',
    '',
]
# Exact lines 7338-7341 of the real release job log of run 39772179832 attempt 2 (macOS aarch64 job 119817491380), GitHub's
# timestamp prefix removed; attempt 1 (job 119802177291) printed the same four lines at 7334-7337. An uncaptured `[queue]`
# stderr line landed after `test X ... `, so libtest's `ok` for X came alone on the next line.
RELEASE_SPLIT_OUTCOME = [
    'test handlers::tests::wp5_queue_revive_refuses_when_active_queue_full ... ok',
    'test native_publication::native_transition_tests::np_actual_partial_progress_cannot_be_retokenized_to_replacement ... [queue] 영속 중단(expired-first) — queue-expired.json 원자 치환 실패: No such file or directory (os error 2) · 원본 보존(다음 틱 재시도)',
    'ok',
    'test native_publication::native_transition_tests::np_actual_writer_operation_blocks_activation_after_prepare ... ok',
]
# 8404 of the same log: the cysd run's real summary
RELEASE_CYSD_SUMMARY = 'test result: ok. 2811 passed; 0 failed; 13 ignored; 0 measured; 16 filtered out; finished in 806.79s'

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
        names = json.loads((ROOT / "scripts/c8_native_tests.json").read_text(encoding="utf-8"))
        # The selection proof below rests on the only marker emitter living in the native test submodule.
        emitters = sorted(str(p.relative_to(ROOT)) for p in (ROOT / "src").rglob("*.rs")
                          if "C8_NATIVE_UNMEASURED" in p.read_text(encoding="utf-8", errors="replace"))
        self.assertEqual(emitters, ["src/bin/cysd/native_publication_transition_tests.rs"],
                         "a C8 marker emitter outside the native test submodule escapes the native-name proof")
        self.assertEqual(unread_cargo_routes(ROOT), [],
                         "a cargo route this inventory cannot read: audit it. " + UNREAD_ROUTE_LIMITS)
        gated = {}
        for path in sorted((ROOT / ".github/workflows").glob("*.yml")):
            for step, body in workflow_step_runs(path):
                for line, args in cysd_test_runs(body):
                    if (path.name, step) in GATED_CYSD_LEGS:
                        gated.setdefault((path.name, step), []).append(line)
                        continue
                    can_select, why = native_selection(args, names)
                    self.assertFalse(can_select, "%s step %r runs cysd tests that can emit the C8 marker but no blocking "
                                     "marker gate reads its log (%s): %s" % (path.name, step, why, line))
        self.assertEqual({leg: len(lines) for leg, lines in gated.items()}, dict.fromkeys(GATED_CYSD_LEGS, 1),
                         "gated cysd leg inventory changed: audit and wire every actual leg")
        gate_logs = {workflow: logs for workflow, _, logs in MARKER_STEPS}
        for (workflow, step), log in GATED_CYSD_LEGS.items():
            self.assertIn(log, gate_logs[workflow], "gated leg log is not read by its workflow's marker gate")
            body = dict(workflow_step_runs(ROOT / ".github/workflows" / workflow))[step]
            for part in log.split("/"):
                self.assertIn(part, body, "gated leg %r no longer writes the log its gate reads" % step)
        for command in ["cargo test", "cargo test --workspace", "cargo test --all-targets", "cargo +stable test --bins",
                        "echo start; cargo test --bin cysd", "(cd x && cargo test --bin cysd)", "cargo t --bin cysd",
                        "if cargo test --bin cysd; then :; fi", "X=1 cargo test --bin=cysd 2>&1 | tee log",
                        "cargo nextest run --bin cysd", "cargo test --bin cysd 'unbalanced"]:
            self.assertTrue(invokes_cysd_tests(command), command)
        for command in ["# cargo test", "echo cargo test", "cargo test --lib", "cargo test --bin cys", "cargo test -p cys-app --bins",
                        "cargo test --bin cysd -- --list", "printf '%s' 'cargo test --bin cysd'", "n=$(cargo test --bin cysd -- --list)"]:
            self.assertFalse(invokes_cysd_tests(command), command)

    def test_native_selection_and_run_extraction_forms(self):
        names = json.loads((ROOT / "scripts/c8_native_tests.json").read_text(encoding="utf-8"))
        args = lambda command: cysd_test_runs(command)[0][1]
        for command in ["cargo test --bin cysd u229_ -- --test-threads=1",
                        "cargo test --bin cysd hwmon:: -- --test-threads=1 --nocapture --skip hwmon::tests::snapshot_has_all_sections",
                        "cargo test --offline --bin cysd -- --test-threads 1 root_registration::",
                        "cargo test --bin cysd u229_ -- --test-threads=1 2>&1 | tee \"$LOG\"",
                        "cargo test --bin cysd -- --exact native_publication"]:
            self.assertFalse(native_selection(args(command), names)[0], command)
        for command in ["cargo test --bin cysd -- --test-threads=1 --skip hwmon::", "cargo test --bin cysd -- --test-threads=1 2>&1",
                        "cargo test --bin cysd native_transition", "cargo test --bin cysd -- np_", "cargo test --bin cysd \"$FILTER\"",
                        "cargo test --bin cysd --frobnicate hwmon::", "cargo test --bin cysd -- --exact " + names[0],
                        "cargo test --bin cysd -- --skip", "cargo nextest run --bin cysd hwmon::"]:
            self.assertTrue(native_selection(args(command), names)[0], command)
        workflow = self.root / "forms.yml"
        workflow.write_text("jobs:\n  a:\n    steps:\n      - name: continued\n        run: |\n          cargo test \\\n"
                            "            --bin cysd\n      - if: always()\n        name: 'quoted '' name'\n"
                            "        run: \"cargo test --bin cysd\"\n      - name: stripped\n        run: |-\n"
                            "          echo x; cargo test --bin cysd u229_\n", encoding="utf-8")
        found = [(step, args) for step, body in workflow_step_runs(workflow) for _, args in cysd_test_runs(body)]
        self.assertEqual(found, [("continued", ["--bin", "cysd"]), ("quoted ' name", ["--bin", "cysd"]),
                                 ("stripped", ["--bin", "cysd", "u229_"])])
        for command in ["C:/rust/bin/cargo.exe test --bin cysd", "\"$CARGO\" test --bin cysd", "${CARGO} t --bin cysd"]:
            self.assertTrue(invokes_cysd_tests(command), command)

    def test_unread_cargo_routes_are_refused(self):
        base = self.root / "routes"
        flows = base / ".github/workflows"
        flows.mkdir(parents=True)
        self.assertEqual(unread_cargo_routes(base), [])
        step = "jobs:\n  a:\n    steps:\n      - name: s\n        run: %s\n"
        cases = [("a.yaml", "x: 1\n", "not named *.yml"),
                 ("b.yml", "jobs:\n  a:\n    steps:\n      - uses: actions-rs/cargo@v1\n", "uses a cargo action"),
                 ("c.yml", step % "cargo ${{ matrix.sub }} --bin cysd", "not classifiable"),
                 ("d.yml", step % "cargo \"$T\" --bin cysd", "not classifiable"),
                 ("e.yml", step % "sh -c 'cargo test --bin cysd'", "wrapper command"),
                 ("f.yml", step % "bash -lc \"cd x && cargo test --bin cysd\"", "wrapper command"),
                 ("g.yml", step % "eval \"$CARGO test --bin cysd\"", "wrapper command"),
                 ("h.yml", step % "cargo-nextest nextest run --bin cysd", "cargo-nextest binary"),
                 ("i.yml", "jobs:\n  a:\n    steps:\n      - uses: example/run-tests@v1\n", "not on the allow-list"),
                 ("j.yml", "jobs:\n  a:\n    uses: ./.github/workflows/other.yml\n", "not on the allow-list"),
                 ("k.yml", step % "C:/Windows/System32/cmd.exe /c \"cargo test --bin cysd\"", "wrapper command")]
        (flows / "ok.yml").write_text("jobs:\n  a:\n    steps:\n      - uses: actions/checkout@v4\n"
                                      "      - name: s\n        run: bash scripts/x.sh && sh -c 'grep version Cargo.toml'\n",
                                      encoding="utf-8")
        self.assertEqual(unread_cargo_routes(base), [], "a wrapper or an allowed action without cargo was refused")
        (flows / "ok.yml").unlink()
        for name, text, needle in cases:
            with self.subTest(route=name):
                (flows / name).write_text(text, encoding="utf-8")
                problems = unread_cargo_routes(base)
                self.assertTrue(any(needle in p for p in problems), "unread route accepted: %s %s" % (name, problems))
                (flows / name).unlink()
        (base / ".github/actions/x").mkdir(parents=True)
        self.assertTrue(any("composite action" in p for p in unread_cargo_routes(base)), "composite action route accepted")

    def test_windows_producer_steps_write_the_logs_their_gate_reads(self):
        # Behavioural link for the two Windows gated legs (R-C8INV-1 issue 3), as ci-branch and release have.
        cargo = self.bin / "cargo"
        cargo.write_text("#!/bin/sh\ncase \"$*\" in *--list*) echo 'hwmon::tests::fake: test'; exit 0;; esac\n"
                         "printf '%s\\n' \"$FAKE_LOG\"\nexit 0\n", encoding="utf-8")
        cargo.chmod(0o755)
        bodies = dict(workflow_step_runs(ROOT / ".github/workflows/windows-health.yml"))
        legs = [(step, log) for (workflow, step), log in GATED_CYSD_LEGS.items() if workflow == "windows-health.yml"]
        for text, gate_rc in [("test result: ok. 1 passed; 0 failed; 0 ignored;", 0), (marker().strip(), 3)]:
            for step, log in legs:
                with self.subTest(step=step, gate_rc=gate_rc):
                    script = self.root / "producer.sh"
                    script.write_text(bodies[step], encoding="utf-8")
                    env = dict(self.env, FAKE_LOG=text)
                    ran = subprocess.run([BASH, "--noprofile", "--norc", str(script)], cwd=self.root, env=env,
                                         capture_output=True, timeout=60)
                    self.assertEqual(ran.returncode, 0, ran.stdout + ran.stderr)
                    self.assertTrue((self.rt / log).is_file(), "producer step no longer writes the log its gate reads: " + log)
                    self.assertEqual((self.rt / log).read_text(encoding="utf-8"), text + "\n")
            gate = next(name for workflow, name, _ in MARKER_STEPS if workflow == "windows-health.yml")
            self.assertEqual(self.run_body("windows-health.yml", gate).returncode, gate_rc,
                             "Windows gate verdict on the logs its producers actually wrote")

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
        found = native_test_functions(source)
        self.assertEqual(len(re.findall(r"#\[(?:tokio::)?test\b", source)), len(self.expected()),
                         "test attribute count differs from the manifest: a test the name pattern cannot see")
        self.assertEqual(native_test_functions("#[test]\n#[cfg(unix)]\nfn np_a() {}\n#[tokio::test] async fn np_b() {}\n"),
                         ["np_a", "np_b"], "an attribute line or tokio::test hid a native test from the manifest check")
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

    def rows(self, names):
        return ["test " + name + " ... ok" for name in names]

    def name_gate(self, lines):
        """Both actual name steps over the same log: they must agree; returns (rc, the JSON they print)."""
        seen = []
        for workflow, step, log in NAME_STEPS:
            self.log(log, "\n".join(lines) + "\n")
            ran = self.run_body(workflow, step)
            seen.append((ran.returncode, json.loads(ran.stdout)))
        self.assertEqual(seen[0], seen[1], "branch and release name gates read the same log differently")
        return seen[0]

    def test_real_release_split_outcome_is_read_from_the_following_line(self):
        names = self.expected()
        self.assertEqual(RELEASE_SPLIT_OUTCOME[1].split()[1], names[0])
        self.assertEqual(RELEASE_SPLIT_OUTCOME[3].split()[1], names[1])
        before, split, outcome, after = RELEASE_SPLIT_OUTCOME
        stray = split.split(" ... ", 1)[1]
        for label, lines in [("real lines", RELEASE_SPLIT_OUTCOME + self.rows(names[2:])),
                             ("coloured outcome", [before, split, "\x1b[32mok\x1b[0m", after] + self.rows(names[2:])),
                             ("more stderr lines first", [before, split, stray, stray, outcome, after] + self.rows(names[2:]))]:
            with self.subTest(case=label):
                rc, doc = self.name_gate(lines)
                self.assertEqual((rc, doc["native_group_state"], doc["observed_rows"], doc["missing"], doc["nonpassing"]),
                                 (0, "COMPLETE", 15, [], []))

    def test_split_outcome_fails_closed(self):
        names = self.expected()
        before, split, outcome, after = RELEASE_SPLIT_OUTCOME
        rest, others = self.rows(names[2:]), self.rows(names[1:])
        missing = {"missing": [names[0]], "nonpassing": [], "observed_rows": 14}
        stray = split.split(" ... ", 1)[1]
        for label, lines, want in [
                ("another test starts before the outcome", [before, split, after, outcome] + rest, missing),
                ("the summary comes before the outcome", others + [split, RELEASE_CYSD_SUMMARY, outcome], missing),
                ("stray text starting with 'test ' comes before the outcome", [before, split, "test " + stray, outcome, after]
                 + rest, missing),
                ("no outcome line before EOF", others + [split], missing),
                ("two outcome lines", [before, split, "ok", "FAILED", after] + rest, missing),
                ("outcome only glued to other text", [before, split + "ok", after] + rest, missing),
                ("outcome glued to a later stderr line", [before, split, "[queue] 재시도 ok", after] + rest, missing),
                ("split FAILED", [before, split, "FAILED", after] + rest,
                 {"missing": [], "nonpassing": [{"test": names[0], "status": "FAILED"}], "observed_rows": 15}),
                ("split ignored", [before, split, "ignored", after] + rest,
                 {"missing": [], "nonpassing": [{"test": names[0], "status": "ignored"}], "observed_rows": 15}),
                ("split ignored with a reason", [before, split, "ignored, slow", after] + rest,
                 {"missing": [], "nonpassing": [{"test": names[0], "status": "ignored"}], "observed_rows": 15}),
                ("a split row duplicates a same-line row", RELEASE_SPLIT_OUTCOME + rest + ["test " + names[0] + " ... ok"],
                 {"missing": [], "duplicates": [names[0]], "observed_rows": 16}),
                ("a split row of a native name outside the manifest", RELEASE_SPLIT_OUTCOME + rest
                 + [split.replace(names[0], names[0] + "_extra"), outcome],
                 {"missing": [], "unexpected": [names[0] + "_extra"], "observed_rows": 16})]:
            with self.subTest(case=label):
                rc, doc = self.name_gate(lines)
                self.assertEqual((rc, doc["native_group_state"]), (4, "INCOMPLETE"))
                self.assertEqual({key: doc[key] for key in want}, want)

    def test_same_line_outcomes_never_read_the_following_line(self):
        names = self.expected()
        first = "test " + names[0] + " ... "
        for label, lines, status in [("ok, then a lone FAILED", [first + "ok", "FAILED"], "ok"),
                                     ("FAILED, then a lone ok", [first + "FAILED", "ok"], "FAILED"),
                                     ("ignored with a reason", [first + "ignored, slow"], "ignored"),
                                     ("CRLF", [first + "ok\r"], "ok")]:
            with self.subTest(case=label):
                rc, doc = self.name_gate(lines + self.rows(names[1:]))
                self.assertEqual((doc["observed_rows"], doc["missing"]), (15, []))
                self.assertEqual(doc["nonpassing"], [] if status == "ok" else [{"test": names[0], "status": status}])
                self.assertEqual(rc, 0 if status == "ok" else 4)

    def test_unmeasured_marker_split_is_ok_here_and_still_blocks_the_marker_gate(self):
        # The fixture writes its marker as "\nC8_NATIVE_UNMEASURED_V1 {...}\n", so a skipped test has the split shape. The name
        # gate counts libtest's ok (it does not read markers); the separate marker gate on the same log keeps the leg red.
        names = self.expected()
        record = json.dumps({"state": "UNMEASURED", "group": "C8_NATIVE_TRANSITIONS", "test": names[0],
                             "reason": "python3_unavailable"}, separators=(",", ":"))
        written = "test " + names[0] + " ... " + "\nC8_NATIVE_UNMEASURED_V1 " + record + "\n" + "ok"
        rc, doc = self.name_gate(written.split("\n") + self.rows(names[1:]))
        self.assertEqual((rc, doc["native_group_state"], doc["observed_rows"]), (0, "COMPLETE", 15))
        name_logs = [log for _, _, log in NAME_STEPS]
        marker_steps = [(workflow, step) for workflow, step, logs in MARKER_STEPS if logs[0] in name_logs]
        self.assertEqual(len(marker_steps), len(NAME_STEPS), "every name-gate log has a marker gate")
        for workflow, step in marker_steps:
            with self.subTest(workflow=workflow):
                self.assertEqual(self.run_body(workflow, step).returncode, 3, "marker gate must still block this log")

class CysdWinFailureListTests(ReaderFixture):
    """windows-health step 9 writes win-cysd-failures.json from cargo.log; run its actual body over real log lines."""
    STEP = "cargo test --bin cysd 감독자 축 (Windows 실기 · IG-31 · 비차단 A10 1단계)"
    EXITED = "activity_tests::exited_seat_serializes_unknown"
    INLINE = "activity_tests::live_quiescing_window_is_reset_pending"

    def produce(self, text, rc):
        (self.root / "fake.log").write_text(text, encoding="utf-8")
        cargo = self.bin / "cargo"
        cargo.write_text("#!/bin/sh\ncat \"$FAKE_LOG_FILE\"\nexit %d\n" % rc, encoding="utf-8")
        cargo.chmod(0o755)
        script = self.root / "producer.sh"
        # extract_step keeps the block's relative indentation (YAML `|` semantics); workflow_step_runs strips every
        # line, which is fine for shell but breaks the Python heredoc this step writes.
        script.write_text(extract_step(str(ROOT / ".github/workflows/windows-health.yml"), self.STEP)["run"], encoding="utf-8")
        env = dict(self.env, FAKE_LOG_FILE=str(self.root / "fake.log"))
        env.pop("GITHUB_WORKSPACE", None)
        ran = subprocess.run([BASH, "--noprofile", "--norc", str(script)], cwd=self.root, env=env, capture_output=True, timeout=60)
        self.assertEqual(ran.returncode, rc, "the failure list must not change cargo's exit code: %r" % ran.stderr[-400:])
        self.assertEqual((self.rt / "cysd-win/cargo.log").read_text(encoding="utf-8"), text, "the raw log must stay byte-identical")
        return json.loads((self.root / "win-cysd-failures.json").read_text(encoding="utf-8"))

    def test_real_windows_panic_headers_with_thread_ids_are_read(self):
        doc = self.produce("\n".join(WIN_LOG_STEP9) + "\n", 101)
        self.assertEqual((doc["panic_headers_seen"], doc["panic_headers_parsed"], doc["panic_headers_unparsed"]), (4, 4, []),
                         "a real header form is not read")
        by = {p["thread"]: p for p in doc["panics"]}
        exited, inline, helper = by[self.EXITED], by[self.INLINE], by["<unnamed>"]
        self.assertEqual((exited["thread_id"], exited["location"]), ("12296", "src\\bin\\cysd\\activity_tests.rs:658:5"))
        self.assertTrue(exited["message"].startswith("assertion `left == right` failed: precondition: {"))
        self.assertTrue(exited["message"].endswith('  left: String("unknown")\n right: "working"'), exited["message"][-80:])
        self.assertEqual((inline["thread_id"], inline["location"]), ("13464", "src\\bin\\cysd\\activity_tests.rs:658:5"))
        self.assertTrue(inline["message"].endswith(' right: "working"'), inline["message"][-80:])
        self.assertEqual((helper["thread_id"], helper["location"], helper["message"]),
                         ("8324", "src\\bin\\cysd\\accounts.rs:4126:17", "surfaces 락 오염(검체)"))
        self.assertEqual(by["activity_tests::identity_guard_overlapping_writers_and_unwinding_writer"]["message"],
                         "writer unwinds mid-mutation")
        self.assertEqual(doc["failed_tests"], [self.EXITED, self.INLINE])
        self.assertEqual((doc["failed_tests_without_panic"], doc["panic_detail_complete"]), ([], True))
        # Names and detail are separate questions: this excerpt keeps libtest's real summary (63 failed) and 2 names.
        self.assertEqual((doc["failed_in_summary"], doc["names_complete"]), (63, False))

    def test_real_captured_output_blocks_end_each_panic_message(self):
        doc = self.produce("\n".join(WIN_LOG_BLOCKS) + "\n", 101)
        self.assertEqual((doc["panic_headers_seen"], doc["panic_headers_parsed"]), (2, 2))
        first, second = doc["panics"]
        self.assertEqual((first["thread_id"], first["message"]),
                         ("16616", "assertion failed: matches!(collect(&s, &mut frame), Collected::Frame)"))
        self.assertEqual(second["thread_id"], "27696")
        self.assertTrue(second["message"].endswith('  left: String("unknown")\n right: "working"'), second["message"][-80:])

    def test_an_unread_header_form_is_reported_not_dropped(self):
        line = "thread 'x::y' [id 5] panicked at src\\a.rs:1:2:"
        doc = self.produce("test x::y ... \n" + line + "\nboom\nFAILED\n", 101)
        self.assertEqual((doc["panic_headers_seen"], doc["panic_headers_parsed"], doc["panic_headers_unparsed"]), (1, 0, [line]))
        self.assertEqual((doc["panic_detail_complete"], doc["failed_tests_without_panic"]), (False, ["x::y"]))


if __name__ == "__main__":
    unittest.main()
