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
//! - 개인 프로필(`~/.claude` · `~/.claude-*`)은 표적이 되어도 쓰지 않는다(`PersonalProfile` — cys 격리 계약).
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
//!   킬스위치는 **이후 기록만** 멈춘다 — 이미 들어간 `"tui": "default"` 는 사용자의 `/tui default` 와 바이트가 같아
//!   구별할 수 없으므로 지우지 않는다. 되돌리려면 pane 에서 `/tui fullscreen`(Claude Code 가 값을 바꾼다).

use serde_json::Value;
use std::path::{Path, PathBuf};

use crate::agy_statusline as surgery;

/// Claude Code 사용자 설정의 렌더러 키.
pub const TUI_KEY: &str = "tui";
/// classic(main-screen) 렌더러 값 — `/tui default` 가 저장하는 값과 같다.
pub const TUI_CLASSIC: &str = "default";
/// 킬스위치 env(값 `"1"` 만 참).
pub const KILL_ENV: &str = "CYS_WIN_TUI_CLASSIC_OFF";
/// 킬스위치 파일(홈 기준 상대 — 형제: `.cys/win-wheel-guard-off` · `.cys/win-no-alt-screen`).
pub const KILL_FILE: &str = ".cys/win-tui-classic-off";
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

/// 개인 Claude 프로필(`<home>/.claude` · `<home>/.claude-*`)인가(순수) — cys 는 그 폴더의 설정을 바꾸지 않는다.
pub fn is_personal_profile(dir: &Path, home: &Path) -> bool {
    if dir.parent() != Some(home) {
        return false;
    }
    dir.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n == ".claude" || n.starts_with(".claude-"))
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
    /// 새로 넣었다. `created` = settings.json 을 새로 만들었다.
    Written { created: bool },
    /// 쓰지 않았다(사유).
    Refused(String),
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
            let lead = if pretty { format!(",{nl}{unit}") } else { ",".to_string() };
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

fn backup(settings: &Path, raw: &[u8]) -> Result<(), String> {
    let dest = PathBuf::from(format!("{}{BACKUP_SUFFIX}", settings.display()));
    crate::pack::write_atomic_mode(&dest, raw, surgery::file_mode(settings))
        .map_err(|e| format!("백업 실패({}): {e}", dest.display()))
}

/// `<config_dir>/settings.json` 에 `tui` 키가 **없을 때만** `"default"` 를 넣는다(게이트 무관 — 게이트는 호출부
/// [`reconcile_for_launch`] 가 건다). `home` = 개인 프로필 판정 기준(주입형 — 시험은 가짜 홈을 준다).
pub fn ensure_classic(config_dir: &Path, home: &Path) -> Outcome {
    if is_personal_profile(config_dir, home) {
        return Outcome::PersonalProfile;
    }
    if !config_dir.is_dir() {
        return Outcome::NoConfigDir;
    }
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
    if std::fs::read(&settings).ok().as_deref() != orig_raw.as_deref() {
        return Outcome::Refused("그 사이 다른 프로그램(claude 등)이 파일을 바꿨다 — 이번에는 쓰지 않는다".into());
    }
    if let Err(e) = crate::pack::write_atomic_mode(&settings, &new_bytes, mode) {
        return Outcome::Refused(format!("원자 쓰기 실패: {e}"));
    }
    // 되읽기 확인 — 반환값은 반영의 증거가 아니다.
    match surgery::load(&settings) {
        Ok(Some((_, _, _, Some(v)))) if v.get(TUI_KEY).and_then(Value::as_str) == Some(TUI_CLASSIC) => {
            Outcome::Written { created }
        }
        Ok(_) => Outcome::Refused("되읽기: tui 가 들어가 있지 않다".into()),
        Err(e) => Outcome::Refused(format!("되읽기 실패: {e}")),
    }
}

/// claude 기동 직전 호출용 — 게이트(Windows ∧ ¬킬스위치)를 지나면 [`ensure_classic`]. 게이트 밖이면 `None`.
/// ★호출부 계약: 결과가 무엇이든 **기동을 계속한다**(로그 1줄만). 이 함수는 패닉하지 않는다.
pub fn reconcile_for_launch(config_dir: &Path) -> Option<Outcome> {
    if cfg!(test) || !gate_now() {
        return None; // 테스트 빌드는 실 HOME·실 설정 폴더를 절대 만지지 않는다
    }
    Some(ensure_classic(config_dir, &crate::home_dir()))
}

