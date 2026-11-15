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
                        expected: Recorded::for_test(expected.clone()),
                        incoming: incoming.clone(),
                        source: source.clone().map(Source::Fresh).unwrap_or(Source::None),
                        bound: Bound::now(&r.pack),
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
                        #[cfg(unix)]
                        {
                            use std::os::unix::fs::PermissionsExt;
                            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode)).unwrap();
                        }
                        #[cfg(not(unix))]
                        let _ = mode;
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
                        // EJ1 (old: absent only where a target state is absent): after a generation
                        // change the deposit-first abandonment places nothing of the old generation
                        assert!(
                            case.incoming.is_none()
                                || case.before.is_none()
                                || (matches!(action, Action::BumpGeneration) && case.form == ApplyForm::DepositFirst),
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
        expected: Recorded::for_test(r.live("hooks/a.sh")),
        incoming: st.clone(),
        source: Source::Fresh(src),
        bound: Bound::now(&r.pack),
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
        expected: Recorded::for_test(r.live("hooks/c.sh")),
        incoming: TargetState::Absent,
        source: Source::None,
        bound: Bound::now(&r.pack),
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
        expected: Recorded::for_test(TargetState::Absent),
        incoming: st,
        source: Source::Fresh(src),
        bound: Bound::now(&r.pack),
    })
    .unwrap();
    // GV1 (old expectation: Superseded): an occupied path is another writer's — a conflict
    assert!(matches!(d.outcome, ApplyOutcome::Declined(_)), "{:?}", d.outcome);
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
    // EJ1 (REWRITTEN needles — old: the direct calls `exchange(live, &slot.payload())`,
    // `exchange(&live, &slot.payload())`, `place_no_replace(live, &slot.payload())`): every exchange
    // and move into a slot now goes through THE door
    // GK2 (REWRITTEN needles — old: `DoorAct::Exchange { payload: &slot.payload() }`,
    // `DoorAct::MoveOut { into: &slot.payload() }`): the acts carry the recorded expected state
    for needle in ["DoorAct::Exchange { payload: &slot.payload(), expected: &spec.expected }", "DoorAct::MoveOut { into: &slot.payload(), expected: &spec.expected }"] {
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

/// OQ-D7 / H6(1) / GK2 (REWRITTEN — old expectations: the other writer's bytes, written between
/// the record and the exchange, were displaced into the vault and re-lived as a copy; an empty one
/// was left displaced with "the verified content" live; "both versions are in the vault"). Now the
/// door verifies the RECORDED expected state at the moment it acts: the other writer's bytes — valid
/// or an empty file caught mid-write — are never displaced; they stay live, the operation closes
/// superseded, nothing of theirs enters the vault.
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
            expected: Recorded::for_test(r.live("schedule.json")),
            incoming: st,
            source: Source::Fresh(src),
            bound: Bound::now(&r.pack),
        })
        .unwrap();
        // GV1 (old expectation: Superseded): another writer's bytes at the door — a conflict
        assert!(matches!(d.outcome, ApplyOutcome::Declined(_)), "{:?}", d.outcome);
        let _ = relived;
        let live = std::fs::read(r.pack.join("schedule.json")).unwrap();
        assert_eq!(live, other, "the other writer's bytes stay live, untouched");
        assert!(!r.vault_bytes().contains(&other), "nothing of the other writer's was displaced");
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
            reason: String::new(),
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
    // GV1 (old expectations: Superseded, the record Superseded): in the same pack generation the
    // bytes are another writer's — a CONFLICT: nothing changed, the record stays in force, the row
    // 충돌 with its report owed
    assert!(matches!(rep.outcome, ApplyOutcome::Declined(_)), "{:?}", rep.outcome);
    assert_eq!(std::fs::read(r.pack.join("hooks/m.sh")).unwrap(), b"UPDATE-WROTE\n", "nothing changed");
    assert_eq!(ledger::read_record_in(&r.root, &moved).unwrap().unwrap().state, RepairState::Probation);
    assert_eq!(ledger::read_lane_in(&r.root, &moved, &r.lane).unwrap().unwrap().state, LaneState::Conflict);
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
/// generation are unchanged; when the generation moved it ABANDONS, and — EJ1 (REWRITTEN; old
/// expectation `(bump) → OLD`: "a copy of the deposit placed back") — the abandonment carries the
/// operation's binding, so the door places NOTHING of the old generation into the new one: the path
/// stays empty, the deposit stays in the vault; an occupied path stops it, nothing deleted.
#[cfg(target_os = "macos")]
#[test]
fn windows_interruption_between_the_two_moves_resume_or_abandon() {
    for (bump, occupy, want) in [(false, false, Some(b"NEW\n".to_vec())), (true, false, None), (false, true, Some(b"THIRD\n".to_vec()))] {
        let r = rig("oqd8");
        r.put("bin/w.py", b"OLD\n", 0o644);
        let (src, st) = r.source("w.py", b"NEW\n", 0o644);
        let inj = Inject::new(Point::AfterFirstMove, Action::Stop, None, &r.pack);
        let env = r.env(&inj, ApplyForm::DepositFirst);
        let first = run_op(&env, &OpSpec { id: rid("R-9"), rel: "bin/w.py".into(), kind: "revert".into(), expected: Recorded::for_test(r.live("bin/w.py")), incoming: st, source: Source::Fresh(src), bound: Bound::now(&r.pack) });
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
        assert_eq!(std::fs::read(r.pack.join("bin/w.py")).ok(), want);
        assert!(r.vault_bytes().contains(&b"OLD\n".to_vec()), "the deposit is never deleted");
        assert!(wal::unfinished_for_lane(&r.root, &r.lane).is_empty());
    }
}

/// The staged init-pack swap under an operation (the whole pack directory replaced between the
/// record and the exchange) → the second install: the vault is outside the pack, every version
/// survives; the operation closes superseded (worker review). GK2 (REWRITTEN — old expectation:
/// "the bytes live at the exchange are in the vault" — the installed V2 was exchanged into a slot):
/// the door verifies the recorded state first, so the installed V2 is never displaced — it stays
/// live; the vault keeps the staged repair.
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
    let d = run_op(&env, &OpSpec { id: rid("R-10"), rel: "hooks/a.sh".into(), kind: "apply".into(), expected: Recorded::for_test(r.live("hooks/a.sh")), incoming: st, source: Source::Fresh(src), bound: Bound::now(&r.pack) }).unwrap();
    // GV1 (old expectation: Superseded): the swapped-in pack states the same generation, so its file
    // is another writer's bytes — a conflict
    assert!(matches!(d.outcome, ApplyOutcome::Declined(_)), "{:?}", d.outcome);
    // second install: the pack swapped again (pack.prev deleted by the installer)
    let _ = std::fs::remove_dir_all(r.base.join("pack.prev"));
    let vault = r.vault_bytes();
    assert_eq!(std::fs::read(r.pack.join("hooks/a.sh")).unwrap(), b"V2-INSTALLED\n", "the installed bytes stay live");
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
        finished: false, outcome: None, note: String::new(), created_at: 2.0, updated_at: 2.0, notice: None,
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
    let d = run_op(&env, &OpSpec { id: rid("R-34"), rel: "hooks/z.sh".into(), kind: "apply".into(), expected: Recorded::for_test(r.live("hooks/z.sh")), incoming: st, source: Source::Fresh(src), bound: Bound::now(&r.pack) }).unwrap();
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
    let spec = OpSpec { id: rid("R-6"), rel: "hooks/f.sh".into(), kind: "revert".into(), expected: Recorded::for_test(before), incoming: incoming.clone(), source: Source::Fresh(src), bound: Bound::now(&r.pack) };
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
/// CT3 / GK2 (REWRITTEN — old expectation: "the previous bytes are back, the unapproved ones are not
/// live", by a CHECKED operation whose expected state was the live bytes read after the exchange).
/// GK2: the engine replaces live bytes ONLY when they are exactly what its durable record says it
/// placed — a payload rewritten by another writer between the record and the exchange put bytes live
/// that no record vouches for: they are NOT exchanged back; the apply closes as conflict (never
/// applied, the approval void), the pre-image stays in the vault, the lane row says 충돌 for the
/// worker (and the hourly scan judges the file by its own check).
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
    assert_eq!(std::fs::read(r.pack.join(rel)).unwrap(), b"UNAPPROVED\n", "nothing exchanged back over bytes no record vouches for");
    assert!(r.vault_bytes().contains(&b"PRE\n".to_vec()), "the vault keeps the displaced pre-image");
    let row = ledger::read_lane_in(&r.root, &id, &r.lane).unwrap().unwrap();
    assert_eq!(row.state, LaneState::Conflict, "the row says 충돌");
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

#[cfg(unix)]
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
        notice: None,
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
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(slot.meta_pre(), std::fs::Permissions::from_mode(0o644)).unwrap();
    }
    #[cfg(not(unix))]
    {
        let mut perm = std::fs::metadata(slot.meta_pre()).unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        perm.set_readonly(false);
        std::fs::set_permissions(slot.meta_pre(), perm).unwrap();
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
                            for f in std::fs::read_dir(e.path().join(sub)).into_iter().flatten().flatten() {
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
        notice: None,
    };
    wal::write_in(&r.root, &op).unwrap();
    let ok = |_: &Path, _: bool| -> Result<String, RepairError> { Ok(report_json(&[("Stop", "l.sh", "unregistered", &[])])) };
    // EJ4 (REWRITTEN — old: the owner held `wal::LiveOp::enter(&op.op)`, the per-operation live set
    // that EJ4 folds into the per-repair guard): the owner holds the repair's guard
    let owner = crate::repair::guard::RepairGuard::try_take(&r.root, &id).unwrap();
    assert_eq!(resume_hook_steps_in(&r.root, &r.pack, &r.lane, &ok, &no_repair_lock, 3.0), 0, "a live hook step is not resumed");
    assert_eq!(wal::unfinished_for_lane(&r.root, &r.lane).len(), 1);
    drop(owner);
    assert_eq!(resume_hook_steps_in(&r.root, &r.pack, &r.lane, &ok, &no_repair_lock, 3.0), 1, "a dead incarnation's step is recovered");
    // a file operation stopped after its record, still owned
    let before = r.put("hooks/l2.sh", b"OLD\n", 0o755);
    let (src, incoming) = r.source("l2-new", b"NEW\n", 0o755);
    let spec = OpSpec { id: rid("R-39"), rel: "hooks/l2.sh".into(), kind: "revert".into(), expected: Recorded::for_test(before), incoming, source: Source::Fresh(src), bound: Bound::now(&r.pack) };
    let stop = StopOn { rel: "hooks/l2.sh", point: Point::AfterRecord, fired: Cell::new(false) };
    assert!(run_op(&r.env(&stop, ApplyForm::Exchange), &spec).is_err());
    let rec = wal::unfinished_for_lane(&r.root, &r.lane).into_iter().next().unwrap();
    let owner = crate::repair::guard::RepairGuard::try_take(&r.root, &rec.repair).unwrap();
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

// ───────────────────────────── RULINGS DR7 — one revert = one recoverable unit ─────────────────────────────

/// A committed general repair of two files in this lane (pre → post), in probation.
fn two_file_general(r: &Rig, id: &RepairId) {
    let mut changes = Vec::new();
    let mut committed = Vec::new();
    for (rel, pre, post) in [("hooks/v1.sh", b"PRE-1\n", b"POST-1\n"), ("hooks/v2.sh", b"PRE-2\n", b"POST-2\n")] {
        r.put(rel, pre, 0o755);
        let pre_img = ledger::copy_preimage_in(&r.root, id, &r.lane, rel, &r.pack.join(rel), 1.0).unwrap();
        let post_state = r.put(rel, post, 0o755);
        committed.push((rel.to_string(), post_state.canon()));
        changes.push(FileChange { rel: rel.into(), kind: ChangeKind::Edit, pre: pre_img, post: post_state, diff: None });
    }
    let mut rec = record(r, id, Classification::General, changes, RepairState::Probation);
    rec.committed = committed.clone();
    ledger::write_record_in(&r.root, &rec).unwrap();
    lane_row(r, id, committed, Vec::new());
}

/// DR7 (major I7): a revert interrupted after its first file — the next run completes the WHOLE
/// revert from its intent (the restored file counted, the remaining file restored, record and row
/// closed 되돌려짐), and so does a manual `revert` issued before that run. Without DR7 the later
/// revert saw the restored file ≠ the row's state and closed the repair superseded, leaving the
/// second file unreverted.
#[cfg(target_os = "macos")]
#[test]
fn a_revert_interrupted_between_its_files_is_completed_as_a_whole() {
    for via_manual in [false, true] {
        let r = rig("dr7revert");
        let id = rid("R-47");
        two_file_general(&r, &id);
        let stop = StopOn { rel: "hooks/v2.sh", point: Point::AfterRecord, fired: Cell::new(false) };
        assert!(revert_in(&r.env(&stop, ApplyForm::Exchange), &id, RevertCaller::Human, &no_hooks, &no_repair_lock).is_err());
        assert_eq!(std::fs::read(r.pack.join("hooks/v1.sh")).unwrap(), b"PRE-1\n");
        assert_eq!(std::fs::read(r.pack.join("hooks/v2.sh")).unwrap(), b"POST-2\n");
        assert_eq!(read_revert_intent_in(&r.root, &id, &r.lane).unwrap().state, "reverting");
        let calm = r.env(&NoFaults, ApplyForm::Exchange);
        resume_interrupted_in(&calm).unwrap();
        if via_manual {
            let rep = revert_in(&calm, &id, RevertCaller::Human, &no_hooks, &no_repair_lock).unwrap();
            assert_eq!(rep.outcome, ApplyOutcome::Applied, "{rep:?}");
        } else {
            let lines = recover_revert_intents_in(&calm, &no_hooks, &no_repair_lock);
            assert_eq!(lines.len(), 1, "{lines:?}");
        }
        assert_eq!(std::fs::read(r.pack.join("hooks/v2.sh")).unwrap(), b"PRE-2\n", "via_manual={via_manual}: the remaining file restored");
        assert_eq!(ledger::read_record_in(&r.root, &id).unwrap().unwrap().state, RepairState::Reverted);
        assert_eq!(read_revert_intent_in(&r.root, &id, &r.lane).unwrap().state, "reverted");
    }
}

/// DR7: the hook step removes only HOOK tombstones; a file's tombstone goes only when that file was
/// restored — a revert whose file restoration fails while its re-registration succeeds keeps the
/// file's tombstone. Without DR7 the hook step removed every tombstone of the repair.
#[cfg(target_os = "macos")]
#[test]
fn the_hook_step_removes_only_hook_tombstones() {
    let r = rig("dr7tomb");
    let id = rid("R-48");
    r.put("hooks/gone.sh", b"WAS\n", 0o755);
    let pre_img = ledger::copy_preimage_in(&r.root, &id, &r.lane, "hooks/gone.sh", &r.pack.join("hooks/gone.sh"), 1.0).unwrap();
    std::fs::remove_file(r.pack.join("hooks/gone.sh")).unwrap();
    let mut rec = record(&r, &id, Classification::General, vec![
        FileChange { rel: "hooks/gone.sh".into(), kind: ChangeKind::Edit, pre: pre_img, post: TargetState::Absent, diff: None },
        FileChange { rel: "settings:Stop:t.sh".into(), kind: ChangeKind::HookUnregister { event: "Stop".into(), script: "t.sh".into() }, pre: PreImage::Absent, post: TargetState::Absent, diff: None },
    ], RepairState::Probation);
    rec.committed = vec![("hooks/gone.sh".into(), "absent".into())];
    ledger::write_record_in(&r.root, &rec).unwrap();
    lane_row(&r, &id, vec![("hooks/gone.sh".into(), "absent".into())], Vec::new());
    let file_t = crate::repair::tombstones::Tombstone { rel: "hooks/gone.sh".into(), kind: crate::repair::tombstones::TombstoneKind::File, repair: id.clone() };
    let hook_t = crate::repair::tombstones::Tombstone { rel: "settings:Stop:t.sh".into(), kind: crate::repair::tombstones::TombstoneKind::Hook { event: "Stop".into(), script: "t.sh".into() }, repair: id.clone() };
    crate::repair::tombstones::record_in(&r.root, &r.lane, &file_t).unwrap();
    crate::repair::tombstones::record_in(&r.root, &r.lane, &hook_t).unwrap();
    // another writer takes the deleted file's place before it can be put back
    let inj = Inject::new(Point::AfterRecord, Action::ReplaceLive(b"SOMEONE\n".to_vec()), None, &r.pack);
    let registrar = |_: &Path, _: bool| -> Result<String, RepairError> { Ok(report_json(&[("Stop", "t.sh", "registered", &[])])) };
    let root = r.root.clone();
    let pack = r.pack.clone();
    let hooks = move |rid: &RepairId, lane: &LaneKey, step: HookStep| hook_lines_step_in(&root, &pack, rid, lane, step, &registrar, 9.0);
    let rep = revert_in(&r.env(&inj, ApplyForm::Exchange), &id, RevertCaller::Human, &hooks, &no_repair_lock).unwrap();
    assert!(!matches!(rep.outcome, ApplyOutcome::Applied), "{rep:?}");
    let left = crate::repair::tombstones::load_in(&r.root, &r.lane).unwrap();
    assert!(left.contains(&file_t), "the file was not restored: its tombstone stays: {left:?}");
    assert!(!left.contains(&hook_t), "the hook tombstone is removed by the hook step: {left:?}");
}

// ───────────────────────────── RULINGS EJ1 / EN3 / EN4 — the door, the binding, completeness ─────────────────────────────

/// EJ1 (codex r3 I1): a Windows deposit-first revert interrupted after its first move (the live file
/// in the vault), then G2 installed with that target absent — the recovery's abandonment would put
/// the G1 deposit back: the door refuses (the abandonment carries the operation's G1 binding), the
/// place stays absent, nothing of G1 lands in G2. Without EJ1 the nested placement ran unbound.
#[test]
fn an_abandonment_never_places_g1_bytes_into_g2() {
    let r = rig("ej1abandon");
    let before = r.put("bin/w2.py", b"G1-LIVE\n", 0o644);
    let (src, st) = r.source("w2-new", b"G1-NEW\n", 0o644);
    let spec = OpSpec { id: rid("R-50"), rel: "bin/w2.py".into(), kind: "revert".into(), expected: Recorded::for_test(before), incoming: st, source: Source::Fresh(src), bound: Bound::now(&r.pack) };
    let stop = StopOn { rel: "bin/w2.py", point: Point::AfterFirstMove, fired: Cell::new(false) };
    assert!(run_op(&r.env(&stop, ApplyForm::DepositFirst), &spec).is_err());
    assert_eq!(r.live("bin/w2.py"), TargetState::Absent, "the first move happened");
    std::fs::write(r.pack.join(".pack-version"), "0.15.1\n").unwrap();
    resume_interrupted_in(&r.env(&NoFaults, ApplyForm::DepositFirst)).unwrap();
    assert_eq!(r.live("bin/w2.py"), TargetState::Absent, "no G1 bytes placed into G2");
    assert!(wal::unfinished_for_lane(&r.root, &r.lane).is_empty());
    assert!(r.vault_bytes().contains(&b"G1-LIVE\n".to_vec()), "the G1 bytes stay in the vault");
}

/// EJ1 (codex r3 I2): a lane row that was not reconstructed yet, and a reconstructed row (its
/// generation UNKNOWN — ""), are missing / unknown members: the one resolution answers UNKNOWN,
/// never NO APPROVAL and never APPROVED.
#[test]
fn a_missing_or_reconstructed_row_makes_the_approved_copy_unknown() {
    let r = rig("ej1row");
    let id = rid("R-51");
    approved_and_applied(&r, &id, "hooks/r.sh", b"VENDOR\n", b"APPROVED\n");
    std::fs::remove_file(ledger::lane_path_in(&r.root, &id, &r.lane)).unwrap();
    assert!(matches!(resolve_approved_copy_in(&r.root, &r.pack, &r.lane, "hooks/r.sh"), ApprovedCopy::Unknown(_)), "missing row");
    let mut row_fields = Bound::Unknown.row_fields();
    assert_eq!(row_fields.0, "", "Unknown is written as the empty string");
    row_fields.0.clear();
    let committed = ledger::read_record_in(&r.root, &id).unwrap().unwrap().committed;
    lane_row(&r, &id, committed, Vec::new());
    let mut row = ledger::read_lane_in(&r.root, &id, &r.lane).unwrap().unwrap();
    row.probation.as_mut().unwrap().pack_generation = String::new();
    row.probation.as_mut().unwrap().closed = Some("대체됨 — 재구성".into());
    ledger::write_lane_in(&r.root, &row).unwrap();
    assert!(matches!(resolve_approved_copy_in(&r.root, &r.pack, &r.lane, "hooks/r.sh"), ApprovedCopy::Unknown(_)), "reconstructed row");
}

/// EJ1: every rename / exchange / placement into a live path of the repair module is inside `fn
/// door` (the form of `h08_target_write_site_census`): the production text of `apply.rs` calls
/// `exchange(` and `place_no_replace(` only inside the door, and `probation.rs` never.
#[test]
fn live_writes_go_through_the_one_door() {
    let src = include_str!("apply.rs");
    let prod = &src[..src.find("#[cfg(test)]\n#[path = \"apply_tests.rs\"]").unwrap()];
    let door_at = prod.find("fn door(").expect("the door");
    let door_end = door_at + prod[door_at..].find("\n}\n").unwrap();
    for needle in ["place_no_replace(", " exchange(", "(exchange(", "swap_paths("] {
        for (at, _) in prod.match_indices(needle) {
            let line_start = prod[..at].rfind('\n').map(|i| i + 1).unwrap_or(0);
            let line = &prod[line_start..prod[at..].find('\n').map(|i| at + i).unwrap_or(prod.len())];
            if line.trim_start().starts_with("//") || line.contains("use super::exchange") {
                continue;
            }
            assert!(at > door_at && at < door_end, "a live write outside the door: {}", line.trim());
        }
    }
    let prob = include_str!("probation.rs");
    let prob = &prob[..prob.find("#[cfg(test)]\n#[path").unwrap_or(prob.len())];
    for needle in ["place_no_replace(", "exchange(", "swap_paths(", "std::fs::rename("] {
        assert!(!prob.contains(needle), "probation.rs writes a live path itself: {needle}");
    }
}

/// EN3: the raw generation string is read in ONE place — `Bound::from_row` / `Bound::of`; nothing
/// else in the repair module compares `pack_generation` strings or the literal "unknown".
#[test]
fn the_generation_string_is_read_in_one_place() {
    for (name, src) in [("apply.rs", include_str!("apply.rs")), ("probation.rs", include_str!("probation.rs"))] {
        let prod = &src[..src.find("#[cfg(test)]\n#[path").unwrap_or(src.len())];
        let bound_impl = prod.find("impl Bound {").map(|a| (a, a + prod[a..].find("\n}\n").unwrap()));
        let gen_fn = prod.find("pub fn pack_generation(").map(|a| (a, a + prod[a..].find("\n}\n").unwrap()));
        for needle in ["\"unknown\"", "pack_generation !=", "pack_generation ==", ".version !=", ".version =="] {
            for (at, _) in prod.match_indices(needle) {
                let inside = |r: Option<(usize, usize)>| r.is_some_and(|(a, b)| at > a && at < b);
                assert!(inside(bound_impl) || inside(gen_fn), "{name}: a raw generation comparison outside Bound: {needle}");
            }
        }
    }
}

// ───────────────────────────── RULINGS EJ2 — every step of the runner is bounded ─────────────────────────────

/// EJ2 (codex r3 I3): a child that writes WITHOUT PAUSE — on stdout, and on stderr — never keeps the
/// runner reading: each turn reads at most a fixed amount per pipe, the one deadline is looked at,
/// the kept output is capped and the rest counted and reported. Without EJ2 the pump drained until
/// WouldBlock (never) and the deadline was never checked.
#[cfg(unix)]
#[test]
fn a_child_writing_without_pause_never_holds_the_runner() {
    use crate::SpawnPolicy as _;
    for script in ["yes", "yes >&2"] {
        let mut cmd = std::process::Command::new("/bin/sh");
        cmd.arg("-c").arg(script);
        cmd.spawn_policy(crate::ChildLifetime::GroupScoped);
        cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
        let t = std::time::Instant::now();
        let res = run_bounded(cmd, Duration::from_secs(2));
        assert!(t.elapsed() < Duration::from_secs(6), "{script}: returned at its deadline");
        let msg = format!("{res:?}");
        assert!(matches!(res, Err(RepairError::CheckTimeout(_))), "{script}: {msg}");
        assert!(msg.contains("세기만 하고 버렸습니다"), "{script}: the cap is reported: {msg}");
    }
}

/// EJ2: when a pipe cannot be made non-blocking, the child is not run on — it is killed at once and
/// the runner says so.
#[cfg(unix)]
#[test]
fn a_failed_non_blocking_setup_does_not_run_the_child() {
    use crate::SpawnPolicy as _;
    let mut cmd = std::process::Command::new("/bin/sleep");
    cmd.arg("30");
    cmd.spawn_policy(crate::ChildLifetime::GroupScoped);
    cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
    FAIL_NONBLOCK.with(|c| c.set(true));
    let t = std::time::Instant::now();
    let res = run_bounded(cmd, Duration::from_secs(10));
    FAIL_NONBLOCK.with(|c| c.set(false));
    assert!(t.elapsed() < Duration::from_secs(5), "killed at once, not run to its deadline");
    let msg = format!("{res:?}");
    assert!(msg.contains("비차단") && msg.contains("멈췄습니다"), "{msg}");
}

/// FG7 (codex r4 M8): a read interrupted again and again (EINTR on every call) never keeps the
/// runner: the interrupted read returns to the one loop, which looks at the deadline. Without FG7 the
/// pump retried the read in its own loop and never returned — the runner ran forever (this test then
/// fails on its own watchdog instead of hanging).
#[cfg(unix)]
#[test]
fn a_read_interrupted_again_and_again_never_holds_the_runner() {
    use crate::SpawnPolicy as _;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut cmd = std::process::Command::new("/bin/sh");
        cmd.arg("-c").arg("echo hi; sleep 30");
        cmd.spawn_policy(crate::ChildLifetime::GroupScoped);
        cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
        INJECT_EINTR.with(|c| c.set(true));
        let t = std::time::Instant::now();
        let res = run_bounded(cmd, Duration::from_secs(1));
        INJECT_EINTR.with(|c| c.set(false));
        let _ = tx.send((t.elapsed(), format!("{res:?}"), matches!(res, Err(RepairError::CheckTimeout(_)))));
    });
    let (took, msg, timed_out) = rx.recv_timeout(Duration::from_secs(8)).expect("the runner returned (no inner retry loop)");
    assert!(took < Duration::from_secs(4), "returned at its deadline: {took:?}");
    assert!(timed_out, "the child was stopped at the deadline: {msg}");
}

