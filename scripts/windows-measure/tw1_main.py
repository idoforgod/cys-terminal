#!/usr/bin/env python3
"""TW1 measurement orchestrator (release 0.15.0 Windows gates on GitHub's hosted Windows runner; RULINGS CW7, EQ1-EQ6).

Commands:
  find-build --sha SHA [--wait-minutes N] [--out FILE]   (ubuntu job) the windows-build run at this commit
  run --work DIR --out DIR [--phase 1|2]                 stages S0..; writes tw1-record.json / tw1-record.md
  probe ID --work DIR --out DIR                          one probe in its own process (called by run)
  cleanup --work DIR --out DIR                           stops every process the run started (always())
  conclude --out DIR                                     red (exit 1) on MEASURED-FAIL or a failed set-up

Every probe runs in a child process with the time limit of matrix.json; on expiry its process tree is ended (the
long-lived processes the run registered are kept for the always() cleanup) and its rows are NOT-MEASURED.
TW2 (RULINGS LH6): a kill by a remembered pid acts only while the pid still names the process that was recorded
(creation time + image, tw1_probes.plan_tree_kill).
Verdicts are computed by tw1_record from the recorded facts only. Standard library only.
"""

import argparse
import json
import os
import subprocess
import sys
import time
import traceback

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import tw1_record as R  # noqa: E402


def _reconf_stdout():
    for s in (sys.stdout, sys.stderr):
        try:
            s.reconfigure(encoding="utf-8", errors="replace")
        except (AttributeError, ValueError):
            pass


def _load_json(p, default=None):
    try:
        with open(p, encoding="utf-8") as f:
            return json.load(f)
    except (OSError, ValueError):
        return default


def _dump_json(p, obj):
    os.makedirs(os.path.dirname(p) or ".", exist_ok=True)
    tmp = p + ".tmp"
    with open(tmp, "w", encoding="utf-8", newline="\n") as f:
        json.dump(obj, f, ensure_ascii=False, indent=1, default=str)
    os.replace(tmp, p)


# ───────────────────────────── find-build (ubuntu job) ─────────────────────────────

def cmd_find_build(a):
    import tw1_probes as P  # stdlib REST helper only; no Windows call on this path
    token = os.environ.get("GH_TOKEN", "")
    repo = os.environ.get("GITHUB_REPOSITORY", "")
    out = {"found": "false", "run_id": "", "conclusion": "", "status": "", "reason": ""}
    given = (os.environ.get("TW1_RUN_ID_INPUT") or "").strip()
    deadline = time.monotonic() + a.wait_minutes * 60
    try:
        while True:
            if given:
                run = P.api("/repos/%s/actions/runs/%s" % (repo, given), token)
                if run.get("head_sha") != a.sha:
                    out["reason"] = "the given run id built another commit"
                    break
            else:
                data = P.api("/repos/%s/actions/workflows/windows-build.yml/runs?head_sha=%s&per_page=30"
                             % (repo, a.sha), token)
                run = R.select_build_run(data.get("workflow_runs", []), a.sha)
            dec = R.build_wait_decision(run)
            if dec == "found":
                out.update(found="true", run_id=str(run["id"]), conclusion=str(run.get("conclusion") or ""),
                           status="completed", reason="")
                break
            out["status"] = (run or {}).get("status") or "absent"
            if time.monotonic() > deadline:
                out["reason"] = "windows-build at this commit not completed within %s min (%s)" % (a.wait_minutes, out["status"])
                break
            print("waiting for windows-build at this commit (status %s)" % out["status"], flush=True)
            time.sleep(60)
    except Exception as e:  # noqa: BLE001 - recorded; the measure job records the set-up failure
        out["reason"] = "lookup failed: %s" % e
    print(json.dumps(out), flush=True)
    if a.out:
        with open(a.out, "a", encoding="utf-8") as f:
            for k, v in out.items():
                f.write("%s=%s\n" % (k, str(v).replace("\n", " ")))
    return 0


# ───────────────────────────── probe (child) ─────────────────────────────

