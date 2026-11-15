//! RULINGS EL5 / EN8 — whether an approval was CONSUMED by an apply of this lane, decided by the
//! apply's durable evidence (T6 holds it; T5b declares the same file and builds against it): the
//! approved apply records the approval reference in its intent before the first exchange and in the
//! lane row it writes. Three answers:
//! - `Consumed` — an apply intent of this lane (any state) or this lane's row names the reference;
//! - `NotConsumed` — the lane's ledger is complete and readable and names it nowhere;
//! - `Unknown` — anything else (an unreadable record, row or intent; one of a newer schema; a listing
//!   that failed) — never NotConsumed on a failed read (FG1).
//!
//! The EP condition (T5b's watchdog calls this HOLDING the daemon's per-repair lock and the
//! approval-table lock): the body takes NO lock — not the per-repair mutex, not the repair guard, not
//! the pack-apply lock, nothing that can be held during a wait for those — and never blocks: one
//! pass of bounded reads (the read layer's three answers), no wait, no retry. It reads only, under the
//! repairs root: the root's listing, each `R-<n>/record.json` and `R-<n>/lanes/<lane>.json` (the
//! completeness check), each `R-<n>/intents/<lane>.json` (the apply intent). Every file is replaced
//! atomically by its writer and the evidence is monotone (an intent is never deleted and keeps its
//! approval reference), so a lock-free read is consistent for this answer.

use super::types::LaneKey;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalUse {
    Consumed,
    NotConsumed,
    Unknown(String),
}

pub fn approval_consumed_in(root: &Path, lane: &LaneKey, approval_ref: &str) -> ApprovalUse {
    if let Some(cause) = super::apply::unreadable_ledger_member(root, lane) {
        return ApprovalUse::Unknown(format!("장부의 {cause} 을(를) 읽을 수 없습니다"));
    }
    // FG1: a failed enumeration or an unreadable intent is never "not consumed"
    let ids = match super::ledger::list_ids_checked_in(root) {
        Ok(ids) => ids,
        Err(e) => return ApprovalUse::Unknown(format!("수리 장부 목록을 읽을 수 없습니다({e:?})")),
    };
    for id in ids {
        match super::apply::read_intent_checked_in(root, &id, lane) {
            Ok(Some(intent)) if intent.approval_ref.as_deref() == Some(approval_ref) => return ApprovalUse::Consumed,
            Ok(_) => {}
            Err(e) => return ApprovalUse::Unknown(format!("{}: 적용 의도를 읽을 수 없습니다({e})", id.as_str())),
        }
        match super::ledger::read_lane_in(root, &id, lane) {
            Ok(Some(row)) if row.approval_ref.as_deref() == Some(approval_ref) => return ApprovalUse::Consumed,
            Ok(_) => {}
            Err(e) => return ApprovalUse::Unknown(format!("{}: 이 부서의 행을 읽을 수 없습니다({e:?})", id.as_str())),
        }
    }
    ApprovalUse::NotConsumed
}

pub fn approval_consumed(lane: &LaneKey, approval_ref: &str) -> ApprovalUse {
    approval_consumed_in(&super::paths::repairs_root(), lane, approval_ref)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repair::apply::tests::{approved_and_applied, rid, rig};

    /// EN8: Consumed once the approved apply ran (its intent and row name the reference);
    /// NotConsumed for a reference the complete ledger names nowhere; Unknown when a record or an
    /// intent cannot be read.
    #[test]
    fn approval_consumed_has_three_answers() {
        let r = rig("en8consumed");
        let id = rid("R-60");
        assert_eq!(approval_consumed_in(&r.root, &r.lane, "rqr-1"), ApprovalUse::NotConsumed, "nothing applied yet");
        approved_and_applied(&r, &id, "hooks/c.sh", b"PRE\n", b"POST\n");
        assert_eq!(approval_consumed_in(&r.root, &r.lane, "rqr-1"), ApprovalUse::Consumed);
        assert_eq!(approval_consumed_in(&r.root, &r.lane, "rqr-other"), ApprovalUse::NotConsumed);
        std::fs::write(crate::repair::apply::intent_path_in(&r.root, &id, &r.lane), b"{ broken").unwrap();
        assert!(matches!(approval_consumed_in(&r.root, &r.lane, "rqr-other"), ApprovalUse::Unknown(_)), "an unreadable intent");
        std::fs::write(crate::repair::ledger::record_path_in(&r.root, &id), b"{ broken").unwrap();
        assert!(matches!(approval_consumed_in(&r.root, &r.lane, "rqr-1"), ApprovalUse::Unknown(_)), "an unreadable record");
    }

    /// The EP condition: an intent of a NEWER schema that names the reference is a record this build
    /// cannot interpret — Unknown, never Consumed or NotConsumed; and the body takes no lock and has
    /// no loop that waits (source pin over this file's production text).
    #[test]
    fn a_newer_schema_is_unknown_and_the_body_takes_no_lock() {
        let r = rig("epschema");
        let id = rid("R-61");
        approved_and_applied(&r, &id, "hooks/s.sh", b"PRE\n", b"POST\n");
        let p = crate::repair::apply::intent_path_in(&r.root, &id, &r.lane);
        let mut v: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        v["schema"] = serde_json::json!(999);
        std::fs::write(&p, serde_json::to_vec(&v).unwrap()).unwrap();
        assert!(matches!(approval_consumed_in(&r.root, &r.lane, "rqr-1"), ApprovalUse::Unknown(_)));
        let src = include_str!("consumed.rs");
        let prod = &src[..src.find("#[cfg(test)]\nmod tests").unwrap()];
        for needle in [".lock(", "try_lock", "try_take", "RepairGuard", "with_apply_lock", "locked(", "sleep(", "loop {", "while "] {
            assert!(!prod.contains(needle), "the seam's body must not lock, wait or loop: {needle}");
        }
    }
}