/// FG7 (codex r4 M8): after a failed non-blocking set-up, the reap of the killed child polls on the
/// run's ONE deadline — a short deadline is a short wait. Without FG7 the reap polled a fixed 50 × 10
/// ms of its own, past a 100 ms deadline.
#[cfg(unix)]
#[test]
fn a_failed_setup_reaps_on_the_shared_deadline() {
    use crate::SpawnPolicy as _;
    let mut cmd = std::process::Command::new("/bin/sleep");
    cmd.arg("30");
    cmd.spawn_policy(crate::ChildLifetime::GroupScoped);
    cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
    FAIL_NONBLOCK.with(|c| c.set(true));
    HOLD_REAP.with(|c| c.set(true));
    let t = std::time::Instant::now();
    let res = run_bounded(cmd, Duration::from_millis(100));
    let took = t.elapsed();
    HOLD_REAP.with(|c| c.set(false));
    FAIL_NONBLOCK.with(|c| c.set(false));
    assert!(took < Duration::from_millis(400), "the reap waited on the shared 100 ms deadline, not 500 ms: {took:?}");
    let msg = format!("{res:?}");
    assert!(msg.contains("비차단") && msg.contains("따로 거두게"), "{msg}");
}

/// FG5 (codex r4 M6): the apply's supersession of older repairs TRIES each older repair's guard — an
/// older repair whose guard is held (a re-check, a return, a recovery owns it) is not written; once
/// the guard is free it is superseded. Without FG5 it was written under the mutex alone.
#[test]
fn the_apply_supersession_defers_a_guarded_older_repair() {
    let r = rig("fg5apply");
    let a = rid("R-1");
    let rec = committed_general(&r, &a, "hooks/q.sh", Some(b"PRE\n"), Some(b"POST\n"));
    let mut newer = rec.clone();
    newer.id = rid("R-9");
    let held = crate::repair::guard::RepairGuard::try_take(&r.root, &a).unwrap();
    let done = supersede_older_in(&r.root, &r.lane, &newer, &no_repair_lock).unwrap();
    assert!(done.is_empty(), "not written while its guard is held: {done:?}");
    assert_eq!(ledger::read_record_in(&r.root, &a).unwrap().unwrap().state, RepairState::Probation);
    drop(held);
    let done = supersede_older_in(&r.root, &r.lane, &newer, &no_repair_lock).unwrap();
    assert_eq!(done, vec![a.clone()]);
    assert_eq!(ledger::read_record_in(&r.root, &a).unwrap().unwrap().state, RepairState::Superseded);
}

// ───────────────────────────── RULINGS FI1 — the binding of a put-back is its bytes' generation ─────────────────────────────

/// The approved apply of `approved_and_applied` with `between` run after the begin (the pre-image
/// copy) and before the apply, and `faults` during the apply.
fn approved_with(r: &Rig, id: &RepairId, rel: &str, between: &dyn Fn(&Rig), faults: &dyn Faults) -> ApplyReport {
    r.put(rel, b"PRE\n", 0o755);
    let pre_img = ledger::copy_preimage_in(&r.root, id, &r.lane, rel, &r.pack.join(rel), 1.0).unwrap();
    between(r);
    let staging = ledger::staging_dir_in(&r.root, id);
    std::fs::create_dir_all(staging.join(rel).parent().unwrap()).unwrap();
    std::fs::write(staging.join(rel), b"POST\n").unwrap();
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
    let env = r.env(faults, ApplyForm::Exchange);
    apply_candidate_in(&env, id, &[CandidateFile { rel: rel.into(), staged: staging.join(rel), mode: 0o755 }], &|_, _, _| Ok(()), &pass_check, &pass_ref, &no_repair_lock).unwrap()
}

fn pre_image_dir(r: &Rig, id: &RepairId) -> PathBuf {
    match &ledger::read_record_in(&r.root, id).unwrap().unwrap().files[0].pre {
        PreImage::Slot { slot_dir, .. } => slot_dir.clone(),
        PreImage::Absent => panic!("no pre-image slot"),
    }
}

/// FI1 (codex r4 addendum): the pack is updated BETWEEN the begin (the pre-image copy, which records
/// the generation its bytes were live in) and the commit — the commit still commits, but the repair
/// is not put back by the mechanism and the row SAYS so; a revert is refused, nothing put back.
/// Without FI1 the row carried the apply's generation and a revert put the old generation's bytes
/// back.
#[cfg(target_os = "macos")]
#[test]
fn an_update_between_begin_and_commit_commits_a_repair_that_is_not_put_back() {
    let r = rig("fi1begin");
    let id = rid("R-11");
    let rep = approved_with(&r, &id, "hooks/fi.sh", &|r| std::fs::write(r.pack.join(".pack-version"), "0.15.1\n").unwrap(), &NoFaults);
    assert_eq!(rep.outcome, ApplyOutcome::Applied, "{rep:?}");
    assert_eq!(vault::slot_generation(&pre_image_dir(&r, &id)).unwrap().unwrap().version, "0.15.0", "the begin recorded its bytes' generation");
    let row = ledger::read_lane_in(&r.root, &id, &r.lane).unwrap().unwrap();
    let closed = row.probation.as_ref().unwrap().closed.clone().unwrap_or_default();
    assert!(closed.contains(NOT_REVERTIBLE_SENTENCE), "the row says so: {closed}");
    let rec = ledger::read_record_in(&r.root, &id).unwrap().unwrap();
    assert!(!revertible_here(&rec, &row, &r.lane));
    let env = r.env(&NoFaults, ApplyForm::Exchange);
    assert!(revert_in(&env, &id, RevertCaller::Human, &no_hooks, &no_repair_lock).is_err(), "refused");
    assert_eq!(std::fs::read(r.pack.join("hooks/fi.sh")).unwrap(), b"POST\n", "nothing put back");
}

/// FI1: the update lands INSIDE the commit's window — after the record is written, before the row:
/// the row, written after it, compares the pre-images' generation with the generation NOW and says
/// the repair is not put back. Without FI1 the row carried the generation of the apply's start.
#[cfg(target_os = "macos")]
#[test]
fn an_update_inside_the_commit_window_is_said_on_the_row() {
    struct UpdateAtCommit;
    impl Faults for UpdateAtCommit {
        fn at(&self, p: Point, live: &Path, _: Option<&Slot>) -> bool {
            if p == Point::AfterRecordCommit {
                let pack = live.parent().unwrap().parent().unwrap();
                std::fs::write(pack.join(".pack-version"), "0.15.2\n").unwrap();
            }
            false
        }
    }
    let r = rig("fi1window");
    let id = rid("R-12");
    let rep = approved_with(&r, &id, "hooks/fw.sh", &|_| {}, &UpdateAtCommit);
    assert_eq!(rep.outcome, ApplyOutcome::Applied, "{rep:?}");
    let closed = ledger::read_lane_in(&r.root, &id, &r.lane).unwrap().unwrap().probation.unwrap().closed.unwrap_or_default();
    assert!(closure_not_revertible(&closed), "{closed}");
}

/// FI1: a slot whose metadata records no generation is Unknown — its bytes are never put back (a
/// revert closes 대체됨 and changes nothing); and the generation read when a slot is filled obeys
/// FG1 — a corrupt pack state at the begin leaves the field absent.
#[cfg(target_os = "macos")]
#[test]
fn a_slot_without_its_generation_is_never_put_back() {
    let r = rig("fi1unknown");
    let id = rid("R-13");
    committed_general(&r, &id, "hooks/fu.sh", Some(b"PRE\n"), Some(b"POST\n"));
    let dir = pre_image_dir(&r, &id);
    assert!(vault::slot_generation(&dir).unwrap().is_some(), "recorded at the begin");
    // the field removed: an older slot
    let p = dir.join("meta").join("pre.json");
    let mut v: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
    v.as_object_mut().unwrap().remove("pack_generation");
    std::fs::write(&p, serde_json::to_vec(&v).unwrap()).unwrap();
    assert_eq!(vault::slot_generation(&dir).unwrap(), None);
    assert_eq!(put_back_bound(Some(&dir), &Bound::now(&r.pack)), Bound::Unknown);
    let env = r.env(&NoFaults, ApplyForm::Exchange);
    let rep = revert_in(&env, &id, RevertCaller::Human, &no_hooks, &no_repair_lock).unwrap();
    assert!(matches!(rep.outcome, ApplyOutcome::Superseded(_)), "{rep:?}");
    assert_eq!(std::fs::read(r.pack.join("hooks/fu.sh")).unwrap(), b"POST\n", "nothing put back");
    // FG1 at the fill: a corrupt pack state → no generation recorded
    let id2 = rid("R-14");
    std::fs::write(r.pack.join(crate::pack::PACK_STATE_FILE), b"{ corrupt").unwrap();
    r.put("hooks/fc.sh", b"X\n", 0o755);
    let pre = ledger::copy_preimage_in(&r.root, &id2, &r.lane, "hooks/fc.sh", &r.pack.join("hooks/fc.sh"), 2.0).unwrap();
    let PreImage::Slot { slot_dir, .. } = pre else { panic!("slot") };
    assert_eq!(vault::slot_generation(&slot_dir).unwrap(), None, "an unreadable generation is never recorded as one");
}

