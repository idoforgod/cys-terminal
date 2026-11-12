//! 「스스로 해결」 daemon side — the `repair.*` request namespace (C3 · O-C3-1 · T0 skeleton).
//!
//! `handlers::dispatch_method` delegates every `repair.<sub>` request here (one prefix branch beside
//! `trash.`; the caller's seat and the owner grade are computed THERE, not in this module).
//! T0: every known sub-verb answers `repair_not_implemented`, an unknown one `method_not_found`.
//! ★Nothing here may panic (F4 — a panicking handler would take the daemon and every pane with it):
//! no `unwrap`/`expect`/indexing/`todo!`/`unimplemented!` in production code of this module.
//! The tracks fill this module in place (T3 verbs and role table, T5b approval, T6 job/tick).
#![allow(dead_code)] // skeleton items (tables, origin record, lock release) are wired by later tracks

use crate::handlers::Reply;
use crate::state::Daemon;
use cys::err_response;
use serde_json::Value;
use std::sync::Arc;

/// The daemon sub-verbs of `repair.` (O-C3-1, N-4; `recheck` H5/H8, `grant_ask` Q1,
/// `owner_token_ask` Q2) — 23.
pub(crate) const REPAIR_SUBVERBS: &[&str] = &[
    "report",
    "begin",
    "record",
    "submit",
    "commit",
    "attest_verify",
    "attest_audit",
    "list",
    "status",
    "revert",
    "reapply",
    "cleanup",
    "approval_list",
    "approval_start",
    "approval_reject",
    "grant",
    "owner_token",
    "rule_doc_allowed",
    "sweep",
    "tick",
    "recheck",
    "grant_ask",
    "owner_token_ask",
];

/// Handles `repair.<sub>`. `caller_sid` = the caller's seat from the kernel peer pid's ancestry
/// (`resolve_caller_surface`), `owner_grade` = `trash_owner_grade_ok` (no seat + operator token).
pub(crate) fn handle(
    daemon: &Arc<Daemon>,
    sub: &str,
    params: &Value,
    id: &Value,
    caller_sid: Option<u64>,
    owner_grade: bool,
    caller_pid: Option<u32>,
) -> Reply {
    let _ = (daemon, params, caller_sid, owner_grade, caller_pid);
    if REPAIR_SUBVERBS.contains(&sub) {
        Reply::Single(err_response(
            id,
            cys::repair::types::ERR_NOT_IMPLEMENTED,
            &format!("repair.{sub}: {}", cys::repair::NOT_IMPLEMENTED_KO),
        ))
    } else {
        Reply::Single(err_response(
            id,
            "method_not_found",
            &format!("unknown method: repair.{sub}"),
        ))
    }
}

/// The pending final-approval requests (C5; memory only — void after a restart). T5b fills it.
#[derive(Default)]
pub(crate) struct ApprovalTable {}

/// The per-seat owner-prompt record (T3 writes, T4 reads). `named_docs` = rule-document base names,
/// lower-cased, found in the seat's latest owner-typed prompt (RULINGS U4(a)); no prompt text kept.
#[derive(Default, Clone)]
pub(crate) struct PromptOrigin {
    pub digest: String,
    pub origin: String,
    pub at: f64,
    pub question: Option<OpenQuestion>,
    pub named_docs: Vec<String>,
}

/// An open question of a master seat (RULINGS T3, Q1 / Q2): `first_human` = (digest, excerpt, at).
#[derive(Default, Clone)]
pub(crate) struct OpenQuestion {
    pub asked_at: f64,
    pub path: Option<String>,
    pub first_human: Option<(String, String, f64)>,
    pub machine_seen: bool,
}

/// Releases every repair lock held by a seat (C1 1.12: seat reap / close). No-op in T0; T3 wires
/// it into `close_surface_effect` / `reap_surface_due`.
pub(crate) fn release_locks_for_seat(daemon: &Arc<Daemon>, sid: u64) {
    let _ = (daemon, sid);
}

