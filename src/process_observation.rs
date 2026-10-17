//! Targeted process observations for callers that cannot treat missing data as alive.
//! This is not proof of stdin ownership or immutable launch history. In particular,
//! argv is mutable and a check cannot close the race before a subsequent write.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    pub pid: u32,
    pub parent: u32,
    pub process_group: u32,
    pub birth_seconds: u64,
    pub birth_microseconds: u64,
    pub pointer_width: usize,
    pub executable: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invocation {
    /// Saved launch path; may differ from the current native executable path.
    pub launch_path: Vec<u8>,
    pub argv: Vec<Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessObservation {
    pub identity: Identity,
    pub invocation: Invocation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    InvalidPid,
    Dead,
    Unknown {
        stage: &'static str,
        os_error: Option<i32>,
    },
    Malformed(&'static str),
    /// A birth, executable, argv, parent or process-group mismatch. This is lost
    /// continuity evidence, not established death; consumers must not infer Dead.
    Changed,
    UnsupportedPlatform,
}

#[cfg(any(target_os = "macos", test))]
fn identity_query_result(n: i32, expected: i32, error: Option<i32>) -> Result<(), Failure> {
    if n == expected {
        return Ok(());
    }
    if n <= 0 && error == Some(libc::ESRCH) {
        return Err(Failure::Dead);
    }
    Err(Failure::Unknown {
        stage: "proc_pidinfo",
        os_error: if n <= 0 { error } else { None },
    })
}

#[cfg(any(target_os = "macos", test))]
fn argument_query_result(
    bytes: &[u8],
    len: usize,
    rc: i32,
    error: Option<i32>,
    width: usize,
) -> Result<Invocation, Failure> {
    if rc != 0 {
        // EINVAL also means access/stack/task failure, not conclusive death.
        return Err(Failure::Unknown {
            stage: "KERN_PROCARGS2",
            os_error: error,
        });
    }
    if len >= bytes.len() {
        return Err(Failure::Unknown {
            stage: "saturated arguments",
            os_error: None,
        });
    }
    parse_darwin_invocation(&bytes[..len], width)
}

/// Read an intact Darwin KERN_PROCARGS2 response. Do not skip leading NULs:
/// they may be empty argv entries rather than executable-path padding.
#[cfg(any(target_os = "macos", test))]
fn parse_darwin_invocation(bytes: &[u8], width: usize) -> Result<Invocation, Failure> {
    if !matches!(width, 4 | 8) {
        return Err(Failure::Malformed("unsupported pointer width"));
    }
    let header = bytes.get(..4).ok_or(Failure::Malformed("missing argc"))?;
    let argc = i32::from_ne_bytes(header.try_into().unwrap());
    if argc <= 0 {
        return Err(Failure::Malformed("nonpositive argc"));
    }
    let strings = &bytes[4..];
    let end = strings
        .iter()
        .position(|&b| b == 0)
        .ok_or(Failure::Malformed("unterminated launch path"))?;
    if end == 0 {
        return Err(Failure::Malformed("empty launch path"));
    }
    // XNU aligns the saved path BEFORE prepending argc. The target may be 32-bit
    // even when the observer is 64-bit. Empty argv[0] starts at this exact offset.
    let offset = end
        .checked_add(width)
        .ok_or(Failure::Malformed("path overflow"))?
        / width
        * width;
    let padding = strings
        .get(end..offset)
        .ok_or(Failure::Malformed("truncated padding"))?;
    if padding.iter().any(|&b| b != 0) {
        return Err(Failure::Malformed("nonzero padding"));
    }
    let mut remaining = &strings[offset..];
    if argc as usize > remaining.len() {
        return Err(Failure::Malformed("impossible argc"));
    }
    let mut argv = Vec::new();
    for _ in 0..argc {
        let end = remaining
            .iter()
            .position(|&b| b == 0)
            .ok_or(Failure::Malformed("unterminated argv"))?;
        argv.push(remaining[..end].to_vec());
        remaining = &remaining[end + 1..];
    }
    Ok(Invocation {
        launch_path: strings[..end].to_vec(),
        argv,
    })
}

#[cfg(any(target_os = "macos", target_os = "linux", test))]
fn read_consistent(
    mut identity: impl FnMut() -> Result<Identity, Failure>,
    invocation: impl FnOnce(usize) -> Result<Invocation, Failure>,
) -> Result<ProcessObservation, Failure> {
    let before = identity()?;
    let invocation = invocation(before.pointer_width)?;
    if identity()? != before {
        return Err(Failure::Changed);
    }
    Ok(ProcessObservation {
        identity: before,
        invocation,
    })
}

pub fn observe(pid: u32) -> Result<ProcessObservation, Failure> {
    if pid == 0 || pid > i32::MAX as u32 {
        return Err(Failure::InvalidPid);
    }
    #[cfg(target_os = "macos")]
    {
        read_consistent(
            || darwin::identity(pid),
            |width| darwin::invocation(pid, width),
        )
    }
    #[cfg(target_os = "linux")]
    {
        read_consistent(
            || linux_backend::identity(pid),
            |_| linux_backend::invocation(pid),
        )
    }
    #[cfg(windows)]
    {
        windows_backend::observe(pid)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
        Err(Failure::UnsupportedPlatform)
    }
}

/// Compare with a fresh targeted observation, including executable and argv.
pub fn recheck(expected: &ProcessObservation) -> Result<(), Failure> {
    if observe(expected.identity.pid)? == *expected {
        Ok(())
    } else {
        Err(Failure::Changed)
    }
}

#[cfg(target_os = "macos")]
mod darwin {
    use super::*;

    fn unknown(stage: &'static str, os_error: Option<i32>) -> Failure {
        Failure::Unknown { stage, os_error }
    }

    pub(super) fn identity(pid: u32) -> Result<Identity, Failure> {
        let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of_val(&info) as i32;
        // SAFETY: writable proc_bsdinfo of the exact size requested; pid was validated.
        let n = unsafe {
            libc::proc_pidinfo(
                pid as i32,
                libc::PROC_PIDTBSDINFO,
                0,
                (&mut info as *mut libc::proc_bsdinfo).cast(),
                size,
            )
        };
        let error = (n <= 0)
            .then(|| std::io::Error::last_os_error().raw_os_error())
            .flatten();
        identity_query_result(n, size, error)?;
        if info.pbi_pid != pid || info.pbi_start_tvsec == 0 || info.pbi_start_tvusec >= 1_000_000 {
            return Err(unknown("native identity fields", None));
        }
        let mut path = vec![0u8; 4096]; // PROC_PIDPATHINFO_MAXSIZE in the Darwin SDK.
                                        // SAFETY: path owns the entire writable byte buffer passed to libproc.
        let n =
            unsafe { libc::proc_pidpath(pid as i32, path.as_mut_ptr().cast(), path.len() as u32) };
        if n <= 0 {
            return Err(unknown(
                "proc_pidpath",
                std::io::Error::last_os_error().raw_os_error(),
            ));
        }
        let end = n as usize; // libproc returns strlen, excluding the terminator.
        if end >= path.len() || path[end] != 0 || path[..end].contains(&0) {
            return Err(unknown("native executable length", None));
        }
        path.truncate(end);
        Ok(Identity {
            pid,
            parent: info.pbi_ppid,
            process_group: info.pbi_pgid,
            birth_seconds: info.pbi_start_tvsec,
            birth_microseconds: info.pbi_start_tvusec,
            pointer_width: if info.pbi_flags & 0x10 != 0 { 8 } else { 4 }, // PROC_FLAG_LP64
            executable: path,
        })
    }

    pub(super) fn invocation(pid: u32, width: usize) -> Result<Invocation, Failure> {
        let mut max: i32 = 0;
        let mut len = std::mem::size_of_val(&max);
        let mut mib = [libc::CTL_KERN, libc::KERN_ARGMAX];
        // SAFETY: MIB and output pointers refer to initialized, correctly sized storage.
        let rc = unsafe {
            libc::sysctl(
                mib.as_mut_ptr(),
                2,
                (&mut max as *mut i32).cast(),
                &mut len,
                std::ptr::null_mut(),
                0,
            )
        };
        if rc != 0 {
            return Err(unknown(
                "KERN_ARGMAX",
                std::io::Error::last_os_error().raw_os_error(),
            ));
        }
        if len != std::mem::size_of_val(&max) || !(1..=1_048_576).contains(&max) {
            return Err(unknown("argument capacity", None));
        }
        // A size-query followed by a short read can succeed with truncated strings.
        // Use the bounded kernel limit plus argc, and refuse a saturated response.
        let mut bytes = vec![0u8; max as usize + 4];
        len = bytes.len();
        let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid as i32];
        // SAFETY: sysctl may write only within the capacity declared by len.
        let rc = unsafe {
            libc::sysctl(
                mib.as_mut_ptr(),
                3,
                bytes.as_mut_ptr().cast(),
                &mut len,
                std::ptr::null_mut(),
                0,
            )
        };
        let error = (rc != 0)
            .then(|| std::io::Error::last_os_error().raw_os_error())
            .flatten();
        argument_query_result(&bytes, len, rc, error, width)
    }
}

/// Native pid-reuse anchor kept for the lifetime of a receiver binding.
/// macOS/Linux: the full observation (birth identity + executable + argv) re-read on every check.
/// Windows: additionally an open process handle (PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE),
/// so a reused PID can never satisfy the check and process exit is read from the handle itself.
pub struct Retained {
    observation: ProcessObservation,
    #[cfg(windows)]
    handle: windows_backend::OwnedProcess,
}

impl std::fmt::Debug for Retained {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Retained")
            .field("observation", &self.observation)
            .finish()
    }
}

