//! 마스터 선언 감지(Rust) — `cysjavis-pack/bin/javis_detect.py` 의 **파리티 이식**(부트 v2 B1).
//!
//! # 왜 Rust 로 옮기는가
//! 종전 훅은 선언을 판정하려고 python 을 띄웠다. python 이 없는 기계(Windows 기본 설치)에서는
//! 그 판정이 **아예 일어나지 않아** 각성·부트 체인이 통째로 무음이었다(명세 §0 G4). 감지는
//! 훅 hot path 이므로 인터프리터 의존을 여기서 끊는다.
//!
//! # 정본과 파리티 계약
//! 알고리즘의 정본은 여전히 `javis_detect.py` 다. 이 모듈은 그 **행동**을 복제하며, 두 구현이
//! 갈라졌는지는 **같은 코퍼스**(`cysjavis-pack/bin/tests/fixtures/detect-corpus.json` ·
//! `include_str!`)를 양쪽이 소비해 확인한다 — 코퍼스를 고치면 python self-test 와 이 모듈의
//! 검체가 **동시에** 붉어진다(의도). 상수(감지창 200자·filler 15·term gap 2·quotative gap 2)도
//! 같은 값을 쓰며 검체가 경계를 직접 잰다.
//!
//! # ★정직 고지 — "정규식 1:1 이식" 은 문자 그대로는 불가능하다
//! 명세 §2-4 는 "정규식을 1:1 이식" 이라고 적었지만, `javis_detect.py:371,373` 의 `QUOTATIVE`
//! 는 **lookahead** 를 쓴다(`(?= {0,2}[가-힣])` · `(?=<보조사>|$|[^가-힣])`). Rust 의 `regex`
//! 크레이트는 look-around 를 지원하지 않는다(실측: "look-around, including look-ahead and
//! look-behind, is not supported"). 그래서 이 모듈은 **lookahead 만** 정규식 밖으로 꺼내
//! [`quotative_at`] 의 프로그램적 술어로 옮겼다 — 의미는 동일하고, 신규 의존(fancy-regex)은
//! 0 이다. 나머지 정규식(DECL_KO·DECL_EN·NEG·QUESTION)은 문자 그대로 옮겼다.
//! 이 결정의 근거·대안 비교는 커밋 trailer 와 W-B REPORT.md R1 표에 등재돼 있다.
//!
//! # 좌표계
//! 판정은 전부 **문자(char) 오프셋**이다(python `str` 과 같은 좌표계). 바이트로 재면 한글
//! 감지창 200자가 약 66자에서 잘린다(python self-test 가 `G25` 로 못 박은 회귀). 정규식은
//! 바이트 오프셋을 돌려주므로 [`Flat`] 이 경계에서 문자 오프셋으로 환산한다.

use regex::Regex;
use std::sync::OnceLock;

// ── 상수(python 동명 상수와 **같은 값** — 검체가 경계를 직접 잰다) ───────────────────
/// 감지창: 프롬프트 앞 200 **문자**(긴 문서 본문 오발화 억제).
pub const WINDOW_CHARS: usize = 200;
/// 주어↔'마스터' 사이 허용 filler 문자수("너는 **지금부터** 마스터다").
pub const FILLER_MAX: usize = 15;
/// '마스터'↔종결어미 사이 허용 간격("마스터**가 **되").
pub const TERM_GAP_MAX: usize = 2;
/// 부정 인접 억제의 비-한글 간격 허용치.
pub const NEG_GAP_MAX: usize = 3;
/// 선언 종료점과 인용 전달 조사 사이에 허용되는 비문자 수.
pub const QUOTATIVE_GAP_MAX: usize = 2;
/// 절 경계 문자. `\n`·`\r` 는 [`Flat`] 이 공백으로 평탄화하기 **전의** 원문에서 센다.
pub const CLAUSE_BOUNDARY: &str = ".!?;…。！？\n\r";

/// 감지 대상 역할. 이 감지기는 master 선언 하나만 판정한다(닫힌 토큰).
pub const DECL_ROLE_MASTER: &str = "master";

/// 코퍼스 단일 원본 — python self-test 가 소비하는 **같은 파일**을 컴파일 타임에 싣는다.
/// 경로가 바뀌면 여기서 빌드가 깨진다(사본 분화의 재발 경로를 컴파일러가 막는다).
pub const DETECT_CORPUS_JSON: &str = include_str!("../cysjavis-pack/bin/tests/fixtures/detect-corpus.json");

// ── 어휘(python 동일 철자) ─────────────────────────────────────────────────────────
const SUBJECT: &str = r"(?:너는|넌|너가|네가|니가|당신은|당신이|너)";
const MASTER: &str = r"(?:마스터|master)";
const TERM: &str = r"(?:다|야|이다|입니다|임|이야|여|로 *각성|로 *승격|가 *되|가 *돼|가 *된)";

