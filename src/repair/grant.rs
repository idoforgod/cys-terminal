//! The autonomous-repair grant (C9 · M5 · RULINGS B1, B4, F1, G1–G3, G6, G7, K3, K5, K8, L2, M3).
//! T0: records, constants and stubs (body: T4).

use super::types::{LaneKey, LockOwner, RepairError, RepairId};
use serde::{Deserialize, Serialize};

/// `repairs/grant.json` — the owner's decision and its provenance only (B1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GrantDecision {
    pub schema: u32,
    pub state: GrantState,
    pub daily_cap: u32,
    pub decided_at: f64,
    pub decided_lane: LaneKey,
    pub decided_seat: u64,
    pub prompt_digest: String,
    pub prompt_excerpt: String,
    pub paused_at: Option<f64>,
    pub resumed_at: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrantState {
    On,
    Off,
}

/// Default daily cap of autonomous starts (C9).
pub const DEFAULT_DAILY_CAP: u32 = 5;
/// The pause ends by itself after 30 minutes without owner input (B4).
pub const PAUSE_QUIET_SECS: u64 = 1_800;
/// RULINGS G7(a).
pub const EXPIRY_GRACE_SECS: u64 = 60;
/// RULINGS K8: an unreadable claim counts toward today's cap only while younger than this. K5's
/// "assume held" bound is `LEASE_SECS` + `EXPIRY_GRACE_SECS` (no own constant).
pub const UNREADABLE_CLAIM_CAP_SECS: u64 = 86_400;

/// `repairs/grant-seq/<n>.json` — one claim of the global sequence (F1 / G1). `day` = the local
/// date at placement (M3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GrantClaim {
    pub schema: u32,
    pub repair: RepairId,
    pub lane_key: LaneKey,
    pub day: String,
    pub claimed_at: f64,
}

/// Outcome of an autonomous start attempt (G1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartVerdict {
    Allowed { claim: u32 },
    Paused,
    Busy,
    CapReached,
    LostRace,
}

/// RULINGS K8 / G1; a further round under a live, unclosed claim never calls it (K3). Stub in T0.
pub fn try_claim_autonomous_start(
    owner: &LockOwner,
    id: &RepairId,
) -> Result<StartVerdict, RepairError> {
    let _ = (owner, id);
    Err(RepairError::NotImplemented(
        "grant::try_claim_autonomous_start",
    ))
}

/// Step 2 of G1: writes `<n>.closed` for every claim ≤ `high_water` that G2 makes closable. Stub.
pub fn observe_and_close_claims(high_water: u32) -> Result<u32, RepairError> {
    let _ = high_water;
    Err(RepairError::NotImplemented(
        "grant::observe_and_close_claims",
    ))
}

/// Stub in T0 (body: T4).
pub fn may_open_new_round(id: &RepairId) -> Result<bool, RepairError> {
    let _ = id;
    Err(RepairError::NotImplemented("grant::may_open_new_round"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// O-C1-5, G7, K8, B4, B1, K4.
    #[test]
    fn timing_constants() {
        assert_eq!(crate::repair::types::LEASE_SECS, 7_200);
        assert_eq!(EXPIRY_GRACE_SECS, 60);
        assert_eq!(UNREADABLE_CLAIM_CAP_SECS, 86_400);
        assert_eq!(PAUSE_QUIET_SECS, 1_800);
        assert_eq!(DEFAULT_DAILY_CAP, 5);
        assert_eq!(crate::repair::rehearsal::REFERENCE_SLOWDOWN_PERMILLE, 1_500);
    }
}
