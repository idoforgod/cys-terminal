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
    /// One intake row (data, never a message): (rel, error line) → `true` only when the intake
    /// ACKNOWLEDGED it (recorded, or definitively refused by rule); `false` = over quota or an error
    /// — the report stays pending and is retried at a later tick (RULINGS DR5).
    pub intake: &'a dyn Fn(&str, &str) -> bool,
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
    // CT10: a BACKWARD clock step (now earlier than the last run) makes the work due at once — it
    // never waits for the wall clock to catch up; a forward step only makes work due sooner (and the
    // re-check checks supersession before anything else)
    let elapsed_at_least = |last: f64, interval: u64| now < last || now - last >= interval as f64;
    let first = state.is_none_or(|s| s.last_incarnation != incarnation);
    let probation = first || state.is_none_or(|s| elapsed_at_least(s.last_probation_check, ledger::PROBATION_RECHECK_SECS));
    let sweep = state.is_none_or(|s| elapsed_at_least(s.last_sweep, ledger::SWEEP_SECS));
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
    /// DR5: the fault keys whose intake was attempted in THIS tick (once per tick per repair).
    pub intake_tried: Vec<String>,
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
        let mut faults: Vec<(RepairId, String)> = Vec::new();
        let mut n = 0u32;
        let injected = |e: &RepairError| format!("{e:?}").contains(apply::INJECTED_STOP);
        match apply::resume_interrupted_report_in(&env.op) {
            Ok((k, errors)) => {
                n += k;
                faults.extend(errors);
            }
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
        Ok((n, lines, faults))
    });
    match recovered {
        Ok(Some(r)) => {
            let (n, lines, faults) = r?;
            report.resumed = n;
            report.lines.extend(lines);
            // DR5: an error of a recovery operation is recorded (a line + one acknowledged report)
            for (id, why) in faults {
                fault_once(env, &id, "recovery-error", &format!("repairs/{}/ops", id.as_str()), &why, &mut report);
            }
        }
        Ok(None) => {
            report.lines.push("다른 적용이나 업데이트가 팩을 바꾸는 중 — 이번 차례는 아무것도 하지 않습니다".into());
            return Ok(report);
        }
        Err(e) => return Err(RepairError::Io(e)),
    }
    // CU1: ONE bad record never stops the job's work — every pass is fault-isolated (a failure is a
    // line and, per repair, ONE intake row); the job state is written for the parts that ran
    if d.first_of_incarnation {
        match start_time_pass(env) {
            Ok(lines) => report.lines.extend(lines),
            Err(e) => report.lines.push(format!("시작 때 정리를 다 하지 못했습니다 — 다음 시작 때 다시({e:?})")),
        }
        let swept = super::vault::sweep_stale_temps(root);
        if !swept.is_empty() {
            report.lines.push(format!("끝난 프로세스가 남긴 임시 항목 {}개를 치웠습니다", swept.len()));
        }
    }
    if d.probation {
        probation_pass(env, &mut report);
    }
    if d.sweep {
        match m10_sweep(env) {
            Ok(f) => report.m10 = f,
            Err(e) => report.lines.push(format!("미승인 변경 검사를 마치지 못했습니다 — 다음 차례에 다시({e:?})")),
        }
    }
    write_job_state(root, lane, env.incarnation, now, &d)?;
    Ok(report)
}

