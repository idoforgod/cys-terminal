//! Path helpers of the repair ledger, vault and side areas (C1 O-C1-3 / RULINGS B1, F1, G4, I8,
//! K8, L3, U3; C2 2.1; C7 O-C7-4; C8 O-C8-4; C10 N-5; C11 O-C11-1; C12 O-C12-3). Pure joins — no
//! directory is created here.

use super::types::{LaneKey, RepairId};
use std::path::{Path, PathBuf};

/// `~/.cys/local/repairs` (= `crate::pack::local_dir().join("repairs")`; override `CYS_LOCAL_DIR`).
pub fn repairs_root() -> PathBuf {
    crate::pack::local_dir().join("repairs")
}

/// `repairs/R-<n>/`
pub fn repair_dir(id: &RepairId) -> PathBuf {
    repairs_root().join(id.as_str())
}

/// `repairs/R-<n>/record.json` — written only by the lock holder's (origin) daemon.
pub fn record_path(id: &RepairId) -> PathBuf {
    repair_dir(id).join("record.json")
}

/// `repairs/R-<n>/lanes/<lane_key>.json` — written (and created) only by that lane's daemon.
pub fn lane_state_path(id: &RepairId, lane: &LaneKey) -> PathBuf {
    repair_dir(id)
        .join("lanes")
        .join(format!("{}.json", lane.0))
}

/// `repairs/R-<n>/ops/<op-id>.json` — one write-ahead operation record per operation.
pub fn op_path(id: &RepairId, op_id: &str) -> PathBuf {
    repair_dir(id).join("ops").join(format!("{op_id}.json"))
}

/// `repairs/intake/<lane_key>/<sig>.json` (RULINGS G4 — per lane, aggregated on read).
pub fn intake_path(lane: &LaneKey, sig_hex: &str) -> PathBuf {
    repairs_root()
        .join("intake")
        .join(&lane.0)
        .join(format!("{sig_hex}.json"))
}

/// `repairs/tombstones/<lane_key>.json` (RULINGS B1 — one file per lane).
pub fn tombstones_path(lane: &LaneKey) -> PathBuf {
    repairs_root()
        .join("tombstones")
        .join(format!("{}.json", lane.0))
}

/// `repairs/grant.json` — the owner's decision and its provenance only (B1).
pub fn grant_path() -> PathBuf {
    repairs_root().join("grant.json")
}

/// `repairs/grant-seq/` — the one global claim sequence (RULINGS F1 / G1–G3).
pub fn grant_seq_dir() -> PathBuf {
    repairs_root().join("grant-seq")
}

/// `repairs/grant-seq/<n>.json`
pub fn grant_claim_path(n: u64) -> PathBuf {
    grant_seq_dir().join(format!("{n}.json"))
}

/// `repairs/grant-seq/<n>.closed`
pub fn grant_closed_path(n: u64) -> PathBuf {
    grant_seq_dir().join(format!("{n}.closed"))
}

/// `repairs/lanes/<lane_key>.job.json` (O-C7-4).
pub fn job_state_path(lane: &LaneKey) -> PathBuf {
    repairs_root()
        .join("lanes")
        .join(format!("{}.job.json", lane.0))
}

/// The pack-instance id file name (RULINGS I8). It starts with `.`, so build.rs never embeds it,
/// it is never in the install manifest and never pruned.
pub const PACK_INSTANCE_FILE: &str = ".repair-pack-id";

/// `<pack>/.repair-pack-id` — inside the lane's pack, never under `repairs/` (I8).
pub fn pack_instance_path(pack_dir: &Path) -> PathBuf {
    pack_dir.join(PACK_INSTANCE_FILE)
}

/// `repairs/R-<n>/staging/` — the repair's fixed working copy (RULINGS U3).
pub fn staging_dir(id: &RepairId) -> PathBuf {
    repair_dir(id).join("staging")
}

/// `repairs/displaced/<R-id>/<시각>/` — one write-once vault slot (C2 2.1, O-C2-1).
pub fn vault_slot_dir(id: &RepairId, ts: &str) -> PathBuf {
    repairs_root().join("displaced").join(id.as_str()).join(ts)
}

