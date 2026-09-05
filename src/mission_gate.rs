//! 임무 유래 판정(Rust) — `cysjavis-pack/bin/javis_mission.py` 의 **파리티 이식**(부트 v2 B1).
//!
//! # 이 모듈이 답하는 질문 하나
//! "이 프롬프트는 **오너가 직접 친 것**인가, 아니면 기계가 만든 것인가."
//! 답이 틀리면 두 방향으로 사고가 난다 — 기계 산출이 오너 임무로 등록되면 자율주행이 엉뚱한
//! 일을 하고(2026-08-22 실사고: `<task-notification>` 이 임무 대장을 덮었다), 반대로 오너
//! 지시가 기계로 접히면 부재중 자율 진행이 통째로 멈춘다. **후자가 훨씬 비싸다** — 그래서
//! 이 모듈의 모든 규칙은 애매하면 **통과**시킨다.
//!
//! # 세 층은 병렬이며 서로를 대체하지 않는다
//! | 층 | 무엇을 보는가 | 왜 다른 층이 못 잡는가 |
//! |---|---|---|
//! | 0 `harness_origin` | 프롬프트 **자체**의 태그 구조 | harness 가 프로세스 **내부에서** 합성한 알림은 배달 원장에 아예 없고 `[` 로 시작하지도 않는다 |
//! | 1 배달 원장 대조 | 데몬이 이 pane 에 **주입한 사실** | 층0 은 태그가 없는 평문 push 를 못 본다 |
//! | 2 라벨 | 선두 `[` push 규약 | 원장이 없거나(부트스트랩) 회전으로 소실된 구간의 2차 방어 |
//!
//! # 정본과 파리티 계약
//! 알고리즘의 정본은 여전히 `javis_mission.py` 다. 이 모듈은 그 **행동**을 복제하며, 두
//! 구현이 갈라졌는지는 차분 하네스(같은 입력을 양쪽에 먹여 결과를 대조)가 확인한다.
//!
//! # 왜 Rust 로 옮기는가
//! 이 판정은 훅 hot path 다. python 이 없는 기계(Windows 기본 설치)에서는 판정이 **아예
//! 일어나지 않아** 기계 산출이 그대로 오너 임무가 됐다(종전 A22). 인터프리터 의존을 끊는다.
//!
//! # ★정직 고지 — lookahead 는 정규식 밖으로 꺼냈다
//! python 의 마커 태그 정규식은 이름 경계를 `(?![\w-])` **부정 lookahead** 로 못박는다
//! (`javis_mission.py:920-936`). Rust `regex` 는 look-around 를 지원하지 않으므로, 그 경계를
//! **정규식 문법으로 등가 변환**했다: 마커 뒤에 올 수 있는 것은 `>` 이거나
//! `[^\w<>-]` 로 시작하는 속성부뿐이다(`[^<>]*` 가 뒤따르므로 문자열 끝은 올 수 없다).
//! 의미는 동일하고 신규 의존은 0 이다. 같은 결정을 [`crate::declaration`] 도 했다.

use regex::Regex;
use std::collections::HashMap;
use std::sync::OnceLock;

// ══════════════════════════════════════════════════════════════════════════════
// 공용 — 배달 정규화·해시(원장 대조의 단일 산식)
// ══════════════════════════════════════════════════════════════════════════════

/// 배달 원장 대조용 정규화(순수). python `_normalize_delivery` · 종전
/// `cysd::delivery::normalize` 와 **같은 함수**다.
///
/// ① 모든 공백 문자를 버리고 ② 공백이 있던 자리는 ASCII 공백 **1개**로 접고 ③ 앞뒤 공백은
/// 제거한다. 대소문자·유니코드 정규화(NFC)는 **하지 않는다** — 접는 폭이 넓을수록 오너 문장이
/// 기계로 오인될 확률만 커지고, TUI 는 코드포인트를 바꾸지 않는다.
///
/// ★공백 집합이 `char::is_whitespace()`(유니코드 White_Space · 25종)인 것은 **의도**다.
/// python 쪽도 같은 25종을 `_WHITESPACE` 상수로 **명시 열거**한다(`javis_mission.py:467-471`)
/// — 즉 python 은 여기서 자기 `\s`(29종)를 쓰지 않는다. 실측으로 두 집합의 대칭차는 0 이다.
/// (이 모듈의 **다른** 자리 — 층0 의 잔여문 계수 — 는 python `str.isspace()` 를 쓰므로
/// [`is_py_space`] 라는 별도 술어가 필요하다. 두 술어를 섞으면 조용히 갈린다.)
///
/// ★이 함수가 lib 로 올라온 이유(부트 v2): 종전에는 `cysd` 바이너리 안에만 있어서
/// `cys hook`(CLI 바이너리)이 같은 산식을 쓸 수 없었다. 사본을 만들면 두 판정이 갈리는데,
/// 갈리는 순간 원장 대조는 **조용히** 무력화된다(해시가 안 맞으니 항상 '미일치').
pub fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut pending_space = false;
    for ch in text.chars() {
        if ch.is_whitespace() {
            pending_space = !out.is_empty();
            continue;
        }
        if pending_space {
            out.push(' ');
            pending_space = false;
        }
        out.push(ch);
    }
    out
}

/// **이미 정규화된** 문자열의 sha256 소문자 hex. python `_digest_norm` 미러.
///
/// 재정규화하지 않는 것이 계약이다 — 부분문자열 대조는 이미 정규화된 프롬프트를 조각내
/// 해시하므로, 다시 정규화하면 조각의 앞뒤 공백을 벗겨 **의미가 바뀐다**.
pub fn digest_normalized(norm: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(norm.as_bytes()))
}

/// 원문 → 정규화 → sha256. 원장 대조 키와 **같은 산식**이다(python `delivery_digest`).
pub fn digest_text(text: &str) -> String {
    digest_normalized(&normalize(text))
}

// ══════════════════════════════════════════════════════════════════════════════
// 이상징후 코드 — 등재소(python `ANOMALY_CODES` 파리티)
// ══════════════════════════════════════════════════════════════════════════════

/// 이상징후 코드 등재소. **이 표에 없는 코드는 만들지 않는다** — 코드가 산발하면 감사에서
/// "무슨 일이 있었나" 를 한 곳에서 셀 수 없다.
///
/// ★이것이 아닌 것: 판정 **자체**(예: 원장이 판독 불가라 게이트를 닫았다)는 이상징후가 아니라
/// 결과다. 그것은 `ledger_status`·`reason` 이 나른다.
pub const ANOMALY_CODES: [(&str, &str); 13] = [
    // ── 원장 상태 유래 — 매 판독마다 재관측된다 ──
    ("ledger_absent", "배달 원장 부재 — 층1(원장 대조) 근거 없이 층2(라벨)로만 판별 중"),
    ("ledger_rotated", "원장 회전 — 소실 구간의 기계 push 는 층1 로 대조 불가"),
    ("ledger_bad_lines", "해석 불가 줄 혼입(부분쓰기·조작 정황)"),
    (
        "ledger_schema_skew",
        "원장에 이 판독자가 모르는 스키마 버전이 섞였다 — 그 배달은 층1 에서 통째로 보이지 않는다",
    ),
    (
        "delivery_parts_capped",
        "배달이 조각 상한을 넘겨 초과분 행이 원장에 없다 — 그 배달 직후의 미매치 프롬프트는 판정을 접는다",
    ),
    // ── 프롬프트 유래 — 그 프롬프트에서 1회만 관측된다 ──
    ("delivery_out_of_window", "창 밖 배달과 전문 일치 — 접었으나 지연이 비정상"),
    ("delivery_concatenated", "기계 배달 둘 이상이 한 프롬프트로 연접 제출됨"),
    ("delivery_substring", "기계 배달이 프롬프트에 통째로 포함됨"),
    (
        "delivery_anchor_capped",
        "부분 일치 탐색이 예산에 도달 — 못 본 구간이 있어 판정을 접었다",
    ),
    (
        "delivery_prompt_within_delivery",
        "프롬프트가 더 긴 기계 배달의 한 조각과 겹침(근거는 preview 평문 대조이며 해시 확증이 아니다)",
    ),
    // ── env 오버라이드 유래 — 기동 시점 확정 ──
    ("env_not_int", "정수 아닌 env 오버라이드 — 기본값 적용"),
    ("env_below_floor", "하한 미만 env 오버라이드 — 거부하고 기본값 적용"),
    ("env_above_cap", "상한 초과 env 오버라이드 — 상한으로 절단"),
];

/// 등재 여부(순수). 미등재 코드는 **버리지 않고** 통과시키되, 검체가 등재소 정합을 잰다 —
/// 판정 중에 예외를 던지면 그 프롬프트의 임무가 통째로 사라진다(치료가 병보다 나쁘다).
pub fn is_registered_anomaly(code: &str) -> bool {
    ANOMALY_CODES.iter().any(|(c, _)| *c == code)
}

// ══════════════════════════════════════════════════════════════════════════════
// 층2 — push 규약 라벨
// ══════════════════════════════════════════════════════════════════════════════

/// 전각 대괄호 — 한국어 IME 에서 흔히 섞여 들어온다.
const FULLWIDTH_BRACKET: char = '［';

/// python `_is_transparent` 와 **같은 집합**(유니코드 카테고리 `Cf` + 폴백 목록).
///
/// ★구간표는 python 구현을 **전수 열거해 얻은 실측치**다(전 코드포인트 대조 · 170자 21구간).
/// Rust 표준 라이브러리에는 유니코드 카테고리 조회가 없어서 카테고리를 근사하면 갈린다 —
/// 근사 대신 측정값을 박제한다. 새 유니코드 버전에서 `Cf` 가 늘면 검체가 그것을 말한다.
const TRANSPARENT_RANGES: [(char, char); 21] = [
    ('\u{AD}', '\u{AD}'),
    ('\u{600}', '\u{605}'),
    ('\u{61C}', '\u{61C}'),
    ('\u{6DD}', '\u{6DD}'),
    ('\u{70F}', '\u{70F}'),
    ('\u{890}', '\u{891}'),
    ('\u{8E2}', '\u{8E2}'),
    ('\u{180E}', '\u{180E}'),
    ('\u{200B}', '\u{200F}'),
    ('\u{202A}', '\u{202E}'),
    ('\u{2060}', '\u{2064}'),
    ('\u{2066}', '\u{206F}'),
    ('\u{FEFF}', '\u{FEFF}'),
    ('\u{FFF9}', '\u{FFFB}'),
    ('\u{110BD}', '\u{110BD}'),
    ('\u{110CD}', '\u{110CD}'),
    ('\u{13430}', '\u{1343F}'),
    ('\u{1BCA0}', '\u{1BCA3}'),
    ('\u{1D173}', '\u{1D17A}'),
    ('\u{E0001}', '\u{E0001}'),
    ('\u{E0020}', '\u{E007F}'),
];

/// 보이지 않는 서식 문자인가(python `_is_transparent`).
pub fn is_transparent(c: char) -> bool {
    TRANSPARENT_RANGES.iter().any(|&(lo, hi)| lo <= c && c <= hi)
}

/// python `str.isspace()` 와 **동일 집합**(29 코드포인트 — 실측 확인).
///
/// Rust `char::is_whitespace` 는 유니코드 `White_Space`(25종)이라 `\x1C`–`\x1F` 가 빠진다.
/// [`normalize`] 는 25종 쪽을 쓰고(python 도 거기선 명시 열거한다) 층0·라벨 판정은 29종 쪽을
/// 쓴다 — **두 술어를 섞으면 조용히 갈린다**. 그래서 이름을 따로 둔다.
pub fn is_py_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1C}'..='\u{1F}').contains(&c)
}

/// 선행 공백·투명문자를 벗긴 첫 글자(python `_label_head`).
pub fn label_head(prompt: &str) -> Option<char> {
    prompt
        .chars()
        .find(|&c| !is_py_space(c) && !is_transparent(c))
}

/// 층2 — 선두 `[`·`［` = push 규약 라벨(python `has_machine_label`).
///
/// 왜 라벨이 근거가 되는가: 라벨 없는 push 는 수신 노드의 임무 게이트에서 '오너가 직접 친
/// 문장' 과 in-band 로 구별되지 않는다(2026-08-01 사고 기제). 1차 방어는 배달 원장(층1)이고
/// 라벨은 원장이 없을 때의 2차 방어다 — 둘 다 있어야 심층 방어가 성립한다.
pub fn has_machine_label(prompt: &str) -> bool {
    matches!(label_head(prompt), Some('[') | Some(FULLWIDTH_BRACKET))
}

// ══════════════════════════════════════════════════════════════════════════════
// 층0 — harness·도구 **내부 알림** 필터
// ══════════════════════════════════════════════════════════════════════════════

/// 잔여문이 이 값 미만이면 임무로 인정하지 않는다(python `MISSION_MIN_CHARS`).
pub const MISSION_MIN_CHARS: usize = 3;
/// 판정 비용 상한(문자). 초과해도 **통과시키지 않고** 앞부분으로 판정한다.
pub const HARNESS_SCAN_MAX_CHARS: usize = 5_000_000;
/// 상한 초과 시 보는 앞부분 길이. 잘린 프리픽스는 태그 짝이 깨져 **오너 통과 방향**으로
/// 기울므로 삼킴 위험을 늘리지 않는다.
pub const HARNESS_SCAN_PREFIX_CHARS: usize = 200_000;

/// ★(v2.1 **A14** · master 채택 2026-09-04) 층1 RPC `prompt_norm` 의 **전송 상한 — 바이트**.
///
/// ## 왜 문자가 아니라 바이트인가(실측)
/// 종전 상한은 [`HARNESS_SCAN_MAX_CHARS`](5,000,000 **자**)였고, 그것은 **판정 비용** 축의
/// 단위다. 그런데 이 값이 RPC 검증에도 쓰이면서 단위가 섞였다 — 전송을 끊는 것은 데몬의
/// `MAX_REQUEST_LINE`(10 MiB **바이트**)이기 때문이다. 실측 결과 상한의 의미가 **입력 언어에
/// 따라 달라졌다**:
///   · 한글 5,000,000자 = 원시 14.3 MiB → **와이어 상한 초과로 애초에 전송 불가**(문자 상한은
///     한 번도 발효하지 못한다 · CJK 3 B/자)
///   · 같은 5,000,000자가 ASCII 면 4.8 MiB 라 전송된다
/// 즉 같은 계약 문장이 언어마다 다른 것을 뜻했다. A14 는 이것을 바이트로 통일한다.
///
/// ## 값의 근거(전부 실측 · 2026-09-04)
/// serde_json 직렬화의 **자당 최대 팽창은 6배**다(제어문자 → `\uXXXX` 6 B · 실측: 한글 3 ·
/// ASCII 1 · 따옴표·역슬래시 2 · U+0001 **6**). 봉투(메서드·digest 64자·플래그)는 **178 B**다.
/// 그래서 원시 1 MiB 는 최악의 입력에서도 `1 MiB × 6 + 178 B ≈ 6 MiB < 10 MiB` 로 **입력 내용과
/// 무관하게** 와이어 상한 아래임이 보장된다 — 상한이 조건부로 발효하지 않는다는 것이 요점이다.
/// 상용 규모 대조: 한글 200,000자 = 0.57 MiB · 한글 349,525자가 이 상한이다(실전 프롬프트를
/// 자르지 않는다). 이 부등식은 검체 `H-A14-1` 이 `MAX_REQUEST_LINE` 소스 핀과 함께 기계 대조한다.
pub const LAYER1_PROMPT_MAX_BYTES: usize = 1024 * 1024;

/// [`LAYER1_PROMPT_MAX_BYTES`] 산출 근거의 상수 — 자당 최대 직렬화 팽창(제어문자 `\uXXXX`).
/// 검체가 이 값으로 부등식을 다시 계산한다(주석의 숫자가 아니라 **값**이 근거다).
pub const JSON_WORST_EXPANSION: usize = 6;

/// 층1 전송용 **바이트 절단** — UTF-8 경계를 지키며 `max` 이하로 자른다. `(자른 문자열, 잘렸는가)`.
///
/// 문자 경계에서 자르는 것이 계약이다: 바이트로 무작정 자르면 다중바이트 문자가 반토막 나
/// **유효하지 않은 UTF-8** 이 되고, 그 문자열은 JSON 직렬화에서 죽거나 대체문자로 바뀌어
/// digest 대조가 조용히 깨진다(층1 이 통째로 무력화되는 경로).
pub fn truncate_utf8_bytes(s: &str, max: usize) -> (&str, bool) {
    if s.len() <= max {
        return (s, false);
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    (&s[..end], true)
}

/// 알림 전용 마커 — harness 가 turn 자체를 합성할 때만 나온다.
pub const HARNESS_NOTIFY_MARKERS: [&str; 12] = [
    "task-notification",
    "system-reminder",
    "local-command-caveat",
    // ★2026-08-22 적대검증 치명②: 같은 슬래시 명령 가족인데 `-stdout` 이 빠져 있었다 —
    //   `/cost` 형태가 그대로 오너 임무로 기록됐다(사고 원문과 동형 재현).
    "local-command-stdout",
    "local-command-stderr",
    "command-name",
    "command-message",
    "command-args",
    "bash-input",
    "bash-stdout",
    "bash-stderr",
    "ide_selection",
];

/// 컨텍스트 마커 — 알림 블록 **내부**에서만 의미를 갖는 범용 어휘.
///
/// ★2계층을 남겨 둔 이유(판정상 구분은 없다): `summary`·`status` 는 일상 단어이자 일반 HTML
/// 태그다. 이것들을 '미종결 절단 폴백' 대상에 두면 오너 문장이 통째로 삼켜진다(실측 관통
/// 2형). 절단 폴백 자체는 제거됐지만, **되살린다면** 이 2계층이 최소 안전장치의 출발점이다.
pub const HARNESS_CONTEXT_MARKERS: [&str; 5] =
    ["tool-use-id", "output-file", "summary", "status", "task-id"];

/// 전 마커(중복 제거 · python `HARNESS_MARKERS` 와 **같은 집합**).
fn harness_markers() -> &'static Vec<String> {
    static M: OnceLock<Vec<String>> = OnceLock::new();
    M.get_or_init(|| {
        let mut v: Vec<String> = Vec::new();
        for m in HARNESS_NOTIFY_MARKERS.iter().chain(HARNESS_CONTEXT_MARKERS.iter()) {
            if !v.iter().any(|x| x == m) {
                v.push((*m).to_string());
            }
        }
        v
    })
}

/// python `(?![\w-])` 이름 경계의 **등가 변환**(모듈 머리말 참조).
///
/// 마커 뒤에 올 수 있는 것은 ⓐ `>` 이거나 ⓑ `[^\w<>-]` 로 시작하는 속성부뿐이다 —
/// `[^<>]*>` 가 뒤따르므로 문자열 끝은 올 수 없고, `[\w-]` 이면 그것은 **다른 태그 이름**이다
/// (`<summary-of-changes>` 를 `<summary>` 로 오인하지 않는 근거).
const NAME_BOUNDARY_TAIL: &str = r"(?:>|[^\w<>-][^<>]*>)";

fn harness_tag_rx() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        // 교대는 **긴 이름 우선**(접두 관계가 생겨도 짧은 쪽이 먼저 먹지 않게).
        let mut names: Vec<&str> = harness_markers().iter().map(|s| s.as_str()).collect();
        names.sort_by_key(|n| std::cmp::Reverse(n.len()));
        let alt = names.iter().map(|n| regex::escape(n)).collect::<Vec<_>>().join("|");
        Regex::new(&format!(r"(?i)<\s*/?\s*(?:{alt}){NAME_BOUNDARY_TAIL}"))
            .expect("HARNESS_TAG 컴파일 실패")
    })
}

/// 마커별 (여는~닫는 블록, 닫는 태그) 정규식 쌍. 순서는 [`harness_markers`] 와 같다.
fn harness_block_rx() -> &'static Vec<(Regex, Regex)> {
    static R: OnceLock<Vec<(Regex, Regex)>> = OnceLock::new();
    R.get_or_init(|| {
        harness_markers()
            .iter()
            .map(|m| {
                let e = regex::escape(m);
                (
                    Regex::new(&format!(
                        r"(?is)<\s*{e}{NAME_BOUNDARY_TAIL}.*?<\s*/\s*{e}\s*>"
                    ))
                    .expect("HARNESS_BLOCK 컴파일 실패"),
                    Regex::new(&format!(r"(?i)<\s*/\s*{e}\s*>")).expect("HARNESS_CLOSE 컴파일 실패"),
                )
            })
            .collect()
    })
}

fn any_tag_rx() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(r"(?s)<\s*(/?)\s*([A-Za-z][\w:-]*)(?:\s[^<>]*?)?(/?)\s*>")
            .expect("ANY_TAG 컴파일 실패")
    })
}

fn fence_rx() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(r"(?s)```.*?```|~~~.*?~~~|`[^`\n]*`").expect("FENCE 컴파일 실패")
    })
}

/// 공백 접기(python `_WS.sub(" ", …).strip()`). **python `\s` 집합(29종)** 을 쓴다.
fn collapse_ws(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut pending = false;
    for c in s.chars() {
        if is_py_space(c) {
            pending = !out.is_empty();
            continue;
        }
        if pending {
            out.push(' ');
            pending = false;
        }
        out.push(c);
    }
    out
}

/// 자유 텍스트 글자 수 — **공백이 아닌 문자 전부**(python `_meaningful_chars`).
///
/// `isalnum()` 이 아닌 이유(실측 봉합): 이모지·기호로만 쓴 지시("`<a>1</a> 👉🔥⚡️`")가 0자로
/// 계산돼 **오너 프롬프트가 기계로 접혔다**. 이 축이 묻는 것은 "태그 밖에 사람이 쓴 것이
/// **전혀** 없는가" 뿐이므로 공백만 빼고 전부 센다.
fn meaningful_chars(text: &str) -> usize {
    text.chars().filter(|&c| !is_py_space(c)).count()
}

