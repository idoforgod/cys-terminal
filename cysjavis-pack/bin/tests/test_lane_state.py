#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_lane_state.py — lane-scoped autopilot state and the sweep guard (0.14.57 P1 parts 1 and 2).

Each lane is a real process running the shipped sweep_ledger, with the env a lane's job has: CYS_SOCKET and
CYS_PACK_DIR from the spawn line, CYS_PROJECT_ROOT as the `_builtin` job sets it (`${CYS_PROJECT_ROOT:-$HOME}`).
Rows are counted in the real STATE_LEDGER files. Sealed: tmp HOME, inherited CYS_*, JAVIS_*, AITERM_* and
CLAUDE_CONFIG_DIR dropped, no `cys` reachable except a stub in tmp.

    python3 -B cysjavis-pack/bin/tests/test_lane_state.py -v

Each `[control]` line prints what was measured.
"""
import json
import os
import shutil
import stat
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(HERE)
PY = sys.executable
DEPTS = ["dept-%d" % i for i in range(1, 11)]
ACTOR = "cycle-autopilot-sweep"
DRIVER = """
import json, os, sys
sys.path.insert(0, sys.argv[1])
import javis_cycle_autopilot as a
print(json.dumps({"sweep": a.sweep_ledger(), "project": a.PROJECT, "state_dir": a.STATE_DIR,
                  "heartbeat": a.HEARTBEAT, "own_root": a.lane_own_root()}))
"""
PROBE = """
import json, os, sys
sys.path.insert(0, sys.argv[1])
import javis_cycle_autopilot as a, javis_cycle_verifier as v, javis_preflight as p
print(json.dumps({"ap": [a.STATE_DIR, a.STATE_JSON, a.HEARTBEAT, a.BASELINE_DIR, a.SWEEP_STATE],
                  "vf": [v.STATE_DIR, v.STATE_JSON, v.HEARTBEAT, v.BASELINE_DIR],
                  "preflight_heartbeat": p._cycle_autopilot_mod().HEARTBEAT, "mode": a.mode()}))