def cmd_probe(a):
    import tw1_probes as P
    ctx_path = os.path.join(a.work, "ctx.json")
    ctx = _load_json(ctx_path, {})
    pr = P.Probe(a.id, a.work, a.out, ctx)
    obs_path = os.path.join(a.work, "obs", a.id + ".json")
    rc = 0
    try:
        P.PROBES[a.id](pr)
        obs = {"status": "ran", "facts": pr.facts, "evidence": pr.evidence, "restored": pr.restored}
    except Exception as e:  # noqa: BLE001 - an exception is an observation: the probe did not run to its end
        rc = 3
        tb = traceback.format_exc()
        pr.ev("exception.txt", tb)
        obs = {"status": "error", "reason": "%s: %s" % (type(e).__name__, e), "facts": pr.facts,
               "evidence": pr.evidence, "restored": False}
    if P.KILL_REFUSED:  # TW2: a kill by a remembered pid that named another process by then (no verdict reads it)
        obs["kill_refused"] = list(P.KILL_REFUSED)
    if pr.ctx_updates:
        ctx = _load_json(ctx_path, {})
        for k, v in pr.ctx_updates.items():
            if isinstance(v, dict) and isinstance(ctx.get(k), dict):
                ctx[k].update(v)
            else:
                ctx[k] = v
        _dump_json(ctx_path, ctx)
    _dump_json(obs_path, obs)
    return rc


def _run_child(pid, a, limit_s):
    import tw1_probes as P
    obs_path = os.path.join(a.work, "obs", pid + ".json")
    if os.path.exists(obs_path):
        os.remove(obs_path)
    cmd = [sys.executable, "-X", "utf8", os.path.abspath(__file__), "probe", pid, "--work", a.work, "--out", a.out]
    log = open(os.path.join(a.out, "logs", pid + ".log"), "wb")
    kw = {"creationflags": P.CREATE_NO_WINDOW} if P.IS_WIN else {}
    t0 = time.monotonic()
    proc = subprocess.Popen(cmd, stdout=log, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL, **kw)
    state, _v, elapsed = R.wait_until(lambda: proc.poll(), time.monotonic, time.sleep, limit_s, step_s=1.0)
    if state == "timeout":
        keep = _registered(a.work)  # spared by identity (TW2): a stale registered number spares nobody else
        killed = P.kill_tree(proc.pid, keep=keep, expect=P.popen_identity(proc)) if P.IS_WIN else []
        try:
            proc.kill()
        except OSError:
            pass
        log.close()
        obs = _load_json(obs_path, {}) or {}
        return {"status": "timeout", "limit_s": limit_s, "facts": obs.get("facts", {}), "evidence": obs.get("evidence", []),
                "restored": False, "killed": killed, "secs": round(time.monotonic() - t0, 1)}
    log.close()
    obs = _load_json(obs_path)
    if obs is None:
        return {"status": "error", "reason": "the probe process ended (exit %s) without an observation" % proc.returncode,
                "facts": {}, "evidence": [], "restored": False, "secs": round(elapsed, 1)}
    obs["secs"] = round(elapsed, 1)
    obs["exit"] = proc.returncode
    return obs


def _registered(work):
    """{pid: registration} of every OPEN registration of the run (a 'stopped' line closes a pid)."""
    import tw1_probes as P
    return P.registry_read(os.path.join(work, "pids.jsonl"))


def stop_registered(work):
    """TW2 (RULINGS LH6): every open registration is stopped with its tree ONLY when its pid still names the process
    that was registered (creation time + image, compared in tw1_probes.plan_tree_kill; each kill then re-checks the
    creation time through its own handle). A pid that names another process now is NOT killed and is reported.
    -> {stopped_registered, not_killed, gone}"""
    import tw1_probes as P
    entries = _registered(work)
    rep = {"stopped_registered": [], "not_killed": [], "gone": []}
    for pid in sorted(entries):
        res = P.kill_tree_report(pid, expect=P.registered_identity(entries[pid]))
        item = {"pid": pid, "purpose": entries[pid].get("purpose"), "probe": entries[pid].get("probe")}
        if res["root"] == "ours":
            item["killed"] = res["killed"]
            rep["stopped_registered"].append(item)
        elif res["root"] == "gone":
            rep["gone"].append(item)
        else:
            item["why"] = ("pid reused, not ours" if res["root"] == "pid-reused"
                           else "identity unknown (none recorded, or the live process cannot be read) - not killed")
            item["registered"] = res["expected"]
            item["live"] = res["live"]
            rep["not_killed"].append(item)
    return rep


# ───────────────────────────── run (orchestrator) ─────────────────────────────

def _units(m, ctx):
    ml = ctx.get("merge_log") or {}
    beh = ctx.get("units_behaviour") or {}
    marked = set(m.get("unit_markers", {})) | set(m.get("unit_source_markers", {}))
    return {u: {"merge_log": ml.get(u), "behaviour": beh.get(u), "has_marker": u in marked,
                "source_marker": u in m.get("unit_source_markers", {})}
            for u in m["units"]}