/// `<root>/lanes/<lane>.job-faults.json` — this lane's job's fault reports, keyed `<R-id>:<kind>`:
/// `pending` until the intake ACKNOWLEDGED the row, then `filed` (RULINGS CU1 / DR5).
fn faults_path_in(root: &Path, lane: &LaneKey) -> PathBuf {
    root.join("lanes").join(format!("{}.job-faults.json", lane.0))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct FaultEntry {
    rel: String,
    line: String,
    state: String,
}

fn load_faults(root: &Path, lane: &LaneKey) -> std::collections::BTreeMap<String, FaultEntry> {
    std::fs::read(faults_path_in(root, lane))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// Tries the intake of one pending fault (once per tick per key); `filed` is written only AFTER the
/// intake acknowledged it.
fn try_file(env: &JobEnv<'_>, key: &str, report: &mut TickReport) {
    if report.intake_tried.iter().any(|k| k == key) {
        return;
    }
    report.intake_tried.push(key.to_string());
    let mut all = load_faults(env.op.root, env.op.lane);
    let Some(entry) = all.get(key).cloned() else {
        return;
    };
    if entry.state == "filed" {
        return;
    }
    if (env.intake)(&entry.rel, &entry.line) {
        all.insert(key.to_string(), FaultEntry { state: "filed".into(), ..entry });
        if let Err(e) = super::durable::write_json_durable(&faults_path_in(env.op.root, env.op.lane), &all) {
            report.lines.push(format!("{key}: 접수된 보고를 '보고함'으로 적지 못했습니다 — 다음 차례에 다시 확인({e:?})"));
        }
    } else {
        report.lines.push(format!("{key}: 보고가 아직 접수되지 않았습니다(한도 초과나 오류) — 다음 차례에 다시"));
    }
}

/// CU1 / DR5: a repair the job could not handle — logged in the tick's lines every time, recorded
/// `pending` (durably) the first time, and filed as ONE intake row: marked `filed` only after the
/// intake acknowledged it; a pending one is retried at later ticks.
fn fault_once(env: &JobEnv<'_>, id: &RepairId, kind: &str, rel: &str, line: &str, report: &mut TickReport) {
    report.lines.push(format!("{}: {line} — 이 수리는 건너뛰고 나머지는 계속합니다", id.as_str()));
    let key = format!("{}:{kind}", id.as_str());
    let mut all = load_faults(env.op.root, env.op.lane);
    if !all.contains_key(&key) {
        all.insert(
            key.clone(),
            FaultEntry {
                rel: rel.to_string(),
                line: format!("자체 해결 작업이 수리 {}를 처리하지 못함 — {}", id.as_str(), line.chars().take(300).collect::<String>()),
                state: "pending".into(),
            },
        );
        if let Err(e) = super::durable::write_json_durable(&faults_path_in(env.op.root, env.op.lane), &all) {
            report.lines.push(format!("{key}: 보고 대기를 기록하지 못했습니다({e:?})"));
        }
    }
    try_file(env, &key, report);
}

/// DR5: every pending report of this lane is retried (once per tick per key).
fn retry_pending_faults(env: &JobEnv<'_>, report: &mut TickReport) {
    let pending: Vec<String> = load_faults(env.op.root, env.op.lane)
        .into_iter()
        .filter(|(_, e)| e.state == "pending")
        .map(|(k, _)| k)
        .collect();
    for key in pending {
        try_file(env, &key, report);
    }
}

/// A report filed through the same acknowledged path, with its own description (the probation's
/// failure reports — DR5 / DR8).
fn report_once(env: &JobEnv<'_>, id: &RepairId, kind: &str, rel: &str, description: &str, report: &mut TickReport) {
    let key = format!("{}:{kind}", id.as_str());
    let mut all = load_faults(env.op.root, env.op.lane);
    if !all.contains_key(&key) {
        all.insert(key.clone(), FaultEntry { rel: rel.to_string(), line: description.to_string(), state: "pending".into() });
        if let Err(e) = super::durable::write_json_durable(&faults_path_in(env.op.root, env.op.lane), &all) {
            report.lines.push(format!("{key}: 보고 대기를 기록하지 못했습니다({e:?})"));
        }
    }
    try_file(env, &key, report);
}

/// Every repair in probation in THIS lane re-checked — only repaired files (no full sweep), with
/// the file's own check (never seat readiness — "보류" is not failure).
fn probation_pass(env: &JobEnv<'_>, report: &mut TickReport) {
    let root = env.op.root;
    let lane = env.op.lane;
    retry_pending_faults(env, report);
    for id in ledger::list_ids_in(root) {
        let ledger_rel = format!("repairs/{}/record.json", id.as_str());
        let row = match ledger::read_lane_in(root, &id, lane) {
            Ok(Some(row)) => row,
            Ok(None) => continue,
            Err(e) => {
                fault_once(env, &id, "lane-row-unreadable", &ledger_rel, &format!("이 부서의 행을 읽을 수 없습니다({e:?})"), report);
                continue;
            }
        };
        let Some(p) = row.probation.clone() else {
            continue;
        };
        if p.closed.is_some() {
            continue;
        }
        let rec = match ledger::read_record_in(root, &id) {
            Ok(Some(rec)) => rec,
            Ok(None) => continue,
            Err(e) => {
                fault_once(env, &id, "record-unreadable", &ledger_rel, &format!("수리 기록을 읽을 수 없습니다(손상 또는 더 높은 판의 기록 — {e:?})"), report);
                continue;
            }
        };
        if rec.origin_lane == *lane && !in_force(&rec) {
            continue;
        }
        if let Err(e) = recheck_one(env, &rec, &row, report) {
            let rel = ledger::file_targets(&rec).map(|f| f.rel.clone()).next().unwrap_or(ledger_rel);
            fault_once(env, &id, "recheck-error", &rel, &format!("재검사를 마치지 못했습니다({e:?})"), report);
        }
    }
}

/// CU1 / CU4: a failing re-check that could NOT be reverted (or, on Windows, is never reverted
/// automatically) ends in a truthful FAILED state the ledger shows — the lane row 충돌 with its
/// probation closed `실패 — …` (never closed 사용 중 later); the record keeps its state with a line.
fn close_failed(env: &JobEnv<'_>, id: &RepairId, why: &str) -> Result<(), RepairError> {
    let root = env.op.root.to_path_buf();
    let lane = env.op.lane.clone();
    let now = env.op.now;
    let idc = id.clone();
    let why = why.to_string();
    (env.locked)(id, &mut || {
        if let Some(mut row) = ledger::read_lane_in(&root, &idc, &lane)? {
            row.state = super::types::LaneState::Conflict;
            if let Some(p) = row.probation.as_mut() {
                if p.closed.is_none() {
                    p.closed = Some(format!("실패 — {why}").chars().take(400).collect());
                }
            }
            row.updated_at = now;
            ledger::write_lane_in(&root, &row)?;
        }
        if let Some(mut r) = ledger::read_record_in(&root, &idc)? {
            if r.origin_lane == lane {
                r.methods.push(format!("실패 — {why}").chars().take(300).collect());
                ledger::write_record_durable_in(&root, &r)?;
            }
        }
        Ok(())
    })
}

/// The probation re-check of ONE repair in this lane.
pub fn recheck_one(env: &JobEnv<'_>, rec: &RepairRecord, row: &LaneRecord, report: &mut TickReport) -> Result<(), RepairError> {
    let Some(p) = row.probation.as_ref() else {
        return Ok(());
    };
    let id = &rec.id;
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
    // 24 hours over → 사용 중 (N3: also after only check_timeouts; recorded, one line). CT10: only
    // AFTER the supersession checks above — a sleeping machine or a forward clock step never closes
    // a superseded probation as in use
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
        let first_rel = ledger::file_targets(rec).map(|f| f.rel.as_str()).next().unwrap_or("");
        if env.op.form == ApplyForm::Exchange {
            let r = crate::pack_apply_lock::with_apply_lock(env.apply_lock, || {
                revert_in(&env.op, id, RevertCaller::DaemonAuto, env.hooks, env.locked)
            })
            .map_err(RepairError::Io)
            .and_then(|r| r);
            // DR8: the intake description is derived from the MEASURED outcome of the revert
            let outcome_text = match &r {
                Ok(x) if x.outcome == ApplyOutcome::Applied => "되돌림".to_string(),
                Ok(x) => format!("되돌리지 못함({:?})", x.outcome),
                Err(e) => format!("되돌리지 못함({e:?})"),
            };
            report_once(env, id, "probation-failed", first_rel, &format!("시험 기간 재검사 실패 — 수리 {} {outcome_text} — {why}", id.as_str()), report);
            match r {
                Ok(r) if r.outcome == ApplyOutcome::Applied => {
                    report.reverted.push(id.clone());
                    report.lines.push(format!("{}: 시험 기간 재검사 실패 — 이 수리 한 건만 되돌렸습니다(고친 사본은 보관소에) · {why}", id.as_str()));
                }
                Ok(r) if matches!(r.outcome, ApplyOutcome::Superseded(_)) && r.files.iter().all(|(_, o)| *o != ApplyOutcome::Applied) => {
                    // nothing was changed: the live files are not the applied state (대체됨 recorded)
                    report.superseded.push(id.clone());
                    report.lines.push(format!("{}: 재검사 실패, 되돌리기는 대체됨 — {:?}", id.as_str(), r.outcome));
                }
                other => {
                    // CU1: a failure that could not be reverted is never closed 사용 중 — failed state
                    let detail = match &other {
                        Ok(r) => format!("{:?}", r.outcome),
                        Err(e) => format!("{e:?}"),
                    };
                    close_failed(env, id, &format!("재검사 실패, 되돌리지 못했습니다({detail}) · {why} — 워커 검토"))?;
                    report.reported_only.push((id.clone(), why.clone()));
                    report.lines.push(format!("{}: 재검사 실패 — 되돌리지 못해 실패로 기록했습니다({detail}) · {why}", id.as_str()));
                }
            }
        } else {
            // Windows 0.15.0: automatic return is off — ONE intake row and a FAILED end of this
            // probation (CU4: never 사용 중 later; the row closed, so it is not re-filed every check)
            note_timeouts(env, id, &timeouts)?;
            report_once(env, id, "probation-failed", first_rel, &format!("시험 기간 재검사 실패(이 운영체제에서는 자동으로 되돌리지 않음) — 수리 {} · {why}", id.as_str()), report);
            close_failed(env, id, &format!("재검사 실패(이 운영체제에서는 자동으로 되돌리지 않음 — 기록·알림) · {why} — 워커 검토"))?;
            report.reported_only.push((id.clone(), why.clone()));
            report.lines.push(format!("{}: 재검사 실패 — 이 운영체제에서는 자동으로 되돌리지 않고 실패로 기록·알림만 합니다 · {why}", id.as_str()));
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
    // CT9: a put-back's finding survives a crash between the exchange and the findings write — it is
    // rebuilt idempotently from the completed operation records of the M10 sub-ledger
    let sub_root = m10_root_in(root, lane);
    let (done_ops, _) = super::wal::list_in(&sub_root, &m10_bucket());
    for op in done_ops.into_iter().filter(|o| {
        o.finished && o.outcome.as_deref() == Some("applied") && o.kind == "m10-revert" && o.lane_key == *lane
    }) {
        if existing.iter().chain(new_findings.iter()).any(|f: &Finding| f.slot.is_some() && f.slot == op.slot) {
            continue;
        }
        let sha = match apply::state_of_canon(&op.expected) {
            Some(TargetState::File { sha256, .. }) => sha256,
            _ => continue,
        };
        new_findings.push(Finding {
            rel: op.rel.clone(),
            sha256: sha,
            kind: "reverted".into(),
            at: op.updated_at,
            notice: true,
            notified: false,
            slot: op.slot.clone(),
            kept_copy: None,
            note: "검사 결과: 실패(기록이 끊겨 동작 기록에서 다시 세움)".into(),
        });
    }
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
            // CU4: a boot-critical file replaced by a symbolic link (or anything not a regular file)
            // is a FINDING — never skipped silently; nothing is followed or put back automatically
            let what = if m.file_type().is_symlink() {
                format!("심볼릭 링크 → {}", std::fs::read_link(&live).map(|t| t.display().to_string()).unwrap_or_default())
            } else {
                "일반 파일이 아님".to_string()
            };
            let key = sha_hex(what.as_bytes());
            if !existing.iter().any(|f| f.rel == rel && f.kind == "not-a-file" && f.sha256 == key) {
                new_findings.push(Finding {
                    rel: rel.to_string(),
                    sha256: key,
                    kind: "not-a-file".into(),
                    at: env.op.now,
                    notice: true,
                    notified: false,
                    slot: None,
                    kept_copy: None,
                    note: format!("기동 필수 파일이 일반 파일이 아닙니다({what}) — 따라가거나 되돌리지 않았습니다(워커 검토)"),
                });
            }
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
        let already = |kind: &str| existing.iter().any(|f| f.rel == rel && f.sha256 == sha && f.kind == kind);
        // CT9: a line-endings-only difference is a LEDGER-ONLY branch before any check or write
        if original.as_ref().is_some_and(|o| strip_cr(o) == strip_cr(&bytes)) {
            if !already("line-endings-only") {
                new_findings.push(Finding {
                    rel: rel.to_string(),
                    sha256: sha.clone(),
                    kind: "line-endings-only".into(),
                    at: env.op.now,
                    notice: false,
                    notified: false,
                    slot: None,
                    kept_copy: None,
                    note: "줄바꿈만 다름 — 장부에만 기록".into(),
                });
            }
            continue;
        }
        if apply::approved_copy_of_current_generation_in(root, pack, lane, rel).unwrap_or(false) {
            continue;
        }
        // CU2: the owner-approved copy of this generation stays the source of the right bytes (also
        // after its probation closed superseded); the approved bytes live → nothing to do; a copy that
        // cannot be verified → NOTHING is put back for this file, a finding for the worker
        let approved = apply::approved_copy_resolution_in(root, pack, lane, rel);
        if let apply::ApprovedCopy::Found(_, holder) = &approved {
            if std::fs::read(holder).ok().as_deref() == Some(bytes.as_slice()) {
                continue;
            }
        }
        if let apply::ApprovedCopy::Unverifiable(by) = &approved {
            if !already("approved-copy-unverifiable") {
                new_findings.push(Finding {
                    rel: rel.to_string(),
                    sha256: sha.clone(),
                    kind: "approved-copy-unverifiable".into(),
                    at: env.op.now,
                    notice: true,
                    notified: false,
                    slot: None,
                    kept_copy: None,
                    note: format!("주인이 승인한 {} 의 승인본을 확인할 수 없어 이 파일은 아무것도 되돌리지 않았습니다(워커 검토)", by.as_str()),
                });
            }
            continue;
        }
        let lf_only = false;
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
            // CT10: an unapproved file has no commit-time timing evidence — without evidence there is
            // no put-back: `check_timeout`, nothing moved, checked again next hour (the flagged
            // "ratio 1" reading was rejected)
            CheckResult::Timeout => {
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
                        note: format!("{rel}: 검사 시간 초과 — 시간 근거가 없어 실패로 단정하지 않고 아무것도 옮기지 않았습니다(다음 차례에 다시)"),
                    });
                }
                None
            }
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
        let target: Option<(Vec<u8>, u32)> = match &approved {
            apply::ApprovedCopy::Found(TargetState::File { mode, .. }, holder) => std::fs::read(holder).ok().map(|b| (b, *mode)),
            apply::ApprovedCopy::Found(..) | apply::ApprovedCopy::Unverifiable(_) => None,
            apply::ApprovedCopy::None => original.clone().map(|o| (o, live_mode)),
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
            let source_now: Option<String> = match apply::approved_copy_resolution_in(root, pack, lane, rel) {
                apply::ApprovedCopy::Found(TargetState::File { .. }, holder) => std::fs::read(&holder).ok().map(|b| sha_hex(&b)),
                apply::ApprovedCopy::Found(..) | apply::ApprovedCopy::Unverifiable(_) => None,
                apply::ApprovedCopy::None => (env.original)(rel).map(|o| sha_hex(&o)),
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
