//! The repair ledger's records (C1 1.2–1.19 · RULINGS G5: every field C1 fixes, so no track adds a
//! field by contract change) and its readers. T0: data, constants and stubs.

use super::boot_critical::CheckKind;
use super::types::{
    Classification, LaneKey, LaneState, Lease, RepairError, RepairId, RepairState, TargetState,
    Trigger,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The daemon-written approval result (RULINGS F9).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApprovalResult {
    pub request_id: String,
    pub fingerprint_sha256: String,
    pub decided_at: f64,
    pub approved: bool,
}

/// A file's pre-image recorded at `begin` (C1 1.7): a fresh vault slot, or `absent`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreImage {
    Slot {
        slot_dir: PathBuf,
        sha256: String,
        mode: u32,
        link_target: Option<String>,
    },
    Absent,
}

impl PreImage {
    /// The pre-image as a target STATE (RULINGS U2): a slot holding a link → `Link`, a slot holding
    /// a file → `File { sha256, mode }`, `absent` → `Absent`.
    pub fn state(&self) -> TargetState {
        match self {
            PreImage::Slot {
                link_target: Some(target),
                ..
            } => TargetState::Link {
                target: target.clone(),
            },
            PreImage::Slot {
                sha256,
                mode,
                link_target: None,
                ..
            } => TargetState::File {
                sha256: sha256.clone(),
                mode: *mode,
            },
            PreImage::Absent => TargetState::Absent,
        }
    }
}

/// Kind of one changed line of a record (C1 1.2). A hook unregistration is a GENERAL repair's line
/// (RULINGS V1) and is skipped by every file-target walker (W3(b)).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Edit,
    Create,
    Delete,
    Mode,
    Link,
    HookUnregister { event: String, script: String },
}

/// One changed file (C1 1.2, 1.8; RULINGS U2: `post` is the target's STATE). A hook line (W3(b)):
/// `rel = "settings:<event>:<script>"`, `pre` / `post` `Absent`, `kind = HookUnregister`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileChange {
    pub rel: String,
    pub kind: ChangeKind,
    pub pre: PreImage,
    pub post: TargetState,
    pub diff: Option<String>,
}

/// A role signature: worker record / reviewer1 verification / reviewer2 audit (C1 1.9–1.10);
/// `fingerprint_sha256` = the fingerprint of the bytes examined (RULINGS N1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Attestation {
    pub seat: u64,
    pub role: String,
    pub verdict: String,
    pub note: String,
    pub at: f64,
    pub fingerprint_sha256: String,
}

/// `repairs/R-<n>/record.json` — written only by the daemon that holds the repair's lock.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RepairRecord {
    pub schema: u32,
    pub id: RepairId,
    pub created_at: f64,
    pub trigger: Trigger,
    pub symptom: String,
    pub cause: String,
    pub classification: Classification,
    pub files: Vec<FileChange>,
    pub worker_record: Option<Attestation>,
    pub verify: Option<Attestation>,
    pub audit: Option<Attestation>,
    pub audit_due_at: Option<f64>,
    pub approval: Option<ApprovalResult>,
    pub owner_token_sha256: Option<String>,
    pub attempt: u32,
    pub methods: Vec<String>,
    /// (candidate fingerprint, requests for it that ended `expired`) — RULINGS S1, U4(c).
    pub resubmits: Vec<(String, u32)>,
    /// (fingerprint, expires_at), written at `submit` — RULINGS V2(d), W3(a).
    pub pending_request: Option<(String, f64)>,
    pub lock: Option<Lease>,
    pub state: RepairState,
    pub intake_signatures: Vec<String>,
    pub owner_original: bool,
    pub origin_lane: LaneKey,
    /// RULINGS R1 / T1 — a reference only, set by `begin --resolves`.
    pub resolves: Option<RepairId>,
    /// (rel, canon) — RULINGS R5, U2; written only by `commit` / the approved apply.
    pub committed: Vec<(String, String)>,
    /// Every seat that recorded in ANY round of this repair (RULINGS AD1): none of them may verify
    /// it (B2's "never the fixer" over all rounds). Appended at each `record`.
    #[serde(default)]
    pub fixer_seats: Vec<u64>,
    /// What was checked at commit, per target (RULINGS AD4): `(rel, how)` with `how` = the check
    /// kind that ran (`sh-syntax` …), `absent-state` for a committed deletion (the path is absent),
    /// `hook-registrar` for a hook line (the registrar's report inside `hook_lines_step`). A check
    /// that did not run is never listed.
    #[serde(default)]
    pub checks: Vec<(String, String)>,
    /// RULINGS AQ5 (T6): set on R′ when `cys repair reapply <R>` started it — `reapplies: R`
    /// (shown by `status`); `None` for every other repair. AR1: a second `reapply R` is refused while
    /// a repair with `reapplies: R` is unfinished or in force.
    #[serde(default)]
    pub reapplies: Option<RepairId>,
}

/// The probation bundle of one lane (C1 1.6, K4, N3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Probation {
    pub pack_generation: String,
    pub pro_revision: Option<u32>,
    pub applied_at: f64,
    /// (rel, canon) — RULINGS U2.
    pub expected: Vec<(String, String)>,
    pub closed: Option<String>,
    pub check_timings: Vec<CheckTiming>,
    /// (rel, at) — RULINGS N3.
    pub check_timeouts: Vec<(String, f64)>,
}

/// Recorded for EVERY target at commit / at the lane's apply (RULINGS K4 / P3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckTiming {
    pub rel: String,
    pub check_ms: u64,
    pub reference_kind: CheckKind,
    pub reference_ms: u64,
}

/// A lane's merge result (RULINGS H8).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergeRecord {
    pub by_seat: u64,
    /// (rel, canon) — RULINGS V2(c).
    pub result: Vec<(String, String)>,
    pub changed: bool,
    pub note: String,
    /// RULINGS R1 / R6: the lane's R′.
    pub via_repair: Option<RepairId>,
}

/// `repairs/R-<n>/lanes/<lane_key>.json` — written and created only by that lane's daemon.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LaneRecord {
    pub schema: u32,
    pub repair: RepairId,
    pub lane: LaneKey,
    pub state: LaneState,
    pub probation: Option<Probation>,
    pub merge: Option<MergeRecord>,
    /// Recorded merge results kept by the full reset (RULINGS L5(b)).
    pub merge_history: Vec<MergeRecord>,
    /// The origin's request id for an O1(a) apply (interpretation 37).
    pub approval_ref: Option<String>,
    /// (rel, canon) live states when 충돌 was recorded — RULINGS R1, U2.
    pub conflict_live: Vec<(String, String)>,
    /// (rel, canon) — RULINGS P2, U2.
    pub applied: Vec<(String, String)>,
    pub applied_history: Vec<Vec<(String, String)>>,
    /// (rel, canon) at `begin --joins` — RULINGS P5, V2(c).
    pub merge_base: Vec<(String, String)>,
    pub merge_lock: Option<Lease>,
    pub merge_attempt: u32,
    pub updated_at: f64,
    /// RULINGS IK2 (T7b) — why the row is in its state, in a person's words (the propagation's reason
    /// for a 충돌, a 대기 or a 검증됨); shown by `status`, printed in the merge ticket. Absent in rows
    /// written before it (empty — the serde default), and not written while empty.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reason: String,
}

/// `repairs/lanes/<lane_key>.job.json` (O-C7-4, RULINGS I7, I8, L1, O4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobState {
    pub schema: u32,
    pub lane: LaneKey,
    pub last_incarnation: String,
    pub last_probation_check: f64,
    pub last_sweep: f64,
    pub initial_catchup_complete: bool,
    pub pack_instance: Option<String>,
    pub pack_dir: Option<PathBuf>,
    pub pack_dir_identity: Option<DirIdentity>,
    pub pack_generation: Option<PackGeneration>,
}

/// The pack directory's own identity (RULINGS L1 — E6 without a content hash).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirIdentity {
    pub volume_uuid: String,
    pub inode: u64,
}

/// The pack generation pair of C1 1.6 (RULINGS O4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackGeneration {
    pub version: String,
    pub pro_revision: Option<u32>,
}

/// What a seat holds (for the [RESUME] line, C6 6.6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldRepair {
    pub id: RepairId,
    pub state: RepairState,
    pub attempt: u32,
    pub methods: Vec<String>,
}

/// Reads one repair record (O-C1-6/7: unreadable or of a higher schema → `Corrupt`; a missing
/// record → `Precondition`, the daemon answers `not_found` itself — RULINGS Z1).
pub fn load_record(id: &RepairId) -> Result<RepairRecord, RepairError> {
    match read_record_in(&super::paths::repairs_root(), id)? {
        Some(r) => Ok(r),
        None => Err(RepairError::Precondition(format!(
            "{}: 그런 수리가 없습니다",
            id.as_str()
        ))),
    }
}

/// The repair a seat of a lane holds, if any (for the [RESUME] line, C6 6.6): the newest record
/// whose lock owner is that lane and seat and whose lease has not passed its expiry + grace.
/// (The CLI cannot see the daemon's incarnation, so a lock released by a restart but not yet
/// expired still shows — the daemon itself treats it as released.)
pub fn held_by_seat(lane: &LaneKey, seat: u64) -> Result<Option<HeldRepair>, RepairError> {
    held_by_seat_in(&super::paths::repairs_root(), lane, seat, now_epoch())
}

/// [`held_by_seat`] under an explicit repairs root and clock.
pub fn held_by_seat_in(
    root: &Path,
    lane: &LaneKey,
    seat: u64,
    now: f64,
) -> Result<Option<HeldRepair>, RepairError> {
    let mut ids = list_ids_in(root);
    ids.reverse();
    for id in ids {
        let Ok(Some(rec)) = read_record_in(root, &id) else {
            continue; // unreadable records are skipped here and listed by `status` (F4)
        };
        let Some(lock) = rec.lock.as_ref() else {
            continue;
        };
        if lock.owner.lane == *lane
            && lock.owner.seat == seat
            && now <= lock.expires_at + super::grant::EXPIRY_GRACE_SECS as f64
        {
            return Ok(Some(HeldRepair {
                id: rec.id.clone(),
                state: rec.state,
                attempt: rec.attempt,
                methods: rec.methods.clone(),
            }));
        }
    }
    Ok(None)
}

