//! The compiled boot-critical list (C4 · M7). T0: the entry type, the six lifeline hooks and the
//! check-timeout clamp. T5a: the build-time generated table (`OUT_DIR/boot_critical.rs`, written by
//! build.rs from `boot_critical_derive.rs` — the ONLY source, C4 4.1), the sibling protection rows
//! (D:941 / C4 4.10), the path normalisation (4.6 — only in the binary) and the LOCAL query
//! `cys repair boot-critical` (O-C4-2: no daemon request, so no autostart question — 4.7).

use super::types::RepairError;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

// `static GENERATED: &[Entry]` and `static GENERATED_WHY: &[&str]` (same order).
include!(concat!(env!("OUT_DIR"), "/boot_critical.rs"));

/// The derivation itself, compiled again for the tests (census, anchors, freshness pin).
#[cfg(test)]
#[path = "boot_critical_derive.rs"]
mod derive;

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

/// The compiled list: boot-critical rows (categories other than `protection`) and the sibling
/// protection rows (`protection`; a rel may appear in both). Sorted by (rel, category).
pub fn entries() -> &'static [Entry] {
    GENERATED
}

/// Why a row exists (the anchor or the binder that put it on the list), by row.
pub fn provenance(entry: &Entry) -> Option<&'static str> {
    GENERATED
        .iter()
        .position(|e| e == entry)
        .and_then(|i| GENERATED_WHY.get(i).copied())
}

/// A pack-relative rel in comparison form: `/` separators, no leading `./`, ASCII case folded
/// (APFS and NTFS are case-insensitive by default; folding can only make the answer "yes" for a
/// spelling that names a listed file on such a volume — the conservative direction).
fn rel_key(rel: &str) -> String {
    let mut r = rel.replace('\\', "/");
    while let Some(t) = r.strip_prefix("./") {
        r = t.to_string();
    }
    r.trim_start_matches('/').to_ascii_lowercase()
}

/// The boot-critical row (any category but `protection`) of a pack-relative rel.
pub fn boot_critical_entry(rel: &str) -> Option<&'static Entry> {
    let k = rel_key(rel);
    GENERATED
        .iter()
        .find(|e| e.category != EntryCategory::Protection && rel_key(e.rel) == k)
}

/// The protection row (D:941) of a pack-relative rel.
pub fn protection_entry(rel: &str) -> Option<&'static Entry> {
    let k = rel_key(rel);
    GENERATED
        .iter()
        .find(|e| e.category == EntryCategory::Protection && rel_key(e.rel) == k)
}

/// Answer of the boot-critical query (O-C4-2: exit 0 yes, 1 no, 2 cannot decide = fail-open).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Yes,
    No,
    Undecidable,
}

impl Decision {
    /// O-C4-2 exit code: 0 yes, 1 no, 2 cannot decide.
    pub fn exit_code(self) -> i32 {
        match self {
            Decision::Yes => 0,
            Decision::No => 1,
            Decision::Undecidable => 2,
        }
    }
    fn wire(self) -> &'static str {
        match self {
            Decision::Yes => "yes",
            Decision::No => "no",
            Decision::Undecidable => "undecidable",
        }
    }
}

/// Whether a path is boot-critical (O-C4-2). The binary resolves which lane's pack the path
/// belongs to (4.6); never `Err` today — "cannot decide" is `Ok(Decision::Undecidable)`.
pub fn is_boot_critical(path: &Path) -> Result<Decision, RepairError> {
    Ok(classify_path(path).decision)
}

/// Whether a path is one of the files implementing the four kept protections (D:941):
/// `Yes` when it lies in a lane's pack and its rel has a `protection` row.
pub fn is_protection_related(path: &Path) -> Result<Decision, RepairError> {
    let a = classify_path(path);
    Ok(match (a.decision, a.protection) {
        (Decision::Undecidable, _) => Decision::Undecidable,
        (_, Some(_)) => Decision::Yes,
        _ => Decision::No,
    })
}

/// The full answer for one path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathAnswer {
    /// boot-critical? (`protection` rows do not make it `Yes`)
    pub decision: Decision,
    /// the lane pack root the path lies in (as configured, not canonicalised)
    pub lane_pack: Option<PathBuf>,
    /// pack-relative path, `/` separators, original spelling
    pub rel: Option<String>,
    pub entry: Option<&'static Entry>,
    pub protection: Option<&'static Entry>,
    pub reason: String,
}

fn undecidable(reason: impl Into<String>) -> PathAnswer {
    PathAnswer {
        decision: Decision::Undecidable,
        lane_pack: None,
        rel: None,
        entry: None,
        protection: None,
        reason: reason.into(),
    }
}

/// The lane pack roots of this installation: the pack env overrides (`PACK_DIR_ENV_KEYS`),
/// `~/.cys/pack` and every `~/.cys/pack-dept-*` directory (one per department lane).
pub fn pack_roots() -> Vec<PathBuf> {
    let mut v = Vec::new();
    for k in crate::pack::PACK_DIR_ENV_KEYS {
        if let Ok(s) = std::env::var(k) {
            if !s.is_empty() {
                v.push(PathBuf::from(s));
            }
        }
    }
    if let Some(home) = dirs::home_dir() {
        let base = home.join(".cys");
        v.push(base.join("pack"));
        if let Ok(rd) = std::fs::read_dir(&base) {
            let mut depts: Vec<PathBuf> = rd
                .flatten()
                .filter(|e| {
                    e.file_name()
                        .to_str()
                        .is_some_and(|n| n.starts_with("pack-dept-"))
                        && e.path().is_dir()
                })
                .map(|e| e.path())
                .collect();
            depts.sort();
            v.extend(depts);
        }
    }
    v.dedup();
    v
}

/// Classifies a path against this installation's pack roots (cwd for relative paths).
pub fn classify_path(path: &Path) -> PathAnswer {
    let cwd = std::env::current_dir().ok();
    classify_with(path, cwd.as_deref(), &pack_roots(), cfg!(windows))
}

/// Lexical normalisation (4.6). Windows (`windows = true`): the `\\?\` / `\\?\UNC\` prefixes are
/// dropped, `\` becomes `/`, Git Bash `/c/…` and Cygwin `/cygdrive/c/…` become `C:/…`, the drive
/// letter is upper-cased. Both: `.` and `..` segments and repeated `/` are resolved.
pub fn normalize_lexical(s: &str, windows: bool) -> String {
    let mut t = s.to_string();
    if windows {
        if let Some(r) = t.strip_prefix(r"\\?\UNC\") {
            t = format!(r"\\{r}");
        } else if let Some(r) = t.strip_prefix(r"\\?\") {
            t = r.to_string();
        }
        t = t.replace('\\', "/");
        if let Some(r) = t.strip_prefix("/cygdrive/") {
            t = format!("/{r}");
        }
        let b = t.as_bytes();
        if b.len() >= 2
            && b[0] == b'/'
            && b[1].is_ascii_alphabetic()
            && (b.len() == 2 || b[2] == b'/')
        {
            t = format!("{}:{}", (b[1] as char).to_ascii_uppercase(), &t[2..]);
        }
        let b = t.as_bytes();
        if b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic() {
            t = format!("{}{}", (b[0] as char).to_ascii_uppercase(), &t[1..]);
        }
    }
    let (prefix, rest) = if let Some(r) = t.strip_prefix("//").filter(|_| windows) {
        ("//".to_string(), r)
    } else if windows && t.len() >= 2 && t.as_bytes()[1] == b':' {
        let p = t[..2].to_string() + "/";
        (p, t[2..].trim_start_matches('/'))
    } else if let Some(r) = t.strip_prefix('/') {
        ("/".to_string(), r)
    } else {
        (String::new(), t.as_str())
    };
    let mut parts: Vec<&str> = Vec::new();
    for c in rest.split('/') {
        match c {
            "" | "." => {}
            ".." => {
                if parts.last().is_some_and(|l| *l != "..") {
                    parts.pop();
                } else if prefix.is_empty() {
                    parts.push("..");
                }
            }
            other => parts.push(other),
        }
    }
    prefix + &parts.join("/")
}

