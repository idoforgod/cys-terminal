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
// Track T5a — the T3 ↔ T5a seam on the REAL compiled lists (RULINGS AN1).
#[cfg(test)]
#[path = "repair_t5a_seam_tests.rs"]
mod t5a_seam_tests;

#[cfg(test)]
#[path = "repair_grant_tests.rs"]
mod grant_tests;

// Track T6 — `revert` / `reapply` through the verb dispatcher.
#[cfg(test)]
#[path = "repair_apply_tests.rs"]
mod t6_tests;

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
/// `owner_token_ask` Q2, `grant_status` CI6 (b)) — 24.
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
    "grant_status",
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

/// The sub-verbs whose bodies are track T4's (C9 grant, C10 owner token).
pub(crate) const T4_VERBS: &[&str] = &[
    "grant",
    "grant_ask",
    "owner_token",
    "owner_token_ask",
    "rule_doc_allowed",
    "grant_status",
];

/// The sub-verbs whose bodies are track T6's (C3 `revert` / `reapply`, C7 `tick` / `sweep`).
pub(crate) const T6_VERBS: &[&str] = &["revert", "reapply", "tick", "sweep"];

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

/// Drops a closed seat's binding (and its DG1 departure mark).
pub(crate) fn forget_role_family(lane: &str, incarnation: &str, sid: u64) {
    let key = (lane.to_string(), incarnation.to_string(), sid);
    role_bindings()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&key);
    family_departures()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&key);
}

/// RULINGS DG1 (T4b) — the seats that LEFT a role family in this daemon incarnation (lane,
/// incarnation, seat): a claim whose family differs from the seat's earlier role's family or from
/// its AD1 binding. A permanent mark for the seat's life in this incarnation (memory only — a
/// daemon restart releases every lock anyway, so nothing it authorised survives); dropped with the
/// seat. A leaf lock like the binding map.
fn family_departures() -> &'static Mutex<std::collections::HashSet<(String, String, u64)>> {
    static D: std::sync::OnceLock<Mutex<std::collections::HashSet<(String, String, u64)>>> =
        std::sync::OnceLock::new();
    D.get_or_init(|| Mutex::new(std::collections::HashSet::new()))
}

/// Whether the seat left a role family in this daemon incarnation (DG1).
pub(crate) fn left_role_family(lane: &str, incarnation: &str, sid: u64) -> bool {
    family_departures()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains(&(lane.to_string(), incarnation.to_string(), sid))
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
    let bound = bind_role_family(lane, incarnation, sid, previous)
        .or_else(|| bind_role_family(lane, incarnation, sid, Some(claimed)));
    // RULINGS DG1 (T4b): an incompatible family transition is remembered for the seat's life — a
    // seat that left the worker family never gets its repair's write authority back
    let claimed_family = role_family(claimed);
    let previous_family = previous.and_then(role_family);
    if (previous_family.is_some() && previous_family != claimed_family)
        || (bound.is_some() && bound != claimed_family)
    {
        family_departures()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert((lane.to_string(), incarnation.to_string(), sid));
    }
    bound
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
    /// The grant claim of K8 / G1 (`grant::try_claim_autonomous_start`, T4's, in production).
    pub claim: &'a ClaimFn<'a>,
    /// T4: the per-seat prompt record (`Daemon.repair_prompt_origin` in production — T3 writes it,
    /// the owner-decision verbs read it and register their questions in it).
    pub prompts: &'a Mutex<HashMap<u64, PromptOrigin>>,
    /// RULINGS HP2 (T4e): the product's pause is in force — the daemon's own kill-switch (`cys pause`)
    /// or the pack's `AUTOPILOT_PAUSED` mark ([`product_pause_in_force`] in production).
    pub product_pause: &'a dyn Fn() -> bool,
}

/// RULINGS HP2 (T4e) — the product's pause, as the pack's tools honour it: the daemon's own kill-switch
/// (`cys pause` → `system.pause`, `Daemon::paused`) or the file-convention mark `<pack>/AUTOPILOT_PAUSED`
/// (the wake queue's rule, its `paused_evidence`; the convention's setter writes it in the
/// pack and in the project's `_round/` together, and the daemon knows the pack).
pub(crate) fn product_pause_in_force(daemon: &Daemon, pack_dir: &Path) -> bool {
    daemon.paused.load(std::sync::atomic::Ordering::Relaxed) || pause_mark_present(pack_dir)
}

/// The pack's `AUTOPILOT_PAUSED` mark (RULINGS HP2), read the bounded way: one look at the name, no link
/// followed, no content read; present → paused, and anything but "not found" (it cannot be judged) →
/// paused too.
pub(crate) fn pause_mark_present(pack_dir: &Path) -> bool {
    match std::fs::symlink_metadata(pack_dir.join("AUTOPILOT_PAUSED")) {
        Ok(_) => true,
        Err(e) => e.kind() != std::io::ErrorKind::NotFound,
    }
}

/// RULINGS HP2 (T4e): an autonomous (`--trigger grant`) start refused under the product's pause.
pub(crate) const PRODUCT_PAUSE_START_KO: &str = "제품이 일시정지 중입니다(`cys pause` 또는 AUTOPILOT_PAUSED) — 자동 수리를 시작하지 않습니다(주인의 '고쳐라' 수리와 이미 시작한 수리의 commit · revert 는 됩니다)";
/// RULINGS HP2 (T4e): a new round of a grant repair refused under the product's pause (B4's rule for the
/// grant's own pause, applied to the product's).
pub(crate) const PRODUCT_PAUSE_ROUND_KO: &str = "제품이 일시정지 중입니다(`cys pause` 또는 AUTOPILOT_PAUSED) — 자동 수리의 새 라운드를 열지 않습니다(commit · revert 는 됩니다)";