// ───────────────────────────── RULINGS FG1 — the remaining reads of the engine ─────────────────────────────

/// FG1: an interrupted operation whose recorded state cannot be read is a REPORTED recovery error —
/// never resumed as if its expected state were `absent`; and an unreadable lane row makes the
/// approval's use Unknown (never NotConsumed). Without FG1 `parse_canon(..).unwrap_or(Absent)`
/// resumed the operation against an invented state and the row's read error was skipped.
#[test]
fn an_unreadable_recorded_state_is_an_error_never_absent() {
    let r = rig("fg1canon");
    let id = rid("R-21");
    committed_general(&r, &id, "hooks/c.sh", Some(b"PRE\n"), Some(b"POST\n"));
    let op = OpRecord {
        schema: 1,
        op: "20261114-apply-x".into(),
        repair: id.clone(),
        lane_key: r.lane.clone(),
        kind: "apply".into(),
        form: "exchange".into(),
        rel: "hooks/c.sh".into(),
        pre_existed: true,
        expected: "file:garbage".into(),
        incoming: "absent".into(),
        pack_generation: None,
        slot: None,
        slot_identity: None,
        live_identity: None,
        incoming_copy: None,
        incoming_copy_identity: None,
        step: "staged".into(),
        hooks: Vec::new(),
        finished: false,
        outcome: None,
        note: String::new(),
        created_at: 1.0,
        updated_at: 1.0,
        notice: None,
    };
    wal::write_in(&r.root, &op).unwrap();
    let env = r.env(&NoFaults, ApplyForm::Exchange);
    let (_, errors) = resume_interrupted_report_in(&env).unwrap();
    assert!(errors.iter().any(|(i, why)| *i == id && why.contains("기록된 상태를 읽을 수 없습니다")), "{errors:?}");
    assert!(!wal::read_in(&r.root, &id, &op.op).unwrap().unwrap().finished, "left unfinished (retried, reported)");
    assert_eq!(std::fs::read(r.pack.join("hooks/c.sh")).unwrap(), b"POST\n", "nothing touched");
    // an unreadable lane row: the approval's use is Unknown
    std::fs::write(ledger::lane_path_in(&r.root, &id, &r.lane), b"{ broken").unwrap();
    assert!(matches!(crate::repair::consumed::approval_consumed_in(&r.root, &r.lane, "rqr-x"), crate::repair::consumed::ApprovalUse::Unknown(_)));
}

// ───────────────────────────── RULINGS FG6 / FG1 — the censuses of the repair module ─────────────────────────────

/// Every production file of the repair module, read from disk (so a NEW file is seen), its test
/// modules cut out (GK6: by their braces — the code after a test module is still counted):
/// (file name, text).
fn census_sources() -> Vec<(String, String)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join("repair");
    let mut out = Vec::new();
    for e in std::fs::read_dir(&dir).unwrap() {
        let p = e.unwrap().path();
        let name = p.file_name().unwrap().to_str().unwrap().to_string();
        if !name.ends_with(".rs") || name.ends_with("_tests.rs") {
            continue;
        }
        let src = std::fs::read_to_string(&p).unwrap();
        out.push((name, census_prod(&src)));
    }
    out.sort();
    out
}

/// RULINGS GK6 — a file's production text: every `#[cfg(test)]` MODULE is cut out — an out-of-line
/// `mod x;` (with its attribute lines) or an inline `mod x { … }` up to its matching brace — and
/// nothing else: the file is never truncated, so production code after a test module is counted;
/// other test-only items (an injection helper, a thread-local) stay counted.
fn census_prod(src: &str) -> String {
    let mark = "\n#[cfg(test)]\n";
    let mut out = String::new();
    let mut at = 0;
    loop {
        let Some(i) = src[at..].find(mark) else {
            out.push_str(&src[at..]);
            return out;
        };
        let start = at + i;
        // the item after the attribute lines
        let mut item = start + mark.len();
        while src[item..].starts_with("#[") {
            item += src[item..].find('\n').map(|n| n + 1).unwrap_or(src.len() - item);
        }
        let head = src[item..].lines().next().unwrap_or("");
        let is_mod = head.starts_with("mod ") || head.starts_with("pub mod ") || head.starts_with("pub(crate) mod ");
        if !is_mod {
            out.push_str(&src[at..start + 1]);
            at = start + 1;
            continue;
        }
        out.push_str(&src[at..=start]);
        let line_end = item + head.len();
        at = if head.trim_end().ends_with(';') {
            line_end
        } else {
            match src[item..].find('{') {
                Some(o) => census_block_end(src, item + o),
                None => line_end,
            }
        };
    }
}

