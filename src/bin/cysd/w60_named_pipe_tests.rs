//! W60 S1 — the Windows named-pipe specimen (DESIGN-W60-r2 §9.3, HEAD GO 2026-11-04 with codex r2 I2).
//!
//! The owner's incident: mio 1.2.1 reports a write that the OS already accepted (ERROR_IO_PENDING) as failed when the
//! client closes the pipe at that moment; the IOCP completion then meets a "no write" state and mio hits
//! `unreachable!()` at `src/sys/windows/named_pipe.rs:969`, which aborts the whole daemon. mio 1.2.4 carries the
//! upstream fix (mio PR #2014), and 0.14.57 ships it on that basis.
//!
//! ★Scope after R (HEAD-RULING-W60-AFTER-R.md): this is a SMOKE check, not a demonstration. At R f9dc2af4 (mio 1.2.1,
//! windows-health run 41761853211) it did NOT reproduce the crash on GitHub's hosted runners — the client close is not
//! aligned with the write submit and a 4-vCPU runner is not oversubscribed (dept-5 W60-MIO/RIG-INVESTIGATION-R.md). So a
//! green run shows only that this pipe path does not panic; it does not show the fix. The upstream-shaped reproducer
//! is backlog row D5H-20261104-W60-S1PRIME-REPRODUCER.
//!
//! This module drives that race shape on the tokio/mio that `Cargo.lock` pins, in a CHILD process (the test binary
//! re-run with `--exact` on [`w60_s1_child`]), so a mio abort kills only the child. The parent prints exactly one
//! `W60-S1 RESULT=` line that binds the outcome to the commit sha, the mio version and checksum from the lock, and
//! the sha256 of the executable that ran. The decision reads that line; the step colour alone is never evidence.
//!
//! Locked protocol (§9.3; numbers change only through the workflow env, never silently):
//! * `W60_S1_SECS` (default 60) seconds of rounds, `W60_S1_CLIENTS` (default 8) client tasks, 128 KiB per server
//!   write. A round = one client connect + close. Close timing varies by round (at once / after one / two yields).
//! * Parent grace = max(60, secs) after the run, then kill and a 10 s bounded reap (cleanup outcome printed).
//! * Work minimum (codex r2 I2): every client completes at least max(100, total / (2 × clients)) rounds, and the
//!   clients' active windows [first, last completed round] share one common window of at least half the run. All
//!   clients also pass a start barrier together.
//! * Classes: R-WITNESS (child exits nonzero AND its stderr names both `mio-1.2.1` and `named_pipe.rs:969`; the only
//!   valid R witness), OTHER-FAILURE, PASS (G arm), NOT-RUN reason=no-reproduce (R arm, passes, blocks G from counting
//!   as a demonstration), NOT-RUN reason=work, NOT-RUN reason=timeout. The arm comes from the lock: mio 1.2.1 = R,
//!   1.2.4 = G.
//! * `r_round` on R-WITNESS = last printed progress count + 10,000: an upper bound, so the factor-10 rule asks more
//!   of G, never less.

use crate::state::HideConsole;
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const WRITE_BYTES: usize = 128 * 1024;
const PROGRESS_EVERY: u64 = 10_000;
const PER_CLIENT_ABSOLUTE_FLOOR: u64 = 100;
const CHILD_TEST: &str = "w60_named_pipe_tests::w60_s1_child";
/// Win32 ERROR_PIPE_BUSY: every server instance is taken; the client retries.
const ERROR_PIPE_BUSY: i32 = 231;
/// Echoed child output is capped so a runaway child cannot flood the step log.
const ECHO_CAP: usize = 256 * 1024;

fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key).ok().and_then(|v| v.trim().parse::<u64>().ok()).filter(|v| *v > 0).unwrap_or(default)
}

/// The child: the race itself. A fixture, not a test of its own — it does nothing unless the parent set `W60_CHILD`.
#[test]
fn w60_s1_child() {
    if std::env::var_os("W60_CHILD").is_none() {
        return;
    }
    let secs = env_u64("W60_S1_SECS", 60);
    let clients = env_u64("W60_S1_CLIENTS", 8) as usize;
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(4).enable_all().build().expect("child runtime");
    rt.block_on(child_race(secs, clients));
    rt.shutdown_timeout(Duration::from_secs(5));
}

