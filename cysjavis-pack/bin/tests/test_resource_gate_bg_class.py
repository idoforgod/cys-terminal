#!/usr/bin/env python3
"""PROMOTE-1 (0.14.52 · B-328 #1, B-328-AMEND, B-333) - the base pack's `javis_resource_gate.py` edit of 2026-10-17
promoted into the product, byte-equal. stdlib unittest only.

What the promoted bytes change, and what this test pins:
  T1  NODE_EXCLUDE_PATTERNS, counted with the language parser (ast, splats resolved - never a line or regex selector):
      exactly 6 patterns in this order; BG_CLASS_PATTERNS = BG_HOST_PATTERNS + BG_SPARE_PATTERNS, all inside it.
  T2  `classify` (ps lines on stdin, deterministic): the 9-pid class measured live on 2026-10-20 19:14 (3 x
      `claude bg-pty-host` + 3 x `claude bg-spare`, 1 x `claude daemon run`, 2 x codex app-server-daemon) is not
      counted as nodes and the 6 bg-class lines are reported as `bg_procs`; seats are still counted; a seat whose
      prompt only MENTIONS those strings is still counted (argv0 anchor); the codex vendor native stays excluded.
  T3  the enforcement contract for a GIVEN node figure is the v0.14.51 one: `check --json` exit codes and decisions
      (verdict, trip metrics) for injected figures and for the context axis (the axis the resource-gate park protects:
      `--context`, `--context-soft 50`, `--context-hard 60`, `--require-context`). `bg_procs` is never a checked
      metric. This class must be green on the v0.14.51 file too.
  T4  `pty-headroom` and `bg-parents` are report-only: exit 0, JSON with `state`; `bg-parents` takes no threshold
      flag (exit 64); `pty-headroom --rate` labels its arithmetic as not a forecast.

Enforcement is untouched; the gate's computed output CHANGES (T2, and `check` on a machine running those processes).

Hermetic: every child runs with a temporary HOME and CYS_STATE_DIR (`check` writes boot-nonce records there), no CYS_*
seat variables, PATH without `cys`, and every daemon/ledger input injected (`--dept-roster-override`,
`--servers-override`). Children run with `-I -B`; nothing is written into the checkout.
Red on v0.14.51: GATE_UNDER_TEST=<the v0.14.51 gate inside a copy of its bin/ directory> (T1, T2, T4 fail; T3 passes).
"""
import ast
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest

sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(HERE)
GATE = os.path.abspath(os.environ.get("GATE_UNDER_TEST") or os.path.join(BIN, "javis_resource_gate.py"))
PY = sys.executable or "python3"
EX_USAGE = 64

# 48 seats in 8 departments -> nodes hard = max(18, 12 + 48) = 60. No servers, quiet load, no fleet CPU, boot long past grace.
ROSTER = json.dumps({"active": 8, "seats": 48, "errors": [], "depts": []})
QUIET = ["--servers-override", "0", "--load-override", "0.5", "--fleet-cpu-override", "0.0",
         "--boot-elapsed-override", "100000", "--dept-roster-override", ROSTER]

# Live argv shapes of 2026-10-20 19:14 (the home directory is replaced by a placeholder).
SEATS = ["16 claude", "17 /Users/x/.local/bin/agy",
         "18 /Users/x/.local/share/claude/versions/2.1.302 --session-id s",
         "19 /Users/x/.local/lib/node_modules/@openai/codex/bin/codex.js"]
NINE = []
for _i, _sid in enumerate(("b90f5ad7", "259bedc4", "7dd105bd")):
    NINE.append("%d claude bg-pty-host --bg-pty-host /tmp/cc-daemon-501/f/spare/%s.pty.sock 200 50 -- "
                "/Users/x/.local/share/claude/versions/2.1.302 --bg-spare /tmp/cc-daemon-501/f/spare/%s.claim.sock"
                % (3000 + _i, _sid, _sid))
    NINE.append("%d claude bg-spare --bg-spare /tmp/cc-daemon-501/f/spare/%s.claim.sock" % (3100 + _i, _sid))
