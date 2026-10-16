// Producers (queue, direct send, schedule/channel/feed/boot notices) are wired by the following S-a
// commits; until then only the writer arm (`state::run_writer_loop_tracked`) and tests build jobs.
#![allow(dead_code)]
//! S-a checked agent writer (DESIGN-S-FROZEN-R3 §4.4-4.5, §4.7; HEAD-R3-RULING S4-1).
//!
//! Every machine write aimed at an agent receiver is ONE [`AgentJob`] executed by the surface's
//! single writer thread (arm `WriteReq::Agent`). Inside a job the order is fixed: durable `writing`
//! journal record -> full admission proof -> OS calls. Before EVERY later OS call — including a
//! short-write or EINTR retry, and after every wait (clear settle, paste-to-Return spacing, submit
//! gap) — the job checks its cancellation flag and re-proves the receiver; each call offers at most
//! [`MAX_CALL_BYTES`]. The terminal outcome is journaled before the producer's settlement closure
//! runs, so no producer can pop, count or announce delivery before the PTY accepted the bytes.
//! Refusal, cancellation and partial results are typed outcomes: they never end the writer loop.
//!
//! S-a residual (stated, not solved): a process can die after a passing check and before the next
//! OS call; bytes already accepted can still be read by the surviving shell; an issued native call
//! is never interrupted. Containment and native cancellation are S-b.

use crate::receiver::ProofLoss;
use serde_json::json;
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

/// Per-OS-call cap (design 4.4 step 3).
pub const MAX_CALL_BYTES: usize = 4096;
const FENCE_OPEN: &[u8] = cys::paste_fence::OPEN.as_bytes();
const FENCE_CLOSE: &[u8] = cys::paste_fence::CLOSE.as_bytes();
const CLEAR: &[u8] = b"\x15";
const RETURN: &[u8] = b"\r";

static NEXT_REQUEST: AtomicU64 = AtomicU64::new(1);

/// Daemon-unique request id (`<daemon start ms>-<seq>` keeps ids distinct across restarts).
pub fn new_request_id(daemon_started_ms: u64) -> String {
    format!("w{daemon_started_ms:x}-{}", NEXT_REQUEST.fetch_add(1, Ordering::Relaxed))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Clear,
    OpenFence,
    Body,
    CloseFence,
    Control,
    Submit,
}

impl Phase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Clear => "clear",
            Self::OpenFence => "open_fence",
            Self::Body => "body",
            Self::CloseFence => "close_fence",
            Self::Control => "control",
            Self::Submit => "submit",
        }
    }
}

/// The request's own Return.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Submit {
    /// Paste-to-Return spacing after this request's body (the existing `cr_delay_ms`).
    pub delay_ms: u64,
    /// Minimum gap since this writer's last program write (`SubmitAfterGap` semantics; 0 = off).
    pub min_gap_ms: u64,
}

/// What one request writes, in order. Built only through the constructors so a fenced body is
/// sanitized exactly once with the existing paste-fence rules.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    clear_first: bool,
    clear_settle_ms: u64,
    body: Option<String>,
    fenced: bool,
    control: Option<Vec<u8>>,
    submit: Option<Submit>,
}

impl Plan {
    fn empty() -> Plan {
        Plan { clear_first: false, clear_settle_ms: 0, body: None, fenced: false, control: None, submit: None }
    }
    /// One bracketed-paste body: open fence + body + close fence are byte-identical to
    /// `cys::paste_fence::wrap(text)` (the existing `WriteReq::Inject` bytes).
    pub fn paste(text: &str) -> Plan {
        Plan { body: Some(cys::paste_fence::sanitize(text).into_owned()), fenced: true, ..Plan::empty() }
    }
    /// An unfenced program body (the direct-send `DirectFence::Raw` mapping): bytes as given.
    pub fn raw(text: &str) -> Plan {
        Plan { body: Some(text.to_string()), ..Plan::empty() }
    }
    /// Control bytes instead of a body (AgentControl: Ctrl-U, Escape, a dialog digit ...).
    pub fn control(bytes: Vec<u8>) -> Plan {
        Plan { control: Some(bytes), ..Plan::empty() }
    }
    /// A Return that is this request's only phase (bound standalone submit, design 4.5).
    pub fn submit_only(min_gap_ms: u64) -> Plan {
        Plan { submit: Some(Submit { delay_ms: 0, min_gap_ms }), ..Plan::empty() }
    }
    /// Ctrl-U before the body, then `settle_ms` (the existing clear-first settle).
    pub fn with_clear(mut self, settle_ms: u64) -> Plan {
        self.clear_first = true;
        self.clear_settle_ms = settle_ms;
        self
    }
    /// The request owns its Return, written `delay_ms` after the body.
    pub fn with_submit(mut self, delay_ms: u64, min_gap_ms: u64) -> Plan {
        self.submit = Some(Submit { delay_ms, min_gap_ms });
        self
    }
    pub fn body(&self) -> Option<&str> {
        self.body.as_deref()
    }
    pub fn owns_submit(&self) -> bool {
        self.submit.is_some()
    }
    /// Every byte the plan writes, in order.
    pub fn planned_bytes(&self) -> Vec<u8> {
        calls_for(self, &Envelope::default()).into_iter().flat_map(|(_, b)| b).collect()
    }
}

/// Where a request is, byte-exactly (S4-1 PartialExact). Cumulative over resumed runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Envelope {
    pub cleared: bool,
    pub opened: bool,
    /// Body bytes accepted so far.
    pub body_offset: usize,
    pub closed: bool,
    pub control_done: bool,
    pub submitted: bool,
}

/// Accepted byte counts of ONE run, per column (payload / fence / control, design 4.4); `calls`
/// counts OS write attempts including EINTR retries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Accepted {
    pub payload: u64,
    pub fence: u64,
    pub control: u64,
    pub calls: u64,
}

impl Accepted {
    pub fn total(&self) -> u64 {
        self.payload + self.fence + self.control
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Terminal {
    /// Journal, proof or transport refused before the first byte: ZERO bytes (incl. fence/Ctrl-U/Return).
    RefusedNoWrite { reason: String },
    CancelledBeforeWrite,
    /// Body (or control) complete; the request owns no submit phase.
    WrittenBody { accepted: Accepted },
    /// Every phase including the request's own Return complete.
    WrittenSubmitted { accepted: Accepted },
    /// Stopped by a resumable (non-death, non-replacement) proof loss with no call outstanding at a
    /// byte-exact point: the SAME request may resume from `envelope` (S4-1).
    PartialExact { envelope: Envelope, accepted: Accepted, reason: String },
    /// Stopped after bytes by death/replacement, transport error, cancellation or a non-exact cut:
    /// never replayed (design 4.6 reconciliation).
    PartialUnknown { envelope: Envelope, accepted: Accepted, reason: String },
}

impl Terminal {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::RefusedNoWrite { .. } => "refused_no_write",
            Self::CancelledBeforeWrite => "cancelled_before_write",
            Self::WrittenBody { .. } => "written_body",
            Self::WrittenSubmitted { .. } => "written_submitted",
            Self::PartialExact { .. } => "partial_exact",
            Self::PartialUnknown { .. } => "partial_unknown",
        }
    }
    pub fn wrote_nothing(&self) -> bool {
        matches!(self, Self::RefusedNoWrite { .. } | Self::CancelledBeforeWrite)
    }
}

