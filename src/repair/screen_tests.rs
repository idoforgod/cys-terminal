//! Tests of the protection screen (T5b). Every run is hermetic: the screen lays out its own scratch
//! roots under this test's private folder, calls only its own stub `cys`, binds only its own scratch
//! socket, and the "real" HOME / lane it compares are folders of this test too.

use super::*;
use crate::repair::replay;
use crate::repair::screen_corpus::{self as corpus, Device, CELLS, CENSUS_PINS};
use std::sync::Arc;

/// A private test root inside the replay's own scratch base (RULINGS DE6: no test writes outside its
/// scratch directory; never under a temporary folder — the registrar refuses a pack there; a
/// `.nosync` folder, so no file provider re-stamps what the after-run comparison watches).
fn test_root(name: &str) -> PathBuf {
    let p = crate::repair::rehearsal::scratch_base()
        .join("tests.nosync")
        .join(format!("screen-{name}-{}", replay::random_hex(4)));
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn released(rel: &str) -> String {
    replay::released_text(rel)
        .unwrap_or_else(|| panic!("{rel} is not embedded"))
        .to_string()
}

/// A lane laid out with the RELEASED copies of every input file (+ `state/` folder).
fn lane_of(root: &Path, overrides: &[(&str, Option<String>)]) -> PathBuf {
    let lane = root.join("lane").join("pack");
    for rel in all_input_rels().unwrap() {
        if let Some(c) = super::super::rehearsal::released_copy(&rel) {
            replay::lay_out(&lane.join(&rel), &c).unwrap();
        }
    }
    for (rel, c) in overrides {
        let p = lane.join(rel);
        let _ = std::fs::remove_file(&p);
        if let Some(text) = c {
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, text).unwrap();
        }
    }
    lane
}

/// A candidate whose targets are given as (rel, Some(text) | None = deleted).
fn candidate_of(root: &Path, targets: &[(&str, Option<String>)]) -> ScreenCandidate {
    let files = root.join("staging");
    let mut t = Vec::new();
    for (rel, c) in targets {
        let p = files.join(rel);
        match c {
            Some(text) => {
                std::fs::create_dir_all(p.parent().unwrap()).unwrap();
                std::fs::write(&p, text).unwrap();
                replay::set_mode(&p, 0o755);
                t.push((rel.to_string(), TargetState::read(&p).unwrap()));
            }
            None => t.push((rel.to_string(), TargetState::Absent)),
        }
    }
    ScreenCandidate { files, targets: t }
}

fn cfg(
    root: &Path,
    classes: Option<Vec<&'static str>>,
    cells: Option<Vec<&'static str>>,
    i3: bool,
) -> ScreenConfig {
    let mut c = ScreenConfig::production();
    c.scratch_base = root.join("scr");
    c.real_home = root.join("realhome");
    std::fs::create_dir_all(&c.real_home).unwrap();
    c.classes = classes;
    c.cells = cells;
    c.run_i3 = i3;
    assert!(
        c.interpreter.is_some(),
        "no python3 on PATH — the screen cannot be measured in this environment"
    );
    c
}

const LONG: Duration = Duration::from_secs(900);

fn decisions<'a>(r: &'a ScreenReport, class: &str) -> Vec<&'a ProbeResult> {
    r.probes
        .iter()
        .filter(|p| p.class == class || p.class.starts_with(&format!("{class}#")))
        .collect()
}

fn cleanup(root: &Path) {
    replay::remove_tree(root).expect("the test root is removed");
}

/// RULINGS DE6: no T5b test writes outside its scratch directory — every test folder and file lies in
/// the replay's own scratch base (a plain `cargo test` must not write into the real HOME).
#[test]
fn t5b_tests_write_only_inside_their_scratch_base() {
    let home_call = ["dirs::", "home_dir()"].concat();
    for (name, src) in [
        ("screen_tests.rs", include_str!("screen_tests.rs")),
        ("rehearsal_tests.rs", include_str!("rehearsal_tests.rs")),
        ("repair_approval_tests.rs", include_str!("../bin/cysd/repair_approval_tests.rs")),
        ("repair_t5b_check_tests.rs", include_str!("../bin/cysd/repair_t5b_check_tests.rs")),
    ] {
        assert!(!src.contains(&home_call), "{name} builds a path from HOME");
        assert!(src.contains("tests.nosync"), "{name} has no folder in the scratch base");
    }
}

/// RULINGS EL4 — a census of the WAIT sites of this unit: no process wait blocks. Every `waitpid` /
/// `waitid` asks for WNOHANG; every `.wait()` is the body of a detached reaper thread (a child that
/// would not die is never waited for by the caller); no `wait_with_output`.
#[test]
fn wait_site_census_no_blocking_wait() {
    let files = [
        ("replay.rs", include_str!("replay.rs")),
        ("rehearsal.rs", include_str!("rehearsal.rs")),
        ("screen.rs", include_str!("screen.rs")),
        ("repair_approval.rs", include_str!("../bin/cysd/repair_approval.rs")),
    ];
    let mut bad = Vec::new();
    for (name, src) in files {
        let lines: Vec<&str> = src.lines().collect();
        for (i, l) in lines.iter().enumerate() {
            let code = l.split("//").next().unwrap_or("");
            if code.contains(concat!("wait_with", "_output(")) {
                bad.push(format!("{name}:{} wait_with_output", i + 1));
            }
            if code.contains("waitpid(") || code.contains("waitid(") {
                let window = lines[i..(i + 6).min(lines.len())].join(" ");
                if !window.contains("WNOHANG") {
                    bad.push(format!("{name}:{} wait without WNOHANG", i + 1));
                }
            }
            if code.contains(".wait()") {
                let before = lines[i.saturating_sub(2)..i].join(" ");
                if !before.contains("spawn(move ||") {
                    bad.push(format!("{name}:{} a blocking .wait() outside a detached reaper", i + 1));
                }
            }
        }
    }
    assert!(bad.is_empty(), "blocking wait sites: {bad:?}");
}

/// RULINGS EL3 / EO4 — a census of the READ sites of the screen / replay modules: a file a candidate
/// could have produced is read only through the bounded reader; the only other opens are the bounded
/// reader itself, `/dev/urandom` and Linux's `/proc` files.
#[test]
fn read_site_census_only_the_bounded_reader() {
    let allowed: &[&str] = &[
        "std::fs::File::open(\"/dev/urandom\")",
        "std::fs::read_to_string(format!(\"/proc/{pid}/stat\"))",
        "std::fs::read(e.path().join(\"environ\"))",
        "std::fs::OpenOptions::new()",
        "    std::fs::File::open(p)",
    ];
    let files = [
        ("screen.rs", include_str!("screen.rs")),
        ("replay.rs", include_str!("replay.rs")),
        ("rehearsal.rs", include_str!("rehearsal.rs")),
        ("screen_corpus.rs", include_str!("screen_corpus.rs")),
    ];
    let mut bad = Vec::new();
    for (name, src) in files {
        for (i, l) in src.lines().enumerate() {
            let code = l.split("//").next().unwrap_or("");
            let opens = ["fs::read(", "read_to_string(", "File::open(", "OpenOptions::new("];
            if opens.iter().any(|o| code.contains(o)) && !allowed.iter().any(|a| l.contains(a)) {
                bad.push(format!("{name}:{}: {}", i + 1, l.trim()));
            }
        }
    }
    assert!(bad.is_empty(), "unbounded read sites: {bad:#?}");
}

/// RULINGS EL3: a candidate registrar that leaves a writer-less FIFO where `settings.json` was cannot
/// hang the screen — the read refuses at once and the I3 rows are not measurable (→ refused).
#[test]
fn registrar_leaving_a_fifo_cannot_hang_the_screen() {
    let root = test_root("el3-fifo");
    let lane = lane_of(&root, &[]);
    let pf = released("bin/javis_preflight.py");
    let at = "        cmd = _cys_hook_cmd(script_name)\n        refused = []\n";
    assert_eq!(pf.matches(at).count(), 1);
    let fifo = "        try:\n            os.remove(settings_path)\n        except OSError:\n            pass\n        os.mkfifo(settings_path)\n        return None\n";
    let cand = candidate_of(
        &root,
        &[("bin/javis_preflight.py", Some(pf.replacen(at, &format!("{fifo}{at}"), 1)))],
    );
    let c = cfg(&root, Some(vec![]), Some(vec![]), true);
    let t0 = std::time::Instant::now();
    let rep = screen_with(&c, &cand, &lane, LONG).unwrap();
    assert!(t0.elapsed() < LONG, "{:?}", t0.elapsed());
    assert_eq!(rep.verdict, ScreenVerdict::Refused, "{:?}", rep.refusals);
    cleanup(&root);
}

// ───────────────────────────── parity, census (i) and (iii) ─────────────────────────────

