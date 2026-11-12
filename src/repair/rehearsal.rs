//! The rehearsal runner (M8 · RULINGS F4, F8, H2, K4/O3). T0: types, a constant and stubs — T5b
//! implements them; T6 calls `run_check` for the recheck's pre-image run.

use super::boot_critical::CheckKind;
use super::types::RepairError;
use std::path::PathBuf;

/// One check of one boot-critical entry against a scratch copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckRequest {
    pub entry_rel: String,
    pub scratch_root: PathBuf,
    pub recorded_input: Option<PathBuf>,
    pub deadline: std::time::Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckResult {
    Pass,
    Fail(String),
    Timeout,
}

/// Runs one check child (killed and reaped at the deadline). Stub in T0 (body: T5b).
pub fn run_check(req: &CheckRequest) -> Result<CheckResult, RepairError> {
    let _ = req;
    Err(RepairError::NotImplemented("rehearsal::run_check"))
}

/// The built-in known-good fixture of a check kind (RULINGS H2). Stub in T0 (body: T5b).
pub fn reference_fixture(kind: CheckKind) -> Result<&'static [u8], RepairError> {
    let _ = kind;
    Err(RepairError::NotImplemented("rehearsal::reference_fixture"))
}

/// Runs the reference check of a kind on its fixture (RULINGS H2). Stub in T0 (body: T5b).
pub fn run_reference_check(
    kind: CheckKind,
    deadline: std::time::Duration,
) -> Result<CheckResult, RepairError> {
    let _ = (kind, deadline);
    Err(RepairError::NotImplemented(
        "rehearsal::run_reference_check",
    ))
}

/// RULINGS K4 / O3: a created file is blamed for a timeout only when its commit-time duration ×
/// (reference now ÷ reference at commit) is under the deadline ÷ 1.5, after a control pass and a
/// second timeout.
pub const REFERENCE_SLOWDOWN_PERMILLE: u32 = 1_500;
