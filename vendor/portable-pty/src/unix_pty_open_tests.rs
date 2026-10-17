//! Source boundary controls for the Unix allocation critical section.
use super::*;

// These controls deliberately hold/poison one process-wide allocator. Serialize
// in the tests themselves so ordinary `cargo test` needs no special CLI flag.
static TEST_SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    TEST_SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn allocation_body(source: &str) -> &str {
    let marker = "fn openpty(size: PtySize)";
    assert_eq!(source.matches(marker).count(), 1);
    let start = source.find(marker).unwrap();
    let open = start + source[start..].find('{').unwrap();
    let mut depth = 0;
    for (offset, byte) in source.as_bytes()[open..].iter().enumerate() {
        match byte { b'{' => depth += 1, b'}' => { depth -= 1; if depth == 0 { return &source[open..=open+offset]; } }, _ => {} }
    }
    panic!("private allocator boundary missing");
}

#[test]
fn unix_allocation_guard_covers_native_open_and_both_cloexec_without_spawn_or_await() {
    let _serial = serial();
    let source = include_str!("unix.rs");
    let body = allocation_body(source);
    let guard = "let _open_guard = acquire_pty_open_lock_until(deadline)?;";
    assert_eq!(body.matches(guard).count(), 1, "one named process-wide guard must cover the allocation");
    assert!(source[..source.find("fn openpty(size: PtySize)").unwrap()].contains(
        "static PTY_OPEN_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());"));
    let native = body.find("libc::openpty(").unwrap();
    let master = body.find("cloexec(master.fd.as_raw_fd())?").unwrap();
    let slave = body.find("cloexec(slave.fd.as_raw_fd())?").unwrap();
    assert!(body.find(guard).unwrap() < native && native < master && master < slave);
    assert!(slave < body.find("Ok((master, slave))").unwrap());
    for forbidden in ["drop(_open_guard)", "forget(_open_guard)", "let PTY_OPEN_LOCK", "Mutex::new", ".await", "spawn_command", ".spawn(", ".exec(", "queue", "feed"] {
        assert!(!body.contains(forbidden), "allocation-only boundary crossed: {}", forbidden);
    }
}

#[test]
fn allocation_lock_stays_behind_the_unix_module_boundary() {
    let _serial = serial();
    let source = include_str!("lib.rs");
    assert!(source.contains("#[cfg(unix)]\npub mod unix;"));
    assert!(source.contains("#[cfg(windows)]\npub mod win;"));
    assert!(!include_str!("win/conpty.rs").contains("PTY_OPEN_LOCK"));
}

#[test]
fn static_allocation_lock_excludes_a_second_thread() {
    let _serial = serial();
    let guard = PTY_OPEN_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let contender = std::thread::spawn(|| matches!(PTY_OPEN_LOCK.try_lock(), Err(std::sync::TryLockError::WouldBlock)));
    let blocked = contender.join();
    drop(guard);
    assert_eq!(blocked.unwrap(), true);
    let recovered = PTY_OPEN_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    drop(recovered);
}

#[test]
fn unit_allocation_lock_is_reusable_after_unwind() {
    let _serial = serial();
    let result = std::panic::catch_unwind(|| {
        let _open_guard = PTY_OPEN_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        panic!("controlled allocation critical-section unwind");
    });
    assert!(result.is_err());
    assert!(PTY_OPEN_LOCK.is_poisoned());
    let guard = acquire_pty_open_lock_until(std::time::Instant::now() + PTY_OPEN_WAIT_LIMIT).unwrap();
    drop(guard);
}

#[test]
fn held_allocator_returns_busy_at_deadline_without_interrupting_holder() {
    let _serial = serial();
    let holder = PTY_OPEN_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let began = std::time::Instant::now();
        let deadline = began + std::time::Duration::from_millis(20);
        entered_tx.send(()).unwrap();
        let result = acquire_pty_open_lock_until(deadline);
        done_tx.send((result.is_err(), began.elapsed())).unwrap();
    });
    let entered = entered_rx.recv_timeout(std::time::Duration::from_secs(1));
    let result_while_held = done_rx.recv_timeout(std::time::Duration::from_millis(200));
    drop(holder); // unconditional cleanup precedes all assertions, including the unbounded control
    let joined = worker.join();
    assert!(entered.is_ok() && joined.is_ok());
    assert!(matches!(result_while_held, Ok((true, _))),
        "busy deadline must return while original holder remains locked: {:?}", result_while_held);
}

#[test]
fn acquired_after_deadline_is_released_without_granting_allocation() {
    let _serial = serial();
    let before = std::time::Instant::now();
    let deadline = before + std::time::Duration::from_millis(1);
    let mut reads = 0;
    let result = acquire_pty_open_lock_with(deadline, || {
        reads += 1;
        if reads == 1 { before } else { deadline }
    }, |_| panic!("free lock must not wait"));
    assert!(result.is_err());
    assert_eq!(reads, 2);
    match PTY_OPEN_LOCK.try_lock() {
        Ok(guard) => drop(guard),
        Err(std::sync::TryLockError::Poisoned(poisoned)) => drop(poisoned.into_inner()),
        Err(std::sync::TryLockError::WouldBlock) => panic!("late acquired guard leaked"),
    }
}

#[test]
fn expired_deadline_refuses_even_a_free_allocator_and_production_budget_is_five_seconds() {
    let _serial = serial();
    let deadline = std::time::Instant::now();
    assert!(acquire_pty_open_lock_with(deadline, || deadline, |_| panic!("expired wait")).is_err());
    assert_eq!(PTY_OPEN_WAIT_LIMIT, std::time::Duration::from_secs(5));
    let body = allocation_body(include_str!("unix.rs"));
    assert!(body.contains("let deadline = std::time::Instant::now() + PTY_OPEN_WAIT_LIMIT;"));
    assert_eq!(body.matches("acquire_pty_open_lock_until(deadline)?").count(), 1);
    assert_eq!(body.matches("libc::openpty(").count(), 1);
}
