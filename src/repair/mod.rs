//! 「스스로 해결」(self-repair, release 0.15.0) — the shared interface of the repair ledger.
//!
//! This module is the T0 SKELETON (impl/TRACKS.md §6): shared types, constants, path helpers and
//! the bodies that ARE the contract (labels, paths, the platform switch, role tables, error codes,
//! `TargetState`, `fingerprint`). Every other function is a stub that returns
//! [`types::RepairError::NotImplemented`]; the later tracks fill those bodies in place and never
//! change a name, a signature or a path here without a contract change (TRACKS §7 / RULINGS D1).
//!
//! Layout: the ledger lives under `crate::pack::local_dir().join("repairs")` (C1 1.1, O-C1-3);
//! the CLI never writes it (C1 1.15) — every verb is a daemon request (`repair.*`).

pub mod apply;
pub mod boot_critical;
pub mod consumed;
pub mod durable;
pub mod exchange;
pub mod grant;
pub mod guard;
pub mod intake;
pub mod ledger;
pub mod lock;
pub mod owner_origin;
pub mod paths;
pub mod platform;
/// T6 — the appended job's work: probation re-check, M10, the start-time pass.
pub mod probation;
/// T7b — department propagation: each lane's own rows, the pack-instance pass, the gate's reader.
pub mod propagate;
pub mod reapply_claim;
pub mod rehearsal;
/// Track T5b — the replay rules shared by the rehearsal runner and the screen (C4 BH2, BK1, BL1).
pub mod replay;
pub mod roles;
pub mod rules;
/// Track T5b — the protection screen beside the rehearsal runner (RULINGS AT9).
pub mod screen;
/// Track T5b — the screen's tables (C4 AY2): corpora G / R / C, the cell table, anchors.
pub mod screen_corpus;
pub mod tombstones;
pub mod types;
/// T6 — the vault's fresh write-once slots and the seal (C2).
pub mod vault;
/// T6 — write-ahead operation records (C1 1.13).
pub mod wal;

/// The ONE candidate reader (RULINGS V2(a)) — see [`ledger::candidate_states`].
pub use ledger::candidate_states;
/// The ONE fingerprint function (RULINGS T2 as amended by U2) — see [`types::fingerprint`].
pub use types::fingerprint;

/// Integer `schema` written into every ledger JSON (O-C1-7): readers treat a higher value as
/// unreadable, writers never downgrade.
pub const REPAIR_SCHEMA: u32 = 1;

/// The "not implemented in this build" sentence (N-2). The CLI prints
/// `"{NOT_IMPLEMENTED_KO}: cys repair <verb>"`, the daemon answers
/// `"repair.<sub>: {NOT_IMPLEMENTED_KO}"`, the app returns it as its `Err`.
pub const NOT_IMPLEMENTED_KO: &str = "이 빌드에는 아직 구현되지 않았습니다";

#[cfg(test)]
mod tests {
    /// Every `.rs` file under `src/repair/` (recursive), as (relative path, text).
    fn repair_sources() -> Vec<(String, String)> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("repair");
        let mut out = Vec::new();
        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir)
                .unwrap_or_else(|e| panic!("read_dir {}: {e}", dir.display()))
            {
                let path = entry.expect("directory entry").path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                    let text = std::fs::read_to_string(&path)
                        .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
                    let rel = path
                        .strip_prefix(&root)
                        .unwrap_or(&path)
                        .to_string_lossy()
                        .replace('\\', "/");
                    out.push((rel, text));
                }
            }
        }
        out.sort();
        out
    }

    /// F3 (RULINGS F3 / O-C2-3): the repair module references no symbol of the app-bundle module
    /// other than `swap_paths` — in particular never its delete-and-rename helper or the public
    /// install functions. Every occurrence of the module's name must be followed by
    /// `::swap_paths` (this also refuses `use …;` of the whole module and aliases).
    /// Passes trivially in T0 (nothing references it); binds T6.
    #[test]
    fn repair_module_uses_only_swap_paths_from_app_bundle() {
        // Built from pieces so this test's own source never contains the needle.
        let needle = concat!("app", "_bundle");
        let allowed = concat!("app", "_bundle", "::swap_paths");
        let files = repair_sources();
        assert!(
            files.len() >= 14,
            "scanned only {} files under src/repair — the scan is blind",
            files.len()
        );
        assert!(
            files.iter().any(|(n, _)| n == "exchange.rs"),
            "exchange.rs was not scanned: {files:?}",
            files = files.iter().map(|(n, _)| n).collect::<Vec<_>>()
        );
        // Byte offsets of every occurrence of the module name NOT followed by `::swap_paths`.
        let offenders = |text: &str| -> Vec<usize> {
            text.match_indices(needle)
                .map(|(at, _)| at)
                .filter(|at| !text[*at..].starts_with(allowed))
                .collect()
        };
        // The checker sees what it must refuse (synthetic controls; built from pieces like the needle).
        for refused in [
            format!("use crate::{needle};"),
            format!("crate::{needle}::two_step_replace(a, b)"),
            format!("use crate::{needle} as ab;"),
        ] {
            assert!(!offenders(&refused).is_empty(), "checker missed: {refused}");
        }
        assert!(offenders(&format!("crate::{allowed}(live, slot)")).is_empty());
        let (mut bad, mut seen_allowed) = (Vec::new(), 0usize);
        for (name, text) in &files {
            // Production code only (the test module holds this test's own name, which contains the needle).
            let text = &text[..text.find("#[cfg(test)]").unwrap_or(text.len())];
            seen_allowed += text.matches(allowed).count();
            for at in offenders(text) {
                let line = text[..at].matches('\n').count() + 1;
                bad.push(format!("{name}:{line}"));
            }
        }
        // Positive control: exchange.rs names the one allowed symbol (its doc in T0, the call from T6 on).
        assert!(
            seen_allowed >= 1,
            "the scan never saw `{allowed}` — it is not reading exchange.rs"
        );
        assert!(
            bad.is_empty(),
            "src/repair references an app-bundle symbol other than swap_paths: {bad:?}"
        );
    }

    /// TESTPLAN OQ-H-10: the test-only cargo feature `repair-test-auth` is never enabled by a
    /// release or branch workflow (neither by name nor through `--all-features`), and it is off in
    /// the default test build.
    #[test]
    fn repair_test_auth_never_enabled_in_release_workflows() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(".github")
            .join("workflows");
        // RULINGS DF5: EVERY workflow file (windows-health.yml and the other non-release ones run
        // cargo too), the four named release / branch workflows among them
        let mut all: Vec<String> = std::fs::read_dir(&root)
            .unwrap_or_else(|e| panic!("cannot list {}: {e}", root.display()))
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".yml") || n.ends_with(".yaml"))
            .collect();
        all.sort();
        for wf in ["release.yml", "pack-release.yml", "windows-build.yml", "ci-branch.yml"] {
            assert!(all.iter().any(|n| n == wf), "{wf} is not under .github/workflows");
        }
        assert!(all.len() >= 8, "only {} workflow files found: {all:?}", all.len());
        for wf in &all {
            let path = root.join(wf);
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
            assert!(
                text.len() > 1000,
                "{wf} is suspiciously short ({} bytes) — not the real workflow",
                text.len()
            );
            assert!(
                !text.contains("repair-test-auth"),
                "{wf} enables the test-only feature repair-test-auth"
            );
            assert!(
                !text.contains("--all-features"),
                "{wf} uses --all-features (would enable repair-test-auth)"
            );
        }
        assert!(
            !cfg!(feature = "repair-test-auth"),
            "repair-test-auth is enabled in the default test build"
        );
    }
}
