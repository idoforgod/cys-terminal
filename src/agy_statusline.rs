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
//! 유닉스: `sh <팩>/hooks/cys-agy-statusline.sh --cys-autolink`. 끝의 `--cys-autolink` 가 **cys 가 넣었다는 표지**다
//! (스크립트는 인자를 읽지 않는다). agy 가 settings.json 을 자기 구조체로 다시 써도 command 문자열은 남으므로 표지가
//! 사라지지 않는다(별도 키 표지는 agy 재기록에 지워질 수 있다). 제거(되돌리기 노브 · 완전 초기화)는 이 표지가 달린
//! 연결만 지운다 — 사용자가 매뉴얼을 보고 직접 넣은 cys 연결(표지 없음)은 건드리지 않는다.
//!
//! 명령은 경로가 **셸 안전 문자**(영숫자 · `/._-+@` · 유닉스는 비ASCII 글자 허용)일 때만 만든다. 그러면 agy 가
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
//! ## 윈도우 — 자동 연결 **끔** (측정 불능은 통과가 아니다)
//!
//! 위 역어셈블은 **macOS 판**이다. Go 는 OS 별로 따로 컴파일하므로 윈도우 판이 같은 `sh -c` 인지(그렇다면 `sh.exe` 가
//! PATH 에 있어야 한다 — Git for Windows 기본 설치는 `Git\cmd` 만 PATH 에 넣고 `sh.exe` 는 `Git\bin`·`Git\usr\bin` 에
//! 있다), 윈도우 전용 분기가 있는지는 이 맥에서 알 수 없다. 공식 문서(https://antigravity.google/docs/cli/statusline/
//! 2026-09-24 확인)도 셸을 적지 않는다. 공개 보고 둘(weby-homelab/antigravity-cli-statusline#63 ·
//! doggy8088/TokenUsageInsights#64)은 command 안의 **따옴표가 글자 그대로 넘어가** 경로가 깨졌다고 적고, 다른 하나
//! (onenowy/antigravity-cli-statusline README)는 공백 경로를 따옴표로 감싸라고 적는다 — 서로 어긋난다. `bash` 가
//! Git Bash 인지(WSL 의 `System32\bash.exe` 가 먼저 잡힐 수 있다), 아예 없는지도 기계마다 다르다. 그래서 윈도우는
//! 쓰지 않고 안내만 한다(`WindowsManualOnly` — agy 가 설치돼 있고 cys 연결이 아직 없을 때만). 제거 경로는 OS 무관하다
//! (표지 달린 연결만 지운다).

use serde_json::Value;
use std::path::{Path, PathBuf};

/// agy 상태줄 전용 래퍼(팩 `hooks/` 안 · 훅이 아니라 상태줄 명령).
pub const SCRIPT: &str = "cys-agy-statusline.sh";
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

/// 경로 문자열이 셸 안전 문자만으로 되어 있는가(순수). 안전 = `sh -c`·공백 분리 직접 실행·(윈도우) cmd·PowerShell 어느
/// 해석으로도 같은 한 토큰이 된다.
fn path_is_shell_safe(p: &str, windows: bool) -> bool {
    if p.is_empty() {
        return false;
    }
    let abs = if windows {
        let b = p.as_bytes();
        b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'/'
    } else {
        p.starts_with('/')
    };
    abs && p.chars().enumerate().all(|(i, c)| {
        c.is_ascii_alphanumeric()
            || matches!(c, '/' | '.' | '_' | '-')
            || (windows && c == ':' && i == 1)
            || (!windows && matches!(c, '+' | '@'))
            // 비ASCII 글자(예: 한글 사용자 폴더)는 sh·공백 분리 모두 안전하다. 윈도우는 코드페이지 해석이 끼어
            // 확신이 없으므로 허용하지 않는다.
            || (!windows && !c.is_ascii() && !c.is_whitespace() && !c.is_control())
    })
}

