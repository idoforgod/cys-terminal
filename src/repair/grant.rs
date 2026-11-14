//! The autonomous-repair grant (C9 · M5 · RULINGS B1, B4, F1, F7(1), G1–G3, G6, G7, K3, K5, K8, L2,
//! M3, O5, R8, S2, T5) — track T4.
//!
//! `repairs/grant.json` holds the owner's decision and its provenance (B1). The ONE global claim
//! sequence `repairs/grant-seq/<n>.json` (+ `<n>.closed`) serialises autonomous starts across every
//! lane daemon: concurrency 1, a daily cap of distinct repairs, and the pause (B4). A claim is created
//! COMPLETE (content flushed under a temporary name in the same directory, then placed by a
//! no-replace placement — never `std::fs::rename`), is immutable, and nothing of the sequence is ever
//! deleted. The record a claim binds to is written only through track T3's ledger calls.
//!
//! ★F1: nothing here sends anything to a queue, an alert route or the event bus — a `Busy`,
//! `CapReached`, `Paused` or `LostRace` is an answer to the caller only (S2: the master sees the
//! grant in the five-minute digest, never as a queue item). ★F4: no panic paths in production code.
//!
//! Stated limit (design §3-2 criterion 6, §4-13): every file here is writable by the same OS user;
//! a process of that user that writes `grant.json` or a claim directly is not stopped — the same
//! class as every other approval device of the product.

use super::ledger;
use super::lock;
use super::types::{LaneKey, LockOwner, RepairError, RepairId, RepairState, LEASE_SECS};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// `repairs/grant.json` — the owner's decision and its provenance only (B1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GrantDecision {
    pub schema: u32,
    pub state: GrantState,
    pub daily_cap: u32,
    pub decided_at: f64,
    pub decided_lane: LaneKey,
    pub decided_seat: u64,
    pub prompt_digest: String,
    pub prompt_excerpt: String,
    pub paused_at: Option<f64>,
    pub resumed_at: Option<f64>,
    /// RULINGS CV1 (T4b): the decision's GENERATION — one more than the decision it replaced (0 = a
    /// file written before T4b). A decision is published only by [`decide_in`], under the shared
    /// grant lock, and only if the generation it validated is still the current one.
    #[serde(default)]
    pub generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrantState {
    On,
    Off,
}

/// Default daily cap of autonomous starts (C9).
pub const DEFAULT_DAILY_CAP: u32 = 5;
/// The pause ends by itself after 30 minutes without owner input (B4).
pub const PAUSE_QUIET_SECS: u64 = 1_800;
/// RULINGS G7(a).
pub const EXPIRY_GRACE_SECS: u64 = 60;
/// RULINGS K8: an unreadable claim counts toward today's cap only while younger than this. K5's
/// "assume held" bound is `LEASE_SECS` + `EXPIRY_GRACE_SECS` (no own constant).
pub const UNREADABLE_CLAIM_CAP_SECS: u64 = 86_400;
/// Largest daily cap `grant on --daily-cap` accepts (interpretation — C9 says "user-adjustable").
pub const MAX_DAILY_CAP: u32 = 100;

/// `repairs/grant-seq/<n>.json` — one claim of the global sequence (F1 / G1). `day` = the local
/// date at placement (M3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GrantClaim {
    pub schema: u32,
    pub repair: RepairId,
    pub lane_key: LaneKey,
    pub day: String,
    pub claimed_at: f64,
}

/// Outcome of an autonomous start attempt (G1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartVerdict {
    Allowed { claim: u32 },
    Paused,
    Busy,
    CapReached,
    LostRace,
}

/// The sentence of a start refused because the grant is off (C9 9.3 / 9.5 — with the grant off
/// nobody starts, whatever the ledger holds).
pub const GRANT_OFF_KO: &str = "자동 수리 허락이 꺼져 있습니다 — 자동 착수는 하지 않습니다(결함은 장부에 남고, 주인의 \"고쳐라\"를 기다립니다)";

// ───────────────────────────── layout under an explicit root ─────────────────────────────
// The same relative paths as `paths::{grant_path, grant_seq_dir, grant_claim_path,
// grant_closed_path}` (pinned by `layout_matches_paths_rs`); tests pass their own root.

/// `<root>/grant.json`
pub fn grant_path_in(root: &Path) -> PathBuf {
    root.join("grant.json")
}

/// `<root>/grant-seq/`
pub fn seq_dir_in(root: &Path) -> PathBuf {
    root.join("grant-seq")
}

/// `<root>/grant-seq/<n>.json`
pub fn claim_path_in(root: &Path, n: u64) -> PathBuf {
    seq_dir_in(root).join(format!("{n}.json"))
}

/// `<root>/grant-seq/<n>.closed`
pub fn closed_path_in(root: &Path, n: u64) -> PathBuf {
    seq_dir_in(root).join(format!("{n}.closed"))
}

/// `<root>/grant-pause/<lane_key>.json` — the last owner input seen by that lane's daemon (B4; one
/// writer per file — see `note_owner_input_in`).
pub fn pause_path_in(root: &Path, lane: &LaneKey) -> PathBuf {
    root.join("grant-pause").join(format!("{}.json", lane.0))
}

fn io_err(p: &Path, e: impl std::fmt::Display) -> RepairError {
    RepairError::Io(format!("{}: {e}", p.display()))
}

fn corrupt(p: &Path, e: impl std::fmt::Display) -> RepairError {
    RepairError::Corrupt(format!("{}: {e}", p.display()))
}

/// Parses a ledger JSON value of this module: a missing or higher `schema` is unreadable (O-C1-7).
fn parse_versioned<T: serde::de::DeserializeOwned>(p: &Path, bytes: &[u8]) -> Result<T, RepairError> {
    let v: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| corrupt(p, e))?;
    let schema = v.get("schema").and_then(|s| s.as_u64()).unwrap_or(u64::MAX);
    if schema > u64::from(super::REPAIR_SCHEMA) {
        return Err(corrupt(p, format!("unknown schema {schema}")));
    }
    serde_json::from_value(v).map_err(|e| corrupt(p, e))
}

// ───────────────────────────── the decision file (B1) ─────────────────────────────

/// Reads `grant.json`: `Ok(None)` if absent; `Err(Corrupt)` if unreadable or of a higher schema.
pub fn read_decision_in(root: &Path) -> Result<Option<GrantDecision>, RepairError> {
    let p = grant_path_in(root);
    match std::fs::read(&p) {
        Ok(b) => parse_versioned(&p, &b).map(Some),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(corrupt(&p, e)),
    }
}

/// Writes `grant.json` by atomic replace (tmp + rename) and flushes it (E1); the latest decision
/// wins (B1). Written only by the daemon of the lane where the owner answered.
pub fn write_decision_in(root: &Path, d: &GrantDecision) -> Result<(), RepairError> {
    let p = grant_path_in(root);
    let mut d = d.clone();
    d.schema = super::REPAIR_SCHEMA;
    crate::atomic_write_json(&p, &d).map_err(|e| io_err(&p, e))?;
    if let Ok(f) = std::fs::File::open(&p) {
        let _ = ledger::full_flush_file(&f);
    }
    let _ = ledger::full_flush_dir(root);
    Ok(())
}

