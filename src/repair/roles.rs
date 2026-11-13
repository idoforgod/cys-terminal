//! Role tables of the repair feature: reviewer slots (RULINGS B2), role blocks (O-C6-3) and the
//! per-verb role classes (RULINGS J1, census of C3), plus the caller kind (RULINGS M1/M4).

/// The product's reviewer slots — a compiled copy of `cysjavis-pack/bin/javis_orchestra.py:REVIEWER_SLOTS`
/// (parity test below; NOT `javis_radio.py`, which has a different constant of the same name).
/// Row = (native role, native agent, substitute role, substitute agent).
pub const REVIEWER_SLOTS: [(&str, &str, &str, &str); 2] = [
    ("reviewer-gemini", "gemini", "reviewer-claude-1", "claude"),
    ("reviewer-codex", "codex", "reviewer-claude-2", "claude"),
];

/// reviewer1 = the codex slot (verification, `attest-verify`); reviewer2 = the gemini slot (audit,
/// `attest-audit`) — RULINGS B2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewerSlot {
    Reviewer1,
    Reviewer2,
}

/// The slot a seat role stands in (native or substitute name); `None` for every other role.
pub fn reviewer_slot_of(role: &str) -> Option<ReviewerSlot> {
    for (native, agent, substitute, _) in REVIEWER_SLOTS {
        if role == native || role == substitute {
            return match agent {
                "codex" => Some(ReviewerSlot::Reviewer1),
                "gemini" => Some(ReviewerSlot::Reviewer2),
                _ => None,
            };
        }
    }
    None
}

/// Which compiled rule block a seat receives (C6, O-C6-2: four role rows + the common part).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoleBlock {
    MasterCeo,
    Worker,
    Cso,
    Reviewer,
    CommonOnly,
}

/// Same value as the daemon's `alert_route::CSO_ROLE_PREFIX` (src/bin/cysd/alert_route.rs:132),
/// which lives in the daemon binary and cannot be imported here.
const CSO_ROLE_PREFIX: &str = "cso";

/// O-C6-3: `master` (any lane) and the CEO seat role `ceo` → master·CEO; `worker`, `worker-*` →
/// worker; `cso*` → CSO; `reviewer*` (incl. the fallback seats `reviewer-claude-1/2`) → reviewer;
/// `planner*` and any unknown role → only the common part.
pub fn role_block_for(role: &str) -> RoleBlock {
    if role == "master" || role == "ceo" {
        RoleBlock::MasterCeo
    } else if role == "worker" || role.starts_with("worker-") {
        RoleBlock::Worker
    } else if role.starts_with(CSO_ROLE_PREFIX) {
        RoleBlock::Cso
    } else if role.starts_with("reviewer") {
        RoleBlock::Reviewer
    } else {
        RoleBlock::CommonOnly
    }
}

/// The role classes of RULINGS J1 (deny by default: a verb without a class is refused for all).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerbClass {
    Read,
    Intake,
    Trigger,
    OwnerDecision,
    OwnerApproval,
    Review,
    Work,
}

/// The per-verb role table: the 24 daemon wire names + the 5 local verbs in CLI spelling, classes
/// exactly as the CONTRACTS C3 table (J1; `auth-helper` per interpretation 29; `grant_status` READ
/// per RULINGS CI6 (b)).
pub const VERB_CLASSES: [(&str, VerbClass); 29] = [
    // READ (9)
    ("list", VerbClass::Read),
    ("status", VerbClass::Read),
    ("rule_doc_allowed", VerbClass::Read),
    ("grant_status", VerbClass::Read),
    ("approval_list", VerbClass::Read),
    ("gate", VerbClass::Read),
    ("rules", VerbClass::Read),
    ("boot-critical", VerbClass::Read),
    ("tombstones", VerbClass::Read),
    // INTAKE (1)
    ("report", VerbClass::Intake),
    // TRIGGER (2)
    ("sweep", VerbClass::Trigger),
    ("tick", VerbClass::Trigger),
    // OWNER-DECISION (4)
    ("grant", VerbClass::OwnerDecision),
    ("owner_token", VerbClass::OwnerDecision),
    ("grant_ask", VerbClass::OwnerDecision),
    ("owner_token_ask", VerbClass::OwnerDecision),
    // OWNER-APPROVAL (3)
    ("approval_start", VerbClass::OwnerApproval),
    ("approval_reject", VerbClass::OwnerApproval),
    ("auth-helper", VerbClass::OwnerApproval),
    // REVIEW (2)
    ("attest_verify", VerbClass::Review),
    ("attest_audit", VerbClass::Review),
    // WORK (8)
    ("begin", VerbClass::Work),
    ("record", VerbClass::Work),
    ("submit", VerbClass::Work),
    ("commit", VerbClass::Work),
    ("reapply", VerbClass::Work),
    ("recheck", VerbClass::Work),
    ("cleanup", VerbClass::Work),
    ("revert", VerbClass::Work),
];

