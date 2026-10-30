#!/usr/bin/env python3
"""0.14.56 GATE 1 — the phoenix `plan` is side-effect-free (DESIGN r1 §10-1 / K3, r2 §9 G1-env + M8).

The daemon calls `plan` on every configured boot and before deciding to skip the embedded restore, so the plan
must write nothing and ask the daemon to do nothing. Measured, not trusted:
  · every file (sha256 + mode) and every directory under the rig HOME (the LIVE state dir incl. phoenix/,
    topology.json, the generation snapshots, ~/.cys/local), the private byte-identical subject pack, and bound roots (TMPDIR/TEMP/TMP,
    CYS_PACK_DIR, CYS_LOCAL_DIR, HUD_STATE_DIR, PHOENIX_DEPT_STATE_ROOT, PHOENIX_DEPTS_JSON) is hashed before and
    after EACH plan call — any difference fails;
  · the fake daemon CLI records every call; anything other than `phoenix-identity`, `status --json`, `list`
    fails (a tombstone resync, a C6 reap, an event emitted over the wire are all daemon mutations);
  · a probe child first proves the temp root really is under the hashed root (else the M8 check is void).
Worlds: healthy, dead role, C6 exited-only role, pending intent, corrupt desired + .bak, desired absent + .bak,
corrupt topology, rev regression, corrupt generation, topology missing, fresh install (no phoenix/), held
restore.run.lock, an ambiguous `fresh` role, a C3 world, and a post-hook `--compare-plan` call.
MUTATION CONTROL (inside the specimen): nine module copies, each reintroducing one effect at an exact anchor
(anchor missing = FAIL); each must be caught, the unmutated module must pass. Four separately gated negative
controls prove the recorder, real spool, containment and invalid-mutant distinction; they do not replace the
original 41 fake / 45 native checks.
HARNESS MODE: when PHOENIX_HARNESS_CYSD is set (release phoenix gate) the plan also runs against a real isolated
cysd — a missing binary there is a FAILURE, not a skip.

Run: python3 cysjavis-pack/bin/tests/test_phoenix_plan_side_effect_free.py   (0 = all PASS). POSIX only.
"""
import fcntl
import hashlib
import importlib.util
import json
import os
import re
import shutil
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import phoenix_plan_rig as rig  # noqa: E402

R = rig.Rig(prefix="phxg-")
check = rig.Checks()
BIN = os.path.dirname(rig.PH)
HASHED = os.path.join(R.root, "hashed")
CTL = os.path.join(R.root, "ctl")
# The subject bytes are unchanged, but their script-owned spool must never be in the checkout.
SUBJECT_PACK = os.path.join(R.root, "subject-pack")
SUBJECT_BIN = os.path.join(SUBJECT_PACK, "bin")
SUBJECT_PH = os.path.join(SUBJECT_BIN, "javis_phoenix.py")
EVENT = os.path.join(SUBJECT_BIN, "javis_event.py")
SPOOL = os.path.join(SUBJECT_PACK, "state", "evt_spool.jsonl")
CLOSURE = ("javis_phoenix.py", "javis_budget.py", "javis_state_snapshot.py",
           "javis_event.py", "javis_scrub.py", "javis_spool_path.py")
os.makedirs(SUBJECT_BIN)
for _name in CLOSURE:
    _src, _dst = os.path.join(BIN, _name), os.path.join(SUBJECT_BIN, _name)
    shutil.copy2(_src, _dst)  # missing dependency is a loud error, never a skipped copy
    with open(_src, "rb") as _a, open(_dst, "rb") as _b:
        assert _a.read() == _b.read(), "subject closure is not byte-identical: " + _name


def event_argv(producer=EVENT):
    """Refuse an outside producer BEFORE launching it; ask the real producer for its actual destination."""
    root, producer = os.path.realpath(SUBJECT_PACK), os.path.realpath(producer)
    if os.path.commonpath((root, producer)) != root:
        raise ValueError("event producer outside observed private pack")
    probe = subprocess.run([sys.executable, "-B", producer, "spool-path"], capture_output=True, timeout=30)
    if probe.returncode != 0:
        raise ValueError("cannot measure event spool path: %r" % probe.stderr)
    path = os.path.realpath(os.fsdecode(probe.stdout).strip())
    if os.path.commonpath((root, path)) != root or path != os.path.realpath(SPOOL):
        raise ValueError("event spool outside observed private pack: " + path)
    return [sys.executable, "-B", producer, "emit", "agent.error",
            "--field", "agent=phoenix", "--field", "summary=m6b", "--spool"]


