//! 선언 기반 todo 상태(Declared State) 파서 — `DESIGN_declared-state.md` §4-1/§4-2 Rust 구현.
//!
//! todo 파일의 귀속·유효성을 **파일 안의 선언 한 줄**로만 판정한다(ADR-1). 파일명·경로·mtime은
//! 사람 편의·저장 위치·진단 정보일 뿐 **판정 입력이 아니다** — 이번 유령 todo 사고의 6개 실패가
//! 전부 "추론으로 메운 자리"에서 났기 때문이다.
//!
//! **lib 계층에 두는 이유**: 소비자가 `cysd` 데몬(`bin/cysd/governance.rs check_todo`)과 `cys`
//! 바이너리(`bin/cys.rs` cycle 저장검증) **둘**이다. 두 곳에 각자 구현하면 즉시 drift가 난다 —
//! 기존 코드가 PATH 합성을 `lib.rs`의 `runtime_prefixed_path`에 두고 *"공용 … 중복 구현 금지"*
//! 라고 못 박은 것과 **같은 패턴**이다.
//!
//! **계약의 SOT는 이 파일이 아니라 골든 픽스처다**(ADR-2). 같은 문법을 Python 정본
//! (`cysjavis-pack/bin/javis_todo_decl.py`)도 구현하며, 양 언어가 동일 픽스처
//! (`cysjavis-pack/bin/tests/fixtures/todo-decl/`)를 돌아 **문자열로 직렬화된 동일 판정**
//! (`counted|retired|foreign-scope|orphan-scope|unclaimed`)을 내는지 CI가 강제한다
//! (아래 `golden_fixture_parity`). 문서로 맞추면 반드시 어긋나므로 픽스처가 심판이다.
//!
//! **설계 프로토타입과의 의도적 차이 2건**: `_round/ghost-todo-fix/sim_declared_state.py`는
//! 문법을 처음 실측한 프로토타입이지 정본이 아니다. 두 지점에서 프로토타입은 골든 픽스처·
//! Python 정본과 어긋나며, 이 구현은 **픽스처를 따른다**.
//!   ① 후보 선별이 `contains("javis:todo")`라 `<!-- javis:todo-retired -->`가 "깨진 v1 선언"으로
//!      잡혀 G10이 무력화된다 → `javis:todo` 직후 **공백**을 요구한다(`is_decl_candidate`).
//!   ② 후행 텍스트를 `토큰 문법 위반: -->`으로 진단해 원인을 감춘다 → `-->`가 body에 남으면
//!      "문법 위반"으로 진단한다(G9의 취지가 원인 고지이므로).
//!
//! **W13 교정 2건(reviewer1 2차 BLOCK · master 심판 2026-07-26)** — 양 언어 동일 규칙이다.
//!   G10' 은퇴 마커 **줄 전체 앵커**(`is_retire_marker_line`) — 부분일치이던 종전 판정은
//!        평범한 머리말 산문 한 줄로 살아있는 파일을 은퇴시켰다(실측 2종).
//!   G12 ⑤ **미닫힘 펜스 회수**(`header_lines`) — 머리말 끝까지 닫히지 않은 펜스는 없었던
//!        것으로 재판정한다. 종전에는 인라인 삼중백틱 한 줄이 그 아래 선언을 삼켰다.
//!
//! **W15 교정 2건(reviewer1 3차 REVISE · master 심판 2026-07-26)** — 양 언어 동일 규칙이다.
//!   G12 ⑤' **회수 충돌 취소**(`header_lines`) — 회수 구간과 정상 구간에 둘 다 후보가 있으면
//!        회수를 취소한다. W13의 회수가 "진짜 선언 + 미닫힘 펜스 안 예시" 파일을 G7
//!        `duplicate`로 죽이는 회귀를 낳았다(실측). 회수는 선언이 **없을 때의** 구제책이다.
//!   G10' **주석 안 앵커**(`is_retire_marker_line`) — `<!--` 와 마커 토큰 사이에는 장식
//!        문자만(`DECOR_CHARS`) 허용한다. 종전에는 `RETIRED`만 앵커되고 나머지 두 토큰은
//!        주석 안 어디든 부분일치해서 **부정문이 파일을 은퇴시켰다**(실측).
//!
//! **정규식을 쓰지 않는다**: 이 저장소에 `regex`가 이미 있으나(Cargo.toml), 이 파서는 데몬
//! 워치독 틱에서 파일마다 호출된다. `Regex::new`를 호출부마다 컴파일하는 비용도, 전역
//! lazy 정적을 새로 들이는 결합도 이 크기의 문법에는 과하다 — Python 정본의 5개 정규식은
//! 전부 선형 문자열 스캔으로 등가 전개된다(아래 각 단계에 대응 관계를 주석으로 박제).
//!
//! **패닉 금지**: 워치독 틱 경로라 `unwrap`/`expect`/슬라이스 인덱싱 패닉을 만들지 않는다.
//! 모든 실패는 `Option`/`Result`로 흐른다.

/// 파싱 예산 — 파일 **원시 바이트** 선두 1 KiB만 본다(G3).
///
/// ★**예산은 정확히 한 곳에서만 적용된다 — `head_from_bytes`다**(W14 S15 교정 · 2026-07-26).
/// 종전에는 호출자가 원시 1 KiB를 자르고 `parse`가 **디코드된 문자열 바이트** 1 KiB로 한 번 더
/// 잘랐다. 이 이중 절단은 순수 ASCII에서는 무해하지만 비UTF-8 파일에서 갈린다: `from_utf8_lossy`가
/// 1바이트를 U+FFFD 3바이트로 팽창시키므로, 두 번째 절단이 **Python이 보는 영역을 잘라먹는다**.
/// 실측(400 B의 `0xFF` 뒤에 놓인 선언) — rust=`unclaimed` / python=`counted`,
/// 은퇴 선언이면 rust=`unclaimed` / python=`retired` = **은퇴 파일을 데몬만 계속 집계**한다.
///
/// 그래서 `parse`는 **자르지 않는다.** 입력은 이미 예산이 적용된 머리말이라고 가정하며, 모든
/// 소비자는 `head_from_bytes`를 **의무 경유**한다(governance.rs C2 · cys.rs C3 · 덤퍼 ·
/// Python `read_head`). 예산이 두 곳에 적히면 언젠가 갈린다 — 설계 §14-4 4번 규율.
pub const HEAD_BYTES: usize = 1024;

/// 지원 버전 토큰(G5·G6) — 모르는 버전은 **미선언**과 동일 취급이다(ADR-2 스큐 정책).
const SUPPORTED_VERSIONS: [&str; 1] = ["v1"];

/// 필수 키 3종(G5). 순서는 진단 문자열의 나열 순서이기도 하다(참조 구현과 동일).
const REQUIRED_KEYS: [&str; 3] = ["owner", "scope", "status"];

/// 고정 접두(G5).
const PREFIX: &str = "javis:todo";

/// 펜스 코드블록을 여는 문자(G12) — 3개 이상 연속이면 펜스다.
const FENCE_CHARS: [char; 2] = ['`', '~'];

/// 레거시 은퇴 마커의 기계 토큰(G10') — ASCII 소문자 표기가 정본이다(대소문자 무시 비교).
const LEGACY_RETIRE_TOKEN: &str = "javis:todo-retired";

/// 주석 `<!--` 와 마커 토큰 사이에 허용되는 **장식 문자** 집합(G10' · W15 교정 2).
///
/// ★Python 정본 `javis_todo_decl.DECOR_CHARS`와 **같은 리터럴**이어야 한다 —
/// `cysjavis-pack/bin/tests/test_todo_shared_constants.py`가 두 문자열을 기계 대조한다.
/// W13은 "장식 집합을 정의하는 순간 2언어가 갈린다"며 집합을 피했고, 그 회피가 이번
/// 결함(부정문 은퇴)의 원인이 됐다. 갈릴 여지를 없애는 방법은 집합을 피하는 것이 아니라
/// 집합을 **한 곳에 적고 기계로 묶는 것**이다.
///
/// 이 집합에는 **한글·라틴 문자가 하나도 없다** — 그것이 유일한 계약이다. 문장의 첫 글자로
/// 오지 않는 기호만 담았으므로, 마커를 설명하거나 부정하는 문장은 이 관문을 통과할 수 없다.
pub const DECOR_CHARS: &str = " \t★☆*=#-~_+";

/// 언어중립 진단 코드(G9 · ADR-4 C-2).
///
/// ★**이 7종이 2언어 파리티 계약의 전부다.** Python 정본 `javis_todo_decl.py`의 `DIAG_CODES`가
/// 같은 7개 문자열을 같은 뜻으로 갖고, `cysjavis-pack/bin/tests/parity_todo_decl.py`가 두 목록을
/// 기계 대조한다. 코드를 추가·개명하려면 양 언어 + 골든 픽스처 `expected.json`을 **같은
/// 커밋에서** 함께 고쳐야 한다.
///
/// 아래 `ParseError::message`(한국어)는 **계약이 아니다** — 문구를 계약에 넣으면 문구를
/// 다듬는 순간 파리티 CI가 결함을 오보한다. 문구는 자유롭게 손질하되 code는 유지하라.
pub mod diag {
    /// 머리말 영역에 v1 선언도 레거시 은퇴 마커도 없다(미선언 = 정상 상태의 하나).
    pub const NO_DECL: &str = "no-decl";
    /// 머리말에 선언 후보가 2개 이상 — 모호성 거부(G7).
    pub const DUPLICATE: &str = "duplicate";
    /// 선언 줄의 **구조**가 깨졌다(따옴표 형태·`-->` 부재·버전 토큰 형태·후행 텍스트).
    pub const SYNTAX: &str = "syntax";
    /// 구조는 맞으나 버전 토큰을 모른다(G6 스큐 정책 — 미선언과 동일 취급).
    pub const UNKNOWN_VERSION: &str = "unknown-version";
    /// `key=value` 토큰 하나가 G4 문자 클래스를 위반한다.
    pub const BAD_TOKEN: &str = "bad-token";
    /// 필수 키 3종(owner·scope·status) 중 누락이 있다(G5 deny-by-default).
    pub const MISSING_KEYS: &str = "missing-keys";
    /// status 값이 `active|retired` 밖이다.
    pub const BAD_STATUS: &str = "bad-status";

    /// 파리티 대조용 전량 목록(선언 순서 = Python `DIAG_CODES` 순서).
    pub const ALL: [&str; 7] = [
        NO_DECL,
        DUPLICATE,
        SYNTAX,
        UNKNOWN_VERSION,
        BAD_TOKEN,
        MISSING_KEYS,
        BAD_STATUS,
    ];
}

/// 참조 구현이 구조 위반을 하나로 뭉뚱그리는 진단 문구(코드는 `diag::SYNTAX`).
const MSG_SYNTAX: &str = "문법 위반(따옴표·후행 텍스트·형식 불일치)";

/// 미선언 사유 — **언어중립 `code`가 계약이고, `message`는 사람을 위한 것**이다(ADR-4 C-2).
///
/// `parse`가 `Err`로 돌려주는 값이며, 소비자는 `code`로 분기하고 `message`는 그대로 보여준다.
/// 진단 분류를 코드로 노출하지 않으면 파리티 대조가 한국어 문구에 걸리게 되고, 그때부터
/// **문구를 고치는 일이 곧 CI 실패**가 된다 — 실제로 구현 첫날 그 지점에서 2언어가 갈렸다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    /// `diag::*` 중 하나. 파리티 계약의 유일한 진단 판정 입력이다.
    pub code: &'static str,
    /// 사람이 읽는 사유(G9). 계약 아님 — 자유롭게 다듬어도 된다.
    pub message: String,
}

impl ParseError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        ParseError {
            code,
            message: message.into(),
        }
    }

    /// 구조 위반(가장 흔한 갈래) 축약 생성자.
    fn syntax() -> Self {
        ParseError::new(diag::SYNTAX, MSG_SYNTAX)
    }
}