/// 마커 블록·잔여 마커 태그를 제거한 잔여문(python `_harness_strip`).
///
/// ① 짝이 맞는 블록을 **수렴할 때까지** 제거(본문 포함)
/// ② 짝이 없는 태그는 **태그 자체만** 제거하고 뒤 본문은 남긴다(절단 금지)
///
/// ★②가 '절단 금지' 인 이유(2026-08-22 적대검증 2회차 · master 결정): 초판은 "여는 태그는
/// 있는데 닫는 태그가 없으면 그 지점부터 **문자열 끝까지** 잘라낸다" 는 폴백을 뒀고, 그것이
/// 오너 지시를 통째로 삼켰다(실측 3형). 대가는 "잘린 기계 알림은 이 축을 통과할 수 있다" 이며
/// 알고 받아들인다 — 잘린 알림은 한 번도 관측된 적 없는 가설이고, 오너 지시 삼킴은 실측된
/// 평시 경로다. **되살리려면 잘린 알림이 실제로 관측된 로그를 먼저 가져와라.**
pub fn harness_strip(text: &str) -> String {
    let mut out = text.to_string();
    for (block, close) in harness_block_rx().iter() {
        // ★비용 가드: 닫는 태그가 하나도 없으면 짝 블록도 있을 수 없다. 그런데 `.*?` 는 그
        //   사실을 모른 채 여는 태그마다 문자열 끝까지 훑는다 = O(n²).
        if !close.is_match(&out) {
            continue;
        }
        loop {
            let next = block.replace_all(&out, " ").into_owned();
            if next == out {
                break;
            }
            out = next;
        }
    }
    out = harness_tag_rx().replace_all(&out, " ").into_owned();
    collapse_ws(&out)
}

/// **짝이 맞는** 일반 태그 블록만 제거(python `_strip_generic_blocks`). 선형 스택 스캐너.
///
/// 반환 `(잔여, 제거 블록 수)`. 정규식 역참조(`<(\w+)>.*?</\1>`)를 쓰지 않는 이유는 그것이
/// 닫히지 않는 여는 태그마다 문자열 끝까지 훑어 O(n²) 이고(실측 10,000개 → 1.84s), 중간에 낀
/// 미종결 태그 때문에 **바깥 블록을 놓치기** 때문이다. Rust `regex` 는 애초에 역참조를
/// 지원하지도 않으므로 이 선형 스캐너가 유일한 정답이자 python 과 같은 구현이다.
fn strip_generic_blocks(seg: &[char]) -> (Vec<char>, usize) {
    let s: String = seg.iter().collect();
    // 문자 오프셋으로 다루기 위해 바이트→문자 환산.
    let mut b2c = vec![0usize; s.len() + 1];
    for (ci, (bi, _)) in s.char_indices().enumerate() {
        b2c[bi] = ci;
    }
    b2c[s.len()] = seg.len();
    let mut stack: Vec<(String, usize)> = Vec::new();
    let mut spans: Vec<(usize, usize)> = Vec::new();
    for m in any_tag_rx().captures_iter(&s) {
        let whole = m.get(0).expect("전체 매치");
        let (start, end) = (b2c[whole.start()], b2c[whole.end()]);
        let closing = !m.get(1).map(|x| x.as_str()).unwrap_or("").is_empty();
        let name = m.get(2).map(|x| x.as_str()).unwrap_or("").to_lowercase();
        let selfclose = !m.get(3).map(|x| x.as_str()).unwrap_or("").is_empty();
        if closing {
            // 같은 이름을 **뒤에서부터** 찾아 짝짓는다(그 사이 미종결 태그는 버린다).
            if let Some(i) = stack.iter().rposition(|(n, _)| *n == name) {
                spans.push((stack[i].1, end));
                stack.truncate(i);
            }
        } else if selfclose {
            spans.push((start, end)); // 자기닫힘 = 그 자체로 완결된 블록
        } else {
            stack.push((name, start));
        }
    }
    if spans.is_empty() {
        return (seg.to_vec(), 0);
    }
    spans.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (a, b) in spans {
        match merged.last_mut() {
            Some(last) if a <= last.1 => last.1 = last.1.max(b),
            _ => merged.push((a, b)),
        }
    }
    let mut out: Vec<char> = Vec::with_capacity(seg.len());
    let mut last = 0usize;
    for &(a, b) in merged.iter() {
        out.extend_from_slice(&seg[last..a]);
        out.push(' ');
        last = b;
    }
    out.extend_from_slice(&seg[last..]);
    (out, merged.len())
}

/// 코드펜스·인라인 코드를 **같은 길이**의 자리표시자(`\0`)로 치환(python `_mask_fences`).
///
/// split 이 아니라 mask 인 이유(적대검증 2회차 치명2): split 은 여는 태그와 닫는 태그가 펜스
/// 양쪽으로 갈리면 짝이 성립하지 않아 구조 축이 통째로 미발화했다 — **백틱 한 쌍이 이 축을
/// 끄는 스위치**가 되는 셈이었다. 마스킹은 길이를 보존하므로 짝 맞추기가 구간을 가로질러
/// 성립한다. 자리표시자가 `\0` 인 이유: `_ANY_TAG` 가 태그로 보지 않고, `meaningful_chars` 는
/// 공백이 아니므로 **자유 텍스트로 센다**(오너가 붙여넣은 코드는 지시의 일부다).
fn mask_fences(src: &str) -> Vec<char> {
    let chars: Vec<char> = src.chars().collect();
    if !src.contains('`') && !src.contains('~') {
        return chars; // 값싼 선검사(대부분의 프롬프트가 여기서 끝난다)
    }
    let mut b2c = vec![0usize; src.len() + 1];
    for (ci, (bi, _)) in src.char_indices().enumerate() {
        b2c[bi] = ci;
    }
    b2c[src.len()] = chars.len();
    let mut out = chars.clone();
    for m in fence_rx().find_iter(src) {
        for slot in out.iter_mut().take(b2c[m.end()]).skip(b2c[m.start()]) {
            *slot = '\0';
        }
    }
    out
}

/// `(자유 텍스트, 제거된 블록 수)` — 태그 밖에 사람이 쓴 것이 남는가
/// (python `generic_block_free_text`).
///
/// ① 코드펜스를 마스킹 ② 짝 맞는 블록·자기닫힘 블록 제거 ③ ★남은 **태그 마크업 자체**를
/// 제거한 뒤 센다 — 종전엔 짝이 안 맞아 남은 `<newtag>` 의 글자가 그대로 '자유 텍스트' 로
/// 계수돼 정의와 코드가 어긋나 있었다.
pub fn generic_block_free_text(text: &str) -> (String, usize) {
    let masked = mask_fences(text);
    let (seg, removed) = strip_generic_blocks(&masked);
    let s: String = seg.iter().collect();
    let s = any_tag_rx().replace_all(&s, " ").into_owned();
    (collapse_ws(&s), removed)
}

/// 층0 판정 — 이 프롬프트가 harness·도구가 **프로세스 내부에서** 합성한 알림인가.
///
/// 반환 `Some(사유)` 면 기계, `None` 이면 통과. 부작용 0(대장·원장 무접촉).
///
/// 축은 둘이며 **둘 다 '남은 내용'만 본다**(위치 무관 — 마커가 앞이든 뒤든 양쪽이든 잔여문이
/// 살아 있으면 오너 임무다):
///   ⓐ **이름 축** — 알려진 마커 블록을 걷어낸 잔여문. 짝이 안 맞는 잘린 알림까지 잡지만
///      목록은 영원히 불완전하다.
///   ⓑ **구조 축**(이름 독립) — 짝 맞는 태그 블록을 전부 걷고 자유 텍스트가 남는지 본다.
///      목록에 없는 새 태그(harness 버전업)도 잡는 1차선이다.
///
/// ★ⓑ의 발화 조건이 `== 0` 인 이유: `< MISSION_MIN_CHARS(3)` 이면 "`<div><p>x</p></div>`
/// 고쳐"(자유 2자) 같은 **오너 프롬프트를 새로 접는다**. 이 축의 취지는 "태그 밖에 사람이 쓴
/// 것이 **전혀** 없다" 이므로 0 이 정직한 구현이다.
///
/// ★기각된 규칙 — "시작부 지배"(초안에 있었다 · master 반려): harness 는 마커를 오너 프롬프트
/// **앞에** 덧붙이는 일이 일상적이라(`<system-reminder>` 선행 첨부 · 슬래시 명령 뒤의 진짜
/// 지시) 그 규칙이면 오너 지시가 임무로 등록되지 않는다. **되살리지 마라.**
pub fn harness_origin(prompt: &str) -> Option<String> {
    if prompt.trim().is_empty() {
        return None;
    }
    let chars: Vec<char> = prompt.chars().collect();
    let mut note = String::new();
    let p: String = if chars.len() > HARNESS_SCAN_MAX_CHARS {
        note = format!(
            " · ★프리픽스 판정(프롬프트 {}자 > 상한 {}자 — 앞 {}자만 보았다)",
            chars.len(),
            HARNESS_SCAN_MAX_CHARS,
            HARNESS_SCAN_PREFIX_CHARS
        );
        chars.iter().take(HARNESS_SCAN_PREFIX_CHARS).collect()
    } else {
        prompt.to_string()
    };
    // ⓐ 이름 축
    if harness_tag_rx().is_match(&p) {
        let residual = harness_strip(&p);
        let n = residual.chars().count();
        if n < MISSION_MIN_CHARS {
            let head: String = residual.chars().take(40).collect();
            return Some(format!(
                "harness 내부 알림 마커 블록을 제거한 **잔여문 {n}자 < 최소 {MISSION_MIN_CHARS}자** \
                 — 프롬프트가 기계 산출로 채워져 있다(잔여 {head:?}){note}"
            ));
        }
    }
    // ⓑ 구조 축
    let (free, nblocks) = generic_block_free_text(&p);
    if nblocks > 0 && meaningful_chars(&free) == 0 {
        return Some(format!(
            "태그 블록 {nblocks}개를 걷어내니 **태그 밖 자유 텍스트가 0자** — 프롬프트가 태그 \
             블록만으로 이루어졌다(마커 이름과 무관한 구조 신호){note}"
        ));
    }
    None
}

// ══════════════════════════════════════════════════════════════════════════════
// 층0-c — **기동 명령문** 필터(사람이 친 것도, 기계 알림도 아닌 제3의 오염원)
// ══════════════════════════════════════════════════════════════════════════════

/// 임무 본문 상한(python `MISSION_MAX_CHARS`). 층0-c 는 이보다 긴 붙여넣기를 대상으로 보지
/// 않는다 — 긴 문장은 기동 한 줄이 아니다.
pub const MISSION_MAX_CHARS: usize = 400;

/// python `_BOOT_CYS` — cys 기동·역할 서브커맨드. 조회·발신 계열(`send`·`status`)은 **일부러
/// 뺐다**: 그 문장을 임무로 주는 경우가 실제로 있다.
fn boot_cys_rx() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(
            r"^cys\s+(launch-agent|boot|boot-node|node-recover|new-surface|claim-role|cycle-agent|init-pack)(\s+\S+)*$",
        )
        .expect("BOOT_CYS 컴파일 실패")
    })
}

/// python `_BOOT_CLI` — 에이전트 CLI 직접 기동(실행 파일명 + 플래그만).
fn boot_cli_rx() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(r"^(claude|claude-[A-Za-z0-9_-]+|agy|codex|gemini)(\s+-{1,2}[A-Za-z0-9_=.,/:-]+)*$")
            .expect("BOOT_CLI 컴파일 실패")
    })
}

/// 한글이 한 글자라도 있는가(python `_HANGUL` = `[가-힣ㄱ-ㆎ]`).
fn has_hangul(s: &str) -> bool {
    s.chars()
        .any(|c| ('\u{AC00}'..='\u{D7A3}').contains(&c) || ('\u{3131}'..='\u{318E}').contains(&c))
}

/// 층0-c 판정 — 이 프롬프트가 **기동 명령문**인가(임무가 아니다).
///
/// 2026-09-03 21:19:23 실사고: `cys launch-agent --role master --agent claude` 가
/// `source=prompt` 로 임무 대장에 등록돼 **자율 착수 게이트가 열렸다**. 세 층 어디에도 안 걸린다
/// — 원장에 없고(층1), `[` 로 시작하지 않고(층2), XML 마커가 없어 잔여문이 45자로 살아남는다(층0).
///
/// 네 조건을 **전부** 만족할 때만 접는다(좁게 잡는다 — 애매하면 통과):
///   ① 공백을 접으면 한 줄 ② **한글 0자** ③ [`MISSION_MAX_CHARS`] 이하
///   ④ 전체가 cys 서브커맨드(A) 또는 에이전트 CLI 기동(B) 형태와 **정확히 일치**
///
/// ②가 사실상의 안전판이다 — 이 팩의 오너 지시문에는 한글이 있다. 그래서 명령이 **인용된**
/// 문장은 전부 통과한다("cys boot 실행하고 결과 보고해"). **받아들이는 대가**: 오너가 영어로
/// `cys boot` 한 줄만 쳐서 그것을 임무로 삼으려 하면 접힌다. 그때는 한 마디 덧붙이면 되고,
/// 반대 방향 실패(기계 문장이 임무가 되어 자율 착수가 열리는 것)는 그렇게 값싸게 되돌릴 수 없다.
pub fn boot_command_origin(prompt: &str) -> Option<String> {
    let p = collapse_ws(prompt.trim());
    if p.is_empty() || p.chars().count() > MISSION_MAX_CHARS {
        return None;
    }
    if has_hangul(&p) {
        return None; // ② 한글이 있으면 오너 문장으로 본다
    }
    if prompt.trim().contains('\n') {
        return None; // ① 원문이 여러 줄이면 대상 아님
    }
    let n = p.chars().count();
    let head: String = p.chars().take(80).collect();
    if boot_cys_rx().is_match(&p) {
        return Some(format!(
            "기동 명령문(cys 서브커맨드 전체 일치 · 한글 0자 · {n}자) — \
             노드 기동 명령이지 이 세션의 임무가 아니다: {head:?}"
        ));
    }
    if boot_cli_rx().is_match(&p) {
        return Some(format!(
            "기동 명령문(에이전트 CLI 기동 전체 일치 · 한글 0자 · {n}자) — \
             노드 기동 명령이지 이 세션의 임무가 아니다: {head:?}"
        ));
    }
    None
}

// ══════════════════════════════════════════════════════════════════════════════
// 층1 — 배달 원장 대조 (**정답 층**)
// ══════════════════════════════════════════════════════════════════════════════
//
// 층0·층2 가 프롬프트 **문자열**을 보는 반면 이 층은 "데몬이 이 pane 에 무엇을 주입했는가"라는
// **사실의 기록**을 본다. 그래서 문안 규약을 우회하는 push(라벨 없는 send)도 잡는다.
//
// 폴드 규칙 7종(python `machine_origin` 의 판정 순서 그대로 — 순서가 곧 우선순위다):
//   ⓐ 전문 일치           — 정규화 해시가 원장에 있다
//   ⓑ 창 밖 전문 일치      — 접되 `delivery_out_of_window` 를 남긴다(R5 ①)
//   ⓒ 조각 연접(concat)    — 일치 구간의 합집합이 프롬프트를 남김없이 덮는다(길이 하한 없음)
//   ⓓ 부분 포함(substr)    — 기계 배달로만 설명되는 연속 구간이 24자 이상(R6 ③ 자격 요건)
//   ⓔ 역포함              — 프롬프트가 더 긴 배달의 한 조각(구 데몬 스큐 · 평문 근거)
//   ⓕ 탐색 예산 소진       — 못 본 구간이 있으면 접는다(R6 ② fail-closed)
//   ⓖ 조각 상한 초과 직후   — 원장이 그 배달을 다 담지 못했다(R7 · 창 600s)
//
// ★비대칭은 여기서도 같다: 애매하면 **접는다**(거짓 양성=자율주행 폭주=치명 / 거짓 음성=한 번
//   더 묻기=경미). 단 원장 **부재**만은 예외다 — 부트스트랩 불가침이라 접으면 오너가 임무를
//   영영 줄 수 없다(층2 라벨로만 판별하고 `ledger_absent` 로 고지한다).


/// 원장 레코드 스키마 버전 — 생산자 `cysd::delivery::LEDGER_SCHEMA` 와 **같은 값**이어야 한다.
/// 갈리는 순간 신 스키마 배달이 통째로 `ledger_schema_skew` 로 떨어져 층1 에서 보이지 않는다.
pub const SCHEMA_VERSION: u64 = 1;
/// 전문 레코드 미리보기 길이(문자) — 부분 일치 탐색의 평문 앵커.
pub const PREVIEW_CHARS: usize = 64;
/// 조각 레코드 미리보기 길이(문자).
pub const PART_PREVIEW_CHARS: usize = 24;
/// 부분 포함(substr) 성립 하한(문자). 짧은 기계 문장("네"·"확인")이 오너 프롬프트에 우연히
/// 섞여 임무가 영영 안 열리는 거짓 음성 폭발을 막는다.
pub const DELIVERY_PART_MIN_CHARS: usize = 24;
/// 역포함(ⓔ) 성립 하한(문자). 같은 취지다.
pub const DELIVERY_WITHIN_MIN_CHARS: usize = 24;
/// 부분 일치 탐색의 **전역** 해시 확증 예산. 소진 = 못 본 구간이 있다 = fail-closed.
pub const DELIVERY_SPAN_OCC_BUDGET: usize = 100_000;
/// 조각 상한 초과 배달을 근거로 접는 창(초).
pub const DELIVERY_CAPPED_FOLD_S: f64 = 600.0;
/// 원장 스캔 줄 수 상한 — **성능이 아니라 안전** 상한이다(도달 = 조작 정황 → 판독 불가).
pub const DELIVERY_SCAN_LINES: usize = 250_000;
/// 원장 1파일 판독 크기 상한(바이트).
pub const LEDGER_MAX_READ_BYTES: u64 = 16 * 1024 * 1024;
/// 배달 창 기본값(초) 및 env 오버라이드 경계.
pub const DELIVERY_WINDOW_S_DEFAULT: i64 = 21_600;
pub const DELIVERY_WINDOW_MIN_S: i64 = 600;
pub const DELIVERY_WINDOW_MAX_S: i64 = 604_800;
/// 창 오버라이드 env 키.
pub const ENV_DELIVERY_WINDOW_S: &str = "CYS_DELIVERY_WINDOW_S";

/// python `str.splitlines()` 와 **같은 경계**(11종). Rust `str::lines()` 는 `\n`·`\r\n` 만
/// 자르므로 U+2028·U+0085·`\x0b`·`\x1c`–`\x1e` 가 섞인 원장에서 **줄 수가 달라진다** —
/// 줄 수는 스캔 상한(`DELIVERY_SCAN_LINES`) 판정 입력이라 조용히 갈리면 안 된다.
fn is_py_linebreak(c: char) -> bool {
    matches!(
        c,
        '\n' | '\r' | '\u{B}' | '\u{C}' | '\u{1C}' | '\u{1D}' | '\u{1E}' | '\u{85}' | '\u{2028}'
            | '\u{2029}'
    )
}

/// python `str.splitlines()` 미러. `""` → `[]`, `"a\n"` → `["a"]`, `"\n"` → `[""]`.
pub fn py_splitlines(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut it = s.char_indices().peekable();
    while let Some((i, c)) = it.next() {
        if !is_py_linebreak(c) {
            continue;
        }
        out.push(&s[start..i]);
        let mut end = i + c.len_utf8();
        if c == '\r' {
            if let Some(&(_, '\n')) = it.peek() {
                it.next();
                end += 1;
            }
        }
        start = end;
    }
    if start < s.len() {
        out.push(&s[start..]);
    }
    out
}

/// python `str.strip()` 미러 — 공백 술어는 [`is_py_space`](29종)다.
/// [`normalize`] 의 White_Space(25종)와 **다른 술어**이며 섞으면 조용히 갈린다.
fn py_strip(s: &str) -> &str {
    s.trim_matches(is_py_space)
}

/// 원장 판독 3상. **'부재'와 '판독 불가'를 절대 융합하지 않는다** — 부재는 정상(데몬 미기동)
/// 이고 판독 불가는 손상이다. 융합하면 손상이 정상으로 은폐돼 게이트가 열린다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LedgerStatus {
    /// 아직 기계 배달이 없었다 = 정상(부트스트랩 불가침).
    Absent,
    Ok,
    /// 손상·권한·디렉터리·상한 초과 — **fail-closed 대상**.
    Unreadable,}

impl LedgerStatus {
    /// 와이어 어휘 → 상태. **미지 값은 판독 불가(fail-closed)** 로 접는다.
    ///
    /// 왜 `Absent` 나 `Ok` 로 접지 않는가: 이 값은 "무슨 근거로 판정했는가"이고, 근거를
    /// **해석하지 못한 채** 정상으로 접으면 판정 근거가 없는 임무가 게이트를 연다. 모르는
    /// 어휘는 계약 스큐(구·신 데몬 혼재)이지 정상 상태가 아니다.
    pub fn from_wire(s: &str) -> LedgerStatus {
        match s {
            "absent" => LedgerStatus::Absent,
            "ok" => LedgerStatus::Ok,
            _ => LedgerStatus::Unreadable,
        }
    }

    /// python 상수(`LEDGER_ABSENT`·`LEDGER_OK`·`LEDGER_UNREADABLE`)와 **같은 문자열**.
    /// 대장·verdict JSON 이 이 어휘를 그대로 싣는다 — 갈리면 소비자가 상태를 못 읽는다.
    pub fn as_str(self) -> &'static str {
        match self {
            LedgerStatus::Absent => "absent",
            LedgerStatus::Ok => "ok",
            LedgerStatus::Unreadable => "unreadable",
        }
    }
}

