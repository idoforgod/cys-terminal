//! Defect intake without queue traffic (design §4-4 · C1 1.17 · O-C1-9 · RULINGS G4, S2) — track T3.
//!
//! A report is DATA, never authority, and never a message: it becomes (or updates) ONE row per
//! (lane, signature) at `repairs/intake/<lane_key>/<signature>.json`, written only by that lane's
//! daemon; rows of the same signature from several lanes are aggregated when READ. Nothing here
//! sends anything to a queue, an alert route or the event bus — the master sees at most one digest
//! line per five-minute report gate (`javis_report_gate.py`). The intake has its OWN quota (new
//! signatures per lane per hour), never the general alert quota (`alert_route.rs` 63–125: a flood
//! there delayed seat-death alerts by up to 1,805 s).
//! The signature (O-C1-9) = sha256(lane-independent rel path NUL error class NUL first stable line).

use super::types::{LaneKey, RepairError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;

/// The intake's own quota: new signature rows one lane's daemon writes per rolling hour. Further
/// new signatures in that hour are counted (shown by `status`), not written. Repeats of a known
/// signature only update its row and are never limited (aggregation is the point).
pub const INTAKE_NEW_SIGNATURES_PER_HOUR: usize = 200;
/// Longest accepted `--file` (bytes).
pub const MAX_REL_BYTES: usize = 512;
/// Longest accepted `--error` text (bytes); longer text is cut (only its first line is used).
pub const MAX_ERROR_BYTES: usize = 8 * 1024;
/// Longest kept first line (characters).
pub const MAX_LINE_CHARS: usize = 200;
/// Distinct reporter labels kept per row.
pub const MAX_REPORTERS: usize = 8;

/// One intake row (per lane, per signature). Field order = JSON key order (`atomic_write_json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IntakeRow {
    pub schema: u32,
    pub signature: String,
    pub lane: LaneKey,
    pub rel: String,
    pub error_class: String,
    pub first_line: String,
    pub first_seen: f64,
    pub last_seen: f64,
    pub count: u64,
    pub reporters: Vec<String>,
}

/// A pack-relative path in its lane-independent spelling: `/` separators, no `.` components, no
/// empty components. Refused: empty, absolute (incl. `C:` / `\\`), any `..`, NUL, over-long.
pub fn normalize_rel(raw: &str) -> Result<String, RepairError> {
    let bad = |why: &str| {
        RepairError::Precondition(format!("팩 기준 상대 경로가 아닙니다({why}): {raw:?}"))
    };
    if raw.is_empty() {
        return Err(bad("비어 있음"));
    }
    if raw.len() > MAX_REL_BYTES {
        return Err(bad("너무 김"));
    }
    if raw.contains('\0') {
        return Err(bad("NUL"));
    }
    let s = raw.replace('\\', "/");
    if s.starts_with('/') || s.as_bytes().get(1) == Some(&b':') {
        return Err(bad("절대 경로"));
    }
    let mut parts = Vec::new();
    for c in s.split('/') {
        match c {
            "" | "." => continue,
            ".." => return Err(bad("..")),
            // RULINGS AW3 (a): a Windows short name (`ROLE-C~1.SH`, `JAVIS_~1.PY`) is another
            // spelling of a long name — refused on every system (a string rule).
            other if is_short_name_component(other) => return Err(bad("윈도우 짧은 이름(~숫자)")),
            other => parts.push(other),
        }
    }
    if parts.is_empty() {
        return Err(bad("비어 있음"));
    }
    Ok(parts.join("/"))
}

/// RULINGS JV1 (WW1): the name the file system gives an EXISTING general target of a repair, when it
/// differs from the typed rel (the output of [`normalize_rel`]) by letter case alone — so the records
/// (a repair's files, its tombstones, a defect report) do not hold a case alias of an existing general
/// file, and the readers that match a recorded rel against the vendor's rel by exact string (the
/// install's tombstone look-up, the preflight's `tombstoned_file`, `cys repair tombstones --json`) meet
/// the file's real name. First way: the directory entries, one component at a time from the pack root —
/// an entry spelled exactly as typed is kept, otherwise the ONLY entry equal to the typed component under
/// `crate::pack::list_key`'s fold (one definition) is taken; the same on every system (no `\\?\` prefix,
/// no reliance on how a canonical path reports case), and a case alias of a folder is found too. Second
/// way, only when a folder on the way cannot be LISTED (RULINGS KH3 — a failed listing is not absence):
/// the canonical path of `pack/rel` against the canonical pack dir (the stored case on macOS and Windows),
/// with no link anywhere on the typed chain. Either way the name is taken only when the difference is
/// letter case alone under the fold and both spellings are the same file on this volume (volume + file
/// id). The typed rel is kept EXACTLY as today when: the rel's fold is on a compiled list (RULINGS KH1 —
/// a listed file's canonical name is the LIST's; the classification refuses aliases of it); the target
/// or a component is absent; a component is a link (a junction on Windows reports as one); two entries
/// fold to the typed component (a volume that distinguishes case); the difference is more than case
/// (e.g. a Unicode normal form); the identities differ; or neither way gives a name (a pack folder that
/// can be neither listed nor resolved — a broken installation). Nothing is refused here.
///
/// RULINGS KY1 (WW2), the rule in one sentence: a general file THIS binary ships is recorded under the
/// embedded template's rel (`crate::pack::PACK_ALL`) when the typed rel folds to exactly that one shipped
/// rel and names the same file on this volume, any other existing general file under the name its
/// directory entries give it, and a listed, absent, linked or more-than-case-different target under the
/// typed spelling. (Two shipped rels that fold alike — none in the template today — keep the typed
/// spelling.)
pub fn on_disk_rel(pack: &Path, rel: &str) -> String {
    on_disk_case_alias(pack, rel, &list_names).unwrap_or_else(|| rel.to_string())
}

/// A folder's names, or None when the listing could not be opened or read to its END (RULINGS KM1).
type Lister<'a> = dyn Fn(&Path) -> Option<Vec<String>> + 'a;

/// The product's [`Lister`]: `read_dir` through [`names_of`].
fn list_names(dir: &Path) -> Option<Vec<String>> {
    names_of(
        std::fs::read_dir(dir)
            .ok()?
            .map(|e| e.map(|e| e.file_name().into_string().ok())),
    )
}

/// RULINGS KM1: the names of a listing read to its end; an error from the entry iterator makes the
/// listing INCOMPLETE (None) — never "no such name". A name that is not UTF-8 is skipped (`Ok(None)`).
fn names_of(entries: impl Iterator<Item = std::io::Result<Option<String>>>) -> Option<Vec<String>> {
    let mut names = Vec::new();
    for e in entries {
        if let Some(n) = e.ok()? {
            names.push(n);
        }
    }
    Some(names)
}

