//! Department propagation (track T7b · mechanism M14's propagation part · contract C11 · design
//! §4-14, §4-9): every lane's daemon applies, at its own job tick, the in-force repairs to its OWN
//! pack and keeps its OWN row of each (K1 — no daemon writes another lane's file); the pack-instance
//! pass (I8 / K2 / K7 / L1 / L5 / O4 / O5); the assignment gate's local reader (I1 / I7 / I8 / P5 /
//! R7); a lane's catch-up status (H3 / H5); the `recheck` verb's lane re-evaluation (H8); the one
//! summary line of lanes left in 충돌 (11.7 · RULINGS HH7 — a pull, never a push).
//!
//! What this module never does (fatal class 1 — RULINGS HH (2)): it sends nothing, queues nothing,
//! injects nothing; a lane's conflict is a row state read from the ledger. Every write of a live pack
//! file goes through T6's engine (`apply::run_op` — the one door); the pass's only write inside a
//! pack is the id file `<pack>/.repair-pack-id` (I8). Every comparison is of STATES (U2).

use super::apply::{self, ApplyOutcome, Bound, CheckFn, HookStep, HookStepFn, OpEnv, OpSpec, Recorded, ReferenceFn, RepairLock, Source};
use super::durable::{self, RecordRead};
use super::ledger::{
    self, DirIdentity, GateVerdict, HoldCause, JobState, LaneRecord, MergeRecord, PackGeneration, RegisteredLanes,
    RepairRecord,
};
use super::platform::ApplyForm;
use super::rehearsal::CheckResult;
use super::types::{Classification, LaneKey, LaneState, RepairError, RepairId, RepairState, TargetState};
use std::path::{Path, PathBuf};

// ───────────────────────────── reads with their answers (FG1 · GK5 · O5) ─────────────────────────────

/// One read of a small file of this mechanism: a VALUE; ABSENT (the name does not exist); MALFORMED
/// (present but not a valid instance — counts as "different", O5); a READ ERROR (a transient failure
/// — the pass is skipped for the tick, O5; the gate answers HELD, I7 (4)).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileRead<T> {
    Value(T),
    Absent,
    Malformed(String),
    ReadError(String),
}

/// The job state's size limit (a JobState is a few hundred bytes).
pub const JOB_STATE_MAX_BYTES: u64 = 1 << 20;
/// O5: the id file is a regular file of at most 128 bytes holding exactly one id line.
pub const PACK_INSTANCE_MAX_BYTES: u64 = 128;

/// `<root>/lanes/<lane>.job.json` (the layout of `paths::job_state_path`).
pub fn job_state_path_in(root: &Path, lane: &LaneKey) -> PathBuf {
    root.join("lanes").join(format!("{}.job.json", lane.0))
}

/// The shape of an entry, read with `lstat` (no link followed): ABSENT, a read error, not a regular
/// file / too large (MALFORMED), or the size of a regular file.
fn shape(p: &Path, max: u64) -> FileRead<u64> {
    match std::fs::symlink_metadata(p) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => FileRead::Absent,
        Err(e) => FileRead::ReadError(format!("{}: {e}", p.display())),
        Ok(m) if !m.file_type().is_file() => FileRead::Malformed(format!("{}: 일반 파일이 아닙니다", p.display())),
        Ok(m) if m.len() > max => FileRead::Malformed(format!("{}: {}바이트 — 한도 {max}바이트를 넘습니다", p.display(), m.len())),
        Ok(m) => FileRead::Value(m.len()),
    }
}

/// The bytes of a small file through THE bounded reader (GK5): no link followed, non-blocking, a
/// regular file, capped.
fn small_bytes(p: &Path, max: u64) -> FileRead<Vec<u8>> {
    match shape(p, max) {
        FileRead::Value(_) => {}
        FileRead::Absent => return FileRead::Absent,
        FileRead::Malformed(w) => return FileRead::Malformed(w),
        FileRead::ReadError(w) => return FileRead::ReadError(w),
    }
    match durable::read_record_bytes(p) {
        RecordRead::Bytes(b) if b.len() as u64 <= max => FileRead::Value(b),
        RecordRead::Bytes(b) => FileRead::Malformed(format!("{}: {}바이트 — 한도를 넘습니다", p.display(), b.len())),
        RecordRead::NotFound => FileRead::Absent,
        RecordRead::Unreadable(why) => FileRead::ReadError(why),
    }
}

/// The lane's job state with its four answers (one reader for the pass, the gate and the catch-up).
pub fn read_job_state_checked(root: &Path, lane: &LaneKey) -> FileRead<JobState> {
    let p = job_state_path_in(root, lane);
    let bytes = match small_bytes(&p, JOB_STATE_MAX_BYTES) {
        FileRead::Value(b) => b,
        FileRead::Absent => return FileRead::Absent,
        FileRead::Malformed(w) => return FileRead::Malformed(w),
        FileRead::ReadError(w) => return FileRead::ReadError(w),
    };
    let v: serde_json::Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(e) => return FileRead::Malformed(format!("{}: {e}", p.display())),
    };
    match v.get("schema").and_then(|s| s.as_u64()) {
        Some(s) if s <= u64::from(super::REPAIR_SCHEMA) => {}
        other => return FileRead::Malformed(format!("{}: schema {other:?}", p.display())),
    }
    match serde_json::from_value::<JobState>(v) {
        Ok(st) if st.lane == *lane => FileRead::Value(st),
        Ok(st) => FileRead::Malformed(format!("{}: 다른 부서({})의 상태입니다", p.display(), st.lane.0)),
        Err(e) => FileRead::Malformed(format!("{}: {e}", p.display())),
    }
}

/// Whether `s` is a pack-instance id: 64 lowercase hex characters.
pub fn is_pack_instance_id(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// `<pack>/.repair-pack-id` with its four answers (O5: anything but a regular file of at most 128
/// bytes holding exactly one id line is MALFORMED; a read error is transient).
pub fn read_pack_instance(pack: &Path) -> FileRead<String> {
    let p = super::paths::pack_instance_path(pack);
    let bytes = match small_bytes(&p, PACK_INSTANCE_MAX_BYTES) {
        FileRead::Value(b) => b,
        FileRead::Absent => return FileRead::Absent,
        FileRead::Malformed(w) => return FileRead::Malformed(w),
        FileRead::ReadError(w) => return FileRead::ReadError(w),
    };
    let text = String::from_utf8_lossy(&bytes);
    let line = text.strip_suffix('\n').unwrap_or(&text);
    if is_pack_instance_id(line) {
        FileRead::Value(line.to_string())
    } else {
        FileRead::Malformed(format!("{}: 한 줄의 식별자가 아닙니다", p.display()))
    }
}

/// The pack directory's identity (L1): volume + inode of the directory itself.
pub fn dir_identity(pack: &Path) -> Result<DirIdentity, String> {
    match super::exchange::entry_identity(pack) {
        Ok((volume_uuid, inode)) => Ok(DirIdentity { volume_uuid, inode }),
        Err(e) => Err(format!("{e:?}")),
    }
}

fn fresh_job_state(lane: &LaneKey) -> JobState {
    JobState {
        schema: super::REPAIR_SCHEMA,
        lane: lane.clone(),
        last_incarnation: String::new(),
        last_probation_check: 0.0,
        last_sweep: 0.0,
        initial_catchup_complete: false,
        pack_instance: None,
        pack_dir: None,
        pack_dir_identity: None,
        pack_generation: None,
    }
}

fn write_job_state(root: &Path, st: &JobState) -> Result<(), RepairError> {
    durable::write_json_durable(&job_state_path_in(root, &st.lane), st)
}

// ───────────────────────────── the in-force set of a lane ─────────────────────────────

/// A repair is in force while it is `시험 기간` or `사용 중`.
pub fn in_force(rec: &RepairRecord) -> bool {
    matches!(rec.state, RepairState::Probation | RepairState::InUse)
}

/// RULINGS R7: a repair made in ANOTHER lane with `resolves` is lane-local — no row here, not in this
/// lane's catch-up set, never propagated.
pub fn concerns_lane(rec: &RepairRecord, lane: &LaneKey) -> bool {
    rec.resolves.is_none() || rec.origin_lane == *lane
}

/// Every in-force repair that concerns `lane` (R7), in id order. `Err` names the first member that
/// could not be read (FG1: an unreadable listing or record is never "no repair").
pub fn in_force_for_lane(root: &Path, lane: &LaneKey) -> Result<Vec<RepairRecord>, String> {
    let ids = ledger::list_ids_checked_in(root).map_err(|e| format!("수리 장부 목록을 읽을 수 없습니다({e:?})"))?;
    let mut out = Vec::new();
    for id in ids {
        match ledger::read_record_in(root, &id) {
            Ok(Some(rec)) if in_force(&rec) && concerns_lane(&rec, lane) => out.push(rec),
            Ok(_) => {}
            Err(e) => return Err(format!("{}: 수리 기록을 읽을 수 없습니다({e:?})", id.as_str())),
        }
    }
    Ok(out)
}

fn is_approval_class(c: Classification) -> bool {
    matches!(c, Classification::BootCritical | Classification::ProtectionRelated)
}

/// The record's class, or a target on either compiled list (the gate has no daemon classifier; the
/// list lookups fold the spelling — BN1 / CD1).
fn approval_class_by_lists(rec: &RepairRecord) -> bool {
    is_approval_class(rec.classification)
        || ledger::file_targets(rec).any(|f| {
            super::boot_critical::protection_entry(&f.rel).is_some() || super::boot_critical::boot_critical_entry(&f.rel).is_some()
        })
}

// ───────────────────────────── the assignment gate (I1 / I7 / I8 / P5) ─────────────────────────────

/// RULINGS I7 / I8 decision table: (1) no in-force repair → OPEN; (2) catch-up recorded and the id
/// file in the recorded pack equals the recorded instance → OPEN; (6) catch-up recorded but the id
/// absent / unreadable / different → HELD "pack instance changed"; (4) job state unreadable → HELD;
/// (3) otherwise HELD with the cause. No time escape, no allow-on-error; reads only.
pub fn gate_verdict_in(root: &Path, lane: &LaneKey) -> GateVerdict {
    let job = read_job_state_checked(root, lane);
    // (2): a recorded catch-up with the same pack instance is OPEN for ever — no other ledger file is
    // read, so a later unreadable record of another repair cannot hold a lane that caught up (E④)
    if let FileRead::Value(st) = &job {
        if st.initial_catchup_complete {
            if let (Some(dir), Some(id)) = (&st.pack_dir, &st.pack_instance) {
                if read_pack_instance(dir) == FileRead::Value(id.clone()) {
                    return GateVerdict::Open;
                }
            }
        }
    }
    let recs = match in_force_for_lane(root, lane) {
        Ok(r) => r,
        Err(why) => return GateVerdict::Held(HoldCause::StateUnreadable(why)),
    };
    if recs.is_empty() {
        return GateVerdict::Open;
    }
    let st = match job {
        FileRead::Value(st) => st,
        FileRead::Absent => return GateVerdict::Held(HoldCause::FirstPassNotDone),
        FileRead::Malformed(w) | FileRead::ReadError(w) => return GateVerdict::Held(HoldCause::StateUnreadable(w)),
    };
    if st.initial_catchup_complete {
        // (6) / interpretation 24: recorded, but the id file absent, unreadable or different
        return GateVerdict::Held(HoldCause::PackInstanceChanged);
    }
    let (mut pending, mut conflict, mut awaiting) = (0u32, 0u32, 0u32);
    for rec in &recs {
        match ledger::read_lane_in(root, &rec.id, lane) {
            Ok(None) => pending += 1,
            Ok(Some(row)) => match row.state {
                LaneState::Pending => pending += 1,
                LaneState::Conflict => {
                    conflict += 1;
                    if rec.origin_lane != *lane && approval_class_by_lists(rec) {
                        awaiting += 1;
                    }
                }
                LaneState::Applied | LaneState::Verified => {}
            },
            Err(e) => return GateVerdict::Held(HoldCause::StateUnreadable(format!("{}: 이 부서의 행({e:?})", rec.id.as_str()))),
        }
    }
    if awaiting > 0 {
        GateVerdict::Held(HoldCause::AwaitingFinalApproval { rows: awaiting })
    } else if pending == 0 && conflict == 0 {
        GateVerdict::Held(HoldCause::FirstPassNotDone)
    } else {
        GateVerdict::Held(HoldCause::Pending { pending, conflict })
    }
}

/// The P5 wording of O1(c)'s cause (contract text, quoted verbatim).
pub const AWAITING_FINAL_APPROVAL_EN: &str =
    "needs a candidate, both reviewer signatures and the owner's final approval in this department — see `cys repair status`";

/// The sentence of a HELD cause (what `cys repair gate` and `task-prompt` print).
pub fn hold_text(cause: &HoldCause) -> String {
    match cause {
        HoldCause::Pending { pending, conflict } => format!(
            "이 부서가 아직 따라잡지 못한 수리가 있습니다(대기 {pending}건 · 충돌 {conflict}건) — 수리 티켓과 팀 편성은 되고, 일반 작업 배정은 따라잡은 뒤에 됩니다(cys repair status)"
        ),
        HoldCause::FirstPassNotDone => "이 부서의 데몬이 수리 따라잡기의 첫 차례를 아직 마치지 않았습니다(first pass not done) — 데몬이 켜져 있으면 약 2분 안에 끝납니다".to_string(),
        HoldCause::StateUnreadable(why) => format!("이 부서의 수리 상태 파일을 읽을 수 없어 일반 작업 배정을 보류합니다 — {why}"),
        HoldCause::PackInstanceChanged => {
            "이 부서의 팩이 바뀌었습니다(pack instance changed — first pass not done) — 데몬의 다음 차례가 다시 맞춘 뒤 열립니다".to_string()
        }
        HoldCause::AwaitingFinalApproval { rows } => format!(
            "이 부서의 기동 필수·보호장치 수리 충돌 {rows}건: 이 부서에서 후보, 리뷰어 두 서명, 주인의 최종 승인이 필요합니다 ({AWAITING_FINAL_APPROVAL_EN})"
        ),
    }
}

/// A short kind name of a cause (the gate's JSON).
pub fn hold_kind(cause: &HoldCause) -> &'static str {
    match cause {
        HoldCause::Pending { .. } => "pending",
        HoldCause::FirstPassNotDone => "first_pass_not_done",
        HoldCause::StateUnreadable(_) => "state_unreadable",
        HoldCause::PackInstanceChanged => "pack_instance_changed",
        HoldCause::AwaitingFinalApproval { .. } => "awaiting_final_approval",
    }
}

