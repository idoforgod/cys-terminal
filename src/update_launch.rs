//! 윈도우 인앱 업데이트 — 설치 파일을 띄우고 **그 결과를 본다**(0.14.43 · WU).
//!
//! 왜 필요한가: `tauri-plugin-updater` 2.10.1 의 윈도우 `install_inner` 는 설치기를 `ShellExecuteW` 로 띄운 뒤 반환값을
//! 보지 않고 곧바로 `std::process::exit(0)` 한다. Windows 의 앱 제어 정책(스마트 앱 컨트롤 등)이 서명 없는 설치 파일의
//! 실행을 막으면(`ShellExecuteW` 반환값 5 · `GetLastError()` = 4551) 앱이 말없이 꺼지고 구버전이 그대로 남는다.
//! 상류는 2.11.0 에서 `if result as isize <= 32 { return Err(..) }` 를 넣었다 — 이 모듈은 **그 동작을 앱 코드로 가져온다**
//! (플러그인은 올리지 않는다: 2.10.1 → 2.13.1 은 맥 경로까지 닿는 리팩터라 이번 판에 넣지 않았다. 2.11.0 의 내용은 티켓 WU 의 조사이고,
//! 이 저장소의 캐시에는 2.10.1 소스만 있어 직접 읽지는 못했다).
//!
//! 성공 경로는 플러그인 2.10.1 과 **같아야 한다**(오너 앵커 "윈도우 인스톨러 업데이트는 극도로 조심"). 같은 것:
//!  · 임시 경로 `%TEMP%\<앱>-<새버전>-updater-<난수 6자>\<앱>-<새버전>-installer.exe`(폴더는 지우지 않고 남긴다 · 파일은 임시 속성)
//!  · 인자 문자열 = `/P /R /UPDATE /ARGS` + 현재 실행 인자(NSIS 이스케이프) — 공백 하나로 이은 문자열
//!  · `ShellExecuteW(NULL, "open", 파일, 인자, NULL, SW_SHOW)` 한 번
//!  · 성공하면 호출부가 곧바로 종료한다(이 모듈은 프로세스를 끝내지 않는다 — 끝내는 것은 호출부)
//!
//! 바뀌는 것은 **하나**다: 반환값이 32 이하(= 실행 실패)면 [`LaunchError`] 를 돌려준다. 호출부는 그때 앱을 닫지 않고
//! 사람 말로 알린다. 이 모듈은 **막혔다는 사실을 정확히 알리는 데까지**만 한다 — 앱 제어 정책을 우회하는 코드는 없다.
//!
//! ★이 파일은 **std 만** 쓴다(외부 크레이트·`crate::`·windows-sys 0 — 윈도우 API 는 `extern "system"` 으로 직접 선언한다).
//! 파일 하나를 단독으로 컴파일·시험할 수 있다: `rustc --edition 2021 --test update_launch.rs`(진짜 스마트 앱 컨트롤 러너에서
//! 이 파일을 그대로 시험하기 위해서다). 순수 함수는 전 OS 에서 컴파일·시험되고, 윈도우 호출([`launch_installer`])만
//! `#[cfg(windows)]` 다.

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// 설치기 임시 폴더 이름의 난수 자리 수 — 플러그인이 쓰는 `tempfile` 의 기본(`NUM_RAND_CHARS` = 6)과 같다.
const SUFFIX_LEN: usize = 6;

/// 난수 폴더 이름이 이미 있어 다시 뽑는 횟수의 상한 — 62^6 가지에서 이만큼 연속으로 겹치면 임시 폴더가 정상이 아니다
/// (무한 재시도 대신 `AlreadyExists` 를 돌려준다).
const MAX_DIR_ATTEMPTS: usize = 100;

/// `FILE_ATTRIBUTE_TEMPORARY`(winnt.h · windows-sys 0.59.0 의 값 256 = 0x100 과 같다) — 플러그인의 `tempfile::NamedTempFile` 이 설치 파일을 만들 때
/// 거는 속성(tempfile 3.27.0 `src/file/imp/windows.rs` `create_named` 의 `custom_flags(FILE_ATTRIBUTE_TEMPORARY)`).
#[cfg(windows)]
const FILE_ATTRIBUTE_TEMPORARY: u32 = 0x0000_0100;

/// 파일 이름·폴더 이름에 쓰는 난수 글자(영숫자 62자 — `tempfile` 의 `alphanumeric` 과 같은 집합).
const ALPHABET: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

/// `LaunchError` 의 `Display` 접두 — UI(`installerLaunchFailure`)가 이 꼴을 파싱한다(`installer_launch_failed:<os_code>:<shell_ret>`).
pub const LAUNCH_FAILED_PREFIX: &str = "installer_launch_failed:";

// ── 순수 함수 ──────────────────────────────────────────────────────────────

/// 설치기에 넘기는 인자 문자열 — 플러그인 2.10.1 의 NSIS 갈래(`updater.rs:801-817`)와 **같은 문자열**.
///
/// `/P /R`(기본 설치 방식 = Passive · `config.rs:41`) + `/UPDATE` + `/ARGS` + (현재 실행 인자 `[1..]` 를
/// [`escape_nsis_arg`] 로 이스케이프한 것)을 **공백 하나**로 잇는다. 현재 실행 인자가 없으면 `"/P /R /UPDATE /ARGS"`.
/// 우리 설정에는 `installer_args` 가 없다(`tauri.conf.json` 의 `plugins.updater` = `pubkey`·`endpoints` 뿐 ·
/// `tauri_plugin_updater::Builder::new().build()` 에 `installer_args` 호출 없음) — 그래서 끝에 덧붙는 것이 없다.
/// `current_args` 는 `argv[1..]`(= 실행 파일 경로를 뺀 나머지)다.
pub fn nsis_update_params(current_args: &[OsString]) -> String {
    let mut parts: Vec<String> = ["/P", "/R", "/UPDATE", "/ARGS"].iter().map(|s| (*s).to_string()).collect();
    parts.extend(current_args.iter().map(|a| escape_nsis_arg(a)));
    parts.join(" ")
}

