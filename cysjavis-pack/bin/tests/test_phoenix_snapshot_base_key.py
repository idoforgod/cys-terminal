#!/usr/bin/env python3
"""0.14.57 P4 — phoenix reads the base lane's roster out of a generation, by the source path in the manifest.

Until 0.14.57 `_snapshot_roster_entries` and `_snapshot_tombstones` opened a bare `topology.json`. do_snapshot stores
`<sha of source path>__topology.json` as soon as two lanes are present, so on a machine with lanes the lookup found
nothing, on every call. This file uses REAL generations: every one is written by javis_state_snapshot.do_snapshot.

KEY    the lookup itself: positive control (the count rises), wrong lane, corrupt newest generation still falls back.
VIEW   caller 1, `_desired_view`, end to end through `plan` and `run_restore` with a fake daemon.
RECOV  caller 2, `_recover_retention_file`, end to end through `run_restore`.

The fake `cys restore` is faithful on one point that matters here: it starts what topology.json lists, nothing else.

  python3 test_phoenix_snapshot_base_key.py                  this tree, exit 0 = all PASS
  python3 test_phoenix_snapshot_base_key.py --module <file>  the same checks on another javis_phoenix.py (a pin)
"""
import importlib.util
import json
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.dont_write_bytecode = True
import phoenix_plan_rig as R  # noqa: E402

SNAP = os.path.normpath(os.path.join(HERE, "..", "javis_state_snapshot.py"))
OLD_T, NEW_T = 1790000000, 1790021600        # two generations, 6 h apart


def load_snapshot_module():
    spec = importlib.util.spec_from_file_location("javis_state_snapshot_p4", SNAP)
    s = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(s)
    return s


class World:
    def __init__(self, rig, snap, m):
        self.rig, self.snap, self.m = rig, snap, m
        self.dept_dir = os.path.join(rig.home, ".local", "state", "cys-dept-dept-1")

    def reset(self, **kw):
        self.rig.world(**kw)

    def gen(self, at, base=None, dept=None, other_dept=None):
        """One real generation. The sources are written where lanes keep them, then do_snapshot takes them."""
        sources = []
        for path, value in ((os.path.join(self.rig.state, "topology.json"), base),
                            (os.path.join(self.dept_dir, "topology.json"), dept),
                            (os.path.join(self.dept_dir + "-b", "topology.json"), other_dept)):
            if value is not None:
                R._write(path, value)
                sources.append(path)
        os.environ["_CYS_SNAPSHOT_NOW"] = str(at)
        try:
            with open(os.devnull, "w") as devnull:
                saved, sys.stdout = sys.stdout, devnull
                try:
                    name = self.snap.do_snapshot(sources=sources, gen_root=self.rig.gen_root)
                finally:
                    sys.stdout = saved
        finally:
            os.environ.pop("_CYS_SNAPSHOT_NOW", None)
        assert name, "do_snapshot made no generation"
        return name

    def gen_path(self, name, *rest):
        return os.path.join(self.rig.gen_root, name, *rest)

    def stored_as(self, name, source):
        with open(self.gen_path(name, "manifest.json"), encoding="utf-8") as f:
            return [e["stored_as"] for e in json.load(f)["files"] if e["source"] == source][0]

    def base_source(self):
        return os.path.join(self.rig.state, "topology.json")

    def live_topology(self, value):
        path = self.base_source()
        if value is None:
            if os.path.exists(path):
                os.remove(path)
        else:
            R._write(path, value)

    def restore(self, ticket):
        """run_restore through the production branch, against the fake daemon below."""
        rig, m = self.rig, self.m
        rig.calls = []

        def fake(*args, socket=None, timeout=25):
            a = [str(x) for x in args]
            if a[:1] == ["restore"]:
                rig.calls.append(a)
                alive = set(s["role"] for s in rig.surfaces)
                path = self.base_source()
                topo = json.load(open(path, encoding="utf-8")) if os.path.exists(path) else {"entries": []}
                n = len(rig.surfaces)
                for e in topo.get("entries", []):
                    if e["role"] not in alive and e["role"] not in topo.get("tombstones", []):
                        n += 1
                        rig.surfaces.append(R.surf("surface:%d" % n, e["role"]))
                return R._R(0, "restored")
            if a[:1] == ["reinject"]:
                return R._R(0, "디렉티브 생존 확인 (ACK 수신)")
            if a[:1] and a[0] in ("read-screen", "watch"):
                return R._R(0, "bypass permissions on")
            return rig.fake_cys(*args, socket=socket, timeout=timeout)

        m.cys = fake
        try:
            return m.run_restore(None, ticket=ticket, stub=False, no_breaker=True, print_result=False)
        finally:
            m.cys = rig.fake_cys

    def restore_called(self):
        return any(c[:1] == ["restore"] for c in self.rig.calls)

    def desired_file(self):
        path = os.path.join(self.rig.phx, "desired_roster.json")
        if not os.path.exists(path):        # a CORRUPT run leaves the file isolated and writes no new one
            return {"roster": {}, "tombstones": []}
        with open(path, encoding="utf-8") as f:
            return json.load(f)


