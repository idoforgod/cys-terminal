#!/usr/bin/env python3
"""0.14.56 phoenix `plan` — the plan's computation IS the restore's computation (DESIGN r1 §6, r2 §4-2, §9 P7-id).

(a) Differential: in each world the plan runs on the pristine world, then the world is rebuilt and the real
    restore runs up to its target line (the restore's own `_compute_targets` call is observed and the run is
    stopped there). Separable worlds: equal targets. C6 world (a role known only through an exited surface,
    CENSUS W2): plan ⊇ restore (the safe direction). Worlds where an omitted effect could SHRINK the restore's
    set (W3 pending intents, W4 corrupt retention) or liveness is unknown (W9): the plan is never `empty`.
(b) One path: `_compute_targets` replaced by a recorder returning a PLANTED set — the restore hands exactly
    that set to spawn, and the plan reports exactly it. Mutation control inside the specimen: a module copy with
    the old inline target list back in `_run_restore_locked` must make (b) fail.
(P7-id) `_snapshot_roster_entries(socket, skipped)` returns the same value as `(socket)`; the list only names
    generations that failed to parse.

Run: python3 cysjavis-pack/bin/tests/test_phoenix_plan_identity.py   (0 = all PASS)
"""
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import phoenix_plan_rig as rig  # noqa: E402

R = rig.Rig(prefix="phx-identity-")
check = rig.Checks()


class Stop(Exception):
    pass


def restore_targets(m, world):
    """Rebuild the world, run the real restore, return the target list its _compute_targets call produced."""
    R.world(**world["files"])
    R.reap_outcome = world.get("reap", "deferred")
    if world.get("liveness_unknown"):
        R.status_ok, R.list_out = False, "surface:1 not the list format\n"
    orig = getattr(m, "_compute_targets", None)
    seen = []
    if orig is None:
        return None

    def rec(*a, **k):
        seen.append(orig(*a, **k)[2])
        raise Stop()

    def tripwire(*a, **k):            # a restore that bypasses _compute_targets must not reach a real spawn
        raise Stop()

    orig_sp = m.spawn_production
    m._compute_targets, m.spawn_production = rec, tripwire
    try:
        m.run_restore(None, ticket="default", stub=False, include_master=True, print_result=False)
    except Stop:
        pass
    finally:
        m._compute_targets, m.spawn_production = orig, orig_sp
    return seen[0] if seen else None


def plan_of(m, world):
    R.world(**world["files"])
    R.reap_outcome = world.get("reap", "deferred")
    if world.get("liveness_unknown"):
        R.status_ok, R.list_out = False, "surface:1 not the list format\n"
    code, d, _ = R.plan(m)
    return code, d or {}