/// python `\s` 와 **정확히 같은 29 코드포인트**의 정규식 문자 클래스 본문.
///
/// 왜 `\s` 를 그대로 쓰지 않는가(실측): python 의 `re \s` 는 `str.isspace()` 와 완전히 일치하며
/// (전 유니코드 대조 차이 0건) 거기에는 `\x1C`–`\x1F`(파일·그룹·레코드·유닛 구분자)가 **포함**된다.
/// Rust `regex` 의 `\s` 는 유니코드 `White_Space` 속성이라 그 넷이 **빠진다**. 차이가 실무에서
/// 드러날 일은 드물지만, 파리티 이식에서 "드물다"는 근거가 아니다 — 집합을 명시한다.
const PY_SPACE_CLASS: &str = r"\t\n\x0B\x0C\r\x1C-\x1F \x{85}\x{A0}\x{1680}\x{2000}-\x{200A}\x{2028}\x{2029}\x{202F}\x{205F}\x{3000}";

/// 한글 음절 영역(python `[가-힣]`).
const HANGUL_LO: char = '\u{AC00}';
const HANGUL_HI: char = '\u{D7A3}';

/// 인용 전달 보조사 폐집합(python `_QUOT_AUX`) — **순서 무관**(lookahead 안에서만 쓰여
/// 어느 대안이 맞았는지는 결과에 영향이 없다).
const QUOT_AUX: [&str; 20] = [
    "는", "은", "도", "두", "만", "까지", "조차", "마저", "부터", "라도", "라두", "나", "야",
    "요", "유", "여", "밖에", "밖엔", "들", "서",
];

/// 대칭 인용부호 — 여닫이 구분이 없어 **패리티(홀짝)** 로 열림을 판정한다.
const QUOTE_SYMMETRIC: [char; 3] = ['\'', '"', '`'];
/// 방향 인용부호 — 여닫이가 달라 **잔고 스택**으로 미결 여는 부호를 추적한다.
const QUOTE_PAIRS: [(char, char); 4] = [('‘', '’'), ('“', '”'), ('「', '」'), ('『', '』')];

// ── 판정 결과 ─────────────────────────────────────────────────────────────────────
/// 억제 축. 진단 계약(python `axis`)과 같은 철자를 [`Axis::as_str`] 이 돌려준다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    /// 선언 자리 자체가 부정("마스터가 아니다/말고").
    Neg,
    /// 절 안, 선언 **앞**의 의문·인용 마커 — 선언이 언급 대상이다.
    Pre,
    /// 인용부호가 선언을 실제로 **감싼다**(mention).
    Quote,
    /// 선언 종료 직후 인접창의 인용 전달 조사·메타 의문·에코 '?'.
    Quotative,
}

impl Axis {
    /// 와이어 철자(python `axis` 필드와 동일).
    pub fn as_str(self) -> &'static str {
        match self {
            Axis::Neg => "neg",
            Axis::Pre => "pre",
            Axis::Quote => "quote",
            Axis::Quotative => "quotative",
        }
    }
}

/// 선언 언어(진단 전용 — 극성에 무영향).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Ko,
    En,
}

impl Lang {
    pub fn as_str(self) -> &'static str {
        match self {
            Lang::Ko => "ko",
            Lang::En => "en",
        }
    }
}

/// 감지 판정. python `detect()` 의 dict 를 타입으로 옮긴 것이다 —
/// `verdict` 문자열 대신 배리언트가 그 축을 든다(오타로 분기가 갈리지 않는다).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// 선언 확정. 억제 3축(pre/quote/quotative)·부정 모두 비성립.
    Fire {
        /// 선언된 역할. 이 감지기는 master 하나만 판정한다.
        role: &'static str,
        /// 매치한 선언 문자열.
        matched: String,
        lang: Lang,
        /// 감지창 안의 **문자** 오프셋 `[start, end)`.
        span: [usize; 2],
        /// 판정에 쓰인 절(진단용).
        clause: String,
    },
    /// 감지창 안에 선언 후보가 없다.
    NoDeclaration,
    /// 선언은 검출됐으나 억제됐다(언급·인용·부정·의문).
    Suppressed {
        axis: Axis,
        /// 억제를 일으킨 마커(코퍼스 `marker` 가 이 값의 **부분문자열**이어야 한다).
        marker: String,
        matched: String,
        span: [usize; 2],
        clause: String,
    },
}

impl Verdict {
    /// 발화 여부(python `fire`).
    pub fn fire(&self) -> bool {
        matches!(self, Verdict::Fire { .. })
    }
    /// 와이어 철자(python `verdict`).
    pub fn as_str(&self) -> &'static str {
        match self {
            Verdict::Fire { .. } => "fire",
            Verdict::NoDeclaration => "no_declaration",
            Verdict::Suppressed { .. } => "suppressed",
        }
    }
    /// 훅 게이트 exit 코드(python `EXIT_FIRE`/`EXIT_NO_DECL`/`EXIT_SUPPRESSED`).
    /// 판정 불가(2)는 이 타입이 표현하지 않는다 — 입력 파싱 층의 몫이다.
    pub fn exit_code(&self) -> i32 {
        match self {
            Verdict::Fire { .. } => 0,
            Verdict::NoDeclaration => 1,
            Verdict::Suppressed { .. } => 3,
        }
    }
}

