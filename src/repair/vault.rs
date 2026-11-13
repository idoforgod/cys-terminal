//! The vault — fresh, write-once slots outside every pack tree (C2 2.1–2.5; RULINGS L3, K8, H1, M2,
//! O5, E2–E4). Layout of one slot `repairs/displaced/<R-id>/<시각>[-n]/`: `payload/<basename>` (the
//! entry that is exchanged or moved in), `sealed/<basename>` (the sealed copy made right after the
//! exchange — the deposit of record), `meta/pre.json` (written before the exchange, write-once) and
//! `meta/sealed.json` (written after the seal, write-once). Nothing else is ever written into a slot
//! and nothing in a slot is ever deleted, moved or overwritten by this mechanism.
//!
//! O5: a slot is BUILT in a temporary sibling (`.building-…`, never a slot name) — its three folders,
//! the staged payload (exact mode, fresh mtime — E2/E3) and `meta/pre.json`, all fully flushed — and
//! then moved under its `<시각>` name with a no-replace rename; so a crash never leaves a half-made
//! slot directory (the temporary sibling is not a slot and holds no displaced bytes).

use super::durable::{full_flush_dir, full_flush_file};
use super::types::{LaneKey, RepairError, RepairId, TargetState};
use std::path::{Path, PathBuf};

/// What a slot's `meta/pre.json` says the slot holds (C2 O-C2-2 `kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DepositKind {
    /// Whatever was live at the instant of an exchange (apply, revert, undo, completion).
    Displaced,
    /// A copy of wanted bytes made to be put back (the "staged restore" of design step 2).
    StagedRestore,
    /// A live entry MOVED in (deposit by move: deletion, revert of a created file, Windows form).
    MovedIn,
}

impl DepositKind {
    fn wire(self) -> &'static str {
        match self {
            DepositKind::Displaced => "displaced",
            DepositKind::StagedRestore => "staged-restore",
            DepositKind::MovedIn => "moved-in",
        }
    }
}

/// What is staged into `payload/<basename>` while the slot is built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Staged {
    /// Nothing: the payload arrives later by a move (deposit by move).
    Empty,
    /// The bytes of `source` (read ONCE) with exactly this permission mode.
    FileFrom { source: PathBuf, mode: u32 },
    /// These bytes with exactly this permission mode.
    Bytes { bytes: Vec<u8>, mode: u32 },
    /// A symbolic link with this target.
    Link { target: String },
}

/// One built slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slot {
    pub dir: PathBuf,
    pub basename: String,
}

impl Slot {
    pub fn name(&self) -> String {
        self.dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string()
    }
    pub fn payload(&self) -> PathBuf {
        self.dir.join("payload").join(&self.basename)
    }
    pub fn sealed(&self) -> PathBuf {
        self.dir.join("sealed").join(&self.basename)
    }
    pub fn meta_pre(&self) -> PathBuf {
        self.dir.join("meta").join("pre.json")
    }
    pub fn meta_sealed(&self) -> PathBuf {
        self.dir.join("meta").join("sealed.json")
    }
    /// The slot of a recorded slot directory and target rel.
    pub fn at(dir: &Path, rel: &str) -> Slot {
        Slot {
            dir: dir.to_path_buf(),
            basename: basename(rel),
        }
    }
}

/// The last component of a pack-relative path.
pub fn basename(rel: &str) -> String {
    rel.rsplit(['/', '\\']).next().unwrap_or(rel).to_string()
}

fn io(p: &Path, e: impl std::fmt::Display) -> RepairError {
    RepairError::Io(format!("{}: {e}", p.display()))
}

/// `<root>/displaced/<R-id>/`
pub fn vault_dir_in(root: &Path, id: &RepairId) -> PathBuf {
    super::ledger::vault_dir_in(root, id)
}

fn create_new_with(path: &Path, bytes: &[u8], mode: u32) -> Result<(), RepairError> {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| io(path, e))?;
    f.write_all(bytes).map_err(|e| io(path, e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        f.set_permissions(std::fs::Permissions::from_mode(mode & 0o7777))
            .map_err(|e| io(path, e))?;
    }
    #[cfg(not(unix))]
    let _ = mode;
    full_flush_file(&f).map_err(|e| io(path, e))
}