/// The index just after the `}` matching the `{` at `open` — strings, raw strings, characters and
/// comments skipped.
fn census_block_end(src: &str, open: usize) -> usize {
    let b = src.as_bytes();
    let (mut i, mut depth) = (open, 0usize);
    while i < b.len() {
        match b[i] {
            b'/' if b.get(i + 1) == Some(&b'/') => {
                i += src[i..].find('\n').unwrap_or(src.len() - i);
                continue;
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i += src[i..].find("*/").map(|n| n + 2).unwrap_or(src.len() - i);
                continue;
            }
            b'r' if (b.get(i + 1) == Some(&b'"') || b.get(i + 1) == Some(&b'#')) && (i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_')) => {
                let mut j = i + 1;
                while b.get(j) == Some(&b'#') {
                    j += 1;
                }
                if b.get(j) == Some(&b'"') {
                    let close = format!("\"{}", "#".repeat(j - i - 1));
                    i = j + 1 + src[j + 1..].find(&close).map(|n| n + close.len()).unwrap_or(src.len() - j - 1);
                    continue;
                }
            }
            b'"' => {
                let mut j = i + 1;
                while j < b.len() && b[j] != b'"' {
                    j += if b[j] == b'\\' { 2 } else { 1 };
                }
                i = j + 1;
                continue;
            }
            b'\'' => {
                if b.get(i + 1) == Some(&b'\\') {
                    i += 2 + src[i + 2..].find('\'').map(|n| n + 1).unwrap_or(0);
                    continue;
                }
                let ch_len = src[i + 1..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
                if b.get(i + 1 + ch_len) == Some(&b'\'') {
                    i += 2 + ch_len;
                    continue;
                }
            }
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    b.len()
}

/// The function a `fn` line declares (visibility and qualifiers skipped).
fn census_fn_name(t: &str) -> Option<String> {
    let mut s = t;
    loop {
        let before = s;
        for p in ["pub(crate) ", "pub(super) ", "pub ", "const ", "unsafe ", "async ", "extern \"C\" "] {
            if let Some(r) = s.strip_prefix(p) {
                s = r;
            }
        }
        if s == before {
            break;
        }
    }
    let r = s.strip_prefix("fn ")?;
    let name: String = r.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
    (!name.is_empty()).then_some(name)
}

/// Occurrences of `name(` not preceded by an identifier character (a call, a path or a method).
fn census_calls(line: &str, name: &str) -> usize {
    let pat = format!("{name}(");
    line.match_indices(&pat)
        .filter(|(i, _)| !line[..*i].chars().last().is_some_and(|c| c.is_ascii_alphanumeric() || c == '_'))
        .count()
}

/// (file, fn) → the count of `calls` and `literals` in production code; a line belongs to the last
/// `fn` line above it; comment lines and `fn` lines themselves are skipped.
fn census_counts(sources: &[(String, String)], calls: &[&str], literals: &[&str]) -> BTreeMap<(String, String), usize> {
    let mut m = BTreeMap::new();
    for (file, prod) in sources {
        let mut cur = "<module>".to_string();
        for line in prod.lines() {
            let t = line.trim_start();
            if t.starts_with("//") {
                continue;
            }
            // GK6: a declaration line's body (after its first `{`) is code of that function
            let line = match census_fn_name(t) {
                Some(n) => {
                    cur = n;
                    match line.find('{') {
                        Some(o) => &line[o + 1..],
                        None => continue,
                    }
                }
                None => line,
            };
            let n: usize = calls.iter().map(|c| census_calls(line, c)).sum::<usize>() + literals.iter().map(|l| line.matches(l).count()).sum::<usize>();
            if n > 0 {
                *m.entry((file.clone(), cur.clone())).or_insert(0) += n;
            }
        }
    }
    m
}

/// The violations of a census: an unlisted file of the module, a (file, fn) with sites but no row, a
/// row whose counts no longer add up, a class outside `classes`, a row for a function without sites.
fn census_violations(
    sources: &[(String, String)],
    calls: &[&str],
    literals: &[&str],
    table: &[(&str, &str, &str, usize, &str)],
    classes: &[&str],
) -> Vec<String> {
    let mut v = Vec::new();
    for (f, _) in sources {
        if !CENSUS_MODULE_FILES.contains(&f.as_str()) {
            v.push(format!("{f}: a file of the repair module the censuses do not list"));
        }
    }
    let counts = census_counts(sources, calls, literals);
    for ((f, func), n) in &counts {
        let rows: Vec<_> = table.iter().filter(|(cf, cfn, ..)| cf == f && cfn == func).collect();
        if rows.is_empty() {
            v.push(format!("{f}::{func}: {n} site(s), unclassified"));
            continue;
        }
        let listed: usize = rows.iter().map(|r| r.3).sum();
        if listed != *n {
            v.push(format!("{f}::{func}: {n} site(s), the census lists {listed} — classify the new one"));
        }
        for r in &rows {
            if !classes.contains(&r.2) {
                v.push(format!("{f}::{func}: class {} is not one of {classes:?}", r.2));
            }
        }
    }
    for (cf, cfn, ..) in table {
        if !counts.contains_key(&(cf.to_string(), cfn.to_string())) {
            v.push(format!("{cf}::{cfn}: listed, but has no site — remove the row"));
        }
    }
    v
}

/// FG6: every write primitive of the repair module.
const CENSUS_WRITE_CALLS: &[&str] = &[
    "write", "copy", "rename", "create_dir", "create_dir_all", "remove_file", "remove_dir", "remove_dir_all", "hard_link", "symlink",
    "set_permissions", "exchange", "swap_paths", "place_no_replace", "write_durable", "write_json_durable", "atomic_write_json",
    "atomic_write_bytes", "clonefile", "renamex_np", "copy_file_new", "copy_link_new", "create_new_with", "write_all", "set_len",
];
const CENSUS_WRITE_LITERALS: &[&str] = &[".create_new(true)", ".create(true)"];
// T7b (RULINGS HH9): one class of ONE row added — `pack-instance-id`, the pack-instance pass's only
// write in the pack (I8 step 3: `<pack>/.repair-pack-id`; its own bookkeeping, not pack content). The
// old list, quoted: ["door", "vault", "staging", "ledger"].
const CENSUS_WRITE_CLASSES: &[&str] = &["door", "vault", "staging", "ledger", "pack-instance-id"];
/// FG1: every site that can turn a read error into a value.
const CENSUS_READ_LITERALS: &[&str] = &[".unwrap_or_default()", ".ok()", ".unwrap_or(", ".unwrap_or_else(", ".exists()", "let Ok(", ".is_ok()", ".is_ok_and(", "Err(_) =>"];
const CENSUS_READ_CLASSES: &[&str] = &["evidence", "reported", "conservative", "not-a-read", "display", "test", "outside"];

fn write_census(sources: &[(String, String)]) -> Vec<String> {
    census_violations(sources, CENSUS_WRITE_CALLS, CENSUS_WRITE_LITERALS, WRITE_CENSUS, CENSUS_WRITE_CLASSES)
}

fn read_census(sources: &[(String, String)]) -> Vec<String> {
    census_violations(sources, &[], CENSUS_READ_LITERALS, READ_DEFAULT_CENSUS, CENSUS_READ_CLASSES)
}

/// RULINGS FG6 (codex r4 M7): every write primitive (write, copy, rename, create_dir*, exchange,
/// place, remove, …) in EVERY file of the repair module is classified — door / vault / staging /
/// ledger; an unclassified one, a new one in a classified function, or a file of the module the
/// census does not list fails. The module's declared files are exactly its files.
#[test]
fn every_write_of_the_repair_module_is_classified() {
    let sources = census_sources();
    let v = write_census(&sources);
    assert!(v.is_empty(), "{v:#?}");
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join("repair");
    let raw: Vec<String> = sources.iter().map(|(n, _)| std::fs::read_to_string(dir.join(n)).unwrap()).collect();
    let modrs = &raw[sources.iter().position(|(n, _)| n == "mod.rs").unwrap()];
    for (f, _) in &sources {
        if f == "mod.rs" {
            continue;
        }
        let m = f.trim_end_matches(".rs");
        let declared = modrs.contains(&format!("mod {m};")) || raw.iter().any(|t| t.contains(&format!("#[path = \"{f}\"]")));
        assert!(declared, "{f}: not a declared module of the repair module");
    }
}

/// FG6's mutation controls: a direct write, a copy, a rename, a directory creation — each added in a
/// new function — and a new module file each make the census fail; so does one more primitive in a
/// function already classified.
#[test]
fn the_write_census_catches_each_mutation() {
    let base = census_sources();
    let with = |file: &str, code: &str| -> Vec<(String, String)> {
        base.iter()
            .map(|(n, t)| (n.clone(), if n == file { format!("{t}\nfn census_mutant() {{\n    {code}\n}}\n") } else { t.clone() }))
            .collect()
    };
    for code in ["std::fs::write(p, b\"x\").unwrap();", "std::fs::copy(a, b).unwrap();", "std::fs::rename(a, b).unwrap();", "std::fs::create_dir_all(d).unwrap();"] {
        assert!(!write_census(&with("probation.rs", code)).is_empty(), "not caught: {code}");
    }
    let more: Vec<(String, String)> = base
        .iter()
        .map(|(n, t)| (n.clone(), if n == "wal.rs" { t.replacen("super::durable::write_json_durable(&p, &r)", "std::fs::write(&p, b\"\").ok();\n    super::durable::write_json_durable(&p, &r)", 1) } else { t.clone() }))
        .collect();
    assert_ne!(more, base, "the mutation applied");
    assert!(!write_census(&more).is_empty(), "one more primitive in a classified function");
    let mut new_file = base.clone();
    new_file.push(("census_mutant.rs".into(), "pub fn x() {}\n".into()));
    assert!(!write_census(&new_file).is_empty(), "a new module file");
    // GK6: a ONE-LINE function with a direct write, and a write in production code that follows a
    // test module, are counted
    let one_line: Vec<(String, String)> = base
        .iter()
        .map(|(n, t)| (n.clone(), if n == "probation.rs" { format!("{t}\nfn census_mutant(p: &std::path::Path) {{ std::fs::write(p, b\"x\").unwrap(); }}\n") } else { t.clone() }))
        .collect();
    assert!(!write_census(&one_line).is_empty(), "a one-line function with a direct write");
    let after_tests = census_prod("pub fn a() {}\n#[cfg(test)]\nmod tests {\n    fn t() { let s = \"}\"; let c = '}'; }\n}\nfn after(p: &Path) { std::fs::write(p, b\"x\").unwrap(); }\n");
    assert!(after_tests.contains("fn after"), "the code after a test module is kept: {after_tests}");
    assert!(!after_tests.contains("fn t()"), "the test module is cut out: {after_tests}");
}

/// RULINGS FG1 (codex r4 B1): every site of the repair module that can turn a read error into a value
/// (unwrap_or_default, .ok(), unwrap_or, exists(), let Ok(..) else, …) is listed with WHY it is not a
/// collapse of "could not read" into "not there" — absent on positive evidence, the error reported,
/// a strictly conservative default, not a read at all, display only, tests only, or another track's
/// file not changed by T6. An unlisted site fails.
#[test]
fn every_read_default_of_the_repair_module_is_listed() {
    let v = read_census(&census_sources());
    assert!(v.is_empty(), "{v:#?}");
}

/// FG1's mutation control: a read error turned into a value in a new function fails the census.
#[test]
fn the_read_census_catches_a_new_collapse() {
    let base = census_sources();
    let with: Vec<(String, String)> = base
        .iter()
        .map(|(n, t)| (n.clone(), if n == "probation.rs" { format!("{t}\nfn census_mutant(p: &Path) -> Vec<u8> {{\n    std::fs::read(p).ok().unwrap_or_default()\n}}\n") } else { t.clone() }))
        .collect();
    assert!(!read_census(&with).is_empty());
    // GK6: a ONE-LINE function turning a failed read into a default
    let one_line: Vec<(String, String)> = base
        .iter()
        .map(|(n, t)| (n.clone(), if n == "probation.rs" { format!("{t}\nfn census_mutant(p: &Path) -> Vec<u8> {{ std::fs::read(p).ok().unwrap_or_default() }}\n") } else { t.clone() }))
        .collect();
    assert!(!read_census(&one_line).is_empty(), "a one-line read collapse");
}

// T7b (RULINGS HH9): `propagate.rs` added — the old list, quoted, ended "… "probation.rs",
// "reapply_claim.rs", …" (27 files with T5b's `replay.rs` / `screen_corpus.rs`; now 28).
const CENSUS_MODULE_FILES: &[&str] = &["apply.rs", "boot_critical.rs", "boot_critical_derive.rs", "consumed.rs", "durable.rs", "exchange.rs", "grant.rs", "guard.rs", "intake.rs", "ledger.rs", "lock.rs", "mod.rs", "owner_origin.rs", "paths.rs", "platform.rs", "probation.rs", "propagate.rs", "reapply_claim.rs", "rehearsal.rs", "replay.rs", "roles.rs", "rules.rs", "screen.rs", "screen_corpus.rs", "tombstones.rs", "types.rs", "vault.rs", "wal.rs"];
const WRITE_CENSUS: &[(&str, &str, &str, usize, &str)] = &[
    ("apply.rs", "door", "door", 4, "the ONE door: exchange / place / move out / make parents"),
    ("apply.rs", "drop", "staging", 4, "the screen's view of the slots' bytes (apply_candidate_in), removed on every exit"),
    ("apply.rs", "incoming_copy", "vault", 1, "the repair's vault folder"),
    ("apply.rs", "incoming_copy", "staging", 1, "our own non-slot copy of a link to place"),
    ("apply.rs", "op_deposit_first", "staging", 3, "our own non-slot incoming copy removed"),
    ("apply.rs", "op_place", "staging", 1, "our own non-slot incoming copy removed"),
    ("apply.rs", "resume_op", "staging", 3, "our own non-slot incoming copy removed"),
    ("apply.rs", "run_op_depth", "vault", 1, "the repair's vault folder"),
    ("apply.rs", "second_move", "staging", 3, "our own non-slot incoming copy removed"),
    ("apply.rs", "write_intent", "ledger", 1, "the apply intent (durable)"),
    ("apply.rs", "write_revert_intent", "ledger", 1, "the revert intent (durable)"),
    ("durable.rs", "write_durable", "ledger", 6, "the durable writer of ledger files: temporary sibling, flush, rename"),
    ("durable.rs", "write_json_durable", "ledger", 1, "the durable writer of ledger files"),
    ("exchange.rs", "exchange", "door", 1, "the exchange primitive the door calls"),
    ("exchange.rs", "rename_excl", "door", 1, "the no-replace primitive under the door and the vault's publication"),
    ("grant.rs", "create_marker", "ledger", 2, "T4: a claim marker"),
    ("grant.rs", "lock_grant_in", "ledger", 3, "T4: the grant lock file (folder, open-create)"),
    ("grant.rs", "place_claim", "ledger", 8, "T4: a claim file placed no-replace"),
    ("grant.rs", "place_no_replace", "ledger", 3, "T4: its own no-replace placement of a claim"),
    ("grant.rs", "write_decision_in", "ledger", 1, "T4: the grant's decision file"),
    ("grant.rs", "write_pause_mark", "ledger", 1, "T4: the pause mark"),
    ("intake.rs", "record_in", "ledger", 1, "T3: an intake row"),
    ("ledger.rs", "allocate_id_in", "ledger", 2, "T3: a repair id folder"),
    ("ledger.rs", "copy_file_new", "vault", 4, "T3: the begin pre-image copy into a fresh slot"),
    ("ledger.rs", "copy_link_new", "vault", 1, "T3: the begin pre-image link into a fresh slot"),
    ("ledger.rs", "copy_preimage_in", "vault", 9, "T3 (FI1 seam): the begin pre-image slot and its meta/pre.json"),
    ("ledger.rs", "create_locked_in", "ledger", 2, "T3: an empty id folder removed after a failed create"),
    ("ledger.rs", "populate_staging_in", "staging", 8, "T3: the fixed working copy R-<n>/staging"),
    ("ledger.rs", "write_lane_durable_in", "ledger", 1, "FG4 seam: a lane row, durable"),
    ("ledger.rs", "write_lane_in", "ledger", 1, "T3: a lane row (T3's callers)"),
    ("ledger.rs", "write_record_in", "ledger", 1, "T3: a repair record"),
    ("owner_origin.rs", "create_token_in", "ledger", 4, "T4: an owner-origin token"),
    ("owner_origin.rs", "write_token_in", "ledger", 1, "T4: an owner-origin token"),
    ("probation.rs", "ack_notices_in", "ledger", 1, "the notice acknowledgements (durable)"),
    ("probation.rs", "attribute_timeout", "staging", 1, "our own scratch copy for the O3 control, removed"),
    ("probation.rs", "file_once", "ledger", 1, "the acknowledgements (durable)"),
    ("probation.rs", "hand_out_due_notices_in", "ledger", 1, "GK3 / GV2: the notices' hand-out counts (durable)"),
    ("probation.rs", "m10_sweep_full", "ledger", 1, "the M10 sub-ledger folder"),
    ("probation.rs", "m10_sweep_full", "staging", 1, "our own source copy of a put-back, removed"),
    ("probation.rs", "save_findings", "ledger", 1, "the findings view (durable)"),
    ("probation.rs", "scratch_pre_image", "staging", 5, "our own scratch copy for the O3 control"),
    ("probation.rs", "write_job_state", "ledger", 1, "the job state (durable)"),
    // T7b (RULINGS HH9) — the propagation's writes
    ("propagate.rs", "copy_checked", "staging", 1, "the lane's own scratch copy for the copy-check (11.2), removed on every exit"),
    ("propagate.rs", "copy_checked_bytes_of", "staging", 1, "the lane's own scratch copy for the copy-check (11.2), removed on every exit"),
    ("propagate.rs", "drop", "staging", 1, "the scratch folder removed on every exit"),
    ("propagate.rs", "replace_pack_instance", "pack-instance-id", 4, "I8 step (3): a different / malformed id unlinked, the new id created NEW, flushed — the pass's only write in the pack"),
    ("propagate.rs", "seal_committed_copy_in", "vault", 3, "HI2 / HJ2: the committed copy's fresh slot in R's vault"),
    ("propagate.rs", "sweep_own_scratch", "staging", 1, "a dead process's scratch of THIS lane, by its exact name"),
    ("propagate.rs", "write_job_state", "ledger", 1, "the lane's job state (durable)"),
    ("reapply_claim.rs", "claim_in", "ledger", 1, "the claims folder of a parked edit"),
    ("rehearsal.rs", "copy_dir", "staging", 1, "T5b: the rehearsal's own scratch copy of the pack layout"),
    ("rehearsal.rs", "run_in_copy", "staging", 3, "T5b: the rehearsal's own scratch copy, laid out and removed"),
    ("rehearsal.rs", "run_reference_check", "staging", 1, "T5b: the reference fixture in its own scratch root"),
    ("replay.rs", "drop", "staging", 1, "T5b: a scratch root / scratch socket removed when dropped"),
    ("replay.rs", "interpreter_dir", "staging", 2, "T5b: the interpreter's private folder in the replay's scratch base"),
    ("replay.rs", "lay_out", "staging", 3, "T5b: a file laid out in a scratch root (replay rules)"),
    ("replay.rs", "new", "staging", 2, "T5b: a fresh scratch root"),
    ("replay.rs", "note", "staging", 2, "T5b: the removal of a scratch tree (remove_tree's retry)"),
    ("replay.rs", "pump_stdin", "staging", 1, "T5b: the replayed child's standard-input pipe (not a file)"),
    ("replay.rs", "set_mode", "staging", 1, "T5b: the mode of a file in a scratch root"),
    ("replay.rs", "write_stub", "staging", 2, "T5b: the stub `cys` in a scratch root"),
    ("replay.rs", "write_stub_conf", "staging", 1, "T5b: the stub's fixed replies in a scratch root"),
    ("screen.rs", "fresh_dir", "staging", 1, "T5b: a fresh scratch folder of the screen's run"),
    ("screen.rs", "i3_template", "staging", 3, "T5b: the screen's own scratch root (the I3 template run)"),
    ("screen.rs", "lay_files", "staging", 1, "T5b: a variant laid out in the screen's own scratch root"),
    ("screen.rs", "make_nopy_usr_bin", "staging", 2, "T5b: the backstop cell's scratch /usr/bin stand-in"),
    ("screen.rs", "run_backstop_probe", "staging", 2, "T5b: the screen's own scratch root (the backstop cell)"),
    ("screen.rs", "run_hook_probe_inner", "staging", 3, "T5b: the screen's own scratch root (a hook probe)"),
    ("screen.rs", "run_i3", "staging", 7, "T5b: the screen's own scratch root (an I3 run)"),
    ("screen.rs", "seed_budget", "staging", 2, "T5b: the gate's budget file in the screen's scratch HOME"),
    ("screen.rs", "touch", "staging", 1, "T5b: a flag file in the screen's scratch pack"),
    ("screen.rs", "write_verdict_in", "ledger", 1, "T5b: the recorded screen verdict (screen.json)"),
    ("tombstones.rs", "write_file", "ledger", 1, "T3: the lane's tombstones (durable)"),
    ("vault.rs", "build_slot", "vault", 10, "a fresh slot built as a temporary sibling, placed no-replace"),
    ("vault.rs", "create_new_with", "vault", 4, "a new file inside a slot under construction"),
    ("vault.rs", "discard_temps_in", "vault", 1, "DX4: our exact temporary names only"),
    ("vault.rs", "publish_with", "vault", 7, "CT1 publication: temporary, flush, no-replace placement"),
    ("vault.rs", "seal_with", "vault", 4, "the seal: clone / copy into a temporary, placed no-replace"),
    ("vault.rs", "sweep_stale_temps", "vault", 2, "temporary entries of ended processes"),
    ("wal.rs", "write_in", "ledger", 1, "an operation record (durable)"),
];
const READ_DEFAULT_CENSUS: &[(&str, &str, &str, usize, &str)] = &[
    ("apply.rs", "after_seal_move_in", "conservative", 1, "as step4"),
    ("apply.rs", "apply_candidate_in", "not-a-read", 1, "a lookup in the record already read"),
    ("apply.rs", "begun_in_another_generation", "evidence", 1, "an unreadable generation → another generation: not revertible (FI1)"),
    ("apply.rs", "drop", "not-a-read", 5, "apply_candidate_in's tail: lookups in values already read"),
    ("apply.rs", "drop", "conservative", 3, "the approved copy not built / the reference not measured → F9 Unknown later / no failure attributed"),
    ("apply.rs", "dry_run_measurement", "reported", 1, "no rows → the step declines"),
    ("apply.rs", "exchange_prepared", "not-a-read", 1, "a path's parent"),
    ("apply.rs", "exchange_prepared", "conservative", 1, "the approved copy's identity unreadable → none recorded → F9 Unknown"),
    ("apply.rs", "f9_agreement", "reported", 4, "every failure is an Err of the agreement → the resolution answers Unknown"),
    ("apply.rs", "found_matcher_timeout_in", "not-a-read", 1, "a field of the settings already read"),
    ("apply.rs", "group_has_members", "not-a-read", 1, "a pid conversion"),
    ("apply.rs", "leads_own_group", "not-a-read", 1, "a pid conversion"),
    ("apply.rs", "mark_incomplete_apply", "not-a-read", 3, "fields of values already read"),
    ("apply.rs", "new", "not-a-read", 1, "a child's pipe (Windows reader thread)"),
    ("apply.rs", "now", "evidence", 1, "an unreadable generation IS the answer Unknown"),
    ("apply.rs", "origin_approval_triple_in", "conservative", 1, "RULINGS R2: false = applies nothing"),
    ("apply.rs", "pack_generation", "display", 1, "display only; decisions use Bound::now"),
    ("apply.rs", "parse_canon", "not-a-read", 1, "a mode digit string; a bad one fails the whole parse (canon_req: Err)"),
    ("apply.rs", "parse_reconcile", "not-a-read", 5, "fields of the registrar's output; completeness is judged by dry_run_measurement / the judge"),
    ("apply.rs", "pre_image_slot_for", "not-a-read", 1, "no slot named → an empty path → deposit_of fails (Err)"),
    ("apply.rs", "pre_of", "not-a-read", 1, "GK2: a lookup in the record already read (no such target = absent)"),
    ("apply.rs", "pump", "not-a-read", 1, "a child's pipe; an unended output is said"),
    ("apply.rs", "put_back_bound", "evidence", 1, "unreadable slot metadata → Unknown binding: never put back (FI1)"),
    ("apply.rs", "read_intent_in", "test", 2, "read by tests only; decisions use read_intent_checked_in"),
    ("apply.rs", "read_revert_intent_in", "test", 2, "read by tests only; decisions use read_revert_intent_checked_in"),
    ("apply.rs", "reconcile_hook_registrations_with", "reported", 1, "a hook not in the report → neither state → Err"),
    ("apply.rs", "recover_one_intent", "not-a-read", 4, "fields of the previous row, read with ? before (None = no row on evidence)"),
    ("apply.rs", "released", "conservative", 2, "T7b (HH1): a link in the parent chain / an unreadable live file → not known vendor content → the deletion is 충돌, nothing moved"),
    ("apply.rs", "resolve_approved_copy_in", "reported", 1, "FI2: an unreadable intent → LedgerUnreadable (EN4: suspended, reported)"),
    ("apply.rs", "resume_hook_steps_report_in", "not-a-read", 1, "a literal id"),
    ("apply.rs", "resume_hook_steps_report_in", "display", 1, "a message part"),
    ("apply.rs", "revert_in", "evidence", 1, "a row without probation → Unknown binding"),
    ("apply.rs", "run_bounded", "not-a-read", 1, "a child's exit poll in the grace; not reaped → said and handed to the detached reap"),
    ("apply.rs", "run_registrar", "not-a-read", 1, "the process environment (current executable)"),
    ("apply.rs", "second_move", "conservative", 1, "an unreadable deposit → not resumed: abandoned, only bytes the record vouches for go back"),
    ("apply.rs", "second_move", "not-a-read", 1, "the copy path, present when copy_ok"),
    ("apply.rs", "slot_binding", "evidence", 1, "GK1: unreadable slot metadata → Unknown binding: never put back"),
    ("apply.rs", "step4", "conservative", 1, "sealed copy if present else payload — verified against the expected state before use"),
    ("apply.rs", "unreadable_intents_in", "reported", 1, "an unlistable root is reported by the recovery and the passes"),
    ("apply.rs", "unreadable_ledger_member", "display", 1, "a path shown relative when it can be"),
    ("boot_critical.rs", "canonical_form", "outside", 2, "another track's file, not changed by T6"),
    ("boot_critical.rs", "classify_path", "outside", 1, "another track's file, not changed by T6"),
    ("boot_critical.rs", "classify_with", "outside", 3, "another track's file, not changed by T6"),
    ("boot_critical.rs", "file_identity", "outside", 2, "another track's file, not changed by T6"),
    ("boot_critical.rs", "link_count", "outside", 2, "another track's file, not changed by T6"),
    ("boot_critical.rs", "outside_hard_link", "outside", 2, "another track's file, not changed by T6"),
    ("boot_critical.rs", "pack_roots", "outside", 2, "another track's file, not changed by T6"),
    ("boot_critical.rs", "physical_form", "outside", 3, "another track's file, not changed by T6"),
    ("boot_critical.rs", "run_query", "outside", 2, "another track's file, not changed by T6"),
    ("boot_critical_derive.rs", "awakening_hooks", "outside", 2, "another track's file, not changed by T6"),
    ("boot_critical_derive.rs", "bindings_impl", "outside", 6, "another track's file, not changed by T6"),
    ("boot_critical_derive.rs", "by_construction_census", "outside", 5, "another track's file, not changed by T6"),
    ("boot_critical_derive.rs", "derive", "outside", 10, "another track's file, not changed by T6"),
    ("boot_critical_derive.rs", "file_kind_check", "outside", 1, "another track's file, not changed by T6"),
    ("boot_critical_derive.rs", "has_err_trap", "outside", 1, "another track's file, not changed by T6"),
    ("boot_critical_derive.rs", "has_errexit", "outside", 1, "another track's file, not changed by T6"),
    ("boot_critical_derive.rs", "is_clear_procedure_rel", "outside", 1, "another track's file, not changed by T6"),
    ("boot_critical_derive.rs", "is_discarding_launch", "outside", 2, "another track's file, not changed by T6"),
    ("boot_critical_derive.rs", "launch_targets", "outside", 1, "another track's file, not changed by T6"),
    ("boot_critical_derive.rs", "preflight_census", "outside", 1, "another track's file, not changed by T6"),
    ("boot_critical_derive.rs", "protection_closure", "outside", 1, "another track's file, not changed by T6"),
    ("boot_critical_derive.rs", "py_def_body", "outside", 1, "another track's file, not changed by T6"),
    ("boot_critical_derive.rs", "shell_commands", "outside", 1, "another track's file, not changed by T6"),
    ("boot_critical_derive.rs", "value", "outside", 1, "another track's file, not changed by T6"),
    ("durable.rs", "full_flush_file", "not-a-read", 2, "an errno"),
    ("durable.rs", "write_durable", "not-a-read", 2, "parts of a temporary name"),
    ("exchange.rs", "volume_serial", "conservative", 1, "Windows: an unknown volume → not the same filesystem → refused"),
    ("grant.rs", "age_of", "outside", 3, "another track's file, not changed by T6"),
    ("grant.rs", "claim_held", "outside", 2, "another track's file, not changed by T6"),
    ("grant.rs", "claim_numbers", "outside", 2, "another track's file, not changed by T6"),
    ("grant.rs", "current_generation_in", "outside", 2, "another track's file, not changed by T6"),
    ("grant.rs", "decide_in", "outside", 4, "another track's file, not changed by T6"),
    ("grant.rs", "forget_pause_memory", "outside", 1, "another track's file, not changed by T6"),
    ("grant.rs", "incarnation_started_at", "outside", 1, "another track's file, not changed by T6"),
    ("grant.rs", "may_open_new_round_in", "outside", 1, "another track's file, not changed by T6"),
    ("grant.rs", "mtime_epoch", "outside", 3, "another track's file, not changed by T6"),
    ("grant.rs", "new_round_refusal_in", "outside", 1, "another track's file, not changed by T6"),
    ("grant.rs", "note_owner_input_in", "outside", 1, "another track's file, not changed by T6"),
    ("grant.rs", "parse_versioned", "outside", 1, "another track's file, not changed by T6"),
    ("grant.rs", "pause_read_in", "outside", 6, "another track's file, not changed by T6"),
    ("grant.rs", "pause_write_lock", "outside", 1, "another track's file, not changed by T6"),
    ("grant.rs", "persist_pause", "outside", 6, "another track's file, not changed by T6"),
    ("grant.rs", "place_claim", "outside", 2, "another track's file, not changed by T6"),
    ("grant.rs", "place_no_replace", "outside", 1, "another track's file, not changed by T6"),
    ("grant.rs", "read_claim", "outside", 3, "another track's file, not changed by T6"),
    ("grant.rs", "refresh_pause_in", "outside", 1, "another track's file, not changed by T6"),
    ("grant.rs", "snapshot", "outside", 2, "another track's file, not changed by T6"),
    ("grant.rs", "try_claim_in", "outside", 5, "another track's file, not changed by T6"),
    ("grant.rs", "write_decision_in", "outside", 1, "another track's file, not changed by T6"),
    ("guard.rs", "drop", "not-a-read", 1, "a poisoned mutex recovered"),
    ("guard.rs", "is_held", "not-a-read", 1, "a poisoned mutex recovered"),
    ("guard.rs", "try_take", "not-a-read", 1, "a poisoned mutex recovered"),
    ("intake.rs", "first_stable_line", "outside", 1, "another track's file, not changed by T6"),
    ("intake.rs", "leading_exit_code", "outside", 1, "another track's file, not changed by T6"),
    ("intake.rs", "load_all_in", "outside", 2, "another track's file, not changed by T6"),
    ("intake.rs", "load_row", "outside", 1, "another track's file, not changed by T6"),
    ("ledger.rs", "allocate_id_in", "outside", 2, "another track's file, not changed by T6"),
    ("ledger.rs", "copy_preimage_in", "not-a-read", 1, "a path's last part"),
    ("ledger.rs", "copy_preimage_in", "evidence", 2, "GK1 seam: the generation read before and after the copy — unreadable → none recorded (Unknown)"),
    ("ledger.rs", "copy_preimage_in", "conservative", 1, "GM1: an unreadable copy is treated as a script whose mode may lag → no generation"),
    ("ledger.rs", "held_by_seat_in", "outside", 1, "another track's file, not changed by T6"),
    ("ledger.rs", "lane_rows_in", "outside", 1, "another track's file, not changed by T6"),
    ("ledger.rs", "list_ids_checked_in", "not-a-read", 1, "FG1 seam: a name that is not an id is not a repair"),
    ("ledger.rs", "list_ids_in", "outside", 3, "another track's file, not changed by T6"),
    ("ledger.rs", "list_slots_in", "outside", 6, "another track's file, not changed by T6"),
    ("ledger.rs", "now_epoch", "outside", 1, "another track's file, not changed by T6"),
    ("ledger.rs", "populate_staging_in", "outside", 1, "another track's file, not changed by T6"),
    ("ledger.rs", "read_json", "outside", 1, "another track's file, not changed by T6"),
    ("lock.rs", "claim_closed", "outside", 1, "another track's file, not changed by T6"),
    ("owner_origin.rs", "folded_base", "outside", 1, "another track's file, not changed by T6"),
    ("owner_origin.rs", "owner_numbers", "outside", 1, "another track's file, not changed by T6"),
    ("owner_origin.rs", "read_token_in", "outside", 1, "another track's file, not changed by T6"),
    ("probation.rs", "ack_notices_in", "conservative", 2, "an unreadable source → nothing pruned (every key kept)"),
    ("probation.rs", "ack_notices_in", "not-a-read", 1, "GV2: a poisoned in-process mutex recovered"),
    ("probation.rs", "attribute_timeout", "not-a-read", 2, "fields of a row already read"),
    ("probation.rs", "closure_obligations", "reported", 2, "an unlistable root / unreadable row is reported by probation_pass in the same tick"),
    ("probation.rs", "closure_obligations", "display", 1, "the report's path"),
    ("probation.rs", "finalize_finding", "display", 2, "the finding's text: which copy it names, a short hash"),
    ("probation.rs", "hand_out_due_notices_in", "not-a-read", 1, "GV2: a poisoned in-process mutex recovered"),
    ("probation.rs", "load_acks_checked", "evidence", 1, "after a checked read: absent on positive evidence"),
    ("probation.rs", "load_findings_checked", "evidence", 1, "after a checked read: absent on positive evidence"),
    ("probation.rs", "load_hand_outs", "evidence", 1, "after a checked read: absent on positive evidence"),
    ("probation.rs", "load_notice_acks", "evidence", 1, "after a checked read: absent on positive evidence"),
    ("probation.rs", "m10_sweep_full", "display", 1, "a link's target in a finding"),
    ("probation.rs", "m10_sweep_full", "conservative", 3, "approved bytes unreadable → not recognised (the check runs) / no runner → nothing decided / no source → nothing put back"),
    ("probation.rs", "probation_pass", "display", 1, "the report's path"),
    // T7b (RULINGS HH5): `read_job_state_in` / `write_job_state` read through ONE three-valued reader
    // (`propagate::read_job_state_checked`) and have no default site any more. The old rows, quoted:
    // ("probation.rs", "read_job_state_in", "conservative", 4, "an unreadable job state = none: every
    // pass due (more work, never less); it holds no duty") and ("probation.rs", "write_job_state",
    // "conservative", 1, "as read_job_state_in").
    ("probation.rs", "recheck_one", "display", 1, "the fault point's path"),
    ("probation.rs", "save_findings", "conservative", 1, "unreadable acknowledgements → the view keeps the notice untold (told again, never lost)"),
    ("probation.rs", "start_time_pass", "not-a-read", 1, "a row read with three answers (None = absent on evidence)"),
    ("probation.rs", "start_time_pass", "display", 1, "a message part"),
    // T7b (RULINGS HH9) — the propagation's read defaults
    ("propagate.rs", "apply_general", "not-a-read", 1, "the scratch name's clock part (a name, not a read)"),
    ("propagate.rs", "committed_copy_slots_in", "conservative", 1, "HI2 / P6: a committed copy's metadata that cannot be parsed → the slot is kept (never removed on a doubt)"),
    ("propagate.rs", "committed_copy_in", "conservative", 2, "a slot whose metadata or bytes cannot be read is not R's committed copy → no bytes → the row 충돌 (nothing applied)"),
    ("propagate.rs", "conflict_summary_line_in", "display", 2, "HH7: the CEO's one summary line — display only; status and the gate read with three answers"),
    ("propagate.rs", "finish_as", "display", 1, "R1: the live states shown with a 충돌 (an unreadable member leaves the list empty; IK3 / IG5 pass the record in — the second site, a re-read of it, is gone; IQ1 renamed `finish` → `finish_as`)"),
    ("propagate.rs", "pack_now", "conservative", 1, "IV1 (`Stamp::pack_now`): an unreadable generation is None — a strict `holds` never calls a None unchanged (nothing written); IQ1's `Observed` is replaced by the stamp"),
    ("propagate.rs", "publish", "not-a-read", 1, "IV1: the stamp's own answer — every reason other than a changed record is 'states changed' (nothing written either way)"),
    ("propagate.rs", "pack_instance_pass_used", "conservative", 2, "interpretation 31 / P5: an unreadable identity / generation skips its watch for this tick (never \"changed\"; IZ3 renamed `pack_instance_pass_in` → `pack_instance_pass_used`)"),
    ("propagate.rs", "read_pack_instance", "not-a-read", 1, "the id line without its newline"),
    ("propagate.rs", "run_checks", "conservative", 2, "IG4: the reference recorded exactly as T6's origin apply records it — one that cannot run is 0, and T6's attribution never blames a timeout on a 0"),
    ("propagate.rs", "sweep_own_scratch", "conservative", 1, "nothing to sweep when the folder cannot be listed — retried at the next tick"),
    ("reapply_claim.rs", "claim_in", "not-a-read", 1, "no claim yet → the first"),
    ("reapply_claim.rs", "numbers", "not-a-read", 1, "a name that is not <k>.json is not a claim"),
    ("reapply_claim.rs", "read_claim_in", "conservative", 2, "an unreadable claim is held (claim_in)"),
    ("rehearsal.rs", "copy_dir", "outside", 1, "another track's file (T5b), not changed by T6"),
    ("rehearsal.rs", "first_line", "outside", 2, "another track's file (T5b), not changed by T6"),
    ("rehearsal.rs", "json_parse", "outside", 1, "another track's file (T5b), not changed by T6"),
    ("rehearsal.rs", "kind_by_type", "outside", 1, "another track's file (T5b), not changed by T6"),
    ("rehearsal.rs", "py_compile", "outside", 1, "another track's file (T5b), not changed by T6"),
    ("rehearsal.rs", "run_kind", "outside", 1, "another track's file (T5b), not changed by T6"),
    ("rehearsal.rs", "scratch_base", "outside", 2, "another track's file (T5b), not changed by T6"),
    ("rehearsal.rs", "sh_syntax", "outside", 1, "another track's file (T5b), not changed by T6"),
    ("rehearsal.rs", "tail", "outside", 1, "another track's file (T5b), not changed by T6"),
    ("replay.rs", "all", "outside", 2, "another track's file (T5b), not changed by T6"),
    ("replay.rs", "base_env", "outside", 1, "another track's file (T5b), not changed by T6"),
    ("replay.rs", "bundled_interpreter", "outside", 1, "another track's file (T5b), not changed by T6"),
    ("replay.rs", "deny_payload", "outside", 1, "another track's file (T5b), not changed by T6"),
    ("replay.rs", "marked_processes_by", "outside", 3, "another track's file (T5b), not changed by T6"),
    ("replay.rs", "printed_shape", "outside", 1, "another track's file (T5b), not changed by T6"),
    ("replay.rs", "pump_out", "outside", 1, "another track's file (T5b), not changed by T6"),
    ("replay.rs", "pump_stdin", "outside", 1, "another track's file (T5b), not changed by T6"),
    ("replay.rs", "random_hex", "outside", 1, "another track's file (T5b), not changed by T6"),
    ("replay.rs", "real_settings_targets", "outside", 3, "another track's file (T5b), not changed by T6"),
    ("replay.rs", "remove_tree", "outside", 1, "another track's file (T5b), not changed by T6"),
    ("replay.rs", "run_child_with", "outside", 1, "another track's file (T5b), not changed by T6"),
    ("replay.rs", "settings_sections", "outside", 4, "another track's file (T5b), not changed by T6"),
    ("replay.rs", "stamp_from", "outside", 1, "another track's file (T5b), not changed by T6"),
    ("replay.rs", "stat", "outside", 4, "another track's file (T5b), not changed by T6"),
    ("replay.rs", "system_path_has_no_cys", "outside", 1, "another track's file (T5b), not changed by T6"),
    ("replay.rs", "visit_map", "outside", 2, "another track's file (T5b), not changed by T6"),
    ("screen.rs", "build_variants", "outside", 2, "another track's file (T5b), not changed by T6"),
    ("screen.rs", "cache_get", "outside", 1, "another track's file (T5b), not changed by T6"),
    ("screen.rs", "cache_put", "outside", 1, "another track's file (T5b), not changed by T6"),
    ("screen.rs", "gate_entry_as_released", "outside", 2, "another track's file (T5b), not changed by T6"),
    ("screen.rs", "gate_registered", "outside", 2, "another track's file (T5b), not changed by T6"),
    ("screen.rs", "holds_token", "outside", 1, "another track's file (T5b), not changed by T6"),
    ("screen.rs", "hook_runs_gate", "outside", 1, "another track's file (T5b), not changed by T6"),
    ("screen.rs", "i3_template", "outside", 10, "another track's file (T5b), not changed by T6"),
    ("screen.rs", "interp_key", "outside", 1, "another track's file (T5b), not changed by T6"),
    ("screen.rs", "make_nopy_usr_bin", "outside", 1, "another track's file (T5b), not changed by T6"),
    ("screen.rs", "other_product_hooks", "outside", 1, "another track's file (T5b), not changed by T6"),
    ("screen.rs", "probe_input", "outside", 1, "another track's file (T5b), not changed by T6"),
    ("screen.rs", "production", "outside", 2, "another track's file (T5b), not changed by T6"),
    ("screen.rs", "protection_files_changed", "outside", 2, "another track's file (T5b), not changed by T6"),
    ("screen.rs", "run", "outside", 1, "another track's file (T5b), not changed by T6"),
    ("screen.rs", "run_all", "outside", 5, "another track's file (T5b), not changed by T6"),
    ("screen.rs", "run_backstop_probe", "outside", 2, "another track's file (T5b), not changed by T6"),
    ("screen.rs", "run_hook_probe_inner", "outside", 3, "another track's file (T5b), not changed by T6"),
    ("screen.rs", "run_i3", "outside", 1, "another track's file (T5b), not changed by T6"),
    ("screen.rs", "run_pool", "outside", 2, "another track's file (T5b), not changed by T6"),
    ("screen.rs", "screen_at4", "outside", 3, "another track's file (T5b), not changed by T6"),
    ("screen.rs", "screen_backstop", "outside", 1, "another track's file (T5b), not changed by T6"),
    ("screen.rs", "screen_build_id", "outside", 1, "another track's file (T5b), not changed by T6"),
    ("screen.rs", "screen_device", "outside", 6, "another track's file (T5b), not changed by T6"),
    ("screen.rs", "screen_i3", "outside", 4, "another track's file (T5b), not changed by T6"),
    ("screen.rs", "screen_with", "outside", 1, "another track's file (T5b), not changed by T6"),
    ("screen.rs", "settings_doc", "outside", 1, "another track's file (T5b), not changed by T6"),
    ("screen.rs", "table_basenames", "outside", 2, "another track's file (T5b), not changed by T6"),
    ("screen.rs", "without_gate", "outside", 2, "another track's file (T5b), not changed by T6"),
    ("tombstones.rs", "lane_mutex", "outside", 1, "another track's file, not changed by T6"),
    ("tombstones.rs", "load_in", "outside", 1, "another track's file, not changed by T6"),
    ("tombstones.rs", "read_file", "outside", 1, "another track's file, not changed by T6"),
    ("tombstones.rs", "record_in", "outside", 2, "another track's file, not changed by T6"),
    ("tombstones.rs", "remove_file_rel_in", "outside", 1, "another track's file, not changed by T6"),
    ("tombstones.rs", "remove_for_repair_in", "outside", 1, "another track's file, not changed by T6"),
    ("tombstones.rs", "remove_hooks_for_repair_in", "outside", 1, "another track's file, not changed by T6"),
    ("types.rs", "lane_pack_roots", "outside", 2, "another track's file, not changed by T6"),
    ("types.rs", "link_count", "outside", 2, "another track's file, not changed by T6"),
    ("types.rs", "link_target_refusal", "outside", 2, "another track's file, not changed by T6"),
    ("types.rs", "observe", "conservative", 1, "T6b IW1 / JH1: the length of a file found over the limit while it is read cannot be re-read → the bytes already counted (more than the limit); the observation is over the limit either way"),
    ("types.rs", "repair_number", "outside", 1, "another track's file, not changed by T6"),
    ("vault.rs", "basename", "not-a-read", 1, "a path's last part"),
    ("vault.rs", "build_slot", "not-a-read", 1, "the clock"),
    ("vault.rs", "build_slot", "evidence", 2, "GK1: the generation read before and after the copy — unreadable → none recorded (Unknown)"),
    ("vault.rs", "deposit_of", "reported", 2, "unreadable metadata → Err(Superseded)"),
    ("vault.rs", "deposit_of", "conservative", 1, "every deposit is verified against its recorded hash"),
    ("vault.rs", "finish_seal_with", "conservative", 1, "a failed probe of the record → the no-replace publication fails loudly"),
    ("vault.rs", "name", "not-a-read", 1, "a slot's folder name"),
    ("vault.rs", "read_sealed_meta", "reported", 2, "unreadable → Err(Superseded): never blessed"),
    ("vault.rs", "seal_with", "conservative", 1, "a failed probe → the no-replace placement fails loudly"),
    ("vault.rs", "sweep_stale_temps", "conservative", 5, "an entry whose age cannot be read is not removed; a removal counted only when done"),
    ("vault.rs", "temp_owner_pid", "not-a-read", 1, "a pid in a temporary name"),
    ("vault.rs", "temp_sibling", "not-a-read", 2, "parts of a temporary name"),
    ("wal.rs", "list_checked_in", "not-a-read", 1, "an order of floats"),
    ("wal.rs", "list_in", "conservative", 1, "its callers take a missing record as no agreement / no view rebuild; decisions use list_checked_in"),
    ("wal.rs", "read_in", "conservative", 1, "a record without its schema is refused"),
];

// ───────────────────────────── RULINGS FI2 — an applying intent is a claim ─────────────────────────────

/// FI2 (= FG1): an approved apply of this lane still `applying` (interrupted, its recovery not done)
/// that names the file is a CLAIM — the resolution answers UNKNOWN, never NO APPROVAL; an intent that
/// cannot be read makes the ledger unreadable (EN4). Without FI2 the resolution read only rows: the
/// half-applied repair had no row yet and the answer was NO APPROVAL (the vendor original went back).
#[test]
fn an_applying_intent_naming_the_file_is_a_claim() {
    let r = rig("fi2claim");
    let id = rid("R-31");
    r.put("hooks/i.sh", b"X\n", 0o755);
    let rec = record(&r, &id, Classification::BootCritical, Vec::new(), RepairState::AwaitingApproval);
    ledger::write_record_in(&r.root, &rec).unwrap();
    let intent = ApplyIntent {
        schema: 1,
        repair: id.clone(),
        lane_key: r.lane.clone(),
        op_tag: "t".into(),
        state: "applying".into(),
        targets: vec![IntentTarget { rel: "hooks/i.sh".into(), pre: "absent".into(), incoming: "absent".into() }],
        note: String::new(),
        pack_generation: None,
        approval_ref: Some("rqr-1".into()),
        created_at: 1.0,
        updated_at: 1.0,
    };
    write_intent(&r.root, &intent).unwrap();
    assert!(matches!(resolve_approved_copy_in(&r.root, &r.pack, &r.lane, "hooks/i.sh"), ApprovedCopy::Unknown(_)));
    assert!(matches!(resolve_approved_copy_in(&r.root, &r.pack, &r.lane, "hooks/other.sh"), ApprovedCopy::NoApproval), "only the named file");
    std::fs::write(intent_path_in(&r.root, &id, &r.lane), b"{ broken").unwrap();
    assert!(matches!(resolve_approved_copy_in(&r.root, &r.pack, &r.lane, "hooks/other.sh"), ApprovedCopy::LedgerUnreadable { .. }));
    assert_eq!(unreadable_intents_in(&r.root, &r.lane).len(), 1);
}

// ───────────────────────────── RULINGS GK1 — the door takes the generation from the slot ─────────────────────────────

/// GK1 (a) (codex r5 B1): an update completing BETWEEN the pre-image's copy and the second reading
/// of the generation leaves the slot without a generation (Unknown) — never stamped with the newer
/// generation while holding the older bytes. Without GK1 the generation was read once, after the
/// copy: the slot said G2 with G1 bytes.
#[test]
fn an_update_between_the_copy_and_the_metadata_leaves_the_generation_unknown() {
    let r = rig("gk1copy");
    let id = rid("R-71");
    r.put("bin/u.py", b"print('g1')\n", 0o644);
    let pack = r.pack.clone();
    ledger::BETWEEN_PREIMAGE_COPY_AND_GENERATION.with(|h| {
        *h.borrow_mut() = Some(Box::new(move || std::fs::write(pack.join(".pack-version"), "0.15.1\n").unwrap()));
    });
    let pre = ledger::copy_preimage_in(&r.root, &id, &r.lane, "bin/u.py", &r.pack.join("bin/u.py"), 1.0);
    ledger::BETWEEN_PREIMAGE_COPY_AND_GENERATION.with(|h| *h.borrow_mut() = None);
    let PreImage::Slot { slot_dir, .. } = pre.unwrap() else { panic!("slot") };
    assert_eq!(vault::slot_generation(&slot_dir).unwrap(), None, "an unstable reading records no generation");
    // a stable reading records it
    let id2 = rid("R-72");
    let pre = ledger::copy_preimage_in(&r.root, &id2, &r.lane, "bin/u.py", &r.pack.join("bin/u.py"), 2.0).unwrap();
    let PreImage::Slot { slot_dir, .. } = pre else { panic!("slot") };
    assert_eq!(vault::slot_generation(&slot_dir).unwrap().unwrap().version, "0.15.1");
}

/// GK1 (a) / GM1: a `#!` pre-image WITHOUT the owner's execute bit gets no generation (its mode may
/// not be its generation's final one — the non-transactional install sets the bit after
/// `.pack-version`); with the bit, the generation.
#[cfg(unix)]
#[test]
fn a_shebang_pre_image_without_its_execute_bit_has_no_generation() {
    let r = rig("gm1");
    r.put("hooks/s.sh", b"#!/bin/sh\necho s\n", 0o644);
    let PreImage::Slot { slot_dir, .. } = ledger::copy_preimage_in(&r.root, &rid("R-73"), &r.lane, "hooks/s.sh", &r.pack.join("hooks/s.sh"), 1.0).unwrap() else { panic!("slot") };
    assert_eq!(vault::slot_generation(&slot_dir).unwrap(), None, "0644 → UNKNOWN");
    r.put("hooks/s.sh", b"#!/bin/sh\necho s\n", 0o755);
    let PreImage::Slot { slot_dir, .. } = ledger::copy_preimage_in(&r.root, &rid("R-74"), &r.lane, "hooks/s.sh", &r.pack.join("hooks/s.sh"), 2.0).unwrap() else { panic!("slot") };
    assert_eq!(vault::slot_generation(&slot_dir).unwrap().unwrap().version, "0.15.0", "0755 → the generation");
}

/// GK1 (b) (codex r5 B1): a PENDING revert (its intent `reverting`, bound to the generation now)
/// whose pre-image slot records no generation — or another one — puts nothing back: the put-back
/// takes the binding from the slot itself, the intent's generation never stands in. Without GK1 the
/// recovery bound the put-back to the intent's generation and restored the bytes.
#[cfg(target_os = "macos")]
#[test]
fn a_pending_revert_never_puts_back_a_slot_without_its_generation_or_with_another() {
    for (tag, slot_gen) in [("none", None), ("other", Some(serde_json::json!({"version": "0.14.0", "pro_revision": null})))] {
        let r = rig(&format!("gk1pending{tag}"));
        let id = rid("R-75");
        let rec = committed_general(&r, &id, "hooks/v.sh", Some(b"PRE\n"), Some(b"POST\n"));
        let PreImage::Slot { slot_dir, .. } = &rec.files[0].pre else { panic!("slot") };
        let p = slot_dir.join("meta").join("pre.json");
        let mut v: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        match &slot_gen {
            None => {
                v.as_object_mut().unwrap().remove("pack_generation");
            }
            Some(g) => v["pack_generation"] = g.clone(),
        }
        std::fs::write(&p, serde_json::to_vec(&v).unwrap()).unwrap();
        let applied = r.live("hooks/v.sh").canon();
        let pre_state = rec.files[0].pre.state().canon();
        let ri = RevertIntent {
            schema: 1,
            repair: id.clone(),
            lane_key: r.lane.clone(),
            op_tag: "t".into(),
            state: "reverting".into(),
            targets: vec![RevertTarget { rel: "hooks/v.sh".into(), applied, pre: pre_state }],
            has_hooks: false,
            pack_generation: Bound::now(&r.pack).known(),
            by: "사람".into(),
            created_at: 1.0,
            updated_at: 1.0,
        };
        write_revert_intent(&r.root, &ri).unwrap();
        let env = r.env(&NoFaults, ApplyForm::Exchange);
        let _ = recover_revert_intents_in(&env, &no_hooks, &no_repair_lock);
        assert_eq!(std::fs::read(r.pack.join("hooks/v.sh")).unwrap(), b"POST\n", "{tag}: nothing put back");
    }
}

// ───────────────────────────── RULINGS GK2 — only what the engine placed is ever replaced ─────────────────────────────

/// GK2 (codex r5 B2) on an ordinary file: another writer writes the file right AFTER the automatic
/// return's exchange (before its own re-read) — the other writer's bytes STAY live, nothing is
/// exchanged back, the return closes 충돌 (its report is owed — FG3) and the vault keeps the
/// repaired copy. Without GK2 the engine took "what is live now" as the expected state and
/// exchanged the old bytes back over the other writer's.
#[cfg(target_os = "macos")]
#[test]
fn another_writers_bytes_after_our_exchange_stay_live() {
    struct WriterAfterExchange;
    impl Faults for WriterAfterExchange {
        fn at(&self, p: Point, live: &Path, _: Option<&Slot>) -> bool {
            if p == Point::AfterSeal {
                std::fs::write(live, b"OTHER WRITER\n").unwrap();
            }
            false
        }
    }
    let r = rig("gk2file");
    let id = rid("R-81");
    committed_general(&r, &id, "hooks/o.sh", Some(b"PRE\n"), Some(b"POST\n"));
    let env = r.env(&WriterAfterExchange, ApplyForm::Exchange);
    let rep = revert_in(&env, &id, RevertCaller::DaemonAuto, &no_hooks, &no_repair_lock).unwrap();
    assert!(matches!(rep.outcome, ApplyOutcome::Declined(_)), "{rep:?}");
    assert_eq!(std::fs::read(r.pack.join("hooks/o.sh")).unwrap(), b"OTHER WRITER\n", "the other writer's bytes stay");
    let closed = ledger::read_lane_in(&r.root, &id, &r.lane).unwrap().unwrap().probation.unwrap().closed.unwrap();
    assert!(closed.starts_with("충돌") && closure_owes_intake(&closed), "closed as conflict, its report owed: {closed}");
    assert!(r.vault_bytes().contains(&b"POST\n".to_vec()), "the vault keeps the repaired copy");
}

// ───────────────────────────── RULINGS GV1 — another writer's bytes are a conflict on every return path ─────────────────────────────

/// GV1 (codex r6 M1): a user-requested revert whose file another writer changed — found by the
/// pre-check, or written between the record and the door — closes as CONFLICT: the bytes stay, the
/// row is 충돌 with a closure whose report is owed, the record stays in force (not "superseded").
/// Without GV1 the pre-check closed it 대체됨 (record Superseded, no report) and the door's refusal
/// was counted "nothing changed".
#[cfg(target_os = "macos")]
#[test]
fn a_revert_meeting_another_writers_bytes_closes_as_conflict() {
    struct WriteAtRecord;
    impl Faults for WriteAtRecord {
        fn at(&self, p: Point, live: &Path, _: Option<&Slot>) -> bool {
            if p == Point::AfterRecord {
                std::fs::write(live, b"FOREIGN AT THE DOOR\n").unwrap();
            }
            false
        }
    }
    for (tag, at_door) in [("precheck", false), ("door", true)] {
        let r = rig(&format!("gv1{tag}"));
        let id = rid("R-91");
        committed_general(&r, &id, "hooks/g.sh", Some(b"PRE\n"), Some(b"POST\n"));
        if !at_door {
            std::fs::write(r.pack.join("hooks/g.sh"), b"FOREIGN\n").unwrap();
        }
        let faults = WriteAtRecord;
        let env = if at_door { r.env(&faults, ApplyForm::Exchange) } else { r.env(&NoFaults, ApplyForm::Exchange) };
        let rep = revert_in(&env, &id, RevertCaller::Human, &no_hooks, &no_repair_lock).unwrap();
        assert!(matches!(rep.outcome, ApplyOutcome::Declined(_)), "{tag}: {rep:?}");
        let want: &[u8] = if at_door { b"FOREIGN AT THE DOOR\n" } else { b"FOREIGN\n" };
        assert_eq!(std::fs::read(r.pack.join("hooks/g.sh")).unwrap(), want, "{tag}: the bytes stay");
        let row = ledger::read_lane_in(&r.root, &id, &r.lane).unwrap().unwrap();
        assert_eq!(row.state, LaneState::Conflict, "{tag}");
        let closed = row.probation.unwrap().closed.unwrap();
        assert_eq!(closure_report_class(&closed), Some("conflict"), "{tag}: its report is owed: {closed}");
        assert_eq!(ledger::read_record_in(&r.root, &id).unwrap().unwrap().state, RepairState::Probation, "{tag}: still in force");
    }
}

/// GV1: a PENDING revert that resumes after another writer changed the file (neither the recorded
/// applied state nor the pre-image) closes as conflict with its report owed. Without GV1 it was
/// counted "nothing changed" and closed 대체됨.
#[cfg(target_os = "macos")]
#[test]
fn a_pending_revert_meeting_another_writers_bytes_closes_as_conflict() {
    let r = rig("gv1pending");
    let id = rid("R-92");
    let rec = committed_general(&r, &id, "hooks/q.sh", Some(b"PRE\n"), Some(b"POST\n"));
    let ri = RevertIntent {
        schema: 1,
        repair: id.clone(),
        lane_key: r.lane.clone(),
        op_tag: "t".into(),
        state: "reverting".into(),
        targets: vec![RevertTarget { rel: "hooks/q.sh".into(), applied: r.live("hooks/q.sh").canon(), pre: rec.files[0].pre.state().canon() }],
        has_hooks: false,
        pack_generation: Bound::now(&r.pack).known(),
        by: BY_DAEMON_AUTO.into(),
        created_at: 1.0,
        updated_at: 1.0,
    };
    write_revert_intent(&r.root, &ri).unwrap();
    std::fs::write(r.pack.join("hooks/q.sh"), b"FOREIGN\n").unwrap();
    let env = r.env(&NoFaults, ApplyForm::Exchange);
    let _ = recover_revert_intents_in(&env, &no_hooks, &no_repair_lock);
    assert_eq!(std::fs::read(r.pack.join("hooks/q.sh")).unwrap(), b"FOREIGN\n", "the bytes stay");
    let closed = ledger::read_lane_in(&r.root, &id, &r.lane).unwrap().unwrap().probation.unwrap().closed.unwrap();
    assert_eq!(closure_report_class(&closed), Some("conflict"), "its report is owed: {closed}");
}

// ───────────────────────────── RULINGS EJ4 — one ownership per repair ─────────────────────────────

/// EJ4 through the real entry points: a revert (the owner) paused in its hook phase holds the
/// repair's guard — the job's recovery of interrupted reverts, run meanwhile, leaves the repair alone;
/// when the owner has finished, the recovery reads the intent AFTER taking the guard and finds it
/// finished (nothing re-done from a stale listing).
#[cfg(target_os = "macos")]
#[test]
fn recovery_leaves_a_repair_alone_while_its_owner_runs() {
    let r = rig("ej4owner");
    let id = rid("R-52");
    let mut rec = committed_general(&r, &id, "hooks/o.sh", Some(b"PRE\n"), Some(b"POST\n"));
    rec.files.push(FileChange {
        rel: "settings:Stop:o.sh".into(),
        kind: ChangeKind::HookUnregister { event: "Stop".into(), script: "o.sh".into() },
        pre: PreImage::Absent,
        post: TargetState::Absent,
        diff: None,
    });
    ledger::write_record_in(&r.root, &rec).unwrap();
    let (in_tx, in_rx) = std::sync::mpsc::channel::<()>();
    let (go_tx, go_rx) = std::sync::mpsc::channel::<()>();
    let rr = &r;
    let idr = &id;
    std::thread::scope(|sc| {
        let owner = sc.spawn(move || {
            let hooks = move |_: &RepairId, _: &LaneKey, _: HookStep| -> Result<ReconcileReport, RepairError> {
                in_tx.send(()).unwrap();
                go_rx.recv().unwrap();
                Ok(ReconcileReport { hooks: Vec::new() })
            };
            let env = OpEnv { root: &rr.root, pack: &rr.pack, lane: &rr.lane, now: 1_800_000_000.0, form: ApplyForm::Exchange, faults: &NoFaults, own_check: &|_, _| None };
            revert_in(&env, idr, RevertCaller::Human, &hooks, &no_repair_lock).map(|x| x.outcome)
        });
        in_rx.recv().unwrap();
        // the owner is inside its hook phase: the revert intent says "reverting", the file restored
        assert_eq!(read_revert_intent_in(&r.root, &id, &r.lane).unwrap().state, "reverting");
        let calm = r.env(&NoFaults, ApplyForm::Exchange);
        let called = Cell::new(0);
        let other_hooks = |_: &RepairId, _: &LaneKey, _: HookStep| -> Result<ReconcileReport, RepairError> {
            called.set(called.get() + 1);
            Ok(ReconcileReport { hooks: Vec::new() })
        };
        assert!(recover_revert_intents_in(&calm, &other_hooks, &no_repair_lock).is_empty(), "left alone while owned");
        assert_eq!(called.get(), 0);
        assert!(revert_in(&calm, &id, RevertCaller::Human, &other_hooks, &no_repair_lock).is_err(), "a second revert is busy, never waits");
        go_tx.send(()).unwrap();
        assert_eq!(owner.join().unwrap().unwrap(), ApplyOutcome::Applied);
        // the owner finished: the recovery reads the intent after taking the guard — nothing to do
        assert!(recover_revert_intents_in(&calm, &other_hooks, &no_repair_lock).is_empty());
        assert_eq!(called.get(), 0);
    });
}

/// EJ5 (codex r3 I8): forward completion REBUILDS the repair's lane row — an earlier incomplete apply
/// left a 충돌 row (old applied subset, old approval reference); the fresh apply crashes after its
/// record commit; the recovery replaces this repair's row fields from the record and the intent.
/// Without EJ5 the existing row was skipped and kept its old subset and reference.
#[cfg(target_os = "macos")]
#[test]
fn forward_completion_rebuilds_an_existing_row() {
    let r = rig("ej5row");
    let id = rid("R-53");
    let files = two_target_candidate(&r, &id);
    // the leftover of an earlier, incomplete apply
    ledger::write_lane_in(&r.root, &LaneRecord {
        schema: 1,
        repair: id.clone(),
        lane: r.lane.clone(),
        state: LaneState::Conflict,
        probation: Some(Probation { pack_generation: "0.15.0".into(), pro_revision: None, applied_at: 1.0, expected: vec![("hooks/a1.sh".into(), "absent".into())], closed: None, check_timings: Vec::new(), check_timeouts: Vec::new() }),
        merge: None,
        merge_history: Vec::new(),
        approval_ref: Some("rqr-old".into()),
        conflict_live: vec![("hooks/a1.sh".into(), "absent".into())],
        applied: vec![("hooks/a1.sh".into(), "absent".into())],
        applied_history: Vec::new(),
        merge_base: Vec::new(),
        merge_lock: None,
        merge_attempt: 0,
        reason: String::new(),
        updated_at: 1.0,
    }).unwrap();
    let stop = StopOn { rel: "hooks/a1.sh", point: Point::AfterRecordCommit, fired: Cell::new(false) };
    assert!(apply_candidate_in(&r.env(&stop, ApplyForm::Exchange), &id, &files, &|_, _, _| Ok(()), &pass_check, &pass_ref, &no_repair_lock).is_err());
    let calm = r.env(&NoFaults, ApplyForm::Exchange);
    recover_apply_intents_in(&calm, &no_repair_lock);
    let row = ledger::read_lane_in(&r.root, &id, &r.lane).unwrap().unwrap();
    assert_eq!(row.state, LaneState::Verified);
    assert_eq!(row.applied.len(), 2, "both targets: {:?}", row.applied);
    assert_eq!(row.approval_ref.as_deref(), Some("rqr-1"), "the fresh approval's reference");
    assert!(row.conflict_live.is_empty());
    assert!(row.probation.as_ref().unwrap().closed.is_none());
}

/// EJ6 (codex r3 I9): the interrupted revert's recovery sentence follows the measured outcome — a
/// generation change before the recovery → "대체됨 — 아무것도 더 되돌리지 않았습니다", never "마저 했습니다".
#[cfg(target_os = "macos")]
#[test]
fn the_revert_recovery_sentence_follows_the_outcome() {
    let r = rig("ej6line");
    let id = rid("R-54");
    two_file_general(&r, &id);
    let stop = StopOn { rel: "hooks/v2.sh", point: Point::AfterRecord, fired: Cell::new(false) };
    assert!(revert_in(&r.env(&stop, ApplyForm::Exchange), &id, RevertCaller::Human, &no_hooks, &no_repair_lock).is_err());
    std::fs::write(r.pack.join(".pack-version"), "0.15.1\n").unwrap();
    let calm = r.env(&NoFaults, ApplyForm::Exchange);
    resume_interrupted_in(&calm).unwrap();
    let lines = recover_revert_intents_in(&calm, &no_hooks, &no_repair_lock);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains("대체됨") && !lines[0].contains("마저 했습니다"), "{lines:?}");
}

// ───────────────────────────── RULINGS FG1 — no read collapses "could not read" into a value ─────────────────────────────

/// FG1 (codex r4 B1): an apply bound to the free generation V is interrupted; the pack's state file
/// then becomes CORRUPT (a pro pack of the same base, unreadable) — the generation "now" is Unknown,
/// so the recovery puts NOTHING back. Without FG1 a corrupt state read as "free pack" and the free
/// generation's pre-image went into the pro pack.
#[cfg(target_os = "macos")]
#[test]
fn a_corrupt_pack_state_is_an_unknown_generation_for_recovery() {
    let r = rig("fg1state");
    let id = rid("R-55");
    let files = two_target_candidate(&r, &id);
    let stop = StopOn { rel: "hooks/a2.sh", point: Point::AfterRecord, fired: Cell::new(false) };
    assert!(apply_candidate_in(&r.env(&stop, ApplyForm::Exchange), &id, &files, &|_, _, _| Ok(()), &pass_check, &pass_ref, &no_repair_lock).is_err());
    std::fs::write(r.pack.join(crate::pack::PACK_STATE_FILE), b"{ corrupt").unwrap();
    assert_eq!(Bound::now(&r.pack), Bound::Unknown);
    let calm = r.env(&NoFaults, ApplyForm::Exchange);
    resume_interrupted_in(&calm).unwrap();
    recover_apply_intents_in(&calm, &no_repair_lock);
    assert_eq!(std::fs::read(r.pack.join("hooks/a1.sh")).unwrap(), b"NEW-1\n", "nothing put back into an unknown generation");
    assert_eq!(read_intent_in(&r.root, &id, &r.lane).unwrap().state, "superseded");
}

/// FG1 (codex r4 B2): the repairs root cannot be listed (injected) while known paths stay readable —
/// the ledger is NOT complete: the resolution answers LedgerUnreadable (never NO APPROVAL) and the
/// seam answers Unknown (never NotConsumed). A row that claims the file without its probation is an
/// inconsistent member → Unknown.
#[test]
fn an_unlistable_root_and_a_row_without_probation_are_never_no_approval() {
    let r = rig("fg1root");
    let id = rid("R-56");
    approved_and_applied(&r, &id, "hooks/u.sh", b"VENDOR\n", b"APPROVED\n");
    crate::repair::ledger::FAIL_LIST_IDS.with(|c| c.set(true));
    let answer = resolve_approved_copy_in(&r.root, &r.pack, &r.lane, "hooks/other.sh");
    let consumed = crate::repair::consumed::approval_consumed_in(&r.root, &r.lane, "rqr-none");
    crate::repair::ledger::FAIL_LIST_IDS.with(|c| c.set(false));
    assert!(matches!(answer, ApprovedCopy::LedgerUnreadable { .. }), "{answer:?}");
    assert!(matches!(consumed, crate::repair::consumed::ApprovalUse::Unknown(_)), "{consumed:?}");
    let mut row = ledger::read_lane_in(&r.root, &id, &r.lane).unwrap().unwrap();
    row.probation = None;
    ledger::write_lane_in(&r.root, &row).unwrap();
    assert!(matches!(resolve_approved_copy_in(&r.root, &r.pack, &r.lane, "hooks/u.sh"), ApprovedCopy::Unknown(_)));
}

// ───────────────────────────── RULINGS IE4 / IG1 — the engine's one bound on what it reads ─────────────────────────────

fn sparse_file(p: &Path, len: u64) {
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).unwrap();
    }
    std::fs::File::create(p).unwrap().set_len(len).unwrap();
}

