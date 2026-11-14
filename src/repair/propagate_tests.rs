//! T7b tests of `propagate.rs`: the gate's reader, the pack-instance pass, each lane's own rows, the
//! general forms through T6's engine, the rule of the core for approval-class repairs (RULINGS HJ1),
//! `recheck` (H8) and the committed copy (HI2 / HJ2). Two lanes share one repairs root: the origin
//! `base` (HQ) and the department `dept-a`, each with its own pack.

use super::*;
use crate::repair::apply::{CheckOutcome, Faults, NoFaults, Point, ReconcileReport};
use crate::repair::ledger::{ChangeKind, FileChange, PreImage, Probation};
use crate::repair::types::{LockOwner, Trigger};
use crate::repair::vault::Slot;
use std::cell::Cell;
use std::time::Duration;

const T0: f64 = 1_800_000_000.0;
const HQ: &str = "base";
const DEPT: &str = "dept-a";

struct Rig {
    base: PathBuf,
    root: PathBuf,
    hq: PathBuf,
    dept: PathBuf,
    registry: PathBuf,
}

impl Drop for Rig {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // a test may have taken a permission away; give it back so the rig can be removed
            for p in [self.hq.clone(), self.dept.clone(), self.root.join("lanes")] {
                let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755));
                let _ = std::fs::set_permissions(crate::repair::paths::pack_instance_path(&p), std::fs::Permissions::from_mode(0o644));
            }
        }
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

fn rig(tag: &str) -> Rig {
    let base = std::env::temp_dir().join(format!(
        "cys-t7b-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    let root = base.join("local").join("repairs");
    let hq = base.join("pack");
    let dept = base.join("pack-dept-dept-a");
    for d in [&root, &hq, &dept] {
        std::fs::create_dir_all(d.join("bin")).unwrap();
        std::fs::create_dir_all(d.join("hooks")).unwrap();
    }
    for p in [&hq, &dept] {
        std::fs::write(p.join(".pack-version"), "0.15.0\n").unwrap();
    }
    let registry = base.join("depts.json");
    std::fs::write(&registry, r#"{"depts": {"a": {"socket": "/x/cys-dept-dept-a/cys.sock"}}}"#).unwrap();
    Rig { base, root, hq, dept, registry }
}

fn lane(s: &str) -> LaneKey {
    LaneKey(s.into())
}

fn rid(s: &str) -> RepairId {
    RepairId::parse(s).unwrap()
}

fn fid(n: u8) -> String {
    format!("{n:064x}")
}

impl Rig {
    fn pack(&self, which: &str) -> &Path {
        if which == HQ {
            &self.hq
        } else {
            &self.dept
        }
    }
    fn put(&self, which: &str, rel: &str, bytes: &[u8], mode: u32) -> TargetState {
        let p = self.pack(which).join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        let _ = std::fs::remove_file(&p);
        std::fs::write(&p, bytes).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode)).unwrap();
        }
        let _ = mode;
        TargetState::read(&p).unwrap()
    }
    fn live(&self, which: &str, rel: &str) -> TargetState {
        TargetState::read(&self.pack(which).join(rel)).unwrap()
    }
    fn mtime(&self, which: &str, rel: &str) -> Option<std::time::SystemTime> {
        std::fs::symlink_metadata(self.pack(which).join(rel)).ok().and_then(|m| m.modified().ok())
    }
    fn row(&self, id: &RepairId, which: &str) -> Option<LaneRecord> {
        ledger::read_lane_in(&self.root, id, &lane(which)).unwrap()
    }
    fn job(&self, which: &str) -> JobState {
        match read_job_state_checked(&self.root, &lane(which)) {
            FileRead::Value(st) => st,
            other => panic!("job state of {which}: {other:?}"),
        }
    }
    fn ops(&self, id: &RepairId) -> Vec<crate::repair::wal::OpRecord> {
        crate::repair::wal::list_in(&self.root, id).0
    }
}

fn record(r: &Rig, id: &RepairId, class: Classification, files: Vec<FileChange>, committed: Vec<(String, String)>, origin: &str) -> RepairRecord {
    std::fs::create_dir_all(ledger::repair_dir_in(&r.root, id)).unwrap();
    let rec = RepairRecord {
        schema: 1,
        id: id.clone(),
        created_at: 1.0,
        trigger: Trigger::OwnerOrder,
        symptom: String::new(),
        cause: String::new(),
        classification: class,
        files,
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
        origin_lane: lane(origin),
        resolves: None,
        committed,
        fixer_seats: Vec::new(),
        checks: Vec::new(),
        reapplies: None,
    };
    ledger::write_record_in(&r.root, &rec).unwrap();
    rec
}

fn kind_of(pre: &TargetState, post: &TargetState) -> ChangeKind {
    match (pre, post) {
        (TargetState::Absent, _) => ChangeKind::Create,
        (_, TargetState::Absent) => ChangeKind::Delete,
        (TargetState::File { sha256: a, .. }, TargetState::File { sha256: b, .. }) if a == b => ChangeKind::Mode,
        _ => ChangeKind::Edit,
    }
}

/// A committed repair made in HQ as T3's commit makes it: the pre-image copied at `begin`, the live
/// edit, the committed copy sealed (HI2), the record in probation, HQ's row verified.
fn committed_in_hq(r: &Rig, id: &RepairId, class: Classification, rel: &str, pre: Option<(&[u8], u32)>, post: Option<(&[u8], u32)>) -> RepairRecord {
    let live = r.hq.join(rel);
    let _ = std::fs::remove_file(&live);
    if let Some((b, m)) = pre {
        r.put(HQ, rel, b, m);
    }
    let pre_img = ledger::copy_preimage_in(&r.root, id, &lane(HQ), rel, &live, T0 - 10.0).unwrap();
    let _ = std::fs::remove_file(&live);
    if let Some((b, m)) = post {
        r.put(HQ, rel, b, m);
    }
    let post_st = TargetState::read(&live).unwrap();
    if matches!(post_st, TargetState::File { .. }) {
        let c = seal_committed_copy_in(&r.root, id, &lane(HQ), rel, &live, T0 - 5.0).unwrap();
        assert_eq!(c.state, post_st);
    }
    let rec = record(
        r,
        id,
        class,
        vec![FileChange { rel: rel.into(), kind: kind_of(&pre_img.state(), &post_st), pre: pre_img, post: post_st.clone(), diff: None }],
        vec![(rel.into(), post_st.canon())],
        HQ,
    );
    let row = LaneRecord {
        schema: 1,
        repair: id.clone(),
        lane: lane(HQ),
        state: LaneState::Verified,
        probation: Some(Probation {
            pack_generation: "0.15.0".into(),
            pro_revision: None,
            applied_at: T0 - 5.0,
            expected: rec.committed.clone(),
            closed: None,
            check_timings: Vec::new(),
            check_timeouts: Vec::new(),
        }),
        merge: None,
        merge_history: Vec::new(),
        approval_ref: None,
        conflict_live: Vec::new(),
        applied: rec.committed.clone(),
        applied_history: Vec::new(),
        merge_base: Vec::new(),
        merge_lock: None,
        merge_attempt: 0,
        updated_at: T0 - 5.0,
    };
    ledger::write_lane_durable_in(&r.root, &row).unwrap();
    rec
}

/// The doubles of one tick.
struct Doubles {
    pass: Cell<bool>,
    checks: Cell<u32>,
    hooks_ok: Cell<bool>,
    hook_calls: Cell<u32>,
    bytes_calls: Cell<u32>,
    class: Cell<Classification>,
    form: Cell<ApplyForm>,
    /// HH1: (rel, content hash) this test's previous-release table knows (`with_prev_vendor`).
    vendor: std::cell::RefCell<Option<(String, String)>>,
}

fn doubles() -> Doubles {
    Doubles {
        pass: Cell::new(true),
        checks: Cell::new(0),
        hooks_ok: Cell::new(true),
        hook_calls: Cell::new(0),
        bytes_calls: Cell::new(0),
        class: Cell::new(Classification::General),
        form: Cell::new(ApplyForm::Exchange),
        vendor: std::cell::RefCell::new(None),
    }
}

/// HH1: the sources the doubles name (the real embedded copies always; the previous-release table
/// replaced by the one entry the test names).
fn sources_of(d: &Doubles) -> crate::pack::UpdateSources {
    match d.vendor.borrow().clone() {
        Some((vrel, h)) => crate::pack::UpdateSources::none().with_prev_vendor(move |rel: &str| if rel == vrel { vec![h.clone()] } else { Vec::new() }),
        None => crate::pack::UpdateSources::none().with_prev_vendor(|_: &str| Vec::new()),
    }
}

