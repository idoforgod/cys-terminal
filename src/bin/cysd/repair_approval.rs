//! 「스스로 해결」 — the FINAL-APPROVAL channel (C5 · design §4-7 steps 2–10, §6-7 "승인"). Track T5b.
//!
//! A dedicated IN-MEMORY request table (never written to disk — C5 5.4) with its own `repair.*`
//! verbs: `submit` (a worker holding the repair's lock; returns its number at once — 5.6),
//! `approval_list` (the app's card list), `approval_start` (the owner's click: the DAEMON spawns a
//! short-lived helper that runs the OS user verification; the daemon trusts only its own child's
//! exit status — 5.9) and `approval_reject`. Step 3 (the rehearsal re-run and the protection
//! screen) runs off the request thread after `submit` has returned; a failing step 3 offers nothing
//! to the owner. Expiry (30 minutes, by timestamp — I3) is swept in the governance watchdog (5.14,
//! not a scheduled job); every result is delivered ONCE to the fixer seat's queue (5.7, never the
//! alert path); the resubmission budget is durable in the repair record (S1, U4(c)).
//! ★Nothing here waits for the owner, and no request handler blocks (F2 / F4 — the 128-request cap).
//! ★Nothing here panics (no `unwrap` / `expect` / indexing in production code).
//! ★The OS check never runs inside the daemon process (a crash there would take every pane — F4).

use crate::repair::{Caller, Ctx, VerbError};
use crate::state::Daemon;
use cys::repair::ledger::{self, ApprovalResult, RepairRecord, APPROVAL_TTL_SECS, MAX_RESUBMITS};
use cys::repair::types::{
    Classification, LaneKey, RepairError, RepairId, RepairState, TargetState, ERR_PRECONDITION,
};
use serde::Serialize;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[cfg(test)]
#[path = "repair_approval_tests.rs"]
mod tests;

/// The sub-verbs whose bodies are T5b's (routed by [`route`] in `repair::handle`).
pub(crate) const T5B_VERBS: &[&str] = &[
    "submit",
    "approval_list",
    "approval_start",
    "approval_reject",
];

/// The request-id prefix (O-C5-2 `rq-<random>`): an id space the daemon's other request ids do not
/// use (`daemon-` feed approvals, team proposals, cycle jobs) — design guard rail "데몬 전용 요청 번호
/// 접두사" (D:1128).
pub(crate) const REQUEST_PREFIX: &str = "rq-";
/// The helper invalidates its own verification at 110 s; the daemon kills + reaps it at 120 s (B8).
#[allow(dead_code)] // the helper owns its own 110 s (`repair_cli::auth_helper`); pinned here beside the kill
pub(crate) const HELPER_INVALIDATE_SECS: u64 = 110;
pub(crate) const HELPER_KILL_SECS: u64 = 120;
/// Step 3's wall time (the rehearsal re-run + the protection screen); exceeded = the candidate is
/// returned to the worker (fail closed).
pub(crate) const STEP3_DEADLINE: Duration = Duration::from_secs(600);
/// How long an ended request stays visible in `status` / the list (memory only).
const ENDED_KEEP_SECS: f64 = 6.0 * 3600.0;
/// Longest worker text shown on the card (filtered, separated from the daemon's facts — E2).
const WORKER_TEXT_MAX: usize = 1200;

/// A request's state (O-C5-2 + `checking` = step 3 still running: no card yet).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReqState {
    Checking,
    Pending,
    Verifying,
    Approved,
    Rejected,
    Expired,
    Voided,
}

impl ReqState {
    fn live(self) -> bool {
        matches!(
            self,
            ReqState::Checking | ReqState::Pending | ReqState::Verifying
        )
    }

    fn label_ko(self) -> &'static str {
        match self {
            ReqState::Checking => "제출 검사 중",
            ReqState::Pending => "주인 승인 대기",
            ReqState::Verifying => "본인 확인 중",
            ReqState::Approved => "승인됨",
            ReqState::Rejected => "승인되지 않음",
            ReqState::Expired => "만료",
            ReqState::Voided => "무효",
        }
    }
}

/// One request (O-C5-2). Memory only.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct Request {
    pub request_id: String,
    pub repair: String,
    pub lane_key: String,
    pub worker_seat: u64,
    pub fingerprint_sha256: String,
    pub targets: Vec<String>,
    #[serde(skip)]
    pub nonce: String,
    pub created_at: f64,
    pub expires_at: f64,
    pub resubmits_used: u32,
    pub state: ReqState,
    pub rehearsal: Option<(bool, String)>,
    pub prerehearsal_possible: bool,
    pub helper_pid: Option<u32>,
    pub classification: Classification,
    /// The screen's notes (accepted with notes) — shown on the card.
    pub screen_notes: Vec<String>,
    /// Why the request ended (voided / not approved / declined at apply).
    pub reason: Option<String>,
    /// The worker's own text (filtered) — shown APART from the daemon's facts.
    pub worker_text: String,
    pub ended_at: Option<f64>,
}

/// The pending final-approval requests (C5; memory only — void after a restart).
#[derive(Default)]
pub(crate) struct ApprovalTable {
    pub(crate) requests: Vec<Request>,
    /// The first sweep of this daemon incarnation resolved the requests a previous one left in the
    /// records (V2(d): expired if their time had passed, else voided by the restart).
    pub(crate) restart_resolved: bool,
}

impl ApprovalTable {
    fn find(&self, id: &str) -> Option<&Request> {
        self.requests.iter().find(|r| r.request_id == id)
    }

    fn find_mut(&mut self, id: &str) -> Option<&mut Request> {
        self.requests.iter_mut().find(|r| r.request_id == id)
    }

    fn live_for(&self, repair: &str) -> Option<&Request> {
        self.requests
            .iter()
            .find(|r| r.repair == repair && r.state.live())
    }
}

/// The side effects of the channel — production: [`DaemonEffects`]; tests: a recorder.
pub(crate) trait Effects: Send + Sync {
    /// Delivers `text` ONCE to the fixer seat (the cleanup-result path, never the alert path).
    fn deliver(&self, seat: u64, text: String);
    /// The lane master's one line to the owner (budget exhausted — once).
    fn tell_master(&self, text: String);
    /// An event on the existing bus (`repair.approval.created` / `.resolved`).
    fn publish(&self, name: &str, payload: Value);
    /// Spawns the OS-verification helper (the daemon's own child).
    fn spawn_helper(&self, request_id: &str, reason: &str) -> std::io::Result<std::process::Child>;
    /// The apply of an approved candidate (T6 — `apply::apply_candidate`).
    fn apply(
        &self,
        id: &RepairId,
        lane: &LaneKey,
        files: &[cys::repair::apply::CandidateFile],
    ) -> Result<cys::repair::apply::ApplyOutcome, RepairError>;
    /// Step 3: the rehearsal re-run and the protection screen (`Err` = returned to the worker).
    fn step3(&self, s3: &Step3Input) -> Result<Step3Ok, String>;
    /// Files one intake row (a lane finding of the screen).
    fn intake(&self, file: &str, error: &str);
}

/// What step 3 is given.
#[derive(Debug, Clone)]
pub(crate) struct Step3Input {
    pub root: PathBuf,
    pub repair: RepairId,
    pub pack: PathBuf,
    pub states: Vec<(String, TargetState)>,
    pub windows: bool,
}

