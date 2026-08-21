//! W3.1 서버측 위험도 파생 — 승인 feed 요청을 cysd가 **발행자 서술(title·body)에서** 분류한다.
//!
//! 발행자의 `tier`·`kind` 자기신고는 **위험 판정에 쓰지 않는다**(위조 가능). title·body의
//! 실제 내용 패턴만으로 판정하며, 미지 패턴은 HighRisk(fail-closed)로 떨어뜨린다.
//!
//! ⚠3R-A 한계: feed 승인은 실행을 게이트하지 않으므로(발행자가 allow 수신 후 독립 실행)
//! derive_risk는 발행자 '서술'만 본다 — 위장 서술 스푸핑을 **차단이 아니라 축소**한다.
//! v1은 자동결재 스코프를 저위험·가역 클래스로 한정해 blast radius를 원천 축소한다.

/// 승인 요청 위험 클래스. cysd가 title·body에서 파생한다(자기신고 무관).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RiskClass {
    /// 저위험·가역(검증·학습추천 등) — flag ON일 때만 CEO 자동결재 대상.
    AutoEligible,
    /// 파괴·외부발행·크레딧·미지 패턴 — v1 사람 결재 유지(fail-closed).
    HighRisk,
    /// 기계가 대신 이행 불가(사람 손·TCC 권한 등) — 즉시 오너 escalation.
    HumanOnly,
}

impl RiskClass {
    /// 영속·이벤트·감사에 싣는 안정 문자열 태그.
    pub fn as_str(self) -> &'static str {
        match self {
            RiskClass::AutoEligible => "auto",
            RiskClass::HighRisk => "high",
            RiskClass::HumanOnly => "human",
        }
    }
}

/// 기계가 대신 이행할 수 없는 사람 단계 마커(정규화형·소문자). 최우선 판정 —
/// 이런 요청은 CEO가 "이행 불가"로 오너에게 즉시 escalation한다(결재의 한 형태).
const HUMAN_ONLY_MARKERS: &[&str] = &[
    "사람단계",   // "★사람 단계 필수" 등
    "사람이직접",
    "사람손",
    "수동재부여",
    "재부여",     // TCC/토큰 재부여 = GUI·Keychain 사람 동작
    "tcc",        // macOS 권한(카메라·화면기록 등) 재승인
    "keychain",
    "nlmlogin",   // NotebookLM 로그인(사람 단계·preflight C20)
    "브라우저에서",
    "직접클릭",
    "직접입력",
];

/// 파괴/외부발행/크레딧 denylist(한글 동사 + 정규화 영문). 하나라도 걸리면 HighRisk —
/// v1 사람 결재 유지. 한글은 어절 경계가 없어 부분일치가 안전하고(fail-closed 방향),
/// 영문 rm·git push·gh release는 아래 has_denylist에서 별도 정밀 매칭한다.
const DENY_KO: &[&str] = &[
    "삭제", "제거", "정리", "발행", "배포", "커밋", "푸시", "릴리스", "크레딧",
];

/// 저위험·가역 allowlist(정규화형). denylist·human-only에 걸리지 않은 요청 중
/// 이 마커를 포함하면 AutoEligible. 실물 표본 기반: RSI 학습 추천.
///
/// cycle 계열("cycle-verify"·"cycleverify"·"저장검증"·"순환전저장")은 여기 넣지 않는다:
/// 저장 검증은 **비가역 컨텍스트 clear의 사전 게이트**라 CEO 자동결재를 허용하면 방어선
/// 자체가 무력화된다 — 미지=HighRisk(fail-closed)로 떨어뜨려 사람 결재를 강제한다.
const AUTO_MARKERS: &[&str] = &[
    "rsi학습",
    "학습추천",
    "학습제안",
];

