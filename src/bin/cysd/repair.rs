//! 「스스로 해결」 daemon side — the `repair.*` request namespace (C3 · O-C3-1).
//!
//! `handlers::dispatch_method` delegates every `repair.<sub>` request here (one prefix branch beside
//! `trash.`; the caller's seat and the owner grade are computed THERE, not in this module).
//! Track T3 fills: the caller's identity from the operating system only (RULINGS M1, M4, N2, O2, P5,
//! interpretation 36), the per-verb role table — DENY BY DEFAULT over `roles::VERB_CLASSES` (J1, P4,
//! B2, F10, R8; O-C3-8: a table in this module, `caps.rs` unchanged) — and the verbs `report`,
//! `begin`, `record`, `commit`, `attest_verify`, `attest_audit`, `list`, `status`, `cleanup`
//! (the inline module `ledger_verbs`), the lock releases at seat reap (C1 1.12 / O5) and the per-seat prompt-origin
//! record (RULINGS T3, U4(a)). Every other sub-verb passes the role table and then answers
//! `repair_not_implemented` until its track fills it (T4 grant / owner token, T5b approval / submit,
//! T6 revert / tick / sweep / reapply, T7b recheck).
//! ★Nothing here may panic (F4 — a panicking handler would take the daemon and every pane with it):
//! no `unwrap`/`expect`/indexing/`todo!`/`unimplemented!` in production code of this module.
//! ★Nothing here sends anything to a queue, an alert route or the event bus (F1): intake is data.

#[cfg(test)]
#[path = "repair_ledger_tests.rs"]
mod ledger_tests;

use crate::handlers::Reply;
use crate::state::Daemon;
use cys::repair::roles::{reviewer_slot_of, verb_class, CallerKind, ReviewerSlot, VerbClass};
use cys::repair::types::{LaneKey, RepairError};
use cys::{err_response, ok_response};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// The daemon sub-verbs of `repair.` (O-C3-1, N-4; `recheck` H5/H8, `grant_ask` Q1,
/// `owner_token_ask` Q2) — 23.
pub(crate) const REPAIR_SUBVERBS: &[&str] = &[
    "report",
    "begin",
    "record",
    "submit",
    "commit",
    "attest_verify",
    "attest_audit",
    "list",
    "status",
    "revert",
    "reapply",
    "cleanup",
    "approval_list",
    "approval_start",
    "approval_reject",
    "grant",
    "owner_token",
    "rule_doc_allowed",
    "sweep",
    "tick",
    "recheck",
    "grant_ask",
    "owner_token_ask",
];

/// The sub-verbs whose bodies are track T3's (the rest answer `repair_not_implemented` after the
/// role table until their own track fills them).
pub(crate) const T3_VERBS: &[&str] = &[
    "report",
    "begin",
    "record",
    "commit",
    "attest_verify",
    "attest_audit",
    "list",
    "status",
    "cleanup",
];

/// M4 (★OWNER-REPORT with the other Windows limits): on Windows a seatless command line is not a
/// human in 0.15.0 — the app's button is the path.
pub(crate) const WINDOWS_SEATLESS_REFUSAL_KO: &str = "윈도우에서는 좌석이 아닌 명령줄의 `cys repair revert`·`cleanup`을 받지 않습니다 — 앱의 '직전 수리 되돌리기' 버튼을 쓰세요.";

/// Who is calling — decided from the operating system only (M1): the kernel peer pid's ancestry
/// or controlling terminal, the app's operator token. No request field, flag or environment
/// variable takes part.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Caller {
    pub kind: CallerKind,
    /// The seat's daemon-recorded role (`None` for a human / detached caller or a seat without one).
    pub role: Option<String>,
    /// The app's operator token was presented by a seatless caller (interpretation 36).
    pub app_token: bool,
    /// RULINGS AD1: the family of the FIRST known-family role this seat held in this daemon
    /// incarnation (`None` for a seatless caller or a seat that never held a known-family role).
    pub bound_family: Option<RoleFamily>,
}

/// The role families of RULINGS AD1. An empty or unknown role has none and binds nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum RoleFamily {
    /// master · CEO · a department's master
    Master,
    Worker,
    Reviewer,
    Cso,
    Planner,
}

impl RoleFamily {
    pub(crate) fn label(self) -> &'static str {
        match self {
            RoleFamily::Master => "master·CEO",
            RoleFamily::Worker => "worker",
            RoleFamily::Reviewer => "reviewer",
            RoleFamily::Cso => "CSO",
            RoleFamily::Planner => "planner",
        }
    }
}

/// The family of a role name (AD1): `master` / `ceo` (+ `-<n>` variants) → master·CEO; `worker`,
/// `worker-*` → worker; `reviewer*` → reviewer; `cso*` → CSO (the alert-route prefix);
/// `planner*` → planner; anything else → none.
pub(crate) fn role_family(role: &str) -> Option<RoleFamily> {
    let r = role.trim();
    if r == "master" || r == "ceo" || r.starts_with("master-") || r.starts_with("ceo-") {
        Some(RoleFamily::Master)
    } else if r == "worker" || r.starts_with("worker-") {
        Some(RoleFamily::Worker)
    } else if r.starts_with("reviewer") {
        Some(RoleFamily::Reviewer)
    } else if r.starts_with("cso") {
        Some(RoleFamily::Cso)
    } else if r.starts_with("planner") {
        Some(RoleFamily::Planner)
    } else {
        None
    }
}

/// RULINGS AD1 — per daemon (lane + incarnation) and seat: the family of the first known-family
/// role the seat held. Memory only (it dies with the daemon — a new incarnation starts empty); the
/// entry goes when the surface closes ([`release_locks_for_seat`]). A leaf lock: it is taken while
/// `claim_role` holds its surface / role locks and never takes another lock itself.
fn role_bindings() -> &'static Mutex<RoleBindings> {
    static B: std::sync::OnceLock<Mutex<RoleBindings>> = std::sync::OnceLock::new();
    B.get_or_init(|| Mutex::new(HashMap::new()))
}

/// (lane, daemon incarnation, seat) → the family of the seat's first known-family role (AD1).
type RoleBindings = HashMap<(String, String, u64), RoleFamily>;

/// Binds `role`'s family to the seat unless the seat is already bound or the role has no known
/// family; returns the binding after the call.
pub(crate) fn bind_role_family(
    lane: &str,
    incarnation: &str,
    sid: u64,
    role: Option<&str>,
) -> Option<RoleFamily> {
    let mut m = role_bindings().lock().unwrap_or_else(|e| e.into_inner());
    let key = (lane.to_string(), incarnation.to_string(), sid);
    if let Some(f) = m.get(&key) {
        return Some(*f);
    }
    let fam = role.and_then(role_family)?;
    m.insert(key, fam);
    Some(fam)
}

/// Drops a closed seat's binding.
pub(crate) fn forget_role_family(lane: &str, incarnation: &str, sid: u64) {
    role_bindings()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&(lane.to_string(), incarnation.to_string(), sid));
}

/// RULINGS AD1 — the ONE recording call where a role is claimed (`handlers.rs`, `system.claim_role`,
/// right before the new role is committed): the role the seat held before the claim binds first
/// (it is the seat's earlier role), else the claimed one. `claim_role` itself is unchanged.
pub(crate) fn note_role_claim(daemon: &Daemon, sid: u64, previous: Option<&str>, claimed: &str) {
    let lane = LaneKey::of_socket(&daemon.socket_path).0;
    record_role_claim(&lane, &daemon.incarnation, sid, previous, claimed);
}

/// The body of [`note_role_claim`] (pure over the binding map): the earlier role binds first, else
/// the claimed one; an existing binding never changes. Returns the binding after the call.
pub(crate) fn record_role_claim(
    lane: &str,
    incarnation: &str,
    sid: u64,
    previous: Option<&str>,
    claimed: &str,
) -> Option<RoleFamily> {
    bind_role_family(lane, incarnation, sid, previous)
        .or_else(|| bind_role_family(lane, incarnation, sid, Some(claimed)))
}

impl Caller {
    pub(crate) fn seat(&self) -> Option<u64> {
        match self.kind {
            CallerKind::Seat { sid } => Some(sid),
            _ => None,
        }
    }

    fn is_worker(&self) -> bool {
        self.seat().is_some()
            && self
                .role
                .as_deref()
                .is_some_and(|r| r == "worker" || r.starts_with("worker-"))
    }

    /// A short label for intake rows (no identity beyond what the daemon already shows).
    pub(crate) fn label(&self) -> String {
        match (&self.kind, &self.role) {
            (CallerKind::Seat { .. }, Some(r)) => r.clone(),
            (CallerKind::Seat { .. }, None) => "seat".into(),
            (CallerKind::Human, _) if self.app_token => "app".into(),
            (CallerKind::Human, _) => "human".into(),
            (CallerKind::Detached, _) => "detached".into(),
        }
    }
}

/// An answer of a verb that is not a success: a daemon error code + one sentence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VerbError {
    pub code: String,
    pub message: String,
}

impl VerbError {
    pub(crate) fn new(code: &str, message: impl Into<String>) -> Self {
        VerbError {
            code: code.to_string(),
            message: message.into(),
        }
    }
    /// O-C3-6 reused code, emitted directly (RULINGS Z1).
    pub(crate) fn invalid(message: impl Into<String>) -> Self {
        Self::new("invalid_params", message)
    }
    /// O-C3-6 reused code, emitted directly (RULINGS Z1).
    pub(crate) fn not_found(message: impl Into<String>) -> Self {
        Self::new("not_found", message)
    }
    pub(crate) fn refused(message: impl Into<String>) -> Self {
        Self::new("acl_denied", message)
    }
}

impl From<RepairError> for VerbError {
    fn from(e: RepairError) -> Self {
        let message = match &e {
            RepairError::NotImplemented(f) => format!("{} ({f})", cys::repair::NOT_IMPLEMENTED_KO),
            RepairError::OldDaemon => "old daemon".to_string(),
            RepairError::PlatformPending(m) => (*m).to_string(),
            RepairError::RoleRefused(m)
            | RepairError::Locked(m)
            | RepairError::Precondition(m)
            | RepairError::CheckFailed(m)
            | RepairError::CheckTimeout(m)
            | RepairError::Corrupt(m)
            | RepairError::Superseded(m)
            | RepairError::Declined(m)
            | RepairError::Io(m) => m.clone(),
        };
        VerbError {
            code: e.code().to_string(),
            message,
        }
    }
}

/// The outcome of the repair's check on one live file at commit (K4: durations recorded).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CheckReport {
    pub passed: bool,
    pub timed_out: bool,
    pub failure: String,
    pub check_ms: u64,
    pub reference_kind: cys::repair::boot_critical::CheckKind,
    pub reference_ms: u64,
}

/// Everything a verb needs, built from the daemon in production and directly by the tests.
pub(crate) struct Ctx<'a> {
    pub root: PathBuf,
    pub lane: LaneKey,
    pub incarnation: String,
    pub pack_dir: PathBuf,
    pub now: f64,
    pub windows: bool,
    pub registry: PathBuf,
    pub mutexes: &'a Mutex<HashMap<String, Arc<Mutex<()>>>>,
    /// Live seats (sid, role) — for the reviewer-slot line of `status` (B2).
    pub seats: Vec<(u64, String)>,
    /// Classifies a pack-relative target (T5a's list in production).
    pub classify: &'a dyn Fn(&str) -> Result<cys::repair::types::Classification, RepairError>,
    /// Runs the repair's check on a live file with a deadline (T5b's runner in production).
    pub check: &'a dyn Fn(&str, &Path, std::time::Duration) -> Result<CheckReport, RepairError>,
    /// The hook step of RULINGS Y1 (`apply::hook_lines_step`, T6's, in production).
    pub hook_step: &'a HookStepFn<'a>,
}

/// The shape of `apply::hook_lines_step` (injected so the commit's hook half is testable).
pub(crate) type HookStepFn<'a> = dyn Fn(
        &cys::repair::types::RepairId,
        &LaneKey,
        cys::repair::apply::HookStep,
    ) -> Result<cys::repair::apply::ReconcileReport, RepairError>
    + 'a;

impl Ctx<'_> {
    /// Runs `f` under the in-memory per-repair mutex of `key` (RULINGS G6, R3): every
    /// read-modify-write of one repair's files in this daemon is serialised. Poison is tolerated
    /// (the house rule) — a panicking holder must not wedge the repair forever.
    pub(crate) fn locked<T>(&self, key: &str, f: impl FnOnce() -> T) -> T {
        let m = {
            let mut map = self.mutexes.lock().unwrap_or_else(|e| e.into_inner());
            Arc::clone(
                map.entry(key.to_string())
                    .or_insert_with(|| Arc::new(Mutex::new(()))),
            )
        };
        let _g = m.lock().unwrap_or_else(|e| e.into_inner());
        f()
    }

    pub(crate) fn owner(&self, sid: u64) -> cys::repair::types::LockOwner {
        cys::repair::types::LockOwner {
            lane: self.lane.clone(),
            seat: sid,
            incarnation: self.incarnation.clone(),
        }
    }
}

/// Handles `repair.<sub>`. `caller_sid` = the caller's seat from the kernel peer pid's ancestry or
/// (macOS) controlling terminal (`resolve_caller_surface`), `owner_grade` = `trash_owner_grade_ok`
/// (no seat + the operator token).
pub(crate) fn handle(
    daemon: &Arc<Daemon>,
    sub: &str,
    params: &Value,
    id: &Value,
    caller_sid: Option<u64>,
    owner_grade: bool,
    caller_pid: Option<u32>,
) -> Reply {
    if !REPAIR_SUBVERBS.contains(&sub) {
        return Reply::Single(err_response(
            id,
            "method_not_found",
            &format!("unknown method: repair.{sub}"),
        ));
    }
    let caller = caller_of(daemon, caller_sid, owner_grade, caller_pid);
    let classify = production_classify;
    let check = production_check;
    let ctx = Ctx {
        root: production_root(daemon),
        lane: LaneKey::of_socket(&daemon.socket_path),
        incarnation: daemon.incarnation.clone(),
        pack_dir: cys::pack::pack_dir(),
        now: cys::repair::ledger::now_epoch(),
        windows: cfg!(windows),
        registry: cys::lane::registry_path(),
        mutexes: &daemon.repair_mutexes,
        seats: if sub == "status" {
            live_seats(daemon)
        } else {
            Vec::new()
        },
        classify: &classify,
        check: &check,
        hook_step: &cys::repair::apply::hook_lines_step,
    };
    Reply::Single(match handle_with(&ctx, &caller, sub, params) {
        Ok(v) => ok_response(id, v),
        Err(e) => err_response(id, &e.code, &e.message),
    })
}

/// The verb dispatcher behind [`handle`]: role table first, then the body.
pub(crate) fn handle_with(
    ctx: &Ctx<'_>,
    caller: &Caller,
    sub: &str,
    params: &Value,
) -> Result<Value, VerbError> {
    role_check(sub, caller, ctx.windows)?;
    let r = match sub {
        "report" => ledger_verbs::report(ctx, caller, params),
        "begin" => ledger_verbs::begin(ctx, caller, params),
        "record" => ledger_verbs::record(ctx, caller, params),
        "commit" => ledger_verbs::commit(ctx, caller, params),
        "attest_verify" => ledger_verbs::attest(ctx, caller, params, ReviewerSlot::Reviewer1),
        "attest_audit" => ledger_verbs::attest(ctx, caller, params, ReviewerSlot::Reviewer2),
        "list" => ledger_verbs::list(ctx),
        "status" => ledger_verbs::status(ctx, params),
        "cleanup" => ledger_verbs::cleanup(ctx, caller, params),
        "revert" => {
            ledger_verbs::worker_lane_check(ctx, caller, params).and_then(|_| not_implemented(sub))
        }
        _ => {
            debug_assert!(
                !T3_VERBS.contains(&sub),
                "a T3 verb fell through to the stub"
            );
            not_implemented(sub)
        }
    };
    r.map_err(|mut e| {
        if !e.message.starts_with("repair.") {
            e.message = format!("repair.{sub}: {}", e.message);
        }
        e
    })
}

fn not_implemented(sub: &str) -> Result<Value, VerbError> {
    Err(VerbError::new(
        cys::repair::types::ERR_NOT_IMPLEMENTED,
        format!("repair.{sub}: {}", cys::repair::NOT_IMPLEMENTED_KO),
    ))
}

