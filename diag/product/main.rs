// diag/product/main.rs -- driver for the PRODUCT module src/update_launch.rs (copied byte for byte next to this file).
// std only, ASCII only. The Windows branch makes the real calls; the non-Windows branch uses a local test double of the launch
// call (only so that the file protocol and the JSON output can be tested on a Mac; it is never built for the runner).
//
//   sac-launch.exe <work_dir> <cmd_exe_path> <installer_path>
//
// The process is started BEFORE Smart App Control (SAC) is turned on (a new unsigned exe could not start afterwards) and
// waits for a signal file; the controlling script (diag/product-launch.ps1) turns REAL SAC on in between.
//
// Protocol (all files live in <work_dir>):
//   OFF stage (immediately, SAC still off)
//     1. write_installer(work_dir, "cys", "0.0.0-diag", <bytes of cmd_exe_path>)      -> path shape is recorded
//     2. launch_installer(that file, "/c exit 0")                                      -> expected Ok (signed cmd.exe copy)
//     3. product-launch.jsonl gets the results, then the file "ready" is created
//   wait for the file "go" (max 10 minutes, polled every 0.5 s; the file "abort" ends the wait early)
//   ON stage (after "go"; SAC was turned on in between)
//     4. write_installer(work_dir, "cys", "0.14.42", <bytes of installer_path>)
//     5. launch_installer(that file, nsis_update_params(&[]))   -> expected Err: os_code 4551, shell_ret 5, block() = AppControl
//     6. remove_installer(that file) -> file and folder must be gone (only after a FAILED launch: module contract)
//     7. control: write_installer(... cmd copy ...) + launch_installer(copy, "/c exit 0") -> expected Ok (signed files still pass)
//     8. product-launch.jsonl gets the results, the file "done" is created, exit code 0
// Every call's real return value is written as it is. Whether it matches the expectation is written next to it
// ("expect" / "matches"); the verdict itself is the controlling script's job.
// A file that is not an exe (no MZ header) is never launched: Windows would show a modal error window for it.

#[allow(dead_code)]
mod update_launch;

use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use update_launch::LaunchError;

const APP: &str = "cys";
const OFF_VERSION: &str = "0.0.0-diag";
const ON_VERSION: &str = "0.14.42";
const OFF_PARAMS: &str = "/c exit 0";
/// a launch call that takes longer than this is flagged "slow" (Windows may be holding the call with a modal window)
const SLOW_MS: u128 = 5000;
const GO_WAIT_SECS: u64 = 600;
const POLL_MS: u64 = 500;
/// a launch call that has not returned after this long is given up on (it stays blocked on its own worker thread and the driver goes on):
/// the driver must still write "done" when Windows holds ShellExecuteW with a modal window
const LAUNCH_LIMIT: Duration = Duration::from_secs(40);
/// after remove_installer the driver only OBSERVES for this long whether file and folder disappear (a handle that another process,
/// e.g. a virus scanner, still holds can leave the delete pending for a moment); the module is not called again
const SETTLE_MAX: Duration = Duration::from_secs(3);

/// the call under test (a plain fn pointer so that it can run on its own worker thread, see timed_launch) and how long it may take
struct Launcher {
    call: fn(&Path, &str) -> Result<(), LaunchError>,
    limit: Duration,
}

// ---------------------------------------------------------------------------
// the launch call: real on Windows, a test double elsewhere
// ---------------------------------------------------------------------------

#[cfg(windows)]
fn launch(file: &Path, params: &str) -> Result<(), LaunchError> {
    update_launch::launch_installer(file, params)
}

/// Test double (non-Windows only): a file whose name contains "0.14.42" is "blocked" the way SAC blocks the unsigned installer
/// (ShellExecuteW 5 / GetLastError 4551), every other file "starts" (ShellExecuteW 42). Built with the module's own shell_result.
#[cfg(not(windows))]
fn launch(file: &Path, _params: &str) -> Result<(), LaunchError> {
    let name = file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    if name.contains("0.14.42") {
        update_launch::shell_result(5, 4551)
    } else {
        update_launch::shell_result(42, 0)
    }
}

#[cfg(windows)]
fn attr_temporary(p: &Path) -> J {
    use std::os::windows::fs::MetadataExt;
    match fs::metadata(p) {
        Ok(m) => J::B(m.file_attributes() & 0x100 != 0),
        Err(_) => J::Null,
    }
}

#[cfg(not(windows))]
fn attr_temporary(_p: &Path) -> J {
    J::Null
}

// ---------------------------------------------------------------------------
// tiny JSON writer (std only): one JSON object per line, everything ASCII
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
enum J {
    S(String),
    N(i64),
    B(bool),
    Null,
}

fn json_str(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 || (c as u32) > 0x7e => {
                let mut buf = [0u16; 2];
                for u in c.encode_utf16(&mut buf).iter() {
                    o.push_str(&format!("\\u{:04x}", u));
                }
            }
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

impl J {
    fn render(&self) -> String {
        match self {
            J::S(s) => json_str(s),
            J::N(n) => n.to_string(),
            J::B(b) => b.to_string(),
            J::Null => "null".to_string(),
        }
    }
}

fn json_obj(fields: &[(String, J)]) -> String {
    let mut o = String::from("{");
    for (i, (k, v)) in fields.iter().enumerate() {
        if i > 0 {
            o.push(',');
        }
        o.push_str(&json_str(k));
        o.push(':');
        o.push_str(&v.render());
    }
    o.push('}');
    o
}

fn f(k: &str, v: J) -> (String, J) {
    (k.to_string(), v)
}

fn s(v: &str) -> J {
    J::S(v.to_string())
}

fn jpath(p: &Path) -> J {
    J::S(p.display().to_string())
}

fn unix_ms() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)
}

