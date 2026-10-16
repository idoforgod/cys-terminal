//! F-01 (0.14.52) display activity observer — reducer, collector, identity and serializer pins.
//!
//! Every frame used here is SYNTHETIC (hand-written bytes / test grammars). These tests prove the
//! substrate rules (R3-1 / R3-2 / U4-1 / U4-2 / U4-4); they cannot and do not prove native CLI grammar —
//! that is the native capture corpus's job.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use serde_json::{json, Value};

use crate::activity::*;
use crate::handlers::{dispatch, Reply};
use crate::state::{Daemon, Surface};
use cys::Request;

// ---------------------------------------------------------------------------------------------
// Pure reducer
// ---------------------------------------------------------------------------------------------

const P: Params = Params::LOCKED;

fn act(sig: u64) -> Sample {
    Sample::Active { sig }
}

/// Establishes Working with four changing active samples at t0, t0+1, t0+2, t0+3.
fn working_at(t0: f64) -> Reducer {
    let mut r = Reducer::new(t0);
    for i in 0..4 {
        r.step(act(100 + i), t0 + i as f64, &P);
    }
    assert_eq!(r.effective, Effective::Working, "precondition: dwell established");
    r
}

#[test]
fn dwell_requires_four_samples_spanning_three_seconds() {
    let mut r = Reducer::new(0.0);
    for (i, t) in [0.0, 1.0, 2.0].iter().enumerate() {
        r.step(act(i as u64), *t, &P);
        assert_eq!(r.effective, Effective::Unknown, "no Working before the dwell at t={t}");
        assert_eq!(r.reason, "active candidate");
    }
    r.step(act(9), 3.0, &P);
    assert_eq!(r.effective, Effective::Working);
    // four samples but spanning < 3 s is not enough
    let mut q = Reducer::new(0.0);
    for (i, t) in [0.0, 0.9, 1.8, 2.7].iter().enumerate() {
        q.step(act(i as u64), *t, &P);
    }
    assert_eq!(q.effective, Effective::Unknown, "span 2.7 s < H");
}

#[test]
fn dwell_gap_exactly_two_seconds_continues_above_two_restarts() {
    let mut ok = Reducer::new(0.0);
    for (i, t) in [0.0, 1.0, 2.0, 4.0].iter().enumerate() {
        ok.step(act(i as u64), *t, &P);
    }
    assert_eq!(ok.effective, Effective::Working, "gap of exactly 2 s keeps the candidate");
    let mut late = Reducer::new(0.0);
    for (i, t) in [0.0, 1.0, 2.0, 4.001].iter().enumerate() {
        late.step(act(i as u64), *t, &P);
    }
    assert_eq!(late.effective, Effective::Unknown, "gap above 2 s restarts the candidate");
}

#[test]
fn transient_rejection_cancels_an_initial_candidate() {
    let mut r = Reducer::new(0.0);
    r.step(act(1), 0.0, &P);
    r.step(act(2), 1.0, &P);
    r.step(act(3), 2.0, &P);
    r.step(Sample::Transient(Transient::OddEndpoint), 2.5, &P);
    r.step(act(4), 3.0, &P);
    assert_eq!(r.effective, Effective::Unknown, "the rejection restarted the dwell");
}

#[test]
fn k2_transient_rejections_retain_working_third_publishes_unknown() {
    let mut r = working_at(0.0);
    r.step(Sample::Transient(Transient::LockContention), 3.5, &P);
    assert_eq!((r.effective, r.rejections, r.retained), (Effective::Working, 1, true));
    r.step(Sample::Transient(Transient::Incomplete), 4.0, &P);
    assert_eq!((r.effective, r.rejections, r.retained), (Effective::Working, 2, true));
    r.step(Sample::Transient(Transient::OddEndpoint), 4.5, &P);
    assert_eq!(r.effective, Effective::Unknown, "third consecutive transient rejection");
    assert_eq!(r.reason, "odd output generation");
}

#[test]
fn retention_requires_last_positive_age_strictly_below_six_seconds() {
    // last positive at t=3
    let mut below = working_at(0.0);
    below.step(Sample::Transient(Transient::LockContention), 8.999, &P);
    assert_eq!(below.effective, Effective::Working, "age 5.999 s is retained");
    let mut exact = working_at(0.0);
    exact.step(Sample::Transient(Transient::LockContention), 9.0, &P);
    assert_eq!(exact.effective, Effective::Unknown, "age exactly 6 s is not retained");
}

#[test]
fn rejection_never_refreshes_the_lease() {
    let mut r = working_at(0.0); // last positive 3.0
    r.step(Sample::Transient(Transient::LockContention), 5.0, &P);
    assert_eq!(r.last_positive, Some(3.0), "a rejection does not renew");
    assert_eq!(r.effective_at(8.999, &P).0, Effective::Working);
    assert_eq!(r.effective_at(9.0, &P), (Effective::Unknown, "lease expired"));
}

#[test]
fn serializer_side_lease_expires_without_any_observation() {
    let r = working_at(0.0);
    assert_eq!(r.effective_at(8.5, &P).0, Effective::Working);
    assert_eq!(r.effective_at(9.0, &P), (Effective::Unknown, "lease expired"), "observer stalled");
}

#[test]
fn accepted_active_resets_the_rejection_counter() {
    let mut r = working_at(0.0);
    r.step(Sample::Transient(Transient::LockContention), 3.2, &P);
    r.step(act(7), 3.4, &P);
    assert_eq!((r.rejections, r.retained), (0, false));
    r.step(Sample::Transient(Transient::LockContention), 3.6, &P);
    r.step(Sample::Transient(Transient::LockContention), 3.8, &P);
    assert_eq!(r.effective, Effective::Working, "two more retained after the reset");
}

