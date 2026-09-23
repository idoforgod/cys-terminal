//! ★U16(0.14.41) 말로 팀 만들기 1차 — '팀 만들기 제안'(feed kind `team-create-request`)의
//! 스키마·검증·코덱·잠금 정책 **단일 정의처**.
//!
//! 흐름(설계 정본 `_reports/수정설계-0.14.41-오너18항목-20260923.md` §3 U16):
//!   본부 대표(master/CEO)가 오너와 대화로 팀 이름·하는 일을 정한다
//!   → `cys team-propose` 1회 → cysd `feed.push`(kind=[`KIND`] · 아래 잠금)
//!   → GUI 카드 [확인 창 열기] → 확인 창(원문 그대로) → 오너 [만들기]
//!   → Tauri `allocate_dept_daemon(team_spec)` → `cys-dept allocate --team-spec-b64 <b64>`
//!   → **생성 성공 뒤에만** GUI `feed_reply allow`(operator token).
//!
//! 데몬 잠금(사고 방지 층):
//!   ① 제안자 = 본부(base) 레인의 master 좌석(데몬이 커널 peer pid 로 각인한 발행 좌석의 역할) — [`publisher_ok`]
//!   ② 같은 kind 대기 [`PENDING_MAX`]건 · [`WINDOW_SECS`] 안 [`PER_WINDOW_MAX`]건 — [`admit`]
//!   ③ 해소 = operator token(GUI)만 · 예외는 발행 좌석 자신의 [`SUPERSEDED`] — [`reply_allowed`]
//!   ④ tier 는 데몬이 d 로 고정(원격 채널 미러 금지) · `wait` 금지 · CEO 자동결재 제외.
//!
//! ★정직한 한계: 이 잠금은 **사고 방지 층**이다 — 에이전트의 오해·실수, 대표가 승인 피드 구독
//!   흐름에서 자기 제안에 결정을 보내 카드를 소각하는 일, 다른 좌석의 무심한 allow 를 막는다.
//!   같은 UID 로 operator.token 을 읽어 raw RPC 를 보내는 **고의 우회**까지 막는 보안 경계는
//!   아니다(src-tauri `feed_reply` 주석의 M11 수준과 같은 성격). "에이전트는 팀을 만들 수 없다"는
//!   주장이 아니라 "에이전트가 실수로 팀을 만들거나 오너 확인을 건너뛰지 못하게 한다"는 주장이다.
//!
//! 사본(테스트가 대조한다 — 여기를 바꾸면 같이 바꾼다):
//!   · `cysjavis-pack/bin/cys-dept` allocate `--team-spec-b64` 파이썬 재검증(한도·스키마)
//!   · `ui/src/teamproposal.ts`(한도·kind — `teamproposal.test.ts` 가 이 파일을 읽어 대조)
//!   · `cysjavis-pack/hooks/session-start.sh` 팀 소개 블록의 소독 필터 = [`FORBIDDEN_TERMS`]
//!     (`test_team_create_u16.py` C1 이 대조)

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// feed kind — 이 값의 항목만 잠금 대상이다(다른 kind 는 바이트 무변경).
pub const KIND: &str = "team-create-request";
/// 본문 스키마 버전.
pub const SCHEMA_VERSION: u64 = 1;
/// 제안 id 접두 — `daemon-` 예약 네임스페이스와 겹치지 않는다.
pub const ID_PREFIX: &str = "tp-";
/// 제안 id 최대 길이(feed request_id 관례 `^[A-Za-z0-9_-]{1,64}$`).
pub const ID_MAX_LEN: usize = 64;
/// 팀 이름(표시명) 최대 글자 수(코드포인트).
pub const DISPLAY_MAX_CHARS: usize = 40;
/// 하는 일 최대 글자 수(코드포인트).
pub const PURPOSE_MAX_CHARS: usize = 2000;
/// 같은 kind 의 대기(pending) 상한.
pub const PENDING_MAX: usize = 1;
/// [`WINDOW_SECS`] 안의 제안 상한(상태 무관 — 거둔 제안도 센다).
pub const PER_WINDOW_MAX: usize = 3;
/// 빈도 창(24시간).
pub const WINDOW_SECS: f64 = 86_400.0;
/// 발행 좌석이 자기 제안을 거둘 때 쓰는 결정 어휘 — 유일한 비-GUI 해소.
pub const SUPERSEDED: &str = "superseded";