def key_checks(w, check):
    """The lookup, directly."""
    m, rig = w.m, w.rig
    w.reset()
    check("KEY-1a positive control: no generation, count 0", len(m._snapshot_roster_entries(None)) == 0)
    g = w.gen(NEW_T, base=R.topo(["master", "worker-1", "worker-2"], tombs=["worker-9"]),
              dept=R.topo(["dept-master"], tombs=["dept-gone"]), other_dept=R.topo(["other-master"]))
    names = sorted(os.listdir(w.gen_path(g)))
    tops = [n for n in names if n.endswith("topology.json")]
    check("KEY-1b the generation has the real layout: 3 prefixed topologies, no bare one",
          len(tops) == 3 and "topology.json" not in names and all("__" in n for n in tops), tops)
    got = m._snapshot_roster_entries(None)
    check("KEY-1c positive control: the planted roster is read, count rises 0 to 3",
          sorted(got) == ["master", "worker-1", "worker-2"], sorted(got))
    check("KEY-1d tombstones come from the same file", m._snapshot_tombstones(None) == {"worker-9"},
          m._snapshot_tombstones(None))
    check("KEY-2 wrong lane: no role of either department is picked",
          not set(got) & {"dept-master", "other-master"}, sorted(got))

    # only one lane's file in a generation: do_snapshot stores it BARE, and it is not the base lane's
    w.reset()
    g = w.gen(NEW_T, dept=R.topo(["dept-master"], tombs=["dept-gone"]))
    check("KEY-3a a lone department topology is stored bare", os.path.exists(w.gen_path(g, "topology.json")))
    check("KEY-3b wrong lane, bare name: roster not picked", m._snapshot_roster_entries(None) == {},
          sorted(m._snapshot_roster_entries(None)))
    check("KEY-3c wrong lane, bare name: tombstones not picked", m._snapshot_tombstones(None) is None,
          m._snapshot_tombstones(None))

    # a single-lane install: bare name, and it IS the base lane's
    w.reset()
    g = w.gen(NEW_T, base=R.topo(["master"]))
    check("KEY-4 single-lane install: bare name with a base source is found",
          os.path.exists(w.gen_path(g, "topology.json")) and sorted(m._snapshot_roster_entries(None)) == ["master"])

    # newest generation has no base lane
    w.reset()
    w.gen(OLD_T, base=R.topo(["master", "worker-1"]), dept=R.topo(["dept-master"]))
    w.live_topology(None)
    w.gen(NEW_T, dept=R.topo(["dept-master"]), other_dept=R.topo(["other-master"]))
    check("KEY-5 newest generation without the base lane: the older one answers",
          sorted(m._snapshot_roster_entries(None)) == ["master", "worker-1"],
          sorted(m._snapshot_roster_entries(None)))

    # corrupt newest generation, three ways
    for name, spoil in (("topology bytes", lambda g: R._write(
            w.gen_path(g, w.stored_as(g, w.base_source())), R.CORRUPT)),
            ("manifest bytes", lambda g: R._write(w.gen_path(g, "manifest.json"), "{ not json")),
            ("file named by the manifest is gone", lambda g: os.remove(w.gen_path(g, w.stored_as(g, w.base_source()))))):
        w.reset()
        w.gen(OLD_T, base=R.topo(["master", "older-role"], tombs=["older-tomb"]), dept=R.topo(["dept-master"]))
        g = w.gen(NEW_T, base=R.topo(["master", "newer-role"], tombs=["newer-tomb"]), dept=R.topo(["dept-master"]))
        spoil(g)
        skipped = []
        got = m._snapshot_roster_entries(None, skipped)
        check("KEY-6 corrupt newest (%s): falls back to the older generation" % name,
              sorted(got) == ["master", "older-role"], sorted(got))
        check("KEY-6 corrupt newest (%s): the generation is named as skipped" % name, skipped == [g], skipped)
        check("KEY-6 corrupt newest (%s): tombstones fall back too" % name,
              m._snapshot_tombstones(None) == {"older-tomb"}, m._snapshot_tombstones(None))

    # the source was spelled through a link: LIVE_STATE is a real path, the manifest keeps the spelling it was given
    w.reset()
    alias = os.path.join(rig.root, "home-alias")
    try:
        os.symlink(rig.home, alias)
    except (OSError, NotImplementedError):
        alias = None
    if alias:
        spelled = os.path.join(alias, os.path.relpath(w.base_source(), rig.home))
        other = os.path.join(w.dept_dir, "topology.json")
        R._write(w.base_source(), R.topo(["master", "worker-1"]))
        R._write(other, R.topo(["dept-master"]))
        os.environ["_CYS_SNAPSHOT_NOW"] = str(NEW_T)
        saved, sys.stdout = sys.stdout, open(os.devnull, "w")
        try:
            w.snap.do_snapshot(sources=[spelled, other], gen_root=rig.gen_root)
        finally:
            sys.stdout.close()
            sys.stdout = saved
            os.environ.pop("_CYS_SNAPSHOT_NOW", None)
        check("KEY-7 a source spelled through a link is still the base lane",
              sorted(m._snapshot_roster_entries(None)) == ["master", "worker-1"],
              sorted(m._snapshot_roster_entries(None)))

    # lanes other than base never read generations
    w.reset()
    w.gen(NEW_T, base=R.topo(["master"]), dept=R.topo(["dept-master"]))
    sock = os.path.join(w.dept_dir, "cys.sock")
    check("KEY-8 a department socket reads no generation", m._snapshot_roster_entries(sock) == {}
          and m._snapshot_tombstones(sock) is None)