/// [`on_disk_rel`]'s answer when it differs from the typed rel (None = keep the typed rel); `list` reads
/// a folder's names (the product passes [`list_names`]).
fn on_disk_case_alias(pack: &Path, rel: &str, list: &Lister<'_>) -> Option<String> {
    if crate::pack::listed_in_compiled_lists(rel) {
        return None;
    }
    // KY1: a general file this binary ships is named by the template — the vendor's spelling is kept as
    // typed (no listing), another spelling of the same file on this volume becomes the vendor's rel;
    // otherwise (another file, an absent target, a link) WW1's rule below decides.
    match shipped_rels_folding(rel, crate::pack::PACK_ALL.iter().map(|(r, _)| *r)).as_slice() {
        [] => {}
        [vendor] if *vendor == rel => return None,
        [vendor] => {
            if same_file_no_link(pack, rel, vendor) {
                return Some(vendor.to_string());
            }
        }
        _ => return None,
    }
    let real = match spelled_by_listing(pack, rel, list) {
        Spelled::Name(n) => n,
        Spelled::NoName => return None,
        Spelled::Unlisted => spelled_by_canonical_path(pack, rel)?,
    };
    if real == rel || crate::pack::list_key(&real) != crate::pack::list_key(rel) {
        return None;
    }
    let typed_id = super::boot_critical::file_identity(&pack.join(rel))?;
    (super::boot_critical::file_identity(&pack.join(&real))? == typed_id).then_some(real)
}

/// RULINGS KY1: the shipped rels that `crate::pack::list_key` folds to the typed rel's key.
fn shipped_rels_folding<'a>(rel: &str, shipped: impl Iterator<Item = &'a str>) -> Vec<&'a str> {
    let key = crate::pack::list_key(rel);
    shipped.filter(|r| crate::pack::list_key(r) == key).collect()
}

/// RULINGS KY1: `pack/a` and `pack/b` are the same file on this volume (volume + file id), with no link
/// on either chain inside the pack (no listing — one `lstat` per component).
fn same_file_no_link(pack: &Path, a: &str, b: &str) -> bool {
    let no_link = |rel: &str| {
        let mut p = pack.to_path_buf();
        rel.split('/').all(|c| {
            p = p.join(c);
            std::fs::symlink_metadata(&p).is_ok_and(|m| !m.file_type().is_symlink())
        })
    };
    no_link(a)
        && no_link(b)
        && super::boot_critical::file_identity(&pack.join(a))
            .is_some_and(|x| super::boot_critical::file_identity(&pack.join(b)) == Some(x))
}

/// What the directory entries say about a typed rel.
enum Spelled {
    /// Every component found (exactly or as the one fold match), none a link.
    Name(String),
    /// A component absent, ambiguous, or a link.
    NoName,
    /// A folder on the way could not be listed.
    Unlisted,
}

/// The first way of [`on_disk_rel`]: the directory entries, component by component. Only a listing read
/// to its end may answer "no such name"; one that could not be opened or read to its end is `Unlisted`.
fn spelled_by_listing(pack: &Path, rel: &str, list: &Lister<'_>) -> Spelled {
    let mut dir = pack.to_path_buf();
    let mut real: Vec<String> = Vec::new();
    for typed in rel.split('/') {
        let Some(names) = list(&dir) else {
            return Spelled::Unlisted;
        };
        let Some(name) = entry_spelling(typed, &names) else {
            return Spelled::NoName;
        };
        dir = dir.join(&name);
        if !std::fs::symlink_metadata(&dir).is_ok_and(|m| !m.file_type().is_symlink()) {
            return Spelled::NoName;
        }
        real.push(name);
    }
    Spelled::Name(real.join("/"))
}

/// The second way of [`on_disk_rel`] (RULINGS KH3), needing no listing: the canonical path of `pack/rel`
/// relative to the canonical pack dir — only with no link on the typed chain and the same number of
/// components inside the pack.
fn spelled_by_canonical_path(pack: &Path, rel: &str) -> Option<String> {
    let mut p = pack.to_path_buf();
    for c in rel.split('/') {
        p = p.join(c);
        if std::fs::symlink_metadata(&p).ok()?.file_type().is_symlink() {
            return None;
        }
    }
    let base = std::fs::canonicalize(pack).ok()?;
    let full = std::fs::canonicalize(pack.join(rel)).ok()?;
    let parts = full
        .strip_prefix(&base)
        .ok()?
        .components()
        .map(|c| match c {
            std::path::Component::Normal(s) => s.to_str(),
            _ => None,
        })
        .collect::<Option<Vec<&str>>>()?;
    (parts.len() == rel.split('/').count()).then(|| parts.join("/"))
}

/// One path component: the entry spelled exactly as typed, else the ONLY entry that `list_key` folds
/// to the same key; None = absent or ambiguous.
fn entry_spelling(typed: &str, names: &[String]) -> Option<String> {
    if names.iter().any(|n| n == typed) {
        return Some(typed.to_string());
    }
    let key = crate::pack::list_key(typed);
    let mut folded = names.iter().filter(|n| crate::pack::list_key(n) == key);
    let only = folded.next()?;
    folded.next().is_none().then(|| only.clone())
}

/// RULINGS AW3 (a): a path component in the Windows 8.3 short-name form — a `~` directly followed
/// by an ASCII digit anywhere in the component.
pub fn is_short_name_component(c: &str) -> bool {
    let b = c.as_bytes();
    b.windows(2).any(|w| w[0] == b'~' && w[1].is_ascii_digit())
}