/// The job-state reader (RULINGS H3 / I1) — T7b: `propagate::read_job_state_checked` (GK5's bounded
/// reader; absent → `Precondition`, malformed or unreadable → `Corrupt`).
pub fn load_job_state(lane: &LaneKey) -> Result<JobState, RepairError> {
    match super::propagate::read_job_state_checked(&super::paths::repairs_root(), lane) {
        super::propagate::FileRead::Value(st) => Ok(st),
        super::propagate::FileRead::Absent => Err(RepairError::Precondition(format!("{}: 이 부서의 수리 상태 파일이 아직 없습니다", lane.0))),
        super::propagate::FileRead::Malformed(why) | super::propagate::FileRead::ReadError(why) => Err(RepairError::Corrupt(why)),
    }
}

/// The registered lanes (RULINGS K1: base ∪ `depts.json` sockets; interpretation 27).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegisteredLanes {
    Known(Vec<LaneKey>),
    Unknown(String),
}

/// RULINGS K1 / L4 / interpretation 27: the HQ lane ∪ every department socket of the lane registry
/// (`crate::lane::registry_path()`, `~/.cys/depts.json` or `CYS_DEPTS_JSON`). A MISSING registry =
/// no departments (HQ alone); an unreadable or malformed one = `Unknown` ("전파 완료" is then never
/// declared). No 64-entry cap (unlike `lane::registry_sockets`).
pub fn registered_lanes() -> Result<RegisteredLanes, RepairError> {
    Ok(registered_lanes_from(&crate::lane::registry_path()))
}

/// [`registered_lanes`] from an explicit registry file.
pub fn registered_lanes_from(path: &Path) -> RegisteredLanes {
    let base = LaneKey("base".to_string());
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return RegisteredLanes::Known(vec![base])
        }
        Err(e) => return RegisteredLanes::Unknown(format!("{}: {e}", path.display())),
    };
    let text = String::from_utf8_lossy(&bytes);
    let v: serde_json::Value = match serde_json::from_str(crate::strip_utf8_bom(&text)) {
        Ok(v) => v,
        Err(e) => return RegisteredLanes::Unknown(format!("{}: {e}", path.display())),
    };
    let Some(depts) = v.get("depts").and_then(|d| d.as_object()) else {
        return RegisteredLanes::Unknown(format!("{}: no `depts` object", path.display()));
    };
    let mut lanes = vec![base];
    for d in depts.values() {
        let Some(sock) = d.get("socket").and_then(|s| s.as_str()) else {
            continue;
        };
        if sock.trim().is_empty() {
            continue;
        }
        let key = LaneKey::of_socket(Path::new(sock));
        if !lanes.contains(&key) {
            lanes.push(key);
        }
    }
    RegisteredLanes::Known(lanes)
}

/// Why the assignment gate holds a lane (RULINGS I1 + I7 + I8 decision table).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HoldCause {
    Pending { pending: u32, conflict: u32 },
    FirstPassNotDone,
    StateUnreadable(String),
    PackInstanceChanged,
    AwaitingFinalApproval { rows: u32 },
}

/// Result of the pack-instance pass (RULINGS I8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackInstanceOutcome {
    Same,
    Reset { demoted: u32 },
}

/// RULINGS I8 — `fresh_id` comes from the daemon's CSPRNG. Body: T7b
/// (`propagate::pack_instance_pass_in`): the same instance (also after a verify-only pass, L5 (a)) →
/// `Same`; a full reset → `Reset`; a pass skipped for a transient read error (O5) → `Err`.
pub fn pack_instance_pass(
    lane: &LaneKey,
    pack_dir: &Path,
    fresh_id: &str,
) -> Result<PackInstanceOutcome, RepairError> {
    let root = super::paths::repairs_root();
    let env = super::propagate::PassEnv {
        root: &root,
        lane,
        pack: pack_dir,
        now: now_epoch(),
        locked: &super::apply::no_repair_lock,
        stop: &|_| false,
    };
    // RULINGS IV2 (T7b): the pass writes rows, the job state and the pack's id file — inside the tried
    // pack-apply lock, as the tick runs it (this T0 entry has no caller; it must not be a way round it)
    let lock = crate::pack_apply_lock::pack_apply_lock_path_for(pack_dir);
    let outcome = match crate::pack_apply_lock::try_with_apply_lock(&lock, || super::propagate::pack_instance_pass_in(&env, fresh_id)) {
        Ok(Some(o)) => o,
        Ok(None) => return Err(RepairError::Locked("다른 적용이나 업데이트가 팩을 바꾸는 중 — 팩 식별 확인은 다음 차례에".into())),
        Err(why) => return Err(RepairError::Io(why)),
    };
    match outcome {
        super::propagate::PassOutcome::Equal | super::propagate::PassOutcome::VerifyOnly { .. } => Ok(PackInstanceOutcome::Same),
        super::propagate::PassOutcome::Reset { demoted } => Ok(PackInstanceOutcome::Reset { demoted }),
        super::propagate::PassOutcome::Skipped(why) => Err(RepairError::Io(why)),
    }
}

/// The assignment gate's verdict for a lane (printed by `cys repair gate`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateVerdict {
    Open,
    Held(HoldCause),
}

/// Local read of the gate (interpretation 24). Body: T7b (`propagate::gate_verdict_in` — reads only;
/// every doubt is HELD with its cause).
pub fn gate_verdict(lane: &LaneKey) -> Result<GateVerdict, RepairError> {
    Ok(super::propagate::gate_verdict_in(&super::paths::repairs_root(), lane))
}

/// THE candidate reader (RULINGS V2(a)), re-exported as `repair::candidate_states`: one line per
/// declared FILE target in `rel` order, `TargetState::read(staging_dir(id)/<rel>)`, a missing entry
/// → `Absent`; hook lines are skipped by kind (W3(b)); a `rel` whose parent chain inside staging
/// contains a link is refused (U3).
pub fn candidate_states(id: &RepairId) -> Result<Vec<(String, TargetState)>, RepairError> {
    candidate_states_in(&super::paths::repairs_root(), id)
}

/// [`candidate_states`] under an explicit repairs root.
pub fn candidate_states_in(
    root: &Path,
    id: &RepairId,
) -> Result<Vec<(String, TargetState)>, RepairError> {
    let rec = read_record_in(root, id)?.ok_or_else(|| {
        RepairError::Precondition(format!("{}: 그런 수리가 없습니다", id.as_str()))
    })?;
    let staging = staging_dir_in(root, id);
    let mut rels: Vec<&str> = file_targets(&rec).map(|f| f.rel.as_str()).collect();
    rels.sort_unstable();
    rels.dedup();
    let mut out = Vec::with_capacity(rels.len());
    for rel in rels {
        // RULINGS GR5 (CCR-T5b-1): the candidate is read with a size limit
        out.push((
            rel.to_string(),
            read_under_no_link_with(&staging, rel, &|p| {
                TargetState::read_capped(p, MAX_CANDIDATE_BYTES)
            })?,
        ));
    }
    Ok(out)
}

/// RULINGS GR5 (CCR-T5b-1) — the most bytes the ONE candidate reader reads of one staged target:
/// 8 MiB = 8,388,608 bytes, more than eight times the largest file the two compiled lists name
/// (`bin/javis_preflight.py`: 780,355 bytes at round 5, 812,411 bytes on the tree with T6 — eight times
/// is 6,499,288; `screen::tests::candidate_read_is_capped` measures it at every run). A larger
/// candidate is refused at once ("over the size limit"); `submit`, `finish_verification` and the
/// apply all read through [`candidate_states_in`].
pub const MAX_CANDIDATE_BYTES: u64 = 8 * 1024 * 1024;

/// The record's FILE targets — every walker skips hook lines by kind (RULINGS W3(b)).
pub fn file_targets(rec: &RepairRecord) -> impl Iterator<Item = &FileChange> {
    rec.files
        .iter()
        .filter(|f| !matches!(f.kind, ChangeKind::HookUnregister { .. }))
}

/// The record's hook lines (RULINGS V1 / W3(b)).
pub fn hook_lines(rec: &RepairRecord) -> impl Iterator<Item = &FileChange> {
    rec.files
        .iter()
        .filter(|f| matches!(f.kind, ChangeKind::HookUnregister { .. }))
}

/// `TargetState::read(base/<rel>)` refusing a `rel` that is not a plain relative path or whose
/// parent chain inside `base` contains a symbolic link (the base itself must be a real directory,
/// or absent → every entry is `Absent`).
pub fn read_under_no_link(base: &Path, rel: &str) -> Result<TargetState, RepairError> {
    read_under_no_link_with(base, rel, &TargetState::read)
}