/// 매칭용 정규화: 공백·각종 중점(·‧・･) 제거 + 소문자화. 한글은 보존한다.
/// "git push"·"git·push"·"gitpush"를 한 형태로 접어 비연속 회피를 막는다(fail-closed).
fn normalize(s: &str) -> String {
    s.chars()
        .filter(|c| {
            !c.is_whitespace()
                && *c != '\u{00B7}' // · MIDDLE DOT
                && *c != '\u{2027}' // ‧ HYPHENATION POINT
                && *c != '\u{30FB}' // ・ KATAKANA MIDDLE DOT
                && *c != '\u{FF65}' // ･ HALFWIDTH KATAKANA MIDDLE DOT
        })
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// 영문·정규화 denylist 판정. `norm`=정규화형(공백·중점 제거), `lower`=소문자 원문(어절 경계용).
fn has_denylist(norm: &str, lower: &str) -> bool {
    // 한글 파괴/발행 동사 + 크레딧: 부분일치(어절 경계 없음).
    if DENY_KO.iter().any(|m| norm.contains(m)) {
        return true;
    }
    // 외부발행 정규화 매칭(비연속 회피 차단): gitpush·ghrelease·credit·크레딧.
    // + 영문 파괴 동사(F3): delete·wipe·drop·truncate·destroy·overwrite. 부분일치 오탐
    // (dropdown 등)은 HighRisk=fail-closed라 안전측 — 자동화 과소보다 유출 차단이 우선.
    for m in [
        "gitpush", "ghrelease", "credit", "delete", "wipe", "drop", "truncate", "destroy",
        "overwrite",
    ] {
        if norm.contains(m) {
            return true;
        }
    }
    // rm 은 어절 단독일 때만(정규화 부분일치는 confirm·format·warm 오탐 → 원문 토큰 경계로).
    // 오탐은 HighRisk(안전측)이라 무해하나, 자동화를 과도 무력화하지 않도록 토큰 경계로 좁힌다.
    lower
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|tok| tok == "rm")
}

/// 승인 요청 위험 파생 — kind는 입력에서 배제(자기신고 위조 방지). title·body만으로 판정.
/// 판정 순서: HumanOnly > HighRisk(denylist) > AutoEligible(allowlist) > HighRisk(미지 fail-closed).
pub fn derive_risk(title: &str, body: &str) -> RiskClass {
    let combined = format!("{title}\n{body}");
    let norm = normalize(&combined);
    let lower = combined.to_lowercase();

    if HUMAN_ONLY_MARKERS.iter().any(|m| norm.contains(m)) {
        return RiskClass::HumanOnly;
    }
    if has_denylist(&norm, &lower) {
        return RiskClass::HighRisk;
    }
    if AUTO_MARKERS.iter().any(|m| norm.contains(m)) {
        return RiskClass::AutoEligible;
    }
    // 미지 패턴 = fail-closed(사람 결재).
    RiskClass::HighRisk
}

