//! Windows 좌석 Claude Code **클래식 렌더러 보장** — 0.14.45 (휠 스크롤 결함 수리 A).
//!
//! ## 무엇이 결함이었나 (2026-10 Windows 11 · Claude Code 2.1.291 · 오너 제보)
//!
//! master·worker pane 에서 마우스 휠로 대화가 올라가지 않고, CSO·reviewer1·reviewer2 pane 에서는 된다.
//! 앞의 둘은 Claude Code 의 **fullscreen(alt screen) 렌더러**로, 뒤의 셋은 **classic** 으로 떠 있었다.
//! Windows 의 cys 는 fullscreen pane 의 휠을 일부러 삼킨다(`ui/src/wheelgate.ts` 의 `shouldSuppressWheelWin` —
//! 휠이 방향키로 합성돼 프롬프트 히스토리를 오염시키는 원 결함의 방어). alt 화면에는 스크롤백이 없으므로
//! 그 pane 에서 휠은 **완전 무동작**이 된다.
//!
//! 렌더러는 Claude Code 가 **프로세스마다** 정한다. 같은 설정 폴더를 쓰는 좌석끼리도 갈린다 — 판정 재료가
//! env(`CLAUDE_CODE_DISABLE_ALTERNATE_SCREEN` · `CLAUDE_CODE_NO_FLICKER`) · 크래시 카나리 · 설정 `tui` ·
//! 첫 설치 업셀 · 서버측 기능 게이트처럼 **바뀌는 공유 상태**이기 때문이다.
//!
//! ## 왜 설정 파일인가 (env D5 가 아니라)
//!
//! D5 env(`lib.rs` `d5_gate_for_os`)는 Windows 에서 옵트인이고, 켜도 **새 surface 를 만드는 launch-agent
//! 기동에만** 닿는다(기존 pane 재기동 = node-recover · in-seat restore 는 env 를 싣지 못한다). 반면 Claude Code 의
//! 사용자 설정 `<CLAUDE_CONFIG_DIR>/settings.json` 의 `tui` 는 **그 폴더로 뜨는 모든 claude 프로세스가** 읽는다 —
//! 부트 · launch-agent · GUI(→ `cys launch-agent`) · node-recover · restore, 그리고 사람이 pane 에서 손으로 친
//! `claude` 까지. 그래서 기록 지점 하나로 모든 기동 경로를 덮는다.
//!
//! ## ★정적 검증 — 키와 값 (2026-10-07 · 이 맥의 `~/.local/share/claude/versions/2.1.291` · `strings` 판독 · 실행 0)
//!
//! - 설정 스키마: `tui:()=>G(["default","fullscreen"]).optional().describe('Terminal UI renderer. "fullscreen" uses
//!   the flicker-free alt-screen renderer with virtualized scrollback (equivalent to CLAUDE_CODE_NO_FLICKER=1).
//!   "default" uses the classic main-screen renderer.')` — **키 `tui` · 값 `"default"` = classic**.
//! - `/tui <default|fullscreen>` 명령: `_n("userSettings",{tui:he},…)` — 사용자 설정(userSettings)에 그 값을 저장한다
//!   (`/tui default` 안내문 "saved to your preferences"). userSettings 의 파일은 설정 폴더의 `settings.json`
//!   (`jP={default:"settings.json",…}`) — 이 저장소의 각성 훅 병합이 쓰는 바로 그 파일이다.
//! - 렌더러 판정 `Hc()`: `bg 세션 → fullscreen` · 스크린리더 → classic · `CLAUDE_CODE_DISABLE_ALTERNATE_SCREEN`(또는
//!   `NO_FLICKER=0`) → classic · `CLAUDE_CODE_NO_FLICKER=1` → fullscreen · 크래시 자동 꺼짐 · tmux -CC · Windows∧SSH →
//!   classic · 그다음 `switch(ct().tui ?? …){case"fullscreen":return!0;case"default":return!1}` — 설정 `tui` 가
//!   있으면 **업셀 · 서버 게이트(`tengu_pewter_brook`)보다 먼저** 결정한다. 같은 모양이 2.1.289 · 2.1.292 에도 있다.
//! - 자동 승격 경로 둘(업셀 체험 영속 · 다운셀 졸업)은 모두 `tui === undefined` 일 때만 `tui:"fullscreen"` 을 쓴다 —
//!   키가 이미 있으면 Claude Code 스스로도 덮지 않는다. 즉 **'키가 없을 때만 넣는다'** 는 이 모듈의 계약과 Claude
//!   Code 자신의 규칙이 같은 방향이다.
//! - 이미 떠 있는 세션은 렌더러를 기동 때 정한다(`/tui` 는 전환을 위해 **스스로 재시작**한다) — 이 기록은
//!   **다음 기동부터** 효력이다.
//!
//! ## ★옛 Claude Code 와 모르는 키 (B7 · 정적 판독 · 실행 0 · 2026-10-07)
//!
//! - 이 저장소가 전제하는 Claude Code 하한: 첫기동 관문 코퍼스 `measured_on` = **2.1.241**(`cysjavis-pack/agents.json` ·
//!   `src/first_run_gates.rs`) · 폴더신뢰 창 문안 핀 2.1.261+(`cys.rs` `GATE_DEFAULT_FOCUS_WARNING`). `tui` 키는 그보다
//!   앞선 **2.1.233** 의 fullscreen 판정(`ra()` · `alt_screen_notice` doc 의 2026-08-17 실측)에 이미 있었다 — 지원
//!   하한의 어느 버전에도 `tui` 는 **아는 키**다.
//! - 모르는 키라도 파일 전체가 버려지지 않는다: 이 맥의 2.1.289 · 2.1.291 · 2.1.292 를 `strings` 로 판독하면 사용자 설정
//!   스키마는 `z.object({… tui: …}).optional(` 로 끝나고(`.strict()` 아님 — zod 기본 = 모르는 키는 **벗겨낼 뿐** 오류가
//!   아니다) 설정 경고 문안도 '위 값만 보수적으로 읽거나 건너뛰고 나머지는 유효'(`The values listed above were
//!   replaced … the rest of the file is in effect`)다 — '파일 전체 건너뜀'(`Files with errors are skipped entirely`)은
//!   관리형(정책) 설정에만 붙는 문안이다. 사용자 `settings.json` 은 그 분기가 아니다.
//! - 결론: **버전 게이트를 두지 않는다**. `claude --version` 을 기동 직전에 부르면 그 자체가 1초급 지연·실패 지점이고,
//!   위 두 사실로 얻는 것이 없다. 이 판단의 전제(지원 하한 ≥ 2.1.233 · 사용자 설정 스키마 비엄격)가 바뀌면 그때 게이트를
//!   단다 — 디스크에 2.1.233 미만 판이 없어 그 판들의 스키마는 **판독하지 못했다**(정직 고지 · 추정 아님 · 하한 밖).
//!
//! ## 안전성 근거 (치명위험 ④ 렌즈)
//!
//! - classic 은 Windows 실기에서 이미 돌고 있는 모드다 — 같은 기계의 CSO·reviewer 좌석이 classic 으로 정상 동작
//!   중이라는 것이 이 수리의 실기 증거다(classic 이 claude 를 깨뜨린다면 그 좌석들이 먼저 죽었다).
//! - **실패 방향 = 언제나 '쓰지 않음'** 이고, 쓰기 실패는 기동을 막지 않는다(호출부는 결과를 로그로만 낸다).
//!
//! ## 계약 (agy 상태줄 자동 연결 `agy_statusline` 과 같은 외과 수술 규약 — 판독 · 스캐너는 그 모듈 것을 공유한다)
//!
//! - `tui` 키가 **이미 있으면 값과 무관하게** 덮지 않는다(`AlreadySet` — 사용자 `/tui fullscreen` 불가침).
//! - JSON 파싱 실패 · UTF-8 아님 · 루트가 객체 아님 · 심볼릭 링크 · 쓰기 권한 없음 · 1 MiB 초과 → 쓰지 않는다.
//! - 설정 폴더 자체가 없으면 만들지 않는다(`NoConfigDir` — 첫 로그인 전 · 엉뚱한 경로).
//! - 상대 경로는 기록하지 않는다 — 다음 기동의 작업 폴더가 바뀌면 다른 설정을 되돌릴 위험이 있다.
//! - 개인 기본 프로필 `~/.claude` 는 출처와 무관하게 쓰지 않는다(`PersonalProfile`). 좌석 spec 에 명시된
//!   `CLAUDE_CONFIG_DIR` 의 `~/.claude-*` 는 좌석 계정이므로 허용하고, 암묵적 경로라면 그 프로필들도 보호한다.
//! - 텍스트 외과 수술: 마지막 멤버 뒤에 `"tui": "default"` 한 칸만 붙이고 나머지 바이트(키 순서 · 들여쓰기 · CRLF ·
//!   BOM)는 그대로 둔다. 결과를 다시 파싱해 '다른 키 전부 동일 ∧ 새 키는 tui 하나 ∧ tui == "default"' 일 때만 쓴다.
//! - 쓰기 전 백업은 `settings.json.bak-cys-tui` — 각성 훅 병합의 `.bak-cys` 를 **클로버하지 않도록** 이름을 가른다.
//! - RMW 전 구간을 훅 병합 · preflight 와 같은 `<settings>.cys-lock` 으로 직렬화(유닉스) · 쓰기 직전 재판독 대조 ·
//!   원자 쓰기(원 권한 유지) · 쓴 뒤 되읽기 확인.
//!
//! ## 게이트와 킬스위치
//!
//! - OS 게이트는 순수 함수 [`gate_for_os`] — **Windows 만**. macOS 는 D5 env 가 기본 주입이라 필요 없고, mac 의
//!   사용자 설정을 바꿀 이유가 없다. 문자열 매핑이라 mac CI 에서 두 분기를 함께 핀으로 박는다(`d5_gate_for_os` 관례).
//! - 킬스위치: env `CYS_WIN_TUI_CLASSIC_OFF=1` 또는 파일 `~/.cys/win-tui-classic-off`(형제 `CYS_WIN_WHEEL_GUARD_OFF` ·
//!   `~/.cys/win-wheel-guard-off` 와 같은 `_OFF` = '우리 기능 끄기' 극성 · 판독 규약 `env == "1"` ∨ 파일 존재).
//!   킬스위치는 이후 기록을 멈추고, `.cys/claude-tui-written.json` 원장에 있는 설정을 프로세스당 한 번 되돌린다.
//!   cys 가 넣은 값이 **아직 정확히 `"default"` 일 때만** 제거하며, 사용자 변경값은 그대로 두고 원장에서 뺀다.
//!   거부한 항목은 원장에 남겨 다음 기동에서 재시도한다. 원장 기록 실패도 로그로 알리되 설정 쓰기를 되감지 않는다.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex, OnceLock,
};

use crate::settings_surgery as surgery;

/// Claude Code 사용자 설정의 렌더러 키.
pub const TUI_KEY: &str = "tui";
/// classic(main-screen) 렌더러 값 — `/tui default` 가 저장하는 값과 같다.
pub const TUI_CLASSIC: &str = "default";
/// 킬스위치 env(값 `"1"` 만 참).
pub const KILL_ENV: &str = "CYS_WIN_TUI_CLASSIC_OFF";
/// 킬스위치 파일(홈 기준 상대 — 형제: `.cys/win-wheel-guard-off` · `.cys/win-no-alt-screen`).
pub const KILL_FILE: &str = ".cys/win-tui-classic-off";
/// cys 가 직접 삽입한 설정의 원장(주입받은 홈 기준).
pub const LEDGER_FILE: &str = ".cys/claude-tui-written.json";
const MAX_LEDGER_ENTRIES: usize = 64;
/// 쓰기 전 백업 접미 — 각성 훅 병합의 `.bak-cys` 와 **다른 이름**이어야 한다(정상 백업 클로버 금지).
pub const BACKUP_SUFFIX: &str = ".bak-cys-tui";

/// OS 게이트의 **단일 진리원**(순수) — `windows ∧ ¬킬스위치` 일 때만 참. 그 외 OS 는 언제나 거짓.
pub fn gate_for_os(os: &str, kill_off: bool) -> bool {
    os == "windows" && !kill_off
}

/// 킬스위치 판독 규약의 순수 코어 — `env == "1"` ∨ 파일 존재(느슨한 truthy 는 거짓 · 형제 게이트와 동형).
pub fn kill_switch_from(env_val: Option<&str>, file_exists: bool) -> bool {
    env_val == Some("1") || file_exists
}

/// 킬스위치 판독(부작용 — env 1회 + 파일 stat 1회).
pub fn kill_switch_on() -> bool {
    kill_switch_from(
        crate::env_compat(KILL_ENV).as_deref(),
        crate::home_dir().join(KILL_FILE).exists(),
    )
}