/// Lifecycle of one request as seen by status readers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    AcceptedPending,
    Writing,
    Terminal(Terminal),
}

/// Shared record of one logical request (status reads never block on the writer).
pub struct Request {
    pub id: String,
    pub surface_id: u64,
    pub origin: &'static str,
    pub digest: String,
    pub created: Instant,
    cancelled: AtomicBool,
    status: Mutex<Status>,
    /// The producer's settlement closure has returned (or there was none).
    settled: AtomicBool,
}

impl Request {
    pub fn new(id: String, surface_id: u64, origin: &'static str, digest: String) -> Arc<Request> {
        Arc::new(Request {
            id,
            surface_id,
            origin,
            digest,
            created: Instant::now(),
            cancelled: AtomicBool::new(false),
            status: Mutex::new(Status::AcceptedPending),
            settled: AtomicBool::new(false),
        })
    }
    /// The terminal is recorded and the producer's settlement has run.
    pub fn is_settled(&self) -> bool {
        self.settled.load(Ordering::Acquire)
    }
    /// Request cancellation: revokes LATER calls only; an issued call is never interrupted (S-a).
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
    pub fn status(&self) -> Status {
        self.status.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
    fn set(&self, s: Status) {
        *self.status.lock().unwrap_or_else(|e| e.into_inner()) = s;
    }
}

/// The receiver proof the writer consults. `admit` runs once, after the journal barrier and before
/// the first byte (binding + current input state); `recheck` runs before every later OS call.
pub trait Proof: Send + Sync {
    fn admit(&self) -> Result<(), ProofLoss>;
    fn recheck(&self, phase: Phase) -> Result<(), ProofLoss>;
}

/// Durable request journal (design 4.4 step 2 / 4.5): `writing` must be durable before any byte;
/// `terminal` is written before settlement runs.
pub trait Journal: Send + Sync {
    fn writing(&self, req: &Request) -> std::io::Result<()>;
    fn terminal(&self, req: &Request, t: &Terminal, envelope: &Envelope) -> std::io::Result<()>;
}

pub type Settle = Box<dyn FnOnce(&Terminal) + Send>;

pub struct AgentJob {
    pub request: Arc<Request>,
    pub plan: Plan,
    pub proof: Arc<dyn Proof>,
    pub journal: Option<Arc<dyn Journal>>,
    /// Resume an earlier PartialExact of the SAME request from this envelope (S4-1).
    pub resume: Option<Envelope>,
    pub settle: Option<Settle>,
    /// The queue's hand-off guard (`state::InjectGuard`): claimed after the journal barrier and
    /// before admission, so its safety probe stays next to the first byte (a failed barrier claims
    /// it too, handing that refusal to the settlement). A lost claim is [`GUARD_ABORTED`] with zero
    /// bytes; the handing-off caller owns that rollback.
    pub guard: Option<Arc<crate::state::InjectGuard>>,
    /// Record the written body as a machine body (`InjectTrack::note_body`): the queue's daemon-made
    /// bodies always (without an owner, like the `Inject` arm); a direct send only with an owner grade
    /// (the old direct path never recorded owner-token / GUI-grade text as a machine body).
    pub note_body: bool,
    /// The owner grade recorded with that body.
    pub body_owner: Option<crate::governance::InputOwner>,
}

/// RefusedNoWrite reason when the hand-off guard was not claimed (the caller already rolled back).
pub const GUARD_ABORTED: &str = "guard_aborted";

/// Writer-thread state a job reads and updates.
pub struct WriterIo<'a> {
    pub sleep: &'a dyn Fn(Duration),
    /// The writer's B2' anchor (`run_writer_loop_tracked`): stamped like `Program` for a body
    /// without its own Return and like `SubmitAfterGap` for a Return-only request; a body with
    /// its own Return does not stamp (like `Inject`).
    pub last_program_write: &'a mut Option<Instant>,
    /// Called once, immediately before the first OS write of this run.
    pub on_first_byte: &'a mut dyn FnMut(),
    /// Called once with the whole body when its last byte is accepted - before the closing fence,
    /// control or Return - so a later phase's proof (the CR withhold probe) sees this body recorded.
    pub on_body_written: &'a mut dyn FnMut(&str),
}

/// Result of [`execute`]: the terminal, where the request stands, and whether the PTY transport
/// broke (the only case in which the writer loop stops).
pub struct Executed {
    pub terminal: Terminal,
    pub envelope: Envelope,
    pub transport_broken: bool,
    /// A `writing` record was made durable, so a terminal record is owed.
    pub journaled: bool,
}

impl Executed {
    /// A panic inside a job: the amount is unknown (a lower bound, never zero by assumption).
    pub fn panicked(job: &AgentJob) -> Executed {
        Executed {
            terminal: Terminal::PartialUnknown {
                envelope: job.resume.unwrap_or_default(),
                accepted: Accepted::default(),
                reason: "writer_panic".into(),
            },
            envelope: job.resume.unwrap_or_default(),
            transport_broken: false,
            journaled: job.journal.is_some(),
        }
    }
    /// Body text accepted so far (lossy only at a short-write cut inside a character).
    pub fn accepted_body(&self, plan: &Plan) -> Option<String> {
        let b = plan.body.as_deref()?.as_bytes();
        let n = self.envelope.body_offset.min(b.len());
        (n > 0).then(|| String::from_utf8_lossy(&b[..n]).into_owned())
    }
}

/// The planned OS calls from `from`: phase + bytes. Body calls are cut at UTF-8 boundaries.
fn calls_for(plan: &Plan, from: &Envelope) -> Vec<(Phase, Vec<u8>)> {
    let mut out = Vec::new();
    if plan.clear_first && !from.cleared {
        out.push((Phase::Clear, CLEAR.to_vec()));
    }
    if let Some(body) = &plan.body {
        if plan.fenced && !from.opened {
            out.push((Phase::OpenFence, FENCE_OPEN.to_vec()));
        }
        let b = body.as_bytes();
        let mut i = from.body_offset.min(b.len());
        while i < b.len() {
            let mut end = (i + MAX_CALL_BYTES).min(b.len());
            while end > i + 1 && end < b.len() && !body.is_char_boundary(end) {
                end -= 1;
            }
            out.push((Phase::Body, b[i..end].to_vec()));
            i = end;
        }
        if plan.fenced && !from.closed {
            out.push((Phase::CloseFence, FENCE_CLOSE.to_vec()));
        }
    }
    if let Some(c) = &plan.control {
        if !from.control_done {
            out.push((Phase::Control, c.clone()));
        }
    }
    if plan.submit.is_some() && !from.submitted {
        out.push((Phase::Submit, RETURN.to_vec()));
    }
    out
}

/// One run's accounting.
struct Run {
    env: Envelope,
    acc: Accepted,
    resumed: bool,
    journaled: bool,
}

impl Run {
    fn account(&mut self, phase: Phase, n: usize, offered: usize) {
        self.acc.calls += 1;
        let whole = n == offered;
        match phase {
            Phase::Clear => {
                self.acc.control += n as u64;
                self.env.cleared = whole;
            }
            Phase::OpenFence => {
                self.acc.fence += n as u64;
                self.env.opened = whole;
            }
            Phase::Body => {
                self.acc.payload += n as u64;
                self.env.body_offset += n;
            }
            Phase::CloseFence => {
                self.acc.fence += n as u64;
                self.env.closed = whole;
            }
            Phase::Control => {
                self.acc.control += n as u64;
                self.env.control_done = whole;
            }
            Phase::Submit => {
                self.acc.control += n as u64;
                self.env.submitted = whole;
            }
        }
    }

