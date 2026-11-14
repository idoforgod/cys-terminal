//! RULINGS EJ4 / EN5 — ONE ownership per repair in this daemon. Everything the daemon does to a
//! repair — commit (its hook phase from before the dry run included), apply, revert, re-check,
//! put-back, recovery — holds that repair's guard for its whole duration; the guard is only ever
//! TRIED (`try_take`), never waited for: a request path that finds it held answers [`BUSY`] and
//! returns, the job's recovery leaves that repair alone this tick. Recovery reads the durable
//! records only AFTER it holds the guard.
//!
//! The daemon's short per-repair mutex (one read-modify-write of a record / row file, held for
//! milliseconds, never across the registrar — AK5) stays a separate thing; the order is pinned:
//! pack-apply lock (tried) → guard (tried) → mutex (short) — the guard is never requested while
//! the mutex is held (a debug assertion; [`MutexScope`] marks the mutex's extent).

use super::types::RepairId;
use std::path::{Path, PathBuf};

/// What a request path answers when the repair is owned by another operation (EN5).
pub const BUSY: &str = "이 수리는 지금 다른 작업 중입니다 — 기다리지 않고 돌아갑니다";

fn held() -> &'static std::sync::Mutex<std::collections::HashSet<(PathBuf, String)>> {
    static HELD: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<(PathBuf, String)>>> = std::sync::OnceLock::new();
    HELD.get_or_init(|| std::sync::Mutex::new(std::collections::HashSet::new()))
}

thread_local! {
    static MUTEX_DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// The ownership of one repair (keyed by the repairs root and the repair id — the M10 sub-ledger
/// has its own root), released when dropped.
#[derive(Debug)]
pub struct RepairGuard {
    key: (PathBuf, String),
}

impl RepairGuard {
    /// Takes the guard of `id` under `root` if no operation of this daemon holds it.
    pub fn try_take(root: &Path, id: &RepairId) -> Option<RepairGuard> {
        debug_assert!(
            MUTEX_DEPTH.with(|d| d.get()) == 0,
            "EN5: the repair guard is never requested while the per-repair mutex is held"
        );
        let key = (root.to_path_buf(), id.as_str().to_string());
        let mut set = held().lock().unwrap_or_else(|e| e.into_inner());
        if !set.insert(key.clone()) {
            return None;
        }
        Some(RepairGuard { key })
    }
}

impl Drop for RepairGuard {
    fn drop(&mut self) {
        held().lock().unwrap_or_else(|e| e.into_inner()).remove(&self.key);
    }
}

/// Whether an operation of this daemon holds the guard of `id` under `root`.
pub fn is_held(root: &Path, id: &RepairId) -> bool {
    held()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains(&(root.to_path_buf(), id.as_str().to_string()))
}

/// Marks the extent of the per-repair mutex on this thread (EN5's order assertion).
pub struct MutexScope;

impl MutexScope {
    pub fn enter() -> MutexScope {
        MUTEX_DEPTH.with(|d| d.set(d.get() + 1));
        MutexScope
    }
}

impl Drop for MutexScope {
    fn drop(&mut self) {
        MUTEX_DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_owner_at_a_time_released_by_scope() {
        let root = std::env::temp_dir().join(format!("cys-guard-{}", std::process::id()));
        let id = RepairId::parse("R-9").unwrap();
        let g = RepairGuard::try_take(&root, &id).expect("free");
        assert!(RepairGuard::try_take(&root, &id).is_none(), "held → refused, never waited for");
        assert!(is_held(&root, &id));
        drop(g);
        assert!(RepairGuard::try_take(&root, &id).is_some(), "released by scope");
    }

    /// EN5: the guard is never requested while the per-repair mutex is held.
    #[test]
    #[should_panic(expected = "EN5")]
    fn the_guard_is_never_requested_under_the_mutex() {
        let root = std::env::temp_dir().join(format!("cys-guard-en5-{}", std::process::id()));
        let _m = MutexScope::enter();
        let _ = RepairGuard::try_take(&root, &RepairId::parse("R-10").unwrap());
    }
}
