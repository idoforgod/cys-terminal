//! The REPLAY RULES (CONTRACTS C4 — RULINGS BH2 (a)(b), BK1, BL1; design D:783–784), shared by the
//! rehearsal runner (`rehearsal.rs`, C5 step 3) and the protection screen (`screen.rs`). Track T5b.
//!
//! Every run that executes candidate code obeys three rules:
//! (a) one scratch root per run — HOME, the laid-out pack and every path the run is given lie under
//!     it; `CLAUDE_CONFIG_DIR` and every `CYS_*` path variable are unset or point inside it;
//! (b) the only `cys` a run can call is the screen's own stub (a script compiled into this binary,
//!     never a pack file) — it never talks to or wakes a real daemon;
//! (c) the SCOPED after-run comparison (BK1 / BL1) — the lane's live listed files, the `hooks` /
//!     `statusLine` sections of every real settings file a registrar could target, and the
//!     processes the run started.
//! ★No daemon is ever started here, and no real socket is ever named (appendix A 1-1, 1-2).
//! ★Nothing here may panic on a run's output (F4 — the screen runs inside the daemon).

use super::types::{RepairError, TargetState};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The screen's own stub `cys` (C4 E-1 … E-3, I3). POSIX `sh`. It reads its fixed replies from
/// `cys.conf` beside itself (written by the screen — never by a probe), logs every call to
/// `cys-calls.log` beside itself, and behaves like the real binary WITHOUT a daemon (exit 1,
/// nothing started) when the socket it is given does not exist as a socket — so a probe in E-2 sees
/// exactly what `cys` answers under `CYS_NO_AUTOSTART=1` with no daemon.
pub const STUB_CYS: &str = r#"#!/bin/sh
# cys stub of the 「스스로 해결」 protection screen / rehearsal (compiled into cys; never a pack file).
d=$(dirname "$0")
STUB_ROLE=""
STUB_STATUS="unknown"
[ -f "$d/cys.conf" ] && . "$d/cys.conf"
printf '%s\n' "cys $*" >> "$d/cys-calls.log" 2>/dev/null
sock="${CYS_SOCKET:-}"
while [ $# -gt 0 ]; do
  case "$1" in
    --socket|-S) sock="${2:-}"; shift; [ $# -gt 0 ] && shift ;;
    --socket=*) sock="${1#--socket=}"; shift ;;
    *) break ;;
  esac
done
[ -n "$sock" ] && [ -S "$sock" ] || exit 1
case "${1:-}" in
  surface-role)
    [ -n "$STUB_ROLE" ] && printf '%s\n' "$STUB_ROLE"
    exit 0 ;;
  status)
    if [ "$STUB_STATUS" = "holds" ]; then
      printf '%s\n' '{"alert_route": {"enabled": true}}'
      exit 0
    fi
    exit 1 ;;
  *) exit 1 ;;
esac
"#;

/// Minimal wall-time any single child may get (a run with less left is not started).
pub const MIN_CHILD_SLICE: Duration = Duration::from_millis(200);

/// Unix socket paths must stay under `sun_path` (104 bytes on macOS); we keep a margin.
const MAX_SOCKET_PATH: usize = 100;

/// A scratch root of one run (BH2 (a)). Removed on drop (permissions restored first, links never
/// followed).
#[derive(Debug)]
pub struct ScratchRoot {
    path: PathBuf,
}

