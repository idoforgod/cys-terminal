#[cfg(test)]
mod p2_consumer_tests {
    use super::*;
    use std::collections::BTreeMap;

    fn classify(args: &[&str]) -> IdentityConsumer {
        identity_consumer(&Cli::try_parse_from(std::iter::once("cys").chain(args.iter().copied())).unwrap().command)
    }

    #[test]
    fn p2_closed_consumer_contract() {
        let kept: &[(&[&str], IdentityConsumer)] = &[
            (&["identify"], IdentityConsumer::Identify), (&["ping"], IdentityConsumer::Ping),
            (&["list"], IdentityConsumer::List), (&["status"], IdentityConsumer::Status),
            (&["send", "--surface", "7", "body"], IdentityConsumer::SendText),
            (&["send-key", "--surface", "7", "Return"], IdentityConsumer::SendKey),
            (&["set-title", "title"], IdentityConsumer::SetTitle),
            (&["set-status"], IdentityConsumer::SetStatus),
            (&["usage-register", "--transcript", "/tmp/transcript"], IdentityConsumer::UsageRegister),
            (&["usage-event-stdin"], IdentityConsumer::UsageEventStdin),
            (&["watch", "--until", "ready"], IdentityConsumer::Watch),
            (&["attest", "pin"], IdentityConsumer::AttestPin),
            (&["attest", "verify", "1:abc"], IdentityConsumer::AttestVerify),
            (&["read-screen"], IdentityConsumer::ReadScreen),
            (&["quiesce"], IdentityConsumer::Quiesce),
            (&["resize", "--rows", "24", "--cols", "80"], IdentityConsumer::Resize),
            (&["claim-role", "worker"], IdentityConsumer::ClaimRole),
        ];
        for (args, expected) in kept { assert_eq!(classify(args), *expected, "{args:?}"); assert!(expected.permits_inference()); }
        let removed: &[&[&str]] = &[
            &["reinject", "--role", "worker"], &["reinject", "--role", "worker", "--check"],
            &["launch-agent", "--role", "worker", "--agent", "claude"], &["boot"], &["restore"],
            &["node-recover", "--role", "worker"], &["cycle-agent", "--role", "worker"],
            &["usage-report-stdin"], &["usage-report-stdin", "--agy"],
            &["todo-path"], &["pack-update"], &["run", "--", "true"],
            &["send", "--surface", "7", "--file", "/tmp/body"], &["doctor"],
        ];
        for args in removed { assert_eq!(classify(args), IdentityConsumer::EnvironmentOnly, "{args:?}"); }
    }

    #[test]
    fn p2_inferred_access_rejects_mixed_consumers() {
        DISCOVERED_IDENTITY.set(Some(lane_fallback::Binding { socket: "/fixture/dept.sock".into(), surface: Some(9) })).unwrap();
        assert_eq!(inferred_identity(IdentityConsumer::Identify).unwrap().surface, Some(9));
        assert!(inferred_identity(IdentityConsumer::EnvironmentOnly).is_none());
        assert!(!IdentityConsumer::EnvironmentOnly.permits_inference());
    }

    #[test]
    fn p2_excluded_command_diagnostic_changes_no_selection() {
        let base=std::path::Path::new("/base/cys.sock");let dept=std::path::Path::new("/dept/cys.sock");
        let line=excluded_identity_diagnostic(base,dept).unwrap();
        assert_eq!(line.lines().count(),1);
        for token in ["/base/cys.sock","/dept/cys.sock","CYS_SOCKET","CYS_PACK_DIR"] { assert!(line.contains(token)); }
        assert!(excluded_identity_diagnostic(base,base).is_none());
        assert_eq!(cli_socket_path(None,None,||base.into()),base);
    }

