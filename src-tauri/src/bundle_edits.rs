//! 「스스로 해결」 M16 — bundle-edit capture before an app update (release 0.15.0).
//!
//! Specification: design v0.5 §4-16 (lines 975–986), §6-16, §1-6 row 결정 19, §9-1 limits 10/11;
//! CONTRACTS C12 12.8 + O-C12-2 (a) + O-C12-3; RULINGS C X4. What it does:
//!
//! * The app's own update handler calls [`capture_before_install`] **before the install starts** —
//!   macOS: before the single download-and-install call of `install_update`; Windows: inside the
//!   checked launch (`install_update_checked_windows`), after the download and before the installer
//!   file is written and launched. Paths the app does not run are not covered (homepage installer,
//!   the checked launch switched off, the non-installer package branch, an old installer run again)
//!   — [`site_applies`] encodes exactly that.
//! * Detection = a Rust port of `cysjavis-pack/bin/javis_runtime_seal.py:classify` over the shipped
//!   `runtime-manifest.json` (no process is started — X4). Updater leftovers (`.prev<digits>`, the
//!   grammar of `cysd::is_update_leftover`) and Python caches (`__pycache__`) are not edits.
//! * Every edit is copied byte for byte to `~/.cys/local/bundle-edits/<old-version>-<UTC ts>/
//!   <bundle-relative path>` plus `index.json` (O-C12-3). The app never deletes anything there.
//! * It NEVER blocks the update: every failure ends in one Korean notice and the caller goes on.
//!   The work runs on a blocking thread under a cooperative deadline, an outer timeout and a byte
//!   cap; a panic or a stuck file system read becomes a notice, never a stuck update.
//! * RULINGS AA3–AA6: below `cap + 2 GiB` free on the store volume nothing is copied (list only); an
//!   identical earlier capture of the same version is not copied again (`same_as`); index entries
//!   carry `kind`, `stored`, the scanned hash and the link target; files are opened without
//!   following links; at the Windows place the notice is recorded inside the store
//!   (`bundle-edits/.notices/`) and handed to the UI at the next app start (AG1–AG3). Nothing is
//!   written outside `bundle-edits/`.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use tauri::{Emitter, Manager};

/// `javis_runtime_seal.MANIFEST_BASENAME` — the manifest file name (and the one root entry the
/// Python walk excludes).
pub(crate) const MANIFEST_BASENAME: &str = "runtime-manifest.json";
/// The bundled runtime folder next to the manifest (`Contents/Resources/runtime` on macOS, the
/// install folder's `runtime` on Windows).
pub(crate) const RUNTIME_DIR: &str = "runtime";
/// `schema` written into `index.json` (O-C12-3).
pub(crate) const INDEX_SCHEMA: u32 = 1;
/// The index file inside one capture folder (O-C12-3).
pub(crate) const INDEX_FILE: &str = "index.json";
/// Cooperative deadline of one capture (scan + copy), checked before every file (RATIFIED, RULINGS
/// AA4: 30 s + 10 s grace — the update waits at most 40 s). The real Windows duration is the WIN-04
/// measurement.
pub(crate) const CAPTURE_DEADLINE: Duration = Duration::from_secs(30);
/// Extra wait of the outer timeout on top of the deadline; past it the update goes on without the
/// worker (which stops at its next file through the abandon flag).
pub(crate) const JOIN_GRACE: Duration = Duration::from_secs(10);
/// Most bytes one capture copies (RATIFIED, RULINGS AA4): edits beyond it are listed, not copied.
pub(crate) const MAX_CAPTURE_BYTES: u64 = 256 * 1024 * 1024;
/// Free space kept on the store volume on top of the cap (RULINGS AA4): below `cap + 2 GiB` nothing is
/// copied — on macOS the capture runs before the download and must never be the reason it fails.
pub(crate) const FREE_SPACE_MARGIN: u64 = 2 * 1024 * 1024 * 1024;
/// Largest manifest read (RULINGS AA6; the shipped macOS manifest is about 2.4 MB).
pub(crate) const MAX_MANIFEST_BYTES: u64 = 32 * 1024 * 1024;
/// Largest earlier `index.json` read when looking for an identical earlier capture.
const MAX_INDEX_BYTES: u64 = 16 * 1024 * 1024;
/// Largest pending-notice record read at start.
const MAX_NOTICE_BYTES: u64 = 64 * 1024;
/// The one sub-folder of the store that holds notice records (RULINGS AG1: nothing is written outside
/// `bundle-edits/`; the name starts with `.` so it never looks like a capture folder `<version>-<ts>`).
pub(crate) const NOTICES_DIR: &str = ".notices";
/// `bundle-edits/.notices/pending-<leaf>.json` — a notice still to be handed to the UI (RULINGS AA3, AG3).
pub(crate) const PENDING_NOTICE_PREFIX: &str = "pending-";
/// The same record once handed over (renamed, never deleted).
pub(crate) const SHOWN_NOTICE_PREFIX: &str = "shown-";
/// The record write at the Windows place has its own limit, taken out of the grace (RULINGS AG2): the
/// capture waits at most `CAPTURE_DEADLINE + JOIN_GRACE - RECORD_LIMIT`, the write at most `RECORD_LIMIT`
/// — 40 s in all, never an unlimited wait before `write_installer`.
pub(crate) const RECORD_LIMIT: Duration = Duration::from_secs(2);

/// The sentence every failure notice ends with (design §1-6 결정 19, limit 11).
const NOTICE_TAIL_KO: &str =
    "앱 안쪽 수정은 「스스로 해결」이 지원하는 수리 범위 밖이라 이번 업데이트에서 사라질 수 있습니다.";

// ───────────────────────────── where the capture runs ─────────────────────────────

/// The two places of the app's update handler that call the capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Site {
    /// `install_update`, right before the single download-and-install call (the macOS place).
    SingleCall,
    /// `install_update_checked_windows`, after the download, before the installer is written
    /// (constructed only in that function's Windows body).
    #[cfg_attr(not(windows), allow(dead_code))]
    CheckedLaunch,
}

/// Whether the capture runs at `site` on this build (design §4-16; C12 12.8). The single call is
/// the macOS place only: on Windows the same line runs only when the checked launch is switched
/// off — a path the design leaves uncovered — and other Unix targets are not shipped
/// ("Unsupported", C12 interpretation). The checked launch is compiled only on Windows and always
/// captures.
pub(crate) fn site_applies(is_macos: bool, site: Site) -> bool {
    match site {
        Site::CheckedLaunch => true,
        Site::SingleCall => is_macos,
    }
}

// ───────────────────────────── the Rust port of `classify` ─────────────────────────────

/// One entry of the runtime tree, in the shape `javis_runtime_seal.walk_entries` records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Entry {
    /// `{"t": "f", "sha256": …, "x": …}` — `x` = the exec bit as CPython reports it.
    File { sha256: String, x: bool },
    /// `{"t": "l", "target": …}` — never followed.
    Link { target: String },
    /// `{"t": "o", "mode": "0o…"}` — fifo, socket, device.
    Other { mode: String },
}

impl Entry {
    /// `javis_runtime_seal._canonical_line` for a scanned entry.
    fn canonical_line(&self, rel: &str) -> String {
        match self {
            Entry::File { sha256, x } => format!("{rel}\0f\0{sha256}\0{}", if *x { "1" } else { "0" }),
            Entry::Link { target } => format!("{rel}\0l\0{target}"),
            Entry::Other { mode } => format!("{rel}\0o\0{mode}"),
        }
    }
}

/// A scanned entry with the real path it came from (copies read from `path`; `rel` may be lossy
/// for a name that is not UTF-8).
#[derive(Debug, Clone)]
pub(crate) struct Found {
    pub entry: Entry,
    pub path: PathBuf,
    pub len: u64,
}

/// Why a scan or a capture stopped early.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Halt {
    Deadline,
    Abandoned,
}

/// A scan that could not finish: unreadable (Python raises — "판정 불가") or halted by the bounds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ScanError {
    Unreadable(String),
    Halted(Halt),
}

/// The bounds a capture runs under: a deadline and the abandon flag set by the outer timeout.
pub(crate) struct Budget<'a> {
    pub deadline: Instant,
    pub abandon: &'a AtomicBool,
}

impl Budget<'_> {
    fn check(&self) -> Result<(), Halt> {
        if self.abandon.load(Ordering::SeqCst) {
            return Err(Halt::Abandoned);
        }
        if Instant::now() >= self.deadline {
            return Err(Halt::Deadline);
        }
        Ok(())
    }
}

/// Counters of one scan (the WIN-04 quantities: files and bytes hashed, elapsed time).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Stats {
    pub files_hashed: u64,
    pub bytes_hashed: u64,
    pub elapsed_ms: u64,
}

fn unreadable(path: &Path, e: impl std::fmt::Display) -> ScanError {
    ScanError::Unreadable(format!("{}: {e}", path.display()))
}

/// The exec bit as CPython's `os.lstat` reports it on this platform: Unix `st_mode & 0o111`;
/// Windows — CPython's stat emulation (`Modules/posixmodule.c:update_st_mode_from_path`) sets 0111
/// only for the extensions in [`cpython_windows_exec_extension`]. The Windows manifest is emitted by
/// the release runner's CPython, so this is what it holds.
fn exec_bit(meta: &std::fs::Metadata, name: &str) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = name;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        cpython_windows_exec_extension(name)
    }
}

/// CPython on Windows: `.exe`, `.bat`, `.cmd`, `.com`, compared case-insensitively with the text
/// from the LAST dot of the name (a name without a dot never matches). Used by the Windows arms;
/// the tests check it on every platform.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn cpython_windows_exec_extension(name: &str) -> bool {
    match name.rfind('.') {
        Some(i) => [".exe", ".bat", ".cmd", ".com"]
            .iter()
            .any(|e| name[i..].eq_ignore_ascii_case(e)),
        None => false,
    }
}

/// `oct(stat.S_IMODE(st_mode))` of a special file. Unix: the permission bits. Windows: CPython's
/// emulated mode (0o444 read-only / 0o666, plus 0o111 for the exec extensions) — the shipped
/// Windows runtime has no such entry.
fn other_mode(meta: &std::fs::Metadata, name: &str) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = name;
        format!("0o{:o}", meta.permissions().mode() & 0o7777)
    }
    #[cfg(not(unix))]
    {
        let base = if meta.permissions().readonly() { 0o444 } else { 0o666 };
        let x = if cpython_windows_exec_extension(name) { 0o111 } else { 0 };
        format!("0o{:o}", base | x)
    }
}

/// `javis_runtime_seal.walk_entries(root, exclude={MANIFEST_BASENAME})`: relative paths with `/`,
/// links recorded and never followed, regular files hashed (sha256 through
/// `cys::repair::types::TargetState::read`, which opens without following links), everything
/// else an `o` entry. Any I/O error fails the whole scan, as the Python walk raises.
pub(crate) fn walk_entries(
    root: &Path,
    budget: &Budget,
    stats: &mut Stats,
) -> Result<BTreeMap<String, Found>, ScanError> {
    use cys::repair::types::TargetState;
    let mut out = BTreeMap::new();
    let mut stack: Vec<(PathBuf, String)> = vec![(root.to_path_buf(), String::new())];
    while let Some((dir, prefix)) = stack.pop() {
        budget.check().map_err(ScanError::Halted)?;
        let listing = std::fs::read_dir(&dir).map_err(|e| unreadable(&dir, e))?;
        for item in listing {
            let item = item.map_err(|e| unreadable(&dir, e))?;
            let name = item.file_name().to_string_lossy().into_owned();
            let rel = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
            let path = item.path();
            let meta = std::fs::symlink_metadata(&path).map_err(|e| unreadable(&path, e))?;
            let kind = meta.file_type();
            if kind.is_dir() && !kind.is_symlink() {
                stack.push((path, rel));
                continue;
            }
            if rel == MANIFEST_BASENAME {
                continue;
            }
            if kind.is_symlink() {
                let target = std::fs::read_link(&path).map_err(|e| unreadable(&path, e))?;
                let target = target.to_string_lossy().into_owned();
                out.insert(rel, Found { entry: Entry::Link { target }, path, len: 0 });
            } else if kind.is_file() {
                budget.check().map_err(ScanError::Halted)?;
                let sha256 = match TargetState::read(&path) {
                    Ok(TargetState::File { sha256, .. }) => sha256,
                    Ok(_) => return Err(unreadable(&path, "바뀌는 중이라 읽지 못함(검사 도중 종류가 바뀜)")),
                    Err(e) => return Err(unreadable(&path, format!("{e:?}"))),
                };
                stats.files_hashed += 1;
                stats.bytes_hashed += meta.len();
                let x = exec_bit(&meta, &name);
                out.insert(rel, Found { entry: Entry::File { sha256, x }, path, len: meta.len() });
            } else {
                let mode = other_mode(&meta, &name);
                out.insert(rel, Found { entry: Entry::Other { mode }, path, len: 0 });
            }
        }
    }
    Ok(out)
}

/// `javis_runtime_seal.load_manifest`: a JSON object holding `entries`, with `schema` equal to 1 in
/// Python's sense (`1`, `1.0` and `true` compare equal to 1 there).
/// Read without following a link and capped at [`MAX_MANIFEST_BYTES`] (RULINGS AA6).
pub(crate) fn load_manifest(path: &Path) -> Result<Value, String> {
    parse_manifest(&read_capped(path, MAX_MANIFEST_BYTES)?)
}

/// The text half of [`load_manifest`].
pub(crate) fn parse_manifest(bytes: &[u8]) -> Result<Value, String> {
    let v: Value = serde_json::from_slice(bytes).map_err(|e| format!("목록표 형식 오류({e})"))?;
    let Some(obj) = v.as_object() else {
        return Err("목록표 형식 아님(객체가 아님)".to_string());
    };
    if !obj.contains_key("entries") {
        return Err("목록표 형식 아님(entries 없음)".to_string());
    }
    let schema_is_one = match obj.get("schema") {
        Some(Value::Number(n)) => n.as_f64() == Some(1.0),
        Some(Value::Bool(b)) => *b,
        _ => false,
    };
    if !schema_is_one {
        return Err(format!("목록표 schema {} — 1 만 읽는다", obj.get("schema").unwrap_or(&Value::Null)));
    }
    Ok(v)
}

/// Python truthiness of a JSON value (`ent.get("x")`).
fn py_truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().map_or(true, |f| f != 0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
    }
}

/// `"%s" % ent.get(key, "")`. Strings, `null` ("None"), booleans and integers render as Python
/// does; any other value renders as a text no scanned value can equal (it contains NUL), which
/// gives Python's verdict except for a value whose Python `str()` happens to equal the scanned one.
fn py_field(obj: &serde_json::Map<String, Value>, key: &str) -> String {
    match obj.get(key) {
        None => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) => "None".to_string(),
        Some(Value::Bool(true)) => "True".to_string(),
        Some(Value::Bool(false)) => "False".to_string(),
        Some(Value::Number(n)) if n.is_i64() || n.is_u64() => n.to_string(),
        Some(_) => "\0<not reproduced>".to_string(),
    }
}

/// `javis_runtime_seal._canonical_line(rel, want[rel])` for a manifest entry. A non-object entry
/// fails, as `ent.get` raises there.
fn manifest_canonical_line(rel: &str, ent: &Value) -> Result<String, ScanError> {
    let Some(obj) = ent.as_object() else {
        return Err(ScanError::Unreadable(format!("목록표 항목 {rel:?} 이 객체가 아님")));
    };
    Ok(match obj.get("t").and_then(Value::as_str) {
        Some("f") => format!(
            "{rel}\0f\0{}\0{}",
            py_field(obj, "sha256"),
            if py_truthy(obj.get("x")) { "1" } else { "0" }
        ),
        Some("l") => format!("{rel}\0l\0{}", py_field(obj, "target")),
        _ => format!("{rel}\0o\0{}", py_field(obj, "mode")),
    })
}

/// The manifest's entries (`manifest.get("entries") or {}`): an object, or nothing when the value
/// is falsy. Any other value (Python would read a list's items as names) is refused.
fn manifest_entries(manifest: &Value) -> Result<BTreeMap<String, Value>, ScanError> {
    match manifest.get("entries") {
        Some(Value::Object(m)) => Ok(m.iter().map(|(k, v)| (k.clone(), v.clone())).collect()),
        v if !py_truthy(v) => Ok(BTreeMap::new()),
        Some(_) => Err(ScanError::Unreadable("목록표 entries 가 객체가 아님".to_string())),
        None => Ok(BTreeMap::new()),
    }
}

/// The three classes of `javis_runtime_seal.classify`, each sorted (code point order = UTF-8 byte
/// order, as Python's `sorted`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Classification {
    pub missing: Vec<String>,
    pub added: Vec<String>,
    pub changed: Vec<String>,
}

/// `javis_runtime_seal.classify(manifest, root)` over an already scanned tree.
pub(crate) fn classify(manifest: &Value, have: &BTreeMap<String, Found>) -> Result<Classification, ScanError> {
    let want = manifest_entries(manifest)?;
    let mut c = Classification::default();
    for (rel, ent) in &want {
        match have.get(rel) {
            None => c.missing.push(rel.clone()),
            Some(found) => {
                if manifest_canonical_line(rel, ent)? != found.entry.canonical_line(rel) {
                    c.changed.push(rel.clone());
                }
            }
        }
    }
    c.added = have.keys().filter(|rel| !want.contains_key(*rel)).cloned().collect();
    Ok(c)
}

// ───────────────────────────── what is not an edit ─────────────────────────────

/// Updater-leftover file name — the SAME body as `src/bin/cysd/main.rs:is_update_leftover` (the
/// daemon's residue rule; pinned equal by `residue_rule_body_is_identical_to_cysd`). Names made by
/// the installer: the placement chain `<bin>.prev.exe` · `.prev2.exe` · `.prev3.exe` ·
/// `.prev<tick>.exe` and the lock sweep's `<name>.prev<rand>` (`src-tauri/nsis-hooks.nsh`).
pub(crate) fn is_update_leftover(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let Some(i) = lower.rfind(".prev") else {
        return false;
    };
    let tail = &lower[i + ".prev".len()..];
    let digits_end = tail
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(tail.len());
    let rest = &tail[digits_end..];
    rest.is_empty() || rest == ".exe"
}

/// Design §4-16: updater leftovers (`.prev<random>`) and Python caches (`__pycache__`) are not
/// edits. Applied to all three classes.
pub(crate) fn not_an_edit(rel: &str) -> bool {
    let leaf = rel.rsplit('/').next().unwrap_or(rel);
    rel.split('/').any(|c| c == "__pycache__") || is_update_leftover(leaf)
}

// ───────────────────────────── where the bundle is ─────────────────────────────

/// The manifest / runtime pair of an install. Bundle-relative paths start at the folder that holds
/// both (`Contents/Resources` on macOS, the install folder on Windows), so a stored copy is always
/// `runtime/<rel>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Install {
    pub manifest: PathBuf,
    pub runtime: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Located {
    Found(Install),
    /// A bundled runtime without a manifest — nothing to compare against.
    NoManifest { runtime: PathBuf },
    /// No bundled runtime at all (a development build): nothing to capture.
    NoRuntime,
}

/// The same candidates, in the same order, as preflight C80 (`_find_runtime_seal_pair`) and
/// `cys doctor` (`diag_runtime_seal`): the macOS bundle's `Contents/Resources`, then the folder of
/// the executable (Windows install folder), then its `resources/` for the manifest.
pub(crate) fn locate(exe: &Path) -> Located {
    let mut cands: Vec<Install> = Vec::new();
    if let Some(bundle) = cys::app_bundle::enclosing_bundle(exe) {
        let res = bundle.join("Contents").join("Resources");
        cands.push(Install { manifest: res.join(MANIFEST_BASENAME), runtime: res.join(RUNTIME_DIR) });
    }
    if let Some(dir) = exe.parent() {
        cands.push(Install { manifest: dir.join(MANIFEST_BASENAME), runtime: dir.join(RUNTIME_DIR) });
        cands.push(Install { manifest: dir.join("resources").join(MANIFEST_BASENAME), runtime: dir.join(RUNTIME_DIR) });
    }
    if let Some(found) = cands.iter().find(|c| c.manifest.is_file() && c.runtime.is_dir()) {
        return Located::Found(found.clone());
    }
    if let Some(c) = cands.iter().find(|c| c.runtime.is_dir()) {
        return Located::NoManifest { runtime: c.runtime.clone() };
    }
    Located::NoRuntime
}

// ───────────────────────────── the capture ─────────────────────────────

/// Everything one capture needs (no global state is read inside [`capture`]).
#[derive(Debug, Clone)]
pub(crate) struct CaptureInput {
    /// The running executable (`None` = unknown → undecidable).
    pub exe: Option<PathBuf>,
    /// `~/.cys/local/bundle-edits` (the parent of `cys::repair::paths::bundle_edits_dir`).
    pub store_parent: PathBuf,
    /// The capture folder name `<old-version>-<UTC ts>` (the leaf of `bundle_edits_dir`).
    pub leaf: String,
    pub from_version: String,
    pub to_version: String,
    pub deadline: Duration,
    pub max_bytes: u64,
    /// Below this many free bytes on the store volume nothing is copied (RULINGS AA4: cap + 2 GiB).
    pub free_floor: u64,
    /// Free bytes of the volume holding a folder (`None` = could not be measured).
    pub free_space: fn(&Path) -> Option<u64>,
    /// The folder the store's own path is held from without following links (RULINGS DB3): the
    /// user's home in the product; every folder below it on the way to the store must be a real folder.
    pub anchor: PathBuf,
    /// Called once between the scan and the copies with the runtime folder — does nothing in the
    /// product; tests change a file there after the scan (RULINGS DB2).
    pub between_scan_and_copy: fn(&Path),
}

