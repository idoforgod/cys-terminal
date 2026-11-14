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