async fn child_race(secs: u64, clients: usize) {
    use tokio::io::AsyncWriteExt;
    use tokio::net::windows::named_pipe::{ClientOptions, ServerOptions};

    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let name = format!(r"\\.\pipe\cys-w60-s1-{}-{}", std::process::id(), nanos);
    println!("W60-S1-CHILD start secs={secs} clients={clients} write_bytes={WRITE_BYTES} pipe={name}");

    let payload = Arc::new(vec![0x5Au8; WRITE_BYTES]);
    let total = Arc::new(AtomicU64::new(0));
    let write_ok = Arc::new(AtomicU64::new(0));
    let write_err = Arc::new(AtomicU64::new(0));
    let connect_err = Arc::new(AtomicU64::new(0));
    let in_write = Arc::new(AtomicUsize::new(0));
    let max_in_write = Arc::new(AtomicUsize::new(0));

    // Server: one instance waits for a client; on connect a fresh instance is created first, then the connected one
    // gets its 128 KiB write in its own task while the client is closing.
    let first = ServerOptions::new().first_pipe_instance(true).create(&name).expect("first pipe instance");
    let server = {
        let (name, payload) = (name.clone(), payload.clone());
        let (write_ok, write_err, connect_err) = (write_ok.clone(), write_err.clone(), connect_err.clone());
        let (in_write, max_in_write) = (in_write.clone(), max_in_write.clone());
        tokio::spawn(async move {
            let mut server = first;
            loop {
                let connected = server.connect().await;
                let next = match ServerOptions::new().create(&name) {
                    Ok(next) => next,
                    Err(e) => {
                        println!("W60-S1-CHILD server create error: {e}");
                        return;
                    }
                };
                let pipe = std::mem::replace(&mut server, next);
                if connected.is_err() {
                    connect_err.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
                let (payload, write_ok, write_err) = (payload.clone(), write_ok.clone(), write_err.clone());
                let (in_write, max_in_write) = (in_write.clone(), max_in_write.clone());
                tokio::spawn(async move {
                    let mut pipe = pipe;
                    let now = in_write.fetch_add(1, Ordering::Relaxed) + 1;
                    max_in_write.fetch_max(now, Ordering::Relaxed);
                    match pipe.write_all(&payload).await {
                        Ok(()) => write_ok.fetch_add(1, Ordering::Relaxed),
                        Err(_) => write_err.fetch_add(1, Ordering::Relaxed),
                    };
                    in_write.fetch_sub(1, Ordering::Relaxed);
                });
            }
        })
    };

    let start = Instant::now();
    let deadline = start + Duration::from_secs(secs);
    let barrier = Arc::new(tokio::sync::Barrier::new(clients));
    let mut handles = Vec::with_capacity(clients);
    for idx in 0..clients {
        let (name, total, barrier) = (name.clone(), total.clone(), barrier.clone());
        handles.push(tokio::spawn(async move {
            barrier.wait().await;
            let (mut rounds, mut open_err) = (0u64, 0u64);
            let (mut first_ms, mut last_ms) = (None::<u128>, 0u128);
            while Instant::now() < deadline { // wallclock: guard the run length is the specimen's exposure budget; the verdict reads exit status, stderr and counted work, never this clock
                match ClientOptions::new().open(&name) {
                    Ok(client) => {
                        // Vary the close timing so the close lands before, during and after the server's submit.
                        for _ in 0..(rounds % 3) {
                            tokio::task::yield_now().await;
                        }
                        drop(client);
                        rounds += 1;
                        let ms = start.elapsed().as_millis();
                        first_ms.get_or_insert(ms);
                        last_ms = ms;
                        let all = total.fetch_add(1, Ordering::Relaxed) + 1;
                        if all % PROGRESS_EVERY == 0 {
                            println!("progress rounds={all}");
                        }
                    }
                    Err(e) => {
                        if e.raw_os_error() != Some(ERROR_PIPE_BUSY) && e.kind() != std::io::ErrorKind::NotFound {
                            open_err += 1;
                        }
                        tokio::time::sleep(Duration::from_millis(1)).await;
                    }
                }
            }
            (idx, rounds, first_ms, last_ms, open_err)
        }));
    }

    let reporter = {
        let total = total.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(5));
            tick.tick().await;
            loop {
                tick.tick().await;
                println!("progress rounds={}", total.load(Ordering::Relaxed));
            }
        })
    };

    let mut open_err_total = 0u64;
    for handle in handles {
        let (idx, rounds, first_ms, last_ms, open_err) = handle.await.expect("client task");
        open_err_total += open_err;
        let first = first_ms.map(|v| v.to_string()).unwrap_or_else(|| "-".into());
        println!("W60-S1-CHILD client={idx} rounds={rounds} first_ms={first} last_ms={last_ms}");
    }
    reporter.abort();
    server.abort();
    println!(
        "W60-S1-CHILD done total={} write_ok={} write_err={} connect_err={} open_err={} max_concurrent_writes={}",
        total.load(Ordering::Relaxed),
        write_ok.load(Ordering::Relaxed),
        write_err.load(Ordering::Relaxed),
        connect_err.load(Ordering::Relaxed),
        open_err_total,
        max_in_write.load(Ordering::Relaxed),
    );
}