#[test]
fn idle_and_decision_stop_working_immediately_regardless_of_k() {
    let mut r = working_at(0.0);
    r.step(Sample::Idle, 3.1, &P);
    assert_eq!((r.effective, r.reason), (Effective::Waiting, "idle frame"));
    let mut d = working_at(0.0);
    d.step(Sample::Decision(DecisionKind::Permission), 3.1, &P);
    assert_eq!((d.effective, d.reason), (Effective::WaitingForInput, "permission panel"));
    let mut q = working_at(0.0);
    q.step(Sample::Decision(DecisionKind::Question), 3.1, &P);
    assert_eq!((q.effective, q.raw), (Effective::WaitingForInput, Raw::Decision(DecisionKind::Question)));
}

#[test]
fn positive_contradiction_is_immediate_unknown_without_k_grace() {
    for s in [Semantic::OtherFamily, Semantic::Conflict, Semantic::UnsupportedGeometry, Semantic::ParserPoison, Semantic::AdapterPanic] {
        let mut r = working_at(0.0);
        r.step(Sample::Semantic(s), 3.1, &P);
        assert_eq!(r.effective, Effective::Unknown, "{s:?} must not be retained");
        assert_eq!(r.reason, s.as_str());
    }
}

#[test]
fn invalidation_discards_working_and_the_last_positive() {
    for i in [Invalid::Identity, Invalid::View, Invalid::ResetPending, Invalid::Staging, Invalid::Exited] {
        let mut r = working_at(0.0);
        r.step(Sample::Invalidated(i), 3.1, &P);
        assert_eq!((r.effective, r.last_positive), (Effective::Unknown, None), "{i:?}");
    }
}

#[test]
fn reacquisition_after_unknown_requires_the_full_dwell() {
    let mut r = working_at(0.0);
    for t in [3.2, 3.4, 3.6] {
        r.step(Sample::Transient(Transient::LockContention), t, &P);
    }
    assert_eq!(r.effective, Effective::Unknown);
    r.step(act(50), 4.0, &P);
    assert_eq!(r.effective, Effective::Unknown, "one good sample cannot restore a discarded latch");
    r.step(act(51), 5.0, &P);
    r.step(act(52), 6.0, &P);
    r.step(act(53), 7.0, &P);
    assert_eq!(r.effective, Effective::Working);
}

/// U4-1 at the reducer: an incomplete frame (no complete signature, no contradiction) is transient.
#[test]
fn u4_1_incomplete_frame_counts_against_k_and_never_ends_working_by_itself() {
    let mut r = working_at(0.0);
    r.step(Sample::Transient(Transient::Incomplete), 3.5, &P);
    assert_eq!(r.effective, Effective::Working, "a split repaint does not end Working");
    assert_eq!(r.reason, "incomplete frame");
    r.step(act(60), 4.0, &P);
    assert_eq!((r.effective, r.rejections), (Effective::Working, 0));
}

/// U4-2: N identical accepted active samples end in Unknown "static active frame".
#[test]
fn u4_2_static_active_frame_cap() {
    let p = Params { static_n: 6, ..P };
    let mut r = Reducer::new(0.0);
    for t in 0..4 {
        r.step(act(7), t as f64, &p);
    }
    assert_eq!(r.effective, Effective::Working, "identical frames can establish before N");
    r.step(act(7), 4.0, &p);
    assert_eq!(r.effective, Effective::Working, "count 5 < N");
    r.step(act(7), 5.0, &p);
    assert_eq!((r.effective, r.reason), (Effective::Unknown, "static active frame"));
    for t in 6..20 {
        r.step(act(7), t as f64, &p);
        assert_eq!(r.effective, Effective::Unknown, "the same static frame never re-establishes (t={t})");
    }
    // a changed frame starts over with a full dwell
    for (i, t) in (20..24).enumerate() {
        r.step(act(100 + i as u64), t as f64, &p);
    }
    assert_eq!(r.effective, Effective::Working);
}

/// U4-2 quiet-active control: a seat whose non-neutral cells keep changing never trips the cap.
#[test]
fn u4_2_quiet_active_control_never_trips() {
    let p = Params { static_n: 6, ..P };
    let mut r = Reducer::new(0.0);
    for t in 0..300u64 {
        r.step(act(t % 3), t as f64, &p);
    }
    assert_eq!(r.effective, Effective::Working);
}

/// U4-2 is not silence-as-idle: no observation (silence) is a non-input and never yields Waiting.
#[test]
fn u4_2_silence_is_a_non_input() {
    let r = working_at(0.0);
    assert_ne!(r.effective_at(100.0, &P).0, Effective::Waiting);
}

// ---------------------------------------------------------------------------------------------
// Grammar precedence (U4-1)
// ---------------------------------------------------------------------------------------------

/// Synthetic grammar: active when a row contains `active`, idle when a row contains `idle`.
struct G {
    fam: Family,
    active: &'static str,
    idle: &'static str,
    decision: &'static str,
}

