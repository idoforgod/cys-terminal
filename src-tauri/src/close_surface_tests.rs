//! S6: execute the production close operation and its real attachment map/handles.
//! Only inert futures, fake token loaders and runtime tasks: no socket or child process.

use super::{close_pane_veto_message, close_surface_with, sock_slug, Attachments};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::future::{poll_fn, ready, Future};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::Poll;

const SID: u64 = 42;
const RELAUNCH: &str = "Relaunch it from";

#[test]
fn s6_close_pane_veto_guidance_matches_platform() {
    for (platform, launcher) in [("windows", "Windows Explorer or the Start menu"),
        ("macos", "Finder"), ("linux", "your desktop launcher")] {
        assert_eq!(close_pane_veto_message(platform), format!(
            "Close refused because this app has a live pane ancestor in the selected daemon. Relaunch it from {launcher}, then try again."));
    }
}

struct Lifetime(Arc<AtomicBool>, Arc<tokio::sync::Notify>);
impl Drop for Lifetime {
    fn drop(&mut self) { self.0.store(true, Ordering::SeqCst); self.1.notify_one(); }
}

struct Fixture {
    state: Attachments,
    socket: PathBuf,
    dropped: Arc<AtomicBool>,
    completion: Arc<tokio::sync::Notify>,
}

impl Fixture {
    fn new() -> Self {
        let socket = PathBuf::from("s6-inert-selected-socket");
        let dropped = Arc::new(AtomicBool::new(false));
        let completion = Arc::new(tokio::sync::Notify::new());
        let lifetime = Lifetime(dropped.clone(), completion.clone());
        let handle = tauri::async_runtime::spawn(async move {
            let _lifetime = lifetime;
            std::future::pending::<()>().await;
        });
        let mut map = HashMap::new();
        map.insert((sock_slug(&socket), SID), handle);
        Self { state: Attachments(Mutex::new(map)), socket, dropped, completion }
    }

    fn attached(&self) -> bool {
        self.state.0.lock().unwrap().contains_key(&(sock_slug(&self.socket), SID))
    }

    async fn assert_aborted(&self) {
        if !self.dropped.load(Ordering::SeqCst) {
            tokio::time::timeout(std::time::Duration::from_secs(5), self.completion.notified())
                .await.expect("attachment lifetime did not complete after abort request");
        }
        assert!(self.dropped.load(Ordering::SeqCst), "observed task lifetime drop after accepted close");
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // A failing test also owns cleanup; no detached fixture task may survive it.
        for (_, handle) in self.state.0.lock().unwrap().drain() { handle.abort(); }
    }
}

fn confirmed(sid: u64) -> Value {
    json!({"ok": true, "result": {"surface_id": sid, "closed": true, "cause": "OwnerClose"}})
}

async fn call(f: &Fixture, reply: Result<Value, String>) -> Result<(), String> {
    close_surface_with(&f.state, &f.socket, SID,
        |_socket| Some("SYNTHETIC-OWNER-CREDENTIAL".to_string()),
        move |_socket, _params| ready(reply)).await
}

#[test]
fn s6_close_pending_keeps_attachment_and_does_not_hold_map_lock() {
    tauri::async_runtime::block_on(async {
        let f = Fixture::new();
        let mut operation = Box::pin(close_surface_with(&f.state, &f.socket, SID,
            |_socket| Some("synthetic".into()),
            |_socket, _params| std::future::pending::<Result<Value, String>>()));
        poll_fn(|cx| {
            assert!(operation.as_mut().poll(cx).is_pending(), "fixture RPC must stay pending");
            Poll::Ready(())
        }).await;
        let retained_without_lock = f.state.0.try_lock()
            .map(|map| map.contains_key(&(sock_slug(&f.socket), SID))).unwrap_or(false);
        // Drop the pending operation first, so a bad held-lock implementation cannot
        // hang fixture teardown when the discriminating assertion fails.
        drop(operation);
        assert!(retained_without_lock, "pending close must retain the same attachment and release the map lock");
        assert!(f.attached(), "cancelling an unconfirmed request must not detach");
        assert!(!f.dropped.load(Ordering::SeqCst), "pending request must not abort its stream");
    });
}

#[test]
fn s6_close_denial_retains_actual_attachment() {
    tauri::async_runtime::block_on(async {
        let f = Fixture::new();
        let result = call(&f, Ok(json!({"ok": false, "error": {
            "code": "close_denied", "message": "S6 denied"}}))).await;
        assert!(result.is_err(), "denial must propagate to the UI");
        assert!(f.attached(), "denied close must keep the attachment entry");
        assert!(!f.dropped.load(Ordering::SeqCst), "denied close must not abort the stream");
    });
}