/// Whether a decision lets autonomous repairs start at all (absent / unreadable / off = no).
pub fn is_on(d: Option<&GrantDecision>) -> bool {
    d.is_some_and(|d| d.state == GrantState::On)
}

// ───────────────────────────── one order for decisions (RULINGS CV1 · T4b) ─────────────────────────────

/// `<root>/grant.lock` — the lock every lane daemon takes to decide (validate the answer, consume
/// the question, publish `grant.json`). An OS advisory lock on an open file (unix `flock`, Windows
/// `LockFileEx` on byte 0): the kernel releases it when the holder's process ends, so a dead holder
/// never leaves it held. The file itself is empty and never removed.
pub fn grant_lock_path_in(root: &Path) -> PathBuf {
    root.join("grant.lock")
}

/// How long a decision waits for the grant lock before it is refused (milliseconds). Only the
/// owner-decision verbs (`grant on` / `grant off`) take the lock — never a start, a boot step, a
/// sweep or a read.
pub const GRANT_LOCK_WAIT_MS: u64 = 2_000;

/// The refusal of a decision that could not get the lock in time.
pub const GRANT_LOCK_BUSY_KO: &str = "다른 좌석이 지금 자동 수리 허락을 결정하는 중입니다 — 잠시 뒤 다시 하세요(아무것도 바꾸지 않았습니다)";

/// The refusal of a decision whose validated generation is no longer the current one.
pub const GRANT_STALE_KO: &str = "이 결정을 확인하는 사이에 다른 결정이 먼저 났습니다 — 아무것도 바꾸지 않았습니다(다시 물으려면 `cys repair grant ask`)";

/// The held grant lock (released when dropped).
pub struct GrantLock {
    file: std::fs::File,
}

impl Drop for GrantLock {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            // SAFETY: the descriptor is owned by `self.file` for the duration of the call.
            unsafe {
                libc::flock(self.file.as_raw_fd(), libc::LOCK_UN);
            }
        }
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::Storage::FileSystem::UnlockFile;
            // SAFETY: the handle is owned by `self.file`; byte 0, length 1, as locked.
            unsafe {
                UnlockFile(self.file.as_raw_handle() as _, 0, 0, 1, 0);
            }
        }
    }
}

/// One non-blocking try of the lock: `Ok(true)` taken, `Ok(false)` another holder, `Err` cannot tell.
fn try_lock_file(file: &std::fs::File) -> std::io::Result<bool> {
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        // SAFETY: the descriptor is owned by `file` for the duration of the call.
        let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if rc == 0 {
            return Ok(true);
        }
        let e = std::io::Error::last_os_error();
        match e.raw_os_error() {
            Some(c) if c == libc::EWOULDBLOCK || c == libc::EAGAIN => Ok(false),
            _ => Err(e),
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            LockFileEx, LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY,
        };
        // SAFETY: a zeroed OVERLAPPED is the documented "offset 0" value; the handle is owned by
        // `file` for the duration of the call.
        let mut ov: windows_sys::Win32::System::IO::OVERLAPPED = unsafe { std::mem::zeroed() };
        let ok = unsafe {
            LockFileEx(
                file.as_raw_handle() as _,
                LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
                0,
                1,
                0,
                &mut ov,
            )
        };
        if ok != 0 {
            return Ok(true);
        }
        let e = std::io::Error::last_os_error();
        match e.raw_os_error() {
            Some(33) => Ok(false), // ERROR_LOCK_VIOLATION
            _ => Err(e),
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = file;
        Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "no file lock"))
    }
}

/// Takes the grant lock, waiting at most `wait` (polled every 20 ms). The lock file is opened
/// without following a link and without blocking (unix `O_NOFOLLOW | O_NONBLOCK`) and must be a
/// regular file. Not taken in time → `Err(Precondition(GRANT_LOCK_BUSY_KO))`.
pub fn lock_grant_in(root: &Path, wait: std::time::Duration) -> Result<GrantLock, RepairError> {
    let p = grant_lock_path_in(root);
    std::fs::create_dir_all(root).map_err(|e| io_err(root, e))?;
    let mut o = std::fs::OpenOptions::new();
    o.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = o.open(&p).map_err(|e| io_err(&p, e))?;
    match file.metadata() {
        Ok(m) if m.is_file() => {}
        Ok(_) => return Err(io_err(&p, "the grant lock is not a regular file")),
        Err(e) => return Err(io_err(&p, e)),
    }
    let deadline = std::time::Instant::now() + wait;
    loop {
        match try_lock_file(&file) {
            Ok(true) => return Ok(GrantLock { file }),
            Ok(false) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Ok(false) => return Err(RepairError::Precondition(GRANT_LOCK_BUSY_KO.to_string())),
            Err(e) => return Err(io_err(&p, e)),
        }
    }
}

/// The decision a [`decide_in`] step validated against.
#[derive(Debug, Clone, Copy)]
pub struct CurrentDecision<'a> {
    /// The decision read under the lock (`None` = no decision yet, or unreadable — see `unreadable`).
    pub decision: Option<&'a GrantDecision>,
    /// `grant.json` exists but cannot be read.
    pub unreadable: bool,
    /// The current generation (0 when there is no readable decision).
    pub generation: u64,
}

/// Points of a decision where a test can suspend it (production: none).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecideStep {
    /// The lock is held; nothing has been read yet.
    Locked,
    /// The decision is validated (the caller's step ran and returned it); publication is next.
    BeforePublish,
}

/// RULINGS CV1 — ONE step for a decision: under the grant lock, read the current decision, let
/// `decide` validate the owner's answer against it (and consume the question — the caller's memory),
/// then publish the result with generation `validated + 1` — refused if the current generation is no
/// longer the one validated (`GRANT_STALE_KO`). A decision that validated an older generation can
/// therefore never overwrite a later one ("an old 예 never overwrites a later off"). `hook` = test
/// suspension points. On any `Err` nothing was published.
pub fn decide_in<F>(
    root: &Path,
    wait: std::time::Duration,
    hook: &dyn Fn(DecideStep),
    decide: F,
) -> Result<GrantDecision, RepairError>
where
    F: FnOnce(CurrentDecision<'_>) -> Result<GrantDecision, RepairError>,
{
    let _lock = lock_grant_in(root, wait)?;
    hook(DecideStep::Locked);
    let read = read_decision_in(root);
    let (current, unreadable) = match &read {
        Ok(d) => (d.as_ref(), false),
        Err(_) => (None, true),
    };
    let generation = current.map(|d| d.generation).unwrap_or(0);
    let mut d = decide(CurrentDecision {
        decision: current,
        unreadable,
        generation,
    })?;
    hook(DecideStep::BeforePublish);
    // defence in depth: a writer that does not take the lock (an older build) may have written
    let now_gen = read_decision_in(root)
        .ok()
        .flatten()
        .map(|x| x.generation)
        .unwrap_or(0);
    if now_gen != generation {
        return Err(RepairError::Precondition(GRANT_STALE_KO.to_string()));
    }
    d.generation = generation.saturating_add(1);
    write_decision_in(root, &d)?;
    Ok(d)
}

/// The current decision's generation (0 when absent or unreadable) — what `grant ask` records in
/// its question (RULINGS CV1: an answer is good only while no decision was published after it).
pub fn current_generation_in(root: &Path) -> u64 {
    read_decision_in(root)
        .ok()
        .flatten()
        .map(|d| d.generation)
        .unwrap_or(0)
}

// ───────────────────────────── the pause (B4) ─────────────────────────────

/// One lane's pause file: the last owner input that lane's daemon saw, and (RULINGS DM1) the time
/// the mark covers through — every reader treats the pause as lasting until `covered_through + 30
/// min`. A mark written before DM1 has no `covered_through` (read as `paused_at`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PauseMark {
    pub schema: u32,
    pub lane_key: LaneKey,
    pub paused_at: f64,
    pub resumed_at: f64,
    #[serde(default)]
    pub covered_through: Option<f64>,
}