/// UTC "YYYY-MM-DDTHH:MM:SS.mmmZ" from Unix milliseconds (civil-from-days algorithm; no time crate).
fn iso_from_unix_ms(ms: u128) -> String {
    let secs = (ms / 1000) as i64;
    let milli = (ms % 1000) as u32;
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    let (h, mi, sec) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y0 = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y0 + 1 } else { y0 };
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z", y, m, d, h, mi, sec, milli)
}

// ---------------------------------------------------------------------------
// result log: <work_dir>/product-launch.jsonl (appended per line, so a kill loses nothing) + the same line on stdout
// ---------------------------------------------------------------------------

struct Log {
    path: PathBuf,
    arch: String,
    pid: u32,
    /// no stdout copy of the lines (the unit tests)
    quiet: bool,
}

impl Log {
    fn new(path: PathBuf) -> Log {
        Log { path, arch: std::env::consts::ARCH.to_string(), pid: std::process::id(), quiet: cfg!(test) }
    }

    fn line(&self, stage: &str, step: &str, fields: Vec<(String, J)>) {
        let now = unix_ms();
        let mut all = vec![
            f("t", J::S(iso_from_unix_ms(now))),
            f("t_unix_ms", J::N(now as i64)),
            f("pid", J::N(self.pid as i64)),
            f("arch", J::S(self.arch.clone())),
            f("stage", s(stage)),
            f("step", s(step)),
        ];
        all.extend(fields);
        let text = json_obj(&all);
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&self.path) {
            let _ = file.write_all(text.as_bytes());
            let _ = file.write_all(b"\n");
            let _ = file.flush();
        }
        // not println!: a closed stdout pipe must not kill the driver in the middle of the measurement
        if !self.quiet {
            let _ = writeln!(std::io::stdout(), "{}", text);
        }
    }
}

