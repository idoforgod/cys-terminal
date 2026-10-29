// 0.14.56-SOCKET-P8 #12: an attach is released only when its surface is gone (reader EOF, or no longer
// this Arc in the daemon map) AND its pending write made no progress for ATTACH_GONE_STALL since that
// write call started. A live surface, or a client that reads, is never cut (design r2 §2-3, §2-4).
// The tests keep only a Weak<Surface>; output that a specimen needs live is released by a gate file
// that the test creates after reading the ack, so it cannot happen before the subscription.
use super::*;
use std::time::{Duration, Instant};
/// Hang guard for waits on an event (task end, EOF, exit): it ends a hang, it never decides a pass.
const ATT_GUARD: Duration = Duration::from_secs(60);
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

struct AttDir(std::path::PathBuf);
impl Drop for AttDir {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
}

fn att_daemon(tag: &str) -> (AttDir, Arc<Daemon>) {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("cys-att-{tag}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let d = Daemon::new(dir.join("cysd.sock"));
    (AttDir(dir), d)
}

/// A 256-byte in-memory duplex: any snapshot or burst larger than that blocks the attach write.
fn att_conn(daemon: &Arc<Daemon>) -> (tokio::io::DuplexStream, tokio::task::JoinHandle<()>) {
    let (client, server) = tokio::io::duplex(256);
    let task = tokio::spawn(handle_connection_capped(
        Arc::clone(daemon), Box::new(server) as Stream, None,
        Some(Duration::from_secs(60)), Some(Duration::from_secs(30)),
    ));
    (client, task)
}

fn att_request(id: u64) -> String {
    format!("{{\"id\":1,\"method\":\"surface.attach\",\"params\":{{\"surface_id\":{id}}}}}\n")
}

/// Creates the surface and returns its id and a Weak (the test drops its own Arc at once).
fn att_surface(daemon: &Arc<Daemon>, cmd: String) -> (u64, std::sync::Weak<crate::state::Surface>) {
    let s = daemon.create_surface(None, Some(cmd), None, None, 24, 80).expect("create surface");
    (s.id, Arc::downgrade(&s))
}

fn att_exited(weak: &std::sync::Weak<crate::state::Surface>) -> Option<bool> {
    weak.upgrade().map(|s| s.exited.load(std::sync::atomic::Ordering::Relaxed))
}

async fn att_wait_until(limit: Duration, mut cond: impl FnMut() -> bool) -> bool {
    let t0 = Instant::now();
    while t0.elapsed() < limit {
        if cond() { return true; }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    cond()
}

/// Sends the attach request and reads the ack line only (the reader then stalls or is handed on).
async fn att_attach(client: tokio::io::DuplexStream, id: u64)
    -> (BufReader<tokio::io::ReadHalf<tokio::io::DuplexStream>>, tokio::io::WriteHalf<tokio::io::DuplexStream>) {
    let (rd, mut wr) = tokio::io::split(client);
    wr.write_all(att_request(id).as_bytes()).await.unwrap();
    let mut rd = BufReader::new(rd);
    let mut ack = String::new();
    tokio::time::timeout(Duration::from_secs(5), rd.read_line(&mut ack)).await.expect("no ack").unwrap();
    let v: serde_json::Value = serde_json::from_str(ack.trim()).expect("ack is JSON");
    assert_eq!(v["ok"], true, "attach refused: {ack}");
    (rd, wr)
}

/// Reads until EOF in `chunk`-byte reads with `pace` between them (a slow but steadily reading client).
fn att_read_paced(mut rd: BufReader<tokio::io::ReadHalf<tokio::io::DuplexStream>>, chunk: usize, pace: Duration)
    -> tokio::task::JoinHandle<Vec<u8>> {
    tokio::spawn(async move {
        let mut out = Vec::new();
        let mut b = vec![0u8; chunk];
        loop {
            match rd.read(&mut b).await { Ok(0) | Err(_) => break, Ok(n) => out.extend_from_slice(&b[..n]) }
            tokio::time::sleep(pace).await;
        }
        out
    })
}

/// Reads everything until EOF; returns the bytes.
fn att_read_all(mut rd: BufReader<tokio::io::ReadHalf<tokio::io::DuplexStream>>) -> tokio::task::JoinHandle<Vec<u8>> {
    tokio::spawn(async move {
        let mut out = Vec::new();
        let mut b = vec![0u8; 8192];
        loop {
            match rd.read(&mut b).await { Ok(0) | Err(_) => break, Ok(n) => out.extend_from_slice(&b[..n]) }
        }
        out
    })
}

fn att_gate(dir: &AttDir, name: &str) -> std::path::PathBuf { dir.0.join(name) }

#[cfg(unix)]
fn att_pid_alive(pid: i32) -> bool { unsafe { libc::kill(pid, 0) == 0 } }
#[cfg(unix)]
fn att_kill(pid: i32) { unsafe { libc::kill(pid, libc::SIGKILL); } }
#[cfg(not(unix))]
fn att_pid_alive(_pid: i32) -> bool { false }
#[cfg(not(unix))]
fn att_kill(_pid: i32) {}

/// The state a ConPTY close (and a PTY held open after close on platforms that keep it) leaves:
/// gone from the daemon map (governance.rs `close_surface_effect` `surfaces.remove(&id)`), child still
/// running, no reader EOF. Built directly because on macOS a session leader's death gives the master
/// EOF even while a re-parented holder keeps the slave (measured, revoke_probe.py) - a real holder
/// cannot construct this state here.
fn att_remove_from_map_keep_child(daemon: &Arc<Daemon>, id: u64) -> i32 {
    let s = daemon.surfaces.lock().unwrap().remove(&id).expect("surface in the map");
    s.pid as i32
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[cfg_attr(not(unix), ignore = "윈도우 ConPTY 는 자식이 끝나도 출력 파이프를 닫지 않는다 — reader EOF 로 좌석 종료를 아는 경로는 유닉스 전제")]
async fn s12_1_stalled_attach_is_released_after_a_natural_exit() {
    let (dir, daemon) = att_daemon("s121");
    let gate = att_gate(&dir, "go");
    let (id, weak) = att_surface(&daemon, format!(
        "while [ ! -e '{g}' ]; do sleep 0.05; done; head -c 200000 /dev/zero | tr '\\0' a; exit 0", g = gate.display()));
    let (client, mut task) = att_conn(&daemon);
    let (_stalled_rd, _wr) = att_attach(client, id).await;
    std::fs::write(&gate, b"1").unwrap();
    assert!(att_wait_until(ATT_GUARD, || att_exited(&weak) == Some(true)).await, "child did not exit");
    let t_exit = Instant::now();
    let ended = tokio::time::timeout(ATT_GUARD, &mut task).await.is_ok(); // wallclock: guard attach task end (old bytes never end; release is the product's 5 s rule)
    assert!(ended, "attach not released after the surface exited (stalled client)");
    // The surface stays in the map (no watchdog here); the attach's own Arc must be gone.
    assert!(att_wait_until(ATT_GUARD, || weak.strong_count() <= 1).await,
        "attach still holds the exited surface: strong count {}", weak.strong_count());
    eprintln!("S12-1 released {} ms after the exit", t_exit.elapsed().as_millis());
    let _ = crate::governance::close_surface(&daemon, id, crate::governance::CloseCause::OwnerClose);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[cfg_attr(not(unix), ignore = "좌석 명령이 POSIX 셸 문법")]
async fn s12_2_stalled_snapshot_write_is_released_after_close() {
    let (_dir, daemon) = att_daemon("s122");
    let (id, weak) = att_surface(&daemon,
        "i=0; while [ $i -lt 30 ]; do echo xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx$i; i=$((i+1)); done; sleep 30".into());
    let filled = att_wait_until(ATT_GUARD, || {
        weak.upgrade().is_some_and(|s| s.parser.lock().unwrap_or_else(|e| e.into_inner()).screen().contents_formatted().len() > 1024)
    }).await;
    assert!(filled, "instrument: the screen never filled, so the snapshot cannot exceed the 256 B duplex");
    let (client, mut task) = att_conn(&daemon);
    // Read the ack only (the attach has started); the snapshot behind it exceeds the 256 B duplex and blocks.
    let (_stalled_rd, _wr) = att_attach(client, id).await;
    assert!(!task.is_finished(), "instrument: the attach ended before the close");
    crate::governance::close_surface(&daemon, id, crate::governance::CloseCause::OwnerClose).expect("close");
    let t_close = Instant::now();
    let ended = tokio::time::timeout(ATT_GUARD, &mut task).await.is_ok(); // wallclock: guard attach task end (old bytes never end; release is the product's 5 s rule)
    assert!(ended, "attach not released after the close (stalled snapshot write)");
    assert!(att_wait_until(ATT_GUARD, || weak.upgrade().is_none()).await, "closed surface still allocated");
    eprintln!("S12-2 released {} ms after the close", t_close.elapsed().as_millis());
}

/// A writer that never accepts a byte: a client that stopped reading.
struct AttNeverWrites;
impl tokio::io::AsyncWrite for AttNeverWrites {
    fn poll_write(self: std::pin::Pin<&mut Self>, _: &mut std::task::Context<'_>, _: &[u8])
        -> std::task::Poll<std::io::Result<usize>> { std::task::Poll::Pending }
    fn poll_flush(self: std::pin::Pin<&mut Self>, _: &mut std::task::Context<'_>)
        -> std::task::Poll<std::io::Result<()>> { std::task::Poll::Ready(Ok(())) }
    fn poll_shutdown(self: std::pin::Pin<&mut Self>, _: &mut std::task::Context<'_>)
        -> std::task::Poll<std::io::Result<()>> { std::task::Poll::Ready(Ok(())) }
}

/// Risk ④ pin, event-based: a stalled write is never abandoned while the surface is live. The test
/// waits for the guard to have asked "gone?" twice (and been told no), or for the write to end — an
/// event, not a duration. The abandonment check comes first, so a guard that cuts without asking is
/// caught there and not by the instrument line.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn s12_3_a_stalled_write_on_a_live_surface_is_never_abandoned() {
    let asked = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let counter = Arc::clone(&asked);
    let write = tokio::spawn(async move {
        let mut w = AttNeverWrites;
        write_attach_bytes(&mut w, b"stalled", move || { counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst); false }).await
    });
    let settled = att_wait_until(ATT_GUARD, || {
        asked.load(std::sync::atomic::Ordering::SeqCst) >= 2 || write.is_finished()
    }).await;
    assert!(!write.is_finished(), "a stalled write on a live surface was abandoned");
    assert!(settled, "instrument: the guard never asked whether the surface is gone");
    write.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[cfg_attr(not(unix), ignore = "좌석 명령이 POSIX 셸 문법")]
async fn s12_a_reading_client_gets_the_whole_final_burst() {
    let (dir, daemon) = att_daemon("s12a");
    let gate = att_gate(&dir, "go");
    let (id, weak) = att_surface(&daemon, format!(
        "while [ ! -e '{g}' ]; do sleep 0.05; done; head -c 32768 /dev/zero | tr '\\0' a; printf 'W30-S12A-END\\n'; exit 0",
        g = gate.display()));
    let (client, mut task) = att_conn(&daemon);
    let (rd, _wr) = att_attach(client, id).await;
    // Reads 256 B per ms: always progressing, but slower than the PTY, so the attach is still
    // writing when the child has exited (the case gemini1 M1 named).
    let reader = att_read_paced(rd, 256, Duration::from_millis(1));
    tokio::time::sleep(Duration::from_secs(6)).await; // live longer than ATTACH_GONE_STALL: a stale clock would be due
    std::fs::write(&gate, b"1").unwrap();
    let got = tokio::time::timeout(ATT_GUARD, reader).await.expect("reader did not reach EOF").unwrap(); // wallclock: aging the 6 s sleep above is an aging premise so a stale-clock regression (r1 M1; mutants M12B/M12D) is due; the verdict on correct code does not depend on it
    let a = got.iter().filter(|&&b| b == b'a').count();
    let end = got.windows(12).any(|w| w == b"W30-S12A-END");
    assert!(a >= 32768 && end, "a reading client lost the final burst: {a} of 32768 bytes, end marker {end}");
    assert_eq!(att_exited(&weak), Some(true), "instrument: the surface did not exit");
    assert!(tokio::time::timeout(ATT_GUARD, &mut task).await.is_ok());
    let _ = crate::governance::close_surface(&daemon, id, crate::governance::CloseCause::OwnerClose);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[cfg_attr(not(unix), ignore = "좌석 명령이 POSIX 셸 문법")]
async fn s12_b_surface_gone_from_the_map_without_eof_releases_a_stalled_attach() {
    let (_dir, daemon) = att_daemon("s12b");
    let (id, weak) = att_surface(&daemon, "while :; do echo w30-s12-b-line-xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx; done".into());
    let (client, task) = att_conn(&daemon);
    let (_stalled_rd, _wr) = att_attach(client, id).await;
    let live = weak.upgrade().expect("surface alive");
    assert!(!attach_surface_gone(&daemon, &live), "a live surface reads as gone");
    let child = att_remove_from_map_keep_child(&daemon, id);
    assert!(attach_surface_gone(&daemon, &live), "a surface removed from the map does not read as gone");
    drop(live);
    let t_gone = Instant::now();
    let mut exited_at_release = None;
    let ended = loop { // wallclock: guard attach task end (old bytes never end; release is the product's 5 s rule)
        if task.is_finished() { break true; }
        if t_gone.elapsed() > ATT_GUARD { break false; }
        if let Some(e) = att_exited(&weak) { exited_at_release = Some(e); }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    let released_ms = t_gone.elapsed().as_millis();
    let child_alive = att_pid_alive(child);
    // The PTY reader thread keeps its own Arc until reader EOF: the attach's Arc must be the one gone.
    let attach_arc_gone = att_wait_until(ATT_GUARD, || weak.strong_count() <= 1).await;
    att_kill(child);
    let freed_after_child = att_wait_until(ATT_GUARD, || weak.upgrade().is_none()).await;
    assert!(ended, "attach not released after its surface left the map (stalled client, no EOF)");
    assert_eq!(exited_at_release, Some(false), "instrument: reader EOF came, so this did not test the map-removal path");
    assert!(child_alive, "instrument: the child died, so reader EOF could have released the attach");
    assert!(attach_arc_gone, "the released attach still holds the surface: strong count {}", weak.strong_count());
    assert!(freed_after_child, "surface not freed after its child died (reader EOF)");
    eprintln!("S12-B released {released_ms} ms after the map removal");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[cfg_attr(not(unix), ignore = "좌석 명령이 POSIX 셸 문법")]
async fn s12_c_reading_client_of_a_surface_gone_from_the_map_reaches_eof() {
    let (_dir, daemon) = att_daemon("s12c");
    // The child outlives ATT_GUARD (the test kills it), so EOF can only come from the map-removal path.
    let (id, weak) = att_surface(&daemon, "echo w30-s12-c-hello; exec sleep 600".into());
    let (client, mut task) = att_conn(&daemon);
    let (rd, _wr) = att_attach(client, id).await;
    let reader = att_read_all(rd);
    let child = att_remove_from_map_keep_child(&daemon, id);
    let t_gone = Instant::now();
    let mut exited_seen = None;
    let eof = loop { // wallclock: guard reader EOF (old bytes never send it; the product ends a gone, quiet stream after 1 s)
        if reader.is_finished() { break true; }
        if t_gone.elapsed() > ATT_GUARD { break false; }
        if let Some(e) = att_exited(&weak) { exited_seen = Some(e); }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    let child_alive = att_pid_alive(child);
    att_kill(child);
    assert!(eof, "a reading client of a surface gone from the map did not reach EOF");
    assert_eq!(exited_seen, Some(false), "instrument: reader EOF came, so this did not test the map-removal path");
    assert!(child_alive, "instrument: the child died, so reader EOF could have ended the stream");
    assert!(tokio::time::timeout(ATT_GUARD, &mut task).await.is_ok());
    eprintln!("S12-C reached EOF {} ms after the map removal", t_gone.elapsed().as_millis());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[cfg_attr(not(unix), ignore = "좌석 명령이 POSIX 셸 문법")]
async fn s12_4_reading_client_ends_after_a_natural_exit() {
    let (dir, daemon) = att_daemon("s124");
    let gate = att_gate(&dir, "go");
    let (id, weak) = att_surface(&daemon, format!(
        "while [ ! -e '{g}' ]; do sleep 0.05; done; echo w30-s12-4-bye; exit 0", g = gate.display()));
    let (client, mut task) = att_conn(&daemon);
    let (rd, _wr) = att_attach(client, id).await;
    let reader = att_read_all(rd);
    std::fs::write(&gate, b"1").unwrap();
    assert!(att_wait_until(ATT_GUARD, || att_exited(&weak) == Some(true)).await, "child did not exit");
    let got = tokio::time::timeout(ATT_GUARD, reader).await.expect("reader did not end after the exit").unwrap(); // wallclock: guard reader EOF after the exit
    assert!(got.windows(12).any(|w| w == b"w30-s12-4-by"), "the exit output was not delivered");
    assert!(tokio::time::timeout(ATT_GUARD, &mut task).await.is_ok());
    let _ = crate::governance::close_surface(&daemon, id, crate::governance::CloseCause::OwnerClose);
}

/// Windows ConPTY close path (design r2 §2-4 S12-BW, §9): a stalled attach on a closed seat is
/// released by map removal. Whether the seat is then freed is printed, not asserted (see the note
/// inside). UNMEASURED until a Windows run executes it (non-blocking cysd_win step).
#[cfg(windows)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn s12_bw_windows_closed_seat_releases_a_stalled_attach_then_reaches_eof() {
    let (_dir, daemon) = att_daemon("s12bw");
    // Floods its output, so the 256 B duplex is full long before the close without any sleep.
    let (id, weak) = att_surface(&daemon, "cmd /c \"for /l %i in (0,0,1) do @echo w30-s12-bw-line-xxxxxxxxxxxxxxxxxxxxxxxx\"".into());
    let (client, mut task) = att_conn(&daemon);
    let (_stalled_rd, _wr) = att_attach(client, id).await;
    crate::governance::close_surface(&daemon, id, crate::governance::CloseCause::OwnerClose).expect("close");
    let t_close = Instant::now();
    let ended = tokio::time::timeout(ATT_GUARD, &mut task).await.is_ok(); // wallclock: guard attach task end (old bytes never end; release is the product's 5 s rule)
    assert!(ended, "attach not released after closing a Windows seat (stalled client)");
    let released_ms = t_close.elapsed().as_millis();
    let attach_arc_gone = att_wait_until(ATT_GUARD, || weak.strong_count() <= 1).await;
    // Measured, not asserted: whether the seat itself is freed. The PTY reader thread keeps its own
    // Arc until reader EOF, and on ConPTY that EOF needs the master to drop (implementation finding,
    // reported to the dept-5 head) - so a closed seat may stay allocated whatever the attach does.
    let freed = att_wait_until(Duration::from_secs(10), || weak.upgrade().is_none()).await;
    eprintln!("S12-BW released {released_ms} ms after the close; attach Arc gone {attach_arc_gone}; \
               seat freed within 10 s more: {freed}; strong count now {}", weak.strong_count());
    assert!(attach_arc_gone, "the released attach still holds the closed Windows seat");
}
