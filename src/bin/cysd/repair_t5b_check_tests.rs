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