/// The product's `between_scan_and_copy`: nothing happens between the scan and the copies.
fn nothing_between(_: &Path) {}

impl CaptureInput {
    /// The input of a real update of this app: the running version → `to_version`, the store
    /// from the shared path helper (O-C12-3), the default bounds.
    pub(crate) fn for_this_app(to_version: &str) -> CaptureInput {
        let from_version = env!("CARGO_PKG_VERSION").to_string();
        let dir = cys::repair::paths::bundle_edits_dir(&from_version, &utc_stamp(SystemTime::now()));
        let leaf = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let store_parent = dir.parent().map(Path::to_path_buf).unwrap_or_else(|| dir.clone());
        CaptureInput {
            exe: std::env::current_exe().ok(),
            store_parent,
            leaf,
            from_version,
            to_version: to_version.to_string(),
            deadline: CAPTURE_DEADLINE,
            max_bytes: MAX_CAPTURE_BYTES,
            free_floor: MAX_CAPTURE_BYTES + FREE_SPACE_MARGIN,
            free_space: store_free_space,
            anchor: cys::home_dir(),
            between_scan_and_copy: nothing_between,
        }
    }
}

/// The result of one capture. `edits` counts changed + added + deleted files after the filter;
/// `not_stored` counts present edits whose bytes are not in the capture folder (deleted files have
/// none to keep).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CaptureOutcome {
    /// No bundled runtime (development build) — nothing to do, no notice.
    NotApplicable,
    /// Compared, nothing edited — no notice, nothing written.
    Clean { stats: Stats },
    /// Every edit kept and the index written.
    Captured { dir: PathBuf, edits: usize, stats: Stats },
    /// The same edits (same paths, kinds and hashes) are already kept by an earlier capture of the
    /// same version: nothing copied, the new index points to it (`same_as`, RULINGS AA4).
    AlreadyKept { dir: PathBuf, same_as: PathBuf, edits: usize, stats: Stats },
    /// Edits found but not all of them (or not the index) could be kept — including "too little free
    /// space: list only" (RULINGS AA4).
    Incomplete { dir: Option<PathBuf>, edits: usize, not_stored: usize, reason: String, stats: Stats },
    /// Could not tell whether anything was edited.
    Undecidable { reason: String, stats: Stats },
}

/// One record of `index.json` (O-C12-3 as extended by RULINGS AA5 and AG4): `kind` = "file" | "link" |
/// "special"; `present` = the entry existed at capture time (`false` = deleted — this field, never an
/// empty hash, says "deleted"); `stored` = the bytes (or the link) are in THIS capture folder;
/// `sha256_found` = the scanned hash of a present file (`null` for links, special files and deleted
/// entries); `link_target` = the target of a present link (`null` otherwise).
#[derive(Debug, Clone, PartialEq, Eq)]
struct IndexRecord {
    rel: String,
    kind: &'static str,
    present: bool,
    stored: bool,
    sha256_manifest: Option<String>,
    sha256_found: Option<String>,
    link_target: Option<String>,
}

/// UTC `YYYYMMDDTHHMMSSffffffZ` (the `<시각>` format of O-C2-1, used for the capture folder too).
pub(crate) fn utc_stamp(t: SystemTime) -> String {
    let d = t.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = d.as_secs();
    let (y, m, day) = civil_from_days((secs / 86_400) as i64);
    let s = secs % 86_400;
    format!(
        "{y:04}{m:02}{day:02}T{:02}{:02}{:02}{:06}Z",
        s / 3_600,
        (s % 3_600) / 60,
        s % 60,
        d.subsec_micros()
    )
}

/// Days since 1970-01-01 → (year, month, day), proleptic Gregorian (H. Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let year = yoe + era * 400 + if month <= 2 { 1 } else { 0 };
    (year, month, day)
}

/// Opens a file for reading WITHOUT following a link (Unix `O_NOFOLLOW | O_NONBLOCK` — a fifo
/// swapped in cannot block; Windows `FILE_FLAG_OPEN_REPARSE_POINT`) and re-checks through the handle
/// that it is a regular file (the pattern of `cys::repair::types::TargetState::read`, RULINGS AA6).
fn open_no_follow(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(not(unix))]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options.open(path)?;
    if !file.metadata()?.file_type().is_file() {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "not a regular file"));
    }
    Ok(file)
}

/// Reads at most `cap` bytes of a regular file opened without following links; a longer file is
/// refused (RULINGS AA6: the manifest, earlier indexes and pending notices are size-capped).
fn read_capped(path: &Path, cap: u64) -> Result<Vec<u8>, String> {
    let file = open_no_follow(path).map_err(|e| format!("{}: {e}", path.display()))?;
    capped_bytes(file, cap, path)
}

/// The nearest existing folder at or above `p` (the store may not exist yet).
fn existing_dir(p: &Path) -> PathBuf {
    let mut cur = p.to_path_buf();
    while !cur.is_dir() {
        match cur.parent() {
            Some(parent) => cur = parent.to_path_buf(),
            None => break,
        }
    }
    cur
}

/// Free bytes available to this user on the volume that holds `dir` (Unix `statvfs`, Windows
/// `GetDiskFreeSpaceExW`); `None` when it cannot be measured.
pub(crate) fn store_free_space(dir: &Path) -> Option<u64> {
    let dir = existing_dir(dir);
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let c = std::ffi::CString::new(dir.as_os_str().as_bytes()).ok()?;
        let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
            return None;
        }
        let (avail, block) = (st.f_bavail as u64, st.f_frsize as u64);
        Some(avail.saturating_mul(block))
    }
    #[cfg(not(unix))]
    {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")]
        extern "system" {
            fn GetDiskFreeSpaceExW(dir: *const u16, avail: *mut u64, total: *mut u64, free: *mut u64) -> i32;
        }
        let wide: Vec<u16> = dir.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        let (mut avail, mut total, mut free) = (0u64, 0u64, 0u64);
        if unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut avail, &mut total, &mut free) } == 0 {
            return None;
        }
        Some(avail)
    }
}

/// Creates the capture folder inside the held store with an exclusive create (`-<n>` appended on a
/// collision, as O-C2-1 does for vault slots) and holds it without following links (RULINGS DB3).
fn create_capture_dir(store: &HeldDir, leaf: &str) -> Result<HeldDir, String> {
    for n in 0..100u32 {
        let name = if n == 0 { leaf.to_string() } else { format!("{leaf}-{n}") };
        match store.create_dir_new(OsStr::new(&name)) {
            Ok(dir) => return Ok(dir),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("보관 폴더를 만들 수 없음: {}: {e}", store.path().join(&name).display())),
        }
    }
    Err(format!("보관 폴더 이름이 모두 쓰이는 중: {}", store.path().join(leaf).display()))
}

/// The folder that holds `<capture>/runtime/<rel>` — every folder made as needed and held without
/// following links (RULINGS DB3) — and the leaf name; refuses a component that could leave the folder.
fn store_place(capture: &HeldDir, rel: &str) -> Result<(HeldDir, String), String> {
    let Some(mut parts) = safe_parts(rel) else {
        return Err(format!("보관할 수 없는 경로: {rel}"));
    };
    let leaf = parts.pop().unwrap_or_default().to_string();
    let mut held = match capture.child(OsStr::new(RUNTIME_DIR), true) {
        Ok(h) => h,
        Err(e) => return Err(format!("{}: {e}", capture.path().join(RUNTIME_DIR).display())),
    };
    for c in parts {
        held = match held.child(OsStr::new(c), true) {
            Ok(next) => next,
            Err(e) => return Err(format!("{}: {e}", held.path().join(c).display())),
        };
    }
    Ok((held, leaf))
}

// ───────────────────────────── folders held without following links (RULINGS DB1, DB3) ─────────────────────────────

/// A folder held open through a handle. Every name below it is opened relative to that handle —
/// Unix `openat` with `O_NOFOLLOW`; Windows through a path whose every folder is held open (without
/// delete sharing, so it cannot be renamed or replaced meanwhile) and was checked to be no reparse
/// point — so a folder replaced by a link after it was checked cannot redirect a read or a write.
/// `path` is for messages and listings only; `_pins` keeps the folders above open.
pub(crate) struct HeldDir {
    path: PathBuf,
    handle: Arc<std::fs::File>,
    _pins: Vec<Arc<std::fs::File>>,
}

/// The error of a name that is a link (Windows: any reparse point) or no folder where one must be.
const NOT_A_REAL_FOLDER: &str = "링크이거나 폴더가 아니라 따라가지 않음";

/// Open options for a folder handle. Windows: list + read-attributes access, shared for read and
/// write but NOT delete (while it is held the folder cannot be renamed or removed), backup semantics
/// (needed to open a folder) and, for `no_follow`, the reparse point itself instead of its target.
fn dir_options(no_follow: bool) -> std::fs::OpenOptions {
    let mut options = std::fs::OpenOptions::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.read(true).custom_flags(libc::O_DIRECTORY | if no_follow { libc::O_NOFOLLOW } else { 0 });
    }
    #[cfg(not(unix))]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_LIST_DIRECTORY: u32 = 0x0001;
        const FILE_READ_ATTRIBUTES: u32 = 0x0080;
        const FILE_SHARE_READ: u32 = 0x0001;
        const FILE_SHARE_WRITE: u32 = 0x0002;
        const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        options
            .access_mode(FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | if no_follow { FILE_FLAG_OPEN_REPARSE_POINT } else { 0 });
    }
    options
}

/// Re-checks through the handle that it is a real folder: a folder, no link and (Windows) no reparse
/// point of any kind (junction, mount point, symbolic link or other).
fn is_real_folder(file: &std::fs::File) -> std::io::Result<bool> {
    let meta = file.metadata()?;
    #[cfg(not(unix))]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
        if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Ok(false);
        }
    }
    Ok(meta.is_dir() && !meta.file_type().is_symlink())
}

/// Reads at most `cap` bytes of an open file; a longer file is refused (`shown` names it).
fn capped_bytes(file: std::fs::File, cap: u64, shown: &Path) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut std::io::Read::take(file, cap + 1), &mut bytes)
        .map_err(|e| format!("{}: {e}", shown.display()))?;
    if bytes.len() as u64 > cap {
        return Err(format!("{}: {} 바이트보다 커서 읽지 않음", shown.display(), cap));
    }
    Ok(bytes)
}

impl HeldDir {
    /// Holds `path` itself as a starting folder (its own path is followed — the caller trusts it).
    pub(crate) fn open_anchor(path: &Path) -> std::io::Result<HeldDir> {
        let file = dir_options(false).open(path)?;
        if !file.metadata()?.is_dir() {
            return Err(std::io::Error::other("폴더가 아님"));
        }
        Ok(HeldDir { path: path.to_path_buf(), handle: Arc::new(file), _pins: Vec::new() })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// Holds the folder `name` directly below, never following a link; with `create` a missing folder
    /// is made first (Unix `mkdirat` through the handle).
    pub(crate) fn child(&self, name: &OsStr, create: bool) -> std::io::Result<HeldDir> {
        let path = self.path.join(name);
        #[cfg(unix)]
        let file = {
            use std::os::fd::{AsRawFd, FromRawFd};
            use std::os::unix::ffi::OsStrExt;
            let c = std::ffi::CString::new(name.as_bytes()).map_err(std::io::Error::other)?;
            let dirfd = self.handle.as_raw_fd();
            let open = || unsafe { libc::openat(dirfd, c.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC) };
            let mut fd = open();
            if fd < 0 && create && std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound {
                if unsafe { libc::mkdirat(dirfd, c.as_ptr(), 0o777) } != 0 {
                    let e = std::io::Error::last_os_error();
                    if e.kind() != std::io::ErrorKind::AlreadyExists {
                        return Err(e);
                    }
                }
                fd = open();
            }
            if fd < 0 {
                return Err(std::io::Error::last_os_error());
            }
            unsafe { std::fs::File::from_raw_fd(fd) }
        };
        #[cfg(not(unix))]
        let file = match dir_options(true).open(&path) {
            Err(e) if create && e.kind() == std::io::ErrorKind::NotFound => {
                match std::fs::create_dir(&path) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(e) => return Err(e),
                }
                dir_options(true).open(&path)?
            }
            other => other?,
        };
        if !is_real_folder(&file)? {
            return Err(std::io::Error::other(NOT_A_REAL_FOLDER));
        }
        let mut pins = self._pins.clone();
        pins.push(Arc::clone(&self.handle));
        Ok(HeldDir { path, handle: Arc::new(file), _pins: pins })
    }

    /// Makes the NEW folder `name` directly below (an existing name fails `AlreadyExists`) and holds it.
    pub(crate) fn create_dir_new(&self, name: &OsStr) -> std::io::Result<HeldDir> {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            use std::os::unix::ffi::OsStrExt;
            let c = std::ffi::CString::new(name.as_bytes()).map_err(std::io::Error::other)?;
            if unsafe { libc::mkdirat(self.handle.as_raw_fd(), c.as_ptr(), 0o777) } != 0 {
                return Err(std::io::Error::last_os_error());
            }
        }
        #[cfg(not(unix))]
        {
            std::fs::create_dir(self.path.join(name))?;
        }
        self.child(name, false)
    }

    /// Creates the NEW regular file `name` directly below for writing (exclusive: an existing name —
    /// a link included — fails, nothing is followed).
    pub(crate) fn create_file(&self, name: &OsStr) -> std::io::Result<std::fs::File> {
        #[cfg(unix)]
        {
            use std::os::fd::{AsRawFd, FromRawFd};
            use std::os::unix::ffi::OsStrExt;
            let c = std::ffi::CString::new(name.as_bytes()).map_err(std::io::Error::other)?;
            let flags = libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC;
            let fd = unsafe { libc::openat(self.handle.as_raw_fd(), c.as_ptr(), flags, 0o666) };
            if fd < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(unsafe { std::fs::File::from_raw_fd(fd) })
        }
        #[cfg(not(unix))]
        {
            std::fs::OpenOptions::new().write(true).create_new(true).open(self.path.join(name))
        }
    }

    /// Makes the link `name` → `target` directly below (Unix). Windows keeps no links (making one needs
    /// a privilege the app does not ask for).
    pub(crate) fn symlink(&self, target: &str, name: &OsStr) -> std::io::Result<()> {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            use std::os::unix::ffi::OsStrExt;
            let c = std::ffi::CString::new(name.as_bytes()).map_err(std::io::Error::other)?;
            let t = std::ffi::CString::new(target).map_err(std::io::Error::other)?;
            if unsafe { libc::symlinkat(t.as_ptr(), self.handle.as_raw_fd(), c.as_ptr()) } != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        }
        #[cfg(not(unix))]
        {
            let _ = (target, name);
            Err(std::io::Error::other("윈도우에서는 링크를 보관하지 않음"))
        }
    }

    /// Renames `from` to `to`, both directly below.
    pub(crate) fn rename(&self, from: &OsStr, to: &OsStr) -> std::io::Result<()> {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            use std::os::unix::ffi::OsStrExt;
            let f = std::ffi::CString::new(from.as_bytes()).map_err(std::io::Error::other)?;
            let t = std::ffi::CString::new(to.as_bytes()).map_err(std::io::Error::other)?;
            let dirfd = self.handle.as_raw_fd();
            if unsafe { libc::renameat(dirfd, f.as_ptr(), dirfd, t.as_ptr()) } != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        }
        #[cfg(not(unix))]
        {
            std::fs::rename(self.path.join(from), self.path.join(to))
        }
    }

    /// Opens the regular file `name` directly below for reading, never following a link (a fifo
    /// cannot block: `O_NONBLOCK`), re-checked through the handle.
    pub(crate) fn open_file(&self, name: &OsStr) -> std::io::Result<std::fs::File> {
        #[cfg(unix)]
        {
            use std::os::fd::{AsRawFd, FromRawFd};
            use std::os::unix::ffi::OsStrExt;
            let c = std::ffi::CString::new(name.as_bytes()).map_err(std::io::Error::other)?;
            let fd = unsafe {
                libc::openat(self.handle.as_raw_fd(), c.as_ptr(), libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
            };
            if fd < 0 {
                return Err(std::io::Error::last_os_error());
            }
            let file = unsafe { std::fs::File::from_raw_fd(fd) };
            if !file.metadata()?.file_type().is_file() {
                return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "not a regular file"));
            }
            Ok(file)
        }
        #[cfg(not(unix))]
        {
            open_no_follow(&self.path.join(name))
        }
    }

    /// Reads at most `cap` bytes of the regular file `name` directly below (never through a link).
    pub(crate) fn read_capped(&self, name: &OsStr, cap: u64) -> Result<Vec<u8>, String> {
        let shown = self.path.join(name);
        let file = self.open_file(name).map_err(|e| format!("{}: {e}", shown.display()))?;
        capped_bytes(file, cap, &shown)
    }

    /// The target of the link `name` directly below (`None`: no link, unreadable or not UTF-8).
    pub(crate) fn read_link(&self, name: &OsStr) -> Option<String> {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            use std::os::unix::ffi::OsStrExt;
            let c = std::ffi::CString::new(name.as_bytes()).ok()?;
            let mut buf = vec![0u8; 4096];
            let n = unsafe { libc::readlinkat(self.handle.as_raw_fd(), c.as_ptr(), buf.as_mut_ptr().cast(), buf.len()) };
            if n < 0 || n as usize >= buf.len() {
                return None;
            }
            buf.truncate(n as usize);
            String::from_utf8(buf).ok()
        }
        #[cfg(not(unix))]
        {
            std::fs::read_link(self.path.join(name)).ok()?.into_os_string().into_string().ok()
        }
    }
}

/// Whether `p` itself is a link (Windows: any reparse point) — only for the wording of a refusal.
fn is_link_like(p: &Path) -> bool {
    let Ok(meta) = std::fs::symlink_metadata(p) else {
        return false;
    };
    #[cfg(not(unix))]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
        if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        }
    }
    meta.file_type().is_symlink()
}

/// RULINGS DB3: holds the folder `target` without following links — from `anchor` (the user's home:
/// trusted, opened by its path) when `target` lies below it, else from the root of `target`. Every
/// folder on the way is opened through the handle of the one above with no link followed, and made
/// when `create` and missing. A folder on the way that is a link — into the app bundle or anywhere
/// else — or no folder is refused: the caller writes nothing there and the update goes on (AQ7).
pub(crate) fn hold_path(anchor: &Path, target: &Path, create: bool) -> Result<HeldDir, String> {
    let cannot = |p: &Path, e: &dyn std::fmt::Display| {
        if is_link_like(p) {
            format!("보관 폴더가 링크라 따라가지 않고 쓰지 않음: {}", p.display())
        } else {
            format!("보관 폴더를 만들 수 없음: {}: {e}", p.display())
        }
    };
    let start: PathBuf = if anchor.is_absolute() && target.starts_with(anchor) {
        anchor.to_path_buf()
    } else {
        target.components().take_while(|c| matches!(c, Component::Prefix(_) | Component::RootDir)).collect()
    };
    let rest = match target.strip_prefix(&start) {
        Ok(rest) if start.is_absolute() => rest,
        _ => return Err(format!("보관 폴더를 만들 수 없음: {}: 절대 경로가 아님", target.display())),
    };
    let mut held = HeldDir::open_anchor(&start).map_err(|e| cannot(&start, &e))?;
    for c in rest.components() {
        let Component::Normal(name) = c else {
            return Err(format!("보관 폴더를 만들 수 없음: {}: 따라갈 수 없는 경로", target.display()));
        };
        held = match held.child(name, create) {
            Ok(next) => next,
            Err(e) => return Err(cannot(&held.path.join(name), &e)),
        };
    }
    Ok(held)
}

/// The components of an index / store path `a/b/c` — `None` for one that could leave the folder.
fn safe_parts(rel: &str) -> Option<Vec<&str>> {
    let parts: Vec<&str> = rel.split('/').collect();
    if parts.iter().any(|c| c.is_empty() || *c == "." || *c == ".." || c.contains('\\') || c.contains(':')) {
        return None;
    }
    Some(parts)
}

/// SHA-256 (FIPS 180-4), streaming: this crate has no hashing dependency, and a copy is hashed while
/// it is written (RULINGS DB2) and a kept copy re-hashed under the budget before it is reused (DB1).
pub(crate) struct Sha256 {
    state: [u32; 8],
    block: [u8; 64],
    filled: usize,
    total: u64,
}

const SHA256_K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01,
    0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc,
    0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147,
    0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08,
    0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
    0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

impl Sha256 {
    pub(crate) fn new() -> Sha256 {
        Sha256 {
            state: [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19],
            block: [0; 64],
            filled: 0,
            total: 0,
        }
    }

