//! A-ID S1 (0.14.52) — close authorization: original-child probe + pure owner evaluator.
//!
//! Locked criterion AI4-1: positive pane membership = raw map hit AND surface not marked exited AND the
//! ORIGINAL child proved running by a NON-BLOCKING probe (`try_lock` + `try_wait` on `Surface.child`).
//! Lock contention or unreadable child state = NOT proved running -> ADMIT. Corroboration (process
//! readable, start time, registration observation) is RECORDED, never used for the decision.
//! AI4-2 row 17: a walk that was not acquired (timeout, reader unavailable) has ONE disposition:
//! ADMIT + recorded reason. AI4-3: no member (M = no rows) is one N/A row = ADMIT.
//!
//! S1 is NOT wired: nothing in production calls this module yet. Consumers: S3 (owner walk builds a
//! `WalkObservation`, captures a `SurfaceSnapshot` and calls `evaluate_owner`), S11 (puts
//! `OwnerVerdict::describe` into the refusal of a vetoed close — head answer Q9), S10 (records the
//! `MemberRecord`s). Tests: `close_auth_tests.rs`.
// Every item below is consumed by S3 / S10 / S11 (see the module doc). Remove this allow when S3 wires
// the module in; test builds still report items that not even the tests use.
#![cfg_attr(not(test), allow(dead_code))]

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, TryLockError};

use portable_pty::Child;

use crate::state::{Daemon, Surface};

/// Surface id as keyed in `Daemon.surfaces`.
pub(crate) type SurfaceId = u64;

/// Result of the non-blocking probe of one surface's ORIGINAL child (`Surface.child`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChildProbe {
    /// `Surface.exited` is set (the PTY reader saw EOF) — not a running member.
    SurfaceExited,
    /// `try_wait` returned `Ok(None)`: the original child is proved running.
    Running,
    /// `try_wait` returned an exit status.
    Ended,
    /// `try_lock` would block (the EOF reader or `close_surface` holds the child) — not proved running.
    Contended,
    /// Poisoned mutex, `try_wait` error, or a caught `try_wait` panic — not proved running.
    Unreadable,
}

impl ChildProbe {
    /// Stable machine string.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            ChildProbe::SurfaceExited => "surface_exited",
            ChildProbe::Running => "running",
            ChildProbe::Ended => "ended",
            ChildProbe::Contended => "contended",
            ChildProbe::Unreadable => "unreadable",
        }
    }

    /// Human text naming what the probe saw (goes into the refusal text of a vetoed close, Q9).
    pub(crate) fn describe(self) -> &'static str {
        match self {
            ChildProbe::SurfaceExited => "surface_exited (exited flag set by the PTY reader)",
            ChildProbe::Running => "running (try_wait: still active)",
            ChildProbe::Ended => "ended (try_wait: exit status available)",
            ChildProbe::Contended => "contended (child mutex held elsewhere; not proved running)",
            ChildProbe::Unreadable => "unreadable (poisoned, try_wait error or panic; not proved running)",
        }
    }
}

/// The non-blocking probe of one surface's original child. Takes the two `Surface` fields so it is
/// testable without a `Surface`.
///
/// 1. `exited` FIRST: the EOF reader stores it before it takes `child` (and then may hold `child` across
///    a 1000 ms sleep), so most EOF-time contention reads as `SurfaceExited` without touching the lock.
/// 2. `try_lock` only — NEVER `lock()`: the probe runs on the request thread, and `close_surface` takes
///    `surfaces` then `child`. WouldBlock = `Contended`, Poisoned = `Unreadable` (both ADMIT).
/// 3. `try_wait` inside `catch_unwind`, with the guard OUTSIDE the closure: a panic is caught while the
///    guard is still alive and the guard is dropped after the catch, when the thread is no longer
///    panicking — the mutex is not poisoned (`close_surface` and the watchdog reaper `lock().unwrap()` it).
///    On unix `try_wait` reaps an exited child; std caches the status, so the EOF reader and
///    `close_surface` (same `Child` object) still see it.
///
/// Known Windows limit (head answer Q9, accepted): `WinChild::is_complete` returns `Ok(None)` when
/// `GetExitCodeProcess` fails — that reads as `Running` (VETO, pane kept; the refusal names the probe
/// result via `OwnerVerdict::describe`). Q9-b propagates `is_complete` clone failures as IO errors,
/// mapped here to `Unreadable`; existing poison panics are still caught. Other vendor clone/wait paths
/// and the `GetExitCodeProcess` failure result remain unchanged.
pub(crate) fn probe_child(exited: &AtomicBool, child: &Mutex<Box<dyn Child + Send + Sync>>) -> ChildProbe {
    if exited.load(Ordering::Relaxed) {
        return ChildProbe::SurfaceExited;
    }
    let mut guard = match child.try_lock() {
        Ok(g) => g,
        Err(TryLockError::WouldBlock) => return ChildProbe::Contended,
        Err(TryLockError::Poisoned(_)) => return ChildProbe::Unreadable,
    };
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| guard.try_wait()));
    drop(guard);
    match caught {
        Ok(Ok(None)) => ChildProbe::Running,
        Ok(Ok(Some(_))) => ChildProbe::Ended,
        Ok(Err(_)) | Err(_) => ChildProbe::Unreadable,
    }
}