/// 원장 파일 하나의 상태. 파일 I/O 는 **호출자 몫**이고 이 모듈은 순수하다
/// (같은 판정을 훅 CLI·데몬·검체가 서로 다른 자료원으로 먹인다).
#[derive(Debug, Clone)]
pub enum LedgerFile {
    /// 파일이 없다.
    Missing,
    /// 판독한 내용.
    Content(String),
    /// 자리에 있는데 읽을 수 없다 — python `_read_ledger_lines` 의 err 문자열을 그대로 담는다
    /// (본 파일이면 `원장 {why}`, 회전 세대면 `회전 원장(.1) {why}` 로 나간다).
    Unreadable(String),
}

/// 원장 레코드 요약(python `read_delivery` 가 돌려주는 dict 미러).
#[derive(Debug, Clone)]
pub struct DeliveryMeta {
    pub ts: f64,
    pub age: f64,
    /// 창 밖인가 — **버리는 기준이 아니라** 이상징후 발행 경계다(R5 ①).
    pub stale: bool,
    /// 정규화 본문의 문자 수. `None` = 필드 부재이거나 python `isinstance(v,int)` 불성립
    /// (float `3.0` 은 int 가 아니다 — 그 레코드는 부분 일치 탐색에서 빠진다).
    pub chars: Option<i64>,
    /// 정규화 본문의 앞 [`PREVIEW_CHARS`] 자. 부분 일치 탐색의 평문 앵커다.
    pub preview: Option<String>,
    pub origin: Option<String>,
    /// 제출 단위 수. `Some(1)` 이면 개행이 없어 쪼개질 수 없다 → 역포함 대상이 아니다.
    pub units: Option<i64>,
    /// 조각 상한 초과분(원값 보존 — 해석은 [`capped_count`] 가 관대하게 한다).
    pub parts_capped: Option<serde_json::Value>,
    /// 감사용(판정 불참).
    pub part: Option<i64>,
    pub parent: Option<String>,
}

/// sha256 → 레코드 요약. **삽입 순서를 보존**한다(python dict 와 같게).
///
/// ★순서가 판정에 영향을 준다(함정): [`delivery_spans`] 의 탐색 예산은 **전역**이라, 어느
/// 레코드를 먼저 훑느냐가 예산 소진 지점을 바꾸고 그것이 `capped`(=접는다) 여부를 바꾼다.
/// `HashMap` 하나로 갈음하면 두 구현이 같은 입력에 다른 답을 낸다.
#[derive(Debug, Clone, Default)]
pub struct DeliveryMap {
    entries: Vec<(String, DeliveryMeta)>,
    index: HashMap<String, usize>,
}

impl DeliveryMap {
    pub fn get(&self, sha: &str) -> Option<&DeliveryMeta> {
        self.index.get(sha).map(|&i| &self.entries[i].1)
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub fn iter(&self) -> impl Iterator<Item = (&str, &DeliveryMeta)> {
        self.entries.iter().map(|(k, v)| (k.as_str(), v))
    }
    /// python `out[sha] = meta` 와 같은 의미 — 기존 키를 덮어도 **자리는 그대로**다.
    fn put(&mut self, sha: String, meta: DeliveryMeta) {
        match self.index.get(&sha) {
            Some(&i) => self.entries[i].1 = meta,
            None => {
                self.index.insert(sha.clone(), self.entries.len());
                self.entries.push((sha, meta));
            }
        }
    }
}

/// [`read_delivery`] 의 입력. 인자가 많아 구조체로 묶는다(호출부에서 순서를 헷갈리면
/// surface 와 경로가 바뀌어 **판정이 통째로 무력화**된다 — 이름으로 못박는다).
pub struct LedgerInput<'a> {
    /// 본 원장 파일.
    pub main: &'a LedgerFile,
    /// 회전 세대(`.jsonl.1`). 데몬은 1세대만 보존한다.
    pub rotated: &'a LedgerFile,
    /// 데몬 인스턴스 표식(`delivery_epoch`). 원장이 **부재인데** 이것이 있으면 삭제·손상이다.
    pub daemon_epoch: Option<&'a str>,
    /// 이 pane 의 surface 식별자(`CYS_SURFACE_ID` 와 같은 표기 = 정수 문자열).
    pub me: &'a str,
    pub now: f64,
    /// 창(초). 표준값은 [`delivery_window_s`] 다 — 호출자가 넘겨 검체가 창을 고정할 수 있게 한다.
    pub window_s: f64,
    /// 진단 문구에 실을 경로(판정 입력 아님).
    pub path_label: &'a str,
}

/// [`read_delivery`] 의 결과.
pub struct DeliveryRead {
    pub map: DeliveryMap,
    pub status: LedgerStatus,
    /// 사람이 읽는 판독 요약(판정 입력 아님).
    pub detail: String,
    /// 관측된 이상징후 `(코드, 사유)` — 발행 순서 보존.
    pub anomalies: Vec<(String, String)>,
}

/// python `isinstance(v, int)` 미러 — **bool 은 int 이고 float 는 아니다**.
///
/// ★이 술어가 따로 필요한 이유(실측 함정): `isinstance(True, int)` 는 파이썬에서 참이다.
/// 그래서 `{"chars": true}` 인 레코드를 python 은 길이 1로 취급한다. Rust 에서 `as_i64()` 만
/// 쓰면 그 레코드는 조용히 건너뛰어져 **두 구현이 다른 답**을 낸다.
fn py_int(v: Option<&serde_json::Value>) -> Option<i64> {
    match v {
        Some(serde_json::Value::Bool(b)) => Some(i64::from(*b)),
        Some(serde_json::Value::Number(n)) if !n.is_f64() => n.as_i64(),
        _ => None,
    }
}

/// python `float(x)` 미러(숫자·bool·수치 문자열까지). 실패는 `None` = 그 줄은 손상 계수.
fn py_float(v: &serde_json::Value) -> Option<f64> {
    match v {
        serde_json::Value::Number(n) => n.as_f64(),
        serde_json::Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        serde_json::Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
}

/// `rec.get("v") != SCHEMA_VERSION` 의 python 의미 — `1`·`1.0`·`true` 가 **모두** 1과 같다.
fn v_is_schema(v: Option<&serde_json::Value>) -> bool {
    match v {
        Some(serde_json::Value::Number(n)) => n.as_f64() == Some(SCHEMA_VERSION as f64),
        Some(serde_json::Value::Bool(b)) => u64::from(*b) == SCHEMA_VERSION,
        _ => false,
    }
}

/// 조각 상한 초과 행 수(0 = 초과 없음) — python `_capped_count` 미러.
///
/// **관대하게 접는 쪽으로** 읽는다: 필드가 붙어 있다는 사실 자체가 "이 배달을 원장이 다 담지
/// 못했다"는 생산자의 자백이다. 값이 정수가 아니거나 0·음수여도 최소 1로 센다 — 숫자만
/// 망가뜨려 fail-open 시키는 경로를 만들지 않는다. **필드 부재만 0** 이다.
pub fn capped_count(meta: &DeliveryMeta) -> i64 {
    use serde_json::Value;
    match &meta.parts_capped {
        None | Some(Value::Null) | Some(Value::Bool(false)) => 0,
        Some(Value::Bool(true)) => 1,
        Some(Value::Number(n)) => {
            let v = if n.is_f64() { n.as_f64().map(|f| f as i64) } else { n.as_i64() };
            match v {
                Some(x) if x > 0 => x,
                _ => 1,
            }
        }
        Some(Value::String(s)) => match s.trim().parse::<i64>() {
            Ok(x) if x > 0 => x,
            _ => 1,
        },
        Some(_) => 1,
    }
}

/// env 정수 오버라이드를 `[lo, hi]` 로 강제한다(python `_env_bounded` 미러).
/// 반환 `(적용값, 이상징후)`. 미설정·빈 값·0 이하는 오버라이드 없음으로 취급한다(이상 아님).
/// 하한 미만은 **절단이 아니라 거부**다 — 짧은 창은 층1 무력화와 사실상 동치라 위험 방향이다.
fn env_bounded(raw: Option<&str>, name: &str, default: i64, lo: i64, hi: i64)
    -> (i64, Option<(String, String)>)
{
    let raw = raw.unwrap_or("").trim().to_string();
    if raw.is_empty() {
        return (default, None);
    }
    let Ok(v) = raw.parse::<i64>() else {
        let head: String = raw.chars().take(40).collect();
        return (
            default,
            Some((
                "env_not_int".to_string(),
                format!("{name}={head:?} 이 정수가 아니다 — 기본값 {default} 적용"),
            )),
        );
    };
    if v <= 0 {
        return (default, None);
    }
    if v < lo {
        return (
            default,
            Some((
                "env_below_floor".to_string(),
                format!(
                    "{name}={v} 이 하한 {lo} 미만이라 **무시**하고 기본값 {default} 적용 \
                     (짧은 값은 게이트를 여는 방향이다)"
                ),
            )),
        );
    }
    if v > hi {
        return (
            hi,
            Some((
                "env_above_cap".to_string(),
                format!("{name}={v} 가 상한 {hi} 초과라 상한으로 절단"),
            )),
        );
    }
    (v, None)
}

/// 배달 창(초) + 그 산출에서 나온 이상징후. python 은 import 시점에 1회 계산해 모듈 상수로
/// 들고 있다 — 여기서도 [`std::sync::OnceLock`] 으로 **1회만** 관측한다(같은 프로세스 안에서
/// 창이 도중에 바뀌면 판정이 흔들린다).
fn delivery_window() -> &'static (f64, Option<(String, String)>) {
    static W: OnceLock<(f64, Option<(String, String)>)> = OnceLock::new();
    W.get_or_init(|| {
        let raw = std::env::var(ENV_DELIVERY_WINDOW_S).ok();
        let (v, a) = env_bounded(
            raw.as_deref(),
            ENV_DELIVERY_WINDOW_S,
            DELIVERY_WINDOW_S_DEFAULT,
            DELIVERY_WINDOW_MIN_S,
            DELIVERY_WINDOW_MAX_S,
        );
        (v as f64, a)
    })
}

/// 이 프로세스의 배달 창(초). [`LedgerInput::window_s`] 의 표준값이다.
pub fn delivery_window_s() -> f64 {
    delivery_window().0
}

/// 창 오버라이드에서 나온 이상징후(있으면 1건). 기동 시점에 확정되며 매 판독마다 재관측되지
/// 않는다 — 소비자가 [`DeliveryRead::anomalies`] 앞에 붙여 함께 보고한다.
pub fn env_anomalies() -> Vec<(String, String)> {
    delivery_window().1.iter().cloned().collect()
}

/// 이상징후 문구 전용 시각 표기(**판정 입력 아님**).
///
/// ★정직 고지 — python `_fmt_ts` 는 현지시각 문자열까지 붙인다(`%Y-%m-%d %H:%M:%S(epoch=…)`).
/// 여기서는 `epoch=` 만 낸다. 판정에 쓰이지 않는 진단 문구이고, 순수 판정 코어에 시간대 변환을
/// 들이면 같은 입력이 로캘·TZ 에 따라 다른 문자열을 내 검체가 흔들린다. 두 구현의 **판정**은
/// 이 차이와 무관하다.
fn fmt_ts(epoch: Option<f64>) -> String {
    match epoch {
        None => "(없음)".to_string(),
        Some(e) => format!("(epoch={e:.0})"),
    }
}

/// 이 pane 앞으로 온 배달 전량을 판독한다(python `read_delivery` 미러 · **순수**).
///
/// 파일 I/O 는 호출자가 [`LedgerFile`] 로 환원해 넘긴다 — 같은 판정을 데몬·훅 CLI·검체가 서로
/// 다른 자료원으로 먹일 수 있어야 "원장을 못 읽는 상황"이 검체에서 재현된다.
///
/// ## 이 함수가 **하지 않는** 것 셋 — 전부 종전 fail-open 의 봉합 지점이다
///   ① 줄 수로 미리 자르지 않는다. 종전엔 tail 절단이 surface 필터보다 **앞**이라, 남의 pane
///      레코드를 4000줄 밀어 넣으면 내 배달이 '창 밖'이 아니라 **'스캔 밖'** 으로 사라져 층1 이
///      조용히 실패했다(R4 ①). 상한을 넘으면 자르지 않고 **판독 불가**로 접는다.
///   ② 창 밖 레코드를 버리지 않는다. `stale` 로 남겨 판별에 쓴다(R5 ①) — 창은 버리는 기준이
///      아니라 '창 밖 일치'라는 이상징후를 발행하는 경계로 격하됐다.
///   ③ 오래된 `ts` 를 만나도 조기 종료하지 않는다. 그 최적화를 쓰면 아주 오래된 `ts_epoch`
///      1줄을 append 하는 것만으로 스캔을 끊어 층1 을 통째로 무력화할 수 있다.
///
/// ## '부재'와 '판독 불가'를 절대 융합하지 않는다
/// 부재가 정상인 것은 **기동 표식을 쓰는 데몬이 돌지 않았을 때뿐**이다(구 데몬·미기동).
/// 그 데몬이 돌았다면(`daemon_epoch` 존재) 표식이 원장에 있어야 하므로 부재는 삭제·손상이다.
/// 같은 이유로 **0바이트도 손상**이다 — 정상 원장은 기동 표식 1줄 때문에 절대 0바이트가 아니다.
pub fn read_delivery(inp: &LedgerInput) -> DeliveryRead {
    let blank = |status: LedgerStatus, detail: String, anomalies: Vec<(String, String)>| DeliveryRead {
        map: DeliveryMap::default(),
        status,
        detail,
        anomalies,
    };
    let path = inp.path_label;

    // ── 본 파일 상태 3분기 ──────────────────────────────────────────────────
    let main_text = match inp.main {
        LedgerFile::Unreadable(why) => {
            return blank(LedgerStatus::Unreadable, format!("원장 {why}"), Vec::new())
        }
        LedgerFile::Missing => {
            if let Some(e) = inp.daemon_epoch {
                return blank(
                    LedgerStatus::Unreadable,
                    format!(
                        "원장이 없는데 데몬 인스턴스 표식은 있다(daemon_epoch={e:?}) — 기동 시 \
                         기록되는 표식조차 없으므로 삭제·손상으로 본다(fail-closed): {path}"
                    ),
                    Vec::new(),
                );
            }
            // ★부재는 판정으로는 정상이지만 **판별 근거가 없다는 사실**은 드러나야 한다.
            //   이 상태에서 판별은 층2(라벨)뿐이고 무라벨 push 는 그대로 오너 임무가 된다 —
            //   부트스트랩 불가침 때문에 fail-closed 로 못 바꾸는 잔여 위험이라, 차단이 아니라
            //   **고지**로 다룬다(흔적 없는 열림을 만들지 않는다).
            return blank(
                LedgerStatus::Absent,
                format!("원장 없음(표식도 없음 — 데몬 미기동/구버전): {path}"),
                vec![(
                    "ledger_absent".to_string(),
                    format!(
                        "배달 원장 부재 — 층1(원장 대조) 근거 없이 층2(라벨)로만 판별한다. \
                         무라벨 기계 push 는 이 상태에서 오너 임무로 기록될 수 있다: {path}"
                    ),
                )],
            );
        }
        LedgerFile::Content(s) => s,
    };

    let mut lines: Vec<&str> = py_splitlines(main_text);
    if lines.is_empty() {
        return blank(
            LedgerStatus::Unreadable,
            format!(
                "원장이 존재하는데 0바이트다(기동 표식조차 없다) — 절단·손상으로 본다\
                 (fail-closed): {path}"
            ),
            Vec::new(),
        );
    }

    // ── 회전 세대(.1)까지 판독 ──────────────────────────────────────────────
    //    회전 직후에는 최근 배달이 `.1` 에 있다. 종전엔 이걸 아예 안 읽어, 원장을 상한 넘게
    //    밀어 회전만 시키면 최근 배달이 판별에서 사라졌다(R4 ①의 짝).
    let mut late: Vec<(String, String)> = Vec::new();
    let (mut generations, mut rotated_lines) = (1usize, 0usize);
    if let LedgerFile::Unreadable(why) = inp.rotated {
        return blank(LedgerStatus::Unreadable, format!("회전 원장(.1) {why}"), Vec::new());
    }
    if let LedgerFile::Content(r) = inp.rotated {
        let rlines = py_splitlines(r);
        generations = 2;
        rotated_lines = rlines.len();
        let mut all = rlines; // 과거(.1) → 현재 순서로 이어 붙인다
        all.extend(lines);
        lines = all;
    }

    if lines.len() > DELIVERY_SCAN_LINES {
        return blank(
            LedgerStatus::Unreadable,
            format!(
                "원장 줄 수({})가 스캔 상한 {DELIVERY_SCAN_LINES} 초과 — 데몬 회전 상한으로는 \
                 만들어질 수 없다(조작 정황). 절단하지 않고 판독 불가로 접는다: {path}",
                lines.len()
            ),
            Vec::new(),
        );
    }

    // ── 레코드 스캔 ────────────────────────────────────────────────────────
    let (mut bad, mut good) = (0usize, 0usize);
    let mut oldest_ts: Option<f64> = None;
    let mut skew: Vec<(String, usize)> = Vec::new(); // 관측 순서 보존
    let mut out = DeliveryMap::default();
    for ln in &lines {
        let ln = py_strip(ln);
        if ln.is_empty() {
            continue;
        }
        let Ok(rec) = serde_json::from_str::<serde_json::Value>(ln) else {
            bad += 1;
            continue;
        };
        let Some(obj) = rec.as_object() else {
            bad += 1; // python 은 `rec["sha256"]` 에서 TypeError → 같은 계수
            continue;
        };
        // `sha256` **부재**는 KeyError(손상)이고, null 은 값이다(키가 된다) — 구분한다.
        let Some(sha_v) = obj.get("sha256") else {
            bad += 1;
            continue;
        };
        let sha = match sha_v {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(), // 문자열이 아니면 어떤 해시와도 같을 수 없다
        };
        let Some(ts) = obj.get("ts_epoch").and_then(py_float) else {
            bad += 1;
            continue;
        };
        if !v_is_schema(obj.get("v")) {
            // ★스키마 혼재는 부분쓰기·잡음과 **다른 코드로** 발행한다. 구 스키마가 한 줄이라도
            //   남아 있으면 `good>0` 이라 상태는 OK 인데 신규 배달만 통째로 안 보이는 상태가
            //   되고, 무라벨 기계 push 는 층2 도 통과해 오너 임무가 된다.
            //   ★채택하지 않은 대안: '스큐 1건이라도 있으면 무조건 판독 불가'. 그러면 같은 UID 가
            //     `{"v":999,…}` 한 줄을 append 하는 것만으로 오너가 임무를 영영 줄 수 없게 된다.
            //     혼재는 탐지로, 전량 스큐(good==0)는 차단으로 — 비대칭 원칙에 맞는 경계가 여기다.
            bad += 1;
            let key = match obj.get("v") {
                Some(serde_json::Value::String(s)) => format!("{s:?}"),
                Some(v) => v.to_string(),
                None => "None".to_string(),
            };
            match skew.iter_mut().find(|(k, _)| *k == key) {
                Some(e) => e.1 += 1,
                None => skew.push((key, 1)),
            }
            continue;
        }
        // ★`good` 은 '이 파일을 해석할 수 있는가'의 척도다 — surface·창 필터링 **전**에 센다.
        //   여기서 세지 않고 내 pane 레코드로 손상을 판정하면, "남의 pane 배달만 있고 깨진 줄
        //   1개가 섞인" 정상 상태가 '손상'으로 접혀 오너 임무가 영영 안 열린다.
        good += 1;
        if oldest_ts.is_none_or(|o| ts < o) {
            oldest_ts = Some(ts);
        }
        let surf_is_me = match obj.get("surface") {
            None => inp.me.is_empty(), // python 기본값 "" 과의 비교
            Some(serde_json::Value::String(s)) => s == inp.me,
            Some(_) => false,          // 문자열이 아니면 python 에서도 != me 다
        };
        if !surf_is_me {
            continue; // 남의 pane 배달 — 내 판별에 들지 않는다(surface 결박)
        }
        if let Some(prev) = out.get(&sha) {
            if prev.ts >= ts {
                continue; // 같은 문장의 더 최근 배달을 이미 잡았다
            }
        }
        out.put(
            sha,
            DeliveryMeta {
                ts,
                age: inp.now - ts,
                stale: (inp.now - ts) > inp.window_s,
                chars: py_int(obj.get("chars")),
                preview: obj.get("preview").and_then(|v| v.as_str()).map(str::to_string),
                origin: obj.get("origin").and_then(|v| v.as_str()).map(str::to_string),
                units: py_int(obj.get("units")),
                parts_capped: obj.get("parts_capped").cloned(),
                part: py_int(obj.get("part")),
                parent: obj.get("parent").and_then(|v| v.as_str()).map(str::to_string),
            },
        );
    }
    let stale_n = out.iter().filter(|(_, m)| m.stale).count();

    // ★R7: 조각 상한 초과는 **원장이 그 배달을 다 담지 못했다**는 자백이다. 창 안이든 밖이든
    //   사실은 사실이므로 관측 즉시 고지한다(접는 것은 창 안에서만 — `capped_recent`).
    let capped: Vec<(&str, &DeliveryMeta)> =
        out.iter().filter(|(_, m)| capped_count(m) > 0).collect();
    if !capped.is_empty() {
        // ★python `max(…, key=…)` 는 **첫 번째** 최대를 돌려준다(Rust `max_by_key` 는 마지막).
        let mut worst = capped[0];
        for &c in capped.iter().skip(1) {
            if capped_count(c.1) > capped_count(worst.1) {
                worst = c;
            }
        }
        late.push((
            "delivery_parts_capped".to_string(),
            format!(
                "이 pane 앞으로 온 배달 {}건이 조각 상한을 넘겼다(최대 {}행 누락 · 배달시각 {} · \
                 sha256={}…). 넘긴 행은 원장에 없으므로 그 행이 단독 제출되면 층1 이 전건 미스이고, \
                 무라벨이면 오너 임무로 기록될 수 있다. 창({DELIVERY_CAPPED_FOLD_S:.0}s) 안이면 \
                 미매치 프롬프트를 접는다(fail-closed). 생산자 상한은 delivery.rs::MAX_PARTS 다.",
                capped.len(),
                capped_count(worst.1),
                fmt_ts(Some(worst.1.ts)),
                worst.0.chars().take(12).collect::<String>()
            ),
        ));
    }

    if bad > 0 && good == 0 {
        // 내용은 있는데 해석 가능한 레코드가 **하나도** 없다 = 손상(fail-closed).
        // ★python 과 같이 여기서 `late`(위 고지)를 **버린다** — 판독 불가면 그 관측 자체가
        //   근거를 잃는다(해석 가능한 레코드가 0인 파일에서 나온 통계다).
        return blank(
            LedgerStatus::Unreadable,
            format!("원장에 판독 가능한 레코드가 없다(손상 줄 {bad}): {path}"),
            Vec::new(),
        );
    }
    if generations > 1 {
        late.push((
            "ledger_rotated".to_string(),
            format!(
                "원장이 회전했다 — 판독 세대 {generations}개(본 {}줄 + {path}.1 {rotated_lines}줄). \
                 데몬은 1세대만 보존하므로 판독 가능한 최고(最古) 배달 {} 이전 구간은 소실돼 층1 \
                 대조가 불가능하다(그 구간의 기계 push 는 층2 라벨로만 걸린다).",
                lines.len() - rotated_lines,
                fmt_ts(oldest_ts)
            ),
        ));
    }
    if !skew.is_empty() {
        let mut sorted = skew.clone();
        sorted.sort_by(|a, b| a.0.cmp(&b.0)); // python `sorted(…, key=repr)` 미러
        late.push((
            "ledger_schema_skew".to_string(),
            format!(
                "원장에 이 판독자(v={SCHEMA_VERSION})가 모르는 스키마 레코드가 섞였다 — {}. 해당 \
                 배달은 층1 대조에서 통째로 보이지 않으므로, 무라벨이면 오너 임무로 기록될 수 \
                 있다(층2 폴백). 생산자(cysd delivery.rs::LEDGER_SCHEMA)와 판독자 버전을 맞춰라",
                sorted
                    .iter()
                    .map(|(k, n)| format!("v={k} {n}건"))
                    .collect::<Vec<_>>()
                    .join(" · ")
            ),
        ));
    }
    let skew_n: usize = skew.iter().map(|(_, n)| *n).sum();
    if bad > skew_n {
        late.push((
            "ledger_bad_lines".to_string(),
            format!(
                "원장에 해석 불가 줄 {}개(부분쓰기·조작 정황 · 스키마 스큐는 별도 코드)",
                bad - skew_n
            ),
        ));
    }

    let detail = format!(
        "원장 해석 {good}건 중 이 pane {}건(창 밖 {stale_n} · 손상 줄 {bad}): {path}",
        out.len()
    );
    DeliveryRead { map: out, status: LedgerStatus::Ok, detail, anomalies: late }
}

