//! ★(0.14.52 · S-a) Queue family settlement on the writer's terminal (DESIGN-S-FROZEN-R3 §4.5 table,
//! S-T02 / S-T08; HEAD-RULINGS-W4-1 F1 pair rule).
//!
//! The queue hands its head off as ONE checked agent write (`WriteReq::Agent`). Only a written body
//! AND its own Return pops the items, writes the receipt and publishes `queue.delivered`; a refusal
//! keeps every item queued and holds the seat on backoff (one coalesced event); a partial write keeps
//! the reservation and the items and never retypes. Seats are real PTYs (`sleep 30`) with the real
//! writer thread; the receiver proof is scripted through `Surface::receiver_proof`.
#![cfg(test)]

use crate::agent_write::{Phase, Proof};
use crate::governance::deliver_head_locked;
use crate::receiver::ProofLoss;
use crate::state::{Daemon, QueueHoldKind, Surface};
use serde_json::Value;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
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
    // CYS_PACK_DIR is process-global: same lock as the governance queue and handlers ACL specimens.
    let g = crate::governance::PACK_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    crate::delivery::tests::isolate_state_dir_for_thread(tag);
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("cys-aq-{tag}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("acl.json"), r#"{"default":"allow","rules":[]}"#).unwrap();
    std::env::set_var(cys::pack::ENV_PACK_DIR, &dir);
    let daemon = Daemon::new(dir.join("cysd.sock"));
    // Hand-offs return after the writer's settlement (exact wait on the request); the specimen
    // that observes the asynchronous window turns this off itself.
    crate::agent_write::set_test_sync_settle(true);
    Fx { daemon, dir, _g: g }
}

fn seat(fx: &Fx) -> Arc<Surface> {
    let s = fx
        .daemon
        .create_surface(None, Some("sleep 30".into()), None, None, 24, 80)
        .expect("create surface");
    fx.daemon.surfaces.lock().unwrap().insert(s.id, s.clone());
    s
}

fn enqueue(fx: &Fx, s: &Arc<Surface>, text: &str) -> String {
    let e = fx.daemon.next_queue_entry(text.into(), None, "test");
    let id = e.id.clone();
    s.pending_queue.lock().unwrap().push_back(e);
    id
}

fn queue_len(s: &Arc<Surface>) -> usize {
    s.pending_queue.lock().unwrap().len()
}

fn events(fx: &Fx, name: &str) -> Vec<Value> {
    fx.daemon.bus.tail(1000).into_iter().filter(|e| e["name"] == name).collect()
}

fn hold_kind(s: &Arc<Surface>) -> Option<QueueHoldKind> {
    s.queue_hold.lock().unwrap().as_ref().map(|h| h.kind)
}