/// 권위어 — 이 낱말이 든 이름·하는 일은 받지 않는다. 팀 소개는 팀의 모든 자리에 매 세션
/// 주입되므로, 권위어가 섞이면 '헌장'급 권위를 얻는다(반박 M7·D8). session-start.sh 의
/// 로컬 오버레이 필터와 **같은 집합**이며 팀 소개 주입도 같은 필터를 한 번 더 건다(이중 방어).
/// 입력 단계에서 거부하는 이유: 주입 단계 필터만 두면 오너가 확인 창에서 본 줄이 조용히 빠진다
/// (오너가 본 내용 ≠ 주입 내용). 대소문자 무시(ASCII).
pub const FORBIDDEN_TERMS: &[&str] = &[
    "denylist",
    "deny list",
    "recovery",
    "kill-switch",
    "killswitch",
    "kill switch",
    "soul.md",
    "헌법",
    "헌장",
    "autopilot",
    "자율주행",
    "안전핵",
    "eval-driven",
];

/// 보이지 않는 서식·양방향 제어 문자 — 확인 창에서 '원문 그대로'를 보여도 눈에 보이는 순서·
/// 내용이 실제와 달라질 수 있어(스푸핑) 받지 않는다. UI 사본(`teamproposal.ts`)과 같은 집합.
const INVISIBLE: &[(char, char)] = &[
    ('\u{200B}', '\u{200F}'),
    ('\u{202A}', '\u{202E}'),
    ('\u{2060}', '\u{2064}'),
    ('\u{2066}', '\u{2069}'),
    ('\u{FEFF}', '\u{FEFF}'),
];

fn is_invisible(c: char) -> bool {
    INVISIBLE.iter().any(|(a, b)| c >= *a && c <= *b)
}

/// 오너가 확인 창에서 보고 만드는 팀의 정의 — 이 셋이 곧 '만들어지는 내용'이다.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamSpec {
    /// 제안 id(= feed request_id).
    pub id: String,
    /// 팀 이름(표시명 · 한글 가능 — 부서 키 `dept-N` 은 cys-dept 가 따로 발급한다).
    pub display: String,
    /// 하는 일(오너와 대표가 정한 문장 그대로).
    pub purpose: String,
}

/// 새 제안 id — `tp-<epoch초>-<4hex>`.
pub fn new_id(epoch_secs: u64, entropy: u32) -> String {
    format!("{ID_PREFIX}{epoch_secs}-{:04x}", entropy & 0xffff)
}

pub fn validate_id(id: &str) -> Result<(), String> {
    let ok = id.starts_with(ID_PREFIX)
        && id.len() > ID_PREFIX.len()
        && id.len() <= ID_MAX_LEN
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if ok {
        Ok(())
    } else {
        Err(format!(
            "제안 id 형식 오류: '{id}' — '{ID_PREFIX}' 로 시작하는 영문·숫자·'-'·'_' {ID_MAX_LEN}자 이내"
        ))
    }
}

fn check_terms(field: &str, s: &str) -> Result<(), String> {
    let low = s.to_lowercase();
    let hit: Vec<&str> = FORBIDDEN_TERMS
        .iter()
        .copied()
        .filter(|t| low.contains(&t.to_lowercase()))
        .collect();
    if hit.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{field}에 쓸 수 없는 낱말: {} — 팀 소개는 모든 자리에 참고 정보로 주입되므로 운영 권위어는 받지 않는다. 다른 말로 바꿔 다시 제안하라",
            hit.join(", ")
        ))
    }
}

