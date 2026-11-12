//! Tombstones — recorded file deletions and non-lifeline hook unregistrations that survive updates
//! (C1 1.5, RULINGS B1: one file per lane, `repairs/tombstones/<lane_key>.json`, written only by
//! that lane's daemon). Bodies: T3 (writer API `record` / `remove_for_repair` for T3's commit, T6's
//! revert, T7b's propagation; reader `load` for T2). Callers treat `Err` as the empty set (today's
//! behaviour: deleted files come back).

use super::types::{LaneKey, RepairError, RepairId};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TombstoneKind {
    File,
    Hook { event: String, script: String },
}

/// One tombstone. A hook tombstone's `rel` is `settings:<event>:<script>` (RULINGS W3(b)).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tombstone {
    pub rel: String,
    pub kind: TombstoneKind,
    pub repair: RepairId,
}

/// The lane's tombstone file `repairs/tombstones/<lane_key>.json` (O-C1-8: the JSON schema of the
/// reader's output is this struct, field order = key order).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TombstoneFile {
    pub schema: u32,
    pub lane: LaneKey,
    pub tombstones: Vec<Tombstone>,
}

/// `<root>/tombstones/<lane>.json` (the layout of `paths::tombstones_path`, relative to a root).
pub fn path_in(root: &Path, lane: &LaneKey) -> PathBuf {
    root.join("tombstones").join(format!("{}.json", lane.0))
}

/// The lane's tombstones. Missing file = empty set. Unreadable / higher schema → `Err(Corrupt)`
/// (callers treat `Err` as the empty set = today's behaviour, the lifeline-safe direction, O-C1-6).
pub fn load(lane: &LaneKey) -> Result<Vec<Tombstone>, RepairError> {
    load_in(&super::paths::repairs_root(), lane)
}

/// [`load`] under an explicit repairs root.
pub fn load_in(root: &Path, lane: &LaneKey) -> Result<Vec<Tombstone>, RepairError> {
    Ok(read_file(root, lane)?
        .map(|f| f.tombstones)
        .unwrap_or_default())
}

fn read_file(root: &Path, lane: &LaneKey) -> Result<Option<TombstoneFile>, RepairError> {
    let p = path_in(root, lane);
    let bytes = match std::fs::read(&p) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(RepairError::Corrupt(format!("{}: {e}", p.display()))),
    };
    let v: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|e| RepairError::Corrupt(format!("{}: {e}", p.display())))?;
    let schema = v.get("schema").and_then(|s| s.as_u64()).unwrap_or(u64::MAX);
    if schema > u64::from(super::REPAIR_SCHEMA) {
        return Err(RepairError::Corrupt(format!(
            "{}: unknown schema {schema}",
            p.display()
        )));
    }
    let f: TombstoneFile = serde_json::from_value(v)
        .map_err(|e| RepairError::Corrupt(format!("{}: {e}", p.display())))?;
    if f.lane != *lane {
        return Err(RepairError::Corrupt(format!(
            "{}: belongs to lane {} (not {})",
            p.display(),
            f.lane.0,
            lane.0
        )));
    }
    Ok(Some(f))
}

fn write_file(root: &Path, lane: &LaneKey, list: Vec<Tombstone>) -> Result<(), RepairError> {
    let p = path_in(root, lane);
    let f = TombstoneFile {
        schema: super::REPAIR_SCHEMA,
        lane: lane.clone(),
        tombstones: list,
    };
    crate::atomic_write_json(&p, &f).map_err(|e| RepairError::Io(format!("{}: {e}", p.display())))
}

/// Records one tombstone in the lane's file (written only by that lane's daemon — B1). Idempotent:
/// the same (rel, kind, repair) is kept once. An unreadable file is never overwritten (refused).
pub fn record(lane: &LaneKey, t: &Tombstone) -> Result<(), RepairError> {
    record_in(&super::paths::repairs_root(), lane, t)
}

/// [`record`] under an explicit repairs root.
pub fn record_in(root: &Path, lane: &LaneKey, t: &Tombstone) -> Result<(), RepairError> {
    let lane_mutex = lane_mutex(root, lane);
    let _lane_guard = lane_mutex.lock().unwrap_or_else(|e| e.into_inner());
    let mut list = read_file(root, lane)?
        .map(|f| f.tombstones)
        .unwrap_or_default();
    if list.iter().any(|x| x == t) {
        return Ok(());
    }
    list.push(t.clone());
    write_file(root, lane, list)
}

