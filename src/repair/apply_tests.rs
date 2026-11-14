//! T6 tests of rule one (the property test of K3 SA:120 over every operation × injected point ×
//! action), revert, the approved apply, F9 / R2 / P6 and the hook step.

use super::*;
use crate::repair::ledger::{
    Attestation, ChangeKind, CheckTiming, FileChange, LaneRecord, Probation, RepairRecord,
};
use crate::repair::types::{LockOwner, Trigger};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;

pub(crate) struct Rig {
    pub base: PathBuf,
    pub root: PathBuf,
    pub pack: PathBuf,
    pub lane: LaneKey,
    pub src: PathBuf,
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

pub(crate) fn rig(tag: &str) -> Rig {
    let base = std::env::temp_dir().join(format!(
        "cys-t6-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let root = base.join("local").join("repairs");
    let pack = base.join("pack");
    let src = base.join("src");
    for d in [&root, &pack, &src] {
        std::fs::create_dir_all(d).unwrap();
    }
    std::fs::create_dir_all(pack.join("hooks")).unwrap();
    std::fs::write(pack.join(".pack-version"), "0.15.0\n").unwrap();
    Rig {
        base,
        root,
        pack,
        lane: LaneKey("base".into()),
        src,
    }
}

impl Rig {
    pub fn env<'a>(&'a self, faults: &'a dyn Faults, form: ApplyForm) -> OpEnv<'a> {
        OpEnv {
            root: &self.root,
            pack: &self.pack,
            lane: &self.lane,
            now: 1_800_000_000.0,
            form,
            faults,
            own_check: &|_, _| None,
        }
    }
    pub fn put(&self, rel: &str, bytes: &[u8], mode: u32) -> TargetState {
        let p = self.pack.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        let _ = std::fs::remove_file(&p);
        std::fs::write(&p, bytes).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode)).unwrap();
        }
        TargetState::read(&p).unwrap()
    }
    pub fn source(&self, name: &str, bytes: &[u8], mode: u32) -> (PathBuf, TargetState) {
        let p = self.src.join(name);
        std::fs::write(&p, bytes).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode)).unwrap();
        }
        let st = TargetState::read(&p).unwrap();
        (p, st)
    }
    pub fn live(&self, rel: &str) -> TargetState {
        TargetState::read(&self.pack.join(rel)).unwrap()
    }
    /// Every regular file's bytes under the vault (`displaced/`), recursively, plus the M10 sub-ledger.
    pub fn vault_bytes(&self) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        let mut stack = vec![self.root.clone()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                let p = e.path();
                let m = std::fs::symlink_metadata(&p).unwrap();
                if m.is_dir() {
                    stack.push(p);
                } else if m.is_file()
                    && p.components().any(|c| c.as_os_str() == "displaced")
                    && (p.parent().unwrap().ends_with("payload") || p.parent().unwrap().ends_with("sealed"))
                {
                    out.push(std::fs::read(&p).unwrap());
                }
            }
        }
        out
    }
    /// Every slot directory (complete ones have payload/, sealed/, meta/pre.json — O5).
    pub fn slots(&self) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let d = self.root.join("displaced");
        for r in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            for s in std::fs::read_dir(r.path()).into_iter().flatten().flatten() {
                out.push(s.path());
            }
        }
        out.sort();
        out
    }
}

pub(crate) fn rid(s: &str) -> RepairId {
    RepairId::parse(s).unwrap()
}

/// What an injected fault does at its point.
#[derive(Clone)]
enum Action {
    /// The process "dies" here.
    Stop,
    /// Another writer replaces the live path (rename-over) with these bytes.
    ReplaceLive(Vec<u8>),
    /// A process that opened the live file BEFORE the operation writes through that descriptor.
    OldFd(Vec<u8>),
    /// A pack update changes the pack generation.
    BumpGeneration,
}

struct Inject {
    point: Point,
    action: Action,
    fired: Cell<bool>,
    held: RefCell<Option<std::fs::File>>,
    /// bytes that were live right after the injected writer wrote
    wrote_live: RefCell<Option<Vec<u8>>>,
    pack: PathBuf,
    /// CU3: after the action fired, stop at this point of a NESTED operation (another slot than the
    /// first one seen) — a crash inside the undo of a superseded exchange.
    then_stop_at: Option<Point>,
    outer_slot: RefCell<Option<PathBuf>>,
    stopped_nested: Cell<bool>,
}

impl Inject {
    fn new(point: Point, action: Action, held: Option<std::fs::File>, pack: &Path) -> Inject {
        Inject {
            point,
            action,
            fired: Cell::new(false),
            held: RefCell::new(held),
            wrote_live: RefCell::new(None),
            pack: pack.to_path_buf(),
            then_stop_at: None,
            outer_slot: RefCell::new(None),
            stopped_nested: Cell::new(false),
        }
    }
}

impl Faults for Inject {
    fn at(&self, point: Point, live: &Path, slot: Option<&Slot>) -> bool {
        if let Some(s) = slot {
            if self.outer_slot.borrow().is_none() {
                *self.outer_slot.borrow_mut() = Some(s.dir.clone());
            }
        }
        if self.fired.get() {
            if let (Some(p), Some(s)) = (self.then_stop_at, slot) {
                let nested = self.outer_slot.borrow().as_ref() != Some(&s.dir);
                if p == point && nested && !self.stopped_nested.get() {
                    self.stopped_nested.set(true);
                    return true;
                }
            }
            return false;
        }
        if point != self.point {
            return false;
        }
        self.fired.set(true);
        match &self.action {
            Action::Stop => true,
            Action::ReplaceLive(b) => {
                let tmp = live.with_extension("other-writer");
                std::fs::write(&tmp, b).unwrap();
                std::fs::rename(&tmp, live).unwrap();
                *self.wrote_live.borrow_mut() = Some(b.clone());
                false
            }
            Action::OldFd(b) => {
                use std::io::Write;
                if let Some(f) = self.held.borrow_mut().as_mut() {
                    f.write_all(b).unwrap();
                }
                false
            }
            Action::BumpGeneration => {
                std::fs::write(self.pack.join(".pack-version"), "0.15.1\n").unwrap();
                false
            }
        }
    }
}

/// One operation of the property test: (name, live before, incoming, form).
struct Case {
    name: &'static str,
    rel: &'static str,
    before: Option<(&'static [u8], u32, Option<&'static str>)>,
    incoming: Option<(&'static [u8], u32, Option<&'static str>)>,
    form: ApplyForm,
}

const CASES: &[Case] = &[
    Case { name: "edit", rel: "hooks/a.sh", before: Some((b"OLD-BYTES\n", 0o755, None)), incoming: Some((b"NEW-BYTES\n", 0o755, None)), form: ApplyForm::Exchange },
    Case { name: "mode", rel: "hooks/m.sh", before: Some((b"SAME\n", 0o644, None)), incoming: Some((b"SAME\n", 0o755, None)), form: ApplyForm::Exchange },
    Case { name: "link", rel: "hooks/l.sh", before: Some((b"FILE\n", 0o644, None)), incoming: Some((b"", 0, Some("/nonexistent/target"))), form: ApplyForm::Exchange },
    Case { name: "move-in", rel: "hooks/c.sh", before: Some((b"CREATED\n", 0o644, None)), incoming: None, form: ApplyForm::Exchange },
    Case { name: "place", rel: "hooks/d.sh", before: None, incoming: Some((b"PLACED\n", 0o644, None)), form: ApplyForm::Exchange },
    Case { name: "windows", rel: "bin/w.py", before: Some((b"WIN-OLD\n", 0o644, None)), incoming: Some((b"WIN-NEW\n", 0o644, None)), form: ApplyForm::DepositFirst },
];

fn make_state(r: &Rig, name: &str, spec: Option<(&[u8], u32, Option<&str>)>) -> (TargetState, Option<PathBuf>) {
    match spec {
        None => (TargetState::Absent, None),
        Some((_, _, Some(t))) => (TargetState::Link { target: t.to_string() }, None),
        Some((b, m, None)) => {
            let (p, st) = r.source(name, b, m);
            (st, Some(p))
        }
    }
}

/// THE property (RULINGS K3 SA:120, C2 2.4; CT1, CU3): for every operation × injected point ×
/// action — the points include crashes INSIDE the seal and inside `meta/sealed.json` (CT1), on the
/// byte-copy path as well as the clone path, a crash inside the nested undo of a superseded
/// exchange, and a SECOND crash during the recovery (CU3) — after the operation and the recoveries
/// the next job runs perform: (P0, CU3) the LIVE content is exactly one of the legitimate versions
/// (the old bytes, the incoming state, a concurrent writer's bytes, the old bytes plus an old
/// descriptor's write in the stated residual instant, or absent where a target state is absent) —
/// never a truncated or foreign copy; (P1) every byte string that was live at the instant of an
/// exchange / move is in the vault or live; (P2) no slot directory that existed is gone, every slot
/// is complete (O5), no temporary entry (`.part-`) is left in a slot, and every published seal
/// agrees with its record (CT1); (P3) the live path is absent only where a target state is absent;
/// (P4) every operation record is finished.
#[test]
fn rule_one_property_every_op_every_point() {
    let points = [
        Point::AfterSlot,
        Point::AfterRecord,
        Point::AfterExchange,
        Point::InSealCreated,
        Point::InSealPartial,
        Point::InSealWritten,
        Point::InSealPlaced,
        Point::InMetaPartial,
        Point::InMetaWritten,
        Point::AfterSeal,
        Point::AfterFirstMove,
        Point::BeforeDone,
    ];
    let inside_seal = |p: Point| {
        matches!(
            p,
            Point::InSealCreated | Point::InSealPartial | Point::InSealWritten | Point::InSealPlaced | Point::InMetaPartial | Point::InMetaWritten
        )
    };
    let mut combos = 0;
    let mut fired_inside_seal = 0;
    for case in CASES {
        for point in points {
            if point == Point::AfterFirstMove && case.form != ApplyForm::DepositFirst {
                continue;
            }
            for byte_copy in [false, true] {
                if !inside_seal(point) && byte_copy != (combos % 2 == 1) {
                    continue; // the other points run once, alternating the seal path
                }
                for (action, nested_stop) in [
                    (Action::Stop, None),
                    (Action::ReplaceLive(b"OTHER-WRITER\n".to_vec()), None),
                    (Action::OldFd(b"+late".to_vec()), None),
                    (Action::BumpGeneration, None),
                    (Action::ReplaceLive(b"OTHER-WRITER\n".to_vec()), Some(Point::AfterExchange)),
                    (Action::ReplaceLive(b"OTHER-WRITER\n".to_vec()), Some(Point::InSealPartial)),
                ] {
                    if nested_stop.is_some() && point != Point::AfterRecord {
                        continue;
                    }
                    combos += 1;
                    crate::repair::vault::FORCE_BYTE_COPY_SEAL.with(|c| c.set(byte_copy));
                    let r = rig(&format!("prop-{}-{combos}", case.name));
                    let (expected, live_bytes) = match case.before {
                        Some((b, m, None)) => (r.put(case.rel, b, m), Some(b.to_vec())),
                        Some((_, _, Some(_))) => unreachable!(),
                        None => (TargetState::Absent, None),
                    };
                    let (incoming, source) = make_state(&r, "incoming", case.incoming);
                    let held = live_bytes.as_ref().map(|_| {
                        std::fs::OpenOptions::new().append(true).open(r.pack.join(case.rel)).unwrap()
                    });
                    let mut inj = Inject::new(point, action.clone(), held, &r.pack);
                    inj.then_stop_at = nested_stop;
                    let env = r.env(&inj, case.form);
                    let spec = OpSpec {
                        id: rid("R-5"),
                        rel: case.rel.to_string(),
                        kind: "revert".into(),
                        expected: expected.clone(),
                        incoming: incoming.clone(),
                        source,
                        generation: None,
                    };
                    let first = run_op(&env, &spec);
                    if let Err(e) = &first {
                        assert!(format!("{e:?}").contains(INJECTED_STOP), "{}/{point:?}: {e:?}", case.name);
                        if inside_seal(point) {
                            fired_inside_seal += 1;
                        }
                    }
                    // CU3: a SECOND crash, at the same point, during the first recovery; then calm runs
                    if matches!(action, Action::Stop) {
                        let again = Inject::new(point, Action::Stop, None, &r.pack);
                        let _ = resume_interrupted_in(&r.env(&again, case.form));
                    }
                    let calm = NoFaults;
                    let env2 = r.env(&calm, case.form);
                    resume_interrupted_in(&env2).unwrap();
                    resume_interrupted_in(&env2).unwrap();
                    crate::repair::vault::FORCE_BYTE_COPY_SEAL.with(|c| c.set(false));
                    let tag = format!(
                        "{} at {point:?} with {}{}{}",
                        case.name,
                        match &action {
                            Action::Stop => "stop",
                            Action::ReplaceLive(_) => "replace-live",
                            Action::OldFd(_) => "old-fd",
                            Action::BumpGeneration => "bump-generation",
                        },
                        if byte_copy { " (byte-copy seal)" } else { "" },
                        nested_stop.map(|p| format!(" + nested stop at {p:?}")).unwrap_or_default()
                    );
                    // P0 (CU3): the live content is one of the legitimate versions
                    let live_state = r.live(case.rel);
                    let mut legit: Vec<TargetState> = vec![expected.clone(), incoming.clone()];
                    if let Some(b) = &live_bytes {
                        let mode = match &expected {
                            TargetState::File { mode, .. } => *mode,
                            _ => 0o644,
                        };
                        let mut late = b.clone();
                        late.extend_from_slice(b"+late");
                        let p = r.src.join("late-version");
                        std::fs::write(&p, &late).unwrap();
                        {
                            use std::os::unix::fs::PermissionsExt;
                            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode)).unwrap();
                        }
                        legit.push(TargetState::read(&p).unwrap());
                    }
                    if let Some(w) = inj.wrote_live.borrow().as_ref() {
                        let p = r.src.join("other-version");
                        std::fs::write(&p, w).unwrap();
                        legit.push(TargetState::read(&p).unwrap());
                        // an other writer's file keeps the mode its rename gave it
                        let live_p = r.pack.join(case.rel);
                        if let Ok(TargetState::File { sha256, .. }) = TargetState::read(&live_p) {
                            let w_sha: String = {
                                use sha2::{Digest, Sha256};
                                Sha256::digest(w).iter().map(|x| format!("{x:02x}")).collect()
                            };
                            if sha256 == w_sha {
                                legit.push(TargetState::read(&live_p).unwrap());
                            }
                        }
                    }
                    if live_state != TargetState::Absent {
                        assert!(legit.contains(&live_state), "P0 {tag}: the live content is not a legitimate version: {}", live_state.canon());
                    }
                    // P1
                    let vault = r.vault_bytes();
                    let before_exchange = matches!(point, Point::AfterSlot | Point::AfterRecord);
                    let up_to_seal = before_exchange || point == Point::AfterExchange || inside_seal(point);
                    if let Some(b) = &live_bytes {
                        let replaced_before = matches!(action, Action::ReplaceLive(_)) && before_exchange;
                        let late_possible = matches!(action, Action::OldFd(_)) && up_to_seal;
                        let mut late = b.clone();
                        late.extend_from_slice(b"+late");
                        let live_now = std::fs::read(r.pack.join(case.rel)).ok();
                        let found = |w: &Vec<u8>| vault.contains(w) || live_now.as_deref() == Some(w.as_slice());
                        assert!(
                            replaced_before || found(b) || (late_possible && found(&late)),
                            "P1 {tag}: the bytes live at the exchange are neither in the vault nor live"
                        );
                    }
                    if let Some(w) = inj.wrote_live.borrow().as_ref() {
                        let live_now = std::fs::read(r.pack.join(case.rel)).ok();
                        assert!(
                            vault.contains(w) || live_now.as_deref() == Some(w.as_slice()),
                            "P1 {tag}: the other writer's bytes were lost"
                        );
                    }
                    // P2 / O5 / CT1
                    for s in r.slots() {
                        let name = s.file_name().unwrap().to_string_lossy().to_string();
                        assert!(!name.starts_with(".building"), "P2 {tag}: half-made slot left: {name}");
                        if crate::repair::ledger::is_slot_name(&name) {
                            assert!(s.join("payload").is_dir() && s.join("sealed").is_dir() && s.join("meta/pre.json").is_file(), "O5 {tag}: incomplete slot {}", s.display());
                            for sub in ["sealed", "meta"] {
                                for e in std::fs::read_dir(s.join(sub)).unwrap().flatten() {
                                    let n = e.file_name().to_string_lossy().to_string();
                                    assert!(!n.contains(".part-"), "CT1 {tag}: unfinished temporary entry left in {}: {n}", s.display());
                                }
                            }
                            let base = std::fs::read_dir(s.join("payload")).unwrap().flatten().next().map(|e| e.file_name().to_string_lossy().to_string());
                            if let Some(base) = base {
                                let slot = Slot { dir: s.clone(), basename: base };
                                if std::fs::symlink_metadata(slot.sealed()).is_ok() && slot.meta_sealed().is_file() {
                                    assert!(crate::repair::vault::deposit_of(&slot).is_ok(), "CT1 {tag}: a published seal disagrees with its record in {}", s.display());
                                }
                            }
                        }
                    }
                    // P3
                    if r.live(case.rel) == TargetState::Absent {
                        assert!(
                            case.incoming.is_none() || case.before.is_none(),
                            "P3 {tag}: the live path ended absent"
                        );
                    }
                    // P4
                    let (ops, broken) = wal::list_in(&r.root, &rid("R-5"));
                    assert!(broken.is_empty(), "{tag}: {broken:?}");
                    for o in ops {
                        assert!(o.finished, "P4 {tag}: unfinished record {} at {}", o.op, o.step);
                    }
                }
            }
        }
    }
    assert!(combos >= 150, "only {combos} combinations ran");
    assert!(fired_inside_seal >= 20, "the crashes inside the seal fired only {fired_inside_seal} times");
}