fn is_absolute_form(s: &str, windows: bool) -> bool {
    s.starts_with('/')
        || (windows && s.len() >= 3 && s.as_bytes()[1] == b':' && s.as_bytes()[2] == b'/')
}

/// The OS-canonical form of an absolute path (symlinks resolved; on Windows also 8.3 short names
/// and case — `GetFinalPathNameByHandle` through `fs::canonicalize`); for a path that does not
/// exist yet (a Write that creates it), the canonical parent plus the leaf.
fn canonical_form(abs: &str, windows: bool) -> Option<String> {
    let p = Path::new(abs);
    if let Ok(c) = std::fs::canonicalize(p) {
        return Some(normalize_lexical(c.to_str()?, windows));
    }
    let leaf = p.file_name()?.to_str()?;
    let parent = std::fs::canonicalize(p.parent()?).ok()?;
    Some(normalize_lexical(
        &format!("{}/{leaf}", parent.to_str()?),
        windows,
    ))
}

/// A component that looks like an 8.3 short name (`PROGRA~1`).
fn has_short_name_component(s: &str) -> bool {
    s.split('/').any(|c| {
        c.char_indices()
            .any(|(i, ch)| ch == '~' && c[i + 1..].starts_with(|d: char| d.is_ascii_digit()))
    })
}

/// [`classify_path`] with explicit inputs (pure apart from `fs::canonicalize`).
pub fn classify_with(
    path: &Path,
    cwd: Option<&Path>,
    roots: &[PathBuf],
    windows: bool,
) -> PathAnswer {
    let Some(raw) = path.to_str() else {
        return undecidable("the path is not valid UTF-8");
    };
    if raw.trim().is_empty() {
        return undecidable("empty path");
    }
    if roots.is_empty() {
        return undecidable("no lane pack root (no home directory and no pack env)");
    }
    let lexical = normalize_lexical(raw, windows);
    let abs = if is_absolute_form(&lexical, windows) {
        lexical
    } else {
        let Some(c) = cwd.and_then(Path::to_str) else {
            return undecidable("relative path and no working directory");
        };
        normalize_lexical(&format!("{c}/{raw}"), windows)
    };
    let mut cands = vec![abs.clone()];
    if let Some(c) = canonical_form(&abs, windows) {
        if c != abs {
            cands.push(c);
        }
    }
    let mut root_forms: Vec<(String, &PathBuf)> = Vec::new();
    for r in roots {
        let Some(rs) = r.to_str() else { continue };
        let l = normalize_lexical(rs, windows);
        if let Some(c) = canonical_form(&l, windows) {
            root_forms.push((c, r));
        }
        root_forms.push((l, r));
    }
    // the longest root that is a whole-component prefix of a candidate
    let mut best: Option<(usize, String, &PathBuf)> = None;
    for c in &cands {
        let cl = c.to_ascii_lowercase();
        for (rf, r) in &root_forms {
            let rl = rf.trim_end_matches('/').to_ascii_lowercase();
            if rl.is_empty() || cl.len() <= rl.len() + 1 {
                continue;
            }
            if cl.starts_with(&rl) && cl.as_bytes()[rl.len()] == b'/' {
                let rel = c[rl.len() + 1..].to_string();
                if best.as_ref().is_none_or(|(n, _, _)| rl.len() > *n) {
                    best = Some((rl.len(), rel, r));
                }
            }
        }
    }
    let short_name = windows && cands.iter().any(|c| has_short_name_component(c));
    let Some((_, rel, root)) = best else {
        if short_name {
            return undecidable("an 8.3 short-name component could not be resolved");
        }
        return PathAnswer {
            decision: Decision::No,
            lane_pack: None,
            rel: None,
            entry: None,
            protection: None,
            reason: "not inside a lane pack".to_string(),
        };
    };
    let entry = boot_critical_entry(&rel);
    let protection = protection_entry(&rel);
    let (decision, reason) = match entry {
        Some(e) => (
            Decision::Yes,
            format!(
                "on the compiled boot-critical list ({})",
                category_wire(e.category)
            ),
        ),
        None if short_name && has_short_name_component(&rel) => {
            return undecidable("an 8.3 short-name component inside the pack could not be resolved")
        }
        None => (
            Decision::No,
            "inside a lane pack, not on the list".to_string(),
        ),
    };
    PathAnswer {
        decision,
        lane_pack: Some(root.clone()),
        rel: Some(rel),
        entry,
        protection,
        reason,
    }
}

fn category_wire(c: EntryCategory) -> &'static str {
    match c {
        EntryCategory::Hook => "hook",
        EntryCategory::HookDep => "hook-dep",
        EntryCategory::StartScript => "start-script",
        EntryCategory::StartDep => "start-dep",
        EntryCategory::Config => "config",
        EntryCategory::Protection => "protection",
    }
}

fn check_wire(c: CheckKind) -> &'static str {
    match c {
        CheckKind::ShSyntax => "sh-syntax",
        CheckKind::PyCompile => "py-compile",
        CheckKind::JsonParse => "json-parse",
        CheckKind::SelfTest => "self-test",
        CheckKind::HookReplay => "hook-replay",
        CheckKind::NonePrerehearsal => "none-prerehearsal",
    }
}

/// One row in the O-C4-3 entry format.
pub fn entry_json(e: &Entry) -> serde_json::Value {
    serde_json::json!({
        "rel": e.rel,
        "category": category_wire(e.category),
        "check": {"kind": check_wire(e.check), "arg": e.check_arg},
        "timeout_s": e.timeout_s,
        "lifeline": e.lifeline,
    })
}

/// `cys repair boot-critical --list --json`: `{"schema": 1, "entries": [<O-C4-3 rows>]}`.
pub fn list_json() -> serde_json::Value {
    serde_json::json!({
        "schema": super::REPAIR_SCHEMA,
        "entries": GENERATED.iter().map(entry_json).collect::<Vec<_>>(),
    })
}

fn answer_json(input: &Path, a: &PathAnswer) -> serde_json::Value {
    serde_json::json!({
        "path": input.to_string_lossy(),
        "decision": a.decision.wire(),
        "boot_critical": a.decision == Decision::Yes,
        "protection": a.protection.is_some(),
        "lane_pack": a.lane_pack.as_ref().map(|p| p.to_string_lossy().into_owned()),
        "rel": a.rel,
        "entry": a.entry.map(entry_json),
        "reason": a.reason,
    })
}