    pub(crate) fn update(&mut self, mut data: &[u8]) {
        self.total = self.total.wrapping_add(data.len() as u64);
        while !data.is_empty() {
            let take = (64 - self.filled).min(data.len());
            self.block[self.filled..self.filled + take].copy_from_slice(&data[..take]);
            self.filled += take;
            data = &data[take..];
            if self.filled == 64 {
                let block = self.block;
                self.compress(&block);
                self.filled = 0;
            }
        }
    }

    fn compress(&mut self, block: &[u8; 64]) {
        let mut w = [0u32; 64];
        for (wi, c) in w.iter_mut().zip(block.chunks_exact(4)) {
            *wi = u32::from_be_bytes([c[0], c[1], c[2], c[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;
        for (k, wi) in SHA256_K.iter().zip(w.iter()) {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = h.wrapping_add(s1).wrapping_add(ch).wrapping_add(*k).wrapping_add(*wi);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (s, v) in self.state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *s = s.wrapping_add(v);
        }
    }

    /// The digest as lowercase hex (the form of the manifest and of `TargetState`).
    pub(crate) fn finish_hex(mut self) -> String {
        let bits = self.total.wrapping_mul(8);
        self.update(&[0x80]);
        while self.filled != 56 {
            self.update(&[0]);
        }
        self.update(&bits.to_be_bytes());
        self.state.iter().map(|v| format!("{v:08x}")).collect()
    }
}

/// Bytes per chunk of a copy or a hash; the deadline and the abandon flag are checked before each
/// (RULINGS DB1, DB2).
pub(crate) const COPY_CHUNK: usize = 64 * 1024;

/// Why a copy stopped early.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CopyStop {
    /// The deadline passed or the update stopped waiting.
    Halted(Halt),
    /// The source holds more bytes than the scan saw (it grew after the scan).
    Grew,
    Io(String),
}

/// RULINGS DB2 / DP1 / DW1: copies at most `expected` bytes (the length the scan saw) in chunks of
/// `COPY_CHUNK`, hashing them as they are written. There is ONE place that starts a read call and ONE
/// that starts a write call, and each asks the budget (deadline, abandon flag) immediately before the
/// call — also before the retry of an interrupted read or write and between the partial writes of one
/// chunk — so no read or write call starts after the timeout. At
/// `expected` one more byte is read to see whether the source grew; that byte is never written and
/// the copy ends `Grew`. Returns the bytes written — only what the write calls returned, however the
/// copy ends, so the capture cap holds for what is really on disk — and the sha256 of the written
/// bytes. Limit: a read or write call already inside the operating system cannot be cancelled; at
/// most that one call completes after the deadline (the caller's own wait stays bounded).
pub(crate) fn copy_stream(input: &mut impl std::io::Read, output: &mut impl std::io::Write, expected: u64, budget: &Budget) -> (u64, Result<String, CopyStop>) {
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; COPY_CHUNK];
    let mut written: u64 = 0;
    loop {
        let room = expected.saturating_sub(written).min(COPY_CHUNK as u64) as usize;
        let n = loop {
            if let Err(h) = budget.check() {
                return (written, Err(CopyStop::Halted(h)));
            }
            match std::io::Read::read(input, &mut buf[..room.max(1)]) {
                Ok(n) => break n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return (written, Err(CopyStop::Io(e.to_string()))),
            }
        };
        if n == 0 {
            return (written, Ok(hasher.finish_hex()));
        }
        if room == 0 {
            return (written, Err(CopyStop::Grew));
        }
        let mut done = 0;
        while done < n {
            if let Err(h) = budget.check() {
                return (written, Err(CopyStop::Halted(h)));
            }
            match std::io::Write::write(output, &buf[done..n]) {
                Ok(0) => return (written, Err(CopyStop::Io(std::io::Error::from(std::io::ErrorKind::WriteZero).to_string()))),
                Ok(k) => {
                    let k = k.min(n - done);
                    hasher.update(&buf[done..done + k]);
                    done += k;
                    written += k as u64;
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return (written, Err(CopyStop::Io(e.to_string()))),
            }
        }
    }
}

/// The sha256 of an open file, read in chunks under the budget — `None` past `cap` bytes, on a read
/// error or when the budget stops it (RULINGS DB1).
fn hash_bounded(file: &mut std::fs::File, cap: u64, budget: &Budget) -> Option<String> {
    match copy_stream(file, &mut std::io::sink(), cap, budget) {
        (_, Ok(sha256)) => Some(sha256),
        _ => None,
    }
}

/// RULINGS DB1: a kept copy is reused only when it still is what its index says — every folder on
/// its way a real folder (no link followed), a file regular and with the indexed sha256 (read under
/// the budget, at most `cap` bytes), a link with the indexed target. Anything else — changed,
/// truncated, replaced, unreadable, cut off by the budget — is not reused: a new capture is made.
fn kept_copy_matches(capture: &HeldDir, rel: &str, kind: &str, sha256: Option<&str>, link_target: Option<&str>, cap: u64, budget: &Budget) -> bool {
    let Some(parts) = safe_parts(rel) else {
        return false;
    };
    let Some((leaf, folders)) = parts.split_last() else {
        return false;
    };
    let mut held: Option<HeldDir> = None;
    for folder in folders {
        let next = held.as_ref().unwrap_or(capture).child(OsStr::new(folder), false);
        match next {
            Ok(h) => held = Some(h),
            Err(_) => return false,
        }
    }
    let parent = held.as_ref().unwrap_or(capture);
    match (kind, sha256, link_target) {
        ("file", Some(want), _) => match parent.open_file(OsStr::new(leaf)) {
            Ok(mut file) => hash_bounded(&mut file, cap, budget).as_deref() == Some(want),
            Err(_) => false,
        },
        ("link", _, Some(want)) => parent.read_link(OsStr::new(leaf)).as_deref() == Some(want),
        _ => false,
    }
}

/// Copies one regular file into the NEW file `name` of a held folder — at most `expected` bytes,
/// under the budget (`copy_stream`, RULINGS DB2) — and returns the bytes written with the sha256 of
/// the stored copy. The source is opened without following links and re-checked through the handle.
fn copy_exact(src: &Path, dst_dir: &HeldDir, name: &str, expected: u64, budget: &Budget) -> (u64, Result<String, CopyStop>) {
    let mut written = 0;
    let result = copy_counted(src, dst_dir, name, expected, budget, &mut written);
    (written, result)
}

/// `copy_exact` with the bytes written reported through `written` (set however the copy ends).
fn copy_counted(src: &Path, dst_dir: &HeldDir, name: &str, expected: u64, budget: &Budget, written: &mut u64) -> Result<String, CopyStop> {
    let fail = |p: &Path, e: std::io::Error| CopyStop::Io(format!("{}: {e}", p.display()));
    let dst = dst_dir.path().join(name);
    let mut input = open_no_follow(src).map_err(|e| fail(src, e))?;
    let permissions = input.metadata().map_err(|e| fail(src, e))?.permissions();
    let mut output = dst_dir.create_file(OsStr::new(name)).map_err(|e| fail(&dst, e))?;
    let (n, copied) = copy_stream(&mut input, &mut output, expected, budget);
    *written = n;
    let sha256 = copied?;
    output.sync_all().map_err(|e| fail(&dst, e))?;
    let _ = output.set_permissions(permissions);
    Ok(sha256)
}

/// Keeps a link as a link inside a held folder (Unix `symlinkat`). Windows does not store links
/// (making one needs a privilege the app does not ask for) — the edit is listed with `stored: false`
/// (RULINGS AA5).
fn store_link(target: &str, dir: &HeldDir, name: &str) -> Result<(), String> {
    dir.symlink(target, OsStr::new(name)).map_err(|e| format!("{}: {e}", dir.path().join(name).display()))
}

/// Writes `index.json` last, through a temporary name and a rename, inside the held capture folder.
fn write_index(dir: &HeldDir, input: &CaptureInput, records: &[IndexRecord], same_as: Option<&str>) -> Result<(), String> {
    let files: Vec<Value> = records
        .iter()
        .map(|r| {
            json!({
                "rel": r.rel,
                "kind": r.kind,
                "present": r.present,
                "stored": r.stored,
                "sha256_manifest": r.sha256_manifest,
                "sha256_found": r.sha256_found,
                "link_target": r.link_target,
            })
        })
        .collect();
    let index = json!({
        "schema": INDEX_SCHEMA,
        "from_version": input.from_version,
        "to_version": input.to_version,
        "same_as": same_as,
        "files": files,
    });
    let mut bytes = serde_json::to_vec_pretty(&index).map_err(|e| e.to_string())?;
    bytes.push(b'\n');
    let partial = format!("{INDEX_FILE}.partial");
    let tmp = dir.path().join(&partial);
    let fin = dir.path().join(INDEX_FILE);
    let mut f = dir.create_file(OsStr::new(&partial)).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::io::Write::write_all(&mut f, &bytes).map_err(|e| format!("{}: {e}", tmp.display()))?;
    f.sync_all().map_err(|e| format!("{}: {e}", tmp.display()))?;
    drop(f);
    dir.rename(OsStr::new(&partial), OsStr::new(INDEX_FILE)).map_err(|e| format!("{}: {e}", fin.display()))
}

/// The identity of an edit set for "already kept" (RULINGS AA4): path, kind, presence, scanned hash,
/// link target of every entry, sorted.
type EditKey = (String, String, bool, Option<String>, Option<String>);
fn edit_set(records: &[IndexRecord]) -> Vec<EditKey> {
    let mut v: Vec<EditKey> = records
        .iter()
        .map(|r| (r.rel.clone(), r.kind.to_string(), r.present, r.sha256_found.clone(), r.link_target.clone()))
        .collect();
    v.sort();
    v
}

/// An earlier capture folder of the same `from_version` that already HOLDS this exact edit set:
/// its index lists the same entries, it points to no other capture, every present entry is stored
/// there and every kept copy is verified against the index (RULINGS DB1: kind, content hash, link
/// target — `kept_copy_matches`). Newest first; `None` when there is none.
fn find_same_capture(store: &HeldDir, input: &CaptureInput, records: &[IndexRecord], budget: &Budget) -> Option<PathBuf> {
    let want = edit_set(records);
    let prefix = format!("{}-", input.from_version);
    let mut names: Vec<String> = std::fs::read_dir(store.path())
        .ok()?
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.starts_with(&prefix))
        .collect();
    names.sort();
    names.reverse();
    for name in names {
        if budget.check().is_err() {
            return None;
        }
        let Ok(held) = store.child(OsStr::new(&name), false) else {
            continue;
        };
        let Ok(bytes) = held.read_capped(OsStr::new(INDEX_FILE), MAX_INDEX_BYTES) else {
            continue;
        };
        let Ok(index) = serde_json::from_slice::<Value>(&bytes) else {
            continue;
        };
        if index["from_version"].as_str() != Some(input.from_version.as_str()) || !index["same_as"].is_null() {
            continue;
        }
        let Some(files) = index["files"].as_array() else {
            continue;
        };
        let mut theirs = Vec::with_capacity(files.len());
        let mut holds_all = true;
        for f in files {
            let (Some(rel), Some(kind), Some(present), Some(stored)) =
                (f["rel"].as_str(), f["kind"].as_str(), f["present"].as_bool(), f["stored"].as_bool())
            else {
                holds_all = false;
                break;
            };
            let sha = f["sha256_found"].as_str().map(str::to_string);
            let target = f["link_target"].as_str().map(str::to_string);
            if present && !stored {
                holds_all = false;
                break;
            }
            theirs.push((rel.to_string(), kind.to_string(), present, sha, target));
        }
        theirs.sort();
        // RULINGS DB1: only an index that lists this edit set is worth the reads; then every kept copy
        // is verified before it is reused.
        if holds_all
            && theirs == want
            && theirs.iter().filter(|e| e.2).all(|(rel, kind, _, sha, target)| {
                kept_copy_matches(&held, rel, kind, sha.as_deref(), target.as_deref(), input.max_bytes, budget)
            })
        {
            return Some(held.path().to_path_buf());
        }
    }
    None
}

fn halt_reason_ko(h: Halt, deadline: Duration) -> String {
    match h {
        Halt::Deadline => format!("{}초 안에 끝나지 않아 멈춤", deadline.as_secs()),
        Halt::Abandoned => "업데이트가 늦어지지 않도록 멈춤".to_string(),
    }
}

fn gib(n: u64) -> String {
    format!("{:.1} GiB", n as f64 / (1024.0 * 1024.0 * 1024.0))
}

/// One capture (blocking). Never panics on I/O; every early end is an outcome.
pub(crate) fn capture(input: &CaptureInput, abandon: &AtomicBool) -> CaptureOutcome {
    let started = Instant::now();
    let budget = Budget { deadline: started + input.deadline, abandon };
    let mut stats = Stats::default();
    let elapsed = |stats: &mut Stats| stats.elapsed_ms = started.elapsed().as_millis() as u64;

    let Some(exe) = input.exe.as_deref() else {
        return CaptureOutcome::Undecidable { reason: "실행 파일 위치를 알 수 없음".to_string(), stats };
    };
    let install = match locate(exe) {
        Located::Found(i) => i,
        Located::NoRuntime => return CaptureOutcome::NotApplicable,
        Located::NoManifest { runtime } => {
            return CaptureOutcome::Undecidable {
                reason: format!("동봉 런타임의 목록표({MANIFEST_BASENAME})가 없어 비교할 수 없음 · {}", runtime.display()),
                stats,
            }
        }
    };
    // RULINGS AA6: a runtime folder that is itself a link is not followed (it could lead anywhere).
    if std::fs::symlink_metadata(&install.runtime).map(|m| m.file_type().is_symlink()).unwrap_or(true) {
        return CaptureOutcome::Undecidable {
            reason: format!("동봉 런타임 폴더가 링크라 따라가지 않음 · {}", install.runtime.display()),
            stats,
        };
    }
    let manifest = match load_manifest(&install.manifest) {
        Ok(m) => m,
        Err(e) => return CaptureOutcome::Undecidable { reason: e, stats },
    };
    let have = match walk_entries(&install.runtime, &budget, &mut stats) {
        Ok(h) => h,
        Err(ScanError::Halted(h)) => {
            elapsed(&mut stats);
            return CaptureOutcome::Undecidable { reason: halt_reason_ko(h, input.deadline), stats };
        }
        Err(ScanError::Unreadable(e)) => {
            elapsed(&mut stats);
            return CaptureOutcome::Undecidable { reason: format!("읽지 못한 파일이 있음: {e}"), stats };
        }
    };
    let classes = match classify(&manifest, &have) {
        Ok(c) => c,
        Err(e) => {
            elapsed(&mut stats);
            let reason = match e {
                ScanError::Unreadable(e) => e,
                ScanError::Halted(h) => halt_reason_ko(h, input.deadline),
            };
            return CaptureOutcome::Undecidable { reason, stats };
        }
    };
    (input.between_scan_and_copy)(&install.runtime);
    let want = manifest_entries(&manifest).unwrap_or_default();
    let mut edits: Vec<&String> = classes
        .changed
        .iter()
        .chain(classes.added.iter())
        .chain(classes.missing.iter())
        .filter(|rel| !not_an_edit(rel))
        .collect();
    edits.sort();
    if edits.is_empty() {
        elapsed(&mut stats);
        return CaptureOutcome::Clean { stats };
    }

    // The index records (RULINGS AA5): kind / scanned hash / link target from the scan for a present
    // entry, kind from the manifest for a deleted one.
    let mut records: Vec<IndexRecord> = edits
        .iter()
        .map(|rel| {
            let manifest_entry = want.get(*rel).and_then(Value::as_object);
            let sha256_manifest = manifest_entry
                .filter(|o| o.get("t").and_then(Value::as_str) == Some("f"))
                .and_then(|o| o.get("sha256"))
                .and_then(Value::as_str)
                .map(str::to_string);
            let (kind, sha256_found, link_target) = match have.get(*rel).map(|f| &f.entry) {
                Some(Entry::File { sha256, .. }) => ("file", Some(sha256.clone()), None),
                Some(Entry::Link { target }) => ("link", None, Some(target.clone())),
                Some(Entry::Other { .. }) => ("special", None, None),
                None => match manifest_entry.and_then(|o| o.get("t")).and_then(Value::as_str) {
                    Some("f") => ("file", None, None),
                    Some("l") => ("link", None, None),
                    _ => ("special", None, None),
                },
            };
            IndexRecord {
                rel: format!("{RUNTIME_DIR}/{rel}"),
                kind,
                present: have.contains_key(*rel),
                stored: false,
                sha256_manifest,
                sha256_found,
                link_target,
            }
        })
        .collect();
    let storable = edits.iter().filter(|rel| have.contains_key(**rel)).count();

    // RULINGS DB3: the store's own path is held without following links — a store, or a folder above
    // it, that is a link (into the app bundle or anywhere else) is refused: nothing is written, the
    // update goes on and the reason is told.
    let store = match hold_path(&input.anchor, &input.store_parent, true) {
        Ok(store) => store,
        Err(reason) => {
            elapsed(&mut stats);
            return CaptureOutcome::Incomplete { dir: None, edits: edits.len(), not_stored: storable, reason, stats };
        }
    };

    // RULINGS AA4 (1): the same edit set already kept by an earlier capture of this version — copy
    // nothing again, point to it.
    if let Some(earlier) = find_same_capture(&store, input, &records, &budget) {
        let same_as = earlier.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let held = match create_capture_dir(&store, &input.leaf) {
            Ok(d) => d,
            Err(reason) => {
                elapsed(&mut stats);
                return CaptureOutcome::Incomplete { dir: None, edits: edits.len(), not_stored: storable, reason, stats };
            }
        };
        let dir = held.path().to_path_buf();
        let index = write_index(&held, input, &records, Some(&same_as));
        elapsed(&mut stats);
        return match index {
            Ok(()) => CaptureOutcome::AlreadyKept { dir, same_as: earlier, edits: edits.len(), stats },
            Err(e) => CaptureOutcome::Incomplete {
                dir: Some(dir),
                edits: edits.len(),
                not_stored: storable,
                reason: format!("목록(index.json)을 쓰지 못함: {e}"),
                stats,
            },
        };
    }

    // RULINGS AA4 (2): never let the capture be the reason the download fails — below the floor copy
    // nothing, list the edits only.
    let low_space = match (input.free_space)(&input.store_parent) {
        None => Some("디스크 여유 공간을 확인하지 못해 복사하지 않고 목록만 남김".to_string()),
        Some(free) if free < input.free_floor => Some(format!(
            "디스크 여유 공간({})이 하한({})보다 적어 복사하지 않고 목록만 남김",
            gib(free),
            gib(input.free_floor)
        )),
        Some(_) => None,
    };

    let held = match create_capture_dir(&store, &input.leaf) {
        Ok(d) => d,
        Err(reason) => {
            elapsed(&mut stats);
            return CaptureOutcome::Incomplete { dir: None, edits: edits.len(), not_stored: storable, reason, stats };
        }
    };
    let dir = held.path().to_path_buf();
    let mut not_stored = 0usize;
    let mut first_reason: Option<String> = low_space.clone();
    let mut copied: u64 = 0;
    let mut halted: Option<Halt> = None;
    for (rel, record) in edits.iter().zip(records.iter_mut()) {
        let Some(found) = have.get(*rel) else {
            continue; // deleted: nothing to keep
        };
        if low_space.is_some() {
            not_stored += 1;
            continue;
        }
        if halted.is_none() {
            halted = budget.check().err();
        }
        let result: Result<(), String> = if let Some(h) = halted {
            Err(halt_reason_ko(h, input.deadline))
        } else if safe_parts(rel).is_none() {
            Err(format!("보관할 수 없는 경로: {rel}"))
        } else {
            match &found.entry {
                Entry::File { sha256, .. } => {
                    if copied.saturating_add(found.len) > input.max_bytes {
                        Err(format!("한 번에 보관하는 크기 상한({} MiB)을 넘음", input.max_bytes / (1024 * 1024)))
                    } else {
                        match store_place(&held, rel) {
                            Err(e) => Err(e),
                            Ok((parent, name)) => {
                                // RULINGS DB2: the cap counts the bytes really written (at most the
                                // scanned length), whatever the end of the copy.
                                let (written, copy) = copy_exact(&found.path, &parent, &name, found.len, &budget);
                                copied = copied.saturating_add(written);
                                match copy {
                                    Ok(copy_sha) if &copy_sha == sha256 => Ok(()),
                                    Ok(_) => Err(format!("검사한 뒤에 바뀌어 검사한 내용과 다름: {rel}")),
                                    Err(CopyStop::Grew) => Err(format!("검사한 뒤에 커져서 보관하지 않음: {rel}")),
                                    Err(CopyStop::Halted(h)) => {
                                        halted = Some(h);
                                        Err(halt_reason_ko(h, input.deadline))
                                    }
                                    Err(CopyStop::Io(e)) => Err(e),
                                }
                            }
                        }
                    }
                }
                Entry::Link { target } => store_place(&held, rel).and_then(|(parent, name)| store_link(target, &parent, &name)),
                Entry::Other { .. } => Err(format!("특수 파일은 보관하지 않음: {rel}")),
            }
        };
        match result {
            Ok(()) => record.stored = true,
            Err(reason) => {
                not_stored += 1;
                first_reason.get_or_insert(reason);
            }
        }
    }
    let index = write_index(&held, input, &records, None);
    elapsed(&mut stats);
    match (not_stored, index) {
        (0, Ok(())) => CaptureOutcome::Captured { dir, edits: edits.len(), stats },
        (_, Err(e)) => CaptureOutcome::Incomplete {
            dir: Some(dir),
            edits: edits.len(),
            not_stored,
            reason: first_reason.unwrap_or_else(|| format!("목록(index.json)을 쓰지 못함: {e}")),
            stats,
        },
        (_, Ok(())) => CaptureOutcome::Incomplete {
            dir: Some(dir),
            edits: edits.len(),
            not_stored,
            reason: first_reason.unwrap_or_default(),
            stats,
        },
    }
}

// ───────────────────────────── the notice ─────────────────────────────

/// One Korean notice: `warn` = something could not be kept or checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Notice {
    pub warn: bool,
    pub title: String,
    pub body: String,
}

/// The notice of an outcome — `None` when there is nothing to tell (no bundled runtime, nothing
/// edited). Every failure says that the update goes on.
pub(crate) fn notice_ko(o: &CaptureOutcome) -> Option<Notice> {
    const KEPT: &str = "앱 안쪽 수정 보관";
    const FAILED: &str = "앱 안쪽 수정 보관 실패";
    match o {
        CaptureOutcome::NotApplicable | CaptureOutcome::Clean { .. } => None,
        CaptureOutcome::Captured { dir, edits, .. } => Some(Notice {
            warn: false,
            title: KEPT.to_string(),
            body: format!(
                "업데이트 전에 앱 안쪽에서 바뀐 파일 {edits}개(고침·추가·삭제)를 찾아, 남아 있는 파일은 {} 에 복사하고 \
                 전부를 목록({INDEX_FILE})에 적었습니다. 업데이트는 그대로 진행합니다. 새 버전에 다시 입히려면 마스터에게 \
                 맡기세요 — 워커가 확인한 뒤 다시 입힙니다.",
                dir.display()
            ),
        }),
        CaptureOutcome::AlreadyKept { dir, same_as, edits, .. } => Some(Notice {
            warn: false,
            title: KEPT.to_string(),
            body: format!(
                "업데이트 전에 앱 안쪽에서 바뀐 파일 {edits}개를 찾았지만, 이전에 {} 에 보관한 것과 같아 다시 복사하지 \
                 않았습니다(목록: {}). 업데이트는 그대로 진행합니다. 새 버전에 다시 입히려면 마스터에게 맡기세요 — 워커가 \
                 확인한 뒤 다시 입힙니다.",
                same_as.display(),
                dir.join(INDEX_FILE).display()
            ),
        }),
        CaptureOutcome::Incomplete { dir: Some(dir), edits, not_stored: 0, reason, .. } => Some(Notice {
            warn: true,
            title: FAILED.to_string(),
            body: format!(
                "업데이트 전에 앱 안쪽에서 바뀐 파일 {edits}개를 {} 에 복사했지만 목록을 남기지 못했습니다({reason}). \
                 업데이트는 그대로 진행합니다. {NOTICE_TAIL_KO}",
                dir.display()
            ),
        }),
        CaptureOutcome::Incomplete { dir: Some(dir), edits, not_stored, reason, .. } => Some(Notice {
            warn: true,
            title: FAILED.to_string(),
            body: format!(
                "앱 안쪽에서 바뀐 파일 {edits}개 가운데 {not_stored}개를 업데이트 전에 보관하지 못했습니다({reason}). \
                 보관한 것과 목록은 {} 에 있습니다(목록: {INDEX_FILE}). 업데이트는 그대로 진행합니다. {NOTICE_TAIL_KO}",
                dir.display()
            ),
        }),
        CaptureOutcome::Incomplete { dir: None, edits, reason, .. } => Some(Notice {
            warn: true,
            title: FAILED.to_string(),
            body: format!(
                "앱 안쪽에서 바뀐 파일 {edits}개를 업데이트 전에 보관하지 못했습니다({reason}). \
                 업데이트는 그대로 진행합니다. {NOTICE_TAIL_KO}"
            ),
        }),
        CaptureOutcome::Undecidable { reason, .. } => Some(Notice {
            warn: true,
            title: "앱 안쪽 수정 확인 실패".to_string(),
            body: format!(
                "앱 안쪽 파일이 바뀌었는지 업데이트 전에 확인하지 못했습니다({reason}). \
                 업데이트는 그대로 진행합니다. {NOTICE_TAIL_KO}"
            ),
        }),
    }
}

/// One stderr line per capture (with the WIN-04 quantities).
pub(crate) fn log_line(o: &CaptureOutcome) -> String {
    let st = |s: &Stats| format!("{} files / {} bytes hashed in {} ms", s.files_hashed, s.bytes_hashed, s.elapsed_ms);
    match o {
        CaptureOutcome::NotApplicable => "bundle-edits: no bundled runtime — nothing to capture".to_string(),
        CaptureOutcome::Clean { stats } => format!("bundle-edits: no edit · {}", st(stats)),
        CaptureOutcome::Captured { dir, edits, stats } => {
            format!("bundle-edits: {edits} edit(s) kept in {} · {}", dir.display(), st(stats))
        }
        CaptureOutcome::AlreadyKept { dir, same_as, edits, stats } => format!(
            "bundle-edits: {edits} edit(s) already kept in {} — index {} · {}",
            same_as.display(),
            dir.display(),
            st(stats)
        ),
        CaptureOutcome::Incomplete { dir, edits, not_stored, reason, stats } => format!(
            "bundle-edits: {not_stored} of {edits} edit(s) not kept ({reason}) · folder {} · {}",
            dir.as_ref().map(|d| d.display().to_string()).unwrap_or_else(|| "-".to_string()),
            st(stats)
        ),
        CaptureOutcome::Undecidable { reason, stats } => format!("bundle-edits: undecidable ({reason}) · {}", st(stats)),
    }
}

// ───────────────────────────── the notice that must outlive the app (RULINGS AA3) ─────────────────────────────

/// Where notice records live: INSIDE the store, `bundle-edits/.notices/` (RULINGS AG1). When the store
/// cannot be created (a plain file in its place) no record is written — the caller says why on stderr.
pub(crate) fn notice_dir_of(store_parent: &Path) -> PathBuf {
    store_parent.join(NOTICES_DIR)
}

/// `bundle-edits/.notices/pending-<leaf>.json` — one write-once record per capture.
pub(crate) fn pending_notice_path(notice_dir: &Path, leaf: &str) -> PathBuf {
    notice_dir.join(format!("{PENDING_NOTICE_PREFIX}{leaf}.json"))
}

/// Records a notice to be shown once at the next app start (write-once: an existing record is never
/// overwritten). The notice folder and every folder above it are held without following links
/// (RULINGS DB3): when one is a link, nothing is written and the caller says why on stderr (AQ7).
pub(crate) fn record_pending_notice(anchor: &Path, store_parent: &Path, leaf: &str, notice: &Notice) -> Result<PathBuf, String> {
    let notices = hold_path(anchor, &notice_dir_of(store_parent), true)?;
    let path = pending_notice_path(notices.path(), leaf);
    let body = json!({"schema": 1, "leaf": leaf, "warn": notice.warn, "title": notice.title, "body": notice.body});
    let mut bytes = serde_json::to_vec(&body).map_err(|e| e.to_string())?;
    bytes.push(b'\n');
    let mut f = notices
        .create_file(path.file_name().unwrap_or_default())
        .map_err(|e| format!("{}: {e}", path.display()))?;
    std::io::Write::write_all(&mut f, &bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    f.sync_all().map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

/// Takes every pending notice ONCE: each record is renamed to `shown-<leaf>.json` BEFORE it is
/// returned (a record that cannot be renamed is not returned, so it can never be handed over twice);
/// unreadable or malformed records are left alone. Nothing is deleted. The notice folder is held
/// without following links (RULINGS DB3) — behind a link nothing is read or renamed; the listing
/// only proposes names, each is opened and renamed through the held folder.
pub(crate) fn take_pending_notices(anchor: &Path, store_parent: &Path) -> Vec<Notice> {
    let Ok(notices) = hold_path(anchor, &notice_dir_of(store_parent), false) else {
        return Vec::new();
    };
    let Ok(listing) = std::fs::read_dir(notices.path()) else {
        return Vec::new();
    };
    let mut names: Vec<String> = listing
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.starts_with(PENDING_NOTICE_PREFIX) && n.ends_with(".json"))
        .collect();
    names.sort();
    let mut out = Vec::new();
    for name in names {
        let Ok(bytes) = notices.read_capped(OsStr::new(&name), MAX_NOTICE_BYTES) else {
            continue;
        };
        let Ok(v) = serde_json::from_slice::<Value>(&bytes) else {
            continue;
        };
        let (Some(1), Some(warn), Some(title), Some(body)) =
            (v["schema"].as_u64(), v["warn"].as_bool(), v["title"].as_str(), v["body"].as_str())
        else {
            continue;
        };
        let leaf = &name[PENDING_NOTICE_PREFIX.len()..name.len() - ".json".len()];
        let shown = format!("{SHOWN_NOTICE_PREFIX}{leaf}.json");
        if notices.rename(OsStr::new(&name), OsStr::new(&shown)).is_ok() {
            out.push(Notice { warn, title: title.to_string(), body: body.to_string() });
        }
    }
    out
}

// ───────────────────────────── the bounded call from the update handler ─────────────────────────────

/// Runs `work` on a blocking thread and waits at most `input.deadline + grace`. On expiry the abandon
/// flag is set (the worker stops at its next file) and an outcome is returned at once; a panic of the
/// worker is an outcome too. The caller always gets an answer — the update is never held.
pub(crate) async fn run_bounded_with<F>(input: CaptureInput, grace: Duration, work: F) -> CaptureOutcome
where
    F: FnOnce(&CaptureInput, &AtomicBool) -> CaptureOutcome + Send + 'static,
{
    let abandon = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&abandon);
    let limit = input.deadline + grace;
    let job = tokio::task::spawn_blocking(move || work(&input, &flag));
    match tokio::time::timeout(limit, job).await {
        Ok(Ok(outcome)) => outcome,
        Ok(Err(_)) => CaptureOutcome::Undecidable {
            reason: "확인 작업이 중간에 비정상으로 끝남".to_string(),
            stats: Stats::default(),
        },
        Err(_) => {
            abandon.store(true, Ordering::SeqCst);
            CaptureOutcome::Undecidable {
                reason: format!("{}초 안에 끝나지 않아 기다리지 않고 진행함", limit.as_secs()),
                stats: Stats::default(),
            }
        }
    }
}

/// Runs blocking `work` on the blocking pool and waits at most `limit` (`None` on expiry or a panic —
/// the caller goes on; the worker is left to finish or die with the process).
pub(crate) async fn bounded_blocking<T, F>(limit: Duration, work: F) -> Option<T>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    match tokio::time::timeout(limit, tokio::task::spawn_blocking(work)).await {
        Ok(Ok(value)) => Some(value),
        _ => None,
    }
}