impl Grammar for G {
    fn family(&self) -> Family {
        self.fam
    }
    fn revision(&self) -> &'static str {
        "synthetic-test-1"
    }
    fn classify(&self, v: &View<'_>) -> Verdict {
        let has = |tok: &str| v.rows.iter().any(|(_, l)| l.contains(tok));
        let (a, i, d) = (has(self.active), has(self.idle), has(self.decision));
        match (a, i || d) {
            (true, true) => Verdict::Conflict,
            (true, false) => {
                use std::hash::{Hash, Hasher};
                let mut h = std::collections::hash_map::DefaultHasher::new();
                for (_, l) in v.rows {
                    l.hash(&mut h);
                }
                Verdict::Active { sig: h.finish() }
            }
            (false, true) if d => Verdict::Decision(DecisionKind::Permission),
            (false, true) => Verdict::Idle,
            (false, false) => Verdict::NoMatch,
        }
    }
}

static CLAUDE_G: G = G { fam: Family::Claude, active: "SYN-CLAUDE-WORKING", idle: "SYN-CLAUDE-IDLE", decision: "SYN-CLAUDE-ALLOW?" };
static CODEX_G: G = G { fam: Family::Codex, active: "SYN-CODEX-WORKING", idle: "SYN-CODEX-IDLE", decision: "SYN-CODEX-APPROVE?" };

fn grammars_t() -> Vec<&'static dyn Grammar> {
    vec![&CLAUDE_G, &CODEX_G]
}

fn view_of(lines: &[&str]) -> Vec<(u16, String)> {
    lines.iter().enumerate().map(|(i, l)| (i as u16, l.to_string())).collect()
}

fn classify_lines(fam: Family, lines: &[&str]) -> Sample {
    let rows = view_of(lines);
    let v = View { rows: &rows, size: (24, 80), cursor: (0, 0), alt: false, title: "" };
    classify_with(fam, &v, &grammars_t()).0
}

#[test]
fn u4_1_no_complete_signature_without_contradiction_is_transient_incomplete() {
    assert_eq!(classify_lines(Family::Claude, &["half a repaint ...", ""]), Sample::Transient(Transient::Incomplete));
}

#[test]
fn u4_1_positive_contradictions_are_semantic() {
    assert_eq!(classify_lines(Family::Claude, &["SYN-CODEX-WORKING"]), Sample::Semantic(Semantic::OtherFamily));
    assert_eq!(
        classify_lines(Family::Claude, &["SYN-CLAUDE-WORKING", "SYN-CODEX-IDLE"]),
        Sample::Semantic(Semantic::OtherFamily),
        "own active plus another family's complete signature"
    );
    assert_eq!(classify_lines(Family::Claude, &["SYN-CLAUDE-WORKING", "SYN-CLAUDE-IDLE"]), Sample::Semantic(Semantic::Conflict));
    assert_eq!(classify_lines(Family::Gemini, &["anything"]), Sample::Semantic(Semantic::UnsupportedFamily));
}

#[test]
fn own_complete_signatures_publish_their_state() {
    assert!(matches!(classify_lines(Family::Claude, &["SYN-CLAUDE-WORKING"]), Sample::Active { .. }));
    assert_eq!(classify_lines(Family::Codex, &["SYN-CODEX-IDLE"]), Sample::Idle);
    assert_eq!(classify_lines(Family::Codex, &["SYN-CODEX-APPROVE?"]), Sample::Decision(DecisionKind::Permission));
}

#[test]
fn family_comes_from_agent_name_aliases_only() {
    assert_eq!(Family::from_agent_name("Claude"), Some(Family::Claude));
    assert_eq!(Family::from_agent_name("codex"), Some(Family::Codex));
    assert_eq!(Family::from_agent_name("gemini"), Some(Family::Gemini));
    assert_eq!(Family::from_agent_name("agy"), Some(Family::Gemini));
    for not_a_family in ["grok", "node", "cmd", "powershell", "bash", "worker", "reviewer-gemini", ""] {
        assert_eq!(Family::from_agent_name(not_a_family), None, "{not_a_family}");
    }
}

#[test]
fn production_registry_has_no_grammar_without_a_native_corpus() {
    // Until a native corpus locks a grammar, every family is Unknown (no fabricated footer).
    assert!(grammars().is_empty());
}

// ---------------------------------------------------------------------------------------------
// Collection window
// ---------------------------------------------------------------------------------------------

#[test]
fn window_is_bottom_eight_plus_cursor_span_deduplicated_and_clipped() {
    let mut out = Vec::new();
    window_rows(40, 10, &mut out);
    assert_eq!(out, vec![7, 8, 9, 10, 11, 12, 13, 32, 33, 34, 35, 36, 37, 38, 39]);
    assert_eq!(out.len(), MAX_ROWS);
    window_rows(40, 37, &mut out);
    assert_eq!(out, (32..40).collect::<Vec<u16>>(), "cursor inside the bottom rows dedups");
    window_rows(5, 4, &mut out);
    assert_eq!(out, vec![0, 1, 2, 3, 4]);
    window_rows(40, 0, &mut out);
    assert_eq!(out, vec![0, 1, 2, 3, 32, 33, 34, 35, 36, 37, 38, 39]);
    window_rows(0, 0, &mut out);
    assert!(out.is_empty());
}

// ---------------------------------------------------------------------------------------------
// Live-surface fixture (synthetic `sleep` seat; the PTY never produces output)
// ---------------------------------------------------------------------------------------------

struct Fx {
    daemon: Arc<Daemon>,
    _dir: std::path::PathBuf,
    _g: std::sync::MutexGuard<'static, ()>,
}

