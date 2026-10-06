//! ★(0.14.44 · §4) 손잡이(되돌리는 스위치) 읽기 — 환경변수와 정책 파일(`~/.cys/policy.json`)의 같은 이름 키.
//!
//! 규칙(설계 §4 머리): 데몬 쪽 손잡이 넷(`CYS_OFFICE_BRIDGE_ANY_OWNER` · `CYS_OFFICE_BRIDGE_MODE` · `CYS_OFFICE_BRIDGE_REPLACE_OLD` · `CYS_FEED_ORPHAN_SWEEP`)은
//! 환경변수로 읽고, **0.14.43 쪽으로 되돌리는 값만** 정책 파일의 같은 이름 키로도 읽는다. 이 파일은 좌석도 쓸 수 있는 파일이라 **켜는 값은 파일에서 받지 않는다**
//! (받으면 윈도우에서 끈 채로 낸 브리지 감독이 파일의 한 줄로 켜진다). 켜는 값이 파일에 적혀 있으면 무시하고 로그 한 줄(처음 보았을 때와 값이 바뀌었을 때만).
//! 판정은 전부 순수 함수(입력 = 환경 문자열 · 정책 JSON)이고, 읽는 얇은 껍질이 그것을 부른다.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use serde_json::Value;

/// 정책 파일을 읽는다 — 없거나 읽지 못하거나 깨졌으면 `None`(손잡이는 기본값으로 둔다 — 되돌리는 값만 읽으므로 모를 때는 기본이 안전한 쪽이다).
/// 경로는 승인 손잡이(`approval::cwd_neutral_enabled`)와 같다(시험 이음매 포함).
pub fn read_policy() -> Option<Value> {
    static WARNED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    let path = crate::approval::policy_path();
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            warn_unreadable(&WARNED, &format!("{e}"));
            return None;
        }
    };
    // UTF-8 BOM 을 벗긴다(앱 쪽 판독과 같다 — 윈도우 편집기가 붙인 BOM 때문에 되돌리는 손잡이가 말없이 안 듣지 않게).
    match serde_json::from_str::<Value>(cys::strip_utf8_bom(&text)) {
        Ok(v) => {
            WARNED.store(false, std::sync::atomic::Ordering::Relaxed);
            Some(v)
        }
        Err(e) => {
            warn_unreadable(&WARNED, &format!("JSON 해석 실패: {e}"));
            None
        }
    }
}

/// 정책 파일이 **있는데** 읽지 못했을 때 로그 한 줄(바뀔 때만 — 읽는 데 성공하면 다시 무장한다). 이 파일의 되돌리는 손잡이는 이때 적용되지 않는다.
fn warn_unreadable(flag: &std::sync::atomic::AtomicBool, why: &str) {
    if !flag.swap(true, std::sync::atomic::Ordering::Relaxed) {
        eprintln!("[cysd] 정책 파일(~/.cys/policy.json)을 읽지 못해 되돌리는 손잡이(CYS_OFFICE_BRIDGE_MODE=legacy · CYS_OFFICE_BRIDGE_REPLACE_OLD=0 · CYS_FEED_ORPHAN_SWEEP=0 · CYS_OFFICE_BRIDGE_ANY_OWNER=1)가 적용되지 않는다 — {why}");
    }
}