/// Shows a notice now: for a warning the existing in-app `update-error` toast ("업데이트 경고"), for every
/// notice an OS notification through the notification plugin already registered by the app. Each
/// channel is best effort and cannot fail the update.
fn deliver_notice(app: &tauri::AppHandle, n: &Notice) {
    if n.warn {
        let _ = app.emit("update-error", n.body.clone());
    }
    if let Some(plugin) = app.try_state::<tauri_plugin_notification::Notification<tauri::Wry>>() {
        let _ = plugin.builder().title(n.title.clone()).body(n.body.clone()).show();
    }
}

/// THE call of the update handler (both places). Returns when the capture is done, cut off or
/// failed — never an error: the update goes on in every case. The notice is told exactly once: at
/// the macOS place now; at the Windows place the handler leaves the process right after (write →
/// launch → exit) before any banner task could run, so it is recorded inside the store and handed to
/// the UI at the next start (RULINGS AA3, AG1–AG3). The whole call waits at most
/// `CAPTURE_DEADLINE + JOIN_GRACE` (40 s): at the Windows place the record write's own limit is taken
/// out of the grace.
pub(crate) async fn capture_before_install(app: &tauri::AppHandle, to_version: &str, site: Site) {
    if !site_applies(cfg!(target_os = "macos"), site) {
        return;
    }
    let input = CaptureInput::for_this_app(to_version);
    let (anchor, store_parent, leaf) = (input.anchor.clone(), input.store_parent.clone(), input.leaf.clone());
    let grace = match site {
        Site::CheckedLaunch => JOIN_GRACE - RECORD_LIMIT,
        Site::SingleCall => JOIN_GRACE,
    };
    let outcome = run_bounded_with(input, grace, capture).await;
    eprintln!("[cys-app] {}", log_line(&outcome));
    let Some(notice) = notice_ko(&outcome) else {
        return;
    };
    match site {
        Site::CheckedLaunch => {
            match bounded_blocking(RECORD_LIMIT, move || record_pending_notice(&anchor, &store_parent, &leaf, &notice)).await {
                Some(Ok(_)) => {}
                Some(Err(e)) => eprintln!("[cys-app] bundle-edits: notice not recorded ({e})"),
                None => eprintln!("[cys-app] bundle-edits: notice not recorded (no answer within {} s)", RECORD_LIMIT.as_secs()),
            }
        }
        Site::SingleCall => deliver_notice(app, &notice),
    }
}