/// 이 기동이 기록 대상인가(부작용 — 킬스위치 판독). 실제 OS 상수를 순수 게이트에 먹인다.
pub fn gate_now() -> bool {
    gate_for_os(std::env::consts::OS, kill_switch_on())
}

/// 설정 폴더 출처 — 명시된 좌석 계정과 개인 기본값을 구분한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirOrigin {
    SeatSpec,
    Implicit,
}

/// 개인 프로필 비교 전용 정규화(순수). OS 경로 파서에 의존하지 않아 맥에서도 Windows 문자열을 시험한다.
/// verbatim 접두 · 구분자 · 끝 구분자 · 점 경로를 접고, Windows 에서만 대소문자를 접는다.
fn normalize_profile_path(path: &Path, windows: bool) -> String {
    let mut text = path.to_string_lossy().into_owned();
    if windows {
        text = text.replace('\\', "/");
        if text.get(..8).is_some_and(|p| p.eq_ignore_ascii_case("//?/UNC/")) {
            text = format!("//{}", &text[8..]);
        } else if let Some(rest) = text.strip_prefix("//?/") {
            text = rest.to_string();
        }
        text = text.to_lowercase();
    }
    let (prefix, rest, rooted, protected) = if windows && text.starts_with("//") {
        ("//", text.trim_start_matches('/'), true, 2)
    } else if windows && text.as_bytes().get(1) == Some(&b':') {
        let rest = &text[2..];
        if rest.starts_with('/') {
            (&text[..3], rest.trim_start_matches('/'), true, 0)
        } else {
            (&text[..2], rest, false, 0)
        }
    } else if text.starts_with('/') {
        ("/", text.trim_start_matches('/'), true, 0)
    } else {
        ("", text.as_str(), false, 0)
    };
    let mut parts = Vec::new();
    for part in rest.split('/') {
        match part {
            "" | "." => {}
            ".." if parts.len() > protected && parts.last() != Some(&"..") => {
                parts.pop();
            }
            ".." if !rooted => parts.push(part),
            ".." => {}
            _ => parts.push(part),
        }
    }
    format!("{prefix}{}", parts.join("/"))
}

/// 개인 기본 프로필은 항상 보호한다. `SeatSpec` 일 때만 명시된 `.claude-*` 좌석 계정을 허용한다(순수).
pub fn is_personal_profile(dir: &Path, home: &Path, origin: DirOrigin) -> bool {
    let dir = normalize_profile_path(dir, cfg!(windows));
    let home = normalize_profile_path(home, cfg!(windows));
    let base = if home.is_empty() {
        ".claude".to_string()
    } else {
        format!("{}/.claude", home.trim_end_matches('/'))
    };
    dir == base
        || (origin == DirOrigin::Implicit
            && dir
                .strip_prefix(&base)
                .is_some_and(|tail| tail.starts_with('-') && !tail.contains('/')))
}

/// 조정 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// 설정 폴더가 없다 — 아무것도 만들지 않는다.
    NoConfigDir,
    /// 표적이 개인 프로필이라 쓰지 않았다.
    PersonalProfile,
    /// `tui` 키가 이미 있다(값 동봉 — JSON 표기) — 덮지 않는다.
    AlreadySet(String),
    /// 새로 넣었다. 원장 기록은 최선 노력이며 실패 사유를 숨기지 않는다.
    Written {
        created: bool,
        ledger_error: Option<String>,
    },
    /// 킬스위치가 원장 항목을 되돌렸다(거부 · 원장 오류 포함).
    RolledBack(Vec<(PathBuf, RollbackOutcome)>),
    /// 쓰지 않았다(사유).
    Refused(String),
}

/// 원장 항목 하나의 되돌림 결과. 거부만 원장에 남아 재시도한다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RollbackOutcome {
    Removed,
    Dropped,
    Refused(String),
}

/// 원장 항목 — cys 가 `"tui": "default"` 를 넣은 설정 폴더 하나. `pub` 인 것은 완전 초기화(`factory_reset`)가 `~/.cys` 를 격리하기 **전에** 읽어 두고
/// 격리 뒤에 항목으로 되돌리기 때문이다([`reset_entries`] · 성찰 2회차 m4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerEntry {
    pub dir: PathBuf,
    pub settings: PathBuf,
    pub written_at: u64,
    /// 설정 파일을 cys 가 **새로 만들었다**(true) / 있던 파일에 키만 넣었다(false — 이때만 옆자리 백업 `.bak-cys-tui` 를 만든다).
    pub created: bool,
}

#[derive(Serialize, Deserialize)]
struct Ledger {
    version: u32,
    entries: Vec<LedgerEntry>,
}

/// 원장 쓰기의 **충돌** 사유 문면 — `write_ledger` 가 '읽은 뒤 다른 쪽이 썼다' 를 이 바이트로 알리고 [`update_ledger`] 가 이것만 재시도한다.
const LEDGER_CONFLICT: &str = "그 사이 원장이 바뀌었다 — 쓰지 않는다";
/// 원장 RMW 재시도 상한 — 그 안에 다섯 번 연속 충돌하면(동시 기동이 다섯 이상) 포기하고 사유를 돌려준다(설정 쓰기는 이미 끝났고 원장만 빠진다 · 로그로 알린다).
const LEDGER_RETRY_MAX: usize = 5;
/// ★(2차 검토 MAJOR-4 · C5) 원장 **전용 잠금 파일**(`<원장>.lock` · 생성 배타 `create_new` — 모든 OS 에서 상호 배제). 윈도우에는 설정 락(flock)이 없어 병합 재시도만으로는
/// '비교 통과 → 둘 다 쓰기' 의 좁은 창(TOCTOU)이 남았다. 획득은 최대 [`LEDGER_LOCK_WAIT_MS`] 동안 기다리고, 그보다 오래된([`LEDGER_LOCK_STALE_SECS`]) 잠금은 죽은
/// 프로세스의 것으로 보아 치운다. 끝내 못 얻으면 잠금 없이 진행한다(실패 방향 = 종전 동작 + 병합 재시도 · 설정 쓰기는 이미 끝났고 원장만 걸린다).
const LEDGER_LOCK_WAIT_MS: u64 = 2_000;
const LEDGER_LOCK_STALE_SECS: u64 = 60;

/// 원장 잠금 — 떨어질 때 잠금 파일을 지운다.
struct LedgerLock(PathBuf);

impl Drop for LedgerLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn ledger_lock(path: &Path) -> Option<LedgerLock> {
    let lock = PathBuf::from(format!("{}.lock", path.display()));
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(LEDGER_LOCK_WAIT_MS);
    loop {
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&lock) {
            Ok(_) => return Some(LedgerLock(lock)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                let stale = std::fs::metadata(&lock)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.elapsed().ok())
                    .is_some_and(|age| age.as_secs() >= LEDGER_LOCK_STALE_SECS);
                if stale {
                    let _ = std::fs::remove_file(&lock);
                    continue;
                }
                if std::time::Instant::now() >= deadline {
                    return None;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(_) => return None, // 폴더 없음·권한 — 잠금 없이 진행(원장 쓰기 자체가 그 오류를 돌려준다)
        }
    }
}

// Windows 의 pack 락은 미획득이므로 프로세스 내부 RMW 도 직렬화한다. 파일락 순서는 항상 설정 → 원장이다.
static MUTATION_LOCK: Mutex<()> = Mutex::new(());
static PERSONAL_DESCRIBED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
/// ★(0.14.45 · 성찰 M3) `tui:"fullscreen"` 이 이미 있어 덮지 않은 폴더 — 프로세스당 폴더별 1회만 알린다(기동마다 같은 줄 반복 금지).
static FULLSCREEN_DESCRIBED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
static ROLLBACK_STARTED: AtomicBool = AtomicBool::new(false);

/// fullscreen 렌더러 값 — Claude Code 가 업셀·다운셀 경로에서 스스로 쓰기도 하는 값(사용자 `/tui fullscreen` 과 구분할 수 없다).
pub const TUI_FULLSCREEN: &str = "fullscreen";

/// ★(성찰 M3) `AlreadySet` 이 **fullscreen** 인가(순수) — `AlreadySet` 의 값은 JSON 표기(`"\"fullscreen\""`)다.
pub fn already_set_is_fullscreen(json_repr: &str) -> bool {
    serde_json::from_str::<Value>(json_repr).ok().and_then(|v| v.as_str().map(|s| s == TUI_FULLSCREEN)).unwrap_or(false)
}

/// ★(성찰 M3) fullscreen 이 이미 적혀 있는 좌석의 안내 한 줄(순수) — launch 로그(`describe`)와 `cys doctor` 가 같은 문장을 쓴다.
pub fn fullscreen_already_set_line(settings: &Path) -> String {
    format!(
        "{} 에 \"tui\": \"fullscreen\" 이 이미 적혀 있어 cys 는 덮지 않습니다(사용자 선택인지 Claude Code 의 전체화면 승격인지 구분할 수 없습니다) — \
         이 좌석은 전체화면이라 마우스 휠 스크롤이 꺼집니다 · 그 pane 에서 /tui default 를 치면 다음 기동부터 classic(휠 스크롤)입니다",
        settings.display()
    )
}

/// ★(성찰 M3 · doctor) 설정 폴더의 `tui` 값 판독 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TuiProbe {
    /// 설정 폴더 자체가 없다(좌석 미사용).
    NoConfigDir,
    /// settings.json 이 없거나 `tui` 키가 없다(기동 때 cys 가 classic 을 넣는 대상).
    Absent,
    /// `"default"`(classic).
    Classic,
    /// `"fullscreen"` — 휠 스크롤이 꺼지는 좌석.
    Fullscreen,
    /// 그 밖 값(JSON 표기).
    Other(String),
    /// 판독 실패(사유).
    Unreadable(String),
}

/// ★(성찰 M3 · doctor) 설정 폴더 하나의 `tui` 값(읽기 전용 · 판독기는 조정과 같은 `surgery::load`).
pub fn tui_probe(config_dir: &Path) -> TuiProbe {
    if !config_dir.is_dir() {
        return TuiProbe::NoConfigDir;
    }
    match surgery::load(&config_dir.join("settings.json")) {
        Err(e) => TuiProbe::Unreadable(e),
        Ok(None) => TuiProbe::Absent,
        Ok(Some((_, _, _, v))) => match v.as_ref().and_then(|v| v.get(TUI_KEY)) {
            None => TuiProbe::Absent,
            Some(Value::String(s)) if s == TUI_CLASSIC => TuiProbe::Classic,
            Some(Value::String(s)) if s == TUI_FULLSCREEN => TuiProbe::Fullscreen,
            Some(other) => TuiProbe::Other(other.to_string()),
        },
    }
}

/// ★(성찰 M3 · doctor) 원장(`~/.cys/claude-tui-written.json`)에 적힌 폴더들 — cys 가 classic 을 넣은 적 있는 좌석 폴더(= 알려진 좌석 폴더). 원장 없음·훼손 = 빈 목록.
pub fn ledger_dirs(home: &Path) -> Vec<PathBuf> {
    read_ledger(&home.join(LEDGER_FILE)).map(|(l, _)| l.entries.into_iter().map(|e| e.dir).collect()).unwrap_or_default()
}

/// ★(성찰 M3 · doctor) 알려진 좌석 폴더들을 중복 없이(정규화 비교) 판독한다 — 입력 순서 보존. 순수(각 폴더 stat·판독만).
pub fn tui_probe_dirs(dirs: &[PathBuf]) -> Vec<(PathBuf, TuiProbe)> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for d in dirs {
        if seen.insert(normalize_profile_path(d, cfg!(windows))) {
            out.push((d.clone(), tui_probe(d)));
        }
    }
    out
}

/// `"tui": "default"` 를 최상위 객체의 마지막 멤버로 붙인 새 텍스트(순수). `text` 는 BOM 을 뗀 본문이고,
/// `tui` 키가 이미 있으면 Err(호출부가 먼저 거르지만 이 함수도 덮어쓰기를 구조적으로 거부한다).
pub fn render_with_classic(text: &str) -> Result<String, String> {
    let scan = surgery::scan_root(text)?;
    if scan.members.iter().any(|m| m.key == TUI_KEY) {
        return Err("tui 키가 이미 있다 — 덮지 않는다".into());
    }
    let nl = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let val = serde_json::to_string(TUI_CLASSIC).map_err(|e| e.to_string())?;
    let first = scan.members.first();
    let pretty = first.is_none() || text[scan.open..=scan.close].contains('\n');
    let unit: String = first
        .and_then(|m| surgery::line_indent(text, m.key_start))
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
    match scan.members.last() {
        None => Ok(format!(
            "{}{nl}{unit}\"{TUI_KEY}\": {val}{nl}{}",
            &text[..scan.open + 1],
            &text[scan.close..]
        )),
        Some(last) => {
            let lead = if pretty {
                format!(",{nl}{unit}")
            } else {
                ",".to_string()
            };
            Ok(format!(
                "{}{lead}\"{TUI_KEY}\"{sep}{val}{}",
                &text[..last.val_end],
                &text[last.val_end..]
            ))
        }
    }
}

