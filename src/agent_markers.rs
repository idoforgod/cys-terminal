//! D-04 (0.14.39) — agents.json 마커 후보와 알려진 옛 vendor 기본값의 메모리 승격.
//!
//! 2026-09-21 실측에서 codex composer 는 버전에 따라 `›` 또는 `»`, gemini 는 `>` 로
//! 확인됐다. 큐 관측기는 커서행의 커서 앞 후보 중 가장 뒤의 것을 채택하므로 스크롤백의
//! 인용 글리프를 composer 로 오인하지 않는다. 이 모듈은 후보 해석·선택만 공유한다.
//!
//! agents.json 은 user-owned 파일이라 새 임베드 기본값만 배달하면 기존 디스크 값이 계속
//! 우선한다. 따라서 데몬과 CLI 의 읽기 시점 병합기가 정확히 알려진 옛 기본값만 새 임베드
//! 값으로 메모리 승격한다. 사용자 커스텀과 목록 값은 보존하고 W-B 에 따라 디스크는 쓰지 않는다.

use serde_json::Value;

/// agents.json 마커 값(`prompt_marker`·`ready_marker`)을 후보 목록으로 해석한다 — 문자열 또는 문자열 목록(additive).
/// 빈 문자열·비문자열 항목은 버린다(빈 문자열 = 미정의 · `readiness::marker_of` 규약). 중복 제거(순서 보존).
pub fn marker_candidates(v: Option<&Value>) -> Vec<String> {
    let values = match v {
        Some(Value::Array(items)) => items.as_slice(),
        Some(value) => std::slice::from_ref(value),
        None => return Vec::new(),
    };
    let mut candidates = Vec::new();
    for marker in values.iter().filter_map(Value::as_str) {
        if !marker.is_empty() && !candidates.iter().any(|candidate| candidate == marker) {
            candidates.push(marker.to_owned());
        }
    }
    candidates
}

/// 후보 중 `text` 에서 가장 뒤에 나오는 것(byte rfind 최대 · 동률이면 더 긴 후보). 하나도 없으면 None.
pub fn pick_marker_last<'a>(cands: &'a [String], text: &str) -> Option<&'a str> {
    cands
        .iter()
        .filter(|marker| !marker.is_empty())
        .filter_map(|marker| {
            text.rfind(marker.as_str())
                .map(|index| ((index, marker.len()), marker.as_str()))
        })
        .max_by_key(|(position, _)| *position)
        .map(|(_, marker)| marker)
}

/// 화면용 해소: 가장 뒤 후보 → 없으면 첫 후보. 선언된 마커가 안 보여도 마커 좌석 등급을
/// 유지하여 보류하고 quiet 폴백으로 내려가지 않는다.
pub fn pick_marker_for_screen<'a>(cands: &'a [String], text: &str) -> Option<&'a str> {
    pick_marker_last(cands, text).or_else(|| cands.first().map(String::as_str))
}

/// 알려진 옛 vendor 기본값 표 (agent, key, 옛 값). 정확히 이 문자열일 때만 옛 기본값이다.
pub const STALE_VENDOR_MARKER_DEFAULTS: &[(&str, &str, &str)] =
    &[("codex", "prompt_marker", "\u{203A}")];

pub fn is_stale_vendor_marker_default(agent: &str, key: &str, v: &Value) -> bool {
    STALE_VENDOR_MARKER_DEFAULTS
        .iter()
        .any(|&(known_agent, known_key, stale)| {
            agent == known_agent && key == known_key && v.as_str() == Some(stale)
        })
}