    /// Terminal for a stop. No byte in this run of a fresh request is a zero-write refusal or
    /// cancellation; a resumed request already owns composer bytes and is never "no write"; after
    /// bytes, only a resumable loss at a byte-exact point is PartialExact.
    fn stop(&self, reason: String, exact: bool) -> Terminal {
        if self.acc.total() == 0 && !self.resumed {
            return if reason == "cancelled" { Terminal::CancelledBeforeWrite } else { Terminal::RefusedNoWrite { reason } };
        }
        if exact {
            Terminal::PartialExact { envelope: self.env, accepted: self.acc, reason }
        } else {
            Terminal::PartialUnknown { envelope: self.env, accepted: self.acc, reason }
        }
    }

    fn done(&self, terminal: Terminal, transport_broken: bool) -> Executed {
        Executed { terminal, envelope: self.env, transport_broken, journaled: self.journaled }
    }
}

/// Execute one job against `writer`. Never retries a whole body; never writes cleanup bytes.
pub fn execute<W: Write>(writer: &mut W, job: &AgentJob, io: &mut WriterIo<'_>) -> Executed {
    let req = &job.request;
    let start = job.resume.unwrap_or_default();
    let mut run = Run { env: start, acc: Accepted::default(), resumed: job.resume.is_some(), journaled: false };
    if req.is_cancelled() {
        return run.done(run.stop("cancelled".into(), false), false);
    }
    // Journal barrier first (design 4.4 step 2); the fsync is a wait, so admission comes after it.
    if let Some(j) = &job.journal {
        if let Err(e) = j.writing(req) {
            // Hand this refusal to the settlement by claiming the guard first: a broken journal is
            // then held on backoff instead of being re-handed every tick. A lost claim stays the
            // handing-off caller's rollback.
            if job.guard.as_ref().is_some_and(|g| !g.claim_for_write()) {
                return run.done(run.stop(GUARD_ABORTED.into(), true), false);
            }
            return run.done(run.stop(format!("journal:{e}"), true), false);
        }
        run.journaled = true;
    }
    if job.guard.as_ref().is_some_and(|g| !g.claim_for_write()) {
        return run.done(run.stop(GUARD_ABORTED.into(), true), false);
    }
    req.set(Status::Writing);
    if let Err(loss) = job.proof.admit() {
        return run.done(run.stop(loss.reason(), loss.resumable()), false);
    }
    let mut proven = true; // the admission just above proves the next call
    let mut first_byte = true;
    let mut prev: Option<Phase> = None;
    for (phase, bytes) in calls_for(&job.plan, &start) {
        // Spacing that already exists in the transport is kept; every wait voids the proof.
        if prev == Some(Phase::Clear) && job.plan.clear_settle_ms > 0 {
            (io.sleep)(Duration::from_millis(job.plan.clear_settle_ms));
            proven = false;
        }
        if phase == Phase::Submit {
            if let Some(s) = job.plan.submit {
                if s.delay_ms > 0 {
                    (io.sleep)(Duration::from_millis(s.delay_ms));
                    proven = false;
                }
                let since = io.last_program_write.map(|t| t.elapsed());
                if let Some(ms) = crate::state::cr_gap_delay_ms(since, s.min_gap_ms) {
                    (io.sleep)(Duration::from_millis(ms));
                    proven = false;
                }
            }
        }
        prev = Some(phase);
        let mut off = 0;
        while off < bytes.len() {
            if req.is_cancelled() {
                return run.done(run.stop("cancelled".into(), false), false);
            }
            if !proven {
                if let Err(loss) = job.proof.recheck(phase) {
                    // Byte-exact resume point: inside the body (its offset is exact) or before the
                    // first byte of a call. A half-written fence/control/Return is not resumable.
                    let exact = loss.resumable() && (phase == Phase::Body || off == 0);
                    return run.done(run.stop(loss.reason(), exact), false);
                }
            }
            if first_byte {
                (io.on_first_byte)();
                first_byte = false;
            }
            proven = false;
            let slice = &bytes[off..];
            match writer.write(slice) {
                Ok(0) => return run.done(run.stop("transport:write_zero".into(), false), true),
                Ok(n) => {
                    run.account(phase, n, slice.len());
                    off += n;
                    if phase == Phase::Body {
                        if let Some(body) = job.plan.body.as_deref() {
                            if run.env.body_offset == body.len() {
                                (io.on_body_written)(body);
                            }
                        }
                    }
                    if let Err(e) = writer.flush() {
                        return run.done(run.stop(format!("transport:{e}"), false), true);
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => run.acc.calls += 1,
                Err(e) => return run.done(run.stop(format!("transport:{e}"), false), true),
            }
        }
    }
    let p = &job.plan;
    let return_only = p.body.is_none() && p.control.is_none();
    if (p.submit.is_none() && p.body.is_some()) || (p.submit.is_some() && return_only) {
        *io.last_program_write = Some(Instant::now());
    }
    let t = if p.submit.is_some() {
        Terminal::WrittenSubmitted { accepted: run.acc }
    } else {
        Terminal::WrittenBody { accepted: run.acc }
    };
    run.done(t, false)
}

/// Record the terminal (journal first) and run the producer's settlement exactly once.
pub fn finish(job: AgentJob, ex: &Executed) -> Terminal {
    let mut terminal = ex.terminal.clone();
    if ex.journaled {
        if let Some(j) = &job.journal {
            if let Err(e) = j.terminal(&job.request, &terminal, &ex.envelope) {
                // Terminal persistence failed after possible bytes: never report success (4.5).
                if let Terminal::WrittenBody { accepted } | Terminal::WrittenSubmitted { accepted } = terminal {
                    terminal = Terminal::PartialUnknown {
                        envelope: ex.envelope,
                        accepted,
                        reason: format!("journal_terminal:{e}"),
                    };
                }
            }
        }
    }
    job.request.set(Status::Terminal(terminal.clone()));
    if let Some(settle) = job.settle {
        let t = terminal.clone();
        // A panicking settlement must not kill the writer thread.
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || settle(&t)));
    }
    job.request.settled.store(true, Ordering::Release);
    terminal
}

#[cfg(test)]
thread_local! {
    static SYNC_SETTLE: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
}

/// Tests only: specimens written against the former settle-at-claim timing see the settlement of
/// the request they just handed off (bounded 10 s wait on the calling test thread). A specimen that
/// observes the asynchronous window turns it off with [`set_test_sync_settle`]. No-op in the product.
pub fn test_wait_settled(req: &Request) {
    #[cfg(test)]
    {
        if SYNC_SETTLE.with(|c| c.get()) {
            let deadline = Instant::now() + Duration::from_secs(10);
            while !req.is_settled() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(2));
            }
        }
    }
    #[cfg(not(test))]
    let _ = req;
}

#[cfg(test)]
pub fn set_test_sync_settle(on: bool) {
    SYNC_SETTLE.with(|c| c.set(on));
}

/// Proof used while a seat has no receiver binding wired yet: every check passes (the caller's
/// existing gates and hand-off guard remain the admission).
pub struct PassThrough;

impl Proof for PassThrough {
    fn admit(&self) -> Result<(), ProofLoss> {
        Ok(())
    }
    fn recheck(&self, _: Phase) -> Result<(), ProofLoss> {
        Ok(())
    }
}