NINE += ["65085 /Users/x/.local/bin/claude daemon run --json-path /Users/x/.cys/claude/daemon.json --log-file /Users/x/l",
         "14471 /Users/x/.codex/packages/app-server-daemon/releases/0.163.0-aarch64-apple-darwin/bin/codex app-server daemon",
         "97212 /Users/x/.codex/packages/app-server-daemon/releases/0.163.0-aarch64-apple-darwin/bin/codex app-server --listen x"]
MENTIONS = ["13 claude -p fix the bg-pty-host leak",
            "14 /Users/x/.local/share/claude/versions/2.1.302 --session-id s -p bg-spare accumulation",
            "15 claude -p investigate claude daemon run growth",
            "21 claude -p look at .codex/packages/app-server-daemon/ logs"]
VENDOR = ["30 /Users/x/.local/lib/node_modules/@openai/codex/vendor/codex-darwin-arm64/codex"]

WANT_PATTERNS = [
    r"codex-darwin-arm64",
    r"(^|/)claude\s+bg-pty-host\b",
    r"(^|/)claude\s+bg-spare\b",
    r"/claude/versions/[^ ]+\s+--bg-spare\b",
    r"(^|/)claude\s+daemon\s+run\b",
    r"/\.codex/packages/app-server-daemon/",
]


def run(args, stdin=None, extra_env=None):
    """One gate child in its own temporary HOME / state dir; returns (rc, stdout, stderr)."""
    td = tempfile.mkdtemp(prefix="gate-bg-")
    try:
        env = {"HOME": td, "PATH": "/usr/bin:/bin:/usr/sbin:/sbin", "LANG": "en_US.UTF-8",
               "CYS_STATE_DIR": os.path.join(td, "state")}
        for k in ("SYSTEMROOT", "WINDIR", "TEMP", "TMP", "USERPROFILE"):   # Windows runtime needs these to start
            if os.environ.get(k):
                env[k] = os.environ[k]
        env.update(extra_env or {})
        p = subprocess.run([PY, "-I", "-B", GATE] + list(args), input=stdin, capture_output=True, text=True,
                           env=env, cwd=td, timeout=120)
        return p.returncode, p.stdout, p.stderr
    finally:
        shutil.rmtree(td, ignore_errors=True)


def classify(lines):
    rc, out, err = run(["classify"], stdin="\n".join(lines) + "\n")
    if rc != 0:
        raise AssertionError("classify rc=%r stderr=%r" % (rc, err[-300:]))
    return json.loads(out)


def module_lists(path):
    """Module-level list constants of `path`, resolved through *NAME splats with ast (refuses anything else)."""
    with open(path, encoding="utf-8") as f:
        tree = ast.parse(f.read())
    env = {n.targets[0].id: n.value for n in tree.body
           if isinstance(n, ast.Assign) and len(n.targets) == 1 and isinstance(n.targets[0], ast.Name)}

    def resolve(v, seen=()):
        if not isinstance(v, (ast.List, ast.Tuple)):
            raise AssertionError("not a list literal: %s" % ast.dump(v)[:80])
        out = []
        for e in v.elts:
            if isinstance(e, ast.Constant) and isinstance(e.value, str):
                out.append(e.value)
            elif isinstance(e, ast.Starred) and isinstance(e.value, ast.Name) and e.value.id in env \
                    and e.value.id not in seen:
                out.extend(resolve(env[e.value.id], seen + (e.value.id,)))
            else:
                raise AssertionError("unresolvable element: %s" % ast.dump(e)[:80])
        return out
    return {k: resolve(v) for k, v in env.items() if isinstance(v, (ast.List, ast.Tuple))
            and k.endswith("_PATTERNS")}


class T1PatternsByAst(unittest.TestCase):
    def test_node_exclude_patterns_are_the_six(self):
        lists = module_lists(GATE)
        self.assertEqual(lists.get("NODE_EXCLUDE_PATTERNS"), WANT_PATTERNS)

    def test_bg_class_is_host_plus_spare_and_excluded(self):
        lists = module_lists(GATE)
        self.assertEqual(lists.get("BG_CLASS_PATTERNS"),
                         lists.get("BG_HOST_PATTERNS", []) + lists.get("BG_SPARE_PATTERNS", []))
        self.assertEqual(len(lists["BG_CLASS_PATTERNS"]), 3)
        self.assertTrue(set(lists["BG_CLASS_PATTERNS"]) <= set(lists["NODE_EXCLUDE_PATTERNS"]))


