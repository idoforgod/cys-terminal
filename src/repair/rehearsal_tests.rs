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
    std::thread::sleep(Duration::from_millis(200));
    assert!(
        !replay::pid_alive(bg),
        "the background child {bg} survived the kill"
    );
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
    let t0 = std::time::Instant::now();
    let o = replay::run_child(
        &["/bin/sh".into(), "-c".into(), script],
        &env,
        &r,
        &input,
        Duration::from_secs(3),
    );
    let took = t0.elapsed();
    assert!(took < Duration::from_secs(6), "the runner was held for {took:?}");
    let esc: u32 = std::fs::read_to_string(&pidfile)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(o.pipe_held, "the held output pipe is reported: {o:?}");
    assert!(o.leftovers.contains(&esc), "the escaped process {esc} is reported: {o:?}");
    assert!(o.left_something().is_some());
    std::thread::sleep(Duration::from_millis(200));
    assert!(!replay::pid_alive(esc), "the escaped process {esc} was killed by its exact pid");
    replay::remove_tree(&r).expect("the test's scratch folder is removed");
}

/// RULINGS DF5: a replayed or compared path is read ONCE through a no-follow, non-blocking handle — a
/// FIFO in its place is refused at once (never a hang), a link stays a link, and the bytes, mode and
/// stamp all come from that handle. No second `std::fs::read` of the same path remains.
#[test]
fn compared_reads_use_one_nonblocking_handle() {
    let r = root("df5");
    let fifo = r.join("fifo");
    let c = std::ffi::CString::new(fifo.to_string_lossy().as_bytes()).unwrap();
    // SAFETY: mkfifo(3) on a path inside this test's folder.
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
    let t0 = std::time::Instant::now();
    assert!(replay::read_stamped(&fifo).is_err());
    assert!(replay::LaidSource::read(&fifo).is_err());
    assert!(t0.elapsed() < Duration::from_secs(2));
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
    std::thread::sleep(Duration::from_millis(20));
    let (mode_before, _) = (std::fs::metadata(&p).unwrap(), ());
    let _ = mode_before;
    replay::set_mode(&p, 0o600);
    replay::set_mode(&p, 0o644);
    let (_, s2) = replay::read_stamped(&p).unwrap();
    assert_ne!(s1, s2, "a ctime-only change is a change of the stamp");
    replay::remove_tree(&r).expect("the test's scratch folder is removed");
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