/// 인자 하나를 NSIS 가 한 낱말로 읽도록 이스케이프한다.
///
/// 출처: tauri-plugin-updater 2.10.1 `src/updater.rs` `escape_nsis_current_exe_arg`(:1501-1532 · MIT OR Apache-2.0)를 그대로
/// 옮겼다 — 그 함수는 Rust std `library/std/src/sys/args/windows.rs` 의 인자 따옴표 규칙(MIT OR Apache-2.0)에서 `/` 를
/// 따옴표 대상으로 더한 변형이다("compared to std we additionally escape `/` so that nsis won't interpret them as a
/// beginning of an nsis argument").
///  · 공백·탭·`/` 가 하나라도 있거나 빈 문자열이면 큰따옴표로 감싼다.
///  · 안쪽 `"` 앞의 역슬래시 n 개는 2n+1 개로(`\"` 로 이스케이프), 닫는 `"` 앞의 역슬래시 n 개는 2n 개로 늘린다.
///  · UTF-8 이 아닌 입력은 `to_string_lossy` 로 접는다(U+FFFD) — 플러그인과 같다.
fn escape_nsis_arg(arg: &OsStr) -> String {
    let arg = arg.to_string_lossy();
    let mut cmd: Vec<char> = Vec::new();

    let quote = arg.chars().any(|c| c == ' ' || c == '\t' || c == '/') || arg.is_empty();
    if quote {
        cmd.push('"');
    }
    let mut backslashes: usize = 0;
    for x in arg.chars() {
        if x == '\\' {
            backslashes += 1;
        } else {
            if x == '"' {
                // 안쪽 '"' 앞: n 개 → 2n+1 개(이미 n 개가 나갔으므로 n+1 개를 더한다).
                cmd.extend((0..=backslashes).map(|_| '\\'));
            }
            backslashes = 0;
        }
        cmd.push(x);
    }
    if quote {
        // 닫는 '"' 앞: n 개 → 2n 개(이미 n 개가 나갔으므로 n 개를 더한다).
        cmd.extend((0..backslashes).map(|_| '\\'));
        cmd.push('"');
    }
    cmd.into_iter().collect()
}

/// 설치기 임시 폴더 이름의 접두 = `"{앱}-{새버전}-updater-"`(플러그인 `make_temp_dir` · `updater.rs:893`).
pub fn installer_dir_prefix(app: &str, version: &str) -> String {
    format!("{app}-{version}-updater-")
}

/// 설치 파일 이름 = `"{앱}-{새버전}-installer.exe"`(플러그인 `write_to_temp` · `updater.rs:941-943` — 접두 + 난수 0자 + `.exe`).
pub fn installer_file_name(app: &str, version: &str) -> String {
    format!("{app}-{version}-installer.exe")
}

/// 받은 바이트가 윈도우 실행 파일(exe)로 보이는가 — 머리 2바이트 `MZ`.
///
/// 플러그인이 쓰는 `infer::app::is_exe`(infer 0.19.0 `src/matchers/app.rs:31-33`)와 **같은 판정**이다:
/// `buf.len() > 1 && buf[0] == 0x4D && buf[1] == 0x5A`. zip(`PK`)·MSI(`D0 CF 11 E0`)는 false — 그런 형식은 호출부가
/// 플러그인의 `install` 에 맡긴다(동작 불변).
pub fn looks_like_exe(bytes: &[u8]) -> bool {
    bytes.len() > 1 && bytes[0] == 0x4D && bytes[1] == 0x5A
}

/// 설치 파일 실행이 막힌 이유의 분류 — 사람에게 보일 문구를 고르는 데 쓴다(UI 의 `installerLaunchFailure` 와 같은 표).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchBlock {
    /// 4551 `ERROR_SYSTEM_INTEGRITY_POLICY_VIOLATION` — 앱 제어 정책(스마트 앱 컨트롤·WDAC)이 이 파일을 막았다.
    AppControl,
    /// 5 `ERROR_ACCESS_DENIED` — 접근 거부(보안 프로그램·권한).
    AccessDenied,
    /// 2 `ERROR_FILE_NOT_FOUND` · 3 `ERROR_PATH_NOT_FOUND` — 방금 쓴 설치 파일이 없다(보안 프로그램이 격리했을 수 있다).
    NotFound,
    /// 225 `ERROR_VIRUS_INFECTED` — 보안 프로그램이 위험한 파일로 판정했다.
    Infected,
    /// 1223 `ERROR_CANCELLED` — 사용자가 취소했다.
    Cancelled,
    /// 그 밖.
    Other,
}

/// 윈도우 오류 번호 → 막힌 이유(4551 → AppControl · 5 → AccessDenied · 2·3 → NotFound · 225 → Infected · 1223 → Cancelled).
pub fn classify_launch_error(os_code: u32) -> LaunchBlock {
    match os_code {
        4551 => LaunchBlock::AppControl,
        5 => LaunchBlock::AccessDenied,
        2 | 3 => LaunchBlock::NotFound,
        225 => LaunchBlock::Infected,
        1223 => LaunchBlock::Cancelled,
        _ => LaunchBlock::Other,
    }
}