fn fx(tag: &str) -> Fx {
    let g = crate::governance::PACK_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    crate::delivery::tests::isolate_state_dir_for_thread(tag);
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("cys-activity-{tag}-{}-{n}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    std::fs::write(dir.join("acl.json"), r#"{"default":"allow","rules":[]}"#).unwrap();
    std::env::set_var(cys::pack::ENV_PACK_DIR, &dir);
    let daemon = Daemon::new(dir.join("cysd.sock"));
    Fx { daemon, _dir: dir, _g: g }
}

fn seat(fx: &Fx, role: &str, agent: Option<&str>) -> Arc<Surface> {
    let s = fx
        .daemon
        .create_surface(None, Some("sleep 30".into()), None, Some(role.into()), 24, 80)
        .expect("create surface");
    fx.daemon.surfaces.lock().unwrap().insert(s.id, s.clone());
    if let Some(a) = agent {
        *s.agent_meta.lock().unwrap() = Some((a.into(), a.into()));
    }
    s
}

/// Writes bytes into the seat's parser as one complete publication (odd → even, like the reader).
fn publish(s: &Surface, bytes: &[u8]) {
    s.output_gen.fetch_add(1, Ordering::AcqRel);
    s.parser.lock().unwrap().process(bytes);
    s.output_gen.fetch_add(1, Ordering::AcqRel);
}

fn set_after_copy(f: Option<Box<dyn FnMut()>>) {
    AFTER_COPY.with(|h| *h.borrow_mut() = f);
}

#[test]
fn collect_copies_bounded_rows_without_growing_its_buffers() {
    let f = fx("collect-bounds");
    let s = seat(&f, "worker", Some("claude"));
    publish(&s, b"\x1b[2J\x1b[24;1Hbottom line\x1b[1;1Htop");
    let mut frame = Frame::new();
    let (cap_cells, cap_rows) = (frame.cells.capacity(), frame.row_index.capacity());
    assert!(matches!(collect(&s, &mut frame), Collected::Frame));
    assert_eq!(frame.cells.len(), frame.row_index.len() * 80);
    assert!(frame.row_index.len() <= MAX_ROWS && frame.cells.len() <= MAX_CELLS);
    assert_eq!((frame.cells.capacity(), frame.row_index.capacity()), (cap_cells, cap_rows), "no reallocation");
    let text = frame.text_rows();
    assert!(text.iter().any(|(r, l)| *r == 23 && l.starts_with("bottom line")));
    assert!(text.iter().any(|(r, l)| *r == 0 && l.starts_with("top")), "cursor row ±3 copied");
}

/// R3-1 deterministic interleaving: copy at an even generation, a whole publication completes before
/// the second endpoint (both even, unequal) — published-only admission MUST accept. Equality-requiring
/// collection would reject this and fail here.
#[test]
fn published_only_admission_accepts_even_unequal_endpoints() {
    let f = fx("even-unequal");
    let s = seat(&f, "worker", Some("claude"));
    publish(&s, b"SYN-CLAUDE-WORKING frame");
    let gen = Arc::clone(&s.output_gen);
    let mut frame = Frame::new();
    let mut r = Reducer::new(0.0);
    for tick in 0..4u64 {
        let g2 = Arc::clone(&gen);
        set_after_copy(Some(Box::new(move || {
            g2.fetch_add(2, Ordering::AcqRel); // a complete reader publication between the endpoints
        })));
        let before = s.output_gen.load(Ordering::Acquire);
        let got = collect(&s, &mut frame);
        set_after_copy(None);
        assert_ne!(s.output_gen.load(Ordering::Acquire), before, "endpoints differed");
        assert!(matches!(got, Collected::Frame), "tick {tick}: even/even unequal must be admitted");
        let text = frame.text_rows();
        let v = View { rows: &text, size: (frame.rows, frame.cols), cursor: frame.cursor, alt: frame.alt, title: &frame.title };
        let (sample, _) = classify_with(Family::Claude, &v, &grammars_t());
        // make each tick's frame distinct so U4-2 cannot interfere
        let sample = match sample {
            Sample::Active { sig } => Sample::Active { sig: sig ^ tick },
            other => other,
        };
        r.step(sample, tick as f64, &P);
    }
    assert_eq!(r.effective, Effective::Working, "accepted samples over four ticks establish Working");
}

#[test]
fn odd_endpoint_is_a_transient_rejection() {
    let f = fx("odd");
    let s = seat(&f, "worker", Some("claude"));
    let mut frame = Frame::new();
    s.output_gen.fetch_add(1, Ordering::AcqRel); // publication in progress at the first endpoint
    assert!(matches!(collect(&s, &mut frame), Collected::Rejected(Sample::Transient(Transient::OddEndpoint))));
    s.output_gen.fetch_add(1, Ordering::AcqRel);
    let g2 = Arc::clone(&s.output_gen);
    set_after_copy(Some(Box::new(move || {
        g2.fetch_add(1, Ordering::AcqRel); // second endpoint odd
    })));
    let got = collect(&s, &mut frame);
    set_after_copy(None);
    assert!(matches!(got, Collected::Rejected(Sample::Transient(Transient::OddEndpoint))));
}

#[test]
fn contended_parser_lock_is_a_transient_rejection_not_a_wait() {
    let f = fx("contended");
    let s = seat(&f, "worker", Some("claude"));
    let held = s.parser.lock().unwrap();
    let mut frame = Frame::new();
    let t0 = std::time::Instant::now();
    let got = collect(&s, &mut frame);
    assert!(t0.elapsed() < std::time::Duration::from_millis(50), "never blocks");
    assert!(matches!(got, Collected::Rejected(Sample::Transient(Transient::LockContention))));
    drop(held);
}

#[test]
fn poisoned_parser_is_a_positive_contradiction_never_recovered() {
    let f = fx("poison");
    let s = seat(&f, "worker", Some("claude"));
    let s2 = Arc::clone(&s);
    let _ = std::thread::spawn(move || {
        let _g = s2.parser.lock().unwrap();
        panic!("poison the parser for the test");
    })
    .join();
    let mut frame = Frame::new();
    assert!(matches!(collect(&s, &mut frame), Collected::Rejected(Sample::Semantic(Semantic::ParserPoison))));
    assert!(s.parser.is_poisoned(), "the display collector does not clear poison");
}

#[test]
fn over_256_columns_is_unsupported_geometry() {
    let f = fx("wide");
    let s = seat(&f, "worker", Some("claude"));
    s.parser.lock().unwrap().set_size(24, 300);
    let mut frame = Frame::new();
    assert!(matches!(collect(&s, &mut frame), Collected::Rejected(Sample::Semantic(Semantic::UnsupportedGeometry))));
    assert!(frame.cells.is_empty());
}

#[test]
fn identity_write_between_endpoints_invalidates_the_sample() {
    let f = fx("ident-between");
    let s = seat(&f, "worker", Some("claude"));
    let s2 = Arc::clone(&s);
    set_after_copy(Some(Box::new(move || invalidate_identity(&s2))));
    let mut frame = Frame::new();
    let got = collect(&s, &mut frame);
    set_after_copy(None);
    assert!(matches!(got, Collected::Rejected(Sample::Invalidated(Invalid::Identity))));
    // a writer still in progress also rejects
    let _w = IdentityWrite::begin(&s);
    assert!(matches!(collect(&s, &mut frame), Collected::Rejected(Sample::Invalidated(Invalid::Identity))));
}

// ---------------------------------------------------------------------------------------------
// Identity guard
// ---------------------------------------------------------------------------------------------

#[test]
fn identity_guard_overlapping_writers_and_unwinding_writer() {
    let f = fx("guard");
    let s = seat(&f, "worker", Some("claude"));
    let rev0 = s.activity_identity_rev.load(Ordering::SeqCst);
    let a = IdentityWrite::begin(&s);
    let b = IdentityWrite::begin(&s);
    assert_eq!(s.activity_pending.load(Ordering::SeqCst), 2);
    drop(a);
    assert_eq!(s.activity_pending.load(Ordering::SeqCst), 1, "one writer still in progress");
    assert_eq!(s.activity_identity_rev.load(Ordering::SeqCst), rev0 + 1);
    drop(b);
    assert_eq!(s.activity_pending.load(Ordering::SeqCst), 0);
    let s2 = Arc::clone(&s);
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        let _w = IdentityWrite::begin(&s2);
        panic!("writer unwinds mid-mutation");
    }));
    assert!(r.is_err());
    assert_eq!(s.activity_pending.load(Ordering::SeqCst), 0, "unwinding releases pending");
    assert_eq!(s.activity_identity_rev.load(Ordering::SeqCst), rev0 + 3, "and still advances the revision");
}

