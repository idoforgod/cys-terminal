//! Track T6 — daemon tests of `repair.revert` and `repair.reapply` through the verb dispatcher
//! (`handle_with`), with the role table of T3 in front of them.

use super::*;
use cys::repair::boot_critical::CheckKind;
use cys::repair::ledger::{self, RepairRecord};
use cys::repair::roles::CallerKind;
use cys::repair::types::{Classification, RepairId, RepairState};
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
    let base = std::env::temp_dir().join(format!("cys-t6d-{tag}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let pack = base.join("pack");
    std::fs::create_dir_all(pack.join("bin")).unwrap();
    std::fs::create_dir_all(pack.join("hooks")).unwrap();
    std::fs::write(pack.join(".pack-version"), "0.15.0\n").unwrap();
    std::fs::write(pack.join("bin/a.py"), b"print('a')\n").unwrap();
    std::fs::write(pack.join("hooks/h.sh"), b"#!/bin/sh\necho h\n").unwrap();
    // GM1: a `#!` file is captured with a generation only with its execute bit (the rig's script is
    // an executable hook, as installed)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(pack.join("hooks/h.sh"), std::fs::Permissions::from_mode(0o755)).unwrap();
    }
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
            product_pause: &|| false,
        }
    }
    fn call(&self, now: f64, who: &Caller, sub: &str, p: Value) -> Result<Value, VerbError> {
        handle_with(&self.ctx(now), who, sub, &p)
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

const T0: f64 = 1_800_000_000.0;

/// begin → edit → record → verify (another seat) → commit of `rel`; returns the id.
fn committed(r: &Rig, rel: &str, post: &[u8]) -> String {
    let w = seat(3, "worker");
    let b = r
        .call(T0, &w, "begin", json!({"trigger": "owner_order", "files": [rel]}))
        .unwrap();
    let id = b["repair"].as_str().unwrap().to_string();
    std::fs::write(r.pack.join(rel), post).unwrap();
    r.call(T0 + 1.0, &w, "record", json!({"repair": id, "note": "fix"}))
        .unwrap();
    r.call(
        T0 + 2.0,
        &seat(7, "reviewer-codex"),
        "attest_verify",
        json!({"repair": id, "verdict": "pass", "note": "ok"}),
    )
    .unwrap();
    let c = r
        .call(T0 + 3.0, &w, "commit", json!({"repair": id}))
        .unwrap();
    assert_eq!(c["state"], "probation", "{c}");
    id
}

/// RULINGS GK2 (codex r5 B2) with the REAL schedule writer: right after the repair engine's
/// exchange of `schedule.json` (a return of the pre-image), the daemon's own writer adds its
/// built-in jobs (`schedule::ensure_builtin_jobs_at` — among them the periodic self-heal job) — the
/// added jobs are in the final live schedule and the operation closes as conflict; nothing is
/// exchanged back over them. Without GK2 the engine exchanged the old schedule back: the added
/// jobs were live nowhere.
#[cfg(target_os = "macos")]
#[test]
fn the_schedule_writers_jobs_after_our_exchange_stay_live() {
    struct ScheduleWriterAfterExchange;
    impl cys::repair::apply::Faults for ScheduleWriterAfterExchange {
        fn at(&self, p: cys::repair::apply::Point, live: &Path, _: Option<&cys::repair::vault::Slot>) -> bool {
            if p == cys::repair::apply::Point::AfterSeal {
                crate::schedule::ensure_builtin_jobs_at(live);
            }
            false
        }
    }
    let r = rig("gk2sched");
    let id = RepairId::parse("R-82").unwrap();
    let live = r.pack.join("schedule.json");
    std::fs::write(&live, b"{\"jobs\": []}\n").unwrap();
    let pre = ledger::copy_preimage_in(&r.root, &id, &r.lane, "schedule.json", &live, T0).unwrap();
    let holder = match &pre {
        ledger::PreImage::Slot { slot_dir, .. } => cys::repair::vault::deposit_of(&cys::repair::vault::Slot::at(slot_dir, "schedule.json")).unwrap().1,
        ledger::PreImage::Absent => panic!("a pre-image"),
    };
    // the repair's applied schedule (what the lane row records)
    std::fs::write(&live, b"{\n \"jobs\": []\n}\n").unwrap();
    let applied = cys::repair::types::TargetState::read(&live).unwrap();
    let row = ledger::LaneRecord {
        schema: 1,
        repair: id.clone(),
        lane: r.lane.clone(),
        state: cys::repair::types::LaneState::Verified,
        probation: None,
        merge: None,
        merge_history: Vec::new(),
        approval_ref: None,
        conflict_live: Vec::new(),
        applied: vec![("schedule.json".into(), applied.canon())],
        applied_history: Vec::new(),
        merge_base: Vec::new(),
        merge_lock: None,
        merge_attempt: 0,
        updated_at: T0,
    };
    let faults = ScheduleWriterAfterExchange;
    let env = cys::repair::apply::OpEnv {
        root: &r.root,
        pack: &r.pack,
        lane: &r.lane,
        now: T0 + 1.0,
        form: cys::repair::platform::ApplyForm::Exchange,
        faults: &faults,
        own_check: &|_, _| None,
    };
    let spec = cys::repair::apply::OpSpec {
        id: id.clone(),
        rel: "schedule.json".into(),
        kind: "revert".into(),
        expected: cys::repair::apply::Recorded::applied_in(&row, "schedule.json").unwrap(),
        incoming: pre.state(),
        source: cys::repair::apply::Source::Slot { holder },
        bound: cys::repair::apply::Bound::now(&r.pack),
    };
    let done = cys::repair::apply::run_op(&env, &spec).unwrap();
    assert!(matches!(done.outcome, cys::repair::apply::ApplyOutcome::Declined(_)), "closed as conflict: {:?}", done.outcome);
    let jobs: serde_json::Value = serde_json::from_slice(&std::fs::read(&live).unwrap()).unwrap();
    assert!(jobs["jobs"].as_array().is_some_and(|a| !a.is_empty()), "the writer's jobs are live: {jobs}");
}

/// RULINGS GK3: a notice closed "not confirmed" (handed out three times without its receiver's
/// confirmation) stays visible in `cys repair status`.
#[test]
fn status_lists_the_notices_not_confirmed() {
    let r = rig("gk3status");
    let p = cys::repair::probation::notices_handed_path_in(&r.root, &r.lane);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, json!({"schema": 1, "lane": r.lane.0, "counts": {"f:hooks/x.sh:ab:kept": 3, "f:hooks/y.sh:cd:kept": 1}}).to_string()).unwrap();
    let s = r.call(T0, &human(), "status", json!({})).unwrap();
    assert_eq!(s["notices_not_confirmed"], json!(["f:hooks/x.sh:ab:kept"]), "{s}");
}

