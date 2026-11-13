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

// ───────────────────────────── the pause (B4) ─────────────────────────────

/// One lane's pause file: the last owner input that lane's daemon saw.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PauseMark {
    pub schema: u32,
    pub lane_key: LaneKey,
    pub paused_at: f64,
    pub resumed_at: f64,
}

/// B4 "주인의 아무 입력이면 즉시 정지": records an owner input seen by `lane`'s daemon at `now`.
/// Each lane writes ONLY its own file (one writer per file — a read-modify-write of `grant.json` by
/// several daemons could lose a concurrent `grant off` and turn the grant back on, the unsafe
/// direction). Written only while the grant is on (off needs no pause). Sends nothing to any queue.
/// Returns whether a mark was written.
pub fn note_owner_input_in(root: &Path, lane: &LaneKey, now: f64) -> Result<bool, RepairError> {
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
    let mark = PauseMark {
        schema: super::REPAIR_SCHEMA,
        lane_key: lane.clone(),
        paused_at: now,
        resumed_at: now + PAUSE_QUIET_SECS as f64,
    };
    crate::atomic_write_json(&p, &mark).map_err(|e| io_err(&p, e))?;
    Ok(true)
}

/// The time of the last owner input known to the grant (`grant.json.paused_at` written with the
/// decision, and every lane's pause file); an unreadable pause file counts with its modification
/// time (fail closed — the pause holds), one whose time cannot be read counts as "now".
pub fn last_owner_input_in(root: &Path, now: f64) -> Option<f64> {
    let mut last: Option<f64> = read_decision_in(root).ok().flatten().and_then(|d| d.paused_at);
    let mut bump = |t: f64| last = Some(last.map_or(t, |l: f64| l.max(t)));
    let dir = root.join("grant-pause");
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
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
                Ok(m) => bump(m.paused_at),
                Err(_) => bump(mtime_epoch(&p).unwrap_or(now)),
            }
        }
    }
    last
}

/// B4: paused while the last owner input is less than 30 minutes old. A clock set back keeps the
/// pause on (the safe side).
pub fn is_paused_in(root: &Path, now: f64) -> bool {
    last_owner_input_in(root, now).is_some_and(|t| now < t + PAUSE_QUIET_SECS as f64)
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
    if is_paused_in(env.root, (env.clock)()) {
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
    // (5) START
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