/// IG1 / IW1 / JH1: the measurement takes at most RECORD_READ_CAP + 1 bytes from ONE handle — a file
/// of exactly the limit is read; a file one byte over the limit has NO state: `TargetState::observe`
/// measures it as `Observed::OverLimit` (its length) with no byte of it read, and `TargetState::read`
/// answers an error naming the limit; a file that grows past the limit while it is read is the same
/// observation after at most the limit + 1 bytes; a size that changes within the limit while it is read
/// gives no answer at all (an error). Before JH1 (old expectation, round 3): `TargetState::read(&over)`
/// gave a STATE (`st.is_over_limit()`, its canon naming the limit) — a value a hash field could hold.
#[test]
fn a_target_is_read_within_the_one_limit() {
    use crate::repair::durable::{read_seam, RECORD_READ_CAP as CAP};
    use crate::repair::types::Observed;
    let r = rig("ig1read");
    let at = r.pack.join("hooks/limit.sh");
    sparse_file(&at, CAP);
    assert!(matches!(TargetState::read(&at).unwrap(), TargetState::File { .. }), "exactly the limit is read");
    assert_eq!(read_seam::bytes_read_from(&at), CAP);
    let over = r.pack.join("hooks/over.sh");
    sparse_file(&over, CAP + 1);
    assert_eq!(TargetState::observe(&over).unwrap(), Observed::OverLimit { len: CAP + 1 }, "measured, no state");
    let e = format!("{:?}", TargetState::read(&over).expect_err("no state to read"));
    assert!(e.contains(&CAP.to_string()), "the limit is named: {e}");
    assert_eq!(read_seam::bytes_read_from(&over), 0, "no byte of it is read");
    for (tag, grow_to) in [("within", 100u64), ("past", CAP * 4)] {
        let g = r.pack.join(format!("hooks/grow-{tag}.sh"));
        sparse_file(&g, 10);
        let target = g.clone();
        read_seam::set_after_open(Some(Box::new(move |p: &Path| {
            if p == target {
                std::fs::OpenOptions::new().write(true).open(p).unwrap().set_len(grow_to).unwrap();
            }
        })));
        let got = TargetState::observe(&g);
        read_seam::set_after_open(None);
        match tag {
            "within" => assert!(got.is_err(), "{tag}: a size changed within the limit gives no answer: {got:?}"),
            _ => assert!(matches!(got, Ok(Observed::OverLimit { .. })), "{tag}: grown past the limit — measured over it: {got:?}"),
        }
        assert!(read_seam::bytes_read_from(&g) <= CAP + 1, "{tag}: at most the limit + 1 bytes");
    }
}

