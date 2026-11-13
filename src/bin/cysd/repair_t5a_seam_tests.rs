//! Track T5a — the T3 ↔ T5a seam (RULINGS AN1, completing AI6): the daemon classifies a target
//! against the REAL compiled lists — the protection list first (a file in both lists is
//! protection-related), then the boot-critical list — and refuses to unregister a protection hook.
use super::ledger_verbs::hook_allowed;
use super::*;
use cys::repair::types::Classification;
use std::path::{Path, PathBuf};

/// A lane pack root for the classification (RULINGS BN2: `<pack root>/<rel>` is classified). It
/// does not need to exist — the lists decide; the tests that need a real tree make their own.
fn pk() -> PathBuf {
    std::env::temp_dir().join(format!("t5a-seam-pack-{}", std::process::id()))
}

#[test]
fn protection_rows_classify_protection_related() {
    for rel in ["hooks/guard.sh", "trusted-keys.json"] {
        assert!(
            cys::repair::boot_critical::protection_entry(rel).is_some(),
            "{rel} is not a protection row of the compiled list"
        );
        assert_eq!(
            production_classify(rel, &pk()).unwrap(),
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
        production_classify(rel, &pk()).unwrap(),
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
        production_classify("bin/javis_bootstrap.py", &pk()).unwrap(),
        Classification::BootCritical
    );
    let dropped = "bin/javis_phoenix.py";
    assert!(cys::repair::boot_critical::boot_critical_entry(dropped).is_none());
    assert!(cys::repair::boot_critical::protection_entry(dropped).is_none());
    assert_eq!(
        production_classify(dropped, &pk()).unwrap(),
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
            production_classify(rel, &pk()).unwrap(),
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
        let e = production_classify(spelled, &pk()).unwrap_err();
        let msg = format!("{e:?}");
        assert!(msg.contains(row), "{spelled}: {msg}");
    }
    // the exact spellings classify
    assert_eq!(
        production_classify("hooks/inject-context.sh", &pk()).unwrap(),
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
        production_classify("bin/javis_guard_register.py", &pk()).unwrap(),
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
    // a tilde that is not a short-name form (`~` not followed by a digit) is not refused for it
    // (RULINGS BN3 (d); `parse_hook` refuses a `~` in a name before `hook_allowed` is reached)
    assert!(hook_allowed("verify~reminder.sh").is_ok());
    assert!(hook_allowed("verify-reminder.sh").is_ok());
}

/// The spellings of a row with U+212A (Kelvin sign) for its first `k` and U+017F (long s) for its
/// first `s` — APFS opens both as the row's file.
fn fold_variants(row: &str) -> Vec<String> {
    let mut v = Vec::new();
    for (from, to) in [('k', '\u{212A}'), ('s', '\u{17F}')] {
        if let Some(i) = row.find(from) {
            v.push(format!("{}{to}{}", &row[..i], &row[i + 1..]));
        }
    }
    v
}

/// RULINGS BN1: a spelling that equals a row only under the FULL case fold is refused naming the
/// row — one boot-critical row, every protection row, and the path-rule row with its file absent;
/// a non-ASCII name that folds to no row is General.
#[test]
fn full_fold_variants_of_a_row_are_refused_naming_the_row() {
    let bc = "hooks/inject-context.sh";
    let mut cases: Vec<(String, &str)> = fold_variants(bc).into_iter().map(|v| (v, bc)).collect();
    assert_eq!(cases.len(), 2);
    for row in cys::repair::boot_critical::protection_entries() {
        let vs = fold_variants(row.rel);
        assert!(!vs.is_empty(), "{}", row.rel);
        cases.extend(vs.into_iter().map(|v| (v, row.rel)));
    }
    let pack = pk();
    assert!(
        !pack.join("state/hook-targets.json").exists(),
        "the path-rule row's file must be absent here"
    );
    cases.push((
        "\u{17F}tate/hoo\u{212A}-targets.json".to_string(),
        "state/hook-targets.json",
    ));
    for (spelled, row) in &cases {
        let e = production_classify(spelled, &pack).unwrap_err();
        let msg = format!("{e:?}");
        assert!(msg.contains(&format!("`{row}`")), "{spelled}: {msg}");
    }
    for other in [
        "bin/caf\u{E9}.py",
        "bin/javi\u{17F}_phoenix.py",
        "hooks/\u{212A}.sh",
    ] {
        assert_eq!(
            production_classify(other, &pack).unwrap(),
            Classification::General,
            "{other}"
        );
    }
    // hook names: a protection or lifeline name under the full fold is refused
    assert!(hook_allowed("guard.\u{17F}h").is_err());
    assert!(hook_allowed("role-capability-gate.\u{17F}h").is_err());
    for l in cys::repair::boot_critical::LIFELINE_HOOKS {
        for v in fold_variants(l) {
            assert!(
                hook_allowed(&v).is_err(),
                "lifeline variant {v} was allowed"
            );
        }
    }
}

/// Restores the process's working directory when dropped (the cysd suite runs one test at a time).
struct CwdGuard(PathBuf);
impl Drop for CwdGuard {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self.0);
    }
}