/// THE per-verb role table (C3 · RULINGS J1 deny by default · P4 · B2 · F10 · O2 · N2 · M4).
/// Verb-level conditions that need the ledger (the repair's lock for `record`/`commit`, "a lane
/// holding a row" for `revert`/`cleanup`, never the fixer / never both by one seat) are checked by
/// the bodies.
pub(crate) fn role_check(sub: &str, caller: &Caller, windows: bool) -> Result<(), VerbError> {
    let Some(class) = verb_class(sub) else {
        return Err(VerbError::refused(format!(
            "`{sub}` 에는 역할 등급이 없습니다 — 어떤 좌석도 받지 않습니다"
        )));
    };
    // RULINGS AD1: for every verb that is not READ or INTAKE, a seat whose current role family
    // differs from the family it first held in this daemon incarnation is refused (renaming a seat
    // with `claim-role` does not change what it may do in the repair flow).
    if !matches!(class, VerbClass::Read | VerbClass::Intake) {
        if let (Some(_), Some(bound)) = (caller.seat(), caller.bound_family) {
            let current = caller.role.as_deref().and_then(role_family);
            if current != Some(bound) {
                return Err(VerbError::refused(format!(
                    "이 좌석은 처음에 '{}' 역할로 섰고 지금은 '{}' 입니다 — 역할 이름을 바꾼 좌석은 수리 명령을 쓸 수 없습니다(RULINGS AD1). 되돌리는 길: 이 창을 닫고 새 창에서 그 역할을 받으세요",
                    bound.label(),
                    caller.role.as_deref().unwrap_or("역할 없음")
                )));
            }
        }
    }
    match class {
        // O2: any caller — a seat, a human, the app, a scheduler child or a detached process.
        VerbClass::Read | VerbClass::Intake | VerbClass::Trigger => Ok(()),
        // F10 / J1: the master seat only (the owner-origin check is T4's).
        VerbClass::OwnerDecision => {
            if caller.seat().is_some() && caller.role.as_deref() == Some("master") {
                Ok(())
            } else {
                Err(VerbError::refused(
                    "주인의 결정을 전하는 명령은 마스터 좌석만 부를 수 있습니다",
                ))
            }
        }
        // No seat at all: the app's operator token (the OS verification is T5b's).
        VerbClass::OwnerApproval => {
            if caller.kind == CallerKind::Human && caller.app_token {
                Ok(())
            } else {
                Err(VerbError::refused(
                    "최종 승인은 앱의 카드에서 주인만 합니다 — 어떤 좌석·명령줄도 받지 않습니다",
                ))
            }
        }
        // B2: reviewer1 = codex slot (attest_verify), reviewer2 = gemini slot (attest_audit).
        VerbClass::Review => {
            let want = if sub == "attest_verify" {
                ReviewerSlot::Reviewer1
            } else {
                ReviewerSlot::Reviewer2
            };
            match (
                caller.seat(),
                caller.role.as_deref().and_then(reviewer_slot_of),
            ) {
                (Some(_), Some(slot)) if slot == want => Ok(()),
                _ => Err(VerbError::refused(match want {
                    ReviewerSlot::Reviewer1 => {
                        "검증 서명은 리뷰어1 좌석(reviewer-codex · reviewer-claude-2)만 씁니다"
                    }
                    ReviewerSlot::Reviewer2 => {
                        "감사 서명은 리뷰어2 좌석(reviewer-gemini · reviewer-claude-1)만 씁니다"
                    }
                })),
            }
        }
        // J1: master / CEO / CSO / reviewer seats are refused for every WORK verb; a worker seat of
        // this lane; `revert` / `cleanup` also a human (M4 per OS; the app button on both).
        VerbClass::Work => {
            if caller.is_worker() {
                return Ok(());
            }
            if matches!(sub, "revert" | "cleanup") && caller.seat().is_none() {
                // interpretation 36: the app's button (operator token) is human on both systems
                if caller.app_token {
                    return Ok(());
                }
                // M4 / AD2: on Windows a seatless command line is never human — every such caller
                // (the decision gives it `Detached`: no terminal is observed there) gets the
                // sentence that points to the app's button
                if windows {
                    return Err(VerbError::refused(WINDOWS_SEATLESS_REFUSAL_KO));
                }
                if caller.kind == CallerKind::Human {
                    return Ok(());
                }
            }
            Err(VerbError::refused(match caller.kind {
                CallerKind::Seat { .. } => "수리 작업 명령은 워커 좌석만 부를 수 있습니다(마스터·CEO·CSO·리뷰어는 거부)",
                CallerKind::Human => "이 명령은 사람이 직접 부를 수 없습니다 — 워커에게 맡기세요",
                CallerKind::Detached => "좌석도 터미널도 없는 호출자는 수리 작업을 할 수 없습니다(예약 작업·분리된 프로세스)",
            }))
        }
    }
}

/// The caller's kind and role from what the daemon observes (M1 / M4 / P5 / interpretation 36).
fn caller_of(
    daemon: &Daemon,
    caller_sid: Option<u64>,
    owner_grade: bool,
    caller_pid: Option<u32>,
) -> Caller {
    if let Some(sid) = caller_sid {
        let role = daemon
            .get_surface(sid)
            .and_then(|s| s.role.lock().unwrap_or_else(|e| e.into_inner()).clone());
        let lane = LaneKey::of_socket(&daemon.socket_path).0;
        return seat_caller(&lane, &daemon.incarnation, sid, role);
    }
    let obs = caller_pid
        .map(|pid| observe_seatless(daemon, pid))
        .unwrap_or_default();
    seatless_caller(owner_grade, &obs)
}

/// A seat caller with its AD1 binding: the existing binding, else (a seat that never claimed — its
/// current role is the first it held) the current role's family is bound now.
pub(crate) fn seat_caller(lane: &str, incarnation: &str, sid: u64, role: Option<String>) -> Caller {
    let bound_family = bind_role_family(lane, incarnation, sid, role.as_deref());
    Caller {
        kind: CallerKind::Seat { sid },
        role,
        app_token: false,
        bound_family,
    }
}

/// A seatless caller from what the daemon observed (the real decision of `caller_of`).
pub(crate) fn seatless_caller(owner_grade: bool, obs: &SeatlessObservation) -> Caller {
    Caller {
        kind: decide_seatless(owner_grade, obs),
        role: None,
        app_token: owner_grade,
        bound_family: None,
    }
}

/// What the daemon can observe about a caller that belongs to no seat (macOS; elsewhere nothing).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct SeatlessObservation {
    /// The caller has a controlling terminal.
    pub has_terminal: bool,
    /// That terminal is one of this daemon's panes (should not happen once the walk's terminal
    /// fallback ran — re-checked).
    pub terminal_is_pane: bool,
    /// Some ancestor is a `cysd` of any lane, or this daemon itself (P5 / N2).
    pub cysd_ancestor: bool,
}

/// RULINGS M4 / P5 / interpretation 36 for a seatless caller: the app's operator token → human
/// (both systems); a controlling terminal that is no pane's and no `cysd` ancestor → human (macOS
/// only — on Windows nothing is observed, so the terminal test never passes); else detached.
pub(crate) fn decide_seatless(owner_grade: bool, o: &SeatlessObservation) -> CallerKind {
    if owner_grade {
        return CallerKind::Human;
    }
    if o.has_terminal && !o.terminal_is_pane && !o.cysd_ancestor {
        CallerKind::Human
    } else {
        CallerKind::Detached
    }
}

#[cfg(target_os = "macos")]
fn observe_seatless(daemon: &Daemon, pid: u32) -> SeatlessObservation {
    let Some(brief) = crate::state::proc_brief(pid) else {
        return SeatlessObservation::default();
    };
    let Some(tdev) = brief.tdev else {
        return SeatlessObservation::default();
    };
    SeatlessObservation {
        has_terminal: true,
        terminal_is_pane: seat_of_terminal(daemon, tdev).is_some(),
        cysd_ancestor: has_cysd_ancestor(
            pid,
            std::process::id(),
            crate::state::proc_brief,
            proc_name_of,
        ),
    }
}

#[cfg(not(target_os = "macos"))]
fn observe_seatless(_daemon: &Daemon, _pid: u32) -> SeatlessObservation {
    SeatlessObservation::default()
}

/// P5 / N2: is some ancestor (up to 32 hops, stopping at pid ≤ 1) a `cysd` process of any lane or
/// this daemon itself? The caller itself is not counted.
#[cfg(target_os = "macos")]
pub(crate) fn has_cysd_ancestor(
    pid: u32,
    self_pid: u32,
    mut brief: impl FnMut(u32) -> Option<crate::state::ProcBrief>,
    mut name: impl FnMut(u32) -> Option<String>,
) -> bool {
    let mut cur = pid;
    for _ in 0..32 {
        let Some(parent) = brief(cur).and_then(|b| b.parent) else {
            return false;
        };
        if parent <= 1 || parent == cur {
            return false;
        }
        if parent == self_pid || name(parent).is_some_and(|n| n == "cysd") {
            return true;
        }
        cur = parent;
    }
    false
}

/// The short process name (`proc_name`) — macOS.
#[cfg(target_os = "macos")]
fn proc_name_of(pid: u32) -> Option<String> {
    let cpid = libc::c_int::try_from(pid).ok()?;
    let mut buf = [0u8; 256];
    // SAFETY: the buffer length is passed as the size; the kernel writes at most that many bytes.
    let n = unsafe {
        libc::proc_name(
            cpid,
            buf.as_mut_ptr() as *mut libc::c_void,
            buf.len() as u32,
        )
    };
    if n <= 0 {
        return None;
    }
    let n = usize::try_from(n).ok()?.min(buf.len());
    std::str::from_utf8(buf.get(..n)?).ok().map(str::to_string)
}

/// The slave device (`st_rdev`) of a pane's PTY master — `TIOCPTYGNAME` then `stat` (macOS).
#[cfg(target_os = "macos")]
pub(crate) fn pty_slave_rdev(master_fd: i32) -> Option<u32> {
    if master_fd < 0 {
        return None;
    }
    let mut name = [0 as libc::c_char; 128];
    // SAFETY: TIOCPTYGNAME writes a NUL-terminated path of at most 128 bytes into the buffer.
    if unsafe { libc::ioctl(master_fd, libc::TIOCPTYGNAME as _, name.as_mut_ptr()) } != 0 {
        return None;
    }
    if !name.contains(&0) {
        return None;
    }
    // SAFETY: the buffer holds a NUL (checked above); zeroed stat is a valid out-parameter.
    unsafe {
        let mut st: libc::stat = std::mem::zeroed();
        if libc::stat(name.as_ptr(), &mut st) != 0 {
            return None;
        }
        Some(st.st_rdev as u32)
    }
}

/// M4 / interpretation 32: the seat whose pane is the terminal `tdev` (macOS). Called by
/// `handlers::walk_caller_ancestry` when the ancestry walk found no pane, so every caller of the
/// resolver sees the extension.
#[cfg(target_os = "macos")]
pub(crate) fn seat_of_terminal(daemon: &Daemon, tdev: u32) -> Option<u64> {
    use std::sync::atomic::Ordering;
    let panes: Vec<(u64, i32)> = daemon
        .surfaces
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .values()
        .map(|s| (s.id, s.pty_master_fd.load(Ordering::Relaxed)))
        .collect();
    seat_by_terminal(&panes, tdev, pty_slave_rdev)
}

/// Pure part of [`seat_of_terminal`]: the first pane whose slave device equals `tdev`.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))] // only the macOS fallback calls it (and the tests)
pub(crate) fn seat_by_terminal(
    panes: &[(u64, i32)],
    tdev: u32,
    mut rdev_of: impl FnMut(i32) -> Option<u32>,
) -> Option<u64> {
    panes
        .iter()
        .find(|(_, fd)| rdev_of(*fd) == Some(tdev))
        .map(|(sid, _)| *sid)
}

/// The ledger root. Production: `~/.cys/local/repairs` (`paths::repairs_root`). In this binary's
/// own unit tests a per-process temporary root, so a test that goes through `handle()` never reads
/// or writes a real ledger.
fn production_root(daemon: &Daemon) -> PathBuf {
    #[cfg(test)]
    {
        return std::env::temp_dir()
            .join(format!("cys-repair-cysd-test-{}", std::process::id()))
            .join(LaneKey::of_socket(&daemon.socket_path).0);
    }
    #[cfg(not(test))]
    {
        let _ = daemon;
        cys::repair::paths::repairs_root()
    }
}

fn live_seats(daemon: &Daemon) -> Vec<(u64, String)> {
    let surfaces: Vec<Arc<crate::state::Surface>> = daemon
        .surfaces
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .values()
        .cloned()
        .collect();
    let mut out: Vec<(u64, String)> = surfaces
        .iter()
        .filter_map(|s| {
            let role = s.role.lock().unwrap_or_else(|e| e.into_inner()).clone()?;
            Some((s.id, role))
        })
        .collect();
    out.sort();
    out
}

/// Classification of one target from the compiled boot-critical list (C4 · T5a): listed with
/// category `protection` → 보호장치 관련, listed → 기동 필수, not listed → 일반. Undecidable or the
/// list not available → an error: `begin` refuses (a boot-critical file is never repaired as a
/// general one). Rule documents are T4's gate.
fn production_classify(rel: &str) -> Result<cys::repair::types::Classification, RepairError> {
    use cys::repair::boot_critical::{entries, is_boot_critical, Decision, EntryCategory};
    use cys::repair::types::Classification;
    if let Some(e) = entries().iter().find(|e| e.rel == rel) {
        return Ok(if e.category == EntryCategory::Protection {
            Classification::ProtectionRelated
        } else {
            Classification::BootCritical
        });
    }
    match is_boot_critical(Path::new(rel))? {
        Decision::Yes => Ok(Classification::BootCritical),
        Decision::No => Ok(Classification::General),
        Decision::Undecidable => Err(RepairError::Precondition(format!(
            "{rel}: 기동 필수 여부를 가릴 수 없어 시작하지 않습니다"
        ))),
    }
}

/// The repair's check on a live file (T5b's runner `rehearsal::run_check`, and the built-in
/// reference check of the same kind for the K4 durations). Until T5b lands both are stubs and
/// every commit is refused with their sentence (fail closed — never commit without the check).
fn production_check(
    rel: &str,
    live: &Path,
    deadline: std::time::Duration,
) -> Result<CheckReport, RepairError> {
    use cys::repair::rehearsal::{run_check, run_reference_check, CheckRequest, CheckResult};
    let kind = check_kind_for(rel);
    let started = std::time::Instant::now();
    let r = run_check(&CheckRequest {
        entry_rel: rel.to_string(),
        scratch_root: live.parent().map(Path::to_path_buf).unwrap_or_default(),
        recorded_input: None,
        deadline,
    })?;
    let check_ms = started.elapsed().as_millis() as u64;
    let started = std::time::Instant::now();
    let reference = run_reference_check(kind, deadline)?;
    let reference_ms = started.elapsed().as_millis() as u64;
    let _ = reference;
    Ok(match r {
        CheckResult::Pass => CheckReport {
            passed: true,
            timed_out: false,
            failure: String::new(),
            check_ms,
            reference_kind: kind,
            reference_ms,
        },
        CheckResult::Fail(m) => CheckReport {
            passed: false,
            timed_out: false,
            failure: m,
            check_ms,
            reference_kind: kind,
            reference_ms,
        },
        CheckResult::Timeout => CheckReport {
            passed: false,
            timed_out: true,
            failure: "검사 시간 초과".into(),
            check_ms,
            reference_kind: kind,
            reference_ms,
        },
    })
}

/// The check kind of a target: its boot-critical entry's, else by file type.
pub(crate) fn check_kind_for(rel: &str) -> cys::repair::boot_critical::CheckKind {
    use cys::repair::boot_critical::{entries, CheckKind};
    if let Some(e) = entries().iter().find(|e| e.rel == rel) {
        return e.check;
    }
    match rel.rsplit('.').next() {
        Some("sh") => CheckKind::ShSyntax,
        Some("py") => CheckKind::PyCompile,
        Some("json") => CheckKind::JsonParse,
        _ => CheckKind::NonePrerehearsal,
    }
}

// Track T3 — the verb bodies (an inline module: the C8 dormancy census follows production file modules
// only from `main.rs`, and fails closed on a nested one — `c8_dormancy_proof_tests.rs`).
mod ledger_verbs {
    //! Track T3 — the bodies of `repair.report|begin|record|commit|attest_verify|attest_audit|list|status|
    //! cleanup` and the seat-reap lock release (C1, C2 2.3(a), C3; RULINGS B1, B2, G4, G6, G7, H8, J1, K1,
    //! L4, N1, O5, P2, P4, P6, R1, R3, R5, R7, T1, U2, U3, V1, V2, W3, Y1, Y2(c), Z1).
    //! Every read-modify-write of a repair's files runs under the per-repair mutex (`Ctx::locked`), and
    //! records / lane rows are written only for repairs of THIS lane / for this lane's own row (B1, K1).
    //! ★No panic paths (F4) and no queue / alert / bus traffic (F1) in this file.

