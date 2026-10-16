//! Q9-b native Windows positive controls using actual vendor types.
//! These do not inject native handle-cloning failures.
use super::WinChild;
use crate::Child as PortableChild;
use filedescriptor::OwnedHandle;
use std::io;
use std::os::windows::io::FromRawHandle;
use std::os::windows::process::CommandExt;
use std::process::{Child as StdChild, Command, ExitStatus as StdExitStatus, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use winapi::um::minwinbase::STILL_ACTIVE;
use winapi::um::processthreadsapi::{GetCurrentProcessId, GetExitCodeProcess, OpenProcess};
use winapi::um::winbase::CREATE_NO_WINDOW;
use winapi::um::winnt::{PROCESS_QUERY_LIMITED_INFORMATION, SYNCHRONIZE};

#[test]
fn real_current_process_handle_reports_running() {
    // OpenProcess returns a fresh, real owning handle, unlike GetCurrentProcess's
    // pseudo-handle. Never pass a pseudo/invalid handle to from_raw_handle.
    let raw = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE, 0, GetCurrentProcessId()) };
    assert!(!raw.is_null(), "OpenProcess(test process): {}", io::Error::last_os_error());
    // SAFETY: OpenProcess succeeded and transferred ownership of this real handle.
    let owned = unsafe { OwnedHandle::from_raw_handle(raw as _) };
    let mut direct_status = 0;
    let direct = unsafe { GetExitCodeProcess(raw, &mut direct_status) };
    assert_ne!(direct, 0, "positive control requires a successful native exit-code read");
    assert_eq!(direct_status, STILL_ACTIVE);
    let mut child = WinChild { proc: Mutex::new(owned) };
    assert!(PortableChild::try_wait(&mut child).expect("actual WinChild try_wait").is_none());
    assert!(!child.proc.is_poisoned());
    assert!(child.proc.try_lock().is_ok());
    // Only the new owned handle is dropped. Never kill or wait for this process.
}

struct FiniteChild { child: StdChild, reaped: bool }
impl FiniteChild {
    fn poll_reap(&mut self, duration: Duration) -> io::Result<Option<StdExitStatus>> {
        let until = Instant::now() + duration;
        loop {
            if let Some(status) = self.child.try_wait()? { self.reaped = true; return Ok(Some(status)); }
            if Instant::now() >= until { return Ok(None); }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    fn cleanup(&mut self) -> io::Result<StdExitStatus> {
        if let Some(status) = self.poll_reap(Duration::from_secs(2))? { return Ok(status); }
        self.child.kill()?; // this exact original owned child only; no PID/name/group kill
        self.poll_reap(Duration::from_secs(2))?.ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut,
            "owned finite child did not finish after kill; retain failed CI cell"))
    }
}
impl Drop for FiniteChild {
    fn drop(&mut self) {
        if !self.reaped {
            let _ = self.child.kill();
            match self.poll_reap(Duration::from_secs(2)) {
                Ok(Some(_)) => {},
                other => eprintln!("Q9-b owned-child cleanup incomplete during failure unwind: {:?}", other),
            }
        }
    }
}

#[test]
fn owned_finite_terminal_child_reports_exit_code() {
    let cmd = std::path::PathBuf::from(std::env::var_os("SystemRoot").expect("SystemRoot"))
        .join("System32").join("cmd.exe");
    // /D disables cmd AutoRun; this command creates no grandchildren and ends
    // independently of input. It is not a user shell/profile or a live cys child.
    let child = Command::new(cmd).args(["/D", "/C", "exit /B 37"])
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW).spawn().expect("spawn finite owned cmd");
    let mut owner = FiniteChild { child, reaped: false };
    // Duplicate the actual std child's valid process handle. Ownership of the
    // original remains with owner throughout observation and bounded cleanup.
    let duplicate = OwnedHandle::dup(&owner.child).expect("duplicate valid owned std child handle");
    let mut observed = WinChild { proc: Mutex::new(duplicate) };
    let until = Instant::now() + Duration::from_secs(5);
    let outcome = loop {
        match PortableChild::try_wait(&mut observed) {
            Ok(Some(status)) => break Ok(status.exit_code()),
            Ok(None) if Instant::now() < until => std::thread::sleep(Duration::from_millis(10)),
            Ok(None) => break Err(io::Error::new(io::ErrorKind::TimedOut, "actual WinChild remained active past finite-child bound")),
            Err(e) => break Err(e),
        }
    };
    let cleanup = owner.cleanup(); // always attempted before any outcome assertion
    let final_status = cleanup.expect("original owned child cleanup must complete");
    assert_eq!(final_status.code(), Some(37));
    assert_eq!(outcome.expect("actual WinChild terminal observation"), 37);
    assert!(!observed.proc.is_poisoned());
    assert!(observed.proc.try_lock().is_ok());
    // WinChild::wait/kill/poll are not invoked or edited by these tests.
}