/// 어댑터 spec(한 에이전트 객체) 안에서 옛 기본값 키를 임베드 값으로 메모리 승격한다.
/// 반환 = 승격한 키 목록. 디스크 무접촉.
pub fn promote_stale_vendor_defaults(
    agent: &str,
    spec: &mut Value,
    embedded: Option<&Value>,
) -> Vec<&'static str> {
    let Some(fields) = spec.as_object_mut() else {
        return Vec::new();
    };
    let mut promoted = Vec::new();
    for &(known_agent, key, _) in STALE_VENDOR_MARKER_DEFAULTS {
        if agent != known_agent {
            continue;
        }
        let Some(current) = fields.get(key) else {
            continue;
        };
        if !is_stale_vendor_marker_default(agent, key, current) {
            continue;
        }
        let Some(replacement) = embedded.and_then(|value| value.get(key)) else {
            continue;
        };
        if current == replacement {
            continue;
        }
        fields.insert(key.to_owned(), replacement.clone());
        promoted.push(key);
    }
    promoted
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn candidates_accept_scalar_and_array() {
        assert_eq!(marker_candidates(Some(&json!("›"))), vec!["›"]);
        assert_eq!(
            marker_candidates(Some(&json!(["›", "»"]))),
            vec!["›", "»"]
        );
    }

    #[test]
    fn candidates_discard_empty_and_nonstring_values() {
        for value in [
            json!(""),
            json!([]),
            json!(null),
            json!(3),
            json!({}),
            json!(false),
        ] {
            assert!(marker_candidates(Some(&value)).is_empty(), "{value}");
        }
        assert!(marker_candidates(None).is_empty());
        assert_eq!(
            marker_candidates(Some(&json!(["›", "", 3, "»"]))),
            vec!["›", "»"]
        );
    }

    #[test]
    fn candidates_deduplicate_in_original_order() {
        assert_eq!(
            marker_candidates(Some(&json!(["›", "»", "›", "❯", "»"]))),
            vec!["›", "»", "❯"]
        );
    }

    #[test]
    fn pick_last_uses_text_position_not_candidate_order() {
        let markers = marker_candidates(Some(&json!(["›", "»"])));
        assert_eq!(pick_marker_last(&markers, "› 인용 » "), Some("»"));
        assert_eq!(pick_marker_last(&markers, "» abc ›def"), Some("›"));
        assert_eq!(pick_marker_last(&markers, "no composer"), None);
        assert_eq!(pick_marker_last(&[], "›"), None);
    }

    #[test]
    fn pick_last_breaks_equal_position_ties_by_length() {
        let markers = marker_candidates(Some(&json!([">>", ">>>"])));
        assert_eq!(pick_marker_last(&markers, ">>>"), Some(">>"));
        let markers = marker_candidates(Some(&json!([">", "> abc"])));
        assert_eq!(pick_marker_last(&markers, "> abc"), Some("> abc"));
        assert_eq!(pick_marker_last(&[String::new()], "anything"), None);
    }

    #[test]
    fn screen_resolution_falls_back_to_first_declared_candidate() {
        let markers = marker_candidates(Some(&json!(["›", "»"])));
        assert_eq!(pick_marker_for_screen(&markers, "no composer"), Some("›"));
        assert_eq!(pick_marker_for_screen(&markers, "› quoted » "), Some("»"));
        assert_eq!(pick_marker_for_screen(&[], "no composer"), None);
    }

    #[test]
    fn stale_default_requires_exact_agent_key_and_scalar() {
        assert!(is_stale_vendor_marker_default(
            "codex", "prompt_marker", &json!("›")
        ));
        for value in [
            json!("▶"),
            json!(["›"]),
            json!(["›", "»"]),
            json!(""),
            json!(null),
        ] {
            assert!(!is_stale_vendor_marker_default(
                "codex", "prompt_marker", &value
            ));
        }
        assert!(!is_stale_vendor_marker_default(
            "gemini", "prompt_marker", &json!("›")
        ));
        assert!(!is_stale_vendor_marker_default(
            "codex", "ready_marker", &json!("›")
        ));
    }

    #[test]
    fn promotion_replaces_only_stale_marker_and_is_idempotent() {
        let embedded = json!({"prompt_marker": ["›", "»"], "ready_marker": "new boot"});
        let mut spec = json!({"prompt_marker": "›", "ready_marker": "›", "custom": true});
        assert_eq!(
            promote_stale_vendor_defaults("codex", &mut spec, Some(&embedded)),
            vec!["prompt_marker"]
        );
        assert_eq!(
            spec,
            json!({"prompt_marker": ["›", "»"], "ready_marker": "›", "custom": true})
        );
        let promoted_spec = spec.clone();
        assert!(promote_stale_vendor_defaults("codex", &mut spec, Some(&embedded)).is_empty());
        assert_eq!(spec, promoted_spec);
    }

    #[test]
    fn promotion_preserves_custom_scalar_and_all_array_values() {
        let embedded = json!({"prompt_marker": ["›", "»"]});
        for marker in [
            json!("▶"),
            json!(["›"]),
            json!(["›", "»"]),
            json!([]),
            json!(null),
        ] {
            let mut spec = json!({"prompt_marker": marker});
            let original = spec.clone();
            assert!(promote_stale_vendor_defaults("codex", &mut spec, Some(&embedded)).is_empty());
            assert_eq!(spec, original);
        }
        let mut other_agent = json!({"prompt_marker": "›"});
        assert!(
            promote_stale_vendor_defaults("gemini", &mut other_agent, Some(&embedded)).is_empty()
        );
        assert_eq!(other_agent, json!({"prompt_marker": "›"}));
    }

    #[test]
    fn promotion_requires_embedded_key_and_an_existing_stale_spec_key() {
        for embedded in [None, Some(json!({})), Some(json!({"ready_marker": "boot"}))] {
            let mut spec = json!({"prompt_marker": "›"});
            assert!(promote_stale_vendor_defaults("codex", &mut spec, embedded.as_ref()).is_empty());
            assert_eq!(spec, json!({"prompt_marker": "›"}));
        }
        let embedded = json!({"prompt_marker": ["›", "»"]});
        for mut spec in [json!({}), json!(null), json!([])] {
            let original = spec.clone();
            assert!(promote_stale_vendor_defaults("codex", &mut spec, Some(&embedded)).is_empty());
            assert_eq!(spec, original);
        }
        let mut spec = json!({"prompt_marker": "›"});
        let unchanged_embed = spec.clone();
        assert!(promote_stale_vendor_defaults("codex", &mut spec, Some(&unchanged_embed)).is_empty());
        assert_eq!(spec, unchanged_embed);
    }
}