/// What a passing step 3 found.
#[derive(Debug, Clone, Default)]
pub(crate) struct Step3Ok {
    pub rehearsal_summary: String,
    pub screen_notes: Vec<String>,
    pub lane_findings: Vec<(String, String)>,
}

fn precondition(msg: impl Into<String>) -> VerbError {
    VerbError::new(ERR_PRECONDITION, msg)
}

fn repair_id(p: &Value) -> Result<RepairId, VerbError> {
    let s = p
        .get("repair")
        .and_then(|v| v.as_str())
        .ok_or_else(|| VerbError::invalid("`repair` 가 필요합니다"))?;
    RepairId::parse(s).map_err(|e| VerbError::invalid(VerbError::from(e).message))
}

fn load(ctx: &Ctx<'_>, id: &RepairId) -> Result<RepairRecord, VerbError> {
    match ledger::read_record_in(&ctx.root, id) {
        Ok(Some(r)) => Ok(r),
        Ok(None) => Err(VerbError::not_found(format!(
            "{}: 그런 수리가 없습니다",
            id.as_str()
        ))),
        Err(e) => Err(VerbError::new(
            cys::repair::types::ERR_CORRUPT,
            format!(
                "{}: 장부 기록을 읽을 수 없습니다 — {}",
                id.as_str(),
                VerbError::from(e).message
            ),
        )),
    }
}

fn write(ctx: &Ctx<'_>, rec: &RepairRecord) -> Result<(), VerbError> {
    ledger::write_record_durable_in(&ctx.root, rec).map_err(VerbError::from)
}

fn is_approval_class(c: Classification) -> bool {
    matches!(
        c,
        Classification::BootCritical | Classification::ProtectionRelated
    )
}

/// The worker's text, filtered for the card: invisible / bidi / control characters removed (the
/// team-proposal precedent, `team_spec.rs` — a hidden instruction must not look like a fact), length
/// capped.
pub(crate) fn filter_worker_text(s: &str) -> String {
    const INVISIBLE: &[(char, char)] = &[
        ('\u{200B}', '\u{200F}'),
        ('\u{2028}', '\u{2029}'),
        ('\u{202A}', '\u{202E}'),
        ('\u{2060}', '\u{2064}'),
        ('\u{2066}', '\u{2069}'),
        ('\u{FEFF}', '\u{FEFF}'),
    ];
    let mut out: String = s
        .chars()
        .filter(|c| !INVISIBLE.iter().any(|(a, b)| c >= a && c <= b))
        .map(|c| if c.is_control() && c != '\n' { ' ' } else { c })
        .collect();
    if out.chars().count() > WORKER_TEXT_MAX {
        out = out.chars().take(WORKER_TEXT_MAX).collect::<String>() + "…";
    }
    out
}

/// The verification prompt's reason text (B8): Korean, naming the repair, its targets and the first
/// 12 hex of the fingerprint.
pub(crate) fn reason_text(r: &Request) -> String {
    let fp12: String = r.fingerprint_sha256.chars().take(12).collect();
    let mut targets = r.targets.join(", ");
    if targets.chars().count() > 160 {
        targets = targets.chars().take(160).collect::<String>() + "…";
    }
    format!(
        "「스스로 해결」 수리 {} 최종 승인 — 대상: {targets} · 지문 {fp12}",
        r.repair
    )
}

fn new_request_id() -> String {
    format!("{REQUEST_PREFIX}{}", cys::repair::replay::random_hex(8))
}

/// The count of requests for `fp` that ended `expired` (RULINGS S1, U4(c)).
fn expired_count(rec: &RepairRecord, fp: &str) -> u32 {
    rec.resubmits
        .iter()
        .find(|(f, _)| f == fp)
        .map(|(_, n)| *n)
        .unwrap_or(0)
}

fn bump_expired(rec: &mut RepairRecord, fp: &str) -> u32 {
    if let Some(e) = rec.resubmits.iter_mut().find(|(f, _)| f == fp) {
        e.1 = e.1.saturating_add(1);
        return e.1;
    }
    rec.resubmits.push((fp.to_string(), 1));
    1
}

/// The one line the lane's master gives the owner when a candidate's budget is used up (D:762).
pub(crate) fn budget_line(repair: &str, lane: &str) -> String {
    format!(
        "수리 {repair} 의 최종 승인 요청이 네 번 만료되어 더는 다시 제출하지 않습니다(후보는 작업 사본에 남아 있습니다). 주인님께 한 줄로 알려 주세요 — 부서 '{lane}' 의 승인 탭에서 다음 요청을 승인하시거나, 워커에게 다시 맡기실 수 있습니다."
    )
}

// ═══════════════════════════════════════════ submit ═══════════════════════════════════════════