/// The journal could not be opened: every barrier fails, so every write is refused before bytes.
pub struct UnavailableJournal(pub String);

impl Journal for UnavailableJournal {
    fn writing(&self, _: &Request) -> std::io::Result<()> {
        Err(std::io::Error::other(self.0.clone()))
    }
    fn terminal(&self, _: &Request, _: &Terminal, _: &Envelope) -> std::io::Result<()> {
        Err(std::io::Error::other(self.0.clone()))
    }
}

/// S4-1 PartialExact resume window, from proof loss (HEAD-RULINGS-W4-1 D3; locked in
/// LOCK-W4-WINDOWS-AND-N-20261016.json before any measurement).
pub const PARTIAL_EXACT_RESUME_WINDOW_MS: u64 = 30_000;

/// File journal size that triggers compaction to the still-open requests.
pub const JOURNAL_COMPACT_BYTES: u64 = 1 << 20;

/// A `writing` record left without a terminal by an earlier daemon run: restores as
/// PartialUnknown (non-replaying reconciliation, design 4.4 step 2 / 4.6).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inconclusive {
    pub id: String,
    pub surface: u64,
    pub origin: String,
    pub digest: String,
}

/// Append-only JSONL journal, `sync_data` per record. Appenders share a read lock (no lock is held
/// across another surface's fsync); compaction alone takes the write lock. Records carry ids,
/// digests and byte counts — never body text.
pub struct FileJournal {
    path: PathBuf,
    file: RwLock<std::fs::File>,
    /// Requests whose last durable record is `writing` (leaf lock; taken under `file`).
    open: Mutex<HashMap<String, String>>,
    bytes: AtomicU64,
    compact_at: u64,
}

impl FileJournal {
    pub fn open(path: PathBuf) -> std::io::Result<(FileJournal, Vec<Inconclusive>)> {
        Self::open_with_limit(path, JOURNAL_COMPACT_BYTES)
    }

    pub fn open_with_limit(path: PathBuf, compact_at: u64) -> std::io::Result<(FileJournal, Vec<Inconclusive>)> {
        let mut open: HashMap<String, String> = HashMap::new();
        let mut order: Vec<String> = Vec::new();
        if let Ok(text) = std::fs::read_to_string(&path) {
            // A torn last line (crash mid-append) does not parse and is ignored.
            for line in text.lines() {
                let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
                let Some(id) = v["id"].as_str() else { continue };
                match v["rec"].as_str() {
                    Some("writing") => {
                        if open.insert(id.to_string(), line.to_string()).is_none() {
                            order.push(id.to_string());
                        }
                    }
                    Some("terminal") => {
                        open.remove(id);
                    }
                    _ => {}
                }
            }
        }
        let inconclusive = order
            .iter()
            .filter_map(|id| open.get(id))
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .map(|v| Inconclusive {
                id: v["id"].as_str().unwrap_or_default().to_string(),
                surface: v["surface"].as_u64().unwrap_or_default(),
                origin: v["origin"].as_str().unwrap_or_default().to_string(),
                digest: v["digest"].as_str().unwrap_or_default().to_string(),
            })
            .collect();
        let file = std::fs::OpenOptions::new().create(true).append(true).open(&path)?;
        let bytes = file.metadata().map(|m| m.len()).unwrap_or(0);
        let j = FileJournal { path, file: RwLock::new(file), open: Mutex::new(open), bytes: AtomicU64::new(bytes), compact_at };
        Ok((j, inconclusive))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn append(f: &std::fs::File, line: &str) -> std::io::Result<()> {
        let mut w = f;
        w.write_all(format!("{line}\n").as_bytes())?;
        f.sync_data()
    }

    /// Rewrite the file to the still-open `writing` records (atomic rename). A failure keeps the
    /// old file: nothing is lost, the file just stays large.
    fn maybe_compact(&self) {
        if self.bytes.load(Ordering::Relaxed) < self.compact_at {
            return;
        }
        let mut file = self.file.write().unwrap_or_else(|e| e.into_inner());
        if self.bytes.load(Ordering::Relaxed) < self.compact_at {
            return;
        }
        let lines: Vec<String> = self.open.lock().unwrap_or_else(|e| e.into_inner()).values().cloned().collect();
        let tmp = self.path.with_extension("jsonl.tmp");
        let rewrite = || -> std::io::Result<std::fs::File> {
            let mut t = std::fs::File::create(&tmp)?;
            for l in &lines {
                t.write_all(format!("{l}\n").as_bytes())?;
            }
            t.sync_all()?;
            std::fs::rename(&tmp, &self.path)?;
            sync_parent_dir(&self.path);
            std::fs::OpenOptions::new().append(true).open(&self.path)
        };
        if let Ok(f) = rewrite() {
            self.bytes.store(f.metadata().map(|m| m.len()).unwrap_or(0), Ordering::Relaxed);
            *file = f;
        }
    }
}

/// Make a rename durable (Unix: fsync the directory; Windows has no directory handle to sync).
fn sync_parent_dir(path: &Path) {
    #[cfg(unix)]
    {
        if let Some(d) = path.parent().and_then(|dir| std::fs::File::open(dir).ok()) {
            let _ = d.sync_all();
        }
    }
    #[cfg(not(unix))]
    let _ = path;
}

impl Journal for FileJournal {
    fn writing(&self, req: &Request) -> std::io::Result<()> {
        let line = json!({
            "rec": "writing",
            "id": req.id,
            "surface": req.surface_id,
            "origin": req.origin,
            "digest": req.digest,
            "ts": crate::state::now_epoch(),
        })
        .to_string();
        let f = self.file.read().unwrap_or_else(|e| e.into_inner());
        self.open.lock().unwrap_or_else(|e| e.into_inner()).insert(req.id.clone(), line.clone());
        let r = Self::append(&f, &line);
        if r.is_err() {
            self.open.lock().unwrap_or_else(|e| e.into_inner()).remove(&req.id);
        }
        self.bytes.fetch_add(line.len() as u64 + 1, Ordering::Relaxed);
        r
    }

