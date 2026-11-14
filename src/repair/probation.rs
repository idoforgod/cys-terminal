//! The ONE appended built-in job's work (C7 · design §4-9 "시험 기간의 재검사", §4-10 M10): what a
//! tick decides (a new daemon incarnation → the start-time pass and the probation checks at once;
//! otherwise probation checks when 10 minutes passed, the unapproved-change scan when an hour
//! passed), the 24-hour re-check of repaired files with RULINGS O3's timeout attribution, the end of
//! probation (N3), the hourly scan of this lane's boot-critical files (M10, F9), and the start-time
//! pass (RULINGS AK7 + DEFERRED_MINOR #49).
//!
//! Outputs are LEDGER FILES ONLY (lane rows, records, at most one intake row per reverted repair,
//! the M10 findings file) — nothing is sent to a seat's queue, an alert route or the event bus (F1).
//! Every check is a bounded child run one at a time by the caller's runner (T5b's
//! `rehearsal::run_check`, killed and reaped at its deadline).

use super::apply::{
    self, check_deadline, check_kind_of, pre_image_for, revert_in, ApplyOutcome, CheckFn, CheckOutcome, HookStep, HookStepFn, OpEnv, OpSpec, ReferenceFn,
    RegistrarFn, RepairLock, RevertCaller,
};
use super::ledger::{self, JobState, LaneRecord, PackGeneration, PreImage, RepairRecord};
use super::platform::ApplyForm;
use super::rehearsal::{CheckResult, REFERENCE_SLOWDOWN_PERMILLE};
use super::types::{LaneKey, RepairError, RepairId, RepairState, TargetState};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Everything the job needs (the daemon builds it; tests build it with doubles).
pub struct JobEnv<'a> {
    pub op: OpEnv<'a>,
    pub incarnation: &'a str,
    pub check: &'a CheckFn<'a>,
    pub reference: &'a ReferenceFn<'a>,
    pub hooks: &'a HookStepFn<'a>,
    pub registrar: &'a RegistrarFn<'a>,
    pub locked: &'a RepairLock<'a>,
    /// Rule documents — never reverted automatically (§4-10).
    pub excluded: &'a dyn Fn(&str) -> bool,
    /// Our original of the CURRENT pack generation for a rel (`None` = not known for this
    /// generation — then nothing is put back).
    pub original: &'a dyn Fn(&str) -> Option<Vec<u8>>,
    /// One intake row (data, never a message): (rel, error line).
    pub intake: &'a dyn Fn(&str, &str),
    /// The existing pack-apply lock file (taken around every put-back, blocking on the job thread).
    pub apply_lock: &'a Path,
}

// ───────────────────────────── job state and the tick decision ─────────────────────────────

/// `<root>/lanes/<lane>.job.json` (the layout of `paths::job_state_path`).
pub fn job_state_path_in(root: &Path, lane: &LaneKey) -> PathBuf {
    root.join("lanes").join(format!("{}.job.json", lane.0))
}

/// The lane's job state (`None` = never written or unreadable — treated as "nothing recorded").
pub fn read_job_state_in(root: &Path, lane: &LaneKey) -> Option<JobState> {
    let b = std::fs::read(job_state_path_in(root, lane)).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&b).ok()?;
    if v.get("schema").and_then(|s| s.as_u64()).unwrap_or(u64::MAX) > u64::from(super::REPAIR_SCHEMA) {
        return None;
    }
    serde_json::from_value(v).ok()
}

/// What a tick has to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Due {
    /// A daemon incarnation this lane's job has not run in yet.
    pub first_of_incarnation: bool,
    pub probation: bool,
    pub sweep: bool,
}

impl Due {
    pub fn anything(&self) -> bool {
        self.first_of_incarnation || self.probation || self.sweep
    }
}

/// C7 7.2 — the tick's own decision (pure): a new incarnation → the probation checks at once;
/// otherwise probation when 10 min passed since its last check, the scan when 1 h passed.
pub fn due(state: Option<&JobState>, incarnation: &str, now: f64) -> Due {
    let first = state.is_none_or(|s| s.last_incarnation != incarnation);
    let probation = first
        || state.is_none_or(|s| now - s.last_probation_check >= ledger::PROBATION_RECHECK_SECS as f64);
    let sweep = state.is_none_or(|s| now - s.last_sweep >= ledger::SWEEP_SECS as f64);
    Due {
        first_of_incarnation: first,
        probation,
        sweep,
    }
}

fn write_job_state(root: &Path, lane: &LaneKey, incarnation: &str, now: f64, d: &Due) -> Result<(), RepairError> {
    let mut st = read_job_state_in(root, lane).unwrap_or(JobState {
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
    });
    st.schema = super::REPAIR_SCHEMA;
    st.last_incarnation = incarnation.to_string();
    if d.probation {
        st.last_probation_check = now;
    }
    if d.sweep {
        st.last_sweep = now;
    }
    super::durable::write_json_durable(&job_state_path_in(root, lane), &st)
}

/// What one tick did (ledger only — the daemon returns it in its reply, nothing is pushed).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TickReport {
    pub resumed: u32,
    pub lines: Vec<String>,
    pub reverted: Vec<RepairId>,
    pub ended: Vec<RepairId>,
    pub superseded: Vec<RepairId>,
    pub timeouts: Vec<(RepairId, String)>,
    pub reported_only: Vec<(RepairId, String)>,
    pub m10: Vec<Finding>,
}

