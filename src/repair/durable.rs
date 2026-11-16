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

/// Unit WY2 — opens an EXISTING file that was NOT just written by the caller (the vault's sealed
/// copy in `vault::finish_seal_with` / `seal_with`, the screen's verdict off unix in
/// `screen::write_verdict_in`) so that [`full_flush_file`] (or the ledger's twin) can flush it. A file
/// the caller writes itself is flushed on the handle that wrote it ([`write_durable`]; WY2 FR1, RULINGS
/// LH15). unix: `File::open`, exactly the call every site made before (a read-only descriptor may be
/// `fsync`ed / `F_FULLFSYNC`ed). Other platforms: write ACCESS — no create, no truncate, no append;
/// Windows' `FlushFileBuffers` needs a handle with write access ("The file handle must have the
/// GENERIC_WRITE access right", Microsoft's FlushFileBuffers reference) and answers a read-only
/// handle with ERROR_ACCESS_DENIED (5) — measured on the Windows runner as `cys repair commit`
/// exit 1 "record.json: Access is denied. (os error 5)" (runs/TW1-ci-7a). No byte is written
/// through this handle and a missing file is an error (never created). The flush stays the
/// caller's and stays fail-closed.
pub fn open_for_flush(p: &Path) -> std::io::Result<std::fs::File> {
    #[cfg(unix)]
    {
        std::fs::File::open(p)
    }
    #[cfg(not(unix))]
    {
        std::fs::OpenOptions::new().write(true).open(p)
    }
}

fn io(p: &Path, e: impl std::fmt::Display) -> RepairError {
    RepairError::Io(format!("{}: {e}", p.display()))
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

/// RULINGS GK5 — the size cap of one engine record (a record, a lane row, an intent, an operation
/// record): larger is never read, it is unreadable.
pub const RECORD_READ_CAP: u64 = 16 * 1024 * 1024;

/// Tests (RULINGS IE4 / IG1): the bytes each read of the engine took from each path, and a hook run
/// right after a file was opened (to make it grow while it is read).
#[cfg(test)]
pub(crate) mod read_seam {
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};
    type Hook = Box<dyn FnMut(&Path)>;
    thread_local! {
        static READS: RefCell<Vec<(PathBuf, u64)>> = const { RefCell::new(Vec::new()) };
        static AFTER_OPEN: RefCell<Option<Hook>> = const { RefCell::new(None) };
    }
    pub(crate) fn count(p: &Path, n: u64) {
        READS.with(|r| r.borrow_mut().push((p.to_path_buf(), n)));
    }
    /// The bytes this thread's reads took from `p` so far.
    pub(crate) fn bytes_read_from(p: &Path) -> u64 {
        READS.with(|r| r.borrow().iter().filter(|(q, _)| q == p).map(|(_, n)| n).sum())
    }
    pub(crate) fn after_open(p: &Path) {
        AFTER_OPEN.with(|h| {
            if let Some(f) = h.borrow_mut().as_mut() {
                f(p)
            }
        })
    }
    pub(crate) fn set_after_open(f: Option<Hook>) {
        AFTER_OPEN.with(|h| *h.borrow_mut() = f);
    }
}

/// The read seam's hook right after a file was opened — a no-op outside tests (callers in files whose
/// source pins cut at the first test attribute call this instead of naming the seam).
pub(crate) fn seam_after_open(p: &Path) {
    #[cfg(test)]
    read_seam::after_open(p);
    #[cfg(not(test))]
    let _ = p;
}

/// The read seam's count of the bytes one read took — a no-op outside tests.
pub(crate) fn seam_count(p: &Path, n: u64) {
    #[cfg(test)]
    read_seam::count(p, n);
    #[cfg(not(test))]
    let _ = (p, n);
}

/// What [`read_record_bytes`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordRead {
    Bytes(Vec<u8>),
    /// The path does not exist (the caller decides what positive evidence of absence it needs).
    NotFound,
    Unreadable(String),
}

/// RULINGS GK5 — THE bounded reader of the engine's own records: a non-blocking open that follows
/// no link (unix: `O_NONBLOCK | O_NOFOLLOW`; Windows: the reparse point itself), a REGULAR file only,
/// at most [`RECORD_READ_CAP`] bytes, read whole — a special file (a FIFO never blocks: it is
/// refused before any read), a link, an oversized file or a size that changed during the read is
/// unreadable. It never waits and never retries.
pub fn read_record_bytes(p: &Path) -> RecordRead {
    read_bounded(p, false)
}

/// RULINGS IZ5 — [`read_record_bytes`] for a file the engine does NOT own whose link a user may have
/// placed (the agents' settings file): a link is followed, and what it reaches must be a REGULAR file
/// (checked through the open handle) — the same non-blocking open, the same one handle and limit.
pub fn read_bounded_following(p: &Path) -> RecordRead {
    read_bounded(p, true)
}