impl PauseMark {
    /// The time the mark covers owner input through (DM1).
    pub fn covered(&self) -> f64 {
        self.covered_through
            .filter(|c| c.is_finite())
            .map_or(self.paused_at, |c| c.max(self.paused_at))
    }
}

/// THE PAUSE (RULINGS DK1 (1)(2)(5)(6) with DM1–DM3 in place of DK1 (3)(4) — T4b):
/// (1) per lane this daemon holds ONE in-memory value, the latest owner-input time (forward only);
/// every owner input updates it first; (2) this daemon's answer to "paused?" = now < 30 minutes after
/// the LATER of that value and every written mark it can read (a mark counts until its
/// `covered_through` + 30 min); a pause directory that cannot be read (anything but "not found")
/// answers "paused" for THAT call — nothing is latched; DM1: a write, under the lane's writer lock,
/// stores `covered_through` = the CURRENT latest input + [`PAUSE_COVER_SECS`] (on the input path the
/// write runs at the input, so this is the write's wall time + W; a retried write covers the last
/// input + W, so the pause still ends 30 minutes + W after the last input — DM4), kept as the later
/// of the stored and the new value; an input at or before
/// the lane's covered-through time needs NO write; "not yet visible" is DERIVED (the in-memory input
/// is later than the covered-through time this daemon wrote) and whoever asks while it is true (an
/// owner input, a start admission, a new-round check) attempts the write, then answers by (2);
/// DM2: only after a FAILED attempt is the next one spaced — by a monotonic clock read under the
/// writer lock, [`PAUSE_RETRY_AFTER_FAILURE_SECS`] — never by a caller's captured input time;
/// DM3: "written", not "flushed" — the mark is published by an atomic replace so other daemons see
/// it, and deliberately NOT forced to stable storage (the write is on the path of the owner's
/// typing; a machine crash that loses a mark restarts every daemon, and the hold after a daemon
/// start covers each lane for the pause length). Each lane writes ONLY its own mark; written only
/// while the grant is on. No flag is set or cleared by a completion; no timer, no thread. Sends
/// nothing to any queue.
#[derive(Debug, Default, Clone, Copy)]
struct PauseMemory {
    /// (1) the latest owner-input time seen by this daemon for the lane (0 = none)
    latest: f64,
    /// DM1: the covered-through time this daemon last wrote for the lane (0 = none)
    visible_through: f64,
    /// DM2: after a failed attempt, no new attempt before this monotonic instant
    retry_after: Option<std::time::Instant>,
}

type PauseKey = (PathBuf, String);

fn pause_memory() -> &'static std::sync::Mutex<std::collections::HashMap<PauseKey, PauseMemory>> {
    static M: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<PauseKey, PauseMemory>>> =
        std::sync::OnceLock::new();
    M.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

/// DM1: W — how far past the wall time of a write a mark covers owner input.
pub const PAUSE_COVER_SECS: f64 = 5.0;

/// DM2: after a failed attempt, the next attempt waits at least this long (monotonic).
pub const PAUSE_RETRY_AFTER_FAILURE_SECS: f64 = 1.0;

#[cfg(test)]
thread_local! {
    /// Test seam (DM4): pause-mark writes on this thread fail (the directory stays readable).
    pub(crate) static FAIL_PAUSE_WRITE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Test seam (DM2): added to the monotonic clock on this thread.
    pub(crate) static MONOTONIC_SKEW: std::cell::Cell<std::time::Duration> =
        const { std::cell::Cell::new(std::time::Duration::ZERO) };
}

fn monotonic_now() -> std::time::Instant {
    let now = std::time::Instant::now();
    #[cfg(test)]
    {
        return now + MONOTONIC_SKEW.with(|c| c.get());
    }
    #[allow(unreachable_code)]
    now
}

fn write_pause_mark(p: &Path, mark: &PauseMark) -> std::io::Result<()> {
    #[cfg(test)]
    {
        if FAIL_PAUSE_WRITE.with(|c| c.get()) {
            return Err(std::io::Error::other("injected pause-write failure"));
        }
    }
    crate::atomic_write_json(p, mark)
}

/// The per-lane writer lock of this process (each lane's mark has one writer: that lane's daemon;
/// its threads take this lock for the read-compare-write of the mark).
fn pause_write_lock(root: &Path, lane: &LaneKey) -> std::sync::Arc<std::sync::Mutex<()>> {
    static W: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<PauseKey, std::sync::Arc<std::sync::Mutex<()>>>>,
    > = std::sync::OnceLock::new();
    W.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry((root.to_path_buf(), lane.0.clone()))
        .or_default()
        .clone()
}