/// One tick of the job (the daemon calls it on its job thread, after the readiness check and the
/// single-flight claim). `force_sweep` = the preflight's `repair.sweep`.
pub fn run_tick(env: &JobEnv<'_>, force_sweep: bool) -> Result<TickReport, RepairError> {
    let root = env.op.root;
    let lane = env.op.lane;
    let now = env.op.now;
    let mut d = due(read_job_state_in(root, lane).as_ref(), env.incarnation, now);
    if force_sweep {
        d.sweep = true;
    }
    let mut report = TickReport::default();
    if !d.anything() {
        return Ok(report);
    }
    // interrupted operations are finished or abandoned from their records at every run
    let m10_root = m10_root_in(root, lane);
    let m10_env = OpEnv {
        root: &m10_root,
        pack: env.op.pack,
        lane,
        now,
        form: env.op.form,
        faults: env.op.faults,
        own_check: env.op.own_check,
    };
    // RULINGS CT5: recovery of unfinished operations (and of CT4's apply intents) runs under the
    // pack-apply lock, TRIED — held means a writer is ACTIVE ("unfinished" is not "interrupted"):
    // this tick then does nothing at all and its job state is not written (due again next time);
    // the unfinished records are read only after the lock is taken
    let recovered = crate::pack_apply_lock::try_with_apply_lock(env.apply_lock, || -> Result<(u32, Vec<String>), RepairError> {
        let mut lines = Vec::new();
        let mut n = 0u32;
        let injected = |e: &RepairError| format!("{e:?}").contains(apply::INJECTED_STOP);
        match apply::resume_interrupted_in(&env.op) {
            Ok(k) => n += k,
            Err(e) if injected(&e) => return Err(e),
            Err(e) => lines.push(format!("끊긴 동작을 마저 하지 못했습니다 — 다음 차례에 다시({e:?})")),
        }
        n += apply::resume_hook_steps_in(root, env.op.pack, lane, env.registrar, env.locked, now);
        if m10_root.exists() {
            match apply::resume_interrupted_in(&m10_env) {
                Ok(k) => n += k,
                Err(e) if injected(&e) => return Err(e),
                Err(e) => lines.push(format!("M10 의 끊긴 동작을 마저 하지 못했습니다 — 다음 차례에 다시({e:?})")),
            }
        }
        lines.extend(apply::recover_apply_intents_in(&env.op, env.locked));
        Ok((n, lines))
    });
    match recovered {
        Ok(Some(r)) => {
            let (n, lines) = r?;
            report.resumed = n;
            report.lines.extend(lines);
        }
        Ok(None) => {
            report.lines.push("다른 적용이나 업데이트가 팩을 바꾸는 중 — 이번 차례는 아무것도 하지 않습니다".into());
            return Ok(report);
        }
        Err(e) => return Err(RepairError::Io(e)),
    }
    if d.first_of_incarnation {
        report.lines.extend(start_time_pass(env)?);
    }
    if d.probation {
        probation_pass(env, &mut report)?;
    }
    if d.sweep {
        report.m10 = m10_sweep(env)?;
    }
    write_job_state(root, lane, env.incarnation, now, &d)?;
    Ok(report)
}

// ───────────────────────────── the start-time pass (AK7 + DEFERRED #49) ─────────────────────────────

fn in_force(rec: &RepairRecord) -> bool {
    matches!(rec.state, RepairState::Probation | RepairState::InUse)
}

/// At the first ready tick of every daemon incarnation: (a) every committed `Absent` of a repair in
/// force rebuilds its File tombstone in this lane (AK7 — a crash right after a deletion commit, or
/// a revert of a later repair, never leaves a committed deletion without its tombstone); (b) the
/// idempotent parts of an interrupted commit's second step (DEFERRED_MINOR #49): the hook step of a
/// committed repair whose unregistration never finished, older in-force repairs on the same targets
/// superseded, the origin lane's row written when it is missing.
pub fn start_time_pass(env: &JobEnv<'_>) -> Result<Vec<String>, RepairError> {
    let root = env.op.root;
    let lane = env.op.lane;
    let mut lines = Vec::new();
    let mut recs: Vec<RepairRecord> = ledger::list_ids_in(root)
        .into_iter()
        .filter_map(|id| ledger::read_record_in(root, &id).ok().flatten())
        .collect();
    recs.sort_by(|a, b| b.id.cmp(&a.id)); // newest first (AK4 supersedes the older ones)
    for rec in &recs {
        if !in_force(rec) {
            continue;
        }
        let row = ledger::read_lane_in(root, &rec.id, lane).ok().flatten();
        // (a) tombstones of committed deletions
        let applied: Vec<(String, String)> = if rec.origin_lane == *lane {
            rec.committed.clone()
        } else {
            row.as_ref().map(|r| r.applied.clone()).unwrap_or_default()
        };
        for (rel, canon) in &applied {
            let pre_present = ledger::file_targets(rec)
                .find(|f| f.rel == *rel)
                .is_some_and(|f| f.pre != PreImage::Absent);
            if canon == "absent" && pre_present {
                let t = super::tombstones::Tombstone {
                    rel: rel.clone(),
                    kind: super::tombstones::TombstoneKind::File,
                    repair: rec.id.clone(),
                };
                if super::tombstones::load_in(root, lane).is_ok_and(|ts| !ts.contains(&t)) {
                    super::tombstones::record_in(root, lane, &t)?;
                    lines.push(format!("{}: 확정된 삭제의 묘비를 다시 세웠습니다 — {rel}", rec.id.as_str()));
                }
            }
        }
        // CT7: a hook-only repair (no committed file) is recovered too — its hook step is its state
        if rec.origin_lane != *lane || (rec.committed.is_empty() && ledger::hook_lines(rec).next().is_none()) {
            continue;
        }
        // (b3) the origin's own row FIRST (the hook step below marks it 충돌 when it cannot finish).
        // CU4: it records the COMMIT's generation — confirmed when the pack's generation files are
        // older than the record's durable commit write — else it is written closed 대체됨 (nothing is
        // re-checked or reverted against bytes of a generation nobody can confirm)
        if row.is_none() {
            let gen = apply::pack_generation(env.op.pack);
            let confirmed = generation_unchanged_since_commit(root, env.op.pack, &rec.id);
            let new_row = LaneRecord {
                schema: super::REPAIR_SCHEMA,
                repair: rec.id.clone(),
                lane: lane.clone(),
                state: super::types::LaneState::Verified,
                probation: Some(ledger::Probation {
                    pack_generation: gen.version,
                    pro_revision: gen.pro_revision,
                    applied_at: env.op.now,
                    expected: rec.committed.clone(),
                    closed: (!confirmed).then(|| "대체됨 — 확정 때의 팩 세대를 확인할 수 없어 시험 기간을 다시 열지 않습니다(워커 검토)".to_string()),
                    check_timings: Vec::new(),
                    check_timeouts: Vec::new(),
                }),
                merge: None,
                merge_history: Vec::new(),
                approval_ref: rec.approval.as_ref().map(|a| a.request_id.clone()),
                conflict_live: Vec::new(),
                applied: rec.committed.clone(),
                applied_history: Vec::new(),
                merge_base: Vec::new(),
                merge_lock: None,
                merge_attempt: 0,
                updated_at: env.op.now,
            };
            let rootb = root.to_path_buf();
            let idc = rec.id.clone();
            let lanec = lane.clone();
            (env.locked)(&rec.id, &mut || {
                if ledger::read_lane_in(&rootb, &idc, &lanec)?.is_none() {
                    ledger::write_lane_in(&rootb, &new_row)?;
                }
                Ok(())
            })?;
            lines.push(format!(
                "{}: 끊긴 확정의 부서 행을 썼습니다{}",
                rec.id.as_str(),
                if confirmed { "" } else { "(세대를 확인할 수 없어 대체됨으로 닫음)" }
            ));
        }
        // (b1) the hook step of the committed repair, when its unregistration never finished
        if ledger::hook_lines(rec).next().is_some() {
            let (ops, _) = super::wal::list_in(root, &rec.id);
            let done = ops.iter().any(|o| o.kind == "hook-unregister" && o.finished && o.outcome.as_deref() == Some("applied"));
            let attempted = ops.iter().any(|o| o.kind == "hook-unregister");
            if !done && !attempted {
                match (env.hooks)(&rec.id, lane, HookStep::Unregister) {
                    Ok(_) => lines.push(format!("{}: 끊긴 확정의 훅 단계를 마쳤습니다", rec.id.as_str())),
                    Err(e) => {
                        apply::mark_lane_conflict(root, &rec.id, lane, env.op.now, env.locked);
                        lines.push(format!("{}: 끊긴 확정의 훅 단계 실패 — 충돌로 남깁니다({e:?})", rec.id.as_str()));
                    }
                }
            }
        }
        // (b2) older in-force repairs on the same targets
        for oid in supersede_older(root, lane, rec, env.locked)? {
            lines.push(format!("{}: 같은 파일의 더 새 수리 {} 로 대체됨", oid.as_str(), rec.id.as_str()));
        }
    }
    Ok(lines)
}