fn tick_with(r: &Rig, which: &str, d: &Doubles, fresh: &str, faults: &dyn Faults, stop: &dyn Fn(PassStep) -> bool) -> Result<TickReport, RepairError> {
    let l = lane(which);
    let pack = r.pack(which).to_path_buf();
    let check = |rel: &str, root: &Path, _: Duration| -> Result<CheckOutcome, RepairError> {
        d.checks.set(d.checks.get() + 1);
        // the check reads the file under the root it is given (a copy or the live pack)
        let result = match TargetState::read(&root.join(rel)) {
            Ok(TargetState::File { .. }) if d.pass.get() => CheckResult::Pass,
            Ok(TargetState::File { .. }) => CheckResult::Fail("검사 실패(시험)".into()),
            other => CheckResult::Fail(format!("{rel}: {other:?}")),
        };
        Ok(CheckOutcome { result, ms: 1 })
    };
    let classify = |_: &str| -> Result<Classification, RepairError> { Ok(d.class.get()) };
    let hooks = |_: &RepairId, _: &LaneKey, _: HookStep| -> Result<ReconcileReport, RepairError> {
        d.hook_calls.set(d.hook_calls.get() + 1);
        if d.hooks_ok.get() {
            Ok(ReconcileReport { hooks: Vec::new() })
        } else {
            Err(RepairError::Io("등록기가 실패했습니다(시험)".into()))
        }
    };
    let bytes = |rec: &RepairRecord, rel: &str| -> Result<PathBuf, String> {
        d.bytes_calls.set(d.bytes_calls.get() + 1);
        committed_copy_in(&r.root, rec, rel)
    };
    let lock_file = r.base.join(".pack-apply.lock");
    let sources = sources_of(d);
    let env = LaneEnv {
        op: OpEnv { root: &r.root, pack: &pack, lane: &l, now: T0, form: d.form.get(), faults, own_check: &|_, _| None },
        check: &check,
        classify: &classify,
        locked: &apply::no_repair_lock,
        hooks: &hooks,
        result_bytes: &bytes,
        sources: &sources,
        apply_lock: &lock_file,
        pass_stop: stop,
    };
    propagation_tick_in(&env, Some(fresh))
}

fn tick(r: &Rig, which: &str, d: &Doubles, fresh: &str) -> TickReport {
    tick_with(r, which, d, fresh, &NoFaults, &|_| false).unwrap()
}

fn pass_env<'a>(r: &'a Rig, l: &'a LaneKey, which: &str, stop: &'a dyn Fn(PassStep) -> bool) -> PassEnv<'a> {
    PassEnv { root: &r.root, lane: l, pack: r.pack(which), now: T0, locked: &apply::no_repair_lock, stop }
}

// ───────────────────────────── the gate (I1 / I7 / I8 / P5 / R7) ─────────────────────────────

/// I7 (1): no in-force repair → OPEN, with no job state and no daemon at all.
#[test]
fn gate_no_in_force_repair_is_open() {
    let r = rig("g1");
    assert_eq!(gate_verdict_in(&r.root, &lane(DEPT)), GateVerdict::Open);
    // a repair that is not in force does not hold
    let id = rid("R-1");
    let mut rec = committed_in_hq(&r, &id, Classification::General, "bin/a.py", Some((b"a\n", 0o644)), Some((b"b\n", 0o644)));
    rec.state = RepairState::Reverted;
    ledger::write_record_in(&r.root, &rec).unwrap();
    assert_eq!(gate_verdict_in(&r.root, &lane(DEPT)), GateVerdict::Open);
}

/// I7 (3) / (2), H3 (no time escape): an in-force repair holds a lane until its own tick applied it;
/// then OPEN — read from files only (no daemon is asked); a very old record holds just the same.
#[test]
fn gate_holds_until_the_lane_caught_up_then_opens_without_a_daemon() {
    let r = rig("g2");
    let id = rid("R-1");
    let mut rec = committed_in_hq(&r, &id, Classification::General, "bin/a.py", Some((b"old\n", 0o644)), Some((b"new\n", 0o644)));
    rec.created_at = 0.0; // ten years old: time never opens the gate
    ledger::write_record_in(&r.root, &rec).unwrap();
    r.put(DEPT, "bin/a.py", b"old\n", 0o644);
    assert_eq!(gate_verdict_in(&r.root, &lane(DEPT)), GateVerdict::Held(HoldCause::FirstPassNotDone));
    let d = doubles();
    let t = tick(&r, DEPT, &d, &fid(1));
    assert!(t.catchup_completed, "{t:?}");
    assert_eq!(gate_verdict_in(&r.root, &lane(DEPT)), GateVerdict::Open);
    assert_eq!(r.live(DEPT, "bin/a.py"), r.live(HQ, "bin/a.py"));
    // once OPEN by catch-up, an unreadable record of another repair does not hold it again
    let bad = rid("R-9");
    std::fs::create_dir_all(ledger::repair_dir_in(&r.root, &bad)).unwrap();
    std::fs::write(ledger::record_path_in(&r.root, &bad), b"{ broken").unwrap();
    assert_eq!(gate_verdict_in(&r.root, &lane(DEPT)), GateVerdict::Open);
    std::fs::remove_dir_all(ledger::repair_dir_in(&r.root, &bad)).unwrap();
    // once OPEN by catch-up, a later repair does not hold it again (E④)
    let id2 = rid("R-2");
    committed_in_hq(&r, &id2, Classification::General, "bin/b.py", Some((b"x\n", 0o644)), Some((b"y\n", 0o644)));
    assert_eq!(gate_verdict_in(&r.root, &lane(DEPT)), GateVerdict::Open);
}

/// I7 (4): an unreadable job state (corrupt, a FIFO, oversized) → HELD with that cause, at once.
#[test]
fn gate_unreadable_job_state_holds_with_the_cause() {
    let r = rig("g4");
    let id = rid("R-1");
    committed_in_hq(&r, &id, Classification::General, "bin/a.py", Some((b"old\n", 0o644)), Some((b"new\n", 0o644)));
    let p = job_state_path_in(&r.root, &lane(DEPT));
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, b"{ not json").unwrap();
    assert!(matches!(gate_verdict_in(&r.root, &lane(DEPT)), GateVerdict::Held(HoldCause::StateUnreadable(_))));
    std::fs::write(&p, vec![b' '; (JOB_STATE_MAX_BYTES + 1) as usize]).unwrap();
    assert!(matches!(gate_verdict_in(&r.root, &lane(DEPT)), GateVerdict::Held(HoldCause::StateUnreadable(_))));
    #[cfg(unix)]
    {
        std::fs::remove_file(&p).unwrap();
        let c = std::ffi::CString::new(p.to_str().unwrap()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o644) }, 0);
        // a FIFO with no writer would block a plain open; the bounded reader refuses it at once
        assert!(matches!(gate_verdict_in(&r.root, &lane(DEPT)), GateVerdict::Held(HoldCause::StateUnreadable(_))));
    }
}

/// I8 / interpretation 24 (decision row 6): catch-up recorded but the id file absent or different
/// → HELD "pack instance changed".
#[test]
fn gate_holds_when_the_pack_instance_changed() {
    let r = rig("g5");
    let id = rid("R-1");
    committed_in_hq(&r, &id, Classification::General, "bin/a.py", Some((b"old\n", 0o644)), Some((b"new\n", 0o644)));
    r.put(DEPT, "bin/a.py", b"old\n", 0o644);
    tick(&r, DEPT, &doubles(), &fid(1));
    assert_eq!(gate_verdict_in(&r.root, &lane(DEPT)), GateVerdict::Open);
    let idp = crate::repair::paths::pack_instance_path(&r.dept);
    std::fs::write(&idp, format!("{}\n", fid(9))).unwrap();
    assert_eq!(gate_verdict_in(&r.root, &lane(DEPT)), GateVerdict::Held(HoldCause::PackInstanceChanged));
    std::fs::remove_file(&idp).unwrap();
    assert_eq!(gate_verdict_in(&r.root, &lane(DEPT)), GateVerdict::Held(HoldCause::PackInstanceChanged));
}

/// O1 (c) / P5 / HJ1: a boot-critical repair in a lane that did not make it → 충돌 and the gate's
/// cause reads the contract's sentence verbatim.
#[test]
fn gate_cause_for_an_approval_class_conflict_is_the_p5_sentence() {
    let r = rig("g6");
    let id = rid("R-1");
    committed_in_hq(&r, &id, Classification::BootCritical, "hooks/x.sh", Some((b"#!/bin/sh\nold\n", 0o755)), Some((b"#!/bin/sh\nnew\n", 0o755)));
    r.put(DEPT, "hooks/x.sh", b"#!/bin/sh\nold\n", 0o755);
    tick(&r, DEPT, &doubles(), &fid(1));
    let v = gate_verdict_in(&r.root, &lane(DEPT));
    assert_eq!(v, GateVerdict::Held(HoldCause::AwaitingFinalApproval { rows: 1 }));
    let GateVerdict::Held(c) = v else { unreachable!() };
    assert!(hold_text(&c).contains(AWAITING_FINAL_APPROVAL_EN));
    assert_eq!(
        AWAITING_FINAL_APPROVAL_EN,
        "needs a candidate, both reviewer signatures and the owner's final approval in this department — see `cys repair status`"
    );
}

/// R7: a `resolves` repair made in ANOTHER lane is not in this lane's set — no row, no hold.
#[test]
fn a_resolves_repair_of_another_lane_is_lane_local() {
    let r = rig("g7");
    let id = rid("R-1");
    let mut rec = committed_in_hq(&r, &id, Classification::General, "bin/a.py", Some((b"old\n", 0o644)), Some((b"new\n", 0o644)));
    rec.resolves = Some(rid("R-9"));
    ledger::write_record_in(&r.root, &rec).unwrap();
    r.put(DEPT, "bin/a.py", b"old\n", 0o644);
    assert_eq!(gate_verdict_in(&r.root, &lane(DEPT)), GateVerdict::Open);
    let t = tick(&r, DEPT, &doubles(), &fid(1));
    assert!(t.catchup_completed);
    assert!(r.row(&id, DEPT).is_none(), "no row for a lane-local repair of another lane");
    assert_eq!(r.live(DEPT, "bin/a.py"), TargetState::read(&r.dept.join("bin/a.py")).unwrap());
    assert_eq!(std::fs::read(r.dept.join("bin/a.py")).unwrap(), b"old\n", "never propagated");
}

