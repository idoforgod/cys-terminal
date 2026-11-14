//! Apply / revert / propagation entry points (C2, design §4-9, 11.2, TESTPLAN OQ-D7/OQ-D8, RULINGS
//! H3, F9, R2, R5/P6, X1, Y1, Y2). Bodies of T6: rule one (the ONE engine every daemon write of a live
//! pack file goes through), the approved apply, revert, recovery of interrupted operations, F9's
//! triple, R2's origin triple, P6's needed slots and the hook step. `propagate`, `catchup_status` and
//! `recheck_lane` are T7b's (stubs here).
//!
//! RULE ONE (owner decision 29, C2 2.4): on macOS every change this mechanism makes to a live pack
//! file is an atomic EXCHANGE of the live entry with a FRESH vault slot (or, where nothing live is
//! replaced, a move INTO a fresh slot / a no-replace PLACEMENT of a copy). The mechanism never
//! deletes, moves or overwrites a slot entry and never writes a live path by rename-over or in place.
//! Windows (no exchange): the deposit-first form — the live file moved into a fresh slot, then the
//! new content placed without replace (general files only; automatic return is off).

use super::boot_critical::CheckKind;
use super::exchange::{entry_identity, exchange, identity_of, place_no_replace, same_filesystem};
use super::ledger::{self, ApprovalResult, PackGeneration, PreImage, RepairRecord};
use super::platform::ApplyForm;
use super::rehearsal::CheckResult;
use super::types::{
    Classification, Identity, LaneKey, LaneState, RepairError, RepairId, RepairState, TargetState,
};
use super::vault::{self, DepositKind, Slot, Staged};
use super::wal::{self, OpRecord};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// One staged file of an approved candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateFile {
    pub rel: String,
    pub staged: PathBuf,
    pub mode: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyOutcome {
    Applied,
    Superseded(String),
    Declined(String),
}

/// Who asks for a revert (C3: human, a worker on a master's ticket, the daemon in-process — never
/// the master).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevertCaller {
    Human,
    TicketedWorker { seat: u64 },
    DaemonAuto,
}

// ───────────────────────────── the rule-one engine ─────────────────────────────

/// Points where a test injects a crash or another writer (the property test of K3 SA:120). In
/// production nothing is injected ([`NoFaults`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Point {
    /// The fresh slot exists (built and named); no operation record yet.
    AfterSlot,
    /// The operation record is durable; nothing live touched yet (another writer may write here —
    /// "between the check and the exchange").
    AfterRecord,
    /// The exchange / first move returned; the seal has not run (M2's instant).
    AfterExchange,
    /// Sealed and flushed; step 4 not run.
    AfterSeal,
    /// Windows form: the first move is done and sealed; the second move not done.
    AfterFirstMove,
    /// Everything done except closing the record.
    BeforeDone,
    /// CT1 — inside the seal: the temporary entry exists (nothing written / a clone just made).
    InSealCreated,
    /// CT1 — inside the byte-copy seal: about half of the bytes written.
    InSealPartial,
    /// CT1 — inside the seal: complete and flushed, not yet placed under `sealed/<basename>`.
    InSealWritten,
    /// CT1 — inside the seal: placed, the folder not yet flushed.
    InSealPlaced,
    /// CT1 — inside the publication of `meta/sealed.json`: about half written.
    InMetaPartial,
    /// CT1 — `meta/sealed.json` complete and flushed, not yet placed.
    InMetaWritten,
    /// CT12 — the approved apply: the slot was validated against the approved state, its operation
    /// not yet recorded or exchanged.
    AfterValidation,
    /// DX2 — the approved apply's record is committed (시험 기간), its lane row not yet written.
    AfterRecordCommit,
}

/// Fault injection (tests). `at` returns `true` to make the operation stop right there as if the
/// process died (the record stays as written).
pub trait Faults {
    fn at(&self, point: Point, live: &Path, slot: Option<&Slot>) -> bool;
}

/// Production: nothing injected.
pub struct NoFaults;

impl Faults for NoFaults {
    fn at(&self, _point: Point, _live: &Path, _slot: Option<&Slot>) -> bool {
        false
    }
}

/// The marker of an injected stop (tests only ever produce it).
pub const INJECTED_STOP: &str = "__injected_stop__";

/// Everything an operation needs.
pub struct OpEnv<'a> {
    /// The repairs root (`paths::repairs_root()` in production).
    pub root: &'a Path,
    /// The lane's live pack root.
    pub pack: &'a Path,
    pub lane: &'a LaneKey,
    pub now: f64,
    /// `platform::apply_form()` in production; tests run the Windows form on any system.
    pub form: ApplyForm,
    pub faults: &'a dyn Faults,
    /// H6(1): the file's OWN check on displaced bytes (`(rel, file holding the bytes)` →
    /// `Some(passes)`, `None` = the file has no own check). The daemon passes the scheduler's
    /// `validate_schedule_root` for `schedule.json`.
    pub own_check: &'a dyn Fn(&str, &Path) -> Option<bool>,
}

/// What one operation does to one target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpSpec {
    pub id: RepairId,
    pub rel: String,
    /// `apply` | `revert` | `undo-superseded` | `m10-revert`
    pub kind: String,
    /// The live state expected at step 0 (what is displaced).
    pub expected: TargetState,
    /// The state put live.
    pub incoming: TargetState,
    /// The file holding the incoming bytes (a `File` incoming state).
    pub source: Option<PathBuf>,
    /// The generation this operation is bound to (EJ1: required — Known or Unknown; Unknown places
    /// nothing).
    pub bound: Bound,
}

/// The result of one operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpDone {
    pub outcome: ApplyOutcome,
    pub op: Option<String>,
    pub slot: Option<PathBuf>,
}

fn done(outcome: ApplyOutcome, op: Option<&OpRecord>) -> OpDone {
    OpDone {
        outcome,
        op: op.map(|r| r.op.clone()),
        slot: op.and_then(|r| r.slot.clone()),
    }
}

fn stop(p: Point) -> RepairError {
    RepairError::Io(format!("{INJECTED_STOP} at {p:?}"))
}

/// The pack generation of a lane's pack (C1 1.6, RULINGS O4): `.pack-version` + the pro revision
/// (only for the pro channel — a pro revision under the same version is another generation).
/// RULINGS FG1 — the pack generation with three answers: `Ok` (a readable `.pack-version`; the pro
/// revision when the state says pro; a state file ABSENT = the free pack — the named legacy case);
/// `Err` for everything else (no / unreadable / empty version, a CORRUPT state, a state whose
/// `base_version` disagrees with `.pack-version`). Decisions read it through `Bound::now`.
pub fn pack_generation_checked(pack: &Path) -> Result<PackGeneration, String> {
    let version = std::fs::read_to_string(pack.join(".pack-version"))
        .map_err(|e| format!(".pack-version: {e}"))?
        .trim()
        .to_string();
    if version.is_empty() {
        return Err(".pack-version: 비어 있음".into());
    }
    let pro_revision = match crate::pack::read_pack_state(pack) {
        crate::pack::PackStateRead::Absent => None,
        crate::pack::PackStateRead::Valid(st) => {
            if !st.base_version.is_empty() && st.base_version != version {
                return Err(format!("팩 상태의 기준 판({})이 .pack-version({version})과 다름", st.base_version));
            }
            (st.channel == "pro").then_some(st.pro_revision)
        }
        crate::pack::PackStateRead::Corrupt(m) => return Err(format!("팩 상태 손상: {m}")),
    };
    Ok(PackGeneration { version, pro_revision })
}

/// The pack generation for DISPLAY and records only ("unknown" when it cannot be read — never a
/// decision: decisions use [`Bound::now`]).
pub fn pack_generation(pack: &Path) -> PackGeneration {
    let version = std::fs::read_to_string(pack.join(".pack-version"))
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    let pro_revision = match crate::pack::read_pack_state(pack) {
        crate::pack::PackStateRead::Valid(st) if st.channel == "pro" => Some(st.pro_revision),
        _ => None,
    };
    PackGeneration {
        version,
        pro_revision,
    }
}

/// RULINGS EJ1 / EN3 — the BOUND generation of an operation: `Known`, or `Unknown`, which equals
/// nothing. It is created ONCE, at the operation's birth (its first durable record — the apply
/// intent, the revert intent from the lane row, the scan's sampling written into its operation
/// record), and carried from there to every nested operation; nobody substitutes "current" or
/// "none".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Bound {
    Known(PackGeneration),
    Unknown,
}

impl Bound {
    /// THE one reading of a generation value (EN3): the empty string and the literal `unknown` are
    /// Unknown; nothing else in the repair module compares the raw string.
    pub fn of(g: Option<PackGeneration>) -> Bound {
        match g {
            Some(g) if !g.version.is_empty() && g.version != "unknown" => Bound::Known(g),
            _ => Bound::Unknown,
        }
    }
    /// THE one reading of a lane row's probation generation (EN3; a reconstructed row holds "").
    pub fn from_row(p: &ledger::Probation) -> Bound {
        Bound::of(Some(PackGeneration { version: p.pack_generation.clone(), pro_revision: p.pro_revision }))
    }
    /// The pack's generation read at this moment (FG1: unreadable / inconsistent metadata → Unknown).
    pub fn now(pack: &Path) -> Bound {
        match pack_generation_checked(pack) {
            Ok(g) => Bound::of(Some(g)),
            Err(_) => Bound::Unknown,
        }
    }
    /// Known AND equal to the pack's generation read now (Unknown equals nothing).
    pub fn equals_now(&self, pack: &Path) -> bool {
        matches!((self, Bound::now(pack)), (Bound::Known(a), Bound::Known(b)) if *a == b)
    }
    pub fn known(&self) -> Option<PackGeneration> {
        match self {
            Bound::Known(g) => Some(g.clone()),
            Bound::Unknown => None,
        }
    }
    /// Whether this is the free-channel generation of release `version` (the binary's own).
    pub fn is_release(&self, version: &str) -> bool {
        matches!(self, Bound::Known(g) if g.pro_revision.is_none() && g.version == version)
    }
    /// The two probation fields a lane row stores for this binding (Unknown → "" — EN3).
    pub fn row_fields(&self) -> (String, Option<u32>) {
        match self {
            Bound::Known(g) => (g.version.clone(), g.pro_revision),
            Bound::Unknown => (String::new(), None),
        }
    }
}

/// The acts of THE door (EJ1).
#[derive(Debug, Clone, Copy)]
enum DoorAct<'a> {
    /// `renamex_np(RENAME_SWAP)` of the live entry with a slot payload.
    Exchange { payload: &'a Path },
    /// A no-replace placement of a non-slot copy at the (absent) live path.
    Place { copy: &'a Path },
    /// The live entry moved into a fresh slot payload (no replace).
    MoveOut { into: &'a Path },
    /// The missing live parent folders of a placement.
    MakeParents,
}

/// RULINGS EJ1 — THE door into a live pack path: every rename, exchange, placement or folder
/// creation at a live path of the repair module happens here (pinned by
/// `live_writes_go_through_the_one_door`). It reads the pack's generation AT THIS MOMENT and acts
/// only when the operation's bound generation is Known and equal; otherwise `Superseded`, nothing
/// touched.
fn door(env: &OpEnv<'_>, bound: &Bound, live: &Path, act: DoorAct<'_>) -> Result<(), RepairError> {
    if !bound.equals_now(env.pack) {
        return Err(RepairError::Superseded(format!(
            "{}: 팩 세대가 이 동작이 묶인 세대와 다르거나 알 수 없어 살아 있는 자리를 바꾸지 않았습니다",
            live.display()
        )));
    }
    match act {
        DoorAct::Exchange { payload } => exchange(live, payload),
        DoorAct::Place { copy } => place_no_replace(copy, live),
        DoorAct::MoveOut { into } => place_no_replace(live, into),
        DoorAct::MakeParents => std::fs::create_dir_all(live).map_err(|e| RepairError::Io(format!("{}: {e}", live.display()))),
    }
}

fn parse_canon(s: &str) -> Option<TargetState> {
    if s == "absent" {
        return Some(TargetState::Absent);
    }
    if let Some(t) = s.strip_prefix("link:") {
        return Some(TargetState::Link {
            target: t.to_string(),
        });
    }
    let rest = s.strip_prefix("file:")?;
    let (sha, mode) = rest.rsplit_once(':')?;
    Some(TargetState::File {
        sha256: sha.to_string(),
        mode: u32::from_str_radix(mode, 8).ok()?,
    })
}

/// `canon` back to a state (the ledger stores `(rel, canon)` lists — U2).
pub fn state_of_canon(s: &str) -> Option<TargetState> {
    parse_canon(s)
}

fn staged_of(st: &TargetState, source: Option<&Path>) -> Result<Staged, RepairError> {
    Ok(match st {
        TargetState::File { mode, .. } => Staged::FileFrom {
            source: source
                .ok_or_else(|| RepairError::Precondition("들일 내용의 원본이 없습니다".into()))?
                .to_path_buf(),
            mode: *mode,
        },
        TargetState::Link { target } => Staged::Link {
            target: target.clone(),
        },
        TargetState::Absent => Staged::Empty,
    })
}

/// A NON-slot copy of the incoming content in the repair's vault folder (`.incoming-<op>`), for a
/// placement: placing moves this copy, never a slot entry.
fn incoming_copy(
    env: &OpEnv<'_>,
    id: &RepairId,
    op: &str,
    st: &TargetState,
    source: Option<&Path>,
) -> Result<PathBuf, RepairError> {
    let vault = vault::vault_dir_in(env.root, id);
    std::fs::create_dir_all(&vault)
        .map_err(|e| RepairError::Io(format!("{}: {e}", vault.display())))?;
    let p = vault.join(format!(".incoming-{op}"));
    match staged_of(st, source)? {
        Staged::FileFrom { source, mode } => {
            let m = std::fs::symlink_metadata(&source)
                .map_err(|e| RepairError::Io(format!("{}: {e}", source.display())))?;
            if !m.file_type().is_file() {
                return Err(RepairError::Precondition(format!(
                    "{}: 일반 파일이 아닙니다",
                    source.display()
                )));
            }
            let bytes = std::fs::read(&source)
                .map_err(|e| RepairError::Io(format!("{}: {e}", source.display())))?;
            write_new(&p, &bytes, mode)?;
        }
        Staged::Bytes { bytes, mode } => write_new(&p, &bytes, mode)?,
        Staged::Link { target } => {
            #[cfg(unix)]
            std::os::unix::fs::symlink(&target, &p)
                .map_err(|e| RepairError::Io(format!("{}: {e}", p.display())))?;
            #[cfg(not(unix))]
            {
                let _ = target;
                return Err(RepairError::PlatformPending(
                    "링크는 이 운영체제에서 놓지 않습니다",
                ));
            }
        }
        Staged::Empty => {
            return Err(RepairError::Precondition("놓을 내용이 없습니다".into()));
        }
    }
    super::durable::full_flush_dir(&vault)?;
    Ok(p)
}

/// Writes a NEW file (never over anything) with exactly this mode, fully flushed — published
/// complete (CT1: a temporary entry in the same folder, a full flush, a no-replace placement).
pub fn write_new(p: &Path, bytes: &[u8], mode: u32) -> Result<(), RepairError> {
    vault::publish_new(p, bytes, mode)
}

/// The seal right after an exchange / deposit move (M2). Only a test's fault hook (a no-op call
/// in production) sits before the clone — no flush, no hash, no record update.
fn seal_now(env: &OpEnv<'_>, slot: &Slot, live: &Path) -> Result<(), RepairError> {
    if env.faults.at(Point::AfterExchange, live, Some(slot)) {
        return Err(stop(Point::AfterExchange));
    }
    seal_hooked(env, slot, live)
}

/// The seal with the test's crash points inside it (CT1).
fn seal_hooked(env: &OpEnv<'_>, slot: &Slot, live: &Path) -> Result<(), RepairError> {
    vault::seal_with(slot, &mut |step| {
        let point = match step {
            vault::PublishStep::Created => Point::InSealCreated,
            vault::PublishStep::Partial => Point::InSealPartial,
            vault::PublishStep::Written => Point::InSealWritten,
            vault::PublishStep::Placed => Point::InSealPlaced,
        };
        if env.faults.at(point, live, Some(slot)) {
            Err(stop(point))
        } else {
            Ok(())
        }
    })
}

/// `meta/sealed.json` with the test's crash points inside its publication (CT1).
fn finish_seal_hooked(env: &OpEnv<'_>, slot: &Slot, live: &Path) -> Result<vault::SealFacts, RepairError> {
    vault::finish_seal_with(slot, env.now, &mut |step| {
        let point = match step {
            vault::PublishStep::Partial => Point::InMetaPartial,
            vault::PublishStep::Written => Point::InMetaWritten,
            _ => return Ok(()),
        };
        if env.faults.at(point, live, Some(slot)) {
            Err(stop(point))
        } else {
            Ok(())
        }
    })
}

fn flush_parents(live: &Path, slot: &Slot) -> Result<(), RepairError> {
    if let Some(p) = live.parent() {
        super::durable::full_flush_dir(p)?;
    }
    super::durable::full_flush_dir(&slot.dir.join("payload"))
}

fn new_record(env: &OpEnv<'_>, spec: &OpSpec, op: &str, form: &str, gen: &PackGeneration) -> OpRecord {
    OpRecord {
        schema: super::REPAIR_SCHEMA,
        op: op.to_string(),
        repair: spec.id.clone(),
        lane_key: env.lane.clone(),
        kind: spec.kind.clone(),
        form: form.to_string(),
        rel: spec.rel.clone(),
        pre_existed: spec.expected != TargetState::Absent,
        expected: spec.expected.canon(),
        incoming: spec.incoming.canon(),
        pack_generation: Some(gen.clone()),
        slot: None,
        slot_identity: None,
        live_identity: None,
        incoming_copy: None,
        incoming_copy_identity: None,
        step: "staged".to_string(),
        hooks: Vec::new(),
        finished: false,
        outcome: None,
        note: String::new(),
        created_at: env.now,
        updated_at: env.now,
        notice: None,
    }
}

/// The door refused an act: a generation mismatch / an occupied place → `superseded`, anything
/// else → `declined`; the operation's record is closed with it (nothing was touched).
fn door_refused(env: &OpEnv<'_>, rec: &mut OpRecord, spec: &OpSpec, e: RepairError) -> Result<OpDone, RepairError> {
    match e {
        RepairError::Superseded(m) => {
            wal::finish(env.root, rec, "superseded", &m, env.now)?;
            Ok(done(ApplyOutcome::Superseded(m), Some(rec)))
        }
        e => {
            wal::finish(env.root, rec, "declined", &format!("{e:?}"), env.now)?;
            Ok(done(ApplyOutcome::Declined(format!("{}: {e:?}", spec.rel)), Some(rec)))
        }
    }
}

fn same_entry(a: &Identity, b: &(String, u64)) -> bool {
    a.volume_uuid == b.0 && a.inode == b.1
}

/// Runs ONE rule-one operation on one target (design §4-9 steps 0–5). Returns the outcome; an
/// `Err` means the operation could not even be decided (nothing changed) or was stopped by a test.
pub fn run_op(env: &OpEnv<'_>, spec: &OpSpec) -> Result<OpDone, RepairError> {
    run_op_depth(env, spec, 0)
}

fn run_op_depth(env: &OpEnv<'_>, spec: &OpSpec, depth: u8) -> Result<OpDone, RepairError> {
    super::types::parent_chain_link(env.pack, &spec.rel)?;
    let live = env.pack.join(&spec.rel);
    let parent = live
        .parent()
        .ok_or_else(|| RepairError::Precondition(format!("{}: 부모 폴더가 없습니다", spec.rel)))?
        .to_path_buf();
    if spec.expected == TargetState::Absent && spec.incoming != TargetState::Absent && std::fs::symlink_metadata(&parent).is_err() {
        if let Err(e) = door(env, &spec.bound, &parent, DoorAct::MakeParents) {
            return match e {
                RepairError::Superseded(m) => Ok(done(ApplyOutcome::Superseded(m), None)),
                e => Err(e),
            };
        }
    }
    // step 0 — same filesystem (the repair's vault folder vs the live PARENT, real device ids)
    let vault_dir = vault::vault_dir_in(env.root, &spec.id);
    std::fs::create_dir_all(&vault_dir)
        .map_err(|e| RepairError::Io(format!("{}: {e}", vault_dir.display())))?;
    match same_filesystem(&vault_dir, &parent) {
        Ok(true) => {}
        Ok(false) => {
            return Ok(done(
                ApplyOutcome::Declined(format!(
                    "{}: 보관소와 살아 있는 파일이 다른 파일시스템에 있어 바꾸지 않습니다(아무것도 바꾸지 않았습니다)",
                    spec.rel
                )),
                None,
            ))
        }
        Err(RepairError::Declined(m)) => return Ok(done(ApplyOutcome::Declined(m), None)),
        Err(e) => return Err(e),
    }
    // step 0 — the live entry is the expected one, the pack generation the bound one
    let now_state = TargetState::read(&live)?;
    if now_state != spec.expected {
        return Ok(done(
            ApplyOutcome::Superseded(format!(
                "{}: 살아 있는 파일이 기대한 상태가 아닙니다(기대 {} · 지금 {}) — 아무것도 바꾸지 않았습니다",
                spec.rel,
                spec.expected.canon(),
                now_state.canon()
            )),
            None,
        ));
    }
    // EJ1: the bound generation must be Known and the pack's now (Unknown equals nothing)
    let Some(gen) = spec.bound.known().filter(|_| spec.bound.equals_now(env.pack)) else {
        return Ok(done(
            ApplyOutcome::Superseded(format!(
                "{}: 팩 세대가 이 동작이 묶인 세대와 다르거나 알 수 없습니다(지금 {}) — 옛 세대의 바이트는 되살리지 않습니다",
                spec.rel,
                pack_generation(env.pack).version
            )),
            None,
        ));
    };
    if spec.incoming == spec.expected {
        return Ok(done(ApplyOutcome::Applied, None));
    }
    let op = wal::new_op_id(env.now, &spec.kind);
    match (&spec.expected, &spec.incoming, env.form) {
        (TargetState::Absent, _, _) => op_place(env, spec, &op, &live, &gen),
        (_, TargetState::Absent, _) => op_move_in(env, spec, &op, &live, &gen, depth),
        (_, _, ApplyForm::Exchange) => op_exchange(env, spec, &op, &live, &gen, depth),
        (_, _, ApplyForm::DepositFirst) => op_deposit_first(env, spec, &op, &live, &gen),
        (_, _, ApplyForm::Unsupported) => Ok(done(
            ApplyOutcome::Declined("이 운영체제에서는 살아 있는 팩 파일을 바꾸지 않습니다".into()),
            None,
        )),
    }
}

fn op_exchange(
    env: &OpEnv<'_>,
    spec: &OpSpec,
    op: &str,
    live: &Path,
    gen: &PackGeneration,
    depth: u8,
) -> Result<OpDone, RepairError> {
    let staged = staged_of(&spec.incoming, spec.source.as_deref())?;
    let slot = vault::build_slot(
        env.root,
        &spec.id,
        env.lane,
        &spec.rel,
        op,
        DepositKind::Displaced,
        &spec.expected,
        &staged,
        env.now,
    )?;
    if env.faults.at(Point::AfterSlot, live, Some(&slot)) {
        return Err(stop(Point::AfterSlot));
    }
    if TargetState::read(&slot.payload())? != spec.incoming {
        return Ok(done(
            ApplyOutcome::Declined(format!(
                "{}: 보관소에 놓은 내용이 들일 상태와 다릅니다 — 바꾸지 않습니다",
                spec.rel
            )),
            None,
        ));
    }
    let (slot_identity, live_identity) = match (identity_of(&slot.payload()), identity_of(live)) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(e), _) | (_, Err(e)) => {
            return Ok(done(
                ApplyOutcome::Superseded(format!("{}: {e:?}", spec.rel)),
                None,
            ))
        }
    };
    let mut rec = new_record(env, spec, op, "exchange", gen);
    rec.slot = Some(slot.dir.clone());
    rec.slot_identity = Some(slot_identity);
    rec.live_identity = Some(live_identity);
    // step 1: the record is durable BEFORE anything live is touched
    if let Err(e) = wal::write_in(env.root, &rec) {
        return Ok(done(
            ApplyOutcome::Declined(format!(
                "{}: 동작 기록을 쓸 수 없어 적용하지 않습니다 — {e:?}",
                spec.rel
            )),
            None,
        ));
    }
    if env.faults.at(Point::AfterRecord, live, Some(&slot)) {
        return Err(stop(Point::AfterRecord));
    }
    // step 3: the exchange; the seal is the very next filesystem operation (M2)
    if let Err(e) = door(env, &spec.bound, live, DoorAct::Exchange { payload: &slot.payload() }) {
        return door_refused(env, &mut rec, spec, e);
    }
    seal_now(env, &slot, live)?;
    after_seal_exchange(env, spec, &mut rec, &slot, live, depth)
}

