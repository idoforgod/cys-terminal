//! agy(Antigravity CLI) 상태줄 **자동 연결** — 0.14.42 · 오너 승인 2026-09-24 ('자동 연결').
//!
//! ## 무엇을 하나
//!
//! agy 1.2 부터 쿼터 조회가 CSRF 토큰을 요구해 cys 는 agy 쿼터를 **agy 공식 상태줄(statusLine) 기능**으로만
//! 받는다(usage.rs 머리 주석 · IMPL-values §1). 그 연결은 `~/.gemini/antigravity-cli/settings.json` 의
//! `statusLine` 한 칸이고, 종전에는 사람이 직접 넣어야 했다(매뉴얼 절차). 이 모듈은 설치·팩 병합 때 그 칸이
//! **비어 있거나 없을 때만** cys 연결을 넣는다.
//!
//! ## 계약 (실패 방향 = 언제나 '쓰지 않음')
//!
//! - 사용자가 넣은 statusLine 이 있으면 **덮지 않는다**(`Outcome::UserOwned` — 로그·doctor 안내만).
//! - 이미 cys 연결(표지 유무 무관)이면 무동작(멱등). agy 안에서 `/statusline off` 로 꺼 둔 것도 그대로 둔다.
//! - 한 번 연결했던 칸이 나중에 비어 있으면 **다시 넣지 않는다**(`PreviouslyLinked`) — agy 의 `/statusline delete`
//!   가 남기는 모양(`{"type":"","command":"","enabled":false}`)은 agy 기본 직렬화와 바이트까지 같아 '사용자가 지웠다'와
//!   '처음부터 비었다'를 파일로는 구별할 수 없다(2026-09-24 이 맥 실측: 기본 파일이 바로 그 모양). 그래서 cys 쪽에
//!   '연결한 적 있음' 기록을 두고, 다시 연결은 사람이 부른 `cys doctor --fix`(force)로만 한다.
//! - 쓰기 전 백업(`settings.json.bak-cys`) · 원자 쓰기(임시 파일 → rename · 원 권한 유지) · 쓰기 직전 **재판독 대조**
//!   (그 사이 agy 가 파일을 바꿨으면 쓰지 않는다) · 쓴 뒤 **되읽기 확인**.
//! - JSON 파싱 실패 · UTF-8 아님 · 루트가 객체 아님 · 같은 키 중복 · 심볼릭 링크 · 쓰기 권한 없음(읽기 전용 · 잠김) ·
//!   1 MiB 초과 → **쓰지 않는다**(`Refused` — 사유를 로그·doctor 로). agy 를 깨는 방향의 쓰기는 없다.
//! - 파일 편집은 **텍스트 외과 수술**이다: statusLine 칸 하나만 넣거나 바꾸고 나머지 바이트(키 순서·들여쓰기·줄바꿈·
//!   BOM)는 그대로 둔다. serde_json 재직렬화는 쓰지 않는다(이 크레이트의 serde_json 은 preserve_order 가 없어 키
//!   순서를 사전순으로 바꾼다). 결과는 다시 파싱해 '다른 키 값 전부 동일 ∧ statusLine 이 원하는 값'일 때만 쓴다.
//!
//! ## 연결 명령과 표지
//!
//! 유닉스: `sh <팩>/hooks/cys-agy-statusline.sh --cys-autolink` · 윈도우: `<팩>\hooks\cys-agy-statusline.cmd --cys-autolink`
//! (아래 '윈도우'). 끝의 `--cys-autolink` 가 **cys 가 넣었다는 표지**다
//! (스크립트는 인자를 읽지 않는다). agy 가 settings.json 을 자기 구조체로 다시 써도 command 문자열은 남으므로 표지가
//! 사라지지 않는다(별도 키 표지는 agy 재기록에 지워질 수 있다). 제거(되돌리기 노브 · 완전 초기화)는 이 표지가 달린
//! 연결만 지운다 — 사용자가 매뉴얼을 보고 직접 넣은 cys 연결(표지 없음)은 건드리지 않는다.
//!
//! 유닉스 명령은 경로가 **셸 안전 문자**(영숫자 · `/._-+@` · 비ASCII 글자)일 때만 만든다(윈도우 규칙은 아래 '윈도우'). 그러면 agy 가
//! `sh -c` 로 부르든, 공백으로 쪼개 직접 실행하든 같은 argv 가 된다. 공백·따옴표가 든 경로는 자동 연결하지 않고
//! 안내만 한다(`UnsafePath`).
//!
//! ## agy 가 상태줄 명령을 부르는 방식 (macOS 판 agy 1.2.9 역어셈블 · 2026-09-24)
//!
//! 공식 문서에는 셸·인용·타임아웃이 없다. 그래서 이 맥의 `~/.local/bin/agy`(Mach-O arm64 · 심볼 표 없음)를 Go 함수표
//! (pclntab)로 풀어 `store.(*StatusLineRunner).run` 을 읽었다(증거: 보고서 폴더 `agy-autolink-evidence/06-agy-binary.txt`).
//! - `context.WithTimeout(…, 5_000_000_000)` — 명령 한 번에 **5초** 상한.
//! - `store.expandTilde(command)` 뒤 `exec.CommandContext(ctx, "sh", "-c", <명령>)` — 사용자 `$SHELL` 이 아니라 **`sh -c`**
//!   다(같은 패키지의 `store.resolveShell`(`$SHELL`·cmd·powershell 판별)은 상태줄이 부르지 않는다).
//! - 환경은 `syscall.Environ()` — agy 자신의 환경을 그대로 넘긴다(cys 좌석의 `CYS_SURFACE_ID` 가 이어진다는 근거 ·
//!   실제 좌석 실측은 아직 없다).
//! - 실패하면 `⚠ Statusline error:` 를 찍고, 연속 실패가 쌓이면 `Statusline disabled after %d consecutive failures` 로
//!   스스로 끈다(`recordFailure`).
//! 그래서 유닉스 명령 `sh <절대경로> --cys-autolink` 는 `sh -c` 안에서 그대로 한 번 더 `sh` 로 래퍼를 부르고, 래퍼의
//! 총예산(판독 1초 + push 0.4초)은 agy 의 5초 상한 안이다.
//!
//! ## 윈도우 — 자동 연결 **켬 + 쓰기 전 실연 검사** (0.14.45 · 오너 지시 '윈도우도 지원')
//!
//! 위 역어셈블은 **macOS 판**이다. 0.14.44 의 정적 분석(증거: 보고서 폴더
//! `_evidence/impl-0.14.44-20261006/WD/RESULT.md` · 윈도우 파일은 실행하지 않았다)으로 윈도우 판 agy 1.2.17(x64·arm64)의
//! `store.(*StatusLineRunner).run` 은 `exec.CommandContext(ctx, "cmd", "/c", <명령>)` 임을 확인했다(5초 상한 · 자기 환경 그대로).
//! `SysProcAttr.CmdLine` 을 쓰지 않으므로 명령줄은 Go `syscall.EscapeArg` 규칙으로 합성된다 — 명령 안의 `"` 는 `\"` 로
//! 바뀌어 cmd 에 넘어가고(공개 보고 weby-homelab#63 · doggy8088#64 와 부합), 공백이 있으면 명령 전체가 `"…"` 로 감싸인다.
//! cmd 는 `/c "…"` 의 첫·끝 따옴표를 벗기므로(따옴표 안이 실행 파일 이름 하나가 아닐 때의 cmd 규칙) 따옴표 없는 명령은
//! 그대로 실행된다. 그래서 윈도우 명령은:
//!
//! - `<팩>\hooks\cys-agy-statusline.cmd --cys-autolink` — **인터프리터 없음**(`bash` 는 PATH 에서 WSL 의
//!   `System32\bash.exe` 로 잡힐 수 있다) · **따옴표 없음** · 역슬래시 · `\\?\` 확장 접두는 벗기고 UNC 는 만들지 않는다.
//! - 경로는 `X:\` + ASCII 영숫자 + `\ . _ -` 만 허용한다. 공백 · `% ^ & ( ) !` · 따옴표 · 비ASCII(코드페이지 해석)는
//!   cmd 문법에서 뜻이 바뀌거나 확신이 없으므로 `UnsafePath`(안내만)다.
//!
//! 정적 분석은 실기 관측이 아니다. 그래서 윈도우는 **쓰기 직전에 그 명령을 agy 와 같은 방식으로 한 번 실행해 본다**
//! (`live_probe`): `cmd /c <Go 규칙으로 인용한 명령>` · 콘솔 창 없음(`ChildLifetime::Attached` = `CREATE_NO_WINDOW`) ·
//! 좌석 환경 변수(`CYS_SURFACE_ID`) 제거(데몬에 아무것도 가지 않는다) · PATH 는 좌석과 같은 선두 주입 · stdin 에 빈 쿼터
//! `{"product":"antigravity","quota":{}}` · 5초 상한(넘으면 kill). **통과 = 종료 코드 0 ∧ 표준출력 마지막 줄이 `cys`**
//! (`cys usage-report-stdin --agy` 가 빈 쿼터에 찍는 줄 — 아무것도 실행되지 않은 경우(.cmd 가 `cys` 를 못 찾으면 출력 없음)와
//! 구별된다). 실패하면 `WindowsProbeFailed` — 설정 파일은 한 바이트도 바꾸지 않고 붙여 넣을 명령을 안내한다. 검사 실패는
//! 설치·기동을 막지 않는다(결과 하나일 뿐이다). 되돌리기 노브(`CYS_AGY_STATUSLINE=0` · `~/.cys/agy-statusline-off`)는 OS 무관.
//!
//! 사용자가 0.14.44 이전 안내를 보고 직접 넣은 `bash C:/…/cys-agy-statusline.sh` 연결은 `CysManual` 로 보아 바꾸지 않는다
//! (doctor 가 `.cmd` 명령을 권하는 안내만 붙인다).

use serde_json::Value;
use std::path::{Path, PathBuf};

/// agy 상태줄 전용 래퍼(팩 `hooks/` 안 · 훅이 아니라 상태줄 명령).
pub const SCRIPT: &str = "cys-agy-statusline.sh";
/// 윈도우용 상태줄 래퍼(같은 폴더 · agy 가 `cmd /c` 로 부른다 — 모듈 머리 '윈도우').
pub const SCRIPT_CMD: &str = "cys-agy-statusline.cmd";
/// 이 OS 의 연결 명령이 부를 래퍼 파일 이름(순수).
pub fn script_name(windows: bool) -> &'static str {
    if windows {
        SCRIPT_CMD
    } else {
        SCRIPT
    }
}
/// cys 가 넣은 연결의 표지 — 명령 끝 토큰.
pub const MARKER: &str = "--cys-autolink";
/// 되돌리기 노브(env) — `0` 이면 자동 연결 끔 + cys 가 넣은 연결 제거.
pub const ENV_KNOB: &str = "CYS_AGY_STATUSLINE";
/// 되돌리기 노브(파일) — `~/.cys/agy-statusline-off` 가 있으면 env `0` 과 같다(GUI 는 셸 env 를 받지 못한다 —
/// `CYS_WIN_WHEEL_GUARD_OFF`·`~/.cys/win-wheel-guard-off` 와 같은 짝 규약).
pub const OFF_FILE: &str = "agy-statusline-off";
/// '연결한 적 있음' 기록(팩 상대 · 팩 설치의 prune 대상이 아닌 state/ 아래).
pub const RECORD_REL: &str = "state/agy-statusline-linked";
/// 쓰기 전 백업 접미(설정 파일 옆 · 실제로 쓸 때만 만든다).
pub const BACKUP_SUFFIX: &str = ".bak-cys";
/// 이보다 큰 settings.json 은 건드리지 않는다(정상 파일은 수백 바이트 — 2026-09-24 이 맥 316B).
const MAX_SETTINGS_BYTES: u64 = 1024 * 1024;

/// agy 설정 파일 경로(순수) — `<home>/.gemini/antigravity-cli/settings.json`.
pub fn settings_path_under(home: &Path) -> PathBuf {
    home.join(".gemini").join("antigravity-cli").join("settings.json")
}

/// 안내 문구용 설정 파일 경로 문자열(순수 · OS 규칙 주입) — 윈도우는 `%USERPROFILE%\.gemini\antigravity-cli\settings.json`
/// 을 역슬래시로, 유닉스는 정슬래시로 적는다. 맥에서도 윈도우 규칙을 시험할 수 있게 문자열로 받는다.
pub fn settings_path_display(home: &str, windows: bool) -> String {
    if windows {
        let h = home.replace('/', "\\");
        format!("{}\\.gemini\\antigravity-cli\\settings.json", h.trim_end_matches('\\'))
    } else {
        format!("{}/.gemini/antigravity-cli/settings.json", home.trim_end_matches('/'))
    }
}