/// CU4: the pack generation is confirmed unchanged since a commit when the files that define it
/// (`.pack-version`, the pack state) were last written BEFORE the record's durable commit write.
fn generation_unchanged_since_commit(root: &Path, pack: &Path, id: &RepairId) -> bool {
    let mtime = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    let Some(committed_at) = mtime(&ledger::record_path_in(root, id)) else {
        return false;
    };
    [pack.join(".pack-version"), pack.join(crate::pack::PACK_STATE_FILE)]
        .iter()
        .all(|p| match std::fs::symlink_metadata(p) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => true,
            Err(_) => false,
            Ok(_) => mtime(p).is_some_and(|t| t <= committed_at),
        })
}

fn supersede_older(
    root: &Path,
    lane: &LaneKey,
    newer: &RepairRecord,
    locked: &RepairLock<'_>,
) -> Result<Vec<RepairId>, RepairError> {
    let targets: Vec<String> = ledger::file_targets(newer).map(|f| f.rel.clone()).collect();
    let mut out = Vec::new();
    for oid in ledger::list_ids_in(root).into_iter().filter(|o| *o < newer.id).rev() {
        let mut changed = false;
        let rootb = root.to_path_buf();
        locked(&oid, &mut || {
            let Some(mut r) = ledger::read_record_in(&rootb, &oid)? else {
                return Ok(());
            };
            if r.origin_lane == *lane && in_force(&r) && r.files.iter().any(|f| targets.contains(&f.rel)) {
                r.state = RepairState::Superseded;
                ledger::write_record_durable_in(&rootb, &r)?;
                changed = true;
            }
            Ok(())
        })?;
        if changed {
            out.push(oid);
        }
    }
    Ok(out)
}

// ───────────────────────────── the 24-hour re-check ─────────────────────────────

/// The result of RULINGS O3's attribution of a re-check timeout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Attribution {
    /// An established failure (the control passed in time, the second run timed out again, and for
    /// EVERY target the scaled recorded duration is under the deadline ÷ 1.5) — or the second run
    /// failed outright.
    Failure(String),
    /// Everything else: recorded, one line, nothing moved, tried again at the next due tick.
    CheckTimeout(String),
}

fn passed_in_time(o: &Result<CheckOutcome, RepairError>) -> bool {
    matches!(o, Ok(CheckOutcome { result: CheckResult::Pass, .. }))
}

