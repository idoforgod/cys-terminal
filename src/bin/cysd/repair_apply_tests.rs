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