/// RULINGS AU1 / BM5: the screen's inputs ARE the generated per-test inputs — exactly G, R, C, I3,
/// each file list EQUAL to `boot_critical::protection_inputs()`; every variant lays out exactly the
/// union of those files.
#[test]
fn parity_inputs_equal_generated_inputs() {
    let gen = crate::repair::boot_critical::protection_inputs();
    let ids: Vec<&str> = gen.iter().map(|(t, _)| *t).collect();
    assert_eq!(
        ids,
        vec!["C", "G", "I3", "R"],
        "generated tests are not exactly G, R, C, I3"
    );
    let mine = inputs_by_test().unwrap();
    for (t, files) in gen {
        let want: Vec<String> = files.iter().map(|f| f.to_string()).collect();
        assert_eq!(
            mine.get(t),
            Some(&want),
            "test {t}: the screen's inputs differ from the generated ones"
        );
    }
    let root = test_root("parity");
    let lane = lane_of(&root, &[]);
    let c = cfg(&root, Some(vec![]), Some(vec![]), false);
    let rels = all_input_rels().unwrap();
    let v = build_variants(&c, &candidate_of(&root, &[]), &lane, &rels).unwrap();
    for f in [&v.released, &v.lane, &v.candidate, &v.clean, &v.attribution] {
        assert_eq!(
            f.keys().cloned().collect::<Vec<_>>(),
            rels,
            "a variant lays out other files"
        );
    }
    // the device subsets are exactly the generated lists
    assert_eq!(
        subset(&v.released, &mine["G"])
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        mine["G"]
    );
    cleanup(&root);
}

/// C4 AY2 census (i): every tabled class has a probe in every tabled cell its device reads — the
/// guard 16 cells (E-21 for the I4 classes), the gate 14.
#[test]
fn census_i_every_class_has_a_probe_in_every_cell() {
    assert_eq!(CELLS.len(), 21);
    let ids: Vec<&str> = CELLS.iter().map(|c| c.id).collect();
    for n in 1..=21 {
        assert!(
            ids.contains(&format!("E-{n}").as_str()),
            "cell E-{n} missing"
        );
    }
    assert_eq!(corpus::cells_of(Device::Guard).count(), 16);
    assert_eq!(corpus::cells_of(Device::Gate).count(), 14);
    let g: Vec<&str> = corpus::CORPUS_G.iter().map(|c| c.id).collect();
    let mut want_g: Vec<String> = (1..=21).map(|n| format!("G-{n}")).collect();
    want_g.push("G-24".into());
    assert_eq!(g, want_g.iter().map(String::as_str).collect::<Vec<_>>());
    let r: Vec<&str> = corpus::CORPUS_R.iter().map(|c| c.id).collect();
    assert_eq!(
        r,
        (1..=16)
            .map(|n| format!("R-{n}"))
            .collect::<Vec<_>>()
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
    );
    let cc: Vec<&str> = corpus::CORPUS_C.iter().map(|c| c.id).collect();
    assert_eq!(
        cc,
        (1..=25)
            .map(|n| format!("C-{n}"))
            .collect::<Vec<_>>()
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
    );
    let bs: Vec<&str> = corpus::BACKSTOP_CLASSES.iter().map(|b| b.0).collect();
    assert_eq!(bs, vec!["G-22", "G-23", "G-24np"]);
    for c in corpus::all_classes() {
        assert!(!c.probes.is_empty(), "{} has no probe", c.id);
        // every probe of a class runs in every cell its device reads: the matrix is the product
        let cells = corpus::cells_of(c.device)
            .filter(|x| x.setup != corpus::Setup::Backstop)
            .count();
        assert!(cells >= 14, "{} would be probed in {cells} cells", c.id);
    }
}

/// C4 AY2 census (iii) — THE test the pack-release gate (unit T2d) runs (RULINGS CM4): a change of a
/// released device's deny rules or environment branches without a table review fails here.
#[test]
fn census_iii_deny_rules_and_branches_pinned() {
    let frees_of = |rel: &str| -> Vec<(&'static str, &'static str)> {
        CENSUS_PINS
            .iter()
            .filter(|p| p.0 == rel && p.1 == "free")
            .map(|p| (p.2, p.3))
            .collect()
    };
    let mut actual = Vec::new();
    let mut differs = Vec::new();
    for (rel, kind, _, _, n, digest) in CENSUS_PINS {
        if *kind != "pin" {
            assert_eq!(*kind, "free", "{rel}: a row is a pin or a free region");
            continue;
        }
        match corpus::census_pinned(&released(rel), &frees_of(rel)) {
            Ok((an, ad)) => {
                actual.push(format!("({rel:?}, {an}, {ad:?})"));
                assert!(an > 100, "{rel}: only {an} pinned lines");
                if (an, ad.as_str()) != (*n, *digest) {
                    differs.push(format!("{rel} ({n} → {an} lines)"));
                }
            }
            Err(e) => differs.push(format!("{rel}: {e}")),
        }
    }
    assert!(
        differs.is_empty(),
        "census (iii): a pinned protection file changed in {}. Review CORPUS_G / CORPUS_R / CORPUS_C \
         and the cell table against the device, then update CENSUS_PINS. Current values: {}",
        differs.join(", "),
        actual.join(", ")
    );
    // every pinned device has its pin row; every free row's device has one too
    for rel in ["hooks/guard.sh", "hooks/role-capability-gate.sh", "hooks/_lib.sh"] {
        assert!(CENSUS_PINS.iter().any(|p| p.0 == rel && p.1 == "pin"), "{rel} unpinned");
    }
    // RULINGS EL6 / EO3 / CZ7 — mutation controls both ways
    let digest = |rel: &str, text: &str| corpus::census_pinned(text, &frees_of(rel));
    let cases: [(&str, &str, &str, bool); 11] = [
        // the gate's dispatch: main() replaced by pass — outside every named region before EL6
        ("hooks/role-capability-gate.sh", "    main()\nPYEOF", "    pass\nPYEOF", true),
        // the CAPGATE_SELF_TEST branch
        (
            "hooks/role-capability-gate.sh",
            "if os.environ.get(\"CAPGATE_SELF_TEST\"):",
            "if os.environ.get(\"CAPGATE_SELF_TESTS\"):",
            true,
        ),
        // the prelude sourcing of both devices (EO3: the fallback and the `exit 0` when missing)
        (
            "hooks/guard.sh",
            "|| . \"${CYS_PACK_DIR:-$HOME/.cys/pack}/hooks/_lib.sh\" 2>/dev/null",
            "|| . \"${CYS_PACK_DIR:-$HOME/.cys/elsewhere}/hooks/_lib.sh\" 2>/dev/null",
            true,
        ),
        (
            "hooks/guard.sh",
            "|| { echo \"[cys-hook] _lib.sh 소실 — 훅 강등(guard)\" >&2; exit 0; }",
            "|| { echo \"[cys-hook] _lib.sh 소실 — 훅 강등(guard)\" >&2; exit 2; }",
            true,
        ),
        (
            "hooks/role-capability-gate.sh",
            "|| { echo \"[cys-hook] _lib.sh 소실 — 훅 강등(role-capability-gate)\" >&2; exit 0; }",
            "|| { echo \"[cys-hook] _lib.sh 소실 — 훅 강등(role-capability-gate)\" >&2; exit 1; }",
            true,
        ),
        // the rules (CZ7's controls stay)
        (
            "hooks/guard.sh",
            "x in args for x in (\"create\", \"merge\")",
            "x in args for x in (\"create\",)",
            true,
        ),
        (
            "hooks/role-capability-gate.sh",
            "{\"commit\", \"push\", \"add\",",
            "{\"commit\", \"add\",",
            true,
        ),
        (
            "hooks/_lib.sh",
            "_cys_sh_d=\"${CYS_DEVTOOLS_SHIM_DIR:-/usr/bin}\"",
            "_cys_sh_d=\"${CYS_DEVTOOLS_SHIM_DIR:-/usr/local/bin}\"",
            true,
        ),
        // default = pinned: a header comment is pinned too
        (
            "hooks/guard.sh",
            "# guard.sh — Autopilot 집행 hook",
            "# guard.sh — Autopilot 집행 훅",
            true,
        ),
        (
            "hooks/_lib.sh",
            "# _lib.sh — cysjavis 훅 공용 프리루드",
            "# _lib.sh — cysjavis 훅 공용 머리말",
            true,
        ),
        // the only free region: the gate's self-test body
        (
            "hooks/role-capability-gate.sh",
            "# ── ① reviewer/planner(종전 핀 · 무변경)",
            "# ── ① reviewer/planner(종전 핀)",
            false,
        ),
    ];
    for (rel, from, to, changes) in cases {
        let t = released(rel);
        assert_eq!(t.matches(from).count(), 1, "{rel}: mutation anchor {from:?}");
        let m = t.replacen(from, to, 1);
        assert_eq!(digest(rel, &t) != digest(rel, &m), changes, "{rel}: {from:?} → {to:?}");
    }
    // a doubled free-region anchor is an error, never a guess
    let g = released("hooks/role-capability-gate.sh");
    let doubled = g.replacen("def self_test():", "def self_test():\ndef self_test():", 1);
    assert!(corpus::census_pinned(&doubled, &frees_of("hooks/role-capability-gate.sh")).is_err());
}

/// RULINGS DF5: an I4 anchor that occurs twice gives no region (never the first copy's).
#[test]
fn i4_anchors_must_occur_exactly_once() {
    let g = released("hooks/guard.sh");
    for (name, first, last) in corpus::I4_REGIONS {
        assert!(corpus::i4_region(&g, first, last).is_some(), "{name}");
        let doubled = g.replacen(first, &format!("{first}\n{first}"), 1);
        assert!(corpus::i4_region(&doubled, first, last).is_none(), "{name}: doubled start");
        if *last != "}" {
            let doubled = g.replacen(last, &format!("{last}\n{last}"), 1);
            assert!(corpus::i4_region(&doubled, first, last).is_none(), "{name}: doubled end");
        }
    }
}