/// 문자 인덱스 ↔ 바이트 오프셋 환산표.
///
/// ★이 표가 필요한 이유(RESUME 함정 ⓐ): python 의 `str.find`·슬라이스는 **문자 인덱스**이고
/// Rust `str::find` 는 **바이트 오프셋**이다. 그대로 섞으면 한글이 한 글자라도 섞이는 순간
/// 구간 계산이 조용히 갈린다 — 그리고 그 구간이 곧 substr/concat 판정의 입력이다.
///
/// `c2b` 한 벌만 들고 역방향은 이진 탐색으로 푼다(표 두 벌 = 문자당 16바이트). `find` 가
/// 돌려주는 오프셋은 언제나 문자 경계이므로 탐색은 반드시 `Ok` 로 끝난다.
struct CharIndex<'a> {
    s: &'a str,
    c2b: Vec<usize>,
}

impl<'a> CharIndex<'a> {
    fn new(s: &'a str) -> Self {
        let mut c2b: Vec<usize> = s.char_indices().map(|(i, _)| i).collect();
        c2b.push(s.len());
        Self { s, c2b }
    }
    fn as_str(&self) -> &'a str {
        self.s
    }
    /// 문자 수.
    fn len(&self) -> usize {
        self.c2b.len() - 1
    }
    /// `[a, b)` 문자 구간(python `norm[a:b]`).
    fn slice(&self, a: usize, b: usize) -> &'a str {
        &self.s[self.c2b[a]..self.c2b[b]]
    }
    /// python `s.find(pat, from)` — 인자도 반환도 **문자** 인덱스다.
    fn find_from(&self, pat: &str, from: usize) -> Option<usize> {
        let sb = self.c2b[from];
        let rel = self.s[sb..].find(pat)?;
        match self.c2b.binary_search(&(sb + rel)) {
            Ok(i) => Some(i),
            Err(i) => Some(i), // 도달 불가(find 는 문자 경계만 돌려준다)
        }
    }
}

/// 프롬프트 안에서 원장 레코드와 **정확히 일치**하는 구간 1건.
#[derive(Debug, Clone)]
struct Span {
    a: usize,
    b: usize,
    sha: String,
}

/// 정규화 프롬프트 안에서 원장 레코드와 정확히 일치하는 구간 전부(python `_delivery_spans`).
///
/// 탐색 방식: `preview`(생산자가 넣는 **정규화 본문의 앞 [`PREVIEW_CHARS`] 자**)를 평문 앵커로
/// 찾은 뒤, 그 자리에서 `chars` 길이를 잘라 **sha256 으로 확증**한다. 앵커는 후보를 좁히는
/// 용도이고 판정은 언제나 해시다 — preview 만 같고 뒤가 다른 문장은 걸러진다.
///
///   · **완전성**: 앵커 출현을 하나도 빠뜨리지 않고(`start = i + 1` 로 겹침까지) 열거하므로 이
///     탐색은 슬라이딩 전수 스캔과 **결과가 같다**(레코드 본문은 반드시 자기 앵커로 시작한다).
///     종전의 레코드당 반복 상한은 그 완전성을 깨서 33회 이상 연접에 게이트를 열어 줬다(R6 ②).
///   · 비용: 해시 확증 횟수만 **전역 예산**으로 묶는다. 소진은 `capped` 로 호출자에게 넘기고
///     호출자가 fail-closed 로 접는다 — 불완전한 spans 로 판정을 계속하는 것 자체가 fail-open 이다.
///   · `chars`·`preview` 가 없는 레코드(구 스키마·수기)는 **건너뛴다**. 전문 해시 대조는 그대로
///     유효하므로 판별이 약해지는 방향이 아니다(부분 일치만 포기).
///   · 기동 표식(sentinel: `chars=0`·`preview=""`)은 여기서 자동 배제된다.
///
/// ★반복 순서가 판정을 바꾼다: 예산이 전역이라 어느 레코드를 먼저 훑느냐가 소진 지점을 바꾸고
///   그것이 `capped` 를 바꾼다. [`DeliveryMap`] 이 삽입 순서를 보존하는 이유가 이것이다.
fn delivery_spans(norm: &CharIndex, delivery: &DeliveryMap) -> (Vec<Span>, bool) {
    let mut spans: Vec<Span> = Vec::new();
    let mut capped = false;
    let n = norm.len();
    let mut budget = DELIVERY_SPAN_OCC_BUDGET;
    for (sha, meta) in delivery.iter() {
        let Some(chars) = meta.chars else { continue };
        if chars < 1 || chars as usize > n {
            continue;
        }
        let chars = chars as usize;
        let Some(prev) = meta.preview.as_deref().filter(|p| !p.is_empty()) else { continue };
        let mut start = 0usize;
        loop {
            let Some(i) = norm.find_from(prev, start) else { break };
            if budget == 0 {
                // 예산 소진 = **못 본 구간이 있다**. 여기서 끊고 호출자가 접게 한다.
                capped = true;
                break;
            }
            budget -= 1;
            let j = i + chars;
            if j <= n && digest_normalized(norm.slice(i, j)) == sha {
                spans.push(Span { a: i, b: j, sha: sha.to_string() });
            }
            start = i + 1;
        }
        if capped {
            break;
        }
    }
    (spans, capped)
}

/// 프롬프트가 기계 배달 조각으로 설명되는가(python `_composition`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Composition {
    /// 설명되지 않는다.
    None,
    /// ① **전량 커버** — 일치 구간의 합집합이 프롬프트를 남김없이 덮는다(사이에 공백만 허용).
    ///   프롬프트 전체가 기계 배달의 조합이라는 뜻이므로 **길이 하한 없이** 접는다.
    Concat(String),
    /// ② **부분 포함** — 기계 배달로만 설명되는 연속 구간이 [`DELIVERY_PART_MIN_CHARS`] 이상.
    Substr(String),
    /// ③ **판정 불가** — 탐색 예산이 소진돼 못 본 구간이 있다(fail-closed).
    Capped(String),
}

/// [`Composition`] 산출(python `_composition` 미러).
///
/// ## 왜 필요한가 (R5 관통 봉합 ③)
/// `cys send` 계열은 텍스트만 넣고 제출(Return)은 따로 한다. 그래서 큐 배달 "A" 가 pane 버퍼에
/// 남아 있는 동안 schedule 배달 "B" 가 들어오면 TUI 는 **"AB" 한 덩어리**를 제출한다. 전문 해시는
/// A 와도 B 와도 다르므로 층1 이 그냥 통과하고, 라벨이 없으면 층2 도 통과해 **오너 임무로 기록**
/// 된다(흔적 0).
///
/// ## 두 규칙의 강도가 다르다 — 섞지 말 것
/// ①이 본진이다(오너 개입이 0인 상황에서 프롬프트는 정의상 100% 기계 조합이다). ②는 ①이
/// 성립하지 **않을 때만** 도는 보조 규칙이고, 그래서 길이 하한과 자격 요건이 붙는다.
///
/// ★★R6 ③ — **같은 조각의 반복만으로는 하한을 채우지 못한다.** 종전엔 병합 구간이
/// "1자 배달 × 24회" 로도 24자를 채워 substr 이 성립했다. 그 결과 원장에 짧은 배달(예 "가")
/// 하나만 있으면 그 글자가 24자 이어지는 **오너의 평범한 문장**(구분선·강조 반복·같은 어절
/// 반복)이 통째로 차단됐다 — 이 규칙의 목적("짧은 기계 문장이 우연히 섞여 임무가 영영 안 열리는
/// 것을 막는다")과 정면으로 배치된다. 그래서 병합 구간이 **서로 다른 sha 2건 이상**으로 덮이거나
/// **단일 레코드 자체가 하한 이상**일 것을 요구한다.
///
/// ★가장 긴 구간이 자격 미달이어도 **다음 구간을 계속 본다** — 자격 있는 구간이 뒤에 있는데 첫
/// 후보에서 포기하면 그것도 fail-open 이다.
fn composition(norm: &CharIndex, delivery: &DeliveryMap) -> Composition {
    let (spans, capped) = delivery_spans(norm, delivery);
    let n = norm.len();
    if capped {
        return Composition::Capped(format!(
            "부분 일치 탐색이 전역 예산 {DELIVERY_SPAN_OCC_BUDGET}(해시 확증 횟수)에 도달해 \
             프롬프트의 일부 구간을 보지 못했다 — 못 본 구간이 기계 배달일 수 있으므로 판정을 \
             열지 않는다(fail-closed). 프롬프트 {n}자 · 원장 레코드 {}건",
            delivery.len()
        ));
    }
    if spans.is_empty() {
        return Composition::None;
    }
    let mut sorted = spans.clone();
    sorted.sort_by(|x, y| x.a.cmp(&y.a).then(x.b.cmp(&y.b)).then(x.sha.cmp(&y.sha)));
    // merged[k] = (시작, 끝, 기여 span…) — 기여를 들고 있어야 R6 ③ 자격 판정이 된다.
    let mut merged: Vec<(usize, usize, Vec<Span>)> = Vec::new();
    for sp in sorted {
        match merged.last_mut() {
            Some(last) if sp.a <= last.1 => {
                last.1 = last.1.max(sp.b);
                last.2.push(sp);
            }
            _ => merged.push((sp.a, sp.b, vec![sp])),
        }
    }
    let mut pos = 0usize;
    let mut gaps: Vec<&str> = Vec::new();
    for (a, b, _) in merged.iter() {
        if *a > pos {
            gaps.push(norm.slice(pos, *a));
        }
        pos = pos.max(*b);
    }
    if pos < n {
        gaps.push(norm.slice(pos, n));
    }
    let covered: usize = merged.iter().map(|(a, b, _)| b - a).sum();
    if gaps.iter().all(|g| py_strip(g).is_empty()) {
        return Composition::Concat(format!(
            "프롬프트 {n}자가 원장 배달 {}조각(구간 {}개)으로 남김없이 설명된다 — 두 기계 배달이 \
             한 프롬프트로 합쳐진 경우다(제출 전 버퍼 연접). 조각 미리보기: {}",
            spans.len(),
            merged.len(),
            merged
                .iter()
                .take(4)
                .map(|(a, b, _)| format!(
                    "{:?}",
                    norm.slice(*a, *b).chars().take(40).collect::<String>()
                ))
                .collect::<Vec<_>>()
                .join(" ⧉ ")
        ));
    }
    let mut best: Option<(usize, usize, usize, usize)> = None; // (a, b, distinct, single)
    for (a, b, contrib) in merged.iter() {
        if b - a < DELIVERY_PART_MIN_CHARS {
            continue;
        }
        let mut distinct: Vec<&str> = contrib.iter().map(|s| s.sha.as_str()).collect();
        distinct.sort_unstable();
        distinct.dedup();
        let single = contrib.iter().map(|s| s.b - s.a).max().unwrap_or(0);
        if distinct.len() < 2 && single < DELIVERY_PART_MIN_CHARS {
            continue; // 같은 짧은 조각의 반복만으로는 접지 않는다(R6 ③)
        }
        if best.is_none_or(|(ba, bb, _, _)| (b - a) > (bb - ba)) {
            best = Some((*a, *b, distinct.len(), single));
        }
    }
    if let Some((a, b, distinct, single)) = best {
        return Composition::Substr(format!(
            "프롬프트 {n}자 안에 기계 배달로만 설명되는 연속 구간 {}자가 있다(서로 다른 레코드 \
             {distinct}건 · 단일 레코드 최장 {single}자 · 총 덮인 {covered}자 · 위치 {a}) — 기계 \
             배달과 다른 문자열이 한 프롬프트로 합쳐졌다. 구간: {:?}",
            b - a,
            norm.slice(a, b).chars().take(60).collect::<String>()
        ));
    }
    Composition::None
}

/// 프롬프트가 **더 긴 배달 레코드의 한 조각**인가(python `_prompt_within_delivery` · R6 ①-ⓐ).
///
/// ## 왜 (라운드5 검증자 실측 · 관통)
/// 데몬은 멀티라인 push 를 전문 1건으로 기록하는데, 원시 바이트 주입 경로에서는 본문 개행이
/// 그대로 Enter 라 TUI 가 **행 단위로 쪼개** 제출한다. 그러면 각 프롬프트는 레코드의 진부분이라
/// ⓐ전문 해시가 어긋나고 ⓑ[`delivery_spans`] 는 `chars > n` 에서 그 레코드를 통째로 건너뛴다 —
/// 층1 전건 미스. 근본 수리는 생산자가 **제출 단위 조각을 따로 기록**하는 것이고(delivery.rs R6),
/// 그러면 이 프롬프트는 조각 레코드와 전문 해시로 일치해 여기까지 오지 않는다. 이 함수는 그
/// 조각 레코드가 없는 경우, 즉 **구 데몬 + 신 팩 스큐**의 잔여 방어선이다.
///
/// ## 왜 이 규칙만 평문인가 (정직 고지)
/// 원장은 본문을 통째로 보관하지 않는다(그 자체가 프롬프트 유출 저장소가 되므로 `preview` 64자만
/// 남긴다). 그래서 "레코드 안에 이 프롬프트가 있는가"는 **해시로 확증할 수 없고** preview 평문
/// 대조밖에 방법이 없다 — 이 모듈에서 유일하게 증거 등급이 낮은 규칙이며 그만큼 좁게 건다.
///
/// ## 오너 오차단을 막는 세 자물쇠 (실측으로 정한 값)
///   ⓐ **최소 길이** [`DELIVERY_WITHIN_MIN_CHARS`] — 짧은 문장이 "어떤 레코드의 부분"이라는
///     이유로 상시 차단되면 실사용 장애다. 하한 미만은 아예 보지 않는다.
///   ⓑ **경계 정합** — 매치가 preview 안에서 행/어절 경계(앞뒤가 공백이거나 preview 끝)에
///     맞아떨어져야 한다. 실제 관통은 '행 하나가 통째로 제출된 것'이므로 정규화 후 그 조각의
///     양옆은 반드시 공백(원래 개행)이다. 어절 중간을 자르는 우연한 포함은 여기서 죽는다.
///   ⓒ `units == 1`(쪼개질 수 없는 배달 — 신 데몬이 알려 준다)은 아예 건너뛴다.
fn prompt_within_delivery(norm: &CharIndex, delivery: &DeliveryMap) -> Option<String> {
    let n = norm.len();
    if n < DELIVERY_WITHIN_MIN_CHARS {
        return None;
    }
    let need: Vec<char> = norm.as_str().chars().collect();
    for (sha, meta) in delivery.iter() {
        let Some(chars) = meta.chars else { continue };
        if chars <= n as i64 {
            continue; // 전문 일치·부분 일치는 앞 규칙들의 몫이다
        }
        let Some(prev) = meta.preview.as_deref().filter(|p| !p.is_empty()) else { continue };
        if let Some(u) = meta.units {
            if u <= 1 {
                continue; // 개행이 없어 쪼개질 수 없는 배달
            }
        }
        let hay: Vec<char> = prev.chars().collect();
        if hay.len() < n {
            continue;
        }
        for i in 0..=(hay.len() - n) {
            if hay[i..i + n] != need[..] {
                continue;
            }
            let left_ok = i == 0 || hay[i - 1] == ' ';
            let right_ok = i + n == hay.len() || hay[i + n] == ' ';
            if left_ok && right_ok {
                return Some(format!(
                    "프롬프트 {n}자가 더 긴 배달(레코드 {chars}자 · sha256={}…)의 한 조각과 \
                     정확히 겹친다(위치 {i} · 행/어절 경계 정합) — 멀티라인 기계 push 가 행 \
                     단위로 쪼개져 제출된 정황이다. ★근거는 preview 평문 대조이며 해시 확증이 \
                     아니다(원장은 본문을 보관하지 않는다)",
                    sha.chars().take(12).collect::<String>()
                ));
            }
        }
    }
    None
}

/// 조각 상한 초과 배달이 **창([`DELIVERY_CAPPED_FOLD_S`]) 안에** 있으면 사유, 없으면 `None`
/// (python `_capped_recent`).
///
/// 창을 두고 **무기한을 기각한** 근거: 무기한 규칙은 원장에 그런 레코드 한 줄만 있으면 오너를
/// **영구 차단**한다. 관측 고지는 [`read_delivery`] 가 창과 무관하게 하고, 여기서는 접을지만 판단한다.
fn capped_recent(delivery: &DeliveryMap) -> Option<String> {
    let mut best: Option<(&str, f64, i64)> = None;
    for (sha, meta) in delivery.iter() {
        let n = capped_count(meta);
        if n == 0 || meta.age > DELIVERY_CAPPED_FOLD_S {
            continue; // 창 밖 — 접지 않는다(고지는 read_delivery 가 이미 했다)
        }
        if best.is_none_or(|(_, age, _)| meta.age < age) {
            best = Some((sha, meta.age, n));
        }
    }
    let (sha, age, n) = best?;
    Some(format!(
        "조각 상한을 넘긴 배달이 {age:.0}s 전에 있었다(누락 {n}행 · sha256={}…)",
        sha.chars().take(12).collect::<String>()
    ))
}

/// [`machine_origin`] 의 결과.
pub struct OriginVerdict {
    /// 기계 채널로 왔는가.
    pub machine: bool,
    /// **어느 층이** 접었는가 — `Some(1)` 배달 원장 · `Some(2)` push 라벨 · `None` 접히지 않음.
    ///
    /// 판정 입력이 아니라 **진단**이다(`machine` 이 판정의 전부다). RPC `hook.machine_origin`
    /// 이 이 값을 실어 훅·감사자가 "무엇이 잡았는지"를 알 수 있게 한다 — 층1이 배선된 뒤에도
    /// 층2 만 잡고 있다면 그것은 원장이 비었다는 신호이지 정상이 아니다.
    pub layer: Option<u8>,
    /// 사람이 읽는 사유(판정 입력 아님 · `machine=false` 면 빈 문자열).
    pub reason: String,
    /// 이 판정에서 **새로** 관측된 이상징후(원장 판독 단계의 것은 [`DeliveryRead::anomalies`]).
    pub anomalies: Vec<(String, String)>,
}