    fn census_text(file: &str, src: &str) -> BTreeMap<String,usize> {
        let (masked, tests)=r2r_lex_production(src);
        let text=String::from_utf8(masked).unwrap();let fns=r2r_fn_spans(&text,&tests);
        let symbols=regex::Regex::new(r"\b(inferred_identity|resolved_caller_surface|target_surface|cli_socket_path|DISCOVERED_IDENTITY|ACTIVE_CONSUMER|current_consumer|initialize_lane_identity|identity_consumer|permits_inference)\b").unwrap();
        let mut out=BTreeMap::new();
        for hit in symbols.find_iter(&text) {
            if tests.iter().any(|&(a,b)|a<=hit.start()&&hit.start()<b) {continue;}
            let owner=fns.iter().filter(|(_,a,b)|*a<=hit.start()&&hit.start()<*b).min_by_key(|(_,a,b)|b-a);
            let name=if let Some((name,a,_))=owner {
                let body=text[*a..].find('{').unwrap()+a;
                if hit.start()<body && hit.as_str()==name {continue;}
                name.as_str()
            } else {"<module>"};
            *out.entry(format!("{file}:{name}:{}",hit.as_str())).or_insert(0)+=1;
        }
        out
    }

    fn collect_sources(dir: &std::path::Path, paths: &mut Vec<std::path::PathBuf>) {
        for e in std::fs::read_dir(dir).unwrap() {let p=e.unwrap().path();if p.is_dir(){collect_sources(&p,paths);}else if p.extension().is_some_and(|x|x=="rs"){paths.push(p);}}
    }

    #[test]
    fn p2_inferred_consumer_production_census() {
        let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut paths=vec![root.join("src/bin/cys.rs")];collect_sources(&root.join("src/bin/cys"),&mut paths);paths.sort();
        let mut got=BTreeMap::new();
        for p in paths {got.extend(census_text(p.strip_prefix(root).unwrap().to_str().unwrap(),&std::fs::read_to_string(&p).unwrap()));}
        let expected: BTreeMap<String,usize> = [
            // Audited reader/caller population; new references require consumer review.
            ("src/bin/cys.rs:main:initialize_lane_identity", 1),
            ("src/bin/cys.rs:initialize_lane_identity:identity_consumer", 1),
            ("src/bin/cys.rs:initialize_lane_identity:permits_inference", 2),
            ("src/bin/cys.rs:inferred_identity:permits_inference", 1),
            ("src/bin/cys.rs:resolved_caller_surface:permits_inference", 1),
            ("src/bin/cys.rs:<module>:ACTIVE_CONSUMER", 1),
            ("src/bin/cys.rs:<module>:DISCOVERED_IDENTITY", 1),
            ("src/bin/cys.rs:current_consumer:ACTIVE_CONSUMER", 1),
            ("src/bin/cys.rs:hook_ack_boot_nonce:target_surface", 1),
            ("src/bin/cys.rs:inferred_identity:DISCOVERED_IDENTITY", 1),
            ("src/bin/cys.rs:initialize_lane_identity:ACTIVE_CONSUMER", 1),
            ("src/bin/cys.rs:initialize_lane_identity:DISCOVERED_IDENTITY", 2),
            ("src/bin/cys.rs:resolve_role_or_surface:target_surface", 1),
            ("src/bin/cys.rs:resolve_targets:target_surface", 1),
            ("src/bin/cys.rs:resolved_caller_surface:current_consumer", 1),
            ("src/bin/cys.rs:resolved_caller_surface:inferred_identity", 1),
            ("src/bin/cys.rs:run:target_surface", 9),
            ("src/bin/cys.rs:run_claim_role:target_surface", 1),
            ("src/bin/cys.rs:run_usage_event_stdin:target_surface", 1),
            ("src/bin/cys.rs:run_usage_report_stdin:target_surface", 2),
            ("src/bin/cys.rs:socket_path:cli_socket_path", 1),
            ("src/bin/cys.rs:socket_path:current_consumer", 1),
            ("src/bin/cys.rs:socket_path:inferred_identity", 1),
            ("src/bin/cys.rs:target_surface:resolved_caller_surface", 1),
        ].into_iter().map(|(k,v):(&str,usize)|(k.to_owned(),v)).collect();
        assert_eq!(got,expected,"new inferred-identity reader/caller requires a consumer-contract review");
    }

