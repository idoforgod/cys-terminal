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
    let v = View { rows: &rows, cursor: (0, 0), title: "" };
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
fn production_registry_has_corpus_grammars_only_and_none_for_gemini() {
    let fams: Vec<Family> = grammars().iter().map(|g| g.family()).collect();
    assert_eq!(fams, vec![Family::Claude, Family::Codex]);
    // gemini/agy: no captured working frame yet (live-seat observation only) → Unknown, never a fabricated grammar.
    let rows = vec![(0u16, "> ".to_string())];
    let v = View { rows: &rows, cursor: (0, 0), title: "anything" };
    assert_eq!(classify_with(Family::Gemini, &v, grammars()).0, Sample::Semantic(Semantic::UnsupportedFamily));
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
    // Seat ids restart at 1 per daemon: a new fixture starts the own-bump ledger empty (see `OWN_GEN`).
    OWN_GEN.with(|m| m.borrow_mut().clear());
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
    bump_gen(s.id, &s.output_gen, 1);
    s.parser.lock().unwrap().process(bytes);
    bump_gen(s.id, &s.output_gen, 1);
}

fn set_after_copy(f: Option<Box<dyn FnMut()>>) {
    AFTER_COPY.with(|h| *h.borrow_mut() = f);
}

// Windows seat output (windows-health run 39207028554). `seat()` is a real PTY seat running `sleep 30`: on Unix it prints
// nothing; on Windows it prints right after it starts (ConPTY start-up output), and its reader thread publishes those
// chunks into the same parser and `output_gen` these tests write (state.rs reader: odd, parser lock, even). A step that
// runs during such a chunk meets a held parser lock or an odd endpoint, which the observer rightly rejects, and then
// measures the seat instead of the observer. Those steps run in `quiet`: an attempt counts only if the seat's reader
// published nothing during it. That is the proof of `state::test_until_no_pty_output` (its core, 60 tries, 25 ms step)
// with one difference: the generation read leaves out this test's own bumps, because `publish()` moves `output_gen` by 2
// per frame by design and a window with a publication in it would otherwise never be quiet.

thread_local! {
    /// `output_gen` bumps made by this test thread, per seat id (`publish()` and the endpoint hooks). Cleared by `fx()`: seat
    /// ids restart per daemon, so an entry from an earlier fixture on this thread (libtest gives each test its own thread,
    /// and a panic mid-`publish` ends its test) can never be read against a later seat with the same id.
    static OWN_GEN: std::cell::RefCell<std::collections::HashMap<u64, u64>> = Default::default();
}

/// Moves a seat's output generation as this test, not as the seat's reader.
fn bump_gen(sid: u64, gen: &std::sync::atomic::AtomicU64, n: u64) {
    gen.fetch_add(n, Ordering::AcqRel);
    OWN_GEN.with(|m| *m.borrow_mut().entry(sid).or_insert(0) += n);
}

/// The seat's output generation without this test's bumps: only the seat's reader moves it.
fn reader_gen(s: &Surface) -> u64 {
    let own = OWN_GEN.with(|m| m.borrow().get(&s.id).copied().unwrap_or(0));
    s.output_gen.load(Ordering::Acquire) - own
}

/// Runs `attempt` until one run saw no publication by the seat's reader and returns that run's value. Assertions go on
/// the returned value, never inside `attempt` (a disturbed run's values are dropped). A silent (Unix) seat keeps the
/// first run. A run that moves `output_gen` by itself other than through `bump_gen` never counts, so it panics here.
fn quiet<T>(s: &Surface, what: &str, attempt: impl FnMut() -> T) -> T {
    match crate::state::test_until_quiet_window(|| reader_gen(s), 60, 25, attempt) {
        Ok(v) => v,
        Err(tries) => panic!(
            "{what}: premise not met - output_gen moved outside this test's bump_gen in all {tries} attempts \
             (the seat keeps printing, or the code under test moved it)"
        ),
    }
}

/// `quiet` for runs that feed the seat's reducer. A retried run first writes the identity, so it starts on a new epoch
/// with a fresh reducer (`observe_seat`) and a disturbed run's rejected samples are not carried into it. The first run
/// is untouched.
fn quiet_run<T>(s: &Surface, what: &str, mut run: impl FnMut() -> T) -> T {
    let mut retried = false;
    quiet(s, what, || {
        if std::mem::replace(&mut retried, true) {
            invalidate_identity(s);
        }
        run()
    })
}

#[test]
fn collect_copies_bounded_rows_without_growing_its_buffers() {
    let f = fx("collect-bounds");
    let s = seat(&f, "worker", Some("claude"));
    let mut frame = Frame::new();
    let (cap_cells, cap_rows) = (frame.cells.capacity(), frame.row_index.capacity());
    let got = quiet(&s, "collect bounds", || {
        publish(&s, b"\x1b[2J\x1b[24;1Hbottom line\x1b[1;1Htop");
        matches!(collect(&s, &mut frame), Collected::Frame)
    });
    assert!(got);
    assert_eq!(frame.cells.len(), frame.row_index.len() * 80);
    assert!(frame.row_index.len() <= MAX_ROWS && frame.cells.len() <= MAX_CELLS);
    assert_eq!((frame.cells.capacity(), frame.row_index.capacity()), (cap_cells, cap_rows), "no reallocation");
    let text = frame.text_rows();
    assert!(text.iter().any(|(r, l)| *r == 23 && l.starts_with("bottom line")));
    assert!(text.iter().any(|(r, l)| *r == 0 && l.starts_with("top")), "cursor row ±3 copied");
}

/// U4-3 hold mitigation: the row-slice copy is byte-for-byte the old per-cell copy (`screen.cell(r, c)` for every cell, Debug
/// form so the comparison-masked dim bit is compared too), and the byte-slice title equals the old char-by-char truncation.
#[test]
fn copy_window_row_slices_equal_the_per_cell_copy() {
    fn per_cell(screen: &vt100::Screen, rows_idx: &[u16], cols: u16) -> Vec<String> {
        let mut v = Vec::new();
        for &r in rows_idx {
            for c in 0..cols {
                v.push(format!("{:?}", screen.cell(r, c).cloned().unwrap_or_default()));
            }
        }
        v
    }
    fn title_by_chars(t: &str) -> String {
        let mut s = String::new();
        for ch in t.chars() {
            if s.len() + ch.len_utf8() > TITLE_CAP {
                break;
            }
            s.push(ch);
        }
        s
    }
    let mut numbered = String::new();
    for i in 1..=300 {
        numbered.push_str(&format!("\x1b[{}mline {i:04} 가나다 😀 \x1b[2mdim\x1b[0m\r\n", 31 + i % 7));
    }
    let long_title = format!("\x1b]0;{}\x07", "\u{25d0}a".repeat(150)); // 600 bytes, multibyte at the cut
    let wide_edge = format!("{}가\r\n{}", "x".repeat(47), "\x1b[5;10Hcursor");
    let cases: Vec<(u16, u16, String)> = vec![
        (24, 80, format!("{numbered}\x1b[12;5H{long_title}")),
        (40, 120, format!("\x1b[?1049h\x1b[2J{numbered}\x1b[40;1H› composer\x1b]0;\u{2819} work\x07")),
        (30, 256, format!("{}\r\n{numbered}\x1b]0;\u{2733} Claude Code\x07", "w".repeat(700))),
        (24, 48, format!("{numbered}{wide_edge}")),
    ];
    for (rows, cols, bytes) in cases {
        let mut p = vt100::Parser::new(rows, cols, 1_000);
        p.process(bytes.as_bytes());
        // window rows whose LAST column is not blank (a dropped last cell must not hide behind default padding):
        // a full-width bottom row and, above it, a row ending in a wide character exactly at the right edge.
        let edge = format!("\x1b[{};1H{}가\x1b[{rows};1H{}", rows - 1, "y".repeat(usize::from(cols) - 2), "Z".repeat(usize::from(cols)));
        p.process(edge.as_bytes());
        let mut frame = Frame::new();
        assert!(copy_window(p.screen(), &mut frame), "{rows}x{cols}: supported geometry");
        let want = per_cell(p.screen(), &frame.row_index, cols);
        let got: Vec<String> = frame.cells.iter().map(|c| format!("{c:?}")).collect();
        assert_eq!(got.len(), want.len(), "{rows}x{cols}: cell count");
        let first_diff = got.iter().zip(&want).position(|(a, b)| a != b);
        assert_eq!(first_diff, None, "{rows}x{cols}: first differing cell index");
        assert_eq!(frame.title, title_by_chars(p.screen().title()), "{rows}x{cols}: title");
    }
}