/// 이 프롬프트가 **기계 채널**(wake 예약·노드 push·훅 알림)로 왔는가(python `machine_origin`).
///
/// 판별은 2층이며 **층1이 정답**이다:
///   층1 배달 원장 대조 — 데몬이 주입 직전 남긴 해시와 일치하면 기계 유래가 **확정**된다.
///     라벨 유무·문안 규약과 무관하므로 **문안 규약을 우회하는 push** 도 잡는다.
///     ⓐ전문 일치 ⓑ창 밖 전문 일치 ⓒ조각 연접·부분 포함 ⓓ제출 단위 조각과의 전문 일치
///     ⓔ역포함(구 데몬 스큐 · 평문 근거) ⓕ탐색 예산 소진 ⓖ조각 상한 초과 배달 직후.
///   층2 push 규약 라벨 — 원장이 없거나 판독 불가일 때의 폴백. 여기서만 문자열에 의존한다.
///
/// ★한 방향으로만 공격적이다: 어느 층이든 걸리면 기계로 접는다. 거짓 양성(기계→임무)은
/// 자율주행 폭주(치명)고, 거짓 음성(오너→임무 아님)은 한 번 더 묻는 것(경미)이다. 오너가 우연히
/// 기계 push 와 **정규화 후 완전히 같은** 문장을 치는 경우가 후자이며, 그 비대칭은 의도된 설계다.
///
/// ★데몬이 검증한 오퍼레이터(GUI) 입력은 원장에 기록되지 않는다(delivery.rs 불변식 ②).
/// 다만 GUI 가 **프로그램적으로 만든** 주입은 오퍼레이터 토큰이 붙어도 기록된다 — 사람이 앉은
/// 세션이라는 사실과 사람이 친 문장이라는 사실은 다르다.
///
/// ★보장 범위: 이 함수가 닫는 것은 **평시 정상 동작 경로**다. 원장·대장 파일을 직접 조작하는
/// 동일 UID 위조는 닫지 못한다 — SOT `docs/THREAT-MODEL-mission-gate.md`.
pub fn machine_origin(
    prompt: &str,
    delivery: &DeliveryMap,
    ledger_status: LedgerStatus,
) -> OriginVerdict {
    let mut anomalies: Vec<(String, String)> = Vec::new();
    if ledger_status == LedgerStatus::Ok && !delivery.is_empty() {
        let norm_s = normalize(prompt);
        let sha = digest_normalized(&norm_s);
        if let Some(meta) = delivery.get(&sha) {
            let head: String = sha.chars().take(12).collect();
            if meta.stale {
                // ★R5 봉합 ①: 창 밖 일치는 **접되 반드시 흔적을 남긴다**. 접는 근거는 "sha 가
                //   같다"는 기계 사실이고 창은 성능·회전 경계일 뿐이다. 다만 창을 넘겼다는 것은
                //   ⓐpane 이 몇 시간 막혀 있었거나 ⓑ원장·시계가 손대졌다는 신호라 보고 대상이다.
                anomalies.push((
                    "delivery_out_of_window".to_string(),
                    format!(
                        "창({:.0}s) 밖 배달과 전문이 일치해 기계로 접었다 — 배달시각 {} · 지연 \
                         {:.0}s({:.1}h) · sha256={head}… . 종전 구현은 이 경우 층1 을 건너뛰어 \
                         무라벨 push 가 오너 임무로 기록됐다(관통 경로).",
                        // 창 수치는 **진단 문구 전용**이라 프로세스 상수에서 읽는다 —
                        // 호출자가 검체용으로 다른 창을 넘겼다면 이 숫자만 그 창과 다를 수
                        // 있고, 판정(meta.stale)은 넘긴 창으로 이미 확정돼 있다.
                        delivery_window_s(),
                        fmt_ts(Some(meta.ts)),
                        meta.age,
                        meta.age / 3600.0
                    ),
                ));
                return OriginVerdict {
                    machine: true,
            layer: Some(1),
                    reason: format!(
                        "배달 원장 일치(**창 밖** · 지연 {:.1}h · sha256={head}…) — 창을 넘겼어도 \
                         해시가 같으면 데몬이 이 pane 에 주입한 그 문장이다",
                        meta.age / 3600.0
                    ),
                    anomalies,
                };
            }
            return OriginVerdict {
                machine: true,
            layer: Some(1),
                reason: format!(
                    "배달 원장 일치(sha256={head}… origin=daemon) — 데몬이 이 pane 에 주입한 \
                     바로 그 문장이다"
                ),
                anomalies,
            };
        }
        let norm = CharIndex::new(&norm_s);
        match composition(&norm, delivery) {
            Composition::Capped(detail) => {
                anomalies.push((
                    "delivery_anchor_capped".to_string(),
                    format!("부분 일치 탐색이 예산에 도달해 판정을 접었다(fail-closed) — {detail}"),
                ));
                return OriginVerdict {
                    machine: true,
            layer: Some(1),
                    reason: format!("배달 원장 대조 불완전 — {detail}"),
                    anomalies,
                };
            }
            Composition::Concat(detail) => {
                anomalies.push((
                    "delivery_concatenated".to_string(),
                    format!("기계 배달 연접을 한 프롬프트로 제출받았다 — {detail}"),
                ));
                return OriginVerdict {
                    machine: true,
            layer: Some(1),
                    reason: format!("배달 원장 조각 연접 — {detail}"),
                    anomalies,
                };
            }
            Composition::Substr(detail) => {
                anomalies.push((
                    "delivery_substring".to_string(),
                    format!("프롬프트에 기계 배달이 통째로 포함됐다 — {detail}"),
                ));
                return OriginVerdict {
                    machine: true,
            layer: Some(1),
                    reason: format!("배달 원장 부분 포함 — {detail}"),
                    anomalies,
                };
            }
            Composition::None => {}
        }
        if let Some(capped) = capped_recent(delivery) {
            // ★R7 — 원장이 그 배달을 **다 담지 못했다**. 넘긴 행은 대조할 해시가 아예 없으므로
            //   "일치하지 않았다"가 "기계가 아니다"를 뜻하지 못한다(근거 불완전은 통과 근거가
            //   아니다). 이상징후는 read_delivery 가 이미 발행했다.
            return OriginVerdict {
                machine: true,
            layer: Some(1),
                reason: format!(
                    "배달 원장 불완전 — {capped}. 이 배달의 초과분 행은 원장에 없어 대조 자체가 \
                     불가능하므로 판정을 열지 않는다(fail-closed · 창 {DELIVERY_CAPPED_FOLD_S:.0}s)"
                ),
                anomalies,
            };
        }
        if let Some(wdetail) = prompt_within_delivery(&norm, delivery) {
            // ★신 데몬이면 조각 레코드가 있어 여기까지 오지 않는다 — 이 발행은 "구 데몬 + 신 팩
            //   스큐에서 평문 근거로 접었다"는 사실의 고지다(증거 등급 명시).
            anomalies.push((
                "delivery_prompt_within_delivery".to_string(),
                format!(
                    "프롬프트가 더 긴 기계 배달의 조각과 겹쳐 접었다(평문 preview 근거) — {wdetail}"
                ),
            ));
            return OriginVerdict {
                machine: true,
            layer: Some(1),
                reason: format!("배달 원장 역포함(멀티라인 행 분할 정황) — {wdetail}"),
                anomalies,
            };
        }
    }
    if has_machine_label(prompt) {
        return OriginVerdict {
            machine: true,
            layer: Some(2),
            reason: format!(
                "push 규약 라벨 선두({:?}) — 기계 채널(wake/노드 push/훅 알림)",
                label_head(prompt).map(String::from).unwrap_or_default()
            ),
            anomalies,
        };
    }
    OriginVerdict { machine: false,
        layer: None, reason: String::new(), anomalies }
}

/// 이상징후 중복 제거(python `collected_anomalies` 미러) — 키는 `(코드, 사유)` 쌍이고 **순서를
/// 보존**한다. 같은 코드라도 사유가 다르면 둘 다 남는다(사유가 곧 근거다).
pub fn dedup_anomalies(all: &[(String, String)]) -> Vec<(String, String)> {
    let mut seen: Vec<(&str, &str)> = Vec::new();
    let mut out: Vec<(String, String)> = Vec::new();
    for (c, w) in all {
        if seen.iter().any(|(sc, sw)| *sc == c.as_str() && *sw == w.as_str()) {
            continue;
        }
        seen.push((c.as_str(), w.as_str()));
        out.push((c.clone(), w.clone()));
    }
    out
}


// ══════════════════════════════════════════════════════════════════════════════
// 임무 추출 — 프롬프트 → 임무 문자열 (순수 · python `split_clauses`/`extract_mission`)
// ══════════════════════════════════════════════════════════════════════════════

/// ack 단독절 — 이것만 남으면 임무가 아니다(python `ACK_CLAUSES`).
/// 비교는 **공백 전량 제거 + 소문자화** 후 수행한다([`is_ack_clause`]).
pub const ACK_CLAUSES: [&str; 30] = [
    "네", "넵", "예", "응", "어", "ㅇㅇ", "ㅇㅋ", "ok", "okay", "오케이", "그래", "좋아",
    "hi", "hello", "안녕", "안녕하세요", "고마워", "고맙다", "감사", "감사합니다", "수고",
    "ㅋㅋ", "ㅎㅎ", "yes", "y", "n", "no", "아니", "잠깐", "대기",
];

/// python `_WS.sub("", c).lower() in ACK_CLAUSES`.
///
/// ★`to_lowercase` 를 쓰는 이유: python `str.lower()` 는 유니코드 전체 소문자화이고
/// `to_ascii_lowercase` 는 ASCII 만 건드린다. 현재 목록은 전부 ASCII·한글이라 두 답이 같지만,
/// 목록에 한 줄만 추가되면 갈린다 — **오늘 같다는 이유로 다른 술어를 쓰지 않는다**.
pub fn is_ack_clause(clause: &str) -> bool {
    let squeezed: String = clause
        .chars()
        .filter(|&c| !is_py_space(c))
        .collect::<String>()
        .to_lowercase();
    ACK_CLAUSES.iter().any(|a| *a == squeezed)
}

/// 절 경계로 분해(python `split_clauses`). 경계 어휘는 [`crate::declaration::CLAUSE_BOUNDARY`]
/// 를 **그대로** 쓴다(사본 금지).
///
/// ★경계 문자는 **앞 절에 포함**한다 — `declaration` 의 절 경계 규약과 동일하다. 이걸 버리면
/// "…무슨 뜻?" 의 `?` 가 사라져 [`crate::declaration::is_question_clause`] 가 영영 매치되지
/// 않고 "오늘 뭐부터 할까?" 가 **임무로 오탐**된다(python self-test 가 박제한 실제 초안 결함).
pub fn split_clauses(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut buf = String::new();
    for ch in text.chars() {
        buf.push(ch);
        if crate::declaration::CLAUSE_BOUNDARY.contains(ch) {
            out.push(std::mem::take(&mut buf));
        }
    }
    out.push(buf);
    out.into_iter()
        .map(|s| s.trim_matches(is_py_space).to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// [`extract_mission`] 의 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissionExtract {
    /// 인정된 임무(≤ [`MISSION_MAX_CHARS`] 자) 또는 없음.
    pub mission: Option<String>,
    /// 사람이 읽는 사유 — 대장 `reason` 필드에 그대로 실린다.
    pub reason: String,
}

/// 프롬프트 → (임무, 사유). **순수 함수**(부작용 0 · 로케일 비의존 · python `extract_mission`).
///
/// 절 단위로 걸러낸다 — 문자 오프셋을 다루지 않으므로 `declaration` 의 200자 감지창에 갇히지
/// 않는다(임무는 선언 뒤 **어디에나** 올 수 있다).
///   ① 선언절 제외 — "너는 마스터다" 자체는 임무가 아니다
///   ② 질의·인용절 제외 — "오늘 뭐부터 할까?" 는 **보고 요구**지 임무가 아니다
///   ③ ack 단독절 제외 — "응"·"ok"
///   ④ 남은 문자수 < [`MISSION_MIN_CHARS`] → 임무 없음
pub fn extract_mission(prompt: &str) -> MissionExtract {
    let mut kept: Vec<String> = Vec::new();
    let mut dropped: Vec<&'static str> = Vec::new();
    for c in split_clauses(prompt) {
        if crate::declaration::is_declaration_clause(&c) {
            dropped.push("선언절");
            continue;
        }
        if crate::declaration::is_question_clause(&c) {
            dropped.push("질의·인용절");
            continue;
        }
        if is_ack_clause(&c) {
            dropped.push("ack절");
            continue;
        }
        kept.push(c);
    }
    let body = collapse_ws(&kept.join(" "));
    let n = body.chars().count();
    if n < MISSION_MIN_CHARS {
        let why = if dropped.is_empty() {
            "없음".to_string()
        } else {
            dropped.join(", ")
        };
        return MissionExtract {
            mission: None,
            reason: format!("잔여문 {n}자 < 최소 {MISSION_MIN_CHARS}자(제외: {why})"),
        };
    }
    MissionExtract {
        mission: Some(body.chars().take(MISSION_MAX_CHARS).collect()),
        reason: format!("잔여문 {n}자 — 오너 임무로 인정"),
    }
}

// ══════════════════════════════════════════════════════════════════════════════
// 대장(mission ledger) — **writer 단일 소유자** (명세 §2-3 · §2-2 (e))
// ══════════════════════════════════════════════════════════════════════════════
//
// 명세 §2-3: "대장 스키마 1 그대로 · writer 단일(훅 Rust) · python 은 reader". 즉 **판정과
// 기록은 여기가 유일 소유자**이고 python `javis_mission.py` 의 gate/status 는 이 파일이 쓴 것을
// 읽기만 한다. 두 벌이 쓰면 스키마가 조용히 갈리고, 갈린 대장은 게이트를 통째로 무력화한다.
//
// ★기록 규율 둘 — 어느 쪽도 어기면 이번 사고의 재발이거나 그 **반대 방향** 사고다:
//   ⓐ 기계는 **착수 권한을 발급하지 못한다** — 기계 유래 프롬프트로는 `mission` 을 쓰지 않는다.
//   ⓑ 기계는 진행 중 **오너 임무를 취소하지 못한다** — 기계 유래 경로에서 살아 있는 임무를
//      덮거나 지우지 않는다(`anomalies` 병합만 한다).
//
// ★I/O 규약: 이 절의 함수는 **파일을 열지 않는다**. 호출자가 읽어 온 바이트를 넘기고
// ([`parse_ledger`]) 쓸 바이트를 돌려받는다([`LedgerRecord::to_json`]) — [`read_delivery`] 와
// 같은 규약이며, 그래야 "대장이 손상됐다"·"자리가 디렉터리다" 를 검체가 재현할 수 있다.

/// 대장에 보존하는 이상징후 최대 개수(python `ANOMALY_KEEP`). 오래된 것부터 밀려난다.
pub const ANOMALY_KEEP: usize = 50;

/// 대장 레코드의 이상징후 1건. python 은 `{"code":…, "detail":…}` dict 다.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Anomaly {
    pub code: String,
    pub detail: String,
}

impl Anomaly {
    pub fn new(code: impl Into<String>, detail: impl Into<String>) -> Self {
        Self { code: code.into(), detail: detail.into() }
    }
}

/// 임무 대장 레코드 — **스키마 1**(python `write_ledger` 와 필드·순서 동일).
///
/// ★`extra` 가 있는 이유(파리티 함정): python 의 `_persist_anomalies` 는 대장을 dict 로 읽어
/// `anomalies` 만 갈고 **통째로 다시 쓴다** — 즉 모르는 필드도 보존된다. Rust 가 엄격 구조체로
/// 역직렬화하면 그 필드들이 **조용히 사라진다**. 한 벌이 쓰고 다른 벌이 읽는 파일에서 이런
/// 손실은 "어제는 있던 필드가 오늘 없다"로 나타나고 원인은 어디에도 안 남는다.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LedgerRecord {
    pub schema: u64,
    pub mission: Option<String>,
    pub source: String,
    pub reason: String,
    pub surface: String,
    /// 사람용 표기(`%Y-%m-%dT%H:%M:%S%z`). **판정 입력이 아니다** — 산술은 `ts_epoch` 로만 한다.
    pub ts: String,
    /// ★판정 입력. 종전엔 `ts` 만 있고 게이트가 읽지 않아 **과거 임무가 무기한 유효**했다.
    pub ts_epoch: f64,
    /// 세션 결박 — 이 임무를 발급한 데몬 인스턴스. 데몬이 재기동하면 무효다.
    pub boot_epoch: Option<f64>,
    /// 배달 원장을 **읽을 수 있는 상태에서** 판별했는가([`LedgerStatus::as_str`]).
    /// `"unreadable"` 로 기록된 임무는 게이트가 열지 않는다(fail-closed).
    pub ledger_status: Option<String>,
    /// 기록 시점에 관측된 이상징후 — 차단할 수 없는 조작이라도 **흔적은 남는다**.
    pub anomalies: Vec<Anomaly>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub prompt_chars: Option<usize>,
    /// 스키마 1 밖의 필드 전량(보존 전용 · 판정 입력 아님). 위 docstring 참조.
    #[serde(flatten)]
    pub extra: std::collections::BTreeMap<String, serde_json::Value>,
}

impl LedgerRecord {
    /// 대장에 쓸 바이트(python `json.dumps(rec, ensure_ascii=False)` 와 같은 표기).
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("LedgerRecord 직렬화 실패(순수 구조체)")
    }
}

/// 대장 판독 3상 — [`LedgerStatus`] 와 **같은 구분**을 대장 쪽에서 되풀이한다.
/// '부재'와 '판독 불가'를 융합하면 손상이 정상으로 은폐돼 게이트가 열린다.
#[derive(Debug, Clone, PartialEq)]
pub enum LedgerRead {
    /// 파일 없음 = 임무 없음(정상 · 오류 아님).
    Absent,
    Ok(Box<LedgerRecord>),
    /// 손상·형식 오류 — 사유를 싣는다. **이 상태의 대장은 덮어쓰지 않는다**(원인 보존).
    Unreadable(String),
}

impl LedgerRead {
    /// 살아 있는 임무가 적혀 있는가 — ⓑ(오너 임무 불가침) 판정의 유일한 입력.
    ///
    /// python `if isinstance(rec, dict) and rec.get("mission")` 미러: **빈 문자열은 거짓**이다
    /// (`""` 는 python 에서 falsy). 그래서 `Some("")` 를 살아 있는 임무로 보지 않는다.
    pub fn has_live_mission(&self) -> bool {
        matches!(self, LedgerRead::Ok(r) if r.mission.as_deref().is_some_and(|m| !m.is_empty()))
    }
}

/// 대장 바이트 → [`LedgerRead`](python `read_ledger` 미러 · **순수**).
///
/// `raw` 는 `None` 이면 파일 부재다. 자리가 디렉터리인 것 같은 '존재하되 파일 아님' 은 호출자가
/// [`LedgerRead::Unreadable`] 로 환원해 넘긴다 — 그 판정은 파일시스템 관측이라 여기 못 온다.
pub fn parse_ledger(raw: Option<&[u8]>) -> LedgerRead {
    let Some(bytes) = raw else {
        return LedgerRead::Absent;
    };
    let text = String::from_utf8_lossy(bytes);
    let v: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => return LedgerRead::Unreadable(format!("대장 판독 실패({e})")),
    };
    if !v.is_object() {
        return LedgerRead::Unreadable("대장 형식 오류(dict 아님)".to_string());
    }
    match serde_json::from_value::<LedgerRecord>(v) {
        Ok(r) => LedgerRead::Ok(Box::new(r)),
        Err(e) => LedgerRead::Unreadable(format!("대장 필드 오류({e})")),
    }
}

/// 대장 `source` 어휘 — python 과 **같은 문자열**이어야 한다(소비자가 이 값으로 경로를 읽는다).
pub const SOURCE_HARNESS: &str = "harness_notification";
pub const SOURCE_BOOT_COMMAND: &str = "boot_command";
pub const SOURCE_DECLARATION_RESIDUAL: &str = "declaration_residual";
pub const SOURCE_PROMPT: &str = "prompt";
/// 임무 없이 **이상징후 기록만을 위해** 만들어진 대장. 게이트는 '대장 없음'과 같게 판정한다.
pub const SOURCE_ANOMALY_ONLY: &str = "anomaly_only";

/// 새 레코드 조립(python `write_ledger` 의 dict 구성부 · **순수**).
///
/// `now` 는 호출자가 넘긴다 — 검체가 시간을 고정할 수 있어야 TTL·세션 결박이 재현된다.
#[allow(clippy::too_many_arguments)]
pub fn build_record(
    mission: Option<&str>,
    source: &str,
    reason: &str,
    surface: &str,
    now: f64,
    boot_epoch: Option<f64>,
    ledger_status: Option<LedgerStatus>,
    anomalies: &[(String, String)],
    prompt_chars: Option<usize>,
) -> LedgerRecord {
    LedgerRecord {
        schema: SCHEMA_VERSION,
        mission: mission.map(str::to_string),
        source: source.to_string(),
        reason: reason.to_string(),
        surface: surface.to_string(),
        ts: local_ts(now),
        ts_epoch: now,
        boot_epoch,
        ledger_status: ledger_status.map(|s| s.as_str().to_string()),
        anomalies: anomalies.iter().map(|(c, d)| Anomaly::new(c, d)).collect(),
        prompt_chars,
        extra: Default::default(),
    }
}

/// python `time.strftime("%Y-%m-%dT%H:%M:%S%z")` 미러 — **로컬 시각 + 오프셋**.
///
/// 사람용 표기다(판정은 `ts_epoch` 로만 한다). 로컬 타임존에 의존하는 것은 python 과 같은
/// 성질이며, 그래서 검체는 이 값을 **모양으로만** 잰다(고정 문자열 대조 금지).
fn local_ts(now: f64) -> String {
    use chrono::{Local, TimeZone};
    match Local.timestamp_opt(now.trunc() as i64, 0) {
        chrono::LocalResult::Single(t) => t.format("%Y-%m-%dT%H:%M:%S%z").to_string(),
        // 애매·불가(윤초·DST 접힘·범위 밖)는 판정 입력이 아니므로 표기만 정직하게 비운다.
        _ => String::new(),
    }
}

/// 대장에 박힌 이상징후 + 지금 관측된 것(python `_persist_anomalies` 의 병합부).
/// 키는 `(code, detail)` 이고 **순서를 보존**하며, 상한 초과 시 **오래된 것부터** 밀린다.
pub fn merge_anomalies(recorded: &[Anomaly], observed: &[(String, String)]) -> Vec<Anomaly> {
    let mut all: Vec<Anomaly> = recorded.to_vec();
    all.extend(observed.iter().map(|(c, d)| Anomaly::new(c, d)));
    let mut seen: Vec<(String, String)> = Vec::new();
    let mut out: Vec<Anomaly> = Vec::new();
    for a in all {
        let key = (a.code.clone(), a.detail.clone());
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        out.push(a);
    }
    if out.len() > ANOMALY_KEEP {
        out.drain(..out.len() - ANOMALY_KEEP);
    }
    out
}

