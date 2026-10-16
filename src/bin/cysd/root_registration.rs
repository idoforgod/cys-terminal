//! A-ID S2: one immutable, nonpersisted observation of the newly spawned root.
//! This is evidence for later strict checks, never a spawn prerequisite or an
//! owner-veto prerequisite. Missing observations are never filled by a lookup.

use portable_pty::Child;
use std::num::NonZeroU64;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::time::{Duration, Instant};

const OBSERVATION_LIMIT: Duration = Duration::from_millis(50);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ObservedRoot {
    pid: u32,
    start: NonZeroU64,
}

/// Only `observe` can create an observed registration outside this module.
/// The Surface retains this value beside the same original Child.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct RootRegistration(Option<ObservedRoot>);

impl RootRegistration {
    /// Does no native I/O. A later caller cannot heal Absent or rebind its PID.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn start_for(self, pid: u32) -> Option<u64> {
        self.0.filter(|root| root.pid == pid).map(|root| root.start.get())
    }
}

/// Called synchronously after successful spawn, before Surface publication.
/// The caller owns the original Child outside this observation's panic boundary.
/// T is a post-hoc acceptance limit, not cancellation or a maximum blocking time.
pub(crate) fn observe(
    child: &mut (dyn Child + Send + Sync),
    expected_pid: u32,
) -> RootRegistration {
    #[cfg(all(test))]
    test_support::before(child, expected_pid);
    let began = Instant::now();
    let result = observe_with(
        child,
        expected_pid,
        traced_alive,
        native_read_start,
        || {
            let elapsed = began.elapsed();
            #[cfg(all(test))]
            test_support::record(test_support::Event::Elapsed(elapsed));
            elapsed
        },
    );
    #[cfg(all(test))]
    test_support::record(test_support::Event::Finished { value: result.start_for(expected_pid) });
    result
}

fn native_read_start(pid: u32) -> Option<u64> {
    #[cfg(all(test))]
    if let Some(value) = test_support::read(pid) { return value; }
    crate::state::peer_start_time(pid)
}

fn traced_alive<C: Child + ?Sized>(child: &mut C) -> bool {
    let running = original_alive(child);
    #[cfg(all(test))]
    test_support::record(test_support::Event::Poll { object: test_support::object(child), running });
    running
}

fn observe_with<C: Child + ?Sized>(
    child: &mut C,
    expected_pid: u32,
    mut alive: impl FnMut(&mut C) -> bool,
    mut read_start: impl FnMut(u32) -> Option<u64>,
    elapsed: impl FnOnce() -> Duration,
) -> RootRegistration {
    let observation = catch_unwind(AssertUnwindSafe(|| {
        if expected_pid == 0
            || child.process_id() != Some(expected_pid)
            || !alive(child)
        {
            return None;
        }
        let start = read_start(expected_pid).and_then(NonZeroU64::new);
        #[cfg(all(test))]
        test_support::after_read(child, expected_pid);
        // Even an absent native read is bracketed by the same original child.
        // Never substitute a fresh process opened by numeric PID for this check.
        let still_original = child.process_id() == Some(expected_pid) && alive(child);
        let within_limit = elapsed() <= OBSERVATION_LIMIT;
        if still_original && within_limit {
            start.map(|start| ObservedRoot { pid: expected_pid, start })
        } else {
            None
        }
    }))
    .ok()
    .flatten();
    RootRegistration(observation)
}

#[cfg(not(windows))]
fn original_alive<C: Child + ?Sized>(child: &mut C) -> bool {
    // std's Unix Child retains a reaped exit status. Error/panic cannot grant R.
    matches!(child.try_wait(), Ok(None))
}

#[cfg(windows)]
fn original_alive<C: Child + ?Sized>(child: &mut C) -> bool {
    // WinChild::try_wait deliberately reports GetExitCodeProcess failure as
    // Ok(None) for existing owner-veto semantics. Do not use that ambiguity to
    // grant registration. Borrow the original owned handle only in this scope;
    // no OpenProcess, clone, CloseHandle, or handle escape.
    let Some(handle) = child.as_raw_handle() else {
        return false;
    };
    let result = unsafe {
        windows_sys::Win32::System::Threading::WaitForSingleObject(handle, 0)
    };
    windows_wait_is_running(result)
}