/// A lane's catch-up status (H3 / H5): `complete` = the job state records the catch-up; `pending` =
/// in-force repairs without a row or with a `대기` row; `conflict` = `충돌` rows.
pub fn catchup_status_in(root: &Path, lane: &LaneKey) -> Result<apply::CatchupStatus, RepairError> {
    let complete = match read_job_state_checked(root, lane) {
        FileRead::Value(st) => st.initial_catchup_complete,
        FileRead::Absent => false,
        FileRead::Malformed(w) | FileRead::ReadError(w) => return Err(RepairError::Corrupt(w)),
    };
    let recs = in_force_for_lane(root, lane).map_err(RepairError::Corrupt)?;
    let (mut pending, mut conflict) = (0u32, 0u32);
    for rec in &recs {
        match ledger::read_lane_in(root, &rec.id, lane)? {
            None => pending += 1,
            Some(row) if row.state == LaneState::Pending => pending += 1,
            Some(row) if row.state == LaneState::Conflict => conflict += 1,
            Some(_) => {}
        }
    }
    Ok(apply::CatchupStatus { complete, pending, conflict })
}

/// RULINGS HH7 — ONE line over every registered lane holding a `충돌` row of an in-force repair
/// (`None` when there is none). A pure read; shown by `status` / the delegation tool — never sent.
pub fn conflict_summary_line_in(root: &Path, registry: &Path) -> Option<String> {
    let lanes = match ledger::registered_lanes_from(registry) {
        RegisteredLanes::Known(l) => l,
        RegisteredLanes::Unknown(_) => return None,
    };
    let ids = ledger::list_ids_checked_in(root).ok()?;
    let mut by_lane: Vec<(String, Vec<String>)> = Vec::new();
    for id in ids {
        let rec = match ledger::read_record_in(root, &id) {
            Ok(Some(rec)) if in_force(&rec) => rec,
            _ => continue, // display only: an unreadable member is shown by `status` itself
        };
        for lane in &lanes {
            if !concerns_lane(&rec, lane) {
                continue;
            }
            if let Ok(Some(row)) = ledger::read_lane_in(root, &id, lane) {
                if row.state == LaneState::Conflict {
                    match by_lane.iter_mut().find(|(l, _)| *l == lane.0) {
                        Some((_, v)) => v.push(id.as_str().to_string()),
                        None => by_lane.push((lane.0.clone(), vec![id.as_str().to_string()])),
                    }
                }
            }
        }
    }
    if by_lane.is_empty() {
        return None;
    }
    by_lane.sort();
    let list: Vec<String> = by_lane.iter().map(|(l, v)| format!("{l}({})", v.join(", "))).collect();
    Some(format!(
        "수리 충돌로 남은 부서 {}곳: {} — 그 부서의 마스터가 티켓으로 그 부서 워커에게 맡깁니다(팀이 없는 부서는 팀을 만들면 그 워커가 합칩니다)",
        by_lane.len(),
        list.join(", ")
    ))
}

// ───────────────────────────── the pack-instance pass (I8 · K2 · K7 · L1 · L5 · O4 · O5) ─────────────────────────────

/// What the pass did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PassOutcome {
    /// Same instance, same directory, same generation — nothing written.
    Equal,
    /// L5 (a) / O4: same id, another directory or generation — rows whose live state differs from
    /// their recorded result went back to 대기.
    VerifyOnly { demoted: u32 },
    /// L5 (b): another instance — the full reset completed.
    Reset { demoted: u32 },
    /// O5: a transient read error (or a step that could not complete) — nothing decided this tick.
    Skipped(String),
}

/// The test points between the four steps of the full reset (a crash injected there).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassStep {
    AfterCatchupCleared,
    AfterRowsDemoted,
    AfterIdWritten,
    AfterRecorded,
}

/// Everything the pass needs.
pub struct PassEnv<'a> {
    pub root: &'a Path,
    pub lane: &'a LaneKey,
    pub pack: &'a Path,
    pub now: f64,
    /// The daemon's per-repair mutex (R3) around each row's re-read and write.
    pub locked: &'a RepairLock<'a>,
    /// Tests: `true` = stop here (a crash).
    pub stop: &'a dyn Fn(PassStep) -> bool,
}

/// Marker of a test's stop (never produced in production).
pub const PASS_STOPPED: &str = "__pass_stopped__";

/// The recorded result of a row: the current merge's result, else what was applied.
fn recorded_result(row: &LaneRecord) -> &[(String, String)] {
    match &row.merge {
        Some(m) => &m.result,
        None => &row.applied,
    }
}

fn live_canon(pack: &Path, rel: &str) -> Result<String, String> {
    TargetState::read(&pack.join(rel)).map(|s| s.canon()).map_err(|e| format!("{rel}: {e:?}"))
}

/// Re-reads a row under the per-repair mutex and writes `f`'s change durably (FG4) — nothing when
/// the row is gone or unreadable.
fn update_row(root: &Path, lane: &LaneKey, id: &RepairId, locked: &RepairLock<'_>, now: f64, f: &dyn Fn(&mut LaneRecord) -> bool) -> Result<bool, RepairError> {
    let mut changed = false;
    locked(id, &mut || {
        let _scope = super::guard::MutexScope::enter();
        if let Some(mut row) = ledger::read_lane_in(root, id, lane)? {
            if f(&mut row) {
                row.updated_at = now;
                ledger::write_lane_durable_in(root, &row)?;
                changed = true;
            }
        }
        Ok(())
    })?;
    Ok(changed)
}

/// RULINGS I8 / K2 / L5 — the pack-instance pass, run at EVERY tick after readiness and before any
/// propagation. `fresh_id` (64 hex, the daemon's CSPRNG) is used only by a full reset.
pub fn pack_instance_pass_in(env: &PassEnv<'_>, fresh_id: &str) -> PassOutcome {
    if !is_pack_instance_id(fresh_id) {
        return PassOutcome::Skipped("새 팩 식별자가 형식에 맞지 않습니다".into());
    }
    let job = read_job_state_checked(env.root, env.lane);
    if let FileRead::ReadError(w) = &job {
        return PassOutcome::Skipped(format!("상태 파일을 지금 읽을 수 없습니다 — {w}"));
    }
    let id_now = read_pack_instance(env.pack);
    if let FileRead::ReadError(w) = &id_now {
        return PassOutcome::Skipped(format!("팩 식별자를 지금 읽을 수 없습니다 — {w}"));
    }
    // interpretation 31 / P5: the identity and the generation are read ONCE at the start and recorded
    // at the end; an unreadable one skips its watch for this tick (never "changed")
    let identity = dir_identity(env.pack).ok();
    let generation = apply::pack_generation_checked(env.pack).ok();
    let same_instance = match (&job, &id_now) {
        (FileRead::Value(st), FileRead::Value(id)) => st.pack_instance.as_deref() == Some(id.as_str()),
        _ => false,
    };
    if same_instance {
        let FileRead::Value(st) = job else { unreachable!("same_instance needs a value") };
        let dir_changed = st.pack_dir.as_deref() != Some(env.pack)
            || identity.as_ref().is_some_and(|i| st.pack_dir_identity.as_ref() != Some(i));
        let gen_changed = generation.as_ref().is_some_and(|g| st.pack_generation.as_ref() != Some(g));
        if !dir_changed && !gen_changed {
            return PassOutcome::Equal;
        }
        return match verify_only(env, st, identity, generation) {
            Ok(demoted) => PassOutcome::VerifyOnly { demoted },
            Err(e) => PassOutcome::Skipped(format!("{e:?}")),
        };
    }
    match full_reset(env, job, id_now, fresh_id, identity, generation) {
        Ok(demoted) => PassOutcome::Reset { demoted },
        Err(e) => PassOutcome::Skipped(format!("{e:?}")),
    }
}

/// L5 (a): every in-force row `적용됨` / `검증됨` whose live state differs from its recorded result
/// goes back to 대기 (the recorded result kept); no check runs; the lane is not held.
fn verify_only(env: &PassEnv<'_>, mut st: JobState, identity: Option<DirIdentity>, generation: Option<PackGeneration>) -> Result<u32, RepairError> {
    let recs = in_force_for_lane(env.root, env.lane).map_err(RepairError::Corrupt)?;
    let mut demoted = 0u32;
    for rec in &recs {
        let Some(row) = ledger::read_lane_in(env.root, &rec.id, env.lane)? else { continue };
        if !matches!(row.state, LaneState::Applied | LaneState::Verified) {
            continue;
        }
        let differs = recorded_result(&row).iter().any(|(rel, canon)| live_canon(env.pack, rel).as_deref() != Ok(canon.as_str()));
        if differs
            && update_row(env.root, env.lane, &rec.id, env.locked, env.now, &|r| {
                if matches!(r.state, LaneState::Applied | LaneState::Verified) {
                    r.state = LaneState::Pending;
                    true
                } else {
                    false
                }
            })?
        {
            demoted += 1;
        }
    }
    st.pack_dir = Some(env.pack.to_path_buf());
    if identity.is_some() {
        st.pack_dir_identity = identity;
    }
    if generation.is_some() {
        st.pack_generation = generation;
    }
    write_job_state(env.root, &st)?;
    Ok(demoted)
}

fn stopped(env: &PassEnv<'_>, at: PassStep) -> Result<(), RepairError> {
    if (env.stop)(at) {
        return Err(RepairError::Io(format!("{PASS_STOPPED} at {at:?}")));
    }
    Ok(())
}

/// L5 (b) / K7 / I8 mechanics, in this order, each step idempotent: (1) catch-up cleared → (2) every
/// in-force row 적용됨 / 검증됨 / 충돌 → 대기 (merge lock and attempt cleared, the merge record moved to
/// history, recorded results kept) → (3) a different id file unlinked, the new one created new and
/// flushed with its folder → (4) the instance, folder and its identity, and the generation recorded
/// LAST. A stop anywhere leaves a mismatch: the next tick repeats.
fn full_reset(
    env: &PassEnv<'_>,
    job: FileRead<JobState>,
    id_now: FileRead<String>,
    fresh_id: &str,
    identity: Option<DirIdentity>,
    generation: Option<PackGeneration>,
) -> Result<u32, RepairError> {
    // (1)
    let mut st = match job {
        FileRead::Value(st) => st,
        _ => fresh_job_state(env.lane),
    };
    st.initial_catchup_complete = false;
    write_job_state(env.root, &st)?;
    stopped(env, PassStep::AfterCatchupCleared)?;
    // (2)
    let recs = in_force_for_lane(env.root, env.lane).map_err(RepairError::Corrupt)?;
    let mut demoted = 0u32;
    for rec in &recs {
        let changed = update_row(env.root, env.lane, &rec.id, env.locked, env.now, &|r| {
            if !matches!(r.state, LaneState::Applied | LaneState::Verified | LaneState::Conflict) {
                return false;
            }
            r.state = LaneState::Pending;
            r.merge_lock = None;
            r.merge_attempt = 0;
            r.merge_base.clear();
            r.conflict_live.clear();
            if let Some(m) = r.merge.take() {
                r.merge_history.push(m);
            }
            true
        })?;
        if changed {
            demoted += 1;
        }
    }
    stopped(env, PassStep::AfterRowsDemoted)?;
    // (3)
    let idp = super::paths::pack_instance_path(env.pack);
    let present = matches!(id_now, FileRead::Value(_) | FileRead::Malformed(_));
    replace_pack_instance(env.pack, &idp, present, fresh_id)?;
    stopped(env, PassStep::AfterIdWritten)?;
    // (4)
    st.pack_instance = Some(fresh_id.to_string());
    st.pack_dir = Some(env.pack.to_path_buf());
    st.pack_dir_identity = identity;
    st.pack_generation = generation;
    write_job_state(env.root, &st)?;
    stopped(env, PassStep::AfterRecorded)?;
    Ok(demoted)
}