/// The first non-empty line of an error text with its volatile parts masked, so the same defect
/// gives the same line on every run: absolute temporary paths → `<tmp>`, long hex runs (≥ 8) →
/// `<hex>`, ISO-like timestamps → `<time>`, numbers of 5+ digits (pids, ports, epochs) → `<n>`.
/// Small numbers (line numbers, exit codes) are kept — they tell defects apart.
pub fn first_stable_line(error: &str) -> String {
    let cut = &error[..floor_char_boundary(error, MAX_ERROR_BYTES)];
    let line = cut
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    let mut out = String::with_capacity(line.len());
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        // temporary absolute paths: /tmp/…, /private/tmp/…, /var/folders/… up to whitespace/quote
        let rest: String = chars[i..].iter().take(16).collect();
        if (i == 0 || !chars[i - 1].is_alphanumeric())
            && (rest.starts_with("/tmp/")
                || rest.starts_with("/private/")
                || rest.starts_with("/var/folders/"))
        {
            out.push_str("<tmp>");
            while i < chars.len()
                && !chars[i].is_whitespace()
                && !matches!(chars[i], '\'' | '"' | ')' | ',' | ':')
            {
                i += 1;
            }
            continue;
        }
        // long hex runs (addresses, hashes, ids) — a whole word of ≥ 8 hex digits with a digit in it
        if chars[i].is_ascii_hexdigit() && (i == 0 || !chars[i - 1].is_alphanumeric()) {
            let mut j = i;
            while j < chars.len() && chars[j].is_ascii_hexdigit() {
                j += 1;
            }
            let boundary = j == chars.len() || !chars[j].is_alphanumeric();
            if j - i >= 8 && boundary && chars[i..j].iter().any(|c| c.is_ascii_digit()) {
                out.push_str("<hex>");
                i = j;
                continue;
            }
        }
        if chars[i].is_ascii_digit() {
            let start = i;
            while i < chars.len()
                && (chars[i].is_ascii_digit() || matches!(chars[i], '-' | ':' | 'T' | '.'))
            {
                i += 1;
            }
            let run: String = chars[start..i].iter().collect();
            let digits = run.chars().filter(|c| c.is_ascii_digit()).count();
            if run.contains('-') && run.contains(':') && digits >= 8 {
                out.push_str("<time>");
            } else {
                // split back into numbers and separators, masking long numbers only
                let mut num = String::new();
                for c in run.chars() {
                    if c.is_ascii_digit() {
                        num.push(c);
                    } else {
                        push_num(&mut out, &num);
                        num.clear();
                        out.push(c);
                    }
                }
                push_num(&mut out, &num);
            }
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out.chars().take(MAX_LINE_CHARS).collect()
}

fn push_num(out: &mut String, num: &str) {
    if num.len() >= 5 {
        out.push_str("<n>");
    } else {
        out.push_str(num);
    }
}

fn floor_char_boundary(s: &str, max: usize) -> usize {
    if s.len() <= max {
        return s.len();
    }
    let mut i = max;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// The error class: the leading identifier of the first stable line when it ends in `:` (as
/// `SyntaxError: …`, `json.decoder.JSONDecodeError: …`, `cys: …`), else `error`.
pub fn error_class(first_line: &str) -> String {
    if let Some((head, _)) = first_line.split_once(':') {
        let head = head.trim();
        let ok = !head.is_empty()
            && head.len() <= 64
            && head.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
            && head
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'));
        if ok {
            return head.to_string();
        }
    }
    "error".to_string()
}

/// The ONE normaliser of a signature's error line (O-C1-9 "first stable line", RULINGS BF3) —
/// `repair.report` ([`parse_report`]) and the daemon's repeated-failure detector both go through it,
/// so one failure gives one signature whichever source files it. For a multi-line text: a leading
/// `Exit code N` line (Claude Code's wrapper of a failed shell call) is skipped; a Python traceback
/// gives its LAST exception line (the last unindented line after the last
/// `Traceback (most recent call last):`); otherwise the first non-empty line is taken. Then the
/// volatile parts are masked ([`first_stable_line`]). A single-line text is masked exactly as before.
/// The whole text is read (a traceback ends a long output); callers bound its size (`report`: 32 KiB).
pub fn stable_error_line(error: &str) -> String {
    let lines: Vec<&str> = error.lines().collect();
    let Some(first) = lines.iter().position(|l| !l.trim().is_empty()) else {
        return first_stable_line(error);
    };
    let mut rest = &lines[first..];
    if leading_exit_code(rest[0]).is_some() && rest[1..].iter().any(|l| !l.trim().is_empty()) {
        rest = &rest[1..];
    }
    if let Some(j) = rest
        .iter()
        .rposition(|l| l.trim_start().starts_with("Traceback (most recent call last)"))
    {
        if let Some(exc) = rest[j + 1..]
            .iter()
            .rev()
            .find(|l| !l.trim().is_empty() && !l.starts_with(char::is_whitespace))
        {
            return first_stable_line(exc);
        }
    }
    first_stable_line(&rest.join("\n"))
}

/// O-C1-9: `sha256(rel NUL class NUL line)` in lowercase hex.
pub fn signature(rel: &str, class: &str, line: &str) -> String {
    let mut h = Sha256::new();
    h.update(rel.as_bytes());
    h.update([0u8]);
    h.update(class.as_bytes());
    h.update([0u8]);
    h.update(line.as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Where an entry of the exclusion list applies (RULINGS AD8 — track T3b owns the list).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExclusionScope {
    /// `cys repair report` AND the daemon's repeated-failure detector.
    All,
    /// Only the detector: the entry needs what only an observed call carries (the subcommand the
    /// command line gave the script, the exit code at the head of the failure text). The detector's
    /// own entries live beside it (`cysd` `alerts.rs`, `DETECTOR_EXCLUSIONS`).
    Detector,
}

/// How an entry recognises its by-design failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExclusionRule {
    /// Windows: `cys-dept` is not on a pane's PATH, so the department check always ends 127 /
    /// "command not found" / "not recognized" (precedent `javis_teamtoken.py` 470–483).
    DeptNotOnPath,
    /// A verdict tool whose non-zero exit IS its designed answer ("not ready", "start refused"),
    /// not a defect — on POSITIVE evidence only (RULINGS BF2): `rel` (and, when `sub` is given, that
    /// first argument) with a failure text whose first line is exactly `Exit code N` AND that carries
    /// the tool's own fixed verdict marker for that N (one of the marker groups listed with it; a group
    /// matches when every one of its strings is in the text), and NEVER when the text shows an
    /// interpreter or launch failure ([`shows_launch_failure`]).
    DesignedAnswer {
        rel: &'static str,
        sub: Option<&'static str>,
        answers: &'static [DesignedExit],
    },
}

/// One designed answer of a verdict tool: its exit code and its verdict markers (alternatives; each
/// an AND-list of fixed strings the tool prints with that answer).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DesignedExit {
    pub exit: i64,
    pub markers: &'static [&'static [&'static str]],
}

/// Texts that show an interpreter or launch failure — a designed-answer entry never excludes them
/// (RULINGS BF2): a syntax error, a crash, a missing script or a missing command is exactly the
/// post-update breakage the intake exists for.
pub const LAUNCH_FAILURE_MARKERS: &[&str] = &[
    "SyntaxError",
    "IndentationError",
    "Traceback",
    "can't open file",
    "No such file or directory",
    "command not found",
    "not recognized",
];

/// Whether a failure text shows an interpreter or launch failure ([`LAUNCH_FAILURE_MARKERS`]).
pub fn shows_launch_failure(error: &str) -> bool {
    LAUNCH_FAILURE_MARKERS.iter().any(|m| error.contains(m))
}

/// One entry of the exclusion list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Exclusion {
    pub reason: &'static str,
    pub windows_only: bool,
    pub scope: ExclusionScope,
    pub rule: ExclusionRule,
}

impl Exclusion {
    /// Whether this entry covers a failure of `rel` (first argument `sub`, when known) with `error`.
    pub fn matches(&self, rel: &str, sub: Option<&str>, error: &str, windows: bool) -> bool {
        if self.windows_only && !windows {
            return false;
        }
        match self.rule {
            ExclusionRule::DeptNotOnPath => {
                // RULINGS BF2: `Exit code 127` as the FIRST line (a call of cys-dept), or the
                // "command not found" / "not recognized" sentence that names cys-dept — never a
                // "127" anywhere in the text.
                let low = error.to_ascii_lowercase();
                let first_127 = leading_exit_code(error) == Some(127)
                    && (rel.contains("cys-dept") || low.contains("cys-dept"));
                let sentence = low.lines().any(|l| {
                    l.contains("cys-dept")
                        && (l.contains("command not found") || l.contains("not recognized"))
                });
                first_127 || sentence
            }
            ExclusionRule::DesignedAnswer {
                rel: r,
                sub: want_sub,
                answers,
            } => {
                let Some(code) = leading_exit_code(error) else {
                    return false;
                };
                r == rel
                    && want_sub.is_none_or(|w| sub == Some(w))
                    && !shows_launch_failure(error)
                    && answers.iter().any(|a| {
                        a.exit == code
                            && a.markers
                                .iter()
                                .any(|group| group.iter().all(|m| error.contains(m)))
                    })
            }
        }
    }
}

/// The exclusion list's entries for BOTH intake sources — operations that fail BY DESIGN and must
/// not become intake (design §4-4, §6-4 (4)): without them a by-design failure files one row per
/// prompt per seat. The detector adds its own detector-scope entries (the designed answers of verdict
/// tools, which need the observed call) and consults this table first.
pub const EXCLUSIONS: &[Exclusion] = &[
    // precedent javis_teamtoken.py 470–483 (T3's entry, unchanged behaviour)
    Exclusion {
        reason: "windows-cys-dept-not-on-path",
        windows_only: true,
        scope: ExclusionScope::All,
        rule: ExclusionRule::DeptNotOnPath,
    },
];

/// The exit code a failure text announces on its first non-empty line, when that line is exactly
/// `Exit code N` (the shape Claude Code gives a failed shell call: "Exit code 1\nError: …").
pub fn leading_exit_code(error: &str) -> Option<i64> {
    let first = error.lines().map(str::trim).find(|l| !l.is_empty())?;
    first
        .strip_prefix("Exit code ")
        .and_then(|n| n.trim().parse::<i64>().ok())
}

/// The first entry of `table` that covers the failure (every scope). Returns its reason.
pub fn excluded_in(
    table: &[Exclusion],
    rel: &str,
    sub: Option<&str>,
    error: &str,
    windows: bool,
) -> Option<&'static str> {
    table
        .iter()
        .find(|e| e.matches(rel, sub, error, windows))
        .map(|e| e.reason)
}

/// Operations that fail BY DESIGN and must not become intake — for `cys repair report` (the entries
/// of [`EXCLUSIONS`] with scope [`ExclusionScope::All`]; a manual report carries no observed call).
/// Returns the reason when excluded.
pub fn excluded(rel: &str, error: &str, windows: bool) -> Option<&'static str> {
    EXCLUSIONS
        .iter()
        .filter(|e| e.scope == ExclusionScope::All)
        .find(|e| e.matches(rel, None, error, windows))
        .map(|e| e.reason)
}

