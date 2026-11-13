//! Repair locks (C1 1.12 · O-C1-5 · RULINGS G6, G7, O5, P4) — track T3.
//!
//! A lock is a [`Lease`] inside the repair's own record (one writer per file, B1): owner = lane (the
//! daemon's socket) + seat + daemon start incarnation — never a process id (the PID-lock trap of
//! `cysjavis-pack/bin/javis_task.py` 50–57: a seat's CLI process dies at every clear, so a PID lock
//! looks abandoned in the middle of a repair). No file lock, no shell `flock` (PortableGit has none).
//! The lease is two hours, renewed by every mutating verb of its holder while it has not expired.
//! It is released by the daemon when the seat is reaped and when the daemon restarts (a lock whose
//! owner is this lane with another incarnation is dead — the "incarnation check at load"), and by
//! lease expiry: an abandoned lock never blocks a boot-critical repair for longer than the lease (F3).
//! Everything here is pure (the caller passes `now`); the per-repair mutex that serialises the
//! read-modify-write of a record lives in the daemon (G6, R3).

use super::grant::EXPIRY_GRACE_SECS;
use super::types::{LaneKey, Lease, LockOwner, LEASE_SECS};
use std::path::Path;

/// A fresh lease for `owner`, expiring `LEASE_SECS` after `now`.
pub fn new_lease(owner: LockOwner, now: f64, claim: Option<u32>) -> Lease {
    Lease {
        owner,
        expires_at: now + LEASE_SECS as f64,
        claim,
    }
}

/// Same lane, same seat, same daemon incarnation.
pub fn owned_by(lease: &Lease, owner: &LockOwner) -> bool {
    lease.owner.lane == owner.lane
        && lease.owner.seat == owner.seat
        && lease.owner.incarnation == owner.incarnation
}

/// A lock of THIS lane taken under another daemon incarnation is dead: the daemon that granted it
/// restarted, which released every lock it held (C1 1.12 — the "incarnation check at load").
/// A lock of another lane is judged by its lease only (this daemon cannot see that lane's
/// incarnation; such a lock is never written by this daemon — one writer per file).
pub fn released_by_restart(lease: &Lease, lane: &LaneKey, incarnation: &str) -> bool {
    lease.owner.lane == *lane && lease.owner.incarnation != incarnation
}

/// RULINGS G7(c): the holder renews only while `now < expires_at` — an expired lock is never
/// silently renewed.
pub fn holder_may_renew(lease: &Lease, now: f64) -> bool {
    now < lease.expires_at
}

/// The renewed lease (same owner and claim, a new two-hour expiry), or `None` once it expired.
pub fn renewed(lease: &Lease, now: f64) -> Option<Lease> {
    if holder_may_renew(lease, now) {
        Some(Lease {
            owner: lease.owner.clone(),
            expires_at: now + LEASE_SECS as f64,
            claim: lease.claim,
        })
    } else {
        None
    }
}

/// RULINGS G7(a): for anyone but its holder a lock counts as expired only on a read whose own
/// timestamp is later than `expires_at` + 60 s; a lock released by a restart is not live.
pub fn live_for_others(lease: &Lease, now: f64, lane: &LaneKey, incarnation: &str) -> bool {
    !released_by_restart(lease, lane, incarnation)
        && now <= lease.expires_at + EXPIRY_GRACE_SECS as f64
}

/// G7(c): a claim with a `.closed` marker authorises no mutating verb (work needs a new claim).
/// `None` = a lock bound to no grant claim (owner-ordered / update / `resolves` repairs).
pub fn claim_closed(repairs_root: &Path, claim: Option<u32>) -> bool {
    match claim {
        None => false,
        Some(k) => {
            let marker = repairs_root.join("grant-seq").join(format!("{k}.closed"));
            // Same layout as `paths::grant_closed_path` (pinned by a test); `symlink_metadata`
            // so a dangling marker still counts as present (the safe direction).
            std::fs::symlink_metadata(marker).is_ok()
        }
    }
}

/// How a caller stands towards a repair's lock.
#[derive(Debug, Clone, PartialEq)]
pub enum LockView {
    /// No live lock (none, released by a restart, or expired for others per G7(a)).
    Free,
    /// The caller's own lock, still renewable.
    Mine,
    /// The caller's own lock, expired but not yet past the 60-s grace — refused, never renewed.
    MineExpired,
    /// Another owner's live lock → `repair_locked` (G6).
    HeldByOther(LockOwner, f64),
}

