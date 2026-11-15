//! Track T7b — daemon tests: the commit's seal of the result bytes (RULINGS HI2 / HJ2) through T3's
//! `commit` verb, and `repair.recheck` (H8) through the verb dispatcher with T3's role table and T3's
//! `begin --joins` in front of it.

use super::*;
use cys::repair::boot_critical::CheckKind;
use cys::repair::ledger::{self, RepairRecord};
use cys::repair::roles::CallerKind;
use cys::repair::types::{Classification, LaneState, RepairId, RepairState, TargetState};
use serde_json::json;
use std::sync::atomic::{AtomicU64, Ordering};

struct Rig {
    root: PathBuf,
    pack: PathBuf,
    registry: PathBuf,
    mutexes: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    lane: LaneKey,
    prompts: Mutex<HashMap<u64, PromptOrigin>>,
    base: PathBuf,
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

static SEQ: AtomicU64 = AtomicU64::new(0);

fn rig(tag: &str) -> Rig {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let base = std::env::temp_dir().join(format!("cys-t7bd-{tag}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let pack = base.join("pack");
    std::fs::create_dir_all(pack.join("bin")).unwrap();
    std::fs::write(pack.join(".pack-version"), "0.15.0\n").unwrap();
    // the lane's pack instance (every lane's pack holds one after its first ready tick — I8); `recheck`
    // validates it as part of its stamp (RULINGS IV4)
    std::fs::write(pack.join(".repair-pack-id"), format!("{}\n", "ab".repeat(32))).unwrap();
    std::fs::write(pack.join("bin/a.py"), b"print('a')\n").unwrap();
    Rig {
        root: base.join("local").join("repairs"),
        pack,
        registry: base.join("depts.json"),
        mutexes: Mutex::new(HashMap::new()),
        lane: LaneKey("base".into()),
        prompts: Mutex::new(HashMap::new()),
        base,
    }
}

fn general(_: &str) -> Result<Classification, RepairError> {
    Ok(Classification::General)
}

fn pass(_: &str, _: &Path, _: std::time::Duration) -> Result<CheckReport, RepairError> {
    Ok(CheckReport { passed: true, timed_out: false, failure: String::new(), check_ms: 3, reference_kind: CheckKind::PyCompile, reference_ms: 2 })
}

impl Rig {
    fn ctx(&self, now: f64) -> Ctx<'_> {
        Ctx {
            root: self.root.clone(),
            lane: self.lane.clone(),
            incarnation: "inc-1".into(),
            pack_dir: self.pack.clone(),
            now,
            windows: false,
            registry: self.registry.clone(),
            mutexes: &self.mutexes,
            seats: Vec::new(),
            classify: &general,
            check: &pass,
            hook_step: &cys::repair::apply::hook_lines_step,
            claim: &cys::repair::grant::try_claim_autonomous_start,
            prompts: &self.prompts,
            // batch 7 (T4e · RULINGS HP2): the product's pause — not in force in this rig
            product_pause: &|| false,
        }
    }
    fn call(&self, now: f64, who: &Caller, sub: &str, p: Value) -> Result<Value, VerbError> {
        handle_with(&self.ctx(now), who, sub, &p)
    }
    fn record_of(&self, id: &str) -> RepairRecord {
        ledger::read_record_in(&self.root, &RepairId::parse(id).unwrap()).unwrap().unwrap()
    }
}

fn seat(sid: u64, role: &str) -> Caller {
    Caller { kind: CallerKind::Seat { sid }, role: Some(role.into()), app_token: false, bound_family: role_family(role) }
}

const T0: f64 = 1_800_000_000.0;

/// begin → edit → record → verify (another seat) — ready for `commit`; returns the id.
fn verified(r: &Rig, rel: &str, post: &[u8]) -> String {
    let w = seat(3, "worker");
    let b = r.call(T0, &w, "begin", json!({"trigger": "owner_order", "files": [rel]})).unwrap();
    let id = b["repair"].as_str().unwrap().to_string();
    std::fs::write(r.pack.join(rel), post).unwrap();
    r.call(T0 + 1.0, &w, "record", json!({"repair": id, "note": "fix"})).unwrap();
    r.call(T0 + 2.0, &seat(7, "reviewer-codex"), "attest_verify", json!({"repair": id, "verdict": "pass", "note": "ok"})).unwrap();
    id
}

fn clear_hooks() {
    super::seal_hooks::BEFORE.with(|h| *h.borrow_mut() = None);
    super::seal_hooks::AFTER.with(|h| *h.borrow_mut() = None);
}

/// HI2: the commit seals the committed bytes (kind `committed-copy`) BEFORE the record is published;
/// a receiving lane finds them by content.
#[test]
fn commit_seals_the_committed_copy() {
    let r = rig("seal");
    let id = verified(&r, "bin/a.py", b"print('fixed')\n");
    let c = r.call(T0 + 3.0, &seat(3, "worker"), "commit", json!({"repair": id})).unwrap();
    assert_eq!(c["state"], "probation", "{c}");
    let rec = r.record_of(&id);
    let holder = cys::repair::propagate::committed_copy_in(&r.root, &rec, "bin/a.py").unwrap();
    assert_eq!(std::fs::read(holder).unwrap(), b"print('fixed')\n");
}

/// HJ2: a result over the limit → the commit is REFUSED before anything is published, the limit in
/// the sentence, the record still in progress.
#[test]
fn a_commit_over_the_seal_limit_is_refused_and_nothing_is_published() {
    let r = rig("big");
    let w = seat(3, "worker");
    let b = r.call(T0, &w, "begin", json!({"trigger": "owner_order", "files": ["bin/a.py"]})).unwrap();
    let id = b["repair"].as_str().unwrap().to_string();
    let f = std::fs::OpenOptions::new().write(true).open(r.pack.join("bin/a.py")).unwrap();
    f.set_len(cys::repair::propagate::SEAL_LIMIT + 1).unwrap();
    drop(f);
    r.call(T0 + 1.0, &w, "record", json!({"repair": id, "note": "big"})).unwrap();
    r.call(T0 + 2.0, &seat(7, "reviewer-codex"), "attest_verify", json!({"repair": id, "verdict": "pass", "note": "ok"})).unwrap();
    let e = r.call(T0 + 3.0, &w, "commit", json!({"repair": id})).unwrap_err();
    assert_eq!(e.code, cys::repair::types::ERR_PRECONDITION);
    assert!(e.message.contains(&cys::repair::propagate::SEAL_LIMIT.to_string()), "{}", e.message);
    let after = r.record_of(&id);
    assert_eq!(after.state, RepairState::InProgress);
    assert!(after.committed.is_empty(), "nothing published");
    assert!(after.verify.is_some(), "the signatures stay: the commit can be tried again");
    assert!(ledger::read_lane_in(&r.root, &RepairId::parse(&id).unwrap(), &r.lane).unwrap().is_none(), "no row");
}

/// HJ2: the live file changed between the copy and the final validation → the commit is refused by
/// the re-validation under the mutex; nothing committed.
#[test]
fn a_change_between_the_copy_and_the_final_validation_refuses() {
    let r = rig("chg");
    let id = verified(&r, "bin/a.py", b"print('fixed')\n");
    let live = r.pack.join("bin/a.py");
    super::seal_hooks::AFTER.with(|h| {
        let live = live.clone();
        *h.borrow_mut() = Some(Box::new(move || std::fs::write(&live, b"print('other')\n").unwrap()))
    });
    let e = r.call(T0 + 3.0, &seat(3, "worker"), "commit", json!({"repair": id}));
    clear_hooks();
    let e = e.unwrap_err();
    assert_eq!(e.code, "target_changed", "{}", e.message);
    assert_eq!(r.record_of(&id).state, RepairState::InProgress);
}

/// HJ2: a sealed copy that is not the verified state (the file changed during the copy and came
/// back) → refused, the verified states never replaced by what the copy saw; tried again → committed.
#[test]
fn a_copy_that_differs_from_the_verified_state_refuses_and_a_retry_commits() {
    let r = rig("diff");
    let id = verified(&r, "bin/a.py", b"print('fixed')\n");
    let live = r.pack.join("bin/a.py");
    {
        let l1 = live.clone();
        super::seal_hooks::BEFORE.with(|h| *h.borrow_mut() = Some(Box::new(move || std::fs::write(&l1, b"print('during')\n").unwrap())));
        let l2 = live.clone();
        super::seal_hooks::AFTER.with(|h| *h.borrow_mut() = Some(Box::new(move || std::fs::write(&l2, b"print('fixed')\n").unwrap())));
    }
    let e = r.call(T0 + 3.0, &seat(3, "worker"), "commit", json!({"repair": id}));
    clear_hooks();
    let e = e.unwrap_err();
    assert_eq!(e.code, "target_changed", "{}", e.message);
    assert!(e.message.contains("보관한 사본"), "{}", e.message);
    assert_eq!(r.record_of(&id).state, RepairState::InProgress);
    // tried again (nothing else changed) → committed, the copy is the verified bytes
    let c = r.call(T0 + 4.0, &seat(3, "worker"), "commit", json!({"repair": id})).unwrap();
    assert_eq!(c["state"], "probation", "{c}");
    let rec = r.record_of(&id);
    let holder = cys::repair::propagate::committed_copy_in(&r.root, &rec, "bin/a.py").unwrap();
    assert_eq!(std::fs::read(holder).unwrap(), b"print('fixed')\n");
    assert_eq!(rec.committed, vec![("bin/a.py".to_string(), TargetState::read(&live).unwrap().canon())]);
}

/// H8 through the dispatcher: a general repair R made in another lane has a 충돌 row here; the
/// worker's `begin --joins` (T3) takes the merge lock, the merge changes the file, `recheck` →
/// 검증됨 with the merge record; a master seat is refused by the role table.
#[test]
fn recheck_after_a_merge_through_the_dispatcher() {
    let r = rig("rk");
    let id = RepairId::parse("R-1").unwrap();
    std::fs::create_dir_all(ledger::repair_dir_in(&r.root, &id)).unwrap();
    let rec = RepairRecord {
        schema: 1,
        id: id.clone(),
        created_at: 1.0,
        trigger: cys::repair::types::Trigger::OwnerOrder,
        symptom: String::new(),
        cause: String::new(),
        classification: Classification::General,
        files: vec![ledger::FileChange {
            rel: "bin/a.py".into(),
            kind: ledger::ChangeKind::Edit,
            pre: ledger::PreImage::Absent,
            post: TargetState::File { sha256: "0".repeat(64), mode: 0o644 },
            diff: None,
        }],
        worker_record: None,
        verify: None,
        audit: None,
        audit_due_at: None,
        approval: None,
        owner_token_sha256: None,
        attempt: 1,
        methods: Vec::new(),
        resubmits: Vec::new(),
        pending_request: None,
        lock: None,
        state: RepairState::Probation,
        intake_signatures: Vec::new(),
        owner_original: false,
        origin_lane: LaneKey("dept-x".into()),
        resolves: None,
        committed: vec![("bin/a.py".into(), TargetState::File { sha256: "0".repeat(64), mode: 0o644 }.canon())],
        fixer_seats: Vec::new(),
        checks: Vec::new(),
        reapplies: None,
    };
    ledger::write_record_in(&r.root, &rec).unwrap();
    let mut row = ledger::LaneRecord {
        schema: 1,
        repair: id.clone(),
        lane: r.lane.clone(),
        state: LaneState::Conflict,
        probation: None,
        merge: None,
        merge_history: Vec::new(),
        approval_ref: None,
        conflict_live: Vec::new(),
        applied: Vec::new(),
        applied_history: Vec::new(),
        merge_base: Vec::new(),
        merge_lock: None,
        merge_attempt: 0,
        reason: String::new(),
        updated_at: T0,
    };
    ledger::write_lane_in(&r.root, &row).unwrap();
    let e = r.call(T0, &seat(2, "master"), "recheck", json!({"repair": "R-1"})).unwrap_err();
    assert_eq!(e.code, "acl_denied", "{}", e.message);
    let w = seat(3, "worker");
    r.call(T0, &w, "begin", json!({"trigger": "owner_order", "files": ["bin/a.py"], "joins": "R-1"})).unwrap();
    std::fs::write(r.pack.join("bin/a.py"), b"print('merged')\n").unwrap();
    let v = r.call(T0 + 1.0, &w, "recheck", json!({"repair": "R-1"})).unwrap();
    assert_eq!(v["lane_state"], "verified", "{v}");
    // RULINGS JD2 (added): the answer names what the call changed
    assert_eq!(v["changed"], "이 호출이 바꾼 것: 이 부서의 행('충돌' → '검증됨', 합친 기록을 남기고 합치기 잠금을 풂)", "{v}");
    row = ledger::read_lane_in(&r.root, &id, &r.lane).unwrap().unwrap();
    assert_eq!(row.state, LaneState::Verified);
    let m = row.merge.unwrap();
    assert!(m.changed && m.by_seat == 3);
    assert!(row.merge_lock.is_none());
}

/// RULINGS HH4 — the job thread's propagation (`propagate_verbs::run_on_job_thread`, what `run_job`
/// calls after T6's tick) in the production environment on an empty ledger: the first tick records
/// the pack instance and the catch-up (the gate's reader answers OPEN); the next tick — nothing due
/// for T6, nothing changed — writes nothing (the job-state and id-file bytes stay identical).
#[test]
fn the_job_threads_propagation_records_the_catchup_and_an_unchanged_tick_writes_nothing() {
    use cys::repair::ledger::GateVerdict;
    use cys::repair::propagate::{gate_verdict_in, job_state_path_in, read_job_state_checked, read_pack_instance, FileRead};
    let r = rig("job");
    std::fs::create_dir_all(&r.root).unwrap();
    let daemon = crate::team_gate_tests::tmp_daemon("t7b-job", false);
    crate::repair_job::mark_ready(&daemon);
    let lane = LaneKey::of_socket(&daemon.socket_path);
    let first = propagate_verbs::run_on_job_thread(&daemon, &r.root, &r.pack).unwrap();
    assert!(first.catchup_completed, "{first:?}");
    let FileRead::Value(st) = read_job_state_checked(&r.root, &lane) else { panic!("no job state") };
    assert!(st.initial_catchup_complete);
    assert_eq!(read_pack_instance(&r.pack), FileRead::Value(st.pack_instance.clone().unwrap()));
    assert_eq!(gate_verdict_in(&r.root, &lane), GateVerdict::Open);
    let js = std::fs::read(job_state_path_in(&r.root, &lane)).unwrap();
    let id = std::fs::read(r.pack.join(".repair-pack-id")).unwrap();
    let second = propagate_verbs::run_on_job_thread(&daemon, &r.root, &r.pack).unwrap();
    assert!(!second.catchup_completed && second.rows.is_empty(), "{second:?}");
    assert_eq!(std::fs::read(job_state_path_in(&r.root, &lane)).unwrap(), js);
    assert_eq!(std::fs::read(r.pack.join(".repair-pack-id")).unwrap(), id);
}

/// RULINGS HH4 — pins on the job: `run_job` calls this lane's propagation AFTER T6's tick (both on
/// the one single-flight thread); the production runner and the job send nothing (F1: no bus, queue,
/// injection, alert or `cys send`).
#[test]
fn the_job_calls_the_propagation_after_t6s_tick_and_sends_nothing() {
    let job = include_str!("repair_job.rs");
    let job = &job[..job.find("#[cfg(test)]\n#[path").unwrap_or(job.len())];
    let body = &job[job.find("fn run_job(").expect("run_job")..];
    let t6 = body.find("probation::run_tick(&env").expect("T6's tick");
    let mine = body.find("propagate_verbs::run_on_job_thread(").expect("the propagation");
    assert!(t6 < mine, "the propagation runs after T6's tick");
    let src = include_str!("repair.rs");
    let verbs = &src[src.find("pub(crate) mod propagate_verbs {").unwrap()..src.find("\nmod apply_verbs {").unwrap()];
    assert!(verbs.contains("fn run_on_job_thread("));
    for (name, text) in [("repair_job.rs", job), ("repair.rs propagate_verbs", verbs)] {
        for needle in ["bus.publish", "enqueue", "send_text", "alert_route", "inject(", "push_queued", "cys send"] {
            assert!(!text.contains(needle), "{name} contains {needle:?}");
        }
    }
}

/// RULINGS HH6 + HH7 — T3's `status`: the catch-up fields read from the daemon's OWN root through
/// T7b's reader (no "unavailable" any more); HQ's status carries the CEO's ONE summary line (null when
/// no lane holds a 충돌 row of an in-force repair; the line names the lane and the repair when one
/// does); a department's status carries no summary field.
#[test]
fn status_reads_the_catchup_from_its_root_and_hq_carries_the_summary_line() {
    let r = rig("st");
    std::fs::create_dir_all(&r.root).unwrap();
    let who = seat(3, "worker");
    let v = r.call(T0, &who, "status", json!({})).unwrap();
    assert_eq!(v["initial_catchup_complete"], false, "{v}");
    assert_eq!(v["catchup_pending"], 0, "{v}");
    assert_eq!(v["catchup_conflict"], 0, "{v}");
    assert!(v.get("catchup_unavailable").is_none(), "{v}");
    assert!(v["conflict_summary"].is_null(), "HQ: the field is there, null with no conflict: {v}");
    // an in-force repair whose HQ row is 충돌
    let id = verified(&r, "bin/a.py", b"print('fixed')\n");
    r.call(T0 + 3.0, &who, "commit", json!({"repair": id})).unwrap();
    let rid = RepairId::parse(&id).unwrap();
    let mut row = ledger::read_lane_in(&r.root, &rid, &r.lane).unwrap().unwrap();
    row.state = LaneState::Conflict;
    ledger::write_lane_in(&r.root, &row).unwrap();
    let v = r.call(T0 + 4.0, &who, "status", json!({})).unwrap();
    assert_eq!(v["catchup_conflict"], 1, "{v}");
    let line = v["conflict_summary"].as_str().unwrap_or_else(|| panic!("no summary line: {v}"));
    assert!(line.contains(&id) && line.contains("base"), "{line}");
    // a department's status: no summary field
    let mut d = rig("st-d");
    d.lane = LaneKey("dept-x".into());
    std::fs::create_dir_all(&d.root).unwrap();
    let v = d.call(T0, &who, "status", json!({})).unwrap();
    assert!(v.get("conflict_summary").is_none(), "{v}");
}

/// RULINGS IK2 — `cys repair status R` carries each department row's reason (and the CLI prints it on a
/// line of its own — `repair_cli::lane_reason_lines`).
#[test]
fn status_of_a_repair_shows_each_rows_reason() {
    let r = rig("st-reason");
    let who = seat(3, "worker");
    let id = verified(&r, "bin/a.py", b"print('fixed')\n");
    r.call(T0 + 3.0, &who, "commit", json!({"repair": id})).unwrap();
    let rid = RepairId::parse(&id).unwrap();
    let mut row = ledger::read_lane_in(&r.root, &rid, &r.lane).unwrap().unwrap();
    row.state = LaneState::Conflict;
    row.reason = "사본 검사 실패 — 살아 있는 파일은 바꾸지 않았습니다 · bin/a.py: 검사 시간 초과".into();
    ledger::write_lane_in(&r.root, &row).unwrap();
    let v = r.call(T0 + 4.0, &who, "status", json!({"repair": id})).unwrap();
    let rows = v["lane_rows"].as_array().unwrap();
    assert!(rows.iter().any(|x| x["lane"] == "base" && x["reason"] == row.reason.as_str()), "{v}");
}

/// RULINGS IG6 — a HOOK-ONLY repair's conflict is resolvable through the merge ticket's lines: `begin
/// --unregister-hook EVENT:SCRIPT --joins R` (no `--files`) takes this lane's merge lock through T3's
/// `begin_joins`, and `recheck R` runs the lane's hook step and ends 검증됨.
#[test]
fn a_hook_only_repairs_conflict_is_merged_through_unregister_hook_joins() {
    let r = rig("hook-merge");
    let id = RepairId::parse("R-1").unwrap();
    std::fs::create_dir_all(ledger::repair_dir_in(&r.root, &id)).unwrap();
    let rec = RepairRecord {
        schema: 1,
        id: id.clone(),
        created_at: 1.0,
        trigger: cys::repair::types::Trigger::Update,
        symptom: String::new(),
        cause: String::new(),
        classification: Classification::General,
        files: vec![ledger::FileChange {
            rel: "settings:Stop:x.sh".into(),
            kind: ledger::ChangeKind::HookUnregister { event: "Stop".into(), script: "x.sh".into() },
            pre: ledger::PreImage::Absent,
            post: TargetState::Absent,
            diff: None,
        }],
        worker_record: None,
        verify: None,
        audit: None,
        audit_due_at: None,
        approval: None,
        owner_token_sha256: None,
        attempt: 1,
        methods: Vec::new(),
        resubmits: Vec::new(),
        pending_request: None,
        lock: None,
        state: RepairState::Probation,
        intake_signatures: Vec::new(),
        owner_original: false,
        origin_lane: LaneKey("dept-x".into()),
        resolves: None,
        committed: Vec::new(),
        fixer_seats: Vec::new(),
        checks: Vec::new(),
        reapplies: None,
    };
    ledger::write_record_in(&r.root, &rec).unwrap();
    let row = ledger::LaneRecord {
        schema: 1,
        repair: id.clone(),
        lane: r.lane.clone(),
        state: LaneState::Conflict,
        probation: None,
        merge: None,
        merge_history: Vec::new(),
        approval_ref: None,
        conflict_live: Vec::new(),
        applied: Vec::new(),
        applied_history: Vec::new(),
        merge_base: Vec::new(),
        merge_lock: None,
        merge_attempt: 0,
        reason: "훅 등록 해제를 이 부서에서 마치지 못했습니다 — (시험)".into(),
        updated_at: T0,
    };
    ledger::write_lane_in(&r.root, &row).unwrap();
    let w = seat(3, "worker");
    // the merge ticket's line 1, as `task-prompt --merge R-1` prints it for a hook-only repair
    r.call(T0, &w, "begin", json!({"trigger": "update", "unregister_hook": ["Stop:x.sh"], "joins": "R-1"})).unwrap();
    let locked = ledger::read_lane_in(&r.root, &id, &r.lane).unwrap().unwrap();
    assert!(locked.merge_lock.is_some(), "begin_joins took this lane's merge lock");
    // line 3: `recheck` — the lane's hook step (a double here) then 검증됨
    let steps = std::cell::Cell::new(0u32);
    let hook = |rid: &RepairId, l: &LaneKey, step: cys::repair::apply::HookStep| -> Result<cys::repair::apply::ReconcileReport, RepairError> {
        assert!(matches!(step, cys::repair::apply::HookStep::Unregister));
        steps.set(steps.get() + 1);
        // JI1: the operation record a real unit writes (the double writes it, completed, under the id the
        // verb's test unit reports)
        hook_op_record(&r, rid, &l.0, "test-hook-op", true);
        Ok(cys::repair::apply::ReconcileReport { hooks: Vec::new() })
    };
    let mut ctx = r.ctx(T0 + 1.0);
    ctx.hook_step = &hook;
    let v = handle_with(&ctx, &w, "recheck", &json!({"repair": "R-1"})).unwrap();
    assert_eq!(v["lane_state"], "verified", "{v}");
    assert_eq!(steps.get(), 1, "the lane's own hook step ran");
    // RULINGS JD2 / JI1 (added): the answer names the hook operation the call completed, READ from its record
    assert!(v["changed"].as_str().is_some_and(|c| c.ends_with("이 부서의 훅 등록 해제(작업 test-hook-op: Stop:x.sh)")), "{v}");
    let done = ledger::read_lane_in(&r.root, &id, &r.lane).unwrap().unwrap();
    assert_eq!(done.state, LaneState::Verified);
    assert!(done.merge_lock.is_none() && done.merge.is_some());
}

// ───────────────────────────── RULINGS JI1 — what a lane's hook units did, read from ONE source ─────────────────────────────

/// A hook operation record of `lane` for R, as T6's hook unit writes it (`finished` = completed, outcome
/// `applied`; else the stop right after the write-ahead record).
fn hook_op_record(r: &Rig, id: &RepairId, lane: &str, op: &str, finished: bool) {
    let rec = cys::repair::wal::OpRecord {
        schema: 1,
        op: op.into(),
        repair: id.clone(),
        lane_key: LaneKey(lane.into()),
        kind: "hook-unregister".into(),
        form: "hook".into(),
        rel: String::new(),
        pre_existed: false,
        expected: String::new(),
        incoming: String::new(),
        pack_generation: Some(cys::repair::apply::pack_generation(&r.pack)),
        slot: None,
        slot_identity: None,
        live_identity: None,
        incoming_copy: None,
        incoming_copy_identity: None,
        step: if finished { "done" } else { "recorded" }.into(),
        hooks: vec![cys::repair::wal::HookLine { event: "Stop".into(), script: "x.sh".into(), targets: Vec::new() }],
        finished,
        outcome: if finished { Some("applied".into()) } else { None },
        note: String::new(),
        created_at: T0,
        updated_at: T0,
        notice: None,
    };
    cys::repair::wal::write_in(&r.root, &rec).unwrap();
}

/// A hook-only repair of another department (`dept-x`), in force; this rig's lane (`base`) receives it.
fn hook_only_in_force(r: &Rig, id: &RepairId) {
    std::fs::create_dir_all(ledger::repair_dir_in(&r.root, id)).unwrap();
    let rec = RepairRecord {
        schema: 1,
        id: id.clone(),
        created_at: 1.0,
        trigger: cys::repair::types::Trigger::Update,
        symptom: String::new(),
        cause: String::new(),
        classification: Classification::General,
        files: vec![ledger::FileChange {
            rel: "settings:Stop:x.sh".into(),
            kind: ledger::ChangeKind::HookUnregister { event: "Stop".into(), script: "x.sh".into() },
            pre: ledger::PreImage::Absent,
            post: TargetState::Absent,
            diff: None,
        }],
        worker_record: None,
        verify: None,
        audit: None,
        audit_due_at: None,
        approval: None,
        owner_token_sha256: None,
        attempt: 1,
        methods: Vec::new(),
        resubmits: Vec::new(),
        pending_request: None,
        lock: None,
        state: RepairState::Probation,
        intake_signatures: Vec::new(),
        owner_original: false,
        origin_lane: LaneKey("dept-x".into()),
        resolves: None,
        committed: Vec::new(),
        fixer_seats: Vec::new(),
        checks: Vec::new(),
        reapplies: None,
    };
    ledger::write_record_in(&r.root, &rec).unwrap();
}

fn status_hook_ops(r: &Rig, id: &str) -> Value {
    r.call(T0 + 50.0, &seat(3, "worker"), "status", json!({"repair": id})).unwrap()
}

/// RULINGS JI (M1) — R in force, this lane's EXISTING 충돌 row with a valid merge lease; `recheck`'s hook
/// unit wrote its write-ahead record and the daemon stopped before any trace; T6's recovery completes
/// the operation and writes no row (the row stays as it was) → `status R` shows the completed operation
/// for this lane, read from its record.
#[test]
fn status_shows_a_hook_operation_recovery_completed_on_an_existing_conflict_row() {
    let r = rig("ji-m1");
    let id = RepairId::parse("R-1").unwrap();
    hook_only_in_force(&r, &id);
    let row = ledger::LaneRecord {
        schema: 1,
        repair: id.clone(),
        lane: r.lane.clone(),
        state: LaneState::Conflict,
        probation: None,
        merge: None,
        merge_history: Vec::new(),
        approval_ref: None,
        conflict_live: Vec::new(),
        applied: Vec::new(),
        applied_history: Vec::new(),
        merge_base: Vec::new(),
        merge_lock: Some(cys::repair::lock::new_lease(cys::repair::types::LockOwner { lane: r.lane.clone(), seat: 3, incarnation: "inc-1".into() }, T0, None)),
        merge_attempt: 1,
        reason: "합친 뒤의 다시 검사 실패 — (시험)".into(),
        updated_at: T0,
    };
    ledger::write_lane_in(&r.root, &row).unwrap();
    let op = cys::repair::wal::new_op_id(T0, "hook-unregister");
    hook_op_record(&r, &id, "base", &op, false);
    let settings = r.base.join("settings.json");
    let registrar = |_: &std::path::Path, _: bool| -> Result<String, RepairError> {
        Ok(json!({"hooks": [{"event": "Stop", "script": "x.sh", "lifeline": false, "desired": "unregistered",
                  "targets": [{"path": settings.to_string_lossy(), "state": "unregistered"}]}], "errors": []}).to_string())
    };
    assert_eq!(cys::repair::apply::resume_hook_steps_in(&r.root, &r.pack, &r.lane, &registrar, &cys::repair::apply::no_repair_lock, T0 + 10.0), 1);
    let after = ledger::read_lane_in(&r.root, &id, &r.lane).unwrap().unwrap();
    assert_eq!(after, row, "the recovery writes no row");
    let v = status_hook_ops(&r, "R-1");
    assert_eq!(v["hook_ops_unreadable"], Value::Null, "{v}");
    assert_eq!(v["hook_ops"], json!([{"lane": "base", "op": op, "kind": "hook-unregister", "lines": ["Stop:x.sh"], "text": format!("훅 등록 해제(작업 {op}: Stop:x.sh)")}]), "{v}");
}

/// RULINGS JI (M2) — a completed hook operation of this lane for R; then R is re-opened by the audit,
/// picked up, recorded, verified and committed again (the commit REBUILDS this lane's row) → `status R`
/// still shows the operation (nothing that rebuilds a row holds the trace).
#[test]
fn status_still_shows_a_completed_hook_operation_after_a_commit_that_follows_an_audit_reopen() {
    let r = rig("ji-m2");
    let w = seat(3, "worker");
    let id = verified(&r, "bin/a.py", b"print('fixed')\n");
    r.call(T0 + 3.0, &w, "commit", json!({"repair": id})).unwrap();
    let rid = RepairId::parse(&id).unwrap();
    let op = cys::repair::wal::new_op_id(T0 + 3.5, "hook-unregister");
    hook_op_record(&r, &rid, "base", &op, true);
    r.call(T0 + 4.0, &seat(8, "reviewer-gemini"), "attest_audit", json!({"repair": id, "verdict": "finding", "note": "again"})).unwrap();
    assert_eq!(r.record_of(&id).state, RepairState::ReopenedByAudit);
    assert_eq!(r.call(T0 + 5.0, &w, "begin", json!({"trigger": "owner_order", "files": ["bin/a.py"]})).unwrap()["begin"], "picked-up");
    std::fs::write(r.pack.join("bin/a.py"), b"print('fixed again')\n").unwrap();
    r.call(T0 + 6.0, &w, "record", json!({"repair": id, "note": "fix again"})).unwrap();
    r.call(T0 + 7.0, &seat(7, "reviewer-codex"), "attest_verify", json!({"repair": id, "verdict": "pass", "note": "ok"})).unwrap();
    r.call(T0 + 8.0, &w, "commit", json!({"repair": id})).unwrap();
    assert!(matches!(r.record_of(&id).state, RepairState::Probation | RepairState::InUse));
    let v = status_hook_ops(&r, &id);
    let ops = v["hook_ops"].as_array().unwrap();
    assert!(ops.iter().any(|o| o["lane"] == "base" && o["op"] == op.as_str() && o["text"] == format!("훅 등록 해제(작업 {op}: Stop:x.sh)")), "{v}");
}

/// RULINGS JL1 (a) — a repair with more operation records than the display read takes: `status` ANSWERS
/// (every other field as always), with `hook_ops` empty and the sentence "확인할 수 없습니다 — 작업 기록이
/// 많습니다"; nothing else fails.
#[test]
fn status_answers_when_a_repair_has_more_records_than_the_display_read_takes() {
    let r = rig("jl-many");
    let id = RepairId::parse("R-1").unwrap();
    hook_only_in_force(&r, &id);
    for k in 0..=cys::repair::propagate::DISPLAY_RECORDS_MAX {
        hook_op_record(&r, &id, "base", &format!("20000101T000000{k:06}Z-hook-unregister-1-{k}"), true);
    }
    let v = status_hook_ops(&r, "R-1");
    assert_eq!(v["repair"], "R-1", "{v}");
    assert!(v["files"].as_array().is_some_and(|f| f.len() == 1) && v["lane_rows"].is_array(), "{v}");
    assert_eq!(v["hook_ops"], json!([]), "{v}");
    let why = v["hook_ops_unreadable"].as_str().unwrap_or_default();
    assert!(why.starts_with("완료한 훅 작업을 확인할 수 없습니다 — 작업 기록이 많습니다"), "{v}");
}

/// RULINGS JI1 — an operation record that cannot be read: `status` says it cannot tell ("확인할 수
/// 없습니다", with the cause), never an empty list as if no hook operation had been completed.
#[test]
fn status_cannot_tell_the_completed_hook_operations_when_a_record_is_unreadable() {
    let r = rig("ji-unreadable");
    let id = RepairId::parse("R-1").unwrap();
    hook_only_in_force(&r, &id);
    hook_op_record(&r, &id, "base", &cys::repair::wal::new_op_id(T0, "hook-unregister"), true);
    std::fs::write(cys::repair::wal::ops_dir_in(&r.root, &id).join("broken.json"), b"{ not json").unwrap();
    let v = status_hook_ops(&r, "R-1");
    assert_eq!(v["hook_ops"], json!([]), "{v}");
    let why = v["hook_ops_unreadable"].as_str().unwrap_or_default();
    assert!(why.starts_with("완료한 훅 작업을 확인할 수 없습니다 — ") && why.contains("broken.json"), "{v}");
}