/// The normal outcomes of each form: applied, the displaced bytes sealed in the vault, E2 mode and
/// E3 fresh mtime on the exchanged-in entry, nothing left unfinished.
#[cfg(target_os = "macos")]
#[test]
fn every_form_applies_with_mode_and_fresh_mtime() {
    use std::os::unix::fs::PermissionsExt;
    let r = rig("forms");
    // exchange with a mode change and an old source mtime
    r.put("hooks/a.sh", b"OLD\n", 0o644);
    let (src, st) = r.source("new.sh", b"NEW\n", 0o755);
    let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000);
    std::fs::File::options().write(true).open(&src).unwrap().set_modified(old).unwrap();
    let env = r.env(&NoFaults, ApplyForm::Exchange);
    let d = run_op(&env, &OpSpec {
        id: rid("R-1"),
        rel: "hooks/a.sh".into(),
        kind: "apply".into(),
        expected: r.live("hooks/a.sh"),
        incoming: st.clone(),
        source: Some(src),
        generation: None,
    })
    .unwrap();
    assert_eq!(d.outcome, ApplyOutcome::Applied);
    let m = std::fs::metadata(r.pack.join("hooks/a.sh")).unwrap();
    assert_eq!(m.permissions().mode() & 0o7777, 0o755, "E2: the intended mode");
    assert!(m.modified().unwrap() > old + std::time::Duration::from_secs(86_400), "E3: fresh mtime");
    assert_eq!(std::fs::read(r.pack.join("hooks/a.sh")).unwrap(), b"NEW\n");
    let slot = Slot::at(&d.slot.unwrap(), "hooks/a.sh");
    assert_eq!(std::fs::read(slot.sealed()).unwrap(), b"OLD\n", "the displaced bytes are sealed");
    assert!(slot.meta_sealed().is_file());
    // move-in: the created file deposited, the path ends absent
    r.put("hooks/c.sh", b"C\n", 0o644);
    let d = run_op(&env, &OpSpec {
        id: rid("R-1"),
        rel: "hooks/c.sh".into(),
        kind: "revert".into(),
        expected: r.live("hooks/c.sh"),
        incoming: TargetState::Absent,
        source: None,
        generation: None,
    })
    .unwrap();
    assert_eq!(d.outcome, ApplyOutcome::Applied);
    assert_eq!(r.live("hooks/c.sh"), TargetState::Absent);
    assert!(r.vault_bytes().contains(&b"C\n".to_vec()));
    // place: an occupied path is never overwritten
    let (src, st) = r.source("p.sh", b"P\n", 0o644);
    r.put("hooks/p.sh", b"SOMEONE\n", 0o644);
    let d = run_op(&env, &OpSpec {
        id: rid("R-1"),
        rel: "hooks/p.sh".into(),
        kind: "revert".into(),
        expected: TargetState::Absent,
        incoming: st,
        source: Some(src),
        generation: None,
    })
    .unwrap();
    assert!(matches!(d.outcome, ApplyOutcome::Superseded(_)));
    assert_eq!(std::fs::read(r.pack.join("hooks/p.sh")).unwrap(), b"SOMEONE\n");
    assert!(wal::unfinished_for_lane(&r.root, &r.lane).is_empty());
}

/// M2: nothing but the seal sits between the exchange / first-move call and the clone — a source
/// pin over this file's production code.
#[test]
fn seal_is_the_next_operation_after_every_exchange_and_move() {
    let src = include_str!("apply.rs");
    let prod = &src[..src.find("#[cfg(test)]\n#[path = \"apply_tests.rs\"]").unwrap()];
    let mut seen = 0;
    for needle in ["exchange(live, &slot.payload())", "exchange(&live, &slot.payload())", "place_no_replace(live, &slot.payload())"] {
        for (at, _) in prod.match_indices(needle) {
            seen += 1;
            // the statement after the call's error arm (which only closes the record and returns)
            let tail = &prod[at..];
            let next = tail.find("seal_now(").expect("no seal after an exchange / move");
            let between = &tail[..next];
            let arm_end = between.rfind("}\n").unwrap_or(0);
            let after_arm = &between[arm_end..];
            assert!(
                after_arm.trim_start_matches("}\n").trim().is_empty(),
                "something sits between the exchange and the seal: {after_arm:?}"
            );
        }
    }
    assert!(seen >= 4, "the pin saw {seen} exchange / move calls");
    // and seal_now does nothing before the clone but the test hook
    let body = &prod[prod.find("fn seal_now(").unwrap()..];
    let body = &body[..body.find("\n}\n").unwrap()];
    assert!(!body.contains("full_flush") && !body.contains("wal::") && !body.contains("identity_of") && !body.contains("TargetState::read"));
}

/// Rule one's negative pin: the engine never renames over or writes a live path in place — no
/// `std::fs::rename`, `std::fs::write`, `fs::copy` onto a live path or `remove_file` of a live path
/// exists in this file's production code (the only renames are `swap_paths` and the no-replace
/// placement; `remove_file` only removes this mechanism's own non-slot `.incoming-*` copies).
#[test]
fn rule_one_never_renames_over_or_writes_live_in_place() {
    let src = include_str!("apply.rs");
    let prod = &src[..src.find("#[cfg(test)]\n#[path = \"apply_tests.rs\"]").unwrap()];
    assert!(!prod.contains("std::fs::rename("), "a rename-over exists");
    assert!(!prod.contains("std::fs::write("), "an in-place write exists");
    for (at, _) in prod.match_indices("std::fs::remove_file(") {
        let arg = &prod[at + "std::fs::remove_file(".len()..];
        let arg = &arg[..arg.find(')').unwrap()];
        assert!(
            ["c", "&copy", "&src_file"].contains(&arg) || arg.starts_with("c)"),
            "remove_file of something else: {arg}"
        );
    }
}

/// OQ-D7 / H6(1): another writer's bytes displaced instead of the expected ones are re-lived as a
/// COPY by the same rule — but a writer caught between truncating and writing (an empty file that
/// fails the file's own check) is never re-lived: the verified content stays live, superseded.
#[cfg(target_os = "macos")]
#[test]
fn displaced_garbage_is_never_relived() {
    for (other, check_passes, relived) in [(b"VALID-OTHER\n".to_vec(), true, true), (Vec::new(), false, false)] {
        let r = rig("h6");
        r.put("schedule.json", b"{\"jobs\":[]}\n", 0o644);
        let (src, st) = r.source("s.json", b"{\"jobs\":[1]}\n", 0o644);
        let inj = Inject::new(Point::AfterRecord, Action::ReplaceLive(other.clone()), None, &r.pack);
        let own = move |_rel: &str, _p: &Path| Some(check_passes);
        let env = OpEnv { own_check: &own, ..r.env(&inj, ApplyForm::Exchange) };
        let d = run_op(&env, &OpSpec {
            id: rid("R-2"),
            rel: "schedule.json".into(),
            kind: "apply".into(),
            expected: r.live("schedule.json"),
            incoming: st,
            source: Some(src),
            generation: None,
        })
        .unwrap();
        assert!(matches!(d.outcome, ApplyOutcome::Superseded(_)), "{:?}", d.outcome);
        let live = std::fs::read(r.pack.join("schedule.json")).unwrap();
        if relived {
            assert_eq!(live, other, "the other writer's bytes are live again (a copy)");
        } else {
            assert_eq!(live, b"{\"jobs\":[1]}\n", "the verified content stays live");
        }
        assert!(r.vault_bytes().contains(&other), "both versions are in the vault");
    }
}

// ───────────────────────────── ledger fixtures ─────────────────────────────

pub(crate) fn record(r: &Rig, id: &RepairId, class: Classification, files: Vec<FileChange>, state: RepairState) -> RepairRecord {
    std::fs::create_dir_all(ledger::repair_dir_in(&r.root, id)).unwrap();
    let rec = RepairRecord {
        schema: 1,
        id: id.clone(),
        created_at: 1.0,
        trigger: Trigger::OwnerOrder,
        symptom: String::new(),
        cause: String::new(),
        classification: class,
        files,
        worker_record: None,
        verify: None,
        audit: None,
        audit_due_at: None,
        approval: None,
        owner_token_sha256: None,
        attempt: 1,
        methods: Vec::new(),
        resubmits: Vec::new(),
        pending_request: None,
        lock: None,
        state,
        intake_signatures: Vec::new(),
        owner_original: false,
        origin_lane: r.lane.clone(),
        resolves: None,
        committed: Vec::new(),
        fixer_seats: Vec::new(),
        checks: Vec::new(),
        reapplies: None,
    };
    ledger::write_record_in(&r.root, &rec).unwrap();
    rec
}

/// A committed GENERAL repair of `rel` in the origin lane: the `begin` copy of `pre`, the worker's
/// live edit to `post`, the record in probation, the lane row with the applied state.
pub(crate) fn committed_general(r: &Rig, id: &RepairId, rel: &str, pre: Option<&[u8]>, post: Option<&[u8]>) -> RepairRecord {
    if let Some(b) = pre {
        r.put(rel, b, 0o755);
    }
    let pre_img = ledger::copy_preimage_in(&r.root, id, &r.lane, rel, &r.pack.join(rel), 1.0).unwrap();
    match post {
        Some(b) => {
            r.put(rel, b, 0o755);
        }
        None => {
            let _ = std::fs::remove_file(r.pack.join(rel));
        }
    }
    let post_state = r.live(rel);
    let mut rec = record(
        r,
        id,
        Classification::General,
        vec![FileChange {
            rel: rel.into(),
            kind: ChangeKind::Edit,
            pre: pre_img,
            post: post_state.clone(),
            diff: None,
        }],
        RepairState::Probation,
    );
    rec.committed = vec![(rel.to_string(), post_state.canon())];
    ledger::write_record_in(&r.root, &rec).unwrap();
    lane_row(r, id, vec![(rel.to_string(), post_state.canon())], Vec::new());
    rec
}

pub(crate) fn lane_row(r: &Rig, id: &RepairId, applied: Vec<(String, String)>, timings: Vec<CheckTiming>) {
    let gen = pack_generation(&r.pack);
    ledger::write_lane_in(
        &r.root,
        &LaneRecord {
            schema: 1,
            repair: id.clone(),
            lane: r.lane.clone(),
            state: LaneState::Verified,
            probation: Some(Probation {
                pack_generation: gen.version,
                pro_revision: gen.pro_revision,
                applied_at: 1_800_000_000.0 - 3600.0,
                expected: applied.clone(),
                closed: None,
                check_timings: timings,
                check_timeouts: Vec::new(),
            }),
            merge: None,
            merge_history: Vec::new(),
            approval_ref: None,
            conflict_live: Vec::new(),
            applied,
            applied_history: Vec::new(),
            merge_base: Vec::new(),
            merge_lock: None,
            merge_attempt: 0,
            updated_at: 1.0,
        },
    )
    .unwrap();
}

fn no_hooks(_: &RepairId, _: &LaneKey, _: HookStep) -> Result<ReconcileReport, RepairError> {
    Ok(ReconcileReport { hooks: Vec::new() })
}