/// DM1 / DM2 — if the lane's latest input is not yet visible (later than the covered-through time
/// this daemon wrote) and no failed attempt is less than a second old (monotonic): under the lane's
/// writer lock, write `covered_through` = max(stored, the CURRENT latest input + W). `Ok(true)` =
/// written; `Ok(false)` = nothing to do (covered already, a failed attempt in the last second, or
/// the grant is off — then nothing is attempted); `Err` = the write failed (the next attempt waits a
/// second; nothing else is latched).
fn persist_pause(root: &Path, lane: &LaneKey) -> Result<bool, RepairError> {
    let key = (root.to_path_buf(), lane.0.clone());
    let write_lock = pause_write_lock(root, lane);
    let _held = write_lock.lock().unwrap_or_else(|e| e.into_inner());
    let mono = monotonic_now();
    let latest = {
        let m = pause_memory().lock().unwrap_or_else(|e| e.into_inner());
        let e = m.get(&key).copied().unwrap_or_default();
        if e.latest <= e.visible_through || e.retry_after.is_some_and(|r| mono < r) {
            return Ok(false);
        }
        e.latest // the CURRENT value, read under the writer lock
    };
    let on = match read_decision_in(root) {
        Ok(d) => is_on(d.as_ref()),
        // an unreadable decision is off for starting — but an owner input must still pause in case
        // a readable "on" comes back: write the mark (fail closed for autonomy)
        Err(_) => true,
    };
    if !on {
        return Ok(false);
    }
    let p = pause_path_in(root, lane);
    let stored = std::fs::read(&p)
        .map_err(|e| corrupt(&p, e))
        .and_then(|b| parse_versioned::<PauseMark>(&p, &b))
        .ok();
    let paused_at = stored.as_ref().map_or(latest, |m| m.paused_at.max(latest));
    let covered = (latest + PAUSE_COVER_SECS).max(stored.as_ref().map_or(0.0, PauseMark::covered));
    let mark = PauseMark {
        schema: super::REPAIR_SCHEMA,
        lane_key: lane.clone(),
        paused_at,
        resumed_at: covered + PAUSE_QUIET_SECS as f64,
        covered_through: Some(covered),
    };
    let written = write_pause_mark(&p, &mark);
    let mut m = pause_memory().lock().unwrap_or_else(|e| e.into_inner());
    let e = m.entry(key).or_default();
    match written {
        Ok(()) => {
            e.visible_through = e.visible_through.max(covered);
            e.retry_after = None;
            Ok(true)
        }
        Err(err) => {
            e.retry_after = Some(mono + std::time::Duration::from_secs_f64(PAUSE_RETRY_AFTER_FAILURE_SECS));
            Err(io_err(&p, err))
        }
    }
}

/// B4 "주인의 아무 입력이면 즉시 정지" — an owner input seen by `lane`'s daemon at `now`: (1) the
/// in-memory latest input moves forward at once; DM1: then the write is attempted if the input is
/// not yet visible. `Ok(true)` = a mark was written by this call; `Err` = the write failed — this
/// lane's daemon stays paused for 30 minutes after its latest input from its memory alone.
pub fn note_owner_input_in(root: &Path, lane: &LaneKey, now: f64) -> Result<bool, RepairError> {
    {
        let mut m = pause_memory().lock().unwrap_or_else(|e| e.into_inner());
        let e = m.entry((root.to_path_buf(), lane.0.clone())).or_default();
        if now > e.latest {
            e.latest = now;
        }
    }
    persist_pause(root, lane)
}

/// DM1 — before answering "paused?" for a start admission or a new-round check: every lane of
/// `root` whose latest input is not yet visible in this daemon attempts its write. A failure is one
/// stderr line scoped to this lane's daemon (DK1 (6)).
pub fn refresh_pause_in(root: &Path) {
    let pending: Vec<String> = pause_memory()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .filter(|((r, _), e)| r == root && e.latest > e.visible_through)
        .map(|((_, lane), _)| lane.clone())
        .collect();
    for lane in pending {
        if let Err(e) = persist_pause(root, &LaneKey(lane)) {
            eprintln!("{} ({e:?})", PAUSE_WRITE_FAILED_KO);
        }
    }
}

/// Test helper: forget this process's memory of a lane — the view of ANOTHER daemon, which has
/// only the written marks (the "separate process" of the DM tests).
#[cfg(test)]
pub(crate) fn forget_pause_memory(root: &Path, lane: &LaneKey) {
    pause_memory()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&(root.to_path_buf(), lane.0.clone()));
}

/// DK1 (6): the sentence of a failed pause write — this lane's daemon only.
pub const PAUSE_WRITE_FAILED_KO: &str = "[cysd] repair grant: 이 부서의 데몬은 주인의 입력을 파일에 남기지 못했습니다 — 이 부서의 데몬은 마지막 입력 뒤 적어도 30분 동안 자동 착수를 멈춥니다(다른 부서의 데몬은 이 입력을 모릅니다)";

/// When the pause ends: the later of this daemon's in-memory latest input + 30 min and every
/// readable written mark's `covered_through` + 30 min (DM1; `grant.json`'s own pause counts from its
/// `paused_at`; an unreadable mark counts with its modification time, or "now" — fail closed).
/// `None` = no owner input known.
pub fn pause_deadline_in(root: &Path, now: f64) -> Option<f64> {
    pause_read_in(root, now).0
}

#[cfg(test)]
thread_local! {
    /// Test seam (DK1 pin): the next reads of the pause directory on this thread fail as an
    /// unreadable directory would — deterministic, independent of the user running the tests.
    pub(crate) static FAIL_PAUSE_DIR_READ: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn read_pause_dir(dir: &Path) -> std::io::Result<std::fs::ReadDir> {
    #[cfg(test)]
    {
        if FAIL_PAUSE_DIR_READ.with(|c| c.get()) {
            return Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
        }
    }
    std::fs::read_dir(dir)
}

/// [`pause_deadline_in`] and whether the pause directory could not be read in full (DK1 (2): a
/// failure to list it other than "not found", or an unreadable entry → "paused" for this call).
fn pause_read_in(root: &Path, now: f64) -> (Option<f64>, bool) {
    let mut unreadable = false;
    let quiet = PAUSE_QUIET_SECS as f64;
    let mut end: Option<f64> = read_decision_in(root)
        .ok()
        .flatten()
        .and_then(|d| d.paused_at.map(|p| p + quiet));
    let mut bump = |t: f64| end = Some(end.map_or(t, |l: f64| l.max(t)));
    let dir = root.join("grant-pause");
    match read_pause_dir(&dir) {
        Ok(rd) => {
            for e in rd {
                let Ok(e) = e else {
                    unreadable = true;
                    continue;
                };
                let p = e.path();
                let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
                    continue;
                };
                if name.starts_with('.') || !name.ends_with(".json") {
                    continue; // a temporary file of a writer
                }
                match std::fs::read(&p)
                    .map_err(|e| corrupt(&p, e))
                    .and_then(|b| parse_versioned::<PauseMark>(&p, &b))
                {
                    Ok(m) => bump(m.covered() + quiet),
                    Err(_) => bump(mtime_epoch(&p).unwrap_or(now) + quiet),
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => unreadable = true,
    }
    for ((r, _), m) in pause_memory()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
    {
        if r == root && m.latest > 0.0 {
            bump(m.latest + quiet);
        }
    }
    (end, unreadable)
}

/// The pause as one read (RULINGS DG2 / DK1 (2)): when it ends and whether the pause directory
/// could not be read in this read. Two reads that compare equal saw no change in between.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PauseState {
    pub deadline: Option<f64>,
    pub unreadable: bool,
}

impl PauseState {
    /// Paused at `now`: the directory could not be read in this read, or the pause has not ended.
    pub fn paused_at(&self, now: f64) -> bool {
        self.unreadable || self.deadline.is_some_and(|d| now < d)
    }
}

/// [`PauseState`] of `root` read now (no write — a READ verb may call it).
pub fn pause_state_in(root: &Path, now: f64) -> PauseState {
    let (deadline, unreadable) = pause_read_in(root, now);
    PauseState {
        deadline,
        unreadable,
    }
}

/// RULINGS DG3 (c) — the start time of the daemon an attempt runs in, from its incarnation
/// (`"{started_at:x}-{pid:x}"` — `seat_token_generation` in the daemon's `state.rs`; the lock owner
/// of every attempt carries it). `None` = not that shape (then no hold).
pub fn incarnation_started_at(incarnation: &str) -> Option<f64> {
    let (hex, pid) = incarnation.split_once('-')?;
    if hex.is_empty()
        || pid.is_empty()
        || !hex.bytes().all(|b| b.is_ascii_hexdigit())
        || !pid.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return None;
    }
    u64::from_str_radix(hex, 16).ok().map(|s| s as f64)
}

/// RULINGS DG3 (c) — after a daemon start its lane holds AUTONOMOUS starts for the pause length:
/// its memory of recent owner input (and of a failed pause write) is gone, so it waits as if an
/// owner input had come at its start. The owner's own "고쳐라" (`owner_order`) never comes here.
pub fn restart_hold_until(owner: &LockOwner) -> Option<f64> {
    incarnation_started_at(&owner.incarnation).map(|s| s + PAUSE_QUIET_SECS as f64)
}

/// B4 / DK1 (2): paused while the pause has not ended (memory and written marks, the later wins),
/// and for a call that cannot read the pause directory. A clock set back keeps the pause on (the
/// safe side). Writes nothing.
pub fn is_paused_in(root: &Path, now: f64) -> bool {
    pause_state_in(root, now).paused_at(now)
}

/// The modification time of `p` in epoch seconds (`None` = cannot be read).
fn mtime_epoch(p: &Path) -> Option<f64> {
    std::fs::symlink_metadata(p)
        .ok()?
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs_f64())
}

