//! Track T3 daemon tests — the role table, the verbs, the locks, intake, caller identity.
//! Every test builds its own ledger root under the temporary directory (no environment variable is
//! set, no real ledger, no daemon socket); `handle()` is exercised through `tmp_daemon`, whose root is
//! the per-process temporary root of `production_root` in this binary's tests.

use super::ledger_verbs::{release_seat, COMMIT_CHECK_BUDGET_SECS};
use super::*;
use cys::repair::boot_critical::CheckKind;
use cys::repair::ledger::{self, ChangeKind, FileChange, PreImage, RepairRecord};
use cys::repair::roles::{CallerKind, VerbClass, VERB_CLASSES};
use cys::repair::types::{
    Classification, LaneState, LockOwner, RepairId, RepairState, TargetState,
};
use serde_json::json;
use std::sync::atomic::{AtomicU64, Ordering};

struct Rig {
    root: PathBuf,
    pack: PathBuf,
    registry: PathBuf,
    mutexes: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    lane: LaneKey,
    inc: String,
}

static SEQ: AtomicU64 = AtomicU64::new(0);

fn rig(tag: &str) -> Rig {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("cys-t3-{tag}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let pack = dir.join("pack");
    std::fs::create_dir_all(pack.join("bin")).unwrap();
    std::fs::create_dir_all(pack.join("hooks")).unwrap();
    std::fs::write(pack.join("bin/a.py"), b"print('a')\n").unwrap();
    std::fs::write(pack.join("hooks/h.sh"), b"#!/bin/sh\necho h\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            pack.join("bin/a.py"),
            std::fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        std::fs::set_permissions(
            pack.join("hooks/h.sh"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
    }
    Rig {
        root: dir.join("local").join("repairs"),
        pack,
        registry: dir.join("depts.json"),
        mutexes: Mutex::new(HashMap::new()),
        lane: LaneKey("base".into()),
        inc: "inc-1".into(),
    }
}

fn general(rel: &str) -> Result<Classification, RepairError> {
    let _ = rel;
    Ok(Classification::General)
}

/// `hooks/*` is boot-critical, everything else general (a stand-in for T5a's list).
fn hooks_boot(rel: &str) -> Result<Classification, RepairError> {
    Ok(if rel.starts_with("hooks/") {
        Classification::BootCritical
    } else {
        Classification::General
    })
}

fn pass(_rel: &str, _live: &Path, _d: std::time::Duration) -> Result<CheckReport, RepairError> {
    Ok(CheckReport {
        passed: true,
        timed_out: false,
        failure: String::new(),
        check_ms: 3,
        reference_kind: CheckKind::PyCompile,
        reference_ms: 2,
    })
}

impl Rig {
    fn ctx<'a>(
        &'a self,
        now: f64,
        classify: &'a dyn Fn(&str) -> Result<Classification, RepairError>,
        check: &'a dyn Fn(&str, &Path, std::time::Duration) -> Result<CheckReport, RepairError>,
    ) -> Ctx<'a> {
        Ctx {
            root: self.root.clone(),
            lane: self.lane.clone(),
            incarnation: self.inc.clone(),
            pack_dir: self.pack.clone(),
            now,
            windows: false,
            registry: self.registry.clone(),
            mutexes: &self.mutexes,
            seats: vec![(7, "reviewer-codex".into()), (8, "reviewer-gemini".into())],
            classify,
            check,
        }
    }
    fn call(&self, now: f64, who: &Caller, sub: &str, p: Value) -> Result<Value, VerbError> {
        handle_with(&self.ctx(now, &general, &pass), who, sub, &p)
    }
    fn record_of(&self, id: &str) -> RepairRecord {
        ledger::read_record_in(&self.root, &RepairId::parse(id).unwrap())
            .unwrap()
            .unwrap()
    }
}

fn seat(sid: u64, role: &str) -> Caller {
    Caller {
        kind: CallerKind::Seat { sid },
        role: Some(role.into()),
        app_token: false,
    }
}
fn human() -> Caller {
    Caller {
        kind: CallerKind::Human,
        role: None,
        app_token: false,
    }
}
fn app() -> Caller {
    Caller {
        kind: CallerKind::Human,
        role: None,
        app_token: true,
    }
}
fn detached() -> Caller {
    Caller {
        kind: CallerKind::Detached,
        role: None,
        app_token: false,
    }
}

const T0: f64 = 1_800_000_000.0;

/// begin → record → verify (other seat) → commit for `bin/a.py`; returns the id.
fn committed_general(r: &Rig) -> String {
    let w = seat(3, "worker");
    let b = r
        .call(T0, &w, "begin", json!({"files": ["bin/a.py"]}))
        .unwrap();
    let id = b["repair"].as_str().unwrap().to_string();
    std::fs::write(r.pack.join("bin/a.py"), b"print('fixed')\n").unwrap();
    r.call(
        T0 + 1.0,
        &w,
        "record",
        json!({"repair": id, "note": "fix print"}),
    )
    .unwrap();
    r.call(
        T0 + 2.0,
        &seat(7, "reviewer-codex"),
        "attest_verify",
        json!({"repair": id, "verdict": "pass", "note": "repro gone"}),
    )
    .unwrap();
    let c = r
        .call(T0 + 3.0, &w, "commit", json!({"repair": id}))
        .unwrap();
    assert_eq!(c["state"], "probation", "{c}");
    id
}

// ───────────────────────────── role table (J1 · O2 · N2 · B2 · F10 · M4) ─────────────────────────────

/// J1 census: every verb × every caller kind. Deny by default; READ/INTAKE/TRIGGER any caller (O2);
/// OWNER-DECISION master only; OWNER-APPROVAL only the app's token; REVIEW only the matching slot;
/// WORK only a worker seat (revert/cleanup also a human; Windows CLI human refused — M4).
#[test]
fn role_table_census_every_verb_every_caller() {
    let callers: Vec<(&str, Caller)> = vec![
        ("master", seat(1, "master")),
        ("ceo", seat(2, "ceo")),
        ("cso", seat(4, "cso")),
        ("cso-2", seat(5, "cso-2")),
        ("reviewer-codex", seat(7, "reviewer-codex")),
        ("reviewer-claude-2", seat(9, "reviewer-claude-2")),
        ("reviewer-gemini", seat(8, "reviewer-gemini")),
        ("reviewer-claude-1", seat(10, "reviewer-claude-1")),
        ("planner", seat(11, "planner")),
        (
            "roleless",
            Caller {
                kind: CallerKind::Seat { sid: 12 },
                role: None,
                app_token: false,
            },
        ),
        ("worker", seat(3, "worker")),
        ("worker-3", seat(13, "worker-3")),
        ("human", human()),
        ("app", app()),
        ("detached", detached()),
    ];
    let mut checked = 0;
    for (verb, class) in VERB_CLASSES {
        for (name, c) in &callers {
            let got = role_check(verb, c, false).is_ok();
            let want = match class {
                VerbClass::Read | VerbClass::Intake | VerbClass::Trigger => true,
                VerbClass::OwnerDecision => *name == "master",
                VerbClass::OwnerApproval => *name == "app",
                VerbClass::Review if verb == "attest_verify" => {
                    matches!(*name, "reviewer-codex" | "reviewer-claude-2")
                }
                VerbClass::Review => matches!(*name, "reviewer-gemini" | "reviewer-claude-1"),
                VerbClass::Work => {
                    matches!(*name, "worker" | "worker-3")
                        || (matches!(verb, "revert" | "cleanup")
                            && matches!(*name, "human" | "app"))
                }
            };
            assert_eq!(got, want, "{verb} ({class:?}) for {name}");
            if !got {
                assert_eq!(role_check(verb, c, false).unwrap_err().code, "acl_denied");
            }
            checked += 1;
        }
    }
    assert_eq!(checked, 28 * callers.len());
    // a verb not in the table is refused for every caller, the master included
    for (_, c) in &callers {
        assert_eq!(
            role_check("no-such-verb", c, false).unwrap_err().code,
            "acl_denied"
        );
    }
    // M4 Windows: the seatless command line is not a human there; the app button is
    for verb in ["revert", "cleanup"] {
        let e = role_check(verb, &human(), true).unwrap_err();
        assert_eq!(e.code, "acl_denied");
        assert_eq!(e.message, WINDOWS_SEATLESS_REFUSAL_KO);
        assert!(role_check(verb, &app(), true).is_ok());
        assert!(
            role_check(verb, &seat(3, "worker"), true).is_ok(),
            "a ticketed worker is unchanged"
        );
    }
}

/// M1: no request field names a seat or a role — a master asking with `role: worker` is still the
/// master; the production code reads no role from the environment or from the seat-token path.
#[test]
fn request_fields_and_env_never_name_a_role() {
    let r = rig("m1");
    for p in [
        json!({"files": ["bin/a.py"], "role": "worker", "seat": 3, "surface_id": 3, "caller": "worker"}),
        json!({"files": ["bin/a.py"], "CYS_ROLE": "worker", "seat_token": "x"}),
    ] {
        let e = r.call(T0, &seat(1, "master"), "begin", p).unwrap_err();
        assert_eq!(e.code, "acl_denied");
    }
    assert!(!r.root.exists(), "a refused begin wrote something");
    let src = [include_str!("../repair.rs")];
    let needles = [
        "env::var",
        "CYS_ROLE",
        "CYS_SEAT_TOKEN",
        "find_surface_by_seat_token",
        "params.get(\"role\")",
        "params.get(\"seat\")",
    ];
    // positive control: the checker sees the needle in a synthetic line
    assert!(needles
        .iter()
        .any(|n| "let r = std::env::var(\"CYS_ROLE\");".contains(n)));
    for text in src {
        let prod = &text[..text.find("#[cfg(test)]\nmod tests").unwrap_or(text.len())];
        for n in needles {
            assert!(
                !prod.contains(n),
                "production code of the repair module reads `{n}`"
            );
        }
    }
}

/// N2 / O2: a detached caller (a scheduler child, no seat, no terminal) may read, report and
/// trigger; every WORK / REVIEW / OWNER verb is refused.
#[test]
fn detached_caller_reads_reports_triggers_only() {
    let r = rig("n2");
    assert!(r.call(T0, &detached(), "status", json!({})).is_ok());
    assert!(r.call(T0, &detached(), "list", json!({})).is_ok());
    assert!(r
        .call(
            T0,
            &detached(),
            "report",
            json!({"file": "bin/a.py", "error": "E: x"})
        )
        .is_ok());
    for verb in ["tick", "sweep", "approval_list"] {
        // accepted by the role table, the body is another track's
        assert_eq!(
            r.call(T0, &detached(), verb, json!({})).unwrap_err().code,
            "repair_not_implemented",
            "{verb}"
        );
    }
    for verb in [
        "begin",
        "record",
        "commit",
        "submit",
        "revert",
        "cleanup",
        "recheck",
        "reapply",
        "attest_verify",
        "attest_audit",
        "grant",
        "owner_token",
        "approval_start",
    ] {
        assert_eq!(
            r.call(T0, &detached(), verb, json!({"repair": "R-1"}))
                .unwrap_err()
                .code,
            "acl_denied",
            "{verb}"
        );
    }
    // the app (operator token, no seat) reads and reports too
    assert!(r.call(T0, &app(), "status", json!({})).is_ok());
    assert!(r
        .call(
            T0,
            &app(),
            "report",
            json!({"file": "bin/a.py", "error": "E: y"})
        )
        .is_ok());
}

// ───────────────────────────── intake (F1 · G4 · own quota) ─────────────────────────────

/// D:1095(7) flood test: the same failure 1,000× → ONE ledger row (count 1,000); nothing reaches a
/// queue, the alert route or the event bus (seat-death / context alerts cannot be delayed by it).
#[test]
fn intake_flood_writes_one_row_and_touches_no_queue_or_alert_quota() {
    let daemon = crate::team_gate_tests::tmp_daemon("t3-flood", false);
    let alert_before = format!(
        "{:?}",
        *daemon.alert_route.lock().unwrap_or_else(|e| e.into_inner())
    );
    let bus_before = daemon.bus.latest_seq();
    for i in 0..1000 {
        let Reply::Single(v) = handle(
            &daemon,
            "report",
            &json!({"file": "bin/javis_x.py", "error": format!("Traceback: KeyError 'k' (pid {})", 40_000 + i)}),
            &json!(i),
            None,
            false,
            None,
        ) else {
            panic!("single reply")
        };
        assert_eq!(v["ok"], true, "{v}");
    }
    let root = production_root(&daemon);
    let lane = LaneKey::of_socket(&daemon.socket_path);
    let rows: Vec<_> = std::fs::read_dir(root.join("intake").join(&lane.0))
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    assert_eq!(rows.len(), 1, "1,000 identical reports must be ONE row");
    let row: serde_json::Value =
        serde_json::from_slice(&std::fs::read(rows[0].path()).unwrap()).unwrap();
    assert_eq!(row["count"], 1000);
    assert_eq!(
        daemon.bus.latest_seq(),
        bus_before,
        "intake published on the event bus"
    );
    assert_eq!(
        format!(
            "{:?}",
            *daemon.alert_route.lock().unwrap_or_else(|e| e.into_inner())
        ),
        alert_before,
        "intake touched the alert route"
    );
    assert!(daemon
        .surfaces
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .is_empty());
    // the guard rails the flood must not weaken (appendix A 1-10; queue cap 100 untouched)
    assert_eq!(crate::alert_route::HOURLY_CAP, 20);
    assert_eq!(crate::alert_route::CLEAR_RESERVE, 26);
    assert_eq!(crate::alert_route::CSO_QUEUE_HEADROOM, 50);
    let _ = std::fs::remove_dir_all(&root);
}

/// Structural pin: the repair module's production code never reaches the alert route, a queue, a
/// pane's input or the event bus (positive control: the scan sees a planted needle).
#[test]
fn intake_path_never_reaches_alert_route_or_queues() {
    let needles = [
        "alert_route::",
        "enqueue",
        "bus.publish",
        "send_input",
        "push_queue",
        "pending_queue",
        "javis_wakeup",
    ];
    let planted = "daemon.bus.publish(\"x\", \"y\", None, v);";
    assert!(
        needles.iter().any(|n| planted.contains(n)),
        "checker is blind"
    );
    for (name, text) in [
        ("repair.rs", include_str!("../repair.rs")),
        ("intake.rs", include_str!("../../../repair/intake.rs")),
    ] {
        let prod = &text[..text.find("#[cfg(test)]\nmod tests").unwrap_or(text.len())];
        for n in needles {
            assert!(!prod.contains(n), "{name} production code contains `{n}`");
        }
    }
}

/// D:658 / appendix A 1-26, 1-28: no per-call intake in the hooks' common frame or the observing hook.
#[test]
fn no_per_call_intake_in_hook_lib() {
    let lib = include_str!("../../../../cysjavis-pack/hooks/_lib.sh");
    let obs = include_str!("../../../../cysjavis-pack/hooks/cys-hook.sh");
    assert!(lib.len() > 500 && obs.len() > 100);
    for (n, t) in [("_lib.sh", lib), ("cys-hook.sh", obs)] {
        assert!(!t.contains("repair report"), "{n} reports intake per call");
        assert!(!t.contains("repair.report"), "{n} reports intake per call");
    }
}

/// G4 + quota + exclusion through the verb; a later report of a defect an open repair holds joins it.
#[test]
fn report_aggregates_excludes_and_joins() {
    let r = rig("intake");
    let w = seat(3, "worker");
    let a = r
        .call(
            T0,
            &w,
            "report",
            json!({"file": "bin/a.py", "error": "ValueError: bad"}),
        )
        .unwrap();
    assert_eq!(a["accepted"], true);
    assert_eq!(a["count"], 1);
    assert!(a["joined"].is_null());
    let b = r
        .call(
            T0,
            &seat(4, "cso"),
            "report",
            json!({"file": "./bin/a.py", "error": "ValueError: bad"}),
        )
        .unwrap();
    assert_eq!(b["count"], 2);
    assert_eq!(a["signature"], b["signature"]);
    assert_eq!(
        r.call(
            T0,
            &w,
            "report",
            json!({"file": "/etc/passwd", "error": "x"})
        )
        .unwrap_err()
        .code,
        "invalid_params"
    );
    assert_eq!(
        r.call(T0, &w, "report", json!({"file": "bin/a.py"}))
            .unwrap_err()
            .code,
        "invalid_params"
    );
    // begin on that file takes the open defect in; a later report says it joined
    let begun = r
        .call(T0, &w, "begin", json!({"files": ["bin/a.py"]}))
        .unwrap();
    let id = begun["repair"].as_str().unwrap().to_string();
    assert_eq!(begun["intake_signatures"], json!([a["signature"]]));
    let c = r
        .call(
            T0,
            &w,
            "report",
            json!({"file": "bin/a.py", "error": "ValueError: bad"}),
        )
        .unwrap();
    assert_eq!(c["joined"], json!(id));
    let l = r.call(T0, &human(), "list", json!({})).unwrap();
    assert_eq!(
        l["open_defects"], 0,
        "a defect held by an open repair is not open"
    );
    // Windows: the by-design cys-dept 127 is excluded
    let mut ctx = r.ctx(T0, &general, &pass);
    ctx.windows = true;
    let x = handle_with(
        &ctx,
        &w,
        "report",
        &json!({"file": "bin/javis_teamtoken.py", "error": "cys-dept: command not found (127)"}),
    )
    .unwrap();
    assert_eq!(x["accepted"], false);
    assert!(x["excluded"].is_string());
}

// ───────────────────────────── the general flow (C1 1.9 / 1.10 / N1 / U2 / K1) ─────────────────────────────

/// The whole general path, and K1: commit writes this lane's own row only; status counts every
/// registered lane, a lane without a row shows 대기 and "전파 완료" is withheld.
#[test]
fn general_flow_commit_and_lane_rows() {
    let r = rig("flow");
    std::fs::write(r.pack.join(".pack-version"), b"0.15.0\n").unwrap();
    let id = committed_general(&r);
    let rec = r.record_of(&id);
    assert_eq!(rec.state, RepairState::Probation);
    assert!(rec.lock.is_none(), "commit releases its own lock");
    assert!(rec.audit_due_at.is_some());
    assert_eq!(rec.committed.len(), 1);
    assert!(rec.committed[0].1.starts_with("file:"));
    let lanes_dir = r.root.join(&id).join("lanes");
    let names: Vec<String> = std::fs::read_dir(&lanes_dir)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    assert_eq!(names, ["base.json"], "commit wrote another lane's file");
    let row = ledger::read_lane_in(&r.root, &rec.id, &r.lane)
        .unwrap()
        .unwrap();
    assert_eq!(row.state, LaneState::Verified);
    let prob = row.probation.unwrap();
    assert_eq!(prob.pack_generation, "0.15.0");
    assert_eq!(prob.expected, rec.committed);
    assert_eq!(
        prob.check_timings.len(),
        1,
        "K4: durations recorded for every target"
    );
    // registry missing = HQ alone → complete
    let s = r
        .call(T0 + 4.0, &human(), "status", json!({"repair": id}))
        .unwrap();
    assert_eq!(s["propagation_complete"], true, "{s}");
    // a registered department without a row → 대기, withheld
    std::fs::write(
        &r.registry,
        br#"{"depts":{"d1":{"socket":"/h/.cys/cys-dept-d1/cys.sock"}}}"#,
    )
    .unwrap();
    let s = r
        .call(T0 + 4.0, &human(), "status", json!({"repair": id}))
        .unwrap();
    assert_eq!(s["propagation_complete"], false);
    let lanes = s["lanes"].as_object().unwrap();
    assert_eq!(lanes.len(), 2);
    assert!(lanes.values().any(|v| v == "대기"));
    // malformed registry → unknown, never declared
    std::fs::write(&r.registry, b"{broken").unwrap();
    let s = r
        .call(T0 + 4.0, &human(), "status", json!({"repair": id}))
        .unwrap();
    assert!(s["propagation_complete"].is_null());
    assert!(s["propagation_note"].as_str().unwrap().contains("등록부"));
    // 65 departments → 66 lanes listed
    let mut depts = serde_json::Map::new();
    for i in 1..=65 {
        depts.insert(
            format!("d{i}"),
            json!({"socket": format!("/h/.cys/cys-dept-d{i}/cys.sock")}),
        );
    }
    std::fs::write(
        &r.registry,
        serde_json::to_vec(&json!({"depts": depts})).unwrap(),
    )
    .unwrap();
    let s = r
        .call(T0 + 4.0, &human(), "status", json!({"repair": id}))
        .unwrap();
    assert_eq!(s["lanes"].as_object().unwrap().len(), 66);
    assert_eq!(s["propagation_complete"], false);
    // the audit after commit (reviewer2, another seat) keeps it in force
    let a = r
        .call(
            T0 + 5.0,
            &seat(8, "reviewer-gemini"),
            "attest_audit",
            json!({"repair": id, "verdict": "ok", "note": "ledger matches"}),
        )
        .unwrap();
    assert!(a["reopened"].is_null());
    assert_eq!(r.record_of(&id).state, RepairState::Probation);
}

/// C1 1.9 / N1 / U2: commit is refused without reviewer1's verification, with a verification from the
/// fixer's seat, after a content change, after a mode-only change, and from a seat without the lock.
#[test]
fn commit_preconditions() {
    let r = rig("commitpre");
    let w = seat(3, "worker");
    let id = r
        .call(T0, &w, "begin", json!({"files": ["bin/a.py"]}))
        .unwrap()["repair"]
        .as_str()
        .unwrap()
        .to_string();
    std::fs::write(r.pack.join("bin/a.py"), b"print('b')\n").unwrap();
    r.call(T0 + 1.0, &w, "record", json!({"repair": id, "note": "n"}))
        .unwrap();
    let e = r
        .call(T0 + 2.0, &w, "commit", json!({"repair": id}))
        .unwrap_err();
    assert_eq!(e.code, "repair_precondition");
    assert!(e.message.contains("리뷰어1"), "{}", e.message);
    // a verification by the fixer's own seat (planted: the role table would refuse the call itself)
    let mut rec = r.record_of(&id);
    let mut att = rec.worker_record.clone().unwrap();
    att.role = "reviewer-codex".into();
    att.verdict = "pass".into();
    rec.verify = Some(att);
    ledger::write_record_in(&r.root, &rec).unwrap();
    let e = r
        .call(T0 + 3.0, &w, "commit", json!({"repair": id}))
        .unwrap_err();
    assert!(e.message.contains("고친 좌석"), "{}", e.message);
    // a proper verification, then a content change → refused
    r.call(
        T0 + 4.0,
        &seat(7, "reviewer-codex"),
        "attest_verify",
        json!({"repair": id, "verdict": "pass", "note": "ok"}),
    )
    .unwrap();
    std::fs::write(r.pack.join("bin/a.py"), b"print('c')\n").unwrap();
    assert_eq!(
        r.call(T0 + 5.0, &w, "commit", json!({"repair": id}))
            .unwrap_err()
            .code,
        "target_changed"
    );
    std::fs::write(r.pack.join("bin/a.py"), b"print('b')\n").unwrap();
    // a mode-only change (same bytes, exec bit) → refused (U2)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            r.pack.join("bin/a.py"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        assert_eq!(
            r.call(T0 + 6.0, &w, "commit", json!({"repair": id}))
                .unwrap_err()
                .code,
            "target_changed"
        );
        std::fs::set_permissions(
            r.pack.join("bin/a.py"),
            std::fs::Permissions::from_mode(0o644),
        )
        .unwrap();
    }
    // another worker seat without the repair's lock → refused (P4)
    assert_eq!(
        r.call(
            T0 + 7.0,
            &seat(13, "worker-3"),
            "commit",
            json!({"repair": id})
        )
        .unwrap_err()
        .code,
        "repair_locked"
    );
    assert_eq!(
        r.call(
            T0 + 7.0,
            &seat(13, "worker-3"),
            "record",
            json!({"repair": id, "note": "x"})
        )
        .unwrap_err()
        .code,
        "repair_locked"
    );
    // and the real holder commits
    assert_eq!(
        r.call(T0 + 8.0, &w, "commit", json!({"repair": id}))
            .unwrap()["state"],
        "probation"
    );
}