/// `repair.submit {repair}` — the worker holding the lock submits the staged candidate (design §4-7
/// step 2). Returns its request number at once; step 3 runs afterwards (`run_step3`).
pub(crate) fn submit(
    ctx: &Ctx<'_>,
    caller: &Caller,
    p: &Value,
    table: &Mutex<ApprovalTable>,
    platform_enabled: Result<(), &'static str>,
) -> Result<(Value, Option<Step3Input>), VerbError> {
    let id = repair_id(p)?;
    let sid = caller
        .seat()
        .ok_or_else(|| VerbError::refused("제출은 수리 잠금을 쥔 워커 좌석만 합니다"))?;
    ctx.locked(id.as_str(), || {
        let mut rec = load(ctx, &id)?;
        if rec.origin_lane != ctx.lane {
            return Err(precondition(format!(
                "{} 은 부서 {} 의 수리입니다 — 그 부서의 데몬에 제출하세요",
                id.as_str(),
                rec.origin_lane.0
            )));
        }
        if !is_approval_class(rec.classification) {
            return Err(precondition(format!(
                "{}: '{}' 수리는 최종 승인 대상이 아닙니다 — `cys repair commit` 으로 확정하세요",
                id.as_str(),
                rec.classification.label_ko()
            )));
        }
        // C12 / AT6: Windows (and a macOS build whose gate is not enabled) answers pending — no
        // screen, no helper, nothing offered
        if let Err(why) = platform_enabled {
            return Err(VerbError::new(
                cys::repair::types::ERR_PLATFORM_PENDING,
                format!("{}: 이 컴퓨터에서는 기동 필수·보호장치 수리의 최종 승인을 아직 받지 않습니다(대기) — {why}", id.as_str()),
            ));
        }
        // the lock: this seat, this lane, this daemon incarnation, lease not run out
        let holds = rec.lock.as_ref().is_some_and(|l| {
            l.owner.lane == ctx.lane
                && l.owner.seat == sid
                && l.owner.incarnation == ctx.incarnation
                && l.expires_at > ctx.now
        });
        if !holds {
            return Err(VerbError::new(
                cys::repair::types::ERR_LOCKED,
                format!("{}: 이 수리의 잠금을 쥔 워커만 제출합니다", id.as_str()),
            ));
        }
        let mut tbl = table.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(live) = tbl.live_for(id.as_str()) {
            return Err(precondition(format!(
                "{}: 이미 요청 {} 이(가) '{}' 상태입니다",
                id.as_str(),
                live.request_id,
                live.state.label_ko()
            )));
        }
        match rec.state {
            RepairState::InProgress | RepairState::AwaitingApproval => {}
            other => {
                return Err(precondition(format!(
                    "{}: '{}' 상태에서는 제출하지 않습니다",
                    id.as_str(),
                    other.label_ko()
                )))
            }
        }
        let worker = rec.worker_record.clone().ok_or_else(|| {
            precondition(format!("{}: 워커의 기록(`record`)이 아직 없습니다", id.as_str()))
        })?;
        // U2 / V2(a): the candidate is read through the ONE reader and fingerprinted over states
        let states = ledger::candidate_states_in(&ctx.root, &id).map_err(VerbError::from)?;
        if states.is_empty() {
            return Err(precondition(format!("{}: 제출할 파일 대상이 없습니다", id.as_str())));
        }
        let fp = cys::repair::fingerprint(&states);
        // N1: both reviewers' signatures over exactly these states
        let signed = |a: &Option<ledger::Attestation>, v: &str| {
            a.as_ref().is_some_and(|a| a.verdict == v && a.fingerprint_sha256 == fp)
        };
        if worker.fingerprint_sha256 != fp || !signed(&rec.verify, "pass") || !signed(&rec.audit, "ok") {
            return Err(VerbError::new(
                "target_changed",
                format!(
                    "{}: 작업 사본이 리뷰어1의 검증 서명·리뷰어2의 감사 서명이 본 후보와 다릅니다(또는 서명이 없습니다) — 서명은 무효입니다. `record` 뒤에 두 서명을 다시 받고 제출하세요",
                    id.as_str()
                ),
            ));
        }
        // S1 / U4(c): at most 3 resubmissions per candidate (requests ended `expired` count)
        let used = expired_count(&rec, &fp);
        if used > MAX_RESUBMITS {
            return Err(precondition(format!(
                "{}: 이 후보는 최종 승인 요청이 {} 번 만료되어 더 다시 제출하지 않습니다(한도 {}번). 후보는 작업 사본에 남아 있습니다",
                id.as_str(),
                used,
                MAX_RESUBMITS
            )));
        }
        let created_at = ctx.now;
        let expires_at = created_at + APPROVAL_TTL_SECS as f64;
        let req = Request {
            request_id: new_request_id(),
            repair: id.as_str().to_string(),
            lane_key: ctx.lane.0.clone(),
            worker_seat: sid,
            fingerprint_sha256: fp.clone(),
            targets: states.iter().map(|(r, _)| r.clone()).collect(),
            nonce: cys::repair::replay::random_hex(16),
            created_at,
            expires_at,
            resubmits_used: used,
            state: ReqState::Checking,
            rehearsal: None,
            prerehearsal_possible: states
                .iter()
                .all(|(r, _)| cys::repair::rehearsal::prerehearsal_possible(r)),
            helper_pid: None,
            classification: rec.classification,
            screen_notes: Vec::new(),
            reason: None,
            worker_text: filter_worker_text(&worker.note),
            ended_at: None,
        };
        // V2(d): the request's fingerprint and expiry are written into the record (durably) before
        // the number is given out
        rec.pending_request = Some((fp.clone(), expires_at));
        rec.state = RepairState::AwaitingApproval;
        write(ctx, &rec)?;
        let out = json!({
            "repair": id.as_str(),
            "request_id": req.request_id,
            "state": req.state,
            "state_label": req.state.label_ko(),
            "fingerprint_sha256": fp,
            "expires_at": expires_at,
            "resubmits_used": used,
            "note": "제출을 받았습니다 — 데몬이 같은 예행 검사와 보호장치 심사를 다시 돌린 뒤, 통과하면 주인의 승인 탭에 카드가 뜹니다. 결과는 한 번 이 좌석으로 옵니다(기다리지 마세요)."
        });
        let s3 = Step3Input {
            root: ctx.root.clone(),
            repair: id.clone(),
            pack: ctx.pack_dir.clone(),
            states,
            windows: ctx.windows,
        };
        tbl.requests.push(req);
        Ok((out, Some(s3)))
    })
}

/// Ends a live request: the record's pending request cleared, the state set, the result delivered
/// once, the event published. `expired` = counted toward the resubmission budget.
fn end_request(
    ctx: &Ctx<'_>,
    table: &Mutex<ApprovalTable>,
    eff: &dyn Effects,
    request_id: &str,
    to: ReqState,
    reason: String,
    back_to: RepairState,
    approval: Option<ApprovalResult>,
) -> Result<(), VerbError> {
    let (repair, seat, fp) = {
        let tbl = table.lock().unwrap_or_else(|e| e.into_inner());
        match tbl.find(request_id) {
            Some(r) if r.state.live() => (
                r.repair.clone(),
                r.worker_seat,
                r.fingerprint_sha256.clone(),
            ),
            _ => return Ok(()),
        }
    };
    let id = RepairId::parse(&repair).map_err(VerbError::from)?;
    let mut count = None;
    ctx.locked(id.as_str(), || -> Result<(), VerbError> {
        let mut rec = load(ctx, &id)?;
        if rec.pending_request.as_ref().is_some_and(|(f, _)| *f == fp) {
            rec.pending_request = None;
        }
        if to == ReqState::Expired {
            count = Some(bump_expired(&mut rec, &fp));
        }
        if rec.state == RepairState::AwaitingApproval {
            rec.state = back_to;
        }
        if let Some(a) = approval.clone() {
            rec.approval = Some(a);
        }
        write(ctx, &rec)
    })?;
    {
        let mut tbl = table.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(r) = tbl.find_mut(request_id) {
            r.state = to;
            r.reason = Some(reason.clone());
            r.ended_at = Some(ctx.now);
            r.helper_pid = None;
        }
    }
    eff.publish(
        "repair.approval.resolved",
        json!({"request_id": request_id, "repair": repair, "state": to, "reason": reason}),
    );
    eff.deliver(
        seat,
        format!(
            "[수리 {repair}] 최종 승인 요청 {request_id}: {} — {reason}",
            to.label_ko()
        ),
    );
    // the budget is used up exactly once: at the expiry that makes the count exceed the limit
    if count == Some(MAX_RESUBMITS + 1) {
        eff.tell_master(budget_line(&repair, &ctx.lane.0));
    }
    Ok(())
}

// ═══════════════════════════════════════════ step 3 ═══════════════════════════════════════════