#[cfg(windows)]
fn windows_wait_is_running(result: u32) -> bool {
    result == windows_sys::Win32::Foundation::WAIT_TIMEOUT
}

#[cfg(test)]
mod tests {
    use super::*;
    use portable_pty::{ChildKiller, ExitStatus};
    use std::cell::{Cell, RefCell};
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    // Script-only identity; no test below opens a process or starts a child.
    const PID: u32 = 4_100_000_002;
    const START: u64 = 1_760_000_000;

    #[derive(Clone, Copy, Debug)]
    enum Poll {
        Running,
        Ended,
        Error,
        Panic,
    }

    #[derive(Debug)]
    struct ScriptedChild {
        pids: Mutex<VecDeque<Option<u32>>>,
        polls: VecDeque<Poll>,
        poll_count: usize,
        kill_count: Arc<AtomicUsize>,
    }

    impl ScriptedChild {
        fn new(polls: &[Poll]) -> Self {
            Self {
                pids: Mutex::new(VecDeque::new()),
                polls: polls.iter().copied().collect(),
                poll_count: 0,
                kill_count: Arc::new(AtomicUsize::new(0)),
            }
        }
    }

    impl ChildKiller for ScriptedChild {
        fn kill(&mut self) -> std::io::Result<()> {
            self.kill_count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        fn clone_killer(&self) -> Box<dyn ChildKiller + Send + Sync> {
            panic!("registration must not clone a killer")
        }
    }

    impl Child for ScriptedChild {
        fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
            self.poll_count += 1;
            match self.polls.pop_front().expect("unexpected extra original-child poll") {
                Poll::Running => Ok(None),
                Poll::Ended => Ok(Some(ExitStatus::with_exit_code(0))),
                Poll::Error => Err(std::io::Error::other("scripted unreadable child")),
                Poll::Panic => panic!("scripted child panic"),
            }
        }
        fn wait(&mut self) -> std::io::Result<ExitStatus> {
            panic!("registration must not block in Child::wait")
        }
        fn process_id(&self) -> Option<u32> {
            self.pids.lock().unwrap().pop_front().unwrap_or(Some(PID))
        }
        #[cfg(windows)]
        fn as_raw_handle(&self) -> Option<std::os::windows::io::RawHandle> {
            None
        }
    }

    fn scripted_alive(child: &mut ScriptedChild) -> bool {
        matches!(child.try_wait(), Ok(None))
    }