/// The shape of `grant::try_claim_autonomous_start` (injected so a pick-up's claim answers —
/// CapReached, Busy … — are testable; RULINGS AQ11 / AW8).
pub(crate) type ClaimFn<'a> = dyn Fn(
        &cys::repair::types::LockOwner,
        &cys::repair::types::RepairId,
    ) -> Result<cys::repair::grant::StartVerdict, RepairError>
    + 'a;

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
    // T6 (C7 O-C7-2 (a)): the appended job's trigger answers at once from the daemon's own state.
    if sub == "tick" || sub == "sweep" {
        return Reply::Single(match role_check(sub, &caller, cfg!(windows)) {
            Ok(()) => ok_response(id, crate::repair_job::trigger(daemon, sub, params)),
            Err(e) => err_response(id, &e.code, &format!("repair.{sub}: {}", e.message)),
        });
    }
    let classify = |rel: &str| production_classify(rel, &cys::pack::pack_dir());
    let check = production_check;
    let product_pause = || product_pause_in_force(daemon, &cys::pack::pack_dir());
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
        claim: &cys::repair::grant::try_claim_autonomous_start,
        prompts: &daemon.repair_prompt_origin,
        product_pause: &product_pause,
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
        // T4 (C9, C10): the owner-decision verbs and the rule-document read
        "grant" => grant_verbs::grant(ctx, caller, params),
        "grant_ask" => grant_verbs::grant_ask(ctx, caller, params),
        "owner_token" => grant_verbs::owner_token(ctx, caller, params),
        "owner_token_ask" => grant_verbs::owner_token_ask(ctx, caller, params),
        "rule_doc_allowed" => grant_verbs::rule_doc_allowed(ctx, caller, params),
        "grant_status" => grant_verbs::grant_status(ctx, params),
        // T6 (C3, design §4-9): revert by rule one; reapply starts R′ (RULINGS AQ5 / AR1)
        "revert" => apply_verbs::revert(ctx, caller, params),
        "reapply" => apply_verbs::reapply(ctx, caller, params),
        // T6: the trigger needs the daemon's readiness / single-flight state — `handle` answers it
        "tick" | "sweep" => Err(VerbError::new(
            cys::repair::types::ERR_PRECONDITION,
            "데몬의 요청 처리기에서만 작업을 시작합니다",
        )),
        _ => {
            debug_assert!(
                !T3_VERBS.contains(&sub) && !T4_VERBS.contains(&sub) && !T6_VERBS.contains(&sub),
                "a T3 / T4 / T6 verb fell through to the stub"
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
    // RULINGS AD1 / AQ1: for every verb that is not READ, INTAKE or TRIGGER, a seat whose current
    // role family differs from the family it first held in this daemon incarnation is refused
    // (renaming a seat with `claim-role` does not change what it may do in the repair flow). A
    // TRIGGER verb (`sweep`, `tick`) only asks the daemon to run its own job and confers nothing —
    // the boot path's `repair.sweep` is never refused because a seat was re-declared.
    if !matches!(
        class,
        VerbClass::Read | VerbClass::Intake | VerbClass::Trigger
    ) {
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

/// Classification of one target from the two compiled lists (C4 · T5a · RULINGS AI6 / AN1 / AP4 /
/// BN1 / BN2): the protection list FIRST — a file in it is 보호장치 관련, also when it is
/// boot-critical too (the stricter class) — then the boot-critical list → 기동 필수, else 일반. A
/// path that matches a row only under the full case fold (ASCII case, U+212A, U+017F, … — BN1) or
/// another spelling of the separators is refused, naming the row's spelling: the check kind and
/// the timeout are looked up by the exact spelling; this holds whether or not the file exists.
/// The target is classified as `<pack_root>/<rel>` — the pack root of the lane the request is
/// served for (`cys::pack::pack_dir()`, the value the verb's `Ctx::pack_dir` holds) — never as a
/// relative path resolved against the daemon's working directory (BN2). A target whose live file is
/// the same file as a listed row's under another rel is refused naming the row (AW3 (d) — the
/// exported `boot_critical::refuse_identity_alias`, which `commit` step (3a) also calls). Either
/// list unavailable,
/// no absolute pack root, or a path that cannot be decided → an error: `begin` refuses (a
/// boot-critical or protection file is never repaired as a general one). Rule documents are T4's
/// gate.
fn production_classify(
    rel: &str,
    pack_root: &Path,
) -> Result<cys::repair::types::Classification, RepairError> {
    use cys::repair::boot_critical::{
        boot_critical_entry, classify_with, entries, pack_roots, protection_entries,
        protection_entry, Decision,
    };
    use cys::repair::types::Classification;
    if protection_entries().is_empty() || entries().is_empty() {
        return Err(RepairError::Precondition(format!(
            "{rel}: 컴파일된 기동 필수 · 보호장치 목록을 읽을 수 없어 시작하지 않습니다"
        )));
    }
    if !pack_root.is_absolute() {
        return Err(RepairError::Precondition(format!(
            "{rel}: 이 레인의 팩 위치를 알 수 없어 보호장치 · 기동 필수 여부를 가리지 못합니다 — 시작하지 않습니다"
        )));
    }
    let exact = |spelled: &str, row: &str| -> Result<(), RepairError> {
        if spelled == row {
            Ok(())
        } else {
            Err(RepairError::Precondition(format!(
                "{rel}: 목록의 표기는 `{row}` 입니다 — 같은 파일의 다른 표기(대소문자 · 링크 · `..`)로는 시작하지 않습니다. 목록의 표기 그대로 다시 시작하세요(검사 종류와 시한은 정확한 표기로 찾습니다)"
            )))
        }
    };
    if let Some(e) = protection_entry(rel) {
        exact(rel, e.rel)?;
        return Ok(Classification::ProtectionRelated);
    }
    // RULINGS AW3 (d): the live file is the SAME file (volume + file id) as a listed row's live file
    // under another rel (a hard link …) — refused naming the row; an absent file is not compared
    cys::repair::boot_critical::refuse_identity_alias(pack_root, [rel])
        .map_err(RepairError::Precondition)?;
    // RULINGS BN2: the lane's pack root joined with the rel (absolute — no working directory on
    // this path); every lane root is a candidate as before, the request's own root included
    let mut roots = pack_roots();
    if !roots.iter().any(|r| r == pack_root) {
        roots.push(pack_root.to_path_buf());
    }
    let a = classify_with(&pack_root.join(rel), None, &roots, cfg!(windows));
    if a.decision == Decision::Undecidable {
        return Err(RepairError::Precondition(format!(
            "{rel}: 보호장치 · 기동 필수 여부를 가릴 수 없어 시작하지 않습니다"
        )));
    }
    // RULINGS CY1: the classification names the row through ANY identity of the path (a link, `..`
    // after a link, a hard link); the REQUESTED spelling must be the row's own
    if let Some(e) = a.protection {
        exact(rel, e.rel)?;
        return Ok(Classification::ProtectionRelated);
    }
    if let Some(e) = boot_critical_entry(rel) {
        exact(rel, e.rel)?;
        return Ok(Classification::BootCritical);
    }
    match (a.decision, a.entry) {
        (Decision::Yes, Some(e)) => {
            exact(rel, e.rel)?;
            Ok(Classification::BootCritical)
        }
        (Decision::Yes, None) => Err(RepairError::Precondition(format!(
            "{rel}: 기동 필수 여부를 가릴 수 없어 시작하지 않습니다"
        ))),
        _ => Ok(Classification::General),
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

    /// RULINGS DJ2 / ED2 — test seams of `report` (test builds only), keyed by signature: the
    /// PAUSE holds one signature's report INSIDE the step the fix made atomic — after the hour's
    /// room was observed and before the place is taken, with the quota mutex held — until the test
    /// opens it (bounded: 20 s); the PROBE tells the test, for another signature, whether the
    /// quota mutex is held at the moment that report is about to take it (never blocking).
    #[cfg(test)]
    pub(crate) struct IntakeSeam {
        pause_sig: String,
        probe_sig: String,
        state: Mutex<(bool, bool, Option<bool>)>, // (arrived, open, probe: quota mutex held?)
        cv: std::sync::Condvar,
    }

    #[cfg(test)]
    static INTAKE_SEAM: Mutex<Option<std::sync::Arc<IntakeSeam>>> = Mutex::new(None);

    #[cfg(test)]
    impl IntakeSeam {
        /// Waits until the held report reached the pause (bounded).
        pub(crate) fn wait_arrived(&self, within: std::time::Duration) -> bool {
            let g = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let (g, _) = self
                .cv
                .wait_timeout_while(g, within, |s| !s.0)
                .unwrap_or_else(|e| e.into_inner());
            g.0
        }
        /// Waits for the probe's answer (bounded): `Some(true)` = the quota mutex was held.
        pub(crate) fn wait_probe(&self, within: std::time::Duration) -> Option<bool> {
            let g = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let (g, _) = self
                .cv
                .wait_timeout_while(g, within, |s| s.2.is_none())
                .unwrap_or_else(|e| e.into_inner());
            g.2
        }
        /// Lets the held report go on.
        pub(crate) fn open(&self) {
            self.state.lock().unwrap_or_else(|e| e.into_inner()).1 = true;
            self.cv.notify_all();
        }
    }

    /// Installs the seams: `pause_sig` is held at the pause, `probe_sig` answers the probe.
    #[cfg(test)]
    pub(crate) fn intake_seam_install(
        pause_sig: &str,
        probe_sig: &str,
    ) -> std::sync::Arc<IntakeSeam> {
        let s = std::sync::Arc::new(IntakeSeam {
            pause_sig: pause_sig.to_string(),
            probe_sig: probe_sig.to_string(),
            state: Mutex::new((false, false, None)),
            cv: std::sync::Condvar::new(),
        });
        *INTAKE_SEAM.lock().unwrap_or_else(|e| e.into_inner()) = Some(s.clone());
        s
    }

    /// Removes the seams.
    #[cfg(test)]
    pub(crate) fn intake_seam_clear() {
        *INTAKE_SEAM.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    #[cfg(test)]
    fn intake_seam() -> Option<std::sync::Arc<IntakeSeam>> {
        INTAKE_SEAM
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    #[cfg(test)]
    fn intake_seam_pause(sig: &str) {
        let Some(s) = intake_seam().filter(|s| s.pause_sig == sig) else {
            return;
        };
        let mut g = s.state.lock().unwrap_or_else(|e| e.into_inner());
        g.0 = true;
        s.cv.notify_all();
        let _ =
            s.cv.wait_timeout_while(g, std::time::Duration::from_secs(20), |st| !st.1)
                .unwrap_or_else(|e| e.into_inner());
    }

    #[cfg(test)]
    fn intake_seam_probe(sig: &str) {
        let Some(s) = intake_seam().filter(|s| s.probe_sig == sig) else {
            return;
        };
        let held = matches!(
            intake_quota().try_lock(),
            Err(std::sync::TryLockError::WouldBlock)
        );
        s.state.lock().unwrap_or_else(|e| e.into_inner()).2 = Some(held);
        s.cv.notify_all();
    }

    /// RULINGS DA2: takes one place in this lane's hour of new signatures if there is room — the
    /// check and the take are ONE step under the quota mutex (ED2: the test's pause sits inside
    /// it). `false` = the hour is full.
    fn reserve_intake(key: &(PathBuf, String), now: f64, sig: &str) -> bool {
        #[cfg(test)]
        intake_seam_probe(sig);
        #[cfg(not(test))]
        let _ = sig;
        let mut q = intake_quota().lock().unwrap_or_else(|e| e.into_inner());
        let e = q.entry(key.clone()).or_default();
        while e.0.front().is_some_and(|t| now - *t > 3600.0) {
            e.0.pop_front();
        }
        if e.0.len() >= intake::INTAKE_NEW_SIGNATURES_PER_HOUR {
            return false;
        }
        #[cfg(test)]
        intake_seam_pause(sig); // ED2: between observing room and taking it, the mutex held
        e.0.push_back(now);
        true
    }

    /// RULINGS DA2: gives back a place taken by [`reserve_intake`] (the row was not written).
    fn release_intake(key: &(PathBuf, String), now: f64) {
        let mut q = intake_quota().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(e) = q.get_mut(key) {
            if let Some(i) = e.0.iter().rposition(|t| *t == now) {
                e.0.remove(i);
            }
        }
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
        let label = caller.label();
        // DA3: the row's read-modify-write under the BOUNDED lock of its signature (a fixed set —
        // no map entry per request). DA2: whether the signature is new is read under that lock, and
        // a new one RESERVES its place in the hour's quota in one step (check + take under the
        // quota mutex); a write that fails returns the reservation.
        let outcome = {
            let _row_lock = super::intake_lock_for(&rep.signature);
            let fresh = intake::excluded(&rep.rel, error, ctx.windows).is_none()
                && intake::load_row(&intake::row_path_in(&ctx.root, &ctx.lane, &rep.signature))?
                    .is_none();
            let reserved = fresh && reserve_intake(&key, ctx.now, &rep.signature);
            let room = if fresh && !reserved {
                intake::INTAKE_NEW_SIGNATURES_PER_HOUR // no room: the record answers over quota
            } else {
                0
            };
            let r = intake::record_in(
                &ctx.root,
                &ctx.lane,
                &rep,
                &label,
                ctx.now,
                room,
                ctx.windows,
                error,
            );
            if reserved && !matches!(r, Ok(intake::Recorded::New(_))) {
                release_intake(&key, ctx.now);
            }
            r?
        };
        let mut out = json!({
            "signature": rep.signature,
            "rel": rep.rel,
            "error_class": rep.error_class,
            "lane": ctx.lane.0,
        });
        match outcome {
            intake::Recorded::New(row) | intake::Recorded::Updated(row) => {
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

    /// RULINGS V1 / AN1 / AP4 / AW6 / BN1: only a non-lifeline (8.7), non-protection (D:941) hook —
    /// the protection rows are the sibling list (AI6); lifeline and protection names compare after
    /// the full case fold, a name ending in a dot or a space or in a short-name form (`~<digit>`) is
    /// refused;
    /// while either compiled list is not available a protection hook cannot be told apart →
    /// refused (fail closed).
    pub(super) fn hook_allowed(script: &str) -> Result<(), VerbError> {
        // RULINGS AP4: a name ending in a dot or a space is the same file on APFS / NTFS — refused
        if script.ends_with('.') || script.ends_with(' ') {
            return Err(VerbError::new(
                cys::repair::types::ERR_PRECONDITION,
                format!("{script:?}: 끝이 점이나 공백인 훅 이름은 받지 않습니다 — 정확한 스크립트 이름으로 다시 하세요"),
            ));
        }
        // RULINGS AW6: a Windows short-name form (`ROLE-C~1.SH`) cannot be told apart from a
        // protection hook without the volume — refused (the case-folded protection lookup below
        // catches `GUARD.SH`)
        if script.split('.').next().is_some_and(|stem| {
            stem.char_indices().any(|(i, c)| {
                c == '~'
                    && stem[i + 1..]
                        .chars()
                        .next()
                        .is_some_and(|d| d.is_ascii_digit())
            })
        }) {
            return Err(VerbError::new(
                cys::repair::types::ERR_PRECONDITION,
                format!("{script}: 짧은 이름(8.3) 형태의 훅 이름은 받지 않습니다 — 정확한 스크립트 이름으로 다시 하세요"),
            ));
        }
        // RULINGS BN1: lifeline names compare after the full case fold (the protection lookup
        // below folds the same way through `rel_key`)
        let fold = cys::repair::types::fold_name;
        if LIFELINE_HOOKS.iter().any(|l| fold(l) == fold(script)) {
            return Err(VerbError::new(
                cys::repair::types::ERR_PRECONDITION,
                format!("{script}: 생명줄 훅의 등록은 수리로 해제할 수 없습니다"),
            ));
        }
        if entries().is_empty() || cys::repair::boot_critical::protection_entries().is_empty() {
            return Err(VerbError::new(
                cys::repair::types::ERR_PRECONDITION,
                format!("{script}: 기동 필수 목록을 읽을 수 없어 보호장치 훅인지 가릴 수 없습니다 — 거부"),
            ));
        }
        let rel = format!("hooks/{script}");
        if cys::repair::boot_critical::protection_entry(&rel)
            .is_some_and(|e| e.category == EntryCategory::Protection)
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
        let trigger = trigger_of(p)?;
        let (files, hooks) = begin_targets(p)?;
        // AU4 / BJ3, AK6, AW3 (b): the name and path rules over every named target, before anything
        // is read, classified or written (also for `--joins`, which checks every file of the joined
        // repair itself — BJ2).
        for rel in &files {
            target_rules(ctx, rel)?;
        }
        // DA4 / ED3: one file named twice is one target; the sentence says what is known — one
        // identity ("같은 파일입니다") or only the conservative rule (one file on some volumes)
        for (i, a) in files.iter().enumerate() {
            if let Some((b, how)) =
                files[i + 1..]
                    .iter()
                    .find_map(|b| match super::same_now(&ctx.pack_dir, a, b) {
                        super::Verdict::Different => None,
                        v => Some((b, v)),
                    })
            {
                return Err(VerbError::invalid(match how {
                    super::Verdict::Same(super::Same::Identity | super::Same::Spelling) => {
                        format!("{a} 와 {b} 는 같은 파일입니다 — 한 번만 지정하세요(RULINGS DA4)")
                    }
                    super::Verdict::Same(_) => format!(
                        "{a} 와 {b} 는 볼륨에 따라 같은 파일일 수 있습니다(대소문자만 다르거나 ASCII 가 아닌 아직 없는 이름) — 한 수리에 함께 지정하지 마세요(RULINGS DJ1 · ED3)"
                    ),
                    super::Verdict::Different | super::Verdict::Unknown => format!(
                        "{a} 와 {b} 가 같은 파일인지 확인할 수 없습니다(파일 시스템 조회 실패) — 한 수리에 함께 지정하지 마세요(RULINGS EG1)"
                    ),
                }));
            }
        }
        // T4 (C10 10.6 · AD6 replaced · AW8): the rule-document branch — a rule document (any
        // spelling that folds to one, also through `--joins`) starts only with a valid owner token.
        if let Some(v) = begin_rule_document(ctx, caller, p, trigger, &files, &hooks, joins, resolves)? {
            return Ok(v);
        }
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
        // Pick-up (G1 step 3 / AD7 / AQ11): only UNFINISHED work — a repair of this lane with the
        // same targets in a being-worked state of G2 (`진행 중`, `승인 대기`, `감사 지적으로 다시
        // 열림`) without a live lock. A pick-up changes ONLY the lock (AH2): the state (a `승인 대기`
        // repair stays `승인 대기`), the signatures, the working copy and the pre-images are untouched;
        // a grant repair without `--resolves` takes a new claim (K3). A repair in force (`시험 기간` /
        // `사용 중`) or in any other state is never picked up: a new `begin` starts a NEW repair with
        // its own pre-image — never a second open repair beside a being-worked one.
        let candidates: Vec<RepairRecord> = ledger::list_ids_in(&ctx.root)
            .into_iter()
            .filter_map(|id| match ledger::read_record_in(&ctx.root, &id) {
                Ok(Some(r))
                    if r.origin_lane == ctx.lane
                        && r.resolves == resolves
                        && pickable(&r.state)
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
                let still = pickable(&rec.state)
                    && lock::view(
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
                    claim = claim_for(ctx, owner, &rec.id)?;
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
        // BJ5: one open repair per file — a being-worked repair of this lane that shares a target
        // with this start but holds another target set (or another `--resolves`) is never doubled;
        // the repair this start resolves (`--resolves`, RULINGS T1) keeps its own rule.
        for oid in ledger::list_ids_in(&ctx.root) {
            let Ok(Some(r)) = ledger::read_record_in(&ctx.root, &oid) else {
                continue;
            };
            if r.origin_lane != ctx.lane || !pickable(&r.state) || resolves.as_ref() == Some(&r.id)
            {
                continue;
            }
            let theirs = record_key(&r);
            if r.resolves == resolves && theirs == key {
                continue; // a pick-up candidate (handled above)
            }
            // DA4 / DJ1: the same FILE, not the same spelling; for a name that does not exist yet
            // the conservative key (ASCII case fold; one creating repair per folder for non-ASCII)
            for k in key {
                if let Some(why) = super::open_overlap(&ctx.pack_dir, k, &r) {
                    return Err(VerbError::new(cys::repair::types::ERR_PRECONDITION, why));
                }
            }
        }
        // A new repair — created ALREADY LOCKED through the ledger's seam (AD9 S1 / G1 "lock first"):
        // the id is allocated, the record is built (claim, pre-images, staging) and written with its
        // lock before anything else can see it; a failure removes the still-empty id directory.
        let id = ledger::create_locked_in(&ctx.root, |id| -> Result<RepairRecord, VerbError> {
            let id = id.clone();
            let claim = if trigger == Trigger::Grant && resolves.is_none() {
                claim_for(ctx, owner, &id)?
            } else {
                None
            };
            let mut changes = Vec::new();
            for rel in files {
                // BJ4: the parent chain once more right before the pre-image is copied
                cys::repair::types::parent_chain_link(&ctx.pack_dir, rel)?;
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
                // BJ4: and right before the working copy is populated from the live files
                for rel in files {
                    cys::repair::types::parent_chain_link(&ctx.pack_dir, rel)?;
                }
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
                reapplies: None, // T6 (AQ5): `reapply` sets it on R′ after this body created R′
            };
            Ok(rec)
        })?;
        let rec = load(ctx, &id)?;
        Ok(begin_reply(ctx, &rec, "new"))
    }

    /// T4 — THE rule-document branch of `begin` (C10 10.3 / 10.6; RULINGS AD6 replaced, AW8, BJ3, F10,
    /// F11, O-C10-2, O-C10-5). `None` = not a rule-document begin (no target folds to a rule document,
    /// no `--joins` of a rule-document repair, no `--owner-token`): T3's flow continues. Otherwise the
    /// begin is refused or done HERE: every target a rule document (no mixing, no hook line), never
    /// `--joins`, trigger `owner-order` only (the grant never covers rule documents), a valid token of
    /// THIS lane, unexpired, covering every target and not bound to another repair; then T3's
    /// `begin_locked` with `RuleDocument` (its pre-image copy into the vault IS the backup — O-C10-5)
    /// and the binding — the record's `owner_token_sha256`, the token's `repair` / `used_by_repair`.
    /// Every refusal names the real file (BJ3) and writes nothing.
    #[allow(clippy::too_many_arguments)]
    fn begin_rule_document(
        ctx: &Ctx<'_>,
        caller: &Caller,
        p: &Value,
        trigger: Trigger,
        files: &[String],
        hooks: &[(String, String)],
        joins: Option<&str>,
        resolves: Option<&str>,
    ) -> Result<Option<Value>, VerbError> {
        use cys::repair::owner_origin as oo;
        use cys::repair::types::{ERR_CORRUPT, ERR_LOCKED, ERR_PRECONDITION};
        let token = param_str(p, "owner_token");
        let named = |rel: &str| -> Option<String> {
            let doc = super::rule_document_name(rel)?;
            let base = rel.rsplit(['/', '\\']).next().unwrap_or(rel);
            let real = super::real_name_in(&ctx.pack_dir, rel).unwrap_or(doc);
            Some(if real == base {
                rel.to_string()
            } else {
                format!("{rel} (= {real})")
            })
        };
        let docs: Vec<String> = files.iter().filter_map(|r| named(r)).collect();
        let joined_doc = joins
            .and_then(|j| RepairId::parse(j).ok())
            .is_some_and(|jid| {
                matches!(ledger::read_record_in(&ctx.root, &jid),
                    Ok(Some(ref r)) if r.classification == Classification::RuleDocument)
            });
        if docs.is_empty() && !joined_doc {
            if token.is_some() {
                return Err(VerbError::invalid(
                    "--owner-token 은 규칙 문서(soul.md·CLAUDE.md·*_DIRECTIVE.md …) 수리에만 씁니다",
                ));
            }
            return Ok(None);
        }
        let list = if docs.is_empty() {
            joins.unwrap_or_default().to_string()
        } else {
            docs.join(", ")
        };
        let refuse = |why: String| -> Result<Option<Value>, VerbError> {
            Err(VerbError::new(ERR_PRECONDITION, why))
        };
        if joins.is_some() {
            return refuse(format!("규칙 문서는 병합 모드(--joins)로 합칠 수 없습니다: {list} — 이 부서에서 주인 지시 표를 받아 `--owner-token` 으로 새 수리를 시작하세요(C10)"));
        }
        if docs.len() != files.len() || !hooks.is_empty() {
            return refuse(format!("규칙 문서({list})와 다른 파일·훅 등록 해제를 한 수리에 섞지 마세요 — 규칙 문서 수리는 주인 지시 표로 따로 시작합니다(C10)"));
        }
        if trigger != Trigger::OwnerOrder {
            return refuse(format!("규칙 문서({list})는 주인의 지시로만 고칩니다 — 자동 수리 허락·업데이트는 규칙 문서에 미치지 않습니다(`--trigger owner-order` 와 주인 지시 표 · C10 10.3)"));
        }
        let Some(token) = token else {
            return refuse(format!("규칙 문서 수리에는 주인 지시 표가 필요합니다: {list} — 마스터가 `cys repair owner-token --path …` 로 받은 표를 티켓대로 `--owner-token` 에 넘기세요(C10 10.6)"));
        };
        if !oo::is_token_shape(token) {
            return refuse(format!(
                "주인 지시 표의 형식이 아닙니다(ot- 뒤 26자): {list}"
            ));
        }
        for rel in files {
            match (ctx.classify)(rel)? {
                Classification::General | Classification::RuleDocument => {}
                other => {
                    return refuse(format!(
                        "{rel}: 규칙 문서이면서 '{}' 로 분류된 파일입니다 — 1차 출시의 주인 지시 표 경로는 이 조합을 다루지 않습니다",
                        other.label_ko()
                    ))
                }
            }
        }
        let resolves = match resolves {
            Some(r) => {
                let rid = RepairId::parse(r).map_err(|_| {
                    VerbError::invalid(format!("--resolves 값이 수리 번호가 아닙니다: {r:?}"))
                })?;
                load(ctx, &rid)?;
                Some(rid)
            }
            None => None,
        };
        let sha = oo::token_sha256(token);
        let sid = caller
            .seat()
            .ok_or_else(|| VerbError::refused("좌석이 없습니다"))?;
        let owner = ctx.owner(sid);
        let key = target_key(files, &[]);
        ctx.locked("begin:lane", || -> Result<Option<Value>, VerbError> {
            let tok = match oo::read_token_in(&ctx.root, &sha) {
                Ok(Some(t)) => t,
                Ok(None) => {
                    return refuse(format!(
                        "이 주인 지시 표는 이 설치에서 발급된 적이 없습니다: {list}"
                    ))
                }
                Err(e) => {
                    return Err(VerbError::new(
                        ERR_CORRUPT,
                        format!(
                            "주인 지시 표를 읽을 수 없어 시작하지 않습니다: {}",
                            VerbError::from(e).message
                        ),
                    ))
                }
            };
            // the repair this begin would pick up (T3's pick-up set: this lane, being worked, the same
            // targets and `--resolves`) — a token is used by ONE repair (O-C10-2)
            let mut bound_to: Option<RepairId> = None;
            for id in ledger::list_ids_in(&ctx.root) {
                let Ok(Some(r)) = ledger::read_record_in(&ctx.root, &id) else {
                    continue;
                };
                if r.origin_lane != ctx.lane
                    || !pickable(&r.state)
                    || r.resolves != resolves
                    || record_key(&r) != key
                {
                    continue;
                }
                match r.owner_token_sha256.as_deref() {
                    Some(x) if x == sha => bound_to = Some(r.id.clone()),
                    Some(_) => {
                        return refuse(format!(
                            "{}: 이 파일의 열린 규칙 문서 수리는 다른 주인 지시 표로 시작됐습니다 — 그 표로 이어받으세요",
                            r.id.as_str()
                        ))
                    }
                    None => {}
                }
            }
            oo::check_token_for_begin(&tok, &ctx.lane, files, ctx.now, bound_to.as_ref())
                .map_err(|m| VerbError::new(ERR_PRECONDITION, format!("{m}: {list}")))?;
            let v = begin_locked(
                ctx,
                &owner,
                Classification::RuleDocument,
                Trigger::OwnerOrder,
                files,
                &[],
                resolves.clone(),
                &key,
            )?;
            let picked = v
                .get("begin")
                .and_then(|b| b.as_str())
                .unwrap_or("new")
                .to_string();
            let id = v
                .get("repair")
                .and_then(|x| x.as_str())
                .and_then(|x| RepairId::parse(x).ok())
                .ok_or_else(|| VerbError::new("io", "begin 응답에 수리 번호가 없습니다"))?;
            ctx.locked(id.as_str(), || -> Result<(), VerbError> {
                let mut rec = load(ctx, &id)?;
                if lock::view(rec.lock.as_ref(), &owner, ctx.now, &ctx.lane, &ctx.incarnation)
                    != LockView::Mine
                {
                    return Err(VerbError::new(
                        ERR_LOCKED,
                        format!("{}: 이 수리의 잠금이 이 좌석의 것이 아닙니다", id.as_str()),
                    ));
                }
                if rec.classification != Classification::RuleDocument {
                    let _ = ledger::release_lock_in(&ctx.root, &id, &owner, None);
                    return Err(VerbError::new(
                        ERR_PRECONDITION,
                        format!(
                            "{}: 규칙 문서 수리가 아닌 열린 수리입니다 — 주인 지시 표를 묶지 않습니다",
                            id.as_str()
                        ),
                    ));
                }
                rec.owner_token_sha256 = Some(sha.clone());
                write(ctx, &rec)?;
                let mut t = tok.clone();
                t.repair = Some(id.clone());
                t.used_by_repair = Some(id.clone());
                oo::write_token_in(&ctx.root, &t).map_err(VerbError::from)
            })?;
            let rec = load(ctx, &id)?;
            let mut out = begin_reply(ctx, &rec, &picked);
            out["owner_token_bound"] = json!(true);
            Ok(Some(out))
        })
    }

    /// T4 — RULINGS AQ8 (AI9; D:937 "CSO 지침을 고치는 수리 후보가 등록 표지를 지우면 데몬이 거부한다"):
    /// `record` and `commit` refuse a repair whose target is the CSO directive when the pre-image's
    /// first 20 lines hold the revision marker the registrar reads and the candidate's do not
    /// (`owner_origin::CSO_DIRECTIVE_REV_MARKER`, parity-tested against `javis_preflight.py`). AS1: the
    /// CSO directive is a rule document and never a protection row — this is its only guard.
    fn cso_marker_refusal(
        ctx: &Ctx<'_>,
        rec: &RepairRecord,
        states: &[(String, TargetState)],
    ) -> Result<(), VerbError> {
        use cys::repair::owner_origin as oo;
        let staging = ledger::staging_dir_in(&ctx.root, &rec.id);
        for f in file_targets(rec) {
            if oo::folded_base(&f.rel) != oo::CSO_DIRECTIVE_NAME {
                continue;
            }
            let Some(pre) = slot_payload(&f.pre, &f.rel).and_then(|p| std::fs::read(p).ok())
            else {
                continue; // absent (or a link) before the repair — no marker to keep
            };
            if !oo::cso_marker_in_head(&String::from_utf8_lossy(&pre)) {
                continue;
            }
            let keeps = match states.iter().find(|(r, _)| *r == f.rel) {
                Some((_, TargetState::File { .. })) => {
                    let path = if is_approval_class(rec.classification) {
                        staging.join(&f.rel)
                    } else {
                        live_path(ctx, &f.rel)
                    };
                    std::fs::read(&path)
                        .map(|b| oo::cso_marker_in_head(&String::from_utf8_lossy(&b)))
                        .unwrap_or(false)
                }
                _ => false,
            };
            if !keeps {
                return Err(VerbError::new(
                    cys::repair::types::ERR_PRECONDITION,
                    oo::cso_marker_refusal_ko(&f.rel),
                ));
            }
        }
        Ok(())
    }

    /// The name and path rules of ONE target (used by `begin` for every named file and by
    /// `begin --joins` for every file of the joined repair): a pack-relative path without Windows
    /// short names (AW3 (a)); not the locked key file in any spelling that folds to it (AU4 / BJ3 —
    /// the sentence names the real file); no Windows alias part (AK6); no link / junction /
    /// cloud placeholder in its parent chain inside the pack (AW3 (b)).
    fn target_rules(ctx: &Ctx<'_>, rel: &str) -> Result<(), VerbError> {
        intake::normalize_rel(rel).map_err(|e| VerbError::invalid(VerbError::from(e).message))?;
        if let Some(locked) = cys::repair::types::system_locked_entry(rel) {
            let named = if rel == locked {
                locked.to_string()
            } else {
                format!("{rel:?} (= {locked})")
            };
            return Err(VerbError::new(
                cys::repair::types::ERR_PRECONDITION,
                format!(
                    "{named}: 서명 키 파일은 수리 대상이 아닙니다 — 사본이 손상됐다면 `cys pack-heal {locked}` 로 내장본을 되살립니다(RULINGS AU4)"
                ),
            ));
        }
        if super::is_windows_alias(rel) {
            return Err(VerbError::invalid(format!(
                "{rel:?}: 끝이 점·공백이거나 ':' 가 든 경로 조각은 받지 않습니다(윈도우에서 다른 파일의 별칭이 됩니다)"
            )));
        }
        cys::repair::types::parent_chain_link(&ctx.pack_dir, rel)?;
        // DA1: the FILE decides — a name that is no rule document must not be another name of one
        // (or of a locked file); a rule-document name goes through the owner's token (T4's branch).
        if super::rule_document_name(rel).is_none() {
            if let Some(why) = super::identity_refusal(&ctx.pack_dir, rel) {
                return Err(VerbError::new(cys::repair::types::ERR_PRECONDITION, why));
            }
        }
        Ok(())
    }

    /// The lane's live states of `rels` (no-follow reads, after the parent chains were checked).
    fn current_live_states(
        ctx: &Ctx<'_>,
        rels: &[String],
    ) -> Result<Vec<(String, TargetState)>, VerbError> {
        rels.iter()
            .map(|rel| -> Result<(String, TargetState), VerbError> {
                Ok((rel.clone(), TargetState::read(&live_path(ctx, rel))?))
            })
            .collect()
    }

    /// RULINGS DA5 (O-C1-6 / O-C1-7): `commit` writes this lane's row; a row that exists but is
    /// corrupt or of a newer schema is never overwritten — refused with the reason, bytes kept.
    fn lane_row_readable(ctx: &Ctx<'_>, id: &RepairId) -> Result<(), VerbError> {
        match ledger::read_lane_in(&ctx.root, id, &ctx.lane) {
            Ok(_) => Ok(()),
            Err(e) => Err(VerbError::new(
                cys::repair::types::ERR_CORRUPT,
                format!(
                    "{}: 이 부서의 수리 행({})을 읽을 수 없습니다(손상되었거나 더 새 형식) — 원본을 그대로 두고 확정하지 않습니다: {}",
                    id.as_str(),
                    ledger::lane_path_in(&ctx.root, id, &ctx.lane).display(),
                    VerbError::from(e).message
                ),
            )),
        }
    }

    /// RULINGS AW3 (b) / (c) over a repair's file targets: no link / junction in a target's parent
    /// chain inside the lane's pack (the leaf may be a link — U2); with `states`, a `Link` state is
    /// refused when it points to a directory or into any lane's pack.
    fn path_rules(
        ctx: &Ctx<'_>,
        rec: &RepairRecord,
        states: Option<&[(String, TargetState)]>,
    ) -> Result<(), VerbError> {
        for f in file_targets(rec) {
            cys::repair::types::parent_chain_link(&ctx.pack_dir, &f.rel)?;
            // DA1: re-checked at `record` (and `commit`) — a target may have become another name of
            // a rule document after `begin`; a rule-document repair is the owner's token route.
            if rec.classification != Classification::RuleDocument {
                if let Some(why) = super::identity_refusal(&ctx.pack_dir, &f.rel) {
                    return Err(VerbError::new(cys::repair::types::ERR_PRECONDITION, why));
                }
            }
        }
        for (rel, st) in states.unwrap_or(&[]) {
            if let TargetState::Link { target } = st {
                if let Some(why) =
                    cys::repair::types::link_target_refusal(&ctx.pack_dir, rel, target)
                {
                    return Err(VerbError::new(cys::repair::types::ERR_PRECONDITION, why));
                }
            }
        }
        Ok(())
    }

    /// RULINGS AD7 / AQ11: the being-worked states of G2 a lock-less repair can be picked up in.
    fn pickable(state: &RepairState) -> bool {
        matches!(
            state,
            RepairState::InProgress | RepairState::AwaitingApproval | RepairState::ReopenedByAudit
        )
    }

    /// K8 / G1 (T4): the grant claim for an autonomous start. Until T4 lands the stub answers
    /// not-implemented and the start is refused.
    fn claim_for(
        ctx: &Ctx<'_>,
        owner: &cys::repair::types::LockOwner,
        id: &RepairId,
    ) -> Result<Option<u32>, VerbError> {
        use cys::repair::grant::StartVerdict;
        // RULINGS HP2 (T4e): the product's pause (`cys pause` or AUTOPILOT_PAUSED) refuses an autonomous start
        // before the grant is asked — in addition to the grant's own pause (B4)
        if (ctx.product_pause)() {
            return Err(VerbError::new(cys::repair::types::ERR_PRECONDITION, super::PRODUCT_PAUSE_START_KO));
        }
        match (ctx.claim)(owner, id)? {
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
        // BJ2: the name / parent-chain rules over EVERY file of the joined repair (not only the
        // ones named in `--files`) before anything is read or copied; then the link rule on the
        // lane's current states of those files.
        for rel in &rels {
            target_rules(ctx, rel)?;
        }
        let live_states = current_live_states(ctx, &rels)?;
        path_rules(ctx, &rec, Some(live_states.as_slice()))?;
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
                    // RULINGS HP2 (T4e): the product's pause refuses a new round as the grant's own does (B4)
                    if (ctx.product_pause)() {
                        return Err(VerbError::new(
                            cys::repair::types::ERR_PRECONDITION,
                            format!("{}: {}", id.as_str(), super::PRODUCT_PAUSE_ROUND_KO),
                        ));
                    }
                    // B4 / K3: a new round of a grant-triggered repair asks T4 (paused → refused).
                    if !cys::repair::grant::may_open_new_round(&id)? {
                        // CH7 (T4): the sentence says whether the grant is off or paused
                        return Err(VerbError::new(cys::repair::types::ERR_PRECONDITION, cys::repair::grant::new_round_refusal(&id)));
                    }
                }
                rec.attempt += 1;
            }
            path_rules(ctx, &rec, None)?;
            let states = current_states(ctx, &rec)?;
            path_rules(ctx, &rec, Some(states.as_slice()))?;
            cso_marker_refusal(ctx, &rec, &states)?; // T4 (AQ8)
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
        // T6 · RULINGS EJ4 / EN6: the repair's ONE guard for the whole commit, its hook phase included
        let Some(_guard) = cys::repair::guard::RepairGuard::try_take(&ctx.root, &id) else {
            return Err(VerbError::new(cys::repair::types::ERR_LOCKED, format!("{}: {}", id.as_str(), cys::repair::guard::BUSY)));
        };
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
            // DA5: this lane's row must be absent or readable — a corrupt or newer-schema row is never
            // overwritten (O-C1-6 / O-C1-7): refused BEFORE any state transition.
            lane_row_readable(ctx, &id)?;
            path_rules(ctx, &rec, None)?;
            let states = current_states(ctx, &rec)?;
            path_rules(ctx, &rec, Some(states.as_slice()))?;
            cso_marker_refusal(ctx, &rec, &states)?; // T4 (AQ8)
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
            path_rules(ctx, &rec, Some(states.as_slice()))?;
            cys::repair::boot_critical::refuse_identity_alias(&ctx.pack_dir, file_targets(&rec).map(|f| f.rel.as_str())).map_err(|why| VerbError::new(cys::repair::types::ERR_PRECONDITION, why))?; // RULINGS AW3 (d)
            lane_row_readable(ctx, &id)?; // DA5
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
            // DA5: read again — a row that became unreadable since step (3a) is left as it is
            let previous = match ledger::read_lane_in(&ctx.root, &id, &ctx.lane) {
                Ok(p) => p,
                Err(e) => {
                    warnings.push(format!(
                        "이 부서의 수리 행을 읽을 수 없게 되어 그대로 두었습니다(덮어쓰지 않음) — {}",
                        VerbError::from(e).message
                    ));
                    return Ok(reply(ctx, &rec, json!({
                        "committed": committed,
                        "audit_due_at": rec.audit_due_at,
                        "hook_step": if !has_hooks { json!(null) } else if let Some(e) = &step_error { json!({"error": e}) } else { json!("ok") },
                        "superseded": superseded.iter().map(|r| r.as_str().to_string()).collect::<Vec<_>>(),
                        "warnings": warnings,
                        "lane_row": "unreadable",
                    })));
                }
            };
            let mut applied_history = previous.as_ref().map(|r| r.applied_history.clone()).unwrap_or_default();
            if let Some(prev) = &previous {
                if !prev.applied.is_empty() && prev.applied != committed {
                    applied_history.push(prev.applied.clone());
                }
            }
            // T6 · RULINGS GK4 (the EN8 seam; FI1 on the general commit): pre-images taken in another
            // generation than the one live NOW, or in an unknown one → the commit stands, the row says
            // it cannot be returned automatically, and it is never offered to the automatic return
            let not_revertible = cys::repair::apply::begun_in_another_generation(&rec, &ctx.pack_dir);
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
                    closed: not_revertible.then(cys::repair::apply::not_revertible_closure),
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

    /// RULINGS AK4 / AW9 / C1 1.6 — scope: only the ORIGIN lane's (this daemon's) older in-force
    /// records are superseded here; another lane's rows of those repairs close through their own
    /// probation binding. Marks 다른 수리로 대체됨 every repair of THIS lane (one writer per record —
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
            // T6 · RULINGS FG5 (the EN6 seam): an older repair is written only under ITS guard, tried
            // — held (a re-check / return / recovery owns it) → left for later (its own re-check sees
            // the live file changed; the job's start pass supersedes it)
            let Some(_older_guard) = cys::repair::guard::RepairGuard::try_take(&ctx.root, &oid) else {
                continue;
            };
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
        // T6 · RULINGS GK3: the owner notices closed "not confirmed" (handed out three times without
        // their receiver's confirmation) stay visible here
        out["notices_not_confirmed"] = match cys::repair::probation::notices_not_confirmed_in(&ctx.root, &ctx.lane) {
            Ok(keys) => json!(keys),
            Err(e) => json!({"unreadable": VerbError::from(e).message}),
        };
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
        // T6b · RULINGS HS2: a probation an update ended is named by its closure (a true sentence —
        // never "다른 수리로 대체됨" when no other repair replaced it)
        let update_sentence = cys::repair::apply::update_closure_sentence(&rec, rows.iter().find(|r| r.lane == rec.origin_lane));
        let mut v = reply(
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
                "reapplies": rec.reapplies.as_ref().map(|r| r.as_str().to_string()), // T6 (AQ5)
                "committed": rec.committed,
                "pending_request": rec.pending_request,
                "resubmits": rec.resubmits,
                "lane_rows": merges,
                "lane_rows_unreadable": broken,
                "vault_slots": slots,
                "staging": if is_approval_class(rec.classification) || staging.exists() { json!(staging) } else { Value::Null },
                "intake_signatures": rec.intake_signatures,
            }),
        );
        if let Some(sentence) = update_sentence {
            v["state_label"] = json!(sentence);
        }
        Ok(v)
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

// Track T6 — `revert` (C3 · design §4-9, the rule-one engine of `cys::repair::apply`) and `reapply`
// (RULINGS AQ5 / AR1: a NEW repair R′ through `begin`'s own body). An inline module like
// `ledger_verbs` (the C8 dormancy census follows production FILE modules from `main.rs` only).
// ★Nothing here may panic; nothing here sends anything to a queue (F1).
mod apply_verbs {
    use super::{Caller, Ctx, VerbError};
    use cys::repair::apply::{revert_in, ApplyOutcome, NoFaults, OpEnv, RevertCaller};
    use cys::repair::ledger;
    use cys::repair::platform::ApplyForm;
    use cys::repair::types::{RepairError, RepairId, RepairState};
    use serde_json::{json, Value};

    fn repair_id(p: &Value) -> Result<RepairId, VerbError> {
        let raw = p
            .get("repair")
            .and_then(|v| v.as_str())
            .ok_or_else(|| VerbError::invalid("수리 번호(R-<n>)가 필요합니다"))?;
        RepairId::parse(raw)
            .map_err(|_| VerbError::invalid(format!("수리 번호는 R-<숫자> 형식이어야 합니다: {raw:?}")))
    }

    fn outcome_json(o: &ApplyOutcome) -> Value {
        match o {
            ApplyOutcome::Applied => json!({"outcome": "applied"}),
            ApplyOutcome::Superseded(m) => json!({"outcome": "superseded", "why": m}),
            ApplyOutcome::Declined(m) => json!({"outcome": "declined", "why": m}),
        }
    }

    /// `cys repair revert <R-id>` / the app's "직전 수리 되돌리기" — a human (M4 per OS, the app's
    /// token on both) or a worker seat of a lane holding a row (T3's `worker_lane_check`; the role table
    /// refuses master / CEO / CSO / reviewers). While the grant is paused this is accepted (B4 / K3 /
    /// CH3: a running repair may still CLOSE). The existing pack-apply lock is TRIED, never waited for
    /// in a request (the CLI waits 40 s): held by `cys pack-update` → refused, nothing changed.
    pub(crate) fn revert(ctx: &Ctx<'_>, caller: &Caller, p: &Value) -> Result<Value, VerbError> {
        // CT11: `{"latest": true}` (the app's "직전 수리 되돌리기") — the most recently APPLIED repair
        // of THIS lane, chosen by this lane's application evidence (the time its row records the
        // apply here), never by repair number or by origin
        let picked;
        let p = if p.get("repair").is_none_or(|v| v.is_null()) && p.get("latest").and_then(|v| v.as_bool()) == Some(true) {
            let Some(id) = latest_applied_here(ctx) else {
                return Err(VerbError::new(
                    cys::repair::types::ERR_PRECONDITION,
                    "되돌릴 수리가 없습니다(이 부서에 적용된 시험 기간·사용 중인 수리가 없습니다)",
                ));
            };
            let mut q = p.clone();
            q["repair"] = json!(id.as_str());
            picked = q;
            &picked
        } else {
            p
        };
        let rec = super::ledger_verbs::worker_lane_check(ctx, caller, p)?;
        let id = rec.id.clone();
        let by = match caller.seat() {
            Some(seat) => RevertCaller::TicketedWorker { seat },
            None => RevertCaller::Human,
        };
        let form = if ctx.windows {
            ApplyForm::DepositFirst
        } else {
            cys::repair::platform::apply_form()
        };
        let env = OpEnv {
            root: &ctx.root,
            pack: &ctx.pack_dir,
            lane: &ctx.lane,
            now: ctx.now,
            form,
            faults: &NoFaults,
            own_check: &crate::repair_job::own_check,
        };
        let locked = |rid: &RepairId, f: &mut dyn FnMut() -> Result<(), RepairError>| {
            ctx.locked(rid.as_str(), f)
        };
        let lock_file = cys::pack_apply_lock::pack_apply_lock_path_for(&ctx.pack_dir);
        let ran = cys::pack_apply_lock::try_with_apply_lock(&lock_file, || {
            revert_in(&env, &id, by, ctx.hook_step, &locked)
        })
        .map_err(|e| VerbError::new("io", e))?;
        let Some(result) = ran else {
            return Err(VerbError::new(
                cys::repair::types::ERR_LOCKED,
                format!(
                    "{}: 다른 적용이나 업데이트가 팩을 바꾸는 중이라 지금은 되돌리지 않습니다(아무것도 바꾸지 않았습니다) — 그 작업이 끝난 뒤 다시 하세요",
                    id.as_str()
                ),
            ));
        };
        let report = result.map_err(VerbError::from)?;
        Ok(json!({
            "repair": id.as_str(),
            "reverted": report.outcome == ApplyOutcome::Applied,
            "result": outcome_json(&report.outcome),
            "files": report.files.iter().map(|(rel, o)| { let mut v = outcome_json(o); v["rel"] = json!(rel); v }).collect::<Vec<_>>(),
            "hook_step": match &report.hook_step { None => Value::Null, Some(Ok(())) => json!("ok"), Some(Err(e)) => json!({"error": e}) },
        }))
    }

    /// CT11 / DX1: the repair whose apply in THIS lane is the most recent (its row's `applied_at`)
    /// among those `revert_in` accepts — asked of THE predicate `cys::repair::apply::revertible_here`
    /// (a probation closed 실패 included); no state list of its own.
    pub(crate) fn latest_applied_here(ctx: &Ctx<'_>) -> Option<RepairId> {
        let mut best: Option<(f64, RepairId)> = None;
        for id in ledger::list_ids_in(&ctx.root) {
            let Ok(Some(row)) = ledger::read_lane_in(&ctx.root, &id, &ctx.lane) else {
                continue;
            };
            let Ok(Some(rec)) = ledger::read_record_in(&ctx.root, &id) else {
                continue;
            };
            if !cys::repair::apply::revertible_here(&rec, &row, &ctx.lane) {
                continue;
            }
            let at = row.probation.as_ref().map(|p| p.applied_at).unwrap_or(row.updated_at);
            if best.as_ref().is_none_or(|(t, _)| at > *t) {
                best = Some((at, id));
            }
        }
        best.map(|(_, id)| id)
    }

    fn being_worked(s: RepairState) -> bool {
        matches!(
            s,
            RepairState::InProgress | RepairState::AwaitingApproval | RepairState::ReopenedByAudit
        )
    }

    /// `cys repair reapply <R-id>` (RULINGS AQ5, AR1; WORK — a worker seat of this lane): R′ is created
    /// through `begin`'s own body with the files of R that the update PARKED in THIS lane's pack — a
    /// `.merge-pending.json` entry of kind `vendor-first` whose side copy (`<rel>.user`, written by the
    /// install core's vendor-first arm, `src/pack.rs` `park_user_copy`) is a regular file — with
    /// `trigger: update`; the vendor bytes now live are R′'s pre-image; `reapplies: R` is recorded on
    /// R′. R is not re-opened and not locked. Refused: no parked copy of R here; a repair with
    /// `reapplies: R` unfinished or in force (it reaches this lane by propagation); a being-worked
    /// repair of this lane already holds one of the files (the ordinary `begin` rule). RULINGS CQ2: when
    /// no target is parked but a target's `<rel>.user` IS on disk without a `vendor-first` record naming
    /// it (e.g. the record was replaced by `kept-drift`), the refusal names that copy and the route that
    /// works — `cys repair begin --files <rel> --trigger update` — instead of "nothing to re-apply".
    pub(crate) fn reapply(ctx: &Ctx<'_>, caller: &Caller, p: &Value) -> Result<Value, VerbError> {
        if !caller.is_worker() {
            return Err(VerbError::refused(
                "다시 입히기(reapply)는 이 부서의 워커 좌석만 합니다",
            ));
        }
        let r_id = repair_id(p)?;
        let r = ledger::read_record_in(&ctx.root, &r_id)
            .map_err(VerbError::from)?
            .ok_or_else(|| VerbError::not_found(format!("{}: 그런 수리가 없습니다", r_id.as_str())))?;
        let pending = cys::pack::load_merge_pending(&ctx.pack_dir);
        let mut parked: Vec<(String, std::path::PathBuf)> = Vec::new();
        // CQ2: a regular `<rel>.user` on disk that no `vendor-first` record names
        let mut unrecorded: Vec<(String, std::path::PathBuf)> = Vec::new();
        let mut foreign: Vec<(String, std::path::PathBuf)> = Vec::new();
        let is_regular = |p: &std::path::Path| std::fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_file());
        for f in ledger::file_targets(&r) {
            let copy_at_rel = ctx.pack_dir.join(format!("{}.user", f.rel));
            let vendor_first = pending
                .get(&f.rel)
                .filter(|e| e.get("kind").and_then(|k| k.as_str()) == Some("vendor-first"));
            let Some(e) = vendor_first else {
                if is_regular(&copy_at_rel) {
                    unrecorded.push((f.rel.clone(), copy_at_rel));
                }
                continue;
            };
            let side = e
                .get("side")
                .and_then(|s| s.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| format!("{}.user", f.rel));
            let path = ctx.pack_dir.join(&side);
            if is_regular(&path) {
                // CU4: the parked copy must be R's OWN edit (its bytes = R's repaired state of the
                // file); a copy of something else is never re-applied as R
                let own = match &f.post {
                    cys::repair::types::TargetState::File { sha256, .. } => cys::repair::types::TargetState::read(&path)
                        .ok()
                        .is_some_and(|st| matches!(st, cys::repair::types::TargetState::File { sha256: s, .. } if s == *sha256)),
                    _ => false,
                };
                if own {
                    parked.push((f.rel.clone(), path));
                } else {
                    foreign.push((f.rel.clone(), path));
                }
            } else if path != copy_at_rel && is_regular(&copy_at_rel) {
                // the record names another side copy that is gone; `<rel>.user` is not the one it names
                unrecorded.push((f.rel.clone(), copy_at_rel));
            }
        }
        if parked.is_empty() && !foreign.is_empty() {
            let copies = foreign.iter().map(|(_, p)| p.display().to_string()).collect::<Vec<_>>().join(", ");
            let rels = foreign.iter().map(|(rel, _)| rel.as_str()).collect::<Vec<_>>().join(" ");
            return Err(VerbError::new(
                cys::repair::types::ERR_PRECONDITION,
                format!(
                    "{id}: 보관 사본 {copies} 은 {id} 의 수정본이 아닙니다(내용이 다릅니다) — 다시 입히기(reapply)로는 입히지 않습니다. 그 사본을 살리는 길은 수리입니다: cys repair begin --files {rels} --trigger update (작업 사본에서 <파일>.user 를 합친 뒤 검증·감사 서명 → 주인 최종 승인)",
                    id = r_id.as_str()
                ),
            ));
        }
        if parked.is_empty() && !unrecorded.is_empty() {
            let copies = unrecorded
                .iter()
                .map(|(_, p)| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            let rels = unrecorded.iter().map(|(rel, _)| rel.as_str()).collect::<Vec<_>>().join(" ");
            return Err(VerbError::new(
                cys::repair::types::ERR_PRECONDITION,
                format!(
                    "{id}: 보관 사본 {copies} 는 그대로 있지만 그 사본을 가리키는 vendor-first 기록이 없어 다시 입히기(reapply)로는 입히지 않습니다 — 그 사본을 살리는 길은 수리입니다: cys repair begin --files {rels} --trigger update (작업 사본에서 <파일>.user 를 합친 뒤 검증·감사 서명 → 주인 최종 승인)",
                    id = r_id.as_str()
                ),
            ));
        }
        if parked.is_empty() {
            return Err(VerbError::new(
                cys::repair::types::ERR_PRECONDITION,
                format!(
                    "{id}: 이 부서의 팩에는 업데이트가 보관한 {id} 의 수정본이 없습니다(vendor-first 기록과 보관 사본 <파일>.user 가 함께 있어야 합니다) — 다시 입힐 것이 없습니다",
                    id = r_id.as_str()
                ),
            ));
        }
        let ids = ledger::list_ids_in(&ctx.root);
        for oid in &ids {
            let Ok(Some(o)) = ledger::read_record_in(&ctx.root, oid) else {
                continue;
            };
            // AR1: one re-application serves every lane
            if o.reapplies.as_ref() == Some(&r_id)
                && (being_worked(o.state) || matches!(o.state, RepairState::Probation | RepairState::InUse))
            {
                return Err(VerbError::new(
                    cys::repair::types::ERR_PRECONDITION,
                    // T6b · RULINGS IL2: what is true for the class of that repair (its record is at hand)
                    format!(
                        "{r}: 이미 {o} 가 {r} 의 수정본을 다시 입히고 있습니다(상태: {s}) — 두 번 입히지 않습니다. {reach}",
                        r = r_id.as_str(),
                        o = o.id.as_str(),
                        s = o.state.label_ko(),
                        reach = cys::repair::apply::reach_of_other_departments(o.classification, o.id.as_str())
                    ),
                ));
            }
            // the ordinary `begin` rule: one open repair per file
            if o.origin_lane == ctx.lane
                && being_worked(o.state)
                && o.files.iter().any(|f| parked.iter().any(|(rel, _)| *rel == f.rel))
            {
                let shared = o
                    .files
                    .iter()
                    .find(|f| parked.iter().any(|(rel, _)| *rel == f.rel))
                    .map(|f| f.rel.clone())
                    .unwrap_or_default();
                return Err(VerbError::new(
                    cys::repair::types::ERR_PRECONDITION,
                    format!(
                        "{shared}: 이 파일을 고치는 수리 {o} 가 이미 열려 있습니다 — 그 수리를 이어받거나 끝낸 뒤 다시 하세요(한 파일에 열린 수리는 하나뿐입니다)",
                        o = o.id.as_str()
                    ),
                ));
            }
        }
        // CT8 / DR4 — ONE re-application per parked edit ACROSS lanes, by the grant's claim protocol:
        // (1) bind R′ first — `begin` (R′ holds its own lock) and `reapplies: R` under R′'s mutex;
        // (2) place claim k naming R′ (no-replace); (3) after placement, our claim must not be
        // closed. A refusal at (2)/(3) closes R′ (truthfully: overtaken) and answers with the holder.
        let files: Vec<String> = parked.iter().map(|(rel, _)| rel.clone()).collect();
        let mut out = super::ledger_verbs::begin(ctx, caller, &json!({"files": files, "trigger": "update"}))?;
        let new_id = out
            .get("repair")
            .and_then(|x| x.as_str())
            .and_then(|s| RepairId::parse(s).ok())
            .ok_or_else(|| VerbError::new("io", "새 수리 번호를 읽을 수 없습니다"))?;
        ctx.locked(new_id.as_str(), || -> Result<(), VerbError> {
            let Some(mut rec) = ledger::read_record_in(&ctx.root, &new_id).map_err(VerbError::from)? else {
                return Err(VerbError::new("io", format!("{}: 방금 만든 수리 기록을 읽을 수 없습니다", new_id.as_str())));
            };
            rec.reapplies = Some(r_id.clone());
            ledger::write_record_durable_in(&ctx.root, &rec).map_err(VerbError::from)
        })?;
        let holder_of = |h: &RepairId| match ledger::read_record_in(&ctx.root, h) {
            Ok(Some(o)) if being_worked(o.state) || matches!(o.state, RepairState::Probation | RepairState::InUse) => {
                cys::repair::reapply_claim::Holder::Alive
            }
            Ok(Some(_)) => cys::repair::reapply_claim::Holder::Ended,
            // unreadable holder evidence (or a holder that is gone) — assume held (G7)
            Ok(None) | Err(_) => cys::repair::reapply_claim::Holder::Unreadable,
        };
        let refused = match cys::repair::reapply_claim::claim_in(&ctx.root, &r_id, &ctx.lane, &new_id, ctx.now, &holder_of) {
            Ok(_) => None,
            Err(cys::repair::reapply_claim::ClaimRefused::Held { holder }) => {
                // T6b · RULINGS IL2: the holder's record (read here already for its state) gives its
                // class; when it cannot be read, R's own class (the edit it re-applies) is the one at hand
                let held = ledger::read_record_in(&ctx.root, &holder).ok().flatten();
                let s = held.as_ref().map(|o| o.state.label_ko()).unwrap_or("확인할 수 없음");
                let class = held.as_ref().map(|o| o.classification).unwrap_or(r.classification);
                Some(format!(
                    "{r}: 이미 {o} 가 {r} 의 수정본을 다시 입히고 있습니다(상태: {s}) — 두 번 입히지 않습니다. {reach}",
                    r = r_id.as_str(),
                    o = holder.as_str(),
                    reach = cys::repair::apply::reach_of_other_departments(class, holder.as_str())
                ))
            }
            // T6b · RULINGS IL2: the winner is not at hand — R's own class (the edit it re-applies) is
            Err(cys::repair::reapply_claim::ClaimRefused::LostRace) | Err(cys::repair::reapply_claim::ClaimRefused::ClosedAfterPlacement { .. }) => Some(format!(
                "{r}: 다른 부서가 방금 {r} 의 수정본을 다시 입히기 시작했습니다 — 두 번 입히지 않습니다. {reach}",
                r = r_id.as_str(),
                reach = cys::repair::apply::reach_of_other_departments(r.classification, "<그 수리 번호 — cys repair status>")
            )),
            Err(cys::repair::reapply_claim::ClaimRefused::Io(e)) => Some(format!("{}: 다시 입히기 주장을 남기지 못했습니다 — {e}", r_id.as_str())),
        };
        if let Some(why) = refused {
            // the bound R′ is closed: it never re-applies anything
            let _ = ctx.locked(new_id.as_str(), || -> Result<(), VerbError> {
                if let Some(mut rec) = ledger::read_record_in(&ctx.root, &new_id).map_err(VerbError::from)? {
                    rec.state = RepairState::Superseded;
                    rec.lock = None;
                    rec.methods.push(format!("다시 입히기 주장에서 밀려 닫음 — {why}").chars().take(300).collect());
                    ledger::write_record_durable_in(&ctx.root, &rec).map_err(VerbError::from)?;
                }
                Ok(())
            });
            return Err(VerbError::new(cys::repair::types::ERR_PRECONDITION, why));
        }
        out["reapplies"] = json!(r_id.as_str());
        out["parked"] = json!(parked
            .iter()
            .map(|(rel, path)| json!({"rel": rel, "parked_copy": path}))
            .collect::<Vec<_>>());
        Ok(out)
    }
}

// Track T4 — the owner-decision verbs `grant` / `grant_ask` / `owner_token` / `owner_token_ask` and
// the READ verb `rule_doc_allowed` (C9, C10; RULINGS B1, B4, F10, F11, Q1, Q2, T3, T5, U4). An inline
// module for the same reason as `ledger_verbs` (the C8 census follows production file modules only
// from `main.rs`).
mod grant_verbs {
    //! The master seat only conveys the OWNER's decision (the role table admits only a master seat
    //! whose role family is unchanged — J1, AK1); every decision takes effect only after the daemon's
    //! own owner-origin check over the per-seat prompt record (`owner_origin`, fail closed). None of
    //! these verbs writes or changes an `R-*` record (F10). ★No queue / alert / bus traffic (F1) and
    //! no panic paths (F4).

    use super::{Caller, Ctx, OpenQuestion, PromptOrigin, VerbError};
    use cys::repair::grant::{self, GrantDecision, GrantState};
    use cys::repair::owner_origin::{self as oo, PromptView, QuestionView};
    use cys::repair::types::{Classification, RepairError, RepairId, ERR_PRECONDITION};
    use cys::repair::{intake, ledger, lock};
    use serde_json::{json, Value};
    use std::path::Path;

    fn param_str<'a>(p: &'a Value, k: &str) -> Option<&'a str> {
        p.get(k).and_then(|v| v.as_str())
    }

    fn seat(caller: &Caller) -> Result<u64, VerbError> {
        caller
            .seat()
            .ok_or_else(|| VerbError::refused("좌석이 없습니다"))
    }

    /// A copy of the seat's prompt record (taken under the lock; judged without it).
    fn record_of(ctx: &Ctx<'_>, sid: u64) -> Option<PromptOrigin> {
        ctx.prompts
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&sid)
            .cloned()
    }

    fn question_view(q: &OpenQuestion) -> QuestionView<'_> {
        QuestionView {
            asked_at: q.asked_at,
            path: q.path.as_deref(),
            first_human: q
                .first_human
                .as_ref()
                .map(|(d, e, at)| (d.as_str(), e.as_str(), *at)),
            machine_seen: q.machine_seen,
        }
    }

    fn prompt_view(r: &PromptOrigin) -> PromptView<'_> {
        PromptView {
            digest: &r.digest,
            origin: &r.origin,
            at: r.at,
            question: r.question.as_ref().map(question_view),
            named_docs: &r.named_docs,
            last_human: r.last_human.as_ref().map(|(d, at)| (d.as_str(), *at)),
            last_typed: r.last_typed.as_ref().map(|(d, at)| (d.as_str(), *at)),
            named_numbers: &r.named_numbers,
        }
    }

    /// Consumes the seat's open question if it is still the one judged (same `asked_at`); `true` =
    /// this call consumed it (a concurrent call that consumed it first leaves `false`).
    fn consume_question(ctx: &Ctx<'_>, sid: u64, asked_at: f64) -> bool {
        let mut map = ctx.prompts.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(r) = map.get_mut(&sid) {
            if r.question.as_ref().is_some_and(|q| q.asked_at == asked_at) {
                r.question = None;
                return true;
            }
        }
        false
    }

    /// RULINGS CI3: takes the naming of `base` out of the seat's latest Level-2 prompt — only while
    /// that prompt is still the one judged (`typed` = its digest and time) and still names `base`;
    /// `true` = this call took it (one naming, one token — a concurrent second call gets `false`).
    fn take_naming(ctx: &Ctx<'_>, sid: u64, base: &str, typed: (&str, f64)) -> bool {
        let mut map = ctx.prompts.lock().unwrap_or_else(|e| e.into_inner());
        let Some(r) = map.get_mut(&sid) else {
            return false;
        };
        if r.last_typed.as_ref().map(|(d, at)| (d.as_str(), *at)) != Some(typed) {
            return false;
        }
        let before = r.named_docs.len();
        r.named_docs.retain(|n| oo::folded_base(n) != base);
        r.named_docs.len() != before
    }

    /// Registers ONE open question for the seat (a newer one replaces an older one — Q1); a grant
    /// question carries its daily limit (CI4). The seat's typing mark is cleared (CJ2: the typing that
    /// answers a question comes after the question).
    fn register(ctx: &Ctx<'_>, sid: u64, path: Option<String>, daily_cap: Option<u32>) -> Value {
        let mut map = ctx.prompts.lock().unwrap_or_else(|e| e.into_inner());
        let r = map.entry(sid).or_default();
        r.typing_mark = None;
        let replaced = r.question.is_some();
        r.question = Some(OpenQuestion {
            asked_at: ctx.now,
            path: path.clone(),
            first_human: None,
            machine_seen: false,
            daily_cap,
            // CV1: a grant answer is good only while this is still the current generation
            asked_generation: if path.is_none() {
                grant::current_generation_in(&ctx.root)
            } else {
                0
            },
        });
        json!({
            "asked_at": ctx.now,
            "lapses_at": ctx.now + oo::QUESTION_LAPSE_SECS as f64,
            "path": path,
            "replaced_open_question": replaced,
        })
    }

    /// The daily limit of the current decision (5 without one — C9; a stored 0 reads as the default).
    fn current_cap(current: Option<&GrantDecision>) -> u32 {
        current
            .map(|d| d.daily_cap)
            .filter(|c| *c > 0)
            .unwrap_or(grant::DEFAULT_DAILY_CAP)
    }

    /// `daily_cap` of a request: absent / null → `None`; an integer 1–100 → `Some`; else refused.
    fn cap_param(p: &Value) -> Result<Option<u32>, VerbError> {
        match p.get("daily_cap") {
            None | Some(Value::Null) => Ok(None),
            Some(v) => v
                .as_u64()
                .filter(|n| (1..=u64::from(grant::MAX_DAILY_CAP)).contains(n))
                .map(|n| Some(n as u32))
                .ok_or_else(|| {
                    VerbError::invalid(format!(
                        "--daily-cap 은 1~{} 사이의 정수입니다",
                        grant::MAX_DAILY_CAP
                    ))
                }),
        }
    }

    /// RULINGS CI4: the limit is set only by `grant ask --daily-cap` (it belongs to the question).
    const CAP_ON_ASK_ONLY_KO: &str = "하루 상한은 질문에 딸립니다 — `cys repair grant ask --daily-cap N`으로 질문을 등록할 때 정하세요(`grant on`은 주인이 허락한 그 질문의 상한을 쓰고, `grant off`·`grant status`는 상한을 바꾸지 않습니다)";

    /// `pack` + an absolute path → the pack-relative path ('/'-separated), also through the
    /// canonical forms of both (a path outside the pack → `None`).
    fn pack_relative(pack: &Path, raw: &Path) -> Option<String> {
        let strip = |base: &Path, p: &Path| -> Option<String> {
            p.strip_prefix(base)
                .ok()
                .map(|r| r.to_string_lossy().replace('\\', "/"))
                .filter(|r| !r.is_empty())
        };
        if let Some(r) = strip(pack, raw) {
            return Some(r);
        }
        let cpack = std::fs::canonicalize(pack).ok()?;
        let cparent = std::fs::canonicalize(raw.parent()?).ok()?;
        strip(&cpack, &cparent.join(raw.file_name()?))
    }

    /// The pack-relative rule document a `--path` names (absolute paths inside this lane's pack are
    /// accepted — `guard.sh` passes the tool's absolute path). Not a rule document → refused.
    pub(super) fn doc_rel(ctx: &Ctx<'_>, raw: &str) -> Result<String, VerbError> {
        let raw = raw.trim();
        let p = Path::new(raw);
        let rel = if p.is_absolute() {
            pack_relative(&ctx.pack_dir, p).ok_or_else(|| {
                VerbError::invalid(format!(
                    "{raw}: 이 부서의 팩 안 경로가 아닙니다 — 규칙 문서는 팩 기준 상대 경로로 적으세요"
                ))
            })?
        } else {
            raw.to_string()
        };
        let rel = intake::normalize_rel(&rel)
            .map_err(|e| VerbError::invalid(VerbError::from(e).message))?;
        if super::is_windows_alias(&rel) {
            return Err(VerbError::invalid(format!(
                "{rel:?}: 끝이 점·공백이거나 ':' 가 든 경로 조각은 받지 않습니다"
            )));
        }
        if super::rule_document_name(&rel).is_none() {
            return Err(VerbError::invalid(format!(
                "{rel}: 규칙 문서(soul.md·CLAUDE.md·*_DIRECTIVE.md·VIBECODING_*·ROUTE_CONTRACT)가 아닙니다 — 주인 지시 표는 규칙 문서에만 씁니다"
            )));
        }
        Ok(rel)
    }

    /// `cys repair grant ask [--daily-cap N]` (RULINGS Q1, CI4): registers the grant question for
    /// this master seat with its daily limit — N if given, else the current decision's limit (5
    /// without one); an N different from that needs the seat's latest Level-2 prompt to hold N as a
    /// number of its own. The answer is the FIRST Level-2 prompt to this seat after it.
    pub(crate) fn grant_ask(ctx: &Ctx<'_>, caller: &Caller, p: &Value) -> Result<Value, VerbError> {
        let sid = seat(caller)?;
        let requested = cap_param(p)?;
        let current = current_cap(grant::read_decision_in(&ctx.root).unwrap_or(None).as_ref());
        let record = record_of(ctx, sid);
        let view = record.as_ref().map(prompt_view);
        let cap = oo::judge_daily_cap(view.as_ref(), current, requested)
            .map_err(|m| VerbError::new(ERR_PRECONDITION, m))?;
        let mut v = register(ctx, sid, None, Some(cap));
        v["question"] = json!("grant");
        v["daily_cap"] = json!(cap);
        // CJ5: the question the master asks names the limit
        v["ask_owner"] = json!(oo::grant_question_ko(cap));
        Ok(v)
    }

    /// `cys repair owner-token ask --path <doc>` (RULINGS Q2 (b)): registers a token question for
    /// exactly that rule document.
    pub(crate) fn owner_token_ask(
        ctx: &Ctx<'_>,
        caller: &Caller,
        p: &Value,
    ) -> Result<Value, VerbError> {
        let sid = seat(caller)?;
        let raw = param_str(p, "path").ok_or_else(|| VerbError::invalid("`path` 가 필요합니다"))?;
        let rel = doc_rel(ctx, raw)?;
        let mut v = register(ctx, sid, Some(rel), None);
        v["question"] = json!("owner_token");
        Ok(v)
    }

    /// `cys repair grant on|off` (C9 · RULINGS B1, Q1, T3, T5, U4(b), CI4, CI5 (a), CV1): written to
    /// `grant.json` (atomic replace) only after the owner-origin check, in ONE step with that check
    /// and the question's consumption under the shared grant lock, with the next generation.
    /// `on` takes the daily limit of the answered question; `off` keeps the current limit; neither
    /// takes `--daily-cap` (CI4 — refused with the sentence naming `grant ask --daily-cap`).
    pub(crate) fn grant(ctx: &Ctx<'_>, caller: &Caller, p: &Value) -> Result<Value, VerbError> {
        let sid = seat(caller)?;
        let mode = param_str(p, "mode").ok_or_else(|| {
            VerbError::invalid("`mode` 가 필요합니다(on · off — 질문 등록은 `grant ask`)")
        })?;
        if !matches!(p.get("daily_cap"), None | Some(Value::Null)) {
            return Err(VerbError::invalid(CAP_ON_ASK_ONLY_KO));
        }
        if !matches!(mode, "on" | "off") {
            return Err(VerbError::invalid(format!(
                "알 수 없는 mode: {mode:?} (on · off)"
            )));
        }
        let refuse = |m: String| RepairError::Precondition(m);
        // RULINGS CV1: validate the answer, take the question and publish the decision as ONE step
        // under the grant lock every lane daemon takes (`grant::decide_in`); the seat's record is
        // judged and its question taken inside one section of the record lock. A decision that is
        // not published gives the question back.
        let mut taken: Option<OpenQuestion> = None;
        let published = grant::decide_in(
            &ctx.root,
            std::time::Duration::from_millis(grant::GRANT_LOCK_WAIT_MS),
            grant::REAL_LOCK_CLOCK,
            &|_| {},
            |cur| {
                let mut map = ctx.prompts.lock().unwrap_or_else(|e| e.into_inner());
                let (decision, take) = {
                    let record = map.get(&sid);
                    let view = record.map(prompt_view);
                    let question = record.and_then(|r| r.question.as_ref());
                    if mode == "on" {
                        if cur.unreadable {
                            return Err(refuse("자동 수리 허락 파일(grant.json)을 읽을 수 없어 켜지 않습니다 — 끄기(`grant off`)는 됩니다".into()));
                        }
                        let a = oo::judge_grant_on(view.as_ref(), ctx.now).map_err(refuse)?;
                        // CV1: the answer is good only while no decision was published after the
                        // question was asked (in any lane)
                        if question.map(|q| q.asked_generation) != Some(cur.generation) {
                            return Err(refuse(GRANT_ASKED_BEFORE_KO.to_string()));
                        }
                        (
                            GrantDecision {
                                schema: cys::repair::REPAIR_SCHEMA,
                                state: GrantState::On,
                                // CI4: the limit of the answered question (registered by `grant ask`)
                                daily_cap: question
                                    .and_then(|q| q.daily_cap)
                                    .filter(|c| (1..=grant::MAX_DAILY_CAP).contains(c))
                                    .unwrap_or_else(|| current_cap(cur.decision)),
                                decided_at: ctx.now,
                                decided_lane: ctx.lane.clone(),
                                decided_seat: sid,
                                prompt_digest: a.digest,
                                prompt_excerpt: a.excerpt,
                                // the owner's answer is itself an owner input (B4); CI5 (a): from NOW
                                paused_at: Some(ctx.now),
                                resumed_at: Some(ctx.now + grant::PAUSE_QUIET_SECS as f64),
                                generation: 0, // set by `decide_in`
                            },
                            true,
                        )
                    } else {
                        let a = oo::judge_grant_off(view.as_ref(), cur.decision.map(|d| d.decided_at))
                            .map_err(refuse)?;
                        (
                            GrantDecision {
                                schema: cys::repair::REPAIR_SCHEMA,
                                state: GrantState::Off,
                                daily_cap: current_cap(cur.decision), // CI4: never changes the limit
                                decided_at: ctx.now,
                                decided_lane: ctx.lane.clone(),
                                decided_seat: sid,
                                prompt_digest: a.digest,
                                prompt_excerpt: a.excerpt,
                                paused_at: None,
                                resumed_at: None,
                                generation: 0, // set by `decide_in`
                            },
                            question.is_some_and(|q| q.path.is_none()),
                        )
                    }
                };
                if take {
                    taken = map.get_mut(&sid).and_then(|r| r.question.take());
                }
                Ok(decision)
            },
        );
        let decision = match published {
            Ok(d) => d,
            Err(e) => {
                if let Some(q) = taken {
                    let mut map = ctx.prompts.lock().unwrap_or_else(|e| e.into_inner());
                    let r = map.entry(sid).or_default();
                    if r.question.is_none() {
                        r.question = Some(q);
                    }
                }
                return Err(e.into());
            }
        };
        Ok(json!({
            "grant": decision.state,
            "daily_cap": decision.daily_cap,
            "decided_at": decision.decided_at,
            "decided_lane": decision.decided_lane,
            "paused_until": decision.resumed_at,
            "generation": decision.generation,
        }))
    }

    /// RULINGS CV1: a `grant on` whose question was asked before the current decision was published.
    const GRANT_ASKED_BEFORE_KO: &str = "질문을 등록한 뒤에 다른 결정(켬·끔)이 먼저 났습니다 — 그 답으로는 켜지 않습니다. `cys repair grant ask`로 다시 물으세요";

    /// `cys repair owner-token [--repair <R-id>] --path <doc>` (C10 · RULINGS F10, F11, Q2, T3, U4):
    /// after the owner-origin check, a scoped token `ot-<base32 128 bit>` for exactly that path, valid
    /// 24 h, single repair, usable by any worker seat of THIS lane; shown once, stored as its hash.
    pub(crate) fn owner_token(
        ctx: &Ctx<'_>,
        caller: &Caller,
        p: &Value,
    ) -> Result<Value, VerbError> {
        let sid = seat(caller)?;
        let raw = param_str(p, "path").ok_or_else(|| VerbError::invalid("`path` 가 필요합니다"))?;
        let rel = doc_rel(ctx, raw)?;
        let repair = match param_str(p, "repair") {
            None => None,
            Some(r) => {
                let id = RepairId::parse(r).map_err(|_| {
                    VerbError::invalid(format!("--repair 값이 수리 번호가 아닙니다: {r:?}"))
                })?;
                match ledger::read_record_in(&ctx.root, &id) {
                    Ok(Some(rec))
                        if rec.origin_lane == ctx.lane
                            && rec.classification == Classification::RuleDocument => {}
                    Ok(Some(_)) => {
                        return Err(VerbError::new(
                            ERR_PRECONDITION,
                            format!(
                                "{}: 이 부서의 규칙 문서 수리가 아닙니다",
                                id.as_str()
                            ),
                        ))
                    }
                    Ok(None) => {
                        return Err(VerbError::not_found(format!(
                            "{}: 그런 수리가 없습니다",
                            id.as_str()
                        )))
                    }
                    Err(e) => return Err(e.into()),
                }
                Some(id)
            }
        };
        let record = record_of(ctx, sid);
        let view = record.as_ref().map(prompt_view);
        let (basis, a) = oo::judge_owner_token(view.as_ref(), &rel, ctx.now)
            .map_err(|m| VerbError::new(ERR_PRECONDITION, m))?;
        // CI3: the basis is TAKEN before the token exists (one naming / one answer → one token, also
        // for two concurrent requests); a failure after this point costs the basis (fail closed)
        let taken = match basis {
            oo::TokenBasis::NamedInPrompt => {
                take_naming(ctx, sid, &oo::folded_base(&rel), (a.digest.as_str(), a.at))
            }
            oo::TokenBasis::AnsweredQuestion => record
                .as_ref()
                .and_then(|r| r.question.as_ref())
                .is_some_and(|q| consume_question(ctx, sid, q.asked_at)),
        };
        if !taken {
            return Err(VerbError::new(
                ERR_PRECONDITION,
                format!("이 근거(주인이 문서를 부른 말 또는 질문의 답)는 방금 다른 요청이 먼저 썼습니다 — 표는 근거 하나에 하나입니다. 주인이 다시 {} 를 부르거나 `cys repair owner-token ask --path {rel}` 뒤 허락해야 합니다", oo::folded_base(&rel)),
            ));
        }
        let hex = crate::channels::random_token_hex()
            .map_err(|e| VerbError::new("io", format!("난수를 만들 수 없어 표를 만들지 않습니다: {e}")))?;
        let mut bytes = [0u8; 16];
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = hex
                .get(2 * i..2 * i + 2)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
                .ok_or_else(|| VerbError::new("io", "난수 형식 오류"))?;
        }
        let token = oo::token_from_bytes(&bytes);
        let record_t = oo::OwnerToken {
            schema: cys::repair::REPAIR_SCHEMA,
            token_sha256: oo::token_sha256(&token),
            lane_key: ctx.lane.clone(),
            issued_to_seat: sid,
            paths: vec![rel.clone()],
            repair,
            prompt_digest: a.digest,
            prompt_excerpt: a.excerpt,
            issued_at: ctx.now,
            expires_at: ctx.now + oo::TOKEN_TTL_SECS as f64,
            used_by_repair: None,
        };
        oo::create_token_in(&ctx.root, &record_t)?;
        Ok(json!({
            "token": token,
            "path": rel,
            "expires_at": record_t.expires_at,
            "basis": match basis {
                oo::TokenBasis::NamedInPrompt => "named_in_owner_prompt",
                oo::TokenBasis::AnsweredQuestion => "answered_question",
            },
            // O-C10-3: the fixed ticket line (the delegation tool prints it — T7b)
            "ticket_line": format!("주인 지시 표: {token} (규칙 문서 {rel})"),
        }))
    }

    /// `cys repair grant status [--json]` (READ · RULINGS CI6 (b)): the grant as readers need it —
    /// `{state: on|off, paused, paused_until, daily_cap}`. Any caller the READ class admits (any seat
    /// of the lane), no owner check; writes and sends nothing. `paused` is the B4 clock (the last
    /// owner input less than 30 minutes old) whatever the state — a reader combines it with `state`
    /// (CI6 (c): held while ON and PAUSED). A decision file that exists but cannot be read is an
    /// error (the reader then behaves as when the state cannot be read); no decision = off.
    pub(crate) fn grant_status(ctx: &Ctx<'_>, p: &Value) -> Result<Value, VerbError> {
        if !matches!(p.get("daily_cap"), None | Some(Value::Null)) {
            return Err(VerbError::invalid(CAP_ON_ASK_ONLY_KO));
        }
        let d = grant::read_decision_in(&ctx.root)?;
        // DK1 (2) / DM1: the later of this daemon's memory and the written marks (each until its
        // covered-through time + 30 min); an unreadable pause
        // directory = paused for this read; the hold after this daemon's start (DG3 (c)). Writes nothing.
        let st = grant::pause_state_in(&ctx.root, ctx.now);
        let hold = grant::incarnation_started_at(&ctx.incarnation)
            .map(|s| s + grant::PAUSE_QUIET_SECS as f64)
            .filter(|h| ctx.now < *h);
        let paused = st.paused_at(ctx.now) || hold.is_some();
        let until = [st.deadline.filter(|t| ctx.now < *t), hold]
            .into_iter()
            .flatten()
            .reduce(f64::max);
        Ok(json!({
            "state": if grant::is_on(d.as_ref()) { "on" } else { "off" },
            "paused": paused,
            "paused_until": if paused { until } else { None },
            "daily_cap": current_cap(d.as_ref()),
        }))
    }

    /// RULINGS CV2: the refusal of a rule-document write asked by a seat that is not a worker now, or
    /// whose role family changed (a master never implements).
    pub(super) const RULE_DOC_WORKER_ONLY_KO: &str = "규칙 문서는 워커 좌석만 고칩니다 — 이 좌석은 지금 워커가 아니거나 처음과 다른 역할 가족으로 바뀌었습니다(역할을 바꾼 좌석은 자기가 시작한 수리의 쓰기 권한도 잃습니다 · 되돌리는 길: 이 창을 닫고 새 창에서 워커 역할을 받으세요)";

    /// `cys repair rule-doc-allowed --path <p>` (READ · O-C10-4 · RULINGS CV2: allowed only for a
    /// seat that is a worker now and bound to the worker family): allowed only when the CALLING seat
    /// holds this lane's live lock on a being-worked rule-document repair whose bound, unexpired
    /// token covers that path. Any other case → `allowed: false` (the hook keeps today's behaviour).
    pub(crate) fn rule_doc_allowed(
        ctx: &Ctx<'_>,
        caller: &Caller,
        p: &Value,
    ) -> Result<Value, VerbError> {
        let raw = param_str(p, "path").ok_or_else(|| VerbError::invalid("`path` 가 필요합니다"))?;
        let no = |why: &str| -> Result<Value, VerbError> {
            Ok(json!({"allowed": false, "path": raw, "reason": why}))
        };
        let Some(sid) = caller.seat() else {
            return no("좌석이 아닌 호출자");
        };
        // RULINGS CV2: a permission answer is not a plain read — the calling seat must be a worker
        // NOW and bound to the worker family since this daemon first saw it (AD1 / AK1's line: a
        // seat re-declared to another family keeps no write authorization of the repair it began)
        let family_now = caller.role.as_deref().and_then(super::role_family);
        if family_now != Some(super::RoleFamily::Worker)
            || caller.bound_family != Some(super::RoleFamily::Worker)
            // DG1: a seat that ever left its family (worker → master → worker) stays refused
            || super::left_role_family(&ctx.lane.0, &ctx.incarnation, sid)
        {
            return no(RULE_DOC_WORKER_ONLY_KO);
        }
        let rel = match doc_rel(ctx, raw) {
            Ok(r) => r,
            Err(e) => return no(&e.message),
        };
        let owner = ctx.owner(sid);
        for id in ledger::list_ids_in(&ctx.root) {
            let Ok(Some(rec)) = ledger::read_record_in(&ctx.root, &id) else {
                continue;
            };
            if rec.origin_lane != ctx.lane
                || rec.classification != Classification::RuleDocument
                || rec.state.closes_for_grant()
                || lock::view(rec.lock.as_ref(), &owner, ctx.now, &ctx.lane, &ctx.incarnation)
                    != lock::LockView::Mine
                || !ledger::file_targets(&rec).any(|f| oo::same_rel(&f.rel, &rel))
            {
                continue;
            }
            let Some(sha) = rec.owner_token_sha256.as_deref() else {
                continue;
            };
            let Ok(Some(t)) = oo::read_token_in(&ctx.root, sha) else {
                continue;
            };
            if t.lane_key == ctx.lane
                && t.used_by_repair.as_ref() == Some(&rec.id)
                && ctx.now < t.expires_at
                && oo::token_covers(&t, &rel)
            {
                return Ok(json!({"allowed": true, "path": rel, "repair": rec.id.as_str()}));
            }
        }
        no("이 좌석이 쥔 규칙 문서 수리(주인 지시 표가 이 경로를 덮는 것)가 없습니다")
    }
}

/// T4 (RULINGS B4, CH3) — the daemon side of "주인의 아무 입력이면 즉시 정지": marks an owner input
/// seen by THIS lane's daemon in the grant's pause file (only while the grant is on; nothing is sent
/// to any queue). Called at the two places the daemon learns of owner input (`handlers.rs`): an
/// owner-typed prompt in `hook.machine_origin` (STRICT judgment — CH2) and an operator-typed app send
/// (`human_verified`). App typing arrives in many small sends: RULINGS DK1 / DM1 — every one of them
/// moves this daemon's in-memory latest input first; a write happens only when that input is later
/// than the time the lane's last written mark covers through (the write's time + 5 s), so a burst
/// writes about once per 5 s and its last input is always covered; the pause lasts at least 30
/// minutes after the last input (at most the covering window longer).
pub(crate) fn grant_note_owner_input(daemon: &Daemon) {
    let root = production_root(daemon);
    let lane = LaneKey::of_socket(&daemon.socket_path);
    note_owner_input_throttled(&root, &lane, cys::repair::ledger::now_epoch());
}

/// T4 (RULINGS CH3 (b), CI1, CJ1) — an operator-verified send into `seat` (`human_verified` in
/// `handlers.rs`): the pause, for EVERY such send (the safe direction); and the typing mark of Level
/// 2 only when the operator token did not come from a pane (`verified_from` is none) AND the sent
/// text is typed or pasted text (`owner_origin::arms_typing_mark` — printable characters plus CR, LF,
/// TAB only; a focus / mouse report or a terminal's answer to a query arms nothing).
pub(crate) fn grant_note_app_typing(
    daemon: &Daemon,
    seat: u64,
    verified_from: Option<u64>,
    text: &str,
) {
    grant_note_owner_input(daemon);
    if verified_from.is_none() && cys::repair::owner_origin::arms_typing_mark(text) {
        mark_app_typing(
            &daemon.repair_prompt_origin,
            seat,
            cys::repair::ledger::now_epoch(),
        );
    }
}

/// [`grant_note_owner_input`]'s body (root and clock explicit for the tests) — RULINGS DK1 / DM1:
/// the in-memory latest input moves at once; the write follows when the input is not yet visible
/// (later than the covered-through time this daemon wrote); a failed write is one stderr line scoped
/// to THIS lane's daemon (DK1 (6)) and only spaces the next attempt by a second (DM2). Nothing to
/// any queue.
pub(crate) fn note_owner_input_throttled(root: &Path, lane: &LaneKey, now: f64) {
    if let Err(e) = cys::repair::grant::note_owner_input_in(root, lane, now) {
        eprintln!("{} ({e:?})", cys::repair::grant::PAUSE_WRITE_FAILED_KO);
    }
}

/// RULINGS CH2 (T4) — the origin the per-seat prompt record takes: the STRICT owner judgment
/// `mission_gate::owner_typed` over `hook.machine_origin`'s verdict (its `origin`, `ledger_status`
/// and anomaly codes) and the prompt itself. `human` only when the prompt can be shown to be the
/// owner's own typing; a harness-synthesised notification, an absent / unreadable delivery ledger,
/// an anomaly that hides deliveries, or a machine / unknown verdict is not (`machine` stays
/// `machine`, everything else → `unknown`) — so it can neither become a question's first owner
/// answer nor name a rule document (fail closed).
pub(crate) fn strict_origin(norm: &str, reply: &Value) -> &'static str {
    let lenient = reply
        .get("origin")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let status = reply
        .get("ledger_status")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let codes: Vec<&str> = reply
        .get("anomalies")
        .and_then(|a| a.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.get("code").and_then(|c| c.as_str()))
                .collect()
        })
        .unwrap_or_default();
    if cys::mission_gate::owner_typed(norm, lenient, status, &codes).owner {
        "human"
    } else if lenient == "machine" {
        "machine"
    } else {
        "unknown"
    }
}

/// The pending final-approval requests (C5; memory only — void after a restart). T5b fills it.
#[derive(Default)]
pub(crate) struct ApprovalTable {}

/// The per-seat owner-prompt record (T3 writes, T4 reads). `named_docs` = rule-document base names,
/// lower-cased, found in the seat's latest owner-typed prompt (RULINGS U4(a)); no prompt text kept.
/// `digest` / `origin` / `at` describe the seat's LATEST prompt (any origin). `origin` is LEVEL 1 of
/// the owner judgment (RULINGS CH2 / CI1 — `strict_origin`): `human` = "may be the owner".
/// `last_human` = (digest, at) of the latest Level-1 prompt (CH4). LEVEL 2 (CI1, CJ1, CJ2, CJ4) =
/// Level 1 AND `typing_mark` — the time the daemon last saw the owner's typed or pasted text arrive
/// from the app into this seat (not from a pane), younger than 30 minutes, not cleared by a question
/// registered since — AND every harness marker tag of the prompt inside a closed block. The next
/// Level-1 prompt takes the mark (whether or not it becomes Level 2). `last_typed`, `named_docs`
/// (U4(a), read from the prompt's free text only) and `named_numbers` (CI4) belong to the latest
/// Level-2 prompt (T4).
#[derive(Default, Clone)]
pub(crate) struct PromptOrigin {
    pub digest: String,
    pub origin: String,
    pub at: f64,
    pub question: Option<OpenQuestion>,
    pub named_docs: Vec<String>,
    pub last_human: Option<(String, f64)>,
    pub typing_mark: Option<f64>,
    pub last_typed: Option<(String, f64)>,
    pub named_numbers: Vec<u32>,
}

/// An open question of a master seat (RULINGS T3, Q1 / Q2): `first_human` = (digest, excerpt, at).
/// T4 registers it (`grant ask` / `owner-token ask`); T3 records the answer and `machine_seen`.
#[derive(Default, Clone)]
pub(crate) struct OpenQuestion {
    pub asked_at: f64,
    pub path: Option<String>,
    pub first_human: Option<(String, String, f64)>,
    pub machine_seen: bool,
    /// RULINGS CI4: the daily limit a grant question carries (`grant ask [--daily-cap N]`).
    pub daily_cap: Option<u32>,
    /// RULINGS CV1 (T4b): the decision generation current when the grant question was asked — its
    /// answer turns the grant on only while no decision was published since.
    pub asked_generation: u64,
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

/// Pure part of [`record_prompt_origin`]. RULINGS CI1 (T4): `origin == "human"` is LEVEL 1 ("may be
/// the owner") and feeds `origin`, `last_human` (and, in `handlers.rs`, the pause); LEVEL 2 = Level 1
/// AND this seat's typing mark, fresh (CJ2: armed at most 30 minutes ago, not cleared by a question
/// since), AND no harness marker tag outside a closed block (CJ4). Level 2 feeds what carries
/// authority — a question's `first_human`, `named_docs`, `named_numbers`, `last_typed` — read from the
/// prompt's FREE TEXT only ([`owner_free_text`]). Every Level-1 prompt TAKES the mark, fresh or not,
/// Level 2 or not (fail closed: one typing, one prompt); machine / unknown prompts leave it. A
/// Level-1-but-not-Level-2 prompt counts for an open question as a non-owner prompt
/// (`machine_seen`). The digest stays the whole normalised prompt's.
pub(crate) fn apply_prompt(
    rec: &mut PromptOrigin,
    digest: &str,
    norm: &str,
    origin: &str,
    now: f64,
) {
    let level1 = origin == "human";
    let fresh = level1
        && rec.typing_mark.take().is_some_and(|armed| {
            now >= armed && now - armed <= cys::repair::owner_origin::TYPING_MARK_SECS as f64
        });
    let level2 = fresh && !cys::mission_gate::harness_tag_outside_closed_blocks(norm);
    rec.digest = digest.to_string();
    rec.origin = origin.to_string();
    rec.at = now;
    if level1 {
        rec.last_human = Some((digest.to_string(), now)); // CH4 (T4)
    }
    let free = if level2 { owner_free_text(norm) } else { String::new() };
    if level2 {
        rec.named_docs = named_rule_docs(&free);
        rec.named_numbers = cys::repair::owner_origin::owner_numbers(&free);
        rec.last_typed = Some((digest.to_string(), now));
    }
    if let Some(q) = rec.question.as_mut() {
        if now > q.asked_at && q.first_human.is_none() {
            if level2 {
                let excerpt: String = free.chars().take(ANSWER_EXCERPT_CHARS).collect();
                q.first_human = Some((digest.to_string(), excerpt, now));
            } else {
                q.machine_seen = true;
            }
        }
    }
}

/// RULINGS CI1 (T4) — what Level 2 reads: the free text outside every tag block and code fence
/// (`mission_gate::generic_block_free_text`, the structure axis's residual). A masked code fence
/// stays visible as `▯` (never as letters), so "예 ```x```" is not the answer "예".
pub(crate) fn owner_free_text(norm: &str) -> String {
    cys::mission_gate::generic_block_free_text(norm)
        .0
        .replace('\0', "▯")
}

/// RULINGS CI1 / CJ1 / CJ2 (T4) — the typing mark: the owner's typed or pasted text arrived from the
/// app into `seat` at `at` (the target surface of a `human_verified` send whose operator token did
/// not come from a pane and whose text `arms_typing_mark`). A later arming moves the time on. Daemon
/// memory only (a restart loses it — fail closed); good for 30 minutes; cleared when a question is
/// registered for the seat; taken by the seat's next Level-1 prompt.
pub(crate) fn mark_app_typing(prompts: &Mutex<HashMap<u64, PromptOrigin>>, seat: u64, at: f64) {
    prompts
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(seat)
        .or_default()
        .typing_mark = Some(at);
}

/// RULINGS DA3: how many intake locks exist — a FIXED number, whatever the callers report.
pub(crate) const INTAKE_LOCK_SHARDS: usize = 16;

/// RULINGS DA3 — the bounded intake lock: one of [`INTAKE_LOCK_SHARDS`] process-wide mutexes,
/// chosen by the signature (a refused or unique report adds no entry to any map). Every
/// read-modify-write of an intake row runs under the lock of its signature; the hour's quota is
/// reserved separately and atomically (DA2). The ONE entry point for every intake writer: the
/// `report` verb and the repeated-failure detector (`alerts.rs`, unit T3e). Poison is tolerated.
pub(crate) fn intake_lock_for(signature: &str) -> std::sync::MutexGuard<'static, ()> {
    INTAKE_SHARDS[intake_shard_of(signature)]
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// The [`INTAKE_LOCK_SHARDS`] intake mutexes (RULINGS DA3; at module scope since RULINGS FF so that the test probe
/// [`intake_lock_is_held`] reaches the same mutexes).
static INTAKE_SHARDS: [Mutex<()>; INTAKE_LOCK_SHARDS] = [const { Mutex::new(()) }; INTAKE_LOCK_SHARDS];

/// Which of the [`INTAKE_LOCK_SHARDS`] intake locks a signature takes (stable within a process).
pub(crate) fn intake_shard_of(signature: &str) -> usize {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    signature.hash(&mut h);
    (h.finish() % INTAKE_LOCK_SHARDS as u64) as usize
}

/// RULINGS FF — tests only: is the intake mutex of `signature` held right now (by anyone, this thread included)?
/// `try_lock` gives `WouldBlock`; a poisoned mutex that is free is not held.
#[cfg(test)]
pub(crate) fn intake_lock_is_held(signature: &str) -> bool {
    matches!(INTAKE_SHARDS[intake_shard_of(signature)].try_lock(), Err(std::sync::TryLockError::WouldBlock))
}

/// RULINGS DA1 — what decides a target's class is the FILE, not its spelling. For a target outside
/// the rule-document route (its name is no rule document — those go through the owner's token,
/// T4), `Some(reason)` when its live file is: (c) a leaf link whose target lies in a pack or is a
/// folder (a link is another name of its target — the AW3 (c) rule, applied to the live leaf); (a)
/// the same file (volume + file id) as one of the pack's protected names it can list — the
/// `SYSTEM_LOCKED` files and the rule documents of the pack root and `directives/`; (b) a file with
/// more than one name (hard links): rule documents named `*_DIRECTIVE.md` may lie anywhere in the
/// pack and cannot all be listed, so a second name is refused conservatively. An absent target
/// has no identity to compare (`None`). `boot_critical::refuse_identity_alias` (T5a) covers the
/// two compiled lists in the classification.
pub(crate) fn identity_refusal(pack: &Path, rel: &str) -> Option<String> {
    use cys::repair::boot_critical::file_identity;
    use cys::repair::types::{fold_name, link_count, link_target_refusal};
    let live = pack.join(rel);
    let meta = std::fs::symlink_metadata(&live).ok()?;
    if meta.file_type().is_symlink() {
        if let Ok(t) = std::fs::read_link(&live) {
            if let Some(why) = link_target_refusal(pack, rel, &t.to_string_lossy()) {
                return Some(why);
            }
        }
    }
    let id = file_identity(&live)?;
    let key = fold_name(&rel.replace('\\', "/"));
    for name in protected_names(pack) {
        if fold_name(&name) != key && file_identity(&pack.join(&name)) == Some(id) {
            // DJ3: the way that exists for the protected file — the key file has none but pack-heal
            if let Some(locked) = cys::repair::types::system_locked_entry(&name) {
                return Some(format!(
                    "{rel}: 서명 키 파일 `{locked}` 과 같은 파일입니다(하드 링크 등 같은 파일의 다른 이름) — 서명 키 파일은 어떤 이름으로도 수리 대상이 아닙니다. 사본이 손상됐다면 `cys pack-heal {locked}` 로 내장본을 되살립니다(RULINGS AU4 · DA1)"
                ));
            }
            return Some(format!(
                "{rel}: 규칙 문서 `{name}` 과 같은 파일입니다(하드 링크 등 같은 파일의 다른 이름) — 규칙 문서는 그 이름 `{name}` 으로, 주인 지시 표가 있을 때만 고칩니다(RULINGS DA1)"
            ));
        }
    }
    if std::fs::metadata(&live).is_ok_and(|m| m.is_file()) {
        if let Some(n) = link_count(&live).filter(|n| *n > 1) {
            return Some(format!(
                "{rel}: 이름이 {n}개인 파일입니다(하드 링크) — 규칙 문서의 다른 이름일 수 있어 수리 대상으로 받지 않습니다(RULINGS DA1)"
            ));
        }
    }
    None
}

/// RULINGS DA1: the protected names DA1 can list in a pack — its `SYSTEM_LOCKED` files and the
/// rule documents (by [`rule_document_name`]) of the pack root and of `directives/`.
fn protected_names(pack: &Path) -> Vec<String> {
    let mut out: Vec<String> = cys::repair::types::system_locked_names()
        .iter()
        .map(|n| n.to_string())
        .collect();
    for dir in ["", "directives"] {
        let Ok(rd) = std::fs::read_dir(pack.join(dir)) else {
            continue;
        };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if rule_document_name(&name).is_some() {
                out.push(if dir.is_empty() {
                    name
                } else {
                    format!("{dir}/{name}")
                });
            }
        }
    }
    out
}

/// How two targets are one file (RULINGS DA4 / ED1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Same {
    /// both live files exist and have one identity (volume + file id)
    Identity,
    /// the same spelling
    Spelling,
    /// not-yet-existing tails under one existing folder, equal after ASCII case folding
    CaseFold,
    /// not-yet-existing tails under one existing folder, one of them holding a non-ASCII character
    NonAscii,
}

/// RULINGS EG1 — the answer of comparing two targets: SAME (how), DIFFERENT (only on positive
/// evidence), or UNKNOWN (a lookup failed or could not be made) — and UNKNOWN refuses like SAME.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Verdict {
    Same(Same),
    Different,
    Unknown,
}