/// Removes every tombstone of one repair from the lane's file (same writer; a revert). Missing file
/// = nothing to remove. An unreadable file is refused (never overwritten).
pub fn remove_for_repair(lane: &LaneKey, id: &RepairId) -> Result<(), RepairError> {
    remove_for_repair_in(&super::paths::repairs_root(), lane, id)
}

/// [`remove_for_repair`] under an explicit repairs root.
pub fn remove_for_repair_in(root: &Path, lane: &LaneKey, id: &RepairId) -> Result<(), RepairError> {
    let lane_mutex = lane_mutex(root, lane);
    let _lane_guard = lane_mutex.lock().unwrap_or_else(|e| e.into_inner());
    let Some(f) = read_file(root, lane)? else {
        return Ok(());
    };
    let before = f.tombstones.len();
    let kept: Vec<Tombstone> = f
        .tombstones
        .into_iter()
        .filter(|t| t.repair != *id)
        .collect();
    if kept.len() == before {
        return Ok(());
    }
    write_file(root, lane, kept)
}

/// RULINGS AD3 / AK3: removes ONE repair's FILE tombstone of one path from the lane's file (that
/// repair committed the path present again). Other repairs' tombstones and Hook tombstones are left
/// alone. Missing file = nothing to remove.
pub fn remove_file_rel(lane: &LaneKey, rel: &str, repair: &RepairId) -> Result<(), RepairError> {
    remove_file_rel_in(&super::paths::repairs_root(), lane, rel, repair)
}

/// [`remove_file_rel`] under an explicit repairs root.
pub fn remove_file_rel_in(
    root: &Path,
    lane: &LaneKey,
    rel: &str,
    repair: &RepairId,
) -> Result<(), RepairError> {
    let lane_mutex = lane_mutex(root, lane);
    let _lane_guard = lane_mutex.lock().unwrap_or_else(|e| e.into_inner());
    let Some(f) = read_file(root, lane)? else {
        return Ok(());
    };
    let before = f.tombstones.len();
    let kept: Vec<Tombstone> = f
        .tombstones
        .into_iter()
        .filter(|t| !(t.rel == rel && t.kind == TombstoneKind::File && t.repair == *repair))
        .collect();
    if kept.len() == before {
        return Ok(());
    }
    write_file(root, lane, kept)
}