// ───────────────────────────── the claim sequence (F1 / G1–G3 …) ─────────────────────────────

/// Points of the start protocol where a test can suspend or "crash" an attempt (production: none).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimStep {
    /// Step 1 done — the snapshot's high water mark is known.
    Snapshot,
    /// Step 2, closer: right before the fresh read of claim k's record(s).
    BeforeFreshRead(u32),
    /// Step 2, closer: between the fresh read (closable) and creating `<k>.closed`.
    BeforeMarker(u32),
    /// Step 2, closer: between creating `<k>.closed` and the second read.
    BeforeSecondRead(u32),
    /// Step 2 done (about to answer Busy / Paused / CapReached, or to lock).
    AfterEvaluate,
    /// Step 3 done (the record carries the attempt's lock).
    AfterLock,
    /// Step 4: the date was read again; the placement is next.
    BeforePlace,
    /// Step 4 done (the claim is placed).
    AfterPlace,
    /// Step 5 (RULINGS DG2): between the first and the second read of the admission state.
    BetweenAdmissionReads,
}

/// What a test hook tells the attempt to do at a [`ClaimStep`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookAction {
    Continue,
    /// The process dies here: the attempt returns at once, cleaning nothing up.
    Crash,
}

/// The environment of one start attempt (production: [`production_env`] via
/// [`try_claim_autonomous_start`]).
pub struct ClaimEnv<'a> {
    /// The repairs root (`paths::repairs_root()` in production).
    pub root: &'a Path,
    /// The wall clock, read afresh at every use.
    pub clock: &'a dyn Fn() -> f64,
    /// The installation's local calendar date `YYYY-MM-DD`, read afresh at every use.
    pub today: &'a dyn Fn() -> String,
    /// Suspension / crash points (tests).
    pub hook: &'a dyn Fn(ClaimStep) -> HookAction,
}

/// The local calendar date of the installation (O-C9-4 "per day").
pub fn local_today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

fn no_hook(_: ClaimStep) -> HookAction {
    HookAction::Continue
}

fn crashed(at: ClaimStep) -> RepairError {
    RepairError::Io(format!("simulated crash at {at:?}"))
}

macro_rules! step {
    ($env:expr, $at:expr) => {
        if ($env.hook)($at) == HookAction::Crash {
            return Err(crashed($at));
        }
    };
}

/// A claim file as read: parsed, or unreadable (garbage, partial, higher schema — K5).
#[derive(Debug, Clone)]
enum ClaimRead {
    Ok(GrantClaim),
    Unreadable,
}

/// The claims of one snapshot: (number, read result, closed?).
type Snapshot = Vec<(u32, ClaimRead, bool)>;

/// Process-wide cache of parsed claims (claims are immutable once placed — F1); unreadable ones are
/// never cached (reads retried at each evaluation — K5).
fn claim_cache() -> &'static std::sync::Mutex<std::collections::HashMap<PathBuf, GrantClaim>> {
    static C: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<PathBuf, GrantClaim>>> =
        std::sync::OnceLock::new();
    C.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

fn read_claim(p: &Path) -> ClaimRead {
    if let Some(c) = claim_cache()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(p)
    {
        return ClaimRead::Ok(c.clone());
    }
    let parsed = std::fs::read(p)
        .map_err(|e| corrupt(p, e))
        .and_then(|b| parse_versioned::<GrantClaim>(p, &b));
    match parsed {
        Ok(c) => {
            claim_cache()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(p.to_path_buf(), c.clone());
            ClaimRead::Ok(c)
        }
        Err(_) => ClaimRead::Unreadable,
    }
}

/// The numbers of every placed claim `<n>.json` (temporary files never counted — F7(1)).
fn claim_numbers(root: &Path) -> Vec<u32> {
    let Ok(rd) = std::fs::read_dir(seq_dir_in(root)) else {
        return Vec::new();
    };
    let mut out: Vec<u32> = rd
        .flatten()
        .filter_map(|e| {
            let name = e.file_name();
            let name = name.to_str()?;
            let n = name.strip_suffix(".json")?;
            if n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            n.parse::<u32>().ok()
        })
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// G1 step 1: list the claim directory ONCE.
fn snapshot(root: &Path) -> (u32, Snapshot) {
    let nums = claim_numbers(root);
    let h = nums.last().copied().unwrap_or(0);
    let snap = nums
        .into_iter()
        .map(|n| {
            let closed = std::fs::symlink_metadata(closed_path_in(root, u64::from(n))).is_ok();
            (n, read_claim(&claim_path_in(root, u64::from(n))), closed)
        })
        .collect();
    (h, snap)
}

/// The age of a file in seconds at `now`: a missing file is OLD (it cannot be renewed —
/// interpretation 33), one whose modification time cannot be read is YOUNG (held).
fn age_of(p: &Path, now: f64) -> f64 {
    match std::fs::symlink_metadata(p) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => f64::INFINITY,
        Err(_) => 0.0,
        Ok(m) => match m
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        {
            Some(d) => (now - d.as_secs_f64()).max(0.0),
            None => 0.0,
        },
    }
}

/// K5 / L2 bound: held until every file of the set is older than this.
fn hold_secs() -> f64 {
    (LEASE_SECS + EXPIRY_GRACE_SECS) as f64
}