"""


def control(msg):
    print("[control] " + msg, flush=True)


class Base(unittest.TestCase):
    def setUp(self):
        self.home = tempfile.mkdtemp(prefix="lane-state-")
        self.addCleanup(shutil.rmtree, self.home, True)
        self.base_round = os.path.join(self.home, "_round")
        os.makedirs(os.path.join(self.base_round, "handoffs"))
        os.makedirs(os.path.join(self.base_round, "tasks"))

    def pack(self, dept):
        return os.path.join(self.home, ".cys", "pack-dept-" + dept)

    def sock(self, dept):
        return os.path.join(self.home, ".local", "state", "cys-dept-" + dept, "cys.sock")

    def lane_round(self, dept):
        r = os.path.join(self.pack(dept), "_round")
        os.makedirs(os.path.join(r, "handoffs"), exist_ok=True)
        os.makedirs(os.path.join(r, "tasks"), exist_ok=True)
        return r

    def env(self, dept=None, exported=False, root=None, **extra):
        env = {k: v for k, v in os.environ.items()
               if not k.startswith(("CYS_", "JAVIS_", "AITERM_")) and k != "CLAUDE_CONFIG_DIR"}
        env.update({"HOME": self.home, "CYS_NO_AUTOSTART": "1", "CYS_AUTOPILOT_NO_SEND": "1",
                    "PATH": "/usr/bin:/bin"})
        env["CYS_PACK_DIR"] = self.pack(dept) if dept else os.path.join(self.home, ".cys", "pack")
        if dept:
            env["CYS_SOCKET"] = self.sock(dept)
        env["CYS_PROJECT_ROOT"] = root or (self.pack(dept) if exported else self.home)
        env.update(extra)
        return env

    def tick(self, dept=None, exported=False, root=None):
        r = subprocess.run([PY, "-B", "-c", DRIVER, BIN], capture_output=True, text=True, cwd=self.home,
                           env=self.env(dept, exported, root), timeout=60)
        self.assertEqual(r.returncode, 0, r.stderr)
        return json.loads(r.stdout.strip().splitlines()[-1])

    def rows(self, round_dir):
        p = os.path.join(round_dir, "STATE_LEDGER.jsonl")
        if not os.path.exists(p):
            return 0
        with open(p, encoding="utf-8") as f:
            return sum(1 for x in f if ACTOR in x)

    @staticmethod
    def handoff(round_dir, name):
        with open(os.path.join(round_dir, "handoffs", name + ".md"), "w", encoding="utf-8") as f:
            f.write("x")

    @staticmethod
    def done(round_dir, tid):
        with open(os.path.join(round_dir, "tasks", tid + ".json"), "w", encoding="utf-8") as f:
            json.dump({"id": tid, "status": "done", "owner": "w"}, f)


class Sweep(Base):
    def test_export_live_gives_zero_phantom_rows_and_a_real_handoff_gives_one(self):
        """p1_sim_sweep.py as a repo test. Its zero means nothing without the row at the end."""
        a, b = self.lane_round("dept-1"), self.lane_round("dept-2")
        for i in range(12):
            self.handoff(a, "a-h%d" % i)
        for i in range(20):
            self.done(a, "a-t%d" % i)
        for i in range(9):
            self.handoff(b, "b-h%d" % i)
        for i in range(15):
            self.done(b, "b-t%d" % i)
        per_tick = []
        for i in range(10):
            per_tick.append(self.tick(DEPTS[i % 2], exported=True)["sweep"]["emitted"])
        phantom = self.rows(a) + self.rows(b)
        control("P1 export live, 2 lanes, 10 ticks: rows per tick=%s phantom rows=%d" % (per_tick, phantom))
        self.assertEqual(phantom, 0)

        self.handoff(a, "real-new")
        other = self.tick("dept-2", exported=True)["sweep"]["emitted"]
        owner = self.tick("dept-1", exported=True)["sweep"]["emitted"]
        again = self.tick("dept-1", exported=True)["sweep"]["emitted"]
        control("P1 positive control, 1 real handoff in dept-1: other lane=%d owner lane=%d next tick=%d "
                "ledger rows dept-1=%d dept-2=%d" % (other, owner, again, self.rows(a), self.rows(b)))
        self.assertEqual((other, owner, again), (0, 1, 0))
        self.assertEqual((self.rows(a), self.rows(b)), (1, 0))

    def test_latent_window_eleven_lanes_one_root_two_events_two_rows(self):
        self.handoff(self.base_round, "old")
        lanes = [None] + DEPTS
        for lane in lanes:
            self.tick(lane)
        self.handoff(self.base_round, "new")
        self.done(self.base_round, "t1")
        results = {lane or "base": self.tick(lane) for lane in lanes}
        rows = self.rows(self.base_round)
        emitted = {k: v["sweep"]["emitted"] for k, v in results.items()}
        reasons = {k: v["sweep"].get("suppressed", {}).get("reason") for k, v in results.items()}
        control("P1 latent window, 11 lanes, one root, 2 real events: rows=%d emitted=%s" % (rows, emitted))
        control("P1 latent window, reasons: %s" % reasons)
        self.assertEqual(rows, 2)
        self.assertEqual(emitted["base"], 2)
        self.assertEqual(sum(emitted.values()), 2)
        self.assertIsNone(reasons["base"])
        self.assertEqual({reasons[d] for d in DEPTS}, {"project-not-lane-root"})
        for lane in lanes:
            self.tick(lane)
        control("P1 latent window, one more round: rows=%d" % self.rows(self.base_round))
        self.assertEqual(self.rows(self.base_round), 2)

    def test_dept_lane_before_the_export_gives_zero_rows_and_the_reason(self):
        self.handoff(self.base_round, "old")
        one = self.tick("dept-7")
        self.handoff(self.base_round, "new")
        two = self.tick("dept-7")
        rec = two["sweep"]["suppressed"]
        state = os.path.dirname(two["heartbeat"])
        control("P1 dept lane before export: PROJECT is base root=%s own_root is pack=%s rows=%d reason=%s "
                "count %d then %d cursor written=%s" % (
                    two["project"] == self.home, two["own_root"] == self.pack("dept-7"),
                    self.rows(self.base_round), rec["reason"], one["sweep"]["suppressed"]["count"], rec["count"],
                    os.path.exists(os.path.join(state, "sweep.json"))))
        self.assertEqual(two["project"], self.home)
        self.assertEqual(two["own_root"], self.pack("dept-7"))
        self.assertEqual(self.rows(self.base_round), 0)
        self.assertEqual(two["sweep"]["emitted"], 0)
        self.assertEqual(rec["reason"], "project-not-lane-root")
        self.assertEqual((one["sweep"]["suppressed"]["count"], rec["count"]), (1, 2))
        with open(os.path.join(state, "sweep_suppressed.json"), encoding="utf-8") as f:
            self.assertEqual(json.load(f)["count"], 2)
        self.assertFalse(os.path.exists(os.path.join(state, "sweep.json")))

    def test_the_reason_reaches_the_real_tick_output(self):
        stub_dir = os.path.join(self.home, "stub")
        os.makedirs(stub_dir)
        stub = os.path.join(stub_dir, "cys")
        with open(stub, "w", encoding="utf-8") as f:
            f.write('#!/bin/sh\ncase "$1" in status) echo \'{}\';; esac\nexit 0\n')
        os.chmod(stub, os.stat(stub).st_mode | stat.S_IXUSR)
        env = self.env("dept-7", PATH=stub_dir + ":/usr/bin:/bin")
        r = subprocess.run([PY, "-B", os.path.join(BIN, "javis_cycle_autopilot.py"), "tick"], capture_output=True,
                           text=True, cwd=self.home, env=env, timeout=60)
        self.assertEqual(r.returncode, 0, r.stderr)
        out = json.loads(r.stdout.strip().splitlines()[-1])
        control("P1 real tick in a dept lane before export: result=%s sweep reason=%s" % (
            out.get("result"), out["sweep"]["suppressed"]["reason"]))
        self.assertEqual(out["sweep"]["suppressed"]["reason"], "project-not-lane-root")

    def test_own_root_comes_from_lane_identity_never_from_project(self):
        self.lane_round("dept-3")
        self.lane_round("dept-4")
        foreign = self.tick("dept-3", root=self.pack("dept-4"))
        mine = self.tick("dept-3", exported=True)
        base = self.tick(None, root=self.pack("dept-4"))
        control("P1 dept-3 pointed at dept-4's root: project=%s suppressed=%s | at its own root: suppressed=%s | "
                "base at any root: suppressed=%s" % (
                    os.path.basename(foreign["project"]), "suppressed" in foreign["sweep"],
                    "suppressed" in mine["sweep"], "suppressed" in base["sweep"]))
        self.assertEqual(foreign["own_root"], self.pack("dept-3"))
        self.assertIn("suppressed", foreign["sweep"])
        self.assertNotIn("suppressed", mine["sweep"])
        self.assertNotIn("suppressed", base["sweep"])

    def test_first_sweep_after_the_export_seeds_without_a_flood(self):
        self.handoff(self.base_round, "old")
        self.tick(None)
        base_cursor = os.path.join(self.home, ".local", "state", "cys", "cycle_autopilot", "sweep.json")
        with open(base_cursor, "rb") as f:
            before = f.read()
        r = self.lane_round("dept-5")
        for i in range(13):
            self.handoff(r, "h%d" % i)
        for i in range(39):
            self.done(r, "t%d" % i)
        self.tick("dept-5")
        seed = self.tick("dept-5", exported=True)["sweep"]
        self.done(r, "real")
        real = self.tick("dept-5", exported=True)["sweep"]
        with open(base_cursor, "rb") as f:
            after = f.read()
        control("P1 first sweep after export, 39 tasks and 13 handoffs waiting: seeded=%s emitted=%d, then 1 real "
                "event emitted=%d, base cursor untouched=%s" % (
                    seed["seeded"], seed["emitted"], real["emitted"], before == after))
        self.assertTrue(seed["seeded"])
        self.assertEqual((seed["emitted"], real["emitted"], self.rows(r)), (0, 1, 1))
        self.assertEqual(before, after)


class Paths(Base):
    def probe(self, dept=None, **extra):
        r = subprocess.run([PY, "-B", "-c", PROBE, BIN], capture_output=True, text=True, cwd=self.home,
                           env=self.env(dept, **extra), timeout=60)
        self.assertEqual(r.returncode, 0, r.stderr)
        return json.loads(r.stdout.strip().splitlines()[-1])

    def expected(self, leaf):
        return os.path.join(self.home, ".local", "state", leaf, "cycle_autopilot")

    def test_base_path_is_unchanged(self):
        no_env = self.probe()["ap"][0]
        base_sock = self.probe(CYS_SOCKET=os.path.join(self.home, ".local", "state", "cys", "cys.sock"))["ap"][0]
        base_pipe = self.probe(CYS_SOCKET="\\\\.\\pipe\\cys")["ap"][0]
        control("P1 base path: no socket env, base socket, base pipe all equal the old path=%s" % (
            {no_env, base_sock, base_pipe} == {self.expected("cys")}))
        self.assertEqual({no_env, base_sock, base_pipe}, {self.expected("cys")})

    def test_dept_paths_and_every_constant_follow_the_lane(self):
        got = self.probe(dept="dept-5")
        d = self.expected("cys-dept-dept-5")
        control("P1 dept path: %s" % os.path.relpath(got["ap"][0], self.home))
        self.assertEqual(got["ap"], [d, d + "/state.json", d + "/verifier.heartbeat", d + "/baselines", d + "/sweep.json"])

    def test_windows_pipe_gives_the_same_lane_key(self):
        got = self.probe(CYS_SOCKET="\\\\.\\pipe\\cys-dept-dept-5")
        control("P1 dept pipe: %s" % os.path.relpath(got["ap"][0], self.home))
        self.assertEqual(got["ap"][0], self.expected("cys-dept-dept-5"))

    def test_aiterm_socket_keeps_its_precedence(self):
        got = self.probe(AITERM_SOCKET=self.sock("dept-2"), CYS_SOCKET=self.sock("dept-1"))
        self.assertEqual(got["ap"][0], self.expected("cys-dept-dept-2"))

    def test_autopilot_verifier_and_preflight_agree_inside_a_lane(self):
        got = self.probe(dept="dept-5")
        control("P1 same lane: verifier equals autopilot=%s, preflight heartbeat equals autopilot=%s" % (
            got["ap"][:4] == got["vf"], got["preflight_heartbeat"] == got["ap"][2]))
        self.assertEqual(got["ap"][:4], got["vf"])
        self.assertEqual(got["preflight_heartbeat"], got["ap"][2])
        other = self.probe(dept="dept-6")
        self.assertNotEqual(other["preflight_heartbeat"], got["preflight_heartbeat"])

    def test_mode_knob_is_per_lane_and_fails_toward_shadow(self):
        base_dir = self.expected("cys")
        os.makedirs(base_dir)
        with open(os.path.join(base_dir, "mode"), "w", encoding="utf-8") as f:
            f.write("live")
        dept_mode = self.probe(dept="dept-5")["mode"]
        base_mode = self.probe()["mode"]
        d = self.expected("cys-dept-dept-5")
        os.makedirs(d)
        with open(os.path.join(d, "mode"), "w", encoding="utf-8") as f:
            f.write("live")
        own = self.probe(dept="dept-5")["mode"]
        control("P1 mode file in base only: base=%s dept=%s, then in the dept's own dir: dept=%s" % (
            base_mode, dept_mode, own))
        self.assertEqual((base_mode, dept_mode, own), ("live", "shadow", "live"))


if __name__ == "__main__":
    unittest.main(verbosity=2)
