//! F-01 (0.14.52) — display-only CLI activity observer behind the role-coloured tab dot.
//!
//! Design: `DESIGN-U-FROZEN-R3.md` §F-01 R3-1..R3-5, with the head's locked decisions U4-1..U4-5
//! (`HEAD-R3-RULING.md`). This module is the daemon half:
//!
//! * a dedicated OS thread (never the reader, delivery, watchdog or an RPC handler) samples every seat
//!   once per second (S), with missed ticks skipped, never queued;
//! * collection is try-lock only and bounded: at most 15 distinct rows (bottom 8 + cursor ±3) and at
//!   most 256 columns are cloned under the parser lock — no formatting, allocation, regex or grammar
//!   runs under it, and a contended lock is a rejected sample, not a wait;
//! * a frame is admitted only when both output-generation endpoints are even (`frame_published`);
//!   equality between them is NOT required (equality starves streaming seats);
//! * one reducer (H = 3 s dwell of ≥ 4 accepted samples with gaps ≤ 2 s, K = 2 transient rejections,
//!   a 6 s lease from the last accepted positive active sample, U4-1 incomplete frames, U4-2 static
//!   active frame cap) produces the effective state;
//! * the `surface.list` serializer reads the cached record, re-validates identity/view revisions and
//!   enforces the lease itself, so a stalled observer still expires Working.
//!
//! What this module must never do (four fatal risks, design §6): change a queue / delivery / clear /
//! cycle / recovery decision, write to a PTY, send a message, block on a parser or queue lock,
//! hold any lock while classifying or sleeping, or persist Working across a restart. (The roster lock
//! `daemon.surfaces` IS taken blocking, once per tick, only to clone the seat `Arc`s, and the record
//! store is pruned with `retain` over the roster — O(records × seats); only the observer thread can wait
//! there. Seat locks — parser, meta, observation, session, status — are `try_lock` and a busy one skips.) Unknown is not a
//! failure signal and triggers nothing. Rollback / OFF arm: `CYS_DISPLAY_ACTIVITY=0` (the thread is not
//! started and every seat serializes Unknown).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, TryLockError};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::state::{Daemon, Surface};

// ---------------------------------------------------------------------------------------------
// Locked parameters (R3-1 / R3-4 / U4). Changing any of these is a design change, not a tuning.
// ---------------------------------------------------------------------------------------------

/// S — observer period. Missed ticks are skipped, never caught up.
pub(crate) const SAMPLE_PERIOD: Duration = Duration::from_secs(1);
/// H — accepted active samples must span at least this long before Working is established.
pub(crate) const DWELL_SECS: f64 = 3.0;
/// ...and there must be at least this many of them.
pub(crate) const DWELL_MIN_SAMPLES: u32 = 4;
/// No adjacent accepted-sample gap above this may occur inside a dwell.
pub(crate) const MAX_ADJACENT_GAP_SECS: f64 = 2.0;
/// K — consecutive transient rejections an established Working may survive.
pub(crate) const TRANSIENT_K: u32 = 2;
/// Lease from the last accepted positive active sample (strictly below this keeps Working).
pub(crate) const LEASE_SECS: f64 = 6.0;
/// U4-2 — N consecutive accepted active samples with byte-identical non-footer-neutral cells end in
/// Unknown "static active frame". LOCKED (head ruling HEAD-RULING-REPLAY-1, lock file F01-LOCK-STATIC-N.md):
/// the signature covers the copied window CELLS only — never the OSC title, because an animated spinner over
/// a stalled CLI must not read as Working. Corpus quiet-active maximum 25 (codex tall window, censored by the
/// turn length). Measured limit: a codex quiet phase longer than N s in a short-transcript tall window, where
/// the status row sits above the copied window, ends in Unknown (safe direction, display only).
pub(crate) const STATIC_ACTIVE_N: u32 = 120;

/// Bounded copy window (R3-1): bottom rows + cursor row ± span, deduplicated and clipped.
pub(crate) const BOTTOM_ROWS: u16 = 8;
pub(crate) const CURSOR_SPAN: u16 = 3;
pub(crate) const MAX_ROWS: usize = 15;
pub(crate) const MAX_COLS: u16 = 256;
pub(crate) const MAX_CELLS: usize = MAX_ROWS * MAX_COLS as usize;

/// Reducer parameters as one value so tests can exercise boundaries; production uses [`Params::LOCKED`].
#[derive(Clone, Copy, Debug)]
pub(crate) struct Params {
    pub dwell_secs: f64,
    pub dwell_min_samples: u32,
    pub max_gap_secs: f64,
    pub k: u32,
    pub lease_secs: f64,
    pub static_n: u32,
}

impl Params {
    pub(crate) const LOCKED: Params = Params {
        dwell_secs: DWELL_SECS,
        dwell_min_samples: DWELL_MIN_SAMPLES,
        max_gap_secs: MAX_ADJACENT_GAP_SECS,
        k: TRANSIENT_K,
        lease_secs: LEASE_SECS,
        static_n: STATIC_ACTIVE_N,
    };
}

// ---------------------------------------------------------------------------------------------
// Families (R3-2). The family comes from the confirmed agent_meta name only — never from the role.
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Family {
    Claude,
    Codex,
    Gemini,
}

impl Family {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Family::Claude => "claude",
            Family::Codex => "codex",
            Family::Gemini => "gemini",
        }
    }

    /// Explicit aliases from the pinned roster (`agents.json` keys + the agy binary name).
    /// Anything else — a wrapper-only name, an unknown CLI, Grok while unmeasured — is no family.
    pub(crate) fn from_agent_name(name: &str) -> Option<Family> {
        match name.trim().to_ascii_lowercase().as_str() {
            "claude" => Some(Family::Claude),
            "codex" => Some(Family::Codex),
            "gemini" | "agy" => Some(Family::Gemini),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Samples and reasons.
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DecisionKind {
    Permission,
    Question,
}

/// Transient rejections: they may count against K and the lease, never stop Working by themselves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Transient {
    /// parser / metadata / status lock was held by someone else (try-lock only).
    LockContention,
    /// an output-generation endpoint was odd: a chunk was being published.
    OddEndpoint,
    /// U4-1: no complete signature and no positive contradiction (e.g. a repaint split across reads).
    Incomplete,
}

/// Positive contradictions / structural failures: immediate Unknown, no K grace (U4-1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Semantic {
    /// the family's own grammar saw both an active and an idle/decision complete signature.
    Conflict,
    /// another family's complete signature is on screen.
    OtherFamily,
    /// the required region cannot be proven inside the bounded window (>256 cols, empty, scrolled view).
    UnsupportedGeometry,
    /// no confirmed agent family for this seat, or no grammar for that family.
    UnsupportedFamily,
    /// the parser mutex is poisoned (never silently recovered here).
    ParserPoison,
    /// the grammar adapter panicked (caught per seat).
    AdapterPanic,
}

/// Invalidation: candidate and Working are discarded immediately.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Invalid {
    /// an accepted identity write is in progress or completed between collection and publication.
    Identity,
    /// resize / reset / parser replacement / screen-mode change between observations.
    View,
    /// accepted quiesce-on (clear in progress) — reset pending suppresses Working.
    ResetPending,
    /// Windows wrapper observation staged but not yet confirmed.
    Staging,
    /// surface exited or the agent's death was noticed — the existing dead presentation wins.
    Exited,
    /// identity revision counter saturated.
    Overflow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Sample {
    /// accepted positive active frame; `sig` hashes the non-footer-neutral cells (U4-2).
    Active { sig: u64 },
    Idle,
    Decision(DecisionKind),
    Transient(Transient),
    Semantic(Semantic),
    Invalidated(Invalid),
}