/// `…/<시각>/sealed/<basename>` — the sealed copy (RULINGS H1, L3).
pub fn vault_sealed_path(id: &RepairId, ts: &str, basename: &str) -> PathBuf {
    vault_slot_dir(id, ts).join("sealed").join(basename)
}

/// `…/<시각>/payload/<basename>` — the exchanged path (RULINGS L3).
pub fn vault_payload_path(id: &RepairId, ts: &str, basename: &str) -> PathBuf {
    vault_slot_dir(id, ts).join("payload").join(basename)
}

/// `…/<시각>/meta/pre.json` (RULINGS L3).
pub fn vault_meta_pre_path(id: &RepairId, ts: &str) -> PathBuf {
    vault_slot_dir(id, ts).join("meta").join("pre.json")
}

/// `…/<시각>/meta/sealed.json` (RULINGS K8, L3).
pub fn vault_meta_sealed_path(id: &RepairId, ts: &str) -> PathBuf {
    vault_slot_dir(id, ts).join("meta").join("sealed.json")
}

/// `local/update-snapshots/<lane_key>/<from-version>-<ts>/` (O-C8-4).
pub fn update_snapshot_dir(lane: &LaneKey, from_version: &str, ts: &str) -> PathBuf {
    crate::pack::local_dir()
        .join("update-snapshots")
        .join(&lane.0)
        .join(format!("{from_version}-{ts}"))
}

/// `local/bundle-edits/<old-version>-<UTC ts>/` (O-C12-3).
pub fn bundle_edits_dir(old_version: &str, ts: &str) -> PathBuf {
    crate::pack::local_dir()
        .join("bundle-edits")
        .join(format!("{old_version}-{ts}"))
}

/// `<pack>/directives/.ceo-receipt.json` (O-C11-1).
pub fn ceo_receipt_path(pack_dir: &Path) -> PathBuf {
    pack_dir.join("directives").join(".ceo-receipt.json")
}

