//! Conclusive per-surface writer availability (S-a C8, S4-2/S4-6).
//!
//! Count admitted operations, including queued work, until the writer's entire arm and settlement
//! return. This is deliberately separate from InjectTrack's expiring input/submit observations.
//! A transition owns the same short admission lock; the lock is never held across OS I/O.
//! Zero pending operations proves operation-idle, not writer-thread exit or native-handle teardown.

use crate::state::WriteReq;
use std::sync::{mpsc, Arc, Condvar, Mutex};

#[derive(Default)]
struct State {
    pending: usize,
    transition: bool,
}

#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}

#[derive(Clone, Default)]
pub(crate) struct WriterGate(Arc<Shared>);

impl WriterGate {
    /// Conclusive operation count, never made smaller by elapsed time or a screen observation.
    pub fn pending(&self) -> usize {
        self.0
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pending
    }

    /// Compare idle and reserve the transition atomically with respect to every admission.
    pub fn try_transition(&self) -> Option<Transition> {
        let mut state = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.pending != 0 || state.transition {
            return None;
        }
        state.transition = true;
        Some(Transition(self.0.clone()))
    }

    fn reserve(&self, wait: bool) -> Option<Operation> {
        let mut state = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
        while state.transition {
            if !wait {
                return None;
            }
            state = self
                .0
                .changed
                .wait(state)
                .unwrap_or_else(|e| e.into_inner());
        }
        state.pending += 1;
        Some(Operation(self.0.clone()))
    }
}

pub(crate) struct Transition(Arc<Shared>);

impl Drop for Transition {
    fn drop(&mut self) {
        self.0
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .transition = false;
        self.0.changed.notify_all();
    }
}

pub(crate) struct Operation(Arc<Shared>);

impl Drop for Operation {
    fn drop(&mut self) {
        // Each lease has one owner and no Clone; poison must not turn unwinding into an abort.
        self.0
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pending -= 1;
    }
}

/// The private sender is the only production constructor of an accounted queue envelope.
pub(crate) struct QueuedWrite {
    request: WriteReq,
    operation: Option<Operation>,
}

impl QueuedWrite {
    pub fn into_parts(self) -> (WriteReq, Option<Operation>) {
        (self.request, self.operation)
    }
}

/// Raw writer-loop fixtures keep their existing channels. Production Surface uses WriterSender.
impl From<WriteReq> for QueuedWrite {
    fn from(request: WriteReq) -> Self {
        Self {
            request,
            operation: None,
        }
    }
}

#[derive(Clone)]
pub(crate) struct WriterSender {
    tx: mpsc::SyncSender<QueuedWrite>,
    gate: WriterGate,
}

impl WriterSender {
    pub fn channel(capacity: usize) -> (Self, mpsc::Receiver<QueuedWrite>) {
        let (tx, rx) = mpsc::sync_channel(capacity);
        (
            Self {
                tx,
                gate: WriterGate::default(),
            },
            rx,
        )
    }

    pub fn gate(&self) -> WriterGate {
        self.gate.clone()
    }

    pub fn try_send(&self, request: WriteReq) -> Result<(), mpsc::TrySendError<WriteReq>> {
        let Some(operation) = self.gate.reserve(false) else {
            return Err(mpsc::TrySendError::Full(request));
        };
        let queued = QueuedWrite {
            request,
            operation: Some(operation),
        };
        self.tx.try_send(queued).map_err(|error| match error {
            mpsc::TrySendError::Full(queued) => mpsc::TrySendError::Full(queued.request),
            mpsc::TrySendError::Disconnected(queued) => {
                mpsc::TrySendError::Disconnected(queued.request)
            }
        })
    }

    // Blocking send is used by fixtures. Production producers use try_send/bounded admission.
    pub fn send(&self, request: WriteReq) -> Result<(), mpsc::SendError<WriteReq>> {
        let operation = self
            .gate
            .reserve(true)
            .expect("waiting admission reserves a lease");
        self.tx
            .send(QueuedWrite {
                request,
                operation: Some(operation),
            })
            .map_err(|error| mpsc::SendError(error.0.request))
    }
}

/// The DSR bounded sender also serves raw-channel fixtures; it cannot bypass a Surface's gate.
pub(crate) trait WriteAdmission {
    fn admit(&self, request: WriteReq) -> Result<(), mpsc::TrySendError<WriteReq>>;
}

impl WriteAdmission for WriterSender {
    fn admit(&self, request: WriteReq) -> Result<(), mpsc::TrySendError<WriteReq>> {
        self.try_send(request)
    }
}

