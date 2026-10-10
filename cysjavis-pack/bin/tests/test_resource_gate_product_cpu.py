#!/usr/bin/env python3
"""F5 (0.14.50 · backlog B-050) — `javis_resource_gate.py product-cpu`: the product's OWN CPU,
REPORT-ONLY. Binding tests 1-11 and 13-15 of PLAN-50 sheet F5-C (design DESIGN-F5 section 7 plus
the derived conditions). stdlib unittest only.

What is pinned (the design's own sentence: "the report-only block must never consume the gate's
deadline"):
  T1  product-cpu attributes the app's WebKit helper (fails on the tag: usage error 64)
  T2  another application's helper is not counted
  T3  `check --json` byte-identical + same exit code (vs the tag file when its git object is
      present, and always vs the module's own `main()` without the new program entry); text
      `check` = the reference text as an exact prefix + exactly one further line, same exit code
  T4  one class per pid; the tag's fleet pids all land in daemon / agent_cli / fleet_other
  T5  lookup missing / raising / garbage -> unattributed with a reason, never zero; `check`
      output and exit code unchanged (F5-A2)
  T6  Windows host -> `unavailable`; no ps, no child, no ctypes lookup
  T7  no app process -> app and app_helpers are 0, not null (and no lookup is made)
  T8  caller boundary, product code, UNSCALED: the real completion guard (hook process) with a
      `ps` that stalls 12 s for the NEW read only -> PASS, inside 10 s, no run.failed,
      infra_count 0; same for the formation (30 s / 15 s) and bootstrap (30 s) call shapes
  T9  text `check` with a stalled ps and with a hung lookup: verdict first, one `not measured`
      line with the reason, bounded by 3.0 s + the reference time, exit code unchanged, no child
      left behind
  T10 `--self-test` starts no product-cpu child and makes no lookup call
  T11 static pin: only the gate itself, the role allow-list and tests name `product-cpu`; the
      tail is reachable only from the program entry; the block never calls `_ps_cpu_lines` /
      `_ppid_map`
  T13 (role gate) lives in the hook's own self-test (role-capability-gate.sh) - run here too
  T14 product-cpu always exits 0; a failure is `state` + `reason`
  T15 no gate / caller timeout is lengthened

Red on the base: run with GATE_UNDER_TEST=<copy of the tag's gate in a dir with its siblings>.
F5_REQUIRE_TAG_REF=1 turns "tag object absent" from a skip into a failure (evidence runs).
"""
import ast
import contextlib
import importlib.util
import io
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
import unittest
from unittest import mock

sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(HERE)
ROOT = os.path.dirname(os.path.dirname(BIN))
GATE = os.path.abspath(os.environ.get("GATE_UNDER_TEST") or os.path.join(BIN, "javis_resource_gate.py"))
GATE_DIR = os.path.dirname(GATE)
TAG_REF = "51b00d7a"            # v0.14.49 = base of integ/0.14.50-b (the "tag" of the design)
TAG_PATH = "cysjavis-pack/bin/javis_resource_gate.py"
LINE_HEAD = "product itself (report-only, not a gate input): "
NEW_COLS = "pid=,ppid=,pcpu=,command="
LOOKUP_ENV = "CYS_GATE_PRODUCT_CPU_LOOKUP_OVERRIDE"
POSIX = os.name == "posix"


def load_gate(path=GATE, name="gate_under_test_f5"):
    if GATE_DIR not in sys.path:
        sys.path.insert(0, GATE_DIR)
    spec = importlib.util.spec_from_file_location(name, path)
    g = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(g)
    return g


G = load_gate()

# ── fixtures (shapes of the captured table sim/ps-table.tsv, 2026-10-10 17:46 KST; pids moved
#    above 99999 so they can never be this process on macOS) ─────────────────────────────────
WK = "/System/Library/Frameworks/WebKit.framework/Versions/A/XPCServices"
APP = "/Applications/cys.app/Contents/MacOS/cys-app"
ROWS_T1 = [
    (900001, 1, 4.3, "/sbin/launchd"),
    (914305, 1, 1.3, APP),
    (914362, 1, 75.5, WK + "/com.apple.WebKit.WebContent.xpc/Contents/MacOS/com.apple.WebKit.WebContent"),
]
ROWS_RICH = ROWS_T1 + [
    (914363, 1, 1.0, WK + "/com.apple.WebKit.GPU.xpc/Contents/MacOS/com.apple.WebKit.GPU"),
    (914371, 1, 1.0, WK + "/com.apple.WebKit.Networking.xpc/Contents/MacOS/com.apple.WebKit.Networking"),
    (901301, 1, 0.0, "/System/Applications/Mail.app/Contents/MacOS/Mail"),
    (901391, 1, 2.5, WK + "/com.apple.WebKit.GPU.xpc/Contents/MacOS/com.apple.WebKit.GPU"),
    (914209, 1, 1.4, "/Applications/cys.app/Contents/MacOS/cysd"),
    (914452, 914209, 0.1, "/Applications/cys.app/Contents/Resources/runtime/python/bin/python3 "
                          "/opt/home/.cys/pack/bin/javis_hud_bridge.py"),
    (914461, 914452, 0.0, "/Applications/cys.app/Contents/MacOS/cys events --reconnect"),
    (902001, 1, 10.0, "/opt/bin/claude --resume"),
    (902002, 1, 3.0, "/opt/bin/codex exec"),
    (902003, 1, 0.5, "/opt/bin/serena start-mcp-server"),
    (903000, 1, 50.0, "/usr/bin/python3 /tmp/agy/report.py"),              # not ours (B2 shape)
    (903001, 1, 9.0, "/Applications/Claude.app/Contents/MacOS/Claude"),    # GUI bundle, not ours
]
MAP_RICH = {"914362": 914305, "914363": 914305, "914371": 914305, "901391": 901301}
OUR_HELPERS = (914362, 914363, 914371)


def table_text(rows):
    return "".join("%d %d %.1f %s\n" % r for r in rows)


def resolver(mapping):
    calls = []

    def resolve():
        def fn(pid):
            calls.append(pid)
            return mapping.get(str(pid), -1)
        return fn
    resolve.calls = calls
    return resolve