#[cfg(target_os = "macos")]
#[test]
fn revert_puts_the_pre_image_back_and_keeps_the_repaired_copy() {
    let r = rig("revert");
    let id = rid("R-3");
    committed_general(&r, &id, "hooks/x.sh", Some(b"PRE\n"), Some(b"POST\n"));
    let env = r.env(&NoFaults, ApplyForm::Exchange);
    let rep = revert_in(&env, &id, RevertCaller::Human, &no_hooks, &no_repair_lock).unwrap();
    assert_eq!(rep.outcome, ApplyOutcome::Applied, "{rep:?}");
    assert_eq!(std::fs::read(r.pack.join("hooks/x.sh")).unwrap(), b"PRE\n");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(r.pack.join("hooks/x.sh")).unwrap().permissions().mode() & 0o7777, 0o755, "F3: the exec bit survives the revert");
    }
    assert!(r.vault_bytes().contains(&b"POST\n".to_vec()), "the repaired copy is kept");
    assert_eq!(ledger::read_record_in(&r.root, &id).unwrap().unwrap().state, RepairState::Reverted);
    let row = ledger::read_lane_in(&r.root, &id, &r.lane).unwrap().unwrap();
    assert!(row.probation.unwrap().closed.unwrap().starts_with("되돌려짐"));
}

/// Created file → moved into a slot; deleted file → its pre-image copy placed (and the
/// tombstone removed); a live file that is not the applied result → the whole revert superseded,
/// nothing changed.
#[cfg(target_os = "macos")]
#[test]
fn revert_of_created_deleted_and_moved_on_files() {
    let r = rig("revert2");
    let created = rid("R-4");
    committed_general(&r, &created, "hooks/new.sh", None, Some(b"NEW\n"));
    let env = r.env(&NoFaults, ApplyForm::Exchange);
    assert_eq!(revert_in(&env, &created, RevertCaller::Human, &no_hooks, &no_repair_lock).unwrap().outcome, ApplyOutcome::Applied);
    assert_eq!(r.live("hooks/new.sh"), TargetState::Absent);
    assert!(r.vault_bytes().contains(&b"NEW\n".to_vec()));
    let deleted = rid("R-5");
    committed_general(&r, &deleted, "hooks/gone.sh", Some(b"GONE\n"), None);
    crate::repair::tombstones::record_in(&r.root, &r.lane, &crate::repair::tombstones::Tombstone {
        rel: "hooks/gone.sh".into(),
        kind: crate::repair::tombstones::TombstoneKind::File,
        repair: deleted.clone(),
    })
    .unwrap();
    assert_eq!(revert_in(&env, &deleted, RevertCaller::Human, &no_hooks, &no_repair_lock).unwrap().outcome, ApplyOutcome::Applied);
    assert_eq!(std::fs::read(r.pack.join("hooks/gone.sh")).unwrap(), b"GONE\n");
    assert!(crate::repair::tombstones::load_in(&r.root, &r.lane).unwrap().is_empty());
    let moved = rid("R-6");
    committed_general(&r, &moved, "hooks/m.sh", Some(b"A\n"), Some(b"B\n"));
    r.put("hooks/m.sh", b"UPDATE-WROTE\n", 0o755);
    let rep = revert_in(&env, &moved, RevertCaller::Human, &no_hooks, &no_repair_lock).unwrap();
    assert!(matches!(rep.outcome, ApplyOutcome::Superseded(_)));
    assert_eq!(std::fs::read(r.pack.join("hooks/m.sh")).unwrap(), b"UPDATE-WROTE\n", "nothing changed");
    assert_eq!(ledger::read_record_in(&r.root, &moved).unwrap().unwrap().state, RepairState::Superseded);
}

/// Windows (run here through the form switch): automatic return is OFF; a human / worker revert of
/// a GENERAL repair is deposit-first; a boot-critical one is refused (pending).
#[cfg(target_os = "macos")]
#[test]
fn windows_form_auto_off_manual_deposit_first_boot_critical_pending() {
    let r = rig("win");
    let id = rid("R-7");
    committed_general(&r, &id, "bin/t.py", Some(b"PRE\n"), Some(b"POST\n"));
    let env = r.env(&NoFaults, ApplyForm::DepositFirst);
    assert!(matches!(revert_in(&env, &id, RevertCaller::DaemonAuto, &no_hooks, &no_repair_lock), Err(RepairError::PlatformPending(_))));
    assert_eq!(std::fs::read(r.pack.join("bin/t.py")).unwrap(), b"POST\n");
    let rep = revert_in(&env, &id, RevertCaller::TicketedWorker { seat: 4 }, &no_hooks, &no_repair_lock).unwrap();
    assert_eq!(rep.outcome, ApplyOutcome::Applied);
    assert_eq!(std::fs::read(r.pack.join("bin/t.py")).unwrap(), b"PRE\n");
    let (ops, _) = wal::list_in(&r.root, &id);
    assert!(ops.iter().any(|o| o.form == "deposit-first" && o.outcome.as_deref() == Some("applied")));
    let bc = rid("R-8");
    let mut rec = committed_general(&r, &bc, "hooks/s.sh", Some(b"S\n"), Some(b"T\n"));
    rec.classification = Classification::BootCritical;
    ledger::write_record_in(&r.root, &rec).unwrap();
    assert!(matches!(revert_in(&env, &bc, RevertCaller::Human, &no_hooks, &no_repair_lock), Err(RepairError::PlatformPending(_))));
}

/// OQ-D8: interrupted between the two moves → the next run RESUMES when the staged copy and the
/// generation are unchanged, and ABANDONS (a copy of the deposit placed back) when the generation
/// moved; an occupied path stops it, nothing deleted.
#[cfg(target_os = "macos")]
#[test]
fn windows_interruption_between_the_two_moves_resume_or_abandon() {
    for (bump, occupy, want) in [(false, false, b"NEW\n".to_vec()), (true, false, b"OLD\n".to_vec()), (false, true, b"THIRD\n".to_vec())] {
        let r = rig("oqd8");
        r.put("bin/w.py", b"OLD\n", 0o644);
        let (src, st) = r.source("w.py", b"NEW\n", 0o644);
        let inj = Inject::new(Point::AfterFirstMove, Action::Stop, None, &r.pack);
        let env = r.env(&inj, ApplyForm::DepositFirst);
        let first = run_op(&env, &OpSpec { id: rid("R-9"), rel: "bin/w.py".into(), kind: "revert".into(), expected: r.live("bin/w.py"), incoming: st, source: Some(src), generation: None });
        assert!(first.is_err());
        assert_eq!(r.live("bin/w.py"), TargetState::Absent, "the path is empty between the two moves");
        if bump {
            std::fs::write(r.pack.join(".pack-version"), "0.15.1\n").unwrap();
        }
        if occupy {
            r.put("bin/w.py", b"THIRD\n", 0o644);
        }
        let calm = NoFaults;
        resume_interrupted_in(&r.env(&calm, ApplyForm::DepositFirst)).unwrap();
        assert_eq!(std::fs::read(r.pack.join("bin/w.py")).unwrap(), want);
        assert!(r.vault_bytes().contains(&b"OLD\n".to_vec()), "the deposit is never deleted");
        assert!(wal::unfinished_for_lane(&r.root, &r.lane).is_empty());
    }
}

/// The staged init-pack swap under an operation (the whole pack directory replaced between the
/// record and the exchange) → the second install: the vault is outside the pack, every version
/// survives; the operation closes superseded (worker review).
#[cfg(target_os = "macos")]
#[test]
fn staged_pack_swap_during_an_operation_then_second_install() {
    let r = rig("swap");
    r.put("hooks/a.sh", b"V1\n", 0o755);
    let (src, st) = r.source("a.sh", b"REPAIRED\n", 0o755);
    struct Swap<'a>(&'a Rig, Cell<bool>);
    impl Faults for Swap<'_> {
        fn at(&self, p: Point, _live: &Path, _s: Option<&Slot>) -> bool {
            if p == Point::AfterRecord && !self.1.get() {
                self.1.set(true);
                let prev = self.0.base.join("pack.prev");
                std::fs::rename(&self.0.pack, &prev).unwrap();
                std::fs::create_dir_all(self.0.pack.join("hooks")).unwrap();
                std::fs::write(self.0.pack.join(".pack-version"), "0.15.0\n").unwrap();
                std::fs::write(self.0.pack.join("hooks/a.sh"), b"V2-INSTALLED\n").unwrap();
            }
            false
        }
    }
    let f = Swap(&r, Cell::new(false));
    let env = r.env(&f, ApplyForm::Exchange);
    let d = run_op(&env, &OpSpec { id: rid("R-10"), rel: "hooks/a.sh".into(), kind: "apply".into(), expected: r.live("hooks/a.sh"), incoming: st, source: Some(src), generation: None }).unwrap();
    assert!(matches!(d.outcome, ApplyOutcome::Superseded(_)), "{:?}", d.outcome);
    // second install: the pack swapped again (pack.prev deleted by the installer)
    let _ = std::fs::remove_dir_all(r.base.join("pack.prev"));
    let vault = r.vault_bytes();
    assert!(vault.contains(&b"V2-INSTALLED\n".to_vec()), "the bytes live at the exchange are in the vault");
    assert!(vault.contains(&b"REPAIRED\n".to_vec()));
}

// ───────────────────────────── F9 · R2 · P6 ─────────────────────────────

pub(crate) fn approval(fp: &str) -> ApprovalResult {
    ApprovalResult { request_id: "rqr-1".into(), fingerprint_sha256: fp.into(), decided_at: 2.0, approved: true }
}

/// An approved boot-critical repair of `rel` applied in the origin lane through `apply_candidate_in`.
pub(crate) fn approved_and_applied(r: &Rig, id: &RepairId, rel: &str, pre: &[u8], post: &[u8]) -> RepairRecord {
    r.put(rel, pre, 0o755);
    let pre_img = ledger::copy_preimage_in(&r.root, id, &r.lane, rel, &r.pack.join(rel), 1.0).unwrap();
    let staging = ledger::staging_dir_in(&r.root, id);
    std::fs::create_dir_all(staging.join(rel).parent().unwrap()).unwrap();
    std::fs::write(staging.join(rel), post).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(staging.join(rel), std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let st = TargetState::read(&staging.join(rel)).unwrap();
    let fp = crate::repair::types::fingerprint(&[(rel.to_string(), st.clone())]);
    let mut rec = record(r, id, Classification::BootCritical, vec![FileChange { rel: rel.into(), kind: ChangeKind::Edit, pre: pre_img, post: st, diff: None }], RepairState::AwaitingApproval);
    rec.approval = Some(approval(&fp));
    rec.verify = Some(Attestation { seat: 2, role: "reviewer-codex".into(), verdict: "pass".into(), note: String::new(), at: 1.0, fingerprint_sha256: fp });
    ledger::write_record_in(&r.root, &rec).unwrap();
    let env = r.env(&NoFaults, ApplyForm::Exchange);
    let rep = apply_candidate_in(
        &env,
        id,
        &[CandidateFile { rel: rel.into(), staged: staging.join(rel), mode: 0o755 }],
        &|_, _, _| Ok(()),
        &pass_check,
        &pass_ref,
        &no_repair_lock,
    )
    .unwrap();
    assert_eq!(rep.outcome, ApplyOutcome::Applied, "{rep:?}");
    ledger::read_record_in(&r.root, id).unwrap().unwrap()
}

pub(crate) fn pass_check(_: &str, _: &Path, _: Duration) -> Result<CheckOutcome, RepairError> {
    Ok(CheckOutcome { result: CheckResult::Pass, ms: 100 })
}
pub(crate) fn pass_ref(_: CheckKind, _: Duration) -> Result<CheckOutcome, RepairError> {
    Ok(CheckOutcome { result: CheckResult::Pass, ms: 50 })
}

/// F9: a planted ledger row is NOT an approved copy; the agreeing triple is; any member missing or
/// mismatched → not.
#[cfg(target_os = "macos")]
#[test]
fn f9_triple_only() {
    let r = rig("f9");
    let id = rid("R-11");
    approved_and_applied(&r, &id, "hooks/b.sh", b"PRE\n", b"APPROVED\n");
    assert!(approved_copy_of_current_generation_in(&r.root, &r.pack, &r.lane, "hooks/b.sh").unwrap());
    // a planted row alone (record + approval + row, no operation record, no deposit)
    let planted = rid("R-12");
    r.put("hooks/p.sh", b"PLANTED\n", 0o755);
    let mut rec = record(&r, &planted, Classification::BootCritical, vec![FileChange { rel: "hooks/p.sh".into(), kind: ChangeKind::Edit, pre: PreImage::Absent, post: r.live("hooks/p.sh"), diff: None }], RepairState::Probation);
    rec.approval = Some(approval("x"));
    ledger::write_record_in(&r.root, &rec).unwrap();
    lane_row(&r, &planted, vec![("hooks/p.sh".into(), r.live("hooks/p.sh").canon())], Vec::new());
    assert!(!approved_copy_of_current_generation_in(&r.root, &r.pack, &r.lane, "hooks/p.sh").unwrap());
    // a member broken: the deposit's sealed copy altered → not approved
    let (ops, _) = wal::list_in(&r.root, &id);
    let op = ops.iter().find(|o| o.kind == "apply").unwrap();
    let slot = Slot::at(op.slot.as_ref().unwrap(), "hooks/b.sh");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(slot.sealed(), std::fs::Permissions::from_mode(0o644)).unwrap();
    }
    std::fs::write(slot.sealed(), b"TAMPERED").unwrap();
    assert!(!approved_copy_of_current_generation_in(&r.root, &r.pack, &r.lane, "hooks/b.sh").unwrap());
}

/// R2: true for the origin's finished apply whose states reproduce the fingerprint — also after the
/// origin hand-edits the file (live bytes are never read); a superseded operation → false.
#[cfg(target_os = "macos")]
#[test]
fn origin_approval_triple_rules() {
    let r = rig("r2");
    let id = rid("R-13");
    let rec = approved_and_applied(&r, &id, "hooks/c.sh", b"PRE\n", b"OK\n");
    let a = rec.approval.clone().unwrap();
    assert!(origin_approval_triple_in(&r.root, &id, &a, &r.lane));
    r.put("hooks/c.sh", b"HAND-EDITED\n", 0o755);
    assert!(origin_approval_triple_in(&r.root, &id, &a, &r.lane), "the origin's live bytes play no part");
    let mut wrong = a.clone();
    wrong.fingerprint_sha256 = "00".repeat(32);
    assert!(!origin_approval_triple_in(&r.root, &id, &wrong, &r.lane));
    let (ops, _) = wal::list_in(&r.root, &id);
    let mut op = ops.into_iter().find(|o| o.kind == "apply").unwrap();
    op.outcome = Some("superseded".into());
    wal::write_in(&r.root, &op).unwrap();
    assert!(!origin_approval_triple_in(&r.root, &id, &a, &r.lane));
}

