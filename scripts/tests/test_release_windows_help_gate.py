"""Pin for the W-GATE step (0.14.52 · CEO ruling 2026-10-22, release condition 2): the packaged Windows cys.exe --help gate.

What it guards: before the Windows release leg uploads anything, release.yml runs the exact sidecar file the NSIS installer packed
(src-tauri/binaries/cys-x86_64-pc-windows-msvc.exe) with --help, queue --help and queue list --help at the default stack, and fails
the leg on any non-zero native exit. windows-build.yml runs the SAME step body before any tag exists (the pre-tag dry run; a
separate build instance, not a replacement). This file pins, from the workflow TEXT only:
  A1 release.yml job `build` has exactly one step with the gate name.
  A2 its `if` is exactly the Windows matrix condition.
  A3 no continue-on-error, on the step or on the job.
  A4 shell: pwsh.
  A5 the body runs the exact file with the three argv sets, resets LASTEXITCODE before each call, reads $LASTEXITCODE, prints it
     in hex, fails on non-zero or missing codes, removes RUST_MIN_STACK, and nothing in the step, job or workflow env sets it.
  A6 build step < gate < every upload step of the job (at least one upload step exists).
  A7 every other job holding an upload step has `build` in needs (so pack-artifacts stays behind the gate).
  A8 windows-build.yml has exactly one gate step, after its build step and before every upload, with no if and no continue-on-error.
  A9 the two run bodies are byte-identical.
  A10 neither body (nor the comment block above it) carries a string that the repo's text-scanning workflow readers act on.
A5 also refuses the plausible ways to neuter the body (an added exit/return/break/throw, a reassigned $code/$failed/$exe,
a forced LASTEXITCODE, a wrapper block). It is a guard against edits, not a proof against deliberate rewriting.

How it reads: step keys and the run body come from extract_step (scripts/tests/test_ci_branch_cysd_step.py, the repo's
YAML-faithful extractor). Step ORDER comes from list_steps below, an indentation reader over jobs.<job>.steps that raises on any
shape it cannot read (controls in StepListerControls). Standard library only (the CI python has no PyYAML).

Change together (the pin turns red on the first three; NOTHING checks the docs line, and a check that compares document text
with code is not allowed here):
  - renaming the step: release.yml, windows-build.yml, GATE below, and the docs/RELEASE.md section 0-C line that names it;
  - the sidecar path: both run bodies, SIDECAR below, and both workflows' VERSIONINFO gates (they read the same file);
  - a statement the pin checks: both run bodies (A9) and the A5 tables (EXPECTED_FLOW, ASSIGNMENTS);
  - renaming either build step named in RELEASE_BUILD_STEP / WB_BUILD_STEP: that constant (A6/A8 go red otherwise).

Mutation runs: RELEASE_YML_UNDER_TEST=<copy> and WINDOWS_BUILD_YML_UNDER_TEST=<copy> point the pin at another file.

    python3 scripts/tests/test_release_windows_help_gate.py
"""
import os
import re
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
if HERE not in sys.path:
    sys.path.insert(0, HERE)

from test_ci_branch_cysd_step import StepShapeError, extract_step  # noqa: E402

WF_DIR = os.path.normpath(os.path.join(HERE, "..", "..", ".github", "workflows"))
RELEASE = os.environ.get("RELEASE_YML_UNDER_TEST") or os.path.join(WF_DIR, "release.yml")
WINDOWS_BUILD = os.environ.get("WINDOWS_BUILD_YML_UNDER_TEST") or os.path.join(WF_DIR, "windows-build.yml")

GATE = "Packaged cys.exe --help gate (default stack · blocking · before upload)"
RELEASE_JOB = "build"
RELEASE_BUILD_STEP = "Windows 직접 빌드 + CRT gate (VCRUNTIME import sweep + 0.14.1 fix markers · 업로드 전)"
WB_JOB = "build"
WB_BUILD_STEP = "Build Windows x64 (Tauri → NSIS setup.exe)"
WINDOWS_IF = "matrix.platform == 'windows-latest'"
SIDECAR = "src-tauri/binaries/cys-x86_64-pc-windows-msvc.exe"

# An upload step: one of these actions, or a run body that writes to a release.
UPLOAD_ACTIONS = ("tauri-apps/tauri-action", "actions/upload-artifact", "actions/upload-release-asset",
                  "softprops/action-gh-release", "ncipollo/release-action", "svenstaro/upload-release-action")