/// RULINGS GK4 (codex r5 M2) through the daemon's commit verb: the pack is updated BETWEEN `begin`
/// (whose pre-image copy records its generation) and `commit` — the commit still succeeds, the row
/// says the repair cannot be returned automatically, it is not offered to a revert, and nothing is
/// put back. Without GK4 the general commit stamped the current generation with `closed: None`.
#[cfg(target_os = "macos")]
#[test]
fn a_general_commit_after_an_update_is_not_returned_automatically() {
    let r = rig("gk4");
    let w = seat(3, "worker");
    let b = r.call(T0, &w, "begin", json!({"trigger": "owner_order", "files": ["bin/a.py"]})).unwrap();
    let id = b["repair"].as_str().unwrap().to_string();
    std::fs::write(r.pack.join(".pack-version"), "0.15.1\n").unwrap();
    std::fs::write(r.pack.join("bin/a.py"), b"print('fixed')\n").unwrap();
    r.call(T0 + 1.0, &w, "record", json!({"repair": id, "note": "fix"})).unwrap();
    r.call(T0 + 2.0, &seat(7, "reviewer-codex"), "attest_verify", json!({"repair": id, "verdict": "pass", "note": "ok"})).unwrap();
    let c = r.call(T0 + 3.0, &w, "commit", json!({"repair": id})).unwrap();
    assert_eq!(c["state"], "probation", "the commit still succeeds: {c}");
    let rid = RepairId::parse(&id).unwrap();
    let row = ledger::read_lane_in(&r.root, &rid, &r.lane).unwrap().unwrap();
    let closed = row.probation.as_ref().unwrap().closed.clone().unwrap_or_default();
    assert!(closed.contains(cys::repair::apply::NOT_REVERTIBLE_SENTENCE), "the row says so: {closed}");
    assert!(!cys::repair::apply::revertible_here(&r.record_of(&id), &row, &r.lane), "not offered to the return");
    assert!(r.call(T0 + 4.0, &human(), "revert", json!({"repair": id})).is_err(), "a revert is refused");
    assert_eq!(std::fs::read(r.pack.join("bin/a.py")).unwrap(), b"print('fixed')\n", "nothing put back");
}