    #[test]
    fn observes_once_with_original_child_on_both_sides_and_binds_pid() {
        let mut child = ScriptedChild::new(&[Poll::Running, Poll::Running]);
        let trace = RefCell::new(Vec::new());
        let registration = observe_with(
            &mut child,
            PID,
            |child| {
                trace.borrow_mut().push("alive");
                scripted_alive(child)
            },
            |pid| {
                assert_eq!(pid, PID);
                trace.borrow_mut().push("native_start");
                Some(START)
            },
            || {
                trace.borrow_mut().push("elapsed");
                Duration::from_millis(1)
            },
        );
        assert_eq!(*trace.borrow(), ["alive", "native_start", "alive", "elapsed"]);
        assert_eq!(registration.start_for(PID), Some(START));
        assert_eq!(registration.start_for(PID + 1), None);
        assert_eq!(child.poll_count, 2);
        assert_eq!(child.kill_count.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn missing_and_zero_native_start_are_absent_after_both_polls() {
        for start in [None, Some(0)] {
            let mut child = ScriptedChild::new(&[Poll::Running, Poll::Running]);
            let r = observe_with(&mut child, PID, scripted_alive, |_| start, || Duration::ZERO);
            assert_eq!(r, RootRegistration::default());
            assert_eq!(child.poll_count, 2);
        }
    }

    #[test]
    fn ended_error_or_panic_before_read_is_absent_and_does_not_read() {
        for poll in [Poll::Ended, Poll::Error, Poll::Panic] {
            let mut child = ScriptedChild::new(&[poll]);
            let reads = Cell::new(0);
            let r = observe_with(&mut child, PID, scripted_alive, |_| {
                reads.set(reads.get() + 1);
                Some(START)
            }, || Duration::ZERO);
            assert_eq!(r.start_for(PID), None);
            assert_eq!(reads.get(), 0);
            assert_eq!(child.poll_count, 1);
            assert_eq!(child.kill_count.load(Ordering::SeqCst), 0);
        }
    }

    #[test]
    fn ended_error_or_panic_after_read_discards_start() {
        for poll in [Poll::Ended, Poll::Error, Poll::Panic] {
            let mut child = ScriptedChild::new(&[Poll::Running, poll]);
            let reads = Cell::new(0);
            let r = observe_with(&mut child, PID, scripted_alive, |_| {
                reads.set(reads.get() + 1);
                Some(START)
            }, || Duration::ZERO);
            assert_eq!(r.start_for(PID), None);
            assert_eq!(reads.get(), 1);
            assert_eq!(child.poll_count, 2);
            assert_eq!(child.kill_count.load(Ordering::SeqCst), 0);
        }
    }

    #[test]
    fn original_pid_must_be_nonzero_available_and_unchanged() {
        for (expected, pids, reads_expected) in [
            (0, vec![], 0),
            (PID, vec![None], 0),
            (PID, vec![Some(PID + 1)], 0),
            (PID, vec![Some(PID), None], 1),
            (PID, vec![Some(PID), Some(PID + 1)], 1),
        ] {
            let mut child = ScriptedChild::new(&[Poll::Running, Poll::Running]);
            *child.pids.lock().unwrap() = pids.into_iter().collect();
            let reads = Cell::new(0);
            let r = observe_with(&mut child, expected, scripted_alive, |_| {
                reads.set(reads.get() + 1);
                Some(START)
            }, || Duration::ZERO);
            assert_eq!(r, RootRegistration::default());
            assert_eq!(reads.get(), reads_expected);
        }
    }

    #[test]
    fn elapsed_limit_is_post_hoc_and_inclusive() {
        for (elapsed, expected) in [
            (Duration::from_millis(50), Some(START)),
            (Duration::from_millis(50) + Duration::from_nanos(1), None),
            (Duration::from_secs(1), None),
        ] {
            let mut child = ScriptedChild::new(&[Poll::Running, Poll::Running]);
            let read_finished = Cell::new(false);
            let r = observe_with(&mut child, PID, scripted_alive, |_| {
                read_finished.set(true);
                Some(START)
            }, || {
                assert!(read_finished.get(), "elapsed check must follow the synchronous read");
                elapsed
            });
            assert_eq!(r.start_for(PID), expected);
            assert_eq!(child.poll_count, 2);
        }
    }

    #[test]
    fn read_panic_does_not_destroy_child_or_poison_outer_guard() {
        let held = Mutex::new(ScriptedChild::new(&[Poll::Running]));
        let mut guard = held.lock().unwrap();
        let r = observe_with(&mut *guard, PID, scripted_alive,
            |_| panic!("scripted native reader panic"), || Duration::ZERO);
        assert_eq!(r.start_for(PID), None);
        assert_eq!(guard.process_id(), Some(PID));
        assert_eq!(guard.kill_count.load(Ordering::SeqCst), 0);
        drop(guard);
        assert!(!held.is_poisoned());
    }

    #[test]
    fn absent_and_observed_reads_are_immutable_no_later_native_backfill() {
        for start in [None, Some(START)] {
            let reads = Cell::new(0);
            let mut child = ScriptedChild::new(&[Poll::Running, Poll::Running]);
            let r = observe_with(&mut child, PID, scripted_alive, |_| {
                reads.set(reads.get() + 1);
                start
            }, || Duration::ZERO);
            for _ in 0..3 {
                assert_eq!(r.start_for(PID), start);
                assert_eq!(r.start_for(PID + 1), None);
            }
            assert_eq!(reads.get(), 1);
        }
    }

    #[cfg(not(windows))]
    #[test]
    fn unix_production_lifetime_reader_accepts_only_running() {
        for (poll, expected) in [(Poll::Running, true), (Poll::Ended, false), (Poll::Error, false)] {
            let mut child = ScriptedChild::new(&[poll]);
            assert_eq!(original_alive(&mut child), expected);
            assert_eq!(child.poll_count, 1);
        }
        let mut child = ScriptedChild::new(&[Poll::Panic]);
        let r = observe_with(&mut child, PID, original_alive, |_| Some(START), || Duration::ZERO);
        assert_eq!(r.start_for(PID), None);
    }

    #[cfg(windows)]
    #[test]
    fn windows_production_wait_result_preserves_unavailable() {
        use windows_sys::Win32::Foundation::{WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT};
        assert!(windows_wait_is_running(WAIT_TIMEOUT));
        for result in [WAIT_OBJECT_0, WAIT_FAILED, 0x80, 0x1234] {
            assert!(!windows_wait_is_running(result));
        }
        let mut child = ScriptedChild::new(&[]);
        assert!(!original_alive(&mut child), "missing original handle is unproven");
        assert_eq!(child.poll_count, 0, "do not reuse the ambiguous owner try_wait");
    }
}

#[cfg(all(test))]
pub(crate) mod test_support {
    use super::*;
    use std::{cell::RefCell, rc::Rc};
    use portable_pty::ExitStatus;

    pub(crate) trait ChildView {
        fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>>;
    }
    struct Borrowed<'a, C: Child + ?Sized>(&'a mut C);
    impl<C: Child + ?Sized> ChildView for Borrowed<'_, C> {
        fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> { self.0.try_wait() }
    }
    pub(crate) type ChildHook = Rc<dyn Fn(u32, &mut dyn ChildView)>;
    pub(crate) type ReadHook = Rc<dyn Fn(u32) -> Option<u64>>;
    #[derive(Clone, Debug)]
    pub(crate) enum Event {
        Entered { pid: u32, object: usize },
        Poll { object: usize, running: bool },
        ReadEntered { pid: u32 },
        ReadReturned { value: Option<u64> },
        Elapsed(Duration),
        Finished { value: Option<u64> },
    }
    pub(crate) struct Hooks {
        pub before: Option<ChildHook>,
        pub after_read: Option<ChildHook>,
        pub read: ReadHook,
    }
    struct Installed { hooks: Hooks, events: RefCell<Vec<Event>> }
    thread_local! { static ACTIVE: RefCell<Option<Rc<Installed>>> = const { RefCell::new(None) }; }
    pub(crate) struct Scope { installed: Rc<Installed>, previous: Option<Rc<Installed>> }
    impl Scope {
        pub(crate) fn install(hooks: Hooks) -> Self {
            let installed = Rc::new(Installed { hooks, events: RefCell::new(Vec::new()) });
            let previous = ACTIVE.with(|v| v.replace(Some(installed.clone())));
            Self { installed, previous }
        }
        pub(crate) fn events(&self) -> Vec<Event> { self.installed.events.borrow().clone() }
    }
    impl Drop for Scope {
        fn drop(&mut self) { ACTIVE.with(|v| { v.replace(self.previous.take()); }); }
    }
    fn current() -> Option<Rc<Installed>> { ACTIVE.with(|v| v.borrow().clone()) }
    pub(crate) fn object<C: Child + ?Sized>(child: &C) -> usize { child as *const C as *const () as usize }
    pub(crate) fn record(event: Event) {
        if let Some(scope) = current() { scope.events.borrow_mut().push(event); }
    }
    pub(crate) fn before<C: Child + ?Sized>(child: &mut C, pid: u32) {
        record(Event::Entered { pid, object: object(child) });
        // Clone out of TLS/RefCell before a callback. A callback may inspect trace.
        let hook = current().and_then(|s| s.hooks.before.clone());
        if let Some(hook) = hook { hook(pid, &mut Borrowed(child)); }
    }
    pub(crate) fn after_read<C: Child + ?Sized>(child: &mut C, pid: u32) {
        let hook = current().and_then(|s| s.hooks.after_read.clone());
        if let Some(hook) = hook { hook(pid, &mut Borrowed(child)); }
    }
    pub(crate) fn read(pid: u32) -> Option<Option<u64>> {
        let scope = current()?;
        record(Event::ReadEntered { pid });
        let value = (scope.hooks.read)(pid);
        record(Event::ReadReturned { value });
        Some(value)
    }
}