def collect(rows, resolve, **kw):
    kw.setdefault("windows", False)
    kw.setdefault("darwin", True)
    kw.setdefault("self_pid", -1)
    kw.setdefault("ncpu", 16)
    return G._product_cpu_collect(read_ps=lambda: (list(rows), None, None), resolve=resolve, **kw)


# ── hermetic subprocess harness: stub `ps` / `cys` first in PATH, scrubbed env ───────────────
STUB_PS = r"""#!/bin/sh
# F5 test stub ps. Logs every call; the NEW read (pid=,ppid=,pcpu=,command=) can be stalled.
echo "$*" >> "$STUB_DIR/ps.log"
case "$*" in
  *"pid=,ppid=,pcpu=,command="*)
    echo "$*" >> "$STUB_DIR/ps-new.log"
    if [ -n "$STUB_PS_NEW_STALL" ]; then
      echo $$ > "$STUB_DIR/ps-new.pid"
      exec sleep "$STUB_PS_NEW_STALL"
    fi
    cat "$STUB_DIR/new-table.txt" ;;
  *"pid,pcpu,command"*)
    echo "  PID  %CPU COMMAND"
    echo "902001 10.0 /opt/bin/claude --resume"
    echo "914209  1.4 /Applications/cys.app/Contents/MacOS/cysd" ;;
  *"pid,command"*)
    echo "  PID COMMAND"
    echo "902001 /opt/bin/claude --resume" ;;
  *) : ;;
esac
exit 0
"""
STUB_CYS = "#!/bin/sh\necho \"$*\" >> \"$STUB_DIR/cys.log\"\nexit 0\n"
ROSTER = '{"active":0,"seats":0,"errors":[],"depts":[]}'
HERMETIC = ["--dept-roster-override", ROSTER, "--boot-elapsed-override", "99999"]


class Harness:
    def __init__(self, rows=ROWS_RICH):
        self.td = tempfile.mkdtemp(prefix="f5-pc-")
        self.stub = os.path.join(self.td, "stub")
        os.makedirs(self.stub)
        for name, body in (("ps", STUB_PS), ("cys", STUB_CYS)):
            p = os.path.join(self.stub, name)
            with open(p, "w", encoding="utf-8") as f:
                f.write(body)
            os.chmod(p, 0o755)
        with open(os.path.join(self.stub, "new-table.txt"), "w", encoding="utf-8") as f:
            f.write(table_text(rows))
        self.home = os.path.join(self.td, "home")
        os.makedirs(os.path.join(self.home, "state"))

    def env(self, **extra):
        env = {k: v for k, v in os.environ.items()
               if not re.match(r"^_?CYS_", k) and k not in ("CLAUDE_BIN", "CLAUDE_CONFIG_DIR",
                                                             LOOKUP_ENV)}
        env.update({"HOME": self.home, "CYS_STATE_DIR": os.path.join(self.home, "state"),
                    "CYS_NO_AUTOSTART": "1", "STUB_DIR": self.stub,
                    "PATH": self.stub + os.pathsep + "/usr/bin" + os.pathsep + "/bin",
                    LOOKUP_ENV: json.dumps({"map": MAP_RICH})})
        for k, v in extra.items():
            if v is None:
                env.pop(k, None)
            else:
                env[k] = v
        return env

    def run(self, gate, args, timeout=60, **extra):
        t0 = time.monotonic()
        r = subprocess.run([sys.executable, gate] + list(args), capture_output=True,
                           timeout=timeout, env=self.env(**extra))
        return r.returncode, r.stdout, r.stderr, time.monotonic() - t0

    def run_main_only(self, gate, args, timeout=60, **extra):
        """The module's own `main()` WITHOUT the program entry (= the tag's code path)."""
        code = ("import sys,importlib.util;sys.dont_write_bytecode=True;"
                "sys.path.insert(0,%r);s=importlib.util.spec_from_file_location('g',%r);"
                "g=importlib.util.module_from_spec(s);s.loader.exec_module(g);"
                "sys.exit(g.main(sys.argv[1:]))" % (os.path.dirname(gate), gate))
        t0 = time.monotonic()
        r = subprocess.run([sys.executable, "-c", code] + list(args), capture_output=True,
                           timeout=timeout, env=self.env(**extra))
        return r.returncode, r.stdout, r.stderr, time.monotonic() - t0

    def log(self, name):
        p = os.path.join(self.stub, name)
        if not os.path.isfile(p):
            return []
        with open(p, encoding="utf-8") as f:
            return [ln for ln in f.read().splitlines() if ln.strip()]

    def close(self):
        shutil.rmtree(self.td, ignore_errors=True)


def tag_bin_dir(td):
    """A copy of this tree's bin/*.py with the TAG's gate file -> path of that gate | None."""
    try:
        blob = subprocess.run(["git", "-C", ROOT, "show", "%s:%s" % (TAG_REF, TAG_PATH)],
                              capture_output=True, timeout=30)
    except (OSError, subprocess.SubprocessError):
        return None
    if blob.returncode != 0 or not blob.stdout:
        return None
    d = os.path.join(td, "tagbin")
    os.makedirs(d)
    for n in os.listdir(BIN):
        if n.endswith(".py"):
            shutil.copy2(os.path.join(BIN, n), os.path.join(d, n))
    with open(os.path.join(d, "javis_resource_gate.py"), "wb") as f:
        f.write(blob.stdout)
    return os.path.join(d, "javis_resource_gate.py")


def need_tag(testcase, tag_gate):
    if tag_gate:
        return
    msg = "tag object %s absent in this checkout (shallow clone?)" % TAG_REF
    if os.environ.get("F5_REQUIRE_TAG_REF") == "1":
        testcase.fail(msg + " and F5_REQUIRE_TAG_REF=1")
    testcase.skipTest(msg)


def alive(pid):
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    # a zombie still answers kill(0); ps tells the state (if ps cannot run, kill(0) stands)
    try:
        r = subprocess.run(["/bin/ps", "-o", "stat=", "-p", str(pid)], capture_output=True,
                           text=True, timeout=5)
    except (OSError, subprocess.SubprocessError):
        return True
    return bool(r.stdout.strip()) and not r.stdout.strip().startswith("Z")