/// 설치 파일을 실행하지 못했다 — `ShellExecuteW` 가 32 이하를 돌려줬다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LaunchError {
    /// `ShellExecuteW` 의 반환값(32 이하 = 실패 · 5 = `SE_ERR_ACCESSDENIED` 등).
    pub shell_ret: isize,
    /// 호출 직후 읽은 `GetLastError()`(막힌 진짜 이유 — 앱 제어 정책 차단이면 4551). 마지막 오류가 비어 있으면(0)
    /// `ShellExecuteW` 반환값으로 대신한다(`SE_ERR_FNF`=2·`SE_ERR_PNF`=3·`SE_ERR_ACCESSDENIED`=5·`SE_ERR_OOM`=8 은 같은 번호의 윈도우 오류
    /// `ERROR_FILE_NOT_FOUND`·`ERROR_PATH_NOT_FOUND`·`ERROR_ACCESS_DENIED`·`ERROR_NOT_ENOUGH_MEMORY` 와 같은 값이다 — windows-sys 0.59.0 상수로 확인).
    pub os_code: u32,
}

impl LaunchError {
    /// 이 실패를 분류한다([`classify_launch_error`] 의 짧은 길).
    pub fn block(&self) -> LaunchBlock {
        classify_launch_error(self.os_code)
    }
}

/// `installer_launch_failed:<os_code>:<shell_ret>` — UI 가 이 꼴을 파싱한다(`ui/src/updatenotice.ts` `installerLaunchFailure`).
/// 형식은 계약이다: 바꾸면 UI 가 사람 말 문구 대신 종전의 날것 토스트로 되돌아간다.
impl fmt::Display for LaunchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}:{}", LAUNCH_FAILED_PREFIX, self.os_code, self.shell_ret)
    }
}

impl std::error::Error for LaunchError {}

/// `ShellExecuteW` 의 결과 판정 — 순수 함수(맥에서도 시험한다).
///
/// 반환값이 **33 이상이면 성공**(MSDN: "greater than 32 if successful") · **32 이하면 실패**다. 실패면 호출 직후 읽은
/// `last_error`(`GetLastError()`)를 `os_code` 로 싣는다(0 이면 반환값으로 대신). 티켓이 인용한 상류 2.11.0 의
/// `if result as isize <= 32 { return Err(Io(last_os_error())) }` 와 같은 경계다.
/// ★이 판정을 건너뛰면(항상 성공 취급) 막힌 실행이 다시 침묵한다 — 돌연변이 M1 이 이 함수를 잰다.
pub fn shell_result(ret: isize, last_error: u32) -> Result<(), LaunchError> {
    if ret > 32 {
        return Ok(());
    }
    let os_code = if last_error != 0 { last_error } else { u32::try_from(ret).unwrap_or(0) };
    Err(LaunchError { shell_ret: ret, os_code })
}

// ── 임시 설치 파일 ─────────────────────────────────────────────────────────

/// 난수 6자(영숫자) — std 만으로(시각 나노초·pid·호출 횟수를 프로세스별 무작위 키의 해시에 섞는다).
/// 예측 불가일 필요는 없고 **충돌만 피하면** 된다(충돌은 `create_dir` 가 원자적으로 잡아 다시 뽑는다).
fn random_suffix() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let mut h = RandomState::new().build_hasher();
    h.write_u128(nanos);
    h.write_u32(std::process::id());
    h.write_u64(n);
    let mut v = h.finish();
    let mut s = String::with_capacity(SUFFIX_LEN);
    for _ in 0..SUFFIX_LEN {
        s.push(ALPHABET[(v % ALPHABET.len() as u64) as usize] as char);
        v /= ALPHABET.len() as u64;
    }
    s
}

/// 설치 파일을 새로 만든다(이미 있으면 실패). 윈도우에서는 플러그인의 `tempfile::NamedTempFile` 과 같은 열기 방식
/// (`create_new` + 읽기·쓰기 + `FILE_ATTRIBUTE_TEMPORARY`)이다.
fn create_new_file(path: &Path) -> io::Result<std::fs::File> {
    let mut o = std::fs::OpenOptions::new();
    o.create_new(true).read(true).write(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        o.custom_flags(FILE_ATTRIBUTE_TEMPORARY);
    }
    o.open(path)
}

/// 받은 바이트를 `<temp_root>\<앱>-<새버전>-updater-<난수 6자>\<앱>-<새버전>-installer.exe` 에 쓰고 그 경로를 돌려준다.
///
/// 플러그인의 `make_temp_dir` + `write_to_temp`(`updater.rs:891-949`)와 같은 경로 꼴이다. 폴더는 새로 만든다 —
/// `create_dir` 는 이미 있으면 실패하므로 원자적이고, 겹치면 다른 난수로 다시 시도한다(상한 [`MAX_DIR_ATTEMPTS`]).
/// 파일은 닫은 뒤에 돌려준다(설치기를 띄우기 전에 핸들이 남아 있지 않다). 폴더는 호출부가 지우지 않는 한 남는다
/// (플러그인의 `.keep()` 과 같다 — 설치기가 그 파일로 돌고 있다).
/// 쓰는 도중 실패하면 만든 파일·폴더를 치우고 오류를 돌려준다.
pub fn write_installer(temp_root: &Path, app: &str, version: &str, bytes: &[u8]) -> io::Result<PathBuf> {
    write_installer_with(temp_root, app, version, bytes, random_suffix)
}

