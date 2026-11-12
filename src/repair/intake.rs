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
            other => parts.push(other),
        }
    }
    if parts.is_empty() {
        return Err(bad("비어 있음"));
    }
    Ok(parts.join("/"))
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

/// Operations that fail BY DESIGN and must not become intake (design §4-4; precedent
/// `cysjavis-pack/bin/javis_teamtoken.py` 470–483: on Windows `cys-dept` is not on a pane's PATH, so
/// the department-creation check always fails with 127 — reported, it would add one row per prompt
/// per seat). Returns the reason when excluded.
pub fn excluded(rel: &str, error: &str, windows: bool) -> Option<&'static str> {
    let low = error.to_ascii_lowercase();
    let mentions_dept = rel.contains("cys-dept") || low.contains("cys-dept");
    let not_found =
        low.contains("127") || low.contains("command not found") || low.contains("not recognized");
    if windows && mentions_dept && not_found {
        return Some("windows-cys-dept-not-on-path");
    }
    None
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
    let first_line = first_stable_line(error);
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