// ══════════════════════════════════════════════════════════════════════════════
// record 폴드 — 층0/층0-c/층1/층2 를 **하나의 결정**으로 접는다 (명세 §2-2 (e))
// ══════════════════════════════════════════════════════════════════════════════

/// record 가 접은 결과 — **어느 층이 걸렸는가**. 진단·고지용이며 판정은 [`LedgerPlan`] 이다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordFold {
    /// 층1·층2 — 기계 채널 유래. 대장을 **읽지도 쓰지도 않는다**(권한 발급 ⓐ · 취소 ⓑ 둘 다 차단).
    MachineOrigin(String),
    /// 층0 — harness·도구 내부 알림.
    Harness(String),
    /// 층0-c — 기동 명령문.
    BootCommand(String),
    /// 선언 발화 = 세션 재개장(잔여문이 없으면 `mission=None` 을 **명시적으로** 박는다).
    DeclarationResidual(String),
    /// 비선언 + 임무 인정 — **상향만**(있던 임무를 지우지 않는다).
    Prompt(String),
    /// 비선언 + 임무 없음.
    NoMission(String),
    /// 빈 프롬프트 — 판정 자체를 하지 않는다.
    EmptyPrompt,
}

/// 대장에 무엇을 할 것인가. **이 열거가 ⓐⓑ 불변식의 집행처**다.
#[derive(Debug, Clone, PartialEq)]
pub enum LedgerPlan {
    /// 아무것도 쓰지 않는다(관측된 이상징후도 없다).
    Nothing,
    /// `anomalies` 만 병합해 다시 쓴다 — **판정 필드는 한 글자도 바꾸지 않는다**.
    /// 대장이 없으면 `source=anomaly_only` 최소 레코드를 만든다(권한은 생기지 않는다).
    /// 대장이 **판독 불가**면 아무것도 쓰지 않는다(손상 파일을 덮어 원인을 지우지 않는다).
    AnomaliesOnly,
    /// 새 레코드를 쓴다.
    Write { mission: Option<String>, source: &'static str, reason: String },
}

/// [`record_fold`] 의 결과.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordDecision {
    pub fold: RecordFold,
    pub plan: LedgerPlan,
    /// 이 판정에서 관측된 이상징후 전량(원장 판독분 + 층1 판정분 · 중복 제거됨).
    pub anomalies: Vec<(String, String)>,
    /// stderr 로 드러낼 사람용 고지(은폐 금지 규약). 판정 입력이 아니다.
    pub notice: Option<String>,
}

/// record 판정 본체 — **순수**. 파일도 stdin 도 읽지 않는다(python `_record_step` 의 판정부).
///
/// ## 층의 순서가 곧 우선순위다 (명세 §2-2 (e) — "판정 → 기록")
///   1. **층1·층2**(기계 유래) — 다른 무엇보다 **먼저**. 선언 감지보다 앞이라 push 본문에 섞인
///      "너는 마스터다" 도 대장을 재개장하지 못한다.
///   2. **층0**(harness) — 층1/층2 가 **구조적으로 볼 수 없는** 경로(원장 미경유·무라벨).
///   3. **층0-c**(기동 명령문) — 층1·층2·층0 셋 다 못 보는 제3의 오염원.
///   4. 선언 감지 → 임무 추출.
///
/// ★층0 배제는 **이 함수 전용**이다 — [`machine_origin`] 은 층1/층2 만 보며, 여기의 층0 폴드가
///   그쪽으로 새면 안 된다(R3-P05-1 계약 · 판정별 fail-closed 규칙 독립).
///
/// `ledger` 는 **현재 대장**이며 ⓑ(오너 임무 불가침) 판정에만 쓴다 — 층0·층0-c 가 살아 있는
/// 오너 임무를 덮지 않게 하는 것이 유일한 용도다.
pub fn record_fold(
    prompt: &str,
    delivery: &DeliveryMap,
    ledger_status: LedgerStatus,
    ledger: &LedgerRead,
    read_anomalies: &[(String, String)],
) -> RecordDecision {
    // 빈 프롬프트는 **원장을 보기 전에** 접는다 — 종전 순서 그대로다(여기서 machine_origin 을
    // 부르면 판정도 이상징후도 없는 입력에 탐색 예산을 쓴다).
    if prompt.trim_matches(is_py_space).is_empty() {
        return RecordDecision {
            fold: RecordFold::EmptyPrompt,
            plan: LedgerPlan::Nothing,
            anomalies: dedup_anomalies(read_anomalies),
            notice: None,
        };
    }
    let v = machine_origin(prompt, delivery, ledger_status);
    record_fold_from_origin(prompt, &v, ledger, read_anomalies)
}

/// [`record_fold`] 의 **판정 주입형**(명세 §2-2 e · B2-c) — 층1·층2 판정을 **값으로** 받는다.
///
/// 왜 필요한가: 훅 CLI 는 층1 을 스스로 판정하지 않고 **데몬에게 묻는다**(A12 ·
/// `hook.machine_origin`). 원장 경로 규약의 소유자가 데몬 하나이기 때문이다. 그래서 CLI 에는
/// [`DeliveryMap`] 이 없고, [`record_fold`] 를 그대로 부를 수 없다. 그렇다고 CLI 가 층0·층0-c·
/// 선언·임무 추출의 **순서를 다시 적으면** 그것이 곧 두 번째 규칙 사본이다 — 순서가 이 함수의
/// 본체이므로(층1/2 → 층0 → 층0-c → 선언 → 임무) 규칙은 여기 한 곳에 두고 판정만 주입한다.
///
/// `origin.anomalies` 는 이 함수가 `read_anomalies` 와 함께 병합·중복제거한다. 데몬 RPC 가 이미
/// 병합해 보낸 경우에는 같은 (code, detail) 쌍이라 dedup 이 흡수한다(이중 계상 없음).
pub fn record_fold_from_origin(
    prompt: &str,
    origin: &OriginVerdict,
    ledger: &LedgerRead,
    read_anomalies: &[(String, String)],
) -> RecordDecision {
    let mut anomalies: Vec<(String, String)> = read_anomalies.to_vec();

    if prompt.trim_matches(is_py_space).is_empty() {
        return RecordDecision {
            fold: RecordFold::EmptyPrompt,
            plan: LedgerPlan::Nothing,
            anomalies: dedup_anomalies(&anomalies),
            notice: None,
        };
    }

    // ── 1. 층1·층2 — 기계 유래는 대장을 읽지도 쓰지도 않는다 ────────────────────────
    let v = origin;
    anomalies.extend(v.anomalies.iter().cloned());
    if v.machine {
        return RecordDecision {
            fold: RecordFold::MachineOrigin(v.reason.clone()),
            // ★기계로 접힌 경로의 이상은 **프롬프트 유래**라 지금 붙잡지 않으면 재관측되지
            //   않는다. 판정 필드는 건드리지 않고 흔적만 영속한다.
            plan: LedgerPlan::AnomaliesOnly,
            anomalies: dedup_anomalies(&anomalies),
            notice: Some(format!("기계 유래 프롬프트 — 대장 무변경(임무 아님): {}", v.reason)),
        };
    }

    // ── 2. 층0 — harness·도구 내부 알림 ────────────────────────────────────────────
    match harness_origin(prompt) {
        Some(why) => {
            return RecordDecision {
                fold: RecordFold::Harness(why.clone()),
                plan: owner_safe_null_write(ledger, SOURCE_HARNESS, &why),
                anomalies: dedup_anomalies(&anomalies),
                notice: Some(format!("harness 내부 알림 — 임무 아님(대장 오염 차단): {why}")),
            };
        }
        None => { /* 통과 — 아래로 */ }
    }

    // ── 3. 층0-c — 기동 명령문 ─────────────────────────────────────────────────────
    if let Some(why) = boot_command_origin(prompt) {
        return RecordDecision {
            fold: RecordFold::BootCommand(why.clone()),
            plan: owner_safe_null_write(ledger, SOURCE_BOOT_COMMAND, &why),
            anomalies: dedup_anomalies(&anomalies),
            notice: Some(format!("기동 명령문 프롬프트 — 임무 아님(대장 오염 차단): {why}")),
        };
    }

    // ── 4. 선언 감지 → 임무 추출 ───────────────────────────────────────────────────
    let ex = extract_mission(prompt);
    if crate::declaration::detect(prompt).fire() {
        // 선언 = 세션 재개장. 잔여문이 없으면 **명시적으로 mission=null 을 박아** 직전 세션의
        // 임무가 새 부팅으로 새어 들어오지 않게 한다.
        return RecordDecision {
            fold: RecordFold::DeclarationResidual(ex.reason.clone()),
            plan: LedgerPlan::Write {
                mission: ex.mission,
                source: SOURCE_DECLARATION_RESIDUAL,
                reason: ex.reason,
            },
            anomalies: dedup_anomalies(&anomalies),
            notice: None,
        };
    }
    // 비선언 프롬프트 = **상향만**: 있던 임무를 지우지 않는다.
    match ex.mission {
        Some(m) => RecordDecision {
            fold: RecordFold::Prompt(ex.reason.clone()),
            plan: LedgerPlan::Write { mission: Some(m), source: SOURCE_PROMPT, reason: ex.reason },
            anomalies: dedup_anomalies(&anomalies),
            notice: None,
        },
        None => RecordDecision {
            fold: RecordFold::NoMission(ex.reason.clone()),
            plan: LedgerPlan::AnomaliesOnly,
            anomalies: dedup_anomalies(&anomalies),
            notice: None,
        },
    }
}

/// 층0·층0-c 의 기록 계획 — **진행 중 오너 임무는 덮지 않는다**(불변식 ⓑ).
///
/// 기계가 오너 임무를 **취소**하는 것은 이번 사고의 **반대 방향 사고**다. 살아 있는 임무가
/// 있으면 흔적만 병합하고, 없을 때만 `mission=null` 판정을 박는다 — 실사고의 증거가 임무 대장
/// 그 자체였으므로(기계 산출이 `source":"prompt"` 로 박혀 있었다) **같은 자리에 판정 근거**가
/// 남아야 다음 사고에서 오너가 1초 만에 읽는다.
fn owner_safe_null_write(ledger: &LedgerRead, source: &'static str, reason: &str) -> LedgerPlan {
    if ledger.has_live_mission() {
        return LedgerPlan::AnomaliesOnly;
    }
    LedgerPlan::Write { mission: None, source, reason: reason.to_string() }
}

/// [`LedgerPlan`] → **실제로 쓸 레코드**(없으면 `None` = 무쓰기). **순수**.
///
/// ★파일을 쓰지 않는 이유(의도된 이음매): 상태 파일의 원자적 쓰기는 명세 §2-10 마지막 줄의
/// 공통 함수 `atomic_write_json` 소관이고 BUILD_PLAN 은 그것을 **B4**(T2-6)에 배정했다. 여기서
/// 두 번째 원자 쓰기를 만들면 tmp 명명·백오프·미러 규약이 두 벌로 갈린다. 그래서 이 절은
/// **무엇을 쓸지**까지만 정하고, 바이트를 디스크에 얹는 일은 훅 CLI(B2)가 B4 의 함수로 한다.
#[allow(clippy::too_many_arguments)]
pub fn apply_plan(
    plan: &LedgerPlan,
    ledger: &LedgerRead,
    surface: &str,
    now: f64,
    boot_epoch: Option<f64>,
    ledger_status: LedgerStatus,
    anomalies: &[(String, String)],
    prompt_chars: Option<usize>,
) -> Option<LedgerRecord> {
    match plan {
        LedgerPlan::Nothing => None,
        LedgerPlan::AnomaliesOnly => {
            // 이상징후가 없으면 **아무 파일도 쓰지 않는다**(평시 경로는 무쓰기).
            if anomalies.is_empty() {
                return None;
            }
            match ledger {
                // 손상 대장은 건드리지 않는다 — 덮어쓰면 원인이 사라진다.
                LedgerRead::Unreadable(_) => None,
                LedgerRead::Absent => {
                    let mut r = build_record(
                        None,
                        SOURCE_ANOMALY_ONLY,
                        "오너 임무 지정 없음 — 이 대장은 **이상징후 기록용으로만** 생성됐다\
                         (권한 없음 · '대장 없음'과 동일하게 판정된다)",
                        surface,
                        now,
                        boot_epoch,
                        Some(ledger_status),
                        &[],
                        None,
                    );
                    r.anomalies = merge_anomalies(&[], anomalies);
                    Some(r)
                }
                LedgerRead::Ok(rec) => {
                    // ★판정 필드 전량을 **원본 그대로** 복사한다 — `mission`·`source`·`ts_epoch`·
                    //   `boot_epoch`·`surface`·`ledger_status` 가 게이트 판정의 입력이고,
                    //   여기서 한 글자라도 갈리면 흔적 기록이 권한 조작으로 둔갑한다.
                    let mut out = (**rec).clone();
                    out.anomalies = merge_anomalies(&rec.anomalies, anomalies);
                    Some(out)
                }
            }
        }
        LedgerPlan::Write { mission, source, reason } => Some(build_record(
            mission.as_deref(),
            source,
            reason,
            surface,
            now,
            boot_epoch,
            Some(ledger_status),
            anomalies,
            prompt_chars,
        )),
    }
}

// ══════════════════════════════════════════════════════════════════════════════
#[cfg(test)]
mod tests {
    use super::*;

    /// 층0·층2 판정 코퍼스 — python self-test 가 소비하는 **같은 파일**을 컴파일 타임에 싣는다.
    /// 경로가 바뀌면 여기서 빌드가 깨진다(사본 분화의 재발 경로를 컴파일러가 막는다).
    const CORPUS: &str =
        include_str!("../cysjavis-pack/bin/tests/fixtures/mission-origin-corpus.json");

    fn corpus() -> serde_json::Value {
        serde_json::from_str(CORPUS).expect("mission-origin-corpus.json 판독 불가")
    }

    /// 코퍼스 항목의 본문. 거대 검체는 파일을 부풀리지 않으려 `{"repeat":{"unit","times"}}`
    /// 로 적혀 있다(5MB 상한·200k 프리픽스 축을 재려면 실제로 그 크기가 필요하다).
    fn item_text(it: &serde_json::Value) -> String {
        gen_text(&it["text"])
    }

    /// 거대 검체를 파일에 그대로 싣지 않으려 `{"repeat":{"unit","times"}}` 로 적힌 본문을 편다.
    /// `text`(층0·층2)·`prompt`·`body`(층1)가 같은 지시자를 쓰므로 값 단위로 일반화한다.
    fn gen_text(t: &serde_json::Value) -> String {
        if let Some(s) = t.as_str() {
            return s.to_string();
        }
        let r = &t["repeat"];
        let unit = r["unit"].as_str().expect("repeat.unit");
        let times = r["times"].as_u64().expect("repeat.times") as usize;
        unit.repeat(times)
    }

    /// ★H-A14-1(v2.1 A14 · master 채택): 층1 전송 상한이 **와이어 상한 아래임이 보장되는가.**
    ///
    /// 주석의 숫자가 아니라 **값으로** 부등식을 다시 계산한다:
    ///   `LAYER1_PROMPT_MAX_BYTES × JSON_WORST_EXPANSION + 봉투 < MAX_REQUEST_LINE`
    /// 좌변이 우변을 넘으면 상한이 **조건부로만** 발효한다 — 그것이 A14 가 없앤 결함이다
    /// (종전 문자 상한은 CJK 에서 도달 불가였고 ASCII 에서만 발효했다).
    /// 우변은 데몬 소스에서 **핀**한다(cysd 는 바이너리 크레이트라 상수를 import 할 수 없다) —
    /// 누군가 와이어 상한을 낮추면 이 검체가 먼저 적색이 된다.
    #[test]
    fn layer1_byte_cap_is_provably_under_the_wire_limit() {
        let daemon_src = include_str!("bin/cysd/main.rs");
        assert!(
            daemon_src.contains("const MAX_REQUEST_LINE: usize = 10 * 1024 * 1024;"),
            "와이어 상한 소스 핀이 깨졌다 — 상한 부등식의 우변을 다시 확인하라"
        );
        let wire = 10 * 1024 * 1024usize;
        // 봉투 실측(2026-09-04): 메서드·digest(64자)·플래그 포함 178 B. 여유를 크게 잡아도 성립한다.
        let envelope = 1024usize;
        let worst = LAYER1_PROMPT_MAX_BYTES * JSON_WORST_EXPANSION + envelope;
        assert!(
            worst < wire,
            "상한이 와이어 아래임을 보장하지 못한다: 최악 {worst} B ≥ 와이어 {wire} B"
        );
        // ★음성 대조: 종전 문자 상한을 CJK(3 B/자)로 환산하면 **와이어를 넘는다** — 그것이
        //   '문자 상한이 한 번도 발효하지 못한' 이유다. 이 단언이 깨지면 전제가 바뀐 것이다.
        assert!(
            HARNESS_SCAN_MAX_CHARS * 3 > wire,
            "전제 붕괴: 문자 상한이 CJK 에서도 와이어 아래다 — A14 의 근거를 다시 재라"
        );
    }

    /// ★H-A14-2: 바이트 절단이 **UTF-8 경계를 지킨다**(다중바이트 반토막 금지).
    ///
    /// 반토막이 나면 그 문자열은 직렬화에서 대체문자로 바뀌고 digest 대조가 **조용히** 깨진다 —
    /// 층1 이 통째로 무력화되는 경로(데몬이 불일치로 거절 → legacy 폴백 → 원장 대조 0회).
    #[test]
    fn byte_truncation_never_splits_a_multibyte_char() {
        // 한글 3바이트 × 4 = 12바이트. 상한을 문자 경계가 아닌 곳에 둔다.
        let s = "가나다라";
        for max in 0..=s.len() + 2 {
            let (cut, truncated) = truncate_utf8_bytes(s, max);
            assert!(std::str::from_utf8(cut.as_bytes()).is_ok(), "유효하지 않은 UTF-8: max={max}");
            assert!(cut.len() <= max.min(s.len()), "상한을 넘겨 잘랐다: max={max}");
            assert_eq!(truncated, s.len() > max, "truncated 표기가 사실과 다르다: max={max}");
            assert!(s.starts_with(cut), "앞부분이 보존되지 않았다: max={max}");
        }
        // 상한 이하면 **원본 그대로**(복사·변형 없음)이고 잘리지 않았다고 말한다.
        let (whole, t) = truncate_utf8_bytes(s, 1024);
        assert_eq!((whole, t), (s, false));
        // ASCII 는 문자 = 바이트라 경계 문제가 없다(대조군).
        let (a, t2) = truncate_utf8_bytes("abcdef", 3);
        assert_eq!((a, t2), ("abc", true));
    }

    /// ★H-MISSION-R1(층0 파리티): 마커 블록을 걷어낸 **잔여문**과 태그 밖 **자유 텍스트**가
    /// python 과 같은 값을 낸다.
    ///
    /// 이 코퍼스가 지키는 가장 비싼 계약은 **오너 삼킴 금지**다 — 실측 관통 3형
    /// (`<system-reminder>…</system-reminder>\n<command-name> … 재작성해라`)이 여기 들어 있고,
    /// 그것이 `harness=false` 로 통과해야 부재중 자율 진행이 멈추지 않는다.
    #[test]
    fn layer0_harness_origin_matches_python_corpus() {
        let c = corpus();
        let items = c["layer0"].as_array().expect("layer0 배열 부재");
        assert!(items.len() >= 18, "층0 코퍼스가 줄었다: {}", items.len());
        let mut fails: Vec<String> = Vec::new();
        for it in items {
            let name = it["name"].as_str().unwrap_or("?");
            let text = item_text(it);
            let text = text.as_str();
            let e = &it["expect"];
            let got = harness_origin(text);
            if let Some(want) = e["harness"].as_bool() {
                if got.is_some() != want {
                    fails.push(format!(
                        "{name}: harness 기대 {want} / 실측 {} (why: {})",
                        got.is_some(),
                        it["why"].as_str().unwrap_or("")
                    ));
                    continue;
                }
            }
            if let Some(want) = e["residual"].as_str() {
                let r = harness_strip(text);
                if r != want {
                    fails.push(format!("{name}: 잔여문 기대 {want:?} / 실측 {r:?}"));
                }
            }
            let (free, nblocks) = generic_block_free_text(text);
            if let Some(want) = e["blocks"].as_u64() {
                if nblocks as u64 != want {
                    fails.push(format!("{name}: 블록수 기대 {want} / 실측 {nblocks}"));
                }
            }
            if let Some(want) = e["free_meaningful"].as_u64() {
                let m = meaningful_chars(&free) as u64;
                if m != want {
                    fails.push(format!("{name}: 자유텍스트 기대 {want}자 / 실측 {m}자 ({free:?})"));
                }
            }
        }
        assert!(fails.is_empty(), "층0 파리티 이탈 {}건:\n  - {}", fails.len(), fails.join("\n  - "));
    }