/// 경로 문자열이 셸 안전 문자만으로 되어 있는가(순수 · 유닉스). 안전 = `sh -c`·공백 분리 직접 실행 어느 해석으로도
/// 같은 한 토큰이 된다.
fn path_is_shell_safe(p: &str) -> bool {
    p.starts_with('/')
        && p.chars().all(|c| {
            c.is_ascii_alphanumeric()
                || matches!(c, '/' | '.' | '_' | '-' | '+' | '@')
                // 비ASCII 글자(예: 한글 사용자 폴더)는 sh·공백 분리 모두 안전하다.
                || (!c.is_ascii() && !c.is_whitespace() && !c.is_control())
        })
}

/// 윈도우 경로가 `cmd /c` + Go `EscapeArg` 를 거쳐도 그대로 한 토큰인가(순수). `X:\` 로 시작하고 나머지는 ASCII 영숫자와
/// `\ . _ -` 뿐이어야 한다 — 공백(Go 가 명령 전체를 따옴표로 감싸고 cmd 가 쪼갠다) · `% ^ & ( ) !`(cmd 확장·연결 문법) ·
/// 따옴표(Go 가 `\"` 로 바꾼다) · 비ASCII(코드페이지 해석 불확실)는 거절한다.
fn windows_path_is_safe(p: &str) -> bool {
    let b = p.as_bytes();
    b.len() >= 4
        && b[0].is_ascii_alphabetic()
        && b[1] == b':'
        && b[2] == b'\\'
        && b[3..].iter().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'\\' | b'.' | b'_' | b'-'))
}

/// 연결 명령 문자열(순수 · OS 규칙 주입). `None` = 이 경로로는 안전한 명령을 만들 수 없다(공백·따옴표·상대경로 등).
///
/// - 유닉스: `sh <pack>/hooks/cys-agy-statusline.sh[ --cys-autolink]`
/// - 윈도우: `<pack>\hooks\cys-agy-statusline.cmd[ --cys-autolink]` — 인터프리터 없음(cmd 가 `.cmd` 를 직접 실행) ·
///   정슬래시는 역슬래시로 · `\\?\` 확장 접두는 벗기고 UNC(`\\server\…` · `\\?\UNC\…`)는 만들지 않는다 ·
///   **따옴표를 두르지 않는다**(Go `EscapeArg` 가 `\"` 로 바꿔 경로가 깨진다 — 모듈 머리).
pub fn link_command_for(pack_dir: &str, windows: bool, marker: bool) -> Option<String> {
    let script = if windows {
        let p = pack_dir.replace('/', "\\");
        let p = p.strip_prefix(r"\\?\").unwrap_or(&p);
        let script = format!("{}\\hooks\\{SCRIPT_CMD}", p.trim_end_matches('\\'));
        if !windows_path_is_safe(&script) {
            return None;
        }
        script
    } else {
        let script = format!("{}/hooks/{SCRIPT}", pack_dir.trim_end_matches('/'));
        if !path_is_shell_safe(&script) {
            return None;
        }
        format!("sh {script}")
    };
    Some(if marker { format!("{script} {MARKER}") } else { script })
}

/// Go `syscall.EscapeArg`(windows) 의 재현(순수) — agy 가 `exec.CommandContext(ctx, "cmd", "/c", <명령>)` 의 명령을
/// 명령줄에 넣을 때 쓰는 규칙이다. 실연 검사(`live_probe`)는 이 결과를 그대로(`raw_arg`) 넘겨 agy 와 같은 명령줄을 만든다.
/// 규칙: 빈 문자열 → `""` · `"`·`\`·공백·탭이 없으면 그대로 · 공백/탭이 있으면 전체를 `"…"` 로 · `"` 앞의 역슬래시는
/// 두 배 + `\"` · 따옴표로 감쌀 때 끝의 역슬래시는 두 배.
pub fn go_escape_arg(s: &str) -> String {
    if s.is_empty() {
        return "\"\"".into();
    }
    let needs_backslash = s.bytes().any(|c| c == b'"' || c == b'\\');
    let has_space = s.bytes().any(|c| c == b' ' || c == b'\t');
    if !needs_backslash && !has_space {
        return s.to_string();
    }
    if !needs_backslash {
        return format!("\"{s}\"");
    }
    let mut out = String::with_capacity(s.len() + 8);
    if has_space {
        out.push('"');
    }
    let mut slashes = 0usize;
    for c in s.chars() {
        match c {
            '\\' => slashes += 1,
            '"' => {
                out.extend(std::iter::repeat_n('\\', slashes + 1));
                slashes = 0;
            }
            _ => slashes = 0,
        }
        out.push(c);
    }
    if has_space {
        out.extend(std::iter::repeat_n('\\', slashes));
        out.push('"');
    }
    out
}

/// 되돌리기 노브 판정(순수) — env `0`(앞뒤 공백 무시) 또는 끔 파일 존재.
pub fn knob_off(env: Option<&str>, off_file_exists: bool) -> bool {
    env.map(str::trim) == Some("0") || off_file_exists
}

/// statusLine 칸의 상태.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Slot {
    /// 키가 없다.
    Absent,
    /// `null` · `{}` · 명령이 빈 agy 기본형(`{"type":"","command":"","enabled":false}` 과 그 부분집합).
    Empty,
    /// cys 가 넣은 연결(표지 있음). `enabled` = 그 칸의 enabled 값(없으면 None).
    OursAuto { enabled: Option<bool> },
    /// 표지 없는 cys 연결(사용자가 매뉴얼을 보고 직접 넣음 · 구 `cys-statusline.sh` 포함).
    CysManual { enabled: Option<bool> },
    /// 그 밖의 사용자 설정 — 불가침.
    User,
}

fn norm_cmd(cmd: &str) -> String {
    cmd.replace('\\', "/").replace(['"', '\''], "")
}

/// 명령이 cys 가 넣은(표지 달린) 연결인가(순수). 유닉스 `…/hooks/cys-agy-statusline.sh` 와 윈도우
/// `…\hooks\cys-agy-statusline.cmd`(윈도우 파일 이름은 대소문자 무관) 둘 다 인정한다 — 제거 경로(노브·완전 초기화)는 OS 무관.
pub fn command_is_ours_auto(cmd: &str) -> bool {
    let n = norm_cmd(cmd);
    let toks: Vec<&str> = n.split_whitespace().collect();
    toks.last() == Some(&MARKER)
        && toks.iter().any(|t| {
            t.ends_with(&format!("/hooks/{SCRIPT}"))
                || t.to_ascii_lowercase().ends_with(&format!("/hooks/{SCRIPT_CMD}"))
        })
}

/// 명령이 cys 상태줄 래퍼를 부르는가(표지 무관 · 순수).
pub fn command_is_cys(cmd: &str) -> bool {
    let n = norm_cmd(cmd);
    n.contains(SCRIPT) || n.to_ascii_lowercase().contains(SCRIPT_CMD) || n.contains("cys-statusline.sh")
}

/// 윈도우에서 사람이 직접 넣은 옛 `bash …/cys-agy-statusline.sh` 꼴의 cys 연결인가(순수) — 윈도우 판 agy 는 `cmd /c` 로
/// 부르므로 `bash` 가 WSL 의 `System32\bash.exe` 로 잡힐 수 있다. cys 는 바꾸지 않고 `.cmd` 명령을 안내만 한다.
pub fn command_is_legacy_windows_sh(cmd: &str) -> bool {
    command_is_cys(cmd) && !norm_cmd(cmd).to_ascii_lowercase().contains(SCRIPT_CMD)
}

/// settings 루트에서 statusLine 칸을 분류한다(순수).
pub fn classify(root: &Value) -> Slot {
    let Some(sl) = root.get("statusLine") else {
        return Slot::Absent;
    };
    let obj = match sl {
        Value::Null => return Slot::Empty,
        Value::Object(m) => m,
        _ => return Slot::User,
    };
    let enabled = obj.get("enabled").and_then(|v| v.as_bool());
    match obj.get("command") {
        Some(Value::String(c)) if !c.trim().is_empty() => {
            if command_is_ours_auto(c) {
                Slot::OursAuto { enabled }
            } else if command_is_cys(c) {
                Slot::CysManual { enabled }
            } else {
                Slot::User
            }
        }
        Some(Value::String(_)) | None => {
            // 명령이 비었다 — agy 기본형과 그 부분집합만 '빈 칸'이다. padding·stack_with_default 등 다른 키가 있으면
            // 사용자가 만진 흔적이라 건드리지 않는다(보수).
            let keys_ok = obj.keys().all(|k| matches!(k.as_str(), "type" | "command" | "enabled"));
            let type_ok = match obj.get("type") {
                None => true,
                Some(Value::String(t)) => t.is_empty() || t == "command",
                _ => false,
            };
            let enabled_ok = obj.get("enabled").is_none_or(|v| v.is_boolean());
            if keys_ok && type_ok && enabled_ok {
                Slot::Empty
            } else {
                Slot::User
            }
        }
        Some(_) => Slot::User,
    }
}

/// 넣을 statusLine 값(순수) — `stack_with_default: true`(agy 기본 줄 아래에 붙인다).
pub fn desired_value(cmd: &str) -> Value {
    serde_json::json!({"type": "command", "command": cmd, "enabled": true, "stack_with_default": true})
}

// ───────────────────────────── 텍스트 외과 수술 (순수) ─────────────────────────────

pub(crate) struct Member {
    pub(crate) key_start: usize,
    pub(crate) key: String,
    pub(crate) val_start: usize,
    pub(crate) val_end: usize,
}

pub(crate) struct RootScan {
    pub(crate) open: usize,
    pub(crate) close: usize,
    pub(crate) members: Vec<Member>,
}

fn skip_ws(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && matches!(b[i], b' ' | b'\t' | b'\n' | b'\r') {
        i += 1;
    }
    i
}

fn skip_string(b: &[u8], mut i: usize) -> Result<usize, String> {
    if b.get(i) != Some(&b'"') {
        return Err("문자열 시작이 아니다".into());
    }
    i += 1;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'"' => return Ok(i + 1),
            _ => i += 1,
        }
    }
    Err("문자열이 닫히지 않았다".into())
}

fn skip_value(b: &[u8], i: usize) -> Result<usize, String> {
    match b.get(i) {
        None => Err("값이 없다".into()),
        Some(b'"') => skip_string(b, i),
        Some(b'{') | Some(b'[') => {
            let mut depth = 0usize;
            let mut j = i;
            while j < b.len() {
                match b[j] {
                    b'"' => {
                        j = skip_string(b, j)?;
                        continue;
                    }
                    b'{' | b'[' => depth += 1,
                    b'}' | b']' => {
                        depth = depth.checked_sub(1).ok_or("괄호 짝이 맞지 않는다")?;
                        if depth == 0 {
                            return Ok(j + 1);
                        }
                    }
                    _ => {}
                }
                j += 1;
            }
            Err("괄호가 닫히지 않았다".into())
        }
        Some(_) => {
            let mut j = i;
            while j < b.len() && !matches!(b[j], b',' | b'}' | b']' | b' ' | b'\t' | b'\n' | b'\r') {
                j += 1;
            }
            if j == i {
                Err("빈 값".into())
            } else {
                Ok(j)
            }
        }
    }
}

/// 최상위 객체의 멤버 위치를 잰다. 입력은 이미 serde_json 으로 유효성이 확인된 텍스트(BOM 제거 뒤)다.
pub(crate) fn scan_root(t: &str) -> Result<RootScan, String> {
    let b = t.as_bytes();
    let mut i = skip_ws(b, 0);
    if b.get(i) != Some(&b'{') {
        return Err("루트가 객체가 아니다".into());
    }
    let open = i;
    i = skip_ws(b, i + 1);
    let mut members = Vec::new();
    if b.get(i) == Some(&b'}') {
        return Ok(RootScan { open, close: i, members });
    }
    loop {
        let key_start = i;
        let key_end = skip_string(b, i)?;
        let key: String = serde_json::from_str(&t[key_start..key_end]).map_err(|e| format!("키 판독 실패: {e}"))?;
        i = skip_ws(b, key_end);
        if b.get(i) != Some(&b':') {
            return Err("콜론이 없다".into());
        }
        i = skip_ws(b, i + 1);
        let val_start = i;
        let val_end = skip_value(b, i)?;
        members.push(Member { key_start, key, val_start, val_end });
        i = skip_ws(b, val_end);
        match b.get(i) {
            Some(b',') => i = skip_ws(b, i + 1),
            Some(b'}') => return Ok(RootScan { open, close: i, members }),
            _ => return Err("멤버 구분자가 없다".into()),
        }
    }
}

pub(crate) fn line_indent(t: &str, pos: usize) -> Option<&str> {
    let line_start = t[..pos].rfind('\n').map_or(0, |i| i + 1);
    let ind = &t[line_start..pos];
    (!ind.is_empty() && ind.chars().all(|c| c == ' ' || c == '\t')).then_some(ind)
}