/// signal files: written to <name>.tmp first and renamed, so the reader never sees a half-written file
fn write_flag(work: &Path, name: &str, content: &str) {
    let tmp = work.join(format!("{}.tmp", name));
    let target = work.join(name);
    if fs::write(&tmp, content).is_ok() {
        for attempt in 0..3 {
            if fs::rename(&tmp, &target).is_ok() {
                return;
            }
            if attempt < 2 {
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
    // the rename did not work (a scanner holding the .tmp file?): the flag file itself must exist, whatever it takes
    let _ = fs::write(&target, content);
}

// ---------------------------------------------------------------------------
// one launch call, its result, its expectation
// ---------------------------------------------------------------------------

struct CallResult {
    ok: bool,
    shell_ret: Option<isize>,
    os_code: Option<u32>,
    display: String,
    block: String,
    ms: u128,
    /// the call did not return within the limit (it is still blocked on its worker thread: Windows holds it, e.g. with a modal window)
    timed_out: bool,
    /// empty unless something other than a return value happened (timeout, a worker thread that ended without a result)
    note: String,
    /// the call was never made, or gave no result at all (a vanished file, a launch thread that ended without a result): it is neither
    /// a success nor a block, so there is nothing to clean up and nothing to compare
    not_run: bool,
}

impl CallResult {
    fn from_result(r: Result<(), LaunchError>, ms: u128) -> CallResult {
        match r {
            Ok(()) => CallResult { ok: true, shell_ret: None, os_code: None, display: String::new(), block: String::new(), ms, timed_out: false, note: String::new(), not_run: false },
            Err(e) => CallResult {
                ok: false,
                shell_ret: Some(e.shell_ret),
                os_code: Some(e.os_code),
                display: e.to_string(),
                block: format!("{:?}", e.block()),
                ms,
                timed_out: false,
                note: String::new(),
                not_run: false,
            },
        }
    }

    fn no_result(ms: u128, timed_out: bool, not_run: bool, note: String) -> CallResult {
        CallResult { ok: false, shell_ret: None, os_code: None, display: String::new(), block: String::new(), ms, timed_out, note, not_run }
    }
}

/// The product call exactly as the app makes it (update_launch::launch_installer(file, params), nothing in between), run on its own worker
/// thread so that a call Windows holds (a modal window) cannot keep the driver from writing "done". The thread keeps the module's
/// SetLastError/GetLastError pair together. A call that has not returned after the limit is recorded as timed_out and left blocked there
/// (the process exit at the end ends it).
fn timed_launch(launcher: &Launcher, file: &Path, params: &str) -> CallResult {
    let t = Instant::now();
    let (tx, rx) = std::sync::mpsc::channel::<Result<(), LaunchError>>();
    let (call, f2, p2) = (launcher.call, file.to_path_buf(), params.to_string());
    let spawned = std::thread::Builder::new().name("launch".to_string()).spawn(move || {
        let r = call(&f2, &p2);
        let _ = tx.send(r);
    });
    match spawned {
        Ok(_) => match rx.recv_timeout(launcher.limit) {
            Ok(r) => CallResult::from_result(r, t.elapsed().as_millis()),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                CallResult::no_result(t.elapsed().as_millis(), true, false, format!("no result within {} ms: the call is still blocked", launcher.limit.as_millis()))
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                CallResult::no_result(t.elapsed().as_millis(), false, true, "the launch thread ended without a result (panic?)".to_string())
            }
        },
        // no thread could be started: make the call here, without the watchdog
        Err(_) => CallResult::from_result((launcher.call)(file, params), t.elapsed().as_millis()),
    }
}

#[derive(Clone, Copy)]
enum Expect {
    Success,
    Blocked,
}

impl Expect {
    fn text(&self) -> &'static str {
        match self {
            Expect::Success => "Ok",
            Expect::Blocked => "Err os_code=4551 shell_ret=5 block=AppControl display=installer_launch_failed:4551:5",
        }
    }

    fn matches(&self, r: &CallResult) -> bool {
        match self {
            Expect::Success => r.ok,
            Expect::Blocked => {
                !r.ok
                    && r.os_code == Some(4551)
                    && r.shell_ret == Some(5)
                    && r.block == "AppControl"
                    && r.display == "installer_launch_failed:4551:5"
            }
        }
    }
}

fn call_fields(file: &Path, params: &str, r: &CallResult, expect: Expect) -> Vec<(String, J)> {
    vec![
        f("file", jpath(file)),
        f("params", s(params)),
        f("ok", J::B(r.ok)),
        f("shell_ret", match r.shell_ret {
            Some(v) => J::N(v as i64),
            None => J::Null,
        }),
        f("os_code", match r.os_code {
            Some(v) => J::N(v as i64),
            None => J::Null,
        }),
        f("display", J::S(r.display.clone())),
        f("block", J::S(r.block.clone())),
        f("ms", J::N(r.ms as i64)),
        f("slow", J::B(r.ms > SLOW_MS)),
        f("timed_out", J::B(r.timed_out)),
        f("not_run", J::B(r.not_run)),
        f("note", J::S(r.note.clone())),
        f("expect", s(expect.text())),
        f("matches", J::B(expect.matches(r))),
    ]
}

fn launch_step(log: &Log, stage: &str, step: &str, file: &Path, params: &str, expect: Expect, launcher: &Launcher) -> CallResult {
    // a file that is not there any more (a scanner may have removed it) is not launched: Windows would show a modal error window for it
    if matches!(file.try_exists(), Ok(false)) {
        let r = CallResult::no_result(0, false, true, "the file is missing right before the call (removed by a scanner?): not launched".to_string());
        let mut fields = call_fields(file, params, &r, expect);
        fields.push(f("skipped_missing_file", J::B(true)));
        log.line(stage, step, fields);
        return r;
    }
    // a begin line first: when Windows holds the call (modal window) the log still says which call it was
    log.line(stage, &format!("{}_begin", step), vec![f("file", jpath(file)), f("params", s(params))]);
    let r = timed_launch(launcher, file, params);
    log.line(stage, step, call_fields(file, params, &r, expect));
    r
}

// ---------------------------------------------------------------------------
// write_installer with the path shape recorded (it must be the plugin's shape: <root>\cys-<ver>-updater-<6 alnum>\cys-<ver>-installer.exe)
// ---------------------------------------------------------------------------

fn shape_fields(file: &Path, work: &Path, version: &str, bytes_len: usize, ms: u128) -> Vec<(String, J)> {
    let dir = file.parent();
    let dir_name = dir.and_then(|d| d.file_name()).map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let prefix = update_launch::installer_dir_prefix(APP, version);
    let prefix_ok = dir_name.starts_with(&prefix);
    let suffix = if prefix_ok { dir_name[prefix.len()..].to_string() } else { String::new() };
    let file_name = file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let file_name_expected = update_launch::installer_file_name(APP, version);
    let size_on_disk = match fs::metadata(file) {
        Ok(m) => J::N(m.len() as i64),
        Err(_) => J::Null,
    };
    vec![
        f("ok", J::B(true)),
        f("path", jpath(file)),
        f("bytes_expected", J::N(bytes_len as i64)),
        f("size_on_disk", size_on_disk),
        f("dir_name", s(&dir_name)),
        f("dir_prefix_expected", s(&prefix)),
        f("dir_prefix_ok", J::B(prefix_ok)),
        f("dir_suffix", s(&suffix)),
        f("dir_suffix_len", J::N(suffix.len() as i64)),
        f("dir_suffix_alnum6", J::B(suffix.len() == 6 && suffix.bytes().all(|b| b.is_ascii_alphanumeric()))),
        f("file_name", s(&file_name)),
        f("file_name_expected", s(&file_name_expected)),
        f("file_name_ok", J::B(file_name == file_name_expected)),
        f("parent_of_dir_is_work_dir", J::B(dir.and_then(|d| d.parent()) == Some(work))),
        f("attr_temporary", attr_temporary(file)),
        f("ms", J::N(ms as i64)),
        f("slow", J::B(ms > SLOW_MS)),
    ]
}

fn write_step(log: &Log, stage: &str, step: &str, work: &Path, version: &str, bytes: &[u8]) -> Option<PathBuf> {
    let t = Instant::now();
    match update_launch::write_installer(work, APP, version, bytes) {
        Ok(file) => {
            log.line(stage, step, shape_fields(&file, work, version, bytes.len(), t.elapsed().as_millis()));
            Some(file)
        }
        Err(e) => {
            log.line(
                stage,
                step,
                vec![f("ok", J::B(false)), f("error", J::S(e.to_string())), f("kind", J::S(format!("{:?}", e.kind())))],
            );
            None
        }
    }
}

/// "yes" / "no" / "error <kind>" (the last when the existence check itself failed)
fn exists_text(p: &Path) -> String {
    match p.try_exists() {
        Ok(true) => "yes".to_string(),
        Ok(false) => "no".to_string(),
        Err(e) => format!("error {:?}", e.kind()),
    }
}

/// read a file; None when it cannot be read or does not look like an exe (never launch that: Windows would show a modal error window)
fn read_exe_bytes(log: &Log, stage: &str, step: &str, path: &Path) -> Option<Vec<u8>> {
    let t = Instant::now();
    match fs::read(path) {
        Ok(b) => {
            let mz = update_launch::looks_like_exe(&b);
            log.line(
                stage,
                step,
                vec![
                    f("path", jpath(path)),
                    f("bytes", J::N(b.len() as i64)),
                    f("looks_like_exe", J::B(mz)),
                    f("ms", J::N(t.elapsed().as_millis() as i64)),
                    f("slow", J::B(t.elapsed().as_millis() > SLOW_MS)),
                ],
            );
            if mz {
                Some(b)
            } else {
                None
            }
        }
        Err(e) => {
            log.line(stage, step, vec![f("path", jpath(path)), f("ok", J::B(false)), f("error", J::S(e.to_string()))]);
            None
        }
    }
}

// ---------------------------------------------------------------------------
// the two stages
// ---------------------------------------------------------------------------

fn stage_off(log: &Log, work: &Path, cmd_path: &Path, launcher: &Launcher) -> (bool, Option<Vec<u8>>) {
    let cmd_bytes = read_exe_bytes(log, "off", "read_cmd_exe", cmd_path);
    let mut off_ok = false;
    match cmd_bytes.as_ref() {
        Some(bytes) => {
            if let Some(file) = write_step(log, "off", "write_cmd_copy", work, OFF_VERSION, bytes) {
                let r = launch_step(log, "off", "launch_cmd_copy", &file, OFF_PARAMS, Expect::Success, launcher);
                off_ok = r.ok;
            }
        }
        None => log.line("off", "launch_cmd_copy", vec![f("skipped", s("cmd.exe bytes unavailable or not an exe"))]),
    }
    (off_ok, cmd_bytes)
}

struct OnSummary {
    block_ok: bool,
    cleanup_ok: bool,
    cleanup_settled_ok: bool,
    control_ok: bool,
    /// only set when the control failed: did the ORIGINAL cmd.exe launch? (diagnostic, not an expectation)
    control_original_ok: Option<bool>,
    /// at least one launch call did not return within the limit
    timed_out: bool,
}

fn stage_on(log: &Log, work: &Path, installer_path: &Path, cmd_path: &Path, cmd_bytes: Option<&[u8]>, launcher: &Launcher) -> OnSummary {
    let mut sum = OnSummary { block_ok: false, cleanup_ok: false, cleanup_settled_ok: false, control_ok: false, control_original_ok: None, timed_out: false };
    // 4 + 5 + 6: the real 0.14.42 installer, exactly the way the app launches it
    match read_exe_bytes(log, "on", "read_real_installer", installer_path) {
        Some(bytes) => {
            let written = write_step(log, "on", "write_real_installer", work, ON_VERSION, &bytes);
            drop(bytes);
            if let Some(file) = written {
                let params = update_launch::nsis_update_params(&[]);
                let r = launch_step(log, "on", "launch_real_installer", &file, &params, Expect::Blocked, launcher);
                sum.block_ok = Expect::Blocked.matches(&r);
                sum.timed_out |= r.timed_out;
                if r.timed_out {
                    log.line("on", "remove_installer", vec![f("skipped", s("the launch call did not return in time: the installer file is left where it is"))]);
                } else if r.not_run {
                    log.line("on", "remove_installer", vec![f("skipped", s("the launch call did not run or gave no result: nothing to clean up"))]);
                } else if !r.ok {
                    let t = Instant::now();
                    update_launch::remove_installer(&file);
                    let call_ms = t.elapsed().as_millis();
                    let dir = file.parent().map(|d| d.to_path_buf());
                    // "gone" only when the check says "not found": any other error counts as still there (Path::exists would read it as gone)
                    let gone = |file: &Path, dir: &Option<PathBuf>| -> (bool, bool) {
                        (matches!(file.try_exists(), Ok(false)), dir.as_ref().map(|d| matches!(d.try_exists(), Ok(false))).unwrap_or(false))
                    };
                    let (file_gone, dir_gone) = gone(&file, &dir);
                    sum.cleanup_ok = file_gone && dir_gone;
                    // observe only: does everything disappear within SETTLE_MAX? (the module is not called again)
                    let t2 = Instant::now();
                    let mut settled = (file_gone, dir_gone);
                    while !(settled.0 && settled.1) && t2.elapsed() < SETTLE_MAX {
                        std::thread::sleep(Duration::from_millis(100));
                        settled = gone(&file, &dir);
                    }
                    sum.cleanup_settled_ok = settled.0 && settled.1;
                    log.line(
                        "on",
                        "remove_installer",
                        vec![
                            f("file", jpath(&file)),
                            f("file_exists_after", J::B(!file_gone)),
                            f("dir", match dir.as_ref() {
                                Some(d) => jpath(d),
                                None => J::Null,
                            }),
                            f("dir_exists_after", J::B(!dir_gone)),
                            f("cleanup_ok", J::B(sum.cleanup_ok)),
                            f("file_exists_settled", J::B(!settled.0)),
                            f("dir_exists_settled", J::B(!settled.1)),
                            f("cleanup_ok_settled", J::B(sum.cleanup_settled_ok)),
                            f("settle_ms", J::N(if sum.cleanup_ok { 0 } else { t2.elapsed().as_millis() as i64 })),
                            f("file_check_settled", J::S(exists_text(&file))),
                            f("dir_check_settled", J::S(dir.as_ref().map(|d| exists_text(d)).unwrap_or_else(|| "no parent".to_string()))),
                            f("ms", J::N(call_ms as i64)),
                            f("slow", J::B(call_ms > SLOW_MS)),
                        ],
                    );
                } else {
                    log.line(
                        "on",
                        "remove_installer",
                        vec![f("skipped", s("the launch returned Ok: the installer runs from that file (module contract: do not remove)"))],
                    );
                }
            }
        }
        None => log.line("on", "launch_real_installer", vec![f("skipped", s("installer bytes unavailable or not an exe"))]),
    }
    // 7: control, a signed file must still pass while SAC is on
    match cmd_bytes {
        Some(bytes) => {
            if let Some(file) = write_step(log, "on", "write_cmd_copy", work, OFF_VERSION, bytes) {
                let r = launch_step(log, "on", "launch_cmd_copy", &file, OFF_PARAMS, Expect::Success, launcher);
                sum.control_ok = r.ok;
                sum.timed_out |= r.timed_out;
                if !r.ok && !r.timed_out && !r.not_run {
                    // diagnostic only (a failing control is the surprise to explain): does the ORIGINAL file still launch? It tells a blocked
                    // copy (location / name) from a blocked cmd.exe. Not part of the expectations.
                    let o = launch_step(log, "on", "launch_cmd_original", cmd_path, OFF_PARAMS, Expect::Success, launcher);
                    sum.control_original_ok = Some(o.ok);
                    sum.timed_out |= o.timed_out;
                }
            }
        }
        None => log.line("on", "launch_cmd_copy", vec![f("skipped", s("cmd.exe bytes unavailable or not an exe"))]),
    }
    sum
}

enum Go {
    Go,
    Abort,
    Timeout,
}

fn wait_for_go(work: &Path, poll: Duration, max: Duration) -> Go {
    let start = Instant::now();
    loop {
        if work.join("go").exists() {
            return Go::Go;
        }
        if work.join("abort").exists() {
            return Go::Abort;
        }
        if start.elapsed() >= max {
            return Go::Timeout;
        }
        std::thread::sleep(poll);
    }
}

struct Options {
    work: PathBuf,
    cmd_path: PathBuf,
    installer_path: PathBuf,
    go_wait: Duration,
    poll: Duration,
}

fn run(opts: &Options, launcher: &Launcher) -> i32 {
    let _ = fs::create_dir_all(&opts.work);
    let log = Log::new(opts.work.join("product-launch.jsonl"));
    let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default();
    log.line(
        "start",
        "start",
        vec![
            f("work_dir", jpath(&opts.work)),
            f("cmd_exe", jpath(&opts.cmd_path)),
            f("installer", jpath(&opts.installer_path)),
            f("exe", s(&exe)),
            f("nsis_update_params", s(&update_launch::nsis_update_params(&[]))),
            f("update_launch_prefix", s(update_launch::LAUNCH_FAILED_PREFIX)),
        ],
    );

    let (off_ok, cmd_bytes) = stage_off(&log, &opts.work, &opts.cmd_path, launcher);
    write_flag(&opts.work, "ready", &json_obj(&[f("off_ok", J::B(off_ok)), f("pid", J::N(std::process::id() as i64))]));
    log.line("off", "ready_written", vec![f("off_ok", J::B(off_ok))]);

    let waited = Instant::now();
    match wait_for_go(&opts.work, opts.poll, opts.go_wait) {
        Go::Go => log.line("wait", "go_seen", vec![f("waited_ms", J::N(waited.elapsed().as_millis() as i64))]),
        Go::Abort => {
            log.line("wait", "abort_seen", vec![f("waited_ms", J::N(waited.elapsed().as_millis() as i64))]);
            write_flag(&opts.work, "done", &json_obj(&[f("aborted", J::B(true))]));
            return 0;
        }
        Go::Timeout => {
            log.line("wait", "go_timeout", vec![f("waited_ms", J::N(waited.elapsed().as_millis() as i64))]);
            write_flag(&opts.work, "done", &json_obj(&[f("go_timeout", J::B(true))]));
            return 0;
        }
    }

    let t = Instant::now();
    let sum = stage_on(&log, &opts.work, &opts.installer_path, &opts.cmd_path, cmd_bytes.as_deref(), launcher);
    let on_ms = t.elapsed().as_millis();
    let fields = vec![
        f("off_ok", J::B(off_ok)),
        f("block_ok", J::B(sum.block_ok)),
        f("cleanup_ok", J::B(sum.cleanup_ok)),
        f("cleanup_ok_settled", J::B(sum.cleanup_settled_ok)),
        f("control_ok", J::B(sum.control_ok)),
        f("control_original_ok", match sum.control_original_ok {
            Some(b) => J::B(b),
            None => J::Null,
        }),
        f("launch_timed_out", J::B(sum.timed_out)),
        f("on_stage_ms", J::N(on_ms as i64)),
        f("driver_alive_through_on_stage", J::B(true)),
    ];
    log.line("on", "stage_done", fields.clone());
    write_flag(&opts.work, "done", &json_obj(&fields));
    0
}

fn main() {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    if args.len() < 3 {
        eprintln!("usage: sac-launch <work_dir> <cmd_exe_path> <installer_path>");
        std::process::exit(2);
    }
    let opts = Options {
        work: PathBuf::from(&args[0]),
        cmd_path: PathBuf::from(&args[1]),
        installer_path: PathBuf::from(&args[2]),
        go_wait: Duration::from_secs(GO_WAIT_SECS),
        poll: Duration::from_millis(POLL_MS),
    };
    let code = run(&opts, &Launcher { call: launch, limit: LAUNCH_LIMIT });
    std::process::exit(code);
}

// ---------------------------------------------------------------------------
// tests of the driver itself (run on a Mac: rustc --edition 2021 --test main.rs); they use the non-Windows test double
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Tmp(PathBuf);
    impl Tmp {
        fn new(tag: &str) -> Tmp {
            static N: AtomicUsize = AtomicUsize::new(0);
            let p = std::env::temp_dir().join(format!("sac-launch-driver-test-{}-{}-{}", std::process::id(), tag, N.fetch_add(1, Ordering::Relaxed)));
            let _ = fs::remove_dir_all(&p);
            fs::create_dir_all(&p).unwrap();
            Tmp(p)
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn fake_launcher(file: &Path, _params: &str) -> Result<(), LaunchError> {
        let name = file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if name.contains("0.14.42") {
            update_launch::shell_result(5, 4551)
        } else {
            update_launch::shell_result(42, 0)
        }
    }

    fn fake() -> Launcher {
        Launcher { call: fake_launcher, limit: Duration::from_secs(10) }
    }

    /// Windows holds the call for the real installer (a modal window): it does not come back for 4 s
    fn held_launcher(file: &Path, _params: &str) -> Result<(), LaunchError> {
        let name = file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if name.contains("0.14.42") {
            std::thread::sleep(Duration::from_secs(4));
            update_launch::shell_result(5, 4551)
        } else {
            update_launch::shell_result(42, 0)
        }
    }

    /// SAC blocks the COPY of cmd.exe (named ...-installer.exe) but not the original file
    fn copy_blocked_launcher(file: &Path, _params: &str) -> Result<(), LaunchError> {
        let name = file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if name.contains("0.14.42") || name.contains("0.0.0-diag") {
            update_launch::shell_result(5, 4551)
        } else {
            update_launch::shell_result(42, 0)
        }
    }

    /// the thread of the launch call panics
    fn panicking_launcher(_file: &Path, _params: &str) -> Result<(), LaunchError> {
        panic!("test: the launch call panics");
    }

    #[test]
    fn json_strings_are_escaped_and_ascii() {
        assert_eq!(json_str("a\"b\\c\n\t\r"), "\"a\\\"b\\\\c\\n\\t\\r\"");
        assert_eq!(json_str("e\u{e9}"), "\"e\\u00e9\"");
        assert_eq!(json_str("\u{1F600}"), "\"\\ud83d\\ude00\"");
        assert_eq!(json_str("\u{1}\u{7f}"), "\"\\u0001\\u007f\"");
        assert!(json_str("caf\u{e9} \u{c11c}\u{c6b8}").is_ascii());
    }

    #[test]
    fn json_object_has_the_given_order_and_types() {
        let o = json_obj(&[f("a", J::N(-3)), f("b", J::B(true)), f("c", J::Null), f("d", s("x\"y"))]);
        assert_eq!(o, "{\"a\":-3,\"b\":true,\"c\":null,\"d\":\"x\\\"y\"}");
    }

    #[test]
    fn iso_time_known_values() {
        assert_eq!(iso_from_unix_ms(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso_from_unix_ms(951_782_400_000), "2000-02-29T00:00:00.000Z");
        assert_eq!(iso_from_unix_ms(1_700_000_000_123), "2023-11-14T22:13:20.123Z");
        assert_eq!(iso_from_unix_ms(1_791_058_063_715), "2026-10-03T20:07:43.715Z");
    }

    fn blocked() -> CallResult {
        CallResult { ok: false, shell_ret: Some(5), os_code: Some(4551), display: "installer_launch_failed:4551:5".to_string(), block: "AppControl".to_string(), ms: 3, timed_out: false, note: String::new(), not_run: false }
    }

    #[test]
    fn expectations_are_exact() {
        assert!(Expect::Blocked.matches(&blocked()));
        let mut r = blocked();
        r.os_code = Some(5);
        r.display = "installer_launch_failed:5:5".to_string();
        r.block = "AccessDenied".to_string();
        assert!(!Expect::Blocked.matches(&r), "5/5 is not the SAC result");
        let mut r2 = blocked();
        r2.shell_ret = Some(2);
        assert!(!Expect::Blocked.matches(&r2));
        let ok = CallResult { ok: true, shell_ret: None, os_code: None, display: String::new(), block: String::new(), ms: 1, timed_out: false, note: String::new(), not_run: false };
        let held = CallResult::no_result(40_000, true, false, "x".to_string());
        assert!(!Expect::Blocked.matches(&held) && !Expect::Success.matches(&held), "a call that never returned matches nothing");
        assert!(Expect::Success.matches(&ok));
        assert!(!Expect::Blocked.matches(&ok), "an unexpected Ok is a mismatch");
        assert!(!Expect::Success.matches(&blocked()));
    }

    #[test]
    fn written_installer_has_the_plugin_path_shape() {
        let t = Tmp::new("shape");
        let file = update_launch::write_installer(&t.0, APP, ON_VERSION, b"MZ-bytes").unwrap();
        let fields = shape_fields(&file, &t.0, ON_VERSION, 8, 1);
        let get = |k: &str| fields.iter().find(|(n, _)| n == k).map(|(_, v)| v.render()).unwrap();
        assert_eq!(get("dir_prefix_ok"), "true");
        assert_eq!(get("dir_suffix_alnum6"), "true");
        assert_eq!(get("file_name_ok"), "true");
        assert_eq!(get("parent_of_dir_is_work_dir"), "true");
        assert_eq!(get("size_on_disk"), "8");
        assert_eq!(get("bytes_expected"), "8");
    }

    #[test]
    fn a_file_without_mz_is_never_read_as_an_exe() {
        let t = Tmp::new("nomz");
        let log = Log::new(t.0.join("x.jsonl"));
        let p = t.0.join("not-an-exe.bin");
        fs::write(&p, b"PK\x03\x04 zip").unwrap();
        assert!(read_exe_bytes(&log, "off", "read_cmd_exe", &p).is_none());
        let missing = t.0.join("missing.exe");
        assert!(read_exe_bytes(&log, "off", "read_cmd_exe", &missing).is_none());
        let good = t.0.join("good.exe");
        fs::write(&good, b"MZ\x90\x00").unwrap();
        assert!(read_exe_bytes(&log, "off", "read_cmd_exe", &good).is_some());
    }

    /// the whole protocol with the test double: OFF stage -> ready -> go -> ON stage -> done
    #[test]
    fn protocol_dry_run() {
        let t = Tmp::new("proto");
        let work = t.0.join("work");
        let cmd = t.0.join("cmd.exe");
        let inst = t.0.join("inst.exe");
        fs::write(&cmd, b"MZ-fake-cmd").unwrap();
        fs::write(&inst, b"MZ-fake-installer-0.14.42").unwrap();
        let opts = Options { work: work.clone(), cmd_path: cmd, installer_path: inst, go_wait: Duration::from_secs(20), poll: Duration::from_millis(20) };
        let w2 = work.clone();
        let th = std::thread::spawn(move || {
            let start = Instant::now();
            while !w2.join("ready").exists() && start.elapsed() < Duration::from_secs(15) {
                std::thread::sleep(Duration::from_millis(10));
            }
            // OFF results are on disk before ready appears
            let off = fs::read_to_string(w2.join("product-launch.jsonl")).unwrap_or_default();
            assert!(off.contains("\"step\":\"launch_cmd_copy\""), "{}", off);
            assert!(!off.contains("launch_real_installer"), "ON stage must wait for go");
            fs::write(w2.join("go"), b"go").unwrap();
        });
        let code = run(&opts, &fake());
        th.join().unwrap();
        assert_eq!(code, 0);
        assert!(work.join("done").exists());
        let log = fs::read_to_string(work.join("product-launch.jsonl")).unwrap();
        for line in log.lines() {
            assert!(line.starts_with('{') && line.ends_with('}') && line.is_ascii(), "{}", line);
        }
        assert!(log.contains("\"stage\":\"off\",\"step\":\"launch_cmd_copy\""));
        assert!(log.contains("\"stage\":\"on\",\"step\":\"launch_real_installer\""));
        let real = log.lines().find(|l| l.contains("\"step\":\"launch_real_installer\"")).unwrap();
        assert!(real.contains("\"os_code\":4551") && real.contains("\"shell_ret\":5") && real.contains("\"block\":\"AppControl\"") && real.contains("\"matches\":true"), "{}", real);
        let rm = log.lines().find(|l| l.contains("\"step\":\"remove_installer\"")).unwrap();
        assert!(rm.contains("\"cleanup_ok\":true") && rm.contains("\"file_exists_after\":false") && rm.contains("\"dir_exists_after\":false"), "{}", rm);
        assert!(rm.contains("\"cleanup_ok_settled\":true") && rm.contains("\"settle_ms\":0"), "{}", rm);
        let done = fs::read_to_string(work.join("done")).unwrap();
        assert!(done.contains("\"block_ok\":true") && done.contains("\"cleanup_ok\":true") && done.contains("\"cleanup_ok_settled\":true") && done.contains("\"control_ok\":true") && done.contains("\"off_ok\":true"), "{}", done);
        // the cmd copies stay (the module leaves launched files alone), the blocked installer folder is gone
        let dirs: Vec<String> = fs::read_dir(&work).unwrap().filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().to_string()).collect();
        assert!(dirs.iter().any(|d| d.starts_with("cys-0.0.0-diag-updater-")));
        assert!(!dirs.iter().any(|d| d.starts_with("cys-0.14.42-updater-")), "{:?}", dirs);
    }

    #[test]
    fn abort_file_ends_the_wait() {
        let t = Tmp::new("abort");
        let work = t.0.join("work");
        let cmd = t.0.join("cmd.exe");
        fs::write(&cmd, b"MZ-fake-cmd").unwrap();
        let opts = Options { work: work.clone(), cmd_path: cmd, installer_path: t.0.join("none.exe"), go_wait: Duration::from_secs(20), poll: Duration::from_millis(20) };
        let w2 = work.clone();
        let th = std::thread::spawn(move || {
            let start = Instant::now();
            while !w2.join("ready").exists() && start.elapsed() < Duration::from_secs(15) {
                std::thread::sleep(Duration::from_millis(10));
            }
            fs::write(w2.join("abort"), b"x").unwrap();
        });
        assert_eq!(run(&opts, &fake()), 0);
        th.join().unwrap();
        assert!(fs::read_to_string(work.join("done")).unwrap().contains("aborted"));
        assert!(!fs::read_to_string(work.join("product-launch.jsonl")).unwrap().contains("launch_real_installer"));
    }

    #[test]
    fn go_timeout_is_reported_not_hung() {
        let t = Tmp::new("timeout");
        let work = t.0.join("work");
        let cmd = t.0.join("cmd.exe");
        fs::write(&cmd, b"MZ-fake-cmd").unwrap();
        let opts = Options { work: work.clone(), cmd_path: cmd, installer_path: t.0.join("none.exe"), go_wait: Duration::from_millis(200), poll: Duration::from_millis(20) };
        assert_eq!(run(&opts, &fake()), 0);
        assert!(fs::read_to_string(work.join("done")).unwrap().contains("go_timeout"));
    }

    /// run the whole protocol with `launcher`, give "go" as soon as "ready" is there; returns (jsonl text, done text, elapsed)
    fn run_whole(tag: &str, launcher: Launcher) -> (String, String, Duration) {
        let t = Tmp::new(tag);
        let work = t.0.join("work");
        let cmd = t.0.join("cmd.exe");
        let inst = t.0.join("inst.exe");
        fs::write(&cmd, b"MZ-fake-cmd").unwrap();
        fs::write(&inst, b"MZ-fake-installer-0.14.42").unwrap();
        let opts = Options { work: work.clone(), cmd_path: cmd, installer_path: inst, go_wait: Duration::from_secs(20), poll: Duration::from_millis(20) };
        let w2 = work.clone();
        let th = std::thread::spawn(move || {
            let start = Instant::now();
            while !w2.join("ready").exists() && start.elapsed() < Duration::from_secs(15) {
                std::thread::sleep(Duration::from_millis(10));
            }
            fs::write(w2.join("go"), b"go").unwrap();
        });
        let t0 = Instant::now();
        let code = run(&opts, &launcher);
        let elapsed = t0.elapsed();
        th.join().unwrap();
        assert_eq!(code, 0);
        let log = fs::read_to_string(work.join("product-launch.jsonl")).unwrap();
        for line in log.lines() {
            assert!(line.starts_with('{') && line.ends_with('}') && line.is_ascii(), "{}", line);
        }
        (log, fs::read_to_string(work.join("done")).unwrap(), elapsed)
    }

    /// the (first) result line of a step, e.g. line_of(&log, "off", "launch_cmd_copy")
    fn line_of<'a>(log: &'a str, stage: &str, step: &str) -> &'a str {
        let key = format!("\"stage\":\"{}\",\"step\":\"{}\"", stage, step);
        log.lines().find(|l| l.contains(&key)).unwrap_or_else(|| panic!("no {}/{} line in:\n{}", stage, step, log))
    }

    /// Windows holds the real-installer call: the driver records timed_out, leaves the file alone, still runs the control and writes done
    #[test]
    fn a_held_launch_call_is_given_up_on_and_done_is_still_written() {
        let (log, done, elapsed) = run_whole("held", Launcher { call: held_launcher, limit: Duration::from_millis(300) });
        assert!(elapsed < Duration::from_millis(3500), "the driver waited for the held call: {:?}", elapsed);
        let real = line_of(&log, "on", "launch_real_installer");
        assert!(real.contains("\"timed_out\":true") && real.contains("\"ok\":false") && real.contains("\"os_code\":null") && real.contains("\"matches\":false") && real.contains("\"slow\":false"), "{}", real);
        assert!(line_of(&log, "on", "remove_installer").contains("\"skipped\""), "no removal while the call may still hold the file");
        assert!(line_of(&log, "on", "launch_cmd_copy").contains("\"ok\":true"));
        assert!(done.contains("\"launch_timed_out\":true") && done.contains("\"block_ok\":false") && done.contains("\"control_ok\":true"), "{}", done);
        assert!(!log.contains("launch_cmd_original"), "the control passed: no extra launch");
    }

    #[test]
    fn a_launch_thread_that_panics_is_recorded_and_does_not_kill_the_driver() {
        let (log, done, _) = run_whole("panic", Launcher { call: panicking_launcher, limit: Duration::from_secs(5) });
        let real = line_of(&log, "on", "launch_real_installer");
        assert!(real.contains("\"timed_out\":false") && real.contains("\"not_run\":true") && real.contains("panic?"), "{}", real);
        assert!(line_of(&log, "on", "remove_installer").contains("\"skipped\""), "nothing to clean up after a call that gave no result");
        assert!(!log.contains("launch_cmd_original"), "a call that gave no result is not a blocked copy: no comparison launch");
        assert!(done.contains("\"off_ok\":false") && done.contains("\"block_ok\":false") && done.contains("\"launch_timed_out\":false") && done.contains("\"control_original_ok\":null"), "{}", done);
    }

    /// the control (cmd.exe copy) fails: the ORIGINAL file is launched once more, only to tell a blocked copy from a blocked cmd.exe
    #[test]
    fn a_failing_control_also_tries_the_original_file() {
        let (log, done, _) = run_whole("origin", Launcher { call: copy_blocked_launcher, limit: Duration::from_secs(5) });
        let on_copy = line_of(&log, "on", "launch_cmd_copy");
        assert!(on_copy.contains("\"ok\":false") && on_copy.contains("\"os_code\":4551"), "{}", on_copy);
        let orig = line_of(&log, "on", "launch_cmd_original");
        assert!(orig.contains("\"ok\":true"), "{}", orig);
        assert!(done.contains("\"control_ok\":false") && done.contains("\"control_original_ok\":true"), "{}", done);
    }

    #[test]
    fn a_file_that_vanished_is_not_launched() {
        let t = Tmp::new("vanished");
        let log = Log::new(t.0.join("x.jsonl"));
        let launcher = Launcher { call: panicking_launcher, limit: Duration::from_secs(5) };
        let r = launch_step(&log, "on", "launch_real_installer", &t.0.join("gone-0.14.42.exe"), "/P", Expect::Blocked, &launcher);
        assert!(!r.ok && !r.timed_out && r.not_run && r.os_code.is_none() && r.note.contains("missing"));
        let text = fs::read_to_string(t.0.join("x.jsonl")).unwrap();
        assert!(text.contains("\"skipped_missing_file\":true") && text.contains("\"matches\":false"), "{}", text);
        assert!(!text.contains("_begin"), "no begin line for a call that was never made");
    }

    #[test]
    fn the_slow_flag_is_on_every_timed_step() {
        let (log, _, _) = run_whole("slow", fake());
        for (stage, step) in [("off", "read_cmd_exe"), ("off", "write_cmd_copy"), ("off", "launch_cmd_copy"), ("on", "read_real_installer"), ("on", "write_real_installer"), ("on", "launch_real_installer"), ("on", "remove_installer"), ("on", "write_cmd_copy"), ("on", "launch_cmd_copy")] {
            assert!(line_of(&log, stage, step).contains("\"slow\":false"), "{}/{}", stage, step);
        }
    }
}