fn read_bounded(p: &Path, follow: bool) -> RecordRead {
    use std::io::Read;
    let mut opts = std::fs::OpenOptions::new();
    opts.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC | if follow { 0 } else { libc::O_NOFOLLOW });
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        if !follow {
            opts.custom_flags(0x0020_0000); // FILE_FLAG_OPEN_REPARSE_POINT: a link is opened as itself
        }
    }
    let f = match opts.open(p) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return RecordRead::NotFound,
        Err(e) => return RecordRead::Unreadable(format!("{}: 열 수 없음(링크이거나 읽을 수 없음) — {e}", p.display())),
    };
    let m = match f.metadata() {
        Ok(m) => m,
        Err(e) => return RecordRead::Unreadable(format!("{}: {e}", p.display())),
    };
    if !m.file_type().is_file() {
        return RecordRead::Unreadable(format!("{}: 일반 파일이 아님", p.display()));
    }
    if m.len() > RECORD_READ_CAP {
        return RecordRead::Unreadable(format!("{}: 크기 {}바이트가 한도 {}바이트를 넘음", p.display(), m.len(), RECORD_READ_CAP));
    }
    #[cfg(test)]
    read_seam::after_open(p);
    let mut buf = Vec::with_capacity(m.len() as usize);
    let took = (&f).take(RECORD_READ_CAP + 1).read_to_end(&mut buf);
    #[cfg(test)]
    read_seam::count(p, buf.len() as u64);
    match took {
        Ok(_) if buf.len() as u64 == m.len() => RecordRead::Bytes(buf),
        // RULINGS IE4: a file that grew past the limit while it was read says the limit
        Ok(_) if buf.len() as u64 > RECORD_READ_CAP => RecordRead::Unreadable(format!("{}: 읽는 동안 한도 {}바이트를 넘음", p.display(), RECORD_READ_CAP)),
        Ok(_) => RecordRead::Unreadable(format!("{}: 읽는 동안 크기가 바뀜({} → {})", p.display(), m.len(), buf.len())),
        Err(e) => RecordRead::Unreadable(format!("{}: {e}", p.display())),
    }
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

    /// GK5: THE bounded reader — a regular file is read whole; a missing one is NotFound; a FIFO
    /// (no writer), a link and a file over the cap are unreadable, each at once.
    #[cfg(unix)]
    #[test]
    fn the_record_reader_is_bounded() {
        let d = tmp("gk5");
        let ok = d.join("r.json");
        std::fs::write(&ok, b"{}").unwrap();
        assert_eq!(read_record_bytes(&ok), RecordRead::Bytes(b"{}".to_vec()));
        assert_eq!(read_record_bytes(&d.join("none.json")), RecordRead::NotFound);
        let fifo = d.join("fifo.json");
        let c = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
        // SAFETY: a valid NUL-terminated path.
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o644) }, 0);
        assert!(matches!(read_record_bytes(&fifo), RecordRead::Unreadable(_)), "a FIFO is never read");
        let link = d.join("link.json");
        std::os::unix::fs::symlink(&ok, &link).unwrap();
        assert!(matches!(read_record_bytes(&link), RecordRead::Unreadable(_)), "a link is not followed");
        let big = d.join("big.json");
        std::fs::File::create(&big).unwrap().set_len(RECORD_READ_CAP + 1).unwrap();
        assert!(matches!(read_record_bytes(&big), RecordRead::Unreadable(_)), "over the cap");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// WY2 — the open that serves a flush: both full flushes (this module's and the ledger's twin)
    /// ACCEPT its handle, and the open changes nothing — the same bytes, length, read-only flag and
    /// modification time; a missing file is an error and is never created. Plain std::fs in a temp
    /// folder: runs on every platform (on Windows it is the proof of the non-unix arm).
    #[test]
    fn wy2_open_for_flush_gives_a_handle_the_full_flush_accepts() {
        let d = tmp("wy2-open");
        let p = d.join("record.json");
        std::fs::write(&p, b"{\"a\":1}\n").unwrap();
        let before = std::fs::metadata(&p).unwrap();
        let f = open_for_flush(&p).expect("open an existing file for its flush");
        full_flush_file(&f).expect("the full flush accepts the handle");
        crate::repair::ledger::full_flush_file(&f).expect("the ledger's full flush accepts it too");
        drop(f);
        let after = std::fs::metadata(&p).unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"{\"a\":1}\n", "no byte changed");
        assert_eq!(after.len(), before.len(), "the length is unchanged (no truncation)");
        assert_eq!(after.permissions().readonly(), before.permissions().readonly(), "the read-only flag is unchanged");
        assert_eq!(after.modified().unwrap(), before.modified().unwrap(), "the open and the flush leave the modification time");
        let missing = d.join("missing.json");
        assert!(open_for_flush(&missing).is_err(), "a missing file is an error");
        assert!(std::fs::symlink_metadata(&missing).is_err(), "and it is never created");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// WY2 — the CAUSE, measured where it lives: Windows refuses the flush of a READ-ONLY handle
    /// with ERROR_ACCESS_DENIED (5) — the shape of the runner's message (runs/TW1-ci-7a commit.json).
    /// A hypothesis until this runs on Windows (not compiled on unix, where a read-only descriptor
    /// may be flushed).
    #[cfg(windows)]
    #[test]
    fn wy2_windows_refuses_the_flush_of_a_read_only_handle() {
        let d = tmp("wy2-ro");
        let p = d.join("record.json");
        std::fs::write(&p, b"{}\n").unwrap();
        let ro = std::fs::File::open(&p).unwrap();
        let e = full_flush_file(&ro).expect_err("FlushFileBuffers on a read-only handle");
        assert_eq!(e.raw_os_error(), Some(5), "{e}");
        drop(ro);
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
