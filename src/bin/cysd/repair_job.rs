//! 「스스로 해결」(0.15.0) — the daemon side of the ONE appended built-in job (track T6 · contract C7):
//! readiness (`listener_ready` + `ready_since`, set at one instant in `post_listen_boot` — O-C7-3 /
//! F8), the request side of `repair.tick` / `repair.sweep` (O-C7-2 (a): the reply comes at once —
//! `not_ready` | `busy` | `idle` | `started` — and the work runs on ONE daemon thread guarded by
//! `repair_tick_running`), and the production environment of the job (`cys::repair::probation`).
//!
//! ★Nothing here may panic (F4) and nothing here sends anything to a seat's queue, an alert route or
//! the event bus (F1): the job's results are ledger files and one daemon log line. Before the
//! listener is ready the tick is a no-op that does not consume the incarnation's first check
//! (C7 7.3 / 7.4 — appendix A 1-1, 1-42, 2-17). Single flight: a new run never starts while the
//! previous one is alive; every check is a bounded child (T5b's runner) run one at a time (7.5).

use crate::state::Daemon;
use cys::repair::types::{LaneKey, RepairId};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(1)
        .max(1)
}

/// O-C7-3 / F8: the daemon is ready — set FIRST in `post_listen_boot` (both accept loops), the time
/// before the flag so a reader that sees the flag also sees its time.
pub(crate) fn mark_ready(daemon: &Daemon) {
    daemon.ready_since.store(now_ms(), Ordering::SeqCst);
    daemon.listener_ready.store(true, Ordering::SeqCst);
}

/// Clears the single-flight claim on every exit of the job thread (a panic included).
struct FlightGuard(Arc<Daemon>);

impl Drop for FlightGuard {
    fn drop(&mut self) {
        self.0.repair_tick_running.store(false, Ordering::SeqCst);
    }
}

/// The repairs root of this daemon (the test build keeps it out of the real ledger).
pub(crate) fn job_root(daemon: &Daemon) -> PathBuf {
    #[cfg(test)]
    {
        std::env::temp_dir()
            .join(format!("cys-repair-job-test-{}", std::process::id()))
            .join(LaneKey::of_socket(&daemon.socket_path).0)
    }
    #[cfg(not(test))]
    {
        let _ = daemon;
        cys::repair::paths::repairs_root()
    }
}

/// `repair.tick` / `repair.sweep` (TRIGGER — any caller, the seatless scheduler included): answers at
/// once. `sweep` (the preflight's trigger, M10) forces the scan and carries the findings not yet
/// told to the owner — each exactly once.
pub(crate) fn trigger(daemon: &Arc<Daemon>, sub: &str) -> Value {
    let root = job_root(daemon);
    trigger_with(daemon, sub, &root, &cys::pack::pack_dir(), &|d, force| {
        spawn_job(d, force)
    })
}

/// The decision of [`trigger`] with the root, the pack and the thread spawner injected (tests).
pub(crate) fn trigger_with(
    daemon: &Arc<Daemon>,
    sub: &str,
    root: &Path,
    pack: &Path,
    spawn: &dyn Fn(&Arc<Daemon>, bool) -> Result<(), String>,
) -> Value {
    let _ = pack;
    let force_sweep = sub == "sweep";
    if !daemon.listener_ready.load(Ordering::SeqCst) {
        // C7 7.3: a no-op before readiness does NOT consume the incarnation's first check
        return json!({"tick": "not_ready"});
    }
    let lane = LaneKey::of_socket(&daemon.socket_path);
    if !force_sweep {
        let state = cys::repair::probation::read_job_state_in(root, &lane);
        let due = cys::repair::probation::due(
            state.as_ref(),
            &daemon.incarnation,
            cys::repair::ledger::now_epoch(),
        );
        if !due.anything() {
            return json!({"tick": "idle"});
        }
    }
    if daemon
        .repair_tick_running
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return json!({"tick": "busy"});
    }
    let notices: Vec<Value> = if force_sweep {
        cys::repair::probation::take_notices_in(root, &lane)
            .unwrap_or_default()
            .into_iter()
            .map(|f| json!({"rel": f.rel, "kind": f.kind, "sha256": f.sha256, "kept_copy": f.kept_copy, "note": f.note, "slot": f.slot}))
            .collect()
    } else {
        Vec::new()
    };
    match spawn(daemon, force_sweep) {
        Ok(()) => json!({"tick": "started", "sweep": force_sweep, "notices": notices}),
        Err(e) => {
            daemon.repair_tick_running.store(false, Ordering::SeqCst);
            json!({"tick": "busy", "error": e})
        }
    }
}

