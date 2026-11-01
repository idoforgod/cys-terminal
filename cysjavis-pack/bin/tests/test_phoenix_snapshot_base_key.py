#!/usr/bin/env python3
"""0.14.57 P4 and P0 — phoenix reads a lane's own roster and tombstones out of a generation, by the source path in the manifest.

Until 0.14.57 `_snapshot_roster_entries` and `_snapshot_tombstones` opened a bare `topology.json`. do_snapshot stores
`<sha of source path>__topology.json` as soon as two lanes are present, so on a machine with lanes the lookup found
nothing, on every call. This file uses REAL generations: every one is written by javis_state_snapshot.do_snapshot.

KEY    the lookup itself: positive control (the count rises), wrong lane, corrupt newest generation still falls back.
VIEW   caller 1, `_desired_view`, end to end through `plan` and `run_restore` with a fake daemon. A role only a generation
       knows is named in snapshot_only_roles, not restored: by decision.
RECOV  caller 2, `_recover_retention_file`, end to end through `run_restore`.
DEPT   P0: the same two callers for a department lane, whose only protection from the other lanes' copies is the lookup.

The fake `cys restore` is faithful on one point that matters here: it starts what topology.json lists, nothing else.

  python3 test_phoenix_snapshot_base_key.py                  this tree, exit 0 = all PASS
  python3 test_phoenix_snapshot_base_key.py --module <file>  the same checks on another javis_phoenix.py (a pin)
"""
import hashlib
import importlib.util
import json
import os
import shutil
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
        for d in (self.dept_dir, self.dept_dir + "-b"):
            shutil.rmtree(d, ignore_errors=True)

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

    def restore(self, ticket, no_breaker=True, socket=None):
        """run_restore through the production branch, against the fake daemon below."""
        rig, m = self.rig, self.m
        rig.calls = []
        rig.assert_sealed(socket)

        def fake(*args, socket=None, timeout=25):
            a = [str(x) for x in args]
            if a[:1] == ["restore"]:
                rig.calls.append(a)
                alive = set(s["role"] for s in rig.surfaces)
                path = os.path.join(os.path.dirname(socket), "topology.json") if socket else self.base_source()
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
            return m.run_restore(socket, ticket=ticket, stub=False, no_breaker=no_breaker, print_result=False)
        finally:
            m.cys = rig.fake_cys

    def restore_called(self):
        return any(c[:1] == ["restore"] for c in self.rig.calls)

    def desired_file(self, socket=None):
        path = os.path.join(os.path.dirname(socket), "phoenix", "desired_roster.json") if socket \
            else os.path.join(self.rig.phx, "desired_roster.json")
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

    # every lane reads its own copy through the manifest, and never another lane's (P0: the root is one for all lanes)
    w.reset()
    sock = os.path.join(w.dept_dir, "cys.sock")
    other_sock = os.path.join(w.dept_dir + "-b", "cys.sock")
    check("KEY-8a positive control: nothing planted, a department reads 0, code generation_absent",
          m._snapshot_roster_entries(sock) == {} and m._snapshot_tombstones(sock) is None
          and m._LANE_READ["reason"] == "generation_absent", m._LANE_READ)
    w.gen(NEW_T, base=R.topo(["master"], tombs=["base-gone"]), dept=R.topo(["dept-master"], tombs=["dept-gone"]),
          other_dept=R.topo(["other-master"], tombs=["other-gone"]))
    check("KEY-8b positive control: the planted copy is read, count rises 0 to 1, its own role",
          sorted(m._snapshot_roster_entries(sock)) == ["dept-master"], sorted(m._snapshot_roster_entries(sock)))
    check("KEY-8c its own tombstones, the foreign ones stay out", m._snapshot_tombstones(sock) == {"dept-gone"},
          m._snapshot_tombstones(sock))
    check("KEY-8c2 a copy with tombstones reads as found", m._LANE_READ["reason"] == "found" and m._LANE_READ["passed_over"] == {}, m._LANE_READ)
    check("KEY-8d the other department gets its own", sorted(m._snapshot_roster_entries(other_sock)) == ["other-master"]
          and m._snapshot_tombstones(other_sock) == {"other-gone"}, m._snapshot_tombstones(other_sock))
    check("KEY-8e base gets its own", sorted(m._snapshot_roster_entries(None)) == ["master"]
          and m._snapshot_tombstones(None) == {"base-gone"}, m._snapshot_tombstones(None))
    absent = os.path.join(rig.home, ".local", "state", "cys-dept-dept-9", "cys.sock")
    check("KEY-8f positive control beside the refusal: the named lane still reads its known set",
          m._snapshot_tombstones(sock) == {"dept-gone"}, m._snapshot_tombstones(sock))
    check("KEY-8f2 a foreign lane reads nothing, code lane_absent", m._snapshot_roster_entries(absent) == {}
          and m._snapshot_tombstones(absent) is None and m._LANE_READ["reason"] == "lane_absent"
          and m._LANE_READ["passed_over"] == {"lane_absent": 1}, m._LANE_READ)
    # present and legitimately empty is not absent: the two never share a code
    w.reset()
    w.gen(NEW_T, dept=R.topo(["dept-master"]), other_dept=R.topo(["other-master"], tombs=["other-gone"]))
    check("KEY-8g a lane with a legitimately empty set reads as empty, code lane_empty", m._snapshot_tombstones(sock) == set()
          and m._LANE_READ["reason"] == "lane_empty", m._LANE_READ)
    # a generation with no manifest: nothing can say whose file it is, so nobody reads it, base included
    w.reset(snapshot=R.topo(["bare-role"], tombs=["bare-gone"]))
    os.remove(os.path.join(rig.gen_root, R.GEN, "manifest.json"))
    check("KEY-8h no manifest: the bare name is not read by a department, code manifest_unreadable",
          m._snapshot_tombstones(sock) is None and m._LANE_READ["reason"] == "manifest_unreadable", m._LANE_READ)
    check("KEY-8i no manifest: the bare name is not read by base either, code manifest_unreadable",
          m._snapshot_roster_entries(None) == {} and m._snapshot_tombstones(None) is None
          and m._LANE_READ["reason"] == "manifest_unreadable", m._LANE_READ)
    # the newest of two good generations answers
    w.reset()
    w.gen(OLD_T, dept=R.topo(["older-role"], tombs=["older-gone"]), base=R.topo(["master"]))
    w.gen(NEW_T, dept=R.topo(["newer-role"], tombs=["newer-gone"]), base=R.topo(["master"]))
    check("KEY-8j two good generations: the newest answers", m._snapshot_tombstones(sock) == {"newer-gone"}, m._snapshot_tombstones(sock))
    # manifest and stored file are damaged independently: each failure has its own code, and nothing falls back to a lane-less name
    dept_src = os.path.join(w.dept_dir, "topology.json")

    def spoiled(spoil):
        w.reset()
        w.gen(OLD_T, dept=R.topo(["older-role"], tombs=["older-gone"]), base=R.topo(["master"]))
        g = w.gen(NEW_T, dept=R.topo(["newer-role"], tombs=["newer-gone"]), base=R.topo(["master"]))
        spoil(g, w.gen_path(g, w.stored_as(g, dept_src)), w.gen_path(g, "manifest.json"))
        return g

    def edit(path, fn):
        with open(path, encoding="utf-8") as f:
            d = json.load(f)
        fn(d)
        R._write(path, d)

    def row_of(d):
        return [e for e in d["files"] if e["source"] == dept_src][0]

    def rewrite(copy, man, body):
        with open(copy, "w", encoding="utf-8") as f:
            f.write(body)
        raw = body.encode("utf-8")
        edit(man, lambda d: row_of(d).update(sha256=hashlib.sha256(raw).hexdigest(), size=len(raw)))

    cases = (
        ("copy bytes changed, same size", "manifest_integrity",
         lambda g, c, man: open(c, "r+b").write(open(c, "rb").read().replace(b"newer-gone", b"newer-g0ne"))),
        ("copy one byte longer", "manifest_integrity", lambda g, c, man: open(c, "ab").write(b" ")),
        ("row has no sha256", "manifest_integrity", lambda g, c, man: edit(man, lambda d: row_of(d).pop("sha256"))),
        ("row has no size", "manifest_integrity", lambda g, c, man: edit(man, lambda d: row_of(d).pop("size"))),
        ("stored file gone", "stored_missing", lambda g, c, man: os.remove(c)),
        ("row has no stored_as", "stored_missing", lambda g, c, man: edit(man, lambda d: row_of(d).pop("stored_as"))),
        ("bytes match the row but have no tombstones list", "copy_unreadable", lambda g, c, man: rewrite(c, man, '{"entries": []}')),
        ("manifest has no created_at", "manifest_undated", lambda g, c, man: edit(man, lambda d: d.pop("created_at"))),
        ("manifest carries another generation's date", "manifest_undated", lambda g, c, man: edit(man, lambda d: d.update(created_at=OLD_T))),
        ("manifest is torn", "manifest_unreadable", lambda g, c, man: open(man, "w").write("{ torn")),
        ("manifest is gone", "manifest_unreadable", lambda g, c, man: os.remove(man)),
    )
    for name, code, spoil in cases:
        g = spoiled(spoil)
        tombs = m._snapshot_tombstones(sock)
        check("KEY-10 %s: refused as %s, the older generation answers" % (name, code),
              tombs == {"older-gone"} and m._LANE_READ["passed_over"] == {code: 1} and m._LANE_READ["generation"] != g,
              "%s %s" % (tombs, m._LANE_READ))

    # with only the refused generation on disk the conclusion is that code, and the read is empty
    w.reset()
    g = w.gen(NEW_T, dept=R.topo(["newer-role"], tombs=["newer-gone"]), base=R.topo(["master"]))
    os.remove(w.gen_path(g, "manifest.json"))
    R._write(w.gen_path(g, "topology.json"), R.topo(["bare-role"], tombs=["bare-gone"]))
    check("KEY-10k manifest gone, a bare and a prefixed copy on disk: neither is read, code manifest_unreadable",
          m._snapshot_tombstones(sock) is None and m._LANE_READ["reason"] == "manifest_unreadable", m._LANE_READ)

    # a row re-pointed at another lane's file, with that file's true sha and size, is caught by the name's sha prefix
    w.reset()
    g = w.gen(NEW_T, dept=R.topo(["dept-role"], tombs=["dept-gone"]), other_dept=R.topo(["other-role"], tombs=["other-gone"]))
    foreign = w.gen_path(g, w.stored_as(g, os.path.join(w.dept_dir + "-b", "topology.json")))
    with open(foreign, "rb") as f:
        raw = f.read()
    edit(w.gen_path(g, "manifest.json"), lambda d: row_of(d).update(
        stored_as=os.path.basename(foreign), sha256=hashlib.sha256(raw).hexdigest(), size=len(raw)))
    check("KEY-10l a row pointing at another lane's file is refused, code lane_mismatch",
          m._snapshot_tombstones(sock) is None and m._LANE_READ["reason"] == "lane_mismatch", m._LANE_READ)

    # the binding, not the content: three cases. Every data check passes in the second one, only the comparison refuses it
    closed = ["closed-%02d" % i for i in range(29)]
    owner_src = os.path.join(w.dept_dir + "-b", "topology.json")
    w.reset()
    g = w.gen(NEW_T, other_dept=R.topo(["owner-role"], tombs=closed), base=R.topo(["master"]))
    check("KEY-11a binding, live == stored: the owner reads its 29", m._snapshot_tombstones(other_sock) == set(closed)
          and m._LANE_READ["reason"] == "found", m._LANE_READ)
    stored = w.gen_path(g, w.stored_as(g, owner_src))
    caller_name = hashlib.sha256(os.path.join(w.dept_dir, "topology.json").encode("utf-8")).hexdigest()[:8] + "__topology.json"
    os.rename(stored, w.gen_path(g, caller_name))
    edit(w.gen_path(g, "manifest.json"), lambda d: [e.update(stored_as=caller_name) for e in d["files"] if e["source"] == owner_src])
    os.makedirs(w.dept_dir, exist_ok=True)
    got = m._snapshot_tombstones(sock)
    check("KEY-11b binding, live != stored: the row under the caller's own name is refused, the 29 never come back, code lane_mismatch",
          got is None and m._LANE_READ["reason"] == "lane_mismatch", "%s %s" % (None if got is None else len(got), m._LANE_READ))

    # differential: the working directory is in lane A, the socket is in lane B. It resolves to B
    w.reset(live=R.LIVE_W2_GONE)
    w.gen(NEW_T, base=R.topo(["master"]), dept=R.topo(["lane-b-role"], tombs=["lane-b-gone"]),
          other_dept=R.topo(["lane-a-role"], tombs=["lane-a-gone"]))
    lane_a = w.dept_dir + "-b"
    os.remove(os.path.join(w.dept_dir, "topology.json"))      # only the generation copies can answer
    os.remove(os.path.join(lane_a, "topology.json"))
    fake = rig.fake_cli()
    world = {"self": R.SELF_ID, "daemon": dict(R.SELF_ID), "surfaces": []}
    rc, _f, plan, out, err, calls = rig.run_cli(fake, ["plan"], world, extra_env={"CYS_SOCKET": sock}, script=w.m.__file__, cwd=lane_a)
    names = (plan or {}).get("observed", {}).get("snapshot_only_roles")
    check("KEY-8l cwd in lane A, CYS_SOCKET in lane B: lane B's copy, not A's", rc == 0 and names == ["lane-b-role"], names)
    rc, _f, plan, out, err, calls = rig.run_cli(fake, ["plan"], world, extra_env={"CYS_SOCKET": other_sock}, script=w.m.__file__, cwd=w.dept_dir)
    names = (plan or {}).get("observed", {}).get("snapshot_only_roles")
    check("KEY-8m the same two lanes swapped: cwd in B, CYS_SOCKET in A reads A", rc == 0 and names == ["lane-a-role"], names)
    sockets_log = os.path.join(rig.root, "cys-calls.log.socket")
    if os.path.exists(sockets_log):      # KEY-9b reads this file for its own run
        os.remove(sockets_log)

    # the store this suite reads is the rig's: a marker that exists nowhere else comes back, and the reader's root is the rig's
    w.reset()
    marker = "rig-only-%d" % os.getpid()
    w.gen(NEW_T, dept=R.topo(["dept-role"], tombs=[marker]), base=R.topo(["master"]))
    check("KEY-13 the store read is the rig's: its root is under the rig and a marker planted there comes back",
          os.path.join(m.HOME, ".cys", "state-generations") == rig.gen_root and rig.gen_root.startswith(rig.root + os.sep)
          and m._snapshot_tombstones(sock) == {marker}, "%s %s" % (m.HOME, m._LANE_READ))

    # the phoenix reader takes the same boundary as the snapshot tool: no policy, nothing opened, a named and counted refusal
    w.reset()
    w.gen(NEW_T, dept=R.topo(["dept-role"], tombs=["dept-gone"]), base=R.topo(["master"]))
    snap = m._snap_mod()
    real_gate, n0 = snap._dataless_gate, snap.policy_refusals()
    snap._dataless_gate = lambda: False
    try:
        got = m._snapshot_tombstones(sock)
    finally:
        snap._dataless_gate = real_gate
    check("KEY-12 the policy cannot be set: nothing is read, code dataless_policy_unset, counted",
          got is None and m._LANE_READ["passed_over"] == {"dataless_policy_unset": 1} and snap.policy_refusals() == n0 + 1,
          "%s %s" % (got, m._LANE_READ))

    # the same department reached implicitly: no --socket, only the environment a department seat exports
    w.reset(live=R.LIVE_W2_GONE)
    w.gen(NEW_T, base=R.topo(["master", "base-only-role"]), dept=R.topo(["dept-master"]))
    w.live_topology(R.topo(["master"]))
    R._write(os.path.join(w.dept_dir, "topology.json"), R.topo(["dept-master"]))
    fake = rig.fake_cli()
    world = {"self": R.SELF_ID, "daemon": dict(R.SELF_ID), "surfaces": []}
    rc, _f, plan, out, err, calls = rig.run_cli(fake, ["plan"], world, extra_env={"CYS_SOCKET": sock}, script=w.m.__file__)
    sockets = open(os.path.join(rig.root, "cys-calls.log.socket"), encoding="utf-8").read().split() if os.path.exists(
        os.path.join(rig.root, "cys-calls.log.socket")) else []
    roles = (plan or {}).get("observed", {}).get("view_roles")
    check("KEY-9a implicit department environment: state and generation are the department's, not base", rc == 0 and roles == ["dept-master"],
          "rc %s roles %s" % (rc, roles))
    check("KEY-9b implicit department environment: every child command names that same socket", sockets and set(sockets) == {sock}, sockets)
    rc, _f, plan, out, err, calls = rig.run_cli(fake, ["plan"], world, script=w.m.__file__)
    roles = (plan or {}).get("observed", {}).get("view_roles")
    names = (plan or {}).get("observed", {}).get("snapshot_only_roles")
    check("KEY-9c control: with no environment socket it is the base lane", rc == 0 and names == ["base-only-role"], "%s %s" % (roles, names))