/// [`read_under_no_link`]'s path checks with the final read given (RULINGS GR5: the candidate
/// reader passes the capped read).
fn read_under_no_link_with(
    base: &Path,
    rel: &str,
    read: &dyn Fn(&Path) -> Result<TargetState, RepairError>,
) -> Result<TargetState, RepairError> {
    let parts: Vec<&str> = rel.split('/').collect();
    if rel.is_empty()
        || parts
            .iter()
            .any(|p| p.is_empty() || *p == "." || *p == "..")
        || rel.contains('\\')
    {
        return Err(RepairError::Precondition(format!(
            "잘못된 대상 경로: {rel:?}"
        )));
    }
    match std::fs::symlink_metadata(base) {
        Ok(m) if m.file_type().is_symlink() || !m.is_dir() => {
            return Err(RepairError::Precondition(format!(
                "{}: 작업 사본 자리가 실제 폴더가 아닙니다",
                base.display()
            )))
        }
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(TargetState::Absent),
        Err(e) => return Err(RepairError::Io(format!("{}: {e}", base.display()))),
    }
    let mut cur = base.to_path_buf();
    for p in &parts[..parts.len() - 1] {
        cur.push(p);
        match std::fs::symlink_metadata(&cur) {
            Ok(m) if m.file_type().is_symlink() => {
                return Err(RepairError::Precondition(format!(
                    "{rel}: 작업 사본 안의 상위 경로 {} 가 링크입니다 — 거부",
                    cur.display()
                )))
            }
            Ok(m) if !m.is_dir() => return Ok(TargetState::Absent),
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(TargetState::Absent),
            Err(e) => return Err(RepairError::Io(format!("{}: {e}", cur.display()))),
        }
    }
    cur.push(parts[parts.len() - 1]);
    read(&cur)
}

/// Round cap (C1 1.11).
pub const MAX_ROUNDS: u32 = 10;
/// Resubmissions after approval expiry per candidate (C1 1.11).
pub const MAX_RESUBMITS: u32 = 3;
/// Approval request lifetime (C5).
pub const APPROVAL_TTL_SECS: u64 = 1_800;
/// Probation length (C1 1.6).
pub const PROBATION_SECS: u64 = 86_400;
/// Probation re-check spacing (C7).
pub const PROBATION_RECHECK_SECS: u64 = 600;
/// Unapproved-change sweep spacing (M10, C7).
pub const SWEEP_SECS: u64 = 3_600;
/// The appended built-in job's period (C7).
pub const JOB_EVERY_MINUTES: u64 = 2;
/// Reviewer2's audit window for general repairs (C1 1.10).
pub const AUDIT_WINDOW_SECS: u64 = 86_400;

// ───────────────────────── T3: the ledger's storage (C1 · O-C1-1/3/6/7 · C2 2.3(a)) ─────────────────────────
// Every function takes the repairs root explicitly (`paths::repairs_root()` in production), so tests
// never touch the process environment. Layouts mirror `paths.rs` (pinned by `layouts_match_paths_rs`).
// One writer per file (B1): `record.json` only by the origin lane's daemon, `lanes/<lane>.json` only
// by that lane's daemon — enforced by the daemon module, which serialises every read-modify-write of
// one repair under its in-memory per-repair mutex (G6, R3). Ledger files are never deleted here.

/// Seconds since the epoch (the ledger's clock).
pub fn now_epoch() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// `<root>/R-<n>/`
pub fn repair_dir_in(root: &Path, id: &RepairId) -> PathBuf {
    root.join(id.as_str())
}

/// `<root>/R-<n>/record.json`
pub fn record_path_in(root: &Path, id: &RepairId) -> PathBuf {
    repair_dir_in(root, id).join("record.json")
}

/// `<root>/R-<n>/lanes/<lane_key>.json`
pub fn lane_path_in(root: &Path, id: &RepairId, lane: &LaneKey) -> PathBuf {
    repair_dir_in(root, id)
        .join("lanes")
        .join(format!("{}.json", lane.0))
}

/// `<root>/R-<n>/staging/` — the fixed working copy (U3).
pub fn staging_dir_in(root: &Path, id: &RepairId) -> PathBuf {
    repair_dir_in(root, id).join("staging")
}

/// `<root>/displaced/R-<n>/` — the repair's vault slots (C2 2.1).
pub fn vault_dir_in(root: &Path, id: &RepairId) -> PathBuf {
    root.join("displaced").join(id.as_str())
}

fn corrupt(p: &Path, why: impl std::fmt::Display) -> RepairError {
    RepairError::Corrupt(format!("{}: {why}", p.display()))
}

/// Reads a ledger JSON file: `Ok(None)` if absent; `Err(Corrupt)` if unreadable, malformed or of a
/// higher `schema` than this build knows (O-C1-6 / O-C1-7).
fn read_json<T: serde::de::DeserializeOwned>(p: &Path) -> Result<Option<T>, RepairError> {
    // T6 · RULINGS GK5 (the seam's reads made bounded): through THE bounded record reader
    let bytes = match super::durable::read_record_bytes(p) {
        super::durable::RecordRead::Bytes(b) => b,
        super::durable::RecordRead::NotFound => return Ok(None),
        super::durable::RecordRead::Unreadable(why) => return Err(corrupt(p, why)),
    };
    let v: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| corrupt(p, e))?;
    let schema = v.get("schema").and_then(|s| s.as_u64()).unwrap_or(u64::MAX);
    if schema > u64::from(super::REPAIR_SCHEMA) {
        return Err(corrupt(p, format!("unknown schema {schema}")));
    }
    serde_json::from_value(v)
        .map(Some)
        .map_err(|e| corrupt(p, e))
}

/// Reads `R-<n>/record.json` (`None` = no such repair). A record whose `id` differs from its
/// directory is corrupt.
pub fn read_record_in(root: &Path, id: &RepairId) -> Result<Option<RepairRecord>, RepairError> {
    let p = record_path_in(root, id);
    let rec: Option<RepairRecord> = read_json(&p)?;
    if let Some(r) = &rec {
        if r.id != *id {
            return Err(corrupt(&p, format!("record names {}", r.id.as_str())));
        }
    }
    Ok(rec)
}

/// Writes `R-<n>/record.json` (tmp + rename, `cys::atomic_write_json`); the schema written is always
/// this build's (writers never downgrade — a higher-schema record is never loaded, so never written).
pub fn write_record_in(root: &Path, rec: &RepairRecord) -> Result<(), RepairError> {
    let p = record_path_in(root, &rec.id);
    let mut r = rec.clone();
    r.schema = super::REPAIR_SCHEMA;
    crate::atomic_write_json(&p, &r).map_err(|e| RepairError::Io(format!("{}: {e}", p.display())))
}

/// [`write_record_in`] made durable (E1: the file and its directory fully flushed) — used where a
/// later step must never run ahead of the record (commit before the hook step, RULINGS Y1).
pub fn write_record_durable_in(root: &Path, rec: &RepairRecord) -> Result<(), RepairError> {
    write_record_in(root, rec)?;
    let p = record_path_in(root, &rec.id);
    let f =
        std::fs::File::open(&p).map_err(|e| RepairError::Io(format!("{}: {e}", p.display())))?;
    full_flush_file(&f).map_err(|e| RepairError::Io(format!("{}: {e}", p.display())))?;
    full_flush_dir(&repair_dir_in(root, &rec.id))
}

/// Reads one lane row (`None` = the lane has no row for this repair — counts as 대기, K1).
pub fn read_lane_in(
    root: &Path,
    id: &RepairId,
    lane: &LaneKey,
) -> Result<Option<LaneRecord>, RepairError> {
    let p = lane_path_in(root, id, lane);
    let row: Option<LaneRecord> = read_json(&p)?;
    if let Some(r) = &row {
        if r.repair != *id || r.lane != *lane {
            return Err(corrupt(&p, "row names another repair or lane"));
        }
    }
    Ok(row)
}

/// Writes one lane row (only that lane's daemon calls this — K1).
pub fn write_lane_in(root: &Path, row: &LaneRecord) -> Result<(), RepairError> {
    let p = lane_path_in(root, &row.repair, &row.lane);
    let mut r = row.clone();
    r.schema = super::REPAIR_SCHEMA;
    crate::atomic_write_json(&p, &r).map_err(|e| RepairError::Io(format!("{}: {e}", p.display())))
}

/// Every lane row of a repair (sorted by lane key) and the unreadable ones (listed, never fatal).
pub fn lane_rows_in(root: &Path, id: &RepairId) -> (Vec<LaneRecord>, Vec<String>) {
    let dir = repair_dir_in(root, id).join("lanes");
    let mut rows = Vec::new();
    let mut broken = Vec::new();
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return (rows, broken);
    };
    let mut files: Vec<PathBuf> = rd.filter_map(Result::ok).map(|e| e.path()).collect();
    files.sort();
    for f in files {
        let Some(stem) = f
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.strip_suffix(".json"))
        else {
            continue;
        };
        match read_lane_in(root, id, &LaneKey(stem.to_string())) {
            Ok(Some(r)) => rows.push(r),
            Ok(None) => {}
            Err(RepairError::Corrupt(m)) | Err(RepairError::Io(m)) => broken.push(m),
            Err(e) => broken.push(format!("{e:?}")),
        }
    }
    (rows, broken)
}

/// Every `R-<n>` directory under the root, in numeric order (anything else is ignored).
pub fn list_ids_in(root: &Path) -> Vec<RepairId> {
    let Ok(rd) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut ids: Vec<RepairId> = rd
        .filter_map(Result::ok)
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter_map(|e| e.file_name().to_str().and_then(|n| RepairId::parse(n).ok()))
        .collect();
    ids.sort();
    ids
}