/// P6: while in force the pre-image deposit and the approved-apply deposit are needed; a deposit of
/// a superseded operation is not; after the repair leaves force none is.
#[cfg(target_os = "macos")]
#[test]
fn slots_needed_in_force_rules() {
    let r = rig("p6");
    let id = rid("R-14");
    let rec = approved_and_applied(&r, &id, "hooks/d.sh", b"PRE\n", b"NEW\n");
    let needed = slots_needed_in_force_in(&r.root, &id).unwrap();
    let pre_slot = match &rec.files[0].pre { PreImage::Slot { slot_dir, .. } => slot_dir.clone(), _ => unreachable!() };
    assert!(needed.contains(&pre_slot));
    let (ops, _) = wal::list_in(&r.root, &id);
    let apply_slot = ops.iter().find(|o| o.kind == "apply").unwrap().slot.clone().unwrap();
    assert!(needed.contains(&apply_slot));
    let mut done = rec.clone();
    done.state = RepairState::Reverted;
    ledger::write_record_in(&r.root, &done).unwrap();
    assert!(slots_needed_in_force_in(&r.root, &id).unwrap().is_empty());
    assert!(slots_needed_in_force_in(&r.root, &rid("R-99")).is_err(), "unreadable → Err (refuse every slot)");
}

// ───────────────────────────── the approved apply ─────────────────────────────

/// The candidate is copied ONCE into a slot and the SLOT is what gets fingerprinted and exchanged: a
/// staging changed after the copy does not reach live; a staging/approval mismatch applies nothing
/// and voids the approval (back to 진행 중); Q3 holds after the apply.
#[cfg(target_os = "macos")]
#[test]
fn apply_uses_the_slot_bytes_and_refuses_a_mismatch() {
    let r = rig("apply");
    let id = rid("R-15");
    let rec = approved_and_applied(&r, &id, "hooks/e.sh", b"PRE\n", b"APPROVED\n");
    assert_eq!(rec.state, RepairState::Probation);
    assert_eq!(std::fs::read(r.pack.join("hooks/e.sh")).unwrap(), b"APPROVED\n");
    let fp = rec.approval.as_ref().unwrap().fingerprint_sha256.clone();
    assert_eq!(crate::repair::types::fingerprint(&[("hooks/e.sh".into(), r.live("hooks/e.sh"))]), fp, "Q3");
    let row = ledger::read_lane_in(&r.root, &id, &r.lane).unwrap().unwrap();
    assert_eq!(row.approval_ref.as_deref(), Some("rqr-1"));
    assert_eq!(row.probation.as_ref().unwrap().check_timings.len(), 1, "K4 durations recorded");
    // a mismatch: the staging no longer matches the approval
    let id2 = rid("R-16");
    r.put("hooks/f.sh", b"PRE2\n", 0o755);
    let pre_img = ledger::copy_preimage_in(&r.root, &id2, &r.lane, "hooks/f.sh", &r.pack.join("hooks/f.sh"), 1.0).unwrap();
    let staging = ledger::staging_dir_in(&r.root, &id2).join("hooks/f.sh");
    std::fs::create_dir_all(staging.parent().unwrap()).unwrap();
    std::fs::write(&staging, b"SWAPPED-AFTER-APPROVAL\n").unwrap();
    let mut rec2 = record(&r, &id2, Classification::BootCritical, vec![FileChange { rel: "hooks/f.sh".into(), kind: ChangeKind::Edit, pre: pre_img, post: TargetState::Absent, diff: None }], RepairState::AwaitingApproval);
    rec2.approval = Some(approval(&"ab".repeat(32)));
    ledger::write_record_in(&r.root, &rec2).unwrap();
    let env = r.env(&NoFaults, ApplyForm::Exchange);
    let rep = apply_candidate_in(&env, &id2, &[CandidateFile { rel: "hooks/f.sh".into(), staged: staging.clone(), mode: 0o755 }], &|_, _, _| Ok(()), &pass_check, &pass_ref, &no_repair_lock).unwrap();
    assert!(matches!(rep.outcome, ApplyOutcome::Declined(_)));
    assert_eq!(std::fs::read(r.pack.join("hooks/f.sh")).unwrap(), b"PRE2\n", "nothing applied");
    let after = ledger::read_record_in(&r.root, &id2).unwrap().unwrap();
    assert_eq!(after.state, RepairState::InProgress);
    assert!(after.approval.is_none(), "AY5: the approval is void");
}

/// The apply-time screen (AY5 / CM) runs over the SLOT bytes; a refusal or a not-measurable probe
/// declines the apply (approval void, nothing live changed). Test double behind the same signature.
#[cfg(target_os = "macos")]
#[test]
fn apply_time_screen_declines_and_sees_the_slot_bytes() {
    let r = rig("screen");
    let id = rid("R-17");
    r.put("hooks/g.sh", b"PRE\n", 0o755);
    let pre_img = ledger::copy_preimage_in(&r.root, &id, &r.lane, "hooks/g.sh", &r.pack.join("hooks/g.sh"), 1.0).unwrap();
    let staging = ledger::staging_dir_in(&r.root, &id).join("hooks/g.sh");
    std::fs::create_dir_all(staging.parent().unwrap()).unwrap();
    std::fs::write(&staging, b"CAND\n").unwrap();
    let st = TargetState::read(&staging).unwrap();
    let fp = crate::repair::types::fingerprint(&[("hooks/g.sh".into(), st.clone())]);
    let mut rec = record(&r, &id, Classification::ProtectionRelated, vec![FileChange { rel: "hooks/g.sh".into(), kind: ChangeKind::Edit, pre: pre_img, post: st, diff: None }], RepairState::AwaitingApproval);
    rec.approval = Some(approval(&fp));
    ledger::write_record_in(&r.root, &rec).unwrap();
    let seen = RefCell::new(None::<Vec<u8>>);
    let screen = |_: &RepairId, c: &crate::repair::screen::ScreenCandidate, _: &Path| -> Result<(), String> {
        *seen.borrow_mut() = std::fs::read(c.files.join("hooks/g.sh")).ok();
        assert_eq!(c.targets.len(), 1);
        Err("보호장치 검사가 이 레인에서 적용을 막았습니다 — 측정 불가: G-1".into())
    };
    let env = r.env(&NoFaults, ApplyForm::Exchange);
    let rep = apply_candidate_in(&env, &id, &[CandidateFile { rel: "hooks/g.sh".into(), staged: staging, mode: 0o644 }], &screen, &pass_check, &pass_ref, &no_repair_lock).unwrap();
    assert!(matches!(rep.outcome, ApplyOutcome::Declined(ref m) if m.contains("측정 불가")));
    assert_eq!(seen.borrow().as_deref(), Some(&b"CAND\n"[..]), "the screen saw the slot's bytes");
    assert_eq!(std::fs::read(r.pack.join("hooks/g.sh")).unwrap(), b"PRE\n");
    assert!(ledger::read_record_in(&r.root, &id).unwrap().unwrap().approval.is_none());
}

/// The slots are fingerprinted BEFORE the screen, which may run for minutes: bytes written into a
/// slot during the screen are never applied — the exchange re-reads the slot and declines, the live
/// file keeps its bytes and the approval is void.
#[cfg(target_os = "macos")]
#[test]
fn slot_changed_during_the_screen_is_never_applied() {
    use std::os::unix::fs::PermissionsExt;
    let r = rig("screentamper");
    let id = rid("R-19");
    r.put("hooks/t.sh", b"PRE\n", 0o755);
    let pre_img = ledger::copy_preimage_in(&r.root, &id, &r.lane, "hooks/t.sh", &r.pack.join("hooks/t.sh"), 1.0).unwrap();
    let staging = ledger::staging_dir_in(&r.root, &id).join("hooks/t.sh");
    std::fs::create_dir_all(staging.parent().unwrap()).unwrap();
    std::fs::write(&staging, b"APPROVED\n").unwrap();
    let st = TargetState::read(&staging).unwrap();
    let fp = crate::repair::types::fingerprint(&[("hooks/t.sh".into(), st.clone())]);
    let mut rec = record(&r, &id, Classification::ProtectionRelated, vec![FileChange { rel: "hooks/t.sh".into(), kind: ChangeKind::Edit, pre: pre_img, post: st, diff: None }], RepairState::AwaitingApproval);
    rec.approval = Some(approval(&fp));
    ledger::write_record_in(&r.root, &rec).unwrap();
    let tampered = RefCell::new(0usize);
    // the screen double passes, but meanwhile another writer rewrites every slot's payload
    let screen = |_: &RepairId, c: &crate::repair::screen::ScreenCandidate, _: &Path| -> Result<(), String> {
        let vault_dir = c.files.parent().unwrap().to_path_buf();
        for e in std::fs::read_dir(&vault_dir).unwrap().flatten() {
            if e.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            let payload = e.path().join("payload").join("t.sh");
            if payload.is_file() {
                std::fs::set_permissions(&payload, std::fs::Permissions::from_mode(0o644)).unwrap();
                std::fs::write(&payload, b"UNAPPROVED\n").unwrap();
                *tampered.borrow_mut() += 1;
            }
        }
        Ok(())
    };
    let env = r.env(&NoFaults, ApplyForm::Exchange);
    let rep = apply_candidate_in(&env, &id, &[CandidateFile { rel: "hooks/t.sh".into(), staged: staging, mode: 0o644 }], &screen, &pass_check, &pass_ref, &no_repair_lock).unwrap();
    assert!(*tampered.borrow() >= 1, "the double reached the slot");
    assert!(matches!(rep.outcome, ApplyOutcome::Declined(ref m) if m.contains("바뀌었습니다")), "{:?}", rep.outcome);
    assert_eq!(std::fs::read(r.pack.join("hooks/t.sh")).unwrap(), b"PRE\n", "the live file keeps its bytes");
    assert!(ledger::read_record_in(&r.root, &id).unwrap().unwrap().approval.is_none(), "the approval is void");
}

/// The production screen step: a recorded verdict that holds is reused (nothing runs); otherwise the
/// screen runs and its error / refusal declines. Until T5b's bodies land the read is "not built" →
/// declined (fail closed).
#[test]
fn production_screen_step_fails_closed_without_the_screen() {
    let r = rig("prodscreen");
    let step = production_screen_step(&r.root);
    let cand = crate::repair::screen::ScreenCandidate { files: r.src.clone(), targets: Vec::new() };
    let res = step(&rid("R-1"), &cand, &r.pack);
    if crate::repair::screen::read_verdict_in(&r.root, &rid("R-1")).is_err() {
        assert!(res.is_err(), "a screen that cannot run must decline");
    }
}

/// The file's own check right after the apply fails (a timeout counts as failure — B3) → that ONE
/// repair is put back by rule one, recorded 되돌려짐.
#[cfg(target_os = "macos")]
#[test]
fn apply_time_check_failure_puts_the_repair_back() {
    let r = rig("applycheck");
    let id = rid("R-18");
    r.put("hooks/h.sh", b"PRE\n", 0o755);
    let pre_img = ledger::copy_preimage_in(&r.root, &id, &r.lane, "hooks/h.sh", &r.pack.join("hooks/h.sh"), 1.0).unwrap();
    let staging = ledger::staging_dir_in(&r.root, &id).join("hooks/h.sh");
    std::fs::create_dir_all(staging.parent().unwrap()).unwrap();
    std::fs::write(&staging, b"BROKEN\n").unwrap();
    let st = TargetState::read(&staging).unwrap();
    let fp = crate::repair::types::fingerprint(&[("hooks/h.sh".into(), st.clone())]);
    let mut rec = record(&r, &id, Classification::BootCritical, vec![FileChange { rel: "hooks/h.sh".into(), kind: ChangeKind::Edit, pre: pre_img, post: st, diff: None }], RepairState::AwaitingApproval);
    rec.approval = Some(approval(&fp));
    ledger::write_record_in(&r.root, &rec).unwrap();
    let timeout = |_: &str, _: &Path, _: Duration| Ok(CheckOutcome { result: CheckResult::Timeout, ms: 60_000 });
    let env = r.env(&NoFaults, ApplyForm::Exchange);
    let rep = apply_candidate_in(&env, &id, &[CandidateFile { rel: "hooks/h.sh".into(), staged: staging, mode: 0o644 }], &|_, _, _| Ok(()), &timeout, &pass_ref, &no_repair_lock).unwrap();
    assert!(matches!(rep.outcome, ApplyOutcome::Declined(_)));
    assert_eq!(std::fs::read(r.pack.join("hooks/h.sh")).unwrap(), b"PRE\n", "put back");
    assert!(r.vault_bytes().contains(&b"BROKEN\n".to_vec()));
    assert_eq!(ledger::read_record_in(&r.root, &id).unwrap().unwrap().state, RepairState::Reverted);
}

/// K2 alias case: a target that is the SAME file as a listed row under another rel (a hard link) is
/// refused at apply (AW3 (d), T5a's `refuse_identity_alias`).
#[cfg(unix)]
#[test]
fn apply_refuses_an_alias_of_a_listed_file() {
    let Some(row) = crate::repair::boot_critical::entries().first() else {
        return; // the compiled list is empty in this build — nothing to alias
    };
    let r = rig("alias");
    r.put(row.rel, b"LISTED\n", 0o755);
    std::fs::create_dir_all(r.pack.join("x")).unwrap();
    std::fs::hard_link(r.pack.join(row.rel), r.pack.join("x/alias.sh")).unwrap();
    let id = rid("R-19");
    let staging = ledger::staging_dir_in(&r.root, &id).join("x/alias.sh");
    std::fs::create_dir_all(staging.parent().unwrap()).unwrap();
    std::fs::write(&staging, b"EVIL\n").unwrap();
    let mut rec = record(&r, &id, Classification::BootCritical, vec![FileChange { rel: "x/alias.sh".into(), kind: ChangeKind::Edit, pre: PreImage::Absent, post: TargetState::Absent, diff: None }], RepairState::AwaitingApproval);
    rec.approval = Some(approval("ff"));
    ledger::write_record_in(&r.root, &rec).unwrap();
    let env = r.env(&NoFaults, ApplyForm::Exchange);
    let rep = apply_candidate_in(&env, &id, &[CandidateFile { rel: "x/alias.sh".into(), staged: staging, mode: 0o644 }], &|_, _, _| Ok(()), &pass_check, &pass_ref, &no_repair_lock).unwrap();
    assert!(matches!(rep.outcome, ApplyOutcome::Declined(ref m) if m.contains("AW3")), "{rep:?}");
    assert_eq!(std::fs::read(r.pack.join(row.rel)).unwrap(), b"LISTED\n");
}

// ───────────────────────────── the hook step ─────────────────────────────

fn hook_record(r: &Rig, id: &RepairId, event: &str, script: &str) {
    record(r, id, Classification::General, vec![FileChange {
        rel: format!("settings:{event}:{script}"),
        kind: ChangeKind::HookUnregister { event: event.into(), script: script.into() },
        pre: PreImage::Absent,
        post: TargetState::Absent,
        diff: None,
    }], RepairState::Probation);
}

