#!/usr/bin/env python3
"""★F-1 R2: real run_restore production stages with a scripted CLI (no daemon).

Run: python3 cysjavis-pack/bin/tests/test_phoenix_f1_production_path.py
Only cys, sleep and the external event emitter are replaced; roster, inventory,
leases, journal persistence and the complete restore state machine remain real.
"""
import copy
import importlib.util, json, os, shutil, sys, tempfile
from types import SimpleNamespace

sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
PH = os.path.normpath(os.path.join(HERE, "..", "javis_phoenix.py"))
spec = importlib.util.spec_from_file_location("javis_phoenix", PH)
m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m)

_results = []
def check(name, cond, extra=""):
    """`extra` is printed only when the row fails — a red row in CI must carry its own diagnosis."""
    _results.append(cond)
    print(("PASS " if cond else "FAIL ") + name + ("" if cond or not extra else "  | " + str(extra)))


def scenario(name):
    td = tempfile.mkdtemp(prefix="phoenix-f1-production-")
    original_cys, original_sleep, original_emit = m.cys, m.time.sleep, m._emit_evt
    original_epoch, original_known = m._ACTIVE_EPOCH, m._LAST_LIVENESS_KNOWN
    try:
        socket_path = os.path.join(td, "cys.sock")
        ticket = "round2-" + name
        entry = {"role": "worker-1", "agent": "claude", "session_id": "",
                 "claude_config_dir": os.path.join(td, "acct"), "cwd": os.path.join(td, "proj"),
                 "surface_id": 7, "surface_ref": "surface:7"}
        os.makedirs(entry["cwd"])
        project_dir = os.path.join(entry["claude_config_dir"], "projects",
                                   m._claude_project_component(entry["cwd"]))
        os.makedirs(project_dir)
        def session_file(sid):
            with open(os.path.join(project_dir, sid + ".jsonl"), "w", encoding="utf-8") as f:
                f.write("{}\n")
        session_file("old")
        now = m._now()
        with open(os.path.join(td, "topology.json"), "w", encoding="utf-8") as f:
            json.dump({"entries": [entry], "updated_at": now, "schema_version": 1,
                       "tombstones": [], "tombstones_rev": 0}, f)
        journal_path = m.journal_path(socket_path, ticket)
        calls, statuses, snapshots, sleeps = [], [], [], []
        state = {"alive": False, "status_count": 0, "checks": 0, "switch_after": None}

        def fake_cys(*args, socket=None, timeout=25):
            calls.append((args[0], args[1:], socket, timeout))
            assert socket == socket_path, "CLI escaped isolated socket"
            verb = args[0]
            stdout = ""
            if verb == "status":
                assert args == ("status", "--json")
                state["status_count"] += 1
                sid = "new1"
                # S3: after the first reinject --check, allow TWO more status
                # replies for reinject_sid capture and the G2 agent gate. The
                # verify status is the third. Switch permanently after that
                # threshold, never on an exact global status-call index.
                if name == "S3" and state["switch_after"] is not None:
                    if state["status_count"] > state["switch_after"]:
                        sid = "new2"
                        session_file(sid)
                # S4: hold the old occupant until the first reinject --check.
                # Thus all F1_REGISTERED_GRACE_TRIES observe polls see old;
                # the subsequent binding/verify sees new1 and triggers retry.
                if name == "S4" and state["checks"] == 0:
                    sid = "old"
                # ★리뷰 R3(codex major) S5: 등록이 **첫 주입 뒤에야** 잡힌다 — 관측 폴링은 전부 미등록을
                # 보고, 각성 핑에는 (등록 전) 세션 A 가 답하며, 그 사이 B(new1)가 등록해 주입 **직후**
                # 읽기는 B 를 준다. 직후만 보면 A 의 ACK 가 B 의 증거로 채택된다(재현 대상).
                if name == "S5" and state["checks"] == 0:
                    sid = None
                rows = []
                if state["alive"]:
                    rows = [{"surface_id": 7, "surface_ref": "surface:7", "role": "worker-1",
                             "exited": False, "agent_alive": True, "seat": "occupied",
                             "agent": "claude", "gate_pending": {"gate": "folder-trust"} if name == "S1" else None,
                             "registered_session_id": sid, "awakened_at": now}]
                statuses.append((state["status_count"], state["checks"], copy.deepcopy(rows)))
                stdout = json.dumps({"daemon": {"started_at": now}, "surfaces": rows})
            elif verb == "restore":
                assert args == ("restore",)
                state["alive"] = True
                session_file("new1")
                stdout = "restored worker-1 surface:7"
            elif verb == "reinject":
                assert args[:6] == ("reinject", "--check", "--role", "worker-1", "--surface", "surface:7")
                # ★U8 P0-M2(0.14.41): G2 는 재주입 없는 ACK 확인 전용(`--ack-only`) — stage_reinject(6)만 재주입 자격이 있다.
                assert args[6:] in (("--timeout", "6"), ("--timeout", "4", "--ack-only")), args
                with open(journal_path, encoding="utf-8") as f:
                    snapshots.append(json.load(f))
                state["checks"] += 1
                if name == "S3" and state["switch_after"] is None:
                    state["switch_after"] = state["status_count"] + 2
                stdout = "디렉티브 생존 확인 (ACK 수신)"
            elif verb == "read-screen":
                stdout = "bypass permissions on"
            elif verb == "watch":
                stdout = "bypass permissions on"
            elif verb == "list":
                stdout = "surface:7 role=worker-1 pid=7 exited=false\n" if state["alive"] else ""
            else:
                # Never silently accept an unmodelled or destructive CLI call.
                raise AssertionError("unexpected cys call: %r" % (args,))
            return SimpleNamespace(returncode=0, stdout=stdout, stderr="")

        m.cys = fake_cys
        m.time.sleep = lambda seconds: sleeps.append(seconds)
        m._emit_evt = lambda *args, **kwargs: None  # production emitter shells out
        result = m.run_restore(socket_path, ticket=ticket, stub=False,
                               no_breaker=True, print_result=False)
        with open(journal_path, encoding="utf-8") as f:
            journal = json.load(f)
        rr = journal["roles"]["worker-1"]
        events = journal["events"]
        verbs = [verb for verb, args, socket, timeout in calls]
        reobserve = [e for e in events if e["stage"] == "verify" and e["status"] == "reobserve"]
        check(name + " production backend", result["backend"] == "production(cys restore)"
              and result["target_roles"] == ["worker-1"])
        check(name + " exactly one restore", verbs.count("restore") == 1)
        check(name + " non-destructive CLI", not set(verbs) & {"close-surface", "kill", "reclaim"})
        check(name + " CLI protocol", set(verbs) == {"status", "restore", "reinject"}
              and all(socket == socket_path for verb, args, socket, timeout in calls)
              and verbs.index("restore") < verbs.index("reinject"))
        check(name + " preserves spawn inventory", rr["fresh_pre_sids"] == ["old"]
              and all(s["roles"]["worker-1"]["fresh_pre_sids"] == ["old"] for s in snapshots)
              and os.path.exists(os.path.join(project_dir, "new1.jsonl")))
        with open(m.desired_roster_path(socket_path), encoding="utf-8") as f:
            desired = json.load(f)
        check(name + " real roster persisted", desired["roster"]["worker-1"] == entry)
        check(name + " no poison fallback", result["fresh_fallback_roles"] == [])
        # ★리뷰 R3b: 실 run_restore 가 어댑터 resume 효력을 저널에 기록한다 — 이 하네스의 팩은 임시 디렉터리라
        #   agents.json 이 없다(=unknown). 비-F1 축은 이 값이 unknown 이면 핀 일치를 verified 로 올리지 않는다.
        check(name + " records resume mode", rr.get("resume_mode") == "unknown"
              and m.legacy_verify_outcome("s1", "s1", rr.get("resume_mode"))[0] == "unverified")
        if name == "S1":
            check(name + " held gate stays unverified", result["phoenix_restore"] == "UNVERIFIED"
                  and result["per_role_outcome"] == {"worker-1": "unverified"}
                  and result["fresh_unverified_roles"] == ["worker-1"]
                  and result["fresh_roles"] == [] and result["fresh_reasons"] == {}
                  and rr["stages"]["verify"]["done"] is False
                  and rr["fresh_evidence"]["gate_cleared"] is False and not reobserve)
        else:
            sid = "new2" if name == "S3" else "new1"
            check(name + " verified fresh", result["phoenix_restore"] == "VERIFIED_FRESH"
                  and result["per_role_outcome"] == {"worker-1": "fresh"}
                  and result["fresh_roles"] == ["worker-1"] and result["fresh_unverified_roles"] == []
                  and result["fresh_reasons"] == {"worker-1": "no_session"}
                  and "F-1 fresh 각성" in result["honesty_note"]
                  and rr["stages"]["verify"]["done"] is True)
            check(name + " bound new session", rr["observed_sid"] == rr["reinject_sid"] == sid
                  and rr["fresh_evidence"]["provenance"] == "new"
                  and rr["fresh_evidence"]["registered_now"] == rr["fresh_evidence"]["reinject_sid"] == sid)
        if name in ("S3", "S4", "S5"):
            expected_prov = "unobserved" if name == "S5" else "changed"
            check(name + " same-run reobserve", len(reobserve) == 1 and expected_prov in reobserve[0]["msg"])
            # Distinguish stage reinject from G2's own reinject --check call.
            check(name + " recollects injection and G2", sum(args[-1] == "6" for verb, args, socket, timeout in calls
                  if verb == "reinject") == 2 and verbs.count("reinject") == 4)
            retry = snapshots[2]["roles"]["worker-1"]
            check(name + " invalidated old evidence before retry", "reinject_sid" not in retry
                  and "reinject_sid_before" not in retry
                  and retry["stages"]["reinject"]["done"] is False
                  and retry["stages"]["g2_ack"]["done"] is False
                  and retry["stages"]["verify"]["done"] is False)
        if name == "S5":
            # 1회차의 ACK 는 **어느 세션에도 귀속되지 않는다** — 주입 직전 결속이 미관측이었기 때문이다.
            first = snapshots[1]["roles"]["worker-1"]
            check(name + " unattributed ACK is not inherited", first.get("observed_sid") is None
                  and first.get("reinject_sid") is None and first.get("reinject_sid_before") is None)
            # 계측 타당성: 그 주입 **직후** 읽기는 실제로 B 를 준다(직후만 보면 A 의 ACK 가 B 로 귀속됐다).
            check(name + " race actually happened", any(
                rows and rows[0]["registered_session_id"] == "new1" for n, checks, rows in statuses if checks >= 1)
                and all(not rows or rows[0]["registered_session_id"] is None
                        for n, checks, rows in statuses if checks == 0))
            # 2회차는 B 가 결속된 채 다시 주입해 증거를 모은다(직전=직후=관측).
            check(name + " second pass binds the evidence", rr["reinject_sid_before"] == rr["reinject_sid"]
                  == rr["observed_sid"] == "new1" and rr["fresh_evidence"]["provenance"] == "new")
        if name == "S3":
            first = snapshots[1]["roles"]["worker-1"]
            check(name + " A bound before verify switches to B", first["observed_sid"] == first["reinject_sid"] == "new1"
                  and any(rows and rows[0]["registered_session_id"] == "new2" for n, count, rows in statuses))
        if name == "S4":
            first = snapshots[0]["roles"]["worker-1"]
            check(name + " stale held through full observe grace", first["observed_sid"] == "old"
                  and "스폰 전 재고" in first["stages"]["resume"]["evidence"]
                  and ("attempt %d" % m.F1_REGISTERED_GRACE_TRIES) in first["stages"]["resume"]["evidence"]
                  # grace 를 한 번은 끝까지 썼다(tries-1 회의 grace sleep) — SPAWN_SETTLE 등 다른 sleep 과 섞이므로 값으로 센다.
                  and sum(1 for s in sleeps if s == m.PHOENIX_SESSION_GRACE_SLEEP) >= m.F1_REGISTERED_GRACE_TRIES - 1)
    finally:
        m.cys, m.time.sleep, m._emit_evt = original_cys, original_sleep, original_emit
        m._ACTIVE_EPOCH, m._LAST_LIVENESS_KNOWN = original_epoch, original_known
        shutil.rmtree(td)