# Existing check fixtures (the override shapes test_resource_gate_fleet_cpu / the self-test use).
CHECK_FIXTURES = {
    "allow": ["--servers-override", "0", "--nodes-override", "0", "--load-override", "0.0",
              "--fleet-cpu-override", "0.0"],
    "servers_soft": ["--servers-override", "2", "--nodes-override", "0", "--load-override", "0.0",
                     "--fleet-cpu-override", "0.0"],
    "servers_hard": ["--servers-override", "99", "--nodes-override", "0", "--load-override", "0.0",
                     "--fleet-cpu-override", "0.0"],
    "fleet_soft": ["--servers-override", "0", "--nodes-override", "0", "--load-override", "0.0",
                   "--fleet-cpu-override", "0.6"],
    "fleet_hard": ["--servers-override", "0", "--nodes-override", "0", "--load-override", "0.0",
                   "--fleet-cpu-override", "1.2"],
    "load_soft": ["--servers-override", "0", "--nodes-override", "0", "--load-override", "9999",
                  "--fleet-cpu-override", "0.0"],
    "require_context": ["--servers-override", "0", "--nodes-override", "0", "--load-override", "0.0",
                        "--fleet-cpu-override", "0.0", "--require-context"],
    "context_hard": ["--servers-override", "0", "--nodes-override", "0", "--load-override", "0.0",
                     "--fleet-cpu-override", "0.0", "--context", "70"],
    "fleet_live_stub_ps": ["--servers-override", "0", "--nodes-override", "0",
                           "--load-override", "0.0"],
}


def fixture_args(name):
    args = list(CHECK_FIXTURES[name])
    roster = ROSTER
    boot = ["--boot-elapsed-override", "99999"]
    if name == "fleet_hard_in_grace":
        boot = ["--boot-elapsed-override", "10"]
    return args + ["--dept-roster-override", roster] + boot


# ═════════════════════════════════════════════════════════════════════════════════════════════
class T01HelperAttributed(unittest.TestCase):
    def test_1_subprocess_product_cpu_counts_the_app_helper(self):
        """Fails on the tag: `product-cpu` does not exist there (usage error 64)."""
        h = Harness(rows=ROWS_T1)
        try:
            rc, out, err, _ = h.run(GATE, ["product-cpu", "--json"],
                                    **{LOOKUP_ENV: json.dumps({"map": {"914362": 914305}})})
            self.assertEqual(rc, 0, "rc=%s stderr=%s" % (rc, err[-400:]))
            b = json.loads(out)
            self.assertEqual(b["state"], "ok")
            self.assertEqual(b["app_helpers"]["pcpu"], 75.5)
            self.assertEqual(b["app"]["pcpu"], 1.3)
            self.assertEqual(b["total"]["pcpu"], 76.8)
            for k in ("state", "reason", "app", "app_helpers", "helpers_unattributed", "daemons",
                      "jobs", "total", "total_known", "agent_cli", "label"):
                self.assertIn(k, b)
        finally:
            h.close()

    def test_1_pure_classifier(self):
        b = collect(ROWS_T1, resolver({"914362": 914305}))
        self.assertEqual(b["app_helpers"], {"procs": 1, "pcpu": 75.5, "cores": 0.755})


class T02ForeignHelper(unittest.TestCase):
    def test_2_other_applications_helper_left_out(self):
        b = collect(ROWS_RICH, resolver(MAP_RICH))
        self.assertEqual(b["state"], "ok")
        self.assertEqual(b["app_helpers"]["procs"], 3)
        self.assertEqual(b["app_helpers"]["pcpu"], 77.5)
        self.assertEqual(b["foreign_helpers_left_out"], 1)
        self.assertEqual(b["daemons"]["pcpu"], 1.4)
        self.assertEqual(b["jobs"], {"procs": 2, "pcpu": 0.1, "cores": 0.001})
        self.assertEqual(b["total"]["pcpu"], round(1.3 + 77.5 + 1.4 + 0.1, 1))
        self.assertEqual(b["agent_cli"]["pcpu"], 13.0)          # separate figure, not in total
        self.assertEqual(b["fleet_other"]["pcpu"], 0.5)

    def test_2_mutant_count_every_webkit_helper_is_caught(self):
        # if attribution were ignored (every helper counted), 901391 would add 2.5
        b = collect(ROWS_RICH, resolver(dict(MAP_RICH, **{"901391": 914305})))
        self.assertEqual(b["app_helpers"]["pcpu"], 80.0, "fixture sanity: the foreign helper is 2.5")


