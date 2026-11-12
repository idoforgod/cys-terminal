//! Durable writes (O-C1-4 · RULINGS E1: file flush, rename, directory flush; on macOS every flush
//! is a full flush). T0: stubs (body: T6).

use super::types::RepairError;
use std::path::Path;

/// Stub in T0 (body: T6).
pub fn write_durable(path: &Path, bytes: &[u8]) -> Result<(), RepairError> {
    let _ = (path, bytes);
    Err(RepairError::NotImplemented("durable::write_durable"))
}

/// Stub in T0 (body: T6).
pub fn full_flush_dir(dir: &Path) -> Result<(), RepairError> {
    let _ = dir;
    Err(RepairError::NotImplemented("durable::full_flush_dir"))
}
