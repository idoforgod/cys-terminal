//! D-04 (0.14.39) — agents.json 마커 후보와 알려진 옛 vendor 기본값의 메모리 승격.
//!
//! 2026-09-21 실측에서 codex composer 는 버전에 따라 `›` 또는 `»`, gemini 는 `>` 로
//! 확인됐다. 큐 관측기는 커서행의 행 선두(공백 제외) 후보만 composer 로 채택하며,
//! 같은 선두에 여러 후보가 맞으면 더 긴 것을 고른다. 이 모듈은 후보 해석·선택만 공유한다.
//!
//! agents.json 은 user-owned 파일이라 새 임베드 기본값만 배달하면 기존 디스크 값이 계속
//! 우선한다. 따라서 데몬과 CLI 의 읽기 시점 병합기가 정확히 알려진 옛 기본값만 새 임베드
//! 값으로 메모리 승격한다. 사용자 커스텀과 목록 값은 보존하고 W-B 에 따라 디스크는 쓰지 않는다.

use serde_json::Value;

/// agents.json `prompt_marker` 값(문자열 또는 목록)을 후보 목록으로 해석한다.
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

/// 커서행의 **행 선두(공백 제외)** 에 오는 후보 — composer 프롬프트 글리프는 세 TUI 모두 행 선두다
/// (claude `❯ ` · codex `› `/`» ` · gemini `>` 실측 col 0). 선두에 여러 후보가 맞으면 더 긴 후보.
/// 반환 = (후보, 그 후보가 시작하는 byte index). 선두에 후보가 없으면 None — 출력 행 끝의 `->` 나
/// 초안 속 글리프(`» abc » `)는 composer 가 아니다(리뷰 PROBE-A · 0.14.39 수정 라운드 1).
pub fn pick_marker_leading<'a>(cands: &'a [String], row: &str) -> Option<(&'a str, usize)> {
    cands
        .iter()
        .filter(|marker| leading_marker_index(marker, row).is_some())
        .max_by_key(|marker| marker.len())
        .and_then(|marker| {
            leading_marker_index(marker, row).map(|index| (marker.as_str(), index))
        })
}

/// ★(0.14.39 · 성찰2 blocking ①) [`pick_marker_leading`] 의 **단일 후보판**(정의 1지점).
///
/// 후보 하나가 이 행의 **선두(공백 제외)** 에 오는가 — 오면 그 후보가 시작하는 byte index.
/// 빈 후보는 어느 행에도 증거가 아니다(`readiness::marker_of` 규약과 같다).
///
/// 【왜 갈라 두나】 CLI 의 composer 행 해소(`scan_composer`·`marker_row_has_draft`)는 후보 **목록**이
/// 아니라 이미 해소된 마커 **하나**를 들고 온다. 그 자리에서 데몬과 같은 규율(`pick_marker_leading`)을
/// 쓰려면 단일 후보 판정이 필요하고, 구현을 복사하면 두 규율이 갈린다(이 저장소에서 살아남는 결함은
/// 전부 그 이음매에 있다). 그래서 `pick_marker_leading` 이 이 함수를 쓴다(거동 불변).
pub fn leading_marker_index(marker: &str, row: &str) -> Option<usize> {
    if marker.is_empty() {
        return None;
    }
    let trimmed = row.trim_start();
    trimmed
        .starts_with(marker)
        .then(|| row.len() - trimmed.len())
}

/// 후보 중 `text` 에서 나오는 것(byte rfind 위치 우선(가장 뒤) · 같은 위치면 더 긴 후보). 하나도 없으면 None.
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
/// 데몬의 마지막 폴백은 이 함수를 유지한다. 관문 이월·사이클 판정은
/// [`pick_marker_leading_on_screen`] 을 쓴다 — 출력·푸터의 비선두 글리프를 composer 로 읽지 않는다.
pub fn pick_marker_for_screen<'a>(cands: &'a [String], text: &str) -> Option<&'a str> {
    pick_marker_last(cands, text).or_else(|| cands.first().map(String::as_str))
}