/// IE4: the vault's copy of a staged candidate is read through the same bound — one byte over the
/// limit is refused before any byte is read (the limit named); a source that grows past the limit while
/// it is copied is refused after at most the limit + 1 bytes; nothing is left as a slot. Without IE4 the
/// copy read the whole file into memory.
#[test]
fn the_vault_copy_of_a_candidate_is_read_within_the_one_limit() {
    use crate::repair::durable::{read_seam, RECORD_READ_CAP as CAP};
    let r = rig("ie4vault");
    let id = rid("R-17");
    let over = r.base.join("cand/over.sh");
    sparse_file(&over, CAP + 1);
    let staged = Staged::FileFrom { source: over.clone(), mode: 0o644 };
    let e = format!("{:?}", vault::build_slot(&r.root, &id, &r.lane, "hooks/over.sh", "t1", DepositKind::Displaced, &TargetState::Absent, &staged, 1.0, &r.pack).expect_err("over the limit"));
    assert!(e.contains(&CAP.to_string()), "the limit is named: {e}");
    assert_eq!(read_seam::bytes_read_from(&over), 0, "refused before any byte is read");
    let grow = r.base.join("cand/grow.sh");
    sparse_file(&grow, 10);
    let target = grow.clone();
    read_seam::set_after_open(Some(Box::new(move |p: &Path| {
        if p == target {
            std::fs::OpenOptions::new().write(true).open(p).unwrap().set_len(CAP * 4).unwrap();
        }
    })));
    let staged = Staged::FileFrom { source: grow.clone(), mode: 0o644 };
    let got = vault::build_slot(&r.root, &id, &r.lane, "hooks/grow.sh", "t2", DepositKind::Displaced, &TargetState::Absent, &staged, 1.0, &r.pack);
    read_seam::set_after_open(None);
    assert!(got.is_err(), "a source that grew past the limit is refused: {got:?}");
    assert!(read_seam::bytes_read_from(&grow) <= CAP + 1);
    assert!(ledger::list_slots_in(&r.root, &id).is_empty(), "no slot is left");
}