impl Retained {
    /// Observe `pid` and retain it. On Windows the handle is opened FIRST and every query goes
    /// through it, so the observation and the retained object are the same process.
    pub fn open(pid: u32) -> Result<Retained, Failure> {
        if pid == 0 || pid > i32::MAX as u32 {
            return Err(Failure::InvalidPid);
        }
        #[cfg(windows)]
        {
            let handle = windows_backend::OwnedProcess::open(pid)?;
            let observation = windows_backend::observe_handle(&handle, pid)?;
            Ok(Retained {
                observation,
                handle,
            })
        }
        #[cfg(not(windows))]
        {
            Ok(Retained {
                observation: observe(pid)?,
            })
        }
    }

    pub fn observation(&self) -> &ProcessObservation {
        &self.observation
    }

    /// Fresh targeted check of the SAME process: Dead when it is gone, Changed when any
    /// identity/executable/argv field differs (not death), Unknown when it cannot be read.
    pub fn recheck(&self) -> Result<(), Failure> {
        #[cfg(windows)]
        {
            if windows_backend::exited(&self.handle)? {
                return Err(Failure::Dead);
            }
            if windows_backend::observe_handle(&self.handle, self.observation.identity.pid)?
                == self.observation
            {
                Ok(())
            } else {
                Err(Failure::Changed)
            }
        }
        #[cfg(not(windows))]
        {
            recheck(&self.observation)
        }
    }
}