/// `repair.revert` from a human and from a worker seat of the lane: the pre-image is live again by
/// rule one, the repaired copy is in the vault, the record is 되돌려짐; a master is refused by the
/// role table (C3: master never) and nothing changes.
#[cfg(target_os = "macos")]
#[test]
fn revert_verb_by_human_and_worker_master_refused() {
    let r = rig("revert");
    let id = committed(&r, "bin/a.py", b"print('fixed')\n");
    let e = r
        .call(T0 + 5.0, &seat(1, "master"), "revert", json!({"repair": id}))
        .unwrap_err();
    assert_eq!(e.code, "acl_denied", "{}", e.message);
    assert_eq!(std::fs::read(r.pack.join("bin/a.py")).unwrap(), b"print('fixed')\n");
    let v = r
        .call(T0 + 6.0, &human(), "revert", json!({"repair": id}))
        .unwrap();
    assert_eq!(v["reverted"], json!(true), "{v}");
    assert_eq!(std::fs::read(r.pack.join("bin/a.py")).unwrap(), b"print('a')\n");
    assert_eq!(r.record_of(&id).state, RepairState::Reverted);
    // a second repair reverted by a worker seat of the lane
    let id2 = committed(&r, "hooks/h.sh", b"#!/bin/sh\necho fixed\n");
    let v = r
        .call(T0 + 7.0, &seat(13, "worker-3"), "revert", json!({"repair": id2}))
        .unwrap();
    assert_eq!(v["reverted"], json!(true), "{v}");
    assert_eq!(std::fs::read(r.pack.join("hooks/h.sh")).unwrap(), b"#!/bin/sh\necho h\n");
}

/// RULINGS B4 / K3 / CH3 (the case T4 left untested because `revert` is T6's): while the grant is
/// PAUSED by an owner input, a running repair may still CLOSE — `revert` is accepted.
#[cfg(target_os = "macos")]
#[test]
fn revert_accepted_while_grant_paused() {
    let r = rig("paused");
    let id = committed(&r, "bin/a.py", b"print('fixed')\n");
    cys::repair::grant::write_decision_in(
        &r.root,
        &cys::repair::grant::GrantDecision {
            schema: 1,
            state: cys::repair::grant::GrantState::On,
            daily_cap: 5,
            decided_at: T0,
            decided_lane: r.lane.clone(),
            decided_seat: 1,
            prompt_digest: String::new(),
            prompt_excerpt: String::new(),
            paused_at: None,
            resumed_at: None,
            generation: 0,
        },
    )
    .unwrap();
    assert!(cys::repair::grant::note_owner_input_in(&r.root, &r.lane, T0 + 4.0).unwrap());
    assert!(cys::repair::grant::is_paused_in(&r.root, T0 + 5.0), "the grant is paused");
    let v = r
        .call(T0 + 5.0, &seat(13, "worker-3"), "revert", json!({"repair": id}))
        .unwrap();
    assert_eq!(v["reverted"], json!(true), "{v}");
}

/// The pack-apply lock held by `cys pack-update` → the request is refused at once (never blocks the
/// CLI's 40 s), nothing changed.
#[cfg(target_os = "macos")]
#[test]
fn revert_refused_while_pack_update_holds_the_lock() {
    let r = rig("locked");
    let id = committed(&r, "bin/a.py", b"print('fixed')\n");
    let lock = cys::pack_apply_lock::pack_apply_lock_path_for(&r.pack);
    let (tx, rx) = std::sync::mpsc::channel();
    let (release, wait) = std::sync::mpsc::channel::<()>();
    let l2 = lock.clone();
    let holder = std::thread::spawn(move || {
        cys::pack_apply_lock::with_apply_lock(&l2, || {
            tx.send(()).unwrap();
            wait.recv().unwrap();
        })
        .unwrap();
    });
    rx.recv().unwrap();
    let e = r
        .call(T0 + 5.0, &human(), "revert", json!({"repair": id}))
        .unwrap_err();
    assert_eq!(e.code, cys::repair::types::ERR_LOCKED, "{}", e.message);
    assert_eq!(std::fs::read(r.pack.join("bin/a.py")).unwrap(), b"print('fixed')\n");
    release.send(()).unwrap();
    holder.join().unwrap();
}

/// Plants what the update's vendor-first arm parks for `rel` (src/pack.rs `park_user_copy` + the
/// `.merge-pending.json` entry) and puts the vendor's bytes live.
fn park(r: &Rig, rel: &str, user: &[u8], vendor: &[u8]) {
    std::fs::write(r.pack.join(format!("{rel}.user")), user).unwrap();
    let mut pending = cys::pack::load_merge_pending(&r.pack);
    pending.insert(
        rel.to_string(),
        json!({"kind": "vendor-first", "side": format!("{rel}.user"), "reason": "vendor-advanced", "parked_at": 1, "version": "0.15.1", "ts": 1}),
    );
    cys::pack::save_merge_pending(&r.pack, &pending);
    std::fs::write(r.pack.join(rel), vendor).unwrap();
}