/// RULINGS EG1 — one file-system lookup with three answers: the path EXISTS (its identity:
/// volume + file id, links followed), is ABSENT (the system said it is not there), or UNKNOWN
/// (any other failure — never evidence of anything).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Lookup {
    Exists((u64, u64)),
    Absent,
    Unknown,
}

/// RULINGS EG1 — the identity of `path` now ([`Lookup`]); directories included.
pub(crate) fn identity_now(path: &Path) -> Lookup {
    #[cfg(test)]
    if lookup_fails(path) {
        return Lookup::Unknown;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        match std::fs::metadata(path) {
            Ok(m) => Lookup::Exists((m.dev(), m.ino())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Lookup::Absent,
            Err(_) => Lookup::Unknown,
        }
    }
    #[cfg(windows)]
    {
        // RULINGS EG2: an open that permits DIRECTORIES (FILE_FLAG_BACKUP_SEMANTICS, no data
        // access), then the same by-handle facts as `boot_critical::file_identity` (volume serial,
        // 64-bit file index) — so a folder's identity is read and the usual case is decided.
        use std::os::windows::fs::OpenOptionsExt;
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, FILE_FLAG_BACKUP_SEMANTICS,
            FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
        };
        let f = match std::fs::OpenOptions::new()
            .access_mode(0)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)
        {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Lookup::Absent,
            Err(_) => return Lookup::Unknown,
        };
        // SAFETY: a zeroed BY_HANDLE_FILE_INFORMATION is a valid out-parameter; the handle is
        // owned by `f`, which outlives the call.
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        if unsafe { GetFileInformationByHandle(f.as_raw_handle() as _, &mut info) } == 0 {
            return Lookup::Unknown;
        }
        Lookup::Exists((
            u64::from(info.dwVolumeSerialNumber),
            (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
        ))
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        Lookup::Unknown
    }
}

