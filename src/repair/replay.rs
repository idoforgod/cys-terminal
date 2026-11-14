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

impl ScratchRoot {
    /// Removes the scratch root and SAYS whether it is gone (RULINGS CZ4: a cleanup error or a
    /// leftover scratch root is returned, never swallowed).
    pub fn close(mut self) -> Result<(), RepairError> {
        let p = std::mem::take(&mut self.path);
        remove_tree(&p)
    }
}

impl Drop for ScratchRoot {
    /// The fallback of a root that was not [`ScratchRoot::close`]d (an early return): removed as far
    /// as possible; the callers that judge a run close it explicitly.
    fn drop(&mut self) {
        if !self.path.as_os_str().is_empty() {
            let _ = remove_tree(&self.path);
        }
    }
}

/// Removes a tree without following links; directories made read-only by a run (E-17) are made
/// writable again first. RULINGS CZ4: every error is RETURNED (the first one; the walk still removes
/// what it can), and a tree still present afterwards is an error — a leftover scratch root makes the
/// run that owned it not measurable.
pub fn remove_tree(p: &Path) -> Result<(), RepairError> {
    let mut first: Option<RepairError> = None;
    remove_walk(p, &mut first);
    if first.is_none() && std::fs::symlink_metadata(p).is_ok() {
        first = Some(RepairError::Io(format!(
            "{}: scratch tree still present after removal",
            p.display()
        )));
    }
    match first {
        None => Ok(()),
        Some(e) => Err(e),
    }
}

fn remove_walk(p: &Path, first: &mut Option<RepairError>) {
    fn note(first: &mut Option<RepairError>, p: &Path, e: std::io::Error) {
        if first.is_none() {
            *first = Some(RepairError::Io(format!("{}: {e}", p.display())));
        }
    }
    let meta = match std::fs::symlink_metadata(p) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
        Err(e) => return note(first, p, e),
    };
    if meta.file_type().is_dir() {
        set_mode(p, 0o700);
        match std::fs::read_dir(p) {
            Ok(rd) => {
                for e in rd {
                    match e {
                        Ok(e) => remove_walk(&e.path(), first),
                        Err(e) => note(first, p, e),
                    }
                }
            }
            Err(e) => note(first, p, e),
        }
        if let Err(e) = std::fs::remove_dir(p) {
            note(first, p, e);
        }
    } else if let Err(e) = std::fs::remove_file(p) {
        note(first, p, e);
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

/// The lines the stub logged (one per call) — read through the bounded reader (RULINGS EL3 / EO4:
/// candidate code could replace the log with a FIFO).
pub fn stub_calls(dir: &Path) -> Vec<String> {
    read_bounded(&dir.join("cys-calls.log"))
        .map(|b| String::from_utf8_lossy(&b).lines().map(str::to_string).collect())
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
            let _ = remove_tree(d);
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
    /// Processes of the run still alive when the child had ended (or at the deadline), the child
    /// itself excepted: a member of its group, a descendant the runner saw while the child lived
    /// (one that left the group included), on Linux any process carrying the run's environment
    /// marker (RULINGS CZ4 / DR3 / DF5) — each killed by its exact pid and reported.
    pub leftovers: Vec<u32>,
    /// A pipe of the run (stdout / stderr) was still held open after the group was killed and
    /// drained — a process outside the group kept it (RULINGS CZ4): the output may be incomplete.
    pub pipe_held: bool,
    /// Processes of the run still alive AFTER the kill — what the runner could not kill (DR3).
    pub survivors: Vec<u32>,
    /// The child was not reaped within the kill grace — left to a detached reap (DR3).
    pub unreaped: bool,
    /// Per output stream (RULINGS EL1 / EL4): the read reached the end of the pipe, how many bytes
    /// beyond the kept cap were counted and dropped, and whether a read failed. An output is WHOLE
    /// only with the end reached, nothing dropped and no read error.
    pub stdout_state: StreamState,
    pub stderr_state: StreamState,
}

/// How one output stream of a run was read (RULINGS EL1 / EL4).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StreamState {
    pub eof: bool,
    pub dropped: u64,
    pub read_error: bool,
}

