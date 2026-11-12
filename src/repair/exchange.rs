//! Rule-one filesystem primitives (RULINGS E5–E7, E9, F3). T0: stubs only — this file references
//! nothing outside the repair module. T6 implements `exchange` by calling
//! `crate::app_bundle::swap_paths` (made `pub(crate)` by T6's one visibility-only change, F3); any
//! exchange error declines and changes nothing (E9).

use super::types::{Identity, RepairError};
use std::path::Path;

/// Atomically swaps the live path with a vault slot entry. Stub in T0 (body: T6).
pub fn exchange(live: &Path, slot: &Path) -> Result<(), RepairError> {
    let _ = (live, slot);
    Err(RepairError::NotImplemented("exchange::exchange"))
}

/// No-overwrite placement (RULINGS E7: lstat first incl. case variants, then `RENAME_EXCL`). Stub.
pub fn place_no_replace(src: &Path, dst: &Path) -> Result<(), RepairError> {
    let _ = (src, dst);
    Err(RepairError::NotImplemented("exchange::place_no_replace"))
}

/// Real-device comparison of two PARENT directories (RULINGS E5; Windows O-C2-5). Stub.
pub fn same_filesystem(a_parent: &Path, b_parent: &Path) -> Result<bool, RepairError> {
    let _ = (a_parent, b_parent);
    Err(RepairError::NotImplemented("exchange::same_filesystem"))
}

/// Volume UUID + inode + content hash (RULINGS E6). Stub.
pub fn identity_of(p: &Path) -> Result<Identity, RepairError> {
    let _ = p;
    Err(RepairError::NotImplemented("exchange::identity_of"))
}