// ───────────────────────────── the pack-instance pass (I8 / K2 / K7 / L1 / L5 / O4 / O5) ─────────────────────────────

/// Equal → nothing written (the job state's bytes and the id file untouched).
#[test]
fn pass_equal_writes_nothing() {
    let r = rig("p1");
    let l = lane(DEPT);
    let no = |_: PassStep| false;
    assert_eq!(pack_instance_pass_in(&pass_env(&r, &l, DEPT, &no), &fid(1)), PassOutcome::Reset { demoted: 0 });
    let js = std::fs::read(job_state_path_in(&r.root, &l)).unwrap();
    let idp = crate::repair::paths::pack_instance_path(&r.dept);
    let id_bytes = std::fs::read(&idp).unwrap();
    assert_eq!(id_bytes, format!("{}\n", fid(1)).into_bytes());
    let m = std::fs::metadata(&idp).unwrap().modified().unwrap();
    assert_eq!(pack_instance_pass_in(&pass_env(&r, &l, DEPT, &no), &fid(2)), PassOutcome::Equal);
    assert_eq!(std::fs::read(job_state_path_in(&r.root, &l)).unwrap(), js);
    assert_eq!(std::fs::read(&idp).unwrap(), id_bytes);
    assert_eq!(std::fs::metadata(&idp).unwrap().modified().unwrap(), m);
}

fn verified_row(r: &Rig, id: &RepairId, which: &str, applied: Vec<(String, String)>, merge: Option<MergeRecord>, state: LaneState) {
    let mut row = new_row(id, &lane(which), T0);
    row.state = state;
    row.applied = applied;
    row.merge = merge;
    row.merge_lock = Some(crate::repair::lock::new_lease(LockOwner { lane: lane(which), seat: 3, incarnation: "i".into() }, T0, None));
    row.merge_attempt = 2;
    ledger::write_lane_durable_in(&r.root, &row).unwrap();
}

/// A different instance → the FULL reset: catch-up cleared, every in-force row 적용됨 / 검증됨 / 충돌 →
/// 대기 with the merge lock and attempt cleared and the merge record moved to history (K7, L5 (b)),
/// the new id created, then recorded LAST.
#[test]
fn pass_different_instance_resets_rows_and_records_the_new_id() {
    let r = rig("p3");
    let l = lane(DEPT);
    let no = |_: PassStep| false;
    let (a, b, c) = (rid("R-1"), rid("R-2"), rid("R-3"));
    for (id, rel) in [(&a, "bin/a.py"), (&b, "bin/b.py"), (&c, "bin/c.py")] {
        committed_in_hq(&r, id, Classification::General, rel, Some((b"old\n", 0o644)), Some((b"new\n", 0o644)));
    }
    pack_instance_pass_in(&pass_env(&r, &l, DEPT, &no), &fid(1));
    // rows of a lane that caught up, then the pack is replaced (another id)
    let m = MergeRecord { by_seat: 3, result: vec![("bin/a.py".into(), "x".into())], changed: true, note: "n".into(), via_repair: None };
    verified_row(&r, &a, DEPT, vec![("bin/a.py".into(), "x".into())], Some(m.clone()), LaneState::Verified);
    verified_row(&r, &b, DEPT, Vec::new(), None, LaneState::Conflict);
    verified_row(&r, &c, DEPT, Vec::new(), None, LaneState::Applied);
    let mut st = r.job(DEPT);
    st.initial_catchup_complete = true;
    write_job_state(&r.root, &st).unwrap();
    std::fs::write(crate::repair::paths::pack_instance_path(&r.dept), format!("{}\n", fid(7))).unwrap();
    assert_eq!(pack_instance_pass_in(&pass_env(&r, &l, DEPT, &no), &fid(2)), PassOutcome::Reset { demoted: 3 });
    let st = r.job(DEPT);
    assert!(!st.initial_catchup_complete);
    assert_eq!(st.pack_instance.as_deref(), Some(fid(2).as_str()));
    assert_eq!(read_pack_instance(&r.dept), FileRead::Value(fid(2)));
    let ra = r.row(&a, DEPT).unwrap();
    assert_eq!(ra.state, LaneState::Pending);
    assert!(ra.merge.is_none() && ra.merge_history == vec![m], "merge record kept as history");
    assert!(ra.merge_lock.is_none() && ra.merge_attempt == 0, "lock and attempt cleared");
    assert_eq!(ra.applied, vec![("bin/a.py".into(), "x".into())], "recorded results kept");
    assert_eq!(r.row(&b, DEPT).unwrap().state, LaneState::Pending, "K7: a stale 충돌 → 대기");
    assert_eq!(r.row(&c, DEPT).unwrap().state, LaneState::Pending);
}

/// I8 mechanics: a crash after each of the four steps → the next pass repeats; there is never an
/// "equal" with rows left un-demoted.
#[test]
fn pass_crash_after_each_step_repeats_and_never_reports_equal_early() {
    for at in [PassStep::AfterCatchupCleared, PassStep::AfterRowsDemoted, PassStep::AfterIdWritten] {
        let r = rig("p4");
        let l = lane(DEPT);
        let id = rid("R-1");
        committed_in_hq(&r, &id, Classification::General, "bin/a.py", Some((b"old\n", 0o644)), Some((b"new\n", 0o644)));
        let no = |_: PassStep| false;
        pack_instance_pass_in(&pass_env(&r, &l, DEPT, &no), &fid(1));
        verified_row(&r, &id, DEPT, vec![("bin/a.py".into(), "y".into())], None, LaneState::Verified);
        std::fs::remove_file(crate::repair::paths::pack_instance_path(&r.dept)).unwrap();
        let stop = move |s: PassStep| s == at;
        assert!(matches!(pack_instance_pass_in(&pass_env(&r, &l, DEPT, &stop), &fid(2)), PassOutcome::Skipped(w) if w.contains(PASS_STOPPED)));
        // the next pass: never Equal while a row could still be un-demoted
        let next = pack_instance_pass_in(&pass_env(&r, &l, DEPT, &no), &fid(3));
        assert!(matches!(next, PassOutcome::Reset { .. }), "{at:?}: {next:?}");
        assert_eq!(r.row(&id, DEPT).unwrap().state, LaneState::Pending, "{at:?}");
        assert_eq!(r.job(DEPT).pack_instance.as_deref(), Some(fid(3).as_str()));
        assert_eq!(pack_instance_pass_in(&pass_env(&r, &l, DEPT, &no), &fid(4)), PassOutcome::Equal);
    }
}