# Not recognised (stated, not covered): scp, cloud CLIs, curl -T to a non-GitHub host.
UPLOAD_RUN = re.compile(r"gh\s+release\s+(upload|create|edit)\b|uploads\.github\.com|gh\s+api\b[^\n]*\breleases\b"
                        r"|api\.github\.com/repos/\S*/releases")

# A10. Strings the repo's workflow readers act on, built so this file does not itself contain them (the pyseal census lists
# every repo file holding either seal needle, and the secret scan refuses a home path).
SEAL_NEEDLES = ("PYTHON" + "DONTWRITEBYTECODE", "ENV_PY_" + "NO_BYTECODE")
# Read over the whole file, comments included: test_release_postprocess_gate test_14/test_23/test_25 (flags), test_29 (index of
# the gatekeeper call), test_pyseal_census (needles), the lane-parity gate's self markers.
WHOLE_FILE_NEEDLES = ("--diagnose-degraded-ok", "--seal2-only", "--gktool-only", "--unsafe-skip-gatekeeper",
                      "--runtime-manifest-only", "release-gate-gatekeeper.sh", "LANE-GATE-SELF-BEGIN",
                      "LANE-GATE-SELF-END", "/" + "Users/") + SEAL_NEEDLES
# Read over non-comment lines or run bodies only: the lane-parity gate and lane-parity-rehearsal (test_ names after a `#` cut,
# so a `#` would also hide the rest of a line), test_c8_ci_consumer (uses:, cargo), run_bootstrap_health U28 (python lines),
# test_runtime_manifest_parity (LOCALAPPDATA recursion, install-dir strings).
BODY_PATTERNS = (
    ("test_ token", re.compile(r"\btest_[a-z0-9_]+")),
    ("cargo", re.compile(r"(?i)cargo")),
    ("uses: key", re.compile(r"(?m)^\s*(-\s+)?uses:")),
    ("python", re.compile(r"\bpython3?\b")),
    ("comment mark #", re.compile(r"#")),
    ("Get-ChildItem", re.compile(r"Get-ChildItem")),
    ("-Recurse", re.compile(r"-Recurse")),
    ("LOCALAPPDATA", re.compile(r"LOCALAPPDATA")),
    ("CYS_INSTALL_", re.compile(r"CYS_INSTALL_")),
)


# A5 structure (R1-1). PowerShell keywords and variable names are case-insensitive. A flow statement counts at statement
# position only (line start or after ; { }), so `native-exit=` in a string is not one. Quoted text is removed before braces
# are counted (the body's strings hold `{0:X8}` and `${label}`).
FLOW = re.compile(r"(?i)(?:^|[;{}])\s*(exit|return|break|continue|throw|trap)\b\s*([^\s;}]*)")
EXPECTED_FLOW = ["exit 1", "continue", "exit 1", "exit 0"]
FORBIDDEN = re.compile(r"(?i)::\s*Exit\s*\(|\b(Stop-Process|Exit-PSSession|Exit-PSHostProcess|Invoke-Expression|iex"
                       r"|Set-Variable|New-Variable|Remove-Variable|Clear-Variable)\b")
# variable -> {assignment operator: expected count}; any other operator on it must not appear.
ASSIGNMENTS = {"code": {"=": 1}, "failed": {"=": 1, "+=": 2}, "exe": {"=": 1}, "full": {"=": 1},
               "LASTEXITCODE": {"=": 1}, "PSNativeCommandUseErrorActionPreference": {"=": 1}, "argv": {}}


def assignments(body, var):
    """{operator: count} of assignments to $var (any scope prefix, ${} braces, case-insensitive)."""
    pat = re.compile(r"(?i)\$\{?(?:global:|script:|local:|private:)?%s\}?\s*([-+*/%%]?=)(?!=)" % re.escape(var))
    out = {}
    for m in pat.finditer(body):
        out[m.group(1)] = out.get(m.group(1), 0) + 1
    return out


def depths(lines):
    """Brace depth at the START of each line, quoted text removed first; raises if braces do not balance."""
    d, out = 0, []
    for line in lines:
        out.append(d)
        bare = re.sub(r"'[^']*'", "", re.sub(r'"[^"]*"', "", line))
        d += bare.count("{") - bare.count("}")
        if d < 0:
            raise ListShapeError("brace depth below 0 at %r" % line)
    if d != 0:
        raise ListShapeError("braces do not balance (end depth %d)" % d)
    return out