/// Test builds only: paths whose lookup a test makes FAIL (an injected failure — no platform
/// trick), exact paths as the daemon builds them.
#[cfg(test)]
static LOOKUP_FAILS: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

#[cfg(test)]
pub(crate) fn lookup_fail_install(path: &Path) {
    LOOKUP_FAILS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(path.to_path_buf());
}

#[cfg(test)]
pub(crate) fn lookup_fail_clear(path: &Path) {
    LOOKUP_FAILS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .retain(|p| p != path);
}

#[cfg(test)]
fn lookup_fails(path: &Path) -> bool {
    LOOKUP_FAILS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .any(|p| p == path)
}

/// RULINGS ED1 / EG1: a target resolved against the file system NOW — `Some((identity of its
/// deepest EXISTING ancestor — the target itself when it exists, not-yet-existing tail))`; walking
/// up passes only paths the system says are ABSENT. `None` = UNKNOWN: a lookup failed (or not
/// even the pack root exists) — never "absent".
fn resolve_now(pack: &Path, rel: &str) -> Option<((u64, u64), Vec<String>)> {
    let parts: Vec<&str> = rel
        .split(['/', '\\'])
        .filter(|p| !p.is_empty() && *p != ".")
        .collect();
    for k in (0..=parts.len()).rev() {
        let path = parts[..k]
            .iter()
            .fold(pack.to_path_buf(), |acc, c| acc.join(c));
        match identity_now(&path) {
            Lookup::Exists(id) => {
                return Some((id, parts[k..].iter().map(|s| s.to_string()).collect()))
            }
            Lookup::Absent => continue,
            Lookup::Unknown => return None,
        }
    }
    None
}