/// O5: a transient read error of the id file → the pass is SKIPPED, nothing reset.
#[cfg(unix)]
#[test]
fn pass_read_error_skips_without_a_reset() {
    use std::os::unix::fs::PermissionsExt;
    let r = rig("p10");
    let l = lane(DEPT);
    let no = |_: PassStep| false;
    pack_instance_pass_in(&pass_env(&r, &l, DEPT, &no), &fid(1));
    let mut st = r.job(DEPT);
    st.initial_catchup_complete = true;
    write_job_state(&r.root, &st).unwrap();
    let js = std::fs::read(job_state_path_in(&r.root, &l)).unwrap();
    let idp = crate::repair::paths::pack_instance_path(&r.dept);
    std::fs::set_permissions(&idp, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read(&idp).is_ok() {
        return; // running with a privilege that ignores the mode: the case cannot be built here
    }
    assert!(matches!(pack_instance_pass_in(&pass_env(&r, &l, DEPT, &no), &fid(2)), PassOutcome::Skipped(_)));
    assert_eq!(std::fs::read(job_state_path_in(&r.root, &l)).unwrap(), js, "nothing reset");
    std::fs::set_permissions(&idp, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(read_pack_instance(&r.dept), FileRead::Value(fid(1)));
}

/// L1 / L5 (a): the same id in ANOTHER directory (a replacement) → the VERIFY-ONLY pass: an
/// unchanged row stays (no check runs, the lane is not held), a row whose live file differs → 대기.
#[test]
fn pass_directory_change_runs_the_verify_only_pass() {
    let r = rig("p8");
    let (a, b) = (rid("R-1"), rid("R-2"));
    committed_in_hq(&r, &a, Classification::General, "bin/a.py", Some((b"old\n", 0o644)), Some((b"new\n", 0o644)));
    committed_in_hq(&r, &b, Classification::General, "bin/b.py", Some((b"old\n", 0o644)), Some((b"new\n", 0o644)));
    r.put(DEPT, "bin/a.py", b"old\n", 0o644);
    r.put(DEPT, "bin/b.py", b"old\n", 0o644);
    let d = doubles();
    tick(&r, DEPT, &d, &fid(1));
    assert_eq!(r.row(&a, DEPT).unwrap().state, LaneState::Verified);
    assert_eq!(gate_verdict_in(&r.root, &lane(DEPT)), GateVerdict::Open);
    // the department's pack is replaced by a copy (a new directory, the id file copied along) whose
    // b.py lost the repair
    let old = r.base.join("pack-dept-old");
    std::fs::rename(&r.dept, &old).unwrap();
    std::fs::create_dir_all(r.dept.join("bin")).unwrap();
    for f in [".pack-version", ".repair-pack-id", "bin/a.py"] {
        std::fs::copy(old.join(f), r.dept.join(f)).unwrap();
    }
    std::fs::write(r.dept.join("bin/b.py"), b"old\n").unwrap();
    let l = lane(DEPT);
    let no = |_: PassStep| false;
    assert_eq!(pack_instance_pass_in(&pass_env(&r, &l, DEPT, &no), &fid(2)), PassOutcome::VerifyOnly { demoted: 1 });
    assert_eq!(r.row(&a, DEPT).unwrap().state, LaneState::Verified, "unchanged: kept");
    assert_eq!(r.row(&b, DEPT).unwrap().state, LaneState::Pending, "changed: 대기");
    assert!(r.job(DEPT).initial_catchup_complete, "the lane is not held");
    assert_eq!(read_pack_instance(&r.dept), FileRead::Value(fid(1)), "the id is not replaced");
    assert_eq!(pack_instance_pass_in(&pass_env(&r, &l, DEPT, &no), &fid(3)), PassOutcome::Equal, "the new directory is recorded");
}

/// O4: a generation change (an in-place update — department packs and `pack-update` change in place,
/// GH1) → the verify-only pass too.
#[test]
fn pass_generation_change_runs_the_verify_only_pass() {
    let r = rig("p9");
    let a = rid("R-1");
    committed_in_hq(&r, &a, Classification::General, "bin/a.py", Some((b"old\n", 0o644)), Some((b"new\n", 0o644)));
    r.put(DEPT, "bin/a.py", b"old\n", 0o644);
    tick(&r, DEPT, &doubles(), &fid(1));
    std::fs::write(r.dept.join(".pack-version"), "0.15.1\n").unwrap();
    r.put(DEPT, "bin/a.py", b"vendor\n", 0o644);
    let l = lane(DEPT);
    let no = |_: PassStep| false;
    assert_eq!(pack_instance_pass_in(&pass_env(&r, &l, DEPT, &no), &fid(2)), PassOutcome::VerifyOnly { demoted: 1 });
    assert_eq!(r.row(&a, DEPT).unwrap().state, LaneState::Pending);
}

/// I8: the id file is no boot-critical file, and only the repair code names it (source census).
#[test]
fn the_id_file_is_not_boot_critical_and_only_repair_code_names_it() {
    let p = std::path::PathBuf::from("/x/pack").join(crate::repair::paths::PACK_INSTANCE_FILE);
    assert_ne!(crate::repair::boot_critical::is_boot_critical(&p).ok(), Some(crate::repair::boot_critical::Decision::Yes));
    assert!(crate::repair::boot_critical::boot_critical_entry(".repair-pack-id").is_none());
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut hits = Vec::new();
    let mut stack = vec![src.clone()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "rs") && std::fs::read_to_string(&p).unwrap_or_default().contains(".repair-pack-id") {
                hits.push(p.strip_prefix(&src).unwrap().display().to_string());
            }
        }
    }
    for h in &hits {
        assert!(h.starts_with("repair/") || h.starts_with("bin/cysd/repair"), "{h} names the id file");
    }
}

// ───────────────────────────── each lane's rows and the general forms ─────────────────────────────

/// K1 / 11.2 / C11 11.4: a department created after a commit gets the repair at its first tick —
/// its OWN row (no other lane's file written), the edit made by T6's engine (an operation of kind
/// `propagate`), the lane's own check, `검증됨` with the probation bundle; the catch-up recorded.
#[test]
fn a_new_department_receives_an_edit_through_the_engine() {
    for form in [ApplyForm::Exchange, ApplyForm::DepositFirst] {
        let r = rig("r1");
        let id = rid("R-1");
        committed_in_hq(&r, &id, Classification::General, "bin/a.py", Some((b"old\n", 0o644)), Some((b"new\n", 0o755)));
        let hq_row = std::fs::read(ledger::lane_path_in(&r.root, &id, &lane(HQ))).unwrap();
        r.put(DEPT, "bin/a.py", b"old\n", 0o644);
        let d = doubles();
        d.form.set(form);
        let t = tick(&r, DEPT, &d, &fid(1));
        assert!(t.catchup_completed, "{form:?}: {t:?}");
        assert_eq!(r.live(DEPT, "bin/a.py"), r.live(HQ, "bin/a.py"), "{form:?}");
        let row = r.row(&id, DEPT).unwrap();
        assert_eq!(row.state, LaneState::Verified);
        assert!(row.probation.as_ref().is_some_and(|p| p.expected == row.applied && p.closed.is_none()));
        assert!(r.ops(&id).iter().any(|o| o.kind == "propagate" && o.lane_key == lane(DEPT) && o.outcome.as_deref() == Some("applied")), "{form:?}");
        assert_eq!(std::fs::read(ledger::lane_path_in(&r.root, &id, &lane(HQ))).unwrap(), hq_row, "HQ's row untouched (K1)");
        assert_eq!(d.bytes_calls.get(), 1, "the bytes came from the committed copy");
        assert!(d.checks.get() >= 2, "copy-check and the live check");
    }
}

/// M3 SA:134: a failing copy-check leaves the lane untouched (bytes and mtime), the row 충돌, no
/// operation recorded; and it is never retried by itself (H8).
#[test]
fn a_failing_copy_check_leaves_the_lane_untouched() {
    let r = rig("a1");
    let id = rid("R-1");
    committed_in_hq(&r, &id, Classification::General, "bin/a.py", Some((b"old\n", 0o644)), Some((b"new\n", 0o644)));
    r.put(DEPT, "bin/a.py", b"old\n", 0o644);
    let m0 = r.mtime(DEPT, "bin/a.py");
    let d = doubles();
    d.pass.set(false);
    let t = tick(&r, DEPT, &d, &fid(1));
    assert!(!t.catchup_completed);
    assert_eq!(std::fs::read(r.dept.join("bin/a.py")).unwrap(), b"old\n");
    assert_eq!(r.mtime(DEPT, "bin/a.py"), m0);
    assert_eq!(r.row(&id, DEPT).unwrap().state, LaneState::Conflict);
    assert!(r.ops(&id).iter().all(|o| o.lane_key != lane(DEPT)), "nothing ran on the live file");
    let n = d.checks.get();
    d.pass.set(true);
    tick(&r, DEPT, &d, &fid(2));
    assert_eq!(r.row(&id, DEPT).unwrap().state, LaneState::Conflict, "a general 충돌 ends only through recheck");
    assert_eq!(d.checks.get(), n, "no check on a standing conflict");
    assert!(matches!(gate_verdict_in(&r.root, &lane(DEPT)), GateVerdict::Held(HoldCause::Pending { conflict: 1, .. })));
}

/// I7 (4) / J2: "already there" writes nothing (bytes and mtime) and becomes 검증됨 only after the
/// lane's own check; a failing check → 충돌 and the new lane stays HELD.
#[test]
fn already_there_never_skips_the_check() {
    let r = rig("a16");
    let id = rid("R-1");
    committed_in_hq(&r, &id, Classification::General, "bin/a.py", Some((b"old\n", 0o644)), Some((b"new\n", 0o644)));
    r.put(DEPT, "bin/a.py", b"new\n", 0o644);
    let m0 = r.mtime(DEPT, "bin/a.py");
    let d = doubles();
    d.pass.set(false);
    tick(&r, DEPT, &d, &fid(1));
    assert_eq!(r.row(&id, DEPT).unwrap().state, LaneState::Conflict);
    assert!(matches!(gate_verdict_in(&r.root, &lane(DEPT)), GateVerdict::Held(_)));
    let r2 = rig("a16b");
    committed_in_hq(&r2, &id, Classification::General, "bin/a.py", Some((b"old\n", 0o644)), Some((b"new\n", 0o644)));
    r2.put(DEPT, "bin/a.py", b"new\n", 0o644);
    let d2 = doubles();
    tick(&r2, DEPT, &d2, &fid(1));
    assert_eq!(r2.row(&id, DEPT).unwrap().state, LaneState::Verified);
    assert_eq!(d2.checks.get(), 1, "the lane's own check ran");
    assert_eq!(d2.bytes_calls.get(), 0, "no bytes needed");
    assert!(r2.ops(&id).iter().all(|o| o.lane_key != lane(DEPT)), "no write");
    let _ = m0;
}

/// 11.3 / E7: a creation is placed where the lane has nothing; anything there → 충돌, unchanged.
#[test]
fn creation_is_placed_only_where_nothing_is() {
    let r = rig("a3");
    let id = rid("R-1");
    committed_in_hq(&r, &id, Classification::General, "bin/new.py", None, Some((b"print(1)\n", 0o644)));
    tick(&r, DEPT, &doubles(), &fid(1));
    assert_eq!(r.live(DEPT, "bin/new.py"), r.live(HQ, "bin/new.py"));
    assert_eq!(r.row(&id, DEPT).unwrap().state, LaneState::Verified);
    let r2 = rig("a3b");
    committed_in_hq(&r2, &id, Classification::General, "bin/new.py", None, Some((b"print(1)\n", 0o644)));
    r2.put(DEPT, "bin/new.py", b"mine\n", 0o644);
    tick(&r2, DEPT, &doubles(), &fid(1));
    assert_eq!(std::fs::read(r2.dept.join("bin/new.py")).unwrap(), b"mine\n");
    assert_eq!(r2.row(&id, DEPT).unwrap().state, LaneState::Conflict);
}