#[cfg(test)]
thread_local! {
    /// Test-only (T6 · FG1): the next enumerations of the repairs root fail (an unlistable root).
    pub(crate) static FAIL_LIST_IDS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// T6 · RULINGS FG1 (a granted seam beside `list_ids_in`, whose callers keep its behaviour): the
/// repairs root's ids with three answers — `Ok(ids)`; `Ok(empty)` only on positive evidence (the root
/// does not exist AND its parent could be read); `Err` for everything else (an unlistable root, an
/// entry whose name or type cannot be read).
pub fn list_ids_checked_in(root: &Path) -> Result<Vec<RepairId>, RepairError> {
    #[cfg(test)]
    if FAIL_LIST_IDS.with(|c| c.get()) {
        return Err(RepairError::Io(format!("{}: 목록을 읽을 수 없습니다(시험 주입)", root.display())));
    }
    let rd = match std::fs::read_dir(root) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return match root.parent().map(std::fs::read_dir) {
                Some(Ok(_)) => Ok(Vec::new()),
                _ => Err(RepairError::Io(format!("{}: 없음, 그러나 그 상위 폴더도 읽을 수 없습니다", root.display()))),
            };
        }
        Err(e) => return Err(RepairError::Io(format!("{}: 목록을 읽을 수 없습니다 — {e}", root.display()))),
    };
    let mut ids = Vec::new();
    for entry in rd {
        let entry = entry.map_err(|e| RepairError::Io(format!("{}: 항목을 읽을 수 없습니다 — {e}", root.display())))?;
        let ft = entry.file_type().map_err(|e| RepairError::Io(format!("{}: 항목 종류를 읽을 수 없습니다 — {e}", entry.path().display())))?;
        if !ft.is_dir() {
            continue;
        }
        if let Some(id) = entry.file_name().to_str().and_then(|n| RepairId::parse(n).ok()) {
            ids.push(id);
        }
    }
    ids.sort();
    Ok(ids)
}

/// T6 · RULINGS FG4 (a granted seam beside `write_lane_in`): the lane row written through the durable
/// writer (file flushed, placed, directory flushed); a failure is returned — the event is not final.
pub fn write_lane_durable_in(root: &Path, row: &LaneRecord) -> Result<(), RepairError> {
    let p = lane_path_in(root, &row.repair, &row.lane);
    let mut r = row.clone();
    r.schema = super::REPAIR_SCHEMA;
    super::durable::write_json_durable(&p, &r)
}

/// Allocates the next `R-<n>` by an exclusive directory create (O-C1-2: n = highest + 1; on
/// `AlreadyExists` — another daemon won that number — the next one is tried). Unique across lanes
/// because the create is atomic on the shared filesystem.
pub fn allocate_id_in(root: &Path) -> Result<RepairId, RepairError> {
    std::fs::create_dir_all(root)
        .map_err(|e| RepairError::Io(format!("{}: {e}", root.display())))?;
    for _ in 0..1000 {
        let next = list_ids_in(root)
            .last()
            .and_then(|id| {
                id.as_str()
                    .strip_prefix("R-")
                    .and_then(|n| n.parse::<u64>().ok())
            })
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| RepairError::Precondition("수리 번호가 다 찼습니다".into()))?;
        let id = RepairId::parse(&format!("R-{next}"))?;
        match std::fs::create_dir(repair_dir_in(root, &id)) {
            Ok(()) => return Ok(id),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(RepairError::Io(format!("{}: {e}", root.display()))),
        }
    }
    Err(RepairError::Io(
        "repair id allocation: too many concurrent creates".into(),
    ))
}

/// RULINGS AD9 (S1) / G1 "LOCK FIRST" — creates a repair's record ALREADY LOCKED and returns its
/// id: the id is allocated by `create_dir`, `make(&id)` builds the record (it may copy pre-images),
/// the record must carry a lock, and it is written before anything else can see the id. If `make`
/// fails, the (still empty) id directory is removed with `remove_dir` (never a ledger file). The
/// caller holds the owning daemon's per-repair serialisation. T4's claim sequence uses this, then
/// [`bind_claim_in`], and on a lost race [`release_lock_in`].
pub fn create_locked_in<E: From<RepairError>>(
    root: &Path,
    make: impl FnOnce(&RepairId) -> Result<RepairRecord, E>,
) -> Result<RepairId, E> {
    let id = allocate_id_in(root)?;
    let rec = match make(&id) {
        Ok(r) => r,
        Err(e) => {
            let _ = std::fs::remove_dir(repair_dir_in(root, &id));
            return Err(e);
        }
    };
    if rec.id != id || rec.lock.is_none() {
        let _ = std::fs::remove_dir(repair_dir_in(root, &id));
        return Err(RepairError::Precondition(format!(
            "{}: a record created through create_locked must carry its own id and a lock",
            id.as_str()
        ))
        .into());
    }
    write_record_in(root, &rec)?;
    Ok(id)
}

/// [`create_locked_in`] under the default repairs root.
pub fn create_locked<E: From<RepairError>>(
    make: impl FnOnce(&RepairId) -> Result<RepairRecord, E>,
) -> Result<RepairId, E> {
    create_locked_in(&super::paths::repairs_root(), make)
}

/// RULINGS AD9 (S1) / G1 step 3: binds a grant claim to the lock `owner` holds on the record
/// (refused when the record carries no lock of that owner — G6).
pub fn bind_claim_in(
    root: &Path,
    id: &RepairId,
    owner: &super::types::LockOwner,
    claim: u32,
) -> Result<(), RepairError> {
    let mut rec = read_record_in(root, id)?
        .ok_or_else(|| RepairError::Precondition(format!("{}: no such repair", id.as_str())))?;
    match rec.lock.as_mut() {
        Some(l) if super::lock::owned_by(l, owner) => l.claim = Some(claim),
        _ => {
            return Err(RepairError::Locked(format!(
                "{}: the record carries no lock of this owner",
                id.as_str()
            )))
        }
    }
    write_record_in(root, &rec)
}

/// [`bind_claim_in`] under the default repairs root.
pub fn bind_claim(
    id: &RepairId,
    owner: &super::types::LockOwner,
    claim: u32,
) -> Result<(), RepairError> {
    bind_claim_in(&super::paths::repairs_root(), id, owner, claim)
}

/// RULINGS AD9 (S1) / G1 step 4 / G6: clears the record's lock only when `owner` holds it (and,
/// when `claim` is given, only when the lock is bound to that claim) — a losing or retried attempt
/// never clears another attempt's lock. The record stays open and unlocked (visible, reassignable).
/// Returns whether a lock was cleared.
pub fn release_lock_in(
    root: &Path,
    id: &RepairId,
    owner: &super::types::LockOwner,
    claim: Option<u32>,
) -> Result<bool, RepairError> {
    let Some(mut rec) = read_record_in(root, id)? else {
        return Ok(false);
    };
    let mine = rec
        .lock
        .as_ref()
        .is_some_and(|l| super::lock::owned_by(l, owner) && (claim.is_none() || l.claim == claim));
    if !mine {
        return Ok(false);
    }
    rec.lock = None;
    write_record_in(root, &rec)?;
    Ok(true)
}

/// [`release_lock_in`] under the default repairs root.
pub fn release_lock(
    id: &RepairId,
    owner: &super::types::LockOwner,
    claim: Option<u32>,
) -> Result<bool, RepairError> {
    release_lock_in(&super::paths::repairs_root(), id, owner, claim)
}

/// The full flush of RULINGS E1: `F_FULLFSYNC` on macOS (plain `fsync` lost the exchange in 10/10
/// simulated power losses), `sync_all` elsewhere. (T6's `durable` module is the shared helper once
/// it lands; this is the same call for T3's own files.)
pub fn full_flush_file(f: &std::fs::File) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::io::AsRawFd;
        // SAFETY: fcntl on a descriptor this function borrows for the duration of the call.
        if unsafe { libc::fcntl(f.as_raw_fd(), libc::F_FULLFSYNC) } == 0 {
            return Ok(());
        }
    }
    f.sync_all()
}

/// [`full_flush_file`] on a directory handle (unix; Windows has no directory flush — no-op there).
pub fn full_flush_dir(dir: &Path) -> Result<(), RepairError> {
    #[cfg(unix)]
    {
        let f = std::fs::File::open(dir)
            .map_err(|e| RepairError::Io(format!("{}: {e}", dir.display())))?;
        full_flush_file(&f).map_err(|e| RepairError::Io(format!("{}: {e}", dir.display())))?;
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
    }
    Ok(())
}

/// O-C2-1: a slot name `YYYYMMDDTHHMMSSffffffZ` (UTC, microseconds) for an epoch time.
pub fn utc_slot_name(epoch: f64) -> String {
    let micros_total = if epoch.is_finite() && epoch > 0.0 {
        (epoch * 1_000_000.0) as u64
    } else {
        0
    };
    let secs = micros_total / 1_000_000;
    let micros = micros_total % 1_000_000;
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (h, mi, se) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    // civil-from-days (Howard Hinnant's algorithm), proleptic Gregorian
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}{m:02}{d:02}T{h:02}{mi:02}{se:02}{micros:06}Z")
}

/// True for a slot directory name of O-C2-1 (`…Z` or `…Z-<n>`).
pub fn is_slot_name(s: &str) -> bool {
    let (base, suffix) = match s.split_once('-') {
        Some((b, n)) => (b, Some(n)),
        None => (s, None),
    };
    let b = base.as_bytes();
    let shape = b.len() == 22
        && b[8] == b'T'
        && b[21] == b'Z'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 8 || i == 21 || c.is_ascii_digit());
    shape
        && suffix
            .is_none_or(|n| !n.is_empty() && n.len() <= 6 && n.bytes().all(|c| c.is_ascii_digit()))
}

/// Opens a regular file for reading without following a link and without blocking on a FIFO.
fn open_source(p: &Path) -> std::io::Result<std::fs::File> {
    let mut o = std::fs::OpenOptions::new();
    o.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    o.open(p)
}

/// Copies one regular file (bytes + permission bits; the copy gets the CURRENT time as mtime —
/// E2/E3) to a new path (`create_new` — never over anything), fully flushed. Returns (sha256, mode).
fn copy_file_new(src: &Path, dst: &Path) -> Result<(String, u32), RepairError> {
    use sha2::{Digest, Sha256};
    use std::io::{Read, Write};
    let io = |p: &Path, e: std::io::Error| RepairError::Io(format!("{}: {e}", p.display()));
    let mut from = open_source(src).map_err(|e| io(src, e))?;
    let meta = from.metadata().map_err(|e| io(src, e))?;
    if !meta.file_type().is_file() {
        return Err(RepairError::Precondition(format!(
            "{}: 일반 파일이 아닙니다",
            src.display()
        )));
    }
    #[cfg(unix)]
    let mode = {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o7777
    };
    #[cfg(not(unix))]
    let mode = 0u32;
    let mut to = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dst)
        .map_err(|e| io(dst, e))?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = from.read(&mut buf).map_err(|e| io(src, e))?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        to.write_all(&buf[..n]).map_err(|e| io(dst, e))?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        to.set_permissions(std::fs::Permissions::from_mode(mode))
            .map_err(|e| io(dst, e))?;
    }
    full_flush_file(&to).map_err(|e| io(dst, e))?;
    Ok((
        h.finalize().iter().map(|b| format!("{b:02x}")).collect(),
        mode,
    ))
}