// ---------------------------------------------------------------------------------------------
// observe_seat + serializer (store-level)
// ---------------------------------------------------------------------------------------------

fn observe(f: &Fx, s: &Surface, frame: &mut Frame, now: f64) {
    let mut st = Stats::default();
    observe_seat(&f.daemon.activity, s, frame, &grammars_t(), now, crate::state::now_epoch(), &mut st);
}

/// Drives a seat to Working with four distinct active frames at t=0..3.
fn drive_working(f: &Fx, s: &Surface, frame: &mut Frame) {
    for t in 0..4 {
        publish(s, format!("\x1b[1;1HSYN-CLAUDE-WORKING tick {t}   ").as_bytes());
        observe(f, s, frame, t as f64);
    }
    let w = wire_at(&f.daemon.activity, s, 3.0, &P);
    assert_eq!(w["state"], "working", "precondition: {w}");
}

#[test]
fn serializer_requires_pending_zero_and_matching_identity_revision() {
    let f = fx("ser-ident");
    let s = seat(&f, "worker", Some("claude"));
    let mut frame = Frame::new();
    drive_working(&f, &s, &mut frame);
    {
        let _w = IdentityWrite::begin(&s);
        assert_eq!(wire_at(&f.daemon.activity, &s, 3.1, &P)["state"], "unknown", "writer in progress");
    }
    let w = wire_at(&f.daemon.activity, &s, 3.2, &P);
    assert_eq!((w["state"].as_str(), w["reason"].as_str()), (Some("unknown"), Some("identity changed")), "{w}");
}

#[test]
fn serializer_enforces_the_lease_when_the_observer_stalls() {
    let f = fx("ser-lease");
    let s = seat(&f, "worker", Some("claude"));
    let mut frame = Frame::new();
    drive_working(&f, &s, &mut frame);
    assert_eq!(wire_at(&f.daemon.activity, &s, 8.9, &P)["state"], "working");
    let w = wire_at(&f.daemon.activity, &s, 9.0, &P);
    assert_eq!((w["state"].as_str(), w["reason"].as_str()), (Some("unknown"), Some("lease expired")));
}