    fn terminal(&self, req: &Request, t: &Terminal, envelope: &Envelope) -> std::io::Result<()> {
        let acc = match t {
            Terminal::WrittenBody { accepted }
            | Terminal::WrittenSubmitted { accepted }
            | Terminal::PartialExact { accepted, .. }
            | Terminal::PartialUnknown { accepted, .. } => *accepted,
            _ => Accepted::default(),
        };
        let reason = match t {
            Terminal::RefusedNoWrite { reason }
            | Terminal::PartialExact { reason, .. }
            | Terminal::PartialUnknown { reason, .. } => Some(reason.as_str()),
            _ => None,
        };
        let line = json!({
            "rec": "terminal",
            "id": req.id,
            "kind": t.kind(),
            "reason": reason,
            "payload": acc.payload,
            "fence": acc.fence,
            "control": acc.control,
            "calls": acc.calls,
            "envelope": {
                "cleared": envelope.cleared,
                "opened": envelope.opened,
                "body_offset": envelope.body_offset,
                "closed": envelope.closed,
                "control_done": envelope.control_done,
                "submitted": envelope.submitted,
            },
            "ts": crate::state::now_epoch(),
        })
        .to_string();
        {
            let f = self.file.read().unwrap_or_else(|e| e.into_inner());
            Self::append(&f, &line)?;
            self.open.lock().unwrap_or_else(|e| e.into_inner()).remove(&req.id);
            self.bytes.fetch_add(line.len() as u64 + 1, Ordering::Relaxed);
        }
        self.maybe_compact();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    const KO: &str = "가나다라마바사아자차카타파하";

    fn long_body() -> String {
        // > 2 x 4,096 bytes of 3-byte characters with an ASCII run, so cuts land inside characters.
        let mut s = String::new();
        while s.len() < 10_000 {
            s.push_str(KO);
            s.push_str(" line\n");
        }
        s
    }

    /// Scripted receiver proof: `admit`, then the n-th `recheck` (0-based) fails with `fail`.
    struct FakeProof {
        admit: Result<(), ProofLoss>,
        fail: Mutex<Option<(usize, ProofLoss)>>,
        rechecks: Mutex<Vec<Phase>>,
        log: Option<Arc<Mutex<Vec<String>>>>,
    }

    impl FakeProof {
        fn healthy() -> Arc<FakeProof> {
            Arc::new(FakeProof { admit: Ok(()), fail: Mutex::new(None), rechecks: Mutex::new(Vec::new()), log: None })
        }
        fn failing_at(n: usize, loss: ProofLoss) -> Arc<FakeProof> {
            Arc::new(FakeProof { admit: Ok(()), fail: Mutex::new(Some((n, loss))), rechecks: Mutex::new(Vec::new()), log: None })
        }
        fn refusing(loss: ProofLoss) -> Arc<FakeProof> {
            Arc::new(FakeProof { admit: Err(loss), fail: Mutex::new(None), rechecks: Mutex::new(Vec::new()), log: None })
        }
        fn rechecks(&self) -> usize {
            self.rechecks.lock().unwrap().len()
        }
    }

    impl Proof for FakeProof {
        fn admit(&self) -> Result<(), ProofLoss> {
            if let Some(l) = &self.log {
                l.lock().unwrap().push("admit".into());
            }
            self.admit.clone()
        }
        fn recheck(&self, phase: Phase) -> Result<(), ProofLoss> {
            let mut r = self.rechecks.lock().unwrap();
            let n = r.len();
            r.push(phase);
            match &*self.fail.lock().unwrap() {
                Some((at, loss)) if *at == n => Err(loss.clone()),
                _ => Ok(()),
            }
        }
    }

    #[derive(Clone, Copy)]
    enum Step {
        Short(usize),
        Intr,
        Fail,
        /// Accept the call fully, then cancel the request (a cancellation that arrives while a call is issued).
        FullThenCancel,
    }

    /// Records each accepted call; follows a script of short writes / EINTR / errors.
    struct Rec {
        calls: Vec<Vec<u8>>,
        attempts: usize,
        script: VecDeque<Step>,
        cancel: Option<Arc<Request>>,
        log: Option<Arc<Mutex<Vec<String>>>>,
    }

    impl Rec {
        fn new() -> Rec {
            Rec { calls: Vec::new(), attempts: 0, script: VecDeque::new(), cancel: None, log: None }
        }
        fn bytes(&self) -> Vec<u8> {
            self.calls.concat()
        }
    }

    impl Write for Rec {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.attempts += 1;
            if let Some(l) = &self.log {
                l.lock().unwrap().push("write".into());
            }
            let n = match self.script.pop_front() {
                Some(Step::Short(n)) => n.min(buf.len()),
                Some(Step::Intr) => return Err(std::io::Error::from(std::io::ErrorKind::Interrupted)),
                Some(Step::Fail) => return Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe)),
                Some(Step::FullThenCancel) => {
                    if let Some(r) = &self.cancel {
                        r.cancel();
                    }
                    buf.len()
                }
                None => buf.len(),
            };
            self.calls.push(buf[..n].to_vec());
            Ok(n)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn job(plan: Plan, proof: Arc<FakeProof>) -> AgentJob {
        AgentJob {
            request: Request::new("w-test".into(), 7, "test", "d".into()),
            plan,
            proof,
            journal: None,
            resume: None,
            settle: None,
            guard: None,
            note_body: true,
            body_owner: None,
        }
    }

    fn run_job(w: &mut Rec, j: &AgentJob) -> Executed {
        let mut anchor = None;
        run_job_with(w, j, &mut anchor, &|_| {})
    }

    fn run_job_with(w: &mut Rec, j: &AgentJob, anchor: &mut Option<Instant>, sleep: &dyn Fn(Duration)) -> Executed {
        let mut first = || {};
        let mut body_done = |_: &str| {};
        let mut io = WriterIo { sleep, last_program_write: anchor, on_first_byte: &mut first, on_body_written: &mut body_done };
        execute(w, j, &mut io)
    }

    fn count(hay: &[u8], needle: &[u8]) -> usize {
        hay.windows(needle.len()).filter(|w| *w == needle).count()
    }

    #[test]
    fn paste_plan_bytes_equal_the_existing_inject_sequence() {
        let t = format!("{}\x1b[201~tail {}", long_body(), "\u{9b}201~");
        let mut want = b"\x15".to_vec();
        want.extend_from_slice(cys::paste_fence::wrap(&t).as_bytes());
        want.push(b'\r');
        assert_eq!(Plan::paste(&t).with_clear(150).with_submit(400, 0).planned_bytes(), want);
        assert_eq!(Plan::raw("x\r\ny").planned_bytes(), b"x\r\ny");
        assert_eq!(Plan::submit_only(150).planned_bytes(), b"\r");
    }

    /// S-T03 healthy arm: a long UTF-8 body > 4,096 bytes arrives once, inside one envelope, with
    /// one Return; every call is capped and every call after the admitted first one is re-proven.
    #[test]
    fn long_utf8_body_is_capped_cut_on_char_boundaries_and_rechecked_before_every_call() {
        let body = long_body();
        let plan = Plan::paste(&body).with_clear(0).with_submit(0, 0);
        let p = FakeProof::healthy();
        let j = job(plan.clone(), p.clone());
        let mut w = Rec::new();
        let ex = run_job(&mut w, &j);
        let out = w.bytes();
        assert_eq!(out, plan.planned_bytes());
        assert!(w.calls.len() >= 6, "clear + open + >=3 body + close + return");
        assert!(w.calls.iter().all(|c| c.len() <= MAX_CALL_BYTES));
        for c in &w.calls[2..w.calls.len() - 2] {
            assert!(std::str::from_utf8(c).is_ok(), "body call cut inside a character");
        }
        assert_eq!((count(&out, FENCE_OPEN), count(&out, FENCE_CLOSE), count(&out, b"\r")), (1, 1, 1));
        assert_eq!(p.rechecks(), w.calls.len() - 1, "every call after the admitted first one is re-proven");
        match ex.terminal {
            Terminal::WrittenSubmitted { accepted } => {
                assert_eq!(accepted.payload, body.len() as u64);
                assert_eq!(accepted.fence, (FENCE_OPEN.len() + FENCE_CLOSE.len()) as u64);
                assert_eq!(accepted.control, 2);
            }
            t => panic!("{t:?}"),
        }
    }

