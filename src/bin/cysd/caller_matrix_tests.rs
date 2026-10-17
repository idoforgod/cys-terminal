// Included INSIDE handlers::tests: no diagnostic RPC or production instrumentation.
#[cfg(unix)]
mod c8_caller_matrix {
    use super::*;
    use crate::state::{Surface, WriteReq};
    use std::fs::{self, OpenOptions};
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

    fn quoted(text: &str) -> String {
        format!("'{}'", text.replace('\'', "'\\''"))
    }

    fn wait_for(what: &str, mut f: impl FnMut() -> bool) {
        let end = Instant::now() + Duration::from_secs(15);
        while !f() {
            assert!(Instant::now() < end, "matrix timeout: {what}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn observe_process(pid: Option<u32>) -> Value {
        let Some(pid) = pid else { return Value::Null };
        match cys::process_observation::observe(pid) {
            Ok(p) => json!({"pid": p.identity.pid, "parent": p.identity.parent,
                "birth_seconds": p.identity.birth_seconds,
                "birth_microseconds": p.identity.birth_microseconds}),
            Err(e) => json!({"pid": pid, "error": format!("{e:?}")}),
        }
    }

    fn same_incarnation(a: &Value, b: &Value) -> bool {
        // Parent is an observation, not process identity: a surviving child is reparented when
        // its shell dies. Comparing the whole observation would falsely call that child gone.
        ["pid", "birth_seconds", "birth_microseconds"].iter()
            .all(|k| a[*k].as_u64().is_some() && a[*k] == b[*k])
    }

    fn incarnation_ended(after: &Value, before: &Value) -> bool {
        // Unknown/malformed/native observation errors are not death observations.
        after["pid"].as_u64().is_some() && after["pid"] == before["pid"]
            && (after["error"] == "Dead" || (["birth_seconds", "birth_microseconds"].iter()
                .all(|k| after[*k].as_u64().is_some()) && !same_incarnation(after, before)))
    }

    fn key_json(key: Option<crate::submit_assoc::CallerKey>) -> Value {
        use crate::submit_assoc::CallerKey;
        match key {
            Some(CallerKey::Surface(sid)) => json!({"grade":"verified_surface", "surface":sid}),
            Some(CallerKey::Claimed(sid)) => json!({"grade":"claimed", "surface":sid}),
            Some(CallerKey::Process {parent_pid, parent_birth_us}) =>
                json!({"grade":"kernel_parent", "pid":parent_pid, "birth_us":parent_birth_us}),
            None => json!({"grade":"unresolved"}),
        }
    }

    fn independent_match(left: &Value, right: &Value) -> bool {
        let supported = match left["grade"].as_str() {
            Some("verified_surface") => left["surface"].as_u64().is_some(),
            Some("kernel_parent") => left["pid"].as_u64().is_some_and(|p| p > 0)
                && left["birth_us"].as_u64().is_some_and(|b| b > 0),
            _ => false,
        };
        supported && left == right
    }

    #[test]
    fn missing_or_claimed_identity_is_never_an_independent_match() {
        for key in [json!({"grade":"unresolved"}), json!({"grade":"claimed","surface":1}),
            json!({"grade":"kernel_parent","pid":1}), json!({"grade":"verified_surface"})] {
            assert!(!independent_match(&key, &key), "unknown equality is not evidence: {key}");
        }
        let prior = json!({"pid":42,"parent":1,"birth_seconds":123,"birth_microseconds":4});
        assert!(!incarnation_ended(&json!({"pid":42,"error":"Unknown"}), &prior), "unknown is not process death");
        assert!(!incarnation_ended(&json!({"pid":999,"error":"Dead"}), &prior), "another pid is not evidence of this child exit");
        assert!(!incarnation_ended(&json!({"pid":999,"birth_seconds":124,"birth_microseconds":4}), &prior));
        assert!(!incarnation_ended(&json!({"pid":42,"parent":999,"birth_seconds":123,"birth_microseconds":4}), &prior));
        assert!(incarnation_ended(&json!({"pid":42,"error":"Dead"}), &prior));
        assert!(incarnation_ended(&json!({"pid":42,"parent":999,"birth_seconds":124,"birth_microseconds":4}), &prior));
        let kernel = json!({"grade":"kernel_parent","pid":42,"birth_us":123});
        assert!(independent_match(&kernel, &kernel));
        assert!(!independent_match(&kernel, &json!({"grade":"kernel_parent","pid":42,"birth_us":124})));
    }

    struct Rig {
        daemon: Arc<Daemon>,
        root: PathBuf,
        peers: Vec<(Arc<Surface>, Value)>,
        observations: Arc<Mutex<Vec<Value>>>,
        stop: Option<tokio::sync::oneshot::Sender<()>>,
        server: Option<std::thread::JoinHandle<()>>,
        _env: Vec<cys::pack::EnvGuard>,
    }

    impl Rig {
        fn new() -> Self {
            let base = PathBuf::from(std::env::var("C8_CALLER_MATRIX_ROOT").expect("explicit private root required"))
                .canonicalize().unwrap();
            assert!(base.is_dir());
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let root = base.join(format!("run-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            fs::create_dir(&root).expect("new root only: never reuse state/evidence");
            let home = root.join("home");
            fs::create_dir(&home).unwrap();
            let pack = root.join("pack");
            fs::create_dir(&pack).unwrap();
            fs::write(pack.join("acl.json"), r#"{"default":"allow","rules":[]}"#).unwrap();
            let socket = root.join("c.sock");
            assert!(socket.as_os_str().len() < 100, "short isolated socket required");
            let env = vec![
                cys::pack::EnvGuard::set("HOME", home.to_str().unwrap()),
                cys::pack::EnvGuard::set("ZDOTDIR", home.to_str().unwrap()),
                cys::pack::EnvGuard::set("CYS_SHELL", "/bin/bash"),
                cys::pack::EnvGuard::set("CYS_PACK_DIR", pack.to_str().unwrap()),
                cys::pack::EnvGuard::set("CYS_STATE_DIR", root.to_str().unwrap()),
                cys::pack::EnvGuard::set("CYS_SOCKET", socket.to_str().unwrap()),
                cys::pack::EnvGuard::set("CYS_NO_AUTOSTART", "1"),
                cys::pack::EnvGuard::set("PYTHONDONTWRITEBYTECODE", "1"),
            ];
            let daemon = Daemon::new(socket.clone());
            let observations = Arc::new(Mutex::new(Vec::new()));
            let (stop, stopped) = tokio::sync::oneshot::channel();
            let (ready, wait_ready) = std::sync::mpsc::sync_channel(1);
            let d = daemon.clone();
            let logs = observations.clone();
            let log_path = root.join("rpc.jsonl");
            let server = std::thread::spawn(move || {
                tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async move {
                    let listener = tokio::net::UnixListener::bind(socket).unwrap();
                    ready.send(()).unwrap();
                    tokio::pin!(stopped);
                    loop {
                        tokio::select! {
                            _ = &mut stopped => break,
                            accepted = listener.accept() => {
                                let (stream, _) = accepted.unwrap();
                                // The product's OS peer lookup, on the CLI's own socket (no proxy).
                                let pid = crate::peer_pid(&stream);
                                let d = d.clone(); let logs = logs.clone(); let path = log_path.clone();
                                tokio::spawn(async move {
                                    let (read, mut write) = stream.into_split();
                                    let mut reader = tokio::io::BufReader::new(read);
                                    loop {
                                        let mut line = String::new();
                                        let n = tokio::time::timeout(Duration::from_secs(10), reader.read_line(&mut line)).await;
                                        if !matches!(n, Ok(Ok(n)) if n > 0) { break; }
                                        let req: Request = serde_json::from_str(&line).unwrap();
                                        let method = req.method.clone();
                                        let measured = matches!(method.as_str(), "surface.send_text" | "surface.send_key");
                                        let peer = observe_process(pid);
                                        let parent = observe_process(peer["parent"].as_u64().map(|p| p as u32));
                                        let verified = pid.and_then(|p| resolve_caller_surface(&d, p));
                                        let historic = key_json(submit_caller_key(&d, verified, pid, &req.params));
                                        // Capture available independent evidence even when historical I9 chooses Claimed first.
                                        let independent = if let Some(sid) = verified {
                                            json!({"grade":"verified_surface", "surface":sid})
                                        } else if let (Some(p), Some(s), Some(us)) = (
                                            parent["pid"].as_u64(), parent["birth_seconds"].as_u64(), parent["birth_microseconds"].as_u64()) {
                                            json!({"grade":"kernel_parent", "pid":p, "birth_us":s.saturating_mul(1_000_000)+us})
                                        } else { json!({"grade":"unresolved"}) };
                                        let params = json!({"surface_id":req.params["surface_id"],
                                            "from":req.params["from"], "text":req.params["text"], "key":req.params["key"]});
                                        crate::agent_write::set_test_sync_settle(true);
                                        // Real ACL, association, writer and settlement. This rig's framing is NOT
                                        // coverage of production connection admission/receipt clocks.
                                        let Reply::Single(response) = dispatch(&d, req, pid) else { panic!("unexpected stream request") };
                                        if measured {
                                            let row = json!({"method":method, "params":params, "peer":peer, "parent":parent,
                                                "historical_key":historic, "independent_key":independent, "response":response});
                                            let mut file = OpenOptions::new().create(true).append(true).open(&path).unwrap();
                                            writeln!(file, "{row}").unwrap(); file.sync_all().unwrap();
                                            logs.lock().unwrap().push(row);
                                        }
                                        if write.write_all(format!("{response}\n").as_bytes()).await.is_err() { break; }
                                    }
                                });
                            }
                        }
                    }
                });
            });
            wait_ready.recv_timeout(Duration::from_secs(10)).unwrap();
            Self { daemon, root, peers:vec![], observations, stop:Some(stop), server:Some(server), _env:env }
        }

        fn shell(&mut self, role: &str, registered: bool) -> Arc<Surface> {
            let s = self.daemon.create_surface(Some(self.root.to_string_lossy().into()),
                Some("exec /bin/bash --noprofile --norc -i".into()), None, Some(role.into()), 24, 240).unwrap();
            self.own(&s);
            // create_surface already publishes. Remove this OWNED fixture root to model a caller
            // outside the receiving daemon, and prove the population before issuing any request.
            if !registered {
                assert!(self.daemon.surfaces.lock().unwrap().remove(&s.id).is_some());
                assert_eq!(self.daemon.roles.lock().unwrap().remove(role), Some(s.id));
                assert_eq!(walk_caller_ancestry(&self.daemon, s.pid).0, None);
            } else {
                assert_eq!(walk_caller_ancestry(&self.daemon, s.pid).0, Some(s.id));
            }
            self.command(&s, &format!("shell-{}-job-control", s.id), "set +m");
            s
        }

        fn own(&mut self, s: &Arc<Surface>) {
            // portable-pty gives this owned child its own session/group. Refuse a fixture that
            // would make group cleanup touch the test runner, and retain native incarnation.
            let identity = observe_process(Some(s.pid));
            // Own the actual Child BEFORE any assertion can unwind. A failed group proof
            // is not authority to signal that group, but the Child handle remains ours.
            self.peers.push((s.clone(), identity.clone()));
            assert!(s.pid > 1 && s.pid != std::process::id());
            assert_eq!(unsafe { libc::getpgid(s.pid as libc::pid_t) }, s.pid as libc::pid_t);
            assert_eq!(unsafe { libc::getsid(s.pid as libc::pid_t) }, s.pid as libc::pid_t);
            assert!(identity["birth_seconds"].as_u64().is_some());
        }

        fn command(&self, shell: &Surface, tag: &str, command: &str) {
            let done = self.root.join(format!("{tag}.done"));
            assert!(!done.exists());
            let text = format!("{command}; matrix_rc=$?; printf '%s' \"$matrix_rc\" > {}\n", quoted(done.to_str().unwrap()));
            shell.write_tx.try_send(WriteReq::Data(text.into_bytes())).unwrap();
            wait_for(tag, || done.exists() && fs::metadata(&done).unwrap().len() > 0);
            assert_eq!(fs::read_to_string(done).unwrap(), "0", "CLI command failed: {tag}");
        }

        fn recorder(&mut self, tag: &str) -> (Arc<Surface>, PathBuf) {
            let out = self.root.join(format!("{tag}.bin"));
            let cmd = format!("stty raw -echo; exec cat > {}", quoted(out.to_str().unwrap()));
            let s = self.daemon.create_surface(Some(self.root.to_string_lossy().into()), Some(cmd), None,
                Some(tag.into()), 24, 240).unwrap();
            self.own(&s);
            self.daemon.surfaces.lock().unwrap().insert(s.id, s.clone());
            self.daemon.roles.lock().unwrap().insert(tag.into(), s.id);
            *s.agent_meta.lock().unwrap() = Some(("claude".into(), "claude".into()));
            wait_for("recorder ready", || out.exists());
            (s, out)
        }

        fn positive(&self, tag: &str, target: &Surface, out: &Path, body: &str, begin: usize) -> Value {
            let logs = self.observations.lock().unwrap();
            let rows: Vec<_> = logs[begin..].iter().filter(|v| v["params"]["surface_id"] == json!(target.id)).cloned().collect();
            assert_eq!(rows.len(), 2, "one body and one Return, no resend/flood: {tag}: {rows:?}");
            assert_eq!(rows[0]["method"], "surface.send_text");
            assert_eq!(rows[1]["method"], "surface.send_key");
            assert_eq!(rows[0]["response"]["result"]["held"], "awaiting_submit", "{tag}: {rows:?}");
            assert_eq!(rows[1]["response"]["result"]["bound"], "legacy", "{tag}: {rows:?}");
            assert_eq!(rows[0]["response"]["result"]["request_id"], rows[1]["response"]["result"]["request_id"]);
            assert!(!target.bracketed_paste.load(Ordering::Relaxed), "raw recorder does not enable paste mode");
            let want = format!("{body}\r").into_bytes();
            wait_for("exact receiver bytes", || fs::read(out).unwrap() == want);
            assert_eq!(target.submit_assoc.lock().unwrap().len(), 0);
            json!({"flow":tag, "rpc":rows, "received_bytes":want,
                "independent_match":independent_match(&rows[0]["independent_key"], &rows[1]["independent_key"]),
                "historical_match":rows[0]["historical_key"]==rows[1]["historical_key"]})
        }
    }

    impl Drop for Rig {
        fn drop(&mut self) {
            let mut cleanup = Vec::new();
            for (peer, identity) in &self.peers {
                let current = observe_process(Some(peer.pid));
                let group = unsafe { libc::getpgid(peer.pid as libc::pid_t) };
                let same_owned_group = peer.pid > 1 && peer.pid != std::process::id()
                    && same_incarnation(&current, identity) && group == peer.pid as libc::pid_t;
                // Interactive fixture job control is disabled before commands. Its hook/helper
                // descendants therefore share this owned group even if a failed run leaves them
                // running. Never signal a recycled or unverifiable identity.
                let mut child = peer.child.lock().unwrap_or_else(|e| e.into_inner());
                let rc = if same_owned_group { unsafe { libc::kill(-group, libc::SIGKILL) } }
                    else if child.kill().is_ok() { 0 } else { -1 };
                let deadline = Instant::now() + Duration::from_secs(5);
                let ended = loop {
                    match child.try_wait() {
                        Ok(Some(_)) => break true,
                        Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
                        _ => break false,
                    }
                };
                cleanup.push(json!({"pid":peer.pid,"identity":identity,"same_owned_group":same_owned_group,
                    "signal_rc":rc,"root_reaped":ended}));
            }
            // Cleanup is also recorded on assertion failure; do not panic while already unwinding.
            let _ = fs::write(self.root.join("cleanup.json"), serde_json::to_vec_pretty(&cleanup).unwrap());
            if let Some(stop) = self.stop.take() { let _ = stop.send(()); }
            if let Some(server) = self.server.take() { server.join().unwrap(); }
        }
    }

    /// HEAD-RULING-C8-SUBMIT-CALLER: opt-in actual-CLI/peer matrix BEFORE enforcing legacy identity.
    /// POSIX fixture, fake target agent metadata + pass-through receiver proof, real bytes/ACL/association.
    /// No claim about vendor hooks, production dispatch timing, native receiver proof, or Windows.
    #[test]
    #[ignore = "requires explicit isolated C8_CALLER_MATRIX_ROOT and built C8_CALLER_CLI; POSIX measurement"]
    fn actual_cli_caller_compatibility_matrix() {
        let _lock = crate::governance::PACK_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let cli = PathBuf::from(std::env::var("C8_CALLER_CLI").expect("explicit built CLI required")).canonicalize().unwrap();
        assert!(cli.is_file());
        let mut rig = Rig::new();
        let master = rig.shell("master", true);
        let worker = rig.shell("worker", true);
        let outside = rig.shell("outside", false);
        let foreign = rig.shell("foreign", false);
        rig.command(&outside, "outside-env", "unset CYS_SURFACE_ID CYS_SURFACE_REF CYS_ROLE CYS_SEAT_TOKEN");
        let c = quoted(cli.to_str().unwrap());
        let mut matrix = Vec::new();
        for tag in ["seat", "hook", "pack", "outside", "file", "foreign-shell", "foreign-pack"] {
            let (target, out) = rig.recorder(tag);
            let body = format!("matrix {tag}");
            let body_cmd = format!("{c} send --surface {} {}", target.id, quoted(&body));
            let ret_cmd = format!("{c} send-key --surface {} Return", target.id);
            let begin = rig.observations.lock().unwrap().len();
            match tag {
                "seat" | "outside" | "foreign-shell" => {
                    let shell = match tag { "seat" => &master, "outside" => &outside, _ => &foreign };
                    rig.command(shell, &format!("{tag}-body"), &body_cmd);
                    assert!(fs::read(&out).unwrap().is_empty(), "held body is not typed");
                    rig.command(shell, &format!("{tag}-return"), &ret_cmd);
                }
                "hook" => {
                    let hook = rig.root.join("hook.sh");
                    fs::write(&hook, format!("#!/bin/bash\nset -e\n{body_cmd}\n{ret_cmd}\n")).unwrap();
                    rig.command(&worker, "hook-pair", &format!("/bin/bash {}", quoted(hook.to_str().unwrap())));
                }
                "pack" | "foreign-pack" => {
                    let helper = Path::new(env!("CARGO_MANIFEST_DIR")).join("cysjavis-pack/bin/javis_cycle_verifier.py");
                    let script = rig.root.join(format!("{tag}.py"));
                    fs::write(&script, format!("import importlib.util\ns=importlib.util.spec_from_file_location('matrix_helper', {helper:?})\nm=importlib.util.module_from_spec(s)\ns.loader.exec_module(m)\nm.CYS={cli:?}\nok,why=m.push_line({tag:?}, {body:?})\nassert ok,why\n", helper=helper.to_str().unwrap(),cli=cli.to_str().unwrap())).unwrap();
                    let shell = if tag=="pack" {&worker} else {&foreign};
                    rig.command(shell, &format!("{tag}-pair"), &format!("/usr/bin/python3 {}", quoted(script.to_str().unwrap())));
                }
                "file" => {
                    let file = rig.root.join("body.txt"); fs::write(&file, &body).unwrap();
                    rig.command(&worker, "file-body", &format!("{c} send --surface {} --file {}",target.id,quoted(file.to_str().unwrap())));
                    assert!(fs::read(&out).unwrap().is_empty());
                    rig.command(&worker, "file-return", &ret_cmd);
                }
                _ => unreachable!(),
            }
            matrix.push(rig.positive(tag, &target, &out, &body, begin));
        }
        let (target, out) = rig.recorder("negative");
        let begin = rig.observations.lock().unwrap().len();
        rig.command(&master, "negative-body", &format!("{c} send --surface {} 'original actor'", target.id));
        rig.command(&worker, "negative-other-return", &format!("CYS_SURFACE_ID={} {c} send-key --surface {} Return",master.id,target.id));
        let rows = rig.observations.lock().unwrap()[begin..].to_vec();
        assert_eq!(rows.len(), 2);
        assert_ne!(rows[0]["independent_key"], rows[1]["independent_key"]);
        assert!(rows[1]["response"]["result"].get("bound").is_none(), "different actor must not bind");
        assert_eq!(target.submit_assoc.lock().unwrap().len(), 1, "original body retained");
        // C5's unbound Return can still be AgentControl. Record it; do not claim zero I/O.
        wait_for("unbound Return's current AgentControl byte", || fs::read(&out).unwrap()==b"\r");
        let before_original = fs::read(&out).unwrap();
        assert!(!String::from_utf8_lossy(&before_original).contains("original actor"));
        // The unrelated control Return starts S21's settling hold. This matrix tests identity,
        // so wait for that existing 300ms guard before asking the ORIGINAL actor to submit.
        // Otherwise the CLI correctly queues its retry and this fixture has no watchdog consumer.
        std::thread::sleep(Duration::from_millis(300));
        rig.command(&master, "negative-own-return", &format!("{c} send-key --surface {} Return",target.id));
        let final_rpc = rig.observations.lock().unwrap().last().unwrap().clone();
        assert_eq!(final_rpc["response"]["result"]["bound"], "legacy");
        assert_eq!(final_rpc["response"]["result"]["request_id"], rows[0]["response"]["result"]["request_id"]);
        let mut expected = before_original.clone(); expected.extend_from_slice(b"original actor\r");
        wait_for("original actor completes exactly once", || fs::read(&out).unwrap()==expected);
        assert_eq!(target.submit_assoc.lock().unwrap().len(),0);
        fs::write(rig.root.join("matrix.json"), serde_json::to_vec_pretty(&json!({
            "platform":std::env::consts::OS, "positive_rows":matrix,
            "different_actor":{"rpc":rows,"bytes_before_original_return":before_original,"final_bytes":fs::read(&out).unwrap()},
            "limits":["fixture target metadata/pass-through proof, not vendor receiver",
                "hook-shaped bash subprocess, not vendor hook invocation", "fixture socket framing, not dispatch-gate timing",
                "unregistered owned PTYs simulate outside/foreign callers, no live peers", "Windows unmeasured"]
        })).unwrap()).unwrap();
        println!("C8_CALLER_MATRIX={}", rig.root.display());
        assert!(matrix.iter().all(|r| r["independent_match"]==true), "normal pair independent identities differ; preserve measurement, do not tighten");
        let root = rig.root.clone();
        drop(rig);
        assert_cleanup(&root);
    }

    fn assert_cleanup(root: &Path) {
        let rows: Vec<Value> = serde_json::from_slice(&fs::read(root.join("cleanup.json")).unwrap()).unwrap();
        assert!(!rows.is_empty());
        for row in rows {
            assert_eq!(row["same_owned_group"], true, "{row}");
            assert_eq!(row["signal_rc"], 0, "{row}");
            assert_eq!(row["root_reaped"], true, "{row}");
        }
    }

    #[test]
    #[ignore = "requires explicit isolated C8_CALLER_MATRIX_ROOT; POSIX cleanup measurement"]
    fn interrupted_fixture_cleans_the_owned_shell_and_its_running_child() {
        let _lock = crate::governance::PACK_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut rig = Rig::new();
        let shell = rig.shell("worker", true);
        let child_file = rig.root.join("child.pid");
        let ready = rig.root.join("child.ready");
        // Survive the PTY session leader's SIGHUP so a root-only kill cannot accidentally
        // satisfy the descendant assertion. Readiness is AFTER installing the ignored signal.
        let child_command = format!("trap '' HUP; printf ready > {}; exec /bin/sleep 60", quoted(ready.to_str().unwrap()));
        let command = format!("/bin/bash -c {} & printf '%s' $! > {}; wait\n", quoted(&child_command), quoted(child_file.to_str().unwrap()));
        shell.write_tx.try_send(WriteReq::Data(command.into_bytes())).unwrap();
        wait_for("background child started", || fs::read_to_string(&child_file).ok().is_some_and(|s| !s.is_empty()));
        wait_for("child ignores session hangup", || ready.exists());
        let child: u32 = fs::read_to_string(&child_file).unwrap().parse().unwrap();
        let before = observe_process(Some(child));
        assert!(before["birth_seconds"].as_u64().is_some());
        assert_eq!(unsafe { libc::getpgid(child as libc::pid_t) }, shell.pid as libc::pid_t);
        let root = rig.root.clone(); let group = shell.pid as libc::pid_t;
        drop(rig); // simulate early unwind while its command is still waiting for a child
        let until = Instant::now() + Duration::from_secs(2);
        while !incarnation_ended(&observe_process(Some(child)), &before) && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(10));
        }
        let after_drop = observe_process(Some(child));
        let ended = incarnation_ended(&after_drop, &before);
        let survived = same_incarnation(&after_drop, &before);
        // A falsifying mutant must not leave its intentionally surviving specimen behind.
        if survived {
            assert_eq!(unsafe { libc::getpgid(child as libc::pid_t) }, group);
            assert_eq!(unsafe { libc::kill(-group, libc::SIGKILL) }, 0);
            wait_for("mutant specimen cleanup", || incarnation_ended(&observe_process(Some(child)), &before));
        }
        fs::write(root.join("child-cleanup.json"), serde_json::to_vec_pretty(&json!({
            "child":before,"after_drop":after_drop,"ended_by_rig_drop":ended,"survived_rig_drop":survived,"fallback_cleanup_used":survived,
            "after":observe_process(Some(child))})).unwrap()).unwrap();
        assert!(!survived, "owned child survived fixture cleanup");
        assert!(ended, "child cleanup remained unverified; native uncertainty is not death");
        assert_cleanup(&root);
        println!("C8_CALLER_CLEANUP={}", root.display());
    }
}