/// 수술 결과 사후 검증(순수) — 다른 최상위 키 값 전부 동일 · 새 키는 tui 하나 · tui == "default".
pub fn verify_render(orig: &Value, new_text: &str) -> Result<(), String> {
    let new: Value = serde_json::from_str(new_text).map_err(|e| format!("수술 결과가 JSON 이 아니다: {e}"))?;
    let (Some(o), Some(n)) = (orig.as_object(), new.as_object()) else {
        return Err("수술 결과 루트가 객체가 아니다".into());
    };
    for (k, v) in o {
        if n.get(k) != Some(v) {
            return Err(format!("다른 키가 바뀌었다: {k}"));
        }
    }
    if n.keys().any(|k| k != TUI_KEY && !o.contains_key(k)) {
        return Err("모르는 키가 생겼다".into());
    }
    if n.get(TUI_KEY).and_then(Value::as_str) != Some(TUI_CLASSIC) {
        return Err("tui 가 원하는 값이 아니다".into());
    }
    Ok(())
}

/// 최상위 cys 삽입값만 제거한다(순수) — 다른 멤버의 순서 · 들여쓰기 · 끝 공백은 그대로 둔다.
pub fn render_without_classic(text: &str) -> Result<String, String> {
    let body = text.strip_prefix('\u{feff}').unwrap_or(text);
    let orig: Value = serde_json::from_str(body).map_err(|e| format!("JSON 파싱 실패: {e}"))?;
    if !orig.is_object() || orig.get(TUI_KEY).and_then(Value::as_str) != Some(TUI_CLASSIC) {
        return Err("tui 가 정확히 default 인 객체만 되돌린다".into());
    }
    let scan = surgery::scan_root(body)?;
    let mut matches = scan.members.iter().enumerate().filter(|(_, m)| m.key == TUI_KEY);
    let Some((index, member)) = matches.next() else {
        return Err("tui 키가 없다".into());
    };
    if matches.next().is_some() {
        return Err("중복 tui 키는 되돌리지 않는다".into());
    }
    let (start, end) = if scan.members.len() == 1 {
        // render_with_classic 의 빈 객체 꼴을 정확히 역산한다. 객체 밖 개행/BOM 은 보존한다.
        (scan.open + 1, scan.close)
    } else if index > 0 {
        (scan.members[index - 1].val_end, member.val_end)
    } else {
        // 사용자가 멤버를 재배치해도 다음 멤버 앞 들여쓰기는 그대로 둔다.
        let tail = &body[member.val_end..scan.members[1].key_start];
        let comma = tail.find(',').ok_or("멤버 구분자가 없다")?;
        (member.key_start, member.val_end + comma + 1)
    };
    let mut out = String::with_capacity(text.len());
    if body.len() != text.len() {
        out.push('\u{feff}');
    }
    out.push_str(&body[..start]);
    out.push_str(&body[end..]);
    verify_removal(&orig, &out)?;
    Ok(out)
}

/// 제거 결과 사후 검증(순수) — tui 부재 · 나머지 키 값 동일 · 새 키 없음.
pub fn verify_removal(orig: &Value, new_text: &str) -> Result<(), String> {
    let body = new_text.strip_prefix('\u{feff}').unwrap_or(new_text);
    let new: Value = serde_json::from_str(body).map_err(|e| format!("제거 결과가 JSON 이 아니다: {e}"))?;
    let (Some(o), Some(n)) = (orig.as_object(), new.as_object()) else {
        return Err("제거 결과 루트가 객체가 아니다".into());
    };
    if n.contains_key(TUI_KEY) {
        return Err("tui 가 남아 있다".into());
    }
    if o.iter().any(|(k, v)| k != TUI_KEY && n.get(k) != Some(v)) {
        return Err("다른 키가 바뀌었다".into());
    }
    if n.keys().any(|k| !o.contains_key(k)) {
        return Err("모르는 키가 생겼다".into());
    }
    Ok(())
}

/// 원장이 없으면 빈 원장. 훼손 · 링크 · 판독 실패는 빈 작업 목록으로 취급하되 원본 보존을 위해 Err 를 낸다.
fn read_ledger(path: &Path) -> Result<(Ledger, Option<Vec<u8>>), String> {
    if let Some(parent) = path.parent() {
        match std::fs::symlink_metadata(parent) {
            Ok(m) if m.file_type().is_symlink() || !m.is_dir() => {
                return Err("원장 폴더가 일반 디렉터리가 아니다".into())
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("원장 폴더 판독 실패: {e}")),
        }
    }
    let loaded = surgery::load(path).map_err(|e| format!("원장 판독 실패: {e}"))?;
    let Some((raw, text, _, _)) = loaded else {
        return Ok((
            Ledger {
                version: 1,
                entries: Vec::new(),
            },
            None,
        ));
    };
    let ledger: Ledger = serde_json::from_str(&text).map_err(|e| format!("원장 파싱 실패(원본 보존): {e}"))?;
    if ledger.version != 1 {
        return Err("지원하지 않는 원장 버전(원본 보존)".into());
    }
    Ok((ledger, Some(raw)))
}

fn write_ledger(path: &Path, ledger: &Ledger, original: Option<&[u8]>) -> Result<(), String> {
    // 링크 교체도 재판독으로 거부한다. 파일이 없어야 하는 경우도 읽기 오류와 구분한다.
    let (_, current) = read_ledger(path)?;
    if current.as_deref() != original {
        return Err(LEDGER_CONFLICT.into());
    }
    if original.is_some() {
        surgery::probe_writable(path).map_err(|e| format!("원장 쓰기 거부: {e}"))?;
    }
    let bytes = serde_json::to_vec(ledger).map_err(|e| format!("원장 직렬화 실패: {e}"))?;
    crate::pack::write_atomic(path, &bytes).map_err(|e| format!("원장 원자 쓰기 실패: {e}"))?;
    let (_, written) = read_ledger(path)?;
    if written.as_deref() != Some(bytes.as_slice()) {
        return Err("원장 되읽기 불일치".into());
    }
    Ok(())
}

fn append_ledger(home: &Path, config_dir: &Path, created: bool) -> Result<(), String> {
    let path = home.join(LEDGER_FILE);
    let parent = path.parent().ok_or("원장 부모 경로가 없다")?;
    // .cys 자체가 링크면 원장 기록을 거부한다. 설정 삽입의 성공 여부와는 별개다.
    match std::fs::symlink_metadata(parent) {
        Ok(m) if m.file_type().is_symlink() || !m.is_dir() => return Err("원장 폴더가 일반 디렉터리가 아니다".into()),
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir_all(parent).map_err(|e| format!("원장 폴더 생성 실패: {e}"))?;
        }
        Err(e) => return Err(format!("원장 폴더 판독 실패: {e}")),
    }
    let _ledger_lock = crate::pack::acquire_settings_lock(&path);
    let _ledger_file_lock = ledger_lock(&path);
    let key = normalize_profile_path(config_dir, cfg!(windows));
    let written_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| format!("원장 시각 판독 실패: {e}"))?
        .as_secs();
    // ★(성찰 2회차 C5) 병합형 RMW — 충돌이면 다시 읽은 원장 **위에** 이 항목을 다시 얹는다(다른 좌석의 항목을 지우지 않는다).
    update_ledger(&path, |ledger| {
        ledger
            .entries
            .retain(|e| normalize_profile_path(&e.dir, cfg!(windows)) != key);
        ledger.entries.push(LedgerEntry {
            dir: config_dir.to_path_buf(),
            settings: config_dir.join("settings.json"),
            written_at,
            created,
        });
        ledger.entries.sort_by_key(|e| e.written_at);
        if ledger.entries.len() > MAX_LEDGER_ENTRIES {
            ledger.entries.drain(..ledger.entries.len() - MAX_LEDGER_ENTRIES);
        }
    })
}

/// ★(성찰 2회차 C5) 원장 갱신 = 읽기 → `apply`(병합) → 재판독 대조 + 원자 쓰기 — **충돌이면 다시 읽어 그 위에 다시 적용**한다(최대 [`LEDGER_RETRY_MAX`]회).
/// 윈도우에는 설정 락이 없어(`acquire_settings_lock` = None) 동시에 뜨는 launch-agent 둘이 같은 원장을 읽고 각자 쓰면 뒤의 것이 앞의 항목을 지웠다
/// (종전엔 뒤의 쓰기가 '바뀜' 으로 거부돼 그 좌석의 항목만 빠졌다 — 킬스위치 되돌림에서 그 폴더가 누락된다). `apply` 는 멱등이어야 한다(재시도마다 새 원장에 다시 적용).
/// 유닉스는 락이 직렬화하므로 충돌 분기가 실행되지 않는다(동작 동일).
fn update_ledger(path: &Path, mut apply: impl FnMut(&mut Ledger)) -> Result<(), String> {
    let mut conflicts = 0usize;
    loop {
        let (mut ledger, raw) = read_ledger(path)?;
        apply(&mut ledger);
        match write_ledger(path, &ledger, raw.as_deref()) {
            Ok(()) => return Ok(()),
            Err(e) if e == LEDGER_CONFLICT && conflicts + 1 < LEDGER_RETRY_MAX => conflicts += 1,
            Err(e) if e == LEDGER_CONFLICT => return Err(format!("{e}(재시도 {LEDGER_RETRY_MAX}회 모두 충돌)")),
            Err(e) => return Err(e),
        }
    }
}

fn validate_ledger_entry(entry: &LedgerEntry, home: &Path) -> Result<(), String> {
    if !entry.dir.is_absolute() || !entry.settings.is_absolute() {
        return Err("상대 경로는 다음 기동의 대상을 보장하지 못한다".into());
    }
    if entry.settings != entry.dir.join("settings.json") {
        return Err("원장 설정 경로가 폴더와 맞지 않는다".into());
    }
    if is_personal_profile(&entry.dir, home, DirOrigin::SeatSpec) {
        return Err("개인 기본 프로필은 되돌리지 않는다".into());
    }
    Ok(())
}

/// 되돌림 백업의 위치 — 킬스위치 되돌림은 설정 파일 옆(`.bak-cys-tui`), 완전 초기화는 격리 폴더 안(흔적을 남기지 않는다 · 성찰 2회차 m4).
enum BackupWhere<'a> {
    Beside,
    Dir(&'a Path),
}

/// 격리 폴더 안 백업 파일 이름 — `claude-tui.<폴더를 파일명으로 접은 것>-<경로 해시 8자리>.settings.json[.bak-cys-tui]`(agy 의 `agy-antigravity-cli.settings.json`
/// 규약과 같은 폴더). ★(2차 검토 MAJOR-2) 접은 이름만으로는 `a_b` 와 `a/b` 가 같은 파일로 접혀 뒤 항목이 앞 항목의 복구 사본을 덮었다 — 정규화 경로의 해시를 붙이고,
/// 쓰는 쪽은 [`reset_backup_dest`] 로 **있는 파일을 덮지 않는다**.
fn reset_backup_name(dir: &Path, suffix: &str) -> String {
    use std::hash::{Hash, Hasher};
    let norm = normalize_profile_path(dir, cfg!(windows));
    let folded: String = norm
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '_' })
        .collect();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    norm.hash(&mut h);
    format!("claude-tui.{}-{:08x}.settings.json{suffix}", folded.trim_matches('_'), (h.finish() & 0xffff_ffff) as u32)
}

/// 격리 폴더 안에서 **아직 없는** 목적지 — 같은 이름이 있으면 `.1` `.2` … 를 붙인다(덮어쓰기 0 · 복구 사본은 서로를 지우지 않는다).
fn reset_backup_dest(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    (1..=999u32)
        .map(|n| dir.join(format!("{name}.{n}")))
        .find(|p| !p.exists())
        .unwrap_or(first)
}

fn rollback_entry(entry: &LedgerEntry, home: &Path) -> Result<RollbackOutcome, String> {
    rollback_entry_with(entry, home, BackupWhere::Beside)
}