/// RULINGS AD3: ONE lane-level mutex for every read-modify-write of a lane's tombstone file —
/// commit (T3), revert (T6) and propagation (T7b) all reach the file through the functions of this
/// module, so they are serialised here (in memory, per process; the one writer per file stays the
/// lane's daemon — B1). Poison is tolerated (house rule).
fn lane_mutex(root: &Path, lane: &LaneKey) -> std::sync::Arc<std::sync::Mutex<()>> {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, OnceLock};
    static LOCKS: OnceLock<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>> = OnceLock::new();
    let mut map = LOCKS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    Arc::clone(map.entry(path_in(root, lane)).or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(tag: u32) -> PathBuf {
        let d = std::env::temp_dir().join(format!("cys-repair-tomb-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn id(s: &str) -> RepairId {
        RepairId::parse(s).unwrap()
    }

    /// One file per lane; record is idempotent; remove_for_repair removes only that repair's
    /// entries; each lane reads only its own file (B1).
    #[test]
    fn per_lane_writer_and_reader() {
        let r = root(line!());
        let base = LaneKey("base".into());
        let dept = LaneKey("dept-2".into());
        assert!(
            load_in(&r, &base).unwrap().is_empty(),
            "missing file = empty set"
        );
        let file_t = Tombstone {
            rel: "bin/old.py".into(),
            kind: TombstoneKind::File,
            repair: id("R-1"),
        };
        let hook_t = Tombstone {
            rel: "settings:Stop:x.sh".into(),
            kind: TombstoneKind::Hook {
                event: "Stop".into(),
                script: "x.sh".into(),
            },
            repair: id("R-2"),
        };
        record_in(&r, &base, &file_t).unwrap();
        record_in(&r, &base, &file_t).unwrap();
        record_in(&r, &base, &hook_t).unwrap();
        record_in(&r, &dept, &hook_t).unwrap();
        assert_eq!(
            load_in(&r, &base).unwrap(),
            vec![file_t.clone(), hook_t.clone()]
        );
        assert_eq!(load_in(&r, &dept).unwrap(), vec![hook_t.clone()]);
        assert!(path_in(&r, &base).ends_with("tombstones/base.json"));
        remove_for_repair_in(&r, &base, &id("R-2")).unwrap();
        assert_eq!(load_in(&r, &base).unwrap(), vec![file_t.clone()]);
        assert_eq!(
            load_in(&r, &dept).unwrap(),
            vec![hook_t],
            "another lane's file untouched"
        );
        remove_for_repair_in(&r, &LaneKey("nobody".into()), &id("R-1")).unwrap();
        let v: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path_in(&r, &base)).unwrap()).unwrap();
        let keys: Vec<&String> = v.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["lane", "schema", "tombstones"]);
        let _ = std::fs::remove_dir_all(&r);
    }

    /// RULINGS AD3: concurrent writers of one lane (commit / revert / propagation threads) lose no
    /// entry — the lane-level mutex serialises the read-modify-write; `remove_file_rel` removes only
    /// that repair's FILE tombstone of that path (AK3).
    #[test]
    fn lane_mutex_serialises_writers_and_rel_removal() {
        let r = root(line!());
        let lane = LaneKey("base".into());
        std::thread::scope(|sc| {
            for t in 0..8u64 {
                let (r, lane) = (r.clone(), lane.clone());
                sc.spawn(move || {
                    for k in 0..10u64 {
                        let ts = Tombstone {
                            rel: format!("bin/t{t}-{k}.py"),
                            kind: TombstoneKind::File,
                            repair: id(&format!("R-{}", t + 1)),
                        };
                        record_in(&r, &lane, &ts).unwrap();
                    }
                });
            }
        });
        assert_eq!(
            load_in(&r, &lane).unwrap().len(),
            80,
            "a concurrent writer lost entries"
        );
        let hook = Tombstone {
            rel: "bin/t0-0.py".into(),
            kind: TombstoneKind::Hook {
                event: "Stop".into(),
                script: "x.sh".into(),
            },
            repair: id("R-9"),
        };
        record_in(&r, &lane, &hook).unwrap();
        // AK3: another repair's File tombstone of the same path stays
        let other = Tombstone {
            rel: "bin/t0-0.py".into(),
            kind: TombstoneKind::File,
            repair: id("R-5"),
        };
        record_in(&r, &lane, &other).unwrap();
        remove_file_rel_in(&r, &lane, "bin/t0-0.py", &id("R-1")).unwrap();
        let left = load_in(&r, &lane).unwrap();
        assert_eq!(left.len(), 81);
        assert!(
            left.contains(&hook),
            "a hook tombstone is not removed by path"
        );
        assert!(
            left.contains(&other),
            "another repair's tombstone was removed"
        );
        assert!(!left.iter().any(|t| t.rel == "bin/t0-0.py"
            && t.kind == TombstoneKind::File
            && t.repair == id("R-1")));
        let _ = std::fs::remove_dir_all(&r);
    }

    /// Unreadable / higher schema / foreign lane → Err (callers: empty set); never overwritten.
    #[test]
    fn unreadable_file_is_err_and_kept() {
        let r = root(line!());
        let base = LaneKey("base".into());
        let p = path_in(&r, &base);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        for bad in [
            &b"{oops"[..],
            br#"{"schema":2,"lane":"base","tombstones":[]}"#,
            br#"{"schema":1,"lane":"dept-9","tombstones":[]}"#,
        ] {
            std::fs::write(&p, bad).unwrap();
            assert!(matches!(load_in(&r, &base), Err(RepairError::Corrupt(_))));
            let t = Tombstone {
                rel: "a".into(),
                kind: TombstoneKind::File,
                repair: id("R-3"),
            };
            assert!(record_in(&r, &base, &t).is_err());
            assert!(remove_for_repair_in(&r, &base, &id("R-3")).is_err());
            assert_eq!(
                std::fs::read(&p).unwrap(),
                bad,
                "an unreadable tombstone file was overwritten"
            );
        }
        let _ = std::fs::remove_dir_all(&r);
    }
}
