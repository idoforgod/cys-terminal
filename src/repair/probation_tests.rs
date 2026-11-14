//! T6 tests of the appended job's work: the tick decision, the probation re-check with RULINGS O3's
//! attribution, the end of probation (N3), the update-inside-probation binding, Windows' report-only
//! mode, the start-time pass (AK7, DEFERRED #49) and M10 (F9, notices, line endings, rule documents).

use super::*;
use crate::repair::apply::tests::{committed_general, lane_row, rid, rig, Rig};
use crate::repair::apply::{no_repair_lock, CheckOutcome, NoFaults, ReconcileReport};
use crate::repair::boot_critical::CheckKind;
use crate::repair::ledger::CheckTiming;
use std::cell::RefCell;
use std::time::Duration;

fn no_hooks(_: &RepairId, _: &LaneKey, _: HookStep) -> Result<ReconcileReport, RepairError> {
    Ok(ReconcileReport { hooks: Vec::new() })
}
fn no_registrar(_: &Path, _: bool) -> Result<String, RepairError> {
    Ok("{\"hooks\": []}".into())
}

struct Doubles<'a> {
    check: &'a CheckFn<'a>,
    reference: &'a ReferenceFn<'a>,
    original: &'a dyn Fn(&str) -> Option<Vec<u8>>,
    intake: &'a dyn Fn(&str, &str) -> bool,
    excluded: &'a dyn Fn(&str) -> bool,
}

fn job<'a>(r: &'a Rig, d: &'a Doubles<'a>, form: ApplyForm, now: f64, lock: &'a Path) -> JobEnv<'a> {
    JobEnv {
        op: OpEnv { now, ..r.env(&NoFaults, form) },
        incarnation: "inc-2",
        check: d.check,
        reference: d.reference,
        hooks: &no_hooks,
        registrar: &no_registrar,
        locked: &no_repair_lock,
        excluded: d.excluded,
        original: d.original,
        intake: d.intake,
        apply_lock: lock,
    }
}

fn outcome(result: CheckResult, ms: u64) -> Result<CheckOutcome, RepairError> {
    Ok(CheckOutcome { result, ms })
}

const NOW: f64 = 1_800_000_000.0;

#[test]
fn due_decides_incarnation_ten_minutes_and_hour() {
    let st = |inc: &str, p: f64, s: f64| JobState {
        schema: 1,
        lane: LaneKey("base".into()),
        last_incarnation: inc.into(),
        last_probation_check: p,
        last_sweep: s,
        initial_catchup_complete: false,
        pack_instance: None,
        pack_dir: None,
        pack_dir_identity: None,
        pack_generation: None,
    };
    // a new incarnation → probation at once (even 1 s after the last check)
    let d = due(Some(&st("inc-1", NOW - 1.0, NOW - 1.0)), "inc-2", NOW);
    assert!(d.first_of_incarnation && d.probation && !d.sweep);
    // same incarnation: 10 min / 1 h
    assert!(!due(Some(&st("inc-2", NOW - 599.0, NOW - 3599.0)), "inc-2", NOW).anything(), "most ticks are no-ops");
    assert!(due(Some(&st("inc-2", NOW - 600.0, NOW - 10.0)), "inc-2", NOW).probation);
    assert!(due(Some(&st("inc-2", NOW - 10.0, NOW - 3600.0)), "inc-2", NOW).sweep);
}

/// A failing re-check (macOS) reverts that ONE repair by rule one, keeps the repaired copy and files
/// ONE intake row; the job writes the ledger only.
#[cfg(target_os = "macos")]
#[test]
fn failing_recheck_reverts_one_repair_and_files_one_defect() {
    let r = rig("recheck");
    let a = rid("R-1");
    let b = rid("R-2");
    committed_general(&r, &a, "hooks/a.sh", Some(b"A0\n"), Some(b"A1\n"));
    committed_general(&r, &b, "hooks/b.sh", Some(b"B0\n"), Some(b"B1\n"));
    let fail_a = |rel: &str, _: &Path, _: Duration| {
        if rel == "hooks/a.sh" { outcome(CheckResult::Fail("syntax error".into()), 5) } else { outcome(CheckResult::Pass, 5) }
    };
    let intake = RefCell::new(Vec::new());
    let note = |rel: &str, e: &str| {
        intake.borrow_mut().push((rel.to_string(), e.to_string()));
        true
    };
    let d = Doubles { check: &fail_a, reference: &|_, _| outcome(CheckResult::Pass, 1), original: &|_| None, intake: &note, excluded: &|_| false };
    let lock = r.base.join(".pack-apply.lock");
    let rep = run_tick(&job(&r, &d, ApplyForm::Exchange, NOW, &lock), false).unwrap();
    assert_eq!(rep.reverted, vec![a.clone()]);
    assert_eq!(std::fs::read(r.pack.join("hooks/a.sh")).unwrap(), b"A0\n");
    assert_eq!(std::fs::read(r.pack.join("hooks/b.sh")).unwrap(), b"B1\n", "only that one repair");
    assert!(r.vault_bytes().contains(&b"A1\n".to_vec()), "the repaired copy is kept");
    assert_eq!(intake.borrow().len(), 1, "one defect row");
    assert_eq!(ledger::read_record_in(&r.root, &a).unwrap().unwrap().state, RepairState::Reverted);
    // the job state was written (this incarnation's first check consumed)
    assert_eq!(read_job_state_in(&r.root, &r.lane).unwrap().last_incarnation, "inc-2");
}

/// Windows 0.15.0: a failing re-check is recorded and reported only — nothing is put back.
#[test]
fn windows_recheck_failure_reports_only() {
    let r = rig("winre");
    let a = rid("R-3");
    committed_general(&r, &a, "bin/a.py", Some(b"A0\n"), Some(b"A1\n"));
    let fail = |_: &str, _: &Path, _: Duration| outcome(CheckResult::Fail("x".into()), 5);
    let d = Doubles { check: &fail, reference: &|_, _| outcome(CheckResult::Pass, 1), original: &|_| None, intake: &|_, _| true, excluded: &|_| false };
    let lock = r.base.join(".pack-apply.lock");
    let rep = run_tick(&job(&r, &d, ApplyForm::DepositFirst, NOW, &lock), false).unwrap();
    assert!(rep.reverted.is_empty());
    assert_eq!(rep.reported_only.len(), 1);
    assert_eq!(std::fs::read(r.pack.join("bin/a.py")).unwrap(), b"A1\n");
}

/// The binding (design §4-9 "자동 복귀가 무엇에 묶이는가", F1 SA:61): an update inside probation
/// wrote the vendor's bytes (vendor-first) → live ≠ expected → the probation closes `대체됨` and the
/// job NEVER restores the old bytes, even though the old repair's check would now fail; the same for
/// a pack-generation change.
#[test]
fn update_inside_probation_never_restores_old_bytes() {
    for gen_change in [false, true] {
        let r = rig("update");
        let a = rid("R-4");
        committed_general(&r, &a, "hooks/s.sh", Some(b"OLD\n"), Some(b"REPAIRED\n"));
        if gen_change {
            std::fs::write(r.pack.join(".pack-version"), "0.15.1\n").unwrap();
        } else {
            r.put("hooks/s.sh", b"VENDOR-NEW\n", 0o755);
        }
        let fail = |_: &str, _: &Path, _: Duration| outcome(CheckResult::Fail("would fail".into()), 5);
        let d = Doubles { check: &fail, reference: &|_, _| outcome(CheckResult::Pass, 1), original: &|_| None, intake: &|_, _| true, excluded: &|_| false };
        let lock = r.base.join(".pack-apply.lock");
        let rep = run_tick(&job(&r, &d, ApplyForm::Exchange, NOW, &lock), false).unwrap();
        assert!(rep.reverted.is_empty(), "never reverted");
        assert_eq!(rep.superseded, vec![a.clone()]);
        let want: &[u8] = if gen_change { b"REPAIRED\n" } else { b"VENDOR-NEW\n" };
        assert_eq!(std::fs::read(r.pack.join("hooks/s.sh")).unwrap(), want);
        let row = ledger::read_lane_in(&r.root, &a, &r.lane).unwrap().unwrap();
        assert!(row.probation.unwrap().closed.unwrap().starts_with("대체됨"));
    }
}

/// Only repaired files are re-checked (A 1-4 — no full per-boot sweep): an unrelated broken file in
/// the pack is not looked at by the probation pass.
#[test]
fn probation_rechecks_only_repaired_files() {
    let r = rig("onlyrepaired");
    let a = rid("R-5");
    committed_general(&r, &a, "hooks/a.sh", Some(b"A0\n"), Some(b"A1\n"));
    r.put("hooks/unrelated.sh", b"BROKEN\n", 0o755);
    let seen = RefCell::new(Vec::new());
    let rec = |rel: &str, _: &Path, _: Duration| {
        seen.borrow_mut().push(rel.to_string());
        outcome(CheckResult::Pass, 5)
    };
    let d = Doubles { check: &rec, reference: &|_, _| outcome(CheckResult::Pass, 1), original: &|_| None, intake: &|_, _| true, excluded: &|_| true };
    let lock = r.base.join(".pack-apply.lock");
    let mut report = TickReport::default();
    let env = job(&r, &d, ApplyForm::Exchange, NOW, &lock);
    probation_pass(&env, &mut report);
    assert_eq!(*seen.borrow(), vec!["hooks/a.sh".to_string()]);
}

fn with_timings(r: &Rig, id: &RepairId, rel: &str, check_ms: u64, reference_ms: u64) {
    let row = ledger::read_lane_in(&r.root, id, &r.lane).unwrap().unwrap();
    lane_row(r, id, row.applied.clone(), vec![CheckTiming { rel: rel.into(), check_ms, reference_kind: CheckKind::ShSyntax, reference_ms }]);
}

/// RULINGS O3 / R4 / P3: one sequence (control → second run → reference now), one decision.
///  · restart storm: the control passes, the second run passes → check_timeout, nothing put back;
///  · the second run times out again but the reference NOW is slower too → check_timeout;
///  · FAILURE only with an unchanged reference (ratio ≈ 1) and a recorded duration well under the
///    deadline;
///  · the control times out → check_timeout.
#[cfg(target_os = "macos")]
#[test]
fn o3_timeout_attribution() {
    struct Case {
        control: CheckResult,
        second: CheckResult,
        ref_now_ms: u64,
        recorded_ms: u64,
        failure: bool,
    }
    let cases = [
        Case { control: CheckResult::Pass, second: CheckResult::Pass, ref_now_ms: 100, recorded_ms: 40_000, failure: false },
        Case { control: CheckResult::Pass, second: CheckResult::Timeout, ref_now_ms: 300, recorded_ms: 40_000, failure: false },
        Case { control: CheckResult::Pass, second: CheckResult::Timeout, ref_now_ms: 100, recorded_ms: 5_000, failure: true },
        Case { control: CheckResult::Timeout, second: CheckResult::Timeout, ref_now_ms: 100, recorded_ms: 5_000, failure: false },
        Case { control: CheckResult::Pass, second: CheckResult::Timeout, ref_now_ms: 100, recorded_ms: 45_000, failure: false },
    ];
    for (n, c) in cases.iter().enumerate() {
        let r = rig(&format!("o3-{n}"));
        let a = rid("R-6");
        committed_general(&r, &a, "hooks/t.sh", Some(b"PRE\n"), Some(b"POST\n"));
        with_timings(&r, &a, "hooks/t.sh", c.recorded_ms, 100);
        let calls = RefCell::new(0u32);
        let pack = r.pack.clone();
        let check = |_: &str, root: &Path, _: Duration| {
            let mut k = calls.borrow_mut();
            *k += 1;
            if root != pack.as_path() {
                return outcome(c.control.clone(), 50); // the control on the pre-image scratch copy
            }
            if *k == 1 { outcome(CheckResult::Timeout, 60_000) } else { outcome(c.second.clone(), 60_000) }
        };
        let reference = |_: CheckKind, _: Duration| outcome(CheckResult::Pass, c.ref_now_ms);
        let d = Doubles { check: &check, reference: &reference, original: &|_| None, intake: &|_, _| true, excluded: &|_| false };
        let lock = r.base.join(".pack-apply.lock");
        let rep = run_tick(&job(&r, &d, ApplyForm::Exchange, NOW, &lock), false).unwrap();
        if c.failure {
            assert_eq!(rep.reverted, vec![a.clone()], "case {n}");
            assert_eq!(std::fs::read(r.pack.join("hooks/t.sh")).unwrap(), b"PRE\n");
        } else {
            assert!(rep.reverted.is_empty(), "case {n}: put back on a timeout");
            assert_eq!(std::fs::read(r.pack.join("hooks/t.sh")).unwrap(), b"POST\n");
            let row = ledger::read_lane_in(&r.root, &a, &r.lane).unwrap().unwrap();
            assert_eq!(row.probation.unwrap().check_timeouts.len(), 1, "case {n}: recorded");
        }
    }
}

