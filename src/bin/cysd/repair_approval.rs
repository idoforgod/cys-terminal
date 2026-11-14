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
    /// Claimed by the verification that is applying it (RULINGS CZ2 / DE4) — ended only by it.
    Applying,
    Approved,
    Rejected,
    Expired,
    Voided,
}

impl ReqState {
    fn live(self) -> bool {
        matches!(
            self,
            ReqState::Checking | ReqState::Pending | ReqState::Verifying | ReqState::Applying
        )
    }

    fn label_ko(self) -> &'static str {
        match self {
            ReqState::Checking => "제출 검사 중",
            ReqState::Pending => "주인 승인 대기",
            ReqState::Verifying => "본인 확인 중",
            ReqState::Applying => "적용 중",
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
    /// RULINGS EL5: the repairs whose unknown approval was already reported (one finding each) —
    /// RULINGS GA8 (2): a cache, written only AFTER the ledger answered for the row.
    pub(crate) unknown_reported: std::collections::BTreeSet<String>,
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
    /// RULINGS GA8 (4): a POINTER to a new intake row for the lane master — ONE attempt, never
    /// retried; no master or a failed send ends it (the row is the record; `cys repair status` shows
    /// it — a retry loop towards an absent master would be the queue-flood class).
    fn point_master(&self, text: String);
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
    fn step3(&self, s3: &Step3Input) -> Result<Step3Ok, Step3Err>;
    /// Files one intake row (a lane finding of the screen, a comparison's finding) — the ledger's
    /// answer (RULINGS GA8).
    fn intake(&self, file: &str, error: &str) -> crate::repair::Filed;
    /// RULINGS EL5 / EP — the seam: was the approval `approval_ref` consumed by an apply?
    fn approval_consumed(
        &self,
        root: &std::path::Path,
        lane: &LaneKey,
        approval_ref: &str,
    ) -> cys::repair::consumed::ApprovalUse;
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

/// Why step 3 returned the candidate (no card).
#[derive(Debug, Clone, Default)]
pub(crate) struct Step3Err {
    pub why: String,
    /// RULINGS DE1 / BK1 / GA8: the intake rows step 3 WROTE (first, when found) for its comparisons —
    /// the refusal's ONE row and a noted difference's row; every later error carries them; the caller
    /// never files them again …
    pub intake: Vec<(String, String)>,
    /// … and the pointer for the lane master, present only when the ledger answered that the refusal's
    /// row is NEW (GA8 (2)) — sent once, one attempt.
    pub owner_line: Option<String>,
}

impl From<String> for Step3Err {
    fn from(why: String) -> Self {
        Self {
            why,
            ..Default::default()
        }
    }
}

impl From<&str> for Step3Err {
    fn from(why: &str) -> Self {
        why.to_string().into()
    }
}

impl Step3Err {
    /// A refusal by the scoped after-run comparison (the rehearsal's or the screen's), whose ONE
    /// intake row naming the candidate's first target was already written (`filed` = the ledger's
    /// answer): the reason, that row, and the master's pointer only for a NEW row.
    fn escaped(repair: &RepairId, rel: &str, line: String, filed: &crate::repair::Filed) -> Self {
        Self {
            owner_line: (*filed == crate::repair::Filed::New).then(|| {
                format!(
                    "수리 {} 의 제출 검사에서 예행이 실제 환경을 건드려 후보를 돌려보냈습니다 — 접수 장부({rel})에 남겼습니다(`cys repair status` · `cys repair list`)",
                    repair.as_str()
                )
            }),
            intake: vec![(rel.to_string(), line.clone())],
            why: line,
        }
    }
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

#[cfg(test)]
thread_local! {
    /// Test seam of RULINGS CZ2's interleaving tests: runs ONCE at this thread's next record write —
    /// the write that follows a claim, inside the repair lock.
    pub(crate) static BEFORE_RECORD_WRITE: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        std::cell::RefCell::new(None);
    /// Test seam of RULINGS GR5: runs ONCE right before this thread's next candidate read in
    /// `submit` — a reader made slow by a test double (no real time).
    pub(crate) static BEFORE_CANDIDATE_READ: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        std::cell::RefCell::new(None);
}

fn write(ctx: &Ctx<'_>, rec: &RepairRecord) -> Result<(), VerbError> {
    #[cfg(test)]
    if let Some(f) = BEFORE_RECORD_WRITE.with(|h| h.borrow_mut().take()) {
        f();
    }
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
    eff: &dyn Effects,
    platform_enabled: Result<(), &'static str>,
) -> Result<(Value, Option<Step3Input>), VerbError> {
    let id = repair_id(p)?;
    let sid = caller
        .seat()
        .ok_or_else(|| VerbError::refused("제출은 수리 잠금을 쥔 워커 좌석만 합니다"))?;
    // RULINGS EL5: a finding is filed after every lock is released (the report verb takes locks)
    let mut found: Option<(String, String)> = None;
    let finding = &mut found;
    let out = ctx.locked(id.as_str(), || {
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
        // U2 / V2(a): the candidate is read through the ONE reader (RULINGS GR5: with a size limit) —
        // BEFORE the approval table is taken, so no read of a worker's file ever holds the table that
        // the approval list and the sweep need (only this repair's own lock is held)
        #[cfg(test)]
        if let Some(f) = BEFORE_CANDIDATE_READ.with(|h| h.borrow_mut().take()) {
            f();
        }
        let states = ledger::candidate_states_in(&ctx.root, &id).map_err(VerbError::from)?;
        let mut tbl = table.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(live) = tbl.live_for(id.as_str()) {
            return Err(precondition(format!(
                "{}: 이미 요청 {} 이(가) '{}' 상태입니다",
                id.as_str(),
                live.request_id,
                live.state.label_ko()
            )));
        }
        // CZ2: a previous durable pending request no request of this table serves (a restart not yet
        // reconciled by the first sweep, an end not written) is settled BEFORE it is replaced — its
        // expiry counted once, never lost
        let consumed = |r: &str| eff.approval_consumed(&ctx.root, &ctx.lane, r);
        match reconcile_stale_pending(ctx.now, &tbl, id.as_str(), &mut rec, &consumed) {
            Reconciled::Nothing => {}
            Reconciled::Settled(count) => {
                write(ctx, &rec)?;
                if count == Some(MAX_RESUBMITS + 1) {
                    eff.tell_master(budget_line(id.as_str(), &ctx.lane.0));
                }
            }
            Reconciled::Unknown(why) => {
                *finding = unknown_finding(&mut tbl, id.as_str(), &rec, &why);
                return Err(precondition(format!(
                    "{}: 이전 승인이 적용에 쓰였는지 알 수 없어 다시 제출을 받지 않습니다({why}) — 접수 줄에 남겼습니다",
                    id.as_str()
                )));
            }
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
        // … and fingerprinted over its states
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
    });
    if let Some((file, line)) = found {
        report_unknown(table, eff, id.as_str(), &file, &line);
    }
    out
}

/// RULINGS EL5 / GA8 (2): files the unknown-approval finding — the repair is remembered as reported
/// only AFTER the ledger answered for the row (a failed write marks nothing; the next sweep or
/// submit files it again).
fn report_unknown(table: &Mutex<ApprovalTable>, eff: &dyn Effects, repair: &str, file: &str, line: &str) {
    if eff.intake(file, line).settled() {
        table
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .unknown_reported
            .insert(repair.to_string());
    }
}

/// RULINGS CZ2: is the record's durable pending request THIS request (identity = fingerprint +
/// expiry, both written at `submit`)?
fn pending_is(rec: &RepairRecord, r: &Request) -> bool {
    rec.pending_request
        .as_ref()
        .is_some_and(|(f, e)| *f == r.fingerprint_sha256 && *e == r.expires_at)
}

/// What an end does to the record's approval result.
#[derive(Clone)]
enum RecApproval {
    Keep,
    Set(ApprovalResult),
    Clear,
}

/// RULINGS CZ2 — THE terminal claim. Under the table lock a request in one of `from` takes `to` at
/// once (reason, end time, the one-time nonce spent, the helper forgotten) and its state before the
/// claim is returned. Of any number of racing enders exactly ONE gets `Some`. Every caller holds the
/// repair's lock around the claim and the durable update that follows, so the pair is atomic against
/// `submit` and the restart reconciliation (both take the same lock).
fn claim(
    table: &Mutex<ApprovalTable>,
    rid: &str,
    from: &[ReqState],
    to: ReqState,
    reason: &str,
    now: f64,
) -> Option<Request> {
    let mut tbl = table.lock().unwrap_or_else(|e| e.into_inner());
    let r = tbl.find_mut(rid).filter(|r| from.contains(&r.state))?;
    let before = r.clone();
    r.state = to;
    r.nonce.clear();
    r.helper_pid = None;
    if !to.live() {
        r.reason = Some(reason.to_string());
        r.ended_at = Some(now);
    }
    Some(before)
}

/// The durable side of an end (the caller holds the repair lock and has claimed the request): the
/// pending request cleared only when it is THIS request's, the expiry counted then and only then
/// (so exactly once), the repair sent back, the approval result set / kept / cleared. Returns the new
/// expiry count when one was counted (and written).
fn settle_record(
    ctx: &Ctx<'_>,
    id: &RepairId,
    r: &Request,
    to: ReqState,
    back_to: RepairState,
    approval: &RecApproval,
) -> (Option<u32>, Result<(), VerbError>) {
    let mut rec = match load(ctx, id) {
        Ok(rec) => rec,
        Err(e) => return (None, Err(e)),
    };
    let mut count = None;
    if pending_is(&rec, r) {
        rec.pending_request = None;
        if to == ReqState::Expired {
            count = Some(bump_expired(&mut rec, &r.fingerprint_sha256));
        }
        if rec.state == RepairState::AwaitingApproval {
            rec.state = back_to;
        }
    }
    match approval {
        RecApproval::Keep => {}
        RecApproval::Set(a) => rec.approval = Some(a.clone()),
        RecApproval::Clear => rec.approval = None,
    }
    match write(ctx, &rec) {
        Ok(()) => (count, Ok(())),
        Err(e) => (None, Err(e)),
    }
}

/// The result of an end told ONCE — only the claimer of the request gets here: the bus event, the
/// one delivery to the fixer seat and, at the expiry that used the budget up, the lane master's one
/// line.
fn announce(ctx: &Ctx<'_>, eff: &dyn Effects, r: &Request, to: ReqState, reason: &str, count: Option<u32>) {
    eff.publish(
        "repair.approval.resolved",
        json!({"request_id": r.request_id, "repair": r.repair, "state": to, "reason": reason}),
    );
    eff.deliver(
        r.worker_seat,
        format!(
            "[수리 {}] 최종 승인 요청 {}: {} — {reason}",
            r.repair,
            r.request_id,
            to.label_ko()
        ),
    );
    // the budget is used up exactly once: at the expiry that makes the count exceed the limit
    if count == Some(MAX_RESUBMITS + 1) {
        eff.tell_master(budget_line(&r.repair, &ctx.lane.0));
    }
}

/// The states an ender (the owner's rejection, the expiry, a void, step 3's return, a failed start)
/// may claim a request from. `Applying` is owned by the verification that claimed it (CZ2 / DE4).
const ENDABLE: &[ReqState] = &[ReqState::Checking, ReqState::Pending, ReqState::Verifying];

/// Ends a request through the ONE terminal claim (RULINGS CZ2, [`claim`]): under the repair lock the
/// claim, then the record; outside every lock the one announcement. `Ok(false)` = the request was
/// not in one of `from` any more (another end came first) — nothing is written or told.
#[allow(clippy::too_many_arguments)]
fn end_request(
    ctx: &Ctx<'_>,
    table: &Mutex<ApprovalTable>,
    eff: &dyn Effects,
    request_id: &str,
    from: &[ReqState],
    to: ReqState,
    reason: String,
    back_to: RepairState,
    approval: RecApproval,
) -> Result<bool, VerbError> {
    let repair = {
        let tbl = table.lock().unwrap_or_else(|e| e.into_inner());
        match tbl.find(request_id) {
            Some(r) => r.repair.clone(),
            None => return Ok(false),
        }
    };
    let id = RepairId::parse(&repair).map_err(VerbError::from)?;
    let done = ctx.locked(id.as_str(), || {
        let r = claim(table, request_id, from, to, &reason, ctx.now)?;
        let (count, res) = settle_record(ctx, &id, &r, to, back_to, &approval);
        Some((r, count, res))
    });
    let Some((r, count, res)) = done else {
        return Ok(false);
    };
    announce(ctx, eff, &r, to, &reason, count);
    res.map(|()| true)
}

/// Ends a request this verification OWNS (state `Applying` — RULINGS CZ2 / DE4): the record first,
/// then the table's truthful terminal state, then the one announcement.
#[allow(clippy::too_many_arguments)]
fn settle_owned(
    ctx: &Ctx<'_>,
    table: &Mutex<ApprovalTable>,
    eff: &dyn Effects,
    r: &Request,
    to: ReqState,
    reason: String,
    back_to: RepairState,
    approval: RecApproval,
) -> Result<(), VerbError> {
    let id = RepairId::parse(&r.repair).map_err(VerbError::from)?;
    let (count, res) = ctx.locked(id.as_str(), || {
        let out = settle_record(ctx, &id, r, to, back_to, &approval);
        let mut tbl = table.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(x) = tbl
            .find_mut(&r.request_id)
            .filter(|x| x.state == ReqState::Applying)
        {
            x.state = to;
            x.reason = Some(reason.clone());
            x.ended_at = Some(ctx.now);
        }
        out
    });
    announce(ctx, eff, r, to, &reason, count);
    res
}

/// What the reconciliation of a stale pending request decided (RULINGS CZ2 / EL5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Reconciled {
    /// No stale pending request (none, or this table still serves it).
    Nothing,
    /// The record was changed (write it); the new expiry count when one was counted.
    Settled(Option<u32>),
    /// RULINGS EL5: whether the request's approval was consumed by an apply is UNKNOWN — the record
    /// is left untouched (the approval neither deleted nor usable); the caller files ONE finding.
    Unknown(String),
}

/// RULINGS CZ2 / EL5 — a durable pending request that no request of this table is serving (a previous
/// daemon's, or one whose end could not be written) is reconciled under the repair lock (the caller
/// holds it and the table lock) BEFORE anything replaces it. When the record holds an `approved`
/// approval of that request, whether an apply CONSUMED it is decided by the apply's durable evidence —
/// the seam `consumed` (`cys::repair::consumed::approval_consumed_in`, T6's), or this table's own
/// `Approved` end: CONSUMED → kept as applied (never deleted, never counted as an expiry, never usable
/// again — no request names it); NOT CONSUMED → the approval is cleared and the request reconciled;
/// UNKNOWN → nothing is touched (`Reconciled::Unknown`). The reconciliation itself: ended in this
/// table → settled as it ended (counted when it expired); unknown here (a restart — D-18-01, V2(d)) →
/// counted as expired when its time had passed, else voided by the restart (not counted — U4(c)).
fn reconcile_stale_pending(
    now: f64,
    tbl: &ApprovalTable,
    repair: &str,
    rec: &mut RepairRecord,
    consumed: &dyn Fn(&str) -> cys::repair::consumed::ApprovalUse,
) -> Reconciled {
    use cys::repair::consumed::ApprovalUse;
    let Some((fp, exp)) = rec.pending_request.clone() else {
        return Reconciled::Nothing;
    };
    let serving = tbl
        .requests
        .iter()
        .find(|r| r.repair == repair && r.fingerprint_sha256 == fp && r.expires_at == exp);
    let expired = match serving {
        Some(r) if r.state.live() => return Reconciled::Nothing,
        Some(r) => r.state == ReqState::Expired,
        None => now >= exp,
    };
    if let Some(a) = rec
        .approval
        .clone()
        .filter(|a| a.approved && a.fingerprint_sha256 == fp)
    {
        let used = if serving.is_some_and(|r| r.state == ReqState::Approved) {
            ApprovalUse::Consumed
        } else {
            consumed(&a.request_id)
        };
        match used {
            ApprovalUse::Consumed => {
                rec.pending_request = None;
                return Reconciled::Settled(None);
            }
            ApprovalUse::NotConsumed => rec.approval = None,
            ApprovalUse::Unknown(why) => return Reconciled::Unknown(why),
        }
    }
    rec.pending_request = None;
    Reconciled::Settled(expired.then(|| bump_expired(rec, &fp)))
}

/// RULINGS EL5: the ONE finding of an approval whose consumption is unknown (once per repair per daemon
/// incarnation — never one per sweep or per submit). `None` when it was already filed.
fn unknown_finding(
    tbl: &mut ApprovalTable,
    repair: &str,
    rec: &RepairRecord,
    why: &str,
) -> Option<(String, String)> {
    // RULINGS GA8 (2): only a cache of rows the ledger already answered for ([`report_unknown`])
    if tbl.unknown_reported.contains(repair) {
        return None;
    }
    let rel = ledger::file_targets(rec)
        .next()
        .map(|f| f.rel.clone())
        .unwrap_or_default();
    let rq = rec
        .approval
        .as_ref()
        .map(|a| a.request_id.clone())
        .unwrap_or_default();
    Some((
        rel,
        format!(
            "수리 {repair}: 이전 승인({rq})이 적용에 쓰였는지 알 수 없습니다({why}) — 승인을 지우지도 다시 쓰지도 않습니다"
        ),
    ))
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
    match outcome {
        Err(err) => {
            let _ = end_request(
                ctx,
                table,
                eff,
                request_id,
                &[ReqState::Checking],
                ReqState::Voided,
                format!(
                    "제출 검사에서 돌려보냈습니다(주인에게 올리지 않았습니다): {}",
                    err.why
                ),
                RepairState::InProgress,
                RecApproval::Keep,
            );
            // RULINGS GA8: step 3 wrote its comparisons' rows FIRST (`err.intake` lists them — never
            // filed again here); the master's pointer exists only for a row the ledger answered NEW,
            // and goes out once, one attempt
            if let Some(line) = err.owner_line {
                eff.point_master(line);
            }
        }
        Ok(ok) => {
            // the card appears only for a request still in step 3 (never over an end that came first)
            let card = {
                let mut tbl = table.lock().unwrap_or_else(|e| e.into_inner());
                match tbl
                    .find_mut(request_id)
                    .filter(|r| r.state == ReqState::Checking)
                {
                    Some(r) => {
                        r.state = ReqState::Pending;
                        r.rehearsal = Some((true, ok.rehearsal_summary.clone()));
                        r.screen_notes = ok.screen_notes.clone();
                        Some(r.clone())
                    }
                    None => None,
                }
            };
            let Some(r) = card else {
                return;
            };
            for (file, line) in &ok.lane_findings {
                eff.intake(file, line);
            }
            eff.publish(
                "repair.approval.created",
                json!({"request_id": r.request_id, "repair": r.repair, "targets": r.targets, "expires_at": r.expires_at}),
            );
        }
    }
}

/// The production step 3: every FILE target checked by the rehearsal runner in a scratch copy of
/// the lane's pack with the candidate laid in — INSIDE the scoped after-run comparison (RULINGS CZ3 /
/// DE1) — then the protection screen (its own comparison). The screen's verdict is recorded
/// (`screen.json`) when it accepts and the live inputs still match (CZ5).
pub(crate) fn production_step3(
    s3: &Step3Input,
    file: &dyn Fn(&str, &str) -> crate::repair::Filed,
) -> Result<Step3Ok, Step3Err> {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    production_step3_with(s3, &home, file)
}

/// One rehearsal pass: a fresh scratch copy of the lane's pack with the candidate's states laid in,
/// EVERY target checked (RULINGS GA3: a failing target does not end the pass early — the plan of the
/// pass, declared before the first check, must be carried out whole). Returns (the summary or the
/// first failure, the pass COMPLETE — `replay::PassPlan`: every planned check ran to a result whose
/// executions were observed whole).
fn rehearse_once(
    s3: &Step3Input,
    staging: &std::path::Path,
    started: std::time::Instant,
) -> (Result<Vec<String>, String>, bool) {
    use cys::repair::rehearsal::{self, CheckRequest, CheckResult};
    use cys::repair::replay::{self, PassPlan, ScratchRoot};
    let plan = PassPlan::default();
    let checks: Vec<&(String, TargetState)> = s3
        .states
        .iter()
        .filter(|(_, st)| *st != TargetState::Absent)
        .collect();
    plan.plan(checks.len());
    let early = |why: String| (Err(why), false);
    let scratch = match ScratchRoot::new(&rehearsal::scratch_base(), "s3-") {
        Ok(s) => s,
        Err(e) => return early(format!("{e:?}")),
    };
    let layout = scratch.path().join("pack");
    if let Err(e) = rehearsal::copy_layout(&s3.pack, &layout) {
        return early(format!("예행 사본을 만들지 못했습니다 — {e:?}"));
    }
    for (rel, st) in &s3.states {
        let dst = layout.join(rel);
        if let Err(e) = replay::remove_tree(&dst) {
            return early(format!("{rel}: {e:?}"));
        }
        let src = match st {
            TargetState::Absent => replay::LaidSource::Absent,
            TargetState::Link { target } => replay::LaidSource::Link(target.clone()),
            TargetState::File { .. } => match replay::LaidSource::read(&staging.join(rel)) {
                Ok(s) => s,
                Err(e) => return early(format!("{rel}: {e:?}")),
            },
        };
        if src.state() != *st {
            return early(format!("{rel}: 작업 사본이 제출 뒤에 바뀌었습니다"));
        }
        if let Err(e) = replay::lay_out(&dst, &src) {
            return early(format!("{rel}: {e:?}"));
        }
    }
    let mut summary = Vec::new();
    let mut first_failure: Option<String> = None;
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
        let (r, whole) = rehearsal::run_check_observed(&CheckRequest {
            entry_rel: rel.clone(),
            scratch_root: layout.clone(),
            recorded_input: None,
            deadline: own.min(left),
        });
        plan.record(whole);
        let failure = match r {
            Ok(CheckResult::Pass) => {
                summary.push(if rehearsal::prerehearsal_possible(rel) {
                    format!("{rel}: 통과")
                } else {
                    format!("{rel}: {}", rehearsal::NOT_REHEARSABLE_KO)
                });
                None
            }
            Ok(CheckResult::Fail(m)) => Some(format!("예행 검사 실패 — {m}")),
            Ok(CheckResult::Timeout) => Some(format!("{rel}: 예행 검사 시간 초과")),
            Err(e) => Some(format!("{rel}: 예행 검사를 돌리지 못했습니다 — {e:?}")),
        };
        if first_failure.is_none() {
            first_failure = failure;
        }
    }
    if let Err(e) = scratch.close() {
        return early(format!("예행 사본을 지우지 못했습니다 — {e:?}"));
    }
    let complete = plan.complete();
    match first_failure {
        Some(why) => (Err(why), complete),
        None => (Ok(summary), complete),
    }
}

/// [`production_step3`] with the HOME whose real settings files the comparisons watch. RULINGS GA8:
/// what a comparison finds is written to the intake ledger through `file` FIRST — when it is found,
/// before any later step can fail — and every later error carries the found list (one exit).
pub(crate) fn production_step3_with(
    s3: &Step3Input,
    real_home: &std::path::Path,
    file: &dyn Fn(&str, &str) -> crate::repair::Filed,
) -> Result<Step3Ok, Step3Err> {
    use cys::repair::rehearsal::{self, Comparison};
    use cys::repair::screen::{self, ScreenCandidate, ScreenVerdict};
    if s3.windows {
        return Err("윈도우에서는 1차 출시에서 이 검사를 하지 않습니다".into());
    }
    let started = std::time::Instant::now();
    let staging = ledger::staging_dir_in(&s3.root, &s3.repair);
    let first_rel = s3
        .states
        .first()
        .map(|(r, _)| r.clone())
        .unwrap_or_default();
    // (a) the rehearsal, INSIDE the scoped after-run comparison (snapshot → run → repeat on a
    // difference; its failure paths included — RULINGS CZ3 / DE1)
    let (rehearsed, cmp) = rehearsal::compared_runs(&s3.pack, real_home, || {
        // RULINGS GA3: complete = the pass's plan carried out whole (one function)
        rehearse_once(s3, &staging, started)
    });
    let mut found: Vec<(String, String)> = Vec::new();
    let result = (|| -> Result<Step3Ok, Step3Err> {
        let mut notes: Vec<String> = Vec::new();
        match cmp {
            Comparison::Refused(line) => {
                let filed = file(&first_rel, &line);
                return Err(Step3Err::escaped(&s3.repair, &first_rel, line, &filed));
            }
            Comparison::Noted(line) => {
                file(&first_rel, &line);
                found.push((first_rel.clone(), line.clone()));
                notes.push(line);
            }
            Comparison::Clean => {}
        }
        let mut summary = rehearsed.map_err(Step3Err::from)?;
        // (b) the protection screen (C4 — every boot-critical and protection-related submit, AT2)
        let cand = ScreenCandidate {
            files: staging.clone(),
            targets: s3.states.clone(),
        };
        let left = STEP3_DEADLINE.saturating_sub(started.elapsed());
        let rep = screen::screen_protections(&cand, &s3.pack, left)
            .map_err(|e| Step3Err::from(format!("보호장치 심사를 끝내지 못했습니다 — {e:?}")))?;
        // RULINGS DE1: a refusal by the screen's own after-run comparison has the same consequences
        if let Some(line) = screen::after_run_refusal(&rep) {
            let filed = file(&first_rel, &line);
            return Err(Step3Err::escaped(&s3.repair, &first_rel, line, &filed));
        }
        // … and its noted difference is written now, whatever the screen's verdict
        let noted = screen::after_run_note(&rep);
        if let Some(line) = &noted {
            file(&first_rel, line);
            found.push((first_rel.clone(), line.clone()));
        }
        if rep.verdict == ScreenVerdict::Refused {
            return Err(format!("보호장치 심사 거부 — {}", rep.refusals.join(" / ")).into());
        }
        // RULINGS CZ5: recorded only while the live inputs still match what the screen tested
        notes.extend(rep.notes.iter().cloned());
        match screen::recordable_verdict(&rep, &cand, &s3.pack)
            .map_err(|e| Step3Err::from(format!("심사 결과를 기록할지 판단하지 못했습니다 — {e:?}")))?
        {
            Some(verdict) => screen::write_verdict_in(&s3.root, &s3.repair, &verdict)
                .map_err(|e| Step3Err::from(format!("심사 결과를 기록하지 못했습니다 — {e:?}")))?,
            None => notes.push(
                "심사하는 동안 레인의 입력이 바뀌어 심사 결과를 기록하지 않았습니다 — 적용할 때 다시 심사합니다".into(),
            ),
        }
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
        // the screen's own lane findings (the comparison's noted line was already written above)
        Ok(Step3Ok {
            rehearsal_summary: summary.join(" · "),
            screen_notes: notes,
            lane_findings: rep
                .lane_findings
                .iter()
                .filter(|l| noted.as_deref() != Some(l.as_str()))
                .map(|l| (device_of(l), l.clone()))
                .collect(),
        })
    })();
    // EO5 (a) / GA8 (1): every error after a finding carries the found list
    result.map_err(|mut e| {
        e.intake.extend(found.iter().cloned());
        e
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
            ReqState::Pending | ReqState::Verifying | ReqState::Applying => {
                open.push(card(r, &other))
            }
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
            &[ReqState::Pending],
            ReqState::Expired,
            "30분이 지나 만료되었습니다".into(),
            RepairState::AwaitingApproval,
            RecApproval::Keep,
        )?;
        return Err(precondition(format!("{rid}: 30분이 지나 만료되었습니다")));
    }
    {
        let mut tbl = table.lock().unwrap_or_else(|e| e.into_inner());
        match tbl.find_mut(&rid) {
            Some(r) if r.state == ReqState::Pending && ctx.now < r.expires_at => {
                r.state = ReqState::Verifying
            }
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
                if let Some(r) = tbl
                    .find_mut(&rid)
                    .filter(|r| r.state == ReqState::Verifying)
                {
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
                &[ReqState::Verifying],
                ReqState::Rejected,
                format!("본인 확인을 시작하지 못했습니다({e}) — 승인이 아닙니다"),
                RepairState::InProgress,
                RecApproval::Keep,
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
    // RULINGS DR3 (the same rule as the replay runner): killed, its exit polled for a short fixed
    // grace, never waited for — a helper stuck in the kernel is left to a detached reap
    let _ = child.kill();
    let grace = std::time::Instant::now();
    while grace.elapsed() < Duration::from_millis(500) {
        if let Ok(Some(_)) = child.try_wait() {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let _ = std::thread::Builder::new()
        .name("repair-helper-reap".into())
        .spawn(move || {
            let _ = child.wait();
        });
    false
}

/// The helper's result consumed (design §4-7 steps 6–9; I3 validity by timestamp; verify → apply →
/// consume). `verified` = the helper exited 0.
///
/// RULINGS CZ2 / DE4: under the repair lock (the record and the candidate read first) and then the
/// table lock, the result is re-validated — the request's identity (this id, still `Verifying`, its
/// live one-time nonce), its expiry, its cancellation (the record still 승인 대기 and still naming
/// THIS request) and the candidate (same fingerprint, same targets) — and the request is CLAIMED:
/// a terminal state, or `Applying` (owned by this verification; no other end can take it). The table
/// says `Approved` only after the apply's outcome is known; a declined apply leaves `Voided`.
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
    let repair = {
        let tbl = table.lock().unwrap_or_else(|e| e.into_inner());
        match tbl.find(rid) {
            Some(r) if r.state == ReqState::Verifying => r.repair.clone(),
            _ => return Ok(()),
        }
    };
    let id = RepairId::parse(&repair).map_err(VerbError::from)?;
    enum Step {
        Ended(ReqState, String, Option<u32>, Result<(), VerbError>),
        Apply(Vec<(String, TargetState)>, Result<(), VerbError>),
    }
    let decided = ctx.locked(id.as_str(), || -> Option<(Request, Step)> {
        let rec = ledger::read_record_in(&ctx.root, &id);
        let states = ledger::candidate_states_in(&ctx.root, &id);
        let t = now(ctx);
        let mut tbl = table.lock().unwrap_or_else(|e| e.into_inner());
        let r = tbl.find_mut(rid)?;
        // a result that does not carry THIS request's live one-time nonce is no result
        if r.state != ReqState::Verifying || r.nonce.is_empty() || r.nonce != nonce {
            return None;
        }
        let snap = r.clone();
        let cancelled = !matches!(&rec, Ok(Some(x)) if x.state == RepairState::AwaitingApproval && pending_is(x, &snap));
        let same_candidate = matches!(&states, Ok(s) if cys::repair::fingerprint(s) == snap.fingerprint_sha256
            && s.iter().map(|(r, _)| r.clone()).collect::<Vec<_>>() == snap.targets);
        let end: Option<(ReqState, String, RepairState, RecApproval)> = if !verified {
            Some((
                ReqState::Rejected,
                "운영체제 본인 확인이 끝나지 않았거나 실패했습니다 — 승인이 아닙니다".into(),
                RepairState::InProgress,
                RecApproval::Set(ApprovalResult {
                    request_id: rid.into(),
                    fingerprint_sha256: snap.fingerprint_sha256.clone(),
                    decided_at: ctx.now,
                    approved: false,
                }),
            ))
        } else if t >= snap.expires_at {
            // I3: a result at or after the deadline is not approved, whatever the stored state says
            Some((
                ReqState::Expired,
                "본인 확인 결과가 30분이 지난 뒤에 왔습니다 — 승인이 아닙니다".into(),
                RepairState::AwaitingApproval,
                RecApproval::Keep,
            ))
        } else if cancelled {
            Some((
                ReqState::Voided,
                "그 사이에 이 요청이 취소되었습니다(되돌리기·새 수리·데몬 재시작) — 승인이 아닙니다".into(),
                RepairState::InProgress,
                RecApproval::Keep,
            ))
        } else if !same_candidate {
            // verify: the candidate re-compared just before the apply (after the card was shown)
            Some((
                ReqState::Voided,
                "카드를 보인 뒤에 후보가 바뀌었습니다 — 승인이 아닙니다. 다시 서명을 받아 제출하세요".into(),
                RepairState::InProgress,
                RecApproval::Keep,
            ))
        } else {
            None
        };
        // THE claim: the request leaves `Verifying` here, exactly once, and the nonce is spent
        r.state = end.as_ref().map_or(ReqState::Applying, |e| e.0);
        r.nonce.clear();
        r.helper_pid = None;
        if let Some((_, why, _, _)) = &end {
            r.reason = Some(why.clone());
            r.ended_at = Some(ctx.now);
        }
        drop(tbl);
        // the durable side, still under the repair lock
        Some(match end {
            Some((to, why, back_to, approval)) => {
                let (count, res) = settle_record(ctx, &id, &snap, to, back_to, &approval);
                (snap, Step::Ended(to, why, count, res))
            }
            None => {
                // F9: the approval result into the record (durably) BEFORE the apply
                let recorded = match rec {
                    Ok(Some(mut x)) => {
                        x.approval = Some(ApprovalResult {
                            request_id: rid.into(),
                            fingerprint_sha256: snap.fingerprint_sha256.clone(),
                            decided_at: ctx.now,
                            approved: true,
                        });
                        write(ctx, &x)
                    }
                    _ => Err(precondition(format!("{rid}: 장부 기록을 읽을 수 없습니다"))),
                };
                (snap, Step::Apply(states.unwrap_or_default(), recorded))
            }
        })
    });
    let Some((req, step)) = decided else {
        return Ok(());
    };
    let (states, recorded) = match step {
        Step::Ended(to, why, count, res) => {
            announce(ctx, eff, &req, to, &why, count);
            return res;
        }
        Step::Apply(states, recorded) => (states, recorded),
    };
    if let Err(e) = recorded {
        settle_owned(
            ctx,
            table,
            eff,
            &req,
            ReqState::Voided,
            format!("승인 결과를 기록하지 못해 적용하지 않았습니다 — {}", e.message),
            RepairState::InProgress,
            RecApproval::Clear,
        )?;
        return Err(e);
    }
    // the apply re-checks the expiry itself too (I3 — "again at apply")
    if now(ctx) >= req.expires_at {
        return settle_owned(
            ctx,
            table,
            eff,
            &req,
            ReqState::Expired,
            "적용 직전에 30분이 지났습니다 — 적용하지 않았습니다".into(),
            RepairState::AwaitingApproval,
            RecApproval::Clear,
        );
    }
    // RULINGS DE3: EVERY approved target goes to the apply — a deletion (nothing staged at its path)
    // and a link (a link staged there) as well as a file; the apply reads each staged state itself
    // and refuses a list that is not the record's whole target set
    let staging = ledger::staging_dir_in(&ctx.root, &id);
    let files: Vec<cys::repair::apply::CandidateFile> = states
        .iter()
        .map(|(rel, st)| cys::repair::apply::CandidateFile {
            rel: rel.clone(),
            staged: staging.join(rel),
            mode: match st {
                TargetState::File { mode, .. } => *mode,
                _ => 0,
            },
        })
        .collect();
    match eff.apply(&id, &ctx.lane, &files) {
        Ok(cys::repair::apply::ApplyOutcome::Applied) => settle_owned(
            ctx,
            table,
            eff,
            &req,
            ReqState::Approved,
            "주인이 승인했고 데몬이 적용했습니다(시험 기간 재검사가 이어집니다)".into(),
            RepairState::AwaitingApproval,
            RecApproval::Keep,
        ),
        // AY5: a declined apply — in the origin the approval is void and the repair returns to
        // `진행 중`; the table says so (DE4)
        Ok(cys::repair::apply::ApplyOutcome::Declined(why))
        | Ok(cys::repair::apply::ApplyOutcome::Superseded(why)) => settle_owned(
            ctx,
            table,
            eff,
            &req,
            ReqState::Voided,
            format!("승인되었지만 적용이 거절되었습니다 — {why}"),
            RepairState::InProgress,
            RecApproval::Clear,
        ),
        Err(e) => settle_owned(
            ctx,
            table,
            eff,
            &req,
            ReqState::Voided,
            format!(
                "승인되었지만 적용하지 못했습니다 — {}",
                VerbError::from(e).message
            ),
            RepairState::InProgress,
            RecApproval::Clear,
        ),
    }
}

/// `repair.approval_reject {request_id}` — the owner says no (claims `Rejected` from `Pending` only).
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
    let ended = end_request(
        ctx,
        table,
        eff,
        &rid,
        &[ReqState::Pending],
        ReqState::Rejected,
        "주인이 거절했습니다".into(),
        RepairState::InProgress,
        RecApproval::Set(ApprovalResult {
            request_id: rid.clone(),
            fingerprint_sha256: fp,
            decided_at: ctx.now,
            approved: false,
        }),
    )?;
    if !ended {
        return Err(precondition(format!(
            "{rid}: 그 사이에 요청이 끝났거나 본인 확인이 시작되었습니다 — 거절하지 않았습니다"
        )));
    }
    Ok(json!({"request_id": rid, "state": ReqState::Rejected}))
}

/// Voids a live request of `repair` (its target was reverted by the daemon's automatic revert —
/// 5.11; the revert never waits for this request's lock). Told to the fixer once. A request being
/// applied is not voided (its verification owns it).
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
            ENDABLE,
            ReqState::Voided,
            why.to_string(),
            RepairState::InProgress,
            RecApproval::Keep,
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
            .filter(|r| ENDABLE.contains(&r.state) && ctx.now >= r.expires_at)
            .map(|r| r.request_id.clone())
            .collect()
    };
    // 5.11: a live request whose repair is no longer 승인 대기 (the daemon reverted or superseded its
    // target) is void — the revert never waited for it; the fixer is told once
    let live: Vec<String> = {
        let tbl = table.lock().unwrap_or_else(|e| e.into_inner());
        tbl.requests
            .iter()
            .filter(|r| {
                matches!(r.state, ReqState::Pending | ReqState::Verifying)
                    && !due.contains(&r.request_id)
            })
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
            ENDABLE,
            ReqState::Expired,
            "30분이 지나 만료되었습니다 — 후보는 작업 사본에 남아 있고 다시 제출할 수 있습니다"
                .into(),
            RepairState::AwaitingApproval,
            RecApproval::Keep,
        );
    }
    let mut tbl = table.lock().unwrap_or_else(|e| e.into_inner());
    tbl.requests
        .retain(|r| r.state.live() || r.ended_at.map_or(true, |t| ctx.now - t < ENDED_KEEP_SECS));
}

/// A record of this lane still naming a pending request that no request of this table serves (after
/// a restart): reconciled under the repair lock by [`reconcile_stale_pending`] — expired if its time
/// had passed (counted), else voided by the restart (not counted — D-18-01). The lock is released by
/// the restart itself (lease incarnation).
fn resolve_after_restart(ctx: &Ctx<'_>, table: &Mutex<ApprovalTable>, eff: &dyn Effects) {
    let consumed = |r: &str| eff.approval_consumed(&ctx.root, &ctx.lane, r);
    for id in ledger::list_ids_in(&ctx.root) {
        let mut count = None;
        let mut found: Option<(String, String)> = None;
        let res = ctx.locked(id.as_str(), || -> Result<bool, VerbError> {
            let Ok(Some(mut rec)) = ledger::read_record_in(&ctx.root, &id) else {
                return Ok(false);
            };
            if rec.origin_lane != ctx.lane {
                return Ok(false);
            }
            let decided = {
                let mut tbl = table.lock().unwrap_or_else(|e| e.into_inner());
                let d = reconcile_stale_pending(ctx.now, &tbl, id.as_str(), &mut rec, &consumed);
                if let Reconciled::Unknown(why) = &d {
                    found = unknown_finding(&mut tbl, id.as_str(), &rec, why);
                }
                d
            };
            let Reconciled::Settled(c) = decided else {
                return Ok(false);
            };
            write(ctx, &rec)?;
            count = c;
            Ok(true)
        });
        // after every lock (the report verb takes its own)
        if let Some((file, line)) = found {
            report_unknown(table, eff, id.as_str(), &file, &line);
        }
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

/// RULINGS DF4: runs `f`, a panic inside it caught and logged (the caller's loop — the governance
/// watchdog, a worker thread of this channel — goes on). `false` = it panicked.
pub(crate) fn guarded(what: &str, f: impl FnOnce()) -> bool {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(()) => true,
        Err(_) => {
            eprintln!("cysd: 「스스로 해결」 {what} 에서 패닉 — 잡아서 기록하고 계속합니다");
            false
        }
    }
}

/// RULINGS DF4: a named thread for this channel; a thread that cannot be created is an error that is
/// handled (logged, `false` — the caller takes its fallback), never a panic.
pub(crate) fn spawn_thread(name: &str, f: impl FnOnce() + Send + 'static) -> bool {
    #[cfg(test)]
    if FAIL_SPAWN_FOR_TEST.with(|x| x.get()) {
        drop(f);
        return false;
    }
    match std::thread::Builder::new()
        .name(format!("repair-{name}"))
        .spawn(move || {
            guarded("작업 스레드", f);
        }) {
        Ok(_) => true,
        Err(e) => {
            eprintln!("cysd: 「스스로 해결」 스레드({name})를 만들지 못했습니다 — {e}");
            false
        }
    }
}

#[cfg(test)]
thread_local! {
    /// Test seam of RULINGS DF4 / EL4: no thread can be made on this thread.
    pub(crate) static FAIL_SPAWN_FOR_TEST: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// One enqueue attempt without waiting (the fallback when no delivery thread could be made).
/// RULINGS GA8 (4): the lane master's pointer — one enqueue attempt on the calling thread, never
/// retried; no master → nothing.
fn point_master_now(daemon: &Arc<Daemon>, text: String) {
    let sid = daemon
        .roles
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get("master")
        .copied();
    if let Some(sid) = sid {
        deliver_now(daemon, sid, text);
    }
}

fn deliver_now(daemon: &Arc<Daemon>, seat: u64, text: String) {
    use crate::alert_route::{enqueue_into_seat, FreezeGuard};
    let _ = enqueue_into_seat(
        daemon,
        seat,
        text,
        Some("repair".to_string()),
        "repair_result",
        100,
        None,
        FreezeGuard::Daemon,
        None,
    );
}

impl Effects for DaemonEffects {
    fn deliver(&self, seat: u64, text: String) {
        let d = self.daemon.clone();
        let (d2, t2) = (d.clone(), text.clone());
        if !spawn_thread("deliver", move || deliver_once(&d, seat, text)) {
            deliver_now(&d2, seat, t2);
        }
    }

    fn tell_master(&self, text: String) {
        let master = |d: &Arc<Daemon>| {
            d.roles
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get("master")
                .copied()
        };
        let d = self.daemon.clone();
        let (d2, t2) = (d.clone(), text.clone());
        if !spawn_thread("tell-master", move || {
            if let Some(sid) = master(&d) {
                deliver_once(&d, sid, text);
            }
        }) {
            if let Some(sid) = master(&d2) {
                deliver_now(&d2, sid, t2);
            }
        }
    }

    fn point_master(&self, text: String) {
        point_master_now(&self.daemon, text);
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

    fn step3(&self, s3: &Step3Input) -> Result<Step3Ok, Step3Err> {
        production_step3(s3, &|f, l| self.intake(f, l))
    }

    fn intake(&self, file: &str, error: &str) -> crate::repair::Filed {
        crate::repair::file_intake(&self.daemon, file, error)
    }

    fn approval_consumed(
        &self,
        root: &std::path::Path,
        lane: &LaneKey,
        approval_ref: &str,
    ) -> cys::repair::consumed::ApprovalUse {
        // the seam's body is T6's; until it is merged it answers Unknown (the safe answer)
        cys::repair::consumed::approval_consumed_in(root, lane, approval_ref)
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
    let eff = DaemonEffects {
        daemon: daemon.clone(),
    };
    let (out, s3) = submit(ctx, caller, p, &daemon.repair_approvals, &eff, gate)?;
    if let (Some(s3), Some(rid)) = (
        s3,
        out.get("request_id")
            .and_then(|v| v.as_str())
            .map(str::to_string),
    ) {
        let d = daemon.clone();
        let rid2 = rid.clone();
        let started = spawn_thread("step3", move || {
            let eff = DaemonEffects { daemon: d.clone() };
            let ok = guarded("제출 검사", || {
                crate::repair::with_daemon_ctx(&d, |ctx| {
                    run_step3(ctx, &d.repair_approvals, &eff, &rid, &s3)
                })
            });
            if !ok {
                // a panic in step 3 returns the candidate (never a request left in step 3 to expire)
                crate::repair::with_daemon_ctx(&d, |ctx| {
                    let _ = end_request(
                        ctx,
                        &d.repair_approvals,
                        &eff,
                        &rid,
                        &[ReqState::Checking],
                        ReqState::Voided,
                        "제출 검사가 비정상으로 끝났습니다(주인에게 올리지 않았습니다)".into(),
                        RepairState::InProgress,
                        RecApproval::Keep,
                    );
                });
            }
        });
        if !started {
            let _ = end_request(
                ctx,
                &daemon.repair_approvals,
                &eff,
                &rid2,
                &[ReqState::Checking],
                ReqState::Voided,
                "제출 검사를 시작하지 못했습니다(주인에게 올리지 않았습니다)".into(),
                RepairState::InProgress,
                RecApproval::Keep,
            );
        }
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
        let pid = child.id();
        let (d1, rid1, nonce1) = (d.clone(), rid.clone(), nonce.clone());
        let started = start_waiter_or_reap(
            move || {
                let verified = wait_helper(child);
                let eff = DaemonEffects { daemon: d1.clone() };
                crate::repair::with_daemon_ctx(&d1, |ctx| {
                    let _ =
                        finish_verification(ctx, &d1.repair_approvals, &eff, &rid1, &nonce1, verified);
                });
            },
            pid,
            true,
        );
        if let Err(unreaped) = started {
            // no waiter: the helper was killed + reaped (bounded) and the click is not an approval
            if let Some(p) = unreaped {
                eprintln!("cysd: 「스스로 해결」 본인 확인 도우미(pid {p})를 거두지 못했습니다 — 기록만 남깁니다");
            }
            let eff = DaemonEffects { daemon: d.clone() };
            crate::repair::with_daemon_ctx(&d, |ctx| {
                let _ = finish_verification(ctx, &d.repair_approvals, &eff, &rid, &nonce, false);
            });
        }
    };
    approval_start(ctx, p, &daemon.repair_approvals, &eff, &waiter)
}

/// RULINGS EL4: kills (unless `kill` is false — the test seam standing for a helper slow to die) and
/// reaps a helper whose `Child` handle is gone, NEVER blocking: `waitpid(…, WNOHANG)` polled for a
/// fixed grace. Returns the pid it could not reap.
fn kill_and_reap(pid: u32, kill: bool) -> Option<u32> {
    #[cfg(unix)]
    {
        let p = pid as libc::pid_t;
        if kill {
            // SAFETY: kill(2) on our own child, not reaped yet (its pid cannot be reused).
            unsafe {
                libc::kill(p, libc::SIGKILL);
            }
        }
        let grace = std::time::Instant::now();
        while grace.elapsed() < Duration::from_millis(500) {
            // SAFETY: waitpid(2) WNOHANG on our own child — returns at once.
            let r = unsafe { libc::waitpid(p, std::ptr::null_mut(), libc::WNOHANG) };
            if r == p || r < 0 {
                return None;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        Some(pid)
    }
    #[cfg(not(unix))]
    {
        let _ = kill;
        Some(pid)
    }
}

/// RULINGS DF4 / EL4: starts the helper's waiter on its own thread; when no thread can be made the
/// helper is killed and reaped HERE, bounded (`Err` carries the pid it could not reap, if any).
fn start_waiter_or_reap(
    start: impl FnOnce() + Send + 'static,
    pid: u32,
    kill: bool,
) -> Result<(), Option<u32>> {
    if spawn_thread("helper-wait", start) {
        Ok(())
    } else {
        Err(kill_and_reap(pid, kill))
    }
}

/// Production: the watchdog's call (governance) — panic-isolated (RULINGS DF4: a panic in the sweep
/// is logged and the watchdog cycle goes on).
pub(crate) fn watchdog_sweep(daemon: &Arc<Daemon>) {
    let eff = DaemonEffects {
        daemon: daemon.clone(),
    };
    guarded("승인 만료 정리", || {
        crate::repair::with_daemon_ctx(daemon, |ctx| sweep(ctx, &daemon.repair_approvals, &eff))
    });
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

/// RULINGS EO5 (a) / DE1 / GA8: what the scoped after-run comparison around the worker's `check`
/// found has the same consequences as in step 3 — ONE intake row (a noted difference too), written
/// FIRST and within this bounded step (no helper thread: the report verb takes one of the intake shard
/// locks — `repair::intake_lock_for`, RULINGS DA3 — never a repair lock), and for a refusal whose row the ledger answered NEW the lane master's pointer —
/// once, one attempt. Nothing is remembered in memory: a restart asks the ledger again.
pub(crate) fn comparison_consequence(
    daemon: &Arc<Daemon>,
    rel: &str,
    c: &cys::repair::rehearsal::Comparison,
) {
    check_consequence(
        rel,
        c,
        &|f, l| crate::repair::file_intake(daemon, f, l),
        &|text| point_master_now(daemon, text),
    );
}

/// [`comparison_consequence`]'s body over a given intake filing and pointer (the ledger's answer).
pub(crate) fn check_consequence(
    rel: &str,
    c: &cys::repair::rehearsal::Comparison,
    file: &dyn Fn(&str, &str) -> crate::repair::Filed,
    point: &dyn Fn(String),
) -> Option<crate::repair::Filed> {
    use cys::repair::rehearsal::Comparison;
    let (line, refused) = match c {
        Comparison::Clean => return None,
        Comparison::Noted(l) => (l, false),
        Comparison::Refused(l) => (l, true),
    };
    let filed = file(rel, line);
    if refused && filed == crate::repair::Filed::New {
        point(format!(
            "{rel} 의 검사(`check`)에서 예행이 실제 환경을 건드려 검사를 통과시키지 않았습니다 — 접수 장부에 남겼습니다(`cys repair status` · `cys repair list`)"
        ));
    }
    Some(filed)
}

/// RULINGS DF3: the repairs of `lane` whose owner HAS A CARD now — a request past step 3 whose
/// decision is pending (`Pending`, `Verifying`). The report gate counts these, never the record's
/// 승인 대기 state (which a request in step 3 and an expired one also show) — so a fix-and-resubmit
/// cycle whose step 3 refuses tells the master nothing.
pub(crate) fn approval_cards(table: &Mutex<ApprovalTable>, lane: &str) -> Vec<String> {
    let tbl = table.lock().unwrap_or_else(|e| e.into_inner());
    let mut ids: Vec<String> = tbl
        .requests
        .iter()
        .filter(|r| r.lane_key == lane && matches!(r.state, ReqState::Pending | ReqState::Verifying))
        .map(|r| r.repair.clone())
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

/// The approval part of `repair.status` (AU5: the worker reads a returned candidate's reason there)
/// and of `repair.list` (DF3: `approval_cards`, the repairs the owner has a card for).
pub(crate) fn augment_status(daemon: &Arc<Daemon>, sub: &str, p: &Value, mut v: Value) -> Value {
    if sub == "list" {
        if let Some(o) = v.as_object_mut() {
            let lane = o.get("lane").and_then(|l| l.as_str()).unwrap_or("").to_string();
            o.insert(
                "approval_cards".into(),
                json!(approval_cards(&daemon.repair_approvals, &lane)),
            );
        }
    }
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