impl std::fmt::Display for ParseError {
    /// 사람 대면 출력은 문구만 낸다(기존 `Result<_, String>` 시절의 표기와 동일).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ParseError {}

/// 선언 블록 v1의 해석 결과.
///
/// `status`는 `active` | `retired` 둘 중 하나임이 파싱 시점에 보장된다(G5). `legacy=true`는
/// 레거시 은퇴 마커(G10)에서 합성된 선언으로, `owner`/`scope`는 알 수 없어 `"?"`가 들어간다 —
/// 판정이 `retired`에서 즉시 끝나므로(§4-2 1분기) 두 필드는 사용되지 않는다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decl {
    pub owner: String,
    pub scope: String,
    pub status: String,
    pub legacy: bool,
}

/// §4-2 판정 5분기. 픽스처 파리티 비교는 `as_str()`의 케밥 문자열로 한다 —
/// **비교 가능한 표현이 없으면 파리티 CI는 형식만 갖춘 껍데기가 된다.**
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// 내 팩의 살아있는 작업 — 진행률에 집계한다.
    Counted,
    /// 은퇴 선언(신규 `status=retired` 또는 레거시 마커) — 조용히 배제.
    Retired,
    /// 남의 레인이라고 스스로 밝혔고 그 팩이 실재 — 조용히 배제(목록에는 노출).
    ForeignScope,
    /// 남의 레인을 가리키는데 그 팩이 디스크에 없다 — **시끄럽게** 보고(개명·teardown 흔적).
    OrphanScope,
    /// 선언 없음·깨짐 — 숨기지 않고 별도 버킷으로 보고한다(ADR-3 fail-open).
    Unclaimed,
}

impl Verdict {
    /// 2언어 파리티 비교용 직렬화(§4-2 계약). 이 문자열 집합은 Python 구현과 동일해야 한다.
    pub fn as_str(&self) -> &'static str {
        match self {
            Verdict::Counted => "counted",
            Verdict::Retired => "retired",
            Verdict::ForeignScope => "foreign-scope",
            Verdict::OrphanScope => "orphan-scope",
            Verdict::Unclaimed => "unclaimed",
        }
    }
}