    #[test]
    fn p2_consumer_census_detects_new_callers_and_aliases_but_excludes_tests() {
        let src=r##"fn added() { let alias = inferred_identity; alias(); target_surface(); }
            #[cfg(test)] mod t { fn hidden() { inferred_identity(); } }
            fn after() { resolved_caller_surface(); } // inferred_identity()
            fn literals() { let x=r#"inferred_identity()"#; }
            "##;
        let got=census_text("fixture.rs",src);
        assert_eq!(got.len(),3,"{got:?}");
        assert_eq!(got["fixture.rs:added:inferred_identity"],1);
        assert_eq!(got["fixture.rs:added:target_surface"],1);
        assert_eq!(got["fixture.rs:after:resolved_caller_surface"],1);
    }

    #[cfg(unix)]

use std::{fs,io::{BufRead,BufReader,Write},sync::{Arc,Mutex,atomic::{AtomicBool,Ordering}},time::Duration};
fn recorder_load() -> String { String::from_utf8_lossy(&std::process::Command::new("/usr/sbin/sysctl").arg("vm.loadavg").output().unwrap().stdout).trim().to_owned() }

    /// The recorder peer's read step: accept one connection and read its one request line. `None` = nothing to
    /// record this turn (no pending connection, or a connection that was not read - the latter is pushed to `errors`).
    /// ★The accepted stream is made BLOCKING before the read: on macOS it inherits O_NONBLOCK from the listener (kept
    /// nonblocking for the stop poll), and a read timeout is inert on a nonblocking socket. Linux does not inherit it.
    /// `before_read` runs once the stream is set up, right before the request line is read (the specimens use it to
    /// order a late write after the accept; the fixture passes a no-op).
    #[cfg(unix)]
    fn recorder_next_request(listener:&std::os::unix::net::UnixListener,read_timeout:Duration,lane:&str,errors:&Mutex<Vec<String>>,before_read:&dyn Fn(&std::os::unix::net::UnixStream))->Option<(std::os::unix::net::UnixStream,Value)> {
     let (stream,_)=match listener.accept(){Ok(x)=>x,Err(e) if e.kind()==std::io::ErrorKind::WouldBlock=>{std::thread::sleep(Duration::from_millis(2));return None},Err(e)=>panic!("{e}")};
     let read=|| -> Result<Value,String> {
      stream.set_nonblocking(false).map_err(|e|format!("set_nonblocking(false): {e}"))?;
      stream.set_read_timeout(Some(read_timeout)).map_err(|e|format!("set_read_timeout: {e}"))?;
      before_read(&stream);
      let mut line=String::new();
      if BufReader::new(&stream).read_line(&mut line).map_err(|e|format!("read_line: {e}"))?==0 {return Err("connection closed before a request line".into());}
      serde_json::from_str(&line).map_err(|e|format!("request is not JSON ({e}): {line:?}"))
     };
     match read() {Ok(req)=>Some((stream,req)),Err(e)=>{errors.lock().unwrap().push(format!("{lane}: {e}"));None}}
    }

    /// ★#16 (0.14.56): a client whose request arrives AFTER the peer accepted must still be read. On macOS the accepted
    /// socket inherits O_NONBLOCK from the nonblocking listener, so `set_read_timeout` alone is inert (read = WouldBlock at
    /// 0 ms) and the request was dropped unread -> the CLI's write met EPIPE (release run 40664016381 attempt 2).
    /// Ordered by events, not by time: the client connects (so the accept below returns at once), and writes only when
    /// the peer's `before_read` says the stream is set up - the request is always written AFTER the accept.
    #[cfg(unix)]
    #[test]
    fn p2_recorder_reads_a_late_writer() {
     use std::os::fd::AsRawFd;
     let sock=std::env::temp_dir().join(format!("p2-late-{}.sock",std::process::id()));let _=fs::remove_file(&sock);
     let listener=std::os::unix::net::UnixListener::bind(&sock).unwrap();listener.set_nonblocking(true).unwrap();
     let path=sock.clone();let (connected_tx,connected_rx)=std::sync::mpsc::channel::<()>();let (go_tx,go_rx)=std::sync::mpsc::channel::<()>();
     let client=std::thread::spawn(move||{let mut s=std::os::unix::net::UnixStream::connect(&path).unwrap();connected_tx.send(()).unwrap();if go_rx.recv().is_ok(){s.write_all(b"{\"id\":1,\"method\":\"system.ping\",\"params\":{}}\n").unwrap();}});
     connected_rx.recv().unwrap();
     let errors=Mutex::new(Vec::new());let blocking_at_read=std::cell::Cell::new(None);
     let got=recorder_next_request(&listener,Duration::from_secs(60),"T",&errors,&|s|{blocking_at_read.set(Some(unsafe{libc::fcntl(s.as_raw_fd(),libc::F_GETFL)}&libc::O_NONBLOCK==0));go_tx.send(()).unwrap();}); // 60 s = hang guard only: the request is written right after the before-read signal
     drop(go_tx);client.join().unwrap();let _=fs::remove_file(&sock);
     assert_eq!(blocking_at_read.get(),Some(true),"the accepted stream was not blocking at the read - on macOS it inherits O_NONBLOCK from the listener (errors {:?})",errors.lock().unwrap());
     assert_eq!(got.as_ref().map(|x|x.1["method"].clone()),Some(json!("system.ping")),"a request written after the peer accepted was not read (errors {:?})",errors.lock().unwrap());
    }

    /// ★#16 (0.14.56, CEO ruling item 2): a connection the peer could not read is REPORTED, not skipped in silence -
    /// otherwise the next fixture fault shows up only as an unrelated EPIPE in the CLI under test.
    /// Two unread shapes, each reported as itself: a client that stays connected and silent until the peer has
    /// returned (the read timeout ends the read), and a client that closes once the peer has set the stream up (EOF
    /// at the read). A client that closed BEFORE the accept is reported too, but on macOS by `set_read_timeout`
    /// (EINVAL, os error 22 - measured), never reaching the read, so it cannot stand for the EOF branch.
    #[cfg(unix)]
    #[test]
    fn p2_recorder_reports_an_unread_connection() {
     let sock=std::env::temp_dir().join(format!("p2-silent-{}.sock",std::process::id()));let _=fs::remove_file(&sock);
     let listener=std::os::unix::net::UnixListener::bind(&sock).unwrap();listener.set_nonblocking(true).unwrap();
     let errors=Mutex::new(Vec::new());
     let path=sock.clone();let (connected_tx,connected_rx)=std::sync::mpsc::channel::<()>();let (done_tx,done_rx)=std::sync::mpsc::channel::<()>();
     let silent=std::thread::spawn(move||{let _s=std::os::unix::net::UnixStream::connect(&path).unwrap();connected_tx.send(()).unwrap();let _=done_rx.recv();});
     connected_rx.recv().unwrap();
     let got_silent=recorder_next_request(&listener,Duration::from_millis(200),"T",&errors,&|_|{});
     drop(done_tx);silent.join().unwrap();
     let path=sock.clone();let (connected_tx,connected_rx)=std::sync::mpsc::channel::<()>();let (close_tx,close_rx)=std::sync::mpsc::channel::<()>();
     let closing=std::thread::spawn(move||{let s=std::os::unix::net::UnixStream::connect(&path).unwrap();connected_tx.send(()).unwrap();let _=close_rx.recv();drop(s);});
     connected_rx.recv().unwrap();
     let got_closed=recorder_next_request(&listener,Duration::from_secs(60),"T",&errors,&|_|{close_tx.send(()).unwrap();}); // 60 s = hang guard only: the client closes right after the before-read signal
     drop(close_tx);closing.join().unwrap();let _=fs::remove_file(&sock);
     let errors=errors.into_inner().unwrap();
     assert!(got_silent.is_none()&&got_closed.is_none(),"an unread client produced a request: {got_silent:?} {got_closed:?}");
     assert_eq!(errors.len(),2,"each unread connection must be reported once: {errors:?}");
     assert!(errors[0].starts_with("T: read_line: "),"a silent client must be reported as a read that timed out, not as EOF: {errors:?}");
     assert_eq!(errors[1],"T: connection closed before a request line","a closed client must be reported as EOF, not as a timeout: {errors:?}");
    }

    #[cfg(unix)]
    fn recorder_fixture(cli: std::path::PathBuf, out: std::path::PathBuf) {
 let root=std::env::temp_dir().join(format!("p2-r1-{}",std::process::id()));
 let base=root.join(".local/state/cys/cys.sock");let dept=root.join("cys-dept-fixture/cys.sock");
 fs::create_dir_all(root.join(".cys/state")).unwrap();
 fs::create_dir_all(root.join("tmp")).unwrap();
 for (name,pack) in [("BASE",root.join(".cys/pack")),("DEPT",root.join(".cys/pack-dept-fixture"))] {
  fs::create_dir_all(pack.join("directives")).unwrap();fs::create_dir_all(pack.join("memory")).unwrap();
  for file in ["directives/WORKER_DIRECTIVE.md","directives/RSI_LEARNING_DIRECTIVE.md","soul.md","memory/MEMORY.md"] { fs::write(pack.join(file),format!("{name}_P2_RECORDER_MARKER\n")).unwrap(); }
  fs::write(pack.join("agents.json"),json!({"probe":{"cmd":"fixture-agent","ready_marker":"READY","inject_delay_secs":1}}).to_string()).unwrap();
 }
 fs::write(root.join(".cys/depts.json"),json!({"depts":{"fixture":{"socket":dept}}}).to_string()).unwrap();
 let stop=Arc::new(AtomicBool::new(false));let seen=Arc::new(Mutex::new(Vec::<Value>::new()));let read_errors=Arc::new(Mutex::new(Vec::<String>::new()));let mut servers=Vec::new();
 for (lane,sock,sid,pid) in [("BASE",base.clone(),7,1),("DEPT",dept.clone(),9,std::process::id())] {
  fs::create_dir_all(sock.parent().unwrap()).unwrap();
  fs::write(cys::lane::epoch_path_in(&root.join(".cys/state"),&sock),json!({"socket":sock,"pid":pid,"daemon_epoch":100.1,"incarnation":format!("64-{pid:x}")}).to_string()).unwrap();
  let session=root.join(format!("{lane}-session.jsonl"));fs::write(&session,"").unwrap();
  let listener=std::os::unix::net::UnixListener::bind(&sock).unwrap();listener.set_nonblocking(true).unwrap();
  let stop=stop.clone();let seen=seen.clone();let read_errors=read_errors.clone();
  servers.push(std::thread::spawn(move|| {
   while !stop.load(Ordering::Relaxed) {
    let Some((mut stream,req))=recorder_next_request(&listener,Duration::from_secs(3),lane,&read_errors,&|_|{}) else {continue};let method=req["method"].as_str().unwrap();
    seen.lock().unwrap().push(json!({"lane":lane,"request":req}));
    if method=="surface.send_text" {
     let text=req["params"]["text"].as_str().unwrap_or("");
     if text.contains("지침 각성 확인 핑:") {
      fs::write(&session,format!("{}\n{}\n",json!({"type":"user","message":{"content":text}}),json!({"type":"assistant","message":{"content":"fixture response without acknowledgement"}}))).unwrap();
     }
    }
    let entry=json!({"surface_id":sid,"surface_ref":format!("surface:{sid}"),"role":"worker-99","agent":"probe","agent_alive":true,"exited":false,"line_count":0,"usage":{"source":"transcript","session_file":session}});
    let result=match method {
     "system.identify"=>json!({"socket_path":sock,"daemon_pid":pid,"started_at":100.2,"incarnation":format!("64-{pid:x}"),"caller":{"surface_id":sid,"surface_ref":format!("surface:{sid}")}}),
     "system.resolve_role"|"surface.create"=>json!({"surface_id":sid,"surface_ref":format!("surface:{sid}")}),
     "surface.list"=>json!({"surfaces":[entry]}),
     "surface.read_text"=>json!({"text":"READY","latest_cursor":10,"next_cursor":10,"quiet_secs":10}),
     "surface.wait_for"=>json!({"matched":!req["params"]["pattern"].as_str().unwrap_or("").starts_with("DIRECTIVE-ACK-"),"line":"READY"}),
     "system.claim_role"=>json!({"surface_id":sid,"role":"worker-99"}),
     _=>json!({"ok":true,"nonce":"fixture","surface_id":sid}),
    };
    let _=writeln!(stream,"{}",json!({"id":req["id"],"ok":true,"result":result}));
   }
  }));
 }
 let mut results=Vec::new();
 let cases=[("identify",vec!["identify"]),("status",vec!["set-status","--state","waiting"]),("claim",vec!["claim-role","worker-99"]),("reinject",vec!["reinject","--role","worker-99"]),("reinject-check",vec!["reinject","--role","worker-99","--check","--timeout","1"]),("launch",vec!["launch-agent","--role","worker-99","--agent","probe"]),("agy-missing-surface",vec!["usage-report-stdin","--agy","--quiet"]),("agy-explicit-surface",vec!["usage-report-stdin","--agy","--quiet","--surface","7"])];
 for explicit in [false,true] {
  for (name,args) in &cases {
   let begin=seen.lock().unwrap().len();let load_before=recorder_load();let started=std::time::Instant::now();
   let mut cmd=std::process::Command::new(&cli);cmd.args(["--exact", "b360_tests::p2_consumer_tests::p2_recorder_child", "--nocapture"]).env_clear().env("HOME",&root).env("PATH","/usr/bin:/bin:/usr/sbin:/sbin").env("CYS_NO_AUTOSTART","1").env("TMPDIR",root.join("tmp")).current_dir(&root);
   cmd.env("P2_RECORDER_ARGS",serde_json::to_string(args).unwrap()).env("RUST_MIN_STACK","16777216"); // libtest hosts the CLI main on a worker thread
   if explicit {cmd.env("CYS_SOCKET",&dept).env("CYS_PACK_DIR",root.join(".cys/pack-dept-fixture"));}
   cmd.stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
   let mut child=cmd.spawn().unwrap();
   if name.starts_with("agy-") { child.stdin.as_mut().unwrap().write_all(br#"{"product":"antigravity","quota":{"gemini-pro-5h":{"remaining_fraction":0.5}}}"#).unwrap(); }
   drop(child.stdin.take());
   let o=child.wait_with_output().unwrap();let elapsed=started.elapsed().as_millis();let load_after=recorder_load();
   let calls=seen.lock().unwrap()[begin..].to_vec();
   results.push(json!({"case":name,"explicit_context":explicit,"rc":o.status.code(),"stdout":String::from_utf8_lossy(&o.stdout),"stderr":String::from_utf8_lossy(&o.stderr),"elapsed_ms":elapsed,"vm.loadavg_before":load_before,"vm.loadavg_after":load_after,"calls":calls}));
  }
 }
 stop.store(true,Ordering::Relaxed);for s in servers{s.join().unwrap();}
 let read_errors=read_errors.lock().unwrap().clone();
 let claim_dir=root.join(".cys/pack/state/reinject-check");
 let claims=fs::read_dir(&claim_dir).map(|x|x.flatten().map(|e|e.file_name().to_string_lossy().into_owned()).collect::<Vec<_>>()).unwrap_or_default();
 fs::create_dir_all(&out).unwrap();fs::write(out.join("RESULT.json"),serde_json::to_vec_pretty(&json!({"root":root,"results":results,"base_claim_files":claims,"fixture_read_errors":read_errors,"scope":"real CLI and finite BASE/DEPT peers; no live daemon, no actual agent launch"})).unwrap()).unwrap();
 assert!(read_errors.is_empty(),"the recorder peer could not read a request (fixture fault, not a CLI result): {read_errors:?}");
 let mut violations=Vec::new();
 if claims.is_empty() || claims.iter().any(|p|!p.starts_with("s7-")) { violations.push(format!("reinject-check claim used inferred instead of real identity: {claims:?}")); }
 let stamp_dir=root.join("tmp").join(format!("cys-outside-usage-{}",unsafe{libc::getuid()}));
 let base_stamp=agy_stamp_path_in(&stamp_dir,&base,7);
 let inferred_stamp=agy_stamp_path_in(&stamp_dir,&dept,9);
 let explicit_stamp=agy_stamp_path_in(&stamp_dir,&dept,7);
 if !base_stamp.exists() || inferred_stamp.exists() || !explicit_stamp.exists() {violations.push(format!("agy stamp keys: base7={} inferred9={} explicit-dept7={}",base_stamp.exists(),inferred_stamp.exists(),explicit_stamp.exists()));}
 fs::write(out.join("LOCAL-STATE.json"),serde_json::to_vec_pretty(&json!({"base_claims":claims,"base_stamp":base_stamp.exists(),"inferred_stamp":inferred_stamp.exists(),"explicit_stamp":explicit_stamp.exists()})).unwrap()).unwrap();
 for r in &results {
  let name=r["case"].as_str().unwrap();let explicit=r["explicit_context"]==true;
  assert_eq!(r["rc"],0,"fixture must reach success: {r}");
  let calls=r["calls"].as_array().unwrap();
  let mixed=["reinject","reinject-check","launch","agy-missing-surface","agy-explicit-surface"].contains(&name);
  let diagnostics=r["stderr"].as_str().unwrap().lines().filter(|l|l.starts_with("cys: inferred lane ")).collect::<Vec<_>>();
  assert_eq!(diagnostics.len(),usize::from(mixed && !explicit),"diagnostic count: {r}");
  if mixed && !explicit {
   assert!(calls.iter().filter(|c|c["request"]["method"]!="system.identify").all(|c|c["lane"]=="BASE"),"mixed command changed the legacy target: {r}");
   for token in [base.to_str().unwrap(),dept.to_str().unwrap(),"CYS_SOCKET","CYS_PACK_DIR"] {assert!(diagnostics[0].contains(token),"diagnostic omits {token}");}
  }

  if ["reinject","reinject-check","launch"].contains(&name) {
   let marker=if explicit{"DEPT_P2_RECORDER_MARKER"}else{"BASE_P2_RECORDER_MARKER"};
   let payloads=calls.iter().filter(|c|c["request"]["method"]=="surface.send_text" && c["request"]["params"]["text"].as_str().unwrap_or("").contains(marker)).collect::<Vec<_>>();
   assert!(!payloads.is_empty(),"fixture did not exercise directive delivery: {name}, {explicit}");
   if payloads.iter().any(|c|c["lane"] != if explicit{"DEPT"}else{"BASE"}) {violations.push(format!("{name}: BASE directive reached DEPT"));}
  } else if !name.starts_with("agy-") { assert!(calls.iter().all(|c|c["lane"]=="DEPT"),"transport recovery/explicit control: {r}"); }
 }
 fs::remove_dir_all(&root).unwrap();
 assert!(violations.is_empty(),"{violations:?}");println!("BASE/DEPT recorder and explicit controls PASS");
}

    #[cfg(unix)]
    #[test]
    fn p2_recorder_child() {
        let Ok(raw)=std::env::var("P2_RECORDER_ARGS") else {return;};
        let args:Vec<String>=serde_json::from_str(&raw).unwrap();
        let cli=Cli::try_parse_from(std::iter::once("cys".to_owned()).chain(args)).unwrap();
        cys::seal_python_bytecode_in_process();
        if let Some(s)=&cli.socket {std::env::set_var(cys::ENV_SOCKET,s);}
        initialize_lane_identity(&cli.command);
        std::process::exit(run(cli.command));
    }

    #[cfg(unix)]
    #[test]
    fn p2_recorder_peer() {
        let Ok(out)=std::env::var("P2_RECORDER_OUTPUT") else {return;};
        recorder_fixture(std::env::current_exe().unwrap(),out.into());
    }

    #[cfg(unix)]
    #[test]
    fn p2_base_dept_marker_recorder() {
        let root=std::env::temp_dir().join(format!("p2-recorder-{}",std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let peer=root.join("cysd");std::fs::copy(std::env::current_exe().unwrap(),&peer).unwrap();
        let mut command=std::process::Command::new(&peer);
        command.args(["--exact","b360_tests::p2_consumer_tests::p2_recorder_peer","--nocapture"])
            .env("P2_RECORDER_OUTPUT",root.join("results"));
        for (key,_) in std::env::vars_os() {
            let k=key.to_string_lossy();if k.starts_with("CYS_")||k.starts_with("JAVIS_")||k.starts_with("AITERM_")||k=="CLAUDE_CONFIG_DIR" {command.env_remove(key);}
        }
        let output=command.output().unwrap();
        let result=root.join("results/RESULT.json");
        assert!(result.is_file(),"recorder did not finish: {}",String::from_utf8_lossy(&output.stdout));
        assert!(output.status.success(),"recorder failed; evidence {}\n{}\n{}",result.display(),String::from_utf8_lossy(&output.stdout),String::from_utf8_lossy(&output.stderr));
        std::fs::remove_dir_all(root).unwrap();
    }
}