/// `repairs/owner-tokens/<token_sha256>.json` (N-5, RULINGS F11).
pub fn owner_token_path(token_sha256: &str) -> PathBuf {
    repairs_root()
        .join("owner-tokens")
        .join(format!("{token_sha256}.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rel(parts: &[&str]) -> PathBuf {
        parts.iter().collect()
    }

    fn id(s: &str) -> RepairId {
        serde_json::from_value(serde_json::json!(s)).expect("RepairId from a JSON string")
    }

    /// Every helper, under a temporary `CYS_LOCAL_DIR` (and a temporary pack directory for the two
    /// pack-relative helpers), yields exactly the contract's relative path.
    #[test]
    fn layout_is_contract() {
        let _lock = crate::pack::PACK_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let local = std::env::temp_dir().join(format!("cys-repair-layout-{}", std::process::id()));
        let _env = crate::pack::EnvGuard::set("CYS_LOCAL_DIR", &local);
        assert_eq!(
            crate::pack::local_dir(),
            local,
            "CYS_LOCAL_DIR override not honoured"
        );
        let r = id("R-7");
        let lane = LaneKey("dept-dept-3".to_string());
        let ts = "20261112T120000123456Z";
        let under = |p: PathBuf| {
            p.strip_prefix(&local)
                .unwrap_or_else(|_| panic!("{} not under CYS_LOCAL_DIR", p.display()))
                .to_path_buf()
        };
        let cases: Vec<(PathBuf, PathBuf)> = vec![
            (repairs_root(), rel(&["repairs"])),
            (repair_dir(&r), rel(&["repairs", "R-7"])),
            (record_path(&r), rel(&["repairs", "R-7", "record.json"])),
            (
                lane_state_path(&r, &lane),
                rel(&["repairs", "R-7", "lanes", "dept-dept-3.json"]),
            ),
            (
                op_path(&r, "op-1"),
                rel(&["repairs", "R-7", "ops", "op-1.json"]),
            ),
            (
                intake_path(&lane, "ab12"),
                rel(&["repairs", "intake", "dept-dept-3", "ab12.json"]),
            ),
            (
                tombstones_path(&lane),
                rel(&["repairs", "tombstones", "dept-dept-3.json"]),
            ),
            (grant_path(), rel(&["repairs", "grant.json"])),
            (grant_seq_dir(), rel(&["repairs", "grant-seq"])),
            (
                grant_claim_path(4),
                rel(&["repairs", "grant-seq", "4.json"]),
            ),
            (
                grant_closed_path(4),
                rel(&["repairs", "grant-seq", "4.closed"]),
            ),
            (
                job_state_path(&lane),
                rel(&["repairs", "lanes", "dept-dept-3.job.json"]),
            ),
            (staging_dir(&r), rel(&["repairs", "R-7", "staging"])),
            (
                vault_slot_dir(&r, ts),
                rel(&["repairs", "displaced", "R-7", ts]),
            ),
            (
                vault_payload_path(&r, ts, "x.py"),
                rel(&["repairs", "displaced", "R-7", ts, "payload", "x.py"]),
            ),
            (
                vault_sealed_path(&r, ts, "x.py"),
                rel(&["repairs", "displaced", "R-7", ts, "sealed", "x.py"]),
            ),
            (
                vault_meta_pre_path(&r, ts),
                rel(&["repairs", "displaced", "R-7", ts, "meta", "pre.json"]),
            ),
            (
                vault_meta_sealed_path(&r, ts),
                rel(&["repairs", "displaced", "R-7", ts, "meta", "sealed.json"]),
            ),
            (
                owner_token_path("cafe"),
                rel(&["repairs", "owner-tokens", "cafe.json"]),
            ),
            (
                update_snapshot_dir(&lane, "0.14.56", ts),
                rel(&["update-snapshots", "dept-dept-3", &format!("0.14.56-{ts}")]),
            ),
            (
                bundle_edits_dir("0.14.56", ts),
                rel(&["bundle-edits", &format!("0.14.56-{ts}")]),
            ),
        ];
        for (got, want) in cases {
            assert_eq!(under(got.clone()), want, "{}", got.display());
        }
        // L3: a payload or sealed copy named like a metadata file never equals a metadata path.
        for base in ["meta.json", "pre.json", "sealed.json"] {
            for p in [
                vault_payload_path(&r, ts, base),
                vault_sealed_path(&r, ts, base),
            ] {
                assert_ne!(p, vault_meta_pre_path(&r, ts));
                assert_ne!(p, vault_meta_sealed_path(&r, ts));
            }
        }
        // Pack-relative helpers (I8, O-C11-1) live inside the given pack, never under repairs/.
        let pack =
            std::env::temp_dir().join(format!("cys-repair-layout-pack-{}", std::process::id()));
        assert_eq!(
            pack_instance_path(&pack).strip_prefix(&pack).unwrap(),
            rel(&[".repair-pack-id"])
        );
        assert_eq!(
            ceo_receipt_path(&pack).strip_prefix(&pack).unwrap(),
            rel(&["directives", ".ceo-receipt.json"])
        );
        assert!(!pack_instance_path(&pack).starts_with(repairs_root()));
    }

    /// I8: the pack-instance file is never shipped — it starts with `.`, no embedded pack rel
    /// equals it, and no embedded rel has a dot component at all (the build.rs 56–59 rule that
    /// keeps it out of the embedded pack, the manifest and the prune set).
    #[test]
    fn pack_instance_file_is_never_shipped() {
        assert!(PACK_INSTANCE_FILE.starts_with('.'));
        assert!(!PACK_INSTANCE_FILE.contains('/') && !PACK_INSTANCE_FILE.contains('\\'));
        let all = crate::pack::PACK_ALL;
        assert!(
            all.len() >= 250,
            "embedded pack has only {} entries",
            all.len()
        );
        for (rel, _) in all {
            assert_ne!(
                *rel, PACK_INSTANCE_FILE,
                "the pack embeds {PACK_INSTANCE_FILE}"
            );
            assert!(
                !rel.split('/').any(|c| c.starts_with('.')),
                "embedded rel {rel} has a dot component — build.rs would no longer keep {PACK_INSTANCE_FILE} out"
            );
        }
    }
}
