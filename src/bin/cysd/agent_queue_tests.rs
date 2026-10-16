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

/// S4-1 window at the queue: a byte-exact stop is held silently inside the 30,000 ms window and
/// announced once after it (until the resume controller lands, nothing else is typed).
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
    let window = Duration::from_millis(crate::agent_write::PARTIAL_EXACT_RESUME_WINDOW_MS);
    let Some(before) = Instant::now().checked_sub(window - Duration::from_millis(200)) else { return };
    s.queue_hold.lock().unwrap().as_mut().unwrap().since = before;
    assert_eq!(crate::governance::queue_write_hold_block(&fx.daemon, &s), Some(crate::governance::BLOCKED_PARTIAL_WRITE));
    assert!(events(&fx, "queue.write_held").is_empty(), "window - 200 ms: still silent");
    let Some(after) = Instant::now().checked_sub(window + Duration::from_millis(1)) else { return };
    s.queue_hold.lock().unwrap().as_mut().unwrap().since = after;
    assert_eq!(crate::governance::queue_write_hold_block(&fx.daemon, &s), Some(crate::governance::BLOCKED_PARTIAL_WRITE));
    assert_eq!(crate::governance::queue_write_hold_block(&fx.daemon, &s), Some(crate::governance::BLOCKED_PARTIAL_WRITE));
    let held = events(&fx, "queue.write_held");
    assert_eq!(held.len(), 1, "exactly one notice after the window");
    assert_eq!(held[0]["payload"]["kind"], "partial_exact");
    assert!(held[0]["payload"]["body_offset"].as_u64().unwrap() >= 4000, "{held:?}");
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
