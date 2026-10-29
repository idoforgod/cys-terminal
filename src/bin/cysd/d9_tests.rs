//! ★(0.14.56 · P1 D9 · design r3 §5, §8, §12; head ruling 646b2a36 + CEO amendment 52facf64) Master/CEO composers:
//! RPC-level specimens for the send conversion, the pair key, the Return arm and the creator boot-gate carve-out.
//!
//! Write axis (as in `return_absorb_tests`): a handler that hands a PTY write to the writer always moves `input_gen`,
//! so "generation unchanged" is the deterministic witness that a request wrote nothing.
//!
//! These specimens call only entry points that exist at 9da2418d (dispatch, create_surface, caller_cache, create_caller,
//! agent_status, return_tickets, feed_items, pending_queue), so this file applied to the pin tree is the RED run.
#![cfg(test)]

use crate::handlers::{dispatch, Reply};
use crate::state::{Daemon, Surface};
use cys::Request;
use serde_json::{json, Value};
use std::io::BufRead;
use std::sync::atomic::Ordering;
use std::sync::{Arc, MutexGuard};

struct Fx {
    daemon: Arc<Daemon>,
    dir: std::path::PathBuf,
    _g: MutexGuard<'static, ()>,
}

impl Drop for Fx {
    fn drop(&mut self) {
        crate::state::test_stop_daemon(&self.daemon);
        std::env::remove_var(cys::pack::ENV_PACK_DIR);
        std::env::remove_var("CYS_RETURN_ABSORB_SECS");
        std::env::remove_var("CYS_RETURN_ABSORB_REFLEX_MS");
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn fx(tag: &str) -> Fx {
    let g = crate::governance::PACK_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    crate::delivery::tests::isolate_state_dir_for_thread(tag);
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("cys-d9-{tag}-{}-{n}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    std::fs::write(dir.join("acl.json"), r#"{"default":"allow","rules":[]}"#).unwrap();
    std::env::set_var(cys::pack::ENV_PACK_DIR, &dir);
    std::env::remove_var("CYS_RETURN_ABSORB_SECS");
    std::env::remove_var("CYS_RETURN_ABSORB_REFLEX_MS");
    let daemon = Daemon::new(dir.join("cysd.sock"));
    Fx { daemon, dir, _g: g }
}

/// Synthetic kernel-peer pids: above the macOS pid ceiling (99,999), so no live process can own one.
const P: u32 = 997_300;

fn seat(fx: &Fx, role: &str) -> Arc<Surface> {
    let s = fx
        .daemon
        .create_surface(None, Some("sleep 30".into()), None, Some(role.into()), 24, 80)
        .expect("create surface");
    fx.daemon.surfaces.lock().unwrap().insert(s.id, s.clone());
    s
}

/// A master seat in `d9_scope`: role `master` in this daemon and an agent seat (claude meta, seat state not Empty).
fn master(fx: &Fx) -> Arc<Surface> {
    let s = seat(fx, "master");
    fx.daemon.roles.lock().unwrap().insert("master".into(), s.id);
    *s.agent_meta.lock().unwrap() = Some(("claude".into(), "claude".into()));
    s
}

fn bind(fx: &Fx, pid: u32, sid: u64) {
    fx.daemon.caller_cache.lock().unwrap().insert(
        pid,
        crate::state::CallerCacheEntry::new(Some(sid), crate::state::now_epoch(), None, fx.daemon.caller_gen.load(Ordering::Relaxed)),
    );
}

/// A caller pane (role `worker-N`) resolved from `pid`.
fn pane(fx: &Fx, role: &str, pid: u32) -> Arc<Surface> {
    let s = seat(fx, role);
    bind(fx, pid, s.id);
    s
}

fn rpc(fx: &Fx, pid: Option<u32>, method: &str, params: Value) -> Value {
    let req = Request { id: json!(42), method: method.into(), params };
    let Reply::Single(resp) = dispatch(&fx.daemon, req, pid) else {
        panic!("expected single reply");
    };
    resp
}

fn send(fx: &Fx, pid: Option<u32>, t: &Arc<Surface>, text: &str, extra: Value) -> Value {
    let mut p = json!({"surface_id": t.id, "text": text, "quiet": true});
    if let (Some(m), Some(e)) = (p.as_object_mut(), extra.as_object()) {
        m.extend(e.clone());
    }
    rpc(fx, pid, "surface.send_text", p)
}

fn ret(fx: &Fx, pid: Option<u32>, t: &Arc<Surface>, extra: Value) -> Value {
    let mut p = json!({"surface_id": t.id, "key": "Return"});
    if let (Some(m), Some(e)) = (p.as_object_mut(), extra.as_object()) {
        m.extend(e.clone());
    }
    rpc(fx, pid, "surface.send_key", p)
}

fn gen(s: &Arc<Surface>) -> u64 {
    s.input_gen.load(Ordering::Acquire)
}

fn queue_texts(s: &Arc<Surface>) -> Vec<String> {
    s.pending_queue.lock().unwrap().iter().map(|e| e.text.clone()).collect()
}

fn held(s: &Arc<Surface>) -> usize {
    s.submit_assoc.lock().unwrap_or_else(|e| e.into_inner()).len()
}

fn ledger_lines(fx: &Fx) -> usize {
    std::fs::read_to_string(crate::delivery::ledger_path(&fx.daemon.socket_path))
        .map(|s| s.lines().filter(|l| !l.trim().is_empty()).count())
        .unwrap_or(0)
}

fn assert_converted(resp: &Value) {
    assert_eq!(resp["ok"], json!(true), "a non-owner send into master must succeed as ONE queue job: {resp}");
    let r = &resp["result"];
    assert_eq!(r["queued"], json!(true), "{resp}");
    assert_eq!(r["converted"], json!("one_job"), "{resp}");
    assert_eq!(r["sent"], json!(true), "{resp}");
}

fn assert_denied(resp: &Value) {
    assert_eq!(resp["ok"], json!(false), "must be refused: {resp}");
    assert_eq!(resp["error"]["code"], json!("composer_key_denied"), "{resp}");
}

fn assert_absorbed(resp: &Value) {
    assert_eq!(resp["ok"], json!(true), "{resp}");
    assert_eq!(resp["result"]["absorbed"], json!(true), "must be absorbed (0 bytes): {resp}");
}

/// A child process of this test process (its parent incarnation = this process). Killed on drop.
struct Kid(std::process::Child);
impl Drop for Kid {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn kid() -> Kid {
    Kid(std::process::Command::new("/bin/sleep").arg("30").spawn().expect("spawn sleep"))
}

/// A grandchild: `sleep` under an intermediate shell, so its parent is NOT this process. Returns (shell, sleep pid).
struct Grandkid(Kid, u32);
impl Drop for Grandkid {
    fn drop(&mut self) {
        let _ = std::process::Command::new("/bin/kill").arg("-9").arg(self.1.to_string()).status();
    }
}
fn grandkid() -> Grandkid {
    let mut c = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg("/bin/sleep 30 & echo $!; wait")
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("spawn sh");
    let mut line = String::new();
    std::io::BufReader::new(c.stdout.take().unwrap()).read_line(&mut line).unwrap();
    let pid: u32 = line.trim().parse().expect("grandchild pid");
    Grandkid(Kid(c), pid)
}

fn record_creator(fx: &Fx, sid: u64, pid: u32, start: Option<u64>, age_secs: f64) {
    fx.daemon
        .create_caller
        .lock()
        .unwrap()
        .insert(sid, (pid, start, crate::state::now_epoch() - age_secs));
}

// ─────────────────────────── conversion (T-S1..S3, fence, control-only) ───────────────────────────

/// T-S1: master inside its 3 s typing guard; a non-owner direct send -> converted, one queue item, 0 bytes.
/// Pin: `ERR_TYPING_GUARD` (the RPC caller must fall back itself). Mutant: convert at the direct intents (after the guard).
#[test]
fn d9_s1_send_into_master_in_its_typing_guard_is_converted() {
    let fx = fx("s1");
    let m = master(&fx);
    let _w = pane(&fx, "worker-1", P);
    *m.last_human_input.lock().unwrap() = Some(std::time::Instant::now());
    let g0 = gen(&m);
    let r = send(&fx, Some(P), &m, "[보고] stage C", json!({}));
    assert_converted(&r);
    assert_eq!(queue_texts(&m), vec!["[보고] stage C".to_string()]);
    assert_eq!(gen(&m), g0, "the conversion writes nothing at receipt");
}

/// T-S2: a converted send writes NO delivery-ledger record at receipt (its one record is written at delivery,
/// `Origin::Queue`, by the unchanged queue path). Pin: an `Origin::Send` record at receipt (handlers.rs:6976), then a
/// second one at delivery. Mutant: convert after the send audit.
#[test]
fn d9_s2_converted_send_is_not_audited_at_receipt() {
    let fx = fx("s2");
    let m = master(&fx);
    let _w = pane(&fx, "worker-1", P + 1);
    let before = ledger_lines(&fx);
    let r = send(&fx, Some(P + 1), &m, "[보고] once", json!({}));
    assert_converted(&r);
    assert_eq!(ledger_lines(&fx), before, "no Origin::Send record at receipt");
    assert_eq!(m.pending_queue.lock().unwrap().front().map(|e| e.origin.clone()), Some("send".into()));
}

/// T-S3: a body ending in `\n` -> ONE queue item without the trailing newline (the job's CR is the submit) and NO pair
/// ticket (no follow-up Return is expected), so a later Return from the same caller is refused, 0 bytes.
/// Pin: SelfSubmitting direct write. Mutant: keep the trailing newline / issue a ticket from the stripped text.
#[test]
fn d9_s3_self_submitting_body_becomes_one_item_without_a_ticket() {
    let fx = fx("s3");
    let m = master(&fx);
    let _w = pane(&fx, "worker-1", P + 2);
    let g0 = gen(&m);
    let r = send(&fx, Some(P + 2), &m, "[보고] line\r\n", json!({}));
    assert_converted(&r);
    assert_eq!(r["result"]["return_absorb"], json!(false), "a self-submitting send gets no ticket: {r}");
    assert_eq!(queue_texts(&m), vec!["[보고] line".to_string()]);
    let k = ret(&fx, Some(P + 2), &m, json!({"pair_return": true}));
    assert_denied(&k);
    assert_eq!(gen(&m), g0, "0 bytes for both requests");
}

/// Head ruling condition 3: a body the CLI already fenced (`inject_text`) is queued fenced exactly ONCE - the queue job
/// adds the one fence. Mutant: enqueue the received text as-is (double fence).
#[test]
fn d9_fenced_body_is_queued_without_its_outer_fence() {
    let fx = fx("fence");
    let m = master(&fx);
    let _w = pane(&fx, "worker-1", P + 3);
    let wrapped = cys::paste_fence::wrap("[CYCLE] save now\n");
    let r = send(&fx, Some(P + 3), &m, &wrapped, json!({"authoritative": true}));
    assert_converted(&r);
    let q = queue_texts(&m);
    assert_eq!(q, vec!["[CYCLE] save now".to_string()]);
    assert!(!q[0].contains(cys::paste_fence::OPEN) && !q[0].contains(cys::paste_fence::CLOSE), "{q:?}");
    assert_eq!(r["result"]["return_absorb"], json!(true), "the fence's inner newline is not a self-submit: {r}");
}

/// r2 §10-4: control-only text into master is refused at receipt, 0 bytes, nothing queued (an empty item would be a
/// bare CR). Pin: written raw (the CR submits whatever the composer holds). Mutant: convert it.
#[test]
fn d9_control_only_text_into_master_is_refused() {
    let fx = fx("ctl");
    let m = master(&fx);
    let _w = pane(&fx, "worker-1", P + 4);
    let g0 = gen(&m);
    for t in ["\r", "", "\x1b[A", "\u{3}", "\x1b]0;t\x07\r\n"] {
        let r = send(&fx, Some(P + 4), &m, t, json!({}));
        assert_denied(&r);
    }
    let r = send(&fx, Some(P + 4), &m, "\r", json!({"queued": true}));
    assert_denied(&r);
    assert_eq!(gen(&m), g0);
    assert!(queue_texts(&m).is_empty());
}

// ─────────────────────────── the Return arm (T-A2..A6) ───────────────────────────

/// T-A2: a converted send's own Return is absorbed (body still queued); a second Return, another caller's Return and a
/// queued Return without a ticket are refused at receipt - 0 bytes, no empty `send-key` item is queued.
/// Pin: the queued Return enqueues an empty item; untokened Returns are written. Mutant: drop the ticket at conversion.
#[test]
fn d9_a2_own_return_absorbed_every_other_return_refused() {
    let fx = fx("a2");
    let m = master(&fx);
    let _a = pane(&fx, "worker-1", P + 10);
    let _b = pane(&fx, "worker-2", P + 11);
    let g0 = gen(&m);
    assert_converted(&send(&fx, Some(P + 10), &m, "[보고] A", json!({})));
    assert_absorbed(&ret(&fx, Some(P + 10), &m, json!({"pair_return": true})));
    assert_denied(&ret(&fx, Some(P + 10), &m, json!({"pair_return": true})));
    assert_denied(&ret(&fx, Some(P + 11), &m, json!({})));
    assert_denied(&ret(&fx, Some(P + 11), &m, json!({"queued": true})));
    assert_eq!(queue_texts(&m), vec!["[보고] A".to_string()], "no empty send-key item");
    assert_eq!(gen(&m), g0);
}

/// T-A3: the `push_line` shape - send and Return from two different children of ONE process - is absorbed through
/// `PairKey::Process` (the parent incarnation); a process with another parent gets refused. Mutant: key by `Claimed`.
#[test]
fn d9_a3_two_children_of_one_parent_share_the_process_key() {
    let fx = fx("a3");
    let m = master(&fx);
    let (k1, k2) = (kid(), kid());
    let other = grandkid();
    let g0 = gen(&m);
    let s = send(&fx, Some(k1.0.id()), &m, "[escalate] once", json!({"from": "surface:7"}));
    assert_converted(&s);
    assert_eq!(s["result"]["return_absorb"], json!(true), "a kernel-peer parent incarnation earns a ticket: {s}");
    assert_denied(&ret(&fx, Some(other.1), &m, json!({"from": "surface:7"})));
    assert_absorbed(&ret(&fx, Some(k2.0.id()), &m, json!({"from": "surface:7", "pair_return": true})));
    assert_eq!(gen(&m), g0);
}

/// T-A4: a live ticket aged 3 s and a live approval on master -> still absorbed, 0 bytes (no approval narrowing in scope;
/// a modal is answered only by a frame-bound key). Pin: `narrow_absorb_for_approval` writes it. Mutant: apply the narrowing.
#[test]
fn d9_a4_aged_ticket_with_live_approval_is_absorbed() {
    let fx = fx("a4");
    let m = master(&fx);
    let a = pane(&fx, "worker-1", P + 20);
    assert_converted(&send(&fx, Some(P + 20), &m, "[보고] A4", json!({})));
    {
        let mut t = m.return_tickets.lock().unwrap();
        let tk = t.get_mut(&crate::state::PairKey::Verified(a.id)).expect("ticket");
        tk.issued = std::time::Instant::now() - std::time::Duration::from_secs(3);
    }
    push_daemon_approval(&fx, m.id);
    let g0 = gen(&m);
    assert_absorbed(&ret(&fx, Some(P + 20), &m, json!({"pair_return": true})));
    assert_eq!(gen(&m), g0);
}

/// T-A5: a partial machine body on master (pending > 0, human 0) and the caller's ticketed Return -> 0 bytes; a bare CR
/// never submits a held partial body (R-3 governs its release). Pin: `PassThrough` writes the Return.
/// Mutant: apply `PassThrough` in scope. (The ticket comes from the `--queued` + `absorb_return` form, which exists at the pin.)
#[test]
fn d9_a5_partial_machine_body_is_not_submitted_by_a_bare_return() {
    let fx = fx("a5");
    let m = master(&fx);
    let _a = pane(&fx, "worker-1", P + 30);
    let q = send(&fx, Some(P + 30), &m, "[보고] A5", json!({"queued": true, "absorb_return": true}));
    assert_eq!(q["result"]["return_absorb"], json!(true), "{q}");
    m.set_pending_input(12);
    let g0 = gen(&m);
    assert_absorbed(&ret(&fx, Some(P + 30), &m, json!({"pair_return": true})));
    assert_eq!(gen(&m), g0);
}

/// T-A6: a caller known only by `from:"cli:send"` (no pane, no observable parent) gets no ticket; its body is still
/// converted once and its Return is refused. Pin: `request_pair_key` reads `params.from`. Mutant: read `params.from` in scope.
#[test]
fn d9_a6_label_only_caller_gets_no_ticket() {
    let fx = fx("a6");
    let m = master(&fx);
    let g0 = gen(&m);
    let s = send(&fx, Some(P + 40), &m, "[보고] A6", json!({"from": "cli:send"}));
    assert_converted(&s);
    assert_eq!(s["result"]["return_absorb"], json!(false), "{s}");
    assert_denied(&ret(&fx, Some(P + 40), &m, json!({"from": "cli:send", "pair_return": true})));
    let s2 = send(&fx, Some(P + 41), &m, "[보고] A6b", json!({"from": "surface:9"}));
    assert_eq!(s2["result"]["return_absorb"], json!(false), "a self-declared seat number earns no ticket: {s2}");
    assert_denied(&ret(&fx, Some(P + 42), &m, json!({"from": "surface:9", "pair_return": true})));
    assert_eq!(queue_texts(&m).len(), 2);
    assert_eq!(gen(&m), g0);
}

fn push_daemon_approval(fx: &Fx, sid: u64) {
    fx.daemon.feed_items.lock().unwrap().push(crate::state::FeedItem {
        request_id: format!("{}d9-approval", crate::state::DAEMON_REQ_PREFIX),
        kind: "approval".into(),
        title: "Allow execution".into(),
        body: "Allow execution of `cargo test`?".into(),
        surface_id: Some(sid),
        status: "pending".into(),
        decision: None,
        created_at: crate::state::now_epoch(),
        resolved_at: None,
        tier: None,
        publisher_pid: None,
        publisher_pgid: None,
        publisher_surface: None,
        risk_class: None,
        auto_route: false,
        resolver_surface: None,
        resolver_pid: None,
    });
}

// ─────────────────────────── the CEO locked test (T-CEO, design r3 §8) ───────────────────────────
// Failure direction declared first (CEO 2230 §3): YES => REVISE if any cell types a body after a Return its supplier did
// not send. PASS only if every such cell writes 0 bytes AND the control P-0 submits its body exactly once (one queue job).

/// P-0 control: A's two-step into master -> one queue job carrying the body, the Return absorbed, nothing held.
#[test]
fn d9_ceo_p0_control_two_step_is_one_job() {
    let fx = fx("ceo-p0");
    let m = master(&fx);
    let _a = pane(&fx, "worker-1", P + 50);
    let g0 = gen(&m);
    assert_converted(&send(&fx, Some(P + 50), &m, "[보고] P-0", json!({})));
    assert_absorbed(&ret(&fx, Some(P + 50), &m, json!({"pair_return": true})));
    assert_eq!(queue_texts(&m), vec!["[보고] P-0".to_string()], "the body exists once, in the job that types it");
    assert_eq!(held(&m), 0);
    assert_eq!(gen(&m), g0);
}

/// S-OP-1 token: A supplies a body; ANOTHER caller sends Return carrying A's request token. Pin: YES (bound:"token",
/// measured r2 F18). r3: nothing is held, the other caller has no ticket -> refused, 0 bytes. Mutant: hold for master.
#[test]
fn d9_ceo_sop1_token_from_another_caller_types_nothing() {
    let fx = fx("ceo-1");
    let m = master(&fx);
    let _a = pane(&fx, "worker-1", P + 60);
    let _c = pane(&fx, "worker-2", P + 61);
    let s = send(&fx, Some(P + 60), &m, "[보고] S-OP-1", json!({}));
    let token = s["result"]["request_id"].as_str().unwrap_or("no-token-issued").to_string();
    let g0 = gen(&m);
    let k = ret(&fx, Some(P + 61), &m, json!({"request_id": token}));
    assert_denied(&k);
    assert_eq!(gen(&m), g0, "YES = a body typed by a Return its supplier did not send");
    assert_eq!(held(&m), 0);
}

/// S-OP-2 claimed: C1 (no pane, claims seat 7) supplies a body; C3 (another process, also claims 7) sends Return.
/// Pin: YES (bound:"legacy", measured). r3: nothing held, claimed callers get no ticket -> refused. Mutant: key by `Claimed`.
#[test]
fn d9_ceo_sop2_claimed_return_from_another_process_types_nothing() {
    let fx = fx("ceo-2");
    let m = master(&fx);
    let _ = send(&fx, Some(P + 70), &m, "[보고] S-OP-2", json!({"from": "surface:7"}));
    let g0 = gen(&m);
    let k = ret(&fx, Some(P + 71), &m, json!({"from": "surface:7", "pair_return": true}));
    assert_denied(&k);
    assert_eq!(gen(&m), g0);
    assert_eq!(held(&m), 0);
}

/// S-OP-3 modal: A supplies a body while a permission modal is up, then A's Return. Pin: association binds the held body
/// first and types it into the modal (predicted YES). r3: nothing to bind; A's Return is absorbed by its own ticket, the
/// modal is untouched (answering it is the frame-bound key, stage E). Mutant: associate before the scope arm.
#[test]
fn d9_ceo_sop3_return_on_a_modal_binds_nothing() {
    let fx = fx("ceo-3");
    let m = master(&fx);
    let _a = pane(&fx, "worker-1", P + 80);
    let modal = cys::first_run_gates::fixtures::LIVE_PERMISSION_PROMPT.replace('\n', "\r\n");
    {
        let mut p = m.parser.lock().unwrap();
        p.process(b"\x1b[2J\x1b[H");
        p.process(modal.as_bytes());
    }
    let _ = send(&fx, Some(P + 80), &m, "[보고] S-OP-3", json!({}));
    let g0 = gen(&m);
    let k = ret(&fx, Some(P + 80), &m, json!({"pair_return": true}));
    assert_eq!(gen(&m), g0, "no body typed into the modal: {k}");
    assert_eq!(held(&m), 0);
}

/// S-OP-9 same seat: A's earlier body, then A's next send while master is in its typing guard, then A's Return.
/// Pin: the first body is held, the second refused by the guard, and the Return binds the EARLIER body (predicted YES).
/// r3: both sends are queue jobs; the Return is absorbed; nothing is typed by the Return. Mutant: re-enable holding.
#[test]
fn d9_ceo_sop9_same_seat_return_binds_no_earlier_body() {
    let fx = fx("ceo-9");
    let m = master(&fx);
    let _a = pane(&fx, "worker-1", P + 90);
    let _ = send(&fx, Some(P + 90), &m, "[보고] first", json!({}));
    *m.last_human_input.lock().unwrap() = Some(std::time::Instant::now());
    let _ = send(&fx, Some(P + 90), &m, "[보고] second", json!({}));
    *m.last_human_input.lock().unwrap() = None;
    let g0 = gen(&m);
    let _ = ret(&fx, Some(P + 90), &m, json!({"pair_return": true}));
    assert_eq!(gen(&m), g0, "the Return typed nothing");
    assert_eq!(held(&m), 0);
}

// ─────────────────────────── creator boot-gate carve-out (head 646b2a36, CEO 52facf64) ───────────────────────────

fn paint_trust(m: &Arc<Surface>) {
    let screen = cys::first_run_gates::fixtures::FOLDER_TRUST.replace('\n', "\r\n");
    let mut p = m.parser.lock().unwrap();
    p.process(b"\x1b[2J\x1b[H");
    p.process(screen.as_bytes());
}

fn set_status_from_the_seat(fx: &Fx, m: &Arc<Surface>, pid: u32) {
    bind(fx, pid, m.id);
    let r = rpc(fx, Some(pid), "status.set", json!({"surface_id": m.id, "state": "waiting"}));
    assert_eq!(r["ok"], json!(true), "{r}");
}

/// (a) the CREATOR (same pid and start time, not a pane) answering the folder-trust gate of the master it just launched,
/// no turn has reached the agent yet -> ACCEPTED through the pin path (written). A literal r3 arm refuses it: the boot
/// chain of a new department master would stall at the prompt. Mutant: drop `d9_boot_creator` (refuse everyone).
#[test]
fn d9_creator_a_trust_confirm_of_a_new_master_is_accepted() {
    let fx = fx("cr-a");
    let m = master(&fx);
    let me = kid();
    let pid = me.0.id();
    record_creator(&fx, m.id, pid, crate::state::peer_start_time(pid), 1.0);
    paint_trust(&m);
    let g0 = gen(&m);
    let r = ret(&fx, Some(pid), &m, json!({"authoritative": true}));
    assert_eq!(r["ok"], json!(true), "the launching operation answers its own boot gate: {r}");
    assert_eq!(r["result"]["sent"], json!(true), "{r}");
    assert!(gen(&m) > g0, "the confirm was written");
}

/// (a, directive half) + head condition 3: the creator's own directive is still converted (one job, fenced once) and
/// its own Return is absorbed by its ticket - the carve-out never writes a bare CR after a converted body.
#[test]
fn d9_creator_a_directive_is_converted_and_its_return_absorbed() {
    let fx = fx("cr-a2");
    let m = master(&fx);
    let me = kid();
    let pid = me.0.id();
    record_creator(&fx, m.id, pid, crate::state::peer_start_time(pid), 1.0);
    let g0 = gen(&m);
    let s = send(&fx, Some(pid), &m, &cys::paste_fence::wrap("[directive] read me"), json!({"authoritative": true}));
    assert_converted(&s);
    assert_eq!(queue_texts(&m), vec!["[directive] read me".to_string()]);
    assert_absorbed(&ret(&fx, Some(pid), &m, json!({"authoritative": true})));
    assert_eq!(gen(&m), g0);
}

/// (b) a different process of the same uid sending the same Return -> REFUSED, 0 bytes.
#[test]
fn d9_creator_b_another_process_is_refused() {
    let fx = fx("cr-b");
    let m = master(&fx);
    let (me, other) = (kid(), kid());
    record_creator(&fx, m.id, me.0.id(), crate::state::peer_start_time(me.0.id()), 1.0);
    paint_trust(&m);
    let g0 = gen(&m);
    assert_denied(&ret(&fx, Some(other.0.id()), &m, json!({"authoritative": true})));
    assert_eq!(gen(&m), g0);
}

/// (c, as amended by CEO 52facf64) the bound is the seat STATE, not an age: an old ledger entry with no turn yet is still
/// the creator (accepted), and the creator once a turn has reached the agent is refused even 1 s after creation.
/// Mutant: key the carve-out on the 30-minute age (the old entry refuses / the fresh one accepts).
#[test]
fn d9_creator_c_state_not_age_decides() {
    let fx = fx("cr-c");
    let m = master(&fx);
    let me = kid();
    let pid = me.0.id();
    record_creator(&fx, m.id, pid, crate::state::peer_start_time(pid), 31.0 * 60.0);
    paint_trust(&m);
    assert_eq!(ret(&fx, Some(pid), &m, json!({"authoritative": true}))["ok"], json!(true), "no turn yet: still new");
    let m2 = {
        let s = seat(&fx, "ceo");
        fx.daemon.roles.lock().unwrap().insert("ceo".into(), s.id);
        *s.agent_meta.lock().unwrap() = Some(("claude".into(), "claude".into()));
        s
    };
    record_creator(&fx, m2.id, pid, crate::state::peer_start_time(pid), 1.0);
    set_status_from_the_seat(&fx, &m2, P + 100);
    let g0 = gen(&m2);
    assert_denied(&ret(&fx, Some(pid), &m2, json!({"authoritative": true})));
    assert_eq!(gen(&m2), g0);
}

/// (d) a recycled pid: the recorded creator pid with a DIFFERENT start time -> REFUSED.
#[test]
fn d9_creator_d_recycled_pid_is_refused() {
    let fx = fx("cr-d");
    let m = master(&fx);
    let me = kid();
    let pid = me.0.id();
    let start = crate::state::peer_start_time(pid).expect("observable start time");
    record_creator(&fx, m.id, pid, Some(start + 1), 1.0);
    paint_trust(&m);
    let g0 = gen(&m);
    assert_denied(&ret(&fx, Some(pid), &m, json!({"authoritative": true})));
    assert_eq!(gen(&m), g0);
}

/// (e) the creator calling from inside a pane -> REFUSED (a pane caller is never the creator grade).
#[test]
fn d9_creator_e_creator_inside_a_pane_is_refused() {
    let fx = fx("cr-e");
    let m = master(&fx);
    let me = kid();
    let pid = me.0.id();
    record_creator(&fx, m.id, pid, crate::state::peer_start_time(pid), 1.0);
    let w = seat(&fx, "worker-1");
    bind(&fx, pid, w.id);
    paint_trust(&m);
    let g0 = gen(&m);
    assert_denied(&ret(&fx, Some(pid), &m, json!({"authoritative": true})));
    assert_eq!(gen(&m), g0);
}

/// (f) the cross-socket department launch shape: the launching CLI is not a pane in the department daemon and carries
/// only a display label -> it is the creator case and passes.
#[test]
fn d9_creator_f_cross_socket_launch_is_the_creator_case() {
    let fx = fx("cr-f");
    let m = master(&fx);
    let me = kid();
    let pid = me.0.id();
    record_creator(&fx, m.id, pid, crate::state::peer_start_time(pid), 1.0);
    paint_trust(&m);
    let r = ret(&fx, Some(pid), &m, json!({"authoritative": true, "from": "cli:launch"}));
    assert_eq!(r["ok"], json!(true), "{r}");
}

/// (g, CEO 52facf64 §3) the CREATOR's Return into a seat that has ALREADY reached ready (its agent reported) and HOLDS
/// TEXT in its composer -> REFUSED, 0 bytes: the residual of the carve-out is closed by the state.
#[test]
fn d9_creator_g_ready_seat_with_text_refuses_the_creator() {
    let fx = fx("cr-g");
    let m = master(&fx);
    let me = kid();
    let pid = me.0.id();
    record_creator(&fx, m.id, pid, crate::state::peer_start_time(pid), 1.0);
    set_status_from_the_seat(&fx, &m, P + 110);
    let h = rpc(&fx, None, "surface.send_text", json!({"surface_id": m.id, "text": "owner words", "human": true, "quiet": true}));
    assert_eq!(h["ok"], json!(true), "{h}");
    *m.last_human_input.lock().unwrap() = None;
    let g0 = gen(&m);
    assert_denied(&ret(&fx, Some(pid), &m, json!({"authoritative": true})));
    assert_eq!(gen(&m), g0);
}

/// (g, the discriminating cell) the same ready seat showing a DIALOG (a permission prompt the owner's session opened):
/// the pin path would press its default option (SubmitKey stays outside the modal axis), so only the state closes it.
/// Mutant: drop the `agent_status` test from `d9_boot_creator` -> the creator's Return is written onto the dialog.
#[test]
fn d9_creator_g_ready_seat_with_a_dialog_refuses_the_creator() {
    let fx = fx("cr-g3");
    let m = master(&fx);
    let me = kid();
    let pid = me.0.id();
    record_creator(&fx, m.id, pid, crate::state::peer_start_time(pid), 1.0);
    set_status_from_the_seat(&fx, &m, P + 120);
    let modal = cys::first_run_gates::fixtures::LIVE_PERMISSION_PROMPT.replace('\n', "\r\n");
    {
        let mut p = m.parser.lock().unwrap();
        p.process(b"\x1b[2J\x1b[H");
        p.process(modal.as_bytes());
    }
    let g0 = gen(&m);
    assert_denied(&ret(&fx, Some(pid), &m, json!({"authoritative": true})));
    assert_eq!(gen(&m), g0, "the dialog stays unpressed");
}

/// (g') the window between a usable composer and the first turn: the creator's untokened Return onto human-typed text
/// is not refused by D9 (no turn yet) - it reaches the pin path, whose draft gate refuses a submit over human bytes.
/// Measured here so the residual statement rests on a run, not on reading.
#[test]
fn d9_creator_g_prime_pin_draft_gate_covers_text_before_the_first_turn() {
    let fx = fx("cr-g2");
    let m = master(&fx);
    let me = kid();
    let pid = me.0.id();
    record_creator(&fx, m.id, pid, crate::state::peer_start_time(pid), 1.0);
    let h = rpc(&fx, None, "surface.send_text", json!({"surface_id": m.id, "text": "owner words", "human": true, "quiet": true}));
    assert_eq!(h["ok"], json!(true), "{h}");
    *m.last_human_input.lock().unwrap() = None;
    let g0 = gen(&m);
    let r = ret(&fx, Some(pid), &m, json!({"authoritative": true}));
    assert_eq!(r["ok"], json!(false), "the pin draft gate must refuse a submit over human text: {r}");
    assert_ne!(r["error"]["code"], json!("composer_key_denied"), "this refusal is the pin's, not D9's: {r}");
    assert_eq!(gen(&m), g0);
}

/// (h, head ruling 28f2b064) an IN-SEAT boot by a non-creator: the master surface was made by someone else (another
/// process, or no recorded creator at all - an empty shell from new-surface/GUI), a relauncher (restore in-seat,
/// node-recover) answers the folder-trust gate with no turn yet -> REFUSED, 0 bytes (residual, named; the CLI makes the
/// refusal loud: restore keeps the seat as gated, node-recover exits 79).
/// This is branch (h2) only: the fixture seat is agent-backed (seat state Unknown = before any tick, exit flag clear).
/// Which branch a real relaunch meets is decided by the watchdog tick - see `d9_residual_h_tick_phase_*` below.
#[test]
fn d9_residual_h_in_seat_boot_by_a_non_creator_is_refused() {
    let fx = fx("res-h");
    let m = master(&fx);
    let (maker, relauncher) = (kid(), kid());
    record_creator(&fx, m.id, maker.0.id(), crate::state::peer_start_time(maker.0.id()), 5.0);
    paint_trust(&m);
    let g0 = gen(&m);
    assert_denied(&ret(&fx, Some(relauncher.0.id()), &m, json!({"authoritative": true})));
    assert_eq!(gen(&m), g0);
    let m2 = {
        let s = seat(&fx, "ceo");
        fx.daemon.roles.lock().unwrap().insert("ceo".into(), s.id);
        *s.agent_meta.lock().unwrap() = Some(("claude".into(), "claude".into()));
        s
    };
    paint_trust(&m2);
    let g1 = gen(&m2);
    assert_denied(&ret(&fx, Some(relauncher.0.id()), &m2, json!({"authoritative": true})));
    assert_eq!(gen(&m2), g1, "no creator recorded at all (an empty shell from new-surface) - refused, 0 bytes");
}

// ─────────────────────────── (h1)/(h2): the tick phase decides an in-seat boot (head ruling on Q-D9-tick-lag) ───────────────────────────
//
// `d9_scope` needs `seat_is_agent_backed`, whose two inputs only the watchdog tick writes (every
// `WATCHDOG_INTERVAL_SECS`): `check_agent_death` sets `agent_exit_notified` when the agent dies and clears it when a live
// agent is seen; `refresh_seat_cache` writes `seat_cache`. After a relaunch into the same seat the death tick's values
// stand until the next tick, so for at most one tick period from agent start the seat is out of scope:
//   (h1) not yet observed -> a non-creator's Return/text takes the pin path (e2e: 8/8 trust confirms accepted);
//   (h2) observed -> in scope -> refused, 0 bytes (loud to the relauncher).
// The specimens set the two flags exactly as the tick writes them; nothing waits for a tick.

/// The two values the watchdog tick writes: `observed` = a tick that saw the (relaunched) agent alive; otherwise the death
/// tick's values. `seat_override` replaces only the seat half (single-flag cells for the mutants).
fn tick_flags(s: &Arc<Surface>, observed: bool, seat_override: Option<crate::governance::SeatState>) {
    use crate::governance::SeatState;
    s.agent_exit_notified.store(!observed, Ordering::Relaxed);
    let seat = seat_override.unwrap_or(if observed { SeatState::Occupied } else { SeatState::Empty });
    s.seat_cache.store(seat.as_u8(), Ordering::Relaxed);
}

/// A master the relauncher did not create, showing the folder-trust gate, with the given tick values.
fn relaunched_master(fx: &Fx, maker: &Kid, observed: bool, seat: Option<crate::governance::SeatState>) -> Arc<Surface> {
    let m = master(fx);
    record_creator(fx, m.id, maker.0.id(), crate::state::peer_start_time(maker.0.id()), 5.0);
    paint_trust(&m);
    tick_flags(&m, observed, seat);
    m
}

/// (h1) + (h2) on the trust gate, the real transition: the death tick's values (exit flag set, seat Empty) -> the
/// relauncher's confirm is WRITTEN (pin behaviour); the next tick's values (flag clear, seat Occupied) -> the same caller
/// on the same kind of seat is REFUSED with 0 bytes.
/// Mutants: drop `seat_is_agent_backed` from `d9_scope` -> (h1) refused; make the scope ignore both flags -> (h1) refused.
#[test]
fn d9_residual_h_tick_phase_decides_the_in_seat_boot_confirm() {
    use crate::governance::SeatState;
    let fx = fx("res-h12");
    let (maker, relauncher) = (kid(), kid());
    let h1 = relaunched_master(&fx, &maker, false, None);
    assert_eq!(SeatState::from_u8(h1.seat_cache.load(Ordering::Relaxed)), SeatState::Empty);
    let g0 = gen(&h1);
    let r = ret(&fx, Some(relauncher.0.id()), &h1, json!({"authoritative": true}));
    assert_eq!(r["ok"], json!(true), "(h1) not yet observed: out of d9_scope, the pin path writes the confirm: {r}");
    assert!(gen(&h1) > g0, "(h1) the confirm was written");
    let h2 = relaunched_master(&fx, &maker, true, None);
    let g1 = gen(&h2);
    assert_denied(&ret(&fx, Some(relauncher.0.id()), &h2, json!({"authoritative": true})));
    assert_eq!(gen(&h2), g1, "(h2) observed: refused, 0 bytes");
}

/// Each flag alone moves the seat out of scope (the two inputs of `seat_is_agent_backed`), and the before-any-tick
/// value (Unknown) counts as observed. Mutants: drop the exit-flag test -> cell 1 refused; drop the Empty test -> cell 2
/// refused; treat Unknown as Empty -> cell 3 written.
#[test]
fn d9_residual_h_tick_phase_each_flag_alone() {
    use crate::governance::SeatState;
    let fx = fx("res-h-flags");
    let (maker, relauncher) = (kid(), kid());
    for (cell, observed, seat, written) in [
        (1, false, Some(SeatState::Occupied), true), // exit flag set, seat seen occupied (agent died between the two writes)
        (2, true, Some(SeatState::Empty), true),     // exit flag clear, seat Empty
        (3, true, Some(SeatState::Unknown), false),  // flag clear, no seat probe yet (a new surface) -> in scope
    ] {
        let m = relaunched_master(&fx, &maker, observed, seat);
        let g0 = gen(&m);
        let r = ret(&fx, Some(relauncher.0.id()), &m, json!({"authoritative": true}));
        if written {
            assert_eq!(r["ok"], json!(true), "cell {cell}: out of d9_scope, pin path: {r}");
            assert!(gen(&m) > g0, "cell {cell}: written");
        } else {
            assert_denied(&r);
            assert_eq!(gen(&m), g0, "cell {cell}: 0 bytes");
        }
    }
}

/// The empty Claude composer a just-started agent draws (marker + U+00A0 as the CLI's glyph spacing, rules, status row).
fn paint_empty_box(m: &Arc<Surface>) {
    let rule = "\u{2500}".repeat(60);
    let screen = format!("\x1b[2J\x1b[H\u{273b} Welcome\r\n\r\n{rule}\r\n\u{276f}\u{a0}\r\n{rule}\r\n  ? for shortcuts\x1b[2A\r\x1b[2C");
    m.parser.lock().unwrap().process(screen.as_bytes());
}

/// (h1) is out of scope for TEXT too, and what a non-creator key can land on there (head ruling on Q-D9-tick-lag: name
/// any path to content the operation did not supply). Within the window a non-owner's direct send is NOT converted: the
/// pin path holds it (`held: awaiting_submit`, 0 bytes) for its own sender's Return. A foreign Return - the relauncher,
/// another process with another parent - writes one bare CR into the empty composer and leaves that held body unsent.
/// Queued deliveries are not a path: the queue holds a role seat whose seat state is Empty (`deliver_queued` empty_seat
/// block), and the same tick writes that state. In (h2) the same send converts to one queue job.
/// Senders: a pane (worker-1) and an off-pane process whose parent is not this process (a different pair key).
/// Mutant: let a foreign Return bind the held body -> held count 0.
#[test]
fn d9_residual_h1_text_is_held_for_its_sender_within_the_window() {
    let fx = fx("res-h1-text");
    let (maker, relauncher, offpane) = (kid(), kid(), grandkid());
    let _w = pane(&fx, "worker-1", P);
    for (label, sender) in [("pane", P), ("off-pane", offpane.1)] {
        let h1 = relaunched_master(&fx, &maker, false, None);
        paint_empty_box(&h1);
        let g0 = gen(&h1);
        let r = send(&fx, Some(sender), &h1, "[보고] window", json!({}));
        assert_eq!(r["ok"], json!(true), "{label} (h1): {r}");
        assert_ne!(r["result"]["converted"], json!("one_job"), "{label} (h1) not converted: {r}");
        assert_eq!(r["result"]["held"], json!("awaiting_submit"), "{label} (h1) the pin path holds the body: {r}");
        assert_eq!((gen(&h1), held(&h1)), (g0, 1), "{label} (h1) 0 bytes, one held body");
        assert!(queue_texts(&h1).is_empty(), "{label} (h1) not a queue job");
        let k = ret(&fx, Some(relauncher.0.id()), &h1, json!({"authoritative": true}));
        assert_eq!(k["ok"], json!(true), "{label} (h1) the foreign Return takes the pin path: {k}");
        assert_eq!(k["result"].get("bound"), None, "{label} (h1) the foreign Return binds no held body: {k}");
        assert_eq!((gen(&h1), held(&h1)), (g0 + 1, 1), "{label} (h1) one bare CR; the held body is still held");
        let h2 = relaunched_master(&fx, &maker, true, None);
        paint_empty_box(&h2);
        let g1 = gen(&h2);
        assert_converted(&send(&fx, Some(sender), &h2, "[보고] window", json!({})));
        assert_eq!(gen(&h2), g1, "{label} (h2) converted: nothing written at receipt");
    }
}

// ===== ★(0.14.56 · P1 D9 reinject order · impl/DESIGN-ADDENDUM-D9-reinject-order.md rev 1 §8; head rulings 171d431a9879596a,
//   d8f5cad3c957c3ed) A converted AUTHORITATIVE send whose caller passes the gate goes ahead of ordinary queued items, behind any
//   pinned (held / in-flight) prefix and behind earlier authoritative jobs. RED on afcbe1d3 (+ the field and gate, placement unchanged). =====

/// An ordinary report queued for the master by a worker pane (`--queued`: the tail).
fn queued_report(fx: &Fx, m: &Arc<Surface>, pid: u32, text: &str) {
    let r = send(fx, Some(pid), m, text, json!({"queued": true}));
    assert_eq!(r["ok"], json!(true), "{r}");
}

/// An authoritative send (as `inject_text` makes it).
fn auth(fx: &Fx, pid: Option<u32>, m: &Arc<Surface>, text: &str) -> Value {
    send(fx, pid, m, text, json!({"authoritative": true}))
}

/// Quiesce `m` as cycle-agent does (`bind_owner`), from `pid`.
fn quiesce_as_cycle(fx: &Fx, pid: u32, m: &Arc<Surface>) {
    let r = rpc(fx, Some(pid), "surface.quiesce", json!({"surface_id": m.id, "on": true, "bind_owner": true}));
    assert_eq!(r["ok"], json!(true), "{r}");
}

fn queue_auth_flags(s: &Arc<Surface>) -> Vec<bool> {
    s.pending_queue.lock().unwrap().iter().map(|e| e.authoritative).collect()
}

/// A1 (arm 1): a cso pane's authoritative inject is placed ahead of reports already queued (the cycle reinject's order at the pin).
/// Mutant: keep `push_back` for authoritative entries.
#[test]
fn d9_order_a1_cso_authoritative_inject_goes_ahead_of_queued_reports() {
    let fx = fx("ord-a1");
    let m = master(&fx);
    let _w = pane(&fx, "worker-1", P + 40);
    let _c = pane(&fx, "cso", P + 41);
    queued_report(&fx, &m, P + 40, "[보고] r1");
    queued_report(&fx, &m, P + 40, "[보고] r2");
    assert_converted(&auth(&fx, Some(P + 41), &m, "[RESUME] a1"));
    assert_eq!(queue_texts(&m), vec!["[RESUME] a1", "[보고] r1", "[보고] r2"]);
    assert_eq!(queue_auth_flags(&m), vec![true, false, false]);
}

/// A1 (arm 2): the target's current cycle owner (a real process: this test runner, quiesce with `bind_owner`) - its authoritative
/// inject goes first although it resolves to no pane (the CSO's cycle-agent case, measured caller null). Mutant: drop arm 2.
#[test]
fn d9_order_a1_cycle_owner_inject_goes_ahead_of_queued_reports() {
    let fx = fx("ord-a1b");
    let m = master(&fx);
    let _w = pane(&fx, "worker-1", P + 42);
    let me = std::process::id();
    quiesce_as_cycle(&fx, me, &m);
    queued_report(&fx, &m, P + 42, "[보고] r1");
    assert_converted(&auth(&fx, Some(me), &m, "[RESUME] a1b"));
    assert_eq!(queue_texts(&m), vec!["[RESUME] a1b", "[보고] r1"]);
}

/// A2 negatives: the flag with no arm stays at the tail - an unresolved off-pane pid, a worker pane, a non-owner process while the
/// target is quiesced by another, and the would-be owner when nothing is quiesced.
#[test]
fn d9_order_a2_authoritative_flag_without_an_arm_stays_at_the_tail() {
    let me = std::process::id();
    for (i, case) in ["unresolved", "worker", "not_owner", "not_quiesced"].into_iter().enumerate() {
        let fx = fx(&format!("ord-a2-{i}"));
        let m = master(&fx);
        let _w = pane(&fx, "worker-1", P + 50);
        if case == "not_owner" {
            quiesce_as_cycle(&fx, me, &m);
        }
        queued_report(&fx, &m, P + 50, "[보고] r1");
        let caller = match case {
            "unresolved" | "not_owner" => Some(P + 59), // no pane, no live process
            "worker" => Some(P + 50),
            _ => Some(me),
        };
        assert_converted(&auth(&fx, caller, &m, "[RESUME] a2"));
        assert_eq!(queue_texts(&m), vec!["[보고] r1", "[RESUME] a2"], "{case}");
        assert_eq!(queue_auth_flags(&m), vec![false, false], "{case}: not marked");
    }
}

/// A3: never ahead of a pinned head - a persisted write hold on the front (the restart-adoption premise) and an in-memory partial
/// QueueHold: the authoritative job goes right behind it, still ahead of the other reports. Mutant: ignore write_holds / queue_hold.
#[test]
fn d9_order_a3_never_ahead_of_a_held_head() {
    for held in ["persisted", "in_memory"] {
        let fx = fx(&format!("ord-a3-{held}"));
        let m = master(&fx);
        let _w = pane(&fx, "worker-1", P + 60);
        let _c = pane(&fx, "cso", P + 61);
        queued_report(&fx, &m, P + 60, "[보고] held");
        queued_report(&fx, &m, P + 60, "[보고] r2");
        let head = m.pending_queue.lock().unwrap()[0].id.clone();
        if held == "persisted" {
            fx.daemon.write_holds.lock().unwrap().insert(
                head.clone(),
                crate::state::PersistedWriteHold {
                    kind: "partial_exact".into(),
                    reason: "test".into(),
                    request_id: "rq".into(),
                    entry_ids: vec![head.clone()],
                    body_offset: 3,
                    held_at: crate::state::now_epoch(),
                },
            );
        } else {
            *m.queue_hold.lock().unwrap() = Some(crate::state::QueueHold {
                kind: crate::state::QueueHoldKind::PartialExact,
                reason: "test".into(),
                request_id: "rq".into(),
                entry_ids: vec![head.clone()],
                since: std::time::Instant::now(),
                attempts: 1,
                next_retry: None,
                envelope: None,
                notified: false,
                resume: None,
                resuming: false,
            });
        }
        assert_converted(&auth(&fx, Some(P + 61), &m, "[RESUME] a3"));
        assert_eq!(queue_texts(&m), vec!["[보고] held", "[RESUME] a3", "[보고] r2"], "{held}");
    }
}

/// A4: FIFO among authoritative jobs - two in a row keep their order, both ahead of the reports. Mutant: insert at index 0 always.
#[test]
fn d9_order_a4_authoritative_jobs_keep_fifo_among_themselves() {
    let fx = fx("ord-a4");
    let m = master(&fx);
    let _w = pane(&fx, "worker-1", P + 70);
    let _c = pane(&fx, "cso", P + 71);
    queued_report(&fx, &m, P + 70, "[보고] r1");
    assert_converted(&auth(&fx, Some(P + 71), &m, "directive one"));
    assert_converted(&auth(&fx, Some(P + 71), &m, "directive two"));
    assert_eq!(queue_texts(&m), vec!["directive one", "directive two", "[보고] r1"]);
}

/// A7 (queue pair, no-runaway half): one authoritative inject is exactly one queue job and its Return is absorbed, not re-sent - no
/// second entry, nothing typed. The delivery half (the report behind it still goes out) is measured in the rig e2e.
#[test]
fn d9_order_a7_one_job_per_authoritative_inject_and_its_return_absorbed() {
    let fx = fx("ord-a7");
    let m = master(&fx);
    let _w = pane(&fx, "worker-1", P + 80);
    let _c = pane(&fx, "cso", P + 81);
    queued_report(&fx, &m, P + 80, "[보고] r1");
    let g0 = gen(&m);
    assert_converted(&auth(&fx, Some(P + 81), &m, "[RESUME] a7"));
    assert_absorbed(&ret(&fx, Some(P + 81), &m, json!({"authoritative": true})));
    assert_eq!(queue_texts(&m).len(), 2, "one job for the inject");
    assert_eq!(gen(&m), g0, "nothing typed");
}

fn qe(id: &str, from: &str, enqueued_at: f64, authoritative: bool) -> crate::state::QueueEntry {
    crate::state::QueueEntry {
        id: id.into(),
        seq: 1,
        text: format!("body {id}"),
        enqueued_at,
        from: Some(from.into()),
        origin: "send".into(),
        ttl_secs: None,
        paused_total_secs: 0.0,
        expired_at: None,
        revived_at: None,
        expired_notified: false,
        expired_event_sent: false,
        authoritative,
    }
}

fn merge_plan(q: &[crate::state::QueueEntry]) -> Vec<usize> {
    let froms: Vec<Option<String>> = q.iter().map(|e| e.from.clone()).collect();
    let origins: Vec<String> = q.iter().map(crate::governance::merge_origin_key).collect();
    let chars: Vec<usize> = q.iter().map(|e| e.text.chars().count()).collect();
    crate::governance::plan_queue_merge(&froms, &origins, &chars, 5, 4000)
}

/// A5 (MERGE closure): an authoritative head goes alone - never merged with a later same-from report (two submissions, [RESUME]
/// first), nor two authoritative jobs with each other; ordinary same-from entries still merge as at the pin (control).
/// Mutant: `merge_origin_key` returns the plain origin.
#[test]
fn d9_order_a5_authoritative_entry_is_a_merge_barrier() {
    let now = crate::state::now_epoch();
    assert_eq!(merge_plan(&[qe("a", "cli:inject", now, true), qe("r", "cli:inject", now, false)]), vec![0], "[RESUME] alone");
    assert_eq!(merge_plan(&[qe("a", "x", now, true), qe("b", "x", now, true)]), vec![0], "two authoritative never merge");
    assert_eq!(merge_plan(&[qe("r1", "x", now, false), qe("r2", "x", now, false)]), vec![0, 1], "control: ordinary merge unchanged");
}

/// A6 (AGES closure): the stuck-queue age reads the oldest blocked item - an old report behind a fresh head-placed job; an old
/// authoritative job ahead of a newer report; the front when nothing is authoritative; a held ordinary front ahead of an
/// authoritative job. Mutant: `queue_age_head` returns the front.
#[test]
fn d9_order_a6_age_head_is_the_oldest_blocked_item() {
    let now = crate::state::now_epoch();
    let q = |v: Vec<crate::state::QueueEntry>| v.into_iter().collect::<std::collections::VecDeque<_>>();
    let id = |q: &std::collections::VecDeque<crate::state::QueueEntry>| crate::governance::queue_age_head(q).map(|e| e.id.clone());
    assert_eq!(id(&q(vec![qe("auth", "c", now, true), qe("old", "w", now - 700.0, false)])), Some("old".into()));
    assert_eq!(id(&q(vec![qe("auth", "c", now - 900.0, true), qe("new", "w", now, false)])), Some("auth".into()));
    assert_eq!(id(&q(vec![qe("r1", "w", now, false), qe("r0", "w", now - 50.0, false)])), Some("r1".into()), "pin: the front");
    assert_eq!(id(&q(vec![qe("held", "w", now - 10.0, false), qe("auth", "c", now - 20.0, true)])), Some("held".into()));
    assert_eq!(id(&q(vec![])), None);
}

/// A2b (mutant O3 survived without it): a caller WITH an arm but WITHOUT the authoritative flag - a cso pane's ordinary send into
/// the master - is converted and stays at the tail, unmarked. Mutant: the gate ignores the flag.
#[test]
fn d9_order_a2b_an_arm_without_the_authoritative_flag_stays_at_the_tail() {
    let fx = fx("ord-a2b");
    let m = master(&fx);
    let _w = pane(&fx, "worker-1", P + 90);
    let _c = pane(&fx, "cso", P + 91);
    queued_report(&fx, &m, P + 90, "[보고] r1");
    assert_converted(&send(&fx, Some(P + 91), &m, "[알림] ordinary", json!({})));
    assert_eq!(queue_texts(&m), vec!["[보고] r1", "[알림] ordinary"]);
    assert_eq!(queue_auth_flags(&m), vec![false, false]);
}

/// A2c (mutant O8 survived without it): arm 2's start-time guard - the window's recorded owner pid now belongs to a process that
/// started AFTER the window was set (a recycled pid: here this runner, with the window's time moved before its start) -> no head
/// placement. Control: the same window with a time after this runner's start -> head placement. Mutant: drop the start-time test.
#[test]
fn d9_order_a2c_cycle_owner_needs_a_start_time_before_the_window() {
    let me = std::process::id();
    let started = crate::state::peer_start_time(me).expect("this runner's start time") as f64;
    for (case, at, want_first) in [("recycled", started - 100.0, false), ("owner", started + 1.0, true)] {
        let fx = fx(&format!("ord-a2c-{case}"));
        let m = master(&fx);
        let _w = pane(&fx, "worker-1", P + 92);
        quiesce_as_cycle(&fx, me, &m);
        // Move the window's time (owner record AND quiescing status, so `at == since` still holds).
        if let Some(st) = m.agent_status.lock().unwrap().as_mut() {
            st.updated_at = at;
        }
        let g = m.quiesce_owner.lock().unwrap().map(|(_, _, g)| g).expect("owner bound");
        *m.quiesce_owner.lock().unwrap() = Some((me, at, g));
        queued_report(&fx, &m, P + 92, "[보고] r1");
        assert_converted(&auth(&fx, Some(me), &m, "[RESUME] a2c"));
        let first = queue_texts(&m)[0] == "[RESUME] a2c";
        assert_eq!(first, want_first, "{case}: {:?}", queue_texts(&m));
    }
}