/// K4 for a CREATED file: the control is the reference check; blamed only when the reference passes
/// and the scaled duration is under the deadline ÷ 1.5 — reference 2× slower → check_timeout,
/// reference 1.2× → the created file is moved into the vault (macOS).
#[cfg(target_os = "macos")]
#[test]
fn created_file_hang_is_blamed_only_within_the_reference_slowdown() {
    for (ratio, moved) in [(2.0f64, false), (1.2f64, true)] {
        let r = rig("created");
        let a = rid("R-7");
        committed_general(&r, &a, "hooks/new.sh", None, Some(b"NEW\n"));
        with_timings(&r, &a, "hooks/new.sh", 30_000, 100);
        let reference = move |_: CheckKind, _: Duration| outcome(CheckResult::Pass, (100.0 * ratio) as u64);
        let check = |_: &str, _: &Path, _: Duration| outcome(CheckResult::Timeout, 60_000);
        let d = Doubles { check: &check, reference: &reference, original: &|_| None, intake: &|_, _| true, excluded: &|_| false };
        let lock = r.base.join(".pack-apply.lock");
        run_tick(&job(&r, &d, ApplyForm::Exchange, NOW, &lock), false).unwrap();
        assert_eq!(r.live("hooks/new.sh") == TargetState::Absent, moved, "ratio {ratio}");
    }
}

/// N3: 24 h over → 사용 중, also when the last results were only timeouts (recorded, one line).
#[test]
fn probation_of_only_timeouts_ends_in_use_at_24h() {
    let r = rig("24h");
    let a = rid("R-8");
    committed_general(&r, &a, "hooks/a.sh", Some(b"A\n"), Some(b"B\n"));
    let mut row = ledger::read_lane_in(&r.root, &a, &r.lane).unwrap().unwrap();
    row.probation.as_mut().unwrap().check_timeouts.push(("hooks/a.sh".into(), 5.0));
    ledger::write_lane_in(&r.root, &row).unwrap();
    let never = |_: &str, _: &Path, _: Duration| -> Result<CheckOutcome, RepairError> { panic!("no check after 24 h") };
    let d = Doubles { check: &never, reference: &|_, _| outcome(CheckResult::Pass, 1), original: &|_| None, intake: &|_, _| true, excluded: &|_| true };
    let lock = r.base.join(".pack-apply.lock");
    let rep = run_tick(&job(&r, &d, ApplyForm::Exchange, NOW + 2.0 * 86_400.0, &lock), false).unwrap();
    assert_eq!(rep.ended, vec![a.clone()]);
    assert!(rep.lines.iter().any(|l| l.contains("시간 초과 1건")));
    assert_eq!(ledger::read_record_in(&r.root, &a).unwrap().unwrap().state, RepairState::InUse);
}

/// AK7: a committed deletion whose tombstone is missing (a crash right after the commit) gets it
/// back at the first tick of the incarnation; DEFERRED #49: the origin's missing row is written and
/// an older in-force repair of the same file is superseded.
#[test]
fn start_time_pass_rebuilds_tombstones_and_finishes_an_interrupted_commit() {
    let r = rig("start");
    let old = rid("R-9");
    committed_general(&r, &old, "hooks/del.sh", Some(b"X\n"), Some(b"Y\n"));
    let newer = rid("R-10");
    committed_general(&r, &newer, "hooks/del.sh", Some(b"Y\n"), None);
    std::fs::remove_file(ledger::lane_path_in(&r.root, &newer, &r.lane)).unwrap();
    let d = Doubles { check: &|_, _, _| outcome(CheckResult::Pass, 1), reference: &|_, _| outcome(CheckResult::Pass, 1), original: &|_| None, intake: &|_, _| true, excluded: &|_| true };
    let lock = r.base.join(".pack-apply.lock");
    let env = job(&r, &d, ApplyForm::Exchange, NOW, &lock);
    let lines = start_time_pass(&env).unwrap();
    assert!(!lines.is_empty());
    let ts = crate::repair::tombstones::load_in(&r.root, &r.lane).unwrap();
    assert!(ts.iter().any(|t| t.rel == "hooks/del.sh" && t.repair == newer), "{ts:?}");
    assert!(ledger::read_lane_in(&r.root, &newer, &r.lane).unwrap().is_some());
    assert_eq!(ledger::read_record_in(&r.root, &old).unwrap().unwrap().state, RepairState::Superseded);
    // idempotent: a second pass changes nothing more
    let again = start_time_pass(&env).unwrap();
    assert!(again.is_empty(), "{again:?}");
}

// ───────────────────────────── M10 ─────────────────────────────

fn listed() -> Option<&'static str> {
    crate::repair::boot_critical::entries().iter().map(|e| e.rel).find(|r| r.ends_with(".sh"))
}

/// M10: a broken unapproved boot-critical file is put back to our original of the current
/// generation (macOS; the changed copy kept in the vault); a passing one is kept and noticed ONCE
/// per bytes; a line-endings-only one → ledger only; rule documents are never touched; Windows →
/// reported only.
#[cfg(target_os = "macos")]
#[test]
fn m10_scan_rules() {
    let Some(rel) = listed() else {
        return; // no compiled list in this build
    };
    let r = rig("m10");
    let original = b"#!/bin/bash\necho original\n".to_vec();
    let orig2 = original.clone();
    let orig_of = move |x: &str| if x == rel { Some(orig2.clone()) } else { None };
    let fails = |x: &str, _: &Path, _: Duration| if x == rel { outcome(CheckResult::Fail("bad".into()), 5) } else { outcome(CheckResult::Pass, 5) };
    let passes = |_: &str, _: &Path, _: Duration| outcome(CheckResult::Pass, 5);
    let lock = r.base.join(".pack-apply.lock");
    // broken → reverted
    r.put(rel, b"#!/bin/bash\nbroken(\n", 0o755);
    let d = Doubles { check: &fails, reference: &|_, _| outcome(CheckResult::Pass, 1), original: &orig_of, intake: &|_, _| true, excluded: &|_| false };
    let f = m10_sweep(&job(&r, &d, ApplyForm::Exchange, NOW, &lock)).unwrap();
    assert!(f.iter().any(|x| x.rel == rel && x.kind == "reverted"), "{f:?}");
    assert_eq!(std::fs::read(r.pack.join(rel)).unwrap(), original);
    assert!(r.vault_bytes().contains(&b"#!/bin/bash\nbroken(\n".to_vec()));
    // a revert is told to the owner once too (D-11-05's control: "reverted AND one notice")
    assert_eq!(take_notices_in(&r.root, &r.lane).unwrap().len(), 1);
    // passing change → kept, noticed once per bytes
    r.put(rel, b"#!/bin/bash\necho edited\n", 0o755);
    let d2 = Doubles { check: &passes, ..d };
    let env2 = job(&r, &d2, ApplyForm::Exchange, NOW, &lock);
    m10_sweep(&env2).unwrap();
    m10_sweep(&env2).unwrap();
    assert_eq!(take_notices_in(&r.root, &r.lane).unwrap().len(), 1);
    assert_eq!(take_notices_in(&r.root, &r.lane).unwrap().len(), 0, "never twice");
    r.put(rel, b"#!/bin/bash\necho edited again\n", 0o755);
    m10_sweep(&env2).unwrap();
    assert_eq!(take_notices_in(&r.root, &r.lane).unwrap().len(), 1, "new bytes → one new notice");
    // line endings only → ledger, no notice
    r.put(rel, b"#!/bin/bash\r\necho original\r\n", 0o755);
    m10_sweep(&env2).unwrap();
    assert_eq!(take_notices_in(&r.root, &r.lane).unwrap().len(), 0);
    assert_eq!(std::fs::read(r.pack.join(rel)).unwrap(), b"#!/bin/bash\r\necho original\r\n");
    // rule documents are never touched
    r.put(rel, b"#!/bin/bash\nbroken(\n", 0o755);
    let d3 = Doubles { check: &fails, excluded: &|_| true, ..d };
    m10_sweep(&job(&r, &d3, ApplyForm::Exchange, NOW, &lock)).unwrap();
    assert_eq!(std::fs::read(r.pack.join(rel)).unwrap(), b"#!/bin/bash\nbroken(\n");
    // Windows: reported only
    let f = m10_sweep(&job(&r, &d, ApplyForm::DepositFirst, NOW, &lock)).unwrap();
    assert!(f.iter().any(|x| x.kind == "report-only"));
    assert_eq!(std::fs::read(r.pack.join(rel)).unwrap(), b"#!/bin/bash\nbroken(\n");
}

/// F9 at M10 with a planted ledger row (REWRITTEN under RULINGS DR2 — old expectation: "a planted
/// ledger row does NOT stop the revert of a failing file", `assert_eq!(live, orig)`). A row of this
/// generation that claims the file without the daemon's operation log is INCONSISTENT evidence: the
/// one resolution answers UNKNOWN — the row never makes the file approved (the check runs), and
/// nothing is put back; a finding goes to the worker.
#[cfg(target_os = "macos")]
#[test]
fn m10_planted_row_still_reverted() {
    let Some(rel) = listed() else {
        return;
    };
    let r = rig("m10planted");
    let id = rid("R-11");
    r.put(rel, b"#!/bin/bash\nbroken(\n", 0o755);
    let mut rec = crate::repair::apply::tests::record(&r, &id, crate::repair::types::Classification::BootCritical, Vec::new(), RepairState::Probation);
    rec.approval = Some(crate::repair::apply::tests::approval("x"));
    ledger::write_record_in(&r.root, &rec).unwrap();
    lane_row(&r, &id, vec![(rel.to_string(), r.live(rel).canon())], Vec::new());
    let orig = b"#!/bin/bash\necho ok\n".to_vec();
    let o2 = orig.clone();
    let orig_of = move |_: &str| Some(o2.clone());
    let fails = |x: &str, _: &Path, _: Duration| if x == rel { outcome(CheckResult::Fail("bad".into()), 5) } else { outcome(CheckResult::Pass, 5) };
    let d = Doubles { check: &fails, reference: &|_, _| outcome(CheckResult::Pass, 1), original: &orig_of, intake: &|_, _| true, excluded: &|_| false };
    let lock = r.base.join(".pack-apply.lock");
    let f = m10_sweep(&job(&r, &d, ApplyForm::Exchange, NOW, &lock)).unwrap();
    assert_ne!(std::fs::read(r.pack.join(rel)).unwrap(), orig, "nothing put back on unknown evidence");
    assert!(f.iter().any(|x| x.rel == rel && x.kind == "approval-unknown"), "{f:?}");
}

/// D-11-08: the file changed between the check and the put-back → no exchange, the new bytes stay.
#[cfg(target_os = "macos")]
#[test]
fn m10_changed_between_check_and_revert_is_left() {
    let Some(rel) = listed() else {
        return;
    };
    let r = rig("m10race");
    r.put(rel, b"#!/bin/bash\nbroken(\n", 0o755);
    let pack = r.pack.clone();
    let fails = move |x: &str, _: &Path, _: Duration| {
        if x == rel {
            std::fs::write(pack.join(rel).with_extension("tmp"), b"#!/bin/bash\nthird\n").unwrap();
            std::fs::rename(pack.join(rel).with_extension("tmp"), pack.join(rel)).unwrap();
            outcome(CheckResult::Fail("bad".into()), 5)
        } else {
            outcome(CheckResult::Pass, 5)
        }
    };
    let orig_of = |_: &str| Some(b"#!/bin/bash\necho ok\n".to_vec());
    let d = Doubles { check: &fails, reference: &|_, _| outcome(CheckResult::Pass, 1), original: &orig_of, intake: &|_, _| true, excluded: &|_| false };
    let lock = r.base.join(".pack-apply.lock");
    let f = m10_sweep(&job(&r, &d, ApplyForm::Exchange, NOW, &lock)).unwrap();
    assert!(f.iter().any(|x| x.kind == "superseded"), "{f:?}");
    assert_eq!(std::fs::read(r.pack.join(rel)).unwrap(), b"#!/bin/bash\nthird\n");
}