/// Step 3 (design §4-7 step 3, AU5): the same deterministic rehearsal checks and the protection
/// screen, run after `submit` has returned. A failure returns the candidate to the worker — no card.
pub(crate) fn run_step3(
    ctx: &Ctx<'_>,
    table: &Mutex<ApprovalTable>,
    eff: &dyn Effects,
    request_id: &str,
    s3: &Step3Input,
) {
    let outcome = eff.step3(s3);
    let still_checking = {
        let tbl = table.lock().unwrap_or_else(|e| e.into_inner());
        tbl.find(request_id)
            .is_some_and(|r| r.state == ReqState::Checking)
    };
    if !still_checking {
        return;
    }
    match outcome {
        Err(why) => {
            let _ = end_request(
                ctx,
                table,
                eff,
                request_id,
                ReqState::Voided,
                format!("제출 검사에서 돌려보냈습니다(주인에게 올리지 않았습니다): {why}"),
                RepairState::InProgress,
                None,
            );
        }
        Ok(ok) => {
            for (file, line) in &ok.lane_findings {
                eff.intake(file, line);
            }
            let card = {
                let mut tbl = table.lock().unwrap_or_else(|e| e.into_inner());
                match tbl.find_mut(request_id) {
                    Some(r) => {
                        r.state = ReqState::Pending;
                        r.rehearsal = Some((true, ok.rehearsal_summary.clone()));
                        r.screen_notes = ok.screen_notes.clone();
                        Some(r.clone())
                    }
                    None => None,
                }
            };
            if let Some(r) = card {
                eff.publish(
                    "repair.approval.created",
                    json!({"request_id": r.request_id, "repair": r.repair, "targets": r.targets, "expires_at": r.expires_at}),
                );
            }
        }
    }
}

/// The production step 3: every FILE target checked by the rehearsal runner in a scratch copy of
/// the lane's pack with the candidate laid in, then the protection screen (both under the replay
/// rules). The screen's verdict is recorded (`screen.json`) when it accepts.
pub(crate) fn production_step3(s3: &Step3Input) -> Result<Step3Ok, String> {
    use cys::repair::rehearsal::{self, CheckRequest, CheckResult};
    use cys::repair::replay::{self, ScratchRoot};
    use cys::repair::screen::{self, ScreenCandidate, ScreenVerdict};
    if s3.windows {
        return Err("윈도우에서는 1차 출시에서 이 검사를 하지 않습니다".into());
    }
    let started = std::time::Instant::now();
    let staging = ledger::staging_dir_in(&s3.root, &s3.repair);
    // (a) the rehearsal: a scratch copy of the lane's pack with the candidate's states laid in
    let scratch =
        ScratchRoot::new(&rehearsal::scratch_base(), "s3-").map_err(|e| format!("{e:?}"))?;
    let layout = scratch.path().join("pack");
    rehearsal::copy_layout(&s3.pack, &layout)
        .map_err(|e| format!("예행 사본을 만들지 못했습니다 — {e:?}"))?;
    for (rel, st) in &s3.states {
        let dst = layout.join(rel);
        replay::remove_tree(&dst);
        let src = match st {
            TargetState::Absent => replay::LaidSource::Absent,
            TargetState::Link { target } => replay::LaidSource::Link(target.clone()),
            TargetState::File { .. } => {
                replay::LaidSource::read(&staging.join(rel)).map_err(|e| format!("{rel}: {e:?}"))?
            }
        };
        if src.state() != *st {
            return Err(format!("{rel}: 작업 사본이 제출 뒤에 바뀌었습니다"));
        }
        replay::lay_out(&dst, &src).map_err(|e| format!("{rel}: {e:?}"))?;
    }
    let mut summary = Vec::new();
    for (rel, st) in &s3.states {
        if *st == TargetState::Absent {
            summary.push(format!("{rel}: 삭제(검사 대상 없음)"));
            continue;
        }
        let left = STEP3_DEADLINE.saturating_sub(started.elapsed());
        let declared =
            cys::repair::boot_critical::boot_critical_entry(rel).and_then(|e| e.timeout_s);
        let own = Duration::from_secs(u64::from(cys::repair::boot_critical::clamp_check_timeout(
            declared,
        )));
        let r = rehearsal::run_check(&CheckRequest {
            entry_rel: rel.clone(),
            scratch_root: layout.clone(),
            recorded_input: None,
            deadline: own.min(left),
        })
        .map_err(|e| format!("{rel}: 예행 검사를 돌리지 못했습니다 — {e:?}"))?;
        match r {
            CheckResult::Pass => summary.push(if rehearsal::prerehearsal_possible(rel) {
                format!("{rel}: 통과")
            } else {
                format!("{rel}: {}", rehearsal::NOT_REHEARSABLE_KO)
            }),
            CheckResult::Fail(m) => return Err(format!("예행 검사 실패 — {m}")),
            CheckResult::Timeout => return Err(format!("{rel}: 예행 검사 시간 초과")),
        }
    }
    drop(scratch);
    // (b) the protection screen (C4 — every boot-critical and protection-related submit, AT2)
    let cand = ScreenCandidate {
        files: staging.clone(),
        targets: s3.states.clone(),
    };
    let left = STEP3_DEADLINE.saturating_sub(started.elapsed());
    let rep = screen::screen_protections(&cand, &s3.pack, left)
        .map_err(|e| format!("보호장치 심사를 끝내지 못했습니다 — {e:?}"))?;
    if rep.verdict == ScreenVerdict::Refused {
        return Err(format!("보호장치 심사 거부 — {}", rep.refusals.join(" / ")));
    }
    let verdict = screen::RecordedVerdict {
        schema: screen::SCREEN_SCHEMA,
        verdict: rep.verdict,
        input_fingerprint: rep.input_fingerprint.clone(),
        build_id: rep.build_id.clone(),
        notes: rep.notes.clone(),
        at: cys::repair::ledger::now_epoch(),
    };
    screen::write_verdict_in(&s3.root, &s3.repair, &verdict)
        .map_err(|e| format!("심사 결과를 기록하지 못했습니다 — {e:?}"))?;
    summary.push(format!("보호장치 심사 {}ms", rep.duration_ms));
    let device_of = |line: &str| -> String {
        if line.contains("guard.sh") {
            "hooks/guard.sh".into()
        } else if line.contains("C28") {
            "bin/javis_preflight.py".into()
        } else {
            "hooks/role-capability-gate.sh".into()
        }
    };
    Ok(Step3Ok {
        rehearsal_summary: summary.join(" · "),
        screen_notes: rep.notes.clone(),
        lane_findings: rep
            .lane_findings
            .iter()
            .map(|l| (device_of(l), l.clone()))
            .collect(),
    })
}

// ═══════════════════════════════════════ the owner's card ═══════════════════════════════════════

/// The card of a request: the daemon's facts APART from the worker's text (E2).
fn card(r: &Request, other_lanes: &[String]) -> Value {
    json!({
        "request_id": r.request_id,
        "repair": r.repair,
        "state": r.state,
        "state_label": r.state.label_ko(),
        "created_at": r.created_at,
        "expires_at": r.expires_at,
        "facts": {
            "classification": r.classification,
            "classification_label": r.classification.label_ko(),
            "targets": r.targets,
            "fingerprint_sha256": r.fingerprint_sha256,
            "rehearsal": r.rehearsal.as_ref().map(|(ok, s)| json!({"ok": ok, "summary": s})),
            "prerehearsal_possible": r.prerehearsal_possible,
            "prerehearsal_note": if r.prerehearsal_possible { Value::Null } else { json!("사전 예행 불가 — 적용한 뒤 사후 검사로 확인합니다") },
            "on_failure": "적용 직후와 24시간 시험 기간에 다시 검사하고, 실패하면 데몬이 이 수리만 이전 상태로 자동으로 되돌립니다",
            "screen_notes": r.screen_notes,
            "resubmits_used": r.resubmits_used,
            "other_lanes": other_lanes,
            "identical_bytes_note": if other_lanes.is_empty() { Value::Null } else { json!("같은 바이트가, 이 파일이 수리 전과 같은 상태인 다른 등록 부서에도 그대로 적용됩니다") },
        },
        "worker_text": r.worker_text,
        "reason": r.reason,
    })
}

