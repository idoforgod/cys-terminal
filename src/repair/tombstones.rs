//! Tombstones — recorded file deletions and non-lifeline hook unregistrations that survive updates
//! (C1 1.5, RULINGS B1: one file per lane, `repairs/tombstones/<lane_key>.json`, written only by
//! that lane's daemon). T0: types and stubs; callers treat `Err` as the empty set (today's
//! behaviour: deleted files come back).

use super::types::{LaneKey, RepairError, RepairId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TombstoneKind {
    File,
    Hook { event: String, script: String },
}

/// One tombstone. A hook tombstone's `rel` is `settings:<event>:<script>` (RULINGS W3(b)).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tombstone {
    pub rel: String,
    pub kind: TombstoneKind,
    pub repair: RepairId,
}

/// The lane's tombstones. Stub in T0 (body: T3).
pub fn load(lane: &LaneKey) -> Result<Vec<Tombstone>, RepairError> {
    let _ = lane;
    Err(RepairError::NotImplemented("tombstones::load"))
}

/// Records one tombstone in the lane's file. Stub in T0 (body: T3).
pub fn record(lane: &LaneKey, t: &Tombstone) -> Result<(), RepairError> {
    let _ = (lane, t);
    Err(RepairError::NotImplemented("tombstones::record"))
}

/// Removes every tombstone of one repair from the lane's file. Stub in T0 (body: T3).
pub fn remove_for_repair(lane: &LaneKey, id: &RepairId) -> Result<(), RepairError> {
    let _ = (lane, id);
    Err(RepairError::NotImplemented("tombstones::remove_for_repair"))
}