def view_checks(w, check):
    """Caller 1: `_desired_view`, through plan (no writes) and through run_restore (persists, spawns)."""
    m, rig = w.m, w.rig
    # the case the snapshot exists for: topology eroded, desired never saw the role
    w.reset(live=R.LIVE_W2_GONE)
    w.gen(NEW_T, base=R.topo(R.ROLES), dept=R.topo(["dept-master"]))
    w.live_topology(R.topo(["master", "worker-1"]))
    rc, plan, _ = rig.plan(m)
    check("VIEW-1a plan: the role only the generation knows is a target", plan and plan["targets"] == ["worker-2"], plan and plan["targets"])
    check("VIEW-1b plan: the roster holds no department role",
          plan and plan["observed"]["view_roles"] == R.ROLES, plan and plan["observed"]["view_roles"])
    check("VIEW-1c plan wrote nothing", not os.path.exists(os.path.join(rig.phx, "desired_roster.json")))
    res = w.restore("view1")
    print("     measured: phoenix_restore=%s, exit %s, cys restore called=%s"
          % (res.get("phoenix_restore"), m.restore_exit_code(res), w.restore_called()))
    check("VIEW-1d restore persists the role into desired", "worker-2" in w.desired_file()["roster"])
    check("VIEW-1e restore calls cys restore for it", w.restore_called())
    check("VIEW-1f cys restore cannot start a role topology.json does not list: it stays dead and restore does not call it done",
          "worker-2" not in [s["role"] for s in rig.surfaces] and res.get("phoenix_restore") not in ("COMPLETE", "VERIFIED", "NOOP"),
          res.get("phoenix_restore"))

    # the merge is one way: once in desired the role stays after generations stop listing it
    w.gen(NEW_T + 21600, base=R.topo(["master", "worker-1"]), dept=R.topo(["dept-master"]))
    rc, plan, _ = rig.plan(m)
    check("VIEW-1g MEASURED: a newer generation without the role does not take it back out", plan and plan["targets"] == ["worker-2"], plan and plan["targets"])

    # closed after the snapshot: the daemon's tombstone wins
    w.reset(live=R.LIVE_W2_GONE)
    w.gen(NEW_T, base=R.topo(R.ROLES), dept=R.topo(["dept-master"]))
    w.live_topology(R.topo(["master", "worker-1"], tombs=["worker-2"], rev=2))
    rc, plan, _ = rig.plan(m)
    check("VIEW-2a role tombstoned after the snapshot is not a target", plan and plan["targets"] == [], plan and plan["targets"])
    check("VIEW-2b its entry is kept, tombstoned", plan and "worker-2" in plan["observed"]["view_roles"]
          and "worker-2" in plan["observed"]["view_tombstones"], plan and plan["observed"])
    w.restore("view2")
    check("VIEW-2c restore does not start it", not w.restore_called())

    # an ephemeral role in a generation stays out
    w.reset(live=R.LIVE_W2_GONE)
    w.gen(NEW_T, base=R.topo(["master", "worker-1", "worker-fresh-12"]), dept=R.topo(["dept-master"]))
    w.live_topology(R.topo(["master", "worker-1"]))
    rc, plan, _ = rig.plan(m)
    check("VIEW-3 ephemeral role from a generation is not merged",
          plan and plan["observed"]["view_roles"] == ["master", "worker-1"], plan and plan["observed"]["view_roles"])

    # corrupt newest generation, seen through plan
    w.reset(live=R.LIVE_W2_GONE)
    w.gen(OLD_T, base=R.topo(R.ROLES), dept=R.topo(["dept-master"]))
    g = w.gen(NEW_T, base=R.topo(R.ROLES), dept=R.topo(["dept-master"]))
    R._write(w.gen_path(g, w.stored_as(g, w.base_source())), R.CORRUPT)
    w.live_topology(R.topo(["master", "worker-1"]))
    rc, plan, _ = rig.plan(m)
    check("VIEW-4a corrupt newest: the older generation still gives the target", plan and plan["targets"] == ["worker-2"], plan and plan["targets"])
    check("VIEW-4b corrupt newest: plan names it as a reason",
          plan and ("topology_unreadable:generation:" + g) in plan["reasons"], plan and plan["reasons"])

    # the priority line in _desired_view: desired < generation < topology. Measured, not chosen here.
    w.reset(live=R.LIVE_W2_GONE)
    old = R.topo(R.ROLES)
    w.gen(OLD_T, base=old, dept=R.topo(["dept-master"]))
    w.live_topology(R.topo(["master", "worker-1"]))
    newer = R.desired(R.ROLES)
    newer["roster"]["worker-2"]["session_id"] = "sid-newer-than-the-generation"
    newer["roster"]["worker-2"]["cwd"] = "/tmp/newer"
    R._write(os.path.join(rig.phx, "desired_roster.json"), newer)
    w.restore("view5")
    pinned = w.desired_file()["roster"]["worker-2"].get("session_id")
    print("     measured: desired held sid-newer-than-the-generation, after restore it holds %s" % pinned)
    check("VIEW-5 MEASURED, see REPORT-P4-P5.md: for a role absent from topology the generation's older entry replaces desired's",
          pinned == "sid-worker-2", pinned)