# ──────────────────────────────────────────────────────────────────────────────
# D1 — defect `D5H-20261028-PHOENIX-PIN-VERIFY-SAME-SOURCE`: the RESUMED-role path through the real
# `run_restore(stub=False)`. Everything but `cys`, `sleep` and the event emitter is the shipped code:
# roster, journal, inventory, the observation stage, the verdict and the enum.
# Design: DESIGN.md 34f04804 + DESIGN-R2.md 4d7ecb83 + DESIGN-R3.md 30efd72f; HEAD-RULING-R3-FINAL 1-6.
# ──────────────────────────────────────────────────────────────────────────────
D1_PIN = "PIN-1"


def d1_scenario(name):
    """One resumed-role restore. `name` selects the shape; every shape is produced by the shipped parser."""
    td = tempfile.mkdtemp(prefix="phoenix-d1-production-")
    original_cys, original_sleep, original_emit = m.cys, m.time.sleep, m._emit_evt
    original_epoch, original_known = m._ACTIVE_EPOCH, m._LAST_LIVENESS_KNOWN
    original_pack = os.environ.get("CYS_PACK_DIR")
    try:
        socket_path = os.path.join(td, "cys.sock")
        ticket = "d1-" + name
        agent = "codex" if name == "R4" else "claude"
        entry = {"role": "worker-1", "agent": agent, "session_id": D1_PIN,
                 "claude_config_dir": os.path.join(td, "acct"), "cwd": os.path.join(td, "proj"),
                 "surface_id": 9, "surface_ref": "surface:9"}
        os.makedirs(entry["cwd"])
        project_dir = os.path.join(entry["claude_config_dir"], "projects",
                                   m._claude_project_component(entry["cwd"]))
        os.makedirs(project_dir)
        for sid in (D1_PIN, "SID-PREV"):
            with open(os.path.join(project_dir, sid + ".jsonl"), "w", encoding="utf-8") as f:
                f.write("{}\n")
        # the adapter declaration the resume axis reads — in-rig, never the live pack.
        pack = os.path.join(td, "pack")
        os.makedirs(pack)
        with open(os.path.join(pack, "agents.json"), "w", encoding="utf-8") as f:
            json.dump({"claude": {"resume_arg": "--resume {session_id}"}}, f)
        os.environ["CYS_PACK_DIR"] = pack
        m._AGENTS_CACHE.clear()
        now = m._now()
        topo = {"entries": [entry], "updated_at": now, "schema_version": 1,
                "tombstones": [], "tombstones_rev": 0}
        topo_path = os.path.join(td, "topology.json")
        with open(topo_path, "w", encoding="utf-8") as f:
            json.dump(topo, f)
        journal_path = m.journal_path(socket_path, ticket)
        calls, sleeps, statuses = [], [], []
        state = {"alive": False, "status_count": 0, "checks": 0}

        def seat_row(sid, seat="occupied", alive=True):
            return {"surface_id": 9, "surface_ref": "surface:9", "role": "worker-1", "exited": False,
                    "agent_alive": alive, "seat": seat, "agent": agent,
                    "gate_pending": None, "registered_session_id": sid}

        # "warm" = the real in-seat reconnect shape: the pane survives but the agent left, so the SEAT is
        # empty (javis_phoenix.py:2720-2721 makes that role a revival target) and the dead occupant's
        # registration is still on it (:2367).
        warm = name in ("R3a", "R3c", "R6")

        def status_reply():
            """(rc, stdout) per shape. Every call BEFORE `cys restore` is a pre-spawn observation — the
            roster merge makes one (:1653-1656) and the target snapshot another (:2708)."""
            after = {"R1": D1_PIN, "R2": "OTHER-9", "R3a": D1_PIN, "R3b": D1_PIN, "R3c": D1_PIN,
                     "R4": None, "R5": D1_PIN, "R6": "SID-PREV"}[name]
            if not state["alive"]:
                if name == "R3a":
                    return 1, ""                      # rc != 0 -> the `cys list` fallback decides
                if name == "R3b":
                    return 0, ""                      # rc 0 with EMPTY stdout -> {} (no "surfaces" key)
                pre_rows = ([seat_row(D1_PIN if name == "R3c" else "SID-PREV", seat="empty", alive=False)]
                            if warm else [])
                return 0, json.dumps({"daemon": {"started_at": now}, "surfaces": pre_rows})
            return 0, json.dumps({"daemon": {"started_at": now}, "surfaces": [seat_row(after)]})

        def fake_cys(*args, socket=None, timeout=25):
            calls.append(args[0])
            assert socket == socket_path, "CLI escaped the isolated socket"
            verb = args[0]
            if verb == "status":
                state["status_count"] += 1
                rc, stdout = status_reply()
                statuses.append((state["status_count"], rc, stdout[:60]))
                return SimpleNamespace(returncode=rc, stdout=stdout, stderr="")
            if verb == "list":
                # R3a: the fallback answers with NOTHING (rc 0, empty) -> ({}, known=True) by :1840
                return SimpleNamespace(returncode=0, stdout="", stderr="")
            if verb == "restore":
                state["alive"] = True
                if name == "R5":
                    # ★the daemon re-pins mid-restore (a qualifying /clear): topology changes under us.
                    moved = copy.deepcopy(topo)
                    moved["entries"][0]["session_id"] = "REPINNED-9"
                    with open(topo_path, "w", encoding="utf-8") as f:
                        json.dump(moved, f)
                return SimpleNamespace(returncode=0, stdout="restored worker-1 surface:9", stderr="")
            if verb == "reinject":
                state["checks"] += 1
                return SimpleNamespace(returncode=0, stdout="디렉티브 생존 확인 (ACK 수신)", stderr="")
            if verb in ("read-screen", "watch"):
                return SimpleNamespace(returncode=0, stdout="bypass permissions on", stderr="")
            raise AssertionError("unexpected cys call: %r" % (args,))

        m.cys = fake_cys
        m.time.sleep = lambda seconds: sleeps.append(seconds)
        m._emit_evt = lambda *a, **k: None
        result = m.run_restore(socket_path, ticket=ticket, stub=False, no_breaker=True, print_result=False)
        with open(journal_path, encoding="utf-8") as f:
            journal = json.load(f)
        rr = journal["roles"]["worker-1"]
        ev = rr.get("resume_evidence") or {}
        n = "d1 " + name

        check(n + " production backend, one restore, non-destructive",
              result["backend"] == "production(cys restore)" and calls.count("restore") == 1
              and not set(calls) & {"close-surface", "kill", "reclaim"})
        check(n + " took the resume arm (not F-1 fresh)", not rr.get("fresh_expected"))
        if name == "R1":
            check(n + " verified: the seat registered the pin on a pane this restore created",
                  rr["outcome"] == "verified" and result["phoenix_restore"] == "VERIFIED"
                  and m.restore_exit_code(result) == 0
                  and ev["pane_is_new"] is True and ev["registered_now"] == D1_PIN)
            check(n + " provenance journalled: structured, known, enumerated",
                  rr["pre_spawn_snapshot_source"] == "status_json"
                  and rr["pre_spawn_snapshot_known"] is True
                  and rr["pre_spawn_snapshot_enumerated"] is True)
            check(n + " the compared values come from different writers",
                  rr["expected_sid"] == D1_PIN and rr["observed_sid"] == D1_PIN
                  # ★D1xD3 BC1: the provenance lives under a defect-1 key, NOT under d3's marker key.
                  # `.get()` so a regression that renames the key back fails as a NAMED row, not a KeyError
                  and rr.get("resume_observed_source") == "registered"
                  and "observed_sid_source" not in rr
                  and ev.get("observed_sid_source") == "registered",
                  extra="resume_observed_source=%r · marker key present=%s · evidence=%r" % (
                      rr.get("resume_observed_source"), "observed_sid_source" in rr,
                      ev.get("observed_sid_source")))
            check(n + " the project dir is journalled for the resume path",
                  rr["resume_project_dir"] == project_dir)
        if name == "R2":
            check(n + " stale pin: the seat registered another conversation -> unverified(compare)",
                  rr["outcome"] == "unverified" and ev["leg"] == "compare"
                  and result["phoenix_restore"] == "UNVERIFIED" and m.restore_exit_code(result) == 3
                  and "OTHER-9" in rr["verify_reason"])
        if name in ("R3a", "R3b", "R3c"):
            want = {"R3a": "list_fallback", "R3b": "status_json", "R3c": "status_json"}[name]
            check(n + " attribution refused and the reason names the snapshot",
                  rr["outcome"] == "unverified" and ev["leg"] == "attribution"
                  and result["phoenix_restore"] == "UNVERIFIED"
                  and rr["pre_spawn_snapshot_source"] == want)
            if name == "R3b":
                check(n + " ★condition 1: rc 0 with empty stdout is NOT an enumerated snapshot",
                      rr["pre_spawn_snapshot_enumerated"] is False
                      and rr["pre_spawn_snapshot_known"] is True)
            if name == "R3c":
                check(n + " the measured pre-read equal to the pin outranks the snapshot",
                      ev["pre_read_here"] is True and ev["pane_is_new"] is False
                      and rr["registered_before"] == D1_PIN)
        if name == "R4":
            check(n + " codex seat: seat_only, never verified, and the enum says so",
                  rr["outcome"] == "seat_only" and result["phoenix_restore"] == "VERIFIED_SEAT_ONLY"
                  and m.restore_exit_code(result) == 0
                  and result["seat_only_roles"] == ["worker-1"] and ev["channel"] == "none")
            check(n + " and the honesty note names the unmeasured continuity",
                  "좌석만" in result["honesty_note"] or "seat_only" in result["honesty_note"])
        if name == "R5":
            check(n + " the expectation is frozen before the spawn (a mid-restore re-pin cannot move it)",
                  rr["expected_sid"] == D1_PIN
                  and json.load(open(topo_path, encoding="utf-8"))["entries"][0]["session_id"] == "REPINNED-9")
        if name == "R6":
            check(n + " grace exhausted with a stale stem: compare refuses and the full grace was spent",
                  rr["outcome"] == "unverified" and ev["leg"] == "compare"
                  and sleeps.count(m.PHOENIX_SESSION_GRACE_SLEEP) >= m.F1_REGISTERED_GRACE_TRIES - 1)
        return journal
    finally:
        m.cys, m.time.sleep, m._emit_evt = original_cys, original_sleep, original_emit
        m._ACTIVE_EPOCH, m._LAST_LIVENESS_KNOWN = original_epoch, original_known
        if original_pack is None:
            os.environ.pop("CYS_PACK_DIR", None)
        else:
            os.environ["CYS_PACK_DIR"] = original_pack
        m._AGENTS_CACHE.clear()
        shutil.rmtree(td)