/// I4's anchors are found in the released guard.sh, and each region is non-trivial.
#[test]
fn i4_anchors_found_in_released_guard() {
    let g = released("hooks/guard.sh");
    for (name, first, last) in corpus::I4_REGIONS {
        let r = corpus::i4_region(&g, first, last)
            .unwrap_or_else(|| panic!("I4 region {name}: anchor not found"));
        assert!(
            r.lines().count() >= 10,
            "I4 region {name} is only {} lines",
            r.lines().count()
        );
    }
    let b = corpus::i4_region(&g, "run_loose_backstop() {", "}").unwrap();
    assert!(b.contains("백스톱: 비가역 명령") && b.contains("백스톱: 파괴적 파일연산"));
    assert!(b.contains(corpus::BACKSTOP_MARKER));
}

// ───────────────────────────── census (ii) — every class binds on the released copies ─────────────────────────────

/// C4 AY2 census (ii): on the RELEASED copies every class BINDS (RELEASED denies) in at least one
/// cell; no RELEASED probe errors (an error there would decline every apply — AY5); the stub was
/// reached in E-1 (its call log); invariant I3 on RELEASED ends registered in every layout, scenario
/// and run (both runs judged — BH5). The screen's duration is measured here.
#[test]
fn census_ii_every_class_binds_on_released() {
    let root = test_root("census2");
    let lane = lane_of(&root, &[]);
    let c = cfg(&root, None, None, true);
    let t0 = std::time::Instant::now();
    let rep = screen_with(&c, &candidate_of(&root, &[]), &lane, LONG).unwrap();
    let took = t0.elapsed();
    eprintln!(
        "T5b MEASURED: full screen on the released copies (no candidate change): {} probes, {:.1} s wall, {} threads",
        rep.probes.len(),
        took.as_secs_f64(),
        c.threads
    );
    // the same screen again in this process: every answer comes from the per-process cache (the same
    // input bytes) — what a later screen in a running daemon costs for devices a candidate does not
    // touch; the verdict and every decision are identical
    let t1 = std::time::Instant::now();
    let again = screen_with(&c, &candidate_of(&root, &[]), &lane, LONG).unwrap();
    eprintln!(
        "T5b MEASURED: the same screen again (answers cached in this process): {:.1} s wall",
        t1.elapsed().as_secs_f64()
    );
    // the measurement is also kept in a file (a passing test's standard error is not shown by the
    // runner) — RULINGS DE6: only inside the cargo target folder the run names (a build scratch
    // folder), never in HOME; without one nothing is written
    if let Some(dir) = std::env::var_os("CARGO_TARGET_DIR").map(PathBuf::from) {
        let _ = std::fs::write(
            dir.join("t5b-screen-measured.txt"),
        format!(
            "full screen, released copies, no candidate change: {} probes, {:.1} s wall, {} threads, after-run repeat: {}\nsame screen again (answers cached in this process): {:.1} s wall\n",
            rep.probes.len(),
            took.as_secs_f64(),
            c.threads,
            !rep.after_run.first.is_empty(),
            t1.elapsed().as_secs_f64()
        ),
        );
    }
    assert_eq!(again.verdict, rep.verdict);
    assert_eq!(
        again
            .probes
            .iter()
            .map(|p| (&p.class, &p.cell, p.released, p.decision))
            .collect::<Vec<_>>(),
        rep.probes
            .iter()
            .map(|p| (&p.class, &p.cell, p.released, p.decision))
            .collect::<Vec<_>>()
    );
    let errors: Vec<String> = rep
        .probes
        .iter()
        .filter(|p| p.released == Answer::Error && p.test != "I4")
        .map(|p| format!("{} {} {} — {}", p.test, p.class, p.cell, p.detail))
        .collect();
    assert!(
        errors.is_empty(),
        "RELEASED errored ({}): {:#?}",
        errors.len(),
        &errors[..errors.len().min(20)]
    );
    for class in corpus::all_classes() {
        let binds = decisions(&rep, class.id)
            .iter()
            .any(|p| p.released == Answer::Deny);
        assert!(
            binds,
            "census (ii): {} binds in no cell on the released copies",
            class.id
        );
    }
    // the stub was reached: guard LOOSE asks `approval check`, the gate asks `surface-role`
    let g1 = rep
        .probes
        .iter()
        .find(|p| p.class == "G-1" && p.cell == "E-1")
        .unwrap();
    assert!(
        g1.detail.contains("cys --socket") && g1.detail.contains("approval check"),
        "G-1 E-1: {}",
        g1.detail
    );
    let r1 = rep
        .probes
        .iter()
        .find(|p| p.class == "R-1" && p.cell == "E-1")
        .unwrap();
    assert!(r1.detail.contains("surface-role"), "R-1 E-1: {}", r1.detail);
    // I3 on the released copies: registered in every layout, scenario and both runs
    let i3: Vec<&ProbeResult> = rep
        .probes
        .iter()
        .filter(|p| p.test == "I3" && p.class.starts_with("I3:"))
        .collect();
    assert!(
        i3.len() >= 12,
        "I3 judged only {} (layout, scenario, run, target)s",
        i3.len()
    );
    // the profiles the RELEASED hook-target table denies (`eligibility.capgate: deny`) end unregistered
    // there and bind nothing; `.claude`, the account folder (L1) and the department account (L2) end
    // registered in every scenario and BOTH runs (BH5)
    for l in ["L1", "L2"] {
        for s in ["S1", "S2", "S3"] {
            for k in ["run1", "run2"] {
                let cell = format!("{l}/{s}/{k}");
                let here: Vec<&&ProbeResult> = i3.iter().filter(|p| p.cell == cell).collect();
                assert!(!here.is_empty(), "I3 {cell} not judged");
                let want: &[&str] = if l == "L1" {
                    &["I3:.claude", "I3:claude"]
                } else {
                    &["I3:acct-dept-x"]
                };
                for w in want {
                    let p = here
                        .iter()
                        .find(|p| p.class == *w)
                        .unwrap_or_else(|| panic!("I3 {cell}: {w} not judged"));
                    assert_eq!(
                        p.released,
                        Answer::Deny,
                        "I3 {w} {cell}: released does not end registered — {}",
                        p.detail
                    );
                }
            }
        }
    }
    assert_eq!(
        rep.verdict,
        ScreenVerdict::Accepted,
        "notes: {:?} refusals: {:?}",
        rep.notes,
        rep.refusals
    );
    assert!(
        rep.after_run.first.is_empty(),
        "the screen changed the lane: {:?}",
        rep.after_run.first
    );
    assert!(rep.after_run.leftover_processes.is_empty());
    cleanup(&root);
}

// ───────────────────────────── the differential rule (AY1, BD1, BC5, BG1) ─────────────────────────────

fn guard_without_push_deny() -> String {
    let g = released("hooks/guard.sh");
    let line =
        "            if sub == \"push\": return \"git push (외부발행=비가역). 오너 승인 필요\"";
    assert!(
        g.contains(line),
        "guard.sh no longer holds the git push LOOSE deny line"
    );
    g.replace(line, "            if sub == \"push\": return None")
}

/// A candidate that lets `git push` through in normal mode is REFUSED (G-1), naming the protection.
#[test]
fn refuses_candidate_that_opens_git_push() {
    let root = test_root("g1");
    let lane = lane_of(&root, &[]);
    let c = cfg(
        &root,
        Some(vec!["G-1", "G-17"]),
        Some(vec!["E-4", "E-5"]),
        false,
    );
    let cand = candidate_of(
        &root,
        &[("hooks/guard.sh", Some(guard_without_push_deny()))],
    );
    let rep = screen_with(&c, &cand, &lane, LONG).unwrap();
    assert_eq!(rep.verdict, ScreenVerdict::Refused, "{:#?}", rep.probes);
    let g1 = rep
        .probes
        .iter()
        .find(|p| p.class == "G-1" && p.cell == "E-4")
        .unwrap();
    assert_eq!(g1.decision, ProbeDecision::Refused);
    assert_eq!(g1.clean, Some(Answer::Allow));
    assert!(
        rep.refusals
            .iter()
            .any(|r| r.contains("외부 발송·영구 삭제 전 주인 승인")),
        "{:?}",
        rep.refusals
    );
    // STRICT still denies git push (G-17 held)
    let g17 = rep
        .probes
        .iter()
        .find(|p| p.class == "G-17" && p.cell == "E-5")
        .unwrap();
    assert_eq!(g17.decision, ProbeDecision::Held);
    assert!(rep.declines_apply());
    cleanup(&root);
}

/// A candidate DELETING hooks/guard.sh is screened as deleted and refused (RULINGS CM1).
#[test]
fn refuses_candidate_deleting_guard() {
    let root = test_root("gdel");
    let lane = lane_of(&root, &[]);
    let c = cfg(&root, Some(vec!["G-1"]), Some(vec!["E-4"]), false);
    let cand = candidate_of(&root, &[("hooks/guard.sh", None)]);
    let rep = screen_with(&c, &cand, &lane, LONG).unwrap();
    assert_eq!(rep.verdict, ScreenVerdict::Refused, "{:#?}", rep.probes);
    let i4: Vec<&ProbeResult> = rep.probes.iter().filter(|p| p.test == "I4").collect();
    assert!(
        i4.iter().all(|p| p.decision == ProbeDecision::Refused),
        "I4 must refuse a deleted guard.sh: {i4:#?}"
    );
    cleanup(&root);
}