    /// ★H-MISSION-R3(층0-c 파리티): 기동 명령문 필터. 2026-09-03 실사고 문자열이 코퍼스에 있다.
    #[test]
    fn layer0c_boot_command_matches_python_corpus() {
        let c = corpus();
        let items = c["layer0c"].as_array().expect("layer0c 배열 부재");
        assert!(items.len() >= 10, "층0-c 코퍼스가 줄었다: {}", items.len());
        let mut fails: Vec<String> = Vec::new();
        for it in items {
            let name = it["name"].as_str().unwrap_or("?");
            let text = item_text(it);
            let want = it["expect"]["boot_command"].as_bool().expect("boot_command");
            let got = boot_command_origin(&text).is_some();
            if got != want {
                fails.push(format!(
                    "{name}: 기대 {want} / 실측 {got} — {:?} (why: {})",
                    text.chars().take(60).collect::<String>(),
                    it["why"].as_str().unwrap_or("")
                ));
            }
        }
        assert!(fails.is_empty(), "층0-c 파리티 이탈:\n  - {}", fails.join("\n  - "));
        // ★비대칭 재확인(음성 대조): 인용된 명령은 통과한다 — 한글 한 글자가 안전판이다.
        assert!(boot_command_origin("cys boot").is_some());
        assert!(boot_command_origin("cys boot 해줘").is_none(), "한글이 있으면 오너 문장이다");
        assert!(boot_command_origin("cys send --to master hi").is_none(), "발신 계열은 대상 아님");
        assert!(boot_command_origin("run cys boot and report").is_none(), "전체 일치가 아니다");
    }

    /// 코퍼스 레코드 명세 1건 → 원장 JSONL 1줄.
    ///
    /// ★`sha256`·`chars`·`preview` 는 코퍼스에 **없다** — 소비자가 자기 정규화·해시로 만든다는
    /// 것이 fixture 계약이다(필드 사본 금지). 그 계약 때문에 두 구현이 **같은 방향으로 함께
    /// 틀린** 이탈은 이 검체로 잡히지 않는다 — 그것을 가르는 것은 `digest_vectors` 골든이다
    /// ([`normalize_and_digest_match_the_golden_vectors`]). 두 검체는 보완재다.
    fn build_ledger_line(r: &serde_json::Value, now: f64, window: f64) -> String {
        if let Some(raw) = r["raw"].as_str() {
            return raw.to_string(); // 손상 줄은 그대로 싣는다
        }
        let norm = normalize(&gen_text(&r["body"]));
        let ts = match r["ts_offset_s"].as_f64() {
            Some(o) => now + o,
            // ★창 배수로 적힌 시각 — env 오버라이드가 걸려 창이 달라져도 '창 밖'이라는 **의미**가
            //   보존된다(초 단위로 굳히면 그 순간 케이스가 env 에 종속된다).
            None => now + r["ts_offset_window_mult"].as_f64().expect("ts_offset_*") * window,
        };
        // 조각 레코드는 생산자가 더 짧은 앵커를 쓴다(delivery.rs::PART_PREVIEW_CHARS).
        let pv = if r.get("part").is_some() { PART_PREVIEW_CHARS } else { PREVIEW_CHARS };
        let mut rec = serde_json::json!({
            "v": r.get("v").cloned().unwrap_or(serde_json::json!(SCHEMA_VERSION)),
            "surface": r["surface"],
            "ts_epoch": ts,
            "sha256": digest_normalized(&norm),
            "chars": norm.chars().count(),
            "preview": norm.chars().take(pv).collect::<String>(),
            "origin": r["origin"],
        });
        for k in ["units", "parts_capped", "part", "parent"] {
            if let Some(v) = r.get(k) {
                rec[k] = v.clone();
            }
        }
        rec.to_string()
    }

    /// ★H-MISSION-R4(층1 파리티): **배달 원장 대조** — 폴드 7종과 그 음성 대조 23건.
    ///
    /// ★이름이 R4 인 이유: 티켓은 `H-MISSION-R1..3` 를 말했고 d3965d5 가 그 셋을 층0·층2·
    /// 층0-c 에 이미 배정했다. 기존 핀 이름을 **바꾸지 않는다**(계약 추가-only)는 규율에 따라
    /// 층1 은 다음 번호를 받는다 — 이름을 재배치하면 리뷰·CI 가 참조하는 핀이 끊긴다.
    ///
    /// 이 검체가 지키는 두 방향의 계약은 값이 서로 다르다:
    ///   · 기계 산출이 임무로 등록되면 자율주행이 엉뚱한 일을 한다(**치명**)
    ///   · 오너 지시가 기계로 접히면 부재중 자율 진행이 멈춘다(경미하지만 실사용 장애)
    /// 그래서 코퍼스에는 양성만이 아니라 **음성 대조**가 함께 있다 — `L1-short-repeat-with-
    /// owner-tail-not-folded`(오너의 반복 어절 문장) · `L1-parts-capped-old-does-not-fold`
    /// (무기한 규칙이 오너를 영구 차단하는 것 방지) · `L1-absent-ledger-no-label`(부트스트랩
    /// 불가침) 셋이 죽으면 오너가 임무를 못 준다.
    #[test]
    fn layer1_ledger_matches_python_corpus() {
        let c = corpus();
        let items = c["layer1"].as_array().expect("layer1 배열 부재");
        assert!(items.len() >= 23, "층1 코퍼스가 줄었다: {}", items.len());
        // 창은 python 과 같은 자료원(env 반영)을 쓴다 — 코퍼스의 배수 시각이 그 전제다.
        assert!(
            env_anomalies().is_empty(),
            "창 env 오버라이드가 비정상이다 — 이 검체는 정상 env 를 전제한다: {:?}",
            env_anomalies()
        );
        let window = delivery_window_s();
        let mut fails: Vec<String> = Vec::new();
        for it in items {
            let name = it["name"].as_str().unwrap_or("?");
            let me = it["my_surface"].as_str().expect("my_surface");
            let now = it["now_epoch"].as_f64().expect("now_epoch");
            let led = &it["ledger"];
            let (main, rotated) = if !led["present"].as_bool().unwrap_or(false) {
                (LedgerFile::Missing, LedgerFile::Missing)
            } else if let Some(raw) = led["raw"].as_str() {
                (LedgerFile::Content(raw.to_string()), LedgerFile::Missing)
            } else {
                let (mut g0, mut g1) = (String::new(), String::new());
                for r in led["records"].as_array().expect("records") {
                    let line = build_ledger_line(r, now, window);
                    let dst = if r["gen"].as_u64().unwrap_or(0) == 1 { &mut g1 } else { &mut g0 };
                    dst.push_str(&line);
                    dst.push('\n');
                }
                (
                    LedgerFile::Content(g0),
                    if g1.is_empty() { LedgerFile::Missing } else { LedgerFile::Content(g1) },
                )
            };
            let prompt = gen_text(&it["prompt"]);
            let read = read_delivery(&LedgerInput {
                main: &main,
                rotated: &rotated,
                daemon_epoch: None,
                me,
                now,
                window_s: window,
                path_label: "(corpus)",
            });
            let verdict = machine_origin(&prompt, &read.map, read.status);
            let e = &it["expect"];
            let why = it["why"].as_str().unwrap_or("");

            if let Some(want) = e["ledger_status"].as_str() {
                if read.status.as_str() != want {
                    fails.push(format!(
                        "{name}: ledger_status 기대 {want} / 실측 {} ({}) — why: {why}",
                        read.status.as_str(),
                        read.detail
                    ));
                    continue; // 상태가 다르면 뒤 축은 파생 실패다
                }
            }
            if let Some(want) = e["machine"].as_bool() {
                if verdict.machine != want {
                    fails.push(format!(
                        "{name}: machine 기대 {want} / 실측 {} ({:?}) — why: {why}",
                        verdict.machine, verdict.reason
                    ));
                }
            }
            if let Some(want) = e["label"].as_bool() {
                let got = has_machine_label(&prompt);
                if got != want {
                    fails.push(format!("{name}: label 기대 {want} / 실측 {got} — why: {why}"));
                }
            }
            if let Some(want) = e["anomalies"].as_array() {
                let mut all = read.anomalies.clone();
                all.extend(verdict.anomalies.clone());
                let got: Vec<String> =
                    dedup_anomalies(&all).into_iter().map(|(c, _)| c).collect();
                let want: Vec<String> =
                    want.iter().map(|v| v.as_str().unwrap_or("").to_string()).collect();
                if got != want {
                    fails.push(format!(
                        "{name}: 이상징후 기대 {want:?} / 실측 {got:?} — why: {why}"
                    ));
                }
                for code in &got {
                    assert!(is_registered_anomaly(code), "{name}: 미등재 코드 {code} 발행");
                }
            }
        }
        assert!(
            fails.is_empty(),
            "층1 파리티 이탈 {}건:\n  - {}",
            fails.len(),
            fails.join("\n  - ")
        );
    }

    /// ★층1 파이썬 의미론 함정 5종 — 코퍼스로는 안 드러나는 **조용한 이탈** 지점들이다.
    ///
    /// 코퍼스 케이스는 정상 데몬이 만드는 원장만 담는다. 아래 다섯은 그 바깥(구 스키마·손상·
    /// 조작·유니코드 개행)에서만 갈리는데, 갈리면 두 구현이 **같은 파일을 다르게 읽는다**.
    #[test]
    fn layer1_python_semantics_traps_are_pinned() {
        // ⓐ `splitlines()` ≠ `lines()` — 줄 수는 스캔 상한(판독 불가) 판정 입력이다.
        assert_eq!(py_splitlines("a\u{2028}b"), vec!["a", "b"], "U+2028 을 줄바꿈으로 안 봤다");
        assert_eq!("a\u{2028}b".lines().count(), 1, "대조군: Rust lines() 는 자르지 않는다");
        assert_eq!(py_splitlines("a\u{85}b\u{B}c\u{1C}d"), vec!["a", "b", "c", "d"]);
        assert_eq!(py_splitlines("a\r\nb"), vec!["a", "b"], "CRLF 는 한 번만 자른다");
        assert!(py_splitlines("").is_empty(), "빈 문자열은 0줄이다(0바이트=손상 판정의 입력)");
        assert_eq!(py_splitlines("\n"), vec![""], "개행 하나는 1줄이다");
        assert_eq!(py_splitlines("a\n"), vec!["a"], "말미 개행은 빈 줄을 만들지 않는다");

        // ⓑ `str.strip()` 은 `\x1C`–`\x1F` 를 공백으로 본다(normalize 의 25종과 **다른 술어**).
        assert_eq!(py_strip("\u{1C} a \u{1F}"), "a");
        assert_eq!(normalize("a\u{1C}b"), "a\u{1C}b", "정규화는 같은 문자를 공백으로 보지 않는다");

        // ⓒ `isinstance(x, int)` — bool 은 int 이고 float 는 아니다.
        assert_eq!(py_int(Some(&serde_json::json!(true))), Some(1), "bool 은 python 에서 int 다");
        assert_eq!(py_int(Some(&serde_json::json!(3))), Some(3));
        assert_eq!(py_int(Some(&serde_json::json!(3.0))), None, "float 는 int 가 아니다");
        assert_eq!(py_int(Some(&serde_json::json!("3"))), None);
        assert_eq!(py_int(None), None);
        // 같은 관대함이 스키마 대조에도 있다 — `1`·`1.0`·`true` 가 전부 v==1 이다.
        for v in [serde_json::json!(1), serde_json::json!(1.0), serde_json::json!(true)] {
            assert!(v_is_schema(Some(&v)), "{v} 를 스키마 1로 안 봤다");
        }
        for v in [serde_json::json!(2), serde_json::json!("1"), serde_json::json!(null)] {
            assert!(!v_is_schema(Some(&v)), "{v} 를 스키마 1로 봤다");
        }

        // ⓓ `parts_capped` 는 **관대하게 접는 쪽**으로 읽는다 — 숫자만 망가뜨려 fail-open 시키는
        //    경로를 만들지 않는다. 필드 부재만 0 이다.
        let meta = |v: Option<serde_json::Value>| DeliveryMeta {
            ts: 0.0, age: 0.0, stale: false, chars: None, preview: None, origin: None,
            units: None, parts_capped: v, part: None, parent: None,
        };
        assert_eq!(capped_count(&meta(None)), 0, "필드 부재는 0이다(평시 배달)");
        assert_eq!(capped_count(&meta(Some(serde_json::json!(null)))), 0);
        assert_eq!(capped_count(&meta(Some(serde_json::json!(false)))), 0);
        assert_eq!(capped_count(&meta(Some(serde_json::json!(3)))), 3);
        for broken in [
            serde_json::json!(true), serde_json::json!(0), serde_json::json!(-2),
            serde_json::json!("쓰레기"), serde_json::json!({"a": 1}), serde_json::json!(1.7),
        ] {
            assert!(capped_count(&meta(Some(broken.clone()))) >= 1, "{broken} 이 0으로 접혔다");
        }

        // ⓔ env 창 오버라이드 — 하한 미만은 **절단이 아니라 거부**다(짧은 창 = 층1 무력화).
        let name = ENV_DELIVERY_WINDOW_S;
        let d = DELIVERY_WINDOW_S_DEFAULT;
        let (lo, hi) = (DELIVERY_WINDOW_MIN_S, DELIVERY_WINDOW_MAX_S);
        assert_eq!(env_bounded(None, name, d, lo, hi), (d, None), "미설정은 이상이 아니다");
        assert_eq!(env_bounded(Some("  "), name, d, lo, hi).0, d, "빈 값도 이상이 아니다");
        assert_eq!(env_bounded(Some("0"), name, d, lo, hi), (d, None), "0 이하는 오버라이드 없음");
        let (v, a) = env_bounded(Some("쓰레기"), name, d, lo, hi);
        assert_eq!((v, a.as_ref().map(|x| x.0.as_str())), (d, Some("env_not_int")));
        let (v, a) = env_bounded(Some("1"), name, d, lo, hi);
        assert_eq!((v, a.as_ref().map(|x| x.0.as_str())), (d, Some("env_below_floor")),
                   "하한 미만을 절단하면 창이 1초가 되어 층1 이 통째로 무력화된다");
        let (v, a) = env_bounded(Some("99999999"), name, d, lo, hi);
        assert_eq!((v, a.as_ref().map(|x| x.0.as_str())), (hi, Some("env_above_cap")));
        assert_eq!(env_bounded(Some("3600"), name, d, lo, hi), (3600, None), "정상 범위는 그대로");
        // 발행되는 코드는 전부 등재소에 있어야 한다(감사에서 한 곳에서 세어진다).
        for c in ["env_not_int", "env_below_floor", "env_above_cap"] {
            assert!(is_registered_anomaly(c), "{c} 미등재");
        }
    }

    /// ★H-MISSION-R2(층2 파리티): 선두 `[`·`［` 라벨 판정과 그 '첫 글자'.
    #[test]
    fn layer2_machine_label_matches_python_corpus() {
        let c = corpus();
        let items = c["layer2"].as_array().expect("layer2 배열 부재");
        assert!(items.len() >= 10, "층2 코퍼스가 줄었다: {}", items.len());
        let mut fails: Vec<String> = Vec::new();
        for it in items {
            let name = it["name"].as_str().unwrap_or("?");
            let text = it["text"].as_str().expect("text");
            let e = &it["expect"];
            if let Some(want) = e["label"].as_bool() {
                let got = has_machine_label(text);
                if got != want {
                    fails.push(format!(
                        "{name}: label 기대 {want} / 실측 {got} (why: {})",
                        it["why"].as_str().unwrap_or("")
                    ));
                    continue;
                }
            }
            if let Some(want) = e["head"].as_str() {
                let got = label_head(text).map(|c| c.to_string()).unwrap_or_default();
                if got != want {
                    fails.push(format!("{name}: head 기대 {want:?} / 실측 {got:?}"));
                }
            }
        }
        assert!(fails.is_empty(), "층2 파리티 이탈:\n  - {}", fails.join("\n  - "));
    }

    /// ★상수·마커·이상징후 등재소 파리티 — 값이 갈리면 두 구현이 **다른 규칙**을 집행한다.
    #[test]
    fn constants_and_registries_match_python_corpus() {
        let c = corpus();
        let k = &c["$constants"];
        assert_eq!(k["MISSION_MIN_CHARS"].as_u64(), Some(MISSION_MIN_CHARS as u64));
        assert_eq!(k["HARNESS_SCAN_MAX_CHARS"].as_u64(), Some(HARNESS_SCAN_MAX_CHARS as u64));
        assert_eq!(
            k["HARNESS_SCAN_PREFIX_CHARS"].as_u64(),
            Some(HARNESS_SCAN_PREFIX_CHARS as u64)
        );
        assert_eq!(k["MISSION_MAX_CHARS"].as_u64(), Some(MISSION_MAX_CHARS as u64));
        // ── 층1 상수 — 갈리면 두 구현이 **다른 규칙**을 집행한다(값 하나가 곧 폴드 경계다) ──
        assert_eq!(k["SCHEMA_VERSION"].as_u64(), Some(SCHEMA_VERSION));
        assert_eq!(k["PREVIEW_CHARS"].as_u64(), Some(PREVIEW_CHARS as u64));
        assert_eq!(k["PART_PREVIEW_CHARS"].as_u64(), Some(PART_PREVIEW_CHARS as u64));
        assert_eq!(k["DELIVERY_PART_MIN_CHARS"].as_u64(), Some(DELIVERY_PART_MIN_CHARS as u64));
        assert_eq!(k["DELIVERY_WITHIN_MIN_CHARS"].as_u64(), Some(DELIVERY_WITHIN_MIN_CHARS as u64));
        assert_eq!(k["DELIVERY_SPAN_OCC_BUDGET"].as_u64(), Some(DELIVERY_SPAN_OCC_BUDGET as u64));
        assert_eq!(k["DELIVERY_CAPPED_FOLD_S"].as_f64(), Some(DELIVERY_CAPPED_FOLD_S));
        assert_eq!(k["DELIVERY_SCAN_LINES"].as_u64(), Some(DELIVERY_SCAN_LINES as u64));
        assert_eq!(k["LEDGER_MAX_READ_BYTES"].as_u64(), Some(LEDGER_MAX_READ_BYTES));
        assert_eq!(k["DELIVERY_WINDOW_S_DEFAULT"].as_i64(), Some(DELIVERY_WINDOW_S_DEFAULT));
        // 창 경계는 코퍼스에 없다(env 가드 전용) — 값이 뒤집히지 않았는지만 잠근다.
        assert!(DELIVERY_WINDOW_MIN_S < DELIVERY_WINDOW_S_DEFAULT);
        assert!(DELIVERY_WINDOW_S_DEFAULT < DELIVERY_WINDOW_MAX_S);
        // 원장 상태 어휘 — python 상수와 같은 문자열이어야 소비자가 상태를 읽는다.
        assert_eq!(LedgerStatus::Absent.as_str(), "absent");
        assert_eq!(LedgerStatus::Ok.as_str(), "ok");
        assert_eq!(LedgerStatus::Unreadable.as_str(), "unreadable");
        // 마커 2계층 — 이름과 **순서**까지 같아야 한다(교대 우선순위가 판정에 영향을 준다).
        let notify: Vec<&str> = c["harness_markers"]["notify"]
            .as_array()
            .expect("notify")
            .iter()
            .map(|v| v.as_str().unwrap_or(""))
            .collect();
        assert_eq!(notify, HARNESS_NOTIFY_MARKERS.to_vec(), "알림 마커 목록이 갈렸다");
        let context: Vec<&str> = c["harness_markers"]["context"]
            .as_array()
            .expect("context")
            .iter()
            .map(|v| v.as_str().unwrap_or(""))
            .collect();
        assert_eq!(context, HARNESS_CONTEXT_MARKERS.to_vec(), "컨텍스트 마커 목록이 갈렸다");
        // 이상징후 등재소 — 집합 동일(순서는 무관 · 코퍼스는 정렬본이다).
        let mut want: Vec<&str> = c["anomaly_codes"]
            .as_array()
            .expect("anomaly_codes")
            .iter()
            .map(|v| v.as_str().unwrap_or(""))
            .collect();
        want.sort_unstable();
        let mut got: Vec<&str> = ANOMALY_CODES.iter().map(|(c, _)| *c).collect();
        got.sort_unstable();
        assert_eq!(got, want, "이상징후 등재소가 갈렸다");
        for code in &want {
            assert!(is_registered_anomaly(code), "{code} 미등재");
        }
        assert!(!is_registered_anomaly("made_up_code"));
    }