impl Transient {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Transient::LockContention => "lock contention",
            Transient::OddEndpoint => "odd output generation",
            Transient::Incomplete => "incomplete frame",
        }
    }
}

impl Semantic {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Semantic::Conflict => "conflicting frame evidence",
            Semantic::OtherFamily => "other family signature",
            Semantic::UnsupportedGeometry => "unsupported geometry",
            Semantic::UnsupportedFamily => "unsupported family",
            Semantic::ParserPoison => "parser poisoned",
            Semantic::AdapterPanic => "adapter panic",
        }
    }
}

impl Invalid {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Invalid::Identity => "identity changed",
            Invalid::View => "view changed",
            Invalid::ResetPending => "reset pending",
            Invalid::Staging => "identity staging",
            Invalid::Exited => "exited",
            Invalid::Overflow => "revision overflow",
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Reducer (R3-1 table + U4-1 + U4-2). Pure and clock-injected: `now` is monotonic seconds.
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Effective {
    Working,
    Waiting,
    WaitingForInput,
    Unknown,
}

impl Effective {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Effective::Working => "working",
            Effective::Waiting => "waiting",
            Effective::WaitingForInput => "waiting_for_input",
            Effective::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Raw {
    Active,
    Idle,
    Decision(DecisionKind),
    Unknown,
}

impl Raw {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Raw::Active => "active",
            Raw::Idle => "idle",
            Raw::Decision(DecisionKind::Permission) => "permission",
            Raw::Decision(DecisionKind::Question) => "question",
            Raw::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Candidate {
    first: f64,
    last: f64,
    count: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Reducer {
    pub(crate) effective: Effective,
    pub(crate) raw: Raw,
    pub(crate) reason: &'static str,
    candidate: Option<Candidate>,
    pub(crate) last_positive: Option<f64>,
    /// consecutive transient rejections while Working; reset only by an accepted active sample.
    pub(crate) rejections: u32,
    pub(crate) retained: bool,
    static_sig: Option<u64>,
    static_count: u32,
    /// monotonic time the effective state last changed (for stable_age_secs).
    pub(crate) stable_since: f64,
}

impl Reducer {
    pub(crate) fn new(now: f64) -> Reducer {
        Reducer {
            effective: Effective::Unknown,
            raw: Raw::Unknown,
            reason: "no observation",
            candidate: None,
            last_positive: None,
            rejections: 0,
            retained: false,
            static_sig: None,
            static_count: 0,
            stable_since: now,
        }
    }

    fn set(&mut self, eff: Effective, now: f64) {
        if self.effective != eff {
            self.effective = eff;
            self.stable_since = now;
        }
    }

    fn drop_working(&mut self, reason: &'static str, now: f64) {
        self.candidate = None;
        self.rejections = 0;
        self.retained = false;
        self.reason = reason;
        self.set(Effective::Unknown, now);
    }

    /// Applies one observation. The match below IS the precedence table — no other rule exists.
    pub(crate) fn step(&mut self, sample: Sample, now: f64, p: &Params) {
        match sample {
            Sample::Active { sig } => {
                self.raw = Raw::Active;
                // U4-2: count consecutive accepted active samples with identical non-neutral cells.
                if self.static_sig == Some(sig) {
                    self.static_count = self.static_count.saturating_add(1);
                } else {
                    self.static_sig = Some(sig);
                    self.static_count = 1;
                }
                if self.static_count >= p.static_n {
                    // stops renewing the lease and ends in Unknown; a changed frame starts over.
                    self.drop_working("static active frame", now);
                    return;
                }
                if self.effective == Effective::Working {
                    self.rejections = 0;
                    self.retained = false;
                    self.last_positive = Some(now);
                    self.reason = "active frame";
                    return;
                }
                let c = match self.candidate {
                    Some(c) if now - c.last <= p.max_gap_secs => Candidate { first: c.first, last: now, count: c.count + 1 },
                    _ => Candidate { first: now, last: now, count: 1 },
                };
                if c.count >= p.dwell_min_samples && c.last - c.first >= p.dwell_secs {
                    self.candidate = None;
                    self.rejections = 0;
                    self.retained = false;
                    self.last_positive = Some(now);
                    self.reason = "active frame";
                    self.set(Effective::Working, now);
                } else {
                    self.candidate = Some(c);
                    self.reason = "active candidate";
                    self.set(Effective::Unknown, now);
                }
            }
            Sample::Idle => {
                self.raw = Raw::Idle;
                self.static_sig = None;
                self.static_count = 0;
                self.candidate = None;
                self.rejections = 0;
                self.retained = false;
                self.reason = "idle frame";
                self.set(Effective::Waiting, now);
            }
            Sample::Decision(kind) => {
                self.raw = Raw::Decision(kind);
                self.static_sig = None;
                self.static_count = 0;
                self.candidate = None;
                self.rejections = 0;
                self.retained = false;
                self.reason = match kind {
                    DecisionKind::Permission => "permission panel",
                    DecisionKind::Question => "question panel",
                };
                self.set(Effective::WaitingForInput, now);
            }
            Sample::Transient(t) => {
                self.raw = Raw::Unknown;
                // a rejection never refreshes the lease and never resets the counter.
                self.candidate = None;
                if self.effective == Effective::Working {
                    self.rejections = self.rejections.saturating_add(1);
                    let fresh = self.last_positive.is_some_and(|lp| now - lp < p.lease_secs);
                    if self.rejections <= p.k && fresh {
                        self.retained = true;
                        self.reason = t.as_str();
                        return;
                    }
                }
                self.drop_working(t.as_str(), now);
            }
            Sample::Semantic(s) => {
                self.raw = Raw::Unknown;
                self.static_sig = None;
                self.static_count = 0;
                self.drop_working(s.as_str(), now);
            }
            Sample::Invalidated(i) => {
                self.raw = Raw::Unknown;
                self.static_sig = None;
                self.static_count = 0;
                self.last_positive = None;
                self.drop_working(i.as_str(), now);
            }
        }
    }

    /// The effective state at `now`, with the lease enforced even if no observation arrived
    /// (the serializer calls this, so a stalled observer still expires Working).
    pub(crate) fn effective_at(&self, now: f64, p: &Params) -> (Effective, &'static str) {
        if self.effective == Effective::Working
            && self.last_positive.map_or(true, |lp| now - lp >= p.lease_secs)
        {
            return (Effective::Unknown, "lease expired");
        }
        (self.effective, self.reason)
    }
}

// ---------------------------------------------------------------------------------------------
// Identity invalidation (R3-2 / U4-4). Display-only atomics on the Surface.
// ---------------------------------------------------------------------------------------------

/// RAII guard taken by an ACCEPTED identity writer around its mutation: pending is raised before the
/// write; on drop (also during unwinding) the revision advances first and pending is lowered after, so
/// any reader that sees `pending == 0` also sees the new revision. Never blocks, never panics, never
/// touches the parser or the activity store — the caller's result, wakeup and PTY behaviour cannot
/// depend on it.
pub(crate) struct IdentityWrite<'a> {
    s: &'a Surface,
}

impl<'a> IdentityWrite<'a> {
    pub(crate) fn begin(s: &'a Surface) -> IdentityWrite<'a> {
        s.activity_pending.fetch_add(1, Ordering::SeqCst);
        IdentityWrite { s }
    }
}

impl Drop for IdentityWrite<'_> {
    fn drop(&mut self) {
        let _ = self
            .s
            .activity_identity_rev
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |r| Some(r.saturating_add(1)));
        self.s.activity_pending.fetch_sub(1, Ordering::SeqCst);
    }
}

/// One-shot invalidation for sites that have no mutation window of their own (e.g. a one-shot session
/// capture that already happened under its own lock).
pub(crate) fn invalidate_identity(s: &Surface) {
    drop(IdentityWrite::begin(s));
}

/// Advances the independent view revision. MUST be called while holding the surface's parser lock
/// (resize, same-size resize, parser replacement) so collection and the bump are ordered.
pub(crate) fn bump_view(s: &Surface) {
    s.activity_view_rev.fetch_add(1, Ordering::Release);
}

// ---------------------------------------------------------------------------------------------
// Bounded collection (R3-1). Try-lock only; clone cells under the lock, stringify after it.
// ---------------------------------------------------------------------------------------------

/// Reusable collection buffer. Capacity is reserved once; collection never grows it beyond
/// [`MAX_CELLS`] / [`MAX_ROWS`], so no allocation happens under the parser lock.
pub(crate) struct Frame {
    pub rows: u16,
    pub cols: u16,
    pub cursor: (u16, u16),
    pub alt: bool,
    pub reset_count: u64,
    pub view_rev: u64,
    pub identity_rev: u64,
    pub row_index: Vec<u16>,
    pub cells: Vec<vt100::Cell>,
    /// OSC 0/2 window title, copied under the lock into reserved capacity (truncated, never grown).
    pub title: String,
    /// a DECSET ?2026 synchronized-output block was open: the frame is mid-update (U4-1 incomplete).
    pub sync: bool,
    /// monotonic hold time of the last parser acquisition (measurement only).
    pub lock_hold: Duration,
}

/// Title bytes kept per sample — enough for every observed family title; longer titles are truncated.
pub(crate) const TITLE_CAP: usize = 256;

impl Frame {
    pub(crate) fn new() -> Frame {
        Frame {
            rows: 0,
            cols: 0,
            cursor: (0, 0),
            alt: false,
            reset_count: 0,
            view_rev: 0,
            identity_rev: 0,
            row_index: Vec::with_capacity(MAX_ROWS),
            cells: Vec::with_capacity(MAX_CELLS),
            title: String::with_capacity(TITLE_CAP),
            sync: false,
            lock_hold: Duration::ZERO,
        }
    }

