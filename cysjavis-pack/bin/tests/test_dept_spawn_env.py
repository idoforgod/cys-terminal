#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_dept_spawn_env.py — what cys-dept prepares for a lane's daemon (0.14.57 P1 parts 3 and 5).

Runs the real cys-dept in test_dept_create_progress.Sandbox: tmp HOME, stub cys and cysd, no live daemon or pack.

    python3 -B cysjavis-pack/bin/tests/test_dept_spawn_env.py -v

Each `[control]` line prints what was measured.
"""
import glob
import os
import re
import subprocess
import sys
import tempfile
import time
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from rig_axes import assert_sealed  # noqa: E402
from test_dept_create_progress import DEPT, Sandbox, UP_AT_PING, _write_exec  # noqa: E402

# The stub daemon dumps its env. It runs as a background child, so a test waits for the rename, never for a partial file.
ENV_CYSD = r'''#!/bin/sh
env > "$HOME/spawn-env.tmp"
mv "$HOME/spawn-env.tmp" "$HOME/spawn-env"
# A98: publish the fixture readiness event before exiting. The old stub exited
# before the synthetic ping-count threshold and never published a ready socket.
# A successful ping now wins over this exited fixture; no persistent child needed.
mkdir -p "$(dirname "$CYS_SOCKET")"
touch "$CYS_SOCKET"
exit 0
'''
VERBS = (("launch", ("launch", "dept-1")), ("allocate", ("allocate",)), ("create", ("create", "k1")))


EXPORT = 'CYS_PROJECT_ROOT="$pack"'
ALIASES = ("JAVIS_SOCKET", "AITERM_SOCKET")


def setUpModule():
    """Sandbox runs inherit this process's cwd, and the resolvers ascend from cwd. Start from nowhere near a real _round."""
    cwd = os.getcwd()
    unittest.addModuleCleanup(os.chdir, cwd)
    os.chdir(tempfile.gettempdir())


def control(msg):
    print("[control] " + msg, flush=True)


def spawn_lines(text):
    """Every logical shell line of cys-dept that starts the daemon. Found by what it does, never by line number."""
    lines = re.sub(r"\\\n\s*", " ", text).splitlines()
    return [ln.strip() for ln in lines if not ln.lstrip().startswith("#") and "nohup" in ln and '"$CYSD"' in ln]


def missing_export(text):
    return [ln for ln in spawn_lines(text) if EXPORT not in ln]


def missing_scrub(text):
    return [ln for ln in spawn_lines(text) if not all("-u %s" % a in ln for a in ALIASES)]


class RigSandbox(Sandbox):
    """Sandbox with every axis named. It leaves cwd and the state env to the caller, and that let a test reach a real store."""

    def __init__(self, case, **env_extra):
        super().__init__(**env_extra)
        self.case = case
        local = os.path.join(self.home, "AppData", "Local")
        os.makedirs(local)
        self.env.update({"USERPROFILE": self.home, "LOCALAPPDATA": local, "JAVIS_ROOT": self.home,
                         "CYS_STATE_DIR": os.path.join(self.home, "st"),
                         "CYS_PACK_DIR": os.path.join(self.home, ".cys", "pack"),
                         "CYS_PROBE_RUNS": os.path.join(self.home, "runs.jsonl")})

    def run(self, *args, **kw):
        before = os.getcwd()
        os.chdir(self.home)
        try:
            assert_sealed(self.case, self.env, self.home)
            return super().run(*args, **kw)
        finally:
            os.chdir(before)


