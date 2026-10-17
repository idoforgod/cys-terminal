//! R-PTY-1: use the real daemon admission function and the real vendor allocator.
//! A child test process owns the global allocator hold, so other daemon tests
//! creating PTYs cannot race with this deliberate stall. No daemon/listener or
//! shell child is created; successful pairs are dropped in their worker closure.
use super::{dispatch_admission_exempt, run_admitted};
use crate::state::HideConsole;
use portable_pty::unix::test_support;
use std::process::{Command, Stdio};
use std::sync::{mpsc, Arc};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::{oneshot, Semaphore};

const CHILD_ENV: &str = "CYS_TEST_PTY_ADMISSION_CHILD";
const CHILD_TEST: &str = "pty_admission_tests::isolated_real_allocator_admission_child";
const MARKER: &str = "@@PTY_ADMISSION ";

struct Holder {
    release: Option<mpsc::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Holder {
    fn start() -> Self {
        let (ready_tx, ready_rx) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            let _guard = test_support::hold_allocator();
            let _ = ready_tx.send(());
            let _ = wait.recv();
        });
        let holder = Self { release: Some(release), thread: Some(thread) };
        assert!(ready_rx.recv_timeout(Duration::from_secs(5)).is_ok(), "holder never acquired allocator");
        holder
    }
    fn held(&self) -> bool {
        self.release.is_some() && self.thread.as_ref().is_some_and(|thread| !thread.is_finished())
    }
}
impl Drop for Holder {
    fn drop(&mut self) {
        // Release before joining, including every assertion/unwind path.
        if let Some(release) = self.release.take() { let _ = release.send(()); }
        if let Some(thread) = self.thread.take() { let _ = thread.join(); }
    }
}

#[derive(Debug)]
struct Allocation {
    succeeded: bool,
    boxed_marker: bool,
    mapped: String,
    context_mapped: String,
    elapsed: Duration,
}