/// Bounded wait until `cond` holds (10 s).
fn wait_until(what: &str, cond: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !cond() {
        assert!(Instant::now() < deadline, "not within 10 s: {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Scripted receiver proof: the first `refuse` admissions fail with `admit_loss`; recheck number
/// `fail_at` (0-based, counted across jobs) fails with `recheck_loss`.
struct Scripted {
    refuse: AtomicU32,
    admit_loss: ProofLoss,
    fail_at: Mutex<Option<(usize, ProofLoss)>>,
    rechecks: AtomicUsize,
}

impl Scripted {
    fn new(refuse: u32, admit_loss: ProofLoss, fail_at: Option<(usize, ProofLoss)>) -> Arc<Scripted> {
        Arc::new(Scripted { refuse: AtomicU32::new(refuse), admit_loss, fail_at: Mutex::new(fail_at), rechecks: AtomicUsize::new(0) })
    }
}

impl Proof for Scripted {
    fn admit(&self) -> Result<(), ProofLoss> {
        let left = self.refuse.load(Ordering::Acquire);
        if left > 0 {
            self.refuse.store(left - 1, Ordering::Release);
            return Err(self.admit_loss.clone());
        }
        Ok(())
    }
    fn recheck(&self, _: Phase) -> Result<(), ProofLoss> {
        let n = self.rechecks.fetch_add(1, Ordering::AcqRel);
        match &*self.fail_at.lock().unwrap() {
            Some((at, loss)) if *at == n => Err(loss.clone()),
            _ => Ok(()),
        }
    }
}

fn hand_off(fx: &Fx, s: &Arc<Surface>) -> bool {
    deliver_head_locked(&fx.daemon, s, true, false, None, None, None, None).is_some()
}

/// S-T08 positive arm: the item is popped, receipted and announced exactly once — and only after
/// the writer wrote the body and its own Return (the guard claim alone no longer counts).
#[test]
fn queue_item_pops_and_announces_only_after_the_written_return() {
    let fx = fx("ok");
    crate::agent_write::set_test_sync_settle(false);
    let s = seat(&fx);
    let id = enqueue(&fx, &s, "hello agent");
    assert!(hand_off(&fx, &s), "claimed hand-off");
    // The writer waits 400 ms between the body and the Return: right now nothing is settled.
    assert!(s.inject_reserved(), "reservation must stay until the writer's terminal");
    assert_eq!(queue_len(&s), 1, "popped before the bytes were written");
    assert!(events(&fx, "queue.delivered").is_empty(), "announced before the bytes were written");
    wait_until("queue.delivered", || !events(&fx, "queue.delivered").is_empty());
    assert!(!s.inject_reserved());
    assert_eq!(queue_len(&s), 0);
    let delivered = events(&fx, "queue.delivered");
    assert_eq!(delivered.len(), 1, "exactly one delivery event");
    assert_eq!(delivered[0]["payload"]["queue_entry_id"], Value::from(id.clone()));
    assert_eq!(delivered[0]["payload"]["remaining"], Value::from(0));
    assert!(hold_kind(&s).is_none());
    assert!(s.inject_track.last_body().is_some(), "the body reached the writer");
    let journal = std::fs::read_to_string(fx.dir.join(crate::state::WRITER_JOURNAL_FILE)).expect("journal");
    assert_eq!(journal.matches("\"rec\":\"writing\"").count(), 1);
    assert_eq!(journal.matches("\"kind\":\"written_submitted\"").count(), 1);
    assert!(!journal.contains("hello agent"), "the journal never carries body text");
}

/// S-T02/S-T08 refusal arm + F1(b) pair rule: a refused hand-off writes zero bytes, keeps the item,
/// holds the seat on backoff with ONE coalesced event, and once the proof returns the item is
/// delivered exactly once (no duplicate, no notice flood).
#[test]
fn refused_hand_off_keeps_the_item_backs_off_coalesces_and_then_delivers_once() {
    let fx = fx("refused");
    let s = seat(&fx);
    *s.receiver_proof.lock().unwrap() = Some(Scripted::new(2, ProofLoss::Unknown("agent:denied".into()), None));
    let id = enqueue(&fx, &s, "directive");
    // Refusal 1.
    assert!(hand_off(&fx, &s));
    assert_eq!(queue_len(&s), 1, "a refused item must stay queued");
    assert!(!s.inject_reserved());
    assert!(s.inject_track.last_body().is_none(), "zero bytes reached the writer");
    assert_eq!(s.pending_input_bytes.load(Ordering::Relaxed), 0, "hand-off counter rolled back");
    let (attempts, next) = {
        let h = s.queue_hold.lock().unwrap();
        let h = h.as_ref().expect("refused hold");
        assert_eq!(h.kind, QueueHoldKind::Refused);
        assert_eq!(h.reason, "unknown:agent:denied");
        (h.attempts, h.next_retry.expect("backoff"))
    };
    assert_eq!(attempts, 1);
    assert!(next > Instant::now() + Duration::from_millis(500) && next <= Instant::now() + Duration::from_secs(1));
    assert_eq!(crate::governance::queue_write_hold_block(&fx.daemon, &s), Some(crate::governance::BLOCKED_RECEIVER_HELD));
    assert_eq!(events(&fx, "queue.inject_aborted").len(), 1);
    // Backoff elapsed -> the tick may retry; refusal 2 grows the backoff and stays silent.
    s.queue_hold.lock().unwrap().as_mut().unwrap().next_retry = Some(Instant::now());
    assert_eq!(crate::governance::queue_write_hold_block(&fx.daemon, &s), None);
    assert!(hand_off(&fx, &s));
    {
        let h = s.queue_hold.lock().unwrap();
        let h = h.as_ref().unwrap();
        assert_eq!(h.attempts, 2);
        assert!(h.next_retry.unwrap() > Instant::now() + Duration::from_millis(1500), "second backoff step is 2 s");
    }
    assert_eq!(events(&fx, "queue.inject_aborted").len(), 1, "same-episode retries must not re-announce");
    assert!(events(&fx, "queue.delivered").is_empty());
    // The proof is back: delivered exactly once, hold ended.
    s.queue_hold.lock().unwrap().as_mut().unwrap().next_retry = Some(Instant::now());
    assert!(hand_off(&fx, &s));
    assert_eq!(queue_len(&s), 0);
    let delivered = events(&fx, "queue.delivered");
    assert_eq!(delivered.len(), 1);
    assert_eq!(delivered[0]["payload"]["queue_entry_id"], Value::from(id));
    assert!(hold_kind(&s).is_none());
}

/// S-T08 crash-after-prefix arm: the receiver dies mid-request. The prefix is never retyped, the
/// items and their reservation stay (no second hand-off into a composer that holds a prefix), one
/// coalesced notice, and disposers treat the ids as in delivery.
#[test]
fn partial_unknown_write_keeps_reservation_and_items_and_never_hands_off_again() {
    let fx = fx("partial-unknown");
    let s = seat(&fx);
    // Calls: open fence (admitted) · body (recheck 0) · close fence (recheck 1 = dead).
    *s.receiver_proof.lock().unwrap() = Some(Scripted::new(0, ProofLoss::AgentDead, Some((1, ProofLoss::AgentDead))));
    let id = enqueue(&fx, &s, "a short directive");
    assert!(hand_off(&fx, &s));
    assert_eq!(hold_kind(&s), Some(QueueHoldKind::PartialUnknown));
    assert!(s.inject_reserved(), "a partial write keeps the reservation");
    assert_eq!(queue_len(&s), 1);
    assert!(events(&fx, "queue.delivered").is_empty());
    assert!(s.pending_input_bytes.load(Ordering::Relaxed) > 0, "the composer holds an accepted prefix");
    let held = events(&fx, "queue.write_held");
    assert_eq!(held.len(), 1);
    assert_eq!(held[0]["payload"]["kind"], "partial_unknown");
    assert_eq!(held[0]["payload"]["body_offset"], Value::from("a short directive".len()));
    assert_eq!(held[0]["payload"]["fence_closed"], Value::from(false));
    assert!(!hand_off(&fx, &s), "no second hand-off while a prefix is in the composer");
    assert_eq!(crate::governance::queue_write_hold_block(&fx.daemon, &s), Some(crate::governance::BLOCKED_PARTIAL_WRITE));
    assert_eq!(events(&fx, "queue.write_held").len(), 1, "one notice per hold episode");
    let in_delivery = {
        let _q = s.pending_queue.lock().unwrap();
        s.cancel_inject_reservation()
    };
    assert_eq!(in_delivery, vec![id], "disposers must not drop a partially written item");
}

/// S4-1 window at the queue: a byte-exact stop that cannot resume (foreign input reached the seat after the hold) is
/// held silently inside the 30,000 ms window and announced once after it. ★(0.14.52 · S-a C7b) The window counts from
/// the loss; both edges are pinned with an explicit clock (no latency in the assertion).
#[test]
fn partial_exact_write_is_held_and_announced_once_after_the_resume_window() {
    let fx = fx("partial-exact");
    let s = seat(&fx);
    let body: String = "가나다라마바사 ".repeat(800); // > 4,096 bytes: two body calls at least
    // Calls: open (admitted) · body 1 (recheck 0) · body 2 (recheck 1 = transient Unknown).
    *s.receiver_proof.lock().unwrap() = Some(Scripted::new(0, ProofLoss::AgentDead, Some((1, ProofLoss::Unknown("agent:denied".into())))));
    enqueue(&fx, &s, &body);
    assert!(hand_off(&fx, &s));
    assert_eq!(hold_kind(&s), Some(QueueHoldKind::PartialExact));
    assert!(s.inject_reserved());
    assert!(events(&fx, "queue.write_held").is_empty(), "inside the window the hold is silent");
    // Foreign input after the hold: this request may never continue into that composer.
    s.set_pending_input(s.pending_input_bytes.load(Ordering::Relaxed) + 1);
    let since = s.queue_hold.lock().unwrap().as_ref().unwrap().since;
    let window = Duration::from_millis(crate::agent_write::PARTIAL_EXACT_RESUME_WINDOW_MS);
    let at = |d: Duration| crate::governance::queue_write_hold_block_at(&fx.daemon, &s, since + d);
    assert_eq!(at(window - Duration::from_millis(1)), Some(crate::governance::BLOCKED_PARTIAL_WRITE));
    assert!(events(&fx, "queue.write_held").is_empty(), "window - 1 ms: still silent");
    assert_eq!(at(window + Duration::from_millis(1)), Some(crate::governance::BLOCKED_PARTIAL_WRITE));
    assert_eq!(at(window + Duration::from_millis(1)), Some(crate::governance::BLOCKED_PARTIAL_WRITE));
    let held = events(&fx, "queue.write_held");
    assert_eq!(held.len(), 1, "exactly one notice after the window");
    assert_eq!(held[0]["payload"]["kind"], "partial_exact");
    assert!(held[0]["payload"]["body_offset"].as_u64().unwrap() >= 4000, "{held:?}");
    assert!(events(&fx, "queue.write_resumed").is_empty() && events(&fx, "queue.delivered").is_empty(), "resumed after foreign input");
}

/// Design 4.4 step 2 with self-healing: while the journal cannot be opened every hand-off is refused
/// before its first byte and held; once the state directory is usable the next retry delivers.
#[test]
fn unavailable_journal_refuses_before_bytes_and_heals_on_the_next_retry() {
    let fx = fx("journal");
    let s = seat(&fx);
    let path = fx.dir.join(crate::state::WRITER_JOURNAL_FILE);
    std::fs::create_dir_all(&path).unwrap(); // a directory where the journal file must go
    enqueue(&fx, &s, "directive");
    assert!(hand_off(&fx, &s));
    assert_eq!(queue_len(&s), 1);
    assert!(s.inject_track.last_body().is_none(), "zero bytes without a durable writing record");
    let reason = s.queue_hold.lock().unwrap().as_ref().map(|h| h.reason.clone()).unwrap();
    assert!(reason.starts_with("journal:"), "{reason}");
    std::fs::remove_dir_all(&path).unwrap();
    s.queue_hold.lock().unwrap().as_mut().unwrap().next_retry = Some(Instant::now());
    assert!(hand_off(&fx, &s));
    assert_eq!(queue_len(&s), 0);
    assert_eq!(events(&fx, "queue.delivered").len(), 1);
}

/// ★(0.14.52 · S-a C7a · F3) A daemon restart never retypes a partially written queue item: the partial-write hold must
/// survive the restart (or be rebuilt), so the restored entry is NOT handed off again onto the composer that already holds
/// its prefix (the double-delivery direction).
#[test]
fn restart_with_a_held_partial_never_retypes_it() {
    let fx = fx("restart-held");
    let s = fx
        .daemon
        .create_surface(None, Some("sleep 30".into()), None, Some("worker".into()), 24, 80)
        .expect("seat");
    fx.daemon.surfaces.lock().unwrap().insert(s.id, s.clone());
    fx.daemon.roles.lock().unwrap().insert("worker".into(), s.id);
    let body = "a held directive ".repeat(400); // > 4,096 bytes: the loss lands mid-body
    *s.receiver_proof.lock().unwrap() = Some(Scripted::new(0, ProofLoss::AgentDead, Some((1, ProofLoss::Unknown("agent:denied".into())))));
    let id = enqueue(&fx, &s, &body);
    assert!(hand_off(&fx, &s), "premise: handed off");
    assert_eq!(hold_kind(&s), Some(QueueHoldKind::PartialExact), "premise: held part-way");
    fx.daemon.persist_queue_state();
    // Restart: a new daemon on the same state directory, the same role on a new seat.
    let d2 = Daemon::new(fx.dir.join("cysd.sock"));
    let s2 = d2
        .create_surface(None, Some("sleep 30".into()), None, Some("worker".into()), 24, 80)
        .expect("seat after restart");
    d2.surfaces.lock().unwrap().insert(s2.id, s2.clone());
    d2.roles.lock().unwrap().insert("worker".into(), s2.id);
    d2.rehome_restored_queue();
    let restored: Vec<String> = s2.pending_queue.lock().unwrap().iter().map(|e| e.id.clone()).collect();
    let retyped = deliver_head_locked(&d2, &s2, true, false, None, None, None, None).is_some();
    let typed = s2.inject_track.last_body().is_some();
    let _ = s2.child.lock().unwrap().kill();
    assert_eq!(restored, vec![id], "premise: the held entry survives the restart");
    assert!(!retyped && !typed, "a restarted daemon handed the partially written item off again (retype)");
}

/// ★(0.14.52 · S-a C7a · head condition on Q2) The automatic, bounded exit of a held partial after a restart: while the new
/// seat's composer shows a prefix the item stays held (no retype, no other hand-off, one notice); once the composer is
/// measured empty at a prompt boundary (after PARTIAL_RELEASE_MIN_AGE_MS) the held item is released as undelivered and the
/// NEXT item is delivered once.
#[test]
fn restarted_hold_releases_only_when_the_composer_is_measured_empty() {
    let fx = fx("restart-release");
    let s = fx
        .daemon
        .create_surface(None, Some("sleep 30".into()), None, Some("worker".into()), 24, 80)
        .expect("seat");
    fx.daemon.surfaces.lock().unwrap().insert(s.id, s.clone());
    fx.daemon.roles.lock().unwrap().insert("worker".into(), s.id);
    *s.receiver_proof.lock().unwrap() = Some(Scripted::new(0, ProofLoss::AgentDead, Some((1, ProofLoss::Unknown("agent:denied".into())))));
    let held = enqueue(&fx, &s, &"a held directive ".repeat(400));
    assert!(hand_off(&fx, &s), "premise: handed off");
    fx.daemon.persist_queue_state();
    let d2 = Daemon::new(fx.dir.join("cysd.sock"));
    let s2 = d2
        .create_surface(None, Some("sleep 30".into()), None, Some("worker".into()), 24, 80)
        .expect("seat after restart");
    d2.surfaces.lock().unwrap().insert(s2.id, s2.clone());
    d2.roles.lock().unwrap().insert("worker".into(), s2.id);
    *s2.agent_meta.lock().unwrap() = Some(("claude".into(), "/usr/local/bin/claude".into()));
    d2.rehome_restored_queue();
    let next = d2.next_queue_entry("the next item".into(), None, "test");
    let next_id = next.id.clone();
    s2.pending_queue.lock().unwrap().push_back(next);
    let backdate = |s: &Arc<Surface>| {
        let mut h = s.queue_hold.lock().unwrap();
        let h = h.as_mut().expect("held");
        if let Some(t) = Instant::now().checked_sub(Duration::from_millis(crate::governance::PARTIAL_RELEASE_MIN_AGE_MS + 100)) {
            h.since = t;
        }
    };
    let ev = |name: &str| d2.bus.tail(1000).into_iter().filter(|e| e["name"] == name).count();
    // Right after adoption an empty-looking composer is not a measurement yet (minimum age): still held.
    crate::governance::h_paint(&s2, crate::governance::H_IDLE_SCREEN);
    assert_eq!(
        crate::governance::queue_write_hold_block(&d2, &s2),
        Some(crate::governance::BLOCKED_PARTIAL_WRITE),
        "released before PARTIAL_RELEASE_MIN_AGE_MS (a race, not a measurement)"
    );
    // Prefix visible: held, nothing handed off, one notice.
    crate::governance::h_paint(&s2, &crate::governance::h_residue_screen("a held directive a held directive"));
    assert_eq!(crate::governance::queue_write_hold_block(&d2, &s2), Some(crate::governance::BLOCKED_PARTIAL_WRITE));
    backdate(&s2);
    assert_eq!(crate::governance::queue_write_hold_block(&d2, &s2), Some(crate::governance::BLOCKED_PARTIAL_WRITE), "released with a prefix visible");
    assert!(deliver_head_locked(&d2, &s2, true, false, None, None, None, None).is_none(), "handed off while held");
    assert!(s2.inject_track.last_body().is_none(), "something was typed while held");
    assert_eq!(ev("queue.write_held"), 1, "one coalesced notice");
    assert_eq!(ev("queue.write_released"), 0);
    // Composer measured empty: released, the held item leaves, the next item is delivered once.
    crate::governance::h_paint(&s2, crate::governance::H_IDLE_SCREEN);
    assert_eq!(crate::governance::queue_write_hold_block(&d2, &s2), None, "not released with an empty composer");
    assert_eq!(ev("queue.write_released"), 1);
    let ids: Vec<String> = s2.pending_queue.lock().unwrap().iter().map(|e| e.id.clone()).collect();
    assert_eq!(ids, vec![next_id], "the held item is still queued (or the next one was lost)");
    assert!(!d2.write_holds.lock().unwrap().contains_key(&held), "the persisted hold outlived its release");
    assert!(deliver_head_locked(&d2, &s2, true, false, None, None, None, None).is_some(), "the next item was not handed off");
    let typed = s2.inject_track.last_body().map(|b| b.norm);
    let _ = s2.child.lock().unwrap().kill();
    assert!(typed.as_deref().is_some_and(|t| t.contains("the next item") && !t.contains("a held directive")), "{typed:?}");
}

// ─── ★(0.14.52 · S-a C7b) S4-1 PartialExact resume controller ───────────────────────────────────────────────────────

/// A seat whose receiver records every byte it is given (raw, no echo) - the bytes the request put on the wire, once.
fn recorder_seat(fx: &Fx, tag: &str) -> (Arc<Surface>, std::path::PathBuf) {
    let out = fx.dir.join(format!("rx-{tag}.bin"));
    let cmd = format!("stty raw -echo; exec cat > '{}'", out.display());
    let s = fx.daemon.create_surface(None, Some(cmd), None, None, 24, 200).expect("recorder seat");
    fx.daemon.surfaces.lock().unwrap().insert(s.id, s.clone());
    wait_until("recorder ready", || out.exists());
    std::thread::sleep(Duration::from_millis(300)); // stty applied before the first byte
    (s, out)
}

/// The exact bytes one queue hand-off of `body` writes (open fence, body, close fence, its own Return).
fn queue_bytes(body: &str) -> Vec<u8> {
    crate::agent_write::Plan::paste(body).with_submit(400, 0).planned_bytes()
}

/// The ~97,000-character directive of the S4-1 test.
fn s4_1_body() -> String {
    (0..5706).map(|i| format!("S4-1 part {i:05}; ")).collect()
}

/// ★S4-1 (HEAD-R3-RULING major 1, locked test): a ~97,000-character directive with ONE injected non-death Unknown
/// mid-body on a healthy agent resumes from its byte-exact offset and submits once - the receiver gets every byte exactly
/// once (no retyped prefix, one Return), the item is delivered once, and the hold stays silent (inside the window).
#[test]
#[cfg_attr(not(unix), ignore = "좌석 명령이 POSIX 셸 문법")]
fn s4_1_mid_body_transient_loss_resumes_and_submits_once() {
    let fx = fx("s4-1-resume");
    let (s, out) = recorder_seat(&fx, "s4-1");
    let body = s4_1_body();
    assert!(body.chars().count() >= 97_000, "premise: ~97,000 characters");
    // Calls: open fence (admitted) · body calls 1..12 (rechecks 0..11) · body call 13 (recheck 12 = transient Unknown).
    *s.receiver_proof.lock().unwrap() = Some(Scripted::new(0, ProofLoss::AgentDead, Some((12, ProofLoss::Unknown("agent:denied".into())))));
    let id = enqueue(&fx, &s, &body);
    assert!(hand_off(&fx, &s), "premise: handed off");
    assert_eq!(hold_kind(&s), Some(QueueHoldKind::PartialExact), "premise: a byte-exact stop mid-body");
    let off = s.queue_hold.lock().unwrap().as_ref().and_then(|h| h.envelope).map(|e| e.body_offset).unwrap();
    assert_eq!(off, 12 * crate::agent_write::MAX_CALL_BYTES, "premise: stopped mid-body at a call boundary");
    {
        let h = s.queue_hold.lock().unwrap();
        let h = h.as_ref().unwrap();
        assert_eq!(Some(h.since), h.resume.as_ref().unwrap().lost_at(), "the window must count from the proof loss, not the settlement");
    }
    // The next tick inside the window: the SAME request continues from its offset; a second tick while it runs does not
    // hand it off again.
    assert_eq!(crate::governance::queue_write_hold_block(&fx.daemon, &s), Some(crate::governance::BLOCKED_PARTIAL_WRITE));
    let _ = crate::governance::queue_write_hold_block(&fx.daemon, &s);
    wait_until("resumed request settled", || queue_len(&s) == 0);
    let want = queue_bytes(&body);
    wait_until("receiver got every byte", || std::fs::metadata(&out).map(|m| m.len() as usize).unwrap_or(0) >= want.len());
    std::thread::sleep(Duration::from_millis(200)); // nothing more arrives (no retype, no second Return)
    let got = std::fs::read(&out).unwrap();
    let delivered = events(&fx, "queue.delivered");
    assert_eq!(delivered.len(), 1, "delivered once");
    assert_eq!(delivered[0]["payload"]["queue_entry_id"], Value::from(id));
    assert!(events(&fx, "queue.write_held").is_empty(), "resumed inside the window: no notice");
    assert!(hold_kind(&s).is_none() && !s.inject_reserved(), "hold and reservation end with the delivery");
    assert_eq!(events(&fx, "queue.write_resumed").len(), 1, "one resumed run");
    assert_eq!(s.pending_input_bytes.load(Ordering::Relaxed), 0, "the line was submitted");
    assert!(got == want, "receiver bytes differ: got {} bytes, want {} (a retyped prefix or a lost tail)", got.len(), want.len());
}

/// Seat events of one surface only (several seats share a fixture).
fn seat_events(fx: &Fx, s: &Arc<Surface>, name: &str) -> Vec<Value> {
    let r = cys::surface_ref(s.id);
    events(fx, name).into_iter().filter(|e| e["payload"]["surface_ref"] == Value::from(r.clone())).collect()
}

/// Hand off `body` on `s` and stop it byte-exactly before body call `at` (a transient Unknown); returns the hold's loss instant.
fn hold_exact(fx: &Fx, s: &Arc<Surface>, body: &str, at: usize) -> Instant {
    *s.receiver_proof.lock().unwrap() = Some(Scripted::new(0, ProofLoss::AgentDead, Some((at, ProofLoss::Unknown("agent:denied".into())))));
    enqueue(fx, s, body);
    assert!(hand_off(fx, s), "premise: handed off");
    let h = s.queue_hold.lock().unwrap();
    let h = h.as_ref().expect("premise: held");
    assert_eq!(h.kind, QueueHoldKind::PartialExact, "premise: a byte-exact stop");
    assert!(h.resume.is_some(), "premise: resumable material");
    h.since
}

fn file_len(p: &std::path::Path) -> usize {
    std::fs::metadata(p).map(|m| m.len() as usize).unwrap_or(0)
}

/// Wait until the recorder holds exactly the held prefix (open fence + accepted body bytes) and return its length.
fn wait_prefix(s: &Arc<Surface>, out: &std::path::Path) -> usize {
    let off = s.queue_hold.lock().unwrap().as_ref().and_then(|h| h.envelope).map(|e| e.body_offset).unwrap();
    let want = cys::paste_fence::OPEN.len() + off;
    wait_until("recorder holds the prefix", || file_len(out) == want);
    want
}

/// ★S4-1 / D3: the resume window is exact - at window - 1 ms from the loss the SAME request continues; at window + 1 ms it
/// does not (held, one notice), with the clock pinned by the specimen.
#[test]
#[cfg_attr(not(unix), ignore = "좌석 명령이 POSIX 셸 문법")]
fn s4_1_resume_window_edges_are_exact() {
    let fx = fx("s4-1-edges");
    // The value is locked (HEAD-RULINGS-W4-1 D3, LOCK-W4-WINDOWS-AND-N-20261016.json): 30,000 ms from proof loss.
    assert_eq!(crate::agent_write::PARTIAL_EXACT_RESUME_WINDOW_MS, 30_000, "locked S4-1 window changed");
    let window = Duration::from_millis(crate::agent_write::PARTIAL_EXACT_RESUME_WINDOW_MS);
    let body = "an edge directive ".repeat(600); // > 4,096 bytes
    // + 1 ms: not resumed.
    let (late, late_out) = recorder_seat(&fx, "late");
    let since = hold_exact(&fx, &late, &body, 1);
    let prefix = wait_prefix(&late, &late_out);
    assert_eq!(
        crate::governance::queue_write_hold_block_at(&fx.daemon, &late, since + window + Duration::from_millis(1)),
        Some(crate::governance::BLOCKED_PARTIAL_WRITE)
    );
    std::thread::sleep(Duration::from_millis(300));
    assert!(seat_events(&fx, &late, "queue.write_resumed").is_empty(), "resumed after the window");
    assert_eq!(queue_len(&late), 1);
    assert_eq!(file_len(&late_out), prefix, "bytes were typed after the window");
    assert_eq!(seat_events(&fx, &late, "queue.write_held").len(), 1, "one notice at expiry");
    // - 1 ms: resumed, delivered once, every byte once.
    let (edge, edge_out) = recorder_seat(&fx, "edge");
    let since = hold_exact(&fx, &edge, &body, 1);
    assert_eq!(
        crate::governance::queue_write_hold_block_at(&fx.daemon, &edge, since + window - Duration::from_millis(1)),
        Some(crate::governance::BLOCKED_PARTIAL_WRITE)
    );
    wait_until("resumed at window - 1 ms", || queue_len(&edge) == 0);
    let want = queue_bytes(&body);
    wait_until("every byte", || file_len(&edge_out) >= want.len());
    std::thread::sleep(Duration::from_millis(200));
    assert!(std::fs::read(&edge_out).unwrap() == want, "receiver bytes differ at window - 1 ms");
    assert_eq!(seat_events(&fx, &edge, "queue.delivered").len(), 1);
}

/// Receiver proof that admits the first hand-off, loses at recheck `fail_at`, and refuses every later admission (a
/// resumed run that never gets its fresh full proof).
struct LostForGood {
    admits: AtomicUsize,
    rechecks: AtomicUsize,
    fail_at: usize,
}

impl Proof for LostForGood {
    fn admit(&self) -> Result<(), ProofLoss> {
        if self.admits.fetch_add(1, Ordering::AcqRel) == 0 {
            Ok(())
        } else {
            Err(ProofLoss::Unknown("agent:denied".into()))
        }
    }
    fn recheck(&self, _: Phase) -> Result<(), ProofLoss> {
        if self.rechecks.fetch_add(1, Ordering::AcqRel) == self.fail_at {
            Err(ProofLoss::Unknown("agent:denied".into()))
        } else {
            Ok(())
        }
    }
}

/// ★D3 "the window does not widen under load": repeated resume attempts that make no progress keep the FIRST loss
/// instant (each failed admission is a new loss of the same request), back off 1, 2, 4, 8 s between attempts (so at most
/// five attempts fit: the fifth backs off 30 s, past the window), never add a phantom byte to the held count, and stop at
/// window + 1 ms with one notice.
#[test]
fn s4_1_window_does_not_widen_under_repeated_no_progress_resumes() {
    let fx = fx("s4-1-widen");
    let s = seat(&fx);
    let proof = Arc::new(LostForGood { admits: AtomicUsize::new(0), rechecks: AtomicUsize::new(0), fail_at: 1 });
    *s.receiver_proof.lock().unwrap() = Some(proof.clone());
    enqueue(&fx, &s, &"a widening directive ".repeat(400));
    assert!(hand_off(&fx, &s));
    let (since, held_count) = {
        let h = s.queue_hold.lock().unwrap();
        (h.as_ref().unwrap().since, s.pending_input_bytes.load(Ordering::Relaxed))
    };
    let held_at = || fx.daemon.write_holds.lock().unwrap().values().next().map(|h| h.held_at);
    let first_held_at = held_at();
    let window = Duration::from_millis(crate::agent_write::PARTIAL_EXACT_RESUME_WINDOW_MS);
    for k in 1..=5u32 {
        let before = proof.admits.load(Ordering::Acquire);
        let _ = crate::governance::queue_write_hold_block_at(&fx.daemon, &s, since + Duration::from_secs(4 * k as u64));
        let what = format!("resumed attempt {k} settled (admits {before}, hold {:?})", s.queue_hold.lock().unwrap().as_ref().map(|h| (h.attempts, h.resuming, h.resume.is_some(), h.next_retry.map(|t| t.saturating_duration_since(since)))));
        wait_until(&what, || {
            proof.admits.load(Ordering::Acquire) == before + 1 && s.queue_hold.lock().unwrap().as_ref().is_some_and(|h| !h.resuming)
        });
        let h = s.queue_hold.lock().unwrap();
        let h = h.as_ref().unwrap();
        assert_eq!(h.kind, QueueHoldKind::PartialExact);
        assert_eq!(h.since, since, "attempt {k}: the window start moved (widened)");
        assert_eq!(h.attempts, k, "attempt {k}: no-progress attempts are counted");
        assert!(h.next_retry.is_some(), "attempt {k}: a no-progress attempt backs off");
        assert_eq!(s.pending_input_bytes.load(Ordering::Relaxed), held_count, "attempt {k}: the held count drifted");
        drop(h);
        assert_eq!(held_at(), first_held_at, "attempt {k}: the persisted hold age restarted");
    }
    // The backoff is real: right now and at 24 s (the fifth attempt backs off 30 s) no attempt is made.
    let before = proof.admits.load(Ordering::Acquire);
    let _ = crate::governance::queue_write_hold_block(&fx.daemon, &s);
    let _ = crate::governance::queue_write_hold_block_at(&fx.daemon, &s, since + Duration::from_secs(24));
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(proof.admits.load(Ordering::Acquire), before, "attempted inside the backoff");
    // Past the window: no attempt, one notice.
    let _ = crate::governance::queue_write_hold_block_at(&fx.daemon, &s, since + window + Duration::from_millis(1));
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(proof.admits.load(Ordering::Acquire), before, "attempted after the window");
    assert_eq!(events(&fx, "queue.write_held").len(), 1, "one notice at expiry");
    assert!(events(&fx, "queue.delivered").is_empty());
}

/// ★S4-1 premises: an agent epoch move, a conversation epoch move (e.g. `/clear`), a foreign writer write, foreign input
/// accounting, or a human keystroke after the hand-off - each stops the resume (held, nothing typed, nothing announced as
/// resumed).
#[test]
fn s4_1_no_resume_after_an_epoch_move_or_foreign_input() {
    let fx = fx("s4-1-premises");
    let body = "a premise directive ".repeat(400);
    let cases: Vec<(&str, Box<dyn Fn(&Arc<Surface>)>)> = vec![
        ("agent epoch", Box::new(|s: &Arc<Surface>| s.bump_agent_epoch())),
        ("conversation epoch", Box::new(|s: &Arc<Surface>| s.bump_conversation_epoch())),
        (
            "foreign writer write",
            Box::new(|s: &Arc<Surface>| {
                let w = s.inject_track.wrote();
                s.write_tx.send(crate::state::WriteReq::Data(b"x".to_vec())).unwrap();
                wait_until("foreign write counted", || s.inject_track.wrote() > w);
            }),
        ),
        ("foreign input accounting", Box::new(|s: &Arc<Surface>| s.set_pending_input(s.pending_input_bytes.load(Ordering::Relaxed) + 1))),
        ("human keystroke", Box::new(|s: &Arc<Surface>| *s.last_human_input.lock().unwrap() = Some(Instant::now()))),
    ];
    for (what, apply) in cases {
        let s = seat(&fx);
        let since = hold_exact(&fx, &s, &body, 1);
        apply(&s);
        assert_eq!(
            crate::governance::queue_write_hold_block_at(&fx.daemon, &s, since + Duration::from_secs(1)),
            Some(crate::governance::BLOCKED_PARTIAL_WRITE),
            "{what}"
        );
        std::thread::sleep(Duration::from_millis(150));
        assert!(seat_events(&fx, &s, "queue.write_resumed").is_empty(), "{what}: resumed");
        assert_eq!(queue_len(&s), 1, "{what}");
        assert!(s.queue_hold.lock().unwrap().as_ref().is_some_and(|h| h.kind == QueueHoldKind::PartialExact && !h.resuming), "{what}");
    }
}

/// ★S4-1 "no new modal": a modal on the seat's current frame makes the resume WAIT inside the window; once it is gone the
/// SAME request continues and is delivered once.
#[test]
#[cfg_attr(not(unix), ignore = "좌석 명령이 POSIX 셸 문법")]
fn s4_1_modal_waits_inside_the_window_then_resumes() {
    let fx = fx("s4-1-modal");
    let (s, out) = recorder_seat(&fx, "modal");
    *s.agent_meta.lock().unwrap() = Some(("claude".into(), "/usr/local/bin/claude".into()));
    let body = "a modal directive ".repeat(400);
    let since = hold_exact(&fx, &s, &body, 1);
    crate::governance::h_paint(&s, cys::first_run_gates::fixtures::LIVE_PERMISSION_PROMPT);
    let _ = crate::governance::queue_write_hold_block_at(&fx.daemon, &s, since + Duration::from_secs(1));
    std::thread::sleep(Duration::from_millis(200));
    assert!(events(&fx, "queue.write_resumed").is_empty(), "resumed into a modal");
    assert_eq!(queue_len(&s), 1);
    crate::governance::h_paint(&s, crate::governance::H_IDLE_SCREEN);
    let _ = crate::governance::queue_write_hold_block_at(&fx.daemon, &s, since + Duration::from_secs(2));
    wait_until("resumed after the modal", || queue_len(&s) == 0);
    let want = queue_bytes(&body);
    wait_until("every byte", || file_len(&out) >= want.len());
    std::thread::sleep(Duration::from_millis(200));
    assert!(std::fs::read(&out).unwrap() == want, "receiver bytes differ after the modal");
    assert_eq!(events(&fx, "queue.delivered").len(), 1);
}

/// ★S4-1 at the first byte: a foreign write already queued in the writer AHEAD of the resumed run (invisible to the tick's
/// check) is caught by the resumed run's claim-time probe - not one byte of the remainder follows it, and the hold never
/// resumes afterwards.
#[test]
#[cfg_attr(not(unix), ignore = "좌석 명령이 POSIX 셸 문법")]
fn s4_1_foreign_write_queued_ahead_of_the_resume_voids_it_at_the_first_byte() {
    let fx = fx("s4-1-ahead");
    let (s, out) = recorder_seat(&fx, "ahead");
    let body = "an ahead directive ".repeat(400);
    let since = hold_exact(&fx, &s, &body, 1);
    let prefix = wait_prefix(&s, &out);
    // The writer is busy with a foreign request that has not written yet when the tick checks.
    s.write_tx.send(crate::state::WriteReq::DataAfter { bytes: b"X".to_vec(), delay_ms: 400 }).unwrap();
    let _ = crate::governance::queue_write_hold_block_at(&fx.daemon, &s, since + Duration::from_secs(1));
    assert_eq!(seat_events(&fx, &s, "queue.write_resumed").len(), 1, "premise: the tick handed the resumed run off");
    // Settled = the hold is no longer resuming (or gone: the run completed - which the byte check below refuses).
    wait_until("resumed run settled", || s.queue_hold.lock().unwrap().as_ref().is_none_or(|h| !h.resuming));
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(file_len(&out), prefix + 1, "only the foreign byte follows the prefix (nothing of the remainder)");
    let h = s.queue_hold.lock().unwrap().as_ref().map(|h| (h.kind, h.resume.is_some())).unwrap();
    assert_eq!(h, (QueueHoldKind::PartialExact, false), "the voided hold must never resume");
    let _ = crate::governance::queue_write_hold_block_at(&fx.daemon, &s, since + Duration::from_secs(2));
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(seat_events(&fx, &s, "queue.write_resumed").len(), 1, "resumed after a void");
    assert_eq!(queue_len(&s), 1);
}

/// ★S4-1 at the first byte: an epoch move AFTER the tick's check but before the resumed run's first byte (the writer was
/// busy without writing a byte) voids the resume at claim time - nothing of the remainder is typed.
#[test]
#[cfg_attr(not(unix), ignore = "좌석 명령이 POSIX 셸 문법")]
fn s4_1_epoch_move_between_the_check_and_the_first_byte_voids_the_resume() {
    let fx = fx("s4-1-claim-epoch");
    let (s, out) = recorder_seat(&fx, "claim-epoch");
    let since = hold_exact(&fx, &s, &"a claim directive ".repeat(400), 1);
    let prefix = wait_prefix(&s, &out);
    let wrote = s.inject_track.wrote();
    // The writer is busy with a withheld Return whose probe takes 800 ms: no byte, no write count.
    let busy: crate::state::SafetyProbe = Arc::new(|| {
        std::thread::sleep(Duration::from_millis(800));
        true
    });
    s.write_tx.send(crate::state::WriteReq::SubmitAfterGap { bytes: b"\r".to_vec(), min_gap_ms: 0, withhold: Some(busy) }).unwrap();
    let _ = crate::governance::queue_write_hold_block_at(&fx.daemon, &s, since + Duration::from_secs(1));
    assert_eq!(seat_events(&fx, &s, "queue.write_resumed").len(), 1, "premise: the tick handed the resumed run off");
    s.bump_agent_epoch(); // between the check and the claim
    // Settled = the hold is no longer resuming (or gone: the run completed - which the byte check below refuses).
    wait_until("resumed run settled", || s.queue_hold.lock().unwrap().as_ref().is_none_or(|h| !h.resuming));
    std::thread::sleep(Duration::from_millis(200));
    let now_wrote = s.inject_track.wrote();
    assert_eq!(file_len(&out), prefix, "bytes were typed after an epoch move (writer count {wrote} -> {now_wrote})");
    assert_eq!(now_wrote, wrote, "premise: the busy request wrote nothing");
    let h = s.queue_hold.lock().unwrap().as_ref().map(|h| (h.kind, h.resume.is_some())).unwrap();
    assert_eq!(h, (QueueHoldKind::PartialExact, false), "a voided hold must never resume");
}

/// Receiver proof that loses at recheck `fail_at` after a keystroke reached the seat's input accounting mid-write.
struct InputDuringWrite {
    s: std::sync::Weak<Surface>,
    rechecks: AtomicUsize,
    fail_at: usize,
}

impl Proof for InputDuringWrite {
    fn admit(&self) -> Result<(), ProofLoss> {
        Ok(())
    }
    fn recheck(&self, _: Phase) -> Result<(), ProofLoss> {
        if self.rechecks.fetch_add(1, Ordering::AcqRel) == self.fail_at {
            if let Some(s) = self.s.upgrade() {
                s.set_pending_input(s.pending_input_bytes.load(Ordering::Relaxed) + 3);
            }
            return Err(ProofLoss::Unknown("agent:denied".into()));
        }
        Ok(())
    }
}

/// ★S4-1 "no foreign input": input that reached the seat WHILE the original request was writing makes the byte-exact
/// stop non-resumable from the start (held; the automatic exit owns it after the window).
#[test]
fn s4_1_input_during_the_original_write_is_never_resumed() {
    let fx = fx("s4-1-input-during");
    let s = seat(&fx);
    *s.receiver_proof.lock().unwrap() = Some(Arc::new(InputDuringWrite { s: Arc::downgrade(&s), rechecks: AtomicUsize::new(0), fail_at: 1 }));
    enqueue(&fx, &s, &"an input directive ".repeat(400));
    assert!(hand_off(&fx, &s));
    let since = {
        let h = s.queue_hold.lock().unwrap();
        let h = h.as_ref().unwrap();
        assert_eq!(h.kind, QueueHoldKind::PartialExact, "premise: byte-exact stop");
        assert!(h.resume.is_none(), "input during the write left the hold resumable");
        h.since
    };
    let _ = crate::governance::queue_write_hold_block_at(&fx.daemon, &s, since + Duration::from_secs(1));
    std::thread::sleep(Duration::from_millis(150));
    assert!(events(&fx, "queue.write_resumed").is_empty());
    assert_eq!(queue_len(&s), 1);
}

/// While a resumed run is in the writer the hold neither announces nor releases - even a tick past the window that
/// measures an empty composer (a release then would tombstone an item the writer is completing).
#[test]
#[cfg_attr(not(unix), ignore = "좌석 명령이 POSIX 셸 문법")]
fn s4_1_a_resumed_run_in_flight_is_never_released_or_announced() {
    let fx = fx("s4-1-inflight");
    let (s, out) = recorder_seat(&fx, "inflight");
    *s.agent_meta.lock().unwrap() = Some(("claude".into(), "/usr/local/bin/claude".into()));
    crate::governance::h_paint(&s, crate::governance::H_IDLE_SCREEN);
    let body = "an inflight directive ".repeat(400);
    let since = hold_exact(&fx, &s, &body, 1);
    let window = Duration::from_millis(crate::agent_write::PARTIAL_EXACT_RESUME_WINDOW_MS);
    let _ = crate::governance::queue_write_hold_block_at(&fx.daemon, &s, since + Duration::from_secs(1));
    assert!(s.queue_hold.lock().unwrap().as_ref().is_some_and(|h| h.resuming), "premise: the resumed run is in flight (400 ms Return delay)");
    // A tick past the window with the composer measured empty (idle frame) while the run is in flight.
    assert_eq!(
        crate::governance::queue_write_hold_block_at(&fx.daemon, &s, since + window + Duration::from_secs(5)),
        Some(crate::governance::BLOCKED_PARTIAL_WRITE)
    );
    wait_until("delivered", || queue_len(&s) == 0);
    let want = queue_bytes(&body);
    wait_until("every byte", || file_len(&out) >= want.len());
    std::thread::sleep(Duration::from_millis(200));
    assert!(std::fs::read(&out).unwrap() == want, "receiver bytes differ");
    assert_eq!(events(&fx, "queue.delivered").len(), 1);
    assert!(events(&fx, "queue.write_released").is_empty(), "released while the writer was completing it");
    assert!(events(&fx, "queue.write_held").is_empty(), "announced while the writer was completing it");
}

// ─── ★(0.14.52 · S-a C7c) design 4.6 reconciliation listing (read-only RPC `queue.reconciliation`) ─────────────────────

fn reconciliation(fx: &Fx, params: Value) -> Value {
    let req = cys::Request { id: Value::from(1), method: "queue.reconciliation".into(), params };
    match crate::handlers::dispatch(&fx.daemon, req, Some(std::process::id())) {
        crate::handlers::Reply::Single(v) => {
            assert_eq!(v["ok"], Value::from(true), "{v}");
            v["result"].clone()
        }
        _ => panic!("single reply"),
    }
}

/// ★Design 4.6: every partial write is listed with id, proof class, byte bounds, reason, age and action owner - and never
/// its body text; a request held on a seat (and persisted for a restart) is one row, not two.
#[test]
fn reconciliation_lists_partial_writes_with_bounds_and_never_body_text() {
    let fx = fx("recon-rows");
    let exact = seat(&fx);
    let since = hold_exact(&fx, &exact, &"SECRET-DIRECTIVE exact ".repeat(400), 1);
    let unknown = seat(&fx);
    *unknown.receiver_proof.lock().unwrap() = Some(Scripted::new(0, ProofLoss::AgentDead, Some((1, ProofLoss::AgentDead))));
    let unknown_id = enqueue(&fx, &unknown, &"SECRET-DIRECTIVE unknown ".repeat(400));
    assert!(hand_off(&fx, &unknown));
    assert_eq!(hold_kind(&unknown), Some(QueueHoldKind::PartialUnknown), "premise");
    assert!(fx.daemon.write_holds.lock().unwrap().len() >= 2, "premise: both holds are persisted too");
    let r = reconciliation(&fx, serde_json::json!({}));
    let text = r.to_string();
    assert!(!text.contains("SECRET-DIRECTIVE"), "body text in the listing: {text}");
    let rows = r["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 2, "one row per request: {r}");
    assert_eq!(r["total"], Value::from(2));
    assert!(r["next_cursor"].is_null());
    let by_kind = |k: &str| rows.iter().find(|x| x["kind"] == k).unwrap_or_else(|| panic!("no {k} row: {r}")).clone();
    let e = by_kind("partial_exact");
    assert_eq!(e["surface_ref"], Value::from(cys::surface_ref(exact.id)));
    assert_eq!(e["source"], "seat_hold");
    assert_eq!(e["bytes"]["body_offset"], Value::from(crate::agent_write::MAX_CALL_BYTES as u64));
    assert!(e["bytes"]["body_bytes"].as_u64().unwrap() > 4096);
    assert_eq!(e["bytes"]["submitted"], Value::from(false));
    assert_eq!(e["proof_class"], "bound", "a seat with a receiver proof");
    assert_eq!(e["action_owner"], "lane_master");
    assert!(e["request_id"].as_str().is_some_and(|x| !x.is_empty()));
    assert!(e["reason"].as_str().unwrap().starts_with("unknown:"));
    assert!(e["age_secs"].as_u64().unwrap() <= since.elapsed().as_secs() + 1);
    assert!(e["resolution"].as_str().is_some_and(|x| x.contains("composer")));
    let u = by_kind("partial_unknown");
    assert_eq!(u["entry_ids"], Value::from(vec![unknown_id]));
    assert_eq!(u["resumable"], Value::from(false));
    // The seat filter.
    let one = reconciliation(&fx, serde_json::json!({"surface_id": exact.id}));
    assert_eq!(one["rows"].as_array().unwrap().len(), 1);
}

/// ★Design 4.6: 100 rows per page with a cursor (a larger limit is clamped); pages are disjoint and cover every row,
/// including persisted holds no seat has adopted and an earlier run's inconclusive journal record.
#[test]
fn reconciliation_pages_100_rows_with_a_cursor() {
    let fx = fx("recon-pages");
    {
        let mut w = fx.daemon.write_holds.lock().unwrap();
        for i in 0..150 {
            let h = crate::state::PersistedWriteHold {
                kind: "partial_unknown".into(),
                reason: "daemon_restart:test".into(),
                request_id: format!("req-{i:03}"),
                entry_ids: vec![format!("W-{i:03}")],
                body_offset: 7,
                held_at: crate::state::now_epoch() - 60.0,
            };
            w.insert(format!("W-{i:03}"), h);
        }
    }
    fx.daemon.writer_inconclusive.lock().unwrap().push(crate::agent_write::Inconclusive {
        id: "req-journal".into(),
        surface: 9_999,
        origin: "queue".into(),
        digest: "d".repeat(64),
    });
    let p1 = reconciliation(&fx, serde_json::json!({"limit": 1000}));
    assert_eq!(p1["limit"], Value::from(100), "limit not clamped to 100");
    assert_eq!(p1["total"], Value::from(151));
    let rows1 = p1["rows"].as_array().unwrap().clone();
    assert_eq!(rows1.len(), 100);
    let cursor = p1["next_cursor"].as_str().expect("a next page").to_string();
    let p2 = reconciliation(&fx, serde_json::json!({"cursor": cursor}));
    let rows2 = p2["rows"].as_array().unwrap().clone();
    assert_eq!(rows2.len(), 51);
    assert!(p2["next_cursor"].is_null());
    let mut ids: Vec<String> = rows1.iter().chain(rows2.iter()).map(|x| x["request_id"].as_str().unwrap().to_string()).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 151, "pages overlap or miss rows");
    let j = rows2.iter().chain(rows1.iter()).find(|x| x["request_id"] == "req-journal").expect("journal row");
    assert_eq!(j["source"], "journal_inconclusive");
    assert_eq!(j["kind"], "inconclusive");
    let p = rows1.iter().chain(rows2.iter()).find(|x| x["request_id"] == "req-000").expect("persisted row");
    assert_eq!(p["source"], "persisted");
    assert!(p["age_secs"].as_u64().unwrap() >= 59);
}
