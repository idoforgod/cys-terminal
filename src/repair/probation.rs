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
use super::ledger::{self, JobState, LaneRecord, PreImage, RepairRecord};
use super::platform::ApplyForm;
use super::rehearsal::{CheckResult, REFERENCE_SLOWDOWN_PERMILLE};
use super::types::{LaneKey, RepairError, RepairId, RepairState, TargetState};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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
    /// DX3: an automatic return / a put-back was skipped because another apply or update held the
    /// pack-apply lock — the probation check / the scan stays due (retried at the next tick).
    pub deferred_probation: bool,
    pub deferred_sweep: bool,
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
    #[allow(clippy::type_complexity)]
    let recovered = crate::pack_apply_lock::try_with_apply_lock(env.apply_lock, || -> Result<(u32, Vec<String>, Vec<(RepairId, String)>, Vec<String>), RepairError> {
        let mut lines = Vec::new();
        let mut faults: Vec<(RepairId, String)> = Vec::new();
        let mut m10_faults: Vec<String> = Vec::new();
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
        let (hk, hook_errors) = apply::resume_hook_steps_report_in(root, env.op.pack, lane, env.registrar, env.locked, now);
        n += hk;
        faults.extend(hook_errors);
        // FG1: the sub-ledger folder probed with three answers (absent = nothing to recover there)
        let m10_present = match std::fs::symlink_metadata(&m10_root) {
            Ok(_) => true,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(e) => {
                m10_faults.push(format!("{}: 확인할 수 없음({e})", m10_root.display()));
                false
            }
        };
        if m10_present {
            match apply::resume_interrupted_report_in(&m10_env) {
                Ok((k, errors)) => {
                    n += k;
                    m10_faults.extend(errors.into_iter().map(|(_, why)| why));
                }
                Err(e) if injected(&e) => return Err(e),
                Err(e) => lines.push(format!("M10 의 끊긴 동작을 마저 하지 못했습니다 — 다음 차례에 다시({e:?})")),
            }
        }
        lines.extend(apply::recover_apply_intents_in(&env.op, env.locked));
        // DR7: interrupted reverts are completed as a whole
        lines.extend(apply::recover_revert_intents_in(&env.op, env.hooks, env.locked));
        // FI2: an intent that cannot be read is a REPORTED fault (one row per repair until acknowledged)
        for (id, why) in apply::unreadable_intents_in(root, lane) {
            faults.push((id, why));
        }
        Ok((n, lines, faults, m10_faults))
    });
    match recovered {
        Ok(Some(r)) => {
            let (n, lines, faults, m10_faults) = r?;
            report.resumed = n;
            report.lines.extend(lines);
            // DR5: an error of a recovery operation is recorded (a line + one acknowledged report)
            for (id, why) in faults {
                fault_once(env, &id, "recovery-error", &format!("repairs/{}/ops", id.as_str()), &why, &mut report);
            }
            // FI4: the M10 sub-ledger's recovery errors have their OWN key (never a repair's R-1)
            if !m10_faults.is_empty() {
                let line = format!("M10: {}", m10_faults.join(" · "));
                report.lines.push(format!("{line} — 다음 차례에 다시"));
                let text = format!("자체 해결 작업이 이 부서의 미승인 변경 되돌리기의 끊긴 동작을 처리하지 못함 — {}", line.chars().take(300).collect::<String>());
                file_once(env, &format!("m10:{}:recovery-error", lane.0), &format!("m10/{}/R-1/ops", lane.0), &text, &mut report);
            }
            // FG3: a hook operation that finished declined is reported until acknowledged
            hook_terminal_obligations(env, &mut report);
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
            Err(e) => {
                let line = format!("시작 때 정리를 다 하지 못했습니다 — 다음 시작 때 다시({e:?})");
                report.lines.push(line.clone());
                file_once(env, &format!("start:{}:error", lane.0), "repairs", &format!("자체 해결 작업이 {line}"), &mut report);
            }
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
        let deferred = std::cell::Cell::new(false);
        let cause = std::cell::Cell::new(None);
        let swept = m10_sweep_full(env, &deferred, &cause);
        report.deferred_sweep = deferred.get();
        if let Some(c) = cause.take() {
            let line = format!("장부의 {c} 을(를) 읽을 수 없어 이 부서의 한 시간마다 되돌리기를 멈췄습니다");
            file_once(env, &format!("ledger:{c}"), &c, &line, &mut report);
        }
        match swept {
            Ok(f) => report.m10 = f,
            Err(e) => {
                let line = format!("미승인 변경 검사를 마치지 못했습니다 — 다음 차례에 다시({e:?})");
                report.lines.push(line.clone());
                // FG1: an unreadable view / acknowledgement file stops the scan — a REPORTED fault
                file_once(env, &format!("m10:{}:sweep-error", lane.0), &format!("m10/{}.json", lane.0), &format!("자체 해결 작업의 {line}"), &mut report);
            }
        }
    }
    // DX3: work skipped for the lock is not marked done — the next tick retries it
    let mut done = d;
    if report.deferred_probation {
        done.probation = false;
    }
    if report.deferred_sweep {
        done.sweep = false;
    }
    write_job_state(root, lane, env.incarnation, now, &done)?;
    Ok(report)
}

/// `<root>/lanes/<lane>.job-acks.json` — ONLY acknowledgements (EN1 / EJ3): the keys whose intake
/// row the intake accepted. The duties themselves are never stored apart: a closure that owes a row
/// is re-derived from the row (`apply::closure_owes_intake`), a recurring fault from the condition
/// re-detected each tick. A lost acknowledgement re-files the same signature (the intake merges it).
fn acks_path_in(root: &Path, lane: &LaneKey) -> PathBuf {
    root.join("lanes").join(format!("{}.job-acks.json", lane.0))
}

/// FG1: the acknowledgements with three answers (absent on positive evidence = none yet).
fn load_acks_checked(root: &Path, lane: &LaneKey) -> Result<std::collections::BTreeMap<String, f64>, RepairError> {
    Ok(read_json_checked::<std::collections::BTreeMap<String, f64>>(&acks_path_in(root, lane))?.unwrap_or_default())
}

/// Files the intake row of one duty `key` unless already acknowledged — at most once per tick; the
/// acknowledgement is written only AFTER the intake accepted it. FG1: an unreadable acknowledgement
/// file is said in a line; the row is filed (the intake merges a repeat) and nothing is written over
/// that file.
fn file_once(env: &JobEnv<'_>, key: &str, rel: &str, line: &str, report: &mut TickReport) {
    if report.intake_tried.iter().any(|k| k == key) {
        return;
    }
    let acks = match load_acks_checked(env.op.root, env.op.lane) {
        Ok(a) => Some(a),
        Err(e) => {
            report.lines.push(format!("{key}: 접수 확인 기록을 읽을 수 없습니다 — 보고는 하되 확인은 적지 않습니다({e:?})"));
            None
        }
    };
    if acks.as_ref().is_some_and(|a| a.contains_key(key)) {
        return;
    }
    report.intake_tried.push(key.to_string());
    if !(env.intake)(rel, line) {
        report.lines.push(format!("{key}: 보고가 아직 접수되지 않았습니다(한도 초과나 오류) — 다음 차례에 다시"));
        return;
    }
    let Some(mut acks) = acks else {
        return;
    };
    acks.insert(key.to_string(), env.op.now);
    if let Err(e) = super::durable::write_json_durable(&acks_path_in(env.op.root, env.op.lane), &acks) {
        report.lines.push(format!("{key}: 접수 확인을 적지 못했습니다 — 다음 차례에 같은 보고가 합쳐집니다({e:?})"));
    }
}