class Verbs(unittest.TestCase):
    def spawn(self, argv):
        """Run one verb to a real spawn. STUB_PING_OK_FROM carries it past the pre-check and the ready wait."""
        sb = RigSandbox(self, CYS_NO_AUTOSTART="1", STUB_PING_OK_FROM=UP_AT_PING, JAVIS_SOCKET="/stale/javis.sock",
                        AITERM_SOCKET="/stale/aiterm.sock")
        self.addCleanup(sb.cleanup)
        sb.seed_catalog("k1", "m1")
        _write_exec(os.path.join(sb.home, ".local", "bin", "cysd"), ENV_CYSD)
        rc, out, err = sb.run(*argv)
        self.assertEqual(rc, 0, err[-600:])
        packs = glob.glob(os.path.join(sb.home, ".cys", "pack-dept-*"))
        self.assertEqual(len(packs), 1, packs)
        return sb, packs[0]

    def formation_pack(self, sb, pack):
        """The real detector the fleet uses for 'is this lane's pack installed'."""
        name = os.path.basename(pack)[len("pack-dept-"):]
        script = ('source "$CYS_DEPT_SCRIPT" sock probe-src >/dev/null 2>&1 || true\n'
                  'formation_lane_pack "$FLP_NAME" "$(dept_sock "$FLP_NAME")"\n')
        r = subprocess.run(["bash", "-c", script], capture_output=True, text=True, encoding="utf-8", cwd=sb.home,
                           env=dict(sb.env, CYS_DEPT_SCRIPT=DEPT, FLP_NAME=name), timeout=30)
        self.assertEqual(r.returncode, 0, r.stderr[-400:])
        return r.stdout.strip()


class RoundDir(Verbs):
    def test_every_spawn_verb_makes_the_round_dir(self):
        for verb, argv in VERBS:
            sb, pack = self.spawn(argv)
            rd = os.path.join(pack, "_round")
            control("P1 %s: _round is a dir=%s, empty=%s" % (verb, os.path.isdir(rd), os.listdir(rd) == []))
            self.assertTrue(os.path.isdir(rd), verb)

    def test_round_dir_cannot_make_an_uninstalled_pack_look_installed(self):
        for verb, argv in VERBS:
            sb, pack = self.spawn(argv)
            markers = {m: os.path.exists(os.path.join(pack, m))
                       for m in (".pack-version", "bin/javis_boot_node.py", "bin/cys-dept", "bin/javis_formation.py")}
            before = self.formation_pack(sb, pack)
            control("P1 %s, not installed, _round present: every install marker absent=%s, detector says=%r" % (
                verb, not any(markers.values()), before))
            self.assertFalse(any(markers.values()), markers)
            self.assertEqual(before, "")
            # The detector can say yes: without this the empty answer above proves nothing.
            os.makedirs(os.path.join(pack, "bin"))
            with open(os.path.join(pack, "bin", "javis_boot_node.py"), "w", encoding="utf-8") as f:
                f.write("")
            after = self.formation_pack(sb, pack)
            control("P1 %s, positive control, real marker planted: detector says the pack=%s" % (verb, after == pack))
            self.assertEqual(after, pack)


class SpawnEnv(Verbs):
    def read_env(self, sb):
        """Wait for the stub daemon, a background child."""
        path = os.path.join(sb.home, "spawn-env")
        deadline = time.time() + 30
        while not os.path.exists(path):
            self.assertLess(time.time(), deadline, "the stub daemon never ran")  # wallclock: guard 30 s only bounds a hang; the verdict is the env read below
            time.sleep(0.02)  # wallclock: guard polling a file a child writes once, not a duration the test rests on
        with open(path, encoding="utf-8") as f:
            return dict(ln.split("=", 1) for ln in f.read().splitlines() if "=" in ln)

    def test_every_spawn_verb_exports_the_lane_root(self):
        for verb, argv in VERBS:
            sb, pack = self.spawn(argv)
            env = self.read_env(sb)
            control("P1 %s: daemon env CYS_PROJECT_ROOT is the lane pack=%s, CYS_PACK_DIR same=%s, _round present=%s" % (
                verb, env.get("CYS_PROJECT_ROOT") == pack, env.get("CYS_PACK_DIR") == pack,
                os.path.isdir(os.path.join(env.get("CYS_PROJECT_ROOT", ""), "_round"))))
            self.assertEqual(env.get("CYS_PROJECT_ROOT"), pack, verb)
            stale = [a for a in ALIASES if a in env]
            control("P1R %s: caller had stale aliases, daemon env has=%s" % (verb, stale))
            self.assertEqual(stale, [], verb)
            self.assertTrue(os.path.isdir(os.path.join(pack, "_round")), verb)


