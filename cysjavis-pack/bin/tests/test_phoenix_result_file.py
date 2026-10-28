#!/usr/bin/env python3
"""0.14.56 phoenix `restore` result file (DESIGN r1 §8-1/§8-2, r2 §7-1) + deploy contention unchanged (r1 §8-3).

exit 0 alone cannot tell VERIFIED/NOOP from LEASE_HELD (another restore holds `restore.run.lock` or
`restore.lease`, and phoenix stepped back without computing anything). When the daemon sets PHOENIX_RESULT_FILE,
`cmd_restore` leaves `{"phoenix_restore": <verdict>, "exit": <code>}` there; without the variable nothing is
written. A failed write must not change the exit code (the daemon reads "no result" and rechecks).

Children run the real `main()` with a fake `cys` (phoenix_plan_rig.py). POSIX only (fcntl lock, shebang CLI).

Run: python3 cysjavis-pack/bin/tests/test_phoenix_result_file.py   (0 = all PASS)
"""
import ast
import fcntl
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import phoenix_plan_rig as rig  # noqa: E402

R = rig.Rig(prefix="phx-result-")
check = rig.Checks()


def read_json(p):
    try:
        with open(p, encoding="utf-8") as f:
            return json.load(f)
    except (OSError, ValueError):
        return None


def main():
    fake = R.fake_cli()
    sock = os.path.join(R.state, "cys.sock")
    world = {"self": rig.SELF_ID, "daemon": dict(rig.SELF_ID), "surfaces": rig.LIVE_ALL}
    edir = os.path.join(R.root, "extract")
    os.makedirs(edir)
    res = os.path.join(edir, "result-1.json")

    # ── AT4: another restore holds restore.run.lock -> LEASE_HELD, exit 0, and the file says so ───────────────
    R.world(desired=rig.desired(rig.ROLES, rev=3), topo=rig.topo(rig.ROLES, rev=3), live=rig.LIVE_ALL)
    holder = open(os.path.join(R.phx, "restore.run.lock"), "a+")
    fcntl.flock(holder.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
    try:
        rc, _f, _d, out, err, calls = R.run_cli(fake, ["--socket", sock, "restore", "--auto"], world,
                                                {"PHOENIX_RESULT_FILE": res})
        got = read_json(res)
        check("held restore.run.lock -> exit 0", rc == 0, "rc=%r" % rc)
        check("held restore.run.lock -> result file {phoenix_restore: LEASE_HELD, exit: 0}",
              got == {"phoenix_restore": "LEASE_HELD", "exit": 0}, got)
        os.remove(res) if os.path.exists(res) else None
        rc, _f, _d, out, err, calls = R.run_cli(fake, ["--socket", sock, "restore", "--auto"], world)
        check("no PHOENIX_RESULT_FILE -> same exit 0 and nothing written in the extraction dir",
              rc == 0 and os.listdir(edir) == [], "rc=%r dir=%r" % (rc, os.listdir(edir)))
        with open(res, "w", encoding="utf-8") as f:
            f.write("stale bytes from an earlier attempt")
        rc, _f, _d, out, err, calls = R.run_cli(fake, ["--socket", sock, "restore", "--auto"], world,
                                                {"PHOENIX_RESULT_FILE": res})
        check("a stale file at the path is replaced by this attempt's verdict",
              read_json(res) == {"phoenix_restore": "LEASE_HELD", "exit": 0}, read_json(res))
        blocker = os.path.join(R.root, "not-a-dir")
        with open(blocker, "w", encoding="utf-8") as f:
            f.write("x")
        rc, _f, _d, out, err, calls = R.run_cli(fake, ["--socket", sock, "restore", "--auto"], world,
                                                {"PHOENIX_RESULT_FILE": os.path.join(blocker, "result-1.json")})
        check("an unwritable result path does not change the exit code (0) and is logged",
              rc == 0 and "PHOENIX_RESULT_FILE" in out, "rc=%r" % rc)
    finally:
        fcntl.flock(holder.fileno(), fcntl.LOCK_UN)
        holder.close()

    # ── the exit field mirrors the process exit: corrupt desired, no fallback -> CORRUPT, exit 6 ──────────────
    os.remove(res) if os.path.exists(res) else None
    R.world(desired=rig.CORRUPT, topo=rig.topo(rig.ROLES, rev=3), live=rig.LIVE_ALL)
    rc, _f, _d, out, err, calls = R.run_cli(fake, ["--socket", sock, "restore", "--auto"], world,
                                            {"PHOENIX_RESULT_FILE": res})
    got = read_json(res)
    check("unrecoverable corrupt desired -> exit 6 and result file {CORRUPT, exit 6}",
          rc == 6 and got == {"phoenix_restore": "CORRUPT", "exit": 6}, "rc=%r got=%r" % (rc, got))

    # ── deploy contention unchanged (r1 §8-3): LEASE_HELD -> code 3, two retries at 1.5 s and 3.0 s ────────────
    m = R.load()
    check("deploy_verdict_for(LEASE_HELD) == (LEASE_HELD, 3)", m.deploy_verdict_for("LEASE_HELD", None) == ("LEASE_HELD", 3),
          m.deploy_verdict_for("LEASE_HELD", None))
    check("deploy backoff base is 1.5 s (PHOENIX_SPAWN_BACKOFF default)", m.SPAWN_BACKOFF == 1.5, m.SPAWN_BACKOFF)
    tree = ast.parse(open(rig.PH, encoding="utf-8").read())
    deploy = next((n for n in tree.body if isinstance(n, ast.FunctionDef) and n.name == "cmd_deploy"), None)
    loops = []
    for n in ast.walk(deploy) if deploy else []:
        if isinstance(n, ast.While) and "LEASE_HELD" in ast.dump(n.test):
            src = ast.get_source_segment(open(rig.PH, encoding="utf-8").read(), n) or ""
            loops.append(src)
    pinned = (len(loops) == 1 and "_lease_retry < 2" in loops[0]
              and "time.sleep(SPAWN_BACKOFF * _lease_retry)" in loops[0])
    check("cmd_deploy keeps ONE LEASE_HELD retry loop: < 2 retries, sleep SPAWN_BACKOFF * n (1.5 s, 3.0 s)",
          pinned, "loops=%d" % len(loops))

    R.close()
    return check.finish()


if __name__ == "__main__":
    sys.exit(main())