/// A candidate capability gate that no longer treats Write as a mutation is REFUSED for the
/// reviewer (R-1) and the CSO (C-4).
#[test]
fn refuses_gate_opened_for_reviewer_and_cso() {
    let root = test_root("gate");
    let lane = lane_of(&root, &[]);
    let gate = released("hooks/role-capability-gate.sh");
    let line = "MUTATION_TOOLS = {\"Edit\", \"Write\", \"NotebookEdit\", \"MultiEdit\"}";
    assert!(gate.contains(line));
    let cand = candidate_of(
        &root,
        &[(
            "hooks/role-capability-gate.sh",
            Some(gate.replace(line, "MUTATION_TOOLS = set()")),
        )],
    );
    let c = cfg(
        &root,
        Some(vec!["R-1", "C-4", "R-8"]),
        Some(vec!["E-1"]),
        false,
    );
    let rep = screen_with(&c, &cand, &lane, LONG).unwrap();
    assert_eq!(rep.verdict, ScreenVerdict::Refused, "{:#?}", rep.probes);
    let r1 = rep.probes.iter().find(|p| p.class == "R-1").unwrap();
    assert_eq!(r1.decision, ProbeDecision::Refused, "{r1:#?}");
    assert!(
        rep.refusals
            .iter()
            .any(|r| r.contains("리뷰어는 검토 대상을 못 고침")),
        "{:?}",
        rep.refusals
    );
    // a probe the change does not touch stays held (git commit by a reviewer)
    let r8 = rep.probes.iter().find(|p| p.class == "R-8").unwrap();
    assert_eq!(r8.decision, ProbeDecision::Held, "{r8:#?}");
    cleanup(&root);
}

/// BD1: a lane whose guard.sh already lets git push through cannot hide a candidate that keeps it
/// open — CLEAN (RELEASED + the candidate's files) does not deny → REFUSED.
#[test]
fn clean_refuses_even_when_lane_already_off() {
    let root = test_root("bd1");
    let open = guard_without_push_deny();
    let lane = lane_of(&root, &[("hooks/guard.sh", Some(open.clone()))]);
    let c = cfg(&root, Some(vec!["G-1"]), Some(vec!["E-4"]), false);
    let cand = candidate_of(&root, &[("hooks/guard.sh", Some(open))]);
    let rep = screen_with(&c, &cand, &lane, LONG).unwrap();
    let g1 = rep.probes.iter().find(|p| p.class == "G-1").unwrap();
    assert_eq!(
        (g1.lane, g1.candidate, g1.clean),
        (
            Some(Answer::Allow),
            Some(Answer::Allow),
            Some(Answer::Allow)
        )
    );
    assert_eq!(g1.decision, ProbeDecision::Refused);
    assert_eq!(rep.verdict, ScreenVerdict::Refused);
    cleanup(&root);
}

/// BC5: a lane whose guard.sh lets git push through, and a candidate that touches no guard input →
/// the attribution run (LANE with the touched files at their released copies) still does not deny →
/// a LANE FINDING (note + intake line), not a refusal of the unrelated candidate.
#[test]
fn attribution_gives_lane_finding_not_refusal() {
    let root = test_root("bc5");
    let lane = lane_of(
        &root,
        &[("hooks/guard.sh", Some(guard_without_push_deny()))],
    );
    let c = cfg(&root, Some(vec!["G-1"]), Some(vec!["E-4"]), false);
    let cand = candidate_of(
        &root,
        &[("bin/javis_lock.py", Some(released("bin/javis_lock.py")))],
    );
    let rep = screen_with(&c, &cand, &lane, LONG).unwrap();
    let g1 = rep.probes.iter().find(|p| p.class == "G-1").unwrap();
    assert_eq!(g1.decision, ProbeDecision::LaneFinding, "{g1:#?}");
    assert_eq!(rep.verdict, ScreenVerdict::AcceptedWithNotes);
    assert!(
        rep.notes.iter().any(|n| n.starts_with("보호장치 시험 1건")),
        "{:?}",
        rep.notes
    );
    assert_eq!(rep.lane_findings.len(), 1);
    assert!(!rep.declines_apply());
    cleanup(&root);
}

/// BG1: restoring guard.sh to its RELEASED bytes in a lane whose _lib.sh is broken is never a ground
/// for refusal (it becomes a lane finding); the same restore with one byte changed IS refused.
#[test]
fn released_restore_never_refused() {
    let root = test_root("bg1");
    let g = released("hooks/guard.sh");
    let src_line = ". \"$(dirname \"$0\")/_lib.sh\" 2>/dev/null \\";
    assert!(g.contains(src_line));
    // the lane: a guard that does not source _lib.sh (still denies — through its backstop) and a
    // broken _lib.sh that ends any hook sourcing it
    let lane_guard = g.replace(src_line, "true \\").replace(
        "  || . \"${CYS_PACK_DIR:-$HOME/.cys/pack}/hooks/_lib.sh\" 2>/dev/null \\",
        "  || true \\",
    );
    let lane = lane_of(
        &root,
        &[
            ("hooks/guard.sh", Some(lane_guard)),
            ("hooks/_lib.sh", Some("exit 0\n".into())),
        ],
    );
    let c = cfg(&root, Some(vec!["G-1"]), Some(vec!["E-4"]), false);
    let rep = screen_with(
        &c,
        &candidate_of(&root, &[("hooks/guard.sh", Some(g.clone()))]),
        &lane,
        LONG,
    )
    .unwrap();
    let g1 = rep.probes.iter().find(|p| p.class == "G-1").unwrap();
    assert_eq!(
        (g1.lane, g1.candidate),
        (Some(Answer::Deny), Some(Answer::Allow)),
        "{g1:#?}"
    );
    assert_eq!(g1.decision, ProbeDecision::LaneFinding, "BG1: {g1:#?}");
    assert_ne!(rep.verdict, ScreenVerdict::Refused);
    // control: the same restore with one extra comment byte is a candidate of its own → refused
    let root2 = test_root("bg1b");
    let lane2 = lane_of(&root2, &[]);
    let _ = lane2;
    let lane_guard2 = released("hooks/guard.sh")
        .replace(src_line, "true \\")
        .replace(
            "  || . \"${CYS_PACK_DIR:-$HOME/.cys/pack}/hooks/_lib.sh\" 2>/dev/null \\",
            "  || true \\",
        );
    let lane2 = lane_of(
        &root2,
        &[
            ("hooks/guard.sh", Some(lane_guard2)),
            ("hooks/_lib.sh", Some("exit 0\n".into())),
        ],
    );
    let rep2 = screen_with(
        &c,
        &candidate_of(&root2, &[("hooks/guard.sh", Some(format!("{g}#\n")))]),
        &lane2,
        LONG,
    )
    .unwrap();
    let g1b = rep2.probes.iter().find(|p| p.class == "G-1").unwrap();
    assert_eq!(g1b.decision, ProbeDecision::Refused, "{g1b:#?}");
    cleanup(&root);
    cleanup(&root2);
}

/// AY1 (5): RELEASED errors → NOT MEASURABLE: refused for a candidate touching an input of that
/// device, a note (and an apply decline) otherwise.
#[test]
fn not_measurable_refuses_only_touching_candidates() {
    let root = test_root("nm");
    let lane = lane_of(&root, &[]);
    let mut c = cfg(&root, Some(vec!["G-1"]), Some(vec!["E-4"]), false);
    // a released source whose guard.sh crashes (exit 3) — injected for this test only
    c.released = Arc::new(|rel: &str| {
        if rel == "hooks/guard.sh" {
            Some(LaidSource::Bytes {
                bytes: b"#!/bin/sh\nexit 3\n".to_vec(),
                mode: 0o755,
            })
        } else {
            super::super::rehearsal::released_copy(rel)
        }
    });
    let touching = candidate_of(
        &root,
        &[(
            "hooks/_lib.sh",
            Some(format!("{}\n# x\n", released("hooks/_lib.sh"))),
        )],
    );
    let rep = screen_with(&c, &touching, &lane, LONG).unwrap();
    let g1 = rep
        .probes
        .iter()
        .find(|p| p.class == "G-1" && p.cell == "E-4")
        .unwrap();
    assert_eq!(g1.released, Answer::Error);
    assert_eq!(g1.decision, ProbeDecision::Refused);
    let other = candidate_of(
        &root,
        &[(
            "bin/javis_lock.py",
            Some(format!("{}\n# x\n", released("bin/javis_lock.py"))),
        )],
    );
    let rep2 = screen_with(&c, &other, &lane, LONG).unwrap();
    let g1 = rep2
        .probes
        .iter()
        .find(|p| p.class == "G-1" && p.cell == "E-4")
        .unwrap();
    assert_eq!(g1.decision, ProbeDecision::NotMeasurable);
    assert_eq!(rep2.verdict, ScreenVerdict::AcceptedWithNotes);
    assert!(
        rep2.declines_apply(),
        "AY5: at apply a not-measurable probe declines"
    );
    cleanup(&root);
}

// ───────────────────────────── I3, I4, E-21, AT4 ─────────────────────────────

