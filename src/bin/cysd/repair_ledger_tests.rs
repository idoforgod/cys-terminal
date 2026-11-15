//! Track T3 daemon tests (`src/bin/cysd/repair_ledger_tests.rs`, declared test-only from `repair.rs`
//! through `#[path]`: the boot-health census H-AUTH-SELFLOOP reads only top-level files of src/bin/cysd) — the role table, the verbs, the locks, intake, caller identity.
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
    /// T4: the per-seat prompt record `Ctx` carries since the owner-decision verbs.
    prompts: Mutex<HashMap<u64, PromptOrigin>>,
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
        prompts: Mutex::new(HashMap::new()),
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
            hook_step: &cys::repair::apply::hook_lines_step,
            claim: &cys::repair::grant::try_claim_autonomous_start,
            prompts: &self.prompts,
            product_pause: &|| false,
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

/// A seat whose current role is the first it held (AD1: bound to that role's family).
fn seat(sid: u64, role: &str) -> Caller {
    Caller {
        kind: CallerKind::Seat { sid },
        role: Some(role.into()),
        app_token: false,
        bound_family: role_family(role),
    }
}
fn human() -> Caller {
    Caller {
        kind: CallerKind::Human,
        role: None,
        app_token: false,
        bound_family: None,
    }
}
fn app() -> Caller {
    Caller {
        kind: CallerKind::Human,
        role: None,
        app_token: true,
        bound_family: None,
    }
}
fn detached() -> Caller {
    Caller {
        kind: CallerKind::Detached,
        role: None,
        app_token: false,
        bound_family: None,
    }
}

const T0: f64 = 1_800_000_000.0;

/// begin → record → verify (other seat) → commit for `bin/a.py`; returns the id.
fn committed_general(r: &Rig) -> String {
    let w = seat(3, "worker");
    let b = r
        .call(
            T0,
            &w,
            "begin",
            json!({"trigger": "owner_order", "files": ["bin/a.py"]}),
        )
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
                bound_family: None,
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
    // T4 (RULINGS CI6 (b)): the READ verb `grant_status` joined the table — 28 → 29 verbs (old
    // expectation `assert_eq!(checked, 28 * callers.len());`)
    assert_eq!(VERB_CLASSES.len(), 29);
    assert_eq!(checked, 29 * callers.len());
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
        json!({"files": ["bin/a.py"], "trigger": "owner_order", "role": "worker", "seat": 3, "surface_id": 3, "caller": "worker"}),
        json!({"files": ["bin/a.py"], "trigger": "owner_order", "CYS_ROLE": "worker", "seat_token": "x"}),
    ] {
        let e = r.call(T0, &seat(1, "master"), "begin", p).unwrap_err();
        assert_eq!(e.code, "acl_denied");
    }
    assert!(!r.root.exists(), "a refused begin wrote something");
    let src = [include_str!("repair.rs")];
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
    for verb in ["tick", "sweep"] {
        // T6 (rewrite — the old expectation was `repair_not_implemented`): accepted by the role table;
        // the bare dispatcher holds no daemon state, so it refuses with a sentence — the daemon's own
        // `handle` answers these two (`repair_job::trigger`, tested in repair_job_tests.rs)
        assert_eq!(
            r.call(T0, &detached(), verb, json!({})).unwrap_err().code,
            "repair_precondition",
            "{verb}"
        );
    }
    for verb in ["approval_list"] {
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
        ("repair.rs", include_str!("repair.rs")),
        ("intake.rs", include_str!("../../repair/intake.rs")),
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
    let lib = include_str!("../../../cysjavis-pack/hooks/_lib.sh");
    let obs = include_str!("../../../cysjavis-pack/hooks/cys-hook.sh");
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
        .call(
            T0,
            &w,
            "begin",
            json!({"trigger": "owner_order", "files": ["bin/a.py"]}),
        )
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
        .call(
            T0,
            &w,
            "begin",
            json!({"trigger": "owner_order", "files": ["bin/a.py"]}),
        )
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
        .call(
            T0,
            &w,
            "begin",
            json!({"trigger": "owner_order", "files": ["bin/a.py"]}),
        )
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
        .call(
            T0,
            &w,
            "begin",
            json!({"trigger": "owner_order", "files": ["bin/a.py"]}),
        )
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
        .call(
            T0,
            &w,
            "begin",
            json!({"trigger": "owner_order", "files": ["bin/a.py"]}),
        )
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
            json!({"trigger": "owner_order", "files": ["bin/a.py"]}),
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
        .call(
            T0,
            &w,
            "begin",
            json!({"trigger": "owner_order", "files": ["bin/a.py"]}),
        )
        .unwrap();
    let id = first["repair"].as_str().unwrap().to_string();
    let again = r
        .call(
            T0 + 1.0,
            &w,
            "begin",
            json!({"trigger": "owner_order", "files": ["bin/a.py"]}),
        )
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
            json!({"trigger": "owner_order", "files": ["bin/a.py"]}),
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
            json!({"trigger": "owner_order", "files": ["bin/a.py"]}),
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
            json!({"trigger": "owner_order", "files": ["bin/a.py"]}),
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
                        json!({"trigger": "owner_order", "files": ["bin/a.py"]}),
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

/// RULINGS AD7 (rewritten from the R5 pick-up test — AD7 replaces "an in-force repair is picked up
/// and set back to 진행 중"): a `begin` on the files of a repair in force (`시험 기간`) starts a NEW
/// repair with its own pre-image and leaves the in-force one exactly as it was; a repair reopened by
/// the audit IS picked up. P4 (unchanged): a worker's `revert` passes the role table without
/// needing, writing or clearing the repair's lock (the body is T6's).
#[test]
fn pick_up_of_in_force_and_revert_role() {
    let r = rig("inforce");
    let id = committed_general(&r);
    assert_eq!(r.record_of(&id).state, RepairState::Probation);
    // T6 (rewrite — the old expectation was `repair_not_implemented` from the stub): a worker seat of
    // the lane passes the role table, the revert body runs and never touches the lock (C1 1.12 / P4).
    // On its own rig, so the in-force repair below stays in force for the pick-up check.
    {
        let rr = rig("inforce-revert");
        // GK4 / FI1: a repair is returned only when its pre-image's generation is the pack's now —
        // this rig's pack states its generation (the T3 rig has none: every repair begun in it is
        // "not returned automatically", which is not what this check is about)
        std::fs::write(rr.pack.join(".pack-version"), "0.15.0\n").unwrap();
        let rid_ = committed_general(&rr);
        let before_lock = rr.record_of(&rid_).lock.clone();
        let v = rr
            .call(
                T0 + 5.0,
                &seat(13, "worker-3"),
                "revert",
                json!({"repair": rid_}),
            )
            .unwrap();
        assert_eq!(v["repair"], json!(rid_), "{v}");
        assert_eq!(
            rr.record_of(&rid_).lock,
            before_lock,
            "revert touched the lock"
        );
    }
    let in_force = r.record_of(&id);
    let p = r
        .call(
            T0 + 6.0,
            &seat(13, "worker-3"),
            "begin",
            json!({"trigger": "owner_order", "files": ["bin/a.py"]}),
        )
        .unwrap();
    assert_eq!(p["begin"], "new", "an in-force repair is never picked up");
    let new_id = p["repair"].as_str().unwrap().to_string();
    assert_ne!(new_id, id);
    assert_eq!(
        r.record_of(&id),
        in_force,
        "the in-force repair was touched"
    );
    let fresh = r.record_of(&new_id);
    assert_eq!(fresh.state, RepairState::InProgress);
    assert_eq!(fresh.attempt, 1);
    // its own pre-image = the bytes in force now (the committed fix), not the first repair's
    assert_eq!(
        fresh.files[0].pre.state(),
        in_force.files[0].post,
        "the new repair's pre-image is the committed state"
    );
    assert_ne!(fresh.files[0].pre, in_force.files[0].pre);
    // a repair reopened by the audit is unfinished work: picked up
    let r2 = rig("reopen-pick");
    let id2 = committed_general(&r2);
    r2.call(
        T0 + 4.0,
        &seat(8, "reviewer-gemini"),
        "attest_audit",
        json!({"repair": id2, "verdict": "finding", "note": "diff ≠ ledger"}),
    )
    .unwrap();
    assert_eq!(r2.record_of(&id2).state, RepairState::ReopenedByAudit);
    let p = r2
        .call(
            T0 + 5.0,
            &seat(13, "worker-3"),
            "begin",
            json!({"trigger": "owner_order", "files": ["bin/a.py"]}),
        )
        .unwrap();
    assert_eq!(p["begin"], "picked-up");
    assert_eq!(p["repair"], json!(id2));
    assert_eq!(r2.record_of(&id2).lock.unwrap().owner.seat, 13);
}

/// G7(c): a lock bound to a closed grant claim authorises no mutating verb.
#[test]
fn closed_claim_refuses_mutating_verbs() {
    let r = rig("claim");
    let w = seat(3, "worker");
    let id = r
        .call(
            T0,
            &w,
            "begin",
            json!({"trigger": "owner_order", "files": ["bin/a.py"]}),
        )
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
    let b = handle_with(
        &ctx,
        &w,
        "begin",
        &json!({"trigger": "owner_order", "files": ["hooks/h.sh"]}),
    )
    .unwrap();
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
        reason: String::new(),
        updated_at: T0,
    };
    ledger::write_lane_in(&r.root, &row).unwrap();
    let e = handle_with(
        &ctx,
        &seat(13, "worker-3"),
        "begin",
        &json!({"trigger": "owner_order", "files": ["hooks/h.sh"], "joins": rid}),
    )
    .unwrap_err();
    assert_eq!(e.code, "repair_precondition");
    assert!(e.message.contains("--resolves"), "{}", e.message);
    assert!(e.message.contains("--trigger"), "AK6: {}", e.message);
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
        &json!({"trigger": "owner_order", "files": ["hooks/h.sh"], "resolves": rid}),
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
        &json!({"trigger": "owner_order", "files": ["hooks/h.sh"]}),
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
            json!({"trigger": "owner_order", "unregister_hook": ["SessionStart:session-start.sh"]}),
        )
        .unwrap_err();
    assert!(e.message.contains("생명줄"), "{}", e.message);
    if cys::repair::boot_critical::entries().is_empty() {
        let e = r
            .call(
                T0,
                &w,
                "begin",
                json!({"trigger": "owner_order", "unregister_hook": ["Stop:x.sh"]}),
            )
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
        &json!({"trigger": "owner_order", "files": ["hooks/h.sh"], "unregister_hook": ["Stop:x.sh"]}),
    )
    .unwrap_err();
    assert_eq!(e.code, "repair_precondition");
    assert!(e.message.contains("두 수리로 나누세요"), "{}", e.message);
    assert_eq!(
        r.call(T0, &w, "begin", json!({"trigger": "owner_order"}))
            .unwrap_err()
            .code,
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
            r.call(
                T0,
                &w,
                "begin",
                json!({"trigger": "owner_order", "unregister_hook": [bad]})
            )
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
        &json!({"trigger": "owner_order", "files": ["bin/a.py"]}),
    )
    .unwrap_err();
    assert_eq!(e.code, "repair_not_implemented");
    assert!(ledger::list_ids_in(&r.root).is_empty());
}