    use super::{Caller, CheckReport, Ctx, VerbError};
    use cys::repair::boot_critical::{clamp_check_timeout, entries, EntryCategory, LIFELINE_HOOKS};
    use cys::repair::intake;
    use cys::repair::ledger::{
        self, file_targets, hook_lines, Attestation, ChangeKind, CheckTiming, FileChange,
        LaneRecord, PreImage, Probation, RegisteredLanes, RepairRecord, AUDIT_WINDOW_SECS,
        MAX_ROUNDS,
    };
    use cys::repair::lock::{self, LockView};
    use cys::repair::types::{
        fingerprint, Classification, LaneKey, LaneState, RepairError, RepairId, RepairState,
        TargetState, Trigger,
    };
    use serde_json::{json, Value};
    use std::collections::{HashMap, VecDeque};
    use std::path::{Path, PathBuf};
    use std::sync::{Mutex, OnceLock};

    /// Total time a `commit` may spend in the repair's checks, so that its reply always arrives inside
    /// the CLI's 40-s wait (`cys.rs:RPC_IDLE_TIMEOUT_SECS`; F2 — a seat inside a long synchronous command
    /// cannot be cleared). A check that hits its share of the budget is a FAILURE at commit (B3).
    pub(crate) const COMMIT_CHECK_BUDGET_SECS: u64 = 30;
    /// AD4: how a target committed as Absent is checked — by that state itself.
    pub(crate) const CHECK_ABSENT_STATE: &str = "absent-state";
    /// AD4: how a hook line is checked — the registrar's report inside `hook_lines_step`.
    pub(crate) const CHECK_HOOK_REGISTRAR: &str = "hook-registrar";
    /// Longest accepted note (`record` / `attest-*`), bytes.
    pub(crate) const MAX_NOTE_BYTES: usize = 4096;
    /// Methods kept in a record (oldest dropped first).
    pub(crate) const MAX_METHODS: usize = 50;
    /// Largest text pair a `record` diffs (bytes, each side).
    pub(crate) const MAX_DIFF_BYTES: usize = 256 * 1024;
    /// The worker record's verdict after a commit refused by the check (opens the next round).
    const CHECK_FAILED: &str = "check-failed";

    // ─────────────────────────────── small helpers ───────────────────────────────