/// N1: a later `record` voids the attestations; a failed verification opens round 2.
#[test]
fn record_voids_attestations_and_counts_rounds() {
    let r = rig("rounds");
    let w = seat(3, "worker");
    let id = r
        .call(T0, &w, "begin", json!({"files": ["bin/a.py"]}))
        .unwrap()["repair"]
        .as_str()
        .unwrap()
        .to_string();
    r.call(
        T0 + 1.0,
        &w,
        "record",
        json!({"repair": id, "note": "try 1"}),
    )
    .unwrap();
    r.call(
        T0 + 2.0,
        &seat(7, "reviewer-codex"),
        "attest_verify",
        json!({"repair": id, "verdict": "fail", "note": "still broken"}),
    )
    .unwrap();
    let rec = r.record_of(&id);
    assert_eq!(rec.attempt, 1);
    let out = r
        .call(
            T0 + 3.0,
            &w,
            "record",
            json!({"repair": id, "note": "try 2"}),
        )
        .unwrap();
    assert_eq!(out["new_round"], true);
    assert_eq!(out["attestations_voided"], true);
    let rec = r.record_of(&id);
    assert_eq!(rec.attempt, 2);
    assert!(rec.verify.is_none() && rec.audit.is_none());
    assert_eq!(rec.methods, ["try 1", "try 2"]);
    // a verification on bytes other than the worker recorded is refused (record again)
    std::fs::write(r.pack.join("bin/a.py"), b"print('later edit')\n").unwrap();
    assert_eq!(
        r.call(
            T0 + 4.0,
            &seat(7, "reviewer-codex"),
            "attest_verify",
            json!({"repair": id, "verdict": "pass", "note": "x"})
        )
        .unwrap_err()
        .code,
        "target_changed"
    );
    // the round cap (C1 1.11): round 11 is refused
    let mut rec = r.record_of(&id);
    rec.attempt = 10;
    rec.verify = Some(
        rec.worker_record
            .clone()
            .map(|mut a| {
                a.verdict = "fail".into();
                a.seat = 7;
                a
            })
            .unwrap(),
    );
    ledger::write_record_in(&r.root, &rec).unwrap();
    let e = r
        .call(
            T0 + 5.0,
            &w,
            "record",
            json!({"repair": id, "note": "try 11"}),
        )
        .unwrap_err();
    assert!(e.message.contains("10"), "{}", e.message);
}