/// Whether claim `k` is HELD — bound to a live lock (G2 negated, with K5 / L2 / O5 for unreadable
/// parts) — judged on reads stamped with the clock taken BEFORE them (G7(a): an earlier stamp can
/// only make a lock look younger).
fn claim_held(env: &ClaimEnv<'_>, owner: &LockOwner, k: u32, claim: &ClaimRead) -> bool {
    let root = env.root;
    let claim_file = claim_path_in(root, u64::from(k));
    match claim {
        ClaimRead::Ok(c) => {
            let now = (env.clock)();
            match ledger::read_record_in(root, &c.repair) {
                Ok(Some(rec)) => {
                    let bound = rec.lock.as_ref().is_some_and(|l| {
                        l.claim == Some(k)
                            && lock::live_for_others(l, now, &owner.lane, &owner.incarnation)
                    });
                    bound && !rec.state.closes_for_grant()
                }
                // K5 / interpretation 33: a missing or unreadable record counts as carrying a live
                // lock bound to this claim until BOTH files are older than lease + 60 s (L2: age on
                // the newer of the two; a missing record is old).
                Ok(None) | Err(_) => {
                    let rec_file = ledger::record_path_in(root, &c.repair);
                    let youngest = age_of(&claim_file, now).min(age_of(&rec_file, now));
                    youngest <= hold_secs()
                }
            }
        }
        // K5 / O5: an unreadable claim is bound to a live lock; its record set is every record whose
        // lock names claim k (a scan) — held until the claim file and all of them are old.
        ClaimRead::Unreadable => {
            let now = (env.clock)();
            let mut youngest = age_of(&claim_file, now);
            for rid in ledger::list_ids_in(root) {
                if let Ok(Some(rec)) = ledger::read_record_in(root, &rid) {
                    if rec.lock.as_ref().is_some_and(|l| l.claim == Some(k)) {
                        youngest = youngest.min(age_of(&ledger::record_path_in(root, &rid), now));
                    }
                }
            }
            youngest <= hold_secs()
        }
    }
}

/// G7(b)'s second read: is claim k's lock now live and bound to k (same scan as the first read)?
fn bound_live_now(env: &ClaimEnv<'_>, owner: &LockOwner, k: u32, claim: &ClaimRead) -> bool {
    let now = (env.clock)();
    let live_bound = |rid: &RepairId| -> bool {
        matches!(ledger::read_record_in(env.root, rid), Ok(Some(rec)) if rec.lock.as_ref().is_some_and(|l| {
            l.claim == Some(k) && lock::live_for_others(l, now, &owner.lane, &owner.incarnation)
        }))
    };
    match claim {
        ClaimRead::Ok(c) => live_bound(&c.repair),
        ClaimRead::Unreadable => ledger::list_ids_in(env.root).iter().any(live_bound),
    }
}

/// Creates `<k>.closed` (create-new; an existing marker is fine — idempotent). The marker is empty.
fn create_marker(root: &Path, k: u32) -> Result<(), RepairError> {
    let p = closed_path_in(root, u64::from(k));
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&p)
    {
        Ok(f) => {
            let _ = ledger::full_flush_file(&f);
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(io_err(&p, e)),
    }
}

/// The outcome of step 2 for the attempt.
enum Evaluated {
    /// Every claim ≤ h is closed now; the cap counts are attached.
    Clear,
    /// A claim remains held (or a closer's second read saw it renewed) — G1 `Busy`.
    Busy,
}

/// G1 step 2 (closing part): closes every closable claim ≤ `h` (G2 + G7(b)).
fn evaluate(env: &ClaimEnv<'_>, owner: &LockOwner, snap: &Snapshot) -> Result<Evaluated, RepairError> {
    let mut busy = false;
    for (k, claim, closed) in snap {
        if *closed {
            continue;
        }
        let k = *k;
        step!(env, ClaimStep::BeforeFreshRead(k));
        if claim_held(env, owner, k, claim) {
            busy = true;
            continue;
        }
        step!(env, ClaimStep::BeforeMarker(k));
        create_marker(env.root, k)?;
        step!(env, ClaimStep::BeforeSecondRead(k));
        if bound_live_now(env, owner, k, claim) {
            // G7(b): the holder renewed between our reads — this attempt does not start.
            busy = true;
        }
    }
    Ok(if busy { Evaluated::Busy } else { Evaluated::Clear })
}

/// G3 as corrected by K3 / K8: the number of DISTINCT autonomous repairs with a claim dated `today`
/// (closed claims count — failures count), an unreadable claim counting as one while younger than
/// 24 h (then reported on stderr and no longer counted); and whether `id` is among them.
fn count_today(env: &ClaimEnv<'_>, snap: &Snapshot, today: &str, id: &RepairId) -> (u32, bool) {
    let mut repairs: Vec<&RepairId> = Vec::new();
    let mut unreadable = 0u32;
    let now = (env.clock)();
    for (k, claim, _) in snap {
        match claim {
            ClaimRead::Ok(c) if c.day == today => {
                if !repairs.contains(&&c.repair) {
                    repairs.push(&c.repair);
                }
            }
            ClaimRead::Ok(_) => {}
            ClaimRead::Unreadable => {
                let p = claim_path_in(env.root, u64::from(*k));
                if age_of(&p, now) < UNREADABLE_CLAIM_CAP_SECS as f64 {
                    unreadable += 1;
                } else {
                    // K8: reported, no longer counted (stderr only — F1: nothing to a queue)
                    eprintln!(
                        "[cysd] repair grant: claim {} is unreadable and older than 24 h — no longer counted toward the daily cap",
                        p.display()
                    );
                }
            }
        }
    }
    let mine = repairs.contains(&id);
    (repairs.len() as u32 + unreadable, mine)
}

/// Writes the claim content under a temporary name in `grant-seq/` (never counted), flushes it
/// (E1), and places it as `<n>.json` without replacing (F7(1)). `Ok(false)` = the number is taken
/// (collision). The temporary file is the attempt's own; it is removed on a collision.
fn place_claim(root: &Path, n: u32, claim: &GrantClaim) -> Result<bool, RepairError> {
    use std::io::Write;
    let dir = seq_dir_in(root);
    std::fs::create_dir_all(&dir).map_err(|e| io_err(&dir, e))?;
    let target = claim_path_in(root, u64::from(n));
    let bytes = serde_json::to_vec_pretty(claim).map_err(|e| io_err(&target, e))?;
    static TMP_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = TMP_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = dir.join(format!(".tmp-claim-{n}-{}-{seq}-{nonce}", std::process::id()));
    {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
            .map_err(|e| io_err(&tmp, e))?;
        let written = f
            .write_all(&bytes)
            .and_then(|()| ledger::full_flush_file(&f));
        if let Err(e) = written {
            drop(f);
            let _ = std::fs::remove_file(&tmp);
            return Err(io_err(&tmp, e));
        }
    }
    match place_no_replace(&tmp, &target) {
        Ok(()) => {
            let _ = ledger::full_flush_dir(&dir);
            claim_cache()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(target, claim.clone());
            Ok(true)
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            let _ = std::fs::remove_file(&tmp);
            Ok(false)
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(io_err(&target, e))
        }
    }
}