impl StreamState {
    /// The output was observed WHOLE (EL1): its end reached, nothing dropped, no read error.
    pub fn whole(&self) -> bool {
        self.eof && self.dropped == 0 && !self.read_error
    }
}

impl ChildOutcome {
    /// RULINGS EL1: the child's standard output was observed whole.
    pub fn stdout_whole(&self) -> bool {
        self.stdout_state.whole()
    }

    /// RULINGS CZ4: a run that left something behind (a process, a held pipe) did not end cleanly —
    /// its answer is not measurable, whatever it printed.
    pub fn left_something(&self) -> Option<String> {
        if !self.leftovers.is_empty() {
            return Some(format!("leftover process(es) {:?}", self.leftovers));
        }
        if self.pipe_held {
            return Some("an output pipe was held open past the run".into());
        }
        if !self.survivors.is_empty() {
            return Some(format!("process(es) the kill did not reach {:?}", self.survivors));
        }
        if self.unreaped {
            return Some("the child did not die within the kill grace (left to a detached reap)".into());
        }
        None
    }
}

/// Largest output kept per stream (the rest is counted and dropped, and the outcome says so — the
/// output is then not WHOLE, RULINGS EL1 / EL4).
pub const MAX_CAPTURE: usize = 1 << 20;

/// At most this many bytes are read (or written) per pipe per turn of the runner's loop, so that the
/// one deadline is looked at between reads even while a child writes without pause (RULINGS EJ2 /
/// EL4).
const PER_TURN: usize = 64 * 1024;

/// At most this many read (or write) ATTEMPTS per pipe per turn — an attempt answered "interrupted"
/// included — so that even a pipe that answers "interrupted" for ever returns to the one deadline
/// (RULINGS GA9 / DW1).
const ATTEMPTS_PER_TURN: usize = 64;

/// A process of a run as the runner saw it: its pid and its start time (a pid that a later, unrelated
/// process reuses never matches).
pub type SeenProcess = (u32, (u64, u64));

/// The process table views the runner needs (RULINGS CZ4 / DR3 / DF5): a process's start time while
/// it is alive (a zombie counts as gone), its children, the members of a process group.
#[cfg(target_os = "macos")]
mod procs {
    const PROC_PGRP_ONLY: u32 = 2;
    const PROC_PPID_ONLY: u32 = 6;
    const SZOMB: u32 = 5;

    fn list(kind: u32, arg: u32) -> Vec<u32> {
        let mut buf: Vec<libc::pid_t> = vec![0; 512];
        let bytes = (buf.len() * std::mem::size_of::<libc::pid_t>()) as libc::c_int;
        // SAFETY: proc_listpids(3) fills at most `bytes` bytes of pid slots and returns the bytes used.
        let n = unsafe { libc::proc_listpids(kind, arg, buf.as_mut_ptr().cast(), bytes) };
        if n <= 0 {
            return Vec::new();
        }
        let count = (n as usize / std::mem::size_of::<libc::pid_t>()).min(buf.len());
        buf.truncate(count);
        buf.into_iter().filter(|p| *p > 0).map(|p| p as u32).collect()
    }

    /// The start time of `pid` while it is a live process (not a zombie, not gone).
    pub fn alive_since(pid: u32) -> Option<(u64, u64)> {
        // SAFETY: a zeroed plain C struct, filled by proc_pidinfo(3) up to its own size.
        let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
        // SAFETY: as above.
        let n = unsafe {
            libc::proc_pidinfo(
                pid as libc::c_int,
                libc::PROC_PIDTBSDINFO,
                0,
                (&mut info as *mut libc::proc_bsdinfo).cast(),
                size,
            )
        };
        (n == size && info.pbi_status != SZOMB)
            .then_some((info.pbi_start_tvsec, info.pbi_start_tvusec))
    }

    pub fn children(pid: u32) -> Vec<u32> {
        list(PROC_PPID_ONLY, pid)
    }

    pub fn group(pgid: u32) -> Vec<u32> {
        list(PROC_PGRP_ONLY, pgid)
    }
}