/// Reads a regular file without following a link (the source of a staged copy is read ONCE).
fn read_regular(p: &Path) -> Result<Vec<u8>, RepairError> {
    let m = std::fs::symlink_metadata(p).map_err(|e| io(p, e))?;
    if !m.file_type().is_file() {
        return Err(RepairError::Precondition(format!(
            "{}: 일반 파일이 아닙니다",
            p.display()
        )));
    }
    std::fs::read(p).map_err(|e| io(p, e))
}

/// Builds one fresh slot for `rel` (O5: a temporary sibling, then a no-replace move under the
/// `<시각>` name, `-<n>` appended while the name exists — O-C2-1). `expected` is the state the
/// slot's deposit is expected to have (what `meta/pre.json` records); `staged` is placed in
/// `payload/<basename>` now. Everything is fully flushed before the slot gets its name.
#[allow(clippy::too_many_arguments)]
pub fn build_slot(
    root: &Path,
    id: &RepairId,
    lane: &LaneKey,
    rel: &str,
    op: &str,
    kind: DepositKind,
    expected: &TargetState,
    staged: &Staged,
    now: f64,
) -> Result<Slot, RepairError> {
    let vault = vault_dir_in(root, id);
    if std::fs::symlink_metadata(&vault).is_err() {
        std::fs::create_dir_all(&vault).map_err(|e| io(&vault, e))?;
        if let Some(p) = vault.parent() {
            full_flush_dir(p)?;
            if let Some(pp) = p.parent() {
                full_flush_dir(pp)?;
            }
        }
    }
    let base = basename(rel);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = vault.join(format!(".building-{}-{nanos}", std::process::id()));
    let build = || -> Result<(), RepairError> {
        std::fs::create_dir(&tmp).map_err(|e| io(&tmp, e))?;
        for sub in ["payload", "sealed", "meta"] {
            let d = tmp.join(sub);
            std::fs::create_dir(&d).map_err(|e| io(&d, e))?;
        }
        let payload = tmp.join("payload").join(&base);
        match staged {
            Staged::Empty => {}
            Staged::FileFrom { source, mode } => {
                let bytes = read_regular(source)?;
                create_new_with(&payload, &bytes, *mode)?;
            }
            Staged::Bytes { bytes, mode } => create_new_with(&payload, bytes, *mode)?,
            Staged::Link { target } => {
                #[cfg(unix)]
                std::os::unix::fs::symlink(target, &payload).map_err(|e| io(&payload, e))?;
                #[cfg(not(unix))]
                {
                    let _ = target;
                    return Err(RepairError::PlatformPending(
                        "링크는 이 운영체제에서 보관소로 옮기지 않습니다",
                    ));
                }
            }
        }
        let (sha256, mode, link_target) = match expected {
            TargetState::File { sha256, mode } => (sha256.clone(), *mode, None),
            TargetState::Link { target } => (String::new(), 0, Some(target.clone())),
            TargetState::Absent => (String::new(), 0, None),
        };
        let pre = serde_json::json!({
            "schema": super::REPAIR_SCHEMA,
            "repair": id.as_str(),
            "lane_key": lane.0,
            "rel": rel,
            "op": op,
            "kind": kind.wire(),
            "sha256": sha256,
            "mode": mode,
            "link_target": link_target,
        });
        let mut bytes =
            serde_json::to_vec_pretty(&pre).map_err(|e| RepairError::Io(e.to_string()))?;
        bytes.push(b'\n');
        create_new_with(&tmp.join("meta").join("pre.json"), &bytes, 0o644)?;
        for sub in ["payload", "sealed", "meta"] {
            full_flush_dir(&tmp.join(sub))?;
        }
        full_flush_dir(&tmp)
    };
    if let Err(e) = build() {
        let _ = std::fs::remove_dir_all(&tmp); // our own unfinished temporary sibling — never a slot
        return Err(e);
    }
    let stem = super::ledger::utc_slot_name(now);
    let mut n = 0u32;
    loop {
        let name = if n == 0 {
            stem.clone()
        } else {
            format!("{stem}-{n}")
        };
        let dir = vault.join(&name);
        match super::exchange::place_no_replace(&tmp, &dir) {
            Ok(()) => {
                full_flush_dir(&vault)?;
                return Ok(Slot { dir, basename: base });
            }
            Err(RepairError::Superseded(_)) if n < 999 => n += 1,
            Err(e) => {
                let _ = std::fs::remove_dir_all(&tmp);
                return Err(e);
            }
        }
    }
}