/// O3 (+ P3, R5, K4, H1, H2, F8): control → second run → reference NOW, one sequence, one decision.
/// The control is the same check with the same deadline against a scratch copy of the repair's
/// PRE-IMAGE (every target at its pre-image, from the sealed deposits; a hash mismatch → timeout),
/// or — for a target the repair CREATED — the built-in reference check of its kind.
pub fn attribute_timeout(env: &JobEnv<'_>, rec: &RepairRecord, row: &LaneRecord, rel: &str) -> Attribution {
    let deadline = check_deadline(rel);
    let pre = match pre_image_for(env.op.root, rec, env.op.lane, rel) {
        Ok(p) => p,
        Err(e) => return Attribution::CheckTimeout(format!("{rel}: 적용 전 내용을 확인할 수 없습니다({e:?}) — 시간 초과로 기록")),
    };
    // (1) the control
    let control = if pre.0 == TargetState::Absent {
        (env.reference)(check_kind_of(rel), deadline)
    } else {
        match scratch_pre_image(env, rec, row) {
            Ok(dir) => {
                let r = (env.check)(rel, &dir, deadline);
                let _ = std::fs::remove_dir_all(&dir); // our own scratch copy (never a slot)
                r
            }
            Err(e) => {
                return Attribution::CheckTimeout(format!("{rel}: 적용 전 사본을 만들 수 없습니다({e:?}) — 시간 초과로 기록"))
            }
        }
    };
    if !passed_in_time(&control) {
        return Attribution::CheckTimeout(format!("{rel}: 대조 검사(적용 전 내용)도 제때 끝나지 않았습니다 — 느린 기계로 봅니다"));
    }
    // (2) the repaired file's check again
    match (env.check)(rel, env.op.pack, deadline) {
        Ok(CheckOutcome { result: CheckResult::Pass, .. }) => {
            return Attribution::CheckTimeout(format!("{rel}: 두 번째 검사는 통과했습니다 — 일시적인 지연으로 봅니다"))
        }
        Ok(CheckOutcome { result: CheckResult::Fail(m), .. }) => return Attribution::Failure(format!("{rel}: {m}")),
        Ok(CheckOutcome { result: CheckResult::Timeout, .. }) => {}
        Err(e) => return Attribution::CheckTimeout(format!("{rel}: 두 번째 검사를 돌릴 수 없습니다({e:?})")),
    }
    // (3) the reference NOW, for EVERY target: recorded duration × (reference now ÷ reference then)
    //     must be under the deadline ÷ 1.5 for every one (no recorded duration → not under)
    let timings = row.probation.as_ref().map(|p| p.check_timings.clone()).unwrap_or_default();
    for (t_rel, canon) in row.probation.as_ref().map(|p| p.expected.clone()).unwrap_or_default() {
        if canon == "absent" {
            continue;
        }
        let dl = check_deadline(&t_rel);
        let Some(t) = timings.iter().find(|t| t.rel == t_rel) else {
            return Attribution::CheckTimeout(format!("{t_rel}: 기록된 검사 시간이 없어 실패로 단정하지 않습니다"));
        };
        let now_ref = match (env.reference)(t.reference_kind, dl) {
            Ok(CheckOutcome { result: CheckResult::Pass, ms }) => ms,
            _ => return Attribution::CheckTimeout(format!("{t_rel}: 기준 검사가 지금 제때 끝나지 않습니다 — 느린 기계로 봅니다")),
        };
        if t.reference_ms == 0 {
            return Attribution::CheckTimeout(format!("{t_rel}: 기록된 기준 검사 시간이 없습니다"));
        }
        // scaled = check_ms × now_ref ÷ reference_ms ; under ⇔ scaled × 1.5 < deadline
        let scaled_x1000 = u128::from(t.check_ms) * u128::from(now_ref) * 1000 / u128::from(t.reference_ms);
        let limit_x1000 = dl.as_millis() * 1_000_000 / u128::from(REFERENCE_SLOWDOWN_PERMILLE);
        if scaled_x1000 >= limit_x1000 {
            return Attribution::CheckTimeout(format!(
                "{t_rel}: 지금의 기계 속도로 환산한 검사 시간이 시한의 2/3 를 넘습니다 — 느린 기계로 봅니다"
            ));
        }
    }
    Attribution::Failure(format!("{rel}: 대조 검사는 제때 통과했는데 고친 파일의 검사가 두 번 모두 시간 초과"))
}

/// A scratch copy of the repair's PRE-IMAGE for the control run (F8 (ii)): every target at its
/// pre-image (absent ones absent), each verified against its recorded hash.
fn scratch_pre_image(env: &JobEnv<'_>, rec: &RepairRecord, row: &LaneRecord) -> Result<PathBuf, RepairError> {
    let dir = env.op.root.join(".scratch").join(super::wal::new_op_id(env.op.now, "control"));
    std::fs::create_dir_all(&dir).map_err(|e| RepairError::Io(format!("{}: {e}", dir.display())))?;
    for (rel, _) in &row.applied {
        let (st, holder) = pre_image_for(env.op.root, rec, env.op.lane, rel)?;
        let dst = dir.join(rel);
        if let Some(p) = dst.parent() {
            std::fs::create_dir_all(p).map_err(|e| RepairError::Io(format!("{}: {e}", p.display())))?;
        }
        match (st, holder) {
            (TargetState::Absent, _) => {}
            (TargetState::Link { target }, _) => {
                #[cfg(unix)]
                std::os::unix::fs::symlink(&target, &dst).map_err(|e| RepairError::Io(e.to_string()))?;
                #[cfg(not(unix))]
                let _ = target;
            }
            (TargetState::File { mode, .. }, Some(h)) => {
                std::fs::copy(&h, &dst).map_err(|e| RepairError::Io(format!("{}: {e}", h.display())))?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = std::fs::set_permissions(&dst, std::fs::Permissions::from_mode(mode));
                }
                #[cfg(not(unix))]
                let _ = mode;
            }
            (TargetState::File { .. }, None) => {
                return Err(RepairError::Superseded(format!("{rel}: 적용 전 내용의 보관본이 없습니다")))
            }
        }
    }
    Ok(dir)
}

fn close_row(env: &JobEnv<'_>, id: &RepairId, closed: &str, timeouts: &[(String, f64)], record_state: Option<RepairState>) -> Result<(), RepairError> {
    let root = env.op.root.to_path_buf();
    let lane = env.op.lane.clone();
    let now = env.op.now;
    let idc = id.clone();
    let closed = closed.to_string();
    (env.locked)(id, &mut || {
        if let Some(mut row) = ledger::read_lane_in(&root, &idc, &lane)? {
            if let Some(p) = row.probation.as_mut() {
                if p.closed.is_none() {
                    p.closed = Some(closed.chars().take(400).collect());
                }
                for t in timeouts {
                    if !p.check_timeouts.contains(t) {
                        p.check_timeouts.push(t.clone());
                    }
                }
            }
            row.updated_at = now;
            ledger::write_lane_in(&root, &row)?;
        }
        if let Some(st) = record_state {
            if let Some(mut r) = ledger::read_record_in(&root, &idc)? {
                if r.origin_lane == lane && r.state == RepairState::Probation || (st == RepairState::Superseded && r.origin_lane == lane && in_force(&r)) {
                    r.state = st;
                    ledger::write_record_durable_in(&root, &r)?;
                }
            }
        }
        Ok(())
    })
}

fn note_timeouts(env: &JobEnv<'_>, id: &RepairId, timeouts: &[(String, f64)]) -> Result<(), RepairError> {
    let root = env.op.root.to_path_buf();
    let lane = env.op.lane.clone();
    let idc = id.clone();
    (env.locked)(id, &mut || {
        if let Some(mut row) = ledger::read_lane_in(&root, &idc, &lane)? {
            if let Some(p) = row.probation.as_mut() {
                for t in timeouts {
                    if !p.check_timeouts.contains(t) {
                        p.check_timeouts.push(t.clone());
                    }
                }
            }
            ledger::write_lane_in(&root, &row)?;
        }
        Ok(())
    })
}