/// I8 step (3) / E1 — the pass's ONLY write inside the pack (RULINGS HH9: one row of its own class
/// in T6's write census): a present id file (a different or malformed one — this mechanism's own
/// bookkeeping file, not pack content) is unlinked; the new one is created NEW (never over anything),
/// flushed, and its folder flushed.
fn replace_pack_instance(pack: &Path, idp: &Path, present: bool, fresh_id: &str) -> Result<(), RepairError> {
    use std::io::Write;
    let io = |e: std::io::Error| RepairError::Io(format!("{}: {e}", idp.display()));
    if present {
        match std::fs::remove_file(idp) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io(e)),
        }
    }
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(idp).map_err(io)?;
    f.write_all(format!("{fresh_id}\n").as_bytes()).map_err(io)?;
    durable::full_flush_file(&f).map_err(io)?;
    durable::full_flush_dir(pack)
}

// ───────────────────────────── the lane's tick: its own rows (K1 · J2 · P2 · R7 · H8) ─────────────────────────────

/// Where an edit's / a creation's result BYTES come from: a file holding R's committed bytes of `rel`
/// (`Err(reason)` → the lane's row 충돌 with that reason). Production: R's sealed committed copy
/// (RULINGS HI2); tests: a double.
pub type ResultBytesFn<'a> = dyn Fn(&RepairRecord, &str) -> Result<PathBuf, String> + 'a;
/// The class of a target in THIS lane (the daemon's `production_classify` over this lane's pack —
/// protection asked first, BN2).
pub type ClassifyFn<'a> = dyn Fn(&str) -> Result<Classification, RepairError> + 'a;

/// Everything a lane's tick needs (the daemon builds it; tests build it with doubles).
pub struct LaneEnv<'a> {
    pub op: OpEnv<'a>,
    pub check: &'a CheckFn<'a>,
    /// RULINGS IG4 (P3 / D④): the built-in reference check of a kind — the SAME function T6's origin
    /// apply and T6's job use (`apply::production_reference` in production).
    pub reference: &'a ReferenceFn<'a>,
    pub classify: &'a ClassifyFn<'a>,
    pub locked: &'a RepairLock<'a>,
    pub hooks: &'a HookStepFn<'a>,
    pub result_bytes: &'a ResultBytesFn<'a>,
    /// RULINGS HH1: what "known vendor content" means for a deletion (C11 11.3) — only its embedded
    /// copies and the compiled previous-release table are read (`pack::released_bytes`); production
    /// passes `UpdateSources::none()`, which holds exactly those.
    pub sources: &'a crate::pack::UpdateSources,
    /// The existing pack-apply lock file (TRIED, never waited for).
    pub apply_lock: &'a Path,
    /// Tests: a stop of the pass between its steps.
    pub pass_stop: &'a dyn Fn(PassStep) -> bool,
}

/// What one tick did (ledger only — nothing is sent).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TickReport {
    pub pass: Option<PassOutcome>,
    /// (repair, the row's state after this tick, why) for every row the tick decided.
    pub rows: Vec<(RepairId, LaneState, String)>,
    /// Rows left as they were this tick (busy, generation unknown, a step not available).
    pub left: Vec<(RepairId, String)>,
    pub lines: Vec<String>,
    pub catchup_completed: bool,
}

/// One tick of this lane: the pack-instance pass, then every in-force repair's row of this lane,
/// then the catch-up completion. Runs under the pack-apply lock, TRIED.
pub fn propagation_tick_in(env: &LaneEnv<'_>, fresh_id: Option<&str>) -> Result<TickReport, RepairError> {
    let mut report = TickReport::default();
    let Some(fresh_id) = fresh_id else {
        report.lines.push("새 팩 식별자를 만들 난수를 얻지 못해 이번 차례는 건너뜁니다".into());
        return Ok(report);
    };
    sweep_own_scratch(env.op.root, env.op.lane);
    // RULINGS IK3 — the machine-wide pack-apply lock is TRIED only around writes: the pass here, then
    // each repair's door operations and each row's publication; NO check child runs while it is held
    // (a check that hangs to its deadline in one lane never holds another lane's turn or an update)
    let pass_env = PassEnv { root: env.op.root, lane: env.op.lane, pack: env.op.pack, now: env.op.now, locked: env.locked, stop: env.pass_stop };
    let pass = match crate::pack_apply_lock::try_with_apply_lock(env.apply_lock, || pack_instance_pass_in(&pass_env, fresh_id)) {
        Ok(Some(p)) => p,
        Ok(None) => {
            report.lines.push("다른 적용이나 업데이트가 팩을 바꾸는 중 — 이번 차례의 전파는 하지 않습니다".into());
            return Ok(report);
        }
        Err(e) => {
            report.lines.push(format!("팩 적용 잠금을 확인할 수 없어 이번 차례의 전파는 하지 않습니다 — {e}"));
            return Ok(report);
        }
    };
    tick_body(env, pass, &mut report)?;
    Ok(report)
}

fn tick_body(env: &LaneEnv<'_>, pass: PassOutcome, report: &mut TickReport) -> Result<(), RepairError> {
    let root = env.op.root;
    let lane = env.op.lane;
    report.pass = Some(pass.clone());
    if let PassOutcome::Skipped(why) = &pass {
        if why.contains(PASS_STOPPED) {
            return Err(RepairError::Io(why.clone()));
        }
        report.lines.push(format!("팩 식별 확인을 이번 차례에 하지 못했습니다 — {why}"));
        return Ok(());
    }
    let recs = match in_force_for_lane(root, lane) {
        Ok(r) => r,
        Err(why) => {
            report.lines.push(why);
            return Ok(());
        }
    };
    // EJ1: the binding of every apply this tick is born here, once
    let bound = Bound::now(env.op.pack);
    // IV3: the generation and instance this tick's completeness is judged on
    let judged = Stamp::of_pack(env);
    let mut complete = true;
    for rec in &recs {
        let left_before = report.left.len();
        match decide(env, rec, &bound, report) {
            Ok(Some(st)) if matches!(st, LaneState::Applied | LaneState::Verified) => {}
            Ok(_) => complete = false,
            Err(e) if format!("{e:?}").contains(apply::INJECTED_STOP) => return Err(e),
            Err(e) => {
                complete = false;
                report.left.push((rec.id.clone(), format!("{e:?}")));
            }
        }
        // RULINGS IK2: why a row was left as it was is kept ON the row (a persistent cause — a
        // generation that cannot be read, an error of the engine — would otherwise hold a lane with no
        // visible reason); a passing cause (the lock held, another writer, the guard) is only logged
        if let Some((_, why)) = report.left[left_before..].iter().rev().find(|(_, w)| !is_passing_cause(w)) {
            note_reason(env, &rec.id, why);
        }
    }
    if complete {
        #[cfg(test)]
        seam(&BEFORE_CATCHUP, env.op.pack);
        // IV3: the catch-up record is written inside the tried lock, after the generation and instance
        // its completeness was judged on were measured again (a difference → not recorded this tick)
        let ran = crate::pack_apply_lock::try_with_apply_lock(env.apply_lock, || -> Result<Option<String>, RepairError> {
            if !judged.pack_holds(env.op.pack, true) {
                return Ok(Some(format!("따라잡기 완료를 이번 차례에 적지 않습니다 — {STATES_CHANGED}")));
            }
            match read_job_state_checked(root, lane) {
                FileRead::Value(mut st) if !st.initial_catchup_complete => {
                    st.initial_catchup_complete = true;
                    write_job_state(root, &st)?;
                    Ok(None)
                }
                FileRead::Value(_) => Ok(Some(String::new())),
                other => Ok(Some(format!("따라잡기 완료를 적지 못했습니다 — 상태 파일 {other:?}"))),
            }
        });
        match ran {
            Ok(Some(Ok(None))) => report.catchup_completed = true,
            Ok(Some(Ok(Some(line)))) if line.is_empty() => {} // recorded before
            Ok(Some(Ok(Some(line)))) => report.lines.push(line),
            Ok(Some(Err(e))) => return Err(e),
            Ok(None) => report.lines.push(format!("따라잡기 완료를 이번 차례에 적지 않습니다 — {LOCK_BUSY_ROW}")),
            Err(why) => report.lines.push(format!("따라잡기 완료를 적지 못했습니다 — 팩 적용 잠금을 확인할 수 없습니다 — {why}")),
        }
    }
    Ok(())
}

/// A new 대기 row of this lane (K1 — created by this lane's daemon only).
fn new_row(id: &RepairId, lane: &LaneKey, now: f64) -> LaneRecord {
    LaneRecord {
        schema: super::REPAIR_SCHEMA,
        repair: id.clone(),
        lane: lane.clone(),
        state: LaneState::Pending,
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
        updated_at: now,
    }
}

/// Writes `row` under the per-repair mutex only if the stored row is still the one this tick decided
/// on (R3: re-read and write; another writer meanwhile → left for the next tick).
fn write_row_if(env: &LaneEnv<'_>, before: Option<&LaneRecord>, row: &LaneRecord) -> Result<bool, RepairError> {
    let mut written = false;
    let root = env.op.root;
    (env.locked)(&row.repair, &mut || {
        let _scope = super::guard::MutexScope::enter();
        let now_row = ledger::read_lane_in(root, &row.repair, &row.lane)?;
        if now_row.as_ref() == before {
            ledger::write_lane_durable_in(root, row)?;
            written = true;
        }
        Ok(())
    })?;
    Ok(written)
}

/// RULINGS IG5 (contract R3) — the record as the decision saw it: still in force, the same committed
/// result. A record that cannot be read is "changed" (nothing is written on a doubt).
fn record_unchanged(env: &LaneEnv<'_>, rec: &RepairRecord) -> bool {
    matches!(ledger::read_record_in(env.op.root, &rec.id), Ok(Some(now)) if in_force(&now) && now.committed == rec.committed)
}

/// The sentence of a decision dropped because the record changed after it was made (IG5).
pub const RECORD_CHANGED: &str = "이 결정 뒤에 수리 기록이 바뀌었습니다(되돌림·대체·다시 확정) — 이 부서에는 아무것도 쓰지 않았고 다음 차례에 새로 결정합니다";
/// The sentence of a publication that found the pack-apply lock held (IK3).
pub const LOCK_BUSY_ROW: &str = "다른 적용이나 업데이트가 팩 적용 잠금을 쥐고 있어 이 행은 다음 차례에 씁니다";

/// A cause that passes by itself at a later tick (logged, not kept on the row — IK2).
fn is_passing_cause(why: &str) -> bool {
    why == LOCK_BUSY_ROW || why.starts_with("다른 작성자가") || why.contains(super::guard::BUSY) || why.starts_with("팩 적용 잠금을 확인할 수 없습니다")
}

/// RULINGS IK2 — keeps `why` as the reason of this lane's EXISTING row of `id` (only when it differs —
/// one write per new reason, never one per tick). IV2: inside the tried pack-apply lock like every
/// other row write; it is validated by NO stamp, by design — it records why the decision's own writes
/// were dropped (a changed record, changed states), so it changes nothing but the reason text (no
/// state, no applied result, no probation). A held lock or a row that cannot be read or written keeps
/// the old reason (the job log still prints the new one).
fn note_reason(env: &LaneEnv<'_>, id: &RepairId, why: &str) {
    let why: String = why.chars().take(400).collect();
    let _ = crate::pack_apply_lock::try_with_apply_lock(env.apply_lock, || {
        update_row(env.op.root, env.op.lane, id, env.locked, env.op.now, &|r| {
            if r.reason == why {
                return false;
            }
            r.reason = why.clone();
            true
        })
    }); // a note only: the job log carries the reason whatever happens here
}

/// The sentence of a check result that is not published because what it was made on changed (IQ1).
pub const STATES_CHANGED: &str = "검사한 뒤 이 부서의 대상 파일·팩 세대·팩 식별 중 하나가 바뀌어 이 검사 결과를 쓰지 않았습니다 — 다음 차례에 새로 결정합니다";

/// RULINGS IV1 — ONE STAMP PER DECISION, TAKEN ONCE, CHECKED AT EVERY WRITE. Fixed when the decision
/// for a repair is made (the first reading of the record and of the lane's states): R's committed result
/// (with "in force" read again from the record), the pack's generation and instance, and each target's
/// measured state (`None` when the decision read no state). Nothing later re-bases it from a fresh
/// reading; the ONLY change is by this lane's own door — after a door operation the target's state is
/// what the door itself placed (`placed`). Every write for the decision begins, inside the tried
/// pack-apply lock, with `holds`.
#[derive(Debug, Clone, PartialEq)]
pub struct Stamp {
    committed: Vec<(String, String)>,
    generation: Option<PackGeneration>,
    instance: Option<String>,
    states: Option<Vec<(String, String)>>,
}

