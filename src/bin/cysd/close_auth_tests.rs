//! A-ID S1 (0.14.52) — fixtures for `close_auth` (original-child probe + pure owner evaluator).
//!
//! Locked criterion AI4-1: running original child -> VETO in every column (corroboration recorded, not
//! required); not running or unprovable (ended, contended, unreadable, surface exited) -> ADMIT.
//! AI4-2 row 17: walk not acquired -> ADMIT + recorded reason. AI4-3: no member -> one N/A row (ADMIT).
//! Fakes: `ScriptedChild` scripts `try_wait` (running / exited / error / panic). One unix test drives a
//! real `sleep 30` child from `portable_pty::native_pty_system()`. No process here is spawned through
//! `std::process`, and no fixture reads or prints a seat token.
#![cfg(test)]

use crate::close_auth::{
    evaluate_owner, probe_child, AdmitReason, ChildProbe, Corroboration, MemberRecord, NotAcquiredReason, OwnerEvaluation,
    OwnerVerdict, SurfaceId, SurfacePidMap, SurfaceSnapshot, VetoReason, WalkHop, WalkObservation,
};
use crate::state::Daemon;
use portable_pty::{Child, ChildKiller, ExitStatus};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

// Fake pids above every OS pid range (macOS 99_998, Linux 4_194_304) — never a real process.
const CALLER: u32 = 4_100_000_001;
const MEMBER_PID: u32 = 4_100_000_002;
const MID: u32 = 4_100_000_003;
const ANCESTOR_PID: u32 = 4_100_000_004;
const MEMBER_SID: SurfaceId = 7;
const ANCESTOR_SID: SurfaceId = 3;

#[derive(Debug, Clone, Copy)]
enum Script {
    Running,
    Exited(u32),
    Error,
    Panic,
}

/// Fake original child with a scripted `try_wait`.
#[derive(Debug, Clone)]
struct ScriptedChild {
    script: Script,
    pid: u32,
}

impl ChildKiller for ScriptedChild {
    fn kill(&mut self) -> std::io::Result<()> {
        Ok(())
    }
    fn clone_killer(&self) -> Box<dyn ChildKiller + Send + Sync> {
        Box::new(self.clone())
    }
}

impl Child for ScriptedChild {
    fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
        match self.script {
            Script::Running => Ok(None),
            Script::Exited(code) => Ok(Some(ExitStatus::with_exit_code(code))),
            Script::Error => Err(std::io::Error::other("scripted try_wait error")),
            Script::Panic => panic!("scripted try_wait panic (F-PANIC fixture)"),
        }
    }
    fn wait(&mut self) -> std::io::Result<ExitStatus> {
        Ok(ExitStatus::with_exit_code(0))
    }
    fn process_id(&self) -> Option<u32> {
        Some(self.pid)
    }
    #[cfg(windows)]
    fn as_raw_handle(&self) -> Option<std::os::windows::io::RawHandle> {
        None
    }
}

/// The two `Surface` fields the probe reads.
struct Pane {
    exited: AtomicBool,
    child: Mutex<Box<dyn Child + Send + Sync>>,
}

fn pane(script: Script, pid: u32, exited: bool) -> Arc<Pane> {
    Arc::new(Pane { exited: AtomicBool::new(exited), child: Mutex::new(Box::new(ScriptedChild { script, pid })) })
}

fn probe(p: &Pane) -> ChildProbe {
    probe_child(&p.exited, &p.child)
}

fn full() -> Corroboration {
    Corroboration { process_readable: Some(true), start: Some(1_760_000_000), registration: Some(1_760_000_000) }
}

fn hop(pid: u32) -> WalkHop {
    WalkHop { pid, corroboration: full() }
}

fn walk(pids: &[u32]) -> WalkObservation {
    WalkObservation::Acquired { hops: pids.iter().map(|p| hop(*p)).collect() }
}

/// Surfaces-only map + the fake panes behind it; probes go through the real `probe_child`.
struct World {
    map: SurfacePidMap,
    panes: HashMap<SurfaceId, Arc<Pane>>,
}

impl World {
    fn new(members: Vec<(u32, SurfaceId, Arc<Pane>)>) -> Self {
        World {
            map: SurfacePidMap::from_pairs(members.iter().map(|(pid, sid, _)| (*pid, *sid))),
            panes: members.into_iter().map(|(_, sid, p)| (sid, p)).collect(),
        }
    }
    fn eval(&self, w: &WalkObservation) -> OwnerEvaluation {
        evaluate_owner(w, &self.map, |sid| probe(&self.panes[&sid]))
    }
}