/// 환경변수의 값(앞뒤 공백 제거 · 비었으면 없음).
pub fn env_value(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// 정책 값이 "0" 을 뜻하는가(숫자 0 · 불리언 false · 문자열 "0").
pub fn policy_is_zero(v: &Value) -> bool {
    match v {
        Value::Number(n) => n.as_i64() == Some(0) || n.as_f64() == Some(0.0),
        Value::Bool(b) => !*b,
        Value::String(s) => s.trim() == "0",
        _ => false,
    }
}

/// 정책 값이 "1" 을 뜻하는가(숫자 1 · 불리언 true · 문자열 "1").
pub fn policy_is_one(v: &Value) -> bool {
    match v {
        Value::Number(n) => n.as_i64() == Some(1),
        Value::Bool(b) => *b,
        Value::String(s) => s.trim() == "1",
        _ => false,
    }
}

/// `CYS_FEED_ORPHAN_SWEEP` — 닫힌 좌석의 화면 감지 승인 쓸기(C1). 환경 `0` 또는 정책 파일 `0` 이면 꺼짐(종전 동작). 그 밖은 켬.
pub fn feed_orphan_sweep_enabled_from(env: Option<&str>, policy: Option<&Value>) -> bool {
    if env.map(|e| e.trim() == "0").unwrap_or(false) {
        return false;
    }
    if let Some(v) = policy.and_then(|p| p.get("CYS_FEED_ORPHAN_SWEEP")) {
        if policy_is_zero(v) {
            return false;
        }
    }
    true
}

pub fn feed_orphan_sweep_enabled() -> bool {
    feed_orphan_sweep_enabled_from(env_value("CYS_FEED_ORPHAN_SWEEP").as_deref(), read_policy().as_ref())
}

// ── 오피스 브리지 손잡이(B1·B2·B3·B7) ─────────────────────────────────────────────

/// 브리지 감독 방식. `Legacy` = 0.14.43 의 감독 루프 그대로(표준입력 `null` · 수명줄·탐침·재기동 상한·교체 없음).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeMode {
    Managed,
    Legacy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModeDecision {
    pub mode: BridgeMode,
    /// 정책 파일에 **켜는 값**(`managed`)이 적혀 있어 무시했다 — 켜는 값은 환경변수로만 받는다(재확인 R3).
    pub ignored_policy_managed: bool,
}

/// `CYS_OFFICE_BRIDGE_MODE` — 기본은 맥·리눅스 `Managed` · **윈도우 `Legacy`**(CI 윈도우 레인과 실기 확인을 통과하기 전에는 종전대로). 환경변수 `managed`|`legacy` 를 받고,
/// 정책 파일은 **`legacy` 만** 받는다(되돌리는 값). 파일의 `managed` 는 무시한다(좌석도 쓸 수 있는 파일이므로 켜는 값을 받으면 윈도우에서 끈 채로 낸 감독이 한 줄로 켜진다).
/// 환경이 `managed` 여도 파일이 `legacy` 면 `Legacy`(되돌리는 쪽이 이긴다).
pub fn bridge_mode_from(env: Option<&str>, policy: Option<&Value>, windows: bool) -> ModeDecision {
    let mut mode = if windows { BridgeMode::Legacy } else { BridgeMode::Managed };
    match env.map(|e| e.trim().to_ascii_lowercase()).as_deref() {
        Some("managed") => mode = BridgeMode::Managed,
        Some("legacy") => mode = BridgeMode::Legacy,
        _ => {}
    }
    let mut ignored = false;
    if let Some(v) = policy.and_then(|p| p.get("CYS_OFFICE_BRIDGE_MODE")).and_then(|v| v.as_str()) {
        match v.trim().to_ascii_lowercase().as_str() {
            "legacy" => mode = BridgeMode::Legacy,
            "managed" => ignored = true,
            _ => {}
        }
    }
    ModeDecision { mode, ignored_policy_managed: ignored }
}

/// 지금의 감독 방식을 읽는다(스폰 때마다 — 실행 중에 파일로 `legacy` 로 바뀌면 **다음 스폰부터** 적용된다). 파일의 켜는 값이 처음 보였거나 값이 바뀌었을 때만 로그 한 줄.
pub fn bridge_mode() -> BridgeMode {
    static SEEN: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
    let d = bridge_mode_from(
        env_value("CYS_OFFICE_BRIDGE_MODE").as_deref(),
        read_policy().as_ref(),
        cfg!(windows),
    );
    let now = if d.ignored_policy_managed { 1u8 } else { 2u8 };
    let prev = SEEN.swap(now, std::sync::atomic::Ordering::Relaxed);
    if d.ignored_policy_managed && prev != 1 {
        eprintln!(
            "[cysd] office-bridge: 정책 파일의 CYS_OFFICE_BRIDGE_MODE=managed 는 무시한다(켜는 값은 환경변수로만 받는다 — 되돌리는 값 legacy 만 파일에서 받는다)"
        );
    }
    d.mode
}

/// `CYS_OFFICE_BRIDGE_ANY_OWNER=1`(환경변수 또는 정책 파일의 `1`) — 종전처럼 어느 데몬이든 브리지를 띄운다(지원·개발자용).
pub fn bridge_any_owner_from(env: Option<&str>, policy: Option<&Value>) -> bool {
    if env.map(|e| e.trim() == "1").unwrap_or(false) {
        return true;
    }
    policy
        .and_then(|p| p.get("CYS_OFFICE_BRIDGE_ANY_OWNER"))
        .map(policy_is_one)
        .unwrap_or(false)
}

pub fn bridge_any_owner() -> bool {
    bridge_any_owner_from(env_value("CYS_OFFICE_BRIDGE_ANY_OWNER").as_deref(), read_policy().as_ref())
}

/// `CYS_OFFICE_BRIDGE_REPLACE_OLD=0`(환경변수 또는 정책 파일의 `0`) — 옛 브리지 교체(B7)만 끈다. 그 밖은 켬(맥). 윈도우는 호출처가 따로 막는다.
pub fn bridge_replace_old_enabled_from(env: Option<&str>, policy: Option<&Value>) -> bool {
    if env.map(|e| e.trim() == "0").unwrap_or(false) {
        return false;
    }
    if let Some(v) = policy.and_then(|p| p.get("CYS_OFFICE_BRIDGE_REPLACE_OLD")) {
        if policy_is_zero(v) {
            return false;
        }
    }
    true
}

pub fn bridge_replace_old_enabled() -> bool {
    bridge_replace_old_enabled_from(env_value("CYS_OFFICE_BRIDGE_REPLACE_OLD").as_deref(), read_policy().as_ref())
}