    /// S-T03 chunk exit: death proven before call k+1 (after open fence, after a payload prefix,
    /// after the close fence, before the Return, after Ctrl-U) writes nothing after call k.
    #[test]
    fn death_at_every_call_boundary_writes_nothing_after_it() {
        let body = long_body();
        for clear in [false, true] {
            let mut plan = Plan::paste(&body).with_submit(0, 0);
            if clear {
                plan = plan.with_clear(0);
            }
            let calls = calls_for(&plan, &Envelope::default());
            for k in 0..calls.len() - 1 {
                let p = FakeProof::failing_at(k, ProofLoss::AgentDead);
                let j = job(plan.clone(), p);
                let mut w = Rec::new();
                let ex = run_job(&mut w, &j);
                let want: Vec<u8> = calls[..=k].iter().flat_map(|(_, b)| b.clone()).collect();
                assert_eq!(w.bytes(), want, "clear={clear} k={k}: bytes after the failed proof");
                assert_eq!(count(&w.bytes(), b"\r"), 0, "clear={clear} k={k}: a Return reached a dead receiver");
                match ex.terminal {
                    Terminal::PartialUnknown { accepted, reason, .. } => {
                        assert_eq!(accepted.total(), want.len() as u64);
                        assert_eq!(reason, "agent_dead");
                    }
                    t => panic!("clear={clear} k={k}: {t:?}"),
                }
            }
        }
    }

    /// S4-1 at the writer: a non-death Unknown mid-body stops byte-exactly; the SAME request resumes
    /// from its envelope and the two runs together are the planned bytes exactly once.
    #[test]
    fn resumable_loss_mid_body_is_partial_exact_and_resume_completes_once() {
        let body = long_body();
        let plan = Plan::paste(&body).with_submit(0, 0);
        // Calls: open(0) body(1) body(2) body(3) close(4) return(5); recheck index 1 precedes call 2.
        let j = job(plan.clone(), FakeProof::failing_at(1, ProofLoss::Unknown("agent:denied".into())));
        let mut w = Rec::new();
        let ex = run_job(&mut w, &j);
        let env = match ex.terminal {
            Terminal::PartialExact { envelope, reason, .. } => {
                assert_eq!(reason, "unknown:agent:denied");
                envelope
            }
            t => panic!("{t:?}"),
        };
        assert!(env.opened && !env.closed && !env.submitted);
        assert_eq!(env.body_offset, w.bytes().len() - FENCE_OPEN.len());
        let mut resumed = job(plan.clone(), FakeProof::healthy());
        resumed.request = j.request.clone();
        resumed.resume = Some(env);
        let mut w2 = Rec::new();
        let ex2 = run_job(&mut w2, &resumed);
        assert!(matches!(ex2.terminal, Terminal::WrittenSubmitted { .. }), "{:?}", ex2.terminal);
        let mut all = w.bytes();
        all.extend(w2.bytes());
        assert_eq!(all, plan.planned_bytes());
        assert_eq!((count(&all, FENCE_OPEN), count(&all, FENCE_CLOSE), count(&all, b"\r")), (1, 1, 1));
    }

    /// Replacement or epoch movement is not resumable (S4-1: "A proven-dead or replaced binding
    /// stays PartialUnknown"), and neither is a cut inside a fence.
    #[test]
    fn replacement_epoch_move_and_half_fence_are_partial_unknown() {
        let plan = Plan::paste(&long_body()).with_submit(0, 0);
        for loss in [ProofLoss::Changed, ProofLoss::EpochMoved, ProofLoss::RootDead] {
            let j = job(plan.clone(), FakeProof::failing_at(1, loss.clone()));
            let mut w = Rec::new();
            let t = run_job(&mut w, &j).terminal;
            assert!(matches!(t, Terminal::PartialUnknown { .. }), "{loss:?} -> {t:?}");
        }
        // Open fence accepted only 3 of 6 bytes, then a resumable loss: the cut is not byte-exact.
        let j = job(plan, FakeProof::failing_at(0, ProofLoss::Unknown("x".into())));
        let mut w = Rec::new();
        w.script.push_back(Step::Short(3));
        let t = run_job(&mut w, &j).terminal;
        assert!(matches!(t, Terminal::PartialUnknown { .. }), "{t:?}");
        assert_eq!(w.bytes(), b"\x1b[2");
    }

    /// S-T07: short writes and EINTR retries are each a separate OS call with a fresh proof; the
    /// body is reconstructed exactly once.
    #[test]
    fn short_writes_and_eintr_retries_each_get_a_fresh_proof() {
        let body = long_body();
        let plan = Plan::paste(&body).with_submit(0, 0);
        let p = FakeProof::healthy();
        let j = job(plan.clone(), p.clone());
        let mut w = Rec::new();
        w.script.extend([Step::Short(2), Step::Intr, Step::Short(1), Step::Short(1000), Step::Intr, Step::Short(1)]);
        let ex = run_job(&mut w, &j);
        assert_eq!(w.bytes(), plan.planned_bytes());
        assert_eq!(p.rechecks(), w.attempts - 1, "a retry was hidden behind an earlier proof");
        match ex.terminal {
            Terminal::WrittenSubmitted { accepted } => assert_eq!(accepted.calls as usize, w.attempts),
            t => panic!("{t:?}"),
        }
        // A resumable loss right after a short body write stops at the exact body offset.
        let p = FakeProof::failing_at(1, ProofLoss::Unknown("u".into()));
        let j = job(plan.clone(), p);
        let mut w = Rec::new();
        w.script.extend([Step::Short(6), Step::Short(777)]);
        match run_job(&mut w, &j).terminal {
            Terminal::PartialExact { envelope, .. } => assert_eq!(envelope.body_offset, 777),
            t => panic!("{t:?}"),
        }
    }

    /// S-T21a: cancellation before bytes, between returned calls and during prescribed spacing;
    /// never a destructor/cleanup byte and never a later phase.
    #[test]
    fn cancellation_before_between_and_during_spacing_stops_later_calls_only() {
        let body = long_body();
        let plan = Plan::paste(&body).with_clear(5).with_submit(5, 0);
        // (a) before any byte
        let j = job(plan.clone(), FakeProof::healthy());
        j.request.cancel();
        let mut w = Rec::new();
        assert_eq!(run_job(&mut w, &j).terminal, Terminal::CancelledBeforeWrite);
        assert_eq!(w.attempts, 0);
        // (b) cancellation arrives while call 1 (open fence) is issued: it completes, nothing follows.
        let j = job(plan.clone(), FakeProof::healthy());
        let mut w = Rec::new();
        w.cancel = Some(j.request.clone());
        w.script.extend([Step::Short(1), Step::FullThenCancel]);
        let t = run_job(&mut w, &j).terminal;
        assert_eq!(w.bytes(), [CLEAR, FENCE_OPEN].concat());
        assert!(matches!(t, Terminal::PartialUnknown { ref reason, .. } if reason == "cancelled"), "{t:?}");
        // (c) cancellation during the clear settle: nothing after Ctrl-U (the submit spacing has its own test).
        let j = job(plan.clone(), FakeProof::healthy());
        let req = j.request.clone();
        let mut w = Rec::new();
        let mut anchor = None;
        let sleep = move |d: Duration| {
            if d == Duration::from_millis(5) && req.status() == Status::Writing {
                req.cancel();
            }
        };
        let t = run_job_with(&mut w, &j, &mut anchor, &sleep).terminal;
        // The first 5 ms wait is the clear settle: cancelled there, only Ctrl-U was written.
        assert_eq!(w.bytes(), CLEAR);
        assert!(matches!(t, Terminal::PartialUnknown { .. }), "{t:?}");
        let out = w.bytes();
        assert!(!out.contains(&b'\r') && !out.contains(&b'\n') && !out.contains(&0x04));
    }