/// Every repair in probation in THIS lane re-checked — only repaired files (no full sweep), with
/// the file's own check (never seat readiness — "보류" is not failure).
fn probation_pass(env: &JobEnv<'_>, report: &mut TickReport) -> Result<(), RepairError> {
    let root = env.op.root;
    let lane = env.op.lane;
    for id in ledger::list_ids_in(root) {
        let Ok(Some(row)) = ledger::read_lane_in(root, &id, lane) else {
            continue;
        };
        let Some(p) = row.probation.clone() else {
            continue;
        };
        if p.closed.is_some() {
            continue;
        }
        let Ok(Some(rec)) = ledger::read_record_in(root, &id) else {
            continue;
        };
        if rec.origin_lane == *lane && !in_force(&rec) {
            continue;
        }
        recheck_one(env, &rec, &row, report)?;
    }
    Ok(())
}

/// The probation re-check of ONE repair in this lane.
pub fn recheck_one(env: &JobEnv<'_>, rec: &RepairRecord, row: &LaneRecord, report: &mut TickReport) -> Result<(), RepairError> {
    let Some(p) = row.probation.as_ref() else {
        return Ok(());
    };
    let id = &rec.id;
    // 24 hours over → 사용 중 (N3: also after only check_timeouts; recorded, one line)
    if env.op.now - p.applied_at >= ledger::PROBATION_SECS as f64 {
        close_row(env, id, "사용 중 — 24시간 시험 기간을 마쳤습니다", &[], Some(RepairState::InUse))?;
        report.ended.push(id.clone());
        let tail = if p.check_timeouts.is_empty() {
            String::new()
        } else {
            format!(" (검사 시간 초과 {}건 기록 — 실패로 확정된 적은 없음)", p.check_timeouts.len())
        };
        report.lines.push(format!("{}: 시험 기간을 마치고 사용 중{tail}", id.as_str()));
        return Ok(());
    }
    // the binding: this lane, the pack generation, the expected live states
    let gen = apply::pack_generation(env.op.pack);
    let bound = PackGeneration {
        version: p.pack_generation.clone(),
        pro_revision: p.pro_revision,
    };
    if gen != bound {
        let why = format!("팩 세대가 바뀌었습니다({} → {}) — 아무것도 되돌리지 않습니다(새 버전 우선)", bound.version, gen.version);
        close_row(env, id, &format!("대체됨 — {why}"), &[], Some(RepairState::Superseded))?;
        report.superseded.push(id.clone());
        report.lines.push(format!("{}: {why}", id.as_str()));
        return Ok(());
    }
    for (rel, canon) in &p.expected {
        let live = TargetState::read(&env.op.pack.join(rel))?;
        if live.canon() != *canon {
            let why = format!("{rel}: 살아 있는 파일이 적용 결과와 다릅니다 — 아무것도 되돌리지 않고 워커 검토로 남깁니다");
            close_row(env, id, &format!("대체됨 — {why}"), &[], Some(RepairState::Superseded))?;
            report.superseded.push(id.clone());
            report.lines.push(format!("{}: {why}", id.as_str()));
            return Ok(());
        }
    }
    // the repaired files' own checks — one bounded child at a time
    let mut failure: Option<String> = None;
    let mut timeouts: Vec<(String, f64)> = Vec::new();
    for (rel, canon) in &p.expected {
        if canon == "absent" {
            continue;
        }
        let deadline = check_deadline(rel);
        match (env.check)(rel, env.op.pack, deadline) {
            Ok(CheckOutcome { result: CheckResult::Pass, .. }) => {}
            Ok(CheckOutcome { result: CheckResult::Fail(m), .. }) => {
                failure = Some(format!("{rel}: {m}"));
                break;
            }
            Ok(CheckOutcome { result: CheckResult::Timeout, .. }) => match attribute_timeout(env, rec, row, rel) {
                Attribution::Failure(m) => {
                    failure = Some(m);
                    break;
                }
                Attribution::CheckTimeout(m) => {
                    timeouts.push((rel.clone(), env.op.now));
                    report.lines.push(format!("{}: check_timeout — {m}", id.as_str()));
                }
            },
            Err(e) => {
                // the runner is not available: nothing is decided this tick
                report.lines.push(format!("{}: 검사를 돌릴 수 없습니다 — 다음 차례에 다시({e:?})", id.as_str()));
                return Ok(());
            }
        }
    }
    if let Some(why) = failure {
        if env.op.form == ApplyForm::Exchange {
            let r = crate::pack_apply_lock::with_apply_lock(env.apply_lock, || {
                revert_in(&env.op, id, RevertCaller::DaemonAuto, env.hooks, env.locked)
            })
            .map_err(RepairError::Io)??;
            (env.intake)(
                ledger::file_targets(rec).map(|f| f.rel.as_str()).next().unwrap_or(""),
                &format!("시험 기간 재검사 실패로 수리 {}를 되돌림 — {why}", id.as_str()),
            );
            if r.outcome == ApplyOutcome::Applied {
                report.reverted.push(id.clone());
                report.lines.push(format!("{}: 시험 기간 재검사 실패 — 이 수리 한 건만 되돌렸습니다(고친 사본은 보관소에) · {why}", id.as_str()));
            } else {
                report.superseded.push(id.clone());
                report.lines.push(format!("{}: 재검사 실패, 되돌리기는 대체됨 — {:?}", id.as_str(), r.outcome));
            }
        } else {
            // Windows 0.15.0: automatic return is off — recorded (one intake row) and reported only
            note_timeouts(env, id, &timeouts)?;
            (env.intake)(
                ledger::file_targets(rec).map(|f| f.rel.as_str()).next().unwrap_or(""),
                &format!("시험 기간 재검사 실패(이 운영체제에서는 자동으로 되돌리지 않음) — 수리 {} · {why}", id.as_str()),
            );
            report.reported_only.push((id.clone(), why.clone()));
            report.lines.push(format!("{}: 재검사 실패 — 이 운영체제에서는 자동으로 되돌리지 않고 기록·알림만 합니다 · {why}", id.as_str()));
        }
        return Ok(());
    }
    if !timeouts.is_empty() {
        note_timeouts(env, id, &timeouts)?;
        report.timeouts.extend(timeouts.iter().map(|(r, _)| (id.clone(), r.clone())));
    }
    Ok(())
}