/// Steps after the seal of an exchange (also the recovery path): flushes, `sealed.json`, step 4.
fn after_seal_exchange(
    env: &OpEnv<'_>,
    spec: &OpSpec,
    rec: &mut OpRecord,
    slot: &Slot,
    live: &Path,
    depth: u8,
) -> Result<OpDone, RepairError> {
    flush_parents(live, slot)?;
    let facts = finish_seal_hooked(env, slot, live)?;
    wal::set_step(env.root, rec, "sealed", env.now)?;
    if env.faults.at(Point::AfterSeal, live, Some(slot)) {
        return Err(stop(Point::AfterSeal));
    }
    step4(env, spec, rec, slot, live, &facts.sealed_state, depth)
}

/// Step 4: the sealed deposit is the expected state and the pack generation unchanged → applied;
/// else superseded, and (OQ-D7, H6(1)) a COPY of the displaced bytes is put back by the same rule
/// only when the live entry still has the identity + hash this operation left there and the bytes
/// pass the file's own check where one exists.
fn step4(
    env: &OpEnv<'_>,
    spec: &OpSpec,
    rec: &mut OpRecord,
    slot: &Slot,
    live: &Path,
    deposit: &TargetState,
    depth: u8,
) -> Result<OpDone, RepairError> {
    let gen_ok = Bound::of(rec.pack_generation.clone()).equals_now(env.pack);
    let expected = parse_canon(&rec.expected).unwrap_or(TargetState::Absent);
    if *deposit == expected && gen_ok {
        // CT3 — the outcome reported is the outcome measured: what is NOW live must be the state
        // this operation was to put live; otherwise that entry goes into a fresh slot by a CHECKED
        // operation (expected = what is actually live) and the previous bytes return
        let incoming = parse_canon(&rec.incoming).unwrap_or(TargetState::Absent);
        let live_now = TargetState::read(live)?;
        if live_now != incoming {
            let mut note = format!(
                "{}: 들인 뒤 살아 있는 것이 들일 상태가 아닙니다(지금 {} ≠ 들일 {})",
                spec.rel,
                live_now.canon(),
                incoming.canon()
            );
            if depth == 0 {
                let holder = if std::fs::symlink_metadata(slot.sealed()).is_ok() {
                    slot.sealed()
                } else {
                    slot.payload()
                };
                let back = OpSpec {
                    id: spec.id.clone(),
                    rel: spec.rel.clone(),
                    kind: "undo-mismatch".into(),
                    expected: live_now,
                    incoming: deposit.clone(),
                    source: Some(holder),
                    bound: Bound::of(rec.pack_generation.clone()),
                };
                let r = run_op_depth(env, &back, depth + 1)?;
                if r.outcome == ApplyOutcome::Applied {
                    note.push_str(" — 그것을 새 보관 칸에 두고 이전 바이트를 되돌렸습니다");
                } else {
                    note.push_str(&format!(" — 이전 바이트를 되돌리지 못했습니다(충돌 · {:?})", r.outcome));
                }
            } else {
                note.push_str(" — 겹친 동작이라 더 바꾸지 않습니다(워커 검토)");
            }
            if env.faults.at(Point::BeforeDone, live, Some(slot)) {
                return Err(stop(Point::BeforeDone));
            }
            wal::finish(env.root, rec, "declined", &note, env.now)?;
            return Ok(done(ApplyOutcome::Declined(note), Some(rec)));
        }
        if env.faults.at(Point::BeforeDone, live, Some(slot)) {
            return Err(stop(Point::BeforeDone));
        }
        wal::finish(env.root, rec, "applied", "", env.now)?;
        return Ok(done(ApplyOutcome::Applied, Some(rec)));
    }
    let mut note = if *deposit != expected {
        format!(
            "{}: 보관소에 들어간 것이 기대한 바이트가 아닙니다(다른 작성자가 그 사이에 썼습니다 · {} ≠ {})",
            spec.rel,
            deposit.canon(),
            expected.canon()
        )
    } else {
        format!("{}: 그 사이 팩 세대가 바뀌었습니다", spec.rel)
    };
    if *deposit != expected && depth == 0 {
        // the other writer's bytes are re-lived only from the SEALED copy (or the link itself)
        let holder = if std::fs::symlink_metadata(slot.sealed()).is_ok() {
            slot.sealed()
        } else {
            slot.payload()
        };
        let relive_ok = (env.own_check)(&spec.rel, &holder) != Some(false);
        let left = match rec.form.as_str() {
            "exchange" => rec.slot_identity.clone(),
            _ => None,
        };
        let live_still_ours = match (&left, identity_of(live)) {
            (Some(l), Ok(now)) => *l == now,
            (None, _) => false,
            (_, Err(_)) => false,
        };
        if !relive_ok {
            note.push_str(" · 밀려난 바이트가 그 파일 자신의 검사를 통과하지 못해 되살리지 않았습니다(검증한 내용이 살아 있음 · 두 판 모두 보관소에)");
        } else if !live_still_ours {
            note.push_str(" · 살아 있는 자리가 이 동작이 놓은 그대로가 아니어서 더 바꾸지 않았습니다");
        } else {
            let relive = OpSpec {
                id: spec.id.clone(),
                rel: spec.rel.clone(),
                kind: "undo-superseded".into(),
                expected: TargetState::read(live)?,
                incoming: deposit.clone(),
                source: Some(holder),
                bound: Bound::of(rec.pack_generation.clone()),
            };
            let r = run_op_depth(env, &relive, depth + 1)?;
            if r.outcome == ApplyOutcome::Applied {
                note.push_str(" · 밀려난 바이트의 사본을 다시 살렸습니다");
            } else {
                note.push_str(&format!(" · 밀려난 바이트의 사본을 다시 살리지 못했습니다({:?})", r.outcome));
            }
        }
    }
    if env.faults.at(Point::BeforeDone, live, Some(slot)) {
        return Err(stop(Point::BeforeDone));
    }
    wal::finish(env.root, rec, "superseded", &note, env.now)?;
    Ok(done(ApplyOutcome::Superseded(note), Some(rec)))
}

fn op_move_in(
    env: &OpEnv<'_>,
    spec: &OpSpec,
    op: &str,
    live: &Path,
    gen: &PackGeneration,
    depth: u8,
) -> Result<OpDone, RepairError> {
    let slot = vault::build_slot(
        env.root,
        &spec.id,
        env.lane,
        &spec.rel,
        op,
        DepositKind::MovedIn,
        &spec.expected,
        &Staged::Empty,
        env.now,
    )?;
    if env.faults.at(Point::AfterSlot, live, Some(&slot)) {
        return Err(stop(Point::AfterSlot));
    }
    let live_identity = match identity_of(live) {
        Ok(i) => i,
        Err(e) => {
            return Ok(done(
                ApplyOutcome::Superseded(format!("{}: {e:?}", spec.rel)),
                None,
            ))
        }
    };
    let mut rec = new_record(env, spec, op, "move-in", gen);
    rec.slot = Some(slot.dir.clone());
    rec.live_identity = Some(live_identity);
    if let Err(e) = wal::write_in(env.root, &rec) {
        return Ok(done(
            ApplyOutcome::Declined(format!(
                "{}: 동작 기록을 쓸 수 없어 적용하지 않습니다 — {e:?}",
                spec.rel
            )),
            None,
        ));
    }
    if env.faults.at(Point::AfterRecord, live, Some(&slot)) {
        return Err(stop(Point::AfterRecord));
    }
    if let Err(e) = door(env, &spec.bound, live, DoorAct::MoveOut { into: &slot.payload() }) {
        return door_refused(env, &mut rec, spec, e);
    }
    seal_now(env, &slot, live)?;
    after_seal_move_in(env, spec, &mut rec, &slot, live, depth)
}

fn after_seal_move_in(
    env: &OpEnv<'_>,
    spec: &OpSpec,
    rec: &mut OpRecord,
    slot: &Slot,
    live: &Path,
    depth: u8,
) -> Result<OpDone, RepairError> {
    flush_parents(live, slot)?;
    let facts = finish_seal_hooked(env, slot, live)?;
    wal::set_step(env.root, rec, "sealed", env.now)?;
    if env.faults.at(Point::AfterSeal, live, Some(slot)) {
        return Err(stop(Point::AfterSeal));
    }
    let expected = parse_canon(&rec.expected).unwrap_or(TargetState::Absent);
    let gen_ok = Bound::of(rec.pack_generation.clone()).equals_now(env.pack);
    if facts.sealed_state == expected && gen_ok {
        if env.faults.at(Point::BeforeDone, live, Some(slot)) {
            return Err(stop(Point::BeforeDone));
        }
        wal::finish(env.root, rec, "applied", "", env.now)?;
        return Ok(done(ApplyOutcome::Applied, Some(rec)));
    }
    // OQ-D7 for a move-into-vault operation: a no-replace placement of a COPY at the now-absent path
    let mut note = format!(
        "{}: 옮겨 넣은 것이 기대한 바이트가 아닙니다({} ≠ {})",
        spec.rel,
        facts.sealed_state.canon(),
        expected.canon()
    );
    if !gen_ok {
        note.push_str(" · 팩 세대가 바뀌었습니다");
    }
    if facts.sealed_state != expected && depth == 0 {
        let holder = if std::fs::symlink_metadata(slot.sealed()).is_ok() {
            slot.sealed()
        } else {
            slot.payload()
        };
        if (env.own_check)(&spec.rel, &holder) == Some(false) {
            note.push_str(" · 그 파일 자신의 검사를 통과하지 못해 되살리지 않았습니다");
        } else if TargetState::read(live)? == TargetState::Absent {
            let relive = OpSpec {
                id: spec.id.clone(),
                rel: spec.rel.clone(),
                kind: "undo-superseded".into(),
                expected: TargetState::Absent,
                incoming: facts.sealed_state.clone(),
                source: Some(holder),
                bound: Bound::of(rec.pack_generation.clone()),
            };
            let r = run_op_depth(env, &relive, depth + 1)?;
            if r.outcome == ApplyOutcome::Applied {
                note.push_str(" · 사본을 그 자리에 다시 놓았습니다");
            } else {
                note.push_str(&format!(" · 사본을 그 자리에 다시 놓지 못했습니다({:?})", r.outcome));
            }
        } else {
            note.push_str(" · 그 자리에 이미 무엇인가 있어 더 바꾸지 않았습니다");
        }
    }
    wal::finish(env.root, rec, "superseded", &note, env.now)?;
    Ok(done(ApplyOutcome::Superseded(note), Some(rec)))
}

/// CT3 for a placement (place form, the Windows second move): what is NOW live must be the state
/// the placement was to put there; otherwise that entry goes into a fresh slot by a CHECKED
/// operation (expected = what is actually live) and the PREVIOUS state returns — absent for the
/// place form, the deposited bytes (`previous`) for the deposit-first form. `None` = the measured
/// state is the incoming one.
fn verify_placed(
    env: &OpEnv<'_>,
    rec: &OpRecord,
    live: &Path,
    previous: (TargetState, Option<PathBuf>),
    depth: u8,
) -> Result<Option<String>, RepairError> {
    let incoming = parse_canon(&rec.incoming).unwrap_or(TargetState::Absent);
    let live_now = TargetState::read(live)?;
    if live_now == incoming {
        return Ok(None);
    }
    let mut note = format!(
        "{}: 놓은 뒤 살아 있는 것이 들일 상태가 아닙니다(지금 {} ≠ 들일 {})",
        rec.rel,
        live_now.canon(),
        incoming.canon()
    );
    if depth == 0 && live_now != previous.0 {
        let back = OpSpec {
            id: rec.repair.clone(),
            rel: rec.rel.clone(),
            kind: "undo-mismatch".into(),
            expected: live_now,
            incoming: previous.0,
            source: previous.1,
            bound: Bound::of(rec.pack_generation.clone()),
        };
        let r = run_op_depth(env, &back, depth + 1)?;
        if r.outcome == ApplyOutcome::Applied {
            note.push_str(" — 그것을 새 보관 칸에 두고 그 자리를 이전 상태로 되돌렸습니다");
        } else {
            note.push_str(&format!(" — 그 자리를 되돌리지 못했습니다(충돌 · {:?})", r.outcome));
        }
    }
    Ok(Some(note))
}

fn op_place(
    env: &OpEnv<'_>,
    spec: &OpSpec,
    op: &str,
    live: &Path,
    gen: &PackGeneration,
) -> Result<OpDone, RepairError> {
    let copy = incoming_copy(env, &spec.id, op, &spec.incoming, spec.source.as_deref())?;
    let drop_copy = |c: &Path| {
        let _ = std::fs::remove_file(c); // our own non-slot copy
    };
    if TargetState::read(&copy)? != spec.incoming {
        drop_copy(&copy);
        return Ok(done(
            ApplyOutcome::Declined(format!("{}: 놓을 사본이 들일 상태와 다릅니다", spec.rel)),
            None,
        ));
    }
    let copy_identity = match identity_of(&copy) {
        Ok(i) => i,
        Err(e) => {
            drop_copy(&copy);
            return Ok(done(ApplyOutcome::Declined(format!("{}: {e:?}", spec.rel)), None));
        }
    };
    let mut rec = new_record(env, spec, op, "place", gen);
    rec.incoming_copy = Some(copy.clone());
    rec.incoming_copy_identity = Some(copy_identity);
    if let Err(e) = wal::write_in(env.root, &rec) {
        drop_copy(&copy);
        return Ok(done(
            ApplyOutcome::Declined(format!(
                "{}: 동작 기록을 쓸 수 없어 적용하지 않습니다 — {e:?}",
                spec.rel
            )),
            None,
        ));
    }
    if env.faults.at(Point::AfterRecord, live, None) {
        return Err(stop(Point::AfterRecord));
    }
    match door(env, &spec.bound, live, DoorAct::Place { copy: &copy }) {
        Ok(()) => {}
        Err(RepairError::Superseded(m)) => {
            drop_copy(&copy);
            wal::finish(env.root, &mut rec, "superseded", &m, env.now)?;
            return Ok(done(ApplyOutcome::Superseded(m), Some(&rec)));
        }
        Err(e) => {
            drop_copy(&copy);
            wal::finish(env.root, &mut rec, "declined", &format!("{e:?}"), env.now)?;
            return Ok(done(ApplyOutcome::Declined(format!("{}: {e:?}", spec.rel)), Some(&rec)));
        }
    }
    if let Some(p) = live.parent() {
        super::durable::full_flush_dir(p)?;
    }
    wal::set_step(env.root, &mut rec, "placed", env.now)?;
    if env.faults.at(Point::BeforeDone, live, None) {
        return Err(stop(Point::BeforeDone));
    }
    if let Some(note) = verify_placed(env, &rec, live, (TargetState::Absent, None), 0)? {
        wal::finish(env.root, &mut rec, "declined", &note, env.now)?;
        return Ok(done(ApplyOutcome::Declined(note), Some(&rec)));
    }
    wal::finish(env.root, &mut rec, "applied", "", env.now)?;
    Ok(done(ApplyOutcome::Applied, Some(&rec)))
}

/// The Windows form (D:836; also run on macOS by the tests): the live file MOVED into a fresh
/// slot first (and sealed by a byte copy), then the new content placed without replace.
fn op_deposit_first(
    env: &OpEnv<'_>,
    spec: &OpSpec,
    op: &str,
    live: &Path,
    gen: &PackGeneration,
) -> Result<OpDone, RepairError> {
    let slot = vault::build_slot(
        env.root,
        &spec.id,
        env.lane,
        &spec.rel,
        op,
        DepositKind::MovedIn,
        &spec.expected,
        &Staged::Empty,
        env.now,
    )?;
    if env.faults.at(Point::AfterSlot, live, Some(&slot)) {
        return Err(stop(Point::AfterSlot));
    }
    let copy = incoming_copy(env, &spec.id, op, &spec.incoming, spec.source.as_deref())?;
    let (live_identity, copy_identity) = match (identity_of(live), identity_of(&copy)) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(e), _) | (_, Err(e)) => {
            let _ = std::fs::remove_file(&copy);
            return Ok(done(ApplyOutcome::Superseded(format!("{}: {e:?}", spec.rel)), None));
        }
    };
    let mut rec = new_record(env, spec, op, "deposit-first", gen);
    rec.slot = Some(slot.dir.clone());
    rec.live_identity = Some(live_identity);
    rec.incoming_copy = Some(copy.clone());
    rec.incoming_copy_identity = Some(copy_identity);
    if let Err(e) = wal::write_in(env.root, &rec) {
        let _ = std::fs::remove_file(&copy);
        return Ok(done(
            ApplyOutcome::Declined(format!(
                "{}: 동작 기록을 쓸 수 없어 적용하지 않습니다 — {e:?}",
                spec.rel
            )),
            None,
        ));
    }
    if env.faults.at(Point::AfterRecord, live, Some(&slot)) {
        return Err(stop(Point::AfterRecord));
    }
    // first move: the live file into the slot (a held file fails here → stop, delete nothing)
    if let Err(e) = door(env, &spec.bound, live, DoorAct::MoveOut { into: &slot.payload() }) {
        let _ = std::fs::remove_file(&copy);
        return door_refused(env, &mut rec, spec, e);
    }
    seal_now(env, &slot, live)?;
    flush_parents(live, &slot)?;
    finish_seal_hooked(env, &slot, live)?;
    wal::set_step(env.root, &mut rec, "moved", env.now)?;
    if env.faults.at(Point::AfterFirstMove, live, Some(&slot)) {
        return Err(stop(Point::AfterFirstMove));
    }
    second_move(env, &mut rec, &slot, live)
}