/// `repair.approval_list` — the cards of this lane's open requests (and the recently ended ones).
pub(crate) fn approval_list(ctx: &Ctx<'_>, table: &Mutex<ApprovalTable>) -> Value {
    let tbl = table.lock().unwrap_or_else(|e| e.into_inner());
    let mut open = Vec::new();
    let mut ended = Vec::new();
    for r in tbl.requests.iter().filter(|r| r.lane_key == ctx.lane.0) {
        // O1(a) / v24: a repair with `resolves` lists no other lane; otherwise the registered
        // lanes other than this one (identical bytes applied where the state equals the pre-image)
        let other = other_lanes_for(ctx, r);
        match r.state {
            ReqState::Pending | ReqState::Verifying => open.push(card(r, &other)),
            ReqState::Checking => {}
            _ => ended.push(card(r, &other)),
        }
    }
    json!({ "lane": ctx.lane.0, "requests": open, "ended": ended, "platform_enabled": cys::repair::platform::boot_critical_enabled() == cys::repair::platform::BootCriticalGate::Enabled })
}

fn other_lanes_for(ctx: &Ctx<'_>, r: &Request) -> Vec<String> {
    let Ok(id) = RepairId::parse(&r.repair) else {
        return Vec::new();
    };
    match ledger::read_record_in(&ctx.root, &id) {
        Ok(Some(rec)) if rec.resolves.is_none() => {
            match ledger::registered_lanes_from(&ctx.registry) {
                ledger::RegisteredLanes::Known(lanes) => lanes
                    .iter()
                    .map(|l| l.0.clone())
                    .filter(|l| *l != ctx.lane.0)
                    .collect(),
                ledger::RegisteredLanes::Unknown(_) => Vec::new(),
            }
        }
        _ => Vec::new(),
    }
}

/// `repair.approval_start {request_id}` — the owner's click. Starts the helper and returns AT ONCE
/// (the result arrives on the bus); a second click while verifying changes nothing.
pub(crate) fn approval_start(
    ctx: &Ctx<'_>,
    p: &Value,
    table: &Mutex<ApprovalTable>,
    eff: &Arc<dyn Effects>,
    spawn_waiter: &dyn Fn(String, String, std::process::Child),
) -> Result<Value, VerbError> {
    let rid = p
        .get("request_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| VerbError::invalid("`request_id` 가 필요합니다"))?
        .to_string();
    let (state, expired, reason, nonce) = {
        let tbl = table.lock().unwrap_or_else(|e| e.into_inner());
        let r = tbl.find(&rid).ok_or_else(|| {
            VerbError::not_found(format!(
                "{rid}: 그런 승인 요청이 없습니다(데몬이 다시 시작되면 대기 요청은 무효입니다)"
            ))
        })?;
        if r.lane_key != ctx.lane.0 {
            return Err(precondition(format!("{rid}: 이 부서의 요청이 아닙니다")));
        }
        (
            r.state,
            ctx.now >= r.expires_at,
            reason_text(r),
            r.nonce.clone(),
        )
    };
    match state {
        ReqState::Verifying => {
            return Ok(
                json!({"request_id": rid, "state": ReqState::Verifying, "note": "이미 본인 확인 중입니다"}),
            )
        }
        ReqState::Pending => {}
        other => {
            return Err(precondition(format!(
                "{rid}: '{}' 요청은 승인할 수 없습니다",
                other.label_ko()
            )))
        }
    }
    if expired {
        end_request(
            ctx,
            table,
            eff.as_ref(),
            &rid,
            ReqState::Expired,
            "30분이 지나 만료되었습니다".into(),
            RepairState::AwaitingApproval,
            None,
        )?;
        return Err(precondition(format!("{rid}: 30분이 지나 만료되었습니다")));
    }
    {
        let mut tbl = table.lock().unwrap_or_else(|e| e.into_inner());
        match tbl.find_mut(&rid) {
            Some(r) if r.state == ReqState::Pending => r.state = ReqState::Verifying,
            Some(r) if r.state == ReqState::Verifying => {
                return Ok(
                    json!({"request_id": rid, "state": ReqState::Verifying, "note": "이미 본인 확인 중입니다"}),
                )
            }
            _ => return Err(precondition(format!("{rid}: 승인할 수 없는 상태입니다"))),
        }
    }
    match eff.spawn_helper(&rid, &reason) {
        Ok(child) => {
            let pid = child.id();
            {
                let mut tbl = table.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(r) = tbl.find_mut(&rid) {
                    r.helper_pid = Some(pid);
                }
            }
            spawn_waiter(rid.clone(), nonce, child);
            Ok(
                json!({"request_id": rid, "state": ReqState::Verifying, "note": "운영체제 본인 확인 창을 띄웠습니다"}),
            )
        }
        Err(e) => {
            // spawn failure = not approved (5.9)
            end_request(
                ctx,
                table,
                eff.as_ref(),
                &rid,
                ReqState::Rejected,
                format!("본인 확인을 시작하지 못했습니다({e}) — 승인이 아닙니다"),
                RepairState::InProgress,
                None,
            )?;
            Err(precondition(format!(
                "{rid}: 본인 확인을 시작하지 못했습니다 — 승인이 아닙니다"
            )))
        }
    }
}

/// Waits for the helper (killed + reaped at [`HELPER_KILL_SECS`]); the exit status alone decides.
pub(crate) fn wait_helper(child: std::process::Child) -> bool {
    wait_helper_for(child, Duration::from_secs(HELPER_KILL_SECS))
}

/// [`wait_helper`] with an explicit limit: past it the child is killed and reaped — not approved.
pub(crate) fn wait_helper_for(mut child: std::process::Child, limit: Duration) -> bool {
    let started = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(st)) => return st.success(),
            Ok(None) => {}
            Err(_) => break,
        }
        if started.elapsed() >= limit {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = child.kill();
    let _ = child.wait();
    false
}

