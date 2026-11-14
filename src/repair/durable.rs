//! Durable writes (O-C1-4 · RULINGS E1: file flush, rename, directory flush; on macOS every flush
//! is a FULL flush — `F_FULLFSYNC`). Measured: plain `fsync(2)` on both parent directories lost the
//! exchange in 10 of 10 simulated power losses, `F_FULLFSYNC` kept it 10 of 10
//! (`impl/measure/Mb-atomic-exchange.md` rows 6). Used for every "확정" of rule one: the operation
//! record and its directory, the staged slot and its directories, both parents after an exchange,
//! and the ledger files that must never run ahead of what they describe (tombstones — AK7).

use super::types::RepairError;
use std::path::Path;

#[cfg(test)]
thread_local! {
    /// Test-only: how many full flushes (`F_FULLFSYNC` that succeeded) this thread issued — the
    /// E1 pin counts them so a regression to plain `fsync` fails a test.
    pub(crate) static FULL_FLUSHES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    /// Test-only (RULINGS CT2): the next full flushes of this thread fail with this errno, as many
    /// times as the count says (`(errno, count)`; count 0 = off).
    pub(crate) static INJECT_FLUSH_ERRNO: std::cell::Cell<(i32, u32)> = const { std::cell::Cell::new((0, 0)) };
}

#[cfg(test)]
fn injected_errno() -> Option<i32> {
    INJECT_FLUSH_ERRNO.with(|c| {
        let (errno, n) = c.get();
        (n > 0).then(|| {
            c.set((errno, n - 1));
            errno
        })
    })
}

/// RULINGS CT2: the errno values by which a volume says "this flush is not supported here" — the
/// ONLY failures of `F_FULLFSYNC` after which a plain `fsync` stands in: `ENOTSUP` (45 on macOS;
/// what SMB / AFP mounts and some FUSE volumes answer — the same single value Go's runtime falls
/// back on, `internal/poll/fd_fsync_darwin.go`) and `EOPNOTSUPP` (102 on macOS, its socket-layer
/// twin some network filesystems pass up). `EINTR` is retried. Every other errno (`EIO`, `ENOSPC`,
/// `EBADF`, `EROFS`, `EDQUOT`, …) is an error: the caller declines and keeps its evidence.
#[cfg(target_os = "macos")]
pub const FLUSH_NOT_SUPPORTED_ERRNOS: [i32; 2] = [libc::ENOTSUP, libc::EOPNOTSUPP];

/// The full flush of one open file or directory handle: `fcntl(F_FULLFSYNC)` on macOS; a plain
/// `fsync` stands in ONLY when the volume answers one of [`FLUSH_NOT_SUPPORTED_ERRNOS`] (RULINGS
/// CT2 — fails closed on every other errno); `sync_all` elsewhere (`fsync` on Linux,
/// `FlushFileBuffers` on Windows).
pub fn full_flush_file(f: &std::fs::File) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::io::AsRawFd;
        loop {
            #[cfg(test)]
            let injected = injected_errno();
            #[cfg(not(test))]
            let injected: Option<i32> = None;
            // SAFETY: fcntl on a descriptor borrowed for the duration of the call.
            let rc = match injected {
                Some(_) => -1,
                None => unsafe { libc::fcntl(f.as_raw_fd(), libc::F_FULLFSYNC) },
            };
            if rc == 0 {
                #[cfg(test)]
                FULL_FLUSHES.with(|c| c.set(c.get() + 1));
                return Ok(());
            }
            let errno = injected.unwrap_or_else(|| std::io::Error::last_os_error().raw_os_error().unwrap_or(0));
            if errno == libc::EINTR {
                continue;
            }
            if FLUSH_NOT_SUPPORTED_ERRNOS.contains(&errno) {
                // SAFETY: fsync on a descriptor borrowed for the duration of the call.
                return match unsafe { libc::fsync(f.as_raw_fd()) } {
                    0 => Ok(()),
                    _ => Err(std::io::Error::last_os_error()),
                };
            }
            return Err(std::io::Error::from_raw_os_error(errno));
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        #[cfg(test)]
        if let Some(errno) = injected_errno() {
            return Err(std::io::Error::from_raw_os_error(errno));
        }
        f.sync_all()
    }
}

/// Full flush of a directory (unix). Windows has no directory flush (the rename is journaled by
/// NTFS; there is no handle to flush) — a no-op there.
pub fn full_flush_dir(dir: &Path) -> Result<(), RepairError> {
    #[cfg(unix)]
    {
        let f = std::fs::File::open(dir).map_err(|e| io(dir, e))?;
        full_flush_file(&f).map_err(|e| io(dir, e))?;
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
    }
    Ok(())
}