class ListShapeError(AssertionError):
    """The step list could not be read - measuring a wrong list silently is worse than stopping."""


def _indent(line):
    return len(line) - len(line.lstrip(" "))


def _content(line):
    s = line.strip()
    return bool(s) and not s.startswith("#")


def _job_region(lines, job):
    """(start, end) line indexes of jobs.<job> (start = the head line)."""
    # A tab-led line is not YAML indentation; read as 'indent 0' it would silently END a region (job or steps) - so the
    # whole file is checked first, not the region it might cut short.
    tabs = [n + 1 for n, l in enumerate(lines) if l.startswith("\t")]
    if tabs:
        raise ListShapeError("tab-indented line(s) %s" % tabs[:5])
    tops = [i for i, l in enumerate(lines) if re.match(r"^jobs:\s*(#.*)?$", l)]
    if len(tops) != 1:
        raise ListShapeError("`jobs:` appears %d times at column 0 (1 expected)" % len(tops))
    heads = [i for i in range(tops[0] + 1, len(lines)) if re.match(r"^  %s:\s*(#.*)?$" % re.escape(job), lines[i])]
    if len(heads) != 1:
        raise ListShapeError("job `%s` appears %d times (1 expected)" % (job, len(heads)))
    end = heads[0] + 1
    while end < len(lines) and not (_content(lines[end]) and _indent(lines[end]) <= 2):
        end += 1
    return heads[0], end


def job_keys(text, job):
    """{key: raw value or child lines} for the indent-4 keys of jobs.<job> (needs, env, continue-on-error, ...)."""
    lines = text.split("\n")
    start, end = _job_region(lines, job)
    keys, i = {}, start + 1
    while i < end:
        line = lines[i]
        if not _content(line):
            i += 1
            continue
        if line.startswith("\t"):
            raise ListShapeError("line %d: tab indentation" % (i + 1))
        m = re.match(r"^    ([A-Za-z0-9_-]+):(?:\s+(.*))?$", line)
        if _indent(line) != 4 or not m:
            raise ListShapeError("line %d: not a job key at indent 4: %r" % (i + 1, line))
        k, v = m.group(1), (m.group(2) or "").strip()
        if k in keys:
            raise ListShapeError("line %d: job key `%s` twice" % (i + 1, k))
        i += 1
        children = []
        while i < end and (not lines[i].strip() or _indent(lines[i]) > 4):
            children.append(lines[i])
            i += 1
        keys[k] = v if v else children
    return keys


def job_needs(text, job):
    """The job's needs as a list: `needs: x`, `needs: [x, y]` or a block list; anything else raises."""
    v = job_keys(text, job).get("needs")
    if v is None:
        return []
    if isinstance(v, str):
        v = re.sub(r"\s+#.*$", "", v).strip()
        m = re.fullmatch(r"\[(.*)\]", v)
        items = [x.strip() for x in m.group(1).split(",")] if m else [v]
        if not all(re.fullmatch(r"[A-Za-z0-9_-]+", x) for x in items):
            raise ListShapeError("job `%s`: needs value not readable: %r" % (job, v))
        return items
    items = []
    for line in v:
        if not _content(line):
            continue
        m = re.fullmatch(r"\s*-\s+([A-Za-z0-9_-]+)\s*(#.*)?", line)
        if not m:
            raise ListShapeError("job `%s`: needs list line not readable: %r" % (job, line))
        items.append(m.group(1))
    return items


def env_lines(text, job):
    """Non-comment lines of the workflow-level env block and of jobs.<job>.env."""
    lines = text.split("\n")
    out = []
    tops = [i for i, l in enumerate(lines) if re.match(r"^env:\s*(#.*)?$", l)]
    for t in tops:
        i = t + 1
        while i < len(lines) and (not lines[i].strip() or _indent(lines[i]) > 0 or lines[i].lstrip().startswith("#")):
            if _content(lines[i]):
                out.append(lines[i])
            i += 1
    env = job_keys(text, job).get("env")
    if isinstance(env, str):
        out.append(env)
    elif env:
        out.extend(l for l in env if _content(l))
    return out