/// The second move of the Windows form, and OQ-D8's resume / abandon decision.
fn second_move(
    env: &OpEnv<'_>,
    rec: &mut OpRecord,
    slot: &Slot,
    live: &Path,
) -> Result<OpDone, RepairError> {
    let rel = rec.rel.clone();
    let deposit = vault::deposit_of(slot).map(|(s, _)| s);
    let expected = parse_canon(&rec.expected).unwrap_or(TargetState::Absent);
    let gen_ok = Bound::of(rec.pack_generation.clone()).equals_now(env.pack);
    let copy_ok = match (&rec.incoming_copy, &rec.incoming_copy_identity) {
        (Some(c), Some(want)) => identity_of(c).ok().as_ref() == Some(want),
        _ => false,
    };
    let deposit_ok = deposit.as_ref().ok() == Some(&expected);
    if copy_ok && gen_ok && deposit_ok {
        // RESUME (OQ-D8): the staged copy still matches and the generation is unchanged
        let copy = rec.incoming_copy.clone().unwrap_or_default();
        match door(env, &Bound::of(rec.pack_generation.clone()), live, DoorAct::Place { copy: &copy }) {
            Ok(()) => {
                if let Some(p) = live.parent() {
                    super::durable::full_flush_dir(p)?;
                }
                wal::set_step(env.root, rec, "placed", env.now)?;
                if env.faults.at(Point::BeforeDone, live, Some(slot)) {
                    return Err(stop(Point::BeforeDone));
                }
                let previous = vault::deposit_of(slot).map(|(st, h)| (st, Some(h))).unwrap_or((TargetState::Absent, None));
                if let Some(note) = verify_placed(env, rec, live, previous, 0)? {
                    wal::finish(env.root, rec, "declined", &note, env.now)?;
                    return Ok(done(ApplyOutcome::Declined(note), Some(rec)));
                }
                wal::finish(env.root, rec, "applied", "", env.now)?;
                return Ok(done(ApplyOutcome::Applied, Some(rec)));
            }
            Err(RepairError::Superseded(m)) => {
                let _ = std::fs::remove_file(&copy);
                let note = format!("{rel}: 두 번째 옮기기 자리를 다른 것이 차지했습니다 — 멈춤, 아무것도 지우지 않음({m})");
                wal::finish(env.root, rec, "superseded", &note, env.now)?;
                return Ok(done(ApplyOutcome::Superseded(note), Some(rec)));
            }
            Err(e) => {
                // stays at `moved`: the next run resumes or abandons
                return Ok(done(
                    ApplyOutcome::Declined(format!("{rel}: 두 번째 옮기기 실패 — 다음 실행이 마저 하거나 그만둡니다 ({e:?})")),
                    Some(rec),
                ));
            }
        }
    }
    // ABANDON (OQ-D8): a COPY of the deposited bytes back with a no-replace move
    if let Some(c) = &rec.incoming_copy {
        let _ = std::fs::remove_file(c);
    }
    // CT1: the sealed deposit, else the moved-in payload itself when its hash is the one the record
    // took of the live entry before the move (never a copy whose bytes nothing vouches for)
    let deposit = vault::deposit_of(slot).or_else(|e| {
        let want = rec.live_identity.as_ref().map(|i| i.sha256.clone());
        match TargetState::read(&slot.payload()) {
            Ok(st @ TargetState::File { .. }) if want.is_some() && matches!(&st, TargetState::File { sha256, .. } if Some(sha256) == want.as_ref()) => {
                Ok((st, slot.payload()))
            }
            _ => Err(e),
        }
    });
    let note = match deposit {
        Ok((st, holder)) => {
            let back = OpSpec {
                id: rec.repair.clone(),
                rel: rel.clone(),
                kind: "undo-superseded".into(),
                expected: TargetState::Absent,
                incoming: st,
                source: Some(holder),
                bound: Bound::of(rec.pack_generation.clone()),
            };
            match run_op_depth(env, &back, 1) {
                Ok(r) if r.outcome == ApplyOutcome::Applied => format!("{rel}: 끊긴 동작을 그만두고 보관한 바이트의 사본을 되놓았습니다"),
                Ok(r) => format!("{rel}: 끊긴 동작을 그만두었으나 사본을 되놓지 못했습니다 — 아무것도 지우지 않음({:?})", r.outcome),
                Err(e) => format!("{rel}: 끊긴 동작을 그만두었으나 사본을 되놓지 못했습니다 — 아무것도 지우지 않음({e:?})"),
            }
        }
        Err(e) => format!("{rel}: 보관한 바이트를 확인할 수 없어 그만둡니다 — 아무것도 지우지 않음({e:?})"),
    };
    wal::finish(env.root, rec, "abandoned", &note, env.now)?;
    Ok(done(ApplyOutcome::Superseded(note), Some(rec)))
}

/// Finishes or abandons ONE interrupted operation from its record (design step 5; C1 1.13;
/// OQ-D8). A follow-up that completes an earlier exchange requires the live entry to match the
/// recorded identity (volume UUID + inode) — content alone is not enough (D:829).
pub fn resume_op(env: &OpEnv<'_>, mut rec: OpRecord) -> Result<OpDone, RepairError> {
    if rec.finished {
        return Ok(done(ApplyOutcome::Applied, Some(&rec)));
    }
    let live = env.pack.join(&rec.rel);
    let spec = OpSpec {
        id: rec.repair.clone(),
        rel: rec.rel.clone(),
        kind: rec.kind.clone(),
        expected: parse_canon(&rec.expected).unwrap_or(TargetState::Absent),
        incoming: parse_canon(&rec.incoming).unwrap_or(TargetState::Absent),
        source: None,
        bound: Bound::of(rec.pack_generation.clone()),
    };
    let slot = rec.slot.as_ref().map(|d| Slot::at(d, &rec.rel));
    let live_entry = entry_identity(&live).ok();
    let abandon = |rec: &mut OpRecord, why: &str| -> Result<OpDone, RepairError> {
        wal::finish(env.root, rec, "abandoned", why, env.now)?;
        Ok(done(ApplyOutcome::Declined(why.to_string()), Some(rec)))
    };
    let supersede = |rec: &mut OpRecord, why: &str| -> Result<OpDone, RepairError> {
        wal::finish(env.root, rec, "superseded", why, env.now)?;
        Ok(done(ApplyOutcome::Superseded(why.to_string()), Some(rec)))
    };
    match (rec.form.as_str(), rec.step.as_str()) {
        ("exchange", "staged") => {
            let (Some(slot), Some(staged)) = (slot.clone(), rec.slot_identity.clone()) else {
                return supersede(&mut rec, "기록이 불완전합니다(칸·정체 없음) — 워커 검토");
            };
            let payload_entry = entry_identity(&slot.payload()).ok();
            if live_entry.as_ref().is_some_and(|e| same_entry(&staged, e)) {
                // the exchange happened: seal what is found (M2 recovery; CT1 — an unfinished
                // temporary seal is discarded, a published one must agree with its record), then step 4
                match seal_hooked(env, &slot, &live) {
                    Err(RepairError::Superseded(m)) => {
                        return supersede(&mut rec, &format!("봉인 사본이 기록과 맞지 않습니다 — 더 바꾸지 않고 워커 검토({m})"))
                    }
                    r => r?,
                }
                match after_seal_exchange(env, &spec, &mut rec, &slot, &live, 0) {
                    Err(RepairError::Superseded(m)) => supersede(&mut rec, &format!("봉인 기록이 맞지 않습니다 — 더 바꾸지 않고 워커 검토({m})")),
                    r => r,
                }
            } else if payload_entry.as_ref().is_some_and(|e| same_entry(&staged, e)) {
                abandon(&mut rec, "맞바꾸기 전에 끊긴 동작 — 살아 있는 파일은 바뀌지 않았습니다(그만둠)")
            } else {
                supersede(&mut rec, "끊긴 동작의 정체가 맞지 않습니다(다른 작성자) — 더 바꾸지 않고 워커 검토")
            }
        }
        ("exchange", "sealed") => {
            let Some(slot) = slot else {
                return supersede(&mut rec, "기록이 불완전합니다(칸 없음) — 워커 검토");
            };
            let facts = match finish_seal_hooked(env, &slot, &live) {
                Err(RepairError::Superseded(m)) => {
                    return supersede(&mut rec, &format!("봉인 기록이 맞지 않습니다 — 더 바꾸지 않고 워커 검토({m})"))
                }
                r => r?,
            };
            step4(env, &spec, &mut rec, &slot, &live, &facts.sealed_state, 0)
        }
        ("move-in", "staged") | ("deposit-first", "staged") => {
            let (Some(slot), Some(was)) = (slot.clone(), rec.live_identity.clone()) else {
                return supersede(&mut rec, "기록이 불완전합니다 — 워커 검토");
            };
            let payload_entry = entry_identity(&slot.payload()).ok();
            if payload_entry.as_ref().is_some_and(|e| same_entry(&was, e)) {
                // CT1: an unfinished temporary seal is discarded; a published seal or record that
                // disagrees is never blessed — the deposit-first form then abandons (OQ-D8) and puts
                // back a copy only of bytes the record vouches for
                let sealed_ok = match seal_hooked(env, &slot, &live) {
                    Err(RepairError::Superseded(_)) => false,
                    r => {
                        r?;
                        true
                    }
                };
                if rec.form == "move-in" {
                    if !sealed_ok {
                        return supersede(&mut rec, "봉인 사본이 기록과 맞지 않습니다 — 더 바꾸지 않고 워커 검토");
                    }
                    match after_seal_move_in(env, &spec, &mut rec, &slot, &live, 0) {
                        Err(RepairError::Superseded(m)) => supersede(&mut rec, &format!("봉인 기록이 맞지 않습니다 — 더 바꾸지 않고 워커 검토({m})")),
                        r => r,
                    }
                } else {
                    flush_parents(&live, &slot)?;
                    let meta_ok = sealed_ok
                        && match finish_seal_hooked(env, &slot, &live) {
                            Err(RepairError::Superseded(_)) => false,
                            r => {
                                r?;
                                true
                            }
                        };
                    if !meta_ok {
                        // the copy is never placed (its deposit is not vouched for): abandon
                        if let Some(c) = rec.incoming_copy.take() {
                            let _ = std::fs::remove_file(c); // our own non-slot incoming copy
                        }
                        rec.incoming_copy_identity = None;
                    }
                    wal::set_step(env.root, &mut rec, "moved", env.now)?;
                    second_move(env, &mut rec, &slot, &live)
                }
            } else if live_entry.as_ref().is_some_and(|e| same_entry(&was, e)) {
                if let Some(c) = &rec.incoming_copy {
                    let _ = std::fs::remove_file(c);
                }
                abandon(&mut rec, "옮기기 전에 끊긴 동작 — 살아 있는 파일은 바뀌지 않았습니다(그만둠)")
            } else {
                supersede(&mut rec, "끊긴 동작의 정체가 맞지 않습니다 — 더 바꾸지 않고 워커 검토")
            }
        }
        ("move-in", "sealed") => {
            let Some(slot) = slot else {
                return supersede(&mut rec, "기록이 불완전합니다 — 워커 검토");
            };
            after_seal_move_in(env, &spec, &mut rec, &slot, &live, 0)
        }
        ("deposit-first", "moved") => {
            let Some(slot) = slot else {
                return supersede(&mut rec, "기록이 불완전합니다 — 워커 검토");
            };
            second_move(env, &mut rec, &slot, &live)
        }
        ("place", "staged") | ("deposit-first", "placed") | ("place", "placed") => {
            let want = rec.incoming_copy_identity.clone();
            if want.as_ref().is_some_and(|w| live_entry.as_ref().is_some_and(|e| same_entry(w, e))) {
                let previous = match (&rec.form[..], &slot) {
                    ("deposit-first", Some(sl)) => vault::deposit_of(sl).map(|(st, h)| (st, Some(h))).unwrap_or((TargetState::Absent, None)),
                    _ => (TargetState::Absent, None),
                };
                if let Some(note) = verify_placed(env, &rec, &live, previous, 0)? {
                    wal::finish(env.root, &mut rec, "declined", &note, env.now)?;
                    return Ok(done(ApplyOutcome::Declined(note), Some(&rec)));
                }
                wal::finish(env.root, &mut rec, "applied", "끊긴 뒤 놓기가 끝나 있음을 확인", env.now)?;
                return Ok(done(ApplyOutcome::Applied, Some(&rec)));
            }
            if let Some(c) = &rec.incoming_copy {
                let _ = std::fs::remove_file(c);
            }
            if rec.form == "place" && live_entry.is_none() {
                abandon(&mut rec, "놓기 전에 끊긴 동작 — 그 자리는 그대로 비어 있습니다(그만둠)")
            } else {
                supersede(&mut rec, "끊긴 놓기의 자리에 다른 것이 있습니다 — 더 바꾸지 않고 워커 검토")
            }
        }
        _ => supersede(&mut rec, "알 수 없는 단계의 끊긴 동작 — 더 바꾸지 않고 워커 검토"),
    }
}

/// The lane's interrupted operations finished or abandoned from their records, in record order.
pub fn resume_interrupted_in(env: &OpEnv<'_>) -> Result<u32, RepairError> {
    resume_interrupted_report_in(env).map(|(n, _)| n)
}

/// [`resume_interrupted_in`] that also returns each operation that could not be finished this time
/// (repair, why) — recorded by the job, never swallowed (RULINGS DR5).
pub fn resume_interrupted_report_in(env: &OpEnv<'_>) -> Result<(u32, Vec<(RepairId, String)>), RepairError> {
    let mut errors = Vec::new();
    let mut n = 0u32;
    // EJ4: per repair — its guard TRIED first (held → left alone this tick), its records read only
    // AFTER the guard is held (never a list snapshotted before). FG1: a failed listing is an error.
    for id in ledger::list_ids_checked_in(env.root)? {
        let Some(_guard) = super::guard::RepairGuard::try_take(env.root, &id) else {
            continue;
        };
        let ops = match wal::list_checked_in(env.root, &id) {
            Ok(ops) => ops,
            Err(e) => {
                errors.push((id.clone(), format!("동작 기록을 읽을 수 없습니다({e})")));
                continue;
            }
        };
        for rec in ops.into_iter().filter(|r| r.lane_key == *env.lane && !r.finished && r.form != "hook") {
        let (repair, op_id) = (rec.repair.clone(), rec.op.clone());
        match resume_op(env, rec) {
            Ok(_) => n += 1,
            Err(e) if format!("{e:?}").contains(INJECTED_STOP) => return Err(e),
            // left for the next run (nothing is forced) — and reported
            Err(e) => errors.push((repair, format!("끊긴 동작 {op_id} 을 마저 하지 못했습니다({e:?})"))),
        }
        }
    }
    Ok((n, errors))
}

/// Finishes or abandons interrupted operations of a lane; returns how many.
pub fn resume_interrupted(lane: &LaneKey) -> Result<u32, RepairError> {
    let root = super::paths::repairs_root();
    let pack = crate::pack::pack_dir();
    let env = OpEnv {
        root: &root,
        pack: &pack,
        lane,
        now: ledger::now_epoch(),
        form: super::platform::apply_form(),
        faults: &NoFaults,
        own_check: &|_, _| None,
    };
    resume_interrupted_in(&env)
}

// ───────────────────────────── checks (the runner is T5b's) ─────────────────────────────

/// One check's result with its wall time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckOutcome {
    pub result: CheckResult,
    pub ms: u64,
}

/// The repair's check of one target: `(rel, root holding <rel>, deadline)`. Production: T5b's
/// `rehearsal::run_check` (the child is killed and reaped at the deadline — RULINGS F3 SA:63).
pub type CheckFn<'a> =
    dyn Fn(&str, &Path, Duration) -> Result<CheckOutcome, RepairError> + 'a;
/// The built-in reference check of a kind (RULINGS H2 / K4 / O3).
pub type ReferenceFn<'a> = dyn Fn(CheckKind, Duration) -> Result<CheckOutcome, RepairError> + 'a;

/// Production check runner (T5b's body).
pub fn production_check(rel: &str, root: &Path, deadline: Duration) -> Result<CheckOutcome, RepairError> {
    let t = std::time::Instant::now();
    let result = super::rehearsal::run_check(&super::rehearsal::CheckRequest {
        entry_rel: rel.to_string(),
        scratch_root: root.to_path_buf(),
        recorded_input: None,
        deadline,
    })?;
    Ok(CheckOutcome {
        result,
        ms: t.elapsed().as_millis() as u64,
    })
}

/// Production reference check (T5b's body).
pub fn production_reference(kind: CheckKind, deadline: Duration) -> Result<CheckOutcome, RepairError> {
    let t = std::time::Instant::now();
    let result = super::rehearsal::run_reference_check(kind, deadline)?;
    Ok(CheckOutcome {
        result,
        ms: t.elapsed().as_millis() as u64,
    })
}

/// The check kind of a target: its compiled row's, else by file type (as T3's `check_kind_for`).
pub fn check_kind_of(rel: &str) -> CheckKind {
    if let Some(e) = super::boot_critical::entries().iter().find(|e| e.rel == rel) {
        return e.check;
    }
    if let Some(e) = super::boot_critical::protection_entry(rel) {
        return e.check;
    }
    match rel.rsplit('.').next() {
        Some("sh") => CheckKind::ShSyntax,
        Some("py") => CheckKind::PyCompile,
        Some("json") => CheckKind::JsonParse,
        _ => CheckKind::NonePrerehearsal,
    }
}

/// The deadline of a target's check (RULINGS B3): the hook's registered timeout clamped 10–60 s,
/// else 60 s.
pub fn check_deadline(rel: &str) -> Duration {
    let declared = super::boot_critical::entries()
        .iter()
        .find(|e| e.rel == rel)
        .and_then(|e| e.timeout_s);
    Duration::from_secs(u64::from(super::boot_critical::clamp_check_timeout(declared)))
}

// ───────────────────────────── the per-repair lock and the record writes ─────────────────────────────

/// The daemon's per-repair mutex (RULINGS G6, R3) as a callback: runs the closure under the lock
/// of that repair. The library's global entry points use [`no_repair_lock`].
pub type RepairLock<'a> =
    dyn Fn(&RepairId, &mut dyn FnMut() -> Result<(), RepairError>) -> Result<(), RepairError> + 'a;

/// No in-memory lock (a caller outside the daemon).
pub fn no_repair_lock(
    _id: &RepairId,
    f: &mut dyn FnMut() -> Result<(), RepairError>,
) -> Result<(), RepairError> {
    f()
}

/// The hook step as a callback (`hook_lines_step` in production).
pub type HookStepFn<'a> =
    dyn Fn(&RepairId, &LaneKey, HookStep) -> Result<ReconcileReport, RepairError> + 'a;

fn is_approval_class(c: Classification) -> bool {
    matches!(c, Classification::BootCritical | Classification::ProtectionRelated)
}

/// The latest finished, applied apply/propagation operation of `lane` for `rel` in repair `id` —
/// its slot holds what that apply displaced (the lane's pre-image for a revert).
fn applied_op_for(root: &Path, id: &RepairId, lane: &LaneKey, rel: &str) -> Option<OpRecord> {
    let (ops, _) = wal::list_in(root, id);
    ops.into_iter()
        .filter(|r| {
            r.lane_key == *lane
                && r.rel == rel
                && r.finished
                && r.outcome.as_deref() == Some("applied")
                && matches!(r.kind.as_str(), "apply" | "propagate")
        })
        .last()
}

/// The bytes (file holding them) and state a revert puts back for `rel`: the deposit of this lane's
/// apply operation (the sealed copy), else (the origin lane of a general repair — the worker edited
/// live) the `begin` pre-image copy. Verified against the recorded hash.
pub fn pre_image_for(
    root: &Path,
    rec: &RepairRecord,
    lane: &LaneKey,
    rel: &str,
) -> Result<(TargetState, Option<PathBuf>), RepairError> {
    if let Some(op) = applied_op_for(root, &rec.id, lane, rel) {
        let want = parse_canon(&op.expected).unwrap_or(TargetState::Absent);
        if want == TargetState::Absent {
            return Ok((TargetState::Absent, None));
        }
        let slot = Slot::at(op.slot.as_deref().unwrap_or(Path::new("")), rel);
        let (st, holder) = vault::deposit_of(&slot)?;
        if st != want {
            return Err(RepairError::Superseded(format!(
                "{rel}: 보관된 적용 전 내용이 기록과 다릅니다"
            )));
        }
        return Ok((st, Some(holder)));
    }
    if rec.origin_lane != *lane {
        return Err(RepairError::Precondition(format!(
            "{rel}: 이 부서의 적용 기록이 없어 되돌릴 내용을 알 수 없습니다"
        )));
    }
    let f = ledger::file_targets(rec)
        .find(|f| f.rel == rel)
        .ok_or_else(|| RepairError::Precondition(format!("{rel}: 수리 기록에 없는 대상입니다")))?;
    match &f.pre {
        PreImage::Absent => Ok((TargetState::Absent, None)),
        PreImage::Slot { slot_dir, .. } => {
            let slot = Slot::at(slot_dir, rel);
            let (st, holder) = vault::deposit_of(&slot)?;
            if st != f.pre.state() {
                return Err(RepairError::Superseded(format!(
                    "{rel}: 보관된 적용 전 내용이 기록과 다릅니다"
                )));
            }
            Ok((st, Some(holder)))
        }
    }
}

/// RULINGS EN1 — the closures that ARE the duty to file an intake row: written by the closing code,
/// re-derived from these constants at every tick (never a parse of free text).
pub const CLOSED_FAILED: &str = "실패";
pub const CLOSED_REVERTED: &str = "되돌려짐";
pub const BY_DAEMON_AUTO: &str = "데몬(자동)";
/// RULINGS EK2 / EN7 — a failed re-check whose automatic return was deferred (the pack-apply lock
/// held) is closed `실패 — 되돌리기 남음 · …` IN THAT TICK: the failure is a fact of the row from the
/// moment it is measured; the return stays owed and is retried at later ticks (never 사용 중).
pub const RETURN_OWED: &str = "되돌리기 남음";

/// Whether a lane row's closure still owes the automatic return (EK2).
pub fn closure_owes_return(closed: &str) -> bool {
    closed.starts_with(&format!("{CLOSED_FAILED} — {RETURN_OWED}"))
}

/// Whether a lane row's closure is one that owes an intake row (EN1): the probation closed FAILED,
/// or closed by the daemon's automatic return.
pub fn closure_owes_intake(closed: &str) -> bool {
    closed.starts_with(&format!("{CLOSED_FAILED} — ")) || closed == format!("{CLOSED_REVERTED} — {BY_DAEMON_AUTO}")
}

/// DX1 — THE definition of "revertible in this lane" (`revert_in` accepts exactly these; the app's
/// "직전 수리 되돌리기" picks among them): the lane row still applies something (a file state or a
/// hook line), it is not closed 되돌려짐 / 대체됨, and — in the origin lane — the record is in force or
/// the row was left 충돌 by an apply / revert that could not finish (a probation closed 실패 included:
/// its record stays in force); in another lane, its row is open or closed otherwise.
pub fn revertible_here(rec: &RepairRecord, row: &ledger::LaneRecord, lane: &LaneKey) -> bool {
    let has_hooks = ledger::hook_lines(rec).next().is_some();
    let applies = !row.applied.is_empty() || has_hooks;
    let closed_for_good = matches!(row.probation.as_ref().and_then(|p| p.closed.as_deref()), Some(c) if c.starts_with("되돌려짐") || c.starts_with("대체됨"));
    if !applies || closed_for_good {
        return false;
    }
    if rec.origin_lane == *lane {
        matches!(rec.state, RepairState::Probation | RepairState::InUse) || row.state == LaneState::Conflict
    } else {
        true
    }
}

/// One target of a revert intent (canon strings).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RevertTarget {
    pub rel: String,
    pub applied: String,
    pub pre: String,
}

