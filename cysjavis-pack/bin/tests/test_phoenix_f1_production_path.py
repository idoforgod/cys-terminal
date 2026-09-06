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
def check(name, cond):
    _results.append(cond); print(("PASS " if cond else "FAIL ") + name)


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
                assert args[6:] in (("--timeout", "6"), ("--timeout", "4"))
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
        if name in ("S3", "S4"):
            check(name + " same-run reobserve", len(reobserve) == 1 and "changed" in reobserve[0]["msg"])
            # Distinguish stage reinject from G2's own reinject --check call.
            check(name + " recollects injection and G2", sum(args[-1] == "6" for verb, args, socket, timeout in calls
                  if verb == "reinject") == 2 and verbs.count("reinject") == 4)
            retry = snapshots[2]["roles"]["worker-1"]
            check(name + " invalidated old evidence before retry", "reinject_sid" not in retry
                  and retry["stages"]["reinject"]["done"] is False
                  and retry["stages"]["g2_ack"]["done"] is False
                  and retry["stages"]["verify"]["done"] is False)
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


def main():
    _results.clear()
    if os.name == "nt":
        print("SKIP run_restore scenarios: Windows named-pipe state mapping is not a temp socket dirname")
        return 0
    for name in ("S1", "S2", "S3", "S4"):
        scenario(name)
    npass = sum(1 for c in _results if c)
    print("\n=== %d/%d PASS ===" % (npass, len(_results)))
    return 0 if npass == len(_results) else 1


if __name__ == "__main__":
    sys.exit(main())