fn veto(pid: u32, member: SurfaceId) -> OwnerVerdict {
    OwnerVerdict::Veto { pid, member, probe: ChildProbe::Running, reason: VetoReason::PaneAncestorRunning }
}

// ── probe + aggregate fixtures ───────────────────────────────────────────────────────────────

/// F-ENDED: the caller's member hop has an ended original child -> ADMIT(child_ended).
#[test]
fn f_ended_admits_child_ended() {
    let p = pane(Script::Exited(0), MEMBER_PID, false);
    let w = World::new(vec![(MEMBER_PID, MEMBER_SID, p.clone())]);
    let ev = w.eval(&walk(&[CALLER, MEMBER_PID]));
    assert!(
        matches!(ev.verdict, OwnerVerdict::Admit(AdmitReason::ChildEnded)),
        "F-ENDED: an ended original child must ADMIT(child_ended), got {:?}",
        ev.verdict
    );
    assert_eq!(probe(&p), ChildProbe::Ended, "F-ENDED: probe of an ended child");
}

/// F-CONTENDED: the fixture holds the child mutex (as the EOF reader does across its 1000 ms sleep).
/// The probe runs on a second thread and must answer `Contended` without blocking; the aggregate admits.
#[test]
fn f_contended_probe_does_not_block_and_admits_child_contended() {
    let p = pane(Script::Running, MEMBER_PID, false);
    let held = p.child.lock().unwrap();
    let (tx, rx) = mpsc::channel();
    let p1 = p.clone();
    let h1 = std::thread::spawn(move || {
        let _ = tx.send(probe_child(&p1.exited, &p1.child));
    });
    assert_eq!(
        rx.recv_timeout(Duration::from_millis(200)),
        Ok(ChildProbe::Contended),
        "F-CONTENDED: the probe must not block on a held child mutex and must read it as Contended"
    );
    let (tx2, rx2) = mpsc::channel();
    let p2 = p.clone();
    let h2 = std::thread::spawn(move || {
        let map = SurfacePidMap::from_pairs([(MEMBER_PID, MEMBER_SID)]);
        let ev = evaluate_owner(&walk(&[CALLER, MEMBER_PID]), &map, |_| probe_child(&p2.exited, &p2.child));
        let _ = tx2.send(ev);
    });
    let ev = rx2
        .recv_timeout(Duration::from_millis(200))
        .expect("F-CONTENDED: the aggregate evaluation must not block on a held child mutex");
    assert!(
        matches!(ev.verdict, OwnerVerdict::Admit(AdmitReason::ChildContended)),
        "F-CONTENDED aggregate: a contended child is not proved running -> ADMIT(child_contended), got {:?}",
        ev.verdict
    );
    drop(held);
    h1.join().unwrap();
    h2.join().unwrap();
    // Control: released, the same child probes Running — the contention was the only cause.
    assert_eq!(probe(&p), ChildProbe::Running, "F-CONTENDED control: released mutex, running child");
}

/// F-EXITED-FLAG: `exited` is set while the child still reports running -> ADMIT(surface_exited).
/// The flag is checked FIRST: with the flag set, a held child mutex is never reached (not Contended).
#[test]
fn f_exited_flag_admits_surface_exited() {
    let p = pane(Script::Running, MEMBER_PID, true);
    let w = World::new(vec![(MEMBER_PID, MEMBER_SID, p.clone())]);
    let ev = w.eval(&walk(&[CALLER, MEMBER_PID]));
    assert!(
        matches!(ev.verdict, OwnerVerdict::Admit(AdmitReason::SurfaceExited)),
        "F-EXITED-FLAG: a surface marked exited must ADMIT(surface_exited), got {:?}",
        ev.verdict
    );
    assert_eq!(probe(&p), ChildProbe::SurfaceExited, "F-EXITED-FLAG: probe");
    let _held = p.child.lock().unwrap();
    assert_eq!(probe(&p), ChildProbe::SurfaceExited, "F-EXITED-FLAG: the exited flag is read before the child lock");
}