/// RULINGS DR7 — the repair-level intent of one revert in one lane, durable BEFORE the first
/// restoring exchange (`<root>/R-<n>/intents/<lane>.revert.json`): `reverting` → `reverted` |
/// `incomplete` | `superseded`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RevertIntent {
    pub schema: u32,
    pub repair: RepairId,
    pub lane_key: LaneKey,
    pub op_tag: String,
    pub state: String,
    pub targets: Vec<RevertTarget>,
    pub has_hooks: bool,
    pub pack_generation: Option<PackGeneration>,
    pub by: String,
    pub created_at: f64,
    pub updated_at: f64,
}

pub fn revert_intent_path_in(root: &Path, id: &RepairId, lane: &LaneKey) -> PathBuf {
    ledger::repair_dir_in(root, id).join("intents").join(format!("{}.revert.json", lane.0))
}

/// FG1: the revert intent with three answers (as [`read_intent_checked_in`]).
pub fn read_revert_intent_checked_in(root: &Path, id: &RepairId, lane: &LaneKey) -> Result<Option<RevertIntent>, String> {
    read_checked(&revert_intent_path_in(root, id, lane), &ledger::repair_dir_in(root, id))
}

pub fn read_revert_intent_in(root: &Path, id: &RepairId, lane: &LaneKey) -> Option<RevertIntent> {
    serde_json::from_slice(&std::fs::read(revert_intent_path_in(root, id, lane)).ok()?).ok()
}

fn write_revert_intent(root: &Path, ri: &RevertIntent) -> Result<(), RepairError> {
    super::durable::write_json_durable(&revert_intent_path_in(root, &ri.repair, &ri.lane_key), ri)
}

/// DR7 recovery: every `reverting` intent of THIS lane no live operation owns is completed AS A
/// WHOLE — files already restored, the remaining files, the hook step, the lane row and the record —
/// bound to its generation (a different generation → nothing put back, closed superseded).
pub fn recover_revert_intents_in(env: &OpEnv<'_>, hooks: &HookStepFn<'_>, locked: &RepairLock<'_>) -> Vec<String> {
    let mut lines = Vec::new();
    let ids = match ledger::list_ids_checked_in(env.root) {
        Ok(ids) => ids,
        Err(e) => return vec![format!("수리 장부 목록을 읽을 수 없어 끊긴 되돌리기를 찾지 못했습니다 — 다음 차례에 다시({e:?})")],
    };
    for id in ids {
        // EJ4: a revert running now holds the repair's guard; its intent is read AFTER holding it
        let Some(_guard) = super::guard::RepairGuard::try_take(env.root, &id) else {
            continue;
        };
        let mut ri = match read_revert_intent_checked_in(env.root, &id, env.lane) {
            Ok(Some(ri)) => ri,
            Ok(None) => continue,
            Err(e) => {
                lines.push(format!("{}: 되돌리기 의도를 읽을 수 없습니다 — 다음 차례에 다시({e})", id.as_str()));
                continue;
            }
        };
        if ri.state != "reverting" {
            continue;
        }
        let rec = match ledger::read_record_in(env.root, &id) {
            Ok(Some(r)) => r,
            Ok(None) => continue,
            Err(e) => {
                lines.push(format!("{}: 끊긴 되돌리기를 마치지 못했습니다(기록을 읽을 수 없음 — {e:?})", id.as_str()));
                continue;
            }
        };
        match recover_one_revert(env, &rec, &mut ri, hooks, locked) {
            // EJ6: the sentence follows the measured outcome
            Ok(r) => lines.push(match &r.outcome {
                ApplyOutcome::Applied => format!("{}: 끊긴 되돌리기를 마저 했습니다(완료)", id.as_str()),
                ApplyOutcome::Superseded(m) => format!("{}: 끊긴 되돌리기는 대체됨 — 아무것도 더 되돌리지 않았습니다({m})", id.as_str()),
                ApplyOutcome::Declined(m) => format!("{}: 끊긴 되돌리기를 다 마치지 못했습니다 — 남은 것: {m}", id.as_str()),
            }),
            Err(e) => lines.push(format!("{}: 끊긴 되돌리기를 마치지 못했습니다 — 다음 차례에 다시({e:?})", id.as_str())),
        }
    }
    lines
}

fn recover_one_revert(
    env: &OpEnv<'_>,
    rec: &RepairRecord,
    ri: &mut RevertIntent,
    hooks: &HookStepFn<'_>,
    locked: &RepairLock<'_>,
) -> Result<RevertReport, RepairError> {
    {
        if !Bound::of(ri.pack_generation.clone()).equals_now(env.pack) {
            let r = close_superseded(env, rec, "되돌리기 중에 끊긴 뒤 팩 세대가 바뀌었습니다 — 아무것도 더 되돌리지 않았습니다(새 버전 우선 · 워커 검토)", locked)?;
            ri.state = "superseded".into();
            ri.updated_at = env.now;
            write_revert_intent(env.root, ri)?;
            return Ok(r);
        }
    }
    let mut files = Vec::new();
    for t in &ri.targets {
        let pre = parse_canon(&t.pre).ok_or_else(|| RepairError::Corrupt(format!("{}: 되돌리기 의도의 상태를 읽을 수 없습니다", t.rel)))?;
        let applied = parse_canon(&t.applied).ok_or_else(|| RepairError::Corrupt(format!("{}: 되돌리기 의도의 상태를 읽을 수 없습니다", t.rel)))?;
        let live_now = TargetState::read(&env.pack.join(&t.rel))?;
        let outcome = if live_now == pre {
            ApplyOutcome::Applied // restored before the crash
        } else if live_now == applied {
            let (_, holder) = pre_image_for(env.root, rec, env.lane, &t.rel)?;
            run_op(
                env,
                &OpSpec {
                    id: rec.id.clone(),
                    rel: t.rel.clone(),
                    kind: "revert".into(),
                    expected: applied,
                    incoming: pre,
                    source: holder,
                    bound: Bound::of(ri.pack_generation.clone()),
                },
            )?
            .outcome
        } else {
            ApplyOutcome::Superseded(format!("{}: 살아 있는 것이 적용 결과도 적용 전도 아닙니다 — 그대로 둠", t.rel))
        };
        files.push((t.rel.clone(), outcome));
    }
    let by = ri.by.clone();
    let report = finish_revert(env, rec, ri.targets.len(), files, ri.has_hooks, hooks, &by, locked)?;
    ri.state = match &report.outcome {
        ApplyOutcome::Applied => "reverted",
        ApplyOutcome::Superseded(_) => "superseded",
        ApplyOutcome::Declined(_) => "incomplete",
    }
    .into();
    ri.updated_at = env.now;
    write_revert_intent(env.root, ri)?;
    Ok(report)
}

/// What a revert did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevertReport {
    pub outcome: ApplyOutcome,
    /// (rel, outcome) per file target.
    pub files: Vec<(String, ApplyOutcome)>,
    pub hook_step: Option<Result<(), String>>,
}

/// Reverts ONE repair in this lane by rule one (design §4-9): every target first checked (live =
/// the lane's applied state, the pack generation = the recorded one, same filesystem) — any
/// mismatch closes the revert `superseded` and NOTHING changes; then per target: a fresh slot
/// holding the pre-image exchanged in (edit / mode / link), the created file moved into a slot, a
/// deleted file's pre-image copy placed without replace; this repair's tombstones removed; its
/// hook lines re-registered (Y1); the record (origin lane) → `되돌려짐` and the lane row's
/// probation closed. The CALLER holds the pack-apply lock. Windows: general repairs only, never
/// automatic.
pub fn revert_in(
    env: &OpEnv<'_>,
    id: &RepairId,
    by: RevertCaller,
    hooks: &HookStepFn<'_>,
    locked: &RepairLock<'_>,
) -> Result<RevertReport, RepairError> {
    // EJ4: the repair's ONE guard for the whole revert (tried; held → busy, never waited for)
    let Some(_guard) = super::guard::RepairGuard::try_take(env.root, id) else {
        return Err(RepairError::Locked(format!("{}: {}", id.as_str(), super::guard::BUSY)));
    };
    let rec = ledger::read_record_in(env.root, id)?
        .ok_or_else(|| RepairError::Precondition(format!("{}: 그런 수리가 없습니다", id.as_str())))?;
    let windows_form = env.form == ApplyForm::DepositFirst;
    if by == RevertCaller::DaemonAuto && !auto_revert_allowed(env.form) {
        return Err(RepairError::PlatformPending(
            "이 운영체제에서는 자동 되돌리기가 꺼져 있습니다(0.15.0) — 기록하고 알리기만 합니다",
        ));
    }
    if windows_form && is_approval_class(rec.classification) {
        return Err(RepairError::PlatformPending(
            "윈도우에서는 기동 필수·보호장치 관련 수리를 되돌리지 않습니다(먼저 넣는 꼴은 기동 필수가 아닌 파일만)",
        ));
    }
    let row = ledger::read_lane_in(env.root, id, env.lane)?.ok_or_else(|| {
        RepairError::Precondition(format!("{}: 이 부서에는 이 수리의 행이 없습니다", id.as_str()))
    })?;
    let has_hooks = ledger::hook_lines(&rec).next().is_some();
    // DR7: an interrupted revert of this repair in this lane is completed first (never re-planned
    // against a row an already-restored file no longer matches)
    let pending = read_revert_intent_checked_in(env.root, id, env.lane)
        .map_err(|e| RepairError::Corrupt(format!("{}: 되돌리기 의도를 읽을 수 없어 되돌리지 않습니다 — {e}", id.as_str())))?;
    if let Some(mut ri) = pending {
        if ri.state == "reverting" {
            return recover_one_revert(env, &rec, &mut ri, hooks, locked);
        }
    }
    // DX1 / CT7: ONE definition of "revertible here" (the app's picker asks the same predicate)
    if !revertible_here(&rec, &row, env.lane) {
        return Err(RepairError::Precondition(format!(
            "{}: '{}' 상태라 되돌릴 것이 없습니다(시험 기간·사용 중인 수리만 되돌립니다)",
            id.as_str(),
            rec.state.label_ko()
        )));
    }
    let bound = row.probation.as_ref().map(Bound::from_row).unwrap_or(Bound::Unknown);
    // step 0 for EVERY target before anything changes
    let mut plan: Vec<OpSpec> = Vec::new();
    for (rel, canon) in &row.applied {
        let expected = parse_canon(canon).ok_or_else(|| {
            RepairError::Corrupt(format!("{rel}: 기록된 상태를 읽을 수 없습니다({canon})"))
        })?;
        let live_now = TargetState::read(&env.pack.join(rel))?;
        if live_now != expected {
            return Ok(close_superseded(
                env,
                &rec,
                &format!(
                    "{rel}: 살아 있는 파일이 적용 결과와 다릅니다(업데이트나 다른 작성자) — 아무것도 되돌리지 않았습니다"
                ),
                locked,
            )?);
        }
        if !bound.equals_now(env.pack) {
            return Ok(close_superseded(
                env,
                &rec,
                "팩 세대가 적용 때와 다르거나 알 수 없습니다 — 옛 세대의 바이트는 되살리지 않습니다(아무것도 되돌리지 않았습니다)",
                locked,
            )?);
        }
        let (pre, holder) = pre_image_for(env.root, &rec, env.lane, rel)?;
        plan.push(OpSpec {
            id: id.clone(),
            rel: rel.clone(),
            kind: "revert".into(),
            expected,
            incoming: pre,
            source: holder,
            bound: bound.clone(),
        });
    }
    let by_label = match by {
        RevertCaller::Human => "사람".to_string(),
        RevertCaller::TicketedWorker { seat } => format!("워커 좌석 {seat}"),
        RevertCaller::DaemonAuto => BY_DAEMON_AUTO.to_string(),
    };
    // DR7: the repair-level intent is durable BEFORE the first restoring exchange
    let op_tag = wal::new_op_id(env.now, "revert");
    let mut ri = RevertIntent {
        schema: super::REPAIR_SCHEMA,
        repair: id.clone(),
        lane_key: env.lane.clone(),
        op_tag: op_tag.clone(),
        state: "reverting".into(),
        targets: plan
            .iter()
            .map(|sp| RevertTarget { rel: sp.rel.clone(), applied: sp.expected.canon(), pre: sp.incoming.canon() })
            .collect(),
        has_hooks,
        pack_generation: bound.known(),
        by: by_label.clone(),
        created_at: env.now,
        updated_at: env.now,
    };
    write_revert_intent(env.root, &ri)?;
    let mut files = Vec::new();
    for spec in &plan {
        let r = run_op(env, spec)?;
        files.push((spec.rel.clone(), r.outcome));
    }
    let report = finish_revert(env, &rec, plan.len(), files, has_hooks, hooks, &by_label, locked)?;
    ri.state = match &report.outcome {
        ApplyOutcome::Applied => "reverted",
        ApplyOutcome::Superseded(_) => "superseded",
        ApplyOutcome::Declined(_) => "incomplete",
    }
    .into();
    ri.updated_at = env.now;
    write_revert_intent(env.root, &ri)?;
    Ok(report)
}

/// The end of a revert (also its DR7 recovery): FILE tombstones of restored targets only, the hook
/// step (which removes only hook tombstones, after its record), the measured outcome, the record
/// and the lane row.
#[allow(clippy::too_many_arguments)]
fn finish_revert(
    env: &OpEnv<'_>,
    rec: &RepairRecord,
    planned: usize,
    files: Vec<(String, ApplyOutcome)>,
    has_hooks: bool,
    hooks: &HookStepFn<'_>,
    by_label: &str,
    locked: &RepairLock<'_>,
) -> Result<RevertReport, RepairError> {
    let id = &rec.id;
    let all_applied = files.iter().all(|(_, o)| *o == ApplyOutcome::Applied);
    // this repair's FILE tombstones of the reverted targets; the hook step removes its own hook
    // tombstones AFTER its operation record (Y1's order — CU4)
    for (rel, o) in &files {
        if *o == ApplyOutcome::Applied {
            let _ = super::tombstones::remove_file_rel_in(env.root, env.lane, rel, id);
        }
    }
    let hook_step = if has_hooks {
        Some(hooks(id, env.lane, HookStep::Reregister).map(|_| ()).map_err(|e| format!("{e:?}")))
    } else {
        None
    };
    // CT3 — the outcome reported is the outcome measured: complete only when every file op AND the
    // re-registration succeeded; nothing changed → 대체됨 (as before); anything in between → 충돌
    // (the row keeps the targets not yet put back, so `revert` can finish it)
    let any_applied = files.iter().any(|(_, o)| *o == ApplyOutcome::Applied);
    let hook_failed = matches!(hook_step, Some(Err(_)));
    let complete = all_applied && !hook_failed;
    let nothing_changed = !any_applied && hook_step.is_none() && planned > 0;
    let label = if complete {
        CLOSED_REVERTED
    } else if nothing_changed {
        "대체됨"
    } else {
        "충돌"
    };
    let left_rels: Vec<String> = files
        .iter()
        .filter(|(_, o)| *o != ApplyOutcome::Applied)
        .map(|(r, _)| r.clone())
        .collect();
    let mut note = files
        .iter()
        .filter(|(_, o)| *o != ApplyOutcome::Applied)
        .map(|(r, o)| format!("{r}: {o:?}"))
        .collect::<Vec<_>>()
        .join(" · ");
    if let Some(Err(e)) = &hook_step {
        if !note.is_empty() {
            note.push_str(" · ");
        }
        note.push_str(&format!("훅 재등록 실패: {e}"));
    }
    let lane = env.lane.clone();
    let now = env.now;
    let root = env.root.to_path_buf();
    let idc = id.clone();
    locked(id, &mut || {
        let _mutex_scope = super::guard::MutexScope::enter();
        if let Some(mut r) = ledger::read_record_in(&root, &idc)? {
            if r.origin_lane == lane {
                let method: String = format!("{label} — {by_label}{}", if note.is_empty() { String::new() } else { format!(" · {note}") }).chars().take(300).collect();
                if complete || nothing_changed {
                    if matches!(r.state, RepairState::Probation | RepairState::InUse) {
                        r.state = if complete { RepairState::Reverted } else { RepairState::Superseded };
                    }
                } // 충돌: the record keeps its state (truthful — not reverted), the row says 충돌
                r.methods.push(method);
                ledger::write_record_durable_in(&root, &r)?;
            }
        }
        if let Some(mut row) = ledger::read_lane_in(&root, &idc, &lane)? {
            if complete || nothing_changed {
                if let Some(p) = row.probation.as_mut() {
                    p.closed = Some(format!("{label} — {by_label}"));
                }
            } else {
                row.state = LaneState::Conflict;
                row.applied.retain(|(rel, _)| left_rels.contains(rel));
                if let Some(p) = row.probation.as_mut() {
                    p.closed = Some(format!("충돌 — 되돌리기를 마치지 못했습니다({by_label}) · {note}").chars().take(400).collect());
                }
            }
            row.updated_at = now;
            ledger::write_lane_in(&root, &row)?;
        }
        Ok(())
    })?;
    Ok(RevertReport {
        outcome: if complete {
            ApplyOutcome::Applied
        } else if nothing_changed {
            ApplyOutcome::Superseded(note)
        } else {
            ApplyOutcome::Declined(format!("{}: 되돌리기를 마치지 못했습니다(충돌 — 다시 되돌리면 남은 것을 마저 합니다) · {note}", id.as_str()))
        },
        files,
        hook_step,
    })
}

fn auto_revert_allowed(form: ApplyForm) -> bool {
    form == ApplyForm::Exchange
}

/// Closes this lane's probation of `rec` as `대체됨` (nothing reverted — the binding of design §4-9
/// "자동 복귀가 무엇에 묶이는가"); the origin's record → superseded for a worker's review.
fn close_superseded(
    env: &OpEnv<'_>,
    rec: &RepairRecord,
    why: &str,
    locked: &RepairLock<'_>,
) -> Result<RevertReport, RepairError> {
    let lane = env.lane.clone();
    let root = env.root.to_path_buf();
    let id = rec.id.clone();
    let now = env.now;
    locked(&rec.id, &mut || {
        let _mutex_scope = super::guard::MutexScope::enter();
        if let Some(mut row) = ledger::read_lane_in(&root, &id, &lane)? {
            if let Some(p) = row.probation.as_mut() {
                if p.closed.is_none() {
                    p.closed = Some(format!("대체됨 — {why}").chars().take(400).collect());
                }
            }
            row.updated_at = now;
            ledger::write_lane_in(&root, &row)?;
        }
        if let Some(mut r) = ledger::read_record_in(&root, &id)? {
            if r.origin_lane == lane && matches!(r.state, RepairState::Probation | RepairState::InUse) {
                r.state = RepairState::Superseded;
                r.methods.push(format!("대체됨(워커 검토) — {why}").chars().take(300).collect());
                ledger::write_record_durable_in(&root, &r)?;
            }
        }
        Ok(())
    })?;
    Ok(RevertReport {
        outcome: ApplyOutcome::Superseded(why.to_string()),
        files: Vec::new(),
        hook_step: None,
    })
}

/// Reverts one repair (T0 entry point; the daemon calls [`revert_in`] with its mutex and lock).
pub fn revert_repair(
    id: &RepairId,
    lane: &LaneKey,
    by: RevertCaller,
) -> Result<ApplyOutcome, RepairError> {
    let root = super::paths::repairs_root();
    let pack = crate::pack::pack_dir();
    let env = OpEnv {
        root: &root,
        pack: &pack,
        lane,
        now: ledger::now_epoch(),
        form: super::platform::apply_form(),
        faults: &NoFaults,
        own_check: &|_, _| None,
    };
    let lock = crate::pack_apply_lock::pack_apply_lock_path_for(&pack);
    crate::pack_apply_lock::with_apply_lock(&lock, || {
        revert_in(&env, id, by, &hook_lines_step, &no_repair_lock).map(|r| r.outcome)
    })
    .map_err(RepairError::Io)?
}

/// Stub in T0 (body: T7b).
pub fn propagate(id: &RepairId, lane: &LaneKey) -> Result<LaneState, RepairError> {
    let _ = (id, lane);
    Err(RepairError::NotImplemented("apply::propagate"))
}

/// A lane's catch-up status (RULINGS H3 / H5 required set).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CatchupStatus {
    pub complete: bool,
    pub pending: u32,
    pub conflict: u32,
}

/// Stub in T0 (body: T7b).
pub fn catchup_status(lane: &LaneKey) -> Result<CatchupStatus, RepairError> {
    let _ = lane;
    Err(RepairError::NotImplemented("apply::catchup_status"))
}

/// The `recheck` verb's lane re-evaluation (RULINGS H5). Stub in T0 (body: T7b).
pub fn recheck_lane(id: &RepairId, lane: &LaneKey) -> Result<LaneState, RepairError> {
    let _ = (id, lane);
    Err(RepairError::NotImplemented("apply::recheck_lane"))
}

// ───────────────────────────── F9 · R2 · P6 ─────────────────────────────

/// RULINGS DR2 — THE one resolution of "the approved copy" of `rel` for the CURRENT generation in this
/// lane; recognition (F9) and restoration (M10's source) both use it. Three answers:
/// - `Approved` — the newest repair of this generation whose lane row claims the file is an approved
///   boot-critical / protection-related repair that still stands (open, 사용 중, or closed 대체됨 by a
///   later live change — CU2) AND the full F9 agreement holds: the approval's fingerprint equals the
///   fingerprint of the operation set this lane applied (the row's applied states), the row's approval
///   reference is that approval's, every applied operation's incoming state is the row's state of its
///   file, the slot's `meta/pre.json` and the verified deposit agree with the operation's expected
///   state and identity, and the approved copy slot verifies (identity + state) → that copy;
/// - `NoApproval` — positively established: no row of this generation in this lane claims the file
///   (rows of reverted / failed repairs and of general repairs do not count) → the release's original;
/// - `Unknown` — any member unreadable or inconsistent (a corrupt lane row or record, an unreadable
///   operation log, a missing operation, a changed fingerprint / reference / deposit metadata, an
///   unverifiable copy) → NOTHING is put back, no check is skipped, a finding for the worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovedCopy {
    Approved { id: RepairId, state: TargetState, holder: PathBuf },
    NoApproval,
    Unknown(String),
    /// EN4: a record or lane row of the ledger cannot be read — the ledger is not complete, so no
    /// file of this lane can be answered NO APPROVAL; `cause` names the unreadable member.
    LedgerUnreadable { cause: String },
}