#[cfg(test)]
mod hook_tests {
    use super::*;
    use super::test_support::{Hooks, Scope};
    use std::rc::Rc;
    #[derive(Debug, Clone)]
    struct TraceChild(u32);
    impl portable_pty::ChildKiller for TraceChild {
        fn kill(&mut self) -> std::io::Result<()> { panic!("trace observation must not kill") }
        fn clone_killer(&self) -> Box<dyn portable_pty::ChildKiller + Send + Sync> { panic!("trace observation must not clone") }
    }
    impl Child for TraceChild {
        fn try_wait(&mut self) -> std::io::Result<Option<portable_pty::ExitStatus>> { Ok(None) }
        fn wait(&mut self) -> std::io::Result<portable_pty::ExitStatus> { panic!("trace observation must not wait") }
        fn process_id(&self) -> Option<u32> { Some(self.0) }
        #[cfg(windows)]
        fn as_raw_handle(&self) -> Option<std::os::windows::io::RawHandle> { None }
    }
    #[test]
    fn trace_belongs_to_the_actual_original_object_and_observer_result() {
        use super::test_support::Event;
        let scope = Scope::install(Hooks {
            before: Some(Rc::new(|_, child| assert!(child.try_wait().unwrap().is_none()))),
            after_read: None,
            read: Rc::new(|_| Some(11)),
        });
        let mut child = TraceChild(4_100_000_001);
        let object = test_support::object(&child);
        let result = observe(&mut child, 4_100_000_001).start_for(4_100_000_001);
        let mut sequence = Vec::new();
        let mut elapsed = None;
        for event in scope.events() {
            match event {
                Event::Entered { pid, object: seen } => { assert_eq!(pid, child.0); assert_eq!(seen, object); sequence.push("enter"); }
                Event::Poll { object: seen, running } => { assert_eq!(seen, object); assert_eq!(running, cfg!(not(windows))); sequence.push("poll"); }
                Event::ReadEntered { pid } => { assert_eq!(pid, child.0); sequence.push("read"); }
                Event::ReadReturned { value } => { assert_eq!(value, Some(11)); sequence.push("read_returned"); }
                Event::Elapsed(value) => { elapsed = Some(value); sequence.push("elapsed"); }
                Event::Finished { value } => { assert_eq!(value, result); sequence.push("finish"); }
            }
        }
        if cfg!(windows) {
            assert_eq!(sequence, ["enter", "poll", "finish"]);
            assert_eq!(elapsed, None);
        } else {
            assert_eq!(sequence, ["enter", "poll", "read", "read_returned", "poll", "elapsed", "finish"]);
            assert!(elapsed.is_some());
        }
    }
    #[test]
    fn read_override_restores_outer_scope_after_panic() {
        let _outer = Scope::install(Hooks { before: None, after_read: None, read: Rc::new(|_| Some(11)) });
        let caught = std::panic::catch_unwind(|| {
            let _inner = Scope::install(Hooks { before: None, after_read: None, read: Rc::new(|_| Some(22)) });
            assert_eq!(native_read_start(4_100_000_001), Some(22));
            panic!("scope unwind control");
        });
        assert!(caught.is_err());
        assert_eq!(native_read_start(4_100_000_001), Some(11));
    }
    #[test]
    fn reader_callback_has_no_tls_borrow_held() {
        let _scope = Scope::install(Hooks { before: None, after_read: None, read: Rc::new(|_| {
            let _nested = Scope::install(Hooks { before: None, after_read: None, read: Rc::new(|_| Some(33)) });
            assert_eq!(native_read_start(4_100_000_001), Some(33));
            Some(44)
        }) });
        assert_eq!(native_read_start(4_100_000_001), Some(44));
    }
}