/// 11.3 / interpretation 44 / B1: a deletion where the lane's STATE is the origin's pre-image →
/// moved into a fresh vault slot, tombstoned in THIS lane's file; a mode difference → 충돌.
#[test]
fn deletion_only_at_the_pre_image_state_and_tombstoned_here() {
    let r = rig("a4");
    let id = rid("R-1");
    committed_in_hq(&r, &id, Classification::General, "bin/gone.py", Some((b"x\n", 0o644)), None);
    r.put(DEPT, "bin/gone.py", b"x\n", 0o644);
    tick(&r, DEPT, &doubles(), &fid(1));
    assert_eq!(r.live(DEPT, "bin/gone.py"), TargetState::Absent);
    assert_eq!(r.row(&id, DEPT).unwrap().state, LaneState::Verified);
    let t = crate::repair::tombstones::load_in(&r.root, &lane(DEPT)).unwrap();
    assert!(t.iter().any(|t| t.rel == "bin/gone.py" && t.repair == id), "{t:?}");
    assert!(crate::repair::tombstones::load_in(&r.root, &lane(HQ)).unwrap().is_empty(), "never another lane's file");
    #[cfg(unix)]
    {
        let r2 = rig("a4b");
        committed_in_hq(&r2, &id, Classification::General, "bin/gone.py", Some((b"x\n", 0o644)), None);
        r2.put(DEPT, "bin/gone.py", b"x\n", 0o600);
        tick(&r2, DEPT, &doubles(), &fid(1));
        assert!(matches!(r2.live(DEPT, "bin/gone.py"), TargetState::File { .. }), "a mode difference is not deleted");
        assert_eq!(r2.row(&id, DEPT).unwrap().state, LaneState::Conflict);
    }
}

/// Whether any vault slot of `id` holds a file with exactly `bytes` (HH1's "moved into a fresh slot").
fn a_slot_holds(r: &Rig, id: &RepairId, bytes: &[u8]) -> bool {
    fn walk(d: &Path, bytes: &[u8]) -> bool {
        std::fs::read_dir(d).into_iter().flatten().flatten().any(|e| {
            let p = e.path();
            match std::fs::symlink_metadata(&p) {
                Ok(m) if m.is_dir() => walk(&p, bytes),
                Ok(m) if m.is_file() => std::fs::read(&p).is_ok_and(|b| b == bytes),
                _ => false,
            }
        })
    }
    ledger::list_slots_in(&r.root, id).iter().any(|s| walk(&s.dir, bytes))
}

/// RULINGS HH1 (C11 11.3, design 4-14 "알려진 원본 판과 바이트까지 같을 때만"): a deletion where this
/// lane's bytes are not the origin's pre-image but ARE a known vendor version (`Recorded::released` —
/// `pack::released_bytes`) → the file is moved into a fresh vault slot, tombstoned in THIS lane's
/// file, the row 검증됨.
#[test]
fn a_deletion_of_known_vendor_bytes_moves_them_to_a_fresh_slot_and_tombstones_here() {
    let r = rig("hh1a");
    let id = rid("R-1");
    committed_in_hq(&r, &id, Classification::General, "bin/gone.py", Some((b"origin's own\n", 0o644)), None);
    r.put(DEPT, "bin/gone.py", b"vendor v1\n", 0o644);
    let d = doubles();
    *d.vendor.borrow_mut() = Some(("bin/gone.py".into(), crate::pack::content_hash_pub("vendor v1\n")));
    tick(&r, DEPT, &d, &fid(1));
    assert_eq!(r.live(DEPT, "bin/gone.py"), TargetState::Absent);
    assert_eq!(r.row(&id, DEPT).unwrap().state, LaneState::Verified);
    assert!(a_slot_holds(&r, &id, b"vendor v1\n"), "the vendor bytes are kept in a fresh slot");
    let t = crate::repair::tombstones::load_in(&r.root, &lane(DEPT)).unwrap();
    assert!(t.iter().any(|t| t.rel == "bin/gone.py" && t.repair == id), "{t:?}");
    assert!(crate::repair::tombstones::load_in(&r.root, &lane(HQ)).unwrap().is_empty(), "never another lane's file");
}

/// HH1: one byte different from every known vendor version (and from the pre-image) → 충돌; nothing
/// moved, no tombstone.
#[test]
fn a_deletion_of_bytes_one_byte_off_vendor_is_a_conflict_and_nothing_moves() {
    let r = rig("hh1b");
    let id = rid("R-1");
    committed_in_hq(&r, &id, Classification::General, "bin/gone.py", Some((b"origin's own\n", 0o644)), None);
    r.put(DEPT, "bin/gone.py", b"vendor v2\n", 0o644);
    let d = doubles();
    *d.vendor.borrow_mut() = Some(("bin/gone.py".into(), crate::pack::content_hash_pub("vendor v1\n")));
    tick(&r, DEPT, &d, &fid(1));
    assert_eq!(std::fs::read(r.dept.join("bin/gone.py")).unwrap(), b"vendor v2\n");
    assert_eq!(r.row(&id, DEPT).unwrap().state, LaneState::Conflict);
    assert!(!a_slot_holds(&r, &id, b"vendor v2\n"), "nothing moved");
    assert!(crate::repair::tombstones::load_in(&r.root, &lane(DEPT)).unwrap().is_empty());
}

/// HH1: the vendor file replaced by another writer between `Recorded::released`'s read and the act →
/// the door refuses (GK2 / GV1: 충돌), the other writer's bytes stay, no tombstone.
#[test]
fn a_vendor_file_replaced_before_the_act_is_refused_by_the_door() {
    struct Replace;
    impl Faults for Replace {
        fn at(&self, p: Point, live: &Path, _: Option<&Slot>) -> bool {
            if p == Point::AfterRecord {
                std::fs::write(live, b"someone else\n").unwrap();
            }
            false
        }
    }
    let r = rig("hh1c");
    let id = rid("R-1");
    committed_in_hq(&r, &id, Classification::General, "bin/gone.py", Some((b"origin's own\n", 0o644)), None);
    r.put(DEPT, "bin/gone.py", b"vendor v1\n", 0o644);
    let d = doubles();
    *d.vendor.borrow_mut() = Some(("bin/gone.py".into(), crate::pack::content_hash_pub("vendor v1\n")));
    tick_with(&r, DEPT, &d, &fid(1), &Replace, &|_| false).unwrap();
    assert_eq!(std::fs::read(r.dept.join("bin/gone.py")).unwrap(), b"someone else\n");
    assert_eq!(r.row(&id, DEPT).unwrap().state, LaneState::Conflict);
    assert!(crate::repair::tombstones::load_in(&r.root, &lane(DEPT)).unwrap().is_empty());
}

/// RULINGS HI2 / P6: the copy sealed at the commit is a slot an in-force repair still depends on (T3's
/// `cleanup` never removes it while R is in force); a copy of OTHER bytes (a refused commit's) is not
/// needed; once R is out of force the copy is not kept for it any more.
#[test]
fn the_committed_copy_is_kept_while_the_repair_is_in_force() {
    let r = rig("p6");
    let id = rid("R-1");
    committed_in_hq(&r, &id, Classification::General, "bin/a.py", Some((b"old\n", 0o644)), Some((b"new\n", 0o644)));
    let copy = committed_copy_in(&r.root, &ledger::read_record_in(&r.root, &id).unwrap().unwrap(), "bin/a.py").unwrap();
    let copy_slot = copy.parent().unwrap().parent().unwrap().to_path_buf();
    r.put(HQ, "bin/a.py", b"other\n", 0o644);
    let other = seal_committed_copy_in(&r.root, &id, &lane(HQ), "bin/a.py", &r.hq.join("bin/a.py"), T0).unwrap();
    let needed = apply::slots_needed_in_force_in(&r.root, &id).unwrap();
    assert!(needed.contains(&copy_slot), "{needed:?}");
    assert!(!needed.contains(&other.slot), "a copy of other bytes is not needed");
    let mut rec = ledger::read_record_in(&r.root, &id).unwrap().unwrap();
    rec.state = RepairState::Reverted;
    ledger::write_record_durable_in(&r.root, &rec).unwrap();
    assert!(!apply::slots_needed_in_force_in(&r.root, &id).unwrap().contains(&copy_slot), "out of force");
}

/// HI2 / HJ2: a committed file target without a readable, matching committed copy → 충돌 with the true
/// reason; nothing live changed.
#[test]
fn no_committed_copy_is_a_conflict_with_the_true_reason() {
    let r = rig("a14");
    let id = rid("R-1");
    committed_in_hq(&r, &id, Classification::General, "bin/a.py", Some((b"old\n", 0o644)), Some((b"new\n", 0o644)));
    // damage every committed copy's bytes
    for s in ledger::list_slots_in(&r.root, &id) {
        let meta = std::fs::read_to_string(s.dir.join("meta/pre.json")).unwrap();
        if meta.contains(COMMITTED_COPY_KIND) {
            let p = s.dir.join("payload/a.py");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
            }
            std::fs::write(&p, b"tampered\n").unwrap();
        }
    }
    r.put(DEPT, "bin/a.py", b"old\n", 0o644);
    let t = tick(&r, DEPT, &doubles(), &fid(1));
    assert_eq!(std::fs::read(r.dept.join("bin/a.py")).unwrap(), b"old\n");
    let row = r.row(&id, DEPT).unwrap();
    assert_eq!(row.state, LaneState::Conflict);
    assert!(t.rows.iter().any(|(_, _, why)| why.contains("결과 사본")), "{t:?}");
}