#[test]
fn same_size_resize_invalidates_continuity() {
    let f = fx("resize-same");
    let s = seat(&f, "worker", Some("claude"));
    let mut frame = Frame::new();
    drive_working(&f, &s, &mut frame);
    {
        let mut p = s.parser.lock().unwrap();
        bump_view(&s); // what repaint::set_parser_size does under the lock
        p.set_size(24, 80);
    }
    assert_eq!(wire_at(&f.daemon.activity, &s, 3.5, &P)["reason"], "view changed", "serializer sees it at once");
    publish(&s, b"\x1b[1;1HSYN-CLAUDE-WORKING after resize");
    observe(&f, &s, &mut frame, 4.0);
    let w = wire_at(&f.daemon.activity, &s, 4.0, &P);
    assert_eq!((w["state"].as_str(), w["reason"].as_str()), (Some("unknown"), Some("view changed")), "{w}");
}

#[test]
fn reset_and_repaint_between_samples_is_not_continuity() {
    let f = fx("ris");
    let s = seat(&f, "worker", Some("claude"));
    let mut frame = Frame::new();
    drive_working(&f, &s, &mut frame);
    // ESC c then an identical repaint: geometry and final frame match, but it is a new view.
    publish(&s, b"\x1bc\x1b[1;1HSYN-CLAUDE-WORKING tick 3   ");
    assert_eq!(s.parser.lock().unwrap().screen().reset_count(), 1);
    observe(&f, &s, &mut frame, 4.0);
    let w = wire_at(&f.daemon.activity, &s, 4.0, &P);
    assert_eq!(w["reason"], "view changed", "{w}");
}

#[test]
fn vt100_reset_count_survives_the_reset_itself() {
    let mut p = vt100::Parser::new(5, 20, 0);
    assert_eq!(p.screen().reset_count(), 0);
    p.process(b"abc\x1bcdef\x1bc");
    assert_eq!(p.screen().reset_count(), 2);
    p.set_size(6, 20);
    assert_eq!(p.screen().reset_count(), 2, "resize is not a reset");
}

#[test]
fn exited_seat_serializes_unknown() {
    let f = fx("exited");
    let s = seat(&f, "worker", Some("claude"));
    let mut frame = Frame::new();
    drive_working(&f, &s, &mut frame);
    s.agent_exit_notified.store(true, Ordering::Relaxed);
    let w = wire(&f.daemon, &s);
    assert_eq!((w["state"].as_str(), w["reason"].as_str()), (Some("unknown"), Some("exited")));
}

#[test]
fn seat_without_a_confirmed_family_is_unknown_and_role_never_selects_one() {
    let f = fx("no-family");
    let mut frame = Frame::new();
    for role in ["master", "cso", "worker-7", "reviewer-gemini", "reviewer-codex", "dept-head"] {
        let s = seat(&f, role, None);
        publish(&s, b"\x1b[1;1HSYN-CLAUDE-WORKING");
        for t in 0..5 {
            observe(&f, &s, &mut frame, t as f64);
        }
        let w = wire_at(&f.daemon.activity, &s, 4.0, &P);
        assert_eq!((w["state"].as_str(), w["reason"].as_str()), (Some("unknown"), Some("unsupported family")), "{role}");
    }
}

#[test]
fn same_working_regardless_of_role_name() {
    let f = fx("role-perm");
    let mut frame = Frame::new();
    for role in ["master", "cso-2", "worker", "reviewer-x", "anything"] {
        let s = seat(&f, role, Some("claude"));
        drive_working(&f, &s, &mut frame);
    }
}

#[test]
fn windows_staging_yields_unknown() {
    let f = fx("staging");
    let s = seat(&f, "worker", None);
    *s.pending_agent_obs.lock().unwrap() = Some(("claude".into(), "node".into(), crate::state::now_epoch()));
    let mut frame = Frame::new();
    publish(&s, b"\x1b[1;1HSYN-CLAUDE-WORKING");
    observe(&f, &s, &mut frame, 0.0);
    assert_eq!(wire_at(&f.daemon.activity, &s, 0.0, &P)["reason"], "identity staging");
}

#[test]
fn live_quiescing_window_is_reset_pending() {
    let f = fx("quiescing");
    let s = seat(&f, "worker", Some("claude"));
    let mut frame = Frame::new();
    drive_working(&f, &s, &mut frame);
    *s.agent_status.lock().unwrap() = Some(crate::state::AgentStatus {
        state: "quiescing".into(),
        context_pct: None,
        task: None,
        updated_at: crate::state::now_epoch(),
    });
    publish(&s, b"\x1b[1;1HSYN-CLAUDE-WORKING during clear");
    observe(&f, &s, &mut frame, 4.0);
    assert_eq!(wire_at(&f.daemon.activity, &s, 4.0, &P)["reason"], "reset pending");
    // an orphaned window older than the default hold no longer suppresses
    s.agent_status.lock().unwrap().as_mut().unwrap().updated_at = crate::state::now_epoch() - 601.0;
    publish(&s, b"\x1b[1;1HSYN-CLAUDE-WORKING after window");
    observe(&f, &s, &mut frame, 5.0);
    assert_eq!(wire_at(&f.daemon.activity, &s, 5.0, &P)["reason"], "active candidate");
}

// ---------------------------------------------------------------------------------------------
// U4-4 census bindings through the production handlers
// ---------------------------------------------------------------------------------------------

fn rpc(f: &Fx, pid: Option<u32>, method: &str, params: Value) -> Value {
    let req = Request { id: json!(7), method: method.into(), params };
    let Reply::Single(resp) = dispatch(&f.daemon, req, pid) else { panic!("single reply expected") };
    resp
}