/// U4-3 hold mitigation, information only (not a gate): no-contention timing of the old per-cell copy vs the row-slice
/// `copy_window` on the same parser (10,000 lines of scrollback). `CYS_HOLD_BENCH_OUT=<json>` receives the numbers.
#[test]
#[ignore = "micro-benchmark; run with --release and CYS_HOLD_BENCH_OUT"]
fn hold_copy_micro_benchmark() {
    fn pct(v: &mut [u64], q: f64) -> u64 {
        v.sort_unstable();
        v[((v.len() as f64 * q).ceil() as usize).clamp(1, v.len()) - 1]
    }
    let mut out = serde_json::Map::new();
    for (rows, cols) in [(24u16, 80u16), (40, 120), (60, 256)] {
        let mut p = vt100::Parser::new(rows, cols, crate::state::SCROLLBACK_LINES);
        for i in 0..12_000 {
            p.process(format!("\x1b[3{}mline {i:05} {} 가나 😀\x1b[0m\r\n", i % 7, "w".repeat(usize::from(cols) / 2)).as_bytes());
        }
        p.process(b"\x1b]0;\xe2\x97\x90 Claude Code\x07");
        let screen = p.screen();
        let mut frame = Frame::new();
        let mut reference: Vec<vt100::Cell> = Vec::with_capacity(MAX_CELLS);
        let (mut old_ns, mut new_ns) = (Vec::with_capacity(20_000), Vec::with_capacity(20_000));
        for _ in 0..20_000 {
            let t = std::time::Instant::now();
            assert!(copy_window(screen, &mut frame));
            new_ns.push(t.elapsed().as_nanos() as u64);
            let t = std::time::Instant::now();
            reference.clear();
            for &r in &frame.row_index {
                for c in 0..cols {
                    reference.push(screen.cell(r, c).cloned().unwrap_or_default());
                }
            }
            old_ns.push(t.elapsed().as_nanos() as u64);
            std::hint::black_box(&reference);
        }
        let key = format!("{rows}x{cols}");
        let stats = |v: &mut Vec<u64>| json!({"p50_ns": pct(v, 0.50), "p99_ns": pct(v, 0.99), "max_ns": pct(v, 1.0)});
        let (o, n) = (stats(&mut old_ns), stats(&mut new_ns));
        eprintln!("{key}: per-cell {o} | row-slice {n} | cells {}", frame.cells.len());
        out.insert(key, json!({"cells": frame.cells.len(), "per_cell_copy": o, "row_slice_copy_window": n}));
    }
    if let Ok(path) = std::env::var("CYS_HOLD_BENCH_OUT") {
        std::fs::write(path, serde_json::to_vec_pretty(&Value::Object(out)).unwrap()).unwrap();
    }
}

