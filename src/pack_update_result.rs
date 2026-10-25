//! Pack-update's terminal result: disk action and command disposition are independent.
//! Schema 2 keeps old U9 readers on their visible-unknown path for incomplete results.

use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackAction {
    NoOp,
    Applied,
    Validated,
}

impl PackAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoOp => "no-op",
            Self::Applied => "applied",
            Self::Validated => "validated",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PendingFacts {
    pub failed: u64,
    pub deferred: u64,
    pub unknown: u64,
    pub unknown_new: u64,
    pub node_unknown: u64,
    pub node_unknown_new: u64,
    pub pending_kept: u64,
    pub intent_refused: u64,
    pub persist_failed: bool,
    pub skipped: bool,
}

impl PendingFacts {
    pub fn degraded(&self) -> bool {
        self.failed > 0
            || self.unknown_new > 0
            || self.node_unknown_new > 0
            || self.intent_refused > 0
            || self.persist_failed
    }

    pub fn incomplete(&self) -> bool {
        self.degraded()
            || self.deferred > 0
            || self.pending_kept > 0
            || self.unknown > 0
            || self.node_unknown > 0
            || self.skipped
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalRecord {
    pub action: PackAction,
    pub code: i32,
    pub remote: String,
    pub disk: Option<String>,
    pub pending: PendingFacts,
    pub accepted_degraded: bool,
}

fn token(s: &str) -> String {
    let s = s.trim();
    if s.is_empty() {
        "-".into()
    } else if s
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+' | '_'))
    {
        s.into()
    } else {
        "?".into()
    }
}

impl TerminalRecord {
    pub fn freshness_confirmed(&self) -> bool {
        match (
            self.disk.as_deref().and_then(crate::pack::parse_semver),
            crate::pack::parse_semver(&self.remote),
        ) {
            (Some(d), Some(r)) => d >= r,
            _ => false,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.pending.unknown_new > self.pending.unknown
            || self.pending.node_unknown_new > self.pending.node_unknown
        {
            return Err("pack terminal: new unknown exceeds total unknown".into());
        }
        let want = if self.accepted_degraded {
            if self.action != PackAction::Applied {
                return Err("pack terminal: accepted degradation without apply".into());
            }
            crate::pack::EXIT_ACCEPTED_DEGRADED
        } else if self.pending.degraded() {
            crate::pack::EXIT_REINJECT_DEGRADED
        } else {
            0
        };
        if self.code != want {
            return Err("pack terminal: disposition does not match measured facts".into());
        }
        if self.action == PackAction::Validated && self.pending.incomplete() {
            return Err("pack terminal: dry-run cannot have reinjection effects".into());
        }
        Ok(())
    }

    /// Exactly one bounded first line. D/U never expose legacy freshness proof.
    pub fn line(&self) -> Result<String, String> {
        self.validate()?;
        let uncertain = self.pending.incomplete() || self.accepted_degraded;
        let confirmed = self.action == PackAction::NoOp && !uncertain && self.freshness_confirmed();
        let gate = if self.action == PackAction::Applied && !uncertain {
            "applied"
        } else {
            "up-to-date"
        };
        let terminal = if self.code == 3 || self.code == 4 {
            "degraded"
        } else {
            self.action.as_str()
        };
        let mut line = format!("{} gate={gate}", crate::pack::PACK_UPDATE_OUTCOME_PREFIX);
        if confirmed {
            line.push_str(&format!(
                " remote={} disk={} disk_parse=ok",
                token(&self.remote),
                token(self.disk.as_deref().unwrap_or(""))
            ));
        }
        line.push_str(&format!(
            " schema=2 terminal={terminal} pack_action={} compat={} exit={} observed_remote={} observed_disk={} failed={} deferred={} unknown={} new_unknown={} node_unknown={} node_new_unknown={} pending_kept={} intent_refused={} pending_persist={} reinject_skipped={} accepted_degraded={}",
            self.action.as_str(), if confirmed { "confirmed" } else { "unknown" }, self.code,
            token(&self.remote), token(self.disk.as_deref().unwrap_or("")), self.pending.failed,
            self.pending.deferred, self.pending.unknown, self.pending.unknown_new, self.pending.node_unknown,
            self.pending.node_unknown_new, self.pending.pending_kept, self.pending.intent_refused,
            u8::from(self.pending.persist_failed), u8::from(self.pending.skipped), u8::from(self.accepted_degraded),
        ));
        if line.len() > 4096 {
            return Err("pack terminal: record exceeds 4096 bytes".into());
        }
        Ok(line)
    }
}

/// None is a legacy producer, never evidence of successful application.
pub fn parse(stdout: &str, code: i32) -> Result<Option<TerminalRecord>, String> {
    let lines: Vec<&str> = stdout.lines().collect();
    let first = lines.first().copied().unwrap_or("");
    let schema_line = lines
        .iter()
        .position(|l| l.split_whitespace().any(|p| p.starts_with("schema=")));
    let Some(index) = schema_line else {
        return Ok(None);
    };
    if index != 0 || first.len() > 4096 {
        return Err("pack terminal: missing or oversized first record".into());
    }
    let mut words = first.split_whitespace();
    if words.next() != Some(crate::pack::PACK_UPDATE_OUTCOME_PREFIX) {
        return Err("pack terminal: unrecognised first-record prefix".into());
    }
    let mut fields = BTreeMap::new();
    for word in words {
        let (key, value) = word.split_once('=').ok_or("pack terminal: invalid field")?;
        if value.is_empty() || fields.insert(key, value).is_some() {
            return Err("pack terminal: empty or duplicate field".into());
        }
    }
    let field = |name: &str| {
        fields
            .get(name)
            .copied()
            .ok_or_else(|| format!("pack terminal: missing {name}"))
    };
    if field("schema")? != "2" {
        return Err("pack terminal: unsupported schema".into());
    }
    let number = |name: &str| -> Result<u64, String> {
        let value = field(name)?;
        if !value.bytes().all(|b| b.is_ascii_digit()) {
            return Err(format!("pack terminal: invalid {name}"));
        }
        value
            .parse()
            .map_err(|_| format!("pack terminal: invalid {name}"))
    };
    let flag = |name: &str| -> Result<bool, String> {
        match field(name)? {
            "0" => Ok(false),
            "1" => Ok(true),
            _ => Err(format!("pack terminal: invalid {name}")),
        }
    };
    let action = match field("pack_action")? {
        "no-op" => PackAction::NoOp,
        "applied" => PackAction::Applied,
        "validated" => PackAction::Validated,
        _ => return Err("pack terminal: invalid pack action".into()),
    };
    let disk = field("observed_disk")?;
    let record = TerminalRecord {
        action,
        code,
        remote: field("observed_remote")?.into(),
        disk: if disk == "-" { None } else { Some(disk.into()) },
        pending: PendingFacts {
            failed: number("failed")?,
            deferred: number("deferred")?,
            unknown: number("unknown")?,
            unknown_new: number("new_unknown")?,
            intent_refused: number("intent_refused")?,
            node_unknown: number("node_unknown")?,
            node_unknown_new: number("node_new_unknown")?,
            pending_kept: number("pending_kept")?,
            persist_failed: flag("pending_persist")?,
            skipped: flag("reinject_skipped")?,
        },
        accepted_degraded: flag("accepted_degraded")?,
    };
    // Reconstruct critical fields from measured facts; D/0, C/3 and legacy-proof contamination fail closed.
    let canonical = record.line()?;
    let expected: BTreeMap<_, _> = canonical
        .split_whitespace()
        .skip(1)
        .filter_map(|s| s.split_once('='))
        .collect();
    for key in [
        "gate",
        "schema",
        "terminal",
        "pack_action",
        "compat",
        "exit",
        "remote",
        "disk",
        "disk_parse",
        "observed_remote",
        "observed_disk",
    ] {
        if fields.get(key) != expected.get(key) {
            return Err(format!("pack terminal: contradictory {key}"));
        }
    }
    if lines.iter().skip(1).any(|l| {
        l.trim_start()
            .starts_with(crate::pack::PACK_UPDATE_OUTCOME_PREFIX)
    }) {
        return Err("pack terminal: multiple terminal records".into());
    }
    Ok(Some(record))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clean() -> TerminalRecord {
        TerminalRecord {
            action: PackAction::NoOp,
            code: 0,
            remote: "2.0.0".into(),
            disk: Some("2.0.0".into()),
            pending: PendingFacts::default(),
            accepted_degraded: false,
        }
    }

    #[test]
    fn three_records_have_distinct_truth_and_round_trip() {
        let c = clean();
        let d = TerminalRecord {
            code: 3,
            pending: PendingFacts {
                persist_failed: true,
                ..Default::default()
            },
            ..clean()
        };
        let u = TerminalRecord {
            disk: None,
            ..clean()
        };
        for record in [&c, &d, &u] {
            let line = record.line().unwrap();
            assert_eq!(parse(&line, record.code).unwrap().as_ref(), Some(record));
            assert_eq!(line.contains(" disk_parse=ok"), record == &c);
        }
        assert_eq!(parse("old sidecar output", 0).unwrap(), None);
    }

    #[test]
    fn code_record_mismatch_and_legacy_silent_success_attempts_are_refused() {
        let c = clean().line().unwrap();
        let d = TerminalRecord {
            code: 3,
            pending: PendingFacts {
                persist_failed: true,
                ..Default::default()
            },
            ..clean()
        }
        .line()
        .unwrap();
        assert!(parse(&c, 3).is_err(), "C/3 is silent success in old GUI");
        assert!(parse(&d, 0).is_err(), "D/0 hides persistence failure");
        for bad in [
            d.replace("gate=up-to-date", "gate=degraded"),
            d.replace("gate=up-to-date ", ""),
            format!("{c}\n{d}"),
            format!("{d} disk_parse=ok disk=2.0.0 remote=2.0.0"),
            format!("{d} gate=up-to-date"),
            d.replacen("PACK_UPDATE_OUTCOME", "PACK_UPDATE_OUTCOMEX", 1),
            format!("progress\n{d}"),
            d.replace("pending_persist=1", "pending_persist=garbage"),
            d.replace("schema=2", "schema=3"),
            d.replace(" intent_refused=0", ""),
        ] {
            assert!(parse(&bad, 3).is_err(), "accepted unsafe record: {bad}");
        }
        let impossible = TerminalRecord {
            pending: PendingFacts {
                persist_failed: true,
                ..Default::default()
            },
            ..clean()
        };
        assert!(
            impossible.line().is_err(),
            "producer must refuse a clean code with pending failure"
        );
    }

    #[test]
    fn carried_unknown_is_not_a_new_failure_and_phases_do_not_double_count() {
        let mut r = clean();
        r.pending.unknown = 1;
        r.pending.node_unknown = 1;
        assert!(!r.pending.degraded());
        assert!(r.pending.incomplete());
        let line = r.line().unwrap();
        assert!(!line.contains(" disk_parse=ok"));
        assert_eq!(parse(&line, 0).unwrap(), Some(r));
    }

    #[test]
    fn accepted_four_and_skipped_zero_are_not_reclassified_as_clean_completion() {
        let mut r = clean();
        r.action = PackAction::Applied;
        r.pending.skipped = true;
        assert!(r.line().unwrap().contains("compat=unknown"));
        r.accepted_degraded = true;
        r.code = 4;
        assert_eq!(parse(&r.line().unwrap(), 4).unwrap(), Some(r));
    }
}