    fn param_str<'a>(p: &'a Value, k: &str) -> Option<&'a str> {
        p.get(k).and_then(|v| v.as_str())
    }

    fn req_str<'a>(p: &'a Value, k: &str) -> Result<&'a str, VerbError> {
        param_str(p, k).ok_or_else(|| VerbError::invalid(format!("`{k}` 가 필요합니다")))
    }

    fn note_of(p: &Value) -> Result<String, VerbError> {
        let n = req_str(p, "note")?;
        if n.trim().is_empty() {
            return Err(VerbError::invalid("`note` 가 비어 있습니다"));
        }
        if n.len() > MAX_NOTE_BYTES {
            return Err(VerbError::invalid(format!(
                "`note` 는 {MAX_NOTE_BYTES}바이트 이하여야 합니다"
            )));
        }
        Ok(n.to_string())
    }

    fn repair_id(p: &Value) -> Result<RepairId, VerbError> {
        let raw = req_str(p, "repair")?;
        RepairId::parse(raw).map_err(|_| {
            VerbError::invalid(format!("수리 번호는 R-<숫자> 형식이어야 합니다: {raw:?}"))
        })
    }

    /// Loads a record: missing → `not_found`, unreadable → `repair_corrupt` (O-C1-6: every mutating
    /// verb on it refused with a clear sentence).
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
                    "{}: 장부 기록을 읽을 수 없어 이 수리에 대한 변경을 거부합니다 — {}",
                    id.as_str(),
                    VerbError::from(e).message
                ),
            )),
        }
    }

    /// B1: only the origin lane's daemon writes a record.
    fn require_origin(ctx: &Ctx<'_>, rec: &RepairRecord) -> Result<(), VerbError> {
        if rec.origin_lane != ctx.lane {
            return Err(VerbError::new(
                cys::repair::types::ERR_PRECONDITION,
                format!(
                    "{} 은 부서 {} 의 수리입니다 — 그 부서의 데몬에서 하세요(장부 파일마다 쓰는 데몬은 하나)",
                    rec.id.as_str(),
                    rec.origin_lane.0
                ),
            ));
        }
        Ok(())
    }

    fn write(ctx: &Ctx<'_>, rec: &RepairRecord) -> Result<(), VerbError> {
        ledger::write_record_in(&ctx.root, rec).map_err(VerbError::from)
    }

    fn live_path(ctx: &Ctx<'_>, rel: &str) -> PathBuf {
        ctx.pack_dir.join(rel)
    }

    fn is_approval_class(c: Classification) -> bool {
        matches!(
            c,
            Classification::BootCritical | Classification::ProtectionRelated
        )
    }

    /// The states a target is compared by for this repair (N1 / U2): the live pack files for a general
    /// repair, the fixed working copy (`candidate_states`) for a boot-critical / protection one.
    fn current_states(
        ctx: &Ctx<'_>,
        rec: &RepairRecord,
    ) -> Result<Vec<(String, TargetState)>, VerbError> {
        if is_approval_class(rec.classification) {
            return ledger::candidate_states_in(&ctx.root, &rec.id).map_err(VerbError::from);
        }
        let mut rels: Vec<&str> = file_targets(rec).map(|f| f.rel.as_str()).collect();
        rels.sort_unstable();
        rels.dedup();
        let mut out = Vec::new();
        for rel in rels {
            out.push((
                rel.to_string(),
                TargetState::read(&live_path(ctx, rel)).map_err(VerbError::from)?,
            ));
        }
        Ok(out)
    }

    fn canon_list(states: &[(String, TargetState)]) -> Vec<(String, String)> {
        states.iter().map(|(r, s)| (r.clone(), s.canon())).collect()
    }

    fn lane_label(s: LaneState) -> &'static str {
        s.label_ko()
    }

    /// The `lanes` map of a reply (O-C3-5 / K1): every registered lane; a lane without a row = 대기.
    fn lanes_map(ctx: &Ctx<'_>, rec: &RepairRecord) -> (Value, Option<bool>, String) {
        let (rows, broken) = ledger::lane_rows_in(&ctx.root, &rec.id);
        let mut map = serde_json::Map::new();
        let in_force = matches!(rec.state, RepairState::Probation | RepairState::InUse);
        // R7: a `resolves` repair is lane-local — its own lane only.
        let registered = if rec.resolves.is_some() {
            RegisteredLanes::Known(vec![rec.origin_lane.clone()])
        } else {
            ledger::registered_lanes_from(&ctx.registry)
        };
        let row_of = |l: &LaneKey| rows.iter().find(|r| r.lane == *l);
        let (complete, why) = match &registered {
            RegisteredLanes::Known(lanes) => {
                let mut all = true;
                for l in lanes {
                    let label = match row_of(l) {
                        Some(r) => {
                            // P2: a row counts only with the CURRENT result (or its own verified merge)
                            let current = r.merge.is_some() || r.applied == rec.committed;
                            if !(r.state == LaneState::Verified && current) {
                                all = false;
                            }
                            lane_label(if current { r.state } else { LaneState::Pending })
                        }
                        None => {
                            all = false;
                            LaneState::Pending.label_ko()
                        }
                    };
                    map.insert(l.0.clone(), json!(label));
                }
                if !broken.is_empty() {
                    all = false;
                }
                (
                    Some(all && in_force),
                    if in_force {
                        String::new()
                    } else {
                        "사용 중인 수리가 아님".into()
                    },
                )
            }
            RegisteredLanes::Unknown(why) => (
                None,
                format!("부서 등록부를 읽을 수 없어 전파 완료를 선언하지 않습니다: {why}"),
            ),
        };
        for r in &rows {
            map.entry(r.lane.0.clone())
                .or_insert_with(|| json!(lane_label(r.state)));
        }
        (Value::Object(map), complete, why)
    }

    fn reply(ctx: &Ctx<'_>, rec: &RepairRecord, extra: Value) -> Value {
        let (lanes, complete, why) = lanes_map(ctx, rec);
        let mut v = json!({
            "repair": rec.id.as_str(),
            "state": rec.state,
            "state_label": rec.state.label_ko(),
            "lanes": lanes,
            "attempt": rec.attempt,
            "max_rounds": MAX_ROUNDS,
            "classification": rec.classification,
            "propagation_complete": complete,
        });
        if !why.is_empty() {
            v["propagation_note"] = json!(why);
        }
        if let (Some(obj), Value::Object(extra)) = (v.as_object_mut(), extra) {
            obj.extend(extra);
        }
        v
    }

    /// G7(c) order for the lock holder's mutating verbs (`record`, `commit`): renew-write (only while
    /// `now < expires_at`) → the claim's `.closed` marker check → act. Returns the record carrying the
    /// renewed lease, already written.
    fn holder_renew(
        ctx: &Ctx<'_>,
        caller: &Caller,
        rec: &mut RepairRecord,
    ) -> Result<(), VerbError> {
        let sid = caller
            .seat()
            .ok_or_else(|| VerbError::refused("좌석이 없습니다"))?;
        let owner = ctx.owner(sid);
        match lock::view(rec.lock.as_ref(), &owner, ctx.now, &ctx.lane, &ctx.incarnation) {
            LockView::Mine => {}
            LockView::MineExpired => {
                return Err(VerbError::new(
                    cys::repair::types::ERR_LOCKED,
                    format!("{}: 잠금 임대가 만료됐습니다 — 다시 잡으세요(`cys repair begin --files … --trigger …`)", rec.id.as_str()),
                ))
            }
            LockView::HeldByOther(o, _) => {
                return Err(VerbError::new(
                    cys::repair::types::ERR_LOCKED,
                    format!("{}: 다른 좌석(부서 {} · 좌석 {})이 이 수리의 잠금을 쥐고 있습니다", rec.id.as_str(), o.lane.0, o.seat),
                ))
            }
            LockView::Free => {
                return Err(VerbError::refused(format!(
                    "{}: 이 수리의 잠금을 쥔 좌석만 부를 수 있습니다 — 먼저 `cys repair begin --files … --trigger …`",
                    rec.id.as_str()
                )))
            }
        }
        let Some(l) = rec.lock.as_ref() else {
            return Err(VerbError::refused("잠금이 없습니다"));
        };
        let renewed = lock::renewed(l, ctx.now).ok_or_else(|| {
            VerbError::new(cys::repair::types::ERR_LOCKED, "잠금 임대가 만료됐습니다")
        })?;
        rec.lock = Some(renewed);
        write(ctx, rec)?;
        if lock::claim_closed(&ctx.root, rec.lock.as_ref().and_then(|l| l.claim)) {
            return Err(VerbError::new(
                cys::repair::types::ERR_LOCKED,
                format!(
                    "{}: 이 잠금의 자동 수리 차례가 닫혔습니다 — 새 차례가 필요합니다",
                    rec.id.as_str()
                ),
            ));
        }
        Ok(())
    }

    // ─────────────────────────────── report (INTAKE) ───────────────────────────────

    /// Per (root, lane): times of the NEW rows written in the last hour, and the over-quota count.
    type IntakeQuota = Mutex<HashMap<(PathBuf, String), (VecDeque<f64>, u64)>>;

    fn intake_quota() -> &'static IntakeQuota {
        static Q: OnceLock<IntakeQuota> = OnceLock::new();
        Q.get_or_init(|| Mutex::new(HashMap::new()))
    }

    pub(crate) fn intake_over_quota(ctx: &Ctx<'_>) -> u64 {
        let q = intake_quota().lock().unwrap_or_else(|e| e.into_inner());
        q.get(&(ctx.root.clone(), ctx.lane.0.clone()))
            .map(|(_, n)| *n)
            .unwrap_or(0)
    }

    /// Repairs of any lane that are open (not reverted) and list a signature.
    fn joined_repair(ctx: &Ctx<'_>, sig: &str) -> Option<RepairId> {
        ledger::list_ids_in(&ctx.root).into_iter().find_map(|id| {
            match ledger::read_record_in(&ctx.root, &id) {
                Ok(Some(r))
                    if r.state != RepairState::Reverted
                        && r.intake_signatures.iter().any(|s| s == sig) =>
                {
                    Some(r.id)
                }
                _ => None,
            }
        })
    }

    /// `cys repair report --file <rel> --error <text>` — one ledger row per (lane, signature); DATA,
    /// never authority, never a message (F1): nothing is queued, pushed, published or alerted.
    pub(crate) fn report(ctx: &Ctx<'_>, caller: &Caller, p: &Value) -> Result<Value, VerbError> {
        let file = req_str(p, "file")?;
        let error = req_str(p, "error")?;
        if error.len() > intake::MAX_ERROR_BYTES * 4 {
            return Err(VerbError::invalid("`error` 가 너무 깁니다"));
        }
        let rep = intake::parse_report(file, error)
            .map_err(|e| VerbError::invalid(VerbError::from(e).message))?;
        let key = (ctx.root.clone(), ctx.lane.0.clone());
        let new_in_hour = {
            let mut q = intake_quota().lock().unwrap_or_else(|e| e.into_inner());
            let e = q.entry(key.clone()).or_default();
            while e.0.front().is_some_and(|t| ctx.now - *t > 3600.0) {
                e.0.pop_front();
            }
            e.0.len()
        };
        let label = caller.label();
        let outcome = ctx.locked(&format!("intake:{}", rep.signature), || {
            intake::record_in(
                &ctx.root,
                &ctx.lane,
                &rep,
                &label,
                ctx.now,
                new_in_hour,
                ctx.windows,
                error,
            )
        })?;
        let mut out = json!({
            "signature": rep.signature,
            "rel": rep.rel,
            "error_class": rep.error_class,
            "lane": ctx.lane.0,
        });
        match outcome {
            intake::Recorded::New(row) | intake::Recorded::Updated(row) => {
                if row.count == 1 {
                    let mut q = intake_quota().lock().unwrap_or_else(|e| e.into_inner());
                    q.entry(key).or_default().0.push_back(ctx.now);
                }
                out["accepted"] = json!(true);
                out["count"] = json!(row.count);
            }
            intake::Recorded::OverQuota { .. } => {
                let mut q = intake_quota().lock().unwrap_or_else(|e| e.into_inner());
                q.entry(key).or_default().1 += 1;
                out["accepted"] = json!(false);
                out["over_quota"] = json!(true);
            }
            intake::Recorded::Excluded { reason } => {
                out["accepted"] = json!(false);
                out["excluded"] = json!(reason);
            }
        }
        out["joined"] = json!(joined_repair(ctx, &rep.signature).map(|i| i.as_str().to_string()));
        Ok(out)
    }

    // ─────────────────────────────── begin (WORK) ───────────────────────────────

    /// A `--unregister-hook <event>:<script>` value.
    fn parse_hook(raw: &str) -> Result<(String, String), VerbError> {
        let (event, script) = raw.split_once(':').ok_or_else(|| {
            VerbError::invalid(format!(
                "--unregister-hook 은 <event>:<script> 형식입니다: {raw:?}"
            ))
        })?;
        let ok_event = !event.is_empty()
            && event.len() <= 64
            && event.chars().all(|c| c.is_ascii_alphanumeric());
        let ok_script = !script.is_empty()
            && script.len() <= 128
            && !script.contains('/')
            && !script.contains('\\')
            && script != "."
            && script != ".."
            && script
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'));
        if !ok_event || !ok_script {
            return Err(VerbError::invalid(format!(
                "--unregister-hook 값이 올바르지 않습니다: {raw:?}"
            )));
        }
        Ok((event.to_string(), script.to_string()))
    }

    /// RULINGS V1: only a non-lifeline (8.7), non-protection (D:941) hook; while the compiled list is
    /// not available a protection hook cannot be told apart → refused (fail closed).
    fn hook_allowed(script: &str) -> Result<(), VerbError> {
        if LIFELINE_HOOKS.contains(&script) {
            return Err(VerbError::new(
                cys::repair::types::ERR_PRECONDITION,
                format!("{script}: 생명줄 훅의 등록은 수리로 해제할 수 없습니다"),
            ));
        }
        let list = entries();
        if list.is_empty() {
            return Err(VerbError::new(
                cys::repair::types::ERR_PRECONDITION,
                format!("{script}: 기동 필수 목록을 읽을 수 없어 보호장치 훅인지 가릴 수 없습니다 — 거부"),
            ));
        }
        let rel = format!("hooks/{script}");
        if list
            .iter()
            .any(|e| e.rel == rel && e.category == EntryCategory::Protection)
        {
            return Err(VerbError::new(
                cys::repair::types::ERR_PRECONDITION,
                format!("{script}: 남기는 보호장치의 훅 등록은 해제할 수 없습니다"),
            ));
        }
        Ok(())
    }

    fn trigger_of(p: &Value) -> Result<Trigger, VerbError> {
        // AD10: required — no default (fail closed).
        let Some(t) = param_str(p, "trigger") else {
            return Err(VerbError::invalid(
                "--trigger 가 필요합니다(owner-order · grant · update 중 하나 — 기본값 없음)",
            ));
        };
        match t {
            "owner_order" | "owner-order" => Ok(Trigger::OwnerOrder),
            "grant" => Ok(Trigger::Grant),
            "update" => Ok(Trigger::Update),
            other => Err(VerbError::invalid(format!("알 수 없는 계기: {other:?}"))),
        }
    }

    /// (file targets, hook lines as (event, script)).
    type BeginTargets = (Vec<String>, Vec<(String, String)>);

    /// The declared targets of a `begin`, normalised and de-duplicated (order kept).
    fn begin_targets(p: &Value) -> Result<BeginTargets, VerbError> {
        let mut files = Vec::new();
        if let Some(v) = p.get("files") {
            let arr = v
                .as_array()
                .ok_or_else(|| VerbError::invalid("`files` 는 목록이어야 합니다"))?;
            for f in arr {
                let raw = f
                    .as_str()
                    .ok_or_else(|| VerbError::invalid("`files` 의 항목은 문자열이어야 합니다"))?;
                let rel = intake::normalize_rel(raw)
                    .map_err(|e| VerbError::invalid(VerbError::from(e).message))?;
                if rel.starts_with("settings:") {
                    return Err(VerbError::invalid(format!("{rel}: 파일 대상이 아닙니다")));
                }
                if !files.contains(&rel) {
                    files.push(rel);
                }
            }
        }
        let mut hooks = Vec::new();
        if let Some(v) = p.get("unregister_hook") {
            let arr = v
                .as_array()
                .ok_or_else(|| VerbError::invalid("`unregister_hook` 은 목록이어야 합니다"))?;
            for h in arr {
                let raw = h.as_str().ok_or_else(|| {
                    VerbError::invalid("`unregister_hook` 의 항목은 문자열이어야 합니다")
                })?;
                let pair = parse_hook(raw)?;
                if !hooks.contains(&pair) {
                    hooks.push(pair);
                }
            }
        }
        // RULINGS Z2(e): `--files` is required unless `--unregister-hook` is given.
        if files.is_empty() && hooks.is_empty() {
            return Err(VerbError::invalid(
                "`--files` 가 필요합니다(`--unregister-hook` 만 있는 일반 수리는 예외)",
            ));
        }
        Ok((files, hooks))
    }

    /// The set a pick-up is matched by: sorted file rels + hook lines.
    fn target_key(files: &[String], hooks: &[(String, String)]) -> Vec<String> {
        let mut k: Vec<String> = files.to_vec();
        k.extend(hooks.iter().map(|(e, s)| format!("settings:{e}:{s}")));
        k.sort();
        k
    }

    fn record_key(rec: &RepairRecord) -> Vec<String> {
        let mut k: Vec<String> = rec.files.iter().map(|f| f.rel.clone()).collect();
        k.sort();
        k.dedup();
        k
    }

    fn begin_reply(ctx: &Ctx<'_>, rec: &RepairRecord, picked: &str) -> Value {
        let staging = ledger::staging_dir_in(&ctx.root, &rec.id);
        let pre: Vec<Value> = file_targets(rec)
            .map(|f| {
                json!({
                    "rel": f.rel,
                    "pre": f.pre.state().canon(),
                    "slot": match &f.pre { PreImage::Slot { slot_dir, .. } => json!(slot_dir), PreImage::Absent => Value::Null },
                })
            })
            .collect();
        reply(
            ctx,
            rec,
            json!({
                "begin": picked,
                "trigger": rec.trigger,
                "resolves": rec.resolves.as_ref().map(|r| r.as_str().to_string()),
                "lock_expires_at": rec.lock.as_ref().map(|l| l.expires_at),
                // U3: the fixed working copy of a boot-critical / protection-related repair
                "staging": if is_approval_class(rec.classification) { json!(staging) } else { Value::Null },
                "pre_images": pre,
                "hook_lines": hook_lines(rec).map(|f| f.rel.clone()).collect::<Vec<_>>(),
                "intake_signatures": rec.intake_signatures,
            }),
        )
    }

    /// `cys repair begin --files <rel…> [--unregister-hook <event>:<script>]… [--trigger …]
    /// [--joins <R-id> | --resolves <R-id>]` (WORK — a worker seat of this lane; it writes the lock).
    pub(crate) fn begin(ctx: &Ctx<'_>, caller: &Caller, p: &Value) -> Result<Value, VerbError> {
        let sid = caller
            .seat()
            .ok_or_else(|| VerbError::refused("좌석이 없습니다"))?;
        let joins = param_str(p, "joins");
        let resolves = param_str(p, "resolves");
        if joins.is_some() && resolves.is_some() {
            return Err(VerbError::invalid(
                "--joins 와 --resolves 는 함께 쓸 수 없습니다",
            ));
        }
        if param_str(p, "owner_token").is_some() {
            // C10 10.6: the rule-document gate and the token binding are T4's hook into `begin`.
            return Err(RepairError::NotImplemented("begin --owner-token (규칙 문서 관문)").into());
        }
        let trigger = trigger_of(p)?;
        let (files, hooks) = begin_targets(p)?;
        if let Some(r) = joins {
            let target = RepairId::parse(r).map_err(|_| {
                VerbError::invalid(format!("--joins 값이 수리 번호가 아닙니다: {r:?}"))
            })?;
            return begin_joins(ctx, sid, &target, &files);
        }
        let resolves = match resolves {
            Some(r) => {
                let rid = RepairId::parse(r).map_err(|_| {
                    VerbError::invalid(format!("--resolves 값이 수리 번호가 아닙니다: {r:?}"))
                })?;
                load(ctx, &rid)?; // must exist (any lane); a reference only (RULINGS T1)
                Some(rid)
            }
            None => None,
        };
        // AK6: a Windows path alias (a part ending in a dot or a space, or containing `:`) opens the
        // file it aliases there but would escape the classification below — refused on every system.
        if let Some(rel) = files.iter().find(|r| super::is_windows_alias(r)) {
            return Err(VerbError::invalid(format!(
                "{rel:?}: 끝이 점·공백이거나 ':' 가 든 경로 조각은 받지 않습니다(윈도우에서 다른 파일의 별칭이 됩니다)"
            )));
        }
        // AD6 / C10 10.6: a rule document is refused until T4's owner-token gate is merged (fail closed).
        if let Some(rel) = files.iter().find(|r| super::is_rule_document(r)) {
            return Err(VerbError::new(
                cys::repair::types::ERR_NOT_IMPLEMENTED,
                format!(
                    "{} (규칙 문서 수리 — 주인 지시 표 관문 · C10): {rel}",
                    cys::repair::NOT_IMPLEMENTED_KO
                ),
            ));
        }
        // Classification (C4 · T5a) — refused when it cannot be decided (fail closed).
        let mut class = Classification::General;
        for rel in &files {
            match (ctx.classify)(rel)? {
                Classification::ProtectionRelated => class = Classification::ProtectionRelated,
                Classification::BootCritical if class != Classification::ProtectionRelated => {
                    class = Classification::BootCritical
                }
                Classification::DaemonSource => {
                    return Err(VerbError::new(
                        cys::repair::types::ERR_PRECONDITION,
                        format!("{rel}: 데몬 소스 수리는 2차 출시입니다"),
                    ))
                }
                _ => {}
            }
        }
        if !hooks.is_empty() && is_approval_class(class) {
            return Err(VerbError::new(
                cys::repair::types::ERR_PRECONDITION,
                "훅 등록 해제는 일반 수리입니다 — 기동 필수·보호장치 대상과 함께 시작하지 말고 두 수리로 나누세요(RULINGS V1)",
            ));
        }
        for (_, script) in &hooks {
            hook_allowed(script)?;
        }
        let owner = ctx.owner(sid);
        let key = target_key(&files, &hooks);
        // G6: every new start / pick-up of this daemon is serialised, so a CLI retry that arrives while
        // the first `begin` is still copying pre-images waits for it and then finds its own completed
        // start — never a second repair for the same targets.
        ctx.locked("begin:lane", || {
            begin_locked(ctx, &owner, class, trigger, &files, &hooks, resolves, &key)
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn begin_locked(
        ctx: &Ctx<'_>,
        owner: &cys::repair::types::LockOwner,
        class: Classification,
        trigger: Trigger,
        files: &[String],
        hooks: &[(String, String)],
        resolves: Option<RepairId>,
        key: &[String],
    ) -> Result<Value, VerbError> {
        // Pick-up (G1 step 3 / AD7): only UNFINISHED work — a repair of this lane with the same targets
        // that is `진행 중` or `감사 지적으로 다시 열림` and has no live lock. A repair in force
        // (`시험 기간` / `사용 중`) or in any other state is never picked up: a new `begin` starts a NEW
        // repair with its own pre-image.
        let candidates: Vec<RepairRecord> = ledger::list_ids_in(&ctx.root)
            .into_iter()
            .filter_map(|id| match ledger::read_record_in(&ctx.root, &id) {
                Ok(Some(r))
                    if r.origin_lane == ctx.lane
                        && r.resolves == resolves
                        && matches!(
                            r.state,
                            RepairState::InProgress | RepairState::ReopenedByAudit
                        )
                        && record_key(&r) == key =>
                {
                    Some(r)
                }
                _ => None,
            })
            .collect();
        let views: Vec<(RepairId, LockView)> = candidates
            .iter()
            .map(|r| {
                (
                    r.id.clone(),
                    lock::view(r.lock.as_ref(), owner, ctx.now, &ctx.lane, &ctx.incarnation),
                )
            })
            .collect();
        // G6: a retry of this seat's own start gets that start back.
        if let Some((id, _)) = views.iter().rev().find(|(_, v)| *v == LockView::Mine) {
            let rec = load(ctx, id)?;
            return Ok(begin_reply(ctx, &rec, "already-held"));
        }
        if let Some((id, _)) = views.iter().rev().find(|(_, v)| *v == LockView::Free) {
            return ctx.locked(id.as_str(), || -> Result<Value, VerbError> {
                let mut rec = load(ctx, id)?;
                // re-read under the repair's mutex: still unfinished and still free?
                let still = matches!(
                    rec.state,
                    RepairState::InProgress | RepairState::ReopenedByAudit
                ) && lock::view(
                    rec.lock.as_ref(),
                    owner,
                    ctx.now,
                    &ctx.lane,
                    &ctx.incarnation,
                ) == LockView::Free;
                if !still {
                    return Err(VerbError::new(
                        cys::repair::types::ERR_LOCKED,
                        format!(
                            "{}: 이 수리는 방금 다른 좌석이 잡았습니다 — 다시 시도하세요",
                            rec.id.as_str()
                        ),
                    ));
                }
                let mut claim = None;
                if rec.trigger == Trigger::Grant && rec.resolves.is_none() {
                    // K3: picking a grant repair up again needs a NEW claim (T4).
                    claim = claim_for(owner, &rec.id)?;
                }
                rec.lock = Some(lock::new_lease(owner.clone(), ctx.now, claim));
                // V2(b) / DEFERRED_MINOR #6: pre-images and staging of the first begin are never rewritten.
                write(ctx, &rec)?;
                Ok(begin_reply(ctx, &rec, "picked-up"))
            });
        }
        if let Some((id, _)) = views.last() {
            // every unfinished repair of these targets is held by a live lock of another seat
            return Err(VerbError::new(
                cys::repair::types::ERR_LOCKED,
                format!(
                    "{}: 이 수리는 이미 잠겨 있습니다(살아 있는 잠금)",
                    id.as_str()
                ),
            ));
        }
        // A new repair — created ALREADY LOCKED through the ledger's seam (AD9 S1 / G1 "lock first"):
        // the id is allocated, the record is built (claim, pre-images, staging) and written with its
        // lock before anything else can see it; a failure removes the still-empty id directory.
        let id = ledger::create_locked_in(&ctx.root, |id| -> Result<RepairRecord, VerbError> {
            let id = id.clone();
            let claim = if trigger == Trigger::Grant && resolves.is_none() {
                claim_for(owner, &id)?
            } else {
                None
            };
            let mut changes = Vec::new();
            for rel in files {
                let live = live_path(ctx, rel);
                let pre = ledger::copy_preimage_in(&ctx.root, &id, &ctx.lane, rel, &live, ctx.now)?;
                let post = pre.state();
                changes.push(FileChange {
                    rel: rel.clone(),
                    kind: if pre == PreImage::Absent {
                        ChangeKind::Create
                    } else {
                        ChangeKind::Edit
                    },
                    pre,
                    post,
                    diff: None,
                });
            }
            for (event, script) in hooks {
                // W3(b): the hook line's shape.
                changes.push(FileChange {
                    rel: format!("settings:{event}:{script}"),
                    kind: ChangeKind::HookUnregister {
                        event: event.clone(),
                        script: script.clone(),
                    },
                    pre: PreImage::Absent,
                    post: TargetState::Absent,
                    diff: None,
                });
            }
            if is_approval_class(class) {
                let targets: Vec<(String, PathBuf)> = files
                    .iter()
                    .map(|r| (r.clone(), live_path(ctx, r)))
                    .collect();
                ledger::populate_staging_in(&ctx.root, &id, &targets)?;
            }
            let (defects, _) = intake::load_all_in(&ctx.root);
            let joined: Vec<intake::Defect> = defects
                .into_iter()
                .filter(|d| files.contains(&d.rel))
                .collect();
            // C1 1.2 "증상": the reported symptom(s) the repair takes in (the cause is the worker's record).
            let symptom = joined
                .iter()
                .map(|d| format!("{}: {}", d.rel, d.first_line))
                .collect::<Vec<_>>()
                .join(" / ");
            let sigs: Vec<String> = joined.into_iter().map(|d| d.signature).collect();
            let rec = RepairRecord {
                schema: cys::repair::REPAIR_SCHEMA,
                id: id.clone(),
                created_at: ctx.now,
                trigger,
                symptom: symptom.chars().take(1000).collect(),
                cause: String::new(),
                classification: class,
                files: changes,
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
                lock: Some(lock::new_lease(owner.clone(), ctx.now, claim)),
                state: RepairState::InProgress,
                intake_signatures: sigs,
                owner_original: false,
                origin_lane: ctx.lane.clone(),
                resolves: resolves.clone(),
                committed: Vec::new(),
                fixer_seats: Vec::new(),
                checks: Vec::new(),
            };
            Ok(rec)
        })?;
        let rec = load(ctx, &id)?;
        Ok(begin_reply(ctx, &rec, "new"))
    }

    /// K8 / G1 (T4): the grant claim for an autonomous start. Until T4 lands the stub answers
    /// not-implemented and the start is refused.
    fn claim_for(
        owner: &cys::repair::types::LockOwner,
        id: &RepairId,
    ) -> Result<Option<u32>, VerbError> {
        use cys::repair::grant::{try_claim_autonomous_start, StartVerdict};
        match try_claim_autonomous_start(owner, id)? {
            StartVerdict::Allowed { claim } => Ok(Some(claim)),
            StartVerdict::Paused => Err(VerbError::new(
                cys::repair::types::ERR_PRECONDITION,
                "자동 수리 허락이 일시정지 중입니다",
            )),
            StartVerdict::Busy => Err(VerbError::new(
                cys::repair::types::ERR_LOCKED,
                "다른 자동 수리가 진행 중입니다(동시 1건)",
            )),
            StartVerdict::CapReached => Err(VerbError::new(
                cys::repair::types::ERR_PRECONDITION,
                "오늘의 자동 수리 한도에 이르렀습니다",
            )),
            StartVerdict::LostRace => Err(VerbError::new(
                cys::repair::types::ERR_LOCKED,
                "다른 데몬이 먼저 차례를 잡았습니다 — 나중에 다시",
            )),
        }
    }

    /// H8 MERGE MODE in this lane (general paths only — RULINGS R1 / T1: refused on boot-critical /
    /// protection-related paths, where `--resolves` makes an ordinary repair instead). Writes only this
    /// lane's row (interpretation 18): the merge lock, the attempt and `merge_base`; the current files
    /// are copied into fresh vault slots (H8).
    fn begin_joins(
        ctx: &Ctx<'_>,
        sid: u64,
        target: &RepairId,
        files: &[String],
    ) -> Result<Value, VerbError> {
        let rec = load(ctx, target)?;
        let rels: Vec<String> = file_targets(&rec).map(|f| f.rel.clone()).collect();
        if let Some(bad) = files.iter().find(|f| !rels.contains(f)) {
            return Err(VerbError::invalid(format!(
                "{bad}: {} 의 대상 파일이 아닙니다",
                target.as_str()
            )));
        }
        let mut boot_like = is_approval_class(rec.classification);
        for rel in &rels {
            match (ctx.classify)(rel) {
                Ok(c) if is_approval_class(c) => boot_like = true,
                Ok(_) => {}
                Err(e) => return Err(e.into()),
            }
        }
        if boot_like {
            return Err(VerbError::new(
                cys::repair::types::ERR_PRECONDITION,
                format!(
                    "{}: 기동 필수·보호장치 경로는 합치기 모드로 직접 고치지 않습니다 — `cys repair begin --files … --trigger … --resolves {}` 로 이 부서의 수리를 따로 시작하세요(RULINGS R1/T1)",
                    target.as_str(),
                    target.as_str()
                ),
            ));
        }
        let owner = ctx.owner(sid);
        ctx.locked(target.as_str(), || -> Result<Value, VerbError> {
            let mut row = match ledger::read_lane_in(&ctx.root, target, &ctx.lane) {
                Ok(Some(r)) => r,
                Ok(None) => {
                    return Err(VerbError::new(
                        cys::repair::types::ERR_PRECONDITION,
                        format!("{}: 이 부서에는 합칠 충돌이 없습니다", target.as_str()),
                    ))
                }
                Err(e) => return Err(e.into()),
            };
            if row.state != LaneState::Conflict {
                return Err(VerbError::new(
                    cys::repair::types::ERR_PRECONDITION,
                    format!(
                        "{}: 이 부서의 상태가 '충돌'이 아닙니다({})",
                        target.as_str(),
                        row.state.label_ko()
                    ),
                ));
            }
            match lock::view(
                row.merge_lock.as_ref(),
                &owner,
                ctx.now,
                &ctx.lane,
                &ctx.incarnation,
            ) {
                LockView::Mine => {}
                LockView::Free => {
                    let mut base = Vec::new();
                    for rel in &rels {
                        let live = live_path(ctx, rel);
                        base.push((rel.clone(), TargetState::read(&live)?.canon()));
                        ledger::copy_preimage_in(
                            &ctx.root, target, &ctx.lane, rel, &live, ctx.now,
                        )?;
                    }
                    row.merge_lock = Some(lock::new_lease(owner.clone(), ctx.now, None));
                    row.merge_attempt = row.merge_attempt.saturating_add(1);
                    row.merge_base = base;
                    row.updated_at = ctx.now;
                    ledger::write_lane_in(&ctx.root, &row)?;
                }
                LockView::MineExpired | LockView::HeldByOther(..) => {
                    return Err(VerbError::new(
                        cys::repair::types::ERR_LOCKED,
                        format!(
                            "{}: 이 부서의 합치기 잠금을 다른 좌석이 쥐고 있습니다",
                            target.as_str()
                        ),
                    ))
                }
            }
            Ok(json!({
                "repair": target.as_str(),
                "lane": ctx.lane.0,
                "begin": "joins",
                "lane_state": row.state,
                "merge_attempt": row.merge_attempt,
                "merge_base": row.merge_base,
                "merge_lock_expires_at": row.merge_lock.as_ref().map(|l| l.expires_at),
            }))
        })
    }

    // ─────────────────────────────── record (WORK, lock holder) ───────────────────────────────

    fn slot_payload(pre: &PreImage, rel: &str) -> Option<PathBuf> {
        match pre {
            PreImage::Slot {
                slot_dir,
                link_target: None,
                ..
            } => Some(
                slot_dir
                    .join("payload")
                    .join(rel.rsplit('/').next().unwrap_or(rel)),
            ),
            _ => None,
        }
    }

    fn read_text_bounded(p: Option<&Path>) -> Option<String> {
        let Some(p) = p else {
            return Some(String::new());
        };
        let meta = std::fs::symlink_metadata(p).ok()?;
        if !meta.is_file() || meta.len() as usize > MAX_DIFF_BYTES {
            return None;
        }
        String::from_utf8(std::fs::read(p).ok()?).ok()
    }

    /// The per-file diff against the pre-image (C1 1.2) for text files up to `MAX_DIFF_BYTES`.
    fn diff_of(pre: &PreImage, rel: &str, post_path: &Path, post: &TargetState) -> Option<String> {
        let old_path = slot_payload(pre, rel);
        if matches!(
            pre,
            PreImage::Slot {
                link_target: Some(_),
                ..
            }
        ) || matches!(post, TargetState::Link { .. })
        {
            return None;
        }
        let old = read_text_bounded(old_path.as_deref())?;
        let new = match post {
            TargetState::Absent => String::new(),
            _ => read_text_bounded(Some(post_path))?,
        };
        if old == new {
            return Some(String::new());
        }
        Some(diffy::create_patch(&old, &new).to_string())
    }

    fn kind_of(pre: &TargetState, post: &TargetState) -> ChangeKind {
        match (pre, post) {
            (TargetState::Absent, TargetState::Absent) => ChangeKind::Edit,
            (TargetState::Absent, _) => ChangeKind::Create,
            (_, TargetState::Absent) => ChangeKind::Delete,
            (TargetState::Link { .. }, _) | (_, TargetState::Link { .. }) => ChangeKind::Link,
            (TargetState::File { sha256: a, .. }, TargetState::File { sha256: b, .. })
                if a == b =>
            {
                ChangeKind::Mode
            }
            _ => ChangeKind::Edit,
        }
    }

    /// `cys repair record <R-id> --note <text>` — the diff, the attempt record and the post-image STATES
    /// (C1 1.8 / U2). Voids both attestations (N1). A record after a failed verification, an audit finding
    /// or a commit refused by the check opens the next round (attempt + 1, max 10 — C1 1.11).
    pub(crate) fn record(ctx: &Ctx<'_>, caller: &Caller, p: &Value) -> Result<Value, VerbError> {
        let id = repair_id(p)?;
        let note = note_of(p)?;
        ctx.locked(id.as_str(), || -> Result<Value, VerbError> {
            let mut rec = load(ctx, &id)?;
            require_origin(ctx, &rec)?;
            holder_renew(ctx, caller, &mut rec)?;
            match rec.state {
                RepairState::InProgress | RepairState::ReopenedByAudit => {}
                RepairState::Probation | RepairState::InUse => {
                    return Err(VerbError::new(
                        cys::repair::types::ERR_PRECONDITION,
                        format!("{}: 이미 적용된 수리입니다 — 더 고치려면 `cys repair begin --files … --trigger …` 로 새 수리를 시작하세요(RULINGS AD7)", id.as_str()),
                    ))
                }
                other => {
                    return Err(VerbError::new(
                        cys::repair::types::ERR_PRECONDITION,
                        format!("{}: '{}' 상태에서는 기록할 수 없습니다", id.as_str(), other.label_ko()),
                    ))
                }
            }
            let new_round = rec.state == RepairState::ReopenedByAudit
                || rec.verify.as_ref().is_some_and(|v| v.verdict == "fail")
                || rec.worker_record.as_ref().is_some_and(|w| w.verdict == CHECK_FAILED);
            if new_round {
                if rec.attempt >= MAX_ROUNDS {
                    return Err(VerbError::new(
                        cys::repair::types::ERR_PRECONDITION,
                        format!("{}: 라운드 상한 {MAX_ROUNDS}회에 이르렀습니다 — 마스터가 주인께 격차를 보고합니다", id.as_str()),
                    ));
                }
                if rec.trigger == Trigger::Grant && rec.resolves.is_none() {
                    // B4 / K3: a new round of a grant-triggered repair asks T4 (paused → refused).
                    if !cys::repair::grant::may_open_new_round(&id)? {
                        return Err(VerbError::new(cys::repair::types::ERR_PRECONDITION, "자동 수리 허락이 일시정지 중이라 새 라운드를 열 수 없습니다"));
                    }
                }
                rec.attempt += 1;
            }
            let states = current_states(ctx, &rec)?;
            let staging = ledger::staging_dir_in(&ctx.root, &id);
            let approval_class = is_approval_class(rec.classification);
            for f in rec.files.iter_mut() {
                if matches!(f.kind, ChangeKind::HookUnregister { .. }) {
                    continue;
                }
                let Some((_, post)) = states.iter().find(|(r, _)| *r == f.rel) else { continue };
                let post_path = if approval_class { staging.join(&f.rel) } else { ctx.pack_dir.join(&f.rel) };
                f.kind = kind_of(&f.pre.state(), post);
                f.diff = diff_of(&f.pre, &f.rel, &post_path, post);
                f.post = post.clone();
            }
            let voided = rec.verify.is_some() || rec.audit.is_some();
            let sid = caller.seat().unwrap_or(0);
            // AD1: every seat that recorded in any round stays a fixer of this repair.
            if !rec.fixer_seats.contains(&sid) {
                rec.fixer_seats.push(sid);
            }
            rec.worker_record = Some(Attestation {
                seat: sid,
                role: caller.role.clone().unwrap_or_default(),
                verdict: "recorded".into(),
                note: note.clone(),
                at: ctx.now,
                fingerprint_sha256: fingerprint(&states),
            });
            rec.verify = None;
            rec.audit = None;
            rec.methods.push(note.chars().take(200).collect());
            while rec.methods.len() > MAX_METHODS {
                rec.methods.remove(0);
            }
            if rec.state == RepairState::ReopenedByAudit {
                rec.state = RepairState::InProgress;
            }
            write(ctx, &rec)?;
            let files: Vec<Value> = file_targets(&rec)
                .map(|f| json!({"rel": f.rel, "kind": f.kind, "post": f.post.canon(), "diff_lines": f.diff.as_ref().map(|d| d.lines().count())}))
                .collect();
            Ok(reply(ctx, &rec, json!({
                "fingerprint": fingerprint(&states),
                "files": files,
                "new_round": new_round,
                "attestations_voided": voided,
            })))
        })
    }

    // ─────────────────────────────── attest-verify / attest-audit (REVIEW) ───────────────────────────────

    /// `attest-verify` (reviewer1) / `attest-audit` (reviewer2): writes the signature only, with the
    /// fingerprint of exactly what was examined (N1); never on one's own repair, never both by one seat
    /// (B2). A general repair is audited after commit; an audit finding — or committed bytes that differ
    /// from what reviewer1 verified — re-opens it (C1 1.10). Boot-critical / protection-related: both
    /// signatures before submit, over the staged candidate.
    pub(crate) fn attest(
        ctx: &Ctx<'_>,
        caller: &Caller,
        p: &Value,
        slot: cys::repair::roles::ReviewerSlot,
    ) -> Result<Value, VerbError> {
        use cys::repair::roles::ReviewerSlot;
        let id = repair_id(p)?;
        let note = note_of(p)?;
        let verdict = req_str(p, "verdict")?.to_string();
        let allowed: &[&str] = match slot {
            ReviewerSlot::Reviewer1 => &["pass", "fail"],
            ReviewerSlot::Reviewer2 => &["ok", "finding"],
        };
        if !allowed.contains(&verdict.as_str()) {
            return Err(VerbError::invalid(format!(
                "판정은 {} 중 하나여야 합니다",
                allowed.join("|")
            )));
        }
        let sid = caller
            .seat()
            .ok_or_else(|| VerbError::refused("좌석이 없습니다"))?;
        ctx.locked(id.as_str(), || -> Result<Value, VerbError> {
            let mut rec = load(ctx, &id)?;
            require_origin(ctx, &rec)?;
            let worker = rec.worker_record.clone().ok_or_else(|| {
                VerbError::new(cys::repair::types::ERR_PRECONDITION, format!("{}: 워커의 기록(`record`)이 아직 없습니다", id.as_str()))
            })?;
            // B2 / AD1: never the fixer — any seat that recorded in ANY round, the last recorder, or
            // the seat holding the repair's lock.
            let fixer = rec.fixer_seats.contains(&sid)
                || worker.seat == sid
                || rec.lock.as_ref().is_some_and(|l| l.owner.lane == ctx.lane && l.owner.seat == sid);
            if fixer {
                return Err(VerbError::refused(format!("{}: 자기가 고친 수리에는 서명할 수 없습니다", id.as_str())));
            }
            let other = match slot {
                ReviewerSlot::Reviewer1 => rec.audit.as_ref(),
                ReviewerSlot::Reviewer2 => rec.verify.as_ref(),
            };
            if other.is_some_and(|a| a.seat == sid) {
                return Err(VerbError::refused(format!("{}: 검증과 감사를 한 좌석이 둘 다 할 수 없습니다", id.as_str())));
            }
            if rec.state == RepairState::Superseded {
                // AK4: a superseded repair is never reopened by an audit (the newer repair is audited).
                return Err(VerbError::new(
                    cys::repair::types::ERR_PRECONDITION,
                    format!("{}: 다른 수리로 대체된 수리는 서명하지 않습니다 — 그 파일의 최신 수리를 감사하세요", id.as_str()),
                ));
            }
            let approval_class = is_approval_class(rec.classification);
            let committed_general = !approval_class && matches!(rec.state, RepairState::Probation | RepairState::InUse);
            match slot {
                ReviewerSlot::Reviewer1 => {
                    if rec.state != RepairState::InProgress {
                        return Err(VerbError::new(
                            cys::repair::types::ERR_PRECONDITION,
                            format!("{}: '{}' 상태에서는 검증 서명을 받지 않습니다", id.as_str(), rec.state.label_ko()),
                        ));
                    }
                }
                ReviewerSlot::Reviewer2 => {
                    if !approval_class && !committed_general {
                        return Err(VerbError::new(
                            cys::repair::types::ERR_PRECONDITION,
                            format!("{}: 일반 수리의 감사는 확정 뒤에 합니다(C1 1.10)", id.as_str()),
                        ));
                    }
                    if approval_class && rec.state != RepairState::InProgress {
                        return Err(VerbError::new(
                            cys::repair::types::ERR_PRECONDITION,
                            format!("{}: 기동 필수·보호장치 수리의 감사는 제출 전에 합니다", id.as_str()),
                        ));
                    }
                }
            }
            let states = current_states(ctx, &rec)?;
            let fp = fingerprint(&states);
            let mut reopened = None;
            if committed_general {
                // N1: an audit that finds the committed bytes different from what was verified reopens.
                let verified = rec.verify.as_ref().map(|v| v.fingerprint_sha256.clone()).unwrap_or_default();
                if fp != verified {
                    reopened = Some("확정된 바이트가 검증 때와 다릅니다");
                }
            } else if fp != worker.fingerprint_sha256 {
                return Err(VerbError::new(
                    "target_changed",
                    format!("{}: 워커의 마지막 기록 뒤에 대상이 바뀌었습니다 — 워커가 다시 `record` 한 뒤 서명하세요", id.as_str()),
                ));
            }
            if slot == ReviewerSlot::Reviewer2 && verdict == "finding" {
                reopened = Some("감사 지적");
            }
            let att = Attestation {
                seat: sid,
                role: caller.role.clone().unwrap_or_default(),
                verdict: verdict.clone(),
                note,
                at: ctx.now,
                fingerprint_sha256: fp.clone(),
            };
            match slot {
                ReviewerSlot::Reviewer1 => rec.verify = Some(att),
                ReviewerSlot::Reviewer2 => rec.audit = Some(att),
            }
            if reopened.is_some() {
                rec.state = RepairState::ReopenedByAudit;
            }
            write(ctx, &rec)?;
            Ok(reply(ctx, &rec, json!({
                "signed": match slot { ReviewerSlot::Reviewer1 => "verify", ReviewerSlot::Reviewer2 => "audit" },
                "verdict": verdict,
                "fingerprint": fp,
                "reopened": reopened,
            })))
        })
    }

    // ─────────────────────────────── commit (WORK, lock holder) ───────────────────────────────

    fn pack_generation(pack: &Path) -> (String, Option<u32>) {
        let version = std::fs::read_to_string(pack.join(".pack-version"))
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|_| "unknown".to_string());
        let pro = match cys::pack::read_pack_state(pack) {
            cys::pack::PackStateRead::Valid(st) if st.channel == "pro" => Some(st.pro_revision),
            _ => None,
        };
        (version, pro)
    }

    /// What the commit's checks produced: the timings of the checks that ran (K4) and, per target,
    /// how it was checked (AD4 — `RepairRecord.checks`).
    type Checked = (Vec<CheckTiming>, Vec<(String, String)>);

    /// Runs the repair's check on every live target inside the commit budget (B3: a timeout at commit
    /// is a failure). A target whose committed state is Absent is checked by that state itself (the
    /// path is absent — `absent-state`, AD4; `current_states` read it so). `Err` = the runner itself
    /// is not available (T5b) → the commit is refused. Called WITHOUT the per-repair mutex (AD10).
    fn run_checks(
        ctx: &Ctx<'_>,
        states: &[(String, TargetState)],
    ) -> Result<Result<Checked, String>, VerbError> {
        let started = std::time::Instant::now();
        let budget = std::time::Duration::from_secs(COMMIT_CHECK_BUDGET_SECS);
        let mut timings = Vec::new();
        let mut checks = Vec::new();
        for (rel, st) in states {
            if *st == TargetState::Absent {
                checks.push((rel.clone(), CHECK_ABSENT_STATE.to_string()));
                continue;
            }
            let declared = entries()
                .iter()
                .find(|e| e.rel == rel)
                .and_then(|e| e.timeout_s);
            let own = std::time::Duration::from_secs(u64::from(clamp_check_timeout(declared)));
            let left = budget.saturating_sub(started.elapsed());
            if left.is_zero() {
                return Ok(Err(format!(
                    "{rel}: 확정 검사 시간 예산({COMMIT_CHECK_BUDGET_SECS}초)을 다 썼습니다"
                )));
            }
            let report: CheckReport = (ctx.check)(rel, &live_path(ctx, rel), own.min(left))?;
            if !report.passed {
                let why = if report.timed_out {
                    "시간 초과".to_string()
                } else {
                    report.failure.clone()
                };
                return Ok(Err(format!("{rel}: {why}")));
            }
            timings.push(CheckTiming {
                rel: rel.clone(),
                check_ms: report.check_ms,
                reference_kind: report.reference_kind,
                reference_ms: report.reference_ms,
            });
            checks.push((rel.clone(), check_name(super::check_kind_for(rel))));
        }
        Ok(Ok((timings, checks)))
    }

    /// AD4: the registrar's report confirms a hook line only when it lists that line as wanted
    /// `unregistered` and every settings file it touched answers `unregistered`.
    pub(crate) fn hook_line_confirmed(
        report: &cys::repair::apply::ReconcileReport,
        event: &str,
        script: &str,
    ) -> bool {
        report.hooks.iter().any(|h| {
            h.event == event
                && h.script == script
                && h.desired == "unregistered"
                && !h.targets.is_empty()
                && h.targets.iter().all(|(_, st)| st == "unregistered")
        })
    }

    /// The O-C4-3 spelling of a check kind (`sh-syntax` …) for the record.
    fn check_name(k: cys::repair::boot_critical::CheckKind) -> String {
        serde_json::to_value(k)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_else(|| format!("{k:?}"))
    }

    /// `cys repair commit <R-id>` — a GENERAL repair (boot-critical / protection-related go through
    /// submit and the owner's approval): the worker's record + reviewer1's `pass` from a seat that never
    /// fixed it (C1 1.9 · AD1) + the live states still reproduce the verified fingerprint (N1 / U2) +
    /// the repair's check passes on the LIVE files. Three steps (RULINGS R3 · AD10): (1) under the
    /// per-repair mutex: the preconditions and the live states; (2) WITHOUT the mutex: the check
    /// children; (3) the mutex re-taken: the record re-read — aborted if it changed or the live states
    /// moved — then the committed state durable (E1) → the hook step (Y1, only after that) → the
    /// lane's File tombstones (AD3) → this lane's own row (K1; 검증됨, or 충돌 on a step error — Y2(c)).
    pub(crate) fn commit(ctx: &Ctx<'_>, caller: &Caller, p: &Value) -> Result<Value, VerbError> {
        let id = repair_id(p)?;
        // (1) preconditions under the per-repair mutex
        let (snapshot, states) = ctx.locked(id.as_str(), || -> Result<(RepairRecord, Vec<(String, TargetState)>), VerbError> {
            let mut rec = load(ctx, &id)?;
            require_origin(ctx, &rec)?;
            holder_renew(ctx, caller, &mut rec)?;
            match rec.classification {
                Classification::General => {}
                Classification::RuleDocument if rec.owner_token_sha256.is_some() => {}
                Classification::RuleDocument => {
                    return Err(VerbError::new(cys::repair::types::ERR_PRECONDITION, format!("{}: 규칙 문서 수리는 주인 지시 표가 있어야 합니다", id.as_str())))
                }
                Classification::BootCritical | Classification::ProtectionRelated => {
                    return Err(VerbError::new(
                        cys::repair::types::ERR_PRECONDITION,
                        format!("{}: 기동 필수·보호장치 관련 수리는 `cys repair submit` 과 주인의 최종 승인으로 적용됩니다", id.as_str()),
                    ))
                }
                Classification::DaemonSource => {
                    return Err(VerbError::new(cys::repair::types::ERR_PRECONDITION, format!("{}: 데몬 소스 수리는 2차 출시입니다", id.as_str())))
                }
            }
            if rec.state != RepairState::InProgress {
                return Err(VerbError::new(
                    cys::repair::types::ERR_PRECONDITION,
                    format!("{}: '{}' 상태에서는 확정할 수 없습니다", id.as_str(), rec.state.label_ko()),
                ));
            }
            let worker = rec.worker_record.clone().ok_or_else(|| {
                VerbError::new(cys::repair::types::ERR_PRECONDITION, format!("{}: 워커의 기록이 없습니다", id.as_str()))
            })?;
            let verify = rec.verify.clone().ok_or_else(|| {
                VerbError::new(cys::repair::types::ERR_PRECONDITION, format!("{}: 리뷰어1의 검증 서명이 없습니다", id.as_str()))
            })?;
            if verify.verdict != "pass" {
                return Err(VerbError::new(cys::repair::types::ERR_PRECONDITION, format!("{}: 리뷰어1의 검증이 통과가 아닙니다", id.as_str())));
            }
            let fixer_seat = rec.lock.as_ref().map(|l| l.owner.seat);
            if verify.seat == worker.seat
                || Some(verify.seat) == fixer_seat
                || rec.fixer_seats.contains(&verify.seat)
            {
                return Err(VerbError::new(
                    cys::repair::types::ERR_PRECONDITION,
                    format!("{}: 검증 서명이 고친 좌석의 것입니다 — 다른 좌석의 리뷰어1 서명이 필요합니다", id.as_str()),
                ));
            }
            let states = current_states(ctx, &rec)?;
            if fingerprint(&states) != verify.fingerprint_sha256 {
                return Err(VerbError::new(
                    "target_changed",
                    format!("{}: 검증 서명 뒤에 살아 있는 파일(내용·권한·링크)이 바뀌었습니다 — 다시 기록하고 검증받으세요", id.as_str()),
                ));
            }
            Ok((rec, states))
        })?;
        // (2) the check children — the per-repair mutex is NOT held (AD10 / R3)
        let checked = run_checks(ctx, &states)?;
        // (3a) re-take, re-read, abort if anything moved, then write the committed state durably
        let (before, written, timings, mut checks, lines) = ctx.locked(id.as_str(), || -> Result<CommitStep, VerbError> {
            let mut rec = load(ctx, &id)?;
            if rec != snapshot {
                return Err(VerbError::new(
                    "target_changed",
                    format!("{}: 검사하는 동안 수리 기록이 바뀌었습니다 — 확정을 멈춥니다(다시 `cys repair commit`)", id.as_str()),
                ));
            }
            if current_states(ctx, &rec)? != states {
                return Err(VerbError::new(
                    "target_changed",
                    format!("{}: 검사하는 동안 살아 있는 파일이 바뀌었습니다 — 다시 기록하고 검증받으세요", id.as_str()),
                ));
            }
            if lock::claim_closed(&ctx.root, rec.lock.as_ref().and_then(|l| l.claim)) {
                return Err(VerbError::new(
                    cys::repair::types::ERR_LOCKED,
                    format!("{}: 이 잠금의 자동 수리 차례가 닫혔습니다 — 새 차례가 필요합니다", id.as_str()),
                ));
            }
            let (timings, checks) = match checked {
                Ok(t) => t,
                Err(why) => {
                    if let Some(w) = rec.worker_record.as_mut() {
                        w.verdict = CHECK_FAILED.to_string(); // the next `record` opens a new round
                    }
                    rec.methods.push(format!("확정 검사 실패: {why}").chars().take(200).collect());
                    write(ctx, &rec)?;
                    return Err(VerbError::new(
                        cys::repair::types::ERR_CHECK_FAILED,
                        format!("{}: 살아 있는 파일의 검사가 실패해 확정을 거부합니다 — {why}", id.as_str()),
                    ));
                }
            };
            let before = rec.clone();
            rec.committed = canon_list(&states);
            rec.checks = checks.clone();
            rec.state = RepairState::Probation;
            rec.audit_due_at = Some(ctx.now + AUDIT_WINDOW_SECS as f64);
            rec.lock = None;
            ledger::write_record_durable_in(&ctx.root, &rec)?;
            let lines: Vec<(String, String)> = hook_lines(&rec)
                .filter_map(|f| match &f.kind {
                    ChangeKind::HookUnregister { event, script } => Some((event.clone(), script.clone())),
                    _ => None,
                })
                .collect();
            Ok((before, rec, timings, checks, lines))
        })?;
        let committed = written.committed.clone();
        // RULINGS Y1: the hook step only AFTER the committed state is durable — and (AK5) WITHOUT the
        // per-repair mutex: the registrar child must not stall a reap or another verb of this repair.
        let has_hooks = !lines.is_empty();
        let step = if has_hooks {
            Some((ctx.hook_step)(
                &id,
                &ctx.lane,
                cys::repair::apply::HookStep::Unregister,
            ))
        } else {
            None
        };
        // (3b) the mutex re-taken: the hook step's result, older repairs superseded, tombstones, the row
        ctx.locked(id.as_str(), || -> Result<Value, VerbError> {
            let mut step_error: Option<String> = None;
            let mut warnings: Vec<String> = Vec::new();
            match step {
                None => {}
                Some(Ok(report)) => {
                    // AD4: a hook line is checked by the registrar's report — claimed only for a
                    // line the report shows unregistered in every settings file it touched.
                    for (event, script) in &lines {
                        if hook_line_confirmed(&report, event, script) {
                            checks.push((format!("settings:{event}:{script}"), CHECK_HOOK_REGISTRAR.to_string()));
                        } else {
                            step_error.get_or_insert_with(|| format!("settings:{event}:{script}: 등록 해제가 등록기의 보고로 확인되지 않았습니다"));
                        }
                    }
                }
                Some(Err(RepairError::NotImplemented(f))) => {
                    // Y1: until T6 lands, a commit with a hook line is refused with that sentence;
                    // the stub changed nothing, so the record goes back to its pre-commit state (the
                    // commit never stood — whatever was signed over it in the meantime is void too).
                    ledger::write_record_durable_in(&ctx.root, &before)?;
                    return Err(RepairError::NotImplemented(f).into());
                }
                Some(Err(e)) => step_error = Some(VerbError::from(e).message),
            }
            // The record as it is now (an audit may have signed while the hook step ran): only the
            // `checks` field is this step's to change.
            let mut rec = load(ctx, &id)?;
            if rec.checks != checks {
                rec.checks = checks.clone();
                ledger::write_record_durable_in(&ctx.root, &rec)?;
            }
            // AK4 / C1 1.6: every OLDER repair of this lane in force on one of the same targets is
            // superseded in this step (never reopened by an audit, never picked up afterwards).
            let superseded = supersede_older(ctx, &written)?;
            // AD3 / AK3 / C1 1.5: only a REAL deletion (the pre-image was present) is tombstoned;
            // a path committed present drops only THIS repair's own File tombstone; an unreadable
            // tombstone file is a warning there (a broken ledger file never stops repairs).
            for (rel, st) in &states {
                let pre_present = written
                    .files
                    .iter()
                    .find(|f| f.rel == *rel)
                    .is_some_and(|f| f.pre != PreImage::Absent);
                if *st == TargetState::Absent {
                    if !pre_present {
                        continue; // created and deleted again inside this repair: nothing was removed
                    }
                    let r = cys::repair::tombstones::record_in(
                        &ctx.root,
                        &ctx.lane,
                        &cys::repair::tombstones::Tombstone {
                            rel: rel.clone(),
                            kind: cys::repair::tombstones::TombstoneKind::File,
                            repair: id.clone(),
                        },
                    );
                    if let Err(e) = r {
                        step_error.get_or_insert_with(|| format!("{rel}: 삭제 표식(묘비) 기록 실패 — {}", VerbError::from(e).message));
                    }
                } else if let Err(e) = cys::repair::tombstones::remove_file_rel_in(&ctx.root, &ctx.lane, rel, &id) {
                    warnings.push(format!("{rel}: 삭제 표식 파일을 읽을 수 없어 이 수리의 표식을 지우지 못했습니다 — {}", VerbError::from(e).message));
                }
            }
            let (version, pro) = pack_generation(&ctx.pack_dir);
            let previous = ledger::read_lane_in(&ctx.root, &id, &ctx.lane).ok().flatten();
            let mut applied_history = previous.as_ref().map(|r| r.applied_history.clone()).unwrap_or_default();
            if let Some(prev) = &previous {
                if !prev.applied.is_empty() && prev.applied != committed {
                    applied_history.push(prev.applied.clone());
                }
            }
            let row = LaneRecord {
                schema: cys::repair::REPAIR_SCHEMA,
                repair: id.clone(),
                lane: ctx.lane.clone(),
                state: if step_error.is_some() { LaneState::Conflict } else { LaneState::Verified },
                probation: Some(Probation {
                    pack_generation: version,
                    pro_revision: pro,
                    applied_at: ctx.now,
                    expected: committed.clone(),
                    closed: None,
                    check_timings: timings,
                    check_timeouts: Vec::new(),
                }),
                merge: None,
                merge_history: previous.as_ref().map(|r| r.merge_history.clone()).unwrap_or_default(),
                approval_ref: None,
                conflict_live: if step_error.is_some() { committed.clone() } else { Vec::new() },
                applied: committed.clone(),
                applied_history,
                merge_base: Vec::new(),
                merge_lock: None,
                merge_attempt: 0,
                updated_at: ctx.now,
            };
            ledger::write_lane_in(&ctx.root, &row)?;
            Ok(reply(ctx, &rec, json!({
                "committed": committed,
                "audit_due_at": rec.audit_due_at,
                "hook_step": if !has_hooks { json!(null) } else if let Some(e) = &step_error { json!({"error": e}) } else { json!("ok") },
                "superseded": superseded.iter().map(|r| r.as_str().to_string()).collect::<Vec<_>>(),
                "warnings": warnings,
            })))
        })
    }

    /// What step (3a) of `commit` hands on: the pre-commit record, the record as written, the check
    /// timings, the checks that ran, the hook lines.
    type CommitStep = (
        RepairRecord,
        RepairRecord,
        Vec<CheckTiming>,
        Vec<(String, String)>,
        Vec<(String, String)>,
    );

    /// RULINGS AK4 / C1 1.6: marks 다른 수리로 대체됨 every repair of THIS lane (one writer per record —
    /// B1) that is older than `newer` (lower number), in force (시험 기간 / 사용 중) and shares one of
    /// its targets. Each is re-read under its own mutex (taken while `newer`'s is held — always from
    /// the higher number to the lower, so no two commits wait on each other). Returns their ids.
    fn supersede_older(ctx: &Ctx<'_>, newer: &RepairRecord) -> Result<Vec<RepairId>, VerbError> {
        let targets: Vec<&str> = newer.files.iter().map(|f| f.rel.as_str()).collect();
        let shares = |r: &RepairRecord| r.files.iter().any(|f| targets.contains(&f.rel.as_str()));
        let in_force = |r: &RepairRecord| {
            r.origin_lane == ctx.lane
                && matches!(r.state, RepairState::Probation | RepairState::InUse)
        };
        let mut done = Vec::new();
        let mut older: Vec<RepairId> = ledger::list_ids_in(&ctx.root)
            .into_iter()
            .filter(|o| *o < newer.id)
            .collect();
        older.sort();
        older.reverse();
        for oid in older {
            let candidate = match ledger::read_record_in(&ctx.root, &oid) {
                Ok(Some(r)) => in_force(&r) && shares(&r),
                _ => false,
            };
            if !candidate {
                continue;
            }
            let changed = ctx.locked(oid.as_str(), || -> Result<bool, VerbError> {
                let Some(mut r) = ledger::read_record_in(&ctx.root, &oid)? else {
                    return Ok(false);
                };
                if !(in_force(&r) && shares(&r)) {
                    return Ok(false);
                }
                r.state = RepairState::Superseded;
                ledger::write_record_durable_in(&ctx.root, &r)?;
                Ok(true)
            })?;
            if changed {
                done.push(oid);
            }
        }
        Ok(done)
    }

    // ─────────────────────────────── list / status (READ) ───────────────────────────────

    fn summary(rec: &RepairRecord) -> Value {
        json!({
            "repair": rec.id.as_str(),
            "state": rec.state,
            "state_label": rec.state.label_ko(),
            "classification": rec.classification,
            "classification_label": rec.classification.label_ko(),
            "trigger": rec.trigger,
            "attempt": rec.attempt,
            "max_rounds": MAX_ROUNDS,
            "origin_lane": rec.origin_lane.0,
            "files": rec.files.iter().map(|f| f.rel.clone()).collect::<Vec<_>>(),
            "lock": rec.lock.as_ref().map(|l| json!({"lane": l.owner.lane.0, "seat": l.owner.seat, "expires_at": l.expires_at, "claim": l.claim})),
            "resolves": rec.resolves.as_ref().map(|r| r.as_str().to_string()),
            "created_at": rec.created_at,
        })
    }

    /// Every repair (unreadable ones listed as corrupt — F4) and the intake aggregated by signature
    /// across lanes (G4); "open defects" = signatures no open repair lists.
    fn scan(ctx: &Ctx<'_>) -> (Vec<RepairRecord>, Vec<Value>) {
        let mut recs = Vec::new();
        let mut corrupt = Vec::new();
        for id in ledger::list_ids_in(&ctx.root) {
            match ledger::read_record_in(&ctx.root, &id) {
                Ok(Some(r)) => recs.push(r),
                Ok(None) => {}
                Err(e) => corrupt
                    .push(json!({"repair": id.as_str(), "corrupt": VerbError::from(e).message})),
            }
        }
        (recs, corrupt)
    }

    fn defects_json(recs: &[RepairRecord], ctx: &Ctx<'_>) -> (Vec<Value>, usize, Vec<String>) {
        let (defects, broken) = intake::load_all_in(&ctx.root);
        let mut open = 0;
        let rows = defects
            .into_iter()
            .map(|d| {
                let joined = recs
                    .iter()
                    .find(|r| {
                        r.state != RepairState::Reverted
                            && r.intake_signatures.contains(&d.signature)
                    })
                    .map(|r| r.id.as_str().to_string());
                if joined.is_none() {
                    open += 1;
                }
                json!({
                    "signature": d.signature,
                    "rel": d.rel,
                    "error_class": d.error_class,
                    "first_line": d.first_line,
                    "count": d.count,
                    "first_seen": d.first_seen,
                    "last_seen": d.last_seen,
                    "lanes": d.lanes.iter().map(|l| l.0.clone()).collect::<Vec<_>>(),
                    "joined": joined,
                    "open": joined.is_none(),
                })
            })
            .collect();
        (rows, open, broken)
    }

    /// `cys repair list [--json]` (READ — any caller).
    pub(crate) fn list(ctx: &Ctx<'_>) -> Result<Value, VerbError> {
        let (recs, corrupt) = scan(ctx);
        let (defects, open, broken) = defects_json(&recs, ctx);
        Ok(json!({
            "lane": ctx.lane.0,
            "repairs": recs.iter().map(summary).collect::<Vec<_>>(),
            "corrupt": corrupt,
            "defects": defects,
            "open_defects": open,
            "intake_unreadable": broken,
            "intake_over_quota": intake_over_quota(ctx),
        }))
    }

    /// `cys repair status [<R-id>] [--json]` (READ — any caller). Without an id: this lane's summary +
    /// H3's catch-up fields + the reviewer slots (B2: an empty slot means repairs wait). With an id:
    /// the record, per-lane states over the registered lanes (K1), "전파 완료" (P2; never declared when
    /// the registry is unknown — L4), which of origin bytes / changed merge / unchanged merge each lane
    /// holds (H8), the vault slots (C2 2.5, E4 flag), the working copy (U3).
    pub(crate) fn status(ctx: &Ctx<'_>, p: &Value) -> Result<Value, VerbError> {
        if p.get("repair").is_some_and(|v| !v.is_null()) {
            let id = repair_id(p)?;
            return status_one(ctx, &id);
        }
        let (recs, corrupt) = scan(ctx);
        let (_, open, broken) = defects_json(&recs, ctx);
        let mut by_state = serde_json::Map::new();
        for r in &recs {
            let k = r.state.label_ko().to_string();
            let n = by_state.get(&k).and_then(|v| v.as_u64()).unwrap_or(0);
            by_state.insert(k, json!(n + 1));
        }
        let mut r1 = Vec::new();
        let mut r2 = Vec::new();
        for (sid, role) in &ctx.seats {
            match cys::repair::roles::reviewer_slot_of(role) {
                Some(cys::repair::roles::ReviewerSlot::Reviewer1) => r1.push(*sid),
                Some(cys::repair::roles::ReviewerSlot::Reviewer2) => r2.push(*sid),
                None => {}
            }
        }
        let mut out = json!({
            "lane": ctx.lane.0,
            "repairs": recs.len(),
            "by_state": by_state,
            "open_defects": open,
            "corrupt": corrupt,
            "intake_unreadable": broken,
            "intake_over_quota": intake_over_quota(ctx),
            "reviewer_slots": {
                "reviewer1": r1,
                "reviewer2": r2,
                "waiting": r1.is_empty() || r2.is_empty(),
            },
        });
        // RULINGS H3: the lane's catch-up fields (display only; T7b computes them).
        match cys::repair::apply::catchup_status(&ctx.lane) {
            Ok(c) => {
                out["initial_catchup_complete"] = json!(c.complete);
                out["catchup_pending"] = json!(c.pending);
                out["catchup_conflict"] = json!(c.conflict);
            }
            Err(e) => {
                out["initial_catchup_complete"] = Value::Null;
                out["catchup_pending"] = Value::Null;
                out["catchup_conflict"] = Value::Null;
                out["catchup_unavailable"] = json!(VerbError::from(e).message);
            }
        }
        Ok(out)
    }

    fn att_json(a: &Option<Attestation>) -> Value {
        a.as_ref().map_or(Value::Null, |a| {
            json!({"seat": a.seat, "role": a.role, "verdict": a.verdict, "at": a.at, "fingerprint": a.fingerprint_sha256, "note": a.note})
        })
    }

    fn status_one(ctx: &Ctx<'_>, id: &RepairId) -> Result<Value, VerbError> {
        let rec = match ledger::read_record_in(&ctx.root, id) {
            Ok(Some(r)) => r,
            Ok(None) => {
                return Err(VerbError::not_found(format!(
                    "{}: 그런 수리가 없습니다",
                    id.as_str()
                )))
            }
            Err(e) => {
                return Ok(json!({"repair": id.as_str(), "corrupt": VerbError::from(e).message}));
            }
        };
        let (rows, broken) = ledger::lane_rows_in(&ctx.root, id);
        let merges: Vec<Value> = rows
            .iter()
            .map(|r| {
                let holds = match &r.merge {
                    None => "origin_bytes",
                    Some(m) if m.changed => "merge_changed",
                    Some(_) => "merge_unchanged",
                };
                json!({"lane": r.lane.0, "state": r.state, "state_label": r.state.label_ko(), "holds": holds,
                       "applied_is_current": r.applied == rec.committed,
                       "merge_lock": r.merge_lock.as_ref().map(|l| json!({"seat": l.owner.seat, "expires_at": l.expires_at}))})
            })
            .collect();
        let slots: Vec<Value> = ledger::list_slots_in(&ctx.root, id)
            .into_iter()
            .map(|s| json!({"slot": s.name, "dir": s.dir, "entries": s.entries, "changed_after_deposit": s.changed_after_deposit}))
            .collect();
        let staging = ledger::staging_dir_in(&ctx.root, id);
        let lock = rec.lock.as_ref().map(|l| {
            let view = if lock::released_by_restart(l, &ctx.lane, &ctx.incarnation) {
                "released (daemon restarted)"
            } else if ctx.now < l.expires_at {
                "live"
            } else {
                "expired"
            };
            json!({"lane": l.owner.lane.0, "seat": l.owner.seat, "expires_at": l.expires_at, "claim": l.claim, "view": view})
        });
        Ok(reply(
            ctx,
            &rec,
            json!({
                "trigger": rec.trigger,
                "trigger_label": rec.trigger.label_ko(),
                "classification_label": rec.classification.label_ko(),
                "symptom": rec.symptom,
                "cause": rec.cause,
                "files": rec.files.iter().map(|f| json!({"rel": f.rel, "kind": f.kind, "pre": f.pre.state().canon(), "post": f.post.canon()})).collect::<Vec<_>>(),
                "methods": rec.methods,
                "worker_record": att_json(&rec.worker_record),
                "verify": att_json(&rec.verify),
                "audit": att_json(&rec.audit),
                "audit_due_at": rec.audit_due_at,
                "lock": lock,
                "resolves": rec.resolves.as_ref().map(|r| r.as_str().to_string()),
                "committed": rec.committed,
                "pending_request": rec.pending_request,
                "resubmits": rec.resubmits,
                "lane_rows": merges,
                "lane_rows_unreadable": broken,
                "vault_slots": slots,
                "staging": if is_approval_class(rec.classification) || staging.exists() { json!(staging) } else { Value::Null },
                "intake_signatures": rec.intake_signatures,
            }),
        ))
    }

    // ─────────────────────────────── cleanup / revert lane condition (WORK) ───────────────────────────────

    /// `revert` / `cleanup` from a worker: a worker seat of a lane holding a row of that repair (or its
    /// origin lane) — C3 P4. A human (M4) passes.
    pub(crate) fn worker_lane_check(
        ctx: &Ctx<'_>,
        caller: &Caller,
        p: &Value,
    ) -> Result<RepairRecord, VerbError> {
        let id = repair_id(p)?;
        let rec = load(ctx, &id)?;
        if caller.seat().is_some() {
            let holds_row = rec.origin_lane == ctx.lane
                || matches!(ledger::read_lane_in(&ctx.root, &id, &ctx.lane), Ok(Some(_)));
            if !holds_row {
                return Err(VerbError::refused(format!(
                    "{}: 이 부서에는 이 수리의 행이 없습니다",
                    id.as_str()
                )));
            }
        }
        Ok(rec)
    }

    fn unfinished_ops(root: &Path, id: &RepairId) -> Vec<String> {
        let dir = ledger::repair_dir_in(root, id).join("ops");
        let Ok(rd) = std::fs::read_dir(&dir) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for e in rd.filter_map(Result::ok) {
            let path = e.path();
            let done = std::fs::read(&path)
                .ok()
                .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                .map(|v| {
                    v["finished"] == json!(true)
                        || v["done"] == json!(true)
                        || v["step"] == json!("done")
                })
                .unwrap_or(false);
            if !done {
                out.push(e.file_name().to_string_lossy().to_string());
            }
        }
        out.sort();
        out
    }

    /// `cys repair cleanup <R-id> [--slot <시각>]` — removes vault slots after a worker's review
    /// (O-C2-6 as amended by P6): only in a terminal state and with no unfinished operation record; while
    /// the repair is in force the slots it still depends on are refused (T6's `slots_needed_in_force`;
    /// its `Err` refuses every slot).
    pub(crate) fn cleanup(ctx: &Ctx<'_>, caller: &Caller, p: &Value) -> Result<Value, VerbError> {
        let rec = worker_lane_check(ctx, caller, p)?;
        let id = rec.id.clone();
        let only = match param_str(p, "slot") {
            Some(s) if ledger::is_slot_name(s) => Some(s.to_string()),
            Some(s) => {
                return Err(VerbError::invalid(format!(
                    "보관소 칸 이름이 올바르지 않습니다: {s:?}"
                )))
            }
            None => None,
        };
        ctx.locked(id.as_str(), || -> Result<Value, VerbError> {
            let rec = load(ctx, &id)?;
            if !matches!(rec.state, RepairState::InUse | RepairState::Reverted | RepairState::UpstreamFixed | RepairState::Superseded) {
                return Err(VerbError::new(
                    cys::repair::types::ERR_PRECONDITION,
                    format!("{}: '{}' 상태에서는 보관소를 정리하지 않습니다", id.as_str(), rec.state.label_ko()),
                ));
            }
            let open_ops = unfinished_ops(&ctx.root, &id);
            if !open_ops.is_empty() {
                return Err(VerbError::new(
                    cys::repair::types::ERR_PRECONDITION,
                    format!("{}: 끝나지 않은 동작 기록이 있어 정리하지 않습니다: {}", id.as_str(), open_ops.join(", ")),
                ));
            }
            let mut slots = ledger::list_slots_in(&ctx.root, &id);
            if let Some(name) = &only {
                slots.retain(|s| s.name == *name);
                if slots.is_empty() {
                    return Err(VerbError::not_found(format!("{}: 그런 보관소 칸이 없습니다: {name}", id.as_str())));
                }
            }
            let in_force = matches!(rec.state, RepairState::Probation | RepairState::InUse);
            let needed: Result<Vec<PathBuf>, String> = if in_force {
                cys::repair::apply::slots_needed_in_force(&id).map_err(|e| VerbError::from(e).message)
            } else {
                Ok(Vec::new())
            };
            let mut removed = Vec::new();
            let mut refused = Vec::new();
            for s in slots {
                let reason = match &needed {
                    Err(why) => Some(format!("사용 중인 수리가 기대는 칸인지 확인할 수 없습니다({why})")),
                    Ok(list) if list.iter().any(|n| n == &s.dir || s.dir.starts_with(n) || n.starts_with(&s.dir)) => {
                        Some("사용 중인 수리가 아직 기대는 칸입니다(되돌리기·재검사·다른 부서의 적용)".to_string())
                    }
                    Ok(_) => None,
                };
                if let Some(r) = reason {
                    refused.push(json!({"slot": s.name, "reason": r}));
                    continue;
                }
                match std::fs::symlink_metadata(&s.dir) {
                    Ok(m) if m.is_dir() && !m.file_type().is_symlink() => match std::fs::remove_dir_all(&s.dir) {
                        Ok(()) => removed.push(s.name),
                        Err(e) => refused.push(json!({"slot": s.name, "reason": e.to_string()})),
                    },
                    _ => refused.push(json!({"slot": s.name, "reason": "실제 폴더가 아닙니다"})),
                }
            }
            if removed.is_empty() && !refused.is_empty() {
                return Err(VerbError::new(
                    cys::repair::types::ERR_PRECONDITION,
                    format!("{}: 정리할 수 있는 칸이 없습니다 — {}", id.as_str(), refused.iter().filter_map(|r| r["reason"].as_str()).collect::<Vec<_>>().join(" · ")),
                ));
            }
            Ok(json!({"repair": id.as_str(), "removed": removed, "refused": refused}))
        })
    }

    // ─────────────────────────────── seat reap release (C1 1.12 / O5 / R5) ───────────────────────────────

    /// Clears every repair lock and lane merge lock held by `sid` of this daemon (lane + seat +
    /// incarnation), each under that repair's mutex — G6's reap exception; nothing else is changed.
    pub(crate) fn release_seat(ctx: &Ctx<'_>, sid: u64) {
        let owner = ctx.owner(sid);
        for id in ledger::list_ids_in(&ctx.root) {
            let quick = ledger::read_record_in(&ctx.root, &id).ok().flatten();
            let holds_record = quick.as_ref().is_some_and(|r| {
                r.origin_lane == ctx.lane
                    && r.lock.as_ref().is_some_and(|l| lock::owned_by(l, &owner))
            });
            let holds_merge = matches!(
                ledger::read_lane_in(&ctx.root, &id, &ctx.lane),
                Ok(Some(ref row)) if row.merge_lock.as_ref().is_some_and(|l| lock::owned_by(l, &owner))
            );
            if !holds_record && !holds_merge {
                continue;
            }
            ctx.locked(id.as_str(), || {
                if let Ok(Some(mut rec)) = ledger::read_record_in(&ctx.root, &id) {
                    if rec.origin_lane == ctx.lane
                        && rec.lock.as_ref().is_some_and(|l| lock::owned_by(l, &owner))
                    {
                        rec.lock = None;
                        let _ = ledger::write_record_in(&ctx.root, &rec);
                    }
                }
                if let Ok(Some(mut row)) = ledger::read_lane_in(&ctx.root, &id, &ctx.lane) {
                    if row
                        .merge_lock
                        .as_ref()
                        .is_some_and(|l| lock::owned_by(l, &owner))
                    {
                        row.merge_lock = None;
                        row.updated_at = ctx.now;
                        let _ = ledger::write_lane_in(&ctx.root, &row);
                    }
                }
            });
        }
    }
}

/// The pending final-approval requests (C5; memory only — void after a restart). T5b fills it.
#[derive(Default)]
pub(crate) struct ApprovalTable {}

/// The per-seat owner-prompt record (T3 writes, T4 reads). `named_docs` = rule-document base names,
/// lower-cased, found in the seat's latest owner-typed prompt (RULINGS U4(a)); no prompt text kept.
/// `digest` / `origin` / `at` describe the seat's LATEST prompt (any origin).
#[allow(dead_code)] // read by T4's owner-origin helper (wave 2)
#[derive(Default, Clone)]
pub(crate) struct PromptOrigin {
    pub digest: String,
    pub origin: String,
    pub at: f64,
    pub question: Option<OpenQuestion>,
    pub named_docs: Vec<String>,
}

/// An open question of a master seat (RULINGS T3, Q1 / Q2): `first_human` = (digest, excerpt, at).
/// T4 registers it (`grant ask` / `owner-token ask`); T3 records the answer and `machine_seen`.
#[allow(dead_code)] // registered and read by T4 (wave 2)
#[derive(Default, Clone)]
pub(crate) struct OpenQuestion {
    pub asked_at: f64,
    pub path: Option<String>,
    pub first_human: Option<(String, String, f64)>,
    pub machine_seen: bool,
}

/// Longest kept excerpt of an owner-typed answer (characters; memory only).
pub(crate) const ANSWER_EXCERPT_CHARS: usize = 120;

/// RULINGS T3 / U4(a): updates the per-seat prompt record from `hook.machine_origin`'s verdict.
/// `origin` = `human` | `machine` | `unknown`. While a question is open: the FIRST owner-typed
/// prompt after `asked_at` is kept once as the answer; any other prompt (machine OR unknown — fail
/// closed) before that answer sets `machine_seen`. `named_docs` changes only on an owner-typed
/// prompt. Memory only; no prompt text beyond the answer's short excerpt.
pub(crate) fn record_prompt_origin(
    daemon: &Daemon,
    sid: u64,
    norm: &str,
    digest_in: Option<&str>,
    origin: Option<&str>,
) {
    let now = cys::repair::ledger::now_epoch();
    let digest = match digest_in {
        Some(d) => d.to_string(),
        None => cys::mission_gate::digest_normalized(norm),
    };
    let mut map = daemon
        .repair_prompt_origin
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let entry = map.entry(sid).or_default();
    apply_prompt(entry, &digest, norm, origin.unwrap_or("unknown"), now);
}

/// Pure part of [`record_prompt_origin`].
pub(crate) fn apply_prompt(
    rec: &mut PromptOrigin,
    digest: &str,
    norm: &str,
    origin: &str,
    now: f64,
) {
    let human = origin == "human";
    rec.digest = digest.to_string();
    rec.origin = origin.to_string();
    rec.at = now;
    if human {
        rec.named_docs = named_rule_docs(norm);
    }
    if let Some(q) = rec.question.as_mut() {
        if now > q.asked_at && q.first_human.is_none() {
            if human {
                let excerpt: String = norm.chars().take(ANSWER_EXCERPT_CHARS).collect();
                q.first_human = Some((digest.to_string(), excerpt, now));
            } else {
                q.machine_seen = true;
            }
        }
    }
}

/// C10's rule-document base names (as `guard.sh:PROTECTED_NAMES`), lower-cased.
const RULE_DOC_NAMES: [&str; 5] = [
    "soul.md",
    "claude.md",
    "vibecoding_constitution.md",
    "vibecoding_enforcement.md",
    "route_contract.md",
];

/// RULINGS AD6 / C10: whether a pack-relative target is a rule document — its base name, lower-cased,
/// is one of C10's names or `<x>_directive.md` (the same classification as [`named_rule_docs`]).
pub(crate) fn is_rule_document(rel: &str) -> bool {
    let base = rel
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(rel)
        .to_ascii_lowercase();
    RULE_DOC_NAMES.contains(&base.as_str())
        || (base.ends_with("_directive.md") && base.len() > "_directive.md".len())
}

/// RULINGS AK6: a path part that Windows resolves to another name — one ending in a dot or a space
/// (`MASTER_DIRECTIVE.md.`, `soul.md `) or containing `:` (`soul.md::$DATA`, a drive or stream).
pub(crate) fn is_windows_alias(rel: &str) -> bool {
    rel.split(['/', '\\']).any(|part| {
        part.ends_with('.') && part != "." && part != ".."
            || part.ends_with(' ')
            || part.contains(':')
    })
}

/// Rule-document base names in a prompt, lower-cased, `.md` form (C10's classification, as
/// `guard.sh:PROTECTED_NAMES` + `*_directive.md`): `soul.md`, `claude.md`,
/// `vibecoding_constitution.md`, `vibecoding_enforcement.md`, `route_contract.md`, any
/// `<x>_directive.md`; a bare `<X>_DIRECTIVE` counts as `<x>_directive.md` (Q2's example).
pub(crate) fn named_rule_docs(text: &str) -> Vec<String> {
    const NAMES: [&str; 5] = RULE_DOC_NAMES;
    let mut out: Vec<String> = Vec::new();
    for tok in text.split(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))) {
        let t = tok.trim_matches('.').to_ascii_lowercase();
        if t.is_empty() {
            continue;
        }
        let name = if NAMES.contains(&t.as_str())
            || (t.ends_with("_directive.md") && t.len() > "_directive.md".len())
        {
            Some(t.clone())
        } else if t.ends_with("_directive") && t.len() > "_directive".len() {
            Some(format!("{t}.md"))
        } else {
            None
        };
        if let Some(n) = name {
            if !out.contains(&n) {
                out.push(n);
            }
        }
    }
    out
}