#[test]
fn s6_close_failed_retained_target_keeps_attachment_without_ancestry_diagnosis() {
    tauri::async_runtime::block_on(async {
        let f = Fixture::new();
        let message = call(&f, Ok(json!({"ok": false, "error": {
            "code": "close_failed", "reason": "pane_ancestor_running", "message": "retained target"}})))
            .await.expect_err("retained target is not confirmed gone");
        assert!(f.attached(), "close_failed must retain the actual attachment");
        assert!(!message.contains(RELAUNCH), "close_failed cannot invent an owner-veto diagnosis");
    });
}

#[test]
fn s6_close_same_key_replacement_survives_failure_and_cancellation() {
    tauri::async_runtime::block_on(async {
        let cases = [
            Some(Ok(json!({"ok": false, "error": {"code": "close_denied", "message": "deny"}}))),
            Some(Ok(json!({"ok": false, "error": {"code": "close_failed", "message": "retained"}}))),
            Some(Ok(json!({"ok": true, "result": {"surface_id": SID + 1, "closed": true}}))),
            Some(Err("transport uncertain".to_owned())),
            None,
        ];
        for reply in cases {
            let f = Fixture::new();
            let (response_tx, response_rx) = tokio::sync::oneshot::channel();
            let mut operation = Box::pin(close_surface_with(&f.state, &f.socket, SID,
                |_socket| None, |_socket, _params| async move { response_rx.await.unwrap() }));
            poll_fn(|cx| { assert!(operation.as_mut().poll(cx).is_pending()); Poll::Ready(()) }).await;
            let replacement_drop = Arc::new(AtomicBool::new(false));
            let completion = Arc::new(tokio::sync::Notify::new());
            let lifetime = Lifetime(replacement_drop.clone(), completion.clone());
            // Dropping this sender also completes a detached fixture task on panic.
            let (_finish, finished) = tokio::sync::oneshot::channel::<()>();
            let replacement = tauri::async_runtime::spawn(async move {
                let _lifetime = lifetime; let _ = finished.await;
            });
            let map_key = (sock_slug(&f.socket), SID);
            {
                let mut map = f.state.0.try_lock().expect("pending RPC must not hold attachment lock");
                if let Some(old) = map.remove(&map_key) { old.abort(); }
                map.insert(map_key.clone(), replacement);
            }
            if let Some(response) = reply {
                response_tx.send(response).unwrap();
                assert!(operation.await.is_err(), "unconfirmed close must reject");
            } else {
                drop(operation);
            }
            assert!(f.attached(), "failure/cancellation must preserve the current replacement entry");
            assert!(!replacement_drop.load(Ordering::SeqCst), "failure must not abort the replacement stream");
            // Prove the retained map entry really is B, rather than resurrected A.
            f.state.0.lock().unwrap().remove(&map_key).unwrap().abort();
            tokio::time::timeout(std::time::Duration::from_secs(5), completion.notified())
                .await.expect("retained entry must own replacement B");
            assert!(replacement_drop.load(Ordering::SeqCst));
        }
    });
}

#[test]
fn s6_close_same_incarnation_confirmation_aborts_current_replacement() {
    tauri::async_runtime::block_on(async {
        for reply in [confirmed(SID), json!({"ok": false, "error": {"code": "not_found", "message": "gone"}})] {
            let f = Fixture::new();
            let (response_tx, response_rx) = tokio::sync::oneshot::channel();
            let mut operation = Box::pin(close_surface_with(&f.state, &f.socket, SID,
                |_socket| None, |_socket, _params| async move { response_rx.await.unwrap() }));
            poll_fn(|cx| { assert!(operation.as_mut().poll(cx).is_pending()); Poll::Ready(()) }).await;
            let dropped = Arc::new(AtomicBool::new(false));
            let completion = Arc::new(tokio::sync::Notify::new());
            let lifetime = Lifetime(dropped.clone(), completion.clone());
            let (_finish, finished) = tokio::sync::oneshot::channel::<()>();
            let replacement = tauri::async_runtime::spawn(async move {
                let _lifetime = lifetime; let _ = finished.await;
            });
            let map_key = (sock_slug(&f.socket), SID);
            {
                let mut map = f.state.0.try_lock().expect("pending RPC must not hold attachment lock");
                if let Some(old) = map.remove(&map_key) { old.abort(); }
                map.insert(map_key, replacement);
            }
            response_tx.send(Ok(reply)).unwrap();
            assert!(operation.await.is_ok(), "same-incarnation confirmation must be accepted");
            assert!(!f.attached(), "confirmed close must remove current replacement B");
            tokio::time::timeout(std::time::Duration::from_secs(5), completion.notified())
                .await.expect("accepted close must complete replacement B's lifetime");
            assert!(dropped.load(Ordering::SeqCst));
        }
    });
}