fn render_obj(cmd: &str, pretty: bool, ind: &str, unit: &str, nl: &str, sep: &str) -> String {
    let c = serde_json::to_string(cmd).unwrap_or_else(|_| "\"\"".into());
    let pairs = [
        ("\"type\"", "\"command\"".to_string()),
        ("\"command\"", c),
        ("\"enabled\"", "true".to_string()),
        ("\"stack_with_default\"", "true".to_string()),
    ];
    if pretty {
        let inner = format!("{ind}{unit}");
        let body: Vec<String> = pairs.iter().map(|(k, v)| format!("{inner}{k}{sep}{v}")).collect();
        format!("{{{nl}{}{nl}{ind}}}", body.join(&format!(",{nl}")))
    } else {
        let body: Vec<String> = pairs.iter().map(|(k, v)| format!("{k}{sep}{v}")).collect();
        format!("{{{}}}", body.join(","))
    }
}

/// statusLine 칸을 cys 연결로 넣은(또는 빈 칸을 바꾼) 새 텍스트(순수). `text` 는 BOM 을 뗀 본문이다.
/// 다른 바이트는 그대로 둔다 — 들여쓰기·줄바꿈(CRLF)·키 사이 구분(`": "`/`":"`)은 파일에서 읽어 맞춘다.
pub fn render_linked(text: &str, cmd: &str) -> Result<String, String> {
    let scan = scan_root(text)?;
    let nl = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let hits: Vec<&Member> = scan.members.iter().filter(|m| m.key == "statusLine").collect();
    if hits.len() > 1 {
        return Err("statusLine 키가 둘 이상이다(모호 — 건드리지 않는다)".into());
    }
    let first = scan.members.first();
    // 파일 모양: 멤버가 없으면 agy 가 쓰는 모양(2칸 들여쓰기)으로 쓴다.
    let pretty = first.is_none() || text[scan.open..=scan.close].contains('\n');
    let unit: String = first
        .and_then(|m| line_indent(text, m.key_start))
        .map(str::to_string)
        .unwrap_or_else(|| "  ".to_string());
    let sep = match first {
        Some(m) => {
            let between = &text[m.key_start..m.val_start];
            let colon = between.rfind(':').unwrap_or(0);
            if between[colon + 1..].is_empty() {
                ":"
            } else {
                ": "
            }
        }
        None => ": ",
    };
    match hits.first() {
        Some(m) => {
            let ind = line_indent(text, m.key_start).unwrap_or(&unit).to_string();
            let obj = render_obj(cmd, pretty, &ind, &unit, nl, sep);
            Ok(format!("{}{}{}", &text[..m.val_start], obj, &text[m.val_end..]))
        }
        None if scan.members.is_empty() => {
            let obj = render_obj(cmd, true, &unit, &unit, nl, ": ");
            Ok(format!(
                "{}{nl}{unit}\"statusLine\": {obj}{nl}{}",
                &text[..scan.open + 1],
                &text[scan.close..]
            ))
        }
        None => {
            let last = scan.members.last().ok_or("멤버 없음")?;
            let obj = render_obj(cmd, pretty, &unit, &unit, nl, sep);
            let lead = if pretty { format!(",{nl}{unit}") } else { ",".to_string() };
            Ok(format!(
                "{}{lead}\"statusLine\"{sep}{obj}{}",
                &text[..last.val_end],
                &text[last.val_end..]
            ))
        }
    }
}

/// statusLine 칸을 통째로 뺀 새 텍스트(순수). 칸이 없으면 Err.
pub fn render_unlinked(text: &str) -> Result<String, String> {
    let scan = scan_root(text)?;
    let idx: Vec<usize> = scan
        .members
        .iter()
        .enumerate()
        .filter(|(_, m)| m.key == "statusLine")
        .map(|(i, _)| i)
        .collect();
    let [i] = idx.as_slice() else {
        return Err("statusLine 키가 없거나 둘 이상이다".into());
    };
    let i = *i;
    let n = scan.members.len();
    let m = &scan.members[i];
    Ok(if n == 1 {
        format!("{}{}", &text[..scan.open + 1], &text[scan.close..])
    } else if i + 1 < n {
        format!("{}{}", &text[..m.key_start], &text[scan.members[i + 1].key_start..])
    } else {
        format!("{}{}", &text[..scan.members[i - 1].val_end], &text[m.val_end..])
    })
}

/// 수술 결과 사후 검증(순수) — 다른 최상위 키의 값이 전부 같고(추가·삭제 0) statusLine 이 원하는 값인가.
pub fn verify_render(orig: &Value, new_text: &str, want: Option<&Value>) -> Result<(), String> {
    let new: Value = serde_json::from_str(new_text).map_err(|e| format!("수술 결과가 JSON 이 아니다: {e}"))?;
    let (Some(o), Some(n)) = (orig.as_object(), new.as_object()) else {
        return Err("수술 결과 루트가 객체가 아니다".into());
    };
    for (k, v) in o {
        if k != "statusLine" && n.get(k) != Some(v) {
            return Err(format!("다른 키가 바뀌었다: {k}"));
        }
    }
    if n.keys().any(|k| k != "statusLine" && !o.contains_key(k)) {
        return Err("모르는 키가 생겼다".into());
    }
    match want {
        Some(w) if n.get("statusLine") == Some(w) => Ok(()),
        None if n.get("statusLine").is_none() => Ok(()),
        _ => Err("statusLine 이 원하는 값이 아니다".into()),
    }
}

// ───────────────────────────── 파일 입출력 ─────────────────────────────

/// 조정 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// agy 설정 폴더가 없다(agy 미설치) — 아무것도 만들지 않는다.
    NotInstalled,
    /// 윈도우 — 쓰기 전 실연 검사(`live_probe`)가 실패해 쓰지 않았다(설정 파일 무변경). `command` = 사람이 직접 넣을 명령.
    WindowsProbeFailed { reason: String, command: String },
    /// 팩 경로에 공백·따옴표 등이 있어 안전한 명령을 만들 수 없다.
    UnsafePath(String),
    /// 이미 cys 연결 — 무동작.
    AlreadyLinked(Slot),
    /// 사용자 statusLine — 덮지 않는다.
    UserOwned,
    /// 전에 연결했던 칸이 비었다 — 사용자 해제를 존중해 다시 넣지 않는다.
    PreviouslyLinked,
    /// 새로 연결했다. `created` = 설정 파일을 새로 만들었다.
    Linked { created: bool },
    /// 표지 달린 연결을 뺐다.
    Unlinked,
    /// 뺄 것이 없다(표지 달린 연결 없음 — 칸 상태 동봉 · 파일 없음이면 None).
    NothingToUnlink(Option<Slot>),
    /// 쓰지 않았다(사유).
    Refused(String),
}

/// 백업 위치.
pub enum Backup<'a> {
    /// 설정 파일 옆 `settings.json.bak-cys`.
    Beside,
    /// 지정 폴더 안 `agy-antigravity-cli.settings.json`(완전 초기화 격리 폴더 규약).
    Dir(&'a Path),
}

/// 쓰기 전 실연 검사 — 연결 명령(표지 포함)을 받아 agy 처럼 실행해 본다. `Ok` = 통과.
pub type Probe<'a> = &'a dyn Fn(&str) -> Result<(), String>;

/// 조정에 필요한 경로(주입형 — 시험은 가짜 홈과 가짜 검사를 준다).
pub struct Ctx<'a> {
    pub settings: &'a Path,
    pub pack_dir: &'a Path,
    pub record: &'a Path,
    pub windows: bool,
    /// 윈도우에서 쓰기 직전에 부르는 실연 검사(운영 = [`live_probe`]). `None` 이면 윈도우는 쓰지 않는다(실패 방향 =
    /// 쓰지 않음). 유닉스는 부르지 않는다(맥 판 agy 의 `sh -c` 는 역어셈블로 확인 · 종전 동작 그대로).
    pub probe: Option<Probe<'a>>,
}

/// 파일 판독 결과: (원문 바이트, BOM 뗀 본문, BOM 여부, 파싱 값 — 빈 파일이면 None).
pub(crate) type Loaded = (Vec<u8>, String, bool, Option<Value>);

pub(crate) fn load(settings: &Path) -> Result<Option<Loaded>, String> {
    let meta = match std::fs::symlink_metadata(settings) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("설정 파일 상태를 읽지 못했다: {e}")),
    };
    if meta.file_type().is_symlink() {
        return Err("심볼릭 링크다(dotfile 관리 도구 등)".into());
    }
    if !meta.is_file() {
        return Err("일반 파일이 아니다".into());
    }
    if meta.len() > MAX_SETTINGS_BYTES {
        return Err(format!("파일이 너무 크다({}바이트)", meta.len()));
    }
    let raw = std::fs::read(settings).map_err(|e| format!("읽지 못했다: {e}"))?;
    let (bom, body) = match raw.strip_prefix(b"\xef\xbb\xbf") {
        Some(rest) => (true, rest),
        None => (false, raw.as_slice()),
    };
    let text = String::from_utf8(body.to_vec()).map_err(|_| "UTF-8 이 아니다".to_string())?;
    if text.trim().is_empty() {
        return Ok(Some((raw, text, bom, None)));
    }
    let v: Value = serde_json::from_str(&text).map_err(|e| format!("JSON 파싱 실패({e})"))?;
    if !v.is_object() {
        return Err("루트가 객체가 아니다".into());
    }
    Ok(Some((raw, text, bom, Some(v))))
}

/// 읽기 전용 점검(doctor) — `Ok(None)` = 설정 파일 없음.
pub fn inspect(settings: &Path) -> Result<Option<Slot>, String> {
    Ok(load(settings)?.map(|(_, _, _, v)| v.as_ref().map_or(Slot::Absent, classify)))
}

/// statusLine 의 command 문자열(읽기 전용 · doctor 안내용) — 파일 없음·판독 불가·명령 없음이면 None.
pub fn inspect_command(settings: &Path) -> Option<String> {
    let (_, _, _, v) = load(settings).ok()??;
    v?.get("statusLine")?.get("command")?.as_str().map(str::to_string)
}

// ───────────────────────────── 윈도우 실연 검사 ─────────────────────────────

/// 실연 검사의 stdin — agy 상태 JSON 의 최소꼴(쿼터 없음 → cys 는 아무것도 보내지 않고 `cys` 한 줄만 찍는다).
pub const PROBE_INPUT: &str = "{\"product\":\"antigravity\",\"quota\":{}}";
/// 실연 검사 상한 — agy 의 명령 한 번 상한(5초 · 역어셈블)과 같다. 이보다 느리면 agy 아래에서도 실패한다.
pub const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
/// 통과 판정 줄 — `cys usage-report-stdin --agy` 가 빈 쿼터에 찍는 사람용 줄(cys.rs `agy_statusline_human_line`).
pub const PROBE_EXPECT: &str = "cys";

/// 검사 출력 판정(순수) — 표준출력의 마지막 비어 있지 않은 줄이 정확히 `cys` 여야 한다. 래퍼가 `cys` 를 못 찾으면(PATH 부재)
/// cmd 의 오류는 `2>nul` 로 버려지고 래퍼는 0 으로 끝나므로, 종료 코드만으로는 '아무것도 실행되지 않음'과 구별되지 않는다.
pub fn probe_output_ok(stdout: &str) -> Result<(), String> {
    let last = stdout.lines().map(str::trim).rfind(|l| !l.is_empty()).unwrap_or("");
    if last == PROBE_EXPECT {
        Ok(())
    } else {
        let shown: String = last.chars().take(80).collect();
        Err(format!("출력이 기대한 `{PROBE_EXPECT}` 가 아니다(마지막 줄 {shown:?}) — cys 가 PATH 에 없거나 래퍼가 실행되지 않았다"))
    }
}