    /// The copied rows as text (wide-character continuation cells skipped, empty cells as spaces).
    /// Runs after the parser lock is released.
    pub(crate) fn text_rows(&self) -> Vec<(u16, String)> {
        let cols = usize::from(self.cols);
        self.row_index
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let mut line = String::with_capacity(cols);
                for cell in self.cells.iter().skip(i * cols).take(cols) {
                    if cell.is_wide_continuation() {
                        continue;
                    }
                    if cell.has_contents() {
                        line.push_str(&cell.contents());
                    } else {
                        line.push(' ');
                    }
                }
                (*r, line)
            })
            .collect()
    }
}

/// Rows to copy: bottom [`BOTTOM_ROWS`] plus cursor ± [`CURSOR_SPAN`], clipped, deduplicated, ascending.
/// At most 15 rows by construction.
pub(crate) fn window_rows(rows: u16, cursor_row: u16, out: &mut Vec<u16>) {
    out.clear();
    if rows == 0 {
        return;
    }
    let bottom_start = rows.saturating_sub(BOTTOM_ROWS);
    let cur_lo = cursor_row.saturating_sub(CURSOR_SPAN);
    let cur_hi = cursor_row.saturating_add(CURSOR_SPAN).min(rows - 1);
    for r in 0..rows {
        let in_bottom = r >= bottom_start;
        let in_cursor = cursor_row < rows && r >= cur_lo && r <= cur_hi;
        if in_bottom || in_cursor {
            out.push(r);
        }
    }
}

#[cfg(test)]
thread_local! {
    /// Runs inside [`collect`] after the parser lock is released and before the second endpoint read.
    pub(crate) static AFTER_COPY: std::cell::RefCell<Option<Box<dyn FnMut()>>> = const { std::cell::RefCell::new(None) };
}

/// Test-only measurement sink and knobs for the U4-3 lock-cost harness (`activity_bench_tests.rs`).
/// Nothing records unless [`bench::ENABLED`] (timing probes) or [`bench::PTY_CAPTURE`] (daemon → PTY
/// writes) is set; both are false by default and only that harness sets them, so other tests are
/// unaffected. Compiled only under `cfg(test)`: the release binary contains none of it.
#[cfg(test)]
pub(crate) mod bench {
    use std::cell::Cell;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    /// observer hold / per-seat / per-tick and reader wait / critical-section samples record while set.
    pub(crate) static ENABLED: AtomicBool = AtomicBool::new(false);
    /// every daemon → PTY write (all seats) is captured while set.
    pub(crate) static PTY_CAPTURE: AtomicBool = AtomicBool::new(false);
    /// SLEEP arm: milliseconds slept after a seat's parser lock is released, before classification (0 = off).
    pub(crate) static SLEEP_MS: AtomicU64 = AtomicU64::new(0);

    #[derive(Clone, Copy, Debug)]
    pub(crate) struct Rec {
        pub at: Instant,
        pub sid: u64,
        pub a: Duration,
        pub b: Duration,
    }

    #[derive(Clone, Debug)]
    pub(crate) struct PtyWrite {
        pub at: Instant,
        pub sid: u64,
        pub len: usize,
        /// the written bytes, truncated to the first 4096 (classification: DSR reply vs other, A1-1).
        pub head: Vec<u8>,
    }

    #[derive(Default, Debug)]
    pub(crate) struct Sink {
        /// a = observer parser-lock hold (every successful try_lock, before any later rejection).
        pub hold: Vec<Rec>,
        /// a = one `observe_seat` call minus slept time, b = slept time.
        pub seat: Vec<Rec>,
        /// sid = seats in the roster snapshot, a = whole `observe_tick` wall, b = slept within the tick.
        pub tick: Vec<Rec>,
        /// at = just before `surf.parser.lock()`, a = lock wait, b = critical section (chunk processing).
        pub reader: Vec<Rec>,
        /// U4-3 R2 attribution: the reader thread's CPU time inside the same critical section (index-aligned with `reader`;
        /// None where the platform has no per-thread CPU clock).
        pub reader_cpu: Vec<Option<Duration>>,
        pub pty: Vec<PtyWrite>,
        /// sleeps performed by the SLEEP knob while ENABLED (= sleeping classifications).
        pub sleeps: u64,
    }

    static SINK: Mutex<Option<Sink>> = Mutex::new(None);

    thread_local! {
        static SEAT_SLEPT: Cell<Duration> = const { Cell::new(Duration::ZERO) };
        static TICK_SLEPT: Cell<Duration> = const { Cell::new(Duration::ZERO) };
        /// A2-1: fsync time / calls on THIS thread (only the harness delivery thread reads its own).
        static FSYNC_SPAN: Cell<Duration> = const { Cell::new(Duration::ZERO) };
        static FSYNC_CALLS: Cell<u64> = const { Cell::new(0) };
    }