fn rollback_entry_with(entry: &LedgerEntry, home: &Path, how: BackupWhere<'_>) -> Result<RollbackOutcome, String> {
    validate_ledger_entry(entry, home)?;
    // ★(2차 검토 MAJOR-3) 경로 문자열 검증만으로는 원장에 적힌 `.claude-team` 이 **그 뒤** 개인 `~/.claude` 를 가리키는 심볼릭 링크로 바뀐 경우를 못 거른다
    //   (`load` 는 settings.json 자신의 링크만 본다). 설정 폴더가 링크면 거부하고, 실경로(canonicalize)로 개인 프로필 판정을 한 번 더 한다(파일이 없으면 Dropped 로 간다).
    match std::fs::symlink_metadata(&entry.dir) {
        Ok(m) if m.file_type().is_symlink() => return Err("설정 폴더가 심볼릭 링크다 — 되돌리지 않는다".into()),
        Ok(_) => {
            if let (Ok(real_dir), Ok(real_home)) = (std::fs::canonicalize(&entry.dir), std::fs::canonicalize(home)) {
                if is_personal_profile(&real_dir, &real_home, DirOrigin::SeatSpec) {
                    return Err("실경로가 개인 기본 프로필이다 — 되돌리지 않는다".into());
                }
            }
        }
        Err(_) => {}
    }
    let Some((raw, text, bom, orig)) = surgery::load(&entry.settings)? else {
        return Ok(RollbackOutcome::Dropped);
    };
    let Some(orig) = orig else {
        return Ok(RollbackOutcome::Dropped);
    };
    if orig.get(TUI_KEY).and_then(Value::as_str) != Some(TUI_CLASSIC) {
        return Ok(RollbackOutcome::Dropped);
    }
    surgery::probe_writable(&entry.settings)?;
    let body = render_without_classic(&text)?;
    verify_removal(&orig, &body)?;
    let mut bytes = Vec::with_capacity(body.len() + 3);
    if bom {
        bytes.extend_from_slice(b"\xef\xbb\xbf");
    }
    bytes.extend_from_slice(body.as_bytes());
    let mode = surgery::file_mode(&entry.settings);
    match how {
        BackupWhere::Beside => backup(&entry.settings, &raw)?,
        BackupWhere::Dir(d) => {
            std::fs::create_dir_all(d).map_err(|e| format!("백업 폴더를 만들지 못했다: {e}"))?;
            let dest = reset_backup_dest(d, &reset_backup_name(&entry.dir, ""));
            crate::pack::write_atomic_mode(&dest, &raw, mode).map_err(|e| format!("백업 실패({}): {e}", dest.display()))?;
        }
    }
    match surgery::load(&entry.settings)? {
        Some((current, _, _, _)) if current == raw => {}
        _ => return Err("그 사이 설정 파일이 바뀌었다 — 되돌리지 않는다".into()),
    }
    crate::pack::write_atomic_mode(&entry.settings, &bytes, mode).map_err(|e| format!("되돌림 원자 쓰기 실패: {e}"))?;
    match surgery::load(&entry.settings)? {
        Some((current, _, _, Some(v))) if current == bytes && v.get(TUI_KEY).is_none() => Ok(RollbackOutcome::Removed),
        _ => Err("되돌림 되읽기 불일치 — 원장 항목을 남긴다".into()),
    }
}

/// 원장에 남은 cys 삽입값만 되돌린다. 직접 호출은 매번 재시도하며, home 은 시험에서 주입한다.
/// 설정 → 원장 순으로 잠근 뒤 현재 원장을 다시 읽어, 다른 좌석의 기록을 지우지 않는다.
pub fn rollback_from_ledger(home: &Path) -> Vec<(PathBuf, RollbackOutcome)> {
    let _serial = match MUTATION_LOCK.lock() {
        Ok(lock) => lock,
        Err(_) => {
            return vec![(
                home.join(LEDGER_FILE),
                RollbackOutcome::Refused("조정 락이 손상되었다".into()),
            )]
        }
    };
    let path = home.join(LEDGER_FILE);
    let snapshot = match read_ledger(&path) {
        Ok((ledger, _)) => ledger.entries,
        Err(e) => return vec![(path, RollbackOutcome::Refused(e))],
    };
    let mut outcomes = Vec::new();
    for entry in snapshot {
        if let Err(e) = validate_ledger_entry(&entry, home) {
            outcomes.push((entry.dir, RollbackOutcome::Refused(e)));
            continue;
        }
        let _settings_lock = crate::pack::acquire_settings_lock(&entry.settings);
        let _ledger_lock = crate::pack::acquire_settings_lock(&path);
        let _ledger_file_lock = ledger_lock(&path);
        let (ledger, _raw) = match read_ledger(&path) {
            Ok(loaded) => loaded,
            Err(e) => {
                outcomes.push((path.clone(), RollbackOutcome::Refused(e)));
                break;
            }
        };
        if !ledger.entries.contains(&entry) {
            continue; // 다른 프로세스가 이미 처리했거나 새 기록으로 갈아 끼웠다.
        }
        let outcome = match rollback_entry(&entry, home) {
            Ok(outcome) => {
                // ★(C5) 병합형 제거 — 그 사이 다른 프로세스가 원장에 항목을 더했어도 그 항목은 살리고 이 항목만 뺀다.
                match update_ledger(&path, |l| l.entries.retain(|e| e != &entry)) {
                    Ok(()) => outcome,
                    Err(e) => RollbackOutcome::Refused(format!("설정 확인/되돌림 후 원장 항목 제거 실패: {e}")),
                }
            }
            Err(e) => RollbackOutcome::Refused(e),
        };
        outcomes.push((entry.dir, outcome));
    }
    outcomes
}

/// 원장의 항목들(읽기 전용) — 완전 초기화가 `~/.cys` 를 격리하기 **전에** 읽어 둔다. 원장이 없으면 빈 목록 · 훼손·링크·버전 불일치는 Err(원본 보존 · 항목 없음으로 다루지 않는다).
pub fn ledger_entries(home: &Path) -> Result<Vec<LedgerEntry>, String> {
    read_ledger(&home.join(LEDGER_FILE)).map(|(l, _)| l.entries)
}

/// 완전 초기화의 되돌림 결과 한 건.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResetTrace {
    pub dir: PathBuf,
    pub outcome: RollbackOutcome,
    /// cys 가 만들었던 옆자리 백업 `settings.json.bak-cys-tui` 를 격리 폴더로 옮겼다.
    pub backup_moved: bool,
}

/// ★(성찰 2회차 m4) **완전 초기화**의 흔적 되돌림 — 원장 항목(`entries` · 격리 전에 읽어 둔 것)마다 킬스위치 되돌림과 **같은 안전 규약**으로 cys 가 넣은
/// `"tui": "default"` 를 뺀다(값이 아직 정확히 `"default"` 일 때만 · 개인 프로필·상대 경로 거부 · 쓰기 직전 재판독 · 원자 쓰기 · 되읽기). 다른 점 둘:
///   · 되돌림 백업은 설정 파일 옆이 아니라 격리 폴더 `backup_dir` 안(`claude-tui.<폴더>.settings.json`) — 초기화가 새 흔적을 남기지 않는다.
///   · cys 가 만들었던 옆자리 백업 `settings.json.bak-cys-tui` 는 **우리가 만들었을 때만**(원장 `created=false` — 있던 파일을 고칠 때만 백업을 만든다) 격리 폴더로 옮긴다
///     (거부된 항목은 사용자가 되돌릴 근거로 남긴다). 원장 파일 자체는 쓰지 않는다(`~/.cys` 와 함께 격리된다). 호출부(`factory_reset`)는 결과를 보고만 한다.
pub fn reset_entries(entries: &[LedgerEntry], home: &Path, backup_dir: &Path) -> Vec<ResetTrace> {
    let _serial = MUTATION_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut out = Vec::with_capacity(entries.len());
    for entry in entries {
        // (2차 검토 MINOR) 거부할 경로(개인 프로필·상대 경로)에는 잠금 파일부터 만들지 않는다 — 검증이 먼저다(킬스위치 되돌림과 같은 순서).
        if let Err(e) = validate_ledger_entry(entry, home) {
            out.push(ResetTrace { dir: entry.dir.clone(), outcome: RollbackOutcome::Refused(e), backup_moved: false });
            continue;
        }
        let _settings_lock = crate::pack::acquire_settings_lock(&entry.settings);
        let outcome = match rollback_entry_with(entry, home, BackupWhere::Dir(backup_dir)) {
            Ok(o) => o,
            Err(e) => RollbackOutcome::Refused(e),
        };
        let mut backup_moved = false;
        if !entry.created && !matches!(outcome, RollbackOutcome::Refused(_)) {
            let beside = PathBuf::from(format!("{}{BACKUP_SUFFIX}", entry.settings.display()));
            if std::fs::symlink_metadata(&beside).map(|m| m.is_file()).unwrap_or(false) {
                let moved = std::fs::create_dir_all(backup_dir).is_ok() && {
                    let dest = reset_backup_dest(backup_dir, &reset_backup_name(&entry.dir, BACKUP_SUFFIX));
                    std::fs::rename(&beside, &dest).is_ok()
                        || (std::fs::copy(&beside, &dest).is_ok() && std::fs::remove_file(&beside).is_ok())
                };
                backup_moved = moved;
            }
        }
        out.push(ResetTrace { dir: entry.dir.clone(), outcome, backup_moved });
    }
    out
}

fn backup(settings: &Path, raw: &[u8]) -> Result<(), String> {
    let dest = PathBuf::from(format!("{}{BACKUP_SUFFIX}", settings.display()));
    crate::pack::write_atomic_mode(&dest, raw, surgery::file_mode(settings))
        .map_err(|e| format!("백업 실패({}): {e}", dest.display()))
}

/// `<config_dir>/settings.json` 에 `tui` 키가 **없을 때만** `"default"` 를 넣는다(게이트 무관 — 게이트는 호출부
/// [`reconcile_for_launch`] 가 건다). `home` = 개인 프로필 판정 기준(주입형 — 시험은 가짜 홈을 준다).
pub fn ensure_classic(config_dir: &Path, home: &Path, origin: DirOrigin) -> Outcome {
    // 원장 경로의 의미가 다음 프로세스의 작업 폴더에 따라 바뀌어서는 안 된다.
    if !config_dir.is_absolute() {
        return Outcome::Refused("상대 설정 경로는 안전하게 되돌릴 수 없어 쓰지 않는다".into());
    }
    if is_personal_profile(config_dir, home, origin) {
        return Outcome::PersonalProfile;
    }
    if !config_dir.is_dir() {
        return Outcome::NoConfigDir;
    }
    let _serial = match MUTATION_LOCK.lock() {
        Ok(lock) => lock,
        Err(_) => return Outcome::Refused("조정 락이 손상되었다".into()),
    };
    let settings = config_dir.join("settings.json");
    // ★훅 병합(pack.rs `merge_desired_hooks`) · python preflight 와 **같은** 락 파일로 RMW 를 직렬화한다
    //   (유닉스만 — 윈도우는 그 함수 doc 의 감수 범위: 재판독 대조가 남은 창을 좁힌다). `_lock` 바인딩이 계약.
    let _lock = crate::pack::acquire_settings_lock(&settings);
    let loaded = match surgery::load(&settings) {
        Ok(l) => l,
        Err(e) => return Outcome::Refused(e),
    };
    if let Some((_, _, _, Some(v))) = &loaded {
        if let Some(cur) = v.get(TUI_KEY) {
            return Outcome::AlreadySet(cur.to_string());
        }
    }
    let (orig_raw, new_bytes, mode, created) = match &loaded {
        None => {
            let body = match render_with_classic("{}\n") {
                Ok(b) => b,
                Err(e) => return Outcome::Refused(e),
            };
            if let Err(e) = verify_render(&serde_json::json!({}), &body) {
                return Outcome::Refused(e);
            }
            (None, body.into_bytes(), None, true)
        }
        Some((raw, text, bom, v)) => {
            if let Err(e) = surgery::probe_writable(&settings) {
                return Outcome::Refused(e);
            }
            let (src, orig) = match v {
                Some(v) => (text.as_str(), v.clone()),
                None => ("{}\n", serde_json::json!({})), // 빈 파일 — 새 본문으로
            };
            let body = match render_with_classic(src) {
                Ok(b) => b,
                Err(e) => return Outcome::Refused(e),
            };
            if let Err(e) = verify_render(&orig, &body) {
                return Outcome::Refused(e);
            }
            if let Err(e) = backup(&settings, raw) {
                return Outcome::Refused(e);
            }
            let mut bytes = Vec::with_capacity(body.len() + 3);
            if *bom {
                bytes.extend_from_slice(b"\xef\xbb\xbf");
            }
            bytes.extend_from_slice(body.as_bytes());
            (Some(raw.clone()), bytes, surgery::file_mode(&settings), false)
        }
    };
    // 쓰기 직전 재판독 대조 — 그 사이 claude(다른 좌석의 `/tui` · 업셀 체험 영속 등)가 파일을 바꿨으면 쓰지 않는다.
    let current = match surgery::load(&settings) {
        Ok(loaded) => loaded.map(|(raw, _, _, _)| raw),
        Err(e) => return Outcome::Refused(format!("쓰기 직전 판독 실패: {e}")),
    };
    if current.as_deref() != orig_raw.as_deref() {
        return Outcome::Refused("그 사이 다른 프로그램(claude 등)이 파일을 바꿨다 — 이번에는 쓰지 않는다".into());
    }
    if let Err(e) = crate::pack::write_atomic_mode(&settings, &new_bytes, mode) {
        return Outcome::Refused(format!("원자 쓰기 실패: {e}"));
    }
    // 되읽기 확인 — 반환값은 반영의 증거가 아니다.
    match surgery::load(&settings) {
        Ok(Some((_, _, _, Some(v)))) if v.get(TUI_KEY).and_then(Value::as_str) == Some(TUI_CLASSIC) => {
            Outcome::Written {
                created,
                ledger_error: append_ledger(home, config_dir, created).err(),
            }
        }
        Ok(_) => Outcome::Refused("되읽기: tui 가 들어가 있지 않다".into()),
        Err(e) => Outcome::Refused(format!("되읽기 실패: {e}")),
    }
}