impl Stamp {
    /// The pack's generation and instance as they are now.
    fn pack_now(pack: &Path) -> (Option<PackGeneration>, Option<String>) {
        let generation = apply::pack_generation_checked(pack).ok();
        let instance = match read_pack_instance(pack) {
            FileRead::Value(id) => Some(id),
            _ => None,
        };
        (generation, instance)
    }
    /// The stamp of a decision on `rec` made on `live` (`None`: the decision read no state).
    fn take(env: &LaneEnv<'_>, rec: &RepairRecord, live: Option<&[(String, TargetState)]>) -> Stamp {
        let (generation, instance) = Stamp::pack_now(env.op.pack);
        Stamp { committed: rec.committed.clone(), generation, instance, states: live.map(canon_list) }
    }
    /// The catch-up's stamp (IV3): the generation and instance the tick's completeness is judged on.
    fn of_pack(env: &LaneEnv<'_>) -> Stamp {
        let (generation, instance) = Stamp::pack_now(env.op.pack);
        Stamp { committed: Vec::new(), generation, instance, states: None }
    }
    /// After a door operation of THIS lane: the target's state is what the door itself placed.
    fn placed(&mut self, rel: &str, st: &TargetState) {
        if let Some(e) = self.states.as_mut().and_then(|v| v.iter_mut().find(|(r, _)| r == rel)) {
            e.1 = st.canon();
        }
    }
    /// The generation and instance measured again; `strict` = every part must also be KNOWN (a check's
    /// result, a door operation, the catch-up — an unreadable part is never "unchanged").
    fn pack_holds(&self, pack: &Path, strict: bool) -> bool {
        if strict && (self.generation.is_none() || self.instance.is_none()) {
            return false;
        }
        Stamp::pack_now(pack) == (self.generation.clone(), self.instance.clone())
    }
    /// Inside the tried lock: the record (in force, the same committed result), the generation and
    /// instance, and every target's state measured again. `Err` = why nothing is written.
    fn holds(&self, env: &LaneEnv<'_>, rec: &RepairRecord, strict: bool) -> Result<(), &'static str> {
        if !record_unchanged(env, rec) || rec.committed != self.committed {
            return Err(RECORD_CHANGED);
        }
        if !self.pack_holds(env.op.pack, strict) {
            return Err(STATES_CHANGED);
        }
        if let Some(states) = &self.states {
            match live_states(env.op.pack, rec) {
                Ok(l) if canon_list(&l) == *states => {}
                _ => return Err(STATES_CHANGED),
            }
        }
        Ok(())
    }
}

#[cfg(test)]
thread_local! {
    /// Test-only (IV1 / IV3): a seam run right after a decision's stamp was taken (the pack path), right
    /// after the door phase released its lock, and right before the catch-up record.
    pub(crate) static AFTER_DECISION: std::cell::RefCell<Option<Box<dyn Fn(&Path)>>> = const { std::cell::RefCell::new(None) };
    pub(crate) static AFTER_DOOR_PHASE: std::cell::RefCell<Option<Box<dyn Fn(&Path)>>> = const { std::cell::RefCell::new(None) };
    pub(crate) static BEFORE_CATCHUP: std::cell::RefCell<Option<Box<dyn Fn(&Path)>>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn seam(which: &'static std::thread::LocalKey<std::cell::RefCell<Option<Box<dyn Fn(&Path)>>>>, pack: &Path) {
    which.with(|h| {
        if let Some(f) = h.borrow().as_ref() {
            f(pack);
        }
    });
}

/// What a publication did.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Published {
    Written,
    OtherWriter,
    RecordChanged,
    StatesChanged,
    LockBusy,
    /// IQ2: the tombstones that belong to this row could not be written — the row is not written either.
    NotWritten(String),
}

/// How a row is published (IV1: ALWAYS against the decision's stamp): `checked` = the result of a check
/// made outside the lock (IQ1 — every part of the stamp must be known and unchanged); `tombstones` = the
/// tombstones of a deleting repair are completed in the same unit as the row (IQ2).
#[derive(Clone, Copy)]
struct Publication<'a> {
    stamp: &'a Stamp,
    checked: bool,
    tombstones: bool,
}