/// RULINGS AQ5 / AR1: `reapply R` from a worker starts R′ through `begin`'s body — trigger update, the
/// vendor bytes as pre-image, `reapplies: R`, the parked copy named; a second `reapply R` is refused
/// naming R′; R without a parked copy here is refused; a reviewer is refused.
#[test]
fn reapply_starts_a_new_repair_once() {
    let r = rig("reapply");
    let w = seat(3, "worker");
    let id = committed(&r, "hooks/h.sh", b"#!/bin/sh\necho mine\n");
    // nothing parked yet → refused
    let e = r
        .call(T0 + 10.0, &w, "reapply", json!({"repair": id}))
        .unwrap_err();
    assert_eq!(e.code, cys::repair::types::ERR_PRECONDITION, "{}", e.message);
    assert!(e.message.contains("보관한"), "{}", e.message);
    // the update put the vendor bytes first and parked the edit
    park(&r, "hooks/h.sh", b"#!/bin/sh\necho mine\n", b"#!/bin/sh\necho vendor\n");
    let mut rec = r.record_of(&id);
    rec.state = RepairState::Superseded;
    ledger::write_record_in(&r.root, &rec).unwrap();
    let e = r
        .call(T0 + 11.0, &seat(7, "reviewer-codex"), "reapply", json!({"repair": id}))
        .unwrap_err();
    assert_eq!(e.code, "acl_denied", "{}", e.message);
    let v = r
        .call(T0 + 12.0, &w, "reapply", json!({"repair": id}))
        .unwrap();
    let new_id = v["repair"].as_str().unwrap().to_string();
    assert_ne!(new_id, id);
    assert_eq!(v["reapplies"], json!(id));
    assert_eq!(v["parked"][0]["rel"], json!("hooks/h.sh"));
    assert!(v["parked"][0]["parked_copy"].as_str().unwrap().ends_with("hooks/h.sh.user"));
    let rp = r.record_of(&new_id);
    assert_eq!(rp.reapplies.as_ref().map(|x| x.as_str()), Some(id.as_str()));
    assert_eq!(rp.trigger, cys::repair::types::Trigger::Update);
    assert_eq!(rp.state, RepairState::InProgress);
    assert!(rp.lock.is_some(), "R′ holds its own lock");
    let vendor = cys::repair::types::TargetState::read(&r.pack.join("hooks/h.sh")).unwrap();
    assert_eq!(rp.files[0].pre.state(), vendor, "the vendor bytes now live are R′'s pre-image");
    assert_eq!(r.record_of(&id).state, RepairState::Superseded, "R is not re-opened");
    assert!(r.record_of(&id).lock.is_none(), "R is not locked");
    // status shows it
    let st = r
        .call(T0 + 13.0, &human(), "status", json!({"repair": new_id}))
        .unwrap();
    assert_eq!(st["reapplies"], json!(id));
    // AR1: a second reapply of R is refused naming R′
    let e = r
        .call(T0 + 14.0, &w, "reapply", json!({"repair": id}))
        .unwrap_err();
    assert!(e.message.contains(&new_id), "{}", e.message);
    assert!(e.message.contains("전파"), "{}", e.message);
}

/// RULINGS CQ2: `reapply`'s "no parked copy" refusal is true in both sub-cases — (i) neither the
/// `vendor-first` record nor `<rel>.user` → "다시 입힐 것이 없습니다" (unchanged sentence); (ii) the copy
/// is on disk but no `vendor-first` record names it (the boot sweep replaced it by `kept-drift`) → the
/// sentence names the copy and the route that works, and says nothing about "nothing to re-apply".
/// Both sentences pinned in full.
#[test]
fn reapply_refusal_is_true_with_and_without_the_kept_copy() {
    let r = rig("reapplycq2");
    let w = seat(3, "worker");
    let id = committed(&r, "hooks/h.sh", b"#!/bin/sh\necho mine\n");
    // (i) neither the record nor the copy
    let e = r.call(T0 + 10.0, &w, "reapply", json!({"repair": id})).unwrap_err();
    assert_eq!(e.code, cys::repair::types::ERR_PRECONDITION, "{}", e.message);
    assert_eq!(
        e.message,
        // (every refusal reaches the wire with the verb's `repair.<verb>: ` prefix — `handle_with`)
        format!("repair.reapply: {id}: 이 부서의 팩에는 업데이트가 보관한 {id} 의 수정본이 없습니다(vendor-first 기록과 보관 사본 <파일>.user 가 함께 있어야 합니다) — 다시 입힐 것이 없습니다")
    );
    // (ii) the update parked the edit, then the boot sweep replaced the record by `kept-drift`
    park(&r, "hooks/h.sh", b"#!/bin/sh\necho mine\n", b"#!/bin/sh\necho vendor\n");
    let mut pending = cys::pack::load_merge_pending(&r.pack);
    pending.insert("hooks/h.sh".into(), json!({"kind": "kept-drift", "version": "0.15.1", "ts": 2}));
    cys::pack::save_merge_pending(&r.pack, &pending);
    let copy = r.pack.join("hooks/h.sh.user");
    let e = r.call(T0 + 11.0, &w, "reapply", json!({"repair": id})).unwrap_err();
    assert_eq!(e.code, cys::repair::types::ERR_PRECONDITION, "{}", e.message);
    assert_eq!(
        e.message,
        format!(
            "repair.reapply: {id}: 보관 사본 {} 는 그대로 있지만 그 사본을 가리키는 vendor-first 기록이 없어 다시 입히기(reapply)로는 입히지 않습니다 — 그 사본을 살리는 길은 수리입니다: cys repair begin --files hooks/h.sh --trigger update (작업 사본에서 <파일>.user 를 합친 뒤 검증·감사 서명 → 주인 최종 승인)",
            copy.display()
        )
    );
    assert!(!e.message.contains("다시 입힐 것이 없습니다"), "{}", e.message);
    // the same with no record at all
    let mut pending = cys::pack::load_merge_pending(&r.pack);
    pending.remove("hooks/h.sh");
    cys::pack::save_merge_pending(&r.pack, &pending);
    let e = r.call(T0 + 12.0, &w, "reapply", json!({"repair": id})).unwrap_err();
    assert!(e.message.contains(&copy.display().to_string()) && e.message.contains("cys repair begin --files hooks/h.sh --trigger update"), "{}", e.message);
    // nothing was started and nothing in the pack changed
    assert_eq!(std::fs::read(&copy).unwrap(), b"#!/bin/sh\necho mine\n");
    assert_eq!(ledger::list_ids_in(&r.root).len(), 1, "no new repair");
    // the route the sentence names is open (RULINGS CN) — R itself was superseded by the update
    let mut rec = r.record_of(&id);
    rec.state = RepairState::Superseded;
    ledger::write_record_in(&r.root, &rec).unwrap();
    let v = r.call(T0 + 13.0, &w, "begin", json!({"trigger": "update", "files": ["hooks/h.sh"]})).unwrap();
    assert_eq!(v["begin"], "new", "{v}");
}