def list_steps(text, job):
    """[{'name', 'uses', 'if', 'keys', 'run', 'line', 'comment'}] for jobs.<job>.steps in file order.

    Reads by indentation: items are `- ` lines at the indent of the first item, keys sit two columns deeper, anything deeper
    belongs to the key above it. Raises ListShapeError on any line it cannot place.
    """
    lines = text.split("\n")
    start, end = _job_region(lines, job)
    st = [i for i in range(start + 1, end) if re.match(r"^    steps:\s*(#.*)?$", lines[i])]
    if len(st) != 1:
        raise ListShapeError("job `%s`: `steps:` appears %d times at indent 4 (1 expected)" % (job, len(st)))
    i, steps, dash = st[0] + 1, [], None
    stop = i
    while stop < end and not (_content(lines[stop]) and _indent(lines[stop]) <= 4):
        stop += 1
    while i < stop:
        line = lines[i]
        if line.startswith("\t"):
            raise ListShapeError("line %d: tab indentation" % (i + 1))
        if not _content(line):
            i += 1
            continue
        if dash is None:
            dash = _indent(line)
        if _indent(line) != dash or not re.match(r"^\s*-(\s|$)", line):
            raise ListShapeError("line %d: expected a step item at indent %d: %r" % (i + 1, dash, line))
        comment, k = [], i - 1
        while k > st[0] and lines[k].strip().startswith("#") and _indent(lines[k]) == dash:
            comment.insert(0, lines[k])
            k -= 1
        item = {"line": i + 1, "keys": {}, "comment": "\n".join(comment), "run": ""}
        first = line[dash + 1:].strip()
        body = [(i, " " * (dash + 2) + first)] if first else []
        i += 1
        while i < stop and (not lines[i].strip() or _indent(lines[i]) > dash
                            or (lines[i].strip().startswith("#") and _indent(lines[i]) >= dash
                                and not (_indent(lines[i]) == dash and _next_is_item(lines, i, stop, dash)))):
            body.append((i, lines[i]))
            i += 1
        key_ind, cur = dash + 2, None
        for n, l in body:
            if not _content(l):
                if cur is not None:
                    item["keys"][cur].append(l)      # blank and comment lines stay with the key above (run bodies)
                continue
            if _indent(l) == key_ind:
                m = re.match(r"^\s*([A-Za-z0-9_-]+):(?:\s+(.*))?$", l)
                if not m:
                    raise ListShapeError("line %d: not a step key: %r" % (n + 1, l))
                cur = m.group(1)
                if cur in item["keys"]:
                    raise ListShapeError("line %d: step key `%s` twice" % (n + 1, cur))
                item["keys"][cur] = [l]
            elif _indent(l) > key_ind and cur is not None:
                item["keys"][cur].append(l)
            else:
                raise ListShapeError("line %d: line outside any step key: %r" % (n + 1, l))
        for key, raw in item["keys"].items():
            head = re.match(r"^\s*[A-Za-z0-9_-]+:(?:\s+(.*))?$", raw[0]).group(1) or ""
            item[key] = _unquote(head.strip()) if key in ("name", "uses", "if") else head.strip()
            if key == "run":
                item["run"] = "\n".join([head] + raw[1:])
        item.setdefault("name", None)
        item.setdefault("uses", None)
        item.setdefault("if", None)
        steps.append(item)
    if not steps:
        raise ListShapeError("job `%s`: 0 steps read" % job)
    return steps


def _unquote(v):
    """Strip quotes only from a FULLY quoted scalar (`'x'` or `"x"`); `a == 'b'` keeps its quotes."""
    if len(v) >= 2 and v[0] == v[-1] and v[0] in "'\"":
        return v[1:-1]
    return v


def _next_is_item(lines, i, stop, dash):
    """A comment at the item indent belongs to the NEXT item when the next content line is an item head."""
    j = i + 1
    while j < stop and not _content(lines[j]):
        j += 1
    return j < stop and _indent(lines[j]) == dash and re.match(r"^\s*-(\s|$)", lines[j]) is not None


def is_upload(step):
    uses = (step.get("uses") or "").split("@")[0].lower()
    return uses in UPLOAD_ACTIONS or bool(UPLOAD_RUN.search(step.get("run") or ""))


def read(path):
    with open(path, encoding="utf-8") as f:
        return f.read()