impl std::fmt::Display for Verdict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 파일 선두 텍스트에서 선언을 파싱한다(G1'~G10).
///
/// `Err`는 **미선언**을 뜻하며 값은 진단이다(G9) — 조용한 실패가 채택을 막는 진짜 원인이라
/// 무엇이 틀렸는지 즉시 알려준다. `ParseError::code`가 언어중립 계약값이고 `message`는 UX다.
/// 호출자는 `parse(head).ok()`로 `Option`화해 `classify`에 넘긴다.
///
/// ★**입력은 이미 예산(G3)이 적용된 머리말이어야 한다** — `head_from_bytes(&raw)`의 반환값이
/// 정확히 그것이다. 여기서 다시 자르지 않는 이유는 `HEAD_BYTES` 주석에 있다(예산 이원화 =
/// 2언어 판정 갈림). Python 정본 `javis_todo_decl.parse`도 동일하게 자르지 않는다.
pub fn parse(head: &str) -> Result<Decl, ParseError> {
    // G8: BOM 선행 제거. 규정이 없으면 2언어 구현이 갈린다(SIM-3 발견).
    let head = head.trim_start_matches('\u{feff}');

    // G1'+G12: 머리말 영역 = **첫 체크박스 이전 중 코드펜스·인용·들여쓰기 밖**.
    // (A2 자해 방어 + W9 교정 1 — 상세는 `header_lines`.)
    let header = header_lines(head);

    // 후보 = `^<!--[ \t]*javis:todo[ \t]` (G1'). `javis:todo` 뒤에 **공백**을 요구하는 것이 계약이다 —
    // `javis:todo-retired`(레거시 은퇴 마커)는 v1 선언의 오작성이 아니라 **다른 토큰**이므로,
    // 공백을 요구하지 않으면 레거시 마커가 "깨진 v1 선언"으로 잡혀 G10이 통째로 무력화된다.
    let candidates: Vec<&str> = header
        .iter()
        .map(|l| trim_ws(l))
        .filter(|l| is_decl_candidate(l))
        .collect();

    if candidates.is_empty() {
        // G10': 레거시 은퇴 **마커 줄**도 은퇴 선언으로 인정한다. 없으면 기존 은퇴 파일이 전부
        // `unclaimed`로 쏟아진다(구→신 스큐 · SIM-2 발견).
        for line in &header {
            if is_retire_marker_line(line) {
                // ADR-4 C-3 — owner/scope 미상은 센티널 `"?"`. Python 정본과 **동일한 표현**이다
                // (Python은 키를 빼지 않고 같은 센티널을 채운다). 이 값이 scope 판정에 새지 않는
                // 것은 `classify`의 retired 단락이 보장하며, 그 불변식은 패닉 콜백으로 핀돼 있다.
                return Ok(Decl {
                    owner: "?".to_string(),
                    scope: "?".to_string(),
                    status: "retired".to_string(),
                    legacy: true,
                });
            }
        }
        return Err(ParseError::new(diag::NO_DECL, "선언 없음"));
    }
    if candidates.len() > 1 {
        // G7: 모호성 거부 = 결정론.
        return Err(ParseError::new(diag::DUPLICATE, "선언 2개 이상(모호)"));
    }

    // ── 참조 구현의 `DECL_RE = <!--[ \t]*javis:todo[ \t]+(v\d+)[ \t]+(.*?)[ \t]*-->\z` 선형 전개 ──
    let line = candidates[0];
    // `<!--[ \t]*`
    let rest = trim_start_ws(line.strip_prefix("<!--").ok_or_else(ParseError::syntax)?);
    // `javis:todo[ \t]+`
    let rest = rest.strip_prefix(PREFIX).ok_or_else(ParseError::syntax)?;
    if !rest.starts_with(is_ws) {
        return Err(ParseError::syntax());
    }
    let rest = trim_start_ws(rest);
    // `(v\d+)[ \t]+` — 버전 토큰 뒤 공백은 정규식이 요구하는 필수 조건이다(`v1-->`는 불일치).
    let ver_end = rest.find(is_ws).ok_or_else(ParseError::syntax)?;
    let version = &rest[..ver_end];
    if !is_version_token(version) {
        return Err(ParseError::syntax());
    }
    // `(.*?)[ \t]*-->\z` — 앵커라 `-->`는 (후행 공백을 뺀) **마지막**이어야 한다.
    // 그래서 `… --> <!-- 메모 -->` 같은 후행 주석은 본문에 `-->`를 남겨 토큰 검사에서 걸린다.
    let tail = trim_end_ws(trim_start_ws(&rest[ver_end..]));
    let body = trim_end_ws(tail.strip_suffix("-->").ok_or_else(ParseError::syntax)?);

    // 후행 텍스트 방어. `-->` 뒤에 뭔가 더 붙으면 비탐욕 그룹이 **뒤쪽** `-->`까지 삼켜
    // body에 `-->`가 남는다. 이걸 토큰 위반으로 흘려보내면 `토큰 문법 위반: -->`이라는
    // 쓸모없는 진단이 나가므로, 진짜 원인(후행 텍스트)을 그대로 말해준다(G9).
    if body.contains("-->") {
        return Err(ParseError::syntax());
    }

    // 버전 판정은 **구조 판정 이후**다(`-->`가 깨진 v2 줄은 "미지 버전"이 아니라
    // "문법 위반"으로 진단된다 — 골든 픽스처가 이 순서를 고정한다).
    if !SUPPORTED_VERSIONS.contains(&version) {
        return Err(ParseError::new(diag::UNKNOWN_VERSION, format!("미지 버전 {}", version))); // G6
    }

    // G4: `key=value` 공백 구분. 값에 따옴표·공백·이스케이프 없음 = 파서 단순.
    // G6: 모르는 키는 무시하되 **문법 검사는 모든 토큰에 적용**한다(참조 구현 동일).
    let (mut owner, mut scope, mut status) = (None, None, None);
    // G11: `split_whitespace()`(유니코드 `White_Space` 전량)가 아니라 스페이스·탭으로만 가른다.
    for tok in split_ws(body) {
        let (k, v) = split_kv(tok)
            .ok_or_else(|| ParseError::new(diag::BAD_TOKEN, format!("토큰 문법 위반: {}", tok)))?;
        match k {
            "owner" => owner = Some(v.to_string()),
            "scope" => scope = Some(v.to_string()),
            "status" => status = Some(v.to_string()),
            _ => {} // 전방 호환
        }
    }

    // G5: 필수 키 3종. 누락 목록은 REQUIRED_KEYS 순서대로 나열한다(진단 문자열 파리티).
    let missing: Vec<&str> = REQUIRED_KEYS
        .iter()
        .copied()
        .filter(|k| match *k {
            "owner" => owner.is_none(),
            "scope" => scope.is_none(),
            _ => status.is_none(),
        })
        .collect();
    if !missing.is_empty() {
        return Err(ParseError::new(
            diag::MISSING_KEYS,
            format!("필수 키 누락: {}", missing.join(",")),
        ));
    }

    let (owner, scope, status) = match (owner, scope, status) {
        (Some(o), Some(s), Some(t)) => (o, s, t),
        // 위 missing 검사가 이미 걸러내므로 도달 불가. `unwrap` 대신 진단으로 흘려보낸다.
        _ => return Err(ParseError::syntax()),
    };
    if status != "active" && status != "retired" {
        return Err(ParseError::new(diag::BAD_STATUS, format!("status 값 위반: {}", status)));
    }

    Ok(Decl {
        owner,
        scope,
        status,
        legacy: false,
    })
}

/// 파일 원시 바이트에서 파싱 대상 머리말을 만든다 — Python 정본 `read_head`의 등가물.
///
/// ★**바이트 기준 절단**이 계약이다(G3). 텍스트로 읽어 문자 수로 자르면 한글이 섞인 파일에서
/// 절단 지점이 Python과 갈리고, 경계 근처에 선언이 있는 파일에서 2언어 판정이 어긋난다.
/// 경계에서 잘린 다바이트 문자는 양쪽 모두 U+FFFD가 된다(`from_utf8_lossy` ≡ Python
/// `errors="replace"`). 소비자가 각자 이 절차를 재현하면 그 자체가 drift 표면이므로 여기 둔다.
///
/// ★**이 함수가 예산의 유일한 적용 지점이다**(W14 S15 · 2026-07-26). 소비자는 자기 손으로
/// `raw[..1024]`·`decoded[..1024]`를 자르지 말고 **반드시 이 함수를 경유**하라 — 종전에는
/// 프로덕션 데몬(`governance.rs`)과 CLI(`cys.rs`)가 각자 재구현했고 유일한 호출자가 테스트
/// 덤퍼였다. **하네스가 검증하는 읽기 경로 ≠ 프로덕션 읽기 경로**인 상태였고, 그래서 파리티
/// CI가 초록인 채로 프로덕션만 갈릴 수 있었다.
pub fn head_from_bytes(raw: &[u8]) -> String {
    String::from_utf8_lossy(&raw[..raw.len().min(HEAD_BYTES)]).into_owned()
}

/// §4-2 판정 5분기. 상수 없음·시간 의존 없음.
///
/// `scope_exists`를 콜러블로 받는 이유: 파일시스템 접근을 파서에 넣지 않기 위해서다
/// (테스트 주입 가능 · 데몬은 팩 디렉터리 존재 검사를, Python은 동일 의미의 검사를 주입한다).
/// `retired`가 첫 분기라 **은퇴 판정에는 디스크 접근이 아예 일어나지 않는다**(워치독 틱 보호).
pub fn classify(decl: Option<&Decl>, my_scope: &str, scope_exists: &dyn Fn(&str) -> bool) -> Verdict {
    let decl = match decl {
        Some(d) => d,
        None => return Verdict::Unclaimed, // ADR-3 fail-open — 숨기지 않고 시끄럽게
    };
    if decl.status == "retired" {
        return Verdict::Retired;
    }
    if decl.scope == my_scope {
        return Verdict::Counted;
    }
    // 실재하는 팩을 가리키면 정상(남의 레인), 실재하지 않으면 orphan으로 시끄럽게 보고한다.
    // 무조건 조용한 배제는 07-11 사고를 거울상으로 재현한다(부서 teardown·팩 개명 시 살아있는
    // 파일이 통째로 사라짐) — R2 적대검증 교정.
    if scope_exists(&decl.scope) {
        Verdict::ForeignScope
    } else {
        Verdict::OrphanScope
    }
}

// ── 내부 헬퍼 ───────────────────────────────────────────────────────────────
//
// ★`truncate_head`(디코드 문자열 재절단)는 W14 S15에서 **삭제**됐다. 예산은 `head_from_bytes`
// 한 곳에서만 적용된다 — 되살리지 마라. 되살리는 순간 비UTF-8 파일에서 Rust만 짧게 보고,
// 은퇴한 파일을 데몬이 계속 집계하는 유령 재발 경로가 다시 열린다.

// ── G11: 개행·공백 문자 집합 (★2언어 수렴 계약 · W9 교정 2) ─────────────────
//
// ★결정: **좁은 쪽(이 구현)으로 수렴한다.** 개행은 `\n`·`\r\n`·`\r` 셋만, 토큰 구분 공백은
// 스페이스·탭 둘만이다. Python 정본 `javis_todo_decl.py`가 같은 결정을 같은 근거로 싣는다.
//
// 왜 좁은 쪽인가 — 방향이 위험을 결정하기 때문이다. 넓은 쪽(Python 기본값)은
// `str.splitlines()`가 U+000B/000C/001C-001F/0085/2028/2029까지 개행으로 보고, 정규식 `\s`와
// `str.split()`이 유니코드 공백 전량을 구분자로 본다. 이 구현은 `\n\r`만 개행으로,
// `char::is_whitespace`(=`White_Space`)만 공백으로 봤다. 실측 13건이 갈렸고(W9 재현), 그중
// 다수가 **Python은 선언을 깨진 것으로 보고 배제하는데 이 데몬은 유효 선언으로 집계**하는
// 방향이었다 — HUD에 유령이 남는 조합이다. 넓은 쪽으로 맞추면 "제어문자 하나가 우연히 섞인
// 줄"이 유효 선언으로 승격되어 은퇴/집계 오판정 표면이 늘어난다. 좁은 쪽으로 맞추면 그런 줄은
// 양 언어에서 **똑같이** 문법 위반으로 떨어진다 — 오판정이 아니라 진단이 나간다(G9).
//
// 대가(명시): `White_Space`이지만 스페이스·탭이 아닌 문자(U+00A0 NBSP·U+3000 등)로 토큰을
// 구분한 선언은 이제 `bad-token`이다. 관용을 넓히지 않는다는 R10 원칙과 같은 자리다.

/// G11 공백 술어 — 스페이스·탭 **둘만**.
fn is_ws(c: char) -> bool {
    c == ' ' || c == '\t'
}

fn trim_ws(s: &str) -> &str {
    s.trim_matches(is_ws)
}

fn trim_start_ws(s: &str) -> &str {
    s.trim_start_matches(is_ws)
}

fn trim_end_ws(s: &str) -> &str {
    s.trim_end_matches(is_ws)
}

/// G11 토큰 분해 — 참조 구현의 `re.split(r"[ \t]+", body)` 등가(빈 조각 제외).
fn split_ws(s: &str) -> impl Iterator<Item = &str> {
    s.split(is_ws).filter(|t| !t.is_empty())
}

/// G11 개행 분해 — `\n`·`\r\n`·`\r` **셋만**. 개행 문자를 남기지 않고, 말미 개행이 빈 줄을
/// 만들지 않는다(참조 구현 `_split_lines`와 동일 규칙).
///
/// 표준 `lines()`를 쓰지 않는 이유: `\r` 단독(구 Mac 개행)을 줄바꿈으로 보지 않아 파일 전체가
/// 한 줄이 되고 G1' 머리말 판정이 갈린다. Python `str.splitlines()`도 쓰지 않는다 — 위 G11
/// 주석의 유니코드 격차가 그쪽에서 온다.
fn split_lines(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let bytes = s.as_bytes();
    let (mut start, mut i) = (0usize, 0usize);
    while i < bytes.len() {
        match bytes[i] {
            b'\n' => {
                out.push(&s[start..i]);
                i += 1;
                start = i;
            }
            b'\r' => {
                out.push(&s[start..i]);
                // CRLF는 한 개의 줄바꿈.
                i += if i + 1 < bytes.len() && bytes[i + 1] == b'\n' { 2 } else { 1 };
                start = i;
            }
            _ => i += 1,
        }
    }
    if start < bytes.len() {
        out.push(&s[start..]);
    }
    out
}

/// G12 — 선두 공백/탭 런을 재서 `(들여쓴 코드블록인가, 본문 시작 바이트 오프셋)`을 낸다.
///
/// 탭이 선두 런에 한 번이라도 나오면 들여쓴 코드블록으로 본다 — 탭 폭은 렌더러마다 다르고
/// "탭 1개 = 4칸"을 가정하는 순간 양 언어가 갈릴 여지가 생긴다. 공백은 4개 이상이 코드블록.
fn indent_of(line: &str) -> (bool, usize) {
    let mut n = 0usize;
    for c in line.chars() {
        match c {
            ' ' => n += 1,
            '\t' => return (true, 0),
            _ => break,
        }
    }
    (n >= 4, n)
}

/// G12 — 줄 선두의 펜스 런 `(문자, 길이)`. 3개 미만이면 `None`.
fn fence_run(rest: &str) -> Option<(char, usize)> {
    let ch = rest.chars().next()?;
    if !FENCE_CHARS.contains(&ch) {
        return None;
    }
    let n = rest.chars().take_while(|c| *c == ch).count();
    if n >= 3 {
        Some((ch, n))
    } else {
        None
    }
}

/// G1'+G12 — 선언 후보가 될 수 있는 **머리말 줄**만 골라 낸다(Python `header_lines` 등가).
///
/// ★G1'(첫 체크박스 이전)만으로는 부족하다는 것이 W9 재현의 결론이다. 선언 문법을
/// **설명하는** todo(코드펜스 안에 예시 선언을 적은 문서)가 자기 자신을 은퇴시켰다 — 그리고
/// 이 프로젝트에서 가장 흔한 todo가 정확히 그 종류다(선언 도입 작업 자체). 머리말 앞 영역을
/// 코드펜스·인용 구분 없이 전량 신뢰한 것이 결함이다.
///
/// 머리말 안에서 다음 4가지를 선언 후보에서 **제외**한다.
///   ① 펜스 코드블록 내부(``` / ~~~ 3개 이상 · 여는 펜스와 같은 문자·같은 길이 이상으로 닫힘)
///   ② 인용문(줄 선두 `>` · 선행 공백 3개까지 허용)
///   ③ 들여쓴 코드블록(선두 공백 4개 이상 또는 탭)
///   ④ 선언 줄 자체의 선두 공백은 3개 이하(=③의 여집합)
///   ⑤ ★단, **미닫힘 펜스는 회수한다**(W13 교정 4 · master 심판) — 머리말이 끝날 때까지
///      닫히지 않은 펜스는 **없었던 것으로 재판정**하고 그 안에 갇혔던 줄을 ②③만 적용해
///      후보로 되돌린다.
///
/// ★엄격 G1(첫 비어있지 않은 줄)으로 되돌리지 않는다 — 기존 todo 63개 중 56개(89%)가
/// `# 제목`으로 시작한다(실측). 완화는 유지하고 구멍만 막는다.
///
/// ★⑤가 왜 필요한가(W13 교정 4). 종전에는 펜스가 열리면 첫 체크박스까지 **무조건** 마스킹하고
/// 회수 규칙이 없었다. 그래서 인라인 삼중백틱 한 줄이 그 아래 **정당한 선언을 통째로 삼켰다** —
/// M3에서 휴리스틱 폴백이 삭제되면 그 파일은 집계에서 사라지고, 지금도 `unclaimed_ratio`
/// (M3 전환 판단 근거)를 왜곡한다. 반대 방향도 같은 무게다: 같은 형태로 `status=retired`
/// 선언을 삼키면 **은퇴시켰다고 믿은 파일이 계속 집계된다**.
/// ★대가(명시): 미닫힘 펜스 안의 *예시* 선언은 이제 진짜 선언으로 읽힌다(골든 픽스처
/// `30-unclosed-fence.md`가 그 계약이다). CommonMark는 미닫힘 펜스를 문서 끝까지로 보지만,
/// "펜스를 닫지 않은 문서"는 오작성이고 그 오작성이 **정당한 선언을 죽이는** 쪽이 더 위험하다는
/// 것이 심판의 근거다. 닫힌 펜스의 예시 선언 보호(G12 ①의 본령)는 그대로다.
///
/// 회수 구간에서 펜스를 다시 해석하지는 않는다 — 펜스 런으로 시작하는 줄은 회수 대상에서
/// 제외한다. 재해석은 "닫히지 않은 펜스 안의 펜스"라는 재귀를 낳고, 그 재귀 규칙을 2언어가
/// 똑같이 구현했다고 보장할 방법이 없다(Python 정본 `header_lines`가 같은 규칙을 쓴다).
///
/// ★⑤' **충돌 시 회수 취소**(W15 교정 1 · master 심판 2026-07-26). ⑤는 도입 직후부터
/// 정당한 선언을 죽이는 회귀를 갖고 있었다 — 진짜 선언과 미닫힘 펜스 안 예시가 같은 파일에
/// 있으면 회수가 후보를 2개로 만들어 G7 `duplicate` → `unclaimed`가 됐다(회수 도입 전에는
/// 예시가 마스킹돼 `counted`였다 · 실측). 선언 도입 작업의 todo가 정확히 그 형태다.
/// 그래서 **회수 구간에 후보가 있고 정상 구간에도 후보가 있으면 회수를 취소한다**
/// (정상 후보 우선). 회수는 "선언이 없을 때의 구제책"이지 "선언을 늘리는 장치"가 아니다.
fn header_lines(head: &str) -> Vec<&str> {
    let mut out: Vec<&str> = Vec::new();
    let mut fence: Option<(char, usize)> = None; // 열린 펜스
    let mut pending: Vec<&str> = Vec::new(); // ⑤ 미닫힘으로 판명되면 회수할 줄들
    for line in split_lines(head) {
        if has_checkbox(line) {
            break; // G1': 첫 체크박스 = 머리말의 끝
        }
        let (indented, ind) = indent_of(line);
        let rest = line.get(ind..).unwrap_or("");
        if let Some(open) = fence {
            // ① 펜스 안 — 닫힘만 살핀다.
            let run = if indented { None } else { fence_run(rest) };
            let closes = match run {
                Some((ch, n)) => {
                    ch == open.0 && n >= open.1 && trim_ws(rest.get(n..).unwrap_or("")).is_empty()
                }
                None => false,
            };
            if closes {
                fence = None;
                pending.clear(); // 닫혔다 = 진짜 코드블록이었다 → 회수하지 않는다
            } else if !indented && run.is_none() && !rest.starts_with('>') {
                pending.push(line); // ⑤ 회수 후보로 적재(②③은 여기서도 적용)
            }
            continue;
        }
        if indented {
            continue; // ③④ 들여쓴 코드블록
        }
        if let Some(run) = fence_run(rest) {
            fence = Some(run); // ① 펜스 개시 줄 자체도 선언이 아니다
            pending.clear();
            continue;
        }
        if rest.starts_with('>') {
            continue; // ② 인용문
        }
        out.push(line);
    }
    if fence.is_some() {
        // ⑤ 미닫힘 펜스 = 없었던 것으로 재판정.
        // ⑤' 단, 회수가 **기존 후보와 충돌**하면 회수를 취소한다(정상 후보 우선 · W15 교정 1).
        if !(has_candidate(&pending) && has_candidate(&out)) {
            out.extend(pending);
        }
    }
    out
}

/// 이 줄이 파일의 판정을 확정시킬 수 있는 **후보 줄**인가(G12 ⑤' 충돌 판정용).
///
/// 후보 = v1 선언 후보(`is_decl_candidate`) ∪ 레거시 은퇴 마커 줄(G10'). 둘 다 세는 이유는
/// 한쪽만 세면 같은 충돌이 다른 토큰으로 재발하기 때문이다 — `parse`가 선언 후보를 먼저 보고
/// 없을 때만 마커를 보므로 "정상 구간의 마커 vs 회수 구간의 선언"도 판정을 뒤집는다.
/// Python 정본 `_is_candidate_line`과 같은 규칙이다.
fn is_candidate_line(line: &str) -> bool {
    let s = trim_ws(line);
    is_decl_candidate(s) || is_retire_marker_line(s)
}

fn has_candidate(lines: &[&str]) -> bool {
    lines.iter().any(|l| is_candidate_line(l))
}

/// Python `RE_DECL_CAND = ^<!--[ \t]*javis:todo[ \t]` 등가. **입력은 이미 trim된 줄**이어야 한다.
///
/// 후보 선별이 레거시 마커(G10) 검사보다 먼저 돌기 때문에, 이 필터가 느슨하면
/// `<!-- javis:todo-retired -->`가 "깨진 v1 선언"으로 잡혀 `unclaimed`가 된다 —
/// 은퇴 파일이 M3에서 전부 쏟아지는 SIM-2 결함이 그대로 되살아난다. 그래서 `javis:todo`
/// **직후 공백**을 요구한다(설계 프로토타입이 `contains("javis:todo")`로 두고 놓친 지점).
fn is_decl_candidate(line: &str) -> bool {
    line.strip_prefix("<!--")
        .map(trim_start_ws)
        .and_then(|r| r.strip_prefix(PREFIX))
        .is_some_and(|r| r.starts_with(is_ws))
}

/// 참조 구현의 `RE_DONE = - \[[xX]\]` / `RE_OPEN = - \[ \]` 등가(줄 어디에나 나타나면 성립).
fn has_checkbox(line: &str) -> bool {
    line.contains("- [x]") || line.contains("- [X]") || line.contains("- [ ]")
}

/// `v\d+` (ASCII 숫자 1개 이상).
fn is_version_token(tok: &str) -> bool {
    match tok.strip_prefix('v') {
        Some(digits) => !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()),
        None => false,
    }
}

