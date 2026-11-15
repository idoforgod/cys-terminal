//! Track T6 — the appended job's request side: readiness (F2 SA:62 pre-bind no-op), the restart case
//! (a new incarnation checks at once), single flight, the sweep's one-time notices, and source pins
//! on `post_listen_boot` and the boot install / journal recovery block.

use super::*;
use std::sync::atomic::AtomicU32;

fn rig(tag: &str) -> (Arc<Daemon>, PathBuf, PathBuf) {
    let daemon = crate::team_gate_tests::tmp_daemon(tag, false);
    let root = std::env::temp_dir().join(format!(
        "cys-t6-job-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let pack = root.join("pack");
    std::fs::create_dir_all(&pack).unwrap();
    (daemon, root.join("repairs"), pack)
}

fn tick(daemon: &Arc<Daemon>, root: &Path, pack: &Path, sub: &str, spawned: &AtomicU32) -> String {
    let spawn = |_: &Arc<Daemon>, _: bool| -> Result<(), String> {
        spawned.fetch_add(1, Ordering::SeqCst);
        Ok(())
    };
    trigger_with(daemon, sub, root, pack, &spawn)["tick"]
        .as_str()
        .unwrap_or("")
        .to_string()
}

/// F2 SA:62 / C7 7.3–7.4: before the listener is ready the tick is a no-op that writes nothing and
/// does NOT consume the incarnation's first check; the first ready tick of the incarnation starts the
/// run at once (the restart case), whatever the last check time.
#[test]
fn tick_before_ready_is_noop_and_keeps_first_check() {
    let (daemon, root, pack) = rig("t6-ready");
    let spawned = AtomicU32::new(0);
    assert_eq!(tick(&daemon, &root, &pack, "tick", &spawned), "not_ready");
    assert_eq!(tick(&daemon, &root, &pack, "sweep", &spawned), "not_ready");
    assert_eq!(spawned.load(Ordering::SeqCst), 0);
    assert!(!root.exists(), "a pre-bind tick wrote something");
    mark_ready(&daemon);
    assert!(daemon.ready_since.load(Ordering::SeqCst) > 0, "the time is set with the flag");
    // a job state from ANOTHER incarnation, checked one second ago → this incarnation checks at once
    let lane = LaneKey::of_socket(&daemon.socket_path);
    let p = cys::repair::probation::job_state_path_in(&root, &lane);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    let now = cys::repair::ledger::now_epoch();
    std::fs::write(
        &p,
        serde_json::json!({"schema": 1, "lane": lane.0, "last_incarnation": "an-older-one",
            "last_probation_check": now - 1.0, "last_sweep": now - 1.0, "initial_catchup_complete": false,
            "pack_instance": null, "pack_dir": null, "pack_dir_identity": null, "pack_generation": null})
        .to_string(),
    )
    .unwrap();
    assert_eq!(tick(&daemon, &root, &pack, "tick", &spawned), "started");
    assert_eq!(spawned.load(Ordering::SeqCst), 1);
    let _ = std::fs::remove_dir_all(root.parent().unwrap());
}

/// C7 7.5: single flight — while a run is alive every tick answers `busy` and nothing new starts;
/// the guard clears the flag on every exit of the job thread (a panic included); a tick with
/// nothing due is `idle` (most ticks are cheap no-ops).
#[test]
fn tick_single_flight_busy_idle_and_cleared() {
    let (daemon, root, pack) = rig("t6-flight");
    mark_ready(&daemon);
    let spawned = AtomicU32::new(0);
    assert_eq!(tick(&daemon, &root, &pack, "tick", &spawned), "started");
    assert!(daemon.repair_tick_running.load(Ordering::SeqCst));
    assert_eq!(tick(&daemon, &root, &pack, "tick", &spawned), "busy");
    assert_eq!(tick(&daemon, &root, &pack, "sweep", &spawned), "busy");
    assert_eq!(spawned.load(Ordering::SeqCst), 1);
    // the guard clears the flag even when the job thread panics
    let d2 = Arc::clone(&daemon);
    let t = std::thread::spawn(move || {
        let _g = FlightGuard(d2);
        panic!("job thread panic (test)");
    });
    assert!(t.join().is_err());
    assert!(!daemon.repair_tick_running.load(Ordering::SeqCst));
    // nothing due in this incarnation → idle, nothing started
    let lane = LaneKey::of_socket(&daemon.socket_path);
    let p = cys::repair::probation::job_state_path_in(&root, &lane);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    let now = cys::repair::ledger::now_epoch();
    std::fs::write(
        &p,
        serde_json::json!({"schema": 1, "lane": lane.0, "last_incarnation": daemon.incarnation,
            "last_probation_check": now, "last_sweep": now, "initial_catchup_complete": false,
            "pack_instance": null, "pack_dir": null, "pack_dir_identity": null, "pack_generation": null})
        .to_string(),
    )
    .unwrap();
    assert_eq!(tick(&daemon, &root, &pack, "tick", &spawned), "idle");
    assert_eq!(spawned.load(Ordering::SeqCst), 1);
    // the preflight's sweep is never idle (it forces the scan)
    assert_eq!(tick(&daemon, &root, &pack, "sweep", &spawned), "started");
    let _ = std::fs::remove_dir_all(root.parent().unwrap());
}

/// The real job thread on an empty ledger: it runs, writes the job state, clears the flag and sets
/// `repair_last_tick` — its reply came back at once.
#[test]
fn real_job_thread_runs_and_clears() {
    let (daemon, root, _pack) = rig("t6-real");
    mark_ready(&daemon);
    let v = trigger(&daemon, "tick", &serde_json::json!({}));
    assert_eq!(v["tick"], "started", "{v}");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while daemon.repair_tick_running.load(Ordering::SeqCst) {
        assert!(std::time::Instant::now() < deadline, "the job thread never finished");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(daemon.repair_last_tick.load(Ordering::SeqCst) > 0);
    let _ = std::fs::remove_dir_all(job_root(&daemon));
    let _ = std::fs::remove_dir_all(root.parent().unwrap());
}

/// The preflight's sweep reply carries each owner notice exactly once.
#[test]
fn sweep_reply_carries_notices_once() {
    let (daemon, root, pack) = rig("t6-notice");
    mark_ready(&daemon);
    let lane = LaneKey::of_socket(&daemon.socket_path);
    let f = cys::repair::probation::findings_path_in(&root, &lane);
    std::fs::create_dir_all(f.parent().unwrap()).unwrap();
    std::fs::write(
        &f,
        serde_json::json!({"schema": 1, "lane": lane.0, "findings": [
            {"rel": "hooks/x.sh", "sha256": "ab", "kind": "kept", "at": 1.0, "notice": true, "notified": false, "slot": null, "note": "n"},
            {"rel": "hooks/y.sh", "sha256": "cd", "kind": "line-endings-only", "at": 1.0, "notice": false, "notified": false, "slot": null, "note": "n"}
        ]})
        .to_string(),
    )
    .unwrap();
    let spawn = |_: &Arc<Daemon>, _: bool| -> Result<(), String> { Ok(()) };
    let v = trigger_with(&daemon, "sweep", &root, &pack, &spawn);
    assert_eq!(v["notices"].as_array().unwrap().len(), 1, "{v}");
    // GK3 (REWRITTEN — old expectation: the second sweep carried 0 notices "never twice" because the
    // first reply marked them told): the reply marks nothing; the receiver confirms the key it took,
    // and only then is the notice never carried again
    let key = v["notices"][0]["key"].as_str().unwrap().to_string();
    assert_eq!(confirm_notices(&daemon, &root, &[key])["confirmed"], 1);
    daemon.repair_tick_running.store(false, Ordering::SeqCst);
    let v = trigger_with(&daemon, "sweep", &root, &pack, &spawn);
    assert_eq!(v["notices"].as_array().unwrap().len(), 0, "never twice once confirmed: {v}");
    let _ = std::fs::remove_dir_all(root.parent().unwrap());
}

/// RULINGS FG2 / FI3 / GK3 through the real trigger path (REWRITTEN — old expectations: a failed
/// acknowledgement written by the trigger left the notices due and the fourth call carried none,
/// "acknowledged once handed over"). Now the trigger never acknowledges: a failed spawn hands out
/// nothing; a hand-out count that cannot be written (the M10 folder read-only) hands out nothing and
/// says so; every later reply carries ALL due notices until their receiver confirms them.
#[cfg(unix)]
#[test]
fn notices_survive_a_failed_spawn_and_a_failed_acknowledgement() {
    use std::os::unix::fs::PermissionsExt;
    let (daemon, root, pack) = rig("t6-fg2");
    mark_ready(&daemon);
    let lane = LaneKey::of_socket(&daemon.socket_path);
    let f = cys::repair::probation::findings_path_in(&root, &lane);
    std::fs::create_dir_all(f.parent().unwrap()).unwrap();
    std::fs::write(
        &f,
        serde_json::json!({"schema": 1, "lane": lane.0, "findings": [
            {"rel": "hooks/x.sh", "sha256": "ab", "kind": "kept", "at": 1.0, "notice": true, "notified": false, "slot": null, "note": "n"},
            {"rel": "hooks/z.sh", "sha256": "ef", "kind": "kept", "at": 1.0, "notice": true, "notified": false, "slot": null, "note": "n"}
        ]})
        .to_string(),
    )
    .unwrap();
    let fail = |_: &Arc<Daemon>, _: bool| -> Result<(), String> { Err("no thread".into()) };
    let ok = |_: &Arc<Daemon>, _: bool| -> Result<(), String> { Ok(()) };
    let v = trigger_with(&daemon, "sweep", &root, &pack, &fail);
    assert_eq!(v["tick"], "busy", "{v}");
    // the hand-out count cannot be written: nothing handed out, said
    let m10 = f.parent().unwrap().to_path_buf();
    std::fs::set_permissions(&m10, std::fs::Permissions::from_mode(0o555)).unwrap();
    let v = trigger_with(&daemon, "sweep", &root, &pack, &ok);
    std::fs::set_permissions(&m10, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(v["notices"].as_array().unwrap().len(), 0, "{v}");
    assert!(v["notices_error"].is_string(), "{v}");
    daemon.repair_tick_running.store(false, Ordering::SeqCst);
    let v = trigger_with(&daemon, "sweep", &root, &pack, &ok);
    assert_eq!(v["notices"].as_array().unwrap().len(), 2, "ALL still due: {v}");
    let keys: Vec<String> = v["notices"].as_array().unwrap().iter().map(|n| n["key"].as_str().unwrap().to_string()).collect();
    assert_eq!(confirm_notices(&daemon, &root, &keys)["confirmed"], 2);
    daemon.repair_tick_running.store(false, Ordering::SeqCst);
    let v = trigger_with(&daemon, "sweep", &root, &pack, &ok);
    assert_eq!(v["notices"].as_array().unwrap().len(), 0, "confirmed by the receiver: {v}");
    let _ = std::fs::remove_dir_all(root.parent().unwrap());
}

/// RULINGS GK3 (codex r5 M1): a stop between the reply and the receiver's confirmation leaves the
/// notices due — the reply carries them with their keys and marks nothing; and a receiver that
/// never confirms is handed the same notice at most three times: the fourth sweep hands out
/// nothing, and the notice is listed as not confirmed (shown by `cys repair status`). Without GK3
/// the trigger acknowledged the notices before its reply left the daemon.
#[test]
fn an_unconfirmed_notice_is_handed_out_three_times_then_closed() {
    let (daemon, root, pack) = rig("t6-gk3");
    mark_ready(&daemon);
    let lane = LaneKey::of_socket(&daemon.socket_path);
    let f = cys::repair::probation::findings_path_in(&root, &lane);
    std::fs::create_dir_all(f.parent().unwrap()).unwrap();
    std::fs::write(
        &f,
        serde_json::json!({"schema": 1, "lane": lane.0, "findings": [
            {"rel": "hooks/x.sh", "sha256": "ab", "kind": "kept", "at": 1.0, "notice": true, "notified": false, "slot": null, "note": "n"}
        ]})
        .to_string(),
    )
    .unwrap();
    let ok = |_: &Arc<Daemon>, _: bool| -> Result<(), String> { Ok(()) };
    for round in 1..=3 {
        let v = trigger_with(&daemon, "sweep", &root, &pack, &ok);
        assert_eq!(v["notices"].as_array().unwrap().len(), 1, "hand-out {round}: {v}");
        assert!(v["notices"][0]["key"].is_string(), "{v}");
        daemon.repair_tick_running.store(false, Ordering::SeqCst);
    }
    let v = trigger_with(&daemon, "sweep", &root, &pack, &ok);
    assert_eq!(v["notices"].as_array().unwrap().len(), 0, "the fourth sweep hands out nothing: {v}");
    let closed = cys::repair::probation::notices_not_confirmed_in(&root, &lane).unwrap();
    assert_eq!(closed, vec!["f:hooks/x.sh:ab:kept".to_string()], "listed as not confirmed");
    let _ = std::fs::remove_dir_all(root.parent().unwrap());
}

/// RULINGS FG1 / FG2 through the trigger: an unreadable notice source (the findings view corrupt) is
/// said in the reply — never an empty list that loses the notices — and nothing is written over it;
/// once readable again, the notices are carried. Without FG2 the error became an empty vector.
#[test]
fn an_unreadable_notice_source_is_said_not_emptied() {
    let (daemon, root, pack) = rig("t6-fg2bad");
    mark_ready(&daemon);
    let lane = LaneKey::of_socket(&daemon.socket_path);
    let f = cys::repair::probation::findings_path_in(&root, &lane);
    std::fs::create_dir_all(f.parent().unwrap()).unwrap();
    std::fs::write(&f, b"{ corrupt").unwrap();
    let ok = |_: &Arc<Daemon>, _: bool| -> Result<(), String> { Ok(()) };
    let v = trigger_with(&daemon, "sweep", &root, &pack, &ok);
    assert!(v["notices_error"].is_string(), "the unreadable source is said: {v}");
    assert_eq!(std::fs::read(&f).unwrap(), b"{ corrupt", "nothing written over it");
    std::fs::write(
        &f,
        serde_json::json!({"schema": 1, "lane": lane.0, "findings": [
            {"rel": "hooks/x.sh", "sha256": "ab", "kind": "kept", "at": 1.0, "notice": true, "notified": false, "slot": null, "note": "n"}
        ]})
        .to_string(),
    )
    .unwrap();
    daemon.repair_tick_running.store(false, Ordering::SeqCst);
    let v = trigger_with(&daemon, "sweep", &root, &pack, &ok);
    assert_eq!(v["notices"].as_array().unwrap().len(), 1, "{v}");
    let _ = std::fs::remove_dir_all(root.parent().unwrap());
}

/// The EP condition (T5b's watchdog, `reconcile_stale_pending`, calls the seam while HOLDING the
/// daemon's per-repair lock and the approval-table lock): `approval_consumed_in` called on THIS
/// thread inside the daemon's real per-repair lock of the repair it reads, and inside a second held
/// mutex standing for T5b's approval-table lock (that code is not in this tree), returns its answer —
/// it takes neither (std mutexes are not re-entrant: a body that took the held repair lock would never
/// return here).
#[test]
fn the_consumed_seam_answers_inside_the_repair_lock_and_the_table_lock() {
    let (daemon, root, _pack) = rig("t6-ep-locks");
    let lane = LaneKey::of_socket(&daemon.socket_path);
    let id = RepairId::parse("R-1").unwrap();
    let intents = root.join("R-1").join("intents");
    std::fs::create_dir_all(&intents).unwrap();
    std::fs::write(
        intents.join(format!("{}.json", lane.0)),
        serde_json::json!({"schema": 1, "repair": "R-1", "lane_key": lane.0, "op_tag": "t", "state": "applied",
            "targets": [], "note": "", "pack_generation": null, "approval_ref": "rqr-ep", "created_at": 1.0, "updated_at": 1.0})
        .to_string(),
    )
    .unwrap();
    let table = std::sync::Mutex::new(());
    let mut answer = None;
    repair_lock(&daemon)(&id, &mut || {
        let _table = table.lock().unwrap();
        answer = Some(cys::repair::consumed::approval_consumed_in(&root, &lane, "rqr-ep"));
        Ok(())
    })
    .unwrap();
    assert_eq!(answer, Some(cys::repair::consumed::ApprovalUse::Consumed));
    let _ = std::fs::remove_dir_all(root.parent().unwrap());
}

/// RULINGS GV2 (codex r6 M2) — a controlled interleaving, not timed: notice K has been handed out
/// twice; trigger A starts its job, and BEFORE A reserves its hand-out the job ends (the flight flag
/// clears) and trigger B runs to its end in between (B hands K out — the third time). A then hands
/// out NOTHING: no fourth hand-out, the count is exactly three (none lost). Without GV2 A chose K
/// before its job started and counted it after — K went out a fourth time.
#[test]
fn a_trigger_overtaken_by_another_never_hands_out_a_fourth_time() {
    let (daemon, root, pack) = rig("t6-gv2");
    mark_ready(&daemon);
    let lane = LaneKey::of_socket(&daemon.socket_path);
    let f = cys::repair::probation::findings_path_in(&root, &lane);
    std::fs::create_dir_all(f.parent().unwrap()).unwrap();
    std::fs::write(
        &f,
        serde_json::json!({"schema": 1, "lane": lane.0, "findings": [
            {"rel": "hooks/x.sh", "sha256": "ab", "kind": "kept", "at": 1.0, "notice": true, "notified": false, "slot": null, "note": "n"}
        ]})
        .to_string(),
    )
    .unwrap();
    let key = "f:hooks/x.sh:ab:kept";
    std::fs::write(
        cys::repair::probation::notices_handed_path_in(&root, &lane),
        serde_json::json!({"schema": 1, "lane": lane.0, "counts": {key: 2}}).to_string(),
    )
    .unwrap();
    let ok = |_: &Arc<Daemon>, _: bool| -> Result<(), String> { Ok(()) };
    let b_reply = std::cell::RefCell::new(Value::Null);
    let a_spawn = |d: &Arc<Daemon>, _: bool| -> Result<(), String> {
        // A's job ends at once (its flight flag clears) and B runs in between
        d.repair_tick_running.store(false, Ordering::SeqCst);
        *b_reply.borrow_mut() = trigger_with(d, "sweep", &root, &pack, &ok);
        Ok(())
    };
    let a = trigger_with(&daemon, "sweep", &root, &pack, &a_spawn);
    let b = b_reply.borrow().clone();
    assert_eq!(b["notices"].as_array().unwrap().len(), 1, "B hands K out the third time: {b}");
    assert_eq!(a["notices"].as_array().unwrap().len(), 0, "no fourth hand-out: {a}");
    let counts: Value = serde_json::from_slice(&std::fs::read(cys::repair::probation::notices_handed_path_in(&root, &lane)).unwrap()).unwrap();
    assert_eq!(counts["counts"][key], 3, "no lost and no extra count: {counts}");
    let _ = std::fs::remove_dir_all(root.parent().unwrap());
}

/// RULINGS GK5 through the seam: a FIFO (no writer) and a file over the cap, at a repair's
/// `record.json` and at the lane's intent, read by `approval_consumed_in` called inside the daemon's
/// real per-repair lock and a held stand-in for T5b's table lock — the answer is Unknown and the call
/// returns (a blocking read of the FIFO would never return here). Without GK5 the read of the FIFO
/// blocked for ever.
#[cfg(unix)]
#[test]
fn the_seam_never_blocks_on_a_fifo_or_an_oversized_record() {
    let (daemon, root, _pack) = rig("t6-gk5");
    let lane = LaneKey::of_socket(&daemon.socket_path);
    let id = RepairId::parse("R-1").unwrap();
    let rdir = root.join("R-1");
    let intents = rdir.join("intents");
    std::fs::create_dir_all(&intents).unwrap();
    let record = rdir.join("record.json");
    let intent = intents.join(format!("{}.json", lane.0));
    let mkfifo = |p: &Path| {
        let c = std::ffi::CString::new(p.to_str().unwrap()).unwrap();
        // SAFETY: a valid NUL-terminated path.
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o644) }, 0);
    };
    let table = std::sync::Mutex::new(());
    let ask = || {
        let mut answer = None;
        repair_lock(&daemon)(&id, &mut || {
            let _table = table.lock().unwrap();
            answer = Some(cys::repair::consumed::approval_consumed_in(&root, &lane, "rqr-gk5"));
            Ok(())
        })
        .unwrap();
        answer.unwrap()
    };
    for at in [&record, &intent] {
        mkfifo(at);
        assert!(matches!(ask(), cys::repair::consumed::ApprovalUse::Unknown(_)), "a FIFO at {}", at.display());
        std::fs::remove_file(at).unwrap();
        std::fs::File::create(at).unwrap().set_len(cys::repair::durable::RECORD_READ_CAP + 1).unwrap();
        assert!(matches!(ask(), cys::repair::consumed::ApprovalUse::Unknown(_)), "an oversized file at {}", at.display());
        std::fs::remove_file(at).unwrap();
    }
    let _ = std::fs::remove_dir_all(root.parent().unwrap());
}

/// O-C7-3 / F8 source pin: the readiness flag and its time are set FIRST in `post_listen_boot` (the
/// one function both accept loops call), before anything it spawns.
#[test]
fn listener_ready_set_in_post_listen_boot() {
    let src = include_str!("main.rs");
    let at = src.find("fn post_listen_boot(socket_path: &std::path::Path, daemon: &Arc<Daemon>) {").expect("post_listen_boot");
    let body = &src[at..at + src[at..].find("\n}\n").unwrap()];
    let ready = body.find("crate::repair_job::mark_ready(daemon);").expect("mark_ready not called");
    for later in ["spawn_auto_restore(", "prune_stale_phoenix_embed(", "spawn_deferred_agy_statusline_reconcile("] {
        assert!(ready < body.find(later).unwrap(), "{later} runs before readiness is marked");
    }
    let mark = include_str!("repair_job.rs");
    let m = &mark[mark.find("pub(crate) fn mark_ready").unwrap()..];
    let m = &m[..m.find("\n}\n").unwrap()];
    assert!(m.find("ready_since.store").unwrap() < m.find("listener_ready.store").unwrap(), "the time before the flag");
}

/// C7 7.12 / appendix A 1-3: the daemon's own boot install and boot journal recovery are unchanged
/// by T6 — the block from the journal recovery to the end of the boot install is byte-identical to
/// the base (sha256 of that text pinned).
#[test]
fn boot_install_and_journal_recovery_untouched() {
    let src = include_str!("main.rs");
    let a = src.find("    match tokio::task::spawn_blocking(cys::pack::recover_pack_journal).await {").expect("journal recovery");
    let b = src.find("    let daemon = Daemon::new(socket_path.clone());").expect("after the boot install");
    use sha2::{Digest, Sha256};
    let h: String = Sha256::digest(src[a..b].as_bytes()).iter().map(|x| format!("{x:02x}")).collect();
    assert_eq!(h, BOOT_BLOCK_SHA256, "the boot install / journal recovery block changed");
}

/// sha256 of the boot block at the base 0422affa (= the tag's block, design main.rs 1344 / 1357–1377).
const BOOT_BLOCK_SHA256: &str = "1e03bd1b901d13a46673dc3be52c93a40ce9c4a662598fd9c46bcdd35eb21fe3";

/// The job module and the trigger never write to a queue, an alert route or the event bus (F1).
#[test]
fn job_module_sends_nothing() {
    let src = include_str!("repair_job.rs");
    let prod = &src[..src.find("#[cfg(test)]\n#[path").unwrap()];
    for needle in ["bus.publish", "enqueue", "send_text", "alert_route", "inject(", "deliver"] {
        assert!(!prod.contains(needle), "repair_job.rs contains {needle:?}");
    }
}

/// CU4: the job's intake writer counts the lane's REAL new rows of the last hour (rows first seen
/// within 3600 s) — the quota is never bypassed by a constant 0.
#[test]
fn job_intake_counts_the_real_hourly_rows() {
    let root = std::env::temp_dir().join(format!("cys-cu4-intake-{}-{}", std::process::id(), now_ms()));
    let lane = cys::repair::types::LaneKey("base".into());
    let dir = root.join("intake").join(&lane.0);
    std::fs::create_dir_all(&dir).unwrap();
    let row = |sig: &str, first_seen: f64| {
        serde_json::json!({"schema": 1, "signature": sig, "lane": "base", "rel": "hooks/a.sh", "error_class": "x",
            "first_line": "x", "first_seen": first_seen, "last_seen": first_seen, "count": 1, "reporters": ["daemon"]})
    };
    let now = 1_800_000_000.0;
    for (sig, t) in [("a", now - 10.0), ("b", now - 3000.0), ("c", now - 4000.0)] {
        std::fs::write(dir.join(format!("{sig}.json")), serde_json::to_vec(&row(sig, t)).unwrap()).unwrap();
    }
    assert_eq!(new_intake_rows_in_last_hour(&root, &lane, now), 2);
    let _ = std::fs::remove_dir_all(&root);
}

/// DX5: the rels the job files its fault reports under — `repairs/<R-id>/record.json` and
/// `repairs/<R-id>/ops` — are accepted by the intake (parsed, not excluded, recorded as a new row),
/// so a fault report is never silently "acknowledged" by a refusal.
#[test]
fn the_intake_accepts_the_jobs_fault_rels() {
    let root = std::env::temp_dir().join(format!("cys-dx5-intake-{}-{}", std::process::id(), now_ms()));
    let lane = cys::repair::types::LaneKey("base".into());
    for rel in ["repairs/R-1/record.json", "repairs/R-1/ops"] {
        let err = "자체 해결 작업이 수리 R-1를 처리하지 못함 — 수리 기록을 읽을 수 없습니다";
        let report = cys::repair::intake::parse_report(rel, err).expect("parsed");
        assert!(cys::repair::intake::excluded(rel, err, false).is_none(), "{rel}: not excluded");
        let rec = cys::repair::intake::record_in(&root, &lane, &report, "daemon", 1_800_000_000.0, 0, false, err).unwrap();
        assert!(matches!(rec, cys::repair::intake::Recorded::New(_)), "{rel}: {rec:?}");
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// T6b · RULINGS JH2: the scheduler's own check of a `schedule.json` held in the vault runs inside the
/// pack-apply exclusion — it reads through the bounded reader: a FIFO (a writer offering a valid
/// schedule to every reader) or a file over the limit fails the check at once, never waited for or
/// read whole; an ordinary schedule passes. Without JH2 it was `std::fs::read`.
#[cfg(unix)]
#[test]
fn the_schedule_own_check_is_bounded_and_never_waits() {
    use std::io::Write;
    let base = std::env::temp_dir().join(format!("cys-t6b-jh2own-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let json = br#"{"jobs": []}"#;
    let ok = base.join("ok.json");
    std::fs::write(&ok, json).unwrap();
    assert!(crate::schedule::schedule_file_passes_own_check(&ok), "an ordinary schedule passes");
    let big = base.join("big.json");
    std::fs::File::create(&big).unwrap().set_len(cys::repair::durable::RECORD_READ_CAP + 1).unwrap();
    assert!(!crate::schedule::schedule_file_passes_own_check(&big), "over the limit: fails");
    let fifo = base.join("fifo.json");
    let c = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
    // SAFETY: a valid NUL-terminated path.
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o644) }, 0);
    let w_at = fifo.clone();
    // ONE reader is served (the check reads the file once): its open waits for a reader
    let writer = std::thread::spawn(move || {
        if let Ok(mut f) = std::fs::OpenOptions::new().write(true).open(&w_at) {
            let _ = f.write_all(json);
        }
    });
    let got = crate::schedule::schedule_file_passes_own_check(&fifo);
    // a writer still waiting (the bounded check never read) is let go by a reader
    let release = {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK).open(&fifo).unwrap()
    };
    writer.join().unwrap();
    drop(release);
    let _ = std::fs::remove_dir_all(&base);
    assert!(!got, "a FIFO fails the check at once");
}