fn allocate() -> Allocation {
    let start = Instant::now();
    let result = portable_pty::native_pty_system().openpty(portable_pty::PtySize {
        rows: 24, cols: 80, pixel_width: 0, pixel_height: 0,
    });
    let elapsed = start.elapsed();
    match result {
        Ok(pair) => {
            drop(pair);
            Allocation { succeeded: true, boxed_marker: false, mapped: String::new(), context_mapped: String::new(), elapsed }
        }
        Err(error) => {
            // This is the actual anyhow::Error returned by the production
            // PtySystem wrapper and the same as_ref() conversion used by state.
            let original_error: &(dyn std::error::Error + 'static) = error.as_ref();
            let boxed_marker = original_error.downcast_ref::<portable_pty::unix::PtyAllocatorBusy>().is_some();
            let mapped = crate::state::map_pty_open_error(original_error);
            let context_mapped = crate::state::map_pty_open_error(error.context("fixture context").as_ref());
            Allocation { succeeded: false, boxed_marker, mapped, context_mapped, elapsed }
        }
    }
}

async fn waiter_seen(expected: usize) -> bool {
    let deadline = Instant::now() + Duration::from_secs(2);
    while test_support::waiter_count() < expected && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    test_support::waiter_count() >= expected
}

fn create(gate: &'static Semaphore) -> tokio::task::JoinHandle<Result<Allocation, tokio::task::JoinError>> {
    tokio::spawn(run_admitted(gate, dispatch_admission_exempt("surface.create"), allocate))
}

fn ordinary(gate: &'static Semaphore, method: &'static str, calls: Arc<AtomicUsize>)
    -> (tokio::task::JoinHandle<Result<(), tokio::task::JoinError>>, oneshot::Receiver<()>)
{
    let (tx, rx) = oneshot::channel();
    let task = tokio::spawn(run_admitted(gate, dispatch_admission_exempt(method), move || {
        calls.fetch_add(1, Ordering::SeqCst);
        let _ = tx.send(());
    }));
    (task, rx)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "parent test launches this alone to isolate the process-wide PTY allocator"]
async fn isolated_real_allocator_admission_child() {
    assert_eq!(std::env::var(CHILD_ENV).as_deref(), Ok("1"), "child must be explicitly isolated");
    assert!(!dispatch_admission_exempt("surface.create"));
    assert!(!dispatch_admission_exempt("status.set"));
    assert!(dispatch_admission_exempt("system.ping"));
    // Same admission implementation, reduced capacity for a bounded composition
    // test. This is not a 128-client or end-to-end listener stress measurement.
    let gate: &'static Semaphore = Box::leak(Box::new(Semaphore::new(2)));
    let calls = Arc::new(AtomicUsize::new(0));
    let holder = Holder::start();
    let baseline = test_support::waiter_count();
    let first = create(gate);
    let first_waited = waiter_seen(baseline + 1).await;
    let permits_with_one = gate.available_permits();
    let (spare, spare_rx) = ordinary(gate, "status.set", calls.clone());
    let spare_progress = matches!(tokio::time::timeout(Duration::from_secs(2), spare_rx).await, Ok(Ok(())));
    let second = create(gate);
    let second_waited = waiter_seen(baseline + 2).await;
    let permits_saturated = gate.available_permits();
    let (queued, mut queued_rx) = ordinary(gate, "status.set", calls.clone());
    let queued_waited = tokio::time::timeout(Duration::from_millis(50), &mut queued_rx).await.is_err();
    let (ping, ping_rx) = ordinary(gate, "system.ping", calls.clone());
    let ping_progress = matches!(tokio::time::timeout(Duration::from_secs(2), ping_rx).await, Ok(Ok(())));
    let still_held = holder.held();
    drop(holder);
    let mut joined_tasks = 0;
    let first = first.await.expect("first task joined").expect("first blocking task joined");
    joined_tasks += 1;
    let second = second.await.expect("second task joined").expect("second blocking task joined");
    joined_tasks += 1;
    spare.await.expect("spare task joined").expect("spare closure joined");
    joined_tasks += 1;
    queued.await.expect("queued task joined").expect("queued closure joined");
    joined_tasks += 1;
    ping.await.expect("ping task joined").expect("ping closure joined");
    joined_tasks += 1;
    let recovered_permits = gate.available_permits();
    let first_phase_calls = calls.load(Ordering::SeqCst);

    // Hold the same real allocator past the production five-second deadline.
    // Retain the task handle after timeout so cleanup can release/join even for
    // the negative control that replaces bounded acquisition with blocking lock.
    let timeout_gate: &'static Semaphore = Box::leak(Box::new(Semaphore::new(1)));
    let timeout_holder = Holder::start();
    let before_timeout = test_support::waiter_count();
    let mut timed = create(timeout_gate);
    let timeout_waited = waiter_seen(before_timeout + 1).await;
    let received = tokio::time::timeout(Duration::from_secs(15), &mut timed).await;
    let returned_while_held = received.is_ok() && timeout_holder.held();
    let permits_after_timeout = timeout_gate.available_permits();
    let (after_timeout, after_timeout_rx) = ordinary(timeout_gate, "status.set", calls.clone());
    let timeout_released_admission = matches!(tokio::time::timeout(Duration::from_secs(2), after_timeout_rx).await, Ok(Ok(())));
    drop(timeout_holder);
    let timed = match received {
        Ok(value) => value,
        Err(_) => timed.await,
    }.expect("timeout task joined").expect("timeout closure joined");
    joined_tasks += 1;
    after_timeout.await.expect("recovered task joined").expect("recovered closure joined");
    joined_tasks += 1;

    let record = serde_json::json!({
        "model_permits": 2, "first_waited": first_waited, "second_waited": second_waited,
        "permits_with_one": permits_with_one, "permits_saturated": permits_saturated,
        "spare_progress": spare_progress, "queued_waited": queued_waited,
        "ping_progress": ping_progress, "holder_retained_until_release": still_held,
        "allocations_after_release": [first.succeeded, second.succeeded],
        "recovered_permits": recovered_permits, "first_phase_ordinary_calls": first_phase_calls,
        "timeout_waited": timeout_waited, "timeout_returned_while_held": returned_while_held,
        "timeout_permits_restored_while_held": permits_after_timeout,
        "ordinary_progress_after_timeout_while_held": timeout_released_admission,
        "timeout_elapsed_ns": timed.elapsed.as_nanos().to_string(),
        "actual_anyhow_marker": timed.boxed_marker, "mapped_error": timed.mapped,
        "context_control_mapping": timed.context_mapped,
        "final_timeout_permits": timeout_gate.available_permits(),
        "ordinary_calls": calls.load(Ordering::SeqCst), "joined_tasks": joined_tasks,
        "native_scope": "two open/drop pairs after release; one real allocator timeout before native open; no shell child or daemon listener"
    });
    println!("{MARKER}{record}");
    assert!(first_waited && second_waited && timeout_waited, "actual WouldBlock observations missing: {record}");
    assert_eq!((permits_with_one, permits_saturated), (1, 0), "real admission not retained during allocator waits: {record}");
    assert!(spare_progress && queued_waited && ping_progress && still_held, "mixed admission behavior changed: {record}");
    assert!(first.succeeded && second.succeeded && recovered_permits == 2 && first_phase_calls == 3, "release recovery incomplete: {record}");
    assert!(returned_while_held && timeout_released_admission && permits_after_timeout == 1, "timeout did not release admission while holder remained: {record}");
    assert!(!timed.succeeded && timed.boxed_marker, "real anyhow boxing lost the typed timeout: {record}");
    assert_eq!(timed.mapped, crate::state::PTY_ALLOCATOR_BUSY_MESSAGE, "real daemon mapping lost typed timeout");
    assert_eq!(timed.context_mapped, "openpty failed: fixture context", "context control must not impersonate the bare marker");
    assert!(timed.elapsed >= Duration::from_secs(5), "timeout returned before the production deadline");
    assert_eq!(timeout_gate.available_permits(), 1);
    assert_eq!(calls.load(Ordering::SeqCst), 4);
    assert_eq!(joined_tasks, 7);
}

#[test]
fn real_allocator_mixed_admission_progress_saturation_ping_recovery_and_timeout() {
    assert!(std::env::var_os(CHILD_ENV).is_none(), "parent must not recurse");
    let mut command = Command::new(std::env::current_exe().expect("test executable"));
    command.hide_console();
    command.args(["--exact", CHILD_TEST, "--ignored", "--nocapture", "--test-threads=1"])
        .env(CHILD_ENV, "1")
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    for (key, _) in std::env::vars_os() {
        if key.to_str().is_some_and(|k| k.starts_with("CYS_") || k.starts_with("AITERM_") || k.starts_with("JAVIS_") || k.starts_with("CLAUDE_")) {
            command.env_remove(key);
        }
    }
    command.env(CHILD_ENV, "1");
    let mut child = command.spawn().expect("isolated child test");
    let deadline = Instant::now() + Duration::from_secs(60);
    let completed = loop {
        match child.try_wait() {
            Ok(Some(_)) => break true,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            _ => break false,
        }
    };
    if !completed { let _ = child.kill(); }
    let output = child.wait_with_output().expect("reap original test child");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(completed && output.status.success(), "child failed or exceeded containment deadline: {:?}\n{stdout}\n{stderr}", output.status.code());
    assert!(stdout.contains("1 passed; 0 failed"), "exact child test did not execute once: {stdout}");
    let raw = stdout.lines().find_map(|line| line.find(MARKER).map(|at| &line[at + MARKER.len()..]))
        .expect("child evidence missing");
    let record: serde_json::Value = serde_json::from_str(raw).expect("child evidence JSON");
    assert_eq!(record["joined_tasks"], 7);
    assert_eq!(record["actual_anyhow_marker"], true);
    println!("{MARKER}{record}");
}