def view_checks(w, check):
    """Caller 1: `_desired_view`, through plan (no writes) and through run_restore (persists, spawns)."""
    m, rig = w.m, w.rig
    # A role only a generation knows. By decision it is reported, not restored: cys restore cannot start it, and as a target it
    # left FAILED in a log and opened the breaker on a second boot. The same run still has to name it.
    w.reset(live=R.LIVE_W2_GONE)
    w.gen(NEW_T, base=R.topo(R.ROLES), dept=R.topo(["dept-master"]))
    w.live_topology(R.topo(["master", "worker-1"]))
    rc, plan, _ = rig.plan(m)
    check("VIEW-1a plan: the role only a generation knows is not a target", plan and plan["targets"] == [] and plan["outcome"] == "empty",
          plan and (plan["targets"], plan["outcome"]))
    check("VIEW-1b plan: it is not in the roster, nor any department role", plan and plan["observed"]["view_roles"] == ["master", "worker-1"],
          plan and plan["observed"]["view_roles"])
    check("VIEW-1c plan wrote nothing", not os.path.exists(os.path.join(rig.phx, "desired_roster.json")))
    res = w.restore("view1")
    print("     measured: phoenix_restore=%s, exit %s, cys restore called=%s, snapshot_only_roles=%s"
          % (res.get("phoenix_restore"), m.restore_exit_code(res), w.restore_called(), res.get("snapshot_only_roles")))
    check("VIEW-1d restore does not put it into desired", "worker-2" not in w.desired_file()["roster"], sorted(w.desired_file()["roster"]))
    check("VIEW-1e restore is NOOP, exit 0, cys restore is not called", res.get("phoenix_restore") == "NOOP" and m.restore_exit_code(res) == 0
          and not w.restore_called(), res.get("phoenix_restore"))
    check("VIEW-1f CONTROL the name appears: plan and restore both carry it in snapshot_only_roles",
          plan and plan["observed"].get("snapshot_only_roles") == ["worker-2"] and res.get("snapshot_only_roles") == ["worker-2"],
          "%s %s" % (plan and plan["observed"].get("snapshot_only_roles"), res.get("snapshot_only_roles")))
    w.gen(NEW_T + 21600, base=R.topo(["master", "worker-1"]), dept=R.topo(["dept-master"]))
    rc, plan, _ = rig.plan(m)
    check("VIEW-1g a newer generation without the role: the name is gone too", plan and plan["observed"].get("snapshot_only_roles") == [] and plan["targets"] == [],
          plan and plan["observed"].get("snapshot_only_roles"))

    # closed after the snapshot: the daemon's tombstone wins
    w.reset(live=R.LIVE_W2_GONE)
    w.gen(NEW_T, base=R.topo(R.ROLES), dept=R.topo(["dept-master"]))
    w.live_topology(R.topo(["master", "worker-1"], tombs=["worker-2"], rev=2))
    rc, plan, _ = rig.plan(m)
    check("VIEW-2a role tombstoned after the snapshot is not a target", plan and plan["targets"] == [], plan and plan["targets"])
    check("VIEW-2b it is tombstoned, and a tombstoned role is not named as snapshot-only", plan and "worker-2" in plan["observed"]["view_tombstones"]
          and plan["observed"].get("snapshot_only_roles") == [], plan and plan["observed"])
    w.restore("view2")
    check("VIEW-2c restore does not start it", not w.restore_called())

    # an ephemeral role in a generation stays out
    w.reset(live=R.LIVE_W2_GONE)
    w.gen(NEW_T, base=R.topo(["master", "worker-1", "worker-fresh-12"]), dept=R.topo(["dept-master"]))
    w.live_topology(R.topo(["master", "worker-1"]))
    rc, plan, _ = rig.plan(m)
    check("VIEW-3 ephemeral role from a generation is not merged and not named",
          plan and plan["observed"]["view_roles"] == ["master", "worker-1"] and plan["observed"].get("snapshot_only_roles") == [], plan and plan["observed"]["view_roles"])

    # corrupt newest generation, seen through plan
    w.reset(live=R.LIVE_W2_GONE)
    w.gen(OLD_T, base=R.topo(R.ROLES), dept=R.topo(["dept-master"]))
    g = w.gen(NEW_T, base=R.topo(R.ROLES), dept=R.topo(["dept-master"]))
    R._write(w.gen_path(g, w.stored_as(g, w.base_source())), R.CORRUPT)
    w.live_topology(R.topo(["master", "worker-1"]))
    rc, plan, _ = rig.plan(m)
    check("VIEW-4a corrupt newest: the older generation still answers, as a name", plan and plan["targets"] == []
          and plan["observed"].get("snapshot_only_roles") == ["worker-2"], plan and plan["observed"].get("snapshot_only_roles"))
    check("VIEW-4b corrupt newest: plan names it as a reason",
          plan and ("topology_unreadable:generation:" + g) in plan["reasons"], plan and plan["reasons"])

    # the priority line in _desired_view: generation < desired < topology. An older generation must not take back a newer pin.
    w.reset(live=R.LIVE_W2_GONE)
    old = R.topo(R.ROLES)
    w.gen(OLD_T, base=old, dept=R.topo(["dept-master"]))
    w.live_topology(R.topo(["master", "worker-1"]))
    newer = R.desired(R.ROLES)
    newer["roster"]["worker-2"]["session_id"] = "sid-newer-than-the-generation"
    newer["roster"]["worker-2"]["cwd"] = "/tmp/newer"
    R._write(os.path.join(rig.phx, "desired_roster.json"), newer)
    res = w.restore("view5")
    pinned = w.desired_file()["roster"]["worker-2"].get("session_id")
    check("VIEW-5a a role absent from topology keeps desired's newer entry, the generation does not take it back",
          pinned == "sid-newer-than-the-generation", pinned)
    check("VIEW-5b its other fields stay too", w.desired_file()["roster"]["worker-2"].get("cwd") == "/tmp/newer")
    check("VIEW-5c a role desired knows stays a target as at the tag, it is not snapshot-only", res.get("snapshot_only_roles") == [] and w.restore_called(), res.get("snapshot_only_roles"))
    # the same generation against a desired that does not know the role: named, not added
    newer = R.desired(["master", "worker-1"])
    R._write(os.path.join(rig.phx, "desired_roster.json"), newer)
    rc, plan, _ = rig.plan(m)
    check("VIEW-5d a role absent from desired is named, not added", plan and plan["targets"] == [] and "worker-2" not in plan["observed"]["view_roles"]
          and plan["observed"].get("snapshot_only_roles") == ["worker-2"], plan and plan["observed"]["view_roles"])


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

    # a healthy topology next to it: a snapshot hold does not lift by itself, the human gate stays. Only --rebase lifts it.
    w.reset(live=[R.surf("surface:1", "master")])
    w.gen(NEW_T, base=R.topo(R.ROLES), dept=R.topo(["dept-master"]))
    w.live_topology(R.topo(["master", "worker-1"], tombs=["worker-2"], rev=2))
    corrupt_desired()
    res = w.restore("rec2")
    d = w.desired_file()
    print("     measured: phoenix_restore=%s, exit %s, cys restore called=%s" % (res.get("phoenix_restore"), m.restore_exit_code(res), w.restore_called()))
    check("RECOV-2a healthy topology: the snapshot provenance stays", (d.get("recovered_from") or {}).get("source") == "snapshot", d.get("recovered_from"))
    check("RECOV-2b healthy topology: restore does not proceed", res.get("phoenix_restore") == "DEGRADED" and not w.restore_called(),
          res.get("phoenix_restore"))
    res = w.restore("rec2b")
    check("RECOV-2c the next automatic run holds too", res.get("phoenix_restore") == "DEGRADED" and not w.restore_called(), res.get("phoenix_restore"))
    m.observe_and_persist_roster(None, rebase=True)
    d = w.desired_file()
    check("RECOV-2d operator --rebase lifts it", "recovered_from" not in d, d.get("recovered_from"))
    res = w.restore("rec2c")
    print("     measured after --rebase: phoenix_restore=%s, exit %s" % (res.get("phoenix_restore"), m.restore_exit_code(res)))
    check("RECOV-2e after --rebase restore proceeds and the daemon's tombstone beats the generation's roster",
          w.restore_called() and "worker-2" not in [s["role"] for s in rig.surfaces], [s["role"] for s in rig.surfaces])

    # a .bak hold is not a snapshot hold: it lifts on a healthy replace, as at the tag
    w.reset(live=[R.surf("surface:1", "master")])
    w.live_topology(R.topo(["master", "worker-1"], rev=2))
    corrupt_desired()
    R._write(os.path.join(rig.phx, "desired_roster.json.bak"), R.desired(["master", "worker-1"]))
    res = w.restore("rec5")
    d = w.desired_file()
    check("RECOV-5a control: a .bak hold lifts on a healthy replace, as at the tag", "recovered_from" not in d and w.restore_called(),
          "%s %s" % (res.get("phoenix_restore"), d.get("recovered_from")))
    check("RECOV-5b control: and restore proceeds", res.get("phoenix_restore") != "DEGRADED", res.get("phoenix_restore"))

    # the snapshot hold against known-unhealthy topologies: it survives each
    held = {"roster": R.desired(R.ROLES)["roster"], "tombstones": [], "tombstones_rev": 9, "daemon_epoch": "sa:1000",
            "updated_at": 1, "recovered_from": {"source": "snapshot", "ts": 1}}
    no_marker = R.topo(["master", "worker-1"])
    del no_marker["schema_version"]
    null_rev = R.topo(["master", "worker-1"])
    null_rev["tombstones_rev"] = None
    for name, topo in (("corrupt", R.CORRUPT), ("schema_version missing", no_marker),
                       ("tombstones_rev null", null_rev), ("rev regressed", R.topo(["master", "worker-1"], rev=2))):
        w.reset(live=[R.surf("surface:1", "master")])
        R._write(os.path.join(rig.phx, "desired_roster.json"), held)
        w.live_topology(topo)
        res = w.restore("rec6")
        d = w.desired_file()
        check("RECOV-6 hold survives a %s topology" % name, (d.get("recovered_from") or {}).get("source") == "snapshot"
              and res.get("phoenix_restore") == "DEGRADED" and m.restore_exit_code(res) == 3 and not w.restore_called(),
              "%s %s" % (res.get("phoenix_restore"), d.get("recovered_from")))
    # and the same healthy shape, rev not regressed: the case the samples are measured against
    w.reset(live=[R.surf("surface:1", "master")])
    R._write(os.path.join(rig.phx, "desired_roster.json"), held)
    w.live_topology(R.topo(["master", "worker-1"], rev=10))
    res = w.restore("rec6h")
    check("RECOV-6 hold survives a healthy topology too", (w.desired_file().get("recovered_from") or {}).get("source") == "snapshot"
          and not w.restore_called(), res.get("phoenix_restore"))

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