impl ScratchRoot {
    /// Creates `<base>/<prefix><16 hex>` (mode 0700). `base` is created if missing.
    pub fn new(base: &Path, prefix: &str) -> Result<Self, RepairError> {
        std::fs::create_dir_all(base)
            .map_err(|e| RepairError::Io(format!("{}: {e}", base.display())))?;
        for _ in 0..16 {
            let p = base.join(format!("{prefix}{}", random_hex(8)));
            match std::fs::create_dir(&p) {
                Ok(()) => {
                    set_mode(&p, 0o700);
                    return Ok(Self { path: p });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(RepairError::Io(format!("{}: {e}", p.display()))),
            }
        }
        Err(RepairError::Io(format!(
            "{}: could not create a fresh scratch root",
            base.display()
        )))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for ScratchRoot {
    fn drop(&mut self) {
        remove_tree(&self.path);
    }
}

/// Removes a tree without following links; directories made read-only by a run (E-17) are made
/// writable again first. Errors are swallowed (a leftover scratch folder is not a failure of the
/// verdict; it is inside the scratch base).
pub fn remove_tree(p: &Path) {
    let Ok(meta) = std::fs::symlink_metadata(p) else {
        return;
    };
    if meta.file_type().is_dir() {
        set_mode(p, 0o700);
        if let Ok(rd) = std::fs::read_dir(p) {
            for e in rd.flatten() {
                remove_tree(&e.path());
            }
        }
        let _ = std::fs::remove_dir(p);
    } else {
        let _ = std::fs::remove_file(p);
    }
}

#[cfg(unix)]
pub fn set_mode(p: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(p, std::fs::Permissions::from_mode(mode));
}

#[cfg(not(unix))]
pub fn set_mode(_p: &Path, _mode: u32) {}

/// `n` random bytes as lowercase hex (from the OS — `/dev/urandom`; falls back to time + pid
/// mixing only when that cannot be read).
pub fn random_hex(n: usize) -> String {
    use std::io::Read;
    let mut buf = vec![0u8; n];
    let ok = std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut buf))
        .is_ok();
    if !ok {
        use sha2::{Digest, Sha256};
        let seed = format!(
            "{:?}{}{:?}",
            std::time::SystemTime::now(),
            std::process::id(),
            std::thread::current().id()
        );
        let h = Sha256::digest(seed.as_bytes());
        for (i, b) in buf.iter_mut().enumerate() {
            *b = h[i % h.len()];
        }
    }
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

/// Writes the stub `cys` (+ its config) into `dir` (created). Returns the stub's path.
pub fn write_stub(dir: &Path, role: &str, status_holds: bool) -> Result<PathBuf, RepairError> {
    std::fs::create_dir_all(dir).map_err(|e| RepairError::Io(format!("{}: {e}", dir.display())))?;
    let stub = dir.join("cys");
    std::fs::write(&stub, STUB_CYS)
        .map_err(|e| RepairError::Io(format!("{}: {e}", stub.display())))?;
    set_mode(&stub, 0o755);
    write_stub_conf(dir, role, status_holds)?;
    Ok(stub)
}

/// (Re)writes the stub's fixed replies. `role` must be a plain role token.
pub fn write_stub_conf(dir: &Path, role: &str, status_holds: bool) -> Result<(), RepairError> {
    if !role
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(RepairError::Precondition(format!(
            "stub role is not a plain token: {role:?}"
        )));
    }
    let conf = format!(
        "STUB_ROLE='{role}'\nSTUB_STATUS='{}'\n",
        if status_holds { "holds" } else { "unknown" }
    );
    std::fs::write(dir.join("cys.conf"), conf)
        .map_err(|e| RepairError::Io(format!("{}: {e}", dir.display())))
}

/// The lines the stub logged (one per call).
pub fn stub_calls(dir: &Path) -> Vec<String> {
    std::fs::read_to_string(dir.join("cys-calls.log"))
        .map(|t| t.lines().map(str::to_string).collect())
        .unwrap_or_default()
}

/// A unix socket the screen binds in its scratch folder (E-1, E-3, I3's daemon axis) — never the
/// live lane socket. Nothing ever connects to it: it exists so that the devices' "is a socket"
/// checks see a daemon. Removed on drop.
#[derive(Debug)]
pub struct ScratchSocket {
    path: PathBuf,
    // A private folder for the socket when the scratch root's own path is too long for `sun_path`.
    short_dir: Option<PathBuf>,
    #[cfg(unix)]
    _listener: Option<std::os::unix::net::UnixListener>,
}

impl ScratchSocket {
    /// Binds `<root>/s`; when that path is too long for a unix socket, a private mode-0700 folder
    /// under the system temporary folder holds it instead (removed on drop).
    pub fn bind(root: &Path) -> Result<Self, RepairError> {
        #[cfg(unix)]
        {
            let mut short_dir = None;
            let mut path = root.join("s");
            if path.as_os_str().len() > MAX_SOCKET_PATH {
                let tmp = if cfg!(target_os = "macos") {
                    "/private/tmp"
                } else {
                    "/tmp"
                };
                let d = ScratchRoot::new(Path::new(tmp), "cys-rs-")?;
                // keep the folder: ScratchRoot's drop would remove it now; we own it from here
                let p = d.path().to_path_buf();
                std::mem::forget(d);
                path = p.join("s");
                short_dir = Some(p);
            }
            let l = std::os::unix::net::UnixListener::bind(&path)
                .map_err(|e| RepairError::Io(format!("{}: {e}", path.display())))?;
            Ok(Self {
                path,
                short_dir,
                _listener: Some(l),
            })
        }
        #[cfg(not(unix))]
        {
            let _ = root;
            Err(RepairError::PlatformPending("replay socket"))
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for ScratchSocket {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        if let Some(d) = &self.short_dir {
            remove_tree(d);
        }
    }
}

/// What a child did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChildOutcome {
    /// Exit code when it exited normally.
    pub code: Option<i32>,
    /// The signal that ended it (incl. our own kill at the deadline).
    pub signal: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub timed_out: bool,
    /// The child could not be started at all.
    pub spawn_error: Option<String>,
    pub elapsed: Duration,
    /// The child's process group still had a member after the run's group was killed.
    pub leftovers: Vec<u32>,
}

/// Largest output kept per stream (the rest is dropped; a probe's verdict never needs more).
const MAX_CAPTURE: usize = 1 << 20;

/// Runs `argv` with EXACTLY `env` (nothing inherited — the fixed clean environment, BA1), in `cwd`,
/// with `stdin` as input, in its own process group, killed (the whole group) and reaped at
/// `deadline`. After a normal end the group is killed as well, so no process of the run survives it
/// (replay rule (c)(3)).
#[cfg(unix)]
pub fn run_child(
    argv: &[String],
    env: &[(String, String)],
    cwd: &Path,
    stdin: &[u8],
    deadline: Duration,
) -> ChildOutcome {
    use std::io::{Read, Write};
    use std::os::unix::process::{CommandExt, ExitStatusExt};
    use std::process::{Command, Stdio};

    let started = Instant::now();
    let mut out = ChildOutcome::default();
    let Some((prog, args)) = argv.split_first() else {
        out.spawn_error = Some("empty argv".into());
        return out;
    };
    let mut cmd = Command::new(prog);
    cmd.args(args)
        .env_clear()
        .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            out.spawn_error = Some(e.to_string());
            out.elapsed = started.elapsed();
            return out;
        }
    };
    let pid = child.id() as libc::pid_t;
    // stdin on its own thread (a child that never reads must not block us)
    let input = stdin.to_vec();
    let stdin_pipe = child.stdin.take();
    let w = std::thread::spawn(move || {
        if let Some(mut s) = stdin_pipe {
            let _ = s.write_all(&input);
        }
    });
    let reader = |p: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut r) = p {
                let mut chunk = [0u8; 8192];
                loop {
                    match r.read(&mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            if buf.len() < MAX_CAPTURE {
                                let take = n.min(MAX_CAPTURE - buf.len());
                                buf.extend_from_slice(&chunk[..take]);
                            }
                        }
                    }
                }
            }
            buf
        })
    };
    let ro = reader(
        child
            .stdout
            .take()
            .map(|s| Box::new(s) as Box<dyn Read + Send>),
    );
    let re = reader(
        child
            .stderr
            .take()
            .map(|s| Box::new(s) as Box<dyn Read + Send>),
    );
    // wait without reaping (WNOWAIT) so the group can be killed while its id is still ours
    let mut sleep = Duration::from_millis(1);
    loop {
        if exited_no_reap(pid) {
            break;
        }
        if started.elapsed() >= deadline {
            out.timed_out = true;
            break;
        }
        std::thread::sleep(sleep);
        sleep = (sleep * 2).min(Duration::from_millis(20));
    }
    // SAFETY: kill(2) on our own child's process group (pgid = the child's pid, set by
    // process_group(0)); the leader is not reaped yet, so the group id cannot have been reused.
    unsafe {
        libc::kill(-pid, libc::SIGKILL);
    }
    let status = child.wait();
    // replay rule (c)(3): nothing of the group may outlive the run — a member still there after
    // the kill is recorded (signal 0 only checks existence)
    // SAFETY: kill(2) with signal 0 sends nothing.
    if unsafe { libc::kill(-pid, 0) } == 0 {
        std::thread::sleep(Duration::from_millis(50));
        // SAFETY: as above.
        if unsafe { libc::kill(-pid, 0) } == 0 {
            out.leftovers.push(pid as u32);
        }
    }
    let _ = w.join();
    out.stdout = join_bounded(ro, Duration::from_secs(2));
    out.stderr = join_bounded(re, Duration::from_secs(2));
    match status {
        Ok(st) => {
            out.code = st.code();
            out.signal = st.signal();
        }
        Err(e) => out.spawn_error = Some(format!("wait: {e}")),
    }
    if out.timed_out {
        out.code = None;
    }
    out.elapsed = started.elapsed();
    out
}