/// CT8 / DR4: AR1 across lanes — another lane's claim on R, naming its live R′, is a durable
/// exclusive claim in the shared ledger: this lane's `reapply R` binds its own R′, loses the claim,
/// closes that R′ (it re-applies nothing) and answers naming the holder. (REWRITTEN with DR4 — old
/// expectation: the refusal said "다른 부서가 방금" and `list_ids_in(..).len() == 1` "nothing
/// started"; under the grant protocol R′ is bound BEFORE the claim, so it exists and is closed.)
#[test]
fn reapply_respects_another_lanes_claim() {
    let r = rig("ct8claim");
    let w = seat(3, "worker");
    let id = committed(&r, "hooks/h.sh", b"#!/bin/sh\necho mine\n");
    park(&r, "hooks/h.sh", b"#!/bin/sh\necho mine\n", b"#!/bin/sh\necho vendor\n");
    let mut rec = r.record_of(&id);
    rec.state = RepairState::Superseded;
    ledger::write_record_in(&r.root, &rec).unwrap();
    let rid = cys::repair::types::RepairId::parse(&id).unwrap();
    let other = cys::repair::types::LaneKey("dept-9".into());
    let theirs = cys::repair::types::RepairId::parse("R-77").unwrap();
    assert_eq!(
        cys::repair::reapply_claim::claim_in(&r.root, &rid, &other, &theirs, T0 + 11.0, &|_| cys::repair::reapply_claim::Holder::Alive),
        Ok(1)
    );
    let e = r.call(T0 + 12.0, &w, "reapply", json!({"repair": id})).unwrap_err();
    assert!(e.message.contains("R-77") && e.message.contains("두 번 입히지 않습니다"), "{}", e.message);
    let ids = ledger::list_ids_in(&r.root);
    assert_eq!(ids.len(), 2, "our R′ was bound, then closed");
    let ours = r.record_of(ids.last().unwrap().as_str());
    assert_eq!(ours.state, RepairState::Superseded, "our R′ re-applies nothing");
    assert!(ours.lock.is_none());
}

/// CU4: a parked `<rel>.user` that is NOT R's own edit (its bytes differ from R's repaired state)
/// is refused by `reapply R`, naming the copy and the hand-edit route.
#[test]
fn reapply_refuses_a_parked_copy_that_is_not_rs_edit() {
    let r = rig("cu4foreign");
    let w = seat(3, "worker");
    let id = committed(&r, "hooks/h.sh", b"#!/bin/sh\necho mine\n");
    park(&r, "hooks/h.sh", b"#!/bin/sh\necho someone-elses\n", b"#!/bin/sh\necho vendor\n");
    let e = r.call(T0 + 12.0, &w, "reapply", json!({"repair": id})).unwrap_err();
    assert!(e.message.contains("의 수정본이 아닙니다") && e.message.contains("cys repair begin --files hooks/h.sh --trigger update"), "{}", e.message);
    assert_eq!(ledger::list_ids_in(&r.root).len(), 1, "nothing started");
}