/// A recurring fault of one repair (unreadable record / row, a re-check or recovery error): a line
/// every tick, ONE intake row until acknowledged.
fn fault_once(env: &JobEnv<'_>, id: &RepairId, kind: &str, rel: &str, line: &str, report: &mut TickReport) {
    report.lines.push(format!("{}: {line} — 이 수리는 건너뛰고 나머지는 계속합니다", id.as_str()));
    let key = format!("{}:{kind}", id.as_str());
    let text = format!("자체 해결 작업이 수리 {}를 처리하지 못함 — {}", id.as_str(), line.chars().take(300).collect::<String>());
    file_once(env, &key, rel, &text, report);
}

/// EN1: every lane row whose closure owes an intake row (closed 실패, or 되돌려짐 by the daemon's
/// automatic return) files it until acknowledged — re-derived at every tick from the closures.
fn closure_obligations(env: &JobEnv<'_>, report: &mut TickReport) {
    // FG1: an unlistable root is the pass's own reported fault (probation_pass); unreadable rows are
    // reported there too (lane-row-unreadable) — here they are only not filed twice
    let Ok(ids) = ledger::list_ids_checked_in(env.op.root) else {
        return;
    };
    for id in ids {
        let Ok(Some(row)) = ledger::read_lane_in(env.op.root, &id, env.op.lane) else {
            continue;
        };
        let Some(closed) = row.probation.as_ref().and_then(|p| p.closed.clone()) else {
            continue;
        };
        // FG3: one duty per CLASS of terminal outcome (re-derived every tick from the row)
        let Some(class) = apply::closure_report_class(&closed) else {
            continue;
        };
        let rel = row.applied.first().map(|(r, _)| r.clone()).unwrap_or_else(|| format!("repairs/{}/record.json", id.as_str()));
        file_once(env, &format!("{}:closure:{class}", id.as_str()), &rel, &format!("시험 기간 재검사 실패 — 수리 {} · {closed}", id.as_str()), report);
    }
}