impl<'a> Publication<'a> {
    fn plain(stamp: &'a Stamp) -> Publication<'a> {
        Publication { stamp, checked: false, tombstones: false }
    }
    fn checked(stamp: &'a Stamp, tombstones: bool) -> Publication<'a> {
        Publication { stamp, checked: true, tombstones }
    }
}

/// RULINGS IK3 + IG5 — a row decided OUTSIDE the lock is PUBLISHED inside it: the pack-apply lock is
/// tried, the record is read AGAIN (still in force, the same committed result), and the row is written
/// only when no other writer changed it meanwhile. ★Never called while this thread holds the lock.
fn publish(env: &LaneEnv<'_>, rec: &RepairRecord, stored: Option<&LaneRecord>, row: &LaneRecord, how: Publication<'_>) -> Result<Published, RepairError> {
    let ran = crate::pack_apply_lock::try_with_apply_lock(env.apply_lock, || -> Result<Published, RepairError> {
        // IV1: the decision's stamp measured AGAIN inside this lock — the record, the generation and
        // instance, every target's state (IQ1 for a check's result: every part known)
        match how.stamp.holds(env, rec, how.checked) {
            Ok(()) => {}
            Err(why) if why == RECORD_CHANGED => return Ok(Published::RecordChanged),
            Err(_) => return Ok(Published::StatesChanged),
        }
        // IQ2: the tombstones of a deleting repair, in the same unit as the row (after the stamp)
        if how.tombstones {
            if let Err(why) = ensure_tombstones(env, rec) {
                return Ok(Published::NotWritten(why));
            }
        }
        Ok(if write_row_if(env, stored, row)? { Published::Written } else { Published::OtherWriter })
    })
    .map_err(|why| RepairError::Io(format!("팩 적용 잠금을 확인할 수 없습니다 — {why}")))?;
    match ran {
        Some(p) => p,
        None => Ok(Published::LockBusy),
    }
}

/// `publish`, with the reason of a row NOT written put in the report (`left`); `true` = written.
fn publish_row(env: &LaneEnv<'_>, rec: &RepairRecord, stored: Option<&LaneRecord>, row: &LaneRecord, stamp: &Stamp, report: &mut TickReport) -> Result<bool, RepairError> {
    publish_row_as(env, rec, stored, row, Publication::plain(stamp), report)
}

fn publish_row_as(env: &LaneEnv<'_>, rec: &RepairRecord, stored: Option<&LaneRecord>, row: &LaneRecord, how: Publication<'_>, report: &mut TickReport) -> Result<bool, RepairError> {
    let why = match publish(env, rec, stored, row, how)? {
        Published::Written => return Ok(true),
        Published::OtherWriter => "다른 작성자가 이 행을 바꿔 다음 차례에 다시 봅니다".to_string(),
        Published::RecordChanged => RECORD_CHANGED.to_string(),
        Published::StatesChanged => STATES_CHANGED.to_string(),
        Published::LockBusy => LOCK_BUSY_ROW.to_string(),
        Published::NotWritten(why) => why,
    };
    report.left.push((rec.id.clone(), why));
    Ok(false)
}

/// The per-target position of a lane's live state.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Position {
    AtResult,
    AtPre,
    AtPrevApplied,
    /// RULINGS HH1 (C11 11.3): a DELETION whose live file is a known vendor version, byte for byte —
    /// the state read once by `Recorded::released` (the door re-verifies it at the act).
    AtVendor(Recorded),
    Other,
}

/// The committed (result) state of `rel` (U2 canon in `committed`).
fn result_state(rec: &RepairRecord, rel: &str) -> Result<TargetState, String> {
    let canon = rec
        .committed
        .iter()
        .find(|(r, _)| r == rel)
        .map(|(_, c)| c.as_str())
        .ok_or_else(|| format!("{rel}: 확정된 상태가 기록에 없습니다"))?;
    apply::state_of_canon(canon).ok_or_else(|| format!("{rel}: 확정된 상태를 읽을 수 없습니다({canon})"))
}

/// The live states of R's file targets in this lane, `(rel, state)` in rel order.
fn live_states(pack: &Path, rec: &RepairRecord) -> Result<Vec<(String, TargetState)>, String> {
    let mut rels: Vec<&str> = ledger::file_targets(rec).map(|f| f.rel.as_str()).collect();
    rels.sort_unstable();
    rels.dedup();
    let mut out = Vec::new();
    for rel in rels {
        out.push((rel.to_string(), TargetState::read(&pack.join(rel)).map_err(|e| format!("{rel}: 읽을 수 없습니다({e:?})"))?));
    }
    Ok(out)
}

fn canon_list(states: &[(String, TargetState)]) -> Vec<(String, String)> {
    states.iter().map(|(r, s)| (r.clone(), s.canon())).collect()
}

/// RULINGS CB1 / CG / T8b: a lane's composed (promoted) MASTER directive is never written by
/// propagation.
fn promoted_directive(pack: &Path, rec: &RepairRecord) -> Option<String> {
    let key = super::types::fold_name("directives/MASTER_DIRECTIVE.md");
    ledger::file_targets(rec)
        .find(|f| super::types::fold_name(&f.rel) == key)
        .filter(|_| crate::pack::ceo_promoted_in(pack))
        .map(|f| format!("{}: 이 부서의 지침은 CEO 조립본입니다 — 전파로 바꾸지 않습니다(이 부서의 워커가 합칩니다)", f.rel))
}

/// The decision of one repair's row in this lane (§4.2 of the plan). `Ok(Some(state))` = decided;
/// `Ok(None)` = left as it was this tick (busy, generation unknown, a step not available).
fn decide(env: &LaneEnv<'_>, rec: &RepairRecord, bound: &Bound, report: &mut TickReport) -> Result<Option<LaneState>, RepairError> {
    let root = env.op.root;
    let lane = env.op.lane;
    let id = &rec.id;
    // EJ4: the repair's ONE guard for the decision and its writes (tried; held → next tick)
    let Some(_guard) = super::guard::RepairGuard::try_take(root, id) else {
        report.left.push((id.clone(), super::guard::BUSY.to_string()));
        return Ok(None);
    };
    let stored = ledger::read_lane_in(root, id, lane)?;
    let mut row = match &stored {
        Some(r) => r.clone(),
        None => new_row(id, lane, env.op.now),
    };
    // T6's probation in THIS lane returned or failed the propagated apply (its closure `실패 — …` /
    // `되돌려짐 — …` on a row of the CURRENT result) while R is still in force: the lane does not hold R
    // — 충돌 for its worker (H8), never "검증됨" over files that were put back
    let closed_failed = row.probation.as_ref().and_then(|p| p.closed.as_deref()).is_some_and(|c| c.starts_with(apply::CLOSED_FAILED) || c.starts_with(apply::CLOSED_REVERTED));
    // IV1: a decision taken from the record and the row only (no state read yet) has its stamp now
    let stamp0 = Stamp::take(env, rec, None);
    if closed_failed && row.applied == rec.committed && row.state != LaneState::Conflict {
        return finish(env, rec, stored.as_ref(), row, LaneState::Conflict, "이 부서의 시험 기간 재검사가 전파된 수리를 되돌렸거나 실패로 닫았습니다 — 이 부서의 워커가 맡습니다", &stamp0, report);
    }
    // P2: a row whose applied result is not the record's current result goes back to 대기; its
    // previous applied result is a valid pre-image — a merge row's bytes are not (interpretation 41)
    let mut prev_applied: Option<LaneRecord> = None;
    let mut prev_was_merge = false;
    if !row.applied.is_empty() && row.applied != rec.committed {
        prev_applied = stored.clone();
        prev_was_merge = row.merge.is_some();
        let old = std::mem::take(&mut row.applied);
        row.applied_history.push(old);
        if let Some(m) = row.merge.take() {
            row.merge_history.push(m);
        }
        row.state = LaneState::Pending;
    }
    // interpretation 42 (R7): a merge pointing to a repair no longer in force → 대기
    if let Some(v) = row.merge.as_ref().and_then(|m| m.via_repair.clone()) {
        let alive = matches!(ledger::read_record_in(root, &v), Ok(Some(r)) if in_force(&r));
        if !alive {
            if let Some(m) = row.merge.take() {
                row.merge_history.push(m);
            }
            row.state = LaneState::Pending;
        }
    }
    // an unchanged 검증됨 row costs nothing more this tick (no classification, no file read): a change of
    // its files is the verify-only pass's (directory / generation) or T6's probation's
    if row.state == LaneState::Verified && stored.as_ref() == Some(&row) {
        return Ok(Some(LaneState::Verified));
    }
    let approval = is_approval_class(rec.classification) || {
        let mut any = false;
        for f in ledger::file_targets(rec) {
            any |= is_approval_class((env.classify)(&f.rel)?);
        }
        any
    };
    // RULINGS HJ1 — THE RULE OF THE CORE: a boot-critical / protection-related repair is never
    // written by the daemon into a lane that did not make it
    let receiving_approval = approval && rec.origin_lane != *lane;
    if !receiving_approval && row.state == LaneState::Conflict {
        // H8: a general conflict ends only through the lane worker's merge + `recheck` (no file read)
        if stored.as_ref() != Some(&row) && !publish_row(env, rec, stored.as_ref(), &row, &stamp0, report)? {
            return Ok(None);
        }
        return Ok(Some(LaneState::Conflict));
    }
    let live = match live_states(env.op.pack, rec) {
        Ok(l) => l,
        Err(why) => return finish(env, rec, stored.as_ref(), row, LaneState::Conflict, &why, &stamp0, report),
    };
    // IV1 — THE stamp of this decision: the record, the generation and instance, every target's state as
    // read just now; every later write for it is checked against THIS, never against a fresh reading
    let stamp = Stamp::take(env, rec, Some(&live));
    #[cfg(test)]
    seam(&AFTER_DECISION, env.op.pack);
    if receiving_approval {
        return approval_row(env, stored.as_ref(), row, rec, &live, &stamp, report);
    }
    match row.state {
        LaneState::Verified => {
            if !publish_row(env, rec, stored.as_ref(), &row, &stamp, report)? {
                return Ok(None);
            }
            return Ok(Some(LaneState::Verified));
        }
        LaneState::Applied => {
            // a stop between the apply and the lane's check: the check now, when every target still
            // holds what was applied
            if canon_list(&live) == *recorded_result(&row) || canon_list(&live) == row.applied {
                return check_live(env, stored.as_ref(), row, rec, None, &stamp, report);
            }
            row.state = LaneState::Pending;
        }
        LaneState::Conflict | LaneState::Pending => {}
    }
    decide_pending(env, stored.as_ref(), row, rec, &live, approval, prev_applied.as_ref(), prev_was_merge, bound, &stamp, report)
}

/// The sentence of an approval-class row in a lane that did not make the repair (O1(c) / P5 · HJ1):
/// true, and what to do now.
pub fn approval_conflict_text(rec: &RepairRecord) -> String {
    let files: Vec<&str> = ledger::file_targets(rec).map(|f| f.rel.as_str()).collect();
    let trigger = match rec.trigger {
        super::types::Trigger::OwnerOrder => "owner-order",
        super::types::Trigger::Grant => "grant",
        super::types::Trigger::Update => "update",
    };
    format!(
        "기동 필수·보호장치 수리는 부서마다 그 부서의 승인으로 적용됩니다 — 이 부서의 워커가 `cys repair begin --files {} --trigger {trigger} --resolves {}` 로 이 부서의 수리를 시작해 후보를 만들고, 리뷰어 두 서명과 주인의 최종 승인을 받습니다(cys repair status · {AWAITING_FINAL_APPROVAL_EN})",
        files.join(" "),
        rec.id.as_str()
    )
}

/// RULINGS HJ1 (+ U1 (a), R6, R7, interpretation 42) — a row of a boot-critical / protection-related
/// repair R in a lane that did NOT make it. The daemon never writes R's bytes here. The row becomes
/// `검증됨` only when R's check passes on the live files AND every live target is an APPROVED copy in
/// this lane — the F9 evidence of an R′ of this lane with `resolves: R` (the merge record points to
/// R′); otherwise it is `충돌` with the O1(c) cause (re-evaluated only when the live files change).
/// ★Unit T7d's identical apply would stand in front of the final `충돌` — nothing of it is built here.
fn approval_row(
    env: &LaneEnv<'_>,
    stored: Option<&LaneRecord>,
    mut row: LaneRecord,
    rec: &RepairRecord,
    live: &[(String, TargetState)],
    stamp: &Stamp,
    report: &mut TickReport,
) -> Result<Option<LaneState>, RepairError> {
    if row.state == LaneState::Verified {
        if stored != Some(&row) && !publish_row(env, rec, stored, &row, stamp, report)? {
            return Ok(None);
        }
        return Ok(Some(LaneState::Verified));
    }
    let now = canon_list(live);
    if row.state == LaneState::Conflict && now == row.conflict_live && stored == Some(&row) {
        return Ok(Some(LaneState::Conflict)); // nothing changed since the conflict: no loop, no check
    }
    // U1 (a) / R6: every live target an approved copy here, by an R′ of THIS lane resolving R
    let mut via: Option<RepairId> = None;
    let mut approved = !live.iter().all(|(_, st)| *st == TargetState::Absent);
    for (rel, st) in live {
        if *st == TargetState::Absent {
            continue;
        }
        match apply::resolve_approved_copy_in(env.op.root, env.op.pack, env.op.lane, rel) {
            apply::ApprovedCopy::Approved { id, state, .. } if state == *st && id != rec.id => {
                let resolves_r = matches!(ledger::read_record_in(env.op.root, &id), Ok(Some(b))
                    if b.resolves.as_ref() == Some(&rec.id) && b.origin_lane == *env.op.lane && in_force(&b));
                if resolves_r && via.as_ref().is_none_or(|v| *v == id) {
                    via = Some(id);
                } else {
                    approved = false;
                }
            }
            _ => approved = false,
        }
    }
    if approved {
        if let Some(v) = via {
            let merge = MergeRecord {
                by_seat: 0,
                result: now.clone(),
                changed: true,
                note: format!("이 부서의 승인된 수리 {} 로 맞춤", v.as_str()),
                via_repair: Some(v),
            };
            return check_live(env, stored, row, rec, Some(merge), stamp, report);
        }
    }
    row.conflict_live = now;
    finish(env, rec, stored, row, LaneState::Conflict, &approval_conflict_text(rec), stamp, report)
}

/// Publishes the row in `state` with `why` (a 충돌 records the live states — R1) — through `publish`
/// (IK3 / IG5: the lock tried, the record read again; changed → nothing written).
#[allow(clippy::too_many_arguments)]
fn finish(
    env: &LaneEnv<'_>,
    rec: &RepairRecord,
    stored: Option<&LaneRecord>,
    row: LaneRecord,
    state: LaneState,
    why: &str,
    stamp: &Stamp,
    report: &mut TickReport,
) -> Result<Option<LaneState>, RepairError> {
    finish_as(env, rec, stored, row, state, why, Publication::plain(stamp), report)
}

/// `finish` with the publication's kind (IQ1 / IQ2 for the result of a check made outside the lock).
#[allow(clippy::too_many_arguments)]
fn finish_as(
    env: &LaneEnv<'_>,
    rec: &RepairRecord,
    stored: Option<&LaneRecord>,
    mut row: LaneRecord,
    state: LaneState,
    why: &str,
    how: Publication<'_>,
    report: &mut TickReport,
) -> Result<Option<LaneState>, RepairError> {
    row.state = state;
    row.reason = why.chars().take(400).collect(); // IK2: the reason travels with the row
    if state == LaneState::Conflict && row.conflict_live.is_empty() {
        // R1: the live states at the conflict (display; an unreadable member leaves the list empty)
        if let Ok(l) = live_states(env.op.pack, rec) {
            row.conflict_live = canon_list(&l);
        }
    }
    row.updated_at = env.op.now;
    if !publish_row_as(env, rec, stored, &row, how, report)? {
        return Ok(None);
    }
    report.rows.push((row.repair.clone(), state, why.to_string()));
    Ok(Some(state))
}

/// The closure of a row whose lane had nothing of its own replaced (it already held R's result — RULINGS
/// IG3): the timing record is kept, no 24-hour re-check (T6's probation skips a closed bundle), no report.
pub const NOTHING_REPLACED_HERE: &str = "사용 중 — 이 부서는 이미 이 수리의 결과를 갖고 있었습니다(이 부서에서 바꾼 것이 없어 24시간 시험 기간이 없습니다)";

/// The closure of a lane's failed check after THIS lane applied R (C11 11.4): on macOS the return of
/// this lane's operations is owed to T6's probation; on Windows the automatic return is off.
fn failed_closure(env: &LaneEnv<'_>, why: &str) -> String {
    let closed = if env.op.form == ApplyForm::Exchange {
        format!("{} — {} · 적용 뒤 이 부서의 검사 실패: {why}", apply::CLOSED_FAILED, apply::RETURN_OWED)
    } else {
        format!("{} — 적용 뒤 이 부서의 검사 실패(윈도우: 자동으로 되돌리지 않음): {why}", apply::CLOSED_FAILED)
    };
    closed.chars().take(400).collect()
}

/// RULINGS IG3 — evidence that THIS lane applied R's current result itself: a completed (`applied`)
/// door operation of kind `propagate`, of this lane, for R, bound to the present generation, whose
/// incoming state is R's committed state of that file AND the live state now (the write-ahead records
/// of 1.13). `Ok(Some(at))` = the time of the latest such operation; `Ok(None)` = none; `Err` = the
/// records cannot be read (nothing is decided on a partial log).
fn own_apply_evidence(env: &LaneEnv<'_>, rec: &RepairRecord, live: &[(String, TargetState)], bound: &Bound) -> Result<Option<f64>, String> {
    let Bound::Known(gen) = bound else { return Ok(None) };
    let mut at: Option<f64> = None;
    for op in super::wal::list_checked_in(env.op.root, &rec.id)? {
        if op.lane_key != *env.op.lane || op.kind != "propagate" || !op.finished || op.outcome.as_deref() != Some("applied") || op.pack_generation.as_ref() != Some(gen) {
            continue;
        }
        let committed = rec.committed.iter().find(|(r, _)| *r == op.rel).map(|(_, c)| c.as_str());
        let live_now = live.iter().find(|(r, _)| *r == op.rel).map(|(_, st)| st.canon());
        if committed == Some(op.incoming.as_str()) && live_now.as_deref() == Some(op.incoming.as_str()) {
            at = Some(at.map_or(op.updated_at, |a| a.max(op.updated_at)));
        }
    }
    Ok(at)
}

/// J2 / 11.4: R's check on the LIVE files of this lane → `검증됨` (with the merge record restored or
/// given, `CheckTiming`s recorded — R5), else `충돌`.
fn check_live(
    env: &LaneEnv<'_>,
    stored: Option<&LaneRecord>,
    mut row: LaneRecord,
    rec: &RepairRecord,
    merge: Option<MergeRecord>,
    stamp: &Stamp,
    report: &mut TickReport,
) -> Result<Option<LaneState>, RepairError> {
    // IQ1 / IV1: this check is attached to the decision's stamp — its result is published only if the
    // stamp still holds inside the lock (never re-based on a fresh reading taken here)
    let timings = match run_checks(env, rec, env.op.pack, true) {
        Ok(t) => t,
        Err(why) => {
            // IG3 / 11.4: a lane that applied R itself (an open probation bundle) owes the return of its
            // own operations, as after an uninterrupted apply
            if let Some(p) = row.probation.as_mut().filter(|p| p.closed.is_none()) {
                p.closed = Some(failed_closure(env, &why));
            }
            return finish_as(env, rec, stored, row, LaneState::Conflict, &format!("이 부서의 검사 실패 — {why}"), Publication::checked(stamp, false), report);
        }
    };
    row.applied = rec.committed.clone();
    if merge.is_some() {
        row.merge = merge;
    }
    row.conflict_live.clear();
    // R5 / IG3: the check's timing record is persisted in BOTH cases — in the open probation bundle of a
    // lane that applied R itself, or in a CLOSED bundle of a lane that replaced nothing (no 24-hour
    // re-check: there is nothing of its own to return to)
    match row.probation.as_mut() {
        Some(p) => {
            if p.check_timings.is_empty() {
                p.check_timings = timings;
            }
        }
        None => {
            let g = apply::pack_generation(env.op.pack);
            row.probation = Some(ledger::Probation {
                pack_generation: g.version,
                pro_revision: g.pro_revision,
                applied_at: env.op.now,
                expected: rec.committed.clone(),
                closed: Some(NOTHING_REPLACED_HERE.to_string()),
                check_timings: timings,
                check_timeouts: Vec::new(),
            });
        }
    }
    // IG2 + IQ2: the tombstones of a deleting repair are completed INSIDE the publication's lock, after
    // the record and the checked states were validated again — one unit with the 검증됨 row
    finish_as(env, rec, stored, row, LaneState::Verified, "이 부서의 검사 통과", Publication::checked(stamp, true), report)
}

/// R's recorded check of every FILE target whose result is a file, against `root` (the live pack, or
/// a copy). `Err` = a failure, a timeout or a check that could not run (never "passed").
fn run_checks(env: &LaneEnv<'_>, rec: &RepairRecord, root: &Path, measure_reference: bool) -> Result<Vec<ledger::CheckTiming>, String> {
    let mut timings = Vec::new();
    for f in ledger::file_targets(rec) {
        if !matches!(result_state(rec, &f.rel)?, TargetState::File { .. }) {
            continue;
        }
        let deadline = apply::check_deadline(&f.rel);
        let out = (env.check)(&f.rel, root, deadline).map_err(|e| format!("{}: 검사를 돌릴 수 없습니다({e:?})", f.rel))?;
        match out.result {
            CheckResult::Pass => timings.push(ledger::CheckTiming {
                rel: f.rel.clone(),
                check_ms: out.ms,
                reference_kind: apply::check_kind_of(&f.rel),
                // RULINGS IG4 — the reference measured in THIS lane at its apply, exactly as T6's origin
                // apply records it (`reference(check_kind_of(rel), deadline).ok()`, its `ms`; 0 when it
                // could not run — T6's attribution then never blames, the conservative side)
                reference_ms: if measure_reference { (env.reference)(apply::check_kind_of(&f.rel), deadline).ok().map(|x| x.ms).unwrap_or(0) } else { 0 },
            }),
            CheckResult::Fail(m) => return Err(format!("{}: {m}", f.rel)),
            CheckResult::Timeout => return Err(format!("{}: 검사 시간 초과", f.rel)),
        }
    }
    Ok(timings)
}

/// A 대기 row (§4.2 / §4.4 of the plan).
#[allow(clippy::too_many_arguments)]
fn decide_pending(
    env: &LaneEnv<'_>,
    stored: Option<&LaneRecord>,
    mut row: LaneRecord,
    rec: &RepairRecord,
    live: &[(String, TargetState)],
    approval: bool,
    prev_applied: Option<&LaneRecord>,
    prev_was_merge: bool,
    bound: &Bound,
    stamp: &Stamp,
    report: &mut TickReport,
) -> Result<Option<LaneState>, RepairError> {
    if let Some(why) = promoted_directive(env.op.pack, rec) {
        return finish(env, rec, stored, row, LaneState::Conflict, &why, stamp, report);
    }
    // the position of every target
    let mut positions: Vec<(String, Position)> = Vec::new();
    for (rel, st) in live {
        let result = match result_state(rec, rel) {
            Ok(r) => r,
            Err(why) => return finish(env, rec, stored, row, LaneState::Conflict, &why, stamp, report),
        };
        let pre = ledger::file_targets(rec).find(|f| f.rel == *rel).map(|f| f.pre.state());
        let prev = prev_applied.and_then(|p| p.applied.iter().find(|(r, _)| r == rel)).and_then(|(_, c)| apply::state_of_canon(c));
        let pos = if *st == result {
            Position::AtResult
        } else if pre.as_ref() == Some(st) && !prev_was_merge {
            Position::AtPre
        } else if prev.as_ref() == Some(st) && !prev_was_merge {
            Position::AtPrevApplied
        } else if result == TargetState::Absent && matches!(st, TargetState::File { .. }) {
            // HH1 / 11.3: delete only if byte-identical to known vendor content; anything else → 충돌
            match Recorded::released(env.op.pack, rel, env.sources) {
                Some(r) => Position::AtVendor(r),
                None => Position::Other,
            }
        } else {
            Position::Other
        };
        positions.push((rel.clone(), pos));
    }
    let all_result = positions.iter().all(|(_, p)| *p == Position::AtResult);
    if all_result {
        // "already there" (I7(4) / J2): no file write; the hook lines (under the tried lock, the record
        // read again — IK3 / IG5), then this lane's own check (outside the lock)
        match locked_hook_step(env, rec, stamp) {
            Ok(None) => {}
            Ok(Some(why)) => return finish(env, rec, stored, row, LaneState::Conflict, &why, stamp, report),
            Err(left) => {
                report.left.push((rec.id.clone(), left));
                return Ok(None);
            }
        }
        row.merge = None;
        // IG3: "already there" because THIS lane applied R and stopped before its row (evidence: its own
        // completed door operations) → the probation bundle, as if the write had not been interrupted
        match own_apply_evidence(env, rec, live, bound) {
            Ok(Some(at)) => {
                // IQ3: the row this lane's uninterrupted apply would have written — R's committed result
                // as `applied` (what T6's `revert_in` / `revertible_here` return), no merge state — BEFORE
                // the check, so every later path (pass, fail, time-out) acts on a RETURNABLE row
                row.applied = rec.committed.clone();
                row.merge_base.clear();
                row.merge_lock = None;
                row.conflict_live.clear();
                row.approval_ref = None;
                let (version, pro) = bound.row_fields();
                row.probation = Some(ledger::Probation {
                    pack_generation: version,
                    pro_revision: pro,
                    applied_at: at,
                    expected: rec.committed.clone(),
                    closed: None,
                    check_timings: Vec::new(),
                    check_timeouts: Vec::new(),
                });
            }
            Ok(None) => row.probation = None,
            Err(why) => {
                report.left.push((rec.id.clone(), format!("이 부서의 동작 기록을 읽을 수 없어 이번 차례에는 결정하지 않습니다 — {why}")));
                return Ok(None);
            }
        }
        return check_live(env, stored, row, rec, None, stamp, report);
    }
    // interpretation 35: a kept merge result OF THE CURRENT RESULT (the row's recorded base is the
    // record's current result — never after a re-commit, P2 / interpretation 41) that is live again is
    // taken back as current; a merge through a lane repair (`via_repair`) is never restored here
    if prev_applied.is_none() && row.applied == rec.committed {
        if let Some(i) = row.merge_history.iter().rposition(|m| m.via_repair.is_none() && m.result == canon_list(live)) {
            let m = row.merge_history.remove(i);
            return check_live(env, stored, row, rec, Some(m), stamp, report);
        }
    }
    if positions.iter().any(|(_, p)| *p == Position::Other) {
        let why = if prev_was_merge {
            "다시 확정된 수리의 결과와 이 부서가 합친 내용이 다릅니다 — 이 부서의 워커가 다시 합칩니다"
        } else {
            "이 부서의 파일이 수리 전 내용과도 수리 결과와도 다릅니다 — 이 부서의 워커가 합칩니다"
        };
        return finish(env, rec, stored, row, LaneState::Conflict, why, stamp, report);
    }
    if approval {
        // the ORIGIN's own approval-class row (receiving lanes never reach here — HJ1): its approved
        // bytes are not live any more; the daemon writes nothing without a new approval
        return finish(env, rec, stored, row, LaneState::Conflict, "이 부서에서 승인되어 적용된 수리의 내용이 지금 살아 있지 않습니다 — 다시 적용하려면 이 부서의 수리로 후보·리뷰어 두 서명·주인의 최종 승인을 거칩니다(cys repair status)", stamp, report);
    }
    if *bound == Bound::Unknown {
        let why = "팩 세대를 알 수 없어 이번 차례에는 적용하지 않습니다";
        report.left.push((rec.id.clone(), why.into()));
        if stored.is_none() {
            row.reason = why.into();
            publish_row(env, rec, None, &row, stamp, report)?;
        }
        return Ok(None);
    }
    apply_general(env, stored, row, rec, &positions, prev_applied, bound, stamp, report)
}

/// The hook lines of a hook-unregistration repair in this lane (X1 / Y1): `None` = done (or none);
/// `Some(why)` = the step's error (Y2(c) → 충돌).
fn hook_step(env: &LaneEnv<'_>, rec: &RepairRecord) -> Option<String> {
    if ledger::hook_lines(rec).next().is_none() {
        return None;
    }
    match (env.hooks)(&rec.id, env.op.lane, HookStep::Unregister) {
        Ok(_) => None,
        Err(e) => Some(format!("훅 등록 해제를 이 부서에서 마치지 못했습니다 — {e:?}")),
    }
}

/// The hook step of a repair with hook lines, run under the TRIED pack-apply lock after the record was
/// read again (IK3 / IG5): `Ok(None)` done (or nothing to do), `Ok(Some(why))` the step's error (→ 충돌),
/// `Err(why)` not run this tick (the lock held, the record changed) — the row stays as it was.
fn locked_hook_step(env: &LaneEnv<'_>, rec: &RepairRecord, stamp: &Stamp) -> Result<Option<String>, String> {
    if ledger::hook_lines(rec).next().is_none() {
        return Ok(None);
    }
    let ran = crate::pack_apply_lock::try_with_apply_lock(env.apply_lock, || {
        // IV1: the hook step writes (the registrar, the lane's tombstones) only after the decision's
        // stamp was measured again inside this lock
        stamp.holds(env, rec, true).map_err(str::to_string)?;
        Ok(hook_step(env, rec))
    });
    match ran {
        Ok(Some(r)) => r,
        Ok(None) => Err(LOCK_BUSY_ROW.to_string()),
        Err(why) => Err(format!("팩 적용 잠금을 확인할 수 없습니다 — {why}")),
    }
}

/// RULINGS IG2 — an applied / verified row of a DELETING repair implies THIS lane's tombstone: every
/// deletion of R (a pre-image present, the committed result absent) whose target is absent here is
/// recorded in this lane's file — idempotently (`tombstones::record_in` keeps one entry), also when the
/// lane was at the result from the start or a stop fell between the door operation and its tombstone
/// (else the next update would put the vendor file back). `Err` = the reason (→ 충돌).
fn ensure_tombstones(env: &LaneEnv<'_>, rec: &RepairRecord) -> Result<(), String> {
    for f in ledger::file_targets(rec) {
        if f.pre == ledger::PreImage::Absent || result_state(rec, &f.rel)? != TargetState::Absent {
            continue;
        }
        let live = TargetState::read(&env.op.pack.join(&f.rel)).map_err(|e| format!("{}: 읽을 수 없습니다 — {e:?}", f.rel))?;
        if live != TargetState::Absent {
            continue;
        }
        let t = super::tombstones::Tombstone { rel: f.rel.clone(), kind: super::tombstones::TombstoneKind::File, repair: rec.id.clone() };
        super::tombstones::record_in(env.op.root, env.op.lane, &t).map_err(|e| format!("{}: 삭제 표식(묘비) 기록 실패 — {e:?}", f.rel))?;
    }
    Ok(())
}

#[cfg(test)]
thread_local! {
    /// Test-only (IG2): a stop right after a door operation completed, before its tombstone.
    pub(crate) static STOP_AFTER_DOOR_OP: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// How the door phase of one repair ended (IK3 / IG5 — everything here ran under ONE tried lock).
enum DoorEnd {
    /// Every target applied, the hook step done, the row published as 적용됨 (this is the row).
    Applied(LaneRecord),
    /// A door refusal / a tombstone or hook error → 충돌 with this reason (published after the lock).
    Conflict(String),
    /// Nothing written: the lock was held, or the record changed before the first operation, or the
    /// generation moved (Superseded) — the row stays as it was; `bool` = write a NEW 대기 row.
    Left(String, bool),
    /// RULINGS IG5 + IO1 — the record changed AFTER some targets were applied: no further target, the row
    /// already PUBLISHED as 충돌 (inside this lock) with its reason; nothing put back.
    ChangedMidway(String),
}

/// This lane's own scratch folder for the copy-check (removed on every exit; a leftover of a dead
/// process is swept at the next tick).
struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0); // our own scratch copy — never the ledger or a pack
    }
}