/// I3 (BC1): a candidate registrar whose condition is always "off" ends the gate unregistered where
/// the released one keeps it registered → REFUSED.
#[test]
fn i3_refuses_registrar_that_unregisters() {
    let root = test_root("i3");
    let lane = lane_of(&root, &[]);
    let pf = released("bin/javis_preflight.py");
    let line = "        state = capgate_registration_state(a, b)";
    assert!(pf.contains(line));
    let cand = candidate_of(
        &root,
        &[(
            "bin/javis_preflight.py",
            Some(pf.replace(line, "        state = CAPGATE_OFF")),
        )],
    );
    let c = cfg(&root, Some(vec![]), Some(vec![]), true);
    let rep = screen_with(&c, &cand, &lane, LONG).unwrap();
    assert_eq!(
        rep.verdict,
        ScreenVerdict::Refused,
        "{:#?}",
        rep.probes
            .iter()
            .filter(|p| p.test == "I3")
            .collect::<Vec<_>>()
    );
    assert!(
        rep.refusals.iter().any(|r| r.contains("관문의 등록")),
        "{:?}",
        rep.refusals
    );
    let s1 = rep
        .probes
        .iter()
        .find(|p| p.test == "I3" && p.cell == "L1/S1/run1" && p.class == "I3:.claude")
        .unwrap();
    assert_eq!(
        (s1.released, s1.clean),
        (Answer::Deny, Some(Answer::Allow)),
        "{s1:#?}"
    );
    cleanup(&root);
}

/// RULINGS CZ1: "registered" = the hook EXECUTES the gate. Each form is read under the closed grammar;
/// the old substring check (`command.contains(<gate path>)`) called every `false` row below registered.
#[test]
fn gate_registration_reads_the_effective_invocation() {
    let root = test_root("cz1-unit");
    let pack = root.join("pack");
    let gate = pack.join("hooks").join(CAPGATE_SCRIPT);
    std::fs::create_dir_all(gate.parent().unwrap()).unwrap();
    std::fs::write(&gate, "#!/bin/sh\nexit 0\n").unwrap();
    replay::set_mode(&gate, 0o644);
    let g = gate.to_string_lossy().into_owned();
    let hook = |cmd: &str| serde_json::json!({"type": "command", "command": cmd, "timeout": 15});
    let rows: Vec<(&str, serde_json::Value, bool)> = vec![
        ("the registrar's form", hook(&format!("sh {g}")), true),
        ("double-quoted path", hook(&format!("sh \"{g}\"")), true),
        ("single-quoted path", hook(&format!("sh '{g}'")), true),
        ("absolute interpreter", hook(&format!("/bin/sh {g}")), true),
        ("bash", hook(&format!("bash {g}")), true),
        ("no timeout (the tool's default)", serde_json::json!({"type": "command", "command": format!("sh {g}")}), true),
        ("comment", hook(&format!("true # sh {g}")), false),
        ("no-op wrapper", hook(&format!("true sh {g}")), false),
        ("colon wrapper", hook(&format!(": sh {g}")), false),
        ("syntax-check-only flag", hook(&format!("sh -n {g}")), false),
        ("conditional", hook(&format!("[ -n \"$CYS_NEVER\" ] && sh {g}")), false),
        ("or-true", hook(&format!("sh {g} || true")), false),
        ("sequence", hook(&format!("exit 0; sh {g}")), false),
        ("an argument", hook(&format!("sh {g} --self-test")), false),
        ("an assignment in front", hook(&format!("CAPGATE_SELF_TEST=1 sh {g}")), false),
        ("env wrapper", hook(&format!("env sh {g}")), false),
        ("redirection", hook(&format!("sh {g} 2>/dev/null")), false),
        ("expansion in quotes", hook(&format!("sh \"$HOME{g}\"")), false),
        ("another interpreter", hook(&format!("zsh {g}")), false),
        ("quoted interpreter", hook(&format!("'sh' {g}")), false),
        ("another path", hook(&format!("sh {}/hooks/../hooks/{CAPGATE_SCRIPT}", pack.display())), false),
        ("direct, not executable", hook(&g), false),
        ("incompatible type", serde_json::json!({"type": "prompt", "command": format!("sh {g}")}), false),
        ("no type", serde_json::json!({"command": format!("sh {g}")}), false),
        ("timeout below the floor", serde_json::json!({"type": "command", "command": format!("sh {g}"), "timeout": 1}), false),
        ("timeout not a number", serde_json::json!({"type": "command", "command": format!("sh {g}"), "timeout": "15"}), false),
        ("a key the registrar never writes", serde_json::json!({"type": "command", "command": format!("sh {g}"), "async": true}), false),
    ];
    let settings = root.join("settings.json");
    let write = |doc: serde_json::Value| std::fs::write(&settings, doc.to_string()).unwrap();
    let mut wrong = Vec::new();
    for (name, h, want) in &rows {
        write(serde_json::json!({"hooks": {"PreToolUse": [{"hooks": [h]}]}}));
        if gate_registered(&settings, &pack) != Some(*want) {
            wrong.push(*name);
        }
    }
    // the direct form runs only an executable file
    replay::set_mode(&gate, 0o755);
    write(serde_json::json!({"hooks": {"PreToolUse": [{"hooks": [hook(&g)]}]}}));
    if gate_registered(&settings, &pack) != Some(true) {
        wrong.push("direct, executable");
    }
    // entry-level forms: a narrower matcher, an unknown entry key, every hook disabled, the gate gone
    for (name, doc) in [
        ("matcher Bash", serde_json::json!({"hooks": {"PreToolUse": [{"matcher": "Bash", "hooks": [hook(&format!("sh {g}"))]}]}})),
        ("unknown entry key", serde_json::json!({"hooks": {"PreToolUse": [{"if": "never", "hooks": [hook(&format!("sh {g}"))]}]}})),
        ("disableAllHooks", serde_json::json!({"disableAllHooks": true, "hooks": {"PreToolUse": [{"hooks": [hook(&format!("sh {g}"))]}]}})),
        ("another event", serde_json::json!({"hooks": {"PostToolUse": [{"hooks": [hook(&format!("sh {g}"))]}]}})),
    ] {
        write(doc);
        if gate_registered(&settings, &pack) != Some(false) {
            wrong.push(name);
        }
    }
    std::fs::remove_file(&gate).unwrap();
    write(serde_json::json!({"hooks": {"PreToolUse": [{"hooks": [hook(&format!("sh {g}"))]}]}}));
    if gate_registered(&settings, &pack) != Some(false) {
        wrong.push("gate file missing");
    }
    assert!(wrong.is_empty(), "misjudged forms: {wrong:?}");
    cleanup(&root);
}

/// RULINGS CZ1 — a registrar-mutation regression per form: a candidate `javis_preflight.py` whose C28
/// registration leaves an entry that names the gate but cannot run it is REFUSED on the real
/// registrar's own run (L1, s3: starts unregistered), the released registrar registering it.
fn i3_refuses_registrar_mutation(name: &str, from: &str, to: &str) {
    let root = test_root(name);
    let lane = lane_of(&root, &[]);
    let pf = released("bin/javis_preflight.py");
    assert_eq!(pf.matches(from).count(), 1, "{name}: mutation anchor");
    let cand = candidate_of(
        &root,
        &[("bin/javis_preflight.py", Some(pf.replace(from, to)))],
    );
    let c = cfg(&root, Some(vec![]), Some(vec![]), true);
    let rep = screen_with(&c, &cand, &lane, LONG).unwrap();
    let i3: Vec<&ProbeResult> = rep.probes.iter().filter(|p| p.test == "I3").collect();
    assert_eq!(rep.verdict, ScreenVerdict::Refused, "{name}: {i3:#?}");
    let s3 = i3
        .iter()
        .find(|p| p.cell == "L1/S3/run1" && p.class == "I3:.claude")
        .unwrap_or_else(|| panic!("{name}: no L1/S3 probe: {i3:#?}"));
    // measured, not an error: the released registrar registers, the candidate's leaves it unrun
    assert_eq!(
        (s3.released, s3.clean, s3.decision),
        (Answer::Deny, Some(Answer::Allow), ProbeDecision::Refused),
        "{name}: {s3:#?}"
    );
    cleanup(&root);
}

const REGISTRAR_FORM: &str = "    return \"sh \" + script";

#[test]
fn i3_refuses_registrar_writing_a_comment() {
    i3_refuses_registrar_mutation("cz1-comment", REGISTRAR_FORM, "    return \"true # sh \" + script");
}

#[test]
fn i3_refuses_registrar_writing_a_noop_wrapper() {
    i3_refuses_registrar_mutation("cz1-wrapper", REGISTRAR_FORM, "    return \"true sh \" + script");
}

#[test]
fn i3_refuses_registrar_writing_a_conditional() {
    i3_refuses_registrar_mutation(
        "cz1-cond",
        REGISTRAR_FORM,
        "    return '[ -n \"$CYS_NEVER_SET\" ] && sh ' + script",
    );
}

#[test]
fn i3_refuses_registrar_writing_another_hook_type() {
    i3_refuses_registrar_mutation(
        "cz1-type",
        "entry = {\"hooks\": [{\"type\": \"command\", \"command\": cmd}]}",
        "entry = {\"hooks\": [{\"type\": \"prompt\", \"command\": cmd}]}",
    );
}