/// No-replace placement of a complete file (F7(1)): macOS `lstat` + `renamex_np(RENAME_EXCL)`;
/// Windows a move WITHOUT `MOVEFILE_REPLACE_EXISTING` (`std::fs::rename` replaces there — the
/// comment at `src/lib.rs` 582–583); other unix `link` (fails if the name exists) + unlink of the
/// source. Never `std::fs::rename`. `AlreadyExists` = the target name is taken.
fn place_no_replace(src: &Path, dst: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::ffi::OsStrExt;
        if std::fs::symlink_metadata(dst).is_ok() {
            return Err(std::io::Error::from(std::io::ErrorKind::AlreadyExists));
        }
        let a = std::ffi::CString::new(src.as_os_str().as_bytes())
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
        let b = std::ffi::CString::new(dst.as_os_str().as_bytes())
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
        // SAFETY: two NUL-terminated paths owned by this frame for the duration of the call.
        let rc = unsafe { libc::renamex_np(a.as_ptr(), b.as_ptr(), libc::RENAME_EXCL) };
        if rc == 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::fs::hard_link(src, dst)?;
        let _ = std::fs::remove_file(src);
        Ok(())
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH};
        let wide = |p: &Path| -> Vec<u16> {
            p.as_os_str().encode_wide().chain(std::iter::once(0)).collect()
        };
        let (a, b) = (wide(src), wide(dst));
        // SAFETY: two NUL-terminated wide strings owned by this frame for the duration of the call.
        let ok = unsafe { MoveFileExW(a.as_ptr(), b.as_ptr(), MOVEFILE_WRITE_THROUGH) };
        if ok != 0 {
            return Ok(());
        }
        let e = std::io::Error::last_os_error();
        // ERROR_FILE_EXISTS (80) / ERROR_ALREADY_EXISTS (183)
        if matches!(e.raw_os_error(), Some(80) | Some(183)) {
            return Err(std::io::Error::from(std::io::ErrorKind::AlreadyExists));
        }
        Err(e)
    }
}

/// G1 step 3 "LOCK FIRST" for an EXISTING record: writes `lock = {owner, expires_at, claim}` durably
/// through T3's ledger calls; refused (`repair_locked`) when another owner's lock is live (G6).
/// `Ok(false)` = there is no record yet (a new repair: the claim comes from inside
/// `ledger::create_locked_in`'s builder — the record is written right after; until then a claim
/// naming it is held by K5 / interpretation 33).
fn lock_first(
    env: &ClaimEnv<'_>,
    owner: &LockOwner,
    id: &RepairId,
    claim: u32,
) -> Result<bool, RepairError> {
    let now = (env.clock)();
    let Some(mut rec) = ledger::read_record_in(env.root, id)? else {
        return Ok(false);
    };
    if let Some(l) = rec.lock.as_ref() {
        if !lock::owned_by(l, owner) && lock::live_for_others(l, now, &owner.lane, &owner.incarnation)
        {
            return Err(RepairError::Locked(format!(
                "{}: 다른 좌석(부서 {} · 좌석 {})이 이 수리의 잠금을 쥐고 있습니다",
                id.as_str(),
                l.owner.lane.0,
                l.owner.seat
            )));
        }
    }
    rec.lock = Some(lock::new_lease(owner.clone(), now, Some(claim)));
    ledger::write_record_durable_in(env.root, &rec)?;
    Ok(true)
}

/// G1 — one autonomous-start attempt, exactly these steps; the target number is never recomputed:
/// (0) grant off → refused; (1) snapshot, h; (2) close closable claims ≤ h (G2 + G7(b)) → `Busy` /
/// `Paused` / `CapReached`; (3) lock first; (4) date re-read (M3) → `LostRace` if it changed, else
/// no-replace placement of h+1 (collision → `LostRace`); a `LostRace` releases only the lock this
/// attempt wrote (G6: own owner and own claim number); (5) `Allowed`.
pub fn try_claim_in(
    env: &ClaimEnv<'_>,
    owner: &LockOwner,
    id: &RepairId,
) -> Result<StartVerdict, RepairError> {
    let decision = read_decision_in(env.root).unwrap_or(None);
    if !is_on(decision.as_ref()) {
        return Err(RepairError::Precondition(GRANT_OFF_KO.to_string()));
    }
    // RULINGS CV1: the generation the attempt is admitted under (re-checked at the end)
    let generation = decision.as_ref().map(|d| d.generation).unwrap_or(0);
    let cap = decision
        .as_ref()
        .map(|d| d.daily_cap)
        .filter(|c| *c > 0)
        .unwrap_or(DEFAULT_DAILY_CAP);
    // (1) SNAPSHOT
    let (h, snap) = snapshot(env.root);
    let target = h
        .checked_add(1)
        .ok_or_else(|| RepairError::Precondition("자동 수리 차례 번호가 다 찼습니다".into()))?;
    step!(env, ClaimStep::Snapshot);
    // (2) EVALUATE
    let day = (env.today)();
    let evaluated = evaluate(env, owner, &snap)?;
    step!(env, ClaimStep::AfterEvaluate);
    if let Evaluated::Busy = evaluated {
        return Ok(StartVerdict::Busy);
    }
    refresh_pause_in(env.root); // DM1: a not-yet-visible input is written first
    let held = |now: f64| restart_hold_until(owner).is_some_and(|h| now < h); // DG3 (c)
    if is_paused_in(env.root, (env.clock)()) || held((env.clock)()) {
        return Ok(StartVerdict::Paused);
    }
    let (count, mine) = count_today(env, &snap, &day, id);
    if count >= cap && !mine {
        return Ok(StartVerdict::CapReached);
    }
    // (3) LOCK FIRST
    let locked = lock_first(env, owner, id, target)?;
    step!(env, ClaimStep::AfterLock);
    let release = || {
        if locked {
            let _ = ledger::release_lock_in(env.root, id, owner, Some(target));
        }
    };
    // (4) PLACE — the date is read again right before placing (M3)
    let placed_day = (env.today)();
    if placed_day != day {
        release();
        return Ok(StartVerdict::LostRace);
    }
    step!(env, ClaimStep::BeforePlace);
    let claim = GrantClaim {
        schema: super::REPAIR_SCHEMA,
        repair: id.clone(),
        lane_key: owner.lane.clone(),
        day: placed_day,
        claimed_at: (env.clock)(),
    };
    match place_claim(env.root, target, &claim) {
        Ok(true) => {}
        Ok(false) => {
            release();
            return Ok(StartVerdict::LostRace);
        }
        Err(e) => {
            release();
            return Err(e);
        }
    }
    step!(env, ClaimStep::AfterPlace);
    // (5) ADMIT — RULINGS CV1 / DG2: admission is decided at the END, on ONE coherent state. With
    // the clock read once (`now`), the decision and the pause are each read TWICE in alternation
    // (decision, pause, decision, pause): admitted only when both decision reads are ON with the
    // generation the attempt started under and both pause reads are equal and not paused at `now`.
    // Generations only grow and stored pause ends never move back (DG4), so equal reads bracket
    // an instant at which that decision and that pause held together — never a combination of
    // two independent reads. A refusal closes the attempt's own claim (`<n>.closed` — a closed
    // claim counts toward today's cap: failures count, G3 / K3) and releases its own lock.
    // Anything published after the second reads is ordered after this start (a running round is
    // not stopped by a pause — B4).
    let now = (env.clock)();
    refresh_pause_in(env.root);
    let d1 = read_decision_in(env.root).unwrap_or(None);
    let p1 = pause_state_in(env.root, now);
    step!(env, ClaimStep::BetweenAdmissionReads);
    let d2 = read_decision_in(env.root).unwrap_or(None);
    let p2 = pause_state_in(env.root, now);
    let gen = |d: &Option<GrantDecision>| d.as_ref().map(|d| (d.state, d.generation));
    let on_now = is_on(d1.as_ref()) && is_on(d2.as_ref());
    let same = gen(&d1) == gen(&d2) && d2.as_ref().map(|d| d.generation) == Some(generation);
    let paused = p1.paused_at(now) || p2.paused_at(now) || p1 != p2 || held(now);
    if !on_now || !same || paused {
        if let Err(e) = create_marker(env.root, target) {
            // stderr only (F1); the claim's lock is released below, so it is closable by any evaluator
            eprintln!("[cysd] repair grant: refused start could not close its claim {target}: {e:?}");
        }
        release();
        if !on_now {
            return Err(RepairError::Precondition(GRANT_OFF_KO.to_string()));
        }
        if paused {
            return Ok(StartVerdict::Paused);
        }
        return Ok(StartVerdict::LostRace);
    }
    Ok(StartVerdict::Allowed { claim: target })
}

