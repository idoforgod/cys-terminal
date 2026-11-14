//! RULINGS EL5 / EN8 — whether an approval was CONSUMED by an apply of this lane, decided by the
//! apply's durable evidence (T6 holds it; T5b declares the same file and builds against it): the
//! approved apply records the approval reference in its intent before the first exchange and in the
//! lane row it writes. Three answers:
//! - `Consumed` — an apply intent of this lane (any state) or this lane's row names the reference;
//! - `NotConsumed` — the lane's ledger is complete and readable and names it nowhere;
//! - `Unknown` — anything else (an unreadable record, row or intent).

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
        if let Ok(Some(row)) = super::ledger::read_lane_in(root, &id, lane) {
            if row.approval_ref.as_deref() == Some(approval_ref) {
                return ApprovalUse::Consumed;
            }
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
}