#[test]
fn s6_close_transport_or_parse_failure_retains_attachment() {
    tauri::async_runtime::block_on(async {
        for error in ["connect failed", "connection closed after send", "invalid JSON response"] {
            let f = Fixture::new();
            assert!(call(&f, Err(error.into())).await.is_err());
            assert!(f.attached(), "transport/parse failure is not confirmed close: {error}");
        }
    });
}

#[test]
fn s6_close_confirmed_exact_sid_detaches_and_aborts() {
    tauri::async_runtime::block_on(async {
        let f = Fixture::new();
        assert!(call(&f, Ok(confirmed(SID))).await.is_ok());
        assert!(!f.attached(), "confirmed close must remove its attachment");
        f.assert_aborted().await;
    });
}

#[test]
fn s6_close_exact_not_found_is_already_gone_even_with_arbitrary_message() {
    tauri::async_runtime::block_on(async {
        let f = Fixture::new();
        let result = call(&f, Ok(json!({"ok": false, "error": {
            "code": "not_found", "message": "arbitrary message with no recognizable words"}}))).await;
        assert!(result.is_ok(), "Q8 exact error.code=not_found must permit UI cleanup: {result:?}");
        assert!(!f.attached()); f.assert_aborted().await;
    });
}

#[test]
fn s6_close_not_found_message_bait_never_normalizes_another_error() {
    tauri::async_runtime::block_on(async {
        for code in ["close_denied", "close_failed", "invalid_params", "method_not_found", "other"] {
            let f = Fixture::new();
            let result = call(&f, Ok(json!({"ok": false, "error": {
                "code": code, "message": "not_found: surface not found"}}))).await;
            assert!(result.is_err(), "message substring must not become target disappearance: {code}");
            assert!(f.attached(), "message bait must retain attachment: {code}");
        }
    });
}

#[test]
fn s6_close_malformed_positive_or_mismatched_sid_retains_attachment() {
    tauri::async_runtime::block_on(async {
        let replies = [
            json!({"ok": true}),
            json!({"ok": true, "result": null}),
            json!({"ok": true, "result": {"surface_id": SID}}),
            json!({"ok": true, "result": {"surface_id": SID, "closed": false}}),
            json!({"ok": true, "result": {"surface_id": SID, "closed": "true"}}),
            json!({"ok": true, "result": {"surface_id": SID + 1, "closed": true}}),
            json!({"ok": true, "result": {"surface_id": "42", "closed": true}}),
            json!({"ok": true, "result": {"surface_id": 42.5, "closed": true}}),
            json!({"ok": true, "result": {"closed": true}}),
            json!({"ok": "true", "result": {"surface_id": SID, "closed": true}}),
            json!({"result": {"surface_id": SID, "closed": true}}),
        ];
        for reply in replies {
            let f = Fixture::new();
            let result = call(&f, Ok(reply.clone())).await;
            assert!(result.is_err(), "positive envelope must confirm exact numeric SID and closed=true: {reply}");
            assert!(f.attached(), "wrong/missing close confirmation must preserve attachment: {reply}");
        }
    });
}

#[test]
fn s6_close_not_found_requires_explicit_false_envelope_and_exact_code() {
    tauri::async_runtime::block_on(async {
        let replies = [
            json!({"error": {"code": "not_found", "message": "gone"}}),
            json!({"ok": "false", "error": {"code": "not_found", "message": "gone"}}),
            json!({"ok": true, "error": {"code": "not_found", "message": "gone"}}),
            json!({"ok": false, "error": {"code": "not_found_extra", "message": "gone"}}),
            json!({"ok": false, "error": {"message": "not_found"}}),
            json!({"ok": false}),
            json!(null),
        ];
        for reply in replies {
            let f = Fixture::new();
            assert!(call(&f, Ok(reply.clone())).await.is_err(), "unconfirmed envelope: {reply}");
            assert!(f.attached(), "unconfirmed envelope must preserve attachment: {reply}");
        }
    });
}

#[test]
fn s6_close_selected_socket_equal_sid_does_not_detach_another_stream() {
    tauri::async_runtime::block_on(async {
        let f = Fixture::new();
        let other = PathBuf::from("s6-inert-other-socket");
        let other_key = (sock_slug(&other), SID);
        let handle = tauri::async_runtime::spawn(std::future::pending::<()>());
        f.state.0.lock().unwrap().insert(other_key.clone(), handle);
        assert!(call(&f, Ok(confirmed(SID))).await.is_ok());
        assert!(!f.attached());
        assert!(f.state.0.lock().unwrap().contains_key(&other_key), "equal SID at another socket must remain attached");
    });
}

