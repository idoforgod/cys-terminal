//! cys (CYSJavis Terminal) — shared protocol types, socket path resolution, and key mapping.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

pub mod action_catalog;
pub mod directive_compose;
pub mod edit_kinds;
pub mod license;
pub mod pack;
pub mod packsig;
pub mod overrides;
pub mod todo_decl;
pub mod todo_scan;
pub mod wire;
#[cfg(target_os = "macos")]
pub mod launchd;

pub const ENV_SOCKET: &str = "CYS_SOCKET";
pub const ENV_SURFACE_ID: &str = "CYS_SURFACE_ID";
pub const ENV_SURFACE_REF: &str = "CYS_SURFACE_REF";
pub const ENV_ROLE: &str = "CYS_ROLE";

/// 이행기 호환: CYS_* 우선 → 구 JAVIS_* → 구 AITERM_* 순 폴백.
pub fn env_compat(primary: &str) -> Option<String> {
    let javis = primary.replacen("CYS_", "JAVIS_", 1);
    let aiterm = primary.replacen("CYS_", "AITERM_", 1);
    [primary, javis.as_str(), aiterm.as_str()]
        .iter()
        .find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))
}

/// Wire protocol: one JSON object per line (NDJSON), request/response with id echo.
#[derive(Debug, Serialize, Deserialize)]
pub struct Request {
    #[serde(default)]
    pub id: Value,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

pub fn ok_response(id: &Value, result: Value) -> Value {
    serde_json::json!({"id": id, "ok": true, "result": result})
}

pub fn err_response(id: &Value, code: &str, message: &str) -> Value {
    serde_json::json!({"id": id, "ok": false, "error": {"code": code, "message": message}})
}

/// Default socket path: ~/.local/state/cys/cys.sock (unix),
/// \\.\pipe\cys (windows). Overridable via CYS_SOCKET (legacy JAVIS_/AITERM_ honored).
pub fn socket_path() -> PathBuf {
    if let Some(p) = env_compat(ENV_SOCKET) {
        return PathBuf::from(p);
    }
    #[cfg(windows)]
    {
        PathBuf::from(r"\\.\pipe\cys")
    }
    #[cfg(not(windows))]
    {
        let base = dirs::state_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("/tmp"));
        let dir = if base.ends_with(".local/state") || base.to_string_lossy().contains("state") {
            base.join("cys")
        } else {
            base.join(".local/state/cys")
        };
        dir.join("cys.sock")
    }
}

/// Windows named pipe busy-retry 정책 — CLI(cys)·GUI(cys-app) 클라이언트 공용 **단일 진실**
/// (이원 정의는 정책 변경 시 샷건 서저리). ERROR_PIPE_BUSY(os error 231, "모든 파이프
/// 인스턴스가 사용 중")는 데몬 다운이 아니라 listening 인스턴스 순간 소진(정상 혼잡)이다 —
/// 서버(cysd 리스너 풀)는 accept 직후 인스턴스를 재생성하므로 잠깐 기다리면 열린다.
/// Microsoft 파이프 클라이언트 계약상 busy 는 대기·재시도가 필수(WaitNamedPipe 관례)이며,
/// 재시도 없는 1회 open 은 멀티 노드 동시 RPC 에서 상시 실패한다(2026-07-10 Windows 실사고).
/// 그 외 오류(파이프 부재 = 데몬 다운 등)는 즉시 반환이 계약이다(autostart 판단은 호출부 몫).
/// 전 OS에서 컴파일되는 pub 상수라 비-Windows 테스트가 정책 불변을 박제할 수 있다.
pub const PIPE_BUSY_ERROR: i32 = 231;
pub const PIPE_BUSY_RETRY_INTERVAL: std::time::Duration = std::time::Duration::from_millis(25);
pub const PIPE_BUSY_RETRY_DEADLINE: std::time::Duration = std::time::Duration::from_secs(5);

