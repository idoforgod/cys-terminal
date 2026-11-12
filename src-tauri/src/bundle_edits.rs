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

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
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
/// Cooperative deadline of one capture (scan + copy), checked before every file. T7a choice (the
/// design gives no number): long enough for a cold scan of the shipped runtime, short enough that
/// the update never looks stuck. The real Windows duration is the WIN-04 measurement.
pub(crate) const CAPTURE_DEADLINE: Duration = Duration::from_secs(30);
/// Extra wait of the outer timeout on top of the deadline; past it the update goes on without the
/// worker (which stops at its next file through the abandon flag).
pub(crate) const JOIN_GRACE: Duration = Duration::from_secs(10);
/// Most bytes one capture copies (T7a choice): edits beyond it are listed, not copied.
pub(crate) const MAX_CAPTURE_BYTES: u64 = 256 * 1024 * 1024;

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
pub(crate) fn load_manifest(path: &Path) -> Result<Value, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    parse_manifest(&bytes)
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
}

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
        }
    }
}

/// The result of one capture. `edits` counts changed + added + deleted files after the filter;
/// `not_stored` counts edits whose bytes could not be kept (deleted files have none to keep).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CaptureOutcome {
    /// No bundled runtime (development build) — nothing to do, no notice.
    NotApplicable,
    /// Compared, nothing edited — no notice, nothing written.
    Clean { stats: Stats },
    /// Every edit kept and the index written.
    Captured { dir: PathBuf, edits: usize, stats: Stats },
    /// Edits found but not all of them (or not the index) could be kept.
    Incomplete { dir: Option<PathBuf>, edits: usize, not_stored: usize, reason: String, stats: Stats },
    /// Could not tell whether anything was edited.
    Undecidable { reason: String, stats: Stats },
}

/// One record of `index.json` (O-C12-3).
#[derive(Debug, Clone, PartialEq, Eq)]
struct IndexRecord {
    rel: String,
    sha256_manifest: Option<String>,
    sha256_found: Option<String>,
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

/// Creates the capture folder with an exclusive create (`-<n>` appended on a collision, as O-C2-1
/// does for vault slots), after creating the store parent.
fn create_capture_dir(store_parent: &Path, leaf: &str) -> Result<PathBuf, String> {
    std::fs::create_dir_all(store_parent).map_err(|e| format!("보관 폴더를 만들 수 없음: {}: {e}", store_parent.display()))?;
    for n in 0..100u32 {
        let name = if n == 0 { leaf.to_string() } else { format!("{leaf}-{n}") };
        let dir = store_parent.join(name);
        match std::fs::create_dir(&dir) {
            Ok(()) => return Ok(dir),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("보관 폴더를 만들 수 없음: {}: {e}", dir.display())),
        }
    }
    Err(format!("보관 폴더 이름이 모두 쓰이는 중: {}", store_parent.join(leaf).display()))
}

/// `<dir>/runtime/<rel>` — refuses a component that could leave the folder.
fn store_path(dir: &Path, rel: &str) -> Option<PathBuf> {
    let mut p = dir.join(RUNTIME_DIR);
    for c in rel.split('/') {
        if c.is_empty() || c == "." || c == ".." || c.contains('\\') || c.contains(':') {
            return None;
        }
        p.push(c);
    }
    Some(p)
}

/// Copies one regular file byte for byte into a NEW file and returns the sha256 of the stored
/// copy. Refuses a source that is no longer a regular file.
fn copy_exact(src: &Path, dst: &Path) -> Result<String, String> {
    use cys::repair::types::TargetState;
    let meta = std::fs::symlink_metadata(src).map_err(|e| format!("{}: {e}", src.display()))?;
    if !meta.file_type().is_file() {
        return Err(format!("{}: 더 이상 일반 파일이 아님", src.display()));
    }
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let mut input = std::fs::File::open(src).map_err(|e| format!("{}: {e}", src.display()))?;
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dst)
        .map_err(|e| format!("{}: {e}", dst.display()))?;
    std::io::copy(&mut input, &mut output).map_err(|e| format!("{}: {e}", dst.display()))?;
    output.sync_all().map_err(|e| format!("{}: {e}", dst.display()))?;
    drop(output);
    let _ = std::fs::set_permissions(dst, meta.permissions());
    match TargetState::read(dst) {
        Ok(TargetState::File { sha256, .. }) => Ok(sha256),
        Ok(_) => Err(format!("{}: 복사본이 일반 파일이 아님", dst.display())),
        Err(e) => Err(format!("{}: {e:?}", dst.display())),
    }
}