T3, T3m = rig.topo(rig.ROLES, rev=3), rig.topo(rig.ROLES, marker=False)
D3 = rig.desired(rig.ROLES, rev=3)
W2_LIVE = rig.LIVE_ALL + [rig.surf("surface:9", "worker-9", exited=True)]
WORLDS = {
    # separable (CENSUS: locks, journals, persist, .bak do not move the target set)
    "R0_control": ("equal", dict(desired=D3, topo=T3, live=rig.LIVE_W2_GONE)),
    "R0_all_alive": ("equal", dict(desired=D3, topo=T3, live=rig.LIVE_ALL)),
    "R1_fresh_topology_corrupt": ("equal", dict(desired=None, topo=rig.CORRUPT, live=rig.LIVE_W2_GONE)),
    "R2c_stale_tomb_healthy": ("equal", dict(desired=rig.desired(rig.ROLES, tombs=["worker-2"], rev=5),
                                             topo=rig.topo(rig.ROLES, rev=6), live=rig.LIVE_W2_GONE)),
    "R3_rev_bump": ("equal", dict(desired=rig.desired(rig.ROLES, tombs=["worker-2"], rev=4), topo=T3,
                                  live=rig.LIVE_W2_GONE)),
    "R4_first_boot": ("equal", dict(desired=None, topo=T3, live=rig.LIVE_W2_GONE)),
    "R5_snapshot_corrupt": ("equal", dict(desired=None, topo=rig.topo(["master", "worker-1"], rev=3),
                                          snapshot=rig.CORRUPT, live=rig.LIVE_W2_GONE)),
    "R5c_snapshot_valid": ("equal", dict(desired=None, topo=rig.topo(["master", "worker-1"], rev=3),
                                         snapshot=T3, live=rig.LIVE_W2_GONE)),
    "R6_foreign_tag": ("equal", dict(desired=rig.desired(rig.ROLES, rev=3, extra={"state_dir_tag": "/elsewhere"}),
                                     topo=T3, live=rig.LIVE_W2_GONE)),
    "R7_topology_missing_tomb": ("equal", dict(desired=rig.desired(rig.ROLES, tombs=["worker-2"], rev=3),
                                               topo=None, live=rig.LIVE_W2_GONE)),
    "R8pre_legacy_first_boot": ("equal", dict(desired=None, topo=T3m, live=rig.LIVE_W2_GONE)),
    "empty_seat": ("equal", dict(desired=D3, topo=T3, live=rig.LIVE_W2_GONE + [
        rig.surf("surface:3", "worker-2", seat="empty")])),
    # C6 (CENSUS W2 / W2b): worker-9 known only through an exited surface
    "W2_c6_reaped": ("superset", dict(desired=D3, topo=T3, live=W2_LIVE), "reaped"),
    "W2b_c6_deferred": ("equal", dict(desired=D3, topo=T3, live=W2_LIVE), "deferred"),
}