fn spawn_job(daemon: &Arc<Daemon>, force_sweep: bool) -> Result<(), String> {
    let d = Arc::clone(daemon);
    std::thread::Builder::new()
        .name("cysd-repair-tick".into())
        .spawn(move || {
            let guard = FlightGuard(Arc::clone(&d));
            run_job(&d, force_sweep);
            drop(guard);
        })
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// The per-repair mutex of this daemon (RULINGS G6, R3) as the library's lock callback.
pub(crate) fn repair_lock<'a>(
    daemon: &'a Daemon,
) -> impl Fn(&RepairId, &mut dyn FnMut() -> Result<(), cys::repair::types::RepairError>) -> Result<(), cys::repair::types::RepairError>
       + 'a {
    move |id, f| {
        let m = {
            let mut map = daemon
                .repair_mutexes
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            Arc::clone(
                map.entry(id.as_str().to_string())
                    .or_insert_with(|| Arc::new(std::sync::Mutex::new(()))),
            )
        };
        let _g = m.lock().unwrap_or_else(|e| e.into_inner());
        f()
    }
}

/// H6(1): `schedule.json`'s own check is the scheduler's validation; other files have none here.
pub(crate) fn own_check(rel: &str, holder: &Path) -> Option<bool> {
    (rel == "schedule.json").then(|| crate::schedule::schedule_file_passes_own_check(holder))
}

/// How many intake rows of `lane` were FIRST seen within the last hour (the hourly quota's count).
pub(crate) fn new_intake_rows_in_last_hour(root: &Path, lane: &LaneKey, now: f64) -> usize {
    let dir = root.join("intake").join(&lane.0);
    std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| cys::repair::intake::load_row(&e.path()).ok().flatten())
        .filter(|row| now - row.first_seen <= 3600.0)
        .count()
}

/// One run of the job on its thread (production environment).
fn run_job(daemon: &Arc<Daemon>, force_sweep: bool) {
    let root = job_root(daemon);
    let pack = cys::pack::pack_dir();
    let lane = LaneKey::of_socket(&daemon.socket_path);
    let now = cys::repair::ledger::now_epoch();
    let windows = cfg!(windows);
    let locked = repair_lock(daemon);
    let lock_file = cys::pack_apply_lock::pack_apply_lock_path_for(&pack);
    let pack_for_original = pack.clone();
    let original = move |rel: &str| cys::repair::probation::embedded_original_for(&pack_for_original, rel);
    let excluded = |rel: &str| crate::repair::rule_document_name(rel).is_some();
    let root_for_intake = root.clone();
    let lane_for_intake = lane.clone();
    let intake = move |rel: &str, err: &str| {
        if let Ok(report) = cys::repair::intake::parse_report(rel, err) {
            // CU4: the REAL count of new rows this lane wrote in the last hour (the quota of C7 /
            // T3 — never bypassed by a constant 0), read from the rows themselves
            let new_in_last_hour = new_intake_rows_in_last_hour(&root_for_intake, &lane_for_intake, now);
            let _ = cys::repair::intake::record_in(
                &root_for_intake,
                &lane_for_intake,
                &report,
                "daemon",
                now,
                new_in_last_hour,
                windows,
                err,
            );
        }
    };
    let env = cys::repair::probation::JobEnv {
        op: cys::repair::apply::OpEnv {
            root: &root,
            pack: &pack,
            lane: &lane,
            now,
            form: cys::repair::platform::apply_form(),
            faults: &cys::repair::apply::NoFaults,
            own_check: &own_check,
        },
        incarnation: &daemon.incarnation,
        check: &cys::repair::apply::production_check,
        reference: &cys::repair::apply::production_reference,
        hooks: &cys::repair::apply::hook_lines_step,
        registrar: &cys::repair::apply::run_registrar,
        locked: &locked,
        excluded: &excluded,
        original: &original,
        intake: &intake,
        apply_lock: &lock_file,
    };
    match cys::repair::probation::run_tick(&env, force_sweep) {
        Ok(r) => {
            if !r.lines.is_empty() || !r.m10.is_empty() || r.resumed > 0 {
                eprintln!(
                    "[cysd] repair tick: {} line(s), resumed {}, reverted {}, ended {}, superseded {}, timeouts {}, M10 findings {}",
                    r.lines.len(),
                    r.resumed,
                    r.reverted.len(),
                    r.ended.len(),
                    r.superseded.len(),
                    r.timeouts.len(),
                    r.m10.len()
                );
            }
        }
        Err(e) => eprintln!("[cysd] repair tick: 이번 차례는 멈췄습니다(다음 차례에 다시) — {e:?}"),
    }
    daemon.repair_last_tick.store(now_ms(), Ordering::SeqCst);
}

#[cfg(test)]
#[path = "repair_job_tests.rs"]
mod tests;