/// The helper's result consumed (design §4-7 steps 6–9; I3 validity by timestamp; verify → apply →
/// consume). `verified` = the helper exited 0.
pub(crate) fn finish_verification(
    ctx: &Ctx<'_>,
    table: &Mutex<ApprovalTable>,
    eff: &dyn Effects,
    rid: &str,
    nonce: &str,
    verified: bool,
) -> Result<(), VerbError> {
    // the clock of this consumption: the context's "now" advanced by the monotonic time spent here
    // (the apply's own expiry check below reads it again — I3 "again at apply")
    let started = std::time::Instant::now();
    let now = |ctx: &Ctx<'_>| ctx.now + started.elapsed().as_secs_f64();
    let req = {
        let mut tbl = table.lock().unwrap_or_else(|e| e.into_inner());
        match tbl.find_mut(rid) {
            // the one-time nonce of THIS request: a result carrying another nonce is no result, and
            // the nonce is spent here (used once — D:761)
            Some(r)
                if r.state == ReqState::Verifying && !r.nonce.is_empty() && r.nonce == nonce =>
            {
                let snapshot = r.clone();
                r.nonce.clear();
                snapshot
            }
            _ => return Ok(()),
        }
    };
    if !verified {
        return end_request(
            ctx,
            table,
            eff,
            rid,
            ReqState::Rejected,
            "운영체제 본인 확인이 끝나지 않았거나 실패했습니다 — 승인이 아닙니다".into(),
            RepairState::InProgress,
            Some(ApprovalResult {
                request_id: rid.into(),
                fingerprint_sha256: req.fingerprint_sha256.clone(),
                decided_at: ctx.now,
                approved: false,
            }),
        );
    }
    // I3: a result at or after the deadline is not approved, whatever the stored state says
    if ctx.now >= req.expires_at {
        return end_request(
            ctx,
            table,
            eff,
            rid,
            ReqState::Expired,
            "본인 확인 결과가 30분이 지난 뒤에 왔습니다 — 승인이 아닙니다".into(),
            RepairState::AwaitingApproval,
            None,
        );
    }
    let id = RepairId::parse(&req.repair).map_err(VerbError::from)?;
    // verify: the candidate re-compared just before the apply (after the card was shown)
    let now_states = ledger::candidate_states_in(&ctx.root, &id).map_err(VerbError::from)?;
    let same_targets = now_states
        .iter()
        .map(|(r, _)| r.clone())
        .collect::<Vec<_>>()
        == req.targets;
    if cys::repair::fingerprint(&now_states) != req.fingerprint_sha256 || !same_targets {
        return end_request(
            ctx,
            table,
            eff,
            rid,
            ReqState::Voided,
            "카드를 보인 뒤에 후보가 바뀌었습니다 — 승인이 아닙니다. 다시 서명을 받아 제출하세요"
                .into(),
            RepairState::InProgress,
            None,
        );
    }
    // F9: the approval result into the record (durably) BEFORE the apply
    let approval = ApprovalResult {
        request_id: rid.into(),
        fingerprint_sha256: req.fingerprint_sha256.clone(),
        decided_at: ctx.now,
        approved: true,
    };
    ctx.locked(id.as_str(), || -> Result<(), VerbError> {
        let mut rec = load(ctx, &id)?;
        rec.approval = Some(approval.clone());
        write(ctx, &rec)
    })?;
    {
        let mut tbl = table.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(r) = tbl.find_mut(rid) {
            r.state = ReqState::Approved; // consumed: never startable again
            r.ended_at = Some(ctx.now);
            r.helper_pid = None;
        }
    }
    // the apply re-checks the expiry itself too (I3 — "again at apply")
    if now(ctx) >= req.expires_at {
        return revert_approval(
            ctx,
            eff,
            &req,
            "적용 직전에 30분이 지났습니다 — 적용하지 않았습니다",
        );
    }
    let staging = ledger::staging_dir_in(&ctx.root, &id);
    let files: Vec<cys::repair::apply::CandidateFile> = now_states
        .iter()
        .filter_map(|(rel, st)| match st {
            TargetState::File { mode, .. } => Some(cys::repair::apply::CandidateFile {
                rel: rel.clone(),
                staged: staging.join(rel),
                mode: *mode,
            }),
            _ => None,
        })
        .collect();
    match eff.apply(&id, &ctx.lane, &files) {
        Ok(cys::repair::apply::ApplyOutcome::Applied) => {
            ctx.locked(id.as_str(), || -> Result<(), VerbError> {
                let mut rec = load(ctx, &id)?;
                rec.pending_request = None;
                write(ctx, &rec)
            })?;
            eff.publish(
                "repair.approval.resolved",
                json!({"request_id": rid, "repair": req.repair, "state": ReqState::Approved}),
            );
            eff.deliver(req.worker_seat, format!("[수리 {}] 최종 승인 요청 {rid}: 주인이 승인했고 데몬이 적용했습니다(시험 기간 재검사가 이어집니다)", req.repair));
            Ok(())
        }
        Ok(cys::repair::apply::ApplyOutcome::Declined(why))
        | Ok(cys::repair::apply::ApplyOutcome::Superseded(why)) => revert_approval(
            ctx,
            eff,
            &req,
            &format!("승인되었지만 적용이 거절되었습니다 — {why}"),
        ),
        Err(e) => revert_approval(
            ctx,
            eff,
            &req,
            &format!(
                "승인되었지만 적용하지 못했습니다 — {}",
                VerbError::from(e).message
            ),
        ),
    }
}

/// A declined apply: in the origin the approval is void and the repair returns to `진행 중` (AY5).
fn revert_approval(
    ctx: &Ctx<'_>,
    eff: &dyn Effects,
    req: &Request,
    why: &str,
) -> Result<(), VerbError> {
    let id = RepairId::parse(&req.repair).map_err(VerbError::from)?;
    ctx.locked(id.as_str(), || -> Result<(), VerbError> {
        let mut rec = load(ctx, &id)?;
        rec.approval = None;
        rec.pending_request = None;
        if rec.state == RepairState::AwaitingApproval {
            rec.state = RepairState::InProgress;
        }
        write(ctx, &rec)
    })?;
    eff.publish("repair.approval.resolved", json!({"request_id": req.request_id, "repair": req.repair, "state": ReqState::Voided, "reason": why}));
    eff.deliver(
        req.worker_seat,
        format!(
            "[수리 {}] 최종 승인 요청 {}: {why}",
            req.repair, req.request_id
        ),
    );
    Ok(())
}