// ---------------- Linux /proc backend (pure parsers compile everywhere for tests) ----------------

/// Fields of /proc/<pid>/stat after the comm field. `comm` may contain spaces and ')' so the
/// parse anchors on the LAST ')'.
#[cfg(any(target_os = "linux", test))]
fn parse_linux_stat(stat: &[u8]) -> Result<(u32, u32, u64), Failure> {
    let close = stat
        .iter()
        .rposition(|&b| b == b')')
        .ok_or(Failure::Malformed("stat without comm"))?;
    let rest =
        std::str::from_utf8(&stat[close + 1..]).map_err(|_| Failure::Malformed("stat utf8"))?;
    let fields: Vec<&str> = rest.split_ascii_whitespace().collect();
    // fields[0]=state [1]=ppid [2]=pgrp ... [19]=starttime (field 22 overall).
    if fields.len() < 20 {
        return Err(Failure::Malformed("short stat"));
    }
    if fields[0] == "Z" || fields[0] == "X" {
        return Err(Failure::Dead); // zombie/dead: exited, not a receiver
    }
    let num = |s: &str, what| s.parse::<u64>().map_err(|_| Failure::Malformed(what));
    let ppid = num(fields[1], "ppid")? as u32;
    let pgrp = num(fields[2], "pgrp")? as u32;
    let start = num(fields[19], "starttime")?;
    if start == 0 {
        return Err(Failure::Unknown {
            stage: "linux starttime",
            os_error: None,
        });
    }
    Ok((ppid, pgrp, start))
}

#[cfg(any(target_os = "linux", test))]
fn parse_linux_cmdline(bytes: &[u8]) -> Result<Vec<Vec<u8>>, Failure> {
    if bytes.is_empty() {
        // Kernel threads and exited/zombie processes have an empty cmdline.
        return Err(Failure::Unknown {
            stage: "empty cmdline",
            os_error: None,
        });
    }
    let body = bytes.strip_suffix(&[0]).unwrap_or(bytes);
    Ok(body.split(|&b| b == 0).map(|s| s.to_vec()).collect())
}

#[cfg(target_os = "linux")]
mod linux_backend {
    use super::*;
    use std::io::ErrorKind;

    fn io_failure(stage: &'static str, e: std::io::Error) -> Failure {
        match e.kind() {
            ErrorKind::NotFound => Failure::Dead,
            _ => Failure::Unknown {
                stage,
                os_error: e.raw_os_error(),
            },
        }
    }

    pub(super) fn identity(pid: u32) -> Result<Identity, Failure> {
        let stat =
            std::fs::read(format!("/proc/{pid}/stat")).map_err(|e| io_failure("/proc stat", e))?;
        let (parent, process_group, start) = parse_linux_stat(&stat)?;
        // SAFETY: sysconf has no memory side effects.
        let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
        if hz <= 0 {
            return Err(Failure::Unknown {
                stage: "clock ticks",
                os_error: None,
            });
        }
        let hz = hz as u64;
        let exe = std::fs::read_link(format!("/proc/{pid}/exe"))
            .map_err(|e| io_failure("/proc exe", e))?;
        use std::os::unix::ffi::OsStrExt;
        Ok(Identity {
            pid,
            parent,
            process_group,
            birth_seconds: start / hz,
            birth_microseconds: (start % hz) * 1_000_000 / hz,
            pointer_width: std::mem::size_of::<usize>(),
            executable: exe.as_os_str().as_bytes().to_vec(),
        })
    }

    pub(super) fn invocation(pid: u32) -> Result<Invocation, Failure> {
        let raw = std::fs::read(format!("/proc/{pid}/cmdline"))
            .map_err(|e| io_failure("/proc cmdline", e))?;
        let argv = parse_linux_cmdline(&raw)?;
        Ok(Invocation {
            launch_path: argv[0].clone(),
            argv,
        })
    }
}