/// F-SECOND-MEMBER: the caller's own member ended, an ancestor member is running -> VETO(ancestor).
#[test]
fn f_second_member_running_ancestor_vetoes() {
    let w = World::new(vec![
        (MEMBER_PID, MEMBER_SID, pane(Script::Exited(0), MEMBER_PID, false)),
        (ANCESTOR_PID, ANCESTOR_SID, pane(Script::Running, ANCESTOR_PID, false)),
    ]);
    let ev = w.eval(&walk(&[CALLER, MEMBER_PID, MID, ANCESTOR_PID]));
    assert_eq!(
        ev.verdict,
        veto(ANCESTOR_PID, ANCESTOR_SID),
        "F-SECOND-MEMBER: a non-running member must not stop the walk; the running ancestor vetoes"
    );
    let probes: Vec<(SurfaceId, ChildProbe)> = ev.members.iter().map(|m| (m.sid, m.probe)).collect();
    assert_eq!(probes, vec![(MEMBER_SID, ChildProbe::Ended), (ANCESTOR_SID, ChildProbe::Running)]);
}

/// F-RUN-UNREADABLE: running child; the walk could not read the hop's process -> still VETO.
#[test]
fn f_run_unreadable_vetoes() {
    let w = World::new(vec![(MEMBER_PID, MEMBER_SID, pane(Script::Running, MEMBER_PID, false))]);
    let c = Corroboration { process_readable: Some(false), start: None, registration: Some(1_760_000_000) };
    let ev = w.eval(&WalkObservation::Acquired { hops: vec![hop(CALLER), WalkHop { pid: MEMBER_PID, corroboration: c }] });
    assert_eq!(ev.verdict, veto(MEMBER_PID, MEMBER_SID), "F-RUN-UNREADABLE: corroboration is not required for a veto");
}

/// F-RUN-ZERO-START: running child; the walk read a zero start time -> still VETO.
#[test]
fn f_run_zero_start_vetoes() {
    let w = World::new(vec![(MEMBER_PID, MEMBER_SID, pane(Script::Running, MEMBER_PID, false))]);
    let c = Corroboration { process_readable: Some(true), start: Some(0), registration: Some(1_760_000_000) };
    let ev = w.eval(&WalkObservation::Acquired { hops: vec![hop(CALLER), WalkHop { pid: MEMBER_PID, corroboration: c }] });
    assert_eq!(ev.verdict, veto(MEMBER_PID, MEMBER_SID), "F-RUN-ZERO-START: corroboration is not required for a veto");
}

/// F-RUN-R-ABSENT: running child; no registration observation -> still VETO.
#[test]
fn f_run_r_absent_vetoes() {
    let w = World::new(vec![(MEMBER_PID, MEMBER_SID, pane(Script::Running, MEMBER_PID, false))]);
    let c = Corroboration { process_readable: Some(true), start: Some(1_760_000_000), registration: None };
    let ev = w.eval(&WalkObservation::Acquired { hops: vec![hop(CALLER), WalkHop { pid: MEMBER_PID, corroboration: c }] });
    assert_eq!(ev.verdict, veto(MEMBER_PID, MEMBER_SID), "F-RUN-R-ABSENT: registration is recorded corroboration, not required");
}

/// Member records carry each member's corroboration verbatim (recorded, never used for the decision).
#[test]
fn f_member_records_carry_corroboration_verbatim() {
    let w = World::new(vec![
        (MEMBER_PID, MEMBER_SID, pane(Script::Exited(3), MEMBER_PID, false)),
        (ANCESTOR_PID, ANCESTOR_SID, pane(Script::Running, ANCESTOR_PID, false)),
    ]);
    let c1 = Corroboration { process_readable: Some(false), start: Some(0), registration: None };
    let c2 = Corroboration { process_readable: None, start: Some(42), registration: Some(41) };
    let ev = w.eval(&WalkObservation::Acquired {
        hops: vec![
            hop(CALLER),
            WalkHop { pid: MEMBER_PID, corroboration: c1 },
            hop(MID),
            WalkHop { pid: ANCESTOR_PID, corroboration: c2 },
        ],
    });
    assert_eq!(
        ev.members,
        vec![
            MemberRecord { pid: MEMBER_PID, sid: MEMBER_SID, probe: ChildProbe::Ended, corroboration: c1 },
            MemberRecord { pid: ANCESTOR_PID, sid: ANCESTOR_SID, probe: ChildProbe::Running, corroboration: c2 },
        ]
    );
    assert_eq!(ev.verdict, veto(ANCESTOR_PID, ANCESTOR_SID));
}