/// 팀 이름(정규형) 검증: 앞뒤 공백 없음 · 1~40자 · 제어문자·보이지 않는 문자 없음 · 권위어 없음.
pub fn validate_display(s: &str) -> Result<(), String> {
    let n = s.chars().count();
    if n == 0 {
        return Err("팀 이름이 비었다".into());
    }
    if n > DISPLAY_MAX_CHARS {
        return Err(format!("팀 이름이 너무 길다({n}자 > {DISPLAY_MAX_CHARS}자)"));
    }
    if s.trim() != s {
        return Err("팀 이름 앞뒤에 공백을 두지 않는다".into());
    }
    if s.chars().any(|c| c.is_control() || is_invisible(c)) {
        return Err("팀 이름에 줄바꿈·제어문자·보이지 않는 문자를 쓰지 않는다".into());
    }
    check_terms("팀 이름", s)
}

/// 하는 일(정규형) 검증: 1~2000자 · 줄바꿈(\n)·탭만 허용(\r·널 등 제어문자 없음) ·
/// 보이지 않는 문자 없음 · 권위어 없음.
pub fn validate_purpose(s: &str) -> Result<(), String> {
    let n = s.chars().count();
    if s.trim().is_empty() {
        return Err("하는 일이 비었다".into());
    }
    if n > PURPOSE_MAX_CHARS {
        return Err(format!("하는 일이 너무 길다({n}자 > {PURPOSE_MAX_CHARS}자)"));
    }
    if s
        .chars()
        .any(|c| (c.is_control() && c != '\n' && c != '\t') || is_invisible(c))
    {
        return Err("하는 일에 제어문자(줄바꿈·탭 외)·보이지 않는 문자를 쓰지 않는다".into());
    }
    check_terms("하는 일", s)
}

pub fn validate(spec: &TeamSpec) -> Result<(), String> {
    validate_id(&spec.id)?;
    validate_display(&spec.display)?;
    validate_purpose(&spec.purpose)
}

/// 사람 입력을 정규형으로 만든다(CLI 전용): 이름은 앞뒤 공백 제거, 하는 일은 CRLF→LF 후 앞뒤
/// 공백 제거. 데몬·Tauri 는 정규화하지 않고 [`validate`] 만 한다(보인 내용 = 만든 내용).
pub fn build(id: &str, display_raw: &str, purpose_raw: &str) -> Result<TeamSpec, String> {
    let spec = TeamSpec {
        id: id.to_string(),
        display: display_raw.trim().to_string(),
        purpose: purpose_raw.replace("\r\n", "\n").trim().to_string(),
    };
    validate(&spec)?;
    Ok(spec)
}

/// feed 본문(JSON) — `{"v":1,"id":…,"display":…,"purpose":…}`.
pub fn to_body(spec: &TeamSpec) -> String {
    json!({"v": SCHEMA_VERSION, "id": spec.id, "display": spec.display, "purpose": spec.purpose})
        .to_string()
}

/// feed 본문 판독 + 검증. 키는 정확히 4개(숨은 필드 거부).
pub fn parse_body(body: &str) -> Result<TeamSpec, String> {
    let v: Value =
        serde_json::from_str(body).map_err(|e| format!("팀 제안 본문이 JSON 이 아니다: {e}"))?;
    let obj = v.as_object().ok_or("팀 제안 본문이 JSON 객체가 아니다")?;
    let mut keys: Vec<&str> = obj.keys().map(|k| k.as_str()).collect();
    keys.sort_unstable();
    if keys != ["display", "id", "purpose", "v"] {
        return Err(format!(
            "팀 제안 본문의 키는 v·id·display·purpose 넷이어야 한다(받은 키: {})",
            keys.join(",")
        ));
    }
    if obj.get("v").and_then(Value::as_u64) != Some(SCHEMA_VERSION) {
        return Err(format!("팀 제안 본문 버전이 {SCHEMA_VERSION} 이 아니다"));
    }
    let s = |k: &str| -> Result<String, String> {
        obj.get(k)
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| format!("팀 제안 본문의 '{k}' 가 문자열이 아니다"))
    };
    let spec = TeamSpec { id: s("id")?, display: s("display")?, purpose: s("purpose")? };
    validate(&spec)?;
    Ok(spec)
}