/// 동봉 runtime PATH 선두 주입(RC-5 · 공용 — cysd PTY 자식·GUI 직스폰이 공유, 중복 구현 금지).
/// `exe_dir`(바이너리 폴더) + Windows 자기완결 설치의 `<install>\runtime\{python, git\cmd, git\usr\bin}`
/// 중 **실재하는** 디렉토리를 `current_path` 앞에 (중복 제거) 얹은 새 PATH를 반환. 얹을 게 없으면
/// None(기존 동작 무변경). current_path를 인자로 받아 순수 함수(테스트 가능·env 비의존).
/// 근거: GUI(Finder/Explorer) 기동 프로세스는 PATH가 빈곤해 bash/python3 lookup 실패(RC-5 ＋부서 무반응).
/// 동봉 runtime의 bin 디렉토리들(디스크에 실재하는 것만) — OS별 레이아웃. 반환 순서 = PATH 선두 우선순위.
/// Windows(RC-5): exe 형제 `runtime/`(python·git/cmd·git/usr/bin).
/// macOS(RC-18·T6b): 앱 번들은 실행바이너리=Contents/MacOS·리소스(runtime/)=Contents/Resources →
///   `exe_dir/../Resources/runtime`(python/bin·git/bin·uv·node/bin). 개발 빌드(exe 형제 runtime/)도 폴백.
/// runtime_prefixed_path(PATH 선두주입)와 state.rs `-lc` 재선두주입(D8 — 로그인셸 path_helper 강등 회피)이 공유.
pub fn runtime_bin_dirs(exe_dir: &Path) -> Vec<PathBuf> {
    #[cfg_attr(not(any(windows, target_os = "macos")), allow(unused_mut))]
    let mut dirs: Vec<PathBuf> = Vec::new();
    #[cfg(windows)]
    {
        let rt = exe_dir.join("runtime");
        for d in [
            rt.join("python"),
            rt.join("git").join("cmd"),
            rt.join("git").join("usr").join("bin"),
            rt.join("node"), // ★T6b 파리티: node.exe·npm·npx (mac runtime/node/bin 대칭 — win은 top-level)
        ] {
            if d.is_dir() {
                dirs.push(d);
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        // 앱 번들 리소스 경로 우선, 개발 빌드(형제 runtime/) 폴백. 첫 유효 루트만 사용.
        let roots = [
            exe_dir.parent().map(|p| p.join("Resources").join("runtime")),
            Some(exe_dir.join("runtime")),
        ];
        for rt in roots.into_iter().flatten() {
            if !rt.is_dir() {
                continue;
            }
            for d in [
                rt.join("python").join("bin"),
                rt.join("git").join("bin"),
                rt.join("uv"),
                rt.join("node").join("bin"),
            ] {
                if d.is_dir() {
                    dirs.push(d);
                }
            }
            break;
        }
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = exe_dir;
    }
    dirs
}

/// pane/자식 프로세스에 물릴 PATH 를 계산 — 결과가 현행과 다르면 Some(주입값), 무변경이면 None.
/// **이중 의미론**: unix = exe_dir + 번들 runtime bins 선두 주입 + `~/.local/bin` 말미 append(나머지 보존) —
/// 로그인 셸(-l) 프로파일이 PATH 를 복원하나 claude native 설치기가 rc 를 수정하지 않음이 실측 확인되어
/// `~/.local/bin` 은 belt-and-braces 로 직접 append 한다(compose_unix_pane_path 참조).
/// Windows = 레지스트리에서 신선 PATH 를 재합성한다. 데몬은 자기 기동 시점 PATH 스냅샷을 자식에 물리는데,
/// 실행 중엔 WM_SETTINGCHANGE 를 못 받아 레지스트리 PATH 변경(claude 사후 설치 등)이 반영되지 않는다 →
/// spawn 마다 HKLM/HKCU 에서 신선 PATH 를 읽어 재합성 = 새 PowerShell 창과 등가(compose_pane_path 참조).
pub fn runtime_prefixed_path(exe_dir: &Path, current_path: &str) -> Option<String> {
    let sep = if cfg!(windows) { ';' } else { ':' };
    let mut prefixes: Vec<String> = vec![exe_dir.to_string_lossy().into_owned()];
    for d in runtime_bin_dirs(exe_dir) {
        prefixes.push(d.to_string_lossy().into_owned());
    }
    #[cfg(windows)]
    {
        // ★Windows stale PATH: 데몬은 WM_SETTINGCHANGE 를 못 받아 레지스트리 PATH 변경(claude 사후 설치)이
        // 프로세스 PATH 스냅샷에 반영 안 됨 → spawn 마다 레지스트리에서 신선 PATH 재합성 = 새 PowerShell 등가.
        // 어떤 실패도 현행보다 나빠지지 않게(fail-open): fresh=None 이면 아래 compose 가 프로세스 PATH 폴백.
        let fresh = windows_registry_path();
        let home = home_dir();
        let appdata = std::env::var_os("APPDATA").map(PathBuf::from);
        let user_bins: Vec<String> = windows_user_bin_dirs(&home, appdata.as_deref())
            .into_iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        let composed = compose_pane_path(&prefixes, fresh.as_deref(), &user_bins, current_path, sep);
        // 기존 계약 유지: 결과가 현행과 같으면 None(무주입). 레지스트리 재합성이 보통 값을 바꾼다.
        if composed == current_path {
            return None;
        }
        return Some(composed);
    }
    #[cfg(not(windows))]
    {
        // unix: 로그인 셸(-l) 프로파일이 PATH 를 복원하지만, claude native 설치기(2.1.207)가 rc 를
        // 수정하지 않음이 실측 확인되어 ~/.local/bin 은 belt-and-braces 로 무조건 append 한다.
        let _ = sep; // unix 합성은 compose_unix_pane_path 가 ':' 고정 사용.
        compose_unix_pane_path(&prefixes, &home_dir(), current_path)
    }
}

/// unix pane PATH 합성(순수·테스트 가능 · **unix 전용 의미론** · OS 무관 컴파일). 순서:
/// `[prefixes: exe_dir + 번들 runtime bins] 중 신규분 선두` ; `current_path 전체(순서 그대로 보존)` ;
/// `~/.local/bin(claude native 설치 위치) 신규분 말미 append`. claude 설치기가 ~/.zshrc 를 수정하지
/// 않아(수동 안내만 출력·실측) 프로파일 복원만으론 claude 가 영원히 미발견 → windows_user_bin_dirs 의
/// belt-and-braces 를 unix 로 대칭 확장, is_dir 게이트 없이 무조건 append(셸은 없는 PATH 항목을 무시).
/// append 인 이유는 MAJ#1 과 동일: 발견이 목적 — 기존 항목의 precedence 를 강등하지 않는다.
/// dedup 은 unix 관례대로 case-sensitive 단순 비교. 결과가 현행과 같으면 None(무변경 계약 유지).
pub fn compose_unix_pane_path(prefixes: &[String], home: &Path, current_path: &str) -> Option<String> {
    let sep = ':';
    let add: Vec<&str> = prefixes
        .iter()
        .map(String::as_str)
        .filter(|p| !current_path.split(sep).any(|e| e == *p))
        .collect();
    let local_bin = home.join(".local").join("bin").to_string_lossy().into_owned();
    let append_local = !current_path.split(sep).any(|e| e == local_bin)
        && !add.iter().any(|p| *p == local_bin);
    if add.is_empty() && !append_local {
        return None;
    }
    let mut composed = String::new();
    if !add.is_empty() {
        composed.push_str(&add.join(&sep.to_string()));
        composed.push(sep);
    }
    composed.push_str(current_path);
    if append_local {
        composed.push(sep);
        composed.push_str(&local_bin);
    }
    Some(composed)
}

/// Windows `%VAR%` 전개(ExpandEnvironmentStrings 의미론 근사) — OS 무관 컴파일(순수·테스트 가능).
/// 짝을 이룬 `%..%` 만 치환하고, **미지 변수·미종결 %·빈 이름(%%)은 원문 그대로 보존**(안전 폴백:
/// 레지스트리 REG_EXPAND_SZ 의 %USERPROFILE% 등을 프로세스 env 로 전개하되 못 푼 건 깨뜨리지 않는다).
pub fn expand_windows_env(s: &str, lookup: impl Fn(&str) -> Option<String>) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '%' {
            if let Some(rel) = chars[i + 1..].iter().position(|&c| c == '%') {
                let close = i + 1 + rel;
                let name: String = chars[i + 1..close].iter().collect();
                if !name.is_empty() {
                    if let Some(val) = lookup(&name) {
                        out.push_str(&val); // %VAR% → 값
                    } else {
                        out.extend(chars[i..=close].iter()); // 미지 변수: %NAME% 원문 유지
                    }
                    i = close + 1;
                    continue;
                }
                // 빈 이름(%%): 첫 % 만 리터럴로 밀어 원문(%%)을 보존.
                out.push('%');
                i += 1;
                continue;
            }
            // 미종결 %: 리터럴.
            out.push('%');
            i += 1;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// Windows 사용자 bin 후보(belt-and-braces) — claude native 설치 위치 `%USERPROFILE%\.local\bin` 와
/// npm 전역 `%APPDATA%\npm`. 설치기가 레지스트리 등록을 빠뜨려도 잡히게 is_dir 게이트 없이 무조건 포함
/// (셸은 없는 PATH 항목을 무시·claude 설치 직후 재시작 없이 발견). OS 무관 컴파일(순수·테스트 가능).
pub fn windows_user_bin_dirs(home: &Path, appdata: Option<&Path>) -> Vec<PathBuf> {
    let mut v = vec![home.join(".local").join("bin")];
    if let Some(a) = appdata {
        v.push(a.join("npm"));
    }
    v
}

/// pane PATH 최종 합성(순서·dedup 규칙 · **Windows 전용 의미론** · OS 무관 컴파일). 순서:
/// `[prefixes: exe_dir + 번들 runtime bins]` ; `process_path 전체(현행 순서 그대로 보존)` ;
/// `fresh_base(신선 레지스트리 머신;유저) 중 신규분만 append` ; `user_bins 중 신규분만 append`.
/// ★MAJ#1(fail-open): 부모가 프로세스 PATH 선두에 의도 주입한 항목(pyenv/nvm shim 등, 레지스트리에 없는
/// 것)의 precedence 를 강등하지 않는다 — 버그 본질은 claude 디렉터리 '발견 가능'이지 '선두'가 아니라,
/// 레지스트리 신규분은 append 로 충분(프로세스 PATH 에 claude 가 없으니 append 로 발견됨). 레지스트리가
/// 새로 주는 게 없으면 composed==current 가 더 자주 성립 → None(무주입) 계약도 더 잘 보존된다.
/// 단, prefixes(동봉 runtime)는 항상 선두 — 레지스트리 유입 python 등에 절대 밀리지 않는다.
/// ★MAJ#2(dedup): Windows PATH 는 case-insensitive 이므로 비교 키를 **소문자 정규화 + 후행 경로구분자
/// (`\`·`/`) 트림**으로 만든다(출력은 원문=최초 등장 형태 유지). casing 변형·후행 슬래시 중복이 잔존해
/// env 블록 32767자 한계를 넘겨 CreateProcess 가 실패(pane 미기동)하는 꼬리위험을 막는다. 빈 항목 제거.
/// fresh_base=None 이면 base=프로세스 PATH(현행 폴백).
pub fn compose_pane_path(
    prefixes: &[String],
    fresh_base: Option<&str>,
    user_bins: &[String],
    process_path: &str,
    sep: char,
) -> String {
    // Windows 비교 키: 소문자 + 후행 '\'·'/' 트림. 출력엔 안 쓰고 dedup 판정에만.
    fn norm_key(s: &str) -> String {
        s.trim_end_matches(['\\', '/']).to_lowercase()
    }
    let mut items: Vec<&str> = Vec::new();
    for p in prefixes {
        items.push(p.as_str());
    }
    for e in process_path.split(sep) {
        items.push(e);
    }
    if let Some(fb) = fresh_base {
        for e in fb.split(sep) {
            items.push(e);
        }
    }
    for u in user_bins {
        items.push(u.as_str());
    }
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut out: Vec<&str> = Vec::new();
    for it in items {
        if it.is_empty() {
            continue;
        }
        if seen.insert(norm_key(it)) {
            out.push(it);
        }
    }
    out.join(&sep.to_string())
}

/// Windows 레지스트리에서 신선 PATH 재합성(머신;유저) — 실행 중 데몬이 못 받는 WM_SETTINGCHANGE 를 우회.
/// HKLM `...\Session Manager\Environment` + HKCU `Environment` 의 `Path` 를 관례대로 머신;유저 순 결합,
/// REG_EXPAND_SZ 의 %VAR% 는 프로세스 env 로 수동 전개. 성공 hive 만 사용·둘 다 실패면 None(fail-open).
#[cfg(windows)]
pub fn windows_registry_path() -> Option<String> {
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    let machine = read_registry_string(
        HKEY_LOCAL_MACHINE,
        r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment",
        "Path",
    );
    let user = read_registry_string(HKEY_CURRENT_USER, "Environment", "Path");
    // %VAR% 는 로그인 후 불변인 프로세스 env(USERPROFILE 등)로 전개 — pane 이 볼 값과 등가.
    let expand = |raw: String| expand_windows_env(&raw, |k| std::env::var(k).ok());
    match (machine, user) {
        (Some(m), Some(u)) => Some(format!("{};{}", expand(m), expand(u))),
        (Some(m), None) => Some(expand(m)),
        (None, Some(u)) => Some(expand(u)),
        (None, None) => None,
    }
}

/// 레지스트리 문자열 값 읽기(REG_SZ·REG_EXPAND_SZ, NOEXPAND 로 원문 취득 후 상위에서 수동 전개).
/// 2-pass: 1) 필요한 바이트 크기 질의 → 2) 버퍼 채움. 실패(값 부재 등)면 None(fail-open).
#[cfg(windows)]
fn read_registry_string(
    hkey: windows_sys::Win32::System::Registry::HKEY,
    subkey: &str,
    value: &str,
) -> Option<String> {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        RegGetValueW, RRF_NOEXPAND, RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ,
    };
    let sub: Vec<u16> = subkey.encode_utf16().chain(std::iter::once(0)).collect();
    let val: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
    let flags = RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ | RRF_NOEXPAND;
    // 1-pass: 크기 질의(데이터 포인터 NULL).
    let mut size: u32 = 0;
    let rc = unsafe {
        RegGetValueW(
            hkey,
            sub.as_ptr(),
            val.as_ptr(),
            flags,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut size,
        )
    };
    if rc != ERROR_SUCCESS || size == 0 {
        return None;
    }
    // 2-pass: u16 버퍼(바이트→워드, 널 여유 +1).
    let mut buf: Vec<u16> = vec![0u16; (size as usize / 2) + 1];
    let mut size2 = (buf.len() * 2) as u32;
    let rc2 = unsafe {
        RegGetValueW(
            hkey,
            sub.as_ptr(),
            val.as_ptr(),
            flags,
            std::ptr::null_mut(),
            buf.as_mut_ptr() as *mut core::ffi::c_void,
            &mut size2,
        )
    };
    if rc2 != ERROR_SUCCESS {
        return None;
    }
    let n = (size2 as usize / 2).min(buf.len());
    let slice = &buf[..n];
    let end = slice.iter().position(|&c| c == 0).unwrap_or(slice.len());
    let s = String::from_utf16_lossy(&slice[..end]);
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// 홈 디렉토리(RC-7 공용). Windows는 HOME 미설정이 기본이라 `env::var("HOME")`은 빈값으로 폴백돼
/// `~/.cys/...` 경로를 CWD 상대경로로 붕괴시킨다(부서목록·프로파일·pending-restore 오지정). dirs::home_dir()
/// (Windows=USERPROFILE/HOMEDRIVE 기반·unix=$HOME)로 OS중립 해소. 코어(cys)·GUI(src-tauri) 공유.
pub fn home_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

/// (W1) claude CLAUDE_CONFIG_DIR 결정론 해소 — agents.json의 `${CYS_ACCOUNT_DIR:-$HOME/.cys/claude}`와
/// 동일 규칙을 **현재 프로세스 env**로 전개한다. pane 셸(=데몬 자식)이 실제로 해소하는 값과 일치하려면
/// 실제 전개 주체인 **데몬 프로세스에서 호출**하는 것이 권위다(state.rs의 CYS_ACCOUNT_DIR 전파와 정합).
/// discover 스캔(usage.rs)이 ~/.cys/claude를 원리적으로 못 보므로, config_dir 권위는 이 결정론 해소뿐이다.
pub fn resolve_claude_config_dir() -> String {
    std::env::var("CYS_ACCOUNT_DIR")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| home_dir().join(".cys").join("claude").to_string_lossy().into_owned())
}

/// Claude Code projects/ 디렉터리명 munge — 실측: '/'와 특수문자가 '-'로 치환된다.
/// ASCII 영숫자·'-'만 보존하는 보수 구현. resume 사전검증 게이트(cys.rs)와 usage 휴리스틱이 공유한다.
pub fn claude_project_component(cwd: &str) -> String {
    cwd.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

/// 부서 데몬 소켓/파이프 경로(RC-4 · 공용 — GUI(src-tauri)·cys fleet가 공유, 규약 단일화).
/// Windows: named pipe `\\.\pipe\cys-dept-<name>`(기본 데몬 `\\.\pipe\cys`와 대칭 · RC-13 state_dir
/// 슬러그 `cys-dept-<name>`과 정합). unix: `~/.local/state/cys-dept-<name>/cys.sock`(cys-dept 규약).
/// HOME 미설정 함정(RC-7) 회피 — unix도 dirs::home_dir() 사용.
pub fn dept_socket_path(name: &str) -> PathBuf {
    #[cfg(windows)]
    {
        PathBuf::from(format!(r"\\.\pipe\cys-dept-{name}"))
    }
    #[cfg(not(windows))]
    {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".local/state")
            .join(format!("cys-dept-{name}"))
            .join("cys.sock")
    }
}

/// 이 소켓/파이프 경로가 부서(dept) 데몬의 것인가 — 부서 규약 `cys-dept-<name>`(dept_socket_path와 정합).
/// 채널은 메인 cysd 단독 소유(DESIGN §2.5)이므로 부서 데몬의 브리지 스폰을 구조적으로 거부하는 데 쓴다.
/// 판별: 경로 컴포넌트(unix 부모 디렉토리 `cys-dept-<name>` / windows 파이프명 `cys-dept-<name>`) 중
/// `cys-dept-` 접두 이름이 있으면 부서. 메인 데몬(슬러그 `cys`·`cys.sock`)은 오판하지 않는다
/// (`cys`는 `cys-dept-` 접두가 아님 — 오탐 시 채널 전면 불능이라 접두를 정확히 요구).
pub fn is_dept_socket(socket_path: &std::path::Path) -> bool {
    socket_path
        .to_string_lossy()
        .split(|c| c == '/' || c == '\\')
        .any(|comp| comp.starts_with("cys-dept-"))
}

/// Parse a surface reference: "surface:31", "31", or 31 → 31.
pub fn parse_surface_ref(s: &str) -> Option<u64> {
    let t = s.trim();
    let t = t.strip_prefix("surface:").unwrap_or(t);
    t.parse::<u64>().ok()
}

pub fn surface_ref(id: u64) -> String {
    format!("surface:{id}")
}

/// Map a named key name to the byte sequence
/// written to the PTY. Supports C- (ctrl), M- (alt/meta) prefixes.
pub fn key_to_bytes(key: &str) -> Option<Vec<u8>> {
    // Modifier prefixes
    if let Some(rest) = key.strip_prefix("C-") {
        // 단일 문자일 때만 ctrl 비트 변환 — "C-Space"의 'S'가 0x13(XOFF, 출력 동결)으로
        // 잘못 변환되어 Space 분기가 사문화되는 것을 차단
        if rest.chars().count() == 1 {
            let c = rest.chars().next()?;
            let lower = c.to_ascii_lowercase();
            if lower.is_ascii_lowercase() {
                return Some(vec![(lower as u8) & 0x1f]);
            }
        }
        return match rest {
            "Space" | "space" => Some(vec![0x00]),
            _ => None,
        };
    }
    if let Some(rest) = key.strip_prefix("M-") {
        let mut b = vec![0x1b];
        b.extend_from_slice(rest.as_bytes());
        return Some(b);
    }
    let seq: &[u8] = match key {
        "Return" | "Enter" => b"\r",
        "Tab" => b"\t",
        "BTab" | "BackTab" => b"\x1b[Z",
        "Space" => b" ",
        "Escape" | "Esc" => b"\x1b",
        "Backspace" => b"\x7f",
        "Delete" | "DC" => b"\x1b[3~",
        "Up" => b"\x1b[A",
        "Down" => b"\x1b[B",
        "Right" => b"\x1b[C",
        "Left" => b"\x1b[D",
        "Home" => b"\x1b[H",
        "End" => b"\x1b[F",
        "PageUp" | "PPage" => b"\x1b[5~",
        "PageDown" | "NPage" => b"\x1b[6~",
        "F1" => b"\x1bOP",
        "F2" => b"\x1bOQ",
        "F3" => b"\x1bOR",
        "F4" => b"\x1bOS",
        "F5" => b"\x1b[15~",
        "F6" => b"\x1b[17~",
        "F7" => b"\x1b[18~",
        "F8" => b"\x1b[19~",
        "F9" => b"\x1b[20~",
        "F10" => b"\x1b[21~",
        "F11" => b"\x1b[23~",
        "F12" => b"\x1b[24~",
        _ => {
            // Single literal character passes through
            if key.chars().count() == 1 {
                return Some(key.as_bytes().to_vec());
            }
            return None;
        }
    };
    Some(seq.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surface_refs() {
        assert_eq!(parse_surface_ref("surface:31"), Some(31));
        assert_eq!(parse_surface_ref("31"), Some(31));
        assert_eq!(parse_surface_ref("x"), None);
    }

    #[test]
    fn dept_socket_path_os_convention() {
        // RC-4 회귀 핀: OS별 부서 소켓 규약. 기본 socket_path와 대칭(둘 다 windows=named pipe).
        let p = dept_socket_path("dept-3");
        let s = p.to_string_lossy();
        #[cfg(windows)]
        assert_eq!(s, r"\\.\pipe\cys-dept-dept-3", "windows named pipe");
        #[cfg(not(windows))]
        {
            assert!(s.ends_with(".local/state/cys-dept-dept-3/cys.sock"), "unix .sock: {s}");
            assert!(!s.starts_with('/') || s.contains("/.local/state/"), "home 기반: {s}");
        }
    }

    #[test]
    fn is_dept_socket_detects_dept_not_main() {
        // H3: dept_socket_path와 정합 — 부서만 true, 메인은 false(오판=채널 전면 불능이라 접두 정확).
        assert!(is_dept_socket(&dept_socket_path("dept-3")), "부서 소켓은 true");
        assert!(is_dept_socket(Path::new("/x/.local/state/cys-dept-future/cys.sock")));
        assert!(is_dept_socket(Path::new(r"\\.\pipe\cys-dept-3")), "windows 파이프");
        // 메인 데몬 — 오판 금지.
        assert!(!is_dept_socket(Path::new("/x/.local/state/cys/cys.sock")), "메인 unix");
        assert!(!is_dept_socket(Path::new(r"\\.\pipe\cys")), "메인 windows");
        assert!(!is_dept_socket(Path::new("/tmp/cys_chan_test_1_tag/cysd.sock")), "테스트 임시");
    }

    #[test]
    fn runtime_prefixed_path_prepends_exe_dir_and_dedups() {
        // RC-5 회귀 핀(양 OS 공통 로직): exe_dir가 PATH에 없으면 선두에 얹는다.
        let sep = if cfg!(windows) { ';' } else { ':' };
        let exe = Path::new("/opt/cysapp/bin");
        let cur = format!("/usr/bin{sep}/bin");
        let got = runtime_prefixed_path(exe, &cur).expect("exe_dir 미포함이면 Some");
        assert!(got.starts_with("/opt/cysapp/bin"), "exe_dir 선두 주입: {got}");
        #[cfg(windows)]
        {
            assert!(got.ends_with(&cur), "기존 PATH 보존(제거 없음): {got}");
            // 이미 PATH에 있으면(중복) 얹지 않는다 → None(무변경). (windows는 runtime 하위 dir가
            // 실재하면 Some일 수 있으나 이 합성 경로엔 없음.)
            let already = format!("/opt/cysapp/bin{sep}/usr/bin");
            assert_eq!(runtime_prefixed_path(exe, &already), None, "중복이면 무변경");
        }
        #[cfg(not(windows))]
        {
            // 갱신 사유: unix 브랜치가 ~/.local/bin 을 무조건 append 하게 되어(claude 설치기 rc
            // 무수정 실측 대응) 합성 결과가 `…:cur:~/.local/bin` — ends_with(&cur) 단정을 교체.
            let local = home_dir().join(".local").join("bin").to_string_lossy().into_owned();
            assert!(
                got.ends_with(&format!("{cur}{sep}{local}")),
                "기존 PATH 보존 + ~/.local/bin 말미 append: {got}"
            );
            // prefixes 도 ~/.local/bin 도 이미 있으면 → None(무변경).
            let already = format!("/opt/cysapp/bin{sep}/usr/bin{sep}{local}");
            assert_eq!(runtime_prefixed_path(exe, &already), None, "중복이면 무변경");
        }
    }

    #[test]
    fn compose_unix_pane_path_appends_local_bin() {
        // Mac 핫픽스 회귀 핀: claude native 설치기(2.1.207)가 rc 무수정임이 실측 확인 →
        // ~/.local/bin 을 is_dir 게이트 없이 무조건 말미 append(발견 목적·기존 precedence 불강등).
        let home = Path::new("/home/fixture-user");
        let local = home.join(".local").join("bin").to_string_lossy().into_owned();
        let prefixes = vec!["/opt/app/bin".to_string()];
        // ① current_path 에 없으면 맨 뒤에 append — prefixes 는 선두.
        let out = compose_unix_pane_path(&prefixes, home, "/usr/bin:/bin").expect("Some");
        assert_eq!(out, format!("/opt/app/bin:/usr/bin:/bin:{local}"));
        // ② 이미 있으면 중복 append 하지 않는다(부정 케이스).
        let cur2 = format!("{local}:/usr/bin");
        let out2 = compose_unix_pane_path(&prefixes, home, &cur2).expect("Some");
        assert_eq!(out2, format!("/opt/app/bin:{cur2}"));
        // ③ prefixes·~/.local/bin 모두 기존재 → None(무변경 계약).
        let cur3 = format!("/opt/app/bin:/usr/bin:{local}");
        assert_eq!(compose_unix_pane_path(&prefixes, home, &cur3), None);
        // ④ append 가 기존 항목 순서를 바꾸지 않는다(선두 의도 주입 강등 금지 — MAJ#1 대칭).
        let out4 = compose_unix_pane_path(&prefixes, home, "/custom/shim:/usr/bin:/bin").expect("Some");
        assert_eq!(out4, format!("/opt/app/bin:/custom/shim:/usr/bin:/bin:{local}"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn runtime_bin_dirs_macos_resolves_bundle_resources_layout() {
        // RC-18(T6b) 회귀 핀: mac 번들 레이아웃(Contents/MacOS·Contents/Resources/runtime)에서
        // python/bin·git/bin·uv·node/bin을 선두 우선순위로 잡는다. 실재 디렉토리만 계상.
        use std::fs;
        let base = std::env::temp_dir().join(format!("cysrt-t6b-{}", std::process::id()));
        let macos = base.join("Contents").join("MacOS");
        let rt = base.join("Contents").join("Resources").join("runtime");
        for d in ["python/bin", "git/bin", "uv", "node/bin"] {
            fs::create_dir_all(rt.join(d)).unwrap();
        }
        fs::create_dir_all(&macos).unwrap();
        let dirs = runtime_bin_dirs(&macos);
        let got: Vec<String> = dirs.iter().map(|p| p.to_string_lossy().into_owned()).collect();
        assert_eq!(got.len(), 4, "4개 runtime bin dir: {got:?}");
        assert!(got[0].ends_with("Resources/runtime/python/bin"), "python 선두: {got:?}");
        assert!(got[1].ends_with("Resources/runtime/git/bin"), "git 2순위: {got:?}");
        assert!(got[2].ends_with("Resources/runtime/uv"), "uv 3순위: {got:?}");
        assert!(got[3].ends_with("Resources/runtime/node/bin"), "node 4순위: {got:?}");
        // PATH 선두주입: runtime dir들이 exe_dir 뒤·기존 PATH 앞.
        let p = runtime_prefixed_path(&macos, "/usr/bin:/bin").expect("Some");
        let py_idx = p.find("Resources/runtime/python/bin").unwrap();
        let usrbin_idx = p.find("/usr/bin").unwrap();
        assert!(py_idx < usrbin_idx, "runtime python이 /usr/bin보다 앞(env 레벨): {p}");
        // 부재 dir는 계상 안 함: uv 제거 후 3개.
        fs::remove_dir_all(rt.join("uv")).unwrap();
        assert_eq!(runtime_bin_dirs(&macos).len(), 3, "uv 부재 시 3개");
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn expand_windows_env_cases() {
        // %VAR% 전개(mac 에서 Windows 로직 검증 — 순수 fn 직접 호출).
        let lk = |k: &str| match k {
            "USERPROFILE" => Some(r"C:\Users\cys".to_string()),
            "APPDATA" => Some(r"C:\Users\cys\AppData\Roaming".to_string()),
            _ => None,
        };
        // 전개.
        assert_eq!(
            expand_windows_env(r"%USERPROFILE%\.local\bin", lk),
            r"C:\Users\cys\.local\bin"
        );
        // 미지 변수는 원문 유지.
        assert_eq!(expand_windows_env(r"%NOPE%\bin", lk), r"%NOPE%\bin");
        // 변수 없음: 그대로.
        assert_eq!(expand_windows_env(r"C:\Windows\System32", lk), r"C:\Windows\System32");
        // 연속 변수(경계 인접).
        assert_eq!(
            expand_windows_env(r"%USERPROFILE%%APPDATA%", lk),
            r"C:\Users\cysC:\Users\cys\AppData\Roaming"
        );
        // 빈 이름(%%)·미종결 %는 원문 보존(안전 폴백).
        assert_eq!(expand_windows_env(r"50%%off", lk), r"50%%off");
        assert_eq!(expand_windows_env(r"tail%USERPROFILE", lk), r"tail%USERPROFILE");
        // 빈 문자열.
        assert_eq!(expand_windows_env("", lk), "");
    }

    #[test]
    fn windows_user_bin_dirs_composition() {
        let home = Path::new(r"C:\Users\cys");
        let appdata = PathBuf::from(r"C:\Users\cys\AppData\Roaming");
        let with = windows_user_bin_dirs(home, Some(&appdata));
        let got: Vec<String> = with.iter().map(|p| p.to_string_lossy().into_owned()).collect();
        // 경로 구분자는 호스트 OS 규약이라 컴포넌트 존재로 검증(mac 에서도 무결).
        assert_eq!(got.len(), 2);
        assert!(got[0].contains(".local") && got[0].ends_with("bin"), "local/bin: {got:?}");
        assert!(got[1].ends_with("npm"), "appdata/npm: {got:?}");
        // APPDATA 부재 시 .local/bin 만.
        let none = windows_user_bin_dirs(home, None);
        assert_eq!(none.len(), 1);
    }

    #[test]
    fn compose_pane_path_rules() {
        let sep = ';';
        let prefixes = vec![r"C:\app\bin".to_string(), r"C:\app\runtime\python".to_string()];
        let user_bins = vec![r"C:\Users\cys\.local\bin".to_string(), r"C:\Users\cys\AppData\Roaming\npm".to_string()];
        // ① 낡은 프로세스 PATH(.local\bin 없음) + 신선 레지스트리(.local\bin 있음) →
        //    새 순서: prefixes ; process ; fresh 신규분 ; user_bins 신규분. .local\bin 정확히 1회.
        let fresh = r"C:\Windows\System32;C:\Users\cys\.local\bin";
        let process = r"C:\Windows\System32;C:\stale";
        let out = compose_pane_path(&prefixes, Some(fresh), &user_bins, process, sep);
        let parts: Vec<&str> = out.split(sep).collect();
        assert_eq!(parts[0], r"C:\app\bin", "prefix 선두: {out}");
        assert_eq!(parts[1], r"C:\app\runtime\python", "runtime 2순위: {out}");
        let local_count = parts.iter().filter(|&&p| p == r"C:\Users\cys\.local\bin").count();
        assert_eq!(local_count, 1, "user bin 정확히 1회(레지스트리·user_bins 중복 dedup): {out}");
        assert!(parts.contains(&r"C:\stale"), "세션 유래 항목 보존: {out}");
        // 세션 유래(process) 항목이 fresh 신규분(.local\bin)보다 앞 — precedence 보존(MAJ#1).
        let stale_idx = parts.iter().position(|&p| p == r"C:\stale").unwrap();
        let local_idx = parts.iter().position(|&p| p == r"C:\Users\cys\.local\bin").unwrap();
        assert!(stale_idx < local_idx, "process 항목이 레지스트리 신규분보다 앞: {out}");
        // System32 도 dedup(레지스트리·프로세스 중복) — 1회.
        assert_eq!(parts.iter().filter(|&&p| p == r"C:\Windows\System32").count(), 1, "전체 dedup: {out}");

        // ② 레지스트리 None → base=프로세스 PATH(현행 동작 등가). prefix 후 프로세스 항목 보존.
        let out2 = compose_pane_path(&prefixes, None, &user_bins, process, sep);
        let parts2: Vec<&str> = out2.split(sep).collect();
        assert_eq!(parts2[0], r"C:\app\bin");
        assert!(parts2.contains(&r"C:\stale") && parts2.contains(&r"C:\Windows\System32"));
        // user_bins 는 여전히 포함(fresh 에 없어도 belt-and-braces).
        assert!(parts2.contains(&r"C:\Users\cys\.local\bin"));

        // ④ 빈 항목 제거·중복 전면 dedup.
        let out3 = compose_pane_path(&prefixes, Some(";;C:\\dup"), &[], "C:\\dup;C:\\app\\bin;", sep);
        let parts3: Vec<&str> = out3.split(sep).collect();
        assert!(!parts3.iter().any(|p| p.is_empty()), "빈 항목 없음: {out3}");
        assert_eq!(parts3.iter().filter(|&&p| p == r"C:\dup").count(), 1, "dup 1회: {out3}");
        assert_eq!(parts3.iter().filter(|&&p| p == r"C:\app\bin").count(), 1, "prefix dup 1회: {out3}");

        // (a) MAJ#1 핀: 프로세스 PATH 선두의 shim(레지스트리에 없음)이 prefixes 바로 다음·레지스트리 항목보다 앞.
        let out_a = compose_pane_path(&prefixes, Some(r"C:\reg\bin"), &[], r"C:\shim\bin;C:\other", sep);
        let pa: Vec<&str> = out_a.split(sep).collect();
        assert_eq!(pa[0], r"C:\app\bin");
        assert_eq!(pa[1], r"C:\app\runtime\python");
        assert_eq!(pa[2], r"C:\shim\bin", "process 선두 shim 이 prefixes 직후: {out_a}");
        let shim_idx = pa.iter().position(|&p| p == r"C:\shim\bin").unwrap();
        let reg_idx = pa.iter().position(|&p| p == r"C:\reg\bin").unwrap();
        assert!(shim_idx < reg_idx, "shim 이 레지스트리 항목보다 앞: {out_a}");

        // (b) MAJ#2 핀: casing·후행 '\' 변형은 동일 항목으로 dedup(Windows case-insensitive) — 1회만.
        let out_b = compose_pane_path(&[], Some(r"C:\WINDOWS\System32"), &[], r"C:\Windows\System32\", sep);
        let pb: Vec<&str> = out_b.split(sep).collect();
        assert_eq!(pb.len(), 1, "casing·후행슬래시 변형 dedup=1개: {out_b}");
        assert_eq!(pb[0], r"C:\Windows\System32\", "출력은 원문(최초 등장=process 형태) 유지: {out_b}");

        // (c) ★순서보장 핀(박사님 통찰): fresh_base 에 경쟁 python(C:\Python312)이 있어도 동봉 runtime 이 항상 앞.
        let out_c = compose_pane_path(&prefixes, Some(r"C:\Python312"), &[], "", sep);
        let pc: Vec<&str> = out_c.split(sep).collect();
        let rt_idx = pc.iter().position(|&p| p == r"C:\app\runtime\python").unwrap();
        let py_idx = pc.iter().position(|&p| p == r"C:\Python312").unwrap();
        assert!(rt_idx < py_idx, "동봉 runtime 이 레지스트리 유입 python 보다 앞(절대 밀리지 않음): {out_c}");
    }

    #[test]
    fn keys() {
        assert_eq!(key_to_bytes("Return"), Some(b"\r".to_vec()));
        assert_eq!(key_to_bytes("C-c"), Some(vec![0x03]));
        assert_eq!(key_to_bytes("Up"), Some(b"\x1b[A".to_vec()));
    }

    #[test]
    fn surface_ref_roundtrip_and_edges() {
        // 왕복: id → surface_ref → parse_surface_ref → id
        for id in [0u64, 1, 31, 65535, u64::MAX] {
            assert_eq!(parse_surface_ref(&surface_ref(id)), Some(id));
        }
        // 공백 trim
        assert_eq!(parse_surface_ref("  42  "), Some(42));
        assert_eq!(parse_surface_ref("\tsurface:7\n"), Some(7));
        // prefix는 1회만 제거 — 이중 prefix는 parse 실패
        assert_eq!(parse_surface_ref("surface:surface:31"), None);
        // 음수·비숫자·빈 문자열
        assert_eq!(parse_surface_ref("-5"), None);
        assert_eq!(parse_surface_ref(""), None);
        assert_eq!(parse_surface_ref("surface:"), None);
        assert_eq!(parse_surface_ref("3.5"), None);
        // u64 초과는 None (오버플로 시 silent wrap 금지)
        assert_eq!(parse_surface_ref("18446744073709551616"), None);
    }

    #[test]
    fn key_ctrl_modifier() {
        // C-c == C-C (대소문자 무관, ctrl 비트 0x1f 마스크)
        assert_eq!(key_to_bytes("C-c"), Some(vec![0x03]));
        assert_eq!(key_to_bytes("C-C"), Some(vec![0x03]));
        assert_eq!(key_to_bytes("C-a"), Some(vec![0x01]));
        assert_eq!(key_to_bytes("C-z"), Some(vec![0x1a]));
        // C-Space → NUL (0x00), 'S'가 0x13(XOFF)으로 오변환되지 않음
        assert_eq!(key_to_bytes("C-Space"), Some(vec![0x00]));
        assert_eq!(key_to_bytes("C-space"), Some(vec![0x00]));
        // ctrl + 비-알파벳 단일문자는 매핑 없음
        assert_eq!(key_to_bytes("C-1"), None);
        assert_eq!(key_to_bytes("C-["), None);
        // 다중문자 C- (Space 외)는 ctrl 비트 변환 금지 → None
        assert_eq!(key_to_bytes("C-Foo"), None);
        // C- + 비-ASCII 단일문자(멀티바이트)는 ctrl 매핑 없음 → None
        // (count==1이라 단일문자 분기에 들지만 is_ascii_lowercase=false라 fall-through)
        assert_eq!(key_to_bytes("C-가"), None);
        // C- 단독(빈 rest)은 단일문자도 Space도 아님 → None
        assert_eq!(key_to_bytes("C-"), None);
    }

    #[test]
    fn key_meta_modifier() {
        // M-x → ESC + 'x'
        assert_eq!(key_to_bytes("M-x"), Some(vec![0x1b, b'x']));
        // M-<여러글자>도 ESC 접두 후 그대로 (Alt 시퀀스)
        assert_eq!(
            key_to_bytes("M-Foo"),
            Some([&[0x1b][..], b"Foo"].concat())
        );
        // M- 단독 (빈 rest) → ESC 단독
        assert_eq!(key_to_bytes("M-"), Some(vec![0x1b]));
    }

    #[test]
    fn key_named_and_literal() {
        assert_eq!(key_to_bytes("Enter"), Some(b"\r".to_vec()));
        assert_eq!(key_to_bytes("Tab"), Some(b"\t".to_vec()));
        assert_eq!(key_to_bytes("Escape"), Some(b"\x1b".to_vec()));
        assert_eq!(key_to_bytes("Backspace"), Some(b"\x7f".to_vec()));
        assert_eq!(key_to_bytes("F5"), Some(b"\x1b[15~".to_vec()));
        // 단일 리터럴 문자는 그대로 통과 (멀티바이트 포함)
        assert_eq!(key_to_bytes("a"), Some(b"a".to_vec()));
        assert_eq!(key_to_bytes("가"), Some("가".as_bytes().to_vec()));
        // 알 수 없는 다중문자 키 이름 → None
        assert_eq!(key_to_bytes("Nonsense"), None);
        assert_eq!(key_to_bytes(""), None);
    }

    #[test]
    fn key_function_keys_use_correct_protocol() {
        // F1-F4는 SS3(\x1bO_), F5+는 CSI(\x1b[_~) — 두 인코딩이 갈리는 경계 박제.
        assert_eq!(key_to_bytes("F1"), Some(b"\x1bOP".to_vec()));
        assert_eq!(key_to_bytes("F4"), Some(b"\x1bOS".to_vec()));
        assert_eq!(key_to_bytes("F5"), Some(b"\x1b[15~".to_vec()));
        assert_eq!(key_to_bytes("F12"), Some(b"\x1b[24~".to_vec()));
        // F5와 F6 사이에 16이 건너뛰는 VT 표준(역사적 결번) 보존
        assert_eq!(key_to_bytes("F6"), Some(b"\x1b[17~".to_vec()));
        // 대소문자 민감 — 'f1'은 명명키 아님, 단일문자도 아님(2글자) → None
        assert_eq!(key_to_bytes("f1"), None);
    }

    #[test]
    fn key_navigation_and_aliases() {
        // 화살표(CSI 종결바이트 A-D)
        assert_eq!(key_to_bytes("Up"), Some(b"\x1b[A".to_vec()));
        assert_eq!(key_to_bytes("Down"), Some(b"\x1b[B".to_vec()));
        assert_eq!(key_to_bytes("Right"), Some(b"\x1b[C".to_vec()));
        assert_eq!(key_to_bytes("Left"), Some(b"\x1b[D".to_vec()));
        // 별칭 동치 (Return=Enter 등 호환 어휘)
        assert_eq!(key_to_bytes("Return"), key_to_bytes("Enter"));
        assert_eq!(key_to_bytes("Esc"), key_to_bytes("Escape"));
        assert_eq!(key_to_bytes("BTab"), key_to_bytes("BackTab"));
        assert_eq!(key_to_bytes("Delete"), key_to_bytes("DC"));
        assert_eq!(key_to_bytes("PageUp"), key_to_bytes("PPage"));
        assert_eq!(key_to_bytes("PageDown"), key_to_bytes("NPage"));
        // BTab은 CSI Z (shift-tab)
        assert_eq!(key_to_bytes("BTab"), Some(b"\x1b[Z".to_vec()));
    }

    #[test]
    fn key_meta_with_named_key_is_literal_not_translated() {
        // ★불변식 박제: M- 접두는 rest를 명명키로 재해석하지 않고 '리터럴 바이트'로 붙인다.
        // 즉 M-Enter는 ESC+CR(\x1b\r)이 아니라 ESC + "Enter"(\x1b + 5글자)다.
        // (이 동작에 의존하는 호출부가 있으면 회귀 시 여기서 드러난다)
        assert_eq!(key_to_bytes("M-Enter"), Some([&[0x1b][..], b"Enter"].concat()));
        assert_ne!(key_to_bytes("M-Enter"), Some(vec![0x1b, b'\r']));
        // M-멀티바이트도 UTF-8 바이트 그대로 ESC 뒤에 (Alt+한글)
        assert_eq!(
            key_to_bytes("M-가"),
            Some([&[0x1b][..], "가".as_bytes()].concat())
        );
    }

    #[test]
    fn env_compat_fallback_priority() {
        // 고유 키로 격리 (다른 테스트·환경과 충돌 방지)
        let p = "CYS_ZZUNIQUETEST";
        let j = "JAVIS_ZZUNIQUETEST";
        let a = "AITERM_ZZUNIQUETEST";
        for k in [p, j, a] {
            std::env::remove_var(k);
        }
        // 셋 다 없으면 None
        assert_eq!(env_compat(p), None);
        // AITERM_만 있으면 폴백
        std::env::set_var(a, "aiterm_val");
        assert_eq!(env_compat(p), Some("aiterm_val".to_string()));
        // JAVIS_가 AITERM_보다 우선
        std::env::set_var(j, "javis_val");
        assert_eq!(env_compat(p), Some("javis_val".to_string()));
        // CYS_(primary)가 최우선
        std::env::set_var(p, "cys_val");
        assert_eq!(env_compat(p), Some("cys_val".to_string()));
        // 빈 문자열은 미설정으로 간주 → 다음 폴백
        std::env::set_var(p, "");
        assert_eq!(env_compat(p), Some("javis_val".to_string()));
        for k in [p, j, a] {
            std::env::remove_var(k);
        }
    }

    #[test]
    fn env_compat_only_first_cys_token_is_rewritten() {
        // replacen(..,1)이 'CYS_'를 첫 1회만 치환 — primary에 CYS_가 없으면
        // 세 후보 키가 모두 primary와 동일(폴백 무의미)임을 박제.
        let only = "CYS_ZZONLYPRIMARY";
        let javis = "JAVIS_ZZONLYPRIMARY";
        std::env::remove_var(only);
        std::env::remove_var(javis);
        // primary에 CYS_가 없는 키: 폴백 키가 자기 자신과 같아져 primary만 본다
        let nocys = "PLAINKEY_ZZ";
        std::env::remove_var(nocys);
        assert_eq!(env_compat(nocys), None);
        std::env::set_var(nocys, "plain");
        assert_eq!(env_compat(nocys), Some("plain".to_string()));
        std::env::remove_var(nocys);
        // 첫 CYS_만 치환 — 'CYS_'가 값 중간에 또 나와도 1회만
        std::env::set_var(javis, "via_javis");
        assert_eq!(env_compat(only), Some("via_javis".to_string()));
        std::env::remove_var(javis);
    }

    #[test]
    fn claude_project_component_munges_path() {
        // 실측 munge 규칙: '/'와 특수문자 → '-', 영숫자·'-' 보존.
        assert_eq!(
            claude_project_component("/Users/user/Desktop/ProjX"),
            "-Users-user-Desktop-ProjX"
        );
        assert_eq!(claude_project_component("/tmp/a.b_c"), "-tmp-a-b-c");
    }

    #[test]
    fn resolve_claude_config_dir_is_deterministic_env_not_scan() {
        // (W1-2 핵심) config_dir 권위는 결정론 env 해소뿐 — discover 스캔(~/.claude*)을 원리적으로
        // 참조하지 않는다. CYS_ACCOUNT_DIR 설정 시 그 값 그대로, 미설정 시 $HOME/.cys/claude.
        let prev = std::env::var("CYS_ACCOUNT_DIR").ok();
        // (a) 명시 계정 dir → 그 절대경로 그대로 (foreign ~/.claude-* 존재 여부와 무관 = 스캔 안 함)
        std::env::set_var("CYS_ACCOUNT_DIR", "/tmp/zz-acct/.cys/claude");
        assert_eq!(resolve_claude_config_dir(), "/tmp/zz-acct/.cys/claude");
        // (b) 빈 문자열 = 미설정 취급 → 기본 $HOME/.cys/claude
        std::env::set_var("CYS_ACCOUNT_DIR", "");
        let def = resolve_claude_config_dir();
        assert!(def.ends_with("/.cys/claude"), "기본 해소: {def}");
        assert!(
            def.starts_with(&home_dir().to_string_lossy().into_owned()),
            "HOME 기반: {def}"
        );
        // (c) 미설정도 동일 기본
        std::env::remove_var("CYS_ACCOUNT_DIR");
        assert_eq!(resolve_claude_config_dir(), def);
        // 원복
        match prev {
            Some(v) => std::env::set_var("CYS_ACCOUNT_DIR", v),
            None => std::env::remove_var("CYS_ACCOUNT_DIR"),
        }
    }
}