/// F-PANIC: `try_wait` panics -> Unreadable -> ADMIT(child_unreadable), and the child mutex is NOT
/// poisoned afterwards (the guard outlives the catch, so `close_surface`'s `lock().unwrap()` stays safe).
#[test]
fn f_panic_admits_child_unreadable_and_leaves_mutex_unpoisoned() {
    let p = pane(Script::Panic, MEMBER_PID, false);
    assert_eq!(probe(&p), ChildProbe::Unreadable, "F-PANIC: a panicking try_wait must be caught and read as Unreadable");
    assert!(
        p.child.try_lock().is_ok(),
        "F-PANIC: the child mutex must not be poisoned by a caught try_wait panic (guard dropped after the catch)"
    );
    let w = World::new(vec![(MEMBER_PID, MEMBER_SID, p.clone())]);
    let ev = w.eval(&walk(&[CALLER, MEMBER_PID]));
    assert!(
        matches!(ev.verdict, OwnerVerdict::Admit(AdmitReason::ChildUnreadable)),
        "F-PANIC aggregate: got {:?}",
        ev.verdict
    );
    assert!(p.child.try_lock().is_ok(), "F-PANIC: still not poisoned after the aggregate");
}

/// `try_wait` returns an error -> Unreadable -> ADMIT(child_unreadable).
#[test]
fn f_try_wait_error_is_unreadable() {
    let p = pane(Script::Error, MEMBER_PID, false);
    assert_eq!(probe(&p), ChildProbe::Unreadable, "try_wait Err must read as Unreadable");
    let w = World::new(vec![(MEMBER_PID, MEMBER_SID, p)]);
    let ev = w.eval(&walk(&[CALLER, MEMBER_PID]));
    assert!(matches!(ev.verdict, OwnerVerdict::Admit(AdmitReason::ChildUnreadable)), "got {:?}", ev.verdict);
}

/// A poisoned child mutex -> Unreadable (never `lock()`; never unwrap the poison).
#[test]
fn f_poisoned_mutex_is_unreadable() {
    let p = pane(Script::Running, MEMBER_PID, false);
    let p1 = p.clone();
    let _ = std::thread::spawn(move || {
        let _g = p1.child.lock().unwrap();
        panic!("poison the child mutex (fixture)");
    })
    .join();
    assert!(p.child.is_poisoned(), "fixture: the mutex must be poisoned");
    assert_eq!(probe(&p), ChildProbe::Unreadable, "a poisoned child mutex must read as Unreadable");
}

/// F-NOT-ACQUIRED (AI4-2 row 17): walk not acquired -> ADMIT + recorded reason; nothing is probed.
#[test]
fn f_not_acquired_admits_with_recorded_reason() {
    let map = SurfacePidMap::from_pairs([(MEMBER_PID, MEMBER_SID)]);
    for (reason, text) in
        [(NotAcquiredReason::Timeout, "not_acquired_timeout"), (NotAcquiredReason::ReaderUnavailable, "not_acquired_reader_unavailable")]
    {
        let mut calls = 0usize;
        let ev = evaluate_owner(&WalkObservation::NotAcquired(reason), &map, |_| {
            calls += 1;
            ChildProbe::Running
        });
        assert!(
            matches!(ev.verdict, OwnerVerdict::Admit(AdmitReason::NotAcquired(r)) if r == reason),
            "F-NOT-ACQUIRED: row 17 has one disposition, ADMIT + recorded reason ({reason:?}), got {:?}",
            ev.verdict
        );
        assert_eq!(ev.verdict.as_str(), text);
        assert_eq!(calls, 0, "F-NOT-ACQUIRED: no hop, nothing to probe");
        assert!(ev.members.is_empty());
    }
}

/// F-NO-MEMBER (AI4-3 N/A row): no hop hits the map -> ADMIT(no_member); nothing is probed.
#[test]
fn f_no_member_admits_the_na_row() {
    let map = SurfacePidMap::from_pairs([(MEMBER_PID, MEMBER_SID)]);
    for w in [walk(&[CALLER, MID]), walk(&[])] {
        let mut calls = 0usize;
        let ev = evaluate_owner(&w, &map, |_| {
            calls += 1;
            ChildProbe::Running
        });
        assert_eq!(ev.verdict, OwnerVerdict::Admit(AdmitReason::NoMember), "F-NO-MEMBER: {w:?}");
        assert_eq!(ev.verdict.as_str(), "no_member");
        assert_eq!(calls, 0);
        assert!(ev.members.is_empty());
    }
}

