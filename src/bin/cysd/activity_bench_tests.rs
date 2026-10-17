//! F-01 U4-3 — lock cost and operational latency: measurement harness (test-only; no production code).
//!
//! Implements the locked plan `_evidence/dept5-0.14.52-P3-impl/U/validation/f01/u43/U43-LOCK-PLAN.md`:
//! 64 real PTY seats (32 claude / 32 codex agent_meta) replaying native corpus captures with their original
//! timing (`u43/replay_gen.py`), 4 idle "ready" seats fed one queued item every 7 s in rotation, the real
//! delivery tick (`deliver_queued` through the test seam) on fixed 5 s deadlines, an attach-snapshot probe at
//! 20/s, and the test-only sinks in `activity::bench` (observer hold / per-seat / per-tick, reader lock wait /
//! critical section, daemon → PTY writes). One arm per process; the arm comes from the environment:
//!
//! * `CYS_U43_ARM` = `off` (observer not started) | `on` (production observer) | `sleep` (observer + 250 ms
//!   test-only sleep after each seat's unlock, before classification)
//! * `CYS_U43_SECS` (default 1000) · `CYS_U43_SEED` (u64) · `CYS_U43_OUT` (result json path)
//! * `CYS_U43_CORPUS` (corpus dir holding INDEX.json) · `CYS_U43_REPLAY` (replay_gen.py) · `CYS_U43_PYTHON`
//! * optional, recorded only: `CYS_U43_LOAD_READING`, `CYS_U43_LOAD_ONE_MIN`, `CYS_U43_LOADED`,
//!   `CYS_U43_LOAD_WAITED_S`, `CYS_U43_ARM_ORDER`, `CYS_U43_ORDER_SEED`, `CYS_U43_BIN_SHA256`, `CYS_U43_ROUND`,
//!   `CYS_U43_ROUNDS`, `CYS_U43_GATE_MAX_S`
//!
//! Plan amendment A1 (U43-LOCK-PLAN-A1.md): A1-1 — each streaming seat's replay_gen reports the `ESC[6n` queries it
//! wrote (`--report`, generator side, independent of the daemon) and every daemon → PTY write is captured in full from
//! seat creation until the seats are killed, so the analyzer can classify DSR replies vs other bytes; A1-2 — rounds are
//! orchestrated by run-arms.sh (one arm per process). Diagnostic: each delivery tick records how many items it delivered.
//! Amendment A2 (U43-LOCK-PLAN-A2.md): A2-1 — every fsync reachable from `deliver_queued` is timed into the delivery
//! thread's own accumulator (`activity::bench::FsyncSpan`); each tick stores `fsync_ns` and `fsync_calls`.
//!
//! The JSON holds raw arrays per metric (no censoring); PASS/FAIL is computed by `u43/analyze.py`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::activity::bench;
use crate::state::{Daemon, HideConsole, Surface};

const STREAM_PER_FAMILY: usize = 32;
const READY_SEATS: usize = 4;
const ROWS: u16 = 24;
const COLS: u16 = 80;
const FEED_PERIOD: Duration = Duration::from_secs(7);
/// feed phase: 1.5 + 7k s is never closer than 0.5 s to a 5 s delivery deadline (residues mod 5 = 1.5, 3.5, 0.5, 2.5,
/// 4.5), so an enqueue never races a tick start (an integer phase collides every 35 s — smoke r1).
const FEED_FIRST: Duration = Duration::from_millis(1500);
/// no enqueue in the last 15 s of the window: 2 delivery ticks + the 2,000 ms settle budget fit before the end.
const FEED_STOP_BEFORE_END: Duration = Duration::from_secs(15);
const ATTACH_PERIOD: Duration = Duration::from_millis(50);
const SLEEP_KNOB_MS: u64 = 250;
const WARMUP_MAX: Duration = Duration::from_secs(15);
const DRAIN_MAX_TICKS: u32 = 3;
const CLAUDE_FILE: &str = "corpus/claude/active-stream-80x24.jsonl";
const CODEX_FILE: &str = "corpus/codex/active-stream-80x24.jsonl";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Arm {
    Off,
    On,
    Sleep,
}

impl Arm {
    fn as_str(self) -> &'static str {
        match self {
            Arm::Off => "off",
            Arm::On => "on",
            Arm::Sleep => "sleep",
        }
    }
}

fn env(k: &str) -> Option<String> {
    std::env::var(k).ok().filter(|v| !v.trim().is_empty())
}

fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn sha256_hex(b: &[u8]) -> String {
    format!("{:x}", Sha256::digest(b))
}

fn ns(d: Duration) -> u64 {
    d.as_nanos().min(u128::from(u64::MAX)) as u64
}