fn report_json(rows: &[(&str, &str, &str, &[(&str, &str)])]) -> String {
    let hooks: Vec<serde_json::Value> = rows.iter().map(|(e, s, d, ts)| serde_json::json!({
        "event": e, "script": s, "lifeline": false, "desired": d,
        "targets": ts.iter().map(|(p, st)| serde_json::json!({"path": p, "state": st})).collect::<Vec<_>>(),
    })).collect();
    serde_json::json!({"hooks": hooks, "errors": []}).to_string()
}

/// Y1: the record FIRST (targets of the dry run, matcher / timeout as found), the tombstone, the
/// registrar, finished; a symlinked target is listed refused and does not fail it.
#[test]
fn hook_step_records_first_and_judges_only_named_hooks() {
    let r = rig("hook");
    let id = rid("R-20");
    hook_record(&r, &id, "Stop", "x.sh");
    let settings = r.base.join("settings.json");
    std::fs::write(&settings, serde_json::json!({"hooks": {"Stop": [{"matcher": "", "hooks": [{"type": "command", "command": "bash \"$HOME/.cys/pack/hooks/x.sh\"", "timeout": 15}]}]}}).to_string()).unwrap();
    let sp = settings.to_string_lossy().to_string();
    let calls = RefCell::new(Vec::new());
    let registrar = |_: &Path, dry: bool| -> Result<String, RepairError> {
        calls.borrow_mut().push(dry);
        let state = if dry { "registered" } else { "unregistered" };
        Ok(report_json(&[("Stop", "x.sh", "unregistered", &[(sp.as_str(), state), ("/link/settings.json", "refused:symlink")]), ("Stop", "other.sh", "registered", &[(sp.as_str(), "registered")])]))
    };
    let rep = hook_lines_step_in(&r.root, &r.pack, &id, &r.lane, HookStep::Unregister, &registrar, 5.0).unwrap();
    assert_eq!(*calls.borrow(), vec![true, false], "a dry run for the record, then the real run");
    assert!(rep.hooks.iter().any(|h| h.script == "x.sh"));
    let (ops, _) = wal::list_in(&r.root, &id);
    let op = ops.iter().find(|o| o.kind == "hook-unregister").unwrap();
    assert!(op.finished);
    let t = &op.hooks[0].targets[0];
    assert_eq!((t.matcher.as_deref(), t.timeout), (Some(""), Some(15)), "matcher / timeout kept as found");
    let ts = crate::repair::tombstones::load_in(&r.root, &r.lane).unwrap();
    assert_eq!(ts.len(), 1);
    assert_eq!(ts[0].rel, "settings:Stop:x.sh");
}

/// AW6: the registrar's names are matched EXACTLY — a case variant matches no row, so the step
/// fails (the commit marks 충돌) and nothing is reported unregistered.
#[test]
fn hook_step_case_variant_unregisters_nothing() {
    let report = parse_reconcile(&report_json(&[("Stop", "guard.sh", "registered", &[("/s.json", "registered")])])).unwrap();
    assert!(judge_named(&report, &[("Stop".into(), "Guard.sh".into())], "unregistered").is_err());
    assert!(judge_named(&report, &[("Stop".into(), "guard.sh".into())], "registered").is_ok());
}

/// The registrar timing out → an error (the caller leaves the row 충돌; `recheck` runs it again) and
/// the record is closed — no endless retry inside the step. A crash after the record and before the
/// tombstone → the next tick finishes it.
#[test]
fn hook_step_timeout_and_resume_after_a_crash() {
    let r = rig("hook2");
    let id = rid("R-21");
    hook_record(&r, &id, "Stop", "y.sh");
    let slow = |_: &Path, dry: bool| -> Result<String, RepairError> {
        if dry { Ok(report_json(&[("Stop", "y.sh", "registered", &[("/s.json", "registered")])])) } else { Err(RepairError::CheckTimeout("등록기가 60초 안에 끝나지 않아 멈췄습니다".into())) }
    };
    assert!(matches!(hook_lines_step_in(&r.root, &r.pack, &id, &r.lane, HookStep::Unregister, &slow, 1.0), Err(RepairError::CheckTimeout(_))));
    assert!(wal::unfinished_for_lane(&r.root, &r.lane).is_empty(), "closed, not retried forever");
    // a crash right after the record: write an unfinished record by hand
    let id2 = rid("R-22");
    hook_record(&r, &id2, "Stop", "z.sh");
    let rec = OpRecord {
        schema: 1, op: wal::new_op_id(2.0, "hook-unregister"), repair: id2.clone(), lane_key: r.lane.clone(),
        kind: "hook-unregister".into(), form: "hook".into(), rel: String::new(), pre_existed: false,
        expected: String::new(), incoming: String::new(), pack_generation: None, slot: None, slot_identity: None,
        live_identity: None, incoming_copy: None, incoming_copy_identity: None, step: "recorded".into(),
        hooks: vec![wal::HookLine { event: "Stop".into(), script: "z.sh".into(), targets: Vec::new() }],
        finished: false, outcome: None, note: String::new(), created_at: 2.0, updated_at: 2.0,
    };
    wal::write_in(&r.root, &rec).unwrap();
    let ok = |_: &Path, _: bool| -> Result<String, RepairError> { Ok(report_json(&[("Stop", "z.sh", "unregistered", &[("/s.json", "unregistered")])])) };
    assert_eq!(resume_hook_steps_in(&r.root, &r.pack, &r.lane, &ok, &no_repair_lock, 3.0), 1);
    assert!(crate::repair::tombstones::load_in(&r.root, &r.lane).unwrap().iter().any(|t| t.rel == "settings:Stop:z.sh"));
    assert!(wal::unfinished_for_lane(&r.root, &r.lane).is_empty());
}

/// The bounded child runner: killed and reaped at its deadline.
#[cfg(unix)]
#[test]
fn run_bounded_kills_at_the_deadline() {
    let mut cmd = std::process::Command::new("/bin/sleep");
    cmd.arg("30").stdout(std::process::Stdio::piped());
    let t = std::time::Instant::now();
    assert!(matches!(run_bounded(cmd, Duration::from_millis(300)), Err(RepairError::CheckTimeout(_))));
    assert!(t.elapsed() < Duration::from_secs(10));
    let mut ok = std::process::Command::new("/bin/echo");
    ok.arg("{}").stdout(std::process::Stdio::piped());
    assert_eq!(run_bounded(ok, Duration::from_secs(10)).unwrap().trim(), "{}");
}

#[allow(dead_code)]
fn _unused(_: BTreeMap<String, String>, _: LockOwner) {}