/// Isolated daemon with one real `sleep 30` surface; Drop kills and reaps every surface child and
/// removes the state dir (also on a failed assertion).
struct DaemonFx {
    daemon: Arc<Daemon>,
    dir: std::path::PathBuf,
}

impl DaemonFx {
    fn new(tag: &str) -> Self {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!("cys-ca-{tag}-{}-{}", std::process::id(), SEQ.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir_all(&dir).expect("fixture state dir");
        DaemonFx { daemon: Daemon::new(dir.join("cysd.sock")), dir }
    }
}

impl Drop for DaemonFx {
    fn drop(&mut self) {
        let surfaces: Vec<_> = self.daemon.surfaces.lock().unwrap_or_else(|e| e.into_inner()).values().cloned().collect();
        for s in surfaces {
            let mut c = s.child.lock().unwrap_or_else(|e| e.into_inner());
            let _ = c.kill();
            let _ = c.wait();
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// F-DELEGATED-ALIAS: a pid present only as a delegated cycle alias (mapped to a real, running seat)
/// is not a member — through the real map builder (`SurfaceSnapshot::capture`). Positive control: the
/// real surface's own pid IS a member, its original child probes running, and that walk vetoes; the
/// probe runs without the `surfaces` lock (captured, then released).
#[test]
fn f_delegated_alias_is_not_a_member_through_the_real_map_builder() {
    let fx = DaemonFx::new("alias");
    let s = fx.daemon.create_surface(None, Some("sleep 30".into()), None, None, 24, 80).expect("create surface");
    let alias: u32 = 4_100_000_005;
    assert_ne!(alias, s.pid);
    fx.daemon.delegated_callers.lock().unwrap().insert(alias, s.id);
    let snap = SurfaceSnapshot::capture(&fx.daemon);
    assert_eq!(snap.pid_map().get(s.pid), Some(s.id), "control: a surface's pid is in the surfaces-only map");
    let ev = evaluate_owner(&walk(&[CALLER, alias]), snap.pid_map(), |sid| snap.probe(sid));
    assert!(
        matches!(ev.verdict, OwnerVerdict::Admit(AdmitReason::NoMember)),
        "F-DELEGATED-ALIAS: a delegated cycle alias is not a pane member (surfaces-only map), got {:?}",
        ev.verdict
    );
    assert_eq!(snap.pid_map().get(alias), None);
    // Control: the real surface's own pid vetoes; evaluated on another thread while this thread holds
    // `daemon.surfaces` — the probe must not need that lock.
    let held = fx.daemon.surfaces.lock().unwrap();
    let (tx, rx) = mpsc::channel();
    let (pid, sid) = (s.pid, s.id);
    let h = std::thread::spawn(move || {
        let ev = evaluate_owner(&walk(&[CALLER, pid]), snap.pid_map(), |sid| snap.probe(sid));
        let _ = tx.send(ev.verdict);
    });
    let v = rx.recv_timeout(Duration::from_secs(2)).expect("control: probing must not wait on the surfaces lock");
    drop(held);
    h.join().unwrap();
    assert_eq!(v, veto(s.pid, sid), "control: the real surface's running original child vetoes");
}

/// F-VETO-REASON (head answer Q9): the veto has a stable machine string and a human text naming the
/// probe result, the pid and the surface id.
#[test]
fn f_veto_reason_names_probe_pid_and_surface() {
    let w = World::new(vec![(MEMBER_PID, MEMBER_SID, pane(Script::Running, MEMBER_PID, false))]);
    let v = w.eval(&walk(&[CALLER, MEMBER_PID])).verdict;
    assert_eq!(v.as_str(), "pane_ancestor_running");
    let text = v.describe();
    assert!(text.contains("probe: running (try_wait: still active)"), "F-VETO-REASON: names the probe result: {text}");
    assert!(text.contains(&format!("pid {MEMBER_PID}")), "F-VETO-REASON: names the pid: {text}");
    assert!(text.contains(&format!("surface {MEMBER_SID}")), "F-VETO-REASON: names the surface id: {text}");
}

/// Real child (unix): `sleep 30` from `native_pty_system()` -> Running; killed and waited -> Ended.
#[cfg(unix)]
#[test]
fn f_real_child_running_then_ended() {
    let pair = portable_pty::native_pty_system()
        .openpty(portable_pty::PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 })
        .expect("openpty");
    let mut cmd = portable_pty::CommandBuilder::new("sleep");
    cmd.arg("30");
    let child = pair.slave.spawn_command(cmd).expect("spawn sleep 30");
    drop(pair.slave);
    let exited = AtomicBool::new(false);
    let child = Mutex::new(child);
    assert_eq!(probe_child(&exited, &child), ChildProbe::Running, "a live sleep 30 is Running");
    {
        let mut c = child.lock().unwrap();
        c.kill().expect("kill");
        c.wait().expect("wait");
    }
    assert_eq!(probe_child(&exited, &child), ChildProbe::Ended, "a killed and reaped child is Ended");
    drop(pair.master);
}

// ── documented choices ───────────────────────────────────────────────────────────────────────

/// Admit reason when no member is running: the WEAKEST basis among the members (unreadable >
/// contended > surface_exited > child_ended; ties -> nearest the caller). Probing stops at the first
/// running member (later members cannot change a veto).
#[test]
fn f_admit_reason_precedence() {
    use ChildProbe::*;
    let map = SurfacePidMap::from_pairs([(MEMBER_PID, MEMBER_SID), (ANCESTOR_PID, ANCESTOR_SID), (MID, 5)]);
    let cases: [(&[ChildProbe], AdmitReason); 7] = [
        (&[Ended, Contended], AdmitReason::ChildContended),
        (&[Contended, Ended], AdmitReason::ChildContended),
        (&[Contended, Unreadable], AdmitReason::ChildUnreadable),
        (&[Unreadable, Contended], AdmitReason::ChildUnreadable),
        (&[Ended, SurfaceExited], AdmitReason::SurfaceExited),
        (&[SurfaceExited, Ended], AdmitReason::SurfaceExited),
        (&[Ended, Ended], AdmitReason::ChildEnded),
    ];
    for (probes, want) in cases {
        let table: HashMap<SurfaceId, ChildProbe> = [(MEMBER_SID, probes[0]), (ANCESTOR_SID, probes[1])].into_iter().collect();
        let ev = evaluate_owner(&walk(&[CALLER, MEMBER_PID, ANCESTOR_PID]), &map, |sid| table[&sid]);
        assert_eq!(ev.verdict, OwnerVerdict::Admit(want), "{probes:?}");
        assert_eq!(ev.members.len(), 2, "{probes:?}: every member is recorded");
    }
    let table: HashMap<SurfaceId, ChildProbe> = [(MEMBER_SID, Ended), (ANCESTOR_SID, Running), (5, Unreadable)].into_iter().collect();
    let ev = evaluate_owner(&walk(&[CALLER, MEMBER_PID, ANCESTOR_PID, MID]), &map, |sid| table[&sid]);
    assert_eq!(ev.verdict, veto(ANCESTOR_PID, ANCESTOR_SID));
    assert_eq!(ev.members.len(), 2, "probing stops at the first running member");
}

/// Stable machine strings (S10 records them, S11 puts them into refusals).
#[test]
fn f_reason_strings_are_stable() {
    let probes = [
        (ChildProbe::SurfaceExited, "surface_exited"),
        (ChildProbe::Running, "running"),
        (ChildProbe::Ended, "ended"),
        (ChildProbe::Contended, "contended"),
        (ChildProbe::Unreadable, "unreadable"),
    ];
    for (p, s) in probes {
        assert_eq!(p.as_str(), s);
    }
    let admits = [
        (AdmitReason::NoMember, "no_member"),
        (AdmitReason::SurfaceExited, "surface_exited"),
        (AdmitReason::ChildEnded, "child_ended"),
        (AdmitReason::ChildContended, "child_contended"),
        (AdmitReason::ChildUnreadable, "child_unreadable"),
        (AdmitReason::NotAcquired(NotAcquiredReason::Timeout), "not_acquired_timeout"),
        (AdmitReason::NotAcquired(NotAcquiredReason::ReaderUnavailable), "not_acquired_reader_unavailable"),
    ];
    let mut seen = std::collections::BTreeSet::new();
    for (r, s) in admits {
        assert_eq!(r.as_str(), s);
        assert_eq!(OwnerVerdict::Admit(r).as_str(), s);
        assert!(seen.insert(s), "duplicate admit string {s}");
    }
    assert!(!seen.contains(veto(MEMBER_PID, MEMBER_SID).as_str()));
    assert_eq!(VetoReason::PaneAncestorRunning.as_str(), "pane_ancestor_running");
}
