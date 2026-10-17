use super::*;
use std::sync::mpsc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

struct TempRoot(std::path::PathBuf);
impl TempRoot {
    fn new() -> Self {
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("cys-receipt-framing-{}-{}-{}", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos(),
            SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
        std::fs::create_dir(&path).unwrap(); Self(path)
    }
}
impl Drop for TempRoot { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }

#[tokio::test]
async fn c8_received_framer_preserves_blank_invalid_and_partial_eof_release() {
    let root = TempRoot::new(); let daemon = Daemon::new(root.0.join("cysd.sock"));
    let at = Instant::now(); daemon.receipt_registry.test_clock(Some(at));
    let (mut write, read) = tokio::io::duplex(128);
    write.write_all(b"  \n{").await.unwrap(); write.shutdown().await.unwrap();
    let mut reader = BufReader::new(read);
    {
        let (line, frame) = next_received_line_capped(&mut reader, MAX_REQUEST_LINE, &daemon.receipt_registry).await.unwrap().unwrap();
        assert_eq!(frame.at(), at);
        assert!(matches!(parse_received_line(&daemon, line, frame), ParsedReceivedLine::Blank));
    }
    let later = at + Duration::from_secs(61);
    assert_eq!(daemon.receipt_registry.with_expiry_clock(later, |t| t), later);
    {
        let (line, frame) = next_received_line_capped(&mut reader, MAX_REQUEST_LINE, &daemon.receipt_registry).await.unwrap().unwrap();
        assert_eq!(line, "{");
        let ParsedReceivedLine::Invalid(error) = parse_received_line(&daemon, line, frame) else { panic!("expected parse error") };
        assert_eq!(error["error"]["code"], "parse_error");
    }
    assert_eq!(daemon.receipt_registry.with_expiry_clock(later, |t| t), later);
    assert!(next_received_line_capped(&mut reader, MAX_REQUEST_LINE, &daemon.receipt_registry).await.unwrap().is_none());
    let (mut write, read) = tokio::io::duplex(128);
    write.write_all(b"12345").await.unwrap(); write.shutdown().await.unwrap();
    let mut reader = BufReader::new(read);
    let result = next_received_line_capped(&mut reader, 4, &daemon.receipt_registry).await;
    assert!(matches!(result, Err(ref e) if e.kind() == std::io::ErrorKind::InvalidData));
    drop(result);
    assert_eq!(daemon.receipt_registry.with_expiry_clock(later, |t| t), later);
}

// No runtime execution is implied by a Send assertion; normal compilation checks the actual
// connection future. The separate compile-fail specimen intentionally keeps PendingFrame over await.
#[test]
fn c8_received_rpc_context_is_send() {
    fn send<T: Send>() {}
    send::<handlers::ReceivedRpc>();
    send::<crate::rpc_receipt::ReceivedFrame<crate::submit_assoc::HeldSubmitRetention>>();
}

#[cfg(unix)]
mod unix_dispatch {
    use super::*;
    use std::ffi::OsString;
    use std::os::unix::fs::PermissionsExt;
    use crate::state::Surface;
    use std::sync::atomic::Ordering;