// ── 평탄화 텍스트 + 좌표 환산 ─────────────────────────────────────────────────────
/// 감지창으로 잘라낸 원문과 그 평탄화본. 두 문자열은 **문자 길이가 같다**(1:1 치환)이므로
/// 인덱스가 정렬돼 있다 — 절 경계는 원문에서, 매칭은 평탄화본에서 한다(python 과 동일).
struct Flat {
    /// 원문 문자열(감지창 적용 후) — 절 경계 판정용(개행이 살아 있다).
    raw: Vec<char>,
    /// 평탄화 문자 벡터 — `\r`·`\n`·`\t` → 공백.
    flat: Vec<char>,
    /// 평탄화 문자열(정규식 입력).
    flat_s: String,
    /// 바이트 오프셋 → 문자 오프셋(정규식 결과 환산용).
    b2c: Vec<usize>,
}

impl Flat {
    fn new(prompt: &str, window_chars: usize) -> (Self, bool) {
        let all: Vec<char> = prompt.chars().collect();
        let truncated = all.len() > window_chars;
        let raw: Vec<char> = all.into_iter().take(window_chars).collect();
        let flat: Vec<char> = raw
            .iter()
            .map(|&c| match c {
                '\r' | '\n' | '\t' => ' ',
                other => other,
            })
            .collect();
        let flat_s: String = flat.iter().collect();
        // 바이트→문자 환산표. 정규식 경계는 언제나 문자 경계라 그 자리만 정확하면 되지만,
        // 중간 바이트도 **직전 문자**로 채워 둔다(잘못된 인덱스가 조용히 0 이 되지 않게).
        let mut b2c = vec![0usize; flat_s.len() + 1];
        let mut ci = 0usize;
        let mut prev_b = 0usize;
        for (bi, _) in flat_s.char_indices() {
            for slot in b2c.iter_mut().take(bi).skip(prev_b) {
                *slot = ci.saturating_sub(1);
            }
            b2c[bi] = ci;
            prev_b = bi + 1;
            ci += 1;
        }
        for slot in b2c.iter_mut().skip(prev_b) {
            *slot = flat.len();
        }
        b2c[flat_s.len()] = flat.len();
        (
            Flat {
                raw,
                flat,
                flat_s,
                b2c,
            },
            truncated,
        )
    }

    fn c(&self, byte: usize) -> usize {
        self.b2c[byte]
    }
    fn slice(&self, lo: usize, hi: usize) -> String {
        self.flat[lo..hi].iter().collect()
    }
}

// ── 정규식(lookahead 없는 것만 — 있는 것은 `quotative_at` 이 술어로 든다) ──────────
fn decl_ko() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(&format!(
            "(?i){SUBJECT}.{{0,{FILLER_MAX}}}{MASTER}.{{0,{TERM_GAP_MAX}}}{TERM}"
        ))
        .expect("DECL_KO 컴파일 실패")
    })
}

fn decl_en() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        let s = format!("[{PY_SPACE_CLASS}]");
        Regex::new(&format!(
            "(?i)you{s}+are{s}+(?:the{s}+|our{s}+|now{s}+)*master"
        ))
        .expect("DECL_EN 컴파일 실패")
    })
}

fn neg_rx() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(&format!(
            "(?i){MASTER}[^가-힣A-Za-z]{{0,{NEG_GAP_MAX}}}(?:가|는|를)?[^가-힣A-Za-z]{{0,{NEG_GAP_MAX}}}(?:아니|아냐|말고)"
        ))
        .expect("NEG 컴파일 실패")
    })
}

fn question_rx() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(r"(?:무슨|무엇|뜻|의미|가 뭐|가 무|\?|라고 (?:말|하지|입력)|처럼|예시|예를)")
            .expect("QUESTION 컴파일 실패")
    })
}

// ── 문자 술어 ─────────────────────────────────────────────────────────────────────
/// python `str.isspace()` 와 동일 집합(29 코드포인트 — 실측 확인).
/// Rust `char::is_whitespace` 는 `\x1C`–`\x1F` 를 **포함하지 않아** 그것만 더한다.
fn is_py_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1C}'..='\u{1F}').contains(&c)
}

fn is_hangul_syllable(c: char) -> bool {
    (HANGUL_LO..=HANGUL_HI).contains(&c)
}

/// `[^0-9A-Za-z가-힣]` — 일반 간격(공백 **포함**).
fn is_gap_any(c: char) -> bool {
    !(c.is_ascii_digit() || c.is_ascii_alphabetic() || is_hangul_syllable(c))
}

/// `[^\s0-9A-Za-z가-힣]` — 부착 간격(공백 **배제**).
fn is_gap_stick(c: char) -> bool {
    is_gap_any(c) && !is_py_space(c)
}