/// `tick` / `sweep` reach the daemon's own handler only; through the bare dispatcher (no daemon) they
/// are refused with a sentence (the old expectation of T3's census was `repair_not_implemented`).
#[test]
fn tick_and_sweep_need_the_daemon() {
    let r = rig("tick");
    for verb in ["tick", "sweep"] {
        let e = r.call(T0, &human(), verb, json!({})).unwrap_err();
        assert_eq!(e.code, cys::repair::types::ERR_PRECONDITION, "{verb}");
    }
}

fn hooks_boot(rel: &str) -> Result<Classification, RepairError> {
    Ok(if rel.starts_with("hooks/") {
        Classification::BootCritical
    } else {
        Classification::General
    })
}

/// RULINGS CN (CCR-2): the route for a user's OWN hand edit parked by the update is open — a worker
/// seat, trigger `update` (no owner order, no grant) → `begin --files <rel>` starts a boot-critical
/// repair whose working copy holds the vendor bytes now live, with the parked `<rel>.user` readable
/// in the lane's pack for the merge; nothing refuses it.
#[test]
fn hand_edit_route_begin_trigger_update_is_open() {
    let r = rig("handedit");
    park(&r, "hooks/h.sh", b"#!/bin/sh\necho my-own-edit\n", b"#!/bin/sh\necho vendor\n");
    let ctx = Ctx { classify: &hooks_boot, ..r.ctx(T0) };
    let v = handle_with(
        &ctx,
        &seat(3, "worker"),
        "begin",
        &json!({"trigger": "update", "files": ["hooks/h.sh"]}),
    )
    .unwrap();
    assert_eq!(v["begin"], "new", "{v}");
    assert_eq!(v["classification"], "boot_critical", "{v}");
    let staging = std::path::PathBuf::from(v["staging"].as_str().expect("a working copy"));
    assert_eq!(std::fs::read(staging.join("hooks/h.sh")).unwrap(), b"#!/bin/sh\necho vendor\n");
    assert_eq!(
        std::fs::read(r.pack.join("hooks/h.sh.user")).unwrap(),
        b"#!/bin/sh\necho my-own-edit\n",
        "the parked edit is readable for the merge"
    );
    let id = v["repair"].as_str().unwrap();
    assert_eq!(r.record_of(id).trigger, cys::repair::types::Trigger::Update);
}

/// CT11: `repair.revert {"latest": true}` reverts the repair most recently APPLIED in this lane by
/// the lane's own application evidence (`applied_at` of its row) — not the highest repair number and
/// not "this lane's own origin first". Without CT11 the app picked R-10 (number) / its own origin.
#[test]
fn revert_latest_is_chosen_by_this_lanes_application_time() {
    let r = rig("ct11latest");
    let newer_number = committed(&r, "hooks/n.sh", b"#!/bin/sh\necho n\n");
    let applied_later = committed(&r, "hooks/l.sh", b"#!/bin/sh\necho l\n");
    // the repair with the LOWER number was applied here later (e.g. a propagated repair)
    let (lo, hi) = if newer_number < applied_later { (newer_number.clone(), applied_later.clone()) } else { (applied_later.clone(), newer_number.clone()) };
    for (id, t) in [(&lo, T0 + 500.0), (&hi, T0 + 100.0)] {
        let rid = cys::repair::types::RepairId::parse(id).unwrap();
        let mut row = ledger::read_lane_in(&r.root, &rid, &r.lane).unwrap().unwrap();
        row.probation.as_mut().unwrap().applied_at = t;
        ledger::write_lane_in(&r.root, &row).unwrap();
    }
    let v = r.call(T0 + 600.0, &human(), "revert", json!({"latest": true})).unwrap();
    assert_eq!(v["repair"], json!(lo), "the one applied here last, not the highest number: {v}");
}

/// DX1: "revertible" has ONE definition — the latest repair applied here failed and was closed
/// `실패 — …` (CU1): the button reverts IT (not an older healthy one); with nothing revertible the
/// answer is an honest "nothing to revert". Without DX1 the picker's own state list skipped the
/// failed repair.
#[test]
fn revert_latest_takes_a_failed_repair_and_says_nothing_honestly() {
    let r = rig("dx1failed");
    let older = committed(&r, "hooks/o.sh", b"#!/bin/sh\necho o\n");
    let failed = committed(&r, "hooks/f.sh", b"#!/bin/sh\necho f\n");
    for (id, t, closed) in [(&older, T0 + 100.0, None), (&failed, T0 + 500.0, Some("실패 — 재검사 실패, 되돌리지 못했습니다"))] {
        let rid = cys::repair::types::RepairId::parse(id).unwrap();
        let mut row = ledger::read_lane_in(&r.root, &rid, &r.lane).unwrap().unwrap();
        let p = row.probation.as_mut().unwrap();
        p.applied_at = t;
        p.closed = closed.map(str::to_string);
        if closed.is_some() {
            row.state = cys::repair::types::LaneState::Conflict;
        }
        ledger::write_lane_in(&r.root, &row).unwrap();
    }
    let v = r.call(T0 + 600.0, &human(), "revert", json!({"latest": true})).unwrap();
    assert_eq!(v["repair"], json!(failed), "the failed repair applied last: {v}");
    // nothing revertible any more → honest refusal
    let empty = rig("dx1empty");
    let e = empty.call(T0, &human(), "revert", json!({"latest": true})).unwrap_err();
    assert!(e.message.contains("되돌릴 수리가 없습니다"), "{}", e.message);
}