/// M10 and a check that times out (RULINGS CT10 — REWRITTEN; old expectation, from the flagged
/// "ratio 1" reading the ruling rejected: `[(control_ok=true, second=Timeout) → reverted = true,
/// (false, Timeout) → false, (true, Pass) → false]`, i.e. a hang was blamed and the file put back
/// when the control passed in time). Now there is no timing evidence for an unapproved file, so a
/// timeout is `check_timeout`: nothing is moved in ANY of the three cases and a `check-timeout`
/// finding (no notice) records it; the next hour checks again.
#[cfg(target_os = "macos")]
#[test]
fn m10_hang_attribution() {
    let Some(rel) = listed() else {
        return;
    };
    for (control_ok, second, reverted) in [(true, CheckResult::Timeout, false), (false, CheckResult::Timeout, false), (true, CheckResult::Pass, false)] {
        let r = rig("m10hang");
        r.put(rel, b"#!/bin/bash\nhang\n", 0o755);
        let pack = r.pack.clone();
        let n = RefCell::new(0);
        let second2 = second.clone();
        let check = move |_: &str, root: &Path, _: Duration| {
            if root != pack.as_path() {
                return if control_ok { outcome(CheckResult::Pass, 100) } else { outcome(CheckResult::Timeout, 60_000) };
            }
            *n.borrow_mut() += 1;
            if *n.borrow() == 1 { outcome(CheckResult::Timeout, 60_000) } else { outcome(second2.clone(), 60_000) }
        };
        let orig_of = |_: &str| Some(b"#!/bin/bash\necho ok\n".to_vec());
        let d = Doubles { check: &check, reference: &|_, _| outcome(CheckResult::Pass, 1), original: &orig_of, intake: &|_, _| true, excluded: &|_| false };
        let lock = r.base.join(".pack-apply.lock");
        let f = m10_sweep(&job(&r, &d, ApplyForm::Exchange, NOW, &lock)).unwrap();
        assert_eq!(std::fs::read(r.pack.join(rel)).unwrap() == b"#!/bin/bash\necho ok\n", reverted, "control_ok={control_ok} second={second:?}");
        assert!(f.iter().any(|x| x.rel == rel && x.kind == "check-timeout" && !x.notice), "{f:?}");
    }
}

// ───────────────────────────── RULINGS CT10 — supersession before expiry; clock steps ─────────────────────────────

/// CT10: at (or past) the 24-hour boundary a probation whose generation changed — or whose live file
/// is not the applied state — closes 대체됨, never 사용 중 (a sleeping machine / a forward clock step).
/// Without CT10 the 24-hour close came first.
#[cfg(target_os = "macos")]
#[test]
fn supersession_is_checked_before_the_24_hour_close() {
    let r = rig("ct10expiry");
    let a = rid("R-1");
    committed_general(&r, &a, "hooks/s.sh", Some(b"PRE\n"), Some(b"POST\n"));
    std::fs::write(r.pack.join(".pack-version"), "0.15.1\n").unwrap();
    let d = Doubles { check: &|_, _, _| outcome(CheckResult::Pass, 1), reference: &|_, _| outcome(CheckResult::Pass, 1), original: &|_| None, intake: &|_, _| true, excluded: &|_| false };
    let lock = r.base.join(".pack-apply.lock");
    let later = NOW + 3.0 * 86_400.0;
    let rep = run_tick(&job(&r, &d, ApplyForm::Exchange, later, &lock), false).unwrap();
    assert!(rep.superseded.contains(&a) && !rep.ended.contains(&a), "{rep:?}");
    let row = ledger::read_lane_in(&r.root, &a, &r.lane).unwrap().unwrap();
    assert!(row.probation.unwrap().closed.unwrap().starts_with("대체됨"));
}

/// CT10: a BACKWARD clock step never postpones a due check beyond one interval — the work is due at
/// once (without CT10 it waited until the wall clock caught up with the last run).
#[test]
fn a_backward_clock_step_makes_the_work_due() {
    let st = JobState {
        schema: 1,
        lane: LaneKey("base".into()),
        last_incarnation: "inc-2".into(),
        last_probation_check: NOW,
        last_sweep: NOW,
        initial_catchup_complete: false,
        pack_instance: None,
        pack_dir: None,
        pack_dir_identity: None,
        pack_generation: None,
    };
    let d = due(Some(&st), "inc-2", NOW - 86_400.0);
    assert!(d.probation && d.sweep, "{d:?}");
}

/// The job's outputs are the ledger only (F1): no source line of this module or of the engine sends
/// anything to a queue, an alert route or the event bus.
#[test]
fn job_writes_ledger_only_source_pin() {
    for (name, src) in [("probation.rs", include_str!("probation.rs")), ("apply.rs", include_str!("apply.rs"))] {
        let prod = &src[..src.find("#[cfg(test)]\n#[path").unwrap_or(src.len())];
        for needle in ["bus.publish", "enqueue", "send_text", "alert_route", "inject(", "push_queued", "cys send"] {
            assert!(!prod.contains(needle), "{name} contains {needle:?}");
        }
    }
}

/// B3 / C7 7.5: every re-check child gets the deadline `clamp_check_timeout(timeout_s)` — 60 s for a
/// file with no registered hook timeout, the listed hook's timeout clamped to 10–60 s otherwise; one
/// check at a time (the runner is called sequentially from the job thread).
#[test]
fn tick_child_deadline_is_clamped() {
    let r = rig("deadline");
    let a = rid("R-40");
    committed_general(&r, &a, "bin/plain.py", Some(b"A\n"), Some(b"B\n"));
    let seen = RefCell::new(Vec::new());
    let check = |rel: &str, _: &Path, d: Duration| {
        seen.borrow_mut().push((rel.to_string(), d));
        outcome(CheckResult::Pass, 1)
    };
    let d = Doubles { check: &check, reference: &|_, _| outcome(CheckResult::Pass, 1), original: &|_| None, intake: &|_, _| true, excluded: &|_| true };
    let lock = r.base.join(".pack-apply.lock");
    run_tick(&job(&r, &d, ApplyForm::Exchange, NOW, &lock), false).unwrap();
    assert_eq!(*seen.borrow(), vec![("bin/plain.py".to_string(), Duration::from_secs(60))]);
    if let Some(e) = crate::repair::boot_critical::entries().iter().find(|e| e.timeout_s.is_some()) {
        let want = u64::from(crate::repair::boot_critical::clamp_check_timeout(e.timeout_s));
        assert_eq!(check_deadline(e.rel), Duration::from_secs(want));
        assert!((10..=60).contains(&want));
    }
}

/// M10 without a known original (pro channel / a pack newer than this binary): a file equal to its
/// install-manifest hash is "unchanged since the install" — no check, no finding, no notice (no
/// false alarm for every vendor file); a file that differs from it is still examined.
#[test]
fn m10_unknown_original_uses_the_install_manifest_for_unchanged_only() {
    let Some(rel) = listed() else {
        return;
    };
    let r = rig("m10manifest");
    r.put(rel, b"#!/bin/bash\necho pro-vendor\n", 0o755);
    let sha: String = {
        use sha2::{Digest, Sha256};
        Sha256::digest(b"#!/bin/bash\necho pro-vendor\n").iter().map(|x| format!("{x:02x}")).collect()
    };
    let mut m = serde_json::Map::new();
    m.insert(rel.to_string(), serde_json::Value::String(sha));
    std::fs::write(r.pack.join(".install-manifest.json"), serde_json::Value::Object(m).to_string()).unwrap();
    let calls = RefCell::new(0);
    let check = |_: &str, _: &Path, _: Duration| {
        *calls.borrow_mut() += 1;
        outcome(CheckResult::Pass, 1)
    };
    let d = Doubles { check: &check, reference: &|_, _| outcome(CheckResult::Pass, 1), original: &|_| None, intake: &|_, _| true, excluded: &|_| false };
    let lock = r.base.join(".pack-apply.lock");
    let f = m10_sweep(&job(&r, &d, ApplyForm::Exchange, NOW, &lock)).unwrap();
    assert!(f.iter().all(|x| x.rel != rel), "{f:?}");
    assert_eq!(*calls.borrow(), 0, "an unchanged file is not even checked");
    r.put(rel, b"#!/bin/bash\necho edited\n", 0o755);
    let f = m10_sweep(&job(&r, &d, ApplyForm::Exchange, NOW, &lock)).unwrap();
    assert!(f.iter().any(|x| x.rel == rel && x.kind == "kept"), "{f:?}");
}

/// RULINGS CP1 (a) — FINDABLE: a put-back finding carries the kept copy's path (the SEALED copy in the
/// M10 sub-ledger) and sha256, and its sentence names that path and the legitimate way back — an
/// ordinary repair (`cys repair begin --files <rel> --trigger …` → rehearsal, signatures, the owner's
/// final approval); it never tells anyone to copy the file back by hand. Pinned.
#[cfg(target_os = "macos")]
#[test]
fn m10_finding_names_the_kept_copy_and_the_repair_route() {
    let Some(rel) = listed() else {
        return;
    };
    let r = rig("m10findable");
    let broken = b"#!/bin/bash\nbroken(\n".to_vec();
    r.put(rel, &broken, 0o755);
    let fails = |x: &str, _: &Path, _: Duration| if x == rel { outcome(CheckResult::Fail("bad".into()), 5) } else { outcome(CheckResult::Pass, 5) };
    let orig_of = |_: &str| Some(b"#!/bin/bash\necho ok\n".to_vec());
    let d = Doubles { check: &fails, reference: &|_, _| outcome(CheckResult::Pass, 1), original: &orig_of, intake: &|_, _| true, excluded: &|_| false };
    let lock = r.base.join(".pack-apply.lock");
    let f = m10_sweep(&job(&r, &d, ApplyForm::Exchange, NOW, &lock)).unwrap();
    let rev = f.iter().find(|x| x.kind == "reverted").expect("a put-back");
    let kept = rev.kept_copy.clone().expect("the kept copy's path");
    assert!(kept.starts_with(m10_root_in(&r.root, &r.lane)), "in the lane's M10 sub-ledger: {}", kept.display());
    assert_eq!(std::fs::read(&kept).unwrap(), broken, "the kept copy holds the changed bytes");
    let sha: String = {
        use sha2::{Digest, Sha256};
        Sha256::digest(&broken).iter().map(|x| format!("{x:02x}")).collect()
    };
    assert_eq!(rev.sha256, sha);
    assert!(rev.note.contains(&kept.display().to_string()), "{}", rev.note);
    assert!(rev.note.contains(&format!("cys repair begin --files {rel} --trigger")), "{}", rev.note);
    assert!(rev.note.contains("주인 최종 승인"), "{}", rev.note);
    for hand in ["복사", "cp ", "copy", "옮겨 놓", "덮어"] {
        assert!(!rev.note.contains(hand), "the sentence tells a hand copy ({hand}): {}", rev.note);
    }
    // the findings file is `repairs/m10/<lane>.json` and holds the same fields
    let file = std::fs::read_to_string(r.root.join("m10").join(format!("{}.json", r.lane.0))).unwrap();
    assert!(file.contains("kept_copy") && file.contains(&sha));
}

/// RULINGS CP1 (b): rule one holds in the M10 sub-ledger exactly as in the vault — every put-back
/// takes a FRESH slot; an earlier slot (its sealed copy and its payload) is never deleted, moved or
/// overwritten by a later put-back of the same file. (Growth: one slot per put-back — DEFERRED #97.)
#[cfg(target_os = "macos")]
#[test]
fn m10_each_put_back_takes_a_fresh_slot() {
    let Some(rel) = listed() else {
        return;
    };
    let r = rig("m10fresh");
    let fails = |x: &str, _: &Path, _: Duration| if x == rel { outcome(CheckResult::Fail("bad".into()), 5) } else { outcome(CheckResult::Pass, 5) };
    let orig_of = |_: &str| Some(b"#!/bin/bash\necho ok\n".to_vec());
    let d = Doubles { check: &fails, reference: &|_, _| outcome(CheckResult::Pass, 1), original: &orig_of, intake: &|_, _| true, excluded: &|_| false };
    let lock = r.base.join(".pack-apply.lock");
    r.put(rel, b"#!/bin/bash\nfirst(\n", 0o755);
    let a = m10_sweep(&job(&r, &d, ApplyForm::Exchange, NOW, &lock)).unwrap();
    let slot_a = a.iter().find(|x| x.kind == "reverted").unwrap().slot.clone().unwrap();
    let kept_a = a.iter().find(|x| x.kind == "reverted").unwrap().kept_copy.clone().unwrap();
    r.put(rel, b"#!/bin/bash\nsecond(\n", 0o755);
    let b = m10_sweep(&job(&r, &d, ApplyForm::Exchange, NOW + 3600.0, &lock)).unwrap();
    let slot_b = b.iter().find(|x| x.kind == "reverted").unwrap().slot.clone().unwrap();
    assert_ne!(slot_a, slot_b, "a fresh slot per put-back");
    assert_eq!(std::fs::read(&kept_a).unwrap(), b"#!/bin/bash\nfirst(\n", "the earlier deposit is untouched");
    assert!(slot_a.join("payload").is_dir() && slot_a.join("meta/pre.json").is_file());
    let slots = std::fs::read_dir(slot_a.parent().unwrap()).unwrap().filter_map(Result::ok)
        .filter(|e| crate::repair::ledger::is_slot_name(&e.file_name().to_string_lossy())).count();
    assert_eq!(slots, 2, "one slot per put-back, none removed");
    assert!(crate::repair::wal::unfinished_for_lane(&m10_root_in(&r.root, &r.lane), &r.lane).is_empty());
}

// ───────────────────────────── RULINGS CT5 — one binding, checked under the lock ─────────────────────────────