# ──────────────────────────────────────────────────────────────────────────────
# D1xD3 — COMPOSED, TWO RUNS. defect 1 must not write a key that belongs to defect 3's marker group:
# `FRESH_MARKER_KEYS` contains "observed_sid_source" and the scope pass tests KEY PRESENCE, so a value of
# None counts. A key written outside the two marker writers carries no `fresh_epoch`, so on the NEXT run the
# scope pass strips it, sets `fresh_scope_stripped`, and `fresh_scope_guard_fires` pre-empts the defect-1
# verdict for the rest of time. Head ruling 1 (e578c42b) BC1. A single run cannot see this — hence two.
# ──────────────────────────────────────────────────────────────────────────────
def d1_composed(name):
    """`name`: C1 resumed claude over two boots · C2 codex over two boots · C3 ★the real same-boot retry
    (run 1 ends in a production-written unverified transient, run 2 recovers in the SAME epoch and must call
    defect 1's verdict a SECOND time) · C3d the dedup-only shape, kept and labelled as dedup (codex impl r1 T1:
    the old C3 ended run 1 verified, so run 2 dedup'd and the verdict ran exactly ONCE across both runs)."""
    td = tempfile.mkdtemp(prefix="phoenix-d1-composed-")
    original_cys, original_sleep, original_emit = m.cys, m.time.sleep, m._emit_evt
    original_epoch, original_known = m._ACTIVE_EPOCH, m._LAST_LIVENESS_KNOWN
    original_pack = os.environ.get("CYS_PACK_DIR")
    original_verdict = m._d1_resume_verdict
    try:
        # ★codex impl r1 T1: count the REAL verdict calls. A same-boot retry that never re-enters
        #   `_d1_resume_verdict` is cached evidence, not a measured retry — the count is what tells them apart.
        d1_calls = []
        def counting_verdict(*a, **k):
            out = original_verdict(*a, **k)
            d1_calls.append(out[0])
            return out
        m._d1_resume_verdict = counting_verdict
        socket_path = os.path.join(td, "cys.sock")
        ticket = "d1x-" + name                      # ONE ticket ⇒ ONE journal across both runs
        agent = "codex" if name == "C2" else "claude"
        entry = {"role": "worker-1", "agent": agent, "session_id": D1_PIN,
                 "claude_config_dir": os.path.join(td, "acct"), "cwd": os.path.join(td, "proj"),
                 "surface_id": 9, "surface_ref": "surface:9"}
        os.makedirs(entry["cwd"])
        project_dir = os.path.join(entry["claude_config_dir"], "projects",
                                   m._claude_project_component(entry["cwd"]))
        os.makedirs(project_dir)
        with open(os.path.join(project_dir, D1_PIN + ".jsonl"), "w", encoding="utf-8") as f:
            f.write("{}\n")
        pack = os.path.join(td, "pack")
        os.makedirs(pack)
        with open(os.path.join(pack, "agents.json"), "w", encoding="utf-8") as f:
            json.dump({"claude": {"resume_arg": "--resume {session_id}"}}, f)
        os.environ["CYS_PACK_DIR"] = pack
        m._AGENTS_CACHE.clear()
        with open(os.path.join(td, "topology.json"), "w", encoding="utf-8") as f:
            json.dump({"entries": [entry], "updated_at": 1.0, "schema_version": 1,
                       "tombstones": [], "tombstones_rev": 0}, f)
        journal_path = m.journal_path(socket_path, ticket)
        # ★C3: run 1 must end in a PRODUCTION-written unverified transient, so the hook has not registered
        #   yet on the first run; it registers before run 2, in the same boot generation.
        state = {"alive": False, "epoch": 1000.0, "sref": "surface:9",
                 "registered": None if (agent == "codex" or name == "C3") else D1_PIN,
                 "alive_from_status": None}

        def fake_cys(*args, socket=None, timeout=25):
            assert socket == socket_path, "CLI escaped the isolated socket"
            verb = args[0]
            if verb == "status":
                # ★C3 (codex impl r1 T1): a same-boot retry is a TRANSITION inside ONE generation, so this
                #   fake scripts liveness by status-call index instead of holding it constant — the pane is
                #   down when phoenix computes its targets and back up, registered, when the stages read it.
                state["n_status"] = state.get("n_status", 0) + 1
                alive = state["alive"] or (state.get("alive_from_status") is not None
                                           and state["n_status"] >= state["alive_from_status"])
                rows = []
                if alive:
                    rows = [{"surface_id": 9, "surface_ref": state["sref"], "role": "worker-1",
                             "exited": False, "agent_alive": True, "seat": "occupied", "agent": agent,
                             "gate_pending": None,
                             "registered_session_id": state["registered"]}]
                return SimpleNamespace(returncode=0, stdout=json.dumps(
                    {"daemon": {"started_at": state["epoch"]}, "surfaces": rows}), stderr="")
            if verb == "list":
                return SimpleNamespace(returncode=0, stdout="", stderr="")
            if verb == "restore":
                state["alive"] = True
                return SimpleNamespace(returncode=0, stdout="restored worker-1 " + state["sref"], stderr="")
            if verb == "reinject":
                return SimpleNamespace(returncode=0, stdout="디렉티브 생존 확인 (ACK 수신)", stderr="")
            if verb in ("read-screen", "watch"):
                return SimpleNamespace(returncode=0, stdout="bypass permissions on", stderr="")
            raise AssertionError("unexpected cys call: %r" % (args,))

        m.cys = fake_cys
        m.time.sleep = lambda seconds: None
        m._emit_evt = lambda *a, **k: None

        def one_run():
            state["n_status"] = 0                   # the scripted liveness index counts per run
            res = m.run_restore(socket_path, ticket=ticket, stub=False, no_breaker=True, print_result=False)
            with open(journal_path, encoding="utf-8") as f:
                return res, json.load(f)["roles"]["worker-1"]

        res1, rr1 = one_run()
        calls_after_run1 = len(d1_calls)
        n = "d1xd3 " + name
        want1 = "unverified" if name == "C3" else ("seat_only" if agent == "codex" else "verified")
        want_leg1 = {"C3": "observe"}.get(name, "channel" if agent == "codex" else "all")
        check(n + " run 1 is judged by defect 1", rr1["outcome"] == want1
              and (rr1.get("resume_evidence") or {}).get("leg") == want_leg1,
              extra="outcome=%r leg=%r want=%r/%r" % (
                  rr1["outcome"], (rr1.get("resume_evidence") or {}).get("leg"), want1, want_leg1))
        if name == "C3":
            check(n + " run 1 is a PRODUCTION-written unverified transient, not a fixture",
                  rr1["stages"]["verify"]["done"] is False
                  and rr1["observed_sid"] is None
                  and (rr1.get("resume_evidence") or {}).get("registered_now") is None
                  and "재개 미확정" in (rr1.get("verify_reason") or "")
                  and calls_after_run1 >= 1,
                  extra="done=%r observed=%r calls=%d reason=%.80s" % (
                      rr1["stages"]["verify"]["done"], rr1["observed_sid"], calls_after_run1,
                      rr1.get("verify_reason")))
        check(n + " run 1 leaves NO key of defect 3's marker group on a non-F-1 role",
              not any(k in rr1 for k in m.FRESH_MARKER_KEYS),
              extra="present: %s" % sorted(k for k in m.FRESH_MARKER_KEYS if k in rr1))

        # ── the second run: the role died again (a later boot, or the daemon's same-boot retry) ──
        state["alive"] = False
        if name in ("C1", "C2"):
            state["epoch"] = 2000.0                 # a NEW boot generation
            state["sref"] = "surface:11"            # a new pane, as a real restore would make
        if name == "C3":
            # ★the same-boot retry world — no new generation, no new pane. The pane phoenix created in run 1
            #   is DOWN when run 2 computes its targets (so the role is a target again, and it is not dedup'd:
            #   run 1 never finished verify), and it is back up WITH the hook's registration by the time the
            #   stages read it — precisely the race run 1 lost. Attribution still rests on run 1's pre/post
            #   snapshots in the journal, because the pane really was created by this restore, in this boot.
            #   The index is MEASURED, not guessed: phoenix consumes its liveness snapshot inside the first
            #   four status calls of a run, and 5..8 all produce this world, so 5 has slack. If a later change
            #   moves it, the assertions below (a target again · a SECOND counted verdict · a new observation)
            #   go red with their own diagnosis instead of silently testing nothing — which is how C3 died.
            state["alive_from_status"] = 5
            state["registered"] = D1_PIN            # the SessionStart hook has registered by now
        res2, rr2 = one_run()
        calls_in_run2 = len(d1_calls) - calls_after_run1
        if name == "C3":
            # ★codex T1 (i): a SECOND verdict invocation actually happened in the same epoch.
            check(n + " run 2 really re-ran the role — a target again in the SAME epoch, not a NOOP or a dedup",
                  res2.get("target_roles") == ["worker-1"] and state["epoch"] == 1000.0
                  and res2.get("phoenix_restore") != "NOOP",
                  extra="targets=%r epoch=%r verdict=%r" % (
                      res2.get("target_roles"), state["epoch"], res2.get("phoenix_restore")))
            check(n + " run 2 calls defect 1's verdict a SECOND time in the same epoch",
                  calls_in_run2 >= 1 and len(d1_calls) >= 2,
                  extra="calls run1=%d run2=%d total=%d outcomes=%r" % (
                      calls_after_run1, calls_in_run2, len(d1_calls), d1_calls))
            # (ii) a NEW observation — not run 1's cached None.
            check(n + " run 2 recovers on a NEW observation, not cached evidence",
                  rr2["observed_sid"] == D1_PIN
                  and (rr2.get("resume_evidence") or {}).get("registered_now") == D1_PIN
                  and rr2["stages"]["verify"]["done"] is True
                  and res2["phoenix_restore"] == "VERIFIED",
                  extra="observed=%r registered_now=%r verdict=%r" % (
                      rr2["observed_sid"], (rr2.get("resume_evidence") or {}).get("registered_now"),
                      res2["phoenix_restore"]))
            # (iv) and the run-2 verdict comes from defect 1's legs, not from defect 3's guard.
            check(n + " run 2's verdict comes from defect 1's legs (leg=all), not the D3 guard",
                  rr2["outcome"] == "verified"
                  and (rr2.get("resume_evidence") or {}).get("leg") == "all"
                  and (rr2.get("resume_evidence") or {}).get("pane_is_new") is True
                  and "D3:" not in (rr2.get("verify_reason") or "")
                  and "재개 확정" in (rr2.get("verify_reason") or ""),
                  extra="outcome=%r leg=%r reason=%.90s" % (
                      rr2["outcome"], (rr2.get("resume_evidence") or {}).get("leg"),
                      rr2.get("verify_reason")))
        elif name == "C3d":
            # ★labelled DEDUP, kept on purpose: this is the shape codex found vacuous. Run 1 ends verified,
            #   so production dedup skips the role and the verdict runs exactly ONCE across both runs. The row
            #   is evidence of the dedup contract — never of a same-boot retry verdict.
            check(n + " DEDUP: run 1 verified ⇒ run 2 reuses it and the verdict runs exactly ONCE",
                  len(d1_calls) == 1 and calls_in_run2 == 0
                  and rr2["outcome"] == "verified" and rr2["stages"]["verify"]["done"] is True,
                  extra="calls=%d (run2=%d) outcome=%r" % (len(d1_calls), calls_in_run2, rr2["outcome"]))
        else:
            check(n + " run 2 is still judged by defect 1, not pre-empted by the D3 guard",
                  rr2["outcome"] == want1 and "D3:" not in (rr2.get("verify_reason") or "")
                  and calls_in_run2 >= 1,
                  extra="run2 outcome=%r calls=%d reason=%.90s" % (
                      rr2["outcome"], calls_in_run2, rr2.get("verify_reason")))
        # (iii) no key of defect 3's marker group, and no strip, on any of the composed shapes.
        check(n + " run 2 has no fresh_scope_stripped on this role",
              "fresh_scope_stripped" not in rr2 and not (res2.get("fresh_scope_stripped") or {}),
              extra="stripped=%r" % (rr2.get("fresh_scope_stripped"),))
        check(n + " the observation provenance is still reported, under a defect-1 key",
              "resume_observed_source" in rr2 and "observed_sid_source" not in rr2
              and (rr2.get("resume_evidence") or {}).get("observed_sid_source")
              == ("registered" if agent == "claude" else None),
              extra="resume_observed_source=%r evidence=%r" % (
                  rr2.get("resume_observed_source"),
                  (rr2.get("resume_evidence") or {}).get("observed_sid_source")))
        return rr1, rr2
    finally:
        m.cys, m.time.sleep, m._emit_evt = original_cys, original_sleep, original_emit
        m._ACTIVE_EPOCH, m._LAST_LIVENESS_KNOWN = original_epoch, original_known
        if original_pack is None:
            os.environ.pop("CYS_PACK_DIR", None)
        else:
            os.environ["CYS_PACK_DIR"] = original_pack
        m._AGENTS_CACHE.clear()
        m._d1_resume_verdict = original_verdict
        shutil.rmtree(td)


def main():
    _results.clear()
    if os.name == "nt":
        print("SKIP run_restore scenarios: Windows named-pipe state mapping is not a temp socket dirname")
        return 0
    for name in ("S1", "S2", "S3", "S4", "S5"):
        scenario(name)
    for name in ("R1", "R2", "R3a", "R3b", "R3c", "R4", "R5", "R6"):
        d1_scenario(name)
    for name in ("C1", "C2", "C3", "C3d"):
        d1_composed(name)
    npass = sum(1 for c in _results if c)
    print("\n=== %d/%d PASS ===" % (npass, len(_results)))
    return 0 if npass == len(_results) else 1


if __name__ == "__main__":
    sys.exit(main())