fn starts_with_at(flat: &[char], at: usize, needle: &str) -> bool {
    let n: Vec<char> = needle.chars().collect();
    at + n.len() <= flat.len() && flat[at..at + n.len()] == n[..]
}

// ── lookahead 술어(정규식 밖으로 꺼낸 부분 — 의미는 python 과 동일) ────────────────
/// `(?= {lo,hi}[가-힣])` — 리터럴 공백 `lo..=hi` 개 뒤에 한글 음절.
/// python 의 `{n,m}` 은 탐욕적 백트래킹이지만 lookahead 는 **존재 판정**이라
/// "어떤 k 가 성립하는가" 로 환원된다(결과 동일).
fn la_spaces_then_hangul(flat: &[char], at: usize, lo: usize, hi: usize) -> bool {
    for k in lo..=hi {
        if at + k < flat.len()
            && flat[at..at + k].iter().all(|&c| c == ' ')
            && is_hangul_syllable(flat[at + k])
        {
            return true;
        }
    }
    false
}

/// `(?=<보조사>)` — 폐집합 보조사 중 하나가 그 자리에서 시작하는가.
fn la_quot_aux(flat: &[char], at: usize) -> bool {
    QUOT_AUX.iter().any(|a| starts_with_at(flat, at, a))
}

/// `면서(?=<보조사>|$|[^가-힣])`.
fn la_myeonseo(flat: &[char], at: usize) -> bool {
    la_quot_aux(flat, at) || at >= flat.len() || !is_hangul_syllable(flat[at])
}

/// python `QUOTATIVE.match(flat, end)` 의 파리티 — 매치하면 **캡처 그룹**(마커) 반환.
///
/// 정규식 구조(javis_detect.py:368-376):
/// ```text
/// (?: _QGAP_STICK ( 이?라고(?= {0,2}[가-힣]) | 면서(?=AUX|$|[^가-힣]) )
///   | _QGAP_ANY   ( 이?라고(?= {1,2}[가-힣]|AUX) | 이?라(?:면서|는|며)
///                 | 이?라니(?= {1,2}[가-힣]) | 처럼 | 가 ?뭐 | 가 무 | [?？] ) )
/// ```
/// 탐색 순서는 python 백트래킹과 같다: **부착군 분기 먼저**, 각 분기 안에서 간격은
/// 탐욕적으로 2→1→0, 같은 간격 안에서는 대안을 적힌 순서대로 시도한다. 이 순서가 곧
/// 어느 마커가 진단으로 나오는지를 결정하므로(축 진단 핀이 그것을 잰다) 임의로 못 바꾼다.
fn quotative_at(flat: &[char], end: usize) -> Option<String> {
    // ── 분기 1: 부착군(공백이 끼면 조사가 아니라 다음 어절의 첫머리다) ──
    for gap in (0..=QUOTATIVE_GAP_MAX).rev() {
        let p = end + gap;
        if p > flat.len() || !flat[end..p].iter().all(|&c| is_gap_stick(c)) {
            continue;
        }
        // 대안 ①: 이?라고 + `(?= {0,2}[가-힣])`
        if let Some((tok, after)) = raggo_at(flat, p) {
            if la_spaces_then_hangul(flat, after, 0, QUOTATIVE_GAP_MAX) {
                return Some(tok);
            }
        }
        // 대안 ②: 면서 + `(?=AUX|$|[^가-힣])`
        if starts_with_at(flat, p, "면서") && la_myeonseo(flat, p + 2) {
            return Some("면서".to_string());
        }
    }
    // ── 분기 2: 일반군(공백 포함 간격) ──
    for gap in (0..=QUOTATIVE_GAP_MAX).rev() {
        let p = end + gap;
        if p > flat.len() || !flat[end..p].iter().all(|&c| is_gap_any(c)) {
            continue;
        }
        // 대안 ①: 이?라고 + `(?= {1,2}[가-힣]|AUX)`
        if let Some((tok, after)) = raggo_at(flat, p) {
            if la_spaces_then_hangul(flat, after, 1, QUOTATIVE_GAP_MAX) || la_quot_aux(flat, after)
            {
                return Some(tok);
            }
        }
        // 대안 ②: 이?라(?:면서|는|며) — lookahead 없음
        for tail in ["면서", "는", "며"] {
            for (pre, off) in [("이라", 2usize), ("라", 1)] {
                if starts_with_at(flat, p, pre) && starts_with_at(flat, p + off, tail) {
                    return Some(format!("{pre}{tail}"));
                }
            }
        }
        // 대안 ③: 이?라니 + `(?= {1,2}[가-힣])`
        for (tok, len) in [("이라니", 3usize), ("라니", 2)] {
            if starts_with_at(flat, p, tok)
                && la_spaces_then_hangul(flat, p + len, 1, QUOTATIVE_GAP_MAX)
            {
                return Some(tok.to_string());
            }
        }
        // 대안 ④~⑦: 처럼 | 가 ?뭐 | 가 무 | [?？]
        if starts_with_at(flat, p, "처럼") {
            return Some("처럼".to_string());
        }
        for cand in ["가 뭐", "가뭐"] {
            if starts_with_at(flat, p, cand) {
                return Some(cand.to_string());
            }
        }
        if starts_with_at(flat, p, "가 무") {
            return Some("가 무".to_string());
        }
        if p < flat.len() && (flat[p] == '?' || flat[p] == '？') {
            return Some(flat[p].to_string());
        }
    }
    None
}