class T03CheckUnchanged(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.h = Harness()
        cls.tag_gate = tag_bin_dir(cls.h.td)

    @classmethod
    def tearDownClass(cls):
        cls.h.close()

    def _cmp_json(self, ref_runner, ref_gate):
        for name in CHECK_FIXTURES:
            args = ["check", "--json"] + fixture_args(name)
            r_rc, r_out, _e, _ = ref_runner(ref_gate, args)
            n_rc, n_out, n_err, _ = self.h.run(GATE, args)
            self.assertEqual(n_out, r_out, "%s: check --json bytes differ" % name)
            self.assertEqual(n_rc, r_rc, "%s: exit code differs" % name)
            self.assertNotIn(b"product", n_out, "%s: product block leaked into check --json" % name)

    def _cmp_text(self, ref_runner, ref_gate):
        for name in CHECK_FIXTURES:
            args = ["check"] + fixture_args(name)
            r_rc, r_out, _e, _ = ref_runner(ref_gate, args)
            n_rc, n_out, n_err, _ = self.h.run(GATE, args)
            self.assertTrue(n_out.startswith(r_out), "%s: reference text is not an exact prefix\n"
                            "REF=%r\nNEW=%r" % (name, r_out[-200:], n_out[-300:]))
            extra = n_out[len(r_out):].decode("utf-8")
            self.assertEqual(extra.count("\n"), 1, "%s: not exactly one further line: %r" % (name, extra))
            self.assertTrue(extra.startswith(LINE_HEAD), "%s: %r" % (name, extra))
            self.assertEqual(n_rc, r_rc, "%s: exit code differs" % name)

    def test_3a_check_json_vs_tag_file(self):
        need_tag(self, self.tag_gate)
        self._cmp_json(self.h.run, self.tag_gate)

    def test_3b_text_check_vs_tag_file(self):
        need_tag(self, self.tag_gate)
        self._cmp_text(self.h.run, self.tag_gate)

    def test_3c_check_json_vs_main_without_entry(self):
        self._cmp_json(self.h.run_main_only, GATE)

    def test_3d_text_check_vs_main_without_entry(self):
        self._cmp_text(self.h.run_main_only, GATE)

    def test_3e_json_abbreviation_gets_no_tail(self):
        args = ["check", "--js"] + fixture_args("allow")
        r_rc, r_out, _e, _ = self.h.run_main_only(GATE, args)
        n_rc, n_out, _e2, _ = self.h.run(GATE, args)
        self.assertEqual((n_rc, n_out), (r_rc, r_out))
        json.loads(n_out)

    def test_3f_check_json_never_runs_the_new_read(self):
        before = len(self.h.log("ps-new.log"))
        for name in CHECK_FIXTURES:
            self.h.run(GATE, ["check", "--json"] + fixture_args(name))
        self.assertEqual(len(self.h.log("ps-new.log")), before)


class T02bParentChainRule(unittest.TestCase):
    """Non-macOS rule (design section 3: a helper is a descendant of the app). The WebKitGTK
    shapes are general knowledge, not verified on a real Linux machine (stated limit)."""

    def test_2b_descendant_counted_foreign_left_out_cycle_safe(self):
        rows = [(950010, 1, 2.0, "/usr/bin/cys-app"),
                (950011, 950010, 30.0, "/usr/lib/x86_64-linux-gnu/webkit2gtk-4.1/WebKitWebProcess 7 9"),
                (950012, 950011, 5.0, "/usr/lib/x86_64-linux-gnu/webkit2gtk-4.1/WebKitNetworkProcess"),
                (950020, 1, 1.0, "/usr/bin/epiphany"),
                (950021, 950020, 40.0, "/usr/lib/x86_64-linux-gnu/webkit2gtk-4.1/WebKitWebProcess"),
                (950031, 950032, 3.0, "/usr/lib/x86_64-linux-gnu/webkit2gtk-4.1/WebKitWebProcess"),
                (950032, 950031, 0.0, "/usr/bin/loop-parent")]
        saved = os.environ.pop(LOOKUP_ENV, None)
        try:
            b = G._product_cpu_collect(read_ps=lambda: (list(rows), None, None), resolve=None,
                                       windows=False, darwin=False, self_pid=-1, ncpu=4)
        finally:
            if saved is not None:
                os.environ[LOOKUP_ENV] = saved
        self.assertEqual(b["attribution"], "parent-chain")
        self.assertEqual(b["state"], "ok")
        self.assertEqual(b["app_helpers"], {"procs": 2, "pcpu": 35.0, "cores": 0.35})
        self.assertEqual(b["foreign_helpers_left_out"], 2)
        self.assertEqual(b["total"]["pcpu"], 37.0)


class T04OneClassPerPid(unittest.TestCase):
    def test_4_classes_disjoint_and_fleet_pids_kept(self):
        def attribute(hp, tp, ap, pm):
            return G._pc_attribute_lookup(resolver(MAP_RICH), hp, tp)
        tot, run_reason, _bad = G._pc_classify(list(ROWS_RICH), attribute, self_pid=-1)
        self.assertIsNone(run_reason)
        # one class per pid: every counted process is counted once -> the class counts add up to
        # exactly the number of our rows (+ the one foreign helper), and each row's pcpu once.
        not_ours = (900001, 901301, 903000, 903001)
        ours = [r for r in ROWS_RICH if r[0] not in not_ours]
        self.assertEqual(sum(v[0] for v in tot.values()), len(ours), tot)
        self.assertAlmostEqual(sum(v[1] for v in tot.values()), sum(r[2] for r in ours))
        fleet_rows, why = G._fleet_rows(["%d %.1f %s" % (r[0], r[2], r[3]) for r in ROWS_RICH],
                                        self_pid=-1)
        self.assertIsNone(why)
        fleet_pids = {r["pid"] for r in fleet_rows}
        self.assertEqual(fleet_pids, {914209, 902001, 902002, 902003})
        self.assertEqual(tot["daemon"][0] + tot["agent_cli"][0] + tot["fleet_other"][0],
                         len(fleet_pids))
        self.assertAlmostEqual(tot["daemon"][1] + tot["agent_cli"][1] + tot["fleet_other"][1],
                               sum(r["pcpu"] for r in fleet_rows))

    def test_4_mutant_daemon_counted_twice_is_caught(self):
        b = collect(ROWS_RICH, resolver(MAP_RICH))
        self.assertEqual(b["daemons"]["procs"], 1)


class T05LookupFailures(unittest.TestCase):
    def _assert_run_failure(self, b, exc_name):
        self.assertEqual(b["state"], "partial")
        self.assertIsNone(b["app_helpers"], "a failed lookup must not read as zero")
        self.assertIsNone(b["total"])
        self.assertEqual(b["helpers_unattributed"]["procs"], 4)         # 3 ours + 1 foreign
        self.assertEqual(b["helpers_unattributed"]["pcpu"], 80.0)
        self.assertIn(exc_name, b["reason"])
        self.assertEqual(b["total_known"]["pcpu"], round(1.3 + 1.4 + 0.1, 1))
        self.assertEqual(b["app"]["pcpu"], 1.3)
        self.assertEqual(b["agent_cli"]["pcpu"], 13.0)

    def test_5_symbol_missing(self):
        def resolve():
            raise AttributeError("dlsym(...): symbol not found")
        self._assert_run_failure(collect(ROWS_RICH, resolve), "AttributeError")

    def test_5_call_raises(self):
        import ctypes
        for exc in (OSError, RuntimeError, ctypes.ArgumentError):
            def resolve(exc=exc):
                def fn(pid):
                    raise exc("stub")
                return fn
            self._assert_run_failure(collect(ROWS_RICH, resolve), exc.__name__)

    def test_5_garbage_answers(self):
        for garbage in (-1, 0, 999999999, "914305", None, True, 914305.0):
            m = dict(MAP_RICH, **{"914362": garbage})
            b = collect(ROWS_RICH, resolver(m))
            self.assertEqual(b["state"], "partial", garbage)
            self.assertEqual(b["helpers_unattributed"]["procs"], 1, garbage)
            self.assertEqual(b["helpers_unattributed"]["pcpu"], 75.5, garbage)
            self.assertIsNone(b["total"], garbage)
            self.assertIn(repr(garbage), b["reason"])
            self.assertEqual(b["app_helpers"]["pcpu"], 2.0, garbage)    # never counted as the app's

    def test_5_real_resolver_failure_is_guarded(self):
        with mock.patch.object(G, "_pc_resolve_responsible_lookup",
                               side_effect=AttributeError("gone")):
            b = G._product_cpu_collect(read_ps=lambda: (list(ROWS_RICH), None, None),
                                       windows=False, darwin=True, self_pid=-1)
        self.assertEqual(b["state"], "partial")
        self.assertIsNone(b["app_helpers"])

    def test_5_check_unchanged_under_lookup_failure(self):
        h = Harness()
        try:
            args = ["check"] + fixture_args("servers_soft")
            r_rc, r_out, _e, _ = h.run_main_only(GATE, args)
            jr_rc, jr_out, _e, _ = h.run_main_only(GATE, ["check", "--json"] + fixture_args("servers_soft"))
            for spec in ({"missing": True}, {"raise": "OSError"}, {"raise": "RuntimeError"},
                         {"raise": "ArgumentError"}, {"map": {"914362": -1, "914363": 0,
                                                              "914371": "x"}}):
                n_rc, n_out, _e, _ = h.run(GATE, args, **{LOOKUP_ENV: json.dumps(spec)})
                self.assertEqual(n_rc, r_rc, spec)
                self.assertTrue(n_out.startswith(r_out), spec)
                tail = n_out[len(r_out):].decode()
                self.assertEqual(tail.count("\n"), 1, (spec, tail))
                self.assertIn("UNATTRIBUTED", tail, (spec, tail))
                j_rc, j_out, _e, _ = h.run(GATE, ["check", "--json"] + fixture_args("servers_soft"),
                                           **{LOOKUP_ENV: json.dumps(spec)})
                self.assertEqual((j_rc, j_out), (jr_rc, jr_out), spec)
                p_rc, p_out, _e, _ = h.run(GATE, ["product-cpu", "--json"],
                                           **{LOOKUP_ENV: json.dumps(spec)})
                self.assertEqual(p_rc, 0, spec)
                self.assertEqual(json.loads(p_out)["state"], "partial", spec)
        finally:
            h.close()


class T06Windows(unittest.TestCase):
    def test_6_windows_host_unavailable_no_ps_no_lookup(self):
        def boom(*_a, **_k):
            raise AssertionError("must not be called on a Windows host")
        with mock.patch.object(G, "_pc_resolve_responsible_lookup", side_effect=boom), \
                mock.patch.object(G, "_pc_read_ps", side_effect=boom):
            b = G._product_cpu_collect(windows=True)
        self.assertEqual(b["state"], "unavailable")
        self.assertIsNone(b["app"])
        self.assertIsNone(b["total"])

    def test_6_windows_decision_is_the_shared_host_function(self):
        def boom(*_a, **_k):
            raise AssertionError("must not be called on a Windows host")
        with mock.patch.object(G, "_is_windows_host", return_value=True), \
                mock.patch.object(G, "_pc_read_ps", side_effect=boom):
            b = G._product_cpu_collect()
        self.assertEqual(b["state"], "unavailable")

    def test_6_windows_text_tail_starts_no_child(self):
        def boom(*_a, **_k):
            raise AssertionError("no child on a Windows host")
        buf = io.StringIO()
        with mock.patch.object(G, "_is_windows_host", return_value=True), \
                mock.patch.object(G.subprocess, "Popen", side_effect=boom), \
                mock.patch.object(G, "_pc_run_child", side_effect=boom), \
                contextlib.redirect_stdout(buf):
            G._product_cpu_tail()
        self.assertEqual(buf.getvalue().count("\n"), 1)
        self.assertTrue(buf.getvalue().startswith(LINE_HEAD + "unavailable - "), buf.getvalue())


class T07NoApp(unittest.TestCase):
    def test_7_no_app_process_zero_not_null(self):
        rows = [r for r in ROWS_RICH if r[0] != 914305]

        def resolve():
            raise AssertionError("no lookup when no app process exists")
        b = collect(rows, resolve)
        self.assertEqual(b["state"], "ok")
        self.assertEqual(b["app"], {"procs": 0, "pcpu": 0.0, "cores": 0.0})
        self.assertEqual(b["app_helpers"], {"procs": 0, "pcpu": 0.0, "cores": 0.0})
        self.assertEqual(b["foreign_helpers_left_out"], 4)


class T08CallerBoundary(unittest.TestCase):
    """Product code, UNSCALED (caller timeouts 10 / 30 / 15 / 30 s); the stub `ps` stalls 12 s
    for the NEW read only (B-055 is not this fix: the tag's own reads answer normally)."""

    def _guard_root(self, h, name):
        root = os.path.join(h.td, name)
        tasks = os.path.join(root, "_round", "tasks")
        os.makedirs(tasks)
        sess = os.path.join(h.td, "sess.jsonl")
        with open(sess, "w") as f:
            f.write("{}\n")
        now = time.time()
        os.utime(sess, (now - 10, now - 10))
        status = os.path.join(h.stub, "status.json")
        with open(status, "w", encoding="utf-8") as f:
            json.dump({"surfaces": [{"surface_id": 7, "role": "worker",
                                     "usage": {"source": "statusline", "ctx_pct": 20.0,
                                               "ctx_tokens": 1000, "session_file": sess,
                                               "updated_at": now}}]}, f)
        with open(os.path.join(h.stub, "cys"), "w", encoding="utf-8") as f:
            f.write('#!/bin/sh\necho "$*" >> "$STUB_DIR/cys.log"\ncase "$1" in\n'
                    '  status) cat "%s" 2>/dev/null || exit 1 ;;\n  send) exit 1 ;;\n'
                    '  *) exit 0 ;;\nesac\n' % status)
        os.chmod(os.path.join(h.stub, "cys"), 0o755)
        boot = os.path.join(h.td, "boot.marker")
        with open(boot, "w") as f:
            f.write("boot\n")
        os.utime(boot, (now - 3600, now - 3600))
        rec = {"id": "T1", "title": "T1", "status": "in_progress", "created_at": "x",
               "updated_at": "x", "verify_spec": {"mode": "command", "cmd": "exit 0",
                                                 "pass_rule": {"kind": "exit_map",
                                                               "pass_exits": [0]},
                                                 "timeout_s": 5}}
        with open(os.path.join(tasks, "T1.json"), "w", encoding="utf-8") as f:
            json.dump(rec, f)
        with open(os.path.join(tasks, ".guard-claim.7"), "w", encoding="utf-8") as f:
            json.dump({"task_id": "T1", "pid": os.getpid(), "pid_source": "caller", "ts": "x"}, f)
        return root, boot

    def _run_guard(self, h, guard, root, boot):
        env = h.env(STUB_PS_NEW_STALL="12")
        env.update({"GRILL_MARKER": os.path.join(root, ".grill-absent.json"), "JAVIS_ROOT": root,
                    "HUD_STATE_DIR": os.path.join(root, "hud"),
                    "CYS_PROBE_RUNS": os.path.join(root, "probe_runs.jsonl"),
                    "CYS_GUARD_BOOT_MARKER": boot, "CYS_GUARD_LIVENESS": "alive",
                    "CYS_ROLE": "worker", "CYS_COMPLETION_GUARD": "1", "CYS_SURFACE_ID": "7",
                    "CYS_GUARD_RESOURCE_ARGS": " ".join(
                        ["--servers-override", "0", "--nodes-override", "0", "--load-override",
                         "0.0", "--boot-elapsed-override", "99999", "--dept-roster-override",
                         ROSTER.replace(" ", "")]),
                    "JAVIS_WAKEUP_LIVENESS": "alive", "JAVIS_FASTFAIL_MAX": "99"})
        t0 = time.monotonic()
        r = subprocess.run([sys.executable, guard],
                           input=json.dumps({"stop_hook_active": False,
                                             "transcript_path": "/nonexistent.jsonl"}),
                           capture_output=True, text=True, env=env, timeout=120)
        return r.returncode, time.monotonic() - t0, r.stderr

    def _guard_rows(self, gate):
        h = Harness()
        try:
            guard = os.path.join(os.path.dirname(gate), "javis_completion_guard.py")
            self.assertTrue(os.path.isfile(guard), guard)
            root, boot = self._guard_root(h, "r")
            out = []
            for _ in range(3):
                rc, dt, err = self._run_guard(h, guard, root, boot)
                st_p = os.path.join(root, "_round", "tasks", "T1.guard.json")
                with open(st_p, encoding="utf-8") as f:
                    st = json.load(f)
                spool = os.path.join(root, "hud", "evt_spool.jsonl")
                evts = []
                if os.path.isfile(spool):
                    with open(spool, encoding="utf-8") as f:
                        evts = [json.loads(x) for x in f.read().splitlines() if x.strip()]
                out.append({"rc": rc, "dt": dt, "verdict": st.get("last_verdict"),
                            "infra_count": int(st.get("infra_count", 0)),
                            "run_failed": sum(1 for e in evts if e.get("type") == "run.failed"),
                            "err": err[-300:]})
            out.append({"new_reads": len(h.log("ps-new.log")),
                        "gate_reads": sum(1 for x in h.log("ps.log") if "pid,pcpu,command" in x)})
            return out
        finally:
            h.close()

    def test_8a_completion_guard_real_hook_new_read_stalled(self):
        if not POSIX:
            self.skipTest("guard battery is POSIX-only (its own contract)")
        rows = self._guard_rows(GATE)
        meta = rows.pop()
        for i, r in enumerate(rows):
            self.assertEqual(r["rc"], 0, (i, r))
            self.assertEqual(r["verdict"], "PASS", (i, r))
            self.assertEqual(r["infra_count"], 0, (i, r))
            self.assertEqual(r["run_failed"], 0, (i, r))
            self.assertLess(r["dt"], 10.0, (i, r))
        self.assertEqual(meta["new_reads"], 0, "the guard's gate call ran the new read")
        self.assertGreaterEqual(meta["gate_reads"], 3, "the gate's own ps read did not happen: %r" % meta)

    def test_8b_same_rows_with_the_tag_gate(self):
        if not POSIX:
            self.skipTest("guard battery is POSIX-only (its own contract)")
        td = tempfile.mkdtemp(prefix="f5-tag-")
        try:
            tag_gate = tag_bin_dir(td)
            need_tag(self, tag_gate)
            new = [{k: r[k] for k in ("rc", "verdict", "infra_count", "run_failed")}
                   for r in self._guard_rows(GATE)[:-1]]
            tag = [{k: r[k] for k in ("rc", "verdict", "infra_count", "run_failed")}
                   for r in self._guard_rows(tag_gate)[:-1]]
            self.assertEqual(new, tag)
        finally:
            shutil.rmtree(td, ignore_errors=True)

    def test_8c_formation_and_bootstrap_shapes(self):
        h = Harness()
        try:
            shapes = [("formation", ["check", "--json", "--formation-size", "3"], 30),
                      ("formation-retry", ["check", "--json"], 15),
                      ("bootstrap", ["check", "--json"], 30)]
            for name, args, tmo in shapes:
                a = args + HERMETIC + ["--servers-override", "0", "--nodes-override", "0",
                                       "--load-override", "0.0"]
                ok_rc, ok_out, _e, _ = h.run(GATE, a, timeout=tmo)
                st_rc, st_out, _e, dt = h.run(GATE, a, timeout=tmo, STUB_PS_NEW_STALL="12")
                self.assertEqual((st_rc, st_out), (ok_rc, ok_out), name)
                self.assertLess(dt, tmo, name)
            self.assertEqual(h.log("ps-new.log"), [])
        finally:
            h.close()

    def test_8d_control_the_stall_does_bite_the_new_read(self):
        h = Harness()
        try:
            rc, out, _e, dt = h.run(GATE, ["product-cpu", "--json"], STUB_PS_NEW_STALL="12")
            self.assertEqual(rc, 0)
            b = json.loads(out)
            self.assertEqual(b["state"], "not_measured")
            self.assertEqual(b["reason"], "ps timeout 1.5 s")
            self.assertLess(dt, 4.0)
            self.assertEqual(len(h.log("ps-new.log")), 1)
            with open(os.path.join(h.stub, "ps-new.pid")) as f:
                pid = int(f.read().strip())
            time.sleep(0.2)
            self.assertFalse(alive(pid), "stalled ps %d left behind" % pid)
        finally:
            h.close()


class T09TextCheckBounded(unittest.TestCase):
    def _timed_lines(self, h, args, **extra):
        t0 = time.monotonic()
        p = subprocess.Popen([sys.executable, GATE] + args, stdout=subprocess.PIPE,
                             stderr=subprocess.DEVNULL, env=h.env(**extra))
        lines = []
        for raw in iter(p.stdout.readline, b""):
            lines.append((time.monotonic() - t0, raw))
        p.stdout.close()
        rc = p.wait(timeout=30)
        return rc, lines, time.monotonic() - t0

    def _case(self, extra, reason, pidfile_name):
        h = Harness()
        try:
            args = ["check"] + fixture_args("servers_soft")
            r_rc, r_out, _e, r_dt = h.run_main_only(GATE, args)
            rc, lines, dt = self._timed_lines(h, args, **extra(h))
            out = b"".join(x[1] for x in lines)
            self.assertEqual(rc, r_rc)
            self.assertTrue(out.startswith(r_out), out[-300:])
            tail = out[len(r_out):].decode()
            self.assertEqual(tail, LINE_HEAD + "not measured - %s\n" % reason)
            self.assertLessEqual(dt, r_dt + 3.0 + 1.0, "dt=%.2f ref=%.2f" % (dt, r_dt))
            first_verdict = [t for t, ln in lines if ln.startswith(b"verdict:")]
            self.assertTrue(first_verdict and first_verdict[0] < r_dt + 1.0,
                            "verdict not readable before the wait: %r" % first_verdict)
            pf = os.path.join(h.stub if pidfile_name == "ps-new.pid" else h.td, pidfile_name)
            with open(pf) as f:
                pid = int(f.read().strip())
            time.sleep(0.2)
            self.assertFalse(alive(pid), "process %d left behind" % pid)
            return dt
        finally:
            h.close()

    def test_9a_stalled_ps(self):
        self._case(lambda h: {"STUB_PS_NEW_STALL": "12"}, "ps timeout 1.5 s", "ps-new.pid")

    def test_9b_hung_lookup(self):
        def extra(h):
            return {LOOKUP_ENV: json.dumps({"map": MAP_RICH, "hang_s": 12,
                                            "pid_file": os.path.join(h.td, "child.pid")})}
        self._case(extra, "deadline 3.0 s exceeded", "child.pid")

    def test_9c_healthy_text_line(self):
        h = Harness()
        try:
            rc, out, _e, _ = h.run(GATE, ["check"] + fixture_args("allow"))
            last = out.decode().splitlines()[-1]
            self.assertTrue(last.startswith(LINE_HEAD + "80.3 %CPU = 0.803 core of "), last)
            self.assertIn("screen helpers 77.5", last)
            self.assertIn("agent CLIs 13.0 %CPU = 0.130 core (separate figure)", last)
        finally:
            h.close()


class T10SelfTestSpawnsNothing(unittest.TestCase):
    def test_10_in_process(self):
        calls = {"tail": 0, "child": 0, "lookup": 0, "popen_pc": 0}
        real_popen = G.subprocess.Popen

        class RecPopen(real_popen):
            def __init__(self, argv, *a, **k):
                if "product-cpu" in [str(x) for x in (argv if isinstance(argv, list) else [argv])]:
                    calls["popen_pc"] += 1
                super().__init__(argv, *a, **k)

        def rec(key):
            def f(*_a, **_k):
                calls[key] += 1
                raise AssertionError(key)
            return f
        env_saved = os.environ.pop(LOOKUP_ENV, None)
        try:
            with mock.patch.object(G, "_product_cpu_tail", side_effect=rec("tail")), \
                    mock.patch.object(G, "_pc_run_child", side_effect=rec("child")), \
                    mock.patch.object(G, "_pc_resolve_responsible_lookup", side_effect=rec("lookup")), \
                    mock.patch.object(G.subprocess, "Popen", RecPopen), \
                    contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
                rc = G.self_test()
        finally:
            if env_saved is not None:
                os.environ[LOOKUP_ENV] = env_saved
        self.assertEqual(rc, 0)
        self.assertEqual(calls, {"tail": 0, "child": 0, "lookup": 0, "popen_pc": 0})

    def test_10_subprocess(self):
        h = Harness()
        try:
            trace = os.path.join(h.td, "lookup-trace.txt")
            rc, out, err, _ = h.run(GATE, ["--self-test"], timeout=120,
                                    **{LOOKUP_ENV: json.dumps({"map": MAP_RICH, "trace_file": trace})})
            self.assertEqual(rc, 0, out[-500:] + err[-500:])
            self.assertEqual(h.log("ps-new.log"), [], "self-test ran the new ps read")
            self.assertFalse(os.path.exists(trace), "self-test resolved / called the lookup")
            self.assertIn(b"F5 product-cpu", out)
        finally:
            h.close()


class T11StaticPin(unittest.TestCase):
    ALLOWED = {"cysjavis-pack/bin/javis_resource_gate.py",
               "cysjavis-pack/hooks/role-capability-gate.sh"}

    def test_11_no_caller_passes_product_cpu(self):
        hits = []
        for top in ("cysjavis-pack", "src-tauri/src", "ui/src", "scripts"):
            base = os.path.join(ROOT, top)
            for dp, dn, fn in os.walk(base):
                dn[:] = [d for d in dn if d not in ("node_modules", "target", ".git", "__pycache__")]
                rel_dp = os.path.relpath(dp, ROOT).replace(os.sep, "/")
                if rel_dp.startswith("cysjavis-pack/bin/tests"):
                    continue
                for n in fn:
                    p = os.path.join(dp, n)
                    try:
                        with open(p, encoding="utf-8", errors="ignore") as f:
                            if "product-cpu" not in f.read():
                                continue
                    except OSError:
                        continue
                    rel = os.path.relpath(p, ROOT).replace(os.sep, "/")
                    if rel not in self.ALLOWED:
                        hits.append(rel)
        self.assertEqual(hits, [], "a call site outside the gate / allow-list names product-cpu")

    def test_11_gate_critical_callers_build_check_json(self):
        shapes = {
            "cysjavis-pack/bin/javis_completion_guard.py":
                'argv = [sys.executable, gate, "check", "--require-context", "--json"]',
            "cysjavis-pack/bin/javis_formation.py":
                '[py, gate, "check", "--json", "--formation-size", str(len(REQUIRED_ROLES))]',
            "cysjavis-pack/bin/javis_bootstrap.py": '_run_split([py, gate, "check", "--json"],',
            "src-tauri/src/main.rs": '.arg("check").arg("--json")',
        }
        for rel, needle in shapes.items():
            with open(os.path.join(ROOT, rel), encoding="utf-8") as f:
                self.assertIn(needle, f.read(), rel)

    def test_11_tail_reachable_only_from_the_entry(self):
        with open(GATE, encoding="utf-8") as f:
            tree = ast.parse(f.read())
        funcs = {n.name: n for n in tree.body if isinstance(n, ast.FunctionDef)}

        def names(node):
            return {x.id for x in ast.walk(node) if isinstance(x, ast.Name)} | \
                   {x.attr for x in ast.walk(node) if isinstance(x, ast.Attribute)}
        new = {n for n in funcs if n.startswith("_pc_") or n.startswith("_product_cpu")
               or n == "cmd_product_cpu"}
        self.assertTrue({"_product_cpu_tail", "_pc_run_child", "_product_cpu_collect"} <= new)
        for fname in ("cmd_check", "main", "measure", "self_test", "evaluate"):
            if fname in funcs:
                used = names(funcs[fname]) & (new - {"cmd_product_cpu"})
                self.assertEqual(used, set(), "%s references %s" % (fname, used))
        callers = [n for n, fn in funcs.items() if "_product_cpu_tail" in names(fn) and n != "_product_cpu_tail"]
        self.assertEqual(callers, [], callers)
        mains = [n for n in tree.body if isinstance(n, ast.If) and "_product_cpu_tail" in names(n)]
        self.assertEqual(len(mains), 1)
        for fname in new:
            used = names(funcs[fname]) & {"_ps_cpu_lines", "_ppid_map", "_ps_lines", "measure"}
            self.assertEqual(used, set(), "%s uses %s" % (fname, used))


class T14AlwaysExitZero(unittest.TestCase):
    def test_14_failures_are_fields(self):
        h = Harness()
        try:
            cases = {
                "ps_stall": {"STUB_PS_NEW_STALL": "12"},
                "bad_override_json": {LOOKUP_ENV: "{not json"},
                "ps_absent": {"PATH": os.path.join(h.td, "empty")},
            }
            os.makedirs(os.path.join(h.td, "empty"))
            for name, extra in cases.items():
                rc, out, err, _ = h.run(GATE, ["product-cpu", "--json"], **extra)
                self.assertEqual(rc, 0, (name, err[-300:]))
                b = json.loads(out)
                self.assertIn(b["state"], ("not_measured", "unavailable", "partial"), (name, b))
                self.assertTrue(b["reason"], (name, b))
                rc2, out2, _e, _ = h.run(GATE, ["product-cpu"], **extra)
                self.assertEqual(rc2, 0, name)
                self.assertTrue(out2.decode().startswith(LINE_HEAD), name)
        finally:
            h.close()

    def test_14_internal_error_is_a_field(self):
        buf = io.StringIO()
        with mock.patch.object(G, "_product_cpu_collect", side_effect=RuntimeError("boom")), \
                contextlib.redirect_stdout(buf):
            rc = G.main(["product-cpu", "--json"])
        self.assertEqual(rc, 0)
        b = json.loads(buf.getvalue())
        self.assertEqual(b["state"], "not_measured")
        self.assertIn("RuntimeError", b["reason"])


class T15NoTimeoutLengthened(unittest.TestCase):
    LITERALS = {
        "cysjavis-pack/bin/javis_resource_gate.py": [
            'DEPT_STATUS_TIMEOUT = 5 ', 'LEDGER_TIMEOUT = 5 ', 'SOCKET_PROBE_TIMEOUT = 0.3 ',
            '["ps", "-axo", "pid,pcpu,command"], capture_output=True,\n'
            '                           text=True, errors="replace", timeout=10)',
            'PRODUCT_CPU_PS_TIMEOUT_S = 1.5 ', 'PRODUCT_CPU_CHILD_DEADLINE_S = 3.0 '],
        "cysjavis-pack/bin/javis_completion_guard.py": [
            'r = subprocess.run(argv, capture_output=True, text=True, timeout=10, env=env)'],
        "cysjavis-pack/bin/javis_formation.py": ['kw = {"timeout": 30}',
                                                 'RESOURCE_RETRY_TIMEOUT_S = 15\n'],
        "cysjavis-pack/bin/javis_bootstrap.py": ['timeout=_budget_leaf("RPC_SLACK_S", 10) * 3)'],
        "cysjavis-pack/bin/javis_budget.py": ['"RPC_SLACK_S": 10,'],
    }

    def test_15_literals(self):
        for rel, needles in self.LITERALS.items():
            with open(os.path.join(ROOT, rel), encoding="utf-8") as f:
                txt = f.read()
            for n in needles:
                self.assertIn(n, txt, "%s: %r" % (rel, n))

    def test_15_gate_timeouts_equal_the_tag_outside_the_new_block(self):
        td = tempfile.mkdtemp(prefix="f5-t15-")
        try:
            tag_gate = tag_bin_dir(td)
            need_tag(self, tag_gate)

            def timeout_lines(path, cut_block):
                with open(path, encoding="utf-8") as f:
                    txt = f.read()
                if cut_block:
                    a = txt.index("# ══ ★F5 (0.14.50")
                    b = txt.index("class _GateArgumentParser")
                    txt = txt[:a] + txt[b:]
                return sorted(ln.strip() for ln in txt.splitlines()
                              if re.search(r"timeout|TIMEOUT", ln) and not ln.strip().startswith("#"))
            self.assertEqual(timeout_lines(GATE, True), timeout_lines(tag_gate, False))
        finally:
            shutil.rmtree(td, ignore_errors=True)


if __name__ == "__main__":
    unittest.main(verbosity=2)