/// U2 / V1: a MODE-only repair and a LINK repair, approved and applied — the live state equals the
/// approved state (`file:<same sha>:0755`, `link:<target>`), and the approved copy counts (F9/U1).
#[cfg(target_os = "macos")]
#[test]
fn approved_mode_only_and_link_applies() {
    let r = rig("modelink");
    // mode only: same bytes, 0644 → 0755
    let id = rid("R-30");
    r.put("hooks/m.sh", b"#!/bin/sh\n", 0o644);
    let pre_img = ledger::copy_preimage_in(&r.root, &id, &r.lane, "hooks/m.sh", &r.pack.join("hooks/m.sh"), 1.0).unwrap();
    let staging = ledger::staging_dir_in(&r.root, &id).join("hooks/m.sh");
    std::fs::create_dir_all(staging.parent().unwrap()).unwrap();
    std::fs::write(&staging, b"#!/bin/sh\n").unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&staging, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let st = TargetState::read(&staging).unwrap();
    let fp = crate::repair::types::fingerprint(&[("hooks/m.sh".into(), st.clone())]);
    let mut rec = record(&r, &id, Classification::BootCritical, vec![FileChange { rel: "hooks/m.sh".into(), kind: ChangeKind::Mode, pre: pre_img, post: st.clone(), diff: None }], RepairState::AwaitingApproval);
    rec.approval = Some(approval(&fp));
    ledger::write_record_in(&r.root, &rec).unwrap();
    let env = r.env(&NoFaults, ApplyForm::Exchange);
    let rep = apply_candidate_in(&env, &id, &[CandidateFile { rel: "hooks/m.sh".into(), staged: staging, mode: 0o755 }], &|_, _, _| Ok(()), &pass_check, &pass_ref, &no_repair_lock).unwrap();
    assert_eq!(rep.outcome, ApplyOutcome::Applied, "{rep:?}");
    assert_eq!(r.live("hooks/m.sh"), st);
    assert!(r.live("hooks/m.sh").canon().ends_with(":0755"));
    assert!(approved_copy_of_current_generation_in(&r.root, &r.pack, &r.lane, "hooks/m.sh").unwrap());
    // link: a file becomes a link whose target is outside every pack
    let id2 = rid("R-31");
    r.put("hooks/l.sh", b"FILE\n", 0o644);
    let pre2 = ledger::copy_preimage_in(&r.root, &id2, &r.lane, "hooks/l.sh", &r.pack.join("hooks/l.sh"), 1.0).unwrap();
    let staging2 = ledger::staging_dir_in(&r.root, &id2).join("hooks/l.sh");
    std::fs::create_dir_all(staging2.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink("/usr/bin/true", &staging2).unwrap();
    let st2 = TargetState::read(&staging2).unwrap();
    let fp2 = crate::repair::types::fingerprint(&[("hooks/l.sh".into(), st2.clone())]);
    let mut rec2 = record(&r, &id2, Classification::BootCritical, vec![FileChange { rel: "hooks/l.sh".into(), kind: ChangeKind::Link, pre: pre2, post: st2.clone(), diff: None }], RepairState::AwaitingApproval);
    rec2.approval = Some(approval(&fp2));
    ledger::write_record_in(&r.root, &rec2).unwrap();
    let rep2 = apply_candidate_in(&env, &id2, &[CandidateFile { rel: "hooks/l.sh".into(), staged: staging2, mode: 0 }], &|_, _, _| Ok(()), &pass_check, &pass_ref, &no_repair_lock).unwrap();
    assert_eq!(rep2.outcome, ApplyOutcome::Applied, "{rep2:?}");
    assert_eq!(r.live("hooks/l.sh").canon(), "link:/usr/bin/true");
}

/// AW3 (b) / (c) at apply: a target whose parent chain inside the pack holds a link is declined; a
/// candidate link that points into a pack is declined (a repair never creates an alias of a pack path).
#[cfg(target_os = "macos")]
#[test]
fn apply_declines_parent_link_and_link_into_a_pack() {
    let r = rig("aw3bc");
    // (b): hooks2 is a link to hooks
    std::os::unix::fs::symlink(r.pack.join("hooks"), r.pack.join("hooks2")).unwrap();
    let id = rid("R-32");
    let staging = ledger::staging_dir_in(&r.root, &id).join("hooks2/x.sh");
    std::fs::create_dir_all(staging.parent().unwrap()).unwrap();
    std::fs::write(&staging, b"X\n").unwrap();
    let st = TargetState::read(&staging).unwrap();
    let mut rec = record(&r, &id, Classification::BootCritical, vec![FileChange { rel: "hooks2/x.sh".into(), kind: ChangeKind::Create, pre: PreImage::Absent, post: st.clone(), diff: None }], RepairState::AwaitingApproval);
    rec.approval = Some(approval(&crate::repair::types::fingerprint(&[("hooks2/x.sh".into(), st)])));
    ledger::write_record_in(&r.root, &rec).unwrap();
    let env = r.env(&NoFaults, ApplyForm::Exchange);
    let rep = apply_candidate_in(&env, &id, &[CandidateFile { rel: "hooks2/x.sh".into(), staged: staging, mode: 0o644 }], &|_, _, _| Ok(()), &pass_check, &pass_ref, &no_repair_lock).unwrap();
    assert!(matches!(rep.outcome, ApplyOutcome::Declined(_)), "{rep:?}");
    assert!(!r.pack.join("hooks/x.sh").exists());
    // (c): a link candidate into the pack
    let id2 = rid("R-33");
    r.put("hooks/y.sh", b"Y\n", 0o644);
    let pre2 = ledger::copy_preimage_in(&r.root, &id2, &r.lane, "hooks/y.sh", &r.pack.join("hooks/y.sh"), 1.0).unwrap();
    let staging2 = ledger::staging_dir_in(&r.root, &id2).join("hooks/y.sh");
    std::fs::create_dir_all(staging2.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(r.pack.join("hooks/a-listed.sh"), &staging2).unwrap();
    let st2 = TargetState::read(&staging2).unwrap();
    let mut rec2 = record(&r, &id2, Classification::BootCritical, vec![FileChange { rel: "hooks/y.sh".into(), kind: ChangeKind::Link, pre: pre2, post: st2.clone(), diff: None }], RepairState::AwaitingApproval);
    rec2.approval = Some(approval(&crate::repair::types::fingerprint(&[("hooks/y.sh".into(), st2)])));
    ledger::write_record_in(&r.root, &rec2).unwrap();
    let rep2 = apply_candidate_in(&env, &id2, &[CandidateFile { rel: "hooks/y.sh".into(), staged: staging2, mode: 0 }], &|_, _, _| Ok(()), &pass_check, &pass_ref, &no_repair_lock).unwrap();
    assert!(matches!(rep2.outcome, ApplyOutcome::Declined(ref m) if m.contains("AW3")), "{rep2:?}");
    assert_eq!(std::fs::read(r.pack.join("hooks/y.sh")).unwrap(), b"Y\n");
}

/// E9 at the engine: an exchange that fails (here the slot entry vanished → ENOENT) DECLINES — the
/// live file untouched, the operation record closed `declined`, nothing renamed over.
#[cfg(target_os = "macos")]
#[test]
fn failing_exchange_declines_and_closes_the_record() {
    struct Vanish;
    impl Faults for Vanish {
        fn at(&self, p: Point, _live: &Path, slot: Option<&Slot>) -> bool {
            if p == Point::AfterRecord {
                if let Some(s) = slot {
                    let _ = std::fs::remove_file(s.payload()); // a test-made failure, never the mechanism
                }
            }
            false
        }
    }
    let r = rig("e9");
    r.put("hooks/z.sh", b"LIVE\n", 0o755);
    let (src, st) = r.source("z.sh", b"NEW\n", 0o755);
    let env = r.env(&Vanish, ApplyForm::Exchange);
    let d = run_op(&env, &OpSpec { id: rid("R-34"), rel: "hooks/z.sh".into(), kind: "apply".into(), expected: r.live("hooks/z.sh"), incoming: st, source: Some(src), generation: None }).unwrap();
    assert!(matches!(d.outcome, ApplyOutcome::Declined(ref m) if m.contains("ENOENT")), "{:?}", d.outcome);
    assert_eq!(std::fs::read(r.pack.join("hooks/z.sh")).unwrap(), b"LIVE\n");
    let (ops, _) = wal::list_in(&r.root, &rid("R-34"));
    assert_eq!(ops.len(), 1);
    assert_eq!((ops[0].finished, ops[0].outcome.as_deref()), (true, Some("declined")));
}

/// RULINGS CT2 at the operation level: the mandatory full flush fails (EIO) right after an exchange
/// → the operation returns an ERROR (nothing reported applied), its record stays UNFINISHED with its
/// evidence (the slot and both identities), and the next run, with the flush working again, finishes
/// it from the record. Fails without CT2 when the flush error is swallowed.
#[cfg(target_os = "macos")]
#[test]
fn flush_failure_after_the_exchange_keeps_the_evidence() {
    struct FailFlushAfterExchange;
    impl Faults for FailFlushAfterExchange {
        fn at(&self, point: Point, _live: &Path, _slot: Option<&Slot>) -> bool {
            if point == Point::AfterExchange {
                crate::repair::durable::INJECT_FLUSH_ERRNO.with(|c| c.set((libc::EIO, 1000)));
            }
            false
        }
    }
    let r = rig("ct2op");
    let before = r.put("hooks/f.sh", b"OLD\n", 0o755);
    let (src, incoming) = r.source("f-new", b"NEW\n", 0o755);
    let spec = OpSpec { id: rid("R-6"), rel: "hooks/f.sh".into(), kind: "revert".into(), expected: before, incoming: incoming.clone(), source: Some(src), generation: None };
    let faults = FailFlushAfterExchange;
    let res = run_op(&r.env(&faults, ApplyForm::Exchange), &spec);
    crate::repair::durable::INJECT_FLUSH_ERRNO.with(|c| c.set((0, 0)));
    assert!(res.is_err(), "a failed full flush is an error, never an outcome: {res:?}");
    let unfinished = wal::unfinished_for_lane(&r.root, &r.lane);
    assert_eq!(unfinished.len(), 1, "the operation keeps its record");
    assert!(unfinished[0].slot.is_some() && unfinished[0].slot_identity.is_some() && unfinished[0].live_identity.is_some());
    resume_interrupted_in(&r.env(&NoFaults, ApplyForm::Exchange)).unwrap();
    assert!(wal::unfinished_for_lane(&r.root, &r.lane).is_empty(), "finished from the record");
    assert_eq!(r.live("hooks/f.sh"), incoming);
    assert!(r.vault_bytes().contains(&b"OLD\n".to_vec()));
}

// ───────────────────────────── RULINGS CT3 — measured outcomes ─────────────────────────────

/// CT3 (blocking 3): the slot is rewritten by another writer AFTER its last validation, in the
/// record → exchange window. What the exchange put live is then NOT the approved state: it is moved
/// into a fresh slot by a checked operation (expected = what is actually live) and the previous
/// bytes return; the apply is declined, never "applied" or "reverted" on an unchecked outcome.
/// Without CT3 the unapproved bytes stayed live while the reply said the change was put back.
#[cfg(target_os = "macos")]
#[test]
fn unapproved_bytes_exchanged_in_are_put_back_by_a_checked_operation() {
    struct TamperSlotAfterRecord(Cell<bool>);
    impl Faults for TamperSlotAfterRecord {
        fn at(&self, point: Point, _live: &Path, slot: Option<&Slot>) -> bool {
            if point == Point::AfterRecord && !self.0.get() {
                if let Some(s) = slot {
                    use std::os::unix::fs::PermissionsExt;
                    self.0.set(true);
                    let _ = std::fs::set_permissions(s.payload(), std::fs::Permissions::from_mode(0o755));
                    let mut f = std::fs::OpenOptions::new().write(true).truncate(true).open(s.payload()).unwrap();
                    std::io::Write::write_all(&mut f, b"UNAPPROVED\n").unwrap();
                }
            }
            false
        }
    }
    let r = rig("ct3tamper");
    let id = rid("R-30");
    let rel = "hooks/t.sh";
    r.put(rel, b"PRE\n", 0o755);
    let pre_img = ledger::copy_preimage_in(&r.root, &id, &r.lane, rel, &r.pack.join(rel), 1.0).unwrap();
    let staging = ledger::staging_dir_in(&r.root, &id).join(rel);
    std::fs::create_dir_all(staging.parent().unwrap()).unwrap();
    std::fs::write(&staging, b"APPROVED\n").unwrap();
    let st = TargetState::read(&staging).unwrap();
    let fp = crate::repair::types::fingerprint(&[(rel.to_string(), st.clone())]);
    let mut rec = record(&r, &id, Classification::BootCritical, vec![FileChange { rel: rel.into(), kind: ChangeKind::Edit, pre: pre_img, post: st, diff: None }], RepairState::AwaitingApproval);
    rec.approval = Some(approval(&fp));
    ledger::write_record_in(&r.root, &rec).unwrap();
    let faults = TamperSlotAfterRecord(Cell::new(false));
    let env = r.env(&faults, ApplyForm::Exchange);
    let rep = apply_candidate_in(&env, &id, &[CandidateFile { rel: rel.into(), staged: staging, mode: 0o644 }], &|_, _, _| Ok(()), &pass_check, &pass_ref, &no_repair_lock).unwrap();
    assert!(faults.0.get(), "the tamper fired");
    assert!(matches!(rep.outcome, ApplyOutcome::Declined(_)), "{:?}", rep.outcome);
    assert_eq!(std::fs::read(r.pack.join(rel)).unwrap(), b"PRE\n", "the previous bytes are back, the unapproved ones are not live");
    assert!(r.vault_bytes().contains(&b"UNAPPROVED\n".to_vec()), "the unapproved entry is kept in a slot (rule one)");
    let after = ledger::read_record_in(&r.root, &id).unwrap().unwrap();
    assert_ne!(after.state, RepairState::Probation, "never applied on an unchecked outcome");
    assert!(after.approval.as_ref().is_none_or(|a| !a.approved), "the approval is void");
    assert!(wal::unfinished_for_lane(&r.root, &r.lane).is_empty());
}

/// CT3: a revert whose re-registration fails is NOT reported reverted — the outcome is incomplete,
/// the lane row says 충돌 and keeps what is left (the hook part), the record keeps its state; the
/// next revert finishes it (hook-only — CT7) and only then says 되돌려짐. Without CT3 the first
/// revert answered `reverted: true` / 되돌려짐 with the failure only inside `hook_step`.
#[cfg(target_os = "macos")]
#[test]
fn revert_with_a_failed_reregistration_is_incomplete_then_finished() {
    let r = rig("ct3hook");
    let id = rid("R-31");
    let mut rec = committed_general(&r, &id, "hooks/x.sh", Some(b"PRE\n"), Some(b"POST\n"));
    rec.files.push(FileChange {
        rel: "settings:Stop:x.sh".into(),
        kind: ChangeKind::HookUnregister { event: "Stop".into(), script: "x.sh".into() },
        pre: PreImage::Absent,
        post: TargetState::Absent,
        diff: None,
    });
    ledger::write_record_in(&r.root, &rec).unwrap();
    let failing = |_: &RepairId, _: &LaneKey, _: HookStep| -> Result<ReconcileReport, RepairError> {
        Err(RepairError::CheckFailed("registrar down".into()))
    };
    let env = r.env(&NoFaults, ApplyForm::Exchange);
    let first = revert_in(&env, &id, RevertCaller::Human, &failing, &no_repair_lock).unwrap();
    assert!(matches!(first.outcome, ApplyOutcome::Declined(ref m) if m.contains("충돌")), "{:?}", first.outcome);
    assert_eq!(std::fs::read(r.pack.join("hooks/x.sh")).unwrap(), b"PRE\n", "the file part is reverted");
    let row = ledger::read_lane_in(&r.root, &id, &r.lane).unwrap().unwrap();
    assert_eq!(row.state, LaneState::Conflict);
    assert!(row.applied.is_empty(), "only the hook part is left");
    assert_eq!(ledger::read_record_in(&r.root, &id).unwrap().unwrap().state, RepairState::Probation, "not 되돌려짐 on an incomplete revert");
    let ok = |_: &RepairId, _: &LaneKey, _: HookStep| -> Result<ReconcileReport, RepairError> { Ok(ReconcileReport { hooks: Vec::new() }) };
    let second = revert_in(&env, &id, RevertCaller::Human, &ok, &no_repair_lock).unwrap();
    assert_eq!(second.outcome, ApplyOutcome::Applied, "{:?}", second.outcome);
    assert_eq!(ledger::read_record_in(&r.root, &id).unwrap().unwrap().state, RepairState::Reverted);
}

/// CT7: a HOOK-ONLY repair (no file state) is revertible — its re-registration is its state.
/// Without CT7 `revert` refused it ("되돌릴 것이 없습니다") before reaching the hook step.
#[test]
fn hook_only_repair_is_revertible() {
    let r = rig("ct7revert");
    let id = rid("R-32");
    hook_record(&r, &id, "Stop", "y.sh");
    lane_row(&r, &id, Vec::new(), Vec::new());
    let called = Cell::new(0);
    let hooks = |_: &RepairId, _: &LaneKey, step: HookStep| -> Result<ReconcileReport, RepairError> {
        assert_eq!(step, HookStep::Reregister);
        called.set(called.get() + 1);
        Ok(ReconcileReport { hooks: Vec::new() })
    };
    let env = r.env(&NoFaults, ApplyForm::Exchange);
    let rep = revert_in(&env, &id, RevertCaller::Human, &hooks, &no_repair_lock).unwrap();
    assert_eq!(rep.outcome, ApplyOutcome::Applied, "{:?}", rep.outcome);
    assert_eq!(called.get(), 1);
    assert_eq!(ledger::read_record_in(&r.root, &id).unwrap().unwrap().state, RepairState::Reverted);
}

/// CT3: the hook step's dry run cannot be measured → the step DECLINES before anything is recorded
/// or changed (no operation record, no tombstone, no real registrar run). Without CT3 the dry run's
/// failure became an empty report and the real run went ahead.
#[test]
fn hook_step_declines_when_its_dry_run_cannot_be_measured() {
    let r = rig("ct3dry");
    let id = rid("R-33");
    hook_record(&r, &id, "Stop", "z.sh");
    let calls = RefCell::new(Vec::new());
    let registrar = |_: &Path, dry: bool| -> Result<String, RepairError> {
        calls.borrow_mut().push(dry);
        if dry {
            Err(RepairError::CheckTimeout("dry run hung".into()))
        } else {
            Ok(report_json(&[("Stop", "z.sh", "unregistered", &[])]))
        }
    };
    let res = hook_lines_step_in(&r.root, &r.pack, &id, &r.lane, HookStep::Unregister, &registrar, 5.0);
    assert!(res.is_err(), "{res:?}");
    assert_eq!(*calls.borrow(), vec![true], "the real run never happened");
    let (ops, _) = wal::list_in(&r.root, &id);
    assert!(ops.is_empty(), "nothing recorded");
    assert!(crate::repair::tombstones::load_in(&r.root, &r.lane).unwrap().is_empty(), "no tombstone");
}

// ───────────────────────────── RULINGS CT4 — one apply = one recoverable unit ─────────────────────────────

/// Two-target approved candidate of `R` (boot-critical), returned with its candidate files.
fn two_target_candidate(r: &Rig, id: &RepairId) -> Vec<CandidateFile> {
    let mut files = Vec::new();
    let mut changes = Vec::new();
    let mut states = Vec::new();
    for (rel, pre, post) in [("hooks/a1.sh", b"PRE-1\n", b"NEW-1\n"), ("hooks/a2.sh", b"PRE-2\n", b"NEW-2\n")] {
        r.put(rel, pre, 0o755);
        let pre_img = ledger::copy_preimage_in(&r.root, id, &r.lane, rel, &r.pack.join(rel), 1.0).unwrap();
        let staging = ledger::staging_dir_in(&r.root, id).join(rel);
        std::fs::create_dir_all(staging.parent().unwrap()).unwrap();
        std::fs::write(&staging, post).unwrap();
        let st = TargetState::read(&staging).unwrap();
        states.push((rel.to_string(), st.clone()));
        changes.push(FileChange { rel: rel.into(), kind: ChangeKind::Edit, pre: pre_img, post: st, diff: None });
        files.push(CandidateFile { rel: rel.into(), staged: staging, mode: 0o644 });
    }
    let fp = crate::repair::types::fingerprint(&states);
    let mut rec = record(r, id, Classification::BootCritical, changes, RepairState::AwaitingApproval);
    rec.approval = Some(approval(&fp));
    ledger::write_record_in(&r.root, &rec).unwrap();
    files
}

/// Stops (a simulated crash) at `point` of the operation on `rel` only.
struct StopOn {
    rel: &'static str,
    point: Point,
    fired: Cell<bool>,
}

impl Faults for StopOn {
    fn at(&self, point: Point, live: &Path, _slot: Option<&Slot>) -> bool {
        if point == self.point && live.ends_with(self.rel) && !self.fired.get() {
            self.fired.set(true);
            return true;
        }
        false
    }
}

/// CT4 (blocking 4): a crash BETWEEN the targets of one approved apply — the first target's
/// operation already finished — is undone AS A WHOLE by the next run: both files back at their
/// pre-images, the approval void, the intent `undone`; and a crash right after the second exchange
/// the same. Without CT4 the first file kept the approved bytes with no record and no lane row.
#[cfg(target_os = "macos")]
#[test]
fn crash_between_the_targets_of_one_apply_is_undone_as_a_whole() {
    for point in [Point::AfterRecord, Point::AfterSeal, Point::BeforeDone] {
        let r = rig("ct4crash");
        let id = rid("R-40");
        let files = two_target_candidate(&r, &id);
        let faults = StopOn { rel: "hooks/a2.sh", point, fired: Cell::new(false) };
        let env = r.env(&faults, ApplyForm::Exchange);
        let res = apply_candidate_in(&env, &id, &files, &|_, _, _| Ok(()), &pass_check, &pass_ref, &no_repair_lock);
        assert!(faults.fired.get(), "{point:?}: the stop fired");
        assert!(res.is_err(), "{point:?}: the simulated crash");
        assert_eq!(std::fs::read(r.pack.join("hooks/a1.sh")).unwrap(), b"NEW-1\n", "{point:?}: the first target was applied before the crash");
        assert_eq!(read_intent_in(&r.root, &id, &r.lane).unwrap().state, "applying");
        // the next ready tick: interrupted operations, then the intent
        let calm = r.env(&NoFaults, ApplyForm::Exchange);
        resume_interrupted_in(&calm).unwrap();
        let lines = recover_apply_intents_in(&calm, &no_repair_lock);
        assert_eq!(lines.len(), 1, "{point:?}: {lines:?}");
        assert_eq!(std::fs::read(r.pack.join("hooks/a1.sh")).unwrap(), b"PRE-1\n", "{point:?}: the finished first target is undone too");
        assert_eq!(std::fs::read(r.pack.join("hooks/a2.sh")).unwrap(), b"PRE-2\n", "{point:?}");
        assert_eq!(read_intent_in(&r.root, &id, &r.lane).unwrap().state, "undone");
        let after = ledger::read_record_in(&r.root, &id).unwrap().unwrap();
        assert_eq!(after.state, RepairState::InProgress, "{point:?}: back to 진행 중, approval void");
        assert!(after.approval.is_none());
        assert!(r.vault_bytes().contains(&b"NEW-1\n".to_vec()), "the approved bytes stay in the vault");
        // idempotent: a second recovery does nothing
        assert!(recover_apply_intents_in(&calm, &no_repair_lock).is_empty());
    }
}

/// CT4: when the recovery cannot put a target back (someone else wrote it after the crash), the
/// lane row says 충돌 with exactly the targets still at the apply's state — revertible — and the
/// intent is `incomplete`; the foreign bytes are left alone.
#[cfg(target_os = "macos")]
#[test]
fn unrecoverable_target_leaves_a_revertible_conflict_row() {
    let r = rig("ct4conflict");
    let id = rid("R-41");
    let files = two_target_candidate(&r, &id);
    let faults = StopOn { rel: "hooks/a2.sh", point: Point::BeforeDone, fired: Cell::new(false) };
    assert!(apply_candidate_in(&r.env(&faults, ApplyForm::Exchange), &id, &files, &|_, _, _| Ok(()), &pass_check, &pass_ref, &no_repair_lock).is_err());
    let calm = r.env(&NoFaults, ApplyForm::Exchange);
    resume_interrupted_in(&calm).unwrap();
    // another writer changes the second file before the recovery runs
    r.put("hooks/a2.sh", b"SOMEONE-ELSE\n", 0o755);
    let lines = recover_apply_intents_in(&calm, &no_repair_lock);
    assert!(lines[0].contains("충돌"), "{lines:?}");
    assert_eq!(std::fs::read(r.pack.join("hooks/a2.sh")).unwrap(), b"SOMEONE-ELSE\n", "foreign bytes left alone");
    assert_eq!(std::fs::read(r.pack.join("hooks/a1.sh")).unwrap(), b"PRE-1\n", "our own target undone");
    assert_eq!(read_intent_in(&r.root, &id, &r.lane).unwrap().state, "incomplete");
    let row = ledger::read_lane_in(&r.root, &id, &r.lane).unwrap().unwrap();
    assert_eq!(row.state, LaneState::Conflict);
    assert!(row.applied.is_empty(), "nothing of the apply is live any more: {:?}", row.applied);
}

// ───────────────────────────── RULINGS CT6 — the job thread always returns ─────────────────────────────

fn pid_gone(pid: i32) -> bool {
    // a killed orphan is reaped by the system shortly; wait for it, bounded
    for _ in 0..200 {
        // SAFETY: signal 0 only probes the pid.
        if unsafe { libc::kill(pid, 0) } != 0 {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50)); // wallclock: guard bounded poll for a guaranteed reap
    }
    false
}

/// CT6 (major 7; CZ10): the child exits at once but its DESCENDANT keeps stdout open. With the
/// child leading its own process group the whole group is stopped and the call returns within the
/// deadline with what was left behind in the error (the descendant's pid, the output received);
/// the descendant is gone. Without CT6 `reader.join()` waited for the descendant (60 s here — in
/// production forever: the job thread never returned and every later tick answered `busy`).
#[cfg(unix)]
#[test]
fn a_descendant_holding_stdout_cannot_hold_the_job() {
    use crate::SpawnPolicy as _;
    let mut cmd = std::process::Command::new("/bin/sh");
    cmd.arg("-c").arg("sleep 60 & echo partial $!");
    cmd.spawn_policy(crate::ChildLifetime::GroupScoped);
    cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null());
    let t = std::time::Instant::now();
    let res = run_bounded(cmd, Duration::from_secs(4));
    assert!(t.elapsed() < Duration::from_secs(20), "returned at the deadline, not when the descendant ended");
    let msg = match res {
        Err(RepairError::CheckFailed(m)) => m,
        other => panic!("a held pipe is reported, not swallowed: {other:?}"),
    };
    assert!(msg.contains("자손") && msg.contains("partial"), "{msg}");
    let pid: i32 = msg.split("partial ").nth(1).and_then(|t| t.split_whitespace().next()).and_then(|p| p.parse().ok()).expect("the descendant's pid");
    assert!(pid_gone(pid), "the descendant (pid {pid}) was stopped with its group");
}

