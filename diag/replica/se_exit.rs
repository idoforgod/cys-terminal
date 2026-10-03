// diag/replica/se_exit.rs -- replica of the updater plugin's Windows install call (QF).
//
// usage: se_exit <main|thread> <file> <params> <log> [nolog]
//   main   : call ShellExecuteW on the main thread, then std::process::exit(0)
//   thread : call ShellExecuteW inside std::thread::spawn (no COM initialisation, like a tokio worker thread)
//            and call exit(0) from that spawned thread; the main thread never joins it.
//   (default) log   : open <log> for append BEFORE the call, and write one line "ret=<HINSTANCE> lasterr=<n>"
//                     right after the call, then exit(0)
//   nolog           : exact plugin pattern: nothing at all between the return of ShellExecuteW and exit(0)
//
// ShellExecuteW(NULL, "open", file, params, NULL, SW_SHOW) -- the arguments the plugin uses.
// Only extern "system" declarations, no external crates.
use std::ffi::c_void;
use std::fs::OpenOptions;
use std::io::Write;
use std::os::windows::ffi::OsStrExt;
use std::time::Duration;

#[link(name = "shell32")]
extern "system" {
    fn ShellExecuteW(
        hwnd: *mut c_void,
        lp_operation: *const u16,
        lp_file: *const u16,
        lp_parameters: *const u16,
        lp_directory: *const u16,
        n_show_cmd: i32,
    ) -> isize;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
}

fn wide(s: &str) -> Vec<u16> {
    std::ffi::OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn call_and_exit(file: String, params: String, log: String, nolog: bool) {
    let op = wide("open");
    let f = wide(&file);
    let p = wide(&params);
    let logf = if nolog {
        None
    } else {
        OpenOptions::new().create(true).append(true).open(&log).ok()
    };
    unsafe {
        SetLastError(0);
        let ret = ShellExecuteW(
            std::ptr::null_mut(),
            op.as_ptr(),
            f.as_ptr(),
            p.as_ptr(),
            std::ptr::null(),
            5,
        );
        if let Some(mut lf) = logf {
            let err = GetLastError();
            let _ = lf.write_all(format!("ret={} lasterr={}\r\n", ret, err).as_bytes());
        }
    }
    std::process::exit(0);
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() < 5 {
        eprintln!("usage: se_exit <main|thread> <file> <params> <log> [nolog]");
        std::process::exit(2);
    }
    let mode = a[1].clone();
    let file = a[2].clone();
    let params = a[3].clone();
    let log = a[4].clone();
    let nolog = a.len() > 5 && a[5] == "nolog";
    if mode == "thread" {
        let _ = std::thread::spawn(move || call_and_exit(file, params, log, nolog));
        // the spawned thread ends the process; reaching the end of this sleep means exit(0) never ran
        std::thread::sleep(Duration::from_secs(60));
        std::process::exit(3);
    } else {
        call_and_exit(file, params, log, nolog);
    }
}