/// EN4: the first record or lane row of THIS lane's ledger that cannot be read (its path relative
/// to the repairs root), if any.
pub fn unreadable_ledger_member(root: &Path, lane: &LaneKey) -> Option<String> {
    let rel_of = |p: PathBuf| p.strip_prefix(root).map(|r| r.display().to_string()).unwrap_or_else(|_| p.display().to_string());
    // FG1: an enumeration that FAILED is not "no repairs"
    let ids = match ledger::list_ids_checked_in(root) {
        Ok(ids) => ids,
        Err(e) => return Some(format!("수리 장부 목록({e:?})")),
    };
    for id in ids {
        if ledger::read_record_in(root, &id).is_err() {
            return Some(rel_of(ledger::record_path_in(root, &id)));
        }
        if ledger::read_lane_in(root, &id, lane).is_err() {
            return Some(rel_of(ledger::lane_path_in(root, &id, lane)));
        }
    }
    None
}

pub fn resolve_approved_copy_in(root: &Path, pack: &Path, lane: &LaneKey, rel: &str) -> ApprovedCopy {
    // EJ1 / EN4: NO APPROVAL only from a COMPLETE, readable ledger of this lane — absence is never
    // evidence
    if let Some(cause) = unreadable_ledger_member(root, lane) {
        return ApprovedCopy::LedgerUnreadable { cause };
    }
    let ids = match ledger::list_ids_checked_in(root) {
        Ok(ids) => ids,
        Err(e) => return ApprovedCopy::LedgerUnreadable { cause: format!("수리 장부 목록({e:?})") },
    };
    for id in ids.into_iter().rev() {
        let (Ok(row), Ok(rec)) = (ledger::read_lane_in(root, &id, lane), ledger::read_record_in(root, &id)) else {
            return ApprovedCopy::LedgerUnreadable { cause: id.as_str().to_string() };
        };
        let Some(row) = row else {
            // a MISSING member: an in-force approved repair of this lane that committed the file but
            // whose lane row is not (yet) reconstructed
            if let Some(r) = &rec {
                if r.origin_lane == *lane
                    && is_approval_class(r.classification)
                    && matches!(r.state, RepairState::Probation | RepairState::InUse)
                    && r.committed.iter().any(|(c, _)| c == rel)
                {
                    return ApprovedCopy::Unknown(format!("{}: 이 파일을 확정했는데 이 부서의 행이 아직 없습니다", id.as_str()));
                }
            }
            continue;
        };
        if !row.applied.iter().any(|(r, _)| r == rel) {
            continue;
        }
        // FG1: a row that claims the file without its probation is an inconsistent member
        let Some(p) = row.probation.as_ref() else {
            return ApprovedCopy::Unknown(format!("{}: 이 파일을 말하는 행에 시험 기간 기록이 없습니다", id.as_str()));
        };
        match Bound::from_row(p) {
            Bound::Unknown => {
                return ApprovedCopy::Unknown(format!("{}: 이 파일을 말하는 행의 팩 세대를 알 수 없습니다(재구성된 행)", id.as_str()))
            }
            b if !b.equals_now(pack) => continue, // another generation's row
            _ => {}
        }
        if p.closed.as_deref().is_some_and(|c| c.starts_with("되돌려짐") || c.starts_with("실패")) {
            continue; // reverted or failed: it no longer defines the right bytes
        }
        let Some(rec) = rec else {
            return ApprovedCopy::Unknown(format!("{}: 행은 이 파일을 말하는데 수리 기록이 없습니다", id.as_str()));
        };
        if !is_approval_class(rec.classification) {
            continue;
        }
        return match f9_agreement(root, pack, &rec, &row, lane, rel) {
            Ok((state, holder)) => ApprovedCopy::Approved { id, state, holder },
            Err(why) => ApprovedCopy::Unknown(format!("{}: {why}", id.as_str())),
        };
    }
    ApprovedCopy::NoApproval
}

/// The full F9 agreement of one claiming repair (DR2); Ok = the verified approved copy.
fn f9_agreement(
    root: &Path,
    pack: &Path,
    rec: &RepairRecord,
    row: &ledger::LaneRecord,
    lane: &LaneKey,
    rel: &str,
) -> Result<(TargetState, PathBuf), String> {
    let id = &rec.id;
    let approval = rec
        .approval
        .as_ref()
        .filter(|a| a.approved)
        .ok_or_else(|| "행은 이 파일을 말하는데 유효한 승인이 없습니다".to_string())?;
    let applied_states: Option<Vec<(String, TargetState)>> =
        row.applied.iter().map(|(r, c)| parse_canon(c).map(|st| (r.clone(), st))).collect();
    let applied_states = applied_states.ok_or_else(|| "행의 상태를 읽을 수 없습니다".to_string())?;
    if super::types::fingerprint(&applied_states) != approval.fingerprint_sha256 {
        return Err("승인 지문이 이 부서가 적용한 상태와 다릅니다".into());
    }
    if row.approval_ref.as_deref() != Some(approval.request_id.as_str()) {
        return Err("행의 승인 참조가 승인과 다릅니다".into());
    }
    let (ops, broken) = wal::list_in(root, id);
    if !broken.is_empty() {
        return Err(format!("작업 기록을 읽을 수 없습니다({})", broken.len()));
    }
    let applied_op = |r: &str| {
        ops.iter()
            .filter(|o| {
                o.lane_key == *lane
                    && o.rel == r
                    && o.finished
                    && o.outcome.as_deref() == Some("applied")
                    && matches!(o.kind.as_str(), "apply" | "propagate")
            })
            .last()
            .cloned()
    };
    for (r, c) in &row.applied {
        if let Some(o) = applied_op(r) {
            if o.incoming != *c {
                return Err(format!("{r}: 적용 작업의 결과가 행의 상태와 다릅니다"));
            }
        }
    }
    let op = applied_op(rel).ok_or_else(|| "이 파일의 적용 작업 기록이 없습니다".to_string())?;
    // EJ1: the apply operation's DURABLE generation equals the pack's generation now
    if !Bound::of(op.pack_generation.clone()).equals_now(pack) {
        return Err("적용 작업이 묶인 팩 세대가 지금 세대와 다르거나 알 수 없습니다".into());
    }
    let incoming = parse_canon(&op.incoming).ok_or_else(|| "적용 작업의 상태를 읽을 수 없습니다".to_string())?;
    let expected = parse_canon(&op.expected).unwrap_or(TargetState::Absent);
    if expected != TargetState::Absent {
        let dir = op.slot.as_deref().ok_or_else(|| "적용 작업에 보관 칸이 없습니다".to_string())?;
        let slot = Slot::at(dir, rel);
        let (want_sha, want_mode, want_link) = match &expected {
            TargetState::File { sha256, mode } => (sha256.clone(), *mode, serde_json::Value::Null),
            TargetState::Link { target } => (String::new(), 0, serde_json::json!(target)),
            TargetState::Absent => (String::new(), 0, serde_json::Value::Null),
        };
        let pre_ok = std::fs::read(slot.meta_pre())
            .ok()
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            .is_some_and(|v| {
                v["repair"] == id.as_str()
                    && v["lane_key"] == lane.0.as_str()
                    && v["rel"] == rel
                    && v["sha256"] == want_sha.as_str()
                    && v["mode"] == want_mode
                    && v["link_target"] == want_link
            });
        if !pre_ok {
            return Err("보관 칸의 기록이 적용 작업의 기대 상태와 다릅니다".into());
        }
        if let (Some(i), TargetState::File { sha256, .. }) = (&op.live_identity, &expected) {
            if i.sha256 != *sha256 {
                return Err("적용 작업이 기록한 정체가 기대 상태와 다릅니다".into());
            }
        }
        if !vault::deposit_of(&slot).is_ok_and(|(st, _)| st == expected) {
            return Err("보관 칸의 내용을 확인할 수 없습니다".into());
        }
    }
    let (Some(dir), Some(want)) = (op.incoming_copy.as_ref(), op.incoming_copy_identity.as_ref()) else {
        return Err("승인본 칸이 기록되어 있지 않습니다".into());
    };
    let copy = Slot::at(dir, rel);
    match TargetState::read(&copy.payload()) {
        Ok(st) if st == incoming && identity_of(&copy.payload()).ok().as_ref() == Some(want) => Ok((st, copy.payload())),
        _ => Err("승인본 칸을 확인할 수 없습니다".into()),
    }
}

/// F9 recognition through THE resolution (DR2): the live state of `rel` IS the approved copy.
pub fn approved_copy_of_current_generation_in(
    root: &Path,
    pack: &Path,
    lane: &LaneKey,
    rel: &str,
) -> Result<bool, RepairError> {
    let live = TargetState::read(&pack.join(rel))?;
    Ok(matches!(resolve_approved_copy_in(root, pack, lane, rel), ApprovedCopy::Approved { state, .. } if state == live))
}

/// [`resolve_approved_copy_in`] as the verified copy only.
pub fn approved_copy_source_in(
    root: &Path,
    pack: &Path,
    lane: &LaneKey,
    rel: &str,
) -> Option<(TargetState, PathBuf)> {
    match resolve_approved_copy_in(root, pack, lane, rel) {
        ApprovedCopy::Approved { state, holder, .. } => Some((state, holder)),
        _ => None,
    }
}

/// F9's triple check (boot-critical OR protection-related — RULINGS U1); M10's.
pub fn approved_copy_of_current_generation(lane: &LaneKey, rel: &str) -> Result<bool, RepairError> {
    approved_copy_of_current_generation_in(
        &super::paths::repairs_root(),
        &crate::pack::pack_dir(),
        lane,
        rel,
    )
}

/// RULINGS R2 (+ Q3, U2): the origin lane has FINISHED, not-superseded apply operation records for
/// that approval whose per-target incoming STATES reproduce the approved fingerprint (through
/// `repair::fingerprint`), each with its sealed deposit (a created target needs only its record);
/// the repair is boot-critical or protection-related. Never reads the origin's live bytes or pack
/// generation (the origin may have hand-edited the file since — that does not void the approval).
pub fn origin_approval_triple_in(
    root: &Path,
    id: &RepairId,
    approval: &ApprovalResult,
    origin_lane: &LaneKey,
) -> bool {
    let Ok(Some(rec)) = ledger::read_record_in(root, id) else {
        return false;
    };
    if !is_approval_class(rec.classification) || !approval.approved {
        return false;
    }
    if rec.approval.as_ref() != Some(approval) {
        return false;
    }
    let (ops, _) = wal::list_in(root, id);
    let mut states: Vec<(String, TargetState)> = Vec::new();
    for f in ledger::file_targets(&rec) {
        let Some(op) = ops
            .iter()
            .filter(|r| {
                r.lane_key == *origin_lane
                    && r.rel == f.rel
                    && r.kind == "apply"
                    && r.finished
                    && r.outcome.as_deref() == Some("applied")
            })
            .last()
        else {
            return false;
        };
        let Some(st) = parse_canon(&op.incoming) else {
            return false;
        };
        if op.expected != "absent" {
            let Some(dir) = op.slot.as_deref() else {
                return false;
            };
            if vault::deposit_of(&Slot::at(dir, &f.rel)).is_err() {
                return false;
            }
        }
        states.push((f.rel.clone(), st));
    }
    !states.is_empty() && super::types::fingerprint(&states) == approval.fingerprint_sha256
}

/// RULINGS R2. False = applies nothing.
pub fn origin_approval_triple(
    id: &RepairId,
    approval: &ApprovalResult,
    origin_lane: &LaneKey,
) -> bool {
    origin_approval_triple_in(&super::paths::repairs_root(), id, approval, origin_lane)
}

/// RULINGS P6 / R5: the vault slots an in-force repair still depends on — (i) its pre-image
/// deposits (the `begin` copies and every lane's applied-operation deposits), (ii) the origin's
/// approved-apply deposits of a boot-critical / protection-related repair, (iii) every slot named by
/// an unfinished operation record. Deposits of operations closed `superseded` / `abandoned` /
/// `declined` and slots no current record refers to stay cleanable. A repair not in force needs
/// none. A record that cannot be read → `Err` (the caller refuses every slot).
pub fn slots_needed_in_force_in(root: &Path, id: &RepairId) -> Result<Vec<PathBuf>, RepairError> {
    let rec = ledger::read_record_in(root, id)?.ok_or_else(|| {
        RepairError::Precondition(format!("{}: 이 장부에서 수리 기록을 찾을 수 없습니다", id.as_str()))
    })?;
    let (ops, broken) = wal::list_in(root, id);
    if !broken.is_empty() {
        return Err(RepairError::Corrupt(format!(
            "{}: 읽을 수 없는 동작 기록이 있습니다",
            id.as_str()
        )));
    }
    let mut out: Vec<PathBuf> = Vec::new();
    for r in ops.iter().filter(|r| !r.finished) {
        if let Some(s) = &r.slot {
            out.push(s.clone());
        }
    }
    if matches!(rec.state, RepairState::Probation | RepairState::InUse) {
        for f in ledger::file_targets(&rec) {
            if let PreImage::Slot { slot_dir, .. } = &f.pre {
                out.push(slot_dir.clone());
            }
        }
        for r in ops.iter().filter(|r| {
            r.finished
                && r.outcome.as_deref() == Some("applied")
                && matches!(r.kind.as_str(), "apply" | "propagate")
        }) {
            if let Some(s) = &r.slot {
                out.push(s.clone());
            }
            // the last approved copy of an approved apply (§6-10 (2) — M10's put-back source)
            if r.form == "exchange" {
                if let Some(c) = &r.incoming_copy {
                    out.push(c.clone());
                }
            }
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}

/// Vault slots an in-force repair still depends on (RULINGS R5 / P6); T3's `cleanup` refuses every
/// slot on `Err`.
pub fn slots_needed_in_force(id: &RepairId) -> Result<Vec<PathBuf>, RepairError> {
    slots_needed_in_force_in(&super::paths::repairs_root(), id)
}

// ───────────────────────────── the approved apply (origin lane) ─────────────────────────────

/// The apply-time protection screen decision (RULINGS AY5 / AT2 / CM): `Ok(())` = the recorded
/// verdict holds or the screen run in this lane accepted; `Err(why)` = decline.
pub type ScreenStepFn<'a> = dyn Fn(&RepairId, &super::screen::ScreenCandidate, &Path) -> Result<(), String> + 'a;

/// The production screen step (AY5 as restated by CM): the verdict recorded at `submit` is re-used
/// ONLY when `recorded_verdict_holds` says so (same input fingerprint AND same screen build id);
/// otherwise the screen runs in THIS lane, and a refusal, a NOT-MEASURABLE probe (`declines_apply`)
/// or any error (a timeout included) declines the apply.
pub fn production_screen_step(
    root: &Path,
) -> impl Fn(&RepairId, &super::screen::ScreenCandidate, &Path) -> Result<(), String> + '_ {
    move |id, cand, lane_pack| {
        let recorded = super::screen::read_verdict_in(root, id).map_err(|e| format!("{e:?}"))?;
        if let Some(v) = recorded {
            if super::screen::recorded_verdict_holds(&v, cand, lane_pack).map_err(|e| format!("{e:?}"))? {
                return Ok(());
            }
        }
        let report = super::screen::screen_protections(cand, lane_pack, SCREEN_DEADLINE)
            .map_err(|e| format!("보호장치 검사를 돌릴 수 없어 적용하지 않습니다 — {e:?}"))?;
        if report.declines_apply() {
            let mut why: Vec<String> = report.refusals.clone();
            why.extend(report.not_measurable.iter().map(|m| format!("측정 불가: {m}")));
            return Err(format!("보호장치 검사가 이 레인에서 적용을 막았습니다 — {}", why.join(" · ")));
        }
        Ok(())
    }
}

/// Wall-time bound of the apply-time screen (the screen's whole run; exceeded → decline).
pub const SCREEN_DEADLINE: Duration = Duration::from_secs(300);

// ───────────────────────────── CT4 — one apply = one recoverable unit ─────────────────────────────

/// One target of an apply intent: its pre-image and the approved state (canon strings).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct IntentTarget {
    pub rel: String,
    pub pre: String,
    pub incoming: String,
}

/// RULINGS CT4 — the repair-level intent of one approved apply in one lane, durable BEFORE the first
/// exchange (`<root>/R-<n>/intents/<lane>.json`): `applying` → `applied` | `undone` | `incomplete`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ApplyIntent {
    pub schema: u32,
    pub repair: RepairId,
    pub lane_key: LaneKey,
    pub op_tag: String,
    pub state: String,
    pub targets: Vec<IntentTarget>,
    #[serde(default)]
    pub note: String,
    /// DR1: the pack generation the apply was bound to, recorded BEFORE the first exchange — no
    /// undo ever puts bytes back across a different generation.
    #[serde(default)]
    pub pack_generation: Option<PackGeneration>,
    /// EN8 / EL5: the reference of the approval this apply uses — the durable evidence that it was
    /// CONSUMED (`consumed::approval_consumed`), written before the first exchange.
    #[serde(default)]
    pub approval_ref: Option<String>,
    pub created_at: f64,
    pub updated_at: f64,
}

pub fn intent_path_in(root: &Path, id: &RepairId, lane: &LaneKey) -> PathBuf {
    ledger::repair_dir_in(root, id).join("intents").join(format!("{}.json", lane.0))
}

/// FG1: the apply intent with three answers — `Ok(Some)`, `Ok(None)` only when the file does not exist
/// AND the repair's folder could be read, `Err` for everything else.
pub fn read_intent_checked_in(root: &Path, id: &RepairId, lane: &LaneKey) -> Result<Option<ApplyIntent>, String> {
    read_checked(&intent_path_in(root, id, lane), &ledger::repair_dir_in(root, id))
}

fn read_checked<T: serde::de::DeserializeOwned>(path: &Path, owner_dir: &Path) -> Result<Option<T>, String> {
    match std::fs::read(path) {
        Ok(b) => serde_json::from_slice(&b).map(Some).map_err(|e| format!("{}: 읽을 수 없음 — {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => match std::fs::read_dir(owner_dir) {
            Ok(_) => Ok(None),
            Err(e2) => Err(format!("{}: 없음, 그러나 {} 도 읽을 수 없음 — {e2}", path.display(), owner_dir.display())),
        },
        Err(e) => Err(format!("{}: 읽을 수 없음 — {e}", path.display())),
    }
}

pub fn read_intent_in(root: &Path, id: &RepairId, lane: &LaneKey) -> Option<ApplyIntent> {
    let b = std::fs::read(intent_path_in(root, id, lane)).ok()?;
    serde_json::from_slice(&b).ok()
}

fn write_intent(root: &Path, intent: &ApplyIntent) -> Result<(), RepairError> {
    super::durable::write_json_durable(&intent_path_in(root, &intent.repair, &intent.lane_key), intent)
}

fn set_intent_state(root: &Path, intent: &mut ApplyIntent, state: &str, note: &str, now: f64) -> Result<(), RepairError> {
    intent.state = state.to_string();
    intent.note = note.chars().take(400).collect();
    intent.updated_at = now;
    write_intent(root, intent)
}

/// CT4 recovery (the next ready tick, under the pack-apply lock — CT5): every `applying` intent of
/// THIS lane is completed or undone AS A WHOLE. Its record already in 시험 기간 with this lane's row
/// → the apply had finished (only the intent's last write was lost): `applied`. Otherwise every
/// target whose live state is the approved one is put back to its pre-image by a checked operation
/// — operations that had individually finished included — and the approval is void (`undone`);
/// what cannot be put back leaves the lane row 충돌 with exactly those targets, revertible
/// (`incomplete`). Per repair fault-isolated (CU1): one repair's failure is a line, not an abort.
pub fn recover_apply_intents_in(env: &OpEnv<'_>, locked: &RepairLock<'_>) -> Vec<String> {
    let mut lines = Vec::new();
    let ids = match ledger::list_ids_checked_in(env.root) {
        Ok(ids) => ids,
        Err(e) => return vec![format!("수리 장부 목록을 읽을 수 없어 끊긴 적용을 찾지 못했습니다 — 다음 차례에 다시({e:?})")],
    };
    for id in ids {
        // EJ4: an apply running now holds the repair's guard; the intent is read AFTER holding it
        let Some(_guard) = super::guard::RepairGuard::try_take(env.root, &id) else {
            continue;
        };
        let mut intent = match read_intent_checked_in(env.root, &id, env.lane) {
            Ok(Some(i)) => i,
            Ok(None) => continue,
            Err(e) => {
                lines.push(format!("{}: 적용 의도를 읽을 수 없습니다 — 다음 차례에 다시({e})", id.as_str()));
                continue;
            }
        };
        if intent.state != "applying" {
            continue;
        }
        match recover_one_intent(env, &mut intent, locked) {
            Ok(line) => lines.push(line),
            Err(e) => lines.push(format!("{}: 끊긴 적용을 정리하지 못했습니다 — 다음 차례에 다시({e:?})", id.as_str())),
        }
    }
    lines
}

fn recover_one_intent(env: &OpEnv<'_>, intent: &mut ApplyIntent, locked: &RepairLock<'_>) -> Result<String, RepairError> {
    let id = intent.repair.clone();
    let rec = ledger::read_record_in(env.root, &id)?
        .ok_or_else(|| RepairError::Corrupt(format!("{}: 적용 의도는 있는데 수리 기록이 없습니다", id.as_str())))?;
    // DX2 — the record's durable transition is the commit point: in force (시험 기간) → the apply
    // COMPLETES forward (its lane row written from the record and the intent, the intent applied);
    // never an undo of a committed apply
    if matches!(rec.state, RepairState::Probation | RepairState::InUse) {
        let (gen_version, gen_pro) = Bound::of(intent.pack_generation.clone()).row_fields();
        let root = env.root.to_path_buf();
        let lane = env.lane.clone();
        let idc = id.clone();
        let applied_at = intent.created_at;
        let committed = rec.committed.clone();
        let request = rec.approval.as_ref().map(|a| a.request_id.clone());
        let now = env.now;
        locked(&id, &mut || {
        let _mutex_scope = super::guard::MutexScope::enter();
            // EJ5: THIS repair's row fields are rebuilt from the committed record and the intent
            // whether or not a row exists (an older row's applied subset, approval reference or
            // probation state is replaced); the lines of other events (merges, history) are kept
            let previous = ledger::read_lane_in(&root, &idc, &lane)?;
            ledger::write_lane_in(
                &root,
                &ledger::LaneRecord {
                    schema: super::REPAIR_SCHEMA,
                    repair: idc.clone(),
                    lane: lane.clone(),
                    state: LaneState::Verified,
                    probation: Some(ledger::Probation {
                        pack_generation: gen_version.clone(),
                        pro_revision: gen_pro,
                        applied_at,
                        expected: committed.clone(),
                        closed: None,
                        check_timings: Vec::new(),
                        check_timeouts: Vec::new(),
                    }),
                    merge: previous.as_ref().and_then(|p| p.merge.clone()),
                    merge_history: previous.as_ref().map(|p| p.merge_history.clone()).unwrap_or_default(),
                    approval_ref: request.clone(),
                    conflict_live: Vec::new(),
                    applied: committed.clone(),
                    applied_history: previous.as_ref().map(|p| p.applied_history.clone()).unwrap_or_default(),
                    merge_base: previous.as_ref().map(|p| p.merge_base.clone()).unwrap_or_default(),
                    merge_lock: previous.as_ref().and_then(|p| p.merge_lock.clone()),
                    merge_attempt: previous.as_ref().map(|p| p.merge_attempt).unwrap_or(0),
                    updated_at: now,
                },
            )?;
            Ok(())
        })?;
        set_intent_state(env.root, intent, "applied", "기록이 이미 시험 기간이었습니다 — 부서 행을 마저 쓰고 끝냄", env.now)?;
        return Ok(format!("{}: 끊긴 적용은 기록이 확정되어 있어 앞으로 마쳤습니다(부서 행)", id.as_str()));
    }
    // DR1 — no put-back across a generation change: the intent's binding is checked first
    if !Bound::of(intent.pack_generation.clone()).equals_now(env.pack) {
        {
            let why = format!(
                "적용 중에 끊긴 뒤 팩 세대가 바뀌었거나 알 수 없습니다(지금 {}) — 아무것도 되돌리지 않았습니다(새 버전 우선 · 워커 검토)",
                pack_generation(env.pack).version
            );
            void_approval(env, &id, &why, locked)?;
            set_intent_state(env.root, intent, "superseded", &why, env.now)?;
            return Ok(format!("{}: {why}", id.as_str()));
        }
    }
    let mut specs = Vec::new();
    for t in &intent.targets {
        let pre = parse_canon(&t.pre).ok_or_else(|| RepairError::Corrupt(format!("{}: 의도의 상태를 읽을 수 없습니다", t.rel)))?;
        let incoming = parse_canon(&t.incoming).ok_or_else(|| RepairError::Corrupt(format!("{}: 의도의 상태를 읽을 수 없습니다", t.rel)))?;
        specs.push(OpSpec {
            id: id.clone(),
            rel: t.rel.clone(),
            kind: "apply".into(),
            expected: pre,
            incoming,
            source: None,
            bound: Bound::of(intent.pack_generation.clone()),
        });
    }
    let left = undo_applied_with(env, &specs, &rec, true)?;
    if left.is_empty() {
        let why = "적용 중에 끊겨 바꾼 것을 모두 되돌렸습니다 — 다시 승인이 필요합니다";
        void_approval(env, &id, why, locked)?;
        set_intent_state(env.root, intent, "undone", why, env.now)?;
        return Ok(format!("{}: {why}", id.as_str()));
    }
    let detail = left.iter().map(|(r, w)| format!("{r}: {w}")).collect::<Vec<_>>().join(" · ");
    let why = format!("적용 중에 끊겼고 바꾼 것을 다 되돌리지 못했습니다(충돌 — cys repair revert {} 로 되돌릴 수 있습니다) · {detail}", id.as_str());
    mark_incomplete_apply(env, &id, &rec, &specs, &why, locked)?;
    set_intent_state(env.root, intent, "incomplete", &why, env.now)?;
    Ok(format!("{}: {why}", id.as_str()))
}

/// What the approved apply did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyReport {
    pub outcome: ApplyOutcome,
    pub files: Vec<(String, ApplyOutcome)>,
}

