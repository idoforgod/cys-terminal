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
}

impl Faults for Inject {
    fn at(&self, point: Point, live: &Path, _slot: Option<&Slot>) -> bool {
        if point != self.point || self.fired.get() {
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

/// THE property (RULINGS K3 SA:120, C2 2.4): for every operation × injected point × action, after
/// the operation and the recovery the next job run performs: (P1) every byte string that was live
/// at the instant of an exchange / move is in the vault — the one stated residual is a write
/// through an old descriptor in the instant between the exchange and the seal (G7 as reworded by
/// M2), which is then part of the deposit; (P2) no slot directory that existed is gone and every
/// slot is complete (O5 — never half-made); (P3) the live path is absent only for the move-in
/// operation; (P4) every operation record is finished.
#[test]
fn rule_one_property_every_op_every_point() {
    let points = [
        Point::AfterSlot,
        Point::AfterRecord,
        Point::AfterExchange,
        Point::AfterSeal,
        Point::AfterFirstMove,
        Point::BeforeDone,
    ];
    let mut combos = 0;
    for case in CASES {
        for point in points {
            if point == Point::AfterFirstMove && case.form != ApplyForm::DepositFirst {
                continue;
            }
            for action in [
                Action::Stop,
                Action::ReplaceLive(b"OTHER-WRITER\n".to_vec()),
                Action::OldFd(b"+late".to_vec()),
                Action::BumpGeneration,
            ] {
                combos += 1;
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
                let inj = Inject {
                    point,
                    action: action.clone(),
                    fired: Cell::new(false),
                    held: RefCell::new(held),
                    wrote_live: RefCell::new(None),
                    pack: r.pack.clone(),
                };
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
                }
                // the next job run
                let calm = NoFaults;
                let env2 = r.env(&calm, case.form);
                resume_interrupted_in(&env2).unwrap();
                let tag = format!("{} at {point:?} with {}", case.name, match &action {
                    Action::Stop => "stop",
                    Action::ReplaceLive(_) => "replace-live",
                    Action::OldFd(_) => "old-fd",
                    Action::BumpGeneration => "bump-generation",
                });
                // P1
                let vault = r.vault_bytes();
                let before_exchange = matches!(point, Point::AfterSlot | Point::AfterRecord);
                let up_to_seal = before_exchange || point == Point::AfterExchange;
                if let Some(b) = &live_bytes {
                    // the bytes live at the instant of the exchange / first move: the initial bytes,
                    // plus an old descriptor's write if it landed before the seal (the stated residual
                    // G7/M2 between the exchange and the seal); another writer's rename-over BEFORE the
                    // exchange replaced them (that writer's bytes are checked below)
                    let replaced_before = matches!(action, Action::ReplaceLive(_)) && before_exchange;
                    let want = if matches!(action, Action::OldFd(_)) && up_to_seal {
                        let mut w = b.clone();
                        w.extend_from_slice(b"+late");
                        w
                    } else {
                        b.clone()
                    };
                    let live_now = std::fs::read(r.pack.join(case.rel)).ok();
                    assert!(
                        replaced_before || vault.contains(&want) || live_now.as_deref() == Some(want.as_slice()),
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
                // P2 / O5
                for s in r.slots() {
                    let name = s.file_name().unwrap().to_string_lossy().to_string();
                    assert!(!name.starts_with(".building"), "P2 {tag}: half-made slot left: {name}");
                    if crate::repair::ledger::is_slot_name(&name) {
                        assert!(s.join("payload").is_dir() && s.join("sealed").is_dir() && s.join("meta/pre.json").is_file(), "O5 {tag}: incomplete slot {}", s.display());
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
    assert!(combos >= 90, "only {combos} combinations ran");
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
        let inj = Inject {
            point: Point::AfterRecord,
            action: Action::ReplaceLive(other.clone()),
            fired: Cell::new(false),
            held: RefCell::new(None),
            wrote_live: RefCell::new(None),
            pack: r.pack.clone(),
        };
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
        let inj = Inject { point: Point::AfterFirstMove, action: Action::Stop, fired: Cell::new(false), held: RefCell::new(None), wrote_live: RefCell::new(None), pack: r.pack.clone() };
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
    assert_eq!(resume_hook_steps_in(&r.root, &r.pack, &r.lane, &ok, 3.0), 1);
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
