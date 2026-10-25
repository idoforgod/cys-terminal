// Frozen legacy parser fixture from f0ac3b0c8fb0726ec2061be328433c4c779b1807.
// Only compiled inside the app test module. Function/type bytes below are unchanged.

struct PackUpdateOutcomeToken {
    gate: String,
    remote: String,
    disk: String,
    disk_parse_ok: bool,
}

enum ReinjectResult {
    Measured { failed: u64, deferred: u64, unknown: u64 },
    Skipped { reason: String },
    Absent,
}

fn parse_pack_update_outcome(stdout: &str) -> Option<PackUpdateOutcomeToken> {
    for line in stdout.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix(cys::pack::PACK_UPDATE_OUTCOME_PREFIX) else { continue };
        if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
            continue; // 접두사만 같은 다른 토큰(PACK_UPDATE_OUTCOMEX…) 배제
        }
        let mut t = PackUpdateOutcomeToken {
            gate: String::new(),
            remote: String::new(),
            disk: String::new(),
            disk_parse_ok: false,
        };
        for kv in rest.split_whitespace() {
            match kv.split_once('=') {
                Some(("gate", v)) => t.gate = v.to_string(),
                Some(("remote", v)) => t.remote = v.to_string(),
                Some(("disk", v)) => t.disk = v.to_string(),
                Some(("disk_parse", v)) => t.disk_parse_ok = v == "ok",
                _ => {}
            }
        }
        return if t.gate.is_empty() { None } else { Some(t) };
    }
    None
}

fn uptodate_confirmed(t: &PackUpdateOutcomeToken) -> bool {
    if t.gate != "up-to-date" || !t.disk_parse_ok {
        return false;
    }
    match (cys::pack::parse_semver(&t.disk), cys::pack::parse_semver(&t.remote)) {
        (Some(d), Some(r)) => d >= r,
        _ => false,
    }
}

fn parse_reinject_result(stdout: &str) -> ReinjectResult {
    for line in stdout.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix(cys::pack::REINJECT_RESULT_PREFIX) {
            let (mut failed, mut deferred, mut unknown) = (0u64, 0u64, 0u64);
            let mut skipped = false;
            let mut reason = String::from("unknown");
            for tok in rest.split_whitespace() {
                if let Some(v) = tok.strip_prefix("failed=") {
                    failed = v.parse().unwrap_or(0);
                } else if let Some(v) = tok.strip_prefix("deferred=") {
                    deferred = v.parse().unwrap_or(0);
                } else if let Some(v) = tok.strip_prefix("unknown=") {
                    unknown = v.parse().unwrap_or(0);
                } else if tok == "reinject=skipped" {
                    skipped = true;
                } else if let Some(v) = tok.strip_prefix("reason=") {
                    if !v.is_empty() {
                        // 표시용 — 길이 상한(외부 문자열 방어).
                        reason = v.chars().take(64).collect();
                    }
                }
            }
            if skipped {
                return ReinjectResult::Skipped { reason };
            }
            return ReinjectResult::Measured { failed, deferred, unknown };
        }
    }
    ReinjectResult::Absent
}

#[test]
fn frozen_legacy_spans_are_the_measured_old_consumer() {
    let src = include_str!("pack_update_legacy_tests.rs").replace("\r\n", "\n");
    for (name, expected) in [
        ("struct PackUpdateOutcomeToken {", "c95bb55c3c25e4d88528fe631e45aa9b927345e86b365408c61bb5c11075b954"),
        ("enum ReinjectResult {", "475a170d9d877260320a2a6fcf458d42b85a118a343a78967f4e87af2fd86f10"),
        ("fn parse_pack_update_outcome(", "734cfc8db7a5c40f540f29753e62030e57dd2ced464e78f03cd47663cd0022ce"),
        ("fn uptodate_confirmed(", "9c04959b3c9acb693fed6e2b071a1d47ecc402045ad2a159c2c4abc74da9c5e7"),
        ("fn parse_reinject_result(", "6ea522828f64c4f50140dc82bf97d5e37ba4939f2dd837d93b2c85179e23c542"),
    ] {
        let at = src.find(name).unwrap();
        let end = at + src[at..].find("\n}\n").unwrap() + 3;
        assert_eq!(cys::pack::content_hash_pub(&src[at..end]), expected, "legacy fixture changed: {name}");
    }
}

#[test]
fn actual_old_parser_consumes_clean_degraded_unknown_and_applied_degraded() {
    use cys::pack_update_result::{PackAction, PendingFacts, TerminalRecord};
    let clean = TerminalRecord { action: PackAction::NoOp, code: 0, remote: "2.0.0".into(), disk: Some("2.0.0".into()), pending: PendingFacts::default(), accepted_degraded: false };
    let degraded = TerminalRecord { code: 3, pending: PendingFacts { persist_failed: true, ..Default::default() }, ..clean.clone() };
    let unknown = TerminalRecord { disk: None, ..clean.clone() };
    let applied = TerminalRecord { action: PackAction::Applied, ..degraded.clone() };
    for (label, record, confirmed) in [("C/0", clean.clone(), true), ("D/3", degraded.clone(), false), ("U/0", unknown, false), ("applied+D/3", applied, false)] {
        let line = record.line().unwrap();
        let old = parse_pack_update_outcome(&line).expect("old reader recognises the first record");
        assert_eq!(old.gate, "up-to-date", "{label}: must take old no-op branch, not pack-updated fallback");
        assert_eq!(uptodate_confirmed(&old), confirmed, "{label}");
        assert!(matches!(parse_reinject_result(&line), ReinjectResult::Absent));
        assert_eq!(cys::pack_update_result::parse(&line, record.code).unwrap(), Some(record));
    }
    let c = clean.line().unwrap();
    let d = degraded.line().unwrap();
    assert!(uptodate_confirmed(&parse_pack_update_outcome(&c).unwrap()), "negative control: old C/3 still appears confirmed");
    assert!(cys::pack_update_result::parse(&c, 3).is_err(), "new consumer must reject the old-reader silent-success pair");
    assert!(!uptodate_confirmed(&parse_pack_update_outcome(&format!("{d}\n{c}")).unwrap()), "the first matching line owns the old verdict");
    assert!(uptodate_confirmed(&parse_pack_update_outcome(&format!("{c}\n{d}")).unwrap()), "control: a later correction cannot protect an old reader");
    match parse_reinject_result("PACK_UPDATE_RESULT failed=1 deferred=2 unknown=3") {
        ReinjectResult::Measured { failed, deferred, unknown } => assert_eq!((failed, deferred, unknown), (1, 2, 3)),
        _ => panic!("legacy measured record changed"),
    }
    match parse_reinject_result("PACK_UPDATE_RESULT reinject=skipped reason=daemon_rpc_failed") {
        ReinjectResult::Skipped { reason } => assert_eq!(reason, "daemon_rpc_failed"),
        _ => panic!("legacy skipped record changed"),
    }
}
