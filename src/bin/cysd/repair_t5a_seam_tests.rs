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

/// RULINGS AP2: the data tables a registrar reads to decide whether a protection hook is
/// registered are protection-related — the shipped fallback table and the operator's table (a
/// path rule: not shipped). The CSO directive (its marker conditions the gate's registration) is a
/// rule document, guarded by RULINGS AQ8, not a protection row.
#[test]
fn registration_tables_classify_protection_related() {
    for rel in ["state/hook-targets.json", "state/hook-targets.json.example"] {
        assert_eq!(
            production_classify(rel).unwrap(),
            Classification::ProtectionRelated,
            "{rel}"
        );
    }
    assert!(cys::repair::boot_critical::protection_entry("directives/CSO_DIRECTIVE.md").is_none());
}

/// RULINGS AP4: a spelling that matches a row only when case is ignored is refused, naming the
/// row's spelling (the check kind and the timeout are looked up by the exact spelling).
#[test]
fn a_case_variant_of_a_listed_file_is_refused_naming_the_row() {
    for (spelled, row) in [
        ("Hooks/Guard.sh", "hooks/guard.sh"),
        ("hooks/Inject-Context.sh", "hooks/inject-context.sh"),
        ("BIN/javis_bootstrap.py", "bin/javis_bootstrap.py"),
        ("State/Hook-Targets.json", "state/hook-targets.json"),
    ] {
        let e = production_classify(spelled).unwrap_err();
        let msg = format!("{e:?}");
        assert!(msg.contains(row), "{spelled}: {msg}");
    }
    // the exact spellings classify
    assert_eq!(
        production_classify("hooks/inject-context.sh").unwrap(),
        Classification::BootCritical
    );
}

/// RULINGS AP4: `hook_allowed` refuses a script name ending in a dot or a space, and compares
/// lifeline names ignoring case.
#[test]
fn hook_names_with_a_trailing_dot_or_space_and_lifeline_case_variants_are_refused() {
    for s in [
        "guard.sh.",
        "guard.sh ",
        "verify-reminder.sh.",
        "verify-reminder.sh ",
    ] {
        assert!(hook_allowed(s).is_err(), "{s:?} was allowed");
    }
    for l in cys::repair::boot_critical::LIFELINE_HOOKS {
        assert!(hook_allowed(l).is_err(), "lifeline {l} was allowed");
        let upper = l.to_ascii_uppercase();
        assert!(
            hook_allowed(&upper).is_err(),
            "lifeline {upper} was allowed"
        );
    }
    assert!(hook_allowed("GUARD.SH").is_err());
    assert!(hook_allowed("verify-reminder.sh").is_ok());
}

/// RULINGS AW2: the registrar's table validator is protection-related.
#[test]
fn the_registrars_validator_is_protection_related() {
    assert_eq!(
        production_classify("bin/javis_guard_register.py").unwrap(),
        Classification::ProtectionRelated
    );
}

/// RULINGS AW6: a protection hook's name is refused ignoring case and in short-name form.
#[test]
fn protection_hook_names_are_refused_in_any_case_and_short_name_form() {
    for s in [
        "GUARD.SH",
        "Guard.sh",
        "ROLE-CAPABILITY-GATE.SH",
        "ROLE-C~1.SH",
        "GUARD~1.SH",
        "role-c~2.sh",
    ] {
        assert!(hook_allowed(s).is_err(), "{s} was allowed");
    }
    // a tilde that is not a short-name form is not refused for it
    assert!(hook_allowed("verify-reminder.sh").is_ok());
}