class ContextCommand(Verbs):
    def test_a_context_command_carries_the_lanes_root(self):
        sb, pack = self.spawn(VERBS[0][1])
        rc, out, err = sb.run("dept-1", "--", "env")
        self.assertEqual(rc, 0, err[-400:])
        env = dict(ln.split("=", 1) for ln in out.splitlines() if "=" in ln)
        control("P1R context command for dept-1: CYS_PROJECT_ROOT is the lane pack=%s" % (env.get("CYS_PROJECT_ROOT") == pack))
        self.assertEqual(env.get("CYS_PROJECT_ROOT"), pack)


class UnrestartedLane(Verbs):
    """A pack with no _round, the case the natural-restart window leaves behind: dept-3, 6, 7, 8, 9, 10 today."""
    # cwd first: the resolvers ascend from cwd, and a sealed HOME does not stop that reaching a real ~/_round.
    RESOLVE = ("import os, sys\nos.chdir(os.environ['HOME'])\nsys.path.insert(0, sys.argv[1])\n"
               "import javis_state_ledger as l\nprint('@@' + l.PROJECT)\n"
               "home = os.path.realpath(os.environ['HOME'])\n"
               "if not os.path.realpath(l.ledger_path()).startswith(home + os.sep):\n"
               "    sys.exit('outside the rig, refusing: ' + l.ledger_path())\n"
               "l.append_record('note', 'test', {'text': 'x'})\n")

    def sandbox(self):
        sb = RigSandbox(self, CYS_NO_AUTOSTART="1")
        self.addCleanup(sb.cleanup)
        os.makedirs(os.path.join(sb.home, "_round"))
        return sb

    def test_a_context_command_resolves_to_the_lane_not_to_base(self):
        sb = self.sandbox()
        pack = os.path.join(sb.home, ".cys", "pack-dept-dept-7")
        os.makedirs(pack)
        rc, out, err = sb.run("dept-7", "--", sys.executable, "-B", "-c", self.RESOLVE, os.path.dirname(HERE))
        self.assertEqual(rc, 0, err[-400:])
        project = [x for x in out.splitlines() if x.startswith("@@")][-1][2:]
        lane = os.path.join(pack, "_round", "STATE_LEDGER.jsonl")
        base = os.path.join(sb.home, "_round", "STATE_LEDGER.jsonl")
        control("P1R3 context command, lane with no _round: resolved to the pack=%s, lane ledger=%s, base ledger=%s" % (
            project == pack, os.path.exists(lane), os.path.exists(base)))
        self.assertEqual(project, pack)
        self.assertTrue(os.path.exists(lane))
        self.assertFalse(os.path.exists(base))

    def test_a_registered_lane_with_no_pack_claims_no_root(self):
        """Accepted limit: the ledger falls to HOME as at the tag. The env must not say otherwise."""
        sb = self.sandbox()
        sb.seed_entry("dept-9", "m9", age=9999)
        sb.env["CYS_PROJECT_ROOT"] = "/stale/root/of/the/caller"
        rc, out, err = sb.run("dept-9", "--", "env")
        env = dict(ln.split("=", 1) for ln in out.splitlines() if "=" in ln)
        control("P1R4 registered lane, pack missing: rc=%s, CYS_PROJECT_ROOT exported=%s, pack made=%s" % (
            rc, "CYS_PROJECT_ROOT" in env, os.path.exists(os.path.join(sb.home, ".cys", "pack-dept-dept-9"))))
        self.assertEqual(rc, 0, err[-300:])
        self.assertNotIn("CYS_PROJECT_ROOT", env)
        self.assertFalse(os.path.exists(os.path.join(sb.home, ".cys", "pack-dept-dept-9")))
        rc, out, err = sb.run("dept-9", "--", sys.executable, "-B", "-c", self.RESOLVE, os.path.dirname(HERE))
        project = [x for x in out.splitlines() if x.startswith("@@")][-1][2:]
        control("P1R4 same lane, the known limit: resolver falls to HOME=%s" % (project == sb.home))
        self.assertEqual(os.path.realpath(project), os.path.realpath(sb.home))

    def test_the_resolver_refuses_to_write_outside_the_rig(self):
        """Control for the guard above, written after a run of this file reached the live base ledger."""
        sb = self.sandbox()
        os.makedirs(os.path.join(sb.home, ".cys", "pack-dept-dept-7"))
        code = self.RESOLVE.replace("os.chdir(os.environ['HOME'])", "os.environ['HOME'] = os.path.join(os.environ['HOME'], 'other')")
        rc, _, err = sb.run("dept-7", "--", sys.executable, "-B", "-c", code, os.path.dirname(HERE))
        control("P1R4 rig check on the resolver: rc=%s refused=%s" % (rc, "outside the rig" in err))
        self.assertNotEqual(rc, 0)
        self.assertIn("outside the rig", err)

    def test_a_context_command_for_no_such_lane_makes_no_pack(self):
        sb = self.sandbox()
        pack = os.path.join(sb.home, ".cys", "pack-dept-dept-9")
        rc, _, err = sb.run("dept-9", "--", "true")
        control("P1R3 context command, no such lane: rc=%s, pack made=%s" % (rc, os.path.exists(pack)))
        self.assertEqual(rc, 0, err[-300:])
        self.assertFalse(os.path.exists(pack))