/// EJ4 / EN6 through the real entry points: a COMMIT holds its repair's guard from its top through
/// its hook phase — while it is in its hook dry run, the job's first-ready start pass leaves the
/// repair alone (no second hook operation). Without the guard at commit's top the start pass saw a
/// committed repair whose unregistration was never attempted and ran its own hook step.
#[test]
fn a_commit_in_its_hook_phase_excludes_the_start_pass() {
    let r = rig("ej4commit");
    let w = seat(3, "worker");
    let id = r.call(T0, &w, "begin", json!({"trigger": "owner_order", "files": ["bin/a.py"]})).unwrap()["repair"].as_str().unwrap().to_string();
    let rid = RepairId::parse(&id).unwrap();
    let mut rec = r.record_of(&id);
    rec.files.push(cys::repair::ledger::FileChange {
        rel: "settings:Stop:x.sh".into(),
        kind: cys::repair::ledger::ChangeKind::HookUnregister { event: "Stop".into(), script: "x.sh".into() },
        pre: cys::repair::ledger::PreImage::Absent,
        post: cys::repair::types::TargetState::Absent,
        diff: None,
    });
    ledger::write_record_in(&r.root, &rec).unwrap();
    r.call(T0 + 1.0, &w, "record", json!({"repair": id, "note": "n"})).unwrap();
    r.call(T0 + 2.0, &seat(7, "reviewer-codex"), "attest_verify", json!({"repair": id, "verdict": "pass", "note": "ok"})).unwrap();
    let (in_tx, in_rx) = std::sync::mpsc::channel::<()>();
    let (go_tx, go_rx) = std::sync::mpsc::channel::<()>();
    let rr = &r;
    let wr = &w;
    let idr = &id;
    std::thread::scope(|sc| {
        let commit = sc.spawn(move || {
            let hook = move |_: &RepairId, _: &cys::repair::types::LaneKey, _: cys::repair::apply::HookStep| -> Result<cys::repair::apply::ReconcileReport, RepairError> {
                in_tx.send(()).unwrap();
                go_rx.recv().unwrap();
                Ok(cys::repair::apply::ReconcileReport { hooks: Vec::new() })
            };
            let ctx = Ctx { hook_step: &hook, ..rr.ctx(T0 + 3.0) };
            handle_with(&ctx, wr, "commit", &json!({"repair": idr}))
        });
        in_rx.recv().unwrap();
        // the job's first-ready start pass, meanwhile
        let calls = std::cell::Cell::new(0);
        let hooks = |_: &RepairId, _: &cys::repair::types::LaneKey, _: cys::repair::apply::HookStep| -> Result<cys::repair::apply::ReconcileReport, RepairError> {
            calls.set(calls.get() + 1);
            Ok(cys::repair::apply::ReconcileReport { hooks: Vec::new() })
        };
        let lock = r.root.join(".pack-apply.lock");
        let env = cys::repair::probation::JobEnv {
            op: cys::repair::apply::OpEnv { root: &r.root, pack: &r.pack, lane: &r.lane, now: T0 + 4.0, form: cys::repair::platform::ApplyForm::Exchange, faults: &cys::repair::apply::NoFaults, own_check: &|_, _| None },
            incarnation: "inc-x",
            check: &|_, _, _| Err(RepairError::Io("no checks here".into())),
            reference: &|_, _| Err(RepairError::Io("no checks here".into())),
            hooks: &hooks,
            registrar: &|_, _| Ok("{\"hooks\": []}".into()),
            locked: &cys::repair::apply::no_repair_lock,
            excluded: &|_| false,
            original: &|_| None,
            intake: &|_, _| true,
            apply_lock: &lock,
        };
        let _ = cys::repair::probation::start_time_pass(&env);
        assert_eq!(calls.get(), 0, "the start pass left the committing repair alone");
        assert!(cys::repair::guard::is_held(&r.root, &rid), "the commit holds the guard in its hook phase");
        go_tx.send(()).unwrap();
        commit.join().unwrap().unwrap();
    });
}

// ───────────────────────────── RULINGS HS — an update inside a general repair's probation ─────────────────────────────

