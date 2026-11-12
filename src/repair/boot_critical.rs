//! The compiled boot-critical list (C4 · M7). T0: the entry type, an empty table, a stub query, the
//! six lifeline hooks and the check-timeout clamp. T5a includes the build-time generated table.

use super::types::RepairError;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Entry category (O-C4-3 spellings).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EntryCategory {
    Hook,
    HookDep,
    StartScript,
    StartDep,
    Config,
    Protection,
}

/// How a listed file is checked (O-C4-3 spellings).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CheckKind {
    ShSyntax,
    PyCompile,
    JsonParse,
    SelfTest,
    HookReplay,
    NonePrerehearsal,
}

/// One entry of the compiled list — FILES only (RULINGS Y3: no `settings:<event>:<hook>` entry).
/// `timeout_s` only for `hook-replay` (the hook's registered timeout; the consumer clamps it with
/// [`clamp_check_timeout`]); `lifeline` mirrors the six lifeline hooks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    pub rel: &'static str,
    pub category: EntryCategory,
    pub check: CheckKind,
    pub check_arg: &'static str,
    pub timeout_s: Option<u32>,
    pub lifeline: bool,
}

/// The compiled list. T0: empty (T5a includes the generated table).
pub fn entries() -> &'static [Entry] {
    &[]
}

/// Answer of the boot-critical query (O-C4-2: exit 0 yes, 1 no, 2 cannot decide = fail-open).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Yes,
    No,
    Undecidable,
}

/// Whether a path is boot-critical. Stub in T0 (body: T5a).
pub fn is_boot_critical(path: &Path) -> Result<Decision, RepairError> {
    let _ = path;
    Err(RepairError::NotImplemented(
        "boot_critical::is_boot_critical",
    ))
}

/// The six lifeline hooks (design 8.7, D:883) — always re-registered, never tombstonable.
pub const LIFELINE_HOOKS: [&str; 6] = [
    "session-start.sh",
    "role-bootstrap.sh",
    "inject-context.sh",
    "save-state.sh",
    "cys-statusline.sh",
    "role-capability-gate.sh",
];

/// Deadline of one check child in seconds (RULINGS B3): a hook replay uses the hook's registered
/// timeout clamped to 10–60 s; no declared timeout → 60 s.
pub fn clamp_check_timeout(declared: Option<u32>) -> u32 {
    match declared {
        None => 60,
        Some(s) => s.clamp(10, 60),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The six lifeline names (D:883) and the B3 clamp.
    #[test]
    fn lifelines_and_clamp() {
        assert_eq!(
            LIFELINE_HOOKS,
            [
                "session-start.sh",
                "role-bootstrap.sh",
                "inject-context.sh",
                "save-state.sh",
                "cys-statusline.sh",
                "role-capability-gate.sh"
            ]
        );
        // Each lifeline is a hook the pack ships (a renamed hook would silently lose its protection).
        for h in LIFELINE_HOOKS {
            let rel = format!("hooks/{h}");
            assert!(
                crate::pack::PACK_ALL.iter().any(|(r, _)| *r == rel),
                "lifeline {rel} is not in the embedded pack"
            );
        }
        assert_eq!(clamp_check_timeout(None), 60);
        assert_eq!(clamp_check_timeout(Some(5)), 10);
        assert_eq!(clamp_check_timeout(Some(30)), 30);
        assert_eq!(clamp_check_timeout(Some(600)), 60);
        assert_eq!(clamp_check_timeout(Some(10)), 10);
        assert_eq!(clamp_check_timeout(Some(60)), 60);
        assert_eq!(clamp_check_timeout(Some(0)), 10);
        assert!(
            entries().is_empty(),
            "T0 ships an empty boot-critical table (T5a fills it)"
        );
    }
}