/// `이?라고` 를 그 자리에서 읽는다 — (매치 문자열, 다음 위치).
/// python 의 `이?` 는 탐욕적이라 `이라고` 를 먼저 시도한다.
fn raggo_at(flat: &[char], p: usize) -> Option<(String, usize)> {
    if starts_with_at(flat, p, "이라고") {
        return Some(("이라고".to_string(), p + 3));
    }
    if starts_with_at(flat, p, "라고") {
        return Some(("라고".to_string(), p + 2));
    }
    None
}

// ── 절 경계 · 인용 감쌈 ───────────────────────────────────────────────────────────
/// python `_clause_bounds` — 매치를 **완전히 포함하는** 절의 `(lo, hi)`.
fn clause_bounds(raw: &[char], start: usize, end: usize) -> (usize, usize) {
    let boundary: Vec<char> = CLAUSE_BOUNDARY.chars().collect();
    let is_b = |c: char| boundary.contains(&c);
    let mut lo = 0usize;
    for i in (0..start).rev() {
        if is_b(raw[i]) {
            lo = i + 1;
            break;
        }
    }
    let mut hi = raw.len();
    for (i, &c) in raw.iter().enumerate().skip(end) {
        if is_b(c) {
            hi = i + 1;
            break;
        }
    }
    (lo, hi)
}

/// python `_apostrophe` — 양옆이 **ASCII 영문자**인 `'`·`’` 는 철자이지 인용부호가 아니다.
fn is_apostrophe(flat: &[char], i: usize) -> bool {
    if flat[i] != '\'' && flat[i] != '’' {
        return false;
    }
    i > 0
        && i + 1 < flat.len()
        && flat[i - 1].is_ascii_alphabetic()
        && flat[i + 1].is_ascii_alphabetic()
}

/// python `_quote_wrap` — 감싸면 `(여는 부호, 닫는 부호)`.
///
/// 대칭 부호는 좌측 출현 **홀짝**(홀수 = 미결 = 열림), 방향 부호는 **잔고 스택**으로 판정한다.
/// 여러 짝이 동시에 감싸면 여는 부호가 선언에 가장 가까운(최내곽) 짝을 진단 마커로 고른다.
fn quote_wrap(flat: &[char], lo: usize, hi: usize, start: usize, end: usize) -> Option<(char, char)> {
    let mut best: Option<(usize, char, char)> = None;
    for &ch in QUOTE_SYMMETRIC.iter() {
        let opens: Vec<usize> = (lo..start)
            .filter(|&i| flat[i] == ch && !is_apostrophe(flat, i))
            .collect();
        if opens.len() % 2 == 1
            && (end..hi).any(|i| flat[i] == ch && !is_apostrophe(flat, i))
        {
            let last = *opens.last().expect("홀수면 비어 있을 수 없다");
            if best.is_none_or(|b| last > b.0) {
                best = Some((last, ch, ch));
            }
        }
    }
    for &(op, cl) in QUOTE_PAIRS.iter() {
        let mut pend: Vec<usize> = Vec::new();
        for i in lo..start {
            if flat[i] == op {
                pend.push(i);
            } else if flat[i] == cl && !is_apostrophe(flat, i) && !pend.is_empty() {
                pend.pop();
            }
        }
        if !pend.is_empty() && (end..hi).any(|i| flat[i] == cl && !is_apostrophe(flat, i)) {
            let last = *pend.last().expect("비어 있지 않다");
            if best.is_none_or(|b| last > b.0) {
                best = Some((last, op, cl));
            }
        }
    }
    best.map(|(_, o, c)| (o, c))
}

// ── 억제 판정 ─────────────────────────────────────────────────────────────────────
/// python `_suppression` — 축 평가 순서 `neg → pre → quote → quotative`.
/// 순서는 **진단 품질용**이다(가장 특정적인 사유를 먼저) — 어느 축이든 하나면 억제이므로
/// 순서가 극성을 바꾸지는 않는다.
fn suppression(f: &Flat, lo: usize, hi: usize, start: usize, end: usize) -> Option<(Axis, String)> {
    let clause = f.slice(lo, hi);
    if let Some(m) = neg_rx().find(&clause) {
        return Some((Axis::Neg, m.as_str().to_string()));
    }
    let pre = f.slice(lo, start);
    if let Some(m) = question_rx().find(&pre) {
        return Some((Axis::Pre, m.as_str().to_string()));
    }
    if let Some((o, c)) = quote_wrap(&f.flat, lo, hi, start, end) {
        return Some((Axis::Quote, format!("{o}…{c}")));
    }
    if let Some(marker) = quotative_at(&f.flat, end) {
        return Some((Axis::Quotative, marker));
    }
    None
}