/// RULINGS DA4 / ED1 / EG1 — ONE comparison, made NOW, for both targets. SAME when (i) both exist
/// and have one identity, or (ii) they share the deepest existing ancestor and their
/// not-yet-existing tails are equal after ASCII case folding — a tail holding ANY non-ASCII
/// character equals every other tail under that ancestor (no normalisation library — RULINGS
/// CD1). DIFFERENT only on positive evidence: two identities read and unequal; one target exists
/// and the other does not; two different deepest ancestors read; one ancestor and unequal ASCII
/// tails. Every failed lookup → UNKNOWN. Hook lines (`settings:…`) compare only with hook lines.
pub(crate) fn same_now(pack: &Path, a: &str, b: &str) -> Verdict {
    let hook = |s: &str| s.starts_with("settings:");
    if hook(a) || hook(b) {
        return if !(hook(a) && hook(b) && a.eq_ignore_ascii_case(b)) {
            Verdict::Different
        } else if a == b {
            Verdict::Same(Same::Spelling)
        } else {
            Verdict::Same(Same::CaseFold)
        };
    }
    if a == b {
        return Verdict::Same(Same::Spelling);
    }
    let (Some(ra), Some(rb)) = (resolve_now(pack, a), resolve_now(pack, b)) else {
        return Verdict::Unknown;
    };
    match (ra.1.is_empty(), rb.1.is_empty()) {
        (true, true) if ra.0 == rb.0 => Verdict::Same(Same::Identity),
        (true, true) => Verdict::Different,
        (true, false) | (false, true) => Verdict::Different,
        (false, false) if ra.0 != rb.0 => Verdict::Different,
        (false, false) => {
            if ra.1.iter().chain(rb.1.iter()).any(|c| !c.is_ascii()) {
                Verdict::Same(Same::NonAscii)
            } else if ra.1.len() == rb.1.len()
                && ra
                    .1
                    .iter()
                    .zip(&rb.1)
                    .all(|(x, y)| x.eq_ignore_ascii_case(y))
            {
                Verdict::Same(Same::CaseFold)
            } else {
                Verdict::Different
            }
        }
    }
}