impl WriteAdmission for mpsc::SyncSender<WriteReq> {
    fn admit(&self, request: WriteReq) -> Result<(), mpsc::TrySendError<WriteReq>> {
        self.try_send(request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_write::{AgentJob, PassThrough, Plan, Request};
    use crate::state::run_writer_loop_tracked;
    use std::io::{self, Write};
    use std::sync::atomic::AtomicBool;
    use std::time::Duration;

    fn queued(sender: &WriterSender, request: WriteReq) {
        assert!(sender.try_send(request).is_ok(), "fixture admission");
    }

    fn body(error: mpsc::TrySendError<WriteReq>) -> Vec<u8> {
        match error {
            mpsc::TrySendError::Full(WriteReq::Data(bytes))
            | mpsc::TrySendError::Disconnected(WriteReq::Data(bytes)) => bytes,
            _ => panic!("must return original request"),
        }
    }

    #[test]
    fn c8a_queued_operation_prevents_transition_until_receiver_drop() {
        let (sender, receiver) = WriterSender::channel(2);
        let gate = sender.gate();
        queued(&sender, WriteReq::Data(b"queued".to_vec()));
        assert_eq!(gate.pending(), 1, "queued bytes count before writer starts");
        assert!(
            gate.try_transition().is_none(),
            "queued work forbids adoption"
        );
        drop(receiver);
        assert_eq!(gate.pending(), 0, "receiver drop releases queued ownership");
        assert!(gate.try_transition().is_some());
    }

    #[test]
    fn c8a_rejected_admission_preserves_request_and_releases_only_its_lease() {
        let (sender, receiver) = WriterSender::channel(1);
        let gate = sender.gate();
        queued(&sender, WriteReq::Data(b"one".to_vec()));
        assert_eq!(
            body(
                sender
                    .try_send(WriteReq::Data(b"two".to_vec()))
                    .err()
                    .unwrap()
            ),
            b"two"
        );
        assert_eq!(gate.pending(), 1, "Full rolls back only the rejected lease");
        drop(receiver);
        assert_eq!(
            body(
                sender
                    .try_send(WriteReq::Data(b"three".to_vec()))
                    .err()
                    .unwrap()
            ),
            b"three"
        );
        assert_eq!(
            gate.pending(),
            0,
            "Disconnected leaves no phantom operation"
        );
    }

    #[test]
    fn c8a_transition_excludes_admission_until_guard_drop() {
        let (sender, receiver) = WriterSender::channel(1);
        let gate = sender.gate();
        let transition = gate.try_transition().unwrap();
        assert!(gate.try_transition().is_none(), "one transition owner");
        assert!(
            matches!(
                sender.try_send(WriteReq::Data(b"held".to_vec())),
                Err(mpsc::TrySendError::Full(_))
            ),
            "transition blocks bytes before enqueue"
        );
        assert!(matches!(
            receiver.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        drop(transition);
        assert!(
            sender.try_send(WriteReq::Data(b"next".to_vec())).is_ok(),
            "dropping transition reopens admission"
        );
        assert_eq!(gate.pending(), 1);
    }

    #[test]
    fn c8a_blocking_fixture_send_accounts_and_releases_rejected_handoff() {
        let (sender, receiver) = WriterSender::channel(1);
        let gate = sender.gate();
        sender.send(WriteReq::Data(b"first".to_vec())).unwrap();
        assert_eq!(gate.pending(), 1, "fixture send accounts queued work");
        let other = sender.clone();
        let runner = std::thread::spawn(move || other.send(WriteReq::Data(b"blocked".to_vec())));
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while gate.pending() != 2 && std::time::Instant::now() < deadline {
            std::thread::yield_now();
        }
        let pending = gate.pending();
        let excluded = gate.try_transition().is_none();
        // Release before asserting, so even a defective counter cannot strand this fixture thread.
        drop(receiver);
        let error = runner
            .join()
            .unwrap()
            .err()
            .expect("receiver drop rejects blocked send");
        assert_eq!(
            pending, 2,
            "blocked handoff owns an admission lease before queue insertion"
        );
        assert!(excluded);
        assert!(matches!(error.0, WriteReq::Data(bytes) if bytes == b"blocked"));
        assert_eq!(
            gate.pending(),
            0,
            "blocked send rejection releases exactly its lease"
        );
    }

    struct BarrierWriter {
        entered: mpsc::Sender<()>,
        release: mpsc::Receiver<()>,
        wait_at_flush: bool,
        wait_at_return: bool,
        waited: bool,
        bytes: Arc<Mutex<Vec<u8>>>,
    }

    impl BarrierWriter {
        fn barrier(&mut self) {
            if !self.waited {
                self.waited = true;
                self.entered.send(()).unwrap();
                self.release
                    .recv_timeout(Duration::from_secs(10))
                    .expect("test releases native call");
            }
        }
    }

    impl Write for BarrierWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if !self.wait_at_flush && (!self.wait_at_return || bytes == b"\r") {
                self.barrier();
            }
            self.bytes.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            let at_return = self.bytes.lock().unwrap().ends_with(b"\r");
            if self.wait_at_flush && (!self.wait_at_return || at_return) {
                self.barrier();
            }
            Ok(())
        }
    }

    fn agent(settle: Option<crate::agent_write::Settle>) -> WriteReq {
        WriteReq::Agent(Box::new(AgentJob {
            request: Request::new("c8a".into(), 1, "test", "digest".into()),
            plan: Plan::raw("agent").with_submit(0, 0),
            proof: Arc::new(PassThrough),
            journal: None,
            resume: None,
            settle,
            guard: None,
            note_body: false,
            body_owner: None,
        }))
    }

    #[test]
    fn c8a_every_writer_arm_holds_lease_through_write_and_flush() {
        for wait_at_flush in [false, true] {
            let cases = [
                (WriteReq::Data(b"data".to_vec()), b"data".to_vec()),
                (
                    WriteReq::DataAfter {
                        bytes: b"after".to_vec(),
                        delay_ms: 0,
                    },
                    b"after".to_vec(),
                ),
                (WriteReq::Program(b"program".to_vec()), b"program".to_vec()),
                (
                    WriteReq::SubmitAfterGap {
                        bytes: b"\r".to_vec(),
                        min_gap_ms: 0,
                        withhold: None,
                    },
                    b"\r".to_vec(),
                ),
                (
                    WriteReq::Inject {
                        text: "inject".into(),
                        cr_delay_ms: 0,
                        clear_first: false,
                        guard: None,
                    },
                    b"\x1b[200~inject\x1b[201~\r".to_vec(),
                ),
                (agent(None), b"agent\r".to_vec()),
            ];
            for (request, expected) in cases {
                assert_leased_during_io(request, expected, wait_at_flush, false);
            }
        }
    }

    fn assert_leased_during_io(
        request: WriteReq,
        expected: Vec<u8>,
        wait_at_flush: bool,
        wait_at_return: bool,
    ) {
        let (sender, receiver) = WriterSender::channel(2);
        let gate = sender.gate();
        let (entered, reached) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let writer = BarrierWriter {
            entered,
            release: wait,
            wait_at_flush,
            wait_at_return,
            waited: false,
            bytes: bytes.clone(),
        };
        queued(&sender, request);
        drop(sender);
        let runner = std::thread::spawn(move || {
            run_writer_loop_tracked(writer, receiver, Arc::new(AtomicBool::new(false)), None)
        });
        reached
            .recv_timeout(Duration::from_secs(10))
            .expect("native write/flush reached");
        let pending = gate.pending();
        let transition = gate.try_transition();
        release.send(()).unwrap();
        runner.join().unwrap();
        assert_eq!(pending, 1, "native write/flush keeps operation outstanding");
        assert!(
            transition.is_none(),
            "blocked operation forbids replacement"
        );
        assert_eq!(
            *bytes.lock().unwrap(),
            expected,
            "all original bytes and ordering preserved"
        );
        assert_eq!(gate.pending(), 0, "terminal writer arm releases lease");
    }

    #[test]
    fn c8a_final_return_and_final_flush_keep_operation_outstanding() {
        for wait_at_flush in [false, true] {
            let inject = WriteReq::Inject {
                text: "inject".into(),
                cr_delay_ms: 0,
                clear_first: false,
                guard: None,
            };
            assert_leased_during_io(
                inject,
                b"\x1b[200~inject\x1b[201~\r".to_vec(),
                wait_at_flush,
                true,
            );
            assert_leased_during_io(agent(None), b"agent\r".to_vec(), wait_at_flush, true);
        }
    }

    #[test]
    fn c8a_agent_terminal_settlement_is_inside_operation_lease() {
        let (sender, receiver) = WriterSender::channel(1);
        let gate = sender.gate();
        let (entered, reached) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        queued(
            &sender,
            agent(Some(Box::new(move |_| {
                entered.send(()).unwrap();
                wait.recv_timeout(Duration::from_secs(10)).unwrap();
            }))),
        );
        drop(sender);
        let runner = std::thread::spawn(move || {
            run_writer_loop_tracked(Vec::new(), receiver, Arc::new(AtomicBool::new(false)), None)
        });
        reached
            .recv_timeout(Duration::from_secs(10))
            .expect("settlement entered");
        let pending = gate.pending();
        let transition = gate.try_transition();
        release.send(()).unwrap();
        runner.join().unwrap();
        assert_eq!(
            pending, 1,
            "terminal record/producer settlement must finish before adoption"
        );
        assert!(transition.is_none());
        assert_eq!(gate.pending(), 0);
    }

    #[test]
    fn c8a_admission_and_transition_race_have_exactly_one_winner() {
        for _ in 0..64 {
            let (sender, receiver) = WriterSender::channel(1);
            let gate = sender.gate();
            let barrier = Arc::new(std::sync::Barrier::new(3));
            let admit_barrier = barrier.clone();
            let admit = std::thread::spawn(move || {
                admit_barrier.wait();
                sender.try_send(WriteReq::Data(b"racing".to_vec())).is_ok()
            });
            let reserve_barrier = barrier.clone();
            let reserve_gate = gate.clone();
            let reserve = std::thread::spawn(move || {
                reserve_barrier.wait();
                reserve_gate.try_transition()
            });
            barrier.wait();
            // Keep BOTH winners' resources until both contenders finish. Releasing a transition
            // in its thread would allow a legitimate subsequent enqueue and make this a false red.
            let transition = reserve.join().unwrap();
            let admitted = admit.join().unwrap();
            assert_ne!(
                admitted,
                transition.is_some(),
                "admission and transition are mutually exclusive"
            );
            assert_eq!(gate.pending(), usize::from(admitted));
            drop(transition);
            drop(receiver);
            assert_eq!(gate.pending(), 0);
            assert!(gate.try_transition().is_some());
        }
    }

    #[test]
    fn c8a_early_continue_releases_each_lease_and_keeps_next_write() {
        let (sender, receiver) = WriterSender::channel(3);
        let gate = sender.gate();
        queued(
            &sender,
            WriteReq::SubmitAfterGap {
                bytes: b"\r".to_vec(),
                min_gap_ms: 0,
                withhold: Some(Arc::new(|| true)),
            },
        );
        let guard = crate::state::InjectGuard::new(
            Some(1),
            Arc::new(std::sync::atomic::AtomicU64::new(2)),
            false,
            None,
        );
        queued(
            &sender,
            WriteReq::Inject {
                text: "refused".into(),
                cr_delay_ms: 0,
                clear_first: false,
                guard: Some(Arc::new(guard)),
            },
        );
        queued(&sender, WriteReq::Data(b"next".to_vec()));
        assert_eq!(gate.pending(), 3);
        drop(sender);
        let mut bytes = Vec::new();
        run_writer_loop_tracked(&mut bytes, receiver, Arc::new(AtomicBool::new(false)), None);
        assert_eq!(
            bytes, b"next",
            "withheld/refused requests write nothing and next request proceeds"
        );
        assert_eq!(gate.pending(), 0, "continue releases the current lease");
        assert!(gate.try_transition().is_some());
    }

    struct FaultWriter {
        panic_once: bool,
        bytes: Vec<u8>,
    }
    impl Write for FaultWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.panic_once {
                self.panic_once = false;
                panic!("deliberate writer panic");
            }
            if self.bytes.is_empty() && bytes != b"next" {
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "deliberate broken pipe",
                ));
            }
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn c8a_transport_break_releases_current_and_queued_leases() {
        for request in [WriteReq::Data(b"broken".to_vec()), agent(None)] {
            let (sender, receiver) = WriterSender::channel(2);
            let gate = sender.gate();
            queued(&sender, request);
            queued(&sender, WriteReq::Data(b"next".to_vec()));
            assert_eq!(gate.pending(), 2);
            let mut writer = FaultWriter {
                panic_once: false,
                bytes: Vec::new(),
            };
            run_writer_loop_tracked(
                &mut writer,
                receiver,
                Arc::new(AtomicBool::new(false)),
                None,
            );
            assert!(
                writer.bytes.is_empty(),
                "broken transport never writes the queued successor"
            );
            assert_eq!(
                gate.pending(),
                0,
                "break drops both active and queued ownership"
            );
            assert!(matches!(
                sender.try_send(WriteReq::Data(Vec::new())),
                Err(mpsc::TrySendError::Disconnected(_))
            ));
            assert_eq!(gate.pending(), 0);
        }
    }