/// The start-time pull (`bundle_edits_pending_notices`, called by the UI after it registered its
/// listeners — RULINGS AG3): every pending record of the store, each `{warn, title, body}`, handed over
/// once (renamed before it is returned).
pub(crate) fn take_pending_notices_for_ui() -> Vec<Value> {
    let store = cys::repair::paths::bundle_edits_dir("0", "0");
    let store_parent = store.parent().map(Path::to_path_buf).unwrap_or(store);
    take_pending_notices(&cys::home_dir(), &store_parent)
        .into_iter()
        .map(|n| json!({"warn": n.warn, "title": n.title, "body": n.body}))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cys::repair::types::TargetState;
    use std::sync::atomic::AtomicU64;

    // ───────── helpers ─────────

    /// A fresh, empty temporary folder (removed by `Drop`).
    struct Tmp(PathBuf);
    impl Tmp {
        fn new(tag: &str) -> Tmp {
            static N: AtomicU64 = AtomicU64::new(0);
            let p = std::env::temp_dir().join(format!(
                "cys-t7a-{tag}-{}-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::SeqCst),
                SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
            ));
            std::fs::create_dir_all(&p).unwrap();
            Tmp(p)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn fixture() -> Value {
        serde_json::from_str(include_str!("../../cysjavis-pack/bin/tests/fixtures/runtime-seal-rust-parity.json"))
            .expect("the parity fixture is JSON")
    }

    fn case<'a>(fx: &'a Value, name: &str) -> &'a Value {
        fx["cases"].as_array().unwrap().iter().find(|c| c["name"] == name).unwrap_or_else(|| panic!("case {name}"))
    }

    fn sha_of(path: &Path) -> String {
        match TargetState::read(path).unwrap() {
            TargetState::File { sha256, .. } => sha256,
            other => panic!("{}: not a file: {other:?}", path.display()),
        }
    }

    /// Builds a tree from a fixture spec (`f` file, `l` link, `p` fifo). Returns false when a kind
    /// cannot be made on this platform.
    fn materialize(spec: &Value, root: &Path) -> bool {
        std::fs::create_dir_all(root).unwrap();
        for (rel, ent) in spec.as_object().unwrap() {
            let p = rel.split('/').fold(root.to_path_buf(), |acc, c| acc.join(c));
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            match ent["t"].as_str().unwrap() {
                "f" => {
                    std::fs::write(&p, ent["text"].as_str().unwrap().as_bytes()).unwrap();
                    set_exec(&p, ent["x"].as_bool().unwrap());
                }
                "l" => {
                    if !make_link(ent["target"].as_str().unwrap(), &p) {
                        return false;
                    }
                }
                "p" => {
                    if !make_fifo(&p, ent["mode"].as_u64().unwrap() as u32) {
                        return false;
                    }
                }
                k => panic!("unknown spec kind {k}"),
            }
        }
        true
    }

    fn set_exec(p: &Path, x: bool) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(if x { 0o755 } else { 0o644 })).unwrap();
        }
        #[cfg(not(unix))]
        {
            let _ = (p, x);
        }
    }

    fn make_link(target: &str, p: &Path) -> bool {
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(target, p).unwrap();
            true
        }
        #[cfg(not(unix))]
        {
            let _ = (target, p);
            false
        }
    }

    fn make_fifo(p: &Path, mode: u32) -> bool {
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            use std::os::unix::fs::PermissionsExt;
            let c = std::ffi::CString::new(p.as_os_str().as_bytes()).unwrap();
            assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0, "mkfifo {}", p.display());
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(mode)).unwrap();
            true
        }
        #[cfg(not(unix))]
        {
            let _ = (p, mode);
            false
        }
    }

    /// On Windows CPython reports the exec bit by extension — the fixture holds the Unix bit.
    fn platform_entries(entries: &Value) -> Value {
        let mut out = entries.clone();
        if cfg!(windows) {
            for (rel, ent) in out.as_object_mut().unwrap() {
                if ent["t"] == "f" {
                    let leaf = rel.rsplit('/').next().unwrap();
                    ent["x"] = Value::Bool(cpython_windows_exec_extension(leaf));
                }
            }
        }
        out
    }

    fn unlimited() -> (AtomicBool, Instant) {
        (AtomicBool::new(false), Instant::now() + Duration::from_secs(3_600))
    }

    fn scan(root: &Path) -> BTreeMap<String, Found> {
        let (flag, deadline) = unlimited();
        walk_entries(root, &Budget { deadline, abandon: &flag }, &mut Stats::default()).expect("scan")
    }

    /// The manifest JSON shape of a scanned entry (what the Python emitter writes).
    fn entry_json(e: &Entry) -> Value {
        match e {
            Entry::File { sha256, x } => json!({"t": "f", "sha256": sha256, "x": x}),
            Entry::Link { target } => json!({"t": "l", "target": target}),
            Entry::Other { mode } => json!({"t": "o", "mode": mode}),
        }
    }

    fn classes_json(c: &Classification) -> Value {
        json!({"missing": c.missing, "added": c.added, "changed": c.changed})
    }

    fn manifest_of(c: &Value) -> Value {
        let mut entries = platform_entries(&c["manifest_entries"]);
        for (k, v) in c["manifest_patch"].as_object().unwrap() {
            entries[k] = v.clone();
        }
        json!({"schema": 1, "entries": entries})
    }

    // ───────── parity with javis_runtime_seal.py (O-C12-2) ─────────

    /// ★O-C12-2: the Rust walk reproduces, entry for entry, what the Python emitter
    /// (`javis_runtime_seal.build_manifest`) recorded for every fixture tree. The Python leg
    /// (`test_runtime_manifest_parity.py:RustPortParityTests`) proves the fixture IS the emitter's output.
    #[test]
    fn parity_walk_reproduces_the_python_emitter_entries() {
        let fx = fixture();
        let mut ran = 0;
        for c in fx["cases"].as_array().unwrap() {
            let t = Tmp::new("parity-walk");
            let root = t.path().join("runtime");
            if !materialize(&c["shipped"], &root) {
                assert!(c["unix_only"].as_bool().unwrap(), "{}: a kind this platform cannot make outside a unix-only case", c["name"]);
                continue;
            }
            let got: serde_json::Map<String, Value> = scan(&root).into_iter().map(|(k, f)| (k, entry_json(&f.entry))).collect();
            assert_eq!(Value::Object(got), platform_entries(&c["manifest_entries"]), "{}: Rust walk ≠ Python emitter", c["name"]);
            ran += 1;
        }
        assert!(ran >= if cfg!(unix) { 4 } else { 2 }, "parity ran on {ran} case(s) only — measurement failure is not a pass");
    }

    /// ★O-C12-2: the Rust classify gives exactly the Python `classify` lists on every fixture case.
    #[test]
    fn parity_classify_matches_python_on_the_fixture() {
        let fx = fixture();
        let mut ran = 0;
        for c in fx["cases"].as_array().unwrap() {
            let t = Tmp::new("parity-classify");
            let root = t.path().join("runtime");
            if !materialize(&c["found"], &root) {
                assert!(c["unix_only"].as_bool().unwrap());
                continue;
            }
            let got = classes_json(&classify(&manifest_of(c), &scan(&root)).expect("classify"));
            assert_eq!(got, c["expected"], "{}: Rust classify ≠ Python classify", c["name"]);
            ran += 1;
        }
        assert!(ran >= if cfg!(unix) { 4 } else { 2 }, "parity ran on {ran} case(s) only");
    }

    /// The fixture is not vacuous: every class, links, a fifo, the root manifest exclusion, the odd
    /// manifest entries and both kinds of non-edit appear in it.
    #[test]
    fn parity_fixture_covers_every_class_and_quirk() {
        let fx = fixture();
        let all = |k: &str| -> Vec<String> {
            fx["cases"].as_array().unwrap().iter().flat_map(|c| c["expected"][k].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect::<Vec<_>>()).collect()
        };
        for k in ["missing", "added", "changed"] {
            assert!(!all(k).is_empty(), "no case with a {k} entry");
        }
        let text = fx.to_string();
        for needle in ["\"t\":\"l\"", "\"t\":\"p\"", "__pycache__", ".prev4213", ".prev2.exe", "notes.preview.txt", "홍길동", "sub/runtime-manifest.json", "not-an-object", "\"x\":\"yes\""] {
            assert!(text.contains(needle), "the fixture lost {needle}");
        }
        assert!(!all("added").contains(&MANIFEST_BASENAME.to_string()), "the root manifest must not be an entry");
    }

    /// The Python reference functions are pinned by text: if `javis_runtime_seal.py` changes how it
    /// walks, hashes, reads or compares, this turns red until the Rust port and the fixture are
    /// re-checked against it (then update the constant).
    #[test]
    fn parity_python_reference_functions_unchanged() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../cysjavis-pack/bin/javis_runtime_seal.py");
        let src = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let def = |name: &str| -> String {
            let at = src.find(&format!("\ndef {name}(")).unwrap_or_else(|| panic!("def {name} gone"));
            let rest = &src[at + 1..];
            let end = rest[1..].find("\ndef ").map(|i| i + 1).unwrap_or(rest.len());
            rest[..end].to_string()
        };
        assert!(src.contains("\nSCHEMA = 1\n") && src.contains("\nMANIFEST_BASENAME = \"runtime-manifest.json\"\n"));
        let joined: String = ["_sha256_file", "walk_entries", "_canonical_line", "classify", "load_manifest"].iter().map(|n| def(n)).collect();
        assert_eq!(
            sha1_hex(joined.as_bytes()),
            PY_REFERENCE_SHA1,
            "javis_runtime_seal.py changed in a function the Rust port mirrors — re-check bundle_edits.rs and regenerate the fixture"
        );
    }
    const PY_REFERENCE_SHA1: &str = "a48879dac1dbcd68bc3daef2d591dc836cb7eb76";

    // ───────── manifest reading and quirks ─────────

    #[test]
    fn manifest_schema_and_entry_defaults_follow_python() {
        for ok in [r#"{"schema":1,"entries":{}}"#, r#"{"schema":1.0,"entries":{}}"#, r#"{"schema":true,"entries":{}}"#] {
            assert!(parse_manifest(ok.as_bytes()).is_ok(), "{ok}");
        }
        for bad in [r#"{"schema":2,"entries":{}}"#, r#"{"schema":"1","entries":{}}"#, r#"{"entries":{}}"#, r#"{"schema":1}"#, "[]", "{", "\u{feff}{\"schema\":1,\"entries\":{}}"] {
            assert!(parse_manifest(bad.as_bytes()).is_err(), "{bad}");
        }
        // `manifest.get("entries") or {}`
        for empty in [json!({"entries": null}), json!({"entries": {}}), json!({"entries": []}), json!({"entries": 0})] {
            assert_eq!(classify(&empty, &BTreeMap::new()).unwrap(), Classification::default());
        }
        assert!(classify(&json!({"entries": ["a"]}), &BTreeMap::new()).is_err(), "a non-empty list is refused, not guessed");
        // a non-object entry fails only when it must be compared (Python's `.get` raises there)
        let mut have = BTreeMap::new();
        have.insert("a".to_string(), Found { entry: Entry::File { sha256: "00".into(), x: false }, path: PathBuf::from("a"), len: 0 });
        assert!(classify(&json!({"entries": {"a": "str"}}), &have).is_err());
        assert_eq!(classify(&json!({"entries": {"b": "str"}}), &have).unwrap().missing, vec!["b".to_string()]);
        // truthiness
        for (v, t) in [(json!(0), false), (json!(0.0), false), (json!(-0.0), false), (json!(""), false), (json!([]), false), (json!({}), false), (Value::Null, false), (json!(1), true), (json!("0"), true), (json!([0]), true), (json!(true), true)] {
            assert_eq!(py_truthy(Some(&v)), t, "{v}");
        }
        assert!(!py_truthy(None));
    }

    /// CPython 3.12 `update_st_mode_from_path`: exec bits for `.exe .bat .cmd .com`, case-insensitive,
    /// from the last dot of the name.
    #[test]
    fn exec_bit_windows_rule_matches_cpython_extensions() {
        for yes in ["bash.exe", "X.EXE", "run.Bat", "a.cmd", "b.COM", ".exe", "a.b.exe"] {
            assert!(cpython_windows_exec_extension(yes), "{yes}");
        }
        for no in ["python3", "lib.dll", "a.exe.prev12", "exe", "a.exex", "a.ex", "readme.md", "a.exe.bak", ""] {
            assert!(!cpython_windows_exec_extension(no), "{no}");
        }
    }

    // ───────── what is not an edit (the residue rule and its five sites) ─────────

    /// The capture's leftover rule is the daemon's rule, character for character (design D:1223 —
    /// the five sites change together; the three installer sites are byte-pinned below).
    #[test]
    fn residue_rule_body_is_identical_to_cysd() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/bin/cysd/main.rs");
        let cysd = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let mine = include_str!("bundle_edits.rs");
        let body = |src: &str| -> String {
            let sig = "pub(crate) fn is_update_leftover(name: &str) -> bool {\n";
            let at = src.find(sig).unwrap_or_else(|| panic!("`{sig}` gone"));
            let rest = &src[at..];
            rest[..rest.find("\n}\n").expect("end of fn")].to_string()
        };
        assert_eq!(body(mine), body(&cysd), "the leftover rule drifted from cysd::is_update_leftover — change both together");
    }

    #[test]
    fn residue_rule_accepts_every_installer_form_and_spares_user_names() {
        // the daemon test's table (cysd `leftover_rule_matches_installer_forms_only`)
        for n in ["cysd.prev.exe", "cys.prev2.exe", "cysd.prev3.exe", "CYSD.PREV.EXE", "cys-app.prev123456.exe", "msys-2.0.dll.prev4213", "bash.exe.prev0", "python313.dll.prev99999", "vcruntime140.dll.prev"] {
            assert!(is_update_leftover(n), "{n}");
        }
        for n in ["notes.preview.png", "release.previous.json", "prev.exe", "cysd.prev.exe.bak", "cys.prevX.exe", "cys.new.exe", "readme.md"] {
            assert!(!is_update_leftover(n), "{n}");
        }
        assert!(not_an_edit("python/Lib/__pycache__/site.cpython-312.pyc"));
        assert!(not_an_edit("__pycache__"));
        assert!(not_an_edit("git/usr/bin/msys-2.0.dll.prev4213"));
        assert!(!not_an_edit("python/Lib/site.py"));
        assert!(!not_an_edit("x.prev12/inner.txt"), "a folder named like a leftover is not a leftover (the daemon never touches folders)");
        assert!(!not_an_edit("notes.preview.txt"));
    }

    /// The installer's three naming sites still produce exactly the forms the rule above accepts. They
    /// cannot change in 0.15.0 (byte identity), but this pin outlives that release.
    #[test]
    fn installer_residue_forms_are_still_the_ones_the_rule_accepts() {
        let nsh = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("nsis-hooks.nsh")).unwrap();
        for form in [
            "\"$INSTDIR\\${BIN}.prev.exe\"",
            "\"$INSTDIR\\${BIN}.prev2.exe\"",
            "\"$INSTDIR\\${BIN}.prev3.exe\"",
            "\"$INSTDIR\\${BIN}.prev$R1.exe\"",
            "$$f.FullName + \".prev\" + (Get-Random -Maximum 99999)",
            "-like \"*.prev*\"",
            "Delete \"$INSTDIR\\cys.prev*.exe\"",
        ] {
            assert!(nsh.contains(form), "nsis-hooks.nsh no longer holds {form}");
        }
        // instances of those forms
        for n in ["cys.prev.exe", "cysd.prev2.exe", "cys-app.prev3.exe", "cysd.prev4294967.exe", "python312.dll.prev0", "libgit2.dll.prev99998"] {
            assert!(not_an_edit(&format!("git/bin/{n}")), "{n}");
        }
    }

    // ───────── the capture (testplan C-p18-03…06, C-sp1-02/03) ─────────

    /// A fixture install. `mac` = `<t>/cys.app/Contents/{MacOS/cys-app, Resources/{runtime-manifest.json,
    /// runtime/…}}`; `win` = `<t>/inst/{cys-app.exe, runtime-manifest.json, runtime/…}` (the NSIS layout of
    /// `tauri.windows.conf.json`); `win-res` = the manifest in `<t>/inst/resources/`. The runtime is case A's
    /// found tree (one edited file, one deleted, three added, two updater leftovers, a Python cache) and the
    /// manifest is case A's Python-emitted manifest. Returns (exe, resource root, top folder).
    fn fixture_install(t: &Path, layout: &str) -> (PathBuf, PathBuf, PathBuf) {
        let fx = fixture();
        let c = case(&fx, "edits-leftovers-pycache-korean");
        let (top, exe, res, manifest_dir) = match layout {
            "mac" => {
                let b = t.join("cys.app");
                (b.clone(), b.join("Contents/MacOS/cys-app"), b.join("Contents/Resources"), b.join("Contents/Resources"))
            }
            "win" => (t.join("inst"), t.join("inst/cys-app.exe"), t.join("inst"), t.join("inst")),
            "win-res" => (t.join("inst"), t.join("inst/cys-app.exe"), t.join("inst"), t.join("inst/resources")),
            other => panic!("layout {other}"),
        };
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, b"app").unwrap();
        assert!(materialize(&c["found"], &res.join(RUNTIME_DIR)));
        std::fs::create_dir_all(&manifest_dir).unwrap();
        std::fs::write(manifest_dir.join(MANIFEST_BASENAME), serde_json::to_vec(&manifest_of(c)).unwrap()).unwrap();
        (exe, res, top)
    }

    const LEAF: &str = "0.15.0-20261113T010203000004Z";

    fn input_for(exe: &Path, store_parent: &Path) -> CaptureInput {
        CaptureInput {
            exe: Some(exe.to_path_buf()),
            store_parent: store_parent.to_path_buf(),
            leaf: LEAF.to_string(),
            from_version: "0.15.0".to_string(),
            to_version: "0.15.1".to_string(),
            deadline: Duration::from_secs(120),
            max_bytes: MAX_CAPTURE_BYTES,
            free_floor: MAX_CAPTURE_BYTES + FREE_SPACE_MARGIN,
            free_space: plenty, // AG4: a stub by default — no test depends on the temp volume's real free space
            anchor: std::env::temp_dir(), // DB3: every temporary store lies below it (`Tmp`)
            between_scan_and_copy: nothing_between,
        }
    }

    /// The default free-space stub: always far above the floor.
    fn plenty(_: &Path) -> Option<u64> {
        Some(1 << 50)
    }

    /// Every entry under `root` (no exclusion), to prove a tree was not touched.
    fn snapshot(root: &Path) -> Vec<(String, Value)> {
        scan(root).into_iter().map(|(k, f)| (k, entry_json(&f.entry))).collect()
    }

    fn join_rel(base: &Path, rel: &str) -> PathBuf {
        rel.split('/').fold(base.to_path_buf(), |acc, c| acc.join(c))
    }

    /// Every file / link below `root` as relative `/` paths.
    fn listing(root: &Path) -> Vec<String> {
        let mut out = Vec::new();
        let mut stack = vec![(root.to_path_buf(), String::new())];
        while let Some((d, pre)) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap() {
                let e = e.unwrap();
                let name = e.file_name().to_string_lossy().into_owned();
                let rel = if pre.is_empty() { name } else { format!("{pre}/{name}") };
                let ft = std::fs::symlink_metadata(e.path()).unwrap().file_type();
                if ft.is_dir() {
                    stack.push((e.path(), rel));
                } else {
                    out.push(rel);
                }
            }
        }
        out.sort();
        out
    }

    /// C-p18-03 (mac) / C-p18-04 and C-sp1-02 (Windows install folder): the edits are copied byte for byte
    /// to `<store>/<old>-<ts>/runtime/<rel>`, the leftovers and the cache are not, the deleted file is listed
    /// without bytes, `index.json` has the O-C12-3 shape, nothing outside the store is written and the
    /// bundle is not touched.
    fn assert_capture_keeps_the_edits(layout: &str) {
        let fx = fixture();
        let c = case(&fx, "edits-leftovers-pycache-korean");
        let t = Tmp::new(&format!("capture-{layout}"));
        let (exe, res, top) = fixture_install(t.path(), layout);
        let before = snapshot(&top);
        let store = t.path().join("home").join(".cys").join("local").join("bundle-edits");
        let outcome = capture(&input_for(&exe, &store), &AtomicBool::new(false));
        let CaptureOutcome::Captured { dir, edits, stats } = &outcome else {
            panic!("{layout}: {outcome:?}");
        };
        assert_eq!(dir, &store.join(LEAF), "{layout}: the capture folder is <store>/<old-version>-<UTC ts>");
        assert_eq!(*edits, 5, "{layout}: site.py changed · mod.py deleted · 3 added (leftovers and cache excluded)");
        assert_eq!(stats.files_hashed, 11, "{layout}: every runtime file was hashed");
        let kept = ["node/lib/node_modules/@user/tool/index.js", "notes.preview.txt", "python/lib/site.py", "python/lib/홍길동 메모.txt"];
        for rel in kept {
            assert_eq!(
                std::fs::read(join_rel(&dir.join(RUNTIME_DIR), rel)).unwrap(),
                std::fs::read(join_rel(&res.join(RUNTIME_DIR), rel)).unwrap(),
                "{layout}: {rel} not byte-exact"
            );
        }
        let memo = std::fs::read(join_rel(&dir.join(RUNTIME_DIR), "python/lib/홍길동 메모.txt")).unwrap();
        assert!(memo.starts_with(&[0xEF, 0xBB, 0xBF]) && memo.ends_with(b"\r\n"), "{layout}: BOM and CRLF must be kept byte for byte (A-3 #22, #24)");
        let mut want: Vec<String> = kept.iter().map(|r| format!("runtime/{r}")).collect();
        want.push(INDEX_FILE.to_string());
        want.sort();
        assert_eq!(listing(dir), want, "{layout}: exactly the edits and the index — no leftover, no cache, no partial index");
        let index: Value = serde_json::from_slice(&std::fs::read(dir.join(INDEX_FILE)).unwrap()).unwrap();
        assert_eq!(index["schema"], 1);
        assert_eq!(index["from_version"], "0.15.0");
        assert_eq!(index["to_version"], "0.15.1");
        assert_eq!(index.as_object().unwrap().len(), 5, "index.json = {{schema, from_version, to_version, same_as, files}}");
        assert!(index["same_as"].is_null());
        let files = index["files"].as_array().unwrap();
        let rels: Vec<&str> = files.iter().map(|f| f["rel"].as_str().unwrap()).collect();
        assert_eq!(
            rels,
            ["runtime/node/lib/node_modules/@user/tool/index.js", "runtime/notes.preview.txt", "runtime/python/lib/mod.py", "runtime/python/lib/site.py", "runtime/python/lib/홍길동 메모.txt"]
        );
        let man = &c["manifest_entries"];
        for f in files {
            assert_eq!(f.as_object().unwrap().len(), 7, "{f}: {{rel, kind, present, stored, sha256_manifest, sha256_found, link_target}} (AA5, AG4)");
            let rel = f["rel"].as_str().unwrap().strip_prefix("runtime/").unwrap();
            let expect_manifest = man.get(rel).map(|e| e["sha256"].clone()).unwrap_or(Value::Null);
            assert_eq!(f["sha256_manifest"], expect_manifest, "{rel}");
            assert_eq!(f["kind"], "file", "{rel}");
            assert!(f["link_target"].is_null(), "{rel}");
            let src = join_rel(&res.join(RUNTIME_DIR), rel);
            let present = src.exists();
            let expect_found = if present { Value::String(sha_of(&src)) } else { Value::Null };
            assert_eq!(f["sha256_found"], expect_found, "{rel}: the scanned hash for a present file, null only for a deleted one");
            assert_eq!(f["stored"], present, "{rel}: every present edit kept, the deleted one not");
            assert_eq!(f["present"], present, "{rel}: `present` says deleted (AG4)");
        }
        assert_eq!(snapshot(&top), before, "{layout}: the capture changed the bundle");
        assert_eq!(listing(&t.path().join("home")), listing(&store).iter().map(|r| format!(".cys/local/bundle-edits/{r}")).collect::<Vec<_>>(), "nothing written outside the store");
        let n = notice_ko(&outcome).expect("a kept edit is told");
        assert!(!n.warn && n.body.contains("5개") && n.body.contains(&dir.display().to_string()) && n.body.contains("업데이트는 그대로 진행합니다"));
    }

    #[test]
    fn capture_keeps_edits_of_a_mac_bundle() {
        assert_capture_keeps_the_edits("mac");
    }

    #[test]
    fn capture_keeps_edits_of_a_windows_install_folder() {
        assert_capture_keeps_the_edits("win");
        assert_capture_keeps_the_edits("win-res");
    }

    /// C-p18-05 / C-p18-06 / C-sp1-03: a plain FILE where the store folder must be. The capture returns at
    /// once without storing, the plain file is untouched, and the Korean notice says the update goes on.
    /// The bounded wrapper (what the update handler awaits) returns the same outcome — the caller proceeds.
    #[test]
    fn copy_failure_never_blocks_and_tells_the_user() {
        for layout in ["mac", "win"] {
            let t = Tmp::new(&format!("copyfail-{layout}"));
            let (exe, _res, _top) = fixture_install(t.path(), layout);
            let local = t.path().join("home").join(".cys").join("local");
            std::fs::create_dir_all(&local).unwrap();
            let blocker = local.join("bundle-edits");
            std::fs::write(&blocker, b"a plain file").unwrap();
            let started = Instant::now();
            let outcome = capture(&input_for(&exe, &blocker), &AtomicBool::new(false));
            assert!(started.elapsed() < Duration::from_secs(10), "{layout}: the failure took {:?}", started.elapsed());
            let CaptureOutcome::Incomplete { dir: None, edits: 5, not_stored: 4, reason, .. } = &outcome else {
                panic!("{layout}: {outcome:?}");
            };
            assert!(reason.contains("보관 폴더를 만들 수 없음"), "{reason}");
            let n = notice_ko(&outcome).unwrap();
            assert!(n.warn);
            for needle in ["보관하지 못했습니다", "업데이트는 그대로 진행합니다", NOTICE_TAIL_KO] {
                assert!(n.body.contains(needle), "{layout}: notice lacks {needle}: {}", n.body);
            }
            assert_eq!(std::fs::read(&blocker).unwrap(), b"a plain file", "the blocking file was touched");
            let rt = tokio::runtime::Runtime::new().unwrap();
            let via_wrapper = rt.block_on(run_bounded_with(input_for(&exe, &blocker), Duration::from_secs(5), capture));
            rt.shutdown_background();
            assert_eq!(std::mem::discriminant(&via_wrapper), std::mem::discriminant(&outcome), "{via_wrapper:?}");
        }
    }

    /// Nothing edited → nothing created, no notice.
    #[test]
    fn a_clean_bundle_writes_nothing_and_tells_nothing() {
        let fx = fixture();
        let c = case(&fx, "edits-leftovers-pycache-korean");
        let t = Tmp::new("clean");
        let res = t.path().join("cys.app/Contents/Resources");
        let exe = t.path().join("cys.app/Contents/MacOS/cys-app");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, b"app").unwrap();
        assert!(materialize(&c["shipped"], &res.join(RUNTIME_DIR)));
        std::fs::write(res.join(MANIFEST_BASENAME), serde_json::to_vec(&manifest_of(c)).unwrap()).unwrap();
        // leftovers and caches alone are not edits either
        std::fs::create_dir_all(res.join("runtime/python/lib/__pycache__")).unwrap();
        std::fs::write(res.join("runtime/python/lib/__pycache__/x.pyc"), b"c").unwrap();
        std::fs::write(res.join("runtime/git/bin/msys-2.0.dll.prev77"), b"old").unwrap();
        let store = t.path().join("store");
        let outcome = capture(&input_for(&exe, &store), &AtomicBool::new(false));
        assert!(matches!(outcome, CaptureOutcome::Clean { .. }), "{outcome:?}");
        assert!(!store.exists(), "a clean bundle must not create the store");
        assert_eq!(notice_ko(&outcome), None);
    }

    // ───────── the bounds (never stall the update) ─────────

    #[test]
    fn deadline_and_abandon_end_the_capture_with_a_notice() {
        let t = Tmp::new("bounds");
        let (exe, _res, _top) = fixture_install(t.path(), "mac");
        let store = t.path().join("store");
        let mut input = input_for(&exe, &store);
        input.deadline = Duration::ZERO;
        let o = capture(&input, &AtomicBool::new(false));
        assert!(matches!(&o, CaptureOutcome::Undecidable { reason, .. } if reason.contains("0초 안에 끝나지 않아")), "{o:?}");
        assert!(notice_ko(&o).unwrap().body.contains("업데이트는 그대로 진행합니다"));
        let o = capture(&input_for(&exe, &store), &AtomicBool::new(true));
        assert!(matches!(&o, CaptureOutcome::Undecidable { reason, .. } if reason.contains("늦어지지 않도록")), "{o:?}");
        assert!(!store.exists());
    }

    /// The byte cap: edits beyond it are listed with `stored: false` (still `present: true` with their
    /// scanned hash), the rest is kept, one notice.
    #[test]
    fn byte_cap_lists_what_it_did_not_copy() {
        let t = Tmp::new("cap");
        let (exe, _res, _top) = fixture_install(t.path(), "win");
        let store = t.path().join("store");
        let mut input = input_for(&exe, &store);
        input.max_bytes = 40; // index.js 20 B kept (20) · notes 46 B over (66) · site.py 30 B over (50) · 홍길동 메모 17 B kept (37)
        let o = capture(&input, &AtomicBool::new(false));
        let CaptureOutcome::Incomplete { dir: Some(dir), edits: 5, not_stored: 2, reason, .. } = &o else {
            panic!("{o:?}");
        };
        assert!(reason.contains("상한"), "{reason}");
        let index: Value = serde_json::from_slice(&std::fs::read(dir.join(INDEX_FILE)).unwrap()).unwrap();
        let found: BTreeMap<&str, bool> = index["files"].as_array().unwrap().iter().map(|f| (f["rel"].as_str().unwrap(), f["stored"].as_bool().unwrap())).collect();
        for f in index["files"].as_array().unwrap() {
            let deleted = f["rel"] == "runtime/python/lib/mod.py";
            assert_eq!(f["present"], !deleted, "{f}");
            assert_eq!(f["sha256_found"].is_null(), deleted, "{f}: a present file always carries its scanned hash, kept or not (AA5)");
        }
        assert_eq!(found["runtime/node/lib/node_modules/@user/tool/index.js"], true);
        assert_eq!(found["runtime/notes.preview.txt"], false);
        assert_eq!(found["runtime/python/lib/site.py"], false);
        assert_eq!(found["runtime/python/lib/홍길동 메모.txt"], true);
        assert!(!dir.join("runtime/python/lib/site.py").exists());
        let n = notice_ko(&o).unwrap();
        assert!(n.warn && n.body.contains("5개 가운데 2개") && n.body.contains(&dir.display().to_string()));
    }

    /// The outer timeout: a worker that does not come back in `deadline + grace` is left behind with the
    /// abandon flag set and the caller gets an outcome at once; a panicking worker is an outcome too.
    #[test]
    fn the_bounded_wrapper_always_answers() {
        let t = Tmp::new("wrapper");
        let observed = Arc::new(AtomicBool::new(false));
        let seen = Arc::clone(&observed);
        let mut input = input_for(&t.path().join("x"), &t.path().join("store"));
        input.deadline = Duration::ZERO;
        let rt = tokio::runtime::Runtime::new().unwrap();
        let started = Instant::now();
        let o = rt.block_on(run_bounded_with(input.clone(), Duration::from_millis(200), move |_, abandon| {
            let until = Instant::now() + Duration::from_secs(20);
            while Instant::now() < until {
                if abandon.load(Ordering::SeqCst) {
                    seen.store(true, Ordering::SeqCst);
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            CaptureOutcome::NotApplicable
        }));
        assert!(started.elapsed() < Duration::from_secs(5), "the wrapper waited {:?}", started.elapsed());
        assert!(matches!(&o, CaptureOutcome::Undecidable { reason, .. } if reason.contains("기다리지 않고 진행함")), "{o:?}");
        let until = Instant::now() + Duration::from_secs(5);
        while !observed.load(Ordering::SeqCst) && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(observed.load(Ordering::SeqCst), "the abandoned worker was not told to stop");
        let o = rt.block_on(run_bounded_with(input, Duration::from_secs(5), |_, _| -> CaptureOutcome { panic!("worker panic (expected by this test)") }));
        assert!(matches!(&o, CaptureOutcome::Undecidable { reason, .. } if reason.contains("비정상으로 끝남")), "{o:?}");
        rt.shutdown_background();
    }

    // ───────── where the bundle is ─────────

    #[test]
    fn locate_finds_each_install_form_and_says_when_it_cannot_compare() {
        for (layout, manifest_rel) in [("mac", "cys.app/Contents/Resources"), ("win", "inst"), ("win-res", "inst/resources")] {
            let t = Tmp::new(&format!("locate-{layout}"));
            let (exe, res, _top) = fixture_install(t.path(), layout);
            assert_eq!(
                locate(&exe),
                Located::Found(Install { manifest: t.path().join(manifest_rel).join(MANIFEST_BASENAME), runtime: res.join(RUNTIME_DIR) }),
                "{layout}"
            );
        }
        // a runtime without a manifest: undecidable (told), never "clean"
        let t = Tmp::new("locate-nomanifest");
        let exe = t.path().join("inst/cys-app.exe");
        std::fs::create_dir_all(t.path().join("inst/runtime/python")).unwrap();
        std::fs::write(&exe, b"app").unwrap();
        assert!(matches!(locate(&exe), Located::NoManifest { .. }));
        let o = capture(&input_for(&exe, &t.path().join("store")), &AtomicBool::new(false));
        assert!(matches!(&o, CaptureOutcome::Undecidable { reason, .. } if reason.contains(MANIFEST_BASENAME)), "{o:?}");
        assert!(notice_ko(&o).unwrap().warn);
        // an unreadable manifest: undecidable
        std::fs::write(t.path().join("inst").join(MANIFEST_BASENAME), b"{not json").unwrap();
        assert!(matches!(capture(&input_for(&exe, &t.path().join("store")), &AtomicBool::new(false)), CaptureOutcome::Undecidable { .. }));
        // no bundled runtime (a development build): nothing to do, nothing told
        let t = Tmp::new("locate-dev");
        let exe = t.path().join("target/debug/cys-app");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, b"app").unwrap();
        assert_eq!(locate(&exe), Located::NoRuntime);
        let o = capture(&input_for(&exe, &t.path().join("store")), &AtomicBool::new(false));
        assert_eq!(o, CaptureOutcome::NotApplicable);
        assert_eq!(notice_ko(&o), None);
        let mut unknown = input_for(&exe, &t.path().join("store"));
        unknown.exe = None;
        assert!(matches!(capture(&unknown, &AtomicBool::new(false)), CaptureOutcome::Undecidable { .. }));
    }

    /// WIN-13 analogue on this Mac: a store path with Korean characters, spaces and more than 260
    /// characters (the Windows run of the same is gap G1).
    #[test]
    fn capture_works_under_a_korean_and_long_store_path() {
        let t = Tmp::new("long");
        let (exe, res, _top) = fixture_install(t.path(), "mac");
        let mut store = t.path().join("Users");
        for _ in 0..6 {
            store = store.join("홍길동 아주 긴 사용자 이름 with spaces 0123456789");
        }
        let store = store.join(".cys").join("local").join("bundle-edits");
        let o = capture(&input_for(&exe, &store), &AtomicBool::new(false));
        let CaptureOutcome::Captured { dir, .. } = &o else { panic!("{o:?}") };
        let dst = join_rel(&dir.join(RUNTIME_DIR), "node/lib/node_modules/@user/tool/index.js");
        assert!(dst.to_string_lossy().chars().count() > 260, "{}", dst.display());
        assert_eq!(std::fs::read(&dst).unwrap(), std::fs::read(join_rel(&res.join(RUNTIME_DIR), "node/lib/node_modules/@user/tool/index.js")).unwrap());
    }

    // ───────── notice, site, store name ─────────

    #[test]
    fn every_failure_notice_says_the_update_goes_on() {
        let st = Stats::default();
        let cases = [
            CaptureOutcome::Incomplete { dir: Some(PathBuf::from("/s/d")), edits: 3, not_stored: 1, reason: "r".into(), stats: st.clone() },
            CaptureOutcome::Incomplete { dir: Some(PathBuf::from("/s/d")), edits: 3, not_stored: 0, reason: "r".into(), stats: st.clone() },
            CaptureOutcome::Incomplete { dir: None, edits: 3, not_stored: 3, reason: "r".into(), stats: st.clone() },
            CaptureOutcome::Undecidable { reason: "r".into(), stats: st.clone() },
        ];
        for o in &cases {
            let n = notice_ko(o).unwrap();
            assert!(n.warn, "{o:?}");
            assert!(n.body.contains("업데이트는 그대로 진행합니다") && n.body.contains(NOTICE_TAIL_KO) && n.body.contains("(r)"), "{}", n.body);
            assert!(!log_line(o).is_empty());
        }
        assert!(notice_ko(&cases[1]).unwrap().body.contains("목록을 남기지 못했습니다"), "an index-only failure must not claim 0 files were not kept");
        let ok = CaptureOutcome::Captured { dir: PathBuf::from("/s/d"), edits: 2, stats: st.clone() };
        let n = notice_ko(&ok).unwrap();
        assert!(!n.warn && n.body.contains("2개") && n.body.contains("/s/d") && n.body.contains("마스터"));
        assert_eq!(notice_ko(&CaptureOutcome::Clean { stats: st }), None);
        assert_eq!(notice_ko(&CaptureOutcome::NotApplicable), None);
    }

    /// Design §4-16 / limit 11: the single call is the macOS place; on Windows that line runs only with
    /// the checked launch switched off (not covered); other Unix is not shipped. The checked launch
    /// always captures.
    #[test]
    fn site_choice_follows_the_design() {
        assert!(site_applies(true, Site::SingleCall));
        assert!(!site_applies(false, Site::SingleCall));
        assert!(site_applies(true, Site::CheckedLaunch));
        assert!(site_applies(false, Site::CheckedLaunch));
    }

    #[test]
    fn store_folder_name_comes_from_the_shared_path_helper() {
        let inp = CaptureInput::for_this_app("9.9.9");
        let stamp = inp.leaf.strip_prefix(&format!("{}-", env!("CARGO_PKG_VERSION"))).expect("leaf = <running version>-<ts>");
        assert_eq!(inp.store_parent.join(&inp.leaf), cys::repair::paths::bundle_edits_dir(env!("CARGO_PKG_VERSION"), stamp));
        assert_eq!(stamp.len(), 22);
        assert!(stamp.as_bytes()[8] == b'T' && stamp.ends_with('Z') && stamp[..8].bytes().all(|b| b.is_ascii_digit()));
        assert_eq!((inp.from_version.as_str(), inp.to_version.as_str()), (env!("CARGO_PKG_VERSION"), "9.9.9"));
        assert_eq!((inp.deadline, inp.max_bytes), (CAPTURE_DEADLINE, MAX_CAPTURE_BYTES));
    }

    #[test]
    fn utc_stamp_known_answers() {
        let at = |s: u64, us: u32| utc_stamp(UNIX_EPOCH + Duration::from_secs(s) + Duration::from_micros(us as u64));
        assert_eq!(at(0, 0), "19700101T000000000000Z");
        assert_eq!(at(951_782_400, 123_456), "20000229T000000123456Z");
        assert_eq!(at(1_762_948_800, 1), "20251112T120000000001Z");
        assert_eq!(at(1_789_000_000, 999_999), "20260910T002640999999Z");
        assert_eq!(at(4_102_444_800, 0), "21000101T000000000000Z");
    }

    // ───────── source pins (C-p18-07/08, RULINGS X4) ─────────

    fn main_prod() -> &'static str {
        let src = include_str!("main.rs");
        &src[..src.find("#[cfg(test)]\nmod tests {").expect("test module boundary")]
    }

    fn fn_code(prod: &str, start: &str) -> String {
        let at = prod.find(start).unwrap_or_else(|| panic!("`{start}` gone"));
        let seg = &prod[at..at + prod[at..].find("\n}\n").expect("end of fn")];
        seg.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n")
    }

    /// ★C12 12.8 / design §4-16 (macOS place): `install_update` calls the capture exactly once, as the only statement
    /// between the Windows branch and the single download-and-install call, as a plain `.await;` statement (no `?`, no
    /// `return`, no condition — the update goes on whatever it returns), after the J2 attempt record.
    #[test]
    fn install_update_captures_right_before_the_single_call() {
        let install = fn_code(main_prod(), "async fn install_update(");
        let call_mac = "bundle_edits::capture_before_install(&app, &update.version, bundle_edits::Site::SingleCall).await;";
        assert_eq!(install.matches("capture_before_install(").count(), 1);
        let branch_end = "        return install_update_checked_windows(&app, &update, &attempt_path, verify_on).await;\n    }\n";
        let old = "    if let Err(e) = update\n        .download_and_install(";
        let (b, o) = (install.find(branch_end).expect("the Windows branch") + branch_end.len(), install.find(old).expect("the single call"));
        assert_eq!(install[b..o].trim(), call_mac, "between the Windows branch and the single download-and-install call there must be exactly the capture call");
        assert!(install.find("write_update_attempt_at(").unwrap() < b, "the J2 attempt record stays before the capture");
    }

    /// ★C12 12.8 / design §4-16 (Windows place): inside the Windows body of `install_update_checked_windows`, after the
    /// download and after the non-installer branch, the capture is the only statement before the installer file is
    /// written (so it can never come after the launch or change the attempt record) — a plain `.await;` statement.
    #[test]
    fn checked_launch_captures_before_the_installer_is_written() {
        let win = fn_code(main_prod(), "async fn install_update_checked_windows(");
        let call_win = "bundle_edits::capture_before_install(app, &update.version, bundle_edits::Site::CheckedLaunch).await;";
        assert_eq!(win.matches("capture_before_install(").count(), 1);
        let body_win = win.find("    #[cfg(windows)]\n    {\n").expect("Windows body");
        let body_other = win.find("    #[cfg(not(windows))]\n    {\n").expect("other body");
        let non_installer_end = win.find("            return Ok(());\n        }\n").expect("the non-installer branch") + "            return Ok(());\n        }\n".len();
        let write = win.find("        let file = match cys::update_launch::write_installer(").expect("write_installer");
        assert_eq!(win[non_installer_end..write].trim(), call_win, "between the non-installer branch and write_installer there must be exactly the capture call");
        let download = win.find(".download(").expect("download");
        assert!(body_win < download && download < non_installer_end && write < body_other);
    }

    /// C-p18-07/08: no other production code calls the capture — the two handler places are the only callers, so no
    /// installer route and no other update path can reach it.
    #[test]
    fn the_capture_has_no_other_caller() {
        let code: String = main_prod().lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");
        assert_eq!(code.matches("capture_before_install(").count(), 2, "the capture is called from the two handler places only");
        assert_eq!(code.matches("bundle_edits::take_pending_notices_for_ui").count(), 1, "pending notices are pulled through one command (AG3)");
        assert_eq!(code.matches("bundle_edits::").count(), 5);
        assert_eq!(code.matches("\nmod bundle_edits;\n").count(), 1);
        for other in [include_str!("feedback.rs"), include_str!("close_surface_tests.rs")] {
            assert!(!other.contains("capture_before_install"));
        }
    }

    /// ★RULINGS X4: detection is Rust-only — this module starts no process, stops none and signals
    /// none (the same words as the app's `capability_census`, over this module's production text).
    #[test]
    fn the_capture_module_starts_and_stops_no_process() {
        let src = include_str!("bundle_edits.rs");
        let prod = &src[..src.find("#[cfg(test)]\nmod tests {").expect("test module boundary")];
        let code: String = prod.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");
        let low = code.to_lowercase();
        for w in ["kill", "sigterm", "sigkill", "sigint", "sighup", "sigquit", "sigstop", "signal", "terminate", "raise("] {
            assert!(!low.contains(w), "`{w}` in the capture module");
        }
        // (`nix::` of the app census is left out: the `nix` crate is no dependency of this crate and the
        // substring also matches `std::os::unix::`, which the exec-bit arm uses.)
        // libc only for the no-follow open flags, the free-space query (AA4, AA6) and the folder handles
        // of the store: reads of a kept copy (DB1), folders / files / links / renames made through a
        // held folder (DB3) — nothing else.
        let allowed = [
            "libc::O_NOFOLLOW", "libc::O_NONBLOCK", "libc::statvfs", "libc::O_DIRECTORY", "libc::O_RDONLY", "libc::O_CLOEXEC",
            "libc::openat", "libc::readlinkat", "libc::mkdirat", "libc::symlinkat", "libc::renameat", "libc::O_WRONLY",
            "libc::O_CREAT", "libc::O_EXCL",
        ];
        assert_eq!(code.matches("libc::").count(), allowed.iter().map(|a| code.matches(a).count()).sum::<usize>(), "a libc call beyond {allowed:?}");
        for w in [
            "Command::new", ".spawn(", "process::exit", "std::process", "launchctl", "osascript", "rpc_oneshot(",
            "rpc_on(", "rpc_once(", " rpc(", "rpc_full", "conn_cell", "request(", "\"surface.", "\"ledger.", "\"system.", "\"daemon.",
            "\"channel.", "remove_file", "remove_dir",
        ] {
            assert!(!code.contains(w), "`{w}` in the capture module");
        }
    }

    /// WIN-04 hook (Parallels, by hand — never run by the suite): time the detection over a real
    /// install, five rounds. `T7A_MEASURE_EXE=<installed cys-app executable> cargo test -p cys-app
    /// --bins win04 -- --ignored --nocapture`.
    #[test]
    #[ignore = "WIN-04 measurement hook — run by hand on a real install"]
    fn win04_measure_detection_time() {
        let exe = std::env::var_os("T7A_MEASURE_EXE").map(PathBuf::from).expect("set T7A_MEASURE_EXE");
        let Located::Found(i) = locate(&exe) else { panic!("no manifest / runtime next to {}", exe.display()) };
        for round in 0..5 {
            let (flag, deadline) = unlimited();
            let mut st = Stats::default();
            let t0 = Instant::now();
            let have = walk_entries(&i.runtime, &Budget { deadline, abandon: &flag }, &mut st).expect("scan");
            let c = classify(&load_manifest(&i.manifest).expect("manifest"), &have).expect("classify");
            println!(
                "WIN-04 round {round}: {} files · {} bytes · {} ms · missing {} · added {} · changed {}",
                st.files_hashed,
                st.bytes_hashed,
                t0.elapsed().as_millis(),
                c.missing.len(),
                c.added.len(),
                c.changed.len()
            );
        }
    }

    // ───────── RULINGS AA4: free space and an identical earlier capture ─────────

    fn unmeasurable(_: &Path) -> Option<u64> {
        None
    }

    fn one_gib(_: &Path) -> Option<u64> {
        Some(1 << 30)
    }

    /// AA4: below the floor (cap + 2 GiB) — or when the free space cannot be measured — nothing is
    /// copied, the edits are listed, one notice; the update goes on.
    #[test]
    fn low_free_space_copies_nothing_and_lists_the_edits() {
        let cases: [(&str, fn(&Path) -> Option<u64>); 2] = [("below the floor (1 GiB free)", one_gib), ("unmeasurable", unmeasurable)];
        for (label, measure) in cases {
            let t = Tmp::new("lowspace");
            let (exe, _res, _top) = fixture_install(t.path(), "mac");
            let mut input = input_for(&exe, &t.path().join("store"));
            input.free_space = measure; // the production floor (cap + 2 GiB) stays
            let o = capture(&input, &AtomicBool::new(false));
            let CaptureOutcome::Incomplete { dir: Some(dir), edits: 5, not_stored: 4, reason, .. } = &o else {
                panic!("{label}: {o:?}");
            };
            assert!(reason.contains("복사하지 않고 목록만 남김"), "{label}: {reason}");
            assert_eq!(listing(dir), vec![INDEX_FILE.to_string()], "{label}: nothing copied, only the list");
            let index: Value = serde_json::from_slice(&std::fs::read(dir.join(INDEX_FILE)).unwrap()).unwrap();
            let files = index["files"].as_array().unwrap();
            assert_eq!(files.len(), 5, "{label}: every edit listed");
            assert!(files.iter().all(|f| f["stored"] == false), "{label}: {files:?}");
            let n = notice_ko(&o).unwrap();
            assert!(n.warn && n.body.contains("업데이트는 그대로 진행합니다") && n.body.contains("목록만 남김"), "{label}: {}", n.body);
        }
    }

    static SEEN: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());
    fn recording(p: &Path) -> Option<u64> {
        SEEN.lock().unwrap().push(p.to_path_buf());
        Some(1 << 50)
    }

    /// AG4: the free-space query is asked about the STORE (where the copies go), never the runtime.
    #[test]
    fn the_free_space_query_receives_the_store_path() {
        let t = Tmp::new("freepath");
        let (exe, res, _top) = fixture_install(t.path(), "mac");
        let store = t.path().join("home").join(".cys").join("local").join("bundle-edits");
        let mut input = input_for(&exe, &store);
        input.free_space = recording;
        SEEN.lock().unwrap().clear();
        assert!(matches!(capture(&input, &AtomicBool::new(false)), CaptureOutcome::Captured { .. }));
        let seen = SEEN.lock().unwrap().clone();
        assert_eq!(seen, vec![store.clone()], "asked exactly once, about the store");
        assert!(!seen[0].starts_with(res.join(RUNTIME_DIR)));
    }

    #[test]
    fn free_space_is_measured_on_the_store_volume() {
        let t = Tmp::new("free");
        let free = store_free_space(&t.path().join("not").join("created").join("yet")).expect("measured on the nearest existing folder");
        assert!(free > 0);
        assert_eq!(CaptureInput::for_this_app("9.9.9").free_floor, MAX_CAPTURE_BYTES + FREE_SPACE_MARGIN);
    }

    /// AA4: a retry of the same update (same version, same edit set) copies nothing again and points to
    /// the earlier capture; another version, a changed edit, an earlier capture that did not keep
    /// everything or whose copy is gone are never reused.
    #[test]
    fn an_identical_earlier_capture_is_not_copied_again() {
        let t = Tmp::new("dedupe");
        let (exe, res, _top) = fixture_install(t.path(), "mac");
        let store = t.path().join("store");
        let first = capture(&input_for(&exe, &store), &AtomicBool::new(false));
        let CaptureOutcome::Captured { dir: first_dir, .. } = &first else { panic!("{first:?}") };
        let with_leaf = |leaf: &str| {
            let mut i = input_for(&exe, &store);
            i.leaf = leaf.to_string();
            i
        };
        let second = capture(&with_leaf("0.15.0-20261113T020000000000Z"), &AtomicBool::new(false));
        let CaptureOutcome::AlreadyKept { dir, same_as, edits: 5, .. } = &second else { panic!("{second:?}") };
        assert_eq!(same_as, first_dir);
        assert_eq!(listing(dir), vec![INDEX_FILE.to_string()], "only the index — no second copy");
        let index: Value = serde_json::from_slice(&std::fs::read(dir.join(INDEX_FILE)).unwrap()).unwrap();
        assert_eq!(index["same_as"], LEAF);
        assert!(index["files"].as_array().unwrap().iter().all(|f| f["stored"] == false), "the bytes are in the earlier folder");
        let n = notice_ko(&second).unwrap();
        assert!(!n.warn && n.body.contains(&first_dir.display().to_string()) && n.body.contains("다시 복사하지 않았습니다"));
        // another version never points to it
        let mut other = with_leaf("0.15.1-20261113T030000000000Z");
        other.from_version = "0.15.1".to_string();
        assert!(matches!(capture(&other, &AtomicBool::new(false)), CaptureOutcome::Captured { .. }));
        // a changed edit is copied again
        std::fs::write(join_rel(&res.join(RUNTIME_DIR), "python/lib/site.py"), b"import os\n# a second edit\n").unwrap();
        let changed = capture(&with_leaf("0.15.0-20261113T040000000000Z"), &AtomicBool::new(false));
        let CaptureOutcome::Captured { dir: changed_dir, .. } = &changed else { panic!("{changed:?}") };
        // an earlier capture whose stored copy is gone is not reused (the test removes it in its own temp folder)
        std::fs::remove_file(join_rel(changed_dir, "runtime/notes.preview.txt")).unwrap();
        assert!(matches!(capture(&with_leaf("0.15.0-20261113T050000000000Z"), &AtomicBool::new(false)), CaptureOutcome::Captured { .. }));
        // an earlier capture that did not keep every edit is not reused
        let t2 = Tmp::new("dedupe-partial");
        let (exe2, _res2, _top2) = fixture_install(t2.path(), "mac");
        let store2 = t2.path().join("store");
        let mut capped = input_for(&exe2, &store2);
        capped.max_bytes = 40;
        assert!(matches!(capture(&capped, &AtomicBool::new(false)), CaptureOutcome::Incomplete { .. }));
        let mut again = input_for(&exe2, &store2);
        again.leaf = "0.15.0-20261113T060000000000Z".to_string();
        assert!(matches!(capture(&again, &AtomicBool::new(false)), CaptureOutcome::Captured { .. }));
    }

    // ───────── RULINGS AA5: unambiguous index entries ─────────

    /// AA5 for links and special files (Unix — they cannot be made on Windows here): a link is
    /// `kind: "link"` with its target and is kept as a link; a fifo is `kind: "special"`, never stored;
    /// a link replaced by a file is `kind: "file"` with its scanned hash.
    #[test]
    fn index_entries_say_kind_stored_and_link_target() {
        if !cfg!(unix) {
            return;
        }
        let fx = fixture();
        for name in ["exec-bits-and-links", "manifest-basename-and-special-files"] {
            let c = case(&fx, name);
            let t = Tmp::new("kinds");
            let exe = t.path().join("cys.app/Contents/MacOS/cys-app");
            let res = t.path().join("cys.app/Contents/Resources");
            std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
            std::fs::write(&exe, b"app").unwrap();
            assert!(materialize(&c["found"], &res.join(RUNTIME_DIR)));
            std::fs::write(res.join(MANIFEST_BASENAME), serde_json::to_vec(&manifest_of(c)).unwrap()).unwrap();
            let o = capture(&input_for(&exe, &t.path().join("store")), &AtomicBool::new(false));
            let dir = match (&o, name) {
                (CaptureOutcome::Captured { dir, .. }, "exec-bits-and-links") => dir.clone(),
                (CaptureOutcome::Incomplete { dir: Some(dir), not_stored: 2, reason, .. }, _) if reason.contains("특수 파일") => dir.clone(),
                _ => panic!("{name}: {o:?}"),
            };
            let index: Value = serde_json::from_slice(&std::fs::read(dir.join(INDEX_FILE)).unwrap()).unwrap();
            let by_rel: BTreeMap<String, Value> =
                index["files"].as_array().unwrap().iter().map(|f| (f["rel"].as_str().unwrap().to_string(), f.clone())).collect();
            let get = |rel: &str| by_rel.get(&format!("runtime/{rel}")).unwrap_or_else(|| panic!("{name}: {rel} missing")).clone();
            if name == "exec-bits-and-links" {
                for (rel, target) in [("dangling", "nowhere"), ("git/libexec/git-add", "git2")] {
                    let f = get(rel);
                    assert_eq!((f["kind"].as_str(), f["stored"].as_bool(), f["link_target"].as_str()), (Some("link"), Some(true), Some(target)), "{f}");
                    assert_eq!(f["present"], true, "a link has no hash — `present` says it exists (AG4)");
                    assert!(f["sha256_found"].is_null());
                    assert_eq!(std::fs::read_link(join_rel(&dir, &format!("runtime/{rel}"))).unwrap(), PathBuf::from(target), "kept as a link");
                }
                let npm = get("node/bin/npm");
                assert_eq!((npm["kind"].as_str(), npm["stored"].as_bool()), (Some("file"), Some(true)), "{npm}");
                assert!(npm["sha256_found"].is_string() && npm["sha256_manifest"].is_null() && npm["link_target"].is_null(), "{npm}");
                for rel in ["tools/run", "tools/data"] {
                    assert_eq!(get(rel)["kind"], "file");
                }
            } else {
                for rel in ["pipe0", "pipe1"] {
                    let f = get(rel);
                    assert_eq!((f["kind"].as_str(), f["stored"].as_bool(), f["present"].as_bool()), (Some("special"), Some(false), Some(true)), "{f}");
                    assert!(f["sha256_found"].is_null() && f["link_target"].is_null());
                }
                let nested = get("sub/runtime-manifest.json");
                assert_eq!((nested["kind"].as_str(), nested["stored"].as_bool()), (Some("file"), Some(true)));
            }
        }
    }

    // ───────── RULINGS AA6: no link is followed, reads are capped ─────────

    #[test]
    fn a_symlinked_runtime_root_is_refused() {
        if !cfg!(unix) {
            return;
        }
        let t = Tmp::new("rootlink");
        let (exe, res, _top) = fixture_install(t.path(), "mac");
        let real = t.path().join("elsewhere");
        std::fs::rename(res.join(RUNTIME_DIR), &real).unwrap();
        assert!(make_link(real.to_str().unwrap(), &res.join(RUNTIME_DIR)));
        let o = capture(&input_for(&exe, &t.path().join("store")), &AtomicBool::new(false));
        assert!(matches!(&o, CaptureOutcome::Undecidable { reason, .. } if reason.contains("링크라 따라가지 않음")), "{o:?}");
        assert!(!t.path().join("store").exists());
        assert!(notice_ko(&o).unwrap().warn);
    }

    #[test]
    fn reads_are_capped_and_never_follow_links() {
        let t = Tmp::new("capped");
        let f = t.path().join("m.json");
        std::fs::write(&f, b"0123456789").unwrap();
        assert_eq!(read_capped(&f, 10).unwrap(), b"0123456789".to_vec());
        assert!(read_capped(&f, 9).is_err(), "a file longer than the cap is refused");
        let src = include_str!("bundle_edits.rs");
        let prod = &src[..src.find("#[cfg(test)]\nmod tests {").unwrap()];
        assert!(prod.contains("parse_manifest(&read_capped(path, MAX_MANIFEST_BYTES)?)"), "the manifest goes through the capped read");
        assert!(prod.contains("let mut input = open_no_follow(src)"), "the copy source is opened without following links");
        if cfg!(unix) {
            let link = t.path().join("link.json");
            assert!(make_link(f.to_str().unwrap(), &link));
            assert!(open_no_follow(&link).is_err() && read_capped(&link, 100).is_err(), "a link is not followed");
            let (flag, deadline) = unlimited();
            let budget = Budget { deadline, abandon: &flag };
            let into = HeldDir::open_anchor(t.path()).unwrap();
            assert!(copy_exact(&link, &into, "copy", 100, &budget).1.is_err(), "a copy source that is a link is refused");
            assert!(!t.path().join("copy").exists());
            let fifo = t.path().join("fifo");
            assert!(make_fifo(&fifo, 0o644));
            assert!(open_no_follow(&fifo).is_err(), "a fifo is refused at once (O_NONBLOCK + re-check through the handle)");
        }
    }

    // ───────── RULINGS AA3 / AG1–AG3: the Windows notice, recorded inside the store, pulled by the UI ─────────

    #[test]
    fn a_pending_notice_is_written_once_and_handed_over_once() {
        let t = Tmp::new("pending");
        let store = t.path().join("local").join("bundle-edits");
        let notices = notice_dir_of(&store);
        let anchor = std::env::temp_dir();
        let n = Notice { warn: true, title: "앱 안쪽 수정 보관 실패".into(), body: "업데이트는 그대로 진행합니다".into() };
        let p = record_pending_notice(&anchor, &store, LEAF, &n).unwrap();
        assert_eq!(p, store.join(".notices").join(format!("pending-{LEAF}.json")), "inside the store (AG1)");
        let later = Notice { warn: false, title: "x".into(), body: "y".into() };
        assert!(record_pending_notice(&anchor, &store, LEAF, &later).is_err(), "write-once: an existing record is never overwritten");
        assert_eq!(take_pending_notices(&anchor, &store), vec![n.clone()]);
        assert!(take_pending_notices(&anchor, &store).is_empty(), "handed over once");
        assert!(!p.exists() && notices.join(format!("shown-{LEAF}.json")).exists(), "renamed, not deleted");
        std::fs::write(notices.join("pending-bad.json"), b"{").unwrap();
        assert!(take_pending_notices(&anchor, &store).is_empty(), "a malformed record is never handed over");
        assert!(notices.join("pending-bad.json").exists(), "and left alone");
        assert_eq!(listing(&t.path().join("local")).iter().filter(|r| !r.starts_with("bundle-edits/")).count(), 0, "nothing outside the store");
    }

    /// AG1: when the store cannot be created (C-p18-06: a plain file in its place) no record is written
    /// anywhere — nothing beside the store either.
    #[test]
    fn a_blocked_store_gets_no_notice_record_anywhere() {
        let t = Tmp::new("pending-blocked");
        let (exe, _res, _top) = fixture_install(t.path(), "win");
        let local = t.path().join("home").join(".cys").join("local");
        std::fs::create_dir_all(&local).unwrap();
        let blocker = local.join("bundle-edits");
        std::fs::write(&blocker, b"a plain file").unwrap();
        let input = input_for(&exe, &blocker);
        let n = notice_ko(&capture(&input, &AtomicBool::new(false))).unwrap();
        assert!(record_pending_notice(&input.anchor, &input.store_parent, &input.leaf, &n).is_err());
        assert_eq!(listing(&local), vec!["bundle-edits".to_string()], "only the blocking file — nothing written");
        assert_eq!(std::fs::read(&blocker).unwrap(), b"a plain file");
    }

    #[test]
    fn the_notice_records_live_inside_the_store() {
        let store = cys::repair::paths::bundle_edits_dir("0", "0");
        let store_parent = store.parent().unwrap();
        assert_eq!(notice_dir_of(store_parent), store_parent.join(".notices"));
        assert!(notice_dir_of(store_parent).starts_with(store_parent), "AG1: nothing outside bundle-edits/");
        assert!(!NOTICES_DIR.contains('-'), "never mistaken for a capture folder <version>-<ts>");
    }

    /// AG2: the record write at the Windows place has its own limit (taken out of the grace) — the whole
    /// call never waits longer than 40 s, and a stuck write does not hold the caller.
    #[test]
    fn the_notice_write_is_bounded_inside_the_40_seconds() {
        assert_eq!(CAPTURE_DEADLINE + (JOIN_GRACE - RECORD_LIMIT) + RECORD_LIMIT, Duration::from_secs(40));
        assert!(RECORD_LIMIT < JOIN_GRACE);
        let rt = tokio::runtime::Runtime::new().unwrap();
        let started = Instant::now();
        let stuck = rt.block_on(bounded_blocking(Duration::from_millis(100), || std::thread::sleep(Duration::from_secs(3))));
        assert_eq!(stuck, None);
        assert!(started.elapsed() < Duration::from_secs(2), "the caller was held {:?}", started.elapsed());
        assert_eq!(rt.block_on(bounded_blocking(Duration::from_secs(5), || 7)), Some(7));
        rt.shutdown_background();
    }

    /// AA3 / AA6 / AG2: `capture_before_install` checks the site with `site_applies(cfg!(target_os = "macos"), site)`
    /// and tells the notice exactly once — recorded under `RECORD_LIMIT` at the Windows place (whose capture
    /// grace is shortened by that limit), delivered at the macOS place.
    #[test]
    fn capture_before_install_checks_the_site_and_tells_exactly_once() {
        let src = include_str!("bundle_edits.rs");
        let prod = &src[..src.find("#[cfg(test)]\nmod tests {").unwrap()];
        let f = fn_code(prod, "pub(crate) async fn capture_before_install(");
        assert!(f.contains("    if !site_applies(cfg!(target_os = \"macos\"), site) {\n        return;\n    }"), "{f}");
        assert!(f.contains("        Site::CheckedLaunch => JOIN_GRACE - RECORD_LIMIT,\n        Site::SingleCall => JOIN_GRACE,"), "{f}");
        assert!(f.contains("    let outcome = run_bounded_with(input, grace, capture).await;"), "{f}");
        assert_eq!(f.matches("deliver_notice(").count(), 1, "{f}");
        assert_eq!(f.matches("record_pending_notice(").count(), 1, "{f}");
        assert!(
            f.contains("            match bounded_blocking(RECORD_LIMIT, move || record_pending_notice(&anchor, &store_parent, &leaf, &notice)).await {"),
            "the record write runs on the blocking pool under its own limit (AG2): {f}"
        );
        assert!(f.contains("        Site::SingleCall => deliver_notice(app, &notice),"), "{f}");
        for banned in ["loop", "for ", "while ", "std::fs::"] {
            assert!(!f.contains(banned), "`{banned}` in capture_before_install");
        }
        assert_eq!(prod.matches("deliver_notice(").count(), 2, "defined once, called by capture_before_install only");
    }

    /// AG3: `update_attempt_report` is exactly its v0.14.56 form again (its J2 pins and doc comment true), and
    /// the pending notices come through their own command, registered, which the UI calls once AFTER it
    /// registered its `update-error` listener (the UI side is pinned by `ui/src/bundleeditsnotice.test.ts`).
    #[test]
    fn the_pending_notices_have_their_own_command_and_the_attempt_report_is_untouched() {
        let prod = main_prod();
        let v0_14_56 = "#[tauri::command]\nasync fn update_attempt_report() -> Option<Value> {\n    if !update_verify_from_env(cys::env_compat(\"CYS_UPDATE_VERIFY\").as_deref()) {\n        return None;\n    }\n    tokio::task::spawn_blocking(|| {\n        update_attempt_report_at(\n            &update_attempt_path(),\n            env!(\"CARGO_PKG_VERSION\"),\n            unix_now_secs(),\n            std::env::consts::OS,\n            smart_app_control_state,\n        )\n    })\n    .await\n    .ok()\n    .flatten()\n}\n";
        assert_eq!(prod.matches(v0_14_56).count(), 1, "update_attempt_report differs from v0.14.56");
        assert!(prod.contains("#[tauri::command]\nasync fn bundle_edits_pending_notices() -> Vec<Value> {\n    tokio::task::spawn_blocking(bundle_edits::take_pending_notices_for_ui).await.unwrap_or_default()\n}\n"));
        let i = prod.find("tauri::generate_handler![").unwrap();
        let reg = &prod[i..i + prod[i..].find("\n        ])").unwrap()];
        assert_eq!(reg.lines().filter(|l| l.trim() == "bundle_edits_pending_notices,").count(), 1, "registered once");
        let ui = include_str!("../../ui/src/main.ts");
        assert_eq!(ui.matches("invoke(\"bundle_edits_pending_notices\")").count(), 1, "the UI pulls once");
        let listener = ui.find("await listen(\"update-error\"").expect("the update-error listener");
        assert!(listener < ui.find("invoke(\"bundle_edits_pending_notices\")").unwrap(), "the pull comes after the listener registration");
    }

    // ───────── the installer byte identity (RULINGS D5 · pass condition I) ─────────

    /// ★RULINGS D5 / design §4-16 / pass condition I (testplan I-WIN-01, H-16-N1-01): 0.15.0 does not change the
    /// Windows installer. `src-tauri/nsis-hooks.nsh` = git blob 6493613d… (CRLF→LF, as git's `text eol=crlf` stores it)
    /// and sha256 c3259014… of the CRLF bytes as checked out (what makensis reads) — the v0.14.56 values. To change it
    /// in a later release, change this pin on purpose (owner approval), never to get green.
    #[test]
    fn installer_hook_file_is_byte_identical_to_v0_14_56() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let nsh = std::fs::read(dir.join("nsis-hooks.nsh")).expect("nsis-hooks.nsh");
        let mut lf = Vec::with_capacity(nsh.len());
        let mut i = 0;
        while i < nsh.len() {
            if nsh[i] == b'\r' && nsh.get(i + 1) == Some(&b'\n') {
                i += 1;
                continue;
            }
            lf.push(nsh[i]);
            i += 1;
        }
        assert_eq!(git_blob_id(&lf), "6493613dad28e71c9c4dcd1a5bc38c5734e2abc3", "src-tauri/nsis-hooks.nsh differs from v0.14.56 (fatal class WIN)");
        // The bytes makensis reads: `.gitattributes` checks `*.nsh` out with CRLF on every platform. sha256 of that form at
        // v0.14.56 = c3259014… (testplan I-WIN-01 / H-16-N1-01; measured on the reference tree's checkout).
        assert_eq!(
            sha_of(&dir.join("nsis-hooks.nsh")),
            "c3259014d913e22c162d583763dcea577eede7104aa364fb5d31b4c64f9c1085",
            "src-tauri/nsis-hooks.nsh (CRLF working-tree bytes) differ from v0.14.56 (fatal class WIN)"
        );
    }

    /// ★RULINGS D5 / pass condition I (I-WIN-01, H-16-N1-01): `src-tauri/tauri.windows.conf.json` (the NSIS bundle input:
    /// install mode, hook file, resources; it names no NSIS template) = sha256 3ba81752…, the v0.14.56 value.
    #[test]
    fn windows_bundle_config_is_byte_identical_to_v0_14_56() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        assert_eq!(
            sha_of(&dir.join("tauri.windows.conf.json")),
            "3ba81752b26ee9ba34eb8e757efb0dd6dd070d5fd5d49ec516f2ce0101ec1953",
            "src-tauri/tauri.windows.conf.json differs from v0.14.56 (fatal class WIN)"
        );
    }

    /// The SHA-1 helper is proven on known answers (FIPS 180 vectors and git's empty blob).
    #[test]
    fn sha1_helper_known_answers() {
        assert_eq!(sha1_hex(b""), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(sha1_hex(b"abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(sha1_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"), "84983e441c3bd26ebaae4aa1f95129e5e54670f1");
        assert_eq!(sha1_hex(&vec![b'a'; 1_000_000]), "34aa973cd4c4daa4f61eeb2bdbad27316534016f");
        assert_eq!(git_blob_id(b""), "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391");
        assert_eq!(git_blob_id(b"hello\n"), "ce013625030ba8dba906f756967f9e9ca394464a");
    }

    fn git_blob_id(content: &[u8]) -> String {
        let mut v = format!("blob {}\0", content.len()).into_bytes();
        v.extend_from_slice(content);
        sha1_hex(&v)
    }

    /// SHA-1 (FIPS 180-4) — test-only, for git object ids; the app crate has no hash dependency of its own.
    fn sha1_hex(data: &[u8]) -> String {
        let mut h: [u32; 5] = [0x6745_2301, 0xEFCD_AB89, 0x98BA_DCFE, 0x1032_5476, 0xC3D2_E1F0];
        let mut msg = data.to_vec();
        let bits = (data.len() as u64).wrapping_mul(8);
        msg.push(0x80);
        while msg.len() % 64 != 56 {
            msg.push(0);
        }
        msg.extend_from_slice(&bits.to_be_bytes());
        for chunk in msg.chunks(64) {
            let mut w = [0u32; 80];
            for t in 0..16 {
                w[t] = u32::from_be_bytes([chunk[4 * t], chunk[4 * t + 1], chunk[4 * t + 2], chunk[4 * t + 3]]);
            }
            for t in 16..80 {
                w[t] = (w[t - 3] ^ w[t - 8] ^ w[t - 14] ^ w[t - 16]).rotate_left(1);
            }
            let (mut a, mut b, mut c, mut d, mut e) = (h[0], h[1], h[2], h[3], h[4]);
            for (t, wt) in w.iter().enumerate() {
                let (f, k) = match t {
                    0..=19 => ((b & c) | (!b & d), 0x5A82_7999),
                    20..=39 => (b ^ c ^ d, 0x6ED9_EBA1),
                    40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
                    _ => (b ^ c ^ d, 0xCA62_C1D6),
                };
                let tmp = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(*wt);
                e = d;
                d = c;
                c = b.rotate_left(30);
                b = a;
                a = tmp;
            }
            for (x, y) in h.iter_mut().zip([a, b, c, d, e]) {
                *x = x.wrapping_add(y);
            }
        }
        h.iter().map(|x| format!("{x:08x}")).collect()
    }

    // ───────── the streaming SHA-256 the copies are hashed with (DB1, DB2) ─────────

    fn sha256_hex_of(bytes: &[u8]) -> String {
        let mut h = Sha256::new();
        h.update(bytes);
        h.finish_hex()
    }

    /// FIPS 180-4 / NIST CAVP example vectors, and the same digests as the library's
    /// `TargetState::read` (the `sha2` crate) for every length 0..=300 fed in uneven pieces.
    #[test]
    fn the_streaming_sha256_matches_the_standard_and_the_library() {
        assert_eq!(sha256_hex_of(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(sha256_hex_of(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(
            sha256_hex_of(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        let mut million = Sha256::new();
        for _ in 0..1000 {
            million.update(&[b'a'; 1000]);
        }
        assert_eq!(million.finish_hex(), "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0");
        let t = Tmp::new("sha256");
        for len in 0..=300usize {
            let bytes: Vec<u8> = (0..len).map(|i| (i * 31 + len) as u8).collect();
            let p = t.path().join(format!("f{len}"));
            std::fs::write(&p, &bytes).unwrap();
            let Ok(TargetState::File { sha256, .. }) = TargetState::read(&p) else { panic!("read {len}") };
            let mut h = Sha256::new();
            for piece in bytes.chunks(1 + len % 70) {
                h.update(piece);
            }
            assert_eq!(h.finish_hex(), sha256, "length {len}");
        }
    }

    // ───────── RULINGS DB1: a kept copy is reused only after it is verified ─────────

    /// The kept copies (`kind: "file"`, stored) listed by a capture's index.
    fn kept_files(dir: &Path) -> Vec<String> {
        let index: Value = serde_json::from_slice(&std::fs::read(dir.join(INDEX_FILE)).unwrap()).unwrap();
        index["files"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|f| f["kind"] == "file" && f["stored"] == true)
            .map(|f| f["rel"].as_str().unwrap().to_string())
            .collect()
    }

    /// DB1 / verdict J8 B1: an earlier capture whose kept copy was changed after it was written — same
    /// length, truncated, replaced by a folder, replaced by a link to the live file — is never reused:
    /// a new capture is made and its copy is byte for byte the file of the bundle, and the user is not
    /// told "already kept".
    #[test]
    fn db1_a_changed_kept_copy_is_never_reused() {
        let mut tampers = vec!["same-length", "truncated", "folder"];
        if cfg!(unix) {
            tampers.push("link-to-the-live-file");
        }
        for tamper in tampers {
            let t = Tmp::new(&format!("db1-{tamper}"));
            let (exe, res, _top) = fixture_install(t.path(), "mac");
            let store = t.path().join("store");
            let first = capture(&input_for(&exe, &store), &AtomicBool::new(false));
            let CaptureOutcome::Captured { dir: first_dir, .. } = &first else { panic!("{tamper}: {first:?}") };
            let rel = "runtime/python/lib/site.py";
            assert!(kept_files(first_dir).contains(&rel.to_string()), "{tamper}: {:?}", kept_files(first_dir));
            let kept = join_rel(first_dir, rel);
            let live = join_rel(&res, rel);
            let original = std::fs::read(&kept).unwrap();
            assert_eq!(original, std::fs::read(&live).unwrap());
            match tamper {
                "same-length" => {
                    let mut changed = original.clone();
                    changed[0] ^= 0x20;
                    std::fs::write(&kept, &changed).unwrap();
                    assert_eq!(std::fs::metadata(&kept).unwrap().len(), original.len() as u64);
                }
                "truncated" => std::fs::OpenOptions::new().write(true).open(&kept).unwrap().set_len(0).unwrap(),
                "folder" => {
                    std::fs::remove_file(&kept).unwrap();
                    std::fs::create_dir(&kept).unwrap();
                }
                _ => {
                    std::fs::remove_file(&kept).unwrap();
                    assert!(make_link(&live.to_string_lossy(), &kept), "a link whose target has the indexed content");
                }
            }
            let mut again = input_for(&exe, &store);
            again.leaf = "0.15.0-20261113T020000000000Z".to_string();
            let second = capture(&again, &AtomicBool::new(false));
            let CaptureOutcome::Captured { dir, .. } = &second else { panic!("{tamper}: a changed kept copy was reused: {second:?}") };
            assert_ne!(dir, first_dir);
            assert_eq!(std::fs::read(join_rel(dir, rel)).unwrap(), std::fs::read(&live).unwrap(), "{tamper}: a fresh exact copy");
            let n = notice_ko(&second).unwrap();
            assert!(!n.body.contains("다시 복사하지 않았습니다"), "{tamper}: {}", n.body);
        }
    }

    /// DB1 for a kept link (Unix — the app keeps no links on Windows): a kept link whose target was
    /// changed is never reused; an untouched one still is.
    #[test]
    fn db1_a_kept_link_with_another_target_is_never_reused() {
        if !cfg!(unix) {
            return;
        }
        let fx = fixture();
        let c = case(&fx, "exec-bits-and-links");
        let t = Tmp::new("db1-link");
        let exe = t.path().join("cys.app/Contents/MacOS/cys-app");
        let res = t.path().join("cys.app/Contents/Resources");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, b"app").unwrap();
        assert!(materialize(&c["found"], &res.join(RUNTIME_DIR)));
        std::fs::write(res.join(MANIFEST_BASENAME), serde_json::to_vec(&manifest_of(c)).unwrap()).unwrap();
        let store = t.path().join("store");
        let first = capture(&input_for(&exe, &store), &AtomicBool::new(false));
        let CaptureOutcome::Captured { dir: first_dir, .. } = &first else { panic!("{first:?}") };
        let with_leaf = |leaf: &str| {
            let mut i = input_for(&exe, &store);
            i.leaf = leaf.to_string();
            i
        };
        let untouched = capture(&with_leaf("0.15.0-20261113T020000000000Z"), &AtomicBool::new(false));
        assert!(matches!(&untouched, CaptureOutcome::AlreadyKept { same_as, .. } if same_as == first_dir), "{untouched:?}");
        let kept = join_rel(first_dir, "runtime/dangling");
        assert_eq!(std::fs::read_link(&kept).unwrap(), PathBuf::from("nowhere"));
        std::fs::remove_file(&kept).unwrap();
        assert!(make_link("elsewhere", &kept));
        let changed = capture(&with_leaf("0.15.0-20261113T030000000000Z"), &AtomicBool::new(false));
        let CaptureOutcome::Captured { dir, .. } = &changed else { panic!("a kept link with another target was reused: {changed:?}") };
        assert_eq!(std::fs::read_link(join_rel(dir, "runtime/dangling")).unwrap(), PathBuf::from("nowhere"));
    }

    // ───────── RULINGS DB2: the limits hold for the bytes actually copied ─────────

    /// DB2: one byte is read past the scanned length to see whether the file grew — it is never
    /// written; the copy stops there.
    #[test]
    fn db2_the_byte_read_to_detect_growth_is_never_written() {
        let (flag, deadline) = unlimited();
        let budget = Budget { deadline, abandon: &flag };
        let mut out = Vec::new();
        let (written, result) = copy_stream(&mut &b"12345"[..], &mut out, 4, &budget);
        assert_eq!(out, b"1234".to_vec(), "only the scanned length is written");
        assert_eq!((written, result), (4, Err(CopyStop::Grew)));
        let mut out = Vec::new();
        let (written, result) = copy_stream(&mut &b"1234"[..], &mut out, 4, &budget);
        assert_eq!((out.as_slice(), written), (&b"1234"[..], 4));
        assert_eq!(result, Ok(sha256_hex_of(b"1234")));
    }

    /// The test's reader: sets the abandon flag (the outer timeout) during its first read.
    struct FlagOnFirstRead<'a> {
        flag: &'a AtomicBool,
        left: usize,
    }
    impl std::io::Read for FlagOnFirstRead<'_> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.flag.store(true, Ordering::SeqCst);
            let n = buf.len().min(self.left);
            buf[..n].fill(0xAB);
            self.left -= n;
            Ok(n)
        }
    }

    /// DB2 / DP1: the deadline and the abandon flag are checked after a read returns and before any
    /// write — the outer timeout setting the flag during a read means nothing of that chunk is written
    /// (was, before DP1: `(COPY_CHUNK as u64, COPY_CHUNK), "one chunk, then the flag stops the copy"`),
    /// and past the deadline nothing is.
    #[test]
    fn db2_no_chunk_is_written_after_the_timeout() {
        let flag = AtomicBool::new(false);
        let budget = Budget { deadline: Instant::now() + Duration::from_secs(3_600), abandon: &flag };
        let total = 8 * COPY_CHUNK;
        let mut out = Vec::new();
        let (written, result) = copy_stream(&mut FlagOnFirstRead { flag: &flag, left: total }, &mut out, total as u64, &budget);
        assert_eq!(result, Err(CopyStop::Halted(Halt::Abandoned)));
        assert_eq!((written, out.len()), (0, 0), "the flag set during the read stops the copy before its write");
        let calm = AtomicBool::new(false);
        let expired = Budget { deadline: Instant::now(), abandon: &calm };
        let mut out = Vec::new();
        let (written, result) = copy_stream(&mut &vec![1u8; total][..], &mut out, total as u64, &expired);
        assert_eq!((written, out.len(), result), (0, 0, Err(CopyStop::Halted(Halt::Deadline))));
    }

    /// What the test's writer does on one call.
    #[derive(Clone, Copy)]
    enum WriteStep {
        Take,
        TakeThenAbandon,
        InterruptThenAbandon,
        Fail,
    }

    /// The test's writer: takes at most `per_call` bytes per call and follows `steps` call by call
    /// (`Take` after the script ends); counts its calls.
    struct ScriptedWriter<'a> {
        flag: &'a AtomicBool,
        per_call: usize,
        steps: Vec<WriteStep>,
        calls: usize,
        out: Vec<u8>,
    }
    impl std::io::Write for ScriptedWriter<'_> {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            let step = self.steps.get(self.calls).copied().unwrap_or(WriteStep::Take);
            self.calls += 1;
            match step {
                WriteStep::Fail => Err(std::io::Error::other("the disk is full (test)")),
                WriteStep::InterruptThenAbandon => {
                    self.flag.store(true, Ordering::SeqCst);
                    Err(std::io::ErrorKind::Interrupted.into())
                }
                WriteStep::Take | WriteStep::TakeThenAbandon => {
                    let n = buf.len().min(self.per_call);
                    self.out.extend_from_slice(&buf[..n]);
                    if matches!(step, WriteStep::TakeThenAbandon) {
                        self.flag.store(true, Ordering::SeqCst);
                    }
                    Ok(n)
                }
            }
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// DP1: between the partial writes and the `Interrupted` retries of one chunk the budget is checked
    /// again — the flag set during a partial write or an interrupted write means no further write call.
    #[test]
    fn dp1_no_write_call_follows_the_flag_inside_a_chunk() {
        let data = vec![7u8; 2 * COPY_CHUNK];
        for (steps, kept) in [(vec![WriteStep::TakeThenAbandon], 1000usize), (vec![WriteStep::InterruptThenAbandon], 0)] {
            let flag = AtomicBool::new(false);
            let budget = Budget { deadline: Instant::now() + Duration::from_secs(3_600), abandon: &flag };
            let mut w = ScriptedWriter { flag: &flag, per_call: 1000, steps, calls: 0, out: Vec::new() };
            let (written, result) = copy_stream(&mut &data[..], &mut w, data.len() as u64, &budget);
            assert_eq!(result, Err(CopyStop::Halted(Halt::Abandoned)));
            assert_eq!(w.calls, 1, "no write call after the flag");
            assert_eq!((written, w.out.len()), (kept as u64, kept));
        }
    }

    /// DP1: the counter adds only what a write call returned — a partial write then an error reports
    /// exactly the bytes really written.
    #[test]
    fn dp1_the_counter_is_the_bytes_really_written() {
        let data = vec![9u8; 2 * COPY_CHUNK];
        let (flag, deadline) = unlimited();
        let budget = Budget { deadline, abandon: &flag };
        let mut w = ScriptedWriter { flag: &flag, per_call: 4096, steps: vec![WriteStep::Take, WriteStep::Fail], calls: 0, out: Vec::new() };
        let (written, result) = copy_stream(&mut &data[..], &mut w, data.len() as u64, &budget);
        assert!(matches!(&result, Err(CopyStop::Io(e)) if e.contains("the disk is full")), "{result:?}");
        assert_eq!((written, w.out.len(), w.calls), (4096, 4096, 2));
    }

    /// The test's reader: on its first call it lets the budget run out — sets the abandon flag, or waits
    /// until the deadline has passed — and returns `Interrupted`; later calls return data. Counts calls.
    struct InterruptedFirstRead<'a> {
        flag: Option<&'a AtomicBool>,
        until: Option<Instant>,
        calls: usize,
    }
    impl std::io::Read for InterruptedFirstRead<'_> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.calls += 1;
            if self.calls > 1 {
                buf.fill(5);
                return Ok(buf.len());
            }
            if let Some(flag) = self.flag {
                flag.store(true, Ordering::SeqCst);
            }
            if let Some(until) = self.until {
                while Instant::now() < until {
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
            Err(std::io::ErrorKind::Interrupted.into())
        }
    }

    /// DW1 (a): the retry of an interrupted read is a new read call and stands behind the same budget
    /// check — the flag set during the first read means no second read and no write.
    #[test]
    fn dw1_no_read_retry_after_the_flag() {
        let flag = AtomicBool::new(false);
        let budget = Budget { deadline: Instant::now() + Duration::from_secs(3_600), abandon: &flag };
        let mut r = InterruptedFirstRead { flag: Some(&flag), until: None, calls: 0 };
        let mut w = ScriptedWriter { flag: &flag, per_call: COPY_CHUNK, steps: Vec::new(), calls: 0, out: Vec::new() };
        let (written, result) = copy_stream(&mut r, &mut w, 1000, &budget);
        assert_eq!((r.calls, w.calls, written), (1, 0, 0), "read calls, write calls, bytes written");
        assert_eq!(result, Err(CopyStop::Halted(Halt::Abandoned)));
    }

    /// DW1 (b): the same when the deadline passes during the first read.
    #[test]
    fn dw1_no_read_retry_after_the_deadline() {
        let calm = AtomicBool::new(false);
        let deadline = Instant::now() + Duration::from_millis(20);
        let budget = Budget { deadline, abandon: &calm };
        let mut r = InterruptedFirstRead { flag: None, until: Some(deadline), calls: 0 };
        let mut w = ScriptedWriter { flag: &calm, per_call: COPY_CHUNK, steps: Vec::new(), calls: 0, out: Vec::new() };
        let (written, result) = copy_stream(&mut r, &mut w, 1000, &budget);
        assert_eq!((r.calls, w.calls, written), (1, 0, 0), "read calls, write calls, bytes written");
        assert_eq!(result, Err(CopyStop::Halted(Halt::Deadline)));
    }

    /// Between the scan and the copies the edited `site.py` grows by 3 MiB.
    fn grow_site_by_3_mib(runtime: &Path) {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().append(true).open(join_rel(runtime, "python/lib/site.py")).unwrap();
        f.write_all(&vec![b'#'; 3 << 20]).unwrap();
    }

    /// DB2 / verdict J8 B2: a file that grew between the scan and the copy cannot push the store past
    /// the capture cap — the store holds no more than the cap, the grown file is not kept, the user is
    /// told, and the capture still answers.
    #[test]
    fn db2_a_file_that_grew_after_the_scan_cannot_fill_the_store() {
        let t = Tmp::new("db2-grew");
        let (exe, _res, _top) = fixture_install(t.path(), "mac");
        let mut input = input_for(&exe, &t.path().join("store"));
        input.max_bytes = 1 << 20;
        input.between_scan_and_copy = grow_site_by_3_mib;
        let o = capture(&input, &AtomicBool::new(false));
        let CaptureOutcome::Incomplete { dir: Some(dir), not_stored: 1, reason, .. } = &o else { panic!("{o:?}") };
        let stored: u64 = listing(dir)
            .iter()
            .filter(|r| r.starts_with("runtime/"))
            .map(|r| std::fs::symlink_metadata(join_rel(dir, r)).unwrap().len())
            .sum();
        assert!(stored <= input.max_bytes, "the store holds {stored} bytes, more than the cap {}", input.max_bytes);
        assert!(reason.contains("커져서"), "{reason}");
        assert!(!kept_files(dir).contains(&"runtime/python/lib/site.py".to_string()));
        assert!(notice_ko(&o).unwrap().body.contains("업데이트는 그대로 진행합니다"));
    }

    // ───────── RULINGS DB3: the store's own path is held without following links ─────────

    /// DB3 / verdict J8 M1: a store folder that is a link into the app bundle's `Resources/runtime` is
    /// refused — nothing is written inside the bundle, the update goes on and the reason is told; a
    /// notice record is not written there either.
    #[test]
    fn db3_a_store_linked_into_the_bundle_gets_nothing_written() {
        if !cfg!(unix) {
            return;
        }
        let t = Tmp::new("db3-store");
        let (exe, res, top) = fixture_install(t.path(), "mac");
        let local = t.path().join("home").join(".cys").join("local");
        std::fs::create_dir_all(&local).unwrap();
        let store = local.join("bundle-edits");
        assert!(make_link(&res.join(RUNTIME_DIR).to_string_lossy(), &store));
        let before = snapshot(&top);
        let input = input_for(&exe, &store);
        let o = capture(&input, &AtomicBool::new(false));
        let CaptureOutcome::Incomplete { dir: None, reason, .. } = &o else { panic!("{o:?}") };
        assert!(reason.contains("링크"), "{reason}");
        let n = notice_ko(&o).unwrap();
        assert!(n.warn && n.body.contains("업데이트는 그대로 진행합니다"), "{}", n.body);
        assert!(record_pending_notice(&input.anchor, &store, &input.leaf, &n).is_err());
        assert_eq!(snapshot(&top), before, "nothing written inside the app bundle");
    }

    /// DB3: an ancestor of the store (`local`) that is a link anywhere is refused the same way.
    #[test]
    fn db3_a_linked_ancestor_of_the_store_gets_nothing_written() {
        if !cfg!(unix) {
            return;
        }
        let t = Tmp::new("db3-ancestor");
        let (exe, _res, _top) = fixture_install(t.path(), "win");
        let cys_dir = t.path().join("home").join(".cys");
        std::fs::create_dir_all(&cys_dir).unwrap();
        let elsewhere = t.path().join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        assert!(make_link(&elsewhere.to_string_lossy(), &cys_dir.join("local")));
        let store = cys_dir.join("local").join("bundle-edits");
        let input = input_for(&exe, &store);
        let o = capture(&input, &AtomicBool::new(false));
        assert!(matches!(&o, CaptureOutcome::Incomplete { dir: None, reason, .. } if reason.contains("링크")), "{o:?}");
        let n = notice_ko(&o).unwrap();
        assert!(record_pending_notice(&input.anchor, &store, &input.leaf, &n).is_err());
        assert!(listing(&elsewhere).is_empty(), "written through the link: {:?}", listing(&elsewhere));
        assert!(std::fs::read_dir(&elsewhere).unwrap().next().is_none(), "no folder made through the link");
    }

    /// DB3: a `.notices` folder that is a link is refused both ways — no record is written through it,
    /// and a record planted behind it is neither handed over nor renamed.
    #[test]
    fn db3_a_linked_notice_folder_is_refused_both_ways() {
        if !cfg!(unix) {
            return;
        }
        let t = Tmp::new("db3-notices");
        let store = t.path().join("home").join(".cys").join("local").join("bundle-edits");
        std::fs::create_dir_all(&store).unwrap();
        let elsewhere = t.path().join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        assert!(make_link(&elsewhere.to_string_lossy(), &notice_dir_of(&store)));
        let anchor = std::env::temp_dir();
        let n = Notice { warn: true, title: "앱 안쪽 수정 보관 실패".into(), body: "업데이트는 그대로 진행합니다".into() };
        assert!(record_pending_notice(&anchor, &store, LEAF, &n).is_err());
        assert!(listing(&elsewhere).is_empty(), "written through the link: {:?}", listing(&elsewhere));
        let planted = elsewhere.join(format!("pending-{LEAF}.json"));
        std::fs::write(&planted, br#"{"schema":1,"leaf":"x","warn":true,"title":"t","body":"b"}"#).unwrap();
        assert!(take_pending_notices(&anchor, &store).is_empty(), "nothing is taken through a link");
        assert!(planted.exists(), "the record behind the link is not renamed");
    }
}