/// 검사 자식 하나를 돌린다(OS 공용 · 시험 이음매). 콘솔 창 없음(`Attached` = `CREATE_NO_WINDOW`) · 데몬 자동 기동 봉인 ·
/// 좌석 표지(`CYS_SURFACE_ID` 와 옛 이름) 제거 · `path` 가 있으면 PATH 교체 · stdin 에 `input` · `timeout` 넘기면 kill.
/// 반환 = 종료 코드 0 일 때의 표준출력. 무엇이 실패해도 Err 로 돌려줄 뿐 패닉하지 않는다.
#[cfg_attr(not(windows), allow(dead_code))] // 운영 호출은 윈도우(`run_like_agy`)뿐 · 유닉스는 시험이 잰다
fn probe_spawn(
    program: &Path,
    add_args: &dyn Fn(&mut std::process::Command),
    path: Option<&std::ffi::OsStr>,
    input: &[u8],
    timeout: std::time::Duration,
) -> Result<String, String> {
    use crate::SpawnPolicy as _;
    use std::io::{Read as _, Write as _};
    use std::process::Stdio;
    let mut c = std::process::Command::new(program);
    c.spawn_policy(crate::ChildLifetime::Attached).no_autostart();
    add_args(&mut c);
    for k in [crate::ENV_SURFACE_ID, "JAVIS_SURFACE_ID", "AITERM_SURFACE_ID"] {
        c.env_remove(k);
    }
    if let Some(p) = path {
        c.env("PATH", p);
    }
    c.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
    let mut child = c.spawn().map_err(|e| format!("실행하지 못했다: {e}"))?;
    if let Some(mut si) = child.stdin.take() {
        let _ = si.write_all(input); // 읽지 않고 끝나는 자식이면 파이프 오류 — 무시
    }
    let (tx, rx) = std::sync::mpsc::channel();
    if let Some(mut so) = child.stdout.take() {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = so.by_ref().take(64 * 1024).read_to_end(&mut buf);
            let _ = tx.send(buf);
        });
    }
    let deadline = std::time::Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{}초 안에 끝나지 않았다(agy 도 같은 상한에서 실패한다)", timeout.as_secs_f32()));
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(20)),
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("종료를 기다리지 못했다: {e}"));
            }
        }
    };
    // 손자가 파이프를 쥐고 남아도 남은 시간 이상은 기다리지 않는다(읽기 스레드는 버린다 — 파이프가 닫히면 스스로 끝난다).
    let left = deadline.saturating_duration_since(std::time::Instant::now()).max(std::time::Duration::from_millis(200));
    let out = rx.recv_timeout(left).unwrap_or_default();
    if !status.success() {
        return Err(format!("종료 코드 {:?}", status.code()));
    }
    Ok(String::from_utf8_lossy(&out).into_owned())
}

/// 운영 실연 검사 — 윈도우 판 agy 1.2.17 이 상태줄 명령을 부르는 방식(`cmd /c` + Go `EscapeArg` · 5초 · 자기 환경)을 그대로
/// 흉내 내 연결 명령을 한 번 실행한다. PATH 는 좌석과 같은 선두 주입(`runtime_prefixed_path` — 실행 파일 폴더 우선)이다.
/// 통과 = 종료 코드 0 ∧ [`probe_output_ok`]. 좌석 표지가 없으므로 cys 는 데몬에 아무것도 보내지 않는다.
#[cfg(windows)]
pub fn live_probe(cmd: &str) -> Result<(), String> {
    let cur = std::env::var("PATH").unwrap_or_default();
    let path = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(Path::to_path_buf))
        .and_then(|d| crate::runtime_prefixed_path(&d, &cur));
    let out = run_like_agy(cmd, path.as_deref().map(std::ffi::OsStr::new))?;
    probe_output_ok(&out)
}

/// agy 처럼 `cmd /c <Go 인용 명령>` 으로 한 번 실행하고 표준출력을 돌려준다(윈도우 · [`live_probe`] 와 시험이 공유).
#[cfg(windows)]
fn run_like_agy(cmd: &str, path: Option<&std::ffi::OsStr>) -> Result<String, String> {
    use std::os::windows::process::CommandExt as _;
    // agy 는 `cmd` 를 PATH 에서 찾는다 — 검사는 작업 폴더·실행 파일 폴더의 가짜 cmd 를 피해 시스템 cmd.exe 를 직접 쓴다.
    let cmd_exe = std::env::var_os("SystemRoot")
        .map(|r| PathBuf::from(r).join("System32").join("cmd.exe"))
        .filter(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from("cmd"));
    let escaped = go_escape_arg(cmd);
    probe_spawn(
        &cmd_exe,
        &|c| {
            c.arg("/c").raw_arg(&escaped);
        },
        path,
        PROBE_INPUT.as_bytes(),
        PROBE_TIMEOUT,
    )
}

/// 유닉스에는 실연 검사가 없다(맥 판 agy 의 `sh -c` 는 역어셈블로 확인 · [`ensure_linked`] 는 유닉스에서 부르지 않는다).
#[cfg(not(windows))]
pub fn live_probe(_cmd: &str) -> Result<(), String> {
    Err("실연 검사는 윈도우 전용이다".into())
}

/// 쓰기 가능 확인 — 추가 모드로 열어 보기만 한다(내용·mtime 무변경). 읽기 전용·권한 없음·(윈도우) 잠김이면 Err.
pub(crate) fn probe_writable(settings: &Path) -> Result<(), String> {
    if std::fs::metadata(settings).map(|m| m.permissions().readonly()).unwrap_or(false) {
        return Err("읽기 전용 파일이다".into());
    }
    std::fs::OpenOptions::new()
        .append(true)
        .open(settings)
        .map(|_| ())
        .map_err(|e| format!("쓰기로 열 수 없다(권한·잠김): {e}"))
}

#[cfg(unix)]
pub(crate) fn file_mode(settings: &Path) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(settings).ok().map(|m| m.permissions().mode() & 0o7777)
}
#[cfg(not(unix))]
pub(crate) fn file_mode(_settings: &Path) -> Option<u32> {
    None
}

/// 쓰기 전 백업 — 파일을 다시 복사하지 않고 **판독·검증한 바이트(`raw`)** 를 쓴다. 판독과 백업 사이에 다른 쓰기(업그레이드
/// 직후 앱과 데몬의 동시 설치 등)가 끼어도 백업이 남의 결과로 바뀌지 않는다(그 쓰기는 `commit` 의 재판독 대조가 막는다).
/// 원 권한을 따른다(agy 설정은 0600 — 백업이 더 넓게 열리면 안 된다).
fn backup(settings: &Path, raw: &[u8], how: &Backup) -> Result<(), String> {
    let dest = match how {
        Backup::Beside => PathBuf::from(format!("{}{BACKUP_SUFFIX}", settings.display())),
        Backup::Dir(d) => {
            std::fs::create_dir_all(d).map_err(|e| format!("백업 폴더를 만들지 못했다: {e}"))?;
            d.join("agy-antigravity-cli.settings.json")
        }
    };
    crate::pack::write_atomic_mode(&dest, raw, file_mode(settings).or(Some(0o600)))
        .map_err(|e| format!("백업 실패({}): {e}", dest.display()))
}

/// 쓰기 직전 재판독 대조 + 원자 쓰기 + 되읽기 분류.
fn commit(settings: &Path, orig_raw: Option<&[u8]>, new_bytes: &[u8], mode: Option<u32>) -> Result<Slot, String> {
    let now = std::fs::read(settings).ok();
    if now.as_deref() != orig_raw {
        return Err("그 사이 다른 프로그램(agy 등)이 파일을 바꿨다 — 이번에는 쓰지 않는다".into());
    }
    crate::pack::write_atomic_mode(settings, new_bytes, mode).map_err(|e| format!("원자 쓰기 실패: {e}"))?;
    match load(settings) {
        Ok(Some((_, _, _, Some(v)))) => Ok(classify(&v)),
        Ok(_) => Err("되읽기: 파일이 비었다".into()),
        Err(e) => Err(format!("되읽기 실패: {e}")),
    }
}

fn write_record(record: &Path, settings: &Path) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if let Some(d) = record.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = crate::pack::write_atomic(record, format!("linked_at={now}\nsettings={}\n", settings.display()).as_bytes());
}

/// statusLine 칸이 비어 있거나 없을 때만 cys 연결을 넣는다. `force` = '연결한 적 있음' 기록을 무시한다
/// (사람이 부른 `cys doctor --fix` 만 쓴다 — 설치 경로는 false).
pub fn ensure_linked(ctx: &Ctx, force: bool) -> Outcome {
    ensure_linked_cmd(ctx, force, link_command_for(&ctx.pack_dir.to_string_lossy(), ctx.windows, true))
}

/// [`ensure_linked`] 의 본체 — 넣을 명령을 인자로 받는다(시험 이음매: 맥 샌드박스 파일로 윈도우 꼴 명령의 흐름을 잰다).
fn ensure_linked_cmd(ctx: &Ctx, force: bool, cmd: Option<String>) -> Outcome {
    // agy 가 없는 기계(대다수)는 OS 무관 조용히 끝낸다 — 설치·업데이트마다 윈도우 안내를 찍지 않는다.
    if !ctx.settings.parent().is_some_and(Path::is_dir) {
        return Outcome::NotInstalled;
    }
    let Some(cmd) = cmd else {
        return Outcome::UnsafePath(ctx.pack_dir.display().to_string());
    };
    let loaded = match load(ctx.settings) {
        Ok(l) => l,
        Err(e) => return Outcome::Refused(e),
    };
    let slot = loaded.as_ref().and_then(|l| l.3.as_ref()).map_or(Slot::Absent, classify);
    match slot {
        Slot::OursAuto { .. } => {
            if !ctx.record.exists() {
                write_record(ctx.record, ctx.settings); // 기록만 보충(설정 파일 무접촉)
            }
            return Outcome::AlreadyLinked(slot);
        }
        Slot::CysManual { .. } => return Outcome::AlreadyLinked(slot),
        Slot::User => return Outcome::UserOwned,
        Slot::Absent | Slot::Empty => {}
    }
    if !force && ctx.record.exists() {
        return Outcome::PreviouslyLinked;
    }
    // 연결 명령이 부를 래퍼가 팩에 실제로 있어야 한다 — 없는 파일을 부르는 상태줄은 agy 화면에 오류를 찍다가 스스로
    // 꺼진다(agy 1.2.9 `Statusline disabled after %d consecutive failures`). 설치 경로는 팩 파일을 다 쓴 뒤에 여기에
    // 오므로 정상 설치에서는 늘 있다(이상 설치·손으로 지운 팩에서만 걸린다).
    let script = script_name(ctx.windows);
    if !ctx.pack_dir.join("hooks").join(script).is_file() {
        return Outcome::Refused(format!(
            "팩에 상태줄 래퍼(hooks/{script})가 없어 연결하지 않았다 — `cys init-pack` 뒤 `cys doctor --fix`"
        ));
    }
    // 윈도우: 쓰기 직전 실연 검사 — 그 명령이 agy 의 `cmd /c` 아래에서 정말 cys 까지 닿는지 본다. 실패는 결과 하나일 뿐
    // (설정 파일 무변경 · 설치 계속). 검사 수단이 없으면(`None`) 쓰지 않는다.
    if ctx.windows {
        let verdict = match ctx.probe {
            Some(p) => p(&cmd),
            None => Err("실연 검사 수단이 없다".to_string()),
        };
        if let Err(reason) = verdict {
            let command = cmd.strip_suffix(&format!(" {MARKER}")).unwrap_or(&cmd).to_string();
            return Outcome::WindowsProbeFailed { reason, command };
        }
    }
    let want = desired_value(&cmd);
    let (orig_raw, new_bytes, mode, created) = match &loaded {
        None => {
            let body = match render_linked("{}\n", &cmd) {
                Ok(b) => b,
                Err(e) => return Outcome::Refused(e),
            };
            if let Err(e) = verify_render(&serde_json::json!({}), &body, Some(&want)) {
                return Outcome::Refused(e);
            }
            (None, body.into_bytes(), Some(0o600), true)
        }
        Some((raw, text, bom, v)) => {
            if let Err(e) = probe_writable(ctx.settings) {
                return Outcome::Refused(e);
            }
            let (src, orig) = match v {
                Some(v) => (text.as_str(), v.clone()),
                None => ("{}\n", serde_json::json!({})), // 빈 파일 — 새 본문으로
            };
            let body = match render_linked(src, &cmd) {
                Ok(b) => b,
                Err(e) => return Outcome::Refused(e),
            };
            if let Err(e) = verify_render(&orig, &body, Some(&want)) {
                return Outcome::Refused(e);
            }
            if let Err(e) = backup(ctx.settings, raw, &Backup::Beside) {
                return Outcome::Refused(e);
            }
            let mut bytes = Vec::with_capacity(body.len() + 3);
            if *bom {
                bytes.extend_from_slice(b"\xef\xbb\xbf");
            }
            bytes.extend_from_slice(body.as_bytes());
            (Some(raw.clone()), bytes, file_mode(ctx.settings), false)
        }
    };
    match commit(ctx.settings, orig_raw.as_deref(), &new_bytes, mode) {
        Ok(Slot::OursAuto { .. }) => {
            write_record(ctx.record, ctx.settings);
            Outcome::Linked { created }
        }
        Ok(other) => Outcome::Refused(format!("되읽기 불일치: {other:?}")),
        Err(e) => Outcome::Refused(e),
    }
}