    /// A2-1 fsync span guard: created right before each `sync_data`/`sync_all` reachable from `deliver_queued`
    /// and dropped right after it (also on an early `?` return), adding the elapsed time and one call to this
    /// thread's accumulator. Inert (no clock read, nothing recorded) unless the bench is enabled.
    pub(crate) struct FsyncSpan(Option<Instant>);

    impl FsyncSpan {
        pub(crate) fn begin() -> FsyncSpan {
            FsyncSpan(on().then(Instant::now))
        }
    }

    impl Drop for FsyncSpan {
        fn drop(&mut self) {
            if let Some(t0) = self.0.take() {
                let d = t0.elapsed();
                FSYNC_SPAN.with(|c| c.set(c.get() + d));
                FSYNC_CALLS.with(|c| c.set(c.get() + 1));
            }
        }
    }

    /// Returns and resets this thread's accumulated (fsync time, fsync calls).
    pub(crate) fn fsync_take() -> (Duration, u64) {
        (FSYNC_SPAN.with(|c| c.replace(Duration::ZERO)), FSYNC_CALLS.with(|c| c.replace(0)))
    }

    /// Installs an empty sink with pre-reserved capacity (no reallocation under load).
    pub(crate) fn install(reader: usize, observer: usize, ticks: usize, pty: usize) {
        let s = Sink {
            hold: Vec::with_capacity(observer),
            seat: Vec::with_capacity(observer),
            tick: Vec::with_capacity(ticks),
            reader: Vec::with_capacity(reader),
            reader_cpu: Vec::with_capacity(reader),
            pty: Vec::with_capacity(pty),
            sleeps: 0,
        };
        *SINK.lock().unwrap_or_else(|e| e.into_inner()) = Some(s);
    }

    pub(crate) fn take() -> Sink {
        SINK.lock().unwrap_or_else(|e| e.into_inner()).take().unwrap_or_default()
    }

    fn with(f: impl FnOnce(&mut Sink)) {
        if let Some(s) = SINK.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            f(s);
        }
    }

    fn on() -> bool {
        ENABLED.load(Ordering::Acquire)
    }

    /// called after the parser guard is dropped (never inside the lock).
    pub(crate) fn hold(sid: u64, at: Instant, d: Duration) {
        if on() {
            with(|s| s.hold.push(Rec { at, sid, a: d, b: Duration::ZERO }));
        }
    }

    /// SLEEP knob — runs after unlock, before classification.
    pub(crate) fn sleep_knob() {
        let ms = SLEEP_MS.load(Ordering::Acquire);
        if ms == 0 {
            return;
        }
        let t = Instant::now();
        std::thread::sleep(Duration::from_millis(ms));
        let slept = t.elapsed();
        SEAT_SLEPT.with(|c| c.set(c.get() + slept));
        if on() {
            with(|s| s.sleeps += 1);
        }
    }

    pub(crate) fn seat_begin() -> Instant {
        SEAT_SLEPT.with(|c| c.set(Duration::ZERO));
        Instant::now()
    }

    pub(crate) fn seat_end(sid: u64, t0: Instant) {
        let total = t0.elapsed();
        let slept = SEAT_SLEPT.with(|c| c.replace(Duration::ZERO));
        TICK_SLEPT.with(|c| c.set(c.get() + slept));
        if on() {
            with(|s| s.seat.push(Rec { at: t0, sid, a: total.saturating_sub(slept), b: slept }));
        }
    }

    pub(crate) fn tick_begin() -> Instant {
        TICK_SLEPT.with(|c| c.set(Duration::ZERO));
        Instant::now()
    }

    pub(crate) fn tick_end(t0: Instant, seats: usize) {
        let total = t0.elapsed();
        let slept = TICK_SLEPT.with(|c| c.replace(Duration::ZERO));
        if on() {
            with(|s| s.tick.push(Rec { at: t0, sid: seats as u64, a: total, b: slept }));
        }
    }

    /// reader: t0 = just before `surf.parser.lock()`, t1 = acquired, t2 = end of the critical section;
    /// cpu = this thread's CPU time between t1 and t2 (U4-3 R2 attribution: wall - cpu = time off-CPU while holding the lock).
    pub(crate) fn reader(sid: u64, t0: Instant, t1: Instant, t2: Instant, cpu: Option<Duration>) {
        if on() {
            with(|s| {
                s.reader.push(Rec { at: t0, sid, a: t1.duration_since(t0), b: t2.duration_since(t1) });
                s.reader_cpu.push(cpu);
            });
        }
    }

    /// U4-3 R2: the calling thread's CPU time (unix `CLOCK_THREAD_CPUTIME_ID`) — only while the bench is enabled, so a
    /// disabled bench adds no clock read. None on other platforms or on a clock error.
    pub(crate) fn thread_cpu_if_on() -> Option<Duration> {
        if !on() {
            return None;
        }
        #[cfg(unix)]
        {
            let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
            // SAFETY: `ts` is a valid, writable timespec; the clock id is a constant supported on macOS (10.12+) and Linux.
            let rc = unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut ts) };
            if rc == 0 && ts.tv_sec >= 0 && ts.tv_nsec >= 0 {
                return Some(Duration::new(ts.tv_sec as u64, ts.tv_nsec as u32));
            }
            None
        }
        #[cfg(not(unix))]
        {
            None
        }
    }

    pub(crate) fn pty_write(sid: u64, bytes: &[u8]) {
        if PTY_CAPTURE.load(Ordering::Acquire) {
            let head = bytes[..bytes.len().min(4096)].to_vec();
            with(|s| s.pty.push(PtyWrite { at: Instant::now(), sid, len: bytes.len(), head }));
        }
    }

    /// Wraps a seat's PTY writer and reports every successful write to [`pty_write`].
    pub(crate) struct CountingWriter<W: std::io::Write> {
        inner: W,
        sid: u64,
    }

    impl<W: std::io::Write> CountingWriter<W> {
        pub(crate) fn new(inner: W, sid: u64) -> Self {
            CountingWriter { inner, sid }
        }
    }

    impl<W: std::io::Write> std::io::Write for CountingWriter<W> {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            let n = self.inner.write(buf)?;
            pty_write(self.sid, &buf[..n]);
            Ok(n)
        }

        fn flush(&mut self) -> std::io::Result<()> {
            self.inner.flush()
        }
    }
}

pub(crate) enum Collected {
    Frame,
    Rejected(Sample),
}