/// C1 1.10 / N1: an audit finding re-opens; an audit that sees committed bytes different from the
/// verified ones re-opens too; reviewer2 is never the verifier's seat nor the fixer.
#[test]
fn audit_finding_reopens() {
    let r = rig("audit");
    let id = committed_general(&r);
    // not both by one seat
    let e = r
        .call(
            T0 + 4.0,
            &seat(7, "reviewer-claude-1"),
            "attest_audit",
            json!({"repair": id, "verdict": "ok", "note": "x"}),
        )
        .unwrap_err();
    assert_eq!(e.code, "acl_denied");
    let a = r
        .call(
            T0 + 4.0,
            &seat(8, "reviewer-gemini"),
            "attest_audit",
            json!({"repair": id, "verdict": "finding", "note": "diff ≠ ledger"}),
        )
        .unwrap();
    assert_eq!(a["reopened"], "감사 지적");
    assert_eq!(r.record_of(&id).state, RepairState::ReopenedByAudit);

    let r2 = rig("audit2");
    let id2 = committed_general(&r2);
    std::fs::write(r2.pack.join("bin/a.py"), b"print('edited after commit')\n").unwrap();
    let a = r2
        .call(
            T0 + 4.0,
            &seat(8, "reviewer-gemini"),
            "attest_audit",
            json!({"repair": id2, "verdict": "ok", "note": "x"}),
        )
        .unwrap();
    assert!(a["reopened"]
        .as_str()
        .unwrap()
        .contains("검증 때와 다릅니다"));
    assert_eq!(r2.record_of(&id2).state, RepairState::ReopenedByAudit);
    // a general repair is audited after commit only
    let r3 = rig("audit3");
    let w = seat(3, "worker");
    let id3 = r3
        .call(T0, &w, "begin", json!({"files": ["bin/a.py"]}))
        .unwrap()["repair"]
        .as_str()
        .unwrap()
        .to_string();
    r3.call(T0, &w, "record", json!({"repair": id3, "note": "n"}))
        .unwrap();
    assert_eq!(
        r3.call(
            T0,
            &seat(8, "reviewer-gemini"),
            "attest_audit",
            json!({"repair": id3, "verdict": "ok", "note": "x"})
        )
        .unwrap_err()
        .code,
        "repair_precondition"
    );
}