/// Keeps a link as a link (Unix). Windows does not store links (making one needs a privilege the
/// app does not ask for) — the edit is listed and counted as not stored.
fn store_link(target: &str, dst: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::os::unix::fs::symlink(target, dst).map_err(|e| format!("{}: {e}", dst.display()))
    }
    #[cfg(not(unix))]
    {
        let _ = (target, dst);
        Err("윈도우에서는 링크를 보관하지 않음".to_string())
    }
}

/// Writes `index.json` last, through a temporary name and a rename.
fn write_index(dir: &Path, input: &CaptureInput, records: &[IndexRecord]) -> Result<(), String> {
    let files: Vec<Value> = records
        .iter()
        .map(|r| json!({"rel": r.rel, "sha256_manifest": r.sha256_manifest, "sha256_found": r.sha256_found}))
        .collect();
    let index = json!({
        "schema": INDEX_SCHEMA,
        "from_version": input.from_version,
        "to_version": input.to_version,
        "files": files,
    });
    let mut bytes = serde_json::to_vec_pretty(&index).map_err(|e| e.to_string())?;
    bytes.push(b'\n');
    let tmp = dir.join(format!("{INDEX_FILE}.partial"));
    let fin = dir.join(INDEX_FILE);
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::io::Write::write_all(&mut f, &bytes).map_err(|e| format!("{}: {e}", tmp.display()))?;
    f.sync_all().map_err(|e| format!("{}: {e}", tmp.display()))?;
    drop(f);
    std::fs::rename(&tmp, &fin).map_err(|e| format!("{}: {e}", fin.display()))
}