/// RULINGS CZ5: the recorded fingerprint is the fingerprint of the inputs the probes ran on — a lane
/// change made after the inputs were read (the seam runs right then) does not enter it; the verdict
/// is then neither recordable (the live lane no longer matches) nor reusable.
#[test]
fn verdict_is_bound_to_the_inputs_tested() {
    let root = test_root("cz5");
    let lane = lane_of(&root, &[]);
    let cand = candidate_of(&root, &[]);
    let before = input_fingerprint(&cand, &lane).unwrap();
    let mut c = cfg(&root, Some(vec![]), Some(vec![]), false);
    let lib = lane.join("hooks/_lib.sh");
    let lib2 = lib.clone();
    c.after_inputs_read = Some(Arc::new(move || {
        let mut t = std::fs::read_to_string(&lib2).unwrap();
        t.push_str("\n# changed while the screen runs\n");
        std::fs::write(&lib2, t).unwrap();
    }));
    let rep = screen_with(&c, &cand, &lane, LONG).unwrap();
    assert_eq!(rep.input_fingerprint, before, "the fingerprint of what was read");
    assert_ne!(input_fingerprint(&cand, &lane).unwrap(), before, "the lane did change");
    assert!(recordable_verdict(&rep, &cand, &lane).unwrap().is_none(), "not recorded");
    let rv = RecordedVerdict {
        schema: SCREEN_SCHEMA,
        verdict: rep.verdict,
        input_fingerprint: rep.input_fingerprint.clone(),
        build_id: rep.build_id.clone(),
        notes: vec![],
        at: 0.0,
    };
    assert!(!recorded_verdict_holds(&rv, &cand, &lane).unwrap(), "not reused");
    // unchanged lane: recordable
    let _ = lib;
    let mut c2 = cfg(&root, Some(vec![]), Some(vec![]), false);
    c2.after_inputs_read = None;
    let rep2 = screen_with(&c2, &cand, &lane, LONG).unwrap();
    assert!(recordable_verdict(&rep2, &cand, &lane).unwrap().is_some());
    cleanup(&root);
}

/// RULINGS CZ6: a live difference in the first pass whose repeat runs out of time is NOT "not written
/// again" — the screen refuses (and says it could not measure). The old code dropped the repeat's
/// out-of-time flag and accepted with a note.
#[test]
fn incomplete_repeat_is_a_failure() {
    let root = test_root("cz6");
    let lane = lane_of(&root, &[]);
    let cand = candidate_of(&root, &[]);
    let mut c = cfg(&root, Some(vec!["G-1"]), Some(vec!["E-1"]), false);
    let deadline = Duration::from_secs(8);
    let t0 = std::time::Instant::now();
    let lib = lane.join("hooks/_lib.sh");
    c.between_snapshots = Some(Arc::new(move |pass| {
        if pass == 1 {
            // a write to a live listed file during the first pass
            let mut t = std::fs::read_to_string(&lib).unwrap();
            t.push_str("\n# written during the first pass\n");
            std::fs::write(&lib, t).unwrap();
        } else {
            // the repeat gets no time
            let end = t0 + deadline + Duration::from_millis(300);
            std::thread::sleep(end.saturating_duration_since(std::time::Instant::now()));
        }
    }));
    let mut rep = screen_with(&c, &cand, &lane, deadline).unwrap();
    assert!(!rep.after_run.first.is_empty(), "{:?}", rep.after_run);
    assert!(rep.after_run.repeat_incomplete, "{:?}", rep.after_run);
    judge_after_run(&mut rep, &|_| false);
    assert_eq!(rep.verdict, ScreenVerdict::Refused, "{:?}", rep.refusals);
    assert!(!rep.not_measurable.is_empty());
    cleanup(&root);
}