// ───────────────────────────── locks (C1 1.12 · G6 · G7 · O5 · F2) ─────────────────────────────

/// F2: the attempt counter and the methods live in the ledger — after the seat is reaped (cleared)
/// another worker seat picks the repair up at round k, not 1.
#[test]
fn counters_survive_a_seat_clear() {
    let r = rig("clear");
    let w = seat(3, "worker");
    let id = r
        .call(T0, &w, "begin", json!({"files": ["bin/a.py"]}))
        .unwrap()["repair"]
        .as_str()
        .unwrap()
        .to_string();
    r.call(T0 + 1.0, &w, "record", json!({"repair": id, "note": "m1"}))
        .unwrap();
    r.call(
        T0 + 2.0,
        &seat(7, "reviewer-codex"),
        "attest_verify",
        json!({"repair": id, "verdict": "fail", "note": "no"}),
    )
    .unwrap();
    r.call(T0 + 3.0, &w, "record", json!({"repair": id, "note": "m2"}))
        .unwrap();
    assert_eq!(r.record_of(&id).attempt, 2);
    release_seat(&r.ctx(T0 + 4.0, &general, &pass), 3);
    assert!(r.record_of(&id).lock.is_none(), "reap released the lock");
    let p = r
        .call(
            T0 + 5.0,
            &seat(13, "worker-3"),
            "begin",
            json!({"files": ["bin/a.py"]}),
        )
        .unwrap();
    assert_eq!(p["begin"], "picked-up");
    assert_eq!(p["repair"], json!(id));
    let rec = r.record_of(&id);
    assert_eq!(rec.attempt, 2, "the cleared seat's successor resumes at k");
    assert_eq!(rec.methods, ["m1", "m2"]);
    assert_eq!(rec.lock.as_ref().unwrap().owner.seat, 13);
    // the [RESUME] reader sees the new holder
    let held = ledger::held_by_seat_in(&r.root, &r.lane, 13, T0 + 6.0)
        .unwrap()
        .unwrap();
    assert_eq!((held.id.as_str(), held.attempt), (id.as_str(), 2));
}

/// C1 1.12: a daemon restart releases its lane's locks (incarnation check at load); a seat reap
/// releases only that seat's locks; a refused holder never clears another's lock (G6); a CLI retry
/// of `begin` finds its own completed start.
#[test]
fn lock_releases_and_retries() {
    let r = rig("locks");
    let w = seat(3, "worker");
    let first = r
        .call(T0, &w, "begin", json!({"files": ["bin/a.py"]}))
        .unwrap();
    let id = first["repair"].as_str().unwrap().to_string();
    let again = r
        .call(T0 + 1.0, &w, "begin", json!({"files": ["bin/a.py"]}))
        .unwrap();
    assert_eq!(again["begin"], "already-held");
    assert_eq!(again["repair"], json!(id));
    assert_eq!(
        ledger::list_ids_in(&r.root).len(),
        1,
        "a retry created a second repair"
    );
    // another seat is refused and the holder's lock is untouched
    let e = r
        .call(
            T0 + 2.0,
            &seat(13, "worker-3"),
            "begin",
            json!({"files": ["bin/a.py"]}),
        )
        .unwrap_err();
    assert_eq!(e.code, "repair_locked");
    assert_eq!(r.record_of(&id).lock.unwrap().owner.seat, 3);
    // a reap of ANOTHER seat releases nothing
    release_seat(&r.ctx(T0 + 3.0, &general, &pass), 13);
    assert_eq!(r.record_of(&id).lock.unwrap().owner.seat, 3);
    // daemon restart: a new incarnation sees the old lock as released
    let mut restarted = rig("locks-unused");
    restarted.root = r.root.clone();
    restarted.pack = r.pack.clone();
    restarted.inc = "inc-2".into();
    let p = restarted
        .call(
            T0 + 4.0,
            &seat(13, "worker-3"),
            "begin",
            json!({"files": ["bin/a.py"]}),
        )
        .unwrap();
    assert_eq!(p["begin"], "picked-up");
    let rec = r.record_of(&id);
    assert_eq!(rec.lock.as_ref().unwrap().owner.incarnation, "inc-2");
    // and the old holder, asking the restarted daemon, is refused (the lock is another seat's now)
    assert_eq!(
        restarted
            .call(T0 + 5.0, &w, "record", json!({"repair": id, "note": "x"}))
            .unwrap_err()
            .code,
        "repair_locked"
    );
    // lease expiry: two hours later (+60 s grace) a third seat may pick it up
    let p = restarted
        .call(
            T0 + 4.0 + 7200.0 + 61.0,
            &seat(14, "worker"),
            "begin",
            json!({"files": ["bin/a.py"]}),
        )
        .unwrap();
    assert_eq!(p["begin"], "picked-up");
    // an expired own lock is never renewed (G7(c))
    let e = restarted
        .call(
            T0 + 4.0 + 7200.0 + 61.0 + 7200.0 + 1.0,
            &seat(14, "worker"),
            "record",
            json!({"repair": id, "note": "x"}),
        )
        .unwrap_err();
    assert_eq!(e.code, "repair_locked");
}