fn halt_reason_ko(h: Halt, deadline: Duration) -> String {
    match h {
        Halt::Deadline => format!("{}초 안에 끝나지 않아 멈춤", deadline.as_secs()),
        Halt::Abandoned => "업데이트를 기다리게 하지 않으려고 멈춤".to_string(),
    }
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
    let want = manifest_entries(&manifest).unwrap_or_default();
    let mut edits: Vec<(String, bool)> = Vec::new(); // (rel, deleted)
    for rel in classes.changed.iter().chain(classes.added.iter()) {
        if !not_an_edit(rel) {
            edits.push((rel.clone(), false));
        }
    }
    for rel in &classes.missing {
        if !not_an_edit(rel) {
            edits.push((rel.clone(), true));
        }
    }
    edits.sort();
    if edits.is_empty() {
        elapsed(&mut stats);
        return CaptureOutcome::Clean { stats };
    }
    let storable = edits.iter().filter(|(_, deleted)| !*deleted).count();
    let dir = match create_capture_dir(&input.store_parent, &input.leaf) {
        Ok(d) => d,
        Err(reason) => {
            elapsed(&mut stats);
            return CaptureOutcome::Incomplete { dir: None, edits: edits.len(), not_stored: storable, reason, stats };
        }
    };

    let mut records = Vec::with_capacity(edits.len());
    let mut not_stored = 0usize;
    let mut first_reason: Option<String> = None;
    let mut copied: u64 = 0;
    let mut halted: Option<Halt> = None;
    for (rel, deleted) in &edits {
        let sha256_manifest = want
            .get(rel)
            .and_then(Value::as_object)
            .filter(|o| o.get("t").and_then(Value::as_str) == Some("f"))
            .and_then(|o| o.get("sha256"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let bundle_rel = format!("{RUNTIME_DIR}/{rel}");
        let mut record = IndexRecord { rel: bundle_rel, sha256_manifest, sha256_found: None };
        if *deleted {
            records.push(record);
            continue;
        }
        if halted.is_none() {
            halted = budget.check().err();
        }
        let result: Result<Option<String>, String> = if let Some(h) = halted {
            Err(halt_reason_ko(h, input.deadline))
        } else {
            match (have.get(rel), store_path(&dir, rel)) {
                (_, None) => Err(format!("보관할 수 없는 경로: {rel}")),
                (None, _) => Err(format!("검사 결과에 없는 경로: {rel}")),
                (Some(found), Some(dst)) => match &found.entry {
                    Entry::File { .. } => {
                        if copied.saturating_add(found.len) > input.max_bytes {
                            Err(format!("한 번에 보관하는 크기 상한({} MiB)을 넘음", input.max_bytes / (1024 * 1024)))
                        } else {
                            copy_exact(&found.path, &dst).map(|sha| {
                                copied = copied.saturating_add(found.len);
                                Some(sha)
                            })
                        }
                    }
                    Entry::Link { target } => store_link(target, &dst).map(|()| None),
                    Entry::Other { .. } => Err(format!("특수 파일은 보관하지 않음: {rel}")),
                },
            }
        };
        match result {
            Ok(sha) => record.sha256_found = sha,
            Err(reason) => {
                not_stored += 1;
                first_reason.get_or_insert(reason);
            }
        }
        records.push(record);
    }
    let index = write_index(&dir, input, &records);
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
    pub title: &'static str,
    pub body: String,
}

/// The notice of an outcome — `None` when there is nothing to tell (no bundled runtime, nothing
/// edited). Every failure says that the update goes on.
pub(crate) fn notice_ko(o: &CaptureOutcome) -> Option<Notice> {
    match o {
        CaptureOutcome::NotApplicable | CaptureOutcome::Clean { .. } => None,
        CaptureOutcome::Captured { dir, edits, .. } => Some(Notice {
            warn: false,
            title: "앱 안쪽 수정 보관",
            body: format!(
                "업데이트 전에 앱 안쪽에서 바뀐 파일 {edits}개(고침·추가·삭제)를 찾아, 남아 있는 파일은 {} 에 복사하고 \
                 전부를 목록({INDEX_FILE})에 적었습니다. 업데이트는 그대로 진행합니다. 새 버전에 다시 입히려면 마스터에게 \
                 맡기세요 — 워커가 확인한 뒤 다시 입힙니다.",
                dir.display()
            ),
        }),
        CaptureOutcome::Incomplete { dir: Some(dir), edits, not_stored: 0, reason, .. } => Some(Notice {
            warn: true,
            title: "앱 안쪽 수정 보관 실패",
            body: format!(
                "업데이트 전에 앱 안쪽에서 바뀐 파일 {edits}개를 {} 에 복사했지만 목록을 남기지 못했습니다({reason}). \
                 업데이트는 그대로 진행합니다. {NOTICE_TAIL_KO}",
                dir.display()
            ),
        }),
        CaptureOutcome::Incomplete { dir: Some(dir), edits, not_stored, reason, .. } => Some(Notice {
            warn: true,
            title: "앱 안쪽 수정 보관 실패",
            body: format!(
                "앱 안쪽에서 바뀐 파일 {edits}개 가운데 {not_stored}개를 업데이트 전에 보관하지 못했습니다({reason}). \
                 보관한 것은 {} 에 있습니다(목록: {INDEX_FILE}). 업데이트는 그대로 진행합니다. {NOTICE_TAIL_KO}",
                dir.display()
            ),
        }),
        CaptureOutcome::Incomplete { dir: None, edits, reason, .. } => Some(Notice {
            warn: true,
            title: "앱 안쪽 수정 보관 실패",
            body: format!(
                "앱 안쪽에서 바뀐 파일 {edits}개를 업데이트 전에 보관하지 못했습니다({reason}). \
                 업데이트는 그대로 진행합니다. {NOTICE_TAIL_KO}"
            ),
        }),
        CaptureOutcome::Undecidable { reason, .. } => Some(Notice {
            warn: true,
            title: "앱 안쪽 수정 확인 실패",
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
        CaptureOutcome::Incomplete { dir, edits, not_stored, reason, stats } => format!(
            "bundle-edits: {not_stored} of {edits} edit(s) not kept ({reason}) · folder {} · {}",
            dir.as_ref().map(|d| d.display().to_string()).unwrap_or_else(|| "-".to_string()),
            st(stats)
        ),
        CaptureOutcome::Undecidable { reason, stats } => format!("bundle-edits: undecidable ({reason}) · {}", st(stats)),
    }
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

/// Tells the outcome: one stderr line; for a warning the existing in-app `update-error` toast
/// ("업데이트 경고"); for every notice an OS notification through the notification plugin already
/// registered by the app (it outlives the app, which closes right after the installer starts on
/// Windows). Each channel is best effort and cannot fail the update.
fn deliver(app: &tauri::AppHandle, outcome: &CaptureOutcome) {
    eprintln!("[cys-app] {}", log_line(outcome));
    let Some(n) = notice_ko(outcome) else {
        return;
    };
    if n.warn {
        let _ = app.emit("update-error", n.body.clone());
    }
    if let Some(plugin) = app.try_state::<tauri_plugin_notification::Notification<tauri::Wry>>() {
        let _ = plugin.builder().title(n.title).body(n.body.clone()).show();
    }
}

/// THE call of the update handler (both places). Returns when the capture is done, cut off or
/// failed — never an error: the update goes on in every case.
pub(crate) async fn capture_before_install(app: &tauri::AppHandle, to_version: &str, site: Site) {
    if !site_applies(cfg!(target_os = "macos"), site) {
        return;
    }
    let outcome = run_bounded_with(CaptureInput::for_this_app(to_version), JOIN_GRACE, capture).await;
    deliver(app, &outcome);
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
        }
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
        assert_eq!(index.as_object().unwrap().len(), 4, "index.json = {{schema, from_version, to_version, files}}");
        let files = index["files"].as_array().unwrap();
        let rels: Vec<&str> = files.iter().map(|f| f["rel"].as_str().unwrap()).collect();
        assert_eq!(
            rels,
            ["runtime/node/lib/node_modules/@user/tool/index.js", "runtime/notes.preview.txt", "runtime/python/lib/mod.py", "runtime/python/lib/site.py", "runtime/python/lib/홍길동 메모.txt"]
        );
        let man = &c["manifest_entries"];
        for f in files {
            assert_eq!(f.as_object().unwrap().len(), 3, "{f}: {{rel, sha256_manifest, sha256_found}}");
            let rel = f["rel"].as_str().unwrap().strip_prefix("runtime/").unwrap();
            let expect_manifest = man.get(rel).map(|e| e["sha256"].clone()).unwrap_or(Value::Null);
            assert_eq!(f["sha256_manifest"], expect_manifest, "{rel}");
            let src = join_rel(&res.join(RUNTIME_DIR), rel);
            let expect_found = if src.exists() { Value::String(sha_of(&src)) } else { Value::Null };
            assert_eq!(f["sha256_found"], expect_found, "{rel}");
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
        assert!(matches!(&o, CaptureOutcome::Undecidable { reason, .. } if reason.contains("기다리게 하지 않으려고")), "{o:?}");
        assert!(!store.exists());
    }

    /// The byte cap: edits beyond it are listed with `sha256_found: null`, the rest is kept, one notice.
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
        let found: BTreeMap<&str, bool> = index["files"].as_array().unwrap().iter().map(|f| (f["rel"].as_str().unwrap(), !f["sha256_found"].is_null())).collect();
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
        assert_eq!(code.matches("bundle_edits::").count(), 4);
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
        for w in [
            "Command::new", ".spawn(", "process::exit", "std::process", "libc::", "launchctl", "osascript", "rpc_oneshot(",
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
}