/// Classifies `lease` for `caller` at `now` in the daemon of `lane` / `incarnation`.
pub fn view(
    lease: Option<&Lease>,
    caller: &LockOwner,
    now: f64,
    lane: &LaneKey,
    incarnation: &str,
) -> LockView {
    let Some(l) = lease else {
        return LockView::Free;
    };
    if released_by_restart(l, lane, incarnation) {
        return LockView::Free;
    }
    if owned_by(l, caller) {
        if holder_may_renew(l, now) {
            return LockView::Mine;
        }
        if now <= l.expires_at + EXPIRY_GRACE_SECS as f64 {
            return LockView::MineExpired;
        }
        return LockView::Free;
    }
    if live_for_others(l, now, lane, incarnation) {
        LockView::HeldByOther(l.owner.clone(), l.expires_at)
    } else {
        LockView::Free
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owner(seat: u64, inc: &str) -> LockOwner {
        LockOwner {
            lane: LaneKey("base".into()),
            seat,
            incarnation: inc.into(),
        }
    }

    /// The owner is lane + seat + incarnation — there is no process id anywhere in the lease.
    #[test]
    fn lock_owner_is_lane_seat_incarnation_never_pid() {
        let l = new_lease(owner(7, "inc-a"), 1000.0, None);
        assert_eq!(l.expires_at, 1000.0 + 7200.0);
        let v = serde_json::to_value(&l).unwrap();
        let mut keys: Vec<&String> = v["owner"].as_object().unwrap().keys().collect();
        keys.sort();
        assert_eq!(keys, ["incarnation", "lane", "seat"]);
        let src = include_str!("lock.rs");
        let prod = &src[..src.find("#[cfg(test)]").unwrap()];
        for needle in ["process::id", "getpid", "flock(", "LOCK_EX"] {
            assert!(!prod.contains(needle), "lock.rs uses {needle}");
        }
        assert!(owned_by(&l, &owner(7, "inc-a")));
        assert!(!owned_by(&l, &owner(8, "inc-a")));
        assert!(!owned_by(&l, &owner(7, "inc-b")));
        let mut other_lane = owner(7, "inc-a");
        other_lane.lane = LaneKey("dept-x".into());
        assert!(!owned_by(&l, &other_lane));
    }

    /// G7(a)/(c): renew only before expiry; others see it live until expiry + 60 s.
    #[test]
    fn lease_expiry_table() {
        let lane = LaneKey("base".into());
        let l = new_lease(owner(7, "inc-a"), 0.0, Some(3));
        let exp = l.expires_at;
        assert!(holder_may_renew(&l, exp - 0.5));
        assert!(!holder_may_renew(&l, exp));
        let r = renewed(&l, 100.0).unwrap();
        assert_eq!(r.expires_at, 100.0 + 7200.0);
        assert_eq!(r.claim, Some(3));
        assert!(
            renewed(&l, exp + 1.0).is_none(),
            "an expired lock is never renewed"
        );
        assert!(live_for_others(&l, exp + 59.0, &lane, "inc-a"));
        assert!(live_for_others(&l, exp + 60.0, &lane, "inc-a"));
        assert!(!live_for_others(&l, exp + 60.5, &lane, "inc-a"));
        let me = owner(7, "inc-a");
        let other = owner(9, "inc-a");
        assert_eq!(view(Some(&l), &me, 10.0, &lane, "inc-a"), LockView::Mine);
        assert_eq!(
            view(Some(&l), &me, exp + 1.0, &lane, "inc-a"),
            LockView::MineExpired
        );
        assert_eq!(
            view(Some(&l), &me, exp + 61.0, &lane, "inc-a"),
            LockView::Free
        );
        assert!(matches!(
            view(Some(&l), &other, exp + 30.0, &lane, "inc-a"),
            LockView::HeldByOther(_, _)
        ));
        assert_eq!(
            view(Some(&l), &other, exp + 61.0, &lane, "inc-a"),
            LockView::Free
        );
        assert_eq!(view(None, &other, 0.0, &lane, "inc-a"), LockView::Free);
    }

    /// C1 1.12: a daemon restart releases its lane's locks (incarnation check at load); another
    /// lane's lock is judged by its lease only.
    #[test]
    fn restart_releases_this_lanes_locks_only() {
        let lane = LaneKey("base".into());
        let l = new_lease(owner(7, "inc-old"), 0.0, None);
        assert!(released_by_restart(&l, &lane, "inc-new"));
        assert!(!released_by_restart(&l, &lane, "inc-old"));
        assert_eq!(
            view(Some(&l), &owner(7, "inc-new"), 1.0, &lane, "inc-new"),
            LockView::Free
        );
        assert_eq!(
            view(Some(&l), &owner(9, "inc-new"), 1.0, &lane, "inc-new"),
            LockView::Free
        );
        let mut foreign = l.clone();
        foreign.owner.lane = LaneKey("dept-x".into());
        assert!(!released_by_restart(&foreign, &lane, "inc-new"));
        assert!(matches!(
            view(Some(&foreign), &owner(9, "inc-new"), 1.0, &lane, "inc-new"),
            LockView::HeldByOther(_, _)
        ));
    }

    /// G7(c): a `.closed` marker of the lock's claim refuses; the marker path is the T0 layout.
    #[test]
    fn closed_claim_marker() {
        let root = std::env::temp_dir().join(format!(
            "cys-repair-lock-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("grant-seq")).unwrap();
        assert!(!claim_closed(&root, None));
        assert!(!claim_closed(&root, Some(4)));
        std::fs::write(root.join("grant-seq").join("4.closed"), b"").unwrap();
        assert!(claim_closed(&root, Some(4)));
        assert!(!claim_closed(&root, Some(5)));
        // (the layout equals `paths::grant_closed_path` — pinned in ledger::storage_tests)
        let _ = std::fs::remove_dir_all(&root);
    }
}