def cmd_run(a):
    _reconf_stdout()
    m = R.load_matrix(os.path.join(HERE, "matrix.json"))
    os.makedirs(a.work, exist_ok=True)
    for d in ("obs", "logs", "evidence", "observations"):
        os.makedirs(os.path.join(a.out if d != "obs" else a.work, d), exist_ok=True)
    env = dict(os.environ)
    profile = env.get("USERPROFILE", "")
    ctx = {
        "redaction": R.redaction_map(env),
        "start_epoch": time.time(),
        "paths": {"install_dir": os.path.join(env.get("LOCALAPPDATA", ""), "cys"),
                  "pack_dir": os.path.join(profile, ".cys", "pack"),
                  "local_dir": os.path.join(profile, ".cys", "local")},
    }
    _dump_json(os.path.join(a.work, "ctx.json"), ctx)
    setup_probes = {s["probe"] for s in m.get("setup", [])}
    state = "clean"
    outcomes, decisions, timeline = {}, {}, []
    by_probe = {}
    for r, parent in R.iter_rows(m):
        if r.get("probe"):
            by_probe.setdefault(r["probe"], []).append((r, parent))
    for pid in m["order"]:
        meta = m["probes"][pid]
        ctx = _load_json(os.path.join(a.work, "ctx.json"), ctx)
        units = _units(m, ctx)
        rows = by_probe.get(pid, [])
        for r, parent in rows:
            decisions[r["id"]] = R.decide_row(r, m["probes"], state, a.phase, units)
        need_state = meta["requires_state"]
        state_ok = need_state == "any" or state == need_state
        wanted = pid in setup_probes or any(decisions[r["id"]][0] for r, _ in rows)
        if not wanted or not state_ok:
            reason = ("state %r required, found %r" % (need_state, state)) if not state_ok else "no row needs it"
            outcomes[pid] = {"status": "skipped", "reason": reason}
            timeline.append({"probe": pid, "status": "skipped", "reason": reason, "state": state})
            print("[tw1] %-14s skipped (%s)" % (pid, reason), flush=True)
            continue
        print("[tw1] %-14s start (limit %ss, state %s)" % (pid, meta["time_limit_s"], state), flush=True)
        oc = _run_child(pid, a, meta["time_limit_s"])
        outcomes[pid] = oc
        prev = state
        state = R.state_after(meta, oc.get("status"), oc.get("restored"), state)
        timeline.append({"probe": pid, "status": oc.get("status"), "reason": oc.get("reason"), "secs": oc.get("secs"),
                         "state_before": prev, "state_after": state})
        print("[tw1] %-14s %s in %ss -> state %s %s" % (pid, oc.get("status"), oc.get("secs"), state,
                                                     ("(" + str(oc.get("reason")) + ")") if oc.get("reason") else ""),
              flush=True)
    ctx = _load_json(os.path.join(a.work, "ctx.json"), ctx)
    mapping = [tuple(x) for x in ctx.get("redaction", [])]
    # ── verdicts ──
    row_results, sub_results = [], []
    for r in m["rows"]:
        sub_v = []
        for s in r.get("subrows", []) or []:
            dec = decisions.get(s["id"]) or R.decide_row(s, m["probes"], state, a.phase, _units(m, ctx))
            res = R.row_result(s, r["id"], dec, outcomes.get(s.get("probe")))
            sub_results.append(res)
            sub_v.append(res["verdict"])
        if r.get("probe"):
            dec = decisions.get(r["id"]) or R.decide_row(r, m["probes"], state, a.phase, _units(m, ctx))
            res = R.row_result(r, None, dec, outcomes.get(r["probe"]))
            if r.get("subrows"):
                own_v = res["verdict"]
                res["verdict"] = R.aggregate(r, own_v, sub_v)
                res["reason"] = R.aggregate_reason(own_v, res.get("reason"),
                                                   [(sr["id"], v) for sr, v in zip(r["subrows"], sub_v)], res["verdict"])
        elif r.get("subrows") and r.get("class") != "NO":
            res = R.row_result(r, None, (False, R.NM, None), None)
            res["verdict"] = R.aggregate(r, None, sub_v)
            res["reason"] = "worst of its sub-rows: " + ", ".join(
                "%s=%s" % (s["id"], v) for s, v in zip(r["subrows"], sub_v))
            res["evidence"] = sorted({e for sr in sub_results if sr.get("parent") == r["id"] for e in sr.get("evidence", [])})
        else:
            res = R.row_result(r, None, R.decide_row(r, m["probes"], state, a.phase, _units(m, ctx)), None)
        row_results.append(res)
    setup = R.evaluate_setup(m.get("setup", []), outcomes)
    environment = dict((outcomes.get("s0_env") or {}).get("facts") or {})
    environment["probe_timeline"] = timeline
    meta = {"commit": os.environ.get("GITHUB_SHA"), "phase": a.phase, "matrix": "scripts/windows-measure/matrix.json",
            "run_id": os.environ.get("GITHUB_RUN_ID"), "run_attempt": os.environ.get("GITHUB_RUN_ATTEMPT"),
            "finished_epoch": time.time(), "units": _units(m, ctx)}
    record = R.build_record(meta, environment, setup, row_results, sub_results)
    record = R.redact(record, mapping)
    for pid, oc in outcomes.items():
        _dump_json(os.path.join(a.out, "observations", pid + ".json"), R.redact(oc, mapping))
    _dump_json(os.path.join(a.out, "tw1-record.json"), record)
    md = R.render_markdown(record)
    with open(os.path.join(a.out, "tw1-record.md"), "w", encoding="utf-8", newline="\n") as f:
        f.write(md)
    summ = os.environ.get("GITHUB_STEP_SUMMARY")
    if summ:
        with open(summ, "a", encoding="utf-8") as f:
            f.write(md[:900_000] + "\n")
    print("[tw1] " + record["summary"]["line"], flush=True)
    for fd in record["findings"]:
        print("[tw1] FINDING %s/%s: %s" % (fd["row"], fd["check"], fd["observed"]), flush=True)
    return 0