// ───────────────────────────── M10 — the hourly scan of unapproved changes ─────────────────────────────

/// One M10 finding, kept in `<root>/m10/<lane>.findings.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    pub rel: String,
    /// sha256 of the live bytes found
    pub sha256: String,
    /// `reverted` | `kept` (passes its check — one owner notice) | `line-endings-only` (ledger only)
    /// | `report-only` (Windows) | `check-timeout` | `no-original` (the current generation's original
    /// is not known — nothing put back) | `superseded` (changed between the check and the put-back)
    pub kind: String,
    pub at: f64,
    /// Whether the owner must be told once (never for line-endings-only).
    pub notice: bool,
    /// Whether that one notice was given (the preflight's `repair.sweep` reply carries it once).
    pub notified: bool,
    pub slot: Option<PathBuf>,
    /// RULINGS CP1 (a): the path of the copy this finding is about — for `reverted` the SEALED copy of
    /// the changed bytes in the M10 sub-ledger's slot, otherwise the live file — with `sha256` above.
    #[serde(default)]
    pub kept_copy: Option<PathBuf>,
    pub note: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct FindingsFile {
    schema: u32,
    lane: String,
    findings: Vec<Finding>,
}

/// `<root>/m10/<lane>.json` (RULINGS CP1 — beside the lane's sub-ledger folder `<root>/m10/<lane>/`).
pub fn findings_path_in(root: &Path, lane: &LaneKey) -> PathBuf {
    root.join("m10").join(format!("{}.json", lane.0))
}

/// The ONE way back for a changed boot-critical file (RULINGS CP1 (a)): an ordinary repair — never a
/// hand copy.
pub fn m10_way_back(rel: &str) -> String {
    format!("cys repair begin --files {rel} --trigger <계기> → 예행·검토 서명 → 주인 최종 승인")
}

/// RULINGS CP1 (a): fills the finding's kept copy (path + the sha256 it carries) and its sentence — the
/// path of that copy and the legitimate way to bring the change back (an ordinary repair). The
/// detail found by the scan stays at the end of the sentence.
fn finalize_finding(f: &mut Finding, pack: &Path) {
    let kept = match (&f.kind[..], &f.slot) {
        ("reverted", Some(dir)) => {
            let slot = super::vault::Slot::at(dir, &f.rel);
            if std::fs::symlink_metadata(slot.sealed()).is_ok() {
                slot.sealed()
            } else {
                slot.payload()
            }
        }
        _ => pack.join(&f.rel),
    };
    let short = f.sha256.get(..12).unwrap_or(&f.sha256).to_string();
    let detail = f.note.clone();
    f.note = match f.kind.as_str() {
        "reverted" => format!(
            "자체 검사에 실패해 지금 세대의 원본(또는 승인본)으로 되돌렸습니다 — 바뀐 사본은 {} (sha256 {short}…)에 보관되어 있습니다. 그 변경을 다시 살리는 길은 수리입니다: {} ({detail})",
            kept.display(),
            m10_way_back(&f.rel)
        ),
        "kept" => format!(
            "승인 없이 바뀌었지만 자체 검사를 통과해 그대로 둡니다 — 지금 파일 {} (sha256 {short}…). 이 변경을 승인된 것으로 남기는 길은 수리입니다: {}",
            kept.display(),
            m10_way_back(&f.rel)
        ),
        _ => format!(
            "{detail} — 지금 파일 {} (sha256 {short}…). 고치는 길은 수리입니다: {}",
            kept.display(),
            m10_way_back(&f.rel)
        ),
    };
    f.kept_copy = Some(kept);
}

/// The lane's own M10 ledger (a sub-ledger under `repairs/m10/<lane>/`): M10 puts a file back with
/// no repair number of its own, so its vault slots and operation records live under this sub-root
/// (`displaced/R-1/…`, `R-1/ops/…` inside it) — outside every pack, write-once, never deleted.
pub fn m10_root_in(root: &Path, lane: &LaneKey) -> PathBuf {
    root.join("m10").join(&lane.0)
}

/// The sub-ledger's one bucket id.
pub fn m10_bucket() -> RepairId {
    RepairId::parse("R-1").expect("R-1 is a canonical repair id")
}

fn load_findings(root: &Path, lane: &LaneKey) -> Vec<Finding> {
    std::fs::read(findings_path_in(root, lane))
        .ok()
        .and_then(|b| serde_json::from_slice::<FindingsFile>(&b).ok())
        .map(|f| f.findings)
        .unwrap_or_default()
}

fn save_findings(root: &Path, lane: &LaneKey, findings: &[Finding]) -> Result<(), RepairError> {
    let f = FindingsFile {
        schema: super::REPAIR_SCHEMA,
        lane: lane.0.clone(),
        findings: findings.iter().rev().take(500).rev().cloned().collect(),
    };
    super::durable::write_json_durable(&findings_path_in(root, lane), &f)
}

/// The findings not yet told to the owner; marks them told (the preflight's sweep reply carries
/// each exactly once — "같은 파일의 같은 바이트는 다시 알리지 않음").
pub fn take_notices_in(root: &Path, lane: &LaneKey) -> Result<Vec<Finding>, RepairError> {
    let mut all = load_findings(root, lane);
    let mut out = Vec::new();
    for f in all.iter_mut() {
        if f.notice && !f.notified {
            f.notified = true;
            out.push(f.clone());
        }
    }
    if !out.is_empty() {
        save_findings(root, lane, &all)?;
    }
    Ok(out)
}

fn strip_cr(b: &[u8]) -> Vec<u8> {
    b.iter().copied().filter(|c| *c != b'\r').collect()
}

fn sha_hex(b: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(b).iter().map(|x| format!("{x:02x}")).collect()
}