    #[test]
    fn cancellation_during_submit_spacing_writes_no_return() {
        let plan = Plan::paste("short").with_submit(5, 0);
        let j = job(plan, FakeProof::healthy());
        let req = j.request.clone();
        let mut w = Rec::new();
        let mut anchor = None;
        let t = run_job_with(&mut w, &j, &mut anchor, &move |_| req.cancel()).terminal;
        assert_eq!(w.bytes(), cys::paste_fence::wrap("short").as_bytes());
        assert!(matches!(t, Terminal::PartialUnknown { envelope, .. } if envelope.closed && !envelope.submitted));
    }

    /// A resumed request already owns composer bytes: a failed admission is never RefusedNoWrite.
    #[test]
    fn resumed_request_whose_admission_fails_is_never_refused_no_write() {
        let env = Envelope { opened: true, body_offset: 10, ..Envelope::default() };
        let cases = [
            (ProofLoss::Unknown("u".into()), "partial_exact"),
            (ProofLoss::ContraryForeground, "partial_exact"),
            (ProofLoss::AgentDead, "partial_unknown"),
            (ProofLoss::Changed, "partial_unknown"),
        ];
        for (loss, kind) in cases {
            let mut j = job(Plan::paste(&long_body()).with_submit(0, 0), FakeProof::refusing(loss.clone()));
            j.resume = Some(env);
            let mut w = Rec::new();
            let ex = run_job(&mut w, &j);
            assert_eq!(ex.terminal.kind(), kind, "{loss:?}");
            assert_eq!(ex.envelope, env);
            assert_eq!(w.attempts, 0);
        }
        let mut j = job(Plan::paste("x").with_submit(0, 0), FakeProof::healthy());
        j.resume = Some(env);
        j.request.cancel();
        assert_eq!(run_job(&mut Rec::new(), &j).terminal.kind(), "partial_unknown");
        // A fresh request refused at admission writes zero bytes.
        let j = job(Plan::paste("x").with_submit(0, 0), FakeProof::refusing(ProofLoss::AgentDead));
        let mut w = Rec::new();
        assert_eq!(run_job(&mut w, &j).terminal, Terminal::RefusedNoWrite { reason: "agent_dead".into() });
        assert_eq!(w.attempts, 0);
    }

    struct LogJournal {
        log: Arc<Mutex<Vec<String>>>,
        fail_writing: bool,
        fail_terminal: bool,
    }

    impl Journal for LogJournal {
        fn writing(&self, _: &Request) -> std::io::Result<()> {
            self.log.lock().unwrap().push("writing".into());
            if self.fail_writing {
                return Err(std::io::Error::other("disk"));
            }
            Ok(())
        }
        fn terminal(&self, _: &Request, t: &Terminal, _: &Envelope) -> std::io::Result<()> {
            self.log.lock().unwrap().push(format!("terminal:{}", t.kind()));
            if self.fail_terminal {
                return Err(std::io::Error::other("disk"));
            }
            Ok(())
        }
    }

    fn journaled_job(log: &Arc<Mutex<Vec<String>>>, fail_writing: bool, fail_terminal: bool) -> AgentJob {
        let proof = Arc::new(FakeProof {
            admit: Ok(()),
            fail: Mutex::new(None),
            rechecks: Mutex::new(Vec::new()),
            log: Some(log.clone()),
        });
        let mut j = job(Plan::paste("hello").with_submit(0, 0), proof);
        j.journal = Some(Arc::new(LogJournal { log: log.clone(), fail_writing, fail_terminal }));
        let l = log.clone();
        j.settle = Some(Box::new(move |t: &Terminal| l.lock().unwrap().push(format!("settle:{}", t.kind()))));
        j
    }

    /// Design 4.4 step 2 / 4.5: the durable `writing` record precedes admission and the first byte;
    /// the terminal record precedes settlement; settlement runs exactly once.
    #[test]
    fn journal_barrier_precedes_first_byte_and_terminal_precedes_settle() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let j = journaled_job(&log, false, false);
        let mut w = Rec::new();
        w.log = Some(log.clone());
        let ex = run_job(&mut w, &j);
        let t = finish(j, &ex);
        assert_eq!(t.kind(), "written_submitted");
        let got = log.lock().unwrap().clone();
        assert_eq!(got, ["writing", "admit", "write", "write", "write", "write", "terminal:written_submitted", "settle:written_submitted"]);
        // A failed barrier refuses with zero bytes and never reaches admission.
        let log = Arc::new(Mutex::new(Vec::new()));
        let j = journaled_job(&log, true, false);
        let mut w = Rec::new();
        let ex = run_job(&mut w, &j);
        assert!(matches!(ex.terminal, Terminal::RefusedNoWrite { ref reason } if reason.starts_with("journal:")));
        assert_eq!(w.attempts, 0);
        finish(j, &ex);
        assert_eq!(log.lock().unwrap().clone(), ["writing", "settle:refused_no_write"]);
        // A failed terminal record after bytes never reports success.
        let log = Arc::new(Mutex::new(Vec::new()));
        let j = journaled_job(&log, false, true);
        let ex = run_job(&mut Rec::new(), &j);
        let req = j.request.clone();
        let t = finish(j, &ex);
        assert!(matches!(t, Terminal::PartialUnknown { ref reason, .. } if reason.starts_with("journal_terminal:")), "{t:?}");
        assert_eq!(req.status(), Status::Terminal(t));
        assert_eq!(log.lock().unwrap().last().unwrap(), "settle:partial_unknown");
    }

    #[test]
    fn panicking_settlement_is_contained() {
        let mut j = job(Plan::paste("x"), FakeProof::healthy());
        j.settle = Some(Box::new(|_| panic!("settle bug")));
        let ex = run_job(&mut Rec::new(), &j);
        assert_eq!(finish(j, &ex).kind(), "written_body");
    }

    /// The writer's B2' anchor keeps the existing variant semantics.
    #[test]
    fn program_write_anchor_follows_program_inject_and_submit_after_gap() {
        let none = |plan: Plan| {
            let mut a = None;
            run_job_with(&mut Rec::new(), &job(plan, FakeProof::healthy()), &mut a, &|_| {});
            a
        };
        assert!(none(Plan::raw("body")).is_some(), "body without own Return stamps (Program)");
        assert!(none(Plan::paste("body").with_submit(0, 0)).is_none(), "body + own Return does not stamp (Inject)");
        assert!(none(Plan::control(vec![0x1b])).is_none(), "control does not stamp (Data)");
        // Return-only waits out the remaining minimum gap, then stamps (SubmitAfterGap).
        let slept = Arc::new(Mutex::new(Vec::new()));
        let s2 = slept.clone();
        let mut a = Some(Instant::now() - Duration::from_millis(50));
        let p = FakeProof::healthy();
        let ex = run_job_with(&mut Rec::new(), &job(Plan::submit_only(150), p.clone()), &mut a, &move |d| s2.lock().unwrap().push(d));
        assert_eq!(ex.terminal.kind(), "written_submitted");
        let d = slept.lock().unwrap()[0];
        assert!(d >= Duration::from_millis(80) && d <= Duration::from_millis(100), "{d:?}");
        assert_eq!(p.rechecks(), 1, "the Return after the gap wait is re-proven");
        assert!(a.unwrap().elapsed() < Duration::from_millis(40));
    }