#[cfg(not(unix))]
pub fn run_child(
    argv: &[String],
    env: &[(String, String)],
    cwd: &Path,
    stdin: &[u8],
    deadline: Duration,
) -> ChildOutcome {
    let _ = (argv, env, cwd, stdin, deadline);
    ChildOutcome {
        spawn_error: Some("replay runs are not built for this platform in release 1".into()),
        ..Default::default()
    }
}

fn join_bounded(h: std::thread::JoinHandle<Vec<u8>>, wait: Duration) -> Vec<u8> {
    let t0 = Instant::now();
    while !h.is_finished() {
        if t0.elapsed() >= wait {
            // a process outside our group still holds the pipe: give up on the rest of the output
            return Vec::new();
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    h.join().unwrap_or_default()
}

/// True when the child `pid` has ended (not reaped — `WNOWAIT`).
#[cfg(unix)]
fn exited_no_reap(pid: libc::pid_t) -> bool {
    // SAFETY: plain waitid(2) on our own child with WNOWAIT|WNOHANG; `info` is zeroed and only read
    // through the libc accessor when the call reports a child.
    unsafe {
        let mut info: libc::siginfo_t = std::mem::zeroed();
        let r = libc::waitid(
            libc::P_PID,
            pid as libc::id_t,
            &mut info,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        );
        r == 0 && info.si_pid() == pid
    }
}

/// The variables of the fixed clean environment that every run gets (BA1): only what an ordinary
/// seat has. The caller adds the cell's own variables. Nothing ambient is copied.
pub fn base_env(home: &Path, tmpdir: &Path, path_dirs: &[PathBuf]) -> Vec<(String, String)> {
    let user = std::env::var("USER").unwrap_or_else(|_| "user".into());
    let path = path_dirs
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .chain(["/usr/bin", "/bin", "/usr/sbin", "/sbin"].map(String::from))
        .collect::<Vec<_>>()
        .join(":");
    vec![
        ("HOME".into(), home.to_string_lossy().into_owned()),
        ("TMPDIR".into(), format!("{}/", tmpdir.to_string_lossy())),
        ("PATH".into(), path),
        ("USER".into(), user.clone()),
        ("LOGNAME".into(), user),
        ("SHELL".into(), "/bin/sh".into()),
        ("LANG".into(), "en_US.UTF-8".into()),
        ("LC_ALL".into(), "en_US.UTF-8".into()),
        // an ordinary seat inherits the daemon's bytecode seal (SEAL-1 — `seal_python_bytecode_in_process`):
        // no replay may leave a `__pycache__` in a laid-out pack
        ("PYTHONDONTWRITEBYTECODE".into(), "1".into()),
    ]
}

/// A private folder `<root>/py` holding `python3` / `python` links to the interpreter, so that the
/// interpreter can be put on PATH without the rest of its folder (which might hold a real `cys`).
#[cfg(unix)]
pub fn interpreter_dir(root: &Path, py: &Path) -> Result<PathBuf, RepairError> {
    let d = root.join("py");
    std::fs::create_dir_all(&d).map_err(|e| RepairError::Io(format!("{}: {e}", d.display())))?;
    for n in ["python3", "python"] {
        let l = d.join(n);
        if std::fs::symlink_metadata(&l).is_err() {
            std::os::unix::fs::symlink(py, &l)
                .map_err(|e| RepairError::Io(format!("{}: {e}", l.display())))?;
        }
    }
    Ok(d)
}

#[cfg(not(unix))]
pub fn interpreter_dir(root: &Path, py: &Path) -> Result<PathBuf, RepairError> {
    let _ = (root, py);
    Err(RepairError::PlatformPending("replay interpreter"))
}

/// The bundled runtime the daemon gives panes (`CYS_PY`), else the first `python3` on this
/// process's PATH (absolute). `None` = no interpreter (every replay is then not measurable).
pub fn bundled_interpreter() -> Option<PathBuf> {
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
    {
        if let Some(p) = crate::macos_devtools::bundled_python3_path(&dir) {
            return Some(p);
        }
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join("python3"))
        .find(|p| p.is_file())
}

/// The system folders of the fixed PATH must hold no `cys` (C4 E-3: "AN4's fixed list holds no
/// `cys`"). `Err` names the offending path.
pub fn system_path_has_no_cys() -> Result<(), RepairError> {
    for d in ["/usr/bin", "/bin", "/usr/sbin", "/sbin"] {
        let p = Path::new(d).join("cys");
        if std::fs::symlink_metadata(&p).is_ok() {
            return Err(RepairError::Precondition(format!(
                "{}: 시스템 경로에 cys 가 있어 재생 환경을 격리할 수 없습니다",
                p.display()
            )));
        }
    }
    Ok(())
}

// ───────────────────────── the scoped after-run comparison (BK1, BL1) ─────────────────────────

/// One compared item: its state and its change stamp (inode, ctime, mtime) — BL1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveItem {
    pub item: String,
    pub state: String,
    pub stamp: String,
}

/// The real settings files a registrar could target from this lane (BK1 (2)): every
/// `<HOME>/.claude` / `<HOME>/.claude-*` profile's `settings.json`, the account folder
/// `<lane parent>/claude/settings.json`, and for a department lane (`pack-dept-<x>`) the
/// department account folders `<lane parent>/accounts/*dept*/settings.json` that exist.
pub fn real_settings_targets(lane: &Path, home: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(home) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if n == ".claude" || n.starts_with(".claude-") {
                out.push(e.path().join("settings.json"));
            }
        }
    }
    if let Some(parent) = lane.parent() {
        out.push(parent.join("claude").join("settings.json"));
        let lane_name = lane
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if lane_name.starts_with("pack-dept-") {
            if let Ok(rd) = std::fs::read_dir(parent.join("accounts")) {
                for e in rd.flatten() {
                    if e.file_name().to_string_lossy().contains("dept") {
                        out.push(e.path().join("settings.json"));
                    }
                }
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// The change stamp of a compared item (BL1): inode + modification time. A write — the same bytes
/// included — moves the modification time; a replacement moves the inode; a mode change moves the
/// state. The inode-change time (ctime) is NOT part of it: system services change it without any
/// write (measured: the iCloud file provider re-stamped every file of a Desktop-backed test folder
/// seconds after it was created — every candidate would then have been "written again").
#[cfg(unix)]
fn stamp_of(p: &Path) -> String {
    use std::os::unix::fs::MetadataExt;
    match std::fs::symlink_metadata(p) {
        Ok(m) => format!("ino={} mtime={}.{}", m.ino(), m.mtime(), m.mtime_nsec()),
        Err(_) => "absent".into(),
    }
}

#[cfg(not(unix))]
fn stamp_of(p: &Path) -> String {
    match std::fs::symlink_metadata(p) {
        Ok(m) => format!("len={} mtime={:?}", m.len(), m.modified().ok()),
        Err(_) => "absent".into(),
    }
}

/// The compared items now: the lane's live files on either compiled list, and the `hooks` /
/// `statusLine` sections of each real settings file.
pub fn live_snapshot(lane: &Path, settings: &[PathBuf]) -> Vec<LiveItem> {
    use super::boot_critical::{entries, protection_entries};
    let mut rels: Vec<&str> = entries()
        .iter()
        .chain(protection_entries().iter())
        .map(|e| e.rel)
        .collect();
    rels.sort();
    rels.dedup();
    let mut out = Vec::new();
    for rel in rels {
        let p = lane.join(rel);
        let state = TargetState::read(&p)
            .map(|s| s.canon())
            .unwrap_or_else(|e| format!("unreadable: {e:?}"));
        out.push(LiveItem {
            item: p.to_string_lossy().into_owned(),
            state,
            stamp: stamp_of(&p),
        });
    }
    for s in settings {
        let (hooks, status) = settings_sections(s);
        let stamp = stamp_of(s);
        out.push(LiveItem {
            item: format!("{}#hooks", s.to_string_lossy()),
            state: hooks,
            stamp: stamp.clone(),
        });
        out.push(LiveItem {
            item: format!("{}#statusLine", s.to_string_lossy()),
            state: status,
            stamp,
        });
    }
    out
}

/// (`hooks` section, `statusLine` section) of a settings file, each canonical JSON or a marker.
fn settings_sections(p: &Path) -> (String, String) {
    let Ok(meta) = std::fs::symlink_metadata(p) else {
        return ("absent".into(), "absent".into());
    };
    if !meta.file_type().is_file() || meta.len() > 8 * 1024 * 1024 {
        return ("not-a-file".into(), "not-a-file".into());
    }
    let Ok(text) = std::fs::read(p) else {
        return ("unreadable".into(), "unreadable".into());
    };
    match serde_json::from_slice::<serde_json::Value>(&text) {
        Ok(v) => (
            v.get("hooks")
                .map(|h| h.to_string())
                .unwrap_or_else(|| "none".into()),
            v.get("statusLine")
                .map(|h| h.to_string())
                .unwrap_or_else(|| "none".into()),
        ),
        Err(_) => {
            use sha2::{Digest, Sha256};
            let h = Sha256::digest(&text);
            let hex: String = h.iter().map(|b| format!("{b:02x}")).collect();
            (format!("invalid-json:{hex}"), format!("invalid-json:{hex}"))
        }
    }
}

/// Items whose state OR stamp changed between two snapshots (BL1: a write of the same bytes is a
/// write).
pub fn changed_items(before: &[LiveItem], after: &[LiveItem]) -> Vec<(LiveItem, LiveItem)> {
    let mut out = Vec::new();
    for a in after {
        match before.iter().find(|b| b.item == a.item) {
            Some(b) if b.state == a.state && b.stamp == a.stamp => {}
            Some(b) => out.push((b.clone(), a.clone())),
            None => out.push((
                LiveItem {
                    item: a.item.clone(),
                    state: "absent".into(),
                    stamp: "absent".into(),
                },
                a.clone(),
            )),
        }
    }
    out
}

/// A pid is alive (signal 0).
#[cfg(unix)]
pub fn pid_alive(pid: u32) -> bool {
    // SAFETY: kill(2) with signal 0 only checks for existence.
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

#[cfg(not(unix))]
pub fn pid_alive(_pid: u32) -> bool {
    false
}

/// A process group still has a member (signal 0).
#[cfg(unix)]
pub fn group_alive(pgid: u32) -> bool {
    // SAFETY: kill(2) with signal 0 only checks for existence.
    unsafe { libc::kill(-(pgid as libc::pid_t), 0) == 0 }
}

#[cfg(not(unix))]
pub fn group_alive(_pgid: u32) -> bool {
    false
}

/// Copies one source state into `dst` (bytes and mode; a link as a link; `None` = absent).
pub fn lay_out(dst: &Path, src: &LaidSource) -> Result<(), RepairError> {
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| RepairError::Io(format!("{}: {e}", parent.display())))?;
    }
    match src {
        LaidSource::Absent => Ok(()),
        LaidSource::Bytes { bytes, mode } => {
            std::fs::write(dst, bytes)
                .map_err(|e| RepairError::Io(format!("{}: {e}", dst.display())))?;
            set_mode(dst, *mode);
            Ok(())
        }
        LaidSource::Link(target) => {
            #[cfg(unix)]
            {
                std::os::unix::fs::symlink(target, dst)
                    .map_err(|e| RepairError::Io(format!("{}: {e}", dst.display())))
            }
            #[cfg(not(unix))]
            {
                let _ = target;
                Err(RepairError::PlatformPending("replay link layout"))
            }
        }
    }
}

/// The content a variant gives one input file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaidSource {
    Absent,
    Bytes { bytes: Vec<u8>, mode: u32 },
    Link(String),
}

impl LaidSource {
    /// Reads a live path with `lstat` semantics (a link is a link).
    pub fn read(p: &Path) -> Result<Self, RepairError> {
        match TargetState::read(p)? {
            TargetState::Absent => Ok(LaidSource::Absent),
            TargetState::Link { target } => Ok(LaidSource::Link(target)),
            TargetState::File { mode, .. } => {
                let bytes = std::fs::read(p)
                    .map_err(|e| RepairError::Io(format!("{}: {e}", p.display())))?;
                Ok(LaidSource::Bytes { bytes, mode })
            }
        }
    }

    /// The `TargetState` of this content (sha256 of the bytes).
    pub fn state(&self) -> TargetState {
        match self {
            LaidSource::Absent => TargetState::Absent,
            LaidSource::Link(t) => TargetState::Link { target: t.clone() },
            LaidSource::Bytes { bytes, mode } => {
                use sha2::{Digest, Sha256};
                let h = Sha256::digest(bytes);
                TargetState::File {
                    sha256: h.iter().map(|b| format!("{b:02x}")).collect(),
                    mode: *mode & 0o7777,
                }
            }
        }
    }
}

/// The text of `rel` embedded in this binary (its RELEASED copy — C4: the copies inside the binary
/// are the only released reference, RULINGS CM1). The embedded table is referenced here once, in a
/// non-generic body (each reference of the `const` from generic code can add a copy of the whole
/// pack to the binary — see `pack::embedded_pack_hash_ref`).
pub fn released_text(rel: &str) -> Option<&'static str> {
    let all: &'static [(&'static str, &'static str)] = crate::pack::PACK_ALL;
    for (r, c) in all {
        if *r == rel {
            return Some(c);
        }
    }
    None
}

/// Seconds left before `until` (zero when passed).
pub fn left(until: Instant) -> Duration {
    until.saturating_duration_since(Instant::now())
}
