//! Tests of the rehearsal runner (M8 · F4 · H2 · S3) and of the replay machinery it shares with
//! the screen (T5b). Every file lives under this test's private folder.

use super::*;
use crate::repair::replay;

fn root(name: &str) -> PathBuf {
    let home = dirs::home_dir().expect("HOME");
    let p = home
        .join("t5b-rehearsal")
        .join(format!("{name}-{}", replay::random_hex(4)));
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
    replay::remove_tree(&r);
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
    replay::remove_tree(&r);
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
    replay::remove_tree(&r);
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
    replay::remove_tree(&r);
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
    replay::remove_tree(&r);
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
    replay::remove_tree(&r);
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
    replay::remove_tree(&r);
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
    replay::remove_tree(&r);
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
    replay::remove_tree(&r);
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
