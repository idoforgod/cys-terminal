//! Tests of the rehearsal runner (M8 · F4 · H2 · S3) and of the replay machinery it shares with
//! the screen (T5b). Every file lives under this test's private folder.

use super::*;
use crate::repair::replay;
use crate::repair::types::TargetState;

/// RULINGS DE6: every test folder lies inside the replay's own scratch base (never HOME).
fn root(name: &str) -> PathBuf {
    let p = scratch_base()
        .join("tests.nosync")
        .join(format!("rehearsal-{name}-{}", replay::random_hex(4)));
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn req(root: &Path, rel: &str) -> CheckRequest {
    CheckRequest {
        entry_rel: rel.into(),
        scratch_root: root.to_path_buf(),
        recorded_input: None,
        deadline: Duration::from_secs(60),
    }
}

/// RULINGS GA4 (c): a process observed from OUTSIDE the runner — identity = pid + start time — has
/// disappeared (within a generous bound that only ends a hang). A runner that merely empties its own
/// report fails this.
fn gone(pid: u32) -> bool {
    let Some(t) = replay::process_identity(pid) else {
        return true;
    };
    let until = std::time::Instant::now() + Duration::from_secs(10);
    while std::time::Instant::now() < until { // wallclock: guard the bound only ends a hang; the verdict is the process's own disappearance
        if replay::process_identity(pid) != Some(t) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

fn put(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

#[test]
fn sh_syntax_pass_and_fail() {
    let r = root("sh");
    put(&r, "bin/ok.sh", "#!/bin/sh\nif true; then echo ok; fi\n");
    put(&r, "bin/bad.sh", "#!/bin/sh\nif true; then echo ok\n");
    assert_eq!(run_check(&req(&r, "bin/ok.sh")).unwrap(), CheckResult::Pass);
    assert!(matches!(
        run_check(&req(&r, "bin/bad.sh")).unwrap(),
        CheckResult::Fail(_)
    ));
    replay::remove_tree(&r).expect("the test's scratch folder is removed");
}

/// The compile check reads the file only: no `__pycache__` is written next to it.
#[test]
fn py_compile_pass_fail_and_writes_no_pycache() {
    let r = root("py");
    put(&r, "bin/ok.py", "def f():\n    return 1\n");
    put(&r, "bin/bad.py", "def f(:\n    return 1\n");
    assert_eq!(run_check(&req(&r, "bin/ok.py")).unwrap(), CheckResult::Pass);
    match run_check(&req(&r, "bin/bad.py")).unwrap() {
        CheckResult::Fail(m) => assert!(m.contains("SyntaxError") || m.contains("실패"), "{m}"),
        other => panic!("expected a failure, got {other:?}"),
    }
    assert!(
        !r.join("bin/__pycache__").exists(),
        "the compile check wrote __pycache__"
    );
    replay::remove_tree(&r).expect("the test's scratch folder is removed");
}

#[test]
fn json_parse_pass_and_fail() {
    let r = root("json");
    put(&r, "x/ok.json", "{\"a\": [1, 2]}\n");
    put(&r, "x/bad.json", "{\"a\": [1, 2}\n");
    assert_eq!(run_check(&req(&r, "x/ok.json")).unwrap(), CheckResult::Pass);
    assert!(matches!(
        run_check(&req(&r, "x/bad.json")).unwrap(),
        CheckResult::Fail(_)
    ));
    replay::remove_tree(&r).expect("the test's scratch folder is removed");
}

/// S3: a file type with no check is not a failure; it is marked "사전 예행 불가". A listed
/// none-prerehearsal row (agents.json) is not rehearsable before approval (D:781) but its JSON is
/// still parsed.
#[test]
fn unknown_type_not_rehearsable_and_agents_json_parsed() {
    let r = root("none");
    put(&r, "README.md", "# x\n");
    assert_eq!(run_check(&req(&r, "README.md")).unwrap(), CheckResult::Pass);
    assert!(!prerehearsal_possible("README.md"));
    assert!(prerehearsal_possible("bin/javis_preflight.py"));
    assert_eq!(
        check_for("agents.json").0,
        crate::repair::boot_critical::CheckKind::NonePrerehearsal
    );
    assert!(!prerehearsal_possible("agents.json"));
    put(&r, "agents.json", "{not json");
    assert!(matches!(
        run_check(&req(&r, "agents.json")).unwrap(),
        CheckResult::Fail(_)
    ));
    put(&r, "agents.json", "{\"agents\": []}");
    assert_eq!(
        run_check(&req(&r, "agents.json")).unwrap(),
        CheckResult::Pass
    );
    replay::remove_tree(&r).expect("the test's scratch folder is removed");
}

/// A rel that is not a plain pack-relative path is never checked (no `..`, no absolute path).
#[test]
fn rel_must_be_plain() {
    let r = root("rel");
    for bad in ["../x.sh", "/etc/passwd", "a/../../x.sh", ""] {
        assert!(
            matches!(run_check(&req(&r, bad)).unwrap(), CheckResult::Fail(_)),
            "{bad}"
        );
    }
    replay::remove_tree(&r).expect("the test's scratch folder is removed");
}

/// The hook replay (D:779): the released capability gate passes against itself; a candidate with a
/// shell syntax error fails by its markers (hooks end with exit 0 on their own errors).
#[test]
fn hook_replay_judged_by_output_and_markers() {
    let r = root("hook");
    let gate = replay::released_text("hooks/role-capability-gate.sh").unwrap();
    let lib = replay::released_text("hooks/_lib.sh").unwrap();
    put(&r, "hooks/_lib.sh", lib);
    put(&r, "hooks/role-capability-gate.sh", gate);
    assert_eq!(
        check_for("hooks/role-capability-gate.sh").0,
        crate::repair::boot_critical::CheckKind::HookReplay
    );
    assert_eq!(
        run_check(&req(&r, "hooks/role-capability-gate.sh")).unwrap(),
        CheckResult::Pass
    );
    // the run happened in a copy: nothing was written into the given layout
    assert!(!r.join("state").exists());
    put(
        &r,
        "hooks/role-capability-gate.sh",
        &format!("{gate}\nif then fi\n"),
    );
    match run_check(&req(&r, "hooks/role-capability-gate.sh")).unwrap() {
        CheckResult::Fail(m) => assert!(
            m.contains("syntax") || m.contains("깨졌") || m.contains("비정상"),
            "{m}"
        ),
        other => panic!("a broken hook passed the replay: {other:?}"),
    }
    replay::remove_tree(&r).expect("the test's scratch folder is removed");
}

/// RULINGS GA1 — the rehearsal's hook replay reads its answer from THE judge: a hook whose standard
/// output or standard error goes past the capture limit (a syntax-error line hidden behind the cut,
/// exit 0) is not observed whole and FAILS. (The old replay read exit 0, found no marker in the kept
/// prefix and passed both.)
#[test]
fn hook_replay_fails_on_output_not_observed_whole() {
    let r = root("ga1-hook");
    let lib = replay::released_text("hooks/_lib.sh").unwrap();
    put(&r, "hooks/_lib.sh", lib);
    let pad = replay::MAX_CAPTURE + 4096;
    for (name, script) in [
        (
            "stdout",
            format!("#!/bin/sh\ncat >/dev/null\nhead -c {pad} /dev/zero | tr '\\000' ' '\nprintf 'x'\nexit 0\n"),
        ),
        (
            "stderr",
            format!("#!/bin/sh\ncat >/dev/null\nhead -c {pad} /dev/zero | tr '\\000' ' ' >&2\necho 'line 9: syntax error: unexpected end of file' >&2\nexit 0\n"),
        ),
    ] {
        put(&r, "hooks/role-capability-gate.sh", &script);
        match run_check(&req(&r, "hooks/role-capability-gate.sh")).unwrap() {
            CheckResult::Fail(m) => assert!(m.contains("끝까지 보지 못했습니다"), "{name}: {m}"),
            other => panic!("{name}: a hook whose output was cut passed the replay: {other:?}"),
        }
    }
    replay::remove_tree(&r).expect("the test's scratch folder is removed");
}

/// A child that outlives the deadline is killed with its whole process group and reaped.
#[test]
fn timeout_kills_and_reaps_the_group() {
    let r = root("timeout");
    let pidfile = r.join("bg.pid");
    let script = format!("sleep 30 & echo $! > '{}'; wait", pidfile.display());
    let env = vec![("PATH".to_string(), "/usr/bin:/bin".to_string())];
    let o = replay::run_child(
        &["/bin/sh".into(), "-c".into(), script],
        &env,
        &r,
        b"",
        Duration::from_millis(500),
    );
    assert!(o.timed_out);
    let bg: u32 = std::fs::read_to_string(&pidfile)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    // RULINGS GA4 (c): observed from outside — the process itself is gone
    assert!(gone(bg), "the background child {bg} survived the kill");
    // a normal end also leaves nothing behind
    let o = replay::run_child(
        &["/bin/sh".into(), "-c".into(), "sleep 30 & exit 0".into()],
        &env,
        &r,
        b"",
        Duration::from_secs(5),
    );
    assert_eq!(o.code, Some(0));
    replay::remove_tree(&r).expect("the test's scratch folder is removed");
}

/// RULINGS CZ4 / DF5: a child whose descendant ESCAPES the process group (`setsid`) and keeps stdin
/// open unread and stdout open after the child itself exits cannot hold the runner: it returns by its
/// deadline plus the bounded drain, says the pipe was held, and reports — and kills — the escaped
/// process by the run's environment marker. (The old runner blocked in the stdin writer's join and in
/// the readers until the descendant's 30 s sleep ended, and saw no leftover outside the group.)
#[test]
fn runner_returns_when_an_escaped_descendant_holds_the_pipes() {
    let r = root("escape");
    let pidfile = r.join("esc.pid");
    let script = format!(
        "/usr/bin/perl -e 'use POSIX; POSIX::setsid(); open(F, \">\", \"{}\"); print F $$; close F; sleep 30' & sleep 0.3; exit 0",
        pidfile.display()
    );
    let env = vec![("PATH".to_string(), "/usr/bin:/bin".to_string())];
    let input = vec![b'x'; 1 << 20]; // far more than a pipe buffer: the writer would block
    let o = replay::run_child(
        &["/bin/sh".into(), "-c".into(), script],
        &env,
        &r,
        &input,
        Duration::from_secs(3),
    );
    let esc: u32 = std::fs::read_to_string(&pidfile)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(o.pipe_held, "the held output pipe is reported: {o:?}");
    assert!(o.leftovers.contains(&esc), "the escaped process {esc} is reported: {o:?}");
    assert!(o.left_something().is_some());
    assert!(gone(esc), "the escaped process {esc} was killed by its exact pid");
    replay::remove_tree(&r).expect("the test's scratch folder is removed");
}

/// RULINGS DF5: a replayed or compared path is read ONCE through a no-follow, non-blocking handle — a
/// FIFO in its place is refused at once (never a hang), a link stays a link, and the bytes, mode and
/// stamp all come from that handle. No second `std::fs::read` of the same path remains.
#[test]
#[cfg(unix)]
fn compared_reads_use_one_nonblocking_handle() {
    let r = root("df5");
    let fifo = r.join("fifo");
    let c = std::ffi::CString::new(fifo.to_string_lossy().as_bytes()).unwrap();
    // SAFETY: mkfifo(3) on a path inside this test's folder.
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
    assert!(replay::read_stamped(&fifo).is_err());
    assert!(replay::LaidSource::read(&fifo).is_err());
    put(&r, "f.sh", "#!/bin/sh\necho hi\n");
    replay::set_mode(&r.join("f.sh"), 0o750);
    let (c, stamp) = replay::read_stamped(&r.join("f.sh")).unwrap();
    assert_eq!(
        c,
        replay::LaidSource::Bytes { bytes: b"#!/bin/sh\necho hi\n".to_vec(), mode: 0o750 }
    );
    assert_eq!(c.state(), TargetState::read(&r.join("f.sh")).unwrap());
    assert!(stamp.contains("ino="), "{stamp}");
    std::os::unix::fs::symlink("f.sh", r.join("l")).unwrap();
    assert_eq!(replay::read_stamped(&r.join("l")).unwrap().0, replay::LaidSource::Link("f.sh".into()));
    let src = include_str!("replay.rs");
    let read_fn = src.split("pub fn read(p: &Path) -> Result<Self, RepairError> {").nth(1).unwrap_or("");
    let read_fn = read_fn.split("\n    }\n").next().unwrap_or("");
    assert!(!read_fn.contains("std::fs::read("), "LaidSource::read opens the path once");
    replay::remove_tree(&r).expect("the test's scratch folder is removed");
}

/// RULINGS DE2 (BL1): a compared item's stamp holds inode, change time and modification time — a
/// mode change alone (no write, the modification time untouched) is a change of the item.
#[test]
fn stamp_holds_inode_ctime_and_mtime() {
    let r = root("de2");
    put(&r, "x", "same bytes\n");
    let p = r.join("x");
    let (_, s1) = replay::read_stamped(&p).unwrap();
    assert!(s1.contains("ino=") && s1.contains("ctime=") && s1.contains("mtime="), "{s1}");
    // the change time moves on a metadata change while the modification time stays
    replay::set_mode(&p, 0o600);
    replay::set_mode(&p, 0o644);
    let (_, s2) = replay::read_stamped(&p).unwrap();
    assert_ne!(s1, s2, "a ctime-only change is a change of the stamp");
    replay::remove_tree(&r).expect("the test's scratch folder is removed");
}

/// RULINGS CZ3 / DE1 / CZ6: the comparison around rehearsal runs — a run that writes a live listed
/// file on every call is refused; one that wrote only in the first call is noted; a repeat that did
/// not complete is refused; nothing written is clean; the first run's value is returned in each case.
#[test]
fn compared_runs_find_a_write_to_a_live_listed_file() {
    let r = root("cz3");
    let lane = r.join("lane").join("pack");
    put(&lane, "hooks/guard.sh", "#!/bin/sh\nexit 0\n");
    let home = r.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let live = lane.join("hooks/guard.sh");
    let write = |p: &Path| {
        let mut t = std::fs::read_to_string(p).unwrap();
        t.push('x');
        std::fs::write(p, t).unwrap();
    };
    let calls = std::cell::Cell::new(0u32);
    let (v, c) = compared_runs(&lane, &home, || {
        calls.set(calls.get() + 1);
        write(&live);
        (calls.get(), true)
    });
    assert_eq!(v, 1, "the first run's value");
    assert!(matches!(c, Comparison::Refused(ref l) if l.contains("guard.sh")), "{c:?}");
    calls.set(0);
    let (_, c) = compared_runs(&lane, &home, || {
        calls.set(calls.get() + 1);
        if calls.get() == 1 {
            write(&live);
        }
        ((), true)
    });
    assert!(matches!(c, Comparison::Noted(_)), "{c:?}");
    calls.set(0);
    let (_, c) = compared_runs(&lane, &home, || {
        calls.set(calls.get() + 1);
        if calls.get() == 1 {
            write(&live);
        }
        ((), calls.get() == 1)
    });
    assert!(matches!(c, Comparison::Refused(_)), "an incomplete repeat: {c:?}");
    let (_, c) = compared_runs(&lane, &home, || ((), true));
    assert_eq!(c, Comparison::Clean);
    replay::remove_tree(&r).expect("the test's scratch folder is removed");
}

/// RULINGS DR3 — on a SUCCESSFUL exit too: a descendant that closed its outputs but lives on (and
/// holds stdin unread) is killed by its exact pid and REPORTED; the runner returns at once. (The old
/// runner killed the group and then saw nothing left — the descendant was never reported.)
#[test]
#[cfg(unix)]
fn descendant_living_on_after_a_successful_exit_is_killed_and_reported() {
    let r = root("dr3-live");
    let pidfile = r.join("bg.pid");
    let script = format!("sleep 30 >/dev/null 2>&1 & echo $! > '{}'; exit 0", pidfile.display());
    let env = vec![("PATH".to_string(), "/usr/bin:/bin".to_string())];
    let input = vec![b'y'; 1 << 20];
    let o = replay::run_child(
        &["/bin/sh".into(), "-c".into(), script],
        &env,
        &r,
        &input,
        Duration::from_secs(10),
    );
    assert_eq!(o.code, Some(0), "{o:?}");
    let bg: u32 = std::fs::read_to_string(&pidfile).unwrap().trim().parse().unwrap();
    assert!(o.leftovers.contains(&bg), "the descendant {bg} is reported: {o:?}");
    assert!(o.left_something().is_some());
    assert!(gone(bg), "the descendant {bg} was killed");
    replay::remove_tree(&r).expect("the test's scratch folder is removed");
}

/// RULINGS DR3 — a child slow to die after the kill (here: the kill withheld by the test seam) does
/// not hold the runner: it returns after the deadline plus the fixed grace, says the child was not
/// reaped and names it among the processes the kill did not reach; a detached thread reaps it later.
#[test]
#[cfg(unix)]
fn child_slow_to_die_is_reported_and_never_waited_for() {
    let r = root("dr3-slow");
    let env = vec![("PATH".to_string(), "/usr/bin:/bin".to_string())];
    let o = replay::run_child_with(
        &["/bin/sh".into(), "-c".into(), "sleep 30; :".into()],
        &env,
        &r,
        b"",
        Duration::from_secs(1),
        replay::KillMode::NoKillForTest,
    );
    assert!(o.timed_out && o.unreaped, "{o:?}");
    assert!(!o.survivors.is_empty(), "{o:?}");
    assert!(o.left_something().is_some());
    for p in &o.survivors {
        // SAFETY: kill(2) of this test's own child processes, by exact pid
        unsafe {
            libc::kill(*p as libc::pid_t, libc::SIGKILL);
        }
    }
    replay::remove_tree(&r).expect("the test's scratch folder is removed");
}

/// RULINGS EJ2 / EL4 — every step is bounded: a child (and its descendant) that writes WITHOUT PAUSE on
/// stdout and stderr cannot keep the pump reading — the runner returns at its deadline, keeps at most
/// the cap and says the output was not observed whole. (The old pump read until the pipe was empty —
/// which it never was — and never looked at the clock.)
#[test]
#[cfg(unix)]
fn continuous_writer_does_not_hold_the_runner() {
    let r = root("el4-flood");
    let env = vec![("PATH".to_string(), "/usr/bin:/bin".to_string())];
    let o = replay::run_child(
        &["/bin/sh".into(), "-c".into(), "yes out & exec yes err 1>&2".into()],
        &env,
        &r,
        b"",
        Duration::from_secs(1),
    );
    assert!(o.timed_out, "{:?}", (o.code, o.signal));
    assert!(o.stdout.len() <= replay::MAX_CAPTURE && o.stderr.len() <= replay::MAX_CAPTURE);
    assert!(!o.stdout_whole(), "a flooded output is not whole");
    assert!(!o.stderr_state.whole());
    replay::remove_tree(&r).expect("the test's scratch folder is removed");
}

/// RULINGS EJ2 / EL4: a pipe that cannot be made non-blocking means the child is not run — killed at
/// once, the runner returns saying so (fail closed).
#[test]
#[cfg(unix)]
fn nonblocking_setup_failure_runs_nothing() {
    let r = root("el4-setup");
    let env = vec![("PATH".to_string(), "/usr/bin:/bin".to_string())];
    replay::FAIL_NONBLOCK_FOR_TEST.with(|f| f.set(true));
    let o = replay::run_child(
        &["/bin/sh".into(), "-c".into(), "sleep 5".into()],
        &env,
        &r,
        b"",
        Duration::from_secs(10),
    );
    replay::FAIL_NONBLOCK_FOR_TEST.with(|f| f.set(false));
    assert!(o.spawn_error.as_deref().is_some_and(|e| e.contains("non-blocking")), "{o:?}");
    assert!(!o.unreaped, "{o:?}");
    replay::remove_tree(&r).expect("the test's scratch folder is removed");
}

/// RULINGS GA4 (a): a child C leaves the group (`setsid`) and is seen; LATER C starts a grandchild G
/// that closes its standard streams, and C exits. G is followed because the sampler asks the
/// children of every process it has seen, at every sample — G is reported and killed, and is gone
/// observed from outside. (The old sampler skipped every process it had seen once, so G was never
/// looked at.)
#[test]
#[cfg(unix)]
fn grandchild_of_an_escaped_child_is_followed() {
    let r = root("ga4-follow");
    let pidfile = r.join("g.pid");
    let script = format!(
        "/usr/bin/perl -e 'use POSIX; POSIX::setsid(); sleep 1; my $g = fork(); if ($g == 0) {{ close STDIN; close STDOUT; close STDERR; sleep 30; exit 0 }} open(F, \">\", \"{}\"); print F $g; close F; sleep 1; exit 0' & sleep 3; exit 0",
        pidfile.display()
    );
    let env = vec![("PATH".to_string(), "/usr/bin:/bin".to_string())];
    let o = replay::run_child(
        &["/bin/sh".into(), "-c".into(), script],
        &env,
        &r,
        b"",
        Duration::from_secs(20),
    );
    assert_eq!(o.code, Some(0), "{o:?}");
    let g: u32 = std::fs::read_to_string(&pidfile).unwrap().trim().parse().unwrap();
    assert!(o.leftovers.contains(&g), "the grandchild {g} is reported: {o:?}");
    assert!(gone(g), "the grandchild {g} was killed");
    replay::remove_tree(&r).expect("the test's scratch folder is removed");
}

/// RULINGS GA3 — THE completeness of a pass (one function, asked by the screen and the rehearsal):
/// complete only when every planned execution was recorded observed whole; an execution whose output
/// hit the cap or could not be read, a spawn failure, one never started — each leaves it incomplete.
#[test]
fn pass_plan_is_complete_only_when_every_planned_execution_was_observed_whole() {
    let ok = replay::ChildOutcome {
        code: Some(0),
        stdout_state: replay::StreamState { eof: true, dropped: 0, read_error: false },
        stderr_state: replay::StreamState { eof: true, dropped: 0, read_error: false },
        ..Default::default()
    };
    assert!(replay::observed_whole(&ok));
    let mut capped = ok.clone();
    capped.stdout_state.dropped = 1;
    let mut unread = ok.clone();
    unread.stderr_state.read_error = true;
    let spawn = replay::ChildOutcome { spawn_error: Some("x".into()), ..Default::default() };
    let mut late = ok.clone();
    late.timed_out = true;
    for o in [&capped, &unread, &spawn, &late] {
        assert!(!replay::observed_whole(o), "{o:?}");
    }
    let plan = replay::PassPlan::default();
    plan.plan(2);
    plan.record(true);
    assert!(!plan.complete(), "one planned execution never ended");
    plan.record(true);
    assert!(plan.complete());
    let plan = replay::PassPlan::default();
    plan.plan(2);
    plan.record(true);
    plan.record(replay::observed_whole(&capped));
    assert!(!plan.complete(), "an execution not observed whole");
}

/// RULINGS GA9: a pipe that answers "interrupted" for ever cannot keep the pump in its turn — every
/// attempt counts against the turn's budget and the pump returns to the outer deadline check. (The
/// old pump retried an interrupted read without counting it: this test never returned there.)
#[test]
#[cfg(unix)]
fn interrupted_reads_return_to_the_deadline() {
    struct Interrupted(usize);
    impl std::io::Read for Interrupted {
        fn read(&mut self, _b: &mut [u8]) -> std::io::Result<usize> {
            self.0 += 1;
            Err(std::io::Error::from(std::io::ErrorKind::Interrupted))
        }
    }
    let mut p = Some(Interrupted(0));
    let mut buf = Vec::new();
    let mut st = replay::StreamState::default();
    replay::pump_out(&mut p, &mut buf, &mut st);
    let attempts = p.as_ref().map_or(0, |r| r.0);
    assert!(attempts > 0 && attempts <= 64, "{attempts} attempts in one turn");
    assert!(p.is_some() && !st.eof && buf.is_empty(), "still open, nothing read");
}

/// RULINGS CZ4: a cleanup error or a leftover scratch tree is RETURNED, never swallowed.
#[test]
fn cleanup_errors_are_returned() {
    let r = root("cleanup");
    let locked = r.join("locked");
    std::fs::create_dir_all(locked.join("inner")).unwrap();
    std::fs::write(locked.join("inner").join("f"), b"x").unwrap();
    // a parent the walk cannot write: its child entries cannot be removed
    replay::set_mode(&r, 0o500);
    let e = replay::remove_tree(&locked);
    replay::set_mode(&r, 0o700);
    assert!(e.is_err(), "a tree that could not be removed is an error");
    assert!(replay::remove_tree(&r.join("never-there")).is_ok());
    replay::remove_tree(&r).expect("the test's scratch folder is removed");
}

/// The replay environment is the allow-list only: nothing of this process's environment leaks in.
#[test]
fn replay_env_is_allow_list_only() {
    let r = root("env");
    let env = replay::base_env(&r.join("h"), &r.join("t"), &[r.join("bin")]);
    let keys: Vec<&str> = env.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(
        keys,
        vec![
            "HOME",
            "TMPDIR",
            "PATH",
            "USER",
            "LOGNAME",
            "SHELL",
            "LANG",
            "LC_ALL",
            concat!("PYTHONDONT", "WRITEBYTECODE")
        ]
    );
    let path = &env.iter().find(|(k, _)| k == "PATH").unwrap().1;
    assert!(path.starts_with(&*r.join("bin").to_string_lossy()));
    for d in path.split(':').skip(1) {
        assert!(
            ["/usr/bin", "/bin", "/usr/sbin", "/sbin"].contains(&d),
            "{d}"
        );
    }
    let o = replay::run_child(
        &["/usr/bin/env".into()],
        &env,
        &r,
        b"",
        Duration::from_secs(5),
    );
    let out = String::from_utf8_lossy(&o.stdout);
    for v in [
        "CYS_SOCKET",
        "CYS_PACK_DIR",
        "CLAUDE_CONFIG_DIR",
        "CYS_SURFACE_ID",
        "CARGO",
        "RUST",
    ] {
        assert!(
            !out.lines().any(|l| l.starts_with(v)),
            "{v} leaked into the replay: {out}"
        );
    }
    replay::remove_tree(&r).expect("the test's scratch folder is removed");
}

/// The stub cys answers like a daemon-less binary when its socket is not a socket, logs every call,
/// and gives the fixed replies otherwise (C4 E-1 … E-3, I3).
#[test]
fn stub_cys_answers_and_logs() {
    let r = root("stub");
    let bin = r.join("bin");
    replay::write_stub(&bin, "reviewer-codex", true).unwrap();
    let sock = replay::ScratchSocket::bind(&r).unwrap();
    let env = |s: &Path| {
        vec![
            ("PATH".to_string(), "/usr/bin:/bin".to_string()),
            ("CYS_SOCKET".to_string(), s.to_string_lossy().into_owned()),
        ]
    };
    let stub = bin.join("cys").to_string_lossy().into_owned();
    let o = replay::run_child(
        &[stub.clone(), "surface-role".into()],
        &env(sock.path()),
        &r,
        b"",
        Duration::from_secs(5),
    );
    assert_eq!(
        (
            o.code,
            String::from_utf8_lossy(&o.stdout).trim().to_string()
        ),
        (Some(0), "reviewer-codex".to_string())
    );
    let o = replay::run_child(
        &[stub.clone(), "status".into(), "--json".into()],
        &env(sock.path()),
        &r,
        b"",
        Duration::from_secs(5),
    );
    assert_eq!(o.code, Some(0));
    assert!(String::from_utf8_lossy(&o.stdout).contains("\"alert_route\": {\"enabled\": true}"));
    let o = replay::run_child(
        &[
            stub.clone(),
            "--socket".into(),
            sock.path().to_string_lossy().into_owned(),
            "approval".into(),
            "check".into(),
        ],
        &env(Path::new("/nonexistent")),
        &r,
        b"",
        Duration::from_secs(5),
    );
    assert_eq!(o.code, Some(1), "approval check is 'not signed'");
    let o = replay::run_child(
        &[stub, "surface-role".into()],
        &env(&r.join("nosock")),
        &r,
        b"",
        Duration::from_secs(5),
    );
    assert_eq!(
        o.code,
        Some(1),
        "no socket → like the binary without a daemon"
    );
    assert_eq!(replay::stub_calls(&bin).len(), 4);
    drop(sock);
    replay::remove_tree(&r).expect("the test's scratch folder is removed");
}

/// RULINGS H2: every kind has a built-in known-good fixture that passes its own check.
#[test]
fn reference_fixtures_pass() {
    use crate::repair::boot_critical::CheckKind::*;
    for k in [
        ShSyntax,
        PyCompile,
        JsonParse,
        SelfTest,
        HookReplay,
        NonePrerehearsal,
    ] {
        assert!(reference_fixture(k).is_ok());
        assert_eq!(
            run_reference_check(k, Duration::from_secs(60)).unwrap(),
            CheckResult::Pass,
            "{k:?}"
        );
    }
}