def dept_checks(w, check):
    """P0: corrupt desired on a department lane, end to end. Same two readers, as a pair, through `run_restore`."""
    m, rig = w.m, w.rig
    sock = os.path.join(w.dept_dir, "cys.sock")
    phx = os.path.join(w.dept_dir, "phoenix")
    live_topo = os.path.join(w.dept_dir, "topology.json")
    closed = ["closed-%02d" % i for i in range(29)]       # the exposed lane's real count

    def corrupt_desired():
        R._write(os.path.join(phx, "desired_roster.json"), "]]] corrupt, no bak")

    def world():
        w.reset(live=[R.surf("surface:1", "master")])
        w.gen(NEW_T, base=R.topo(["master"], tombs=["base-gone"]), dept=R.topo(R.ROLES, tombs=closed, rev=75),
              other_dept=R.topo(["other-master"], tombs=["other-gone"]))

    # no live topology: comes back from its own copy, held
    world()
    os.remove(live_topo)
    corrupt_desired()
    res = w.restore("drec1", socket=sock)
    d = w.desired_file(sock)
    print("     measured, department lane: phoenix_restore=%s, exit %s" % (res.get("phoenix_restore"), m.restore_exit_code(res)))
    check("DEPT-1a corrupt desired on a department lane: DEGRADED exit 3, not CORRUPT",
          res.get("phoenix_restore") == "DEGRADED" and m.restore_exit_code(res) == 3,
          "%s exit %s" % (res.get("phoenix_restore"), m.restore_exit_code(res)))
    check("DEPT-1b the roster is the lane's own", sorted(d["roster"]) == sorted(R.ROLES), sorted(d["roster"]))
    check("DEPT-1c all 29 tombstones come back, no foreign one", d["tombstones"] == closed, len(d["tombstones"]))
    check("DEPT-1d provenance: source snapshot", (d.get("recovered_from") or {}).get("source") == "snapshot", d.get("recovered_from"))
    check("DEPT-1e revival is held, nothing is started", not w.restore_called()
          and sorted(res.get("held_roles") or []) == ["worker-1", "worker-2"], res.get("held_roles"))
    res = w.restore("drec1b", socket=sock)
    check("DEPT-1f the hold survives the next automatic run", res.get("phoenix_restore") == "DEGRADED" and not w.restore_called(),
          res.get("phoenix_restore"))

    # a healthy topology beside it: the hold lifts only by --rebase, and base is not touched
    world()
    R._write(live_topo, R.topo(["master", "worker-1"], tombs=closed + ["worker-2"], rev=76))
    corrupt_desired()
    res = w.restore("drec2", socket=sock)
    d = w.desired_file(sock)
    check("DEPT-2a healthy topology: the snapshot provenance stays", (d.get("recovered_from") or {}).get("source") == "snapshot",
          d.get("recovered_from"))
    check("DEPT-2b healthy topology: nothing is started", res.get("phoenix_restore") == "DEGRADED" and not w.restore_called(),
          res.get("phoenix_restore"))
    res = w.restore("drec2b", socket=sock)
    check("DEPT-2c the next automatic run holds too", res.get("phoenix_restore") == "DEGRADED" and not w.restore_called(),
          res.get("phoenix_restore"))
    check("DEPT-2d base's own roster file was not written", not os.path.exists(os.path.join(rig.phx, "desired_roster.json")))
    m.observe_and_persist_roster(sock, rebase=True)
    d = w.desired_file(sock)
    check("DEPT-2e operator --rebase lifts it", "recovered_from" not in d, d.get("recovered_from"))
    res = w.restore("drec2c", socket=sock)
    print("     measured after --rebase, department lane: phoenix_restore=%s, exit %s" % (res.get("phoenix_restore"), m.restore_exit_code(res)))
    check("DEPT-2f after --rebase restore proceeds, a closed role stays closed", w.restore_called()
          and "worker-2" not in [s["role"] for s in rig.surfaces], [s["role"] for s in rig.surfaces])

    # only other lanes on disk: not healed from them
    w.reset(live=[R.surf("surface:1", "master")])
    w.gen(NEW_T, base=R.topo(["master"], tombs=["base-gone"]), other_dept=R.topo(["other-master"], tombs=["other-gone"]))
    os.makedirs(phx, exist_ok=True)
    corrupt_desired()
    res = w.restore("drec3", socket=sock)
    check("DEPT-3 only other lanes' copies on disk: CORRUPT exit 6, not healed from them, code lane_absent",
          res.get("phoenix_restore") == "CORRUPT" and m.restore_exit_code(res) == 6
          and (res.get("lane_read") or {}).get("reason") == "lane_absent", "%s %s" % (res.get("phoenix_restore"), res.get("lane_read")))

    # what the lane gains on a healthy day: caller 1 reports, it does not restore, and an unreadable copy is named
    world()
    R._write(live_topo, R.topo(["master"], tombs=closed))
    res = w.restore("drec5", socket=sock)
    check("DEPT-5a roles only the lane's generation knows are reported, not restored",
          (res.get("snapshot_only_roles") or []) == ["worker-1", "worker-2"] and not w.restore_called(),
          "%s %s" % (res.get("phoenix_restore"), res.get("snapshot_only_roles")))
    g = sorted(x for x in os.listdir(rig.gen_root) if x[:1].isdigit())[-1]
    R._write(w.gen_path(g, w.stored_as(g, live_topo)), R.CORRUPT)
    skipped = []
    check("DEPT-5b an unreadable copy of this lane is named as skipped", m._snapshot_roster_entries(sock, skipped) == {} and skipped == [g], skipped)

    # the dept_roster branch reads neither reader: its source is discovery, before and after P0
    world()
    R._write(m.dept_roster_path(sock), "]]] corrupt, no bak")
    calls = []
    saved = m._snapshot_roster_entries, m._snapshot_tombstones
    m._snapshot_roster_entries = lambda *a: calls.append("roster") or {}
    m._snapshot_tombstones = lambda *a: calls.append("tombstones")
    try:
        r = m._recover_retention_file(sock, m.dept_roster_path(sock), "dept_roster")
    finally:
        m._snapshot_roster_entries, m._snapshot_tombstones = saved
    with open(m.dept_roster_path(sock), encoding="utf-8") as f:
        dr = json.load(f)
    check("DEPT-4a dept_roster branch: degraded from discovery, neither reader called",
          r.get("status") == "degraded" and r.get("source") == "discovery" and calls == [], "%s %s" % (r, calls))
    check("DEPT-4b dept_roster branch: an empty roster and no tombstones, as at the tag", dr["roster"] == {} and dr["tombstones"] == [], dr)