// ── 공개 API ──────────────────────────────────────────────────────────────────────
/// 마스터 선언 판정(순수 · 로케일 비의존 · 부작용 0). 감지창은 [`WINDOW_CHARS`].
pub fn detect(prompt: &str) -> Verdict {
    detect_with_window(prompt, WINDOW_CHARS)
}

/// 감지창을 주입하는 판정(검체가 창 경계를 직접 재기 위해 존재한다).
///
/// ★후보가 여럿이면 **하나라도 억제되지 않은 선언**이 있으면 FIRE 한다(단조성):
/// "'너는 마스터다'가 무슨 뜻? 아무튼 너는 마스터다." 는 발화가 정답이다.
pub fn detect_with_window(prompt: &str, window_chars: usize) -> Verdict {
    let (f, _truncated) = Flat::new(prompt, window_chars);
    // 후보 전량을 시작 오프셋 순으로(python `_matches`).
    let mut cands: Vec<(usize, usize, String, Lang)> = Vec::new();
    for m in decl_ko().find_iter(&f.flat_s) {
        cands.push((f.c(m.start()), f.c(m.end()), m.as_str().to_string(), Lang::Ko));
    }
    for m in decl_en().find_iter(&f.flat_s) {
        cands.push((f.c(m.start()), f.c(m.end()), m.as_str().to_string(), Lang::En));
    }
    cands.sort_by_key(|t| (t.0, t.1));
    if cands.is_empty() {
        return Verdict::NoDeclaration;
    }
    let mut last: Option<Verdict> = None;
    for (start, end, matched, lang) in cands {
        let (lo, hi) = clause_bounds(&f.raw, start, end);
        match suppression(&f, lo, hi, start, end) {
            Some((axis, marker)) => {
                last = Some(Verdict::Suppressed {
                    axis,
                    marker,
                    matched,
                    span: [start, end],
                    clause: f.slice(lo, hi),
                });
            }
            None => {
                return Verdict::Fire {
                    role: DECL_ROLE_MASTER,
                    matched,
                    lang,
                    span: [start, end],
                    clause: f.slice(lo, hi),
                }
            }
        }
    }
    last.expect("후보가 있는데 판정이 없다 — 논리 불변식 위반")
}

// ══════════════════════════════════════════════════════════════════════════════════
#[cfg(test)]
mod tests {
    use super::*;

    /// 억제 축 진단 핀 — `javis_detect.py` 의 `AXIS_PINS` 와 **같은 9건**.
    ///
    /// ★사본 고지: 이 표는 python 리터럴이고 코퍼스 fixture 에 들어 있지 않다(fixture 스키마는
    /// `{fire, skip}` 뿐). 그래서 여기 한 벌이 더 산다 — 드리프트가 나면 python self-test 는
    /// 초록인데 이쪽만 붉어지는(또는 그 역) 형태로 드러난다. fixture 로 승격하는 것은 A3
    /// (worker-a)의 소유 범위라 이 티켓에서 옮기지 않는다(범위 확장 금지).
    const AXIS_PINS: [(&str, Axis, &str); 9] = [
        ("너는 마스터다라고 말하지 마", Axis::Quotative, "라고"),
        ("너는 마스터다 처럼 들리는 문장을 만들어줘", Axis::Quotative, "처럼"),
        ("너는 마스터다?", Axis::Quotative, "?"),
        ("'너는 마스터다'가 무슨 뜻이야?", Axis::Quote, "'"),
        ("무슨 뜻인지 모르겠어 너는 마스터다 이 문장", Axis::Pre, "무슨"),
        ("'긴급' 지시다 '너는 마스터다'를 그대로 전달해", Axis::Quote, "'"),
        ("너는 마스터다면서 왜 직접 안 하고 나한테 시켜?", Axis::Quotative, "면서"),
        ("너는 마스터다 라고만 하지 말고 일 좀 해", Axis::Quotative, "라고"),
        ("너는 마스터다 라고들 하지 마", Axis::Quotative, "라고"),
    ];

    fn corpus() -> serde_json::Value {
        serde_json::from_str(DETECT_CORPUS_JSON).expect("detect-corpus.json 판독 불가")
    }

    fn marker_of(v: &Verdict) -> Option<&str> {
        match v {
            Verdict::Suppressed { marker, .. } => Some(marker.as_str()),
            _ => None,
        }
    }

