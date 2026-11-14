//! Write-ahead operation records (C1 1.13 · RULINGS E1, E6 · TESTPLAN OQ-D8): one file per daemon
//! operation that changes a live pack file — `repairs/R-<n>/ops/<op-id>.json` — written durably
//! (full flush of the file and its folder) BEFORE the operation touches anything live, updated per
//! step, closed with `finished: true` and an outcome. Only the lane's own daemon writes its records
//! (B1). An interrupted operation is finished or abandoned from its record by the job's next run
//! (`apply::resume_interrupted`). If the record cannot be written, nothing is applied.

use super::ledger::PackGeneration;
use super::types::{Identity, LaneKey, RepairError, RepairId};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// One settings target of a hook line as found by the registrar's dry run (RULINGS Y1): `matcher`
/// and `timeout` are kept as found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookTarget {
    pub path: PathBuf,
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matcher: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
}

/// One hook line of a hook operation record (RULINGS Y1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookLine {
    pub event: String,
    pub script: String,
    pub targets: Vec<HookTarget>,
}

/// The operation record (C1 1.13): repair, file, whether a pre-image existed, the expected live
/// state (what the operation displaces — step 0), the incoming state, the pack generation, the
/// fresh slot's path and identity (volume UUID + inode + hash — E6), the current step. States are
/// `TargetState::canon` strings (U2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpRecord {
    pub schema: u32,
    pub op: String,
    pub repair: RepairId,
    pub lane_key: LaneKey,
    /// `apply` | `revert` | `undo-superseded` | `m10-revert` | `hook-unregister` | `hook-reregister`
    pub kind: String,
    /// `exchange` | `move-in` | `place` | `deposit-first` | `hook`
    pub form: String,
    pub rel: String,
    pub pre_existed: bool,
    pub expected: String,
    pub incoming: String,
    pub pack_generation: Option<PackGeneration>,
    pub slot: Option<PathBuf>,
    /// The staged slot entry (the exchange makes it live) — exchange form.
    pub slot_identity: Option<Identity>,
    /// The live entry seen at step 0 (what is displaced or moved in).
    pub live_identity: Option<Identity>,
    /// The copy to be placed (place / deposit-first form) — never a slot entry.
    pub incoming_copy: Option<PathBuf>,
    pub incoming_copy_identity: Option<Identity>,
    /// `staged` | `moved` | `sealed` | `placed` | `done`
    pub step: String,
    #[serde(default)]
    pub hooks: Vec<HookLine>,
    pub finished: bool,
    /// `applied` | `superseded` | `abandoned` | `declined` (when finished)
    pub outcome: Option<String>,
    pub note: String,
    pub created_at: f64,
    pub updated_at: f64,
    /// RULINGS EN2 / EJ3 — the duty to notify of this operation's outcome (an M10 put-back):
    /// `due`, written by the SAME write that completes the operation, until its reader consumed it
    /// (`delivered`). Absent = no notice due.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notice: Option<String>,
}

/// `<root>/R-<n>/ops/`
pub fn ops_dir_in(root: &Path, id: &RepairId) -> PathBuf {
    super::ledger::repair_dir_in(root, id).join("ops")
}

// ───────────────────────────── RULINGS DR9 — what a live operation owns ─────────────────────────────

fn live_set() -> &'static std::sync::Mutex<std::collections::HashSet<String>> {
    static LIVE: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> = std::sync::OnceLock::new();
    LIVE.get_or_init(|| std::sync::Mutex::new(std::collections::HashSet::new()))
}

/// RULINGS DR9 — this process is EXECUTING the operation (or apply / revert intent) `id` while the
/// guard lives. Recovery never resumes or abandons an id in this set; an unfinished record of this
/// lane NOT in the set belongs to a dead incarnation and is recovered.
pub struct LiveOp(String);

impl LiveOp {
    pub fn enter(id: &str) -> LiveOp {
        live_set().lock().unwrap_or_else(|e| e.into_inner()).insert(id.to_string());
        LiveOp(id.to_string())
    }
    /// Claims `id` for a recovery: `None` when a live operation already owns it.
    pub fn try_enter(id: &str) -> Option<LiveOp> {
        let mut set = live_set().lock().unwrap_or_else(|e| e.into_inner());
        if set.contains(id) {
            return None;
        }
        set.insert(id.to_string());
        Some(LiveOp(id.to_string()))
    }
}

impl Drop for LiveOp {
    fn drop(&mut self) {
        live_set().lock().unwrap_or_else(|e| e.into_inner()).remove(&self.0);
    }
}

/// Whether a live operation of this process owns `id`.
pub fn is_live(id: &str) -> bool {
    live_set().lock().unwrap_or_else(|e| e.into_inner()).contains(id)
}

/// `<root>/R-<n>/ops/<op>.json` (the layout of `paths::op_path`).
pub fn op_path_in(root: &Path, id: &RepairId, op: &str) -> PathBuf {
    ops_dir_in(root, id).join(format!("{op}.json"))
}

/// A new operation id: `<UTC slot time>-<kind>-<pid>-<n>` (unique within the daemon; the file is
/// created new).
pub fn new_op_id(now: f64, kind: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    format!(
        "{}-{kind}-{}-{}",
        super::ledger::utc_slot_name(now),
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    )
}