def breaker_checks(w, check):
    """A role only a generation knows, driven through repeated boots with the real breaker: it must not count. The numbers that
    decided this are in REPORT-P4-R2.md: as a target it opened the breaker on the third attempt. One boot is two attempts at most."""
    m, rig = w.m, w.rig
    clock = {"t": 2000000000}
    real_now = m._now
    m._now = lambda: clock["t"]
    try:
        w.reset(live=[R.surf("surface:1", "master"), R.surf("surface:2", "worker-1")])
        w.gen(NEW_T, base=R.topo(R.ROLES), dept=R.topo(["dept-master"]))
        w.live_topology(R.topo(["master", "worker-1"]))
        verdicts = []
        for i in range(4):
            res = w.restore("brk%d" % i, no_breaker=False)
            verdicts.append((res.get("phoenix_restore"), m.restore_exit_code(res)))
            clock["t"] += 40
        attempts = []
        if os.path.exists(m.breaker_file(None)):
            with open(m.breaker_file(None), encoding="utf-8") as f:
                attempts = json.load(f).get("attempts", [])
        print("     measured 4 restores inside the window: %s, breaker attempts %d" % (verdicts, len(attempts)))
        check("BREAK-1 a generation-only role never counts: NOOP exit 0 each time, 0 attempts", verdicts == [("NOOP", 0)] * 4 and attempts == [], verdicts)
        # the breaker itself is untouched: a role topology lists still comes back, with the name reported beside it
        rig.surfaces = [s for s in rig.surfaces if s["role"] != "worker-1"]
        res = w.restore("brk5", no_breaker=False)
        back = "worker-1" in [s["role"] for s in rig.surfaces]
        print("     measured a real dead role beside it: %s exit %s, worker-1 back %s, snapshot_only_roles %s"
              % (res.get("phoenix_restore"), m.restore_exit_code(res), back, res.get("snapshot_only_roles")))
        check("BREAK-2 a role topology lists still comes back, and the name still appears", back and res.get("snapshot_only_roles") == ["worker-2"], res.get("phoenix_restore"))
    finally:
        m._now = real_now


def main(argv):
    target = R.PH
    if argv[:1] == ["--module"]:
        target = os.path.abspath(argv[1])
    rig = R.Rig(prefix="phx-p4-")
    check = R.Checks()
    try:
        m = rig.load(path=target, name="javis_phoenix_p4")
        if not hasattr(m, "_LANE_READ"):      # --module on a tree from before P0: the checks read red, they do not crash
            m._LANE_READ = {"reason": None, "passed_over": {}, "generation": None}
        m.time.sleep = lambda _s: None
        m.SPAWN_RETRIES, m.SPAWN_SETTLE, m.SPAWN_BACKOFF = 1, 0.0, 0.0
        w = World(rig, load_snapshot_module(), m)
        key_checks(w, check)
        view_checks(w, check)
        recover_checks(w, check)
        dept_checks(w, check)
        breaker_checks(w, check)
    finally:
        rig.close()
    return check.finish()


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
