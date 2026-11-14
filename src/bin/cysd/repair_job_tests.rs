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
    let v = trigger(&daemon, "tick");
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
    daemon.repair_tick_running.store(false, Ordering::SeqCst);
    let v = trigger_with(&daemon, "sweep", &root, &pack, &spawn);
    assert_eq!(v["notices"].as_array().unwrap().len(), 0, "never twice: {v}");
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