/// cys 가 넣은(표지 달린) 연결만 뺀다. 표지 없는 cys 연결·사용자 설정은 건드리지 않는다. `record` 가 주어지면
/// 뺀 뒤 '연결한 적 있음' 기록도 지운다(노브를 다시 켜면 다시 연결되게).
pub fn unlink(settings: &Path, record: Option<&Path>, how: Backup) -> Outcome {
    let loaded = match load(settings) {
        Ok(Some(l)) => l,
        Ok(None) => {
            if let Some(r) = record {
                let _ = std::fs::remove_file(r);
            }
            return Outcome::NothingToUnlink(None);
        }
        Err(e) => return Outcome::Refused(e),
    };
    let (raw, text, bom, v) = loaded;
    let slot = v.as_ref().map_or(Slot::Absent, classify);
    let Some(v) = v.filter(|_| matches!(slot, Slot::OursAuto { .. })) else {
        if let Some(r) = record {
            if matches!(slot, Slot::Absent | Slot::Empty) {
                let _ = std::fs::remove_file(r);
            }
        }
        return Outcome::NothingToUnlink(Some(slot));
    };
    if let Err(e) = probe_writable(settings) {
        return Outcome::Refused(e);
    }
    let body = match render_unlinked(&text) {
        Ok(b) => b,
        Err(e) => return Outcome::Refused(e),
    };
    if let Err(e) = verify_render(&v, &body, None) {
        return Outcome::Refused(e);
    }
    if let Err(e) = backup(settings, &raw, &how) {
        return Outcome::Refused(e);
    }
    let mut bytes = Vec::with_capacity(body.len() + 3);
    if bom {
        bytes.extend_from_slice(b"\xef\xbb\xbf");
    }
    bytes.extend_from_slice(body.as_bytes());
    match commit(settings, Some(&raw), &bytes, file_mode(settings)) {
        Ok(Slot::OursAuto { .. }) => Outcome::Refused("되읽기: 연결이 그대로 남아 있다".into()),
        Ok(_) => {
            if let Some(r) = record {
                let _ = std::fs::remove_file(r);
            }
            Outcome::Unlinked
        }
        // 되읽기 실패(그 사이 다른 쓰기 등) — 성공이라 적지 않는다.
        Err(e) => Outcome::Refused(e),
    }
}