/// The approved apply of a boot-critical / protection-related repair in its ORIGIN lane (T5b calls
/// it after the owner's approval): every staged candidate copied ONCE into a fresh slot (made
/// durable), the SLOTS' states fingerprinted and compared with the approved fingerprint (mismatch →
/// nothing applied; the working copy is never read again), the path / alias rules (AW3 (b)(c)(d)),
/// the protection screen at apply (AY5) right before the exchanges, the live states = the pre-images
/// recorded at `begin`, then rule one per target; afterwards the Q3 invariant (the live states
/// reproduce the approved fingerprint) and the file's own check right away (a timeout is a failure
/// — B3): a failure puts that ONE repair back by rule one. A declined apply voids the approval and
/// returns the repair to `진행 중`. The CALLER holds the pack-apply lock.
#[allow(clippy::too_many_arguments)]
pub fn apply_candidate_in(
    env: &OpEnv<'_>,
    id: &RepairId,
    files: &[CandidateFile],
    screen: &ScreenStepFn<'_>,
    check: &CheckFn<'_>,
    reference: &ReferenceFn<'_>,
    locked: &RepairLock<'_>,
) -> Result<ApplyReport, RepairError> {
    // EJ4: the repair's ONE guard for the whole apply (tried; held → busy, never waited for)
    let Some(_guard) = super::guard::RepairGuard::try_take(env.root, id) else {
        return Err(RepairError::Locked(format!("{}: {}", id.as_str(), super::guard::BUSY)));
    };
    let rec = ledger::read_record_in(env.root, id)?
        .ok_or_else(|| RepairError::Precondition(format!("{}: 그런 수리가 없습니다", id.as_str())))?;
    let decline = |why: String| -> Result<ApplyReport, RepairError> {
        void_approval(env, id, &why, locked)?;
        Ok(ApplyReport {
            outcome: ApplyOutcome::Declined(why),
            files: Vec::new(),
        })
    };
    if !is_approval_class(rec.classification) {
        return Err(RepairError::Precondition(format!(
            "{}: 기동 필수·보호장치 관련 수리만 승인 뒤 데몬이 적용합니다",
            id.as_str()
        )));
    }
    if env.form != ApplyForm::Exchange {
        return Err(RepairError::PlatformPending(
            "이 운영체제에서는 기동 필수·보호장치 관련 수리를 적용하지 않습니다(대기)",
        ));
    }
    let Some(approval) = rec.approval.clone().filter(|a| a.approved) else {
        return Err(RepairError::Precondition(format!(
            "{}: 데몬이 기록한 승인 결과가 없습니다",
            id.as_str()
        )));
    };
    if rec.origin_lane != *env.lane {
        return Err(RepairError::Precondition(format!(
            "{}: 승인된 적용은 처음 고친 부서에서만 합니다(다른 부서는 전파)",
            id.as_str()
        )));
    }
    if matches!(rec.state, RepairState::Probation | RepairState::InUse | RepairState::Reverted | RepairState::Superseded | RepairState::UpstreamFixed) {
        return Err(RepairError::Precondition(format!(
            "{}: '{}' 상태라 적용하지 않습니다",
            id.as_str(),
            rec.state.label_ko()
        )));
    }
    // every declared file target must be in `files`, and nothing else
    let mut declared: Vec<&str> = ledger::file_targets(&rec).map(|f| f.rel.as_str()).collect();
    declared.sort_unstable();
    declared.dedup();
    let mut given: Vec<&str> = files.iter().map(|f| f.rel.as_str()).collect();
    given.sort_unstable();
    if declared != given {
        return decline(format!(
            "{}: 적용할 파일 목록이 수리 기록의 대상과 다릅니다",
            id.as_str()
        ));
    }
    // AW3 (b)(c)(d) at apply
    for f in files {
        if let Err(e) = super::types::parent_chain_link(env.pack, &f.rel) {
            return decline(format!("{e:?}"));
        }
    }
    if let Err(why) =
        super::boot_critical::refuse_identity_alias(env.pack, files.iter().map(|f| f.rel.as_str()))
    {
        return decline(why);
    }
    // the candidate copied ONCE into fresh slots; from here on the staging is never read again
    let op_tag = wal::new_op_id(env.now, "candidate");
    let mut slot_states: Vec<(String, TargetState)> = Vec::new();
    let mut slots: Vec<(String, Option<Slot>)> = Vec::new();
    for f in files {
        let staged_state = TargetState::read(&f.staged)?;
        let (staged, st_kind) = match &staged_state {
            TargetState::Absent => (Staged::Empty, TargetState::Absent),
            TargetState::Link { target } => {
                if let Some(why) = super::types::link_target_refusal(env.pack, &f.rel, target) {
                    return decline(why);
                }
                (Staged::Link { target: target.clone() }, staged_state.clone())
            }
            TargetState::File { mode, .. } => (
                Staged::FileFrom {
                    source: f.staged.clone(),
                    mode: *mode,
                },
                staged_state.clone(),
            ),
        };
        if st_kind == TargetState::Absent {
            slot_states.push((f.rel.clone(), TargetState::Absent));
            slots.push((f.rel.clone(), None));
            continue;
        }
        let pre = ledger::file_targets(&rec)
            .find(|x| x.rel == f.rel)
            .map(|x| x.pre.state())
            .unwrap_or(TargetState::Absent);
        let slot = vault::build_slot(
            env.root,
            id,
            env.lane,
            &f.rel,
            &op_tag,
            DepositKind::Displaced,
            &pre,
            &staged,
            env.now,
        )?;
        slot_states.push((f.rel.clone(), TargetState::read(&slot.payload())?));
        slots.push((f.rel.clone(), Some(slot)));
    }
    if super::types::fingerprint(&slot_states) != approval.fingerprint_sha256 {
        return decline(format!(
            "{}: 보관소에 옮긴 후보가 승인된 지문과 다릅니다 — 아무것도 적용하지 않았습니다",
            id.as_str()
        ));
    }
    // the protection screen at apply (AY5), over exactly the slots' bytes. CT12: the view is removed
    // on EVERY exit (a drop guard — a panic included; a crash's leftover is swept by the start pass:
    // its name carries this process's pid) and its bytes are bound back to the slot states after
    // the screen, before anything is exchanged
    struct ViewGuard(PathBuf);
    impl Drop for ViewGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0); // our own view of the slots' bytes (never a slot)
        }
    }
    let view = vault::vault_dir_in(env.root, id).join(format!(".screen-{}-{op_tag}", std::process::id()));
    let view_guard = ViewGuard(view.clone());
    let screen_result = (|| -> Result<(), String> {
        for (rel, slot) in &slots {
            if let Some(s) = slot {
                let dst = view.join(rel);
                if let Some(p) = dst.parent() {
                    std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
                }
                match TargetState::read(&s.payload()).map_err(|e| format!("{e:?}"))? {
                    TargetState::Link { target } => {
                        #[cfg(unix)]
                        std::os::unix::fs::symlink(&target, &dst).map_err(|e| e.to_string())?;
                        #[cfg(not(unix))]
                        let _ = target;
                    }
                    _ => {
                        std::fs::copy(s.payload(), &dst).map_err(|e| e.to_string())?;
                    }
                }
            }
        }
        let cand = super::screen::ScreenCandidate {
            files: view.clone(),
            targets: slot_states.clone(),
        };
        screen(id, &cand, env.pack)?;
        for (rel, st) in &slot_states {
            if *st == TargetState::Absent {
                continue;
            }
            let seen = TargetState::read(&view.join(rel)).map_err(|e| format!("{e:?}"))?;
            if seen != *st {
                return Err(format!(
                    "{rel}: 보호장치 검사가 본 내용이 보관 칸의 승인된 상태와 다릅니다 — 적용하지 않습니다"
                ));
            }
        }
        Ok(())
    })();
    drop(view_guard);
    if let Err(why) = screen_result {
        return decline(why);
    }
    // CU4: the alias check repeated right before the exchanges (the screen may have run for minutes)
    if let Err(why) =
        super::boot_critical::refuse_identity_alias(env.pack, files.iter().map(|f| f.rel.as_str()))
    {
        return decline(why);
    }
    // EJ1: the apply's binding is born HERE, once (written into its intent before any exchange)
    let apply_bound = Bound::now(env.pack);
    if apply_bound == Bound::Unknown {
        return decline(format!("{}: 팩 세대를 알 수 없어 적용하지 않았습니다(아무것도 바꾸지 않음)", id.as_str()));
    }
    // step 0 of every target first: the live state is the pre-image recorded at `begin`
    let mut plan: Vec<(OpSpec, Option<Slot>)> = Vec::new();
    for (rel, slot) in &slots {
        let pre = ledger::file_targets(&rec)
            .find(|x| x.rel == *rel)
            .map(|x| x.pre.state())
            .unwrap_or(TargetState::Absent);
        let live_now = TargetState::read(&env.pack.join(rel))?;
        if live_now != pre {
            return decline(format!(
                "{rel}: 살아 있는 파일이 수리를 시작할 때와 다릅니다(그 사이 되돌리기·업데이트·다른 작성자) — 승인은 무효가 되었습니다"
            ));
        }
        let incoming = slot_states
            .iter()
            .find(|(r, _)| r == rel)
            .map(|(_, s)| s.clone())
            .unwrap_or(TargetState::Absent);
        plan.push((
            OpSpec {
                id: id.clone(),
                rel: rel.clone(),
                kind: "apply".into(),
                expected: pre,
                incoming,
                source: slot.as_ref().map(|s| s.payload()),
                // EJ1: bound to the apply's binding (its undo carries the same binding)
                bound: apply_bound.clone(),
            },
            slot.clone(),
        ));
    }
    // CT4: the repair-level intent is durable BEFORE the first exchange
    let mut intent = ApplyIntent {
        schema: super::REPAIR_SCHEMA,
        repair: id.clone(),
        lane_key: env.lane.clone(),
        op_tag: op_tag.clone(),
        state: "applying".into(),
        targets: plan
            .iter()
            .map(|(sp, _)| IntentTarget { rel: sp.rel.clone(), pre: sp.expected.canon(), incoming: sp.incoming.canon() })
            .collect(),
        note: String::new(),
        pack_generation: apply_bound.known(),
        approval_ref: Some(approval.request_id.clone()),
        created_at: env.now,
        updated_at: env.now,
    };
    if let Err(e) = write_intent(env.root, &intent) {
        return decline(format!("{}: 적용 의도를 기록할 수 없어 아무것도 바꾸지 않았습니다 — {e:?}", id.as_str()));
    }
    // rule one per target — the exchange uses the very slot that was fingerprinted
    let mut results: Vec<(String, ApplyOutcome)> = Vec::new();
    let mut applied: Vec<OpSpec> = Vec::new();
    let mut failed: Option<String> = None;
    for (spec, slot) in &plan {
        let r = match (&spec.expected, &spec.incoming, slot) {
            (TargetState::Absent, _, _) | (_, TargetState::Absent, _) | (_, _, None) => run_op(env, spec)?,
            (_, _, Some(s)) => {
                let copy = vault::build_slot(
                    env.root,
                    id,
                    env.lane,
                    &spec.rel,
                    &op_tag,
                    DepositKind::StagedRestore,
                    &spec.incoming,
                    &Staged::FileFrom {
                        source: s.payload(),
                        mode: match &spec.incoming {
                            TargetState::File { mode, .. } => *mode,
                            _ => 0,
                        },
                    },
                    env.now,
                )
                .ok()
                .filter(|_| matches!(spec.incoming, TargetState::File { .. }));
                exchange_prepared(env, spec, s, copy.as_ref())?
            }
        };
        if r.outcome == ApplyOutcome::Applied {
            applied.push(spec.clone());
        } else if failed.is_none() {
            failed = Some(format!("{}: {:?}", spec.rel, r.outcome));
        }
        results.push((spec.rel.clone(), r.outcome));
        if failed.is_some() {
            break;
        }
    }
    // CT3: every target this apply touched (the failed one included — its own measured undo may
    // have failed) is put back by a CHECKED operation; what could not be put back is never reported
    // as reverted: the lane row says 충돌 and `revert` can finish it
    let attempted: Vec<OpSpec> = plan.iter().take(results.len()).map(|(s, _)| s.clone()).collect();
    let _ = &applied;
    let undo_then = |why: &str, files: Vec<(String, ApplyOutcome)>| -> Result<Option<ApplyReport>, RepairError> {
        let left = undo_applied(env, &attempted, &rec)?;
        if left.is_empty() {
            return Ok(None);
        }
        let detail = left.iter().map(|(r, w)| format!("{r}: {w}")).collect::<Vec<_>>().join(" · ");
        let msg = format!(
            "{id}: 적용을 끝내지 못했고 바꾼 것을 다 되돌리지도 못했습니다(충돌 — cys repair revert {id} 로 되돌릴 수 있습니다) — {why} · {detail}",
            id = id.as_str()
        );
        mark_incomplete_apply(env, id, &rec, &attempted, &msg, locked)?;
        let mut i2 = intent.clone();
        set_intent_state(env.root, &mut i2, "incomplete", &msg, env.now)?;
        Ok(Some(ApplyReport {
            outcome: ApplyOutcome::Declined(msg),
            files,
        }))
    };
    let mark_undone = |why: &str| -> Result<(), RepairError> {
        let mut i2 = intent.clone();
        set_intent_state(env.root, &mut i2, "undone", why, env.now)
    };
    if let Some(why) = failed {
        // put back what this apply already changed (rule one), then decline
        if let Some(r) = undo_then(&why, results.clone())? {
            return Ok(r);
        }
        mark_undone(&why)?;
        return decline(format!("{}: 적용이 끝까지 가지 못해 바꾼 것을 되돌렸습니다 — {why}", id.as_str()));
    }
    // Q3: the live states reproduce the approved fingerprint
    let mut live_states = Vec::new();
    for (rel, _) in &slots {
        live_states.push((rel.clone(), TargetState::read(&env.pack.join(rel))?));
    }
    if super::types::fingerprint(&live_states) != approval.fingerprint_sha256 {
        if let Some(r) = undo_then("적용 뒤 살아 있는 파일이 승인된 지문과 다릅니다", results.clone())? {
            return Ok(r);
        }
        mark_undone("적용 뒤 살아 있는 파일이 승인된 지문과 다릅니다")?;
        return decline(format!(
            "{}: 적용 뒤 살아 있는 파일이 승인된 지문과 다릅니다 — 되돌렸습니다",
            id.as_str()
        ));
    }
    // the file's own check right after the apply (a timeout is a failure — B3)
    let mut timings = Vec::new();
    for (rel, st) in &live_states {
        if *st == TargetState::Absent {
            continue;
        }
        let deadline = check_deadline(rel);
        let c = check(rel, env.pack, deadline)?;
        let r = reference(check_kind_of(rel), deadline).ok();
        match c.result {
            CheckResult::Pass => timings.push(ledger::CheckTiming {
                rel: rel.clone(),
                check_ms: c.ms,
                reference_kind: check_kind_of(rel),
                reference_ms: r.map(|x| x.ms).unwrap_or(0),
            }),
            other => {
                let why = match other {
                    CheckResult::Fail(m) => m,
                    _ => "검사 시간 초과(적용 때의 시간 초과는 실패)".to_string(),
                };
                if let Some(r) = undo_then(&format!("{rel}: 적용 직후 검사 실패 — {why}"), results.clone())? {
                    return Ok(r);
                }
                mark_undone(&format!("{rel}: 적용 직후 검사 실패 — {why}"))?;
                mark_reverted_after_apply_check(env, id, &format!("{rel}: {why}"), locked)?;
                return Ok(ApplyReport {
                    outcome: ApplyOutcome::Declined(format!(
                        "{rel}: 적용 직후 그 파일의 검사가 실패해 이 수리 한 건을 되돌렸습니다 — {why}"
                    )),
                    files: results,
                });
            }
        }
    }
    // record: committed states, 시험 기간, this lane's row (O1(a) approval_ref), older repairs superseded
    let committed: Vec<(String, String)> = live_states.iter().map(|(r, s)| (r.clone(), s.canon())).collect();
    let (gen_version, gen_pro) = apply_bound.row_fields();
    let lane = env.lane.clone();
    let root = env.root.to_path_buf();
    let now = env.now;
    let idc = id.clone();
    let request_id = approval.request_id.clone();
    let first_live = env.pack.join(slots.first().map(|(r, _)| r.as_str()).unwrap_or(""));
    locked(id, &mut || {
        let _mutex_scope = super::guard::MutexScope::enter();
        let Some(mut r) = ledger::read_record_in(&root, &idc)? else {
            return Ok(());
        };
        r.committed = committed.clone();
        r.state = RepairState::Probation;
        r.lock = None;
        ledger::write_record_durable_in(&root, &r)?;
        // DX2's window (tests): the record is committed, the lane row not yet written
        if env.faults.at(Point::AfterRecordCommit, &first_live, None) {
            return Err(stop(Point::AfterRecordCommit));
        }
        let previous = ledger::read_lane_in(&root, &idc, &lane)?;
        let row = ledger::LaneRecord {
            schema: super::REPAIR_SCHEMA,
            repair: idc.clone(),
            lane: lane.clone(),
            state: LaneState::Verified,
            probation: Some(ledger::Probation {
                pack_generation: gen_version.clone(),
                pro_revision: gen_pro,
                applied_at: now,
                expected: committed.clone(),
                closed: None,
                check_timings: timings.clone(),
                check_timeouts: Vec::new(),
            }),
            merge: None,
            merge_history: previous.as_ref().map(|p| p.merge_history.clone()).unwrap_or_default(),
            approval_ref: Some(request_id.clone()),
            conflict_live: Vec::new(),
            applied: committed.clone(),
            applied_history: previous.as_ref().map(|p| p.applied_history.clone()).unwrap_or_default(),
            merge_base: Vec::new(),
            merge_lock: None,
            merge_attempt: 0,
            updated_at: now,
        };
        ledger::write_lane_in(&root, &row)?;
        Ok(())
    })?;
    // committed deletions are tombstoned in this lane (only real deletions — AD3)
    for (rel, st) in &live_states {
        let pre_present = ledger::file_targets(&rec)
            .find(|f| f.rel == *rel)
            .is_some_and(|f| f.pre != PreImage::Absent);
        if *st == TargetState::Absent && pre_present {
            let _ = super::tombstones::record_in(
                env.root,
                env.lane,
                &super::tombstones::Tombstone {
                    rel: rel.clone(),
                    kind: super::tombstones::TombstoneKind::File,
                    repair: id.clone(),
                },
            );
        }
    }
    set_intent_state(env.root, &mut intent, "applied", "", env.now)?;
    supersede_older_in(env.root, env.lane, &rec, locked)?;
    Ok(ApplyReport {
        outcome: ApplyOutcome::Applied,
        files: results,
    })
}