/// Copies the bounded window of `screen` into `frame`. In production this is the ONLY work done under
/// the parser lock (no formatting, allocation or grammar); the corpus replay calls the same function on
/// its own parser so live and replay share one copy path. `false` = unsupported geometry.
pub(crate) fn copy_window(screen: &vt100::Screen, frame: &mut Frame) -> bool {
    let (rows, cols) = screen.size();
    let cursor = screen.cursor_position();
    let scrolled = screen.scrollback() != 0;
    frame.rows = rows;
    frame.cols = cols;
    frame.cursor = cursor;
    frame.alt = screen.alternate_screen();
    frame.reset_count = screen.reset_count();
    frame.sync = screen.synchronized_output();
    // one byte-slice copy cut at a char boundary into reserved capacity (U4-3 hold mitigation: no per-char loop).
    frame.title.clear();
    let title = screen.title();
    let mut end = title.len().min(TITLE_CAP);
    while !title.is_char_boundary(end) {
        end -= 1;
    }
    frame.title.push_str(&title[..end]);
    frame.cells.clear();
    if rows == 0 || cols == 0 || cols > MAX_COLS || scrolled {
        frame.row_index.clear();
        return false;
    }
    window_rows(rows, cursor.0, &mut frame.row_index);
    // each window row is resolved once and copied as one contiguous slice (`Cell: Copy` → memcpy); before the
    // U4-3 mitigation every cell re-resolved its row through the scrollback iterator. A row shorter than `cols`
    // (not produced by the grid, kept for safety) is padded with default cells, as the per-cell copy did.
    let width = usize::from(cols);
    for &r in &frame.row_index {
        let row = screen.visible_row_cells(r).unwrap_or(&[]);
        let n = row.len().min(width);
        frame.cells.extend_from_slice(&row[..n]);
        frame.cells.resize(frame.cells.len() + (width - n), vt100::Cell::default());
    }
    true
}

/// Classifies a collected frame for `family`; runs after the parser lock is released. Shared by the live
/// observer and the corpus replay. An open synchronized-output block is a frame mid-update (U4-1): no
/// complete signature can be claimed and no contradiction shown. A panicking adapter is caught here.
pub(crate) fn sample_frame(family: Family, frame: &Frame, grammars: &[&dyn Grammar]) -> (Sample, Option<&'static str>) {
    if frame.sync {
        return (Sample::Transient(Transient::Incomplete), None);
    }
    let text = frame.text_rows();
    let view = View { rows: &text, cursor: frame.cursor, title: &frame.title };
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| classify_with(family, &view, grammars)))
        .unwrap_or((Sample::Semantic(Semantic::AdapterPanic), None))
}

/// Collects one bounded frame from `s` into `frame`. Never blocks on any lock.
pub(crate) fn collect(s: &Surface, frame: &mut Frame) -> Collected {
    let pend0 = s.activity_pending.load(Ordering::SeqCst);
    let ident0 = s.activity_identity_rev.load(Ordering::SeqCst);
    if pend0 != 0 {
        return Collected::Rejected(Sample::Invalidated(Invalid::Identity));
    }
    if ident0 == u64::MAX {
        return Collected::Rejected(Sample::Invalidated(Invalid::Overflow));
    }
    let gen0 = s.output_gen.load(Ordering::Acquire);
    let guard = match s.parser.try_lock() {
        Ok(g) => g,
        Err(TryLockError::WouldBlock) => return Collected::Rejected(Sample::Transient(Transient::LockContention)),
        Err(TryLockError::Poisoned(_)) => return Collected::Rejected(Sample::Semantic(Semantic::ParserPoison)),
    };
    let t0 = Instant::now();
    frame.view_rev = s.activity_view_rev.load(Ordering::Acquire);
    let supported = copy_window(guard.screen(), frame);
    frame.lock_hold = t0.elapsed();
    drop(guard);
    #[cfg(test)]
    bench::hold(s.id, t0, frame.lock_hold);
    if !supported {
        return Collected::Rejected(Sample::Semantic(Semantic::UnsupportedGeometry));
    }
    // test-only seam: a deterministic interleaving between unlock and the second endpoint read.
    #[cfg(test)]
    AFTER_COPY.with(|h| {
        if let Some(f) = h.borrow_mut().as_mut() {
            f();
        }
    });
    let gen1 = s.output_gen.load(Ordering::Acquire);
    if gen0 % 2 != 0 || gen1 % 2 != 0 {
        return Collected::Rejected(Sample::Transient(Transient::OddEndpoint));
    }
    let pend1 = s.activity_pending.load(Ordering::SeqCst);
    let ident1 = s.activity_identity_rev.load(Ordering::SeqCst);
    if pend1 != 0 || ident1 != ident0 {
        return Collected::Rejected(Sample::Invalidated(Invalid::Identity));
    }
    frame.identity_rev = ident0;
    Collected::Frame
}

// ---------------------------------------------------------------------------------------------
// Grammar adapters (versioned, per family). Production adapters are derived ONLY from the native
// capture corpus (`validation/F01-NATIVE-CORPUS-*`); none is registered without it.
// ---------------------------------------------------------------------------------------------

/// What a frame looks like to the bounded classifier — text rows (after unlock) plus geometry.
pub(crate) struct View<'a> {
    pub rows: &'a [(u16, String)],
    pub cursor: (u16, u16),
    /// OSC window title (not an editable region: a draft cannot change it).
    pub title: &'a str,
}

/// One grammar's verdict on ITS OWN complete signatures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// a complete active signature; `sig` hashes the non-footer-neutral cells for U4-2.
    Active { sig: u64 },
    Idle,
    Decision(DecisionKind),
    /// both an active and an idle/decision complete signature (own grammar conflict).
    Conflict,
    /// the required region cannot be proven inside the window.
    Unsupported,
    /// no complete signature of this grammar.
    NoMatch,
}

pub(crate) trait Grammar: Sync {
    fn family(&self) -> Family;
    fn revision(&self) -> &'static str;
    fn classify(&self, v: &View<'_>) -> Verdict;
}

/// Production grammar registry. Each grammar is derived from the native capture corpus
/// (`_evidence/dept5-0.14.52-P3-impl/U/validation/f01/FINDINGS-<family>.md`, corpus/INDEX.json sha256 bec81de5…).
/// gemini/agy has no grammar yet: its only route is read-only observation of a live seat (head ruling
/// 2026-10-16) and no working frame has been observed, so that family stays Unknown (PROVISIONAL).
pub(crate) fn grammars() -> &'static [&'static dyn Grammar] {
    static ALL: [&dyn Grammar; 2] = [&CLAUDE_R1, &CODEX_R1];
    &ALL
}

fn row_at<'a>(v: &'a View<'_>, r: u16) -> Option<&'a str> {
    v.rows.iter().find(|(i, _)| *i == r).map(|(_, l)| l.as_str())
}

fn hash_rows(v: &View<'_>, mask: impl Fn(&str) -> String) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for (i, l) in v.rows {
        i.hash(&mut h);
        mask(l).hash(&mut h);
    }
    h.finish()
}

/// Replaces the first non-space character of a row when it is one of `glyphs` (animation-only cells).
fn mask_lead(line: &str, glyphs: &[char]) -> String {
    let trimmed = line.trim_start();
    match trimmed.chars().next() {
        Some(c) if glyphs.contains(&c) => {
            let lead = line.len() - trimmed.len();
            format!("{}*{}", &line[..lead], &trimmed[c.len_utf8()..])
        }
        _ => line.to_string(),
    }
}

/// claude 2.1.301 (FINDINGS-claude §2–§5): OSC title `◐|◑ Claude Code` while working (model wait, streaming,
/// tool — 99.1–100 %), `✳ Claude Code` when idle AND in permission/question panels. The composer is a `❯` row
/// framed by `─` rules with the (hidden) cursor parked in it; a frame without that structure is mid-redraw.
pub(crate) struct ClaudeR1;
pub(crate) static CLAUDE_R1: ClaudeR1 = ClaudeR1;
const CLAUDE_SPINNER: [char; 6] = ['·', '✢', '✳', '✶', '✻', '✽'];

