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

pub(crate) struct Fx {
    pub(crate) daemon: Arc<Daemon>,
    pub(crate) dir: std::path::PathBuf,
    _g: MutexGuard<'static, ()>,
}

impl Drop for Fx {
    fn drop(&mut self) {
        crate::state::test_stop_daemon(&self.daemon);
        std::env::remove_var(cys::pack::ENV_PACK_DIR);
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

pub(crate) fn fx(tag: &str) -> Fx {
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
pub(crate) fn wait_until(what: &str, cond: impl Fn() -> bool) {
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
    hand_off_on(&fx.daemon, s)
}

/// One forced hand-off, re-run only when the seat's own PTY output fell inside the attempt. The hand-off gate refuses
/// while the reader publishes a chunk (odd `output_gen`): a Unix `sleep 30` seat never prints, a Windows seat prints
/// right after it starts (ConPTY start-up output; windows-health run 39207028554). `state::test_done_or_no_pty_output`
/// returns at once on a hand-off, keeps a refusal only from a quiet window and never re-runs on Unix, so an expected
/// refusal is still a measured one and an unexpected hand-off is never retried away.
fn hand_off_on(daemon: &Arc<Daemon>, s: &Arc<Surface>) -> bool {
    crate::state::test_done_or_no_pty_output(s, || deliver_head_locked(daemon, s, true, false, None, None, None, None, None).is_some())
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
    let retyped = hand_off_on(&d2, &s2);
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
    // Past the minimum age the release depends on the screen measurement, which reads nothing while the seat's reader
    // publishes (see `hand_off_on`): keep only a verdict from a quiet window. The two checks above are held by age alone.
    let held_with_prefix = crate::state::test_until_no_pty_output(&s2, "prefix-visible hold", || {
        crate::governance::h_paint(&s2, &crate::governance::h_residue_screen("a held directive a held directive"));
        crate::governance::queue_write_hold_block(&d2, &s2)
    });
    assert_eq!(held_with_prefix, Some(crate::governance::BLOCKED_PARTIAL_WRITE), "released with a prefix visible");
    assert!(!hand_off_on(&d2, &s2), "handed off while held");
    assert!(s2.inject_track.last_body().is_none(), "something was typed while held");
    assert_eq!(ev("queue.write_held"), 1, "one coalesced notice");
    assert_eq!(ev("queue.write_released"), 0);
    // Composer measured empty: released, the held item leaves, the next item is delivered once.
    let released = crate::state::test_until_no_pty_output(&s2, "empty-composer release", || {
        crate::governance::h_paint(&s2, crate::governance::H_IDLE_SCREEN);
        crate::governance::queue_write_hold_block(&d2, &s2)
    });
    assert_eq!(released, None, "not released with an empty composer");
    assert_eq!(ev("queue.write_released"), 1);
    let ids: Vec<String> = s2.pending_queue.lock().unwrap().iter().map(|e| e.id.clone()).collect();
    assert_eq!(ids, vec![next_id], "the held item is still queued (or the next one was lost)");
    assert!(!d2.write_holds.lock().unwrap().contains_key(&held), "the persisted hold outlived its release");
    assert!(hand_off_on(&d2, &s2), "the next item was not handed off");
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

/// A partial request must retain its original proof even if the surface publishes a successor.
#[test]
fn c8i_partial_resume_never_borrows_a_successor_proof() {
    let fx=fx("c8i-proof-stamp");let s=seat(&fx);
    let original=Arc::new(LostForGood{admits:AtomicUsize::new(0),rechecks:AtomicUsize::new(0),fail_at:1});
    *s.receiver_proof.lock().unwrap()=Some(original.clone());
    enqueue(&fx,&s,&"original bound directive ".repeat(400));
    assert!(hand_off(&fx,&s));
    assert_eq!(hold_kind(&s),Some(QueueHoldKind::PartialExact));
    let since=s.queue_hold.lock().unwrap().as_ref().unwrap().since;
    let wrote=s.inject_track.wrote();
    *s.receiver_proof.lock().unwrap()=Some(Arc::new(crate::agent_write::PassThrough));
    let _=crate::governance::queue_write_hold_block_at(&fx.daemon,&s,since+Duration::from_secs(1));
    wait_until("resume attempt settled",|| queue_len(&s)==0 || (original.admits.load(Ordering::Acquire)==2 && s.queue_hold.lock().unwrap().as_ref().is_some_and(|h| !h.resuming)));
    assert_eq!(original.admits.load(Ordering::Acquire),2,"partial resume must recheck its captured original proof");
    assert_eq!(queue_len(&s),1,"successor proof cannot complete an old partial obligation");
    assert_eq!(s.inject_track.wrote(),wrote,"no byte follows a successor proof");
    assert!(events(&fx,"queue.delivered").is_empty());
    assert_eq!(s.queue_hold.lock().unwrap().as_ref().unwrap().since,since,"first partial hold age remains fixed");
}

/// A caller whose outer precheck was earlier must still observe newly installed refusal backoff.
#[test]
fn c8pp_refusal_backoff_is_rechecked_at_locked_handoff() {
    let fx=fx("c8pp-backoff");let s=seat(&fx);
    *s.receiver_proof.lock().unwrap()=Some(Scripted::new(1,ProofLoss::Unknown("once".into()),None));
    enqueue(&fx,&s,"one retained obligation");assert!(hand_off(&fx,&s));
    s.queue_hold.lock().unwrap().as_mut().unwrap().next_retry=Some(Instant::now()+Duration::from_secs(3600));
    assert!(!hand_off(&fx,&s),"locked handoff must respect refusal installed after an outer precheck");
    assert_eq!(queue_len(&s),1);assert!(events(&fx,"queue.delivered").is_empty());
    s.queue_hold.lock().unwrap().as_mut().unwrap().next_retry=Some(Instant::now());
    assert!(hand_off(&fx,&s),"expired refusal backoff must permit healthy retry");
    assert_eq!(queue_len(&s),0);assert_eq!(events(&fx,"queue.delivered").len(),1);
}

fn c8pp_force_rpc(fx: &Fx, s: &Arc<Surface>) -> Value {
    // The forced delivery refuses ("delivery raced") while the seat's reader publishes, like `hand_off_on`: retried only
    // for a disturbed attempt, each attempt re-stamping c8pp_quiet_shell's quiet output (a seat chunk stamps it anew).
    let mut reply=Value::Null;
    let admitted=crate::state::test_done_or_no_pty_output(s,|| {
        *s.last_output.lock().unwrap()=Instant::now()-Duration::from_secs(2);
        let req=cys::Request{id:Value::from(1),method:"queue.deliver".into(),params:serde_json::json!({"surface_id":s.id})};
        match crate::handlers::dispatch(&fx.daemon,req,None) {
            crate::handlers::Reply::Single(r)=>{reply=r;reply["ok"]==Value::from(true)},
            _=>panic!("single reply"),
        }
    });
    assert!(admitted,"fixture admission: {reply}");
    reply["result"].clone()
}
fn c8pp_quiet_shell(s: &Arc<Surface>) {
    std::thread::sleep(Duration::from_millis(700));
    *s.last_output.lock().unwrap()=Instant::now()-Duration::from_secs(2);
}

#[test]
fn c8pp_force_rpc_never_reports_refusal_as_delivered() {
    let fx=fx("c8pp-rpc-refused");let s=seat(&fx);
    *s.receiver_proof.lock().unwrap()=Some(Scripted::new(1,ProofLoss::Unknown("test-refused".into()),None));
    enqueue(&fx,&s,"retained refusal");c8pp_quiet_shell(&s);
    let reply=c8pp_force_rpc(&fx,&s);
    assert_eq!(hold_kind(&s),Some(QueueHoldKind::Refused),"premise: terminal refusal settled");
    assert!(!s.queue_completion_pending.load(Ordering::Acquire),"refusal is not a completed-delivery notification");
    assert_eq!(reply["delivered"],Value::from(false),"queue RPC must not label a zero-byte refusal delivered");
    assert_eq!(reply["written"],Value::from("refused_no_write"));
    assert!(reply["request_id"].as_str().is_some_and(|id|!id.is_empty()));
    assert_eq!(queue_len(&s),1);assert!(events(&fx,"queue.delivered").is_empty());
}

#[test]
fn c8pp_force_rpc_reports_pending_while_receiver_proof_is_blocked() {
    struct WaitingProof { entered:std::sync::mpsc::Sender<()>, release:Mutex<std::sync::mpsc::Receiver<()>> }
    impl Proof for WaitingProof {
        fn admit(&self)->Result<(),ProofLoss>{
            self.entered.send(()).map_err(|_|ProofLoss::Unknown("fixture gone".into()))?;
            self.release.lock().unwrap().recv().map_err(|_|ProofLoss::Unknown("fixture released".into()))?;Ok(())
        }
        fn recheck(&self,_:Phase)->Result<(),ProofLoss>{Ok(())}
    }
    let fx=fx("c8pp-rpc-pending");let s=seat(&fx);
    let (entered,observed)=std::sync::mpsc::channel();let (release,wait)=std::sync::mpsc::channel();
    *s.receiver_proof.lock().unwrap()=Some(Arc::new(WaitingProof{entered,release:Mutex::new(wait)}));
    enqueue(&fx,&s,"pending then one submission");c8pp_quiet_shell(&s);
    crate::agent_write::set_test_sync_settle(false);
    let reply=c8pp_force_rpc(&fx,&s);
    let entered=observed.recv_timeout(Duration::from_secs(2));
    let before=events(&fx,"queue.delivered").len();
    let premature_completion=s.queue_completion_pending.load(Ordering::Acquire);
    release.send(()).unwrap(); // release the owned fixture before any discriminating assertion
    wait_until("pending request and settlement completed",||queue_len(&s)==0 && events(&fx,"queue.delivered").len()==1 && s.write_tx.gate().pending()==0);
    assert!(entered.is_ok(),"premise: receiver proof was held by the fixture barrier");
    assert_eq!(before,0);assert_eq!(events(&fx,"queue.delivered").len(),1);
    assert!(!premature_completion,"pending proof must not reset the starvation episode");
    assert!(s.queue_completion_pending.load(Ordering::Acquire),"terminal submission notifies the existing queue tick");
    assert_eq!(reply["delivered"],Value::from(false),"queue RPC acceptance is pending until the writer submits");
    assert_eq!(reply["written"],Value::from("pending"));
    assert!(reply["request_id"].as_str().is_some_and(|id|!id.is_empty()));
}

#[test]
fn c8pp_resumed_success_releases_original_reservation_after_settlement() {
    let fx=fx("c8pp-reservation-resume");let s=seat(&fx);
    let since=hold_exact(&fx,&s,&"resume reservation body ".repeat(500),1);
    let original=s.inject_reservation.lock().unwrap().as_ref().unwrap().guard.clone();
    assert!(!original.abort_if_pending(),"premise: original prefix keeps disposal protection");
    let _=crate::governance::queue_write_hold_block_at(&fx.daemon,&s,since+Duration::from_secs(1));
    assert_eq!(events(&fx,"queue.write_resumed").len(),1,"premise: resumed run entered the writer");
    wait_until("resumed writer and settlement finished",||s.write_tx.gate().pending()==0);
    assert_eq!(queue_len(&s),0,"resumed success releases the original reservation owner");
    assert!(!s.inject_reserved());assert!(hold_kind(&s).is_none());
    assert_eq!(events(&fx,"queue.delivered").len(),1);
}

// WP17: exercise the real queue selection/render/settlement API, so the old implementation
// fails at the singleton body assertion rather than at an unavailable new helper.
fn wp17_normal_handoff(fx: &Fx, s: &Arc<Surface>) -> crate::governance::QueueHandoff {
    let mut result = None;
    crate::state::test_done_or_no_pty_output(s, || {
        result = deliver_head_locked(&fx.daemon, s, false, false, None, Some(0), None, None, None);
        result.is_some()
    });
    result.expect("eligible queue head must progress")
}

#[test]
fn wp17_adjacent_commands_are_separate_submissions_and_keep_interval() {
    let fx = fx("wp17-pair");
    let s = seat(&fx);
    let first = enqueue(&fx, &s, "/remote-control test-master");
    let second = enqueue(&fx, &s, "/model opus");
    let one = wp17_normal_handoff(&fx, &s);
    assert_eq!(one.body, "/remote-control test-master", "slash head acquired digest text");
    assert_eq!(one.merged_ids, vec![first]);
    assert!(one.written_submitted());
    assert_eq!(queue_len(&s), 1);
    assert!(deliver_head_locked(&fx.daemon, &s, false, false, None, Some(0), None, None, None).is_none(),
        "command isolation must not bypass delivery pacing");
    *s.last_queue_delivery_at.lock().unwrap() = Some(Instant::now() - Duration::from_secs(60));
    let two = wp17_normal_handoff(&fx, &s);
    assert_eq!(two.body, "/model opus");
    assert_eq!(two.merged_ids, vec![second]);
    assert!(two.written_submitted());
    assert_eq!(queue_len(&s), 0);
    assert_eq!(events(&fx, "queue.delivered").len(), 2);
}

#[test]
fn wp17_text_digest_stops_at_command_without_skipping_it() {
    let fx = fx("wp17-mixed");
    let s = seat(&fx);
    let ids: Vec<_> = ["first", "second", "/model opus", "third", "fourth"]
        .iter().map(|t| enqueue(&fx, &s, t)).collect();
    let a = wp17_normal_handoff(&fx, &s);
    assert_eq!(a.merged_ids, ids[..2]);
    assert_eq!(a.body, "[큐 다이제스트 2건 · 발신 unknown]\n[1/2] first\n[2/2] second\n");
    assert_eq!(queue_len(&s), 3);
    *s.last_queue_delivery_at.lock().unwrap() = Some(Instant::now() - Duration::from_secs(60));
    let b = wp17_normal_handoff(&fx, &s);
    assert_eq!(b.body, "/model opus");
    assert_eq!(b.merged_ids, ids[2..3]);
    *s.last_queue_delivery_at.lock().unwrap() = Some(Instant::now() - Duration::from_secs(60));
    let c = wp17_normal_handoff(&fx, &s);
    assert_eq!(c.merged_ids, ids[3..]);
    assert_eq!(c.body, "[큐 다이제스트 2건 · 발신 unknown]\n[1/2] third\n[2/2] fourth\n");
    assert_eq!(queue_len(&s), 0);
}

#[test]
fn wp17_multiline_command_is_one_identity_and_byte_preserved() {
    let fx = fx("wp17-multiline");
    let s = seat(&fx);
    let body = " \t/model opus\r\n/remote-control 이름\nprose";
    let id = enqueue(&fx, &s, body);
    enqueue(&fx, &s, "later");
    let d = wp17_normal_handoff(&fx, &s);
    assert_eq!(d.body, body);
    assert_eq!(d.merged_ids, vec![id]);
    assert!(d.written_submitted());
    assert_eq!(queue_len(&s), 1);
    assert_eq!(events(&fx, "queue.delivered").len(), 1);
}

fn wp17_has_admission_record(fx: &Fx, reason: &str) -> bool {
    std::fs::read_to_string(crate::delivery::ledger_path(&fx.daemon.socket_path)).unwrap()
        .lines().filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .any(|v| v["slash_command_admission"] == reason)
}

#[test]
fn wp17_markerless_command_delivers_and_records_fallback() {
    let fx = fx("wp17-markerless");
    let s = seat(&fx);
    enqueue(&fx, &s, "/model opus");
    let d = wp17_normal_handoff(&fx, &s);
    assert_eq!(d.body, "/model opus");
    assert!(d.written_submitted());
    assert_eq!(queue_len(&s), 0);
    assert!(wp17_has_admission_record(&fx, "fallback_markerless"));
}

#[test]
fn wp17_unknown_hint_command_delivers_and_records_fallback() {
    let fx = fx("wp17-hint");
    let s = seat(&fx);
    // Cursor immediately after the marker; unknown suggestion is AFTER the cursor.
    s.parser.lock().unwrap().process("\x1b[2J\x1b[H❯ Unrecognised visual hint\x1b[1;3H".as_bytes());
    let recheck = crate::governance::ScreenRecheck {
        marker: Some(vec!["❯".into()]), placeholder: None,
        gen_at_verdict: s.output_gen.load(Ordering::Acquire), approval_pending: false,
    };
    enqueue(&fx, &s, "/model opus");
    let d = deliver_head_locked(&fx.daemon, &s, false, false, None, Some(0), None, Some(&recheck), None)
        .expect("head ruling: unknown hint keeps ordinary gate, not a new hold");
    assert_eq!(d.body, "/model opus");
    assert!(d.written_submitted());
    assert_eq!(queue_len(&s), 0);
    assert!(wp17_has_admission_record(&fx, "fallback_unrecognized_hint"));
}

#[test]
fn wp17_known_empty_command_records_proof_and_draft_then_empty_progresses() {
    let fx = fx("wp17-empty");
    let s = seat(&fx);
    enqueue(&fx, &s, "/model opus");
    let recheck = crate::governance::ScreenRecheck {
        marker: Some(vec!["❯".into()]), placeholder: None,
        gen_at_verdict: s.output_gen.load(Ordering::Acquire), approval_pending: false,
    };
    // Same generation deliberately exercises the fresh slash observation, even when the
    // ordinary handoff recheck considers its saved generation unchanged.
    s.parser.lock().unwrap().process("\x1b[2J\x1b[H❯ draft".as_bytes());
    let audit_before = crate::governance::h_ledger_count(&fx.daemon, "queue");
    assert!(deliver_head_locked(&fx.daemon, &s, false, false, None, Some(0), None, Some(&recheck), None).is_none());
    assert_eq!(crate::governance::h_ledger_count(&fx.daemon, "queue"), audit_before,
        "a later writer refusal cannot stand in for pre-publication admission");
    assert_eq!(queue_len(&s), 1);
    assert!(s.inject_track.last_body().is_none());
    s.parser.lock().unwrap().process("\x1b[2J\x1b[H❯ ".as_bytes());
    let d = deliver_head_locked(&fx.daemon, &s, false, false, None, Some(0), None, Some(&recheck), None)
        .expect("empty observed composer must progress");
    assert!(d.written_submitted());
    assert_eq!(queue_len(&s), 0);
    assert!(wp17_has_admission_record(&fx, "measured_empty"));
}

#[test]
fn wp17_markerless_pending_draft_keeps_ordinary_block_then_recovers() {
    let fx = fx("wp17-pending");
    let s = seat(&fx);
    enqueue(&fx, &s, "/model opus");
    s.pending_input_bytes.store(9, Ordering::Relaxed);
    assert!(deliver_head_locked(&fx.daemon, &s, true, false, None, Some(9), None, None, None).is_none(),
        "fallback must not bypass ordinary pending-input gate, even forced");
    assert_eq!(queue_len(&s), 1);
    assert!(s.inject_track.last_body().is_none());
    s.pending_input_bytes.store(0, Ordering::Relaxed);
    let d = wp17_normal_handoff(&fx, &s);
    assert!(d.written_submitted());
    assert_eq!(queue_len(&s), 0);
}

/// R1 S1: the cursor row and counter can both be empty while the recognized editor box
/// still contains a lower-row draft. Refuse before audit/body publication; clearing that
/// same region must let the original command progress without a new queue hold type.
#[test]
fn wp17_r2_known_multiline_draft_zero_counter_blocks_then_empty_delivers() {
    let fx = fx("wp17-r2-region");
    let rule = "────────────────────────────────────────";
    for forced in [false, true] {
        let s = seat(&fx);
        let id = enqueue(&fx, &s, "/model opus");
        s.parser.lock().unwrap().process(format!(
            "\x1b[2J\x1b[H{rule}\r\n❯ \r\n  real lower-row draft\r\n{rule}\r\n  ? for shortcuts\x1b[2;3H"
        ).as_bytes());
        assert_eq!(s.pending_input_bytes.load(Ordering::Relaxed), 0, "zero-counter premise");
        let recheck = crate::governance::ScreenRecheck {
            marker: Some(vec!["❯".into()]), placeholder: None,
            gen_at_verdict: s.output_gen.load(Ordering::Acquire), approval_pending: false,
        };
        let before = crate::governance::h_ledger_count(&fx.daemon, "queue");
        assert!(deliver_head_locked(&fx.daemon, &s, forced, false, None, Some(0), None, Some(&recheck), None).is_none(),
            "recognized occupied multiline region must refuse before publication, forced={forced}");
        assert_eq!(crate::governance::h_ledger_count(&fx.daemon, "queue"), before);
        assert!(s.inject_track.last_body().is_none());
        assert_eq!(s.pending_queue.lock().unwrap().front().unwrap().id, id);
        assert_eq!(s.queue_blocked.lock().unwrap().as_ref().unwrap().0, crate::governance::BLOCKED_INPUT_PENDING);
        assert!(hold_kind(&s).is_none(), "no new writer hold/retry state");
        s.parser.lock().unwrap().process(format!(
            "\x1b[2J\x1b[H{rule}\r\n❯ \r\n{rule}\r\n  ? for shortcuts\x1b[2;3H"
        ).as_bytes());
        let d = deliver_head_locked(&fx.daemon, &s, forced, false, None, Some(0), None, Some(&recheck), None)
            .expect("cleared recognized region must progress");
        assert!(d.written_submitted());
        assert_eq!(d.merged_ids, vec![id]);
        assert_eq!(d.body, "/model opus");
        assert_eq!(queue_len(&s), 0);
    }
}

#[test]
fn wp17_r2_unknown_lower_layout_and_framed_hint_keep_fallback() {
    let fx = fx("wp17-r2-unknown");
    let rule = "────────────────────────────────────────";
    for (paint, reason) in [
        ("\x1b[2J\x1b[H❯ \r\n  unfamiliar widget without editor bounds\x1b[1;3H".to_string(),
            "fallback_unrecognized_layout"),
        (format!("\x1b[2J\x1b[H{rule}\r\n❯ unfamiliar inline hint\r\n{rule}\r\n  ? for shortcuts\x1b[2;3H"),
            "fallback_unrecognized_hint"),
    ] {
        let s = seat(&fx);
        enqueue(&fx, &s, "/model opus");
        s.parser.lock().unwrap().process(paint.as_bytes());
        let recheck = crate::governance::ScreenRecheck {
            marker: Some(vec!["❯".into()]), placeholder: None,
            gen_at_verdict: s.output_gen.load(Ordering::Acquire), approval_pending: false,
        };
        let d = deliver_head_locked(&fx.daemon, &s, false, false, None, Some(0), None, Some(&recheck), None)
            .expect("unknown layout/hint retains ordinary progress");
        assert!(d.written_submitted());
        assert_eq!(queue_len(&s), 0);
        assert!(wp17_has_admission_record(&fx, reason));
    }
}

/// Gemini R1 shape (b): the cursor is on a pasted marker row inside the editor, below
/// an upper-row draft. The actual cursor anchor must not hide text above it in the box.
#[test]
fn wp17_r2_upper_draft_marker_row_zero_counter_blocks_before_ledger() {
    let fx = fx("wp17-r2-upper");
    let rule = "────────────────────────────────────────";
    for forced in [false, true] {
        let s = seat(&fx);
        let id = enqueue(&fx, &s, "/model opus");
        s.parser.lock().unwrap().process(format!(
            "\x1b[2J\x1b[H{rule}\r\n❯ upper draft\r\n\r\n❯ \r\n{rule}\r\n  ? for shortcuts\x1b[4;3H"
        ).as_bytes());
        assert_eq!(s.pending_input_bytes.load(Ordering::Relaxed), 0);
        let recheck = crate::governance::ScreenRecheck {
            marker: Some(vec!["❯".into()]), placeholder: None,
            gen_at_verdict: s.output_gen.load(Ordering::Acquire), approval_pending: false,
        };
        let audit_before = crate::governance::h_ledger_count(&fx.daemon, "queue");
        assert!(deliver_head_locked(&fx.daemon, &s, forced, false, None, Some(0), None, Some(&recheck), None).is_none(),
            "upper-row draft inside recognized box must refuse before publication, forced={forced}");
        assert_eq!(crate::governance::h_ledger_count(&fx.daemon, "queue"), audit_before);
        assert!(s.inject_track.last_body().is_none());
        assert_eq!(s.pending_queue.lock().unwrap().front().unwrap().id, id);
        assert_eq!(s.queue_blocked.lock().unwrap().as_ref().unwrap().0, crate::governance::BLOCKED_INPUT_PENDING);
        assert!(hold_kind(&s).is_none());
        s.parser.lock().unwrap().process(format!(
            "\x1b[2J\x1b[H{rule}\r\n❯ \r\n{rule}\r\n  ? for shortcuts\x1b[2;3H"
        ).as_bytes());
        let d = deliver_head_locked(&fx.daemon, &s, forced, false, None, Some(0), None, Some(&recheck), None)
            .expect("empty box must release ordinary delivery eligibility");
        assert!(d.written_submitted());
        assert_eq!(d.merged_ids, vec![id]);
        assert_eq!(queue_len(&s), 0);
    }
}

// ─── ★(0.14.56 · P1 D9 pre-CR · design r3 §6) master/CEO queue jobs: the R-1 window (hand-off -> first byte) is CHECKED at the
//   writer's claim (§6-2); the body -> CR settle is ELIMINATED by construction (§6-3). Recorder seats: the receiver's bytes are exact. ──

/// A recorder seat that is this daemon's `master` and an agent seat, so `d9_scope` holds; its receiver proof always admits.
fn d9_master_recorder(fx: &Fx, tag: &str) -> (Arc<Surface>, std::path::PathBuf) {
    let (s, out) = recorder_seat(fx, tag);
    fx.daemon.roles.lock().unwrap().insert("master".into(), s.id);
    *s.agent_meta.lock().unwrap() = Some(("claude".into(), "claude".into()));
    *s.receiver_proof.lock().unwrap() = Some(Scripted::new(0, ProofLoss::AgentDead, None));
    assert!(crate::governance::d9_scope(&fx.daemon, &s, false), "premise: the seat is in D9 scope");
    (s, out)
}

/// Holds the seat's writer inside a job's own admission (after that job's claim, before any byte) until released, then
/// refuses it: zero bytes, so the holder itself moves none of the three counters.
struct HoldWriter {
    entered: std::sync::mpsc::Sender<()>,
    release: Mutex<std::sync::mpsc::Receiver<()>>,
}

impl Proof for HoldWriter {
    fn admit(&self) -> Result<(), ProofLoss> {
        let _ = self.entered.send(());
        let _ = self.release.lock().unwrap().recv_timeout(Duration::from_secs(10));
        Err(ProofLoss::Unknown("test:hold_writer".into()))
    }
    fn recheck(&self, _: Phase) -> Result<(), ProofLoss> {
        Ok(())
    }
}

/// Puts a [`HoldWriter`] job at the front of the seat's writer FIFO and waits until the writer is inside it.
pub(crate) fn hold_writer(s: &Arc<Surface>) -> std::sync::mpsc::Sender<()> {
    let (entered, observed) = std::sync::mpsc::channel();
    let (release, wait) = std::sync::mpsc::channel();
    let job = crate::agent_write::AgentJob {
        attempt: None,
        request: crate::agent_write::Request::new(crate::agent_write::new_request_id(1), s.id, "test", crate::delivery::digest_text("hold")),
        plan: crate::agent_write::Plan::paste("hold"),
        proof: Arc::new(HoldWriter { entered, release: Mutex::new(wait) }),
        journal: None,
        resume: None,
        settle: None,
        guard: None,
        note_body: false,
        body_owner: None,
    };
    s.write_tx.send(crate::state::WriteReq::Agent(Box::new(job))).unwrap();
    observed.recv_timeout(Duration::from_secs(5)).expect("premise: the writer is held inside the job ahead");
    release
}

/// The `queue.inject_aborted` events of one seat.
fn aborted(fx: &Fx, s: &Arc<Surface>) -> Vec<Value> {
    seat_events(fx, s, "queue.inject_aborted")
}

/// T-W1 (design r3 §12; §6-2 `wrote`): a write queued AHEAD of a master queue job is written between the job's hand-off and its
/// claim (here a terminal-reply-like `Data` request, the render-lag case: bytes on the PTY the screen may not show yet). The claim
/// refuses: 0 bytes of the job reach the receiver, the item stays at the head, and the next attempt delivers it exactly once.
/// Mutant: drop `wrote` from the claim check.
#[test]
#[cfg_attr(not(unix), ignore = "좌석 명령이 POSIX 셸 문법")]
fn d9_pre_cr_t_w1_a_write_ahead_inside_the_r1_window_refuses_the_claim() {
    let fx = fx("d9-tw1");
    let mut recorder = p1_master_recorder(&fx, "tw1"); let s = recorder.surface.clone(); let out = recorder.output.clone();
    let id = enqueue(&fx, &s, "[보고] tw1");
    let held = p1_late_hold(&s, Some(b"T"));
    let w0 = s.inject_track.wrote();
    let action = p1_queue_action(&s, move |_,_| { held.lock().unwrap().take().expect("late seam holder").release(); });
    let _handed = hand_off(&fx, &s);
    action.finish();
    p1_wait_completion(p1_writer_idle(&s), P1_GUARD).expect("writer operation completed");
    assert_eq!(aborted(&fx, &s).len(), 1, "the claim after a write ahead inside the R-1 window must be refused");
    assert_eq!(s.inject_track.wrote(), w0 + 1, "premise: exactly the write ahead was counted");
    assert_eq!(recorder.barrier(), b"T".to_vec(), "only the write ahead reached the receiver (0 bytes of the job)");
    assert_eq!(s.pending_queue.lock().unwrap().front().map(|e| e.id.clone()), Some(id.clone()), "the item stays at the head");
    let ab = aborted(&fx, &s);
    assert_eq!(ab.len(), 1, "{ab:?}");
    assert_eq!(ab[0]["payload"]["foreign"], serde_json::json!(["wrote"]), "only the write count moved: {ab:?}");
    assert!(events(&fx, "queue.delivered").is_empty());
    // The next attempt is a fresh judgement: delivered once.
    assert!(hand_off(&fx, &s), "the next attempt hands off");
    p1_wait_completion(p1_writer_idle(&s), P1_GUARD).expect("delivery operation completed"); assert_eq!(queue_len(&s), 0);
    let mut want = b"T".to_vec();
    want.extend(queue_bytes("[보고] tw1"));
    // P1Recorder Drop stops this test-owned child after the final ACK/assertions.
    assert_eq!(recorder.barrier(), want, "the job's bytes once, after the write ahead");
    let delivered = events(&fx, "queue.delivered");
    assert_eq!(delivered.len(), 1);
    assert_eq!(delivered[0]["payload"]["queue_entry_id"], Value::from(id));
}

/// T-W2 (design r3 §12; §6-2 `last_human_input`, `input_gen`): a human key received between the hand-off and the claim of a master
/// queue job (the writer is busy with a job ahead that writes nothing) -> the claim refuses: 0 bytes of the job, the key is
/// written after it as queued, the item stays at the head. The key is Ctrl-U: it leaves the pending count at 0 (V2 resets on it,
/// V3 ignores BS/DEL), so the pin's claim probe - which reads that count - lets the claim through; a Backspace on this V2 recorder
/// seat would be counted and refused by the pin already (measured: RED-stageD1-on-24fe6a7b.log). Mutant: compare neither handler
/// counter.
#[test]
#[cfg_attr(not(unix), ignore = "좌석 명령이 POSIX 셸 문법")]
fn d9_pre_cr_t_w2_a_human_key_inside_the_r1_window_refuses_the_claim() {
    let fx = fx("d9-tw2");
    let mut recorder = p1_master_recorder(&fx, "tw2"); let s = recorder.surface.clone(); let out = recorder.output.clone();
    let id = enqueue(&fx, &s, "[보고] tw2");
    let held=p1_late_hold(&s,None);let w0=s.inject_track.wrote();
    let gen0=s.input_gen.load(Ordering::Acquire);let daemon=fx.daemon.clone();
    let action=p1_queue_action(&s,move |sc,_| {
        let _async=crate::agent_write::p1_sync_settle(false);
        let req=cys::Request{id:Value::from(1),method:"surface.send_text".into(),params:serde_json::json!({"surface_id":sc.id,"text":"\u{15}","human":true,"quiet":true})};
        let reply=match crate::handlers::dispatch(&daemon,req,None){crate::handlers::Reply::Single(v)=>v,_=>panic!("single reply")};
        assert_eq!(reply["ok"],true,"human request admission");
        held.lock().unwrap().take().expect("late holder").release();
        reply
    });
    let _handed = hand_off(&fx, &s);
    let reply = action.finish();
    p1_wait_completion(p1_writer_idle(&s), P1_GUARD).expect("writer operation completed");
    assert_ne!(s.input_gen.load(Ordering::Acquire), gen0, "premise: input accounting moved");
    assert_eq!(reply["ok"], Value::from(true), "premise: the keystroke was received: {reply}");
    assert_eq!(aborted(&fx, &s).len(), 1, "the claim after a human key inside the R-1 window must be refused");
    assert_eq!(s.inject_track.wrote(), w0 + 1, "premise: only the key was written (after the refusal)");
    assert_eq!(s.pending_input_bytes.load(Ordering::Relaxed), 0, "premise: the key left the pending count at 0");
    assert_eq!(recorder.barrier(), b"\x15".to_vec(), "0 bytes of the job reached the receiver");
    assert_eq!(s.pending_queue.lock().unwrap().front().map(|e| e.id.clone()), Some(id), "the item stays at the head");
    let ab = aborted(&fx, &s);
    assert_eq!(ab.len(), 1, "{ab:?}");
    assert_eq!(ab[0]["payload"]["foreign"], serde_json::json!(["input_gen", "last_human_input"]), "{ab:?}");
    // P1Recorder Drop stops this test-owned child after the final ACK/assertions.
}

/// Arms the pre-record seam (`agent_write::set_test_after_pre_record`, D3-BOUND r2 §1.5) to admit one foreign `Data` write on `s`
/// and wait until it has FINISHED (the gate is idle again) before the producer continues to its hand-off.
pub(crate) fn d9_s0_foreign_write_after_pre_record(s: &Arc<Surface>, bytes: &'static [u8]) {
    let s = s.clone();
    crate::agent_write::set_test_after_pre_record(move |_| {
        s.write_tx.send(crate::state::WriteReq::Data(bytes.to_vec())).unwrap();
        wait_until("the foreign write finished inside the window", || s.write_tx.gate().pending() == 0);
    });
}

/// T-S0 (D3-BOUND r2 §1.5, F1): a foreign `Data` write admitted AFTER the queue's judgment and ledger pre-record and FINISHED
/// before the hand-off. The claim baseline is S0, read before the judgment, so the finished write is seen: the claim refuses
/// (`foreign` = ["wrote"]), 0 bytes of the job, the item stays at the head; the next attempt delivers it once. On the D3 bytes
/// the baseline was read after that window and the job was written over the foreign write (the RED). Mutant MS1: base from the
/// late counters.
#[test]
#[cfg_attr(not(unix), ignore = "좌석 명령이 POSIX 셸 문법")]
fn d9_bound_t_s0_a_write_finished_between_judgment_and_hand_off_refuses_the_claim() {
    let fx = fx("d9-ts0");
    let mut recorder = p1_master_recorder(&fx, "ts0"); let s = recorder.surface.clone(); let out = recorder.output.clone();
    let id = enqueue(&fx, &s, "[보고] ts0");
    let w0 = s.inject_track.wrote();
    p1_s0_foreign_write_after_pre_record(&s, b"F");
    let _handed = hand_off(&fx, &s);
    p1_wait_completion(p1_writer_idle(&s), P1_GUARD).expect("writer operation completed");
    // The discriminating assertions first (a written job also moves `wrote`), then the premise.
    assert_eq!(aborted(&fx, &s).len(), 1, "the claim after a write finished between the judgment and the hand-off must be refused");
    assert_eq!(s.inject_track.wrote(), w0 + 1, "premise: the foreign write was counted inside the window");
    assert_eq!(recorder.barrier(), b"F".to_vec(), "0 bytes of the job - only the foreign write");
    assert_eq!(s.pending_queue.lock().unwrap().front().map(|e| e.id.clone()), Some(id.clone()), "the item stays at the head");
    let ab = aborted(&fx, &s);
    assert_eq!(ab.len(), 1, "{ab:?}");
    assert_eq!(ab[0]["payload"]["foreign"], serde_json::json!(["wrote"]), "only the write count moved: {ab:?}");
    // Next attempt (no seam): delivered once, after the foreign bytes.
    assert!(hand_off(&fx, &s), "the next attempt hands off");
    p1_wait_completion(p1_writer_idle(&s), P1_GUARD).expect("delivery operation completed"); assert_eq!(queue_len(&s), 0);
    let mut want = b"F".to_vec();
    want.extend(queue_bytes("[보고] ts0"));
    // P1Recorder Drop stops this test-owned child after the final ACK/assertions.
    assert_eq!(recorder.barrier(), want, "the job's bytes once, after the foreign write");
}

/// T-S0h (D3-BOUND r2 §1.5): as T-S0, but the window carries a human key time (`last_human_input` set, nothing written) -> the
/// claim refuses naming `last_human_input` only, 0 bytes of the job.
#[test]
#[cfg_attr(not(unix), ignore = "좌석 명령이 POSIX 셸 문법")]
fn d9_bound_t_s0h_a_human_key_time_between_judgment_and_hand_off_refuses_the_claim() {
    let fx = fx("d9-ts0h");
    let mut recorder = p1_master_recorder(&fx, "ts0h"); let s = recorder.surface.clone(); let out = recorder.output.clone();
    let id = enqueue(&fx, &s, "[보고] ts0h");
    {
        let s = s.clone();
        crate::agent_write::set_test_after_pre_record(move |_| {
            *s.last_human_input.lock().unwrap() = Some(Instant::now());
        });
    }
    let _handed = hand_off(&fx, &s);
    p1_wait_completion(p1_writer_idle(&s), P1_GUARD).expect("writer operation completed");
    assert_eq!(aborted(&fx, &s).len(), 1, "the claim after a human key time set between the judgment and the hand-off must be refused");
    assert_eq!(recorder.barrier(), Vec::<u8>::new(), "0 bytes of the job");
    assert_eq!(s.pending_queue.lock().unwrap().front().map(|e| e.id.clone()), Some(id), "the item stays at the head");
    let ab = aborted(&fx, &s);
    assert_eq!(ab.len(), 1, "{ab:?}");
    assert_eq!(ab[0]["payload"]["foreign"], serde_json::json!(["last_human_input"]), "{ab:?}");
    // P1Recorder Drop stops this test-owned child after the final ACK/assertions.
}

/// T-L1 (design r3 §6-3, premise specimen): a human byte received DURING the body -> CR settle of a master queue job reaches the
/// receiver AFTER the job's own CR, and the job's body is submitted exactly (one writer job owns body, settle and CR; no other
/// request is dequeued in between). The specimen asserts the byte was sent while the CR was still unwritten. Mutant: split the CR
/// into its own later request.
#[test]
#[cfg_attr(not(unix), ignore = "좌석 명령이 POSIX 셸 문법")]
fn d9_pre_cr_t_l1_a_human_byte_during_the_settle_lands_after_the_cr() {
    let fx = fx("d9-tl1");
    let _async = crate::agent_write::p1_sync_settle(false);
    let mut recorder=p1_master_recorder(&fx,"tl1");let s=recorder.surface.clone();
    let body="[보고] tl1";enqueue(&fx,&s,body);
    let want=queue_bytes(body);let before_cr=want[..want.len()-1].to_vec();
    let (release,held)=std::sync::mpsc::channel();
    let paused=Arc::new(std::sync::atomic::AtomicBool::new(false));
    let pause_state=paused.clone();
    let entered=p1_start_completion("admitted queue body acknowledged before held CR",|tx| {
        *s.receiver_proof.lock().unwrap()=Some(Arc::new(P1PauseAtSubmit {
            entered:Mutex::new(Some(tx)),release:Mutex::new(held),paused:pause_state,
        }));
        if hand_off(&fx,&s) { Ok(()) } else { Err("queue handoff refused".into()) }
    });
    p1_wait_completion(entered,P1_GUARD).expect("actual admitted job reached preCR rendezvous");
    assert_eq!(recorder.received_prefix(before_cr.len()),before_cr,"receiver acknowledged complete body before CR");
    let req=cys::Request{id:Value::from(1),method:"surface.send_text".into(),params:serde_json::json!({"surface_id":s.id,"text":"Z","human":true,"quiet":true})};
    let reply=match crate::handlers::dispatch(&fx.daemon,req,None){crate::handlers::Reply::Single(v)=>v,_=>panic!("single reply")};
    let inside_settle=paused.load(Ordering::Acquire);
    assert_eq!(p1_wait_completion(p1_writer_idle(&s),Duration::ZERO),Err(P1WaitError::CompletionTimeout {event:"writer operation leases released"}),"actual preCR producer remains held until explicit release");
    assert_eq!(reply["ok"],Value::from(true),"premise: the byte was received: {reply}");
    assert!(inside_settle,"premise: the human byte was sent while the job's CR was still unwritten");
    release.send(()).expect("explicit preCR release");
    p1_wait_completion(p1_writer_idle(&s),P1_GUARD).expect("queue and admitted human byte settled");
    let mut expect=want.clone();expect.push(b'Z');
    assert_eq!(recorder.barrier(),expect,"the job's body and CR exactly, then the human byte");
    assert_eq!(queue_len(&s),0);
    assert_eq!(events(&fx,"queue.delivered").len(),1);
}

// ── ★(0.14.56 · P1 D9 pre-CR · design r3 §6-4 W-1) At the CR: a master/CEO queue job carries the pin's CR withhold probe, and
//   the probe writes NO resubmit record - the queue hold's own resume is the one owner of a withheld CR. ──

/// Paints the permission dialog straight into the seat's parser, the cursor on its `❯ 1. Yes` row (the shape of
/// `send_settle_tests::paint` / `DIALOG`); `lines` empty = a blank frame.
fn d9_paint(s: &Arc<Surface>, lines: &[&str], row: u16, col: u16) {
    let mut p = s.parser.lock().unwrap_or_else(|e| e.into_inner());
    p.process(b"\x1b[2J\x1b[H");
    for (i, l) in lines.iter().enumerate() {
        p.process(format!("\x1b[{};1H{}", i + 1, l).as_bytes());
    }
    p.process(format!("\x1b[{};{}H", row + 1, col + 1).as_bytes());
}

const D9_DIALOG: [&str; 4] = ["Do you want to proceed?", "❯ 1. Yes", "  2. No", "  Esc to cancel"];

/// The seat's receiver proof for T-W3: always admits; when the writer reaches the Return phase (after the body, before the
/// CR withhold probe looks) it paints [`D9_DIALOG`] - ONCE, so the resume of the same request meets the dialog only if the
/// specimen paints it again.
struct DialogAtSubmitOnce {
    seat: std::sync::Weak<Surface>,
    painted: std::sync::atomic::AtomicBool,
}

impl Proof for DialogAtSubmitOnce {
    fn admit(&self) -> Result<(), ProofLoss> {
        Ok(())
    }
    fn recheck(&self, phase: Phase) -> Result<(), ProofLoss> {
        if phase == Phase::Submit && !self.painted.swap(true, Ordering::AcqRel) {
            if let Some(s) = self.seat.upgrade() {
                d9_paint(&s, &D9_DIALOG, 1, 8);
            }
        }
        Ok(())
    }
}

/// One T-W3 seat: a recorder seat with a claude agent meta, its dialog premise measured, a blank frame, the dialog-at-Return
/// proof installed. `master` = make it this daemon's master (D9 scope) or leave it role-less (outside D9 scope).
fn d9_dialog_seat(fx: &Fx, tag: &str, master: bool) -> (Arc<Surface>, std::path::PathBuf) {
    let (s, out) = if master {
        d9_master_recorder(fx, tag)
    } else {
        let (s, out) = recorder_seat(fx, tag);
        *s.agent_meta.lock().unwrap() = Some(("claude".into(), "claude".into()));
        assert!(!crate::governance::d9_scope(&fx.daemon, &s, false), "premise: the seat is outside D9 scope");
        (s, out)
    };
    // Premise: on this seat a CR now would press this dialog (the pin's own observation).
    d9_paint(&s, &D9_DIALOG, 1, 8);
    let ctx = crate::governance::submit_cr_guard_context(&s).expect("premise: the seat's prompt markers resolve");
    assert!(ctx.dialog_at_handoff, "premise: the withhold probe sees this dialog");
    d9_paint(&s, &[], 0, 0);
    *s.receiver_proof.lock().unwrap() =
        Some(Arc::new(DialogAtSubmitOnce { seat: Arc::downgrade(&s), painted: std::sync::atomic::AtomicBool::new(false) }));
    (s, out)
}

/// T-W3 (design r3 §12; §6-4 W-1): a permission dialog rises during a master queue job's settle - after its body, before its
/// CR. The CR is withheld (`queue.submit_withheld`, armed_by `queue_job`); the job holds PartialExact with the body written;
/// NO resubmit record exists, so the resubmit tick writes nothing; once the dialog has closed the hold's own resume writes
/// exactly the one CR. Mutants: do not arm the probe on queue jobs; arm it WITH a resubmit record.
#[test]
#[cfg_attr(not(unix), ignore = "좌석 명령이 POSIX 셸 문법")]
fn d9_pre_cr_t_w3_a_dialog_during_the_settle_withholds_the_cr_with_one_owner() {
    let fx=fx("d9-tw3");let mut recorder=p1_dialog_recorder(&fx,"tw3",true);let s=recorder.surface.clone();
    let body="[보고] tw3";enqueue(&fx,&s,body);
    assert!(hand_off(&fx,&s),"premise: handed off");
    p1_wait_completion(p1_writer_idle(&s),P1_GUARD).expect("withheld terminal settled");
    let want=queue_bytes(body);assert_eq!(want.last(),Some(&b'\r'),"premise: the job's own Return is its last byte");
    let body_bytes=want[..want.len()-1].to_vec();
    assert_eq!(recorder.barrier(),body_bytes,"the body only - a CR here presses `1. Yes`");
    let w=seat_events(&fx,&s,"queue.submit_withheld");assert_eq!(w.len(),1,"one withhold: {w:?}");
    assert_eq!(w[0]["payload"]["armed_by"],serde_json::json!("queue_job"),"{w:?}");
    assert_eq!(hold_kind(&s),Some(QueueHoldKind::PartialExact),"the item is held byte-exact");assert_eq!(queue_len(&s),1);
    assert!(s.inject_track.withheld().is_none(),"a resubmit record would be a second owner of this CR");
    crate::governance::resubmit_withheld_submits(&fx.daemon);
    assert_eq!(recorder.barrier(),body_bytes,"the resubmit tick wrote");
    crate::governance::h_paint(&s,crate::governance::H_IDLE_SCREEN);
    let since=s.queue_hold.lock().unwrap().as_ref().map(|h|h.since).expect("held");
    let _=crate::governance::queue_write_hold_block_at(&fx.daemon,&s,since+Duration::from_secs(1));
    assert_eq!(events(&fx,"queue.write_resumed").len(),1,"synchronous successful resume admission receipt");
    p1_wait_completion(p1_writer_idle(&s),P1_GUARD).expect("admitted resume settled");
    assert_eq!(queue_len(&s),0);
    assert_eq!(recorder.barrier(),want,"the body, then exactly one CR");
    assert_eq!(events(&fx,"queue.delivered").len(),1);
}

/// T-W3 scope control: the same dialog on a queue job OUTSIDE D9 scope (role-less agent seat) - the job's proof is the
/// pin's, unchanged in this release (design r3 §10-5): no withhold. Mutant: arm the probe on every queue job.
#[test]
#[cfg_attr(not(unix), ignore = "좌석 명령이 POSIX 셸 문법")]
fn d9_pre_cr_t_w3c_outside_d9_scope_the_queue_job_proof_is_unchanged() {
    let fx=fx("d9-tw3c");let mut recorder=p1_dialog_recorder(&fx,"tw3",false);let s=recorder.surface.clone();
    let body="[보고] tw3c";enqueue(&fx,&s,body);
    assert!(hand_off(&fx,&s),"premise: handed off");
    p1_wait_completion(p1_writer_idle(&s),P1_GUARD).expect("outside scope terminal settled");
    assert!(seat_events(&fx,&s,"queue.submit_withheld").is_empty(),"a withhold outside D9 scope");
    let want=queue_bytes(body);assert_eq!(recorder.barrier(),want,"the pin's bytes");
    assert_eq!(queue_len(&s),0);
}

// ── ★(0.14.56 · P1 D9 pre-CR · design r3 §5-7) Shared helpers for T-W4, whose specimens live beside each daemon-internal
//   writer into master/CEO (W-5 handlers, W-6 channels, W-7 boot_supervisor, W-8 schedule - the producers are private). ──

/// A recorder seat on `daemon` that holds `role` there and is an agent seat (D9 scope); its receiver proof always admits.
pub(crate) fn d9_role_recorder(daemon: &Arc<Daemon>, dir: &std::path::Path, tag: &str, role: &str) -> (Arc<Surface>, std::path::PathBuf) {
    let out = dir.join(format!("rx-{tag}.bin"));
    let cmd = format!("stty raw -echo; exec cat > '{}'", out.display());
    let s = daemon.create_surface(None, Some(cmd), None, None, 24, 200).expect("recorder seat");
    daemon.surfaces.lock().unwrap().insert(s.id, s.clone());
    wait_until("recorder ready", || out.exists());
    std::thread::sleep(Duration::from_millis(300)); // stty applied before the first byte
    daemon.roles.lock().unwrap().insert(role.into(), s.id);
    *s.agent_meta.lock().unwrap() = Some(("claude".into(), "claude".into()));
    *s.receiver_proof.lock().unwrap() = Some(Scripted::new(0, ProofLoss::AgentDead, None));
    assert!(crate::governance::d9_scope(daemon, &s, false), "premise: the seat is in D9 scope");
    (s, out)
}

/// Runs `call` (one daemon-internal hand-off into `s`) while a one-byte write `T` sits AHEAD of its job in the seat's writer:
/// the writer is held inside a refusing zero-byte job until both are queued, so `T` is written between the call's hand-off
/// and its claim. Returns once the writer has taken both.
pub(crate) fn d9_with_write_ahead<R>(s: &Arc<Surface>, call: impl FnOnce() -> R) -> R {
    let release = hold_writer(s);
    s.write_tx.send(crate::state::WriteReq::Data(b"T".to_vec())).unwrap();
    // Released only when the call's own job has joined the writer's queue (one more than with `T` alone) - the count's
    // absolute value (whether the held job is in it) is not assumed.
    let with_t = s.write_tx.gate().pending();
    let releaser = {
        let s = s.clone();
        std::thread::spawn(move || {
            wait_until("the call's job queued behind the write ahead", || s.write_tx.gate().pending() > with_t);
            release.send(()).unwrap();
        })
    };
    let r = call();
    releaser.join().unwrap();
    wait_until("the writer took both", || s.write_tx.gate().pending() == 0);
    r
}

/// The receiver's bytes once it holds at least `n` (300 ms grace for anything more).
pub(crate) fn d9_received(out: &std::path::Path, n: usize) -> Vec<u8> {
    wait_until("the receiver's bytes", || std::fs::metadata(out).map(|m| m.len() as usize).unwrap_or(0) >= n);
    std::thread::sleep(Duration::from_millis(300));
    std::fs::read(out).unwrap()
}

// ── ★(0.14.56 · PKG1-M1 · DESIGN-ADDENDUM-PKG1-M1.md sec 5) Shared helpers: the cycle's step-7 [RESUME] as the daemon receives it
//   (one converted authoritative queue job), and the order of bodies on the receiver's wire. ──

/// Sends `text` into `s` the way the cycle's step 7 does (`inject_text`: `authoritative:true`, quiet) from a CSO pane, so the
/// daemon's `authoritative_caller_ok` passes, and returns the id of the ONE authoritative queue job D9 converted it into.
pub(crate) fn pkg1_authoritative_job(daemon: &Arc<Daemon>, s: &Arc<Surface>, text: &str) -> String {
    static PID: AtomicU32 = AtomicU32::new(997_500);
    let pid = PID.fetch_add(1, Ordering::Relaxed);
    let cso = daemon.create_surface(None, Some("sleep 30".into()), None, Some("cso".into()), 24, 80).expect("cso pane");
    daemon.surfaces.lock().unwrap().insert(cso.id, cso.clone());
    daemon.caller_cache.lock().unwrap().insert(
        pid,
        crate::state::CallerCacheEntry::new(Some(cso.id), crate::state::now_epoch(), None, daemon.caller_gen.load(Ordering::Relaxed)),
    );
    let req = cys::Request {
        id: serde_json::json!(1),
        method: "surface.send_text".into(),
        params: serde_json::json!({"surface_id": s.id, "text": text, "quiet": true, "authoritative": true}),
    };
    let crate::handlers::Reply::Single(resp) = crate::handlers::dispatch(daemon, req, Some(pid)) else {
        panic!("expected a single reply");
    };
    assert_eq!(resp["result"]["converted"], serde_json::json!("one_job"), "premise: the step-7 send was converted: {resp}");
    let id = resp["result"]["queue_entry_id"].as_str().expect("queue entry id").to_string();
    assert!(
        s.pending_queue.lock().unwrap().iter().any(|e| e.id == id && e.authoritative),
        "premise: the converted job is authoritative"
    );
    id
}

/// An authoritative entry put straight into the queue (as a WAL restore leaves it - no in-memory record of its insertion)
/// whose wall-clock age is `age_secs`. Placed at the front, as `authoritative_insert_pos` places one on an empty queue.
pub(crate) fn pkg1_raw_authoritative(daemon: &Arc<Daemon>, s: &Arc<Surface>, text: &str, age_secs: f64) -> String {
    let mut e = daemon.next_queue_entry(text.into(), None, "send");
    e.authoritative = true;
    e.enqueued_at = crate::state::now_epoch() - age_secs;
    let id = e.id.clone();
    s.pending_queue.lock().unwrap().push_front(e);
    id
}

/// Hands the queue's head off (the tick's write step) until the queue is empty, at most `max` times.
pub(crate) fn pkg1_drain(daemon: &Arc<Daemon>, s: &Arc<Surface>, max: usize) {
    for _ in 0..max {
        if s.pending_queue.lock().unwrap().is_empty() {
            return;
        }
        wait_until("writer idle before the next hand-off", || s.write_tx.gate().pending() == 0);
        let _ = deliver_head_locked(daemon, s, true, false, None, None, None, None, None);
        wait_until("writer idle after the hand-off", || s.write_tx.gate().pending() == 0);
    }
}

/// Where `needle` first occurs in `hay` (the receiver's bytes).
pub(crate) fn pkg1_at(hay: &[u8], needle: &str) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle.as_bytes())
}

/// The `queue.authoritative_hold_lapsed` events published for `sid`.
pub(crate) fn pkg1_lapse_events(daemon: &Arc<Daemon>, sid: u64) -> Vec<Value> {
    daemon
        .bus
        .tail(1000)
        .into_iter()
        .filter(|e| e["name"] == "queue.authoritative_hold_lapsed" && e["payload"]["surface_ref"] == cys::surface_ref(sid))
        .collect()
}

/// S-Q1 / S-R16: admitted foreign work holds all five attempts without a ledger pre-record or claim refusal.
#[test]
#[cfg_attr(not(unix), ignore = "좌석 명령이 POSIX 셸 문법")]
fn d9_bound_i2_queue_busy_holds_then_first_idle_pass_delivers() {
    let fx = fx("i2-q1");
    let mut recorder = p1_master_recorder(&fx, "i2-q1"); let s = recorder.surface.clone(); let out = recorder.output.clone();
    enqueue(&fx, &s, "i2-q1");
    let release = p1_hold_writer(&s);
    // The same bounded sender used for PTY terminal replies admits this behind the held arm.
    assert!(crate::state::send_write_req_bounded(&s.write_tx, crate::state::WriteReq::Data(b"T".to_vec()), Duration::from_millis(250)));
    for k in 0..5 {
        let _ = hand_off(&fx, &s);
        assert_eq!(crate::governance::h_ledger_count(&fx.daemon, "queue"), 0, "S-Q1: zero ledger pre-records during hold {k}");
        let journal = std::fs::read_to_string(crate::state::state_dir(&fx.daemon.socket_path).join(crate::state::WRITER_JOURNAL_FILE)).expect("queue attempt opened its journal");
        let writing = journal.lines().filter_map(|x| serde_json::from_str::<Value>(x).ok()).filter(|x| x["rec"] == "writing" && x["origin"] == "queue").count();
        assert_eq!(writing, 0, "S-Q1: no queue journal writing during hold");
        assert!(aborted(&fx, &s).is_empty(), "S-Q1: hold is not refusal");
        assert!(s.queue_blocked.lock().unwrap().as_ref().is_some_and(|b| b.0.starts_with("writer_active")), "S-Q1: writer_active is visible");
    }
    release.release();
    p1_wait_completion(p1_writer_idle(&s), P1_GUARD).expect("writer operation completed");
    assert!(hand_off(&fx, &s), "S-Q1: first idle pass delivers");
    p1_wait_completion(p1_writer_idle(&s), P1_GUARD).expect("delivery operation completed"); assert_eq!(queue_len(&s), 0);
    let mut expected = b"T".to_vec(); expected.extend(queue_bytes("i2-q1"));
    assert_eq!(recorder.barrier(), expected, "S-Q1: exact body and one Return");
}

/// S-Q1b: work admitted during the pre-record window reaches the late check, not the claim.
#[test]
#[cfg_attr(not(unix), ignore = "좌석 명령이 POSIX 셸 문법")]
fn d9_bound_i2_late_busy_undoes_owned_marks_without_refusal() {
    let fx = fx("i2-late");
    let mut recorder = p1_master_recorder(&fx, "i2-late"); let s = recorder.surface.clone(); let out = recorder.output.clone();
    enqueue(&fx, &s, "i2-late");
    let release = Arc::new(Mutex::new(None));
    {
        let s = s.clone(); let release = release.clone();
        crate::agent_write::set_test_after_pre_record(move |_| *release.lock().unwrap() = Some(p1_hold_writer(&s)));
    }
    let _ = hand_off(&fx, &s);
    assert!(aborted(&fx, &s).is_empty(), "S-Q1b: late busy is a hold, not a claim refusal");
    assert!(s.queue_blocked.lock().unwrap().as_ref().is_some_and(|b| b.0.starts_with("writer_active")), "S-Q1b: late writer hold recorded");
    assert!(!s.inject_reserved(), "S-Q1b: no reservation on hold");
    assert!(s.last_queue_delivery_at.lock().unwrap().is_none(), "S-Q1b: owned timing mark undone");
    release.lock().unwrap().take().unwrap().release();
    p1_wait_completion(p1_writer_idle(&s), P1_GUARD).expect("writer operation completed");
    assert!(hand_off(&fx, &s), "S-Q1b: first idle pass delivers");
    let expected = queue_bytes("i2-late");
    assert_eq!(recorder.barrier(), expected);
}

/// S-SCOPE: a non-master keeps its prior admission even while its writer is busy.
#[test]
#[cfg_attr(not(unix), ignore = "좌석 명령이 POSIX 셸 문법")]
fn d9_bound_i2_non_master_is_not_idle_gated() {
    let fx = fx("i2-scope");
    let mut recorder = p1_recorder(&fx.daemon, &fx.dir, "i2-scope", None);
    let s = recorder.surface.clone(); *s.agent_meta.lock().unwrap() = None;
    enqueue(&fx, &s, "i2-scope");
    let holder = p1_hold_writer(&s);
    let worker = p1_queue_action(&s, move |_, _| { holder.release(); });
    assert!(hand_off(&fx, &s), "S-SCOPE: non-master is not held by idle gate");
    worker.finish();
    assert_eq!(s.writer_holds.load(Ordering::Relaxed), 0);
    assert!(s.claim_streaks.lock().unwrap().iter().all(Option::is_none));
    assert_eq!(recorder.barrier(), queue_bytes("i2-scope"));
}

#[test]
#[cfg_attr(not(unix), ignore = "좌석 명령이 POSIX 셸 문법")]
fn d9_bound_i2_writer_hold_keeps_queue_alarms_live() {
    let fx = fx("i2-alarm");
    let recorder = p1_master_recorder(&fx, "i2-alarm"); let s = recorder.surface.clone();
    // Unwind-safe: the meta test p1_completion_wc1_each_case_* forces a panic inside holder.release() below; a manual remove_var after it
    // never ran and left CYS_QUEUE_QUIET_SECS=0 in the process for every later test (F-4 (C)).
    let _queue_env = crate::governance::ReapEnvGuard::set(&[("CYS_QUEUE_STARVE_ALERT_SECS", "1"), ("CYS_QUEUE_DEPTH_ALERT", "1"), ("CYS_QUEUE_QUIET_SECS", "0")]);
    let id = enqueue(&fx, &s, "i2-alarm");
    let entry_at = s.pending_queue.lock().unwrap().front().unwrap().enqueued_at;
    let now = entry_at.max(fx.daemon.started_at).max(s.created_at) + 2.0;
    let _clock = crate::governance::P1QueueClockSnapshot::set(now, Instant::now());
    crate::governance::h_paint(&s, crate::governance::H_IDLE_SCREEN);
    let holder = p1_hold_writer(&s);
    crate::governance::h_queue_tick(&fx.daemon);
    let starved = events(&fx, "queue.starved"); let depth = events(&fx, "queue.depth_high");
    holder.release();
    assert_eq!(starved.len(), 1, "S-Q4: queue.starved count while writer held");
    assert!(starved[0]["payload"]["blocked_by"].as_str().unwrap().starts_with("writer_active"));
    assert_eq!(starved[0]["payload"]["head_entry_id"], id, "the aged item is the observed item");
    assert_eq!(depth.len(), 1, "S-Q4: queue.depth_high count while writer held");
}

#[test]
fn d9_bound_i2_writer_active_remedy_is_wait() {
    assert_eq!(crate::governance::queue_remedy("writer_active(test)", &Default::default()).0, "wait", "F6: calm wait remedy");
}

/// Head F2 ruling: another producer's concurrent hold cannot overwrite this attempt's non-writer reason.
#[test]
fn d9_bound_i2_channel_hold_does_not_steal_queue_attribution() {
    let fx = fx("i2-attribution");
    let s = seat(&fx);
    fx.daemon.roles.lock().unwrap().insert("master".into(), s.id);
    *s.agent_meta.lock().unwrap() = Some(("claude".into(), "claude".into()));
    enqueue(&fx, &s, "i2-attribution");
    crate::governance::h_paint(&s, crate::governance::H_IDLE_SCREEN);
    *s.queue_paused_until.lock().unwrap() = Some(Instant::now() + Duration::from_secs(30));
    crate::governance::h_queue_tick(&fx.daemon);
    let paused_reason = s.queue_blocked.lock().unwrap().as_ref().unwrap().0.clone();
    assert!(paused_reason.starts_with("queue_paused"), "premise: real pause reason {paused_reason}");
    *s.queue_paused_until.lock().unwrap() = None;
    let release = Arc::new(Mutex::new(None));
    {
        let s = s.clone(); let release = release.clone();
        let daemon_for_race = fx.daemon.clone();
        crate::agent_write::set_test_after_pre_record(move |_| {
            *release.lock().unwrap() = Some(hold_writer(&s));
            let sc = s.clone();
            let dc = daemon_for_race.clone();
            std::thread::spawn(move || {
                assert!(crate::governance::d9_writer_busy(&dc, &sc, crate::governance::ClaimProducer::ChannelNew, "channel-race", false));
            }).join().unwrap();
            // The helper's second frame gate refuses on this actual pause; its prior reason is still true.
            *s.queue_paused_until.lock().unwrap() = Some(Instant::now() + Duration::from_secs(30));
        });
    }
    crate::governance::h_queue_tick(&fx.daemon);
    release.lock().unwrap().take().expect("pre-record race ran").send(()).unwrap();
    assert_eq!(s.queue_blocked.lock().unwrap().as_ref().unwrap().0, paused_reason,
        "F2: concurrent channel hold must not become queue writer_active");
}

/// Race fixture: introduce a holder only after the final idle observation, preserving claim coverage.
pub(crate) fn d9_late_hold(s: &Arc<Surface>, bytes: Option<&'static [u8]>) -> std::sync::mpsc::Receiver<std::sync::mpsc::Sender<()>> {
    let (tx, rx) = std::sync::mpsc::channel();
    let s = s.clone();
    crate::agent_write::set_test_after_writer_idle(move || {
        let release = hold_writer(&s);
        if let Some(bytes) = bytes { s.write_tx.send(crate::state::WriteReq::Data(bytes.to_vec())).unwrap(); }
        tx.send(release).unwrap();
    });
    rx
}

pub(crate) fn d9_with_late_write_ahead<R>(s: &Arc<Surface>, call: impl FnOnce() -> R) -> R {
    let release = d9_late_hold(s, Some(b"T"));
    let sr = s.clone();
    let worker = std::thread::spawn(move || {
        let release = release.recv_timeout(Duration::from_secs(10)).expect("late seam ran");
        wait_until("subject queued behind holder and T", || sr.write_tx.gate().pending() >= 3);
        release.send(()).unwrap();
    });
    let result = call();
    worker.join().unwrap();
    wait_until("late race settled", || s.write_tx.gate().pending() == 0);
    result
}

#[test]
fn d9_bound_i2_slot_lifecycle_and_head_replacement() {
    use crate::governance::{ClaimProducer as P, d9_writer_busy, d9_nonwriter_hold, d9_hold_age, d9_terminal};
    let fx = fx("i2-slots");
    let recorder = p1_recorder(&fx.daemon, &fx.dir, "i2-slots", None); let s = recorder.surface.clone(); *s.agent_meta.lock().unwrap() = None;
    let first = enqueue(&fx, &s, "first");
    let release = p1_hold_writer(&s);
    for p in [P::Queue, P::ChannelNew, P::ChannelRedeliver] {
        assert!(d9_writer_busy(&fx.daemon, &s, p, &first, false));
    }
    d9_nonwriter_hold(&s, P::Queue);
    assert!(d9_hold_age(&s, P::Queue, &first).is_some(), "busy non-writer hold keeps held_since");
    release.release();
    p1_wait_completion(p1_writer_idle(&s), P1_GUARD).expect("writer operation completed");
    assert!(!d9_writer_busy(&fx.daemon, &s, P::Queue, &first, false));
    assert!(d9_hold_age(&s, P::Queue, &first).is_some(), "MI11: passing early idle clears nothing");
    d9_nonwriter_hold(&s, P::ChannelNew);
    assert!(d9_hold_age(&s, P::ChannelNew, &first).is_none(), "idle non-writer hold clears held_since");
    assert!(d9_hold_age(&s, P::ChannelRedeliver, &first).is_some(), "producer slots independent");
    d9_terminal(&s, P::ChannelRedeliver, &first, true);
    assert!(s.claim_streaks.lock().unwrap()[2].is_none(), "writing terminal clears slot");
    fx.daemon.paused.store(true, Ordering::Relaxed);
    crate::governance::h_queue_tick(&fx.daemon);
    assert!(d9_hold_age(&s, P::Queue, &first).is_none(), "idle daemon pause clears the observed writer hold");
    fx.daemon.paused.store(false, Ordering::Relaxed);
    s.pending_queue.lock().unwrap().pop_front();
    enqueue(&fx, &s, "replacement");
    *s.queue_paused_until.lock().unwrap() = Some(Instant::now() + Duration::from_secs(30));
    crate::governance::h_queue_tick(&fx.daemon);
    assert!(s.claim_streaks.lock().unwrap()[0].is_none(), "head changed behind non-writer gate clears old slot");
}

pub(crate) fn d9_status(daemon: &Arc<Daemon>, sid: u64) -> Value {
    let req = cys::Request { id: Value::from(1), method: "org.status".into(), params: serde_json::json!({}) };
    let crate::handlers::Reply::Single(v) = crate::handlers::dispatch(daemon, req, None) else { panic!("single status reply") };
    v["result"]["surfaces"].as_array().expect("status surfaces").iter().find(|x| x["surface_id"] == sid).expect("status seat").clone()
}

fn d9_refuse_queue_once(fx: &Fx, s: &Arc<Surface>) {
    let sc = s.clone();
    crate::agent_write::set_test_after_pre_record(move |_| { sc.input_gen.fetch_add(1, Ordering::AcqRel); });
    assert!(!hand_off(fx, s), "claim refuses injected foreign input");
    wait_until("refusal terminal", || s.write_tx.gate().pending() == 0);
}

#[test]
#[cfg_attr(not(unix), ignore = "좌석 명령이 POSIX 셸 문법")]
fn d9_bound_i3_queue_refusals_are_counted_bounded_and_cleared() {
    let fx = fx("i3-q2"); let mut recorder = p1_master_recorder(&fx, "i3-q2"); let s = recorder.surface.clone(); let out = recorder.output.clone();
    enqueue(&fx, &s, "i3-q2");
    for n in 1..=13 {
        p1_refuse_queue_once(&fx, &s);
        assert_eq!(events(&fx, "writer.claim_escalated").len(), usize::from(n >= 3), "S-Q2: escalation edge at N=3, attempt {n}");
    }
    let ns: Vec<_> = aborted(&fx, &s).iter().map(|e| e["payload"]["refusal_n"].as_u64().unwrap_or(0)).collect();
    assert_eq!(ns, vec![1,2,3,6,12], "S-Q2: bounded refusal event sequence");
    let row = d9_status(&fx.daemon, s.id);
    assert_eq!(row["writer_claim"]["refusals_total"]["queue"], 13, "S-Q2: every refusal counted");
    assert_eq!(row["writer_claim"]["slots"]["queue"]["refusals"], 13);
    assert_eq!(row["writer_claim"]["slots"]["queue"]["last_foreign"], serde_json::json!(["input_gen"]));
    assert!(recorder.barrier().is_empty());
    let observed = Instant::now()-Duration::from_secs(2);
    s.claim_streaks.lock().unwrap()[0].as_mut().unwrap().updated_at=observed;
    for _ in 0..8 {
        let _=d9_status(&fx.daemon,s.id);
        assert_eq!(s.claim_streaks.lock().unwrap()[0].as_ref().unwrap().updated_at,observed,"status reads preserve the actual age anchor");
    }
    assert!(d9_status(&fx.daemon,s.id)["writer_claim"]["slots"]["queue"]["age_secs"].as_u64().unwrap_or(0)>=1,"status reads do not refresh the streak age");
    assert!(hand_off(&fx, &s), "S-Q2: delivered after fault removed");
    p1_wait_completion(p1_writer_idle(&s), P1_GUARD).expect("delivery operation completed"); assert_eq!(queue_len(&s), 0);
    let row = d9_status(&fx.daemon, s.id);
    assert!(row["writer_claim"]["slots"]["queue"].is_null(), "S-Q2: delivery clears slot");
    assert_eq!(row["writer_claim"]["refusals_total"]["queue"],13);
}

#[test]
#[cfg_attr(not(unix), ignore = "좌석 명령이 POSIX 셸 문법")]
fn d9_bound_i3_busy_holds_do_not_reset_refusals() {
    let fx=fx("i3-q2b");let mut recorder = p1_master_recorder(&fx, "i3-q2b"); let s = recorder.surface.clone(); let out = recorder.output.clone();enqueue(&fx,&s,"i3-q2b");
    for _ in 0..3 {
        p1_refuse_queue_once(&fx,&s);
        let release=p1_hold_writer(&s);assert!(!hand_off(&fx,&s));release.release();
        p1_wait_completion(p1_writer_idle(&s), P1_GUARD).expect("writer operation completed");
    }
    assert_eq!(events(&fx,"writer.claim_escalated").len(),1,"S-Q2b: hold preserves the refusal streak");
    assert_eq!(d9_status(&fx.daemon,s.id)["writer_claim"]["slots"]["queue"]["refusals"],3);
}

#[test]
#[cfg_attr(not(unix), ignore = "좌석 명령이 POSIX 셸 문법")]
fn d9_bound_i3_busy_queue_escalates_once_then_delivers() {
    let fx=fx("i3-q3");let mut recorder = p1_master_recorder(&fx, "i3-q3"); let s = recorder.surface.clone(); let out = recorder.output.clone();enqueue(&fx,&s,"i3-q3");
    let _knob=crate::governance::HKnobGuard::set(&[("CYS_WRITER_HOLD_ESCALATE_SECS","1")]);
    let release=p1_hold_writer(&s);
    assert!(!hand_off(&fx,&s));
    s.claim_streaks.lock().unwrap()[0].as_mut().expect("held queue slot").held_since = Some(Instant::now()-Duration::from_secs(2));
    assert!(!hand_off(&fx,&s));
    let ev=events(&fx,"writer.claim_escalated");
    assert_eq!(ev.len(),1,"S-Q3: continuous busy escalates without resetting its clock");
    assert_eq!(ev[0]["payload"]["cause"],"not_idle");assert_eq!(ev[0]["payload"]["refusals"],0);
    for _ in 0..3 { assert!(!hand_off(&fx,&s)); }
    assert_eq!(events(&fx,"writer.claim_escalated").len(),1,"S-Q3: one event per streak");
    release.release();p1_wait_completion(p1_writer_idle(&s), P1_GUARD).expect("writer operation completed");
    assert!(hand_off(&fx,&s),"S-Q3: first idle attempt delivers");
}

#[test]
#[cfg_attr(not(unix), ignore = "좌석 명령이 POSIX 셸 문법")]
fn d9_bound_i3_idle_nonwriter_hold_breaks_busy_continuity() {
    let fx=fx("i3-f3a");let recorder=p1_master_recorder(&fx,"i3-f3a");let s=recorder.surface.clone();enqueue(&fx,&s,"i3-f3a");
    let _knob=crate::governance::HKnobGuard::set(&[("CYS_WRITER_HOLD_ESCALATE_SECS","1")]);
    let holder=p1_hold_writer(&s);assert!(!hand_off(&fx,&s));holder.release();
    let old=Instant::now()-Duration::from_secs(2);
    s.claim_streaks.lock().unwrap()[0].as_mut().unwrap().held_since=Some(old);
    *s.queue_paused_until.lock().unwrap()=Some(Instant::now()+Duration::from_secs(20));
    crate::governance::h_queue_tick(&fx.daemon);
    { let slots=s.claim_streaks.lock().unwrap();let slot=slots[0].as_ref().unwrap();
      assert!(slot.held_since.is_none() && slot.held_epoch.is_none(),"idle nonwriter hold synchronously clears both hold anchors"); }
    *s.queue_paused_until.lock().unwrap()=None;
    let holder=p1_hold_writer(&s);assert!(!hand_off(&fx,&s));
    assert!(s.claim_streaks.lock().unwrap()[0].as_ref().unwrap().held_since.unwrap()>old,"new hold owns a fresh anchor");
    assert!(events(&fx,"writer.claim_escalated").is_empty(),"S-F3a: idle non-writer interval must not count as continuously busy");
    holder.release();
}

#[test]
#[cfg_attr(not(unix), ignore = "좌석 명령이 POSIX 셸 문법")]
fn d9_bound_i3_controller_owned_claim_is_counted_once() {
    let fx=fx("i3-controller"); let recorder=p1_master_recorder(&fx,"i3-controller"); let s=recorder.surface.clone(); enqueue(&fx,&s,"i3-controller");
    // Synthetic API ownership control: no production defer_to_controller caller.
    let held=p1_late_hold(&s,None);
    let action=p1_queue_action(&s,move |sc,_| {
        let guard=sc.inject_reservation.lock().unwrap().as_ref().expect("actual reservation").guard.clone();
        let(wake,_rx)=crate::state::controller_wake_channel(); guard.defer_to_controller(wake).unwrap();
        sc.input_gen.fetch_add(1,Ordering::AcqRel);
        held.lock().unwrap().take().expect("late seam holder").release();
    });
    let _=hand_off(&fx,&s); action.finish();
    p1_wait_completion(p1_writer_idle(&s),P1_GUARD).expect("controller terminal");
    assert_eq!(d9_status(&fx.daemon,s.id)["writer_claim"]["refusals_total"]["queue"],1,"controller claim refusal counted exactly once");
    assert_eq!(hold_kind(&s),Some(QueueHoldKind::Refused),"controller disposition/backoff unchanged");
}

#[test]
fn d9_bound_i3_status_key_present_and_null_outside_scope() {
    let fx=fx("i3-status-scope");let s=seat(&fx);let row=d9_status(&fx.daemon,s.id);
    assert!(row.as_object().unwrap().contains_key("writer_claim"),"writer_claim is an explicit additive status key");
    assert!(row["writer_claim"].is_null(),"outside D9 scope stays null");
}

#[test]
#[cfg_attr(not(unix), ignore = "좌석 명령이 POSIX 셸 문법")]
fn d9_bound_i3_late_refusal_does_not_restore_a_replaced_head_slot() {
    let fx=fx("i3-head-race");let recorder=p1_master_recorder(&fx,"i3-head-race");let s=recorder.surface.clone();enqueue(&fx,&s,"old head");
    let next=fx.daemon.next_queue_entry("[RESUME] new head".into(),None,"test");
    let held=p1_late_hold(&s,Some(b"T"));
    let action=p1_queue_action(&s,move |sc,_| {
        sc.pending_queue.lock().unwrap().push_front(next);
        held.lock().unwrap().take().expect("late holder").release();
    });
    let _=hand_off(&fx,&s);action.finish();
    p1_wait_completion(p1_writer_idle(&s),P1_GUARD).expect("terminal");
    let row=d9_status(&fx.daemon,s.id);
    assert!(row["writer_claim"]["slots"]["queue"].is_null(),"late refusal must not recreate a slot for the former head");
    assert_eq!(row["writer_claim"]["refusals_total"]["queue"],1,"historical refusal still counted");
}

// WC-1: P1-only admission/completion channel. Historical wait_until callers are unchanged.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum P1WaitError {
    Admission { event: &'static str, reason: String },
    CompletionTimeout { event: &'static str },
    CompletionDisconnected { event: &'static str },
}

/// Non-forgeable outside this module: created only after the producer's synchronous admission.
pub(crate) struct P1Admitted<T> {
    event: &'static str,
    receiver: std::sync::mpsc::Receiver<T>,
}

/// `start` must return the actual producer admission, installing `tx` at its named completion.
/// No predicate or background state poll is accepted; the channel exists before admission runs.
pub(crate) fn p1_start_completion<T>(
    event: &'static str,
    start: impl FnOnce(std::sync::mpsc::Sender<T>) -> Result<(), String>,
) -> Result<P1Admitted<T>, P1WaitError> {
    let (tx, receiver) = std::sync::mpsc::channel();
    start(tx).map_err(|reason| P1WaitError::Admission { event, reason })?;
    Ok(P1Admitted { event, receiver })
}

/// Only an admitted operation can reach this named completion guard.
pub(crate) fn p1_wait_completion<T: Send + 'static>(
    admitted: Result<P1Admitted<T>, P1WaitError>,
    guard: Duration,
) -> Result<T, P1WaitError> {
    let P1Admitted { event, mut receiver } = admitted?;
    let mut guard = guard;
    let fault = P1_COMPLETION_CONTROL.with(|slot| {
        let mut slot = slot.borrow_mut();
        slot.as_mut().filter(|c| !c.consumed && c.event == event && !guard.is_zero()).map(|c| {
            c.consumed = true; c.fault
        })
    });
    // Fault controls are installed only by the caller-control test. Admission above is real.
    let mut retained_sender = None;
    match fault {
        Some(P1CompletionFault::Delayed) => {
            let (tx, delayed) = std::sync::mpsc::channel();
            let relay = std::thread::spawn(move || {
                let actual = receiver.recv();
                // Inject delayed delivery of the ACTUAL event, not a successful replacement value.
                std::thread::sleep(Duration::from_millis(100));
                if let Ok(value) = actual { let _ = tx.send(value); }
            });
            P1_COMPLETION_CONTROL.with(|slot| slot.borrow_mut().as_mut().unwrap().relays.push(relay));
            receiver = delayed;
        }
        Some(P1CompletionFault::Missing) => {
            let (tx, missing) = std::sync::mpsc::channel();
            retained_sender = Some(tx); // no disconnect masquerading as the named guard
            receiver = missing; // deliberately withhold the real producer's completion
            guard = Duration::ZERO;
        }
        None => {}
    }
    let _retain_through_wait = retained_sender;
    match receiver.recv_timeout(guard) { // wallclock: guard receipt of the admitted producer's named completion event
        Ok(value) => Ok(value),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Err(P1WaitError::CompletionTimeout { event }),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Err(P1WaitError::CompletionDisconnected { event }),
    }
}

#[test]
fn p1_completion_refusal_is_not_a_timeout_or_success() {
    let admitted = p1_start_completion("refused producer", |tx| {
        tx.send(7).unwrap(); // even a premature completion cannot replace admission
        Err("enqueue refused".into())
    });
    assert_eq!(p1_wait_completion(admitted, Duration::ZERO), Err(P1WaitError::Admission {
        event: "refused producer", reason: "enqueue refused".into(),
    }));
}

#[test]
fn p1_completion_synchronous_receipt_cannot_be_lost() {
    let admitted = p1_start_completion("synchronous terminal", |tx| {
        tx.send(9).unwrap();
        Ok(())
    });
    assert_eq!(p1_wait_completion(admitted, Duration::ZERO), Ok(9));
}

#[test]
fn p1_completion_delayed_success_uses_explicit_release() {
    let (release, held) = std::sync::mpsc::channel();
    let (entered, observed) = std::sync::mpsc::channel();
    let mut producer = None;
    let admitted = p1_start_completion("released terminal", |tx| {
        producer = Some(std::thread::spawn(move || {
            entered.send(()).unwrap();
            held.recv().expect("explicit holder release");
            tx.send(11).unwrap();
        }));
        Ok(()) // spawn returned a live owned thread; receiver above is its completion
    });
    observed.recv().unwrap();
    let waiter = std::thread::spawn(move || p1_wait_completion(admitted, Duration::from_secs(60)));
    release.send(()).unwrap();
    assert_eq!(waiter.join().unwrap(), Ok(11));
    producer.unwrap().join().unwrap();
}

#[test]
fn p1_completion_never_arrives_is_a_named_guard_failure() {
    let mut held = None;
    let admitted = p1_start_completion("missing terminal", |tx: std::sync::mpsc::Sender<()>| {
        held = Some(tx);
        Ok(())
    });
    assert_eq!(p1_wait_completion(admitted, Duration::ZERO), Err(P1WaitError::CompletionTimeout {
        event: "missing terminal",
    }));
    drop(held); // no timer can release the holder before the guard
}

#[test]
fn p1_completion_disconnect_is_not_a_timeout() {
    let admitted = p1_start_completion("lost producer", |_tx: std::sync::mpsc::Sender<()>| Ok(()));
    assert_eq!(p1_wait_completion(admitted, Duration::ZERO), Err(P1WaitError::CompletionDisconnected {
        event: "lost producer",
    }));
}

#[test]
fn p1_completion_one_producers_receipt_cannot_complete_another() {
    let first = p1_start_completion("first", |tx| { tx.send(1).unwrap(); Ok(()) });
    let mut held = None;
    let second = p1_start_completion("second", |tx: std::sync::mpsc::Sender<i32>| { held = Some(tx); Ok(()) });
    assert_eq!(p1_wait_completion(second, Duration::ZERO), Err(P1WaitError::CompletionTimeout { event: "second" }));
    assert_eq!(p1_wait_completion(first, Duration::ZERO), Ok(1));
    drop(held);
}


/// Register at a real per-instance idle event; callers must first check their producer admission.
pub(crate) fn p1_writer_idle(s: &Arc<Surface>) -> Result<P1Admitted<()>, P1WaitError> {
    p1_start_completion("writer operation leases released", |tx| s.write_tx.gate().p1_subscribe_idle(tx))
}

/// Ordinary supervisor only. Returning `()` from the producer cannot fabricate admission.
/// The adapter restores the calling thread's previous test_sync_settle mode on every exit.
pub(crate) fn p1_supervisor_call<R>(
    s: &Arc<Surface>, call: impl FnOnce() -> R,
) -> (R, Result<P1Admitted<crate::agent_write::Terminal>, P1WaitError>) {
    let _async_scope = crate::agent_write::p1_sync_settle(false);
    let seen = std::rc::Rc::new(std::cell::Cell::new(0usize));
    let failure = std::rc::Rc::new(std::cell::RefCell::new(None));
    let mut result = None;
    let admitted = p1_start_completion("ordinary supervisor settlement", |tx| {
        let seen_at_admit = seen.clone();
        let failure_at_admit = failure.clone();
        let _observer = crate::agent_write::p1_observe_admission(s.id, "supervisor", move |request| {
            seen_at_admit.set(seen_at_admit.get() + 1);
            *failure_at_admit.borrow_mut() = request.p1_subscribe_supervisor_completion(tx).err();
        });
        result = Some(call());
        if seen.get() != 1 { return Err(format!("expected one real supervisor enqueue receipt, observed {}", seen.get())); }
        if let Some(reason) = failure.borrow_mut().take() { return Err(reason); }
        Ok(())
    });
    (result.expect("producer returned"), admitted)
}

#[test]
fn p1_completion_supervisor_early_and_late_observers_see_post_settlement() {
    use crate::agent_write::{Request, AgentJob, Plan, WriterIo};
    let request = Request::new("p1-supervisor".into(), 7, "supervisor", "d".into());
    let early = p1_start_completion("early supervisor terminal", |tx| request.p1_subscribe_supervisor_completion(tx));
    let completed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let completed_at_settle = completed.clone();
    let job = AgentJob { request: request.clone(), plan: Plan::raw("x"),
        proof: Scripted::new(0, ProofLoss::AgentDead, None), journal: None, attempt: None, resume: None,
        settle: Some(Box::new(move |_| { completed_at_settle.store(true, Ordering::Release); })),
        guard: None, note_body: false, body_owner: None };
    let mut anchor = None; let mut first = || {}; let mut body = |_: &str| {};
    let mut io = WriterIo { sleep: &|_| {}, last_program_write: &mut anchor, on_first_byte: &mut first, on_body_written: &mut body };
    let mut writer = Vec::new();
    let executed = crate::agent_write::execute(&mut writer, &job, &mut io);
    assert!(matches!(early.as_ref().unwrap().receiver.try_recv(), Err(std::sync::mpsc::TryRecvError::Empty)), "execute is not settlement");
    let terminal = crate::agent_write::finish(job, &executed);
    assert!(completed.load(Ordering::Acquire));
    assert_eq!(p1_wait_completion(early, Duration::ZERO), Ok(terminal.clone()));
    let late = p1_start_completion("late supervisor terminal", |tx| request.p1_subscribe_supervisor_completion(tx));
    assert_eq!(p1_wait_completion(late, Duration::ZERO), Ok(terminal));
    let other = Request::new("not-supervisor".into(), 7, "queue", "d".into());
    assert!(matches!(p1_start_completion("reject queue", |tx| other.p1_subscribe_supervisor_completion(tx)), Err(P1WaitError::Admission { .. })));
    let managed = Request::new_managed("managed".into(), 7, "supervisor", "d".into(), &Plan::raw("x"));
    assert!(matches!(p1_start_completion("reject managed", |tx| managed.p1_subscribe_supervisor_completion(tx)), Err(P1WaitError::Admission { .. })));
}

#[test]
fn p1_completion_admission_observer_is_scoped_and_one_shot() {
    crate::agent_write::set_test_sync_settle(false);
    let seen = std::rc::Rc::new(std::cell::Cell::new(0));
    let seen_inner = seen.clone();
    let observer = crate::agent_write::p1_observe_admission(7, "supervisor", move |_| seen_inner.set(seen_inner.get()+1));
    let wrong = crate::agent_write::Request::new("wrong".into(), 8, "supervisor", "d".into());
    crate::agent_write::test_wait_settled(&wrong);
    assert_eq!(seen.get(), 0);
    let right = crate::agent_write::Request::new("right".into(), 7, "supervisor", "d".into());
    crate::agent_write::test_wait_settled(&right);
    crate::agent_write::test_wait_settled(&right);
    assert_eq!(seen.get(), 1);
    drop(observer);
    crate::agent_write::test_wait_settled(&right);
    assert_eq!(seen.get(), 1);
    crate::agent_write::set_test_sync_settle(true);
}

#[test]
fn p1_completion_idle_ack_is_after_release_and_late_subscription_is_safe() {
    let (sender, receiver) = crate::writer_gate::WriterSender::channel(4);
    sender.send(crate::state::WriteReq::Data(vec![1])).unwrap(); // actual admission
    let queued = receiver.recv().unwrap();
    let admitted = p1_start_completion("owned operation release", |tx| sender.gate().p1_subscribe_idle(tx));
    assert!(matches!(admitted.as_ref().unwrap().receiver.try_recv(), Err(std::sync::mpsc::TryRecvError::Empty)));
    let (release, held) = std::sync::mpsc::channel();
    let holder = std::thread::spawn(move || { held.recv().unwrap(); drop(queued); });
    release.send(()).unwrap();
    assert_eq!(p1_wait_completion(admitted, Duration::from_secs(60)), Ok(()));
    holder.join().unwrap();
    let late = p1_start_completion("already idle", |tx| sender.gate().p1_subscribe_idle(tx));
    assert_eq!(p1_wait_completion(late, Duration::ZERO), Ok(()));
    sender.send(crate::state::WriteReq::Data(vec![2])).unwrap();
    let held = receiver.recv().unwrap();
    let never = p1_start_completion("unreleased operation", |tx| sender.gate().p1_subscribe_idle(tx));
    assert_eq!(p1_wait_completion(never, Duration::ZERO), Err(P1WaitError::CompletionTimeout { event: "unreleased operation" }));
    drop(held);
}


struct P1QueueReservationNotification {
    surface_id: u64,
    notify: Box<dyn FnOnce(&Arc<Surface>, &crate::agent_write::Request)>,
}
thread_local! {
    static P1_QUEUE_RESERVATION: std::cell::RefCell<Option<P1QueueReservationNotification>> = const { std::cell::RefCell::new(None) };
}
pub(crate) struct P1QueueReservationObserver(Option<P1QueueReservationNotification>);
impl Drop for P1QueueReservationObserver {
    fn drop(&mut self) { P1_QUEUE_RESERVATION.with(|slot| *slot.borrow_mut() = self.0.take()); }
}
/// Install before the tested producer call; the producer invokes this only after real admission.
pub(crate) fn p1_on_queue_reserved(
    surface_id: u64, notify: impl FnOnce(&Arc<Surface>, &crate::agent_write::Request) + 'static,
) -> P1QueueReservationObserver {
    P1QueueReservationObserver(P1_QUEUE_RESERVATION.with(|slot| slot.replace(Some(P1QueueReservationNotification {
        surface_id, notify: Box::new(notify),
    }))))
}
/// Notification only: called once after successful enqueue and installation of its reservation.
pub(crate) fn p1_queue_reservation_admitted(s: &Arc<Surface>, request: &crate::agent_write::Request) {
    P1_QUEUE_RESERVATION.with(|slot| {
        let matching = slot.borrow().as_ref().is_some_and(|observer| observer.surface_id == s.id);
        if matching {
            let observer = slot.borrow_mut().take().unwrap();
            assert_eq!(request.surface_id, s.id, "P1 reservation receipt belongs to this surface");
            (observer.notify)(s, request);
        }
    });
}

#[test]
fn p1_completion_reservation_registration_restores_outer_scope() {
    let _outer = p1_on_queue_reserved(7, |_, _| {});
    assert_eq!(P1_QUEUE_RESERVATION.with(|slot| slot.borrow().as_ref().unwrap().surface_id), 7);
    {
        let _inner = p1_on_queue_reserved(8, |_, _| {});
        assert_eq!(P1_QUEUE_RESERVATION.with(|slot| slot.borrow().as_ref().unwrap().surface_id), 8);
    }
    assert_eq!(P1_QUEUE_RESERVATION.with(|slot| slot.borrow().as_ref().unwrap().surface_id), 7);
    assert!(std::thread::spawn(|| P1_QUEUE_RESERVATION.with(|slot| slot.borrow().is_none())).join().unwrap());
}

const P1_GUARD: Duration = Duration::from_secs(60);
const P1_BARRIER_MAGIC: &[u8] = b"\0P1WC:";

/// One private receiver. ACKs carry complete prefixes; bytes travel only through the PTY writer.
pub(crate) struct P1Recorder {
    pub(crate) surface: Arc<Surface>,
    pub(crate) output: std::path::PathBuf,
    ack: std::net::TcpStream,
    sequence: u64,
    barrier_admitted: Option<std::sync::mpsc::Sender<()>>,
}

fn p1_read_frame(stream: &mut std::net::TcpStream) -> Result<(u8, u64, Vec<u8>), String> {
    use std::io::Read;
    let mut header = [0u8; 17];
    stream.read_exact(&mut header).map_err(|e| format!("receiver ACK header: {e}"))?;
    let sequence = u64::from_be_bytes(header[1..9].try_into().unwrap());
    let n = u64::from_be_bytes(header[9..17].try_into().unwrap());
    if n > 16 * 1024 * 1024 { return Err("receiver ACK exceeds fixture limit".into()); }
    let mut prefix = vec![0; n as usize];
    stream.read_exact(&mut prefix).map_err(|e| format!("receiver ACK payload: {e}"))?;
    Ok((header[0], sequence, prefix))
}

impl P1Recorder {
    /// Opt-in matching the old boot/queue recorder. Channels leave their real proof unchanged.
    pub(crate) fn install_always_admit(&self) {
        *self.surface.receiver_proof.lock().unwrap() = Some(Scripted::new(0, ProofLoss::AgentDead, None));
    }

    fn acknowledge(&mut self, barrier: Option<u64>, minimum: usize) -> Vec<u8> {
        let mut reader = self.ack.try_clone().expect("private ACK stream");
        let mut worker = None;
        let event = if barrier.is_some() { "receiver FIFO barrier ACK" } else { "receiver body ACK at held producer" };
        let admitted = p1_start_completion(event, |tx| {
            worker = Some(std::thread::spawn(move || {
                let result = (|| {
                    loop {
                        let (kind, seq, prefix) = p1_read_frame(&mut reader)?;
                        match barrier {
                            Some(want) if kind == b'B' => {
                                if seq != want { return Err(format!("receiver barrier mismatch: {seq} != {want}")); }
                                return Ok(prefix);
                            }
                            None if kind == b'D' && prefix.len() >= minimum => return Ok(prefix),
                            _ => {}
                        }
                    }
                })();
                let _ = tx.send(result);
            }));
            if let Some(sequence) = barrier {
                let mut marker = P1_BARRIER_MAGIC.to_vec();
                marker.extend(sequence.to_be_bytes()); marker.push(0xff);
                // This is the actual admission, before waiting for this exact marker's ACK.
                self.surface.write_tx.send(crate::state::WriteReq::P1ReceiverBarrier(marker)).map_err(|_| "barrier writer enqueue refused".to_string())?;
                if let Some(notify) = self.barrier_admitted.take() { let _ = notify.send(()); }
                Ok(())
            } else {
                Ok(()) // subscriber thread spawned on the receiver's already-established ACK stream
            }
        });
        let result = p1_wait_completion(admitted, P1_GUARD);
        if result.is_err() { let _ = self.ack.shutdown(std::net::Shutdown::Both); }
        worker.unwrap().join().expect("receiver observer joined");
        result.expect("named receiver completion guard").expect("receiver ACK valid")
    }

    pub(crate) fn barrier(&mut self) -> Vec<u8> {
        self.sequence += 1;
        let prefix = self.acknowledge(Some(self.sequence), 0);
        p1_wait_completion(p1_writer_idle(&self.surface), P1_GUARD).expect("barrier operation lease released");
        prefix
    }

    /// Only meaningful while the tested producer is held at an acknowledged pre-CR seam.
    pub(crate) fn received_prefix(&mut self, minimum: usize) -> Vec<u8> {
        self.acknowledge(None, minimum)
    }
}

impl Drop for P1Recorder {
    fn drop(&mut self) {
        let _ = self.ack.shutdown(std::net::Shutdown::Both);
        let _ = self.surface.child.lock().unwrap().kill();
    }
}

pub(crate) fn p1_recorder(
    daemon: &Arc<Daemon>, dir: &std::path::Path, tag: &str, role: Option<&str>,
) -> P1Recorder {
    use std::io::Write;
    assert!(tag.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'));
    let output = dir.join(format!("p1-rx-{tag}.bin"));
    let script = dir.join(format!("p1-rx-{tag}.py"));
    // An ephemeral loopback ACK endpoint avoids Unix path limits and fixed-port collisions.
    let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
    let address = listener.local_addr().unwrap();
    std::fs::write(&script, r#"import os,socket,struct,sys,tty
out=open(sys.argv[1],'wb',buffering=0)
tty.setraw(0)
ack=socket.create_connection(('127.0.0.1',int(sys.argv[2])))
prefix=bytearray(); pending=bytearray(); magic=b'\x00P1WC:'
def frame(kind,seq=0):
    ack.sendall(kind+struct.pack('!QQ',seq,len(prefix))+prefix)
def append(data):
    if data:
        out.write(data); prefix.extend(data); frame(b'D')
frame(b'R')
while True:
    data=os.read(0,4096)
    if not data: break
    pending.extend(data)
    while pending:
        at=pending.find(magic)
        if at>=0:
            append(pending[:at]); del pending[:at]
            if len(pending)<len(magic)+9: break
            seq=struct.unpack('!Q',pending[len(magic):len(magic)+8])[0]
            if pending[len(magic)+8]!=255: raise RuntimeError('invalid barrier frame')
            del pending[:len(magic)+9]; frame(b'B',seq)
        else:
            keep=0
            for n in range(1,min(len(pending),len(magic)-1)+1):
                if pending[-n:]==magic[:n]: keep=n
            take=len(pending)-keep
            append(pending[:take]); del pending[:take]
            break
"#).unwrap();
    let command = format!("exec python3 -B -u '{}' '{}' {}", script.display(), output.display(), address.port());
    let mut surface = None;
    let mut worker = None;
    let active = Arc::new(Mutex::new(None::<std::net::TcpStream>));
    let active_reader = active.clone();
    let ready = p1_start_completion("recorder raw-mode READY ACK", |tx| {
        let s = daemon.create_surface(None, Some(command), None, None, 24, 200).map_err(|e| format!("recorder spawn refused: {e}"))?;
        daemon.surfaces.lock().unwrap().insert(s.id, s.clone());
        surface = Some(s);
        worker = Some(std::thread::spawn(move || {
            let result: Result<std::net::TcpStream, String> = (|| {
                let (mut stream, _) = listener.accept().map_err(|e| format!("ready accept: {e}"))?;
                *active_reader.lock().unwrap() = Some(stream.try_clone().map_err(|e| e.to_string())?);
                let (kind, seq, prefix) = p1_read_frame(&mut stream)?;
                if kind != b'R' || seq != 0 || !prefix.is_empty() { return Err("invalid recorder READY".into()); }
                Ok(stream)
            })();
            let _ = tx.send(result);
        }));
        Ok(()) // actual create_surface succeeded; READY confirms tty/output initialization
    });
    let result = p1_wait_completion(ready, P1_GUARD);
    if result.is_err() {
        if let Some(stream) = active.lock().unwrap().as_ref() { let _ = stream.shutdown(std::net::Shutdown::Both); }
        if let Ok(mut unblock) = std::net::TcpStream::connect(address) { let _ = unblock.write_all(&[0; 17]); }
    }
    if let Some(worker) = worker { worker.join().expect("ready observer joined"); }
    let ack = result.expect("recorder readiness guard").expect("recorder initialized");
    let s = surface.expect("recorder admission");
    if let Some(role) = role { daemon.roles.lock().unwrap().insert(role.into(), s.id); }
    *s.agent_meta.lock().unwrap() = Some(("claude".into(), "claude".into()));
    P1Recorder { surface: s, output, ack, sequence: 0, barrier_admitted: None }
}

struct P1HeldProof {
    entered: std::sync::mpsc::Sender<()>,
    release: Mutex<std::sync::mpsc::Receiver<()>>,
}
impl Proof for P1HeldProof {
    fn admit(&self) -> Result<(), ProofLoss> {
        let _ = self.entered.send(());
        let _ = self.release.lock().unwrap().recv(); // explicit release/disconnect, no expiry
        Err(ProofLoss::Unknown("p1 explicit writer holder".into()))
    }
    fn recheck(&self, _: Phase) -> Result<(), ProofLoss> { Ok(()) }
}

pub(crate) struct P1WriterHold {
    surface: Arc<Surface>,
    release: Option<std::sync::mpsc::Sender<()>>,
}
impl P1WriterHold {
    pub(crate) fn release_signal(mut self) -> Result<P1Admitted<()>, P1WaitError> {
        assert_eq!(p1_wait_completion(p1_writer_idle(&self.surface), Duration::ZERO),
            Err(P1WaitError::CompletionTimeout { event: "writer operation leases released" }),
            "retained actual holder must fail at its named completion guard");
        self.release.take().unwrap().send(()).expect("held writer still owns release receiver");
        p1_writer_idle(&self.surface)
    }
    pub(crate) fn release(self) {
        p1_wait_completion(self.release_signal(), P1_GUARD).expect("explicit holder release completed");
    }
}
impl Drop for P1WriterHold {
    fn drop(&mut self) { if let Some(release) = self.release.take() { let _ = release.send(()); } }
}

pub(crate) fn p1_hold_writer(s: &Arc<Surface>) -> P1WriterHold {
    let (release, held) = std::sync::mpsc::channel();
    let entered = p1_start_completion("admitted holder proof entered", |tx| {
        let job = crate::agent_write::AgentJob {
            attempt: None,
            request: crate::agent_write::Request::new(crate::agent_write::new_request_id(1), s.id, "test", "p1-holder".into()),
            plan: crate::agent_write::Plan::control(Vec::new()),
            proof: Arc::new(P1HeldProof { entered: tx, release: Mutex::new(held) }),
            journal: None, resume: None, settle: None, guard: None, note_body: false, body_owner: None,
        };
        s.write_tx.send(crate::state::WriteReq::Agent(Box::new(job))).map_err(|_| "holder enqueue refused".into())
    });
    let holder = P1WriterHold { surface: s.clone(), release: Some(release) };
    p1_wait_completion(entered, P1_GUARD).expect("holder admission and proof-entry guard");
    holder
}

pub(crate) fn p1_write_finished(s: &Arc<Surface>, bytes: &[u8]) {
    let admitted = p1_start_completion("foreign writer operation released", |tx| {
        s.write_tx.send(crate::state::WriteReq::Data(bytes.to_vec())).map_err(|_| "foreign enqueue refused".to_string())?;
        s.write_tx.gate().p1_subscribe_idle(tx)
    });
    p1_wait_completion(admitted, P1_GUARD).expect("admitted foreign write finished");
}

/// The held producer is installed in the exact old late-idle seam, without an automatic release.
pub(crate) fn p1_late_hold(s: &Arc<Surface>, bytes: Option<&'static [u8]>) -> Arc<Mutex<Option<P1WriterHold>>> {
    let slot = Arc::new(Mutex::new(None));
    let ready = slot.clone(); let s = s.clone();
    crate::agent_write::set_test_after_writer_idle(move || {
        let holder = p1_hold_writer(&s);
        if let Some(bytes) = bytes { s.write_tx.send(crate::state::WriteReq::Data(bytes.to_vec())).expect("foreign admission"); }
        *ready.lock().unwrap() = Some(holder);
    });
    slot
}


#[test]
#[cfg_attr(not(unix), ignore = "좌석 명령이 POSIX 셸 문법")]
fn p1_completion_recorder_barrier_covers_delayed_extra_bytes() {
    let fx = fx("p1-recorder-control");
    let mut recorder = p1_recorder(&fx.daemon, &fx.dir, "control", Some("master"));
    assert!(recorder.surface.receiver_proof.lock().unwrap().is_none(), "channel default proof unchanged");
    recorder.install_always_admit();
    p1_write_finished(&recorder.surface, b"first\r");
    let holder = p1_hold_writer(&recorder.surface);
    recorder.surface.write_tx.send(crate::state::WriteReq::Data(b"late-extra\r".to_vec())).expect("late bytes admitted");
    let mut worker = None;
    let marker_admitted = p1_start_completion("FIFO marker queued after delayed extra bytes", |tx| {
        recorder.barrier_admitted = Some(tx);
        worker = Some(std::thread::spawn(move || { let prefix = recorder.barrier(); (prefix, recorder) }));
        Ok(())
    });
    p1_wait_completion(marker_admitted, P1_GUARD).expect("actual barrier enqueue receipt");
    holder.release();
    let (prefix, mut recorder) = worker.unwrap().join().unwrap();
    assert_eq!(prefix, b"first\rlate-extra\r", "receiver ACK includes bytes delayed behind an explicit holder");
    assert_eq!(std::fs::read(&recorder.output).unwrap(), prefix, "protocol marker is not product output");
    assert_eq!(recorder.barrier(), prefix, "a second barrier introduces no extra product byte");
    p1_write_finished(&recorder.surface, b"\r");
    let duplicate = recorder.barrier();
    let mut expected = prefix.clone(); expected.push(b'\r');
    assert_eq!(duplicate, expected, "an extra duplicate CR remains visible before the receiver barrier");
    assert_ne!(duplicate, prefix, "the observer cannot erase a duplicate CR");
}

#[test]
#[cfg_attr(not(unix), ignore = "좌석 명령이 POSIX 셸 문법")]
fn p1_completion_explicit_holder_does_not_release_before_owner() {
    let fx = fx("p1-holder-control");
    let mut recorder = p1_recorder(&fx.daemon, &fx.dir, "hold", None);
    let holder = p1_hold_writer(&recorder.surface);
    let idle = p1_writer_idle(&recorder.surface);
    assert_eq!(p1_wait_completion(idle, Duration::ZERO), Err(P1WaitError::CompletionTimeout {
        event: "writer operation leases released",
    }), "never-completes control is the named guard while owner retains holder");
    holder.release();
    assert_eq!(recorder.barrier(), b"", "holder itself writes no bytes");
}

pub(crate) fn p1_with_late_write_ahead<R>(s: &Arc<Surface>, call: impl FnOnce() -> R) -> R {
    let _async_scope = crate::agent_write::p1_sync_settle(false);
    let held = p1_late_hold(s, Some(b"T"));
    let result = call();
    let holder = held.lock().unwrap().take().expect("actual late-idle seam was reached");
    holder.release();
    result
}

pub(crate) fn p1_s0_foreign_write_after_pre_record(s: &Arc<Surface>, bytes: &'static [u8]) {
    let s = s.clone();
    crate::agent_write::set_test_after_pre_record(move |_| p1_write_finished(&s, bytes));
}

/// Worker starts ONLY in the real post-enqueue reservation callback; no reservation poll.
struct P1QueueAction<T> {
    _observer: P1QueueReservationObserver,
    completion: std::rc::Rc<std::cell::RefCell<Option<Result<P1Admitted<T>, P1WaitError>>>>,
    worker: std::rc::Rc<std::cell::RefCell<Option<std::thread::JoinHandle<()>>>>,
}
impl<T: Send + 'static> P1QueueAction<T> {
    fn finish(self) -> T {
        let admitted = self.completion.borrow_mut().take().unwrap_or_else(|| Err(P1WaitError::Admission {
            event: "queue reservation action completed", reason: "no successful enqueue/reservation callback".into(),
        }));
        let result = p1_wait_completion(admitted, P1_GUARD);
        if let Some(worker) = self.worker.borrow_mut().take() { worker.join().expect("queue action worker joined"); }
        result.expect("admitted queue action completion")
    }
}
fn p1_queue_action<T: Send + 'static>(
    s: &Arc<Surface>, action: impl FnOnce(Arc<Surface>, String) -> T + Send + 'static,
) -> P1QueueAction<T> {
    let completion = std::rc::Rc::new(std::cell::RefCell::new(None));
    let worker = std::rc::Rc::new(std::cell::RefCell::new(None));
    let record = completion.clone(); let record_worker = worker.clone();
    let observer = p1_on_queue_reserved(s.id, move |surface, request| {
        let surface = surface.clone(); let request_id = request.id.clone();
        let admitted = p1_start_completion("queue reservation action completed", |tx| {
            let worker = std::thread::Builder::new().spawn(move || {
                let result = action(surface, request_id);
                let _ = tx.send(result);
            }).map_err(|e| format!("queue action worker admission: {e}"))?;
            *record_worker.borrow_mut() = Some(worker);
            Ok(())
        });
        *record.borrow_mut() = Some(admitted);
    });
    P1QueueAction { _observer: observer, completion, worker }
}

fn p1_master_recorder(fx: &Fx, tag: &str) -> P1Recorder {
    let recorder = p1_recorder(&fx.daemon, &fx.dir, tag, Some("master"));
    recorder.install_always_admit();
    assert!(crate::governance::d9_scope(&fx.daemon, &recorder.surface, false));
    recorder
}

fn p1_refuse_queue_once(fx: &Fx, s: &Arc<Surface>) {
    let sc = s.clone();
    crate::agent_write::set_test_after_pre_record(move |_| { sc.input_gen.fetch_add(1, Ordering::AcqRel); });
    assert!(!hand_off(fx, s), "claim refuses injected foreign input");
    p1_wait_completion(p1_writer_idle(s), P1_GUARD).expect("refused queue operation settled");
}

fn p1_dialog_recorder(fx: &Fx, tag: &str, master: bool) -> P1Recorder {
    let recorder = p1_recorder(&fx.daemon, &fx.dir, tag, master.then_some("master"));
    let s = &recorder.surface;
    assert_eq!(crate::governance::d9_scope(&fx.daemon, s, false), master);
    d9_paint(s, &D9_DIALOG, 1, 8);
    let ctx = crate::governance::submit_cr_guard_context(s).expect("prompt markers resolve");
    assert!(ctx.dialog_at_handoff);
    d9_paint(s, &[], 0, 0);
    *s.receiver_proof.lock().unwrap() = Some(Arc::new(DialogAtSubmitOnce {
        seat: Arc::downgrade(s), painted: std::sync::atomic::AtomicBool::new(false),
    }));
    recorder
}


struct P1PauseAtSubmit {
    entered: Mutex<Option<std::sync::mpsc::Sender<()>>>,
    release: Mutex<std::sync::mpsc::Receiver<()>>,
    paused: Arc<std::sync::atomic::AtomicBool>,
}
impl Proof for P1PauseAtSubmit {
    fn admit(&self) -> Result<(), ProofLoss> { Ok(()) }
    fn recheck(&self, phase: Phase) -> Result<(), ProofLoss> {
        if phase == Phase::Submit {
            if let Some(entered) = self.entered.lock().unwrap().take() {
                self.paused.store(true, Ordering::Release);
                let _ = entered.send(());
                let _ = self.release.lock().unwrap().recv();
                self.paused.store(false, Ordering::Release);
            }
        }
        Ok(())
    }
}


#[derive(Clone, Copy, Debug)]
enum P1CompletionFault { Delayed, Missing }
struct P1CompletionControl {
    event: &'static str,
    fault: P1CompletionFault,
    consumed: bool,
    relays: Vec<std::thread::JoinHandle<()>>,
}
thread_local! {
    static P1_COMPLETION_CONTROL: std::cell::RefCell<Option<P1CompletionControl>> = const { std::cell::RefCell::new(None) };
}

#[test]
#[cfg_attr(not(unix), ignore = "좌석 명령이 POSIX 셸 문법")]
fn p1_completion_wc1_each_case_delayed_and_missing_actual_receipt() {
    let cases: &[(&str, fn(), &str)] = &[
        ("row17", d9_pre_cr_t_w3_a_dialog_during_the_settle_withholds_the_cr_with_one_owner, "writer operation leases released"),
        ("row14", d9_pre_cr_t_l1_a_human_byte_during_the_settle_lands_after_the_cr, "admitted queue body acknowledged before held CR"),
        ("row18", d9_pre_cr_t_w3c_outside_d9_scope_the_queue_job_proof_is_unchanged, "writer operation leases released"),
        ("row1", d9_bound_i2_late_busy_undoes_owned_marks_without_refusal, "writer operation leases released"),
        ("row2", d9_bound_i2_non_master_is_not_idle_gated, "queue reservation action completed"),
        ("row3", d9_bound_i2_queue_busy_holds_then_first_idle_pass_delivers, "writer operation leases released"),
        ("row4", d9_bound_i2_slot_lifecycle_and_head_replacement, "writer operation leases released"),
        ("row5", d9_bound_i2_writer_hold_keeps_queue_alarms_live, "writer operation leases released"),
        ("row6", d9_bound_i3_busy_holds_do_not_reset_refusals, "writer operation leases released"),
        ("row7", d9_bound_i3_busy_queue_escalates_once_then_delivers, "writer operation leases released"),
        ("row8", d9_bound_i3_controller_owned_claim_is_counted_once, "queue reservation action completed"),
        ("row9", d9_bound_i3_idle_nonwriter_hold_breaks_busy_continuity, "writer operation leases released"),
        ("row10", d9_bound_i3_late_refusal_does_not_restore_a_replaced_head_slot, "queue reservation action completed"),
        ("row11", d9_bound_i3_queue_refusals_are_counted_bounded_and_cleared, "writer operation leases released"),
        ("row12", d9_bound_t_s0_a_write_finished_between_judgment_and_hand_off_refuses_the_claim, "foreign writer operation released"),
        ("row13", d9_bound_t_s0h_a_human_key_time_between_judgment_and_hand_off_refuses_the_claim, "writer operation leases released"),
        ("row15", d9_pre_cr_t_w1_a_write_ahead_inside_the_r1_window_refuses_the_claim, "queue reservation action completed"),
        ("row16", d9_pre_cr_t_w2_a_human_key_inside_the_r1_window_refuses_the_claim, "queue reservation action completed"),
    ];
    for &(name, case, event) in cases {
        for fault in [P1CompletionFault::Delayed, P1CompletionFault::Missing] {
            P1_COMPLETION_CONTROL.with(|slot| {
                assert!(slot.borrow().is_none());
                *slot.borrow_mut() = Some(P1CompletionControl { event, fault, consumed: false, relays: Vec::new() });
            });
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(case));
            let control = P1_COMPLETION_CONTROL.with(|slot| slot.borrow_mut().take().unwrap());
            for relay in control.relays { relay.join().expect("completion relay exits after fixture cleanup"); }
            assert!(control.consumed, "{name}: actual named receipt was never reached: {event}");
            match fault {
                P1CompletionFault::Delayed => assert!(outcome.is_ok(), "{name}: delayed actual completion changed verdict"),
                P1CompletionFault::Missing => {
                    let panic = outcome.expect_err("missing actual completion must fail");
                    let message = panic.downcast_ref::<String>().cloned()
                        .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default();
                    assert!(message.contains("CompletionTimeout") && message.contains(event),
                        "{name}: missing completion must fail at named guard, got {message}");
                }
            }
            println!("P1-CONTROL {name} {fault:?}: {event}");
        }
    }
}

#[test]
fn p1_completion_neutral_marker_preserves_track_and_fifo_order() {
    let track = Arc::new(crate::state::InjectTrack::default());
    track.begin(); track.note_handoff(); track.note_wrote(); track.submit_handed();
    track.submit_written(); track.note_body("seed", None); track.end();
    let before = format!("{track:?}"); // derived Debug enumerates every current field, including all counters
    let (tx, rx) = std::sync::mpsc::channel();
    tx.send(crate::state::WriteReq::P1ReceiverBarrier(b"marker".to_vec())).unwrap();
    drop(tx);
    let mut bytes = Vec::new();
    crate::state::run_writer_loop_tracked(&mut bytes, rx, Arc::new(std::sync::atomic::AtomicBool::new(false)), Some(track.clone()));
    assert_eq!(bytes, b"marker", "receiver barrier writes its marker at its FIFO position");
    assert_eq!(format!("{track:?}"), before, "receiver marker leaves every InjectTrack field unchanged");
    let wrote = track.wrote();
    let (tx, rx) = std::sync::mpsc::channel();
    tx.send(crate::state::WriteReq::P1ReceiverBarrier(b"first".to_vec())).unwrap();
    tx.send(crate::state::WriteReq::Data(b"ordinary".to_vec())).unwrap();
    drop(tx);
    let mut bytes = Vec::new();
    crate::state::run_writer_loop_tracked(&mut bytes, rx, Arc::new(std::sync::atomic::AtomicBool::new(false)), Some(track.clone()));
    assert_eq!(bytes, b"firstordinary", "receiver barrier precedes the next ordinary writer operation");
    assert_eq!(track.wrote(), wrote + 1, "ordinary Data remains a counted write");
}

#[test]
fn p1_completion_neutral_marker_write_error_stops_before_later_data() {
    struct FailFirst { calls: Vec<Vec<u8>> }
    impl std::io::Write for FailFirst {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.calls.push(bytes.to_vec());
            if self.calls.len() == 1 { Err(std::io::ErrorKind::BrokenPipe.into()) } else { Ok(bytes.len()) }
        }
        fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
    }
    let track = Arc::new(crate::state::InjectTrack::default());
    let before = format!("{track:?}");
    let (tx, rx) = std::sync::mpsc::channel();
    tx.send(crate::state::WriteReq::P1ReceiverBarrier(b"failed marker".to_vec())).unwrap();
    tx.send(crate::state::WriteReq::Data(b"must not run".to_vec())).unwrap();
    drop(tx);
    let mut writer = FailFirst { calls: Vec::new() };
    crate::state::run_writer_loop_tracked(&mut writer, rx, Arc::new(std::sync::atomic::AtomicBool::new(false)), Some(track.clone()));
    assert_eq!(writer.calls, vec![b"failed marker".to_vec()], "barrier write error stops the writer before subsequent Data");
    assert_eq!(format!("{track:?}"), before, "failed marker also leaves InjectTrack untouched");
}