/// W3.2 멱등 의미 키: kind+title+publisher_surface+body의 SHA-256(hex). request_id 기준이
/// 아니라 **의미** 기준이라, 재발행이 매번 새 request_id를 받아도 같은 요청을 한 키로 접는다.
/// 필드 사이에 개행을 끼워 경계 위조(값에 구분자 삽입)를 막는다.
pub fn semantic_key(kind: &str, title: &str, publisher_surface: Option<u64>, body: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(kind.as_bytes());
    h.update(b"\n");
    h.update(title.as_bytes());
    h.update(b"\n");
    h.update(
        publisher_surface
            .map(|s| s.to_string())
            .unwrap_or_default()
            .as_bytes(),
    );
    h.update(b"\n");
    h.update(body.as_bytes());
    let digest: [u8; 32] = h.finalize().into();
    let mut out = String::with_capacity(64);
    for b in digest {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── 실물 표본 기반 필수 케이스(설계 §W3.1) ──────────────────────────────────
    #[test]
    fn real_samples_classify_correctly() {
        assert_eq!(
            derive_risk("★사람 단계 필수: TCC 재부여", ""),
            RiskClass::HumanOnly
        );
        assert_eq!(
            derive_risk("디스크 위기 tmp 9건 삭제", ""),
            RiskClass::HighRisk
        );
        assert_eq!(derive_risk("백업본 정리", ""), RiskClass::HighRisk);
        assert_eq!(derive_risk("[RSI 학습 추천]", ""), RiskClass::AutoEligible);
        // cycle-verify는 비가역 clear의 사전 게이트 — 자동결재 금지(미지=HighRisk fail-closed).
        assert_eq!(
            derive_risk("[CYCLE-VERIFY] 저장 검증", "SESSION_STATE/TODO 확인"),
            RiskClass::HighRisk
        );
    }

    // ── 부정 8종 중 위험파생 관련(§W3.9) ────────────────────────────────────────

    /// tier 스푸핑: tier 자기신고는 derive_risk 입력이 아니므로 denylist 서술이면 여전히 HighRisk.
    /// (여기서 검증: denylist 한글 동사가 auto allowlist 마커와 공존해도 denylist가 이긴다.)
    #[test]
    fn denylist_beats_allowlist_no_auto_leak() {
        // "학습 추천"(auto 마커) + "삭제"(denylist)를 함께 넣어도 자동으로 새지 않는다.
        assert_eq!(
            derive_risk("[RSI 학습 추천] 로그 삭제", ""),
            RiskClass::HighRisk
        );
    }

    /// kind 위조: derive_risk 시그니처가 kind를 받지 않음을 컴파일·의미로 보장(이 호출은 title·body만).
    #[test]
    fn kind_excluded_from_risk_input() {
        // 동일 title·body면 kind가 무엇이든 결과 동일(kind는 호출 인자 아님).
        let a = derive_risk("백업본 정리", "");
        let b = derive_risk("백업본 정리", "");
        assert_eq!(a, b);
        assert_eq!(a, RiskClass::HighRisk);
    }

    #[test]
    fn git_push_noncontiguous_caught() {
        assert_eq!(derive_risk("원격 git push 반영", ""), RiskClass::HighRisk);
        assert_eq!(derive_risk("원격 git·push 반영", ""), RiskClass::HighRisk);
        assert_eq!(derive_risk("원격 gitpush 반영", ""), RiskClass::HighRisk);
    }

    #[test]
    fn gh_release_and_credit_caught() {
        assert_eq!(derive_risk("gh release v1 발행", ""), RiskClass::HighRisk);
        assert_eq!(derive_risk("credit 반영", ""), RiskClass::HighRisk);
        assert_eq!(derive_risk("크레딧 반영", ""), RiskClass::HighRisk);
    }

    /// F3: 영문 파괴 동사 6종 각각 HighRisk(fail-closed).
    #[test]
    fn english_destructive_verbs_caught() {
        for phrase in [
            "delete stale records",
            "wipe the cache",
            "drop table users",
            "truncate the log",
            "destroy the volume",
            "overwrite config file",
        ] {
            assert_eq!(derive_risk(phrase, ""), RiskClass::HighRisk, "미포착: {phrase}");
        }
    }

    #[test]
    fn rm_token_boundary() {
        // 어절 단독 rm = denylist.
        assert_eq!(derive_risk("rm -rf tmp", ""), RiskClass::HighRisk);
        // confirm·format·warm 은 rm 부분일치지만 어절 단독이 아니라 auto 마커 유무로 판정된다.
        // (auto 마커 없으면 미지=HighRisk이므로 여기선 auto 마커를 붙여 rm 오탐 부재를 증명한다.)
        assert_eq!(
            derive_risk("[RSI 학습 추천] confirm", ""),
            RiskClass::AutoEligible
        );
    }

    /// 회귀 핀: cycle 계열 4마커는 어떤 표기 변형으로도 AutoEligible로 되돌아오면 안 된다.
    /// 저장 검증은 비가역 clear의 사전 게이트라 자동결재 금지 — allowlist 재등재가 곧 결함이다.
    #[test]
    fn cycle_markers_never_auto() {
        for (title, body) in [
            ("[CYCLE-VERIFY] 저장 검증", "SESSION_STATE/TODO 확인"),
            ("cycle-verify 요청", ""),
            ("cycleverify", ""),
            ("저장 검증 요청", ""),   // normalize → "저장검증"
            ("순환 전 저장 요청", ""), // normalize → "순환전저장"
            // 실물 주입문(cys.rs cycle-agent) 표본 — 실전 서술 그대로도 auto로 새지 않는다.
            ("[CYCLE-VERIFY] role 'master'(surface:3)의 컨텍스트 순환 전 저장 검증 요청", ""),
        ] {
            assert_eq!(
                derive_risk(title, body),
                RiskClass::HighRisk,
                "cycle 계열이 자동결재로 되돌아옴: {title}"
            );
        }
    }

    #[test]
    fn unknown_is_fail_closed() {
        assert_eq!(
            derive_risk("무언가 애매한 요청", "특별한 마커 없음"),
            RiskClass::HighRisk
        );
    }

    #[test]
    fn as_str_stable() {
        assert_eq!(RiskClass::AutoEligible.as_str(), "auto");
        assert_eq!(RiskClass::HighRisk.as_str(), "high");
        assert_eq!(RiskClass::HumanOnly.as_str(), "human");
    }
}