def main():
    m = R.load()
    for name, spec in WORLDS.items():
        kind, files = spec[0], spec[1]
        w = {"files": files, "reap": spec[2] if len(spec) > 2 else "deferred"}
        code, d = plan_of(m, w)
        p = d.get("targets")
        r = restore_targets(m, w)
        if kind == "equal":
            ok = p is not None and r is not None and sorted(p) == sorted(r)
        else:
            ok = p is not None and r is not None and set(p) >= set(r) and set(p) != set(r)
        check("(a) %s: plan %s restore" % (name, "==" if kind == "equal" else "strict superset of"), ok,
              "plan=%r restore=%r" % (p, r))

    # worlds where an omitted effect could shrink the restore's set, or liveness is unknown: never empty
    cases = {
        "W3_pending_intent": (dict(desired=D3, topo=T3, live=rig.LIVE_ALL,
                                   intents=json.dumps({"op": "add", "role": "worker-1", "ts": 1}) + "\n"),
                              "pending_tombstone_intents", False),
        "W4_corrupt_with_bak": (dict(desired=rig.CORRUPT, topo=T3, live=rig.LIVE_ALL,
                                     files={os.path.join("phoenix", "desired_roster.json.bak"):
                                            rig.desired(rig.ROLES + ["worker-3"], rev=3)}),
                                "retention_corrupt:desired_roster", False),
        "W9_liveness_unknown": (dict(desired=D3, topo=T3, live=rig.LIVE_ALL), "liveness_unknown", True),
    }
    for name, (files, reason, lu) in cases.items():
        code, d = plan_of(m, {"files": files, "liveness_unknown": lu})
        r = restore_targets(m, {"files": files, "liveness_unknown": lu})
        check("(a) %s: plan never empty, names %s" % (name, reason),
              d.get("outcome") in ("targets", "uncertified") and reason in (d.get("reasons") or []),
              "outcome=%r reasons=%r restore=%r" % (d.get("outcome"), d.get("reasons"), r))
    code, d = plan_of(m, {"files": cases["W4_corrupt_with_bak"][0]})
    r = restore_targets(m, {"files": cases["W4_corrupt_with_bak"][0]})
    check("(a) W4: the restore's recovery finds worker-3 the plan cannot see (why P3 must uncertify)",
          r is not None and "worker-3" in r and "worker-3" not in (d.get("targets") or []), "plan=%r restore=%r"
          % (d.get("targets"), r))

    # (b) one path
    def one_path(mod):
        planted = ["planted-role"]
        orig_ct, orig_sp = getattr(mod, "_compute_targets", None), mod.spawn_production
        spawned = []
        if orig_ct is None:
            return None, None

        def rec(entries, tombstones, live, roles=None):
            return list(planted), [], list(planted)

        def sp(socket, pending_roles, **k):
            spawned.append(list(pending_roles))
            raise Stop()

        mod._compute_targets, mod.spawn_production = rec, sp
        try:
            R.world(desired=D3, topo=T3, live=rig.LIVE_W2_GONE)
            code, d, _ = R.plan(mod)
            R.world(desired=D3, topo=T3, live=rig.LIVE_W2_GONE)
            try:
                mod.run_restore(None, ticket="default", stub=False, include_master=True, print_result=False)
            except Stop:
                pass
        finally:
            mod._compute_targets, mod.spawn_production = orig_ct, orig_sp
        return (d or {}).get("targets"), (spawned[0] if spawned else None)

    pt, st = one_path(m)
    check("(b) planted set: the plan reports exactly it", pt == ["planted-role"], pt)
    check("(b) planted set: the restore spawns exactly it", st == ["planted-role"], st)

    src = open(rig.PH, encoding="utf-8").read()
    anchor = "    _requested, _tomb_skipped, target_roles = _compute_targets(entries, _tombstones, live, roles)\n"
    inline = ("    _requested = roles if roles is not None else [r for r in entries if not _alive(r)]\n"
              "    _tomb_skipped = [r for r in _requested if r in _tombstones]\n"
              "    target_roles = [r for r in _requested if not _alive(r) and r not in _tombstones]\n")
    if src.count(anchor) == 1:
        mut = R.save("javis_phoenix_inline_targets.py", src.replace(anchor, inline, 1))
        mm = R.load(mut, name="javis_phoenix_inline_mutant")
        mpt, mst = one_path(mm)
        check("(b) mutation control: inline target list back in the restore -> the restore no longer spawns the "
              "planted set", mst is not None and mst != ["planted-role"], "plan=%r spawn=%r" % (mpt, mst))
    else:
        check("(b) mutation control anchor found exactly once", False, "count=%d" % src.count(anchor))

    # P7-id: the skipped side channel is inert
    gens = {"all_valid": [("20261027T000000Z", rig.topo(["a"])), ("20261028T000000Z", rig.topo(["b"]))],
            "newest_corrupt": [("20261027T000000Z", rig.topo(["a"])), ("20261028T000000Z", rig.CORRUPT)],
            "all_corrupt": [("20261027T000000Z", rig.CORRUPT), ("20261028T000000Z", rig.CORRUPT)],
            "none": []}
    expect_skipped = {"all_valid": [], "newest_corrupt": ["20261028T000000Z"],
                      "all_corrupt": ["20261028T000000Z", "20261027T000000Z"], "none": []}
    for label, layout in gens.items():
        R.world(desired=None, topo=T3, live=rig.LIVE_ALL)
        for g, body in layout:
            rig._write(os.path.join(R.gen_root, g, "topology.json"), body)
        a = m._snapshot_roster_entries(None)
        lst = []
        try:
            b = m._snapshot_roster_entries(None, lst)
        except TypeError as e:
            b = "TypeError: %s" % e
        check("P7-id %s: same return with and without the list; skipped=%r" % (label, expect_skipped[label]),
              a == b and lst == expect_skipped[label], "a=%r b=%r lst=%r" % (sorted(a), b if isinstance(b, str)
                                                                               else sorted(b), lst))

    R.close()
    return check.finish()


if __name__ == "__main__":
    sys.exit(main())