// ---------------- Windows backend ----------------

/// Parse a Windows command line with the rules CommandLineToArgvW / the MSVC CRT use:
/// argv[0] is taken up to the first unquoted whitespace with quotes only toggling (no escapes);
/// later arguments use the 2n / 2n+1 backslash-before-quote rule and "" inside quotes.
#[cfg(any(windows, test))]
fn parse_windows_command_line(cmd: &[u16]) -> Vec<Vec<u16>> {
    const SP: u16 = b' ' as u16;
    const TAB: u16 = b'\t' as u16;
    const Q: u16 = b'"' as u16;
    const BS: u16 = b'\\' as u16;
    let mut out = Vec::new();
    let mut i = 0;
    // argv[0]
    let mut arg0 = Vec::new();
    let mut quoted = false;
    while i < cmd.len() {
        let c = cmd[i];
        if c == Q {
            quoted = !quoted;
        } else if !quoted && (c == SP || c == TAB) {
            break;
        } else {
            arg0.push(c);
        }
        i += 1;
    }
    out.push(arg0);
    loop {
        while i < cmd.len() && (cmd[i] == SP || cmd[i] == TAB) {
            i += 1;
        }
        if i >= cmd.len() {
            break;
        }
        let mut arg = Vec::new();
        let mut quoted = false;
        while i < cmd.len() {
            let c = cmd[i];
            if c == BS {
                let mut n = 0;
                while i < cmd.len() && cmd[i] == BS {
                    n += 1;
                    i += 1;
                }
                if i < cmd.len() && cmd[i] == Q {
                    arg.extend(std::iter::repeat(BS).take(n / 2));
                    if n % 2 == 1 {
                        arg.push(Q);
                        i += 1;
                    }
                } else {
                    arg.extend(std::iter::repeat(BS).take(n));
                }
                continue;
            }
            if c == Q {
                if quoted && i + 1 < cmd.len() && cmd[i + 1] == Q {
                    arg.push(Q); // "" inside quotes is a literal quote
                    i += 2;
                    continue;
                }
                quoted = !quoted;
                i += 1;
                continue;
            }
            if !quoted && (c == SP || c == TAB) {
                break;
            }
            arg.push(c);
            i += 1;
        }
        out.push(arg);
    }
    out
}

/// Lossless WTF-8 bytes of a UTF-16 string (unpaired surrogates included), so identity
/// comparison never merges two different native paths.
#[cfg(any(windows, test))]
fn wide_to_bytes(w: &[u16]) -> Vec<u8> {
    let mut out = Vec::with_capacity(w.len());
    for r in char::decode_utf16(w.iter().copied()) {
        match r {
            Ok(c) => {
                let mut b = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut b).as_bytes());
            }
            Err(e) => {
                // WTF-8 generalized encoding of a lone surrogate (3 bytes).
                let u = e.unpaired_surrogate() as u32;
                out.push(0xE0 | (u >> 12) as u8);
                out.push(0x80 | ((u >> 6) & 0x3F) as u8);
                out.push(0x80 | (u & 0x3F) as u8);
            }
        }
    }
    out
}

/// FILETIME (100 ns ticks since 1601) -> (seconds, microseconds) on the Unix epoch scale.
#[cfg(any(windows, test))]
fn filetime_to_unix(ticks: u64) -> Option<(u64, u64)> {
    const EPOCH_DIFF_TICKS: u64 = 116_444_736_000_000_000;
    let t = ticks.checked_sub(EPOCH_DIFF_TICKS)?;
    Some((t / 10_000_000, (t % 10_000_000) / 10))
}