/// CT6: even a descendant that ESCAPED the group (here: the child does not lead a group at all)
/// cannot hold the call past its deadline — the reader is left to it and the error says so. The
/// test stops its own stray descendant afterwards by its exact pid.
#[cfg(unix)]
#[test]
fn an_escaped_descendant_cannot_hold_the_reader_past_the_deadline() {
    let r = rig("ct6escape");
    let pidfile = r.base.join("sleep.pid");
    let mut cmd = std::process::Command::new("/bin/sh");
    cmd.arg("-c").arg(format!("sleep 30 & echo $! > '{}'; echo escaped", pidfile.display()));
    cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null());
    let t = std::time::Instant::now();
    let res = run_bounded(cmd, Duration::from_secs(4));
    assert!(t.elapsed() < Duration::from_secs(20), "returned at the deadline");
    let msg = format!("{res:?}");
    let pid: i32 = std::fs::read_to_string(&pidfile).unwrap().trim().parse().unwrap();
    // SAFETY: our own child's descendant, by its exact pid.
    unsafe {
        libc::kill(pid, libc::SIGKILL);
    }
    assert!(res.is_err() && msg.contains("파이프"), "{msg}");
}

/// CT7: a resumed hook operation that fails marks the lane row 충돌 (as the first run's caller
/// does). Without CT7 the resumed failure closed the record `declined` and the row stayed 검증됨.
#[test]
fn resumed_hook_step_failure_marks_the_lane_conflict() {
    let r = rig("ct7resume");
    let id = rid("R-34");
    hook_record(&r, &id, "Stop", "q.sh");
    lane_row(&r, &id, Vec::new(), Vec::new());
    let op = OpRecord {
        schema: 1,
        op: wal::new_op_id(1.0, "hook-unregister"),
        repair: id.clone(),
        lane_key: r.lane.clone(),
        kind: "hook-unregister".into(),
        form: "hook".into(),
        rel: String::new(),
        pre_existed: false,
        expected: String::new(),
        incoming: String::new(),
        pack_generation: Some(pack_generation(&r.pack)),
        slot: None,
        slot_identity: None,
        live_identity: None,
        incoming_copy: None,
        incoming_copy_identity: None,
        step: "recorded".into(),
        hooks: vec![wal::HookLine { event: "Stop".into(), script: "q.sh".into(), targets: Vec::new() }],
        finished: false,
        outcome: None,
        note: String::new(),
        created_at: 1.0,
        updated_at: 1.0,
    };
    wal::write_in(&r.root, &op).unwrap();
    let failing = |_: &Path, _: bool| -> Result<String, RepairError> { Err(RepairError::CheckFailed("registrar down".into())) };
    let _ = resume_hook_steps_in(&r.root, &r.pack, &r.lane, &failing, &no_repair_lock, 3.0);
    let row = ledger::read_lane_in(&r.root, &id, &r.lane).unwrap().unwrap();
    assert_eq!(row.state, LaneState::Conflict);
}

/// CT9 (major 13): F9's triple is an AGREEMENT — each member's independent mismatch makes it false:
/// (a) an UNRELATED approval fingerprint on an otherwise complete triple, (b) a lane row whose
/// approval reference is not the approval's, (c) slot metadata that disagrees with the operation's
/// expected state. Without CT9 all three stayed "approved".
#[test]
fn f9_is_an_agreement_of_its_members() {
    let rel = "hooks/f.sh";
    let fresh = |tag: &str| {
        let r = rig(tag);
        let id = rid("R-13");
        approved_and_applied(&r, &id, rel, b"PRE\n", b"APPROVED\n");
        assert!(approved_copy_of_current_generation_in(&r.root, &r.pack, &r.lane, rel).unwrap(), "{tag}: complete triple");
        (r, id)
    };
    // (a) an unrelated approval fingerprint
    let (r, id) = fresh("f9a");
    let mut rec = ledger::read_record_in(&r.root, &id).unwrap().unwrap();
    rec.approval.as_mut().unwrap().fingerprint_sha256 = "0".repeat(64);
    ledger::write_record_in(&r.root, &rec).unwrap();
    assert!(!approved_copy_of_current_generation_in(&r.root, &r.pack, &r.lane, rel).unwrap(), "(a)");
    // (b) the approval reference
    let (r, id) = fresh("f9b");
    let mut row = ledger::read_lane_in(&r.root, &id, &r.lane).unwrap().unwrap();
    row.approval_ref = Some("rqr-other".into());
    ledger::write_lane_in(&r.root, &row).unwrap();
    assert!(!approved_copy_of_current_generation_in(&r.root, &r.pack, &r.lane, rel).unwrap(), "(b)");
    // (c) the slot's metadata disagrees with the operation's expected state
    let (r, id) = fresh("f9c");
    let (ops, _) = wal::list_in(&r.root, &id);
    let op = ops.iter().find(|o| o.kind == "apply" && o.outcome.as_deref() == Some("applied")).unwrap();
    let slot = Slot::at(op.slot.as_ref().unwrap(), rel);
    let mut pre: serde_json::Value = serde_json::from_slice(&std::fs::read(slot.meta_pre()).unwrap()).unwrap();
    pre["sha256"] = serde_json::json!("f".repeat(64));
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(slot.meta_pre(), std::fs::Permissions::from_mode(0o644)).unwrap();
    }
    std::fs::write(slot.meta_pre(), serde_json::to_vec(&pre).unwrap()).unwrap();
    assert!(!approved_copy_of_current_generation_in(&r.root, &r.pack, &r.lane, rel).unwrap(), "(c)");
}

// ───────────────────────────── RULINGS CT12 / CU3 — the approved apply as one transaction ─────────────────────────────

/// CT12 / CU3: THE property over the COMPLETE approved-apply transaction of a two-file candidate —
/// a crash at every point of either file's operation (between the slot's validation and its
/// exchange, after the record, after the exchange, inside the seal on both seal paths, inside
/// `meta/sealed.json`, after the seal, before done), then a SECOND crash at the same point during the
/// first recovery, then the recovery runs: (P0) each live file is exactly one of its legitimate
/// versions — the whole set at the pre-images (undone) or, when no crash fired, the whole set at the
/// approved states; (P4) no operation unfinished; the intent ends `undone` / `applied`; (P2) no slot
/// half-made, no temporary entry, no screen view left.
#[cfg(target_os = "macos")]
#[test]
fn approved_apply_property_every_file_every_point() {
    let points = [
        Point::AfterValidation,
        Point::AfterRecord,
        Point::AfterExchange,
        Point::InSealCreated,
        Point::InSealPartial,
        Point::InSealWritten,
        Point::InSealPlaced,
        Point::InMetaPartial,
        Point::InMetaWritten,
        Point::AfterSeal,
        Point::BeforeDone,
        Point::AfterRecordCommit,
    ];
    let mut fired = 0;
    for target in ["hooks/a1.sh", "hooks/a2.sh"] {
        for point in points {
            for byte_copy in [false, true] {
                let r = rig("ct12prop");
                let id = rid("R-42");
                let files = two_target_candidate(&r, &id);
                crate::repair::vault::FORCE_BYTE_COPY_SEAL.with(|c| c.set(byte_copy));
                let faults = StopOn { rel: target, point, fired: Cell::new(false) };
                let res = apply_candidate_in(&r.env(&faults, ApplyForm::Exchange), &id, &files, &|_, _, _| Ok(()), &pass_check, &pass_ref, &no_repair_lock);
                let crashed = faults.fired.get();
                if crashed {
                    fired += 1;
                    assert!(res.is_err(), "{target}/{point:?}: the stop is a crash");
                    let again = StopOn { rel: target, point, fired: Cell::new(false) };
                    let _ = resume_interrupted_in(&r.env(&again, ApplyForm::Exchange));
                }
                let calm = r.env(&NoFaults, ApplyForm::Exchange);
                resume_interrupted_in(&calm).unwrap();
                let _ = recover_apply_intents_in(&calm, &no_repair_lock);
                crate::repair::vault::FORCE_BYTE_COPY_SEAL.with(|c| c.set(false));
                let tag = format!("{target} at {point:?}{}", if byte_copy { " (byte-copy seal)" } else { "" });
                let a1 = std::fs::read(r.pack.join("hooks/a1.sh")).unwrap();
                let a2 = std::fs::read(r.pack.join("hooks/a2.sh")).unwrap();
                if crashed && point == Point::AfterRecordCommit {
                    // DX2: the record was committed — the recovery completes forward
                    assert_eq!((a1.as_slice(), a2.as_slice()), (&b"NEW-1\n"[..], &b"NEW-2\n"[..]), "P0 {tag}: completed forward");
                    assert_eq!(read_intent_in(&r.root, &id, &r.lane).unwrap().state, "applied", "{tag}");
                    assert!(ledger::read_lane_in(&r.root, &id, &r.lane).unwrap().is_some(), "{tag}: the lane row was written");
                } else if crashed {
                    assert_eq!((a1.as_slice(), a2.as_slice()), (&b"PRE-1\n"[..], &b"PRE-2\n"[..]), "P0 {tag}: the whole set undone");
                    assert_eq!(read_intent_in(&r.root, &id, &r.lane).unwrap().state, "undone", "{tag}");
                } else {
                    assert_eq!((a1.as_slice(), a2.as_slice()), (&b"NEW-1\n"[..], &b"NEW-2\n"[..]), "P0 {tag}: the whole set applied");
                    assert_eq!(read_intent_in(&r.root, &id, &r.lane).unwrap().state, "applied", "{tag}");
                }
                assert!(wal::unfinished_for_lane(&r.root, &r.lane).is_empty(), "P4 {tag}");
                let vault_dir = vault::vault_dir_in(&r.root, &id);
                for e in std::fs::read_dir(&vault_dir).unwrap().flatten() {
                    let n = e.file_name().to_string_lossy().to_string();
                    assert!(!n.starts_with(".building") && !n.starts_with(".screen") && !n.contains(".part-"), "P2 {tag}: left behind: {n}");
                    if crate::repair::ledger::is_slot_name(&n) {
                        for sub in ["sealed", "meta"] {
                            for f in std::fs::read_dir(e.path().join(sub)).unwrap().flatten() {
                                assert!(!f.file_name().to_string_lossy().contains(".part-"), "P2 {tag}: temporary entry in a slot");
                            }
                        }
                    }
                }
            }
        }
    }
    assert!(fired >= 30, "only {fired} crashes fired");
}