    /// Sink shared with the real writer loop; `fail` makes every write a broken pipe.
    #[derive(Clone)]
    struct Shared {
        buf: Arc<Mutex<Vec<u8>>>,
        fail: bool,
    }

    impl Write for Shared {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            if self.fail {
                return Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe));
            }
            self.buf.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    struct Loop {
        tx: std::sync::mpsc::SyncSender<crate::state::WriteReq>,
        track: Arc<crate::state::InjectTrack>,
        stop: Arc<AtomicBool>,
        handle: std::thread::JoinHandle<()>,
        buf: Arc<Mutex<Vec<u8>>>,
    }

    /// The production writer loop (`run_writer_loop_tracked`) on a shared sink.
    fn spawn_loop(fail: bool) -> Loop {
        let buf = Arc::new(Mutex::new(Vec::new()));
        let sink = Shared { buf: buf.clone(), fail };
        let (tx, rx) = std::sync::mpsc::sync_channel(8);
        let stop = Arc::new(AtomicBool::new(false));
        let track = Arc::new(crate::state::InjectTrack::default());
        let (s2, t2) = (stop.clone(), track.clone());
        let handle = std::thread::spawn(move || crate::state::run_writer_loop_tracked(sink, rx, s2, Some(t2)));
        Loop { tx, track, stop, handle, buf }
    }

    impl Loop {
        fn wait_for(&self, want: &[u8]) {
            let deadline = Instant::now() + Duration::from_secs(5);
            while *self.buf.lock().unwrap() != want {
                assert!(Instant::now() < deadline, "writer output {:?} != {:?}", self.buf.lock().unwrap(), want);
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        fn close(self) {
            self.stop.store(true, Ordering::Relaxed);
            drop(self.tx);
            self.handle.join().unwrap();
        }
    }

    fn settled(mut j: AgentJob) -> (AgentJob, std::sync::mpsc::Receiver<Terminal>) {
        let (tx, rx) = std::sync::mpsc::channel();
        j.settle = Some(Box::new(move |t: &Terminal| {
            let _ = tx.send(t.clone());
        }));
        (j, rx)
    }

    /// Design 4.7 / S-T21a: a safety refusal is a typed outcome — zero bytes, no track marks, and
    /// the writer loop keeps serving the next request (no exit, so no Drop newline+VEOF).
    #[test]
    fn refused_agent_job_on_the_real_writer_loop_writes_nothing_and_keeps_the_loop() {
        let l = spawn_loop(false);
        let (j, rx) = settled(job(Plan::paste("hello").with_submit(0, 0), FakeProof::refusing(ProofLoss::AgentDead)));
        l.tx.send(crate::state::WriteReq::Agent(Box::new(j))).unwrap();
        l.tx.send(crate::state::WriteReq::Data(b"after".to_vec())).unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), Terminal::RefusedNoWrite { reason: "agent_dead".into() });
        l.wait_for(b"after");
        assert!(!l.track.busy_within(Duration::from_secs(60)), "track marked for a request that wrote nothing");
        assert!(l.track.last_body().is_none());
        l.close();
    }

    #[test]
    fn partial_agent_job_on_the_real_writer_loop_keeps_the_loop_and_notes_only_the_prefix() {
        let l = spawn_loop(false);
        let body = long_body();
        let plan = Plan::paste(&body).with_submit(0, 0);
        let (j, rx) = settled(job(plan.clone(), FakeProof::failing_at(1, ProofLoss::AgentDead)));
        l.tx.send(crate::state::WriteReq::Agent(Box::new(j))).unwrap();
        l.tx.send(crate::state::WriteReq::Data(b"after".to_vec())).unwrap();
        let t = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let Terminal::PartialUnknown { envelope, .. } = t else { panic!("{t:?}") };
        let prefix = &body.as_bytes()[..envelope.body_offset];
        let mut want = FENCE_OPEN.to_vec();
        want.extend_from_slice(prefix);
        want.extend_from_slice(b"after");
        l.wait_for(&want);
        let noted = l.track.last_body().expect("accepted prefix noted").norm;
        assert_eq!(noted, crate::state::normalize_input_text(std::str::from_utf8(prefix).unwrap()));
        assert_eq!(l.track.submit_settle_obs(crate::state::settle_mono_ms(), 60_000).since_written_ms, None);
        l.close();
    }

    #[test]
    fn healthy_agent_job_on_the_real_writer_loop_submits_once_and_marks_the_track() {
        let l = spawn_loop(false);
        let plan = Plan::paste("hello agent").with_clear(1).with_submit(1, 0);
        let (j, rx) = settled(job(plan.clone(), FakeProof::healthy()));
        l.tx.send(crate::state::WriteReq::Agent(Box::new(j))).unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap().kind(), "written_submitted");
        l.wait_for(&plan.planned_bytes());
        assert_eq!(l.track.last_body().unwrap().norm, "hello agent");
        assert!(l.track.submit_settle_obs(crate::state::settle_mono_ms(), 60_000).since_written_ms.is_some());
        assert!(l.track.busy_within(Duration::from_secs(60)));
        l.close();
    }

    /// A genuinely broken transport still ends the loop, as before; the record says so truthfully.
    #[test]
    fn transport_error_still_ends_the_writer_loop_with_a_truthful_record() {
        let l = spawn_loop(true);
        let (j, rx) = settled(job(Plan::paste("hello").with_submit(0, 0), FakeProof::healthy()));
        l.tx.send(crate::state::WriteReq::Agent(Box::new(j))).unwrap();
        let t = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(t, Terminal::RefusedNoWrite { ref reason } if reason.starts_with("transport:")), "{t:?}");
        let deadline = Instant::now() + Duration::from_secs(5);
        while !l.handle.is_finished() {
            assert!(Instant::now() < deadline, "writer loop survived a broken transport");
            std::thread::sleep(Duration::from_millis(5));
        }
        l.close();
    }

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("cys-agent-write-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn file_journal_restores_writing_without_terminal_as_inconclusive_and_compacts() {
        let dir = scratch("journal");
        let path = dir.join("writer-journal.jsonl");
        let r1 = Request::new("w1".into(), 3, "queue", "aa".into());
        let r2 = Request::new("w2".into(), 4, "send", "bb".into());
        {
            let (j, inc) = FileJournal::open_with_limit(path.clone(), 2_000).unwrap();
            assert!(inc.is_empty());
            j.writing(&r1).unwrap();
            j.terminal(&r1, &Terminal::WrittenSubmitted { accepted: Accepted::default() }, &Envelope::default()).unwrap();
            j.writing(&r2).unwrap(); // the daemon "crashes" here
            for i in 0..40 {
                let r = Request::new(format!("x{i}"), 5, "queue", "cc".into());
                j.writing(&r).unwrap();
                j.terminal(&r, &Terminal::CancelledBeforeWrite, &Envelope::default()).unwrap();
            }
            assert!(std::fs::metadata(&path).unwrap().len() < 2_000, "compaction did not run");
        }
        // A torn final line from a crash mid-append is ignored.
        let mut f = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(b"{\"rec\":\"term").unwrap();
        drop(f);
        let (_, inc) = FileJournal::open(path.clone()).unwrap();
        assert_eq!(inc, vec![Inconclusive { id: "w2".into(), surface: 4, origin: "send".into(), digest: "bb".into() }]);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("hello") && !text.contains("가나"), "journal must not carry body text");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