/// A mode-only change needs no foreign bytes: the lane's own bytes with the result's mode.
#[cfg(unix)]
#[test]
fn a_mode_only_change_uses_the_lanes_own_bytes() {
    let r = rig("a15");
    let id = rid("R-1");
    committed_in_hq(&r, &id, Classification::General, "bin/run.sh", Some((b"#!/bin/sh\n", 0o644)), Some((b"#!/bin/sh\n", 0o755)));
    r.put(DEPT, "bin/run.sh", b"#!/bin/sh\n", 0o644);
    let d = doubles();
    tick(&r, DEPT, &d, &fid(1));
    assert_eq!(r.live(DEPT, "bin/run.sh"), r.live(HQ, "bin/run.sh"));
    assert_eq!(d.bytes_calls.get(), 0);
}

/// P2: a re-committed repair reaches a verified lane again — its previous applied result is a valid
/// pre-image; a lane that holds a MERGE of the old result → 충돌 (interpretation 41).
#[test]
fn a_re_commit_reaches_the_lane_again_and_a_merge_lane_conflicts() {
    let r = rig("r7");
    let id = rid("R-1");
    committed_in_hq(&r, &id, Classification::General, "bin/a.py", Some((b"v0\n", 0o644)), Some((b"v1\n", 0o644)));
    r.put(DEPT, "bin/a.py", b"v0\n", 0o644);
    tick(&r, DEPT, &doubles(), &fid(1));
    assert_eq!(std::fs::read(r.dept.join("bin/a.py")).unwrap(), b"v1\n");
    // round 2 committed at the origin (the audit reopened it; T3 re-commits)
    r.put(HQ, "bin/a.py", b"v2\n", 0o644);
    let c = seal_committed_copy_in(&r.root, &id, &lane(HQ), "bin/a.py", &r.hq.join("bin/a.py"), T0).unwrap();
    let mut rec = ledger::read_record_in(&r.root, &id).unwrap().unwrap();
    rec.committed = vec![("bin/a.py".into(), c.state.canon())];
    ledger::write_record_in(&r.root, &rec).unwrap();
    tick(&r, DEPT, &doubles(), &fid(2));
    assert_eq!(std::fs::read(r.dept.join("bin/a.py")).unwrap(), b"v2\n");
    let row = r.row(&id, DEPT).unwrap();
    assert_eq!(row.state, LaneState::Verified);
    assert_eq!(row.applied, rec.committed);
    assert_eq!(row.applied_history.len(), 1, "round 1 kept as history");
    // a merge lane: its row's applied (round 2) differs after round 3; its bytes are its own merge
    let mut row = r.row(&id, DEPT).unwrap();
    row.merge = Some(MergeRecord { by_seat: 3, result: vec![("bin/a.py".into(), r.live(DEPT, "bin/a.py").canon())], changed: false, note: String::new(), via_repair: None });
    ledger::write_lane_durable_in(&r.root, &row).unwrap();
    r.put(HQ, "bin/a.py", b"v3\n", 0o644);
    let c = seal_committed_copy_in(&r.root, &id, &lane(HQ), "bin/a.py", &r.hq.join("bin/a.py"), T0 + 1.0).unwrap();
    rec.committed = vec![("bin/a.py".into(), c.state.canon())];
    ledger::write_record_in(&r.root, &rec).unwrap();
    tick(&r, DEPT, &doubles(), &fid(3));
    assert_eq!(r.row(&id, DEPT).unwrap().state, LaneState::Conflict, "a merge's bytes are not a pre-image");
    assert_eq!(std::fs::read(r.dept.join("bin/a.py")).unwrap(), b"v2\n");
}

/// A propagated apply that T6's probation returned in THIS lane (`되돌려짐 — 데몬(자동)`) while R is still
/// in force is no longer 검증됨: the row becomes 충돌 for the lane's worker.
#[test]
fn a_lane_return_by_the_probation_makes_the_row_a_conflict() {
    let r = rig("r12");
    let id = rid("R-1");
    committed_in_hq(&r, &id, Classification::General, "bin/a.py", Some((b"old\n", 0o644)), Some((b"new\n", 0o644)));
    r.put(DEPT, "bin/a.py", b"old\n", 0o644);
    tick(&r, DEPT, &doubles(), &fid(1));
    let mut row = r.row(&id, DEPT).unwrap();
    assert_eq!(row.state, LaneState::Verified);
    row.probation.as_mut().unwrap().closed = Some(format!("{} — {}", apply::CLOSED_REVERTED, apply::BY_DAEMON_AUTO));
    ledger::write_lane_durable_in(&r.root, &row).unwrap();
    r.put(DEPT, "bin/a.py", b"old\n", 0o644);
    tick(&r, DEPT, &doubles(), &fid(2));
    assert_eq!(r.row(&id, DEPT).unwrap().state, LaneState::Conflict);
}

/// Interpretation 42 (R7): a merge record pointing to a repair no longer in force → 대기 at the next
/// tick, and ordinary propagation decides.
#[test]
fn a_merge_via_a_repair_out_of_force_goes_back_to_pending() {
    let r = rig("r11");
    let id = rid("R-1");
    committed_in_hq(&r, &id, Classification::General, "bin/a.py", Some((b"old\n", 0o644)), Some((b"new\n", 0o644)));
    let r2 = rid("R-2");
    let mut gone = committed_in_hq(&r, &r2, Classification::General, "bin/z.py", Some((b"1\n", 0o644)), Some((b"2\n", 0o644)));
    gone.state = RepairState::Reverted;
    ledger::write_record_in(&r.root, &gone).unwrap();
    r.put(DEPT, "bin/a.py", b"mine\n", 0o644);
    let rec = ledger::read_record_in(&r.root, &id).unwrap().unwrap();
    let mut row = new_row(&id, &lane(DEPT), T0);
    row.state = LaneState::Verified;
    row.applied = rec.committed.clone();
    row.merge = Some(MergeRecord { by_seat: 0, result: vec![("bin/a.py".into(), r.live(DEPT, "bin/a.py").canon())], changed: true, note: String::new(), via_repair: Some(r2.clone()) });
    ledger::write_lane_durable_in(&r.root, &row).unwrap();
    tick(&r, DEPT, &doubles(), &fid(1));
    let row = r.row(&id, DEPT).unwrap();
    assert_ne!(row.state, LaneState::Verified, "it left 검증됨");
    assert!(row.merge.is_none() && row.merge_history.len() == 1);
}

/// GV1: another writer's bytes at the door → 충돌 (never "superseded"), the foreign bytes kept.
#[test]
fn foreign_bytes_at_the_door_are_a_conflict_and_stay() {
    struct Foreign;
    impl Faults for Foreign {
        fn at(&self, p: Point, live: &Path, _: Option<&Slot>) -> bool {
            if p == Point::AfterRecord {
                std::fs::write(live, b"foreign\n").unwrap();
            }
            false
        }
    }
    let r = rig("a7");
    let id = rid("R-1");
    committed_in_hq(&r, &id, Classification::General, "bin/a.py", Some((b"old\n", 0o644)), Some((b"new\n", 0o644)));
    r.put(DEPT, "bin/a.py", b"old\n", 0o644);
    tick_with(&r, DEPT, &doubles(), &fid(1), &Foreign, &|_| false).unwrap();
    assert_eq!(std::fs::read(r.dept.join("bin/a.py")).unwrap(), b"foreign\n");
    assert_eq!(r.row(&id, DEPT).unwrap().state, LaneState::Conflict);
}

/// EJ1: the pack generation moved during the apply → nothing placed, the row stays 대기 (decided
/// again at the next tick).
#[test]
fn a_generation_change_during_the_apply_leaves_the_row_pending() {
    struct Bump;
    impl Faults for Bump {
        fn at(&self, p: Point, live: &Path, _: Option<&Slot>) -> bool {
            if p == Point::AfterRecord {
                let pack = live.parent().unwrap().parent().unwrap();
                std::fs::write(pack.join(".pack-version"), "0.15.9\n").unwrap();
            }
            false
        }
    }
    let r = rig("a8");
    let id = rid("R-1");
    committed_in_hq(&r, &id, Classification::General, "bin/a.py", Some((b"old\n", 0o644)), Some((b"new\n", 0o644)));
    r.put(DEPT, "bin/a.py", b"old\n", 0o644);
    let t = tick_with(&r, DEPT, &doubles(), &fid(1), &Bump, &|_| false).unwrap();
    assert_eq!(std::fs::read(r.dept.join("bin/a.py")).unwrap(), b"old\n");
    assert_eq!(r.row(&id, DEPT).unwrap().state, LaneState::Pending, "{t:?}");
    assert!(!t.catchup_completed);
}

/// CB1 / CG / T8b: a lane's promoted (composed) MASTER directive is never replaced by propagation.
#[test]
fn a_promoted_master_directive_is_never_replaced() {
    let r = rig("a10");
    let id = rid("R-1");
    let rel = "directives/MASTER_DIRECTIVE.md";
    committed_in_hq(&r, &id, Classification::General, rel, Some((b"# M\n", 0o644)), Some((b"# M2\n", 0o644)));
    r.put(DEPT, rel, b"# M\n", 0o644);
    std::fs::write(r.dept.join("directives/MASTER_DIRECTIVE.md.pre-ceo"), b"# M\n").unwrap();
    assert!(crate::pack::ceo_promoted_in(&r.dept));
    tick(&r, DEPT, &doubles(), &fid(1));
    assert_eq!(std::fs::read(r.dept.join(rel)).unwrap(), b"# M\n");
    assert_eq!(r.row(&id, DEPT).unwrap().state, LaneState::Conflict);
}