/// Writes `bytes` to `path` durably: a temporary sibling (created new, never over anything), the
/// bytes, a full flush of the file, an atomic rename onto `path`, a full flush of the directory.
/// The parent directory is created when missing (and flushed with its own parent). Only LEDGER
/// files are written this way — a live pack file is never written by rename-over (rule one).
pub fn write_durable(path: &Path, bytes: &[u8]) -> Result<(), RepairError> {
    use std::io::Write;
    let parent = path
        .parent()
        .ok_or_else(|| RepairError::Io(format!("{}: no parent directory", path.display())))?;
    if std::fs::symlink_metadata(parent).is_err() {
        std::fs::create_dir_all(parent).map_err(|e| io(parent, e))?;
        if let Some(gp) = parent.parent() {
            full_flush_dir(gp)?;
        }
    }
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("ledger");
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = parent.join(format!(".{name}.tmp-{}-{nanos}", std::process::id()));
    let result = (|| -> Result<(), RepairError> {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
            .map_err(|e| io(&tmp, e))?;
        f.write_all(bytes).map_err(|e| io(&tmp, e))?;
        full_flush_file(&f).map_err(|e| io(&tmp, e))?;
        drop(f);
        std::fs::rename(&tmp, path).map_err(|e| io(path, e))?;
        full_flush_dir(parent)
    })();
    if result.is_err() {
        // our own temporary file only (never a ledger file, never a slot, never a live path)
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// [`write_durable`] of a JSON value — the SAME bytes `crate::atomic_write_json` writes (one-space
/// pretty printer, trailing LF), made durable.
pub fn write_json_durable<T: serde::Serialize>(path: &Path, v: &T) -> Result<(), RepairError> {
    let mut bytes = Vec::new();
    let fmt = serde_json::ser::PrettyFormatter::with_indent(b" ");
    let mut ser = serde_json::Serializer::with_formatter(&mut bytes, fmt);
    v.serialize(&mut ser)
        .map_err(|e| RepairError::Io(format!("{}: {e}", path.display())))?;
    bytes.push(b'\n');
    write_durable(path, &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!(
            "cys-durable-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// E1 pin: on macOS a durable write issues FULL flushes (the file and its directory) — a
    /// regression to plain `fsync`/`sync_all` leaves the counter at zero and fails here.
    #[test]
    fn write_durable_issues_full_flushes_on_macos() {
        let d = tmp("ff");
        let p = d.join("sub").join("x.json");
        let before = FULL_FLUSHES.with(|c| c.get());
        write_durable(&p, b"{\"a\":1}\n").unwrap();
        let after = FULL_FLUSHES.with(|c| c.get());
        assert_eq!(std::fs::read(&p).unwrap(), b"{\"a\":1}\n");
        if cfg!(target_os = "macos") {
            assert!(
                after >= before + 2,
                "expected >= 2 F_FULLFSYNC (file + directory), saw {}",
                after - before
            );
        }
        // overwrite keeps the new bytes and leaves no temporary sibling behind
        write_durable(&p, b"two").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"two");
        let left: Vec<_> = std::fs::read_dir(d.join("sub"))
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(left, vec!["x.json".to_string()], "temporary sibling left: {left:?}");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// RULINGS CT2: a failure of the mandatory full flush other than "not supported" is an ERROR —
    /// the durable write fails, the target keeps its old bytes, no temporary sibling is left; a
    /// "not supported" errno (ENOTSUP) lets a plain fsync stand in and the write succeeds.
    #[cfg(target_os = "macos")]
    #[test]
    fn full_flush_failure_fails_closed_except_not_supported() {
        let d = tmp("ct2");
        let p = d.join("x.json");
        write_durable(&p, b"old").unwrap();
        INJECT_FLUSH_ERRNO.with(|c| c.set((libc::EIO, 1)));
        let r = write_durable(&p, b"new");
        INJECT_FLUSH_ERRNO.with(|c| c.set((0, 0)));
        assert!(r.is_err(), "EIO on the full flush must fail the write");
        assert_eq!(std::fs::read(&p).unwrap(), b"old", "the target keeps its bytes");
        let left = std::fs::read_dir(&d).unwrap().count();
        assert_eq!(left, 1, "no temporary sibling left");
        for errno in FLUSH_NOT_SUPPORTED_ERRNOS {
            INJECT_FLUSH_ERRNO.with(|c| c.set((errno, 1)));
            let r = write_durable(&p, b"fallback");
            INJECT_FLUSH_ERRNO.with(|c| c.set((0, 0)));
            assert!(r.is_ok(), "errno {errno} = not supported → plain fsync stands in: {r:?}");
        }
        INJECT_FLUSH_ERRNO.with(|c| c.set((libc::EINTR, 1)));
        assert!(write_durable(&p, b"intr").is_ok(), "EINTR is retried");
        INJECT_FLUSH_ERRNO.with(|c| c.set((0, 0)));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn full_flush_dir_on_missing_dir_is_an_error() {
        let d = tmp("missing");
        assert!(full_flush_dir(&d.join("nope")).is_err());
        assert!(full_flush_dir(&d).is_ok());
        let _ = std::fs::remove_dir_all(&d);
    }
}