/// RULINGS FG3 — every hook operation of THIS lane that FINISHED declined (its step could not complete
/// — a first run or a resumed one) owes ONE intake row until acknowledged: re-derived every tick from
/// the operation records (recovery selects only unfinished records, so nothing else finds it again).
fn hook_terminal_obligations(env: &JobEnv<'_>, report: &mut TickReport) {
    let ids = match ledger::list_ids_checked_in(env.op.root) {
        Ok(ids) => ids,
        Err(e) => {
            report.lines.push(format!("훅 단계의 끝을 확인할 수 없습니다 — 수리 장부 목록을 읽을 수 없음({e:?})"));
            return;
        }
    };
    for id in ids {
        let ops = match super::wal::list_checked_in(env.op.root, &id) {
            Ok(ops) => ops,
            Err(e) => {
                fault_once(env, &id, "ops-unreadable", &format!("repairs/{}/ops", id.as_str()), &format!("동작 기록을 읽을 수 없습니다({e})"), report);
                continue;
            }
        };
        for op in ops.iter().filter(|o| o.lane_key == *env.op.lane && o.form == "hook" && o.finished && o.outcome.as_deref() == Some("declined")) {
            let hooks = op.hooks.iter().map(|h| format!("{}:{}", h.event, h.script)).collect::<Vec<_>>().join(", ");
            file_once(
                env,
                &format!("{}:hook-op:{}", id.as_str(), op.op),
                &format!("repairs/{}/ops/{}.json", id.as_str(), op.op),
                &format!("훅 단계가 끝나지 못했습니다 — 수리 {} · {} ({hooks}) · {}", id.as_str(), op.kind, op.note.chars().take(300).collect::<String>()),
                report,
            );
        }
    }
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
    // FG1: an unlistable root is an error of the pass (reported by the tick); an unreadable record is
    // said and skipped — never taken as absent
    let mut recs: Vec<RepairRecord> = Vec::new();
    for id in ledger::list_ids_checked_in(root)? {
        match ledger::read_record_in(root, &id) {
            Ok(Some(r)) => recs.push(r),
            Ok(None) => {}
            Err(e) => lines.push(format!("{}: 수리 기록을 읽을 수 없어 시작 때 정리에서 건너뜁니다({e:?})", id.as_str())),
        }
    }
    recs.sort_by(|a, b| b.id.cmp(&a.id)); // newest first (AK4 supersedes the older ones)
    for listed in &recs {
        // EJ4: the guard first (held → this repair is left alone), then the record re-read
        let Some(_guard) = super::guard::RepairGuard::try_take(root, &listed.id) else {
            continue;
        };
        let fresh = match ledger::read_record_in(root, &listed.id) {
            Ok(Some(f)) => f,
            Ok(None) => continue,
            Err(e) => {
                lines.push(format!("{}: 수리 기록을 읽을 수 없어 시작 때 정리에서 건너뜁니다({e:?})", listed.id.as_str()));
                continue;
            }
        };
        let rec = &fresh;
        if !in_force(rec) {
            continue;
        }
        // FG1: an unreadable row is never "no row" (that would reconstruct one over it)
        let row = match ledger::read_lane_in(root, &rec.id, lane) {
            Ok(r) => r,
            Err(e) => {
                lines.push(format!("{}: 이 부서의 행을 읽을 수 없어 시작 때 정리에서 건너뜁니다({e:?})", rec.id.as_str()));
                continue;
            }
        };
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
                match super::tombstones::load_in(root, lane) {
                    Ok(ts) if !ts.contains(&t) => match super::tombstones::record_in(root, lane, &t) {
                        Ok(()) => lines.push(format!("{}: 확정된 삭제의 묘비를 다시 세웠습니다 — {rel}", rec.id.as_str())),
                        Err(e) => lines.push(format!("{}: 묘비를 다시 세우지 못했습니다 — {rel}({e:?})", rec.id.as_str())),
                    },
                    Ok(_) => {}
                    Err(e) => lines.push(format!("{}: 묘비 목록을 읽을 수 없어 다시 세우지 않았습니다 — {rel}({e:?})", rec.id.as_str())),
                }
            }
        }
        // CT7: a hook-only repair (no committed file) is recovered too — its hook step is its state
        if rec.origin_lane != *lane || (rec.committed.is_empty() && ledger::hook_lines(rec).next().is_none()) {
            continue;
        }
        // (b3) the origin's own row FIRST (the hook step below marks it 충돌 when it cannot finish).
        // DR1 / CU4: the record holds no field naming the commit's generation, and a file's
        // modification time never decides anything — so the reconstructed row is written CLOSED 대체됨:
        // nothing is re-checked or put back against a generation nobody can confirm
        if row.is_none() {
            // EJ1 / EN3: a reconstructed row's generation is UNKNOWN (written as "")
            let (gen_version, gen_pro) = apply::Bound::Unknown.row_fields();
            let confirmed = false;
            let new_row = LaneRecord {
                schema: super::REPAIR_SCHEMA,
                repair: rec.id.clone(),
                lane: lane.clone(),
                state: super::types::LaneState::Verified,
                probation: Some(ledger::Probation {
                    pack_generation: gen_version,
                    pro_revision: gen_pro,
                    applied_at: env.op.now,
                    expected: rec.committed.clone(),
                    closed: (!confirmed).then(|| "대체됨 — 기록에 확정 때의 팩 세대가 없어 시험 기간을 다시 열지 않습니다(워커 검토)".to_string()),
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
            if let Err(e) = (env.locked)(&rec.id, &mut || {
        let _mutex_scope = super::guard::MutexScope::enter();
                if ledger::read_lane_in(&rootb, &idc, &lanec)?.is_none() {
                    ledger::write_lane_durable_in(&rootb, &new_row)?;
                }
                Ok(())
            }) {
                lines.push(format!("{}: 끊긴 확정의 부서 행을 쓰지 못했습니다({e:?})", rec.id.as_str()));
                continue;
            }
            lines.push(format!(
                "{}: 끊긴 확정의 부서 행을 썼습니다{}",
                rec.id.as_str(),
                if confirmed { "" } else { "(세대를 확인할 수 없어 대체됨으로 닫음)" }
            ));
        }
        // (b1) the hook step of the committed repair, when its unregistration never finished
        // FG1: an unreadable log is never "never attempted" (that would run the step again)
        let hook_ops = if ledger::hook_lines(rec).next().is_some() {
            match super::wal::list_checked_in(root, &rec.id) {
                Ok(ops) => Some(ops),
                Err(e) => {
                    lines.push(format!("{}: 동작 기록을 읽을 수 없어 끊긴 훅 단계를 확인하지 않았습니다({e})", rec.id.as_str()));
                    None
                }
            }
        } else {
            None
        };
        if let Some(ops) = hook_ops {
            let done = ops.iter().any(|o| o.kind == "hook-unregister" && o.finished && o.outcome.as_deref() == Some("applied"));
            let attempted = ops.iter().any(|o| o.kind == "hook-unregister");
            if !done && !attempted {
                match (env.hooks)(&rec.id, lane, HookStep::Unregister) {
                    Ok(_) => lines.push(format!("{}: 끊긴 확정의 훅 단계를 마쳤습니다", rec.id.as_str())),
                    Err(e) => {
                        let marked = apply::mark_lane_conflict(root, &rec.id, lane, env.op.now, env.locked)
                            .err()
                            .map(|m| format!(" · 충돌 표시도 쓰지 못함({m:?})"))
                            .unwrap_or_default();
                        lines.push(format!("{}: 끊긴 확정의 훅 단계 실패 — 충돌로 남깁니다({e:?}){marked}", rec.id.as_str()));
                    }
                }
            }
        }
        // (b2) older in-force repairs on the same targets
        match supersede_older(root, lane, rec, env.locked) {
            Ok((ids, deferred)) => {
                for oid in ids {
                    lines.push(format!("{}: 같은 파일의 더 새 수리 {} 로 대체됨", oid.as_str(), rec.id.as_str()));
                }
                for oid in deferred {
                    lines.push(format!("{}: {} — 더 새 수리 {} 로의 대체는 다음에 다시", oid.as_str(), super::guard::BUSY, rec.id.as_str()));
                }
            }
            Err(e) => lines.push(format!("{}: 더 오래된 수리를 정리하지 못했습니다({e:?})", rec.id.as_str())),
        }
    }
    Ok(lines)
}

/// The older in-force repairs of this lane on `newer`'s targets → superseded. FG5: each older repair
/// is written only under ITS guard, TRIED — held (a re-check, a return, a recovery owns it right now)
/// → deferred (returned second), never written under the mutex alone.
fn supersede_older(
    root: &Path,
    lane: &LaneKey,
    newer: &RepairRecord,
    locked: &RepairLock<'_>,
) -> Result<(Vec<RepairId>, Vec<RepairId>), RepairError> {
    let targets: Vec<String> = ledger::file_targets(newer).map(|f| f.rel.clone()).collect();
    let mut out = Vec::new();
    let mut deferred = Vec::new();
    let mut failed: Vec<String> = Vec::new();
    for oid in ledger::list_ids_checked_in(root)?.into_iter().filter(|o| *o < newer.id).rev() {
        let Some(_older_guard) = super::guard::RepairGuard::try_take(root, &oid) else {
            deferred.push(oid);
            continue;
        };
        let mut changed = false;
        let rootb = root.to_path_buf();
        // CU1: one older record that cannot be read or written never stops the pass — FG1: it is SAID
        if let Err(e) = locked(&oid, &mut || {
        let _mutex_scope = super::guard::MutexScope::enter();
            let Some(mut r) = ledger::read_record_in(&rootb, &oid)? else {
                return Ok(());
            };
            if r.origin_lane == *lane && in_force(&r) && r.files.iter().any(|f| targets.contains(&f.rel)) {
                r.state = RepairState::Superseded;
                ledger::write_record_durable_in(&rootb, &r)?;
                changed = true;
            }
            Ok(())
        }) {
            failed.push(format!("{}: {e:?}", oid.as_str()));
        }
        if changed {
            out.push(oid);
        }
    }
    if !failed.is_empty() {
        return Err(RepairError::Io(format!("더 오래된 수리를 대체됨으로 적지 못했습니다(대체된 것: {:?}) — {}", out, failed.join(" · "))));
    }
    Ok((out, deferred))
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
        let _mutex_scope = super::guard::MutexScope::enter();
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
            ledger::write_lane_durable_in(&root, &row)?;
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
        let _mutex_scope = super::guard::MutexScope::enter();
        if let Some(mut row) = ledger::read_lane_in(&root, &idc, &lane)? {
            if let Some(p) = row.probation.as_mut() {
                for t in timeouts {
                    if !p.check_timeouts.contains(t) {
                        p.check_timeouts.push(t.clone());
                    }
                }
            }
            ledger::write_lane_durable_in(&root, &row)?;
        }
        Ok(())
    })
}