/// The job's probation pass over the rig (the daemon's job with test doubles; after an update no
/// check and no reference check may run).
fn run_probation(r: &Rig, now: f64) -> cys::repair::probation::TickReport {
    let never = |_: &str, _: &Path, _: std::time::Duration| -> Result<cys::repair::apply::CheckOutcome, RepairError> { panic!("no check after an update") };
    let reference = |_: CheckKind, _: std::time::Duration| -> Result<cys::repair::apply::CheckOutcome, RepairError> { panic!("no reference check after an update") };
    let hooks = |_: &RepairId, _: &LaneKey, _: cys::repair::apply::HookStep| -> Result<cys::repair::apply::ReconcileReport, RepairError> {
        Ok(cys::repair::apply::ReconcileReport { hooks: Vec::new() })
    };
    let registrar = |_: &Path, _: bool| -> Result<String, RepairError> { Ok("{\"hooks\": []}".into()) };
    let lock = r.base.join(".pack-apply.lock");
    let env = cys::repair::probation::JobEnv {
        op: cys::repair::apply::OpEnv {
            root: &r.root,
            pack: &r.pack,
            lane: &r.lane,
            now,
            form: cys::repair::platform::ApplyForm::Exchange,
            faults: &cys::repair::apply::NoFaults,
            own_check: &|_, _| None,
        },
        incarnation: "inc-2",
        check: &never,
        reference: &reference,
        hooks: &hooks,
        registrar: &registrar,
        locked: &cys::repair::apply::no_repair_lock,
        excluded: &|_| true,
        original: &|_| None,
        intake: &|_, _| true,
        apply_lock: &lock,
    };
    cys::repair::probation::run_tick(&env, false).unwrap()
}

/// Two committed general repairs, then an update inside their probation: a new pack generation that
/// keeps `bin/a.py` (the first repair's bytes survive — HS1 (b)) and writes `hooks/h.sh` (the
/// second's — HS1 (a)); then the job's probation pass. Returns (kept, wrote).
fn an_update_inside_two_probations(r: &Rig) -> (String, String) {
    let kept = committed(r, "bin/a.py", b"print('fixed')\n");
    let wrote = committed(r, "hooks/h.sh", b"#!/bin/sh\necho fixed\n");
    std::fs::write(r.pack.join(".pack-version"), "0.15.1\n").unwrap();
    std::fs::write(r.pack.join("hooks/h.sh"), b"#!/bin/sh\necho vendor\n").unwrap();
    let _ = run_probation(r, T0 + 60.0);
    (kept, wrote)
}

/// RULINGS HS1 through the daemon's verbs: the repair whose bytes survived the update is in force
/// (사용 중) in `list`, and `cleanup` refuses its vault slots (a repair in use still depends on them);
/// the one whose file the update wrote is superseded and its slots may go (as before HS1). Without
/// HS1 both were superseded and both vaults were offered to `cleanup`.
#[test]
fn an_update_that_kept_the_bytes_leaves_the_repair_in_force_with_its_vault() {
    let r = rig("hs1verbs");
    let (kept, wrote) = an_update_inside_two_probations(&r);
    let l = r.call(T0 + 61.0, &human(), "list", json!({})).unwrap();
    let state_of = |id: &str| l["repairs"].as_array().unwrap().iter().find(|s| s["repair"] == id).map(|s| s["state"].clone());
    assert_eq!(state_of(&kept), Some(json!("in_use")), "{l}");
    assert_eq!(state_of(&wrote), Some(json!("superseded")), "{l}");
    let kept_id = RepairId::parse(&kept).unwrap();
    let slots = ledger::list_slots_in(&r.root, &kept_id);
    assert!(!slots.is_empty());
    // every slot it has is one a repair in force depends on (P6 — the verb's own match)
    let needed = cys::repair::apply::slots_needed_in_force_in(&r.root, &kept_id).unwrap();
    for s in &slots {
        assert!(needed.iter().any(|n| n == &s.dir || s.dir.starts_with(n) || n.starts_with(&s.dir)), "{} not needed: {needed:?}", s.dir.display());
    }
    // the verb removes nothing of it (in this rig the verb reads the product root for the needed
    // slots, cannot, and refuses every slot — the refusal path of an in-force repair)
    match r.call(T0 + 62.0, &human(), "cleanup", json!({"repair": kept})) {
        Ok(c) => assert!(c["removed"].as_array().is_some_and(|a| a.is_empty()), "nothing of a repair in use is removed: {c}"),
        Err(e) => assert_eq!(e.code, "repair_precondition", "{}", e.message),
    }
    assert_eq!(ledger::list_slots_in(&r.root, &kept_id).len(), slots.len(), "nothing of a repair in use is removed");
    let c = r.call(T0 + 63.0, &human(), "cleanup", json!({"repair": wrote})).unwrap();
    assert!(c["removed"].as_array().is_some_and(|a| !a.is_empty()), "a superseded repair's slots may go: {c}");
}