/// The body of `cys repair boot-critical (--path <p> | --list) [--json]` (LOCAL — no daemon
/// request, no socket, so it can never start a daemon; 4.7). Returns the exit code: `--path` →
/// 0 boot-critical / 1 not / 2 cannot decide; `--list` → 0. Write errors are ignored (a closed
/// stdout must not become a panic).
pub fn run_query(
    path: Option<&Path>,
    list: bool,
    json: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    if list {
        if json {
            let _ = writeln!(out, "{}", list_json());
        } else {
            for e in GENERATED {
                let arg = if e.check_arg.is_empty() {
                    String::new()
                } else {
                    format!(":{}", e.check_arg)
                };
                let t = e
                    .timeout_s
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "-".into());
                let _ = writeln!(
                    out,
                    "{}\t{}\t{}{arg}\t{t}\t{}",
                    e.rel,
                    category_wire(e.category),
                    check_wire(e.check),
                    if e.lifeline { "lifeline" } else { "-" }
                );
            }
        }
        return 0;
    }
    let Some(p) = path else {
        let _ = writeln!(
            err,
            "cys repair boot-critical: --path <경로> 또는 --list 가 필요합니다"
        );
        return Decision::Undecidable.exit_code();
    };
    let a = classify_path(p);
    if json {
        let _ = writeln!(out, "{}", answer_json(p, &a));
    } else {
        let line = match (a.decision, &a.rel, &a.lane_pack) {
            (Decision::Yes, Some(rel), Some(pack)) => format!(
                "기동 필수 파일: {rel} [{}] — 팩 {}",
                a.entry.map(|e| category_wire(e.category)).unwrap_or("?"),
                pack.display()
            ),
            (Decision::No, Some(rel), _) if a.protection.is_some() => {
                format!("기동 필수 아님(보호장치 관련 파일): {rel}")
            }
            (Decision::No, _, _) => format!("기동 필수 아님: {}", p.display()),
            _ => format!("판정 불가: {} — {}", p.display(), a.reason),
        };
        let _ = writeln!(out, "{line}");
    }
    a.decision.exit_code()
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
        // T5a (rewritten — T0 pinned an empty table "until T5a fills it"): the table is filled and
        // every lifeline except the statusline script (excluded by O-C4-4) is a hook row flagged
        // `lifeline` (C8 O-C8-6: one compiled list).
        assert!(
            !entries().is_empty(),
            "the compiled boot-critical table is empty"
        );
        for h in LIFELINE_HOOKS {
            if h == "cys-statusline.sh" {
                continue;
            }
            let rel = format!("hooks/{h}");
            assert!(
                entries()
                    .iter()
                    .any(|e| e.rel == rel && e.category == EntryCategory::Hook && e.lifeline),
                "lifeline {rel} is not a hook row flagged lifeline"
            );
        }
    }

    use std::collections::BTreeSet;
    use CheckKind as K;
    use EntryCategory as C;

    type Row = (
        &'static str,
        EntryCategory,
        CheckKind,
        &'static str,
        Option<u32>,
        bool,
    );

    /// The fixed input of the derivation tests (D:734 / RULINGS B6): the compiled list at this
    /// revision, row by row. A change of the derivation or of an anchor shows up here first.
    const PINNED: &[Row] = &[
        (
            "agents.json",
            C::Config,
            K::NonePrerehearsal,
            "json-parse",
            None,
            false,
        ),
        (
            "bin/_owned_process.py",
            C::HookDep,
            K::PyCompile,
            "",
            None,
            false,
        ),
        ("bin/cys-dept", C::StartScript, K::ShSyntax, "", None, false),
        (
            "bin/grill_gate.py",
            C::HookDep,
            K::PyCompile,
            "",
            None,
            false,
        ),
        (
            "bin/javis_bootstrap.py",
            C::StartScript,
            K::PyCompile,
            "",
            None,
            false,
        ),
        (
            "bin/javis_checklist.py",
            C::HookDep,
            K::PyCompile,
            "",
            None,
            false,
        ),
        (
            "bin/javis_detect.py",
            C::HookDep,
            K::PyCompile,
            "",
            None,
            false,
        ),
        (
            "bin/javis_formation.py",
            C::StartDep,
            K::PyCompile,
            "",
            None,
            false,
        ),
        (
            "bin/javis_mission.py",
            C::HookDep,
            K::PyCompile,
            "",
            None,
            false,
        ),
        (
            "bin/javis_phoenix.py",
            C::StartDep,
            K::PyCompile,
            "",
            None,
            false,
        ),
        (
            "bin/javis_preflight.py",
            C::StartScript,
            K::PyCompile,
            "",
            None,
            false,
        ),
        (
            "bin/javis_snapshot.py",
            C::HookDep,
            K::PyCompile,
            "",
            None,
            false,
        ),
        ("hooks/_lib.sh", C::HookDep, K::ShSyntax, "", None, false),
        (
            "hooks/appbuild-gate.sh",
            C::Hook,
            K::HookReplay,
            "PreToolUse",
            None,
            false,
        ),
        (
            "hooks/commit-memory-nudge.sh",
            C::Hook,
            K::HookReplay,
            "PostToolUse",
            None,
            false,
        ),
        (
            "hooks/cys-hook.sh",
            C::Hook,
            K::HookReplay,
            "PermissionRequest,PostToolUse,PreToolUse",
            None,
            false,
        ),
        (
            "hooks/grill-arm.sh",
            C::Hook,
            K::HookReplay,
            "PreToolUse",
            None,
            false,
        ),
        (
            "hooks/grill-count.sh",
            C::Hook,
            K::HookReplay,
            "PostToolUse",
            None,
            false,
        ),
        (
            "hooks/grill-gate.sh",
            C::Hook,
            K::HookReplay,
            "PreToolUse",
            None,
            false,
        ),
        (
            "hooks/grill-stop.sh",
            C::Hook,
            K::HookReplay,
            "Stop",
            None,
            false,
        ),
        (
            "hooks/guard.sh",
            C::Protection,
            K::ShSyntax,
            "",
            None,
            false,
        ),
        (
            "hooks/inject-context.sh",
            C::Hook,
            K::HookReplay,
            "SessionStart",
            None,
            true,
        ),
        (
            "hooks/pack-guard.sh",
            C::Hook,
            K::HookReplay,
            "PostToolUse",
            None,
            false,
        ),
        (
            "hooks/reflect-scan.sh",
            C::Hook,
            K::HookReplay,
            "SessionEnd,Stop",
            None,
            false,
        ),
        (
            "hooks/role-bootstrap-legacy.sh",
            C::HookDep,
            K::ShSyntax,
            "",
            None,
            false,
        ),
        (
            "hooks/role-bootstrap.sh",
            C::Hook,
            K::HookReplay,
            "UserPromptSubmit",
            Some(600),
            true,
        ),
        (
            "hooks/role-capability-gate.sh",
            C::Hook,
            K::HookReplay,
            "PreToolUse",
            Some(15),
            true,
        ),
        (
            "hooks/role-capability-gate.sh",
            C::Protection,
            K::SelfTest,
            "--self-test",
            None,
            false,
        ),
        (
            "hooks/save-state.sh",
            C::Hook,
            K::HookReplay,
            "PreCompact,Stop",
            None,
            true,
        ),
        (
            "hooks/session-start.sh",
            C::Hook,
            K::HookReplay,
            "SessionStart",
            None,
            true,
        ),
        (
            "trusted-keys.json",
            C::Protection,
            K::JsonParse,
            "",
            None,
            false,
        ),
    ];

    fn row(e: &Entry) -> Row {
        (
            e.rel,
            e.category,
            e.check,
            e.check_arg,
            e.timeout_s,
            e.lifeline,
        )
    }

    #[test]
    fn generated_list_is_pinned() {
        let got: Vec<Row> = entries().iter().map(row).collect();
        assert_eq!(
            got,
            PINNED.to_vec(),
            "the compiled boot-critical list moved"
        );
        assert_eq!(GENERATED_WHY.len(), GENERATED.len());
        for e in entries() {
            assert!(
                provenance(e).is_some_and(|w| !w.is_empty()),
                "{} has no provenance",
                e.rel
            );
        }
    }

    fn manifest() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    }
    fn pack_rels() -> Vec<String> {
        crate::pack::PACK_ALL
            .iter()
            .map(|(r, _)| r.to_string())
            .collect()
    }
    fn pack_text(rel: &str) -> Option<String> {
        crate::pack::PACK_ALL
            .iter()
            .find(|(r, _)| *r == rel)
            .map(|(_, c)| c.to_string())
    }
    fn repo_text(p: &str) -> Option<String> {
        std::fs::read_to_string(manifest().join(p)).ok()
    }

    /// Runs the derivation on the real sources, each text passed through `edit(path, text)`;
    /// `extra` adds pack files (rel, text).
    fn derive_edited(
        edit: &dyn Fn(&str, String) -> String,
        extra: &[(&str, &str)],
    ) -> Result<Vec<derive::GenEntry>, String> {
        let mut rels = pack_rels();
        rels.extend(extra.iter().map(|(r, _)| r.to_string()));
        let pack_file = |rel: &str| -> Option<String> {
            extra
                .iter()
                .find(|(r, _)| *r == rel)
                .map(|(_, t)| t.to_string())
                .or_else(|| pack_text(rel))
                .map(|t| edit(&format!("cysjavis-pack/{rel}"), t))
        };
        let repo_file = |p: &str| -> Option<String> {
            let t = match p.strip_prefix("cysjavis-pack/") {
                Some(rel) => pack_text(rel).or_else(|| repo_text(p)),
                None => repo_text(p),
            };
            t.map(|t| edit(p, t))
        };
        derive::derive(&derive::Sources {
            pack_rels: &rels,
            pack_file: &pack_file,
            repo_file: &repo_file,
        })
    }

    fn gen_rows(
        v: &[derive::GenEntry],
    ) -> Vec<(
        String,
        &'static str,
        &'static str,
        String,
        Option<u32>,
        bool,
    )> {
        v.iter()
            .map(|e| {
                (
                    e.rel.clone(),
                    e.category,
                    e.check,
                    e.check_arg.clone(),
                    e.timeout_s,
                    e.lifeline,
                )
            })
            .collect()
    }

    /// The compiled table equals a derivation of the CURRENT sources — this also re-checks the
    /// bin / app proof files that build.rs does not watch (`src/bin/cys.rs`,
    /// `src/bin/cysd/schedule.rs`), so a removed proof literal fails here even on an incremental
    /// build.
    #[test]
    fn compiled_table_equals_a_fresh_derivation() {
        let fresh = derive_edited(&|_, t| t, &[]).expect("derivation of the current sources");
        assert_eq!(fresh.len(), GENERATED.len());
        for (f, g) in fresh.iter().zip(GENERATED.iter()) {
            assert_eq!(f.rel, g.rel);
            assert_eq!(f.category, category_wire(g.category), "{}", g.rel);
            assert_eq!(f.check, check_wire(g.check), "{}", g.rel);
            assert_eq!(f.check_arg, g.check_arg, "{}", g.rel);
            assert_eq!(f.timeout_s, g.timeout_s, "{}", g.rel);
            assert_eq!(f.lifeline, g.lifeline, "{}", g.rel);
        }
        let rendered = derive::render(&fresh);
        assert!(rendered.contains("static GENERATED: &[Entry]"));
    }

    /// W1 + Y3: the list holds FILES only — every rel is a shipped pack file, none is a
    /// `settings:<event>:<hook>` registration.
    #[test]
    fn list_holds_files_only() {
        let shipped: BTreeSet<&str> = crate::pack::PACK_ALL.iter().map(|(r, _)| *r).collect();
        for e in entries() {
            assert!(
                shipped.contains(e.rel),
                "{} is not a file of the embedded pack",
                e.rel
            );
            assert!(
                !e.rel.contains(':') && !e.rel.starts_with("settings"),
                "{} is not a path",
                e.rel
            );
            assert!(
                !e.rel.contains('\\') && !e.rel.starts_with('/'),
                "{} is not pack-relative",
                e.rel
            );
        }
        let v = list_json();
        for row in v["entries"].as_array().unwrap() {
            let rel = row["rel"].as_str().unwrap();
            assert!(
                shipped.contains(rel),
                "--list --json row {rel} is not a path"
            );
        }
    }

    /// C8 O-C8-6: the lifeline flag mirrors the six lifeline hooks, only on hook rows.
    #[test]
    fn lifeline_flags_mirror_c8() {
        let flagged: BTreeSet<&str> = entries()
            .iter()
            .filter(|e| e.lifeline)
            .map(|e| {
                assert_eq!(
                    e.category,
                    C::Hook,
                    "{} is flagged lifeline but is no hook row",
                    e.rel
                );
                e.rel.trim_start_matches("hooks/")
            })
            .collect();
        let want: BTreeSet<&str> = LIFELINE_HOOKS
            .iter()
            .copied()
            .filter(|h| *h != "cys-statusline.sh")
            .collect();
        assert_eq!(flagged, want);
        for e in entries().iter().filter(|e| e.category == C::Hook) {
            let base = e.rel.trim_start_matches("hooks/");
            assert_eq!(e.lifeline, LIFELINE_HOOKS.contains(&base), "{}", e.rel);
        }
    }

    /// C4 4.3 / O-C4-4 (F2): files of the context-clear procedure are never on the list — the
    /// session state (shipped as `round/SESSION_STATE.md`!), todo files, the statusline script and
    /// the agy statusline hooks — and a write to them is answered "not boot-critical".
    #[test]
    fn clear_procedure_files_are_excluded() {
        assert!(
            crate::pack::PACK_ALL
                .iter()
                .any(|(r, _)| *r == "round/SESSION_STATE.md"),
            "the control is gone: the pack no longer ships round/SESSION_STATE.md"
        );
        for e in entries() {
            let base = e.rel.rsplit('/').next().unwrap();
            assert!(!base.starts_with("SESSION_STATE"), "{}", e.rel);
            assert!(
                !base.ends_with("_TODO.md") && base != "TODO.md",
                "{}",
                e.rel
            );
            assert!(!base.contains("statusline"), "{}", e.rel);
            assert!(!derive::is_clear_procedure_rel(e.rel), "{}", e.rel);
        }
        let root = PathBuf::from("/h/.cys/pack");
        for p in [
            "/h/.cys/pack/round/SESSION_STATE.md",
            "/h/.cys/pack/round/WORKER_TODO.md",
            "/h/.cys/pack/round/MASTER_TODO.md",
            "/h/.cys/pack/hooks/cys-statusline.sh",
            "/h/.cys/pack/hooks/cys-agy-statusline.sh",
            "/h/.cys/pack/hooks/cys-agy-statusline.cmd",
        ] {
            let a = classify_with(Path::new(p), None, &[root.clone()], false);
            assert_eq!(a.decision, Decision::No, "{p}: {a:?}");
        }
        // A derived binding to a clear-procedure file is dropped (synthetic: a hook that binds one).
        let got = derive_edited(
            &|p, t| {
                if p == "cysjavis-pack/hooks/save-state.sh" {
                    format!("{t}\nSTATE=\"$PACK/hooks/SESSION_STATE.md\"\n")
                } else {
                    t
                }
            },
            &[("hooks/SESSION_STATE.md", "x\n")],
        )
        .expect("derivation");
        assert!(got.iter().all(|e| e.rel != "hooks/SESSION_STATE.md"));
    }

    /// RULINGS B3: `timeout_s` comes from the product's timeout table (Rust `AWAKENING_HOOKS`
    /// and preflight `HOOK_TIMEOUT_S`), only on hook-replay rows; the consumer clamps it.
    #[test]
    fn timeouts_come_from_the_product_table() {
        let t = |rel: &str| boot_critical_entry(rel).unwrap().timeout_s;
        assert_eq!(
            t("hooks/role-bootstrap.sh"),
            Some(crate::pack::HOOK_TIMEOUT_ROLE_BOOTSTRAP_S as u32)
        );
        assert_eq!(t("hooks/role-capability-gate.sh"), Some(15));
        for e in entries() {
            if e.timeout_s.is_some() {
                assert_eq!(
                    e.check,
                    K::HookReplay,
                    "{} carries a timeout but is no hook replay",
                    e.rel
                );
            }
        }
        assert_eq!(clamp_check_timeout(t("hooks/role-bootstrap.sh")), 60);
        assert_eq!(clamp_check_timeout(t("hooks/role-capability-gate.sh")), 15);
        assert_eq!(clamp_check_timeout(t("hooks/session-start.sh")), 60);
    }

    /// D:941 / C4 4.10: the four kept protections are a sibling list (separate rows); a
    /// protection-only file is not boot-critical (exit 1) but `is_protection_related`.
    #[test]
    fn protection_rows_are_a_sibling_list() {
        let prot: Vec<&str> = entries()
            .iter()
            .filter(|e| e.category == C::Protection)
            .map(|e| e.rel)
            .collect();
        assert_eq!(
            prot,
            vec![
                "hooks/guard.sh",
                "hooks/role-capability-gate.sh",
                "trusted-keys.json"
            ]
        );
        assert!(boot_critical_entry("trusted-keys.json").is_none());
        assert!(protection_entry("trusted-keys.json").is_some());
        assert!(boot_critical_entry("hooks/role-capability-gate.sh").is_some());
        assert!(protection_entry("hooks/role-capability-gate.sh").is_some());
        let root = PathBuf::from("/h/.cys/pack");
        let a = classify_with(
            Path::new("/h/.cys/pack/trusted-keys.json"),
            None,
            &[root.clone()],
            false,
        );
        assert_eq!(a.decision, Decision::No);
        assert!(a.protection.is_some());
        let a = classify_with(
            Path::new("/h/.cys/pack/hooks/guard.sh"),
            None,
            &[root],
            false,
        );
        assert_eq!((a.decision, a.protection.is_some()), (Decision::No, true));
    }

    /// 4.6 / D:1125(3): Windows forms — separator, case, drive letter, `/c/…`, `\\?\`, `..`,
    /// short names (unresolvable → cannot decide = fail-open for hooks).
    #[test]
    fn windows_normalisation_table() {
        let n = |s: &str| normalize_lexical(s, true);
        assert_eq!(
            n(r"C:\Users\x\.cys\pack\hooks\a.sh"),
            "C:/Users/x/.cys/pack/hooks/a.sh"
        );
        assert_eq!(n("c:/Users/x"), "C:/Users/x");
        assert_eq!(n("/c/Users/x/.cys"), "C:/Users/x/.cys");
        assert_eq!(n("/cygdrive/d/w/x"), "D:/w/x");
        assert_eq!(n(r"\\?\C:\Users\x"), "C:/Users/x");
        assert_eq!(n(r"\\?\UNC\srv\share\p"), "//srv/share/p");
        assert_eq!(n(r"C:\a\b\..\.\c\\d"), "C:/a/c/d");
        assert_eq!(n("C:/a/../../b"), "C:/b");
        assert_eq!(normalize_lexical("/a/./b/../c//d", false), "/a/c/d");
        assert_eq!(
            normalize_lexical(r"/a\b", false),
            r"/a\b",
            "a backslash is a file-name byte on unix"
        );

        let roots = vec![
            PathBuf::from(r"C:\Users\x\.cys\pack"),
            PathBuf::from(r"C:\Users\x\.cys\pack-dept-d1"),
        ];
        let cls = |p: &str| {
            classify_with(
                Path::new(p),
                Some(Path::new(r"C:\Users\x\.cys\pack\bin")),
                &roots,
                true,
            )
        };
        for p in [
            r"C:\Users\x\.cys\pack\hooks\inject-context.sh",
            "c:/users/X/.CYS/pack/Hooks/Inject-Context.sh",
            "/c/Users/x/.cys/pack/hooks/inject-context.sh",
            r"\\?\C:\Users\x\.cys\pack\hooks\inject-context.sh",
            r"C:\Users\x\.cys\pack\hooks\..\hooks\inject-context.sh",
            r"..\hooks\inject-context.sh",
            r"C:\Users\x\.cys\pack-dept-d1\hooks\inject-context.sh",
            r"C:\Users\x\.cys\pack\bin\javis_bootstrap.py",
            r"C:\Users\x\.cys\pack\agents.json",
        ] {
            let a = cls(p);
            assert_eq!(a.decision, Decision::Yes, "{p}: {a:?}");
        }
        assert_eq!(
            cls(r"C:\Users\x\.cys\pack-dept-d1\hooks\inject-context.sh").lane_pack,
            Some(roots[1].clone())
        );
        for p in [
            r"C:\Users\x\.cys\pack\hooks\README.md",
            r"C:\Users\x\.cys\pack\round\SESSION_STATE.md",
            r"C:\Users\x\.cys\local\repairs\R-1\staging\hooks\inject-context.sh",
            r"C:\Users\x\.cys\pack-dept-d10x\hooks\inject-context.sh",
            r"D:\elsewhere\hooks\inject-context.sh",
        ] {
            assert_eq!(cls(p).decision, Decision::No, "{p}");
        }
        for p in [
            r"C:\Users\user~1\.cys\pack\hooks\inject-context.sh",
            r"C:\Users\x\.cys\pack\hooks\INJECT~1.SH",
            r"C:\PROGRA~1\x",
        ] {
            assert_eq!(cls(p).decision, Decision::Undecidable, "{p}");
        }
    }

    /// Unix path variants of TESTPLAN D-1-01 (v1–v5) on a real tree: `./`, `..`, a case variant,
    /// a symlinked parent (also for a file that does not exist yet), each department pack; the
    /// control (a file not on the list) is never "yes"; the repair staging copy is never gated.
    #[cfg(unix)]
    #[test]
    fn unix_path_variants_resolve() {
        use std::os::unix::ffi::OsStrExt;
        let t = std::env::temp_dir().join(format!("t5a-paths-{}-{}", std::process::id(), line!()));
        let _ = std::fs::remove_dir_all(&t);
        let pack = t.join("home/.cys/pack");
        let dept = t.join("home/.cys/pack-dept-d1");
        for p in [&pack, &dept] {
            for rel in [
                "hooks/inject-context.sh",
                "hooks/README.md",
                "bin/javis_bootstrap.py",
                "agents.json",
            ] {
                let f = p.join(rel);
                std::fs::create_dir_all(f.parent().unwrap()).unwrap();
                std::fs::write(&f, "x\n").unwrap();
            }
        }
        let outside = t.join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(pack.join("hooks"), outside.join("hooks-link")).unwrap();
        let staging = t.join("home/.cys/local/repairs/R-1/staging/hooks");
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(staging.join("inject-context.sh"), "x\n").unwrap();
        let roots = vec![pack.clone(), dept.clone()];
        let cls = |p: &Path, cwd: &Path| classify_with(p, Some(cwd), &roots, false);
        let yes = [
            (PathBuf::from("./hooks/inject-context.sh"), pack.clone()),
            (
                PathBuf::from("hooks/../hooks/inject-context.sh"),
                pack.clone(),
            ),
            (
                PathBuf::from("../hooks/inject-context.sh"),
                pack.join("bin"),
            ),
            (PathBuf::from("Hooks/Inject-Context.sh"), pack.clone()),
            (outside.join("hooks-link/inject-context.sh"), t.clone()),
            (pack.join("hooks/inject-context.sh"), t.clone()),
            (dept.join("hooks/inject-context.sh"), t.clone()),
            (dept.join("bin/javis_bootstrap.py"), t.clone()),
            (pack.join("agents.json"), t.clone()),
        ];
        for (p, cwd) in &yes {
            let a = cls(p, cwd);
            assert_eq!(a.decision, Decision::Yes, "{}: {a:?}", p.display());
            assert_eq!(
                a.rel.as_deref().map(str::to_ascii_lowercase).as_deref(),
                Some(if p.to_string_lossy().contains("bootstrap") {
                    "bin/javis_bootstrap.py"
                } else if p.to_string_lossy().contains("agents") {
                    "agents.json"
                } else {
                    "hooks/inject-context.sh"
                })
            );
        }
        let no = [
            (PathBuf::from("./hooks/README.md"), pack.clone()),
            (PathBuf::from("Hooks/ReadMe.md"), pack.clone()),
            (outside.join("hooks-link/README.md"), t.clone()),
            (outside.join("hooks-link/new-file.sh"), t.clone()),
            (staging.join("inject-context.sh"), t.clone()),
            (outside.join("inject-context.sh"), t.clone()),
        ];
        for (p, cwd) in &no {
            assert_eq!(cls(p, cwd).decision, Decision::No, "{}", p.display());
        }
        // a symlinked parent + a leaf that does not exist yet → still resolved into the pack
        let a = cls(&outside.join("hooks-link/new-file.sh"), &t);
        assert_eq!(a.lane_pack.as_ref(), Some(&pack));
        // cannot decide
        let bad = Path::new(std::ffi::OsStr::from_bytes(b"/tmp/\xff/x"));
        assert_eq!(cls(bad, &t).decision, Decision::Undecidable);
        assert_eq!(
            classify_with(Path::new("hooks/x"), None, &roots, false).decision,
            Decision::Undecidable
        );
        assert_eq!(
            classify_with(Path::new("/x"), Some(&t), &[], false).decision,
            Decision::Undecidable
        );
        assert_eq!(
            classify_with(Path::new(""), Some(&t), &roots, false).decision,
            Decision::Undecidable
        );
        let _ = std::fs::remove_dir_all(&t);
    }

    /// The query: exit codes 0 / 1 / 2, `--list --json` = the compiled table, plain lines.
    #[test]
    fn query_answers_and_exit_codes() {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        assert_eq!(run_query(None, true, true, &mut out, &mut err), 0);
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v, list_json());
        assert_eq!(v["schema"], 1);
        let rows = v["entries"].as_array().unwrap();
        assert_eq!(rows.len(), PINNED.len());
        assert_eq!(rows[0]["rel"], "agents.json");
        assert_eq!(rows[0]["check"]["kind"], "none-prerehearsal");
        assert_eq!(rows[0]["check"]["arg"], "json-parse");
        assert!(rows.iter().any(|r| r["rel"] == "hooks/role-bootstrap.sh"
            && r["timeout_s"] == 600
            && r["lifeline"] == true));
        let (mut out, mut err) = (Vec::new(), Vec::new());
        assert_eq!(run_query(None, true, false, &mut out, &mut err), 0);
        assert_eq!(
            String::from_utf8(out).unwrap().lines().count(),
            PINNED.len()
        );

        let _g = crate::pack::PACK_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let t = std::env::temp_dir().join(format!("t5a-query-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        std::fs::create_dir_all(t.join("hooks")).unwrap();
        let saved = std::env::var(crate::pack::ENV_PACK_DIR).ok();
        std::env::set_var(crate::pack::ENV_PACK_DIR, &t);
        let q = |p: &Path, json: bool| {
            let (mut out, mut err) = (Vec::new(), Vec::new());
            let code = run_query(Some(p), false, json, &mut out, &mut err);
            (code, String::from_utf8(out).unwrap())
        };
        let (code, o) = q(&t.join("hooks/session-start.sh"), true);
        assert_eq!(code, 0, "{o}");
        let v: serde_json::Value = serde_json::from_str(o.trim()).unwrap();
        assert_eq!(v["decision"], "yes");
        assert_eq!(v["boot_critical"], true);
        assert_eq!(v["rel"], "hooks/session-start.sh");
        assert_eq!(v["entry"]["category"], "hook");
        let (code, o) = q(&t.join("hooks/README.md"), true);
        assert_eq!(code, 1);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(o.trim()).unwrap()["decision"],
            "no"
        );
        let (code, o) = q(&t.join("trusted-keys.json"), true);
        assert_eq!(code, 1);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(o.trim()).unwrap()["protection"],
            true
        );
        let (code, o) = q(&t.join("hooks/session-start.sh"), false);
        assert_eq!(code, 0);
        assert!(
            o.starts_with("기동 필수 파일: hooks/session-start.sh"),
            "{o}"
        );
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            let (code, o) = q(Path::new(std::ffi::OsStr::from_bytes(b"/tmp/\xff")), true);
            assert_eq!(code, 2, "{o}");
        }
        match saved {
            Some(v) => std::env::set_var(crate::pack::ENV_PACK_DIR, v),
            None => std::env::remove_var(crate::pack::ENV_PACK_DIR),
        }
        let _ = std::fs::remove_dir_all(&t);
    }

    /// Structural pin of 4.7 / appendix A 1-17: the production code of this module has no
    /// daemon, socket or process path (the end-to-end check — a listener on `CYS_SOCKET` that
    /// receives nothing while the BUILT binary answers — is
    /// `test_capgate_boot_critical.py::GateWithBuiltBinary::test_query_never_contacts_a_daemon`;
    /// it runs in its own process, so no shared env is touched here).
    #[test]
    fn query_source_has_no_daemon_path() {
        let src = include_str!("boot_critical.rs");
        let prod = &src[..src
            .find("#[cfg(test)]\nmod tests {")
            .expect("test module boundary")];
        for needle in [
            "UnixStream",
            "TcpStream",
            "connect(",
            "request(",
            "Command::new",
            "spawn(",
            "launchctl",
            "socket_path",
        ] {
            assert!(
                !prod.contains(needle),
                "boot_critical.rs production code contains {needle}"
            );
        }
    }

    // ── isolated-HOME start replay (D:734 / C4 4.4 — what this level can measure) ───────

    /// One hook run of the replay.
    #[cfg(target_os = "macos")]
    #[derive(Debug, Clone)]
    struct Run {
        code: Option<i32>,
        stdout: String,
        stderr: String,
    }

    #[cfg(target_os = "macos")]
    impl Run {
        /// The start step failed: non-zero / killed, or the hook's own loud degradation lines.
        fn failed(&self) -> bool {
            self.code != Some(0)
                || self.stderr.contains("syntax error")
                || self.stderr.contains("SyntaxError")
                || self.stderr.contains("_lib.sh 소실")
        }
    }

    /// The derivation test of D:734 at the level a library test can reach without agent CLIs or
    /// a daemon: every registered hook (the `hook` rows) is run the way the product registers it
    /// (`sh <pack>/hooks/<script>`, the recorded event JSON on stdin) from a HERMETIC copy of the
    /// embedded pack, with a private HOME / TMPDIR, the four isolation knobs and a socket path
    /// that does not exist. Measured:
    ///   (1) healthy: every hook replay succeeds (exit 0, no degradation line — A-1 26);
    ///   (2) break each listed hook file → its replay fails;
    ///   (3) break each listed non-hook file that a replay actually reads (tripwire) → at
    ///       least one replay that read it changes (fails, or its deterministic output changes);
    ///   (4) break EVERY unlisted pack file at once → no replay fails ("still starts");
    ///   (5) W1: no lifeline hook's replay reads a non-lifeline hook's file, so removing a
    ///       non-lifeline registration (that hook simply never runs) leaves them unchanged.
    /// Listed files that no replay reads (start scripts that need a daemon / agent CLI to run,
    /// `agents.json`) are recorded as "not exercised here" in the matrix — their rows rest on
    /// their proof anchors; the full-product run is the stage-5 integration E2E.
    #[cfg(target_os = "macos")]
    #[test]
    fn isolated_home_start_replay() {
        use std::collections::BTreeMap;
        use std::os::unix::fs::PermissionsExt;
        use std::os::unix::process::CommandExt;
        use std::process::{Command, Stdio};

        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let root = std::env::temp_dir().join(format!("t5a-replay-{}-{nanos}", std::process::id()));
        let pack = root.join("pack");
        for (rel, content) in crate::pack::PACK_ALL {
            let f = pack.join(rel);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(&f, content).unwrap();
            if content.starts_with("#!") {
                std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
        }
        let original = |rel: &str| -> String { pack_text(rel).unwrap() };
        let put = |rel: &str, text: &str| std::fs::write(pack.join(rel), text).unwrap();
        let broken = |rel: &str| -> &'static str {
            if rel.ends_with(".py")
                || original(rel)
                    .lines()
                    .next()
                    .is_some_and(|l| l.contains("python"))
            {
                "def (:\n"
            } else if rel.ends_with(".json") {
                "{\n"
            } else {
                "#!/bin/sh\n(\n"
            }
        };
        let trip_log = root.join("trip.log");
        let tripwired = |rel: &str| -> Option<String> {
            let t = original(rel);
            let (first, rest) = match t.split_once('\n') {
                Some((f, r)) if f.starts_with("#!") => (format!("{f}\n"), r.to_string()),
                _ => (String::new(), t.clone()),
            };
            let line = if broken(rel) == "def (:\n" {
                format!("import os as _t5a_os; _t5a_p = _t5a_os.environ.get(\"T5A_TRIP_LOG\"); _t5a_p and open(_t5a_p, \"a\").write(\"{rel}\\n\")\n")
            } else if broken(rel) == "#!/bin/sh\n(\n" {
                format!(
                    "printf '%s\\n' '{rel}' >> \"${{T5A_TRIP_LOG:-/dev/null}}\" 2>/dev/null || :\n"
                )
            } else {
                return None;
            };
            Some(format!("{first}{line}{rest}"))
        };
        let event_json = |ev: &str| -> String {
            let cwd = root.to_string_lossy();
            match ev {
                "SessionStart" => format!(
                    r#"{{"hook_event_name":"SessionStart","session_id":"t5a","source":"startup","cwd":"{cwd}"}}"#
                ),
                "UserPromptSubmit" => format!(
                    r#"{{"hook_event_name":"UserPromptSubmit","session_id":"t5a","prompt":"hello","cwd":"{cwd}"}}"#
                ),
                "PreToolUse" => format!(
                    r#"{{"hook_event_name":"PreToolUse","session_id":"t5a","tool_name":"Read","tool_input":{{"file_path":"{cwd}/x"}},"cwd":"{cwd}"}}"#
                ),
                "PostToolUse" => format!(
                    r#"{{"hook_event_name":"PostToolUse","session_id":"t5a","tool_name":"Read","tool_input":{{"file_path":"{cwd}/x"}},"tool_response":{{}},"cwd":"{cwd}"}}"#
                ),
                "PermissionRequest" => format!(
                    r#"{{"hook_event_name":"PermissionRequest","session_id":"t5a","tool_name":"Bash","tool_input":{{"command":"ls"}},"cwd":"{cwd}"}}"#
                ),
                other => {
                    format!(r#"{{"hook_event_name":"{other}","session_id":"t5a","cwd":"{cwd}"}}"#)
                }
            }
        };
        let mut counter = 0u32;
        let mut run = |e: &Entry| -> Run {
            counter += 1;
            let home = root.join(format!("home-{counter}"));
            let tmp = root.join(format!("tmp-{counter}"));
            std::fs::create_dir_all(&home).unwrap();
            std::fs::create_dir_all(&tmp).unwrap();
            let (out_p, err_p) = (
                root.join(format!("o-{counter}")),
                root.join(format!("e-{counter}")),
            );
            let event = e.check_arg.split(',').next().unwrap_or("SessionStart");
            let mut child = Command::new("sh")
                .arg(pack.join(e.rel))
                .env_clear()
                .env(
                    "PATH",
                    std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".into()),
                )
                .env("HOME", &home)
                .env("TMPDIR", &tmp)
                .env("CYS_PACK_DIR", &pack)
                .env("CYS_SOCKET", root.join("no-daemon.sock"))
                .env("CYS_NO_AUTOSTART", "1")
                .env("CYS_NO_OFFICE_BRIDGE", "1")
                .env("CYS_NO_PERSONAL_HOOK_MERGE", "1")
                .env("CYS_NO_AUTORESTORE", "1")
                .env("T5A_TRIP_LOG", &trip_log)
                .current_dir(&root)
                .stdin(Stdio::piped())
                .stdout(Stdio::from(std::fs::File::create(&out_p).unwrap()))
                .stderr(Stdio::from(std::fs::File::create(&err_p).unwrap()))
                .process_group(0)
                .spawn()
                .expect("spawn sh");
            {
                use std::io::Write as _;
                let mut stdin = child.stdin.take().unwrap();
                let _ = stdin.write_all(event_json(event).as_bytes());
            }
            let deadline = std::time::Instant::now()
                + std::time::Duration::from_secs(u64::from(clamp_check_timeout(e.timeout_s)));
            // Wait WITHOUT reaping (WNOWAIT): while the leader is an unreaped zombie its pid — and
            // so its process-group id — cannot be reused, so the group kill below can only reach
            // what this hook left behind (no process outlives the test, no pattern kill).
            let pid = child.id() as libc::pid_t;
            let exited = || -> bool {
                let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
                let r = unsafe {
                    libc::waitid(
                        libc::P_PID,
                        pid as libc::id_t,
                        &mut info,
                        libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
                    )
                };
                r == 0 && info.si_pid != 0
            };
            let mut timed_out = false;
            while !exited() {
                if std::time::Instant::now() >= deadline {
                    timed_out = true;
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
            }
            let status = child.wait().ok();
            let code = if timed_out {
                None
            } else {
                status.and_then(|s| s.code())
            };
            Run {
                code,
                stdout: std::fs::read_to_string(&out_p).unwrap_or_default(),
                stderr: std::fs::read_to_string(&err_p).unwrap_or_default(),
            }
        };

        let hooks: Vec<&Entry> = entries().iter().filter(|e| e.category == C::Hook).collect();
        let hook_rels: BTreeSet<&str> = hooks.iter().map(|e| e.rel).collect();
        let listed: BTreeSet<&str> = entries().iter().map(|e| e.rel).collect();
        let deps: Vec<&str> = entries()
            .iter()
            .filter(|e| e.category != C::Protection && !hook_rels.contains(e.rel))
            .map(|e| e.rel)
            .collect();
        let mut problems: Vec<String> = Vec::new();
        let mut matrix: Vec<String> = Vec::new();

        // (1) healthy, twice (the second run tells whether stdout is deterministic)
        let mut healthy: BTreeMap<&str, (Run, bool)> = BTreeMap::new();
        for h in &hooks {
            let a = run(h);
            let b = run(h);
            if a.failed() || b.failed() {
                problems.push(format!("(1) healthy replay of {} failed: {a:?}", h.rel));
            }
            let det = a.stdout == b.stdout;
            matrix.push(format!(
                "healthy {} exit={:?} deterministic_stdout={det}",
                h.rel, a.code
            ));
            healthy.insert(h.rel, (a, det));
        }

        // tripwires: which listed files each replay reads
        for rel in listed.iter().copied() {
            if let Some(t) = tripwired(rel) {
                put(rel, &t);
            }
        }
        let mut reads: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
        for h in &hooks {
            let _ = std::fs::remove_file(&trip_log);
            let r = run(h);
            if r.failed() {
                problems.push(format!("tripwire replay of {} failed: {r:?}", h.rel));
            }
            let used: BTreeSet<String> = std::fs::read_to_string(&trip_log)
                .unwrap_or_default()
                .lines()
                .filter(|l| *l != h.rel)
                .map(str::to_string)
                .collect();
            matrix.push(format!("reads {} -> {used:?}", h.rel));
            reads.insert(h.rel, used);
        }
        for rel in listed.iter().copied() {
            put(rel, &original(rel));
        }

        // (2) break each listed hook file → its replay fails
        for h in &hooks {
            put(h.rel, broken(h.rel));
            let r = run(h);
            if !r.failed() {
                problems.push(format!(
                    "(2) {} broken but its replay still succeeded: {r:?}",
                    h.rel
                ));
            }
            put(h.rel, &original(h.rel));
        }

        // (3) break each listed non-hook file that a replay reads → at least one start step
        //     (a replay that read it) changes; readers whose recorded input does not reach the
        //     broken path are recorded, not failed (e.g. a counter hook on a tool it ignores)
        for d in &deps {
            let readers: Vec<&&Entry> =
                hooks.iter().filter(|h| reads[h.rel].contains(*d)).collect();
            if readers.is_empty() {
                matrix.push(format!(
                    "not exercised here: {d} (proof anchor / stage-5 E2E)"
                ));
                continue;
            }
            put(d, broken(d));
            let mut changed_any = false;
            for h in readers {
                let r = run(h);
                let (base, det) = &healthy[h.rel];
                let changed = r.failed() || (*det && r.stdout != base.stdout);
                changed_any |= changed;
                matrix.push(format!("break {d} -> {} changed={changed}", h.rel));
            }
            if !changed_any {
                problems.push(format!(
                    "(3) {d} broken but no start step that reads it changed"
                ));
            }
            put(d, &original(d));
        }

        // (4) break every unlisted file at once → no replay fails
        let unlisted: Vec<&str> = crate::pack::PACK_ALL
            .iter()
            .map(|(r, _)| *r)
            .filter(|r| !listed.contains(r))
            .collect();
        for rel in &unlisted {
            put(rel, broken(rel));
        }
        for h in &hooks {
            let r = run(h);
            if r.failed() {
                problems.push(format!(
                    "(4) unlisted files broken and the replay of {} failed: {r:?}",
                    h.rel
                ));
            }
        }
        matrix.push(format!(
            "(4) {} unlisted files broken together",
            unlisted.len()
        ));

        // (5) W1: lifeline replays never read a non-lifeline hook file
        for h in hooks.iter().filter(|h| h.lifeline) {
            for n in hooks.iter().filter(|n| !n.lifeline) {
                if reads[h.rel].contains(n.rel) {
                    problems.push(format!(
                        "(5) lifeline {} reads non-lifeline hook {}",
                        h.rel, n.rel
                    ));
                }
            }
        }

        let report = format!(
            "{}\nPROBLEMS:\n{}\n",
            matrix.join("\n"),
            problems.join("\n")
        );
        if let Ok(td) = std::env::var("CARGO_TARGET_DIR") {
            let _ = std::fs::write(Path::new(&td).join("t5a-start-replay.txt"), &report);
        }
        let _ = std::fs::remove_dir_all(&root);
        assert!(problems.is_empty(), "{report}");
    }

    // ── census and anchors (RULINGS B6) ─────────────────────────────────────────────

    fn edit_one(target: &'static str, f: fn(String) -> String) -> impl Fn(&str, String) -> String {
        move |p: &str, t: String| if p == target { f(t) } else { t }
    }

    #[test]
    fn census_fails_on_a_new_registrar_call_site() {
        let e = derive_edited(
            &edit_one("cysjavis-pack/bin/javis_preflight.py", |t| {
                format!("{t}\n        err = self._register_event_hook(t, \"Stop\", \"verify-reminder.sh\")\n")
            }),
            &[],
        )
        .unwrap_err();
        assert!(
            e.contains("registrar call") && e.contains("verify-reminder.sh"),
            "{e}"
        );
    }

    #[test]
    fn census_fails_on_a_new_registrar_def() {
        let e = derive_edited(
            &edit_one("cysjavis-pack/bin/javis_preflight.py", |t| {
                format!("{t}\n    def _register_extra_hook(self, settings_path):\n        return None\n")
            }),
            &[],
        )
        .unwrap_err();
        assert!(e.contains("_register_extra_hook"), "{e}");
    }

    #[test]
    fn census_fails_on_a_new_settings_writer() {
        let e = derive_edited(
            &|_, t| t,
            &[(
                "bin/javis_new_tool.py",
                "def f(d):\n    d.setdefault(\"hooks\", {})\n",
            )],
        )
        .unwrap_err();
        assert!(e.contains("bin/javis_new_tool.py"), "{e}");
    }

    #[test]
    fn census_fails_on_a_second_rust_hook_set() {
        let e = derive_edited(
            &edit_one("src/pack.rs", |t| {
                t.replacen(
                    "pub fn acquire_settings_lock(",
                    "fn extra() { let _ = merge_desired_hooks(&a, &b, &OTHER_HOOKS); }\npub fn acquire_settings_lock(",
                    1,
                )
            }),
            &[],
        )
        .unwrap_err();
        assert!(e.contains("merge_desired_hooks"), "{e}");
    }

    #[test]
    fn missing_anchor_fails_naming_it() {
        let e = derive_edited(
            &edit_one("cysjavis-pack/bin/javis_preflight.py", |t| {
                t.replace("SELFCORR_HOOKS = [", "SELFCORR_RENAMED = [")
            }),
            &[],
        )
        .unwrap_err();
        assert!(e.contains("SELFCORR_HOOKS"), "{e}");
        let e = derive_edited(
            &edit_one("src/pack.rs", |t| {
                t.replace("pub const AWAKENING_HOOKS", "pub const AWAKE_HOOKS")
            }),
            &[],
        )
        .unwrap_err();
        assert!(e.contains("AWAKENING_HOOKS"), "{e}");
        let e = derive_edited(
            &edit_one("src/bin/cys.rs", |t| {
                t.replace(
                    "cys::pack::pack_dir().join(\"agents.json\")",
                    "agents_path()",
                )
            }),
            &[],
        )
        .unwrap_err();
        assert!(
            e.contains("agents.json") && e.contains("src/bin/cys.rs"),
            "{e}"
        );
        let e = derive_edited(
            &edit_one("src/agy_statusline.rs", |t| {
                t.replace("pub const SCRIPT_CMD", "pub const SCRIPT_WIN")
            }),
            &[],
        )
        .unwrap_err();
        assert!(e.contains("SCRIPT_CMD"), "{e}");
        let e = derive_edited(
            &edit_one("cysjavis-pack/bin/javis_preflight.py", |t| {
                t.replace(
                    "for hname, hevent, hmatcher in GRILL_HOOKS:",
                    "for hname, hevent, hmatcher in OTHER:",
                )
            }),
            &[],
        )
        .unwrap_err();
        assert!(e.contains("GRILL_HOOKS"), "{e}");
    }

    /// The list follows the anchors (it is not a hand list): a new hook in a registration
    /// constant appears as a hook row without touching the derivation.
    #[test]
    fn a_new_registration_in_an_anchor_is_listed() {
        let got = derive_edited(
            &edit_one("cysjavis-pack/bin/javis_preflight.py", |t| {
                t.replacen(
                    "SELFCORR_HOOKS = [",
                    "SELFCORR_HOOKS = [\n    (\"verify-reminder.sh\", [(\"Stop\", None)]),",
                    1,
                )
            }),
            &[],
        )
        .expect("derivation");
        let r = got
            .iter()
            .find(|e| e.rel == "hooks/verify-reminder.sh")
            .expect("new hook row");
        assert_eq!(
            (r.category, r.check, r.check_arg.as_str()),
            ("hook", "hook-replay", "Stop")
        );
    }

    /// B6: CRLF sources derive the same list.
    #[test]
    fn crlf_sources_derive_the_same_list() {
        let lf = derive_edited(&|_, t| t, &[]).unwrap();
        let crlf = derive_edited(&|_, t| t.replace('\n', "\r\n"), &[]).unwrap();
        assert_eq!(gen_rows(&lf), gen_rows(&crlf));
    }

    /// Rule 2 examples: what binds and what does not.
    #[test]
    fn binding_rule_examples() {
        let rels: BTreeSet<&str> = ["hooks/_lib.sh", "bin/a.py", "bin/b.py", "hooks/x.sh"]
            .into_iter()
            .collect();
        let sh = "#!/bin/sh\n. \"$(dirname \"$0\")/_lib.sh\" 2>/dev/null \\\n  || . \"${CYS_PACK_DIR:-$HOME/.cys/pack}/hooks/_lib.sh\"\n\
                  A=\"$PACK/bin/a.py\"\nMSG='run \"$PACK/bin/b.py\" add'\nR=\"$(\"$PY\" \"$PACK/bin/b.py\" 2>/dev/null)\"\n# B=\"$PACK/bin/b.py\"\n";
        let got: Vec<String> = derive::bindings("hooks/x.sh", sh, &rels)
            .into_iter()
            .map(|(r, _)| r)
            .collect();
        assert_eq!(
            got,
            vec!["bin/a.py".to_string(), "hooks/_lib.sh".to_string()]
        );
        let py = "import os, b\nfrom a import x\ndef f():\n    import zzz\n";
        let got: Vec<String> = derive::bindings("bin/c.py", py, &rels)
            .into_iter()
            .map(|(r, _)| r)
            .collect();
        assert_eq!(got, vec!["bin/a.py".to_string(), "bin/b.py".to_string()]);
    }

    /// B6: no new build-dependency (Rust std + the existing `serde_json`; Windows `winresource`).
    #[test]
    fn no_new_build_dependency() {
        let toml = repo_text("Cargo.toml").unwrap();
        let section = |head: &str| -> Vec<String> {
            let at = toml.find(head).unwrap_or_else(|| panic!("{head} missing")) + head.len();
            toml[at..]
                .lines()
                .skip(1)
                .take_while(|l| !l.starts_with('['))
                .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
                .map(|l| l.split('=').next().unwrap().trim().to_string())
                .collect()
        };
        assert_eq!(section("\n[build-dependencies]"), vec!["serde_json"]);
        assert_eq!(
            section("\n[target.'cfg(windows)'.build-dependencies]"),
            vec!["winresource"]
        );
    }

    /// B6: no Python (no process at all) is started by the derivation; build.rs keeps its two
    /// pre-existing `git` calls only.
    #[test]
    fn build_script_executes_no_python() {
        let b = repo_text("build.rs").unwrap();
        assert_eq!(
            b.matches("Command::new(").count(),
            2,
            "build.rs gained a process"
        );
        assert_eq!(b.matches("Command::new(\"git\")").count(), 2);
        let d = include_str!("boot_critical_derive.rs");
        assert!(!d.contains("std::process") && !d.contains(concat!("Command", "::new")));
    }
}