/// handlers.rs `surface.set_meta`: accepted (identical pair included) invalidates; rejected does not.
#[test]
fn set_meta_accepted_invalidates_rejected_does_not() {
    let f = fx("set-meta");
    let s = seat(&f, "worker", Some("claude"));
    let rev0 = s.activity_identity_rev.load(Ordering::SeqCst);
    let ok = rpc(&f, None, "surface.set_meta", json!({"surface_id": s.id, "agent": "claude", "agent_bin": "claude"}));
    assert!(ok.get("error").is_none(), "{ok}");
    assert_eq!(s.activity_identity_rev.load(Ordering::SeqCst), rev0 + 1, "identical registration still invalidates");
    // a caller pane that is a different live surface may not overwrite → rejected, no invalidation
    let other = seat(&f, "master", Some("claude"));
    let pid = 4_100_001u32;
    f.daemon.caller_cache.lock().unwrap().insert(
        pid,
        crate::state::CallerCacheEntry::new(Some(other.id), crate::state::now_epoch(), None, f.daemon.caller_gen.load(Ordering::Relaxed)),
    );
    let denied = rpc(&f, Some(pid), "surface.set_meta", json!({"surface_id": s.id, "agent": "codex", "agent_bin": "codex"}));
    assert!(denied.get("error").is_some(), "{denied}");
    assert_eq!(s.activity_identity_rev.load(Ordering::SeqCst), rev0 + 1, "rejected registration does not invalidate");
    assert_eq!(s.activity_pending.load(Ordering::SeqCst), 0);
}

/// handlers.rs `surface.quiesce`: accepted on invalidates before the mutation; off does not.
#[test]
fn quiesce_on_invalidates_off_does_not() {
    let f = fx("quiesce");
    let s = seat(&f, "worker", Some("claude"));
    let rev0 = s.activity_identity_rev.load(Ordering::SeqCst);
    let on = rpc(&f, None, "surface.quiesce", json!({"surface_id": s.id, "on": true}));
    assert!(on.get("error").is_none(), "{on}");
    assert_eq!(s.activity_identity_rev.load(Ordering::SeqCst), rev0 + 1);
    let off = rpc(&f, None, "surface.quiesce", json!({"surface_id": s.id, "on": false, "outcome": "cleared"}));
    assert!(off.get("error").is_none(), "{off}");
    assert_eq!(s.activity_identity_rev.load(Ordering::SeqCst), rev0 + 1, "off never establishes anything");
}

/// The wire carries an opaque epoch and a per-epoch increasing sequence; an identity change starts a new epoch.
#[test]
fn epoch_changes_with_identity_and_sequence_is_monotonic_within_it() {
    let f = fx("epoch");
    let s = seat(&f, "worker", Some("claude"));
    let mut frame = Frame::new();
    drive_working(&f, &s, &mut frame);
    let w1 = wire_at(&f.daemon.activity, &s, 3.0, &P);
    publish(&s, b"\x1b[1;1HSYN-CLAUDE-WORKING more");
    observe(&f, &s, &mut frame, 4.0);
    let w2 = wire_at(&f.daemon.activity, &s, 4.0, &P);
    assert_eq!(w1["display_epoch"], w2["display_epoch"]);
    assert!(w2["observation_seq"].as_u64() > w1["observation_seq"].as_u64());
    invalidate_identity(&s);
    observe(&f, &s, &mut frame, 5.0);
    let w3 = wire_at(&f.daemon.activity, &s, 5.0, &P);
    assert_ne!(w3["display_epoch"], w2["display_epoch"], "new epoch after an identity write");
    assert_eq!(w3["observation_seq"], 1);
}

/// Display never touches operational state: status, quiescing, queue and PTY input are unchanged by
/// observation (risk ①②④ boundary at the store level).
#[test]
fn observation_changes_no_operational_field() {
    let f = fx("noninterference");
    let s = seat(&f, "worker", Some("claude"));
    *s.agent_status.lock().unwrap() = Some(crate::state::AgentStatus {
        state: "waiting".into(),
        context_pct: Some(12),
        task: Some("t".into()),
        updated_at: 1.0,
    });
    let before = (
        s.agent_status.lock().unwrap().as_ref().map(|a| (a.state.clone(), a.context_pct, a.task.clone(), a.updated_at)),
        s.pending_queue.lock().unwrap().len(),
        s.input_gen.load(Ordering::Acquire),
        s.pending_input_bytes.load(Ordering::Relaxed),
        s.output_gen.load(Ordering::Acquire),
    );
    let mut frame = Frame::new();
    for t in 0..8 {
        observe(&f, &s, &mut frame, t as f64);
    }
    let after = (
        s.agent_status.lock().unwrap().as_ref().map(|a| (a.state.clone(), a.context_pct, a.task.clone(), a.updated_at)),
        s.pending_queue.lock().unwrap().len(),
        s.input_gen.load(Ordering::Acquire),
        s.pending_input_bytes.load(Ordering::Relaxed),
        s.output_gen.load(Ordering::Acquire),
    );
    assert_eq!(before, after);
}

/// repaint.rs production resize path (`apply_resize` → `set_parser_size`): a same-size resize is a new view.
#[test]
fn production_resize_path_bumps_the_view_even_at_the_same_size() {
    let f = fx("apply-resize");
    let s = seat(&f, "worker", Some("claude"));
    let v0 = s.activity_view_rev.load(Ordering::SeqCst);
    let (rows, cols) = s.parser.lock().unwrap().screen().size();
    crate::repaint::apply_resize(&s, rows, cols).expect("same-size resize");
    assert!(s.activity_view_rev.load(Ordering::SeqCst) > v0, "same-size resize must invalidate continuity");
}