/// Releases every repair lock (and lane merge lock — R5) a seat of THIS daemon holds (C1 1.12:
/// seat reap / close; O5: under the same per-repair mutex). Called once from
/// `governance::close_surface_effect`, which both the reaper and an owner close go through. Never
/// panics; an unreadable record is skipped (it cannot carry a usable lock anyway).
pub(crate) fn release_locks_for_seat(daemon: &Arc<Daemon>, sid: u64) {
    daemon
        .repair_prompt_origin
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&sid);
    forget_role_family(
        &LaneKey::of_socket(&daemon.socket_path).0,
        &daemon.incarnation,
        sid,
    );
    let classify = production_classify;
    let check = production_check;
    let ctx = Ctx {
        root: production_root(daemon),
        lane: LaneKey::of_socket(&daemon.socket_path),
        incarnation: daemon.incarnation.clone(),
        pack_dir: PathBuf::new(),
        now: cys::repair::ledger::now_epoch(),
        windows: cfg!(windows),
        registry: PathBuf::new(),
        mutexes: &daemon.repair_mutexes,
        seats: Vec::new(),
        classify: &classify,
        check: &check,
        hook_step: &cys::repair::apply::hook_lines_step,
    };
    ledger_verbs::release_seat(&ctx, sid);
}

#[cfg(test)]
mod tests {
    use super::*;
    use cys::repair::roles::{verb_class, VerbClass, VERB_CLASSES};
    use cys::Request;
    use serde_json::json;
    use std::sync::atomic::Ordering;