/// signed ns of `at` relative to the window origin (a sample may be stamped a few µs before it).
fn rel(at: Instant, w0: Instant) -> i64 {
    if at >= w0 {
        ns(at - w0) as i64
    } else {
        -(ns(w0 - at) as i64)
    }
}

fn sleep_until(deadline: Instant) {
    loop {
        let now = Instant::now();
        if now >= deadline {
            return;
        }
        std::thread::sleep(deadline - now);
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn cmd_out(prog: &str, args: &[&str]) -> Option<String> {
    let o = std::process::Command::new(prog).args(args).stdin(std::process::Stdio::null()).hide_console().output().ok()?;
    o.status.success().then(|| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

/// One verified capture: path, sha256, loop period (ns) and the INDEX row's sha256.
struct Capture {
    family: &'static str,
    path: PathBuf,
    sha256: String,
    index_sha256: String,
    period_ns: u64,
    outputs: usize,
}

fn verify_capture(base: &Path, index: &Value, family: &'static str, rel_file: &str) -> Capture {
    let row = index["captures"]
        .as_array()
        .expect("INDEX captures array")
        .iter()
        .find(|c| c["file"] == json!(rel_file))
        .unwrap_or_else(|| panic!("INDEX.json has no capture row for {rel_file}"));
    let index_sha256 = row["sha256"].as_str().expect("INDEX sha256").to_string();
    let path = base.join(rel_file);
    let raw = std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let sha256 = sha256_hex(&raw);
    assert_eq!(sha256, index_sha256, "corpus sha256 mismatch for {rel_file}: file {sha256} vs INDEX {index_sha256}");
    let (mut t0, mut last, mut outputs) = (None::<u64>, 0u64, 0usize);
    for line in raw.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
        let r: Value = serde_json::from_slice(line).expect("capture record json");
        if let Some(t) = r["t"].as_u64() {
            last = last.max(t);
            if r["s"] == json!("h") && r["ev"] == json!("spawn") && t0.is_none() {
                t0 = Some(t);
            }
        }
        if r["s"] == json!("o") {
            outputs += 1;
        }
    }
    let t0 = t0.expect("capture spawn record");
    Capture { family, path, sha256, index_sha256, period_ns: last - t0, outputs }
}

fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

struct Enq {
    id: String,
    sid: u64,
    at: Instant,
    epoch: f64,
}

struct Delivered {
    at: Instant,
    sid: Option<u64>,
    payload: Value,
}

struct TickRec {
    deadline: Instant,
    start: Instant,
    end: Instant,
    start_epoch: f64,
    end_epoch: f64,
    in_window: bool,
    /// A2-1: fsync time and calls made by this (delivery) thread inside the tick; 0 outside the window.
    fsync: Duration,
    fsync_calls: u64,
}

struct AttachRec {
    at: Instant,
    sid: u64,
    wait: Duration,
    total: Duration,
    bytes: usize,
}

fn rec_arrays(recs: &[bench::Rec], w0: Instant, a_key: &str, b_key: Option<&str>) -> Value {
    let mut m = serde_json::Map::new();
    m.insert("unit".into(), json!("ns"));
    m.insert("n".into(), json!(recs.len()));
    m.insert("t_ns".into(), json!(recs.iter().map(|r| rel(r.at, w0)).collect::<Vec<_>>()));
    m.insert("sid".into(), json!(recs.iter().map(|r| r.sid).collect::<Vec<_>>()));
    m.insert(a_key.into(), json!(recs.iter().map(|r| ns(r.a)).collect::<Vec<_>>()));
    if let Some(b) = b_key {
        m.insert(b.into(), json!(recs.iter().map(|r| ns(r.b)).collect::<Vec<_>>()));
    }
    Value::Object(m)
}

fn children_of_self() -> Vec<String> {
    let me = std::process::id().to_string();
    let Ok(child) = std::process::Command::new("/bin/ps")
        .args(["-axo", "pid=,ppid=,stat=,command="])
        .hide_console()
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .spawn()
    else {
        return vec!["<ps failed to start>".into()];
    };
    let ps_pid = child.id().to_string();
    let out = child.wait_with_output().map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default();
    out.lines()
        .filter(|l| {
            let mut it = l.split_whitespace();
            let pid = it.next().unwrap_or("");
            let ppid = it.next().unwrap_or("");
            ppid == me && pid != ps_pid
        })
        .map(|l| l.trim().to_string())
        .collect()
}

#[test]
#[ignore = "U4-3 measurement harness — run through u43/run-arms.sh (CYS_U43_ARM, CYS_U43_OUT, CYS_U43_CORPUS, CYS_U43_REPLAY)"]
fn u43_lock_cost_arm() {
    // ── configuration ────────────────────────────────────────────────────────────────────────
    let arm = match env("CYS_U43_ARM").as_deref() {
        Some("off") => Arm::Off,
        Some("on") => Arm::On,
        Some("sleep") => Arm::Sleep,
        other => panic!("CYS_U43_ARM must be off|on|sleep (got {other:?})"),
    };
    let secs: u64 = env("CYS_U43_SECS").map(|v| v.parse().expect("CYS_U43_SECS u64")).unwrap_or(1000);
    assert!(secs >= 20, "CYS_U43_SECS must be >= 20 (the feeder stops 15 s before the end)");
    let seed: u64 = env("CYS_U43_SEED").map(|v| v.parse().expect("CYS_U43_SEED u64")).expect("CYS_U43_SEED not set");
    let out_path = PathBuf::from(env("CYS_U43_OUT").expect("CYS_U43_OUT (result json path) not set"));
    let corpus = PathBuf::from(env("CYS_U43_CORPUS").expect("CYS_U43_CORPUS (dir holding INDEX.json) not set"));
    let replay = PathBuf::from(env("CYS_U43_REPLAY").expect("CYS_U43_REPLAY (replay_gen.py path) not set"));
    let python = env("CYS_U43_PYTHON").unwrap_or_else(|| "python3".into());
    if arm != Arm::Off {
        assert_ne!(std::env::var("CYS_DISPLAY_ACTIVITY").as_deref(), Ok("0"), "CYS_DISPLAY_ACTIVITY=0 would silently disable the observer arm");
    }
    let window = Duration::from_secs(secs);

    // ── isolation (same pattern as activity_tests::fx) ────────────────────────────────────────
    let _g = crate::governance::PACK_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    crate::delivery::tests::isolate_state_dir_for_thread("u43-main");
    let dir = std::env::temp_dir().join(format!("cys-u43-{}-{}", arm.as_str(), std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    std::fs::write(dir.join("acl.json"), r#"{"default":"allow","rules":[]}"#).unwrap();
    std::env::set_var(cys::pack::ENV_PACK_DIR, &dir);

    // ── corpus verification (sha256 vs INDEX.json) ────────────────────────────────────────────
    let index_raw = std::fs::read(corpus.join("INDEX.json")).expect("read INDEX.json");
    let index_sha = sha256_hex(&index_raw);
    let index: Value = serde_json::from_slice(&index_raw).expect("parse INDEX.json");
    let base = corpus.parent().expect("corpus dir has a parent").to_path_buf();
    let caps = [verify_capture(&base, &index, "claude", CLAUDE_FILE), verify_capture(&base, &index, "codex", CODEX_FILE)];
    let replay_sha = sha256_hex(&std::fs::read(&replay).expect("read replay_gen.py"));

    // ── provenance ───────────────────────────────────────────────────────────────────────────
    let root = env!("CARGO_MANIFEST_DIR");
    let head = cmd_out("git", &["-C", root, "rev-parse", "HEAD"]);
    let status = cmd_out("git", &["-C", root, "status", "--porcelain"]).unwrap_or_default();
    let src_sha: serde_json::Map<String, Value> = ["activity.rs", "state.rs", "main.rs", "activity_bench_tests.rs"]
        .iter()
        .map(|f| {
            let p = Path::new(root).join("src/bin/cysd").join(f);
            (f.to_string(), json!(std::fs::read(&p).map(|b| sha256_hex(&b)).ok()))
        })
        .collect();
    let load_at_start = cmd_out("/usr/sbin/sysctl", &["-n", "vm.loadavg"]);

    // ── daemon + seats ───────────────────────────────────────────────────────────────────────
    let daemon = Daemon::new(dir.join("cysd.sock"));
    let reserve = secs as usize;
    bench::install(reserve * 2_000 + 200_000, reserve * 120 + 10_000, reserve + 1_000, 100_000);
    bench::PTY_CAPTURE.store(true, Ordering::Release);
    let mut streaming: Vec<(Arc<Surface>, &'static str, u64)> = Vec::new();
    let mut reports: Vec<PathBuf> = Vec::new();
    for i in 0..2 * STREAM_PER_FAMILY {
        let cap = &caps[i % 2];
        let offset = splitmix64(seed ^ splitmix64(i as u64 + 1)) % cap.period_ns.max(1);
        let report = dir.join(format!("dsr-queries-{i}.txt"));
        let cmd = format!(
            "exec {} -I {} --file {} --offset-ns {offset} --sha256 {} --report {}",
            sh_quote(&python),
            sh_quote(&replay.to_string_lossy()),
            sh_quote(&cap.path.to_string_lossy()),
            cap.sha256,
            sh_quote(&report.to_string_lossy())
        );
        reports.push(report);
        let s = daemon.create_surface(None, Some(cmd), None, None, ROWS, COLS).expect("create streaming seat");
        daemon.surfaces.lock().unwrap().insert(s.id, s.clone());
        *s.agent_meta.lock().unwrap() = Some((cap.family.into(), cap.family.into()));
        streaming.push((s, cap.family, offset));
    }
    let mut ready: Vec<Arc<Surface>> = Vec::new();
    for _ in 0..READY_SEATS {
        let s = daemon
            .create_surface(None, Some("exec /bin/cat >/dev/null".into()), None, None, ROWS, COLS)
            .expect("create ready seat");
        daemon.surfaces.lock().unwrap().insert(s.id, s.clone());
        *s.agent_meta.lock().unwrap() = Some(("claude".into(), "/usr/local/bin/claude".into()));
        ready.push(s);
    }
    // warm-up: every streaming seat has published at least one chunk (bounded).
    let warm0 = Instant::now();
    while warm0.elapsed() < WARMUP_MAX && streaming.iter().any(|(s, _, _)| s.output_gen.load(Ordering::Acquire) < 2) {
        std::thread::sleep(Duration::from_millis(100));
    }
    let warm_unready: Vec<u64> =
        streaming.iter().filter(|(s, _, _)| s.output_gen.load(Ordering::Acquire) < 2).map(|(s, _, _)| s.id).collect();
    let warmup_s = warm0.elapsed().as_secs_f64();
    for s in &ready {
        crate::governance::h_paint(s, crate::governance::H_IDLE_SCREEN);
    }

    // ── measurement window ───────────────────────────────────────────────────────────────────
    bench::SLEEP_MS.store(if arm == Arm::Sleep { SLEEP_KNOB_MS } else { 0 }, Ordering::Release);
    let window_closed = Arc::new(AtomicBool::new(false));
    let enq_count = Arc::new(AtomicUsize::new(0));
    let del_count = Arc::new(AtomicUsize::new(0));
    let delivered: Arc<Mutex<Vec<Delivered>>> = Arc::new(Mutex::new(Vec::new()));
    let lagged = Arc::new(AtomicUsize::new(0));
    // bus listener: wall instant of every queue.delivered (the item has left pending_queue by then).
    let listener = {
        let mut rx = daemon.bus.subscribe();
        let delivered = delivered.clone();
        let del_count = del_count.clone();
        let lagged = lagged.clone();
        std::thread::Builder::new()
            .name("u43-bus".into())
            .spawn(move || loop {
                match rx.blocking_recv() {
                    Ok(ev) => {
                        let at = Instant::now();
                        if ev["name"] == json!("queue.delivered") {
                            let merged = ev["payload"]["merged_ids"].as_array().map(|a| a.len()).unwrap_or(1).max(1);
                            del_count.fetch_add(merged, Ordering::AcqRel);
                            delivered.lock().unwrap().push(Delivered { at, sid: ev["surface_id"].as_u64(), payload: ev["payload"].clone() });
                        } else if ev["name"] == json!("u43.harness_stop") {
                            return;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        lagged.fetch_add(n as usize, Ordering::AcqRel);
                    }
                    Err(_) => return,
                }
            })
            .expect("spawn listener")
    };

    let w0 = Instant::now();
    let start_wall = crate::state::now_epoch();
    bench::ENABLED.store(true, Ordering::Release);
    if arm != Arm::Off {
        crate::activity::spawn_observer(Arc::clone(&daemon));
    }

    // delivery: the real deliver_queued tick on fixed deadlines (production period), then a bounded drain.
    let delivery = {
        let daemon = Arc::clone(&daemon);
        let closed = window_closed.clone();
        let enq_count = enq_count.clone();
        let del_count = del_count.clone();
        std::thread::Builder::new()
            .name("u43-delivery".into())
            .spawn(move || {
                crate::delivery::tests::isolate_state_dir_for_thread("u43-delivery");
                let ledger = crate::delivery::ledger_path(&daemon.socket_path);
                let period = Duration::from_secs(crate::governance::WATCHDOG_INTERVAL_SECS);
                let mut ticks: Vec<TickRec> = Vec::new();
                let mut drain = 0u32;
                let mut k = 1u32;
                loop {
                    let deadline = w0 + period * k;
                    k += 1;
                    // wait for the deadline, but look at the window state every 100 ms.
                    while Instant::now() < deadline {
                        if closed.load(Ordering::Acquire) && del_count.load(Ordering::Acquire) >= enq_count.load(Ordering::Acquire) {
                            return (ticks, ledger);
                        }
                        std::thread::sleep((deadline - Instant::now()).min(Duration::from_millis(100)));
                    }
                    let in_window = !closed.load(Ordering::Acquire);
                    if !in_window {
                        if del_count.load(Ordering::Acquire) >= enq_count.load(Ordering::Acquire) || drain >= DRAIN_MAX_TICKS {
                            return (ticks, ledger);
                        }
                        drain += 1;
                    }
                    let _ = bench::fsync_take(); // A2-1: this thread's accumulator starts at zero for the tick
                    let start_epoch = crate::state::now_epoch();
                    let start = Instant::now();
                    crate::governance::h_queue_tick(&daemon);
                    let end = Instant::now();
                    let (fsync, fsync_calls) = bench::fsync_take();
                    let end_epoch = crate::state::now_epoch();
                    ticks.push(TickRec { deadline, start, end, start_epoch, end_epoch, in_window, fsync, fsync_calls });
                }
            })
            .expect("spawn delivery")
    };

    // attach snapshot probe: blocking parser lock + subscribe + contents_formatted (run_attach's locked work), 20/s.
    let attach = {
        let seats: Vec<Arc<Surface>> = streaming.iter().map(|(s, _, _)| s.clone()).collect();
        let closed = window_closed.clone();
        std::thread::Builder::new()
            .name("u43-attach".into())
            .spawn(move || {
                let mut out: Vec<AttachRec> = Vec::with_capacity(secs as usize * 25 + 100);
                let mut k = 0u32;
                loop {
                    let deadline = w0 + ATTACH_PERIOD * k;
                    sleep_until(deadline);
                    if closed.load(Ordering::Acquire) {
                        return out;
                    }
                    let s = &seats[k as usize % seats.len()];
                    k += 1;
                    let t0 = Instant::now();
                    let (rx, snap, t1) = {
                        let parser = s.parser.lock().unwrap_or_else(|e| e.into_inner());
                        let t1 = Instant::now();
                        let rx = s.out_tx.subscribe();
                        (rx, parser.screen().contents_formatted(), t1)
                    };
                    let t2 = Instant::now();
                    drop(rx);
                    out.push(AttachRec { at: t0, sid: s.id, wait: t1 - t0, total: t2 - t0, bytes: snap.len() });
                }
            })
            .expect("spawn attach probe")
    };

    // queue feeder: one item every 7 s to the 4 ready seats in rotation (idle screen repainted first).
    let feeder = {
        let daemon = Arc::clone(&daemon);
        let ready = ready.clone();
        let closed = window_closed.clone();
        let enq_count = enq_count.clone();
        std::thread::Builder::new()
            .name("u43-feeder".into())
            .spawn(move || {
                let mut out: Vec<Enq> = Vec::new();
                let last = w0 + window - FEED_STOP_BEFORE_END;
                let mut n = 0u32;
                loop {
                    let at = w0 + FEED_FIRST + FEED_PERIOD * n;
                    if at > last {
                        return out;
                    }
                    sleep_until(at);
                    if closed.load(Ordering::Acquire) {
                        return out;
                    }
                    let s = &ready[n as usize % ready.len()];
                    crate::governance::h_paint(s, crate::governance::H_IDLE_SCREEN);
                    let e = daemon.next_queue_entry(format!("[u43] queued item {n}"), None, "test");
                    let (id, epoch) = (e.id.clone(), e.enqueued_at);
                    let at = Instant::now();
                    s.pending_queue.lock().unwrap().push_back(e);
                    enq_count.fetch_add(1, Ordering::AcqRel);
                    out.push(Enq { id, sid: s.id, at, epoch });
                    n += 1;
                }
            })
            .expect("spawn feeder")
    };

    // window end: SECS wall; the ON arm also needs >= SECS observer ticks (cap SECS + 10 s).
    loop {
        std::thread::sleep(Duration::from_millis(50));
        let el = w0.elapsed();
        let ticks = daemon.activity.stats.lock().map(|s| s.ticks).unwrap_or(0);
        if (el >= window && (arm != Arm::On || ticks >= secs)) || el >= window + Duration::from_secs(10) {
            break;
        }
    }
    let w1 = Instant::now();
    bench::ENABLED.store(false, Ordering::Release);
    window_closed.store(true, Ordering::Release);
    let end_wall = crate::state::now_epoch();
    let attach_recs = attach.join().expect("attach probe");
    let enqueued = feeder.join().expect("feeder");
    let (ticks, ledger_path) = delivery.join().expect("delivery");
    std::thread::sleep(Duration::from_millis(200));
    daemon.bus.publish("u43.harness_stop", "test", None, json!({}));
    listener.join().expect("listener");
    bench::SLEEP_MS.store(0, Ordering::Release);
    let stats = daemon.activity.stats.lock().map(|g| g.clone()).unwrap_or_default();
    let observer_running = daemon.activity.running.load(Ordering::Relaxed);
    let load_at_end = cmd_out("/usr/sbin/sysctl", &["-n", "vm.loadavg"]);

    // ── queue accounting ─────────────────────────────────────────────────────────────────────
    let delivered = std::mem::take(&mut *delivered.lock().unwrap());
    let mut by_id: HashMap<String, (Instant, Option<u64>, f64, usize)> = HashMap::new();
    for (i, d) in delivered.iter().enumerate() {
        let at_epoch = d.payload["delivered_at"].as_f64().unwrap_or(f64::NAN);
        let ids: Vec<String> = d.payload["merged_ids"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
            .filter(|v: &Vec<String>| !v.is_empty())
            .unwrap_or_else(|| vec![d.payload["queue_entry_id"].as_str().unwrap_or("").to_string()]);
        for id in ids {
            by_id.entry(id).or_insert((d.at, d.sid, at_epoch, i));
        }
    }
    let ledger_text = std::fs::read_to_string(&ledger_path).unwrap_or_default();
    let ledger_rows: Vec<Value> = ledger_text.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()).collect();
    let ledger_queue: Vec<&Value> = ledger_rows.iter().filter(|r| r["origin"] == json!("queue") && r.get("part").is_none()).collect();
    let ledger_receipts: Vec<&Value> = ledger_rows.iter().filter(|r| r["origin"] == json!("queue_receipt")).collect();
    let items: Vec<Value> = enqueued
        .iter()
        .map(|e| {
            let d = by_id.get(&e.id);
            json!({
                "id": e.id, "sid": e.sid, "enqueued_t_ns": rel(e.at, w0), "enqueued_at_epoch": e.epoch,
                "delivered": d.is_some(),
                "delivered_t_ns": d.map(|x| rel(x.0, w0)),
                "delivered_sid": d.and_then(|x| x.1),
                "ready_to_submission_ns": d.map(|x| if x.0 >= e.at { ns(x.0 - e.at) as i64 } else { -(ns(e.at - x.0) as i64) }),
                "epoch_delta_s": d.map(|x| x.2 - e.epoch),
            })
        })
        .collect();

    // ── teardown: kill every seat child and reap it; then list our remaining children ───────────
    let all: Vec<Arc<Surface>> = streaming.iter().map(|(s, _, _)| s.clone()).chain(ready.iter().cloned()).collect();
    for s in &all {
        let _ = s.child.lock().unwrap_or_else(|e| e.into_inner()).kill();
    }
    let reap0 = Instant::now();
    let mut unreaped: Vec<u64> = all.iter().map(|s| s.id).collect();
    while !unreaped.is_empty() && reap0.elapsed() < Duration::from_secs(10) {
        unreaped.retain(|id| {
            let s = all.iter().find(|s| s.id == *id).unwrap();
            let r = s.child.lock().unwrap_or_else(|e| e.into_inner()).try_wait();
            // Err = already reaped by the reader thread's EOF path (ECHILD): gone.
            !matches!(r, Ok(Some(_)) | Err(_))
        });
        if !unreaped.is_empty() {
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    std::thread::sleep(Duration::from_millis(300));
    let children_left = children_of_self();
    // A1-1: daemon → PTY writes are captured until every seat child is dead (a reply to a query in flight at the
    // kill is the only allowed mismatch); timing probes were already off at the window end.
    bench::PTY_CAPTURE.store(false, Ordering::Release);
    let sink = bench::take();
    // A1-1: generator-side ESC[6n counts, one report file per streaming seat.
    let dsr_queries: Vec<Value> = streaming
        .iter()
        .zip(reports.iter())
        .map(|((s, fam, _), path)| {
            let text = std::fs::read_to_string(path).ok();
            let mut total = 0u64;
            let mut at: Vec<u64> = Vec::new();
            for line in text.as_deref().unwrap_or("").lines().skip(1) {
                let mut it = line.split_whitespace();
                if let (Some(t), Some(n)) = (it.next().and_then(|v| v.parse::<u64>().ok()), it.next().and_then(|v| v.parse::<u64>().ok())) {
                    at.push(t);
                    total = n;
                }
            }
            json!({"sid": s.id, "family": fam, "report_found": text.is_some(),
                   "generator_started": text.as_deref().is_some_and(|t| t.starts_with("start ")),
                   "queries": total, "query_monotonic_ns": at})
        })
        .collect();

    // ── result ───────────────────────────────────────────────────────────────────────────────
    let stream_sids: Vec<u64> = streaming.iter().map(|(s, _, _)| s.id).collect();
    let ready_sids: Vec<u64> = ready.iter().map(|s| s.id).collect();
    let pty: Vec<Value> = sink
        .pty
        .iter()
        .map(|p| json!({"t_ns": rel(p.at, w0), "sid": p.sid, "len": p.len, "bytes_hex": hex(&p.head), "truncated": p.head.len() < p.len}))
        .collect();
    let bytes_to = |sids: &[u64]| -> u64 { sink.pty.iter().filter(|p| sids.contains(&p.sid)).map(|p| p.len as u64).sum() };
    let tick_in: Vec<&TickRec> = ticks.iter().filter(|t| t.in_window).collect();
    let tick_drain: Vec<&TickRec> = ticks.iter().filter(|t| !t.in_window).collect();
    // diagnostic: items delivered inside each tick (queue.delivered is published inside the tick; its delivered_at is
    // the daemon's epoch clock, compared with the tick's own epoch stamps).
    let delivered_in = |t: &TickRec| -> u64 {
        delivered
            .iter()
            .filter(|d| d.payload["delivered_at"].as_f64().is_some_and(|e| e >= t.start_epoch && e <= t.end_epoch))
            .map(|d| d.payload["merged_ids"].as_array().map(|a| a.len()).unwrap_or(1).max(1) as u64)
            .sum()
    };
    let tick_arrays = |ts: &[&TickRec]| {
        json!({
            "unit": "ns", "n": ts.len(),
            "delivered_items": ts.iter().map(|t| delivered_in(t)).collect::<Vec<_>>(),
            "fsync_ns": ts.iter().map(|t| ns(t.fsync)).collect::<Vec<_>>(),
            "fsync_calls": ts.iter().map(|t| t.fsync_calls).collect::<Vec<_>>(),
            "deadline_t_ns": ts.iter().map(|t| rel(t.deadline, w0)).collect::<Vec<_>>(),
            "exec_ns": ts.iter().map(|t| ns(t.end - t.start)).collect::<Vec<_>>(),
            "deadline_to_start_ns": ts.iter().map(|t| ns(t.start.saturating_duration_since(t.deadline))).collect::<Vec<_>>(),
        })
    };
    let result = json!({
        "schema": "u43-arm-v1",
        "plan": "U43-LOCK-PLAN.md (locked 2026-10-16T04:59:16Z)",
        "arm": arm.as_str(),
        "secs": secs,
        "smoke": secs < 1000,
        "seed": seed,
        "round": env("CYS_U43_ROUND"),
        "rounds": env("CYS_U43_ROUNDS"),
        "gate_max_wait_s": env("CYS_U43_GATE_MAX_S"),
        "arm_order": env("CYS_U43_ARM_ORDER"),
        "order_seed": env("CYS_U43_ORDER_SEED"),
        "start_wall_epoch": start_wall,
        "end_wall_epoch": end_wall,
        "window_ns": ns(w1 - w0),
        "warmup_s": warmup_s,
        "warmup_unready_sids": warm_unready,
        "git": {"head": head, "status_porcelain": status.lines().collect::<Vec<_>>(), "source_sha256": src_sha},
        "test_binary_sha256": env("CYS_U43_BIN_SHA256"),
        "corpus": {
            "index_sha256": index_sha,
            "captures": caps.iter().map(|c| json!({
                "family": c.family, "file": c.path.to_string_lossy(), "sha256": c.sha256,
                "index_sha256": c.index_sha256, "match": c.sha256 == c.index_sha256,
                "period_ns": c.period_ns, "output_records": c.outputs,
            })).collect::<Vec<_>>(),
        },
        "replay_gen": {"path": replay.to_string_lossy(), "sha256": replay_sha, "python": python},
        "load": {
            "reading": env("CYS_U43_LOAD_READING"), "one_min": env("CYS_U43_LOAD_ONE_MIN"),
            "loaded": env("CYS_U43_LOADED"), "waited_s": env("CYS_U43_LOAD_WAITED_S"),
            "at_test_start": load_at_start, "at_test_end": load_at_end,
        },
        "workload": {
            "streaming_seats": streaming.iter().map(|(s, f, o)| json!({"sid": s.id, "family": f, "offset_ns": o})).collect::<Vec<_>>(),
            "ready_seats": ready_sids,
            "rows": ROWS, "cols": COLS,
            "feed_period_ms": ns(FEED_PERIOD) / 1_000_000,
            "feed_stop_before_end_ms": ns(FEED_STOP_BEFORE_END) / 1_000_000,
            "delivery_period_ms": crate::governance::WATCHDOG_INTERVAL_SECS * 1000,
            "settle_budget_ms": crate::governance::QUEUE_TICK_SETTLE_BUDGET_MS,
            "attach_period_ms": ns(ATTACH_PERIOD) / 1_000_000,
            "sleep_knob_ms": if arm == Arm::Sleep { SLEEP_KNOB_MS } else { 0 },
            "observer_started": arm != Arm::Off,
        },
        "metrics": {
            "observer_hold": rec_arrays(&sink.hold, w0, "v", None),
            "observer_tick": rec_arrays(&sink.tick, w0, "v", Some("slept_ns")),
            "observer_seat": rec_arrays(&sink.seat, w0, "v", Some("slept_ns")),
            // (parenthesised: inside json! a bare `{` would start a JSON object)
            "reader": ({
                let mut r = rec_arrays(&sink.reader, w0, "lock_wait_ns", Some("chunk_ns"));
                // U4-3 R2 attribution: reader thread CPU time per chunk (null = no per-thread clock), index-aligned with chunk_ns.
                if let Some(m) = r.as_object_mut() {
                    m.insert("chunk_cpu_ns".into(), json!(sink.reader_cpu.iter().map(|c| c.map(ns)).collect::<Vec<_>>()));
                }
                r
            }),
            "attach_snapshot": {
                "unit": "ns", "n": attach_recs.len(),
                "t_ns": attach_recs.iter().map(|r| rel(r.at, w0)).collect::<Vec<_>>(),
                "sid": attach_recs.iter().map(|r| r.sid).collect::<Vec<_>>(),
                "v": attach_recs.iter().map(|r| ns(r.total)).collect::<Vec<_>>(),
                "lock_wait_ns": attach_recs.iter().map(|r| ns(r.wait)).collect::<Vec<_>>(),
                "snapshot_bytes": attach_recs.iter().map(|r| r.bytes).collect::<Vec<_>>(),
            },
            "delivery_tick": tick_arrays(&tick_in),
            "delivery_tick_drain": tick_arrays(&tick_drain),
        },
        "counts": {
            "observer_sleeping_classifications": sink.sleeps,
            "observer_ticks_in_window": sink.tick.len(),
            "observer_hold_samples": sink.hold.len(),
            "reader_samples": sink.reader.len(),
            "attach_samples": attach_recs.len(),
            "delivery_ticks_in_window": tick_in.len(),
            "delivery_ticks_drain": tick_drain.len(),
            "bus_lagged_events": lagged.load(Ordering::Acquire),
        },
        "observer_stats": {
            "running": observer_running, "nonce_present": daemon.activity.nonce.is_some(),
            "ticks": stats.ticks, "ticks_skipped": stats.ticks_skipped, "attempts": stats.attempts,
            "accepted": stats.accepted, "transient_lock": stats.transient[0], "transient_odd": stats.transient[1],
            "transient_incomplete": stats.transient[2], "semantic": stats.semantic, "invalidated": stats.invalidated,
            "hold_us_max": stats.hold_us_max, "tick_us_max": stats.tick_us_max,
        },
        "queue": {
            "enqueued": enqueued.len(),
            "delivered_items": items.iter().filter(|i| i["delivered"] == json!(true)).count(),
            "delivered_events": delivered.len(),
            "items": items,
            "ledger_path": ledger_path.to_string_lossy(),
            "ledger_queue_records": ledger_queue.len(),
            "ledger_queue_receipts": ledger_receipts.len(),
            "ledger_rows_total": ledger_rows.len(),
            "ledger_queue_rows": ledger_queue,
            "ledger_receipt_rows": ledger_receipts,
        },
        "dsr_queries": dsr_queries,
        "pty_writes": {
            "capture": "every daemon -> PTY write from before seat creation until all seat children are reaped",
            "streaming_bytes_total": bytes_to(&stream_sids),
            "ready_bytes_total": bytes_to(&ready_sids),
            "writes": pty,
        },
        "teardown": {"unreaped_seat_ids": unreaped, "children_left": children_left},
    });
    if let Some(d) = out_path.parent() {
        std::fs::create_dir_all(d).expect("result dir");
    }
    let f = std::fs::File::create(&out_path).expect("create result json");
    serde_json::to_writer(std::io::BufWriter::new(f), &result).expect("write result json");
    let _ = std::fs::remove_dir_all(&dir);
    eprintln!(
        "u43 arm={} secs={secs} window_s={:.3} hold={} tick={} reader={} attach={} delivery_ticks={} enq={} delivered={} children_left={} -> {}",
        arm.as_str(),
        (w1 - w0).as_secs_f64(),
        sink.hold.len(),
        sink.tick.len(),
        sink.reader.len(),
        attach_recs.len(),
        tick_in.len(),
        enqueued.len(),
        result["queue"]["delivered_items"],
        result["teardown"]["children_left"].as_array().map(|a| a.len()).unwrap_or(0),
        out_path.display()
    );
    assert!(result["teardown"]["children_left"].as_array().is_some_and(|a| a.is_empty()), "seat children left running: {}", result["teardown"]["children_left"]);
}