/// Creates `dst` as a symbolic link with the given target (a link is kept as a link, U3 / C2).
fn copy_link_new(target: &str, dst: &Path) -> Result<(), RepairError> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, dst)
            .map_err(|e| RepairError::Io(format!("{}: {e}", dst.display())))
    }
    #[cfg(not(unix))]
    {
        let _ = (target, dst);
        Err(RepairError::PlatformPending(
            "링크 대상은 이 운영체제에서 보관하지 않습니다",
        ))
    }
}

#[cfg(test)]
thread_local! {
    /// Test-only (T6 · GK1): runs between the pre-image's copy and the second reading of the
    /// generation (an update completing there).
    pub(crate) static BETWEEN_PREIMAGE_COPY_AND_GENERATION: std::cell::RefCell<Option<Box<dyn Fn()>>> = const { std::cell::RefCell::new(None) };
}

/// C2 2.3(a) / C1 1.7 — the `begin` pre-image: an existing live entry is COPIED into a fresh,
/// write-once vault slot `displaced/R-<n>/<시각>/payload/<basename>` with `meta/pre.json` (the live
/// file is not touched); a path that does not exist is recorded as `absent` (no copy). The slot and
/// its directories are fully flushed (E1) before this returns. A directory or another kind of entry
/// is refused.
pub fn copy_preimage_in(
    root: &Path,
    id: &RepairId,
    lane: &LaneKey,
    rel: &str,
    live: &Path,
    now: f64,
) -> Result<PreImage, RepairError> {
    // T6b · RULINGS IW1 / JH1: a file longer than the engine's read limit has no state — `begin` is refused
    let state = TargetState::read(live)?;
    if state == TargetState::Absent {
        return Ok(PreImage::Absent);
    }
    // T6 · RULINGS FI1 / GK1 (a granted seam in T3's begin): the pack is `live` without `rel` — every
    // caller passes `pack.join(rel)`; when it is not, the generation is absent (Unknown). Read here,
    // BEFORE the copy (and again after it, below).
    let pack_of_live = {
        let mut p = live.to_path_buf();
        let mut ok = true;
        for _ in rel.split('/').filter(|c| !c.is_empty()) {
            ok &= p.pop();
        }
        (ok && p.join(rel) == live).then_some(p)
    };
    let generation_before = pack_of_live.as_ref().and_then(|p| super::apply::pack_generation_checked(p).ok());
    let io = |p: &Path, e: std::io::Error| RepairError::Io(format!("{}: {e}", p.display()));
    let vault = vault_dir_in(root, id);
    std::fs::create_dir_all(&vault).map_err(|e| io(&vault, e))?;
    let base_name = utc_slot_name(now);
    let mut slot = vault.join(&base_name);
    let mut n = 0u32;
    loop {
        match std::fs::create_dir(&slot) {
            Ok(()) => break,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && n < 999 => {
                n += 1;
                slot = vault.join(format!("{base_name}-{n}"));
            }
            Err(e) => return Err(io(&slot, e)),
        }
    }
    let payload = slot.join("payload");
    let meta = slot.join("meta");
    std::fs::create_dir(&payload).map_err(|e| io(&payload, e))?;
    std::fs::create_dir(&meta).map_err(|e| io(&meta, e))?;
    let basename = rel.rsplit('/').next().unwrap_or(rel).to_string();
    let dst = payload.join(&basename);
    let (sha256, mode, link_target) = match &state {
        TargetState::Link { target } => {
            copy_link_new(target, &dst)?;
            (String::new(), 0, Some(target.clone()))
        }
        _ => {
            let (sha, mode) = copy_file_new(live, &dst)?;
            (sha, mode, None)
        }
    };
    let mut pre = serde_json::json!({
        "schema": super::REPAIR_SCHEMA,
        "repair": id.as_str(),
        "lane_key": lane.0,
        "rel": rel,
        "op": "begin",
        "kind": "pre-image",
        "sha256": sha256,
        "mode": mode,
        "link_target": link_target,
    });
    // (T6 · FI1 / GK1: the generation measured around the copy is written below)
    let pre_path = meta.join("pre.json");
    // T6 · RULINGS GK1 (a) / GM1 (the seam): the generation read BEFORE and AFTER the copy — equal →
    // recorded; unequal or unreadable → absent (Unknown); and a `#!` regular file without the
    // owner's execute bit (unix) → absent: its mode may not be the generation's final one (the
    // non-transactional install sets that bit after `.pack-version`)
    #[cfg(test)]
    BETWEEN_PREIMAGE_COPY_AND_GENERATION.with(|h| {
        if let Some(f) = h.borrow().as_ref() {
            f();
        }
    });
    let generation_after = pack_of_live.as_ref().and_then(|p| super::apply::pack_generation_checked(p).ok());
    let stable = matches!((&generation_before, &generation_after), (Some(b), Some(a)) if b == a);
    let mode_may_lag = cfg!(unix) && link_target.is_none() && mode & 0o100 == 0 && std::fs::read(&dst).map(|b| b.starts_with(b"#!")).unwrap_or(true);
    if stable && !mode_may_lag {
        if let Some(g) = &generation_before {
            pre["pack_generation"] = serde_json::json!(g);
        }
    }
    {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&pre_path)
            .map_err(|e| io(&pre_path, e))?;
        let mut bytes =
            serde_json::to_vec_pretty(&pre).map_err(|e| RepairError::Io(e.to_string()))?;
        bytes.push(b'\n');
        f.write_all(&bytes).map_err(|e| io(&pre_path, e))?;
        full_flush_file(&f).map_err(|e| io(&pre_path, e))?;
    }
    for d in [&payload, &meta, &slot, &vault] {
        full_flush_dir(d)?;
    }
    if let Some(parent) = vault.parent() {
        full_flush_dir(parent)?;
    }
    Ok(PreImage::Slot {
        slot_dir: slot,
        sha256,
        mode,
        link_target,
    })
}

/// U3 / V2(b): creates the fixed working copy `R-<n>/staging/` ONLY if it does not exist yet, and
/// copies every existing declared target into it (bytes and mode; a link as a link; nothing
/// followed). It is populated in a temporary sibling and renamed into place once, so a crash never
/// leaves a half-populated `staging/` (DEFERRED_MINOR #5's proposal). Returns the directory and
/// whether this call created it. After this, the daemon never writes, moves or deletes there.
pub fn populate_staging_in(
    root: &Path,
    id: &RepairId,
    targets: &[(String, PathBuf)],
) -> Result<(PathBuf, bool), RepairError> {
    let staging = staging_dir_in(root, id);
    if std::fs::symlink_metadata(&staging).is_ok() {
        return Ok((staging, false));
    }
    let io = |p: &Path, e: std::io::Error| RepairError::Io(format!("{}: {e}", p.display()));
    let parent = repair_dir_in(root, id);
    let tmp = parent.join(format!(
        ".staging-tmp-{}-{}",
        std::process::id(),
        (now_epoch() * 1e6) as u64
    ));
    std::fs::create_dir(&tmp).map_err(|e| io(&tmp, e))?;
    let fill = || -> Result<(), RepairError> {
        for (rel, live) in targets {
            match TargetState::read(live)? {
                TargetState::Absent => continue,
                TargetState::Link { target } => {
                    let dst = tmp.join(rel);
                    if let Some(p) = dst.parent() {
                        std::fs::create_dir_all(p).map_err(|e| io(p, e))?;
                    }
                    copy_link_new(&target, &dst)?;
                }
                TargetState::File { .. } => {
                    let dst = tmp.join(rel);
                    if let Some(p) = dst.parent() {
                        std::fs::create_dir_all(p).map_err(|e| io(p, e))?;
                    }
                    copy_file_new(live, &dst)?;
                }
            }
        }
        full_flush_dir(&tmp)
    };
    if let Err(e) = fill() {
        let _ = std::fs::remove_dir_all(&tmp); // our own temporary directory, never the ledger
        return Err(e);
    }
    std::fs::rename(&tmp, &staging).map_err(|e| {
        let _ = std::fs::remove_dir_all(&tmp);
        io(&staging, e)
    })?;
    full_flush_dir(&parent)?;
    Ok((staging, true))
}

/// One vault slot as `status` shows it (C2 2.5; E4 flag).
#[derive(Debug, Clone, PartialEq)]
pub struct SlotInfo {
    pub name: String,
    pub dir: PathBuf,
    pub entries: Vec<String>,
    /// E4: the payload no longer matches the hash recorded for it (a writer that held the file
    /// open kept writing) — `None` when nothing recorded can be compared.
    pub changed_after_deposit: Option<bool>,
}