/// Writes the record durably (E1). The first write of an operation happens BEFORE anything live
/// is touched; a failure here means nothing is applied.
pub fn write_in(root: &Path, rec: &OpRecord) -> Result<(), RepairError> {
    let p = op_path_in(root, &rec.repair, &rec.op);
    let mut r = rec.clone();
    r.schema = super::REPAIR_SCHEMA;
    super::durable::write_json_durable(&p, &r)
}

/// Reads one record (`None` = absent; unreadable / higher schema → `Corrupt`).
pub fn read_in(root: &Path, id: &RepairId, op: &str) -> Result<Option<OpRecord>, RepairError> {
    let p = op_path_in(root, id, op);
    let bytes = match std::fs::read(&p) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(RepairError::Corrupt(format!("{}: {e}", p.display()))),
    };
    let v: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|e| RepairError::Corrupt(format!("{}: {e}", p.display())))?;
    if v.get("schema").and_then(|s| s.as_u64()).unwrap_or(u64::MAX)
        > u64::from(super::REPAIR_SCHEMA)
    {
        return Err(RepairError::Corrupt(format!("{}: unknown schema", p.display())));
    }
    serde_json::from_value(v)
        .map(Some)
        .map_err(|e| RepairError::Corrupt(format!("{}: {e}", p.display())))
}

/// Every record of one repair (oldest first by name), unreadable ones skipped and listed.
pub fn list_in(root: &Path, id: &RepairId) -> (Vec<OpRecord>, Vec<String>) {
    let dir = ops_dir_in(root, id);
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return (Vec::new(), Vec::new());
    };
    let mut names: Vec<String> = rd
        .filter_map(Result::ok)
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        .filter_map(|n| n.strip_suffix(".json").map(str::to_string))
        .filter(|n| !n.starts_with('.'))
        .collect();
    names.sort();
    let mut out = Vec::new();
    let mut broken = Vec::new();
    for n in names {
        match read_in(root, id, &n) {
            Ok(Some(r)) => out.push(r),
            Ok(None) => {}
            Err(e) => broken.push(format!("{e:?}")),
        }
    }
    (out, broken)
}

/// The lane's unfinished records across every repair (the job resumes them).
pub fn unfinished_for_lane(root: &Path, lane: &LaneKey) -> Vec<OpRecord> {
    let mut out = Vec::new();
    for id in super::ledger::list_ids_in(root) {
        let (recs, _) = list_in(root, &id);
        out.extend(
            recs.into_iter()
                .filter(|r| r.lane_key == *lane && !r.finished),
        );
    }
    out
}

/// Advances a record to `step` (durable).
pub fn set_step(root: &Path, rec: &mut OpRecord, step: &str, now: f64) -> Result<(), RepairError> {
    rec.step = step.to_string();
    rec.updated_at = now;
    write_in(root, rec)
}

/// Closes a record with an outcome (durable).
pub fn finish(
    root: &Path,
    rec: &mut OpRecord,
    outcome: &str,
    note: &str,
    now: f64,
) -> Result<(), RepairError> {
    rec.step = "done".to_string();
    rec.finished = true;
    rec.outcome = Some(outcome.to_string());
    // EJ3: a completed put-back owes its notice — in THIS write, the one that makes it final
    if rec.kind == "m10-revert" && outcome == "applied" && rec.notice.is_none() {
        rec.notice = Some("due".to_string());
    }
    if !note.is_empty() {
        rec.note = note.chars().take(1000).collect();
    }
    rec.updated_at = now;
    write_in(root, rec)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_round_trip_and_unfinished_scan() {
        let root = std::env::temp_dir().join(format!(
            "cys-wal-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let id = RepairId::parse("R-2").unwrap();
        std::fs::create_dir_all(super::super::ledger::repair_dir_in(&root, &id)).unwrap();
        let lane = LaneKey("base".into());
        let mut r = OpRecord {
            schema: 1,
            op: new_op_id(1_800_000_000.0, "revert"),
            repair: id.clone(),
            lane_key: lane.clone(),
            kind: "revert".into(),
            form: "exchange".into(),
            rel: "hooks/a.sh".into(),
            pre_existed: true,
            expected: "file:aa:0755".into(),
            incoming: "file:bb:0755".into(),
            pack_generation: Some(PackGeneration {
                version: "0.15.0".into(),
                pro_revision: None,
            }),
            slot: Some(root.join("s")),
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
        write_in(&root, &r).unwrap();
        assert_eq!(read_in(&root, &id, &r.op).unwrap().unwrap(), r);
        assert_eq!(unfinished_for_lane(&root, &lane).len(), 1);
        assert!(unfinished_for_lane(&root, &LaneKey("dept-x".into())).is_empty());
        finish(&root, &mut r, "applied", "", 2.0).unwrap();
        assert!(unfinished_for_lane(&root, &lane).is_empty());
        // T3's cleanup reads the same field (`finished: true`)
        let v: serde_json::Value =
            serde_json::from_slice(&std::fs::read(op_path_in(&root, &id, &r.op)).unwrap()).unwrap();
        assert_eq!(v["finished"], serde_json::json!(true));
        let _ = std::fs::remove_dir_all(&root);
    }
}