/// CT5 (blocking 5): an update lands BETWEEN the scan's sampling and its put-back (here: inside the
/// file's check — the pack generation changes and the file's bytes stay what was sampled). The
/// binding (generation, live state, source of the right bytes) is revalidated under the pack-apply
/// lock right before the exchange: it changed, so NOTHING is put back in this tick (a `superseded`
/// finding, the live bytes untouched). Without CT5 the original resolved for the OLD generation was
/// put back into the new one.
#[cfg(target_os = "macos")]
#[test]
fn m10_put_back_revalidates_its_binding_under_the_lock() {
    let Some(rel) = listed() else {
        return;
    };
    let r = rig("ct5bind");
    r.put(rel, b"#!/bin/bash\nbroken(\n", 0o755);
    let pack = r.pack.clone();
    let fails = move |x: &str, _: &Path, _: Duration| {
        if x == rel {
            // an update installs a new generation while the check runs
            std::fs::write(pack.join(".pack-version"), "0.15.1\n").unwrap();
            outcome(CheckResult::Fail("bad".into()), 5)
        } else {
            outcome(CheckResult::Pass, 5)
        }
    };
    let orig_of = |_: &str| Some(b"#!/bin/bash\necho ok\n".to_vec());
    let d = Doubles { check: &fails, reference: &|_, _| outcome(CheckResult::Pass, 1), original: &orig_of, intake: &|_, _| true, excluded: &|_| false };
    let lock = r.base.join(".pack-apply.lock");
    let f = m10_sweep(&job(&r, &d, ApplyForm::Exchange, NOW, &lock)).unwrap();
    assert!(f.iter().any(|x| x.rel == rel && x.kind == "superseded"), "{f:?}");
    assert!(!f.iter().any(|x| x.kind == "reverted"), "nothing put back: {f:?}");
    assert_eq!(std::fs::read(r.pack.join(rel)).unwrap(), b"#!/bin/bash\nbroken(\n");
}

/// CT5 (major 6): while another writer holds the pack-apply lock, the tick's recovery does NOTHING —
/// an unfinished operation is not resumed or abandoned (it may be a live writer's, not a crashed
/// one), and the job state is not written (the tick stays due); once the lock is free the next tick
/// resumes it. Without CT5 the recovery ran outside the lock.
#[cfg(target_os = "macos")]
#[test]
fn recovery_waits_for_an_active_writer() {
    let r = rig("ct5lock");
    let before = r.put("hooks/w.sh", b"OLD\n", 0o755);
    let (src, incoming) = r.source("w-new", b"NEW\n", 0o755);
    struct StopAfterRecord;
    impl crate::repair::apply::Faults for StopAfterRecord {
        fn at(&self, p: crate::repair::apply::Point, _: &Path, _: Option<&crate::repair::vault::Slot>) -> bool {
            p == crate::repair::apply::Point::AfterRecord
        }
    }
    let spec = crate::repair::apply::OpSpec { id: rid("R-7"), rel: "hooks/w.sh".into(), kind: "revert".into(), expected: before, incoming, source: Some(src), bound: crate::repair::apply::Bound::now(&r.pack) };
    assert!(crate::repair::apply::run_op(&r.env(&StopAfterRecord, ApplyForm::Exchange), &spec).is_err());
    assert_eq!(crate::repair::wal::unfinished_for_lane(&r.root, &r.lane).len(), 1);
    let lock = r.base.join(".pack-apply.lock");
    let d = Doubles { check: &|_, _, _| outcome(CheckResult::Pass, 1), reference: &|_, _| outcome(CheckResult::Pass, 1), original: &|_| None, intake: &|_, _| true, excluded: &|_| false };
    let (held_tx, held_rx) = std::sync::mpsc::channel::<()>();
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let lock2 = lock.clone();
    let holder = std::thread::spawn(move || {
        crate::pack_apply_lock::with_apply_lock(&lock2, || {
            held_tx.send(()).unwrap();
            release_rx.recv().unwrap();
        })
        .unwrap();
    });
    held_rx.recv().unwrap();
    let rep = run_tick(&job(&r, &d, ApplyForm::Exchange, NOW, &lock), false).unwrap();
    assert!(rep.lines.iter().any(|l| l.contains("아무것도 하지 않습니다")), "{:?}", rep.lines);
    assert_eq!(crate::repair::wal::unfinished_for_lane(&r.root, &r.lane).len(), 1, "not touched while a writer is active");
    assert!(read_job_state_in(&r.root, &r.lane).is_none(), "the tick stays due");
    release_tx.send(()).unwrap();
    holder.join().unwrap();
    let rep = run_tick(&job(&r, &d, ApplyForm::Exchange, NOW, &lock), false).unwrap();
    assert_eq!(rep.resumed, 1, "{:?}", rep.lines);
    assert!(crate::repair::wal::unfinished_for_lane(&r.root, &r.lane).is_empty());
}

// ───────────────────────────── RULINGS CT7 / CU4 — the start pass ─────────────────────────────

/// CT7: a HOOK-ONLY repair committed durably, then a crash before its hook operation record — the
/// start pass does not skip it because its committed file set is empty: the origin's row is written
/// and the hook step runs. Without CT7 it was skipped for good.
#[test]
fn hook_only_commit_interrupted_before_its_hook_record_is_recovered() {
    let r = rig("ct7start");
    let id = rid("R-35");
    let mut rec = crate::repair::apply::tests::record(&r, &id, crate::repair::types::Classification::General, vec![crate::repair::ledger::FileChange {
        rel: "settings:Stop:k.sh".into(),
        kind: crate::repair::ledger::ChangeKind::HookUnregister { event: "Stop".into(), script: "k.sh".into() },
        pre: crate::repair::ledger::PreImage::Absent,
        post: TargetState::Absent,
        diff: None,
    }], RepairState::Probation);
    rec.committed = Vec::new();
    ledger::write_record_in(&r.root, &rec).unwrap();
    let calls = RefCell::new(Vec::new());
    let hooks = |_: &RepairId, _: &LaneKey, step: HookStep| -> Result<ReconcileReport, RepairError> {
        calls.borrow_mut().push(step);
        Ok(ReconcileReport { hooks: Vec::new() })
    };
    let d = Doubles { check: &|_, _, _| outcome(CheckResult::Pass, 1), reference: &|_, _| outcome(CheckResult::Pass, 1), original: &|_| None, intake: &|_, _| true, excluded: &|_| false };
    let lock = r.base.join(".pack-apply.lock");
    let env = JobEnv { hooks: &hooks, ..job(&r, &d, ApplyForm::Exchange, NOW, &lock) };
    let lines = start_time_pass(&env).unwrap();
    assert_eq!(*calls.borrow(), vec![HookStep::Unregister], "{lines:?}");
    assert!(ledger::read_lane_in(&r.root, &id, &r.lane).unwrap().is_some(), "the origin's row is written");
}

/// CU4 / DR1: the reconstructed origin row is written CLOSED 대체됨 — the record holds no field naming
/// the commit's generation and no modification time decides anything — whether or not an update
/// rewrote the generation files since. (REWRITTEN with DR1 — old expectation: a row "confirmed" by
/// the generation files' mtimes being older than `record.json` stayed open; only an update after
/// the commit closed it.)
#[test]
fn reconstructed_origin_row_is_closed_when_the_generation_cannot_be_confirmed() {
    for updated_since in [false, true] {
        let r = rig("cu4row");
        let id = rid("R-36");
        committed_general(&r, &id, "hooks/g.sh", Some(b"PRE\n"), Some(b"POST\n"));
        std::fs::remove_file(ledger::lane_path_in(&r.root, &id, &r.lane)).unwrap();
        if updated_since {
            std::fs::write(r.pack.join(".pack-version"), "0.15.1\n").unwrap();
        }
        let d = Doubles { check: &|_, _, _| outcome(CheckResult::Pass, 1), reference: &|_, _| outcome(CheckResult::Pass, 1), original: &|_| None, intake: &|_, _| true, excluded: &|_| false };
        let lock = r.base.join(".pack-apply.lock");
        start_time_pass(&job(&r, &d, ApplyForm::Exchange, NOW, &lock)).unwrap();
        let row = ledger::read_lane_in(&r.root, &id, &r.lane).unwrap().unwrap();
        assert!(row.probation.as_ref().unwrap().closed.as_deref().is_some_and(|c| c.starts_with("대체됨")), "updated_since={updated_since}: {:?}", row.probation);
    }
}

// ───────────────────────────── RULINGS CT9 — the scan's three gaps ─────────────────────────────

/// CT9 (major 12): a line-endings-only difference is a ledger-only branch BEFORE any check or write —
/// even when the file's check would fail, nothing is checked and nothing is put back. Without CT9 a
/// failing check of a CR-only file reached the put-back.
#[cfg(target_os = "macos")]
#[test]
fn line_endings_only_is_ledger_only_before_any_check() {
    let Some(rel) = listed() else {
        return;
    };
    let r = rig("ct9cr");
    r.put(rel, b"#!/bin/bash\r\necho ok\r\n", 0o755);
    let checked = RefCell::new(0);
    let fails = |_: &str, _: &Path, _: Duration| {
        *checked.borrow_mut() += 1;
        outcome(CheckResult::Fail("bad".into()), 5)
    };
    let orig_of = |_: &str| Some(b"#!/bin/bash\necho ok\n".to_vec());
    let d = Doubles { check: &fails, reference: &|_, _| outcome(CheckResult::Pass, 1), original: &orig_of, intake: &|_, _| true, excluded: &|_| false };
    let lock = r.base.join(".pack-apply.lock");
    let f = m10_sweep(&job(&r, &d, ApplyForm::Exchange, NOW, &lock)).unwrap();
    assert!(f.iter().any(|x| x.rel == rel && x.kind == "line-endings-only" && !x.notice), "{f:?}");
    assert_eq!(*checked.borrow(), 0, "never checked");
    assert_eq!(std::fs::read(r.pack.join(rel)).unwrap(), b"#!/bin/bash\r\necho ok\r\n", "never written");
}

/// CT9 (major 14): a put-back's finding survives a crash between the exchange and the findings
/// write — the next scan rebuilds it from the completed operation record, naming the kept copy;
/// a further scan adds nothing (idempotent). Without CT9 the restored file was skipped and the
/// kept bytes had no finding.
#[cfg(target_os = "macos")]
#[test]
fn a_put_back_finding_survives_a_crash_before_it_is_written() {
    let Some(rel) = listed() else {
        return;
    };
    let r = rig("ct9find");
    r.put(rel, b"#!/bin/bash\nbroken(\n", 0o755);
    let fails = |x: &str, _: &Path, _: Duration| if x == rel { outcome(CheckResult::Fail("bad".into()), 5) } else { outcome(CheckResult::Pass, 5) };
    let orig_of = |_: &str| Some(b"#!/bin/bash\necho ok\n".to_vec());
    let d = Doubles { check: &fails, reference: &|_, _| outcome(CheckResult::Pass, 1), original: &orig_of, intake: &|_, _| true, excluded: &|_| false };
    let lock = r.base.join(".pack-apply.lock");
    let first = m10_sweep(&job(&r, &d, ApplyForm::Exchange, NOW, &lock)).unwrap();
    assert!(first.iter().any(|x| x.kind == "reverted"));
    // the crash: the findings write never happened
    std::fs::remove_file(findings_path_in(&r.root, &r.lane)).unwrap();
    let again = m10_sweep(&job(&r, &d, ApplyForm::Exchange, NOW + 3600.0, &lock)).unwrap();
    let rebuilt = again.iter().find(|x| x.rel == rel && x.kind == "reverted").expect("rebuilt from the operation record");
    let kept = rebuilt.kept_copy.clone().expect("names the kept copy");
    assert_eq!(std::fs::read(&kept).unwrap(), b"#!/bin/bash\nbroken(\n");
    assert!(rebuilt.note.contains("cys repair begin --files"), "{}", rebuilt.note);
    let third = m10_sweep(&job(&r, &d, ApplyForm::Exchange, NOW + 7200.0, &lock)).unwrap();
    assert!(!third.iter().any(|x| x.kind == "reverted"), "idempotent: {third:?}");
}

// ───────────────────────────── RULINGS CU1 — one bad record never stops the job ─────────────────────────────