    struct EnvRestore { key: &'static str, prior: Option<OsString> }
    impl EnvRestore {
        fn set(key: &'static str, value: &std::path::Path) -> Self {
            let prior = std::env::var_os(key); std::env::set_var(key, value); Self { key, prior }
        }
    }
    impl Drop for EnvRestore {
        fn drop(&mut self) { match &self.prior {
            Some(v) => std::env::set_var(self.key, v), None => std::env::remove_var(self.key),
        } }
    }
    struct Rig { daemon: Arc<Daemon>, owned: Vec<Arc<Surface>>, tasks: Vec<tokio::task::JoinHandle<()>>, closed: bool }
    impl Rig {
        fn seat(&mut self, root: &std::path::Path) -> Arc<Surface> {
            let env = vec![("HOME".into(), root.to_string_lossy().into_owned()),
                ("ZDOTDIR".into(), root.to_string_lossy().into_owned())];
            let surface = self.daemon.create_surface_with_env(Some(root.to_string_lossy().into_owned()),
                Some("ignored".into()), None, None, 24, 80, &env, None).unwrap();
            self.owned.push(surface.clone());
            // Legacy intent classification fixture only. No receiver/native/write proof is minted.
            *surface.agent_meta.lock().unwrap() = Some(("claude".into(), "claude".into()));
            surface
        }
        fn connection(&mut self) -> (BufReader<tokio::io::DuplexStream>, usize) {
            let (client, server) = tokio::io::duplex(8192); let daemon = self.daemon.clone();
            let task = tokio::spawn(handle_connection_capped(daemon, Box::new(server) as Stream, None,
                Some(Duration::from_secs(60)), Some(Duration::from_secs(5))));
            let n = self.tasks.len(); self.tasks.push(task); (BufReader::new(client), n)
        }