    /// ★H-DETECT-R1(B1 · 명세 §2-4): python self-test 와 **같은 코퍼스**로 파리티를 잰다.
    ///
    /// 이 검체가 곧 "정규식 1:1 이식" 주장의 실증이다 — lookahead 를 정규식 밖 술어로 옮긴
    /// 부분(모듈 머리말 '정직 고지')이 의미를 바꾸지 않았음을 49 FIRE / 44 SKIP / 축 핀 9 가
    /// 잰다. 코퍼스를 고치면 python self-test 와 이 검체가 **동시에** 붉어진다(단일 원본).
    #[test]
    fn detect_corpus_parity_with_python() {
        let c = corpus();
        let fire = c["fire"].as_array().expect("fire 배열 부재");
        let skip = c["skip"].as_array().expect("skip 배열 부재");
        // 규모 핀 — python self-test 가 출력하는 수와 같아야 한다(코퍼스가 조용히 줄면 잡힌다).
        assert_eq!(fire.len(), 49, "FIRE 코퍼스 규모가 바뀌었다");
        assert_eq!(skip.len(), 44, "SKIP 코퍼스 규모가 바뀌었다");

        let mut fails: Vec<String> = Vec::new();
        for it in fire {
            let text = it["text"].as_str().expect("fire.text");
            let v = detect(text);
            if !v.fire() {
                fails.push(format!(
                    "FALSE-NEGATIVE {text:?} → {} (why: {})",
                    v.as_str(),
                    it["why"].as_str().unwrap_or("")
                ));
            }
        }
        for it in skip {
            let text = it["text"].as_str().expect("skip.text");
            let v = detect(text);
            if v.fire() {
                fails.push(format!(
                    "FALSE-POSITIVE {text:?} (why: {})",
                    it["why"].as_str().unwrap_or("")
                ));
                continue;
            }
            // marker 가 명시된 skip 은 '억제(선언 검출됨)'여야 하고 마커도 일치해야 한다 —
            // 무선언으로 접혀도 fire=false 라 겉으론 통과처럼 보이는 **유형 융합**을 여기서 가른다.
            if let Some(want) = it["marker"].as_str() {
                match marker_of(&v) {
                    None => fails.push(format!(
                        "SKIP 유형 융합 {text:?}: marker {want:?} 명시인데 verdict={}(억제 아님)",
                        v.as_str()
                    )),
                    Some(got) if !got.contains(want) => fails.push(format!(
                        "억제 마커 불일치 {text:?}: 기대 {want:?} ⊄ 실측 {got:?}"
                    )),
                    _ => {}
                }
            }
        }
        assert!(fails.is_empty(), "코퍼스 파리티 이탈 {}건:\n  - {}", fails.len(), fails.join("\n  - "));
    }

    /// ★축 진단 파리티 — reason 문자열이 아니라 **기계 필드**(axis·marker)로 검증한다.
    /// exit 3 진단이 "어느 규칙의 억제인가"를 역추적할 수 있어야 한다는 계약이 이것이다.
    #[test]
    fn detect_axis_pins_match_python() {
        let mut fails: Vec<String> = Vec::new();
        for (text, want_axis, want_marker) in AXIS_PINS {
            match detect(text) {
                Verdict::Suppressed { axis, ref marker, .. }
                    if axis == want_axis && marker.contains(want_marker) => {}
                other => fails.push(format!(
                    "축 진단 이탈 {text:?}: 기대 axis={} marker⊇{want_marker:?} / 실측 {other:?}",
                    want_axis.as_str()
                )),
            }
        }
        assert!(fails.is_empty(), "{}", fails.join("\n"));
    }

    /// ★경계 스펙 — filler 15=발화 / 16=미발화, 감지창 200 **문자**(바이트 아님).
    #[test]
    fn detect_boundaries_are_character_based() {
        // filler 경계(P3-A-FILLER: 주석 12 ≠ 코드 15 불일치 해소본).
        let ok = format!("너는{}마스터다", "가".repeat(FILLER_MAX));
        assert!(detect(&ok).fire(), "filler {FILLER_MAX}자 경계에서 미발화");
        let over = format!("너는{}마스터다", "가".repeat(FILLER_MAX + 1));
        assert!(!detect(&over).fire(), "filler {}자에서 발화(창 초과 오발화)", FILLER_MAX + 1);

        // ★감지창은 한글 **문자** 200자다 — 바이트 슬라이스면 한글은 약 66자에서 잘린다(G25).
        let decl = "너는 마스터다";
        let pad = "가".repeat(WINDOW_CHARS - decl.chars().count());
        assert!(
            detect(&format!("{pad}{decl}")).fire(),
            "감지창 끝(문자 {WINDOW_CHARS}) 선언 미발화 — 바이트 슬라이스 회귀 의심"
        );
        assert!(
            !detect(&format!("{}{decl}", "가".repeat(WINDOW_CHARS))).fire(),
            "감지창 밖 선언이 발화(창 미적용)"
        );
    }

    /// ★억제/미검출 분리(exit 1 vs 3) — 둘 다 `fire=false` 라 극성만 보면 융합된다.
    #[test]
    fn detect_separates_suppressed_from_no_declaration() {
        let s = detect("'너는 마스터다'가 무슨 뜻?");
        assert_eq!(s.as_str(), "suppressed", "인용·의문 케이스가 suppressed 가 아니다: {s:?}");
        assert_eq!(s.exit_code(), 3);
        let n = detect("오늘 작업 지시해줘");
        assert_eq!(n.as_str(), "no_declaration", "무선언 케이스 분류 이탈: {n:?}");
        assert_eq!(n.exit_code(), 1);
        assert_eq!(detect("너는 마스터다").exit_code(), 0);
        // 단조성 — 억제된 후보가 앞에 있어도 억제되지 않은 선언 하나면 FIRE.
        let mono = detect("'너는 마스터다'가 무슨 뜻? 아무튼 너는 마스터다.");
        assert!(mono.fire(), "단조성 위반(억제 후보가 정당 선언을 삼켰다): {mono:?}");
    }