/// [`same_now`] as a yes / no (the tests' shorthand; the verbs use `same_now` for its sentence):
/// `true` for SAME and for UNKNOWN (both refuse).
#[cfg(test)]
pub(crate) fn same_target(pack: &Path, a: &str, b: &str) -> bool {
    same_now(pack, a, b) != Verdict::Different
}

/// RULINGS DA4 / ED1 / EG1 — the refusal sentence when the new target `new` may be, NOW, the
/// same file as a target of the being-worked repair `rec` of this lane ([`same_now`]), or
/// `None` (DIFFERENT for every target). The sentence says what is known: one identity / one
/// spelling → BJ5's sentence; the conservative rule → a conditional sentence; UNKNOWN → it
/// could not be determined.
pub(crate) fn open_overlap(
    pack: &Path,
    new: &str,
    rec: &cys::repair::ledger::RepairRecord,
) -> Option<String> {
    let id = rec.id.as_str();
    let theirs: Vec<&str> = rec.files.iter().map(|f| f.rel.as_str()).collect();
    for t in &theirs {
        match same_now(pack, new, t) {
            Verdict::Different => continue,
            Verdict::Same(Same::Identity | Same::Spelling) => {
                return Some(format!(
                    "{new}: 이 파일을 고치는 수리 {id} 가 이미 열려 있습니다(대상 파일 묶음 또는 --resolves 가 다릅니다) — `--joins {id}` 로 합류하거나, 같은 파일들({})로 `cys repair begin` 해서 그 수리를 이어받으세요. 한 파일에 열린 수리는 하나뿐입니다(RULINGS BJ5)",
                    theirs.join(", ")
                ))
            }
            Verdict::Same(Same::CaseFold) => {
                return Some(format!(
                    "{new}: 수리 {id} 가 연 `{t}` 와 대소문자만 다른 이름입니다 — 대소문자만 다른 이름은 볼륨에 따라 같은 파일이라 동시에 열지 않습니다. 먼저 열린 수리 {id} 가 끝난 뒤 다시 하세요(RULINGS DJ1 · ED1)"
                ))
            }
            Verdict::Same(Same::NonAscii) => {
                return Some(format!(
                    "{new}: 아직 없는 이름이 ASCII 가 아닌 글자를 담고 있어, 같은 폴더 아래의 수리 {id} 대상 `{t}` 와 볼륨에 따라 같은 파일일 수 있습니다(정규화 형식만 다른 이름을 가릴 수 없음) — {id} 가 끝난 뒤 다시 하세요(RULINGS ED1)"
                ))
            }
            Verdict::Unknown => {
                return Some(format!(
                    "{new}: 수리 {id} 의 대상 `{t}` 와 같은 파일인지 확인할 수 없습니다(파일 시스템 조회 실패) — 같은 파일일 수 있어 {id} 가 끝난 뒤 다시 하세요(RULINGS EG1)"
                ))
            }
        }
    }
    None
}