/// M10 (§4-10, F9, U1): this lane's boot-critical files, rule documents excluded. A file equal to
/// our original of the current generation, or an approved copy of the current generation (the F9
/// triple — never a ledger row alone), is fine. A line-endings-only difference → ledger only. Else
/// its own check: passes → kept, the owner told once per (file, bytes); fails → (macOS) put back by
/// rule one — the expected bytes at step 0 are the bytes the check saw — to the original or approved
/// copy of the CURRENT generation (never an older generation's bytes; the changed copy kept in the
/// vault); Windows → recorded and reported only. A check timeout is attributed first (O3: the
/// control is the content that would be put back, as a scratch copy).
pub fn m10_sweep(env: &JobEnv<'_>) -> Result<Vec<Finding>, RepairError> {
    let root = env.op.root;
    let lane = env.op.lane;
    let pack = env.op.pack;
    let mut existing = load_findings(root, lane);
    let mut new_findings = Vec::new();
    // The install manifest (rel → sha256 at install) is read ONLY to tell "unchanged since the install"
    // when this binary does not know the current generation's original (a pro pack, a pack-only update
    // newer than the binary): then a file equal to its installed bytes is not an unapproved change. It
    // is never a put-back source and never stands in for a known original (BO3: the manifest is not
    // evidence of a released version).
    let (manifest, _) = crate::pack::load_manifest_state(pack);
    for e in super::boot_critical::entries() {
        let rel = e.rel;
        if (env.excluded)(rel) {
            continue;
        }
        // CT5: the binding is taken at sampling — the pack generation FIRST, then the live state and
        // the source of the right bytes; it is revalidated under the pack-apply lock right before a
        // put-back (any change → no put-back in this tick)
        let bound_gen = apply::pack_generation(pack);
        let live = pack.join(rel);
        let Ok(m) = std::fs::symlink_metadata(&live) else {
            continue; // absent: not a changed file (the heal paths own a missing file)
        };
        if !m.file_type().is_file() {
            continue;
        }
        let Ok(bytes) = std::fs::read(&live) else {
            continue;
        };
        let sha = sha_hex(&bytes);
        let original = (env.original)(rel);
        if original.as_deref() == Some(bytes.as_slice()) {
            continue;
        }
        if original.is_none() && manifest.get(rel).map(String::as_str) == Some(sha.as_str()) {
            continue;
        }
        if apply::approved_copy_of_current_generation_in(root, pack, lane, rel).unwrap_or(false) {
            continue;
        }
        let already = |kind: &str| existing.iter().any(|f| f.rel == rel && f.sha256 == sha && f.kind == kind);
        let lf_only = original.as_ref().is_some_and(|o| strip_cr(o) == strip_cr(&bytes));
        let deadline = check_deadline(rel);
        let verdict = match (env.check)(rel, pack, deadline) {
            Ok(o) => o.result,
            Err(_) => continue, // no runner: nothing decided
        };
        let failed: Option<String> = match verdict {
            CheckResult::Pass => {
                let kind = if lf_only { "line-endings-only" } else { "kept" };
                if !already(kind) {
                    new_findings.push(Finding {
                        rel: rel.to_string(),
                        sha256: sha.clone(),
                        kind: kind.to_string(),
                        at: env.op.now,
                        notice: !lf_only,
                        notified: false,
                        slot: None,
                        kept_copy: None,
                        note: if lf_only {
                            "줄바꿈만 다름 — 장부에만 기록".into()
                        } else {
                            "승인 없이 바뀌었지만 자체 검사를 통과 — 그대로 둠".into()
                        },
                    });
                }
                None
            }
            CheckResult::Fail(m) => Some(m),
            CheckResult::Timeout => match m10_attribute_timeout(env, rel, original.as_deref(), deadline) {
                Ok(m) => Some(m),
                Err(m) => {
                    if !already("check-timeout") {
                        new_findings.push(Finding {
                            rel: rel.to_string(),
                            sha256: sha.clone(),
                            kind: "check-timeout".into(),
                            at: env.op.now,
                            notice: false,
                            notified: false,
                            slot: None,
                            kept_copy: None,
                            note: m,
                        });
                    }
                    None
                }
            },
        };
        let Some(why) = failed else {
            continue;
        };
        let notice = !lf_only;
        if env.op.form != ApplyForm::Exchange {
            if !already("report-only") {
                new_findings.push(Finding {
                    rel: rel.to_string(),
                    sha256: sha.clone(),
                    kind: "report-only".into(),
                    at: env.op.now,
                    notice,
                    notified: false,
                    slot: None,
                    kept_copy: None,
                    note: format!("자체 검사 실패 — 이 운영체제에서는 기록·알림만 합니다: {why}"),
                });
            }
            continue;
        }
        let live_mode = TargetState::read(&live)
            .ok()
            .and_then(|s| match s {
                TargetState::File { mode, .. } => Some(mode),
                _ => None,
            })
            .unwrap_or(0o644);
        // the put-back content: the approved copy of the current generation, else our original of it
        let target: Option<(Vec<u8>, u32)> = match apply::approved_copy_source_in(root, pack, lane, rel) {
            Some((TargetState::File { mode, .. }, holder)) => std::fs::read(&holder).ok().map(|b| (b, mode)),
            Some(_) => None,
            None => original.clone().map(|o| (o, live_mode)),
        };
        let Some((orig, mode)) = target else {
            if !already("no-original") {
                new_findings.push(Finding {
                    rel: rel.to_string(),
                    sha256: sha.clone(),
                    kind: "no-original".into(),
                    at: env.op.now,
                    notice,
                    notified: false,
                    slot: None,
                    kept_copy: None,
                    note: format!("자체 검사 실패, 그러나 지금 세대의 원본·승인본을 알지 못해 되돌리지 않았습니다: {why}"),
                });
            }
            continue;
        };
        // put back by rule one; the expected bytes are exactly what the check saw
        let sampled_live = TargetState::File { sha256: sha.clone(), mode: live_mode };
        let source_sha = sha_hex(&orig);
        let sub = m10_root_in(root, lane);
        std::fs::create_dir_all(&sub).map_err(|e| RepairError::Io(format!("{}: {e}", sub.display())))?;
        let src_file = sub.join(format!(".original-{}", super::wal::new_op_id(env.op.now, "orig")));
        apply::write_new(&src_file, &orig, mode)?;
        let sub_env = OpEnv {
            root: &sub,
            pack,
            lane,
            now: env.op.now,
            form: env.op.form,
            faults: env.op.faults,
            own_check: env.op.own_check,
        };
        let spec = OpSpec {
            id: m10_bucket(),
            rel: rel.to_string(),
            kind: "m10-revert".into(),
            expected: sampled_live.clone(),
            incoming: TargetState::File { sha256: source_sha.clone(), mode },
            source: Some(src_file.clone()),
            generation: Some(bound_gen.clone()),
        };
        let r = crate::pack_apply_lock::with_apply_lock(env.apply_lock, || -> Result<Option<apply::OpDone>, RepairError> {
            // CT5: THE binding, revalidated under the lock — generation, live state, source of the bytes
            if apply::pack_generation(pack) != bound_gen {
                return Ok(None);
            }
            if TargetState::read(&live)? != sampled_live {
                return Ok(None);
            }
            let source_now: Option<String> = match apply::approved_copy_source_in(root, pack, lane, rel) {
                Some((TargetState::File { .. }, holder)) => std::fs::read(&holder).ok().map(|b| sha_hex(&b)),
                Some(_) => None,
                None => (env.original)(rel).map(|o| sha_hex(&o)),
            };
            if source_now.as_deref() != Some(source_sha.as_str()) {
                return Ok(None);
            }
            apply::run_op(&sub_env, &spec).map(Some)
        })
        .map_err(RepairError::Io)?;
        let _ = std::fs::remove_file(&src_file); // our own source copy (the slot holds what went live)
        let Some(r) = r? else {
            new_findings.push(Finding {
                rel: rel.to_string(),
                sha256: sha,
                kind: "superseded".into(),
                at: env.op.now,
                notice: false,
                notified: false,
                slot: None,
                kept_copy: None,
                note: "검사와 되돌리기 사이에 팩 세대·살아 있는 파일·원본 중 하나가 바뀜 — 이번 차례에는 되돌리지 않음".into(),
            });
            continue;
        };
        let (kind, note) = match &r.outcome {
            ApplyOutcome::Applied => ("reverted", format!("검사 결과: {why}")),
            ApplyOutcome::Superseded(m) => ("superseded", format!("검사와 되돌리기 사이에 바뀜 — 되돌리지 않고 다음 검사로: {m}")),
            ApplyOutcome::Declined(m) => ("declined", format!("되돌리지 못함(아무것도 바꾸지 않음): {m}")),
        };
        new_findings.push(Finding {
            rel: rel.to_string(),
            sha256: sha,
            kind: kind.to_string(),
            at: env.op.now,
            notice,
            notified: false,
            slot: r.slot,
            kept_copy: None,
            note,
        });
    }
    for f in new_findings.iter_mut() {
        finalize_finding(f, pack);
    }
    if !new_findings.is_empty() {
        existing.extend(new_findings.iter().cloned());
        save_findings(root, lane, &existing)?;
    }
    Ok(new_findings)
}