/// CU1: one unreadable (corrupt) record among three repairs in probation — the other two are
/// re-checked, the scan runs, the job state is written; the bad repair is filed as ONE intake row
/// (not again on the next run). Without CU1 the whole tick aborted every 2 minutes.
#[cfg(target_os = "macos")]
#[test]
fn one_unreadable_record_never_stops_the_job() {
    let r = rig("cu1corrupt");
    for (n, rel) in [(1, "hooks/c1.sh"), (2, "hooks/c2.sh"), (3, "hooks/c3.sh")] {
        committed_general(&r, &rid(&format!("R-{n}")), rel, Some(b"PRE\n"), Some(b"POST\n"));
    }
    std::fs::write(ledger::record_path_in(&r.root, &rid("R-2")), b"{ not json").unwrap();
    let checked = RefCell::new(Vec::new());
    let check = |rel: &str, _: &Path, _: Duration| {
        checked.borrow_mut().push(rel.to_string());
        outcome(CheckResult::Pass, 5)
    };
    let filed = RefCell::new(Vec::new());
    let intake = |rel: &str, line: &str| {
        filed.borrow_mut().push((rel.to_string(), line.to_string()));
        true
    };
    let d = Doubles { check: &check, reference: &|_, _| outcome(CheckResult::Pass, 1), original: &|_| None, intake: &intake, excluded: &|_| false };
    let lock = r.base.join(".pack-apply.lock");
    let rep = run_tick(&job(&r, &d, ApplyForm::Exchange, NOW, &lock), true).unwrap();
    assert!(checked.borrow().contains(&"hooks/c1.sh".to_string()) && checked.borrow().contains(&"hooks/c3.sh".to_string()), "{:?}", checked.borrow());
    assert!(read_job_state_in(&r.root, &r.lane).is_some(), "the job state is written");
    // EJ3 / EN4: one row for the repair's fault, one for the cause of the suspended scan
    assert_eq!(filed.borrow().iter().filter(|(_, l)| l.contains("처리하지 못함")).count(), 1, "one fault row: {:?}", filed.borrow());
    assert!(rep.lines.iter().any(|l| l.starts_with("R-2:")), "{:?}", rep.lines);
    // the next probation run: logged again, filed no more
    let _ = run_tick(&job(&r, &d, ApplyForm::Exchange, NOW + 700.0, &lock), false).unwrap();
    assert_eq!(filed.borrow().iter().filter(|(_, l)| l.contains("처리하지 못함")).count(), 1, "filed once per repair");
}

/// CU1: a failing re-check whose revert errors every time ends in a truthful FAILED state — the row
/// 충돌 with its probation closed `실패 — …` — and is never closed 사용 중 at 24 hours. Without CU1
/// the error aborted the tick every 2 minutes and the repair ended 사용 중.
#[cfg(target_os = "macos")]
#[test]
fn a_revert_that_errors_every_time_ends_failed() {
    let r = rig("cu1revert");
    let a = rid("R-1");
    let rec = committed_general(&r, &a, "hooks/e.sh", Some(b"PRE\n"), Some(b"POST\n"));
    // the pre-image copy is damaged: every revert errors
    if let crate::repair::ledger::PreImage::Slot { slot_dir, .. } = &rec.files[0].pre {
        let p = crate::repair::vault::Slot::at(slot_dir, "hooks/e.sh").payload();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
        std::fs::write(&p, b"DAMAGED").unwrap();
    }
    let fails = |_: &str, _: &Path, _: Duration| outcome(CheckResult::Fail("bad".into()), 5);
    let d = Doubles { check: &fails, reference: &|_, _| outcome(CheckResult::Pass, 1), original: &|_| None, intake: &|_, _| true, excluded: &|_| false };
    let lock = r.base.join(".pack-apply.lock");
    let rep = run_tick(&job(&r, &d, ApplyForm::Exchange, NOW, &lock), false).unwrap();
    assert!(rep.reported_only.iter().any(|(i, _)| *i == a), "{rep:?}");
    let row = ledger::read_lane_in(&r.root, &a, &r.lane).unwrap().unwrap();
    assert_eq!(row.state, crate::repair::types::LaneState::Conflict);
    assert!(row.probation.as_ref().unwrap().closed.as_deref().unwrap().starts_with("실패"));
    let rep = run_tick(&job(&r, &d, ApplyForm::Exchange, NOW + 2.0 * 86_400.0, &lock), false).unwrap();
    assert!(!rep.ended.contains(&a), "never 사용 중");
    assert_ne!(ledger::read_record_in(&r.root, &a).unwrap().unwrap().state, RepairState::InUse);
}

// ───────────────────────────── RULINGS CU2 — never less than what the owner approved ─────────────────────────────

/// CU2: an approved repair of this generation whose probation was closed SUPERSEDED (a later change
/// of the live file) still defines the right bytes: when the file is then broken, the scan puts back
/// the owner-APPROVED copy, never the vendor original; and when the approved copy cannot be verified
/// NOTHING is put back for the file and a finding is recorded for the worker. Without CU2 the scan
/// ignored the superseded repair and restored the vendor original (re-introducing the fixed bug).
#[cfg(target_os = "macos")]
#[test]
fn the_scan_never_puts_back_less_than_the_approved_copy() {
    let Some(rel) = listed() else {
        return;
    };
    for tamper in [false, true] {
        let r = rig("cu2approved");
        let id = rid("R-21");
        crate::repair::apply::tests::approved_and_applied(&r, &id, rel, b"#!/bin/bash\necho vendor\n", b"#!/bin/bash\necho approved\n");
        // the probation was closed superseded by a later change of the live file
        let mut row = ledger::read_lane_in(&r.root, &id, &r.lane).unwrap().unwrap();
        row.probation.as_mut().unwrap().closed = Some("대체됨 — 살아 있는 파일이 바뀜".into());
        ledger::write_lane_in(&r.root, &row).unwrap();
        let mut rec = ledger::read_record_in(&r.root, &id).unwrap().unwrap();
        rec.state = RepairState::Superseded;
        ledger::write_record_in(&r.root, &rec).unwrap();
        if tamper {
            let (ops, _) = crate::repair::wal::list_in(&r.root, &id);
            let op = ops.iter().find(|o| o.incoming_copy.is_some()).unwrap();
            let p = crate::repair::vault::Slot::at(op.incoming_copy.as_ref().unwrap(), rel).payload();
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
            std::fs::write(&p, b"TAMPERED").unwrap();
        }
        r.put(rel, b"#!/bin/bash\nbroken(\n", 0o755);
        let fails = |x: &str, _: &Path, _: Duration| if x == rel { outcome(CheckResult::Fail("bad".into()), 5) } else { outcome(CheckResult::Pass, 5) };
        let orig_of = |_: &str| Some(b"#!/bin/bash\necho vendor\n".to_vec());
        let d = Doubles { check: &fails, reference: &|_, _| outcome(CheckResult::Pass, 1), original: &orig_of, intake: &|_, _| true, excluded: &|_| false };
        let lock = r.base.join(".pack-apply.lock");
        let f = m10_sweep(&job(&r, &d, ApplyForm::Exchange, NOW, &lock)).unwrap();
        let live = std::fs::read(r.pack.join(rel)).unwrap();
        if tamper {
            assert_eq!(live, b"#!/bin/bash\nbroken(\n", "nothing put back when the approved copy cannot be verified");
            // DR2 (old expectation of this CU2 test: kind "approved-copy-unverifiable"): UNKNOWN
            assert!(f.iter().any(|x| x.kind == "approval-unknown"), "{f:?}");
        } else {
            assert_eq!(live, b"#!/bin/bash\necho approved\n", "the APPROVED copy, never the vendor original");
        }
    }
}

/// CU4: a boot-critical file replaced by a symbolic link is a FINDING (noticed once), never skipped
/// silently; nothing is followed or put back.
#[cfg(unix)]
#[test]
fn a_boot_critical_symlink_is_a_finding() {
    let Some(rel) = listed() else {
        return;
    };
    let r = rig("cu4link");
    let p = r.pack.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    let _ = std::fs::remove_file(&p);
    std::os::unix::fs::symlink("/nonexistent/elsewhere", &p).unwrap();
    let d = Doubles { check: &|_, _, _| outcome(CheckResult::Pass, 1), reference: &|_, _| outcome(CheckResult::Pass, 1), original: &|_| Some(b"x".to_vec()), intake: &|_, _| true, excluded: &|_| false };
    let lock = r.base.join(".pack-apply.lock");
    let f = m10_sweep(&job(&r, &d, ApplyForm::Exchange, NOW, &lock)).unwrap();
    assert!(f.iter().any(|x| x.rel == rel && x.kind == "not-a-file" && x.notice), "{f:?}");
    assert!(std::fs::symlink_metadata(&p).unwrap().file_type().is_symlink(), "left as it is");
    let again = m10_sweep(&job(&r, &d, ApplyForm::Exchange, NOW + 3600.0, &lock)).unwrap();
    assert!(!again.iter().any(|x| x.kind == "not-a-file"), "noticed once");
}

// ───────────────────────────── RULINGS DR5 — "reported" after the report was accepted ─────────────────────────────

/// DR5: a fault report refused by the intake (over quota / an error) stays PENDING and is retried at
/// the next tick — once per tick — and is marked filed only when the intake acknowledges it; after
/// that it is never sent again. Without DR5 the first unacknowledged attempt marked it reported for
/// good and the report was lost.
#[cfg(target_os = "macos")]
#[test]
fn a_report_is_marked_filed_only_after_the_intake_accepted_it() {
    let r = rig("dr5intake");
    committed_general(&r, &rid("R-1"), "hooks/k1.sh", Some(b"PRE\n"), Some(b"POST\n"));
    std::fs::write(ledger::record_path_in(&r.root, &rid("R-1")), b"{ broken").unwrap();
    let accept = std::cell::Cell::new(false);
    let calls = std::cell::Cell::new(0);
    // (EJ3: the unreadable record also suspends the scan — its ONE cause row is a different duty;
    // only this repair's fault attempts are counted)
    let intake = |_: &str, line: &str| {
        if line.contains("처리하지 못함") {
            calls.set(calls.get() + 1);
        }
        accept.get() || !line.contains("처리하지 못함")
    };
    let d = Doubles { check: &|_, _, _| outcome(CheckResult::Pass, 1), reference: &|_, _| outcome(CheckResult::Pass, 1), original: &|_| None, intake: &intake, excluded: &|_| false };
    let lock = r.base.join(".pack-apply.lock");
    run_tick(&job(&r, &d, ApplyForm::Exchange, NOW, &lock), false).unwrap();
    assert_eq!(calls.get(), 1, "tried once in the tick");
    run_tick(&job(&r, &d, ApplyForm::Exchange, NOW + 700.0, &lock), false).unwrap();
    assert_eq!(calls.get(), 2, "still pending → retried at the next tick, once");
    accept.set(true);
    run_tick(&job(&r, &d, ApplyForm::Exchange, NOW + 1400.0, &lock), false).unwrap();
    assert_eq!(calls.get(), 3, "accepted now");
    run_tick(&job(&r, &d, ApplyForm::Exchange, NOW + 2100.0, &lock), false).unwrap();
    assert_eq!(calls.get(), 3, "filed: never sent again");
}

// ───────────────────────────── RULINGS DR6 — a finding is notified once, for ever ─────────────────────────────

/// EJ3 / DR6 (REWRITTEN — old version: 501 operations with hand-written `acks/<op>.json` files, three
/// sweeps return nothing, one unacknowledged operation rebuilt once). Now through the PRODUCTION path:
/// 501 put-back operations completed by `wal::finish` (which writes `notice: due` in that same write),
/// the sweep's view rebuilt, the notices consumed by `take_notices_in` (which marks each operation
/// `delivered`): all 501 are delivered exactly once — none lost to the 500-row window, none repeated.
#[test]
fn a_finding_is_notified_once_for_ever() {
    let r = rig("ej3notice");
    let sub = m10_root_in(&r.root, &r.lane);
    for i in 0..501 {
        let mut op = crate::repair::wal::OpRecord {
            schema: 1,
            op: format!("m10-{i:04}"),
            repair: m10_bucket(),
            lane_key: r.lane.clone(),
            kind: "m10-revert".into(),
            form: "exchange".into(),
            rel: "hooks/z.sh".into(),
            pre_existed: true,
            expected: format!("file:{:064x}:0755", i),
            incoming: format!("file:{}:0755", "a".repeat(64)),
            pack_generation: None,
            slot: Some(sub.join("displaced/R-1").join(format!("slot-{i}"))),
            slot_identity: None,
            live_identity: None,
            incoming_copy: None,
            incoming_copy_identity: None,
            step: "sealed".into(),
            hooks: Vec::new(),
            finished: false,
            outcome: None,
            note: String::new(),
            created_at: 1.0,
            updated_at: 1.0,
            notice: None,
        };
        crate::repair::wal::finish(&sub, &mut op, "applied", "", 2.0).unwrap();
        assert_eq!(op.notice.as_deref(), Some("due"), "the completing write carries the duty");
    }
    let d = Doubles { check: &|_, _, _| outcome(CheckResult::Pass, 1), reference: &|_, _| outcome(CheckResult::Pass, 1), original: &|_| None, intake: &|_, _| true, excluded: &|_| true };
    let lock = r.base.join(".pack-apply.lock");
    let _view = m10_sweep(&job(&r, &d, ApplyForm::Exchange, NOW, &lock)).unwrap();
    let delivered = take_notices_in(&r.root, &r.lane).unwrap();
    assert_eq!(delivered.iter().filter(|f| f.kind == "reverted").count(), 501, "every put-back delivered once");
    assert!(take_notices_in(&r.root, &r.lane).unwrap().is_empty(), "never twice");
    for k in 1..3 {
        let f = m10_sweep(&job(&r, &d, ApplyForm::Exchange, NOW + 3600.0 * k as f64, &lock)).unwrap();
        assert!(f.is_empty(), "sweep {k}: a delivered finding came back: {}", f.len());
    }
}

