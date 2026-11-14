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
    /// The pack generation the operation is bound to (`None` = the current one at step 0).
    pub generation: Option<PackGeneration>,
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
    if spec.expected == TargetState::Absent && spec.incoming != TargetState::Absent {
        std::fs::create_dir_all(&parent)
            .map_err(|e| RepairError::Io(format!("{}: {e}", parent.display())))?;
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
    let gen = pack_generation(env.pack);
    if let Some(bound) = &spec.generation {
        if *bound != gen {
            return Ok(done(
                ApplyOutcome::Superseded(format!(
                    "{}: 팩 세대가 바뀌었습니다({} → {}) — 옛 세대의 바이트는 되살리지 않습니다",
                    spec.rel, bound.version, gen.version
                )),
                None,
            ));
        }
    }
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
    if let Err(e) = exchange(live, &slot.payload()) {
        wal::finish(env.root, &mut rec, "declined", &format!("{e:?}"), env.now)?;
        return Ok(done(ApplyOutcome::Declined(format!("{}: {e:?}", spec.rel)), Some(&rec)));
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
    let gen_after = pack_generation(env.pack);
    let gen_ok = rec.pack_generation.as_ref() == Some(&gen_after);
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
                    generation: None,
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
                generation: None,
            };
            let r = run_op_depth(env, &relive, depth + 1)?;
            note.push_str(&format!(" · 밀려난 바이트의 사본을 다시 살렸습니다({:?})", r.outcome));
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
    if let Err(e) = place_no_replace(live, &slot.payload()) {
        wal::finish(env.root, &mut rec, "declined", &format!("{e:?}"), env.now)?;
        return Ok(done(ApplyOutcome::Declined(format!("{}: {e:?}", spec.rel)), Some(&rec)));
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
    let gen_ok = rec.pack_generation.as_ref() == Some(&pack_generation(env.pack));
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
                generation: None,
            };
            let r = run_op_depth(env, &relive, depth + 1)?;
            note.push_str(&format!(" · 사본을 그 자리에 다시 놓았습니다({:?})", r.outcome));
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
            generation: None,
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
    match place_no_replace(&copy, live) {
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
    if let Err(e) = place_no_replace(live, &slot.payload()) {
        let _ = std::fs::remove_file(&copy);
        wal::finish(env.root, &mut rec, "declined", &format!("{e:?}"), env.now)?;
        return Ok(done(ApplyOutcome::Declined(format!("{}: {e:?}", spec.rel)), Some(&rec)));
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
    let gen_ok = rec.pack_generation.as_ref() == Some(&pack_generation(env.pack));
    let copy_ok = match (&rec.incoming_copy, &rec.incoming_copy_identity) {
        (Some(c), Some(want)) => identity_of(c).ok().as_ref() == Some(want),
        _ => false,
    };
    let deposit_ok = deposit.as_ref().ok() == Some(&expected);
    if copy_ok && gen_ok && deposit_ok {
        // RESUME (OQ-D8): the staged copy still matches and the generation is unchanged
        let copy = rec.incoming_copy.clone().unwrap_or_default();
        match place_no_replace(&copy, live) {
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
                generation: None,
            };
            match run_op_depth(env, &back, 1) {
                Ok(r) => format!("{rel}: 끊긴 동작을 그만두고 보관한 바이트의 사본을 되놓았습니다({:?})", r.outcome),
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
        generation: rec.pack_generation.clone(),
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
    let mut n = 0u32;
    for rec in wal::unfinished_for_lane(env.root, env.lane) {
        if rec.form == "hook" {
            continue; // hook records are finished by the hook step's own resume
        }
        match resume_op(env, rec) {
            Ok(_) => n += 1,
            Err(e) if format!("{e:?}").contains(INJECTED_STOP) => return Err(e),
            Err(_) => {} // left for the next run (nothing is forced)
        }
    }
    Ok(n)
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
    let closed_for_good = matches!(row.probation.as_ref().and_then(|p| p.closed.as_deref()), Some(c) if c.starts_with("되돌려짐") || c.starts_with("대체됨"));
    // CT3 / CT4: a lane row 충돌 left by an apply or a revert that could not finish is revertible
    let unfinished_here = row.state == LaneState::Conflict && !closed_for_good && (!row.applied.is_empty() || has_hooks);
    let in_force = if rec.origin_lane == *env.lane {
        matches!(rec.state, RepairState::Probation | RepairState::InUse) || unfinished_here
    } else {
        (!row.applied.is_empty() || has_hooks) && !closed_for_good
    };
    // CT7: a hook-only repair (no file state) is revertible — its re-registration is its state
    if !in_force || (row.applied.is_empty() && !has_hooks) {
        return Err(RepairError::Precondition(format!(
            "{}: '{}' 상태라 되돌릴 것이 없습니다(시험 기간·사용 중인 수리만 되돌립니다)",
            id.as_str(),
            rec.state.label_ko()
        )));
    }
    let bound = row.probation.as_ref().map(|p| PackGeneration {
        version: p.pack_generation.clone(),
        pro_revision: p.pro_revision,
    });
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
        if let Some(b) = &bound {
            if *b != pack_generation(env.pack) {
                return Ok(close_superseded(
                    env,
                    &rec,
                    "팩 세대가 적용 때와 다릅니다 — 옛 세대의 바이트는 되살리지 않습니다(아무것도 되돌리지 않았습니다)",
                    locked,
                )?);
            }
        }
        let (pre, holder) = pre_image_for(env.root, &rec, env.lane, rel)?;
        plan.push(OpSpec {
            id: id.clone(),
            rel: rel.clone(),
            kind: "revert".into(),
            expected,
            incoming: pre,
            source: holder,
            generation: bound.clone(),
        });
    }
    let mut files = Vec::new();
    let mut all_applied = true;
    for spec in &plan {
        let r = run_op(env, spec)?;
        if r.outcome != ApplyOutcome::Applied {
            all_applied = false;
        }
        files.push((spec.rel.clone(), r.outcome));
    }
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
    let nothing_changed = !any_applied && hook_step.is_none() && !plan.is_empty();
    let label = if complete {
        "되돌려짐"
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
    let by_label = match by {
        RevertCaller::Human => "사람".to_string(),
        RevertCaller::TicketedWorker { seat } => format!("워커 좌석 {seat}"),
        RevertCaller::DaemonAuto => "데몬(자동)".to_string(),
    };
    let lane = env.lane.clone();
    let now = env.now;
    let root = env.root.to_path_buf();
    let idc = id.clone();
    locked(id, &mut || {
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

/// F9 (as widened by U1): the live state of `rel` in this lane is an approved copy of the CURRENT
/// generation only when THREE daemon-written things agree — (1) a repair record of class
/// boot-critical or protection-related with a daemon-written approval (approved) whose lane row is
/// in force and bound to the current pack generation; (2) the FINISHED, applied operation record of
/// that apply in this lane, whose incoming state is the live state now; (3) that apply's vault
/// deposit (slot `meta/pre.json` naming this repair / lane / rel, the sealed copy matching
/// `meta/sealed.json`). A ledger row alone is never enough.
pub fn approved_copy_of_current_generation_in(
    root: &Path,
    pack: &Path,
    lane: &LaneKey,
    rel: &str,
) -> Result<bool, RepairError> {
    let live = TargetState::read(&pack.join(rel))?;
    let gen = pack_generation(pack);
    for id in ledger::list_ids_in(root).into_iter().rev() {
        let Ok(Some(rec)) = ledger::read_record_in(root, &id) else {
            continue;
        };
        if !is_approval_class(rec.classification) {
            continue;
        }
        if !rec.approval.as_ref().is_some_and(|a| a.approved) {
            continue;
        }
        let Ok(Some(row)) = ledger::read_lane_in(root, &id, lane) else {
            continue;
        };
        let Some(p) = row.probation.as_ref() else {
            continue;
        };
        if p.closed.as_deref().is_some_and(|c| !c.starts_with("사용 중"))
            || p.pack_generation != gen.version
            || p.pro_revision != gen.pro_revision
        {
            continue;
        }
        if !row.applied.iter().any(|(r, c)| r == rel && *c == live.canon()) {
            continue;
        }
        // CT9 — the triple is an AGREEMENT: (i) the approval's fingerprint equals the fingerprint of
        // the operation set this lane applied, (ii) the lane row's approval reference is that
        // approval's, (iii) every applied operation's incoming state is the row's state of its file
        let approval = rec.approval.as_ref().unwrap_or_else(|| unreachable!());
        let applied_states: Option<Vec<(String, TargetState)>> =
            row.applied.iter().map(|(r, c)| parse_canon(c).map(|st| (r.clone(), st))).collect();
        let Some(applied_states) = applied_states else {
            continue;
        };
        if super::types::fingerprint(&applied_states) != approval.fingerprint_sha256 {
            continue;
        }
        if row.approval_ref.as_deref() != Some(approval.request_id.as_str()) {
            continue;
        }
        let ops_agree = row.applied.iter().all(|(r, c)| match applied_op_for(root, &id, lane, r) {
            Some(o) => o.incoming == *c,
            None => true, // a target the apply left unchanged has no operation
        });
        if !ops_agree {
            continue;
        }
        let Some(op) = applied_op_for(root, &id, lane, rel) else {
            continue;
        };
        if parse_canon(&op.incoming) != Some(live.clone()) {
            continue;
        }
        if op.expected == "absent" {
            // a created target: the operation record is the evidence (nothing was displaced)
            return Ok(true);
        }
        let Some(dir) = op.slot.as_deref() else {
            continue;
        };
        let slot = Slot::at(dir, rel);
        // (iv) the slot's metadata agrees with the operation's EXPECTED identity and state, and so
        // does the verified deposit
        let expected = parse_canon(&op.expected).unwrap_or(TargetState::Absent);
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
        let identity_ok = match (&op.live_identity, &expected) {
            (Some(i), TargetState::File { sha256, .. }) => i.sha256 == *sha256,
            _ => true,
        };
        if pre_ok && identity_ok && vault::deposit_of(&slot).is_ok_and(|(st, _)| st == expected) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// What the hourly scan knows about the owner-approved copy of `rel` for the CURRENT generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovedCopy {
    /// No approved repair of this generation applied `rel` in this lane.
    None,
    /// The verified payload of the last approved copy slot.
    Found(TargetState, PathBuf),
    /// An approved repair of this generation applied `rel` here, but its copy cannot be verified —
    /// the scan then puts NOTHING back for the file (CU2).
    Unverifiable(RepairId),
}

/// CU2 — the approved copy of the CURRENT generation for `rel` in this lane (§6-10 (2) "마지막
/// 승인본 저장소"): the NEWEST approved boot-critical / protection-related repair whose row is bound
/// to the current generation and applied `rel` here — in force, OR whose probation was closed as
/// superseded by a later change of the live file (the owner's approval of this generation still
/// stands); a repair that was reverted or failed no longer counts. Its applied operation's copy slot
/// must verify (identity + state); otherwise `Unverifiable`.
pub fn approved_copy_resolution_in(root: &Path, pack: &Path, lane: &LaneKey, rel: &str) -> ApprovedCopy {
    let gen = pack_generation(pack);
    for id in ledger::list_ids_in(root).into_iter().rev() {
        let Ok(Some(rec)) = ledger::read_record_in(root, &id) else {
            continue;
        };
        if !is_approval_class(rec.classification) || !rec.approval.as_ref().is_some_and(|a| a.approved) {
            continue;
        }
        let Ok(Some(row)) = ledger::read_lane_in(root, &id, lane) else {
            continue;
        };
        let Some(p) = row.probation.as_ref() else {
            continue;
        };
        if p.pack_generation != gen.version || p.pro_revision != gen.pro_revision {
            continue;
        }
        let stands = match p.closed.as_deref() {
            None => matches!(rec.state, RepairState::Probation | RepairState::InUse),
            Some(c) => c.starts_with("사용 중") || c.starts_with("대체됨"),
        };
        if !stands || !row.applied.iter().any(|(r, _)| r == rel) {
            continue;
        }
        // a ledger row alone is never evidence (F9): without the daemon's finished apply operation the
        // row is not an approved apply at all
        let Some(op) = applied_op_for(root, &id, lane, rel) else {
            continue;
        };
        let (Some(dir), Some(want)) = (op.incoming_copy.as_ref(), op.incoming_copy_identity.as_ref()) else {
            return ApprovedCopy::Unverifiable(id);
        };
        let slot = Slot::at(dir, rel);
        match TargetState::read(&slot.payload()) {
            Ok(st) if identity_of(&slot.payload()).ok().as_ref() == Some(want) && parse_canon(&op.incoming) == Some(st.clone()) => {
                return ApprovedCopy::Found(st, slot.payload());
            }
            _ => return ApprovedCopy::Unverifiable(id),
        }
    }
    ApprovedCopy::None
}

/// [`approved_copy_resolution_in`] as the verified copy only.
pub fn approved_copy_source_in(
    root: &Path,
    pack: &Path,
    lane: &LaneKey,
    rel: &str,
) -> Option<(TargetState, PathBuf)> {
    match approved_copy_resolution_in(root, pack, lane, rel) {
        ApprovedCopy::Found(st, p) => Some((st, p)),
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
    pub created_at: f64,
    pub updated_at: f64,
}

pub fn intent_path_in(root: &Path, id: &RepairId, lane: &LaneKey) -> PathBuf {
    ledger::repair_dir_in(root, id).join("intents").join(format!("{}.json", lane.0))
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
    for id in ledger::list_ids_in(env.root) {
        let Some(mut intent) = read_intent_in(env.root, &id, env.lane) else {
            continue;
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
    let row = ledger::read_lane_in(env.root, &id, env.lane)?;
    if matches!(rec.state, RepairState::Probation | RepairState::InUse)
        && row.as_ref().is_some_and(|r| r.probation.is_some() && r.state != LaneState::Conflict)
    {
        set_intent_state(env.root, intent, "applied", "기록과 부서 행이 이미 확정되어 있었습니다", env.now)?;
        return Ok(format!("{}: 끊긴 적용은 이미 끝나 있었습니다(의도 기록만 마저 씀)", id.as_str()));
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
            generation: None,
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
    // the protection screen at apply (AY5), over exactly the slots' bytes
    let view = vault::vault_dir_in(env.root, id).join(format!(".screen-{op_tag}"));
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
        screen(id, &cand, env.pack)
    })();
    let _ = std::fs::remove_dir_all(&view); // our own view of the slots' bytes (never a slot)
    if let Err(why) = screen_result {
        return decline(why);
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
                generation: None,
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
    let gen = pack_generation(env.pack);
    let lane = env.lane.clone();
    let root = env.root.to_path_buf();
    let now = env.now;
    let idc = id.clone();
    let request_id = approval.request_id.clone();
    locked(id, &mut || {
        let Some(mut r) = ledger::read_record_in(&root, &idc)? else {
            return Ok(());
        };
        r.committed = committed.clone();
        r.state = RepairState::Probation;
        r.lock = None;
        ledger::write_record_durable_in(&root, &r)?;
        let previous = ledger::read_lane_in(&root, &idc, &lane)?;
        let row = ledger::LaneRecord {
            schema: super::REPAIR_SCHEMA,
            repair: idc.clone(),
            lane: lane.clone(),
            state: LaneState::Verified,
            probation: Some(ledger::Probation {
                pack_generation: gen.version.clone(),
                pro_revision: gen.pro_revision,
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
    let gen = pack_generation(env.pack);
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
    if let Err(e) = exchange(&live, &slot.payload()) {
        wal::finish(env.root, &mut rec, "declined", &format!("{e:?}"), env.now)?;
        return Ok(done(ApplyOutcome::Declined(format!("{}: {e:?}", spec.rel)), Some(&rec)));
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
            generation: None,
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
    let gen = pack_generation(env.pack);
    let root = env.root.to_path_buf();
    let lane = env.lane.clone();
    let now = env.now;
    let idc = id.clone();
    let request = rec.approval.as_ref().map(|a| a.request_id.clone());
    let why = why.to_string();
    locked(id, &mut || {
        let previous = ledger::read_lane_in(&root, &idc, &lane)?;
        let row = ledger::LaneRecord {
            schema: super::REPAIR_SCHEMA,
            repair: idc.clone(),
            lane: lane.clone(),
            state: LaneState::Conflict,
            probation: Some(ledger::Probation {
                pack_generation: gen.version.clone(),
                pro_revision: gen.pro_revision,
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
        if let Some(mut r) = ledger::read_record_in(&root, &idc)? {
            r.state = RepairState::Reverted;
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
        .stderr(std::process::Stdio::null());
    run_bounded(cmd, RECONCILE_DEADLINE)
}

/// RULINGS CT6 / CZ10 — THE bounded child discipline of the job and of every apply (the registrar):
/// ONE deadline covers the child's run, the termination of its whole process group, the reaping and
/// the collection of its output. The child should lead its own process group (the caller's spawn
/// policy `GroupScoped`). After the child exits, its output must reach EOF before `deadline − reserve`
/// (`reserve` = ¼ of the deadline, at most 2 s); otherwise — or when the child itself is still running
/// then — the whole group is killed (unix `killpg(SIGKILL)`; Windows: the child only — residual),
/// the child reaped, and the output collected until the deadline; a descendant that escaped the
/// group and still holds the pipe cannot hold this call: the reader thread is left to it and the
/// call returns at the deadline. What was left behind is RETURNED in the error, never swallowed.
pub fn run_bounded(mut cmd: std::process::Command, deadline: Duration) -> Result<String, RepairError> {
    let start = std::time::Instant::now();
    let reserve = (deadline / 4).min(Duration::from_secs(2));
    let exit_by = deadline.saturating_sub(reserve);
    let mut child = cmd
        .spawn()
        .map_err(|e| RepairError::Io(format!("등록기를 시작할 수 없습니다: {e}")))?;
    let pid = child.id();
    // whether the child leads its own process group — measured NOW (spawn returns after exec, the
    // session already made): once the child is reaped its pid no longer answers, but its group
    // (the descendants) still exists under that id
    let leads = leads_own_group(pid);
    let out = child.stdout.take();
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let _ = std::thread::Builder::new().name("cys-bounded-reader".into()).spawn(move || {
        let mut s = String::new();
        if let Some(mut o) = out {
            let _ = std::io::Read::read_to_string(&mut o, &mut s);
        }
        let _ = tx.send(s);
    });
    let left = |d: Duration| d.saturating_sub(start.elapsed());
    // (1) the child's own run, until `exit_by`
    let mut status = None;
    while start.elapsed() < exit_by {
        match child.try_wait() {
            Ok(Some(st)) => {
                status = Some(st);
                break;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => {
                kill_group_of(pid, leads);
                let _ = child.kill();
                let _ = child.wait();
                return Err(RepairError::Io(format!("등록기 상태를 읽을 수 없어 그 프로세스 그룹을 멈췄습니다: {e}")));
            }
        }
    }
    // (2) its output, until `exit_by` as well
    let mut output = match status {
        Some(_) => rx.recv_timeout(left(exit_by)).ok(),
        None => None,
    };
    if status.is_some() && output.is_some() {
        let st = status.unwrap_or_else(|| unreachable!());
        let s = output.unwrap_or_default();
        if !st.success() {
            return Err(RepairError::CheckFailed(format!("등록기가 실패했습니다(종료 {:?})", st.code())));
        }
        return Ok(s);
    }
    // (3) termination of the whole group, reaping, and the rest of the output — within the deadline
    kill_group_of(pid, leads);
    let _ = child.kill();
    let reaped = child.wait().ok();
    if output.is_none() {
        output = rx.recv_timeout(left(deadline)).ok();
    }
    let got = output.as_deref().unwrap_or("");
    let shown: String = got.chars().take(400).collect();
    let pipe = if output.is_none() {
        " · 그룹 밖으로 나간 자손이 출력 파이프를 아직 쥐고 있어 읽기를 기다리지 않고 돌아왔습니다"
    } else {
        ""
    };
    if status.is_none() {
        return Err(RepairError::CheckTimeout(format!(
            "등록기가 {}초 안에 끝나지 않아 그 프로세스 그룹(pgid {pid})을 멈추고 거두었습니다(종료 {:?}){pipe} — 받은 출력 {}바이트: {shown}",
            deadline.as_secs(),
            reaped.and_then(|s| s.code()),
            got.len()
        )));
    }
    Err(RepairError::CheckFailed(format!(
        "등록기는 끝났지만 그 자손이 출력 파이프를 쥐고 있어 기한 안에 보고를 다 받지 못했습니다 — 프로세스 그룹(pgid {pid})을 멈췄습니다{pipe} — 받은 출력 {}바이트: {shown}",
        got.len()
    )))
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

/// Kills the process group `pid` led at spawn (unix, only when it did — never the daemon's group);
/// elsewhere nothing (the caller kills the child itself).
fn kill_group_of(pid: u32, leads: bool) {
    #[cfg(unix)]
    if leads {
        if let Ok(p) = i32::try_from(pid) {
            // SAFETY: killpg takes plain integers; the group was measured to be the child's own.
            unsafe {
                libc::killpg(p, libc::SIGKILL);
            }
        }
    }
    #[cfg(not(unix))]
    let _ = (pid, leads);
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
    let dry = registrar(pack, true).and_then(|s| parse_reconcile(&s)).map_err(|e| {
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
        _ => super::tombstones::remove_for_repair_in(root, &op.lane_key, &rec.id)?,
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
    let mut n = 0;
    for mut op in wal::unfinished_for_lane(root, lane).into_iter().filter(|r| r.form == "hook") {
        let Ok(Some(rec)) = ledger::read_record_in(root, &op.repair) else {
            continue;
        };
        if finish_hook_op(root, pack, &rec, &mut op, registrar, now).is_ok() {
            n += 1;
        } else {
            // CT7: a resumed hook operation that fails marks the lane 충돌 (as the first run would)
            mark_lane_conflict(root, &rec.id, lane, now, locked);
            n += u32::from(op.finished);
        }
    }
    n
}

/// The lane row of `id` → 충돌 (a hook step that could not complete — CT7).
pub fn mark_lane_conflict(root: &Path, id: &RepairId, lane: &LaneKey, now: f64, locked: &RepairLock<'_>) {
    let rootb = root.to_path_buf();
    let idc = id.clone();
    let lanec = lane.clone();
    let _ = locked(id, &mut || {
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