/// 사람용 한 줄(로그·doctor 공용). `None` = 알릴 것 없음(이미 연결 등 정상 무동작).
pub fn describe(o: &Outcome, settings: &Path) -> Option<String> {
    let p = settings.display();
    Some(match o {
        Outcome::NotInstalled | Outcome::AlreadyLinked(_) | Outcome::NothingToUnlink(_) => return None,
        Outcome::WindowsProbeFailed { reason, command } => format!(
            "Antigravity 사용량 자동 연결을 하지 않았습니다 — 연결 명령을 시험 실행했지만 통과하지 못했습니다({reason}). \
             {p} 는 그대로입니다 · 다시 시도: `cys doctor --fix` · 직접 넣기: statusLine command = `{command}`(사용 설명서 agy 절)"
        ),
        Outcome::UnsafePath(pack) => format!(
            "팩 경로({pack})에 공백·따옴표 등이 있어 안전한 연결 명령을 만들 수 없어 연결하지 않았습니다 — 사용 설명서 agy 절"
        ),
        Outcome::UserOwned => format!(
            "{p} 에 사용자가 설정한 statusLine 이 있어 덮지 않았습니다 — agy 쿼터 값은 들어오지 않습니다(사용 설명서 agy 절)"
        ),
        Outcome::PreviouslyLinked => format!(
            "{p} 의 statusLine 이 전에 cys 가 연결했던 칸인데 지금 비어 있어(agy 의 /statusline delete 등) 다시 넣지 \
             않았습니다 — 다시 연결하려면 `cys doctor --fix`"
        ),
        Outcome::Linked { created } => format!(
            "{p} 에 cys 상태줄 연결을 넣었습니다{} — 이미 떠 있는 agy 는 다시 켜야 적용될 수 있습니다 · 끄기: {ENV_KNOB}=0 \
             또는 ~/.cys/{OFF_FILE}",
            if *created { "(파일 새로 만듦)" } else { "" }
        ),
        Outcome::Unlinked => format!("{p} 에서 cys 가 넣은 상태줄 연결을 뺐습니다(백업 {BACKUP_SUFFIX})"),
        Outcome::Refused(why) => format!("{p} 를 건드리지 않았습니다 — {why}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 가짜 홈 샌드박스 — 테스트는 실 HOME·~/.gemini 를 절대 만지지 않는다.
    struct Sandbox {
        root: PathBuf,
    }
    impl Sandbox {
        fn new(tag: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "cys-agy-sl-{tag}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
            ));
            std::fs::create_dir_all(root.join("home")).unwrap();
            let sb = Sandbox { root };
            // 설치된 팩처럼 래퍼 스크립트를 둔다(연결 전제 — `missing_wrapper_script_is_not_linked`)
            std::fs::create_dir_all(sb.script().parent().unwrap()).unwrap();
            std::fs::write(sb.script(), "#!/bin/sh\nexit 0\n").unwrap();
            sb
        }
        fn home(&self) -> PathBuf {
            self.root.join("home")
        }
        fn script(&self) -> PathBuf {
            self.pack().join("hooks").join(SCRIPT)
        }
        fn settings(&self) -> PathBuf {
            settings_path_under(&self.home())
        }
        fn pack(&self) -> PathBuf {
            self.home().join(".cys").join("pack")
        }
        fn record(&self) -> PathBuf {
            self.pack().join(RECORD_REL)
        }
        fn agy_dir(&self) {
            std::fs::create_dir_all(self.settings().parent().unwrap()).unwrap();
        }
        fn put(&self, body: &[u8]) {
            self.agy_dir();
            std::fs::write(self.settings(), body).unwrap();
        }
        fn read(&self) -> String {
            std::fs::read_to_string(self.settings()).unwrap()
        }
        fn ensure(&self, force: bool) -> Outcome {
            let (s, p, r) = (self.settings(), self.pack(), self.record());
            ensure_linked(&Ctx { settings: &s, pack_dir: &p, record: &r, windows: false, probe: None }, force)
        }
        /// 윈도우 규칙으로 조정(검사 주입) — 래퍼 `.cmd` 를 함께 둔다(설치된 윈도우 팩처럼).
        fn ensure_win(&self, force: bool, probe: Option<Probe>) -> Outcome {
            let cmd_script = self.pack().join("hooks").join(SCRIPT_CMD);
            if !cmd_script.exists() {
                std::fs::write(&cmd_script, "@echo off\ncys usage-report-stdin --agy 2>nul\nexit /b 0\n").unwrap();
            }
            let (s, p, r) = (self.settings(), self.pack(), self.record());
            ensure_linked_cmd(&Ctx { settings: &s, pack_dir: &p, record: &r, windows: true, probe }, force, Some(WIN_CMD.to_string()))
        }
        fn want_cmd(&self) -> String {
            link_command_for(&self.pack().to_string_lossy(), false, true).unwrap()
        }
    }
    impl Drop for Sandbox {
        fn drop(&mut self) {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(self.settings(), std::fs::Permissions::from_mode(0o600));
            }
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    /// agy 가 이 맥에서 실제로 쓴 모양(2026-09-24 판독 — 값은 합성) · 2칸 들여쓰기 · 끝 줄바꿈.
    const AGY_DEFAULT: &str = "{\n  \"enableTerminalSandbox\": false,\n  \"statusLine\": {\n    \"type\": \"\",\n    \"command\": \"\",\n    \"enabled\": false\n  },\n  \"trustedWorkspaces\": [\n    \"/Users/x/work\"\n  ]\n}\n";

    /// 윈도우 시험의 넣을 명령(맥 샌드박스 경로는 윈도우 규칙으로 명령을 못 만들므로 고정 꼴을 준다 — 파일은 샌드박스).
    const WIN_CMD: &str = r"C:\Users\x\.cys\pack\hooks\cys-agy-statusline.cmd --cys-autolink";

    // ───────── 순수 함수 ─────────

    #[test]
    fn command_strings_follow_os_rules() {
        assert_eq!(
            link_command_for("/Users/x/.cys/pack", false, true).as_deref(),
            Some("sh /Users/x/.cys/pack/hooks/cys-agy-statusline.sh --cys-autolink")
        );
        assert_eq!(
            link_command_for("/Users/x/.cys/pack/", false, false).as_deref(),
            Some("sh /Users/x/.cys/pack/hooks/cys-agy-statusline.sh")
        );
        // 윈도우: `.cmd` 직접 실행(인터프리터 없음) · 역슬래시 · 확장 접두 제거 · 따옴표 없음
        assert_eq!(
            link_command_for(r"C:\Users\x\.cys\pack", true, true).as_deref(),
            Some(r"C:\Users\x\.cys\pack\hooks\cys-agy-statusline.cmd --cys-autolink")
        );
        assert_eq!(
            link_command_for(r"\\?\C:\Users\x\.cys\pack\", true, false).as_deref(),
            Some(r"C:\Users\x\.cys\pack\hooks\cys-agy-statusline.cmd")
        );
        assert_eq!(
            link_command_for("C:/Users/x/.cys/pack", true, false).as_deref(),
            Some(r"C:\Users\x\.cys\pack\hooks\cys-agy-statusline.cmd"),
            "정슬래시 입력도 역슬래시로"
        );
        assert_eq!(
            link_command_for("//?/D:/cys-pack", true, true).as_deref(),
            Some(r"D:\cys-pack\hooks\cys-agy-statusline.cmd --cys-autolink")
        );
        // 윈도우 cmd 문법에서 뜻이 바뀌는 글자 · 공백 · 따옴표 · 비ASCII · UNC · 상대경로 → 명령을 만들지 않는다(안내만)
        for bad in [
            r"C:\Users\x\Kim Lee\.cys\pack",
            r"C:\Users\x\a%b\.cys\pack",
            r"C:\Users\x\a^b\.cys\pack",
            r"C:\Users\x\a&b\.cys\pack",
            r"C:\Users\x\a(b)\.cys\pack",
            r"C:\Users\x\a!b\.cys\pack",
            "C:\\Users\\x\\a\"b\\.cys\\pack",
            r"C:\Users\x\a'b\.cys\pack",
            r"C:\Users\x\a@b\.cys\pack",
            r"C:\Users\x\a+b\.cys\pack",
            r"C:\Users\x\a;b\.cys\pack",
            r"C:\Users\x\a,b\.cys\pack",
            r"C:\Users\홍길동\.cys\pack",
            r"\\server\share\pack",
            r"\\?\UNC\server\share\pack",
            r"Users\x\.cys\pack",
            r"C:Users\x\pack",
            "",
        ] {
            assert_eq!(link_command_for(bad, true, true), None, "{bad:?}");
        }
        assert_eq!(link_command_for(r"C:\Users\x\Kim Lee\.cys\pack", true, true), None);
        assert_eq!(link_command_for("/Users/x/a b/.cys/pack", false, true), None);
        assert_eq!(link_command_for("/Users/x/a'b/.cys/pack", false, true), None);
        assert_eq!(link_command_for(".cys/pack", false, true), None);
        assert_eq!(link_command_for(r"\\server\share\pack", true, true), None);
        assert_eq!(link_command_for(r"C:\Users\$x\.cys\pack", true, true), None);
        // 비ASCII 사용자 폴더: 유닉스 허용 · 윈도우 불허(코드페이지 불확실)
        assert!(link_command_for("/Users/홍길동/.cys/pack", false, true).is_some());
        assert_eq!(link_command_for(r"C:\Users\홍길동\.cys\pack", true, true), None);
    }

    #[test]
    fn settings_path_strings_for_both_oses() {
        assert_eq!(
            settings_path_display(r"C:\Users\x", true),
            r"C:\Users\x\.gemini\antigravity-cli\settings.json"
        );
        assert_eq!(
            settings_path_display("C:/Users/x/", true),
            r"C:\Users\x\.gemini\antigravity-cli\settings.json"
        );
        assert_eq!(settings_path_display("/Users/x", false), "/Users/x/.gemini/antigravity-cli/settings.json");
        assert_eq!(
            settings_path_under(Path::new("/h")),
            Path::new("/h").join(".gemini").join("antigravity-cli").join("settings.json")
        );
    }

    #[test]
    fn knob_is_zero_or_file() {
        assert!(knob_off(Some("0"), false));
        assert!(knob_off(Some(" 0 "), false));
        assert!(knob_off(None, true));
        assert!(!knob_off(None, false));
        assert!(!knob_off(Some("1"), false));
        assert!(!knob_off(Some(""), false));
    }

    #[test]
    fn classify_covers_every_shape() {
        assert_eq!(classify(&json!({})), Slot::Absent);
        assert_eq!(classify(&json!({"statusLine": null})), Slot::Empty);
        assert_eq!(classify(&json!({"statusLine": {}})), Slot::Empty);
        assert_eq!(classify(&json!({"statusLine": {"type": "", "command": "", "enabled": false}})), Slot::Empty);
        assert_eq!(classify(&json!({"statusLine": {"type": "command", "command": "  "}})), Slot::Empty);
        // 명령은 비었지만 사용자가 만진 흔적(padding 등) — 건드리지 않는다
        assert_eq!(classify(&json!({"statusLine": {"command": "", "padding": 1}})), Slot::User);
        assert_eq!(classify(&json!({"statusLine": {"type": "weird", "command": ""}})), Slot::User);
        assert_eq!(classify(&json!({"statusLine": "sh x.sh"})), Slot::User);
        assert_eq!(classify(&json!({"statusLine": {"type": "command", "command": "~/my.sh"}})), Slot::User);
        assert_eq!(
            classify(&json!({"statusLine": {"command": "sh /h/.cys/pack/hooks/cys-agy-statusline.sh --cys-autolink", "enabled": false}})),
            Slot::OursAuto { enabled: Some(false) }
        );
        assert_eq!(
            classify(&json!({"statusLine": {"command": "sh ~/.cys/pack/hooks/cys-agy-statusline.sh"}})),
            Slot::CysManual { enabled: None }
        );
        assert_eq!(
            classify(&json!({"statusLine": {"command": "sh ~/.cys/pack/hooks/cys-statusline.sh", "enabled": true}})),
            Slot::CysManual { enabled: Some(true) }
        );
        // 표지만 흉내 낸 남의 명령은 cys 것이 아니다
        assert_eq!(classify(&json!({"statusLine": {"command": "my.sh --cys-autolink"}})), Slot::User);
    }

    #[test]
    fn surgery_keeps_every_other_byte() {
        let cmd = "sh /h/.cys/pack/hooks/cys-agy-statusline.sh --cys-autolink";
        let out = render_linked(AGY_DEFAULT, cmd).unwrap();
        let want = "{\n  \"enableTerminalSandbox\": false,\n  \"statusLine\": {\n    \"type\": \"command\",\n    \"command\": \"sh /h/.cys/pack/hooks/cys-agy-statusline.sh --cys-autolink\",\n    \"enabled\": true,\n    \"stack_with_default\": true\n  },\n  \"trustedWorkspaces\": [\n    \"/Users/x/work\"\n  ]\n}\n";
        assert_eq!(out, want);
        // 되돌리면 statusLine 칸만 빠진다(나머지 바이트 동일)
        let back = render_unlinked(&out).unwrap();
        assert_eq!(back, "{\n  \"enableTerminalSandbox\": false,\n  \"trustedWorkspaces\": [\n    \"/Users/x/work\"\n  ]\n}\n");
        // 키 없음 → 마지막 멤버 뒤에 같은 들여쓰기로 붙는다(키 순서 보존 — 사전순 재정렬 없음)
        let src = "{\n    \"zeta\": 1,\n    \"alpha\": {\"x\": \"}\\\"{\"}\n}";
        let out2 = render_linked(src, cmd).unwrap();
        assert!(out2.starts_with("{\n    \"zeta\": 1,\n    \"alpha\": {\"x\": \"}\\\"{\"},\n    \"statusLine\": {\n        \"type\""), "{out2}");
        assert!(out2.ends_with("\n    }\n}"), "{out2}");
        assert_eq!(render_unlinked(&out2).unwrap(), src);
        // CRLF · 탭 들여쓰기
        let crlf = "{\r\n\t\"a\": 1\r\n}\r\n";
        let out3 = render_linked(crlf, cmd).unwrap();
        assert!(out3.contains(",\r\n\t\"statusLine\": {\r\n\t\t\"type\": \"command\",\r\n"), "{out3:?}");
        assert!(!out3.replace("\r\n", "").contains('\n'), "LF 가 섞이면 안 된다: {out3:?}");
        assert_eq!(render_unlinked(&out3).unwrap(), crlf);
        // 한 줄(압축) 파일은 한 줄로
        let out4 = render_linked("{\"a\":1}", cmd).unwrap();
        assert_eq!(out4, format!("{{\"a\":1,\"statusLine\":{{\"type\":\"command\",\"command\":\"{cmd}\",\"enabled\":true,\"stack_with_default\":true}}}}"));
        assert_eq!(render_unlinked(&out4).unwrap(), "{\"a\":1}");
        // 빈 객체
        let out5 = render_linked("{}\n", cmd).unwrap();
        assert!(out5.starts_with("{\n  \"statusLine\": {\n    \"type\": \"command\""), "{out5}");
        assert_eq!(render_unlinked(&out5).unwrap(), "{}\n");
        // statusLine 이 첫 멤버일 때의 제거
        let first = "{\n  \"statusLine\": null,\n  \"b\": 2\n}";
        let linked = render_linked(first, cmd).unwrap();
        assert_eq!(render_unlinked(&linked).unwrap(), "{\n  \"b\": 2\n}");
        // 중복 키는 모호 — 수술 거부
        assert!(render_linked("{\"statusLine\": null, \"statusLine\": {}}", cmd).is_err());
        assert!(render_unlinked("{\"statusLine\": null, \"statusLine\": {}}").is_err());
        // 사후 검증: 다른 키가 바뀌면 거부
        let orig: Value = serde_json::from_str(AGY_DEFAULT).unwrap();
        assert!(verify_render(&orig, &out, Some(&desired_value(cmd))).is_ok());
        assert!(verify_render(&orig, &out.replace("false,\n  \"statusLine", "true,\n  \"statusLine"), Some(&desired_value(cmd))).is_err());
    }

    // ───────── 샌드박스(가짜 홈) 입출력 ─────────

    #[test]
    fn missing_agy_dir_creates_nothing() {
        let sb = Sandbox::new("noagy");
        assert_eq!(sb.ensure(false), Outcome::NotInstalled);
        assert!(!sb.settings().parent().unwrap().exists(), "agy 가 없는 기계에 폴더를 만들면 안 된다");
        assert!(!sb.record().exists());
    }

    #[test]
    fn missing_file_is_created_0600_and_linked() {
        let sb = Sandbox::new("nofile");
        sb.agy_dir();
        assert_eq!(sb.ensure(false), Outcome::Linked { created: true });
        let v: Value = serde_json::from_str(&sb.read()).unwrap();
        assert_eq!(v["statusLine"], desired_value(&sb.want_cmd()));
        assert!(sb.record().exists(), "연결 기록이 남아야 한다");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(sb.settings()).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "새 설정 파일은 0600(agy 실측 권한)");
        }
        assert!(!PathBuf::from(format!("{}{BACKUP_SUFFIX}", sb.settings().display())).exists(), "없던 파일은 백업할 것도 없다");
    }

    #[test]
    fn empty_file_is_linked() {
        let sb = Sandbox::new("empty");
        sb.put(b"");
        assert_eq!(sb.ensure(false), Outcome::Linked { created: false });
        assert_eq!(classify(&serde_json::from_str(&sb.read()).unwrap()), Slot::OursAuto { enabled: Some(true) });
    }

    #[test]
    fn agy_default_shape_is_linked_with_backup_mode_and_idempotent() {
        let sb = Sandbox::new("default");
        sb.put(AGY_DEFAULT.as_bytes());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(sb.settings(), std::fs::Permissions::from_mode(0o640)).unwrap();
        }
        assert_eq!(sb.ensure(false), Outcome::Linked { created: false });
        let after = sb.read();
        assert_eq!(render_unlinked(&after).unwrap(), render_unlinked(&render_linked(AGY_DEFAULT, &sb.want_cmd()).unwrap()).unwrap());
        let bak = std::fs::read_to_string(format!("{}{BACKUP_SUFFIX}", sb.settings().display())).unwrap();
        assert_eq!(bak, AGY_DEFAULT, "백업은 쓰기 직전 원본이어야 한다");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(sb.settings()).unwrap().permissions().mode() & 0o777, 0o640, "원 권한 유지");
        }
        // 멱등: 두 번째는 무동작(파일 바이트·백업 무변경)
        let mtime = std::fs::metadata(sb.settings()).unwrap().modified().unwrap();
        assert!(matches!(sb.ensure(false), Outcome::AlreadyLinked(Slot::OursAuto { .. })));
        assert_eq!(sb.read(), after);
        assert_eq!(std::fs::metadata(sb.settings()).unwrap().modified().unwrap(), mtime);
    }

    #[test]
    fn bom_is_preserved() {
        let sb = Sandbox::new("bom");
        let mut body = b"\xef\xbb\xbf".to_vec();
        body.extend_from_slice(b"{\n  \"a\": 1\n}\n");
        sb.put(&body);
        assert_eq!(sb.ensure(false), Outcome::Linked { created: false });
        let raw = std::fs::read(sb.settings()).unwrap();
        assert!(raw.starts_with(b"\xef\xbb\xbf"), "BOM 이 사라졌다");
        assert_eq!(std::str::from_utf8(&raw[3..]).unwrap(), render_linked("{\n  \"a\": 1\n}\n", &sb.want_cmd()).unwrap());
    }

    #[test]
    fn user_statusline_is_never_overwritten() {
        let sb = Sandbox::new("user");
        let body = "{\n  \"statusLine\": {\"type\": \"command\", \"command\": \"~/.gemini/antigravity-cli/statusline.sh\"}\n}\n";
        sb.put(body.as_bytes());
        assert_eq!(sb.ensure(false), Outcome::UserOwned);
        assert_eq!(sb.ensure(true), Outcome::UserOwned, "force 도 사용자 설정을 덮지 않는다");
        assert_eq!(sb.read(), body);
        assert!(!sb.record().exists());
        // 사용자가 직접 넣은 cys 연결(표지 없음)은 이미 연결 — 무동작 · 제거 대상도 아니다
        let manual = "{\"statusLine\": {\"type\": \"command\", \"command\": \"sh ~/.cys/pack/hooks/cys-agy-statusline.sh\", \"enabled\": true}}";
        sb.put(manual.as_bytes());
        assert!(matches!(sb.ensure(false), Outcome::AlreadyLinked(Slot::CysManual { .. })));
        assert!(matches!(unlink(&sb.settings(), Some(&sb.record()), Backup::Beside), Outcome::NothingToUnlink(Some(Slot::CysManual { .. }))));
        assert_eq!(sb.read(), manual);
    }

    #[test]
    fn broken_json_and_non_object_are_refused() {
        let sb = Sandbox::new("broken");
        for body in ["{\"statusLine\": ", "[1,2]", "{\"a\": 1} trailing", "// comment\n{}"] {
            sb.put(body.as_bytes());
            assert!(matches!(sb.ensure(false), Outcome::Refused(_)), "{body}");
            assert_eq!(sb.read(), body, "깨진 파일은 한 바이트도 바뀌면 안 된다");
        }
        sb.put(b"\xff\xfe{\x00}\x00"); // UTF-16
        assert!(matches!(sb.ensure(false), Outcome::Refused(_)));
        assert!(!sb.record().exists());
    }

    #[cfg(unix)]
    #[test]
    fn read_only_file_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let sb = Sandbox::new("ro");
        sb.put(AGY_DEFAULT.as_bytes());
        std::fs::set_permissions(sb.settings(), std::fs::Permissions::from_mode(0o400)).unwrap();
        let o = sb.ensure(false);
        assert!(matches!(&o, Outcome::Refused(w) if w.contains("읽기 전용")), "{o:?}");
        assert_eq!(sb.read(), AGY_DEFAULT);
        assert!(!PathBuf::from(format!("{}{BACKUP_SUFFIX}", sb.settings().display())).exists());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_settings_are_refused() {
        let sb = Sandbox::new("link");
        sb.agy_dir();
        let real = sb.root.join("dotfiles-settings.json");
        std::fs::write(&real, AGY_DEFAULT).unwrap();
        std::os::unix::fs::symlink(&real, sb.settings()).unwrap();
        assert!(matches!(sb.ensure(false), Outcome::Refused(_)));
        assert_eq!(std::fs::read_to_string(&real).unwrap(), AGY_DEFAULT);
        assert!(std::fs::symlink_metadata(sb.settings()).unwrap().file_type().is_symlink(), "링크를 일반 파일로 바꾸면 안 된다");
    }

    #[test]
    fn user_removal_is_respected_until_forced() {
        let sb = Sandbox::new("seedonce");
        sb.put(AGY_DEFAULT.as_bytes());
        assert_eq!(sb.ensure(false), Outcome::Linked { created: false });
        // 사용자가 agy 안에서 /statusline delete — agy 기본형으로 돌아간다
        sb.put(AGY_DEFAULT.as_bytes());
        assert_eq!(sb.ensure(false), Outcome::PreviouslyLinked);
        assert_eq!(sb.read(), AGY_DEFAULT, "지운 것을 설치가 되살리면 안 된다");
        assert_eq!(sb.ensure(true), Outcome::Linked { created: false }, "사람이 부른 doctor --fix 는 다시 연결한다");
    }

    #[test]
    fn unlink_removes_only_our_marker_and_clears_record() {
        let sb = Sandbox::new("unlink");
        sb.put(AGY_DEFAULT.as_bytes());
        assert_eq!(sb.ensure(false), Outcome::Linked { created: false });
        assert_eq!(unlink(&sb.settings(), Some(&sb.record()), Backup::Beside), Outcome::Unlinked);
        let v: Value = serde_json::from_str(&sb.read()).unwrap();
        assert!(v.get("statusLine").is_none());
        assert_eq!(v["trustedWorkspaces"], json!(["/Users/x/work"]));
        assert!(!sb.record().exists(), "노브를 다시 켜면 다시 연결되도록 기록을 지운다");
        // 두 번째 제거는 무동작
        assert!(matches!(unlink(&sb.settings(), Some(&sb.record()), Backup::Beside), Outcome::NothingToUnlink(Some(Slot::Absent))));
        // 파일 없음 · agy 없음
        let sb2 = Sandbox::new("unlink-none");
        assert_eq!(unlink(&sb2.settings(), None, Backup::Beside), Outcome::NothingToUnlink(None));
        // 사용자 설정은 제거하지 않는다
        let body = "{\"statusLine\": {\"command\": \"my.sh\"}}";
        sb.put(body.as_bytes());
        assert_eq!(unlink(&sb.settings(), None, Backup::Beside), Outcome::NothingToUnlink(Some(Slot::User)));
        assert_eq!(sb.read(), body);
        // 격리 폴더 백업(완전 초기화 규약)
        sb.put(AGY_DEFAULT.as_bytes());
        let _ = std::fs::remove_file(sb.record());
        assert_eq!(sb.ensure(false), Outcome::Linked { created: false });
        let trash = sb.root.join("trash");
        assert_eq!(unlink(&sb.settings(), None, Backup::Dir(&trash)), Outcome::Unlinked);
        assert!(trash.join("agy-antigravity-cli.settings.json").is_file());
    }

    /// 윈도우: 실연 검사 통과 → 유닉스와 같은 쓰기 경로(백업·원자 쓰기·되읽기) · 넣는 명령은 `.cmd` 꼴 · 검사는 표지 달린
    /// 그 명령 그대로를 받는다 · 이미 연결이면 검사도 부르지 않는다(설치마다 자식 프로세스를 띄우지 않는다).
    #[test]
    fn windows_probe_pass_links_with_cmd_wrapper() {
        let sb = Sandbox::new("winok");
        sb.put(AGY_DEFAULT.as_bytes());
        let seen = std::cell::RefCell::new(Vec::<String>::new());
        let pass = |c: &str| {
            seen.borrow_mut().push(c.to_string());
            Ok(())
        };
        assert_eq!(sb.ensure_win(false, Some(&pass)), Outcome::Linked { created: false });
        let want = link_command_for(r"C:\Users\x\.cys\pack", true, true);
        assert_eq!(want.as_deref(), Some(WIN_CMD));
        let v: Value = serde_json::from_str(&sb.read()).unwrap();
        let put = v["statusLine"]["command"].as_str().unwrap().to_string();
        assert_eq!(*seen.borrow(), vec![put.clone()], "검사는 넣을 명령 그대로를 한 번 받는다");
        assert!(put.ends_with(&format!("{SCRIPT_CMD} {MARKER}")), "{put}");
        assert!(!put.starts_with("bash") && !put.contains('"'), "인터프리터·따옴표 없음: {put}");
        assert_eq!(Some(put.clone()), want);
        assert!(command_is_ours_auto(&put));
        let bak = std::fs::read_to_string(format!("{}{BACKUP_SUFFIX}", sb.settings().display())).unwrap();
        assert_eq!(bak, AGY_DEFAULT);
        assert!(sb.record().exists());
        // 멱등 — 두 번째는 검사도 부르지 않는다
        assert!(matches!(sb.ensure_win(false, Some(&pass)), Outcome::AlreadyLinked(Slot::OursAuto { .. })));
        assert_eq!(seen.borrow().len(), 1);
        // 노브(제거)는 윈도우 연결도 뺀다
        assert_eq!(unlink(&sb.settings(), Some(&sb.record()), Backup::Beside), Outcome::Unlinked);
        assert!(serde_json::from_str::<Value>(&sb.read()).unwrap().get("statusLine").is_none());
    }

    /// 윈도우: 실연 검사 실패 · 검사 수단 없음 → 설정 파일 **바이트 동일** · 백업·기록 없음 · 안내에 붙여 넣을 `.cmd` 명령.
    #[test]
    fn windows_probe_failure_leaves_file_byte_identical() {
        let sb = Sandbox::new("winfail");
        sb.put(AGY_DEFAULT.as_bytes());
        let before = std::fs::read(sb.settings()).unwrap();
        let mtime = std::fs::metadata(sb.settings()).unwrap().modified().unwrap();
        let fail = |_: &str| Err("종료 코드 Some(1)".to_string());
        for probe in [Some(&fail as Probe), None] {
            let o = sb.ensure_win(true, probe);
            match &o {
                Outcome::WindowsProbeFailed { reason, command } => {
                    assert!(!reason.is_empty());
                    assert!(command.ends_with(SCRIPT_CMD) && !command.contains(MARKER), "직접 넣는 명령은 표지 없음: {command}");
                    let line = describe(&o, &sb.settings()).unwrap();
                    assert!(line.contains(command.as_str()) && line.contains("cys doctor --fix"), "{line}");
                }
                other => panic!("검사 실패인데 {other:?}"),
            }
            assert_eq!(std::fs::read(sb.settings()).unwrap(), before, "검사 실패는 한 바이트도 바꾸면 안 된다");
            assert_eq!(std::fs::metadata(sb.settings()).unwrap().modified().unwrap(), mtime);
            assert!(!PathBuf::from(format!("{}{BACKUP_SUFFIX}", sb.settings().display())).exists());
            assert!(!sb.record().exists());
        }
        // 파일이 없을 때도 만들지 않는다
        std::fs::remove_file(sb.settings()).unwrap();
        assert!(matches!(sb.ensure_win(false, Some(&fail)), Outcome::WindowsProbeFailed { .. }));
        assert!(!sb.settings().exists(), "검사 실패에 새 파일을 만들면 안 된다");
    }

    /// 윈도우: agy 가 없는 기계(대다수)는 조용히 끝나고 검사도 부르지 않는다 · 직접 넣은 cys 연결(옛 `bash …sh` 포함)과
    /// 사용자 설정은 검사 없이 무동작 · `.cmd` 래퍼가 팩에 없으면 검사 전에 거절.
    #[test]
    fn windows_is_quiet_without_agy_and_respects_existing_links() {
        let sb = Sandbox::new("winq");
        let called = std::cell::Cell::new(0u32);
        let probe = |_: &str| {
            called.set(called.get() + 1);
            Ok(())
        };
        assert_eq!(sb.ensure_win(false, Some(&probe)), Outcome::NotInstalled, "agy 미설치 윈도우에 아무것도 하지 않는다");
        assert!(!sb.settings().parent().unwrap().exists());
        let manual = "{\"statusLine\": {\"type\": \"command\", \"command\": \"bash C:/Users/x/.cys/pack/hooks/cys-agy-statusline.sh\"}}";
        sb.put(manual.as_bytes());
        assert!(matches!(sb.ensure_win(true, Some(&probe)), Outcome::AlreadyLinked(Slot::CysManual { .. })), "직접 넣은 cys 연결은 무동작");
        assert_eq!(sb.read(), manual);
        assert!(describe(&sb.ensure_win(false, Some(&probe)), &sb.settings()).is_none());
        let manual_cmd = r#"{"statusLine": {"type": "command", "command": "C:\\Users\\x\\.cys\\pack\\hooks\\cys-agy-statusline.cmd"}}"#;
        sb.put(manual_cmd.as_bytes());
        assert!(matches!(sb.ensure_win(true, Some(&probe)), Outcome::AlreadyLinked(Slot::CysManual { .. })));
        let user = "{\"statusLine\": {\"command\": \"mine.cmd\"}}";
        sb.put(user.as_bytes());
        assert_eq!(sb.ensure_win(true, Some(&probe)), Outcome::UserOwned);
        assert_eq!(sb.read(), user);
        assert_eq!(called.get(), 0, "쓰지 않는 갈래에서 검사를 부르면 안 된다");
        // `.cmd` 래퍼 부재 → 검사 전에 거절(없는 파일을 부르는 상태줄은 agy 가 연속 실패로 스스로 끈다)
        sb.put(AGY_DEFAULT.as_bytes());
        let (s, p, r) = (sb.settings(), sb.pack(), sb.record());
        let _ = std::fs::remove_file(sb.pack().join("hooks").join(SCRIPT_CMD));
        let o = ensure_linked_cmd(&Ctx { settings: &s, pack_dir: &p, record: &r, windows: true, probe: Some(&probe) }, true, Some(WIN_CMD.into()));
        assert!(matches!(&o, Outcome::Refused(w) if w.contains(SCRIPT_CMD)), "{o:?}");
        assert_eq!(called.get(), 0);
        assert_eq!(sb.read(), AGY_DEFAULT);
    }

    /// 연결 명령이 부를 래퍼가 팩에 없으면 연결하지 않는다 — 없는 파일을 부르는 상태줄은 agy 화면에 오류를 찍다가
    /// 스스로 꺼진다(agy 1.2.9 `Statusline disabled after %d consecutive failures`). 이미 된 연결·사용자 설정은 이 검사와
    /// 무관하다(쓰지 않으므로).
    #[test]
    fn missing_wrapper_script_is_not_linked() {
        let sb = Sandbox::new("noscript");
        sb.put(AGY_DEFAULT.as_bytes());
        std::fs::remove_file(sb.script()).unwrap();
        let o = sb.ensure(true);
        assert!(matches!(&o, Outcome::Refused(w) if w.contains(SCRIPT)), "{o:?}");
        assert_eq!(sb.read(), AGY_DEFAULT);
        assert!(!sb.record().exists());
        assert!(!PathBuf::from(format!("{}{BACKUP_SUFFIX}", sb.settings().display())).exists());
        // 래퍼가 디렉터리(이상 설치)여도 연결하지 않는다
        std::fs::create_dir_all(sb.script()).unwrap();
        assert!(matches!(sb.ensure(true), Outcome::Refused(_)));
        assert_eq!(sb.read(), AGY_DEFAULT);
    }

    #[test]
    fn unsafe_pack_path_is_not_linked() {
        let sb = Sandbox::new("unsafe");
        sb.put(AGY_DEFAULT.as_bytes());
        let (s, r) = (sb.settings(), sb.record());
        let p = sb.root.join("with space").join("pack");
        assert!(matches!(ensure_linked(&Ctx { settings: &s, pack_dir: &p, record: &r, windows: false, probe: None }, false), Outcome::UnsafePath(_)));
        assert_eq!(sb.read(), AGY_DEFAULT);
    }

    /// 백업은 **판독·검증한 바이트** 그대로다 — 판독과 백업 사이에 다른 쓰기(업그레이드 직후 앱과 데몬이 동시에 설치하는
    /// 경우 등)가 끼어도 백업이 남의 결과로 바뀌지 않는다(그 쓰기는 뒤의 재판독 대조가 막는다). 원 권한도 따른다.
    #[test]
    fn backup_is_the_bytes_that_were_read() {
        let sb = Sandbox::new("bakraw");
        sb.put(AGY_DEFAULT.as_bytes());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(sb.settings(), std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        let raw = std::fs::read(sb.settings()).unwrap();
        std::fs::write(sb.settings(), "{\"changed\": true}").unwrap(); // 판독 뒤 누군가 바꿨다
        backup(&sb.settings(), &raw, &Backup::Beside).unwrap();
        let bak = PathBuf::from(format!("{}{BACKUP_SUFFIX}", sb.settings().display()));
        assert_eq!(std::fs::read(&bak).unwrap(), raw, "백업이 판독한 원본이 아니다");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&bak).unwrap().permissions().mode() & 0o777, 0o600, "백업이 원본보다 넓게 열렸다");
        }
        let trash = sb.root.join("trash2");
        backup(&sb.settings(), &raw, &Backup::Dir(&trash)).unwrap();
        assert_eq!(std::fs::read(trash.join("agy-antigravity-cli.settings.json")).unwrap(), raw);
    }

    #[test]
    fn concurrent_change_between_read_and_write_is_refused() {
        let sb = Sandbox::new("cas");
        sb.put(AGY_DEFAULT.as_bytes());
        let raw = std::fs::read(sb.settings()).unwrap();
        std::fs::write(sb.settings(), AGY_DEFAULT.replace("false,\n  \"statusLine", "true,\n  \"statusLine")).unwrap();
        let r = commit(&sb.settings(), Some(&raw), b"{}", None);
        assert!(r.is_err(), "재판독이 다르면 쓰지 않는다");
        assert!(sb.read().contains("\"enableTerminalSandbox\": true"));
    }

    /// Go `syscall.EscapeArg`(windows) 재현 — agy 가 만드는 명령줄과 같아야 실연 검사가 agy 를 대변한다.
    #[test]
    fn go_escape_arg_matches_go_rules() {
        let table: &[(&str, &str)] = &[
            ("", r#""""#),
            ("a", "a"),
            (" ", r#"" ""#),
            ("\t", "\"\t\""),
            (r"\", r"\"),
            (r#"""#, r#"\""#),
            (r#"\""#, r#"\\\""#),
            (r#"\\""#, r#"\\\\\""#),
            ("a b", r#""a b""#),
            (r"\\ ", r#""\\ ""#),
            (r" \\", r#"" \\\\""#),
            (r"C:\", r"C:\"),
            (r"C:\Users\x\Gopher\", r"C:\Users\x\Gopher\"),
            (r"C:\Program Files (x32)\Common\", r#""C:\Program Files (x32)\Common\\""#),
            (r#"C:\Users\x\Gopher\""#, r#"C:\Users\x\Gopher\\\""#),
            // 우리 명령: 공백이 있어 통째로 따옴표 — cmd 가 `/c "…"` 의 첫·끝 따옴표를 벗겨 그대로 실행한다
            (WIN_CMD, r#""C:\Users\x\.cys\pack\hooks\cys-agy-statusline.cmd --cys-autolink""#),
            (r"C:\p\hooks\cys-agy-statusline.cmd", r"C:\p\hooks\cys-agy-statusline.cmd"),
            // 따옴표 두른 명령은 `\"` 로 바뀐다 — 공개 보고(따옴표가 글자 그대로 넘어가 경로가 깨짐)의 기전
            (r#"sh "C:/a b/x.sh""#, r#""sh \"C:/a b/x.sh\"""#),
        ];
        for (input, want) in table {
            assert_eq!(go_escape_arg(input), *want, "입력 {input:?}");
        }
        // 넣는 명령에는 따옴표가 없다 → 인용 결과 안의 따옴표는 감싼 두 개뿐이다(cmd `/c` 의 '정확히 두 개' 규칙)
        assert_eq!(go_escape_arg(WIN_CMD).matches('"').count(), 2);
    }

    /// `.cmd` 연결의 소유 판정 — 표지 달린 `.cmd` 는 cys 자동 연결(제거 대상) · 표지 없는 `.cmd` 는 직접 넣은 cys 연결 ·
    /// 옛 `bash …sh` 는 '윈도우 옛 꼴'(안내 대상) · 표지만 흉내 낸 남의 `.cmd` 는 사용자 것.
    #[test]
    fn cmd_wrapper_ownership_is_recognized() {
        assert!(command_is_ours_auto(WIN_CMD));
        assert!(command_is_ours_auto(r"C:\USERS\X\.CYS\PACK\HOOKS\CYS-AGY-STATUSLINE.CMD --cys-autolink"), "윈도우 파일 이름은 대소문자 무관");
        assert_eq!(classify(&json!({"statusLine": {"command": WIN_CMD}})), Slot::OursAuto { enabled: None });
        assert_eq!(
            classify(&json!({"statusLine": {"command": r"C:\Users\x\.cys\pack\hooks\cys-agy-statusline.cmd", "enabled": true}})),
            Slot::CysManual { enabled: Some(true) }
        );
        assert_eq!(classify(&json!({"statusLine": {"command": r"C:\mine\cys-agy-statusline.cmd.bak --cys-autolink"}})), Slot::CysManual { enabled: None });
        assert_eq!(classify(&json!({"statusLine": {"command": r"C:\tools\mine.cmd --cys-autolink"}})), Slot::User);
        assert!(command_is_legacy_windows_sh("bash C:/Users/x/.cys/pack/hooks/cys-agy-statusline.sh"));
        assert!(!command_is_legacy_windows_sh(r"C:\Users\x\.cys\pack\hooks\cys-agy-statusline.cmd"));
        assert!(!command_is_legacy_windows_sh("mine.cmd"));
        // 수술 결과 JSON 안에서 역슬래시는 이스케이프되고, 되읽으면 같은 명령이다
        let out = render_linked(AGY_DEFAULT, WIN_CMD).unwrap();
        assert!(out.contains(r#""command": "C:\\Users\\x\\.cys\\pack\\hooks\\cys-agy-statusline.cmd --cys-autolink""#), "{out}");
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["statusLine"], desired_value(WIN_CMD));
        // 제거는 OS 무관 — `.cmd` 표지 연결을 맥에서도 뺀다(완전 초기화 경로와 같은 함수)
        let sb = Sandbox::new("cmdunlink");
        sb.put(out.as_bytes());
        assert_eq!(unlink(&sb.settings(), None, Backup::Beside), Outcome::Unlinked);
        assert_eq!(render_unlinked(&out).unwrap(), sb.read());
    }

    /// 윈도우 규칙으로 안전한 명령을 못 만드는 팩 경로(맥 샌드박스 `/…` 도 그 하나) → `UnsafePath` · 검사를 부르지 않는다.
    #[test]
    fn windows_unsafe_pack_path_is_not_probed() {
        let sb = Sandbox::new("winunsafe");
        sb.put(AGY_DEFAULT.as_bytes());
        let called = std::cell::Cell::new(false);
        let probe = |_: &str| {
            called.set(true);
            Ok(())
        };
        let (s, p, r) = (sb.settings(), sb.pack(), sb.record());
        let o = ensure_linked(&Ctx { settings: &s, pack_dir: &p, record: &r, windows: true, probe: Some(&probe) }, true);
        assert!(matches!(o, Outcome::UnsafePath(_)), "{o:?}");
        assert!(!called.get());
        assert_eq!(sb.read(), AGY_DEFAULT);
    }

    #[test]
    fn probe_output_must_end_with_the_cys_line() {
        assert!(probe_output_ok("cys\n").is_ok());
        assert!(probe_output_ok("cys\r\n").is_ok());
        assert!(probe_output_ok("noise\ncys\n\n").is_ok());
        assert!(probe_output_ok("").is_err(), "아무것도 실행되지 않은 경우(빈 출력)는 실패다");
        assert!(probe_output_ok("'cys' is not recognized\n").is_err());
        assert!(probe_output_ok("cys\nerror\n").is_err());
        assert!(probe_output_ok("5h 12% · cys").is_err(), "빈 쿼터에는 `cys` 한 줄만 나와야 한다");
    }

    /// 검사 자식 실행기(OS 공용 부분) — 종료 코드·표준출력·stdin 전달·좌석 표지 제거·시간 상한(kill)을 맥에서 잰다.
    #[cfg(unix)]
    #[test]
    fn probe_spawn_reports_rc_output_and_timeout() {
        let sh = Path::new("/bin/sh");
        let run = |script: &'static str, t_ms: u64| {
            probe_spawn(
                sh,
                &|c| {
                    c.env("CYS_SURFACE_ID", "7").env("JAVIS_SURFACE_ID", "7").args(["-c", script]);
                },
                None,
                PROBE_INPUT.as_bytes(),
                std::time::Duration::from_millis(t_ms),
            )
        };
        // stdin 이 그대로 넘어가고 좌석 표지는 지워진다
        let out = run(r#"read l; [ -z "$CYS_SURFACE_ID$JAVIS_SURFACE_ID" ] && echo "$l""#, 5000).unwrap();
        assert_eq!(out.trim(), PROBE_INPUT);
        assert!(probe_output_ok(&run("cat >/dev/null; echo cys", 5000).unwrap()).is_ok());
        let e = run("echo cys; exit 3", 5000).unwrap_err();
        assert!(e.contains("종료 코드"), "{e}");
        let t0 = std::time::Instant::now();
        let e = run("sleep 30", 300).unwrap_err();
        assert!(e.contains("끝나지 않았다"), "{e}");
        assert!(t0.elapsed() < std::time::Duration::from_secs(5), "상한을 넘겨 기다렸다: {:?}", t0.elapsed());
        // stdin 을 읽지 않고 끝나는 자식도 실패가 아니다(파이프 오류 무시)
        assert!(probe_output_ok(&run("echo cys", 5000).unwrap()).is_ok());
        assert!(probe_spawn(Path::new("/nonexistent/cmd"), &|_| {}, None, b"", std::time::Duration::from_secs(1)).is_err());
        assert!(live_probe(WIN_CMD).is_err(), "유닉스에는 실연 검사가 없다(쓰기 경로에 쓰이면 안 된다)");
    }

    /// ★윈도우 실기: 팩의 LF `.cmd` 래퍼 사본을 agy 와 같은 방식(`cmd /c` + Go 인용)으로 부른다 — 가짜 `cys.cmd` 가 PATH 에
    /// 있으면 5초 안에 0 으로 끝나고 `cys` 줄이 나온다(통과) · PATH 에 cys 가 없으면 0 으로 끝나도 출력이 비어 실패다(구별).
    #[cfg(windows)]
    #[test]
    fn windows_cmd_wrapper_runs_under_cmd_c_like_agy() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(format!("agy-probe-win-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let pack = root.join("pack");
        let bin = root.join("fakebin");
        std::fs::create_dir_all(pack.join("hooks")).unwrap();
        std::fs::create_dir_all(&bin).unwrap();
        let wrapper = include_str!("../cysjavis-pack/hooks/cys-agy-statusline.cmd");
        assert!(!wrapper.contains('\r'), "팩 래퍼는 LF 로 출하된다(그 바이트 그대로 실행해 본다)");
        std::fs::write(pack.join("hooks").join(SCRIPT_CMD), wrapper).unwrap();
        // 가짜 cys: 인자를 확인하고 사람용 줄을 찍는다(진짜 cys 의 빈 쿼터 출력과 같은 줄)
        std::fs::write(
            bin.join("cys.cmd"),
            "@echo off\r\nif not \"%1 %2\"==\"usage-report-stdin --agy\" exit /b 9\r\necho cys\r\nexit /b 0\r\n",
        )
        .unwrap();
        let cmd = link_command_for(&pack.to_string_lossy(), true, true)
            .unwrap_or_else(|| panic!("시험 폴더 경로가 윈도우 안전 규칙을 통과하지 못했다(계측 무효): {}", pack.display()));
        let sysroot = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        let sys32 = format!(r"{sysroot}\System32");
        let with_cys = format!("{};{sys32}", bin.display());
        let t0 = std::time::Instant::now();
        let out = run_like_agy(&cmd, Some(std::ffi::OsStr::new(&with_cys))).expect("cmd /c 실행이 0 으로 끝나야 한다");
        assert!(t0.elapsed() < PROBE_TIMEOUT, "5초 상한 안이어야 한다: {:?}", t0.elapsed());
        assert!(probe_output_ok(&out).is_ok(), "출력: {out:?}");
        // PATH 에 cys 가 없으면: 래퍼는 0 으로 끝나지만 출력이 비어 검사는 실패한다
        let out2 = run_like_agy(&cmd, Some(std::ffi::OsStr::new(&sys32))).expect("래퍼는 언제나 exit 0");
        assert!(probe_output_ok(&out2).is_err(), "cys 없이도 통과했다: {out2:?}");
        // 실제 조정 흐름: 가짜 홈에서 검사 통과 → 연결
        let home = root.join("home");
        let settings = settings_path_under(&home);
        std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
        std::fs::write(&settings, AGY_DEFAULT).unwrap();
        let record = pack.join(RECORD_REL);
        let probe = |c: &str| probe_output_ok(&run_like_agy(c, Some(std::ffi::OsStr::new(&with_cys)))?);
        let o = ensure_linked(&Ctx { settings: &settings, pack_dir: &pack, record: &record, windows: true, probe: Some(&probe) }, false);
        assert_eq!(o, Outcome::Linked { created: false });
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
        assert_eq!(v["statusLine"]["command"].as_str(), Some(cmd.as_str()));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn describe_speaks_only_when_needed() {
        let s = Path::new("/h/settings.json");
        assert!(describe(&Outcome::AlreadyLinked(Slot::OursAuto { enabled: Some(true) }), s).is_none());
        assert!(describe(&Outcome::NotInstalled, s).is_none());
        assert!(describe(&Outcome::UserOwned, s).unwrap().contains("덮지 않았습니다"));
        let wf = describe(&Outcome::WindowsProbeFailed { reason: "r".into(), command: r"C:\p\hooks\cys-agy-statusline.cmd".into() }, s).unwrap();
        assert!(wf.contains("시험 실행") && wf.contains(r"C:\p\hooks\cys-agy-statusline.cmd") && !wf.contains("다음 판에서"), "{wf}");
        assert!(describe(&Outcome::Linked { created: false }, s).unwrap().contains("CYS_AGY_STATUSLINE=0"));
    }
}