/// The production environment: the real repairs root, wall clock and local date, no hook.
pub fn production_env<'a>(
    root: &'a Path,
    clock: &'a dyn Fn() -> f64,
    today: &'a dyn Fn() -> String,
) -> ClaimEnv<'a> {
    ClaimEnv {
        root,
        clock,
        today,
        hook: &no_hook,
    }
}

/// RULINGS K8 / G1; a further round under a live, unclosed claim never calls it (K3).
pub fn try_claim_autonomous_start(
    owner: &LockOwner,
    id: &RepairId,
) -> Result<StartVerdict, RepairError> {
    let root = super::paths::repairs_root();
    let clock = ledger::now_epoch;
    let today = local_today;
    try_claim_in(&production_env(&root, &clock, &today), owner, id)
}

/// Step 2 of G1 under `root`: writes `<n>.closed` for every claim ≤ `high_water` that G2 makes
/// closable (`owner` = the evaluating daemon's lane and incarnation); returns how many claims ≤
/// `high_water` remain open.
pub fn observe_and_close_claims_in(
    env: &ClaimEnv<'_>,
    owner: &LockOwner,
    high_water: u32,
) -> Result<u32, RepairError> {
    let (_, snap) = snapshot(env.root);
    let snap: Snapshot = snap.into_iter().filter(|(n, _, _)| *n <= high_water).collect();
    evaluate(env, owner, &snap)?;
    let (_, after) = snapshot(env.root);
    Ok(after
        .iter()
        .filter(|(n, _, closed)| *n <= high_water && !closed)
        .count() as u32)
}

/// Step 2 of G1: writes `<n>.closed` for every claim ≤ `high_water` that G2 makes closable; returns
/// how many remain open. Judged as a daemon with no lock of its own (lane `""`, incarnation `""`):
/// every lock is judged by its lease alone.
pub fn observe_and_close_claims(high_water: u32) -> Result<u32, RepairError> {
    let root = super::paths::repairs_root();
    let clock = ledger::now_epoch;
    let today = local_today;
    let anyone = LockOwner {
        lane: LaneKey(String::new()),
        seat: 0,
        incarnation: String::new(),
    };
    observe_and_close_claims_in(&production_env(&root, &clock, &today), &anyone, high_water)
}

/// B4 / K3: whether a new round (attempt k+1) of a grant repair may open now — refused while the
/// grant is paused, and while it is off (the owner withdrew the standing mission; `commit` and
/// `revert` stay open — only a NEW round is refused).
pub fn may_open_new_round_in(root: &Path, now: f64) -> Result<bool, RepairError> {
    refresh_pause_in(root); // DM1: the new-round check writes a not-yet-visible input first
    let on = is_on(read_decision_in(root).unwrap_or(None).as_ref());
    Ok(on && !is_paused_in(root, now))
}

/// RULINGS CH7: the refusal of a new round when the grant is OFF says so.
pub const NEW_ROUND_OFF_KO: &str = "자동 수리 허락이 꺼져 있습니다 — 새 라운드를 열 수 없습니다(확정·되돌리기는 그대로 할 수 있습니다)";
/// B4: the refusal of a new round while the grant is paused (owner input in the last 30 minutes).
pub const NEW_ROUND_PAUSED_KO: &str = "자동 수리 허락이 일시정지 중이라 새 라운드를 열 수 없습니다";

/// The sentence of a refused new round (`may_open_new_round_in` false): off or paused (CH7).
pub fn new_round_refusal_in(root: &Path) -> &'static str {
    if is_on(read_decision_in(root).unwrap_or(None).as_ref()) {
        NEW_ROUND_PAUSED_KO
    } else {
        NEW_ROUND_OFF_KO
    }
}

/// [`new_round_refusal_in`] on the production root (the same root `may_open_new_round` reads).
pub fn new_round_refusal(id: &RepairId) -> &'static str {
    let _ = id;
    new_round_refusal_in(&super::paths::repairs_root())
}

/// B4 / K3 (production root and clock).
pub fn may_open_new_round(id: &RepairId) -> Result<bool, RepairError> {
    let _ = id; // the rule is the installation's pause, not the repair's (K3: rounds take no claim)
    may_open_new_round_in(&super::paths::repairs_root(), ledger::now_epoch())
}

/// Whether a repair state is one the grant's concurrency counts as being worked (F1 / G2).
pub fn being_worked(state: RepairState) -> bool {
    !state.closes_for_grant()
}

#[cfg(test)]
#[path = "grant_tests.rs"]
mod claim_tests;

#[cfg(test)]
mod tests {
    use super::*;

    /// O-C1-5, G7, K8, B4, B1, K4.
    #[test]
    fn timing_constants() {
        assert_eq!(crate::repair::types::LEASE_SECS, 7_200);
        assert_eq!(EXPIRY_GRACE_SECS, 60);
        assert_eq!(UNREADABLE_CLAIM_CAP_SECS, 86_400);
        assert_eq!(PAUSE_QUIET_SECS, 1_800);
        assert_eq!(DEFAULT_DAILY_CAP, 5);
        assert_eq!(crate::repair::rehearsal::REFERENCE_SLOWDOWN_PERMILLE, 1_500);
    }
}