/// `cys-dept allocate --team-spec-b64` 인자 — 본문 JSON 의 URL-safe base64(패딩 포함).
/// ASCII 만 지나가므로 윈도우 argv·코드페이지와 무관하고, 알파벳에 '/' 가 없어 MSYS 경로 변환
/// 대상도 아니다(JSON 이 `{"` 로 시작하므로 첫 글자는 항상 `e` — 옵션으로 오인되지 않는다).
pub fn to_b64(spec: &TeamSpec) -> String {
    base64::engine::general_purpose::URL_SAFE.encode(to_body(spec).as_bytes())
}

pub fn from_b64(s: &str) -> Result<TeamSpec, String> {
    let raw = base64::engine::general_purpose::URL_SAFE
        .decode(s.as_bytes())
        .map_err(|e| format!("팀 제안 b64 판독 실패: {e}"))?;
    let body = String::from_utf8(raw).map_err(|_| "팀 제안 b64 가 UTF-8 이 아니다".to_string())?;
    parse_body(&body)
}

/// 카드·목록에 보이는 제목 — 데몬이 정한다(클라이언트 제목은 버린다).
pub fn title_for(spec: &TeamSpec) -> String {
    format!("팀 만들기 제안: {}", spec.display)
}

/// `cys team-propose` 가 보내는 feed.push 파라미터(데몬이 제목·tier 를 다시 정한다).
pub fn push_params(spec: &TeamSpec, surface_id: Option<u64>) -> Value {
    json!({
        "kind": KIND,
        "title": title_for(spec),
        "body": to_body(spec),
        "surface_id": surface_id,
        "request_id": spec.id,
        "wait": false,
        "tier": "d",
    })
}

/// ① 제안자 판정 — 본부(base) 레인 데몬 ∧ 발행 좌석 역할 = `master`.
pub fn publisher_ok(is_dept_lane: bool, publisher_role: Option<&str>) -> Result<(), String> {
    if is_dept_lane {
        return Err("팀 만들기 제안은 본부 데몬에서만 받는다 — 팀장(부서 레인)은 '새 팀은 본부 대표 자리에서 만들 수 있습니다'라고 안내만 한다".into());
    }
    match publisher_role {
        Some("master") => Ok(()),
        Some(r) => Err(format!(
            "팀 만들기 제안은 본부 대표(master) 자리에서만 올릴 수 있다(이 자리의 역할: {r})"
        )),
        None => Err("팀 만들기 제안은 본부 대표(master) 자리의 pane 안에서만 올릴 수 있다(발행 좌석을 확인하지 못했다)".into()),
    }
}

/// ② 건수 판정 — `existing` = (kind, status, created_at) 전 항목. 대기 [`PENDING_MAX`]건 ·
/// [`WINDOW_SECS`] 안 [`PER_WINDOW_MAX`]건(상태 무관).
pub fn admit<'a, I>(existing: I, now: f64) -> Result<(), String>
where
    I: IntoIterator<Item = (&'a str, &'a str, f64)>,
{
    let mut pending = 0usize;
    let mut recent = 0usize;
    for (kind, status, created_at) in existing {
        if kind != KIND {
            continue;
        }
        if status == "pending" {
            pending += 1;
        }
        if now - created_at < WINDOW_SECS {
            recent += 1;
        }
    }
    if pending >= PENDING_MAX {
        return Err(format!(
            "대기 중인 팀 만들기 제안이 이미 {pending}건 있다 — 오너가 확인 창에서 처리할 때까지 기다리거나, 고쳐 올리려면 먼저 `cys feed reply <id> superseded` 로 자기 제안을 거둔다"
        ));
    }
    if recent >= PER_WINDOW_MAX {
        return Err(format!(
            "24시간 안의 팀 만들기 제안이 이미 {recent}건이다(상한 {PER_WINDOW_MAX}) — 오너에게 사정을 말하고 나중에 다시 제안하거나, 오너가 전문가용 메뉴로 직접 만들게 한다"
        ));
    }
    Ok(())
}