impl ClaudeR1 {
    fn is_rule(l: &str) -> bool {
        l.trim_start().starts_with('─')
    }
    /// A `❯` row at or above the cursor row (scanning up through draft lines, stopping at a rule), with a rule
    /// row directly above it and a rule row somewhere below the cursor row.
    fn composer_intact(v: &View<'_>) -> bool {
        let cur = v.cursor.0;
        let mut r = cur;
        loop {
            match row_at(v, r) {
                Some(l) if l.trim_start().starts_with('❯') => break,
                Some(l) if Self::is_rule(l) => return false,
                Some(_) => {}
                None => return false,
            }
            if r == 0 {
                return false;
            }
            r -= 1;
        }
        let above = r.checked_sub(1).and_then(|a| row_at(v, a)).is_some_and(Self::is_rule);
        let below = v.rows.iter().any(|(i, l)| *i > cur && Self::is_rule(l));
        above && below
    }
}

impl Grammar for ClaudeR1 {
    fn family(&self) -> Family {
        Family::Claude
    }
    fn revision(&self) -> &'static str {
        "claude-2.1.301-r1"
    }
    fn classify(&self, v: &View<'_>) -> Verdict {
        let title = v.title.trim();
        if !title.ends_with("Claude Code") {
            return Verdict::NoMatch;
        }
        let glyph = title.chars().next();
        let active = matches!(glyph, Some('◐' | '◑'));
        let resting = glyph == Some('✳');
        let trimmed = || v.rows.iter().map(|(_, l)| l.trim());
        let permission = trimmed().any(|l| l.starts_with("Esc to cancel · Tab to amend") || l == "Do you want to proceed?");
        let question = trimmed().any(|l| l.starts_with("Enter to select · ↑/↓ to navigate · Esc to cancel"));
        if active && (permission || question) {
            return Verdict::Conflict;
        }
        if resting && question {
            return Verdict::Decision(DecisionKind::Question);
        }
        if resting && permission {
            return Verdict::Decision(DecisionKind::Permission);
        }
        if !Self::composer_intact(v) {
            return Verdict::NoMatch;
        }
        if active {
            return Verdict::Active { sig: hash_rows(v, |l| mask_lead(l, &CLAUDE_SPINNER)) };
        }
        if resting {
            return Verdict::Idle;
        }
        Verdict::NoMatch
    }
}

/// codex-cli 0.162.1 (FINDINGS-codex §2–§6): alternate screen; OSC title = braille spinner + cwd while working
/// (100 % of active phases), plain cwd when idle, `[ ! ]/[ . ] Action Required | cwd` in permission/question
/// panels. The cursor sits on the `›` composer row after every frame.
pub(crate) struct CodexR1;
pub(crate) static CODEX_R1: CodexR1 = CodexR1;

impl Grammar for CodexR1 {
    fn family(&self) -> Family {
        Family::Codex
    }
    fn revision(&self) -> &'static str {
        "codex-0.162.1-r1"
    }
    fn classify(&self, v: &View<'_>) -> Verdict {
        let title = v.title.trim();
        if title.is_empty() {
            return Verdict::NoMatch;
        }
        if title.starts_with("[ ! ] Action Required") || title.starts_with("[ . ] Action Required") {
            let question = v
                .rows
                .iter()
                .any(|(_, l)| l.contains("enter to submit answer") || l.trim_start().starts_with("Question "));
            return Verdict::Decision(if question { DecisionKind::Question } else { DecisionKind::Permission });
        }
        let composer = row_at(v, v.cursor.0).is_some_and(|l| l.trim_start().starts_with('›'));
        if !composer {
            return Verdict::NoMatch;
        }
        let spinning = title.chars().next().is_some_and(|c| ('\u{2800}'..='\u{28FF}').contains(&c));
        if spinning {
            Verdict::Active { sig: hash_rows(v, |l| mask_lead(l, &['•', '◦'])) }
        } else {
            Verdict::Idle
        }
    }
}

/// Classifies a view for `family` against `grammars` (U4-1 precedence):
/// own complete signature → that sample; another family's complete signature → positive contradiction;
/// own conflict / unsupported region → positive contradiction; nothing complete and no contradiction →
/// transient Incomplete (counts against K and the lease, never ends Working by itself).
pub(crate) fn classify_with(
    family: Family,
    v: &View<'_>,
    all: &[&dyn Grammar],
) -> (Sample, Option<&'static str>) {
    let Some(own) = all.iter().find(|g| g.family() == family) else {
        return (Sample::Semantic(Semantic::UnsupportedFamily), None);
    };
    let rev = Some(own.revision());
    let other = all
        .iter()
        .filter(|g| g.family() != family)
        .any(|g| matches!(g.classify(v), Verdict::Active { .. } | Verdict::Idle | Verdict::Decision(_)));
    let sample = match own.classify(v) {
        Verdict::Unsupported => Sample::Semantic(Semantic::UnsupportedGeometry),
        Verdict::Conflict => Sample::Semantic(Semantic::Conflict),
        _ if other => Sample::Semantic(Semantic::OtherFamily),
        Verdict::Active { sig } => Sample::Active { sig },
        Verdict::Idle => Sample::Idle,
        Verdict::Decision(k) => Sample::Decision(k),
        Verdict::NoMatch => Sample::Transient(Transient::Incomplete),
    };
    (sample, rev)
}

// ---------------------------------------------------------------------------------------------
// Store, observer thread and serializer.
// ---------------------------------------------------------------------------------------------

