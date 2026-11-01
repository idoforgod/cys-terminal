//! Diagnostic-only recorder. No producer logic, policy exemption, or receiver oracle.
//! I/K MUST be built from the product pin without this module/instrumentation.
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

struct Session { root: PathBuf, start: Instant, seq: u64, failed: bool }
static SESSION: OnceLock<Mutex<Session>> = OnceLock::new();

pub(crate) fn enabled() -> bool { SESSION.get().is_some() }

pub(crate) fn initialize(root: &Path) -> Result<(), String> {
    if root.exists() { return Err("trace destination already exists".into()); }
    let parent = root.parent().ok_or("trace root parent absent")?;
    let marker = parent.join("owned-run.json");
    if !marker.is_file() || !parent.file_name().unwrap_or_default().to_string_lossy().starts_with("cys-native-") {
        return Err("not an owned diagnostic root".into());
    }
    std::fs::create_dir(root).map_err(|e| e.to_string())?;
    SESSION.set(Mutex::new(Session { root: root.to_path_buf(), start: Instant::now(), seq: 0, failed: false }))
        .map_err(|_| "trace session already initialized".to_string())?;
    emit("session", 0, || json!({"pid":std::process::id(),"build":env!("CYS_BUILD_ID"),"release_verdict":"NONE"}));
    Ok(())
}

pub(crate) fn emit(kind: &str, surface: u64, data: impl FnOnce() -> Value) -> Option<u64> {
    let session = SESSION.get()?;
    // Evaluate observations before the recorder lock; never acquire product locks beneath it.
    let data = data();
    let mut state = session.lock().unwrap_or_else(|e| e.into_inner());
    if state.failed { return None; }
    let seq = state.seq; state.seq += 1;
    let row = json!({"seq":seq,"mono_ns":u64::try_from(state.start.elapsed().as_nanos()).unwrap(),
        "pid":std::process::id(),"surface":surface,"kind":kind,"data":data});
    let final_path = state.root.join(format!("{seq:012}.json"));
    let temp = state.root.join(format!("{seq:012}.tmp"));
    let written = (|| -> std::io::Result<()> {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&temp)?;
        file.write_all(serde_json::to_string(&row).unwrap().as_bytes())?; file.sync_all()?;
        std::fs::rename(&temp, &final_path)
    })();
    if written.is_err() { state.failed = true; return None; }
    Some(seq)
}

// Explicitly ignored child mode, selected only by the externally authorized controller.
// No public daemon flag or production RPC is added.
#[test]
#[ignore = "diagnostic child server; requires explicit native authority and owned roots"]
#[cfg(windows)]
fn native_probe_daemon() {
    let root = PathBuf::from(std::env::var("NATIVE_OWNED_ROOT").expect("owned root"));
    let socket = std::env::var("CYS_SOCKET").expect("private pipe");
    let root_name = root.file_name().expect("owned root name").to_str().expect("UTF-8 root name");
    let nonce = root_name.strip_prefix("cys-native-").expect("owned root prefix");
    assert_eq!(nonce.len(), 32);
    assert!(nonce.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
    assert_eq!(socket, format!(r"\\.\pipe\cys-native-{nonce}"));
    let canonical = root.canonicalize().expect("owned root exists");
    for name in ["HOME", "LOCALAPPDATA", "CYS_PACK_DIR"] {
        let path = PathBuf::from(std::env::var(name).expect("isolated path"));
        assert!(path.canonicalize().expect("isolated path exists").starts_with(&canonical));
    }
    assert_eq!(std::env::var("NATIVE_EXECUTION_AUTHORIZED").as_deref(), Ok("1"));
    initialize(&root.join("trace")).expect("private trace setup");
    crate::async_main();
}

pub(crate) struct Writer<W> { inner: W, surface: u64, operation: Option<u64> }
impl<W> Writer<W> {
    pub(crate) fn new(inner: W, surface: u64) -> Self { Self { inner, surface, operation: None } }
    pub(crate) fn operation(&mut self, operation: Option<u64>, surface: u64) { self.operation = operation; self.surface = surface; }
}
impl<W: std::io::Write> std::io::Write for Writer<W> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let result = self.inner.write(bytes);
        emit("native_write", self.surface, || json!({"operation":self.operation,"requested":bytes,
            "accepted":result.as_ref().ok(),"error_kind":result.as_ref().err().map(|e|format!("{:?}",e.kind())),
            "raw_error":result.as_ref().err().and_then(|e|e.raw_os_error())}));
        result
    }
    fn write_all(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        // Preserve a native writer's override. On error its partial progress is UNKNOWN,
        // not zero; the independent receiver bytes are needed to bound that outcome.
        let result = self.inner.write_all(bytes);
        emit("native_write_all", self.surface, || json!({"operation":self.operation,"requested":bytes,
            "accepted":result.as_ref().ok().map(|_|bytes.len()),"error_kind":result.as_ref().err().map(|e|format!("{:?}",e.kind())),
            "raw_error":result.as_ref().err().and_then(|e|e.raw_os_error())}));
        result
    }
    fn flush(&mut self) -> std::io::Result<()> {
        let result = self.inner.flush();
        emit("native_flush", self.surface, || json!({"operation":self.operation,"complete":result.is_ok(),
            "raw_error":result.as_ref().err().and_then(|e|e.raw_os_error())}));
        result
    }
}

#[test]
fn wrapper_preserves_write_all_override() {
    use std::io::Write;
    struct Override(bool);
    impl Write for Override {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> { panic!("native write_all override was bypassed") }
        fn write_all(&mut self, _: &[u8]) -> std::io::Result<()> { self.0 = true; Ok(()) }
        fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
    }
    let mut sink = Override(false);
    Writer::new(&mut sink, 1).write_all(b"opaque native operation").unwrap();
    assert!(sink.0);
}
