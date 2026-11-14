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
    row = ledger::read_lane_in(&r.root, &id, &r.lane).unwrap().unwrap();
    assert_eq!(row.state, LaneState::Verified);
    let m = row.merge.unwrap();
    assert!(m.changed && m.by_seat == 3);
    assert!(row.merge_lock.is_none());
}