/// RULINGS DF1: exit 0 is a deny only when the whole trimmed output is exactly one deny object.
#[test]
fn deny_is_read_as_strictly_as_the_harness() {
    let deny = r#"{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"x"}}"#;
    let rows: Vec<(String, bool)> = vec![
        (deny.to_string(), true),
        (format!("\n  {deny}\n\n"), true),
        (format!("one extra line\n{deny}\n"), false),
        (format!("{deny}\nafter\n"), false),
        (format!("{deny}\n{deny}\n"), false),
        (r#"{"decision":"block"}"#.to_string(), false),
        (deny.replace("deny\"", "allow\""), false),
        (deny.replace("PreToolUse", "PostToolUse"), false),
        ("[1]".to_string(), false),
        (String::new(), false),
    ];
    for (out, want) in rows {
        assert_eq!(deny_payload(out.as_bytes()), want, "{out:?}");
    }
}

/// RULINGS EO1: the deny object is a byte grammar — only JSON white space around exactly the
/// released devices' shape; every other white space, a second object, another key, another value
/// type, a missing or doubled key is "not shown". (Rust's `str::trim` took U+0085 / U+00A0 / U+FEFF
/// as white space and the old reading accepted any object with a deny decision.)
#[test]
fn deny_is_a_byte_grammar() {
    let deny = r#"{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"x"}}"#;
    let mut rows: Vec<(Vec<u8>, bool)> = vec![(deny.as_bytes().to_vec(), true)];
    rows.push((format!(" \t\r\n{deny}\r\n ").into_bytes(), true));
    for ws in ["\u{0085}", "\u{00A0}", "\u{FEFF}"] {
        rows.push((format!("{ws}{deny}").into_bytes(), false));
        rows.push((format!("{deny}{ws}").into_bytes(), false));
    }
    rows.push((format!("{deny}{deny}").into_bytes(), false));
    rows.push((deny.replacen("{\"hookSpecificOutput\"", "{\"extra\":\"1\",\"hookSpecificOutput\"", 1).into_bytes(), false));
    rows.push((deny.replace("\"permissionDecisionReason\":\"x\"", "\"permissionDecisionReason\":1").into_bytes(), false));
    rows.push((deny.replace(",\"permissionDecisionReason\":\"x\"", "").into_bytes(), false));
    rows.push((deny.replace("\"deny\",", "\"deny\",\"permissionDecision\":\"deny\",").into_bytes(), false));
    rows.push((deny.replace("\"x\"}", "\"x\",\"more\":\"y\"}").into_bytes(), false));
    for (out, want) in rows {
        assert_eq!(deny_payload(&out), want, "{:?}", String::from_utf8_lossy(&out));
    }
}

/// RULINGS EL1, through the real runner: a deny object followed by white space up to past the capture
/// limit and then plain text, exit 0 — the kept prefix parses, the WHOLE output does not: the answer
/// is "not shown" (ERROR), never DENY.
#[test]
fn output_behind_the_capture_limit_is_not_a_deny() {
    let root = test_root("el1-cap");
    let deny = r#"{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"x"}}"#;
    let script = format!(
        "printf '%s' '{deny}'; head -c {} /dev/zero | tr '\\000' ' '; printf 'not json'; exit 0",
        replay::MAX_CAPTURE + 4096
    );
    let env = vec![("PATH".to_string(), "/usr/bin:/bin".to_string())];
    let o = replay::run_child(&["/bin/sh".into(), "-c".into(), script], &env, &root, b"", Duration::from_secs(20));
    assert_eq!(o.code, Some(0), "{:?}", o.stderr_state);
    assert!(o.stdout_state.dropped > 0 && !o.stdout_whole());
    assert!(deny_payload(&o.stdout), "the kept prefix alone would read as a deny");
    let (a, d) = answer_of(&o);
    assert_eq!(a, Answer::Error, "{d}");
    cleanup(&root);
}

/// RULINGS EL1: a repeat is complete only when every planned execution ran to a result — ONE spawn
/// failure in the repeat with plenty of time left makes it incomplete, and the screen refuses. (The
/// old completeness was "time was left" and accepted with a note.)
#[test]
fn repeat_with_one_spawn_failure_is_incomplete() {
    let root = test_root("el1-spawn");
    let lane = lane_of(&root, &[]);
    let cand = candidate_of(&root, &[]);
    let mut c = cfg(&root, Some(vec!["G-1"]), Some(vec!["E-1"]), false);
    let lib = lane.join("hooks/_lib.sh");
    c.between_snapshots = Some(Arc::new(move |pass| {
        if pass == 1 {
            let mut t = std::fs::read_to_string(&lib).unwrap();
            t.push_str("\n# written during the first pass\n");
            std::fs::write(&lib, t).unwrap();
        }
    }));
    c.fail_first_spawn_in_repeat = true;
    let mut rep = screen_with(&c, &cand, &lane, LONG).unwrap();
    assert!(!rep.after_run.first.is_empty());
    assert!(rep.after_run.repeat_incomplete, "{:?}", rep.after_run);
    judge_after_run(&mut rep, &|_| false);
    assert_eq!(rep.verdict, ScreenVerdict::Refused, "{:?}", rep.refusals);
    cleanup(&root);
}

/// RULINGS EL1: an answer of the BL1 repeat — which runs without the cache — never enters the cache,
/// and an ERROR answer never does: a screen after a repeat whose execution failed to start still
/// observes G-1 in E-1 (in T5b-cz-c4 the injected failure was cached and a later screen read it).
#[test]
fn repeat_answers_never_enter_the_cache() {
    let root = test_root("el1-cache");
    let lane = lane_of(&root, &[]);
    let cand = candidate_of(&root, &[]);
    let mut c = cfg(&root, Some(vec!["G-1"]), Some(vec!["E-1"]), false);
    let lib = lane.join("hooks/_lib.sh");
    c.between_snapshots = Some(Arc::new(move |pass| {
        if pass == 1 {
            let mut t = std::fs::read_to_string(&lib).unwrap();
            t.push_str("\n# written during the first pass\n");
            std::fs::write(&lib, t).unwrap();
        }
    }));
    c.fail_first_spawn_in_repeat = true;
    let _ = screen_with(&c, &cand, &lane, LONG).unwrap();
    let c2 = cfg(&root, Some(vec!["G-1"]), Some(vec!["E-1"]), false);
    let lane2 = lane_of(&root.join("second"), &[]);
    let rep = screen_with(&c2, &candidate_of(&root.join("second"), &[]), &lane2, LONG).unwrap();
    for p in rep.probes.iter().filter(|p| p.class == "G-1") {
        assert!(!p.detail.contains("injected spawn failure"), "{p:#?}");
        assert_ne!(p.released, Answer::Error, "{p:#?}");
    }
    cleanup(&root);
}

/// RULINGS DF1 regression: a candidate `hooks/_lib.sh` that makes every hook print ONE extra stdout
/// line keeps a deny object in the output — the loose reading still saw DENY; the strict one sees no
/// decision, so every R / C class the released gate denies here is REFUSED.
#[test]
fn extra_stdout_line_from_lib_refuses_every_gate_class() {
    let root = test_root("df1");
    let lane = lane_of(&root, &[]);
    let lib = released("hooks/_lib.sh") + "\necho 'one extra line'\n";
    let cand = candidate_of(&root, &[("hooks/_lib.sh", Some(lib))]);
    let ids: Vec<&'static str> = corpus::CORPUS_R
        .iter()
        .chain(corpus::CORPUS_C.iter())
        .map(|c| c.id)
        .collect();
    let c = cfg(&root, Some(ids.clone()), Some(vec!["E-1"]), false);
    let rep = screen_with(&c, &cand, &lane, LONG).unwrap();
    let mut bound = std::collections::BTreeSet::new();
    for p in rep.probes.iter().filter(|p| p.test == "R" || p.test == "C") {
        if p.released == Answer::Deny {
            bound.insert(p.class.split('#').next().unwrap_or("").to_string());
            assert_eq!(p.decision, ProbeDecision::Refused, "{p:#?}");
        }
    }
    assert!(bound.len() >= 30, "R/C classes the released gate denies in E-1: {bound:?}");
    assert_eq!(rep.verdict, ScreenVerdict::Refused);
    cleanup(&root);
}

/// RULINGS DF2 — behaviour decides: a registration whose command names the gate but cannot make it
/// deny (`true || sh <gate>`; the gate's output swallowed) is not registered → REFUSED.
#[test]
fn i3_refuses_registrar_writing_true_or_gate() {
    i3_refuses_registrar_mutation("df2-or", REGISTRAR_FORM, "    return \"true || sh \" + script");
}

#[test]
fn i3_refuses_registrar_swallowing_the_gate_output() {
    i3_refuses_registrar_mutation(
        "df2-quiet",
        REGISTRAR_FORM,
        "    return \"sh \" + script + \" >/dev/null\"",
    );
}

/// RULINGS DF2 → EL2 (rewritten): a command outside CZ1's closed grammar that really runs the gate
/// (`exec sh <gate>`) is NOT the entry the released registrar writes — EL2 compares the entry as a
/// whole, so it is not shown registered and I3 REFUSES. (Old expectation, DF2: `(s3.released,
/// s3.clean) == (Answer::Deny, Some(Answer::Deny))` and no I3 probe refused — behaviour alone decided.)
#[test]
fn i3_accepts_a_registration_that_really_runs_the_gate() {
    i3_refuses_registrar_mutation("df2-exec", REGISTRAR_FORM, "    return \"exec sh \" + script");
}

/// RULINGS EL2 / EO2 — real registrar mutations whose command still denies the reviewer probe R-4, or
/// whose attributes the harness would honour: each differs from the released entry → REFUSED.
#[test]
fn i3_refuses_registrar_with_a_tiny_timeout() {
    i3_refuses_registrar_mutation(
        "el2-timeout",
        "entry[\"hooks\"][0][\"timeout\"] = timeout",
        "entry[\"hooks\"][0][\"timeout\"] = 0.001",
    );
}

#[test]
fn i3_refuses_registrar_with_async() {
    i3_refuses_registrar_mutation(
        "el2-async",
        "entry = {\"hooks\": [{\"type\": \"command\", \"command\": cmd}]}",
        "entry = {\"hooks\": [{\"type\": \"command\", \"command\": cmd, \"async\": True}]}",
    );
}

#[test]
fn i3_refuses_registrar_with_an_extra_key() {
    i3_refuses_registrar_mutation(
        "el2-key",
        "entry = {\"hooks\": [{\"type\": \"command\", \"command\": cmd}]}",
        "entry = {\"hooks\": [{\"type\": \"command\", \"command\": cmd, \"statusMessage\": \"x\"}]}",
    );
}

#[test]
fn i3_refuses_registrar_wrapping_the_command() {
    i3_refuses_registrar_mutation("el2-wrap", REGISTRAR_FORM, "    return \"sleep 2; sh \" + script");
}

/// EO2: a role-conditional command denies the one reviewer probe and switches the gate off for a CSO
/// seat — not the released entry → REFUSED.
#[test]
fn i3_refuses_registrar_with_a_role_condition() {
    i3_refuses_registrar_mutation(
        "eo2-role",
        REGISTRAR_FORM,
        "    return '[ \"$CYS_ROLE\" = cso ] || sh ' + script",
    );
}

/// EL2: `bash <gate>` passes CZ1's grammar and denies R-4, yet differs from the released `sh <gate>`
/// entry → REFUSED (the case only the whole-entry comparison catches).
#[test]
fn i3_refuses_registrar_naming_another_interpreter() {
    i3_refuses_registrar_mutation("el2-bash", REGISTRAR_FORM, "    return \"bash \" + script");
}

/// I4: a candidate guard.sh whose backstop region differs from the released copy → REFUSED; a
/// candidate whose region anchor is gone → REFUSED; a lane whose own copy differs and a candidate
/// that does not touch guard.sh → a lane finding (BA4).
#[test]
fn i4_regions_judged() {
    let root = test_root("i4");
    let lane = lane_of(&root, &[]);
    let c = cfg(&root, Some(vec![]), Some(vec![]), false);
    let g = released("hooks/guard.sh");
    let changed = g.replace("백스톱: 비가역 명령. 오너 승인 필요", "백스톱: 비가역 명령");
    let rep = screen_with(
        &c,
        &candidate_of(&root, &[("hooks/guard.sh", Some(changed.clone()))]),
        &lane,
        LONG,
    )
    .unwrap();
    let b = rep
        .probes
        .iter()
        .find(|p| p.class == "I4:bash-backstop")
        .unwrap();
    assert_eq!(b.decision, ProbeDecision::Refused, "{b:#?}");
    let renamed = g.replace("run_loose_backstop", "run_backstop_loose");
    let rep = screen_with(
        &c,
        &candidate_of(&root, &[("hooks/guard.sh", Some(renamed))]),
        &lane,
        LONG,
    )
    .unwrap();
    let b = rep
        .probes
        .iter()
        .find(|p| p.class == "I4:bash-backstop")
        .unwrap();
    assert_eq!(b.decision, ProbeDecision::Refused);
    assert!(b.detail.contains("anchor not found"));
    // not touching guard.sh in a lane whose copy differs → lane finding
    let root2 = test_root("i4b");
    let lane2 = lane_of(&root2, &[("hooks/guard.sh", Some(changed))]);
    let rep = screen_with(
        &c,
        &candidate_of(
            &root2,
            &[("bin/javis_lock.py", Some(released("bin/javis_lock.py")))],
        ),
        &lane2,
        LONG,
    )
    .unwrap();
    let b = rep
        .probes
        .iter()
        .find(|p| p.class == "I4:bash-backstop")
        .unwrap();
    assert_eq!(b.decision, ProbeDecision::LaneFinding);
    assert_ne!(rep.verdict, ScreenVerdict::Refused);
    cleanup(&root);
    cleanup(&root2);
}

/// E-21 (BC3, BH1): a candidate _lib.sh that switches the released device's own no-Python switch off
/// cannot turn the backstop cell into a normal run that passes — the marker is missing → REFUSED. A
/// harmless _lib.sh change keeps the backstop proven.
#[test]
fn e21_backstop_marker_required() {
    let root = test_root("e21");
    let lane = lane_of(&root, &[]);
    let c = cfg(
        &root,
        Some(vec!["G-22", "G-23", "G-24np"]),
        Some(vec!["E-21"]),
        false,
    );
    let lib = released("hooks/_lib.sh");
    let rep = screen_with(
        &c,
        &candidate_of(
            &root,
            &[("hooks/_lib.sh", Some(format!("{lib}\n# harmless\n")))],
        ),
        &lane,
        LONG,
    )
    .unwrap();
    for id in ["G-22", "G-23", "G-24np"] {
        let p = rep
            .probes
            .iter()
            .find(|p| p.class == id)
            .unwrap_or_else(|| panic!("{id} not probed"));
        assert_eq!(p.decision, ProbeDecision::Held, "{p:#?}");
    }
    let evasive = format!("{lib}\nunset GUARD_TEST_MODE GUARD_FORCE_NOPY\n");
    let rep = screen_with(
        &c,
        &candidate_of(&root, &[("hooks/_lib.sh", Some(evasive))]),
        &lane,
        LONG,
    )
    .unwrap();
    let p = rep.probes.iter().find(|p| p.class == "G-22").unwrap();
    assert_eq!(p.decision, ProbeDecision::Refused, "{p:#?}");
    assert_eq!(rep.verdict, ScreenVerdict::Refused);
    cleanup(&root);
}

/// AT4 on the FILE (BD3) with BG1: a table that gains a `capgate: deny` the lane's file does not hold
/// is refused; restoring the shipped `.example` to its released bytes is not.
#[test]
fn at4_capgate_deny_added_refused() {
    let root = test_root("at4");
    let ex = released("state/hook-targets.json.example");
    let mut doc: serde_json::Value = serde_json::from_str(&ex).unwrap();
    let profiles = doc["profiles"].as_array_mut().unwrap();
    profiles.push(serde_json::json!({"basename": ".claude", "eligibility": {"capgate": "deny"}}));
    let with_deny = serde_json::to_string_pretty(&doc).unwrap();
    let lane = lane_of(&root, &[]);
    let c = cfg(&root, Some(vec![]), Some(vec![]), false);
    let rep = screen_with(
        &c,
        &candidate_of(
            &root,
            &[("state/hook-targets.json.example", Some(with_deny.clone()))],
        ),
        &lane,
        LONG,
    )
    .unwrap();
    let a = rep
        .probes
        .iter()
        .find(|p| p.class == "AT4:state/hook-targets.json.example")
        .unwrap();
    assert_eq!(a.decision, ProbeDecision::Refused, "{a:#?}");
    // BG1: the lane's .example had its denies removed; the candidate restores the released bytes
    let mut stripped: serde_json::Value = serde_json::from_str(&ex).unwrap();
    for p in stripped["profiles"].as_array_mut().unwrap() {
        if let Some(e) = p.get_mut("eligibility").and_then(|e| e.as_object_mut()) {
            e.remove("capgate");
        }
    }
    let root2 = test_root("at4b");
    let lane2 = lane_of(
        &root2,
        &[(
            "state/hook-targets.json.example",
            Some(serde_json::to_string_pretty(&stripped).unwrap()),
        )],
    );
    let rep = screen_with(
        &c,
        &candidate_of(&root2, &[("state/hook-targets.json.example", Some(ex))]),
        &lane2,
        LONG,
    )
    .unwrap();
    let a = rep
        .probes
        .iter()
        .find(|p| p.class == "AT4:state/hook-targets.json.example")
        .unwrap();
    assert_eq!(a.decision, ProbeDecision::Held, "BG1: {a:#?}");
    cleanup(&root);
    cleanup(&root2);
}

// ───────────────────────────── replay rules: socket, after-run comparison ─────────────────────────────

/// BH2 (b): the only cys a probe reaches is the screen's own stub on its own scratch socket —
/// never the socket of the process running the screen.
#[test]
fn screen_never_reaches_a_real_socket() {
    let root = test_root("sock");
    let lane = lane_of(&root, &[]);
    let c = cfg(&root, Some(vec!["G-1"]), Some(vec!["E-1"]), false);
    let rep = screen_with(&c, &candidate_of(&root, &[]), &lane, LONG).unwrap();
    let g1 = rep
        .probes
        .iter()
        .find(|p| p.class == "G-1" && p.cell == "E-1")
        .unwrap();
    let sock_arg = g1
        .detail
        .split("cys --socket ")
        .nth(1)
        .and_then(|s| s.split_whitespace().next())
        .unwrap_or("");
    assert!(
        sock_arg.starts_with(&*c.scratch_base.to_string_lossy())
            || sock_arg.starts_with("/private/tmp/cys-rs-")
            || sock_arg.starts_with("/tmp/cys-rs-"),
        "the probe asked a socket outside the screen's scratch: {sock_arg:?} ({})",
        g1.detail
    );
    if let Ok(own) = std::env::var("CYS_SOCKET") {
        assert_ne!(sock_arg, own);
    }
    assert!(
        !std::path::Path::new(sock_arg).exists(),
        "the scratch socket was not removed"
    );
    cleanup(&root);
}

/// BK1 / BL1: a candidate guard.sh that writes a live listed file of the lane on every run is caught
/// by the after-run comparison — written again during the repeat → REFUSED.
#[test]
fn after_run_rewrite_refused() {
    let root = test_root("bl1");
    let lane = lane_of(&root, &[]);
    let target = lane.join("hooks/_lib.sh");
    let g = released("hooks/guard.sh");
    let escaping = g.replacen(
        "set -u\n",
        &format!("set -u\ntouch '{}' 2>/dev/null\n", target.display()),
        1,
    );
    let c = cfg(&root, Some(vec!["G-1"]), Some(vec!["E-4"]), false);
    let mut rep = screen_with(
        &c,
        &candidate_of(&root, &[("hooks/guard.sh", Some(escaping))]),
        &lane,
        LONG,
    )
    .unwrap();
    assert!(
        rep.after_run
            .first
            .iter()
            .any(|d| d.item.ends_with("hooks/_lib.sh")),
        "{:?}",
        rep.after_run
    );
    assert!(rep
        .after_run
        .rewritten_in_repeat
        .iter()
        .any(|d| d.item.ends_with("hooks/_lib.sh")));
    judge_after_run(&mut rep, &|_| false);
    assert_eq!(rep.verdict, ScreenVerdict::Refused);
    assert!(rep.refusals.iter().any(|r| r.contains("다시 썼습니다")));
    cleanup(&root);
}

/// BL1: a difference that is not written again in the repeat is a note, not a refusal; a difference
/// the daemon's own recorded live operation explains is no finding at all.
#[test]
fn after_run_single_difference_noted() {
    let d = LiveDifference {
        item: "/x/pack/hooks/_lib.sh".into(),
        before: "a".into(),
        after: "b".into(),
    };
    let mut rep = summarize(Vec::new(), "fp".into(), Duration::from_millis(1));
    rep.after_run.first = vec![d.clone()];
    let mut once = rep.clone();
    judge_after_run(&mut once, &|_| false);
    assert_eq!(once.verdict, ScreenVerdict::AcceptedWithNotes);
    assert_eq!(once.lane_findings.len(), 1);
    let mut explained = rep.clone();
    judge_after_run(&mut explained, &|_| true);
    assert_eq!(explained.verdict, ScreenVerdict::Accepted);
    let mut again = rep;
    again.after_run.rewritten_in_repeat = vec![d];
    judge_after_run(&mut again, &|_| false);
    assert_eq!(again.verdict, ScreenVerdict::Refused);
}

// ───────────────────────────── the verdict travels with what was screened (AY5, BC7, BD5) ─────────────────────────────

#[test]
fn verdict_record_roundtrip_and_holds() {
    let root = test_root("verdict");
    let lane = lane_of(&root, &[]);
    let cand = candidate_of(
        &root,
        &[(
            "bin/javis_lock.py",
            Some(format!("{}\n# x\n", released("bin/javis_lock.py"))),
        )],
    );
    let fp = input_fingerprint(&cand, &lane).unwrap();
    let ledger = root.join("repairs");
    let id = RepairId::parse("R-7").unwrap();
    std::fs::create_dir_all(super::super::ledger::repair_dir_in(&ledger, &id)).unwrap();
    assert_eq!(read_verdict_in(&ledger, &id).unwrap(), None);
    let v = RecordedVerdict {
        schema: SCREEN_SCHEMA,
        verdict: ScreenVerdict::Accepted,
        input_fingerprint: fp.clone(),
        build_id: screen_build_id(),
        notes: vec![],
        at: 1.0,
    };
    write_verdict_in(&ledger, &id, &v).unwrap();
    let back = read_verdict_in(&ledger, &id).unwrap().unwrap();
    assert_eq!(back, v);
    assert!(recorded_verdict_holds(&back, &cand, &lane).unwrap());
    // another build id → never reused (BD5)
    let mut other = back.clone();
    other.build_id = "elsewhere".into();
    assert!(!recorded_verdict_holds(&other, &cand, &lane).unwrap());
    // a refused verdict never holds
    let mut refused = back.clone();
    refused.verdict = ScreenVerdict::Refused;
    assert!(!recorded_verdict_holds(&refused, &cand, &lane).unwrap());
    // the lane's inputs differ (another lane) → does not hold
    std::fs::write(lane.join("hooks/_lib.sh"), "# other lane\n").unwrap();
    assert!(!recorded_verdict_holds(&back, &cand, &lane).unwrap());
    // a file outside the inputs does not move the fingerprint
    let fp2 = input_fingerprint(&cand, &lane).unwrap();
    std::fs::write(lane.join("README.md"), "x").unwrap();
    assert_eq!(fp2, input_fingerprint(&cand, &lane).unwrap());
    // a higher schema on disk is corrupt, not a verdict
    std::fs::write(verdict_path_in(&ledger, &id), r#"{"schema": 99, "verdict": "accepted", "input_fingerprint": "x", "build_id": "y", "notes": [], "at": 1.0}"#).unwrap();
    assert!(matches!(
        read_verdict_in(&ledger, &id),
        Err(RepairError::Corrupt(_))
    ));
    cleanup(&root);
}

/// The candidate's bytes must be the bytes it declares (a swap after the states were read is not
/// screened).
#[test]
fn candidate_bytes_must_match_declared_states() {
    let root = test_root("swap");
    let lane = lane_of(&root, &[]);
    let mut cand = candidate_of(
        &root,
        &[("hooks/guard.sh", Some(released("hooks/guard.sh")))],
    );
    std::fs::write(cand.files.join("hooks/guard.sh"), "#!/bin/sh\nexit 0\n").unwrap();
    let c = cfg(&root, Some(vec![]), Some(vec![]), false);
    assert!(matches!(
        screen_with(&c, &cand, &lane, LONG),
        Err(RepairError::Precondition(_))
    ));
    cand.targets[0].0 = "../escape".into();
    assert!(screen_with(&c, &cand, &lane, LONG).is_err());
    cleanup(&root);
}