/// The exchange of a slot that was ALREADY built and fingerprinted (the approved apply — R2/Q3:
/// "exchanges that slot"): steps 0–5 of rule one without staging again.
fn exchange_prepared(
    env: &OpEnv<'_>,
    spec: &OpSpec,
    slot: &Slot,
    approved_copy: Option<&Slot>,
) -> Result<OpDone, RepairError> {
    let live = env.pack.join(&spec.rel);
    let parent = live.parent().unwrap_or(env.pack).to_path_buf();
    match same_filesystem(&slot.dir, &parent) {
        Ok(true) => {}
        Ok(false) => {
            return Ok(done(
                ApplyOutcome::Declined(format!("{}: 보관소와 살아 있는 파일이 다른 파일시스템입니다", spec.rel)),
                None,
            ))
        }
        Err(e) => return Ok(done(ApplyOutcome::Declined(format!("{e:?}")), None)),
    }
    if TargetState::read(&live)? != spec.expected {
        return Ok(done(
            ApplyOutcome::Superseded(format!("{}: 살아 있는 파일이 기대한 상태가 아닙니다", spec.rel)),
            None,
        ));
    }
    // the slot was fingerprinted before the screen (which may run for minutes): it must still hold
    // exactly the approved state at the exchange — bytes written into it since are never applied
    if TargetState::read(&slot.payload())? != spec.incoming {
        return Ok(done(
            ApplyOutcome::Declined(format!(
                "{}: 보관소의 후보가 승인된 지문을 확인한 뒤에 바뀌었습니다 — 적용하지 않습니다",
                spec.rel
            )),
            None,
        ));
    }
    if env.faults.at(Point::AfterValidation, &live, Some(slot)) {
        return Err(stop(Point::AfterValidation));
    }
    // EJ1: the operation's record carries the apply's binding (born with its intent), never "now"
    let Some(gen) = spec.bound.known().filter(|_| spec.bound.equals_now(env.pack)) else {
        return Ok(done(
            ApplyOutcome::Superseded(format!("{}: 팩 세대가 적용이 묶인 세대와 다르거나 알 수 없습니다", spec.rel)),
            None,
        ));
    };
    let (slot_identity, live_identity) = match (identity_of(&slot.payload()), identity_of(&live)) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(e), _) | (_, Err(e)) => {
            return Ok(done(ApplyOutcome::Superseded(format!("{}: {e:?}", spec.rel)), None))
        }
    };
    let op = wal::new_op_id(env.now, &spec.kind);
    let mut rec = new_record(env, spec, &op, "exchange", &gen);
    rec.slot = Some(slot.dir.clone());
    rec.slot_identity = Some(slot_identity);
    rec.live_identity = Some(live_identity);
    if let Some(c) = approved_copy {
        // §6-10 (2): the last approved copy of the current generation (a write-once slot that is
        // never exchanged) — M10's put-back source while this repair is in force
        rec.incoming_copy = Some(c.dir.clone());
        rec.incoming_copy_identity = identity_of(&c.payload()).ok();
    }
    if let Err(e) = wal::write_in(env.root, &rec) {
        return Ok(done(
            ApplyOutcome::Declined(format!("{}: 동작 기록을 쓸 수 없어 적용하지 않습니다 — {e:?}", spec.rel)),
            None,
        ));
    }
    if env.faults.at(Point::AfterRecord, &live, Some(slot)) {
        return Err(stop(Point::AfterRecord));
    }
    if let Err(e) = door(env, &spec.bound, &live, DoorAct::Exchange { payload: &slot.payload() }) {
        return door_refused(env, &mut rec, spec, e);
    }
    seal_now(env, slot, &live)?;
    after_seal_exchange(env, spec, &mut rec, slot, &live, 0)
}

/// Puts back (rule one) the targets an unfinished apply already changed — each a CHECKED operation
/// whose expected state is what is actually live now (CT3). Returns the targets that could NOT be
/// put back (rel, why) — empty = every change undone.
fn undo_applied(env: &OpEnv<'_>, applied: &[OpSpec], rec: &RepairRecord) -> Result<Vec<(String, String)>, RepairError> {
    undo_applied_with(env, applied, rec, false)
}

/// [`undo_applied`]; `only_ours` (CT4 recovery): a target whose live state is neither the pre-image
/// nor the apply's state is left alone and reported (someone else wrote it after the crash).
fn undo_applied_with(env: &OpEnv<'_>, applied: &[OpSpec], rec: &RepairRecord, only_ours: bool) -> Result<Vec<(String, String)>, RepairError> {
    let mut left = Vec::new();
    for spec in applied.iter().rev() {
        if TargetState::read(&env.pack.join(&spec.rel))? == spec.expected {
            continue; // nothing of this target was changed (its pre-image is live)
        }
        let (pre, holder) = match pre_image_for(env.root, rec, env.lane, &spec.rel) {
            Ok(x) => x,
            Err(e) => {
                left.push((spec.rel.clone(), format!("적용 전 내용을 확인할 수 없습니다({e:?})")));
                continue;
            }
        };
        let live_now = TargetState::read(&env.pack.join(&spec.rel))?;
        if live_now == pre {
            continue; // already the pre-image (e.g. undone by the operation's own CT3 check)
        }
        if only_ours && live_now != spec.incoming {
            left.push((spec.rel.clone(), format!("살아 있는 것이 적용 전도 적용 결과도 아닙니다({}) — 그대로 둠", live_now.canon())));
            continue;
        }
        let back = OpSpec {
            id: spec.id.clone(),
            rel: spec.rel.clone(),
            kind: "revert".into(),
            expected: live_now,
            incoming: pre,
            source: holder,
            // DR1: the apply's binding goes with its undo (a different generation → nothing back)
            bound: spec.bound.clone(),
        };
        let r = run_op(env, &back)?;
        if r.outcome != ApplyOutcome::Applied {
            left.push((spec.rel.clone(), format!("{:?}", r.outcome)));
        }
    }
    Ok(left)
}

/// CT3 / CT4: an approved apply that could not be completed NOR fully undone — the lane row says
/// 충돌 with exactly the targets whose live state is the apply's (so `revert` can put them back),
/// its probation block open and bound to this generation; the record keeps its state with a method
/// line; the approval is void. Never reported as applied or reverted.
fn mark_incomplete_apply(
    env: &OpEnv<'_>,
    id: &RepairId,
    rec: &RepairRecord,
    plan: &[OpSpec],
    why: &str,
    locked: &RepairLock<'_>,
) -> Result<(), RepairError> {
    let mut applied_now: Vec<(String, String)> = Vec::new();
    for spec in plan {
        let live_now = TargetState::read(&env.pack.join(&spec.rel))?;
        if live_now == spec.incoming && spec.incoming != spec.expected {
            applied_now.push((spec.rel.clone(), live_now.canon()));
        }
    }
    let (gen_version, gen_pro) = plan.first().map(|s| s.bound.clone()).unwrap_or(Bound::Unknown).row_fields();
    let root = env.root.to_path_buf();
    let lane = env.lane.clone();
    let now = env.now;
    let idc = id.clone();
    let request = rec.approval.as_ref().map(|a| a.request_id.clone());
    let why = why.to_string();
    locked(id, &mut || {
        let _mutex_scope = super::guard::MutexScope::enter();
        let previous = ledger::read_lane_in(&root, &idc, &lane)?;
        let row = ledger::LaneRecord {
            schema: super::REPAIR_SCHEMA,
            repair: idc.clone(),
            lane: lane.clone(),
            state: LaneState::Conflict,
            probation: Some(ledger::Probation {
                pack_generation: gen_version.clone(),
                pro_revision: gen_pro,
                applied_at: now,
                expected: applied_now.clone(),
                closed: None,
                check_timings: Vec::new(),
                check_timeouts: Vec::new(),
            }),
            merge: None,
            merge_history: previous.as_ref().map(|p| p.merge_history.clone()).unwrap_or_default(),
            approval_ref: request.clone(),
            conflict_live: applied_now.clone(),
            applied: applied_now.clone(),
            applied_history: previous.as_ref().map(|p| p.applied_history.clone()).unwrap_or_default(),
            merge_base: Vec::new(),
            merge_lock: None,
            merge_attempt: 0,
            updated_at: now,
        };
        ledger::write_lane_in(&root, &row)?;
        if let Some(mut r) = ledger::read_record_in(&root, &idc)? {
            if let Some(a) = r.approval.as_mut() {
                a.approved = false;
            }
            r.methods.push(format!("충돌 — 적용을 끝내지도 다 되돌리지도 못했습니다(되돌리기 가능: cys repair revert {}) · {why}", idc.as_str()).chars().take(300).collect());
            ledger::write_record_durable_in(&root, &r)?;
        }
        Ok(())
    })
}

/// AY5: a declined apply in the origin — the approval is void and the repair returns to `진행 중`.
fn void_approval(
    env: &OpEnv<'_>,
    id: &RepairId,
    why: &str,
    locked: &RepairLock<'_>,
) -> Result<(), RepairError> {
    let root = env.root.to_path_buf();
    let idc = id.clone();
    locked(id, &mut || {
        let _mutex_scope = super::guard::MutexScope::enter();
        if let Some(mut r) = ledger::read_record_in(&root, &idc)? {
            if !matches!(r.state, RepairState::Probation | RepairState::InUse) {
                r.approval = None;
                r.pending_request = None;
                r.state = RepairState::InProgress;
                r.methods.push(format!("적용 거절(승인 무효): {why}").chars().take(300).collect());
                ledger::write_record_durable_in(&root, &r)?;
            }
        }
        Ok(())
    })
}

fn mark_reverted_after_apply_check(
    env: &OpEnv<'_>,
    id: &RepairId,
    why: &str,
    locked: &RepairLock<'_>,
) -> Result<(), RepairError> {
    let root = env.root.to_path_buf();
    let idc = id.clone();
    locked(id, &mut || {
        let _mutex_scope = super::guard::MutexScope::enter();
        if let Some(mut r) = ledger::read_record_in(&root, &idc)? {
            r.state = RepairState::Reverted;
            // DX2: an undone apply never keeps its approval
            if let Some(a) = r.approval.as_mut() {
                a.approved = false;
            }
            r.methods.push(format!("적용 직후 검사 실패로 되돌림: {why}").chars().take(300).collect());
            ledger::write_record_durable_in(&root, &r)?;
        }
        Ok(())
    })
}

/// AK4 / C1 1.6 at the origin's approved apply: every OLDER repair of this lane in force on one of
/// the same targets is superseded (each under its own lock).
fn supersede_older_in(
    root: &Path,
    lane: &LaneKey,
    newer: &RepairRecord,
    locked: &RepairLock<'_>,
) -> Result<Vec<RepairId>, RepairError> {
    let targets: Vec<String> = ledger::file_targets(newer).map(|f| f.rel.clone()).collect();
    let mut done_ids = Vec::new();
    let mut older: Vec<RepairId> = ledger::list_ids_in(root).into_iter().filter(|o| *o < newer.id).collect();
    older.sort();
    older.reverse();
    for oid in older {
        let root = root.to_path_buf();
        let lane = lane.clone();
        let targets = targets.clone();
        let mut changed = false;
        locked(&oid, &mut || {
        let _mutex_scope = super::guard::MutexScope::enter();
            let Some(mut r) = ledger::read_record_in(&root, &oid)? else {
                return Ok(());
            };
            if r.origin_lane == lane
                && matches!(r.state, RepairState::Probation | RepairState::InUse)
                && r.files.iter().any(|f| targets.contains(&f.rel))
            {
                r.state = RepairState::Superseded;
                ledger::write_record_durable_in(&root, &r)?;
                changed = true;
            }
            Ok(())
        })?;
        if changed {
            done_ids.push(oid);
        }
    }
    Ok(done_ids)
}

/// The approved apply (T0 entry point; T5b's approve path calls it — or [`apply_candidate_in`]
/// with the daemon's mutex). Takes the pack-apply lock.
pub fn apply_candidate(
    id: &RepairId,
    lane: &LaneKey,
    files: &[CandidateFile],
) -> Result<ApplyOutcome, RepairError> {
    let root = super::paths::repairs_root();
    let pack = crate::pack::pack_dir();
    let env = OpEnv {
        root: &root,
        pack: &pack,
        lane,
        now: ledger::now_epoch(),
        form: super::platform::apply_form(),
        faults: &NoFaults,
        own_check: &|_, _| None,
    };
    let screen = production_screen_step(&root);
    let lock = crate::pack_apply_lock::pack_apply_lock_path_for(&pack);
    crate::pack_apply_lock::with_apply_lock(&lock, || {
        apply_candidate_in(
            &env,
            id,
            files,
            &screen,
            &production_check,
            &production_reference,
            &no_repair_lock,
        )
        .map(|r| r.outcome)
    })
    .map_err(RepairError::Io)?
}

// ───────────────────────────── the hook step (RULINGS X1, Y1, Y2, AW6, AK5) ─────────────────────────────

/// The hook step of RULINGS Y1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookStep {
    Unregister,
    Reregister,
}

/// The registrar mode's report (RULINGS Y2(a)).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconcileReport {
    pub hooks: Vec<ReconcileHook>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconcileHook {
    pub event: String,
    pub script: String,
    pub lifeline: bool,
    /// `registered` | `unregistered` | `unknown`
    pub desired: String,
    /// (settings path, `registered` | `unregistered` | `refused:<reason>`)
    pub targets: Vec<(PathBuf, String)>,
}

/// The deadline of one registrar run (the child is killed and reaped at it).
pub const RECONCILE_DEADLINE: Duration = Duration::from_secs(60);

/// Runs the lane's registrar in its narrow mode (`python3 <pack>/bin/javis_preflight.py
/// --reconcile-hooks --json [--dry-run]`) with the daemon's (= the lane's) environment, a bounded
/// deadline (killed + reaped), a hidden console on Windows. Returns its stdout.
pub type RegistrarFn<'a> = dyn Fn(&Path, bool) -> Result<String, RepairError> + 'a;

/// Production registrar runner.
pub fn run_registrar(pack: &Path, dry_run: bool) -> Result<String, RepairError> {
    let script = pack.join("bin").join("javis_preflight.py");
    // SEAL-1: the one factory for a direct python spawn (PYTHONDONTWRITEBYTECODE — never a new
    // unsealed spawn site; `bundled_python_spawn_sites_are_enumerated_and_sealed`)
    let mut cmd = crate::python_command("python3");
    cmd.arg(&script).arg("--reconcile-hooks").arg("--json");
    if dry_run {
        cmd.arg("--dry-run");
    }
    if let Some(exe_dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)) {
        for (k, v) in crate::spawn_env_pairs_from_process(&exe_dir) {
            cmd.env(k, v);
        }
    }
    cmd.env("CYS_PACK_DIR", pack);
    // U-7: the one spawn-policy helper. CT6: its OWN process group (`GroupScoped` — a new session on
    // unix, a new process group on Windows, the console hidden) so that `run_bounded` can stop the
    // registrar AND every descendant at the deadline; no autostart of a rival daemon from it.
    {
        use crate::SpawnPolicy as _;
        cmd.spawn_policy(crate::ChildLifetime::GroupScoped).no_autostart();
    }
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    run_bounded(cmd, RECONCILE_DEADLINE)
}