fn scratch_prefix(lane: &LaneKey) -> String {
    format!(".propagate-{}-", lane.0)
}

/// Removes leftovers `lanes/.propagate-<lane>-<pid>-<n>` of OTHER processes (exact names only): only
/// this lane's daemon makes this lane's scratch and one daemon runs per lane (the startup lock), so a
/// scratch of another pid is a previous incarnation's.
fn sweep_own_scratch(root: &Path, lane: &LaneKey) {
    let dir = root.join("lanes");
    let Ok(rd) = std::fs::read_dir(&dir) else { return }; // nothing to sweep (absent or unreadable — retried next tick)
    let prefix = scratch_prefix(lane);
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let Some(rest) = name.strip_prefix(&prefix) else { continue };
        let mut parts = rest.split('-');
        let (Some(pid), Some(n), None) = (parts.next(), parts.next(), parts.next()) else { continue };
        if pid.parse::<u32>().is_err() || n.parse::<u128>().is_err() || pid == std::process::id().to_string() {
            continue;
        }
        let _ = std::fs::remove_dir_all(dir.join(&name)); // a dead process's scratch copy (exact name)
    }
}

/// Copies `src` into `dst` from ONE bounded read ([`read_pack_file_bounded`]), checking that the
/// bytes' hash AND the mode of that same handle are exactly `want` (a file state).
fn copy_checked(src: &Path, dst: &Path, want: &TargetState) -> Result<(), String> {
    let TargetState::File { sha256, mode } = want else {
        return Err("파일 상태가 아닙니다".into());
    };
    let (bytes, got_mode) = read_pack_file_bounded(src)?;
    if sha256_hex(&bytes) != *sha256 || got_mode != *mode {
        return Err(format!("{}: 내용이나 권한이 수리 결과({})와 다릅니다", src.display(), want.canon()));
    }
    if let Some(p) = dst.parent() {
        std::fs::create_dir_all(p).map_err(|e| format!("{}: {e}", p.display()))?;
    }
    apply::write_new(dst, &bytes, *mode).map_err(|e| format!("{e:?}"))
}

/// A mode-only change: this lane's own bytes (their hash equal to the result's) with the result's
/// mode — read from one handle, the hash checked.
fn copy_checked_bytes_of(src: &Path, dst: &Path, want: &TargetState) -> Result<(), String> {
    let TargetState::File { sha256, mode } = want else {
        return Err("파일 상태가 아닙니다".into());
    };
    let (bytes, _) = read_pack_file_bounded(src)?;
    if sha256_hex(&bytes) != *sha256 {
        return Err(format!("{}: 내용이 수리 결과와 다릅니다", src.display()));
    }
    if let Some(p) = dst.parent() {
        std::fs::create_dir_all(p).map_err(|e| format!("{}: {e}", p.display()))?;
    }
    apply::write_new(dst, &bytes, *mode).map_err(|e| format!("{e:?}"))
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

// ───────────────────────────── the committed copy (RULINGS HI2 as bounded by HJ2) ─────────────────────────────

/// The `kind` of a committed copy's write-once metadata.
pub const COMMITTED_COPY_KIND: &str = "committed-copy";

/// RULINGS HJ2 — ONE size rule for sealing and receiving: the bounded reader's cap.
pub const SEAL_LIMIT: u64 = durable::RECORD_READ_CAP;

/// The sentence part that names the limit (a refused seal says it).
pub fn seal_limit_text() -> String {
    format!("한도 {SEAL_LIMIT}바이트 — {} MiB", SEAL_LIMIT / (1024 * 1024))
}

/// RULINGS HJ2 — the bytes AND the mode of a pack file from ONE handle, bounded: the same open as
/// `durable::read_record_bytes` (non-blocking, no link followed — unix `O_NONBLOCK | O_NOFOLLOW`,
/// Windows the reparse point itself), a regular file per that handle's `fstat`, at most
/// [`SEAL_LIMIT`] bytes (at most limit + 1 are read, so a source that keeps growing cannot hold the
/// copy), and the length read equal to the length at open (a size change refuses). The mode is that
/// handle's permission bits (`TargetState`'s rule: 0 on Windows).
pub fn read_pack_file_bounded(p: &Path) -> Result<(Vec<u8>, u32), String> {
    use std::io::Read;
    let mut opts = std::fs::OpenOptions::new();
    opts.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        opts.custom_flags(0x0020_0000); // FILE_FLAG_OPEN_REPARSE_POINT: a link is opened as itself
    }
    let f = opts.open(p).map_err(|e| format!("{}: 열 수 없음(링크이거나 읽을 수 없음) — {e}", p.display()))?;
    let m = f.metadata().map_err(|e| format!("{}: {e}", p.display()))?;
    if !m.file_type().is_file() {
        return Err(format!("{}: 일반 파일이 아님", p.display()));
    }
    if m.len() > SEAL_LIMIT {
        return Err(format!("{}: 크기 {}바이트가 {}를 넘음", p.display(), m.len(), seal_limit_text()));
    }
    #[cfg(test)]
    BETWEEN_STAT_AND_READ.with(|h| {
        if let Some(f) = h.borrow().as_ref() {
            f(p);
        }
    });
    let mut buf = Vec::with_capacity(m.len() as usize);
    (&f).take(SEAL_LIMIT + 1).read_to_end(&mut buf).map_err(|e| format!("{}: {e}", p.display()))?;
    if buf.len() as u64 != m.len() {
        return Err(format!("{}: 읽는 동안 크기가 바뀜({} → {}바이트 · {})", p.display(), m.len(), buf.len(), seal_limit_text()));
    }
    #[cfg(unix)]
    let mode = {
        use std::os::unix::fs::PermissionsExt;
        m.permissions().mode() & 0o7777
    };
    #[cfg(not(unix))]
    let mode = 0u32;
    Ok((buf, mode))
}