class Census(unittest.TestCase):
    def setUp(self):
        with open(DEPT, encoding="utf-8") as f:
            self.text = f.read()

    def test_every_spawn_line_exports_the_lane_root(self):
        lines = spawn_lines(self.text)
        control("P1 census: %d spawn lines, without the export=%d" % (len(lines), len(missing_export(self.text))))
        self.assertGreaterEqual(len(lines), 1, "the census found no spawn line, so it proves nothing")
        self.assertEqual(missing_export(self.text), [])
        control("P1R census: spawn lines that keep an inherited alias=%d" % len(missing_scrub(self.text)))
        self.assertEqual(missing_scrub(self.text), [])

    def test_positive_control_a_new_spawn_line_without_it_fails(self):
        planted = self.text + '\n      CYS_SOCKET="$sock" CYS_PACK_DIR="$pack" env -u CYS_ROLE nohup "$CYSD" >/dev/null 2>&1 &\n'
        control("P1 census control, one planted line: caught=%d" % len(missing_export(planted)))
        self.assertEqual(len(missing_export(planted)), 1)
        self.assertEqual(len(missing_scrub(planted)), 1)

    def test_positive_control_the_continuation_form_is_seen(self):
        planted = 'CYS_SOCKET="$sock" CYS_PACK_DIR="$pack" \\\n  env -u CYS_ROLE nohup "$CYSD" &\n'
        self.assertEqual(len(spawn_lines(planted)), 1)
        self.assertEqual(len(missing_export(planted)), 1)

    def test_every_pack_dir_mkdir_also_makes_the_round_dir(self):
        sites = [ln for ln in self.text.splitlines() if "mkdir -p" in ln and '"$pack/directives"' in ln]
        control("P1 census: %d mkdir sites, all with _round=%s" % (len(sites), all('"$pack/_round"' in s for s in sites)))
        self.assertGreaterEqual(len(sites), 1)
        self.assertTrue(all('"$pack/_round"' in s for s in sites), sites)


if __name__ == "__main__":
    unittest.main(verbosity=2)