/// H1 / M2 — THE SEAL: a copy-on-write clone (`clonefile`; a byte copy where unavailable) of the
/// displaced entry `payload/<basename>` to `sealed/<basename>`. Called as the very FIRST filesystem
/// operation after an exchange or a deposit move returns (no flush, no hash, no record update in
/// between — the flushes follow). A link payload has nothing to seal (interpretation 30). Recovery
/// after a crash seals what it finds; an existing sealed copy is never rewritten (write-once).
pub fn seal(slot: &Slot) -> Result<(), RepairError> {
    let payload = slot.payload();
    let sealed = slot.sealed();
    if std::fs::symlink_metadata(&sealed).is_ok() {
        return Ok(());
    }
    let m = match std::fs::symlink_metadata(&payload) {
        Ok(m) => m,
        Err(e) => return Err(io(&payload, e)),
    };
    if m.file_type().is_symlink() {
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::ffi::OsStrExt;
        let cs = std::ffi::CString::new(payload.as_os_str().as_bytes())
            .map_err(|_| io(&payload, "NUL in path"))?;
        let cd = std::ffi::CString::new(sealed.as_os_str().as_bytes())
            .map_err(|_| io(&sealed, "NUL in path"))?;
        // SAFETY: two NUL-terminated paths alive for the call; CLONE_NOFOLLOW = 0x0001.
        if unsafe { libc::clonefile(cs.as_ptr(), cd.as_ptr(), 0x0001) } == 0 {
            return Ok(());
        }
    }
    // no copy-on-write here: a byte copy made the same way (create_new — never over anything)
    let bytes = std::fs::read(&payload).map_err(|e| io(&payload, e))?;
    #[cfg(unix)]
    let mode = {
        use std::os::unix::fs::PermissionsExt;
        m.permissions().mode() & 0o7777
    };
    #[cfg(not(unix))]
    let mode = 0u32;
    create_new_with(&sealed, &bytes, mode)
}

/// The post-seal facts written once into `meta/sealed.json` (K8 / L3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealFacts {
    /// The sealed copy's content hash (`None` for a link — interpretation 30).
    pub sealed_sha256: Option<String>,
    /// The deposit's state as sealed (a file's sha256 + mode, or the link).
    pub sealed_state: TargetState,
}