#[cfg(test)]
mod tests {
    use super::*;
    use cys::repair::roles::{verb_class, VerbClass, VERB_CLASSES};
    use cys::Request;
    use serde_json::json;
    use std::sync::atomic::Ordering;

    fn code_of(reply: Reply) -> (String, String) {
        let Reply::Single(v) = reply else {
            panic!("single reply expected")
        };
        (
            v["error"]["code"].as_str().unwrap_or("").to_string(),
            v["error"]["message"].as_str().unwrap_or("").to_string(),
        )
    }

    /// Each of the 23 sub-verbs → `repair_not_implemented` (whatever the params, caller and grade);
    /// an unknown sub → `method_not_found`. Nothing panics.
    #[test]
    fn every_subverb_is_not_implemented_and_never_panics() {
        assert_eq!(REPAIR_SUBVERBS.len(), 23);
        let mut uniq = REPAIR_SUBVERBS.to_vec();
        uniq.sort_unstable();
        uniq.dedup();
        assert_eq!(uniq.len(), 23, "REPAIR_SUBVERBS has duplicates");
        let daemon = crate::team_gate_tests::tmp_daemon("repair-t0-subverbs", false);
        let params_set = [
            json!({}),
            json!(null),
            json!({"repair": "R-1", "files": ["a"], "role": "master"}),
            json!([1, 2]),
        ];
        for sub in REPAIR_SUBVERBS {
            for params in &params_set {
                for (sid, grade, pid) in [
                    (None, false, None),
                    (Some(3), false, Some(1)),
                    (None, true, Some(u32::MAX)),
                ] {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        code_of(handle(&daemon, sub, params, &json!(7), sid, grade, pid))
                    }));
                    let (code, msg) = result.unwrap_or_else(|_| panic!("repair.{sub} panicked"));
                    assert_eq!(code, "repair_not_implemented", "repair.{sub}");
                    assert_eq!(
                        msg,
                        format!("repair.{sub}: 이 빌드에는 아직 구현되지 않았습니다")
                    );
                }
            }
        }
        for sub in [
            "",
            "nope",
            "Begin",
            "begin ",
            "attest-verify",
            "approval.start",
            "../x",
            "\u{0}",
        ] {
            let (code, msg) = code_of(handle(
                &daemon,
                sub,
                &json!({}),
                &json!(1),
                None,
                false,
                None,
            ));
            assert_eq!(code, "method_not_found", "repair.{sub:?}");
            assert_eq!(msg, format!("unknown method: repair.{sub}"));
        }
    }

    /// J1 census: every daemon sub-verb has exactly one class; the table is exactly the 23 daemon
    /// verbs + the 5 local verbs; the WORK / OWNER-DECISION / OWNER-APPROVAL sets as C3.
    #[test]
    fn verb_class_census() {
        for sub in REPAIR_SUBVERBS {
            let n = VERB_CLASSES.iter().filter(|(v, _)| v == sub).count();
            assert_eq!(n, 1, "{sub} has {n} classes in VERB_CLASSES");
        }
        let local = [
            "gate",
            "rules",
            "boot-critical",
            "tombstones",
            "auth-helper",
        ];
        let mut table: Vec<&str> = VERB_CLASSES.iter().map(|(v, _)| *v).collect();
        table.sort_unstable();
        let mut expected: Vec<&str> = REPAIR_SUBVERBS.iter().copied().chain(local).collect();
        expected.sort_unstable();
        assert_eq!(
            table, expected,
            "VERB_CLASSES must be exactly the 23 daemon verbs + the 5 local verbs"
        );
        assert_eq!(VERB_CLASSES.len(), 28);
        let of = |c: VerbClass| -> Vec<&str> {
            let mut v: Vec<&str> = VERB_CLASSES
                .iter()
                .filter(|(_, k)| *k == c)
                .map(|(v, _)| *v)
                .collect();
            v.sort_unstable();
            v
        };
        let sorted = |mut v: Vec<&'static str>| {
            v.sort_unstable();
            v
        };
        assert_eq!(
            of(VerbClass::Work),
            sorted(vec![
                "begin", "record", "submit", "commit", "reapply", "recheck", "cleanup", "revert"
            ])
        );
        assert_eq!(
            of(VerbClass::OwnerDecision),
            sorted(vec!["grant", "owner_token", "grant_ask", "owner_token_ask"])
        );
        assert_eq!(
            of(VerbClass::OwnerApproval),
            sorted(vec!["approval_start", "approval_reject", "auth-helper"])
        );
        assert_eq!(
            of(VerbClass::Review),
            sorted(vec!["attest_verify", "attest_audit"])
        );
        assert_eq!(of(VerbClass::Trigger), sorted(vec!["sweep", "tick"]));
        assert_eq!(of(VerbClass::Intake), vec!["report"]);
        assert_eq!(
            of(VerbClass::Read),
            sorted(vec![
                "list",
                "status",
                "rule_doc_allowed",
                "approval_list",
                "gate",
                "rules",
                "boot-critical",
                "tombstones"
            ])
        );
        assert_eq!(verb_class("no-such-verb"), None);
        assert_eq!(
            verb_class("attest-verify"),
            None,
            "daemon verbs are listed by their wire names"
        );
    }

    /// `repair.status` through the real dispatcher reaches this module (no `method_not_found`), and a
    /// request outside the namespace still takes its old path.
    #[test]
    fn repair_prefix_is_delegated() {
        let daemon = crate::team_gate_tests::tmp_daemon("repair-t0-dispatch", false);
        let req = |m: &str| Request {
            id: json!(1),
            method: m.into(),
            params: json!({}),
        };
        let (code, _) = code_of(crate::handlers::dispatch(
            &daemon,
            req("repair.status"),
            None,
        ));
        assert_ne!(code, "method_not_found");
        assert_eq!(code, "repair_not_implemented");
        let (code, _) = code_of(crate::handlers::dispatch(
            &daemon,
            req("repair.no_such_verb"),
            Some(std::process::id()),
        ));
        assert_eq!(code, "method_not_found");
        let Reply::Single(pong) = crate::handlers::dispatch(&daemon, req("system.ping"), None)
        else {
            panic!("single reply")
        };
        assert_eq!(
            pong["result"],
            json!("pong"),
            "a non-repair request must take its old path: {pong}"
        );
        let (code, _) = code_of(crate::handlers::dispatch(
            &daemon,
            req("repairs.status"),
            None,
        ));
        assert_ne!(
            code, "repair_not_implemented",
            "only the exact `repair.` prefix is delegated"
        );
    }

    /// The seven Daemon fields start at their defaults.
    #[test]
    fn daemon_repair_fields_default() {
        let daemon = crate::team_gate_tests::tmp_daemon("repair-t0-fields", false);
        assert!(!daemon.listener_ready.load(Ordering::SeqCst));
        assert!(!daemon.repair_tick_running.load(Ordering::SeqCst));
        assert_eq!(daemon.ready_since.load(Ordering::SeqCst), 0);
        assert_eq!(daemon.repair_last_tick.load(Ordering::SeqCst), 0);
        assert!(daemon
            .repair_prompt_origin
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_empty());
        assert!(daemon
            .repair_mutexes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_empty());
        let _table: &ApprovalTable = &daemon
            .repair_approvals
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        release_locks_for_seat(&daemon, 1); // the T0 no-op must not touch anything
        assert!(daemon
            .repair_mutexes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_empty());
    }

    /// The library's `cso*` role prefix (roles::role_block_for) is the daemon's alert-route prefix.
    #[test]
    fn role_block_cso_prefix_equals_alert_route() {
        use cys::repair::roles::{role_block_for, RoleBlock};
        let prefix = crate::alert_route::CSO_ROLE_PREFIX;
        for role in [
            prefix.to_string(),
            format!("{prefix}-2"),
            format!("{prefix}x"),
        ] {
            assert_eq!(
                crate::alert_route::is_cso_role(&role),
                role_block_for(&role) == RoleBlock::Cso,
                "{role}"
            );
        }
        assert_eq!(role_block_for(prefix), RoleBlock::Cso);
    }
}
