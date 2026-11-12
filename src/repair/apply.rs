//! Apply / revert / propagation entry points (C2, design §4-9, 11.2, TESTPLAN OQ-D7/OQ-D8, RULINGS
//! H3, F9, R2, R5/P6, X1, Y1, Y2). T0: signatures only — every function is a stub (bodies: T6,
//! T7b), so a caller in T0 changes nothing.

use super::ledger::ApprovalResult;
use super::types::{LaneKey, LaneState, RepairError, RepairId};
use std::path::PathBuf;

/// One staged file of an approved candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateFile {
    pub rel: String,
    pub staged: PathBuf,
    pub mode: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyOutcome {
    Applied,
    Superseded(String),
    Declined(String),
}

/// Who asks for a revert (C3: human, a worker on a master's ticket, the daemon in-process — never
/// the master).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevertCaller {
    Human,
    TicketedWorker { seat: u64 },
    DaemonAuto,
}

/// Stub in T0 (body: T6).
pub fn apply_candidate(
    id: &RepairId,
    lane: &LaneKey,
    files: &[CandidateFile],
) -> Result<ApplyOutcome, RepairError> {
    let _ = (id, lane, files);
    Err(RepairError::NotImplemented("apply::apply_candidate"))
}

/// Stub in T0 (body: T6).
pub fn revert_repair(
    id: &RepairId,
    lane: &LaneKey,
    by: RevertCaller,
) -> Result<ApplyOutcome, RepairError> {
    let _ = (id, lane, by);
    Err(RepairError::NotImplemented("apply::revert_repair"))
}

/// Stub in T0 (body: T7b).
pub fn propagate(id: &RepairId, lane: &LaneKey) -> Result<LaneState, RepairError> {
    let _ = (id, lane);
    Err(RepairError::NotImplemented("apply::propagate"))
}

/// Finishes or abandons interrupted operations of a lane; returns how many. Stub in T0 (body: T6).
pub fn resume_interrupted(lane: &LaneKey) -> Result<u32, RepairError> {
    let _ = lane;
    Err(RepairError::NotImplemented("apply::resume_interrupted"))
}

/// A lane's catch-up status (RULINGS H3 / H5 required set).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CatchupStatus {
    pub complete: bool,
    pub pending: u32,
    pub conflict: u32,
}

/// Stub in T0 (body: T7b).
pub fn catchup_status(lane: &LaneKey) -> Result<CatchupStatus, RepairError> {
    let _ = lane;
    Err(RepairError::NotImplemented("apply::catchup_status"))
}

/// The `recheck` verb's lane re-evaluation (RULINGS H5). Stub in T0 (body: T7b).
pub fn recheck_lane(id: &RepairId, lane: &LaneKey) -> Result<LaneState, RepairError> {
    let _ = (id, lane);
    Err(RepairError::NotImplemented("apply::recheck_lane"))
}

/// F9's triple check (boot-critical OR protection-related — RULINGS U1); M10's. Stub in T0 (body: T6).
pub fn approved_copy_of_current_generation(lane: &LaneKey, rel: &str) -> Result<bool, RepairError> {
    let _ = (lane, rel);
    Err(RepairError::NotImplemented(
        "apply::approved_copy_of_current_generation",
    ))
}

/// RULINGS R2. Stub in T0 (body: T6): returns `false` = applies nothing.
pub fn origin_approval_triple(
    id: &RepairId,
    approval: &ApprovalResult,
    origin_lane: &LaneKey,
) -> bool {
    let _ = (id, approval, origin_lane);
    false
}

/// Vault slots an in-force repair still depends on (RULINGS R5 / P6). Stub in T0 (body: T6);
/// T3's `cleanup` refuses every slot on `Err`.
pub fn slots_needed_in_force(id: &RepairId) -> Result<Vec<PathBuf>, RepairError> {
    let _ = id;
    Err(RepairError::NotImplemented("apply::slots_needed_in_force"))
}

/// The hook step of RULINGS Y1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookStep {
    Unregister,
    Reregister,
}

/// The registrar mode's report (RULINGS Y2(a)).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconcileReport {
    pub hooks: Vec<ReconcileHook>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconcileHook {
    pub event: String,
    pub script: String,
    pub lifeline: bool,
    /// `registered` | `unregistered` | `unknown`
    pub desired: String,
    /// (settings path, `registered` | `unregistered` | `refused:<reason>`)
    pub targets: Vec<(PathBuf, String)>,
}

/// RULINGS X1 / Y2(b): runs `javis_preflight.py --reconcile-hooks --json` for the lane and judges
/// only the named hooks. Stub in T0 (body: T6).
pub fn reconcile_hook_registrations(
    lane: &LaneKey,
    hooks: &[(String, String)],
) -> Result<ReconcileReport, RepairError> {
    let _ = (lane, hooks);
    Err(RepairError::NotImplemented(
        "apply::reconcile_hook_registrations",
    ))
}

/// RULINGS Y1: the ONE hook step (operation record first, tombstone, reconcile, record finished).
/// Stub in T0 (body: T6) — until then a commit with a hook line is refused with this error.
pub fn hook_lines_step(
    id: &RepairId,
    lane: &LaneKey,
    step: HookStep,
) -> Result<ReconcileReport, RepairError> {
    let _ = (id, lane, step);
    Err(RepairError::NotImplemented("apply::hook_lines_step"))
}
