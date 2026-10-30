#!/usr/bin/env python3
"""0.14.56 phoenix `plan` — certification (DESIGN-r2 §4-3 P1–P8 + the gemini1 r2 fold on P6).

The daemon skips the embedded restore only on a certified `empty` plan, so every condition that can make the
plan's view smaller than the restore's, or that phoenix itself reads as damaged, must turn `empty` into
`uncertified` with a named reason. Each block below builds the world in which exactly that condition is the
only thing standing between the plan and a false `empty`, and a positive control shows `empty` is reachable.
No daemon: a throw-away HOME and a fake `cys()` (phoenix_plan_rig.py).

Run: python3 cysjavis-pack/bin/tests/test_phoenix_plan_certify.py   (0 = all PASS)
"""
import json
import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import phoenix_plan_rig as rig  # noqa: E402

R = rig.Rig(prefix="phx-certify-")
m = R.load()
check = rig.Checks()


def plan(compare=None):
    code, data, text = R.plan(m, compare)
    return code, (data or {}), text


def reasons_of(d):
    return d.get("reasons") or []


def pre_plan_file(name, **world):
    R.world(**world)
    _code, _d, text = R.plan(m)
    return R.save(name, text)


def main():
    # ── controls: empty is reachable, and a dead role is a target ────────────────────────────────────────────
    R.world(desired=rig.desired(rig.ROLES, rev=3), topo=rig.topo(rig.ROLES, rev=3), live=rig.LIVE_ALL)
    code, d, text = plan()
    check("control: healthy world, all alive -> exit 0, outcome empty, no reason",
          code == 0 and d.get("outcome") == "empty" and reasons_of(d) == [] and d.get("targets") == [],
          "code=%r outcome=%r reasons=%r" % (code, d.get("outcome"), reasons_of(d)))
    check("control: exactly one framed line on stdout",
          text.count(rig.PREFIX) == 1, "framed=%d" % text.count(rig.PREFIX))
    check("control: identity field carries the resolved contract", d.get("identity") == "verified", d.get("identity"))
    R.world(desired=rig.desired(rig.ROLES, rev=3), topo=rig.topo(rig.ROLES, rev=3), live=rig.LIVE_W2_GONE)
    code, d, _ = plan()
    check("control: worker-2 dead -> outcome targets ['worker-2']",
          d.get("outcome") == "targets" and d.get("targets") == ["worker-2"], "%r %r" % (d.get("outcome"), d.get("targets")))

    # ── P2: a pending downtime tombstone intent (the omitted resync can shrink the target set, W3) ─────────────
    R.world(desired=rig.desired(rig.ROLES, rev=3), topo=rig.topo(rig.ROLES, rev=3), live=rig.LIVE_ALL,
            intents=json.dumps({"op": "add", "role": "ghost", "ts": 1}) + "\n")
    code, d, _ = plan()
    check("P2 pending intent -> uncertified(pending_tombstone_intents)",
          d.get("outcome") == "uncertified" and "pending_tombstone_intents" in reasons_of(d), reasons_of(d))

    # ── P3: retention files corrupt or carrying recovered_from (the omitted recovery can shrink it, W4) ────────
    R.world(desired=rig.CORRUPT, topo=rig.topo(rig.ROLES, rev=3), live=rig.LIVE_ALL)
    code, d, _ = plan()
    check("P3 corrupt desired_roster -> uncertified(retention_corrupt:desired_roster)",
          d.get("outcome") == "uncertified" and "retention_corrupt:desired_roster" in reasons_of(d), reasons_of(d))
    R.world(desired=rig.desired(rig.ROLES, rev=3, extra={"recovered_from": {"source": ".bak", "ts": 1}}),
            topo=rig.topo(rig.ROLES, rev=3), live=rig.LIVE_ALL)
    code, d, _ = plan()
    check("P3 valid desired_roster with recovered_from -> uncertified(retention_degraded:desired_roster)",
          d.get("outcome") == "uncertified" and "retention_degraded:desired_roster" in reasons_of(d), reasons_of(d))
    R.world(desired=rig.desired(rig.ROLES, rev=3), topo=rig.topo(rig.ROLES, rev=3), live=rig.LIVE_ALL,
            files={os.path.join("phoenix", "dept_roster.json"): rig.CORRUPT})
    code, d, _ = plan()
    check("P3 corrupt dept_roster -> uncertified(retention_corrupt:dept_roster)",
          d.get("outcome") == "uncertified" and "retention_corrupt:dept_roster" in reasons_of(d), reasons_of(d))

    # ── P4: liveness unknown (status --json fails, list output does not parse) ─────────────────────────────────
    R.world(desired=rig.desired(rig.ROLES, rev=3), topo=rig.topo(rig.ROLES, rev=3), live=rig.LIVE_ALL)
    R.status_ok, R.list_out = False, "surface:1 this line does not match the four-field list format\n"
    code, d, _ = plan()
    check("P4 liveness unknown -> reason liveness_unknown and never empty",
          "liveness_unknown" in reasons_of(d) and d.get("outcome") in ("targets", "uncertified")
          and d.get("observed", {}).get("live_known") is False, "%r %r" % (d.get("outcome"), reasons_of(d)))

    # ── P6-a: first boot, legacy topology; the hook CREATES desired with a tombstone (only P6 can refuse) ──────
    f = pre_plan_file("plan-pre-r8.out", desired=None, topo=rig.topo(rig.ROLES, marker=False), live=rig.LIVE_W2_GONE)
    R.world(desired=rig.desired(["master", "worker-1"], tombs=["worker-2"], rev=1),
            topo=rig.topo(rig.ROLES, marker=False), live=rig.LIVE_W2_GONE)
    code, d, _ = plan(compare=f)
    check("P6-a first boot + hook-created tombstone -> uncertified(tombstoned_across_hook_unproven:worker-2)",
          d.get("outcome") == "uncertified" and "tombstoned_across_hook_unproven:worker-2" in reasons_of(d),
          "%r %r" % (d.get("outcome"), reasons_of(d)))
    code2, d2, _ = plan()
    check("P6-a control: the same post-hook world WITHOUT --compare-plan certifies empty (P6 is the only refusal)",
          d2.get("outcome") == "empty" and reasons_of(d2) == [], "%r %r" % (d2.get("outcome"), reasons_of(d2)))

    # ── P6-b: topology missing (legacy add-merge); the hook adds worker-2 to desired tombstones ────────────────
    f = pre_plan_file("plan-pre-r7.out", desired=rig.desired(rig.ROLES, rev=3), topo=None, live=rig.LIVE_W2_GONE)
    R.world(desired=rig.desired(rig.ROLES, tombs=["worker-2"], rev=3), topo=None, live=rig.LIVE_W2_GONE)
    code, d, _ = plan(compare=f)
    check("P6-b legacy branch tombstone -> uncertified(tombstoned_across_hook_unproven:worker-2)",
          d.get("outcome") == "uncertified" and "tombstoned_across_hook_unproven:worker-2" in reasons_of(d)
          and d.get("observed", {}).get("replace") == "merged_legacy", "%r %r" % (d.get("outcome"), reasons_of(d)))
    # P6-b2: the only branch where `replace == "replaced"` is load-bearing. In R7 the daemon's tombstone set is
    #   empty (no topology.json), so membership alone already refuses. Here a marker-less topology.json itself lists
    #   worker-2 as a tombstone (a hook can write that file too): membership passes, the branch is merged_legacy,
    #   and only the replace check stops the false empty.
    f = pre_plan_file("plan-pre-r8b.out", desired=None, topo=rig.topo(rig.ROLES, marker=False), live=rig.LIVE_W2_GONE)
    R.world(desired=None, topo=rig.topo(rig.ROLES, tombs=["worker-2"], marker=False), live=rig.LIVE_W2_GONE)
    code, d, _ = plan(compare=f)
    check("P6-b2 marker-less topology lists the tombstone -> uncertified(tombstoned_across_hook_unproven:worker-2)",
          d.get("outcome") == "uncertified" and "tombstoned_across_hook_unproven:worker-2" in reasons_of(d)
          and d.get("observed", {}).get("replace") == "merged_legacy", "%r %r" % (d.get("outcome"), reasons_of(d)))
    # control: the DAEMON retired worker-2 (topology marker, rev up, worker-2 in its tombstones) -> proven
    f = pre_plan_file("plan-pre-proven.out", desired=rig.desired(rig.ROLES, rev=3), topo=rig.topo(rig.ROLES, rev=3),
                      live=rig.LIVE_W2_GONE)
    R.world(desired=rig.desired(rig.ROLES, rev=3), topo=rig.topo(rig.ROLES, tombs=["worker-2"], rev=4),
            live=rig.LIVE_W2_GONE)
    code, d, _ = plan(compare=f)
    check("P6-b control: a retirement the daemon recorded (replaced + in topo_tombs) certifies empty",
          d.get("outcome") == "empty" and reasons_of(d) == [] and d.get("observed", {}).get("replace") == "replaced",
          "%r %r" % (d.get("outcome"), reasons_of(d)))

    # ── P6 shrink: the hook deleted desired_roster.json; worker-3 lived only there ─────────────────────────────
    live3 = rig.LIVE_ALL
    f = pre_plan_file("plan-pre-shrink.out", desired=rig.desired(rig.ROLES + ["worker-3"], rev=3),
                      topo=rig.topo(rig.ROLES, rev=3), live=live3)
    with open(f, encoding="utf-8") as fh:
        pre = [json.loads(ln[len(rig.PREFIX):]) for ln in fh.read().split("\n") if ln.startswith(rig.PREFIX)]
    pre_targets = pre[0].get("targets") if len(pre) == 1 else None
    R.world(desired=None, topo=rig.topo(rig.ROLES, rev=3), live=live3)
    code, d, _ = plan(compare=f)
    check("P6 shrink: pre-hook targets ['worker-3'] (the baseline saw it)", pre_targets == ["worker-3"], pre_targets)
    check("P6 shrink: post-hook view lost worker-3 -> uncertified(roster_shrunk_across_hook:worker-3)",
          d.get("outcome") == "uncertified" and "roster_shrunk_across_hook:worker-3" in reasons_of(d),
          "%r %r" % (d.get("outcome"), reasons_of(d)))

    # ── P6-c: an unreadable or invalid baseline fails closed (it proves nothing) ───────────────────────────────
    R.world(desired=rig.desired(rig.ROLES, rev=3), topo=rig.topo(rig.ROLES, rev=3), live=rig.LIVE_ALL)
    good = {"phoenix_plan": 1, "outcome": "empty", "targets": [], "reasons": [], "identity": "verified",
            "observed": {"view_roles": rig.ROLES, "view_tombstones": []}}

    def framed(obj):
        return rig.PREFIX + json.dumps(obj) + "\n"

    variants = {
        "empty file": "",
        "no framed line": "[phoenix] log only\n",
        "two framed lines": framed(good) + framed(good),
        "invalid JSON": rig.PREFIX + "{not json\n",
        "protocol 2": framed(dict(good, phoenix_plan=2)),
        "protocol true": framed(dict(good, phoenix_plan=True)),
        "unknown outcome": framed(dict(good, outcome="error")),
        "view_roles not a list": framed(dict(good, observed={"view_roles": "master", "view_tombstones": []})),
        "view_tombstones missing": framed(dict(good, observed={"view_roles": rig.ROLES})),
        "reasons not a list": framed(dict(good, reasons="none")),
        "oversized": framed(good) + ("x" * (64 * 1024)) + "\n",
    }
    for label, body in variants.items():
        p = R.save("baseline-%s.out" % label.replace(" ", "-"), body)
        code, d, _ = plan(compare=p)
        check("P6-c baseline %s -> uncertified(compare_baseline_unreadable)" % label,
              d.get("outcome") == "uncertified" and "compare_baseline_unreadable" in reasons_of(d), reasons_of(d))
    code, d, _ = plan(compare=os.path.join(R.root, "artifacts", "does-not-exist.out"))
    check("P6-c baseline file missing -> uncertified(compare_baseline_unreadable)",
          d.get("outcome") == "uncertified" and "compare_baseline_unreadable" in reasons_of(d), reasons_of(d))
    p = R.save("baseline-good-crlf.out", "[phoenix] x\r\n" + framed(good).replace("\n", "\r\n"))
    code, d, _ = plan(compare=p)
    check("P6-c control: a valid baseline (CRLF lines, log noise) is accepted -> empty",
          d.get("outcome") == "empty" and reasons_of(d) == [], "%r %r" % (d.get("outcome"), reasons_of(d)))
    # S5 (CEO d34b1099 §4 ⒝): the daemon appends its W/X/U line to the pre-hook plan output before the post plan reads
    # it — that line is not a plan frame, so the baseline stays exactly one framed line and is still accepted.
    wall = 'cysd-hook-wall/1 {"wall_s":1800,"stall_s":90,"wall_from":"default","restore_outer_s":1300,' \
           '"restore_units":19,"restore_unit_s":69,"check":"ok"}\n'
    p = R.save("baseline-good-with-daemon-wall-line.out", "[phoenix] x\n" + framed(good) + wall)
    code, d, _ = plan(compare=p)
    check("P6-c control: a valid baseline followed by the daemon's cysd-hook-wall/1 line is accepted -> empty",
          d.get("outcome") == "empty" and reasons_of(d) == [], "%r %r" % (d.get("outcome"), reasons_of(d)))

    # ── P6-d (gemini1 r2 fold): a baseline that was itself uncertified is not a baseline ───────────────────────
    #    pre-hook: desired_roster corrupt (worker-3 recoverable only from it); the hook writes a clean file without
    #    worker-3. Post-hook P3 passes; only the baseline's own reason stops the false empty.
    f = pre_plan_file("plan-pre-corrupt.out", desired=rig.CORRUPT, topo=rig.topo(rig.ROLES, rev=3), live=rig.LIVE_ALL)
    R.world(desired=rig.desired(rig.ROLES, rev=3), topo=rig.topo(rig.ROLES, rev=3), live=rig.LIVE_ALL)
    code, d, _ = plan(compare=f)
    check("P6-d uncertified baseline -> uncertified(compare_baseline_uncertified:retention_corrupt:desired_roster)",
          d.get("outcome") == "uncertified"
          and "compare_baseline_uncertified:retention_corrupt:desired_roster" in reasons_of(d), reasons_of(d))
    code2, d2, _ = plan()
    check("P6-d control: the same post-hook world without --compare-plan is empty", d2.get("outcome") == "empty",
          "%r %r" % (d2.get("outcome"), reasons_of(d2)))

    # ── P7-s: topology.json unparsable ──────────────────────────────────────────────────────────────────────────
    R.world(desired=None, topo=rig.CORRUPT, live=rig.LIVE_W2_GONE)
    code, d, _ = plan()
    check("P7-s fresh + topology corrupt -> uncertified(topology_unreadable:state)",
          d.get("outcome") == "uncertified" and "topology_unreadable:state" in reasons_of(d)
          and d.get("observed", {}).get("replace") == "error_kept", "%r %r" % (d.get("outcome"), reasons_of(d)))
    R.world(desired=rig.desired(rig.ROLES, tombs=["worker-2"], rev=5), topo=rig.CORRUPT, live=rig.LIVE_W2_GONE)
    code, d, _ = plan()
    check("P7-s stale desired tombstone + topology corrupt -> uncertified(topology_unreadable:state)",
          d.get("outcome") == "uncertified" and "topology_unreadable:state" in reasons_of(d), reasons_of(d))

    # ── P7-g: the newest generation snapshot does not parse (the role lived only there) ────────────────────────
    R.world(desired=None, topo=rig.topo(["master", "worker-1"], rev=3), snapshot=rig.CORRUPT, live=rig.LIVE_W2_GONE)
    code, d, _ = plan()
    check("P7-g corrupt generation -> uncertified(topology_unreadable:generation:%s)" % rig.GEN,
          d.get("outcome") == "uncertified" and ("topology_unreadable:generation:" + rig.GEN) in reasons_of(d)
          and d.get("observed", {}).get("snapshot_skipped") == [rig.GEN], "%r %r" % (d.get("outcome"), reasons_of(d)))
    R.world(desired=None, topo=rig.topo(["master", "worker-1"], rev=3), snapshot=rig.topo(rig.ROLES, rev=3),
            live=rig.LIVE_W2_GONE)
    code, d, _ = plan()
    check("P7-g control: a valid generation is read (worker-2 known only there -> target)",
          d.get("targets") == ["worker-2"] and d.get("observed", {}).get("snapshot_skipped") == [], d.get("targets"))
    orig = m._snapshot_roster_entries
    m._snapshot_roster_entries = lambda s: {}
    try:
        R.world(desired=rig.desired(rig.ROLES, rev=3), topo=rig.topo(rig.ROLES, rev=3), live=rig.LIVE_ALL)
        code, d, _ = plan()
    finally:
        m._snapshot_roster_entries = orig
    check("P7 substituted generation reader -> uncertified(snapshot_reader_substituted)",
          d.get("outcome") == "uncertified" and "snapshot_reader_substituted" in reasons_of(d), reasons_of(d))

    # ── P8: unjustified tombstone rev regression (phoenix's own 「부분절단/조작 의심」 branch) ─────────────────
    R.world(desired=rig.desired(rig.ROLES, tombs=["worker-2"], rev=4), topo=rig.topo(rig.ROLES, rev=3),
            live=rig.LIVE_W2_GONE)
    code, d, _ = plan()
    check("P8 rev regression -> uncertified(tombstone_rev_regress)",
          d.get("outcome") == "uncertified" and "tombstone_rev_regress" in reasons_of(d)
          and d.get("observed", {}).get("replace") == "regress_kept", "%r %r" % (d.get("outcome"), reasons_of(d)))
    R.world(desired=rig.desired(rig.ROLES, tombs=["worker-2"], rev=3), topo=rig.topo(rig.ROLES, rev=3),
            live=rig.LIVE_W2_GONE)
    code, d, _ = plan()
    check("P8 control: same edit without the rev bump -> healthy replace, worker-2 a target",
          d.get("targets") == ["worker-2"] and d.get("observed", {}).get("replace") == "replaced", d.get("targets"))

    # ── C3 refusal inside a plan (structurally unreachable, r2 §4-3) still fails closed ────────────────────────
    orig = getattr(m, "_desired_view", None)

    def _c3(socket, rebase=False, intents=None, intents_truncated=False):
        roster, tombs, meta = orig(socket, rebase, intents, intents_truncated)
        meta["_persist"]["c3_refused"] = ["worker-9"]
        return roster, tombs, meta

    d = {}
    if orig is not None:
        m._desired_view = _c3
        try:
            R.world(desired=rig.desired(rig.ROLES, rev=3), topo=rig.topo(rig.ROLES, rev=3), live=rig.LIVE_ALL)
            code, d, _ = plan()
        finally:
            m._desired_view = orig
    check("C3 refusal -> uncertified(c3_refused:worker-9)",
          d.get("outcome") == "uncertified" and "c3_refused:worker-9" in reasons_of(d), reasons_of(d))

    # ── observed: the view, the branch, and the D4 / D4-prime budget inputs (derived from javis_budget) ────────
    R.world(desired=rig.desired(rig.ROLES, rev=3), topo=rig.topo(rig.ROLES, rev=3), live=rig.LIVE_W2_GONE)
    code, d, _ = plan()
    ob = d.get("observed", {})
    check("observed view_roles / view_tombstones / replace",
          ob.get("view_roles") == rig.ROLES and ob.get("view_tombstones") == [] and ob.get("replace") == "replaced", ob)
    b = m._budget_mod()
    units = ob.get("restore_units")
    if b is not None and isinstance(units, int):
        u = max(1, units)
        x = max(90, int(math.ceil(b.cys_restore_outer_s(u))))
        unit = max(90, int(math.ceil(b.cys_restore_outer_s(u + 1)))) - x
    else:
        x = unit = None
    check("observed restore_units = 1 (worker-2 dead, no seat)", units == 1, units)
    check("observed restore_outer_s / restore_unit_s come from javis_budget at call time",
          ob.get("restore_outer_s") == x and ob.get("restore_unit_s") == unit
          and ob.get("restore_budget_source") == "javis_budget" and x is not None,
          "outer=%r unit=%r src=%r expected %r/%r" % (ob.get("restore_outer_s"), ob.get("restore_unit_s"),
                                                     ob.get("restore_budget_source"), x, unit))
    saved = m._BUDGET_MOD
    m._BUDGET_MOD = False
    try:
        code, d, _ = plan()
    finally:
        m._BUDGET_MOD = saved
    ob = d.get("observed", {})
    check("budget module missing -> outer 90, unit null, source fallback (never a made-up unit)",
          ob.get("restore_outer_s") == 90 and ob.get("restore_unit_s") is None
          and ob.get("restore_budget_source") == "fallback", ob.get("restore_budget_source"))
    check("observed status_samples counts the liveness reads the plan made (2: view merge + target liveness)",
          d.get("observed", {}).get("status_samples") == 2, d.get("observed", {}).get("status_samples"))

    R.close()
    return check.finish()


if __name__ == "__main__":
    sys.exit(main())
