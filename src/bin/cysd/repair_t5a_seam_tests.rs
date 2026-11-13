//! Track T5a — the T3 ↔ T5a seam (RULINGS AN1, completing AI6): the daemon classifies a target
//! against the REAL compiled lists — the protection list first (a file in both lists is
//! protection-related), then the boot-critical list — and refuses to unregister a protection hook.
use super::ledger_verbs::hook_allowed;
use super::*;
use cys::repair::types::Classification;

#[test]
fn protection_rows_classify_protection_related() {
    for rel in ["hooks/guard.sh", "trusted-keys.json"] {
        assert!(
            cys::repair::boot_critical::protection_entry(rel).is_some(),
            "{rel} is not a protection row of the compiled list"
        );
        assert_eq!(
            production_classify(rel).unwrap(),
            Classification::ProtectionRelated,
            "{rel}"
        );
    }
}

#[test]
fn a_file_in_both_lists_is_protection_related() {
    let rel = "hooks/role-capability-gate.sh";
    assert!(cys::repair::boot_critical::boot_critical_entry(rel).is_some());
    assert!(cys::repair::boot_critical::protection_entry(rel).is_some());
    assert_eq!(
        production_classify(rel).unwrap(),
        Classification::ProtectionRelated
    );
}

#[test]
fn a_protection_hook_cannot_be_unregistered() {
    let e = hook_allowed("guard.sh").unwrap_err();
    assert_eq!(e.code, cys::repair::types::ERR_PRECONDITION, "{e:?}");
    assert!(hook_allowed("role-capability-gate.sh").is_err());
    // an ordinary, non-lifeline hook can be
    assert!(hook_allowed("verify-reminder.sh").is_ok());
}

#[test]
fn a_listed_start_script_is_boot_critical_and_a_dropped_tool_is_general() {
    assert_eq!(
        production_classify("bin/javis_bootstrap.py").unwrap(),
        Classification::BootCritical
    );
    let dropped = "bin/javis_phoenix.py";
    assert!(cys::repair::boot_critical::boot_critical_entry(dropped).is_none());
    assert!(cys::repair::boot_critical::protection_entry(dropped).is_none());
    assert_eq!(
        production_classify(dropped).unwrap(),
        Classification::General
    );
}