/// The class of a verb; `None` = refused for every seat.
pub fn verb_class(verb: &str) -> Option<VerbClass> {
    VERB_CLASSES
        .iter()
        .find(|(v, _)| *v == verb)
        .map(|(_, c)| *c)
}

/// Who is calling, decided from the operating system only (RULINGS M1/M4): a seat (ancestry or,
/// on macOS, the pane's controlling terminal), a human (no seat + a non-pane terminal, or the
/// app's operator token; on Windows the operator token only), or neither (`Detached` — refused
/// wherever a worker or a human is required).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallerKind {
    Seat { sid: u64 },
    Human,
    Detached,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// B2: the Rust slot table equals `javis_orchestra.py:REVIEWER_SLOTS` (two languages, one test).
    #[test]
    fn reviewer_slots_parity_with_orchestra() {
        let py = include_str!("../../cysjavis-pack/bin/javis_orchestra.py");
        let head = "\nREVIEWER_SLOTS = [";
        assert_eq!(
            py.matches(head).count(),
            1,
            "javis_orchestra.py must define REVIEWER_SLOTS exactly once"
        );
        let start = py.find(head).unwrap() + head.len();
        let block = &py[start..start + py[start..].find("\n]").expect("end of REVIEWER_SLOTS")];
        let mut rows: Vec<Vec<String>> = Vec::new();
        for line in block
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
        {
            let quoted: Vec<String> = line
                .split('"')
                .skip(1)
                .step_by(2)
                .map(str::to_string)
                .collect();
            rows.push(quoted);
        }
        let rust: Vec<Vec<String>> = REVIEWER_SLOTS
            .iter()
            .map(|(a, b, c, d)| vec![a.to_string(), b.to_string(), c.to_string(), d.to_string()])
            .collect();
        assert_eq!(
            rows, rust,
            "REVIEWER_SLOTS drifted between javis_orchestra.py and cys::repair::roles"
        );
    }

    /// B2 slot mapping (native + substitute names) and O-C6-3 role blocks.
    #[test]
    fn slot_and_block_mapping() {
        assert_eq!(
            reviewer_slot_of("reviewer-codex"),
            Some(ReviewerSlot::Reviewer1)
        );
        assert_eq!(
            reviewer_slot_of("reviewer-claude-2"),
            Some(ReviewerSlot::Reviewer1)
        );
        assert_eq!(
            reviewer_slot_of("reviewer-gemini"),
            Some(ReviewerSlot::Reviewer2)
        );
        assert_eq!(
            reviewer_slot_of("reviewer-claude-1"),
            Some(ReviewerSlot::Reviewer2)
        );
        for other in [
            "reviewer",
            "reviewer-grok",
            "reviewer-claude-3",
            "master",
            "worker",
            "cso",
            "",
            "planner",
        ] {
            assert_eq!(reviewer_slot_of(other), None, "{other}");
        }
        let blocks = [
            ("master", RoleBlock::MasterCeo),
            ("ceo", RoleBlock::MasterCeo),
            ("worker", RoleBlock::Worker),
            ("worker-3", RoleBlock::Worker),
            ("cso", RoleBlock::Cso),
            ("cso-2", RoleBlock::Cso),
            ("reviewer-codex", RoleBlock::Reviewer),
            ("reviewer-gemini", RoleBlock::Reviewer),
            ("reviewer-claude-1", RoleBlock::Reviewer),
            ("reviewer-claude-2", RoleBlock::Reviewer),
            ("planner", RoleBlock::CommonOnly),
            ("planner-2", RoleBlock::CommonOnly),
            ("workerx", RoleBlock::CommonOnly),
            ("masters", RoleBlock::CommonOnly),
            ("", RoleBlock::CommonOnly),
            ("unknown-role", RoleBlock::CommonOnly),
        ];
        for (role, want) in blocks {
            assert_eq!(role_block_for(role), want, "{role:?}");
        }
    }
}