/// IE4 (codex T5b r6 M1) through the approved apply: the candidate was approved, THEN its working copy
/// was replaced by a file over the limit — the apply is REFUSED before any byte of it is read, nothing
/// is displaced, the pack-apply exclusion is free again, and neither the record nor its approval moved.
/// Without IE4 the file was hashed to its end and copied whole under the exclusion before the
/// fingerprint comparison declined it.
#[cfg(target_os = "macos")]
#[test]
fn an_approved_candidate_replaced_by_an_over_limit_file_is_refused_unread() {
    use crate::repair::durable::{read_seam, RECORD_READ_CAP as CAP};
    let r = rig("ie4cand");
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
    // after the approval the working copy is replaced by a file over the limit
    std::fs::remove_file(&staging).unwrap();
    sparse_file(&staging, CAP + 1);
    let before = read_seam::bytes_read_from(&staging);
    let lock = r.base.join(".pack-apply.lock");
    let env = r.env(&NoFaults, ApplyForm::Exchange);
    let res = crate::pack_apply_lock::with_apply_lock(&lock, || {
        apply_candidate_in(&env, &id, &[CandidateFile { rel: "hooks/g.sh".into(), staged: staging.clone(), mode: 0o644 }], &|_, _, _| Ok(()), &pass_check, &pass_ref, &no_repair_lock)
    })
    .unwrap();
    let e = format!("{:?}", res.expect_err("the apply is refused"));
    assert!(e.contains(&CAP.to_string()), "the limit is named: {e}");
    assert_eq!(read_seam::bytes_read_from(&staging) - before, 0, "the working copy is never read");
    assert_eq!(std::fs::read(r.pack.join("hooks/g.sh")).unwrap(), b"PRE\n", "nothing displaced");
    assert_eq!(crate::pack_apply_lock::try_with_apply_lock(&lock, || ()).unwrap(), Some(()), "the exclusion is free");
    let rec = ledger::read_record_in(&r.root, &id).unwrap().unwrap();
    assert_eq!(rec.state, RepairState::AwaitingApproval, "the record did not move");
    assert!(rec.approval.is_some(), "the approval did not move");
}

/// IG1 / IW1 through the approved apply: a LIVE target over the limit when the apply measures it (step
/// 0) is ANOTHER writer's bytes (a measured difference) — the apply declines without reading it, the
/// approval void (the accepted path for a live file that is not the pre-image), nothing displaced, the
/// exclusion free. Before IW1 (old expectation): `res.expect_err("the apply is refused")` naming the
/// limit, the record still `AwaitingApproval` with its approval.
#[cfg(target_os = "macos")]
#[test]
fn a_live_target_over_the_limit_refuses_the_apply_unread() {
    use crate::repair::durable::{read_seam, RECORD_READ_CAP as CAP};
    let r = rig("ig1live");
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
    let live = r.pack.join("hooks/g.sh");
    std::fs::OpenOptions::new().write(true).open(&live).unwrap().set_len(CAP + 1).unwrap();
    let before = read_seam::bytes_read_from(&live);
    let lock = r.base.join(".pack-apply.lock");
    let env = r.env(&NoFaults, ApplyForm::Exchange);
    let res = crate::pack_apply_lock::with_apply_lock(&lock, || {
        apply_candidate_in(&env, &id, &[CandidateFile { rel: "hooks/g.sh".into(), staged: staging.clone(), mode: 0o644 }], &|_, _, _| Ok(()), &pass_check, &pass_ref, &no_repair_lock)
    })
    .unwrap();
    let rep = res.expect("the accepted path: a decline");
    assert!(matches!(rep.outcome, ApplyOutcome::Declined(ref m) if m.contains("살아 있는 파일이 수리를 시작할 때와 다릅니다")), "{rep:?}");
    assert_eq!(read_seam::bytes_read_from(&live) - before, 0, "the live file is never read");
    assert_eq!(std::fs::metadata(&live).unwrap().len(), CAP + 1, "nothing displaced");
    assert_eq!(crate::pack_apply_lock::try_with_apply_lock(&lock, || ()).unwrap(), Some(()), "the exclusion is free");
    assert!(ledger::read_record_in(&r.root, &id).unwrap().unwrap().approval.is_none(), "the approval is void");
}

/// IV5 (reviewer1 on T7b, M4): the pack's generation marker is read inside the pack-apply exclusion,
/// so it is read through the engine's one bounded reader — a marker over the limit is "unknown" without
/// a byte of it read; a FIFO at its name (a writer offering a VALID marker to every reader) is "unknown"
/// at once, never waited for. Without IV5 the old read took the oversized marker whole as a version and
/// waited on the FIFO for its writer (here: got the writer's valid version).
#[cfg(unix)]
#[test]
fn the_generation_marker_is_read_within_the_one_limit_and_never_waited_for() {
    use crate::repair::durable::{read_seam, RECORD_READ_CAP as CAP};
    use std::io::Write;
    let r = rig("iv5");
    let marker = r.pack.join(".pack-version");
    assert!(Bound::now(&r.pack).known().is_some());
    std::fs::remove_file(&marker).unwrap();
    sparse_file(&marker, CAP + 1);
    let before = read_seam::bytes_read_from(&marker); // the first read above took the small marker
    assert_eq!(Bound::now(&r.pack), Bound::Unknown, "an oversized marker is unknown");
    assert_eq!(pack_generation(&r.pack).version, "unknown");
    assert_eq!(read_seam::bytes_read_from(&marker) - before, 0, "never read");
    std::fs::remove_file(&marker).unwrap();
    let c = std::ffi::CString::new(marker.to_str().unwrap()).unwrap();
    // SAFETY: a valid NUL-terminated path.
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o644) }, 0);
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (w_stop, w_at) = (std::sync::Arc::clone(&stop), marker.clone());
    let writer = std::thread::spawn(move || {
        // each open waits for a reader of the FIFO and serves it a valid marker; it ends when told to
        // or when the name no longer holds the FIFO
        use std::os::unix::fs::FileTypeExt;
        while !w_stop.load(std::sync::atomic::Ordering::SeqCst) && std::fs::symlink_metadata(&w_at).is_ok_and(|m| m.file_type().is_fifo()) {
            match std::fs::OpenOptions::new().write(true).open(&w_at) {
                Ok(mut f) => {
                    let _ = f.write_all(b"0.15.0\n");
                }
                Err(_) => break,
            }
        }
    });
    let checked = Bound::now(&r.pack);
    let plain = pack_generation(&r.pack).version;
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    let release = {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK).open(&marker).unwrap()
    };
    writer.join().unwrap();
    drop(release);
    assert_eq!(checked, Bound::Unknown, "a FIFO marker is unknown at once");
    assert_eq!(plain, "unknown");
}

/// IW1 (codex T6b r2 M2) — the reviewer's sequence to its END: an approved apply placed A and B and
/// stopped (a crash) before it finished; B then grows past the engine's read limit; the restart's
/// recovery puts A back (it still holds this attempt's own bytes), keeps B as another writer's bytes
/// (a measured difference — its length exceeds the limit), records the intent `incomplete`; a second
/// start does nothing more. Without IW1 the bounded read of B was an ERROR: the recovery stopped there,
/// never undid A, and repeated at every start.
#[cfg(target_os = "macos")]
#[test]
fn a_target_grown_past_the_limit_is_kept_as_a_conflict_and_the_rest_undone() {
    use crate::repair::durable::RECORD_READ_CAP as CAP;
    let r = rig("iw1grow");
    let id = rid("R-41");
    let files = two_target_candidate(&r, &id);
    let faults = StopOn { rel: "hooks/a2.sh", point: Point::BeforeDone, fired: Cell::new(false) };
    assert!(apply_candidate_in(&r.env(&faults, ApplyForm::Exchange), &id, &files, &|_, _, _| Ok(()), &pass_check, &pass_ref, &no_repair_lock).is_err());
    let calm = r.env(&NoFaults, ApplyForm::Exchange);
    resume_interrupted_in(&calm).unwrap();
    assert_eq!(std::fs::read(r.pack.join("hooks/a1.sh")).unwrap(), b"NEW-1\n");
    // B grows past the limit before the restart's recovery
    let b = r.pack.join("hooks/a2.sh");
    std::fs::OpenOptions::new().write(true).open(&b).unwrap().set_len(CAP + 1).unwrap();
    let lines = recover_apply_intents_in(&calm, &no_repair_lock);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains("충돌") && lines[0].contains(&CAP.to_string()), "the conflict names the limit: {lines:?}");
    assert_eq!(std::fs::read(r.pack.join("hooks/a1.sh")).unwrap(), b"PRE-1\n", "A is back");
    assert_eq!(std::fs::metadata(&b).unwrap().len(), CAP + 1, "B is kept");
    assert_eq!(read_intent_in(&r.root, &id, &r.lane).unwrap().state, "incomplete");
    // a second start does nothing more
    assert!(recover_apply_intents_in(&calm, &no_repair_lock).is_empty());
    assert_eq!(std::fs::metadata(&b).unwrap().len(), CAP + 1);
    assert_eq!(read_intent_in(&r.root, &id, &r.lane).unwrap().state, "incomplete");
}