    #[test]
    fn c8a_raw_writer_unwind_releases_current_and_queued_leases() {
        let (sender, receiver) = WriterSender::channel(2);
        let gate = sender.gate();
        queued(&sender, WriteReq::Data(b"panic".to_vec()));
        queued(&sender, WriteReq::Data(b"next".to_vec()));
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_writer_loop_tracked(
                FaultWriter {
                    panic_once: true,
                    bytes: Vec::new(),
                },
                receiver,
                Arc::new(AtomicBool::new(false)),
                None,
            );
        }));
        let panic = outcome.expect_err("raw writer panic exercises unwinding");
        assert_eq!(
            panic.downcast_ref::<&str>().copied(),
            Some("deliberate writer panic"),
            "the intended writer call must be the unwind origin"
        );
        assert_eq!(gate.pending(), 0, "unwind drops active and queued leases");
        assert!(gate.try_transition().is_some());
    }

    #[test]
    fn c8a_caught_agent_and_settlement_panics_keep_then_release_lease() {
        for panic_in_writer in [false, true] {
            let (sender, receiver) = WriterSender::channel(2);
            let gate = sender.gate();
            let settle_gate = gate.clone();
            let (observed, received) = mpsc::channel();
            queued(
                &sender,
                agent(Some(Box::new(move |terminal| {
                    observed
                        .send((
                            settle_gate.pending(),
                            settle_gate.try_transition().is_none(),
                            terminal.clone(),
                        ))
                        .unwrap();
                    if !panic_in_writer {
                        panic!("deliberate settlement panic");
                    }
                }))),
            );
            queued(&sender, WriteReq::Data(b"next".to_vec()));
            drop(sender);
            let mut writer = FaultWriter {
                panic_once: panic_in_writer,
                bytes: if panic_in_writer {
                    Vec::new()
                } else {
                    b"seed".to_vec()
                },
            };
            run_writer_loop_tracked(
                &mut writer,
                receiver,
                Arc::new(AtomicBool::new(false)),
                None,
            );
            let (pending, excluded, terminal) =
                received.recv_timeout(Duration::from_secs(10)).unwrap();
            assert_eq!(
                pending, 2,
                "caught panic still settles inside the admission lease"
            );
            assert!(excluded);
            if panic_in_writer {
                assert!(matches!(
                    terminal,
                    crate::agent_write::Terminal::PartialUnknown { .. }
                ));
            }
            assert!(
                writer.bytes.ends_with(b"next"),
                "caught panic preserves writer progress"
            );
            assert_eq!(gate.pending(), 0, "caught panic releases exactly once");
        }
    }

    #[test]
    fn c8a_poisoned_lock_and_transition_unwind_release_without_panicking() {
        let (sender, receiver) = WriterSender::channel(1);
        let gate = sender.gate();
        queued(&sender, WriteReq::Data(b"queued".to_vec()));
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _lock = gate.0.state.lock().unwrap();
            panic!("poison without mutating gate state");
        }))
        .is_err());
        assert_eq!(gate.pending(), 1);
        drop(receiver);
        assert_eq!(gate.pending(), 0, "operation drop tolerates poison");
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _transition = gate.try_transition().unwrap();
            panic!("unwind while transition is reserved");
        }))
        .is_err());
        assert!(
            gate.try_transition().is_some(),
            "poisoned transition drop releases reservation"
        );
        assert!(
            matches!(
                sender.try_send(WriteReq::Data(Vec::new())),
                Err(mpsc::TrySendError::Disconnected(_))
            ),
            "poison must not leave admission held"
        );
        assert_eq!(gate.pending(), 0);
    }

    #[test]
    fn c8a_bounded_dsr_admission_uses_the_same_transition_gate() {
        let (sender, receiver) = WriterSender::channel(1);
        let gate = sender.gate();
        let transition = gate.try_transition().unwrap();
        assert!(
            !crate::state::send_write_req_bounded(
                &sender,
                WriteReq::Data(b"dsr".to_vec()),
                Duration::ZERO
            ),
            "DSR refuses while transition owns the gate"
        );
        assert!(matches!(
            receiver.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        assert_eq!(gate.pending(), 0);
        drop(transition);
        assert!(crate::state::send_write_req_bounded(
            &sender,
            WriteReq::Data(b"dsr".to_vec()),
            Duration::ZERO
        ));
        assert_eq!(gate.pending(), 1);
        let (request, lease) = receiver.recv().unwrap().into_parts();
        assert!(matches!(request, WriteReq::Data(bytes) if bytes == b"dsr"));
        drop(lease);
        assert_eq!(gate.pending(), 0);
    }
}