# ───────────────────────────── cleanup / conclude ─────────────────────────────

def cmd_cleanup(a):
    _reconf_stdout()
    import tw1_probes as P
    ctx = _load_json(os.path.join(a.work, "ctx.json"), {}) or {}
    mapping = [tuple(x) for x in ctx.get("redaction", [])]
    report = {"stopped_registered": [], "not_killed": [], "gone": [], "stopped_by_path": [], "defender_restore": None}
    if P.IS_WIN:
        report.update(stop_registered(a.work))
        folders = [ctx.get("paths", {}).get("install_dir"), a.work]
        for fo in folders:
            if fo:
                for pid in P.pids_under(fo):
                    if P.terminate(pid):
                        report["stopped_by_path"].append(pid)
        orig = ctx.get("defender_original_rtp")
        if orig is False:
            report["defender_restore"] = P.powershell("Set-MpPreference -DisableRealtimeMonitoring $true")["code"]
        report["remaining_cys_processes"] = [p["name"] for p in P.processes()
                                             if (p.get("name") or "").lower() in ("cys.exe", "cysd.exe", "cys-app.exe")]
    # RULINGS EQ6 / HC6: outputs a child wrote straight to a file (daemons, holders) and the probes' logs - redacted
    # once every process of the run is stopped, before the artifact is uploaded
    report["redacted_files"] = sum(R.redact_tree(os.path.join(a.out, d), mapping) for d in ("evidence", "logs")
                                   if mapping and os.path.isdir(os.path.join(a.out, d)))
    _dump_json(os.path.join(a.out, "cleanup.json"), R.redact(report, mapping))
    print("[tw1] cleanup: %s" % json.dumps(R.redact(report, mapping)), flush=True)
    return 0


def cmd_conclude(a):
    _reconf_stdout()
    rec = _load_json(os.path.join(a.out, "tw1-record.json"))
    concl, why = R.job_conclusion(rec)
    if rec:
        print("[tw1] " + rec["summary"]["line"])
        print("[tw1] findings: %d (each needs a ruling before Windows assets are published - RULINGS EQ3)"
              % rec["summary"]["findings"])
    print("[tw1] job conclusion: %s %s" % (concl, "; ".join(why)))
    return 1 if concl == "red" else 0


def main(argv=None):
    ap = argparse.ArgumentParser(prog="tw1_main.py")
    sub = ap.add_subparsers(dest="cmd", required=True)
    fb = sub.add_parser("find-build")
    fb.add_argument("--sha", required=True)
    fb.add_argument("--wait-minutes", type=int, default=75)
    fb.add_argument("--out")
    for name in ("run", "cleanup"):
        p = sub.add_parser(name)
        p.add_argument("--work", required=True)
        p.add_argument("--out", required=True)
        if name == "run":
            p.add_argument("--phase", type=int, default=1, choices=(1, 2))
    p = sub.add_parser("probe")
    p.add_argument("id")
    p.add_argument("--work", required=True)
    p.add_argument("--out", required=True)
    c = sub.add_parser("conclude")
    c.add_argument("--out", required=True)
    a = ap.parse_args(argv)
    if a.cmd in ("run", "cleanup", "probe"):
        a.work = os.path.abspath(a.work)
        a.out = os.path.abspath(a.out)
        os.makedirs(a.out, exist_ok=True)
    return {"find-build": cmd_find_build, "run": cmd_run, "probe": cmd_probe, "cleanup": cmd_cleanup,
            "conclude": cmd_conclude}[a.cmd](a)


if __name__ == "__main__":
    sys.exit(main())