/// `(version, checksum)` of every `mio` entry in the lock this test binary was built from, joined with `+` if the
/// lock holds more than one.
fn lock_mio() -> (String, String) {
    let lock = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.lock")).unwrap_or_default();
    let (mut versions, mut sums) = (Vec::new(), Vec::new());
    for block in lock.split("[[package]]") {
        let field = |key: &str| {
            block.lines().find_map(|l| {
                let rest = l.trim().strip_prefix(key)?.trim_start().strip_prefix('=')?;
                Some(rest.trim().trim_matches('"').to_string())
            })
        };
        if field("name").as_deref() == Some("mio") {
            versions.push(field("version").unwrap_or_else(|| "?".into()));
            sums.push(field("checksum").unwrap_or_else(|| "?".into()));
        }
    }
    let join = |v: Vec<String>| if v.is_empty() { "?".to_string() } else { v.join("+") };
    (join(versions), join(sums))
}

fn sha256_file(path: &std::path::Path) -> String {
    use sha2::{Digest, Sha256};
    let Ok(mut file) = std::fs::File::open(path) else { return "unreadable".into() };
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => hasher.update(&buf[..n]),
            Err(_) => return "unreadable".into(),
        }
    }
    hasher.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Reads one child stream to the end on its own thread, so a full pipe can never stall the child.
fn drain(stream: impl Read + Send + 'static) -> std::sync::mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut stream = stream;
        let mut out = Vec::new();
        let _ = stream.read_to_end(&mut out);
        let _ = tx.send(out);
    });
    rx
}

struct ClientWork {
    rounds: u64,
    first_ms: Option<u64>,
    last_ms: u64,
}

fn parse_clients(stdout: &str) -> Vec<ClientWork> {
    let value = |line: &str, key: &str| {
        line.split_whitespace().find_map(|t| t.strip_prefix(key)).map(str::to_string)
    };
    stdout
        .lines()
        .filter(|l| l.starts_with("W60-S1-CHILD client="))
        .map(|l| ClientWork {
            rounds: value(l, "rounds=").and_then(|v| v.parse().ok()).unwrap_or(0),
            first_ms: value(l, "first_ms=").and_then(|v| v.parse().ok()),
            last_ms: value(l, "last_ms=").and_then(|v| v.parse().ok()).unwrap_or(0),
        })
        .collect()
}

/// `(ok, total, per_client_min, floor, overlap_ms, reason)` of the work minimum (codex r2 I2).
fn work_minimum(stdout: &str, clients: u64, secs: u64) -> (bool, u64, u64, u64, i128, &'static str) {
    let work = parse_clients(stdout);
    let total: u64 = work.iter().map(|c| c.rounds).sum();
    let floor = PER_CLIENT_ABSOLUTE_FLOOR.max(total / (2 * clients.max(1)));
    let per_client_min = work.iter().map(|c| c.rounds).min().unwrap_or(0);
    let latest_first = work.iter().map(|c| c.first_ms.map(i128::from).unwrap_or(i128::MAX)).max().unwrap_or(i128::MAX);
    let earliest_last = work.iter().map(|c| i128::from(c.last_ms)).min().unwrap_or(0);
    let overlap_ms = if latest_first == i128::MAX { -1 } else { earliest_last - latest_first };
    let reason = if work.len() as u64 != clients {
        "client-lines"
    } else if per_client_min < floor {
        "per-client-floor"
    } else if overlap_ms < i128::from(secs) * 1000 / 2 {
        "overlap-window"
    } else {
        ""
    };
    (reason.is_empty(), total, per_client_min, floor, overlap_ms, reason)
}

fn last_progress(stdout: &str) -> u64 {
    stdout.lines().filter_map(|l| l.strip_prefix("progress rounds=")).filter_map(|v| v.trim().parse().ok()).max().unwrap_or(0)
}

fn echo(label: &str, bytes: &[u8]) {
    let text = String::from_utf8_lossy(&bytes[..bytes.len().min(ECHO_CAP)]);
    for line in text.lines() {
        // Prefixed so the child's own libtest `test result:` line can never be read as the parent's.
        println!("[child {label}] {line}");
    }
    if bytes.len() > ECHO_CAP {
        println!("[child {label}] ... {} more bytes not echoed", bytes.len() - ECHO_CAP);
    }
}