/// 연결 명령 문자열(순수 · OS 규칙 주입). `None` = 이 경로로는 안전한 명령을 만들 수 없다(공백·따옴표·상대경로 등).
///
/// - 유닉스: `sh <pack>/hooks/cys-agy-statusline.sh[ --cys-autolink]`
/// - 윈도우(안내 전용 — 자동 연결은 하지 않는다): `bash C:/…/hooks/cys-agy-statusline.sh[ --cys-autolink]` — 역슬래시는
///   정슬래시로, `\\?\` 확장 접두는 벗긴다. **따옴표를 두르지 않는다**(따옴표가 글자 그대로 넘어간 보고가 있다 — 모듈 머리).
pub fn link_command_for(pack_dir: &str, windows: bool, marker: bool) -> Option<String> {
    let script = if windows {
        let p = pack_dir.replace('\\', "/");
        let p = p.strip_prefix("//?/").unwrap_or(&p).to_string();
        format!("{}/hooks/{SCRIPT}", p.trim_end_matches('/'))
    } else {
        format!("{}/hooks/{SCRIPT}", pack_dir.trim_end_matches('/'))
    };
    if !path_is_shell_safe(&script, windows) {
        return None;
    }
    let interp = if windows { "bash" } else { "sh" };
    Some(if marker {
        format!("{interp} {script} {MARKER}")
    } else {
        format!("{interp} {script}")
    })
}