/// After [`seal`] and the parent flushes: the sealed copy made durable, its hash and the step-4
/// hash of the payload recorded in the write-once `meta/sealed.json`. An existing `sealed.json`
/// (recovery) is read back, never rewritten.
pub fn finish_seal(slot: &Slot, now: f64) -> Result<SealFacts, RepairError> {
    let sealed = slot.sealed();
    let payload = slot.payload();
    let (sealed_state, sealed_sha) = if std::fs::symlink_metadata(&sealed).is_ok() {
        if let Ok(f) = std::fs::File::open(&sealed) {
            full_flush_file(&f).map_err(|e| io(&sealed, e))?;
        }
        full_flush_dir(&slot.dir.join("sealed"))?;
        let st = TargetState::read(&sealed)?;
        let sha = match &st {
            TargetState::File { sha256, .. } => Some(sha256.clone()),
            _ => None,
        };
        (st, sha)
    } else {
        // a link payload: the link itself is the deposit
        (TargetState::read(&payload)?, None)
    };
    let meta = slot.meta_sealed();
    if std::fs::symlink_metadata(&meta).is_err() {
        let slot_now = match TargetState::read(&payload)? {
            TargetState::File { sha256, .. } => serde_json::json!(sha256),
            TargetState::Link { target } => serde_json::json!(format!("link:{target}")),
            TargetState::Absent => serde_json::Value::Null,
        };
        let link_seen = match &sealed_state {
            TargetState::Link { target } => serde_json::json!(target),
            _ => serde_json::Value::Null,
        };
        let v = serde_json::json!({
            "schema": super::REPAIR_SCHEMA,
            "sealed_sha256": sealed_sha,
            "slot_sha256_at_check": slot_now,
            "link_target_at_check": link_seen,
            "sealed_at": now,
        });
        let mut bytes = serde_json::to_vec_pretty(&v).map_err(|e| RepairError::Io(e.to_string()))?;
        bytes.push(b'\n');
        create_new_with(&meta, &bytes, 0o644)?;
        full_flush_dir(&slot.dir.join("meta"))?;
    }
    Ok(SealFacts {
        sealed_sha256: sealed_sha,
        sealed_state,
    })
}

