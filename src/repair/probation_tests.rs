//! T6 tests of the appended job's work: the tick decision, the probation re-check with RULINGS O3's
//! attribution, the end of probation (N3), the update-inside-probation binding, Windows' report-only
//! mode, the start-time pass (AK7, DEFERRED #49) and M10 (F9, notices, line endings, rule documents).

use super::*;
use crate::repair::apply::tests::{committed_general, lane_row, rid, rig, Rig};
use crate::repair::apply::{no_repair_lock, CheckOutcome, NoFaults, ReconcileReport};
use crate::repair::boot_critical::CheckKind;
use crate::repair::ledger::CheckTiming;
use std::cell::RefCell;

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
    intake: &'a dyn Fn(&str, &str),
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
    let note = |rel: &str, e: &str| intake.borrow_mut().push((rel.to_string(), e.to_string()));
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
    let d = Doubles { check: &fail, reference: &|_, _| outcome(CheckResult::Pass, 1), original: &|_| None, intake: &|_, _| {}, excluded: &|_| false };
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
        let d = Doubles { check: &fail, reference: &|_, _| outcome(CheckResult::Pass, 1), original: &|_| None, intake: &|_, _| {}, excluded: &|_| false };
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
    let d = Doubles { check: &rec, reference: &|_, _| outcome(CheckResult::Pass, 1), original: &|_| None, intake: &|_, _| {}, excluded: &|_| true };
    let lock = r.base.join(".pack-apply.lock");
    let mut report = TickReport::default();
    let env = job(&r, &d, ApplyForm::Exchange, NOW, &lock);
    probation_pass(&env, &mut report).unwrap();
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
        let d = Doubles { check: &check, reference: &reference, original: &|_| None, intake: &|_, _| {}, excluded: &|_| false };
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
        let d = Doubles { check: &check, reference: &reference, original: &|_| None, intake: &|_, _| {}, excluded: &|_| false };
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
    let d = Doubles { check: &never, reference: &|_, _| outcome(CheckResult::Pass, 1), original: &|_| None, intake: &|_, _| {}, excluded: &|_| true };
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
    let d = Doubles { check: &|_, _, _| outcome(CheckResult::Pass, 1), reference: &|_, _| outcome(CheckResult::Pass, 1), original: &|_| None, intake: &|_, _| {}, excluded: &|_| true };
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
    let d = Doubles { check: &fails, reference: &|_, _| outcome(CheckResult::Pass, 1), original: &orig_of, intake: &|_, _| {}, excluded: &|_| false };
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

/// F9 at M10: a planted ledger row does NOT stop the revert of a failing file.
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
    let d = Doubles { check: &fails, reference: &|_, _| outcome(CheckResult::Pass, 1), original: &orig_of, intake: &|_, _| {}, excluded: &|_| false };
    let lock = r.base.join(".pack-apply.lock");
    m10_sweep(&job(&r, &d, ApplyForm::Exchange, NOW, &lock)).unwrap();
    assert_eq!(std::fs::read(r.pack.join(rel)).unwrap(), orig);
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
    let d = Doubles { check: &fails, reference: &|_, _| outcome(CheckResult::Pass, 1), original: &orig_of, intake: &|_, _| {}, excluded: &|_| false };
    let lock = r.base.join(".pack-apply.lock");
    let f = m10_sweep(&job(&r, &d, ApplyForm::Exchange, NOW, &lock)).unwrap();
    assert!(f.iter().any(|x| x.kind == "superseded"), "{f:?}");
    assert_eq!(std::fs::read(r.pack.join(rel)).unwrap(), b"#!/bin/bash\nthird\n");
}

/// M10's flagged O3 reading (D-11-09 / D-11-10): a hang is blamed only when the control (the content
/// that would be put back) passed in time — within the deadline ÷ 1.5 — and the second run timed out.
#[cfg(target_os = "macos")]
#[test]
fn m10_hang_attribution() {
    let Some(rel) = listed() else {
        return;
    };
    for (control_ok, second, reverted) in [(true, CheckResult::Timeout, true), (false, CheckResult::Timeout, false), (true, CheckResult::Pass, false)] {
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
        let d = Doubles { check: &check, reference: &|_, _| outcome(CheckResult::Pass, 1), original: &orig_of, intake: &|_, _| {}, excluded: &|_| false };
        let lock = r.base.join(".pack-apply.lock");
        m10_sweep(&job(&r, &d, ApplyForm::Exchange, NOW, &lock)).unwrap();
        assert_eq!(std::fs::read(r.pack.join(rel)).unwrap() == b"#!/bin/bash\necho ok\n", reverted, "control_ok={control_ok} second={second:?}");
    }
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
    let d = Doubles { check: &check, reference: &|_, _| outcome(CheckResult::Pass, 1), original: &|_| None, intake: &|_, _| {}, excluded: &|_| true };
    let lock = r.base.join(".pack-apply.lock");
    run_tick(&job(&r, &d, ApplyForm::Exchange, NOW, &lock), false).unwrap();
    assert_eq!(*seen.borrow(), vec![("bin/plain.py".to_string(), Duration::from_secs(60))]);
    if let Some(e) = crate::repair::boot_critical::entries().iter().find(|e| e.timeout_s.is_some()) {
        let want = u64::from(crate::repair::boot_critical::clamp_check_timeout(e.timeout_s));
        assert_eq!(check_deadline(e.rel), Duration::from_secs(want));
        assert!((10..=60).contains(&want));
    }
}
