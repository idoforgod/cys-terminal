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

/// Reads one repair record. Stub in T0 (body: T3).
pub fn load_record(id: &RepairId) -> Result<RepairRecord, RepairError> {
    let _ = id;
    Err(RepairError::NotImplemented("ledger::load_record"))
}

/// The repair a seat of a lane holds, if any. Stub in T0 (body: T3).
pub fn held_by_seat(lane: &LaneKey, seat: u64) -> Result<Option<HeldRepair>, RepairError> {
    let _ = (lane, seat);
    Err(RepairError::NotImplemented("ledger::held_by_seat"))
}

/// The job-state reader (RULINGS H3 / I1). Stub in T0.
pub fn load_job_state(lane: &LaneKey) -> Result<JobState, RepairError> {
    let _ = lane;
    Err(RepairError::NotImplemented("ledger::load_job_state"))
}

/// The registered lanes (RULINGS K1: base ∪ `depts.json` sockets; interpretation 27).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegisteredLanes {
    Known(Vec<LaneKey>),
    Unknown(String),
}

/// Stub in T0 (body: T3).
pub fn registered_lanes() -> Result<RegisteredLanes, RepairError> {
    Err(RepairError::NotImplemented("ledger::registered_lanes"))
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

/// RULINGS I8 — `fresh_id` comes from the daemon's CSPRNG. Stub in T0 (body: T7b).
pub fn pack_instance_pass(
    lane: &LaneKey,
    pack_dir: &Path,
    fresh_id: &str,
) -> Result<PackInstanceOutcome, RepairError> {
    let _ = (lane, pack_dir, fresh_id);
    Err(RepairError::NotImplemented("ledger::pack_instance_pass"))
}

/// The assignment gate's verdict for a lane (printed by `cys repair gate`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateVerdict {
    Open,
    Held(HoldCause),
}

/// Local read of the gate (interpretation 24). Stub in T0 (body: T7b).
pub fn gate_verdict(lane: &LaneKey) -> Result<GateVerdict, RepairError> {
    let _ = lane;
    Err(RepairError::NotImplemented("ledger::gate_verdict"))
}

/// THE candidate reader (RULINGS V2(a)), re-exported as `repair::candidate_states`: one line per
/// declared FILE target in `rel` order, `TargetState::read(staging_dir(id)/<rel>)`, a missing entry
/// → `Absent`. Stub in T0 (body: T3).
pub fn candidate_states(id: &RepairId) -> Result<Vec<(String, TargetState)>, RepairError> {
    let _ = id;
    Err(RepairError::NotImplemented("ledger::candidate_states"))
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