        /// Success means every owned exec-cat root was observed exited through its child handle.
        /// Connection tasks are requested to abort, not re-awaited: two handles were already joined
        /// by the cancellation assertions. This method makes no task-join or descendant guarantee.
        fn close(&mut self) -> Result<(), String> {
            if self.closed { return Ok(()); }
            for task in &self.tasks { task.abort(); }
            let mut errors = Vec::new();
            for surface in &self.owned {
                let mut child = surface.child.lock().unwrap_or_else(|e| e.into_inner());
                let needs_kill = match child.try_wait() {
                    Ok(Some(_)) => false,
                    Ok(None) => true,
                    Err(error) => {
                        errors.push(format!("surface {} initial child wait failed: {error}", surface.id));
                        true
                    }
                };
                if needs_kill {
                    if let Err(error) = child.kill() {
                        errors.push(format!("surface {} child kill failed: {error}", surface.id));
                    }
                }
            }
            let deadline = Instant::now() + Duration::from_secs(5);
            for surface in &self.owned {
                loop {
                    // Release the child mutex before the bounded cleanup wait.
                    let status = surface.child.lock().unwrap_or_else(|e| e.into_inner()).try_wait();
                    match status {
                        Ok(Some(_)) => { eprintln!("C8_RECEIPT_CLOSE_REAPED pid={}", surface.pid); break; },
                        Ok(None) if Instant::now() < deadline => {
                            std::thread::sleep(Duration::from_millis(10));
                        }
                        Ok(None) => {
                            errors.push(format!("surface {} child was not reaped before cleanup deadline", surface.id));
                            break;
                        }
                        Err(error) => {
                            errors.push(format!("surface {} child reap failed: {error}", surface.id));
                            break;
                        }
                    }
                }
            }
            if !errors.is_empty() {
                // Keep ownership and leave Drop armed so a failed close still gets cleanup fallback.
                return Err(errors.join("; "));
            }
            self.daemon.surfaces.lock().unwrap_or_else(|e| e.into_inner()).clear();
            self.closed = true;
            Ok(())
        }
    }
    impl Drop for Rig {
        fn drop(&mut self) {
            if self.closed { return; }
            // Best-effort fallback after panic/failed close. This is not the success oracle.
            for task in &self.tasks { task.abort(); }
            // Both actual roots exec only /bin/cat. No profile or descendant command is run.
            for surface in &self.owned {
                let mut child = surface.child.lock().unwrap_or_else(|e| e.into_inner());
                if child.try_wait().ok().flatten().is_none() { let _ = child.kill(); }
            }
            let deadline = Instant::now() + Duration::from_secs(5);
            for surface in &self.owned {
                loop {
                    match surface.child.lock().unwrap_or_else(|e| e.into_inner()).try_wait() {
                        Ok(Some(_)) => { eprintln!("C8_RECEIPT_FALLBACK_REAPED pid={}", surface.pid); break; },
                        _ if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
                        _ => { eprintln!("receipt fixture child cleanup unresolved for {}", surface.id); break; }
                    }
                }
            }
            self.daemon.surfaces.lock().unwrap_or_else(|e| e.into_inner()).clear();
        }
    }
    async fn send(client: &mut BufReader<tokio::io::DuplexStream>, request: serde_json::Value) {
        client.get_mut().write_all(format!("{request}\n").as_bytes()).await.unwrap();
    }
    async fn reply(client: &mut BufReader<tokio::io::DuplexStream>) -> serde_json::Value {
        let mut line = String::new();
        tokio::time::timeout(Duration::from_secs(10), client.read_line(&mut line)).await.unwrap().unwrap();
        serde_json::from_str(&line).unwrap()
    }
    fn checkpoint(rx: &mpsc::Receiver<crate::rpc_receipt::TestCheckpoint>, id: &str, kind: &str) -> Instant {
        loop {
            let point = rx.recv_timeout(Duration::from_secs(10)).expect("missing actual receipt/dispatch checkpoint");
            if point.id == json!(id) && point.kind == kind { return point.at; }
        }
    }
    async fn all_dispatch_permits() -> tokio::sync::SemaphorePermit<'static> {
        tokio::time::timeout(Duration::from_secs(10), DISPATCH_GATE.acquire_many(DISPATCH_INFLIGHT_MAX as u32))
            .await.unwrap().unwrap()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn c8_actual_connection_preserves_receipt_and_residency_through_gate_and_cancel() {
        let _lock = crate::governance::PACK_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _state = crate::delivery::tests::isolate_state_dir("receipt-dispatch");
        let root = TempRoot::new(); let shell = root.0.join("fixture-shell");
        std::fs::write(&shell, b"#!/bin/sh\nexec /bin/cat\n").unwrap();
        std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::write(root.0.join("acl.json"), r#"{"default":"allow","rules":[]}"#).unwrap();
        let _pack = EnvRestore::set(cys::pack::ENV_PACK_DIR, &root.0);
        let _shell = EnvRestore::set("CYS_SHELL", &shell);
        let mut rig = Rig { daemon: Daemon::new(root.0.join("cysd.sock")), owned: Vec::new(), tasks: Vec::new(), closed: false };
        let surface = rig.seat(&root.0); let other = rig.seat(&root.0);
        let (tx, rx) = mpsc::channel(); rig.daemon.receipt_registry.test_observe(tx);
        let (mut body_conn, _) = rig.connection();
        // Controlled monotonic receipt values model time spent at the real admission gate.
        // Runtime processing uses its real clock, which is already after this original deadline.
        let t0 = Instant::now().checked_sub(Duration::from_secs(61)).expect("fixture backdate span");
        rig.daemon.receipt_registry.test_clock(Some(t0));
        send(&mut body_conn, json!({"id":"body","method":"surface.send_text", "params":{
            "surface_id":surface.id,"from":"surface:9001","text":"original retained directive"}})).await;
        assert_eq!(checkpoint(&rx, "body", "narrowed"), t0);
        let body_reply = reply(&mut body_conn).await;
        assert_eq!(body_reply["result"]["held"], "awaiting_submit", "{body_reply}");
        let body_id = body_reply["result"]["request_id"].as_str().unwrap().to_owned();
        let deadline = {
            let table = surface.submit_assoc.lock().unwrap(); let body = table.records().next().unwrap();
            assert_eq!(body.received_at, t0, "handler processing must not renew the full-frame clock");
            assert_eq!(body.deadline, t0 + Duration::from_secs(60)); body.deadline
        };
        let permit = all_dispatch_permits().await;
        let return_at = deadline - Duration::from_millis(1);
        rig.daemon.receipt_registry.test_clock(Some(return_at));
        let (mut return_conn, return_task) = rig.connection();
        let (release_tx, release_rx) = mpsc::channel();
        rig.daemon.receipt_registry.test_pause_dispatch(json!("return"), release_rx);
        send(&mut return_conn, json!({"id":"return","method":"surface.send_key","params":{
            "surface_id":surface.id,"from":"surface:9001","key":"Return","queued":true}})).await;
        assert_eq!(checkpoint(&rx, "return", "narrowed"), return_at);
        let expiry_now = deadline + Duration::from_secs(2);
        assert_eq!(rig.daemon.receipt_registry.with_expiry_clock(expiry_now, |t| t), expiry_now,
            "no global unparsed barrier may survive into admission wait");
        crate::handlers::sweep_held_submits_on(&rig.daemon, &surface, expiry_now);
        assert_eq!(surface.submit_assoc.lock().unwrap().ids(), [body_id.clone()], "timely Return lost residency while admission was saturated");
        // Ping uses the SAME production connection function and remains exempt while Return waits.
        let (mut ping_conn, _) = rig.connection();
        send(&mut ping_conn, json!({"id":"ping","method":"system.ping","params":{}})).await;
        assert_eq!(reply(&mut ping_conn).await["ok"], true);
        drop(permit);
        assert_eq!(checkpoint(&rx, "return", "started"), return_at);
        rig.tasks[return_task].abort();
        let cancelled = (&mut rig.tasks[return_task]).await.expect_err("connection should be cancelled");
        assert!(cancelled.is_cancelled());
        crate::handlers::sweep_held_submits_on(&rig.daemon, &surface, expiry_now);
        assert_eq!(surface.submit_assoc.lock().unwrap().ids(), [body_id.clone()],
            "cancelled awaiter must not release a running dispatch's retention");
        release_tx.send(()).unwrap();
        checkpoint(&rx, "return", "finished");
        assert_eq!(surface.submit_assoc.lock().unwrap().len(), 0);
        let queue = surface.pending_queue.lock().unwrap();
        assert_eq!(queue.len(), 1); assert_eq!(queue.front().unwrap().text, "original retained directive");
        drop(queue);
        let events = rig.daemon.bus.tail(200);
        assert_eq!(events.iter().filter(|e| e["name"] == "queue.enqueued").count(), 1,
            "the original body is associated exactly once despite caller cancellation");

        // Late receipt: holding an older body does not make an out-of-window token eligible.
        surface.submit_assoc.lock().unwrap().hold(crate::submit_assoc::HeldBody::new(
            "late-token".into(), crate::submit_assoc::CallerKey::Claimed(9001),
            surface.agent_epoch.load(Ordering::Acquire), t0, "must not queue".into(), false)).unwrap();
        rig.daemon.receipt_registry.test_clock(Some(deadline + Duration::from_millis(1)));
        let (mut late_conn, _) = rig.connection();
        send(&mut late_conn, json!({"id":"late","method":"surface.send_key","params":{
            "surface_id":surface.id,"key":"Return","request_id":"late-token","queued":true}})).await;
        let late = reply(&mut late_conn).await;
        assert_eq!(late["error"]["code"], "submit_unbound");
        assert_eq!(surface.pending_queue.lock().unwrap().len(), 1);

        // Cancellation BEFORE spawn releases only residency, and never dispatches the Return.
        surface.submit_assoc.lock().unwrap().hold(crate::submit_assoc::HeldBody::new(
            "cancel-before-spawn".into(), crate::submit_assoc::CallerKey::Claimed(9001),
            surface.agent_epoch.load(Ordering::Acquire), t0, "cancelled waiting body".into(), false)).unwrap();
        let permit = all_dispatch_permits().await;
        rig.daemon.receipt_registry.test_clock(Some(return_at));
        let (mut waiting_conn, waiting_task) = rig.connection();
        send(&mut waiting_conn, json!({"id":"cancel-before","method":"surface.send_key","params":{
            "surface_id":surface.id,"key":"Return","request_id":"cancel-before-spawn","queued":true}})).await;
        checkpoint(&rx, "cancel-before", "narrowed");
        rig.tasks[waiting_task].abort();
        let cancelled = (&mut rig.tasks[waiting_task]).await.expect_err("waiting connection cancellation");
        assert!(cancelled.is_cancelled());
        crate::handlers::sweep_held_submits_on(&rig.daemon, &surface, expiry_now);
        assert_eq!(surface.submit_assoc.lock().unwrap().len(), 0);
        assert_eq!(surface.pending_queue.lock().unwrap().len(), 1);
        drop(permit);

        // A target absent at receipt cannot inherit an object installed before admitted dispatch.
        let permit = all_dispatch_permits().await;
        let missing = surface.id.max(other.id) + 1000;
        let (mut moved_conn, _) = rig.connection();
        send(&mut moved_conn, json!({"id":"missing-target","method":"surface.send_text","params":{
            "surface_id":missing,"from":"surface:9001","text":"must not attach later"}})).await;
        checkpoint(&rx, "missing-target", "narrowed");
        rig.daemon.surfaces.lock().unwrap().insert(missing, other.clone());
        drop(permit);
        assert_eq!(reply(&mut moved_conn).await["error"]["code"], "target_changed", "missing receipt target inherited a later object");
        assert_eq!(other.submit_assoc.lock().unwrap().len(), 0);
        rig.daemon.surfaces.lock().unwrap().remove(&missing);

        // Previously present target replaced after narrowing likewise cannot inherit the call.
        let permit = all_dispatch_permits().await;
        let (mut moved_conn, _) = rig.connection();
        send(&mut moved_conn, json!({"id":"replaced-target","method":"surface.send_text","params":{
            "surface_id":surface.id,"from":"surface:9001","text":"wrong incarnation"}})).await;
        checkpoint(&rx, "replaced-target", "narrowed");
        rig.daemon.surfaces.lock().unwrap().insert(surface.id, other.clone());
        drop(permit);
        assert_eq!(reply(&mut moved_conn).await["error"]["code"], "target_changed", "replacement inherited the original receipt target");
        rig.daemon.surfaces.lock().unwrap().insert(surface.id, surface.clone());
        assert_eq!(other.submit_assoc.lock().unwrap().len(), 0);

        // Empty narrowing: body was received FIRST but remained unadmitted behind the gate.
        // Return waits at its actual dispatch barrier while body admission finishes.
        let permit = all_dispatch_permits().await;
        let fresh = Instant::now(); rig.daemon.receipt_registry.test_clock(Some(fresh));
        let (mut deferred_body, _) = rig.connection();
        send(&mut deferred_body, json!({"id":"deferred-body","method":"surface.send_text","params":{
            "surface_id":surface.id,"from":"surface:9002","text":"late admission earlier receipt"}})).await;
        checkpoint(&rx, "deferred-body", "narrowed");
        rig.daemon.receipt_registry.test_clock(Some(fresh + Duration::from_millis(1)));
        let (release_tx, release_rx) = mpsc::channel();
        rig.daemon.receipt_registry.test_pause_dispatch(json!("empty-return"), release_rx);
        let (mut empty_return, _) = rig.connection();
        send(&mut empty_return, json!({"id":"empty-return","method":"surface.send_key","params":{
            "surface_id":surface.id,"from":"surface:9002","key":"Return","queued":true}})).await;
        checkpoint(&rx, "empty-return", "narrowed"); drop(permit);
        checkpoint(&rx, "empty-return", "started");
        let deferred = reply(&mut deferred_body).await;
        assert_eq!(deferred["result"]["held"], "awaiting_submit");
        let deferred_id = deferred["result"]["request_id"].as_str().unwrap().to_owned();
        release_tx.send(()).unwrap();
        let empty = reply(&mut empty_return).await;
        assert!(empty["result"]["bound"].is_null(), "empty narrowing floated: {empty}");
        assert!(surface.submit_assoc.lock().unwrap().ids().contains(&deferred_id));
        assert!(!surface.pending_queue.lock().unwrap().iter().any(|e| e.text == "late admission earlier receipt"));
        // Legacy NoBody remains ordinary control; this test deliberately does not require a new error.
        rig.daemon.receipt_registry.test_clock(None);
        rig.close().expect("receipt fixture must reap every owned exec-cat root before success");
    }

    // Each case uses the production shared connection/framer, then holds the actual
    // dispatch gate until receipt narrowing has observed the old (or absent) target.
    async fn check_return_target_does_not_inherit_another_object(replaced: bool) {
        let _lock = crate::governance::PACK_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _state = crate::delivery::tests::isolate_state_dir("receipt-return-target");
        let root = TempRoot::new(); let shell = root.0.join("fixture-shell");
        std::fs::write(&shell, b"#!/bin/sh\nexec /bin/cat\n").unwrap();
        std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::write(root.0.join("acl.json"), r#"{"default":"allow","rules":[]}"#).unwrap();
        let _pack = EnvRestore::set(cys::pack::ENV_PACK_DIR, &root.0);
        let _shell = EnvRestore::set("CYS_SHELL", &shell);
        let mut rig = Rig { daemon: Daemon::new(root.0.join("cysd.sock")), owned: Vec::new(), tasks: Vec::new(), closed: false };
        let original = rig.seat(&root.0); let replacement = rig.seat(&root.0);
        let (tx, rx) = mpsc::channel(); rig.daemon.receipt_registry.test_observe(tx);
        let at = Instant::now(); rig.daemon.receipt_registry.test_clock(Some(at));
        let addressed = if replaced { original.id } else { original.id.max(replacement.id).checked_add(1000).unwrap() };
        let request_id = if replaced { "return-replaced-target" } else { "return-missing-target" };
        let permit = all_dispatch_permits().await;
        let (mut client, _) = rig.connection();
        send(&mut client, json!({"id":request_id,"method":"surface.send_key","params":{
            "surface_id":addressed,"from":"surface:9001","key":"Return","queued":true}})).await;
        assert_eq!(checkpoint(&rx, request_id, "narrowed"), at);
        // Change only the daemon's lookup after actual synchronous receipt narrowing.
        rig.daemon.surfaces.lock().unwrap().insert(addressed, replacement.clone());
        drop(permit);
        let answer = reply(&mut client).await;
        assert_eq!(answer["error"]["code"], "target_changed",
            "send_key Return inherited a missing/replaced receipt target: case={request_id} reply={answer}");
        assert!(replacement.pending_queue.lock().unwrap().is_empty(), "refused Return enqueued on replacement");
        assert!(original.pending_queue.lock().unwrap().is_empty());
        assert_eq!(rig.daemon.bus.tail(100).iter().filter(|e| e["name"] == "queue.enqueued").count(), 0);
        if replaced { rig.daemon.surfaces.lock().unwrap().insert(addressed, original.clone()); }
        else { rig.daemon.surfaces.lock().unwrap().remove(&addressed); }

        // Positive control: the same real send_key Return path accepts a stable target.
        // True NoBody retains ordinary queued-control behavior; no new association policy.
        send(&mut client, json!({"id":"return-stable-target","method":"surface.send_key","params":{
            "surface_id":original.id,"from":"surface:9001","key":"Return","queued":true}})).await;
        let stable = reply(&mut client).await;
        assert_eq!(stable["ok"], true, "stable Return path unexpectedly refused: {stable}");
        assert_eq!(original.pending_queue.lock().unwrap().len(), 1);
        assert!(replacement.pending_queue.lock().unwrap().is_empty());
        rig.daemon.receipt_registry.test_clock(None);
        rig.close().expect("receipt Return target fixture must reap every owned exec-cat root before success");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn c8_actual_send_key_return_missing_at_receipt_cannot_inherit_later_target() {
        check_return_target_does_not_inherit_another_object(false).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn c8_actual_send_key_return_replaced_after_receipt_cannot_inherit_target() {
        check_return_target_does_not_inherit_another_object(true).await;
    }
}