/// Every repair in probation in THIS lane re-checked — only repaired files (no full sweep), with
/// the file's own check (never seat readiness — "보류" is not failure).
fn probation_pass(env: &JobEnv<'_>, report: &mut TickReport) {
    let root = env.op.root;
    let lane = env.op.lane;
    // FG1: an unlistable repairs root is never "no repairs" — the pass stops with ONE report
    let ids = match ledger::list_ids_checked_in(root) {
        Ok(ids) => ids,
        Err(e) => {
            let line = format!("수리 장부 목록을 읽을 수 없어 시험 기간 재검사를 하지 못했습니다({e:?})");
            report.lines.push(line.clone());
            report.deferred_probation = true;
            file_once(env, &format!("ledger:{}:repairs-root", lane.0), "repairs", &format!("자체 해결 작업이 {line}"), report);
            return;
        }
    };
    for id in ids {
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
        if p.closed.as_deref().is_some_and(|c| !apply::closure_owes_return(c) && !apply::closure_not_revertible(c)) {
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
    // EN1: the closures this pass (or an earlier one) wrote owe their intake rows — filed in THIS
    // tick, until acknowledged
    closure_obligations(env, report);
}

/// CU1 / CU4: a failing re-check that could NOT be reverted (or, on Windows, is never reverted
/// automatically) ends in a truthful FAILED state the ledger shows — the lane row 충돌 with its
/// probation closed `실패 — …` (never closed 사용 중 later); the record keeps its state with a line.
fn close_failed(env: &JobEnv<'_>, id: &RepairId, why: &str) -> Result<(), RepairError> {
    close_failed_as(env, id, why, true, false)
}

/// [`close_failed`]: `conflict` — the row → 충돌 (a final failure; not for a return still owed);
/// `replace_owed` — an existing closure that still OWES the return (EK2) is replaced (FG5: the owed
/// return ended otherwise); any other closure is never replaced.
fn close_failed_as(env: &JobEnv<'_>, id: &RepairId, why: &str, conflict: bool, replace_owed: bool) -> Result<(), RepairError> {
    let root = env.op.root.to_path_buf();
    let lane = env.op.lane.clone();
    let now = env.op.now;
    let idc = id.clone();
    let why = why.to_string();
    (env.locked)(id, &mut || {
        let _mutex_scope = super::guard::MutexScope::enter();
        if let Some(mut row) = ledger::read_lane_in(&root, &idc, &lane)? {
            if conflict {
                row.state = super::types::LaneState::Conflict;
            }
            if let Some(p) = row.probation.as_mut() {
                let replaceable = replace_owed && p.closed.as_deref().is_some_and(apply::closure_owes_return);
                if p.closed.is_none() || replaceable {
                    p.closed = Some(format!("{} — {why}", apply::CLOSED_FAILED).chars().take(400).collect());
                }
            }
            row.updated_at = now;
            ledger::write_lane_durable_in(&root, &row)?;
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

/// FG5 — an owed return (EK2) that `revert_in` refused or could not do: after the change of lock
/// scope the repair's guard is taken AGAIN (tried) and the state re-read; only a closure that still
/// owes the return is replaced — by `대체됨` when the repair is no longer in force (a newer repair took
/// its place meanwhile), else by a final FAILED closure. Guard held → left owed (retried next tick).
fn settle_owed_return(env: &JobEnv<'_>, id: &RepairId, detail: &str, report: &mut TickReport) -> Result<(), RepairError> {
    let Some(_guard) = super::guard::RepairGuard::try_take(env.op.root, id) else {
        report.lines.push(format!("{}: {} — 되돌리기는 남은 채로 다음 차례에 다시", id.as_str(), super::guard::BUSY));
        return Ok(());
    };
    let (Some(rec), Some(row)) = (ledger::read_record_in(env.op.root, id)?, ledger::read_lane_in(env.op.root, id, env.op.lane)?) else {
        return Ok(());
    };
    if !row.probation.as_ref().and_then(|p| p.closed.as_deref()).is_some_and(apply::closure_owes_return) {
        return Ok(()); // settled meanwhile
    }
    if rec.origin_lane == *env.op.lane && !in_force(&rec) {
        let root = env.op.root.to_path_buf();
        let lane = env.op.lane.clone();
        let now = env.op.now;
        let idc = id.clone();
        let text: String = format!("대체됨 — 재검사 실패 뒤 이 수리가 더 새 수리로 대체되어 되돌리지 않았습니다 · {detail}").chars().take(400).collect();
        (env.locked)(id, &mut || {
            let _mutex_scope = super::guard::MutexScope::enter();
            if let Some(mut row) = ledger::read_lane_in(&root, &idc, &lane)? {
                if let Some(p) = row.probation.as_mut() {
                    if p.closed.as_deref().is_some_and(apply::closure_owes_return) {
                        p.closed = Some(text.clone());
                    }
                }
                row.updated_at = now;
                ledger::write_lane_durable_in(&root, &row)?;
            }
            Ok(())
        })?;
        report.superseded.push(id.clone());
        report.lines.push(format!("{}: 재검사 실패 뒤 더 새 수리로 대체됨 — 되돌리지 않음 · {detail}", id.as_str()));
        return Ok(());
    }
    close_failed_as(env, id, &format!("재검사 실패, 되돌리지 못했습니다({detail}) — 워커 검토"), true, true)?;
    report.reported_only.push((id.clone(), detail.to_string()));
    report.lines.push(format!("{}: 재검사 실패 — 되돌리지 못해 실패로 기록했습니다({detail})", id.as_str()));
    Ok(())
}

/// The probation re-check of ONE repair in this lane.
/// `_listed_row` is the row as listed by the caller; every decision uses the row re-read under the
/// repair's guard (EJ4 / FG5).
pub fn recheck_one(env: &JobEnv<'_>, rec: &RepairRecord, _listed_row: &LaneRecord, report: &mut TickReport) -> Result<(), RepairError> {
    // EJ4: the repair's guard (tried) for the re-check, and every decision on state read AFTER it is
    // held; it is released before the pack-apply lock is tried for a return (EN5's order: lock →
    // guard → mutex) — `revert_in` takes the guard again and re-reads
    let Some(guard) = super::guard::RepairGuard::try_take(env.op.root, &rec.id) else {
        report.lines.push(format!("{}: {} — 이번 차례에는 다시 검사하지 않습니다", rec.id.as_str(), super::guard::BUSY));
        return Ok(());
    };
    let (Some(rec), Some(row)) = (ledger::read_record_in(env.op.root, &rec.id)?, ledger::read_lane_in(env.op.root, &rec.id, env.op.lane)?) else {
        return Ok(());
    };
    let (rec, row) = (&rec, &row);
    let Some(p) = row.probation.as_ref() else {
        return Ok(());
    };
    if let Some(closed) = p.closed.as_deref() {
        // FI1: not put back by the mechanism — nothing is re-checked for a return; at 24 hours the
        // record is in use (the row keeps its sentence)
        if apply::closure_not_revertible(closed) {
            if env.op.now - p.applied_at >= ledger::PROBATION_SECS as f64 && rec.state == RepairState::Probation {
                close_row(env, &rec.id, "사용 중 — 24시간 시험 기간을 마쳤습니다", &[], Some(RepairState::InUse))?;
                report.ended.push(rec.id.clone());
                report.lines.push(format!("{}: 시험 기간을 마치고 사용 중({})", rec.id.as_str(), apply::NOT_REVERTIBLE_SENTENCE));
            }
            return Ok(());
        }
        if !apply::closure_owes_return(closed) || env.op.form != ApplyForm::Exchange {
            return Ok(());
        }
        // EK2: the owed return is retried (the recorded failure stands — nothing is re-measured)
        drop(guard);
        match crate::pack_apply_lock::try_with_apply_lock(env.apply_lock, || {
            revert_in(&env.op, &rec.id, RevertCaller::DaemonAuto, env.hooks, env.locked)
        }) {
            Ok(Some(Ok(r))) if r.outcome == ApplyOutcome::Applied => {
                report.reverted.push(rec.id.clone());
                report.lines.push(format!("{}: 미뤄 둔 되돌리기를 했습니다(시험 기간 재검사 실패)", rec.id.as_str()));
            }
            // the return ran to its end (충돌 / 대체됨 — `finish_revert` wrote the closure)
            Ok(Some(Ok(r))) => report.lines.push(format!("{}: 미뤄 둔 되돌리기를 마치지 못했습니다({:?})", rec.id.as_str(), r.outcome)),
            // refused or failed before its end: FG5 — settled under the guard taken again
            Ok(Some(Err(e))) => settle_owed_return(env, &rec.id, &format!("{e:?}"), report)?,
            Ok(None) => {
                report.deferred_probation = true;
                report.lines.push(format!("{}: 미뤄 둔 되돌리기 — 팩이 아직 쓰이는 중(다음 차례에 다시)", rec.id.as_str()));
            }
            Err(e) => report.lines.push(format!("{}: 미뤄 둔 되돌리기 — 잠금을 확인할 수 없음({e})", rec.id.as_str())),
        }
        return Ok(());
    }
    let id = &rec.id;
    // the binding: this lane, the pack generation, the expected live states
    if !apply::Bound::from_row(p).equals_now(env.op.pack) {
        let why = format!(
            "팩 세대가 적용 때와 다르거나 알 수 없습니다(지금 {}) — 아무것도 되돌리지 않습니다(새 버전 우선)",
            apply::pack_generation(env.op.pack).version
        );
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
            // FG5: the measured failure is made durable WHILE the guard is held (the closure; the
            // return still owed — EK2); only then is the guard released for the lock
            close_failed_as(env, id, &format!("{} · 재검사 실패 · {why}", apply::RETURN_OWED), false, false)?;
            drop(guard);
            if env.op.faults.at(apply::Point::RecheckHandOver, &env.op.pack.join(first_rel), None) {
                return Err(apply::stop(apply::Point::RecheckHandOver));
            }
            // DX3: the job NEVER waits for the pack-apply lock — held → no return this tick, retried
            let r = match crate::pack_apply_lock::try_with_apply_lock(env.apply_lock, || {
                revert_in(&env.op, id, RevertCaller::DaemonAuto, env.hooks, env.locked)
            }) {
                Ok(Some(r)) => r,
                Ok(None) => {
                    report.deferred_probation = true;
                    report.lines.push(format!("{}: 재검사 실패 — 다른 적용이나 업데이트가 팩을 바꾸는 중이라 이번 차례에는 되돌리지 않았습니다(실패로 기록 · 다음 차례에 다시)", id.as_str()));
                    return Ok(());
                }
                Err(e) => {
                    report.lines.push(format!("{}: 재검사 실패 — 잠금을 확인할 수 없어 되돌리기는 남은 채로 다음 차례에 다시({e})", id.as_str()));
                    return Ok(());
                }
            };
            // DR8: the intake description is derived from the MEASURED outcome of the revert
            let outcome_text = match &r {
                Ok(x) if x.outcome == ApplyOutcome::Applied => "되돌림".to_string(),
                Ok(x) => format!("되돌리지 못함({:?})", x.outcome),
                Err(e) => format!("되돌리지 못함({e:?})"),
            };
            // EN1: the intake row is owed by the CLOSURE this revert / failure writes (filed by
            // `closure_obligations`), never by a separate pending entry
            report.lines.push(format!("{}: 시험 기간 재검사 실패 — {outcome_text} — {why}", id.as_str()));
            let _ = first_rel;
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
                Ok(r) => {
                    // the return ran to its end but not completely: `finish_revert` closed the row
                    // 충돌 (it owes its report — FG3)
                    report.reported_only.push((id.clone(), why.clone()));
                    report.lines.push(format!("{}: 재검사 실패 — 되돌리기를 마치지 못했습니다({:?}) · {why}", id.as_str(), r.outcome));
                }
                Err(e) => {
                    // CU1 / FG5: refused or failed before its end — settled under the guard taken
                    // again (never closed 사용 중)
                    settle_owed_return(env, id, &format!("{e:?} · {why}"), report)?;
                }
            }
        } else {
            // Windows 0.15.0: automatic return is off — ONE intake row and a FAILED end of this
            // probation (CU4: never 사용 중 later; the row closed, so it is not re-filed every check);
            // FG5: written while the guard is held
            note_timeouts(env, id, &timeouts)?;
            let _ = first_rel;
            close_failed(env, id, &format!("재검사 실패(이 운영체제에서는 자동으로 되돌리지 않음 — 기록·알림) · {why} — 워커 검토"))?;
            report.reported_only.push((id.clone(), why.clone()));
            report.lines.push(format!("{}: 재검사 실패 — 이 운영체제에서는 자동으로 되돌리지 않고 실패로 기록·알림만 합니다 · {why}", id.as_str()));
            drop(guard);
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

/// RULINGS FG1 — a JSON file of the repair module read with three answers: `Ok(Some(value))`;
/// `Ok(None)` only on positive evidence (the file does not exist AND its folder — or, when the folder
/// is missing too, the nearest existing ancestor — can be read); `Err` for everything else (a read
/// error, a parse error). RULINGS HF2 — read through GK5's ONE bounded reader (a non-blocking open
/// that follows no link, a regular file, the size cap): a FIFO, a link or an oversized file is
/// unreadable at once, never waited for (the notice book reads these files under its mutex).
fn read_json_checked<T: serde::de::DeserializeOwned>(p: &Path) -> Result<Option<T>, RepairError> {
    match super::durable::read_record_bytes(p) {
        super::durable::RecordRead::Bytes(b) => serde_json::from_slice::<T>(&b).map(Some).map_err(|e| RepairError::Io(format!("{}: 읽을 수 없음(손상) — {e}", p.display()))),
        super::durable::RecordRead::Unreadable(why) => Err(RepairError::Io(format!("읽을 수 없음 — {why}"))),
        super::durable::RecordRead::NotFound => {
            let mut up = p.parent();
            while let Some(a) = up {
                match std::fs::read_dir(a) {
                    Ok(_) => return Ok(None),
                    Err(e2) if e2.kind() == std::io::ErrorKind::NotFound => up = a.parent(),
                    Err(e2) => return Err(RepairError::Io(format!("{}: 없음, 그러나 상위 폴더({})를 읽을 수 없음 — {e2}", p.display(), a.display()))),
                }
            }
            Err(RepairError::Io(format!("{}: 없음, 그러나 읽을 수 있는 상위 폴더가 없음", p.display())))
        }
    }
}

/// FG1: the findings view with three answers (absent on positive evidence = no findings).
fn load_findings_checked(root: &Path, lane: &LaneKey) -> Result<Vec<Finding>, RepairError> {
    Ok(read_json_checked::<FindingsFile>(&findings_path_in(root, lane))?.map(|f| f.findings).unwrap_or_default())
}

/// RULINGS FG2 / FI3 — `<root>/m10/<lane>.notices-acked.json`: the keys of the owner notices the
/// receiver was handed (the acknowledgement), written in ONE durable write per hand-over.
pub fn notices_acked_path_in(root: &Path, lane: &LaneKey) -> PathBuf {
    root.join("m10").join(format!("{}.notices-acked.json", lane.0))
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct NoticeAcks {
    schema: u32,
    lane: String,
    keys: std::collections::BTreeSet<String>,
}

/// FG1 / FG2: the acknowledged notice keys (absent on positive evidence = none acknowledged).
fn load_notice_acks(root: &Path, lane: &LaneKey) -> Result<std::collections::BTreeSet<String>, RepairError> {
    Ok(read_json_checked::<NoticeAcks>(&notices_acked_path_in(root, lane))?.map(|a| a.keys).unwrap_or_default())
}

/// RULINGS GK3 — how many times a notice may be handed out without its receiver's confirmation;
/// after that it is closed "not confirmed": never handed out again, shown in `cys repair status`.
pub const NOTICE_HAND_OUTS: u32 = 3;

/// `<root>/m10/<lane>.notices-handed.json` — the hand-out count of each notice key (GK3).
pub fn notices_handed_path_in(root: &Path, lane: &LaneKey) -> PathBuf {
    root.join("m10").join(format!("{}.notices-handed.json", lane.0))
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct NoticeHandOuts {
    schema: u32,
    lane: String,
    counts: std::collections::BTreeMap<String, u32>,
}

/// FG1 / GK3: the hand-out counts (absent on positive evidence = none handed out yet).
fn load_hand_outs(root: &Path, lane: &LaneKey) -> Result<std::collections::BTreeMap<String, u32>, RepairError> {
    Ok(read_json_checked::<NoticeHandOuts>(&notices_handed_path_in(root, lane))?.map(|h| h.counts).unwrap_or_default())
}

/// RULINGS GV2 — the ONE short in-process critical section of the notice book-keeping: choosing
/// the due notices with reserving their hand-outs, and the acknowledgement. It never spans the job
/// or a send (it lives here, not in the job's flight guard).
static NOTICE_BOOK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// RULINGS GK3 / GV2 — chooses the notices due NOW and reserves a hand-out for each, in ONE critical
/// section and ONE durable write of the counts (keys acknowledged meanwhile are dropped from the
/// counts); a key at its limit is never chosen. Returns only the notices whose reservation was
/// written (an error = none handed out — they stay due). Nothing is acknowledged.
pub fn hand_out_due_notices_in(root: &Path, lane: &LaneKey) -> Result<Vec<DueNotice>, RepairError> {
    let _book = NOTICE_BOOK.lock().unwrap_or_else(|e| e.into_inner());
    let due = due_notices_in(root, lane)?;
    if due.is_empty() {
        return Ok(due);
    }
    let acked = load_notice_acks(root, lane)?;
    let mut counts = load_hand_outs(root, lane)?;
    counts.retain(|k, _| !acked.contains(k));
    for d in &due {
        *counts.entry(d.key.clone()).or_insert(0) += 1;
    }
    super::durable::write_json_durable(
        &notices_handed_path_in(root, lane),
        &NoticeHandOuts { schema: super::REPAIR_SCHEMA, lane: lane.0.clone(), counts },
    )?;
    Ok(due)
}

/// RULINGS GK3 — the notices closed "not confirmed": handed out [`NOTICE_HAND_OUTS`] times without
/// their receiver's confirmation (and not acknowledged since).
pub fn notices_not_confirmed_in(root: &Path, lane: &LaneKey) -> Result<Vec<String>, RepairError> {
    let acked = load_notice_acks(root, lane)?;
    Ok(load_hand_outs(root, lane)?
        .into_iter()
        .filter(|(k, n)| *n >= NOTICE_HAND_OUTS && !acked.contains(k))
        .map(|(k, _)| k)
        .collect())
}

/// The acknowledgement key of a finding's notice (one per file, bytes and kind).
pub fn notice_key_of_finding(f: &Finding) -> String {
    format!("f:{}:{}:{}", f.rel, f.sha256, f.kind)
}

/// The acknowledgement key of a completed put-back's notice (one per operation).
pub fn notice_key_of_op(op: &super::wal::OpRecord) -> String {
    format!("op:{}", op.op)
}

/// RULINGS FG2 — one notice the owner is owed and the key that acknowledges it.
#[derive(Debug, Clone, PartialEq)]
pub struct DueNotice {
    pub key: String,
    pub finding: Finding,
}

fn save_findings(root: &Path, lane: &LaneKey, findings: &[Finding]) -> Result<(), RepairError> {
    // FG2: an acknowledged notice is folded into the view as told (when the acknowledgements can be
    // read; otherwise the view keeps it untold — it is never dropped)
    let acked = load_notice_acks(root, lane).unwrap_or_default();
    let findings: Vec<Finding> = findings
        .iter()
        .cloned()
        .map(|mut f| {
            if f.notice && !f.notified && acked.contains(&notice_key_of_finding(&f)) {
                f.notified = true;
            }
            f
        })
        .collect();
    let f = FindingsFile {
        schema: super::REPAIR_SCHEMA,
        lane: lane.0.clone(),
        // EJ3: a view never evicts an undelivered notice; only delivered / notice-less entries age out
        findings: {
            let keep_old = findings.len().saturating_sub(500);
            findings
                .iter()
                .enumerate()
                .filter(|(i, f)| *i >= keep_old || (f.notice && !f.notified))
                .map(|(_, f)| f.clone())
                .collect()
        },
    };
    super::durable::write_json_durable(&findings_path_in(root, lane), &f)
}

/// RULINGS FG2 / FI3 — step ONE: the notices the owner is owed, READ ONLY (nothing is marked). The
/// findings not yet told, and every completed put-back whose notice is due (EJ3: the duty lives on
/// its operation record), minus the acknowledged keys. Any source that cannot be read is an `Err` —
/// never an empty list (FG1).
pub fn due_notices_in(root: &Path, lane: &LaneKey) -> Result<Vec<DueNotice>, RepairError> {
    let acked = load_notice_acks(root, lane)?;
    // GK3: a notice handed out NOTICE_HAND_OUTS times without confirmation is closed — not due
    let closed: std::collections::BTreeSet<String> = load_hand_outs(root, lane)?
        .into_iter()
        .filter(|(_, n)| *n >= NOTICE_HAND_OUTS)
        .map(|(k, _)| k)
        .collect();
    let acked: std::collections::BTreeSet<String> = acked.union(&closed).cloned().collect();
    let mut out = Vec::new();
    for f in load_findings_checked(root, lane)? {
        let key = notice_key_of_finding(&f);
        if f.notice && !f.notified && !acked.contains(&key) {
            out.push(DueNotice { key, finding: f });
        }
    }
    let sub = m10_root_in(root, lane);
    let ops = super::wal::list_checked_in(&sub, &m10_bucket()).map_err(RepairError::Io)?;
    for op in ops.into_iter().filter(|o| o.lane_key == *lane && o.notice.as_deref() == Some("due")) {
        let key = notice_key_of_op(&op);
        if acked.contains(&key) {
            continue;
        }
        let sha = match apply::state_of_canon(&op.expected) {
            Some(TargetState::File { sha256, .. }) => sha256,
            _ => String::new(),
        };
        let mut f = Finding {
            rel: op.rel.clone(),
            sha256: sha,
            kind: "reverted".into(),
            at: op.updated_at,
            notice: true,
            notified: true,
            slot: op.slot.clone(),
            kept_copy: None,
            note: "검사 결과: 실패".into(),
        };
        finalize_finding(&mut f, Path::new(""));
        out.push(DueNotice { key, finding: f });
    }
    Ok(out)
}

/// RULINGS FG2 / FI3 — step TWO: the receiver was handed these notices — their keys are acknowledged
/// in ONE durable write (all of them or none: a failure leaves every one of them due). Keys whose
/// notice is no longer owed (a finding the view now shows as told or no longer holds) are dropped in
/// the same write when the sources can be read.
pub fn ack_notices_in(root: &Path, lane: &LaneKey, keys: &[String]) -> Result<(), RepairError> {
    if keys.is_empty() {
        return Ok(());
    }
    // GV2: serialised with the choice-and-reservation of hand-outs
    let _book = NOTICE_BOOK.lock().unwrap_or_else(|e| e.into_inner());
    let mut acked = load_notice_acks(root, lane)?;
    let owed: Option<std::collections::BTreeSet<String>> = (|| {
        let mut s = std::collections::BTreeSet::new();
        for f in load_findings_checked(root, lane).ok()? {
            if f.notice && !f.notified {
                s.insert(notice_key_of_finding(&f));
            }
        }
        for op in super::wal::list_checked_in(&m10_root_in(root, lane), &m10_bucket()).ok()? {
            if op.notice.as_deref() == Some("due") {
                s.insert(notice_key_of_op(&op));
            }
        }
        Some(s)
    })();
    if let Some(owed) = owed {
        acked.retain(|k| owed.contains(k));
    }
    acked.extend(keys.iter().cloned());
    super::durable::write_json_durable(
        &notices_acked_path_in(root, lane),
        &NoticeAcks { schema: super::REPAIR_SCHEMA, lane: lane.0.clone(), keys: acked },
    )
}

/// The notices owed and, in the same call, acknowledged ([`due_notices_in`] then [`ack_notices_in`]) —
/// for a receiver that holds them the moment this returns `Ok` (the daemon's trigger uses the two
/// steps itself, acknowledging only once the reply is handed over).
pub fn take_notices_in(root: &Path, lane: &LaneKey) -> Result<Vec<Finding>, RepairError> {
    let due = due_notices_in(root, lane)?;
    let keys: Vec<String> = due.iter().map(|d| d.key.clone()).collect();
    ack_notices_in(root, lane, &keys)?;
    Ok(due.into_iter().map(|d| d.finding).collect())
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
    m10_sweep_with(env, &std::cell::Cell::new(false))
}

/// [`m10_sweep`]; `report_deferred_sweep` is set when a put-back was skipped for the lock (DX3).
fn m10_sweep_with(env: &JobEnv<'_>, report_deferred_sweep: &std::cell::Cell<bool>) -> Result<Vec<Finding>, RepairError> {
    m10_sweep_full(env, report_deferred_sweep, &std::cell::Cell::new(None))
}

/// [`m10_sweep_with`]; `unreadable_cause` is set to the unreadable ledger member that suspended the
/// scan (EN4 — the tick files ONE intake row for it).
fn m10_sweep_full(env: &JobEnv<'_>, report_deferred_sweep: &std::cell::Cell<bool>, unreadable_cause: &std::cell::Cell<Option<String>>) -> Result<Vec<Finding>, RepairError> {
    let root = env.op.root;
    let lane = env.op.lane;
    let pack = env.op.pack;
    // FG1: an unreadable view or acknowledgement file stops the scan (a reported fault) — it is never
    // read as empty and overwritten
    let mut existing = load_findings_checked(root, lane)?;
    let acked = load_notice_acks(root, lane)?;
    let mut new_findings = Vec::new();
    // EJ3: the findings file is a VIEW; a put-back's notice duty lives on its operation record
    let mut ledger_unreadable: Option<(String, Vec<String>)> = None;
    // CT9: a put-back's finding survives a crash between the exchange and the findings write — it is
    // rebuilt idempotently from the completed operation records of the M10 sub-ledger
    let sub_root = m10_root_in(root, lane);
    let (done_ops, _) = super::wal::list_in(&sub_root, &m10_bucket());
    for op in done_ops.into_iter().filter(|o| {
        o.finished && o.outcome.as_deref() == Some("applied") && o.kind == "m10-revert" && o.lane_key == *lane
    }) {
        // EJ3: the view is rebuilt only for a put-back whose notice is still DUE and that the view
        // does not show; a delivered one never comes back
        if op.notice.as_deref() != Some("due") || acked.contains(&notice_key_of_op(&op)) {
            continue;
        }
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
            notice: false, // a view: the duty is the operation's `notice`
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
        let bound = apply::Bound::now(pack);
        let live = pack.join(rel);
        let m = match std::fs::symlink_metadata(&live) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue, // absent: the heal paths own a missing file
            Err(e) => {
                // FG1: a file that cannot be examined is a FINDING — never "absent"
                unreadable_finding(&existing, &mut new_findings, rel, &format!("{e}"), env.op.now);
                continue;
            }
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
        let bytes = match std::fs::read(&live) {
            Ok(b) => b,
            Err(e) => {
                unreadable_finding(&existing, &mut new_findings, rel, &format!("{e}"), env.op.now);
                continue;
            }
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
        // DR2 — ONE resolution for recognition AND restoration: the approved bytes live → nothing to
        // do (F9); APPROVED → the approved copy is the source; NO APPROVAL → our original; UNKNOWN →
        // the check still runs, nothing is put back, a finding for the worker
        let approved = apply::resolve_approved_copy_in(root, pack, lane, rel);
        if let apply::ApprovedCopy::Approved { holder, .. } = &approved {
            if std::fs::read(holder).ok().as_deref() == Some(bytes.as_slice()) {
                continue;
            }
        }
        // EN4: an unreadable ledger member suspends every put-back of this lane — the file is listed in
        // ONE finding for the cause (written at the end of the sweep), nothing per file
        if let apply::ApprovedCopy::LedgerUnreadable { cause } = &approved {
            ledger_unreadable.get_or_insert_with(|| (cause.clone(), Vec::new())).1.push(rel.to_string());
            continue;
        }
        let approval_unknown = match &approved {
            apply::ApprovedCopy::Unknown(why) => Some(why.clone()),
            _ => None,
        };
        let lf_only = false;
        let deadline = check_deadline(rel);
        let verdict = match (env.check)(rel, pack, deadline) {
            Ok(o) => o.result,
            Err(_) => continue, // no runner: nothing decided
        };
        if let Some(why) = &approval_unknown {
            if !already("approval-unknown") {
                new_findings.push(Finding {
                    rel: rel.to_string(),
                    sha256: sha.clone(),
                    kind: "approval-unknown".into(),
                    at: env.op.now,
                    notice: true,
                    notified: false,
                    slot: None,
                    kept_copy: None,
                    note: format!("주인 승인 여부를 확인할 수 없어 아무것도 되돌리지 않았습니다({why}) — 검사 결과: {verdict:?}(워커 검토)"),
                });
            }
            continue;
        }
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
        let live_mode = match TargetState::read(&live) {
            Ok(TargetState::File { mode, .. }) => mode,
            Ok(_) => continue, // changed kind since it was read — the next scan sees it
            Err(e) => {
                unreadable_finding(&existing, &mut new_findings, rel, &format!("{e:?}"), env.op.now);
                continue;
            }
        };
        // the put-back content: the approved copy of the current generation, else our original of it
        let target: Option<(Vec<u8>, u32)> = match &approved {
            apply::ApprovedCopy::Approved { state: TargetState::File { mode, .. }, holder, .. } => std::fs::read(holder).ok().map(|b| (b, *mode)),
            apply::ApprovedCopy::Approved { .. } | apply::ApprovedCopy::Unknown(_) | apply::ApprovedCopy::LedgerUnreadable { .. } => None,
            apply::ApprovedCopy::NoApproval => original.clone().map(|o| (o, live_mode)),
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
        // GK1: an approved copy is a slot's bytes — that slot's own generation binds the put-back;
        // our original of the current generation is fresh bytes (bound by the scan's sampling)
        let source = match &approved {
            apply::ApprovedCopy::Approved { holder, .. } => apply::Source::Slot { holder: holder.clone() },
            _ => {
                apply::write_new(&src_file, &orig, mode)?;
                apply::Source::Fresh(src_file.clone())
            }
        };
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
            // GK2: M10 replaces bytes it did not place (F9 / M10) — ONLY the exact state its check
            // sampled, revalidated under the lock below; a later write by anyone is never replaced
            expected: apply::Recorded::sampled_by_scan(sampled_live.clone()),
            incoming: TargetState::File { sha256: source_sha.clone(), mode },
            source,
            bound: bound.clone(),
        };
        let busy = std::cell::Cell::new(false);
        let r = crate::pack_apply_lock::try_with_apply_lock(env.apply_lock, || -> Result<Option<apply::OpDone>, RepairError> {
            // EJ4: the sub-ledger's guard under the lock (EN5's order); held → this put-back waits
            let Some(_guard) = super::guard::RepairGuard::try_take(&sub, &m10_bucket()) else {
                busy.set(true);
                return Ok(None);
            };
            // CT5: THE binding, revalidated under the lock — generation, live state, source of the bytes
            if !bound.equals_now(pack) {
                return Ok(None);
            }
            if TargetState::read(&live)? != sampled_live {
                return Ok(None);
            }
            let source_now: Option<String> = match apply::resolve_approved_copy_in(root, pack, lane, rel) {
                apply::ApprovedCopy::Approved { state: TargetState::File { .. }, holder, .. } => std::fs::read(&holder).ok().map(|b| sha_hex(&b)),
                apply::ApprovedCopy::Approved { .. } | apply::ApprovedCopy::Unknown(_) | apply::ApprovedCopy::LedgerUnreadable { .. } => None,
                apply::ApprovedCopy::NoApproval => (env.original)(rel).map(|o| sha_hex(&o)),
            };
            if source_now.as_deref() != Some(source_sha.as_str()) {
                return Ok(None);
            }
            apply::run_op(&sub_env, &spec).map(Some)
        })
        .map_err(RepairError::Io)?;
        let _ = std::fs::remove_file(&src_file); // our own source copy (the slot holds what went live)
        // DX3: the job never waits for the lock — held → no put-back this tick, the scan stays due
        let Some(r) = r else {
            report_deferred_sweep.set(true);
            continue;
        };
        if busy.get() {
            report_deferred_sweep.set(true);
            continue;
        }
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
            // EJ3: a completed put-back's notice is its operation's duty (`notice: due`); the view
            // entry carries none; an attempt that put nothing back is noticed through the view
            notice: notice && kind != "reverted",
            notified: false,
            slot: r.slot,
            kept_copy: None,
            note,
        });
    }
    for f in new_findings.iter_mut() {
        finalize_finding(f, pack);
    }
    // EN4: ONE finding for the CAUSE of a suspended scan (the unreadable record, the files inside it)
    if let Some((cause, _)) = &ledger_unreadable {
        unreadable_cause.set(Some(cause.clone()));
    }
    if let Some((cause, rels)) = ledger_unreadable {
        let key = sha_hex(cause.as_bytes());
        if !existing.iter().chain(new_findings.iter()).any(|f| f.kind == "ledger-unreadable" && f.sha256 == key) {
            new_findings.push(Finding {
                rel: cause.clone(),
                sha256: key,
                kind: "ledger-unreadable".into(),
                at: env.op.now,
                notice: true,
                notified: false,
                slot: None,
                kept_copy: None,
                note: format!(
                    "장부의 {cause} 을(를) 읽을 수 없어 이 부서의 한 시간마다 되돌리기를 멈췄습니다(워커가 그 기록을 고칠 때까지) — 해당 파일: {}",
                    rels.join(", ")
                ),
            });
        }
    }
    if !new_findings.is_empty() {
        existing.extend(new_findings.iter().cloned());
        save_findings(root, lane, &existing)?;
    }
    Ok(new_findings)
}


/// FG1: a boot-critical file the scan could not examine — ONE finding per (file, cause), told once.
fn unreadable_finding(existing: &[Finding], new_findings: &mut Vec<Finding>, rel: &str, cause: &str, now: f64) {
    let key = sha_hex(cause.as_bytes());
    if existing.iter().chain(new_findings.iter()).any(|f| f.rel == rel && f.kind == "unreadable" && f.sha256 == key) {
        return;
    }
    new_findings.push(Finding {
        rel: rel.to_string(),
        sha256: key,
        kind: "unreadable".into(),
        at: now,
        notice: true,
        notified: false,
        slot: None,
        kept_copy: None,
        note: format!("기동 필수 파일을 살펴볼 수 없습니다({cause}) — 아무것도 바꾸지 않았습니다(워커 검토)"),
    });
}

/// The resolved restore content of M10 for `rel` in a pack: the binary's embedded copy, only when
/// the pack's generation is this binary's own (version = this build's version, free channel) —
/// otherwise unknown (never an older or foreign generation's bytes).
pub fn embedded_original_for(pack: &Path, rel: &str) -> Option<Vec<u8>> {
    if !apply::Bound::now(pack).is_release(env!("CARGO_PKG_VERSION")) {
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