/// 사람용 한 줄(로그). `None` = 알릴 것 없음(이미 설정됨 · 폴더 없음 등 정상 무동작).
pub fn describe(o: &Outcome, config_dir: &Path) -> Option<String> {
    let p = config_dir.join("settings.json");
    let p = p.display();
    Some(match o {
        Outcome::NoConfigDir | Outcome::AlreadySet(_) => return None,
        Outcome::PersonalProfile => format!(
            "{p} 는 개인 프로필이라 렌더러 설정을 넣지 않았습니다(전체화면으로 뜨면 그 pane 에서 /tui default)"
        ),
        Outcome::Written { created } => format!(
            "{p} 에 \"tui\": \"default\"(classic 렌더러 — 휠 스크롤)를 넣었습니다{} · 이미 떠 있는 전체화면 좌석은 \
             다음 기동부터 적용 · 끄기: {KILL_ENV}=1 또는 ~/{KILL_FILE}",
            if *created { "(파일 새로 만듦)" } else { "" }
        ),
        Outcome::Refused(why) => format!("{p} 를 건드리지 않았습니다 — {why}"),
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
        assert!(!gate_for_os("macos", false), "macOS 는 D5 env 가 기본 — 사용자 설정을 바꾸지 않는다");
        assert!(!gate_for_os("linux", false));
        // 판독 규약 — 엄격 비교(형제 게이트와 동형)
        assert!(!kill_switch_from(None, false), "기본 = 킬스위치 꺼짐(=기록 켜짐)");
        assert!(kill_switch_from(Some("1"), false));
        assert!(kill_switch_from(None, true));
        for loose in ["true", "yes", "", "0", " 1"] {
            assert!(!kill_switch_from(Some(loose), false), "느슨한 truthy {loose:?} 는 참이 아니다");
        }
    }

    #[test]
    fn claude_tui_key_value_pins_match_claude_code_schema() {
        // 2.1.291 스키마 `tui: enum["default","fullscreen"]` · "default" = classic main-screen 렌더러(모듈 doc 증거).
        assert_eq!(TUI_KEY, "tui");
        assert_eq!(TUI_CLASSIC, "default");
        assert_ne!(BACKUP_SUFFIX, ".bak-cys", "훅 병합 백업과 이름이 같으면 정상 백업을 클로버한다");
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
        assert_eq!(render_with_classic(crlf).unwrap(), "{\r\n    \"a\": 1,\r\n    \"tui\": \"default\"\r\n}");
        // 한 줄 압축형은 압축형으로
        assert_eq!(render_with_classic("{\"a\":1}").unwrap(), "{\"a\":1,\"tui\":\"default\"}");
        // 빈 객체
        assert_eq!(render_with_classic("{}\n").unwrap(), "{\n  \"tui\": \"default\"\n}\n");
        // 이미 있으면 거부(값 무관)
        assert!(render_with_classic("{\"tui\":\"fullscreen\"}").is_err());
        assert!(render_with_classic("{\"tui\":null}").is_err());
    }

    #[test]
    fn claude_tui_verify_rejects_collateral_change() {
        let orig = json!({"a": 1});
        assert!(verify_render(&orig, "{\"a\":2,\"tui\":\"default\"}").is_err());
        assert!(verify_render(&orig, "{\"a\":1,\"b\":0,\"tui\":\"default\"}").is_err());
        assert!(verify_render(&orig, "{\"a\":1,\"tui\":\"fullscreen\"}").is_err());
        assert!(verify_render(&orig, "{\"a\":1}").is_err(), "결측은 값이 아니다 — tui 부재는 실패");
        verify_render(&orig, "{\"a\":1,\"tui\":\"default\"}").unwrap();
    }

    #[test]
    fn claude_tui_ensure_writes_once_and_never_overwrites() {
        let home = sandbox("home");
        let cfg = home.join(".cys").join("claude");
        // ① 폴더 없음 → 만들지 않는다
        assert_eq!(ensure_classic(&cfg, &home), Outcome::NoConfigDir);
        assert!(!cfg.exists());
        std::fs::create_dir_all(&cfg).unwrap();
        let s = cfg.join("settings.json");
        // ② 파일 없음 → 새로 만든다
        assert_eq!(ensure_classic(&cfg, &home), Outcome::Written { created: true });
        let v: Value = serde_json::from_slice(&std::fs::read(&s).unwrap()).unwrap();
        assert_eq!(v, json!({"tui": "default"}));
        // ③ 멱등 — 이미 있으면 무동작(바이트 불변)
        let before = std::fs::read(&s).unwrap();
        assert_eq!(ensure_classic(&cfg, &home), Outcome::AlreadySet("\"default\"".into()));
        assert_eq!(std::fs::read(&s).unwrap(), before);
        // ④ 사용자 값 불가침 — fullscreen 도, null 도
        for user in ["{\"tui\":\"fullscreen\",\"x\":1}", "{\"tui\":null}"] {
            std::fs::write(&s, user).unwrap();
            assert!(matches!(ensure_classic(&cfg, &home), Outcome::AlreadySet(_)));
            assert_eq!(std::fs::read_to_string(&s).unwrap(), user);
        }
        // ⑤ 기존 파일(BOM · 훅) → 한 칸 추가 + 백업(원문) + 훅 백업 이름 무접촉
        let orig = "\u{feff}{\n  \"hooks\": {\"SessionStart\": [1]}\n}\n";
        std::fs::write(&s, orig).unwrap();
        std::fs::write(cfg.join("settings.json.bak-cys"), "HOOK-BACKUP").unwrap();
        assert_eq!(ensure_classic(&cfg, &home), Outcome::Written { created: false });
        let now = std::fs::read_to_string(&s).unwrap();
        assert!(now.starts_with('\u{feff}'), "BOM 보존");
        assert_eq!(now, "\u{feff}{\n  \"hooks\": {\"SessionStart\": [1]},\n  \"tui\": \"default\"\n}\n");
        assert_eq!(std::fs::read_to_string(cfg.join("settings.json.bak-cys-tui")).unwrap(), orig);
        assert_eq!(std::fs::read_to_string(cfg.join("settings.json.bak-cys")).unwrap(), "HOOK-BACKUP");
        // ⑥ 빈 파일 → 새 본문
        std::fs::write(&s, "").unwrap();
        assert_eq!(ensure_classic(&cfg, &home), Outcome::Written { created: false });
        // ⑦ 깨진 JSON · 루트가 배열 → 쓰지 않는다(바이트 불변)
        for bad in ["{\"a\": ", "[1,2]", "not json"] {
            std::fs::write(&s, bad).unwrap();
            assert!(matches!(ensure_classic(&cfg, &home), Outcome::Refused(_)), "{bad:?}");
            assert_eq!(std::fs::read_to_string(&s).unwrap(), bad);
        }
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn claude_tui_personal_profile_is_never_touched() {
        let home = sandbox("pp");
        for name in [".claude", ".claude-3"] {
            let d = home.join(name);
            std::fs::create_dir_all(&d).unwrap();
            assert!(is_personal_profile(&d, &home));
            assert_eq!(ensure_classic(&d, &home), Outcome::PersonalProfile);
            assert!(!d.join("settings.json").exists());
        }
        // cys 격리 폴더 · 부서 폴더는 개인 프로필이 아니다
        assert!(!is_personal_profile(&home.join(".cys").join("claude"), &home));
        assert!(!is_personal_profile(&home.join(".cys").join("claude-dept-a"), &home));
        assert!(!is_personal_profile(&home.join(".claudex"), &home));
        let _ = std::fs::remove_dir_all(&home);
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
        assert!(matches!(ensure_classic(&cfg, &home), Outcome::Refused(_)));
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "{}");
        std::fs::remove_file(cfg.join("settings.json")).unwrap();
        let s = cfg.join("settings.json");
        std::fs::write(&s, "{\"a\":1}").unwrap();
        std::fs::set_permissions(&s, std::fs::Permissions::from_mode(0o444)).unwrap();
        assert!(matches!(ensure_classic(&cfg, &home), Outcome::Refused(_)));
        assert_eq!(std::fs::read_to_string(&s).unwrap(), "{\"a\":1}");
        std::fs::set_permissions(&s, std::fs::Permissions::from_mode(0o644)).unwrap();
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn claude_tui_reconcile_is_inert_in_test_builds() {
        // 테스트 빌드는 실 HOME 을 만지지 않는다 — 게이트와 무관하게 None.
        assert_eq!(reconcile_for_launch(Path::new("/nonexistent")), None);
    }
}