/// CT12: the temporary screen view is removed on EVERY exit — a screen that panics included.
/// Without CT12 a panic (or a crash) left `.screen-*` behind.
#[test]
fn the_screen_view_is_removed_even_when_the_screen_panics() {
    let r = rig("ct12view");
    let id = rid("R-43");
    let files = two_target_candidate(&r, &id);
    let screen = |_: &RepairId, _: &crate::repair::screen::ScreenCandidate, _: &Path| -> Result<(), String> { panic!("screen blew up") };
    let env = r.env(&NoFaults, ApplyForm::Exchange);
    let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        apply_candidate_in(&env, &id, &files, &screen, &pass_check, &pass_ref, &no_repair_lock)
    }));
    assert!(res.is_err());
    let left: Vec<String> = std::fs::read_dir(vault::vault_dir_in(&r.root, &id)).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).filter(|n| n.starts_with(".screen")).collect();
    assert!(left.is_empty(), "{left:?}");
    assert_eq!(std::fs::read(r.pack.join("hooks/a1.sh")).unwrap(), b"PRE-1\n");
}

/// CT12: the view's bytes are bound back to the slot states after the screen — a view changed while
/// the screen ran (the screen judged other bytes than those that would be applied) declines.
#[test]
fn a_view_changed_during_the_screen_declines() {
    let r = rig("ct12rebind");
    let id = rid("R-44");
    let files = two_target_candidate(&r, &id);
    let screen = |_: &RepairId, c: &crate::repair::screen::ScreenCandidate, _: &Path| -> Result<(), String> {
        std::fs::write(c.files.join("hooks/a1.sh"), b"OTHER\n").unwrap();
        Ok(())
    };
    let env = r.env(&NoFaults, ApplyForm::Exchange);
    let rep = apply_candidate_in(&env, &id, &files, &screen, &pass_check, &pass_ref, &no_repair_lock).unwrap();
    assert!(matches!(rep.outcome, ApplyOutcome::Declined(ref m) if m.contains("보호장치 검사가 본 내용")), "{:?}", rep.outcome);
    assert_eq!(std::fs::read(r.pack.join("hooks/a1.sh")).unwrap(), b"PRE-1\n");
}

// ───────────────────────────── RULINGS DR3 — the runner never blocks ─────────────────────────────

/// DR3: after a SUCCESSFUL exit, a descendant that closed stdout but lives on in the child's group is
/// killed and reported (never left running while later checks start). Without DR3 the success path
/// returned `Ok` with the descendant alive.
#[cfg(unix)]
#[test]
fn a_live_descendant_after_a_successful_exit_is_killed_and_reported() {
    use crate::SpawnPolicy as _;
    let mut cmd = std::process::Command::new("/bin/sh");
    cmd.arg("-c").arg("sleep 30 >/dev/null 2>&1 & echo ok $!");
    cmd.spawn_policy(crate::ChildLifetime::GroupScoped);
    cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null());
    let t = std::time::Instant::now();
    let res = run_bounded(cmd, Duration::from_secs(4));
    assert!(t.elapsed() < Duration::from_secs(6), "within the deadline (+ scheduling)");
    let msg = match res {
        Err(RepairError::CheckFailed(m)) => m,
        other => panic!("a live descendant is reported: {other:?}"),
    };
    assert!(msg.contains("자손이 남아"), "{msg}");
    let pid: i32 = msg.split("ok ").nth(1).and_then(|t| t.split_whitespace().next()).and_then(|p| p.parse().ok()).expect("pid");
    assert!(pid_gone(pid), "the descendant (pid {pid}) was killed");
}

/// DR3: a child that ignores SIGTERM (and would take long to end) is stopped and the call returns
/// within its deadline; a kill that FAILS is reported truthfully, the call still returns.
#[cfg(unix)]
#[test]
fn a_stubborn_child_and_a_failing_kill_never_hold_the_runner() {
    use crate::SpawnPolicy as _;
    let mk = || {
        let mut cmd = std::process::Command::new("/bin/sh");
        cmd.arg("-c").arg("trap '' TERM; sleep 30");
        cmd.spawn_policy(crate::ChildLifetime::GroupScoped);
        cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null());
        cmd
    };
    let t = std::time::Instant::now();
    assert!(matches!(run_bounded(mk(), Duration::from_secs(2)), Err(RepairError::CheckTimeout(_))));
    assert!(t.elapsed() < Duration::from_secs(4));
    FAIL_GROUP_KILL.with(|c| c.set(libc::EPERM));
    let t = std::time::Instant::now();
    let res = run_bounded(mk(), Duration::from_secs(2));
    FAIL_GROUP_KILL.with(|c| c.set(0));
    assert!(t.elapsed() < Duration::from_secs(4), "returns in every case");
    let msg = format!("{res:?}");
    // leave nothing behind: stop the group this test kept alive, by its exact id from the report
    if let Some(pgid) = msg.split("pgid ").nth(1).and_then(|t| t.split(|c: char| !c.is_ascii_digit()).next()).and_then(|p| p.parse::<i32>().ok()) {
        // SAFETY: the test's own child's group.
        unsafe {
            libc::killpg(pgid, libc::SIGKILL);
        }
    }
    assert!(msg.contains("멈추지 못했습니다"), "the failed kill is reported: {msg}");
}

// ───────────────────────────── RULINGS DR8 — a dry run must name everything asked ─────────────────────────────

/// DR8: a dry run that is valid JSON but empty (`{"hooks": []}`), or names the hook without a
/// complete captured state, is NOT a measurement — the step declines before any record, tombstone
/// or real registrar run, exactly like an error. Without DR8 it proceeded with empty captured targets.
#[test]
fn an_empty_or_incomplete_dry_run_declines_like_an_error() {
    for answer in [
        r#"{"hooks": []}"#.to_string(),
        r#"{"hooks": [{"event": "Stop", "script": "e.sh", "lifeline": false}]}"#.to_string(),
        r#"{"hooks": [{"event": "Stop", "script": "e.sh", "desired": "registered", "targets": [{"path": "/x/settings.json"}]}]}"#.to_string(),
        report_json(&[("Stop", "E.sh", "registered", &[])]),
    ] {
        let r = rig("dr8dry");
        let id = rid("R-37");
        hook_record(&r, &id, "Stop", "e.sh");
        let calls = RefCell::new(Vec::new());
        let ans = answer.clone();
        let registrar = move |_: &Path, dry: bool| -> Result<String, RepairError> {
            calls.borrow_mut().push(dry);
            assert!(dry, "the real run never happens");
            Ok(ans.clone())
        };
        let res = hook_lines_step_in(&r.root, &r.pack, &id, &r.lane, HookStep::Unregister, &registrar, 5.0);
        assert!(res.is_err(), "{answer}: {res:?}");
        let (ops, _) = wal::list_in(&r.root, &id);
        assert!(ops.is_empty(), "{answer}: nothing recorded");
        assert!(crate::repair::tombstones::load_in(&r.root, &r.lane).unwrap().is_empty(), "{answer}: no tombstone");
    }
}

/// DR8: a nested put-back that does not succeed is never described as done — the abandon of an
/// interrupted Windows operation whose copy cannot be placed says it could NOT put it back.
#[test]
fn an_unsuccessful_put_back_is_never_described_as_done() {
    let src = include_str!("apply.rs");
    let prod = &src[..src.find("#[cfg(test)]\n#[path = \"apply_tests.rs\"]").unwrap()];
    for done_sentence in ["사본을 되놓았습니다(", "다시 살렸습니다(", "다시 놓았습니다("] {
        assert!(!prod.contains(done_sentence), "a success sentence carries an unchecked outcome: {done_sentence}");
    }
}

// ───────────────────────────── RULINGS DR9 — recovery touches only what no live operation owns ─────────────────────────────

/// DR9: an unfinished operation that a live thread of this process is executing — a hook step
/// paused after its record (it holds no pack-apply lock by design) or a file operation — is never
/// resumed or abandoned by recovery; once its owner is gone it is recovered. Without DR9 the tick
/// re-ran the live hook step / file operation.
#[cfg(target_os = "macos")]
#[test]
fn recovery_never_touches_a_live_operation() {
    let r = rig("dr9live");
    // a hook step paused after its record
    let id = rid("R-38");
    hook_record(&r, &id, "Stop", "l.sh");
    lane_row(&r, &id, Vec::new(), Vec::new());
    let op = OpRecord {
        schema: 1,
        op: wal::new_op_id(1.0, "hook-unregister"),
        repair: id.clone(),
        lane_key: r.lane.clone(),
        kind: "hook-unregister".into(),
        form: "hook".into(),
        rel: String::new(),
        pre_existed: false,
        expected: String::new(),
        incoming: String::new(),
        pack_generation: Some(pack_generation(&r.pack)),
        slot: None,
        slot_identity: None,
        live_identity: None,
        incoming_copy: None,
        incoming_copy_identity: None,
        step: "recorded".into(),
        hooks: vec![wal::HookLine { event: "Stop".into(), script: "l.sh".into(), targets: Vec::new() }],
        finished: false,
        outcome: None,
        note: String::new(),
        created_at: 1.0,
        updated_at: 1.0,
    };
    wal::write_in(&r.root, &op).unwrap();
    let ok = |_: &Path, _: bool| -> Result<String, RepairError> { Ok(report_json(&[("Stop", "l.sh", "unregistered", &[])])) };
    let owner = wal::LiveOp::enter(&op.op);
    assert_eq!(resume_hook_steps_in(&r.root, &r.pack, &r.lane, &ok, &no_repair_lock, 3.0), 0, "a live hook step is not resumed");
    assert_eq!(wal::unfinished_for_lane(&r.root, &r.lane).len(), 1);
    drop(owner);
    assert_eq!(resume_hook_steps_in(&r.root, &r.pack, &r.lane, &ok, &no_repair_lock, 3.0), 1, "a dead incarnation's step is recovered");
    // a file operation stopped after its record, still owned
    let before = r.put("hooks/l2.sh", b"OLD\n", 0o755);
    let (src, incoming) = r.source("l2-new", b"NEW\n", 0o755);
    let spec = OpSpec { id: rid("R-39"), rel: "hooks/l2.sh".into(), kind: "revert".into(), expected: before, incoming, source: Some(src), generation: None };
    let stop = StopOn { rel: "hooks/l2.sh", point: Point::AfterRecord, fired: Cell::new(false) };
    assert!(run_op(&r.env(&stop, ApplyForm::Exchange), &spec).is_err());
    let rec = wal::unfinished_for_lane(&r.root, &r.lane).into_iter().next().unwrap();
    let owner = wal::LiveOp::enter(&rec.op);
    assert_eq!(resume_interrupted_in(&r.env(&NoFaults, ApplyForm::Exchange)).unwrap(), 0);
    drop(owner);
    assert_eq!(resume_interrupted_in(&r.env(&NoFaults, ApplyForm::Exchange)).unwrap(), 1);
}

// ───────────────────────────── RULINGS DR1 / DX2 — bindings and the commit point ─────────────────────────────

/// DR1 (blocking I1): an apply interrupted after its first target, then an UPDATE installs a new
/// generation — the recovery puts NOTHING back (the intent carried the generation it was bound to),
/// closes the intent superseded and voids the approval. Without DR1 the G1 pre-image was restored
/// into G2.
#[cfg(target_os = "macos")]
#[test]
fn an_interrupted_apply_is_never_undone_across_a_generation_change() {
    let r = rig("dr1gen");
    let id = rid("R-45");
    let files = two_target_candidate(&r, &id);
    let stop = StopOn { rel: "hooks/a2.sh", point: Point::AfterRecord, fired: Cell::new(false) };
    assert!(apply_candidate_in(&r.env(&stop, ApplyForm::Exchange), &id, &files, &|_, _, _| Ok(()), &pass_check, &pass_ref, &no_repair_lock).is_err());
    std::fs::write(r.pack.join(".pack-version"), "0.15.1\n").unwrap();
    let calm = r.env(&NoFaults, ApplyForm::Exchange);
    resume_interrupted_in(&calm).unwrap();
    let lines = recover_apply_intents_in(&calm, &no_repair_lock);
    assert!(lines[0].contains("세대"), "{lines:?}");
    assert_eq!(std::fs::read(r.pack.join("hooks/a1.sh")).unwrap(), b"NEW-1\n", "nothing put back across the generation change");
    assert_eq!(read_intent_in(&r.root, &id, &r.lane).unwrap().state, "superseded");
    assert!(ledger::read_record_in(&r.root, &id).unwrap().unwrap().approval.is_none(), "the approval is void");
}

/// DX2: a crash between the record's durable transition to 시험 기간 and the lane row — the record is
/// the commit point: the recovery completes FORWARD (row written, intent applied), never an undo of a
/// committed apply, never "intent undone while the record keeps its approval".
#[cfg(target_os = "macos")]
#[test]
fn a_committed_apply_is_completed_forward() {
    let r = rig("dx2commit");
    let id = rid("R-46");
    let files = two_target_candidate(&r, &id);
    let stop = StopOn { rel: "hooks/a1.sh", point: Point::AfterRecordCommit, fired: Cell::new(false) };
    assert!(apply_candidate_in(&r.env(&stop, ApplyForm::Exchange), &id, &files, &|_, _, _| Ok(()), &pass_check, &pass_ref, &no_repair_lock).is_err());
    assert!(stop.fired.get());
    assert!(ledger::read_lane_in(&r.root, &id, &r.lane).unwrap().is_none(), "the window: no row yet");
    let calm = r.env(&NoFaults, ApplyForm::Exchange);
    let lines = recover_apply_intents_in(&calm, &no_repair_lock);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(std::fs::read(r.pack.join("hooks/a1.sh")).unwrap(), b"NEW-1\n");
    assert_eq!(std::fs::read(r.pack.join("hooks/a2.sh")).unwrap(), b"NEW-2\n");
    assert_eq!(read_intent_in(&r.root, &id, &r.lane).unwrap().state, "applied");
    let row = ledger::read_lane_in(&r.root, &id, &r.lane).unwrap().unwrap();
    assert!(row.probation.is_some() && row.applied.len() == 2);
    assert_eq!(ledger::read_record_in(&r.root, &id).unwrap().unwrap().state, RepairState::Probation);
}
