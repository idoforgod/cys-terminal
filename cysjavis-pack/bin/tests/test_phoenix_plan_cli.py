#!/usr/bin/env python3
"""0.14.56 phoenix `plan` — the command line contract the daemon parses (DESIGN r1 §4-1, §4-4; r2 §4-1).

Through the real `main()` in a child process, with a fake `cys` executable (PHOENIX_CYS) that answers the
identity probe and the status/list reads:
  · `plan` is a subcommand: exit 0 and exactly ONE `phoenix-plan/1 <JSON>` line among the log lines;
  · the identity contract is the restore's: a skewed CLI → exit 6 and no plan line; an inconclusive probe →
    accepted as `degraded-unverified` (reported in the JSON, never as `verified`);
  · `plan` writes no resolve journal (`_resolve_cys(record=False)`), while any other subcommand does
    (positive control on the same fake CLI);
  · an internal exception → one framed `outcome: "error"` line and exit 1.

Run: python3 cysjavis-pack/bin/tests/test_phoenix_plan_cli.py   (0 = all PASS). POSIX only (shebang fake CLI).
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import phoenix_plan_rig as rig  # noqa: E402

R = rig.Rig(prefix="phx-cli-")
check = rig.Checks()
SELF = rig.SELF_ID


def setup():
    return R.fake_cli()


def run(fake, args, world, extra_env=None):
    return R.run_cli(fake, args, world, extra_env)


def main():
    fake = setup()
    sock = os.path.join(R.state, "cys.sock")
    world = {"self": SELF, "daemon": dict(SELF), "surfaces": rig.LIVE_ALL}
    R.world(desired=rig.desired(rig.ROLES, rev=3), topo=rig.topo(rig.ROLES, rev=3), live=rig.LIVE_ALL)
    resolve_journal = os.path.join(R.phx, "journal-resolve.json")

    rc, framed, d, out, err, calls = run(fake, ["--socket", sock, "plan"], world)
    check("plan is a subcommand: exit 0", rc == 0, "rc=%r err=%s" % (rc, err[-200:]))
    check("exactly one framed line on stdout", len(framed) == 1, "framed=%d" % len(framed))
    check("framed JSON: protocol 1, outcome empty, identity verified (healthy world, identity match)",
          isinstance(d, dict) and d.get("phoenix_plan") == 1 and d.get("outcome") == "empty"
          and d.get("identity") == "verified", d)
    check("the framed line is one JSON object on one line (no embedded newline)",
          len(framed) == 1 and "\n" not in framed[0] and framed[0].endswith("}"), framed[:1])
    check("plan ran and wrote no resolve journal (record=False)", rc == 0 and not os.path.exists(resolve_journal))
    allowed = {("phoenix-identity",), ("status", "--json"), ("list",)}
    odd = [c for c in calls if tuple(c[:2]) not in allowed and tuple(c[:1]) not in allowed]
    check("plan made only identity/status/list calls on the CLI", calls and not odd, "odd=%r" % odd)

    rc2, framed2, _d, _o, _e, _c = run(fake, ["--socket", sock, "status"], world)
    check("positive control: a record=True subcommand (status) writes the resolve journal",
          os.path.exists(resolve_journal), "rc=%r" % rc2)
    os.remove(resolve_journal) if os.path.exists(resolve_journal) else None

    skew = dict(world, daemon=dict(SELF, build_id="b2"))
    rc, framed, d, out, err, calls = run(fake, ["--socket", sock, "plan"], skew)
    check("identity skew -> exit 6 and no plan line", rc == 6 and framed == [], "rc=%r framed=%d" % (rc, len(framed)))
    check("identity skew under plan (exit 6) writes no resolve journal either",
          rc == 6 and not os.path.exists(resolve_journal))

    inconclusive = dict(world, self=None)
    rc, framed, d, out, err, calls = run(fake, ["--socket", sock, "plan"], inconclusive)
    check("inconclusive identity -> exit 0, identity degraded-unverified (never claimed verified)",
          rc == 0 and isinstance(d, dict) and d.get("identity") == "degraded-unverified", "rc=%r d=%r" % (rc, d))
    check("degraded identity under plan (exit 0) writes no resolve journal",
          rc == 0 and not os.path.exists(resolve_journal))

    rc, framed, d, out, err, calls = run(fake, ["--socket", sock, "plan", "--bogus"], world)
    check("an unknown plan flag is an argparse exit 2 (the daemon reads it as no plan)", rc == 2 and framed == [],
          "rc=%r" % rc)

    # internal exception (in-process: nothing in a real world is guaranteed to raise)
    m = R.load()
    if hasattr(m, "_desired_view") and hasattr(m, "cmd_plan"):
        def _boom(*a, **k):
            raise RuntimeError("planted\nfailure")
        m._desired_view = _boom
    code, data, text = R.plan(m)
    check("internal exception -> exit 1 and one framed error line",
          code == 1 and isinstance(data, dict) and data.get("outcome") == "error"
          and "planted" in (data.get("error") or "") and text.count(rig.PREFIX) == 1, "code=%r data=%r" % (code, data))

    R.close()
    return check.finish()


if __name__ == "__main__":
    sys.exit(main())