/// The repair's vault slots, oldest first.
pub fn list_slots_in(root: &Path, id: &RepairId) -> Vec<SlotInfo> {
    let vault = vault_dir_in(root, id);
    let Ok(rd) = std::fs::read_dir(&vault) else {
        return Vec::new();
    };
    let mut names: Vec<String> = rd
        .filter_map(Result::ok)
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        .filter(|n| is_slot_name(n))
        .collect();
    names.sort();
    names
        .into_iter()
        .map(|name| {
            let dir = vault.join(&name);
            let payload = dir.join("payload");
            let mut entries: Vec<String> = std::fs::read_dir(&payload)
                .map(|rd| {
                    rd.filter_map(Result::ok)
                        .filter_map(|e| e.file_name().to_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            entries.sort();
            let recorded: Option<String> = std::fs::read(dir.join("meta").join("sealed.json"))
                .ok()
                .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                .and_then(|v| {
                    v.get("slot_sha256_at_check")
                        .and_then(|s| s.as_str())
                        .map(str::to_string)
                })
                .or_else(|| {
                    std::fs::read(dir.join("meta").join("pre.json"))
                        .ok()
                        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                        .and_then(|v| v.get("sha256").and_then(|s| s.as_str()).map(str::to_string))
                        .filter(|s| !s.is_empty())
                });
            let changed_after_deposit = match (recorded, entries.as_slice()) {
                (Some(want), [one]) => match TargetState::read(&payload.join(one)) {
                    Ok(TargetState::File { sha256, .. }) => Some(sha256 != want),
                    _ => None,
                },
                _ => None,
            };
            SlotInfo {
                name,
                dir,
                entries,
                changed_after_deposit,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repair::types::LockOwner;
    use serde_json::Value;

    fn id(s: &str) -> RepairId {
        serde_json::from_value(serde_json::json!(s)).unwrap()
    }

    fn keys(v: &Value) -> Vec<String> {
        let mut k: Vec<String> = v
            .as_object()
            .unwrap_or_else(|| panic!("not an object: {v}"))
            .keys()
            .cloned()
            .collect();
        k.sort();
        k
    }

    fn want(names: &[&str]) -> Vec<String> {
        let mut k: Vec<String> = names.iter().map(|s| s.to_string()).collect();
        k.sort();
        k
    }

    fn lease(claim: Option<u32>) -> Lease {
        Lease {
            owner: LockOwner {
                lane: LaneKey("base".into()),
                seat: 7,
                incarnation: "inc-1".into(),
            },
            expires_at: 1_800_000_000.5,
            claim,
        }
    }

    fn att(role: &str) -> Attestation {
        Attestation {
            seat: 9,
            role: role.into(),
            verdict: "pass".into(),
            note: "n".into(),
            at: 1.25,
            fingerprint_sha256: "ff".repeat(32),
        }
    }

    /// Serde round trip of the three ledger files with EXACTLY the field names C1 / §6 fix
    /// (G5, H8, I7, I8, L1, L5, N1, N3, O1, O4, K4/P3, P2/P5, R1, R5, S1, V2, W3).
    #[test]
    fn record_stub_has_every_c1_field() {
        let rec = RepairRecord {
            schema: crate::repair::REPAIR_SCHEMA,
            id: id("R-12"),
            created_at: 1_700_000_000.0,
            trigger: Trigger::Grant,
            symptom: "s".into(),
            cause: "c".into(),
            classification: Classification::BootCritical,
            files: vec![
                FileChange {
                    rel: "bin/a.py".into(),
                    kind: ChangeKind::Edit,
                    pre: PreImage::Slot {
                        slot_dir: PathBuf::from("/x/slot"),
                        sha256: "aa".into(),
                        mode: 0o644,
                        link_target: None,
                    },
                    post: TargetState::File {
                        sha256: "bb".into(),
                        mode: 0o755,
                    },
                    diff: Some("@@".into()),
                },
                FileChange {
                    rel: "settings:Stop:x.sh".into(),
                    kind: ChangeKind::HookUnregister {
                        event: "Stop".into(),
                        script: "x.sh".into(),
                    },
                    pre: PreImage::Absent,
                    post: TargetState::Absent,
                    diff: None,
                },
            ],
            worker_record: Some(att("worker")),
            verify: Some(att("reviewer-codex")),
            audit: Some(att("reviewer-gemini")),
            audit_due_at: Some(2.5),
            approval: Some(ApprovalResult {
                request_id: "rq".into(),
                fingerprint_sha256: "ab".into(),
                decided_at: 3.5,
                approved: true,
            }),
            owner_token_sha256: Some("cd".into()),
            attempt: 2,
            methods: vec!["m1".into()],
            resubmits: vec![("fp".into(), 1)],
            pending_request: Some(("fp".into(), 4.5)),
            lock: Some(lease(Some(3))),
            state: RepairState::AwaitingApproval,
            intake_signatures: vec!["sig".into()],
            owner_original: true,
            origin_lane: LaneKey("base".into()),
            resolves: Some(id("R-11")),
            committed: vec![("bin/a.py".into(), "file:bb:0755".into())],
            fixer_seats: vec![3, 9],
            checks: vec![("bin/a.py".into(), "py-compile".into())],
            reapplies: Some(id("R-11")),
        };
        let v = serde_json::to_value(&rec).unwrap();
        assert_eq!(
            keys(&v),
            want(&[
                "schema",
                "id",
                "created_at",
                "trigger",
                "symptom",
                "cause",
                "classification",
                "files",
                "worker_record",
                "verify",
                "audit",
                "audit_due_at",
                "approval",
                "owner_token_sha256",
                "attempt",
                "methods",
                "resubmits",
                "pending_request",
                "lock",
                "state",
                "intake_signatures",
                "owner_original",
                "origin_lane",
                "resolves",
                "committed",
                "fixer_seats",
                "checks",
                // T6 (RULINGS AQ5) — the old expectation ended at "checks"
                "reapplies",
            ])
        );
        assert_eq!(
            keys(&v["lock"]),
            want(&["owner", "expires_at", "claim"]),
            "Lease.claim (G1/G2)"
        );
        assert_eq!(
            keys(&v["lock"]["owner"]),
            want(&["lane", "seat", "incarnation"])
        );
        assert_eq!(
            keys(&v["verify"]),
            want(&[
                "seat",
                "role",
                "verdict",
                "note",
                "at",
                "fingerprint_sha256"
            ]),
            "Attestation (N1)"
        );
        assert_eq!(
            keys(&v["approval"]),
            want(&["request_id", "fingerprint_sha256", "decided_at", "approved"])
        );
        assert_eq!(
            keys(&v["files"][0]),
            want(&["rel", "kind", "pre", "post", "diff"])
        );
        assert_eq!(v["id"], serde_json::json!("R-12"));
        assert_eq!(v["state"], serde_json::json!("awaiting_approval"));
        assert_eq!(v["trigger"], serde_json::json!("grant"));
        assert_eq!(v["classification"], serde_json::json!("boot_critical"));
        assert_eq!(serde_json::from_value::<RepairRecord>(v).unwrap(), rec);

        let merge = |via: Option<RepairId>| MergeRecord {
            by_seat: 4,
            result: vec![("bin/a.py".into(), "absent".into())],
            changed: true,
            note: "n".into(),
            via_repair: via,
        };
        let lane = LaneRecord {
            schema: 1,
            repair: id("R-12"),
            lane: LaneKey("dept-x".into()),
            state: LaneState::Conflict,
            probation: Some(Probation {
                pack_generation: "0.15.0".into(),
                pro_revision: Some(2),
                applied_at: 5.5,
                expected: vec![("bin/a.py".into(), "file:bb:0755".into())],
                closed: Some("superseded".into()),
                check_timings: vec![CheckTiming {
                    rel: "bin/a.py".into(),
                    check_ms: 12,
                    reference_kind: CheckKind::PyCompile,
                    reference_ms: 10,
                }],
                check_timeouts: vec![("bin/a.py".into(), 6.5)],
            }),
            merge: Some(merge(Some(id("R-13")))),
            merge_history: vec![merge(None)],
            approval_ref: Some("rq".into()),
            conflict_live: vec![("bin/a.py".into(), "file:cc:0644".into())],
            applied: vec![("bin/a.py".into(), "file:bb:0755".into())],
            applied_history: vec![vec![("bin/a.py".into(), "absent".into())]],
            merge_base: vec![("bin/a.py".into(), "file:aa:0644".into())],
            merge_lock: Some(lease(None)),
            merge_attempt: 1,
            reason: String::new(),
            updated_at: 7.5,
        };
        let v = serde_json::to_value(&lane).unwrap();
        assert_eq!(
            keys(&v),
            want(&[
                "schema",
                "repair",
                "lane",
                "state",
                "probation",
                "merge",
                "merge_history",
                "approval_ref",
                "conflict_live",
                "applied",
                "applied_history",
                "merge_base",
                "merge_lock",
                "merge_attempt",
                "updated_at",
            ])
        );
        assert_eq!(
            keys(&v["probation"]),
            want(&[
                "pack_generation",
                "pro_revision",
                "applied_at",
                "expected",
                "closed",
                "check_timings",
                "check_timeouts"
            ])
        );
        assert_eq!(
            keys(&v["probation"]["check_timings"][0]),
            want(&["rel", "check_ms", "reference_kind", "reference_ms"])
        );
        assert_eq!(
            v["probation"]["check_timings"][0]["reference_kind"],
            serde_json::json!("py-compile")
        );
        assert_eq!(
            keys(&v["merge"]),
            want(&["by_seat", "result", "changed", "note", "via_repair"]),
            "MergeRecord (H8, R1)"
        );
        assert_eq!(v["state"], serde_json::json!("conflict"));
        assert_eq!(serde_json::from_value::<LaneRecord>(v).unwrap(), lane);

        let job = JobState {
            schema: 1,
            lane: LaneKey("base".into()),
            last_incarnation: "inc-1".into(),
            last_probation_check: 8.5,
            last_sweep: 9.5,
            initial_catchup_complete: true,
            pack_instance: Some("0123abcd".into()),
            pack_dir: Some(PathBuf::from("/x/pack")),
            pack_dir_identity: Some(DirIdentity {
                volume_uuid: "UUID".into(),
                inode: 42,
            }),
            pack_generation: Some(PackGeneration {
                version: "0.15.0".into(),
                pro_revision: None,
            }),
        };
        let v = serde_json::to_value(&job).unwrap();
        assert_eq!(
            keys(&v),
            want(&[
                "schema",
                "lane",
                "last_incarnation",
                "last_probation_check",
                "last_sweep",
                "initial_catchup_complete",
                "pack_instance",
                "pack_dir",
                "pack_dir_identity",
                "pack_generation",
            ])
        );
        assert_eq!(
            keys(&v["pack_dir_identity"]),
            want(&["volume_uuid", "inode"]),
            "DirIdentity (L1)"
        );
        assert_eq!(
            keys(&v["pack_generation"]),
            want(&["version", "pro_revision"]),
            "PackGeneration (O4)"
        );
        assert_eq!(serde_json::from_value::<JobState>(v).unwrap(), job);
    }
}

/// T3: the ledger's storage (allocation, schema, corruption, pre-image copy, staging, readers).
#[cfg(test)]
mod storage_tests {
    use super::*;
    use crate::repair::types::LockOwner;

    fn tmp(tag: u32) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("cys-repair-ledger-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    pub(crate) fn sample(id: &RepairId) -> RepairRecord {
        RepairRecord {
            schema: crate::repair::REPAIR_SCHEMA,
            id: id.clone(),
            created_at: 1.0,
            trigger: Trigger::OwnerOrder,
            symptom: String::new(),
            cause: String::new(),
            classification: Classification::General,
            files: vec![FileChange {
                rel: "bin/a.py".into(),
                kind: ChangeKind::Edit,
                pre: PreImage::Absent,
                post: TargetState::Absent,
                diff: None,
            }],
            worker_record: None,
            verify: None,
            audit: None,
            audit_due_at: None,
            approval: None,
            owner_token_sha256: None,
            attempt: 1,
            methods: vec![],
            resubmits: vec![],
            pending_request: None,
            lock: None,
            state: RepairState::InProgress,
            intake_signatures: vec![],
            owner_original: false,
            origin_lane: LaneKey("base".into()),
            resolves: None,
            committed: vec![],
            fixer_seats: vec![],
            checks: vec![],
            reapplies: None,
        }
    }

    /// The `_in` layouts are exactly the T0 path helpers relative to the repairs root.
    #[test]
    fn layouts_match_paths_rs() {
        let _lock = crate::pack::PACK_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let local = tmp(line!());
        let _env = crate::pack::EnvGuard::set("CYS_LOCAL_DIR", &local);
        let root = crate::repair::paths::repairs_root();
        let id = RepairId::parse("R-7").unwrap();
        let lane = LaneKey("dept-3".into());
        use crate::repair::paths as p;
        assert_eq!(record_path_in(&root, &id), p::record_path(&id));
        assert_eq!(
            lane_path_in(&root, &id, &lane),
            p::lane_state_path(&id, &lane)
        );
        assert_eq!(staging_dir_in(&root, &id), p::staging_dir(&id));
        assert_eq!(repair_dir_in(&root, &id), p::repair_dir(&id));
        assert_eq!(
            vault_dir_in(&root, &id).join("T"),
            p::vault_slot_dir(&id, "T")
        );
        assert_eq!(
            crate::repair::intake::row_path_in(&root, &lane, "ab"),
            p::intake_path(&lane, "ab")
        );
        assert_eq!(
            crate::repair::tombstones::path_in(&root, &lane),
            p::tombstones_path(&lane)
        );
        // lock::claim_closed joins the same marker as paths::grant_closed_path
        std::fs::create_dir_all(p::grant_seq_dir()).unwrap();
        assert!(!crate::repair::lock::claim_closed(&root, Some(9)));
        std::fs::write(p::grant_closed_path(9), b"").unwrap();
        assert!(crate::repair::lock::claim_closed(&root, Some(9)));
        let _ = std::fs::remove_dir_all(&local);
    }

    /// O-C1-2: ids by exclusive directory create — concurrent allocators never share a number.
    #[test]
    fn id_allocation_is_exclusive() {
        let root = tmp(line!());
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let r = root.clone();
                std::thread::spawn(move || {
                    (0..10)
                        .map(|_| allocate_id_in(&r).unwrap())
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        let mut all: Vec<RepairId> = threads
            .into_iter()
            .flat_map(|t| t.join().unwrap())
            .collect();
        all.sort();
        let n = all.len();
        all.dedup();
        assert_eq!(all.len(), n, "two allocators got the same id");
        assert_eq!(all.first().unwrap().as_str(), "R-1");
        assert_eq!(all.last().unwrap().as_str(), "R-80");
        // a stray non-id directory is ignored, numbering continues numerically past R-9 → R-10
        std::fs::create_dir_all(root.join("intake")).unwrap();
        assert_eq!(list_ids_in(&root).len(), 80);
        assert_eq!(allocate_id_in(&root).unwrap().as_str(), "R-81");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// O-C1-6/7: unreadable, higher-schema or mis-named records are `Corrupt` (never a panic, never
    /// overwritten); a missing one is `None`; writers write this build's schema.
    #[test]
    fn corrupt_and_schema() {
        let root = tmp(line!());
        let id = allocate_id_in(&root).unwrap();
        assert!(read_record_in(&root, &id).unwrap().is_none());
        let mut rec = sample(&id);
        rec.schema = 0;
        write_record_in(&root, &rec).unwrap();
        let back = read_record_in(&root, &id).unwrap().unwrap();
        assert_eq!(back.schema, crate::repair::REPAIR_SCHEMA);
        let p = record_path_in(&root, &id);
        for bad in [
            &b"{garbage"[..],
            &b""[..],
            br#"{"schema": 2}"#,
            &serde_json::to_vec(&{
                let mut r = sample(&id);
                r.id = RepairId::parse("R-99").unwrap();
                r
            })
            .unwrap()[..],
        ] {
            std::fs::write(&p, bad).unwrap();
            assert!(
                matches!(read_record_in(&root, &id), Err(RepairError::Corrupt(_))),
                "{:?}",
                String::from_utf8_lossy(bad)
            );
        }
        // a bad id inside a record (not R-<n>) → corrupt, never a path
        std::fs::write(&p, br#"{"schema":1,"id":"../../x"}"#).unwrap();
        assert!(matches!(
            read_record_in(&root, &id),
            Err(RepairError::Corrupt(_))
        ));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// C2 2.3(a): `begin` COPIES a live file into a fresh write-once slot (mode kept, fresh mtime,
    /// live untouched), a link as a link, an absent path as `absent`; two copies in the same
    /// microsecond get distinct slots.
    #[cfg(unix)]
    #[test]
    fn preimage_copy_into_fresh_slot() {
        use std::os::unix::fs::PermissionsExt;
        let root = tmp(line!());
        let pack = root.join("pack");
        std::fs::create_dir_all(pack.join("bin")).unwrap();
        let live = pack.join("bin").join("a.sh");
        std::fs::write(&live, b"#!/bin/sh\necho hi\n").unwrap();
        std::fs::set_permissions(&live, std::fs::Permissions::from_mode(0o755)).unwrap();
        // an old mtime on the live file — the copy must not inherit it (E3)
        let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000);
        std::fs::File::options()
            .write(true)
            .open(&live)
            .unwrap()
            .set_modified(old)
            .unwrap();
        let before = std::fs::read(&live).unwrap();
        let id = RepairId::parse("R-3").unwrap();
        let lane = LaneKey("base".into());
        let pre = copy_preimage_in(&root, &id, &lane, "bin/a.sh", &live, 1_700_000_000.25).unwrap();
        let PreImage::Slot {
            slot_dir,
            sha256,
            mode,
            link_target,
        } = &pre
        else {
            panic!("{pre:?}")
        };
        assert_eq!(*mode, 0o755);
        assert!(link_target.is_none());
        assert_eq!(pre.state(), TargetState::read(&live).unwrap());
        let copy = slot_dir.join("payload").join("a.sh");
        assert_eq!(std::fs::read(&copy).unwrap(), before);
        assert_eq!(
            std::fs::metadata(&copy).unwrap().permissions().mode() & 0o7777,
            0o755
        );
        assert!(
            std::fs::metadata(&copy).unwrap().modified().unwrap()
                > old + std::time::Duration::from_secs(86_400)
        );
        assert_eq!(
            std::fs::read(&live).unwrap(),
            before,
            "the live file was touched"
        );
        let meta: serde_json::Value =
            serde_json::from_slice(&std::fs::read(slot_dir.join("meta").join("pre.json")).unwrap())
                .unwrap();
        assert_eq!(meta["kind"], "pre-image");
        assert_eq!(meta["sha256"], serde_json::json!(sha256));
        assert_eq!(meta["rel"], "bin/a.sh");
        let name = slot_dir.file_name().unwrap().to_str().unwrap();
        assert_eq!(name, "20231114T221320250000Z");
        assert!(is_slot_name(name));
        // same instant → a second, distinct slot (`-1`)
        let pre2 =
            copy_preimage_in(&root, &id, &lane, "bin/a.sh", &live, 1_700_000_000.25).unwrap();
        let PreImage::Slot { slot_dir: s2, .. } = &pre2 else {
            panic!()
        };
        assert_ne!(s2, slot_dir);
        assert!(s2.to_str().unwrap().ends_with("Z-1"));
        // a link is copied as a link (not followed)
        let link = pack.join("bin").join("l");
        std::os::unix::fs::symlink("a.sh", &link).unwrap();
        let pl = copy_preimage_in(&root, &id, &lane, "bin/l", &link, 5.0).unwrap();
        assert_eq!(
            pl.state(),
            TargetState::Link {
                target: "a.sh".into()
            }
        );
        // absent → no copy
        assert_eq!(
            copy_preimage_in(&root, &id, &lane, "bin/none", &pack.join("bin/none"), 6.0).unwrap(),
            PreImage::Absent
        );
        assert_eq!(list_slots_in(&root, &id).len(), 3);
        // E4: a payload changed after deposit is flagged
        std::fs::write(&copy, b"changed").unwrap();
        let slots = list_slots_in(&root, &id);
        let by_name = |n: &str| {
            slots
                .iter()
                .find(|s| s.name == n)
                .unwrap()
                .changed_after_deposit
        };
        assert_eq!(by_name("20231114T221320250000Z"), Some(true));
        assert_eq!(by_name("20231114T221320250000Z-1"), Some(false));
        assert_eq!(
            by_name("19700101T000005000000Z"),
            None,
            "a link slot has no content hash to compare"
        );
        // a directory target is refused
        assert!(copy_preimage_in(&root, &id, &lane, "bin", &pack.join("bin"), 7.0).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// U3 / V2(a)(b): staging is populated once (bytes, mode, links as links), never again; the
    /// candidate reader reads it in rel order, missing → absent, a symlinked parent → refused.
    #[cfg(unix)]
    #[test]
    fn staging_once_and_candidate_reader() {
        use std::os::unix::fs::PermissionsExt;
        let root = tmp(line!());
        let pack = root.join("pack");
        std::fs::create_dir_all(pack.join("hooks")).unwrap();
        std::fs::write(pack.join("hooks/b.sh"), b"b").unwrap();
        std::fs::set_permissions(
            pack.join("hooks/b.sh"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        std::fs::write(pack.join("a.json"), b"{}").unwrap();
        let id = allocate_id_in(&root).unwrap();
        let mut rec = sample(&id);
        rec.files = ["hooks/b.sh", "a.json", "hooks/new.sh"]
            .iter()
            .map(|r| FileChange {
                rel: r.to_string(),
                kind: ChangeKind::Edit,
                pre: PreImage::Absent,
                post: TargetState::Absent,
                diff: None,
            })
            .chain(std::iter::once(FileChange {
                rel: "settings:Stop:x.sh".into(),
                kind: ChangeKind::HookUnregister {
                    event: "Stop".into(),
                    script: "x.sh".into(),
                },
                pre: PreImage::Absent,
                post: TargetState::Absent,
                diff: None,
            }))
            .collect();
        write_record_in(&root, &rec).unwrap();
        let targets: Vec<(String, PathBuf)> = ["hooks/b.sh", "a.json", "hooks/new.sh"]
            .iter()
            .map(|r| (r.to_string(), pack.join(r)))
            .collect();
        let (dir, created) = populate_staging_in(&root, &id, &targets).unwrap();
        assert!(created);
        assert_eq!(dir, staging_dir_in(&root, &id));
        let c = candidate_states_in(&root, &id).unwrap();
        let rels: Vec<&str> = c.iter().map(|(r, _)| r.as_str()).collect();
        assert_eq!(
            rels,
            ["a.json", "hooks/b.sh", "hooks/new.sh"],
            "rel order, hook line skipped"
        );
        assert_eq!(c[1].1, TargetState::read(&pack.join("hooks/b.sh")).unwrap());
        assert_eq!(c[2].1, TargetState::Absent);
        // the worker edits staging; a second populate (pick-up) never touches it (V2(b))
        std::fs::write(dir.join("a.json"), b"{\"x\":1}").unwrap();
        let (_, again) = populate_staging_in(&root, &id, &targets).unwrap();
        assert!(!again);
        assert_eq!(std::fs::read(dir.join("a.json")).unwrap(), b"{\"x\":1}");
        // a symlinked parent inside staging → refused
        std::fs::remove_dir_all(dir.join("hooks")).unwrap();
        std::os::unix::fs::symlink(pack.join("hooks"), dir.join("hooks")).unwrap();
        assert!(matches!(
            candidate_states_in(&root, &id),
            Err(RepairError::Precondition(_))
        ));
        // no leftover temporary directory
        let leftovers: Vec<_> = std::fs::read_dir(repair_dir_in(&root, &id))
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().starts_with(".staging-tmp"))
            .collect();
        assert!(leftovers.is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// K1 / L4 / interpretation 27: missing registry = HQ alone; malformed = unknown; every
    /// department counted (no 64 cap).
    #[test]
    fn registered_lanes_rules() {
        let root = tmp(line!());
        let reg = root.join("depts.json");
        assert_eq!(
            registered_lanes_from(&reg),
            RegisteredLanes::Known(vec![LaneKey("base".into())])
        );
        std::fs::write(&reg, b"{nope").unwrap();
        assert!(matches!(
            registered_lanes_from(&reg),
            RegisteredLanes::Unknown(_)
        ));
        std::fs::write(&reg, br#"{"other": 1}"#).unwrap();
        assert!(matches!(
            registered_lanes_from(&reg),
            RegisteredLanes::Unknown(_)
        ));
        let mut depts = serde_json::Map::new();
        for i in 1..=65 {
            depts.insert(
                format!("dept-{i}"),
                serde_json::json!({"socket": format!("/h/.cys/cys-dept-dept-{i}/cys.sock")}),
            );
        }
        std::fs::write(
            &reg,
            serde_json::to_vec(&serde_json::json!({"depts": depts})).unwrap(),
        )
        .unwrap();
        let RegisteredLanes::Known(lanes) = registered_lanes_from(&reg) else {
            panic!()
        };
        assert_eq!(lanes.len(), 66, "HQ + 65 departments");
        assert_eq!(lanes[0], LaneKey("base".into()));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The [RESUME] reader: the seat's live lock, skipping unreadable records (F4).
    #[test]
    fn held_by_seat_reader() {
        let root = tmp(line!());
        let lane = LaneKey("base".into());
        let a = allocate_id_in(&root).unwrap();
        let b = allocate_id_in(&root).unwrap();
        let mut ra = sample(&a);
        ra.attempt = 3;
        ra.methods = vec!["m".into()];
        ra.lock = Some(crate::repair::lock::new_lease(
            LockOwner {
                lane: lane.clone(),
                seat: 4,
                incarnation: "i".into(),
            },
            100.0,
            None,
        ));
        write_record_in(&root, &ra).unwrap();
        std::fs::write(record_path_in(&root, &b), b"{broken").unwrap();
        let h = held_by_seat_in(&root, &lane, 4, 200.0).unwrap().unwrap();
        assert_eq!((h.id.as_str(), h.attempt, h.methods.len()), ("R-1", 3, 1));
        assert!(held_by_seat_in(&root, &lane, 5, 200.0).unwrap().is_none());
        assert!(
            held_by_seat_in(&root, &lane, 4, 100.0 + 7200.0 + 61.0)
                .unwrap()
                .is_none(),
            "expired lease"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// RULINGS AD9 (S1): a record is created already locked; a claim binds to the owner's lock only;
    /// the release clears only the owner's (and claim's) lock; a failed `make` leaves no directory.
    #[test]
    fn create_locked_bind_and_release() {
        let root = tmp(line!());
        let owner = LockOwner {
            lane: LaneKey("base".into()),
            seat: 3,
            incarnation: "i".into(),
        };
        let other = LockOwner {
            seat: 4,
            ..owner.clone()
        };
        let id = create_locked_in::<RepairError>(&root, |id| {
            let mut r = sample(id);
            r.lock = Some(crate::repair::lock::new_lease(owner.clone(), 1.0, None));
            Ok(r)
        })
        .unwrap();
        assert!(
            read_record_in(&root, &id).unwrap().unwrap().lock.is_some(),
            "written locked"
        );
        assert!(bind_claim_in(&root, &id, &other, 7).is_err());
        bind_claim_in(&root, &id, &owner, 7).unwrap();
        assert_eq!(
            read_record_in(&root, &id)
                .unwrap()
                .unwrap()
                .lock
                .unwrap()
                .claim,
            Some(7)
        );
        assert!(
            !release_lock_in(&root, &id, &other, None).unwrap(),
            "another owner never clears it"
        );
        assert!(
            !release_lock_in(&root, &id, &owner, Some(8)).unwrap(),
            "another claim never clears it"
        );
        assert!(release_lock_in(&root, &id, &owner, Some(7)).unwrap());
        let rec = read_record_in(&root, &id).unwrap().unwrap();
        assert!(rec.lock.is_none());
        assert_eq!(rec.state, RepairState::InProgress, "the record stays open");
        // an unlocked record is refused; a failing make leaves no id directory behind
        assert!(create_locked_in::<RepairError>(&root, |id| Ok(sample(id))).is_err());
        assert!(create_locked_in(&root, |_| Err(RepairError::Io("x".into()))).is_err());
        assert_eq!(list_ids_in(&root).len(), 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// AD1 / AD4: a record written before the two fields existed still reads (serde default).
    #[test]
    fn old_record_without_new_fields_reads() {
        let root = tmp(line!());
        let id = allocate_id_in(&root).unwrap();
        let mut v = serde_json::to_value(sample(&id)).unwrap();
        v.as_object_mut().unwrap().remove("fixer_seats");
        v.as_object_mut().unwrap().remove("checks");
        v.as_object_mut().unwrap().remove("reapplies"); // T6 (AQ5): a record from before the field
        std::fs::write(record_path_in(&root, &id), serde_json::to_vec(&v).unwrap()).unwrap();
        let r = read_record_in(&root, &id).unwrap().unwrap();
        assert!(r.fixer_seats.is_empty() && r.checks.is_empty() && r.reapplies.is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn slot_names() {
        assert_eq!(utc_slot_name(0.0), "19700101T000000000000Z");
        assert_eq!(utc_slot_name(951_782_400.5), "20000229T000000500000Z");
        assert!(is_slot_name("20261112T120000123456Z"));
        assert!(is_slot_name("20261112T120000123456Z-3"));
        for bad in [
            "",
            "x",
            "20261112T120000123456",
            "20261112X120000123456Z",
            "../x",
            "20261112T120000123456Z-",
            "20261112T120000123456Z-a",
        ] {
            assert!(!is_slot_name(bad), "{bad}");
        }
    }
}