/// ★(0.14.39 · 성찰2 blocking ①) 화면에서 **composer 행**을 해소한다 — `trim_start()` 후 후보로
/// 시작하는 **마지막 행**의 후보. 그런 행이 하나도 없으면 `None`.
///
/// 【[`pick_marker_for_screen`] 과 무엇이 다른가】 저쪽은 후보가 화면에 하나도 없어도 **첫 후보로
/// 폴백**한다(= "선언했으니 마커 좌석" 이라는 등급 유지 장치). 그 폴백은 글리프가 희귀할 때의
/// 편의였는데, 0.14.39 가 gemini 에 `>` 를 선언하면서 치명적이 됐다: `>` 는 셸 명령·리다이렉션·
/// diff·인용·JSON·마크다운에 일상적으로 나오는 글자라 그 좌석은 **다시는 마커 축을 벗어나지
/// 못하고**, 마커 축이 증거를 세우지 못하는 프레임에서 `gate_carry_ok` 가 영원히 거짓이 된다
/// (= 관문을 한 번 본 `reviewer-gemini` 에 역할 디렉티브가 영영 주입되지 않는다 · 치명위험 ③).
///
/// 【장치】 데몬 `governance::observe_prompt` 와 **같은 규율**을 쓴다(판정 분리 금지) — composer
/// 프롬프트 글리프는 세 TUI 모두 행 선두다. 출력 행의 `cat a > b`·푸터의 `[main] ~/dev > 62%`·
/// 상태줄의 `‹ prev » next` 는 선두가 아니므로 composer 행이 아니다.
///
/// 【실패 방향】 해소 실패(`None`)의 귀결은 소비처마다 다르고, **어댑터마다 비대칭**이다(정정 2026-09-22):
///   · gemini(0.14.38 에 `prompt_marker` 선언 없음)에게는 종전 등급 그대로다 — 회귀가 아니다.
///   · claude(`❯`)·codex(`›`·`»`)에게는 **강등**이다. 종전 [`pick_marker_for_screen`] 의 첫-후보 폴백이
///     `Some(m)` 을 유지해 `composer_layout_static_ok` 가 보류시키던 프레임이, 폴백 없는 이 해소기에서는
///     `None` 이 되어 출력 정적(`idle_quiet`) 축으로 내려간다. 그 강등은 0.14.38 거동이 아니다.
///   그래서 소비처가 등급을 되살린다: 후보 글리프가 화면에 **있는데 선두가 아니면** 마커 축에 남겨
///   보류하고(`gate_carry_ok` 의 `glyph_off_composer`), 글리프가 아예 없을 때만 quiet 축으로 내려간다.
///   그 quiet 축에도 관문·모달 부재를 AND 로 건다(`readiness::gate_or_modal_present`).
pub fn pick_marker_leading_on_screen<'a>(cands: &'a [String], screen: &str) -> Option<&'a str> {
    screen
        .lines()
        .rev()
        .find_map(|row| pick_marker_leading(cands, row))
        .map(|(marker, _)| marker)
}