/// IW1: a REAL read failure still decides nothing — B cannot be opened (no permission): the
/// recovery says so and changes nothing (A keeps this attempt's bytes, the intent stays `applying`);
/// once B can be read again, the next recovery finishes as usual.
#[cfg(target_os = "macos")]
#[test]
fn a_real_read_failure_still_decides_nothing() {
    use std::os::unix::fs::PermissionsExt;
    let r = rig("iw1perm");
    let id = rid("R-42");
    let files = two_target_candidate(&r, &id);
    let faults = StopOn { rel: "hooks/a2.sh", point: Point::BeforeDone, fired: Cell::new(false) };
    assert!(apply_candidate_in(&r.env(&faults, ApplyForm::Exchange), &id, &files, &|_, _, _| Ok(()), &pass_check, &pass_ref, &no_repair_lock).is_err());
    let calm = r.env(&NoFaults, ApplyForm::Exchange);
    resume_interrupted_in(&calm).unwrap();
    let b = r.pack.join("hooks/a2.sh");
    let mode = std::fs::metadata(&b).unwrap().permissions().mode();
    std::fs::set_permissions(&b, std::fs::Permissions::from_mode(0o000)).unwrap();
    let lines = recover_apply_intents_in(&calm, &no_repair_lock);
    std::fs::set_permissions(&b, std::fs::Permissions::from_mode(mode)).unwrap();
    assert!(lines.iter().any(|l| l.contains("정리하지 못했습니다")), "{lines:?}");
    assert_eq!(std::fs::read(r.pack.join("hooks/a1.sh")).unwrap(), b"NEW-1\n", "nothing decided");
    assert_eq!(read_intent_in(&r.root, &id, &r.lane).unwrap().state, "applying");
    let lines = recover_apply_intents_in(&calm, &no_repair_lock);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(std::fs::read(r.pack.join("hooks/a1.sh")).unwrap(), b"PRE-1\n");
    assert_eq!(std::fs::read(&b).unwrap(), b"PRE-2\n");
    assert_eq!(read_intent_in(&r.root, &id, &r.lane).unwrap().state, "undone");
}

/// IW1: `begin` on a file over the engine's read limit is refused with the limit named — the ledger
/// never records such a state (so an over-limit state equals no recorded state).
#[test]
fn begin_refuses_a_file_over_the_limit() {
    use crate::repair::durable::RECORD_READ_CAP as CAP;
    let r = rig("iw1begin");
    let id = rid("R-43");
    let big = r.pack.join("hooks/big.sh");
    sparse_file(&big, CAP + 1);
    let e = format!("{:?}", ledger::copy_preimage_in(&r.root, &id, &r.lane, "hooks/big.sh", &big, 1.0).expect_err("refused"));
    assert!(e.contains(&CAP.to_string()), "the limit is named: {e}");
}

/// IZ5 (reviewer1 on T7b, review 4): the settings file the registrar's dry run named is read inside
/// the pack-apply exclusion by the hook step — through the bounded reader: a FIFO at its name (a writer
/// offering valid settings to every reader) is refused at once (the hook unit declines), never waited
/// for; a file over the limit is refused without a byte of it read; a link to a regular settings file
/// is still followed (as before). Without IZ5 the plain read waited on the FIFO for its writer (here:
/// got the writer's settings) and read an oversized file whole.
#[cfg(unix)]
#[test]
fn the_hook_steps_settings_read_is_bounded_and_never_waits() {
    use crate::repair::durable::{read_seam, RECORD_READ_CAP as CAP};
    use std::io::Write;
    let r = rig("iz5");
    let json = br#"{"hooks": {"Stop": [{"matcher": "m", "hooks": [{"type": "command", "command": "bash h.sh", "timeout": 7}]}]}}"#;
    let real = r.base.join("cfg/settings.json");
    std::fs::create_dir_all(real.parent().unwrap()).unwrap();
    std::fs::write(&real, json).unwrap();
    let link = r.base.join("cfg/linked.json");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    assert!(found_matcher_timeout(&link, "Stop", "h.sh").is_ok(), "a link to a regular settings file is followed");
    let big = r.base.join("cfg/big.json");
    std::fs::File::create(&big).unwrap().set_len(CAP + 1).unwrap();
    assert!(found_matcher_timeout(&big, "Stop", "h.sh").is_err(), "over the limit: refused");
    assert_eq!(read_seam::bytes_read_from(&big), 0, "never read");
    let fifo = r.base.join("cfg/fifo.json");
    let c = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
    // SAFETY: a valid NUL-terminated path.
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o644) }, 0);
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (w_stop, w_at) = (std::sync::Arc::clone(&stop), fifo.clone());
    let writer = std::thread::spawn(move || {
        use std::os::unix::fs::FileTypeExt;
        while !w_stop.load(std::sync::atomic::Ordering::SeqCst) && std::fs::symlink_metadata(&w_at).is_ok_and(|m| m.file_type().is_fifo()) {
            match std::fs::OpenOptions::new().write(true).open(&w_at) {
                Ok(mut f) => {
                    let _ = f.write_all(json);
                }
                Err(_) => break,
            }
        }
    });
    let got = found_matcher_timeout(&fifo, "Stop", "h.sh");
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    let release = {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK).open(&fifo).unwrap()
    };
    writer.join().unwrap();
    drop(release);
    assert!(got.is_err(), "a FIFO settings file is refused at once: {got:?}");
}

/// Grows the DISPLACED inode (now the slot's payload) past the limit right after the exchange — a
/// writer that still holds a descriptor to the file that was live (codex T6b r3 M1).
struct GrowDisplaced {
    rel: &'static str,
    fired: Cell<bool>,
}

impl Faults for GrowDisplaced {
    fn at(&self, point: Point, live: &Path, slot: Option<&Slot>) -> bool {
        if point == Point::AfterExchange && live.ends_with(self.rel) && !self.fired.get() {
            if let Some(s) = slot {
                std::fs::OpenOptions::new().write(true).open(s.payload()).unwrap().set_len(crate::repair::durable::RECORD_READ_CAP + 1).unwrap();
                self.fired.set(true);
            }
        }
        false
    }
}

/// JH1 / JH2 (codex T6b r3 M1, M2): the inode an exchange displaced grows past the limit before the
/// seal — with a copy-on-write clone and with the byte copy: no hash of it is recorded
/// (`meta/sealed.json` holds neither `sealed_sha256` nor `slot_sha256_at_check` for it and says
/// "kept_unsealed" with the limit), its bytes stay in the slot as moved, nothing is re-lived from it,
/// and the deposit refuses to be a return's source (the slot's place and the limit named) — the same
/// answer after a restart (the record is on disk). Without JH1 / JH2 the reserved value was persisted
/// as `sealed_sha256` and later compared as a hash.
#[cfg(target_os = "macos")]
#[test]
fn a_displaced_file_grown_past_the_limit_is_kept_unsealed_and_never_a_hash() {
    use crate::repair::durable::RECORD_READ_CAP as CAP;
    for byte_copy in [false, true] {
        let r = rig("jh1grow");
        r.put("hooks/g.sh", b"OLD\n", 0o755);
        let (src, st) = r.source("g.sh", b"NEW\n", 0o755);
        let faults = GrowDisplaced { rel: "hooks/g.sh", fired: Cell::new(false) };
        crate::repair::vault::FORCE_BYTE_COPY_SEAL.with(|c| c.set(byte_copy));
        let env = r.env(&faults, ApplyForm::Exchange);
        let d = run_op(&env, &OpSpec {
            id: rid("R-2"),
            rel: "hooks/g.sh".into(),
            kind: "apply".into(),
            expected: Recorded::for_test(r.live("hooks/g.sh")),
            incoming: st,
            source: Source::Fresh(src),
            bound: Bound::now(&r.pack),
        });
        crate::repair::vault::FORCE_BYTE_COPY_SEAL.with(|c| c.set(false));
        let d = d.unwrap();
        assert!(faults.fired.get(), "byte_copy={byte_copy}");
        let slot = Slot::at(d.slot.as_ref().expect("the slot"), "hooks/g.sh");
        let meta: serde_json::Value = serde_json::from_slice(&std::fs::read(slot.meta_sealed()).unwrap()).unwrap();
        assert!(meta["sealed_sha256"].is_null(), "byte_copy={byte_copy}: no hash recorded: {meta}");
        assert!(meta["slot_sha256_at_check"].is_null(), "byte_copy={byte_copy}: no hash recorded: {meta}");
        assert_eq!(meta["kept_unsealed"]["limit"], serde_json::json!(CAP), "byte_copy={byte_copy}: {meta}");
        assert_eq!(std::fs::metadata(slot.payload()).unwrap().len(), CAP + 1, "byte_copy={byte_copy}: the bytes are kept as moved");
        assert_eq!(std::fs::read(r.pack.join("hooks/g.sh")).unwrap(), b"NEW\n", "byte_copy={byte_copy}: nothing re-lived");
        let e = format!("{:?}", crate::repair::vault::deposit_of(&slot).expect_err("never a source"));
        assert!(e.contains(&CAP.to_string()) && e.contains(&slot.dir.display().to_string()), "byte_copy={byte_copy}: {e}");
    }
}

/// JH2: an approved apply of two files; the inode B's exchange displaced grows past the limit (a
/// writer holds it) — B's operation ends superseded with its displaced bytes KEPT in its slot as moved
/// (never re-lived, never read whole), the apply is declined and undone from the begin copies (the
/// kept slot is not needed: A and B are back), the intent ends `undone`, and the next recovery does
/// nothing more. Before JH2 (run jh-red) the re-live read the over-limit sealed copy and the operation
/// ended in an ERROR — retried at every start.
#[cfg(target_os = "macos")]
#[test]
fn an_apply_whose_displaced_file_grew_past_the_limit_keeps_those_bytes_and_is_undone_once() {
    use crate::repair::durable::RECORD_READ_CAP as CAP;
    let r = rig("jh2undo");
    let id = rid("R-44");
    let files = two_target_candidate(&r, &id);
    let faults = GrowDisplaced { rel: "hooks/a2.sh", fired: Cell::new(false) };
    let rep = apply_candidate_in(&r.env(&faults, ApplyForm::Exchange), &id, &files, &|_, _, _| Ok(()), &pass_check, &pass_ref, &no_repair_lock).unwrap();
    assert!(faults.fired.get());
    let ApplyOutcome::Declined(why) = &rep.outcome else { panic!("{rep:?}") };
    assert!(why.contains(&CAP.to_string()), "the limit is said: {why}");
    assert_eq!(std::fs::read(r.pack.join("hooks/a1.sh")).unwrap(), b"PRE-1\n", "A is back");
    assert_eq!(std::fs::read(r.pack.join("hooks/a2.sh")).unwrap(), b"PRE-2\n", "B is back from its begin copy");
    assert!(
        r.slots().iter().any(|d| std::fs::metadata(d.join("payload").join("a2.sh")).is_ok_and(|m| m.len() == CAP + 1)),
        "the grown displaced bytes are kept in their slot"
    );
    assert_eq!(read_intent_in(&r.root, &id, &r.lane).unwrap().state, "undone");
    let calm = r.env(&NoFaults, ApplyForm::Exchange);
    assert!(recover_apply_intents_in(&calm, &no_repair_lock).is_empty(), "no retry");
}

/// JH2: a return whose pre-image would come from a slot kept unsealed over the limit never uses it —
/// the revert closes 충돌 (the slot's place and the limit in the sentence) and changes nothing; the
/// pending-revert recovery takes that target as a conflict and ends its intent `incomplete`, once.
/// Before JH2 the slot was a source like any other (the old deposit check ignored the field).
#[cfg(target_os = "macos")]
#[test]
fn a_return_needing_a_slot_kept_over_the_limit_is_a_conflict_once() {
    use crate::repair::durable::RECORD_READ_CAP as CAP;
    let kept = |slot_dir: &Path| {
        std::fs::write(
            slot_dir.join("meta").join("sealed.json"),
            serde_json::json!({"schema": 1, "sealed_sha256": null, "slot_sha256_at_check": null, "link_target_at_check": null,
                "kept_unsealed": {"reason": "over the limit", "limit": CAP, "len": CAP + 1}, "sealed_at": 1.0}).to_string(),
        )
        .unwrap();
    };
    // the revert
    let r = rig("jh2kept");
    let id = rid("R-46");
    let rec = committed_general(&r, &id, "hooks/k.sh", Some(b"PRE\n"), Some(b"POST\n"));
    let PreImage::Slot { slot_dir, .. } = &rec.files[0].pre else { panic!("a pre-image slot") };
    kept(slot_dir);
    let rep = revert_in(&r.env(&NoFaults, ApplyForm::Exchange), &id, RevertCaller::Human, &no_hooks, &no_repair_lock).unwrap();
    assert!(matches!(&rep.outcome, ApplyOutcome::Declined(m) if m.contains(&CAP.to_string()) && m.contains(&slot_dir.display().to_string())), "{rep:?}");
    assert_eq!(std::fs::read(r.pack.join("hooks/k.sh")).unwrap(), b"POST\n", "nothing changed");
    assert_eq!(ledger::read_lane_in(&r.root, &id, &r.lane).unwrap().unwrap().state, LaneState::Conflict);
    // the pending revert
    let r = rig("jh2keptpending");
    let id = rid("R-47");
    let rec = committed_general(&r, &id, "hooks/q.sh", Some(b"PRE\n"), Some(b"POST\n"));
    let PreImage::Slot { slot_dir, .. } = &rec.files[0].pre else { panic!("a pre-image slot") };
    kept(slot_dir);
    let ri = RevertIntent {
        schema: 1,
        repair: id.clone(),
        lane_key: r.lane.clone(),
        op_tag: "t".into(),
        state: "reverting".into(),
        targets: vec![RevertTarget { rel: "hooks/q.sh".into(), applied: r.live("hooks/q.sh").canon(), pre: rec.files[0].pre.state().canon() }],
        has_hooks: false,
        pack_generation: Bound::now(&r.pack).known(),
        by: BY_DAEMON_AUTO.into(),
        created_at: 1.0,
        updated_at: 1.0,
    };
    write_revert_intent(&r.root, &ri).unwrap();
    let env = r.env(&NoFaults, ApplyForm::Exchange);
    let lines = recover_revert_intents_in(&env, &no_hooks, &no_repair_lock);
    assert!(lines.iter().any(|l| l.contains(&CAP.to_string())), "{lines:?}");
    assert_eq!(std::fs::read(r.pack.join("hooks/q.sh")).unwrap(), b"POST\n", "nothing changed");
    assert!(recover_revert_intents_in(&env, &no_hooks, &no_repair_lock).is_empty(), "once — no retry");
}

/// JH2: the copy of the incoming bytes for the place / deposit-first forms reads through the bound —
/// a source over the limit is refused, never read whole. Without JH2 it was `std::fs::read`.
#[test]
fn the_incoming_copy_is_read_within_the_one_limit() {
    use crate::repair::durable::RECORD_READ_CAP as CAP;
    let r = rig("jh2incoming");
    let src = r.base.join("src-big");
    sparse_file(&src, CAP + 1);
    let st = TargetState::File { sha256: "0".repeat(64), mode: 0o644 };
    let env = r.env(&NoFaults, ApplyForm::DepositFirst);
    let got = incoming_copy(&env, &rid("R-45"), "op-jh2", &st, Some(&src));
    assert!(got.is_err(), "over the limit: refused: {got:?}");
}

// ───────────────────────────── RULINGS IL2 — what reaches another department ─────────────────────────────

/// IL2: the sentence about another department is true for the CLASS of the repair (0.15.0 has no
/// identical propagation of an approved repair — RULINGS II): a boot-critical or protection-related
/// repair reaches another department only through that department's own repair, both signatures and
/// the owner's final approval there; any other repair is carried there by that department's daemon,
/// and a department whose file differs keeps its bytes and shows 충돌. The approved apply's refusal in
/// another department says so. Before IL2 it said "(다른 부서는 전파)".
#[test]
fn the_sentence_about_other_departments_is_true_for_each_class() {
    let approval_form = "기동 필수·보호장치 관련 수리는 다른 부서에 저절로 전해지지 않습니다 — 다른 부서에는 그 부서 자신의 수리(cys repair begin … --resolves R-7)와 두 리뷰어의 서명, 그 부서에서의 주인 최종 승인으로만 들어갑니다";
    let carried_form = "이 수리는 각 부서의 데몬이 그 부서에 전파합니다 — 파일이 다른 부서는 제 바이트를 그대로 두고 그 수리를 충돌로 보입니다";
    for c in [Classification::BootCritical, Classification::ProtectionRelated] {
        assert_eq!(reach_of_other_departments(c, "R-7"), approval_form, "{c:?}");
    }
    for c in [Classification::General, Classification::RuleDocument, Classification::DaemonSource] {
        assert_eq!(reach_of_other_departments(c, "R-7"), carried_form, "{c:?}");
    }
    // the approved apply asked in another department
    let r = rig("il2apply");
    let id = rid("R-7");
    r.put("hooks/g.sh", b"PRE\n", 0o755);
    let pre_img = ledger::copy_preimage_in(&r.root, &id, &r.lane, "hooks/g.sh", &r.pack.join("hooks/g.sh"), 1.0).unwrap();
    let staging = ledger::staging_dir_in(&r.root, &id).join("hooks/g.sh");
    std::fs::create_dir_all(staging.parent().unwrap()).unwrap();
    std::fs::write(&staging, b"CAND\n").unwrap();
    let st = TargetState::read(&staging).unwrap();
    let fp = crate::repair::types::fingerprint(&[("hooks/g.sh".into(), st.clone())]);
    let mut rec = record(&r, &id, Classification::BootCritical, vec![FileChange { rel: "hooks/g.sh".into(), kind: ChangeKind::Edit, pre: pre_img, post: st, diff: None }], RepairState::AwaitingApproval);
    rec.approval = Some(approval(&fp));
    rec.origin_lane = LaneKey("dept-2".into());
    ledger::write_record_in(&r.root, &rec).unwrap();
    let env = r.env(&NoFaults, ApplyForm::Exchange);
    let e = format!("{:?}", apply_candidate_in(&env, &id, &[CandidateFile { rel: "hooks/g.sh".into(), staged: staging, mode: 0o644 }], &|_, _, _| Ok(()), &pass_check, &pass_ref, &no_repair_lock).expect_err("another department"));
    assert!(e.contains(approval_form), "{e}");
    assert!(!e.contains("다른 부서는 전파"), "{e}");
}