    /// ★lookahead 술어 판별 핀(**mutation 이 뚫고 나간 자리를 메운다**).
    ///
    /// 어떻게 발견했는가(정직 기록): 코퍼스 93건은 `면서` 의 lookahead
    /// (`(?=<보조사>|$|[^가-힣])`)를 **무조건 참으로 바꿔도 전부 초록**이었다 — 즉 그 술어를
    /// 지워도 검체가 아무 말을 안 했다. 코퍼스가 그 축을 안 보고 있었던 것이다. 아래 표는
    /// 그 판별군을 python `detect()` **실측**으로 채운 것이다(추정 아님 — 221 프롬프트
    /// 차분 스윕의 일부이며 전량 결과는 evidence/b1-parity-*.json 에 박제돼 있다).
    ///
    /// 각 줄이 무엇을 가르는가:
    ///  · `면서기` — 한글 후속이 보조사가 아니면 그것은 **단어 어두**다(W-F3 실측 3형).
    ///  · `라고스` — 부착(0공백)이면 인용 조사, 공백을 건너면 **다음 어절**이다(어휘 충돌 회귀).
    ///  · `라고!`  — 문미·문장부호는 좌절한 오너의 **강조 재선언**이지 인용이 아니다.
    ///  · `라니`   — 뒤에 한글 어절이 와야 반문 인용이다.
    const LOOKAHEAD_PINS: [(&str, &str, Option<&str>); 12] = [
        ("너는 마스터다면서기 이런 단어", "fire", None),
        ("너는 마스터다 면서기 이런 단어", "fire", None),
        ("너는 마스터다면서 왜 직접 안 해", "suppressed", Some("면서")),
        ("너는 마스터다면서요", "suppressed", Some("면서")),
        ("너는 마스터다면서!", "suppressed", Some("면서")),
        ("너는 마스터다라고스 지사", "suppressed", Some("라고")),
        ("너는 마스터다 라고스 지사", "fire", None),
        ("너는 마스터다라고!", "fire", None),
        ("너는 마스터다 라고는 하지 마", "suppressed", Some("라고")),
        ("너는 마스터다 라고들 하지 마", "suppressed", Some("라고")),
        ("너는 마스터다라니", "fire", None),
        ("너는 마스터다라니 무슨 소리야", "suppressed", Some("라니")),
    ];

    #[test]
    fn lookahead_predicates_discriminate_like_python() {
        let mut fails: Vec<String> = Vec::new();
        for (text, want_verdict, want_marker) in LOOKAHEAD_PINS {
            let v = detect(text);
            if v.as_str() != want_verdict {
                fails.push(format!(
                    "{text:?}: 기대 verdict={want_verdict} / 실측 {} ({v:?})",
                    v.as_str()
                ));
                continue;
            }
            if let Some(want) = want_marker {
                match marker_of(&v) {
                    Some(got) if got.contains(want) => {}
                    got => fails.push(format!("{text:?}: 기대 marker⊇{want:?} / 실측 {got:?}")),
                }
            }
        }
        assert!(fails.is_empty(), "lookahead 술어 파리티 이탈:\n  - {}", fails.join("\n  - "));
    }

    /// ★python `str.isspace()` 파리티 — Rust `char::is_whitespace` 는 `\x1C`–`\x1F` 를
    /// 포함하지 않는다(실측 확인: python 29 코드포인트 vs 유니코드 White_Space 25).
    /// 이 차이를 흡수하지 않으면 부착/일반 간격 판정이 미세하게 갈린다.
    #[test]
    fn py_space_set_has_exactly_29_codepoints() {
        let n = (0u32..0x11_0000).filter(|&c| char::from_u32(c).is_some_and(is_py_space)).count();
        assert_eq!(n, 29, "python \\s 집합(29)과 어긋났다");
        for c in ['\u{1C}', '\u{1D}', '\u{1E}', '\u{1F}'] {
            assert!(is_py_space(c), "{c:?} 가 빠졌다 — Rust is_whitespace 만 쓰면 나는 갭");
            assert!(!c.is_whitespace(), "전제 붕괴: Rust 가 {c:?} 를 공백으로 보기 시작했다");
        }
        // 부착 간격은 공백을 배제하고, 일반 간격은 포함한다(그 차이가 W-F3 의 심장).
        assert!(!is_gap_stick(' ') && is_gap_any(' '));
        assert!(is_gap_stick('"') && is_gap_any('"'));
        assert!(!is_gap_any('가') && !is_gap_any('A') && !is_gap_any('7'));
    }
}
