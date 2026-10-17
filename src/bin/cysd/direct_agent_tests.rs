//! ★(0.14.52 · S-a C5b) Direct send family on agent seats (DESIGN-S-FROZEN-R3 §4.3/§4.5; HEAD-R3-RULING
//! S4-3; HEAD-RULINGS-W4-2 window 60,000 ms + supersede rule; C5B-PLAN.md).
//!
//! An untokened body without its own Return is held untyped; the same caller's Return binds it and the
//! daemon writes body + Return as ONE checked request. A newer body from the same caller supersedes the
//! older one (explicit undelivered event); another caller never does. Bookkeeping waits for the writer's
//! terminal. Shell seats and human keys keep today's path. Seats are real PTYs (`sleep 30`) with the real
//! writer thread; "typed" is read from the durable writer journal (one `writing` record per request).
#![cfg(test)]

use crate::handlers::dispatch;
use crate::state::{Daemon, Surface};
use cys::Request;
use serde_json::{json, Value};
use std::sync::atomic::Ordering;
use std::sync::{Arc, MutexGuard};
use std::time::{Duration, Instant};

struct Fx {
    daemon: Arc<Daemon>,
    dir: std::path::PathBuf,
    _g: MutexGuard<'static, ()>,
}

impl Drop for Fx {
    fn drop(&mut self) {
        for s in self.daemon.surfaces.lock().unwrap().values() {
            let mut child = s.child.lock().unwrap();
            let _ = child.kill();
            let _ = child.wait();
        }
        std::env::remove_var(cys::pack::ENV_PACK_DIR);
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn fx(tag: &str) -> Fx {
    let g = crate::governance::PACK_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    crate::delivery::tests::isolate_state_dir_for_thread(tag);
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("cys-da-{tag}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("acl.json"), r#"{"default":"allow","rules":[]}"#).unwrap();
    std::env::set_var(cys::pack::ENV_PACK_DIR, &dir);
    let daemon = Daemon::new(dir.join("cysd.sock"));
    crate::agent_write::set_test_sync_settle(true);
    Fx { daemon, dir, _g: g }
}

fn seat(fx: &Fx, agent: bool) -> Arc<Surface> {
    let s = fx.daemon.create_surface(None, Some("sleep 30".into()), None, None, 24, 80).expect("create surface");
    fx.daemon.surfaces.lock().unwrap().insert(s.id, s.clone());
    if agent {
        *s.agent_meta.lock().unwrap() = Some(("claude".into(), "claude".into()));
    }
    s
}

/// Callers are told apart by their self-declared seat (`Claimed`, C5B-PLAN I9); the peer pid is fake.
fn rpc(fx: &Fx, method: &str, params: Value) -> Value {
    let req = Request { id: json!(1), method: method.into(), params };
    let crate::handlers::Reply::Single(resp) = dispatch(&fx.daemon, req, Some(1)) else { panic!("single reply") };
    resp
}

fn send(fx: &Fx, s: &Arc<Surface>, from: u64, text: &str) -> Value {
    rpc(fx, "surface.send_text", json!({"surface_id": s.id, "text": text, "from": format!("surface:{from}")}))
}

fn ret(fx: &Fx, s: &Arc<Surface>, from: u64) -> Value {
    rpc(fx, "surface.send_key", json!({"surface_id": s.id, "key": "Return", "from": format!("surface:{from}")}))
}

/// The seat proof for a Return that is WITHHELD at the Return phase (as if a dialog rose between the body and its
/// Return): the existing pass-through proof plus a probe that always says "withhold".
pub(crate) fn withholding_proof(probe: crate::state::SafetyProbe) -> Arc<dyn crate::agent_write::Proof> {
    Arc::new(crate::handlers::SubmitWithhold { inner: Arc::new(crate::agent_write::PassThrough), probe })
}

/// A seat proof whose Return phase loses proof (a non-death Unknown): the body is written, its Return is not.
struct LoseAtSubmit;
impl crate::agent_write::Proof for LoseAtSubmit {
    fn admit(&self) -> Result<(), crate::receiver::ProofLoss> {
        Ok(())
    }
    fn recheck(&self, phase: crate::agent_write::Phase) -> Result<(), crate::receiver::ProofLoss> {
        if phase == crate::agent_write::Phase::Submit {
            return Err(crate::receiver::ProofLoss::Unknown("test:return_lost".into()));
        }
        Ok(())
    }
}

/// Shared with the specimens written against the old two-step (send_settle S21, D-12, H residue, fence): since C5b
/// an automatic body without its Return is HELD untyped, so a typed, counted, owner-graded machine body left in the
/// composer arises only when the caller's Return binds it and the job stops before its Return (PartialExact). This
/// binds `sid`'s held body for the given caller shape (peer pid + the send's `from`/token keys in `caller`) with
/// such a loss and waits for settlement.
pub(crate) fn bind_with_return_lost(daemon: &Arc<Daemon>, sid: u64, caller_pid: Option<u32>, caller: Option<Value>) -> Value {
    let s = daemon.get_surface(sid).expect("surface");
    let prev = s.receiver_proof.lock().unwrap().replace(Arc::new(LoseAtSubmit));
    let mut params = json!({"surface_id": sid, "key": "Return", "quiet": true});
    for k in ["from", "owner_token", "operator_token"] {
        if let Some(v) = caller.as_ref().and_then(|c| c.get(k)) {
            params[k] = v.clone();
        }
    }
    let req = Request { id: json!(1), method: "surface.send_key".into(), params };
    let crate::handlers::Reply::Single(v) = dispatch(daemon, req, caller_pid) else { panic!("single reply") };
    *s.receiver_proof.lock().unwrap() = prev;
    assert!(v["result"]["bound"].is_string(), "premise: the held body was bound: {v}");
    v
}

fn journal(fx: &Fx) -> String {
    std::fs::read_to_string(fx.dir.join(crate::state::WRITER_JOURNAL_FILE)).unwrap_or_default()
}

/// Requests the writer started (one durable `writing` record each), by request id.
fn written_ids(fx: &Fx) -> Vec<String> {
    journal(fx)
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|v| v["rec"] == "writing")
        .map(|v| v["id"].as_str().unwrap_or_default().to_string())
        .collect()
}

fn events(fx: &Fx, name: &str) -> Vec<Value> {
    fx.daemon.bus.tail(2000).into_iter().filter(|e| e["name"] == name).collect()
}

fn id_of(resp: &Value) -> String {
    resp["result"]["request_id"].as_str().unwrap_or_else(|| panic!("no request_id: {resp}")).to_string()
}

/// S3-5 / S4-3: a legacy body is held with ZERO bytes; its caller's Return then writes body + Return once.
#[test]
fn a_legacy_body_is_held_untyped_until_its_callers_return_then_written_once_with_it() {
    let fx = fx("held");
    let s = seat(&fx, true);
    let r = send(&fx, &s, 900, "report one");
    assert_eq!(r["result"]["held"], "awaiting_submit", "{r}");
    assert_eq!(r["result"]["window_ms"], 60_000);
    let a = id_of(&r);
    assert!(written_ids(&fx).is_empty(), "a held body writes nothing");
    assert_eq!(s.pending_input_bytes.load(Ordering::Relaxed), 0, "no fictitious composer bytes");
    let k = ret(&fx, &s, 900);
    assert_eq!(k["result"]["bound"], "legacy", "{k}");
    assert_eq!(k["result"]["request_id"], json!(a));
    assert_eq!(written_ids(&fx), [a.clone()]);
    assert_eq!(journal(&fx).matches("\"kind\":\"written_submitted\"").count(), 1);
    assert_eq!(s.pending_input_bytes.load(Ordering::Relaxed), 0, "body and its own Return: the line is submitted");
    assert_eq!(s.submit_assoc.lock().unwrap().len(), 0);
}

/// Head condition (2): A (no Return), B, Return -> B written once with its Return; A undelivered as
/// superseded naming B; nothing typed twice (a second Return binds nothing).
#[test]
fn a_newer_body_supersedes_the_older_and_only_it_is_written_once() {
    let fx = fx("supersede");
    let s = seat(&fx, true);
    let a = id_of(&send(&fx, &s, 900, "body A"));
    let b = id_of(&send(&fx, &s, 900, "body B"));
    let gone = events(&fx, "surface.submit_undelivered");
    assert_eq!(gone.len(), 1, "{gone:?}");
    assert_eq!(gone[0]["payload"]["request_id"], json!(a));
    assert_eq!(gone[0]["payload"]["reason"], "superseded");
    assert_eq!(gone[0]["payload"]["superseded_by"], json!(b));
    let k = ret(&fx, &s, 900);
    assert_eq!(k["result"]["request_id"], json!(b), "{k}");
    let _ = ret(&fx, &s, 900); // the conventional extra Return binds nothing (today's bare-Return path)
    assert_eq!(written_ids(&fx), [b.clone()], "A never typed, B typed once");
}

/// Head condition (3): a body from a DIFFERENT caller never supersedes; each Return binds its own body.
#[test]
fn a_different_callers_body_never_supersedes_and_each_return_binds_its_own() {
    let fx = fx("callers");
    let s = seat(&fx, true);
    let a = id_of(&send(&fx, &s, 900, "from 900"));
    let b = id_of(&send(&fx, &s, 901, "from 901"));
    assert!(events(&fx, "surface.submit_undelivered").is_empty());
    assert_eq!(ret(&fx, &s, 901)["result"]["request_id"], json!(b));
    std::thread::sleep(Duration::from_millis(300)); // S21 split hold after the written Return
    assert_eq!(ret(&fx, &s, 900)["result"]["request_id"], json!(a));
    assert_eq!(written_ids(&fx), [b, a]);
}

/// A migrated caller's Return carries the token; an unknown token is a typed zero-byte refusal.
#[test]
fn a_token_binds_its_body_and_an_unknown_token_writes_nothing() {
    let fx = fx("token");
    let s = seat(&fx, true);
    let a = id_of(&send(&fx, &s, 900, "tokened"));
    let bad = rpc(&fx, "surface.send_key", json!({"surface_id": s.id, "key": "Return", "request_id": "w0-0"}));
    assert_eq!(bad["error"]["code"], "submit_unbound", "{bad}");
    assert!(written_ids(&fx).is_empty());
    let k = rpc(&fx, "surface.send_key", json!({"surface_id": s.id, "key": "Return", "request_id": a}));
    assert_eq!(k["result"]["bound"], "token", "{k}");
    assert_eq!(written_ids(&fx), [a]);
}

/// D3 as amended, through the handler: a Return inside the window binds; one 60,001 ms after receipt does not and
/// the tick disposes the body as expired - never typed. The exact 59,999 / 60,001 ms edges are pinned with an
/// injected clock in `submit_assoc::tests` (a real clock keeps running between backdating and the Return, so the
/// inside case here uses 59,000 ms).
#[test]
fn the_return_binds_inside_the_window_and_after_60001_ms_the_body_expires_untyped() {
    let fx = fx("window");
    let s = seat(&fx, true);
    let a = id_of(&send(&fx, &s, 900, "early"));
    s.submit_assoc.lock().unwrap().backdate(Duration::from_millis(59_000));
    assert_eq!(ret(&fx, &s, 900)["result"]["request_id"], json!(a));
    std::thread::sleep(Duration::from_millis(300)); // S21 split hold after the written Return
    let b = id_of(&send(&fx, &s, 900, "late"));
    s.submit_assoc.lock().unwrap().backdate(Duration::from_millis(60_001));
    let k = ret(&fx, &s, 900);
    assert!(k["result"].get("bound").is_none(), "late Return is not associated: {k}");
    crate::handlers::sweep_held_submits(&fx.daemon);
    assert_eq!(s.submit_assoc.lock().unwrap().len(), 0);
    let gone = events(&fx, "surface.submit_undelivered");
    assert!(gone.iter().any(|e| e["payload"]["request_id"] == json!(b) && e["payload"]["reason"] == "expired"), "{gone:?}");
    assert_eq!(written_ids(&fx), [a], "the expired body was never typed");
}

/// A queued Return turns the held body into ONE queue entry (the queue writes it with its own Return).
#[test]
fn a_queued_return_turns_the_held_body_into_one_queue_entry() {
    let fx = fx("queued");
    let s = seat(&fx, true);
    let a = id_of(&send(&fx, &s, 900, "queue me"));
    let k = rpc(&fx, "surface.send_key", json!({"surface_id": s.id, "key": "Return", "queued": true, "from": "surface:900"}));
    assert_eq!(k["result"]["request_id"], json!(a), "{k}");
    assert_eq!(k["result"]["queued"], true);
    let q = s.pending_queue.lock().unwrap();
    assert_eq!(q.len(), 1);
    assert_eq!(q[0].text, "queue me", "the entry carries the body, not an empty Return");
    drop(q);
    assert_eq!(s.submit_assoc.lock().unwrap().len(), 0);
    assert!(written_ids(&fx).is_empty(), "nothing typed until the queue delivers");
}

/// A clear-first write erases the line every held body would have been typed into: they are disposed.
#[test]
fn a_clear_first_write_disposes_the_held_bodies_as_cleared() {
    let fx = fx("cleared");
    let s = seat(&fx, true);
    let a = id_of(&send(&fx, &s, 900, "stale draft"));
    let c = rpc(&fx, "surface.send_text", json!({"surface_id": s.id, "text": "/clear", "clear_first": true, "authoritative": true}));
    assert!(c["result"]["request_id"].is_string(), "{c}");
    let gone = events(&fx, "surface.submit_undelivered");
    assert!(gone.iter().any(|e| e["payload"]["request_id"] == json!(a) && e["payload"]["reason"] == "cleared"), "{gone:?}");
    assert_eq!(written_ids(&fx).len(), 1, "only the clear-first request was typed");
    assert!(!written_ids(&fx).contains(&a));
}

/// A body held for one agent never binds to the next one (agent epoch).
#[test]
fn a_body_held_for_one_agent_never_binds_to_the_next() {
    let fx = fx("epoch");
    let s = seat(&fx, true);
    let a = id_of(&send(&fx, &s, 900, "for the old agent"));
    s.bump_agent_epoch();
    let k = ret(&fx, &s, 900);
    assert!(k["result"].get("bound").is_none(), "{k}");
    let gone = events(&fx, "surface.submit_undelivered");
    assert!(gone.iter().any(|e| e["payload"]["request_id"] == json!(a) && e["payload"]["reason"] == "agent_changed"), "{gone:?}");
    assert!(!written_ids(&fx).contains(&a));
}

/// Shell seats and human keys keep today's path: typed at once, counted at once, no journal request.
#[test]
fn shell_seats_and_human_keys_keep_the_existing_path() {
    let fx = fx("compat");
    let sh = seat(&fx, false);
    let r = send(&fx, &sh, 900, "echo hi");
    assert!(r["result"].get("held").is_none() && r["result"].get("request_id").is_none(), "{r}");
    assert_eq!(sh.pending_input_bytes.load(Ordering::Relaxed), "echo hi".len() as u64);
    let ag = seat(&fx, true);
    let h = rpc(&fx, "surface.send_text", json!({"surface_id": ag.id, "text": "abc", "human": true, "quiet": true}));
    assert!(h["result"].get("held").is_none() && h["result"].get("request_id").is_none(), "{h}");
    assert_eq!(ag.pending_input_bytes.load(Ordering::Relaxed), 3, "human keys counted at once");
    // The owner's Enter (GUI sendRaw "\r", human) stays a raw human key: no agent request, submitted at once.
    let enter = rpc(&fx, "surface.send_text", json!({"surface_id": ag.id, "text": "\r", "human": true, "quiet": true}));
    assert!(enter["result"].get("request_id").is_none(), "a human Enter is not an agent write: {enter}");
    assert_eq!(ag.pending_input_bytes.load(Ordering::Relaxed), 0, "the human Enter submitted the line at once");
    assert!(written_ids(&fx).is_empty());
}

/// Bookkeeping only after the terminal: a refused owned-submit write leaves counters, ticket and echo
/// window untouched and says so; a written one counts after its terminal.
#[test]
fn a_refused_direct_write_counts_nothing_and_says_so() {
    let fx = fx("refused");
    let s = seat(&fx, true);
    *s.receiver_proof.lock().unwrap() = Some(Arc::new(Refuse));
    let r = rpc(&fx, "surface.send_text", json!({"surface_id": s.id, "text": "own submit", "submit": true, "from": "surface:900"}));
    let a = id_of(&r);
    assert_eq!(s.pending_input_bytes.load(Ordering::Relaxed), 0);
    assert!(s.last_injected.lock().unwrap().is_none(), "no echo window for a refused write");
    let refused = events(&fx, "surface.write_refused");
    assert_eq!(refused.len(), 1, "{refused:?}");
    assert_eq!(refused[0]["payload"]["request_id"], json!(a));
    assert!(events(&fx, "surface.input_injected").is_empty());
}

struct Refuse;
impl crate::agent_write::Proof for Refuse {
    fn admit(&self) -> Result<(), crate::receiver::ProofLoss> {
        Err(crate::receiver::ProofLoss::Unknown("test:refuse".into()))
    }
    fn recheck(&self, _: crate::agent_write::Phase) -> Result<(), crate::receiver::ProofLoss> {
        Err(crate::receiver::ProofLoss::Unknown("test:refuse".into()))
    }
}

/// I8 ordering: the body is recorded (with its owner) BEFORE the Return's withhold probe runs, so a
/// withheld Return leaves exactly the old typed-unsubmitted state for the withheld-CR resubmission.
#[test]
fn the_body_is_recorded_before_the_returns_withhold_probe_runs() {
    let fx = fx("withhold");
    let s = seat(&fx, true);
    let seen = Arc::new(std::sync::Mutex::new(None::<String>));
    let (ws, seen2) = (Arc::downgrade(&s), seen.clone());
    let probe: crate::state::SafetyProbe = Arc::new(move || {
        if let Some(s) = ws.upgrade() {
            *seen2.lock().unwrap() = s.inject_track.last_body().map(|b| b.norm);
        }
        true // withhold
    });
    *s.receiver_proof.lock().unwrap() = Some(withholding_proof(probe));
    let request = crate::agent_write::Request::new("w-wh".into(), s.id, "test", "d".into());
    let handle = request.clone();
    let job = crate::agent_write::AgentJob {
        attempt: None,
        request,
        plan: crate::agent_write::Plan::raw("withheld body").with_submit(0, 0),
        proof: s.agent_proof(),
        journal: None,
        resume: None,
        settle: None,
        guard: None,
        note_body: true,
        body_owner: Some(crate::governance::InputOwner::Claimed(900)),
    };
    s.write_tx.try_send(crate::state::WriteReq::Agent(Box::new(job))).unwrap();
    crate::agent_write::test_wait_settled(&handle);
    assert_eq!(seen.lock().unwrap().as_deref(), Some("withheld body"), "probe saw the body recorded");
    let body = s.inject_track.last_body().expect("body recorded");
    assert_eq!(body.owner, Some(crate::governance::InputOwner::Claimed(900)));
    match handle.status() {
        crate::agent_write::Status::Terminal(crate::agent_write::Terminal::PartialExact { envelope, .. }) => {
            assert!(!envelope.submitted);
            assert_eq!(envelope.body_offset, "withheld body".len());
        }
        other => panic!("{other:?}"),
    }
    let _ = Instant::now();
}

/// ★(0.14.52 · S-a C7e) A held body is recorded durably the moment it is held - ids, length and digest, never its text -
/// and the record goes when the body is bound.
#[test]
fn a_held_body_is_recorded_durably_without_its_text_until_bound() {
    let fx = fx("held-durable");
    let s = seat(&fx, true);
    let a = id_of(&send(&fx, &s, 900, "HELD-SECRET body"));
    let read = || std::fs::read_to_string(fx.dir.join(crate::state::HELD_BODIES_FILE)).unwrap_or_default();
    let file = read();
    assert!(file.contains(&a), "the held body is not recorded: {file}");
    assert!(!file.contains("HELD-SECRET"), "body text on disk: {file}");
    let k = ret(&fx, &s, 900);
    assert!(k["result"]["bound"].is_string(), "{k}");
    assert!(!read().contains(&a), "a bound body is still recorded as held");
    // The tick's sweep: an expired body leaves the record without any RPC (a second seat: the first one's typing guard
    // follows the write that just happened).
    let s2 = seat(&fx, true);
    let b = id_of(&send(&fx, &s2, 901, "expires on the tick"));
    assert!(read().contains(&b), "premise: recorded");
    s2.submit_assoc.lock().unwrap().backdate(Duration::from_millis(crate::submit_assoc::LEGACY_ASSOCIATION_WINDOW_MS + 1));
    crate::handlers::sweep_held_submits(&fx.daemon);
    assert!(!read().contains(&b), "an expired body is still recorded as held after the tick's sweep");
}

/// ★(0.14.52 · S-a C7e) A restart never types a held body and never loses it silently: the next daemon disposes each
/// recorded body explicitly, once (`surface.submit_undelivered` reason=daemon_restart), and keeps durable evidence.
#[test]
fn a_restart_disposes_held_bodies_explicitly_once() {
    let fx = fx("held-restart");
    let s = seat(&fx, true);
    let a = id_of(&send(&fx, &s, 900, "held across a restart"));
    let d2 = Daemon::new(fx.dir.join("cysd.sock"));
    crate::handlers::sync_held_bodies(&d2);
    let gone = |d: &Arc<Daemon>| -> Vec<Value> {
        d.bus.tail(2000).into_iter().filter(|e| e["name"] == "surface.submit_undelivered").collect()
    };
    let ev = gone(&d2);
    assert_eq!(ev.len(), 1, "{ev:?}");
    assert_eq!(ev[0]["payload"]["request_id"], json!(a));
    assert_eq!(ev[0]["payload"]["reason"], "daemon_restart");
    let evidence = std::fs::read_to_string(fx.dir.join(crate::state::HELD_UNDELIVERED_LOG)).unwrap_or_default();
    assert!(evidence.contains(&a) && evidence.contains("daemon_restart"), "no durable evidence: {evidence}");
    let file = std::fs::read_to_string(fx.dir.join(crate::state::HELD_BODIES_FILE)).unwrap_or_default();
    assert!(!file.contains(&a), "a disposed body is still recorded as held: {file}");
    crate::handlers::sync_held_bodies(&d2);
    assert_eq!(gone(&d2).len(), 1, "disposed twice");
}