class T2Classify(unittest.TestCase):
    def test_nine_pid_class_is_not_nodes(self):
        r = classify(NINE)
        self.assertEqual((r["nodes"], r.get("bg_procs")), (0, 6), r)

    def test_seats_still_counted_beside_the_class(self):
        self.assertEqual(classify(SEATS)["nodes"], 4)
        r = classify(SEATS + NINE)
        self.assertEqual((r["nodes"], r.get("bg_procs")), (4, 6), r)

    def test_each_shape_alone(self):
        for label, lines, nodes, bg in (
                ("bg pair", NINE[0:2], 0, 2),
                ("bg exec shape", ["2700 /Users/x/.local/share/claude/versions/2.1.302 --bg-spare /tmp/x.claim.sock"], 0, 1),
                ("transient daemon", NINE[6:7], 0, 0),
                ("codex app-server-daemon x2", NINE[7:9], 0, 0)):
            with self.subTest(label):
                r = classify(lines)
                self.assertEqual((r["nodes"], r.get("bg_procs")), (nodes, bg), r)

    def test_mentions_are_seats(self):
        r = classify(MENTIONS)
        self.assertEqual((r["nodes"], r.get("bg_procs")), (4, 0), r)

    def test_codex_vendor_native_still_excluded(self):
        self.assertEqual(classify(VENDOR)["nodes"], 0)


class T3EnforcementContract(unittest.TestCase):
    """Measured on v0.14.51 (d768fe08) and on the promoted bytes (f264f99e): identical."""
    ROWS = [
        (["--nodes-override", "5"], 0, "allow", []),
        (["--nodes-override", "12"], 1, "soft_warn", ["nodes"]),
        (["--nodes-override", "13"], 1, "soft_warn", ["nodes"]),
        (["--nodes-override", "59"], 1, "soft_warn", ["nodes"]),
        (["--nodes-override", "60"], 2, "hard_block", ["nodes"]),
        (["--nodes-override", "61"], 2, "hard_block", ["nodes"]),
        (["--nodes-override", "67"], 2, "hard_block", ["nodes"]),
        (["--nodes-override", "5", "--context", "55"], 1, "soft_warn", ["context_pct"]),
        (["--nodes-override", "5", "--context", "65"], 2, "hard_block", ["context_pct"]),
        (["--nodes-override", "5", "--require-context"], 1, "soft_warn", []),
        (["--nodes-override", "5", "--require-context", "--context", "30"], 0, "allow", []),
    ]

    def test_rows(self):
        for extra, want_rc, want_verdict, want_trips in self.ROWS:
            with self.subTest(extra):
                rc, out, err = run(["check", "--json"] + extra + QUIET)
                self.assertEqual(rc, want_rc, err[-300:])
                doc = json.loads(out)
                self.assertEqual(doc.get("verdict"), want_verdict)
                self.assertEqual([t.get("metric") for t in doc.get("trips") or []], want_trips)
                metrics = [c.get("metric") for c in doc.get("checks") or []]
                self.assertNotIn("bg_procs", metrics + want_trips)


class T4ReportOnlySubcommands(unittest.TestCase):
    def test_always_exit_zero_with_state(self):
        for args in (["pty-headroom", "--json"], ["bg-parents", "--json"]):
            with self.subTest(args):
                rc, out, err = run(args)
                self.assertEqual(rc, 0, err[-300:])
                self.assertIn("state", json.loads(out))

    def test_bg_parents_takes_no_threshold(self):
        rc, _out, err = run(["bg-parents", "--max-age", "1"])
        self.assertEqual(rc, EX_USAGE)
        self.assertIn("unrecognized arguments: --max-age", err)   # the subcommand exists; the flag does not

    def test_rate_is_labelled_arithmetic(self):
        rc, out, _err = run(["pty-headroom", "--rate", "2"])
        self.assertEqual(rc, 0)
        if out.startswith("pty_headroom: used="):          # measured on this host
            self.assertIn("예보가 아니다", out)
        else:                                              # lsof / sysctl unreadable: honest not_measured line
            self.assertTrue(out.startswith("pty_headroom: not_measured"), out)


if __name__ == "__main__":
    unittest.main(verbosity=2)
