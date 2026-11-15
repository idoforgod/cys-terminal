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
        reason: String::new(),
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
    /// IG4: the reference check's duration (ms) this test's reference double reports.
    reference_ms: Cell<u64>,
    /// IZ2 / JA1: a hook unit used INSTEAD of the default double (e.g. T6's real guarded unit with a
    /// fake registrar).
    unit: std::cell::RefCell<Option<Box<UnitDouble>>>,
    /// IK3 / IG5: runs inside every check call (the root it checks) — a seam that "parks" a check.
    during_check: std::cell::RefCell<Option<Box<dyn Fn(&Path)>>>,
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
        reference_ms: Cell::new(7),
        during_check: std::cell::RefCell::new(None),
        unit: std::cell::RefCell::new(None),
    }
}

type UnitDouble = dyn Fn(&RepairId, &LaneKey, HookStep, &dyn Fn() -> Result<(), String>) -> Result<apply::HookUnitEnd, RepairError>;

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
        if let Some(f) = d.during_check.borrow().as_ref() {
            f(root);
        }
        // the check reads the file under the root it is given (a copy or the live pack)
        let result = match TargetState::read(&root.join(rel)) {
            Ok(TargetState::File { .. }) if d.pass.get() => CheckResult::Pass,
            Ok(TargetState::File { .. }) => CheckResult::Fail("검사 실패(시험)".into()),
            other => CheckResult::Fail(format!("{rel}: {other:?}")),
        };
        Ok(CheckOutcome { result, ms: 1 })
    };
    let classify = |_: &str| -> Result<Classification, RepairError> { Ok(d.class.get()) };
    // the hook unit's double (IZ2): the guard first (a unit with no slow preparation), then the step
    let hooks = |id: &RepairId, l: &LaneKey, step: HookStep, guard: &dyn Fn() -> Result<(), String>| -> Result<apply::HookUnitEnd, RepairError> {
        if let Some(u) = d.unit.borrow().as_ref() {
            return u(id, l, step, guard);
        }
        if let Err(why) = guard() {
            return Ok(apply::HookUnitEnd::Refused(why));
        }
        d.hook_calls.set(d.hook_calls.get() + 1);
        if d.hooks_ok.get() {
            Ok(apply::HookUnitEnd::Ran { op: "op-test".into(), lines: vec!["Stop:x.sh".into()], result: Ok(ReconcileReport { hooks: Vec::new() }) })
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
        reference: &|_, _| Ok(CheckOutcome { result: CheckResult::Pass, ms: d.reference_ms.get() }),
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

/// RULINGS IG7 — THE ONE POSITIVE PATH of HJ1 (a), with REAL approved-apply evidence of T6 (the helper
/// T6's own F9 tests use): a boot-critical R committed in HQ is 충돌 in the department (never applied);
/// the department's OWN R′ with `resolves: R` is approved and applied there through T6's
/// `apply_candidate_in`; the next tick finds every live target an approved copy of R′ (F9), runs R's
/// check on the live files and writes 검증됨 with the merge record pointing to R′ — the gate OPENS.
/// Variants (each on its own rig): one piece of the evidence removed (the deposit's sealed copy
/// altered), the live check failing, R′ reverted — each stays 충돌.
#[cfg(target_os = "macos")]
#[test]
fn an_approval_class_row_is_verified_only_through_the_lanes_own_approved_repair() {
    for variant in ["positive", "evidence-removed", "check-fails", "resolving-repair-reverted"] {
        let r = rig(&format!("ig7-{variant}"));
        let id = rid("R-1");
        let rel = "hooks/b.sh";
        committed_in_hq(&r, &id, Classification::BootCritical, rel, Some((b"PRE\n", 0o755)), Some((b"HQ\n", 0o755)));
        r.put(DEPT, rel, b"PRE\n", 0o755);
        let d = doubles();
        d.class.set(Classification::BootCritical);
        tick(&r, DEPT, &d, &fid(1));
        assert_eq!(r.row(&id, DEPT).unwrap().state, LaneState::Conflict, "{variant}: never applied here");
        assert!(matches!(gate_verdict_in(&r.root, &lane(DEPT)), GateVerdict::Held(HoldCause::AwaitingFinalApproval { .. })));
        // the department's OWN repair R′ (resolves R), approved and applied in this lane by T6's engine
        let t6 = crate::repair::apply::tests::Rig { base: r.base.join("t6"), root: r.root.clone(), pack: r.dept.clone(), lane: lane(DEPT), src: r.base.join("t6").join("src") };
        std::fs::create_dir_all(&t6.src).unwrap();
        let rp = rid("R-2");
        let mut prime = crate::repair::apply::tests::approved_and_applied(&t6, &rp, rel, b"PRE\n", b"DEPT-APPROVED\n");
        prime.resolves = Some(id.clone());
        ledger::write_record_in(&r.root, &prime).unwrap();
        assert!(in_force(&prime), "{variant}: R′ applied and in force");
        match variant {
            "evidence-removed" => {
                let op = r.ops(&rp).into_iter().find(|o| o.kind == "apply").unwrap();
                let slot = crate::repair::vault::Slot::at(op.slot.as_ref().unwrap(), rel);
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(slot.sealed(), std::fs::Permissions::from_mode(0o644)).unwrap();
                std::fs::write(slot.sealed(), b"TAMPERED").unwrap();
            }
            "check-fails" => d.pass.set(false),
            "resolving-repair-reverted" => {
                prime.state = RepairState::Reverted;
                ledger::write_record_in(&r.root, &prime).unwrap();
            }
            _ => {}
        }
        tick(&r, DEPT, &d, &fid(1));
        let row = r.row(&id, DEPT).unwrap();
        if variant == "positive" {
            assert_eq!(row.state, LaneState::Verified, "{}", row.reason);
            assert_eq!(row.merge.as_ref().and_then(|m| m.via_repair.clone()), Some(rp.clone()));
            assert_eq!(gate_verdict_in(&r.root, &lane(DEPT)), GateVerdict::Open);
            assert_eq!(std::fs::read(r.dept.join(rel)).unwrap(), b"DEPT-APPROVED\n", "the daemon wrote nothing of R");
        } else {
            assert_eq!(row.state, LaneState::Conflict, "{variant}");
            assert!(row.merge.as_ref().and_then(|m| m.via_repair.clone()).is_none(), "{variant}");
            assert!(matches!(gate_verdict_in(&r.root, &lane(DEPT)), GateVerdict::Held(_)), "{variant}");
        }
        assert!(r.ops(&id).iter().all(|o| o.lane_key != lane(DEPT)), "{variant}: no operation of R in this lane");
    }
}

/// RULINGS IG2 — a verified row of a deleting repair implies this lane's tombstone: a stop between the
/// deletion's door operation and its tombstone; after ONE restart the target is absent ("already at the
/// result") and the row is 검증됨 WITH the tombstone — so an update does not put the vendor file back
/// (shown with the per-file install fed this lane's tombstones; without them it would restore the file).
#[test]
fn a_stop_between_the_deletion_and_its_tombstone_is_completed_at_the_next_tick() {
    let r = rig("ig2");
    let id = rid("R-1");
    committed_in_hq(&r, &id, Classification::General, "bin/gone.py", Some((b"x\n", 0o644)), None);
    r.put(DEPT, "bin/gone.py", b"x\n", 0o644);
    STOP_AFTER_DOOR_OP.with(|c| c.set(true));
    let e = tick_with(&r, DEPT, &doubles(), &fid(1), &NoFaults, &|_| false).unwrap_err();
    assert!(format!("{e:?}").contains(apply::INJECTED_STOP), "{e:?}");
    assert_eq!(r.live(DEPT, "bin/gone.py"), TargetState::Absent, "the door operation was done");
    assert!(crate::repair::tombstones::load_in(&r.root, &lane(DEPT)).unwrap().is_empty(), "the stop fell before the tombstone");
    // one restart
    tick(&r, DEPT, &doubles(), &fid(1));
    assert_eq!(r.row(&id, DEPT).unwrap().state, LaneState::Verified);
    let ts = crate::repair::tombstones::load_in(&r.root, &lane(DEPT)).unwrap();
    assert!(ts.iter().any(|t| t.rel == "bin/gone.py" && t.repair == id), "{ts:?}");
    // an update: the install honours the lane's tombstone (and, as the control, restores without it)
    let item = vec![("bin/gone.py", "x\n")];
    let with = crate::pack::UpdateSources::none().with_tombstones(ts);
    crate::pack::install_into_with(r.dept.clone(), item.clone(), false, "0.15.0", false, false, crate::pack::PackScope::Dept, None, None, &with).unwrap();
    assert_eq!(r.live(DEPT, "bin/gone.py"), TargetState::Absent, "the update does not put the file back");
    crate::pack::install_into_with(r.dept.clone(), item, false, "0.15.0", false, false, crate::pack::PackScope::Dept, None, None, &crate::pack::UpdateSources::none()).unwrap();
    assert!(matches!(r.live(DEPT, "bin/gone.py"), TargetState::File { .. }), "control: without the tombstone the install restores it");
}

/// RULINGS IG3 (C11 11.4, 1.6) — a stop after the last door operation and before the 적용됨 row: at the
/// next tick the lane is "already there", and THIS lane's own completed door operations (the write-ahead
/// records at the present generation, results equal to the live states) show that it applied R — the
/// row gets its probation bundle as if the write had not been interrupted (24-hour re-check, the return
/// owed on a failure); the check's timing record is kept.
#[test]
fn a_stop_before_the_row_keeps_the_lanes_probation() {
    let r = rig("ig3");
    let id = rid("R-1");
    committed_in_hq(&r, &id, Classification::General, "bin/a.py", Some((b"old\n", 0o644)), Some((b"new\n", 0o644)));
    r.put(DEPT, "bin/a.py", b"old\n", 0o644);
    STOP_AFTER_DOOR_OP.with(|c| c.set(true));
    assert!(tick_with(&r, DEPT, &doubles(), &fid(1), &NoFaults, &|_| false).is_err());
    assert_eq!(std::fs::read(r.dept.join("bin/a.py")).unwrap(), b"new\n", "the door operation was done");
    assert!(r.row(&id, DEPT).is_none(), "the stop fell before the row");
    let op = r.ops(&id).into_iter().find(|o| o.lane_key == lane(DEPT) && o.kind == "propagate").unwrap();
    tick(&r, DEPT, &doubles(), &fid(1));
    let row = r.row(&id, DEPT).unwrap();
    assert_eq!(row.state, LaneState::Verified);
    let p = row.probation.unwrap();
    assert!(p.closed.is_none(), "an open probation (the 24-hour re-check)");
    assert_eq!(p.applied_at, op.updated_at, "from the lane's own operation");
    assert_eq!((p.expected, p.check_timings.len()), (row.applied, 1));
    // the same stop, and the lane's check fails at the restart → 충돌 with the return OWED (as after an
    // uninterrupted apply), never a bare 충돌 that T6's probation would skip
    let r2 = rig("ig3b");
    committed_in_hq(&r2, &id, Classification::General, "bin/a.py", Some((b"old\n", 0o644)), Some((b"new\n", 0o644)));
    r2.put(DEPT, "bin/a.py", b"old\n", 0o644);
    STOP_AFTER_DOOR_OP.with(|c| c.set(true));
    assert!(tick_with(&r2, DEPT, &doubles(), &fid(1), &NoFaults, &|_| false).is_err());
    let d = doubles();
    d.pass.set(false);
    tick(&r2, DEPT, &d, &fid(1));
    let row = r2.row(&id, DEPT).unwrap();
    assert_eq!(row.state, LaneState::Conflict);
    let closed = row.probation.and_then(|p| p.closed).unwrap_or_default();
    assert!(apply::closure_owes_return(&closed), "the exchange form owes the return: {closed}");
}

/// RULINGS IG4 (P3 / D④) — the reference measurement is taken in the RECEIVING lane at its apply (the
/// same function T6's origin apply uses), so T6's load-corrected verdict exists there: the control (the
/// lane's pre-image) passes, the repaired check times out twice, the reference is normal → a FAILURE is
/// attributed → T6's automatic return of THIS lane's operation. (With `reference_ms` 0, T6 never blames
/// a timeout — the old propagation recorded 0 for every target.)
#[cfg(target_os = "macos")]
#[test]
fn a_receiving_lanes_repeated_timeout_is_attributed_with_its_own_reference() {
    use crate::repair::probation::{run_tick, JobEnv};
    let r = rig("ig4");
    let id = rid("R-1");
    committed_in_hq(&r, &id, Classification::General, "bin/a.py", Some((b"old\n", 0o644)), Some((b"new\n", 0o644)));
    r.put(DEPT, "bin/a.py", b"old\n", 0o644);
    let d = doubles();
    d.reference_ms.set(100);
    tick(&r, DEPT, &d, &fid(1));
    let row = r.row(&id, DEPT).unwrap();
    assert_eq!(row.state, LaneState::Verified);
    let timing = row.probation.as_ref().unwrap().check_timings[0].clone();
    assert_eq!((timing.reference_ms, timing.reference_kind), (100, apply::check_kind_of("bin/a.py")), "measured in this lane");
    // T6's 24-hour re-check in THIS lane
    let dept = r.dept.clone();
    let check = move |_: &str, root: &Path, _: Duration| -> Result<CheckOutcome, RepairError> {
        if root == dept.as_path() {
            Ok(CheckOutcome { result: CheckResult::Timeout, ms: 60_000 }) // the repaired file, twice
        } else {
            Ok(CheckOutcome { result: CheckResult::Pass, ms: 5 }) // the control on the pre-image copy
        }
    };
    let reference = |_: crate::repair::boot_critical::CheckKind, _: Duration| -> Result<CheckOutcome, RepairError> { Ok(CheckOutcome { result: CheckResult::Pass, ms: 100 }) };
    let hooks = |_: &RepairId, _: &LaneKey, _: HookStep| -> Result<ReconcileReport, RepairError> { Ok(ReconcileReport { hooks: Vec::new() }) };
    let registrar = |_: &Path, _: bool| -> Result<String, RepairError> { Ok("{\"hooks\": []}".into()) };
    let l = lane(DEPT);
    let lock = r.base.join(".pack-apply.lock");
    let job = JobEnv {
        op: OpEnv { root: &r.root, pack: &r.dept, lane: &l, now: T0 + 700.0, form: ApplyForm::Exchange, faults: &NoFaults, own_check: &|_, _| None },
        incarnation: "inc-ig4",
        check: &check,
        reference: &reference,
        hooks: &hooks,
        registrar: &registrar,
        locked: &apply::no_repair_lock,
        excluded: &|_| false,
        original: &|_| None,
        intake: &|_, _| true,
        apply_lock: &lock,
    };
    let rep = run_tick(&job, false).unwrap();
    assert_eq!(rep.reverted, vec![id.clone()], "{rep:?}");
    assert_eq!(std::fs::read(r.dept.join("bin/a.py")).unwrap(), b"old\n", "this lane's operation returned");
    assert_eq!(std::fs::read(r.hq.join("bin/a.py")).unwrap(), b"new\n", "never another lane's file");
}

/// RULINGS IQ3 — the probation restored by IG3 is RETURNABLE: stop after the door operation and before
/// the row → restart → the lane's check FAILS → the row is 충돌 "실패 — 되돌리기 남음" WITH R's result as
/// `applied` (before: empty, and T6's `revert_in` refused it — `revertible_here`) → T6's REAL `run_tick`
/// in this lane takes the owed return: the lane's original bytes come back and the return ends as done
/// (`되돌려짐 — 데몬(자동)`), not as a closure string that stays owed or is settled as failed.
#[cfg(target_os = "macos")]
#[test]
fn a_restored_probation_is_returned_by_t6s_tick() {
    use crate::repair::probation::{run_tick, JobEnv};
    let r = rig("iq3");
    let id = rid("R-1");
    committed_in_hq(&r, &id, Classification::General, "bin/a.py", Some((b"old\n", 0o644)), Some((b"new\n", 0o644)));
    r.put(DEPT, "bin/a.py", b"old\n", 0o644);
    STOP_AFTER_DOOR_OP.with(|c| c.set(true));
    assert!(tick_with(&r, DEPT, &doubles(), &fid(1), &NoFaults, &|_| false).is_err());
    assert!(r.row(&id, DEPT).is_none(), "the stop fell before the row");
    let d = doubles();
    d.pass.set(false);
    tick(&r, DEPT, &d, &fid(1));
    let row = r.row(&id, DEPT).unwrap();
    assert_eq!(row.state, LaneState::Conflict);
    assert_eq!(row.applied, ledger::read_record_in(&r.root, &id).unwrap().unwrap().committed, "returnable: what was applied is on the row");
    assert!(apply::closure_owes_return(row.probation.as_ref().and_then(|p| p.closed.as_deref()).unwrap_or("")));
    assert!(apply::revertible_here(&ledger::read_record_in(&r.root, &id).unwrap().unwrap(), &row, &lane(DEPT)));
    // T6's job in THIS lane
    let check = |_: &str, _: &Path, _: Duration| -> Result<CheckOutcome, RepairError> { Ok(CheckOutcome { result: CheckResult::Pass, ms: 5 }) };
    let reference = |_: crate::repair::boot_critical::CheckKind, _: Duration| -> Result<CheckOutcome, RepairError> { Ok(CheckOutcome { result: CheckResult::Pass, ms: 100 }) };
    let hooks = |_: &RepairId, _: &LaneKey, _: HookStep| -> Result<ReconcileReport, RepairError> { Ok(ReconcileReport { hooks: Vec::new() }) };
    let registrar = |_: &Path, _: bool| -> Result<String, RepairError> { Ok("{\"hooks\": []}".into()) };
    let l = lane(DEPT);
    let lock = r.base.join(".pack-apply.lock");
    let job = JobEnv {
        op: OpEnv { root: &r.root, pack: &r.dept, lane: &l, now: T0 + 60.0, form: ApplyForm::Exchange, faults: &NoFaults, own_check: &|_, _| None },
        incarnation: "inc-iq3",
        check: &check,
        reference: &reference,
        hooks: &hooks,
        registrar: &registrar,
        locked: &apply::no_repair_lock,
        excluded: &|_| false,
        original: &|_| None,
        intake: &|_, _| true,
        apply_lock: &lock,
    };
    let rep = run_tick(&job, false).unwrap();
    assert_eq!(rep.reverted, vec![id.clone()], "{rep:?}");
    assert_eq!(std::fs::read(r.dept.join("bin/a.py")).unwrap(), b"old\n", "the lane's original bytes are back");
    assert_eq!(std::fs::read(r.hq.join("bin/a.py")).unwrap(), b"new\n", "never another lane's file");
    let closed = r.row(&id, DEPT).unwrap().probation.and_then(|p| p.closed).unwrap_or_default();
    assert_eq!(closed, format!("{} — {}", apply::CLOSED_REVERTED, apply::BY_DAEMON_AUTO), "the owed return ended as DONE");
}

/// IG3: a lane that was at R's result WITHOUT having applied it (no operation of its own) gets no
/// 24-hour re-check — nothing of its own was replaced — but its check's timing record is kept (a
/// CLOSED bundle that T6's probation skips and that owes no report).
#[test]
fn a_lane_at_the_result_without_its_own_apply_keeps_the_timings_and_no_probation() {
    let r = rig("ig3c");
    let id = rid("R-1");
    committed_in_hq(&r, &id, Classification::General, "bin/a.py", Some((b"old\n", 0o644)), Some((b"new\n", 0o644)));
    r.put(DEPT, "bin/a.py", b"new\n", 0o644);
    tick(&r, DEPT, &doubles(), &fid(1));
    let row = r.row(&id, DEPT).unwrap();
    assert_eq!(row.state, LaneState::Verified);
    let p = row.probation.unwrap();
    assert_eq!(p.closed.as_deref(), Some(NOTHING_REPLACED_HERE));
    assert_eq!(p.check_timings.len(), 1);
    assert!(!apply::closure_owes_return(NOTHING_REPLACED_HERE) && !apply::closure_owes_intake(NOTHING_REPLACED_HERE));
}

/// IG2: a lane that was at the deleting repair's result from the start gets the tombstone too.
#[test]
fn a_lane_already_at_a_deletions_result_gets_the_tombstone() {
    let r = rig("ig2b");
    let id = rid("R-1");
    committed_in_hq(&r, &id, Classification::General, "bin/gone.py", Some((b"x\n", 0o644)), None);
    tick(&r, DEPT, &doubles(), &fid(1));
    assert_eq!(r.row(&id, DEPT).unwrap().state, LaneState::Verified);
    let ts = crate::repair::tombstones::load_in(&r.root, &lane(DEPT)).unwrap();
    assert!(ts.iter().any(|t| t.rel == "bin/gone.py" && t.repair == id), "{ts:?}");
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
    // REWRITTEN under RULINGS IV1 — old expectation, quoted: `assert_eq!(r.row(&id, DEPT).unwrap().state,
    // LaneState::Conflict)` after this ONE tick. The door refused; the target differs from the
    // decision's stamp → nothing written this tick; the next tick: the other writer's bytes are not a
    // known vendor version → 충돌, nothing moved
    assert!(r.row(&id, DEPT).is_none());
    tick(&r, DEPT, &d, &fid(1));
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
    // RULINGS IK2: the true reason is KEPT on the row (status and the merge ticket show it)
    assert!(row.reason.contains("결과 사본") && row.reason.contains("바이트를 마련하지 못했습니다"), "{}", row.reason);
}

/// RULINGS IK2 — the reviewer's sequence: a department's copy-check TIMES OUT (or cannot run); the row
/// is 충돌 and its reason — kept on the row — says that R's bytes never arrived (the live file is
/// untouched), so the worker's merge ticket and `status` say it; a row left 대기 by a persistent cause
/// keeps that cause too, and a passing cause (the lock held) is not written.
#[test]
fn a_rows_reason_is_kept_on_the_row() {
    let r = rig("ik2");
    let id = rid("R-1");
    committed_in_hq(&r, &id, Classification::General, "bin/a.py", Some((b"old\n", 0o644)), Some((b"new\n", 0o644)));
    r.put(DEPT, "bin/a.py", b"old\n", 0o644);
    let d = doubles();
    d.pass.set(false); // the copy-check fails (a timeout or "cannot run" takes the same path with its own words)
    tick(&r, DEPT, &d, &fid(1));
    let row = r.row(&id, DEPT).unwrap();
    assert_eq!(row.state, LaneState::Conflict);
    assert!(row.reason.starts_with("사본 검사 실패 — 살아 있는 파일은 바꾸지 않았습니다"), "{}", row.reason);
    assert_eq!(std::fs::read(r.dept.join("bin/a.py")).unwrap(), b"old\n");
    // a 대기 row left by a cause that persists (the generation cannot be read) keeps it
    let r2 = rig("ik2b");
    committed_in_hq(&r2, &id, Classification::General, "bin/a.py", Some((b"old\n", 0o644)), Some((b"new\n", 0o644)));
    r2.put(DEPT, "bin/a.py", b"old\n", 0o644);
    std::fs::remove_file(r2.dept.join(".pack-version")).unwrap();
    tick(&r2, DEPT, &doubles(), &fid(1));
    let row = r2.row(&id, DEPT).unwrap();
    assert_eq!((row.state, row.reason.as_str()), (LaneState::Pending, "팩 세대를 알 수 없어 이번 차례에는 적용하지 않습니다"));
    let before = std::fs::read(ledger::lane_path_in(&r2.root, &id, &lane(DEPT))).unwrap();
    tick(&r2, DEPT, &doubles(), &fid(1));
    assert_eq!(std::fs::read(ledger::lane_path_in(&r2.root, &id, &lane(DEPT))).unwrap(), before, "the same reason is not written again");
    assert!(is_passing_cause(LOCK_BUSY_ROW) && !is_passing_cause(RECORD_CHANGED));
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
    let t = tick_with(&r, DEPT, &doubles(), &fid(1), &Foreign, &|_| false).unwrap();
    assert_eq!(std::fs::read(r.dept.join("bin/a.py")).unwrap(), b"foreign\n");
    // REWRITTEN under RULINGS IV1 — old expectation, quoted: `assert_eq!(r.row(&id, DEPT).unwrap().state,
    // LaneState::Conflict)` after this ONE tick. The door declined (GV1: another writer's bytes stay);
    // the target's state now differs from the decision's stamp, so nothing is written for that decision
    // this tick; the next tick decides from states — the foreign bytes are neither R's pre-image nor its
    // result → 충돌 (never "superseded"), the bytes still kept
    assert!(r.row(&id, DEPT).is_none(), "{t:?}");
    tick(&r, DEPT, &doubles(), &fid(1));
    assert_eq!(std::fs::read(r.dept.join("bin/a.py")).unwrap(), b"foreign\n");
    assert_eq!(r.row(&id, DEPT).unwrap().state, LaneState::Conflict);
}

/// A committed repair of SEVERAL files made in HQ (as `committed_in_hq` for one): per file the pre-image
/// copied, the HQ file edited, the committed copy sealed; the record in probation, HQ's row verified.
#[allow(clippy::type_complexity)]
fn committed_many_in_hq(r: &Rig, id: &RepairId, files: &[(&str, &[u8], &[u8])]) -> RepairRecord {
    let mut changes = Vec::new();
    let mut committed = Vec::new();
    for (rel, pre, post) in files {
        let live = r.hq.join(rel);
        let _ = std::fs::remove_file(&live);
        r.put(HQ, rel, pre, 0o644);
        let pre_img = ledger::copy_preimage_in(&r.root, id, &lane(HQ), rel, &live, T0 - 10.0).unwrap();
        r.put(HQ, rel, post, 0o644);
        let post_st = TargetState::read(&live).unwrap();
        seal_committed_copy_in(&r.root, id, &lane(HQ), rel, &live, T0 - 5.0).unwrap();
        changes.push(FileChange { rel: (*rel).into(), kind: ChangeKind::Edit, pre: pre_img, post: post_st.clone(), diff: None });
        committed.push(((*rel).to_string(), post_st.canon()));
    }
    record(r, id, Classification::General, changes, committed, HQ)
}

/// RULINGS IO1 — the record changes BETWEEN two targets of this lane's apply (here: superseded right
/// after the first door operation): no undo — the second target is not applied, and the row is
/// PUBLISHED as 충돌 at once with the reason naming the target already changed and the live states.
/// Then (each from states, no new code): R re-committed → the next tick keeps the 충돌; R superseded by
/// a newer repair → THAT repair's own row meets the half-changed files and is 충돌 where they differ.
#[test]
fn a_record_changed_between_two_targets_publishes_a_visible_conflict() {
    struct SupersedeAfterFirst(PathBuf, RepairId, Cell<bool>);
    impl Faults for SupersedeAfterFirst {
        fn at(&self, p: Point, live: &Path, _: Option<&Slot>) -> bool {
            if p == Point::BeforeDone && !self.2.get() && live.ends_with("bin/a.py") {
                self.2.set(true);
                let mut rec = ledger::read_record_in(&self.0, &self.1).unwrap().unwrap();
                rec.state = RepairState::Superseded;
                ledger::write_record_in(&self.0, &rec).unwrap();
            }
            false
        }
    }
    for follow_up in ["re-committed", "superseded-by-a-newer-repair"] {
        let r = rig(&format!("io1-{follow_up}"));
        let id = rid("R-1");
        committed_many_in_hq(&r, &id, &[("bin/a.py", b"old-a\n", b"new-a\n"), ("bin/b.py", b"old-b\n", b"new-b\n")]);
        r.put(DEPT, "bin/a.py", b"old-a\n", 0o644);
        r.put(DEPT, "bin/b.py", b"old-b\n", 0o644);
        let f = SupersedeAfterFirst(r.root.clone(), id.clone(), Cell::new(false));
        let t = tick_with(&r, DEPT, &doubles(), &fid(1), &f, &|_| false).unwrap();
        assert_eq!(std::fs::read(r.dept.join("bin/a.py")).unwrap(), b"new-a\n", "the first target was changed");
        assert_eq!(std::fs::read(r.dept.join("bin/b.py")).unwrap(), b"old-b\n", "the second target was NOT applied");
        let row = r.row(&id, DEPT).unwrap();
        assert_eq!(row.state, LaneState::Conflict, "{t:?}");
        assert!(row.reason.contains("도중 수리 기록이 바뀌었습니다") && row.reason.contains("이미 바꾼 대상: bin/a.py"), "{}", row.reason);
        assert!(!row.reason.contains("bin/b.py"), "{}", row.reason);
        let live_a = r.live(DEPT, "bin/a.py").canon();
        let live_b = r.live(DEPT, "bin/b.py").canon();
        assert_eq!(row.conflict_live, vec![("bin/a.py".to_string(), live_a), ("bin/b.py".to_string(), live_b)]);
        let ops_here = r.ops(&id).into_iter().filter(|o| o.lane_key == lane(DEPT)).count();
        assert_eq!(ops_here, 1, "one door operation, nothing put back, nothing further written");
        let bytes_before = (std::fs::read(r.dept.join("bin/a.py")).unwrap(), std::fs::read(r.dept.join("bin/b.py")).unwrap());
        if follow_up == "re-committed" {
            // R committed again with another result: the next tick keeps 충돌 for the changed targets
            let mut rec = ledger::read_record_in(&r.root, &id).unwrap().unwrap();
            rec.state = RepairState::Probation;
            rec.committed = vec![
                ("bin/a.py".into(), TargetState::File { sha256: "1".repeat(64), mode: 0o644 }.canon()),
                ("bin/b.py".into(), TargetState::File { sha256: "2".repeat(64), mode: 0o644 }.canon()),
            ];
            ledger::write_record_in(&r.root, &rec).unwrap();
            tick(&r, DEPT, &doubles(), &fid(1));
            assert_eq!(r.row(&id, DEPT).unwrap().state, LaneState::Conflict);
        } else {
            // a newer repair R-2 of the same files (HQ's pre = R's result): its own row meets the
            // half-changed files — a.py is at its pre-image, b.py is neither → 충돌, nothing applied
            let id2 = rid("R-2");
            committed_many_in_hq(&r, &id2, &[("bin/a.py", b"new-a\n", b"newer-a\n"), ("bin/b.py", b"new-b\n", b"newer-b\n")]);
            tick(&r, DEPT, &doubles(), &fid(1));
            let row2 = r.row(&id2, DEPT).unwrap();
            assert_eq!(row2.state, LaneState::Conflict, "{}", row2.reason);
            assert!(r.ops(&id2).iter().all(|o| o.lane_key != lane(DEPT)), "nothing of R-2 applied over a mixed lane");
        }
        assert_eq!((std::fs::read(r.dept.join("bin/a.py")).unwrap(), std::fs::read(r.dept.join("bin/b.py")).unwrap()), bytes_before, "{follow_up}: the department's bytes are never overwritten");
    }
}

/// RULINGS IQ1 — a check made outside the lock is published only for the state it checked: an
/// "update" placed by a seam between the live check and the publication (inside the check call, after
/// the states were taken) → nothing published as 검증됨 and the catch-up NOT recorded; the row stays
/// as it was, with the reason. Three updates: a target that was passed over as "already at the
/// result" changed; the pack generation changed; on the "already there" path, the checked file changed.
#[test]
fn a_check_is_published_only_for_the_state_it_checked() {
    for update in ["at-result-target", "generation", "already-there"] {
        let r = rig(&format!("iq1-{update}"));
        let id = rid("R-1");
        if update == "already-there" {
            committed_in_hq(&r, &id, Classification::General, "bin/b.py", Some((b"old-b\n", 0o644)), Some((b"new-b\n", 0o644)));
            r.put(DEPT, "bin/b.py", b"new-b\n", 0o644);
        } else {
            committed_many_in_hq(&r, &id, &[("bin/a.py", b"old-a\n", b"new-a\n"), ("bin/b.py", b"old-b\n", b"new-b\n")]);
            r.put(DEPT, "bin/a.py", b"new-a\n", 0o644); // already at R's result: passed over by the door
            r.put(DEPT, "bin/b.py", b"old-b\n", 0o644);
        }
        let d = doubles();
        {
            let (dept, update) = (r.dept.clone(), update.to_string());
            *d.during_check.borrow_mut() = Some(Box::new(move |root: &Path| {
                if root != dept.as_path() {
                    return; // the copy-check on the scratch copy
                }
                match update.as_str() {
                    "at-result-target" => std::fs::write(dept.join("bin/a.py"), b"updated-a\n").unwrap(),
                    "generation" => std::fs::write(dept.join(".pack-version"), "0.15.1\n").unwrap(),
                    _ => std::fs::write(dept.join("bin/b.py"), b"updated-b\n").unwrap(),
                }
            }));
        }
        let t = tick(&r, DEPT, &d, &fid(1));
        assert!(!t.catchup_completed, "{update}: {t:?}");
        assert!(!r.job(DEPT).initial_catchup_complete, "{update}: the catch-up is not recorded");
        assert!(t.left.iter().any(|(i, why)| *i == id && why == STATES_CHANGED), "{update}: {t:?}");
        match r.row(&id, DEPT) {
            Some(row) => {
                assert_ne!(row.state, LaneState::Verified, "{update}");
                assert_eq!(row.state, LaneState::Applied, "{update}: the door's row stays");
                assert_eq!(row.reason, STATES_CHANGED, "{update}: the reason kept on the row");
            }
            None => assert_eq!(update, "already-there", "only the already-there path has no row to keep"),
        }
    }
}

/// RULINGS IQ2 — the tombstone of IG2 is written WITH the row: R (delete a, edit b) is reverted at the
/// origin while this lane's live check runs on the "already there" path → no row AND the tombstone file
/// untouched — an install then restores the vendor file. (Before: the tombstone was written before the
/// publication was refused, and it stayed.)
#[test]
fn a_tombstone_is_written_only_with_its_row() {
    let r = rig("iq2");
    let id = rid("R-1");
    // a deleting + editing repair made in HQ
    let mut changes = Vec::new();
    let mut committed = Vec::new();
    for (rel, pre, post) in [("bin/a.py", &b"x\n"[..], None), ("bin/b.py", &b"old-b\n"[..], Some(&b"new-b\n"[..]))] {
        let live = r.hq.join(rel);
        r.put(HQ, rel, pre, 0o644);
        let pre_img = ledger::copy_preimage_in(&r.root, &id, &lane(HQ), rel, &live, T0 - 10.0).unwrap();
        let _ = std::fs::remove_file(&live);
        if let Some(b) = post {
            r.put(HQ, rel, b, 0o644);
            seal_committed_copy_in(&r.root, &id, &lane(HQ), rel, &live, T0 - 5.0).unwrap();
        }
        let post_st = TargetState::read(&live).unwrap();
        changes.push(FileChange { rel: rel.into(), kind: kind_of(&pre_img.state(), &post_st), pre: pre_img, post: post_st.clone(), diff: None });
        committed.push((rel.to_string(), post_st.canon()));
    }
    record(&r, &id, Classification::General, changes, committed, HQ);
    // the department is already at R's result (a absent, b edited) — no row, no operation, no tombstone yet
    r.put(DEPT, "bin/b.py", b"new-b\n", 0o644);
    let tomb_file = crate::repair::tombstones::path_in(&r.root, &lane(DEPT));
    let before = std::fs::read(&tomb_file).ok();
    let d = doubles();
    {
        let (root, id, dept) = (r.root.clone(), id.clone(), r.dept.clone());
        *d.during_check.borrow_mut() = Some(Box::new(move |checked: &Path| {
            if checked == dept.as_path() {
                let mut rec = ledger::read_record_in(&root, &id).unwrap().unwrap();
                rec.state = RepairState::Reverted;
                ledger::write_record_in(&root, &rec).unwrap();
            }
        }));
    }
    tick(&r, DEPT, &d, &fid(1));
    assert!(r.row(&id, DEPT).is_none(), "no row");
    assert_eq!(std::fs::read(&tomb_file).ok(), before, "the tombstone file is not changed");
    let ts = crate::repair::tombstones::load_in(&r.root, &lane(DEPT)).unwrap();
    assert!(ts.is_empty(), "{ts:?}");
    let with = crate::pack::UpdateSources::none().with_tombstones(ts);
    crate::pack::install_into_with(r.dept.clone(), vec![("bin/a.py", "x\n")], false, "0.15.0", false, false, crate::pack::PackScope::Dept, None, None, &with).unwrap();
    assert!(matches!(r.live(DEPT, "bin/a.py"), TargetState::File { .. }), "an install restores the vendor file");
}

/// RULINGS IK3 — NO LOCK ACROSS A CHECK: while this lane's copy-check and its live check run (a check
/// child "parked" by the seam), the machine-wide pack-apply lock is free — a `try` of it succeeds and
/// ANOTHER lane's tick runs its pass (which needs the lock) to its end. (On the old shape the whole tick
/// held the lock: the `try` answered "held" and the other lane skipped its turn.)
#[test]
fn no_lock_is_held_across_a_check() {
    let r = std::rc::Rc::new(rig("ik3"));
    let id = rid("R-1");
    committed_in_hq(&r, &id, Classification::General, "bin/a.py", Some((b"old\n", 0o644)), Some((b"new\n", 0o644)));
    r.put(DEPT, "bin/a.py", b"old\n", 0o644);
    let lock = r.base.join(".pack-apply.lock");
    let seen = std::rc::Rc::new(Cell::new(0u32));
    let d = doubles();
    {
        let (lock, seen, rr) = (lock.clone(), seen.clone(), r.clone());
        *d.during_check.borrow_mut() = Some(Box::new(move |_root: &Path| {
            assert_eq!(crate::pack_apply_lock::try_with_apply_lock(&lock, || 7).unwrap(), Some(7), "the lock is free during a check");
            let other = tick(&rr, HQ, &doubles(), &fid(9));
            assert!(other.pass.is_some() && !other.lines.iter().any(|l| l.contains("바꾸는 중")), "{other:?}");
            seen.set(seen.get() + 1);
        }));
    }
    let t = tick(&r, DEPT, &d, &fid(1));
    assert_eq!(seen.get(), 2, "the copy-check and the live check both ran parked: {t:?}");
    assert_eq!(r.row(&id, DEPT).unwrap().state, LaneState::Verified, "{t:?}");
}

/// RULINGS IG5 (contract R3) — the record changed while this lane's copy-check ran (the origin
/// reverted R, or committed it again with another result): the decision is DROPPED — nothing is
/// written in this lane (no door operation, no row), the reason is said; the next tick decides anew.
#[test]
fn a_record_changed_during_the_copy_check_drops_the_decision() {
    for change in ["reverted", "re-committed"] {
        let r = rig(&format!("ig5-{change}"));
        let id = rid("R-1");
        committed_in_hq(&r, &id, Classification::General, "bin/a.py", Some((b"old\n", 0o644)), Some((b"new\n", 0o644)));
        r.put(DEPT, "bin/a.py", b"old\n", 0o644);
        let d = doubles();
        {
            let (root, id, change) = (r.root.clone(), id.clone(), change.to_string());
            *d.during_check.borrow_mut() = Some(Box::new(move |_root: &Path| {
                let mut rec = ledger::read_record_in(&root, &id).unwrap().unwrap();
                if change == "reverted" {
                    rec.state = RepairState::Reverted;
                } else {
                    rec.committed = vec![("bin/a.py".into(), TargetState::File { sha256: "1".repeat(64), mode: 0o644 }.canon())];
                }
                ledger::write_record_in(&root, &rec).unwrap();
            }));
        }
        let t = tick(&r, DEPT, &d, &fid(1));
        assert_eq!(std::fs::read(r.dept.join("bin/a.py")).unwrap(), b"old\n", "{change}: nothing applied");
        assert!(r.row(&id, DEPT).is_none(), "{change}: no row written");
        assert!(r.ops(&id).is_empty(), "{change}: no door operation");
        assert!(t.left.iter().any(|(i, why)| *i == id && why == RECORD_CHANGED), "{change}: {t:?}");
    }
}

/// IG5: the record changed while the LIVE check ran (after the door operations and the 적용됨 row):
/// the 검증됨 row is not published — the row stays 적용됨 with its probation bundle.
#[test]
fn a_record_changed_during_the_live_check_publishes_nothing() {
    let r = rig("ig5-live");
    let id = rid("R-1");
    committed_in_hq(&r, &id, Classification::General, "bin/a.py", Some((b"old\n", 0o644)), Some((b"new\n", 0o644)));
    r.put(DEPT, "bin/a.py", b"old\n", 0o644);
    let d = doubles();
    {
        let (root, id, dept) = (r.root.clone(), id.clone(), r.dept.clone());
        *d.during_check.borrow_mut() = Some(Box::new(move |root_checked: &Path| {
            if root_checked == dept.as_path() {
                let mut rec = ledger::read_record_in(&root, &id).unwrap().unwrap();
                rec.state = RepairState::Reverted;
                ledger::write_record_in(&root, &rec).unwrap();
            }
        }));
    }
    let t = tick(&r, DEPT, &d, &fid(1));
    let row = r.row(&id, DEPT).unwrap();
    assert_eq!(row.state, LaneState::Applied, "{t:?}");
    assert!(row.probation.is_some());
    assert!(t.left.iter().any(|(i, why)| *i == id && why == RECORD_CHANGED), "{t:?}");
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
    // REWRITTEN under RULINGS IV1 — old expectation, quoted: `assert_eq!(r.row(&id, DEPT).unwrap().state,
    // LaneState::Pending)` (a new 대기 row written in the same tick). The generation is part of the
    // decision's stamp: it changed, so NOTHING is written for that decision this tick (the reason said)
    assert!(r.row(&id, DEPT).is_none(), "{t:?}");
    assert!(t.left.iter().any(|(i, why)| *i == id && why == STATES_CHANGED), "{t:?}");
    assert!(!t.catchup_completed);
    // the next tick decides anew, bound to the new generation: the edit is applied there
    tick(&r, DEPT, &doubles(), &fid(1));
    assert_eq!(std::fs::read(r.dept.join("bin/a.py")).unwrap(), b"new\n");
    assert_eq!(r.row(&id, DEPT).unwrap().state, LaneState::Verified);
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
        if let Some(f) = d.during_check.borrow().as_ref() {
            f(root);
        }
        let ok = matches!(TargetState::read(&root.join(rel)), Ok(TargetState::File { .. })) && d.pass.get();
        Ok(CheckOutcome { result: if ok { CheckResult::Pass } else { CheckResult::Fail("x".into()) }, ms: 1 })
    };
    let classify = |_: &str| -> Result<Classification, RepairError> { Ok(d.class.get()) };
    // the hook unit's double (IZ2): the guard first (a unit with no slow preparation), then the step
    let hooks = |id: &RepairId, l: &LaneKey, step: HookStep, guard: &dyn Fn() -> Result<(), String>| -> Result<apply::HookUnitEnd, RepairError> {
        if let Some(u) = d.unit.borrow().as_ref() {
            return u(id, l, step, guard);
        }
        if let Err(why) = guard() {
            return Ok(apply::HookUnitEnd::Refused(why));
        }
        d.hook_calls.set(d.hook_calls.get() + 1);
        if d.hooks_ok.get() {
            Ok(apply::HookUnitEnd::Ran { op: "op-test".into(), lines: vec!["Stop:x.sh".into()], result: Ok(ReconcileReport { hooks: Vec::new() }) })
        } else {
            Err(RepairError::Io("등록기가 실패했습니다(시험)".into()))
        }
    };
    let bytes = |_: &RepairRecord, _: &str| -> Result<PathBuf, String> { Err("none".into()) };
    let lock_file = r.base.join(".pack-apply.lock");
    let sources = sources_of(d);
    let env = LaneEnv {
        op: OpEnv { root: &r.root, pack: &r.dept, lane: &l, now: T0, form: ApplyForm::Exchange, faults: &NoFaults, own_check: &|_, _| None },
        check: &check,
        reference: &|_, _| Ok(CheckOutcome { result: CheckResult::Pass, ms: d.reference_ms.get() }),
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

// ───────────────────────────── RULINGS IV2 / IZ4 — the census of the propagation's writes ─────────────────────────────

/// Every write PRIMITIVE and every allowed writer's boundary in `propagate.rs`'s production text (and
/// in its verbs, `cysd/repair.rs` `propagate_verbs`, which hold none), per function, each classified:
/// the four write units of a REPAIR DECISION (RULINGS IZ1 / JA4) — (a) a door operation + the
/// tombstone of the deletion it made, (b) the hook unit (inside T6's hook step), (c) a row's
/// publication + its tombstone completion, (d) the catch-up record — or a write with its own row and
/// contract: the instance pass, the reason note, IO1's exception publication, the committed copy, the
/// lane's own scratch, a wrapper's own boundary. A new primitive or boundary without a row fails
/// `every_write_site_of_the_propagation_is_listed`. (fn, primitive / boundary, count, unit, why)
const WRITE_SITES: &[(&str, &str, usize, &str, &str)] = &[
    ("changed_midway", "write_row_if(", 1, "IO1 exception publication", "inside the door phase's lock; reports what this lane already changed — not validated against R (IO1)"),
    ("copy_checked", "create_dir_all(", 1, "the lane's own scratch", "the copy-check's scratch: not shared, not the pack, removed on every exit"),
    ("copy_checked", "write_new(", 1, "the lane's own scratch", "as above"),
    ("copy_checked_bytes_of", "create_dir_all(", 1, "the lane's own scratch", "as above"),
    ("copy_checked_bytes_of", "write_new(", 1, "the lane's own scratch", "as above"),
    ("door_phase", "record_in(", 1, "unit (a)", "the tombstone of the deletion the door operation just made — one fact with it"),
    ("door_phase", "run_op(", 1, "unit (a)", "the door operation (T6's door re-verifies the target and the generation at the act); the stamp validated once before it"),
    ("door_phase", "write_row_if(", 1, "unit (c)", "the 적용됨 row, after the stamp held, inside the door phase's lock"),
    ("drop", "remove_dir_all(", 1, "the lane's own scratch", "`Scratch::drop`"),
    ("ensure_tombstones", "record_in(", 1, "unit (c)", "the tombstone completion of a row's publication (IQ2) — called from `publish` and the door phase's 적용됨 row"),
    ("full_reset", "update_row(", 1, "the instance pass", "I8 step 2 — the pass reads and writes inside ONE tried lock"),
    ("full_reset", "write_job_state(", 2, "the instance pass", "I8 steps 1 and 4"),
    ("hook_step", "(env.hooks)(", 1, "unit (b)", "inside T6's hook step — its write-ahead record, tombstone and registrar run are classified by T6's own write census"),
    ("note_reason", "update_row(", 1, "the reason note", "inside the tried lock; the reason text only (IK2); validated by no stamp by design"),
    ("publish", "write_row_if(", 1, "unit (c)", "every row the tick and `recheck` publish, after the stamp held (IV1 / IQ1)"),
    ("record_hook_trace", "write_row_if(", 1, "unit (b)", "JA1: the trace of a hook unit that BEGAN writing — inside the unit's own lock, no stamp (it records what THIS lane changed)"),
    ("replace_pack_instance", ".create_new(true)", 1, "the instance pass", "I8 step 3: the new id created NEW"),
    ("replace_pack_instance", ".write(true)", 1, "the instance pass", "the same open"),
    ("replace_pack_instance", "remove_file(", 1, "the instance pass", "I8 step 3: a different / malformed id unlinked"),
    ("replace_pack_instance", "write_all(", 1, "the instance pass", "the id line"),
    ("seal_committed_copy_in", "create_dir(", 2, "the committed copy", "at the ORIGIN's commit, in R's vault (not a pack); HJ2 validates the copy against the verified states before the record is published"),
    ("seal_committed_copy_in", "create_dir_all(", 1, "the committed copy", "as above"),
    ("seal_committed_copy_in", "write_new(", 2, "the committed copy", "as above"),
    ("sweep_own_scratch", "remove_dir_all(", 1, "the lane's own scratch", "a dead process's scratch of THIS lane, by its exact name"),
    ("tick_body", "write_job_state(", 1, "unit (d)", "the catch-up record — the pass's generation and instance measured again inside the lock (IZ3)"),
    ("update_row", "write_lane_durable_in(", 1, "a wrapper's boundary", "the row primitive of the instance pass and the reason note"),
    ("verify_only", "update_row(", 1, "the instance pass", "L5 (a): rows back to 대기"),
    ("verify_only", "write_job_state(", 1, "the instance pass", "the recorded directory / generation"),
    ("write_job_state", "write_json_durable(", 1, "a wrapper's boundary", "the job-state primitive of the instance pass and unit (d)"),
    ("write_row_if", "write_lane_durable_in(", 1, "a wrapper's boundary", "the row primitive of units (c), IO1's publication (compare-and-write under the per-repair mutex)"),
];

/// The PRIMITIVES a durable write would use (`std::fs::write`, `File::create`, `OpenOptions` in write /
/// create / truncate / append mode, copy, rename, remove, create_dir, set_permissions, links) and the
/// allowed writers' boundaries (the census's vocabulary).
const WRITE_VOCABULARY: &[&str] = &[
    "fs::write(", "File::create(", ".write(true)", ".create(true)", ".create_new(true)", ".append(true)", ".truncate(true)", "fs::copy(", "rename(",
    "remove_file(", "remove_dir(", "remove_dir_all(", "create_dir(", "create_dir_all(", "set_permissions(", "hard_link(", "symlink(",
    "write_row_if(", "update_row(", "write_lane_durable_in(", "write_job_state(", "write_json_durable(", "write_durable(", "record_in(",
    "remove_for_repair_in(", "run_op(", "(env.hooks)(", "write_new(", "write_all(", "write_in(",
];

/// (fn, word) → count over a production text; a line belongs to the last `fn` above it; comment lines
/// and the declaration's own name are skipped; a word that starts with a letter counts only when it is
/// not the tail of a longer identifier.
fn write_sites_of(prod: &str) -> std::collections::BTreeMap<(String, String), usize> {
    let mut m = std::collections::BTreeMap::new();
    let mut cur = "<module>".to_string();
    for line in prod.lines() {
        let t = line.trim_start();
        if t.starts_with("//") {
            continue;
        }
        let code = match line.find("//") {
            Some(i) if line[..i].matches('"').count() % 2 == 0 => &line[..i],
            _ => line,
        };
        let mut body = code;
        let decl = t.trim_start_matches("pub(crate) ").trim_start_matches("pub ").trim_start_matches("const ");
        if let Some(rest) = decl.strip_prefix("fn ") {
            cur = rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
            match code.find('{') {
                Some(o) => body = &code[o + 1..],
                None => continue,
            }
        }
        for w in WRITE_VOCABULARY {
            let n = body
                .match_indices(w)
                .filter(|(i, _)| !w.starts_with(|x: char| x.is_ascii_alphabetic()) || !body[..*i].chars().last().is_some_and(|p| p.is_ascii_alphanumeric() || p == '_'))
                .count();
            if n > 0 {
                *m.entry((cur.clone(), w.to_string())).or_insert(0) += n;
            }
        }
    }
    m
}

fn write_site_violations(prod: &str, verbs: &str) -> Vec<String> {
    let counts = write_sites_of(prod);
    let mut v = Vec::new();
    for ((f, w), n) in &counts {
        match WRITE_SITES.iter().find(|r| r.0 == f && r.1 == w) {
            None => v.push(format!("{f}: {n} × {w} — a write with no row (which unit? which stamp?)")),
            Some(r) if r.2 != *n => v.push(format!("{f}: {n} × {w}, the census lists {} — classify the new one", r.2)),
            Some(_) => {}
        }
    }
    for r in WRITE_SITES {
        if !counts.contains_key(&(r.0.to_string(), r.1.to_string())) {
            v.push(format!("{}: {} listed but not found — remove the row", r.0, r.1));
        }
    }
    // the verbs write nothing themselves (they call `recheck_lane_in` / `propagation_tick_in`)
    for ((f, w), n) in write_sites_of(verbs) {
        v.push(format!("propagate_verbs::{f}: {n} × {w} — the verbs hold no write of their own"));
    }
    v
}

fn propagate_production_text() -> String {
    let src = include_str!("propagate.rs");
    src[..src.find("#[cfg(test)]\n#[path").unwrap_or(src.len())].to_string()
}

fn propagate_verbs_text() -> String {
    let src = include_str!("../bin/cysd/repair.rs");
    let a = src.find("pub(crate) mod propagate_verbs {").expect("the verbs module");
    let b = src[a..].find("\nmod apply_verbs {").map(|o| a + o).expect("its end");
    src[a..b].to_string()
}

/// RULINGS IV2 / IZ4 — every write primitive and writer boundary of the propagation is listed with its
/// unit; the census CAN fail: mutation controls — a new `std::fs::write` in a new function, a new
/// `OpenOptions … create`, one more row write in a listed function, a write inside the verbs — each
/// turns it red.
#[test]
fn every_write_site_of_the_propagation_is_listed() {
    let prod = propagate_production_text();
    let verbs = propagate_verbs_text();
    let v = write_site_violations(&prod, &verbs);
    assert!(v.is_empty(), "{v:#?}");
    let with = |code: &str| format!("{prod}\nfn census_mutant(p: &Path) {{\n    {code}\n}}\n");
    assert!(!write_site_violations(&with("std::fs::write(p, b\"x\").unwrap();"), &verbs).is_empty(), "a new std::fs::write");
    assert!(!write_site_violations(&with("let _ = std::fs::OpenOptions::new().append(true).create(true).open(p);"), &verbs).is_empty(), "a new OpenOptions … create");
    assert!(!write_site_violations(&with("let _ = std::fs::File::create(p);"), &verbs).is_empty(), "a new File::create");
    let more = prod.replacen("fn note_reason(env: &LaneEnv<'_>, id: &RepairId, why: &str) {", "fn note_reason(env: &LaneEnv<'_>, id: &RepairId, why: &str) {\n    let _ = std::fs::write(env.op.root.join(\"x\"), why);", 1);
    assert_ne!(more, prod, "the mutation applied");
    assert!(!write_site_violations(&more, &verbs).is_empty(), "one more write in a listed function");
    assert!(!write_site_violations(&prod, &format!("{verbs}\nfn v(p: &Path) {{ let _ = std::fs::write(p, b\"x\"); }}")).is_empty(), "a write inside the verbs");
    assert!(WRITE_SITES.iter().all(|r| !r.3.is_empty() && !r.4.is_empty()), "every row names its unit and why");
}

// ───────────────────────────── RULINGS IV1 / IV3 / IV4 — the stamp at every write ─────────────────────────────

fn clear_seams() {
    AFTER_DECISION.with(|h| *h.borrow_mut() = None);
    AFTER_DOOR_PHASE.with(|h| *h.borrow_mut() = None);
    BEFORE_CATCHUP.with(|h| *h.borrow_mut() = None);
    AFTER_PASS.with(|h| *h.borrow_mut() = None);
}

/// RULINGS IZ3 — the tick's stamp for completeness and the catch-up record is the generation and
/// instance the INSTANCE PASS used inside its lock: a 검증됨 row and an interrupted catch-up (its mark
/// not written); at the next tick an update lands right after the pass released its lock (a new
/// generation, new bytes). The row is taken as complete by the fast path, but the catch-up is NOT
/// recorded: the pass's generation is no longer the pack's. (With the stamp read again after the pass,
/// the updated generation became the base and the catch-up was recorded for a generation nothing was
/// verified at.)
#[test]
fn the_catchup_is_judged_on_what_the_pass_used() {
    let r = rig("iz3");
    let id = rid("R-1");
    committed_in_hq(&r, &id, Classification::General, "bin/a.py", Some((b"old\n", 0o644)), Some((b"new\n", 0o644)));
    r.put(DEPT, "bin/a.py", b"old\n", 0o644);
    tick(&r, DEPT, &doubles(), &fid(1));
    assert_eq!(r.row(&id, DEPT).unwrap().state, LaneState::Verified);
    let mut st = r.job(DEPT);
    st.initial_catchup_complete = false; // the catch-up's mark was never written (a stop right before it)
    write_job_state(&r.root, &st).unwrap();
    let dept = r.dept.clone();
    AFTER_PASS.with(|h| {
        *h.borrow_mut() = Some(Box::new(move |_: &Path| {
            std::fs::write(dept.join("bin/a.py"), b"updated\n").unwrap();
            std::fs::write(dept.join(".pack-version"), "0.15.1\n").unwrap();
        }))
    });
    let t = tick(&r, DEPT, &doubles(), &fid(1));
    clear_seams();
    assert!(!t.catchup_completed, "{t:?}");
    assert!(!r.job(DEPT).initial_catchup_complete, "nothing was verified at the new generation");
    assert!(t.lines.iter().any(|l| l.contains(STATES_CHANGED)), "{t:?}");
}

/// IV1 (M1) — an update placed RIGHT AFTER the decision (the stamp taken) is never taken as the
/// baseline: the "already there" path → no 검증됨, no catch-up; the apply path, an update right after
/// the door phase released its lock → the 적용됨 row stays, no 검증됨, no catch-up. (With a stamp taken
/// at the start of the check, the updated bytes became the baseline and 검증됨 was published.)
#[test]
fn an_update_right_after_the_decision_is_never_the_baseline() {
    for path in ["already-there", "after-the-door"] {
        let r = rig(&format!("iv1-{path}"));
        let id = rid("R-1");
        committed_in_hq(&r, &id, Classification::General, "bin/a.py", Some((b"old\n", 0o644)), Some((b"new\n", 0o644)));
        r.put(DEPT, "bin/a.py", if path == "already-there" { b"new\n" } else { b"old\n" }, 0o644);
        let dept = r.dept.clone();
        let update: Box<dyn Fn(&Path)> = Box::new(move |_: &Path| {
            std::fs::write(dept.join("bin/a.py"), b"updated\n").unwrap();
            std::fs::write(dept.join(".pack-version"), "0.15.1\n").unwrap();
        });
        if path == "already-there" {
            AFTER_DECISION.with(|h| *h.borrow_mut() = Some(update));
        } else {
            AFTER_DOOR_PHASE.with(|h| *h.borrow_mut() = Some(update));
        }
        let t = tick(&r, DEPT, &doubles(), &fid(1));
        clear_seams();
        assert!(!t.catchup_completed, "{path}: {t:?}");
        assert!(!t.rows.iter().any(|(_, st, _)| *st == LaneState::Verified), "{path}: {t:?}");
        assert!(t.left.iter().any(|(i, why)| *i == id && why == STATES_CHANGED), "{path}: {t:?}");
        match r.row(&id, DEPT) {
            Some(row) => assert_eq!((path, row.state), ("after-the-door", LaneState::Applied)),
            None => assert_eq!(path, "already-there"),
        }
    }
}

/// IV3 (M2) — the catch-up record is written inside the tried lock after the generation and instance
/// its completeness was judged on were measured again: an update between the last publication and the
/// catch-up → not recorded this tick (the gate stays held by it); the next tick decides anew.
#[test]
fn an_update_before_the_catchup_record_is_not_recorded() {
    let r = rig("iv3");
    let id = rid("R-1");
    committed_in_hq(&r, &id, Classification::General, "bin/a.py", Some((b"old\n", 0o644)), Some((b"new\n", 0o644)));
    r.put(DEPT, "bin/a.py", b"old\n", 0o644);
    let dept = r.dept.clone();
    BEFORE_CATCHUP.with(|h| *h.borrow_mut() = Some(Box::new(move |_: &Path| std::fs::write(dept.join(".pack-version"), "0.15.1\n").unwrap())));
    let t = tick(&r, DEPT, &doubles(), &fid(1));
    clear_seams();
    assert_eq!(r.row(&id, DEPT).unwrap().state, LaneState::Verified, "the row was published before the update");
    assert!(!t.catchup_completed, "{t:?}");
    assert!(!r.job(DEPT).initial_catchup_complete, "the catch-up is not recorded");
    assert!(t.lines.iter().any(|l| l.contains(STATES_CHANGED)), "{t:?}");
}

/// IV4 (M3) — `recheck` follows the same stamp and the same publication as the tick: (1) an update
/// between its check and its row → nothing written (the row stays 충돌, the merge lock kept), the verb
/// refused with the reason; (2) R reverted right after the decision, before the hook step of a hook
/// repair → the hook step does not run (no registrar call, no tombstone), nothing written.
#[test]
fn recheck_uses_the_same_stamp_and_publication_as_the_tick() {
    // (1) a file repair, an update during recheck's check
    let r = rig("iv4a");
    let id = rid("R-1");
    committed_in_hq(&r, &id, Classification::General, "bin/a.py", Some((b"old\n", 0o644)), Some((b"new\n", 0o644)));
    r.put(DEPT, "bin/a.py", b"mine\n", 0o644);
    tick(&r, DEPT, &doubles(), &fid(1));
    conflicted_with_lock(&r, &id, b"mine\n");
    let d = doubles();
    {
        let dept = r.dept.clone();
        *d.during_check.borrow_mut() = Some(Box::new(move |_: &Path| std::fs::write(dept.join("bin/a.py"), b"updated\n").unwrap()));
    }
    match recheck(&r, &id, &d, 5, "합침") {
        Err(RepairError::Locked(m)) => assert!(m.contains(STATES_CHANGED), "{m}"),
        other => panic!("{other:?}"),
    }
    let row = r.row(&id, DEPT).unwrap();
    assert_eq!(row.state, LaneState::Conflict);
    assert!(row.merge_lock.is_some() && row.merge.is_none(), "nothing written");
    // (2) a hook-only repair, R reverted right after recheck's decision
    let r2 = rig("iv4b");
    let id2 = rid("R-2");
    record(&r2, &id2, Classification::General, vec![FileChange { rel: "settings:Stop:x.sh".into(), kind: ChangeKind::HookUnregister { event: "Stop".into(), script: "x.sh".into() }, pre: PreImage::Absent, post: TargetState::Absent, diff: None }], Vec::new(), HQ);
    let d2 = doubles();
    d2.hooks_ok.set(false); // the first tick: the lane's hook step fails → 충돌
    tick(&r2, DEPT, &d2, &fid(1));
    let mut row2 = r2.row(&id2, DEPT).unwrap();
    assert_eq!(row2.state, LaneState::Conflict);
    row2.merge_lock = Some(crate::repair::lock::new_lease(LockOwner { lane: lane(DEPT), seat: 5, incarnation: "inc".into() }, T0, None));
    ledger::write_lane_durable_in(&r2.root, &row2).unwrap();
    let d3 = doubles();
    {
        let (root, id2) = (r2.root.clone(), id2.clone());
        AFTER_DECISION.with(|h| *h.borrow_mut() = Some(Box::new(move |_: &Path| {
            let mut rec = ledger::read_record_in(&root, &id2).unwrap().unwrap();
            rec.state = RepairState::Reverted;
            ledger::write_record_in(&root, &rec).unwrap();
        })));
    }
    let tombs_before = std::fs::read(crate::repair::tombstones::path_in(&r2.root, &lane(DEPT))).ok();
    let out = recheck(&r2, &id2, &d3, 5, "");
    clear_seams();
    match out {
        Err(RepairError::Locked(m)) => assert!(m.contains(RECORD_CHANGED), "{m}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(d3.hook_calls.get(), 0, "the hook step did not run");
    assert_eq!(std::fs::read(crate::repair::tombstones::path_in(&r2.root, &lane(DEPT))).ok(), tombs_before, "no tombstone written");
    assert_eq!(r2.row(&id2, DEPT).unwrap().state, LaneState::Conflict, "nothing written");
}

// ───────────────────────────── RULINGS IZ2 / JA1 — the hook unit ─────────────────────────────

/// A hook-only general repair made in HQ (Stop:x.sh unregistered), in probation.
fn hook_repair_in_hq(r: &Rig, id: &RepairId) -> RepairRecord {
    record(r, id, Classification::General, vec![FileChange { rel: "settings:Stop:x.sh".into(), kind: ChangeKind::HookUnregister { event: "Stop".into(), script: "x.sh".into() }, pre: PreImage::Absent, post: TargetState::Absent, diff: None }], Vec::new(), HQ)
}

/// A fake registrar's report for Stop:x.sh (the shape T6's dry-run measurement reads).
fn registrar_report(settings: &Path, state: &str) -> String {
    serde_json::json!({"hooks": [{"event": "Stop", "script": "x.sh", "lifeline": false, "desired": "unregistered",
        "targets": [{"path": settings.to_string_lossy(), "state": state}]}], "errors": []}).to_string()
}

/// T6's REAL guarded hook unit over this rig's root and DEPT's pack with a fake registrar; `during_dry`
/// runs inside the registrar's dry run, `during_real` inside its real run (the origin's daemon acting
/// while this lane's unit waits).
fn real_unit(r: &Rig, during_dry: Box<dyn Fn()>, during_real: Box<dyn Fn()>, calls: std::rc::Rc<std::cell::RefCell<Vec<bool>>>) -> Box<UnitDouble> {
    let (root, pack, settings) = (r.root.clone(), r.dept.clone(), r.base.join("settings.json"));
    Box::new(move |id: &RepairId, l: &LaneKey, step: HookStep, guard: &dyn Fn() -> Result<(), String>| {
        let registrar = |_: &Path, dry: bool| -> Result<String, RepairError> {
            calls.borrow_mut().push(dry);
            if dry {
                during_dry();
                Ok(registrar_report(&settings, "registered"))
            } else {
                during_real();
                Ok(registrar_report(&settings, "unregistered"))
            }
        };
        apply::hook_lines_unit_in(&root, &pack, id, l, step, &registrar, T0, guard)
    })
}

fn supersede(root: &Path, id: &RepairId) -> Box<dyn Fn()> {
    let (root, id) = (root.to_path_buf(), id.clone());
    Box::new(move || {
        let mut rec = ledger::read_record_in(&root, &id).unwrap().unwrap();
        if rec.state != RepairState::Superseded {
            rec.state = RepairState::Superseded;
            ledger::write_record_in(&root, &rec).unwrap();
        }
    })
}

/// RULINGS IZ2 — the hook unit validates AFTER the registrar's dry run (which may wait up to its
/// deadline) and the settings measurement, right before its first durable write: R superseded at the
/// origin DURING the dry run → no write-ahead record, no tombstone, no real run; `recheck` says nothing
/// was written. (Validated before the dry run, the record, the tombstone and the real run happened.)
#[test]
fn the_hook_unit_validates_after_its_dry_run() {
    let r = rig("iz2");
    let id = rid("R-1");
    hook_repair_in_hq(&r, &id);
    let d0 = doubles();
    d0.hooks_ok.set(false); // the first tick: the lane's hook step fails → 충돌 (for a recheck)
    tick(&r, DEPT, &d0, &fid(1));
    let mut row = r.row(&id, DEPT).unwrap();
    assert_eq!(row.state, LaneState::Conflict);
    row.merge_lock = Some(crate::repair::lock::new_lease(LockOwner { lane: lane(DEPT), seat: 5, incarnation: "inc".into() }, T0, None));
    ledger::write_lane_durable_in(&r.root, &row).unwrap();
    let calls = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let d = doubles();
    *d.unit.borrow_mut() = Some(real_unit(&r, supersede(&r.root, &id), Box::new(|| {}), calls.clone()));
    match recheck(&r, &id, &d, 5, "") {
        Err(RepairError::Locked(m)) => assert!(m.contains(RECORD_CHANGED) && m.contains("아무것도 쓰지 않았습니다"), "{m}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(*calls.borrow(), vec![true], "the dry run only — no real run");
    assert!(r.ops(&id).iter().all(|o| o.lane_key != lane(DEPT)), "no write-ahead record");
    assert!(crate::repair::tombstones::load_in(&r.root, &lane(DEPT)).unwrap().is_empty(), "no tombstone");
}

/// RULINGS JA1 — a hook unit that BEGAN to write leaves its trace on the row, on every call path:
/// (1) `recheck`: the unit validates, writes its record; the origin supersedes R during the real run;
/// the tombstone and the un-registration complete; the publication after it is refused → the row's
/// reason names the operation, and the verb's answer says the same (never "nothing was written");
/// (2) the tick's "already there" arm: the same → the row carries the trace (a new 대기 row) and the
/// reason says why nothing more was written.
#[test]
fn a_hook_unit_that_began_leaves_its_trace_on_the_row() {
    // (1) recheck
    let r = rig("ja1a");
    let id = rid("R-1");
    hook_repair_in_hq(&r, &id);
    let d0 = doubles();
    d0.hooks_ok.set(false);
    tick(&r, DEPT, &d0, &fid(1));
    let mut row = r.row(&id, DEPT).unwrap();
    row.merge_lock = Some(crate::repair::lock::new_lease(LockOwner { lane: lane(DEPT), seat: 5, incarnation: "inc".into() }, T0, None));
    ledger::write_lane_durable_in(&r.root, &row).unwrap();
    let calls = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let d = doubles();
    *d.unit.borrow_mut() = Some(real_unit(&r, Box::new(|| {}), supersede(&r.root, &id), calls.clone()));
    let m = match recheck(&r, &id, &d, 5, "") {
        Err(RepairError::Locked(m)) => m,
        other => panic!("{other:?}"),
    };
    assert!(m.contains("이 부서의 훅 등록을 해제했습니다(작업 ") && m.contains("Stop:x.sh") && !m.contains("아무것도 쓰지 않았습니다"), "{m}");
    assert_eq!(*calls.borrow(), vec![true, false], "the unit ran to its end");
    let op = r.ops(&id).into_iter().find(|o| o.lane_key == lane(DEPT) && o.kind == "hook-unregister").expect("its write-ahead record");
    assert!(op.finished);
    assert!(crate::repair::tombstones::load_in(&r.root, &lane(DEPT)).unwrap().iter().any(|t| t.rel == "settings:Stop:x.sh"));
    let row = r.row(&id, DEPT).unwrap();
    assert_eq!(row.state, LaneState::Conflict, "the row keeps its state");
    assert!(row.reason.contains(&format!("작업 {}", op.op)) && row.reason.contains("Stop:x.sh"), "{}", row.reason);
    // (2) the tick
    let r2 = rig("ja1b");
    let id2 = rid("R-2");
    hook_repair_in_hq(&r2, &id2);
    let calls2 = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let d2 = doubles();
    *d2.unit.borrow_mut() = Some(real_unit(&r2, Box::new(|| {}), supersede(&r2.root, &id2), calls2.clone()));
    let t = tick(&r2, DEPT, &d2, &fid(1));
    assert_eq!(*calls2.borrow(), vec![true, false]);
    let row2 = r2.row(&id2, DEPT).expect("the trace is on a row");
    assert_ne!(row2.state, LaneState::Verified, "{t:?}");
    assert!(row2.reason.starts_with("이 부서의 훅 등록을 해제했습니다(작업 ") && row2.reason.contains("그 뒤 수리 기록이 바뀌었습니다"), "{}", row2.reason);
}

/// JA1 in the door phase: a mixed repair (an edit + a hook) whose hook unit began and did not finish
/// (the registrar's real run fails) → the row is 충돌 NOW, naming the edited file and the hook
/// operation (IO1's exception publication), whatever R is now.
#[test]
fn a_hook_unit_that_failed_after_it_began_is_reported_in_the_door_phase() {
    let r = rig("ja1c");
    let id = rid("R-1");
    let mut rec = committed_in_hq(&r, &id, Classification::General, "bin/a.py", Some((b"old\n", 0o644)), Some((b"new\n", 0o644)));
    rec.files.push(FileChange { rel: "settings:Stop:x.sh".into(), kind: ChangeKind::HookUnregister { event: "Stop".into(), script: "x.sh".into() }, pre: PreImage::Absent, post: TargetState::Absent, diff: None });
    ledger::write_record_in(&r.root, &rec).unwrap();
    r.put(DEPT, "bin/a.py", b"old\n", 0o644);
    let d = doubles();
    *d.unit.borrow_mut() = Some(Box::new(|_: &RepairId, _: &LaneKey, _: HookStep, guard: &dyn Fn() -> Result<(), String>| {
        guard().map_err(RepairError::Precondition)?;
        Ok(apply::HookUnitEnd::Ran { op: "op-ja1c".into(), lines: vec!["Stop:x.sh".into()], result: Err(RepairError::CheckTimeout("등록기가 시한 안에 끝나지 않았습니다(시험)".into())) })
    }));
    tick(&r, DEPT, &d, &fid(1));
    let row = r.row(&id, DEPT).unwrap();
    assert_eq!(row.state, LaneState::Conflict);
    assert!(row.reason.contains("bin/a.py") && row.reason.contains("작업 op-ja1c") && row.reason.contains("마치지 못했습니다"), "{}", row.reason);
    assert_eq!(std::fs::read(r.dept.join("bin/a.py")).unwrap(), b"new\n", "what was changed stays (no undo — IO1)");
}

/// JA1 — a hook unit INTERRUPTED after its write-ahead record (a stop): T6's recovery completes the hook
/// operation from its record, and the next tick's evidence path (IG3: this lane's own completed
/// operations for R) writes the row WITH its probation bundle — not the closed "nothing replaced here".
/// (The tick runs the hook unit once more — un-registration is idempotent — and its operation is
/// evidence too.)
#[test]
fn an_interrupted_hook_unit_is_completed_by_recovery_and_its_row_has_a_probation() {
    let r = rig("ja1d");
    let id = rid("R-1");
    hook_repair_in_hq(&r, &id);
    // the stop: an unfinished hook record of THIS lane for R, written by hand as T6's own test does
    let op = crate::repair::wal::OpRecord {
        schema: 1, op: crate::repair::wal::new_op_id(T0 - 1.0, "hook-unregister"), repair: id.clone(), lane_key: lane(DEPT),
        kind: "hook-unregister".into(), form: "hook".into(), rel: String::new(), pre_existed: false,
        expected: String::new(), incoming: String::new(), pack_generation: Some(apply::pack_generation(&r.dept)), slot: None, slot_identity: None,
        live_identity: None, incoming_copy: None, incoming_copy_identity: None, step: "recorded".into(),
        hooks: vec![crate::repair::wal::HookLine { event: "Stop".into(), script: "x.sh".into(), targets: Vec::new() }],
        finished: false, outcome: None, note: String::new(), created_at: T0 - 1.0, updated_at: T0 - 1.0, notice: None,
    };
    crate::repair::wal::write_in(&r.root, &op).unwrap();
    // T6's recovery completes it (the job runs it before the propagation)
    let settings = r.base.join("settings.json");
    let ok = |_: &Path, _: bool| -> Result<String, RepairError> { Ok(registrar_report(&settings, "unregistered")) };
    assert_eq!(apply::resume_hook_steps_in(&r.root, &r.dept, &lane(DEPT), &ok, &apply::no_repair_lock, T0), 1);
    let done = crate::repair::wal::list_in(&r.root, &id).0.into_iter().find(|o| o.op == op.op).unwrap();
    assert!(done.finished && done.outcome.as_deref() == Some("applied"), "{done:?}");
    // the next tick
    let calls = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let d = doubles();
    *d.unit.borrow_mut() = Some(real_unit(&r, Box::new(|| {}), Box::new(|| {}), calls.clone()));
    tick(&r, DEPT, &d, &fid(1));
    let row = r.row(&id, DEPT).unwrap();
    assert_eq!(row.state, LaneState::Verified, "{}", row.reason);
    let p = row.probation.unwrap();
    assert!(p.closed.is_none(), "an OPEN probation — the lane applied R itself: {:?}", p.closed);
}

// ───────────────────────────── RULINGS JA2 / JA3 — what a row and the catch-up mean ─────────────────────────────

/// JA2 (a meaning, stated): a published 검증됨 row records the observation made right before it; a
/// target edited by another writer afterwards is not measured again while the generation and instance
/// stay — the row stays 검증됨; at a generation change the verify-only pass demotes it (and the lane's
/// next decision is a 충돌 for the foreign bytes).
#[test]
fn a_row_records_its_observation_until_the_generation_changes() {
    let r = rig("ja2");
    let id = rid("R-1");
    committed_in_hq(&r, &id, Classification::General, "bin/a.py", Some((b"old\n", 0o644)), Some((b"new\n", 0o644)));
    r.put(DEPT, "bin/a.py", b"new\n", 0o644);
    tick(&r, DEPT, &doubles(), &fid(1));
    assert_eq!(r.row(&id, DEPT).unwrap().state, LaneState::Verified);
    std::fs::write(r.dept.join("bin/a.py"), b"edited by someone\n").unwrap();
    tick(&r, DEPT, &doubles(), &fid(1));
    assert_eq!(r.row(&id, DEPT).unwrap().state, LaneState::Verified, "not measured again: same generation and instance");
    std::fs::write(r.dept.join(".pack-version"), "0.15.1\n").unwrap();
    let t = tick(&r, DEPT, &doubles(), &fid(1));
    assert!(matches!(t.pass, Some(PassOutcome::VerifyOnly { demoted: 1 })), "{t:?}");
    assert_eq!(r.row(&id, DEPT).unwrap().state, LaneState::Conflict, "the edit is now the lane's 충돌");
}

/// JA3 (a meaning, stated): the catch-up is the first processing of the repairs observed for this
/// pack instance; a re-commit after it does not cancel it — the gate stays OPEN, the row is demoted by
/// P2 and decided anew.
#[test]
fn a_recommit_after_the_catchup_leaves_the_gate_open_and_demotes_the_row() {
    let r = rig("ja3");
    let id = rid("R-1");
    committed_in_hq(&r, &id, Classification::General, "bin/a.py", Some((b"old\n", 0o644)), Some((b"new\n", 0o644)));
    r.put(DEPT, "bin/a.py", b"old\n", 0o644);
    tick(&r, DEPT, &doubles(), &fid(1));
    assert_eq!(gate_verdict_in(&r.root, &lane(DEPT)), GateVerdict::Open);
    // R committed again with another result (the HQ file and its sealed copy)
    r.put(HQ, "bin/a.py", b"newer\n", 0o644);
    let mut rec = ledger::read_record_in(&r.root, &id).unwrap().unwrap();
    let st = TargetState::read(&r.hq.join("bin/a.py")).unwrap();
    seal_committed_copy_in(&r.root, &id, &lane(HQ), "bin/a.py", &r.hq.join("bin/a.py"), T0).unwrap();
    rec.committed = vec![("bin/a.py".into(), st.canon())];
    ledger::write_record_in(&r.root, &rec).unwrap();
    assert_eq!(gate_verdict_in(&r.root, &lane(DEPT)), GateVerdict::Open, "the catch-up is not cancelled");
    tick(&r, DEPT, &doubles(), &fid(1));
    let row = r.row(&id, DEPT).unwrap();
    assert_eq!(row.applied, rec.committed, "P2: demoted and decided anew — the new result applied");
    assert_eq!(std::fs::read(r.dept.join("bin/a.py")).unwrap(), b"newer\n");
    assert_eq!(gate_verdict_in(&r.root, &lane(DEPT)), GateVerdict::Open);
}