/// 참조 구현의 `KV_RE = ^([A-Za-z][A-Za-z0-9_]*)=([A-Za-z0-9._:-]+)$` 등가.
/// 키·값 어느 클래스에도 `=`가 없으므로 **첫 `=`** 로 가르는 것이 정규식과 동치다.
fn split_kv(tok: &str) -> Option<(&str, &str)> {
    let eq = tok.find('=')?;
    let key = tok.get(..eq)?;
    let value = tok.get(eq + 1..)?;
    let mut kc = key.chars();
    if !kc.next()?.is_ascii_alphabetic() {
        return None;
    }
    if !kc.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    if value.is_empty()
        || !value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '-'))
    {
        return None;
    }
    Some((key, value))
}

/// G10' — 이 **줄 하나**가 레거시 은퇴 마커 줄인가(줄 전체 앵커 · W13 교정 1).
///
/// ★종전 결함: 판정이 부분일치(`줄 어디에나`)였다. G12 마스킹은 펜스·인용·들여쓰기만 덮으므로
/// **평범한 머리말 산문**이 그대로 통과했다 — reviewer1 실측:
///     `이번 작업 목표: STALE 무효화 마커를 기계가 읽도록 구현한다.`
/// 이 한 줄이 미완 2건짜리 살아있는 파일을 `retired`로 확정시켰고, Rust 데몬도 같이 지웠다.
/// 문구의 출처가 우리 자신(디렉티브의 *"레인이 끝나면 status=retired 로 갱신하라"*)이라
/// **워커가 지침을 자기 todo 머리말에 적으면 자해**하는 구조였다.
///
/// 인정 형태는 2종뿐이다(Python 정본 `is_retire_marker_line`과 **같은 규칙**).
///   (i)  주석 줄: 줄이 `<!--` 로 시작하고, `<!--` 와 마커 토큰 사이에 **장식 문자만**
///        (`DECOR_CHARS`) 있다. 토큰 **뒤**의 꼬리 텍스트는 허용한다.
///   (ii) 비주석 줄: 줄 **전체**가 마커 토큰 하나와 정확히 일치한다(꼬리 텍스트 불가).
///
/// ★(i)이 `-->`(주석 닫힘)를 요구하지 않는 이유는 실측이다. 이 조직의 실물 은퇴 마커
/// (07-11 teardown이 삽입한 `REVIEWER_GEMINI_TODO.md` 머리말 · 코퍼스에 10건)가 **여러 줄
/// 주석의 개시 줄**이다. 같은 줄의 `-->`를 요구하면 그 파일들이 즉시 `unclaimed`가 되어
/// 07-26 유령 집계가 재발한다.
///
/// ★W15 교정 2 — 장식 접두만 허용한다. 종전에는 `RETIRED`만 `<!--` 직후로 앵커되고
/// `javis:todo-retired`·`stale 무효화`는 주석 안 **어디든** 부분일치했다. 그 비대칭이
/// `<!-- 이 파일은 STALE 무효화 대상이 **아니다** -->` 라는 **부정문에 파일을 은퇴시킬
/// 권한**을 줬다(실측 재현). 실물 마커 10개가 전부 `<!-- ★★★★ STALE 무효화 (…)` 형태로
/// 장식이 토큰 앞에 오므로 엄격 앵커는 쓸 수 없었고, 장식 집합을 리터럴 상수로 고정해
/// 2언어를 기계로 묶는 쪽을 택했다(`DECOR_CHARS`).
///
/// 대가(명시): 비주석 줄은 꼬리 텍스트가 한 글자라도 붙으면 마커가 아니다
/// (`STALE 무효화 (2026-07-11)` 도 탈락 — Python `RE_STALE_INVALIDATE_FULL`의 `\Z` 앵커).
///
/// ── ★방어 범위 정본 (W18 교정 2 · 릴리스 노트 인용용) ─────────────────────────────
/// 문장 문자가 **토큰 앞**에 오면 마커가 아니다. 토큰 **뒤**의 꼬리 텍스트는 마커를
/// 무력화하지 못한다(실물 마커가 꼬리를 요구하므로 불가피). 따라서 마커 토큰으로 시작하는
/// 설명·부정문은 은퇴로 읽힌다 — 마커를 **설명하려면 토큰을 문장 뒤에 두어라.**
///
/// 즉 아래 두 줄은 (i)의 계약상 **은퇴로 읽힌다**(실측 재현 · Python과 동일 판정):
///   · `<!-- **STALE 무효화** 는 이 파일에 해당하지 않는다 -->`  (`**`가 DECOR_CHARS)
///   · `<!-- javis:todo-retired 마커는 레거시 표기다 -->`
/// 이는 결함이 아니라 **계약의 경계**다. 꼬리 텍스트를 금지하면 실물 마커 10개가 전부
/// `unclaimed`가 되어 07-26 유령 집계가 재발한다(실측). 실물 코퍼스에서 이 형태는 0건이다.
/// 안전한 설명 표기: `<!-- 이 파일은 STALE 무효화 대상이 아니다 -->`(토큰이 뒤 = 탈락).
pub fn is_retire_marker_line(line: &str) -> bool {
    let s = trim_ws(line);
    if let Some(inner) = s.strip_prefix("<!--") {
        // (i) 주석 줄 — 장식 접두만 소거하고 토큰을 **앵커**한다(문장 문자가 오면 즉시 탈락).
        let inner = trim_start_decor(inner);
        if starts_with_ci(inner, "retired") {
            return true;
        }
        if starts_with_ci(inner, LEGACY_RETIRE_TOKEN) {
            return true;
        }
        return starts_with_stale_invalidate(inner);
    }
    // (ii) 마커 전용 줄
    if eq_ci(s, LEGACY_RETIRE_TOKEN) {
        return true;
    }
    is_exactly_stale_invalidate(s)
}

/// 선두의 `DECOR_CHARS` 런을 소거한다(G10' · W15 교정 2 — Python `str.lstrip(DECOR_CHARS)` 등가).
fn trim_start_decor(s: &str) -> &str {
    s.trim_start_matches(|c| DECOR_CHARS.contains(c))
}

/// 선두가 `stale[ \t]*무효화` 인가(ASCII 대소문자 무시) — Python `RE_STALE_INVALIDATE_HEAD.match`.
fn starts_with_stale_invalidate(s: &str) -> bool {
    match s.get(.."stale".len()) {
        Some(head) if starts_with_ci(head, "stale") => {
            trim_start_ws(s.get("stale".len()..).unwrap_or("")).starts_with("무효화")
        }
        _ => false,
    }
}

/// 줄 **전체**가 `stale[ \t]*무효화` — 참조 구현 `RE_STALE_INVALIDATE_FULL.match`.
fn is_exactly_stale_invalidate(s: &str) -> bool {
    match s.get(.."stale".len()) {
        Some(head) if starts_with_ci(head, "stale") => {
            trim_start_ws(s.get("stale".len()..).unwrap_or("")) == "무효화"
        }
        _ => false,
    }
}

/// ASCII 대소문자 무시 **전체 일치**.
fn eq_ci(a: &str, b: &str) -> bool {
    a.len() == b.len() && starts_with_ci(a, b)
}

// ★W15 교정 2로 `scan_after`·`find_ci`(부분일치 스캔 헬퍼)가 사라졌다. 마커 판정이 전부
// **앵커**(접두 일치)로 바뀌었기 때문이다 — 부분일치 스캔이 남아 있으면 다음 수정자가
// "여기서 쓰면 되겠네" 하고 되살릴 수 있고, 그 순간 부정문 은퇴 결함이 그대로 재발한다.
// 죽은 코드를 남기지 않는 것이 이 자리에서는 회귀 방어다.