/// C10's rule-document base names (as `guard.sh:PROTECTED_NAMES`), lower-cased.
const RULE_DOC_NAMES: [&str; 5] = [
    "soul.md",
    "claude.md",
    "vibecoding_constitution.md",
    "vibecoding_enforcement.md",
    "route_contract.md",
];

/// RULINGS AD6 / BJ3: the rule-document name a target's base name folds to (full case folding —
/// `cys::repair::types::fold_name`, the same helper as the key-file rule), or `None`.
pub(crate) fn rule_document_name(rel: &str) -> Option<String> {
    let base = rel.rsplit(['/', '\\']).next().unwrap_or(rel);
    let folded = cys::repair::types::fold_name(base);
    let is_doc = RULE_DOC_NAMES.contains(&folded.as_str())
        || (folded.ends_with("_directive.md") && folded.len() > "_directive.md".len());
    is_doc.then_some(folded)
}

/// RULINGS BJ3: the on-disk spelling of the file `rel` names in `pack` — the entry of its folder
/// whose name folds to the same name as `rel`'s base name — or `None` when there is none.
pub(crate) fn real_name_in(pack: &Path, rel: &str) -> Option<String> {
    let base = rel.rsplit(['/', '\\']).next().unwrap_or(rel);
    let folded = cys::repair::types::fold_name(base);
    let dir = pack.join(rel);
    let dir = dir.parent()?;
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .find(|n| cys::repair::types::fold_name(n) == folded)
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
    let classify = |rel: &str| production_classify(rel, &cys::pack::pack_dir());
    let check = production_check;
    let product_pause = || product_pause_in_force(daemon, &cys::pack::pack_dir());
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
        claim: &cys::repair::grant::try_claim_autonomous_start,
        prompts: &daemon.repair_prompt_origin,
        product_pause: &product_pause,
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

    /// (T3 rewrite of T0's pin — T3 filled 9 verbs; T4 rewrite — T4 filled 5, and a 6th, the READ
    /// `grant_status` of RULINGS CI6 (b): 23 → 24 sub-verbs.) Every sub-verb answers
    /// ONE structured reply and never panics, whatever the params, caller and grade; the 9 verbs whose
    /// bodies are other tracks' still answer `repair_not_implemented` to a caller the role table admits
    /// (the app for OWNER-APPROVAL, any caller for READ/TRIGGER) and `acl_denied` otherwise; unknown →
    /// `method_not_found`.
    #[test]
    fn every_subverb_is_not_implemented_and_never_panics() {
        assert_eq!(REPAIR_SUBVERBS.len(), 24);
        let mut uniq = REPAIR_SUBVERBS.to_vec();
        uniq.sort_unstable();
        uniq.dedup();
        assert_eq!(uniq.len(), 24, "REPAIR_SUBVERBS has duplicates");
        assert!(T3_VERBS.iter().all(|v| REPAIR_SUBVERBS.contains(v)));
        assert_eq!(T3_VERBS.len(), 9);
        assert!(T4_VERBS.iter().all(|v| REPAIR_SUBVERBS.contains(v)));
        assert!(T4_VERBS.iter().all(|v| !T3_VERBS.contains(v)));
        assert_eq!(T4_VERBS.len(), 6);
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
                    // T6 (rewrite: `revert` / `reapply` / `tick` / `sweep` left the stub set — the old
                    // expectation for them was `repair_not_implemented` after the role table)
                    if T3_VERBS.contains(sub) || T4_VERBS.contains(sub) || T6_VERBS.contains(sub) {
                        // T3's / T4's / T6's bodies answer a success or a structured error — never the stub
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

    /// J1 census: every daemon sub-verb has exactly one class; the table is exactly the 24 daemon
    /// verbs + the 5 local verbs; the WORK / OWNER-DECISION / OWNER-APPROVAL sets as C3 (READ gains
    /// `grant_status`, RULINGS CI6 (b)).
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
            "VERB_CLASSES must be exactly the 24 daemon verbs + the 5 local verbs"
        );
        assert_eq!(VERB_CLASSES.len(), 29);
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
                "grant_status",
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
