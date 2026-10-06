//! ★(0.14.44 · §4) 손잡이(되돌리는 스위치) 읽기 — 환경변수와 정책 파일(`~/.cys/policy.json`)의 같은 이름 키.
//!
//! 규칙(설계 §4 머리): 데몬 쪽 손잡이 넷(`CYS_OFFICE_BRIDGE_ANY_OWNER` · `CYS_OFFICE_BRIDGE_MODE` · `CYS_OFFICE_BRIDGE_REPLACE_OLD` · `CYS_FEED_ORPHAN_SWEEP`)은
//! 환경변수로 읽고, **0.14.43 쪽으로 되돌리는 값만** 정책 파일의 같은 이름 키로도 읽는다. 이 파일은 좌석도 쓸 수 있는 파일이라 **켜는 값은 파일에서 받지 않는다**
//! (받으면 윈도우에서 끈 채로 낸 브리지 감독이 파일의 한 줄로 켜진다). 켜는 값이 파일에 적혀 있으면 무시하고 로그 한 줄(처음 보았을 때와 값이 바뀌었을 때만).
//! 판정은 전부 순수 함수(입력 = 환경 문자열 · 정책 JSON)이고, 읽는 얇은 껍질이 그것을 부른다.

use serde_json::Value;

/// 정책 파일을 읽는다 — 없거나 읽지 못하거나 깨졌으면 `None`(손잡이는 기본값으로 둔다 — 되돌리는 값만 읽으므로 모를 때는 기본이 안전한 쪽이다).
/// 경로는 승인 손잡이(`approval::cwd_neutral_enabled`)와 같다(시험 이음매 포함).
pub fn read_policy() -> Option<Value> {
    let text = std::fs::read_to_string(crate::approval::policy_path()).ok()?;
    serde_json::from_str::<Value>(&text).ok()
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