/// ASCII 대소문자 무시 접두 검사.
fn starts_with_ci(hay: &str, needle: &str) -> bool {
    let (h, n) = (hay.as_bytes(), needle.as_bytes());
    h.len() >= n.len()
        && h[..n.len()]
            .iter()
            .zip(n.iter())
            .all(|(a, b)| a.eq_ignore_ascii_case(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 테스트용 팩 실재 판정 주입 — 파일시스템 비의존(`classify`가 콜러블을 받는 이유).
    fn packs(scope: &str) -> bool {
        matches!(scope, "pack" | "pack-dept-dept-1")
    }

    /// 참조 구현 SIM-3의 정상 선언 한 줄.
    const D: &str = "<!-- javis:todo v1 owner=worker-2 scope=pack status=active -->";

    /// 픽스처 파리티와 동일한 방식으로 최종 문자열 판정까지 굴린다.
    ///
    /// ★입력은 **이미 예산이 적용된 머리말**로 취급한다(W14 S15 이후 `parse`는 자르지 않는다).
    /// 예산 자체를 검사하는 케이스는 아래 `verdict_bytes`(= 프로덕션 읽기 경로)를 쓴다.
    fn verdict(body: &str) -> &'static str {
        classify(parse(body).ok().as_ref(), "pack", &packs).as_str()
    }

    /// ★**프로덕션 읽기 경로 그대로** — 원시 바이트 → `head_from_bytes`(예산 유일 적용 지점)
    /// → `parse` → `classify`. 예산·비UTF-8 케이스는 반드시 이 경로로 검사해야 한다.
    /// 종전에는 `parse`가 자체 재절단을 갖고 있어서, 이 경로를 쓰지 않는 테스트가 예산을
    /// "검사했다고 믿는" 상태였다(S15의 정확한 구조).
    fn verdict_bytes(raw: &[u8]) -> &'static str {
        classify(parse(&head_from_bytes(raw)).ok().as_ref(), "pack", &packs).as_str()
    }

    // ── 참조 구현(sim_declared_state.py SIM-3) 케이스 표 전건 ──
    // 이 15종은 Python 프로토타입이 실측으로 통과시킨 표와 **같은 입력·같은 기대값**이다.
    // 하나라도 갈리면 2언어 파서 drift(리스크 R1)가 실재한다는 뜻이다.

    #[test]
    fn sim3_normal_declaration_is_counted() {
        assert_eq!(verdict(&format!("{}\n\n# T\n- [ ] a\n", D)), "counted");
    }

    #[test]
    fn sim3_declaration_after_title_is_counted() {
        // G1' 완화의 핵심 근거: 실측 코퍼스 89%가 `# 제목`으로 시작한다. 제목 뒤 선언을
        // 거부하면 채택률이 죽는다.
        assert_eq!(
            verdict(&format!("# WORKER TODO\n\n{}\n- [ ] a\n", D)),
            "counted"
        );
    }

    #[test]
    fn sim3_quoted_value_is_rejected() {
        // G4: 따옴표 없음. 관용 파싱은 2언어 불일치의 씨앗이라 엄격 유지한다.
        let s = "<!-- javis:todo v1 owner=\"worker-2\" scope=pack status=active -->\n- [ ] a\n";
        assert_eq!(verdict(s), "unclaimed");
        assert_eq!(parse(s).unwrap_err().code, diag::BAD_TOKEN);
    }

    #[test]
    fn sim3_value_with_space_is_rejected() {
        let s = "<!-- javis:todo v1 owner=worker-2 scope=pack status=active lane=ghost todo fix -->\n- [ ] a\n";
        assert_eq!(verdict(s), "unclaimed");
    }

    #[test]
    fn sim3_missing_required_key_is_rejected() {
        let s = "<!-- javis:todo v1 owner=worker-2 status=active -->\n- [ ] a\n";
        assert_eq!(verdict(s), "unclaimed");
        let e = parse(s).unwrap_err();
        assert_eq!(e.code, diag::MISSING_KEYS);          // 계약
        assert_eq!(e.message, "필수 키 누락: scope");  // 문구는 계약 아님(이 언어 내부 회귀 핀)
    }

    #[test]
    fn sim3_uppercase_key_is_rejected() {
        // `Owner`는 KV_RE 키 클래스는 통과하지만 필수 키 `owner`를 채우지 못한다.
        let s = "<!-- javis:todo v1 Owner=worker-2 scope=pack status=active -->\n- [ ] a\n";
        assert_eq!(verdict(s), "unclaimed");
        let e = parse(s).unwrap_err();
        assert_eq!(e.code, diag::MISSING_KEYS);
        assert_eq!(e.message, "필수 키 누락: owner");
    }

    #[test]
    fn sim3_unknown_version_is_unclaimed() {
        let s = "<!-- javis:todo v2 owner=worker-2 scope=pack status=active -->\n- [ ] a\n";
        assert_eq!(verdict(s), "unclaimed");
        let e = parse(s).unwrap_err();
        assert_eq!(e.code, diag::UNKNOWN_VERSION);
        assert_eq!(e.message, "미지 버전 v2");
    }

    #[test]
    fn sim3_two_declarations_are_ambiguous() {
        let s = format!("{}\n{}\n- [ ] a\n", D, D);
        assert_eq!(verdict(&s), "unclaimed");
        assert_eq!(parse(&s).unwrap_err().code, diag::DUPLICATE);
    }

    /// ★A2 자해 회귀 핀 — 본문 체크박스 **안**의 위장 선언은 절대 인정하지 않는다.
    /// 이 테스트가 무너지면 임의의 todo 본문이 자기 파일을 은퇴시킬 수 있다.
    #[test]
    fn sim3_declaration_inside_checkbox_body_is_unclaimed() {
        let s = format!("# T\n- [ ] {}\n", D);
        assert_eq!(verdict(&s), "unclaimed");
        assert_eq!(parse(&s).unwrap_err().code, diag::NO_DECL);
    }

    /// 위장 선언의 유해 변종: 본문에서 `status=retired`로 은퇴를 위조하려는 시도.
    #[test]
    fn body_forged_retired_declaration_cannot_retire_file() {
        let s = "# T\n- [x] done\n<!-- javis:todo v1 owner=w scope=pack status=retired -->\n- [ ] open\n";
        assert_eq!(verdict(s), "unclaimed");
    }

    /// 레거시 은퇴 마커도 본문 영역에서는 인정되지 않는다(같은 위치 계약).
    #[test]
    fn body_legacy_retire_marker_is_not_honored() {
        assert_eq!(verdict("# T\n- [ ] a\n<!-- ★ STALE 무효화 -->\n"), "unclaimed");
    }

    #[test]
    fn sim3_bom_prefix_is_stripped() {
        assert_eq!(verdict(&format!("\u{feff}{}\n- [ ] a\n", D)), "counted");
    }

    #[test]
    fn sim3_crlf_line_endings_are_handled() {
        assert_eq!(verdict(&format!("{}\r\n- [ ] a\r\n", D)), "counted");
    }

    #[test]
    fn sim3_empty_input_is_unclaimed() {
        assert_eq!(verdict(""), "unclaimed");
        assert_eq!(parse("").unwrap_err().code, diag::NO_DECL);
    }

    #[test]
    fn sim3_typo_scope_is_orphan_scope() {
        // 존재하지 않는 팩 → 조용한 배제가 아니라 시끄러운 orphan(R2 교정).
        let s = "<!-- javis:todo v1 owner=worker-2 scope=pack-dept-dept-9 status=active -->\n- [ ] a\n";
        assert_eq!(verdict(s), "orphan-scope");
    }

    #[test]
    fn sim3_other_existing_pack_is_foreign_scope() {
        let s = "<!-- javis:todo v1 owner=worker-2 scope=pack-dept-dept-1 status=active -->\n- [ ] a\n";
        assert_eq!(verdict(s), "foreign-scope");
    }

    #[test]
    fn sim3_trailing_comment_after_declaration_is_rejected() {
        // `$` 앵커 계약: `-->`는 마지막이어야 한다. 후행 주석은 본문에 `-->`를 남긴다.
        let s = format!("{} <!-- 메모 -->\n- [ ] a\n", D);
        assert_eq!(verdict(&s), "unclaimed");
        // 진단은 `토큰 문법 위반: -->`(쓸모없음)이 아니라 진짜 원인인 후행 텍스트를 말한다.
        assert_eq!(parse(&s).unwrap_err().code, diag::SYNTAX);
    }

    // ── SIM-2 배송 스큐(ADR-2 불변식) ──

    #[test]
    fn skew_new_retired_declaration_is_retired() {
        let s = "<!-- javis:todo v1 owner=worker scope=pack status=retired -->\n\n# T\n- [x] a\n- [ ] b\n";
        assert_eq!(verdict(s), "retired");
        let d = parse(s).expect("유효 선언");
        assert!(!d.legacy);
    }

    #[test]
    fn skew_legacy_stale_marker_is_retired() {
        // G10 — 없으면 기존 은퇴 파일이 M3에서 전부 unclaimed로 쏟아진다.
        let s = "<!-- ★ STALE 무효화 -->\n\n# T\n- [x] a\n- [ ] b\n";
        assert_eq!(verdict(s), "retired");
        let d = parse(s).expect("레거시 은퇴 선언");
        assert!(d.legacy && d.owner == "?" && d.scope == "?");
    }

    #[test]
    fn skew_legacy_markers_all_variants() {
        for line in [
            "<!-- javis:todo-retired -->",
            "<!-- JAVIS:TODO-RETIRED -->",
            "<!-- ★ STALE  무효화 -->",
            "stale 무효화",
            "javis:todo-retired",
            "<!-- RETIRED 2026-07-11 -->",
            "<!--retired-->",
            // ★실측 정본 — 이 조직의 유일한 실물 마커는 **여러 줄 주석의 개시 줄**이다.
            // 같은 줄의 `-->`를 요구하면 그 파일이 unclaimed로 되살아난다(07-26 유령 재발).
            "<!-- ★★★★ STALE 무효화 (2026-07-11 dept-1 master 삽입) ★★★★",
        ] {
            assert_eq!(verdict(&format!("{}\n# T\n- [ ] a\n", line)), "retired", "{}", line);
        }
    }

    /// ★W13 치명 A 회귀 핀 — 마커 판정은 **줄 전체 앵커**다(부분일치 금지).
    ///
    /// 무너지면 평범한 머리말 산문 한 줄이 살아있는 파일을 통째로 은퇴시킨다. 아래 두 문장은
    /// reviewer1이 실측으로 제출한 재현 입력이고, 그 문구의 출처는 **우리 자신의 디렉티브**다
    /// (*"레인이 끝나면 status=retired 로 갱신하라"*) — 워커가 지침을 옮겨 적으면 자해했다.
    #[test]
    fn prose_mentioning_retire_marker_does_not_retire_the_file() {
        for line in [
            "규약: 레인이 끝나면 javis:todo v1 선언의 status=retired 로 바꾼다.",
            "이번 작업 목표: STALE 무효화 마커를 기계가 읽도록 구현한다.",
            "STALE 무효화 마커를 기계가 읽도록 구현한다.", // 줄 **선두**에 와도 산문은 산문이다
            "이 파일을 javis:todo-retired 로 표시하는 방법을 설명한다.",
            "★ STALE  무효화", // 주석이 아니고 마커 전용 줄도 아니다(장식 = 산문 취급)
            "★ javis:todo-retired",
            "- 종결 시 <!-- RETIRED --> 를 넣는다", // 줄이 `<!--`로 시작하지 않는다
        ] {
            let s = format!("# WORKER TODO\n{}\n- [ ] a\n- [ ] b\n", line);
            assert_eq!(verdict(&s), "unclaimed", "{}", line);
            assert_eq!(parse(&s).unwrap_err().code, diag::NO_DECL, "{}", line);
        }
    }

    /// 주석 줄이어도 `retired` 단독 토큰은 **`<!--` 바로 뒤**에서만 인정한다(옛 계약 유지).
    /// 아니면 "retired 로 바꾸는 법"을 적은 주석이 자기 파일을 은퇴시킨다.
    #[test]
    fn bare_retired_token_is_only_honored_right_after_comment_open() {
        assert_eq!(verdict("<!-- 참고: retired 로 바꾸려면 -->\n# T\n- [ ] a\n"), "unclaimed");
        assert_eq!(verdict("<!--  RETIRED  -->\n# T\n- [ ] a\n"), "retired");
    }

    /// ★W15 중대 ② 회귀 핀 — **부정문·설명문이 파일을 은퇴시키면 안 된다**(주석 안 앵커).
    ///
    /// W13은 산문(비주석)만 앵커하고 주석 안은 `RETIRED`만 앵커했다. `javis:todo-retired`·
    /// `stale 무효화`는 주석 안 **어디든** 부분일치라, 아래 두 줄이 미완 2건짜리 살아있는
    /// 파일을 `retired`로 확정시켰다(reviewer1 3차 실측). 자해의 형태는 W13이 막은 것과
    /// 똑같고 자리만 주석 안으로 옮겼을 뿐이다. Python 정본 테스트와 같은 케이스다.
    #[test]
    fn marker_token_is_anchored_inside_comments_too() {
        for line in [
            "<!-- 이 파일은 STALE 무효화 대상이 **아니다** -->",
            "<!-- 은퇴시키려면 STALE 무효화 라고 적는다 -->",
            "<!-- TODO: STALE 무효화 마커를 기계가 읽도록 구현한다 -->",
            "<!-- 이 파일을 javis:todo-retired 로 표시하는 방법 -->",
            "<!-- note: javis:todo-retired 는 레거시 토큰이다 -->",
            "<!-- 종결 시 RETIRED 를 넣는다 -->",
        ] {
            assert!(!is_retire_marker_line(line), "{}", line);
            let s = format!("# WORKER TODO\n{}\n- [ ] a\n- [ ] b\n", line);
            assert_eq!(verdict(&s), "unclaimed", "{}", line);
            assert_eq!(parse(&s).unwrap_err().code, diag::NO_DECL, "{}", line);
        }
    }

    /// 반대 방향 — 장식 접두는 **허용한다**(실물 마커 10개가 전부 이 형태다).
    /// 엄격 앵커를 택했다면 그 10파일이 즉시 unclaimed가 되어 07-26 유령 집계가 부활한다.
    #[test]
    fn decorated_marker_prefix_is_still_honored() {
        for line in [
            "<!-- ==== RETIRED ==== -->",
            "<!-- ###javis:todo-retired -->",
            "<!-- ★★★★ STALE 무효화 (2026-07-11 dept-1 master 삽입) ★★★★",
        ] {
            assert!(is_retire_marker_line(line), "{}", line);
            assert_eq!(verdict(&format!("{}\n# T\n- [ ] a\n", line)), "retired", "{}", line);
        }
    }

    /// ★W18 교정 2 — 방어 범위를 **문서가 주장하는 그대로** 기계로 핀한다(Python 정본과 동일).
    ///
    /// 정본 문장(코드 주석·설계 문서·릴리스 노트 동일 문구):
    ///   "문장 문자가 **토큰 앞**에 오면 마커가 아니다. 토큰 **뒤**의 꼬리 텍스트는 마커를
    ///    무력화하지 못한다(실물 마커가 꼬리를 요구하므로 불가피). 따라서 마커 토큰으로
    ///    시작하는 설명·부정문은 은퇴로 읽힌다 — 마커를 설명하려면 토큰을 문장 뒤에 두어라."
    ///
    /// 문서와 코드가 갈리면 **없는 방어를 있다고 믿게 된다** — 그것이 이 프로젝트가 반복해서
    /// 당한 사고의 형태다. 실물 코퍼스에 아래 형태는 0건이고, 꼬리를 금지하면 실물 마커 10개가
    /// 죽는다. 결함이 아니라 계약의 경계이므로 계약으로 못 박는다.
    #[test]
    fn documented_defense_boundary_is_exactly_token_position() {
        // 토큰이 **앞** → 은퇴로 읽힌다(설명·부정문이어도).
        for line in [
            "<!-- **STALE 무효화** 는 이 파일에 해당하지 않는다 -->",
            "<!-- javis:todo-retired 마커는 레거시 표기다 -->",
            "<!-- RETIRED 는 은퇴 토큰이다 -->",
        ] {
            assert!(is_retire_marker_line(line), "{}", line);
            assert_eq!(verdict(&format!("{}\n# T\n- [ ] a\n", line)), "retired", "{}", line);
        }
        // 문장 문자가 **토큰 앞** → 마커가 아니다(권장 표기).
        for line in [
            "<!-- 이 파일은 STALE 무효화 대상이 아니다 -->",
            "<!-- 레거시 표기는 javis:todo-retired 다 -->",
            "<!-- 은퇴 토큰은 RETIRED 다 -->",
        ] {
            assert!(!is_retire_marker_line(line), "{}", line);
        }
    }

    /// `DECOR_CHARS`의 **유일한 계약**: 문장 문자(영숫자)가 하나도 없다.
    /// 한 글자라도 들어가면 그 글자로 시작하는 문장이 관문을 통과해 부정문 은퇴가 되살아난다.
    /// (Python 리터럴과의 동일성은 `test_todo_shared_constants.py`가 대조한다.)
    #[test]
    fn decor_charset_contains_no_sentence_characters() {
        for c in DECOR_CHARS.chars() {
            assert!(!c.is_alphanumeric(), "장식 집합에 문장 문자가 있다: {c:?}");
        }
        assert!(DECOR_CHARS.contains(' '));
        assert!(DECOR_CHARS.contains('★'));
    }

    /// ★후보 필터가 `javis:todo` **직후 공백**을 요구해야 하는 이유의 회귀 핀.
    /// 느슨한 필터(`contains("javis:todo")`)면 `<!-- javis:todo-retired -->`가 "깨진 v1 선언"으로
    /// 잡혀 `unclaimed`가 되고, G10(레거시 은퇴 인정)이 **가장 흔한 실물 표기에서** 무력화된다.
    /// 설계 프로토타입(`sim_declared_state.py`)이 실제로 이 구멍을 갖고 있었다 — 골든 픽스처
    /// `18-legacy-todo-retired.md`가 정본이며 Python 정본 파서도 같은 규칙을 쓴다.
    #[test]
    fn legacy_retire_marker_in_html_comment_is_not_shadowed_by_candidate_filter() {
        assert_eq!(verdict("<!-- javis:todo-retired -->\n# T\n- [ ] a\n"), "retired");
        // 반대 방향 — 공백이 있으면 후보이며, 버전이 없으므로 깨진 선언으로 진단된다.
        assert_eq!(
            parse("<!-- javis:todo retired -->\n").unwrap_err().code,
            diag::SYNTAX
        );
    }

    /// 후보가 되려면 `javis:todo`가 `<!--` **바로 뒤**여야 한다. 주석 안 아무 데나 적힌
    /// `javis:todo`는 선언이 아니라 산문이므로 "선언 없음"이다(문법 위반이 아니다).
    #[test]
    fn prose_mentioning_declaration_is_not_a_candidate() {
        for s in [
            "<!-- 참고: javis:todo v1 형식을 쓰세요 -->\n# T\n- [ ] a\n",
            "<!-- javis:todo-->\n# T\n- [ ] a\n",
            "설명문에 javis:todo v1 owner=w scope=pack status=active 가 나온다\n- [ ] a\n",
        ] {
            assert_eq!(verdict(s), "unclaimed", "{}", s);
            assert_eq!(parse(s).unwrap_err().code, diag::NO_DECL, "{}", s);
        }
    }

    #[test]
    fn skew_old_file_without_declaration_is_unclaimed() {
        assert_eq!(verdict("# T\n- [x] a\n- [ ] b\n"), "unclaimed");
    }

    /// 레거시 은퇴 마커는 **정상 선언이 있으면** 무시된다(선언이 유일한 진실 — ADR-1).
    #[test]
    fn valid_declaration_wins_over_legacy_marker() {
        let s = format!("<!-- STALE 무효화 -->\n{}\n- [ ] a\n", D);
        assert_eq!(verdict(&s), "counted");
    }

    // ── 문법 세부(G4~G6) ──

    #[test]
    fn unknown_keys_are_ignored_forward_compat() {
        let s = "<!-- javis:todo v1 owner=w scope=pack status=active lane=ghost-todo-fix since=2026-07-26 future=x -->\n- [ ] a\n";
        assert_eq!(verdict(s), "counted");
        let d = parse(s).expect("유효 선언");
        assert_eq!((d.owner.as_str(), d.scope.as_str(), d.status.as_str()), ("w", "pack", "active"));
    }

    #[test]
    fn invalid_status_value_is_rejected() {
        let s = "<!-- javis:todo v1 owner=w scope=pack status=done -->\n- [ ] a\n";
        assert_eq!(verdict(s), "unclaimed");
        let e = parse(s).unwrap_err();
        assert_eq!(e.code, diag::BAD_STATUS);
        assert_eq!(e.message, "status 값 위반: done");
    }

    #[test]
    fn all_required_keys_missing_lists_them_in_order() {
        assert_eq!(
            parse("<!-- javis:todo v1 lane=x -->\n").unwrap_err().message,
            "필수 키 누락: owner,scope,status"
        );
    }

    #[test]
    fn malformed_structure_diagnostics() {
        // `-->` 부재 / 버전 토큰 형태 위반 / 버전 뒤 공백 부재 / 후행 텍스트 — 전부 같은 진단.
        for s in [
            "<!-- javis:todo v1 owner=w scope=pack status=active\n",
            "<!-- javis:todo vx owner=w scope=pack status=active -->\n",
            "<!-- javis:todo v1-->\n",
            "<!-- javis:todo v1 owner=w scope=pack status=active -->x\n",
            "<!-- javis:todo v2 owner=w scope=pack status=active --> x -->\n",
        ] {
            assert_eq!(parse(s).unwrap_err().code, diag::SYNTAX, "{}", s);
        }
    }

    #[test]
    fn tight_and_loose_whitespace_forms_both_parse() {
        for s in [
            "<!--javis:todo v1 owner=w scope=pack status=active-->\n- [ ] a\n",
            "<!--   javis:todo   v1   owner=w   scope=pack   status=active   -->\n- [ ] a\n",
            "<!--\tjavis:todo\tv1\towner=w\tscope=pack\tstatus=active\t-->\n- [ ] a\n",
            // 선두 공백 3개까지는 선언 줄이다(G12 ④ — 4개부터 들여쓴 코드블록).
            "   <!-- javis:todo v1 owner=w scope=pack status=active -->\n- [ ] a\n",
        ] {
            assert_eq!(verdict(s), "counted", "{}", s);
        }
    }

    // ── ★G12 머리말 마스킹(W9 교정 1) ──────────────────────────────────────
    // 선언 문법을 **설명하는** todo가 자기 자신을 은퇴시키던 결함의 회귀 핀이다.
    // 이 프로젝트에서 가장 흔한 종류의 todo가 정확히 그 피해자였다(선언 도입 작업 자체).

    /// ★치명 회귀 핀 — 코드펜스 안의 문서용 예시 선언은 선언이 아니다.
    /// 무너지면 "선언 문법 안내" 문단을 가진 살아있는 todo가 통째로 사라진다.
    #[test]
    fn fenced_example_declaration_does_not_retire_the_file() {
        let r = "<!-- javis:todo v1 owner=w scope=pack status=retired -->";
        // ① 자기 선언(active) + 펜스 안 예시(retired) → 예시는 무시되고 살아남는다.
        for fence in ["```", "~~~", "````", "~~~~"] {
            let s = format!("{D}\n\n# 안내\n\n{fence}\n{r}\n{fence}\n\n- [ ] a\n- [ ] b\n");
            assert_eq!(verdict(&s), "counted", "{}", fence);
        }
        // ② 정보 문자열이 붙은 펜스도 펜스다.
        assert_eq!(
            verdict(&format!("{D}\n```markdown\n{r}\n```\n- [ ] a\n")),
            "counted"
        );
        // ③ 자기 선언 없이 예시만 있는 안내 문서 → 은퇴가 아니라 미선언이다.
        let s = format!("# 선언 문법 안내\n\n```\n{r}\n```\n\n- [ ] a\n- [ ] b\n");
        assert_eq!(verdict(&s), "unclaimed");
        assert_eq!(parse(&s).unwrap_err().code, diag::NO_DECL);
    }

    /// 펜스는 **여는 펜스와 같은 문자·같은 길이 이상**으로만 닫힌다.
    #[test]
    fn fence_closing_rules() {
        let r = "<!-- javis:todo v1 owner=w scope=pack status=retired -->";
        // 다른 문자·짧은 런은 닫지 못한다 → 선언은 여전히 펜스 안이다.
        for close in ["~~~", "``", "```` x"] {
            let s = format!("# T\n````\n{r}\n{close}\n{r}\n- [ ] a\n");
            assert_eq!(verdict(&s), "unclaimed", "close={}", close);
        }
        // 같은 문자로 더 길게 닫는 것은 허용(CommonMark).
        let s = format!("# T\n```\n예시\n`````\n{D}\n- [ ] a\n");
        assert_eq!(verdict(&s), "counted");
    }

    /// ★W13 중대 D 회귀 핀 — 미닫힘 펜스는 머리말 끝에서 **회수**한다(G12 ⑤).
    ///
    /// 무너지면 인라인 삼중백틱 한 줄이 그 아래 정당한 선언을 삼킨다(M3에서 파일이 집계에서
    /// 사라지고, 지금도 `unclaimed_ratio`를 왜곡한다). 반대 방향도 같은 무게로 핀한다 —
    /// 은퇴 선언이 삼켜지면 은퇴시켰다고 믿은 파일이 계속 집계된다.
    #[test]
    fn unclosed_fence_is_recovered_at_end_of_header() {
        let r = "<!-- javis:todo v1 owner=w scope=pack status=retired -->";
        // ① 살아있는 선언이 미닫힘 펜스에 삼켜지지 않는다.
        assert_eq!(verdict(&format!("# T\n```\n{D}\n\n- [ ] a\n")), "counted");
        // ② 은퇴 선언도 대칭으로 회수된다.
        assert_eq!(verdict(&format!("# T\n```\n{r}\n\n- [ ] a\n")), "retired");
        // ③ 레거시 마커 줄도 같다.
        assert_eq!(
            verdict("# T\n```\n<!-- ★ STALE 무효화 -->\n\n- [ ] a\n"),
            "retired"
        );
        // ④ 회수해도 ②인용·③들여쓰기는 그대로 적용된다.
        assert_eq!(verdict(&format!("# T\n```\n> {D}\n- [ ] a\n")), "unclaimed");
        assert_eq!(verdict(&format!("# T\n```\n    {D}\n- [ ] a\n")), "unclaimed");
        // ⑤ **닫힌** 펜스는 종전대로 마스킹된다(G12 ①의 본령은 그대로다).
        assert_eq!(verdict(&format!("# T\n```\n{r}\n```\n- [ ] a\n")), "unclaimed");
        // ⑥ 앞선 펜스가 정상적으로 닫혔으면 그 안의 예시는 회수 대상이 아니다.
        assert_eq!(
            verdict(&format!("# T\n```\n{r}\n```\n\n```\n{D}\n- [ ] a\n")),
            "counted"
        );
        // ⑦ ★W15 중대 ① — **정상 후보가 있으면 회수를 취소한다**(회귀 핀).
        //   W13 직후 이 자리는 `unclaimed`(G7 duplicate)였고 그것이 회귀였다 — 회수 도입
        //   **전에는** 펜스 안 예시가 마스킹돼 `counted`였다.
        assert_eq!(verdict(&format!("{D}\n```\n{r}\n- [ ] a\n")), "counted");
    }

    /// ★W15 중대 ① — 회수는 "선언이 없을 때의 구제책"이지 "선언을 늘리는 장치"가 아니다.
    ///
    /// 회수 구간과 정상 구간에 **둘 다** 후보가 있으면 회수를 취소한다(정상 후보 우선).
    /// 후보에는 v1 선언 후보와 **레거시 은퇴 마커 줄**이 모두 포함된다 — 한쪽만 세면 같은
    /// 충돌이 다른 토큰으로 재발한다. Python 정본 테스트와 같은 케이스다.
    #[test]
    fn unclosed_fence_recovery_yields_to_existing_candidate() {
        let r = "<!-- javis:todo v1 owner=w scope=pack status=retired -->";
        // ① 진짜 선언(정상) vs 회수될 예시 선언 → 진짜 선언이 이긴다.
        assert_eq!(verdict(&format!("{D}\n\n# 안내\n```\n{r}\n\n- [ ] a\n")), "counted");
        // ② 은퇴 선언(정상) vs 회수될 active 예시 → 은퇴가 이긴다(반대 방향).
        assert_eq!(verdict(&format!("{r}\n\n# 안내\n```\n{D}\n\n- [ ] a\n")), "retired");
        // ③ 정상 구간의 **레거시 마커** vs 회수될 선언 → 마커가 이긴다(교차 축).
        assert_eq!(
            verdict(&format!("<!-- ★ STALE 무효화 -->\n# T\n```\n{D}\n\n- [ ] a\n")),
            "retired"
        );
        // ④ 정상 구간에 후보가 없으면 회수는 종전대로 발동한다(W13 계약 불변).
        assert_eq!(verdict(&format!("# T\n```\n{D}\n\n- [ ] a\n")), "counted");
        // ⑤ 회수 구간 **안에서만** 후보가 2개면 종전대로 G7 모호성 거부다.
        assert_eq!(verdict(&format!("# T\n```\n{D}\n{r}\n- [ ] a\n")), "unclaimed");
    }

    /// 인용문·들여쓴 코드블록 안의 선언도 선언이 아니다(G12 ②③).
    #[test]
    fn quoted_and_indented_declarations_are_not_candidates() {
        let r = "<!-- javis:todo v1 owner=w scope=pack status=retired -->";
        for masked in [
            format!("> {r}"),        // 인용문
            format!(">{r}"),         // 공백 없는 인용
            format!("   > {r}"),     // 선행 공백 3개까지 인용
            format!("    {r}"),      // 들여쓴 코드블록(공백 4)
            format!("\t{r}"),        // 들여쓴 코드블록(탭)
            format!("        {r}"),  // 깊은 들여쓰기
        ] {
            // 자기 선언과 함께면 자기 선언만 남는다(중복 판정이 아니다).
            assert_eq!(verdict(&format!("{D}\n{masked}\n- [ ] a\n")), "counted", "{masked}");
            // 단독이면 미선언이다(은퇴가 아니다).
            let s = format!("# T\n{masked}\n- [ ] a\n");
            assert_eq!(verdict(&s), "unclaimed", "{masked}");
            assert_eq!(parse(&s).unwrap_err().code, diag::NO_DECL, "{masked}");
        }
    }

    /// 레거시 은퇴 마커도 같은 마스킹을 받는다 — 마커를 **설명하는** 문서가 자기를 은퇴시키면 안 된다.
    #[test]
    fn masked_legacy_retire_marker_is_not_honored() {
        for masked in [
            "```\n<!-- ★ STALE 무효화 -->\n```",
            "> <!-- javis:todo-retired -->",
            "    <!-- RETIRED 2026-07-11 -->",
        ] {
            let s = format!("# T\n{masked}\n- [ ] a\n");
            assert_eq!(verdict(&s), "unclaimed", "{masked}");
            assert_eq!(parse(&s).unwrap_err().code, diag::NO_DECL, "{masked}");
        }
    }

    // ── ★G11 개행·공백 수렴(W9 교정 2) ─────────────────────────────────────

    /// 개행은 `\n`·`\r\n`·`\r` 셋뿐이고, 그 밖의 유니코드 개행 후보는 **평범한 문자**다.
    /// Python `str.splitlines()`가 가르는 문자들이 여기서 줄을 가르면 즉시 파리티가 깨진다.
    #[test]
    fn only_lf_crlf_cr_split_lines() {
        assert_eq!(split_lines("a\nb\r\nc\rd"), ["a", "b", "c", "d"]);
        for c in ['\u{0b}', '\u{0c}', '\u{1c}', '\u{1d}', '\u{1e}', '\u{1f}',
                  '\u{85}', '\u{2028}', '\u{2029}'] {
            assert_eq!(split_lines(&format!("a{c}b")), [format!("a{c}b")], "{:?}", c);
        }
    }

    /// 토큰 구분 공백은 스페이스·탭뿐 — `White_Space`(NBSP·U+3000 …)로 넓히지 않는다.
    /// 넓은 쪽으로 맞추면 제어문자가 섞인 줄이 유효 선언으로 승격되어 오판정 표면이 늘어난다.
    #[test]
    fn token_separator_is_space_and_tab_only() {
        for c in ['\u{0b}', '\u{0c}', '\u{1c}', '\u{1f}', '\u{85}', '\u{a0}',
                  '\u{2028}', '\u{2029}', '\u{3000}'] {
            // 키 사이에 심으면 토큰이 갈리지 않아 문자 클래스 위반이다.
            let s = format!(
                "<!-- javis:todo v1 owner=w{c}scope=pack status=active -->\n- [ ] a\n");
            assert_eq!(verdict(&s), "unclaimed", "{:?}", c);
            assert_eq!(parse(&s).unwrap_err().code, diag::BAD_TOKEN, "{:?}", c);
            // 줄 꼬리에 심으면 `-->` 앵커가 깨진다(후행 공백으로 흡수되지 않는다).
            let s = format!("{D}{c}\n- [ ] a\n");
            assert_eq!(parse(&s).unwrap_err().code, diag::SYNTAX, "{:?}", c);
        }
        // 스페이스·탭은 정상 구분자다.
        assert_eq!(
            verdict("<!-- javis:todo v1 owner=w\tscope=pack \t status=active -->\n- [ ] a\n"),
            "counted"
        );
    }

    #[test]
    fn value_charset_boundaries() {
        // 허용: 영숫자 . _ : - / 거부: 그 밖(슬래시·한글·공백).
        let ok = "<!-- javis:todo v1 owner=worker-2 scope=pack.dept_1:a-b status=active -->\n";
        assert_eq!(parse(ok).expect("유효").scope, "pack.dept_1:a-b");
        for bad in ["scope=a/b", "scope=한글", "scope=", "=pack", "1owner=w", "scope=a=b"] {
            let s = format!("<!-- javis:todo v1 owner=w status=active {} -->\n", bad);
            assert!(parse(&s).is_err(), "{}", bad);
        }
    }

    #[test]
    fn duplicate_key_last_wins() {
        // 참조 구현의 dict 대입 의미론과 동일(마지막 값 채택).
        let s = "<!-- javis:todo v1 owner=a owner=b scope=pack status=active -->\n";
        assert_eq!(parse(s).expect("유효").owner, "b");
    }

    // ── 예산·경계(G3) ──

    #[test]
    fn declaration_beyond_head_budget_is_not_seen() {
        // 1 KiB 밖의 선언은 보이지 않는다(워치독 틱 보호 — 예산이 계약이다).
        // ★예산은 `head_from_bytes`에서만 적용되므로 **바이트 경로**로 검사한다.
        let pad = "x".repeat(HEAD_BYTES);
        assert_eq!(
            verdict_bytes(format!("{}\n{}\n- [ ] a\n", pad, D).as_bytes()),
            "unclaimed"
        );
        // 경계 바로 안쪽이면 보인다(예산이 "그냥 늘 안 보인다"가 아님을 핀).
        let pad = "x".repeat(HEAD_BYTES - D.len() - 2);
        assert_eq!(
            verdict_bytes(format!("{}\n{}\n- [ ] a\n", pad, D).as_bytes()),
            "counted"
        );
    }

    #[test]
    fn truncation_at_multibyte_boundary_does_not_panic() {
        // 한글(3바이트)로 경계를 정확히 가로지르게 채운 뒤 절단 — 패닉 0이 핵심.
        for extra in 0..4 {
            let s = format!("{}{}\n{}\n", "가".repeat(HEAD_BYTES), "x".repeat(extra), D);
            assert_eq!(verdict_bytes(s.as_bytes()), "unclaimed");
        }
    }

    /// ★**W14 S15 회귀 핀 — 예산은 원시 바이트 기준 1회뿐이다.**
    ///
    /// 무너지면(=`parse`가 디코드 문자열을 다시 자르면) 비UTF-8 파일에서 Rust만 짧게 본다.
    /// `from_utf8_lossy`는 잘못된 바이트 1개를 U+FFFD **3바이트**로 팽창시키므로, 두 번째
    /// 절단은 Python이 보는 영역을 잘라먹는다. 실측 재현이 그대로 이 케이스다 —
    /// 400 B의 `0xFF`(디코드 시 1200 B) 뒤의 선언을 Python은 보고 Rust는 못 봤다.
    /// 은퇴 선언 방향이 특히 위험하다: **은퇴한 파일을 데몬만 계속 집계**한다(유령 재발).
    #[test]
    fn non_utf8_expansion_does_not_shrink_the_budget() {
        for (status, want) in [("active", "counted"), ("retired", "retired")] {
            let decl = format!(
                "<!-- javis:todo v1 owner=worker scope=pack status={status} -->\n"
            );
            let mut raw: Vec<u8> = vec![0xff; 400]; // 디코드하면 1200 B — 1 KiB를 넘긴다
            raw.push(b'\n');
            raw.extend_from_slice(decl.as_bytes());
            raw.extend_from_slice(b"# T\n- [x] a\n- [ ] b\n");
            assert!(raw.len() < HEAD_BYTES, "원시 기준으로는 예산 안이어야 한다");
            assert_eq!(verdict_bytes(&raw), want, "status={status}");
        }
    }

    /// `head_from_bytes`는 **원시 바이트** 1 KiB만 잘라 넘기고 그 이상 손대지 않는다.
    /// 팽창한 디코드 문자열을 다시 자르는 코드가 어디에도 없다는 사실의 직접 핀.
    #[test]
    fn head_from_bytes_cuts_raw_bytes_only() {
        let raw = vec![0xffu8; HEAD_BYTES * 2];
        let head = head_from_bytes(&raw);
        // 1024개의 잘못된 바이트 → 1024개의 U+FFFD(각 3바이트) = 3072바이트.
        assert_eq!(head.chars().count(), HEAD_BYTES);
        assert_eq!(head.len(), HEAD_BYTES * 3);
    }

    #[test]
    fn huge_and_binaryish_input_is_safe() {
        assert_eq!(verdict(&"\u{0}\u{1}\u{feff}\r\n".repeat(4096)), "unclaimed");
        assert_eq!(verdict(&"- [ ] a\n".repeat(4096)), "unclaimed");
    }

    // ── classify 단독 ──

    #[test]
    fn classify_none_is_unclaimed_and_retired_skips_scope_lookup() {
        assert_eq!(classify(None, "pack", &packs), Verdict::Unclaimed);
        let retired = Decl {
            owner: "w".into(),
            scope: "pack-dept-dept-9".into(),
            status: "retired".into(),
            legacy: false,
        };
        // 은퇴는 scope 실재 검사를 **호출하지 않는다** — 콜백이 불리면 패닉으로 잡힌다.
        let never = |_: &str| -> bool { panic!("retired 분기는 scope 실재를 조회하면 안 된다") };
        assert_eq!(classify(Some(&retired), "pack", &never), Verdict::Retired);
    }

    /// ★ADR-4 C-3 불변식 — 레거시 은퇴 선언의 센티널 `"?"`가 **scope 판정에 새지 않는다**.
    ///
    /// 새면 `"?"`가 팩 이름으로 조회돼(당연히 부재) 은퇴 파일이 orphan-scope로 시끄럽게
    /// 쏟아진다 — G10이 막으려던 구→신 스큐 폭주가 다른 얼굴로 재발한다. 콜백이 불리면
    /// 패닉으로 즉시 잡는다(Python 정본은 예외를 던지는 콜백으로 같은 계약을 핀한다).
    #[test]
    fn legacy_sentinel_never_reaches_scope_lookup() {
        let never = |_: &str| -> bool { panic!("retired 분기가 scope 실재를 조회했다") };
        for line in [
            "<!-- javis:todo-retired -->",
            "<!-- ★ STALE 무효화 -->",
            "<!-- RETIRED 2026-07-11 -->",
        ] {
            let d = parse(&format!("{}\n# T\n- [ ] a\n", line)).expect("레거시 은퇴 선언");
            assert!(d.legacy && d.owner == "?" && d.scope == "?", "{}", line);
            assert_eq!(classify(Some(&d), "pack", &never), Verdict::Retired, "{}", line);
        }
        // 센티널이 살아있는 선언에 쓰이면 조용히 counted 되지 않고 orphan으로 잡힌다.
        let alive = Decl {
            owner: "?".into(),
            scope: "?".into(),
            status: "active".into(),
            legacy: false,
        };
        assert_eq!(classify(Some(&alive), "pack", &packs), Verdict::OrphanScope);
    }

    /// 7종 진단 코드가 전부 실제 입력으로 도달 가능해야 한다 — 죽은 코드는 계약이 아니다.
    /// (Python 정본에도 같은 이름의 도달성 테스트가 있다.)
    #[test]
    fn every_diag_code_is_reachable() {
        let samples: [(&str, String); 7] = [
            (diag::NO_DECL, "# T\n- [ ] a\n".to_string()),
            (diag::DUPLICATE, format!("{}\n{}\n", D, D)),
            (diag::SYNTAX, format!("{} x\n", D)),
            (
                diag::UNKNOWN_VERSION,
                "<!-- javis:todo v9 owner=w scope=pack status=active -->\n".to_string(),
            ),
            (
                diag::BAD_TOKEN,
                "<!-- javis:todo v1 owner=\"w\" scope=pack status=active -->\n".to_string(),
            ),
            (
                diag::MISSING_KEYS,
                "<!-- javis:todo v1 owner=w scope=pack -->\n".to_string(),
            ),
            (
                diag::BAD_STATUS,
                "<!-- javis:todo v1 owner=w scope=pack status=done -->\n".to_string(),
            ),
        ];
        let mut covered: Vec<&str> = Vec::new();
        for (code, head) in &samples {
            let e = parse(head).unwrap_err();
            assert_eq!(&e.code, code, "입력: {head:?}");
            covered.push(e.code);
        }
        covered.sort_unstable();
        let mut all = diag::ALL.to_vec();
        all.sort_unstable();
        assert_eq!(covered, all, "도달하지 못한 진단 코드가 있다");
    }

    // ── 골든 픽스처 파리티(ADR-2 — 계약의 SOT는 이 파일이 아니라 픽스처다) ──

    /// `cysjavis-pack/bin/tests/fixtures/todo-decl/` 를 읽어 `expected.json`과 대조한다.
    /// Python 정본(`javis_todo_decl.py`)이 **같은 파일·같은 기대값**으로 검사되므로, 이 테스트가
    /// 초록이면 두 언어가 같은 계약 위에 있다는 뜻이다(리스크 R1 = 2언어 drift 차단).
    ///
    /// 픽스처가 없으면 **hard fail** 한다 — 조용히 skip하면 파리티 CI가 형식만 갖춘 껍데기가 되고,
    /// 계측기 실패가 합격으로 둔갑한다(계측 타당성 게이트).
    #[test]
    fn golden_fixture_parity() {
        use std::path::Path;
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("cysjavis-pack/bin/tests/fixtures/todo-decl");
        let raw = std::fs::read_to_string(dir.join("expected.json")).unwrap_or_else(|e| {
            panic!("골든 픽스처 대장을 읽을 수 없다({}): {e} — 계약 SOT 부재는 skip이 아니라 실패다", dir.display())
        });
        let spec: serde_json::Value = serde_json::from_str(&raw).expect("expected.json 파싱");
        let head_bytes = spec["head_bytes"].as_u64().expect("head_bytes") as usize;
        assert_eq!(head_bytes, HEAD_BYTES, "파싱 예산(G3)이 픽스처 계약과 다르다");
        let my_scope = spec["my_scope"].as_str().expect("my_scope");
        let existing: Vec<String> = spec["existing_scopes"]
            .as_array()
            .expect("existing_scopes")
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect();
        let scope_exists = |s: &str| existing.iter().any(|e| e == s);

        let cases = spec["cases"].as_object().expect("cases");
        assert!(cases.len() >= 15, "픽스처 케이스가 15종 미만이다: {}", cases.len());
        let mut names: Vec<&String> = cases.keys().collect();
        names.sort();
        for name in names {
            let want = &cases[name];
            // 픽스처 읽기 계약: 선두 head_bytes **바이트** → lossy UTF-8 디코드.
            let bytes = std::fs::read(dir.join(name))
                .unwrap_or_else(|e| panic!("픽스처 {name} 읽기 실패: {e}"));
            let head = head_from_bytes(&bytes); // 예산은 위에서 픽스처 계약과 동일함을 확인했다
            let parsed = parse(&head);

            let got = classify(parsed.as_ref().ok(), my_scope, &scope_exists).as_str();
            assert_eq!(got, want["classify"].as_str().expect("classify"), "[{name}] 판정");

            // ★대조는 **언어중립 diag_code**로 한다(ADR-4 C-2). 한국어 문구는 계약이 아니라
            // UX 어포던스이므로 픽스처에 아예 싣지 않는다 — 문구를 계약에 넣으면 문구를
            // 다듬는 순간 이 테스트가 결함을 오보한다.
            match (&parsed, want["diag_code"].as_str()) {
                (Err(e), Some(want_code)) => {
                    assert_eq!(e.code, want_code, "[{name}] 진단 코드(G9) · 문구={}", e.message)
                }
                (Ok(_), None) => {}
                (Ok(d), Some(w)) => panic!("[{name}] 진단 {w:?}를 기대했으나 선언이 파싱됐다: {d:?}"),
                (Err(e), None) => panic!("[{name}] 유효 선언을 기대했으나 미선언: {e}"),
            }

            // decl 대조는 필수 3키 + _legacy만 한다(픽스처 계약이 명시적으로 허용 —
            // 이 Rust 구현은 전방호환 목적상 모르는 키를 보존하지 않는다).
            // 레거시 은퇴 선언도 예외가 아니다: 센티널 `"?"`가 양 언어 공통 표현이므로
            // owner/scope까지 그대로 대조한다(ADR-4 C-3 — 예외를 두면 표현 drift가 숨는다).
            if let Ok(d) = &parsed {
                let w = &want["decl"];
                assert!(w.is_object(), "[{name}] decl 기대값이 객체가 아니다");
                assert_eq!(d.legacy, w["_legacy"].as_bool().unwrap_or(false), "[{name}] _legacy");
                assert_eq!(d.status, w["status"].as_str().unwrap_or_default(), "[{name}] status");
                assert_eq!(d.owner, w["owner"].as_str().unwrap_or_default(), "[{name}] owner");
                assert_eq!(d.scope, w["scope"].as_str().unwrap_or_default(), "[{name}] scope");
            }
        }
    }

    /// 계약 파일이 스스로 코드 집합을 싣고 있고, 이 구현의 `diag::ALL`과 **순서까지** 같다.
    /// (Python `DIAG_CODES`도 같은 목록을 같은 순서로 갖는다 — 3자 동시 고정.)
    #[test]
    fn diag_code_set_matches_contract_file() {
        use std::path::Path;
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("cysjavis-pack/bin/tests/fixtures/todo-decl/expected.json");
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("계약 파일을 읽을 수 없다({}): {e}", path.display()));
        let spec: serde_json::Value = serde_json::from_str(&raw).expect("expected.json 파싱");
        let listed: Vec<&str> = spec["_diag_codes"]
            .as_array()
            .expect("_diag_codes — 계약 파일이 코드 집합을 선언해야 한다")
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert_eq!(listed, diag::ALL.to_vec(), "진단 코드 집합이 계약 파일과 어긋난다");
        // 문구가 계약 파일로 되돌아오는 회귀 차단(ADR-4 C-2).
        for (name, case) in spec["cases"].as_object().expect("cases") {
            assert!(case.get("diag").is_none(), "[{name}] 한국어 문구는 계약이 아니다");
        }
    }

    #[test]
    fn verdict_serialization_contract() {
        // 파리티 CI의 비교 표현 — 이 문자열이 바뀌면 Python 구현과 즉시 갈린다.
        assert_eq!(
            [
                Verdict::Counted,
                Verdict::Retired,
                Verdict::ForeignScope,
                Verdict::OrphanScope,
                Verdict::Unclaimed,
            ]
            .map(|v| v.as_str()),
            ["counted", "retired", "foreign-scope", "orphan-scope", "unclaimed"]
        );
    }
}
