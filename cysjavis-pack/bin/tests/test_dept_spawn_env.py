#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_dept_spawn_env.py — what cys-dept prepares for a lane's daemon (0.14.57 P1 parts 3 and 5).

Runs the real cys-dept in test_dept_create_progress.Sandbox: tmp HOME, stub cys and cysd, no live daemon or pack.

    python3 -B cysjavis-pack/bin/tests/test_dept_spawn_env.py -v

Each `[control]` line prints what was measured.
"""
import glob
import os
import subprocess
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from test_dept_create_progress import DEPT, Sandbox, UP_AT_PING, _write_exec  # noqa: E402

# The stub daemon dumps its env. It runs as a background child, so a test waits for the rename, never for a partial file.
ENV_CYSD = r'''#!/bin/sh
env > "$HOME/spawn-env.tmp"
mv "$HOME/spawn-env.tmp" "$HOME/spawn-env"
exit 0
'''
VERBS = (("launch", ("launch", "dept-1")), ("allocate", ("allocate",)), ("create", ("create", "k1")))


def control(msg):
    print("[control] " + msg, flush=True)


class Verbs(unittest.TestCase):
    def spawn(self, argv):
        """Run one verb to a real spawn. STUB_PING_OK_FROM carries it past the pre-check and the ready wait."""
        sb = Sandbox(CYS_NO_AUTOSTART="1", STUB_PING_OK_FROM=UP_AT_PING)
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
        r = subprocess.run(["bash", "-c", script], capture_output=True, text=True, encoding="utf-8",
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


if __name__ == "__main__":
    unittest.main(verbosity=2)