/// claude 기동 직전 호출용 — Windows 좌석 spec 경로만 조정한다. 킬스위치면 원장을 프로세스당 한 번 되돌린다.
/// ★호출부 계약: 결과가 무엇이든 **기동을 계속한다**(결과별 로그만). 시험 · 다른 OS 는 HOME 판독 전 무동작.
pub fn reconcile_for_launch(config_dir: &Path) -> Option<Outcome> {
    if cfg!(test) || std::env::consts::OS != "windows" {
        return None; // 테스트 빌드는 실 HOME·실 설정 폴더를 절대 만지지 않는다
    }
    if kill_switch_on() {
        return if ROLLBACK_STARTED
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            Some(Outcome::RolledBack(rollback_from_ledger(&crate::home_dir())))
        } else {
            None
        };
    }
    Some(ensure_classic(config_dir, &crate::home_dir(), DirOrigin::SeatSpec))
}

/// 사람용 한 줄(로그). `None` = 알릴 것 없음(이미 설정됨 · 폴더 없음 등 정상 무동작).
pub fn describe(o: &Outcome, config_dir: &Path) -> Option<String> {
    let p = config_dir.join("settings.json");
    let p = p.display();
    Some(match o {
        Outcome::NoConfigDir | Outcome::RolledBack(_) => return None,
        // ★(성찰 M3) fullscreen 이 이미 적혀 있으면 침묵하지 않는다 — 덮지는 않되(사용자 선택과 Claude Code 승격을 구분 못 한다) 폴더당 1회 안내.
        Outcome::AlreadySet(v) if already_set_is_fullscreen(v) => {
            let mut dirs = FULLSCREEN_DESCRIBED.get_or_init(|| Mutex::new(HashSet::new())).lock().ok()?;
            if !dirs.insert(normalize_profile_path(config_dir, cfg!(windows))) {
                return None;
            }
            fullscreen_already_set_line(&config_dir.join("settings.json"))
        }
        Outcome::AlreadySet(_) => return None,
        Outcome::PersonalProfile => {
            let mut dirs = PERSONAL_DESCRIBED.get_or_init(|| Mutex::new(HashSet::new())).lock().ok()?;
            if !dirs.insert(normalize_profile_path(config_dir, cfg!(windows))) {
                return None;
            }
            format!("{p} 는 개인 프로필이라 렌더러 설정을 넣지 않았습니다(전체화면으로 뜨면 그 pane 에서 /tui default)")
        }
        Outcome::Written { created, ledger_error } => format!(
            "{p} 에 \"tui\": \"default\"(classic 렌더러 — 휠 스크롤)를 넣었습니다{} · 이미 떠 있는 전체화면 좌석은 \
             다음 기동부터 적용 · 끄기: {KILL_ENV}=1 또는 ~/{KILL_FILE}(원장에 기록된 default 만 제거, 사용자 변경값 보존){}",
            if *created { "(파일 새로 만듦)" } else { "" },
            ledger_error.as_ref().map(|e| format!(" · 원장 기록 실패(자동 되돌림 불가): {e}")).unwrap_or_default()
        ),
        Outcome::Refused(why) => format!("{p} 를 건드리지 않았습니다 — {why}"),
    })
}