/// Thin wrapper: probe a live `Surface`.
pub(crate) fn probe_surface(s: &Surface) -> ChildProbe {
    probe_child(&s.exited, &s.child)
}

/// Corroboration recorded per member. RECORDED ONLY — `evaluate_owner` never reads it (AI4-1:
/// "Registration start and fresh process start are recorded corroboration, NOT required").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Corroboration {
    /// Whether the walk reader could read this hop's process (`None` = not attempted).
    pub(crate) process_readable: Option<bool>,
    /// Process start time read during the walk (`None` = not read; `Some(0)` = read as zero).
    pub(crate) start: Option<u64>,
    /// Start time observed at spawn registration (S2); `None` = registration observation absent.
    pub(crate) registration: Option<u64>,
}

/// One hop of the caller's ancestry walk, in walk order (caller first, then parents).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WalkHop {
    pub(crate) pid: u32,
    pub(crate) corroboration: Corroboration,
}

/// Why a walk was not acquired (AI4-2 row 17).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NotAcquiredReason {
    Timeout,
    ReaderUnavailable,
}

/// What the owner walk (S3) observed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WalkObservation {
    Acquired { hops: Vec<WalkHop> },
    NotAcquired(NotAcquiredReason),
}

/// Record of one walk hop that hit the surfaces-only PID map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MemberRecord {
    pub(crate) pid: u32,
    pub(crate) sid: SurfaceId,
    pub(crate) probe: ChildProbe,
    pub(crate) corroboration: Corroboration,
}

/// Finite reasons for an admitted close.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AdmitReason {
    /// No hop hit the map (the AI4-3 N/A row).
    NoMember,
    SurfaceExited,
    ChildEnded,
    ChildContended,
    ChildUnreadable,
    /// The walk was not acquired (AI4-2 row 17).
    NotAcquired(NotAcquiredReason),
}

impl AdmitReason {
    /// Stable machine string.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            AdmitReason::NoMember => "no_member",
            AdmitReason::SurfaceExited => "surface_exited",
            AdmitReason::ChildEnded => "child_ended",
            AdmitReason::ChildContended => "child_contended",
            AdmitReason::ChildUnreadable => "child_unreadable",
            AdmitReason::NotAcquired(NotAcquiredReason::Timeout) => "not_acquired_timeout",
            AdmitReason::NotAcquired(NotAcquiredReason::ReaderUnavailable) => "not_acquired_reader_unavailable",
        }
    }

    /// The admit reason a non-running probe gives; `None` for `Running` (that member vetoes).
    fn of_probe(p: ChildProbe) -> Option<AdmitReason> {
        match p {
            ChildProbe::Running => None,
            ChildProbe::SurfaceExited => Some(AdmitReason::SurfaceExited),
            ChildProbe::Ended => Some(AdmitReason::ChildEnded),
            ChildProbe::Contended => Some(AdmitReason::ChildContended),
            ChildProbe::Unreadable => Some(AdmitReason::ChildUnreadable),
        }
    }

    /// Precedence when no member is running: the WEAKEST basis for the admission wins, so a record never
    /// hides that an admission rested on an unproven member. Unreadable > contended (not proved either
    /// way) > surface_exited (flag only; the child itself was not asked) > child_ended (proved ended) >
    /// no_member. `NotAcquired` never competes (no members are evaluated).
    fn weakness(self) -> u8 {
        match self {
            AdmitReason::NoMember => 0,
            AdmitReason::ChildEnded => 1,
            AdmitReason::SurfaceExited => 2,
            AdmitReason::ChildContended => 3,
            AdmitReason::ChildUnreadable => 4,
            AdmitReason::NotAcquired(_) => 5,
        }
    }
}

/// Finite reasons for a vetoed close (one today).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VetoReason {
    /// A pane process in the caller's ancestry has its original child proved running.
    PaneAncestorRunning,
}

impl VetoReason {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            VetoReason::PaneAncestorRunning => "pane_ancestor_running",
        }
    }
}

/// Owner verdict for one close.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OwnerVerdict {
    /// Pane kept. Carries the member pid, its surface id and the probe value so the refusal names them (Q9).
    Veto { pid: u32, member: SurfaceId, probe: ChildProbe, reason: VetoReason },
    Admit(AdmitReason),
}