/// What one report did.
#[derive(Debug, Clone, PartialEq)]
pub enum Recorded {
    /// A new row was written.
    New(IntakeRow),
    /// An existing row of this lane was updated (count + 1).
    Updated(IntakeRow),
    /// A new signature over this lane's hourly quota — not written.
    OverQuota { signature: String },
    /// Excluded by design (see [`excluded`]).
    Excluded { reason: &'static str },
}

/// A parsed report before it touches the disk.
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    pub rel: String,
    pub error_class: String,
    pub first_line: String,
    pub signature: String,
}

/// Validates and normalises one report.
pub fn parse_report(file: &str, error: &str) -> Result<Report, RepairError> {
    let rel = normalize_rel(file)?;
    if error.trim().is_empty() {
        return Err(RepairError::Precondition(
            "오류 내용이 비어 있습니다".into(),
        ));
    }
    let first_line = stable_error_line(error);
    let class = error_class(&first_line);
    let sig = signature(&rel, &class, &first_line);
    Ok(Report {
        rel,
        error_class: class,
        first_line,
        signature: sig,
    })
}

/// `<root>/intake/<lane>/<sig>.json` (the layout of `paths::intake_path`, relative to a root).
pub fn row_path_in(root: &Path, lane: &LaneKey, sig: &str) -> std::path::PathBuf {
    root.join("intake")
        .join(&lane.0)
        .join(format!("{sig}.json"))
}

/// Reads one row: `Ok(None)` when absent, `Err(Corrupt)` when unreadable or of a higher schema.
pub fn load_row(path: &Path) -> Result<Option<IntakeRow>, RepairError> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(RepairError::Corrupt(format!("{}: {e}", path.display()))),
    };
    let v: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|e| RepairError::Corrupt(format!("{}: {e}", path.display())))?;
    if v.get("schema").and_then(|s| s.as_u64()).unwrap_or(u64::MAX)
        > u64::from(super::REPAIR_SCHEMA)
    {
        return Err(RepairError::Corrupt(format!(
            "{}: unknown schema",
            path.display()
        )));
    }
    serde_json::from_value(v)
        .map(Some)
        .map_err(|e| RepairError::Corrupt(format!("{}: {e}", path.display())))
}

#[allow(clippy::too_many_arguments)] // one call site (the daemon); every argument is a distinct input
/// Records one report in `lane`'s row. `new_in_last_hour` = how many NEW rows this lane wrote in
/// the last hour (the caller keeps that count in memory — the quota is per daemon). The caller
/// serialises calls for one signature (one writer per file).
pub fn record_in(
    root: &Path,
    lane: &LaneKey,
    report: &Report,
    reporter: &str,
    now: f64,
    new_in_last_hour: usize,
    windows: bool,
    raw_error: &str,
) -> Result<Recorded, RepairError> {
    if let Some(reason) = excluded(&report.rel, raw_error, windows) {
        return Ok(Recorded::Excluded { reason });
    }
    let path = row_path_in(root, lane, &report.signature);
    let existing = load_row(&path)?;
    let (row, fresh) = match existing {
        Some(mut row) => {
            row.count = row.count.saturating_add(1);
            row.last_seen = now;
            if !row.reporters.iter().any(|r| r == reporter) && row.reporters.len() < MAX_REPORTERS {
                row.reporters.push(reporter.to_string());
            }
            (row, false)
        }
        None => {
            if new_in_last_hour >= INTAKE_NEW_SIGNATURES_PER_HOUR {
                return Ok(Recorded::OverQuota {
                    signature: report.signature.clone(),
                });
            }
            (
                IntakeRow {
                    schema: super::REPAIR_SCHEMA,
                    signature: report.signature.clone(),
                    lane: lane.clone(),
                    rel: report.rel.clone(),
                    error_class: report.error_class.clone(),
                    first_line: report.first_line.clone(),
                    first_seen: now,
                    last_seen: now,
                    count: 1,
                    reporters: vec![reporter.to_string()],
                },
                true,
            )
        }
    };
    crate::atomic_write_json(&path, &row)
        .map_err(|e| RepairError::Io(format!("{}: {e}", path.display())))?;
    Ok(if fresh {
        Recorded::New(row)
    } else {
        Recorded::Updated(row)
    })
}

/// One aggregated defect (all lanes' rows of one signature).
#[derive(Debug, Clone, PartialEq)]
pub struct Defect {
    pub signature: String,
    pub rel: String,
    pub error_class: String,
    pub first_line: String,
    pub count: u64,
    pub first_seen: f64,
    pub last_seen: f64,
    pub lanes: Vec<LaneKey>,
}