class StepListerControls(unittest.TestCase):
    """The lister must read a known shape exactly and refuse shapes it cannot read (positive and negative controls)."""

    SAMPLE = "\n".join([
        "name: sample",
        "jobs:",
        "  first:",
        "    runs-on: x",
        "    steps:",
        "      - uses: actions/checkout@v4",
        "",
        "      # comment that belongs to the next step",
        "      - name: one",
        "        if: always()",
        "        run: |",
        "          echo a",
        "          # a shell comment inside the body",
        "          - name: not a step (body text)",
        "",
        "      - name: two",
        "        uses: actions/upload-artifact@v4",
        "        with:",
        "          name: x",
        "  second:",
        "    needs: [first, other]",
        "    steps:",
        "      - name: three",
        "        run: gh release upload v1 file",
        "",
    ])

    def test_lister_reads_the_known_shape(self):
        steps = list_steps(self.SAMPLE, "first")
        self.assertEqual([s["name"] for s in steps], [None, "one", "two"])
        self.assertEqual(steps[0]["uses"], "actions/checkout@v4")
        self.assertEqual(steps[1]["if"], "always()")
        self.assertEqual(list_steps(self.SAMPLE.replace("if: always()", "if: matrix.os == 'w'"), "first")[1]["if"],
                         "matrix.os == 'w'")
        self.assertEqual(list_steps(self.SAMPLE.replace("- name: one", "- name: 'one'"), "first")[1]["name"], "one")
        self.assertIn("- name: not a step (body text)", steps[1]["run"])
        self.assertEqual(steps[1]["comment"], "      # comment that belongs to the next step")
        self.assertEqual([is_upload(s) for s in steps], [False, False, True])
        second = list_steps(self.SAMPLE, "second")
        self.assertEqual([s["name"] for s in second], ["three"])
        self.assertTrue(is_upload(second[0]))
        self.assertEqual(job_needs(self.SAMPLE, "second"), ["first", "other"])
        self.assertEqual(job_needs(self.SAMPLE, "first"), [])

    def test_lister_refuses_shapes_it_cannot_read(self):
        bad = {
            "item not starting with -": self.SAMPLE.replace("      - name: two", "      name: two"),
            "indent-5 line": self.SAMPLE.replace("        if: always()", "     if: always()"),
            "no steps key": self.SAMPLE.replace("    steps:\n      - uses", "    stops:\n      - uses"),
            "missing job": self.SAMPLE.replace("  first:", "  furst:"),
            "duplicate key": self.SAMPLE.replace("        if: always()", "        if: always()\n        if: never()"),
            "tab indent": self.SAMPLE.replace("        if: always()", "\tif: always()"),
            "odd needs": self.SAMPLE.replace("needs: [first, other]", "needs: ${{ x }}"),
        }
        for label, text in bad.items():
            with self.subTest(label):
                with self.assertRaises(ListShapeError):
                    list_steps(text, "first")
                    job_needs(text, "second")

    def test_lister_matches_extract_step_on_the_real_files(self):
        """Non-vacuity on the real files: many steps read, and the run body the lister sees for the gate is the one
        extract_step sees (same lines, before dedent)."""
        for path, job in ((RELEASE, RELEASE_JOB), (WINDOWS_BUILD, WB_JOB)):
            with self.subTest(os.path.basename(path)):
                steps = list_steps(read(path), job)
                self.assertGreaterEqual(len(steps), 8, "too few steps read - the lister is broken")
                gate = [s for s in steps if s["name"] == GATE]
                if len(gate) == 1:
                    raw = gate[0]["run"].split("\n")[1:]
                    first = next(l for l in raw if l.strip())
                    cut = _indent(first)
                    lister_body = "\n".join(l[cut:] for l in raw).rstrip("\n") + "\n"
                    self.assertEqual(lister_body, extract_step(path, GATE)["run"])