#[test]
fn s6_close_reads_selected_token_fresh_and_attaches_only_owner_key() {
    tauri::async_runtime::block_on(async {
        let f = Fixture::new();
        let reads = AtomicUsize::new(0);
        let seen = Mutex::new(Vec::<(PathBuf, Value)>::new());
        for expected in ["SYNTHETIC-FIRST", "SYNTHETIC-ROTATED"] {
            let result = close_surface_with(&f.state, &f.socket, SID,
                |socket: &Path| {
                    assert_eq!(socket, f.socket.as_path(), "read the selected daemon's credential");
                    reads.fetch_add(1, Ordering::SeqCst);
                    Some(expected.to_owned())
                },
                |socket, params| {
                    seen.lock().unwrap().push((socket, params));
                    // Keep the stream for the second request while exercising token rotation.
                    ready(Ok(json!({"ok": false, "error": {"code": "close_denied", "message": "fixture"}})))
                }).await;
            assert!(result.is_err());
            let sent = seen.lock().unwrap();
            let (socket, params) = sent.last().unwrap();
            assert_eq!(socket, &f.socket);
            assert_eq!(params["surface_id"], json!(SID));
            assert_eq!(params["owner_token"], json!(expected), "close must attach the fresh owner credential");
            assert!(params.get("operator_token").is_none(), "close may not attach broad operator exemptions");
        }
        assert_eq!(reads.load(Ordering::SeqCst), 2, "each call must read once without a token cache");
    });
}

#[test]
fn s6_close_missing_or_empty_token_never_falls_back_to_other_credentials() {
    tauri::async_runtime::block_on(async {
        for token in [None, Some(String::new())] {
            let f = Fixture::new();
            let sent = Mutex::new(None);
            let result = close_surface_with(&f.state, &f.socket, SID, |_socket| token,
                |socket, params| {
                    *sent.lock().unwrap() = Some((socket, params));
                    ready(Ok(json!({"ok": false, "error": {"code": "close_denied", "message": "missing credential"}})))
                }).await;
            assert!(result.is_err());
            let sent = sent.lock().unwrap(); let (_, params) = sent.as_ref().unwrap();
            assert!(params.get("owner_token").is_none());
            assert!(params.get("operator_token").is_none());
            assert!(f.attached());
        }
    });
}

#[test]
fn s6_close_only_finite_pane_reason_emits_specific_relaunch_message() {
    tauri::async_runtime::block_on(async {
        let f = Fixture::new();
        let result = call(&f, Ok(json!({"ok": false, "error": {
            "code": "close_denied", "reason": "pane_ancestor_running", "message": "opaque"}}))).await;
        let message = result.expect_err("positive veto must reject close");
        let launcher = match std::env::consts::OS {
            "windows" => "Windows Explorer or the Start menu",
            "macos" => "Finder",
            _ => "your desktop launcher",
        };
        assert_eq!(message, format!("Close refused because this app has a live pane ancestor in the selected daemon. Relaunch it from {launcher}, then try again."));
        assert!(f.attached());
    });
}

#[test]
fn s6_close_generic_denial_never_invents_pane_ancestry() {
    tauri::async_runtime::block_on(async {
        let replies = [
            json!({"ok": false, "error": {"code": "close_denied", "message": "pane_ancestor_running is only text"}}),
            json!({"ok": false, "error": {"code": "close_denied", "reason": "peer_unavailable", "message": "peer missing"}}),
            json!({"ok": false, "error": {"code": "close_denied", "reason": "unknown", "message": "unproven"}}),
            json!({"ok": false, "error": {"code": "other", "reason": "pane_ancestor_running", "message": "not a close denial"}}),
        ];
        for reply in replies {
            let f = Fixture::new();
            let message = call(&f, Ok(reply.clone())).await.expect_err("refusal must propagate");
            assert!(!message.contains(RELAUNCH), "generic error must not invent positive ancestry: {reply}");
            assert!(f.attached());
        }
    });
}

#[test]
fn s6_close_tauri_wrapper_uses_the_exercised_operation_and_full_reply() {
    let source = include_str!("main.rs");
    let start = source.find("async fn close_surface(").expect("close command exists");
    let end = source[start..].find("\n}\n").expect("close command end") + start;
    let wrapper = &source[start..end];
    // Supplemental wiring check, not a replacement for the operation assertions.
    assert!(wrapper.contains("close_surface_with("), "Tauri must execute the tested operation");
    assert!(wrapper.contains("rpc_full("), "exact error.code must survive to the tested operation");
    assert!(wrapper.contains("read_operator_token_for"), "production token reader must be bound to the operation");
    assert!(!wrapper.contains("handle.abort()"), "wrapper must not detach before the tested operation");
}