def recover_checks(w, check):
    """Caller 2: `_recover_retention_file`, through run_restore. desired is corrupt and has no .bak."""
    m, rig = w.m, w.rig

    def corrupt_desired():
        R._write(os.path.join(rig.phx, "desired_roster.json"), "]]] corrupt, no bak")

    # no live topology: the provenance stays, nothing starts
    w.reset(live=[R.surf("surface:1", "master")])
    w.gen(NEW_T, base=R.topo(R.ROLES, tombs=["closed-role"]), dept=R.topo(["dept-master"], tombs=["dept-gone"]))
    w.live_topology(None)
    corrupt_desired()
    res = w.restore("rec1")
    d = w.desired_file()
    check("RECOV-1a corrupt desired, generation good, no topology: DEGRADED, not CORRUPT",
          res.get("phoenix_restore") == "DEGRADED" and m.restore_exit_code(res) == 3,
          "%s exit %s" % (res.get("phoenix_restore"), m.restore_exit_code(res)))
    check("RECOV-1b the roster comes back from the base lane only", sorted(d["roster"]) == sorted(R.ROLES), sorted(d["roster"]))
    check("RECOV-1c tombstones come back from the base lane only", d["tombstones"] == ["closed-role"], d["tombstones"])
    check("RECOV-1d provenance is kept: source snapshot", (d.get("recovered_from") or {}).get("source") == "snapshot", d.get("recovered_from"))
    check("RECOV-1e revival is held, nothing is started", not w.restore_called() and sorted(res.get("held_roles") or []) == ["worker-1", "worker-2"],
          res.get("held_roles"))
    res2 = w.restore("rec1b")
    check("RECOV-1f the hold survives the next automatic run", res2.get("phoenix_restore") == "DEGRADED" and not w.restore_called(),
          res2.get("phoenix_restore"))

    # a healthy topology next to it: the hold lifts in the same run, as the code says, so restore proceeds with no human
    w.reset(live=[R.surf("surface:1", "master")])
    w.gen(NEW_T, base=R.topo(R.ROLES), dept=R.topo(["dept-master"]))
    w.live_topology(R.topo(["master", "worker-1"], tombs=["worker-2"], rev=2))
    corrupt_desired()
    res = w.restore("rec2")
    d = w.desired_file()
    print("     measured: phoenix_restore=%s, exit %s, cys restore called=%s" % (res.get("phoenix_restore"), m.restore_exit_code(res), w.restore_called()))
    check("RECOV-2a healthy topology: the provenance is dropped in the same run", "recovered_from" not in d, d.get("recovered_from"))
    check("RECOV-2b healthy topology: restore proceeds", res.get("phoenix_restore") != "CORRUPT" and w.restore_called(), res.get("phoenix_restore"))
    check("RECOV-2c healthy topology: its tombstone beats the generation's roster", "worker-2" not in [s["role"] for s in rig.surfaces], [s["role"] for s in rig.surfaces])

    # only another lane on disk: no recovery from it
    w.reset(live=[R.surf("surface:1", "master")])
    w.gen(NEW_T, dept=R.topo(["dept-master"], tombs=["dept-gone"]))
    w.live_topology(None)
    corrupt_desired()
    res = w.restore("rec3")
    check("RECOV-3 only another lane's roster on disk: CORRUPT, exit 6, not healed from it",
          res.get("phoenix_restore") == "CORRUPT" and m.restore_exit_code(res) == 6, res.get("phoenix_restore"))

    # corrupt newest generation under recovery
    w.reset(live=[R.surf("surface:1", "master")])
    w.gen(OLD_T, base=R.topo(R.ROLES), dept=R.topo(["dept-master"]))
    g = w.gen(NEW_T, base=R.topo(["master"]), dept=R.topo(["dept-master"]))
    R._write(w.gen_path(g, w.stored_as(g, w.base_source())), R.CORRUPT)
    w.live_topology(None)
    corrupt_desired()
    res = w.restore("rec4")
    d = w.desired_file()
    check("RECOV-4 corrupt newest generation: recovery uses the older one",
          res.get("phoenix_restore") == "DEGRADED" and sorted(d["roster"]) == sorted(R.ROLES), sorted(d["roster"]))


def main(argv):
    target = R.PH
    if argv[:1] == ["--module"]:
        target = os.path.abspath(argv[1])
    rig = R.Rig(prefix="phx-p4-")
    check = R.Checks()
    try:
        m = rig.load(path=target, name="javis_phoenix_p4")
        m.time.sleep = lambda _s: None
        m.SPAWN_RETRIES, m.SPAWN_SETTLE, m.SPAWN_BACKOFF = 1, 0.0, 0.0
        w = World(rig, load_snapshot_module(), m)
        key_checks(w, check)
        view_checks(w, check)
        recover_checks(w, check)
    finally:
        rig.close()
    return check.finish()


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