/// [`write_installer`] 의 몸통 — 난수 공급을 바깥에서 받아 겹침 재시도를 결정론으로 시험한다.
fn write_installer_with(
    temp_root: &Path,
    app: &str,
    version: &str,
    bytes: &[u8],
    mut next_suffix: impl FnMut() -> String,
) -> io::Result<PathBuf> {
    let prefix = installer_dir_prefix(app, version);
    let name = installer_file_name(app, version);
    for _ in 0..MAX_DIR_ATTEMPTS {
        let dir = temp_root.join(format!("{prefix}{}", next_suffix()));
        match std::fs::create_dir(&dir) {
            Ok(()) => {
                let file = dir.join(&name);
                // 클로저가 끝나면 파일 핸들이 닫힌다 — 돌려주기 전에 닫혀 있어야 설치기가 읽을 수 있다.
                let written = create_new_file(&file).and_then(|mut f| f.write_all(bytes));
                return match written {
                    Ok(()) => Ok(file),
                    Err(e) => {
                        let _ = std::fs::remove_file(&file);
                        let _ = std::fs::remove_dir(&dir);
                        Err(e)
                    }
                };
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!("임시 설치 폴더 이름이 {MAX_DIR_ATTEMPTS}번 연속으로 이미 있다: {}", temp_root.display()),
    ))
}

/// 띄우지 못한 설치 파일과 그 임시 폴더를 치운다(최선 노력 — 실패해도 조용하다).
/// 폴더는 **비어 있고 이름에 `-updater-` 가 들어 있을 때만** 지운다(`remove_dir` 는 빈 폴더만 지운다 — 엉뚱한 폴더를 건드리지 않는다).
/// 설치기가 **뜬** 경우에는 부르지 않는다(설치기가 그 파일로 돌고 있다).
pub fn remove_installer(file: &Path) {
    let _ = std::fs::remove_file(file);
    if let Some(dir) = file.parent() {
        let ours = dir.file_name().and_then(|n| n.to_str()).map_or(false, |n| n.contains("-updater-"));
        if ours {
            let _ = std::fs::remove_dir(dir);
        }
    }
}

// ── 윈도우 호출 ────────────────────────────────────────────────────────────

/// 윈도우 API 직접 선언(windows-sys 를 쓰지 않는다 — 이 파일은 std 만으로 서 있어야 한다).
#[cfg(windows)]
mod sys {
    use std::ffi::c_void;

    #[link(name = "shell32")]
    extern "system" {
        /// `HINSTANCE ShellExecuteW(HWND, LPCWSTR, LPCWSTR, LPCWSTR, LPCWSTR, INT)` — `HINSTANCE` 는 포인터 폭 정수로
        /// 돌아온다(33 이상 = 성공 · 32 이하 = 오류 번호).
        pub fn ShellExecuteW(
            hwnd: *mut c_void,
            operation: *const u16,
            file: *const u16,
            parameters: *const u16,
            directory: *const u16,
            show_cmd: i32,
        ) -> isize;
    }

    #[link(name = "kernel32")]
    extern "system" {
        pub fn GetLastError() -> u32;
        pub fn SetLastError(err_code: u32);
    }
}

/// 설치기를 띄운다 — 플러그인 2.10.1 과 **같은 인자**로 `ShellExecuteW(NULL, "open", 파일, 인자, NULL, SW_SHOW)` 를 한 번 부르고,
/// 반환값이 **32 이하면 호출 직후** `GetLastError()` 를 읽어 [`LaunchError`] 를 돌려준다([`shell_result`]).
///
/// 이 함수는 **프로세스를 끝내지 않는다** — 성공했을 때 앱을 닫는 것은 호출부다(성공 경로의 종료 시점은 종전과 같다).
/// `ShellExecuteW` 가 마지막 오류를 채운다고 보장되지 않으므로 호출 전에 0 으로 비운다(남은 값이 오류로 읽히지 않게).
/// 와이드 문자열은 NUL 로 끝낸다(플러그인의 `encode_wide` 와 같다). `file` 은 따옴표 없이 그대로 넘긴다(플러그인과 같다).
/// 호출 한 번이 끝나면 돌아온다 — 설치기가 끝나기를 기다리지 않는다.
/// 대상 파일이 없거나 형식이 틀리면 Windows 가 자체 오류 창을 띄운 뒤(사용자가 닫으면) 실패를 돌려준다 — 이 API 에는 창을 끄는 플래그가 없다
/// (플러그인과 같은 호출을 유지한다).
#[cfg(windows)]
pub fn launch_installer(file: &Path, params: &str) -> Result<(), LaunchError> {
    use std::os::windows::ffi::OsStrExt;

    /// SW_SHOW(winuser.h · windows-sys 0.59.0 의 값 5) — 플러그인이 넘기는 값과 같다.
    const SW_SHOW: i32 = 5;

    fn wide(s: &OsStr) -> Vec<u16> {
        s.encode_wide().chain(std::iter::once(0)).collect()
    }

    let operation = wide(OsStr::new("open"));
    let file_w = wide(file.as_os_str());
    let params_w = wide(OsStr::new(params));
    // SAFETY: 세 문자열은 NUL 로 끝나는 UTF-16 이고 호출이 끝날 때까지 살아 있다. hwnd·directory 는 null(= 플러그인과 같다).
    // GetLastError 는 ShellExecuteW 반환 **직후** — 그 사이에 다른 윈도우 호출이 없다.
    let (ret, last_error) = unsafe {
        sys::SetLastError(0);
        let ret = sys::ShellExecuteW(
            std::ptr::null_mut(),
            operation.as_ptr(),
            file_w.as_ptr(),
            params_w.as_ptr(),
            std::ptr::null(),
            SW_SHOW,
        );
        (ret, sys::GetLastError())
    };
    shell_result(ret, last_error)
}

// ── 검체 ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn os(args: &[&str]) -> Vec<OsString> {
        args.iter().map(|s| OsString::from(*s)).collect()
    }

    /// 시험용 임시 루트 — 끝나면 통째로 지운다.
    struct TmpRoot(PathBuf);
    impl TmpRoot {
        fn new(tag: &str) -> Self {
            static N: AtomicUsize = AtomicUsize::new(0);
            let p = std::env::temp_dir().join(format!(
                "cys-update-launch-test-{}-{tag}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            TmpRoot(p)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TmpRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    // ── 인자 문자열 ──

    /// 인자 없음 — 플러그인의 `nsis_args()`(Passive = `/P`·`/R`) + `/UPDATE` + `/ARGS` 가 공백 하나로 이어진 것뿐이다.
    #[test]
    fn params_without_args_is_the_fixed_head() {
        assert_eq!(nsis_update_params(&[]), "/P /R /UPDATE /ARGS");
    }

    /// 손으로 따라간 기대값(플러그인 `escape_nsis_current_exe_arg` 알고리즘 · updater.rs:1501-1532):
    ///  따옴표 판정 `quote` = (공백·탭·`/` 가 하나라도 있다) 또는 빈 문자열.
    ///  글자마다: `\` 면 `backslashes += 1` · 그 밖이면(`"` 일 때만 앞에 `backslashes + 1` 개의 `\` 를 더 내보낸 뒤) `backslashes = 0`.
    ///  끝에서 `quote` 면 `backslashes` 개의 `\` 를 더 내고 `"` 로 닫는다.
    #[test]
    fn params_escape_vectors_hand_derived() {
        // (입력 인자, 이스케이프 결과)
        let cases: Vec<(&str, &str)> = vec![
            // A. `--flag` — 공백·탭·`/` 없음, 비어 있지 않음 → 따옴표 없음. `\`·`"` 없음 → 그대로.
            ("--flag", "--flag"),
            // B. `a b` — 공백 → 따옴표. `\`·`"` 없음 → `"a b"`.
            ("a b", "\"a b\""),
            // C. `a<TAB>b` — 탭 → 따옴표 → `"a<TAB>b"`.
            ("a\tb", "\"a\tb\""),
            // D. `/x` — `/` → 따옴표(std 와 다른 점: NSIS 가 `/x` 를 자기 옵션으로 읽지 않게) → `"/x"`.
            ("/x", "\"/x\""),
            // E. `a/b` — `/` 가 가운데에 있어도 → `"a/b"`.
            ("a/b", "\"a/b\""),
            // F. `` — 빈 문자열 → 따옴표 → `""`(NSIS 에서 빈 인자가 사라지지 않게).
            ("", "\"\""),
            // G. `--dir=C:\Program Files\x` — 공백 → 따옴표. `\` 는 뒤가 `"` 가 아니므로 `backslashes` 가 올랐다 0 으로 돌아올 뿐
            //    아무것도 더 내보내지 않는다. 끝 글자가 `x` → `backslashes` = 0 → 닫는 따옴표 앞에 더할 것 없음.
            (r"--dir=C:\Program Files\x", r#""--dir=C:\Program Files\x""#),
            // H. `say "hi"` — 공백 → 따옴표. 안쪽 `"` 두 개 앞의 역슬래시는 0 개 → `0 + 1` = 1 개씩 더한다 → `\"hi\"`.
            //    끝 `backslashes` = 0 → `"say \"hi\""`.
            (r#"say "hi""#, r#""say \"hi\"""#),
            // I. `C:\my dir\` — 공백 → 따옴표. 끝 글자가 `\` → `backslashes` = 1 → 닫는 따옴표 앞에 1 개 더 → `\\` 두 개 + `"`.
            (r"C:\my dir\", r#""C:\my dir\\""#),
            // J. `a\"b c` — 공백 → 따옴표. `\` 1 개(n=1) 뒤에 `"` → `n + 1` = 2 개 더 → 모두 3 개 + `"`. 끝 `backslashes` = 0.
            (r#"a\"b c"#, r#""a\\\"b c""#),
            // K. `a\\"b c` — `\` 2 개(n=2) 뒤에 `"` → 3 개 더 → 모두 5 개 + `"`.
            (r#"a\\"b c"#, r#""a\\\\\"b c""#),
            // L. `"` 하나 — 공백·탭·`/` 없음 → 따옴표 없음. `"` 앞 `backslashes` = 0 → 1 개 더 → `\"`.
            (r#"""#, r#"\""#),
            // M. `C:\dir\` — 따옴표 없음(공백 없음) → 끝 역슬래시는 그대로 한 개(`quote` 가 거짓이라 더하지 않는다).
            (r"C:\dir\", r"C:\dir\"),
            // N. 한글 — 공백이 있어 따옴표 → `"한 글"`(글자 단위로 다룬다).
            ("한 글", "\"한 글\""),
        ];
        for (input, want) in cases {
            let got = nsis_update_params(&os(&[input]));
            assert_eq!(got, format!("/P /R /UPDATE /ARGS {want}"), "인자 {input:?}");
        }
    }

    /// 여러 인자는 이스케이프한 낱말을 공백 하나로 잇는다(앞에 고정 머리 · 끝에 공백 없음).
    #[test]
    fn params_join_with_single_spaces() {
        let got = nsis_update_params(&os(&["--a", "b c", "/d", "e"]));
        assert_eq!(got, "/P /R /UPDATE /ARGS --a \"b c\" \"/d\" e");
        assert!(!got.ends_with(' '), "끝 공백 없음");
        assert!(!got.contains("  "), "공백 둘 연속 없음");
    }

    /// UTF-8 이 아닌 인자는 `to_string_lossy` 로 접힌다(플러그인과 같다 — U+FFFD).
    #[cfg(unix)]
    #[test]
    fn params_non_utf8_arg_is_lossy() {
        use std::os::unix::ffi::OsStringExt;
        let a = OsString::from_vec(vec![b'a', 0xFF, b'b']);
        assert_eq!(nsis_update_params(&[a]), "/P /R /UPDATE /ARGS a\u{FFFD}b");
    }

    // ── 이름·판정 ──

    #[test]
    fn dir_prefix_and_file_name_follow_the_plugin() {
        assert_eq!(installer_dir_prefix("cys", "0.14.43"), "cys-0.14.43-updater-");
        assert_eq!(installer_file_name("cys", "0.14.43"), "cys-0.14.43-installer.exe");
        // 앱 이름·버전은 그대로 끼워 넣는다(가공 없음)
        assert_eq!(installer_dir_prefix("My App", "1.0.0-rc.1"), "My App-1.0.0-rc.1-updater-");
        assert_eq!(installer_file_name("My App", "1.0.0-rc.1"), "My App-1.0.0-rc.1-installer.exe");
    }

    /// `infer::app::is_exe` 와 같은 판정(머리 `MZ` 두 바이트 · 길이 2 이상).
    #[test]
    fn looks_like_exe_is_the_mz_header() {
        assert!(looks_like_exe(b"MZ"), "정확히 두 바이트 MZ");
        assert!(looks_like_exe(b"MZ\x90\x00\x03\x00\x00\x00"), "실제 PE 머리");
        assert!(looks_like_exe(&[0x4D, 0x5A, 0, 0, 0]));
        assert!(!looks_like_exe(b""), "빈 바이트");
        assert!(!looks_like_exe(b"M"), "한 바이트");
        assert!(!looks_like_exe(b"mz"), "소문자");
        assert!(!looks_like_exe(b"ZM"), "순서 뒤집힘");
        assert!(!looks_like_exe(b" MZ"), "앞에 다른 바이트");
        assert!(!looks_like_exe(b"PK\x03\x04"), "zip 은 exe 가 아니다(플러그인의 zip 갈래로 간다)");
        assert!(!looks_like_exe(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]), "MSI(OLE) 머리");
        assert!(!looks_like_exe(b"<html>"), "HTML 오류 페이지");
    }

    #[test]
    fn classification_table() {
        let table: [(u32, LaunchBlock); 12] = [
            (4551, LaunchBlock::AppControl),
            (5, LaunchBlock::AccessDenied),
            (2, LaunchBlock::NotFound),
            (3, LaunchBlock::NotFound),
            (225, LaunchBlock::Infected),
            (1223, LaunchBlock::Cancelled),
            (0, LaunchBlock::Other),
            (1, LaunchBlock::Other),
            (4, LaunchBlock::Other),
            (193, LaunchBlock::Other),
            (4552, LaunchBlock::Other),
            (u32::MAX, LaunchBlock::Other),
        ];
        for (code, want) in table {
            assert_eq!(classify_launch_error(code), want, "오류 {code}");
        }
        // 4551 이 Other 로 떨어지면 UI 가 '앱 제어 정책이 막았다'를 말하지 못한다 — 가장 중요한 한 줄
        assert_eq!(LaunchError { shell_ret: 5, os_code: 4551 }.block(), LaunchBlock::AppControl);
    }

    /// `Display` 는 UI 가 파싱하는 계약이다: `installer_launch_failed:<os_code>:<shell_ret>`.
    #[test]
    fn launch_error_display_is_the_ui_contract() {
        assert_eq!(LaunchError { shell_ret: 5, os_code: 4551 }.to_string(), "installer_launch_failed:4551:5");
        assert_eq!(LaunchError { shell_ret: 2, os_code: 2 }.to_string(), "installer_launch_failed:2:2");
        assert_eq!(LaunchError { shell_ret: 0, os_code: 0 }.to_string(), "installer_launch_failed:0:0");
        assert_eq!(LaunchError { shell_ret: -1, os_code: 7 }.to_string(), "installer_launch_failed:7:-1");
        assert_eq!(LAUNCH_FAILED_PREFIX, "installer_launch_failed:");
        // 꼴 점검: 접두 뒤가 `:` 로 나뉘는 두 정수다
        let s = LaunchError { shell_ret: 5, os_code: 4551 }.to_string();
        let rest = s.strip_prefix(LAUNCH_FAILED_PREFIX).expect("접두");
        let parts: Vec<&str> = rest.split(':').collect();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].parse::<u32>().unwrap(), 4551);
        assert_eq!(parts[1].parse::<isize>().unwrap(), 5);
    }

    /// 실행 결과 판정 — 33 이상 성공 · 32 이하 실패(경계 포함) · 실패면 호출 직후 읽은 마지막 오류가 `os_code`.
    /// ★`launch_installer` 가 반환값을 무시하면(돌연변이 M1) 이 표가 붉어진다.
    #[test]
    fn shell_result_boundary_and_codes() {
        // 성공: 33 이상(실측 성공 반환값은 42)
        assert_eq!(shell_result(42, 0), Ok(()));
        assert_eq!(shell_result(33, 0), Ok(()), "경계 33 = 성공");
        assert_eq!(shell_result(33, 4551), Ok(()), "성공이면 마지막 오류는 보지 않는다");
        assert_eq!(shell_result(isize::MAX, 0), Ok(()));
        // 실패: 32 이하(경계 32 포함) — 앱 제어 정책 차단의 실측값: 반환 5 · 마지막 오류 4551
        assert_eq!(shell_result(5, 4551), Err(LaunchError { shell_ret: 5, os_code: 4551 }));
        assert_eq!(shell_result(32, 5), Err(LaunchError { shell_ret: 32, os_code: 5 }), "경계 32 = 실패");
        assert_eq!(shell_result(31, 1155), Err(LaunchError { shell_ret: 31, os_code: 1155 }));
        assert_eq!(shell_result(2, 2), Err(LaunchError { shell_ret: 2, os_code: 2 }));
        // 마지막 오류가 비어 있으면 반환값으로 대신한다(2·3·5·8·11 은 윈도우 오류 번호와 같은 값)
        assert_eq!(shell_result(5, 0), Err(LaunchError { shell_ret: 5, os_code: 5 }));
        assert_eq!(shell_result(0, 0), Err(LaunchError { shell_ret: 0, os_code: 0 }));
        // 음수 반환(있을 수 없는 값)도 성공으로 읽지 않는다
        assert_eq!(shell_result(-1, 0), Err(LaunchError { shell_ret: -1, os_code: 0 }));
        assert_eq!(shell_result(-1, 7), Err(LaunchError { shell_ret: -1, os_code: 7 }));
        // 실패를 분류까지 이어 보면: 실측 차단 한 건이 AppControl 이다
        assert_eq!(shell_result(5, 4551).unwrap_err().block(), LaunchBlock::AppControl);
        // ★실패 경로의 실제 API 검체를 대신한다(`ShellExecuteW` 는 없는 파일에 모달 오류 창을 띄워 CI 에서 돌릴 수 없다 — 윈도우 실기 검체는 성공 경로 하나뿐).
        //   (반환값, 마지막 오류) → (os_code, 분류): 반환 2·오류 0 → 2 → NotFound · 반환 5·오류 4551 → 4551 → AppControl(실측 SAC 차단) 등
        let table: [(isize, u32, u32, LaunchBlock); 12] = [
            (2, 0, 2, LaunchBlock::NotFound),
            (3, 0, 3, LaunchBlock::NotFound),
            (2, 2, 2, LaunchBlock::NotFound),
            (3, 3, 3, LaunchBlock::NotFound),
            (5, 0, 5, LaunchBlock::AccessDenied),
            (5, 5, 5, LaunchBlock::AccessDenied),
            (5, 4551, 4551, LaunchBlock::AppControl),
            (32, 4551, 4551, LaunchBlock::AppControl),
            (5, 225, 225, LaunchBlock::Infected),
            (5, 1223, 1223, LaunchBlock::Cancelled),
            (31, 1155, 1155, LaunchBlock::Other),
            (0, 0, 0, LaunchBlock::Other),
        ];
        for (ret, last, code, block) in table {
            let e = shell_result(ret, last).expect_err("32 이하는 실패여야 한다");
            assert_eq!((e.shell_ret, e.os_code, e.block()), (ret, code, block), "반환 {ret} · 마지막 오류 {last}");
        }
    }

    // ── 임시 설치 파일 ──

    #[test]
    fn random_suffix_is_six_alphanumerics_and_varies() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..200 {
            let s = random_suffix();
            assert_eq!(s.len(), SUFFIX_LEN, "{s}");
            assert!(s.bytes().all(|b| b.is_ascii_alphanumeric()), "{s}");
            seen.insert(s);
        }
        assert!(seen.len() > 190, "200번 중 서로 다른 값이 너무 적다: {}", seen.len());
    }

    #[test]
    fn write_installer_writes_bytes_under_a_prefixed_fresh_dir() {
        let root = TmpRoot::new("write");
        let bytes = b"MZ\x90\x00 fake installer bytes".to_vec();
        let file = write_installer(root.path(), "cys", "0.14.43", &bytes).unwrap();
        // 파일: 이름 · 내용
        assert_eq!(file.file_name().unwrap().to_str().unwrap(), "cys-0.14.43-installer.exe");
        assert_eq!(std::fs::read(&file).unwrap(), bytes);
        // 폴더: 루트 바로 아래 · `cys-0.14.43-updater-` + 영숫자 6자
        let dir = file.parent().unwrap();
        assert_eq!(dir.parent().unwrap(), root.path());
        let dname = dir.file_name().unwrap().to_str().unwrap();
        let suffix = dname.strip_prefix("cys-0.14.43-updater-").expect("접두");
        assert_eq!(suffix.len(), 6, "{dname}");
        assert!(suffix.bytes().all(|b| b.is_ascii_alphanumeric()), "{dname}");
        // 폴더에는 설치 파일 하나뿐이다
        assert_eq!(std::fs::read_dir(dir).unwrap().count(), 1);
    }

    #[test]
    fn write_installer_twice_gives_distinct_dirs() {
        let root = TmpRoot::new("twice");
        let a = write_installer(root.path(), "cys", "0.14.43", b"MZ-a").unwrap();
        let b = write_installer(root.path(), "cys", "0.14.43", b"MZ-b").unwrap();
        assert_ne!(a.parent(), b.parent(), "같은 루트에 두 번 불러도 다른 폴더");
        assert_eq!(std::fs::read(&a).unwrap(), b"MZ-a");
        assert_eq!(std::fs::read(&b).unwrap(), b"MZ-b");
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 2);
    }

    #[test]
    fn write_installer_empty_bytes_still_writes_the_file() {
        let root = TmpRoot::new("empty");
        let file = write_installer(root.path(), "cys", "1.0.0", b"").unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"");
    }

    /// 겹치면 다른 난수로 다시 시도한다 — 미리 만든 폴더가 두 번 걸리고 세 번째에 성공(난수 공급을 바깥에서 준다).
    #[test]
    fn write_installer_retries_when_the_dir_name_is_taken() {
        let root = TmpRoot::new("retry");
        std::fs::create_dir(root.path().join("cys-0.14.43-updater-AAAAAA")).unwrap();
        let mut seq = vec!["BBBBBB", "AAAAAA", "AAAAAA"]; // pop 은 뒤에서 — AAAAAA, AAAAAA, BBBBBB 순
        let file = write_installer_with(root.path(), "cys", "0.14.43", b"MZ", || seq.pop().unwrap().to_string()).unwrap();
        assert_eq!(file.parent().unwrap().file_name().unwrap(), "cys-0.14.43-updater-BBBBBB");
        assert_eq!(std::fs::read(&file).unwrap(), b"MZ");
        assert!(seq.is_empty(), "세 번 뽑았다");
        // 이미 있던 폴더는 건드리지 않았다
        assert!(root.path().join("cys-0.14.43-updater-AAAAAA").is_dir());
    }

    /// 계속 겹치면 무한 재시도하지 않고 `AlreadyExists` 로 끝난다.
    #[test]
    fn write_installer_gives_up_after_the_attempt_cap() {
        let root = TmpRoot::new("cap");
        std::fs::create_dir(root.path().join("cys-0.14.43-updater-ZZZZZZ")).unwrap();
        let mut calls = 0usize;
        let err = write_installer_with(root.path(), "cys", "0.14.43", b"MZ", || {
            calls += 1;
            "ZZZZZZ".to_string()
        })
        .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(calls, MAX_DIR_ATTEMPTS, "상한만큼만 시도한다");
    }

    /// 루트가 없으면(겹침이 아닌 오류) 재시도하지 않고 그 오류를 돌려준다 — 패닉 없음.
    #[test]
    fn write_installer_missing_root_is_an_error_not_a_retry_loop() {
        let root = TmpRoot::new("noroot");
        let missing = root.path().join("no").join("such").join("dir");
        let mut calls = 0usize;
        let err = write_installer_with(&missing, "cys", "0.14.43", b"MZ", || {
            calls += 1;
            random_suffix()
        })
        .unwrap_err();
        assert_ne!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(calls, 1, "겹침이 아닌 오류에는 다시 시도하지 않는다");
        assert!(!missing.exists());
    }

    /// 치우기: 파일과 빈 임시 폴더를 지운다 · 두 번째 호출은 무해 · 이름이 우리 것이 아닌 폴더는 건드리지 않는다.
    #[test]
    fn remove_installer_cleans_file_and_its_own_dir_only() {
        let root = TmpRoot::new("remove");
        let file = write_installer(root.path(), "cys", "0.14.43", b"MZ").unwrap();
        let dir = file.parent().unwrap().to_path_buf();
        remove_installer(&file);
        assert!(!file.exists(), "설치 파일이 남았다");
        assert!(!dir.exists(), "임시 폴더가 남았다");
        remove_installer(&file); // 없어도 무해
        assert!(root.path().is_dir(), "루트는 건드리지 않는다");
        // 이름에 `-updater-` 가 없는 폴더 안의 파일은 파일만 지우고 폴더는 둔다
        let other = root.path().join("some-other-dir");
        std::fs::create_dir(&other).unwrap();
        let f = other.join("x.exe");
        std::fs::write(&f, b"MZ").unwrap();
        remove_installer(&f);
        assert!(!f.exists());
        assert!(other.is_dir(), "우리 것이 아닌 폴더를 지웠다");
        // 폴더에 다른 파일이 남아 있으면(비어 있지 않으면) 폴더는 둔다
        let file2 = write_installer(root.path(), "cys", "0.14.43", b"MZ").unwrap();
        let dir2 = file2.parent().unwrap().to_path_buf();
        std::fs::write(dir2.join("keep.txt"), b"x").unwrap();
        remove_installer(&file2);
        assert!(!file2.exists());
        assert!(dir2.join("keep.txt").exists(), "비어 있지 않은 폴더의 다른 파일을 지웠다");
    }

    // ── 윈도우 실기(윈도우 CI 가 실제로 돌린다) — 성공 경로 하나뿐이다 ──
    // 실패 경로는 실제 API 로 돌리지 않는다: `ShellExecuteW` 는 `SEE_MASK_FLAG_NO_UI` 가 없는 구식 API 라 대상 파일이 없거나 형식이 틀리면 Windows 가
    // 모달 오류 창을 띄우고 닫힐 때까지 돌아오지 않는다(hwnd 가 NULL 이어도 뜬다) — 대화형 세션인 CI 러너에서는 그 창이 실제로 떠 잡이 멈춘다.
    // 실패 판정(반환 2·3·5 · 마지막 오류 4551 등 → `os_code` · 분류)은 순수 검체 `shell_result_boundary_and_codes` 가 전 OS 에서 잰다.

    #[cfg(windows)]
    fn cmd_exe() -> PathBuf {
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| OsString::from(r"C:\Windows"));
        PathBuf::from(root).join("System32").join("cmd.exe")
    }

    /// 실제 `ShellExecuteW` — 뜨는 프로그램은 곧바로 끝나는 `cmd.exe /c exit 0`. 성공이면 `Ok`.
    /// (콘솔 창이 잠깐 떴다 닫힐 뿐 모달 창은 없다 — 성공 경로에서는 오류 창이 뜰 일이 없어 호출이 곧바로 돌아온다.)
    #[cfg(windows)]
    #[test]
    fn windows_launch_cmd_exe_succeeds() {
        let cmd = cmd_exe();
        assert!(cmd.is_file(), "cmd.exe 가 없다: {}", cmd.display());
        launch_installer(&cmd, "/c exit 0").expect("cmd.exe 실행은 성공해야 한다");
    }
}