/// 자동 연결을 이 OS 에서 하는가(순수) — 윈도우는 끔(모듈 머리 '윈도우').
pub fn auto_link_supported(windows: bool) -> bool {
    !windows
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

/// 명령이 cys 가 넣은(표지 달린) 연결인가(순수).
pub fn command_is_ours_auto(cmd: &str) -> bool {
    let n = norm_cmd(cmd);
    let toks: Vec<&str> = n.split_whitespace().collect();
    toks.last() == Some(&MARKER) && toks.iter().any(|t| t.ends_with(&format!("/hooks/{SCRIPT}")))
}

/// 명령이 cys 상태줄 래퍼를 부르는가(표지 무관 · 순수).
pub fn command_is_cys(cmd: &str) -> bool {
    let n = norm_cmd(cmd);
    n.contains(SCRIPT) || n.contains("cys-statusline.sh")
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

struct Member {
    key_start: usize,
    key: String,
    val_start: usize,
    val_end: usize,
}

struct RootScan {
    open: usize,
    close: usize,
    members: Vec<Member>,
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
fn scan_root(t: &str) -> Result<RootScan, String> {
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

fn line_indent(t: &str, pos: usize) -> Option<&str> {
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
    /// 윈도우 — 자동 연결 안 함(안내만).
    WindowsManualOnly,
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

/// 조정에 필요한 경로(주입형 — 시험은 가짜 홈을 준다).
pub struct Ctx<'a> {
    pub settings: &'a Path,
    pub pack_dir: &'a Path,
    pub record: &'a Path,
    pub windows: bool,
}

/// 파일 판독 결과: (원문 바이트, BOM 뗀 본문, BOM 여부, 파싱 값 — 빈 파일이면 None).
type Loaded = (Vec<u8>, String, bool, Option<Value>);

fn load(settings: &Path) -> Result<Option<Loaded>, String> {
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

/// 쓰기 가능 확인 — 추가 모드로 열어 보기만 한다(내용·mtime 무변경). 읽기 전용·권한 없음·(윈도우) 잠김이면 Err.
fn probe_writable(settings: &Path) -> Result<(), String> {
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
fn file_mode(settings: &Path) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(settings).ok().map(|m| m.permissions().mode() & 0o7777)
}
#[cfg(not(unix))]
fn file_mode(_settings: &Path) -> Option<u32> {
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
    // agy 가 없는 기계(대다수)는 OS 무관 조용히 끝낸다 — 설치·업데이트마다 윈도우 안내를 찍지 않는다.
    if !ctx.settings.parent().is_some_and(Path::is_dir) {
        return Outcome::NotInstalled;
    }
    if !auto_link_supported(ctx.windows) {
        // 윈도우는 읽기만 한다: 이미 cys 연결이면 무동작(안내 없음), 그 밖(빈 칸·사용자 설정·판독 불가)은 안내만.
        return match inspect(ctx.settings) {
            Ok(Some(slot @ (Slot::OursAuto { .. } | Slot::CysManual { .. }))) => Outcome::AlreadyLinked(slot),
            _ => Outcome::WindowsManualOnly,
        };
    }
    let Some(cmd) = link_command_for(&ctx.pack_dir.to_string_lossy(), ctx.windows, true) else {
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
    if !ctx.pack_dir.join("hooks").join(SCRIPT).is_file() {
        return Outcome::Refused(format!(
            "팩에 상태줄 래퍼(hooks/{SCRIPT})가 없어 연결하지 않았다 — `cys init-pack` 뒤 `cys doctor --fix`"
        ));
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
        Outcome::WindowsManualOnly => format!(
            "윈도우는 자동 연결을 하지 않습니다(agy 가 상태줄 명령을 어떤 셸로 부르는지 미확인) — {p} 에 직접 넣는 방법은 \
             사용 설명서 agy 절"
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
            ensure_linked(&Ctx { settings: &s, pack_dir: &p, record: &r, windows: false }, force)
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
        // 윈도우: 역슬래시 → 정슬래시 · 확장 접두 제거 · 따옴표 없음 · bash
        assert_eq!(
            link_command_for(r"C:\Users\x\.cys\pack", true, true).as_deref(),
            Some("bash C:/Users/x/.cys/pack/hooks/cys-agy-statusline.sh --cys-autolink")
        );
        assert_eq!(
            link_command_for(r"\\?\C:\Users\x\.cys\pack", true, false).as_deref(),
            Some("bash C:/Users/x/.cys/pack/hooks/cys-agy-statusline.sh")
        );
        // 공백·따옴표·상대경로·UNC 는 명령을 만들지 않는다(안내만)
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
        assert!(!auto_link_supported(true), "윈도우 자동 연결은 꺼져 있어야 한다(측정 불능은 통과가 아니다)");
        assert!(auto_link_supported(false));
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

    #[test]
    fn windows_never_writes() {
        let sb = Sandbox::new("win");
        sb.put(AGY_DEFAULT.as_bytes());
        let (s, p, r) = (sb.settings(), sb.pack(), sb.record());
        assert_eq!(ensure_linked(&Ctx { settings: &s, pack_dir: &p, record: &r, windows: true }, true), Outcome::WindowsManualOnly);
        assert_eq!(sb.read(), AGY_DEFAULT);
    }

    /// 윈도우 안내는 **알릴 것이 있을 때만** — agy 가 없는 기계(대다수)에 설치·업데이트마다 '윈도우는 자동 연결 안 함'을
    /// 찍지 않고, 이미 cys 연결이 있는 기계에도 찍지 않는다. 어느 경우도 쓰지 않는다.
    #[test]
    fn windows_is_quiet_without_agy_and_when_already_linked() {
        let sb = Sandbox::new("winq");
        let (s, p, r) = (sb.settings(), sb.pack(), sb.record());
        let win = |force| ensure_linked(&Ctx { settings: &s, pack_dir: &p, record: &r, windows: true }, force);
        assert_eq!(win(false), Outcome::NotInstalled, "agy 미설치 윈도우에 안내를 찍으면 안 된다");
        assert!(!sb.settings().parent().unwrap().exists());
        let manual = "{\"statusLine\": {\"type\": \"command\", \"command\": \"bash C:/Users/x/.cys/pack/hooks/cys-agy-statusline.sh\"}}";
        sb.put(manual.as_bytes());
        assert!(matches!(win(true), Outcome::AlreadyLinked(Slot::CysManual { .. })), "직접 넣은 cys 연결은 무동작");
        assert_eq!(sb.read(), manual);
        assert!(describe(&win(false), &s).is_none());
        let user = "{\"statusLine\": {\"command\": \"mine.cmd\"}}";
        sb.put(user.as_bytes());
        assert_eq!(win(true), Outcome::WindowsManualOnly);
        assert_eq!(sb.read(), user);
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
        assert!(matches!(ensure_linked(&Ctx { settings: &s, pack_dir: &p, record: &r, windows: false }, false), Outcome::UnsafePath(_)));
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

    #[test]
    fn describe_speaks_only_when_needed() {
        let s = Path::new("/h/settings.json");
        assert!(describe(&Outcome::AlreadyLinked(Slot::OursAuto { enabled: Some(true) }), s).is_none());
        assert!(describe(&Outcome::NotInstalled, s).is_none());
        assert!(describe(&Outcome::UserOwned, s).unwrap().contains("덮지 않았습니다"));
        assert!(describe(&Outcome::WindowsManualOnly, s).unwrap().contains("윈도우는 자동 연결을 하지 않습니다"));
        assert!(describe(&Outcome::Linked { created: false }, s).unwrap().contains("CYS_AGY_STATUSLINE=0"));
    }
}