/// O3 for the scan (flagged interpretation — see the T6 report): M10 has no commit-time duration,
/// so the control's own duration measured now stands for it (ratio 1): FAILURE when the control —
/// the same check, same deadline, on a scratch copy of the content that would be put back — passed
/// within the deadline ÷ 1.5 AND the file's check timed out again; else `check_timeout`.
fn m10_attribute_timeout(env: &JobEnv<'_>, rel: &str, original: Option<&[u8]>, deadline: Duration) -> Result<String, String> {
    let Some(orig) = original else {
        return Err(format!("{rel}: 검사 시간 초과 — 대조할 지금 세대의 원본이 없어 실패로 단정하지 않습니다"));
    };
    let dir = env.op.root.join(".scratch").join(super::wal::new_op_id(env.op.now, "m10-control"));
    let dst = dir.join(rel);
    let made = dst
        .parent()
        .map(|p| std::fs::create_dir_all(p).is_ok())
        .unwrap_or(false)
        && std::fs::write(&dst, orig).is_ok();
    if !made {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(format!("{rel}: 대조 사본을 만들 수 없습니다"));
    }
    let control = (env.check)(rel, &dir, deadline);
    let _ = std::fs::remove_dir_all(&dir);
    let control_ms = match control {
        Ok(CheckOutcome { result: CheckResult::Pass, ms }) => ms,
        _ => return Err(format!("{rel}: 대조 검사(원본)도 제때 끝나지 않았습니다 — 느린 기계로 봅니다")),
    };
    if u128::from(control_ms) * u128::from(REFERENCE_SLOWDOWN_PERMILLE) >= deadline.as_millis() * 1000 {
        return Err(format!("{rel}: 원본의 검사도 시한의 2/3 를 넘게 걸렸습니다 — 느린 기계로 봅니다"));
    }
    match (env.check)(rel, env.op.pack, deadline) {
        Ok(CheckOutcome { result: CheckResult::Timeout, .. }) => Ok(format!("{rel}: 원본은 제때 통과했는데 이 파일의 검사가 두 번 모두 시간 초과")),
        Ok(CheckOutcome { result: CheckResult::Fail(m), .. }) => Ok(format!("{rel}: {m}")),
        Ok(CheckOutcome { result: CheckResult::Pass, .. }) => Err(format!("{rel}: 두 번째 검사는 통과 — 일시적인 지연으로 봅니다")),
        Err(e) => Err(format!("{rel}: 두 번째 검사를 돌릴 수 없습니다({e:?})")),
    }
}

/// The resolved restore content of M10 for `rel` in a pack: the binary's embedded copy, only when
/// the pack's generation is this binary's own (version = this build's version, free channel) —
/// otherwise unknown (never an older or foreign generation's bytes).
pub fn embedded_original_for(pack: &Path, rel: &str) -> Option<Vec<u8>> {
    let gen = apply::pack_generation(pack);
    if gen.version != env!("CARGO_PKG_VERSION") || gen.pro_revision.is_some() {
        return None;
    }
    crate::pack::PACK
        .iter()
        .find(|(r, _)| *r == rel)
        .map(|(_, c)| c.as_bytes().to_vec())
}

#[cfg(test)]
#[path = "probation_tests.rs"]
mod tests;