/// S1. Blocking in windows-health from commit R (D-W60-4) as a SMOKE check: R (mio 1.2.1) gave NOT-RUN no-reproduce on
/// hosted runners (an R-WITNESS was the design's hope); G (mio 1.2.4) must give PASS, which means "no panic here", not "fix shown".
#[test]
fn w60_named_pipe_large_write_while_clients_close() {
    let secs = env_u64("W60_S1_SECS", 60);
    let clients = env_u64("W60_S1_CLIENTS", 8);
    let grace = secs.max(60);
    let (mio, mio_checksum) = lock_mio();
    let arm = match mio.as_str() {
        "1.2.1" => "R",
        "1.2.4" => "G",
        _ => "?",
    };
    let exe = std::env::current_exe().expect("test executable");
    let exe_sha256 = sha256_file(&exe);
    let sha = std::env::var("GITHUB_SHA").unwrap_or_else(|_| "unset".into());

    let mut child = Command::new(&exe)
        .args(["--exact", CHILD_TEST, "--nocapture", "--test-threads=1"])
        .env("W60_CHILD", "1")
        .env("W60_S1_SECS", secs.to_string())
        .env("W60_S1_CLIENTS", clients.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .hide_console()
        .spawn()
        .expect("spawn the S1 child");
    let out_rx = drain(child.stdout.take().expect("child stdout"));
    let err_rx = drain(child.stderr.take().expect("child stderr"));

    // Independent parent deadline: the child's own clock never decides the outcome.
    let deadline = Instant::now() + Duration::from_secs(secs + grace);
    let status = loop { // wallclock: guard bounds a child that always ends (its run is secs long); expiry is classified NOT-RUN timeout, never a pass
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
            _ => break None,
        }
    };
    let mut cleanup = "";
    if status.is_none() {
        cleanup = if child.kill().is_err() { "kill-failed" } else { "unreaped" };
        let reap_by = Instant::now() + Duration::from_secs(10);
        while cleanup == "unreaped" && Instant::now() < reap_by { // wallclock: guard bounded reap after kill; the outcome is printed, never a pass
            if let Ok(Some(_)) = child.try_wait() {
                cleanup = "reaped";
            } else {
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
    let stdout = out_rx.recv_timeout(Duration::from_secs(10)).unwrap_or_default(); // wallclock: guard the stream ends when the child ends; a timeout only loses echo text
    let stderr = err_rx.recv_timeout(Duration::from_secs(10)).unwrap_or_default(); // wallclock: guard the stream ends when the child ends; a timeout only loses echo text
    let out_text = String::from_utf8_lossy(&stdout).into_owned();
    let err_text = String::from_utf8_lossy(&stderr).into_owned();

    let site = err_text.contains("mio-1.2.1") && err_text.contains("named_pipe.rs:969");
    let any_pipe_site = err_text.contains("named_pipe.rs");
    let (work_ok, total, per_client_min, floor, overlap_ms, work_reason) = work_minimum(&out_text, clients, secs);
    let exit = match &status {
        Some(s) => s.code().map(|c| format!("0x{:08X}", c as u32)).unwrap_or_else(|| "signal".into()),
        None => "none".into(),
    };
    let class = match &status {
        None => "NOT-RUN reason=timeout".to_string(),
        Some(s) if !s.success() && site => format!("R-WITNESS r_round={}", last_progress(&out_text) + PROGRESS_EVERY),
        Some(s) if !s.success() => "OTHER-FAILURE".to_string(),
        Some(_) if any_pipe_site => "OTHER-FAILURE site_seen=1".to_string(),
        Some(_) if !work_ok => format!("NOT-RUN reason=work work_fail={work_reason}"),
        Some(_) if arm == "R" => "NOT-RUN reason=no-reproduce".to_string(),
        Some(_) => "PASS".to_string(),
    };
    let cleanup = if cleanup.is_empty() { String::new() } else { format!(" cleanup={cleanup}") };
    let line = format!(
        "W60-S1 RESULT={class} arm={arm} sha={sha} mio={mio} mio_checksum={mio_checksum} exe_sha256={exe_sha256} \
         secs={secs} clients={clients} grace={grace} rounds={total} per_client_min={per_client_min} \
         per_client_floor={floor} overlap_ms={overlap_ms} exit={exit}{cleanup}"
    );
    echo("stdout", &stdout);
    echo("stderr", &stderr);
    println!("{line}");
    assert!(class == "PASS" || class == "NOT-RUN reason=no-reproduce", "{line}");
}