    /// ★정규화·해시 파리티 — 원장 대조의 **단일 산식**이다. 갈리면 해시가 안 맞아 원장 대조가
    /// 조용히 무력화된다(항상 '미일치' = fail-open).
    #[test]
    fn normalize_and_digest_are_the_ledger_key() {
        assert_eq!(normalize("  a \t\n b  "), "a b");
        assert_eq!(normalize("a\u{3000}b"), "a b", "전각 공백도 접힌다");
        // python `delivery_digest("abc")` = sha256 표준 벡터.
        assert_eq!(
            digest_text("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(digest_text("다음 액션 착수"), digest_text("  다음   액션\t착수 \n"));
        assert_ne!(digest_text("가"), digest_text("나"));
        // ★공백 술어 2종의 **분리** 계약: 정규화는 White_Space(25), 층0 계수는 python \s(29).
        //   섞으면 조용히 갈린다.
        assert!(!'\u{1C}'.is_whitespace() && is_py_space('\u{1C}'));
        assert_eq!(normalize("a\u{1C}b"), "a\u{1C}b", "정규화는 \\x1C 를 공백으로 보지 않는다");
        assert_eq!(
            (0u32..0x11_0000).filter(|&c| char::from_u32(c).is_some_and(is_py_space)).count(),
            29
        );
    }

    /// ★정규화·해시 **골든 벡터** 파리티(W-A cd3b8e5 fixture `digest_vectors` 소비).
    ///
    /// 바로 위 [`normalize_and_digest_are_the_ledger_key`] 와 무엇이 다른가 — 그쪽은 이 모듈이
    /// 자기 규칙에 충실한지를 재고, 이쪽은 **python 과 같은 값을 내는지**를 잰다. 그리고 층1
    /// 케이스(`layer1`)로는 그것을 잴 수 없다: 그 케이스들의 원장 레코드는 **소비자 자신의**
    /// 정규화·digest 로 만들어지므로, python 과 Rust 가 **같은 방향으로 함께 틀린**(대칭) 이탈은
    /// 양쪽 다 초록인 채 통과한다. 리터럴 골든만이 그 대칭 이탈을 가른다 — 명세 §2-3 이 층1
    /// 파리티의 핵심으로 지목한 것이 바로 `해시 = digest_normalized` 이기 때문이다.
    ///
    /// 담긴 축: 공백 접기 · 전각 공백 · 트림 · **sha256 표준 벡터**(대조 키가 진짜 sha256 인가) ·
    /// 한글 기본형 ↔ 공백 재배치본이 **같은 해시**(TUI 재배치 내성) · 빈 입력 · 공백뿐 · CRLF ·
    /// 내부 다중 공백. 기대값은 전부 python 현행 함수의 **실측치**다(지어낸 값 0).
    #[test]
    fn normalize_and_digest_match_the_golden_vectors() {
        let c = corpus();
        let items = c["digest_vectors"]
            .as_array()
            .expect("digest_vectors 섹션 부재 — 대칭 이탈 탐지기가 사라졌다");
        assert!(items.len() >= 10, "골든 벡터가 줄었다: {}", items.len());
        let mut fails: Vec<String> = Vec::new();
        for it in items {
            let name = it["name"].as_str().unwrap_or("?");
            let text = item_text(it);
            let e = &it["expect"];
            let want_norm = e["normalized"].as_str().expect("expect.normalized");
            let got_norm = normalize(&text);
            if got_norm != want_norm {
                fails.push(format!("{name}: 정규화 기대 {want_norm:?} / 실측 {got_norm:?}"));
                continue; // 해시는 정규화의 함수다 — 앞이 틀리면 뒤는 파생 실패다
            }
            let want_sha = e["sha256"].as_str().expect("expect.sha256");
            let got_sha = digest_text(&text);
            if got_sha != want_sha {
                fails.push(format!("{name}: sha256 기대 {want_sha} / 실측 {got_sha}"));
            }
            // 재정규화 금지 계약 — 이미 정규화된 문자열에 digest_normalized 를 걸면 같은 값이다.
            assert_eq!(digest_normalized(&got_norm), got_sha, "{name}: 두 진입점이 갈렸다");
        }
        assert!(
            fails.is_empty(),
            "정규화·해시 골든 이탈 {}건 — python 과 **다른 키**로 원장을 대조하게 된다(층1 전건 미스):\n  - {}",
            fails.len(),
            fails.join("\n  - ")
        );
    }

    /// ★투명문자 집합 파리티 — python `unicodedata.category(ch)=="Cf"` + 폴백 목록의 실측
    /// 전수(170자 21구간). 근사하면 갈리므로 측정값을 박제한다.
    #[test]
    fn transparent_set_is_the_measured_python_set() {
        let n = (0u32..0x11_0000)
            .filter(|&c| char::from_u32(c).is_some_and(is_transparent))
            .count();
        assert_eq!(n, 170, "투명문자 집합(170)이 갈렸다");
        for c in ['\u{200B}', '\u{FEFF}', '\u{AD}', '\u{2060}', '\u{E0020}'] {
            assert!(is_transparent(c), "{c:?} 누락");
        }
        for c in ['a', '가', ' ', '[', '\u{3000}'] {
            assert!(!is_transparent(c), "{c:?} 를 투명문자로 봤다");
        }
        // 투명문자 선행 라벨은 **라벨이다**(우회 차단).
        assert!(has_machine_label("\u{200B}[보고] x"));
        assert!(has_machine_label("  \t[wakeup] x"));
        assert!(!has_machine_label("보고: 라벨 없음"));
    }

    // ── H-MISSION-R5 — 대장 writer (B1-5) ────────────────────────────────────────
    //
    // 이 검체군이 지키는 것은 **두 방향의 불변식**이다(둘 다 실사고에서 왔다):
    //   ⓐ 기계는 착수 권한을 **발급**하지 못한다 — 2026-08-01 사고의 본체.
    //   ⓑ 기계는 진행 중 오너 임무를 **취소**하지 못한다 — 그 반대 방향 사고.
    // ⓐ만 재면 '모든 기록을 막는' 구현이 만점을 받는데, 그 구현은 오너 임무를 워커 push 가
    // 지워 버리는 ⓑ 위반이다. 그래서 두 축을 **같은 검체군**에서 잰다.

    fn empty_map() -> DeliveryMap {
        DeliveryMap::default()
    }

    /// 폴드 1건 — 원장 없음(부트스트랩 불가침 상태)에서의 판정.
    fn fold_absent(prompt: &str, ledger: &LedgerRead) -> RecordDecision {
        record_fold(prompt, &empty_map(), LedgerStatus::Absent, ledger, &[])
    }

    fn live_ledger(mission: &str) -> LedgerRead {
        LedgerRead::Ok(Box::new(build_record(
            Some(mission),
            SOURCE_PROMPT,
            "잔여문 9자 — 오너 임무로 인정",
            "101",
            1_700_000_000.0,
            Some(1_699_999_000.0),
            Some(LedgerStatus::Ok),
            &[],
            Some(9),
        )))
    }

    /// ★H-MISSION-R5-a: **임무 추출**(`split_clauses`·`extract_mission`) python 파리티.
    ///
    /// 경계 문자를 앞 절에 붙이는 규약이 죽으면 `?` 가 사라져 "오늘 뭐부터 할까?" 가 임무로
    /// 오탐된다 — python self-test 가 박제한 실제 초안 결함이라 여기서도 박는다.
    #[test]
    fn extract_mission_drops_declaration_question_and_ack_clauses() {
        // 선언절만 → 임무 없음(선언 자체는 임무가 아니다).
        let e = extract_mission("너는 마스터이다.");
        assert!(e.mission.is_none(), "선언절이 임무로 남았다: {e:?}");
        assert!(e.reason.contains("선언절"), "제외 사유에 선언절이 없다: {}", e.reason);

        // 질의절 → 임무 없음(보고 요구지 착수 지시가 아니다).
        let e = extract_mission("오늘 뭐부터 할까?");
        assert!(e.mission.is_none(), "질의절이 임무가 됐다: {e:?}");

        // ack 단독절 → 임무 없음. 공백 제거 + 소문자화 후 비교한다.
        for s in ["응", "  O K  ", "ㅇㅋ", "감사합니다"] {
            let e = extract_mission(s);
            assert!(e.mission.is_none(), "ack 절이 임무가 됐다({s:?}): {e:?}");
        }

        // 선언 + 임무 → 잔여문만 임무가 된다.
        let e = extract_mission("너는 마스터이다. 릴리스 게이트를 통과시켜라.");
        assert_eq!(
            e.mission.as_deref(),
            Some("릴리스 게이트를 통과시켜라."),
            "선언 뒤 잔여문이 임무로 안 잡혔다: {e:?}"
        );

        // 길이 하한·상한.
        assert!(extract_mission("ㄱ").mission.is_none(), "1자가 임무로 인정됐다");
        let long = "가".repeat(MISSION_MAX_CHARS + 50);
        let e = extract_mission(&long);
        assert_eq!(
            e.mission.as_deref().map(|m| m.chars().count()),
            Some(MISSION_MAX_CHARS),
            "임무 상한이 문자 단위로 안 잘렸다"
        );
    }

    /// ★H-MISSION-R5-b(**ⓐ 권한 발급 차단**): 기계 유래 프롬프트는 `mission` 을 쓰지 않는다.
    ///
    /// 층1·층2·층0·층0-c 넷 다 각각 이 성질을 가져야 한다 — 한 층만 새도 사고는 재발한다.
    #[test]
    fn machine_channels_never_write_a_mission() {
        let none = LedgerRead::Absent;
        // 층2 — push 규약 라벨(원장 부재 폴백).
        let d = fold_absent("[worker-1 완료] 릴리스 게이트를 통과시켜라", &none);
        assert!(
            matches!(d.fold, RecordFold::MachineOrigin(_)),
            "층2 라벨 push 가 기계로 안 접혔다: {:?}",
            d.fold
        );
        assert_eq!(d.plan, LedgerPlan::AnomaliesOnly, "기계 유래가 대장을 썼다: {:?}", d.plan);

        // 층0 — harness 내부 알림.
        let d = fold_absent(
            "<system-reminder>배경 작업이 끝났다</system-reminder>",
            &none,
        );
        assert!(matches!(d.fold, RecordFold::Harness(_)), "층0 이 안 걸렸다: {:?}", d.fold);
        match &d.plan {
            LedgerPlan::Write { mission, source, .. } => {
                assert!(mission.is_none(), "층0 이 임무를 발급했다: {mission:?}");
                assert_eq!(*source, SOURCE_HARNESS);
            }
            other => panic!("층0 판정 근거가 대장에 안 남는다: {other:?}"),
        }

        // ★핵심: push **본문에 섞인 선언**도 대장을 재개장하지 못한다(층1/2 가 선언 감지보다 앞).
        let d = fold_absent("[wakeup] 너는 마스터이다. 지금 배포해라.", &none);
        assert!(
            matches!(d.fold, RecordFold::MachineOrigin(_)),
            "push 본문의 선언이 세션을 재개장했다 — 사고 재발 경로: {:?}",
            d.fold
        );
        assert_eq!(d.plan, LedgerPlan::AnomaliesOnly);
    }

    /// ★H-MISSION-R5-c(**ⓑ 오너 임무 불가침**): 살아 있는 오너 임무를 기계가 지우지 못한다.
    ///
    /// ⓐ만 재는 구현(=전부 무쓰기)이 여기서 죽는다. 층0·층0-c 는 대장이 **비었을 때만**
    /// `mission=null` 판정을 박고, 임무가 살아 있으면 흔적만 병합한다.
    #[test]
    fn machine_channels_never_cancel_a_live_owner_mission() {
        let live = live_ledger("릴리스 게이트를 통과시켜라");
        for prompt in [
            "<system-reminder>배경 작업이 끝났다</system-reminder>",
            "cys boot",
        ] {
            let d = fold_absent(prompt, &live);
            assert!(
                matches!(d.fold, RecordFold::Harness(_) | RecordFold::BootCommand(_)),
                "층0/층0-c 가 안 걸렸다({prompt:?}): {:?}",
                d.fold
            );
            assert_eq!(
                d.plan,
                LedgerPlan::AnomaliesOnly,
                "기계가 살아 있는 오너 임무를 덮었다({prompt:?}) — 반대 방향 사고",
                );
        }
        // 같은 프롬프트라도 대장이 비었으면 판정 근거를 박는다(진단 가능성 보존).
        let d = fold_absent("cys boot", &LedgerRead::Absent);
        assert!(
            matches!(&d.plan, LedgerPlan::Write { mission: None, source, .. } if *source == SOURCE_BOOT_COMMAND),
            "빈 대장에 층0-c 판정 근거가 안 남는다: {:?}",
            d.plan
        );
    }

    /// ★H-MISSION-R5-d: 선언은 **세션 재개장**이고, 잔여문이 없으면 `mission=null` 을
    /// **명시적으로** 박는다 — 직전 세션의 임무가 새 부팅으로 새어 들어오는 경로를 끊는다.
    #[test]
    fn declaration_reopens_the_session_and_nulls_a_stale_mission() {
        let stale = live_ledger("지난 세션의 임무");
        let d = fold_absent("너는 마스터이다.", &stale);
        match &d.plan {
            LedgerPlan::Write { mission, source, .. } => {
                assert!(mission.is_none(), "잔여문 없는 선언이 임무를 남겼다: {mission:?}");
                assert_eq!(*source, SOURCE_DECLARATION_RESIDUAL);
            }
            other => panic!("선언이 대장을 재개장하지 않았다: {other:?}"),
        }
        // 잔여문이 있으면 그것이 새 임무다.
        let d = fold_absent("너는 마스터이다. 릴리스 게이트를 통과시켜라.", &stale);
        assert!(
            matches!(&d.plan, LedgerPlan::Write { mission: Some(m), source, .. }
                     if m == "릴리스 게이트를 통과시켜라." && *source == SOURCE_DECLARATION_RESIDUAL),
            "선언 뒤 잔여문이 새 임무로 안 잡혔다: {:?}",
            d.plan
        );
    }

    /// ★H-MISSION-R5-e: 비선언 프롬프트는 **상향만** — 있던 임무를 지우지 않는다.
    #[test]
    fn plain_prompt_only_raises_never_clears() {
        let live = live_ledger("이전 임무");
        // 임무가 있는 평문 → 새 임무로 기록.
        let d = fold_absent("릴리스 게이트를 통과시켜라.", &live);
        assert!(
            matches!(&d.plan, LedgerPlan::Write { mission: Some(_), source, .. } if *source == SOURCE_PROMPT),
            "평문 임무가 기록되지 않았다: {:?}",
            d.plan
        );
        // 임무가 없는 평문 → 대장 무변경(지우지 않는다).
        let d = fold_absent("응", &live);
        assert_eq!(d.plan, LedgerPlan::AnomaliesOnly, "ack 가 오너 임무를 지웠다: {:?}", d.plan);
        assert!(matches!(d.fold, RecordFold::NoMission(_)), "{:?}", d.fold);
    }

    /// ★H-MISSION-R5-f: `apply_plan` — **평시 무쓰기**와 **손상 대장 불가침**.
    ///
    /// 이상징후가 없으면 파일을 만들지 않고, 대장이 판독 불가면 덮어쓰지 않는다(원인 보존).
    /// 후자가 죽으면 손상 원인이 다음 기록에 지워져 진단이 영영 불가능해진다.
    #[test]
    fn apply_plan_writes_nothing_in_the_quiet_path_and_never_overwrites_a_damaged_ledger() {
        let a = |plan: &LedgerPlan, led: &LedgerRead, an: &[(String, String)]| {
            apply_plan(plan, led, "101", 1_700_000_100.0, None, LedgerStatus::Absent, an, None)
        };
        // 이상징후 0 → 무쓰기.
        assert!(
            a(&LedgerPlan::AnomaliesOnly, &LedgerRead::Absent, &[]).is_none(),
            "평시 경로가 파일을 만들었다"
        );
        let an = vec![("ledger_absent".to_string(), "원장 없음".to_string())];
        // 손상 대장 → 무쓰기.
        assert!(
            a(&LedgerPlan::AnomaliesOnly, &LedgerRead::Unreadable("깨짐".into()), &an).is_none(),
            "손상 대장을 덮어썼다 — 원인이 지워진다"
        );
        // 대장 부재 + 이상징후 → anomaly_only 최소 레코드(권한은 생기지 않는다).
        let r = a(&LedgerPlan::AnomaliesOnly, &LedgerRead::Absent, &an).expect("흔적이 안 남았다");
        assert_eq!(r.source, SOURCE_ANOMALY_ONLY);
        assert!(r.mission.is_none(), "흔적 기록이 임무를 발급했다: {:?}", r.mission);
        assert_eq!(r.anomalies.len(), 1);
        assert_eq!(LedgerRead::Ok(Box::new(r)).has_live_mission(), false);
    }

    /// ★H-MISSION-R5-g(**판정 필드 불가침**): 흔적 병합이 게이트 판정 입력을 한 글자도 바꾸지
    /// 않는다. 이 검체가 죽으면 '흔적 기록'이 권한 조작 경로로 둔갑한다.
    #[test]
    fn anomaly_merge_preserves_every_verdict_field() {
        let before = live_ledger("릴리스 게이트를 통과시켜라");
        let an = vec![("delivery_out_of_window".to_string(), "창 밖 일치".to_string())];
        let after = apply_plan(
            &LedgerPlan::AnomaliesOnly,
            &before,
            "999",                       // ← 다른 surface 를 넘겨도
            9_999_999_999.0,             // ← 다른 시각을 넘겨도
            Some(1.0),                   // ← 다른 boot_epoch 를 넘겨도
            LedgerStatus::Unreadable,    // ← 다른 원장 상태를 넘겨도
            &an,
            Some(1234),
        )
        .expect("흔적 병합이 아무것도 안 냈다");
        let LedgerRead::Ok(b) = &before else { unreachable!() };
        assert_eq!(after.mission, b.mission, "mission 이 바뀌었다");
        assert_eq!(after.source, b.source, "source 가 바뀌었다");
        assert_eq!(after.surface, b.surface, "surface 가 바뀌었다 — 결박이 풀린다");
        assert_eq!(after.ts_epoch, b.ts_epoch, "ts_epoch 가 바뀌었다 — TTL 이 되살아난다");
        assert_eq!(after.boot_epoch, b.boot_epoch, "boot_epoch 가 바뀌었다 — 세션 결박이 풀린다");
        assert_eq!(after.ledger_status, b.ledger_status, "ledger_status 가 바뀌었다");
        assert_eq!(after.prompt_chars, b.prompt_chars, "prompt_chars 가 바뀌었다");
        // 바뀌는 것은 anomalies 하나뿐이다.
        assert_eq!(after.anomalies.len(), 1);
        assert_eq!(after.anomalies[0].code, "delivery_out_of_window");
    }

    /// ★H-MISSION-R5-h: 대장 왕복 — 스키마 1 필드 보존 + **모르는 필드 보존**.
    ///
    /// python `_persist_anomalies` 는 대장을 dict 로 읽어 통째로 다시 쓰므로 모르는 필드가
    /// 보존된다. Rust 가 엄격 구조체로 읽으면 그 필드들이 **조용히 사라진다** — 한 벌이 쓰고
    /// 다른 벌이 읽는 파일에서 그 손실은 원인 없이 나타난다.
    #[test]
    fn ledger_roundtrip_keeps_schema1_fields_and_unknown_ones() {
        let raw = r#"{"schema":1,"mission":"배포해라","source":"prompt","reason":"인정",
            "surface":"101","ts":"2026-09-04T20:00:00+0900","ts_epoch":1788000000.0,
            "boot_epoch":1787999000.0,"ledger_status":"ok",
            "anomalies":[{"code":"ledger_absent","detail":"원장 없음"}],
            "prompt_chars":12,"future_field":{"keep":"me"}}"#.as_bytes();
        let LedgerRead::Ok(rec) = parse_ledger(Some(raw)) else {
            panic!("정상 대장이 판독 불가로 접혔다");
        };
        assert_eq!(rec.mission.as_deref(), Some("배포해라"));
        assert_eq!(rec.anomalies.len(), 1);
        assert_eq!(
            rec.extra.get("future_field").and_then(|v| v["keep"].as_str()),
            Some("me"),
            "모르는 필드가 사라졌다 — 두 벌이 쓰는 파일에서 조용한 손실"
        );
        // 다시 쓴 바이트에도 그대로 있다.
        let s = rec.to_json();
        assert!(s.contains("\"future_field\""), "재기록에서 모르는 필드가 빠졌다: {s}");
        assert!(s.contains("\"배포해라\""), "ensure_ascii=False 파리티가 깨졌다: {s}");

        // 3상 — 부재·손상·형식 오류를 융합하지 않는다.
        assert_eq!(parse_ledger(None), LedgerRead::Absent);
        assert!(matches!(parse_ledger(Some(b"{oops")), LedgerRead::Unreadable(_)));
        assert!(
            matches!(parse_ledger(Some(b"[1,2]")), LedgerRead::Unreadable(_)),
            "dict 아닌 대장이 정상으로 접혔다"
        );
        // 빈 문자열 mission 은 **살아 있는 임무가 아니다**(python falsy 파리티).
        let LedgerRead::Ok(_) = parse_ledger(Some(
            r#"{"schema":1,"mission":"","source":"prompt","reason":"r","surface":"101",
                "ts":"t","ts_epoch":1.0,"boot_epoch":null,"ledger_status":"ok","anomalies":[]}"#.as_bytes(),
        )) else {
            panic!("빈 임무 대장이 판독 불가로 접혔다");
        };
        assert!(
            !parse_ledger(Some(
                r#"{"schema":1,"mission":"","source":"prompt","reason":"r","surface":"101",
                    "ts":"t","ts_epoch":1.0,"boot_epoch":null,"ledger_status":"ok","anomalies":[]}"#.as_bytes()
            ))
            .has_live_mission(),
            "빈 문자열 임무가 '살아 있는 임무'로 잡혔다 — python 은 falsy 다"
        );
    }

    /// ★H-MISSION-R5-i: 이상징후 병합의 중복 제거·순서·상한.
    #[test]
    fn merge_anomalies_dedups_preserves_order_and_drops_oldest() {
        let rec: Vec<Anomaly> = vec![Anomaly::new("a", "1"), Anomaly::new("b", "2")];
        let obs = vec![
            ("b".to_string(), "2".to_string()),   // 중복 — 빠진다
            ("b".to_string(), "3".to_string()),   // 같은 코드·다른 사유 — 남는다(사유가 근거다)
            ("c".to_string(), "4".to_string()),
        ];
        let m = merge_anomalies(&rec, &obs);
        assert_eq!(
            m.iter().map(|a| (a.code.as_str(), a.detail.as_str())).collect::<Vec<_>>(),
            vec![("a", "1"), ("b", "2"), ("b", "3"), ("c", "4")],
            "중복 제거·순서 보존이 깨졌다"
        );
        // 상한 초과 시 **오래된 것부터** 밀린다.
        let many: Vec<(String, String)> =
            (0..ANOMALY_KEEP + 10).map(|i| ("x".to_string(), i.to_string())).collect();
        let m = merge_anomalies(&[], &many);
        assert_eq!(m.len(), ANOMALY_KEEP);
        assert_eq!(m[0].detail, "10", "밀려난 쪽이 최신이다 — 순서가 뒤집혔다");
    }

    /// ★H-MISSION-R5-j: 빈 프롬프트는 판정 자체를 하지 않는다(대장 무변경).
    #[test]
    fn blank_prompt_decides_nothing() {
        for s in ["", "   ", "\n\t "] {
            let d = fold_absent(s, &live_ledger("임무"));
            assert_eq!(d.fold, RecordFold::EmptyPrompt, "{s:?}");
            assert_eq!(d.plan, LedgerPlan::Nothing, "빈 프롬프트가 대장을 건드렸다: {s:?}");
        }
    }

}