/// U4-4 census pin: every production identity/view write site named in the census carries its display
/// guard right at the site. Structural, anchored on exact source lines (each anchor must occur once).
/// Behavioural bindings exist for set_meta, quiesce-on and resize; this pin binds the rest
/// (unix re-observation, Windows staging, Windows second-sample commit, session capture, clear repin,
/// reader parser replacement).
#[test]
fn u4_4_identity_census_every_named_production_site_is_guarded() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bin/cysd");
    // (file, anchor line (trimmed), guard token, lines before (+) or after (-) the anchor to search)
    let census: &[(&str, &str, &str, i32)] = &[
        ("handlers.rs", "*s.agent_meta.lock().unwrap() = Some((agent.clone(), bin.clone()));", "crate::activity::IdentityWrite::begin(&s)", 3),
        ("handlers.rs", "*s.pending_agent_obs.lock().unwrap() = Some((agent, bin, crate::state::now_epoch()));", "crate::activity::IdentityWrite::begin(&s)", 2),
        ("handlers.rs", "*surface.agent_meta.lock().unwrap() = Some((agent.clone(), agent_bin));", "crate::activity::IdentityWrite::begin(&surface)", 2),
        ("handlers.rs", "*pin = Some(new_sid.clone());", "crate::activity::IdentityWrite::begin(&surface)", 2),
        ("handlers.rs", "let at = crate::state::now_epoch();", "crate::activity::invalidate_identity(&surface)", 2),
        ("governance.rs", "*s.agent_meta.lock().unwrap() = Some((agent.clone(), bin.clone()));", "crate::activity::IdentityWrite::begin(&s)", 2),
        ("usage.rs", "*s.agent_session_id.lock().unwrap() = Some(sid);", "crate::activity::IdentityWrite::begin(s)", 2),
        ("repaint.rs", "match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| p.set_size(rows, cols))) {", "crate::activity::bump_view(s)", 3),
        ("state.rs", "process_chunk_isolated(&mut parser, chunk, dsr_count);", "crate::activity::bump_view(&surf)", -4),
    ];
    for (file, anchor, guard, span) in census {
        let src = std::fs::read_to_string(root.join(file)).unwrap_or_else(|e| panic!("read {file}: {e}"));
        let lines: Vec<&str> = src.lines().collect();
        let hits: Vec<usize> = lines.iter().enumerate().filter(|(_, l)| l.trim() == *anchor).map(|(i, _)| i).collect();
        assert_eq!(hits.len(), 1, "{file}: anchor must occur exactly once: {anchor}");
        let i = hits[0];
        let window: Vec<&str> = if *span > 0 {
            lines[i.saturating_sub(*span as usize)..i].to_vec()
        } else {
            lines[i + 1..(i + 1 + (-*span) as usize).min(lines.len())].to_vec()
        };
        assert!(window.iter().any(|l| l.contains(guard)), "{file}:{}: `{anchor}` lacks `{guard}` within {span} lines", i + 1);
    }
}

#[test]
fn vt100_synchronized_output_flag_follows_decset_2026_and_reset() {
    let mut p = vt100::Parser::new(5, 20, 0);
    assert!(!p.screen().synchronized_output());
    p.process(b"\x1b[?2026hpartial");
    assert!(p.screen().synchronized_output());
    p.process(b"\x1b[?2026l");
    assert!(!p.screen().synchronized_output());
    p.process(b"\x1b[?2026h\x1bc");
    assert!(!p.screen().synchronized_output(), "a full reset ends any open block");
}

#[test]
fn title_is_copied_into_reserved_capacity_and_truncated_not_grown() {
    let f = fx("title");
    let s = seat(&f, "worker", Some("claude"));
    publish(&s, "\x1b]0;\u{25d0} Claude Code\x07".as_bytes());
    let mut frame = Frame::new();
    let cap = frame.title.capacity();
    assert!(matches!(collect(&s, &mut frame), Collected::Frame));
    assert_eq!(frame.title, "\u{25d0} Claude Code");
    let long = format!("\x1b]0;{}\x07", "\u{25d0}".repeat(400)); // 1,200 bytes
    publish(&s, long.as_bytes());
    assert!(matches!(collect(&s, &mut frame), Collected::Frame));
    assert!(frame.title.len() <= TITLE_CAP && frame.title.len() > TITLE_CAP - 4, "truncated at a char boundary");
    assert_eq!(frame.title.capacity(), cap, "never grown under the parser lock");
}

/// U4-1: an open synchronized-output block is a frame mid-update → transient incomplete, Working retained.
#[test]
fn open_synchronized_output_block_is_an_incomplete_frame() {
    let f = fx("sync-open");
    let s = seat(&f, "worker", Some("claude"));
    let mut frame = Frame::new();
    drive_working(&f, &s, &mut frame);
    publish(&s, b"\x1b[?2026h\x1b[1;1HSYN-CLAUDE-WORKING half");
    observe(&f, &s, &mut frame, 3.5);
    let w = wire_at(&f.daemon.activity, &s, 3.5, &P);
    assert_eq!((w["state"].as_str(), w["reason"].as_str(), w["retained"].as_bool()), (Some("working"), Some("incomplete frame"), Some(true)), "{w}");
    publish(&s, b" done\x1b[?2026l");
    observe(&f, &s, &mut frame, 4.0);
    let w = wire_at(&f.daemon.activity, &s, 4.0, &P);
    assert_eq!((w["state"].as_str(), w["retained"].as_bool()), (Some("working"), Some(false)), "{w}");
}