class HelpGatePin(unittest.TestCase):

    def _gate(self, path, job):
        """(steps, index of the gate, extract_step keys) - asserts exactly one gate step in that job."""
        steps = list_steps(read(path), job)
        idx = [k for k, s in enumerate(steps) if s["name"] == GATE]
        self.assertEqual(len(idx), 1, "%s job %s: %d steps named %r (1 expected) - the gate is missing or doubled"
                         % (os.path.basename(path), job, len(idx), GATE))
        try:
            keys = extract_step(path, GATE)
        except StepShapeError as e:
            self.fail("%s: the gate step cannot be read: %s" % (os.path.basename(path), e))
        return steps, idx[0], keys

    # A1-A4
    def test_a1_a4_release_step_shape(self):
        steps, k, keys = self._gate(RELEASE, RELEASE_JOB)
        self.assertEqual(keys.get("if"), WINDOWS_IF, "A2: the gate must run on the Windows leg only, unconditionally there")
        self.assertNotIn("continue-on-error", keys, "A3: continue-on-error would let a red gate upload")
        self.assertNotIn("continue-on-error", job_keys(read(RELEASE), RELEASE_JOB), "A3: job-level continue-on-error")
        self.assertEqual(keys.get("shell"), "pwsh", "A4: the native exit is read from $LASTEXITCODE in pwsh, not Git bash")
        self.assertEqual(set(keys) - {"name", "if", "timeout-minutes", "shell", "run", "env"}, set(),
                         "unexpected step keys (uses/with/working-directory change what runs)")

    # A5
    def _assert_body(self, path, job):
        _, _, keys = self._gate(path, job)
        body = keys["run"]
        lines = body.split("\n")
        def at(needle):
            hits = [n for n, l in enumerate(lines) if needle in l]
            self.assertEqual(len(hits), 1, "A5 %s: %r found on %d lines (1 expected)" % (os.path.basename(path), needle, len(hits)))
            return hits[0]
        self.assertIn("$exe = '%s'" % SIDECAR, lines, "A5: the gate must run the installer's sidecar file, nothing else")
        self.assertLess(at("$exe = '%s'" % SIDECAR), at("$full = (Resolve-Path -LiteralPath $exe).Path"),
                        "A5: the called path must be resolved from that file")
        loop = at("foreach ($argv in ")
        for argv in ("@('--help')", "@('queue', '--help')", "@('queue', 'list', '--help')"):
            self.assertIn(argv, lines[loop], "A5: argv set %s missing" % argv)
        reset = at("$global:LASTEXITCODE = $null")
        call = at("& $full @argv 2>&1")
        read_code = at("$code = $LASTEXITCODE")
        null_check = at("if ($null -eq $code) {")
        hexed = at("$hex = '0x{0:X8}' -f $code")
        nonzero = at("if ($code -ne 0) { $failed += ")
        failed = at("if ($failed.Count -gt 0) {")
        self.assertTrue(loop < reset < call < read_code < null_check < hexed < nonzero < failed,
                        "A5: order loop < LASTEXITCODE reset < call < read < null check < hex < non-zero check < verdict")
        self.assertIn("$failed += ", lines[null_check + 2], "A5: a missing exit code must count as a failure")
        self.assertEqual(lines[failed + 2].strip(), "exit 1", "A5: a failed run must end the step with exit 1")
        self.assertEqual(lines[at("is missing - the installer's sidecar") + 1].strip(), "exit 1",
                         "A5: a missing sidecar must end the step with exit 1")
        self.assertIn("Remove-Item Env:RUST_MIN_STACK -ErrorAction SilentlyContinue", lines, "A5: RUST_MIN_STACK not removed")
        self.assertIn("$PSNativeCommandUseErrorActionPreference = $false", lines, "A5: native error preference not pinned")
        self.assertLess(at("Remove-Item Env:RUST_MIN_STACK"), loop)
        self.assertEqual(lines[-2].strip(), "exit 0", "A5: the step must end with an explicit exit 0 after the verdict")
        # R1-1: nothing that neuters the gate was added (measured survivors before this check: M23-M30).
        flow = ["%s %s" % (m.group(1).lower(), m.group(2)) if m.group(2) else m.group(1).lower()
                for l in lines for m in FLOW.finditer(l)]
        self.assertEqual(flow, EXPECTED_FLOW, "A5: flow statements changed - an added exit/return/break/throw can make "
                         "the gate pass without running or judging")
        self.assertIsNone(FORBIDDEN.search(body), "A5: a process-exit or variable-rewriting command in the body")
        for var, want in ASSIGNMENTS.items():
            self.assertEqual(assignments(body, var), want, "A5: assignments to $%s changed" % var)
        depth = depths(lines)
        self.assertEqual([depth[loop], depth[call], depth[failed], depth[len(lines) - 2]], [0, 1, 0, 0],
                         "A5: the loop, the verdict and the final exit must be top level and the call inside the loop "
                         "(a wrapper such as if ($false) { ... } would skip them)")
        step_env = keys.get("env") or {}
        self.assertNotIn("RUST_MIN_STACK", step_env, "A5: the step env sets RUST_MIN_STACK")
        for line in env_lines(read(path), job):
            self.assertNotIn("RUST_MIN_STACK", line, "A5: the workflow or job env sets RUST_MIN_STACK: %r" % line)

    def test_a5_release_body(self):
        self._assert_body(RELEASE, RELEASE_JOB)

    def test_a5_windows_build_body(self):
        self._assert_body(WINDOWS_BUILD, WB_JOB)

    # A6
    def test_a6_release_order_build_gate_uploads(self):
        steps, k, _ = self._gate(RELEASE, RELEASE_JOB)
        build = [n for n, s in enumerate(steps) if s["name"] == RELEASE_BUILD_STEP]
        self.assertEqual(len(build), 1, "A6: the Windows build step %r is not found once" % RELEASE_BUILD_STEP)
        uploads = [n for n, s in enumerate(steps) if is_upload(s)]
        self.assertTrue(uploads, "A6: no upload step found in release.yml job build - the order check would be vacuous")
        self.assertLess(build[0], k, "A6: the gate runs before the build that makes the file")
        self.assertLess(k, min(uploads), "A6: an upload step runs before the gate: %s"
                        % [steps[n]["name"] or steps[n]["uses"] for n in uploads if n < k])

    # A7
    def test_a7_other_upload_jobs_need_build(self):
        text = read(RELEASE)
        jobs = re.findall(r"(?m)^  ([A-Za-z0-9_-]+):\s*(?:#.*)?$", text.split("\njobs:", 1)[1])
        self.assertIn(RELEASE_JOB, jobs)
        checked = 0
        for job in jobs:
            if job == RELEASE_JOB:
                continue
            if any(is_upload(s) for s in list_steps(text, job)):
                checked += 1
                self.assertIn(RELEASE_JOB, job_needs(text, job),
                              "A7: job %s uploads without needs: [%s] - it could upload while the gate is red" % (job, RELEASE_JOB))
        self.assertGreaterEqual(checked, 1, "A7: no other upload job found - pack-artifacts was expected (vacuous check)")

    # A8
    def test_a8_windows_build_dry_run_step(self):
        steps, k, keys = self._gate(WINDOWS_BUILD, WB_JOB)
        self.assertNotIn("if", keys, "A8: the dry run must not be conditional")
        self.assertNotIn("continue-on-error", keys, "A8: a non-blocking dry run would not stop the tag")
        self.assertNotIn("continue-on-error", job_keys(read(WINDOWS_BUILD), WB_JOB), "A8: job-level continue-on-error")
        self.assertEqual(keys.get("shell"), "pwsh", "A8: shell must be pwsh")
        self.assertEqual(set(keys) - {"name", "timeout-minutes", "shell", "run", "env"}, set(),
                         "A8: unexpected step keys (working-directory/uses/with change what runs)")
        build = [n for n, s in enumerate(steps) if s["name"] == WB_BUILD_STEP]
        self.assertEqual(len(build), 1, "A8: the build step %r is not found once" % WB_BUILD_STEP)
        uploads = [n for n, s in enumerate(steps) if is_upload(s)]
        self.assertTrue(uploads, "A8: no upload step found in windows-build.yml - the order check would be vacuous")
        self.assertLess(build[0], k, "A8: the dry run runs before the build that makes the file")
        self.assertLess(k, min(uploads), "A8: the dry run runs after an upload")

    # A9
    def test_a9_bodies_are_identical(self):
        a = self._gate(RELEASE, RELEASE_JOB)[2]["run"]
        b = self._gate(WINDOWS_BUILD, WB_JOB)[2]["run"]
        self.assertEqual(a, b, "A9: the dry run is no longer the same check as the release gate")

    # A10
    def test_a10_no_strings_the_workflow_readers_act_on(self):
        for path, job in ((RELEASE, RELEASE_JOB), (WINDOWS_BUILD, WB_JOB)):
            steps, k, keys = self._gate(path, job)
            body = keys["run"]
            whole = steps[k]["comment"] + "\n" + "\n".join(
                "%s: %s" % (key, val) for key, val in keys.items() if key != "run") + "\n" + body
            for needle in WHOLE_FILE_NEEDLES:
                with self.subTest(file=os.path.basename(path), needle=needle):
                    self.assertNotIn(needle, whole)
            for label, pattern in BODY_PATTERNS:
                with self.subTest(file=os.path.basename(path), pattern=label):
                    self.assertIsNone(pattern.search(body), "A10: %r in the run body" % label)
            self.assertNotIn("uses", keys)


if __name__ == "__main__":
    unittest.main(verbosity=2)