/// RULINGS FG2 / FI3 (codex r4 M3): a finding's notice and a put-back's notice are acknowledged in ONE
/// write — when the operation log cannot be written (its folder read-only) both are still delivered
/// together; when the acknowledgement itself fails (the M10 folder read-only) BOTH stay due. Without
/// FG2 the findings were marked told first and then each operation in a loop: the operation's write
/// failed, the call returned an error, and the finding was lost (the next call carried only the
/// operation).
#[cfg(unix)]
#[test]
fn notices_are_acknowledged_all_together_or_not_at_all() {
    use std::os::unix::fs::PermissionsExt;
    let r = rig("fg2acks");
    let sub = m10_root_in(&r.root, &r.lane);
    let mut op = crate::repair::wal::OpRecord {
        schema: 1,
        op: "m10-fg2".into(),
        repair: m10_bucket(),
        lane_key: r.lane.clone(),
        kind: "m10-revert".into(),
        form: "exchange".into(),
        rel: "hooks/z.sh".into(),
        pre_existed: true,
        expected: format!("file:{:064x}:0755", 7),
        incoming: format!("file:{}:0755", "a".repeat(64)),
        pack_generation: None,
        slot: Some(sub.join("displaced/R-1").join("slot-fg2")),
        slot_identity: None,
        live_identity: None,
        incoming_copy: None,
        incoming_copy_identity: None,
        step: "sealed".into(),
        hooks: Vec::new(),
        finished: false,
        outcome: None,
        note: String::new(),
        created_at: 1.0,
        updated_at: 1.0,
        notice: None,
    };
    crate::repair::wal::finish(&sub, &mut op, "applied", "", 2.0).unwrap();
    save_findings(&r.root, &r.lane, &[Finding {
        rel: "hooks/x.sh".into(),
        sha256: "ab".into(),
        kind: "kept".into(),
        at: 1.0,
        notice: true,
        notified: false,
        slot: None,
        kept_copy: None,
        note: "n".into(),
    }])
    .unwrap();
    let m10 = findings_path_in(&r.root, &r.lane).parent().unwrap().to_path_buf();
    // the acknowledgement fails: both stay due
    std::fs::set_permissions(&m10, std::fs::Permissions::from_mode(0o555)).unwrap();
    let res = take_notices_in(&r.root, &r.lane);
    std::fs::set_permissions(&m10, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(res.is_err(), "the acknowledgement failed");
    assert_eq!(due_notices_in(&r.root, &r.lane).unwrap().len(), 2, "both still due");
    // the operation log cannot be written: both delivered together (the operation is not rewritten)
    let ops = crate::repair::wal::ops_dir_in(&sub, &m10_bucket());
    std::fs::set_permissions(&ops, std::fs::Permissions::from_mode(0o555)).unwrap();
    let got = take_notices_in(&r.root, &r.lane);
    std::fs::set_permissions(&ops, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(got.unwrap().len(), 2, "the finding and the put-back delivered together");
    assert!(due_notices_in(&r.root, &r.lane).unwrap().is_empty(), "never twice");
}

// ───────────────────────────── RULINGS DR2 — one resolution, three answers ─────────────────────────────

/// DR2: starting from a genuine applied approved repair, each member of the agreement broken ON ITS
/// OWN — the approval fingerprint, the approval reference, the deposit metadata, a corrupt record, a
/// corrupt lane row, an unreadable operation log — makes the ONE resolution answer UNKNOWN on BOTH
/// paths: recognition (the approved bytes live are not taken as approved — the check runs) and
/// restoration (a failing file: nothing is put back, neither the approved copy nor the vendor
/// original; a finding). Without DR2 the second resolver still returned the copy or fell open to
/// the vendor original.
#[cfg(target_os = "macos")]
#[test]
fn every_broken_member_makes_the_approved_copy_unknown_on_both_paths() {
    let Some(rel) = listed() else {
        return;
    };
    for member in ["fingerprint", "reference", "metadata", "record", "row", "oplog"] {
        let r = rig("dr2member");
        let id = rid("R-22");
        crate::repair::apply::tests::approved_and_applied(&r, &id, rel, b"#!/bin/bash\necho vendor\n", b"#!/bin/bash\necho approved\n");
        match member {
            "fingerprint" => {
                let mut rec = ledger::read_record_in(&r.root, &id).unwrap().unwrap();
                rec.approval.as_mut().unwrap().fingerprint_sha256 = "0".repeat(64);
                ledger::write_record_in(&r.root, &rec).unwrap();
            }
            "reference" => {
                let mut row = ledger::read_lane_in(&r.root, &id, &r.lane).unwrap().unwrap();
                row.approval_ref = Some("rqr-other".into());
                ledger::write_lane_in(&r.root, &row).unwrap();
            }
            "metadata" => {
                let (ops, _) = crate::repair::wal::list_in(&r.root, &id);
                let op = ops.iter().find(|o| o.kind == "apply" && o.outcome.as_deref() == Some("applied")).unwrap();
                let slot = crate::repair::vault::Slot::at(op.slot.as_ref().unwrap(), rel);
                let mut pre: serde_json::Value = serde_json::from_slice(&std::fs::read(slot.meta_pre()).unwrap()).unwrap();
                pre["sha256"] = serde_json::json!("f".repeat(64));
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(slot.meta_pre(), std::fs::Permissions::from_mode(0o644)).unwrap();
                std::fs::write(slot.meta_pre(), serde_json::to_vec(&pre).unwrap()).unwrap();
            }
            "record" => std::fs::write(ledger::record_path_in(&r.root, &id), b"{ corrupt").unwrap(),
            "row" => std::fs::write(ledger::lane_path_in(&r.root, &id, &r.lane), b"{ corrupt").unwrap(),
            _ => {
                let (ops, _) = crate::repair::wal::list_in(&r.root, &id);
                let op = ops.iter().find(|o| o.kind == "apply" && o.outcome.as_deref() == Some("applied")).unwrap();
                std::fs::write(crate::repair::wal::op_path_in(&r.root, &id, &op.op), b"{ unreadable").unwrap();
            }
        }
        // EN4 (REWRITTEN for the unreadable members — old: Unknown for every member): an unreadable
        // record or row makes the LEDGER incomplete → `LedgerUnreadable`, one finding for the cause
        let unreadable = matches!(member, "record" | "row");
        let answer = crate::repair::apply::resolve_approved_copy_in(&r.root, &r.pack, &r.lane, rel);
        if unreadable {
            assert!(matches!(answer, crate::repair::apply::ApprovedCopy::LedgerUnreadable { .. }), "{member}: {answer:?}");
        } else {
            assert!(matches!(answer, crate::repair::apply::ApprovedCopy::Unknown(_)), "{member}: {answer:?}");
        }
        // recognition: the approved bytes live are not "approved" — the check runs
        assert!(!crate::repair::apply::approved_copy_of_current_generation_in(&r.root, &r.pack, &r.lane, rel).unwrap(), "{member}: recognised");
        let checked = RefCell::new(0);
        let pass = |x: &str, _: &Path, _: Duration| {
            if x == rel {
                *checked.borrow_mut() += 1;
            }
            outcome(CheckResult::Pass, 5)
        };
        let orig_of = |_: &str| Some(b"#!/bin/bash\necho vendor\n".to_vec());
        let lock = r.base.join(".pack-apply.lock");
        let d = Doubles { check: &pass, reference: &|_, _| outcome(CheckResult::Pass, 1), original: &orig_of, intake: &|_, _| true, excluded: &|_| false };
        m10_sweep(&job(&r, &d, ApplyForm::Exchange, NOW, &lock)).unwrap();
        assert!(unreadable || *checked.borrow() >= 1, "{member}: no check is skipped");
        // restoration: a failing file — nothing put back
        r.put(rel, b"#!/bin/bash\nbroken(\n", 0o755);
        let fails = |x: &str, _: &Path, _: Duration| if x == rel { outcome(CheckResult::Fail("bad".into()), 5) } else { outcome(CheckResult::Pass, 5) };
        let d = Doubles { check: &fails, reference: &|_, _| outcome(CheckResult::Pass, 1), original: &orig_of, intake: &|_, _| true, excluded: &|_| false };
        let f = m10_sweep(&job(&r, &d, ApplyForm::Exchange, NOW + 3600.0, &lock)).unwrap();
        assert_eq!(std::fs::read(r.pack.join(rel)).unwrap(), b"#!/bin/bash\nbroken(\n", "{member}: nothing put back");
        if unreadable {
            assert!(f.iter().any(|x| x.kind == "ledger-unreadable") || f.is_empty(), "{member}: {f:?}");
        } else {
            assert!(f.iter().any(|x| x.rel == rel && x.kind == "approval-unknown"), "{member}: {f:?}");
        }
    }
}

// ───────────────────────────── RULINGS DX3 — the job never waits for the lock ─────────────────────────────

/// DX3: while another apply / update holds the pack-apply lock, a failing re-check does NOT wait for
/// it: `recheck_one` returns at once with no return made and the probation marked deferred (the job
/// state keeps it due); once the lock is free the next tick reverts. Without DX3 the call blocked
/// until the holder let go (here up to 30 s; in production indefinitely).
#[cfg(target_os = "macos")]
#[test]
fn the_job_never_waits_for_the_pack_apply_lock() {
    let r = rig("dx3lock");
    let a = rid("R-1");
    let rec = committed_general(&r, &a, "hooks/w.sh", Some(b"PRE\n"), Some(b"POST\n"));
    let fails = |_: &str, _: &Path, _: Duration| outcome(CheckResult::Fail("bad".into()), 5);
    let d = Doubles { check: &fails, reference: &|_, _| outcome(CheckResult::Pass, 1), original: &|_| None, intake: &|_, _| true, excluded: &|_| false };
    let lock = r.base.join(".pack-apply.lock");
    let (held_tx, held_rx) = std::sync::mpsc::channel::<()>();
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let lock2 = lock.clone();
    let holder = std::thread::spawn(move || {
        crate::pack_apply_lock::with_apply_lock(&lock2, || {
            held_tx.send(()).unwrap();
            let _ = release_rx.recv_timeout(Duration::from_secs(30));
        })
        .unwrap();
    });
    held_rx.recv().unwrap();
    let row = ledger::read_lane_in(&r.root, &a, &r.lane).unwrap().unwrap();
    let mut report = TickReport::default();
    let t = std::time::Instant::now();
    recheck_one(&job(&r, &d, ApplyForm::Exchange, NOW, &lock), &rec, &row, &mut report).unwrap();
    assert!(t.elapsed() < Duration::from_secs(10), "returned without waiting for the lock");
    assert!(report.deferred_probation, "{:?}", report.lines);
    assert_eq!(std::fs::read(r.pack.join("hooks/w.sh")).unwrap(), b"POST\n", "nothing reverted while held");
    release_tx.send(()).unwrap();
    holder.join().unwrap();
    let rep = run_tick(&job(&r, &d, ApplyForm::Exchange, NOW + 1.0, &lock), false).unwrap();
    assert!(rep.reverted.contains(&a), "reverted at the next tick: {:?}", rep.lines);
}

// ───────────────────────────── RULINGS EN4 — an unreadable ledger suspends the put-back ─────────────────────────────

/// EN4: while a record of the lane's ledger cannot be read, NO file of the lane is put back (the
/// ledger is not complete — absence is never evidence) and the sweep records ONE finding for the
/// CAUSE, naming the unreadable record and listing the affected files, notified once — not one per
/// file, not one per tick. Without EN4 the scan answered NO APPROVAL and put the original back.
#[cfg(target_os = "macos")]
#[test]
fn an_unreadable_ledger_suspends_the_put_back_with_one_finding() {
    let Some(rel) = listed() else {
        return;
    };
    let r = rig("en4ledger");
    committed_general(&r, &rid("R-1"), "hooks/other.sh", Some(b"PRE\n"), Some(b"POST\n"));
    std::fs::write(ledger::record_path_in(&r.root, &rid("R-1")), b"{ unreadable").unwrap();
    r.put(rel, b"#!/bin/bash\nbroken(\n", 0o755);
    let fails = |x: &str, _: &Path, _: Duration| if x == rel { outcome(CheckResult::Fail("bad".into()), 5) } else { outcome(CheckResult::Pass, 5) };
    let orig_of = |_: &str| Some(b"#!/bin/bash\necho ok\n".to_vec());
    let d = Doubles { check: &fails, reference: &|_, _| outcome(CheckResult::Pass, 1), original: &orig_of, intake: &|_, _| true, excluded: &|_| false };
    let lock = r.base.join(".pack-apply.lock");
    let f = m10_sweep(&job(&r, &d, ApplyForm::Exchange, NOW, &lock)).unwrap();
    assert_eq!(std::fs::read(r.pack.join(rel)).unwrap(), b"#!/bin/bash\nbroken(\n", "nothing put back");
    let cause: Vec<_> = f.iter().filter(|x| x.kind == "ledger-unreadable").collect();
    assert_eq!(cause.len(), 1, "ONE finding for the cause: {f:?}");
    assert!(cause[0].rel.contains("R-1") && cause[0].note.contains(rel) && cause[0].notice);
    assert!(f.iter().all(|x| x.kind == "ledger-unreadable"), "no finding per file: {f:?}");
    let again = m10_sweep(&job(&r, &d, ApplyForm::Exchange, NOW + 3600.0, &lock)).unwrap();
    assert!(again.is_empty(), "not again next hour: {again:?}");
}

/// EJ3 / EN1: the duty to file a failed probation's intake row IS its closure — when the write that
/// would close it fails, the event is not final (no row, no closure) and the next tick (after a
/// "restart") closes it and files exactly ONE row; later ticks file nothing more.
#[cfg(target_os = "macos")]
#[test]
fn a_failing_closure_write_then_a_restart_files_once() {
    let r = rig("ej3closure");
    let a = rid("R-1");
    committed_general(&r, &a, "hooks/k.sh", Some(b"PRE\n"), Some(b"POST\n"));
    let fails = |_: &str, _: &Path, _: Duration| outcome(CheckResult::Fail("bad".into()), 5);
    let filed = RefCell::new(Vec::<String>::new());
    let intake = |_: &str, line: &str| {
        filed.borrow_mut().push(line.to_string());
        true
    };
    let d = Doubles { check: &fails, reference: &|_, _| outcome(CheckResult::Pass, 1), original: &|_| None, intake: &intake, excluded: &|_| true };
    let lock = r.base.join(".pack-apply.lock");
    // the closure write fails: the lane rows' folder is read-only
    let lanes_dir = ledger::lane_path_in(&r.root, &a, &r.lane).parent().unwrap().to_path_buf();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&lanes_dir, std::fs::Permissions::from_mode(0o555)).unwrap();
    let _ = run_tick(&job(&r, &d, ApplyForm::DepositFirst, NOW, &lock), false);
    std::fs::set_permissions(&lanes_dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(ledger::read_lane_in(&r.root, &a, &r.lane).unwrap().unwrap().probation.unwrap().closed.is_none(), "not final");
    assert!(!filed.borrow().iter().any(|l| l.contains("시험 기간 재검사 실패")), "no closure row before the closure");
    // the restart: the next probation tick closes it and files ONE row
    let _ = run_tick(&job(&r, &d, ApplyForm::DepositFirst, NOW + 700.0, &lock), false);
    let closed = ledger::read_lane_in(&r.root, &a, &r.lane).unwrap().unwrap().probation.unwrap().closed.unwrap();
    assert!(closed.starts_with("실패"), "{closed}");
    let n = |f: &RefCell<Vec<String>>| f.borrow().iter().filter(|l| l.contains("시험 기간 재검사 실패")).count();
    assert_eq!(n(&filed), 1);
    let _ = run_tick(&job(&r, &d, ApplyForm::DepositFirst, NOW + 1400.0, &lock), false);
    assert_eq!(n(&filed), 1, "never filed twice");
}

// ───────────────────────────── RULINGS FG4 — the write that makes an event final is durable ─────────────────────────────

/// FG4 (codex r4 M5) at the durable boundary: the lane-row writes that close a probation (`close_failed`
/// — 충돌 / 실패; `close_row` — the 24-hour close) go through the durable writer: when the full flush
/// fails the write fails AND the row on disk is NOT final (the old bytes stay); the next tick, with the
/// flush working, makes it final. Without FG4 the row went through `atomic_write_json` (no flush) — it
/// read as closed although never made durable.
#[cfg(target_os = "macos")]
#[test]
fn a_closure_whose_flush_fails_is_not_final() {
    let r = rig("fg4durable");
    let a = rid("R-1");
    committed_general(&r, &a, "hooks/d.sh", Some(b"PRE\n"), Some(b"POST\n"));
    let fails = |_: &str, _: &Path, _: Duration| outcome(CheckResult::Fail("bad".into()), 5);
    let d = Doubles { check: &fails, reference: &|_, _| outcome(CheckResult::Pass, 1), original: &|_| None, intake: &|_, _| true, excluded: &|_| true };
    let lock = r.base.join(".pack-apply.lock");
    let env = job(&r, &d, ApplyForm::DepositFirst, NOW, &lock);
    let not_final = |r: &Rig| ledger::read_lane_in(&r.root, &a, &r.lane).unwrap().unwrap().probation.unwrap().closed.is_none();
    for which in ["close_failed", "close_row"] {
        crate::repair::durable::INJECT_FLUSH_ERRNO.with(|c| c.set((libc::EIO, 1000)));
        let res = if which == "close_failed" { close_failed(&env, &a, "bad") } else { close_row(&env, &a, "사용 중", &[], None) };
        crate::repair::durable::INJECT_FLUSH_ERRNO.with(|c| c.set((0, 0)));
        assert!(res.is_err(), "{which}: the failed flush fails the write");
        assert!(not_final(&r), "{which}: the row on disk is not final");
    }
    let _ = run_tick(&job(&r, &d, ApplyForm::DepositFirst, NOW + 700.0, &lock), false);
    let closed = ledger::read_lane_in(&r.root, &a, &r.lane).unwrap().unwrap().probation.unwrap().closed.unwrap();
    assert!(closed.starts_with(crate::repair::apply::CLOSED_FAILED), "the next tick made it final: {closed}");
}

// ───────────────────────────── RULINGS FG5 — the guard is held wherever the repair is written ─────────────────────────────

/// FG5 (codex r4 M6) — the check-to-return hand-over: the re-check's measured failure is durable
/// BEFORE its guard is released for the pack-apply lock — the process stopping right at the hand-over
/// leaves the closure `실패 — 되돌리기 남음 …`; the next tick makes the owed return. Without FG5 the
/// guard was dropped first and nothing was written until after the lock: a stop there lost the
/// measured failure.
#[cfg(target_os = "macos")]
#[test]
fn a_stop_at_the_hand_over_keeps_the_measured_failure() {
    struct AtHandOver;
    impl crate::repair::apply::Faults for AtHandOver {
        fn at(&self, p: crate::repair::apply::Point, _: &Path, _: Option<&crate::repair::vault::Slot>) -> bool {
            p == crate::repair::apply::Point::RecheckHandOver
        }
    }
    let r = rig("fg5handover");
    let a = rid("R-1");
    let rec = committed_general(&r, &a, "hooks/h.sh", Some(b"PRE\n"), Some(b"POST\n"));
    let fails = |_: &str, _: &Path, _: Duration| outcome(CheckResult::Fail("bad".into()), 5);
    let d = Doubles { check: &fails, reference: &|_, _| outcome(CheckResult::Pass, 1), original: &|_| None, intake: &|_, _| true, excluded: &|_| true };
    let lock = r.base.join(".pack-apply.lock");
    let stop = AtHandOver;
    let mut env = job(&r, &d, ApplyForm::Exchange, NOW, &lock);
    env.op.faults = &stop;
    let row = ledger::read_lane_in(&r.root, &a, &r.lane).unwrap().unwrap();
    let res = recheck_one(&env, &rec, &row, &mut TickReport::default());
    assert!(format!("{res:?}").contains(crate::repair::apply::INJECTED_STOP), "{res:?}");
    let closed = ledger::read_lane_in(&r.root, &a, &r.lane).unwrap().unwrap().probation.unwrap().closed;
    assert!(closed.as_deref().is_some_and(crate::repair::apply::closure_owes_return), "durable before the hand-over: {closed:?}");
    assert_eq!(std::fs::read(r.pack.join("hooks/h.sh")).unwrap(), b"POST\n", "nothing put back yet");
    let rep = run_tick(&job(&r, &d, ApplyForm::Exchange, NOW + 700.0, &lock), false).unwrap();
    assert!(rep.reverted.contains(&a), "the owed return was made: {:?}", rep.lines);
    assert_eq!(std::fs::read(r.pack.join("hooks/h.sh")).unwrap(), b"PRE\n");
}

/// FG5 (codex r4 M6) — a REAL re-check against a supersession: while the re-check runs its check (it
/// holds the repair's guard), a newer repair's supersession of it is attempted — it TRIES the guard,
/// finds it held and defers; the re-check's failure is then persisted and its return made under the
/// guard (Reverted). Once the guard is free the supersession acts. Without FG5 the supersession wrote
/// the record `Superseded` under the mutex alone in the middle of the re-check.
#[cfg(target_os = "macos")]
#[test]
fn a_supersession_during_a_recheck_defers() {
    let r = rig("fg5supersede");
    let a = rid("R-1");
    let rec = committed_general(&r, &a, "hooks/s.sh", Some(b"PRE\n"), Some(b"POST\n"));
    let mut newer = rec.clone();
    newer.id = rid("R-9");
    let seen = RefCell::new(None);
    let check = |_: &str, _: &Path, _: Duration| {
        let res = supersede_older(&r.root, &r.lane, &newer, &no_repair_lock).unwrap();
        *seen.borrow_mut() = Some(res);
        outcome(CheckResult::Fail("bad".into()), 5)
    };
    let d = Doubles { check: &check, reference: &|_, _| outcome(CheckResult::Pass, 1), original: &|_| None, intake: &|_, _| true, excluded: &|_| true };
    let lock = r.base.join(".pack-apply.lock");
    let row = ledger::read_lane_in(&r.root, &a, &r.lane).unwrap().unwrap();
    let mut report = TickReport::default();
    recheck_one(&job(&r, &d, ApplyForm::Exchange, NOW, &lock), &rec, &row, &mut report).unwrap();
    let (done, deferred) = seen.borrow().clone().unwrap();
    assert!(done.is_empty() && deferred == vec![a.clone()], "the supersession deferred: {done:?} {deferred:?}");
    assert!(report.reverted.contains(&a), "{:?}", report.lines);
    assert_eq!(ledger::read_record_in(&r.root, &a).unwrap().unwrap().state, RepairState::Reverted);
    assert_eq!(std::fs::read(r.pack.join("hooks/s.sh")).unwrap(), b"PRE\n");
}

// ───────────────────────────── RULINGS FG3 — every terminal outcome that owes a report is found again ─────────────────────────────

/// FG3 (codex r4 M4): a PARTIAL automatic return (the file put back, the hook re-registration
/// failing) closes the row 충돌 by the daemon — a terminal outcome that owes a report. The intake
/// rejects it in the first tick; after a restart the next tick files it ONCE; never again after.
/// Without FG3 `closure_owes_intake` rejected the 충돌 closure: it was never filed.
#[cfg(target_os = "macos")]
#[test]
fn a_partial_automatic_return_is_reported_until_acknowledged() {
    let r = rig("fg3partial");
    let a = rid("R-1");
    let mut rec = committed_general(&r, &a, "hooks/p.sh", Some(b"PRE\n"), Some(b"POST\n"));
    rec.files.push(crate::repair::ledger::FileChange {
        rel: "settings:Stop:p.sh".into(),
        kind: crate::repair::ledger::ChangeKind::HookUnregister { event: "Stop".into(), script: "p.sh".into() },
        pre: PreImage::Absent,
        post: TargetState::Absent,
        diff: None,
    });
    ledger::write_record_in(&r.root, &rec).unwrap();
    let fails = |_: &str, _: &Path, _: Duration| outcome(CheckResult::Fail("bad".into()), 5);
    let accept = std::cell::Cell::new(false);
    let filed = RefCell::new(Vec::<String>::new());
    let intake = |_: &str, line: &str| {
        if accept.get() {
            filed.borrow_mut().push(line.to_string());
        }
        accept.get()
    };
    let failing_hooks = |_: &RepairId, _: &LaneKey, _: HookStep| -> Result<ReconcileReport, RepairError> { Err(RepairError::Io("등록기 없음".into())) };
    let d = Doubles { check: &fails, reference: &|_, _| outcome(CheckResult::Pass, 1), original: &|_| None, intake: &intake, excluded: &|_| true };
    let lock = r.base.join(".pack-apply.lock");
    let mut env = job(&r, &d, ApplyForm::Exchange, NOW, &lock);
    env.hooks = &failing_hooks;
    let _ = run_tick(&env, false).unwrap();
    let closed = ledger::read_lane_in(&r.root, &a, &r.lane).unwrap().unwrap().probation.unwrap().closed.unwrap();
    assert!(closed.starts_with("충돌"), "a partial return: {closed}");
    assert_eq!(std::fs::read(r.pack.join("hooks/p.sh")).unwrap(), b"PRE\n", "the file was put back");
    // the restart: a new job environment, the intake accepting
    accept.set(true);
    let mut env = job(&r, &d, ApplyForm::Exchange, NOW + 700.0, &lock);
    env.hooks = &failing_hooks;
    let _ = run_tick(&env, false).unwrap();
    let n = || filed.borrow().iter().filter(|l| l.contains("충돌")).count();
    assert_eq!(n(), 1, "filed once after the restart: {:?}", filed.borrow());
    let mut env = job(&r, &d, ApplyForm::Exchange, NOW + 1400.0, &lock);
    env.hooks = &failing_hooks;
    let _ = run_tick(&env, false).unwrap();
    assert_eq!(n(), 1, "never twice");
}

/// FG3 (codex r4 M4): a hook operation that FINISHED declined (its step's terminal error) owes a
/// report found again at every tick from its record — rejected by the intake, then a restart: filed
/// ONCE. Without FG3 recovery selected only unfinished records and nothing found it again.
#[test]
fn a_terminal_hook_error_is_reported_until_acknowledged() {
    let r = rig("fg3hook");
    let a = rid("R-1");
    committed_general(&r, &a, "hooks/k.sh", Some(b"PRE\n"), Some(b"POST\n"));
    let op = crate::repair::wal::OpRecord {
        schema: 1,
        op: "20261114-hook-1".into(),
        repair: a.clone(),
        lane_key: r.lane.clone(),
        kind: "hook-reregister".into(),
        form: "hook".into(),
        rel: "settings".into(),
        pre_existed: true,
        expected: String::new(),
        incoming: String::new(),
        pack_generation: None,
        slot: None,
        slot_identity: None,
        live_identity: None,
        incoming_copy: None,
        incoming_copy_identity: None,
        step: "tombstoned".into(),
        hooks: Vec::new(),
        finished: true,
        outcome: Some("declined".into()),
        note: "등록기가 실패했습니다".into(),
        created_at: 1.0,
        updated_at: 2.0,
        notice: None,
    };
    crate::repair::wal::write_in(&r.root, &op).unwrap();
    let pass = |_: &str, _: &Path, _: Duration| outcome(CheckResult::Pass, 5);
    let accept = std::cell::Cell::new(false);
    let filed = RefCell::new(Vec::<String>::new());
    let intake = |_: &str, line: &str| {
        if accept.get() {
            filed.borrow_mut().push(line.to_string());
        }
        accept.get()
    };
    let d = Doubles { check: &pass, reference: &|_, _| outcome(CheckResult::Pass, 1), original: &|_| None, intake: &intake, excluded: &|_| true };
    let lock = r.base.join(".pack-apply.lock");
    let _ = run_tick(&job(&r, &d, ApplyForm::Exchange, NOW, &lock), false).unwrap();
    accept.set(true);
    let _ = run_tick(&job(&r, &d, ApplyForm::Exchange, NOW + 700.0, &lock), false).unwrap();
    let n = || filed.borrow().iter().filter(|l| l.contains("훅 단계가 끝나지 못했습니다")).count();
    assert_eq!(n(), 1, "filed once after the restart: {:?}", filed.borrow());
    let _ = run_tick(&job(&r, &d, ApplyForm::Exchange, NOW + 1400.0, &lock), false).unwrap();
    assert_eq!(n(), 1, "never twice");
}

/// FI4: the M10 sub-ledger's recovery errors are acknowledged under `m10:<lane>:recovery-error` — a
/// real repair R-1's recovery error in the same tick is its own report. Without FI4 both were filed
/// under `R-1:recovery-error`: the second was never filed.
#[test]
fn m10_recovery_errors_have_their_own_key() {
    let r = rig("fi4key");
    let a = rid("R-1");
    committed_general(&r, &a, "hooks/m.sh", Some(b"PRE\n"), Some(b"POST\n"));
    let real = crate::repair::wal::op_path_in(&r.root, &a, "broken-op");
    std::fs::create_dir_all(real.parent().unwrap()).unwrap();
    std::fs::write(&real, b"{ broken").unwrap();
    let sub = m10_root_in(&r.root, &r.lane);
    let m10 = crate::repair::wal::op_path_in(&sub, &m10_bucket(), "broken-m10");
    std::fs::create_dir_all(m10.parent().unwrap()).unwrap();
    std::fs::write(&m10, b"{ broken").unwrap();
    let pass = |_: &str, _: &Path, _: Duration| outcome(CheckResult::Pass, 5);
    let d = Doubles { check: &pass, reference: &|_, _| outcome(CheckResult::Pass, 1), original: &|_| None, intake: &|_, _| true, excluded: &|_| true };
    let lock = r.base.join(".pack-apply.lock");
    let _ = run_tick(&job(&r, &d, ApplyForm::Exchange, NOW, &lock), false).unwrap();
    let acks = load_acks_checked(&r.root, &r.lane).unwrap();
    assert!(acks.contains_key(&format!("m10:{}:recovery-error", r.lane.0)), "{acks:?}");
    assert!(acks.contains_key("R-1:recovery-error"), "the real repair's own report: {acks:?}");
}

// ───────────────────────────── RULINGS FG1 — the job's own reads ─────────────────────────────

/// FG1 (codex r4 B2) in the job: an unlistable repairs root is never "no repairs" — the probation
/// pass stops with ONE report (`ledger:<lane>:repairs-root`) and stays due; an unreadable
/// acknowledgement file is said, the report is still filed, and nothing is written over that file.
/// Without FG1 the pass listed nothing and marked the probation done, and the acknowledgement file
/// read as empty was overwritten.
#[test]
fn an_unlistable_root_and_unreadable_acknowledgements_are_reported() {
    let r = rig("fg1job");
    let a = rid("R-1");
    committed_general(&r, &a, "hooks/j.sh", Some(b"PRE\n"), Some(b"POST\n"));
    let pass = |_: &str, _: &Path, _: Duration| outcome(CheckResult::Pass, 5);
    let d = Doubles { check: &pass, reference: &|_, _| outcome(CheckResult::Pass, 1), original: &|_| None, intake: &|_, _| true, excluded: &|_| true };
    let lock = r.base.join(".pack-apply.lock");
    crate::repair::ledger::FAIL_LIST_IDS.with(|c| c.set(true));
    let rep = run_tick(&job(&r, &d, ApplyForm::Exchange, NOW, &lock), false);
    crate::repair::ledger::FAIL_LIST_IDS.with(|c| c.set(false));
    let rep = rep.unwrap();
    assert!(rep.deferred_probation, "the probation stays due: {:?}", rep.lines);
    let acks = load_acks_checked(&r.root, &r.lane).unwrap();
    assert!(acks.contains_key(&format!("ledger:{}:repairs-root", r.lane.0)), "{acks:?}");
    // the acknowledgement file corrupt: a new duty is filed and the file is left as it is
    let p = acks_path_in(&r.root, &r.lane);
    std::fs::write(&p, b"{ broken").unwrap();
    let filed = RefCell::new(0u32);
    let intake = |_: &str, _: &str| {
        *filed.borrow_mut() += 1;
        true
    };
    let fails = |_: &str, _: &Path, _: Duration| outcome(CheckResult::Fail("bad".into()), 5);
    let d = Doubles { check: &fails, reference: &|_, _| outcome(CheckResult::Pass, 1), original: &|_| None, intake: &intake, excluded: &|_| true };
    let rep = run_tick(&job(&r, &d, ApplyForm::DepositFirst, NOW + 700.0, &lock), false).unwrap();
    assert!(*filed.borrow() >= 1, "filed: {:?}", rep.lines);
    assert_eq!(std::fs::read(&p).unwrap(), b"{ broken", "nothing written over it");
    assert!(rep.lines.iter().any(|l| l.contains("접수 확인 기록을 읽을 수 없습니다")), "{:?}", rep.lines);
}

// ───────────────────────────── RULINGS FG1 — the scan never takes "could not read" for "absent" ─────────────────────────────

/// FG1: a boot-critical file the scan cannot read is a FINDING (told once), never skipped as absent;
/// nothing is put back. Without FG1 `let Ok(bytes) = read(..) else continue` skipped it silently.
#[cfg(unix)]
#[test]
fn an_unreadable_boot_critical_file_is_a_finding() {
    let Some(rel) = listed() else {
        return;
    };
    let r = rig("fg1m10read");
    r.put(rel, b"#!/bin/bash\nchanged\n", 0o755);
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(r.pack.join(rel), std::fs::Permissions::from_mode(0o000)).unwrap();
    let d = Doubles { check: &|_, _, _| outcome(CheckResult::Pass, 1), reference: &|_, _| outcome(CheckResult::Pass, 1), original: &|_| None, intake: &|_, _| true, excluded: &|x| x != rel };
    let lock = r.base.join(".pack-apply.lock");
    let found = m10_sweep(&job(&r, &d, ApplyForm::Exchange, NOW, &lock)).unwrap();
    std::fs::set_permissions(r.pack.join(rel), std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(found.iter().any(|f| f.rel == rel && f.kind == "unreadable" && f.notice), "{found:?}");
    let again = m10_sweep(&job(&r, &d, ApplyForm::Exchange, NOW + 3600.0, &lock));
    assert!(again.is_ok());
}

// ───────────────────────────── RULINGS EK2 — a failed re-check is a fact from the moment it is measured ─────────────────────────────

/// EK2: a re-check fails while another apply / update holds the pack-apply lock — the failure is made
/// durable IN THAT TICK (the row closed `실패 — 되돌리기 남음 …`); the next tick, past the 24-hour mark
/// and with the lock free, performs the owed return — never closes it 사용 중. Without EK2 the
/// deferred failure lived only in memory and the 24-hour close said 사용 중.
#[cfg(target_os = "macos")]
#[test]
fn a_deferred_failed_recheck_is_never_closed_in_use() {
    let r = rig("ek2owed");
    let a = rid("R-1");
    let rec = committed_general(&r, &a, "hooks/e2.sh", Some(b"PRE\n"), Some(b"POST\n"));
    let fails = |_: &str, _: &Path, _: Duration| outcome(CheckResult::Fail("bad".into()), 5);
    let d = Doubles { check: &fails, reference: &|_, _| outcome(CheckResult::Pass, 1), original: &|_| None, intake: &|_, _| true, excluded: &|_| true };
    let lock = r.base.join(".pack-apply.lock");
    let (held_tx, held_rx) = std::sync::mpsc::channel::<()>();
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let lock2 = lock.clone();
    let holder = std::thread::spawn(move || {
        crate::pack_apply_lock::with_apply_lock(&lock2, || {
            held_tx.send(()).unwrap();
            let _ = release_rx.recv();
        })
        .unwrap();
    });
    held_rx.recv().unwrap();
    let row = ledger::read_lane_in(&r.root, &a, &r.lane).unwrap().unwrap();
    let mut report = TickReport::default();
    recheck_one(&job(&r, &d, ApplyForm::Exchange, NOW, &lock), &rec, &row, &mut report).unwrap();
    let closed = ledger::read_lane_in(&r.root, &a, &r.lane).unwrap().unwrap().probation.unwrap().closed.unwrap();
    assert!(crate::repair::apply::closure_owes_return(&closed), "the failure is durable in that tick: {closed}");
    release_tx.send(()).unwrap();
    holder.join().unwrap();
    let rep = run_tick(&job(&r, &d, ApplyForm::Exchange, NOW + 2.0 * 86_400.0, &lock), false).unwrap();
    assert!(rep.reverted.contains(&a), "the owed return was made: {:?}", rep.lines);
    assert!(!rep.ended.contains(&a), "never 사용 중");
    assert_eq!(ledger::read_record_in(&r.root, &a).unwrap().unwrap().state, RepairState::Reverted);
}

/// FG1 (codex r4 B1) through M10: with the pack's state file corrupt the generation is Unknown — the
/// scan puts nothing back (its binding at sampling is Unknown) and the binary's embedded originals
/// are not offered as this generation's.
#[cfg(target_os = "macos")]
#[test]
fn a_corrupt_pack_state_suspends_the_put_back() {
    let Some(rel) = listed() else {
        return;
    };
    let r = rig("fg1m10");
    r.put(rel, b"#!/bin/bash\nbroken(\n", 0o755);
    std::fs::write(r.pack.join(crate::pack::PACK_STATE_FILE), b"{ corrupt").unwrap();
    assert!(embedded_original_for(&r.pack, rel).is_none(), "no embedded original for an unknown generation");
    let fails = |x: &str, _: &Path, _: Duration| if x == rel { outcome(CheckResult::Fail("bad".into()), 5) } else { outcome(CheckResult::Pass, 5) };
    let orig_of = |_: &str| Some(b"#!/bin/bash\necho ok\n".to_vec());
    let d = Doubles { check: &fails, reference: &|_, _| outcome(CheckResult::Pass, 1), original: &orig_of, intake: &|_, _| true, excluded: &|_| false };
    let lock = r.base.join(".pack-apply.lock");
    let _ = m10_sweep(&job(&r, &d, ApplyForm::Exchange, NOW, &lock)).unwrap();
    assert_eq!(std::fs::read(r.pack.join(rel)).unwrap(), b"#!/bin/bash\nbroken(\n", "nothing put back");
}