#[cfg(test)]
thread_local! {
    /// Test-only: runs between the `fstat` of the open handle and the read (a source growing or
    /// changing while it is copied).
    pub(crate) static BETWEEN_STAT_AND_READ: std::cell::RefCell<Option<Box<dyn Fn(&Path)>>> = const { std::cell::RefCell::new(None) };
}

/// One sealed committed copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealedCopy {
    pub rel: String,
    pub slot: PathBuf,
    /// The state of the sealed bytes (hash + the mode of the handle they were read from).
    pub state: TargetState,
}

/// RULINGS HI2 / HJ2 — seals the bytes of `live` (the repair's target `rel` in the ORIGIN lane) at
/// `commit`: ONE bounded read ([`read_pack_file_bounded`]), then a FRESH slot of R's vault —
/// `displaced/R-<n>/<시각>[-n]/payload/<basename>` published new (temporary, full flush, no-replace
/// placement) and a write-once `meta/pre.json` `{…, op: "commit", kind: "committed-copy", sha256,
/// mode}` — every folder flushed. The caller compares the returned state with the state it verified
/// (never the other way round) before it publishes the commit.
pub fn seal_committed_copy_in(root: &Path, id: &RepairId, lane: &LaneKey, rel: &str, live: &Path, now: f64) -> Result<SealedCopy, String> {
    let (bytes, mode) = read_pack_file_bounded(live)?;
    let sha256 = sha256_hex(&bytes);
    let vault = ledger::vault_dir_in(root, id);
    std::fs::create_dir_all(&vault).map_err(|e| format!("{}: {e}", vault.display()))?;
    let base_name = ledger::utc_slot_name(now);
    let mut slot = vault.join(&base_name);
    let mut n = 0u32;
    loop {
        match std::fs::create_dir(&slot) {
            Ok(()) => break,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && n < 999 => {
                n += 1;
                slot = vault.join(format!("{base_name}-{n}"));
            }
            Err(e) => return Err(format!("{}: {e}", slot.display())),
        }
    }
    let payload = slot.join("payload");
    let meta = slot.join("meta");
    for d in [&payload, &meta] {
        std::fs::create_dir(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    let basename = super::vault::basename(rel);
    apply::write_new(&payload.join(&basename), &bytes, mode).map_err(|e| format!("{e:?}"))?;
    let pre = serde_json::json!({
        "schema": super::REPAIR_SCHEMA,
        "repair": id.as_str(),
        "lane_key": lane.0,
        "rel": rel,
        "op": "commit",
        "kind": COMMITTED_COPY_KIND,
        "sha256": sha256,
        "mode": mode,
        "link_target": serde_json::Value::Null,
    });
    let mut pre_bytes = serde_json::to_vec_pretty(&pre).map_err(|e| e.to_string())?;
    pre_bytes.push(b'\n');
    apply::write_new(&meta.join("pre.json"), &pre_bytes, 0o644).map_err(|e| format!("{e:?}"))?;
    for d in [&payload, &meta, &slot, &vault] {
        durable::full_flush_dir(d).map_err(|e| format!("{e:?}"))?;
    }
    Ok(SealedCopy { rel: rel.to_string(), slot, state: TargetState::File { sha256, mode } })
}

/// RULINGS HI2 / P6 — the committed copies an in-force repair still depends on (every lane takes R's
/// bytes from them): the slots whose write-once metadata names R, R's origin lane, `committed-copy`
/// and R's committed state of that file (hash and mode). Metadata only — no payload is read. A slot
/// whose metadata cannot be read is kept as well (never deleted on a doubt); a copy of other bytes
/// (a refused or interrupted commit) is not needed and stays for `cleanup` like any other slot.
pub fn committed_copy_slots_in(root: &Path, rec: &RepairRecord) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for s in ledger::list_slots_in(root, &rec.id) {
        let v: serde_json::Value = match small_bytes(&s.dir.join("meta").join("pre.json"), JOB_STATE_MAX_BYTES) {
            FileRead::Absent => continue, // no metadata (positive evidence): not a committed copy
            FileRead::Value(b) => match serde_json::from_slice(&b) {
                Ok(v) => v,
                Err(_) => {
                    out.push(s.dir.clone()); // unreadable metadata: kept
                    continue;
                }
            },
            FileRead::Malformed(_) | FileRead::ReadError(_) => {
                out.push(s.dir.clone()); // unreadable metadata: kept
                continue;
            }
        };
        if v["kind"] != COMMITTED_COPY_KIND || v["repair"] != rec.id.as_str() || v["lane_key"] != rec.origin_lane.0.as_str() {
            continue;
        }
        let sealed = match (v["rel"].as_str(), v["sha256"].as_str(), v["mode"].as_u64()) {
            (Some(rel), Some(sha256), Some(mode)) => Some((rel, TargetState::File { sha256: sha256.to_string(), mode: mode as u32 })),
            _ => None,
        };
        match sealed {
            Some((rel, st)) if result_state(rec, rel).as_ref() == Ok(&st) => out.push(s.dir.clone()),
            Some(_) => {}
            None => out.push(s.dir.clone()), // a committed copy whose metadata lacks a field: kept
        }
    }
    out
}

/// The file holding R's committed bytes of `rel`: a committed copy of R's vault whose write-once
/// metadata names (R, the origin lane, `rel`, `committed-copy`) and whose bytes — read through the
/// bounded path, from one handle — hash, with that handle's mode, to R's committed state of `rel`.
/// Content-addressed: a copy left by a refused or interrupted commit holds other bytes (never
/// accepted) or the same bytes (equally valid). `Err` = the true reason (→ the lane's row 충돌).
pub fn committed_copy_in(root: &Path, rec: &RepairRecord, rel: &str) -> Result<PathBuf, String> {
    let want = result_state(rec, rel)?;
    let none = || format!("{rel}: 고친 부서가 확정 때 남긴 결과 사본이 없거나 읽을 수 없거나 확정 상태와 다릅니다 — 이 부서의 워커가 합칩니다");
    let basename = super::vault::basename(rel);
    for s in ledger::list_slots_in(root, &rec.id) {
        let meta = match small_bytes(&s.dir.join("meta").join("pre.json"), JOB_STATE_MAX_BYTES) {
            FileRead::Value(b) => b,
            _ => continue, // a slot without readable metadata is not a committed copy
        };
        let Ok(v) = serde_json::from_slice::<serde_json::Value>(&meta) else { continue }; // not a committed copy (unreadable metadata)
        if v["kind"] != COMMITTED_COPY_KIND || v["repair"] != rec.id.as_str() || v["rel"] != rel || v["lane_key"] != rec.origin_lane.0.as_str() {
            continue;
        }
        let holder = s.dir.join("payload").join(&basename);
        if let Ok((bytes, mode)) = read_pack_file_bounded(&holder) {
            if (TargetState::File { sha256: sha256_hex(&bytes), mode }) == want {
                return Ok(holder);
            }
        }
    }
    Err(none())
}

/// The general apply: copy-check of the whole set, then every target not yet at R's result through
/// T6's engine (`run_op`, kind `propagate`), deletions tombstoned in THIS lane's file, the hook lines,
/// the row `적용됨`, then this lane's own check → `검증됨` (with the probation bundle — C11 11.4) or
/// `충돌` with the return owed to T6's probation.
#[allow(clippy::too_many_arguments)]
fn apply_general(
    env: &LaneEnv<'_>,
    stored: Option<&LaneRecord>,
    row: LaneRecord,
    rec: &RepairRecord,
    positions: &[(String, Position)],
    prev_applied: Option<&LaneRecord>,
    bound: &Bound,
    stamp: &Stamp,
    report: &mut TickReport,
) -> Result<Option<LaneState>, RepairError> {
    let pack = env.op.pack;
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let scratch = Scratch(env.op.root.join("lanes").join(format!("{}{}-{nanos}", scratch_prefix(env.op.lane), std::process::id())));
    // 11.2: the would-be state of every file target in a COPY (nothing live is touched yet)
    for (rel, pos) in positions {
        let result = match result_state(rec, rel) {
            Ok(r) => r,
            Err(why) => return finish(env, rec, stored, row, LaneState::Conflict, &why, stamp, report),
        };
        if !matches!(result, TargetState::File { .. }) {
            continue;
        }
        let dst = scratch.0.join(rel);
        let got = match pos {
            Position::AtResult => copy_checked(&pack.join(rel), &dst, &result),
            _ => {
                // the lane's own bytes already ARE the result's (a mode-only change): no foreign bytes
                let pre = ledger::file_targets(rec).find(|f| f.rel == *rel).map(|f| f.pre.state());
                let own_bytes_are_result = match pos {
                    Position::AtPre => matches!((&pre, &result), (Some(TargetState::File { sha256: a, .. }), TargetState::File { sha256: b, .. }) if a == b),
                    Position::AtPrevApplied => same_bytes(prev_applied, rel, &result),
                    _ => false,
                };
                if own_bytes_are_result {
                    copy_checked_bytes_of(&pack.join(rel), &dst, &result)
                } else {
                    match (env.result_bytes)(rec, rel) {
                        Ok(holder) => copy_checked(&holder, &dst, &result),
                        Err(why) => Err(why),
                    }
                }
            }
        };
        if let Err(why) = got {
            return finish(env, rec, stored, row, LaneState::Conflict, &format!("{rel}: 수리 결과의 바이트를 마련하지 못했습니다 — {why}"), stamp, report);
        }
    }
    // the copy-check runs OUTSIDE the pack-apply lock (IK3)
    if let Err(why) = run_checks(env, rec, &scratch.0, false) {
        return finish(env, rec, stored, row, LaneState::Conflict, &format!("사본 검사 실패 — 살아 있는 파일은 바꾸지 않았습니다 · {why}"), stamp, report);
    }
    // IK3 + IG5: the door operations, the tombstones, the hook step and the 적용됨 row under ONE tried
    // lock; the record is read AGAIN before the first operation, before each further target and before
    // the row is written
    // IV1: the door works on a copy of the decision's stamp; each of its operations updates it with
    // what the door itself placed — that stamp is what the live check below is published against
    let mut stamp = stamp.clone();
    let ran = crate::pack_apply_lock::try_with_apply_lock(env.apply_lock, || door_phase(env, stored, &row, rec, positions, prev_applied, bound, &scratch, &mut stamp));
    let end = match ran {
        Ok(Some(r)) => r?,
        Ok(None) => DoorEnd::Left(LOCK_BUSY_ROW.to_string(), false),
        Err(why) => DoorEnd::Left(format!("팩 적용 잠금을 확인할 수 없습니다 — {why}"), false),
    };
    drop(scratch);
    let mut row = match end {
        DoorEnd::Applied(r) => r,
        DoorEnd::Conflict(why) => return finish(env, rec, stored, row, LaneState::Conflict, &why, &stamp, report),
        DoorEnd::Left(why, new_row) => {
            report.left.push((rec.id.clone(), why.clone()));
            if new_row && stored.is_none() {
                let mut row = row;
                row.reason = why.chars().take(400).collect();
                publish_row(env, rec, None, &row, &stamp, report)?;
            }
            return Ok(None);
        }
        DoorEnd::ChangedMidway(why) => {
            report.rows.push((rec.id.clone(), LaneState::Conflict, why));
            return Ok(Some(LaneState::Conflict));
        }
    };
    #[cfg(test)]
    seam(&AFTER_DOOR_PHASE, env.op.pack);
    let applied_row = row.clone();
    match run_checks(env, rec, pack, true) {
        Ok(timings) => {
            if let Some(p) = row.probation.as_mut() {
                p.check_timings = timings;
            }
            finish_as(env, rec, Some(&applied_row), row, LaneState::Verified, "적용 뒤 이 부서의 검사 통과", Publication::checked(&stamp, true), report)
        }
        Err(why) => {
            // the lane's own failure: T6's probation owes the return of THIS lane's operations
            // (macOS); on Windows the automatic return is off — reported only
            if let Some(p) = row.probation.as_mut() {
                p.closed = Some(failed_closure(env, &why));
            }
            finish_as(env, rec, Some(&applied_row), row, LaneState::Conflict, &format!("적용 뒤 이 부서의 검사 실패 — {why}"), Publication::checked(&stamp, false), report)
        }
    }
}

/// The door phase of one repair in this lane, run under ONE tried pack-apply lock (IK3 — no check in
/// it): per target the record read AGAIN (IG5 / R3), then `apply::run_op` kind `propagate`, then a real
/// deletion's tombstone in THIS lane's file; the hook step (a target of its own); the record read once
/// more; the row published as 적용됨 with its probation bundle.
#[allow(clippy::too_many_arguments)]
fn door_phase(
    env: &LaneEnv<'_>,
    stored: Option<&LaneRecord>,
    row: &LaneRecord,
    rec: &RepairRecord,
    positions: &[(String, Position)],
    prev_applied: Option<&LaneRecord>,
    bound: &Bound,
    scratch: &Scratch,
    stamp: &mut Stamp,
) -> Result<DoorEnd, RepairError> {
    let mut done: Vec<String> = Vec::new();
    // IV1 + IO1: the stamp measured again before every write; a difference before the first write →
    // nothing written; MIDWAY → the 충돌 row published now (naming what this lane already changed)
    let changed = |done: &Vec<String>, why: &'static str| -> Result<DoorEnd, RepairError> {
        if done.is_empty() {
            Ok(DoorEnd::Left(why.to_string(), false))
        } else {
            changed_midway(env, stored, row, rec, done, why)
        }
    };
    // rule one per target through T6's engine; the binding was born at the tick's start (EJ1)
    for (rel, pos) in positions {
        if *pos == Position::AtResult {
            continue;
        }
        if let Err(why) = stamp.holds(env, rec, true) {
            return changed(&done, why);
        }
        let result = match result_state(rec, rel) {
            Ok(r) => r,
            Err(why) => return Ok(DoorEnd::Conflict(why)),
        };
        let expected = match pos {
            Position::AtPrevApplied => match prev_applied.map(|p| Recorded::applied_in(p, rel)) {
                Some(Ok(r)) => r,
                _ => return Ok(DoorEnd::Conflict(format!("{rel}: 이전에 적용한 상태를 읽을 수 없습니다"))),
            },
            Position::AtVendor(r) => r.clone(),
            _ => Recorded::pre_of(rec, rel),
        };
        let source = match &result {
            TargetState::File { .. } => Source::Fresh(scratch.0.join(rel)),
            _ => Source::None,
        };
        let spec = OpSpec {
            id: rec.id.clone(),
            rel: rel.clone(),
            kind: "propagate".into(),
            expected,
            incoming: result.clone(),
            source,
            bound: bound.clone(),
        };
        match apply::run_op(&env.op, &spec)?.outcome {
            ApplyOutcome::Applied => {
                // IV1: the stamp now holds what the door itself placed (never a read after the lock)
                stamp.placed(rel, &result);
                done.push(rel.clone());
            }
            ApplyOutcome::Superseded(m) => return Ok(DoorEnd::Left(format!("{rel}: {m}"), true)),
            ApplyOutcome::Declined(m) => return Ok(DoorEnd::Conflict(format!("{rel}: {m}"))),
        }
        #[cfg(test)]
        if STOP_AFTER_DOOR_OP.with(|c| c.replace(false)) {
            return Err(RepairError::Io(format!("{} after the door operation of {rel}", apply::INJECTED_STOP)));
        }
        // B1 / AD3: a real deletion is tombstoned in THIS lane's own file
        let pre_present = ledger::file_targets(rec).find(|f| f.rel == *rel).is_some_and(|f| f.pre != ledger::PreImage::Absent);
        if result == TargetState::Absent && pre_present {
            let t = super::tombstones::Tombstone { rel: rel.clone(), kind: super::tombstones::TombstoneKind::File, repair: rec.id.clone() };
            if let Err(e) = super::tombstones::record_in(env.op.root, env.op.lane, &t) {
                return Ok(DoorEnd::Conflict(format!("{rel}: 삭제 표식(묘비) 기록 실패 — {e:?}")));
            }
        }
    }
    if ledger::hook_lines(rec).next().is_some() {
        if let Err(why) = stamp.holds(env, rec, true) {
            return changed(&done, why);
        }
        if let Some(why) = hook_step(env, rec) {
            return Ok(DoorEnd::Conflict(why));
        }
        done.push("훅 등록 해제".into());
    }
    if let Err(why) = stamp.holds(env, rec, true) {
        return changed(&done, why);
    }
    // IG2: an applied row of a deleting repair implies this lane's tombstones
    if let Err(why) = ensure_tombstones(env, rec) {
        return Ok(DoorEnd::Conflict(why));
    }
    // 적용됨, durably, before this lane's own check (a stop here is resumed by the next tick's check)
    let mut row = row.clone();
    row.state = LaneState::Applied;
    row.applied = rec.committed.clone();
    row.merge = None;
    row.merge_base.clear();
    row.merge_lock = None;
    row.conflict_live.clear();
    row.approval_ref = None;
    let (version, pro) = bound.row_fields();
    row.probation = Some(ledger::Probation {
        pack_generation: version,
        pro_revision: pro,
        applied_at: env.op.now,
        expected: rec.committed.clone(),
        closed: None,
        check_timings: Vec::new(),
        check_timeouts: Vec::new(),
    });
    row.updated_at = env.op.now;
    if !write_row_if(env, stored, &row)? {
        return Ok(DoorEnd::Left("적용한 뒤 다른 작성자가 행을 바꿔 다음 차례에 검사합니다".into(), false));
    }
    Ok(DoorEnd::Applied(row))
}