/// 되돌림 항목별 한 줄. 값 변경 · 파일 없음은 정상 무동작이라 알리지 않는다.
pub fn describe_rollback(outcome: &RollbackOutcome, dir: &Path) -> Option<String> {
    Some(match outcome {
        RollbackOutcome::Removed => format!(
            "{} 에 cys 가 넣었던 \"tui\": \"default\" 를 제거했습니다",
            dir.display()
        ),
        RollbackOutcome::Dropped => return None,
        RollbackOutcome::Refused(why) => format!("{} 되돌림을 완료하지 못했습니다(원장 보존) — {why}", dir.display()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sandbox(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "cys-claude-tui-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn claude_tui_gate_is_windows_only_and_kill_switch_wins() {
        assert!(gate_for_os("windows", false), "Windows 기본 = 기록");
        assert!(!gate_for_os("windows", true), "Windows + 킬스위치 = 기록 안 함");
        assert!(
            !gate_for_os("macos", false),
            "macOS 는 D5 env 가 기본 — 사용자 설정을 바꾸지 않는다"
        );
        assert!(!gate_for_os("linux", false));
        // 판독 규약 — 엄격 비교(형제 게이트와 동형)
        assert!(!kill_switch_from(None, false), "기본 = 킬스위치 꺼짐(=기록 켜짐)");
        assert!(kill_switch_from(Some("1"), false));
        assert!(kill_switch_from(None, true));
        for loose in ["true", "yes", "", "0", " 1"] {
            assert!(
                !kill_switch_from(Some(loose), false),
                "느슨한 truthy {loose:?} 는 참이 아니다"
            );
        }
    }

    #[test]
    fn claude_tui_key_value_pins_match_claude_code_schema() {
        // 2.1.291 스키마 `tui: enum["default","fullscreen"]` · "default" = classic main-screen 렌더러(모듈 doc 증거).
        assert_eq!(TUI_KEY, "tui");
        assert_eq!(TUI_CLASSIC, "default");
        assert_ne!(
            BACKUP_SUFFIX, ".bak-cys",
            "훅 병합 백업과 이름이 같으면 정상 백업을 클로버한다"
        );
    }

    #[test]
    fn claude_tui_render_preserves_bytes_and_layout() {
        let pretty = "{\n  \"hooks\": {\n    \"SessionStart\": []\n  },\n  \"model\": \"opus\"\n}\n";
        let out = render_with_classic(pretty).unwrap();
        assert_eq!(
            out,
            "{\n  \"hooks\": {\n    \"SessionStart\": []\n  },\n  \"model\": \"opus\",\n  \"tui\": \"default\"\n}\n"
        );
        verify_render(&serde_json::from_str(pretty).unwrap(), &out).unwrap();
        // CRLF · 4칸 들여쓰기
        let crlf = "{\r\n    \"a\": 1\r\n}";
        assert_eq!(
            render_with_classic(crlf).unwrap(),
            "{\r\n    \"a\": 1,\r\n    \"tui\": \"default\"\r\n}"
        );
        // 한 줄 압축형은 압축형으로
        assert_eq!(
            render_with_classic("{\"a\":1}").unwrap(),
            "{\"a\":1,\"tui\":\"default\"}"
        );
        // 빈 객체
        assert_eq!(render_with_classic("{}\n").unwrap(), "{\n  \"tui\": \"default\"\n}\n");
        // 이미 있으면 거부(값 무관)
        assert!(render_with_classic("{\"tui\":\"fullscreen\"}").is_err());
        assert!(render_with_classic("{\"tui\":null}").is_err());
        for original in [pretty, crlf, "{\"a\":1}", "{}\n"] {
            assert_eq!(
                render_without_classic(&render_with_classic(original).unwrap()).unwrap(),
                original
            );
        }
    }

    #[test]
    fn claude_tui_verify_rejects_collateral_change() {
        let orig = json!({"a": 1});
        assert!(verify_render(&orig, "{\"a\":2,\"tui\":\"default\"}").is_err());
        assert!(verify_render(&orig, "{\"a\":1,\"b\":0,\"tui\":\"default\"}").is_err());
        assert!(verify_render(&orig, "{\"a\":1,\"tui\":\"fullscreen\"}").is_err());
        assert!(
            verify_render(&orig, "{\"a\":1}").is_err(),
            "결측은 값이 아니다 — tui 부재는 실패"
        );
        verify_render(&orig, "{\"a\":1,\"tui\":\"default\"}").unwrap();
    }

    #[test]
    fn claude_tui_ensure_writes_once_and_never_overwrites() {
        let home = sandbox("home");
        let cfg = home.join(".cys").join("claude");
        // ① 폴더 없음 → 만들지 않는다
        assert_eq!(ensure_classic(&cfg, &home, DirOrigin::SeatSpec), Outcome::NoConfigDir);
        assert!(!cfg.exists());
        std::fs::create_dir_all(&cfg).unwrap();
        let s = cfg.join("settings.json");
        // ② 파일 없음 → 새로 만든다
        assert_eq!(
            ensure_classic(&cfg, &home, DirOrigin::SeatSpec),
            Outcome::Written {
                created: true,
                ledger_error: None
            }
        );
        let v: Value = serde_json::from_slice(&std::fs::read(&s).unwrap()).unwrap();
        assert_eq!(v, json!({"tui": "default"}));
        // ③ 멱등 — 이미 있으면 무동작(바이트 불변)
        let before = std::fs::read(&s).unwrap();
        assert_eq!(
            ensure_classic(&cfg, &home, DirOrigin::SeatSpec),
            Outcome::AlreadySet("\"default\"".into())
        );
        assert_eq!(std::fs::read(&s).unwrap(), before);
        // ④ 사용자 값 불가침 — fullscreen 도, null 도
        for user in ["{\"tui\":\"fullscreen\",\"x\":1}", "{\"tui\":null}"] {
            std::fs::write(&s, user).unwrap();
            assert!(matches!(
                ensure_classic(&cfg, &home, DirOrigin::SeatSpec),
                Outcome::AlreadySet(_)
            ));
            assert_eq!(std::fs::read_to_string(&s).unwrap(), user);
        }
        // ⑤ 기존 파일(BOM · 훅) → 한 칸 추가 + 백업(원문) + 훅 백업 이름 무접촉
        let orig = "\u{feff}{\n  \"hooks\": {\"SessionStart\": [1]}\n}\n";
        std::fs::write(&s, orig).unwrap();
        std::fs::write(cfg.join("settings.json.bak-cys"), "HOOK-BACKUP").unwrap();
        assert_eq!(
            ensure_classic(&cfg, &home, DirOrigin::SeatSpec),
            Outcome::Written {
                created: false,
                ledger_error: None
            }
        );
        let now = std::fs::read_to_string(&s).unwrap();
        assert!(now.starts_with('\u{feff}'), "BOM 보존");
        assert_eq!(
            now,
            "\u{feff}{\n  \"hooks\": {\"SessionStart\": [1]},\n  \"tui\": \"default\"\n}\n"
        );
        assert_eq!(
            std::fs::read_to_string(cfg.join("settings.json.bak-cys-tui")).unwrap(),
            orig
        );
        assert_eq!(
            std::fs::read_to_string(cfg.join("settings.json.bak-cys")).unwrap(),
            "HOOK-BACKUP"
        );
        // ⑥ 빈 파일 → 새 본문
        std::fs::write(&s, "").unwrap();
        assert_eq!(
            ensure_classic(&cfg, &home, DirOrigin::SeatSpec),
            Outcome::Written {
                created: false,
                ledger_error: None
            }
        );
        // ⑦ 깨진 JSON · 루트가 배열 → 쓰지 않는다(바이트 불변)
        for bad in ["{\"a\": ", "[1,2]", "not json"] {
            std::fs::write(&s, bad).unwrap();
            assert!(
                matches!(ensure_classic(&cfg, &home, DirOrigin::SeatSpec), Outcome::Refused(_)),
                "{bad:?}"
            );
            assert_eq!(std::fs::read_to_string(&s).unwrap(), bad);
        }
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn claude_tui_personal_profile_is_never_touched() {
        let home = sandbox("pp");
        for name in [".claude", ".claude-3", ".claude-anything"] {
            let d = home.join(name);
            std::fs::create_dir_all(&d).unwrap();
            assert!(is_personal_profile(&d, &home, DirOrigin::Implicit));
            assert_eq!(ensure_classic(&d, &home, DirOrigin::Implicit), Outcome::PersonalProfile);
            assert!(!d.join("settings.json").exists());
            if name == ".claude" {
                assert!(is_personal_profile(&d, &home, DirOrigin::SeatSpec));
                assert_eq!(ensure_classic(&d, &home, DirOrigin::SeatSpec), Outcome::PersonalProfile);
                assert!(!d.join("settings.json").exists());
            } else {
                assert!(!is_personal_profile(&d, &home, DirOrigin::SeatSpec));
                assert_eq!(
                    ensure_classic(&d, &home, DirOrigin::SeatSpec),
                    Outcome::Written {
                        created: true,
                        ledger_error: None
                    }
                );
                assert!(d.join("settings.json").exists());
            }
        }
        // cys 격리 폴더 · 비슷한 이름은 개인 프로필이 아니다.
        for name in [".cys/claude", ".cys/claude-dept-a", ".claudex", ".claude-3/child"] {
            assert!(!is_personal_profile(&home.join(name), &home, DirOrigin::Implicit));
        }
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn claude_tui_profile_normalization_is_lexical_and_portable() {
        for (input, windows, expected) in [
            ("/Users/x/./.claude-3/../.claude/", false, "/Users/x/.claude"),
            ("/Users/x/../../..", false, "/"),
            ("../x/../.claude", false, "../.claude"),
            ("/Users/x/.Claude", false, "/Users/x/.Claude"),
            (r"C:\Users\x\.claude-3\", true, "c:/users/x/.claude-3"),
            (r"C:\Users\x\.\UNUSED\..\.CLAUDE", true, "c:/users/x/.claude"),
            (r"\\?\C:\Users\x\.CLAUDE-3\..\.claude\", true, "c:/users/x/.claude"),
            ("C:/Users/x/./.Claude-3/", true, "c:/users/x/.claude-3"),
            (
                r"\\?\UNC\Server\Share\Users\x\..\.claude\",
                true,
                "//server/share/users/.claude",
            ),
            (r"\\Server\Share\..\..\.claude", true, "//server/share/.claude"),
            (r"C:\..\.claude", true, "c:/.claude"),
            (r"C:..\.claude", true, "c:../.claude"),
        ] {
            assert_eq!(normalize_profile_path(Path::new(input), windows), expected, "{input}");
        }
        let home = Path::new("/Users/x/.");
        assert!(is_personal_profile(
            Path::new("/Users/x/a/../.claude/"),
            home,
            DirOrigin::SeatSpec
        ));
        assert!(is_personal_profile(
            Path::new("/Users/x/.claude-3/"),
            home,
            DirOrigin::Implicit
        ));
        assert!(!is_personal_profile(
            Path::new("/Users/x/.claude-3/"),
            home,
            DirOrigin::SeatSpec
        ));
        assert!(is_personal_profile(
            Path::new("./.claude"),
            Path::new("."),
            DirOrigin::SeatSpec
        ));
    }

    #[cfg(windows)]
    #[test]
    fn claude_tui_windows_personal_comparison_ignores_case_and_verbatim_prefix() {
        let home = Path::new(r"C:\Users\x\");
        assert!(is_personal_profile(
            Path::new(r"\\?\C:\USERS\X\a\..\.CLAUDE\"),
            home,
            DirOrigin::SeatSpec
        ));
        assert!(is_personal_profile(
            Path::new(r"C:\USERS\X\.CLAUDE-3"),
            home,
            DirOrigin::Implicit
        ));
        assert!(!is_personal_profile(
            Path::new(r"C:\USERS\X\.CLAUDE-3"),
            home,
            DirOrigin::SeatSpec
        ));
        assert!(is_personal_profile(
            Path::new(r"\\?\UNC\SERVER\SHARE\x\.claude"),
            Path::new(r"\\server\share\x"),
            DirOrigin::SeatSpec
        ));
    }

    #[test]
    fn claude_tui_personal_refusal_is_described_once_per_dir() {
        let home = sandbox("describe-once");
        let dir = home.join(".claude");
        assert!(describe(&Outcome::PersonalProfile, &dir).is_some());
        assert_eq!(describe(&Outcome::PersonalProfile, &dir), None);
        assert_eq!(describe(&Outcome::PersonalProfile, &dir.join(".")), None);
        assert!(describe(&Outcome::PersonalProfile, &home.join(".claude-other")).is_some());
        let _ = std::fs::remove_dir_all(home);
    }

    /// ★(성찰 M3) 이미 `fullscreen` 이면 덮지 않되 침묵하지 않는다 — 폴더당 1회 `/tui default` 안내 · `default`·그 밖 값은 종전처럼 무언.
    #[test]
    fn claude_tui_already_fullscreen_is_described_once_with_tui_default_guidance() {
        let home = sandbox("describe-fullscreen");
        let dir = home.join(".claude-7");
        assert!(already_set_is_fullscreen("\"fullscreen\"") && !already_set_is_fullscreen("\"default\"") && !already_set_is_fullscreen("null") && !already_set_is_fullscreen("fullscreen"));
        let line = describe(&Outcome::AlreadySet("\"fullscreen\"".into()), &dir).expect("첫 안내");
        assert!(line.contains("fullscreen") && line.contains("/tui default") && line.contains("휠 스크롤") && line.contains("덮지 않습니다"), "{line}");
        assert_eq!(describe(&Outcome::AlreadySet("\"fullscreen\"".into()), &dir), None, "같은 폴더는 1회");
        assert_eq!(describe(&Outcome::AlreadySet("\"fullscreen\"".into()), &dir.join(".")), None, "정규화된 같은 폴더");
        assert!(describe(&Outcome::AlreadySet("\"fullscreen\"".into()), &home.join(".claude-8")).is_some());
        assert_eq!(describe(&Outcome::AlreadySet("\"default\"".into()), &home.join(".claude-9")), None, "classic 은 알릴 것 없음");
        assert_eq!(describe(&Outcome::AlreadySet("null".into()), &home.join(".claude-9")), None);
        // doctor 판독기 — 폴더 없음 · 키 없음 · default · fullscreen · 그 밖 · 판독 실패(심볼릭 링크).
        assert_eq!(tui_probe(&home.join("nope")), TuiProbe::NoConfigDir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(tui_probe(&dir), TuiProbe::Absent, "settings.json 없음");
        let s = dir.join("settings.json");
        std::fs::write(&s, "{\"hooks\":{}}").unwrap();
        assert_eq!(tui_probe(&dir), TuiProbe::Absent);
        std::fs::write(&s, "{\"tui\":\"default\"}").unwrap();
        assert_eq!(tui_probe(&dir), TuiProbe::Classic);
        std::fs::write(&s, "\u{feff}{\n  \"tui\": \"fullscreen\"\n}\n").unwrap();
        assert_eq!(tui_probe(&dir), TuiProbe::Fullscreen);
        std::fs::write(&s, "{\"tui\":null}").unwrap();
        assert_eq!(tui_probe(&dir), TuiProbe::Other("null".into()));
        std::fs::write(&s, "{not json").unwrap();
        assert!(matches!(tui_probe(&dir), TuiProbe::Unreadable(_)));
        // 중복 폴더는 한 번 · 원장 폴더 열거(없으면 빈 목록).
        std::fs::write(&s, "{\"tui\":\"fullscreen\"}").unwrap();
        let got = tui_probe_dirs(&[dir.clone(), dir.join("."), home.join("nope")]);
        assert_eq!(got.len(), 2, "{got:?}");
        assert_eq!(got[0].1, TuiProbe::Fullscreen);
        assert!(ledger_dirs(&home).is_empty());
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn claude_tui_removal_handles_positions_and_rejects_ambiguous_values() {
        for (input, expected) in [
            ("{\"tui\":\"default\",\"a\":1}", "{\"a\":1}"),
            ("{\"a\":1,\"tui\":\"default\",\"b\":2}", "{\"a\":1,\"b\":2}"),
            (
                "{\"nested\":{\"tui\":\"default\"},\"tui\":\"default\"}",
                "{\"nested\":{\"tui\":\"default\"}}",
            ),
            ("\u{feff}{\r\n    \"tui\": \"default\"\r\n}\r\n", "\u{feff}{}\r\n"),
            ("{\"tui\":\"def\\u0061ult\"}", "{}"),
        ] {
            assert_eq!(render_without_classic(input).unwrap(), expected);
        }
        for input in [
            "{}",
            "{\"tui\":null}",
            "{\"tui\":\"fullscreen\"}",
            "{\"tui\":\"default\",\"tui\":\"default\"}",
            "[]",
            "{\"tui\":\"default\"} garbage",
        ] {
            assert!(render_without_classic(input).is_err(), "{input}");
        }
    }

    #[test]
    fn claude_tui_verify_removal_rejects_collateral_change() {
        let orig = json!({"a": {"nested": true}, "tui": "default"});
        verify_removal(&orig, "{\"a\":{\"nested\":true}}").unwrap();
        for text in [
            "{}",
            "{\"a\":2}",
            "{\"a\":{\"nested\":true},\"new\":1}",
            "{\"a\":{\"nested\":true},\"tui\":null}",
            "[]",
            "broken",
        ] {
            assert!(verify_removal(&orig, text).is_err(), "{text}");
        }
        assert!(verify_removal(&json!([]), "{}").is_err());
    }

    #[test]
    fn claude_tui_ledger_records_dedupes_and_caps() {
        let home = sandbox("ledger");
        let cfg = home.join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        assert_eq!(
            ensure_classic(&cfg, &home, DirOrigin::SeatSpec),
            Outcome::Written {
                created: true,
                ledger_error: None
            }
        );
        let path = home.join(LEDGER_FILE);
        let (ledger, _) = read_ledger(&path).unwrap();
        assert_eq!(ledger.version, 1);
        assert_eq!(ledger.entries.len(), 1);
        assert_eq!(ledger.entries[0].dir, cfg);
        assert_eq!(ledger.entries[0].settings, cfg.join("settings.json"));
        assert!(ledger.entries[0].created);
        assert!(ledger.entries[0].written_at > 0);
        // 다시 키를 넣어도 같은 폴더의 원장 항목은 하나다.
        std::fs::write(cfg.join("settings.json"), "{}").unwrap();
        assert_eq!(
            ensure_classic(&cfg, &home, DirOrigin::SeatSpec),
            Outcome::Written {
                created: false,
                ledger_error: None
            }
        );
        let (mut ledger, raw) = read_ledger(&path).unwrap();
        assert_eq!(ledger.entries.len(), 1);
        assert!(!ledger.entries[0].created);
        ledger.entries[0].written_at = 1; // 가장 오래된 항목이 빠지는지 시간과 무관하게 검증.
        write_ledger(&path, &ledger, raw.as_deref()).unwrap();
        for i in 0..MAX_LEDGER_ENTRIES {
            let dir = home.join(format!("seat-{i}"));
            std::fs::create_dir(&dir).unwrap();
            assert_eq!(
                ensure_classic(&dir, &home, DirOrigin::SeatSpec),
                Outcome::Written {
                    created: true,
                    ledger_error: None
                }
            );
        }
        let (ledger, _) = read_ledger(&path).unwrap();
        assert_eq!(ledger.entries.len(), MAX_LEDGER_ENTRIES);
        assert!(!ledger.entries.iter().any(|e| e.dir == cfg));
        assert!(ledger
            .entries
            .iter()
            .any(|e| e.dir == home.join(format!("seat-{}", MAX_LEDGER_ENTRIES - 1))));
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn claude_tui_rollback_restores_bytes_backup_and_is_idempotent() {
        let home = sandbox("rollback-bytes");
        assert!(rollback_from_ledger(&home).is_empty());
        assert!(!home.join(".cys").exists());
        for (i, original) in [
            "{\n  \"hooks\": {\"SessionStart\": []},\n  \"model\": \"opus\"\n}\n",
            "{\r\n    \"a\": 1\r\n}\r\n",
            "{\"a\":1,\"b\":[2,3]}",
            "\u{feff}{\r\n    \"a\": 1\r\n}\r\n",
            "{}\n",
        ]
        .iter()
        .enumerate()
        {
            let dir = home.join(format!("cfg-{i}"));
            std::fs::create_dir(&dir).unwrap();
            let settings = dir.join("settings.json");
            std::fs::write(&settings, original).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&settings, std::fs::Permissions::from_mode(0o600)).unwrap();
            }
            assert_eq!(
                ensure_classic(&dir, &home, DirOrigin::SeatSpec),
                Outcome::Written {
                    created: false,
                    ledger_error: None
                }
            );
            let inserted = std::fs::read(&settings).unwrap();
            assert_eq!(
                rollback_from_ledger(&home),
                vec![(dir.clone(), RollbackOutcome::Removed)]
            );
            assert_eq!(std::fs::read(&settings).unwrap(), original.as_bytes());
            assert_eq!(std::fs::read(dir.join("settings.json.bak-cys-tui")).unwrap(), inserted);
            #[cfg(unix)]
            assert_eq!(surgery::file_mode(&settings), Some(0o600));
            assert!(read_ledger(&home.join(LEDGER_FILE)).unwrap().0.entries.is_empty());
            assert!(rollback_from_ledger(&home).is_empty());
        }
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn claude_tui_rollback_drops_user_changes_absent_and_missing() {
        let home = sandbox("rollback-skip");
        for (i, user) in [
            Some("{\"tui\":\"fullscreen\",\"x\":1}"),
            Some("{\"tui\":null}"),
            Some("{\"x\":1}"),
            None,
        ]
        .iter()
        .enumerate()
        {
            let dir = home.join(format!("cfg-{i}"));
            std::fs::create_dir(&dir).unwrap();
            let settings = dir.join("settings.json");
            assert!(matches!(
                ensure_classic(&dir, &home, DirOrigin::SeatSpec),
                Outcome::Written { ledger_error: None, .. }
            ));
            match user {
                Some(text) => std::fs::write(&settings, text).unwrap(),
                None => std::fs::remove_file(&settings).unwrap(),
            }
            assert_eq!(
                rollback_from_ledger(&home),
                vec![(dir.clone(), RollbackOutcome::Dropped)]
            );
            assert_eq!(std::fs::read_to_string(&settings).ok().as_deref(), *user);
            assert!(!dir.join("settings.json.bak-cys-tui").exists());
            assert!(read_ledger(&home.join(LEDGER_FILE)).unwrap().0.entries.is_empty());
        }
        assert!(rollback_from_ledger(&home).is_empty());
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn claude_tui_corrupt_ledger_is_preserved_and_reported() {
        let home = sandbox("corrupt-ledger");
        let dir = home.join("cfg");
        std::fs::create_dir(&dir).unwrap();
        assert!(matches!(
            ensure_classic(&dir, &home, DirOrigin::SeatSpec),
            Outcome::Written { ledger_error: None, .. }
        ));
        let before = std::fs::read(dir.join("settings.json")).unwrap();
        let path = home.join(LEDGER_FILE);
        for bad in [
            "{bad-json",
            "",
            "{\"version\":2,\"entries\":[]}",
            "{\"version\":1,\"entries\":[{}]}",
        ] {
            std::fs::write(&path, bad).unwrap();
            let outcomes = rollback_from_ledger(&home);
            assert!(matches!(outcomes.as_slice(), [(_, RollbackOutcome::Refused(_))]));
            assert!(describe_rollback(&outcomes[0].1, &outcomes[0].0).is_some());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), bad);
            assert_eq!(std::fs::read(dir.join("settings.json")).unwrap(), before);
            // 훼손 원장을 빈 데이터로 덮지 않는다. 설정 삽입 성공과 원장 실패를 함께 보고한다.
            std::fs::write(dir.join("settings.json"), "{}\n").unwrap();
            let outcome = ensure_classic(&dir, &home, DirOrigin::SeatSpec);
            assert!(matches!(
                outcome,
                Outcome::Written {
                    ledger_error: Some(_),
                    ..
                }
            ));
            assert!(describe(&outcome, &dir).unwrap().contains("원장 기록 실패"));
            assert_eq!(std::fs::read_to_string(&path).unwrap(), bad);
        }
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn claude_tui_rollback_retains_parse_failures_and_invalid_paths() {
        let home = sandbox("rollback-invalid");
        let dir = home.join("cfg");
        std::fs::create_dir(&dir).unwrap();
        assert!(matches!(
            ensure_classic(&dir, &home, DirOrigin::SeatSpec),
            Outcome::Written { .. }
        ));
        std::fs::write(dir.join("settings.json"), "{broken").unwrap();
        assert!(matches!(
            rollback_from_ledger(&home).as_slice(),
            [(_, RollbackOutcome::Refused(_))]
        ));
        assert_eq!(std::fs::read_to_string(dir.join("settings.json")).unwrap(), "{broken");
        let path = home.join(LEDGER_FILE);
        let (mut ledger, _) = read_ledger(&path).unwrap();
        assert_eq!(ledger.entries.len(), 1);
        let personal = home.join(".claude");
        std::fs::create_dir(&personal).unwrap();
        std::fs::write(personal.join("settings.json"), "{\"tui\":\"default\"}").unwrap();
        for entry in [
            LedgerEntry {
                dir: personal.clone(),
                settings: personal.join("settings.json"),
                written_at: 1,
                created: false,
            },
            LedgerEntry {
                dir: dir.clone(),
                settings: personal.join("settings.json"),
                written_at: 1,
                created: false,
            },
        ] {
            ledger.entries = vec![entry];
            std::fs::write(&path, serde_json::to_vec(&ledger).unwrap()).unwrap();
            assert!(matches!(
                rollback_from_ledger(&home).as_slice(),
                [(_, RollbackOutcome::Refused(_))]
            ));
            assert_eq!(read_ledger(&path).unwrap().0.entries.len(), 1);
            assert_eq!(
                std::fs::read_to_string(personal.join("settings.json")).unwrap(),
                "{\"tui\":\"default\"}"
            );
            assert!(!personal.join("settings.json.cys-lock").exists());
        }
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn claude_tui_relative_paths_are_refused_before_writes() {
        let home = sandbox("relative-refusal");
        let dir = home.join("cfg");
        std::fs::create_dir(&dir).unwrap();
        let settings = dir.join("settings.json");
        let original = "{\"tui\":\"default\",\"owner\":\"user\"}";
        std::fs::write(&settings, original).unwrap();
        // 존재 확인도 하기 전에 거부한다. 시험 중 작업 폴더 변경이나 상대 경로 파일 I/O 는 없다.
        let relative = PathBuf::from("cys-tui-relative-must-refuse");
        assert!(matches!(
            ensure_classic(&relative, &home, DirOrigin::SeatSpec),
            Outcome::Refused(why) if why.contains("상대")
        ));
        assert!(!home.join(".cys").exists());
        std::fs::create_dir(home.join(".cys")).unwrap();
        let ledger_path = home.join(LEDGER_FILE);
        for (entry_dir, entry_settings) in [
            (relative.clone(), relative.join("settings.json")),
            (relative.clone(), settings.clone()),
            (dir.clone(), relative.join("settings.json")),
        ] {
            let ledger = Ledger {
                version: 1,
                entries: vec![LedgerEntry {
                    dir: entry_dir,
                    settings: entry_settings,
                    written_at: 1,
                    created: false,
                }],
            };
            let raw = serde_json::to_vec(&ledger).unwrap();
            std::fs::write(&ledger_path, &raw).unwrap();
            let outcomes = rollback_from_ledger(&home);
            assert!(matches!(outcomes.as_slice(), [(_, RollbackOutcome::Refused(why))] if why.contains("상대")));
            assert_eq!(std::fs::read(&ledger_path).unwrap(), raw);
            assert_eq!(std::fs::read_to_string(&settings).unwrap(), original);
            assert!(!dir.join("settings.json.cys-lock").exists());
        }
        let _ = std::fs::remove_dir_all(home);
    }

    #[cfg(unix)]
    #[test]
    fn claude_tui_rollback_keeps_symlink_and_readonly_entries_for_retry() {
        use std::os::unix::fs::PermissionsExt;
        let home = sandbox("rollback-refuse");
        let dir = home.join("cfg");
        std::fs::create_dir(&dir).unwrap();
        let settings = dir.join("settings.json");
        std::fs::write(&settings, "{\"x\":1}").unwrap();
        assert!(matches!(
            ensure_classic(&dir, &home, DirOrigin::SeatSpec),
            Outcome::Written { .. }
        ));
        let inserted = std::fs::read(&settings).unwrap();
        let target = home.join("target.json");
        std::fs::rename(&settings, &target).unwrap();
        std::os::unix::fs::symlink(&target, &settings).unwrap();
        assert!(matches!(
            rollback_from_ledger(&home).as_slice(),
            [(_, RollbackOutcome::Refused(_))]
        ));
        assert_eq!(std::fs::read(&target).unwrap(), inserted);
        assert_eq!(read_ledger(&home.join(LEDGER_FILE)).unwrap().0.entries.len(), 1);
        std::fs::remove_file(&settings).unwrap();
        std::fs::rename(&target, &settings).unwrap();
        std::fs::set_permissions(&settings, std::fs::Permissions::from_mode(0o444)).unwrap();
        assert!(matches!(
            rollback_from_ledger(&home).as_slice(),
            [(_, RollbackOutcome::Refused(_))]
        ));
        assert_eq!(std::fs::read(&settings).unwrap(), inserted);
        assert_eq!(read_ledger(&home.join(LEDGER_FILE)).unwrap().0.entries.len(), 1);
        std::fs::set_permissions(&settings, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(
            rollback_from_ledger(&home),
            vec![(dir.clone(), RollbackOutcome::Removed)]
        );
        assert_eq!(std::fs::read(&settings).unwrap(), b"{\"x\":1}");
        assert_eq!(std::fs::read(dir.join("settings.json.bak-cys-tui")).unwrap(), inserted);
        assert!(rollback_from_ledger(&home).is_empty());
        let _ = std::fs::remove_dir_all(home);
    }

    #[cfg(unix)]
    #[test]
    fn claude_tui_symlink_and_readonly_are_refused() {
        use std::os::unix::fs::PermissionsExt;
        let home = sandbox("sym");
        let cfg = home.join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        let real = home.join("real.json");
        std::fs::write(&real, "{}").unwrap();
        std::os::unix::fs::symlink(&real, cfg.join("settings.json")).unwrap();
        assert!(matches!(
            ensure_classic(&cfg, &home, DirOrigin::SeatSpec),
            Outcome::Refused(_)
        ));
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "{}");
        std::fs::remove_file(cfg.join("settings.json")).unwrap();
        let s = cfg.join("settings.json");
        std::fs::write(&s, "{\"a\":1}").unwrap();
        std::fs::set_permissions(&s, std::fs::Permissions::from_mode(0o444)).unwrap();
        assert!(matches!(
            ensure_classic(&cfg, &home, DirOrigin::SeatSpec),
            Outcome::Refused(_)
        ));
        assert_eq!(std::fs::read_to_string(&s).unwrap(), "{\"a\":1}");
        std::fs::set_permissions(&s, std::fs::Permissions::from_mode(0o644)).unwrap();
        let _ = std::fs::remove_dir_all(&home);
    }

    /// ★(성찰 2회차 C5) 원장 RMW 는 병합형이다 — 읽은 뒤 다른 프로세스가 원장을 바꿨으면(윈도우에는 설정 락이 없다) 다시 읽어 **그 위에** 적용한다.
    /// 재현: `apply` 첫 호출 때 밖에서 다른 좌석의 항목을 써 넣는다(동시 launch-agent 흉내) → 결과 원장에 두 항목이 모두 있다 · 적용 횟수 2(재시도 1회).
    /// 계속 충돌하면 상한(5회)에서 사유와 함께 포기한다(원장만 빠지고 설정 쓰기는 영향 없음).
    #[test]
    fn claude_tui_ledger_update_merges_on_conflict_and_gives_up_after_cap() {
        let home = sandbox("ledger-merge");
        std::fs::create_dir_all(home.join(".cys")).unwrap();
        let path = home.join(LEDGER_FILE);
        let other = LedgerEntry { dir: home.join("other"), settings: home.join("other/settings.json"), written_at: 5, created: false };
        let mine = LedgerEntry { dir: home.join("mine"), settings: home.join("mine/settings.json"), written_at: 7, created: true };
        let mut applied = 0usize;
        let other_bytes = serde_json::to_vec(&Ledger { version: 1, entries: vec![other.clone()] }).unwrap();
        update_ledger(&path, |l| {
            applied += 1;
            if applied == 1 {
                // 읽기와 쓰기 사이에 다른 프로세스가 원장을 썼다.
                crate::pack::write_atomic(&path, &other_bytes).unwrap();
            }
            l.entries.retain(|e| e.dir != mine.dir);
            l.entries.push(mine.clone());
        })
        .unwrap();
        assert_eq!(applied, 2, "첫 쓰기는 충돌 → 다시 읽어 다시 적용");
        let (ledger, _) = read_ledger(&path).unwrap();
        assert_eq!(ledger.entries, vec![other.clone(), mine.clone()], "다른 프로세스의 항목을 지우지 않고 내 항목을 얹는다");
        // append_ledger 도 같은 경로 — 같은 폴더는 갈아끼우고 다른 폴더는 남는다.
        append_ledger(&home, &home.join("mine"), false).unwrap();
        let (ledger, _) = read_ledger(&path).unwrap();
        assert_eq!(ledger.entries.len(), 2);
        assert!(ledger.entries.iter().any(|e| e.dir == other.dir) && ledger.entries.iter().any(|e| e.dir == mine.dir && !e.created));
        // 계속 충돌 — 상한에서 포기(사유에 재시도 횟수).
        let mut n = 0usize;
        let bump = |n: usize| serde_json::to_vec(&Ledger { version: 1, entries: vec![LedgerEntry { written_at: n as u64, ..other.clone() }] }).unwrap();
        let err = update_ledger(&path, |l| {
            n += 1;
            crate::pack::write_atomic(&path, &bump(100 + n)).unwrap();
            l.entries.push(mine.clone());
        })
        .unwrap_err();
        assert_eq!(n, LEDGER_RETRY_MAX);
        assert!(err.contains("재시도") && err.contains(LEDGER_CONFLICT), "{err}");
        let _ = std::fs::remove_dir_all(home);
    }

    /// ★(성찰 2회차 m4) 완전 초기화의 흔적 되돌림 — 원장 항목으로(원장 파일 없이) 되돌린다: 값이 아직 `"default"` 면 빼고(바이트 원복 · 백업은 격리 폴더 안) ·
    /// cys 가 만든 옆자리 `.bak-cys-tui` 는 우리가 만든 것(created=false)만 격리 폴더로 옮긴다 · 사용자가 바꾼 값은 그대로(Dropped) · 새로 만든 파일(created=true)의
    /// 되돌림은 키만 빼고 옆자리 백업은 건드리지 않는다(우리가 만든 백업이 없다) · 개인 프로필 항목은 거부 · 설정 파일 옆에 새 흔적 0.
    #[test]
    fn claude_tui_reset_entries_removes_our_traces_only() {
        let home = sandbox("reset-entries");
        let trash = home.join("trash").join("settings-backups");
        // ① 있던 파일에 넣은 경우 — 백업이 생긴다.
        let d1 = home.join(".claude-2");
        std::fs::create_dir(&d1).unwrap();
        let orig1 = "{\n  \"model\": \"opus\"\n}\n";
        std::fs::write(d1.join("settings.json"), orig1).unwrap();
        assert!(matches!(ensure_classic(&d1, &home, DirOrigin::SeatSpec), Outcome::Written { created: false, .. }));
        assert!(d1.join("settings.json.bak-cys-tui").is_file());
        // ② 새로 만든 경우 — 백업이 없다.
        let d2 = home.join(".claude-3");
        std::fs::create_dir(&d2).unwrap();
        assert!(matches!(ensure_classic(&d2, &home, DirOrigin::SeatSpec), Outcome::Written { created: true, .. }));
        assert!(!d2.join("settings.json.bak-cys-tui").exists());
        // ③ 사용자가 값을 바꾼 경우 — 건드리지 않는다(우리 백업은 옮긴다).
        let d3 = home.join(".claude-4");
        std::fs::create_dir(&d3).unwrap();
        std::fs::write(d3.join("settings.json"), "{\"a\":1}").unwrap();
        assert!(matches!(ensure_classic(&d3, &home, DirOrigin::SeatSpec), Outcome::Written { created: false, .. }));
        let user3 = "{\"a\":1,\"tui\":\"fullscreen\"}";
        std::fs::write(d3.join("settings.json"), user3).unwrap();
        // ④ 다른 사용자 백업(우리 이름이 아닌 것)은 그대로.
        std::fs::write(d1.join("settings.json.bak-cys"), "x").unwrap();
        let entries = ledger_entries(&home).unwrap();
        assert_eq!(entries.len(), 3);
        // 원장 파일이 격리돼 사라진 뒤에도 항목으로 되돌린다.
        std::fs::remove_file(home.join(LEDGER_FILE)).unwrap();
        let traces = reset_entries(&entries, &home, &trash);
        let by = |d: &Path| traces.iter().find(|t| t.dir == d).unwrap();
        assert_eq!((by(&d1).outcome.clone(), by(&d1).backup_moved), (RollbackOutcome::Removed, true));
        assert_eq!((by(&d2).outcome.clone(), by(&d2).backup_moved), (RollbackOutcome::Removed, false));
        assert_eq!((by(&d3).outcome.clone(), by(&d3).backup_moved), (RollbackOutcome::Dropped, true));
        assert_eq!(std::fs::read_to_string(d1.join("settings.json")).unwrap(), orig1, "바이트 원복");
        assert!(!d1.join("settings.json.bak-cys-tui").exists(), "우리가 만든 백업은 옮겨졌다");
        assert!(d1.join("settings.json.bak-cys").is_file(), "남의 백업은 그대로");
        assert_eq!(std::fs::read_to_string(d3.join("settings.json")).unwrap(), user3, "사용자 변경값 불가침");
        assert!(!d3.join("settings.json.bak-cys-tui").exists());
        let v2: Value = serde_json::from_str(&std::fs::read_to_string(d2.join("settings.json")).unwrap()).unwrap();
        assert!(v2.get(TUI_KEY).is_none(), "새로 만든 파일도 키는 뺀다: {v2}");
        // 격리 폴더에 되돌림 백업 2건(d1·d2 — 되돌린 것만) + 옮긴 옆자리 백업 2건(d1·d3) · 설정 폴더 옆에 새 파일 0.
        let names: Vec<String> = std::fs::read_dir(&trash).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(names.iter().filter(|n| n.starts_with("claude-tui.") && n.ends_with(".settings.json")).count(), 2, "{names:?}");
        assert_eq!(names.iter().filter(|n| n.ends_with(BACKUP_SUFFIX)).count(), 2, "{names:?}");
        for d in [&d1, &d2, &d3] {
            // 설정 락 파일(`.cys-lock` · 유닉스)은 훅 병합과 공유하는 기존 흔적이라 세지 않는다.
            let n = std::fs::read_dir(d).unwrap().filter(|e| !e.as_ref().unwrap().file_name().to_string_lossy().ends_with(".cys-lock")).count();
            assert!(n <= 2, "{}: 설정 폴더에 새 흔적이 생겼다", d.display());
        }
        // 멱등 — 다시 돌리면 전부 Dropped · 옮길 백업 없음.
        let again = reset_entries(&entries, &home, &trash);
        assert!(again.iter().all(|t| t.outcome == RollbackOutcome::Dropped && !t.backup_moved), "{again:?}");
        // 개인 프로필 항목은 거부(킬스위치 되돌림과 같은 규약).
        let personal = LedgerEntry { dir: home.join(".claude"), settings: home.join(".claude/settings.json"), written_at: 1, created: false };
        let t = reset_entries(&[personal], &home, &trash);
        assert!(matches!(t[0].outcome, RollbackOutcome::Refused(_)) && !t[0].backup_moved);
        let _ = std::fs::remove_dir_all(home);
    }

    /// ★(2차 검토 MAJOR-2) 격리 백업 이름은 경로마다 다르고(`a_b` 와 `a/b` 가 같은 이름으로 접히지 않는다) 있는 파일을 덮지 않는다(`.1` `.2` …).
    #[test]
    fn claude_tui_reset_backup_names_do_not_collide_or_overwrite() {
        let home = sandbox("reset-names");
        let a = home.join("seats").join("a_b");
        let b = home.join("seats").join("a").join("b");
        assert_ne!(reset_backup_name(&a, ""), reset_backup_name(&b, ""));
        assert!(reset_backup_name(&a, "").ends_with(".settings.json") && reset_backup_name(&a, BACKUP_SUFFIX).ends_with(BACKUP_SUFFIX));
        let trash = home.join("trash");
        std::fs::create_dir_all(&trash).unwrap();
        let n = reset_backup_name(&a, "");
        let d1 = reset_backup_dest(&trash, &n);
        std::fs::write(&d1, "1").unwrap();
        let d2 = reset_backup_dest(&trash, &n);
        assert_ne!(d1, d2);
        assert_eq!(d2, trash.join(format!("{n}.1")));
        assert_eq!(std::fs::read_to_string(&d1).unwrap(), "1", "있던 사본은 그대로");
        // 실제 되돌림 두 번(같은 폴더 항목 둘) — 두 복구 사본이 모두 남는다.
        std::fs::create_dir_all(&a).unwrap();
        std::fs::write(a.join("settings.json"), "{\"x\":1}").unwrap();
        assert!(matches!(ensure_classic(&a, &home, DirOrigin::SeatSpec), Outcome::Written { .. }));
        let entries = ledger_entries(&home).unwrap();
        let t = reset_entries(&entries, &home, &trash);
        assert_eq!(t[0].outcome, RollbackOutcome::Removed);
        assert!(matches!(ensure_classic(&a, &home, DirOrigin::SeatSpec), Outcome::Written { .. }));
        let t = reset_entries(&entries, &home, &trash);
        assert_eq!(t[0].outcome, RollbackOutcome::Removed);
        let copies = std::fs::read_dir(&trash).unwrap().filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().contains(".settings.json")).count();
        assert!(copies >= 4, "되돌림 사본 2 + 옆자리 백업 2 가 모두 남는다({copies})");
        let _ = std::fs::remove_dir_all(home);
    }

    /// ★(2차 검토 MAJOR-3) 원장에 적힌 폴더가 그 뒤 개인 `~/.claude` 로 가는 심볼릭 링크로 바뀌면 되돌리지 않는다(킬스위치 되돌림·완전 초기화 공통).
    #[cfg(unix)]
    #[test]
    fn claude_tui_rollback_refuses_symlinked_dir_to_personal_profile() {
        let home = sandbox("rollback-symlink");
        let personal = home.join(".claude");
        std::fs::create_dir_all(&personal).unwrap();
        let personal_settings = "{\"tui\":\"default\",\"keep\":true}";
        std::fs::write(personal.join("settings.json"), personal_settings).unwrap();
        let team = home.join(".claude-team");
        std::fs::create_dir_all(&team).unwrap();
        std::fs::write(team.join("settings.json"), "{}").unwrap();
        assert!(matches!(ensure_classic(&team, &home, DirOrigin::SeatSpec), Outcome::Written { created: false, .. }));
        // 폴더를 링크로 갈아 끼운다.
        std::fs::remove_dir_all(&team).unwrap();
        std::os::unix::fs::symlink(&personal, &team).unwrap();
        let out = rollback_from_ledger(&home);
        assert!(matches!(&out[0].1, RollbackOutcome::Refused(why) if why.contains("심볼릭 링크")), "{out:?}");
        assert_eq!(std::fs::read_to_string(personal.join("settings.json")).unwrap(), personal_settings, "개인 설정 불변");
        let entries = ledger_entries(&home).unwrap();
        let t = reset_entries(&entries, &home, &home.join("trash"));
        assert!(matches!(&t[0].outcome, RollbackOutcome::Refused(_)) && !t[0].backup_moved, "{t:?}");
        assert_eq!(std::fs::read_to_string(personal.join("settings.json")).unwrap(), personal_settings);
        let _ = std::fs::remove_dir_all(home);
    }

    /// ★(2차 검토 MAJOR-4 · C5) 원장 전용 잠금 파일 — 생성 배타 · 잡힌 동안은 2초 뒤 포기(None) · 죽은 프로세스의 낡은 잠금(60초 이상)은 치우고 얻는다 · 떨어지면 지운다.
    #[test]
    fn claude_tui_ledger_lock_is_exclusive_and_clears_stale_locks() {
        let home = sandbox("ledger-lock");
        std::fs::create_dir_all(home.join(".cys")).unwrap();
        let path = home.join(LEDGER_FILE);
        let lock_path = PathBuf::from(format!("{}.lock", path.display()));
        let held = ledger_lock(&path).expect("첫 획득");
        assert!(lock_path.is_file());
        let t = std::time::Instant::now();
        assert!(ledger_lock(&path).is_none(), "잡힌 동안은 얻지 못한다");
        assert!(t.elapsed() >= std::time::Duration::from_millis(LEDGER_LOCK_WAIT_MS - 100));
        drop(held);
        assert!(!lock_path.exists(), "떨어지면 지운다");
        // 낡은 잠금 — mtime 을 과거로.
        std::fs::write(&lock_path, "").unwrap();
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(LEDGER_LOCK_STALE_SECS + 5);
        let f = std::fs::OpenOptions::new().write(true).open(&lock_path).unwrap();
        f.set_modified(old).unwrap();
        drop(f);
        assert!(ledger_lock(&path).is_some(), "낡은 잠금은 치우고 얻는다");
        assert!(!lock_path.exists());
        // 원장이 들어갈 폴더가 없으면 잠금 없이 진행(None) — 원장 쓰기가 그 오류를 돌려준다.
        assert!(ledger_lock(&home.join("nope").join("ledger.json")).is_none());
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn claude_tui_reconcile_is_inert_in_test_builds() {
        // 테스트 빌드는 실 HOME 을 만지지 않는다 — 게이트와 무관하게 None.
        assert_eq!(reconcile_for_launch(Path::new("/nonexistent")), None);
    }
}