    fn code_of(reply: Reply) -> (String, String) {
        let Reply::Single(v) = reply else {
            panic!("single reply expected")
        };
        (
            v["error"]["code"].as_str().unwrap_or("").to_string(),
            v["error"]["message"].as_str().unwrap_or("").to_string(),
        )
    }

    /// (T3 rewrite of T0's pin — T3 filled 9 verbs.) Every sub-verb answers ONE structured reply and
    /// never panics, whatever the params, caller and grade; the 14 verbs whose bodies are other
    /// tracks' still answer `repair_not_implemented` to a caller the role table admits (the app for
    /// OWNER-APPROVAL, any caller for READ/TRIGGER) and `acl_denied` otherwise; unknown → `method_not_found`.
    #[test]
    fn every_subverb_is_not_implemented_and_never_panics() {
        assert_eq!(REPAIR_SUBVERBS.len(), 23);
        let mut uniq = REPAIR_SUBVERBS.to_vec();
        uniq.sort_unstable();
        uniq.dedup();
        assert_eq!(uniq.len(), 23, "REPAIR_SUBVERBS has duplicates");
        assert!(T3_VERBS.iter().all(|v| REPAIR_SUBVERBS.contains(v)));
        assert_eq!(T3_VERBS.len(), 9);
        let daemon = crate::team_gate_tests::tmp_daemon("repair-t0-subverbs", false);
        let params_set = [
            json!({}),
            json!(null),
            json!({"repair": "R-1", "files": ["a"], "role": "master"}),
            json!([1, 2]),
        ];
        for sub in REPAIR_SUBVERBS {
            for params in &params_set {
                for (sid, grade, pid) in [
                    (None, false, None),
                    (Some(3), false, Some(1)),
                    (None, true, Some(u32::MAX)),
                ] {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        code_of(handle(&daemon, sub, params, &json!(7), sid, grade, pid))
                    }));
                    let (code, msg) = result.unwrap_or_else(|_| panic!("repair.{sub} panicked"));
                    if T3_VERBS.contains(sub) {
                        // T3's bodies answer a success or a structured error — never the stub
                        assert_ne!(code, "repair_not_implemented", "repair.{sub}: {msg}");
                        assert_ne!(code, "method_not_found", "repair.{sub}");
                        continue;
                    }
                    let class = verb_class(sub).expect("every sub-verb has a class");
                    if *sub == "revert" && grade && sid.is_none() {
                        // a human (the app button) passes the role table; the lane check needs a
                        // repair that exists — none here
                        assert!(
                            matches!(code.as_str(), "invalid_params" | "not_found"),
                            "repair.revert: {code}"
                        );
                        continue;
                    }
                    let admitted = match class {
                        VerbClass::Read | VerbClass::Intake | VerbClass::Trigger => true,
                        VerbClass::OwnerApproval => grade && sid.is_none(),
                        _ => false, // seat 3 of tmp_daemon has no surface → no role
                    };
                    if admitted {
                        assert_eq!(code, "repair_not_implemented", "repair.{sub}");
                        assert_eq!(
                            msg,
                            format!("repair.{sub}: 이 빌드에는 아직 구현되지 않았습니다")
                        );
                    } else {
                        assert_eq!(code, "acl_denied", "repair.{sub} ({sid:?}, {grade})");
                    }
                }
            }
        }
        for sub in [
            "",
            "nope",
            "Begin",
            "begin ",
            "attest-verify",
            "approval.start",
            "../x",
            "\u{0}",
        ] {
            let (code, msg) = code_of(handle(
                &daemon,
                sub,
                &json!({}),
                &json!(1),
                None,
                false,
                None,
            ));
            assert_eq!(code, "method_not_found", "repair.{sub:?}");
            assert_eq!(msg, format!("unknown method: repair.{sub}"));
        }
        let _ = std::fs::remove_dir_all(production_root(&daemon));
    }

    /// J1 census: every daemon sub-verb has exactly one class; the table is exactly the 23 daemon
    /// verbs + the 5 local verbs; the WORK / OWNER-DECISION / OWNER-APPROVAL sets as C3.
    #[test]
    fn verb_class_census() {
        for sub in REPAIR_SUBVERBS {
            let n = VERB_CLASSES.iter().filter(|(v, _)| v == sub).count();
            assert_eq!(n, 1, "{sub} has {n} classes in VERB_CLASSES");
        }
        let local = [
            "gate",
            "rules",
            "boot-critical",
            "tombstones",
            "auth-helper",
        ];
        let mut table: Vec<&str> = VERB_CLASSES.iter().map(|(v, _)| *v).collect();
        table.sort_unstable();
        let mut expected: Vec<&str> = REPAIR_SUBVERBS.iter().copied().chain(local).collect();
        expected.sort_unstable();
        assert_eq!(
            table, expected,
            "VERB_CLASSES must be exactly the 23 daemon verbs + the 5 local verbs"
        );
        assert_eq!(VERB_CLASSES.len(), 28);
        let of = |c: VerbClass| -> Vec<&str> {
            let mut v: Vec<&str> = VERB_CLASSES
                .iter()
                .filter(|(_, k)| *k == c)
                .map(|(v, _)| *v)
                .collect();
            v.sort_unstable();
            v
        };
        let sorted = |mut v: Vec<&'static str>| {
            v.sort_unstable();
            v
        };
        assert_eq!(
            of(VerbClass::Work),
            sorted(vec![
                "begin", "record", "submit", "commit", "reapply", "recheck", "cleanup", "revert"
            ])
        );
        assert_eq!(
            of(VerbClass::OwnerDecision),
            sorted(vec!["grant", "owner_token", "grant_ask", "owner_token_ask"])
        );
        assert_eq!(
            of(VerbClass::OwnerApproval),
            sorted(vec!["approval_start", "approval_reject", "auth-helper"])
        );
        assert_eq!(
            of(VerbClass::Review),
            sorted(vec!["attest_verify", "attest_audit"])
        );
        assert_eq!(of(VerbClass::Trigger), sorted(vec!["sweep", "tick"]));
        assert_eq!(of(VerbClass::Intake), vec!["report"]);
        assert_eq!(
            of(VerbClass::Read),
            sorted(vec![
                "list",
                "status",
                "rule_doc_allowed",
                "approval_list",
                "gate",
                "rules",
                "boot-critical",
                "tombstones"
            ])
        );
        assert_eq!(verb_class("no-such-verb"), None);
        assert_eq!(
            verb_class("attest-verify"),
            None,
            "daemon verbs are listed by their wire names"
        );
    }

    /// `repair.status` through the real dispatcher reaches this module (no `method_not_found`), and a
    /// request outside the namespace still takes its old path.
    #[test]
    fn repair_prefix_is_delegated() {
        let daemon = crate::team_gate_tests::tmp_daemon("repair-t0-dispatch", false);
        let req = |m: &str| Request {
            id: json!(1),
            method: m.into(),
            params: json!({}),
        };
        // (T3 rewrite: `repair.status` is now implemented — a READ verb answers any caller)
        let Reply::Single(status) = crate::handlers::dispatch(&daemon, req("repair.status"), None)
        else {
            panic!("single reply")
        };
        assert_eq!(status["ok"], true, "{status}");
        assert!(status["result"]["lane"].is_string(), "{status}");
        let (code, _) = code_of(crate::handlers::dispatch(
            &daemon,
            req("repair.no_such_verb"),
            Some(std::process::id()),
        ));
        assert_eq!(code, "method_not_found");
        let Reply::Single(pong) = crate::handlers::dispatch(&daemon, req("system.ping"), None)
        else {
            panic!("single reply")
        };
        assert_eq!(
            pong["result"],
            json!("pong"),
            "a non-repair request must take its old path: {pong}"
        );
        let (code, _) = code_of(crate::handlers::dispatch(
            &daemon,
            req("repairs.status"),
            None,
        ));
        assert_ne!(
            code, "repair_not_implemented",
            "only the exact `repair.` prefix is delegated"
        );
    }

    /// The seven Daemon fields start at their defaults.
    #[test]
    fn daemon_repair_fields_default() {
        let daemon = crate::team_gate_tests::tmp_daemon("repair-t0-fields", false);
        assert!(!daemon.listener_ready.load(Ordering::SeqCst));
        assert!(!daemon.repair_tick_running.load(Ordering::SeqCst));
        assert_eq!(daemon.ready_since.load(Ordering::SeqCst), 0);
        assert_eq!(daemon.repair_last_tick.load(Ordering::SeqCst), 0);
        assert!(daemon
            .repair_prompt_origin
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_empty());
        assert!(daemon
            .repair_mutexes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_empty());
        let _table: &ApprovalTable = &daemon
            .repair_approvals
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        release_locks_for_seat(&daemon, 1); // nothing held → nothing touched
        assert!(daemon
            .repair_mutexes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_empty());
    }

    /// The library's `cso*` role prefix (roles::role_block_for) is the daemon's alert-route prefix.
    #[test]
    fn role_block_cso_prefix_equals_alert_route() {
        use cys::repair::roles::{role_block_for, RoleBlock};
        let prefix = crate::alert_route::CSO_ROLE_PREFIX;
        for role in [
            prefix.to_string(),
            format!("{prefix}-2"),
            format!("{prefix}x"),
        ] {
            assert_eq!(
                crate::alert_route::is_cso_role(&role),
                role_block_for(&role) == RoleBlock::Cso,
                "{role}"
            );
        }
        assert_eq!(role_block_for(prefix), RoleBlock::Cso);
    }
}