impl OwnerVerdict {
    /// Stable machine string.
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            OwnerVerdict::Veto { reason, .. } => reason.as_str(),
            OwnerVerdict::Admit(r) => r.as_str(),
        }
    }

    /// Human text. For a veto it names the probe result, the pid and the surface id (Q9: a kept pane
    /// must be explainable, not a silent stuck state).
    pub(crate) fn describe(&self) -> String {
        match self {
            OwnerVerdict::Veto { pid, member, probe, reason } => format!(
                "{}: pid {pid} in the caller's ancestry is the process of surface {member}, whose original child is still running (probe: {})",
                reason.as_str(),
                probe.describe()
            ),
            OwnerVerdict::Admit(r) => format!("admitted: {}", r.as_str()),
        }
    }
}

/// Evaluator output: the verdict plus every member probed, in walk order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OwnerEvaluation {
    pub(crate) verdict: OwnerVerdict,
    pub(crate) members: Vec<MemberRecord>,
}

/// Pure owner evaluator (AI4-1, AI4-2 row 17, AI4-3).
///
/// - Walk not acquired -> `Admit(NotAcquired(reason))`, nothing probed (row 17: one disposition).
/// - Every hop in walk order; a hop whose pid is in the surfaces-only `map` is a member and is probed
///   once. The first `Running` member vetoes; probing stops there (later members cannot change a veto).
///   A non-running member does NOT stop the walk — a running member further up still vetoes.
/// - No running member -> `Admit` with the weakest basis among the members (`AdmitReason::weakness`,
///   ties -> the member nearest the caller), or `NoMember` when no hop hit the map (AI4-3 N/A row).
/// - Corroboration is copied into the member records and never read for the decision.
pub(crate) fn evaluate_owner(
    walk: &WalkObservation,
    map: &SurfacePidMap,
    mut probe: impl FnMut(SurfaceId) -> ChildProbe,
) -> OwnerEvaluation {
    let hops = match walk {
        WalkObservation::Acquired { hops } => hops,
        WalkObservation::NotAcquired(reason) => {
            return OwnerEvaluation { verdict: OwnerVerdict::Admit(AdmitReason::NotAcquired(*reason)), members: Vec::new() }
        }
    };
    let mut members = Vec::new();
    let mut admit = AdmitReason::NoMember;
    for hop in hops {
        let Some(sid) = map.get(hop.pid) else { continue };
        let p = probe(sid);
        members.push(MemberRecord { pid: hop.pid, sid, probe: p, corroboration: hop.corroboration });
        match AdmitReason::of_probe(p) {
            None => {
                return OwnerEvaluation {
                    verdict: OwnerVerdict::Veto { pid: hop.pid, member: sid, probe: p, reason: VetoReason::PaneAncestorRunning },
                    members,
                }
            }
            Some(r) if r.weakness() > admit.weakness() => admit = r,
            Some(_) => {}
        }
    }
    OwnerEvaluation { verdict: OwnerVerdict::Admit(admit), members }
}

/// Surfaces-only PID -> surface id map for the owner walk (Q17). Built from `Daemon.surfaces` ONLY —
/// never merged with the delegated cycle aliases (`Daemon.delegated_callers`) the legacy caller walk
/// adds: an alias has no `Surface.child` to probe, and a bare delegated alias is not an owner veto.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct SurfacePidMap {
    by_pid: HashMap<u32, SurfaceId>,
}

impl SurfacePidMap {
    pub(crate) fn from_pairs(pairs: impl IntoIterator<Item = (u32, SurfaceId)>) -> Self {
        SurfacePidMap { by_pid: pairs.into_iter().collect() }
    }

    pub(crate) fn get(&self, pid: u32) -> Option<SurfaceId> {
        self.by_pid.get(&pid).copied()
    }
}

/// Point-in-time capture of the surfaces the owner walk may probe.
pub(crate) struct SurfaceSnapshot {
    pids: SurfacePidMap,
    surfaces: HashMap<SurfaceId, Arc<Surface>>,
}

impl SurfaceSnapshot {
    /// Clone the surface `Arc`s under the `surfaces` lock and release it before anything is probed
    /// (lock order: `close_surface` takes `surfaces` then `Surface.child`; the probe only `try_lock`s).
    pub(crate) fn capture(daemon: &Daemon) -> Self {
        let surfaces: Vec<Arc<Surface>> = daemon.surfaces.lock().unwrap().values().cloned().collect();
        let by_pid: HashMap<u32, SurfaceId> = surfaces.iter().map(|s| (s.pid, s.id)).collect();
        SurfaceSnapshot {
            pids: SurfacePidMap { by_pid },
            surfaces: surfaces.into_iter().map(|s| (s.id, s)).collect(),
        }
    }

    pub(crate) fn pid_map(&self) -> &SurfacePidMap {
        &self.pids
    }

    /// Probe one captured surface. A sid outside the capture is `Unreadable` (not proved running).
    pub(crate) fn probe(&self, sid: SurfaceId) -> ChildProbe {
        match self.surfaces.get(&sid) {
            Some(s) => probe_surface(s),
            None => ChildProbe::Unreadable,
        }
    }
}