#[cfg(windows)]
mod windows_backend {
    use super::*;
    use windows_sys::Wdk::System::Threading::{
        NtQueryInformationProcess, ProcessBasicInformation, ProcessCommandLineInformation,
    };
    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, ERROR_INVALID_PARAMETER, FILETIME, HANDLE, WAIT_OBJECT_0,
        WAIT_TIMEOUT,
    };
    use windows_sys::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, QueryFullProcessImageNameW, WaitForSingleObject,
        PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
    };

    const STATUS_INFO_LENGTH_MISMATCH: i32 = 0xC000_0004_u32 as i32;
    /// 32,767 UTF-16 units is the documented command-line maximum; header + slack on top.
    const CMDLINE_CAP_BYTES: usize = 32_768 * 2 + 64;

    fn last_error(stage: &'static str) -> Failure {
        // SAFETY: reads the calling thread's last-error value only.
        Failure::Unknown {
            stage,
            os_error: Some(unsafe { GetLastError() } as i32),
        }
    }

    pub(super) struct OwnedProcess(HANDLE);
    // SAFETY: a process handle is a kernel object reference usable from any thread; it is
    // closed exactly once in Drop.
    unsafe impl Send for OwnedProcess {}
    unsafe impl Sync for OwnedProcess {}
    impl Drop for OwnedProcess {
        fn drop(&mut self) {
            // SAFETY: the handle was returned by OpenProcess and is owned by this value.
            unsafe { CloseHandle(self.0) };
        }
    }

    impl OwnedProcess {
        pub(super) fn open(pid: u32) -> Result<Self, Failure> {
            // SAFETY: plain Win32 call; a null return is handled below.
            let h = unsafe {
                OpenProcess(
                    PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                    0,
                    pid,
                )
            };
            if h.is_null() {
                // SAFETY: as in last_error.
                let e = unsafe { GetLastError() };
                return Err(if e == ERROR_INVALID_PARAMETER {
                    Failure::Dead // no such process id
                } else {
                    Failure::Unknown {
                        stage: "OpenProcess",
                        os_error: Some(e as i32),
                    }
                });
            }
            Ok(OwnedProcess(h))
        }
    }

    /// Exit is read from the retained object, never from a PID lookup.
    pub(super) fn exited(p: &OwnedProcess) -> Result<bool, Failure> {
        // SAFETY: zero-timeout wait on an owned handle.
        match unsafe { WaitForSingleObject(p.0, 0) } {
            WAIT_OBJECT_0 => Ok(true),
            WAIT_TIMEOUT => Ok(false),
            _ => Err(last_error("WaitForSingleObject")),
        }
    }

    #[repr(C)]
    struct BasicInfo {
        exit_status: i32,
        peb: *mut core::ffi::c_void,
        affinity: usize,
        base_priority: i32,
        unique_pid: usize,
        parent_pid: usize,
    }

    #[repr(C)]
    struct UnicodeString {
        length: u16,
        maximum_length: u16,
        buffer: *mut u16,
    }

    fn identity(p: &OwnedProcess, pid: u32) -> Result<Identity, Failure> {
        let zero = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let (mut c, mut e, mut k, mut u) = (zero, zero, zero, zero);
        // SAFETY: four writable FILETIME outputs.
        if unsafe { GetProcessTimes(p.0, &mut c, &mut e, &mut k, &mut u) } == 0 {
            return Err(last_error("GetProcessTimes"));
        }
        let ticks = ((c.dwHighDateTime as u64) << 32) | c.dwLowDateTime as u64;
        let (birth_seconds, birth_microseconds) =
            filetime_to_unix(ticks).ok_or(Failure::Unknown {
                stage: "creation time",
                os_error: None,
            })?;
        let mut path = vec![0u16; 32_768];
        let mut len = path.len() as u32;
        // SAFETY: len is the writable capacity in UTF-16 units and is updated to the length.
        if unsafe {
            QueryFullProcessImageNameW(p.0, PROCESS_NAME_WIN32, path.as_mut_ptr(), &mut len)
        } == 0
        {
            return Err(last_error("QueryFullProcessImageNameW"));
        }
        if len == 0 || len as usize >= path.len() {
            return Err(Failure::Unknown {
                stage: "image path length",
                os_error: None,
            });
        }
        // SAFETY: BasicInfo is plain data; zeroed is a valid initial value.
        let mut bi: BasicInfo = unsafe { std::mem::zeroed() };
        let mut ret = 0u32;
        let size = std::mem::size_of::<BasicInfo>() as u32;
        // SAFETY: writable buffer of exactly `size` bytes.
        let st = unsafe {
            NtQueryInformationProcess(
                p.0,
                ProcessBasicInformation,
                (&mut bi as *mut BasicInfo).cast(),
                size,
                &mut ret,
            )
        };
        if st < 0 || ret != size || bi.unique_pid != pid as usize {
            return Err(Failure::Unknown {
                stage: "ProcessBasicInformation",
                os_error: Some(st),
            });
        }
        Ok(Identity {
            pid,
            parent: bi.parent_pid as u32,
            process_group: 0, // no POSIX process groups on Windows
            birth_seconds,
            birth_microseconds,
            pointer_width: std::mem::size_of::<usize>(),
            executable: wide_to_bytes(&path[..len as usize]),
        })
    }

    fn command_line(p: &OwnedProcess) -> Result<Vec<u16>, Failure> {
        // u64 storage keeps the UNICODE_STRING header pointer-aligned.
        let mut words = vec![0u64; 512];
        for _ in 0..2 {
            let cap = (words.len() * 8) as u32;
            let mut need = 0u32;
            // SAFETY: writable buffer of `cap` bytes; `need` receives the required size.
            let st = unsafe {
                NtQueryInformationProcess(
                    p.0,
                    ProcessCommandLineInformation,
                    words.as_mut_ptr().cast(),
                    cap,
                    &mut need,
                )
            };
            if st == STATUS_INFO_LENGTH_MISMATCH
                && (need as usize) <= CMDLINE_CAP_BYTES
                && need > cap
            {
                words.resize((need as usize + 7) / 8, 0);
                continue;
            }
            if st < 0 {
                return Err(Failure::Unknown {
                    stage: "ProcessCommandLineInformation",
                    os_error: Some(st),
                });
            }
            let header = std::mem::size_of::<UnicodeString>();
            if (need as usize) < header || need > cap {
                return Err(Failure::Malformed("command line length"));
            }
            // SAFETY: the buffer holds at least `header` initialized bytes, suitably aligned.
            let us = unsafe { std::ptr::read(words.as_ptr() as *const UnicodeString) };
            let units = us.length as usize / 2;
            let base = words.as_ptr() as usize;
            let ptr = us.buffer as usize;
            if us.length % 2 != 0 || ptr < base + header || ptr + units * 2 > base + need as usize {
                return Err(Failure::Malformed("command line buffer"));
            }
            // SAFETY: the range [ptr, ptr + units*2) was just shown to lie inside our buffer.
            let wide =
                unsafe { std::slice::from_raw_parts(us.buffer as *const u16, units) }.to_vec();
            return Ok(wide);
        }
        Err(Failure::Unknown {
            stage: "command line resize",
            os_error: None,
        })
    }

    /// Read identity, command line, identity again through ONE retained handle.
    pub(super) fn observe_handle(
        p: &OwnedProcess,
        pid: u32,
    ) -> Result<ProcessObservation, Failure> {
        if exited(p)? {
            return Err(Failure::Dead);
        }
        let before = identity(p, pid)?;
        let wide = command_line(p)?;
        if identity(p, pid)? != before {
            return Err(Failure::Changed);
        }
        if exited(p)? {
            return Err(Failure::Dead); // an exited process still answers queries
        }
        let argv = parse_windows_command_line(&wide)
            .into_iter()
            .map(|a| wide_to_bytes(&a))
            .collect();
        Ok(ProcessObservation {
            invocation: Invocation {
                launch_path: before.executable.clone(),
                argv,
            },
            identity: before,
        })
    }

    pub(super) fn observe(pid: u32) -> Result<ProcessObservation, Failure> {
        let p = OwnedProcess::open(pid)?;
        observe_handle(&p, pid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blob(path: &[u8], args: &[&[u8]], width: usize) -> Vec<u8> {
        let mut b = (args.len() as i32).to_ne_bytes().to_vec();
        b.extend_from_slice(path);
        b.push(0);
        while (b.len() - 4) % width != 0 {
            b.push(0);
        }
        for arg in args {
            b.extend_from_slice(arg);
            b.push(0);
        }
        b.extend_from_slice(b"DUMMY_ENV=not_an_argument\0");
        b
    }

    fn id() -> Identity {
        Identity {
            pid: 42,
            parent: 1,
            process_group: 42,
            birth_seconds: 1,
            birth_microseconds: 123,
            pointer_width: 8,
            executable: b"/bin/node".to_vec(),
        }
    }

    fn invocation() -> Invocation {
        Invocation {
            launch_path: b"node".to_vec(),
            argv: vec![b"node".to_vec(), b"agent.js".to_vec()],
        }
    }

    #[test]
    fn keeps_empty_and_non_utf8_arguments_at_every_alignment() {
        for width in [4, 8] {
            for len in 1..=width {
                let path = vec![b'x'; len];
                let args: &[&[u8]] = &[b"", b"script.js", b"", b"a b", b"\xff", b""];
                let got = parse_darwin_invocation(&blob(&path, args, width), width).unwrap();
                assert_eq!(got.launch_path, path);
                assert_eq!(got.argv, args);
            }
        }
    }

    #[test]
    fn stops_at_argc_before_environment() {
        let b = blob(b"/node", &[b"node", b""], 8);
        assert_eq!(
            parse_darwin_invocation(&b, 8).unwrap().argv,
            [b"node".to_vec(), vec![]]
        );
    }

    #[test]
    fn refuses_malformed_header_padding_and_truncation() {
        let b = blob(b"x", &[b"a"], 8);
        for width in [0, 1, 16] {
            assert!(parse_darwin_invocation(&b, width).is_err());
        }
        for len in 0..14 {
            assert!(parse_darwin_invocation(&b[..len], 8).is_err());
        }
        for argc in [0i32, -1, i32::MAX] {
            let mut bad = b.clone();
            bad[..4].copy_from_slice(&argc.to_ne_bytes());
            assert!(parse_darwin_invocation(&bad, 8).is_err());
        }
        let mut bad = b.clone();
        bad[6] = 1;
        assert!(parse_darwin_invocation(&bad, 8).is_err());
        let mut bad = b;
        bad[4] = 0;
        assert!(parse_darwin_invocation(&bad, 8).is_err());
    }

    #[test]
    fn observation_reads_identity_twice_and_preserves_bytes() {
        let mut calls = 0;
        let got = read_consistent(
            || {
                calls += 1;
                Ok(id())
            },
            |width| {
                assert_eq!(width, 8);
                Ok(invocation())
            },
        )
        .unwrap();
        assert_eq!(calls, 2);
        assert_eq!(
            got,
            ProcessObservation {
                identity: id(),
                invocation: invocation()
            }
        );
    }

    #[test]
    fn reused_pid_exec_and_ancestry_changes_refuse_observation() {
        for field in 0..6 {
            let first = id();
            let mut changed = first.clone();
            match field {
                0 => changed.birth_microseconds += 1,
                1 => changed.executable = b"/bin/sh".to_vec(),
                2 => changed.parent += 1,
                3 => changed.process_group += 1,
                4 => changed.pointer_width = 4,
                _ => changed.pid += 1,
            }
            let mut samples = [Ok(first), Ok(changed)].into_iter();
            assert_eq!(
                read_consistent(|| samples.next().unwrap(), |_| Ok(invocation())),
                Err(Failure::Changed)
            );
        }
    }

    #[test]
    fn unavailable_is_not_dead_or_a_known_observation() {
        let denied = Failure::Unknown {
            stage: "identity",
            os_error: Some(1),
        };
        assert_eq!(
            read_consistent(|| Err(denied.clone()), |_| panic!("must not read argv")),
            Err(denied)
        );
        assert_eq!(
            read_consistent(
                || Ok(id()),
                |_| Err(Failure::Unknown {
                    stage: "argv",
                    os_error: Some(1)
                })
            ),
            Err(Failure::Unknown {
                stage: "argv",
                os_error: Some(1)
            })
        );
        let mut samples = [Ok(id()), Err(Failure::Dead)].into_iter();
        assert_eq!(
            read_consistent(|| samples.next().unwrap(), |_| Ok(invocation())),
            Err(Failure::Dead)
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn native_owned_child_is_observed_and_reaped_death_is_distinct() {
        use std::process::{Command, Stdio};
        // Owned, sanitized child only: cat blocks on its pipe until the parent sends EOF.
        let mut child = Command::new("/bin/cat")
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let observed = observe(child.id());
        let checked = observed.as_ref().ok().map(recheck);
        let changed_argv = observed.as_ref().ok().map(|got| {
            let mut old = got.clone();
            old.invocation.argv.push(b"different-script.js".to_vec());
            recheck(&old)
        });
        let changed_exe = observed.as_ref().ok().map(|got| {
            let mut old = got.clone();
            old.identity.executable = b"/bin/sh".to_vec();
            recheck(&old)
        });
        drop(child.stdin.take());
        assert!(child.wait().unwrap().success());
        let got = observed.unwrap();
        assert_eq!(got.identity.pid, child.id());
        assert_eq!(got.identity.parent, std::process::id());
        assert!(got.identity.birth_seconds > 0);
        assert_eq!(got.identity.executable, b"/bin/cat");
        assert_eq!(got.invocation.argv, [b"/bin/cat".to_vec()]);
        assert_eq!(checked.unwrap(), Ok(()));
        assert_eq!(changed_argv.unwrap(), Err(Failure::Changed));
        assert_eq!(changed_exe.unwrap(), Err(Failure::Changed));
        assert_eq!(observe(child.id()), Err(Failure::Dead));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn native_empty_argv0_is_preserved_without_consuming_environment() {
        use std::os::unix::process::CommandExt;
        use std::process::{Command, Stdio};
        let mut child = Command::new("/bin/cat")
            .arg0("")
            .env_clear()
            .env("DUMMY_ENV", "not_an_argument")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let observed = observe(child.id());
        drop(child.stdin.take());
        assert!(child.wait().unwrap().success());
        let got = observed.unwrap();
        assert_eq!(got.identity.executable, b"/bin/cat");
        // Parsing an empty argv0 is valid; a CLI invocation matcher must still
        // reject it as evidence for a supported adapter invocation.
        assert_eq!(got.invocation.argv, [Vec::<u8>::new()]);
    }

    #[test]
    fn invalid_pid_is_not_a_native_query() {
        for pid in [0, u32::MAX] {
            assert_eq!(observe(pid), Err(Failure::InvalidPid));
        }
    }

    #[test]
    fn identity_query_distinguishes_absence_denial_and_short_read() {
        assert_eq!(
            identity_query_result(0, 136, Some(libc::ESRCH)),
            Err(Failure::Dead)
        );
        assert_eq!(
            identity_query_result(0, 136, Some(libc::EPERM)),
            Err(Failure::Unknown {
                stage: "proc_pidinfo",
                os_error: Some(libc::EPERM)
            })
        );
        // A short successful call must not use stale errno as evidence of death.
        assert_eq!(
            identity_query_result(64, 136, Some(libc::ESRCH)),
            Err(Failure::Unknown {
                stage: "proc_pidinfo",
                os_error: None
            })
        );
        assert_eq!(identity_query_result(136, 136, Some(libc::EPERM)), Ok(()));
    }

    #[test]
    fn argument_query_refuses_saturation_denial_and_truncated_returned_length() {
        let b = blob(b"x", &[b"a"], 8);
        assert_eq!(
            argument_query_result(&b, b.len(), 0, None, 8),
            Err(Failure::Unknown {
                stage: "saturated arguments",
                os_error: None
            })
        );
        assert_eq!(
            argument_query_result(&b, b.len() + 1, 0, None, 8),
            Err(Failure::Unknown {
                stage: "saturated arguments",
                os_error: None
            })
        );
        for error in [libc::EINVAL, libc::EPERM, libc::ESRCH] {
            assert_eq!(
                argument_query_result(&b, 14, -1, Some(error), 8),
                Err(Failure::Unknown {
                    stage: "KERN_PROCARGS2",
                    os_error: Some(error)
                })
            );
        }
        assert!(argument_query_result(&b, 13, 0, None, 8).is_err());
        assert_eq!(
            argument_query_result(&b, 14, 0, Some(libc::EPERM), 8)
                .unwrap()
                .argv,
            [b"a".to_vec()]
        );
    }

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    fn args(cmd: &str) -> Vec<String> {
        parse_windows_command_line(&w(cmd))
            .into_iter()
            .map(|a| String::from_utf16(&a).unwrap())
            .collect()
    }

    #[test]
    fn windows_command_line_follows_msvc_argv_rules() {
        // argv0: quotes toggle, backslashes are literal.
        assert_eq!(
            args(r#""C:\Program Files\nodejs\node.exe" C:\npm\codex.js --flag"#),
            [
                r"C:\Program Files\nodejs\node.exe",
                r"C:\npm\codex.js",
                "--flag"
            ]
        );
        assert_eq!(args(r#"C:\a\b\"c d" e"#)[0], r"C:\a\b\c d");
        // 2n backslashes + quote -> n backslashes, quote toggles; 2n+1 -> n backslashes + literal quote.
        assert_eq!(args(r#"x a\\"b c" d"#), ["x", r"a\b c", "d"]);
        assert_eq!(args(r#"x a\"b c"#), ["x", r#"a"b"#, "c"]);
        // Backslashes not before a quote are literal; "" inside quotes is a literal quote.
        assert_eq!(args(r#"x a\b "c""d" """#), ["x", r"a\b", r#"c"d"#, ""]);
        // Empty quoted argument and tab separators.
        assert_eq!(args("x\t\"\"\ty"), ["x", "", "y"]);
    }

    #[test]
    fn wide_bytes_are_lossless_for_lone_surrogates() {
        assert_eq!(wide_to_bytes(&w("claude.exe")), b"claude.exe");
        let a = wide_to_bytes(&[0x61, 0xD800, 0x62]);
        let b = wide_to_bytes(&[0x61, 0xDC00, 0x62]);
        assert_ne!(a, b, "two different native paths must not merge");
        assert_eq!(a.len(), 5);
    }

    #[test]
    fn filetime_converts_to_unix_microseconds_and_refuses_pre_epoch() {
        // 2026-10-16T00:00:00Z = 1792108800 s.
        let ticks = 116_444_736_000_000_000 + 1_792_108_800 * 10_000_000 + 1_234_567;
        assert_eq!(filetime_to_unix(ticks), Some((1_792_108_800, 123_456)));
        assert_eq!(filetime_to_unix(1), None);
    }

    #[test]
    fn linux_stat_anchors_on_last_paren_and_rejects_zombies() {
        let stat = b"4321 (a) b) c) S 77 4300 4300 0 -1 4194560 1 0 0 0 0 0 0 0 20 0 1 0 98765 0 0";
        assert_eq!(parse_linux_stat(stat).unwrap(), (77, 4300, 98765));
        let zombie = b"9 (x) Z 1 9 9 0 -1 0 0 0 0 0 0 0 0 0 20 0 1 0 5 0 0";
        assert_eq!(parse_linux_stat(zombie), Err(Failure::Dead));
        assert!(parse_linux_stat(b"9 (x) S 1 9").is_err());
        assert!(parse_linux_stat(b"no paren").is_err());
        let zero_start = b"9 (x) S 1 9 9 0 -1 0 0 0 0 0 0 0 0 0 20 0 1 0 0 0 0";
        assert!(matches!(
            parse_linux_stat(zero_start),
            Err(Failure::Unknown { .. })
        ));
    }

    #[test]
    fn linux_cmdline_keeps_empty_arguments_and_refuses_empty() {
        assert_eq!(
            parse_linux_cmdline(b"node\0\0cli.js\0").unwrap(),
            [b"node".to_vec(), vec![], b"cli.js".to_vec()]
        );
        assert!(matches!(
            parse_linux_cmdline(b""),
            Err(Failure::Unknown { .. })
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn native_retained_rechecks_same_process_and_reports_death() {
        use std::process::{Command, Stdio};
        let mut child = Command::new("/bin/cat")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .env_clear()
            .spawn()
            .unwrap();
        let pid = child.id();
        let r = Retained::open(pid).unwrap();
        assert_eq!(r.observation().identity.pid, pid);
        assert_eq!(r.recheck(), Ok(()));
        drop(child.stdin.take());
        child.wait().unwrap();
        // After reaping, the pid is gone: a recheck must be Dead (or Changed on instant reuse), never Ok.
        let after = r.recheck();
        assert!(
            matches!(after, Err(Failure::Dead) | Err(Failure::Changed)),
            "{after:?}"
        );
        assert_eq!(Retained::open(0).unwrap_err(), Failure::InvalidPid);
    }
}