/// Every lane's rows aggregated by signature (G4), sorted by signature; unreadable rows are
/// returned separately (listed, never fatal — F4).
pub fn load_all_in(root: &Path) -> (Vec<Defect>, Vec<String>) {
    let mut by_sig: std::collections::BTreeMap<String, Defect> = std::collections::BTreeMap::new();
    let mut broken = Vec::new();
    let base = root.join("intake");
    let Ok(lanes) = std::fs::read_dir(&base) else {
        return (Vec::new(), broken);
    };
    let mut lane_dirs: Vec<_> = lanes.filter_map(Result::ok).map(|e| e.path()).collect();
    lane_dirs.sort();
    for dir in lane_dirs {
        let Ok(rows) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut files: Vec<_> = rows.filter_map(Result::ok).map(|e| e.path()).collect();
        files.sort();
        for f in files {
            if f.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            match load_row(&f) {
                Ok(Some(row)) => {
                    let d = by_sig
                        .entry(row.signature.clone())
                        .or_insert_with(|| Defect {
                            signature: row.signature.clone(),
                            rel: row.rel.clone(),
                            error_class: row.error_class.clone(),
                            first_line: row.first_line.clone(),
                            count: 0,
                            first_seen: row.first_seen,
                            last_seen: row.last_seen,
                            lanes: Vec::new(),
                        });
                    d.count = d.count.saturating_add(row.count);
                    d.first_seen = d.first_seen.min(row.first_seen);
                    d.last_seen = d.last_seen.max(row.last_seen);
                    if !d.lanes.contains(&row.lane) {
                        d.lanes.push(row.lane);
                    }
                }
                Ok(None) => {}
                Err(e) => broken.push(match e {
                    RepairError::Corrupt(m) | RepairError::Io(m) => m,
                    other => format!("{other:?}"),
                }),
            }
        }
    }
    (by_sig.into_values().collect(), broken)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: u32) -> std::path::PathBuf {
        let d =
            std::env::temp_dir().join(format!("cys-repair-intake-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn rel_normalisation() {
        assert_eq!(normalize_rel("bin/a.py").unwrap(), "bin/a.py");
        assert_eq!(normalize_rel("./bin//a.py").unwrap(), "bin/a.py");
        assert_eq!(normalize_rel("hooks\\x.sh").unwrap(), "hooks/x.sh");
        for bad in [
            "",
            "/etc/passwd",
            "C:/x",
            "\\\\srv\\x",
            "../x",
            "bin/../../x",
            "a\0b",
            ".",
            "./",
        ] {
            assert!(normalize_rel(bad).is_err(), "{bad:?} accepted");
        }
        assert!(normalize_rel(&"a".repeat(MAX_REL_BYTES + 1)).is_err());
    }

    /// WW1 (RULINGS JV1): does the volume that holds `dir` give one file to two spellings that differ
    /// by letter case? Measured (written through one spelling, read through the other).
    fn case_blind(dir: &std::path::Path) -> bool {
        let _ = std::fs::write(dir.join("ww1-probe.txt"), b"ww1\n");
        let same = std::fs::read(dir.join("WW1-PROBE.TXT")).is_ok_and(|b| b == b"ww1\n");
        let _ = std::fs::remove_file(dir.join("ww1-probe.txt"));
        same
    }

    /// WW1 (RULINGS JV1): the component rule alone (no file system) — exact first; else the ONE entry
    /// that folds to it; two entries that fold to it (a case-sensitive volume holding both) → none;
    /// a difference beyond case (a Unicode normal form) → none.
    #[test]
    fn ww1_entry_spelling_rule() {
        let names = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(entry_spelling("a.py", &names(&["a.py", "b.py"])).as_deref(), Some("a.py"));
        assert_eq!(entry_spelling("A.PY", &names(&["a.py", "b.py"])).as_deref(), Some("a.py"));
        assert_eq!(entry_spelling("A.py", &names(&["a.py", "A.py"])).as_deref(), Some("A.py"), "exact wins");
        assert_eq!(entry_spelling("a.PY", &names(&["a.py", "A.py"])), None, "two files that differ by case are never merged");
        assert_eq!(entry_spelling("c.py", &names(&["a.py"])), None, "absent");
        assert_eq!(entry_spelling("\u{e9}.md", &names(&["e\u{301}.md"])), None, "a normal-form difference is not a case difference");
    }

    /// WW1 (RULINGS JV1) on this machine's temporary volume: an existing file named in another case
    /// (file and folder) → its on-disk spelling, only where the volume is measured case-blind; the typed
    /// rel is kept for an absent target, for the real spelling itself, for a link in the chain and for a
    /// normal-form difference. On a case-sensitive volume every answer is the typed rel (an alias does not
    /// exist there) — asserted too, so the test says which volume it ran on.
    #[test]
    fn ww1_on_disk_rel_takes_the_real_name_of_an_existing_file_only() {
        let d = tmp(31);
        std::fs::create_dir_all(d.join("bin")).unwrap();
        std::fs::write(d.join("bin/a.py"), b"a\n").unwrap();
        let blind = case_blind(&d);
        eprintln!("ww1: the temporary volume is {}", if blind { "case-blind" } else { "case-sensitive" });
        let expect = |typed: &str, real: &str| if blind { real.to_string() } else { typed.to_string() };
        assert_eq!(on_disk_rel(&d, "bin/A.py"), expect("bin/A.py", "bin/a.py"));
        assert_eq!(on_disk_rel(&d, "BIN/A.PY"), expect("BIN/A.PY", "bin/a.py"), "a folder alias too");
        assert_eq!(on_disk_rel(&d, "bin/a.py"), "bin/a.py", "the real spelling is unchanged");
        assert_eq!(on_disk_rel(&d, "bin/New.py"), "bin/New.py", "an absent target keeps the typed spelling");
        assert_eq!(on_disk_rel(&d, "BIN/New.py"), "BIN/New.py", "absent target: nothing is taken from its folder either");
        // a normal-form difference (NFC typed, NFD on disk) is not taken, whatever the volume does with it
        std::fs::write(d.join("bin/e\u{301}.md"), b"e\n").unwrap();
        assert_eq!(on_disk_rel(&d, "bin/\u{e9}.md"), "bin/\u{e9}.md");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(d.join("bin"), d.join("lnk")).unwrap();
            assert_eq!(on_disk_rel(&d, "LNK/a.py"), "LNK/a.py", "a link in the chain keeps the typed spelling");
            assert_eq!(on_disk_rel(&d, "lnk/A.py"), "lnk/A.py");
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    /// WW1 round 2 (RULINGS KH1): a name whose fold is on a compiled list keeps the typed spelling — for
    /// a listed file the canonical name is the LIST's, and the classification refuses aliases of it. Here
    /// the disk spells the protection file `hooks/GUARD.SH`; the list's `hooks/guard.sh` stays as typed.
    #[test]
    fn ww1_kh1_on_disk_rel_keeps_a_listed_name() {
        let d = tmp(32);
        std::fs::create_dir_all(d.join("hooks")).unwrap();
        std::fs::write(d.join("hooks/GUARD.SH"), b"#!/bin/sh\n").unwrap();
        assert!(crate::pack::listed_in_compiled_lists("hooks/guard.sh"), "hooks/guard.sh is a compiled-list row");
        assert_eq!(on_disk_rel(&d, "hooks/guard.sh"), "hooks/guard.sh");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// WW1 round 2 (RULINGS KH3): a folder that cannot be LISTED is not an absent target — when the target
    /// exists, the real name comes from the canonical path (the stored case on macOS and Windows) under the
    /// same conditions. Unix only (the listing is denied with mode 0o311); skipped with the reason where the
    /// volume distinguishes case or the listing cannot be denied (e.g. as root).
    #[cfg(unix)]
    #[test]
    fn ww1_kh3_an_unlistable_folder_still_gives_the_real_name() {
        use std::os::unix::fs::PermissionsExt;
        let d = tmp(33);
        std::fs::create_dir_all(d.join("bin")).unwrap();
        std::fs::write(d.join("bin/a.py"), b"a\n").unwrap();
        if !case_blind(&d) {
            eprintln!("SKIPPED ww1_kh3_…: the temporary volume distinguishes letter case");
            let _ = std::fs::remove_dir_all(&d);
            return;
        }
        struct Restore(std::path::PathBuf);
        impl Drop for Restore {
            fn drop(&mut self) {
                let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o755));
            }
        }
        std::fs::set_permissions(d.join("bin"), std::fs::Permissions::from_mode(0o311)).unwrap();
        let restore = Restore(d.join("bin"));
        if std::fs::read_dir(d.join("bin")).is_ok() {
            eprintln!("SKIPPED ww1_kh3_…: the folder's listing could not be denied here (running as root?)");
            drop(restore);
            let _ = std::fs::remove_dir_all(&d);
            return;
        }
        assert!(std::fs::metadata(d.join("BIN/A.py")).is_ok(), "the target exists although its folder cannot be listed");
        let got = on_disk_rel(&d, "BIN/A.py");
        drop(restore);
        let _ = std::fs::remove_dir_all(&d);
        assert_eq!(got, "bin/a.py");
    }

    /// WW1 (RULINGS KM1): a listing that OPENS and then fails mid-way, before the target's name, is an
    /// incomplete listing — the canonical-path way then gives the real name (same conditions). The rule
    /// over the entry iterator (an error after some names → incomplete; a non-UTF-8 name skipped), and the
    /// whole map with the folder `bin`'s iterator failing after one name (the seam: the lister).
    #[test]
    fn ww1_km1_an_iterator_error_is_an_incomplete_listing() {
        let io = |m: &str| std::io::Error::other(m.to_string());
        assert_eq!(names_of(vec![Ok(Some("x".into())), Err(io("eio")), Ok(Some("a.py".into()))].into_iter()), None);
        assert_eq!(
            names_of(vec![Ok(Some("x".into())), Ok(None), Ok(Some("a.py".into()))].into_iter()),
            Some(vec!["x".to_string(), "a.py".to_string()])
        );
        let d = tmp(34);
        std::fs::create_dir_all(d.join("bin")).unwrap();
        std::fs::write(d.join("bin/a.py"), b"a\n").unwrap();
        std::fs::write(d.join("bin/b.py"), b"b\n").unwrap();
        if !case_blind(&d) {
            eprintln!("SKIPPED ww1_km1_…'s map half: the temporary volume distinguishes letter case");
            let _ = std::fs::remove_dir_all(&d);
            return;
        }
        let mid_way = |dir: &std::path::Path| -> Option<Vec<String>> {
            if dir.file_name().is_some_and(|n| n == "bin") {
                names_of(vec![Ok(Some("b.py".to_string())), Err(io("mid-way"))].into_iter())
            } else {
                list_names(dir)
            }
        };
        let got = on_disk_case_alias(&d, "BIN/A.py", &mid_way);
        let _ = std::fs::remove_dir_all(&d);
        assert_eq!(got.as_deref(), Some("bin/a.py"));
    }

    /// WW2 (RULINGS KY1): the fold over the shipped rels — exactly one match, none, or (a template that
    /// would ship two names differing by case — none today) two, which keeps the typed spelling.
    #[test]
    fn ww2_shipped_rels_folding() {
        let shipped = ["hooks/README.md", "bin/a.py"];
        assert_eq!(shipped_rels_folding("HOOKS/readme.MD", shipped.iter().copied()), ["hooks/README.md"]);
        assert_eq!(shipped_rels_folding("hooks/README.md", shipped.iter().copied()), ["hooks/README.md"]);
        assert!(shipped_rels_folding("hooks/other.md", shipped.iter().copied()).is_empty());
        let two = ["docs/X.md", "docs/x.md"];
        assert_eq!(shipped_rels_folding("DOCS/x.MD", two.iter().copied()).len(), 2);
        assert!(crate::pack::PACK_ALL.iter().any(|(r, _)| *r == "hooks/README.md"), "the template ships hooks/README.md");
    }

    /// WW2 (RULINGS KY1) on this machine's temporary volume (measured): the shipped `hooks/README.md`
    /// renamed on disk to `hooks/readme.MD` — every spelling records the template's rel on a case-blind
    /// volume; a file the template does not ship keeps WW1's rule (the disk's spelling). On a volume that
    /// distinguishes case `hooks/README.md` and `hooks/README.MD` are two files and each stays itself.
    #[test]
    fn ww2_a_shipped_rel_is_named_by_the_template() {
        let d = tmp(35);
        std::fs::create_dir_all(d.join("hooks")).unwrap();
        let blind = case_blind(&d);
        eprintln!("ww2: the temporary volume is {}", if blind { "case-blind" } else { "case-sensitive" });
        if blind {
            std::fs::write(d.join("hooks/README.md"), b"vendor\n").unwrap();
            std::fs::rename(d.join("hooks/README.md"), d.join("hooks/readme.MD")).unwrap();
            for typed in ["hooks/README.md", "hooks/readme.MD", "HOOKS/Readme.Md"] {
                assert_eq!(on_disk_rel(&d, typed), "hooks/README.md", "{typed}");
            }
            // a file the template does not ship: WW1's rule, the disk's spelling
            std::fs::write(d.join("hooks/Not-Shipped-WW2.md"), b"mine\n").unwrap();
            assert_eq!(on_disk_rel(&d, "hooks/not-shipped-ww2.MD"), "hooks/Not-Shipped-WW2.md");
            // an absent shipped file keeps the typed spelling (#191 stays a stated limit)
            std::fs::remove_file(d.join("hooks/readme.MD")).unwrap();
            assert_eq!(on_disk_rel(&d, "HOOKS/Readme.Md"), "HOOKS/Readme.Md");
        } else {
            std::fs::write(d.join("hooks/README.md"), b"vendor\n").unwrap();
            std::fs::write(d.join("hooks/README.MD"), b"another file\n").unwrap();
            assert_eq!(on_disk_rel(&d, "hooks/README.md"), "hooks/README.md");
            assert_eq!(on_disk_rel(&d, "hooks/README.MD"), "hooks/README.MD", "two files stay two");
            assert_eq!(on_disk_rel(&d, "hooks/readme.md"), "hooks/readme.md", "absent here: typed");
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    /// RULINGS AW3 (a): Windows short names are refused on every system; a `~` that is not followed
    /// by a digit stays allowed.
    #[test]
    fn short_name_components_refused() {
        for bad in [
            "hooks/ROLE-C~1.SH",
            "bin/JAVIS_~1.PY",
            "HOOKS~2/guard.sh",
            "bin\\JAVIS_~12.PY",
            "a/b/c~9",
        ] {
            let e = normalize_rel(bad).unwrap_err();
            assert!(format!("{e:?}").contains("짧은 이름"), "{bad:?}: {e:?}");
        }
        for ok in ["bin/a~b.py", "notes/~draft.md", "bin/x~", "bin/~.py"] {
            assert!(normalize_rel(ok).is_ok(), "{ok:?} refused");
        }
    }

    /// The same defect gives the same signature whatever pid, temp path, time or address it printed;
    /// a different line number or file is a different defect.
    #[test]
    fn signature_is_stable_and_discriminating() {
        let a = parse_report("bin/a.py", "SyntaxError: invalid syntax (line 12) pid 48213 at /var/folders/xy/T/tmp.AbC9/run.py\nmore").unwrap();
        let b = parse_report(
            "./bin/a.py",
            "\n  SyntaxError: invalid syntax (line 12) pid 51177 at /private/tmp/q/run.py",
        )
        .unwrap();
        assert_eq!(
            a.signature, b.signature,
            "{} vs {}",
            a.first_line, b.first_line
        );
        assert_eq!(a.error_class, "SyntaxError");
        let c = parse_report("bin/a.py", "SyntaxError: invalid syntax (line 13) pid 1").unwrap();
        assert_ne!(a.signature, c.signature);
        let d = parse_report("bin/b.py", "SyntaxError: invalid syntax (line 12)").unwrap();
        assert_ne!(a.signature, d.signature);
        let t1 = first_stable_line("boom at 2026-11-13T01:02:03.123Z id deadbeefcafe1234");
        let t2 = first_stable_line("boom at 2026-11-14T09:08:07.999Z id 0123456789abcdef");
        assert_eq!(t1, t2);
        assert_eq!(error_class("exit 127 boom"), "error");
        assert_eq!(
            error_class("json.decoder.JSONDecodeError: x"),
            "json.decoder.JSONDecodeError"
        );
        assert!(parse_report("bin/a.py", "   \n ").is_err());
        assert_eq!(signature("r", "c", "l"), {
            let mut h = Sha256::new();
            h.update(b"r\0c\0l");
            h.finalize()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        });
    }

    /// G4: two lanes reporting one signature keep BOTH rows; reading aggregates them.
    #[test]
    fn two_lanes_keep_both_rows() {
        let root = tmp(line!());
        let r = parse_report("bin/a.py", "ValueError: x").unwrap();
        let base = LaneKey("base".into());
        let dept = LaneKey("dept-3".into());
        assert!(matches!(
            record_in(&root, &base, &r, "worker", 1.0, 0, false, "ValueError: x").unwrap(),
            Recorded::New(_)
        ));
        assert!(matches!(
            record_in(&root, &dept, &r, "cso", 2.0, 0, false, "ValueError: x").unwrap(),
            Recorded::New(_)
        ));
        assert!(matches!(
            record_in(&root, &base, &r, "worker", 3.0, 0, false, "ValueError: x").unwrap(),
            Recorded::Updated(_)
        ));
        assert!(row_path_in(&root, &base, &r.signature).is_file());
        assert!(row_path_in(&root, &dept, &r.signature).is_file());
        let (all, broken) = load_all_in(&root);
        assert!(broken.is_empty());
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].count, 3);
        assert_eq!(all[0].lanes, vec![base, dept]);
        assert_eq!(all[0].first_seen, 1.0);
        assert_eq!(all[0].last_seen, 3.0);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The intake's own quota: new signatures over the hourly quota are not written; repeats are.
    #[test]
    fn own_quota_limits_new_signatures_only() {
        let root = tmp(line!());
        let lane = LaneKey("base".into());
        let r = parse_report("bin/a.py", "E: one").unwrap();
        assert!(matches!(
            record_in(
                &root,
                &lane,
                &r,
                "w",
                1.0,
                INTAKE_NEW_SIGNATURES_PER_HOUR,
                false,
                "E: one"
            )
            .unwrap(),
            Recorded::OverQuota { .. }
        ));
        assert!(!row_path_in(&root, &lane, &r.signature).exists());
        assert!(matches!(
            record_in(&root, &lane, &r, "w", 1.0, 0, false, "E: one").unwrap(),
            Recorded::New(_)
        ));
        assert!(matches!(
            record_in(
                &root,
                &lane,
                &r,
                "w",
                2.0,
                INTAKE_NEW_SIGNATURES_PER_HOUR,
                false,
                "E: one"
            )
            .unwrap(),
            Recorded::Updated(_)
        ));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The by-design failure (Windows `cys-dept` 127) is excluded there and only there.
    #[test]
    fn exclusion_list() {
        let err = "bash: cys-dept: command not found (exit 127)";
        assert_eq!(
            excluded("bin/javis_teamtoken.py", err, true),
            Some("windows-cys-dept-not-on-path")
        );
        assert_eq!(excluded("bin/javis_teamtoken.py", err, false), None);
        assert_eq!(excluded("bin/a.py", "SyntaxError: x", true), None);
        let root = tmp(line!());
        let r = parse_report("bin/javis_teamtoken.py", err).unwrap();
        assert!(matches!(
            record_in(&root, &LaneKey("base".into()), &r, "w", 1.0, 0, true, err).unwrap(),
            Recorded::Excluded { .. }
        ));
        assert!(
            !root.join("intake").exists(),
            "an excluded report wrote something"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// T3b (RULINGS AD8, BF2): the list's own table keeps T3's entry for both intake sources,
    /// Windows only, and it matches `Exit code 127` as the FIRST line or the "command not found" /
    /// "not recognized" sentence naming cys-dept — never a "127" anywhere.
    #[test]
    fn exclusion_table_keeps_the_windows_dept_entry_for_both_sources() {
        let mut reasons: Vec<&str> = EXCLUSIONS.iter().map(|e| e.reason).collect();
        reasons.sort_unstable();
        reasons.dedup();
        assert_eq!(reasons.len(), EXCLUSIONS.len(), "a reason names two entries");
        assert!(EXCLUSIONS.iter().all(|e| e.scope == ExclusionScope::All));
        let dept = "Exit code 127\n/usr/bin/bash: line 1: cys-dept: command not found";
        for sub in [None, Some("create")] {
            assert_eq!(
                excluded_in(EXCLUSIONS, "bin/cys-dept", sub, dept, true),
                Some("windows-cys-dept-not-on-path")
            );
        }
        assert_eq!(excluded_in(EXCLUSIONS, "bin/cys-dept", None, dept, false), None, "macOS: a missing cys-dept IS a defect");
        assert_eq!(excluded("bin/cys-dept", dept, true), Some("windows-cys-dept-not-on-path"));
        assert_eq!(excluded("bin/cys-dept", "Exit code 127", true), Some("windows-cys-dept-not-on-path"));
        assert_eq!(
            excluded("bin/javis_teamtoken.py", "'cys-dept' is not recognized as an internal or external command", true),
            Some("windows-cys-dept-not-on-path")
        );
        // a "127" elsewhere is not the precedent: a cys-dept failure that is not "not on PATH" is a defect
        assert_eq!(excluded("bin/cys-dept", "Exit code 1\ncys-dept: team 127 already exists", true), None);
        assert_eq!(excluded("bin/cys-dept", "Exit code 2\nretry in 127 s", true), None);
        assert_eq!(excluded("bin/a.py", "Exit code 127\nfoo: command not found", true), None, "names no cys-dept");
        // a detector-scope entry is never applied to a manual report through `excluded`
        let only_detector = [Exclusion {
            reason: "t",
            windows_only: false,
            scope: ExclusionScope::Detector,
            rule: ExclusionRule::DesignedAnswer {
                rel: "bin/x.py",
                sub: None,
                answers: &[DesignedExit { exit: 9, markers: &[&["verdict: no"]] }],
            },
        }];
        assert_eq!(excluded_in(&only_detector, "bin/x.py", None, "Exit code 9\nverdict: no", false), Some("t"));
        assert_eq!(excluded("bin/x.py", "Exit code 9\nverdict: no", false), None);
    }

    /// RULINGS BF2: a designed answer needs POSITIVE evidence — its exit code as the first line AND one
    /// of its own verdict-marker groups (every string of the group) for THAT exit code; a launch or
    /// interpreter failure is never excluded; a subcommand named by the entry must match.
    #[test]
    fn designed_answer_rule_needs_its_marker_and_never_a_launch_failure() {
        let e = Exclusion {
            reason: "t",
            windows_only: false,
            scope: ExclusionScope::Detector,
            rule: ExclusionRule::DesignedAnswer {
                rel: "bin/g.py",
                sub: Some("check"),
                answers: &[
                    DesignedExit { exit: 2, markers: &[&["verdict: hard_block"], &["\"verdict\": \"hard_block\""]] },
                    DesignedExit { exit: 1, markers: &[&["summary:", " missing "]] },
                ],
            },
        };
        assert!(e.matches("bin/g.py", Some("check"), "Exit code 2\nverdict: hard_block\n  x", false));
        assert!(e.matches("bin/g.py", Some("check"), "Exit code 2\n{\"verdict\": \"hard_block\"}", true));
        assert!(e.matches("bin/g.py", Some("check"), "Exit code 1\nsummary: 3 missing worker", false), "every string of the group");
        assert!(!e.matches("bin/g.py", Some("check"), "Exit code 1\nsummary: all here", false), "half a group is no marker");
        assert!(!e.matches("bin/g.py", Some("check"), "Exit code 2", false), "an exit code alone is no evidence");
        assert!(!e.matches("bin/g.py", Some("check"), "Exit code 1\nverdict: hard_block", false), "the marker of ANOTHER exit code");
        assert!(!e.matches("bin/g.py", Some("enforce"), "Exit code 2\nverdict: hard_block", false), "another subcommand");
        assert!(!e.matches("bin/g.py", None, "Exit code 2\nverdict: hard_block", false), "no subcommand known");
        assert!(!e.matches("bin/other.py", Some("check"), "Exit code 2\nverdict: hard_block", false));
        assert!(!e.matches("bin/g.py", Some("check"), "verdict: hard_block\nExit code 2", false), "not the first line");
        for launch in [
            "Exit code 2\nverdict: hard_block\nTraceback (most recent call last):\n  File \"x\"\nKeyError: 'k'",
            "Exit code 1\n  File \"g.py\", line 3\n    def (\n        ^\nSyntaxError: invalid syntax\nsummary: 3 missing x",
            "Exit code 1\nIndentationError: unexpected indent\nsummary: 1 missing x",
            "Exit code 2\npython3: can't open file '/x/bin/g.py': [Errno 2] No such file or directory\nverdict: hard_block",
            "Exit code 1\nbash: g.py: command not found\nsummary: 1 missing x",
            "Exit code 1\n'g.py' is not recognized as an internal or external command\nsummary: 1 missing x",
        ] {
            assert!(!e.matches("bin/g.py", Some("check"), launch, false), "a launch failure was excluded: {launch:?}");
            assert!(shows_launch_failure(launch));
        }
        let any_sub = Exclusion {
            rule: ExclusionRule::DesignedAnswer {
                rel: "bin/g.py",
                sub: None,
                answers: &[DesignedExit { exit: 4, markers: &[&["refused: PAUSED"]] }],
            },
            ..e
        };
        assert!(any_sub.matches("bin/g.py", Some("drain"), "Exit code 4\nrefused: PAUSED", false));
        assert!(any_sub.matches("bin/g.py", None, "Exit code 4\nrefused: PAUSED", false));
        assert_eq!(leading_exit_code("\n  Exit code -1 \nx"), Some(-1));
        assert_eq!(leading_exit_code("SyntaxError: x"), None);
        assert_eq!(leading_exit_code("Exit code 9x"), None);
    }

    /// RULINGS BF3: ONE normaliser. A single-line text is masked exactly as `first_stable_line` always
    /// did (so a `cys repair report --error "<one line>"` keeps its signature); a multi-line text skips
    /// the `Exit code N` wrapper and takes a traceback's LAST exception line; `parse_report` uses it.
    #[test]
    fn one_normaliser_for_the_error_line() {
        for one in [
            "SyntaxError: invalid syntax (line 12) pid 48213 at /var/folders/xy/T/tmp.AbC9/run.py",
            "KeyError: 'k' (pid 41234)",
            "Traceback: KeyError 'k' (pid 40001)",
            "Exit code 1",
            "boom at 2026-11-13T01:02:03.123Z id deadbeefcafe1234",
        ] {
            assert_eq!(stable_error_line(one), first_stable_line(one), "a single line changed: {one:?}");
        }
        assert_eq!(stable_error_line("Exit code 1\nError: Cannot find module 'express'"), "Error: Cannot find module 'express'");
        assert_eq!(
            stable_error_line("Exit code 1\nTraceback (most recent call last):\n  File \"/x/a.py\", line 3, in <module>\n    import javis_y\nModuleNotFoundError: No module named 'javis_y'\n"),
            "ModuleNotFoundError: No module named 'javis_y'"
        );
        let chained = "Traceback (most recent call last):\n  File \"a\", line 1\nKeyError: 'k'\n\nDuring handling of the above exception, another exception occurred:\n\nTraceback (most recent call last):\n  File \"a\", line 2\nValueError: bad (pid 77777)";
        assert_eq!(stable_error_line(chained), "ValueError: bad (pid <n>)");
        assert_eq!(stable_error_line("Exit code 2"), "Exit code 2", "nothing else → the exit line stays");
        assert_eq!(stable_error_line("\n\nboom\nmore"), "boom");
        // stdout before the traceback (Claude Code puts stdout first): the exception line still wins
        assert_eq!(
            stable_error_line("Exit code 1\nloading 12 rows\nTraceback (most recent call last):\n  File \"a\"\nKeyError: 'k'"),
            "KeyError: 'k'"
        );
        // parse_report uses it: the same failure, wrapped or not, is one signature
        let wrapped = parse_report("bin/a.py", "Exit code 1\nTraceback (most recent call last):\n  File \"/tmp/r1/a.py\", line 9\nKeyError: 'k' (pid 40001)").unwrap();
        let plain = parse_report("bin/a.py", "KeyError: 'k' (pid 40002)").unwrap();
        assert_eq!(wrapped.signature, plain.signature);
        assert_eq!(wrapped.first_line, "KeyError: 'k' (pid <n>)");
        assert_eq!(wrapped.error_class, "KeyError");
    }

    /// A broken row is listed, never fatal, and never overwritten by a report (one writer, O-C1-6).
    #[test]
    fn broken_row_is_reported_not_fatal() {
        let root = tmp(line!());
        let lane = LaneKey("base".into());
        let r = parse_report("bin/a.py", "E: x").unwrap();
        let p = row_path_in(&root, &lane, &r.signature);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, b"{not json").unwrap();
        assert!(matches!(
            record_in(&root, &lane, &r, "w", 1.0, 0, false, "E: x"),
            Err(RepairError::Corrupt(_))
        ));
        assert_eq!(std::fs::read(&p).unwrap(), b"{not json");
        let (all, broken) = load_all_in(&root);
        assert!(all.is_empty());
        assert_eq!(broken.len(), 1);
        std::fs::write(&p, br#"{"schema": 99}"#).unwrap();
        assert!(matches!(load_row(&p), Err(RepairError::Corrupt(_))));
        let _ = std::fs::remove_dir_all(&root);
    }
}