/// RULINGS IO1 — the record changed BETWEEN two targets of this lane's apply (a writer that takes no
/// pack-apply lock — T3's `commit` superseding an older repair — or Windows): no undo in 0.15.0. The
/// remaining targets are not applied and the row is PUBLISHED NOW as 충돌 — called inside the door
/// phase's lock, so without asking the record to be unchanged (the row describes what THIS lane did,
/// not what R is now) — with the reason naming the targets already changed and the live states
/// (`conflict_live`). Nothing is put back, nothing further is written; the lane's worker merges it.
fn changed_midway(env: &LaneEnv<'_>, stored: Option<&LaneRecord>, row: &LaneRecord, rec: &RepairRecord, done: &[String], cause: &str) -> Result<DoorEnd, RepairError> {
    let what = if cause == RECORD_CHANGED { "수리 기록이 바뀌었습니다(대체·다시 확정·되돌림)" } else { "이 부서의 대상 파일·팩 세대·팩 식별 중 하나가 바뀌었습니다" };
    let why = format!(
        "이 부서에 적용하는 도중 {what} — 이미 바꾼 대상: {} · 나머지 대상은 바꾸지 않았고 아무것도 되돌리지 않았습니다 — 이 부서의 워커가 합칩니다(cys repair status {})",
        done.join(", "),
        rec.id.as_str()
    );
    let mut row = row.clone();
    row.state = LaneState::Conflict;
    row.reason = why.chars().take(400).collect();
    row.conflict_live = match live_states(env.op.pack, rec) {
        Ok(l) => canon_list(&l),
        Err(e) => vec![("?".into(), e)], // shown as it is: the state could not be read
    };
    row.updated_at = env.op.now;
    if !write_row_if(env, stored, &row)? {
        return Ok(DoorEnd::Left("다른 작성자가 이 행을 바꿔 다음 차례에 다시 봅니다".into(), false));
    }
    Ok(DoorEnd::ChangedMidway(why))
}

fn same_bytes(prev: Option<&LaneRecord>, rel: &str, result: &TargetState) -> bool {
    let prev_state = prev.and_then(|p| p.applied.iter().find(|(r, _)| r == rel)).and_then(|(_, c)| apply::state_of_canon(c));
    matches!((prev_state, result), (Some(TargetState::File { sha256: a, .. }), TargetState::File { sha256: b, .. }) if a == *b)
}

// ───────────────────────────── `recheck` (H8) ─────────────────────────────

/// RULINGS H8 — the lane's re-evaluation after its worker's merge: a general repair's `충돌` row
/// whose merge lock this seat holds; R's check on the LIVE files → `검증됨` with the merge record
/// `{by_seat, result (rel, canon), changed, note}` (V2(c)), the merge lock released; a failure leaves
/// `충돌` (the lock kept). A boot-critical / protection-related path is refused (R1 / T1).
#[allow(clippy::too_many_arguments)]
pub fn recheck_lane_in(
    env: &LaneEnv<'_>,
    id: &RepairId,
    seat: u64,
    owner: &super::types::LockOwner,
    incarnation: &str,
    note: &str,
) -> Result<LaneState, RepairError> {
    let root = env.op.root;
    let lane = env.op.lane;
    let rec = ledger::read_record_in(root, id)?.ok_or_else(|| RepairError::Precondition(format!("{}: 그런 수리가 없습니다", id.as_str())))?;
    let mut approval = is_approval_class(rec.classification);
    for f in ledger::file_targets(&rec) {
        approval |= is_approval_class((env.classify)(&f.rel)?);
    }
    if approval {
        return Err(RepairError::Precondition(format!(
            "{}: 기동 필수·보호장치 경로는 합치기 모드로 직접 고치지 않습니다 — `cys repair begin --files … --trigger … --resolves {}` 로 이 부서의 수리를 따로 시작하세요(RULINGS R1/T1)",
            id.as_str(),
            id.as_str()
        )));
    }
    if !in_force(&rec) || !concerns_lane(&rec, lane) {
        return Err(RepairError::Precondition(format!("{}: 이 부서에 적용할 수리가 아닙니다('{}')", id.as_str(), rec.state.label_ko())));
    }
    let Some(_guard) = super::guard::RepairGuard::try_take(root, id) else {
        return Err(RepairError::Locked(format!("{}: {}", id.as_str(), super::guard::BUSY)));
    };
    let stored = ledger::read_lane_in(root, id, lane)?
        .ok_or_else(|| RepairError::Precondition(format!("{}: 이 부서에는 합칠 충돌이 없습니다", id.as_str())))?;
    if stored.state != LaneState::Conflict {
        return Err(RepairError::Precondition(format!("{}: 이 부서의 상태가 '충돌'이 아닙니다({})", id.as_str(), stored.state.label_ko())));
    }
    match super::lock::view(stored.merge_lock.as_ref(), owner, env.op.now, lane, incarnation) {
        super::lock::LockView::Mine => {}
        _ => {
            return Err(RepairError::Locked(format!(
                "{}: 이 부서의 합치기 잠금을 이 좌석이 쥐고 있지 않습니다 — 먼저 `cys repair begin --files … --joins {} --trigger …`",
                id.as_str(),
                id.as_str()
            )))
        }
    }
    let live = live_states(env.op.pack, &rec).map_err(RepairError::Precondition)?;
    // RULINGS IV4 — the same stamp and the same publication as the tick: fixed now, at the decision
    let stamp = Stamp::take(env, &rec, Some(&live));
    #[cfg(test)]
    seam(&AFTER_DECISION, env.op.pack);
    let mut row = stored.clone();
    // the hook step (it writes the registrar's file and the lane's tombstones) inside the tried lock,
    // after the stamp was measured again — never with no lock or before the record was read again
    match locked_hook_step(env, &rec, &stamp) {
        Ok(None) => {}
        Ok(Some(why)) => return Err(RepairError::CheckFailed(why)),
        Err(why) => return Err(RepairError::Locked(format!("{}: {why} — 다시 하세요", id.as_str()))),
    }
    let refused = |p: Published| -> RepairError {
        let why = match p {
            Published::RecordChanged => RECORD_CHANGED.to_string(),
            Published::StatesChanged => STATES_CHANGED.to_string(),
            Published::LockBusy => LOCK_BUSY_ROW.to_string(),
            Published::NotWritten(w) => w,
            Published::OtherWriter => "이 부서의 행이 그 사이 바뀌었습니다".to_string(),
            Published::Written => String::new(),
        };
        RepairError::Locked(format!("{}: {why} — 아무것도 쓰지 않았습니다. 다시 하세요", id.as_str()))
    };
    match run_checks(env, &rec, env.op.pack, false) {
        Ok(_) => {
            let result = canon_list(&live);
            row.merge = Some(MergeRecord {
                by_seat: seat,
                changed: result != row.merge_base,
                result,
                note: note.chars().take(400).collect(),
                via_repair: None,
            });
            row.applied = rec.committed.clone();
            row.state = LaneState::Verified;
            row.merge_lock = None;
            row.conflict_live.clear();
            row.probation = None;
            row.reason = "합친 뒤 이 부서의 살아 있는 파일에서 검사 통과(recheck)".into();
            row.updated_at = env.op.now;
            // IQ1 + IQ2 + IV4: the checked publication (the stamp measured again; a deleting repair's
            // tombstones in the same unit as the 검증됨 row)
            match publish(env, &rec, Some(&stored), &row, Publication::checked(&stamp, true))? {
                Published::Written => Ok(LaneState::Verified),
                other => Err(refused(other)),
            }
        }
        Err(why) => {
            row.conflict_live = canon_list(&live);
            row.reason = format!("합친 뒤의 다시 검사 실패 — {why}").chars().take(400).collect();
            row.updated_at = env.op.now;
            match publish(env, &rec, Some(&stored), &row, Publication::checked(&stamp, false))? {
                Published::Written => Err(RepairError::CheckFailed(format!("{}: 이 부서의 살아 있는 파일에서 검사가 실패해 '충돌'로 남습니다 — {why}", id.as_str()))),
                other => Err(refused(other)),
            }
        }
    }
}

#[cfg(test)]
#[path = "propagate_tests.rs"]
mod tests;
