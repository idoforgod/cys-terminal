//! RULINGS CM3 — `production_check` hands T5b's runner the PACK root, so a target with a folder is
//! checked at `<pack>/<rel>` (before CM3: `<pack>/hooks/hooks/x.sh`, never found).

use std::path::PathBuf;

fn pack(name: &str) -> PathBuf {
    // RULINGS DE6: inside the replay's own scratch base, never HOME
    let p = cys::repair::rehearsal::scratch_base()
        .join("tests.nosync")
        .join(format!("cm3-{name}-{}", std::process::id()))
        .join("pack");
    std::fs::create_dir_all(p.join("hooks")).unwrap();
    p
}

#[test]
fn production_check_finds_a_rel_with_a_folder() {
    let p = pack("cm3");
    std::fs::write(
        p.join("hooks/x.sh"),
        "#!/bin/sh\nif true; then echo ok; fi\n",
    )
    .unwrap();
    let r = super::production_check(
        "hooks/x.sh",
        &p.join("hooks/x.sh"),
        std::time::Duration::from_secs(30),
    )
    .expect("the runner answers");
    assert!(
        r.passed,
        "a valid hooks/x.sh failed its check: {}",
        r.failure
    );
    std::fs::write(p.join("hooks/x.sh"), "#!/bin/sh\nif true; then echo ok\n").unwrap();
    let r = super::production_check(
        "hooks/x.sh",
        &p.join("hooks/x.sh"),
        std::time::Duration::from_secs(30),
    )
    .expect("the runner answers");
    assert!(!r.passed, "a broken hooks/x.sh passed its check");
    let _ = std::fs::remove_dir_all(p.parent().unwrap());
}

/// RULINGS CZ3 / DE1: the worker's `check` runs inside the scoped after-run comparison too — a hook
/// whose replay writes another live listed file of the pack on every run fails the check with the
/// comparison's sentence.
#[test]
fn production_check_finds_a_check_writing_the_live_pack() {
    let p = pack("cz3");
    let live = p.join("hooks/guard.sh");
    std::fs::write(&live, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::write(
        p.join("hooks/appbuild-gate.sh"),
        format!("#!/bin/sh\nprintf 'x\\n' >> '{}'\nexit 0\n", live.display()),
    )
    .unwrap();
    let r = super::production_check(
        "hooks/appbuild-gate.sh",
        &p.join("hooks/appbuild-gate.sh"),
        std::time::Duration::from_secs(30),
    )
    .expect("the runner answers");
    assert!(!r.passed);
    assert!(r.failure.contains("다시 썼습니다"), "{}", r.failure);
}

/// RULINGS GR6 — through the production wrapper: a one-off finding (the check's replay writes a live
/// listed file in its first run only — the repeat does not write it again → "noted") whose intake row
/// could not be written is NOT passed; the answer says the finding could not be recorded and carries
/// the finding. The same finding with its row written (new, or the existing row of its signature)
/// passes. (The old wrapper discarded the intake's answer and answered "passed".)
#[test]
fn a_finding_that_could_not_be_recorded_is_not_passed() {
    use super::Filed;
    let one_off = |name: &str| {
        let p = pack(name);
        let live = p.join("hooks/guard.sh");
        std::fs::write(&live, "#!/bin/sh\nexit 0\n").unwrap();
        let marker = p.parent().unwrap().join("wrote-once");
        std::fs::write(
            p.join("hooks/appbuild-gate.sh"),
            format!(
                "#!/bin/sh\nif [ ! -f '{m}' ]; then : > '{m}'; printf 'x\\n' >> '{l}'; fi\nexit 0\n",
                m = marker.display(),
                l = live.display()
            ),
        )
        .unwrap();
        p
    };
    let check = |p: &std::path::Path, answer: Filed| {
        super::production_check_with(
            "hooks/appbuild-gate.sh",
            &p.join("hooks/appbuild-gate.sh"),
            std::time::Duration::from_secs(30),
            &|_| Some(answer.clone()),
        )
        .expect("the runner answers")
    };
    let p = one_off("gr6-failed");
    let r = check(&p, Filed::Failed("injected: the ledger cannot be written".into()));
    assert!(!r.passed, "a finding that could not be recorded passed");
    assert!(r.failure.contains(super::UNRECORDED_FINDING), "{}", r.failure);
    assert!(r.failure.contains("바뀌었으나"), "the finding travels in the answer: {}", r.failure);
    let _ = std::fs::remove_dir_all(p.parent().unwrap());
    for (name, answer) in [("gr6-new", Filed::New), ("gr6-exists", Filed::Exists)] {
        let p = one_off(name);
        let r = check(&p, answer);
        assert!(r.passed, "{name}: a recorded finding refused: {}", r.failure);
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }
}