/// ③ 해소 판정 — operator token(GUI) 이면 어떤 결정이든, 아니면 발행 좌석 자신의
/// `superseded` 만.
pub fn reply_allowed(
    decision: &str,
    operator_ok: bool,
    caller_sid: Option<u64>,
    publisher_sid: Option<u64>,
) -> bool {
    operator_ok || (decision == SUPERSEDED && caller_sid.is_some() && caller_sid == publisher_sid)
}

/// 해소 거부 문구(에이전트에게 보인다 — 합법 경로를 함께 적는다).
pub const REPLY_DENIED: &str = "팀 만들기 제안은 결정 대상이 아니다 — 오너가 앱의 확인 창에서만 만들거나 만들지 않기로 정한다(operator token). 제안한 대표 자리는 `superseded` 로 자기 제안을 거둘 수만 있다";

/// Tauri 가 생성 직전에 대조한다: base 데몬 `feed.list` 결과에서 같은 id 의 항목이 아직
/// pending 이고 kind·본문이 오너가 본 spec 과 같은가(확인 창이 떠 있는 동안 제안이 거둬지거나
/// 바뀐 경우 만들지 않는다 — TOCTOU 차단).
pub fn match_pending(feed_list_result: &Value, spec: &TeamSpec) -> Result<(), String> {
    let items = feed_list_result
        .get("items")
        .and_then(Value::as_array)
        .ok_or("feed 목록을 읽지 못했다")?;
    let it = items
        .iter()
        .find(|i| i.get("request_id").and_then(Value::as_str) == Some(spec.id.as_str()))
        .ok_or("이 팀 제안이 승인 피드에 없다 — 이미 정리됐다")?;
    if it.get("kind").and_then(Value::as_str) != Some(KIND) {
        return Err("팀 만들기 제안 항목이 아니다".into());
    }
    if it.get("status").and_then(Value::as_str) != Some("pending") {
        return Err("이 팀 제안은 이미 처리됐거나 거둬졌다 — 새로 만들지 않는다".into());
    }
    let body = it.get("body").and_then(Value::as_str).unwrap_or("");
    let cur = parse_body(body)?;
    if &cur != spec {
        return Err("확인 창의 내용과 현재 제안이 다르다 — 카드를 다시 열어 확인하라".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> TeamSpec {
        TeamSpec { id: "tp-1726000000-00ab".into(), display: "영상편집팀".into(), purpose: "유튜브 영상을 편집한다.\n자막도 단다.".into() }
    }

    #[test]
    fn body_and_b64_round_trip() {
        let s = spec();
        assert_eq!(parse_body(&to_body(&s)).unwrap(), s);
        let b = to_b64(&s);
        assert!(b.starts_with('e'), "JSON '{{\"' → b64 첫 글자 e: {b}");
        assert!(b.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_' || c == b'='),
                "URL-safe 알파벳 밖 문자: {b}");
        assert!(!b.contains('/'), "MSYS 경로 변환 대상 문자 '/' 포함");
        assert_eq!(from_b64(&b).unwrap(), s);
    }

    #[test]
    fn validation_limits_and_terms() {
        assert!(validate_display(&"가".repeat(DISPLAY_MAX_CHARS)).is_ok());
        assert!(validate_display(&"가".repeat(DISPLAY_MAX_CHARS + 1)).is_err());
        assert!(validate_display("").is_err());
        assert!(validate_display(" 팀").is_err());
        assert!(validate_display("팀\n둘").is_err());
        assert!(validate_display("팀\u{202E}둘").is_err(), "양방향 제어(스푸핑)");
        assert!(validate_purpose(&"가".repeat(PURPOSE_MAX_CHARS)).is_ok());
        assert!(validate_purpose(&"가".repeat(PURPOSE_MAX_CHARS + 1)).is_err());
        assert!(validate_purpose("줄1\n\t줄2").is_ok());
        assert!(validate_purpose("줄1\r\n줄2").is_err(), "정규형은 LF 만");
        assert!(validate_purpose("일\u{0}").is_err());
        assert!(validate_purpose("   ").is_err());
        for t in FORBIDDEN_TERMS {
            assert!(validate_purpose(&format!("앞 {} 뒤", t.to_uppercase())).is_err(), "권위어 통과: {t}");
        }
        assert!(validate_id("tp-1-ab").is_ok());
        assert!(validate_id("tp-").is_err());
        assert!(validate_id("daemon-1").is_err());
        assert!(validate_id("tp-a b").is_err());
        assert!(validate_id(&format!("tp-{}", "a".repeat(ID_MAX_LEN))).is_err());
    }

    #[test]
    fn build_normalizes_only_in_cli() {
        let s = build("tp-9-0001", "  팀  ", "\r\n줄1\r\n줄2\r\n").unwrap();
        assert_eq!(s.display, "팀");
        assert_eq!(s.purpose, "줄1\n줄2");
        let id = new_id(1_726_000_000, 0x1_00ab);
        assert_eq!(id, "tp-1726000000-00ab");
        assert!(validate_id(&id).is_ok());
    }

    #[test]
    fn parse_body_rejects_hidden_fields_and_versions() {
        let s = spec();
        let mut v: Value = serde_json::from_str(&to_body(&s)).unwrap();
        v["extra"] = json!("x");
        assert!(parse_body(&v.to_string()).is_err());
        let mut v: Value = serde_json::from_str(&to_body(&s)).unwrap();
        v["v"] = json!(2);
        assert!(parse_body(&v.to_string()).is_err());
        assert!(parse_body("[]").is_err());
        assert!(parse_body("not json").is_err());
    }

    #[test]
    fn policy_admit_and_reply() {
        let now = 1_000_000.0;
        assert!(admit(Vec::<(&str, &str, f64)>::new(), now).is_ok());
        assert!(admit(vec![(KIND, "pending", now - 10.0)], now).is_err());
        assert!(admit(vec![("permission", "pending", now)], now).is_ok(), "다른 kind 무관");
        let three = vec![(KIND, "resolved", now - 1.0), (KIND, "resolved", now - 2.0), (KIND, "resolved", now - 3.0)];
        assert!(admit(three.clone(), now).is_err());
        let old: Vec<(&str, &str, f64)> = three.iter().map(|(k, s, t)| (*k, *s, *t - WINDOW_SECS)).collect();
        assert!(admit(old, now).is_ok(), "24시간 밖은 계수 제외");

        assert!(reply_allowed("allow", true, None, Some(3)));
        assert!(reply_allowed("deny", true, None, Some(3)));
        assert!(reply_allowed(SUPERSEDED, false, Some(3), Some(3)));
        assert!(!reply_allowed(SUPERSEDED, false, Some(4), Some(3)), "다른 좌석의 superseded");
        assert!(!reply_allowed(SUPERSEDED, false, None, None), "발행 좌석 미상 — 무귀속 호출자");
        for d in ["allow", "deny", "yes", "approve", "dismissed"] {
            assert!(!reply_allowed(d, false, Some(3), Some(3)), "발행자의 {d}");
        }

        assert!(publisher_ok(false, Some("master")).is_ok());
        assert!(publisher_ok(true, Some("master")).is_err());
        assert!(publisher_ok(false, Some("worker")).is_err());
        assert!(publisher_ok(false, Some("cso")).is_err());
        assert!(publisher_ok(false, None).is_err());
    }

    #[test]
    fn match_pending_toctou() {
        let s = spec();
        let list = |status: &str, body: String| json!({"items": [
            {"request_id": s.id, "kind": KIND, "status": status, "body": body}
        ]});
        assert!(match_pending(&list("pending", to_body(&s)), &s).is_ok());
        assert!(match_pending(&list("resolved", to_body(&s)), &s).is_err(), "이미 처리됨");
        let mut other = s.clone();
        other.purpose = "다른 일".into();
        assert!(match_pending(&list("pending", to_body(&other)), &s).is_err(), "내용 변경");
        assert!(match_pending(&json!({"items": []}), &s).is_err(), "항목 없음");
        assert!(match_pending(&json!({}), &s).is_err(), "목록 판독 불가");
    }
}
