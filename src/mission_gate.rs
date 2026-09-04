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
        let t = &it["text"];
        if let Some(s) = t.as_str() {
            return s.to_string();
        }
        let r = &t["repeat"];
        let unit = r["unit"].as_str().expect("repeat.unit");
        let times = r["times"].as_u64().expect("repeat.times") as usize;
        unit.repeat(times)
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
}