/// Per-epoch identity: a change starts a new display epoch and a fresh reducer.
#[derive(Clone, Debug, PartialEq, Eq)]
struct EpochKey {
    identity_rev: u64,
    family: Option<Family>,
    meta: Option<(String, String)>,
    session: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ViewKey {
    view_rev: u64,
    reset_count: u64,
    rows: u16,
    cols: u16,
    alt: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct Record {
    key: EpochKey,
    view: Option<ViewKey>,
    pub(crate) epoch: String,
    pub(crate) seq: u64,
    pub(crate) reducer: Reducer,
    pub(crate) family: Option<Family>,
    pub(crate) grammar_rev: Option<&'static str>,
    pub(crate) observed_at: f64,
    pub(crate) identity_rev: u64,
    pub(crate) view_rev: u64,
}

/// Bounded, measurement-only counters (no per-sample storage).
#[derive(Clone, Debug, Default)]
pub(crate) struct Stats {
    pub ticks: u64,
    pub ticks_skipped: u64,
    pub attempts: u64,
    pub accepted: u64,
    pub transient: [u64; 3],
    pub semantic: u64,
    pub invalidated: u64,
    /// parser-lock hold histogram, bucket i = [2^i, 2^(i+1)) microseconds (bucket 0 also holds < 1 µs).
    pub hold_us_log2: [u64; 24],
    pub hold_us_max: u64,
    /// aggregate collect+classify time per tick, microseconds, same bucketing.
    pub tick_us_log2: [u64; 24],
    pub tick_us_max: u64,
}

fn log2_bucket(us: u64) -> usize {
    (63 - us.max(1).leading_zeros() as usize).min(23)
}

pub(crate) struct ActivityStore {
    /// display-only nonce minted once; `None` = display disabled (every seat serializes Unknown).
    pub(crate) nonce: Option<String>,
    pub(crate) running: AtomicBool,
    records: Mutex<HashMap<u64, Record>>,
    pub(crate) stats: Mutex<Stats>,
}

impl ActivityStore {
    pub(crate) fn new() -> ActivityStore {
        // a separate random value — never a seat or operator token (R3-2).
        let nonce = crate::channels::random_token_hex().ok().map(|h| h.chars().take(16).collect());
        ActivityStore { nonce, running: AtomicBool::new(false), records: Mutex::new(HashMap::new()), stats: Mutex::new(Stats::default()) }
    }

}

fn epoch_id(nonce: &str, surface: u64, key: &EpochKey) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    nonce.hash(&mut h);
    surface.hash(&mut h);
    key.hash(&mut h);
    format!("{:016x}", h.finish())
}

impl std::hash::Hash for EpochKey {
    fn hash<H: std::hash::Hasher>(&self, h: &mut H) {
        self.identity_rev.hash(h);
        self.family.map(Family::as_str).hash(h);
        self.meta.hash(h);
        self.session.hash(h);
    }
}

/// Reads a seat's identity/reset inputs with try-locks only. `Err` = a sample that ends here.
fn identity_inputs(s: &Surface, now_epoch: f64) -> Result<EpochKey, Sample> {
    if s.exited.load(Ordering::Relaxed) || s.agent_exit_notified.load(Ordering::Relaxed) {
        return Err(Sample::Invalidated(Invalid::Exited));
    }
    let identity_rev = s.activity_identity_rev.load(Ordering::SeqCst);
    let meta = match s.agent_meta.try_lock() {
        Ok(g) => g.clone(),
        Err(TryLockError::WouldBlock) => return Err(Sample::Transient(Transient::LockContention)),
        Err(TryLockError::Poisoned(_)) => return Err(Sample::Semantic(Semantic::UnsupportedFamily)),
    };
    match s.pending_agent_obs.try_lock() {
        Ok(g) if g.is_some() => return Err(Sample::Invalidated(Invalid::Staging)),
        Ok(_) => {}
        Err(TryLockError::WouldBlock) => return Err(Sample::Transient(Transient::LockContention)),
        Err(TryLockError::Poisoned(_)) => return Err(Sample::Invalidated(Invalid::Staging)),
    }
    let session = match s.agent_session_id.try_lock() {
        Ok(g) => g.clone(),
        Err(TryLockError::WouldBlock) => return Err(Sample::Transient(Transient::LockContention)),
        Err(TryLockError::Poisoned(_)) => None,
    };
    // read-only cycle observation: a live quiescing window means a clear is in progress.
    // (never calls the releasing helpers — `effective_quiescing_since` mutates.) The bound is the fixed
    // default hold, not the operational knob: `0` switches the queue axis off, but an unbounded
    // display suppression would leave an orphaned window Unknown forever.
    match s.agent_status.try_lock() {
        Ok(g) => {
            if let Some(st) = g.as_ref().filter(|st| st.state == "quiescing") {
                if crate::governance::quiesce_hold_active(
                    Some(now_epoch - st.updated_at),
                    crate::governance::QUEUE_QUIESCE_HOLD_DEFAULT_SECS,
                ) {
                    return Err(Sample::Invalidated(Invalid::ResetPending));
                }
            }
        }
        Err(TryLockError::WouldBlock) => return Err(Sample::Transient(Transient::LockContention)),
        Err(TryLockError::Poisoned(_)) => {}
    }
    let family = meta.as_ref().and_then(|(name, _)| Family::from_agent_name(name));
    Ok(EpochKey { identity_rev, family, meta, session })
}

/// Observes one seat: identity → collect → classify → reduce → version-checked publication.
pub(crate) fn observe_seat(
    store: &ActivityStore,
    s: &Surface,
    frame: &mut Frame,
    grammars: &[&dyn Grammar],
    now: f64,
    now_epoch: f64,
    stats: &mut Stats,
) {
    let Some(nonce) = store.nonce.as_deref() else { return };
    stats.attempts += 1;
    let key = identity_inputs(s, now_epoch);
    let mut view_now: Option<ViewKey> = None;
    let mut grammar_rev = None;
    let sample = match &key {
        Err(sample) => *sample,
        Ok(k) => match k.family {
            None => Sample::Semantic(Semantic::UnsupportedFamily),
            Some(fam) => match collect(s, frame) {
                Collected::Rejected(sample) => sample,
                Collected::Frame => {
                    let hold = frame.lock_hold.as_micros().min(u128::from(u64::MAX)) as u64;
                    stats.hold_us_log2[log2_bucket(hold)] += 1;
                    stats.hold_us_max = stats.hold_us_max.max(hold);
                    view_now = Some(ViewKey {
                        view_rev: frame.view_rev,
                        reset_count: frame.reset_count,
                        rows: frame.rows,
                        cols: frame.cols,
                        alt: frame.alt,
                    });
                    #[cfg(test)]
                    bench::sleep_knob();
                    let (sample, rev) = sample_frame(fam, frame, grammars);
                    grammar_rev = rev;
                    sample
                }
            },
        },
    };
    match sample {
        Sample::Active { .. } | Sample::Idle | Sample::Decision(_) => stats.accepted += 1,
        Sample::Transient(Transient::LockContention) => stats.transient[0] += 1,
        Sample::Transient(Transient::OddEndpoint) => stats.transient[1] += 1,
        Sample::Transient(Transient::Incomplete) => stats.transient[2] += 1,
        Sample::Semantic(_) => stats.semantic += 1,
        Sample::Invalidated(_) => stats.invalidated += 1,
    }

    let mut records = store.records.lock().unwrap_or_else(|e| e.into_inner());
    let rec = records.entry(s.id).or_insert_with(|| {
        let k = EpochKey { identity_rev: 0, family: None, meta: None, session: None };
        Record {
            epoch: epoch_id(nonce, s.id, &k),
            key: k,
            view: None,
            seq: 0,
            reducer: Reducer::new(now),
            family: None,
            grammar_rev: None,
            observed_at: now,
            identity_rev: 0,
            // start from the surface's current view so a seat that never yields a frame is judged on
            // its identity, while any later bump without an accepted frame still mismatches.
            view_rev: s.activity_view_rev.load(Ordering::Acquire),
        }
    });
    // a new epoch (identity revision / meta pair / session id) starts a fresh reducer.
    if let Ok(k) = &key {
        if rec.key != *k {
            rec.key = k.clone();
            rec.epoch = epoch_id(nonce, s.id, k);
            rec.seq = 0;
            rec.view = None;
            rec.reducer = Reducer::new(now);
        }
        rec.family = k.family;
    }
    let mut sample = sample;
    // view continuity: a changed view revision / reset count / geometry / screen mode invalidates.
    if let Some(v) = view_now {
        if rec.view.is_some_and(|old| old != v) {
            sample = Sample::Invalidated(Invalid::View);
        }
        rec.view = Some(v);
    }
    // publication re-check: an identity write between collection and now voids the observation.
    let pend = s.activity_pending.load(Ordering::SeqCst);
    let ident = s.activity_identity_rev.load(Ordering::SeqCst);
    if matches!(sample, Sample::Active { .. } | Sample::Idle | Sample::Decision(_))
        && (pend != 0 || key.as_ref().map_or(true, |k| k.identity_rev != ident))
    {
        sample = Sample::Invalidated(Invalid::Identity);
    }
    rec.reducer.step(sample, now, &Params::LOCKED);
    rec.seq += 1;
    rec.observed_at = now;
    rec.grammar_rev = grammar_rev.or(rec.grammar_rev);
    // the serializer compares these with the surface's CURRENT revisions, so they only ever move with
    // an epoch (identity) or an accepted frame (view) — a rejected sample must not launder a change.
    rec.identity_rev = rec.key.identity_rev;
    if let Some(v) = view_now {
        rec.view_rev = v.view_rev;
    }
}

/// Spawns the observer on its own OS thread. Failure leaves every seat Unknown; there is NO fallback
/// to a shared/blocking pool (R3-1).
pub(crate) fn spawn_observer(daemon: Arc<Daemon>) {
    if std::env::var("CYS_DISPLAY_ACTIVITY").as_deref() == Ok("0") {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("cysd-display-activity".into())
        .spawn(move || observer_loop(&daemon));
    if let Err(e) = spawned {
        eprintln!("cysd: display activity observer did not start ({e}) — tab dots stay Unknown");
    }
}

fn observer_loop(daemon: &Arc<Daemon>) {
    daemon.activity.running.store(true, Ordering::SeqCst);
    let mut frame = Frame::new();
    let mut next = Instant::now();
    loop {
        let tick_start = Instant::now();
        observe_tick(daemon, &mut frame);
        let spent = tick_start.elapsed().as_micros().min(u128::from(u64::MAX)) as u64;
        // missed ticks are skipped, never queued for catch-up.
        next += SAMPLE_PERIOD;
        let now = Instant::now();
        let mut skipped = 0u64;
        while next <= now {
            next += SAMPLE_PERIOD;
            skipped += 1;
        }
        if let Ok(mut st) = daemon.activity.stats.lock() {
            st.ticks += 1;
            st.ticks_skipped += skipped;
            st.tick_us_log2[log2_bucket(spent)] += 1;
            st.tick_us_max = st.tick_us_max.max(spent);
        }
        std::thread::sleep(next.saturating_duration_since(Instant::now()));
    }
}

/// One observer tick over a short roster snapshot (the roster lock is released before any seat work).
pub(crate) fn observe_tick(daemon: &Daemon, frame: &mut Frame) {
    #[cfg(test)]
    let bench_tick = bench::tick_begin();
    let seats: Vec<Arc<Surface>> = {
        let g = daemon.surfaces.lock().unwrap_or_else(|e| e.into_inner());
        g.values().cloned().collect()
    };
    let now = daemon.started_instant.elapsed().as_secs_f64();
    let now_epoch = crate::state::now_epoch();
    let mut stats = Stats::default();
    for s in &seats {
        #[cfg(test)]
        let bench_seat = bench::seat_begin();
        observe_seat(&daemon.activity, s, frame, grammars(), now, now_epoch, &mut stats);
        #[cfg(test)]
        bench::seat_end(s.id, bench_seat);
    }
    // bounded cleanup over seat churn: drop records of seats that left the roster.
    {
        let mut records = daemon.activity.records.lock().unwrap_or_else(|e| e.into_inner());
        records.retain(|id, _| seats.iter().any(|s| s.id == *id));
    }
    if let Ok(mut st) = daemon.activity.stats.lock() {
        st.attempts += stats.attempts;
        st.accepted += stats.accepted;
        for i in 0..3 {
            st.transient[i] += stats.transient[i];
        }
        st.semantic += stats.semantic;
        st.invalidated += stats.invalidated;
        for i in 0..24 {
            st.hold_us_log2[i] += stats.hold_us_log2[i];
        }
        st.hold_us_max = st.hold_us_max.max(stats.hold_us_max);
    }
    #[cfg(test)]
    bench::tick_end(bench_tick, seats.len());
}

fn unknown_wire(reason: &str) -> Value {
    json!({"state": "unknown", "raw": "unknown", "reason": reason, "retained": false})
}

/// `surface.list` per-surface `display_activity`. Reads the cached record only; re-validates identity
/// and view revisions around the snapshot and enforces the lease at `now` (R3-1 / R3-4).
pub(crate) fn wire(daemon: &Daemon, s: &Surface) -> Value {
    let store = &daemon.activity;
    if store.nonce.is_none() {
        return unknown_wire("observer unavailable");
    }
    if s.exited.load(Ordering::Relaxed) || s.agent_exit_notified.load(Ordering::Relaxed) {
        return unknown_wire(Invalid::Exited.as_str());
    }
    let now = daemon.started_instant.elapsed().as_secs_f64();
    wire_at(store, s, now, &Params::LOCKED)
}

pub(crate) fn wire_at(store: &ActivityStore, s: &Surface, now: f64, p: &Params) -> Value {
    let p0 = s.activity_pending.load(Ordering::SeqCst);
    let r0 = s.activity_identity_rev.load(Ordering::SeqCst);
    let rec = store.records.lock().unwrap_or_else(|e| e.into_inner()).get(&s.id).cloned();
    let p1 = s.activity_pending.load(Ordering::SeqCst);
    let r1 = s.activity_identity_rev.load(Ordering::SeqCst);
    let Some(rec) = rec else {
        return unknown_wire("no observation");
    };
    if p0 != 0 || p1 != 0 || r0 != r1 || rec.identity_rev != r0 || r0 == u64::MAX {
        return unknown_wire(Invalid::Identity.as_str());
    }
    if rec.view_rev != s.activity_view_rev.load(Ordering::Acquire) {
        return unknown_wire(Invalid::View.as_str());
    }
    let (state, reason) = rec.reducer.effective_at(now, p);
    let retained = state == Effective::Working && rec.reducer.retained;
    let age = |t: Option<f64>| t.map(|t| ((now - t).max(0.0) * 1000.0).round() / 1000.0);
    json!({
        "state": state.as_str(),
        "raw": rec.reducer.raw.as_str(),
        "reason": reason,
        "family": rec.family.map(Family::as_str),
        "grammar_revision": rec.grammar_rev,
        "observed_age_secs": age(Some(rec.observed_at)),
        "last_positive_age_secs": age(rec.reducer.last_positive),
        "stable_age_secs": age(Some(rec.reducer.stable_since)),
        "observation_seq": rec.seq,
        "display_epoch": rec.epoch,
        "retained": retained,
        "consecutive_rejections": rec.reducer.rejections,
    })
}

/// org.status diagnostic exposure (additive; never drives the dot).
pub(crate) fn stats_wire(daemon: &Daemon) -> Value {
    let st = daemon.activity.stats.lock().map(|g| g.clone()).unwrap_or_default();
    json!({
        "enabled": daemon.activity.nonce.is_some(),
        "running": daemon.activity.running.load(Ordering::Relaxed),
        "ticks": st.ticks,
        "ticks_skipped": st.ticks_skipped,
        "attempts": st.attempts,
        "accepted": st.accepted,
        "transient_lock": st.transient[0],
        "transient_odd": st.transient[1],
        "transient_incomplete": st.transient[2],
        "semantic": st.semantic,
        "invalidated": st.invalidated,
        "hold_us_log2": st.hold_us_log2.to_vec(),
        "hold_us_max": st.hold_us_max,
        "tick_us_log2": st.tick_us_log2.to_vec(),
        "tick_us_max": st.tick_us_max,
    })
}