/// `repair.approval_reject {request_id}` — the owner says no.
pub(crate) fn approval_reject(
    ctx: &Ctx<'_>,
    p: &Value,
    table: &Mutex<ApprovalTable>,
    eff: &dyn Effects,
) -> Result<Value, VerbError> {
    let rid = p
        .get("request_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| VerbError::invalid("`request_id` 가 필요합니다"))?
        .to_string();
    let (fp, state) = {
        let tbl = table.lock().unwrap_or_else(|e| e.into_inner());
        let r = tbl
            .find(&rid)
            .ok_or_else(|| VerbError::not_found(format!("{rid}: 그런 승인 요청이 없습니다")))?;
        if r.lane_key != ctx.lane.0 {
            return Err(precondition(format!("{rid}: 이 부서의 요청이 아닙니다")));
        }
        (r.fingerprint_sha256.clone(), r.state)
    };
    if state != ReqState::Pending {
        return Err(precondition(format!(
            "{rid}: '{}' 요청은 거절할 수 없습니다",
            state.label_ko()
        )));
    }
    end_request(
        ctx,
        table,
        eff,
        &rid,
        ReqState::Rejected,
        "주인이 거절했습니다".into(),
        RepairState::InProgress,
        Some(ApprovalResult {
            request_id: rid.clone(),
            fingerprint_sha256: fp,
            decided_at: ctx.now,
            approved: false,
        }),
    )?;
    Ok(json!({"request_id": rid, "state": ReqState::Rejected}))
}

/// Voids a live request of `repair` (its target was reverted by the daemon's automatic revert —
/// 5.11; the revert never waits for this request's lock). Told to the fixer once.
pub(crate) fn void_for_repair(
    ctx: &Ctx<'_>,
    table: &Mutex<ApprovalTable>,
    eff: &dyn Effects,
    repair: &str,
    why: &str,
) {
    let rid = {
        let tbl = table.lock().unwrap_or_else(|e| e.into_inner());
        tbl.live_for(repair).map(|r| r.request_id.clone())
    };
    if let Some(rid) = rid {
        let _ = end_request(
            ctx,
            table,
            eff,
            &rid,
            ReqState::Voided,
            why.to_string(),
            RepairState::InProgress,
            None,
        );
    }
}

// ═══════════════════════════════════════ the watchdog sweep ═══════════════════════════════════════

/// The expiry sweep inside the governance watchdog (5.14). Only updates what the card shows and the
/// budget; validity itself is by timestamp (I3). The first sweep of an incarnation resolves the
/// requests a previous incarnation left in this lane's records (V2(d)).
pub(crate) fn sweep(ctx: &Ctx<'_>, table: &Mutex<ApprovalTable>, eff: &dyn Effects) {
    let first = {
        let mut tbl = table.lock().unwrap_or_else(|e| e.into_inner());
        let f = !tbl.restart_resolved;
        tbl.restart_resolved = true;
        f
    };
    if first {
        resolve_after_restart(ctx, table, eff);
    }
    let due: Vec<String> = {
        let tbl = table.lock().unwrap_or_else(|e| e.into_inner());
        tbl.requests
            .iter()
            .filter(|r| {
                matches!(
                    r.state,
                    ReqState::Pending | ReqState::Verifying | ReqState::Checking
                ) && ctx.now >= r.expires_at
            })
            .map(|r| r.request_id.clone())
            .collect()
    };
    // 5.11: a live request whose repair is no longer 승인 대기 (the daemon reverted or superseded its
    // target) is void — the revert never waited for it; the fixer is told once
    let live: Vec<String> = {
        let tbl = table.lock().unwrap_or_else(|e| e.into_inner());
        tbl.requests
            .iter()
            .filter(|r| r.state.live() && !due.contains(&r.request_id))
            .map(|r| r.repair.clone())
            .collect()
    };
    for repair in live {
        let gone = RepairId::parse(&repair)
            .ok()
            .and_then(|id| ledger::read_record_in(&ctx.root, &id).ok().flatten())
            .is_some_and(|rec| rec.state != RepairState::AwaitingApproval);
        if gone {
            void_for_repair(
                ctx,
                table,
                eff,
                &repair,
                "대상이 데몬의 되돌리기 또는 새 수리로 바뀌어 이 요청은 무효입니다",
            );
        }
    }
    for rid in due {
        let _ = end_request(
            ctx,
            table,
            eff,
            &rid,
            ReqState::Expired,
            "30분이 지나 만료되었습니다 — 후보는 작업 사본에 남아 있고 다시 제출할 수 있습니다"
                .into(),
            RepairState::AwaitingApproval,
            None,
        );
    }
    let mut tbl = table.lock().unwrap_or_else(|e| e.into_inner());
    tbl.requests
        .retain(|r| r.state.live() || r.ended_at.map_or(true, |t| ctx.now - t < ENDED_KEEP_SECS));
}

/// A record of this lane still naming a pending request after a restart: expired if its time had
/// passed (counted), else voided by the restart (not counted — D-18-01). The lock is released by the
/// restart itself (lease incarnation).
fn resolve_after_restart(ctx: &Ctx<'_>, table: &Mutex<ApprovalTable>, eff: &dyn Effects) {
    for id in ledger::list_ids_in(&ctx.root) {
        let live = {
            let tbl = table.lock().unwrap_or_else(|e| e.into_inner());
            tbl.live_for(id.as_str()).is_some()
        };
        if live {
            continue;
        }
        let mut count = None;
        let res = ctx.locked(id.as_str(), || -> Result<bool, VerbError> {
            let Ok(Some(mut rec)) = ledger::read_record_in(&ctx.root, &id) else {
                return Ok(false);
            };
            if rec.origin_lane != ctx.lane {
                return Ok(false);
            }
            let Some((fp, expires_at)) = rec.pending_request.clone() else {
                return Ok(false);
            };
            rec.pending_request = None;
            if ctx.now >= expires_at {
                count = Some(bump_expired(&mut rec, &fp));
            }
            write(ctx, &rec)?;
            Ok(true)
        });
        if matches!(res, Ok(true)) && count == Some(MAX_RESUBMITS + 1) {
            eff.tell_master(budget_line(id.as_str(), &ctx.lane.0));
        }
    }
}

/// The approval part of `repair.status`: the requests of a repair (memory only).
pub(crate) fn status_part(table: &Mutex<ApprovalTable>, repair: Option<&str>) -> Value {
    let tbl = table.lock().unwrap_or_else(|e| e.into_inner());
    let rows: Vec<Value> = tbl
        .requests
        .iter()
        .filter(|r| repair.map_or(true, |x| r.repair == x))
        .map(|r| {
            json!({
                "request_id": r.request_id,
                "repair": r.repair,
                "state": r.state,
                "state_label": r.state.label_ko(),
                "expires_at": r.expires_at,
                "reason": r.reason,
                "rehearsal": r.rehearsal.as_ref().map(|(ok, s)| json!({"ok": ok, "summary": s})),
                "screen_notes": r.screen_notes,
            })
        })
        .collect();
    json!(rows)
}

// ═══════════════════════════════════════ production wiring ═══════════════════════════════════════

/// The daemon's effects.
pub(crate) struct DaemonEffects {
    pub daemon: Arc<Daemon>,
}

impl Effects for DaemonEffects {
    fn deliver(&self, seat: u64, text: String) {
        let d = self.daemon.clone();
        std::thread::spawn(move || deliver_once(&d, seat, text));
    }

    fn tell_master(&self, text: String) {
        let d = self.daemon.clone();
        std::thread::spawn(move || {
            let master = d
                .roles
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get("master")
                .copied();
            if let Some(sid) = master {
                deliver_once(&d, sid, text);
            }
        });
    }

    fn publish(&self, name: &str, payload: Value) {
        self.daemon.bus.publish(name, "repair", None, payload);
    }

    fn spawn_helper(&self, request_id: &str, reason: &str) -> std::io::Result<std::process::Child> {
        spawn_helper(request_id, reason)
    }

    fn apply(
        &self,
        id: &RepairId,
        lane: &LaneKey,
        files: &[cys::repair::apply::CandidateFile],
    ) -> Result<cys::repair::apply::ApplyOutcome, RepairError> {
        cys::repair::apply::apply_candidate(id, lane, files)
    }

    fn step3(&self, s3: &Step3Input) -> Result<Step3Ok, String> {
        production_step3(s3)
    }

    fn intake(&self, file: &str, error: &str) {
        crate::repair::file_intake(&self.daemon, file, error);
    }
}

/// One delivery to a seat's queue (O-C5-5: one attempt; a frozen or full queue is retried a few
/// times on this thread; a gone seat ends it — the result stays in `cys repair status`).
fn deliver_once(daemon: &Arc<Daemon>, seat: u64, text: String) {
    use crate::alert_route::{enqueue_into_seat, EnqueueErr, FreezeGuard};
    for attempt in 0..=10u32 {
        match enqueue_into_seat(
            daemon,
            seat,
            text.clone(),
            Some("repair".to_string()),
            "repair_result",
            100,
            None,
            FreezeGuard::Daemon,
            None,
        ) {
            Ok(_) => return,
            Err(EnqueueErr::Frozen) | Err(EnqueueErr::QueueFull) if attempt < 10 => {
                std::thread::sleep(Duration::from_secs(30));
            }
            Err(_) => return,
        }
    }
}

/// The helper spawn (5.15, O-C5-4): the hidden `cys repair auth-helper` beside the daemon, no
/// console window, stdin null, the seat-identity variables removed. ★In the unit tests of this
/// binary the real helper is NEVER spawned (it would put a Touch ID / password dialog on the screen
/// with nobody to answer) — tests use their own scripted child.
#[cfg(not(any(test, feature = "repair-test-auth")))]
pub(crate) fn spawn_helper(request_id: &str, reason: &str) -> std::io::Result<std::process::Child> {
    use cys::SpawnPolicy;
    // a bounded child the daemon keeps hold of (killed + reaped at 120 s); on Windows no console
    // window; the rival-daemon autostart sealed (a `cys` child never starts a daemon — flood ①)
    std::process::Command::new(crate::state::sibling_cli_path())
        .args([
            "repair",
            "auth-helper",
            "--request",
            request_id,
            "--reason",
            reason,
        ])
        .current_dir("/")
        .env_remove("CYS_SURFACE_ID")
        .env_remove("CYS_ROLE")
        .env_remove("JAVIS_SURFACE_ID")
        .env_remove("AITERM_SURFACE_ID")
        .env_remove("CYS_SURFACE_ROLE")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn_policy(cys::ChildLifetime::Attached)
        .no_autostart()
        .spawn()
}

/// TESTPLAN OQ-H-10 — the scripted double behind the cargo feature `repair-test-auth` (off by
/// default; no workflow enables it): it replaces ONLY the helper spawn. It reads its answer from the
/// file the rig names (`ok` → exit 0, `hang` → never answers, `crash` → killed by a signal, anything
/// else → exit 1). Request table, fingerprint, expiry, signatures and apply are the production code.
#[cfg(all(feature = "repair-test-auth", not(test)))]
pub(crate) fn spawn_helper(
    request_id: &str,
    _reason: &str,
) -> std::io::Result<std::process::Child> {
    use cys::SpawnPolicy;
    let rig = std::env::var("CYS_REPAIR_TEST_AUTH_ANSWER").unwrap_or_default();
    std::process::Command::new("/bin/sh")
        .args(["-c", TEST_AUTH_SCRIPT, "sh", &rig, request_id])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn_policy(cys::ChildLifetime::Attached)
        .spawn()
}

#[cfg(all(feature = "repair-test-auth", not(test)))]
const TEST_AUTH_SCRIPT: &str = "a=$(cat \"$1\" 2>/dev/null); case \"$a\" in ok) exit 0 ;; hang) exec sleep 600 ;; crash) kill -SEGV $$ ;; *) exit 1 ;; esac";

/// In this binary's unit tests the production spawn refuses (no dialog can ever appear).
#[cfg(test)]
pub(crate) fn spawn_helper(
    _request_id: &str,
    _reason: &str,
) -> std::io::Result<std::process::Child> {
    Err(std::io::Error::other(
        "the real helper never runs in a unit test",
    ))
}

/// Production: `repair.submit` — the verb, then step 3 on its own thread.
pub(crate) fn handle_submit(
    daemon: &Arc<Daemon>,
    ctx: &Ctx<'_>,
    caller: &Caller,
    p: &Value,
) -> Result<Value, VerbError> {
    let gate = match cys::repair::platform::boot_critical_enabled() {
        cys::repair::platform::BootCriticalGate::Enabled => Ok(()),
        cys::repair::platform::BootCriticalGate::Pending(why) => Err(why),
    };
    let (out, s3) = submit(ctx, caller, p, &daemon.repair_approvals, gate)?;
    if let (Some(s3), Some(rid)) = (
        s3,
        out.get("request_id")
            .and_then(|v| v.as_str())
            .map(str::to_string),
    ) {
        let d = daemon.clone();
        std::thread::spawn(move || {
            let eff = DaemonEffects { daemon: d.clone() };
            crate::repair::with_daemon_ctx(&d, |ctx| {
                run_step3(ctx, &d.repair_approvals, &eff, &rid, &s3)
            });
        });
    }
    Ok(out)
}

/// Production: `repair.approval_start` — the helper waits on its own thread.
pub(crate) fn handle_start(
    daemon: &Arc<Daemon>,
    ctx: &Ctx<'_>,
    p: &Value,
) -> Result<Value, VerbError> {
    let eff: Arc<dyn Effects> = Arc::new(DaemonEffects {
        daemon: daemon.clone(),
    });
    let d = daemon.clone();
    let waiter = move |rid: String, nonce: String, child: std::process::Child| {
        let d = d.clone();
        std::thread::spawn(move || {
            let verified = wait_helper(child);
            let eff = DaemonEffects { daemon: d.clone() };
            crate::repair::with_daemon_ctx(&d, |ctx| {
                let _ = finish_verification(ctx, &d.repair_approvals, &eff, &rid, &nonce, verified);
            });
        });
    };
    approval_start(ctx, p, &daemon.repair_approvals, &eff, &waiter)
}

/// Production: the watchdog's call (governance).
pub(crate) fn watchdog_sweep(daemon: &Arc<Daemon>) {
    let eff = DaemonEffects {
        daemon: daemon.clone(),
    };
    crate::repair::with_daemon_ctx(daemon, |ctx| sweep(ctx, &daemon.repair_approvals, &eff));
}

/// Routes the T5b verbs (`None` = not one of them). The role table comes first — T3's
/// `role_check`, deny by default: `submit` = a worker seat; `approval_start` / `approval_reject` =
/// no seat at all and the app's operator token (`OwnerApproval`); `approval_list` = a read.
pub(crate) fn route(
    daemon: &Arc<Daemon>,
    ctx: &Ctx<'_>,
    caller: &Caller,
    sub: &str,
    p: &Value,
) -> Option<Result<Value, VerbError>> {
    if !matches!(
        sub,
        "submit" | "approval_list" | "approval_start" | "approval_reject"
    ) {
        return None;
    }
    let r = crate::repair::role_check(sub, caller, ctx.windows).and_then(|_| match sub {
        "submit" => handle_submit(daemon, ctx, caller, p),
        "approval_list" => Ok(approval_list(ctx, &daemon.repair_approvals)),
        "approval_start" => handle_start(daemon, ctx, p),
        _ => approval_reject(
            ctx,
            p,
            &daemon.repair_approvals,
            &DaemonEffects {
                daemon: daemon.clone(),
            },
        ),
    });
    Some(r.map_err(|mut e| {
        if !e.message.starts_with("repair.") {
            e.message = format!("repair.{sub}: {}", e.message);
        }
        e
    }))
}

/// The approval part of `repair.status` (AU5: the worker reads a returned candidate's reason there).
pub(crate) fn augment_status(daemon: &Arc<Daemon>, sub: &str, p: &Value, mut v: Value) -> Value {
    if sub == "status" {
        let repair = p.get("repair").and_then(|x| x.as_str());
        if let Some(o) = v.as_object_mut() {
            o.insert(
                "approval_requests".into(),
                status_part(&daemon.repair_approvals, repair),
            );
        }
    }
    v
}