/// A-2 #11 / F3: a department schedule that is not HQ's pre-image is never replaced by HQ's.
#[test]
fn a_department_schedule_is_never_overwritten_by_hqs() {
    let r = rig("a11");
    let id = rid("R-1");
    committed_in_hq(&r, &id, Classification::General, "schedule.json", Some((br#"{"jobs":[]}"#, 0o644)), Some((br#"{"jobs":[{"id":"x"}]}"#, 0o644)));
    r.put(DEPT, "schedule.json", br#"{"jobs":[{"id":"dept-self-heal"}]}"#, 0o644);
    tick(&r, DEPT, &doubles(), &fid(1));
    assert_eq!(std::fs::read(r.dept.join("schedule.json")).unwrap(), br#"{"jobs":[{"id":"dept-self-heal"}]}"#);
    assert_eq!(r.row(&id, DEPT).unwrap().state, LaneState::Conflict);
}

// ───────────────────────────── RULINGS HJ1 — the rule of the core ─────────────────────────────

/// HJ1 / O1 (c): a boot-critical or protection-related repair in a lane that did not make it is
/// never written by the daemon — at R's pre-image AND at R's result, on both forms: 충돌 with the
/// O1 (c) cause; nothing in the lane's pack changes, no committed / approved copy is read.
#[test]
fn an_approval_class_repair_is_never_applied_in_a_receiving_lane() {
    for (form, class, at_result) in [
        (ApplyForm::Exchange, Classification::BootCritical, false),
        (ApplyForm::DepositFirst, Classification::BootCritical, false),
        (ApplyForm::Exchange, Classification::ProtectionRelated, false),
        (ApplyForm::Exchange, Classification::BootCritical, true),
    ] {
        let r = rig("hj1");
        let id = rid("R-1");
        committed_in_hq(&r, &id, class, "hooks/x.sh", Some((b"#!/bin/sh\nold\n", 0o755)), Some((b"#!/bin/sh\nnew\n", 0o755)));
        let content: &[u8] = if at_result { b"#!/bin/sh\nnew\n" } else { b"#!/bin/sh\nold\n" };
        r.put(DEPT, "hooks/x.sh", content, 0o755);
        let m0 = r.mtime(DEPT, "hooks/x.sh");
        let d = doubles();
        d.form.set(form);
        let t = tick(&r, DEPT, &d, &fid(1));
        assert_eq!(std::fs::read(r.dept.join("hooks/x.sh")).unwrap(), content, "{form:?} {class:?}");
        assert_eq!(r.mtime(DEPT, "hooks/x.sh"), m0);
        let row = r.row(&id, DEPT).unwrap();
        assert_eq!(row.state, LaneState::Conflict, "{form:?} {class:?} {at_result}");
        assert!(t.rows.iter().any(|(_, _, why)| why.contains("--resolves R-1") && why.contains(AWAITING_FINAL_APPROVAL_EN)), "{t:?}");
        assert_eq!(d.bytes_calls.get(), 0, "no copy read");
        assert_eq!(d.checks.get(), 0, "no check without an approved copy here");
        assert!(r.ops(&id).iter().all(|o| o.lane_key != lane(DEPT)), "no operation in the lane");
        assert_eq!(gate_verdict_in(&r.root, &lane(DEPT)), GateVerdict::Held(HoldCause::AwaitingFinalApproval { rows: 1 }));
        // re-evaluated only when the live file changes: no loop, no check
        tick(&r, DEPT, &d, &fid(2));
        assert_eq!(d.checks.get(), 0);
    }
}

/// HJ1: the classification of a TARGET in this lane counts too (a general record whose file is
/// listed here).
#[test]
fn a_listed_target_in_this_lane_is_approval_class() {
    let r = rig("hj1b");
    let id = rid("R-1");
    committed_in_hq(&r, &id, Classification::General, "bin/a.py", Some((b"old\n", 0o644)), Some((b"new\n", 0o644)));
    r.put(DEPT, "bin/a.py", b"old\n", 0o644);
    let d = doubles();
    d.class.set(Classification::ProtectionRelated);
    tick(&r, DEPT, &d, &fid(1));
    assert_eq!(std::fs::read(r.dept.join("bin/a.py")).unwrap(), b"old\n");
    assert_eq!(r.row(&id, DEPT).unwrap().state, LaneState::Conflict);
}

// ───────────────────────────── hook lines (X1 / Y1 / Y2) ─────────────────────────────

fn hook_repair(r: &Rig, id: &RepairId) -> RepairRecord {
    record(
        r,
        id,
        Classification::General,
        vec![FileChange {
            rel: "settings:Stop:x.sh".into(),
            kind: ChangeKind::HookUnregister { event: "Stop".into(), script: "x.sh".into() },
            pre: PreImage::Absent,
            post: TargetState::Absent,
            diff: None,
        }],
        Vec::new(),
        HQ,
    )
}

/// X1 / Y1 / Y2 (c): a hook unregistration reaches the lane through the lane's own step; Ok →
/// 검증됨; the step's error → 충돌.
#[test]
fn a_hook_unregistration_runs_the_lanes_own_step() {
    let r = rig("h1");
    let id = rid("R-1");
    hook_repair(&r, &id);
    let d = doubles();
    tick(&r, DEPT, &d, &fid(1));
    assert_eq!(d.hook_calls.get(), 1);
    assert_eq!(r.row(&id, DEPT).unwrap().state, LaneState::Verified);
    let r2 = rig("h1b");
    hook_repair(&r2, &id);
    let d2 = doubles();
    d2.hooks_ok.set(false);
    tick(&r2, DEPT, &d2, &fid(1));
    assert_eq!(r2.row(&id, DEPT).unwrap().state, LaneState::Conflict);
}

// ───────────────────────────── recheck (H8) ─────────────────────────────

fn recheck(r: &Rig, id: &RepairId, d: &Doubles, seat: u64, note: &str) -> Result<LaneState, RepairError> {
    let l = lane(DEPT);
    let check = |rel: &str, root: &Path, _: Duration| -> Result<CheckOutcome, RepairError> {
        d.checks.set(d.checks.get() + 1);
        let ok = matches!(TargetState::read(&root.join(rel)), Ok(TargetState::File { .. })) && d.pass.get();
        Ok(CheckOutcome { result: if ok { CheckResult::Pass } else { CheckResult::Fail("x".into()) }, ms: 1 })
    };
    let classify = |_: &str| -> Result<Classification, RepairError> { Ok(d.class.get()) };
    let hooks = |_: &RepairId, _: &LaneKey, _: HookStep| -> Result<ReconcileReport, RepairError> { Ok(ReconcileReport { hooks: Vec::new() }) };
    let bytes = |_: &RepairRecord, _: &str| -> Result<PathBuf, String> { Err("none".into()) };
    let lock_file = r.base.join(".pack-apply.lock");
    let sources = sources_of(d);
    let env = LaneEnv {
        op: OpEnv { root: &r.root, pack: &r.dept, lane: &l, now: T0, form: ApplyForm::Exchange, faults: &NoFaults, own_check: &|_, _| None },
        check: &check,
        classify: &classify,
        locked: &apply::no_repair_lock,
        hooks: &hooks,
        result_bytes: &bytes,
        sources: &sources,
        apply_lock: &lock_file,
        pass_stop: &|_| false,
    };
    let owner = LockOwner { lane: l.clone(), seat, incarnation: "inc".into() };
    recheck_lane_in(&env, id, seat, &owner, "inc", note)
}

fn conflicted_with_lock(r: &Rig, id: &RepairId, base: &[u8]) {
    let mut row = r.row(id, DEPT).unwrap();
    assert_eq!(row.state, LaneState::Conflict);
    row.merge_lock = Some(crate::repair::lock::new_lease(LockOwner { lane: lane(DEPT), seat: 5, incarnation: "inc".into() }, T0, None));
    row.merge_base = vec![("bin/a.py".into(), {
        let p = r.base.join("mb");
        std::fs::write(&p, base).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
        }
        TargetState::read(&p).unwrap().canon()
    })];
    ledger::write_lane_durable_in(&r.root, &row).unwrap();
}

/// H8: an unchanged merge → 검증됨 with `changed: false`; a changed merge → `changed: true`; the
/// merge record names the seat and the live result (V2 (c)); the lock is released.
#[test]
fn recheck_records_an_unchanged_and_a_changed_merge() {
    for changed in [false, true] {
        let r = rig("m1");
        let id = rid("R-1");
        committed_in_hq(&r, &id, Classification::General, "bin/a.py", Some((b"old\n", 0o644)), Some((b"new\n", 0o644)));
        r.put(DEPT, "bin/a.py", b"mine\n", 0o644);
        tick(&r, DEPT, &doubles(), &fid(1));
        conflicted_with_lock(&r, &id, b"mine\n");
        if changed {
            r.put(DEPT, "bin/a.py", b"mine+new\n", 0o644);
        }
        let d = doubles();
        assert_eq!(recheck(&r, &id, &d, 5, "합침").unwrap(), LaneState::Verified);
        let row = r.row(&id, DEPT).unwrap();
        let m = row.merge.unwrap();
        assert_eq!(m.changed, changed);
        assert_eq!(m.by_seat, 5);
        assert_eq!(m.result, vec![("bin/a.py".into(), r.live(DEPT, "bin/a.py").canon())]);
        assert!(row.merge_lock.is_none());
        let rec = ledger::read_record_in(&r.root, &id).unwrap().unwrap();
        assert_eq!(row.applied, rec.committed, "the origin result it merged against (interpretation 41)");
    }
}

/// H8: a failing check → 충돌 stands (the lock kept); another seat, or no merge lock → refused; an
/// approval-class repair → refused naming R1 / T1.
#[test]
fn recheck_refusals() {
    let r = rig("m3");
    let id = rid("R-1");
    committed_in_hq(&r, &id, Classification::General, "bin/a.py", Some((b"old\n", 0o644)), Some((b"new\n", 0o644)));
    r.put(DEPT, "bin/a.py", b"mine\n", 0o644);
    tick(&r, DEPT, &doubles(), &fid(1));
    let d = doubles();
    assert!(matches!(recheck(&r, &id, &d, 5, ""), Err(RepairError::Locked(_))), "no merge lock yet");
    conflicted_with_lock(&r, &id, b"mine\n");
    assert!(matches!(recheck(&r, &id, &d, 6, ""), Err(RepairError::Locked(_))), "another seat");
    d.pass.set(false);
    assert!(matches!(recheck(&r, &id, &d, 5, ""), Err(RepairError::CheckFailed(_))));
    let row = r.row(&id, DEPT).unwrap();
    assert_eq!(row.state, LaneState::Conflict);
    assert!(row.merge_lock.is_some() && row.merge.is_none());
    d.class.set(Classification::BootCritical);
    match recheck(&r, &id, &d, 5, "") {
        Err(RepairError::Precondition(m)) => assert!(m.contains("RULINGS R1/T1") && m.contains("--resolves R-1"), "{m}"),
        other => panic!("{other:?}"),
    }
}

// ───────────────────────────── the committed copy (HI2 / HJ2) ─────────────────────────────

/// HJ2: exactly the limit is sealed; one byte over is refused with the limit in the sentence.
#[test]
fn the_seal_takes_exactly_the_limit_and_refuses_one_byte_over() {
    let r = rig("s1");
    let id = rid("R-1");
    let live = r.hq.join("bin/big.bin");
    let f = std::fs::File::create(&live).unwrap();
    f.set_len(SEAL_LIMIT).unwrap();
    drop(f);
    let c = seal_committed_copy_in(&r.root, &id, &lane(HQ), "bin/big.bin", &live, T0).unwrap();
    assert_eq!(c.state, TargetState::read(&live).unwrap());
    let f = std::fs::OpenOptions::new().write(true).open(&live).unwrap();
    f.set_len(SEAL_LIMIT + 1).unwrap();
    drop(f);
    let e = seal_committed_copy_in(&r.root, &id, &lane(HQ), "bin/big.bin", &live, T0).unwrap_err();
    assert!(e.contains(&SEAL_LIMIT.to_string()), "{e}");
}

/// HJ2: a source that grows while it is copied is refused (at most limit + 1 bytes are read and the
/// length must equal the length at open).
#[test]
fn the_seal_refuses_a_source_that_grows_while_copied() {
    let r = rig("s2");
    let id = rid("R-1");
    let live = r.hq.join("bin/a.py");
    std::fs::write(&live, b"0123456789").unwrap();
    BETWEEN_STAT_AND_READ.with(|h| {
        *h.borrow_mut() = Some(Box::new(|p: &Path| {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new().append(true).open(p).unwrap();
            f.write_all(b"more").unwrap();
        }))
    });
    let e = seal_committed_copy_in(&r.root, &id, &lane(HQ), "bin/a.py", &live, T0);
    BETWEEN_STAT_AND_READ.with(|h| *h.borrow_mut() = None);
    assert!(e.is_err(), "{e:?}");
}

/// The committed copy is found by its metadata and accepted only when its bytes hash, with that
/// handle's mode, to R's committed state (content-addressed).
#[test]
fn the_committed_copy_is_content_addressed() {
    let r = rig("s4");
    let id = rid("R-1");
    let rec = committed_in_hq(&r, &id, Classification::General, "bin/a.py", Some((b"old\n", 0o644)), Some((b"new\n", 0o644)));
    let holder = committed_copy_in(&r.root, &rec, "bin/a.py").unwrap();
    assert_eq!(std::fs::read(&holder).unwrap(), b"new\n");
    let mut other = rec.clone();
    other.committed = vec![("bin/a.py".into(), TargetState::File { sha256: "0".repeat(64), mode: 0o644 }.canon())];
    assert!(committed_copy_in(&r.root, &other, "bin/a.py").is_err());
}

// ───────────────────────────── pins ─────────────────────────────

fn production_text() -> String {
    let src = include_str!("propagate.rs");
    let cut = src.find("\n#[cfg(test)]\n#[path = \"propagate_tests.rs\"]").unwrap_or(src.len());
    src[..cut].lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n")
}

/// The one door (EJ1) and rule one: this module never exchanges, places, renames or writes a live
/// pack path itself — every live change is `apply::run_op`.
#[test]
fn propagation_writes_live_files_only_through_the_engine() {
    let t = production_text();
    for needle in ["exchange(", "place_no_replace(", "swap_paths(", "std::fs::rename(", "std::fs::copy(", "renamex_np"] {
        assert!(!t.contains(needle), "{needle} in propagate.rs");
    }
    assert!(t.contains("apply::run_op(&env.op, &spec)"));
}

/// F1: the propagation sends nothing (ledger files only).
#[test]
fn propagation_sends_nothing() {
    let t = production_text();
    for needle in ["bus.publish", "enqueue", "send_text", "alert_route", "inject(", "push_queued", "cys send"] {
        assert!(!t.contains(needle), "{needle} in propagate.rs");
    }
}

/// The scratch of a dead process is swept by its exact name; this process's and other names stay.
#[test]
fn only_dead_scratch_names_are_swept() {
    let r = rig("sw");
    let l = lane(DEPT);
    let lanes = r.root.join("lanes");
    let dead = lanes.join(format!("{}1-5", scratch_prefix(&l)));
    let mine = lanes.join(format!("{}{}-5", scratch_prefix(&l), std::process::id()));
    let other = lanes.join(".propagate-base-1-5");
    let odd = lanes.join(format!("{}x-5", scratch_prefix(&l)));
    for d in [&dead, &mine, &other, &odd] {
        std::fs::create_dir_all(d).unwrap();
    }
    sweep_own_scratch(&r.root, &l);
    assert!(!dead.exists());
    assert!(mine.exists() && other.exists() && odd.exists());
}

/// HH7: ONE line over every registered lane holding a 충돌 row of an in-force repair.
#[test]
fn the_conflict_summary_is_one_line_over_all_lanes() {
    let r = rig("sum");
    assert_eq!(conflict_summary_line_in(&r.root, &r.registry), None);
    let id = rid("R-1");
    committed_in_hq(&r, &id, Classification::General, "bin/a.py", Some((b"old\n", 0o644)), Some((b"new\n", 0o644)));
    // the registry's lane key of the department socket
    let dept_key = crate::repair::types::LaneKey::of_socket(Path::new("/x/cys-dept-dept-a/cys.sock"));
    let mut row = new_row(&id, &dept_key, T0);
    row.state = LaneState::Conflict;
    ledger::write_lane_durable_in(&r.root, &row).unwrap();
    let line = conflict_summary_line_in(&r.root, &r.registry).unwrap();
    assert!(!line.contains('\n') && line.contains("R-1") && line.contains(&dept_key.0), "{line}");
}


/// I8 — the id file survives the product's per-file install (the daemon's boot install and, since
/// GH1, `init-pack` on an existing pack run this same function in place) and its prune of stale
/// manifest entries: byte-identical, the same mtime; the pass then answers Equal (no row demoted, the
/// gate unchanged).
#[test]
fn the_id_file_survives_the_per_file_install_and_its_prune() {
    let r = rig("i8");
    let id = rid("R-1");
    committed_in_hq(&r, &id, Classification::General, "bin/a.py", Some((b"old\n", 0o644)), Some((b"new\n", 0o644)));
    r.put(DEPT, "bin/a.py", b"old\n", 0o644);
    tick(&r, DEPT, &doubles(), &fid(1));
    assert_eq!(gate_verdict_in(&r.root, &lane(DEPT)), GateVerdict::Open);
    let idp = crate::repair::paths::pack_instance_path(&r.dept);
    let (bytes, mtime) = (std::fs::read(&idp).unwrap(), std::fs::metadata(&idp).unwrap().modified().unwrap());
    // two installs: the second drops an item (its manifest entry is pruned) and changes another
    for items in [vec![("bin/x.py", "print(1)\n"), ("bin/y.py", "print(2)\n")], vec![("bin/x.py", "print(3)\n")]] {
        crate::pack::install_into(r.dept.clone(), items, false, "0.15.0", false, false, crate::pack::PackScope::Dept, None, None).unwrap();
    }
    assert_eq!(std::fs::read(&idp).unwrap(), bytes, "byte-identical");
    assert_eq!(std::fs::metadata(&idp).unwrap().modified().unwrap(), mtime, "not rewritten");
    let l = lane(DEPT);
    let no = |_: PassStep| false;
    assert_eq!(pack_instance_pass_in(&pass_env(&r, &l, DEPT, &no), &fid(2)), PassOutcome::Equal);
    assert_eq!(r.row(&id, DEPT).unwrap().state, LaneState::Verified, "no row demoted");
    assert_eq!(gate_verdict_in(&r.root, &lane(DEPT)), GateVerdict::Open, "the gate unchanged");
}