/// RULINGS Y1 / Y2(c) — T6 rewrite (the old pin: "until T6's `hook_lines_step` lands, a commit whose
/// record has a hook line is refused with the not-implemented sentence and the record is left exactly
/// as before; no lane row appears"). T6's step is real now: when it cannot complete (here the
/// production step cannot see this rig's ledger — any registrar or record error behaves the same) the
/// commit STANDS (the committed state was durable before the step ran — Y1 order) and this lane's row
/// is `충돌` with the step's error — never a silent `검증됨`; the confirming case is `ak5_…` below.
#[test]
fn commit_with_hook_line_refused_while_step_is_stub() {
    let r = rig("hookcommit");
    let w = seat(3, "worker");
    let id = r
        .call(
            T0,
            &w,
            "begin",
            json!({"trigger": "owner_order", "files": ["bin/a.py"]}),
        )
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
    let c = r
        .call(T0 + 3.0, &w, "commit", json!({"repair": id}))
        .unwrap();
    assert!(c["hook_step"]["error"].is_string(), "{c}");
    let after = r.record_of(&id);
    assert_eq!(after.state, RepairState::Probation, "the commit stands");
    assert!(!after.committed.is_empty());
    assert_eq!(after.files, before.files);
    assert!(!after.checks.iter().any(|(_, how)| how == "hook-registrar"), "never claimed");
    let row = ledger::read_lane_in(&r.root, &after.id, &r.lane)
        .unwrap()
        .expect("this lane's row");
    assert_eq!(row.state, LaneState::Conflict, "충돌 — Y2(c)");
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
        .call(
            T0,
            &w,
            "begin",
            json!({"trigger": "owner_order", "files": ["bin/a.py"]}),
        )
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
        .call(
            T0,
            &w,
            "begin",
            json!({"trigger": "owner_order", "files": ["bin/a.py"]}),
        )
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
    // The bound is structural (no real-clock window): the deadline handed to the check never
    // exceeds the commit budget, and a timed-out check is a refusal, not a wait.
    let e = handle_with(
        &r.ctx(T0 + 3.0, &general, &timeout),
        &w,
        "commit",
        &json!({"repair": id}),
    )
    .unwrap_err();
    assert_eq!(e.code, "repair_check_failed");
    assert!(
        seen.get() > std::time::Duration::ZERO,
        "the check was never called"
    );
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
/// REWRITTEN by T7b (RULINGS HI2 / HJ2): `commit` now seals the committed bytes into a SECOND slot of
/// R's vault (kind `committed-copy`), so a committed general repair holds two slots — the begin
/// pre-image and the committed copy. T3's old expectations, quoted: `list_slots_in(..).len() == 1`
/// after the commit and in the in-force branch, and `out["removed"].len() == 1` at the end.
#[test]
fn cleanup_rules() {
    let r = rig("cleanup");
    let id = committed_general(&r);
    let rid = RepairId::parse(&id).unwrap();
    assert_eq!(ledger::list_slots_in(&r.root, &rid).len(), 2, "the pre-image and the committed copy (old: 1)");
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
        assert_eq!(ledger::list_slots_in(&r.root, &rid).len(), 2, "nothing removed (old: 1)");
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
    assert_eq!(out["removed"].as_array().unwrap().len(), 2, "both slots of a reverted repair (old: 1)");
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
    let handlers = include_str!("handlers.rs");
    let walk = &handlers[handlers.find("fn walk_caller_ancestry(").unwrap()..];
    let walk = &walk[..walk.find("\nfn walk_ancestry_with(").unwrap()];
    assert!(walk.contains("crate::repair::seat_of_terminal"));
    let arm = &handlers[handlers.find("\"hook.machine_origin\" => {").unwrap()..];
    let arm = &arm[..arm.find("\"boot.exit_ack\" =>").unwrap()];
    assert!(arm.contains("crate::repair::record_prompt_origin("));
    let gov = include_str!("governance.rs");
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
/// owner-typed prompt; the first owner answer after the question is kept once. RULINGS CI1 (T4):
/// "owner-typed" for an answer or a naming is Level 2 — the hook's `human` (Level 1) AND the seat's
/// typing mark, set here first (`typing_mark = Some(<the prompt's time>)`, the test seam of
/// `mark_app_typing`; RULINGS CJ2 gives the mark a time); the
/// hook's `human` alone answers nothing and names nothing (pinned at the end).
#[test]
fn prompt_origin_is_question_aware() {
    let mut rec = PromptOrigin {
        question: Some(OpenQuestion {
            asked_at: 10.0,
            ..Default::default()
        }),
        ..Default::default()
    };
    rec.typing_mark = Some(5.0);
    apply_prompt(&mut rec, "d-old", "earlier", "human", 5.0);
    assert!(
        rec.question.as_ref().unwrap().first_human.is_none(),
        "a prompt before the question is no answer"
    );
    apply_prompt(&mut rec, "d1", "[worker] done", "machine", 11.0);
    rec.typing_mark = Some(12.0);
    apply_prompt(&mut rec, "d2", "예", "human", 12.0);
    let q = rec.question.clone().unwrap();
    assert!(q.machine_seen);
    assert_eq!(q.first_human.as_ref().map(|f| f.0.as_str()), Some("d2"));
    rec.typing_mark = Some(13.0);
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
    let mut n = PromptOrigin {
        typing_mark: Some(1.0),
        ..Default::default()
    };
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
    // CI1: the hook's `human` alone (no typing mark) — Level 1: origin and last_human only
    let mut h = PromptOrigin {
        question: Some(OpenQuestion {
            asked_at: 1.0,
            ..Default::default()
        }),
        ..Default::default()
    };
    apply_prompt(&mut h, "d", "예 고쳐라 soul.md", "human", 2.0);
    assert_eq!(h.origin, "human");
    assert_eq!(h.last_human, Some(("d".to_string(), 2.0)));
    assert!(h.named_docs.is_empty() && h.last_typed.is_none());
    let q = h.question.unwrap();
    assert!(q.machine_seen && q.first_human.is_none());
}

/// The record lives in the daemon's map and is dropped with the seat.
#[test]
fn prompt_origin_record_on_daemon() {
    let daemon = crate::team_gate_tests::tmp_daemon("t3-origin", false);
    // RULINGS CI1 (T4): without the seat's typing mark the hook's `human` names nothing
    record_prompt_origin(&daemon, 5, "고쳐라 soul.md", None, Some("human"));
    assert!(daemon
        .repair_prompt_origin
        .lock()
        .unwrap_or_else(|e| e.into_inner())[&5]
        .named_docs
        .is_empty());
    mark_app_typing(
        &daemon.repair_prompt_origin,
        5,
        cys::repair::ledger::now_epoch(),
    );
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
            json!({"trigger": "owner_order", "files": ["bin/a.py"], "joins": id}),
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

// ───────────────────────────── RULINGS AD (review of T3) ─────────────────────────────

static AD_INC: AtomicU64 = AtomicU64::new(0);

/// A fresh (lane, incarnation) for the role-binding map, so tests never share entries.
fn ad_daemon_key(tag: &str) -> (String, String) {
    (
        format!("ad-{tag}"),
        format!(
            "inc-{}-{}",
            std::process::id(),
            AD_INC.fetch_add(1, Ordering::Relaxed)
        ),
    )
}

/// AD1: a master that renames itself `worker-9` cannot `begin`; READ and INTAKE stay open to it.
#[test]
fn ad1_master_renamed_worker_cannot_begin() {
    let r = rig("ad1-master");
    let (lane, inc) = ad_daemon_key("m");
    // first seen as master (its current role binds on the first repair call)
    assert_eq!(
        seat_caller(&lane, &inc, 41, Some("master".into())).bound_family,
        Some(RoleFamily::Master)
    );
    // `cys claim-role worker-9` → the recording call in claim_role keeps the first family
    assert_eq!(
        record_role_claim(&lane, &inc, 41, Some("master"), "worker-9"),
        Some(RoleFamily::Master)
    );
    let renamed = seat_caller(&lane, &inc, 41, Some("worker-9".into()));
    assert_eq!(renamed.bound_family, Some(RoleFamily::Master));
    let e = r
        .call(
            T0,
            &renamed,
            "begin",
            json!({"trigger": "owner_order", "files": ["bin/a.py"]}),
        )
        .unwrap_err();
    assert_eq!(e.code, "acl_denied");
    assert!(
        e.message.contains("AD1") && e.message.contains("master"),
        "{}",
        e.message
    );
    assert!(!r.root.exists(), "a refused begin wrote something");
    // READ / INTAKE unaffected
    assert!(r.call(T0, &renamed, "list", json!({})).is_ok());
    assert!(r
        .call(
            T0,
            &renamed,
            "report",
            json!({"file": "bin/a.py", "error": "E: x"})
        )
        .is_ok());
    // the plain worker of the same lane still begins
    assert!(r
        .call(
            T0,
            &seat(3, "worker"),
            "begin",
            json!({"trigger": "owner_order", "files": ["bin/a.py"]})
        )
        .is_ok());
}

/// AD1: a worker that renames itself `reviewer-claude-2` cannot sign a verification.
#[test]
fn ad1_worker_renamed_reviewer_cannot_verify() {
    let r = rig("ad1-worker");
    let (lane, inc) = ad_daemon_key("w");
    let w = seat_caller(&lane, &inc, 42, Some("worker".into()));
    let id = r
        .call(
            T0,
            &w,
            "begin",
            json!({"trigger": "owner_order", "files": ["bin/a.py"]}),
        )
        .unwrap()["repair"]
        .as_str()
        .unwrap()
        .to_string();
    std::fs::write(r.pack.join("bin/a.py"), b"print('fixed')\n").unwrap();
    r.call(T0 + 1.0, &w, "record", json!({"repair": id, "note": "n"}))
        .unwrap();
    record_role_claim(&lane, &inc, 42, Some("worker"), "reviewer-claude-2");
    let renamed = seat_caller(&lane, &inc, 42, Some("reviewer-claude-2".into()));
    assert_eq!(renamed.bound_family, Some(RoleFamily::Worker));
    let e = r
        .call(
            T0 + 2.0,
            &renamed,
            "attest_verify",
            json!({"repair": id, "verdict": "pass", "note": "mine"}),
        )
        .unwrap_err();
    assert_eq!(e.code, "acl_denied");
    assert!(e.message.contains("AD1"), "{}", e.message);
    assert!(r.record_of(&id).verify.is_none());
}

/// AD1 (B2 over every round): a seat that recorded in round 1 cannot verify round 2, even when it
/// later sits in a reviewer slot (a new daemon incarnation — no binding memory survives).
#[test]
fn ad1_round_one_fixer_cannot_verify_round_two() {
    let r = rig("ad1-rounds");
    let a = seat(21, "worker");
    let id = r
        .call(
            T0,
            &a,
            "begin",
            json!({"trigger": "owner_order", "files": ["bin/a.py"]}),
        )
        .unwrap()["repair"]
        .as_str()
        .unwrap()
        .to_string();
    r.call(
        T0 + 1.0,
        &a,
        "record",
        json!({"repair": id, "note": "round 1"}),
    )
    .unwrap();
    r.call(
        T0 + 2.0,
        &seat(7, "reviewer-codex"),
        "attest_verify",
        json!({"repair": id, "verdict": "fail", "note": "still broken"}),
    )
    .unwrap();
    release_seat(&r.ctx(T0 + 3.0, &general, &pass), 21);
    let b = seat(22, "worker-2");
    let p = r
        .call(
            T0 + 4.0,
            &b,
            "begin",
            json!({"trigger": "owner_order", "files": ["bin/a.py"]}),
        )
        .unwrap();
    assert_eq!(p["begin"], "picked-up");
    std::fs::write(r.pack.join("bin/a.py"), b"print('round 2')\n").unwrap();
    let out = r
        .call(
            T0 + 5.0,
            &b,
            "record",
            json!({"repair": id, "note": "round 2"}),
        )
        .unwrap();
    assert_eq!(out["new_round"], true);
    assert_eq!(r.record_of(&id).fixer_seats, [21, 22]);
    // seat 21, now in the reviewer1 slot (first role of its new incarnation = reviewer)
    let e = r
        .call(
            T0 + 6.0,
            &seat(21, "reviewer-codex"),
            "attest_verify",
            json!({"repair": id, "verdict": "pass", "note": "looks fine"}),
        )
        .unwrap_err();
    assert_eq!(e.code, "acl_denied");
    assert!(e.message.contains("자기가 고친"), "{}", e.message);
    // a seat that never fixed it verifies
    assert!(r
        .call(
            T0 + 6.0,
            &seat(7, "reviewer-codex"),
            "attest_verify",
            json!({"repair": id, "verdict": "pass", "note": "ok"}),
        )
        .is_ok());
}

/// AD1: a seat first seen without a role binds nothing; its first claim (`worker`) binds and works.
#[test]
fn ad1_roleless_seat_then_worker_works() {
    let r = rig("ad1-roleless");
    let (lane, inc) = ad_daemon_key("r");
    let first = seat_caller(&lane, &inc, 43, None);
    assert_eq!(first.bound_family, None, "an empty role binds nothing");
    assert_eq!(
        role_family("dept-helper"),
        None,
        "an unknown role binds nothing either"
    );
    assert_eq!(
        record_role_claim(&lane, &inc, 43, None, "worker"),
        Some(RoleFamily::Worker)
    );
    let w = seat_caller(&lane, &inc, 43, Some("worker".into()));
    assert!(r
        .call(
            T0,
            &w,
            "begin",
            json!({"trigger": "owner_order", "files": ["bin/a.py"]})
        )
        .is_ok());
    // the families of the table
    for (role, fam) in [
        ("master", RoleFamily::Master),
        ("ceo", RoleFamily::Master),
        ("worker-12", RoleFamily::Worker),
        ("reviewer-gemini", RoleFamily::Reviewer),
        ("cso-fresh-1700000000", RoleFamily::Cso),
        ("planner", RoleFamily::Planner),
    ] {
        assert_eq!(role_family(role), Some(fam), "{role}");
    }
    // another incarnation (or lane) has its own memory
    let (lane2, inc2) = ad_daemon_key("r2");
    assert_eq!(
        seat_caller(&lane2, &inc2, 43, Some("reviewer-codex".into())).bound_family,
        Some(RoleFamily::Reviewer)
    );
}

/// AD1 wiring: the daemon's recording call binds, a refused verb names the binding, and the entry
/// goes with the surface (the close effect's release call); `handlers.rs` records at claim_role.
#[test]
fn ad1_binding_lives_on_the_daemon_and_dies_with_the_seat() {
    let daemon = crate::team_gate_tests::tmp_daemon("t3-ad1", false);
    note_role_claim(&daemon, 77, Some("master"), "worker-9");
    let begin = |d: &Arc<Daemon>| -> Value {
        let Reply::Single(v) = handle(
            d,
            "begin",
            &json!({"trigger": "owner_order", "files": ["bin/a.py"]}),
            &json!(1),
            Some(77),
            false,
            None,
        ) else {
            panic!("single reply")
        };
        v
    };
    let v = begin(&daemon);
    assert!(
        v["error"]["message"].as_str().unwrap_or("").contains("AD1"),
        "{v}"
    );
    release_locks_for_seat(&daemon, 77);
    let v = begin(&daemon);
    let msg = v["error"]["message"].as_str().unwrap_or("").to_string();
    assert!(!msg.contains("AD1"), "the binding outlived the seat: {v}");
    let handlers = include_str!("handlers.rs");
    let arm = &handlers[handlers.find("\"system.claim_role\" => {").unwrap()..];
    let arm = &arm[..arm.find("*srole = Some(final_role.clone());").unwrap()];
    assert_eq!(
        arm.matches("crate::repair::note_role_claim(daemon, sid, srole.as_deref(), &final_role);")
            .count(),
        1
    );
    // AK2: and where `reclaim_commit` sets a role
    let reclaim = &handlers[handlers.find("\nfn reclaim_commit(").unwrap()..];
    let reclaim = &reclaim[..reclaim.find("\n}\n").unwrap()];
    assert_eq!(
        reclaim
            .matches("crate::repair::note_role_claim(daemon, caller_sid, caller.role.lock().unwrap().as_deref(), role);")
            .count(),
        1
    );
    assert_eq!(
        handlers.matches("crate::repair::note_role_claim(").count(),
        2
    );
}

/// AD2: on Windows nothing about a seatless caller is observed, so the real decision gives
/// `Detached` — and `revert` / `cleanup` answer the sentence that points to the app's button; the
/// app's operator token still passes; on macOS the same caller gets the detached sentence.
#[test]
fn ad2_windows_seatless_refusal_is_reachable() {
    let r = rig("ad2");
    let windows_cli = seatless_caller(false, &SeatlessObservation::default());
    assert_eq!(windows_cli.kind, CallerKind::Detached);
    let app_button = seatless_caller(true, &SeatlessObservation::default());
    for verb in ["revert", "cleanup"] {
        let mut ctx = r.ctx(T0, &general, &pass);
        ctx.windows = true;
        let e = handle_with(&ctx, &windows_cli, verb, &json!({"repair": "R-1"})).unwrap_err();
        assert_eq!(e.code, "acl_denied");
        assert_eq!(e.message, WINDOWS_SEATLESS_REFUSAL_KO, "{verb}");
        assert!(role_check(verb, &app_button, true).is_ok());
        let e = role_check(verb, &windows_cli, false).unwrap_err();
        assert_ne!(e.message, WINDOWS_SEATLESS_REFUSAL_KO);
        assert!(e.message.contains("좌석도 터미널도 없는"), "{}", e.message);
    }
}

/// AD3 / AD4 (rewritten for RULINGS AK3 / AK4: a later repair that brings the path back no longer
/// removes an EARLIER repair's tombstone, and it supersedes that repair): a committed deletion is
/// checked by its state (no check child runs), the record says `absent-state`, the lane gets ONE
/// File tombstone; a present target records the check kind that ran.
#[test]
fn ad3_ad4_deletion_is_tombstoned_and_checked_by_state() {
    let r = rig("ad3");
    let runs = AtomicU64::new(0);
    let counting = |rel: &str, live: &Path, d: std::time::Duration| {
        runs.fetch_add(1, Ordering::Relaxed);
        pass(rel, live, d)
    };
    let call = |now: f64, who: &Caller, sub: &str, p: Value| {
        handle_with(&r.ctx(now, &general, &counting), who, sub, &p)
    };
    let w = seat(3, "worker");
    let id = call(
        T0,
        &w,
        "begin",
        json!({"trigger": "owner_order", "files": ["bin/a.py"]}),
    )
    .unwrap()["repair"]
        .as_str()
        .unwrap()
        .to_string();
    std::fs::remove_file(r.pack.join("bin/a.py")).unwrap();
    call(
        T0 + 1.0,
        &w,
        "record",
        json!({"repair": id, "note": "delete it"}),
    )
    .unwrap();
    call(
        T0 + 2.0,
        &seat(7, "reviewer-codex"),
        "attest_verify",
        json!({"repair": id, "verdict": "pass", "note": "gone is right"}),
    )
    .unwrap();
    call(T0 + 3.0, &w, "commit", json!({"repair": id})).unwrap();
    assert_eq!(
        runs.load(Ordering::Relaxed),
        0,
        "a check child ran for an absent target"
    );
    let rec = r.record_of(&id);
    assert_eq!(
        rec.checks,
        [("bin/a.py".to_string(), "absent-state".to_string())]
    );
    let ts = cys::repair::tombstones::load_in(&r.root, &r.lane).unwrap();
    assert_eq!(ts.len(), 1);
    assert_eq!(ts[0].rel, "bin/a.py");
    assert_eq!(ts[0].kind, cys::repair::tombstones::TombstoneKind::File);
    assert_eq!(ts[0].repair.as_str(), id);
    // a later repair brings the file back → the earlier repair's tombstone stays (AK3), the earlier
    // repair is superseded (AK4)
    let id2 = call(
        T0 + 10.0,
        &w,
        "begin",
        json!({"trigger": "owner_order", "files": ["bin/a.py"]}),
    )
    .unwrap()["repair"]
        .as_str()
        .unwrap()
        .to_string();
    assert_ne!(id2, id, "an in-force repair is not picked up (AD7)");
    std::fs::write(r.pack.join("bin/a.py"), b"print('back')\n").unwrap();
    call(
        T0 + 11.0,
        &w,
        "record",
        json!({"repair": id2, "note": "restore"}),
    )
    .unwrap();
    call(
        T0 + 12.0,
        &seat(7, "reviewer-codex"),
        "attest_verify",
        json!({"repair": id2, "verdict": "pass", "note": "ok"}),
    )
    .unwrap();
    let c = call(T0 + 13.0, &w, "commit", json!({"repair": id2})).unwrap();
    assert_eq!(runs.load(Ordering::Relaxed), 1);
    assert_eq!(c["superseded"], json!([id]));
    assert_eq!(r.record_of(&id).state, RepairState::Superseded);
    let ts = cys::repair::tombstones::load_in(&r.root, &r.lane).unwrap();
    assert_eq!(ts.len(), 1, "another repair's tombstone was removed");
    assert_eq!(ts[0].repair.as_str(), id);
    assert_eq!(
        r.record_of(&id2).checks,
        [("bin/a.py".to_string(), "py-compile".to_string())]
    );
}

/// AD4: the registrar's report confirms a hook line only when it wants it unregistered and every
/// settings file it touched says so; anything else is no check (never claimed).
#[test]
fn ad4_hook_line_confirmed_only_by_the_registrar_report() {
    use super::ledger_verbs::hook_line_confirmed;
    use cys::repair::apply::{ReconcileHook, ReconcileReport};
    let hook = |desired: &str, targets: &[&str]| ReconcileHook {
        event: "Stop".into(),
        script: "x.sh".into(),
        lifeline: false,
        desired: desired.into(),
        targets: targets
            .iter()
            .map(|t| (PathBuf::from("/s/settings.json"), t.to_string()))
            .collect(),
    };
    let rep = |h: Vec<ReconcileHook>| ReconcileReport { hooks: h };
    assert!(hook_line_confirmed(
        &rep(vec![hook(
            "unregistered",
            &["unregistered", "unregistered"]
        )]),
        "Stop",
        "x.sh"
    ));
    assert!(!hook_line_confirmed(
        &rep(vec![hook(
            "unregistered",
            &["unregistered", "refused:busy"]
        )]),
        "Stop",
        "x.sh"
    ));
    assert!(!hook_line_confirmed(
        &rep(vec![hook("unknown", &["unregistered"])]),
        "Stop",
        "x.sh"
    ));
    assert!(!hook_line_confirmed(
        &rep(vec![hook("unregistered", &[])]),
        "Stop",
        "x.sh"
    ));
    assert!(!hook_line_confirmed(
        &rep(vec![hook("unregistered", &["unregistered"])]),
        "Stop",
        "y.sh"
    ));
    assert!(!hook_line_confirmed(&rep(vec![]), "Stop", "x.sh"));
}

/// AD6 (rewritten by T4 — the owner-token gate replaced the interim not-implemented refusal): rule
/// documents (C10 names, any folder, any case, `<x>_directive.md`) are refused at `begin` without an
/// owner-instruction token — here mixed with a general file — with T4's rule-document sentence, and
/// nothing is written.
#[test]
fn ad6_rule_documents_fail_closed() {
    let r = rig("ad6");
    let w = seat(3, "worker");
    for rel in [
        "soul.md",
        "CLAUDE.md",
        "agents/MASTER_DIRECTIVE.md",
        "docs/route_contract.md",
        "Vibecoding_Constitution.md",
    ] {
        assert!(rule_document_name(rel).is_some(), "{rel}");
        let e = r
            .call(
                T0,
                &w,
                "begin",
                json!({"trigger": "owner_order", "files": ["bin/a.py", rel]}),
            )
            .unwrap_err();
        assert_eq!(e.code, "repair_precondition", "{rel}");
        assert!(e.message.contains("규칙 문서"), "{}", e.message);
        assert!(e.message.contains(rel), "the refusal names the file: {}", e.message);
    }
    for rel in [
        "bin/a.py",
        "_directive.md",
        "soul.md.bak",
        "docs/soul.markdown",
    ] {
        assert!(rule_document_name(rel).is_none(), "{rel}");
    }
    assert!(!r.root.exists(), "a refused begin wrote something");
}

/// AD10: `--trigger` is required (no default); nothing is written without it.
#[test]
fn ad10_begin_requires_a_trigger() {
    let r = rig("ad10-trigger");
    let e = r
        .call(
            T0,
            &seat(3, "worker"),
            "begin",
            json!({"files": ["bin/a.py"]}),
        )
        .unwrap_err();
    assert_eq!(e.code, "invalid_params");
    assert!(e.message.contains("--trigger"), "{}", e.message);
    assert!(!r.root.exists());
    for t in ["owner_order", "owner-order", "update"] {
        let r = rig("ad10-ok");
        let b = r
            .call(
                T0,
                &seat(3, "worker"),
                "begin",
                json!({"files": ["bin/a.py"], "trigger": t}),
            )
            .unwrap();
        assert_eq!(b["begin"], "new", "{t}");
    }
}

/// AD10 / R3: `commit` does not hold the per-repair mutex while its check child runs, and aborts
/// when the record or the live file changed meanwhile (nothing committed, no lane row).
#[test]
fn ad10_commit_releases_the_mutex_around_the_check() {
    fn prepared(tag: &str) -> (Rig, String) {
        let r = rig(tag);
        let w = seat(3, "worker");
        let id = r
            .call(
                T0,
                &w,
                "begin",
                json!({"trigger": "owner_order", "files": ["bin/a.py"]}),
            )
            .unwrap()["repair"]
            .as_str()
            .unwrap()
            .to_string();
        std::fs::write(r.pack.join("bin/a.py"), b"print('fixed')\n").unwrap();
        r.call(T0 + 1.0, &w, "record", json!({"repair": id, "note": "n"}))
            .unwrap();
        r.call(
            T0 + 2.0,
            &seat(7, "reviewer-codex"),
            "attest_verify",
            json!({"repair": id, "verdict": "pass", "note": "ok"}),
        )
        .unwrap();
        (r, id)
    }
    let free_during_check = |r: &Rig, id: &str| -> bool {
        let m = r
            .mutexes
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .expect("the repair's mutex exists");
        let free = m.try_lock().is_ok();
        free
    };
    // (a) the mutex is free while the check runs; the commit then succeeds
    let (r, id) = prepared("ad10-free");
    let seen = std::sync::Mutex::new(None);
    let check = |rel: &str, live: &Path, d: std::time::Duration| {
        *seen.lock().unwrap() = Some(free_during_check(&r, &id));
        pass(rel, live, d)
    };
    handle_with(
        &r.ctx(T0 + 3.0, &general, &check),
        &seat(3, "worker"),
        "commit",
        &json!({"repair": id}),
    )
    .unwrap();
    assert_eq!(
        *seen.lock().unwrap(),
        Some(true),
        "the mutex was held across the check child"
    );
    assert_eq!(r.record_of(&id).state, RepairState::Probation);
    // (b) the record changes while the check runs → aborted, nothing committed
    let (r, id) = prepared("ad10-record");
    let check = |rel: &str, live: &Path, d: std::time::Duration| {
        let mut rec = r.record_of(&id);
        rec.methods.push("written meanwhile".into());
        ledger::write_record_in(&r.root, &rec).unwrap();
        pass(rel, live, d)
    };
    let e = handle_with(
        &r.ctx(T0 + 3.0, &general, &check),
        &seat(3, "worker"),
        "commit",
        &json!({"repair": id}),
    )
    .unwrap_err();
    assert_eq!(e.code, "target_changed");
    assert!(
        e.message.contains("수리 기록이 바뀌었습니다"),
        "{}",
        e.message
    );
    let rec = r.record_of(&id);
    assert_eq!(rec.state, RepairState::InProgress);
    assert!(rec.committed.is_empty() && rec.lock.is_some());
    assert!(ledger::read_lane_in(&r.root, &rec.id, &r.lane)
        .unwrap()
        .is_none());
    // (c) the live file changes while the check runs → aborted
    let (r, id) = prepared("ad10-live");
    let check = |rel: &str, live: &Path, d: std::time::Duration| {
        std::fs::write(r.pack.join("bin/a.py"), b"print('moved')\n").unwrap();
        pass(rel, live, d)
    };
    let e = handle_with(
        &r.ctx(T0 + 3.0, &general, &check),
        &seat(3, "worker"),
        "commit",
        &json!({"repair": id}),
    )
    .unwrap_err();
    assert_eq!(e.code, "target_changed");
    assert!(
        e.message.contains("살아 있는 파일이 바뀌었습니다"),
        "{}",
        e.message
    );
    assert_eq!(r.record_of(&id).state, RepairState::InProgress);
}

// ───────────────────────────── RULINGS AK (re-check of the fix round) ─────────────────────────────

/// AK1: no exemption — a worker pane re-declared `master` is refused `grant` and `owner-token` (and
/// `begin`) as AD1 rules, and the sentence tells the way back.
#[test]
fn ak1_worker_redeclared_master_refused_with_the_way_back() {
    let r = rig("ak1");
    let (lane, inc) = ad_daemon_key("ak1");
    seat_caller(&lane, &inc, 51, Some("worker".into()));
    record_role_claim(&lane, &inc, 51, Some("worker"), "master");
    let master = seat_caller(&lane, &inc, 51, Some("master".into()));
    assert_eq!(master.bound_family, Some(RoleFamily::Worker));
    for (verb, p) in [
        ("grant", json!({"on": true})),
        ("owner_token", json!({"path": "soul.md"})),
        (
            "begin",
            json!({"trigger": "owner_order", "files": ["bin/a.py"]}),
        ),
    ] {
        let e = r.call(T0, &master, verb, p).unwrap_err();
        assert_eq!(e.code, "acl_denied", "{verb}");
        assert!(e.message.contains("AD1"), "{verb}: {}", e.message);
        assert!(
            e.message.contains("이 창을 닫고 새 창에서"),
            "{verb}: the way back is missing: {}",
            e.message
        );
    }
    assert!(
        r.call(T0, &master, "status", json!({})).is_ok(),
        "READ stays open"
    );
}

/// AK3: a target that was absent before the repair and is absent at commit is no deletion (no
/// tombstone); a repair whose deletion was committed and that later commits the path present again
/// removes its OWN tombstone; an unreadable tombstone file is only a warning for a present target.
#[test]
fn ak3_only_real_deletions_and_own_tombstones() {
    let r = rig("ak3");
    let w = seat(3, "worker");
    let rv = seat(7, "reviewer-codex");
    let flow = |files: Value, edit: &dyn Fn()| -> (String, Value) {
        let id = r
            .call(
                T0,
                &w,
                "begin",
                json!({"trigger": "owner_order", "files": files}),
            )
            .unwrap()["repair"]
            .as_str()
            .unwrap()
            .to_string();
        edit();
        r.call(T0 + 1.0, &w, "record", json!({"repair": id, "note": "n"}))
            .unwrap();
        r.call(
            T0 + 2.0,
            &rv,
            "attest_verify",
            json!({"repair": id, "verdict": "pass", "note": "ok"}),
        )
        .unwrap();
        let c = r
            .call(T0 + 3.0, &w, "commit", json!({"repair": id}))
            .unwrap();
        (id, c)
    };
    // (a) absent before, absent at commit → nothing removed, no tombstone
    let (_, c) = flow(json!(["bin/new.py"]), &|| {});
    assert_eq!(c["state"], "probation", "{c}");
    assert!(cys::repair::tombstones::load_in(&r.root, &r.lane)
        .unwrap()
        .is_empty());
    // (b) a real deletion → its tombstone; reopened by the audit, picked up, the file restored and
    //     committed again → its own tombstone goes
    let (id, _) = flow(json!(["bin/a.py"]), &|| {
        std::fs::remove_file(r.pack.join("bin/a.py")).unwrap()
    });
    assert_eq!(
        cys::repair::tombstones::load_in(&r.root, &r.lane)
            .unwrap()
            .len(),
        1
    );
    r.call(
        T0 + 4.0,
        &seat(8, "reviewer-gemini"),
        "attest_audit",
        json!({"repair": id, "verdict": "finding", "note": "keep the file"}),
    )
    .unwrap();
    let p = r
        .call(
            T0 + 5.0,
            &w,
            "begin",
            json!({"trigger": "owner_order", "files": ["bin/a.py"]}),
        )
        .unwrap();
    assert_eq!(p["begin"], "picked-up");
    std::fs::write(r.pack.join("bin/a.py"), b"print('kept')\n").unwrap();
    r.call(
        T0 + 6.0,
        &w,
        "record",
        json!({"repair": id, "note": "restored"}),
    )
    .unwrap();
    r.call(
        T0 + 7.0,
        &rv,
        "attest_verify",
        json!({"repair": id, "verdict": "pass", "note": "ok"}),
    )
    .unwrap();
    r.call(T0 + 8.0, &w, "commit", json!({"repair": id}))
        .unwrap();
    assert!(
        cys::repair::tombstones::load_in(&r.root, &r.lane)
            .unwrap()
            .is_empty(),
        "the repair's own tombstone stayed"
    );
    // (c) an unreadable tombstone file: a present commit stands (검증됨) with a warning
    let r2 = rig("ak3-broken");
    std::fs::create_dir_all(r2.root.join("tombstones")).unwrap();
    std::fs::write(r2.root.join("tombstones").join("base.json"), b"{broken").unwrap();
    let id2 = r2
        .call(
            T0,
            &w,
            "begin",
            json!({"trigger": "owner_order", "files": ["bin/a.py"]}),
        )
        .unwrap()["repair"]
        .as_str()
        .unwrap()
        .to_string();
    std::fs::write(r2.pack.join("bin/a.py"), b"print('fixed')\n").unwrap();
    r2.call(T0 + 1.0, &w, "record", json!({"repair": id2, "note": "n"}))
        .unwrap();
    r2.call(
        T0 + 2.0,
        &rv,
        "attest_verify",
        json!({"repair": id2, "verdict": "pass", "note": "ok"}),
    )
    .unwrap();
    let c = r2
        .call(T0 + 3.0, &w, "commit", json!({"repair": id2}))
        .unwrap();
    assert_eq!(c["warnings"].as_array().unwrap().len(), 1, "{c}");
    let row = ledger::read_lane_in(&r2.root, &RepairId::parse(&id2).unwrap(), &r2.lane)
        .unwrap()
        .unwrap();
    assert_eq!(
        row.state,
        LaneState::Verified,
        "a broken tombstone file made a 충돌 row"
    );
    assert_eq!(
        std::fs::read(r2.root.join("tombstones").join("base.json")).unwrap(),
        b"{broken",
        "the unreadable file was overwritten"
    );
}

/// AK4: committing a newer repair on the same file supersedes the older one in force; the older one
/// is then never reopened by an audit and never picked up.
#[test]
fn ak4_newer_commit_supersedes_the_older_in_force() {
    let r = rig("ak4");
    let w = seat(3, "worker");
    let older = committed_general(&r);
    // a repair in force on ANOTHER file (older too) must be left alone
    let other = r
        .call(
            T0 + 5.0,
            &w,
            "begin",
            json!({"trigger": "owner_order", "files": ["hooks/h.sh"]}),
        )
        .unwrap()["repair"]
        .as_str()
        .unwrap()
        .to_string();
    std::fs::write(r.pack.join("hooks/h.sh"), b"#!/bin/sh\necho h2\n").unwrap();
    r.call(
        T0 + 6.0,
        &w,
        "record",
        json!({"repair": other, "note": "n"}),
    )
    .unwrap();
    r.call(
        T0 + 7.0,
        &seat(7, "reviewer-codex"),
        "attest_verify",
        json!({"repair": other, "verdict": "pass", "note": "ok"}),
    )
    .unwrap();
    r.call(T0 + 8.0, &w, "commit", json!({"repair": other}))
        .unwrap();
    assert_eq!(r.record_of(&other).state, RepairState::Probation);
    let b = r
        .call(
            T0 + 10.0,
            &w,
            "begin",
            json!({"trigger": "owner_order", "files": ["bin/a.py"]}),
        )
        .unwrap();
    assert_eq!(b["begin"], "new");
    let newer = b["repair"].as_str().unwrap().to_string();
    std::fs::write(r.pack.join("bin/a.py"), b"print('better')\n").unwrap();
    r.call(
        T0 + 11.0,
        &w,
        "record",
        json!({"repair": newer, "note": "n"}),
    )
    .unwrap();
    r.call(
        T0 + 12.0,
        &seat(7, "reviewer-codex"),
        "attest_verify",
        json!({"repair": newer, "verdict": "pass", "note": "ok"}),
    )
    .unwrap();
    let c = r
        .call(T0 + 13.0, &w, "commit", json!({"repair": newer}))
        .unwrap();
    assert_eq!(c["superseded"], json!([older]));
    assert_eq!(r.record_of(&older).state, RepairState::Superseded);
    assert_eq!(r.record_of(&newer).state, RepairState::Probation);
    let e = r
        .call(
            T0 + 14.0,
            &seat(8, "reviewer-gemini"),
            "attest_audit",
            json!({"repair": older, "verdict": "finding", "note": "stale"}),
        )
        .unwrap_err();
    assert_eq!(e.code, "repair_precondition");
    assert!(e.message.contains("대체된"), "{}", e.message);
    assert_eq!(
        r.record_of(&older).state,
        RepairState::Superseded,
        "an audit reopened it"
    );
    let p = r
        .call(
            T0 + 15.0,
            &w,
            "begin",
            json!({"trigger": "owner_order", "files": ["bin/a.py"]}),
        )
        .unwrap();
    assert_eq!(p["begin"], "new");
    assert_ne!(
        p["repair"],
        json!(older),
        "a superseded repair was picked up"
    );
    assert_eq!(
        r.record_of(&other).state,
        RepairState::Probation,
        "a repair on another file was superseded"
    );
    assert_eq!(r.record_of(&newer).state, RepairState::Probation);
}

/// AK5 (and AD4 end to end): the hook step runs WITHOUT the per-repair mutex; a confirming report is
/// recorded as `hook-registrar`, a non-confirming one is never claimed and makes the row 충돌.
#[test]
fn ak5_hook_step_runs_without_the_mutex() {
    use cys::repair::apply::{HookStep, ReconcileHook, ReconcileReport};
    fn with_hook_line(tag: &str) -> (Rig, String) {
        let r = rig(tag);
        let w = seat(3, "worker");
        let id = r
            .call(
                T0,
                &w,
                "begin",
                json!({"trigger": "owner_order", "files": ["bin/a.py"]}),
            )
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
        std::fs::write(r.pack.join("bin/a.py"), b"print('fixed')\n").unwrap();
        r.call(T0 + 1.0, &w, "record", json!({"repair": id, "note": "n"}))
            .unwrap();
        r.call(
            T0 + 2.0,
            &seat(7, "reviewer-codex"),
            "attest_verify",
            json!({"repair": id, "verdict": "pass", "note": "ok"}),
        )
        .unwrap();
        (r, id)
    }
    let report = |targets: &[&str]| ReconcileReport {
        hooks: vec![ReconcileHook {
            event: "Stop".into(),
            script: "x.sh".into(),
            lifeline: false,
            desired: "unregistered".into(),
            targets: targets
                .iter()
                .map(|t| (PathBuf::from("/s/settings.json"), t.to_string()))
                .collect(),
        }],
    };
    // (a) confirmed
    let (r, id) = with_hook_line("ak5-ok");
    let free = std::sync::Mutex::new(None);
    let step = |rid: &RepairId, _lane: &LaneKey, s: HookStep| {
        assert_eq!(s, HookStep::Unregister);
        let m = r
            .mutexes
            .lock()
            .unwrap()
            .get(rid.as_str())
            .cloned()
            .unwrap();
        let ok = m.try_lock().is_ok();
        *free.lock().unwrap() = Some(ok);
        Ok(report(&["unregistered"]))
    };
    let mut ctx = r.ctx(T0 + 3.0, &general, &pass);
    ctx.hook_step = &step;
    let c = handle_with(&ctx, &seat(3, "worker"), "commit", &json!({"repair": id})).unwrap();
    assert_eq!(
        *free.lock().unwrap(),
        Some(true),
        "the mutex was held across the hook step"
    );
    assert_eq!(c["hook_step"], "ok", "{c}");
    let rec = r.record_of(&id);
    assert!(rec.checks.contains(&(
        "settings:Stop:x.sh".to_string(),
        "hook-registrar".to_string()
    )));
    let row = ledger::read_lane_in(&r.root, &rec.id, &r.lane)
        .unwrap()
        .unwrap();
    assert_eq!(row.state, LaneState::Verified);
    // (b) not confirmed → never claimed, 충돌
    let (r, id) = with_hook_line("ak5-no");
    let step = |_: &RepairId, _: &LaneKey, _: HookStep| Ok(report(&["refused:busy"]));
    let mut ctx = r.ctx(T0 + 3.0, &general, &pass);
    ctx.hook_step = &step;
    let c = handle_with(&ctx, &seat(3, "worker"), "commit", &json!({"repair": id})).unwrap();
    assert!(
        c["hook_step"]["error"]
            .as_str()
            .unwrap()
            .contains("확인되지"),
        "{c}"
    );
    let rec = r.record_of(&id);
    assert!(!rec.checks.iter().any(|(_, how)| how == "hook-registrar"));
    let row = ledger::read_lane_in(&r.root, &rec.id, &r.lane)
        .unwrap()
        .unwrap();
    assert_eq!(row.state, LaneState::Conflict);
}

/// AK6: Windows path aliases of a file are refused at `begin` (they would escape the rule-document
/// classification); nothing is written.
#[test]
fn ak6_windows_path_aliases_refused() {
    let r = rig("ak6");
    for rel in [
        "agents/MASTER_DIRECTIVE.md.",
        "soul.md ",
        "soul.md::$DATA",
        "bin/a.py:x",
        "bin./a.py",
    ] {
        assert!(is_windows_alias(rel), "{rel:?}");
        let e = r
            .call(
                T0,
                &seat(3, "worker"),
                "begin",
                json!({"trigger": "owner_order", "files": [rel]}),
            )
            .unwrap_err();
        assert_eq!(e.code, "invalid_params", "{rel:?}");
    }
    for rel in ["bin/a.py", "hooks/h.sh", "docs/a.b.md"] {
        assert!(!is_windows_alias(rel), "{rel:?}");
    }
    assert!(!r.root.exists(), "a refused begin wrote something");
}

// ───────────────────────────── track T3c (RULINGS AQ1, AQ11, AU4, AW3) ─────────────────────────────

/// A rig on the same ledger and pack as `r`, seen by a daemon of another incarnation (a restart:
/// every lock of `r`'s incarnation is released).
fn restarted(r: &Rig, inc: &str) -> Rig {
    let mut x = rig("t3c-restart-unused");
    x.root = r.root.clone();
    x.pack = r.pack.clone();
    x.inc = inc.into();
    x
}

/// RULINGS AQ1: TRIGGER verbs are outside the role-family binding — a re-declared seat may call
/// `sweep` / `tick` (their bodies are another track's); every other non-READ/INTAKE verb stays
/// refused.
#[test]
fn aq1_trigger_verbs_outside_the_binding() {
    let r = rig("aq1");
    let (lane, inc) = ad_daemon_key("aq1");
    seat_caller(&lane, &inc, 61, Some("worker".into()));
    record_role_claim(&lane, &inc, 61, Some("worker"), "master");
    let redeclared = seat_caller(&lane, &inc, 61, Some("master".into()));
    assert_eq!(redeclared.bound_family, Some(RoleFamily::Worker));
    for verb in ["sweep", "tick"] {
        assert!(role_check(verb, &redeclared, false).is_ok(), "{verb}");
        let e = r.call(T0, &redeclared, verb, json!({})).unwrap_err();
        assert_ne!(e.code, "acl_denied", "{verb}: {}", e.message);
        assert!(!e.message.contains("AD1"), "{verb}: {}", e.message);
    }
    for verb in ["begin", "grant", "attest_verify"] {
        let e = role_check(verb, &redeclared, false).unwrap_err();
        assert!(e.message.contains("AD1"), "{verb}: {}", e.message);
    }
}

/// `begin` on the boot-critical `hooks/h.sh` (classified by the stand-in list `hooks_boot`).
fn begin_bc(r: &Rig, now: f64, who: &Caller) -> Result<Value, VerbError> {
    handle_with(
        &r.ctx(now, &hooks_boot, &pass),
        who,
        "begin",
        &json!({"trigger": "owner_order", "files": ["hooks/h.sh"]}),
    )
}

/// The working copy of `hooks/h.sh` of repair `id`.
fn staged(r: &Rig, id: &str) -> PathBuf {
    ledger::staging_dir_in(&r.root, &RepairId::parse(id).unwrap()).join("hooks/h.sh")
}

/// A boot-critical repair of `hooks/h.sh` put into 승인 대기 as T5b's `submit` leaves it (RULINGS
/// BJ4): a working copy holding the worker's candidate, a pending request, signatures, intake
/// signatures — with the lock of seat 3 in incarnation `inc-1`.
fn awaiting_approval(r: &Rig, grant: bool) -> (String, RepairRecord) {
    let w = seat(3, "worker");
    // a grant start is refused while T4's claim is a stub: the repair is started as an owner-order
    // one and turned into a grant one (the pick-up rule is what is under test)
    let id = begin_bc(r, T0, &w).unwrap()["repair"]
        .as_str()
        .unwrap()
        .to_string();
    std::fs::write(staged(r, &id), b"#!/bin/sh\necho candidate\n").unwrap();
    let mut rec = r.record_of(&id);
    assert_eq!(rec.classification, Classification::BootCritical);
    if grant {
        rec.trigger = cys::repair::types::Trigger::Grant;
    }
    rec.state = RepairState::AwaitingApproval;
    rec.pending_request = Some(("fp-pending".into(), T0 + 600.0));
    rec.verify = Some(cys::repair::ledger::Attestation {
        seat: 7,
        role: "reviewer-codex".into(),
        verdict: "pass".into(),
        note: "ok".into(),
        at: T0,
        fingerprint_sha256: "f".into(),
    });
    rec.intake_signatures = vec!["sig-1".into()];
    ledger::write_record_in(&r.root, &rec).unwrap();
    let rec = r.record_of(&id);
    (id, rec)
}

/// RULINGS AQ11 / BJ4: a lock-less 승인 대기 repair is PICKED UP (never a second open repair on the
/// file); the pick-up changes only the lock — state, signatures, the pending request, the working
/// copy and the pre-images stay — by the same seat and by another worker seat after a restart;
/// within one incarnation the holder's retry is "already-held" and another seat is refused.
#[test]
fn aq11_awaiting_approval_is_picked_up_changing_only_the_lock() {
    let r = rig("aq11");
    let (id, before) = awaiting_approval(&r, false);
    let candidate = std::fs::read(staged(&r, &id)).unwrap();
    let w = seat(3, "worker");
    let again = begin_bc(&r, T0 + 1.0, &w).unwrap();
    assert_eq!(again["begin"], "already-held");
    let e = begin_bc(&r, T0 + 1.0, &seat(13, "worker-3")).unwrap_err();
    assert_eq!(e.code, "repair_locked");
    // a restart releases the lock: the same seat picks it up
    let r2 = restarted(&r, "inc-2");
    let p = begin_bc(&r2, T0 + 2.0, &w).unwrap();
    assert_eq!(p["begin"], "picked-up");
    assert_eq!(p["repair"], json!(id));
    let after = r.record_of(&id);
    assert_eq!(
        after.state,
        RepairState::AwaitingApproval,
        "the state changed"
    );
    let lock = after.lock.clone().unwrap();
    assert_eq!(
        (lock.owner.seat, lock.owner.incarnation.as_str()),
        (3, "inc-2")
    );
    let mut same = after.clone();
    same.lock = before.lock.clone();
    assert_eq!(same, before, "a pick-up changed more than the lock");
    assert_eq!(
        after.pending_request,
        Some(("fp-pending".to_string(), T0 + 600.0))
    );
    assert_eq!(
        std::fs::read(staged(&r, &id)).unwrap(),
        candidate,
        "the working copy changed"
    );
    // another restart: another worker seat picks it up
    let r3 = restarted(&r, "inc-3");
    let p = begin_bc(&r3, T0 + 3.0, &seat(13, "worker-3")).unwrap();
    assert_eq!(p["begin"], "picked-up");
    let after = r.record_of(&id);
    assert_eq!(after.lock.as_ref().unwrap().owner.seat, 13);
    assert_eq!(after.state, RepairState::AwaitingApproval);
    assert!(after.pending_request.is_some());
    assert_eq!(std::fs::read(staged(&r, &id)).unwrap(), candidate);
    assert_eq!(
        ledger::list_ids_in(&r.root).len(),
        1,
        "a second open repair was made"
    );
}

/// RULINGS AQ11 / K3 / audit major 8: picking up a lock-less 승인 대기 GRANT repair (no
/// `--resolves`) takes a NEW claim; CapReached / Busy / Paused / LostRace answer `claim_for`'s
/// sentences and leave the repair exactly as it was (no lock, no second repair); Allowed binds
/// the new claim to the new lock.
#[test]
fn aq11_grant_pick_up_takes_a_new_claim() {
    use cys::repair::grant::StartVerdict;
    let r = rig("aq11-grant");
    let (id, before) = awaiting_approval(&r, true);
    assert_eq!(before.trigger, cys::repair::types::Trigger::Grant);
    let r2 = restarted(&r, "inc-2");
    fn begin_with(r2: &Rig, claim: &ClaimFn<'_>) -> Result<Value, VerbError> {
        let mut ctx = r2.ctx(T0 + 2.0, &hooks_boot, &pass);
        ctx.claim = claim;
        handle_with(
            &ctx,
            &seat(3, "worker"),
            "begin",
            &json!({"trigger": "grant", "files": ["hooks/h.sh"]}),
        )
    }
    for (verdict, code, sentence) in [
        (
            StartVerdict::CapReached,
            "repair_precondition",
            "오늘의 자동 수리 한도에 이르렀습니다",
        ),
        (
            StartVerdict::Busy,
            "repair_locked",
            "다른 자동 수리가 진행 중입니다(동시 1건)",
        ),
        (
            StartVerdict::Paused,
            "repair_precondition",
            "자동 수리 허락이 일시정지 중입니다",
        ),
        (
            StartVerdict::LostRace,
            "repair_locked",
            "다른 데몬이 먼저 차례를 잡았습니다 — 나중에 다시",
        ),
    ] {
        let asked = std::cell::Cell::new(0);
        let claim = |_: &LockOwner, rid: &RepairId| -> Result<StartVerdict, RepairError> {
            assert_eq!(
                rid.as_str(),
                id,
                "the claim is asked for the repair being picked up"
            );
            asked.set(asked.get() + 1);
            Ok(verdict)
        };
        let e = begin_with(&r2, &claim).unwrap_err();
        assert_eq!(asked.get(), 1);
        // what the worker sees: the daemon prefixes the verb (`handle_with`), the CLI prints the rest
        assert_eq!(
            (e.code.as_str(), e.message.as_str()),
            (code, format!("repair.begin: {sentence}").as_str())
        );
        assert_eq!(
            r.record_of(&id),
            before,
            "a refused pick-up changed the repair"
        );
        assert_eq!(
            ledger::list_ids_in(&r.root).len(),
            1,
            "a second open repair was made"
        );
    }
    let allowed = |_: &LockOwner, _: &RepairId| -> Result<StartVerdict, RepairError> {
        Ok(StartVerdict::Allowed { claim: 7 })
    };
    let p = begin_with(&r2, &allowed).unwrap();
    assert_eq!(p["begin"], "picked-up");
    let after = r.record_of(&id);
    assert_eq!(after.lock.as_ref().unwrap().claim, Some(7));
    assert_eq!(after.state, RepairState::AwaitingApproval);
}

/// RULINGS AU4: `trusted-keys.json` (the pack's `SYSTEM_LOCKED` file, any case) is refused at
/// `begin`, also inside a multi-file `begin`, with the pack-heal sentence; nothing is written.
/// AW3 (a): a Windows short name is refused at `begin` too.
#[test]
fn au4_key_file_refused_and_short_names() {
    let r = rig("au4");
    let w = seat(3, "worker");
    for files in [
        json!(["trusted-keys.json"]),
        json!(["bin/a.py", "Trusted-Keys.JSON"]),
    ] {
        let e = r
            .call(
                T0,
                &w,
                "begin",
                json!({"trigger": "owner_order", "files": files}),
            )
            .unwrap_err();
        assert_eq!(e.code, "repair_precondition", "{files}");
        assert!(
            e.message.contains("수리 대상이 아닙니다")
                && e.message.contains("cys pack-heal trusted-keys.json"),
            "{}",
            e.message
        );
    }
    for rel in ["hooks/ROLE-C~1.SH", "bin/JAVIS_~1.PY"] {
        let e = r
            .call(
                T0,
                &w,
                "begin",
                json!({"trigger": "owner_order", "files": [rel]}),
            )
            .unwrap_err();
        assert_eq!(e.code, "invalid_params", "{rel}");
        assert!(e.message.contains("짧은 이름"), "{}", e.message);
    }
    assert!(!r.root.exists(), "a refused begin wrote something");
}

/// RULINGS AW3 (b): `x -> hooks` then `begin --files x/h.sh` is refused; a link made in a target's
/// parent chain between `begin` and `commit` refuses `record` and `commit`.
#[cfg(unix)]
#[test]
fn aw3_parent_chain_links_refused() {
    let r = rig("aw3b");
    let w = seat(3, "worker");
    std::os::unix::fs::symlink(r.pack.join("hooks"), r.pack.join("x")).unwrap();
    let e = r
        .call(
            T0,
            &w,
            "begin",
            json!({"trigger": "owner_order", "files": ["x/h.sh"]}),
        )
        .unwrap_err();
    assert_eq!(e.code, "repair_precondition");
    assert!(e.message.contains("링크"), "{}", e.message);
    assert!(!r.root.exists(), "a refused begin wrote something");
    // begin → record → verify, then `bin` becomes a link to an identical folder
    let id = r
        .call(
            T0,
            &w,
            "begin",
            json!({"trigger": "owner_order", "files": ["bin/a.py"]}),
        )
        .unwrap()["repair"]
        .as_str()
        .unwrap()
        .to_string();
    std::fs::write(r.pack.join("bin/a.py"), b"print('fixed')\n").unwrap();
    r.call(T0 + 1.0, &w, "record", json!({"repair": id, "note": "n"}))
        .unwrap();
    r.call(
        T0 + 2.0,
        &seat(7, "reviewer-codex"),
        "attest_verify",
        json!({"repair": id, "verdict": "pass", "note": "ok"}),
    )
    .unwrap();
    std::fs::rename(r.pack.join("bin"), r.pack.join("bin-real")).unwrap();
    std::os::unix::fs::symlink(r.pack.join("bin-real"), r.pack.join("bin")).unwrap();
    let e = r
        .call(T0 + 3.0, &w, "commit", json!({"repair": id}))
        .unwrap_err();
    assert_eq!(e.code, "repair_precondition");
    assert!(e.message.contains("링크"), "{}", e.message);
    assert_eq!(
        r.record_of(&id).state,
        RepairState::InProgress,
        "a refused commit committed"
    );
    let e = r
        .call(T0 + 4.0, &w, "record", json!({"repair": id, "note": "n"}))
        .unwrap_err();
    assert_eq!(e.code, "repair_precondition");
}

/// RULINGS AW3 (c): a candidate link into any lane's pack or to a directory is refused at `record`
/// and `commit`; a link to a user file outside the packs is allowed and commits.
#[cfg(unix)]
#[test]
fn aw3_link_candidates_into_packs_refused() {
    let r = rig("aw3c");
    let w = seat(3, "worker");
    let base = r.pack.parent().unwrap().to_path_buf();
    let user = base.join("user");
    let dept = base.join("pack-dept-d1");
    std::fs::create_dir_all(&user).unwrap();
    std::fs::create_dir_all(dept.join("bin")).unwrap();
    std::fs::write(user.join("notes.txt"), b"user\n").unwrap();
    std::fs::write(dept.join("bin/a.py"), b"print('d')\n").unwrap();
    let id = r
        .call(
            T0,
            &w,
            "begin",
            json!({"trigger": "owner_order", "files": ["bin/a.py"]}),
        )
        .unwrap()["repair"]
        .as_str()
        .unwrap()
        .to_string();
    let live = r.pack.join("bin/a.py");
    let relink = |to: &Path| {
        let _ = std::fs::remove_file(&live);
        std::os::unix::fs::symlink(to, &live).unwrap();
    };
    for (to, why) in [
        (r.pack.join("hooks/h.sh"), "팩 안의 경로"),
        (dept.join("bin/a.py"), "팩 안의 경로"),
        (user.clone(), "폴더"),
    ] {
        relink(to.as_path());
        let e = r
            .call(T0 + 1.0, &w, "record", json!({"repair": id, "note": "n"}))
            .unwrap_err();
        assert_eq!(e.code, "repair_precondition", "{}", to.display());
        assert!(e.message.contains(why), "{}: {}", to.display(), e.message);
    }
    // RULINGS BJ1: an outside link followed by `..` opens the pack's hooks — resolved physically
    let outside = base.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::os::unix::fs::symlink(r.pack.join("hooks"), outside.join("lnk")).unwrap();
    relink(Path::new(&format!(
        "{}/lnk/../hooks/h.sh",
        outside.display()
    )));
    let e = r
        .call(T0 + 1.0, &w, "record", json!({"repair": id, "note": "n"}))
        .unwrap_err();
    assert!(e.message.contains("팩 안의 경로"), "{}", e.message);
    // a link to a user file outside the packs: recorded, verified, committed
    relink(user.join("notes.txt").as_path());
    r.call(
        T0 + 2.0,
        &w,
        "record",
        json!({"repair": id, "note": "link to the user's notes"}),
    )
    .unwrap();
    r.call(
        T0 + 3.0,
        &seat(7, "reviewer-codex"),
        "attest_verify",
        json!({"repair": id, "verdict": "pass", "note": "ok"}),
    )
    .unwrap();
    // re-pointed into the pack after the verification → commit refused by the rule, not committed
    relink(r.pack.join("hooks/h.sh").as_path());
    let e = r
        .call(T0 + 4.0, &w, "commit", json!({"repair": id}))
        .unwrap_err();
    assert_eq!(e.code, "repair_precondition");
    assert!(e.message.contains("팩 안의 경로"), "{}", e.message);
    relink(user.join("notes.txt").as_path());
    let c = r
        .call(T0 + 5.0, &w, "commit", json!({"repair": id}))
        .unwrap();
    assert_eq!(c["state"], "probation", "{c}");
}

// ───────────────────────────── T3c fix round (RULINGS BJ1–BJ5) ─────────────────────────────

/// RULINGS BJ2: `begin --joins` checks EVERY file of the joined repair (not only those named in
/// `--files`) before anything is read or copied: a linked parent of another file of the joined
/// repair refuses the join; no vault slot is made and the lane row is untouched.
#[cfg(unix)]
#[test]
fn bj2_joins_checks_every_file_of_the_joined_repair() {
    let r = rig("bj2");
    let w = seat(3, "worker");
    let rid = r
        .call(
            T0,
            &w,
            "begin",
            json!({"trigger": "owner_order", "files": ["bin/a.py", "hooks/h.sh"]}),
        )
        .unwrap()["repair"]
        .as_str()
        .unwrap()
        .to_string();
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
        reason: String::new(),
        updated_at: T0,
    };
    ledger::write_lane_in(&r.root, &row).unwrap();
    let id = RepairId::parse(&rid).unwrap();
    let slots_before = ledger::list_slots_in(&r.root, &id).len();
    std::fs::rename(r.pack.join("hooks"), r.pack.join("hooks-real")).unwrap();
    std::os::unix::fs::symlink(r.pack.join("hooks-real"), r.pack.join("hooks")).unwrap();
    let e = r
        .call(
            T0 + 1.0,
            &seat(13, "worker-3"),
            "begin",
            json!({"trigger": "owner_order", "files": ["bin/a.py"], "joins": rid}),
        )
        .unwrap_err();
    assert_eq!(e.code, "repair_precondition", "{}", e.message);
    assert!(
        e.message.contains("hooks/h.sh") && e.message.contains("링크"),
        "{}",
        e.message
    );
    assert_eq!(
        ledger::list_slots_in(&r.root, &id).len(),
        slots_before,
        "a slot was copied"
    );
    let after = ledger::read_lane_in(&r.root, &id, &r.lane)
        .unwrap()
        .unwrap();
    assert_eq!(after, row, "the lane row changed");
}

/// RULINGS BJ3: spellings that fold to a protected name ARE that file on a case-insensitive file
/// system: the key file (Kelvin sign, long s) and rule documents are refused naming the real file.
#[test]
fn bj3_folded_spellings_of_protected_names_refused() {
    let r = rig("bj3");
    let w = seat(3, "worker");
    for rel in ["trusted-\u{212A}eys.json", "tru\u{017F}ted-keys.json"] {
        let e = r
            .call(
                T0,
                &w,
                "begin",
                json!({"trigger": "owner_order", "files": ["bin/a.py", rel]}),
            )
            .unwrap_err();
        assert_eq!(e.code, "repair_precondition", "{rel}");
        assert!(
            e.message.contains("(= trusted-keys.json)")
                && e.message.contains("cys pack-heal trusted-keys.json"),
            "{rel}: {}",
            e.message
        );
    }
    std::fs::write(r.pack.join("SOUL.md"), b"# soul\n").unwrap();
    for (rel, real) in [
        ("\u{017F}oul.md", "SOUL.md"),
        ("agents/MA\u{017F}TER_DIRECTIVE.md", "master_directive.md"),
    ] {
        assert!(rule_document_name(rel).is_some(), "{rel}");
        let e = r
            .call(
                T0,
                &w,
                "begin",
                json!({"trigger": "owner_order", "files": [rel]}),
            )
            .unwrap_err();
        // T4: refused by the rule-document branch (no owner token), naming the real file (BJ3)
        assert_eq!(e.code, "repair_precondition", "{rel}");
        assert!(e.message.contains("주인 지시 표"), "{rel}: {}", e.message);
        assert!(
            e.message.contains(&format!("(= {real})")),
            "{rel}: {}",
            e.message
        );
    }
    assert!(!r.root.exists(), "a refused begin wrote something");
}

/// RULINGS BJ5: one open repair per file — a `begin` whose files overlap a being-worked repair of
/// this lane with another target set, or with another `--resolves`, is refused naming that repair;
/// a 승인 대기 repair counts; the repair a `--resolves` start resolves keeps its own rule; a
/// repair in force does not block.
#[test]
fn bj5_one_open_repair_per_file() {
    let r = rig("bj5");
    let w = seat(3, "worker");
    let r0 = r
        .call(
            T0,
            &w,
            "begin",
            json!({"trigger": "owner_order", "files": ["hooks/h.sh"]}),
        )
        .unwrap()["repair"]
        .as_str()
        .unwrap()
        .to_string();
    let r1 = r
        .call(
            T0,
            &w,
            "begin",
            json!({"trigger": "owner_order", "files": ["bin/a.py"]}),
        )
        .unwrap()["repair"]
        .as_str()
        .unwrap()
        .to_string();
    let other = seat(13, "worker-3");
    // another target set overlapping r1
    let e = r
        .call(
            T0 + 1.0,
            &other,
            "begin",
            json!({"trigger": "owner_order", "files": ["bin/a.py", "bin/b.py"]}),
        )
        .unwrap_err();
    assert_eq!(e.code, "repair_precondition");
    assert!(
        e.message.contains(&r1) && e.message.contains(&format!("--joins {r1}")),
        "{}",
        e.message
    );
    // another `--resolves` on r1's file
    let e = r
        .call(
            T0 + 1.0,
            &other,
            "begin",
            json!({"trigger": "owner_order", "files": ["bin/a.py"], "resolves": r0}),
        )
        .unwrap_err();
    assert!(e.message.contains(&r1), "{}", e.message);
    // the repair a `--resolves` start resolves keeps its own rule (RULINGS T1)
    let p = r
        .call(
            T0 + 1.0,
            &other,
            "begin",
            json!({"trigger": "owner_order", "files": ["hooks/h.sh"], "resolves": r0}),
        )
        .unwrap();
    assert_eq!(p["begin"], "new");
    // a lock-less 승인 대기 repair counts too
    let mut rec = r.record_of(&r1);
    rec.state = RepairState::AwaitingApproval;
    rec.lock = None;
    ledger::write_record_in(&r.root, &rec).unwrap();
    let e = r
        .call(
            T0 + 2.0,
            &other,
            "begin",
            json!({"trigger": "owner_order", "files": ["bin/a.py", "bin/b.py"]}),
        )
        .unwrap_err();
    assert!(e.message.contains(&r1), "{}", e.message);
    assert_eq!(
        ledger::list_ids_in(&r.root).len(),
        3,
        "a second open repair was made"
    );
    // the same files pick it up
    let p = r
        .call(
            T0 + 2.0,
            &other,
            "begin",
            json!({"trigger": "owner_order", "files": ["bin/a.py"]}),
        )
        .unwrap();
    assert_eq!(
        (p["begin"].as_str(), p["repair"].as_str()),
        (Some("picked-up"), Some(r1.as_str()))
    );
    // a repair in force does not block a new one with another target set
    let r2 = rig("bj5-inforce");
    let done = committed_general(&r2);
    let p = r2
        .call(
            T0 + 10.0,
            &w,
            "begin",
            json!({"trigger": "owner_order", "files": ["bin/a.py", "hooks/h.sh"]}),
        )
        .unwrap();
    assert_eq!(p["begin"], "new");
    assert_ne!(p["repair"], json!(done));
}

// ───────────────────────────── T3d (RULINGS DA1–DA5; codex J4) ─────────────────────────────

/// RULINGS DA1: a target that is the SAME FILE as a rule document or a system-locked file (a real
/// hard link) is refused at `begin` naming the protected file; where the rule documents cannot be
/// enumerated (a `*_DIRECTIVE.md` outside the listed places) a target with more than one name is
/// refused conservatively; a general file with one name still begins.
#[test]
fn da1_hard_link_aliases_refused_at_begin() {
    let r = rig("da1-begin");
    let w = seat(3, "worker");
    std::fs::write(r.pack.join("soul.md"), b"# soul\n").unwrap();
    std::fs::write(r.pack.join("trusted-keys.json"), b"{}\n").unwrap();
    std::fs::create_dir_all(r.pack.join("notes")).unwrap();
    std::fs::create_dir_all(r.pack.join("extra")).unwrap();
    std::fs::write(r.pack.join("extra/NEW_DIRECTIVE.md"), b"# d\n").unwrap();
    std::fs::hard_link(r.pack.join("soul.md"), r.pack.join("notes/repair.txt")).unwrap();
    std::fs::hard_link(
        r.pack.join("trusted-keys.json"),
        r.pack.join("notes/k.json"),
    )
    .unwrap();
    std::fs::hard_link(
        r.pack.join("extra/NEW_DIRECTIVE.md"),
        r.pack.join("notes/d.txt"),
    )
    .unwrap();
    for (rel, names) in [
        ("notes/repair.txt", "soul.md"),
        ("notes/k.json", "trusted-keys.json"),
        ("notes/d.txt", "하드 링크"),
    ] {
        let e = r
            .call(
                T0,
                &w,
                "begin",
                json!({"trigger": "owner_order", "files": [rel]}),
            )
            .unwrap_err();
        assert_eq!(e.code, "repair_precondition", "{rel}: {}", e.message);
        assert!(e.message.contains(names), "{rel}: {}", e.message);
    }
    assert!(!r.root.exists(), "a refused begin wrote something");
    std::fs::write(r.pack.join("notes/plain.txt"), b"plain\n").unwrap();
    let ok = r
        .call(
            T0,
            &w,
            "begin",
            json!({"trigger": "owner_order", "files": ["notes/plain.txt"]}),
        )
        .unwrap();
    assert_eq!(ok["begin"], "new");
}

/// RULINGS DA1: a target that BECOMES a second name of a rule document after `begin` is refused at
/// `record`; nothing is recorded.
#[test]
fn da1_alias_made_after_begin_refused_at_record() {
    let r = rig("da1-record");
    let w = seat(3, "worker");
    std::fs::write(r.pack.join("soul.md"), b"# soul\n").unwrap();
    std::fs::create_dir_all(r.pack.join("notes")).unwrap();
    std::fs::write(r.pack.join("notes/x.txt"), b"x\n").unwrap();
    let id = r
        .call(
            T0,
            &w,
            "begin",
            json!({"trigger": "owner_order", "files": ["notes/x.txt"]}),
        )
        .unwrap()["repair"]
        .as_str()
        .unwrap()
        .to_string();
    std::fs::remove_file(r.pack.join("notes/x.txt")).unwrap();
    std::fs::hard_link(r.pack.join("soul.md"), r.pack.join("notes/x.txt")).unwrap();
    let e = r
        .call(
            T0 + 1.0,
            &w,
            "record",
            json!({"repair": id, "note": "edited"}),
        )
        .unwrap_err();
    assert_eq!(e.code, "repair_precondition", "{}", e.message);
    assert!(e.message.contains("soul.md"), "{}", e.message);
    assert!(
        r.record_of(&id).worker_record.is_none(),
        "the alias was recorded"
    );
}

/// RULINGS DA1 (the link form of "the same file"): an existing leaf link into the pack — here to a
/// directive in a place the pack does not list — is refused at `begin`.
#[cfg(unix)]
#[test]
fn da1_leaf_link_into_the_pack_refused_at_begin() {
    let r = rig("da1-link");
    std::fs::create_dir_all(r.pack.join("extra")).unwrap();
    std::fs::create_dir_all(r.pack.join("notes")).unwrap();
    std::fs::write(r.pack.join("extra/BAR_DIRECTIVE.md"), b"# d\n").unwrap();
    std::os::unix::fs::symlink("../extra/BAR_DIRECTIVE.md", r.pack.join("notes/s.txt")).unwrap();
    let e = r
        .call(
            T0,
            &seat(3, "worker"),
            "begin",
            json!({"trigger": "owner_order", "files": ["notes/s.txt"]}),
        )
        .unwrap_err();
    assert_eq!(e.code, "repair_precondition", "{}", e.message);
    assert!(e.message.contains("팩 안의 경로"), "{}", e.message);
    assert!(!r.root.exists());
}

/// RULINGS DA2: at 199 of the hour's 200 new signatures, eight concurrent reports of eight NEW
/// signatures (behind a barrier) yield exactly ONE new row; the others are over quota.
#[test]
fn da2_intake_quota_reserved_atomically() {
    let r = rig("da2");
    let w = seat(3, "worker");
    for i in 0..199 {
        let v = r
            .call(
                T0,
                &w,
                "report",
                json!({"file": "bin/a.py", "error": format!("E: case {i}")}),
            )
            .unwrap();
        assert_eq!(v["accepted"], true, "{i}: {v}");
    }
    let barrier = std::sync::Barrier::new(8);
    let outs: Vec<Value> = std::thread::scope(|s| {
        let hs: Vec<_> = (0..8)
            .map(|k| {
                let (b, r) = (&barrier, &r);
                s.spawn(move || {
                    b.wait();
                    r.call(
                        T0,
                        &seat(3, "worker"),
                        "report",
                        json!({"file": "bin/a.py", "error": format!("E: racer {k}")}),
                    )
                    .unwrap()
                })
            })
            .collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let accepted = outs.iter().filter(|v| v["accepted"] == true).count();
    let over = outs.iter().filter(|v| v["over_quota"] == true).count();
    assert_eq!((accepted, over), (1, 7), "{outs:?}");
    let rows = std::fs::read_dir(r.root.join("intake").join(&r.lane.0))
        .unwrap()
        .count();
    assert_eq!(rows, 200, "the hour's quota was exceeded");
}

/// RULINGS DA3: once the quota is full, a loop of reports with UNIQUE signatures adds no entry to
/// the daemon's lock registry (a refusal costs nothing); intake holds one lock per lane.
#[test]
fn da3_refusal_loop_adds_no_lock_entry() {
    let r = rig("da3");
    let w = seat(3, "worker");
    for i in 0..200 {
        r.call(
            T0,
            &w,
            "report",
            json!({"file": "bin/a.py", "error": format!("E: fill {i}")}),
        )
        .unwrap();
    }
    let before = r.mutexes.lock().unwrap().len();
    assert!(before <= 1, "intake made {before} locks for one lane");
    for i in 0..100 {
        let v = r
            .call(
                T0,
                &w,
                "report",
                json!({"file": "bin/a.py", "error": format!("E: refused {i}")}),
            )
            .unwrap();
        assert_eq!(v["over_quota"], true, "{i}: {v}");
    }
    assert_eq!(
        r.mutexes.lock().unwrap().len(),
        before,
        "the lock registry grew under refusals"
    );
}

/// RULINGS DA3: the intake lock set is FIXED and is the entry point other intake writers call
/// (`intake_lock_for` — the repeated-failure detector, unit T3e).
#[test]
fn da3_intake_lock_set_is_fixed() {
    assert_eq!(INTAKE_LOCK_SHARDS, 16);
    drop(intake_lock_for("sig-a"));
    drop(intake_lock_for("sig-a")); // released on drop: taking it twice in a row never blocks
}

/// RULINGS DA4: one open repair per FILE — `notes/ITEM.txt` beside an open `notes/item.txt` (the
/// same file on this case-insensitive volume), the same for a file that does not exist yet, one
/// `begin` naming one file twice, and two hard-linked names of a general file.
#[test]
fn da4_one_open_repair_per_file_not_per_spelling() {
    let r = rig("da4");
    std::fs::create_dir_all(r.pack.join("notes")).unwrap();
    std::fs::write(r.pack.join("notes/item.txt"), b"item\n").unwrap();
    let a = r
        .call(
            T0,
            &seat(3, "worker"),
            "begin",
            json!({"trigger": "owner_order", "files": ["notes/item.txt"]}),
        )
        .unwrap()["repair"]
        .as_str()
        .unwrap()
        .to_string();
    let b = seat(13, "worker-3");
    let e = r
        .call(
            T0 + 1.0,
            &b,
            "begin",
            json!({"trigger": "owner_order", "files": ["notes/ITEM.txt"]}),
        )
        .unwrap_err();
    assert_eq!(e.code, "repair_precondition", "{}", e.message);
    assert!(e.message.contains(&a), "{}", e.message);
    // a file that does not exist yet: the folded name decides
    let a2 = r
        .call(
            T0,
            &seat(3, "worker"),
            "begin",
            json!({"trigger": "owner_order", "files": ["notes/new.txt"]}),
        )
        .unwrap()["repair"]
        .as_str()
        .unwrap()
        .to_string();
    let e = r
        .call(
            T0 + 1.0,
            &b,
            "begin",
            json!({"trigger": "owner_order", "files": ["notes/NEW.txt"]}),
        )
        .unwrap_err();
    assert!(e.message.contains(&a2), "{}", e.message);
    // one begin naming one file twice
    std::fs::write(r.pack.join("notes/other.txt"), b"o\n").unwrap();
    let e = r
        .call(
            T0 + 1.0,
            &b,
            "begin",
            json!({"trigger": "owner_order", "files": ["notes/other.txt", "notes/OTHER.txt"]}),
        )
        .unwrap_err();
    assert_eq!(e.code, "invalid_params", "{}", e.message);
    // two hard-linked names of a general file: neither name is a second repair
    std::fs::write(r.pack.join("notes/h1.txt"), b"h\n").unwrap();
    std::fs::hard_link(r.pack.join("notes/h1.txt"), r.pack.join("notes/h2.txt")).unwrap();
    for rel in ["notes/h1.txt", "notes/h2.txt"] {
        let e = r
            .call(
                T0 + 1.0,
                &b,
                "begin",
                json!({"trigger": "owner_order", "files": [rel]}),
            )
            .unwrap_err();
        assert_eq!(e.code, "repair_precondition", "{rel}: {}", e.message);
    }
    assert_eq!(
        ledger::list_ids_in(&r.root).len(),
        2,
        "a second open repair was made"
    );
}

/// RULINGS DA4: `same_target` follows the file's identity when both live files exist (a hard link
/// is the same file, another file is not) and the folded name when one does not exist yet.
#[test]
fn da4_same_target_follows_identity_then_name() {
    let r = rig("da4-same");
    std::fs::create_dir_all(r.pack.join("notes")).unwrap();
    std::fs::write(r.pack.join("notes/one.txt"), b"1\n").unwrap();
    std::fs::hard_link(r.pack.join("notes/one.txt"), r.pack.join("notes/two.txt")).unwrap();
    std::fs::write(r.pack.join("notes/three.txt"), b"3\n").unwrap();
    assert!(
        same_target(&r.pack, "notes/one.txt", "notes/two.txt"),
        "a hard link is the same file"
    );
    assert!(!same_target(&r.pack, "notes/one.txt", "notes/three.txt"));
    assert!(same_target(&r.pack, "notes/absent.txt", "NOTES/Absent.TXT"));
    assert!(!same_target(
        &r.pack,
        "notes/absent.txt",
        "notes/absent2.txt"
    ));
}

/// A deletion repair committed, reopened by the audit, picked up, recorded and verified again —
/// ready for its re-commit (the target is Absent: no check child runs).
fn reopened_deletion(tag: &str) -> (Rig, String) {
    let r = rig(tag);
    let w = seat(3, "worker");
    let id = r
        .call(
            T0,
            &w,
            "begin",
            json!({"trigger": "owner_order", "files": ["bin/a.py"]}),
        )
        .unwrap()["repair"]
        .as_str()
        .unwrap()
        .to_string();
    std::fs::remove_file(r.pack.join("bin/a.py")).unwrap();
    r.call(
        T0 + 1.0,
        &w,
        "record",
        json!({"repair": id, "note": "delete"}),
    )
    .unwrap();
    r.call(
        T0 + 2.0,
        &seat(7, "reviewer-codex"),
        "attest_verify",
        json!({"repair": id, "verdict": "pass", "note": "ok"}),
    )
    .unwrap();
    r.call(T0 + 3.0, &w, "commit", json!({"repair": id}))
        .unwrap();
    r.call(
        T0 + 4.0,
        &seat(8, "reviewer-gemini"),
        "attest_audit",
        json!({"repair": id, "verdict": "finding", "note": "again"}),
    )
    .unwrap();
    assert_eq!(
        r.call(
            T0 + 5.0,
            &w,
            "begin",
            json!({"trigger": "owner_order", "files": ["bin/a.py"]})
        )
        .unwrap()["begin"],
        "picked-up"
    );
    r.call(
        T0 + 6.0,
        &w,
        "record",
        json!({"repair": id, "note": "delete again"}),
    )
    .unwrap();
    r.call(
        T0 + 7.0,
        &seat(7, "reviewer-codex"),
        "attest_verify",
        json!({"repair": id, "verdict": "pass", "note": "ok"}),
    )
    .unwrap();
    (r, id)
}

/// RULINGS DA5: a re-commit meets this lane's row corrupt, or of a newer schema: the commit is
/// refused saying why, the row's bytes stay as they were, and the record does not change state.
#[test]
fn da5_unreadable_lane_row_never_overwritten() {
    for (tag, newer) in [("da5-corrupt", false), ("da5-future", true)] {
        let (r, id) = reopened_deletion(tag);
        let rid = RepairId::parse(&id).unwrap();
        let path = ledger::lane_path_in(&r.root, &rid, &r.lane);
        let bytes: Vec<u8> = if newer {
            let mut v: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            v["schema"] = json!(999);
            serde_json::to_vec(&v).unwrap()
        } else {
            b"{ not json".to_vec()
        };
        std::fs::write(&path, &bytes).unwrap();
        let before = r.record_of(&id);
        let e = r
            .call(
                T0 + 8.0,
                &seat(3, "worker"),
                "commit",
                json!({"repair": id}),
            )
            .unwrap_err();
        assert_eq!(e.code, "repair_corrupt", "{tag}: {}", e.message);
        assert!(e.message.contains("lanes"), "{tag}: {}", e.message);
        assert_eq!(
            std::fs::read(&path).unwrap(),
            bytes,
            "{tag}: the lane row was overwritten"
        );
        let after = r.record_of(&id);
        assert_eq!(
            after.state,
            RepairState::InProgress,
            "{tag}: the record moved"
        );
        assert_eq!(after.committed, before.committed, "{tag}");
    }
}

/// RULINGS DJ1, rewritten for RULINGS ED1 (one comparison, made now): the NFC / NFD pair of one
/// name → the second refused; `a.txt` / `A.txt` → the second refused saying that names
/// differing only in case are one file on some volumes; two different ASCII names → both
/// allowed. REWRITTEN EXPECTATION (ED1): the old last case read "a non-ASCII creation beside an
/// ASCII creation in the same folder → both allowed" (`assert!(begin(13, "other/plain.md").is_ok())`);
/// under ED1 a not-yet-existing tail holding a non-ASCII character equals every other tail under
/// its deepest existing folder, so the ASCII creation is refused with the conditional sentence
/// until the first repair is finished. Each case has its own folder.
#[test]
fn dj1_names_that_do_not_exist_yet() {
    let r = rig("dj1");
    for d in ["n1", "n2", "n3", "n4"] {
        std::fs::create_dir_all(r.pack.join(d)).unwrap();
    }
    let begin = |sid: u64, rel: &str| {
        r.call(
            T0,
            &seat(sid, if sid == 3 { "worker" } else { "worker-3" }),
            "begin",
            json!({"trigger": "owner_order", "files": [rel]}),
        )
    };
    let id_of = |v: Value| v["repair"].as_str().unwrap().to_string();
    let nfc = id_of(begin(3, "n1/caf\u{e9}.txt").unwrap());
    let e = begin(13, "n1/cafe\u{301}.txt").unwrap_err();
    assert_eq!(e.code, "repair_precondition", "{}", e.message);
    assert!(
        e.message.contains(&nfc) && e.message.contains("ASCII"),
        "{}",
        e.message
    );
    let a = id_of(begin(3, "n2/a.txt").unwrap());
    let e = begin(13, "n2/A.txt").unwrap_err();
    assert!(
        e.message.contains(&a)
            && e.message
                .contains("대소문자만 다른 이름은 볼륨에 따라 같은 파일"),
        "{}",
        e.message
    );
    assert!(begin(3, "n3/x1.txt").is_ok());
    assert!(begin(13, "n3/x2.txt").is_ok());
    let k = id_of(begin(3, "n4/\u{c0c8}.md").unwrap());
    let e = begin(13, "n4/plain.md").unwrap_err();
    assert!(
        e.message.contains(&k) && e.message.contains("볼륨에 따라 같은 파일일 수 있습니다"),
        "{}",
        e.message
    );
}

/// RULINGS DJ2, rewritten for RULINGS ED2 — the race of DA2, made deterministic, with the stop
/// INSIDE the step the fix made atomic: at 199 of 200, report X is held after it observed room and
/// before it takes the place; report Y (another new signature, on another intake lock) then comes
/// to the quota. The probe tells whether Y finds the quota mutex held: with the fix it is (X holds
/// it across observe and take), so X is let go first and Y finds the hour full; with the check and
/// the take in two critical sections it is not, Y runs to its end in the gap and X takes a place it
/// only observed → 201 rows. REWRITTEN EXPECTATION (ED2): the old stop was BEFORE the quota was
/// observed (`intake_seam_install(&sig(ex))`, X resumed after Y finished), which that mutation
/// also passed; asserted now: exactly ONE of X and Y accepted, 200 rows.
#[test]
fn dj2_observe_reserve_boundary_is_one_step() {
    use super::ledger_verbs::{intake_seam_clear, intake_seam_install};
    let r = rig("dj2-race");
    let w = seat(3, "worker");
    for i in 0..199 {
        let v = r
            .call(
                T0,
                &w,
                "report",
                json!({"file": "bin/a.py", "error": format!("E: slot {i}")}),
            )
            .unwrap();
        assert_eq!(v["accepted"], true, "{i}: {v}");
    }
    let sig = |e: &str| {
        cys::repair::intake::parse_report("bin/a.py", e)
            .unwrap()
            .signature
    };
    let ex = "E: held at the boundary";
    let ey = (0..)
        .map(|k| format!("E: passes the held one {k}"))
        .find(|e| intake_shard_of(&sig(e)) != intake_shard_of(&sig(ex)))
        .unwrap();
    let seam = intake_seam_install(&sig(ex), &sig(&ey));
    let call = |e: &str| {
        r.call(
            T0,
            &seat(3, "worker"),
            "report",
            json!({"file": "bin/a.py", "error": e}),
        )
        .unwrap()
    };
    let (x, y, held) = std::thread::scope(|s| {
        let hx = s.spawn(|| call(ex));
        assert!(
            seam.wait_arrived(std::time::Duration::from_secs(20)),
            "X never reached the boundary"
        );
        let hy = s.spawn(|| call(&ey));
        let held = seam
            .wait_probe(std::time::Duration::from_secs(20))
            .expect("Y never came to the quota");
        let (x, y) = if held {
            seam.open(); // the fix: Y waits for X's one step
            let x = hx.join().unwrap();
            (x, hy.join().unwrap())
        } else {
            let y = hy.join().unwrap(); // two critical sections: Y runs in the gap
            seam.open();
            (hx.join().unwrap(), y)
        };
        (x, y, held)
    });
    intake_seam_clear();
    let accepted = [&x, &y].iter().filter(|v| v["accepted"] == true).count();
    assert_eq!(accepted, 1, "held={held}: X={x} Y={y}");
    let rows = std::fs::read_dir(r.root.join("intake").join(&r.lane.0))
        .unwrap()
        .count();
    assert_eq!(rows, 200, "the hour's quota was exceeded");
}

/// RULINGS DJ2: a reservation is RETURNED after a failed write — at 199 of 200, a new signature
/// whose row cannot be written (the lane's intake folder made read-only) fails; the next new
/// signature then gets the 200th place, and the one after it is over quota.
#[cfg(unix)]
#[test]
fn dj2_failed_write_returns_its_reservation() {
    use std::os::unix::fs::PermissionsExt;
    let r = rig("dj2-fail");
    let w = seat(3, "worker");
    for i in 0..199 {
        r.call(
            T0,
            &w,
            "report",
            json!({"file": "bin/a.py", "error": format!("E: slot {i}")}),
        )
        .unwrap();
    }
    let dir = r.root.join("intake").join(&r.lane.0);
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
    let failed = r.call(
        T0,
        &w,
        "report",
        json!({"file": "bin/a.py", "error": "E: cannot be written"}),
    );
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(failed.is_err(), "the write did not fail: {failed:?}");
    let v = r
        .call(
            T0,
            &w,
            "report",
            json!({"file": "bin/a.py", "error": "E: takes the returned place"}),
        )
        .unwrap();
    assert_eq!(
        v["accepted"], true,
        "the failed write kept its reservation: {v}"
    );
    let v = r
        .call(
            T0,
            &w,
            "report",
            json!({"file": "bin/a.py", "error": "E: one too many"}),
        )
        .unwrap();
    assert_eq!(v["over_quota"], true, "{v}");
}

/// RULINGS DJ3: a second name of the key file is refused naming the canonical file and its only
/// way (`cys pack-heal`), never the owner-token route of the rule documents.
#[test]
fn dj3_key_file_alias_names_the_canonical_file_and_pack_heal() {
    let r = rig("dj3");
    std::fs::write(r.pack.join("trusted-keys.json"), b"{}\n").unwrap();
    std::fs::create_dir_all(r.pack.join("notes")).unwrap();
    std::fs::hard_link(
        r.pack.join("trusted-keys.json"),
        r.pack.join("notes/k.json"),
    )
    .unwrap();
    let e = r
        .call(
            T0,
            &seat(3, "worker"),
            "begin",
            json!({"trigger": "owner_order", "files": ["notes/k.json"]}),
        )
        .unwrap_err();
    assert_eq!(e.code, "repair_precondition", "{}", e.message);
    assert!(
        e.message.contains("`trusted-keys.json`")
            && e.message.contains("cys pack-heal trusted-keys.json"),
        "{}",
        e.message
    );
    assert!(
        !e.message.contains("주인 지시 표"),
        "the owner-token route was offered: {}",
        e.message
    );
}

/// RULINGS ED1, codex's sequence (a): on a volume that may treat composed and decomposed names as
/// one, the PARENT does not exist yet either — `p/caf\u{e9}/x.txt` then `p/cafe\u{301}/x.txt`:
/// one existing ancestor (`p`), tails with a non-ASCII character → the second is refused.
#[test]
fn ed1_parent_absent_composed_and_decomposed() {
    let r = rig("ed1-a");
    std::fs::create_dir_all(r.pack.join("p")).unwrap();
    let first = r
        .call(
            T0,
            &seat(3, "worker"),
            "begin",
            json!({"trigger": "owner_order", "files": ["p/caf\u{e9}/x.txt"]}),
        )
        .unwrap()["repair"]
        .as_str()
        .unwrap()
        .to_string();
    let e = r
        .call(
            T0,
            &seat(13, "worker-3"),
            "begin",
            json!({"trigger": "owner_order", "files": ["p/cafe\u{301}/x.txt"]}),
        )
        .unwrap_err();
    assert_eq!(e.code, "repair_precondition", "{}", e.message);
    assert!(e.message.contains(&first), "{}", e.message);
}

/// RULINGS ED1, codex's sequence (b): a repair opened on an EXISTING `q/caf\u{e9}.txt` deletes it;
/// `begin` under the decomposed name is compared with the recorded target AS IT IS NOW (both
/// absent, one ancestor, non-ASCII tails) → refused.
#[test]
fn ed1_existing_file_deleted_then_other_composition() {
    let r = rig("ed1-b");
    std::fs::create_dir_all(r.pack.join("q")).unwrap();
    std::fs::write(r.pack.join("q/caf\u{e9}.txt"), b"c\n").unwrap();
    let first = r
        .call(
            T0,
            &seat(3, "worker"),
            "begin",
            json!({"trigger": "owner_order", "files": ["q/caf\u{e9}.txt"]}),
        )
        .unwrap()["repair"]
        .as_str()
        .unwrap()
        .to_string();
    std::fs::remove_file(r.pack.join("q/caf\u{e9}.txt")).unwrap();
    let e = r
        .call(
            T0,
            &seat(13, "worker-3"),
            "begin",
            json!({"trigger": "owner_order", "files": ["q/cafe\u{301}.txt"]}),
        )
        .unwrap_err();
    assert_eq!(e.code, "repair_precondition", "{}", e.message);
    assert!(e.message.contains(&first), "{}", e.message);
}

/// RULINGS ED1: the conservative refusal lasts only while the first repair is open — once it is
/// committed (in force, no longer being worked) the case-twin name begins.
#[test]
fn ed1_finished_repair_no_longer_refuses() {
    let r = rig("ed1-done");
    std::fs::create_dir_all(r.pack.join("f")).unwrap();
    let w = seat(3, "worker");
    let other = seat(13, "worker-3");
    let id = r
        .call(
            T0,
            &w,
            "begin",
            json!({"trigger": "owner_order", "files": ["f/a.txt"]}),
        )
        .unwrap()["repair"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(r
        .call(
            T0,
            &other,
            "begin",
            json!({"trigger": "owner_order", "files": ["f/A.txt"]})
        )
        .is_err());
    std::fs::write(r.pack.join("f/a.txt"), b"a\n").unwrap();
    r.call(T0 + 1.0, &w, "record", json!({"repair": id, "note": "n"}))
        .unwrap();
    r.call(
        T0 + 2.0,
        &seat(7, "reviewer-codex"),
        "attest_verify",
        json!({"repair": id, "verdict": "pass", "note": "ok"}),
    )
    .unwrap();
    r.call(T0 + 3.0, &w, "commit", json!({"repair": id}))
        .unwrap();
    let p = r
        .call(
            T0 + 4.0,
            &other,
            "begin",
            json!({"trigger": "owner_order", "files": ["f/A.txt"]}),
        )
        .unwrap();
    assert_eq!(p["begin"], "new", "{p}");
}

/// RULINGS ED3: two names in ONE `begin` that only the conservative rule equates (case twins
/// that do not exist yet) get the conditional sentence — "같은 파일입니다" is said only for one
/// identity.
#[test]
fn ed3_one_begin_conservative_pair_gets_the_conditional_sentence() {
    let r = rig("ed3");
    std::fs::create_dir_all(r.pack.join("g")).unwrap();
    let e = r
        .call(
            T0,
            &seat(3, "worker"),
            "begin",
            json!({"trigger": "owner_order", "files": ["g/a.txt", "g/A.txt"]}),
        )
        .unwrap_err();
    assert_eq!(e.code, "invalid_params", "{}", e.message);
    assert!(
        e.message.contains("볼륨에 따라 같은 파일일 수 있습니다"),
        "{}",
        e.message
    );
    assert!(!e.message.contains("같은 파일입니다"), "{}", e.message);
}

/// RULINGS EG1: a lookup that FAILS is never evidence of "different" — with an injected failing
/// lookup (test builds only, no platform trick) for the NEW target's ancestor, for the OPEN
/// repair's ancestor and for an EXISTING file's identity, the second `begin` is refused saying it
/// could not be determined; positive "different" cases still open.
#[test]
fn eg1_a_failed_lookup_refuses_like_same() {
    let r = rig("eg1");
    for d in ["u1", "u2", "v1", "v2", "w", "z"] {
        std::fs::create_dir_all(r.pack.join(d)).unwrap();
    }
    std::fs::write(r.pack.join("w/x.txt"), b"x\n").unwrap();
    let begin = |sid: u64, rel: &str| {
        r.call(
            T0,
            &seat(sid, if sid == 3 { "worker" } else { "worker-3" }),
            "begin",
            json!({"trigger": "owner_order", "files": [rel]}),
        )
    };
    let id_of = |v: Value| v["repair"].as_str().unwrap().to_string();
    let unknown = |e: VerbError, first: &str| {
        assert_eq!(e.code, "repair_precondition", "{}", e.message);
        assert!(
            e.message.contains(first) && e.message.contains("확인할 수 없습니다"),
            "{}",
            e.message
        );
    };
    // the new target's ancestor cannot be read
    let a = id_of(begin(3, "u1/a.txt").unwrap());
    lookup_fail_install(&r.pack.join("u2"));
    unknown(begin(13, "u2/b.txt").unwrap_err(), &a);
    lookup_fail_clear(&r.pack.join("u2"));
    // the open repair's ancestor cannot be read
    let c = id_of(begin(3, "v1/c.txt").unwrap());
    lookup_fail_install(&r.pack.join("v1"));
    let e = begin(13, "v2/d.txt").unwrap_err();
    lookup_fail_clear(&r.pack.join("v1"));
    unknown(e, &c);
    // an existing file's identity cannot be read
    let x = id_of(begin(3, "w/x.txt").unwrap());
    lookup_fail_install(&r.pack.join("w/x.txt"));
    let e = begin(13, "w/y.txt").unwrap_err();
    lookup_fail_clear(&r.pack.join("w/x.txt"));
    unknown(e, &x);
    // positive "different": with every lookup readable the same begins open
    assert!(begin(13, "u2/b.txt").is_ok());
    assert!(begin(13, "v2/d.txt").is_ok());
    assert!(
        begin(13, "w/y.txt").is_ok(),
        "an existing file and an absent name are different"
    );
    std::fs::write(r.pack.join("z/one.txt"), b"1\n").unwrap();
    std::fs::write(r.pack.join("z/two.txt"), b"2\n").unwrap();
    assert!(begin(3, "z/one.txt").is_ok());
    assert!(
        begin(13, "z/two.txt").is_ok(),
        "two existing files with two identities are different"
    );
}