fn scratch(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let d = std::env::temp_dir().join(format!("t5a-{tag}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// RULINGS BN2: the classification never depends on the daemon's working directory — removed
/// after start, a general target still classifies General and a listed one stays listed.
#[test]
fn a_removed_working_directory_does_not_change_the_classification() {
    let _back = CwdGuard(std::env::current_dir().unwrap());
    let gone = scratch("gone-cwd");
    std::env::set_current_dir(&gone).unwrap();
    std::fs::remove_dir(&gone).unwrap();
    assert!(std::env::current_dir().is_err() || !gone.exists());
    let pack = pk();
    assert_eq!(
        production_classify("bin/javis_phoenix.py", &pack).unwrap(),
        Classification::General
    );
    assert_eq!(
        production_classify("bin/javis_bootstrap.py", &pack).unwrap(),
        Classification::BootCritical
    );
    assert_eq!(
        production_classify("hooks/guard.sh", &pack).unwrap(),
        Classification::ProtectionRelated
    );
}

/// RULINGS BN2: the working directory inside a pack's `hooks/` — an unlisted rel named like a
/// row's basename stays General, the listed rels stay listed; a relative pack root is refused.
#[test]
fn a_working_directory_inside_a_pack_does_not_change_the_classification() {
    let _back = CwdGuard(std::env::current_dir().unwrap());
    let pack = scratch("cwd-pack");
    std::fs::create_dir_all(pack.join("hooks")).unwrap();
    for f in ["guard.sh", "inject-context.sh"] {
        std::fs::write(pack.join("hooks").join(f), "#!/bin/sh\n").unwrap();
    }
    std::env::set_current_dir(pack.join("hooks")).unwrap();
    for rel in ["guard.sh", "inject-context.sh"] {
        assert_eq!(
            production_classify(rel, &pack).unwrap(),
            Classification::General,
            "{rel}"
        );
    }
    assert_eq!(
        production_classify("hooks/guard.sh", &pack).unwrap(),
        Classification::ProtectionRelated
    );
    assert_eq!(
        production_classify("hooks/inject-context.sh", &pack).unwrap(),
        Classification::BootCritical
    );
    assert!(production_classify("bin/javis_phoenix.py", Path::new("rel/pack")).is_err());
    drop(_back);
    let _ = std::fs::remove_dir_all(&pack);
}

/// RULINGS AW3 (d): a target whose live file is the SAME file (volume + file id) as a listed row's
/// live file under another rel — a hard link — is refused naming the row's spelling, for a
/// protection row and a boot-critical row; the row's own rel is classified by its own rule; the
/// exported function `commit` step (3a) calls refuses the same set.
#[test]
fn a_hard_link_to_a_listed_file_is_refused_naming_the_row() {
    use cys::repair::boot_critical::refuse_identity_alias;
    let pack = scratch("aw3d-pack");
    for d in ["hooks", "notes"] {
        std::fs::create_dir_all(pack.join(d)).unwrap();
    }
    for (row, alias) in [
        ("hooks/guard.sh", "notes/g.sh"),
        ("hooks/inject-context.sh", "notes/ic.sh"),
    ] {
        std::fs::write(pack.join(row), "#!/bin/sh\n").unwrap();
        std::fs::hard_link(pack.join(row), pack.join(alias)).unwrap();
        let e = production_classify(alias, &pack).unwrap_err();
        assert!(
            format!("{e:?}").contains(&format!("`{row}`")),
            "{alias}: {e:?}"
        );
    }
    assert_eq!(
        production_classify("hooks/guard.sh", &pack).unwrap(),
        Classification::ProtectionRelated
    );
    assert_eq!(
        production_classify("hooks/inject-context.sh", &pack).unwrap(),
        Classification::BootCritical
    );
    std::fs::write(pack.join("notes/plain.sh"), "#!/bin/sh\n").unwrap();
    assert_eq!(
        production_classify("notes/plain.sh", &pack).unwrap(),
        Classification::General
    );
    let e = refuse_identity_alias(&pack, ["notes/plain.sh", "notes/ic.sh"]).unwrap_err();
    assert!(e.contains("`hooks/inject-context.sh`"), "{e}");
    assert!(refuse_identity_alias(&pack, ["notes/plain.sh", "hooks/guard.sh"]).is_ok());
    let _ = std::fs::remove_dir_all(&pack);
}

/// RULINGS AW3 (d): no false refusal — listed files absent (the path-rule row
/// `state/hook-targets.json` included), or the target itself absent.
#[test]
fn an_absent_listed_file_gives_no_false_refusal() {
    use cys::repair::boot_critical::refuse_identity_alias;
    let pack = scratch("aw3d-absent");
    std::fs::create_dir_all(pack.join("notes")).unwrap();
    std::fs::write(pack.join("notes/x.json"), "{}\n").unwrap();
    assert!(!pack.join("state/hook-targets.json").exists());
    assert_eq!(
        production_classify("notes/x.json", &pack).unwrap(),
        Classification::General
    );
    assert_eq!(
        production_classify("notes/not-there.sh", &pack).unwrap(),
        Classification::General
    );
    assert!(refuse_identity_alias(&pack, ["notes/x.json", "notes/not-there.sh"]).is_ok());
    let _ = std::fs::remove_dir_all(&pack);
}

/// RULINGS CD1: the fold is a fixpoint — with the listed file ABSENT, the capital sharp s (U+1E9E,
/// which one upper-then-lower pass turns only into `ß`) for "ss" in a boot-critical row is refused
/// naming the row; the same through the `st` ligature (U+FB06) for a protection row and for the
/// path-rule row (no protection row holds "ss"); a rule document folds the same way.
#[test]
fn a_capital_sharp_s_spelling_of_an_absent_row_is_refused_naming_the_row() {
    let pack = pk();
    for (spelled, row) in [
        ("hooks/se\u{1E9E}ion-start.sh", "hooks/session-start.sh"),
        ("hooks/teamtoken-i\u{1E9E}ue.sh", "hooks/teamtoken-issue.sh"),
        ("bin/_owned_proce\u{1E9E}.py", "bin/_owned_process.py"),
        ("bin/javis_mi\u{1E9E}ion.py", "bin/javis_mission.py"),
        ("tru\u{FB06}ed-keys.json", "trusted-keys.json"),
        ("\u{FB06}ate/hook-targets.json", "state/hook-targets.json"),
    ] {
        assert!(!pack.join(row).exists(), "{row} must be absent here");
        let e = production_classify(spelled, &pack).unwrap_err();
        assert!(
            format!("{e:?}").contains(&format!("`{row}`")),
            "{spelled}: {e:?}"
        );
    }
    assert!(hook_allowed("se\u{1E9E}ion-start.sh").is_err(), "lifeline");
    assert_eq!(
        super::rule_document_name("directives/BO\u{1E9E}_DIRECTIVE.md").as_deref(),
        Some("boss_directive.md")
    );
}