/// G6: two `begin`s for the same targets racing inside one daemon (a CLI retry during a slow first
/// start) give ONE repair — the second gets the first's completed start or `repair_locked`.
#[test]
fn concurrent_begins_make_one_repair() {
    let r = rig("race");
    let outcomes: Vec<Result<Value, VerbError>> = std::thread::scope(|s| {
        let hs: Vec<_> = (0..8)
            .map(|_| {
                s.spawn(|| {
                    r.call(
                        T0,
                        &seat(3, "worker"),
                        "begin",
                        json!({"files": ["bin/a.py"]}),
                    )
                })
            })
            .collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    assert_eq!(
        ledger::list_ids_in(&r.root).len(),
        1,
        "two repairs for one start"
    );
    let news = outcomes
        .iter()
        .filter(|o| o.as_ref().is_ok_and(|v| v["begin"] == "new"))
        .count();
    assert_eq!(news, 1);
    assert!(outcomes.iter().all(|o| match o {
        Ok(v) => v["begin"] == "new" || v["begin"] == "already-held",
        Err(e) => e.code == "repair_locked",
    }));
}

/// R5: picking an in-force repair up again sets 진행 중 before any `record`; P4: a worker's `revert`
/// passes the role table without needing, writing or clearing the repair's lock (the body is T6's).
#[test]
fn pick_up_of_in_force_and_revert_role() {
    let r = rig("inforce");
    let id = committed_general(&r);
    assert_eq!(r.record_of(&id).state, RepairState::Probation);
    let before_lock = r.record_of(&id).lock.clone();
    let e = r
        .call(
            T0 + 5.0,
            &seat(13, "worker-3"),
            "revert",
            json!({"repair": id}),
        )
        .unwrap_err();
    assert_eq!(e.code, "repair_not_implemented", "{}", e.message);
    assert_eq!(
        r.record_of(&id).lock,
        before_lock,
        "revert touched the lock"
    );
    let p = r
        .call(
            T0 + 6.0,
            &seat(13, "worker-3"),
            "begin",
            json!({"files": ["bin/a.py"]}),
        )
        .unwrap();
    assert_eq!(p["begin"], "picked-up");
    assert_eq!(r.record_of(&id).state, RepairState::InProgress);
    assert_eq!(
        r.record_of(&id).attempt,
        1,
        "a pick-up opens no round by itself"
    );
}

/// G7(c): a lock bound to a closed grant claim authorises no mutating verb.
#[test]
fn closed_claim_refuses_mutating_verbs() {
    let r = rig("claim");
    let w = seat(3, "worker");
    let id = r
        .call(T0, &w, "begin", json!({"files": ["bin/a.py"]}))
        .unwrap()["repair"]
        .as_str()
        .unwrap()
        .to_string();
    let mut rec = r.record_of(&id);
    rec.lock.as_mut().unwrap().claim = Some(5);
    ledger::write_record_in(&r.root, &rec).unwrap();
    assert!(r
        .call(T0 + 1.0, &w, "record", json!({"repair": id, "note": "ok"}))
        .is_ok());
    std::fs::create_dir_all(r.root.join("grant-seq")).unwrap();
    std::fs::write(r.root.join("grant-seq").join("5.closed"), b"").unwrap();
    let e = r
        .call(T0 + 2.0, &w, "record", json!({"repair": id, "note": "no"}))
        .unwrap_err();
    assert_eq!(e.code, "repair_locked");
    // renew-write happened before the marker check (G7 order): the lease was renewed
    assert_eq!(r.record_of(&id).lock.unwrap().expires_at, T0 + 2.0 + 7200.0);
}

// ───────────────────────────── boot-critical paths (R1 · T1 · U3 · V1 · V2) ─────────────────────────────

/// RULINGS T1 / R1: `--joins` (merge mode) is refused on a boot-critical path; `--resolves` makes an
/// ordinary lane-local repair R′ there with its own lock and leaves R's row untouched; the working
/// copy is printed; a pick-up never rewrites staging (V2(b)); a symlinked staging parent is refused.
#[test]
fn boot_critical_joins_refused_resolves_accepted() {
    let r = rig("bc");
    let w = seat(3, "worker");
    let ctx = r.ctx(T0, &hooks_boot, &pass);
    let b = handle_with(&ctx, &w, "begin", &json!({"files": ["hooks/h.sh"]})).unwrap();
    let rid = b["repair"].as_str().unwrap().to_string();
    assert_eq!(b["classification"], "boot_critical");
    let staging = b["staging"].as_str().unwrap().to_string();
    assert!(
        Path::new(&staging).join("hooks/h.sh").is_file(),
        "begin populates the working copy"
    );
    // a row of R in this lane in 충돌 (written as T7b would)
    let row = cys::repair::ledger::LaneRecord {
        schema: 1,
        repair: RepairId::parse(&rid).unwrap(),
        lane: r.lane.clone(),
        state: LaneState::Conflict,
        probation: None,
        merge: None,
        merge_history: vec![],
        approval_ref: None,
        conflict_live: vec![],
        applied: vec![],
        applied_history: vec![],
        merge_base: vec![],
        merge_lock: None,
        merge_attempt: 0,
        updated_at: T0,
    };
    ledger::write_lane_in(&r.root, &row).unwrap();
    let e = handle_with(
        &ctx,
        &seat(13, "worker-3"),
        "begin",
        &json!({"files": ["hooks/h.sh"], "joins": rid}),
    )
    .unwrap_err();
    assert_eq!(e.code, "repair_precondition");
    assert!(e.message.contains("--resolves"), "{}", e.message);
    // R′ via --resolves: accepted, own lock, R's row untouched
    let before = std::fs::read(ledger::lane_path_in(
        &r.root,
        &RepairId::parse(&rid).unwrap(),
        &r.lane,
    ))
    .unwrap();
    let rp = handle_with(
        &ctx,
        &seat(13, "worker-3"),
        "begin",
        &json!({"files": ["hooks/h.sh"], "resolves": rid}),
    )
    .unwrap();
    let rprime = rp["repair"].as_str().unwrap().to_string();
    assert_ne!(rprime, rid);
    assert_eq!(rp["resolves"], json!(rid));
    assert_eq!(r.record_of(&rprime).lock.unwrap().owner.seat, 13);
    assert_eq!(
        std::fs::read(ledger::lane_path_in(
            &r.root,
            &RepairId::parse(&rid).unwrap(),
            &r.lane
        ))
        .unwrap(),
        before
    );
    assert!(r.record_of(&rid).lock.is_some(), "R's own lock untouched");
    // the worker edits R's working copy; a pick-up by another seat leaves it as it is
    std::fs::write(
        Path::new(&staging).join("hooks/h.sh"),
        b"#!/bin/sh\necho fixed\n",
    )
    .unwrap();
    release_seat(&r.ctx(T0 + 1.0, &hooks_boot, &pass), 3);
    let p = handle_with(
        &r.ctx(T0 + 2.0, &hooks_boot, &pass),
        &seat(14, "worker"),
        "begin",
        &json!({"files": ["hooks/h.sh"]}),
    )
    .unwrap();
    assert_eq!(p["begin"], "picked-up");
    assert_eq!(
        std::fs::read(Path::new(&staging).join("hooks/h.sh")).unwrap(),
        b"#!/bin/sh\necho fixed\n"
    );
    // record reads the candidate from staging; commit is not the path for it
    let ctx2 = r.ctx(T0 + 3.0, &hooks_boot, &pass);
    let rec = handle_with(
        &ctx2,
        &seat(14, "worker"),
        "record",
        &json!({"repair": rid, "note": "fixed candidate"}),
    )
    .unwrap();
    assert!(rec["files"][0]["post"]
        .as_str()
        .unwrap()
        .starts_with("file:"));
    assert_eq!(
        handle_with(
            &ctx2,
            &seat(14, "worker"),
            "commit",
            &json!({"repair": rid})
        )
        .unwrap_err()
        .code,
        "repair_precondition"
    );
    // reviewer1 signs the candidate; a mode-only change of the staged file afterwards no longer matches
    handle_with(
        &ctx2,
        &seat(7, "reviewer-codex"),
        "attest_verify",
        &json!({"repair": rid, "verdict": "pass", "note": "ok"}),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            Path::new(&staging).join("hooks/h.sh"),
            std::fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        let e = handle_with(
            &ctx2,
            &seat(8, "reviewer-gemini"),
            "attest_audit",
            &json!({"repair": rid, "verdict": "ok", "note": "x"}),
        )
        .unwrap_err();
        assert_eq!(e.code, "target_changed");
        // a symlinked parent inside staging → refused
        std::fs::remove_dir_all(Path::new(&staging).join("hooks")).unwrap();
        std::os::unix::fs::symlink(r.pack.join("hooks"), Path::new(&staging).join("hooks"))
            .unwrap();
        let e = handle_with(
            &ctx2,
            &seat(14, "worker"),
            "record",
            &json!({"repair": rid, "note": "x"}),
        )
        .unwrap_err();
        assert_eq!(e.code, "repair_precondition");
    }
}

/// RULINGS V1: `--unregister-hook` — the six lifelines refused; while the compiled list is not
/// available (T5a) a protection hook cannot be told apart → refused (fail closed); together with a
/// boot-critical target → refused (two repairs); `--files` may be left out only with it (Z2(e)).
#[test]
fn unregister_hook_rules() {
    let r = rig("hooks");
    let w = seat(3, "worker");
    let e = r
        .call(
            T0,
            &w,
            "begin",
            json!({"unregister_hook": ["SessionStart:session-start.sh"]}),
        )
        .unwrap_err();
    assert!(e.message.contains("생명줄"), "{}", e.message);
    if cys::repair::boot_critical::entries().is_empty() {
        let e = r
            .call(T0, &w, "begin", json!({"unregister_hook": ["Stop:x.sh"]}))
            .unwrap_err();
        assert!(
            e.message.contains("보호장치 훅인지 가릴 수 없습니다"),
            "{}",
            e.message
        );
    }
    let ctx = r.ctx(T0, &hooks_boot, &pass);
    let e = handle_with(
        &ctx,
        &w,
        "begin",
        &json!({"files": ["hooks/h.sh"], "unregister_hook": ["Stop:x.sh"]}),
    )
    .unwrap_err();
    assert_eq!(e.code, "repair_precondition");
    assert!(e.message.contains("두 수리로 나누세요"), "{}", e.message);
    assert_eq!(
        r.call(T0, &w, "begin", json!({})).unwrap_err().code,
        "invalid_params"
    );
    for bad in [
        "Stop",
        "Stop:",
        ":x.sh",
        "Stop:../x.sh",
        "Stop:a/b.sh",
        "St op:x.sh",
    ] {
        assert_eq!(
            r.call(T0, &w, "begin", json!({"unregister_hook": [bad]}))
                .unwrap_err()
                .code,
            "invalid_params",
            "{bad}"
        );
    }
    // classification that cannot be decided → begin refused (fail closed), nothing written
    let undecided = |_: &str| -> Result<Classification, RepairError> {
        Err(RepairError::NotImplemented(
            "boot_critical::is_boot_critical",
        ))
    };
    let e = handle_with(
        &r.ctx(T0, &undecided, &pass),
        &w,
        "begin",
        &json!({"files": ["bin/a.py"]}),
    )
    .unwrap_err();
    assert_eq!(e.code, "repair_not_implemented");
    assert!(ledger::list_ids_in(&r.root).is_empty());
}

/// RULINGS Y1: until T6's `hook_lines_step` lands, a commit whose record has a hook line is refused
/// with the not-implemented sentence and the record is left exactly as before; no lane row appears.
#[test]
fn commit_with_hook_line_refused_while_step_is_stub() {
    let r = rig("hookcommit");
    let w = seat(3, "worker");
    let id = r
        .call(T0, &w, "begin", json!({"files": ["bin/a.py"]}))
        .unwrap()["repair"]
        .as_str()
        .unwrap()
        .to_string();
    let mut rec = r.record_of(&id);
    rec.files.push(FileChange {
        rel: "settings:Stop:x.sh".into(),
        kind: ChangeKind::HookUnregister {
            event: "Stop".into(),
            script: "x.sh".into(),
        },
        pre: PreImage::Absent,
        post: TargetState::Absent,
        diff: None,
    });
    ledger::write_record_in(&r.root, &rec).unwrap();
    r.call(T0 + 1.0, &w, "record", json!({"repair": id, "note": "n"}))
        .unwrap();
    r.call(
        T0 + 2.0,
        &seat(7, "reviewer-codex"),
        "attest_verify",
        json!({"repair": id, "verdict": "pass", "note": "ok"}),
    )
    .unwrap();
    let before = r.record_of(&id);
    let e = r
        .call(T0 + 3.0, &w, "commit", json!({"repair": id}))
        .unwrap_err();
    assert_eq!(e.code, "repair_not_implemented", "{}", e.message);
    assert!(e.message.contains("hook_lines_step"));
    let after = r.record_of(&id);
    assert_eq!(after.state, RepairState::InProgress);
    assert!(after.committed.is_empty());
    assert_eq!(after.files, before.files);
    assert!(after.lock.is_some());
    assert!(
        !r.root.join(&id).join("lanes").exists(),
        "no lane row for a refused commit"
    );
    // the hook line is skipped by every file walker (W3(b))
    assert_eq!(
        cys::repair::ledger::candidate_states_in(&r.root, &after.id)
            .unwrap()
            .len(),
        1
    );
}

// ───────────────────────────── F4 · F2 · time bounds ─────────────────────────────

/// F4: a broken ledger file never stops the daemon — list / status / report / begin keep serving,
/// the broken record is listed, every mutating verb on it is refused with `repair_corrupt`.
#[test]
fn corrupt_record_daemon_keeps_serving() {
    let r = rig("corrupt");
    let w = seat(3, "worker");
    let good = r
        .call(T0, &w, "begin", json!({"files": ["bin/a.py"]}))
        .unwrap()["repair"]
        .as_str()
        .unwrap()
        .to_string();
    let bad = ledger::allocate_id_in(&r.root).unwrap();
    std::fs::write(ledger::record_path_in(&r.root, &bad), b"\x00\xffnot json").unwrap();
    let l = r.call(T0, &human(), "list", json!({})).unwrap();
    assert_eq!(l["repairs"].as_array().unwrap().len(), 1);
    assert_eq!(l["corrupt"][0]["repair"], json!(bad.as_str()));
    let s = r.call(T0, &human(), "status", json!({})).unwrap();
    assert_eq!(s["corrupt"].as_array().unwrap().len(), 1);
    let one = r
        .call(T0, &human(), "status", json!({"repair": bad.as_str()}))
        .unwrap();
    assert!(one["corrupt"].is_string());
    for verb in ["record", "commit"] {
        assert_eq!(
            r.call(T0, &w, verb, json!({"repair": bad.as_str(), "note": "x"}))
                .unwrap_err()
                .code,
            "repair_corrupt",
            "{verb}"
        );
    }
    assert_eq!(
        r.call(
            T0,
            &seat(7, "reviewer-codex"),
            "attest_verify",
            json!({"repair": bad.as_str(), "verdict": "pass", "note": "x"})
        )
        .unwrap_err()
        .code,
        "repair_corrupt"
    );
    assert_eq!(
        std::fs::read(ledger::record_path_in(&r.root, &bad)).unwrap(),
        b"\x00\xffnot json",
        "a broken record was rewritten"
    );
    assert!(r
        .call(
            T0,
            &w,
            "report",
            json!({"file": "bin/a.py", "error": "E: still serving"})
        )
        .is_ok());
    assert!(r
        .call(
            T0,
            &w,
            "record",
            json!({"repair": good, "note": "still works"})
        )
        .is_ok());
    // an unknown higher schema is unreadable too (O-C1-7)
    std::fs::write(ledger::record_path_in(&r.root, &bad), br#"{"schema": 9}"#).unwrap();
    assert_eq!(
        r.call(T0, &human(), "list", json!({})).unwrap()["corrupt"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

/// F2: no T3 verb waits on anything slow — a check that would take its full deadline is cut by the
/// commit budget (< the CLI's 40 s); a check runner that is not available refuses the commit; the
/// production runner (T5b stubs in this build) refuses rather than commit unchecked.
#[test]
fn commit_is_bounded_and_never_unchecked() {
    assert!(COMMIT_CHECK_BUDGET_SECS < 40);
    let r = rig("bound");
    let w = seat(3, "worker");
    let id = r
        .call(T0, &w, "begin", json!({"files": ["bin/a.py"]}))
        .unwrap()["repair"]
        .as_str()
        .unwrap()
        .to_string();
    r.call(T0 + 1.0, &w, "record", json!({"repair": id, "note": "n"}))
        .unwrap();
    r.call(
        T0 + 2.0,
        &seat(7, "reviewer-codex"),
        "attest_verify",
        json!({"repair": id, "verdict": "pass", "note": "ok"}),
    )
    .unwrap();
    let seen = std::cell::Cell::new(std::time::Duration::ZERO);
    let timeout = |_: &str, _: &Path, d: std::time::Duration| -> Result<CheckReport, RepairError> {
        seen.set(d);
        Ok(CheckReport {
            passed: false,
            timed_out: true,
            failure: String::new(),
            check_ms: d.as_millis() as u64,
            reference_kind: CheckKind::PyCompile,
            reference_ms: 1,
        })
    };
    let started = std::time::Instant::now();
    let e = handle_with(
        &r.ctx(T0 + 3.0, &general, &timeout),
        &w,
        "commit",
        &json!({"repair": id}),
    )
    .unwrap_err();
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
    assert_eq!(e.code, "repair_check_failed");
    assert!(seen.get() <= std::time::Duration::from_secs(COMMIT_CHECK_BUDGET_SECS));
    assert_eq!(
        r.record_of(&id).state,
        RepairState::InProgress,
        "a failed check commits nothing"
    );
    // the next record opens round 2 (a failed attempt)
    let out = r
        .call(T0 + 4.0, &w, "record", json!({"repair": id, "note": "n2"}))
        .unwrap();
    assert_eq!(out["new_round"], true);
    // no runner → refused, nothing committed
    r.call(
        T0 + 5.0,
        &seat(7, "reviewer-codex"),
        "attest_verify",
        json!({"repair": id, "verdict": "pass", "note": "ok"}),
    )
    .unwrap();
    let missing = |_: &str, _: &Path, _: std::time::Duration| -> Result<CheckReport, RepairError> {
        Err(RepairError::NotImplemented("rehearsal::run_check"))
    };
    let e = handle_with(
        &r.ctx(T0 + 6.0, &general, &missing),
        &w,
        "commit",
        &json!({"repair": id}),
    )
    .unwrap_err();
    assert_eq!(e.code, "repair_not_implemented");
    assert_eq!(r.record_of(&id).state, RepairState::InProgress);
    // the production runner of this build: T5b's stubs → refused (fail closed)
    let e = super::production_check(
        "bin/a.py",
        &r.pack.join("bin/a.py"),
        std::time::Duration::from_secs(1),
    );
    if cys::repair::rehearsal::run_check(&cys::repair::rehearsal::CheckRequest {
        entry_rel: "x".into(),
        scratch_root: PathBuf::new(),
        recorded_input: None,
        deadline: std::time::Duration::from_secs(1),
    })
    .is_err()
    {
        assert!(matches!(e, Err(RepairError::NotImplemented(_))));
    }
}

// ───────────────────────────── cleanup (O-C2-6 · P6) ─────────────────────────────

/// Cleanup: refused while not terminal, refused for an in-force repair while T6 cannot say which
/// slots it needs, allowed for a reverted repair; a worker of a lane without a row is refused.
#[test]
fn cleanup_rules() {
    let r = rig("cleanup");
    let id = committed_general(&r);
    let rid = RepairId::parse(&id).unwrap();
    assert_eq!(ledger::list_slots_in(&r.root, &rid).len(), 1);
    // 시험 기간 = not terminal
    assert_eq!(
        r.call(T0 + 5.0, &human(), "cleanup", json!({"repair": id}))
            .unwrap_err()
            .code,
        "repair_precondition"
    );
    // 사용 중 (in force): every slot refused while slots_needed_in_force is T6's stub
    let mut rec = r.record_of(&id);
    rec.state = RepairState::InUse;
    ledger::write_record_in(&r.root, &rec).unwrap();
    if cys::repair::apply::slots_needed_in_force(&rid).is_err() {
        let e = r
            .call(T0 + 5.0, &human(), "cleanup", json!({"repair": id}))
            .unwrap_err();
        assert!(e.message.contains("확인할 수 없습니다"), "{}", e.message);
        assert_eq!(ledger::list_slots_in(&r.root, &rid).len(), 1);
    }
    // 되돌려짐: all slots may go — but not with an unfinished operation record
    rec.state = RepairState::Reverted;
    ledger::write_record_in(&r.root, &rec).unwrap();
    std::fs::create_dir_all(r.root.join(&id).join("ops")).unwrap();
    std::fs::write(
        r.root.join(&id).join("ops").join("op-1.json"),
        br#"{"finished": false}"#,
    )
    .unwrap();
    assert_eq!(
        r.call(T0 + 5.0, &human(), "cleanup", json!({"repair": id}))
            .unwrap_err()
            .code,
        "repair_precondition"
    );
    std::fs::write(
        r.root.join(&id).join("ops").join("op-1.json"),
        br#"{"finished": true}"#,
    )
    .unwrap();
    // a worker of a department that holds no row of it is refused; the origin lane's worker passes
    let mut other = rig("cleanup-other");
    other.root = r.root.clone();
    other.lane = LaneKey("dept-x".into());
    assert_eq!(
        other
            .call(
                T0 + 5.0,
                &seat(3, "worker"),
                "cleanup",
                json!({"repair": id})
            )
            .unwrap_err()
            .code,
        "acl_denied"
    );
    assert_eq!(
        r.call(
            T0 + 5.0,
            &human(),
            "cleanup",
            json!({"repair": id, "slot": "../x"})
        )
        .unwrap_err()
        .code,
        "invalid_params"
    );
    let out = r
        .call(
            T0 + 5.0,
            &seat(3, "worker"),
            "cleanup",
            json!({"repair": id}),
        )
        .unwrap();
    assert_eq!(out["removed"].as_array().unwrap().len(), 1);
    assert!(ledger::list_slots_in(&r.root, &rid).is_empty());
    assert!(
        r.root.join(&id).join("record.json").is_file(),
        "cleanup never deletes the record"
    );
}

// ───────────────────────────── caller identity (M1 · M4 · P5 · interp. 36) ─────────────────────────────

/// The seatless decision table: the app's token → human (both systems); a non-pane terminal with no
/// `cysd` ancestor → human; no terminal, a pane's terminal, or a `cysd` ancestor (a scheduler child,
/// another lane's pane shell) → detached.
#[test]
fn seatless_decision_table() {
    let o = |t, p, c| SeatlessObservation {
        has_terminal: t,
        terminal_is_pane: p,
        cysd_ancestor: c,
    };
    assert_eq!(
        decide_seatless(true, &o(false, false, false)),
        CallerKind::Human
    );
    assert_eq!(
        decide_seatless(true, &o(false, false, true)),
        CallerKind::Human
    );
    assert_eq!(
        decide_seatless(false, &o(true, false, false)),
        CallerKind::Human
    );
    assert_eq!(
        decide_seatless(false, &o(false, false, false)),
        CallerKind::Detached
    );
    assert_eq!(
        decide_seatless(false, &o(true, true, false)),
        CallerKind::Detached
    );
    assert_eq!(
        decide_seatless(false, &o(true, false, true)),
        CallerKind::Detached
    );
    // seat_by_terminal: the first pane whose slave device is the terminal
    let panes = [(4u64, 10i32), (9, 11), (12, -1)];
    let rdev = |fd: i32| match fd {
        10 => Some(100u32),
        11 => Some(111),
        _ => None,
    };
    assert_eq!(seat_by_terminal(&panes, 111, rdev), Some(9));
    assert_eq!(seat_by_terminal(&panes, 999, rdev), None);
}

/// P5 / N2: a `cysd` ancestor (any lane) or this daemon itself makes a caller not human.
#[cfg(target_os = "macos")]
#[test]
fn cysd_ancestor_detection() {
    use crate::state::ProcBrief;
    let chain = |pid: u32| -> Option<ProcBrief> {
        let parent = match pid {
            500 => 400,
            400 => 300,
            300 => 1,
            _ => return None,
        };
        Some(ProcBrief {
            parent: Some(parent),
            start_time: 1,
            tdev: None,
        })
    };
    let names_none = |_: u32| -> Option<String> { Some("zsh".into()) };
    assert!(!has_cysd_ancestor(500, 77, chain, names_none));
    let names_cysd = |p: u32| -> Option<String> {
        Some(if p == 300 {
            "cysd".into()
        } else {
            "zsh".into()
        })
    };
    assert!(has_cysd_ancestor(500, 77, chain, names_cysd));
    assert!(
        has_cysd_ancestor(500, 400, chain, names_none),
        "this daemon itself is a cysd ancestor"
    );
}

/// M4: a real PTY — a child that made the slave its controlling terminal (setsid + TIOCSCTTY)
/// reports it as `e_tdev` through `proc_brief`, it equals the slave device derived from the master
/// fd, and the terminal fallback attributes the child to that pane; a child without a terminal has
/// none. (Terminal.app / sshd / a `nohup` job re-parented to launchd need real panes — not measured
/// here; see the report.)
#[cfg(target_os = "macos")]
#[test]
fn pty_controlling_terminal_is_attributed_to_its_pane() {
    use portable_pty::{native_pty_system, CommandBuilder, PtySize};
    // The pane path of the daemon itself: the PTY library makes the slave the child's controlling
    // terminal (setsid + TIOCSCTTY inside the library — no raw spawn hook here, U-7).
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("openpty");
    let master_fd = pair.master.as_raw_fd().expect("master fd");
    let rdev = pty_slave_rdev(master_fd).expect("slave device of the master");
    let mut builder = CommandBuilder::new("/bin/sleep");
    builder.arg("30");
    let mut child = pair
        .slave
        .spawn_command(builder)
        .expect("spawn sleep on the PTY");
    let pid = child.process_id().expect("child pid");
    // a child with no terminal at all (stdio to null, a new session through the spawn helper)
    let mut plain = std::process::Command::new("/bin/sleep");
    plain.arg("30");
    cys::SpawnPolicy::spawn_policy(&mut plain, cys::ChildLifetime::Survivor);
    let mut detached_child = plain.spawn().expect("spawn sleep without a terminal");
    let mut got = None;
    for _ in 0..50 {
        got = crate::state::proc_brief(pid).and_then(|b| b.tdev);
        if got.is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let none = crate::state::proc_brief(detached_child.id()).map(|b| b.tdev);
    let _ = child.kill();
    let _ = child.wait();
    let _ = detached_child.kill();
    let _ = detached_child.wait();
    assert_eq!(
        got,
        Some(rdev),
        "e_tdev of the child is not the pane's slave device"
    );
    assert_eq!(
        seat_by_terminal(&[(1, -1), (42, master_fd)], rdev, pty_slave_rdev),
        Some(42)
    );
    assert_eq!(
        none,
        Some(None),
        "a new-session child without a terminal must report none"
    );
    drop(pair.master);
}

/// The wiring pins: the terminal fallback lives in `walk_caller_ancestry` (every resolver caller sees
/// it), the seat release in `close_surface_effect`, the prompt record in `hook.machine_origin`.
#[test]
fn wiring_pins() {
    let handlers = include_str!("../handlers.rs");
    let walk = &handlers[handlers.find("fn walk_caller_ancestry(").unwrap()..];
    let walk = &walk[..walk.find("\nfn walk_ancestry_with(").unwrap()];
    assert!(walk.contains("crate::repair::seat_of_terminal"));
    let arm = &handlers[handlers.find("\"hook.machine_origin\" => {").unwrap()..];
    let arm = &arm[..arm.find("\"boot.exit_ack\" =>").unwrap()];
    assert!(arm.contains("crate::repair::record_prompt_origin("));
    let gov = include_str!("../governance.rs");
    let eff = &gov[gov.find("fn close_surface_effect(").unwrap()..];
    let eff = &eff[..eff.find("\n}\n").unwrap()];
    assert_eq!(
        eff.matches("crate::repair::release_locks_for_seat(daemon, id)")
            .count(),
        1
    );
    assert_eq!(
        gov.matches("crate::repair::release_locks_for_seat(")
            .count(),
        1,
        "exactly one call (reap goes through the effect)"
    );
}

// ───────────────────────────── prompt-origin record (RULINGS T3 · U4(a)) ─────────────────────────────

/// A question at t0, a machine push at t1, the owner's "예" at t2 → `machine_seen` (T4 then refuses
/// `grant on`); an unknown origin counts as machine (fail closed); `named_docs` changes only on an
/// owner-typed prompt; the first owner answer after the question is kept once.
#[test]
fn prompt_origin_is_question_aware() {
    let mut rec = PromptOrigin {
        question: Some(OpenQuestion {
            asked_at: 10.0,
            ..Default::default()
        }),
        ..Default::default()
    };
    apply_prompt(&mut rec, "d-old", "earlier", "human", 5.0);
    assert!(
        rec.question.as_ref().unwrap().first_human.is_none(),
        "a prompt before the question is no answer"
    );
    apply_prompt(&mut rec, "d1", "[worker] done", "machine", 11.0);
    apply_prompt(&mut rec, "d2", "예", "human", 12.0);
    let q = rec.question.clone().unwrap();
    assert!(q.machine_seen);
    assert_eq!(q.first_human.as_ref().map(|f| f.0.as_str()), Some("d2"));
    apply_prompt(&mut rec, "d3", "아니", "human", 13.0);
    assert_eq!(
        rec.question
            .as_ref()
            .unwrap()
            .first_human
            .as_ref()
            .map(|f| f.0.as_str()),
        Some("d2"),
        "kept once"
    );
    let mut u = PromptOrigin {
        question: Some(OpenQuestion {
            asked_at: 1.0,
            ..Default::default()
        }),
        ..Default::default()
    };
    apply_prompt(&mut u, "d", "?", "unknown", 2.0);
    assert!(u.question.unwrap().machine_seen);
    let mut n = PromptOrigin::default();
    apply_prompt(
        &mut n,
        "d",
        "고쳐라 soul.md 그리고 MASTER_DIRECTIVE 도",
        "human",
        1.0,
    );
    assert_eq!(n.named_docs, ["soul.md", "master_directive.md"]);
    apply_prompt(&mut n, "d", "worker_directive.md 고쳐", "machine", 2.0);
    assert_eq!(
        n.named_docs,
        ["soul.md", "master_directive.md"],
        "a machine prompt does not change named_docs"
    );
    assert_eq!(n.origin, "machine");
    assert_eq!(
        named_rule_docs("CLAUDE.md, ROUTE_CONTRACT.md. x_directive.md"),
        ["claude.md", "route_contract.md", "x_directive.md"]
    );
    assert!(named_rule_docs("_directive.md soul notes.md").is_empty());
}

/// The record lives in the daemon's map and is dropped with the seat.
#[test]
fn prompt_origin_record_on_daemon() {
    let daemon = crate::team_gate_tests::tmp_daemon("t3-origin", false);
    record_prompt_origin(&daemon, 5, "고쳐라 soul.md", None, Some("human"));
    {
        let m = daemon
            .repair_prompt_origin
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let r = m.get(&5).unwrap();
        assert_eq!(r.origin, "human");
        assert_eq!(r.named_docs, ["soul.md"]);
        assert_eq!(
            r.digest,
            cys::mission_gate::digest_normalized("고쳐라 soul.md")
        );
    }
    release_locks_for_seat(&daemon, 5);
    assert!(daemon
        .repair_prompt_origin
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&5)
        .is_none());
    let _ = std::fs::remove_dir_all(production_root(&daemon));
}

/// `status` (no id) carries H3's catch-up fields (T7b's — not available in this build → null with
/// the reason) and the reviewer slots (B2: an empty slot = repairs wait, visible).
#[test]
fn status_summary_fields() {
    let r = rig("status");
    let mut ctx = r.ctx(T0, &general, &pass);
    let s = handle_with(&ctx, &detached(), "status", &json!({})).unwrap();
    for k in [
        "initial_catchup_complete",
        "catchup_pending",
        "catchup_conflict",
    ] {
        assert!(s.get(k).is_some(), "{k}");
    }
    assert_eq!(s["reviewer_slots"]["reviewer1"], json!([7]));
    assert_eq!(s["reviewer_slots"]["waiting"], false);
    ctx.seats = vec![(7, "reviewer-codex".into())];
    let s = handle_with(&ctx, &detached(), "status", &json!({})).unwrap();
    assert_eq!(s["reviewer_slots"]["waiting"], true);
    // a lane merge lock on a GENERAL path: begin --joins writes this lane's row only (H8, P4)
    let id = committed_general(&r);
    let rid = RepairId::parse(&id).unwrap();
    let mut row = ledger::read_lane_in(&r.root, &rid, &r.lane)
        .unwrap()
        .unwrap();
    row.state = LaneState::Conflict;
    ledger::write_lane_in(&r.root, &row).unwrap();
    let j = r
        .call(
            T0 + 10.0,
            &seat(13, "worker-3"),
            "begin",
            json!({"files": ["bin/a.py"], "joins": id}),
        )
        .unwrap();
    assert_eq!(j["begin"], "joins");
    assert_eq!(j["merge_attempt"], 1);
    let row = ledger::read_lane_in(&r.root, &rid, &r.lane)
        .unwrap()
        .unwrap();
    assert_eq!(
        row.merge_lock.as_ref().unwrap().owner,
        LockOwner {
            lane: r.lane.clone(),
            seat: 13,
            incarnation: r.inc.clone()
        }
    );
    assert!(
        r.record_of(&id).lock.is_none(),
        "--joins never writes the repair's lock (P4)"
    );
    // reap releases the merge lock too (R5)
    release_seat(&r.ctx(T0 + 11.0, &general, &pass), 13);
    assert!(ledger::read_lane_in(&r.root, &rid, &r.lane)
        .unwrap()
        .unwrap()
        .merge_lock
        .is_none());
}