ALLOWED = {("phoenix-identity",), ("status", "--json"), ("list",)}
BOUND = {"TMPDIR": "tmp", "TEMP": "tmp", "TMP": "tmp", "CYS_PACK_DIR": "pack", "CYS_LOCAL_DIR": "local",
         "HUD_STATE_DIR": "hud", "PHOENIX_DEPT_STATE_ROOT": "depts-root", "PHOENIX_DEPTS_JSON": "depts.json"}


def bound_env(fake):
    env = {k: os.path.join(HASHED, v) for k, v in BOUND.items()}
    env["CYS_BIN"] = fake
    env["PLAN_GATE_EVT_LOG"] = os.path.join(CTL, "evt-calls.log")
    # interpreter bytecode caches (Apple's python defaults to ~/Library/Caches/com.apple.python) are not the
    # plan's writes: send every child's cache outside the hashed roots so a hash difference means a real write
    env["PYTHONPYCACHEPREFIX"] = os.path.join(CTL, "pycache")
    return env


def snapshot(roots=None):
    out = {}
    for top in ((R.home, HASHED, SUBJECT_PACK) if roots is None else roots):
        for dp, dns, fns in os.walk(top):
            for d in dns:
                out[os.path.join(dp, d)] = "dir"
            for f in fns:
                p = os.path.join(dp, f)
                try:
                    st = os.lstat(p)
                    with open(p, "rb") as fh:
                        out[p] = "%s:%o" % (hashlib.sha256(fh.read()).hexdigest(), st.st_mode)
                except OSError as e:
                    out[p] = "unreadable:%s" % e.errno
    return out


def diff(a, b):
    return sorted(k for k in set(a) | set(b) if a.get(k) != b.get(k))


def sock():
    return os.path.join(R.state, "cys.sock")


def worlds():
    D3, T3 = rig.desired(rig.ROLES, rev=3), rig.topo(rig.ROLES, rev=3)
    bak = {os.path.join("phoenix", "desired_roster.json.bak"): rig.desired(rig.ROLES + ["worker-3"], rev=3)}
    return [
        ("healthy", dict(desired=D3, topo=T3, live=rig.LIVE_ALL), {}),
        ("dead_role", dict(desired=D3, topo=T3, live=rig.LIVE_W2_GONE), {}),
        ("c6_exited_only", dict(desired=D3, topo=T3,
                                live=rig.LIVE_ALL + [rig.surf("surface:9", "worker-9", exited=True)]), {}),
        ("pending_intent", dict(desired=D3, topo=T3, live=rig.LIVE_ALL,
                                intents=json.dumps({"op": "add", "role": "worker-1", "ts": 1}) + "\n"), {}),
        ("corrupt_desired_bak", dict(desired=rig.CORRUPT, topo=T3, live=rig.LIVE_ALL, files=bak), {}),
        ("desired_absent_bak", dict(desired=None, topo=T3, live=rig.LIVE_ALL, files=bak), {}),
        ("topology_corrupt", dict(desired=D3, topo=rig.CORRUPT, live=rig.LIVE_W2_GONE), {}),
        ("rev_regress", dict(desired=rig.desired(rig.ROLES, tombs=["worker-2"], rev=4), topo=T3,
                             live=rig.LIVE_W2_GONE), {}),
        ("generation_corrupt", dict(desired=None, topo=rig.topo(["master", "worker-1"], rev=3),
                                    snapshot=rig.CORRUPT, live=rig.LIVE_W2_GONE), {}),
        ("topology_missing", dict(desired=D3, topo=None, live=rig.LIVE_W2_GONE), {}),
        ("fresh_install", dict(desired=None, topo=None, live=rig.LIVE_ALL), {"no_phoenix_dir": True}),
        ("held_run_lock", dict(desired=D3, topo=T3, live=rig.LIVE_W2_GONE), {"hold": "restore.run.lock"}),
        ("ambiguous_fresh_role", dict(desired=rig.desired(rig.ROLES + ["worker-fresh2"], rev=3), topo=T3,
                                      live=rig.LIVE_ALL), {}),
        ("c3_world", dict(desired=rig.desired(rig.ROLES + ["worker-7"], rev=3), topo=T3, live=rig.LIVE_ALL), {}),
        ("compare_plan", dict(desired=rig.desired(rig.ROLES, tombs=["worker-2"], rev=3), topo=None,
                              live=rig.LIVE_W2_GONE), {"compare": True}),
    ]