/// 알려진 옛 vendor 기본값 표 (agent, key, 옛 값). 정확히 이 문자열일 때만 옛 기본값이다.
/// 문자열 `›` 는 항상 vendor 옛 기본값으로 취급한다 — 구버전 codex 에 고정하려면 목록 `["›"]` 로 선언(보존됨).
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
    fn pick_leading_accepts_marker_after_leading_whitespace() {
        let markers = marker_candidates(Some(&json!(["›", "»"])));
        assert_eq!(pick_marker_leading(&markers, "» draft"), Some(("»", 0)));
        assert_eq!(pick_marker_leading(&markers, "  › draft"), Some(("›", 2)));
        assert_eq!(
            pick_marker_leading(&markers, "\u{3000}» draft"),
            Some(("»", 3))
        );
    }

    #[test]
    fn pick_leading_rejects_nonleading_marker() {
        let markers = marker_candidates(Some(&json!(["»", ">"])));
        assert_eq!(pick_marker_leading(&markers, "out » "), None);
        assert_eq!(pick_marker_leading(&markers, "out ->"), None);
    }

    #[test]
    fn pick_leading_prefers_longest_matching_candidate() {
        let markers = marker_candidates(Some(&json!([">", ">>"])));
        assert_eq!(pick_marker_leading(&markers, ">> x"), Some((">>", 0)));
    }

    #[test]
    fn pick_leading_rejects_empty_candidates_and_rows() {
        assert_eq!(pick_marker_leading(&[], ">"), None);
        assert_eq!(pick_marker_leading(&[String::new()], ">"), None);
        let markers = marker_candidates(Some(&json!(">")));
        assert_eq!(pick_marker_leading(&markers, ""), None);
        assert_eq!(pick_marker_leading(&markers, "  "), None);
    }

    #[test]
    fn leading_marker_index_accepts_whitespace_and_returns_byte_index() {
        assert_eq!(leading_marker_index(">", "> draft"), Some(0));
        assert_eq!(leading_marker_index("›", " \t› draft"), Some(2));
        assert_eq!(leading_marker_index("»", "\u{3000}» draft"), Some(3));
        assert_eq!(leading_marker_index("❯", " \u{3000}\t❯ draft"), Some(5));
    }

    #[test]
    fn leading_marker_index_rejects_nonleading_and_empty_markers() {
        // 출력·상태줄의 글리프는 composer 가 아니며 빈 후보는 어느 행에도 증거가 아니다.
        for row in ["cat a > b", "[main] ~/dev > 62%", "  out ->", "", " \t"] {
            assert_eq!(leading_marker_index(">", row), None, "{row:?}");
        }
        for row in ["> draft", "", " \t"] {
            assert_eq!(leading_marker_index("", row), None, "{row:?}");
        }
    }

    #[test]
    fn leading_screen_resolution_prefers_longest_candidate_on_the_same_row() {
        let markers = marker_candidates(Some(&json!([">", ">>>", ">>"])));
        assert_eq!(
            pick_marker_leading_on_screen(&markers, "출력\n \u{3000}>>> draft\n"),
            Some(">>>")
        );
    }

    #[test]
    fn leading_screen_resolution_has_no_declared_candidate_fallback() {
        let markers = marker_candidates(Some(&json!(["›", "»", ">"])));
        for screen in ["", "…\n", "cat a > b\n‹ prev » next\n", " \t\n"] {
            assert_eq!(pick_marker_leading_on_screen(&markers, screen), None);
        }
        assert_eq!(pick_marker_leading_on_screen(&[], "> \n"), None);
        assert_eq!(
            pick_marker_leading_on_screen(&[String::new()], "> \n"),
            None
        );
    }

    #[test]
    fn leading_screen_resolution_prefers_the_last_leading_row() {
        let markers = marker_candidates(Some(&json!(["›", "»", ">>>", ">"])));
        // 긴 후보 우선은 같은 행 안의 규율이다. 화면에서는 가장 아래 선두 행이 먼저다.
        assert_eq!(
            pick_marker_leading_on_screen(&markers, ">>> old\n  › \n\t» \n"),
            Some("»")
        );
        assert_eq!(
            pick_marker_leading_on_screen(&markers, "» old\n> \n"),
            Some(">")
        );
    }

    #[test]
    fn leading_screen_resolution_ignores_nonleading_footer_glyphs() {
        let markers = marker_candidates(Some(&json!(["›", "»"])));
        assert_eq!(
            pick_marker_leading_on_screen(&markers, "› \n‹ prev » next\n"),
            Some("›")
        );
        let markers = marker_candidates(Some(&json!([">"])));
        assert_eq!(
            pick_marker_leading_on_screen(&markers, "> \n[main] ~/dev/x > 62% ctx\n"),
            Some(">")
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
    fn pick_last_prefers_later_position_then_longer_candidate() {
        // 첫 케이스는 더 뒤의 위치 우선, 둘째 케이스는 같은 위치에서 더 긴 후보 우선이다.
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