#[cfg(target_os = "linux")]
mod procs {
    /// (state, ppid, pgrp, starttime) from `/proc/<pid>/stat`.
    fn stat(pid: u32) -> Option<(char, u32, u32, u64)> {
        let t = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        let rest = &t[t.rfind(')')? + 1..];
        let f: Vec<&str> = rest.split_whitespace().collect();
        let state = f.first()?.chars().next()?;
        let ppid = f.get(1)?.parse().ok()?;
        let pgrp = f.get(2)?.parse().ok()?;
        let start = f.get(19)?.parse().ok()?;
        Some((state, ppid, pgrp, start))
    }

    fn all() -> Vec<u32> {
        std::fs::read_dir("/proc")
            .map(|rd| {
                rd.flatten()
                    .filter_map(|e| e.file_name().to_string_lossy().parse::<u32>().ok())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn alive_since(pid: u32) -> Option<(u64, u64)> {
        let (state, _, _, start) = stat(pid)?;
        (state != 'Z' && state != 'X').then_some((start, 0))
    }

    pub fn children(pid: u32) -> Vec<u32> {
        all()
            .into_iter()
            .filter(|p| stat(*p).is_some_and(|s| s.1 == pid))
            .collect()
    }

    pub fn group(pgid: u32) -> Vec<u32> {
        all()
            .into_iter()
            .filter(|p| stat(*p).is_some_and(|s| s.2 == pgid))
            .collect()
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod procs {
    pub fn alive_since(_pid: u32) -> Option<(u64, u64)> {
        None
    }
    pub fn children(_pid: u32) -> Vec<u32> {
        Vec::new()
    }
    pub fn group(_pgid: u32) -> Vec<u32> {
        Vec::new()
    }
}

/// Records every descendant of `root` alive now (children, recursively; at most 256) and every
/// member of its process group, with its start time, into `seen` (RULINGS CZ4 / DR3: a process that
/// leaves the group is still the child's descendant while the child lives — the runner samples the
/// tree on every turn of its loop).
fn sample_tree(root: u32, seen: &mut Vec<SeenProcess>) {
    let mut stack: Vec<u32> = procs::children(root);
    stack.extend(procs::group(root));
    let mut visited = 0usize;
    while let Some(p) = stack.pop() {
        visited += 1;
        if visited > 256 {
            break;
        }
        if p == root || seen.iter().any(|(q, _)| *q == p) {
            continue;
        }
        if let Some(t) = procs::alive_since(p) {
            seen.push((p, t));
            stack.extend(procs::children(p));
        }
    }
}

/// The processes of `seen` (and of the group `pgid`, the leader excepted) that are STILL the same
/// live processes.
fn still_alive(seen: &[SeenProcess], pgid: u32) -> Vec<u32> {
    let mut out: Vec<u32> = seen
        .iter()
        .filter(|(p, t)| procs::alive_since(*p) == Some(*t))
        .map(|(p, _)| *p)
        .collect();
    for p in procs::group(pgid) {
        if p != pgid && !out.contains(&p) && procs::alive_since(p).is_some() {
            out.push(p);
        }
    }
    out
}

/// How long the output pipes of an ENDED child are still read (never past the run's deadline) before
/// a still-open one is reported as held.
const DRAIN: Duration = Duration::from_millis(500);

/// After the deadline's kill: how long the child's exit is polled before the runner returns anyway
/// and leaves the child to a detached reap (RULINGS DR3).
const KILL_GRACE: Duration = Duration::from_millis(500);

/// How long a killed process may take to disappear before it counts as one the kill did not reach.
const SETTLE: Duration = Duration::from_millis(300);

/// The environment variable that marks every process of one run (RULINGS DF5: leftovers tracked
/// beyond the process group — a process that called `setsid` / `setpgid` keeps its environment).
pub const RUN_MARK_VAR: &str = "CYS_REPLAY_RUN";

/// Whether the runner kills (production) or — a test seam standing for "a child slow to die" — sends
/// no kill at all (RULINGS DR3's third test).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KillMode {
    Kill,
    #[doc(hidden)]
    NoKillForTest,
}

/// Runs `argv` with EXACTLY `env` (nothing inherited — the fixed clean environment, BA1) plus the
/// run's own marker [`RUN_MARK_VAR`], in `cwd`, with `stdin` as input, in its own process group.
///
/// RULINGS CZ4 / DR3 — THE BOUNDED RUNNER NEVER BLOCKS: no thread, no blocking read, write or wait.
/// ONE loop with ONE deadline writes stdin and reads stdout / stderr through non-blocking pipes and
/// polls the child's exit without reaping it (`waitid … WNOHANG | WNOWAIT`). When the child has
/// ended, its outputs are read until both close or [`DRAIN`] passes — never past the deadline; every
/// process still carrying the run's marker then (a descendant in the group or one that left it, e.g.
/// one that closed stdout and lives on) is a leftover: killed by exact pid and reported, and the whole
/// group is killed. At the deadline the group is killed and the exit polled for [`KILL_GRACE`]; the
/// runner RETURNS in every case, saying truthfully what it could not kill (`survivors`) or reap
/// (`unreaped` — that child is left to a detached reap; the calling thread never waits for it). A
/// still-open output is reported (`pipe_held`).
#[cfg(unix)]
pub fn run_child(
    argv: &[String],
    env: &[(String, String)],
    cwd: &Path,
    stdin: &[u8],
    deadline: Duration,
) -> ChildOutcome {
    run_child_with(argv, env, cwd, stdin, deadline, KillMode::Kill)
}

/// [`run_child`] with the kill mode (tests only pass [`KillMode::NoKillForTest`]).
#[cfg(unix)]
pub fn run_child_with(
    argv: &[String],
    env: &[(String, String)],
    cwd: &Path,
    stdin: &[u8],
    deadline: Duration,
    kill: KillMode,
) -> ChildOutcome {
    use std::os::fd::AsRawFd;
    use std::os::unix::process::{CommandExt, ExitStatusExt};
    use std::process::{Command, Stdio};

    let started = Instant::now();
    let until = started + deadline;
    let mut out = ChildOutcome::default();
    let Some((prog, args)) = argv.split_first() else {
        out.spawn_error = Some("empty argv".into());
        return out;
    };
    let mark = random_hex(12);
    let mut cmd = Command::new(prog);
    cmd.args(args)
        .env_clear()
        .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .env(RUN_MARK_VAR, &mark)
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
    let kill_group = || {
        if kill == KillMode::Kill {
            // SAFETY: kill(2) on our own child's process group (pgid = the child's pid, set by
            // process_group(0)); the leader is not reaped yet, so the group id is still ours.
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
            }
        }
    };
    let mut sin = child.stdin.take();
    let mut sout = child.stdout.take();
    let mut serr = child.stderr.take();
    // RULINGS EJ2 / EL4: a pipe that cannot be made non-blocking means the child is not run — it is
    // killed at once and the runner returns saying so (fail closed)
    let all_nonblocking = [
        sin.as_ref().map(|s| s.as_raw_fd()),
        sout.as_ref().map(|s| s.as_raw_fd()),
        serr.as_ref().map(|s| s.as_raw_fd()),
    ]
    .into_iter()
    .flatten()
    .all(set_nonblocking);
    if !all_nonblocking {
        // SAFETY: kill(2) on our own child's process group (not reaped yet).
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
        drop((sin, sout, serr));
        let grace = Instant::now() + KILL_GRACE;
        let mut reaped = false;
        while Instant::now() < grace {
            if let Ok(Some(_)) = child.try_wait() {
                reaped = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        if !reaped {
            out.unreaped = true;
            let _ = std::thread::Builder::new()
                .name("replay-reap".into())
                .spawn(move || {
                    let _ = child.wait();
                });
        }
        out.spawn_error = Some("non-blocking set-up of a pipe failed — the child was not run".into());
        out.elapsed = started.elapsed();
        return out;
    }
    let mut written = 0usize;
    if stdin.is_empty() {
        sin = None; // EOF at once
    }
    // THE loop: one deadline, non-blocking I/O on every pipe, the exit polled
    let mut ended_at: Option<Instant> = None;
    let mut seen: Vec<SeenProcess> = Vec::new();
    loop {
        pump_stdin(&mut sin, stdin, &mut written);
        pump_out(&mut sout, &mut out.stdout, &mut out.stdout_state);
        pump_out(&mut serr, &mut out.stderr, &mut out.stderr_state);
        if ended_at.is_none() {
            // the child's descendants while it lives (a process that leaves the group is still one)
            sample_tree(pid as u32, &mut seen);
        }
        let now = Instant::now();
        if ended_at.is_none() && exited_no_reap(pid) {
            ended_at = Some(now);
            sin = None; // nobody reads it any more; a descendant holding it can never block us
        }
        if let Some(t) = ended_at {
            if (sout.is_none() && serr.is_none()) || now >= t + DRAIN {
                break;
            }
        }
        if now >= until {
            if ended_at.is_none() {
                out.timed_out = true;
            }
            break;
        }
        let cap = until.saturating_duration_since(now).min(Duration::from_millis(20));
        wait_io(&sin, &sout, &serr, cap);
    }
    drop(sin);
    // an output still open when the drain of an ENDED child ran out is held by a process of the run
    // that outlived it — the output may be incomplete (CZ4)
    let held_at_drain = ended_at.is_some() && (sout.is_some() || serr.is_some());
    // what is still alive of the run NOW besides the child itself (an ended leader is a zombie and is
    // not listed): a descendant seen during the run or a member of the group — and, on Linux, any
    // process carrying the run's marker — killed by exact pid and reported
    if ended_at.is_some() {
        std::thread::sleep(Duration::from_millis(10)); // a descendant that has just closed its pipes
    }
    let me = pid as u32;
    let alive_now = |seen: &[SeenProcess]| -> Vec<u32> {
        let mut v = still_alive(seen, me);
        for p in marked_processes(&mark) {
            if !v.contains(&p) {
                v.push(p);
            }
        }
        v.retain(|p| *p != me);
        v
    };
    for p in alive_now(&seen) {
        if kill == KillMode::Kill {
            // SAFETY: kill(2) of a process of THIS run (seen as the child's descendant with this start
            // time, a member of its group, or carrying its marker), by its exact pid.
            unsafe {
                libc::kill(p as libc::pid_t, libc::SIGKILL);
            }
        }
        out.leftovers.push(p);
    }
    kill_group();
    // the exit: already seen, or polled for a short fixed grace after the kill
    let grace_until = Instant::now() + KILL_GRACE;
    while ended_at.is_none() {
        if exited_no_reap(pid) {
            ended_at = Some(Instant::now());
            break;
        }
        if Instant::now() >= grace_until {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    // what the kill did not reach, once the kill has had a moment to land (a killed process is torn
    // down asynchronously — re-read for at most SETTLE)
    let settle_until = Instant::now() + SETTLE;
    out.survivors = alive_now(&seen);
    while !out.survivors.is_empty() && Instant::now() < settle_until {
        std::thread::sleep(Duration::from_millis(20));
        out.survivors = alive_now(&seen);
    }
    // the outputs, once more without blocking; still open → held by something the kill did not reach
    pump_out(&mut sout, &mut out.stdout, &mut out.stdout_state);
    pump_out(&mut serr, &mut out.stderr, &mut out.stderr_state);
    out.pipe_held = held_at_drain || sout.is_some() || serr.is_some();
    drop(sout);
    drop(serr);
    if ended_at.is_some() {
        // the exit was seen: this reaps at once
        match child.try_wait() {
            Ok(Some(st)) => {
                out.code = st.code();
                out.signal = st.signal();
            }
            Ok(None) => out.unreaped = true,
            Err(e) => out.spawn_error = Some(format!("wait: {e}")),
        }
    } else {
        out.unreaped = true;
    }
    if out.unreaped && !out.survivors.contains(&me) {
        out.survivors.push(me); // the child itself did not die within the grace
    }
    if out.unreaped {
        // a child that does not die (stuck in the kernel): reported; reaped by a detached thread —
        // the caller never waits for it
        let detached = std::thread::Builder::new()
            .name("replay-reap".into())
            .spawn(move || {
                let _ = child.wait();
            });
        if detached.is_err() {
            out.spawn_error = Some("the unreaped child could not be handed to a reaper".into());
        }
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

#[cfg(unix)]
fn set_nonblocking(fd: std::os::fd::RawFd) -> bool {
    #[cfg(test)]
    if FAIL_NONBLOCK_FOR_TEST.with(|f| f.get()) {
        return false;
    }
    // SAFETY: fcntl(2) on a pipe descriptor this process owns.
    unsafe {
        let fl = libc::fcntl(fd, libc::F_GETFL);
        fl >= 0 && libc::fcntl(fd, libc::F_SETFL, fl | libc::O_NONBLOCK) == 0
    }
}

#[cfg(test)]
thread_local! {
    /// Test seam of RULINGS EJ2 / EL4: the non-blocking set-up fails on this thread.
    pub(crate) static FAIL_NONBLOCK_FOR_TEST: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Writes what the (non-blocking) stdin pipe takes now; done or broken → the pipe is closed.
#[cfg(unix)]
fn pump_stdin(sin: &mut Option<std::process::ChildStdin>, input: &[u8], written: &mut usize) {
    use std::io::Write;
    let Some(s) = sin.as_mut() else {
        return;
    };
    let turn_end = (*written + PER_TURN).min(input.len());
    let mut attempts = 0usize;
    while *written < turn_end {
        // RULINGS GA9: every attempt — an interrupted one included — is behind the turn's budget
        attempts += 1;
        if attempts > ATTEMPTS_PER_TURN {
            return;
        }
        match s.write(&input[*written..turn_end]) {
            Ok(0) => break,
            Ok(n) => *written += n,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => {
                *sin = None; // the reader is gone (EPIPE)
                return;
            }
        }
    }
    if *written >= input.len() {
        *sin = None; // all written: EOF for the child
    }
}

/// Reads what a (non-blocking) output pipe holds now — at most [`PER_TURN`] bytes, then back to the
/// clock (RULINGS EJ2 / EL4); kept up to [`MAX_CAPTURE`], the rest counted and dropped; EOF or a read
/// error closes the pipe and is recorded in `st`.
#[cfg(unix)]
pub(crate) fn pump_out<R: std::io::Read>(p: &mut Option<R>, buf: &mut Vec<u8>, st: &mut StreamState) {
    let Some(r) = p.as_mut() else {
        return;
    };
    let mut chunk = [0u8; 8192];
    let mut this_turn = 0usize;
    let mut attempts = 0usize;
    while this_turn < PER_TURN {
        // RULINGS GA9: every read attempt — an interrupted one included — is behind the turn's budget
        attempts += 1;
        if attempts > ATTEMPTS_PER_TURN {
            return;
        }
        match r.read(&mut chunk) {
            Ok(0) => {
                st.eof = true;
                *p = None;
                return;
            }
            Ok(n) => {
                this_turn += n;
                let take = n.min(MAX_CAPTURE.saturating_sub(buf.len()));
                buf.extend_from_slice(&chunk[..take]);
                st.dropped += (n - take) as u64;
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => {
                st.read_error = true;
                *p = None;
                return;
            }
        }
    }
}

/// Waits (at most `limit`) until one of the open pipes is ready.
#[cfg(unix)]
fn wait_io(
    sin: &Option<std::process::ChildStdin>,
    sout: &Option<std::process::ChildStdout>,
    serr: &Option<std::process::ChildStderr>,
    limit: Duration,
) {
    use std::os::fd::AsRawFd;
    let mut fds: Vec<libc::pollfd> = Vec::with_capacity(3);
    if let Some(s) = sin {
        fds.push(libc::pollfd {
            fd: s.as_raw_fd(),
            events: libc::POLLOUT,
            revents: 0,
        });
    }
    for fd in [sout.as_ref().map(|s| s.as_raw_fd()), serr.as_ref().map(|s| s.as_raw_fd())]
        .into_iter()
        .flatten()
    {
        fds.push(libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        });
    }
    let ms = limit.as_millis().clamp(1, 1000) as libc::c_int;
    if fds.is_empty() {
        std::thread::sleep(Duration::from_millis(ms as u64));
        return;
    }
    // SAFETY: poll(2) over a vector of pollfd we own, its exact length.
    unsafe {
        libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, ms);
    }
}

/// The processes (pid) whose environment carries `<RUN_MARK_VAR>=<mark>` (RULINGS DF5) — Linux
/// (`/proc/<pid>/environ`) only: macOS does not show another process's environment (measured on this
/// Mac: `ps -E` lists none, `KERN_PROCARGS2` gives the arguments only), so there the runner tracks the
/// child's process TREE instead ([`sample_tree`]); elsewhere none is found.
pub fn marked_processes(mark: &str) -> Vec<u32> {
    let needle = format!("{RUN_MARK_VAR}={mark}");
    marked_processes_by(needle.as_bytes())
}

#[cfg(target_os = "linux")]
fn has_entry(area: &[u8], needle: &[u8]) -> bool {
    area.split(|b| *b == 0).any(|e| e == needle)
}

#[cfg(target_os = "linux")]
fn marked_processes_by(needle: &[u8]) -> Vec<u32> {
    let me = std::process::id();
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir("/proc") {
        for e in rd.flatten() {
            let Ok(pid) = e.file_name().to_string_lossy().parse::<u32>() else {
                continue;
            };
            if pid == me {
                continue;
            }
            if let Ok(area) = std::fs::read(e.path().join("environ")) {
                if has_entry(&area, needle) {
                    out.push(pid);
                }
            }
        }
    }
    out
}

#[cfg(not(target_os = "linux"))]
fn marked_processes_by(_needle: &[u8]) -> Vec<u32> {
    Vec::new()
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

/// Opens `p` without following a link and without blocking (a FIFO swapped in cannot hang the
/// thread — the class of RULINGS CW3); the caller re-checks the kind on the handle.
#[cfg(unix)]
pub(crate) fn open_nofollow_nonblock(p: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(p)
}

#[cfg(not(unix))]
pub(crate) fn open_nofollow_nonblock(p: &Path) -> std::io::Result<std::fs::File> {
    std::fs::File::open(p)
}

/// The change stamp of a compared item from its metadata (BL1 / RULINGS DE2): inode, change time AND
/// modification time, each with nanoseconds. A write — the same bytes included — moves the
/// modification time; a replacement moves the inode; a mode or owner change, a link count change or
/// a write that resets the modification time moves the change time. (A system service that re-stamps
/// files without writing — measured: the iCloud provider on a Desktop-backed folder — moves the
/// change time too: the compared items are the lane's pack and the settings files, which are not
/// synced; a repeat then judges whether the item was written AGAIN.)
#[cfg(unix)]
fn stamp_from(m: &std::fs::Metadata) -> String {
    use std::os::unix::fs::MetadataExt;
    format!(
        "ino={} ctime={}.{:09} mtime={}.{:09}",
        m.ino(),
        m.ctime(),
        m.ctime_nsec(),
        m.mtime(),
        m.mtime_nsec()
    )
}

#[cfg(not(unix))]
fn stamp_from(m: &std::fs::Metadata) -> String {
    format!("len={} mtime={:?}", m.len(), m.modified().ok())
}

/// Largest file a replayed / compared read takes (a pack file or a settings file is far smaller).
const MAX_READ: u64 = 64 * 1024 * 1024;

/// RULINGS DF5: ONE read of a replayed or compared path — its content, mode and change stamp all from
/// the SAME no-follow, non-blocking handle (a link is reported as a link from `lstat`; a missing
/// path is absent; anything that is not a regular file on the opened handle is refused). Nothing is
/// opened twice, so the bytes laid out, hashed and compared are the bytes that were stamped.
/// RULINGS EL3 — THE way the screen and the replay read a file a candidate could have produced (the
/// scratch settings, the templates, the stub's log, every other scratch file): [`read_stamped`] —
/// no link followed, a non-blocking open, a regular file, a size cap — and anything else (a FIFO, a
/// link, a missing file) is an error at once.
pub fn read_bounded(p: &Path) -> Result<Vec<u8>, RepairError> {
    match read_stamped(p)? {
        (LaidSource::Bytes { bytes, .. }, _) => Ok(bytes),
        _ => Err(RepairError::Precondition(format!(
            "{}: not a regular file",
            p.display()
        ))),
    }
}

pub fn read_stamped(p: &Path) -> Result<(LaidSource, String), RepairError> {
    let io = |e: std::io::Error| RepairError::Io(format!("{}: {e}", p.display()));
    let lmeta = match std::fs::symlink_metadata(p) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok((LaidSource::Absent, "absent".into()))
        }
        Err(e) => return Err(io(e)),
    };
    if lmeta.file_type().is_symlink() {
        let target = std::fs::read_link(p).map_err(io)?;
        let target = target.into_os_string().into_string().map_err(|_| {
            RepairError::Io(format!("{}: link target is not valid UTF-8", p.display()))
        })?;
        return Ok((LaidSource::Link(target), stamp_from(&lmeta)));
    }
    if !lmeta.file_type().is_file() {
        return Err(RepairError::Precondition(format!(
            "{}: not a regular file or symbolic link",
            p.display()
        )));
    }
    let f = open_nofollow_nonblock(p).map_err(io)?;
    let meta = f.metadata().map_err(io)?;
    if !meta.file_type().is_file() {
        return Err(RepairError::Precondition(format!(
            "{}: changed while being read (no longer a regular file)",
            p.display()
        )));
    }
    let mut bytes = Vec::with_capacity(meta.len().min(MAX_READ) as usize);
    std::io::Read::read_to_end(&mut std::io::Read::take(&f, MAX_READ + 1), &mut bytes)
        .map_err(io)?;
    if bytes.len() as u64 > MAX_READ {
        return Err(RepairError::Precondition(format!(
            "{}: larger than {MAX_READ} bytes",
            p.display()
        )));
    }
    Ok((
        LaidSource::Bytes {
            bytes,
            mode: permission_bits(&meta),
        },
        stamp_from(&meta),
    ))
}

#[cfg(unix)]
fn permission_bits(m: &std::fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    m.permissions().mode() & 0o7777
}

#[cfg(not(unix))]
fn permission_bits(m: &std::fs::Metadata) -> u32 {
    if m.permissions().readonly() {
        0o444
    } else {
        0o644
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
        let (state, stamp) = match read_stamped(&p) {
            Ok((c, stamp)) => (c.state().canon(), stamp),
            Err(e) => (format!("unreadable: {e:?}"), "unreadable".into()),
        };
        out.push(LiveItem {
            item: p.to_string_lossy().into_owned(),
            state,
            stamp,
        });
    }
    for s in settings {
        let (hooks, status, stamp) = settings_sections(s);
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

/// (`hooks` section, `statusLine` section, change stamp) of a settings file, each section canonical
/// JSON or a marker — read through ONE no-follow, non-blocking handle ([`read_stamped`], DF5).
fn settings_sections(p: &Path) -> (String, String, String) {
    let (bytes, stamp) = match read_stamped(p) {
        Ok((LaidSource::Absent, st)) => return ("absent".into(), "absent".into(), st),
        Ok((LaidSource::Link(_), st)) => return ("not-a-file".into(), "not-a-file".into(), st),
        Ok((LaidSource::Bytes { bytes, .. }, st)) => (bytes, st),
        Err(_) => return ("unreadable".into(), "unreadable".into(), "unreadable".into()),
    };
    match serde_json::from_slice::<serde_json::Value>(&bytes) {
        Ok(v) => (
            v.get("hooks")
                .map(|h| h.to_string())
                .unwrap_or_else(|| "none".into()),
            v.get("statusLine")
                .map(|h| h.to_string())
                .unwrap_or_else(|| "none".into()),
            stamp,
        ),
        Err(_) => {
            use sha2::{Digest, Sha256};
            let h = Sha256::digest(&bytes);
            let hex: String = h.iter().map(|b| format!("{b:02x}")).collect();
            (format!("invalid-json:{hex}"), format!("invalid-json:{hex}"), stamp)
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
    /// Reads a live path with `lstat` semantics (a link is a link) — the content and mode from ONE
    /// no-follow, non-blocking handle ([`read_stamped`], RULINGS DF5).
    pub fn read(p: &Path) -> Result<Self, RepairError> {
        read_stamped(p).map(|(c, _)| c)
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