def gate(fake, script, stop_at_first=False):
    """Run every world through `script` → list of (world, changed paths, odd calls, rc)."""
    findings = []
    world_cli = {"self": rig.SELF_ID, "daemon": dict(rig.SELF_ID), "surfaces": []}
    for name, files, opt in worlds():
        R.world(**files)
        if os.path.isdir(HASHED):
            shutil.rmtree(HASHED)
        for k, v in BOUND.items():
            p = os.path.join(HASHED, v)
            if v.endswith(".json"):
                rig._write(p, {"depts": {}})
            else:
                os.makedirs(p, exist_ok=True)
        os.makedirs(os.path.join(R.home, ".cys", "local"), exist_ok=True)
        if opt.get("no_phoenix_dir"):
            shutil.rmtree(R.phx)
        holder = None
        if opt.get("hold"):
            holder = open(os.path.join(R.phx, opt["hold"]), "a+")
            fcntl.flock(holder.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
        args = ["--socket", sock(), "plan"]
        if opt.get("compare"):
            base = os.path.join(CTL, "plan-pre.out")
            rig._write(base, rig.PREFIX + json.dumps({"phoenix_plan": 1, "outcome": "targets",
                                                      "targets": ["worker-2"], "reasons": [],
                                                      "observed": {"view_roles": rig.ROLES,
                                                                   "view_tombstones": []}}) + "\n")
            args += ["--compare-plan", base]
        world_cli["surfaces"] = R.surfaces
        evt_log = os.path.join(CTL, "evt-calls.log")
        if os.path.exists(evt_log):
            os.remove(evt_log)
        before = snapshot()
        try:
            rc, framed, d, out, err, calls = R.run_cli(fake, args, world_cli, bound_env(fake), script=script)
        finally:
            if holder is not None:
                fcntl.flock(holder.fileno(), fcntl.LOCK_UN)
                holder.close()
        after = snapshot()
        odd = [c for c in calls if tuple(c[:2]) not in ALLOWED and tuple(c[:1]) not in ALLOWED]
        if os.path.exists(evt_log):
            odd += [["javis_event"] + json.loads(ln) for ln in open(evt_log, encoding="utf-8") if ln.strip()]
        findings.append((name, diff(before, after), odd, rc, (d or {}).get("outcome")))
        if stop_at_first and (findings[-1][1] or odd):
            break
    return findings


MUTANTS = [
    ("M1 journal write", "    intents = _read_tombstone_intents(socket)\n    # restore 와 같은 한 경로",
     "    save_journal(socket, \"plan\", {\"ticket_id\": \"plan\", \"roles\": {}, \"events\": []})\n"
     "    intents = _read_tombstone_intents(socket)\n    # restore 와 같은 한 경로"),
    ("M2 persisting observe", "    roster, tombstones, meta = _desired_view(socket, False, intents, False)\n",
     "    roster, tombstones = observe_and_persist_roster(socket)\n"
     "    meta = _desired_view(socket, False, intents, False)[2]\n"),
    ("M3 C6 reap", "    live, live_known = _live_role_surfaces_checked(socket)\n    _requested,",
     "    c6_reap_stale_surfaces(socket)\n    live, live_known = _live_role_surfaces_checked(socket)\n    _requested,"),
    ("M4 intent resync", "    intents = _read_tombstone_intents(socket)\n    # restore 와 같은 한 경로",
     "    intents = _read_tombstone_intents(socket)\n    _resync_intents_if_daemon_up(socket, intents)\n"
     "    # restore 와 같은 한 경로"),
    ("M5 creating read path", "def desired_roster_path(socket):\n    return os.path.join(phoenix_home_path(socket),",
     "def desired_roster_path(socket):\n    return os.path.join(phoenix_home(socket),"),
    ("M6 event emit", "    units = _restore_units_now(socket, live, True, roster, tombstones)\n",
     "    subprocess.run([sys.executable, \"-B\", os.path.join(os.path.dirname(__file__), \"javis_event.py\"), "
     "\"emit\", \"agent.error\", \"--field\", \"agent=phoenix\", \"--field\", \"summary=plan mutant\"], "
     "capture_output=True, check=True)\n"
     "    units = _restore_units_now(socket, live, True, roster, tombstones)\n"),
    ("M6b event emit --spool", "    units = _restore_units_now(socket, live, True, roster, tombstones)\n",
     "    subprocess.run(%r, capture_output=True, check=True)\n"
     "    units = _restore_units_now(socket, live, True, roster, tombstones)\n" % event_argv()),
    ("M7 resolve journal", "_resolve_cys(args.socket, record=(args.cmd != \"plan\"))",
     "_resolve_cys(args.socket, record=True)"),
    ("M8 temp file", "    units = _restore_units_now(socket, live, True, roster, tombstones)\n",
     "    __import__(\"tempfile\").NamedTemporaryFile(delete=False)\n"
     "    units = _restore_units_now(socket, live, True, roster, tombstones)\n"),
]


EVT_RECORDER = """import json, os, sys
with open(os.environ["PLAN_GATE_EVT_LOG"], "a") as f:
    f.write(json.dumps(sys.argv[1:]) + "\\n")
"""


def module_copy(label, text):
    """A copy of the module beside copies of the siblings it imports, with javis_event.py replaced by a recorder:
    M6 invokes that recorder directly; its outside-the-hash log is a wire-call witness, not a file-effect witness."""
    d = os.path.join(CTL, re.sub(r"\W+", "_", label))
    os.makedirs(d, exist_ok=True)
    for sib in ("javis_budget.py", "javis_state_snapshot.py"):
        if os.path.exists(os.path.join(BIN, sib)):
            shutil.copy2(os.path.join(BIN, sib), os.path.join(d, sib))
    with open(os.path.join(d, "javis_event.py"), "w", encoding="utf-8") as f:
        f.write(EVT_RECORDER)
    p = os.path.join(d, "javis_phoenix.py")
    with open(p, "w", encoding="utf-8") as f:
        f.write(text)
    return p


def mutant_script(label, anchor, repl):
    src = open(SUBJECT_PH, encoding="utf-8").read()
    if src.count(anchor) != 1:
        return None, src.count(anchor)
    return module_copy(label, src.replace(anchor, repl, 1)), 1


def event_hits(rows, label):
    """An event mutation must actually execute successfully, and be caught on its intended channel."""
    valid = [(w, c, o) for w, c, o, rc, outcome in rows if rc == 0 and outcome in rig_outcomes()]
    if label == "M6 event emit":
        return [(w, c, o) for w, c, o in valid if any(call[:1] == ["javis_event"] for call in o)]
    payload_seen = False
    if os.path.isfile(SPOOL):
        with open(SPOOL, encoding="utf-8") as f:
            payload_seen = any(json.loads(line).get("payload", {}).get("summary") == "m6b"
                               for line in f if line.strip())
    return [(w, c, o) for w, c, o in valid if SPOOL in c and payload_seen]


def negative_controls(fake):
    results = []

    def record(name, ok, detail):
        results.append(bool(ok))
        print("NEGATIVE-CONTROL %s %s | %s" % ("PASS" if ok else "FAIL", name, detail))

    m6, m6b = MUTANTS[5], MUTANTS[6]
    script, n = mutant_script("negative recorder", m6[1], m6[2])
    assert n == 1
    with open(os.path.join(os.path.dirname(script), "javis_event.py"), "w", encoding="utf-8") as f:
        f.write("import sys\nsys.exit(0)\n")
    rows = gate(fake, script)
    record("recorder append off is not caught", all(r[3] == 0 for r in rows)
           and not event_hits(rows, m6[0]) and not any(r[1] or r[2] for r in rows),
           "worlds=%d rc=%r" % (len(rows), sorted(set(r[3] for r in rows))))

    with open(EVENT, "rb") as f:
        original = f.read()
    anchor = b"            _spool_append(a.type, payload, surface)"
    assert original.count(anchor) == 1, "spool suppression anchor moved"
    try:
        with open(EVENT, "wb") as f:
            f.write(original.replace(anchor, b"            pass  # negative control: no append", 1))
        script, n = mutant_script("negative spool", m6b[1], m6b[2])
        assert n == 1
        rows = gate(fake, script)
        record("spool write off is not caught", all(r[3] == 0 for r in rows)
               and not event_hits(rows, m6b[0]) and not any(r[1] or r[2] for r in rows),
               "worlds=%d rc=%r" % (len(rows), sorted(set(r[3] for r in rows))))
    finally:
        with open(EVENT, "wb") as f:
            f.write(original)

    outside = os.path.join(R.root, "outside-observer", "javis_event.py")
    launched = []
    real_run = subprocess.run
    def forbidden_run(*args, **kwargs):
        launched.append(args)
        raise AssertionError("outside producer was launched")
    refused = False
    try:
        subprocess.run = forbidden_run
        try:
            event_argv(outside)
        except ValueError as e:
            refused = "outside observed private pack" in str(e)
    finally:
        subprocess.run = real_run
    record("outside producer refused before launch", refused and not launched and not os.path.exists(outside),
           "refused=%r launches=%d" % (refused, len(launched)))

    bad = "    _emit_evt(\"agent.error\", agent=\"phoenix\", summary=\"invalid mutant\")\n" + m6[1]
    script, n = mutant_script("negative invalid helper", m6[1], bad)
    assert n == 1
    # Preserve the framed error for this control, without changing the run_cli return value.
    errors = []
    real_cli = R.run_cli
    def observed_cli(*args, **kwargs):
        value = real_cli(*args, **kwargs)
        errors.append("NameError" in value[3] and "_emit_evt" in value[3])
        return value
    try:
        R.run_cli = observed_cli
        rows = gate(fake, script)
    finally:
        R.run_cli = real_cli
    record("invalid NameError mutant is not credited", len(rows) == 15 and all(errors)
           and all(r[3] == 1 and r[4] == "error" for r in rows) and not event_hits(rows, m6[0]),
           "worlds=%d framed_NameErrors=%d" % (len(rows), sum(errors)))
    print("NEGATIVE-CONTROLS %d/%d PASS" % (sum(results), len(results)))
    return len(results) == 4 and all(results)


def harness_mode():
    cysd = os.environ.get("PHOENIX_HARNESS_CYSD")
    if not cysd:
        print("harness mode not requested (PHOENIX_HARNESS_CYSD unset) — fake-daemon mode only")
        return
    cys = os.environ.get("PHOENIX_CYS") or shutil.which("cys")
    if not (os.path.exists(cysd) and cys and os.path.exists(cys)):
        check("harness mode: PHOENIX_HARNESS_CYSD and the cys CLI exist (missing = failure, not skip)", False,
              "cysd=%r cys=%r" % (cysd, cys))
        return
    spec = importlib.util.spec_from_file_location("phx_harness_g1", os.path.join(BIN, "javis_phoenix_harness.py"))
    h = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(h)
    h.CYS, h.CYSD = cys, cysd
    h.guard_isolation()
    h.teardown(verbose=False)
    h._fresh_harness()
    h.start_daemon(wait=12.0)
    try:
        ns = h.cys("new-surface", "--role", "worker-g1").stdout or ""
        ref = re.search(r"(surface:\d+)", ns)
        pdir = os.path.join(h.HARN_DIR, "phoenix")
        os.makedirs(pdir, exist_ok=True)
        rig._write(os.path.join(pdir, "desired_roster.json"), rig.desired(["worker-g1", "worker-gone"], rev=0))

        def norm_status():
            r = h.cys("status", "--json")
            try:
                st = json.loads(r.stdout or "{}")
            except ValueError:
                return None
            return sorted((s.get("surface_ref"), s.get("role"), bool(s.get("exited")), s.get("seat"))
                          for s in st.get("surfaces", []))

        def files():
            out = {"subject:" + os.path.relpath(p, SUBJECT_PACK): value
                   for p, value in snapshot((SUBJECT_PACK,)).items()}
            for rel in ("phoenix", "phoenix-embed", "topology.json", "phoenix-restore.log"):
                p = os.path.join(h.HARN_DIR, rel)
                if os.path.isfile(p):
                    out[rel] = hashlib.sha256(open(p, "rb").read()).hexdigest()
                for dp, dns, fns in os.walk(p) if os.path.isdir(p) else []:
                    for d in dns:
                        out[os.path.relpath(os.path.join(dp, d), h.HARN_DIR)] = "dir"
                    for f in fns:
                        q = os.path.join(dp, f)
                        out[os.path.relpath(q, h.HARN_DIR)] = hashlib.sha256(open(q, "rb").read()).hexdigest()
            return out

        s0, f0 = norm_status(), files()
        env = dict(os.environ, PHOENIX_CYS=cys)
        p = subprocess.run([sys.executable, "-B", SUBJECT_PH, "--socket", h.HARN_SOCK, "plan"], env=env,
                           stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=120)
        s1, f1 = norm_status(), files()
        out = p.stdout.decode("utf-8", "replace")
        framed = [ln for ln in out.split("\n") if ln.startswith(rig.PREFIX)]
        d = json.loads(framed[0][len(rig.PREFIX):]) if len(framed) == 1 else {}
        check("harness mode: real cysd plan exit 0, identity verified, worker-gone a target",
              p.returncode == 0 and d.get("identity") == "verified" and "worker-gone" in (d.get("targets") or []),
              "rc=%r d=%r ref=%r" % (p.returncode, d, ref and ref.group(1)))
        check("harness mode: daemon surfaces unchanged by the plan", s0 is not None and s0 == s1, "%r -> %r" % (s0, s1))
        check("harness mode: phoenix/, phoenix-embed/, topology.json, phoenix-restore.log unchanged",
              f0 == f1, "changed=%r" % sorted(k for k in set(f0) | set(f1) if f0.get(k) != f1.get(k)))
        # positive control on the same real daemon: the persisting-observe mutant (M2) must show up here
        label, anchor, repl = MUTANTS[1]
        m2, _n = mutant_script("harness " + label, anchor, repl)
        if m2 is not None:
            subprocess.run([sys.executable, "-B", m2, "--socket", h.HARN_SOCK, "plan"], env=env,
                           stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=120)
        f2 = files()
        check("harness mode positive control: the M2 copy changes the measured files (the comparison can fail)",
              m2 is not None and f2 != f1, "changed=%r" % sorted(k for k in set(f1) | set(f2) if f1.get(k) != f2.get(k)))
    finally:
        h.teardown(verbose=False)


def main():
    os.makedirs(CTL, exist_ok=True)
    fake = R.fake_cli()
    env = dict(bound_env(fake), HOME=R.home, PATH="/usr/bin:/bin")
    os.makedirs(os.path.join(HASHED, "tmp"), exist_ok=True)
    probe = subprocess.run([sys.executable, "-c", "import tempfile; print(tempfile.gettempdir())"], env=env,
                           stdout=subprocess.PIPE, timeout=30)
    tmpdir = os.path.realpath(probe.stdout.decode().strip())
    check("probe: the child's temp root is under the hashed root (M8 check is not void)",
          tmpdir.startswith(os.path.realpath(HASHED) + os.sep), tmpdir)

    src = open(SUBJECT_PH, encoding="utf-8").read()
    for layout, script in (("shipped layout", SUBJECT_PH), ("recorder copy", module_copy("unmutated", src))):
        for name, changed, odd, rc, outcome in gate(fake, script):
            check("%s / %s: plan exit 0, nothing written, only identity/status/list calls, no event" % (layout, name),
                  rc == 0 and not changed and not odd and outcome in rig_outcomes(),
                  "rc=%r outcome=%r changed=%r odd=%r" % (rc, outcome, changed[:4], odd[:3]))

    for label, anchor, repl in MUTANTS:
        script, n = mutant_script(label, anchor, repl)
        if script is None:
            check("mutation control %s: anchor found exactly once" % label, False, "count=%d" % n)
            continue
        rows = gate(fake, script, stop_at_first=True)
        if label in ("M6 event emit", "M6b event emit --spool"):
            hits = event_hits(rows, label)
        else:
            hits = [(w, c[:2], o[:2]) for w, c, o, rc, _ in rows if c or o]
        check("mutation control %s: caught" % label, bool(hits), hits[:1])
        if label == "M6 event emit":
            check("mutation control M6 is caught by the recorded wire call (not by a file artefact)",
                  any(o for _w, _c, o in hits), hits[:1])

    controls_ok = negative_controls(fake)
    harness_mode()
    R.close()
    result = check.finish()
    return result if result else (0 if controls_ok else 1)


def rig_outcomes():
    return ("empty", "targets", "uncertified")


if __name__ == "__main__":
    sys.exit(main())