/// The deposit of record of a slot as a STATE and the file that holds its bytes: the sealed copy
/// (verified against `sealed_sha256` — H1), else (a `begin` pre-image copy, which needs no seal)
/// the payload verified against `meta/pre.json`'s sha256. A link deposit is the payload link. A
/// hash that does not match → `Superseded` ("the deposit changed after it was recorded" — the
/// caller decides: the re-check makes that `check_timeout`, a revert refuses).
pub fn deposit_of(slot: &Slot) -> Result<(TargetState, PathBuf), RepairError> {
    let read_json = |p: &Path| -> Option<serde_json::Value> {
        std::fs::read(p)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
    };
    let sealed = slot.sealed();
    if std::fs::symlink_metadata(&sealed).is_ok() {
        let st = TargetState::read(&sealed)?;
        let want = read_json(&slot.meta_sealed())
            .and_then(|v| v.get("sealed_sha256").and_then(|s| s.as_str()).map(str::to_string));
        match (&st, want) {
            (TargetState::File { sha256, .. }, Some(w)) if *sha256 == w => return Ok((st, sealed)),
            _ => {
                return Err(RepairError::Superseded(format!(
                    "{}: 봉인 사본이 기록된 해시와 다릅니다",
                    sealed.display()
                )))
            }
        }
    }
    let payload = slot.payload();
    let st = TargetState::read(&payload)?;
    match &st {
        TargetState::Link { .. } => Ok((st, payload)),
        TargetState::File { sha256, .. } => {
            let pre = read_json(&slot.meta_pre());
            let want = pre
                .as_ref()
                .and_then(|v| v.get("sha256").and_then(|s| s.as_str()).map(str::to_string))
                .filter(|s| !s.is_empty());
            let sealed_recorded = read_json(&slot.meta_sealed()).is_some();
            if sealed_recorded {
                // a file payload with a sealed.json but no sealed copy is not a deposit of record
                return Err(RepairError::Superseded(format!(
                    "{}: 봉인 사본이 없습니다",
                    slot.dir.display()
                )));
            }
            match want {
                Some(w) if w == *sha256 => Ok((st, payload)),
                _ => Err(RepairError::Superseded(format!(
                    "{}: 보관 내용이 기록된 해시와 다릅니다",
                    payload.display()
                ))),
            }
        }
        TargetState::Absent => Err(RepairError::Superseded(format!(
            "{}: 보관 내용이 없습니다",
            payload.display()
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "cys-vault-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn rid(s: &str) -> RepairId {
        RepairId::parse(s).unwrap()
    }

    /// L3: a payload named `meta.json` (or `pre.json`) lives beside its metadata without any
    /// collision; O5: the slot appears complete under its name, the temporary sibling is gone.
    #[test]
    fn slot_layout_payload_named_like_metadata() {
        let root = tmp("l3");
        let id = rid("R-3");
        let lane = LaneKey("base".into());
        for rel in ["config/meta.json", "x/pre.json", "sealed.json"] {
            let exp = TargetState::File {
                sha256: "ab".repeat(32),
                mode: 0o644,
            };
            let s = build_slot(
                &root,
                &id,
                &lane,
                rel,
                "op-1",
                DepositKind::Displaced,
                &exp,
                &Staged::Bytes {
                    bytes: b"payload".to_vec(),
                    mode: 0o755,
                },
                1_800_000_000.0,
            )
            .unwrap();
            assert!(super::super::ledger::is_slot_name(&s.name()), "{}", s.name());
            assert_eq!(std::fs::read(s.payload()).unwrap(), b"payload");
            let pre: serde_json::Value =
                serde_json::from_slice(&std::fs::read(s.meta_pre()).unwrap()).unwrap();
            assert_eq!(pre["rel"], rel);
            assert_eq!(pre["kind"], "displaced");
            assert!(s.dir.join("sealed").is_dir());
        }
        let names: Vec<String> = std::fs::read_dir(vault_dir_in(&root, &id))
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(names.len(), 3, "{names:?}");
        assert!(names.iter().all(|n| !n.starts_with('.')), "a temporary sibling remained: {names:?}");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// E2/E3: the staged payload carries exactly the intended mode and a FRESH mtime (the source's
    /// old mtime is never carried over).
    #[cfg(unix)]
    #[test]
    fn staged_payload_has_intended_mode_and_fresh_mtime() {
        use std::os::unix::fs::PermissionsExt;
        let root = tmp("mode");
        let src = root.join("src.sh");
        std::fs::write(&src, b"#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&src, std::fs::Permissions::from_mode(0o600)).unwrap();
        let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000);
        std::fs::File::options()
            .write(true)
            .open(&src)
            .unwrap()
            .set_modified(old)
            .unwrap();
        let s = build_slot(
            &root,
            &rid("R-1"),
            &LaneKey("base".into()),
            "hooks/a.sh",
            "op",
            DepositKind::StagedRestore,
            &TargetState::Absent,
            &Staged::FileFrom {
                source: src.clone(),
                mode: 0o755,
            },
            1.0,
        )
        .unwrap();
        let m = std::fs::metadata(s.payload()).unwrap();
        assert_eq!(m.permissions().mode() & 0o7777, 0o755);
        assert!(m.modified().unwrap() > old + std::time::Duration::from_secs(86_400));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// H1/K8: the seal is a new write-once entry; a later write into the payload (an old
    /// descriptor) changes the payload only — the sealed copy and `sealed.json` stay; `deposit_of`
    /// returns the sealed copy; the E4 flag of `list_slots_in` sees the difference.
    #[test]
    fn seal_is_write_once_and_survives_old_descriptor_writes() {
        let root = tmp("seal");
        let id = rid("R-4");
        let s = build_slot(
            &root,
            &id,
            &LaneKey("base".into()),
            "hooks/b.sh",
            "op",
            DepositKind::Displaced,
            &TargetState::Absent,
            &Staged::Bytes {
                bytes: b"displaced".to_vec(),
                mode: 0o644,
            },
            2.0,
        )
        .unwrap();
        let mut held = std::fs::OpenOptions::new()
            .append(true)
            .open(s.payload())
            .unwrap();
        seal(&s).unwrap();
        let facts = finish_seal(&s, 3.0).unwrap();
        assert!(facts.sealed_sha256.is_some());
        {
            use std::io::Write;
            held.write_all(b"+late").unwrap();
        }
        seal(&s).unwrap(); // idempotent: the existing sealed copy is not rewritten
        assert_eq!(std::fs::read(s.sealed()).unwrap(), b"displaced");
        let (st, from) = deposit_of(&s).unwrap();
        assert_eq!(from, s.sealed());
        assert!(matches!(st, TargetState::File { .. }));
        let slots = super::super::ledger::list_slots_in(&root, &id);
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].changed_after_deposit, Some(true), "E4 flag");
        let _ = std::fs::remove_dir_all(&root);
    }
}
