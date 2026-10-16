//! Q9-b source-body/leaf-model assertion suite.
//! The extraction driver inserts the actual method bytes at the sole marker.
//! Leaf outcomes are modeled; std Mutex, poisoning and catch_unwind are real.
//! This is not filedescriptor type validation or native Windows fault injection.
#![allow(non_snake_case, non_camel_case_types)]
use std::io::{Error as IoError, Result as IoResult};
use std::sync::{Arc, Mutex, atomic::{AtomicUsize, Ordering}};

type DWORD = u32;
const STILL_ACTIVE: DWORD = 259;

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExitStatus { code: u32 }
impl ExitStatus {
    fn with_exit_code(code: u32) -> Self { Self { code } }
}

#[derive(Debug)]
struct CloneFailure;
impl std::fmt::Display for CloneFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("scripted distinct non-IO clone error") }
}
impl std::error::Error for CloneFailure {}
// Deliberately no From<CloneFailure> for std::io::Error.

#[derive(Debug)]
struct Leaves {
    clone_fails: bool,
    exit_api_succeeds: bool,
    status: DWORD,
    clones: AtomicUsize,
    exit_calls: AtomicUsize,
}
#[derive(Debug)]
struct OwnedHandle { leaves: Arc<Leaves> }
impl OwnedHandle {
    fn try_clone(&self) -> Result<Self, CloneFailure> {
        self.leaves.clones.fetch_add(1, Ordering::SeqCst);
        if self.leaves.clone_fails { Err(CloneFailure) } else { Ok(Self { leaves: self.leaves.clone() }) }
    }
    fn as_raw_handle(&self) -> *const Leaves { Arc::as_ptr(&self.leaves) }
}

unsafe fn GetExitCodeProcess(handle: *const Leaves, status: *mut DWORD) -> i32 {
    // Model pointer only: Arc storage remains owned by WinChild and the duplicate.
    // No OS handle is fabricated, owned, closed, or passed to a native API here.
    let leaves = unsafe { &*handle };
    leaves.exit_calls.fetch_add(1, Ordering::SeqCst);
    unsafe { *status = leaves.status; }
    i32::from(leaves.exit_api_succeeds)
}

#[derive(Debug)]
struct WinChild { proc: Mutex<OwnedHandle> }
impl WinChild {
/*__Q9B_EXACT_METHOD_BYTES__*/
}

#[cfg(test)]
mod q9b_model_tests {
    use super::*;
    use std::panic::{catch_unwind, AssertUnwindSafe};

    fn fixture(clone_fails: bool, exit_api_succeeds: bool, status: DWORD) -> (WinChild, Arc<Leaves>) {
        let leaves = Arc::new(Leaves { clone_fails, exit_api_succeeds, status,
            clones: AtomicUsize::new(0), exit_calls: AtomicUsize::new(0) });
        (WinChild { proc: Mutex::new(OwnedHandle { leaves: leaves.clone() }) }, leaves)
    }
    fn counts(leaves: &Leaves, clones: usize, exits: usize) {
        assert_eq!(leaves.clones.load(Ordering::SeqCst), clones, "clone leaf call count");
        assert_eq!(leaves.exit_calls.load(Ordering::SeqCst), exits, "exit-code leaf call count");
    }

    #[test]
    fn clone_error_returns_io_error_without_unwind_or_exit_api_call() {
        let (mut child, leaves) = fixture(true, true, 37);
        let caught = catch_unwind(AssertUnwindSafe(|| child.is_complete()));
        assert!(caught.is_ok(), "clone Err must propagate; unwinding while proc is locked is the regression");
        let error = caught.unwrap().expect_err("clone failure must be an IO Err, never an exit result");
        assert_eq!(error.kind(), std::io::ErrorKind::Other);
        assert!(error.get_ref().and_then(|e| e.downcast_ref::<CloneFailure>()).is_some(),
            "preserve the distinct non-IO error inside the IO wrapper");
        counts(&leaves, 1, 0);
        assert!(!child.proc.is_poisoned());
        assert!(child.proc.try_lock().is_ok(), "proc mutex usable after returned Err");
    }

    #[test]
    fn clone_error_does_not_poison_proc_mutex() {
        let (mut child, leaves) = fixture(true, true, STILL_ACTIVE);
        let _ = catch_unwind(AssertUnwindSafe(|| child.is_complete()));
        // Independent assertion: a baseline unwind must not hide this property
        // behind the preceding test's first failed normal-return assertion.
        assert!(!child.proc.is_poisoned(), "a clone error must not poison the real proc mutex");
        assert!(child.proc.try_lock().is_ok());
        counts(&leaves, 1, 0);
    }

    #[test]
    fn active_process_keeps_ok_none() {
        let (mut child, leaves) = fixture(false, true, STILL_ACTIVE);
        assert_eq!(child.is_complete().unwrap(), None);
        counts(&leaves, 1, 1);
        assert!(!child.proc.is_poisoned());
    }

    #[test]
    fn terminal_process_keeps_exit_code() {
        let (mut child, leaves) = fixture(false, true, 37);
        assert_eq!(child.is_complete().unwrap(), Some(ExitStatus::with_exit_code(37)));
        counts(&leaves, 1, 1);
        assert!(!child.proc.is_poisoned());
    }

    #[test]
    fn exit_code_api_failure_keeps_ok_none() {
        let (mut child, leaves) = fixture(false, false, 37);
        assert_eq!(child.is_complete().unwrap(), None, "Q9: failed API read remains Ok(None)");
        counts(&leaves, 1, 1);
        assert!(!child.proc.is_poisoned());
    }

    #[test]
    fn prepoisoned_mutex_still_panics_before_native_leaves() {
        let (mut child, leaves) = fixture(false, true, STILL_ACTIVE);
        let poison = catch_unwind(AssertUnwindSafe(|| {
            let _held = child.proc.lock().unwrap();
            panic!("intentional pre-existing poison fixture");
        }));
        assert!(poison.is_err());
        assert!(child.proc.is_poisoned());
        let observed = catch_unwind(AssertUnwindSafe(|| child.is_complete()));
        assert!(observed.is_err(), "Q9-b does not authorize recovery from pre-existing lock poison");
        counts(&leaves, 0, 0);
        assert!(child.proc.is_poisoned());
    }
}