/// R3-1 deterministic interleaving: copy at an even generation, a whole publication completes before
/// the second endpoint (both even, unequal) — published-only admission MUST accept. Equality-requiring
/// collection would reject this and fail here.
#[test]
fn published_only_admission_accepts_even_unequal_endpoints() {
    let f = fx("even-unequal");
    let s = seat(&f, "worker", Some("claude"));
    let gen = Arc::clone(&s.output_gen);
    let sid = s.id;
    let mut frame = Frame::new();
    let mut r = Reducer::new(0.0);
    for tick in 0..4u64 {
        // the frame is published inside each tick's quiet run, so no seat chunk can repaint it between ticks
        let (before, after, got) = quiet(&s, "even/even unequal", || {
            publish(&s, b"\x1b[1;1HSYN-CLAUDE-WORKING frame");
            let g2 = Arc::clone(&gen);
            set_after_copy(Some(Box::new(move || {
                bump_gen(sid, &g2, 2); // a complete reader publication between the endpoints
            })));
            let before = s.output_gen.load(Ordering::Acquire);
            let got = matches!(collect(&s, &mut frame), Collected::Frame);
            set_after_copy(None);
            (before, s.output_gen.load(Ordering::Acquire), got)
        });
        assert_ne!(after, before, "endpoints differed");
        assert!(got, "tick {tick}: even/even unequal must be admitted");
        let text = frame.text_rows();
        let v = View { rows: &text, cursor: frame.cursor, title: &frame.title };
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
    let sid = s.id;
    let mut frame = Frame::new();
    let first = quiet(&s, "odd first endpoint", || {
        bump_gen(sid, &s.output_gen, 1); // publication in progress at the first endpoint
        let got = matches!(collect(&s, &mut frame), Collected::Rejected(Sample::Transient(Transient::OddEndpoint)));
        bump_gen(sid, &s.output_gen, 1);
        got
    });
    assert!(first);
    let second = quiet(&s, "odd second endpoint", || {
        let g2 = Arc::clone(&s.output_gen);
        set_after_copy(Some(Box::new(move || {
            bump_gen(sid, &g2, 1); // second endpoint odd
        })));
        let got = matches!(collect(&s, &mut frame), Collected::Rejected(Sample::Transient(Transient::OddEndpoint)));
        set_after_copy(None);
        bump_gen(sid, &s.output_gen, 1); // even again, so a retried run starts from an even first endpoint
        got
    });
    assert!(second);
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
    let got = quiet(&s, "poisoned parser", || {
        matches!(collect(&s, &mut frame), Collected::Rejected(Sample::Semantic(Semantic::ParserPoison)))
    });
    assert!(got);
    assert!(s.parser.is_poisoned(), "the display collector does not clear poison");
}

#[test]
fn over_256_columns_is_unsupported_geometry() {
    let f = fx("wide");
    let s = seat(&f, "worker", Some("claude"));
    s.parser.lock().unwrap().set_size(24, 300);
    let mut frame = Frame::new();
    let got = quiet(&s, "wide geometry", || {
        matches!(collect(&s, &mut frame), Collected::Rejected(Sample::Semantic(Semantic::UnsupportedGeometry)))
    });
    assert!(got);
    assert!(frame.cells.is_empty());
}

#[test]
fn identity_write_between_endpoints_invalidates_the_sample() {
    let f = fx("ident-between");
    let s = seat(&f, "worker", Some("claude"));
    let mut frame = Frame::new();
    let got = quiet(&s, "identity write between endpoints", || {
        let s2 = Arc::clone(&s);
        set_after_copy(Some(Box::new(move || invalidate_identity(&s2))));
        let got = matches!(collect(&s, &mut frame), Collected::Rejected(Sample::Invalidated(Invalid::Identity)));
        set_after_copy(None);
        got
    });
    assert!(got);
    // a writer still in progress also rejects (before any endpoint read: no seat output can change this one)
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

/// Drives a seat to Working with four distinct active frames at t=0..3 (the four steps are one `quiet_run`).
fn drive_working(f: &Fx, s: &Surface, frame: &mut Frame) {
    let w = quiet_run(s, "drive_working", || drive(f, s, frame));
    assert_eq!(w["state"], "working", "precondition: {w}");
}

/// The four steps of `drive_working` and the wire at t=3, unchecked — for `quiet_run`s that continue after the dwell
/// (the caller asserts the precondition on the returned wire).
fn drive(f: &Fx, s: &Surface, frame: &mut Frame) -> Value {
    for t in 0..4 {
        publish(s, format!("\x1b[1;1HSYN-CLAUDE-WORKING tick {t}   ").as_bytes());
        observe(f, s, frame, t as f64);
    }
    wire_at(&f.daemon.activity, s, 3.0, &P)
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
    let (pre, at_once, w) = quiet_run(&s, "same-size resize", || {
        let pre = drive(&f, &s, &mut frame);
        {
            let mut p = s.parser.lock().unwrap();
            bump_view(&s); // what repaint::set_parser_size does under the lock
            p.set_size(24, 80);
        }
        let at_once = wire_at(&f.daemon.activity, &s, 3.5, &P)["reason"].clone();
        publish(&s, b"\x1b[1;1HSYN-CLAUDE-WORKING after resize");
        observe(&f, &s, &mut frame, 4.0);
        (pre, at_once, wire_at(&f.daemon.activity, &s, 4.0, &P))
    });
    assert_eq!(pre["state"], "working", "precondition: {pre}");
    assert_eq!(at_once, "view changed", "serializer sees it at once");
    assert_eq!((w["state"].as_str(), w["reason"].as_str()), (Some("unknown"), Some("view changed")), "{w}");
}

#[test]
fn reset_and_repaint_between_samples_is_not_continuity() {
    let f = fx("ris");
    let s = seat(&f, "worker", Some("claude"));
    let mut frame = Frame::new();
    let (pre, resets, w) = quiet_run(&s, "reset + repaint", || {
        let pre = drive(&f, &s, &mut frame);
        let r0 = s.parser.lock().unwrap().screen().reset_count();
        // ESC c then an identical repaint: geometry and final frame match, but it is a new view.
        publish(&s, b"\x1bc\x1b[1;1HSYN-CLAUDE-WORKING tick 3   ");
        let resets = s.parser.lock().unwrap().screen().reset_count() - r0;
        observe(&f, &s, &mut frame, 4.0);
        (pre, resets, wire_at(&f.daemon.activity, &s, 4.0, &P))
    });
    assert_eq!(pre["state"], "working", "precondition: {pre}");
    // counted per run (a fresh seat starts at 0, so the first run reads 1 as before; a retried run adds its own reset)
    assert_eq!(resets, 1);
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
    let (pre, during, after) = quiet_run(&s, "quiescing window", || {
        *s.agent_status.lock().unwrap() = None; // the seat as created (a retried run starts here again)
        let pre = drive(&f, &s, &mut frame);
        *s.agent_status.lock().unwrap() = Some(crate::state::AgentStatus {
            state: "quiescing".into(),
            context_pct: None,
            task: None,
            updated_at: crate::state::now_epoch(),
        });
        publish(&s, b"\x1b[1;1HSYN-CLAUDE-WORKING during clear");
        observe(&f, &s, &mut frame, 4.0);
        let during = wire_at(&f.daemon.activity, &s, 4.0, &P)["reason"].clone();
        // an orphaned window older than the default hold no longer suppresses
        s.agent_status.lock().unwrap().as_mut().unwrap().updated_at = crate::state::now_epoch() - 601.0;
        publish(&s, b"\x1b[1;1HSYN-CLAUDE-WORKING after window");
        observe(&f, &s, &mut frame, 5.0);
        (pre, during, wire_at(&f.daemon.activity, &s, 5.0, &P)["reason"].clone())
    });
    assert_eq!(pre["state"], "working", "precondition: {pre}");
    assert_eq!(during, "reset pending");
    assert_eq!(after, "active candidate");
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

/// Head-ruled U/S merge invariant through the actual set_meta handler.
#[test]
fn set_meta_publishes_epochs_before_identity_revision() {
    // Head's dry-run resolution: the epoch mutation stays inside IdentityWrite.
    // Pin the production ordering as well, so the reversed tiny window is a
    // deterministic red even when a concurrent sampler misses that interleaving.
    let source = include_str!("handlers.rs");
    let arm = source.split_once("\"surface.set_meta\" => {").unwrap().1;
    let begin = arm.find("let display_write = crate::activity::IdentityWrite::begin(&surface);").unwrap();
    let bump = arm.find("surface.bump_agent_epoch();").unwrap();
    let end = arm.find("drop(display_write);").unwrap();
    assert!(begin < bump && bump < end, "epoch update must precede identity publication");

    let f = fx("integ-meta-order");
    let s = seat(&f, "worker", Some("claude"));
    let rev0 = s.activity_identity_rev.load(Ordering::SeqCst);
    let agent0 = s.agent_epoch.load(Ordering::Acquire);
    let conversation0 = s.conversation_epoch.load(Ordering::Acquire);
    let pending0 = s.activity_pending.load(Ordering::SeqCst);
    let sample = || {
        let rev = s.activity_identity_rev.load(Ordering::SeqCst);
        let agent = s.agent_epoch.load(Ordering::Acquire);
        let conversation = s.conversation_epoch.load(Ordering::Acquire);
        if rev > rev0 {
            assert!(agent > agent0 && conversation > conversation0,
                    "new identity revision with old epochs: {rev}/{agent}/{conversation}");
        }
        rev
    };
    assert_eq!(sample(), rev0);
    // Hold the actual handler at its metadata write after IdentityWrite begins.
    let metadata = s.agent_meta.lock().unwrap();
    let daemon = Arc::clone(&f.daemon);
    let sid = s.id;
    let writer = std::thread::spawn(move || {
        crate::delivery::tests::isolate_state_dir_for_thread("integ-meta-writer");
        dispatch(&daemon, Request { id: json!(1), method: "surface.set_meta".into(),
            params: json!({"surface_id": sid, "agent": "claude", "agent_bin": "claude"}) }, None)
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while s.activity_pending.load(Ordering::SeqCst) == pending0 {
        assert!(std::time::Instant::now() < deadline, "writer did not enter identity window");
        std::thread::yield_now();
    }
    assert_eq!(sample(), rev0, "metadata lock holds the writer before epoch changes");
    drop(metadata);
    while !writer.is_finished() {
        sample();
        assert!(std::time::Instant::now() < deadline, "set_meta did not complete");
        std::thread::yield_now();
    }
    let Reply::Single(reply) = writer.join().unwrap() else { panic!("single reply required") };
    assert!(reply.get("error").is_none(), "{reply}");
    assert_eq!(sample(), rev0 + 1);
    assert_eq!(s.agent_epoch.load(Ordering::Acquire), agent0 + 1);
    assert_eq!(s.conversation_epoch.load(Ordering::Acquire), conversation0 + 1);
    assert_eq!(s.activity_pending.load(Ordering::SeqCst), pending0);
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
    let (pre, w1, w2, w3) = quiet_run(&s, "epoch", || {
        let pre = drive(&f, &s, &mut frame);
        let w1 = wire_at(&f.daemon.activity, &s, 3.0, &P);
        publish(&s, b"\x1b[1;1HSYN-CLAUDE-WORKING more");
        observe(&f, &s, &mut frame, 4.0);
        let w2 = wire_at(&f.daemon.activity, &s, 4.0, &P);
        invalidate_identity(&s);
        observe(&f, &s, &mut frame, 5.0);
        (pre, w1, w2, wire_at(&f.daemon.activity, &s, 5.0, &P))
    });
    assert_eq!(pre["state"], "working", "precondition: {pre}");
    assert_eq!(w1["display_epoch"], w2["display_epoch"]);
    assert!(w2["observation_seq"].as_u64() > w1["observation_seq"].as_u64());
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
    let fields = || {
        (
            s.agent_status.lock().unwrap().as_ref().map(|a| (a.state.clone(), a.context_pct, a.task.clone(), a.updated_at)),
            s.pending_queue.lock().unwrap().len(),
            s.input_gen.load(Ordering::Acquire),
            s.pending_input_bytes.load(Ordering::Relaxed),
        )
    };
    // `output_gen` has one other writer here: the seat's reader (a Windows seat prints at start), which bumps it twice per
    // chunk and sends every chunk on `out_tx` inside that bracket. So the eight observations run once, against one
    // baseline, with no retry, and the reader's chunks are counted on `out_tx`: any other move of `output_gen` is the
    // observer's (a Unix `sleep 30` seat sends 0 chunks).
    let mut chunks = s.out_tx.subscribe();
    let (gen0, _, settled0) = settled_gen(&s, &mut chunks);
    assert!(settled0, "premise: the seat's reader never paused for 5 s");
    let before = fields();
    let mut frame = Frame::new();
    for t in 0..8 {
        observe(&f, &s, &mut frame, t as f64);
    }
    let after = fields();
    let (gen1, reader_chunks, settled1) = settled_gen(&s, &mut chunks);
    let foreign = gen1 as i128 - gen0 as i128 - 2 * reader_chunks as i128;
    assert_eq!(
        (before, 0),
        (after, foreign),
        "output_gen {gen0} -> {gen1}: {reader_chunks} reader chunk(s) explain {} of it (settled: {settled1})",
        2 * reader_chunks
    );
}

/// Every chunk the seat's reader sent on `out_tx` since the last call (a lagged receiver reports how many it skipped).
fn drain_chunks(rx: &mut tokio::sync::broadcast::Receiver<Vec<u8>>) -> u64 {
    use tokio::sync::broadcast::error::TryRecvError;
    let mut n = 0;
    loop {
        match rx.try_recv() {
            Ok(_) => n += 1,
            Err(TryRecvError::Lagged(k)) => n += k,
            Err(TryRecvError::Empty) | Err(TryRecvError::Closed) => return n,
        }
    }
}

/// Reads `output_gen` where no reader chunk is in flight and returns it with the chunks drained on the way.
///
/// The reader bumps `output_gen` (odd), sends the chunk on `out_tx`, then bumps it again (even). A read that is even, with
/// no chunk sent between the drains around it, therefore has every drained chunk's two bumps in it and none of a later
/// chunk's. Successive calls thus split the reader's bumps exactly at the reads: 2 per chunk drained between them.
/// An odd read is accepted only once the reader is idle across it — no chunk and no change over three 10 ms looks — so
/// an odd bump by someone else is counted exactly instead of being waited out (a reader stalled between its odd bump and
/// its send for 30 ms would show as a foreign bump: a red, never a green). Gives up after 5 s (`settled` false).
fn settled_gen(s: &Surface, rx: &mut tokio::sync::broadcast::Receiver<Vec<u8>>) -> (u64, u64, bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut drained = 0;
    loop {
        drained += drain_chunks(rx);
        let gen = s.output_gen.load(Ordering::Acquire);
        let late = drain_chunks(rx);
        drained += late;
        if late == 0 && gen % 2 == 0 {
            return (gen, drained, true);
        }
        if late == 0 {
            let mut idle = true;
            for _ in 0..3 {
                std::thread::sleep(std::time::Duration::from_millis(10));
                // every chunk drained here is counted: a dropped one would be charged to the observer
                let n = drain_chunks(rx);
                drained += n;
                if n != 0 || s.output_gen.load(Ordering::Acquire) != gen {
                    idle = false;
                    break;
                }
            }
            if idle {
                return (gen, drained, true);
            }
        }
        if std::time::Instant::now() >= deadline {
            return (gen, drained, false);
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
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
/// LIMIT (verification-device weakness, U gate R1 minor 10): the pin matches the NAMED lines only — a NEW identity writer
/// added elsewhere is invisible to it (an independent grep at 4aa679db found no unguarded writer; a later one needs a census).
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
    let mut frame = Frame::new();
    let cap = frame.title.capacity();
    // each title is published inside its quiet run: a Windows seat also sets the title (OSC) when it starts
    let got = quiet(&s, "title copy", || {
        publish(&s, "\x1b]0;\u{25d0} Claude Code\x07".as_bytes());
        matches!(collect(&s, &mut frame), Collected::Frame)
    });
    assert!(got);
    assert_eq!(frame.title, "\u{25d0} Claude Code");
    let long = format!("\x1b]0;{}\x07", "\u{25d0}".repeat(400)); // 1,200 bytes
    let got = quiet(&s, "long title copy", || {
        publish(&s, long.as_bytes());
        matches!(collect(&s, &mut frame), Collected::Frame)
    });
    assert!(got);
    assert!(frame.title.len() <= TITLE_CAP && frame.title.len() > TITLE_CAP - 4, "truncated at a char boundary");
    assert_eq!(frame.title.capacity(), cap, "never grown under the parser lock");
}

/// U4-1: an open synchronized-output block is a frame mid-update → transient incomplete, Working retained.
#[test]
fn open_synchronized_output_block_is_an_incomplete_frame() {
    let f = fx("sync-open");
    let s = seat(&f, "worker", Some("claude"));
    let mut frame = Frame::new();
    let (pre, half, done) = quiet_run(&s, "open sync block", || {
        let pre = drive(&f, &s, &mut frame);
        publish(&s, b"\x1b[?2026h\x1b[1;1HSYN-CLAUDE-WORKING half");
        observe(&f, &s, &mut frame, 3.5);
        let half = wire_at(&f.daemon.activity, &s, 3.5, &P);
        publish(&s, b" done\x1b[?2026l");
        observe(&f, &s, &mut frame, 4.0);
        (pre, half, wire_at(&f.daemon.activity, &s, 4.0, &P))
    });
    assert_eq!(pre["state"], "working", "precondition: {pre}");
    let w = half;
    assert_eq!((w["state"].as_str(), w["reason"].as_str(), w["retained"].as_bool()), (Some("working"), Some("incomplete frame"), Some(true)), "{w}");
    let w = done;
    assert_eq!((w["state"].as_str(), w["retained"].as_bool()), (Some("working"), Some(false)), "{w}");
}

// ---------------------------------------------------------------------------------------------
// Production grammars against frames transcribed from the native capture FINDINGS (rows, cursor, title)
// ---------------------------------------------------------------------------------------------

const RULE: &str = "────────────────────────────────────────────────────────────────────────────────";

fn frame_of(rows: &[(u16, &str)], cursor: (u16, u16), title: &str) -> (Vec<(u16, String)>, (u16, u16), String) {
    (rows.iter().map(|(i, l)| (*i, l.to_string())).collect(), cursor, title.to_string())
}

fn prod(fam: Family, f: &(Vec<(u16, String)>, (u16, u16), String), _cols: u16) -> Sample {
    // (`_cols`: the View no longer carries geometry — View.size/View.alt were never read by a grammar; R2 minor 11)
    let v = View { rows: &f.0, cursor: f.1, title: &f.2 };
    classify_with(fam, &v, grammars()).0
}

fn claude_idle(title: &str) -> (Vec<(u16, String)>, (u16, u16), String) {
    frame_of(&[(5, RULE), (6, "❯"), (7, RULE), (8, "  ⏵⏵ bypass permissions on (shift+tab to cycle) · ← for agents")], (6, 2), title)
}

#[test]
fn claude_idle_empty_is_waiting() {
    assert_eq!(prod(Family::Claude, &claude_idle("✳ Claude Code"), 80), Sample::Idle);
}

/// FINDINGS-claude §3: a multi-line draft quoting every busy token is still idle (title ✳). A token-only
/// grammar (search for "esc to interrupt"/"Working") would turn this into Working — this pins it.
#[test]
fn claude_draft_with_quoted_busy_tokens_is_never_active() {
    let f = frame_of(
        &[
            (5, RULE),
            (6, "❯ echo \"esc to interrupt\" Working Thinking (esc to interrupt)"),
            (7, "  second line Working"),
            (8, "  ✻ Thinking… (12s · esc to interrupt)"),
            (9, "  • Working (3s • esc to interrupt)"),
            (10, RULE),
            (11, "  ⏵⏵ bypass permissions on (shift+tab to cycle)"),
        ],
        (9, 35),
        "✳ Claude Code",
    );
    assert_eq!(prod(Family::Claude, &f, 80), Sample::Idle);
}

#[test]
fn claude_active_tool_and_streaming_and_narrow_width_are_active() {
    let tool = frame_of(
        &[
            (10, "  ⎿  Running… (12s)"),
            (12, "✶ Honking… (13s · ↓ 77 tokens)"),
            (14, RULE),
            (15, "❯"),
            (16, RULE),
            (17, "  ⏵⏵ bypass permissions on … · esc to interrupt · ← for ag…"),
        ],
        (15, 2),
        "◐ Claude Code",
    );
    assert!(matches!(prod(Family::Claude, &tool, 80), Sample::Active { .. }));
    // streaming: no spinner row at all; title alone carries the state
    let stream = frame_of(
        &[(11, "  juliet quebec, xray echo lima, sierra zulu golf, november uniform bravo."), (14, RULE), (15, "❯"), (16, RULE)],
        (15, 2),
        "◑ Claude Code",
    );
    assert!(matches!(prod(Family::Claude, &stream, 80), Sample::Active { .. }));
    // 48 columns: the footer token is truncated away (`· e…`); still active by title
    let narrow = frame_of(&[(14, "────────────────────────────────────────────────"), (15, "❯"), (16, "────────────────────────────────────────────────"), (17, "  ⏵⏵ bypass permissions on (shift+tab to  · e…")], (15, 2), "◐ Claude Code");
    assert!(matches!(prod(Family::Claude, &narrow, 48), Sample::Active { .. }));
}

#[test]
fn claude_permission_and_question_panels_are_waiting_for_input() {
    let perm = frame_of(
        &[(16, " Do you want to proceed?"), (17, " ❯ 1. Yes"), (18, "   2. Yes, and don’t ask again for: touch *"), (20, "   4. No"), (22, " Esc to cancel · Tab to amend")],
        (17, 1),
        "✳ Claude Code",
    );
    assert_eq!(prod(Family::Claude, &perm, 80), Sample::Decision(DecisionKind::Permission));
    let q = frame_of(&[(11, "F01 which option?"), (13, "❯ 1. Alpha"), (15, "  2. Beta"), (21, "Enter to select · ↑/↓ to navigate · Esc to cancel")], (13, 0), "✳ Claude Code");
    assert_eq!(prod(Family::Claude, &q, 80), Sample::Decision(DecisionKind::Question));
}

/// U4-1 on the real grammar: a working title over a frame whose composer is mid-redraw (rule row not yet
/// painted) is an incomplete frame — transient, not a contradiction.
#[test]
fn claude_working_title_over_a_half_painted_composer_is_incomplete() {
    let torn = frame_of(&[(14, RULE), (15, "❯"), (16, ""), (17, "  ⏵⏵ bypass permissions on (shift+tab to")], (15, 2), "◐ Claude Code");
    assert_eq!(prod(Family::Claude, &torn, 80), Sample::Transient(Transient::Incomplete));
}

fn codex_frame(title: &str, extra: &[(u16, &str)]) -> (Vec<(u16, String)>, (u16, u16), String) {
    let mut rows: Vec<(u16, &str)> = extra.to_vec();
    rows.extend_from_slice(&[(20, "› Ask Codex to do anything"), (22, "  GPT-6.1-Sol default · ~/work"), (23, "  ? for shortcuts")]);
    rows.sort_by_key(|r| r.0);
    frame_of(&rows, (20, 2), title)
}

#[test]
fn codex_idle_waiting_and_streaming_active_differ_only_by_title() {
    assert_eq!(prod(Family::Codex, &codex_frame("work", &[]), 80), Sample::Idle);
    // FINDINGS-codex §5: while streaming there is NO Working row and the bottom rows equal idle
    assert!(matches!(prod(Family::Codex, &codex_frame("⠹ work", &[(17, "• lima sierra zulu, …")]), 80), Sample::Active { .. }));
    assert!(matches!(prod(Family::Codex, &codex_frame("⠋ work", &[(17, "• Working (13s • esc to interrupt)")]), 80), Sample::Active { .. }));
}

#[test]
fn codex_draft_with_quoted_busy_tokens_is_never_active() {
    let f = frame_of(
        &[
            (17, "› echo \"esc to interrupt\" Working Thinking (esc to interrupt)"),
            (18, "  second line Working"),
            (19, "  ✻ Thinking… (12s · esc to interrupt)"),
            (20, "  • Working (3s • esc to interrupt)"),
        ],
        (17, 60),
        "work",
    );
    assert_eq!(prod(Family::Codex, &f, 80), Sample::Idle);
}

#[test]
fn codex_action_required_panels_are_waiting_for_input() {
    let perm = frame_of(&[(4, "  Would you like to run the following command?"), (17, "› 1. Yes, proceed (y)"), (23, "  Press enter to confirm or esc to cancel")], (23, 79), "[ ! ] Action Required | work");
    assert_eq!(prod(Family::Codex, &perm, 80), Sample::Decision(DecisionKind::Permission));
    let q = frame_of(&[(15, "  Question 1/1 (1 unanswered)"), (18, "  › 1. Alpha (Recommended)  first option"), (22, "  tab to add notes | enter to submit answer | esc to interrupt")], (23, 1), "[ . ] Action Required | work");
    assert_eq!(prod(Family::Codex, &q, 80), Sample::Decision(DecisionKind::Question));
}

/// Cross-family: a claude seat whose transcript shows a codex-like `›` row (not the cursor row) stays claude-active;
/// a codex frame is no claude signature and vice versa.
#[test]
fn families_do_not_cross_match() {
    let mut claude_busy = frame_of(&[(12, "› Ask Codex to do anything"), (14, RULE), (15, "❯"), (16, RULE)], (15, 2), "◐ Claude Code");
    assert!(matches!(prod(Family::Claude, &claude_busy, 80), Sample::Active { .. }));
    claude_busy.2 = "✳ Claude Code".into();
    assert_eq!(prod(Family::Claude, &claude_busy, 80), Sample::Idle);
    assert_eq!(prod(Family::Claude, &codex_frame("⠹ work", &[]), 80), Sample::Semantic(Semantic::OtherFamily));
    assert_eq!(prod(Family::Codex, &claude_idle("✳ Claude Code"), 80), Sample::Semantic(Semantic::OtherFamily));
}

/// U4-2 mask: the claude spinner/done glyph and the codex Working bullet are animation; timers are not.
#[test]
fn static_signature_masks_animation_glyphs_but_not_timers() {
    let at = |glyph: &str, secs: u32| {
        let row = format!("{glyph} Honking… ({secs}s · ↓ 77 tokens)");
        let f = frame_of(&[(12, row.as_str()), (14, RULE), (15, "❯"), (16, RULE)], (15, 2), "◐ Claude Code");
        match prod(Family::Claude, &f, 80) {
            Sample::Active { sig } => sig,
            other => panic!("{other:?}"),
        }
    };
    assert_eq!(at("✶", 13), at("✻", 13), "spinner glyph is neutral");
    assert_ne!(at("✶", 13), at("✶", 14), "the elapsed timer is progress");
    let cx = |bullet: &str| match prod(Family::Codex, &codex_frame("⠋ work", &[(17, &format!("{bullet} Working (1s • esc to interrupt)"))]), 80) {
        Sample::Active { sig } => sig,
        other => panic!("{other:?}"),
    };
    assert_eq!(cx("•"), cx("◦"));
}

// ---------------------------------------------------------------------------------------------
// F-01 native corpus replay (ignored: the corpus is evidence, not repo data).
//
//   CYS_F01_CORPUS=<dir holding INDEX.json>  CYS_F01_REPORT=<report.json>  cargo test --bin cysd \
//     activity_tests::corpus_replay -- --ignored
//
// Replays every capture's PTY output through a fresh vt100 parser of the capture's size and samples it
// through the SAME `copy_window` + `sample_frame` path the live observer uses. Ground truth comes from
// INDEX.json phase intervals (fake server / tool markers / harness actions — never screen text).
//   (a) per-read: a sample after every PTY read (the densest split-repaint exposure). U4-1 positive
//       control = an incomplete sample sandwiched between accepted active samples while Working; it must
//       never end Working. The static cap is disabled here (read cadence is not the sample cadence).
//   (b) 1 Hz: samples at offset + k·S for four offsets through the LOCKED reducer and the serializer's
//       lease, judged against the ground-truth phases.
// The report is written before any assertion so a red run still leaves its numbers.
// ---------------------------------------------------------------------------------------------

mod corpus_replay {
    use super::*;
    use base64::Engine as _;
    use sha2::{Digest, Sha256};
    use std::io::Read;
    use std::path::{Path, PathBuf};

    /// A sample taken within one sample period after an idle/decision boundary may still see the active
    /// frame (the CLI repaints after the ground-truth event). Fixed before the first replay run.
    const BOUNDARY_GRACE_MS: f64 = 1000.0;
    const OFFSETS_MS: [f64; 4] = [0.0, 250.0, 500.0, 750.0];
    /// dwell (3 s) + two sample periods: an active turn at least this long must establish Working.
    const ESTABLISH_MIN_TURN_MS: f64 = 5000.0;
    /// after establishment, the share of 1 Hz samples inside the turn that must read Working.
    const WORKING_FRAC_MIN: f64 = 0.95;
    /// A1 hand-disposition (head ruling HEAD-RULING-REPLAY-1 D1, after the RED REPLAY-1): in these captures the
    /// ground-truth "idle" that starts at the main stream's server close is a proxy error — the CLI's own screen kept
    /// declaring work (title frozen at the working glyph, footer `esc to interrupt`, SIGTERM ignored; FINDINGS-claude
    /// §7). Working there is tolerated only until the CLI's first resting title (`✳`) + BOUNDARY_GRACE_MS; any Working
    /// after that bound still fails. The disposition covers exactly the idle interval that begins where an
    /// `active:stream` interval ends.
    const A1_DISPOSED: [&str; 2] = ["claude/heavy-stream-80x24", "claude/heavy-stream-longhold-80x24"];

    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    enum Cat {
        Active,
        Idle,
        Decision,
        Excluded,
    }

    fn cat(phase: &str) -> Cat {
        if phase.starts_with("active:") {
            Cat::Active
        } else if phase.starts_with("idle") {
            Cat::Idle
        } else if phase == "permission" || phase == "question" {
            Cat::Decision
        } else {
            Cat::Excluded
        }
    }

    struct Gt {
        phase: String,
        start: f64,
        end: f64,
    }

    fn phase_at(gt: &[Gt], t: f64) -> Option<&Gt> {
        gt.iter().find(|g| t >= g.start && t < g.end)
    }

    fn cat_at(gt: &[Gt], t: f64) -> Cat {
        phase_at(gt, t).map_or(Cat::Excluded, |g| cat(&g.phase))
    }

    fn name(s: &Sample) -> String {
        match s {
            Sample::Active { .. } => "active".into(),
            Sample::Idle => "idle".into(),
            Sample::Decision(DecisionKind::Permission) => "permission".into(),
            Sample::Decision(DecisionKind::Question) => "question".into(),
            Sample::Transient(t) => format!("transient:{}", t.as_str()),
            Sample::Semantic(x) => format!("semantic:{}", x.as_str()),
            Sample::Invalidated(i) => format!("invalid:{}", i.as_str()),
        }
    }

    fn bump(m: &mut serde_json::Map<String, Value>, k: &str) {
        let n = m.get(k).and_then(Value::as_u64).unwrap_or(0) + 1;
        m.insert(k.to_string(), json!(n));
    }

    fn sample_at(parser: &vt100::Parser, frame: &mut Frame, fam: Family) -> Sample {
        if copy_window(parser.screen(), frame) {
            sample_frame(fam, frame, grammars()).0
        } else {
            Sample::Semantic(Semantic::UnsupportedGeometry)
        }
    }

    struct Capture {
        family: Family,
        id: String,
        rows: u16,
        cols: u16,
        gt: Vec<Gt>,
        /// (ms since h:spawn, bytes) for every PTY output record, file order.
        outputs: Vec<(f64, Vec<u8>)>,
        end_ms: f64,
        final_screen: Option<(f64, Vec<String>)>,
    }

    fn load(base: &Path, c: &Value, failures: &mut Vec<String>) -> Option<(Capture, Value)> {
        let id = format!("{}/{}", c["family"].as_str().unwrap_or("?"), c["capture"].as_str().unwrap_or("?"));
        let family = match c["family"].as_str() {
            Some("claude") => Family::Claude,
            Some("codex") => Family::Codex,
            Some("gemini") | Some("agy") => Family::Gemini,
            other => {
                failures.push(format!("{id}: unknown family {other:?}"));
                return None;
            }
        };
        let path: PathBuf = base.join(c["file"].as_str().unwrap_or(""));
        let raw = std::fs::read(&path).unwrap_or_else(|e| panic!("{id}: read {}: {e}", path.display()));
        let sha = format!("{:x}", Sha256::digest(&raw));
        let mut prov = json!({"file_sha256": sha, "index_sha256": c["sha256"], "sha_ok": c["sha256"].as_str() == Some(sha.as_str())});
        if prov["sha_ok"] != json!(true) {
            failures.push(format!("{id}: file sha256 {sha} != INDEX {}", c["sha256"]));
        }
        let text = if c["compressed"].as_str() == Some("gzip") {
            let mut out = Vec::new();
            flate2::read::GzDecoder::new(&raw[..]).read_to_end(&mut out).unwrap_or_else(|e| panic!("{id}: gunzip: {e}"));
            let usha = format!("{:x}", Sha256::digest(&out));
            let ok = c["uncompressed_sha256"].as_str() == Some(usha.as_str());
            prov["uncompressed_sha_ok"] = json!(ok);
            if !ok {
                failures.push(format!("{id}: uncompressed sha256 {usha} != INDEX {}", c["uncompressed_sha256"]));
            }
            out
        } else {
            raw
        };
        let text = String::from_utf8(text).unwrap_or_else(|e| panic!("{id}: utf8: {e}"));
        let mut spawn_t: Option<f64> = None;
        let mut outputs = Vec::new();
        let mut end_ms: f64 = 0.0;
        let mut final_screen = None;
        for (n, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let r: Value = serde_json::from_str(line).unwrap_or_else(|e| panic!("{id}: line {}: {e}", n + 1));
            let Some(t) = r["t"].as_f64() else { continue };
            if spawn_t.is_none() && r["s"] == "h" && r["ev"] == "spawn" {
                spawn_t = Some(t);
            }
            let Some(t0) = spawn_t else { continue };
            let ms = (t - t0) / 1e6;
            end_ms = end_ms.max(ms);
            match r["s"].as_str() {
                Some("o") => {
                    let b = base64::engine::general_purpose::STANDARD
                        .decode(r["b64"].as_str().unwrap_or(""))
                        .unwrap_or_else(|e| panic!("{id}: line {}: b64: {e}", n + 1));
                    outputs.push((ms, b));
                }
                Some("h") if r["ev"] == "final_screen" => {
                    let rows = r["rows"].as_array().map(|a| a.iter().map(|v| v.as_str().unwrap_or("").to_string()).collect());
                    final_screen = rows.map(|rows| (ms, rows));
                }
                _ => {}
            }
        }
        if spawn_t.is_none() {
            failures.push(format!("{id}: no h:spawn record"));
            return None;
        }
        let gt = c["ground_truth_intervals"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|g| Gt {
                        phase: g["phase"].as_str().unwrap_or("").to_string(),
                        start: g["start_ms"].as_f64().unwrap_or(0.0),
                        end: g["end_ms"].as_f64().unwrap_or(0.0),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let rows = c["rows"].as_u64().unwrap_or(0) as u16;
        let cols = c["cols"].as_u64().unwrap_or(0) as u16;
        Some((Capture { family, id, rows, cols, gt, outputs, end_ms, final_screen }, prov))
    }

    /// A1: (interval start, first resting-title output time, tolerated-until bound) for a disposed capture.
    fn a1_window(cap: &Capture) -> Option<(f64, Option<f64>, f64)> {
        if !A1_DISPOSED.contains(&cap.id.as_str()) {
            return None;
        }
        let stream_end = cap.gt.iter().find(|g| g.phase == "active:stream")?.end;
        let idle = cap.gt.iter().find(|g| g.phase == "idle" && (g.start - stream_end).abs() < 1.0)?;
        let mut parser = vt100::Parser::new(cap.rows, cap.cols, crate::state::SCROLLBACK_LINES);
        let mut resting = None;
        for (t, b) in &cap.outputs {
            parser.process(b);
            if *t >= idle.start && parser.screen().title().trim_start().starts_with('✳') {
                resting = Some(*t);
                break;
            }
        }
        // no resting title found → no tolerance at all (the bound collapses to the interval start).
        let bound = resting.map_or(idle.start, |r| r + BOUNDARY_GRACE_MS);
        Some((idle.start, resting, bound))
    }

    /// A2: the frame ITSELF is incomplete — an open ?2026 block, or the family's own grammar returns NoMatch (raw
    /// `Grammar::classify`, bypassing the classify_with / sample_frame mapping U4-1 governs) with no other family's
    /// complete signature. A mutant of that mapping cannot hide these frames from the positive control.
    fn frame_incomplete(frame: &Frame, fam: Family) -> bool {
        if frame.sync {
            return true;
        }
        let text = frame.text_rows();
        let v = View { rows: &text, cursor: frame.cursor, title: &frame.title };
        let complete = |g: &&&dyn Grammar| matches!(g.classify(&v), Verdict::Active { .. } | Verdict::Idle | Verdict::Decision(_));
        let own_nomatch = grammars().iter().find(|g| g.family() == fam).is_some_and(|g| g.classify(&v) == Verdict::NoMatch);
        own_nomatch && !grammars().iter().filter(|g| g.family() != fam).any(|g| complete(&g))
    }

    /// (a) a sample after every read. Returns (report, sandwiches, held).
    fn per_read(cap: &Capture) -> (Value, u64, u64) {
        let no_cap = Params { static_n: u32::MAX, ..P };
        let mut parser = vt100::Parser::new(cap.rows, cap.cols, crate::state::SCROLLBACK_LINES);
        let mut frame = Frame::new();
        let mut red = Reducer::new(0.0);
        let mut seq: Vec<(f64, Sample, Effective, Effective, bool)> = Vec::with_capacity(cap.outputs.len());
        let mut by_phase = serde_json::Map::new();
        let mut fidelity = Value::Null;
        for (t, b) in &cap.outputs {
            if fidelity.is_null() {
                if let Some((ft, want)) = &cap.final_screen {
                    if t > ft {
                        fidelity = compare_screen(&parser, want);
                    }
                }
            }
            parser.process(b);
            let (s, raw_incomplete) = if copy_window(parser.screen(), &mut frame) {
                (sample_frame(cap.family, &frame, grammars()).0, frame_incomplete(&frame, cap.family))
            } else {
                (Sample::Semantic(Semantic::UnsupportedGeometry), false)
            };
            let before = red.effective;
            red.step(s, t / 1000.0, &no_cap);
            seq.push((*t, s, before, red.effective, raw_incomplete));
            let phase = phase_at(&cap.gt, *t).map_or("(none)".to_string(), |g| g.phase.clone());
            let entry = by_phase.entry(phase).or_insert_with(|| json!({}));
            bump(entry.as_object_mut().expect("object"), &name(&s));
        }
        if fidelity.is_null() {
            if let Some((_, want)) = &cap.final_screen {
                fidelity = compare_screen(&parser, want);
            }
        }
        let (mut sandwiches, mut held, mut run, mut max_run, mut active_reads, mut active_incomplete) = (0u64, 0u64, 0u64, 0u64, 0u64, 0u64);
        // A2 cross-check: frames the pipeline labelled Incomplete vs frames that ARE incomplete (equal on a correct tree).
        let (mut verdict_incomplete_frames, mut raw_incomplete_frames) = (0u64, 0u64);
        let mut examples = Vec::new();
        for i in 0..seq.len() {
            let (t, s, before, after, raw) = seq[i];
            verdict_incomplete_frames += u64::from(s == Sample::Transient(Transient::Incomplete));
            raw_incomplete_frames += u64::from(raw);
            let in_active = cat_at(&cap.gt, t) == Cat::Active;
            let incomplete = s == Sample::Transient(Transient::Incomplete);
            if in_active {
                active_reads += 1;
                if incomplete {
                    active_incomplete += 1;
                    run += 1;
                    max_run = max_run.max(run);
                } else {
                    run = 0;
                }
            } else {
                run = 0;
            }
            // A2: candidates are frames that ARE incomplete; the pipeline's verdict on them is what is being judged.
            if raw && in_active && i > 0 && i + 1 < seq.len() && before == Effective::Working {
                let prev_active = matches!(seq[i - 1].1, Sample::Active { .. });
                let next_active = matches!(seq[i + 1].1, Sample::Active { .. });
                if prev_active && next_active {
                    sandwiches += 1;
                    if after == Effective::Working {
                        held += 1;
                    } else if examples.len() < 5 {
                        examples.push(json!({"t_ms": t, "after": after.as_str(), "sample": name(&s)}));
                    }
                }
            }
        }
        let rate = if active_reads == 0 { 0.0 } else { active_incomplete as f64 / active_reads as f64 };
        (
            json!({
                "reads": seq.len(),
                "by_phase": by_phase,
                "active_reads": active_reads,
                "active_incomplete": active_incomplete,
                "active_incomplete_rate": rate,
                "max_consecutive_incomplete_in_active": max_run,
                "frames_labelled_incomplete": verdict_incomplete_frames,
                "frames_raw_incomplete": raw_incomplete_frames,
                "split_repaint_sandwiches": sandwiches,
                "sandwiches_working_held": held,
                "sandwich_drop_examples": examples,
                "final_screen_fidelity": fidelity,
            }),
            sandwiches,
            held,
        )
    }

    fn compare_screen(parser: &vt100::Parser, want: &[String]) -> Value {
        let (rows, cols) = parser.screen().size();
        let got: Vec<String> = parser.screen().rows(0, cols).take(usize::from(rows)).map(|l| l.trim_end().to_string()).collect();
        let n = got.len().min(want.len());
        let equal = (0..n).filter(|&i| got[i] == want[i].trim_end()).count();
        json!({"rows_compared": n, "rows_equal": equal, "row_count_parser": got.len(), "row_count_harness": want.len()})
    }

    /// Merges contiguous active ground-truth intervals into turns [start, end).
    fn turns(gt: &[Gt]) -> Vec<(f64, f64, Vec<String>)> {
        let mut out: Vec<(f64, f64, Vec<String>)> = Vec::new();
        for g in gt.iter().filter(|g| cat(&g.phase) == Cat::Active) {
            match out.last_mut() {
                Some(last) if (g.start - last.1).abs() < 1.0 => {
                    last.1 = g.end;
                    last.2.push(g.phase.clone());
                }
                _ => out.push((g.start, g.end, vec![g.phase.clone()])),
            }
        }
        out
    }

    /// (b) 1 Hz through the locked reducer and the serializer's lease.
    fn hz1(cap: &Capture, offset: f64, failures: &mut Vec<String>) -> Value {
        let period_ms = SAMPLE_PERIOD.as_secs_f64() * 1000.0;
        let mut parser = vt100::Parser::new(cap.rows, cap.cols, crate::state::SCROLLBACK_LINES);
        let mut frame = Frame::new();
        let mut red = Reducer::new(0.0);
        let mut next = 0usize;
        let mut samples: Vec<(f64, Sample, Effective)> = Vec::new();
        // independent static-run measurement with the reducer's counting rule (U4-2): same-sig active +1,
        // changed-sig active restarts at 1, idle/decision/semantic/invalidation reset, transient leaves it.
        let (mut st_sig, mut st_run, mut st_max, mut st_max_at) = (None::<u64>, 0u32, 0u32, 0.0f64);
        let mut st_max_rows: Vec<String> = Vec::new();
        let mut t = offset;
        while t <= cap.end_ms {
            while next < cap.outputs.len() && cap.outputs[next].0 <= t {
                parser.process(&cap.outputs[next].1);
                next += 1;
            }
            let s = sample_at(&parser, &mut frame, cap.family);
            red.step(s, t / 1000.0, &P);
            let (eff, _) = red.effective_at(t / 1000.0, &P);
            match s {
                Sample::Active { sig } => {
                    if st_sig == Some(sig) {
                        st_run += 1;
                    } else {
                        st_sig = Some(sig);
                        st_run = 1;
                    }
                    if st_run > st_max {
                        st_max = st_run;
                        st_max_at = t;
                        st_max_rows = frame.text_rows().into_iter().map(|(r, l)| format!("{r:>3}|{}", l.trim_end())).collect();
                        st_max_rows.insert(0, format!("title|{}", frame.title));
                    }
                }
                Sample::Transient(_) => {}
                _ => {
                    st_sig = None;
                    st_run = 0;
                }
            }
            samples.push((t, s, eff));
            t += period_ms;
        }
        let mut eff_by_cat = serde_json::Map::new();
        let mut false_working = Vec::new();
        let mut grace_working = 0u64;
        let a1 = a1_window(cap);
        let mut a1_working = 0u64;
        for (t, _, eff) in &samples {
            let c = cat_at(&cap.gt, *t);
            let entry = eff_by_cat.entry(format!("{c:?}")).or_insert_with(|| json!({}));
            bump(entry.as_object_mut().expect("object"), eff.as_str());
            if *eff == Effective::Working && matches!(c, Cat::Idle | Cat::Decision) {
                let g = phase_at(&cap.gt, *t).expect("categorised sample has a phase");
                if t - g.start < BOUNDARY_GRACE_MS {
                    grace_working += 1;
                } else if a1.is_some_and(|(start, _, bound)| (g.start - start).abs() < 1e-9 && *t < bound) {
                    a1_working += 1;
                } else {
                    false_working.push(json!({"t_ms": t, "phase": g.phase, "since_phase_start_ms": t - g.start}));
                }
            }
        }
        if !false_working.is_empty() {
            failures.push(format!("{} offset {offset}: {} false Working sample(s) in idle/decision phases: {:?}", cap.id, false_working.len(), false_working.iter().take(3).collect::<Vec<_>>()));
        }
        let mut turn_rows = Vec::new();
        for (s, e, phases) in turns(&cap.gt) {
            let inside: Vec<&(f64, Sample, Effective)> = samples.iter().filter(|(t, _, _)| *t >= s && *t < e).collect();
            let est = inside.iter().position(|(_, _, eff)| *eff == Effective::Working);
            let (frac, est_ms) = match est {
                Some(i) => {
                    let after = &inside[i..];
                    let w = after.iter().filter(|(_, _, eff)| *eff == Effective::Working).count();
                    (Some(w as f64 / after.len() as f64), Some(inside[i].0 - s))
                }
                None => (None, None),
            };
            let long = e - s >= ESTABLISH_MIN_TURN_MS;
            if long && est.is_none() {
                failures.push(format!("{} offset {offset}: active turn {s:.0}..{e:.0} ms ({:?}) never established Working", cap.id, phases));
            }
            if let Some(f) = frac {
                if f < WORKING_FRAC_MIN {
                    failures.push(format!("{} offset {offset}: active turn {s:.0}..{e:.0} ms Working share {f:.3} < {WORKING_FRAC_MIN}", cap.id));
                }
            }
            let reasons: Vec<String> = inside
                .iter()
                .skip(est.unwrap_or(inside.len()))
                .filter(|(_, _, eff)| *eff != Effective::Working)
                .map(|(t, smp, eff)| format!("{t:.0}:{}:{}", eff.as_str(), name(smp)))
                .take(5)
                .collect();
            turn_rows.push(json!({
                "start_ms": s, "end_ms": e, "phases": phases, "samples": inside.len(),
                "established_after_ms": est_ms, "working_share_after_establishment": frac,
                "non_working_after_establishment": reasons,
            }));
        }
        let has_active = cap.gt.iter().any(|g| cat(&g.phase) == Cat::Active);
        if has_active && st_max >= P.static_n {
            failures.push(format!("{} offset {offset}: identical active run {st_max} >= STATIC_ACTIVE_N {}", cap.id, P.static_n));
        }
        json!({
            "offset_ms": offset,
            "samples": samples.len(),
            "effective_by_category": eff_by_cat,
            "false_working": false_working,
            "working_within_boundary_grace": grace_working,
            "a1_disposition": a1.map(|(start, resting, bound)| json!({
                "interval_start_ms": start, "first_resting_title_ms": resting, "tolerated_until_ms": bound,
                "working_samples_disposed": a1_working,
            })),
            "turns": turn_rows,
            "max_identical_active_run": st_max,
            "max_identical_active_run_at_ms": st_max_at,
            "max_identical_active_run_phase": phase_at(&cap.gt, st_max_at).map(|g| g.phase.clone()),
            "max_identical_active_run_window": st_max_rows,
        })
    }

    #[test]
    #[ignore = "needs the F-01 native capture corpus (CYS_F01_CORPUS) and a report path (CYS_F01_REPORT)"]
    fn corpus_replay() {
        let corpus = PathBuf::from(std::env::var("CYS_F01_CORPUS").expect("CYS_F01_CORPUS (dir holding INDEX.json) not set"));
        let report_path = PathBuf::from(std::env::var("CYS_F01_REPORT").expect("CYS_F01_REPORT (report json path) not set"));
        let index_raw = std::fs::read(corpus.join("INDEX.json")).expect("read INDEX.json");
        let index_sha = format!("{:x}", Sha256::digest(&index_raw));
        let index: Value = serde_json::from_slice(&index_raw).expect("parse INDEX.json");
        // INDEX `file` entries are relative to the corpus directory's parent ("corpus/<family>/<capture>…").
        let base = corpus.parent().expect("corpus dir has a parent").to_path_buf();
        let mut failures: Vec<String> = Vec::new();
        let mut caps = Vec::new();
        let mut fam_totals: std::collections::BTreeMap<&'static str, (u64, u64, u64, u64)> = Default::default();
        for c in index["captures"].as_array().expect("captures array") {
            let Some((cap, prov)) = load(&base, c, &mut failures) else { continue };
            let (pr, sandwiches, held) = per_read(&cap);
            if pr["frames_labelled_incomplete"] != pr["frames_raw_incomplete"] {
                failures.push(format!("{}: A2 cross-check — {} frames labelled Incomplete but {} frames are incomplete", cap.id, pr["frames_labelled_incomplete"], pr["frames_raw_incomplete"]));
            }
            let tot = fam_totals.entry(cap.family.as_str()).or_default();
            tot.0 += 1;
            tot.1 += sandwiches;
            tot.2 += held;
            tot.3 += pr["active_incomplete"].as_u64().unwrap_or(0);
            let hz: Vec<Value> = OFFSETS_MS.iter().map(|&o| hz1(&cap, o, &mut failures)).collect();
            caps.push(json!({
                "family": cap.family.as_str(), "capture": cap.id, "size": [cap.cols, cap.rows],
                "provenance": prov, "outputs": cap.outputs.len(), "end_ms": cap.end_ms,
                "per_read": pr, "hz1": hz,
            }));
        }
        for fam in ["claude", "codex"] {
            let (n, s, h, _) = fam_totals.get(fam).copied().unwrap_or_default();
            if n == 0 {
                failures.push(format!("{fam}: no captures replayed"));
            } else if s == 0 {
                failures.push(format!("{fam}: U4-1 positive control not exercised — no incomplete sample sandwiched between active samples while Working"));
            } else if h != s {
                failures.push(format!("{fam}: U4-1 positive control red — {} of {s} split-repaint incomplete samples ended Working", s - h));
            }
        }
        let summary: serde_json::Map<String, Value> = fam_totals
            .iter()
            .map(|(f, (n, s, h, inc))| (f.to_string(), json!({"captures": n, "split_repaint_sandwiches": s, "held": h, "active_incomplete_reads": inc})))
            .collect();
        let report = json!({
            "corpus_index_sha256": index_sha,
            "grammars": grammars().iter().map(|g| g.revision()).collect::<Vec<_>>(),
            "params": {"sample_period_s": SAMPLE_PERIOD.as_secs_f64(), "dwell_s": P.dwell_secs, "dwell_min_samples": P.dwell_min_samples,
                        "max_gap_s": P.max_gap_secs, "k": P.k, "lease_s": P.lease_secs, "static_active_n": P.static_n},
            "criteria": {"boundary_grace_ms": BOUNDARY_GRACE_MS, "offsets_ms": OFFSETS_MS, "establish_min_turn_ms": ESTABLISH_MIN_TURN_MS,
                          "working_share_min": WORKING_FRAC_MIN,
                          "a1_disposed": A1_DISPOSED, "a1_ruling": "HEAD-RULING-REPLAY-1.md D1 (REPLAY-1 RED on record)"},
            "gemini": "no capture — family PROVISIONAL (live-seat read-only route, head ruling 2026-10-16)",
            "summary": summary,
            "failures": failures,
            "captures": caps,
        });
        if let Some(dir) = report_path.parent() {
            std::fs::create_dir_all(dir).expect("report dir");
        }
        std::fs::write(&report_path, serde_json::to_vec_pretty(&report).expect("serialize")).expect("write report");
        assert!(failures.is_empty(), "{} replay failure(s) (report {}):\n{}", failures.len(), report_path.display(), failures.join("\n"));
    }
}
