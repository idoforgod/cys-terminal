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
    Changed,
    UnsupportedPlatform,
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

#[cfg(any(target_os = "macos", test))]
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
    #[cfg(not(target_os = "macos"))]
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
        if n != size {
            let error = (n <= 0)
                .then(|| std::io::Error::last_os_error().raw_os_error())
                .flatten();
            return Err(if n <= 0 && error == Some(libc::ESRCH) {
                Failure::Dead
            } else {
                unknown("proc_pidinfo", error)
            });
        }
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
        if rc != 0 {
            // EINVAL also means access/stack/task failure, not conclusive death.
            return Err(unknown(
                "KERN_PROCARGS2",
                std::io::Error::last_os_error().raw_os_error(),
            ));
        }
        if len >= bytes.len() {
            return Err(unknown("saturated arguments", None));
        }
        parse_darwin_invocation(&bytes[..len], width)
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

    #[test]
    fn invalid_pid_is_not_a_native_query() {
        for pid in [0, u32::MAX] {
            assert_eq!(observe(pid), Err(Failure::InvalidPid));
        }
    }
}