/// RULINGS CT6 / CZ10 / DR3 — THE bounded child runner of the job and of every apply (the
/// registrar). It NEVER blocks: ONE loop with ONE deadline reads every pipe without blocking (unix:
/// `O_NONBLOCK`; Windows: a detached reader thread whose chunks the loop takes without waiting) and
/// polls the child's exit (`try_wait`). Whenever the child is still running at `deadline − grace`,
/// its output has not ended, or — after a SUCCESSFUL exit too — members of its process group are
/// still alive, the group is killed (`killpg(SIGKILL)`; the child itself as well) and the loop polls
/// the exit and drains the output for the short fixed grace; then it RETURNS in every case, saying
/// truthfully what it could not kill or reap — a child it could not reap is left to a detached reap
/// thread (the caller never waits for it) — and what it received. The child should lead its own
/// process group (spawn policy `GroupScoped`). Windows residual: descendants are not killed there.
pub fn run_bounded(mut cmd: std::process::Command, deadline: Duration) -> Result<String, RepairError> {
    let start = std::time::Instant::now();
    let grace = (deadline / 4).min(Duration::from_millis(1500));
    let run_until = deadline.saturating_sub(grace);
    let mut child = cmd
        .spawn()
        .map_err(|e| RepairError::Io(format!("등록기를 시작할 수 없습니다: {e}")))?;
    let pid = child.id();
    // measured NOW (spawn returns after exec — the session already made): once the child is reaped
    // its pid no longer answers, but its group still exists under that id
    let leads = leads_own_group(pid);
    // EJ2: non-blocking mode on EVERY pipe, or the child does not run — killed at once
    let pipes = OutPipe::new(child.stdout.take().map(PipeEnd::Out)).and_then(|o| Ok((o, OutPipe::new(child.stderr.take().map(PipeEnd::Err))?)));
    let (mut pipe, mut errp) = match pipes {
        Ok(p) => p,
        Err(why) => {
            let mut problems = vec![format!("출력 파이프를 비차단으로 만들 수 없어 실행하지 않고 바로 멈췄습니다: {why}")];
            if leads {
                if let Err(e) = kill_group_of(pid) {
                    problems.push(format!("프로세스 그룹(pgid {pid})을 멈추지 못했습니다: {e}"));
                }
            }
            if let Err(e) = child.kill() {
                problems.push(format!("자식(pid {pid})을 멈추지 못했습니다: {e}"));
            }
            let reaped = (0..50).any(|_| {
                let done = matches!(child.try_wait(), Ok(Some(_)));
                if !done {
                    std::thread::sleep(Duration::from_millis(10));
                }
                done
            });
            if !reaped {
                let _ = std::thread::Builder::new().name("cys-bounded-reap".into()).spawn(move || {
                    let _ = child.wait();
                });
                problems.push(format!("자식(pid {pid})을 바로 거두지 못해 따로 거두게 두었습니다"));
            }
            return Err(RepairError::CheckFailed(problems.join(" · ")));
        }
    };
    let mut status: Option<std::process::ExitStatus> = None;
    let mut problems: Vec<String> = Vec::new();
    loop {
        let read = pipe.pump() + errp.pump();
        if status.is_none() {
            match child.try_wait() {
                Ok(st) => status = st,
                Err(e) => {
                    problems.push(format!("자식 상태를 읽을 수 없습니다: {e}"));
                    break;
                }
            }
        }
        if status.is_some() && pipe.eof && errp.eof {
            break;
        }
        if start.elapsed() >= run_until {
            break;
        }
        if read == 0 {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    let timed_out = status.is_none();
    let group_alive = leads && group_has_members(pid);
    if status.is_none() || !pipe.eof || !errp.eof || group_alive || !problems.is_empty() {
        if leads {
            match kill_group_of(pid) {
                Ok(()) if group_alive && status.is_some() => {
                    problems.push(format!("자식은 끝났지만 그 프로세스 그룹(pgid {pid})에 자손이 남아 있어 멈췄습니다"))
                }
                Ok(()) => {}
                Err(e) => problems.push(format!("프로세스 그룹(pgid {pid})을 멈추지 못했습니다: {e}")),
            }
        }
        if status.is_none() {
            if let Err(e) = child.kill() {
                problems.push(format!("자식(pid {pid})을 멈추지 못했습니다: {e}"));
            }
        }
        // the short fixed grace: poll the exit and drain the output — never past the deadline
        while start.elapsed() < deadline {
            let read = pipe.pump() + errp.pump();
            if status.is_none() {
                if let Ok(Some(st)) = child.try_wait() {
                    status = Some(st);
                }
            }
            if status.is_some() && pipe.eof && errp.eof {
                break;
            }
            if read == 0 {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        if !pipe.eof || !errp.eof {
            problems.push("출력 파이프를 그룹 밖의 자손이 아직 쥐고 있어 기한에 읽기를 그만두었습니다".to_string());
        }
    }
    let got = String::from_utf8_lossy(&pipe.buf).to_string();
    let shown: String = got.chars().take(400).collect();
    // EJ2: the cap on kept output is reported, never hidden
    if pipe.dropped > 0 || errp.dropped > 0 {
        problems.push(format!(
            "출력이 한도({}바이트)를 넘어 표준출력 {}바이트·표준오류 {}바이트를 세기만 하고 버렸습니다",
            OUTPUT_KEEP_CAP, pipe.dropped, errp.dropped
        ));
    }
    let Some(st) = status else {
        // not reaped within the deadline: a detached reap, never waited for by this thread
        let _ = std::thread::Builder::new()
            .name("cys-bounded-reap".into())
            .spawn(move || {
                let _ = child.wait();
            });
        problems.push(format!("자식(pid {pid})을 기한 안에 거두지 못해 따로 거두게 두었습니다(기다리지 않음)"));
        return Err(RepairError::CheckTimeout(format!(
            "등록기가 {}초 안에 끝나지 않았습니다 — {} — 받은 출력 {}바이트: {shown}",
            deadline.as_secs(),
            problems.join(" · "),
            got.len()
        )));
    };
    if timed_out {
        return Err(RepairError::CheckTimeout(format!(
            "등록기가 {}초 안에 끝나지 않아 멈추고 거두었습니다(종료 {:?}){}{} — 받은 출력 {}바이트: {shown}",
            deadline.as_secs(),
            st.code(),
            if problems.is_empty() { "" } else { " — " },
            problems.join(" · "),
            got.len()
        )));
    }
    if !problems.is_empty() {
        return Err(RepairError::CheckFailed(format!(
            "등록기 실행이 깨끗하게 끝나지 않았습니다(종료 {:?}) — {} — 받은 출력 {}바이트: {shown}",
            st.code(),
            problems.join(" · "),
            got.len()
        )));
    }
    if !st.success() {
        return Err(RepairError::CheckFailed(format!("등록기가 실패했습니다(종료 {:?})", st.code())));
    }
    Ok(got)
}

/// EJ2: at most this many bytes are read from ONE pipe in one turn of the runner's loop.
pub const PIPE_TURN_BYTES: usize = 64 * 1024;
/// EJ2: at most this many bytes of a pipe's output are kept; the rest is counted and dropped.
pub const OUTPUT_KEEP_CAP: usize = 256 * 1024;

#[cfg(test)]
thread_local! {
    /// Test-only (EJ2): setting a pipe non-blocking fails.
    pub(crate) static FAIL_NONBLOCK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// One end of the child's output.
enum PipeEnd {
    Out(std::process::ChildStdout),
    Err(std::process::ChildStderr),
}

impl std::io::Read for PipeEnd {
    fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
        match self {
            PipeEnd::Out(p) => p.read(b),
            PipeEnd::Err(p) => p.read(b),
        }
    }
}

/// A child's pipe read WITHOUT blocking and WITH bounds (DR3 / EJ2).
struct OutPipe {
    #[cfg(unix)]
    pipe: Option<PipeEnd>,
    #[cfg(not(unix))]
    rx: Option<std::sync::mpsc::Receiver<Option<Vec<u8>>>>,
    buf: Vec<u8>,
    dropped: u64,
    eof: bool,
}

impl OutPipe {
    /// `Err` when the pipe cannot be made non-blocking (the runner then does not run the child).
    fn new(end: Option<PipeEnd>) -> Result<OutPipe, String> {
        let eof = end.is_none();
        #[cfg(unix)]
        {
            if let Some(e) = &end {
                use std::os::unix::io::AsRawFd;
                let fd = match e {
                    PipeEnd::Out(p) => p.as_raw_fd(),
                    PipeEnd::Err(p) => p.as_raw_fd(),
                };
                #[cfg(test)]
                if FAIL_NONBLOCK.with(|c| c.get()) {
                    return Err("O_NONBLOCK 설정 실패(시험 주입)".into());
                }
                // SAFETY: fcntl on a descriptor we own for the lifetime of `e`.
                let ok = unsafe {
                    let fl = libc::fcntl(fd, libc::F_GETFL);
                    fl >= 0 && libc::fcntl(fd, libc::F_SETFL, fl | libc::O_NONBLOCK) == 0
                };
                if !ok {
                    return Err(format!("O_NONBLOCK 설정 실패: {}", std::io::Error::last_os_error()));
                }
            }
            Ok(OutPipe { pipe: end, buf: Vec::new(), dropped: 0, eof })
        }
        #[cfg(not(unix))]
        {
            let rx = end.map(|mut o| {
                // a bounded channel: the reader thread waits when the loop has not taken its chunks
                let (tx, rx) = std::sync::mpsc::sync_channel::<Option<Vec<u8>>>(8);
                let _ = std::thread::Builder::new().name("cys-bounded-pipe".into()).spawn(move || {
                    let mut chunk = vec![0u8; 8192];
                    loop {
                        match std::io::Read::read(&mut o, &mut chunk) {
                            Ok(0) | Err(_) => {
                                let _ = tx.send(None);
                                break;
                            }
                            Ok(n) => {
                                if tx.send(Some(chunk[..n].to_vec())).is_err() {
                                    break;
                                }
                            }
                        }
                    }
                });
                rx
            });
            Ok(OutPipe { rx, buf: Vec::new(), dropped: 0, eof })
        }
    }

    fn keep(&mut self, bytes: &[u8]) {
        let room = OUTPUT_KEEP_CAP.saturating_sub(self.buf.len());
        let take = room.min(bytes.len());
        self.buf.extend_from_slice(&bytes[..take]);
        self.dropped += (bytes.len() - take) as u64;
    }

    /// Takes at most [`PIPE_TURN_BYTES`] of output available NOW; never waits. Returns the bytes read.
    fn pump(&mut self) -> usize {
        if self.eof {
            return 0;
        }
        #[cfg(unix)]
        {
            let Some(p) = self.pipe.as_mut() else {
                self.eof = true;
                return 0;
            };
            let mut chunk = vec![0u8; PIPE_TURN_BYTES];
            loop {
                match std::io::Read::read(p, &mut chunk) {
                    Ok(0) => {
                        self.eof = true;
                        return 0;
                    }
                    Ok(n) => {
                        let got = chunk[..n].to_vec();
                        self.keep(&got);
                        return n;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return 0,
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => {
                        self.eof = true;
                        return 0;
                    }
                }
            }
        }
        #[cfg(not(unix))]
        {
            let mut read = 0usize;
            while read < PIPE_TURN_BYTES {
                let Some(rx) = self.rx.as_ref() else {
                    self.eof = true;
                    return read;
                };
                match rx.try_recv() {
                    Ok(Some(c)) => {
                        read += c.len();
                        self.keep(&c);
                    }
                    Ok(None) | Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        self.eof = true;
                        return read;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => return read,
                }
            }
            read
        }
    }
}

/// Whether the process group `pgid` still has a member (unix; `false` elsewhere).
fn group_has_members(pgid: u32) -> bool {
    #[cfg(unix)]
    {
        let Ok(p) = i32::try_from(pgid) else {
            return false;
        };
        // SAFETY: signal 0 only probes the group.
        let rc = unsafe { libc::killpg(p, 0) };
        rc == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
    #[cfg(not(unix))]
    {
        let _ = pgid;
        false
    }
}

#[cfg(test)]
thread_local! {
    /// Test-only (DR3): the next group kills fail with this errno (0 = off).
    pub(crate) static FAIL_GROUP_KILL: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
}

/// Whether `pid` leads its own process group (never the daemon's own group).
fn leads_own_group(pid: u32) -> bool {
    #[cfg(unix)]
    {
        let Ok(p) = i32::try_from(pid) else {
            return false;
        };
        // SAFETY: getpgid / getpgrp take and return plain integers.
        unsafe { libc::getpgid(p) == p && p != libc::getpgrp() }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

/// Kills the process group `pid` led at spawn (unix); the error is returned, never swallowed. A
/// group that has no member any more (ESRCH) is not an error.
fn kill_group_of(pid: u32) -> Result<(), String> {
    #[cfg(test)]
    {
        let e = FAIL_GROUP_KILL.with(|c| c.get());
        if e != 0 {
            return Err(std::io::Error::from_raw_os_error(e).to_string());
        }
    }
    #[cfg(unix)]
    {
        let p = i32::try_from(pid).map_err(|e| e.to_string())?;
        // SAFETY: killpg takes plain integers; the group was measured to be the child's own.
        if unsafe { libc::killpg(p, libc::SIGKILL) } != 0 {
            let e = std::io::Error::last_os_error();
            if e.raw_os_error() != Some(libc::ESRCH) {
                return Err(e.to_string());
            }
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        Ok(())
    }
}

/// Parses the registrar's Y2(a) JSON into a report.
pub fn parse_reconcile(stdout: &str) -> Result<ReconcileReport, RepairError> {
    let v: serde_json::Value = serde_json::from_str(stdout.trim())
        .map_err(|e| RepairError::CheckFailed(format!("등록기 보고를 읽을 수 없습니다: {e}")))?;
    let arr = v
        .get("hooks")
        .and_then(|h| h.as_array())
        .ok_or_else(|| RepairError::CheckFailed("등록기 보고에 hooks 가 없습니다".into()))?;
    let mut hooks = Vec::new();
    for h in arr {
        let s = |k: &str| h.get(k).and_then(|x| x.as_str()).unwrap_or_default().to_string();
        let targets = h
            .get("targets")
            .and_then(|t| t.as_array())
            .map(|ts| {
                ts.iter()
                    .map(|t| {
                        (
                            PathBuf::from(t.get("path").and_then(|p| p.as_str()).unwrap_or_default()),
                            t.get("state").and_then(|p| p.as_str()).unwrap_or_default().to_string(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        hooks.push(ReconcileHook {
            event: s("event"),
            script: s("script"),
            lifeline: h.get("lifeline").and_then(|x| x.as_bool()).unwrap_or(false),
            desired: s("desired"),
            targets,
        });
    }
    Ok(ReconcileReport { hooks })
}

/// RULINGS DR8 — a dry run is a MEASUREMENT only when it names EVERY requested hook (exact event
/// and script) with its complete captured state: a `desired` of `registered` / `unregistered` and a
/// `targets` array whose every entry has a `path` and a `state`. A syntactically valid answer that
/// is empty or incomplete is refused exactly like an error.
pub fn dry_run_measurement(stdout: &str, named: &[(String, String)]) -> Result<ReconcileReport, RepairError> {
    let report = parse_reconcile(stdout)?;
    let raw: serde_json::Value = serde_json::from_str(stdout.trim())
        .map_err(|e| RepairError::CheckFailed(format!("등록기 보고를 읽을 수 없습니다: {e}")))?;
    let rows = raw.get("hooks").and_then(|h| h.as_array()).cloned().unwrap_or_default();
    for (event, script) in named {
        let row = rows.iter().find(|h| {
            h.get("event").and_then(|x| x.as_str()) == Some(event.as_str())
                && h.get("script").and_then(|x| x.as_str()) == Some(script.as_str())
        });
        let Some(row) = row else {
            return Err(RepairError::CheckFailed(format!("예행 보고에 요청한 훅 {event}:{script} 가 없습니다")));
        };
        let desired_ok = matches!(row.get("desired").and_then(|x| x.as_str()), Some("registered") | Some("unregistered"));
        let targets_ok = row.get("targets").and_then(|t| t.as_array()).is_some_and(|ts| {
            ts.iter().all(|t| t.get("path").and_then(|p| p.as_str()).is_some_and(|p| !p.is_empty()) && t.get("state").and_then(|p| p.as_str()).is_some_and(|p| !p.is_empty()))
        });
        if !desired_ok || !targets_ok {
            return Err(RepairError::CheckFailed(format!(
                "예행 보고의 훅 {event}:{script} 에 원하는 상태나 대상 목록이 온전하지 않습니다"
            )));
        }
    }
    Ok(report)
}

/// Y2(b): judges ONLY the named hooks — matched by EXACT event and script name (AW6: a case variant
/// matches nothing). Ok when every writable target of each named hook is in `want`; `refused:*`
/// targets are listed and do not fail it; a named hook the report does not list fails it.
pub fn judge_named(report: &ReconcileReport, named: &[(String, String)], want: &str) -> Result<(), String> {
    for (event, script) in named {
        let Some(h) = report.hooks.iter().find(|h| h.event == *event && h.script == *script) else {
            return Err(format!("settings:{event}:{script}: 등록기 보고에 없는 훅입니다(이름이 정확히 같아야 합니다)"));
        };
        for (path, st) in &h.targets {
            if st.starts_with("refused:") {
                continue;
            }
            if st != want {
                return Err(format!(
                    "settings:{event}:{script}: {} 이(가) '{st}' 입니다(기대 '{want}')",
                    path.display()
                ));
            }
        }
    }
    Ok(())
}

/// RULINGS X1 / Y2(b): runs `javis_preflight.py --reconcile-hooks --json` for the lane and judges
/// only the named hooks — the desired state of each named hook is read from the report itself.
pub fn reconcile_hook_registrations_with(
    pack: &Path,
    hooks: &[(String, String)],
    registrar: &RegistrarFn<'_>,
) -> Result<ReconcileReport, RepairError> {
    let out = registrar(pack, false)?;
    let report = parse_reconcile(&out)?;
    for (event, script) in hooks {
        let want = report
            .hooks
            .iter()
            .find(|h| h.event == *event && h.script == *script)
            .map(|h| h.desired.clone())
            .unwrap_or_default();
        if want != "registered" && want != "unregistered" {
            return Err(RepairError::CheckFailed(format!(
                "settings:{event}:{script}: 등록기가 원하는 상태를 정하지 못했습니다('{want}')"
            )));
        }
        judge_named(&report, &[(event.clone(), script.clone())], &want).map_err(RepairError::CheckFailed)?;
    }
    Ok(report)
}

/// RULINGS X1 / Y2(b): the lane's registrar run and judged for the named hooks.
pub fn reconcile_hook_registrations(
    lane: &LaneKey,
    hooks: &[(String, String)],
) -> Result<ReconcileReport, RepairError> {
    let _ = lane; // the daemon serving the lane runs this: its pack is the lane's pack
    reconcile_hook_registrations_with(&crate::pack::pack_dir(), hooks, &run_registrar)
}

/// The settings entries as found (matcher / timeout) for one hook line — read only (Y1).
fn found_matcher_timeout(settings: &Path, event: &str, script: &str) -> (Option<String>, Option<u64>) {
    let Ok(bytes) = std::fs::read(settings) else {
        return (None, None);
    };
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return (None, None);
    };
    let Some(groups) = v.get("hooks").and_then(|h| h.get(event)).and_then(|e| e.as_array()) else {
        return (None, None);
    };
    for g in groups {
        for h in g.get("hooks").and_then(|x| x.as_array()).into_iter().flatten() {
            let cmd = h.get("command").and_then(|c| c.as_str()).unwrap_or_default();
            if cmd.split(['/', '\\', '"', ' ', '\'']).any(|part| part == script) {
                return (
                    g.get("matcher").and_then(|m| m.as_str()).map(str::to_string),
                    h.get("timeout").and_then(|t| t.as_u64()),
                );
            }
        }
    }
    (None, None)
}

/// RULINGS Y1: the ONE hook step — (i) the operation record FIRST (targets from a dry run, matcher /
/// timeout as found), (ii) the tombstone through T3's writer, (iii) the registrar, (iv) the record
/// finished. Takes NO repair mutex and assumes none (AK5: `commit` calls it after releasing it). An
/// unfinished record is resumed by the next tick ([`resume_hook_steps_in`]).
pub fn hook_lines_step_in(
    root: &Path,
    pack: &Path,
    id: &RepairId,
    lane: &LaneKey,
    step: HookStep,
    registrar: &RegistrarFn<'_>,
    now: f64,
) -> Result<ReconcileReport, RepairError> {
    let rec = ledger::read_record_in(root, id)?
        .ok_or_else(|| RepairError::Precondition(format!("{}: 그런 수리가 없습니다", id.as_str())))?;
    let named: Vec<(String, String)> = ledger::hook_lines(&rec)
        .filter_map(|f| match &f.kind {
            ledger::ChangeKind::HookUnregister { event, script } => Some((event.clone(), script.clone())),
            _ => None,
        })
        .collect();
    if named.is_empty() {
        return Ok(ReconcileReport { hooks: Vec::new() });
    }
    // (i) the record first, with the targets of a dry run (matcher / timeout as found). CT3: a dry
    // run that cannot be measured DECLINES before anything is recorded or changed (the caller marks
    // the lane row 충돌; nothing was unregistered without the captured registration state)
    let dry = registrar(pack, true)
        .and_then(|s| dry_run_measurement(&s, &named))
        .map_err(|e| {
            RepairError::CheckFailed(format!(
                "{}: 훅 등록기의 예행을 측정할 수 없어 훅 단계를 하지 않았습니다(아무것도 바꾸지 않음) — {e:?}",
                id.as_str()
            ))
        })?;
    let lines: Vec<wal::HookLine> = named
        .iter()
        .map(|(event, script)| wal::HookLine {
            event: event.clone(),
            script: script.clone(),
            targets: dry
                .hooks
                .iter()
                .filter(|h| h.event == *event && h.script == *script)
                .flat_map(|h| h.targets.iter())
                .map(|(path, state)| {
                    let (matcher, timeout) = found_matcher_timeout(path, event, script);
                    wal::HookTarget {
                        path: path.clone(),
                        state: state.clone(),
                        matcher,
                        timeout,
                    }
                })
                .collect(),
        })
        .collect();
    let kind = match step {
        HookStep::Unregister => "hook-unregister",
        HookStep::Reregister => "hook-reregister",
    };
    let mut op = OpRecord {
        schema: super::REPAIR_SCHEMA,
        op: wal::new_op_id(now, kind),
        repair: id.clone(),
        lane_key: lane.clone(),
        kind: kind.to_string(),
        form: "hook".to_string(),
        rel: String::new(),
        pre_existed: false,
        expected: String::new(),
        incoming: String::new(),
        pack_generation: Some(pack_generation(pack)),
        slot: None,
        slot_identity: None,
        live_identity: None,
        incoming_copy: None,
        incoming_copy_identity: None,
        step: "recorded".to_string(),
        hooks: lines,
        finished: false,
        outcome: None,
        note: String::new(),
        created_at: now,
        updated_at: now,
        notice: None,
    };
    wal::write_in(root, &op)?;
    finish_hook_op(root, pack, &rec, &mut op, registrar, now)
}

fn finish_hook_op(
    root: &Path,
    pack: &Path,
    rec: &RepairRecord,
    op: &mut OpRecord,
    registrar: &RegistrarFn<'_>,
    now: f64,
) -> Result<ReconcileReport, RepairError> {
    let named: Vec<(String, String)> = op.hooks.iter().map(|h| (h.event.clone(), h.script.clone())).collect();
    // (ii) the tombstone (T3's writer; one lane-level mutex there)
    match op.kind.as_str() {
        "hook-unregister" => {
            for (event, script) in &named {
                super::tombstones::record_in(
                    root,
                    &op.lane_key,
                    &super::tombstones::Tombstone {
                        rel: format!("settings:{event}:{script}"),
                        kind: super::tombstones::TombstoneKind::Hook {
                            event: event.clone(),
                            script: script.clone(),
                        },
                        repair: rec.id.clone(),
                    },
                )?;
            }
        }
        // DR7: the hook step removes only this repair's HOOK tombstones (a file's goes only when
        // that file was restored — the file step)
        _ => super::tombstones::remove_hooks_for_repair_in(root, &op.lane_key, &rec.id)?,
    }
    wal::set_step(root, op, "tombstoned", now)?;
    // (iii) the registrar, judged for the named hooks
    let result = reconcile_hook_registrations_with(pack, &named, registrar);
    // (iv) finished — an error leaves no endless retry inside the step: the caller marks `충돌`
    match &result {
        Ok(_) => wal::finish(root, op, "applied", "", now)?,
        Err(e) => wal::finish(root, op, "declined", &format!("{e:?}"), now)?,
    }
    result
}

/// Resumes the lane's unfinished hook records (a crash after the record): the step is completed
/// from the record.
pub fn resume_hook_steps_in(
    root: &Path,
    pack: &Path,
    lane: &LaneKey,
    registrar: &RegistrarFn<'_>,
    locked: &RepairLock<'_>,
    now: f64,
) -> u32 {
    resume_hook_steps_report_in(root, pack, lane, registrar, locked, now).0
}

/// [`resume_hook_steps_in`] that also returns each resumed hook step that failed (EJ3: recorded by
/// the job like every recovery error).
pub fn resume_hook_steps_report_in(
    root: &Path,
    pack: &Path,
    lane: &LaneKey,
    registrar: &RegistrarFn<'_>,
    locked: &RepairLock<'_>,
    now: f64,
) -> (u32, Vec<(RepairId, String)>) {
    let mut errors = Vec::new();
    let mut n = 0;
    // EJ4: a hook step running now holds its repair's guard (from before its dry run — the commit
    // takes it at its top); recovery tries the guard and reads the hook records AFTER holding it
    let ids = match ledger::list_ids_checked_in(root) {
        Ok(ids) => ids,
        Err(e) => return (0, vec![(RepairId::parse("R-1").unwrap_or_else(|_| unreachable!()), format!("수리 장부 목록을 읽을 수 없습니다({e:?})"))]),
    };
    for id in ids {
        let Some(_guard) = super::guard::RepairGuard::try_take(root, &id) else {
            continue;
        };
        let ops = match wal::list_checked_in(root, &id) {
            Ok(ops) => ops,
            Err(e) => {
                errors.push((id.clone(), format!("훅 동작 기록을 읽을 수 없습니다({e})")));
                continue;
            }
        };
        for mut op in ops.into_iter().filter(|r| r.lane_key == *lane && !r.finished && r.form == "hook") {
        let Ok(Some(rec)) = ledger::read_record_in(root, &op.repair) else {
            continue;
        };
        match finish_hook_op(root, pack, &rec, &mut op, registrar, now) {
            Ok(_) => n += 1,
            Err(e) => {
                // CT7: a resumed hook operation that fails marks the lane 충돌 (as the first run would)
                mark_lane_conflict(root, &rec.id, lane, now, locked);
                errors.push((rec.id.clone(), format!("끊긴 훅 단계를 마치지 못했습니다({e:?})")));
                n += u32::from(op.finished);
            }
        }
        }
    }
    (n, errors)
}

/// The lane row of `id` → 충돌 (a hook step that could not complete — CT7).
pub fn mark_lane_conflict(root: &Path, id: &RepairId, lane: &LaneKey, now: f64, locked: &RepairLock<'_>) {
    let rootb = root.to_path_buf();
    let idc = id.clone();
    let lanec = lane.clone();
    let _ = locked(id, &mut || {
        let _mutex_scope = super::guard::MutexScope::enter();
        if let Some(mut row) = ledger::read_lane_in(&rootb, &idc, &lanec)? {
            row.state = LaneState::Conflict;
            row.updated_at = now;
            ledger::write_lane_in(&rootb, &row)?;
        }
        Ok(())
    });
}

/// RULINGS Y1: the ONE hook step (operation record first, tombstone, reconcile, record finished).
pub fn hook_lines_step(
    id: &RepairId,
    lane: &LaneKey,
    step: HookStep,
) -> Result<ReconcileReport, RepairError> {
    hook_lines_step_in(
        &super::paths::repairs_root(),
        &crate::pack::pack_dir(),
        id,
        lane,
        step,
        &run_registrar,
        ledger::now_epoch(),
    )
}

#[cfg(test)]
#[path = "apply_tests.rs"]
pub(crate) mod tests;
