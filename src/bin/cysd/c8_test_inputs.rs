// Test-only private scratch creation, included separately by the two fixture modules.
// No process environment is mutated. Caller-supplied roots keep precedence.
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT_ROOT: AtomicU64 = AtomicU64::new(1);

pub(super) fn private_root(variable: &str, kind: &str) -> io::Result<PathBuf> {
    assert!(matches!(kind, "p" | "n"));
    let explicit = std::env::var_os(variable);
    let mut base = explicit.as_ref().map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    #[cfg(unix)] {
        use std::os::unix::ffi::OsStrExt;
        // Honor an already-short TMPDIR (including the isolated C8 runner). macOS's
        // default per-user temporary directory can be too long for later socket names.
        // Reserve the longest generated child name and 32 bytes for a socket suffix.
        if explicit.is_none() {
            let canonical = base.canonicalize()?;
            if canonical.as_os_str().as_bytes().len() + 1 + 29 + 32 >= 104 {
                base = PathBuf::from("/tmp");
            }
        }
    }
    let base = base.canonicalize()?;
    if !base.is_dir() { return Err(io::Error::other("C8 fixture root is not a directory")); }
    for _ in 0..256 {
        let sequence = NEXT_ROOT.fetch_update(Ordering::Relaxed, Ordering::Relaxed,
            |n| n.checked_add(1)).map_err(|_| io::Error::other("C8 fixture root counter exhausted"))?;
        let path = base.join(format!("c8{kind}-{:x}-{sequence:x}", std::process::id()));
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)] { use std::os::unix::fs::DirBuilderExt; builder.mode(0o700); }
        match builder.create(&path) {
            Ok(()) => return Ok(path), // Exclusive creation; an existing path is never adopted.
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::other("C8 fixture root collision budget exhausted"))
}
