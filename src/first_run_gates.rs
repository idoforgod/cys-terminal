//! 첫기동 관문(first-run gate) 코퍼스 — **코드 임베드 정본(SOT)** · U-12.
//!
//! ## 왜 코드에 두는가 (K-1)
//! `cysjavis-pack/agents.json` 은 `Ownership::User`(pack.rs)다. **기존 설치 기계에는
//! `ready_marker`·`approval_patterns` 가 값으로 이미 있으므로**, 벤더가 그 값을 고쳐 출하해도
//! `cys.rs fill_missing_fields` 의 "키가 아예 없을 때만 보강" 규칙에 막혀 **결함이 있는 바로 그
//! 기계들에는 영영 도달하지 않는다**. 그래서 관문 판정에 쓰이는 데이터는 `agents.json` **값
//! 수정**으로 배달할 수 없고, 배달 가능한 경로는 둘뿐이다 —
//!   ⓐ **코드 임베드**(새 바이너리 = 새 코퍼스. 이 파일이 그것이다)
//!   ⓑ **신규 키**(디스크에 부재 → 계층이 채운다. `agents.json` 의 `first_run_gates` 봉투가 그것).
//! ⓐ가 정본이고 ⓑ는 **override 전용 봉투**다(코퍼스를 복사해 두지 않는다 — 사본이 늘면 S-1
//! 샷건 서저리가 재발한다).
//!
//! ## 이 파일이 하지 않는 것
//! **readiness 판정을 하지 않는다**(U-13 소관). 여기 있는 것은 데이터와 순수 판별기뿐이고,
//! 어떤 키도 스스로 보내지 않는다. `action` 은 "그 관문을 통과시키려면 무엇을 눌러야 하는가"의
//! **선언**이며, 실제 전송은 뒤 단위(U-14/U-19)가 자기 게이트 아래에서 한다.
//!
//! ## 실측 근거 (2026-08-23 · macOS + Windows 실기 · claude 2.1.241)
//! 관문 6종의 **실제 순서**: 테마 → 로그인방식 → OAuth → 폴더신뢰 → 면책 → 새기능안내.
//! - 6화면 **전부에 `❯` 가 있다** = `agents.json` 의 `ready_marker` 와 같은 문자.
//!   → 신규 프로필에서 마커 기반 readiness 는 **필연 오탐**이다(그래서 U-13 이 필요하다).
//! - 면책 창 기본 포커스 = `No, exit` · Return → **rc 1 종료**. 통과 = 아래 1회 + Return.
//! - 폴더신뢰 창 기본 포커스 = `Yes, I trust this folder` → Return **안전**.
//! - 2026-07-29 실사고("기계 Return 이 폴더신뢰창을 종료시킨다")의 진범은 폴더신뢰 창이 아니라
//!   **그 직후의 면책 창**이었다 — 확인 에코 `Yes, I trust this folder ✔` 가 구 needle
//!   `trustthisfolder` 에 재매칭돼 2발째 Return 이 면책 창의 `No, exit` 를 눌렀다.
//!   → 이 코퍼스의 `needles` 는 **질문형 문면만** 담고, 확인 에코·버튼 라벨은
//!     `confirm_echo` 에 따로 적어 **관문의 근거가 아님**을 못박는다(불변식 검체 있음).
//! - 로그인·OAuth 는 **기계가 통과시킬 수 없다**: Return 은 브라우저를 열고 코드 입력
//!   프롬프트에 갇히며 이후 Return 은 무한 재시도다(프로세스는 계속 살아 있어 허위 READY 를
//!   영구화한다). 자격증명은 config dir **절대경로 sha256 로 봉인**되어 파일 복사로도 브리지
//!   불가(mac Keychain). → 액션을 정의하지 않고 "사람 1회 필요"로 표시한다.
//!
//! ## 버전 핀
//! 벤더가 새 기능을 낼 때마다 관문이 는다(실측: 6번째 `Try the new fullscreen renderer?` 가
//! 조사 목록에 없다가 Windows 실기에서 발견됐다). 화면 문면 대조는 구조적으로 이 드리프트를
//! 못 이긴다. 그래서 `measured_on` 이 지금 도는 바이너리 버전과 **다르거나 미측정**이면
//! `action_policy` 가 액션을 **보류**하고 관측만 허용한다 — 측정 불능은 통과가 아니다.
//! 보류의 귀결은 언제나 '아무 키도 보내지 않음'이므로, 이 게이트는 **오살 방향으로 열리지 않는다**.
//!
//! ## 롤백 스위치 (env 1지점)
//! `CYS_FIRST_RUN_GATES_OVERRIDE=0` → `agents.json` 봉투 파싱을 통째로 끄고 코드 정본만 쓴다.
//! 읽는 곳은 `override_enabled()` 하나뿐이고, 판정은 순수 `override_enabled_from()` 에 있다.
//!
//! ## ★부재의 비용 — 왜 '버린다' 가 자기규칙의 집행 수단이 될 수 없는가 (P4-10 · 2026-08-24)
//!
//! 자기규칙(아래 ⓐⓑⓒ)이 프로덕션에서 집행되기 시작한 첫 판(P4-3)은 위반 관문을 **버렸다**.
//! 이종 리뷰어 둘이 동시에 critical 로 지목한 그 판의 두 귀결이 이 절의 근거다.
//!
//! **① 사용자 주권 침해** — 사용자가 `agents.json` 에 `source=replace` 로 자기 코퍼스 하나를
//!   선언하면, 그 선언은 코드 정본에 위젯 AND 가드가 없다는 이유로 버려지고 "집행 후 코퍼스가
//!   비면 코드 정본으로 되돌린다" 는 폴백이 **코드 6종을 다시 세웠다**. 사용자가 선언한 것이
//!   조용히 벤더 정본으로 뒤집히는 것은 이 파일이 지키려는 계약(디스크 선언 > 임베드)의 반대다.
//!
//! **② 실패 방향의 역전(재난 ④)** — 봉투가 `bypass-disclaimer` 의 위젯을 비우면 종전 귀결은
//!   "needle 하나로 관문 성립 → **보류**"(안전측 오탐)였다. 버리기 시작한 뒤의 귀결은
//!   "관문 없음 → **주입**" 이다. 면책 창의 기본 포커스는 `No, exit` 이고 그 Return 은 rc 1 이므로
//!   **집행이 안전한 오탐을 좌석 사망으로 바꿔 놨다**. 규칙을 지키려다 규칙이 지키려던 것을 죽인다.
//!
//! ### 그래서 집행 수단은 '버리기' 가 아니라 **수리(repair)** 다
//!
//! | 위반 관문의 정체 | 집행 | 근거 |
//! |---|---|---|
//! | 빌트인 대응물이 **있다**(봉투가 정본을 덮은 것) | 위반 축을 **정본 선언으로 복원** | 관문을 잃지 않은 채 AND 구멍이 닫힌다. BLOCK-1 의 봉투 공격이 정확히 이 경로이고, 복원은 그 공격을 무효로 만들면서 관문은 남긴다. |
//! | 빌트인에 **없다**(사용자 신설 관문) | needle 이 정상 화면에 걸리는 것만 **좁히고**, 관문 자체는 **유지**(사유는 `notes`) | 버리는 것은 "사용자를 조용히 무시" 하는 것이다. 그리고 좁히기가 뒤집는 귀결은 **정상 화면 위에서의 보류**뿐이라 — 그 화면에서는 주입이 애초에 옳다 — 위험 방향이 아니다. |
//!
//! 어느 경우에도 **집행은 관문을 코퍼스에서 제거하지 않는다.** 그것이 성질 ②(실패 방향 불역전)를
//! 구조로 보장하는 유일한 방법이다: 제거가 없으면 `보류 → 주입` 으로 뒤집힐 자리도 없다.
//!
//! ### 부재의 비용은 관문마다 다르다 — [`AbsenceCost`]
//!
//! 이 파일의 종전 비용표는 "오탐(영구 라이브락) > 미탐" 한 줄이었다. 그 표는 **관문 전체를
//! 뭉뚱그린다**. 실제로는 면책·신뢰 계열(킬체인 관문)에서 부호가 반대다 —
//!
//! | 관문 | 오탐(관문이 아닌데 잡음) | **부재**(관문인데 코퍼스에 없음) |
//! |---|---|---|
//! | `theme` | 영구 부트 라이브락 | 주입 Return 이 기본 포커스(= 통과 액션)를 눌러 **그냥 통과**한다 → 가역 |
//! | `bypass-disclaimer` | 영구 부트 라이브락 | 주입 Return 이 `No, exit` 를 눌러 **rc 1 좌석 사망** → 비가역 |
//! | `folder-trust` | 영구 부트 라이브락 | 2026-07-29 킬체인의 1발째 자리 — 놓치면 2발째가 면책 창을 누른다 → 비가역 |
//! | `login-method`·`oauth-code` | 영구 부트 라이브락 | Return 이 브라우저·무한 재시도로 가고 **프로세스는 살아 있다** → 허위 READY 영구화 → 비가역 |
//! | `feature-announce-fullscreen` | 영구 부트 라이브락 | 기본 포커스 `Yes, try it` 수락 = 대체 화면·마우스 보고 → **화면을 읽는다는 관측 전제 자체가 무너진다** → 비가역 |
//!
//! 그래서 관문마다 부재의 비용을 **선언**하고([`Def::absence_cost`]), 집행이 그 비용을 넘지
//! 않는지 [`resolve_with`] 가 집행 전후를 대조해 확인한다(넘었으면 되살린다).

use serde_json::Value;

/// 이 코퍼스를 실측한 claude 바이너리 버전. 관문 액션의 유효 조건이다.
pub const MEASURED_ON: &str = "2.1.241";

/// `agents.json` 의 어댑터 스펙에서 override 봉투를 담는 **신규 키**.
/// 기존 디스크 파일에는 **없으므로** `cys.rs fill_missing_fields` 계층이 채운다 → 기존 기계 도달.
pub const ADAPTER_KEY: &str = "first_run_gates";

/// ★롤백 스위치(env 1지점). `0`/`off`/`false`/`no` → override 파싱 비활성(코드 정본만).
pub const OVERRIDE_ENV: &str = "CYS_FIRST_RUN_GATES_OVERRIDE";

/// 기계가 이 관문을 통과시킬 수 있는가.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Passability {
    /// 통과 액션이 실재한다(그래도 집행은 `action_policy` 의 버전 핀을 통과해야 한다).
    Machine,
    /// **사람이 1회** 해야 한다 — 액션을 정의하지 않는다(정의할 수 없다).
    HumanOnly,
}

/// 관문 통과 액션 — "세로 리스트에서 몇 번째 항목을 고르는가"의 선언.
/// 실제 키 시퀀스는 `Gate::down_presses()`(아래키 횟수) + Return 이며, 실측상 리터럴 숫자 입력도 등가다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateAction {
    /// 선택할 항목(1-based · 화면에 보이는 번호).
    pub select_index: u8,
    /// 그 항목의 라벨(사람 확인용 · 판정 근거 아님).
    pub label: String,
    /// 등가 리터럴 입력(예: `"2"`). 없으면 방향키+Return 만.
    pub literal: Option<String>,
}

/// ★관문이 **코퍼스에서 사라졌을 때** 치르는 값(모듈 doc 의 비용표가 근거다).
///
/// 오탐(관문이 아닌데 잡음)의 비용은 관문마다 같다 — 영구 부트 라이브락. 그러나 **부재**의
/// 비용은 관문마다 다르고, 킬체인 관문에서는 그것이 오탐보다 비싸다. 자기규칙 집행처럼
/// "관문을 줄이는" 조작은 이 값을 넘어설 수 없다([`resolve_with`] 의 대조가 집행한다).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbsenceCost {
    /// ★킬체인 — 부재의 귀결이 **비가역**이다(좌석 rc 1 종료 · 허위 READY 영구화 · 관측 전제 파괴).
    Fatal,
    /// 부재의 귀결이 **가역**이다 — 그 화면을 한 번 놓치고, 뒤 단위의 다른 축이 한 번 더 본다.
    Recoverable,
}

/// 이 관문 선언이 어디서 왔는가.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// 코드 임베드 정본.
    Builtin,
    /// 코드 정본 위에 `agents.json` 봉투가 덮어쓴 것.
    Overridden,
    /// `agents.json` 봉투가 새로 선언한 것(코드 정본에 없는 id).
    Added,
}

/// 첫기동 관문 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gate {
    pub id: String,
    pub title: String,
    /// **식별 문면 — 질문형만.** 확인 에코·버튼 라벨은 절대 넣지 않는다(`confirm_echo` 참조).
    pub needles: Vec<String>,
    /// 위젯 서명 — 전부 화면에 있어야 관문으로 본다(AND).
    pub widget: Vec<String>,
    /// 확인 에코·버튼 라벨. **관문 존재의 근거가 아니다.** 어떤 needle 도 여기에 부분일치하면
    /// 안 된다(불변식 검체가 집행 — 2026-07-29 킬체인의 형태).
    pub confirm_echo: Vec<String>,
    pub passability: Passability,
    /// 실측 기본 포커스(1-based). Return 만 눌렀을 때 선택되는 항목.
    /// ★면책 창은 이 값이 `1`(=`No, exit`)이라서 Return 한 발이 좌석을 죽인다.
    pub default_index: Option<u8>,
    pub action: Option<GateAction>,
    /// `HumanOnly` 인 이유(사람에게 보여줄 처방 근거).
    pub human_reason: Option<String>,
    /// ★이 관문이 코퍼스에서 사라졌을 때의 비용. 집행이 넘을 수 없는 상한이다.
    pub absence_cost: AbsenceCost,
    pub measured_on: String,
    pub origin: Origin,
}

impl Gate {
    /// 이 관문의 **부재**가 비가역인가(킬체인 관문인가).
    pub fn absence_is_fatal(&self) -> bool {
        self.absence_cost == AbsenceCost::Fatal
    }

    /// 기본 포커스에서 목표 항목까지 필요한 **아래키 횟수**.
    /// 위로 올라가야 하거나(음수) 기본 포커스가 미측정이면 `None` = **보류**(fail-closed).
    pub fn down_presses(&self) -> Option<u8> {
        let a = self.action.as_ref()?;
        let d = self.default_index?;
        a.select_index.checked_sub(d)
    }

    /// 이 화면이 이 관문인가. needle(OR) ∧ 위젯 서명(AND).
    ///
    /// 매칭은 **공백 정규화본**과 **공백 제거본** 양쪽에 건다: TUI 폭에 따라 프롬프트가 접히면
    /// 원문 매칭이 깨지고(그 대가가 '노드 0 + 고아 좌석'이다), 박스 렌더는 공백을 임의로 넣는다.
    /// 공백 제거본이 위험해지는 것은 needle 이 **짧고 에코에 포함될 때**뿐인데, 그 조건은
    /// `no_needle_is_contained_in_any_confirm_echo` 불변식이 원천 차단한다.
    pub fn matches(&self, screen: &str) -> bool {
        let norm = normalize(screen);
        let flat = flatten(screen);
        let hit = |s: &String| norm.contains(&normalize(s)) || flat.contains(&flatten(s));
        if !self.needles.iter().any(hit) {
            return false;
        }
        self.widget.iter().all(hit)
    }
}

/// 공백을 1칸으로 접는다(줄바꿈·들여쓰기 흡수).
pub fn normalize(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 공백을 전부 제거한다(박스 렌더·열 정렬 흡수).
pub fn flatten(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

// ═══════════════════════════════════════════════════════════════════════════
// 코퍼스 정본 — 아래 표가 SOT 다. 값을 고칠 때는 **실측 근거**를 같은 커밋에 남긴다.
// ═══════════════════════════════════════════════════════════════════════════

struct Def {
    id: &'static str,
    title: &'static str,
    needles: &'static [&'static str],
    widget: &'static [&'static str],
    confirm_echo: &'static [&'static str],
    passability: Passability,
    default_index: Option<u8>,
    /// (select_index, label, literal)
    action: Option<(u8, &'static str, Option<&'static str>)>,
    human_reason: Option<&'static str>,
    /// ★이 관문이 코퍼스에서 **사라졌을 때** 무엇이 일어나는가(모듈 doc 비용표).
    /// 선언 규율: `Fatal` 은 부재의 귀결이 **비가역**(좌석 종료·허위 READY 영구화·관측 전제
    /// 파괴)일 때만 쓴다. 그리고 각 Def 에 그 귀결을 한 줄로 적는다 — 근거 없는 `Fatal` 은
    /// 집행을 무력화하고, 근거 없는 `Recoverable` 은 좌석을 죽인다.
    absence_cost: AbsenceCost,
}

/// ★관문 정본 6종(실측 순서).
///
/// **선언 규율**(집행: `corpus_self_rule_*` · `no_needle_alone_matches_a_non_gate_screen`):
///   · `needles` = 그 관문이 떠 있을 때만 나오는 **질문·지시형 문면**. 인사 배너·상태 메시지·
///     에러 문자열은 정상 화면에도 나오므로 절대 넣지 않는다.
///   · `widget`  = 그 화면의 **위젯 서명**(AND). 비워 두지 않고, `❯` 처럼 6화면과 정상 화면에
///     **모두** 있는 보편 토큰을 단독으로 쓰지 않는다 — 그러면 AND 가 무의미해진다.
///   · 선택지 라벨은 식별(needle)이 아니라 서명(widget) 자리에 둔다.
const DEFS: &[Def] = &[
    // ── ① 테마 선택 ────────────────────────────────────────────────────────
    Def {
        id: "theme",
        title: "온보딩 · 테마 선택",
        // ★BLOCK-1 수리(2026-08-24 리뷰어 e2e): 종전 needle 에 **인사 배너**
        //   `"Welcome to Claude Code"` 가 있었고 위젯은 `❯` 하나뿐이었다. 그 배너는 온보딩이
        //   끝난 **정상 세션에도** 뜨고 `❯` 는 모든 claude 프롬프트에 있으므로, 둘의 AND 는
        //   사실상 "정상 화면"을 선언한 것과 같았다 — 건강한 노드가 `관문 보류: 온보딩 · 테마
        //   선택` 으로 잡혀 rc 78 · 디렉티브 미주입 · **영구 부트 라이브락**(화면에 통과시킬
        //   관문이 없으므로 사람도 못 푼다)이 됐다. 지금은 이 선택기에서만 나오는 질문형
        //   문면 하나만 needle 로 두고, 위젯은 **테마 목록 항목**(다른 화면에 존재하지 않는다)
        //   으로 AND 를 세운다. 규칙 집행은 `corpus_self_rules_*` 검체.
        needles: &["Choose the text style that looks best with your terminal"],
        widget: &["Auto (match terminal)", "Dark mode"],
        // 선택 직후 화면에 남는 체크 에코. 관문 근거가 아니다.
        confirm_echo: &["Dark mode ✔", "Light mode ✔", "Auto (match terminal) ✔"],
        passability: Passability::Machine,
        // 실측: `❯ 2. Dark mode ✔` 가 기본 포커스.
        default_index: Some(2),
        // 기본 포커스를 그대로 확정한다(아래키 0회 + Return) — 종전 동작에서 벗어나지 않는다.
        action: Some((2, "Dark mode", None)),
        human_reason: None,
        // 부재의 귀결: 주입 Return 이 기본 포커스(2 = Dark mode)를 누르고 그것이 곧 통과
        // 액션이다 → 관문을 **정상 통과**한다. 남는 결과는 테마 하나이며 사람이 되돌릴 수 있다.
        absence_cost: AbsenceCost::Recoverable,
    },
    // ── ② 로그인 방식 선택 ────────────────────────────────────────────────
    Def {
        id: "login-method",
        title: "로그인 방식 선택",
        // ★BLOCK-1 동반 수리: 이 관문도 위젯이 `❯` 단독이었다(리뷰어가 지목하지 않았지만
        //   ⑤ 자기규칙 검사가 같은 형태로 잡아낸 세 번째 사례다). 선택지 라벨은 **식별**이
        //   아니라 **위젯 서명**이 제자리다 — 라벨만으로는 이 화면이 떠 있음을 뜻하지 않고
        //   (요금제 안내문·문서에도 실린다), 질문형 프롬프트와 AND 로 묶여야 관문이 된다.
        needles: &["Select login method"],
        widget: &["Claude account with subscription", "Anthropic Console account"],
        confirm_echo: &[],
        // ★기계 통과 불가. Return 은 브라우저를 열고 ③으로 갈 뿐이다.
        passability: Passability::HumanOnly,
        default_index: Some(1),
        action: None,
        human_reason: Some(
            "OAuth 브라우저 로그인 — 기계가 대신할 수 없다. 자격증명은 CLAUDE_CONFIG_DIR \
             절대경로의 sha256 로 봉인되므로(mac Keychain `Claude Code-credentials-<8hex>`) \
             다른 프로필에서 복사해 올 수도 없다. 새 기계·새 부서마다 사람이 1회 로그인해야 한다.",
        ),
        // 부재의 귀결: 주입 Return 이 브라우저를 열고 좌석은 OAuth 대기에 갇힌 채 **살아 있다**
        // → 생존만 보는 판정이 그 좌석을 영원히 '준비됨'으로 읽는다(허위 READY 영구화 · 비가역).
        absence_cost: AbsenceCost::Fatal,
    },
    // ── ③ OAuth 코드 붙여넣기 ─────────────────────────────────────────────
    Def {
        id: "oauth-code",
        title: "OAuth 코드 입력",
        // ★BLOCK-2 수리(2026-08-24 리뷰어 e2e): 종전에는 위젯 AND 가 **하나도 없고**(`&[]`)
        //   needle 넷이 화면 전문에 그대로 걸렸다. 그중 둘은 질문형이 아니라 상태·에러
        //   문자열이었다 — `"Opening browser to sign in"`(다른 CLI 의 브라우저 로그인 화면·
        //   로그 한 줄에도 나온다) · `"OAuth error: Invalid code"`(그 문자열을 **grep 한 출력**
        //   에도 나온다). 그래서 claude 와 무관한 화면이 `oauth-code(human_only=true)` 로
        //   식별돼 주입 거부 + 다른 CLI 에 대한 오처방이 났다. 둘을 제거하고, 이 화면에만
        //   실재하는 **인가 URL** 로 AND 가드를 세운다(세로 리스트가 아니라 텍스트 입력
        //   프롬프트라 `❯` 는 여기서 위젯이 아니다).
        needles: &[
            "Browser didn't open? Use the url below to sign in",
            "Paste code here if prompted",
        ],
        widget: &["claude.com/cai/oauth/authorize"],
        confirm_echo: &[],
        passability: Passability::HumanOnly,
        default_index: None,
        action: None,
        human_reason: Some(
            "브라우저에서 받은 코드는 사람만 얻는다. 빈 Return 은 'Invalid code · Press Enter to \
             retry' 무한 루프이고 **프로세스는 계속 살아 있다** — 생존만 보는 판정은 이 좌석을 \
             영원히 '준비됨'으로 오탐한다(허위 READY 의 영구화 경로).",
        ),
        // 부재의 귀결: 빈 Return 이 'Invalid code · Press Enter to retry' 무한 루프에 들어가고
        // 프로세스는 계속 살아 있다 → 허위 READY 영구화(비가역).
        absence_cost: AbsenceCost::Fatal,
    },
    // ── ④ 폴더 신뢰 ───────────────────────────────────────────────────────
    Def {
        id: "folder-trust",
        title: "작업 폴더 신뢰 확인",
        needles: &[
            // 2.1.241 실측 문면(질문형).
            "Is this a project you created or one you trust",
            "Quick safety check",
            // 구 문면 — agents.json trust-prompt 선언과 같은 문면(하위호환).
            "Do you trust the files in this folder",
            "Do you trust this folder",
        ],
        widget: &["Enter to confirm", "Esc to cancel"],
        // ★2026-07-29 킬체인의 실체: 이 에코가 구 needle `trustthisfolder` 에 재매칭됐다.
        confirm_echo: &["Yes, I trust this folder", "No, exit"],
        passability: Passability::Machine,
        // 실측: 기본 포커스가 `Yes, I trust this folder` → Return 이 안전하다.
        default_index: Some(1),
        action: Some((1, "Yes, I trust this folder", None)),
        human_reason: None,
        // ★부재의 귀결: 이 창은 2026-07-29 킬체인의 **1발째 자리**다. 여기를 관문으로 잡지
        //   못하면 주입이 계속되고, 확인 에코가 남은 다음 화면(면책)에서 2발째 Return 이
        //   `No, exit` 를 누른다 — 실사고의 정확한 경로이며 좌석은 rc 1 로 죽는다(비가역).
        absence_cost: AbsenceCost::Fatal,
    },
    // ── ⑤ Bypass Permissions 면책 ─────────────────────────────────────────
    Def {
        id: "bypass-disclaimer",
        title: "Bypass Permissions 면책 확인",
        needles: &[
            "WARNING: Claude Code running in Bypass Permissions mode",
            "In Bypass Permissions mode, Claude Code will not ask for your approval",
        ],
        widget: &["Enter to confirm", "Esc to cancel"],
        confirm_echo: &["Yes, I accept", "No, exit"],
        passability: Passability::Machine,
        // ★★실측: 기본 포커스가 `1. No, exit` 다 — **Return 한 발이 rc 1 로 좌석을 죽인다.**
        //   그래서 이 관문만은 "Return 이 안전한가"를 절대 추정하면 안 된다.
        default_index: Some(1),
        action: Some((2, "Yes, I accept", Some("2"))),
        human_reason: None,
        // ★★부재의 귀결: 기본 포커스가 `No, exit` 이므로 주입의 **Return 한 발이 rc 1** 이다.
        //   좌석이 죽으면 되돌릴 것이 없다 — 이 코퍼스에서 부재가 가장 비싼 관문이고, 재난 ④
        //   (집행이 보류를 주입으로 뒤집는다)가 겨냥하는 자리가 정확히 여기다.
        absence_cost: AbsenceCost::Fatal,
    },
    // ── ⑥ 신기능 안내(벤더 업그레이드마다 증식) ───────────────────────────
    Def {
        id: "feature-announce-fullscreen",
        title: "신기능 안내 · fullscreen renderer",
        needles: &["Try the new fullscreen renderer?"],
        widget: &["Enter to confirm", "Esc to cancel"],
        confirm_echo: &["Yes, try it", "Not now"],
        passability: Passability::Machine,
        // 실측(Windows 실기): 기본 포커스가 `1. Yes, try it`.
        default_index: Some(1),
        // ★기본 포커스를 **따르지 않는다**: fullscreen renderer 수락은 화면 렌더 계약(대체 화면
        //   진입·마우스 보고)을 바꾸므로, 화면을 읽어 판정하는 이 시스템의 관측 전제를 흔든다.
        //   '종전 동작 유지' 쪽인 `2. Not now` 를 고른다(아래 1회 + Return).
        action: Some((2, "Not now", Some("2"))),
        human_reason: None,
        // 부재의 귀결: 주입 Return 이 기본 포커스 `Yes, try it` 를 눌러 fullscreen renderer 가
        // 켜진다 = 대체 화면 진입 + 마우스 보고. **화면을 읽어 판정한다**는 이 시스템의 관측
        // 전제가 그 순간 무너지고, 이후 모든 축(마커·관문·꼬리)이 같이 무효가 된다(비가역).
        absence_cost: AbsenceCost::Fatal,
    },
];

// ═══════════════════════════════════════════════════════════════════════════
// ★코퍼스 자기규칙 (2026-08-24 · BLOCK-1/BLOCK-2 구조적 재발 차단)
//
// **대원칙**: 관문은 "그 관문이 화면에 떠 있을 때만 나타나는 것"으로만 식별해야 한다.
// 인사 배너·상태 메시지·에러 문자열은 그 조건을 만족하지 않는다 — 정상 화면에도 나타나기
// 때문이다. 관문 오탐의 귀결은 **영구 부트 라이브락**이다: 화면에 통과시킬 관문이 실제로는
// 없으므로 사람도 풀 수 없고, 이 제품의 존재 이유("팀을 결정론적으로 세운다")가 무너진다.
// 그래서 오탐은 미탐보다 비싸고, 아래 세 규칙은 **선언 시점에** 그것을 금지한다.
//
//   ⓐ needle 은 **관문 전용**이어야 한다 — 질문형(`?` 포함)이거나, 아니면
//      [`NEEDLE_EXEMPTIONS`] 에 근거와 함께 등재돼야 한다. 그리고 (근거 문장과 무관하게)
//      어떤 needle 도 [`fixtures::NON_GATE_SCREENS`] 중 어느 화면에도 **단독으로** 걸리면
//      안 된다. 단독 조건인 이유: 감지 경로(`inject_guard::needle_hit`)는 위젯 AND 를 보지
//      않으므로 needle 이 스스로 관문 전용이 아니면 그 경로가 통째로 오탐한다.
//   ⓑ widget 은 비어 있으면 안 된다 — AND 가드가 0이면 needle 이 화면 전문에 그대로 걸린다.
//   ⓒ widget 이 [`UNIVERSAL_WIDGET_TOKENS`](모든 정상 claude 화면에 있는 문자) **단독**이면
//      안 된다. `❯` 를 위젯으로 선언하면 AND 가 무의미해지고 관문은 needle 하나로 성립한다.
// ═══════════════════════════════════════════════════════════════════════════

/// 정상 화면에도 늘 있는 보편 토큰. **단독**으로는 위젯 서명이 될 수 없다(규칙 ⓒ).
///
/// `❯` 는 실측 관문 6화면 전부에 있고 동시에 **모든 정상 claude 프롬프트**에 있다
/// (= `agents.json` 의 `ready_marker` 와 같은 문자). 나머지는 셸 프롬프트 종결자로,
/// 화면 어디에나 나타난다.
pub const UNIVERSAL_WIDGET_TOKENS: &[&str] = &["❯", ">", "$", "%", "#", "·", "…"];

/// 질문형(`?`)이 아니지만 **관문 전용**임을 근거와 함께 선언한 needle 목록(규칙 ⓐ의 예외구).
///
/// `(gate_id, needle, 근거)`. 표에 없는 비질문형 needle 은 검체에서 적색이고, 표에 있으나
/// 정본이 더는 선언하지 않는 항목도 적색이다(면제표가 쓰레기통이 되는 것을 막는다).
/// ★면제는 "정상 화면에 안 나온다"를 **면제해 주지 않는다** — 그 축은 아래
/// `no_needle_alone_matches_a_non_gate_screen` 이 표와 무관하게 전수로 집행한다.
pub const NEEDLE_EXEMPTIONS: &[(&str, &str, &str)] = &[
    (
        "theme",
        "Choose the text style that looks best with your terminal",
        "테마 선택기의 지시형 프롬프트. 이 선택기가 떠 있는 동안에만 렌더되며 온보딩 완료 \
         프로필에서는 다시 나오지 않는다(실측 2026-08-23 · 완료 플래그 시드 후 미출현).",
    ),
    (
        "login-method",
        "Select login method",
        "로그인 방식 선택기의 지시형 프롬프트(실측 화면 문면은 `Select login method:`). \
         선택이 끝나면 브라우저 안내 화면으로 넘어가며 이 줄은 사라진다.",
    ),
    (
        "oauth-code",
        "Browser didn't open? Use the url below to sign in",
        "OAuth 브라우저 대기 화면의 **지시형 안내 줄**(실측 문면 뒤에 `(c to copy)` 가 붙는다). \
         물음표가 문장 **중간**에 있어 질문형 판정(`is_question_form`)에 걸리지 않으므로 근거를 \
         명시한다 — 종전 느슨한 규칙(물음표 포함 여부만 봄)에서는 이 needle 이 심사 없이 \
         통과했다(P4-9). 이 줄은 브라우저 로그인 대기 화면이 떠 있는 동안에만 렌더되며, 코드 \
         입력이 끝나면 사라진다.",
    ),
    (
        "oauth-code",
        "Paste code here if prompted",
        "OAuth 코드 **입력 프롬프트 줄 그 자체**다(실측 `Paste code here if prompted >`). \
         상태 보고가 아니라 입력을 기다리는 위젯이므로 떠 있는 동안에만 존재한다.",
    ),
    (
        "folder-trust",
        "Is this a project you created or one you trust",
        "2.1.241 실측 질문문. 화면에서는 `?` 로 끝나지만 TUI 폭에 따라 물음표가 다음 줄로 \
         접히는 것을 봤으므로 needle 에서는 뺐다(접힘 내성).",
    ),
    (
        "folder-trust",
        "Quick safety check",
        "폴더 신뢰 대화상자의 제목 줄(실측 `Quick safety check: Is this a project …`). \
         이 대화상자 밖에서는 렌더되지 않는다.",
    ),
    (
        "folder-trust",
        "Do you trust the files in this folder",
        "`agents.json` 의 `trust-prompt` 선언 문면(구 버전 질문문) — 하위호환 유지. 질문문이나 \
         선언 문면과 글자 단위로 같아야 해서 `?` 를 붙이지 않는다.",
    ),
    (
        "folder-trust",
        "Do you trust this folder",
        "더 짧은 구 문면 하위호환. 질문문이지만 `?` 를 붙이면 접힘·구두점 변형에 걸려 감지가 \
         죽으므로 본문만 선언한다(확인 에코 재매칭은 `confirm_echo` 불변식이 따로 막는다).",
    ),
    (
        "bypass-disclaimer",
        "WARNING: Claude Code running in Bypass Permissions mode",
        "면책 대화상자의 경고 헤더. 이 대화상자가 떠 있는 동안에만 렌더되며, 수락 뒤에는 \
         화면에 남지 않는다(실측: 수락 결과가 userSettings 에 기록되고 대화상자는 사라진다).",
    ),
    (
        "bypass-disclaimer",
        "In Bypass Permissions mode, Claude Code will not ask for your approval",
        "같은 면책 대화상자의 본문 설명 줄. 헤더가 화면 폭에 접혀 잘릴 때를 위한 두 번째 축이며, \
         대화상자 밖에서는 렌더되지 않는다(수락 후 화면에 남지 않음 — 실측).",
    ),
];

/// ★관문 ↔ 오탐 대조군 **커버리지 표** (2026-08-24 · 적대 리뷰어 권고 채택).
///
/// `(gate_id, non_gate_screen_id)`. **새 관문을 들이는 사람은 이 표에 줄을 더해야 하고**, 그
/// 줄이 가리키는 대조군 화면은 그 관문의 **위젯 서명을 전부 만족**해야 한다
/// (집행: `every_gate_has_a_control_screen_that_satisfies_its_widget_signature`).
///
/// ★왜 필요한가: 종전에 `NON_GATE_SCREENS` 는 고정 6항목이었고, 새 관문을 들일 때 생기는
///   유일한 마찰은 헬스 러너의 `need(len(blocks) == 6)` 하나였다 — 손으로 `6 → 7` 만 고치면
///   **대조군 0으로 통과**한다. 그 상태에서는 관문이 늘수록 오탐 표면만 넓어지고 그것을 재는
///   자리는 하나도 늘지 않는다(리뷰어 표현: "지금 이빨은 자라지 않는다").
///
/// ★왜 '존재'가 아니라 '위젯 서명 만족'을 요구하는가: 아무 화면이나 갖다 붙이면 위젯 AND 가
///   애초에 불만족이라 관문이 안 잡히는 것이 당연해지고, 그 커버리지는 아무것도 재지 못한다.
///   서명을 만족시켜 두면 "그 화면에서 관문이 안 잡히는 이유가 **오직 needle 축**" 이라는 사실이
///   실행으로 증명된다.
pub const GATE_CONTROL_COVERAGE: &[(&str, &str)] = &[
    ("theme", "audit-log-line"),
    ("theme", "config-theme-setting"),
    ("login-method", "account-status-panel"),
    ("oauth-code", "doc-mentioning-oauth-url"),
    ("folder-trust", "live-permission-prompt"),
    ("folder-trust", "audit-log-line"),
    ("bypass-disclaimer", "live-permission-prompt"),
    ("bypass-disclaimer", "audit-log-line"),
    ("feature-announce-fullscreen", "live-permission-prompt"),
    ("feature-announce-fullscreen", "audit-log-line"),
];

/// 이 문자열이 보편 토큰 단독인가(규칙 ⓒ의 판정 핵).
pub fn is_universal_widget_token(w: &str) -> bool {
    let f = flatten(w);
    f.is_empty() || UNIVERSAL_WIDGET_TOKENS.iter().any(|u| flatten(u) == f)
}

/// 관문 하나가 자기규칙 ⓑⓒ를 만족하는가 — 위반 사유를 돌려준다(만족하면 빈 벡터).
///
/// ★순수함수로 두는 이유: 검체가 **구 선언을 그대로 지어 넣어** 적색을 재현할 수 있어야
///   한다(계측 타당성). 규칙이 테스트 본문에만 있으면 그 재현이 불가능하다.
pub fn widget_rule_violations(g: &Gate) -> Vec<String> {
    let mut out = Vec::new();
    if g.widget.is_empty() {
        out.push(format!(
            "{}: widget AND 가드가 0 — needle 이 화면 전문에 그대로 걸린다(BLOCK-2 형태)",
            g.id
        ));
    } else if !g.widget.iter().any(|w| !is_universal_widget_token(w)) {
        out.push(format!(
            "{}: widget 이 보편 토큰 단독({:?}) — 모든 정상 화면에 있는 문자를 위젯으로 쓰면 \
             AND 가 무의미해지고 관문이 needle 하나로 성립한다(BLOCK-1 형태)",
            g.id, g.widget
        ));
    }
    out
}

/// ★질문형 판정(규칙 ⓐ) — 물음표가 **문장을 끝내는** 구두점일 때만 참이다.
///
/// 【무엇이 틀렸었는가 — P4-9 · 2026-08-24 적대 리뷰어】 종전 판정은 검체 본문의
/// `if n.contains('?') { continue; }` 한 줄이었고 **위치를 보지 않았다**. 그래서 문장 중간에
/// `?` 가 하나만 있으면 면제 심사를 통째로 건너뛴다. 실증 반례:
/// `"Do you want to proceed?"` 는 **살아 있는 claude 세션의 권한 프롬프트 문면**인데 근거 없이
/// 통과했고, 당시 대조군 6종에 그 화면이 없어 대조군도 통과했다.
///
/// 수리는 두 겹이다 —
///   ⓐ 여기: 물음표가 **끝** 구두점일 것(중간 `?` 는 면제표에 근거를 적어야 한다).
///   ⓑ [`gate_rule_violations`]: **질문형이어도** 관문이 아닌 화면 전량 대조를 여전히 요구한다.
///     ⓐ 만으로는 "그 관문에서만 나온다" 가 조금도 보장되지 않기 때문이다.
pub fn is_question_form(needle: &str) -> bool {
    needle
        .trim_end()
        .strip_suffix('?')
        .is_some_and(|head| head.chars().any(|c| !c.is_whitespace()))
}

/// ★관문 하나가 자기규칙을 만족하는가 — **프로덕션 집행용** 전량 판정(위반 사유를 돌려준다).
///
/// [`widget_rule_violations`](ⓑⓒ) 에 규칙 ⓐ의 **이빨**(어떤 needle 도 관문이 아닌 화면에
/// 단독으로 걸리지 않는다)을 더한 것이다. 질문형 여부·면제표는 여기서 보지 **않는다** —
/// 면제표는 코드 정본을 쓰는 사람이 근거를 적는 자리이고(검체 ⓐ가 집행), override 봉투로
/// 들어온 선언에는 적을 자리가 없기 때문이다. 반면 "정상 화면에 걸리는가" 는 선언의 출처와
/// 무관하게 **화면으로 잴 수 있는 사실**이라 어디서 온 선언이든 똑같이 집행할 수 있다.
///
/// ★단독으로 보는 이유: 감지 경로(`inject_guard::needle_hit`)는 위젯 AND 를 보지 않으므로,
///   needle 이 스스로 관문 전용이 아니면 그 경로가 통째로 오탐한다.
pub fn gate_rule_violations(g: &Gate) -> Vec<String> {
    let mut out = widget_rule_violations(g);
    for n in &g.needles {
        for sid in needle_non_gate_hits(n) {
            out.push(format!(
                "{}: needle {n:?} 이 **관문이 아닌** 화면 {sid} 에 단독으로 걸린다 — 정상 \
                 화면에도 나타나는 문면은 관문의 근거가 될 수 없다(오탐의 귀결은 영구 부트 \
                 라이브락)",
                g.id
            ));
        }
    }
    out
}

/// 이 needle 이 **관문이 아닌 화면**에 단독으로 걸리는가 — 걸린 대조군 화면 id 를 돌려준다.
///
/// 규칙 ⓐ의 판정 핵이자 **수리(repair)의 판정 핵**이다: 위반을 아는 것만으로는 무엇을 고쳐야
/// 하는지 알 수 없으므로, "어느 needle 이 문제인가" 를 사유 문자열이 아니라 값으로 돌려준다
/// (사유 문자열을 되파싱해 고치는 코드는 다음 판에서 반드시 갈린다).
pub fn needle_non_gate_hits(needle: &str) -> Vec<&'static str> {
    let (nn, nf) = (normalize(needle), flatten(needle));
    if nf.is_empty() {
        return Vec::new();
    }
    fixtures::NON_GATE_SCREENS
        .iter()
        .filter(|(_, screen)| normalize(screen).contains(&nn) || flatten(screen).contains(&nf))
        .map(|&(sid, _)| sid)
        .collect()
}

/// 코드 임베드 정본 코퍼스.
pub fn builtin() -> Vec<Gate> {
    DEFS.iter()
        .map(|d| Gate {
            id: d.id.to_string(),
            title: d.title.to_string(),
            needles: d.needles.iter().map(|s| s.to_string()).collect(),
            widget: d.widget.iter().map(|s| s.to_string()).collect(),
            confirm_echo: d.confirm_echo.iter().map(|s| s.to_string()).collect(),
            passability: d.passability,
            default_index: d.default_index,
            action: d.action.map(|(i, l, lit)| GateAction {
                select_index: i,
                label: l.to_string(),
                literal: lit.map(|s| s.to_string()),
            }),
            human_reason: d.human_reason.map(|s| s.to_string()),
            absence_cost: d.absence_cost,
            measured_on: MEASURED_ON.to_string(),
            origin: Origin::Builtin,
        })
        .collect()
}

/// 화면에서 관문 하나를 식별한다. 코퍼스 **선언 순서**(= 실측 등장 순서)로 첫 매칭을 돌려준다.
pub fn identify<'a>(gates: &'a [Gate], screen: &str) -> Option<&'a Gate> {
    gates.iter().find(|g| g.matches(screen))
}

// ═══════════════════════════════════════════════════════════════════════════
// 버전 핀
// ═══════════════════════════════════════════════════════════════════════════

/// 관문 액션을 지금 집행해도 되는가.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionPolicy {
    /// 집행 가능 — 아래키 `down` 회 + Return(또는 `literal`).
    Allowed {
        down: u8,
        literal: Option<String>,
        label: String,
    },
    /// 사람이 1회 해야 한다.
    HumanRequired { reason: String },
    /// 액션 선언이 없거나 키 시퀀스를 산출할 수 없다 — 관측만.
    HeldNoAction,
    /// 실측 버전과 지금 도는 바이너리가 다르다 — 관측만.
    HeldVersionDrift {
        measured_on: String,
        detected: String,
    },
    /// 버전을 재지 못했다 — 관측만(**측정 불능은 통과가 아니다**).
    HeldVersionUnknown { measured_on: String },
}

impl ActionPolicy {
    pub fn is_allowed(&self) -> bool {
        matches!(self, ActionPolicy::Allowed { .. })
    }
}

/// ★버전 핀 게이트. 보류의 귀결은 언제나 '아무 키도 보내지 않음' 이므로 이 게이트는
/// **오살 방향으로 열리지 않는다** — 잘못 보류하면 사람이 한 번 눌러 주면 되고,
/// 잘못 집행하면 좌석이 죽는다(면책 창 rc 1). 비대칭이 이 fail-closed 를 정당화한다.
pub fn action_policy(gate: &Gate, detected_version: Option<&str>) -> ActionPolicy {
    if gate.passability == Passability::HumanOnly {
        return ActionPolicy::HumanRequired {
            reason: gate
                .human_reason
                .clone()
                .unwrap_or_else(|| "사람 1회 필요(사유 미선언)".to_string()),
        };
    }
    let Some(action) = gate.action.as_ref() else {
        return ActionPolicy::HeldNoAction;
    };
    let Some(down) = gate.down_presses() else {
        return ActionPolicy::HeldNoAction;
    };
    match detected_version {
        None => ActionPolicy::HeldVersionUnknown {
            measured_on: gate.measured_on.clone(),
        },
        Some(v) if v != gate.measured_on => ActionPolicy::HeldVersionDrift {
            measured_on: gate.measured_on.clone(),
            detected: v.to_string(),
        },
        Some(_) => ActionPolicy::Allowed {
            down,
            literal: action.literal.clone(),
            label: action.label.clone(),
        },
    }
}

/// 배너·`--version` 출력에서 claude 버전을 뽑는다(순수 · 서브프로세스 없음).
///
/// 실측 문면: `Welcome to Claude Code v2.1.241` / `Claude Code v2.1.241` /
/// `claude --version` = `2.1.241 (Claude Code)`. 어느 것도 못 찾으면 `None` 이고,
/// `None` 은 `action_policy` 에서 **보류**로 접힌다(추정 금지).
pub fn parse_cli_version(text: &str) -> Option<String> {
    const ANCHOR: &str = "Claude Code v";
    if let Some(i) = text.find(ANCHOR) {
        if let Some(v) = take_dotted(&text[i + ANCHOR.len()..]) {
            return Some(v);
        }
    }
    take_dotted(text.trim_start())
}

fn take_dotted(s: &str) -> Option<String> {
    let head: String = s
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let trimmed = head.trim_end_matches('.');
    if trimmed.split('.').filter(|p| !p.is_empty()).count() >= 2
        && trimmed.starts_with(|c: char| c.is_ascii_digit())
    {
        Some(trimmed.to_string())
    } else {
        None
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// override 봉투 (`agents.json` → <어댑터> → first_run_gates)
// ═══════════════════════════════════════════════════════════════════════════

/// 최종 코퍼스가 어디서 왔는가.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// 코드 정본만.
    Builtin,
    /// 코드 정본 + 봉투 병합.
    Merged { overridden: usize, added: usize },
    /// 봉투가 코퍼스를 통째로 교체.
    Replaced { count: usize },
    /// 롤백 스위치로 override 파싱이 꺼져 있다.
    OverrideDisabled,
}

/// 코퍼스 해소 결과. `notes` 는 사람용 진단이며 **판정 재료가 아니다**(호출부가 원할 때 표출).
#[derive(Debug, Clone)]
pub struct Resolved {
    pub gates: Vec<Gate>,
    pub notes: Vec<String>,
    pub source: Source,
}

/// ★env 를 읽는 **유일한 지점**(롤백 스위치 1지점 규약).
pub fn override_enabled() -> bool {
    override_enabled_from(std::env::var(OVERRIDE_ENV).ok().as_deref())
}

/// 위 판정의 순수 절반(테스트가 env 를 건드리지 않게 분리).
pub fn override_enabled_from(raw: Option<&str>) -> bool {
    !matches!(
        raw.map(|s| s.trim().to_ascii_lowercase()).as_deref(),
        Some("0") | Some("off") | Some("false") | Some("no")
    )
}

/// 어댑터 스펙(`load_agent_spec` 산출물)에서 코퍼스를 해소한다.
pub fn resolve_from_spec(spec: &Value) -> Resolved {
    resolve_with(spec.get(ADAPTER_KEY), override_enabled())
}

/// 위의 순수 본체. `envelope` = `first_run_gates` 값.
///
/// 봉투 형식:
/// ```json
/// { "source": "builtin" | "replace",
///   "measured_on": "2.1.241",
///   "gates": [ { "id": "...", "needles": [...], ... } ] }
/// ```
/// 규칙 — ① 손상된 선언은 **그 항목만** 버리고 코드 정본을 유지한다(부트를 멈추지 않는다).
/// ② `HumanOnly` 로 실측된 관문은 override 로 **기계 통과로 승격되지 않는다**(로그인은 우회
/// 불가라는 것이 측정 결과이지 정책이 아니다). 반대 방향(Machine→HumanOnly)은 조이는 쪽이라 허용.
/// ③ `replace` 인데 **파싱 가능한 선언이 0건**이면 코드 정본으로 되돌린다 — 빈 코퍼스는
/// '관문 없음'이 아니라 '눈을 감음'이고, 뒤 단위가 '관문 0매칭'을 ready 의 AND 항으로 쓰는
/// 순간 허위 ready 가 된다. ★이 폴백은 **`resolve_raw` 안에서만** 산다: 사용자가 유효하게
/// 선언한 관문이 하나라도 있으면 그것이 코퍼스이며, 자기규칙 집행이 그 코퍼스를 비워 정본으로
/// 되돌리는 일은 없다(P4-10 결함 ① — 그 폴백이 사용자 주권 침해의 직접 원인이었다).
/// ④ ★해소 **직전**에 자기규칙을 집행한다(아래 [`enforce_self_rules`]) — 규칙을 아는 것과
/// 규칙이 집행되는 것은 다른 사실이다(P4-3). 단 집행 수단은 **수리이지 제거가 아니다**(P4-10).
pub fn resolve_with(envelope: Option<&Value>, override_on: bool) -> Resolved {
    let Resolved {
        gates,
        mut notes,
        source,
    } = resolve_raw(envelope, override_on);
    // ★해소 **직후**에 Fatal 바닥을 세운다(아래 [`restore_fatal_builtin_floor`]). 아래
    //   [`enforce_absence_cost`] 는 '집행 전후 대조' 라 replace 모드에서는 눈이 멀어 있다 —
    //   그 모드의 `pre` 는 사용자 목록이라 빌트인 Fatal 관문이 **순회 대상에 애초에 없다**.
    let gates = restore_fatal_builtin_floor(gates, &mut notes);
    // ★집행 전 코퍼스를 남긴다 — 아래 [`enforce_absence_cost`] 가 "집행이 부재의 비용을
    //   넘지 않았는가" 를 **대조로** 판정하려면 전후 두 벌이 있어야 한다.
    let pre = gates.clone();
    let mut kept = enforce_self_rules(gates, &mut notes);
    enforce_absence_cost(&pre, &mut kept, &mut notes);
    // 출처 회계는 그대로다: 집행은 관문을 제거하지 않으므로 개수·출처가 변하지 않는다
    // (변했다면 위 게이트가 되살리고 사유를 남긴다 — 조용한 변형은 없다).
    Resolved {
        gates: kept,
        notes,
        source,
    }
}

/// ★Fatal 바닥 — [`AbsenceCost::Fatal`] 인 빌트인 관문은 **어떤 해소 모드에서도** 코퍼스에서
/// 사라지지 않는다(N1 · 2026-08-24).
///
/// 【무엇이 틀렸었는가】 [`enforce_absence_cost`] 는 "집행이 관문을 없앴는가"를 **집행 전후
/// 대조**로 본다. 그래서 `source=replace` 에는 눈이 멀어 있었다 — 그 모드의 해소 산출은
/// 사용자 목록이고, 빌트인 Fatal 관문은 대조의 `pre` 에 **애초에 없어서** 되살리는 코드가
/// 한 줄도 실행되지 않았다. `{"first_run_gates":{"source":"replace","gates":[…1건…]}}`
/// 한 줄이면 킬체인 관문 5종이 통째로 사라지고, 그중 면책 창의 부재는 **주입 Return 한 발이
/// rc 1**(좌석 사망)이다. 발동에 사용자 명시 선언이 필요하다는 사실은 **비용을 낮추지 않는다**.
///
/// 【왜 '바닥'인가 — 사용자 주권과 충돌하지 않는다】 이 게이트는 사용자 선언을 **지우거나
/// 덮지 않는다**. 선언은 그대로 살고, 선언에 없는 Fatal 빌트인만 뒤에 덧붙는다(추가만).
/// 관문이 하나 더 있어서 생기는 최악은 '그 화면에서 한 번 더 보류' 이고, 없어서 생기는
/// 최악은 '좌석이 rc 1 로 죽는다' 다 — 비대칭이 방향을 정한다.
///
/// 【id 를 가로챈 선언】 사용자가 Fatal 빌트인과 **같은 id** 를 선언했으면 그 선언이 코퍼스에
/// 남되, 부재의 비용만 실측값(`Fatal`)으로 되돌린다. 부재의 귀결은 선언이 아니라 측정이며,
/// merge 경로의 [`apply_patch`] 가 이미 같은 완화를 거부한다(두 경로의 대칭).
fn restore_fatal_builtin_floor(mut gates: Vec<Gate>, notes: &mut Vec<String>) -> Vec<Gate> {
    for b in builtin().into_iter().filter(|b| b.absence_is_fatal()) {
        match gates.iter_mut().find(|g| g.id == b.id) {
            Some(g) => {
                if g.absence_cost != AbsenceCost::Fatal {
                    notes.push(format!(
                        "{}: 선언이 부재의 비용을 낮췄다 — 실측값(fatal)으로 되돌린다(부재의 \
                         귀결은 선언이 아니라 측정이다)",
                        g.id
                    ));
                    g.absence_cost = AbsenceCost::Fatal;
                }
            }
            None => {
                notes.push(format!(
                    "{}: ★해소본에 **부재의 비용이 비가역인** 빌트인 관문이 없다 — 코드 정본을 \
                     강제 **복원**했다(선언은 그대로 두고 덧붙이기만 한다). 이 관문의 부재는 \
                     좌석 rc 1 종료·허위 READY 영구화·관측 전제 파괴 중 하나로 귀결한다",
                    b.id
                ));
                gates.push(b);
            }
        }
    }
    gates
}

/// ★자기규칙 집행 — 위반 관문은 **버리고** `notes` 에 사유를 남긴다(P4-3 · 2026-08-24).
///
/// 【무엇이 틀렸었는가 — 적대 리뷰어 격리 실행】 [`widget_rule_violations`] 는 정의만 되어 있고
/// **프로덕션 호출이 0**이었다(`git grep` 결과가 정의 1 + `#[cfg(test)]` 3). 그래서 자기규칙은
/// 검체 안에서만 살아 있었고, 검체가 도는 코퍼스는 전부 `builtin()` 이었다 — **override 봉투로
/// 해소된 코퍼스는 규칙 밖**이었다. 그 틈으로 BLOCK-1 이 봉투 한 줄로 그대로 복원된다:
///
/// ```json
/// {"first_run_gates":{"gates":[{"id":"theme","needles":["Welcome to Claude Code"],"widget":[]}]}}
/// ```
///
/// `"widget": []` 는 `Some(vec![])` 이라 [`apply_patch`] 가 빌트인 관문의 **AND 가드를 비운다**.
/// 그러면 배너 needle 하나로 관문이 성립하고, 건강한 노드 전원이 `gate_pending` 으로 접혀
/// **영구 부트 라이브락**이 된다. 규칙은 그 위반을 알고 있었지만 **아무도 묻지 않았다.**
///
/// 【왜 버리지 **않는가** — P4-10 · 2026-08-24 이종 리뷰어 2인 일치】 첫 판의 집행 수단은
/// '버리기' 였고, 그것이 두 가지를 부쉈다 —
///
///   ① **사용자 주권**: `source=replace` 로 선언한 사용자 코퍼스가 통째로 버려지고, 뒤이은
///      "비면 정본으로 되돌린다" 폴백이 벤더 6종을 다시 세웠다(디스크 선언 > 임베드의 반대).
///   ② **실패 방향의 역전(재난 ④)**: 봉투가 `bypass-disclaimer` 의 위젯을 비웠을 때 종전
///      귀결은 `needle 하나로 관문 성립 → 보류`(안전측 오탐)였는데, 버린 뒤의 귀결은
///      `관문 없음 → 주입` 이다. 그 창의 기본 포커스는 `No, exit` 이고 Return 은 rc 1 이므로
///      **집행이 안전한 오탐을 좌석 사망으로 바꿨다.**
///
/// 그래서 집행 수단을 [`repair_gate`](수리)로 바꾼다. 관문은 **어떤 경우에도 코퍼스에서
/// 제거되지 않으며**, 위반한 축만 고쳐진다. 제거가 없으므로 `보류 → 주입` 으로 뒤집힐 자리도
/// 구조적으로 없다. 조용히 고치지 않는 것이 계약의 나머지 절반이라 사유는 반드시 `notes` 에 남는다.
fn enforce_self_rules(gates: Vec<Gate>, notes: &mut Vec<String>) -> Vec<Gate> {
    let canon = builtin();
    gates
        .into_iter()
        .map(|g| repair_gate(g, &canon, notes))
        .collect()
}

/// ★자기규칙 위반을 **버리지 않고 고친다**(P4-10). 반환값은 언제나 관문 하나다.
///
/// | 위반 축 | 빌트인 대응물이 있다 | 사용자 신설 관문(대응물 없음) |
/// |---|---|---|
/// | needle 이 정상 화면에 걸린다 | 정본 needle 로 **복원** | 걸리는 needle 만 **제거**(나머지는 유지) |
/// | 위젯 AND 가드 0 · 보편 토큰 단독 | 정본 위젯 서명으로 **복원** | **유지** + 사유(아래 근거) |
///
/// ★왜 사용자 신설 관문의 위젯 위반은 유지하는가: 규칙 ⓑⓒ는 needle 품질을 위한 **심층 방어**
///   이고, 실제로 오탐을 재는 축은 "정상 화면에 걸리는가"(규칙 ⓐ의 이빨) 하나다. 그 축을 이미
///   통과한 사용자 needle 이라면 AND 가드의 부재는 **약한 가드이지 위험한 가드가 아니다**.
///   반면 관문을 버리는 것은 사용자를 조용히 무시하면서 귀결을 주입 방향으로 뒤집는다.
///
/// ★왜 needle 제거는 안전한가: 제거되는 것은 **관문이 아닌 대조군 화면에 걸리는** needle 뿐이고,
///   그 화면에서 뒤집히는 귀결은 `보류 → 주입` 이지만 **그 화면에서는 주입이 애초에 옳다**
///   (정상 화면이다). 실측 관문 화면 위에서의 귀결은 한 톨도 바뀌지 않는다(검체가 전수 대조).
fn repair_gate(mut g: Gate, canon: &[Gate], notes: &mut Vec<String>) -> Gate {
    if gate_rule_violations(&g).is_empty() {
        return g;
    }
    let builtin_of = canon.iter().find(|b| b.id == g.id);

    // ── ⓐ needle 축 — 정상 화면에 걸리는 문면만 손댄다.
    let offending: Vec<String> = g
        .needles
        .iter()
        .filter(|n| !needle_non_gate_hits(n).is_empty())
        .cloned()
        .collect();
    if !offending.is_empty() {
        match builtin_of {
            Some(b) => {
                notes.push(format!(
                    "{}: needle {offending:?} 이 정상 화면에 걸린다 — 버리지 않고 코드 정본의 \
                     needle 로 **복원**했다(관문은 남고 오탐 경로만 닫힌다)",
                    g.id
                ));
                g.needles = b.needles.clone();
            }
            None => {
                g.needles.retain(|n| needle_non_gate_hits(n).is_empty());
                notes.push(format!(
                    "{}: 사용자 신설 관문의 needle {offending:?} 이 정상 화면에 걸려 그 항목만 \
                     **제거**했다(관문 선언 자체는 유지 — 남은 needle {}건)",
                    g.id,
                    g.needles.len()
                ));
            }
        }
    }

    // ── ⓑⓒ 위젯 축 — AND 가드가 0이거나 보편 토큰 단독이다.
    if !widget_rule_violations(&g).is_empty() {
        match builtin_of.filter(|b| widget_rule_violations(b).is_empty()) {
            Some(b) => {
                notes.push(format!(
                    "{}: 위젯 AND 가드 위반({:?}) — 버리지 않고 코드 정본의 위젯 서명 {:?} 으로 \
                     **복원**했다(BLOCK-1 봉투 공격이 정확히 이 경로다: 관문을 잃지 않은 채 \
                     AND 구멍이 닫힌다)",
                    g.id, g.widget, b.widget
                ));
                g.widget = b.widget.clone();
            }
            None => {
                notes.push(format!(
                    "{}: 사용자 신설 관문의 위젯 AND 가드가 없다 — 복원할 정본 서명이 없으므로 \
                     선언을 **그대로 유지**한다(버리면 사용자를 조용히 무시하면서 귀결을 \
                     보류 → 주입 으로 뒤집는다). needle 축은 정상 화면 대조를 이미 통과했다",
                    g.id
                ));
            }
        }
    }
    g
}

/// ★부재의 비용 게이트 — 집행이 [`AbsenceCost::Fatal`] 관문을 사라지게 했는지 **대조로** 본다.
///
/// [`repair_gate`] 는 관문을 제거하지 않으므로 평시에 이 게이트는 아무것도 하지 않는다.
/// 그래도 두는 이유: "제거하지 않는다" 는 지금 코드의 **성질**일 뿐이고, 다음 판의 집행기가
/// 그 성질을 잃어도 컴파일러는 아무 말도 하지 않는다. 킬체인 관문의 부재는 rc 1 좌석 사망이라
/// 그 회귀를 사람의 주의력에 맡길 수 없다 — 그래서 계약을 **실행되는 대조**로 남긴다.
///
/// 되살릴 때는 **코드 정본**을 우선한다(정본은 규칙을 만족한다). 정본에 없는 사용자 신설
/// 관문이면 집행 전 선언 그대로 되살린다 — 부재보다 비싼 것은 없다는 것이 이 게이트의 전제다.
fn enforce_absence_cost(pre: &[Gate], kept: &mut Vec<Gate>, notes: &mut Vec<String>) {
    let canon = builtin();
    for g in pre {
        if kept.iter().any(|k| k.id == g.id) {
            continue;
        }
        if !g.absence_is_fatal() {
            notes.push(format!(
                "{}: 집행이 관문을 제거했다(부재의 비용 = 가역) — 그 화면은 뒤 단위의 다른 축이 \
                 한 번 더 본다",
                g.id
            ));
            continue;
        }
        let restored = canon.iter().find(|b| b.id == g.id).unwrap_or(g).clone();
        notes.push(format!(
            "{}: ★집행이 **부재의 비용이 비가역인** 관문을 제거했다 — 되살린다. 이 관문의 \
             부재는 좌석 종료·허위 READY 영구화·관측 전제 파괴 중 하나로 귀결하며, 자기규칙 \
             위반보다 비싸다(모듈 doc 비용표)",
            g.id
        ));
        kept.push(restored);
    }
}

/// 위 [`resolve_with`] 의 해소 본체(자기규칙 집행 **전**의 코퍼스를 만든다).
fn resolve_raw(envelope: Option<&Value>, override_on: bool) -> Resolved {
    let base = builtin();
    if !override_on {
        return Resolved {
            gates: base,
            notes: vec![format!(
                "{OVERRIDE_ENV}=0 — agents.json override 파싱 비활성(코드 정본만 사용)"
            )],
            source: Source::OverrideDisabled,
        };
    }
    let mut notes: Vec<String> = Vec::new();
    let Some(env_v) = envelope else {
        return Resolved {
            gates: base,
            notes,
            source: Source::Builtin,
        };
    };
    if env_v.is_null() {
        // 명시 null = "봉투를 의도적으로 비움" → 코드 정본만(사용자 주권 · 코퍼스는 남는다).
        return Resolved {
            gates: base,
            notes,
            source: Source::Builtin,
        };
    }
    let Some(obj) = env_v.as_object() else {
        notes.push(format!(
            "{ADAPTER_KEY} 가 객체가 아니다 — 봉투를 무시하고 코드 정본만 사용한다"
        ));
        return Resolved {
            gates: base,
            notes,
            source: Source::Builtin,
        };
    };
    let mode = obj.get("source").and_then(|v| v.as_str()).unwrap_or("builtin");
    let default_measured = obj
        .get("measured_on")
        .and_then(|v| v.as_str())
        .unwrap_or(MEASURED_ON)
        .to_string();
    let decls: Vec<&Value> = obj
        .get("gates")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().collect())
        .unwrap_or_default();

    if mode == "replace" {
        let mut out: Vec<Gate> = Vec::new();
        for d in &decls {
            match parse_new_gate(d, &default_measured) {
                Ok(g) => out.push(g),
                Err(e) => notes.push(format!("replace 선언 무시: {e}")),
            }
        }
        if out.is_empty() {
            notes.push(
                "source=replace 인데 유효 선언이 0건 — 빈 코퍼스는 관측이 아니라 맹목이므로 \
                 코드 정본으로 되돌린다"
                    .to_string(),
            );
            return Resolved {
                gates: base,
                notes,
                source: Source::Builtin,
            };
        }
        let count = out.len();
        return Resolved {
            gates: out,
            notes,
            source: Source::Replaced { count },
        };
    }
    if mode != "builtin" {
        notes.push(format!(
            "{ADAPTER_KEY}.source={mode:?} 는 미지 값 — builtin 병합으로 취급한다"
        ));
    }

    let mut gates = base;
    let (mut overridden, mut added) = (0usize, 0usize);
    for d in &decls {
        let Some(dm) = d.as_object() else {
            notes.push("gates[] 항목이 객체가 아니다 — 무시".to_string());
            continue;
        };
        let Some(id) = dm.get("id").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) else {
            notes.push("gates[] 항목에 id 가 없다 — 무시".to_string());
            continue;
        };
        match gates.iter_mut().find(|g| g.id == id) {
            Some(g) => {
                notes.extend(apply_patch(g, dm, &default_measured));
                g.origin = Origin::Overridden;
                overridden += 1;
            }
            None => match parse_new_gate(d, &default_measured) {
                Ok(g) => {
                    gates.push(g);
                    added += 1;
                }
                Err(e) => notes.push(format!("신규 관문 선언 무시: {e}")),
            },
        }
    }
    Resolved {
        gates,
        notes,
        source: if overridden == 0 && added == 0 {
            Source::Builtin
        } else {
            Source::Merged { overridden, added }
        },
    }
}

fn str_vec(v: Option<&Value>) -> Option<Vec<String>> {
    let arr = v?.as_array()?;
    Some(
        arr.iter()
            .filter_map(|x| x.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect(),
    )
}

fn parse_action(v: Option<&Value>) -> Option<GateAction> {
    let o = v?.as_object()?;
    let idx = o.get("select_index").and_then(|x| x.as_u64())?;
    if idx == 0 || idx > u8::MAX as u64 {
        return None;
    }
    Some(GateAction {
        select_index: idx as u8,
        label: o
            .get("label")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        literal: o
            .get("literal")
            .and_then(|x| x.as_str())
            .map(|s| s.to_string()),
    })
}

fn parse_passability(v: Option<&Value>) -> Option<Passability> {
    match v?.as_str()? {
        "machine" => Some(Passability::Machine),
        "human_only" => Some(Passability::HumanOnly),
        _ => None,
    }
}

fn parse_absence_cost(v: Option<&Value>) -> Option<AbsenceCost> {
    match v?.as_str()? {
        "fatal" => Some(AbsenceCost::Fatal),
        "recoverable" => Some(AbsenceCost::Recoverable),
        _ => None,
    }
}

/// 코드 정본에 없는 id 의 신규 관문 선언 → `Gate`. 최소 요건은 id + needle ≥ 1 이다
/// (needle 없는 관문은 아무것도 식별하지 못하므로 받아도 무의미하고, 무의미한 선언을
///  조용히 받아 두면 "선언했는데 왜 안 잡히나"의 진단 비용만 남는다).
///
/// ★`absence_cost` 기본값은 [`AbsenceCost::Recoverable`] 이다 — 사용자 신설 관문은 **실측된
///   킬체인이 아니므로** 그 부재가 비가역이라고 주장할 근거가 없고, 근거 없는 `Fatal` 은
///   [`enforce_absence_cost`] 가 규칙 위반 선언을 되살리는 구멍이 된다(BLOCK-1 의 형태).
///   자기 관문이 킬체인임을 아는 사용자는 `"absence_cost": "fatal"` 로 명시 선언한다.
fn parse_new_gate(v: &Value, default_measured: &str) -> Result<Gate, String> {
    let o = v.as_object().ok_or("항목이 객체가 아니다")?;
    let id = o
        .get("id")
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())
        .ok_or("id 결손")?
        .to_string();
    let needles = str_vec(o.get("needles")).unwrap_or_default();
    if needles.is_empty() {
        return Err(format!("{id}: needles 가 비었다(식별 불가 선언)"));
    }
    let action = parse_action(o.get("action"));
    let passability = parse_passability(o.get("passability")).unwrap_or(Passability::Machine);
    Ok(Gate {
        id,
        title: o
            .get("title")
            .and_then(|x| x.as_str())
            .unwrap_or("(사용자 선언 관문)")
            .to_string(),
        needles,
        widget: str_vec(o.get("widget")).unwrap_or_default(),
        confirm_echo: str_vec(o.get("confirm_echo")).unwrap_or_default(),
        passability,
        default_index: o
            .get("default_index")
            .and_then(|x| x.as_u64())
            .filter(|n| *n > 0 && *n <= u8::MAX as u64)
            .map(|n| n as u8),
        action: if passability == Passability::HumanOnly {
            None
        } else {
            action
        },
        human_reason: o
            .get("human_reason")
            .and_then(|x| x.as_str())
            .map(|s| s.to_string()),
        absence_cost: parse_absence_cost(o.get("absence_cost")).unwrap_or(AbsenceCost::Recoverable),
        measured_on: o
            .get("measured_on")
            .and_then(|x| x.as_str())
            .unwrap_or(default_measured)
            .to_string(),
        origin: Origin::Added,
    })
}

/// 코드 정본 관문 하나에 봉투 선언을 덮는다. 반환값은 사람용 진단 메모.
fn apply_patch(
    g: &mut Gate,
    d: &serde_json::Map<String, Value>,
    default_measured: &str,
) -> Vec<String> {
    let mut notes = Vec::new();
    if let Some(t) = d.get("title").and_then(|v| v.as_str()) {
        g.title = t.to_string();
    }
    if let Some(n) = str_vec(d.get("needles")) {
        if n.is_empty() {
            notes.push(format!("{}: needles 를 빈 배열로 덮으려 했다 — 무시(식별 불가)", g.id));
        } else {
            g.needles = n;
        }
    }
    if let Some(w) = str_vec(d.get("widget")) {
        g.widget = w;
    }
    if let Some(e) = str_vec(d.get("confirm_echo")) {
        g.confirm_echo = e;
    }
    if let Some(i) = d
        .get("default_index")
        .and_then(|v| v.as_u64())
        .filter(|n| *n > 0 && *n <= u8::MAX as u64)
    {
        g.default_index = Some(i as u8);
    }
    match parse_passability(d.get("passability")) {
        // ★조이는 방향만 허용. 로그인·OAuth 가 사람 1회를 요구하는 것은 정책이 아니라 측정
        //   결과이므로(자격증명 경로해시 봉인 · OAuth 무한 루프), 선언으로 뒤집게 두면
        //   "기계가 통과할 수 있다"는 거짓 전제 위에서 키를 쏘게 된다.
        Some(Passability::Machine) if g.passability == Passability::HumanOnly => {
            notes.push(format!(
                "{}: human_only → machine 승격 선언 거부(실측상 기계 통과 불가)",
                g.id
            ));
        }
        Some(p) => {
            g.passability = p;
            if p == Passability::HumanOnly {
                g.action = None;
            }
        }
        None => {}
    }
    if d.contains_key("action") {
        if g.passability == Passability::HumanOnly {
            notes.push(format!("{}: human_only 관문의 action 선언 무시", g.id));
        } else if d.get("action").is_some_and(|v| v.is_null()) {
            g.action = None;
        } else {
            match parse_action(d.get("action")) {
                Some(a) => g.action = Some(a),
                None => notes.push(format!("{}: action 선언이 손상 — 종전 액션 유지", g.id)),
            }
        }
    }
    if let Some(r) = d.get("human_reason").and_then(|v| v.as_str()) {
        g.human_reason = Some(r.to_string());
    }
    match parse_absence_cost(d.get("absence_cost")) {
        // ★조이는 방향만 허용(passability 와 같은 비대칭). 부재의 비용은 **실측된 귀결**이지
        //   정책이 아니다 — 면책 창의 Return 이 rc 1 이라는 사실은 선언으로 바뀌지 않는다.
        //   낮추기를 허용하면 봉투 한 줄로 킬체인 관문의 보호가 꺼진다.
        Some(AbsenceCost::Recoverable) if g.absence_cost == AbsenceCost::Fatal => {
            notes.push(format!(
                "{}: fatal → recoverable 완화 선언 거부(부재의 귀결은 실측 사실이다)",
                g.id
            ));
        }
        Some(c) => g.absence_cost = c,
        None => {}
    }
    g.measured_on = d
        .get("measured_on")
        .and_then(|v| v.as_str())
        .unwrap_or(default_measured)
        .to_string();
    notes
}

// ═══════════════════════════════════════════════════════════════════════════
// 실측 화면 픽스처 — 손으로 지어내지 않는다(PROBE_RESULTS 2026-08-23 전사).
// ═══════════════════════════════════════════════════════════════════════════

/// 실측 캡처 전사본. 소비자: 아래 테스트 · 뒤 단위(U-13 진리표)의 공유 사료.
///
/// ★작업경로 자리표시자 규약(2026-08-24 · PUBLIC 발행 게이트 `scripts/secret-scan.sh`).
///   화면에 실제로 찍히는 것은 **그 기계의 홈 경로**다. 원측정 대장은 그것을 이미
///   `<cwd>` 로 봉해 두었다(`docs/evidence/probe-2026-08-23-first-run-gates.json` §screens
///   `folder-trust.text` = `"Accessing workspace: <cwd>\n…"`). 그러므로 여기서 경로 자리를
///   `<cwd>` 로 두는 것은 전사를 **고치는** 것이 아니라 대장 문면으로 **되돌리는** 것이다.
///   관문이 아닌 대조군 화면은 대장에 항목이 없는 합성 픽스처이므로, "절대경로가 한 줄
///   찍혀 있다" 는 화면 모양만 지키면 된다 — 스캐너가 등재한 더미 username 을 쓴다
///   (`secret-scan.sh` `dummy_user_re` · 리포 관례 `/Users/x/` — `ui/src/deptlabel.test.ts:33`).
///   ★어느 쪽이든 경로 문자열은 **판정에 쓰이지 않는다**: 관문 성립은 `needles`(질문형 문면)
///   ∧ `widget`(위젯 서명)뿐이고, 어느 관문의 어느 항목에도 경로가 없다. 실경로로 되돌려도
///   식별력은 한 톨도 늘지 않고 발행만 막힌다.
pub mod fixtures {
    pub const THEME: &str = "Welcome to Claude Code v2.1.241\n\
        Let's get started.\n\n\
        Choose the text style that looks best with your terminal\n\
        \x20 1. Auto (match terminal)\n\
        ❯ 2. Dark mode ✔\n\
        \x20 3. Light mode\n";

    pub const LOGIN_METHOD: &str = "Select login method:\n\
        ❯ 1. Claude account with subscription · Pro, Max, Team, or Enterprise\n\
        \x20 2. Anthropic Console account · API usage billing\n\
        \x20 3. 3rd-party platform\n";

    pub const OAUTH_CODE: &str = "Opening browser to sign in…\n\
        Browser didn't open? Use the url below to sign in (c to copy)\n\
        https://claude.com/cai/oauth/authorize?x=1\n\
        Paste code here if prompted >\n";

    pub const FOLDER_TRUST: &str = "Accessing workspace: <cwd>\n\
        Quick safety check: Is this a project you created or one you trust?\n\
        ❯ 1. Yes, I trust this folder\n\
        \x20 2. No, exit\n\
        Enter to confirm · Esc to cancel\n";

    /// ★킬체인 화면: 폴더신뢰를 통과한 **직후**. 확인 에코가 남아 있고 면책 창이 떠 있다.
    pub const TRUST_ECHO_THEN_DISCLAIMER: &str = "Yes, I trust this folder ✔\n\
        ─────────────────────────────────────────\n\
        WARNING: Claude Code running in Bypass Permissions mode\n\
        In Bypass Permissions mode, Claude Code will not ask for your approval …\n\
        ❯ 1. No, exit\n\
        \x20 2. Yes, I accept\n\
        Enter to confirm · Esc to cancel\n";

    pub const FEATURE_FULLSCREEN: &str = "Try the new fullscreen renderer?\n\
        · Flicker-free output  · Mouse support  · Selected text auto-copies\n\
        ❯ 1. Yes, try it\n\
        \x20 2. Not now\n\
        Enter to confirm · Esc to cancel\n";

    /// 관문이 아닌 정상 화면(오탐 대조군).
    pub const READY_SHELL: &str = "> worker ready. no prompts here.\n\
        ? for shortcuts\n";

    // ── ★오탐 대조군(2026-08-24 · 리뷰어 e2e 재현 · BLOCK-1/BLOCK-2) ────────────
    //
    // 아래 화면들은 **관문이 아니다.** 종전 코퍼스는 이 화면들을 관문으로 식별했고 그 귀결이
    // 영구 부트 라이브락(BLOCK-1)과 다른 CLI 오처방(BLOCK-2)이었다. 검체
    // `no_gate_matches_a_non_gate_screen` · `no_needle_alone_matches_a_non_gate_screen` 이
    // 이 집합 전량에 대해 코퍼스를 전수 대조한다.

    /// ★BLOCK-1 e2e 재현 — 온보딩이 **끝난** 정상 claude 노드의 첫 화면.
    /// 인사 배너와 `❯` 가 함께 있다: 종전 theme 선언(배너 needle + `❯` 단독 위젯)은 바로 이
    /// 화면을 `관문 보류: 온보딩 · 테마 선택(id=theme)` 으로 잡아 rc 78 을 냈다.
    pub const HEALTHY_WELCOME_BOX: &str = "✻ Welcome to Claude Code!\n\
        \x20 /help for help, /status for your current setup\n\
        \x20 cwd: /Users/x/work\n\
        ❯ \n";

    /// ★P3-0 픽스처 — **살아 있는** Claude Code TUI. 꼬리가 `❯`(입력 프롬프트 그 자체)이고
    /// 상태줄이 살아 있다. 배너·프레임 전사 근거는 PROBE_RESULTS_WINDOWS.md WIN-2 실화면
    /// (`─ Claude Code ─` · `Welcome back …` · 모델/플랜 줄).
    ///
    /// 이 부류가 **안전 밸브의 시험 대상**이다: 꼬리가 `❯` 라 '끝문자 4종' 술어로는 셸
    /// 프롬프트로 읽히지만, 화면은 명백히 TUI 를 그리고 있다.
    pub const LIVE_TUI_AT_PROMPT: &str = "─ Claude Code ─\n\
        \x20 Welcome back user!   Opus 5 (1M context) · Claude Max\n\
        \x20 /Users/x/work\n\
        ? for shortcuts\n\
        \x20 …43% context left\n\
        ❯ \n";

    /// ★BLOCK-2 e2e 재현 ① — **다른 CLI** 가 자기 브라우저 로그인을 진행하는 화면.
    /// 종전 oauth-code 선언은 `"Opening browser to sign in"` 을 가드 없이 needle 로 들고
    /// 있어 이 화면을 claude 의 OAuth 관문(human_only)으로 식별했다.
    pub const FOREIGN_CLI_BROWSER_LOGIN: &str = "gh auth login\n\
        ! First copy your one-time code: ABCD-1234\n\
        Opening browser to sign in to github.com …\n\
        Press Enter to open github.com in your browser...\n";

    /// ★BLOCK-2 e2e 재현 ② — 그 문자열을 **grep 한 출력**(또는 로그 한 줄).
    /// 화면에 문자열이 '있다'는 것과 그 관문이 '떠 있다'는 것은 다른 사실이다.
    pub const GREP_OUTPUT_MENTIONING_OAUTH_ERROR: &str =
        "$ grep -rn 'OAuth error: Invalid code' logs/\n\
        logs/boot-2026-08-23.log:412: OAuth error: Invalid code. Please make sure …\n\
        $ \n";

    /// 관문 문면이 **본문으로** 출력된 화면(감사 문서·소스 열람) 중 코퍼스가 **닫을 수 있는** 쪽.
    ///
    /// ★P4-8 수리(2026-08-24 적대 리뷰어): 종전 내용은 `[launch-agent] ready(…)` ·
    ///   `[boot] worker=claude` · 셸 프롬프트 세 줄뿐이라 **이름이 약속한 관문 문면을 한 글자도
    ///   담지 않았다**. 어떤 코퍼스를 넣어도 통과하는 화면은 아무것도 재지 못한다(공허한 대조군).
    ///   지금 내용은 관문 **넷의 위젯 서명을 전부 만족**한다(`Auto (match terminal)` ·
    ///   `Dark mode` · `Enter to confirm` · `Esc to cancel`). 그런데도 관문으로 잡히면 안 된다 —
    ///   즉 이 화면에서 일하는 것은 오직 **needle 축**이고, 이 대조군이 그 사실을 실행으로 증명한다.
    ///
    /// ★needle 까지 본문에 실린 화면은 이 표에 **넣을 수 없다**. 원리와 실제 방어선은
    ///   [`BODY_TEXT_SCREENS`] 의 doc 에 있다(잔여 위험 명시).
    pub const AUDIT_LOG_LINE: &str = "❯ cat _round/handoffs/boot-gate-audit.md\n\
        \x20 ## 관문 코퍼스 감사 — 위젯 서명 전사(2026-08-24)\n\
        \x20 | id | 제목 | 위젯 서명 | 기본 포커스 |\n\
        \x20 | theme | 온보딩 · 테마 선택 | Auto (match terminal) / Dark mode | 2 |\n\
        \x20 | folder-trust | 작업 폴더 신뢰 확인 | Enter to confirm · Esc to cancel | 1 |\n\
        \x20 | bypass-disclaimer | 면책 확인 | Enter to confirm · Esc to cancel | 1 = No, exit |\n\
        \x20 | feature-announce-fullscreen | 신기능 안내 | Enter to confirm · Esc to cancel | 1 |\n\
        \x20 실측 기본 포커스 행 전사: `❯ 2. Dark mode ✔`\n\
        user@mac cys-terminal-rel %\n";

    /// ★살아 있는 claude 세션의 **권한 프롬프트**(관문이 **아니다**).
    ///
    /// 첫기동 관문이 아니라 작업 중 수시로 뜨는 화면인데, 관문 3종(폴더신뢰·면책·신기능)의
    /// **위젯 서명을 그대로** 갖고 있고 질문형 문면까지 있다.
    ///
    /// ★P4-9 의 이빨: `"Do you want to proceed?"` 를 needle 로 들이면 물음표가 **끝 구두점**이라
    ///   질문형 심사를 통과한다. 질문형이라는 사실만으로는 "그 관문에서만 나온다" 가 보장되지
    ///   않는다는 증거가 이 화면이고, [`super::gate_rule_violations`] 가 이 표를 전수 대조한다.
    pub const LIVE_PERMISSION_PROMPT: &str = "● Bash(cargo test --lib)\n\
        \x20 ⎿ Running…\n\
        Do you want to proceed?\n\
        ❯ 1. Yes\n\
        \x20 2. Yes, and don't ask again for cargo commands\n\
        \x20 3. No, tell Claude what to do differently\n\
        Enter to confirm · Esc to cancel\n";

    /// ★`/config` 화면 — **테마 관문의 위젯 서명이 전부** 실려 있지만 관문은 아니다.
    /// 선택지 라벨은 설정·문서 화면 어디에나 실리므로 식별의 근거가 될 수 없다.
    pub const CONFIG_THEME_SETTING: &str = "❯ /config\n\
        \x20 Settings\n\
        \x20 Theme          Dark mode\n\
        \x20 Available      Auto (match terminal), Dark mode, Light mode\n\
        \x20 Notifications  off\n\
        ❯ \n";

    /// ★`/status` 화면 — **로그인 관문의 위젯 서명이 전부** 실려 있지만 관문은 아니다.
    pub const ACCOUNT_STATUS_PANEL: &str = "❯ /status\n\
        \x20 Account        Claude account with subscription · Max\n\
        \x20 Alternative    Anthropic Console account · API usage billing\n\
        \x20 Model          Opus 5 (1M context)\n\
        ❯ \n";

    /// ★인가 URL 이 본문에 실린 문서 열람 — **OAuth 관문의 위젯 서명(URL)이 그대로** 있지만
    /// 관문은 아니다.
    pub const DOC_MENTIONING_OAUTH_URL: &str = "❯ cat docs/login.md\n\
        \x20 로그인은 브라우저에서 https://claude.com/cai/oauth/authorize 로 진행한다.\n\
        \x20 코드는 사람이 1회 붙여넣는다(기계 대행 불가).\n\
        user@mac cys-terminal-rel %\n";

    /// ★관문이 **아닌** 화면 전량(id, 화면). 코퍼스 자기규칙 검체가 이 표를 전수로 돈다.
    ///
    /// 이 표의 **계약**: 어떤 관문도 이 화면들을 식별하지 않고(`no_gate_matches_a_non_gate_screen`),
    /// 어떤 needle 도 이 화면들에 **단독으로** 걸리지 않는다(`no_needle_alone_matches_a_non_gate_screen`).
    /// 뒤 조항 때문에 **needle 문면을 본문에 담은 화면은 이 표에 들어올 수 없다** — 그런 화면은
    /// [`BODY_TEXT_SCREENS`] 가 따로 받는다.
    pub const NON_GATE_SCREENS: &[(&str, &str)] = &[
        ("ready-shell", READY_SHELL),
        ("healthy-welcome-box", HEALTHY_WELCOME_BOX),
        ("live-tui-at-prompt", LIVE_TUI_AT_PROMPT),
        ("foreign-cli-browser-login", FOREIGN_CLI_BROWSER_LOGIN),
        ("grep-output", GREP_OUTPUT_MENTIONING_OAUTH_ERROR),
        ("audit-log-line", AUDIT_LOG_LINE),
        ("live-permission-prompt", LIVE_PERMISSION_PROMPT),
        ("config-theme-setting", CONFIG_THEME_SETTING),
        ("account-status-panel", ACCOUNT_STATUS_PANEL),
        ("doc-mentioning-oauth-url", DOC_MENTIONING_OAUTH_URL),
    ];

    /// ★정본 소스 열람 — needle 이 **본문으로** 실린 화면(`cat src/first_run_gates.rs`).
    pub const CAT_GATE_CORPUS_SOURCE: &str = "❯ cat src/first_run_gates.rs\n\
        \x20 …\n\
        \x20     Def {\n\
        \x20         id: \"theme\",\n\
        \x20         title: \"온보딩 · 테마 선택\",\n\
        \x20         needles: &[\"Choose the text style that looks best with your terminal\"],\n\
        \x20         widget: &[\"Auto (match terminal)\", \"Dark mode\"],\n\
        \x20     },\n\
        \x20 …\n\
        user@mac cys-terminal-rel %\n";

    /// ★관문 문면이 **needle 까지 본문으로** 실린 화면 — 코퍼스가 **원리상 닫을 수 없는** 쪽.
    ///
    /// ## 왜 [`NON_GATE_SCREENS`] 에 넣을 수 없는가 (P4-8 · 2026-08-24)
    ///
    /// 저 표의 계약은 "어떤 needle 도 이 화면에 걸리지 않는다" 이고, 관문 식별기는 화면 텍스트에
    /// 대한 **부분문자열 검사**다. 그런데 이 코퍼스의 정본은 **이 소스 파일 자신**이므로, 소스를
    /// 화면에 출력한 화면은 정의상 **모든 needle 을 글자 그대로 포함**한다(자기참조). 즉 그
    /// 계약은 이 화면에 대해 **만족 불가능**이며, 통과시키려고 needle 을 바꾸면 다음 판의 소스가
    /// 다시 그 새 needle 을 담는다. 이 불가능성은 주장이 아니라 검체
    /// `body_text_screens_are_unclosable_at_the_corpus_layer` 가 소스 자신(`include_str!`)을 읽어
    /// **기계로 증명**한다.
    ///
    /// ## 그래서 무엇이 이 화면을 막는가 — **생애 창**(코퍼스가 아니다)
    ///
    /// 주입 가드(`inject_guard::decide`)의 `awakened` 래치와 데몬 스캐너
    /// (`governance::gate_scan_open`)의 창이 그것이다: 첫 각성 ack 이후에는 스캔 자체를 하지
    /// 않는다. `readiness::judge` 의 관문 축에도 같은 방향의 창이 P4-7 에서 들어왔다.
    ///
    /// ## ★잔여 위험 (명시)
    ///
    /// **첫 각성 ack 이전**(부트 창 안)에 좌석이 이 화면을 그리면 관문으로 식별된다 →
    /// `GatePending`(보류 · 좌석 보존 · 키 0 · 주입 0)으로 접히고, 사람이 화면을 넘기면 풀린다.
    /// 부트 창 안에서 감사 문서·소스를 여는 좌석은 정상 시나리오가 아니므로 이 잔여 위험은
    /// 받아들인다 — 그리고 그 귀결이 **보류이지 파괴가 아니라는 것**이 받아들이는 근거다.
    pub const BODY_TEXT_SCREENS: &[(&str, &str)] =
        &[("cat-gate-corpus-source", CAT_GATE_CORPUS_SOURCE)];
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── 코퍼스 불변식 ──────────────────────────────────────────────────────

    #[test]
    fn corpus_shape_is_well_formed() {
        let gs = builtin();
        assert_eq!(gs.len(), 6, "실측 관문은 6종이다(테마·로그인·OAuth·폴더신뢰·면책·신기능)");
        let mut ids: Vec<&str> = gs.iter().map(|g| g.id.as_str()).collect();
        ids.sort_unstable();
        let n = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), n, "관문 id 중복");
        for g in &gs {
            assert!(!g.id.is_empty() && !g.title.is_empty(), "{}: id/title 결손", g.id);
            assert!(!g.needles.is_empty(), "{}: needle 0 = 식별 불가 선언", g.id);
            assert_eq!(g.measured_on, MEASURED_ON, "{}: measured_on 드리프트", g.id);
            assert_eq!(g.origin, Origin::Builtin);
            match g.passability {
                Passability::HumanOnly => {
                    assert!(g.action.is_none(), "{}: 사람 전용인데 액션이 선언됐다", g.id);
                    assert!(g.human_reason.is_some(), "{}: 사람 전용 사유 미선언", g.id);
                }
                Passability::Machine => {
                    assert!(g.action.is_some(), "{}: 기계 통과 가능인데 액션이 없다", g.id);
                    assert!(
                        g.down_presses().is_some(),
                        "{}: 기본 포커스에서 목표까지의 키 시퀀스를 산출할 수 없다",
                        g.id
                    );
                }
            }
            // ★부재의 비용 선언이 **화면 실측과 어긋나지 않는가**(P4-10).
            //
            //   ⓐ 사람 전용 관문의 부재는 언제나 비가역이다 — 기계가 통과시킬 수 없는 화면에
            //     키가 나가면 좌석은 살아 있는 채로 갇히고(OAuth 무한 재시도) 생존만 보는
            //     판정이 그것을 영원히 '준비됨'으로 읽는다.
            //   ⓑ 기계 통과 가능이어도 **기본 포커스가 통과 액션이 아니면**(아래키 ≥ 1) 부재는
            //     비가역이다 — 주입의 Return 한 발이 기본 포커스를 확정해 버리고, 면책 창에서
            //     그것은 곧 `No, exit`(rc 1)다.
            //   선언이 이 둘보다 느슨하면(= Recoverable) 집행이 그 관문을 지우도록 허가한
            //   것이므로 적색이다. 반대로 이 둘에 걸리지 않는 관문을 Fatal 로 **조여** 선언하는
            //   것은 허용한다(folder-trust 가 그 사례 — 킬체인의 1발째 자리라는 사고 근거).
            let return_commits_a_non_pass = g.down_presses().map(|d| d > 0).unwrap_or(true);
            if g.passability == Passability::HumanOnly || return_commits_a_non_pass {
                assert_eq!(
                    g.absence_cost,
                    AbsenceCost::Fatal,
                    "{}: 부재의 비용이 Recoverable 로 선언됐지만 이 관문은 부재 시 주입의 \
                     Return 이 비가역 결과(좌석 종료·허위 READY 영구화)를 낸다 — 선언이 실측과 \
                     어긋나면 집행이 이 관문을 지워도 아무도 막지 않는다",
                    g.id
                );
            }
        }
        // 실측 6종 중 **부재가 비가역인 것**이 다수다 — 이 사실이 뒤집히면(전부 가역) 위
        // 판정기가 고장난 것이므로 계측이 무효다(공허한 초록 방지).
        assert!(
            gs.iter().filter(|g| g.absence_is_fatal()).count() >= 5,
            "킬체인 관문이 5건 미만 — 부재의 비용 판정기가 고장났거나 코퍼스 서사가 바뀌었다"
        );
    }

    /// ★2026-07-29 킬체인의 형태를 **구조적으로** 금지한다.
    /// 그 사고는 확인 에코(`Yes, I trust this folder ✔`)가 needle(`trustthisfolder`)에
    /// 재매칭돼 2발째 Return 이 면책 창을 눌러 좌석을 죽인 것이다.
    #[test]
    fn no_needle_is_contained_in_any_confirm_echo() {
        let gs = builtin();
        let echoes: Vec<String> = gs.iter().flat_map(|g| g.confirm_echo.clone()).collect();
        for g in &gs {
            for n in &g.needles {
                for e in &echoes {
                    assert!(
                        !normalize(e).contains(&normalize(n)) && !flatten(e).contains(&flatten(n)),
                        "{}: needle {n:?} 가 확인 에코 {e:?} 에 포함된다 — 킬체인 재발 형태",
                        g.id
                    );
                }
            }
        }
    }

    // ── ★코퍼스 자기규칙(BLOCK-1/BLOCK-2 구조적 재발 차단) ──────────────────

    /// ⓐ 모든 needle 은 질문형이거나, 면제표에 **근거와 함께** 등재돼 있어야 한다.
    /// 그리고 면제표에 정본이 더는 선언하지 않는 항목이 남아 있어도 적색이다(쓰레기통 금지).
    #[test]
    fn corpus_self_rule_a_every_needle_is_question_form_or_justified() {
        let gs = builtin();
        let mut used: Vec<(&str, &str)> = Vec::new();
        for g in &gs {
            // ★규칙 ⓐ의 둘째 절반(P4-9): **질문형이어도** 대조군 통과는 여전히 요구된다.
            //   질문형이라는 사실은 "그 관문에서만 나온다" 를 조금도 보장하지 않는다.
            let v = gate_rule_violations(g);
            assert!(v.is_empty(), "{}", v.join(" · "));
            for n in &g.needles {
                // ★P4-9: 물음표가 **끝 구두점**일 때만 질문형이다. 종전 `n.contains('?')` 는
                //   위치를 보지 않아 문장 중간의 `?` 하나로 면제 심사를 통째로 건너뛰었다.
                if is_question_form(n) {
                    continue;
                }
                let hit = NEEDLE_EXEMPTIONS
                    .iter()
                    .find(|(gid, nd, _)| *gid == g.id && nd == n);
                let Some((gid, nd, why)) = hit else {
                    panic!(
                        "{}: needle {n:?} 이 질문형도 아니고 면제표에도 없다 — '그 관문이 떠 \
                         있을 때만 나타나는가' 를 근거와 함께 선언하라(배너·상태·에러 문자열 금지)",
                        g.id
                    );
                };
                assert!(
                    why.chars().filter(|c| !c.is_whitespace()).count() >= 20,
                    "{gid}: needle {nd:?} 의 면제 근거가 사실상 비어 있다 — 근거 없는 면제는 면제가 \
                     아니라 통과다"
                );
                used.push((gid, nd));
            }
        }
        for (gid, nd, _) in NEEDLE_EXEMPTIONS {
            assert!(
                used.contains(&(gid, nd)),
                "면제표 항목 ({gid}, {nd:?}) 이 정본 선언에 없다 — 면제표가 쓰레기통이 되면 다음 \
                 감사자가 '선언됐다' 고 오독한다"
            );
        }
    }

    /// ⓑⓒ widget AND 가드는 비어 있어도, 보편 토큰 단독이어도 안 된다.
    #[test]
    fn corpus_self_rule_bc_widget_and_guard_is_present_and_not_universal() {
        for g in builtin() {
            let v = widget_rule_violations(&g);
            assert!(v.is_empty(), "{}", v.join(" · "));
        }
        // 보편 토큰 판정 자체의 대조군(판정기가 실제로 구분하는가).
        assert!(is_universal_widget_token("❯"));
        assert!(is_universal_widget_token(" ❯ "));
        assert!(!is_universal_widget_token("Enter to confirm"));
        assert!(!is_universal_widget_token("Auto (match terminal)"));
    }

    /// ★규칙 ⓐ의 이빨 — **면제 문장과 무관하게** 어떤 needle 도 관문이 아닌 화면에 단독으로
    /// 걸리면 안 된다. 단독으로 보는 이유: 감지 경로(`inject_guard::needle_hit`)는 위젯 AND 를
    /// 보지 않으므로, needle 이 스스로 관문 전용이 아니면 그 경로가 통째로 오탐한다.
    #[test]
    fn no_needle_alone_matches_a_non_gate_screen() {
        for g in builtin() {
            for n in &g.needles {
                for &(sid, screen) in fixtures::NON_GATE_SCREENS {
                    let (norm, flat) = (normalize(screen), flatten(screen));
                    assert!(
                        !norm.contains(&normalize(n)) && !flat.contains(&flatten(n)),
                        "{}: needle {n:?} 이 **관문이 아닌** 화면 {sid} 에 걸린다 — 정상 화면에도 \
                         나타나는 문면은 관문의 근거가 될 수 없다(오탐의 귀결은 영구 부트 라이브락)",
                        g.id
                    );
                }
            }
        }
    }

    /// 위와 같은 축을 **판정 경로 그대로**(needle ∧ 위젯) 확인한다.
    #[test]
    fn no_gate_matches_a_non_gate_screen() {
        let gs = builtin();
        for &(sid, screen) in fixtures::NON_GATE_SCREENS {
            assert_eq!(
                identify(&gs, screen).map(|g| g.id.clone()),
                None,
                "관문이 아닌 화면 {sid} 가 관문으로 식별됐다 — 화면에 통과시킬 관문이 없으므로 \
                 사람도 풀 수 없다(영구 부트 라이브락)"
            );
        }
    }

    /// ★계측 타당성(in-band) — **수리 전 선언**을 그대로 지어 넣으면 위 규칙이 전부 적색이다.
    /// 이 대조군이 없으면 위 검체들이 "원래 안 나는 일을 안 난다고 확인" 하는 공허한 초록일 수 있다.
    #[test]
    fn self_rules_are_red_on_the_pre_fix_declarations() {
        let mk = |id: &str, needles: &[&str], widget: &[&str]| Gate {
            id: id.to_string(),
            title: "(수리 전 선언)".to_string(),
            needles: needles.iter().map(|s| s.to_string()).collect(),
            widget: widget.iter().map(|s| s.to_string()).collect(),
            confirm_echo: vec![],
            passability: Passability::Machine,
            default_index: Some(1),
            action: None,
            human_reason: None,
            absence_cost: AbsenceCost::Recoverable,
            measured_on: MEASURED_ON.to_string(),
            origin: Origin::Builtin,
        };

        // ── BLOCK-1: 배너 needle + `❯` 단독 위젯 ──
        let old_theme = mk(
            "theme",
            &[
                "Choose the text style that looks best with your terminal",
                "Welcome to Claude Code",
            ],
            &["❯"],
        );
        assert!(
            !widget_rule_violations(&old_theme).is_empty(),
            "규칙 ⓒ가 `❯` 단독 위젯을 잡지 못한다 — 계측 무효"
        );
        assert!(
            old_theme.matches(fixtures::HEALTHY_WELCOME_BOX),
            "구 theme 선언이 건강한 웰컴 화면을 잡지 않는다면 BLOCK-1 서사가 틀린 것 — 근거를 \
             재확인하라"
        );
        // 그리고 지금 정본은 같은 화면을 잡지 않는다.
        let gs = builtin();
        assert!(identify(&gs, fixtures::HEALTHY_WELCOME_BOX).is_none());

        // ── BLOCK-2: AND 가드 0 + 상태·에러 needle ──
        let old_oauth = mk(
            "oauth-code",
            &[
                "Paste code here if prompted",
                "Opening browser to sign in",
                "Browser didn't open? Use the url below to sign in",
                "OAuth error: Invalid code",
            ],
            &[],
        );
        assert!(
            !widget_rule_violations(&old_oauth).is_empty(),
            "규칙 ⓑ가 빈 위젯을 잡지 못한다 — 계측 무효"
        );
        assert!(
            old_oauth.matches(fixtures::FOREIGN_CLI_BROWSER_LOGIN),
            "구 oauth-code 선언이 다른 CLI 의 브라우저 로그인 화면을 잡지 않는다면 BLOCK-2 서사가 \
             틀린 것"
        );
        assert!(
            old_oauth.matches(fixtures::GREP_OUTPUT_MENTIONING_OAUTH_ERROR),
            "구 oauth-code 선언이 grep 출력을 잡지 않는다면 BLOCK-2 서사가 틀린 것"
        );
        // 지금 정본은 둘 다 잡지 않는다.
        for &(sid, screen) in &[
            ("foreign-cli", fixtures::FOREIGN_CLI_BROWSER_LOGIN),
            ("grep", fixtures::GREP_OUTPUT_MENTIONING_OAUTH_ERROR),
        ] {
            assert!(identify(&gs, screen).is_none(), "{sid} 오탐 잔존");
        }
    }

    /// ★P4-3 — **자기규칙이 프로덕션 경로에서 집행된다.**
    ///
    /// 리뷰어 격리 실행이 재현한 BLOCK-1 복원 경로를 그대로 먹인다: override 봉투 한 줄
    /// (`"widget": []`)이 빌트인 관문의 AND 가드를 비우고, 배너 needle 하나로 관문이 성립해
    /// 건강한 노드 전원이 `gate_pending` 으로 접힌다(영구 부트 라이브락).
    ///
    /// ★P4-10 정정: 집행 수단은 '버리기' 가 아니라 **수리**다. 판정 넷을 한 자리에서 본다 —
    /// ⓐ그 관문이 **코퍼스에 남는다**(버려지지 않는다) ⓑ`notes` 에 **사유가 남는다**
    /// ⓒ`identify` 가 **정상 화면을 잡지 않는다**(성질 ③ — BLOCK-1 재발 차단)
    /// ⓓ출처 회계가 실제 코퍼스를 말한다.
    #[test]
    fn production_resolve_repairs_the_violating_gate_instead_of_dropping_it() {
        // 리뷰어 격리 harness 가 쓴 봉투 그대로.
        let env = json!({"gates": [
            {"id": "theme", "needles": ["Welcome to Claude Code"], "widget": []}
        ]});

        // ★계측 타당성(in-band): 집행이 없었다면 이 봉투가 무엇을 만들었는지 먼저 못 박는다.
        //   `resolve_raw` = 자기규칙 집행 **전**의 코퍼스다.
        let raw = resolve_raw(Some(&env), true);
        let raw_theme = raw.gates.iter().find(|g| g.id == "theme").expect("집행 전 theme");
        assert!(raw_theme.widget.is_empty(), "봉투가 AND 가드를 비우지 못한다면 서사가 틀렸다");
        assert!(
            !gate_rule_violations(raw_theme).is_empty(),
            "규칙이 이 선언의 위반을 알지 못한다 — 계측 무효"
        );
        assert_eq!(
            identify(&raw.gates, fixtures::HEALTHY_WELCOME_BOX).map(|g| g.id.clone()),
            Some("theme".to_string()),
            "집행 전 코퍼스가 건강한 노드를 잡지 않는다면 BLOCK-1 복원 서사가 틀린 것"
        );

        // ── 집행 후 ──
        let r = resolve_with(Some(&env), true);
        // ⓐ ★위반 관문은 **버려지지 않는다** — 위반한 축만 정본으로 복원된다(P4-10).
        //   버리면 그 관문의 귀결이 `보류 → 주입` 으로 뒤집히고, 그것이 재난 ④의 기전이다.
        let theme = r
            .gates
            .iter()
            .find(|g| g.id == "theme")
            .expect("자기규칙 위반을 이유로 관문이 코퍼스에서 사라졌다(P4-10 결함 ②의 형태)");
        assert_eq!(r.gates.len(), 6, "집행이 코퍼스의 관문 수를 바꿨다");
        assert_eq!(
            theme.widget,
            builtin().iter().find(|b| b.id == "theme").unwrap().widget,
            "위젯 AND 가드가 정본 서명으로 복원되지 않았다"
        );
        assert_eq!(
            theme.needles,
            builtin().iter().find(|b| b.id == "theme").unwrap().needles,
            "정상 화면에 걸리는 배너 needle 이 정본 needle 로 복원되지 않았다"
        );
        // ⓑ 사유가 남는다(조용히 고치지 않는다).
        assert!(
            r.notes.iter().any(|n| n.contains("복원") && n.contains("theme")),
            "복원 사유가 notes 에 없다: {:?}",
            r.notes
        );
        // ⓒ 그래서 건강한 노드가 관문으로 잡히지 않는다 = 영구 부트 라이브락이 닫힌다.
        assert_eq!(identify(&r.gates, fixtures::HEALTHY_WELCOME_BOX), None);
        for &(sid, screen) in fixtures::NON_GATE_SCREENS {
            assert_eq!(identify(&r.gates, screen), None, "{sid} 오탐 잔존");
        }
        // ⓓ 출처 회계는 실제 코퍼스를 말한다 — 봉투가 theme 를 덮은 것은 사실이므로 그렇게 센다
        //   (관문을 지우지 않았으니 '버렸는데 overridden 1' 같은 거짓말이 생길 자리도 없다).
        assert_eq!(r.source, Source::Merged { overridden: 1, added: 0 });
        // ⓔ ★그리고 실측 관문 화면에서의 귀결은 한 톨도 바뀌지 않았다(수리가 관문을 죽이지 않는다).
        assert_eq!(
            identify(&r.gates, fixtures::THEME).map(|g| g.id.clone()),
            Some("theme".to_string()),
            "수리 후 정작 진짜 테마 관문을 못 잡는다 — 관문을 잃지 않는 것이 수리의 목적이다"
        );
    }

    /// 실측 관문 화면(등장 순서) — 아래 **방향 불역전** 검체가 이 표를 전수로 돈다.
    /// 문면은 픽스처를 **참조**한다(사본 0).
    const MEASURED_GATE_SCREENS: &[(&str, &str)] = &[
        ("theme", fixtures::THEME),
        ("login-method", fixtures::LOGIN_METHOD),
        ("oauth-code", fixtures::OAUTH_CODE),
        ("folder-trust", fixtures::FOLDER_TRUST),
        ("bypass-disclaimer", fixtures::TRUST_ECHO_THEN_DISCLAIMER),
        ("feature-announce-fullscreen", fixtures::FEATURE_FULLSCREEN),
    ];

    /// ★★P4-10 성질 ① **사용자 주권** — 디스크 선언이 임베드·코드 정본에 덮이지 않는다.
    ///
    /// 【수리 전에는 왜 적색인가】 첫 판의 집행은 위젯 AND 가드가 없는 이 선언을 **버렸고**,
    /// 뒤이은 "집행 후 코퍼스가 비면 코드 정본으로 되돌린다" 폴백이 벤더 6종을 다시 세웠다.
    /// 사용자가 하나를 선언했는데 결과가 여섯이면 그것은 override 가 아니라 **무시**다.
    /// 같은 형태가 프로덕션 경로에도 핀으로 있다 —
    /// `cys.rs::h_deliver_1_old_agents_json_receives_new_key_from_embed` ⑥(디스크 선언이 이긴다).
    #[test]
    fn user_replace_declaration_survives_the_self_rule_enforcement() {
        // `cys.rs` 핀이 쓰는 봉투와 같은 모양 — 위젯 없이 needle 하나.
        let env = json!({"source": "replace",
                         "gates": [{"id": "mine", "needles": ["Proceed with the migration?"]}]});

        // ★계측 타당성(in-band): 집행 전 코퍼스가 무엇인지 먼저 못 박는다.
        let raw = resolve_raw(Some(&env), true);
        assert_eq!(raw.gates.len(), 1, "봉투가 코퍼스를 교체하지 못한다면 서사가 틀렸다");
        assert!(
            !gate_rule_violations(&raw.gates[0]).is_empty(),
            "이 선언이 자기규칙 위반이 아니라면 이 검체는 아무것도 재지 못한다(계측 무효)"
        );

        // ── 집행 후: 그대로 살아 있다.
        let r = resolve_with(Some(&env), true);
        // ★N1 정정 — 정본은 "선언 1건 + Fatal 바닥"이다. 종전 이 자리의 `len()==1` 은
        //   "replace 한 줄이 킬체인 관문 5종을 없애는 것이 정상"을 박제하고 있었다.
        //   주권 침해와 Fatal 바닥을 가르는 **판별자**는 개수가 아니라 `theme` 다:
        //   종전 실패 모드(폴백이 벤더 6종을 다시 세움)에서는 가역 관문인 `theme` 까지
        //   되살아났고, 바닥은 그것을 절대 되살리지 않는다.
        let fatal = fatal_builtin_ids();
        assert_eq!(
            r.gates.len(),
            1 + fatal.len(),
            "디스크 선언이 코드 정본에 덮였거나 Fatal 바닥이 사라졌다 — notes: {:?}",
            r.notes
        );
        assert!(
            !r.gates.iter().any(|g| g.id == "theme"),
            "가역 관문까지 되살아났다 = 코드 정본 폴백(사용자 주권 침해)의 형태다"
        );
        for id in &fatal {
            assert!(
                r.gates.iter().any(|g| &g.id == id),
                "Fatal 관문 {id} 이 replace 선언 한 줄로 사라졌다"
            );
        }
        assert_eq!(r.gates[0].id, "mine", "사용자 선언이 첫 자리에서 밀려났다");
        assert_eq!(r.gates[0].needles, vec!["Proceed with the migration?".to_string()]);
        assert!(matches!(r.source, Source::Replaced { count: 1 }));
        // 유지의 사유는 남는다(조용한 통과가 아니다).
        assert!(
            r.notes.iter().any(|n| n.contains("mine")),
            "유지 사유가 notes 에 없다: {:?}",
            r.notes
        );
    }

    /// ★★P4-10 성질 ② **실패 방향 불역전** — 이번 수리의 핵심 핀(재난 ④).
    ///
    /// 봉투가 `bypass-disclaimer` 의 위젯을 비우면 AND 가드가 0이 되어 needle 하나로 관문이
    /// 성립한다 = **보류**(안전측 오탐). 첫 판의 집행은 그 관문을 버렸고, 관문이 없으면 그
    /// 화면은 '준비됨'으로 읽혀 **주입**이 열린다. 그 창의 기본 포커스는 `No, exit` 이고
    /// Return 은 rc 1 이므로 **주입의 Return 이 곧 킬 스텝**이다 — 집행이 안전한 오탐을
    /// 좌석 사망으로 바꾼 것이다.
    ///
    /// 계약: 집행은 어떤 관문의 귀결도 `보류 → 주입` 으로 바꾸지 않는다.
    #[test]
    fn enforcement_never_reverses_a_gate_from_hold_to_inject() {
        let env = json!({"gates": [{"id": "bypass-disclaimer", "widget": []}]});

        // ── 계측 타당성(in-band): 집행 전 귀결이 **보류**임을 먼저 못 박는다.
        let raw = resolve_raw(Some(&env), true);
        let raw_disc = raw
            .gates
            .iter()
            .find(|g| g.id == "bypass-disclaimer")
            .expect("집행 전 면책 관문");
        assert!(raw_disc.widget.is_empty(), "봉투가 AND 가드를 비우지 못한다면 서사가 틀렸다");
        assert!(
            !gate_rule_violations(raw_disc).is_empty(),
            "규칙이 이 선언의 위반을 알지 못한다 — 계측 무효"
        );
        assert_eq!(
            identify(&raw.gates, fixtures::TRUST_ECHO_THEN_DISCLAIMER).map(|g| g.id.clone()),
            Some("bypass-disclaimer".to_string()),
            "집행 전 귀결이 보류가 아니라면 '역전' 서사가 성립하지 않는다"
        );

        // ── 집행 후: 귀결은 **여전히 보류**다.
        let r = resolve_with(Some(&env), true);
        let g = identify(&r.gates, fixtures::TRUST_ECHO_THEN_DISCLAIMER).unwrap_or_else(|| {
            panic!(
                "★집행이 면책 관문을 지워 귀결이 보류 → 주입 으로 뒤집혔다 — 그 화면의 기본 \
                 포커스는 `No, exit` 이고 주입의 Return 이 곧 rc 1 이다(재난 ④). notes: {:?}",
                r.notes
            )
        });
        assert_eq!(g.id, "bypass-disclaimer");
        assert_eq!(g.default_index, Some(1), "면책 기본 포커스(No, exit) 소실");
        assert_eq!(g.down_presses(), Some(1), "면책 통과 시퀀스 소실");
        assert_eq!(g.absence_cost, AbsenceCost::Fatal, "면책 관문의 부재 비용 선언 소실");
        // 수리는 정본 위젯 서명으로 AND 구멍도 함께 닫는다(성질 ③과 동시 만족).
        assert_eq!(
            g.widget,
            builtin()
                .iter()
                .find(|b| b.id == "bypass-disclaimer")
                .unwrap()
                .widget
        );

        // ── ★전수 대조: 봉투가 **어느 관문의** 위젯을 비우든, 실측 관문 6화면에서
        //    '집행 전에 잡히던 것이 집행 후에 안 잡히는' 일은 한 건도 없다.
        for &(gid, _) in MEASURED_GATE_SCREENS {
            let env = json!({"gates": [{"id": gid, "widget": []}]});
            let raw = resolve_raw(Some(&env), true);
            let done = resolve_with(Some(&env), true);
            for &(sid, screen) in MEASURED_GATE_SCREENS {
                if identify(&raw.gates, screen).is_some() {
                    assert!(
                        identify(&done.gates, screen).is_some(),
                        "봉투가 {gid} 를 위반시켰을 때 화면 {sid} 의 귀결이 보류 → 주입 으로 \
                         뒤집혔다(실패 방향 역전 — 재난 ④)"
                    );
                }
            }
            // 그리고 그 집행이 정상 화면을 새로 잡지도 않는다(성질 ③ 동시 유지).
            for &(sid, screen) in fixtures::NON_GATE_SCREENS {
                assert_eq!(identify(&done.gates, screen), None, "{gid}/{sid} 오탐 잔존");
            }
        }
    }

    /// ★★P4-10 성질 ③ **BLOCK-1 재발 차단** — 봉투로 빌트인 관문의 AND 가드를 비워
    /// **건강한 화면을 관문으로 잡는** 경로는 수리 뒤에도 막혀 있다.
    ///
    /// 성질 ①②를 만족시키느라 이 축이 열리면 수리가 아니라 맞바꾸기다. 그래서 같은 봉투
    /// 공격을 여기서 한 번 더 세운다(needle 을 배너로 바꾸고 widget 을 비운다).
    #[test]
    fn enforcement_still_closes_the_block1_envelope_attack() {
        let env = json!({"gates": [
            {"id": "theme", "needles": ["Welcome to Claude Code"], "widget": []}
        ]});

        // 계측 타당성: 집행이 없으면 이 봉투는 건강한 화면을 잡는다.
        let raw = resolve_raw(Some(&env), true);
        assert_eq!(
            identify(&raw.gates, fixtures::HEALTHY_WELCOME_BOX).map(|g| g.id.clone()),
            Some("theme".to_string()),
            "집행 전 봉투가 건강한 화면을 잡지 않는다면 BLOCK-1 서사가 틀린 것"
        );

        let r = resolve_with(Some(&env), true);
        assert_eq!(
            identify(&r.gates, fixtures::HEALTHY_WELCOME_BOX),
            None,
            "건강한 화면이 관문으로 잡힌다 — 화면에 통과시킬 관문이 없으므로 사람도 못 푼다\
             (영구 부트 라이브락). notes: {:?}",
            r.notes
        );
        // 그러면서 관문 자체는 살아 있다(성질 ②와 동시 만족 — 맞바꾸기가 아니다).
        assert_eq!(
            identify(&r.gates, fixtures::THEME).map(|g| g.id.clone()),
            Some("theme".to_string())
        );
        for &(sid, screen) in fixtures::NON_GATE_SCREENS {
            assert_eq!(identify(&r.gates, screen), None, "{sid} 오탐 잔존");
        }
    }

    /// ★부재의 비용 게이트 — 집행기가 킬체인 관문을 지우면 **되살린다**.
    ///
    /// [`repair_gate`] 는 관문을 지우지 않으므로 이 게이트는 평시에 무동작이다. 그래서
    /// 게이트가 실제로 무는지를 확인하려면 "지워진 상태"를 **지어 넣어** 직접 먹여야 한다
    /// (그렇지 않으면 이 검체는 아무것도 재지 못하는 공허한 초록이다).
    #[test]
    fn absence_cost_gate_restores_a_killed_kill_chain_gate() {
        let pre = builtin();
        let mut notes = Vec::new();

        // ⓐ 킬체인 관문(면책)이 사라진 코퍼스를 먹인다 → 되살아난다.
        let mut kept: Vec<Gate> = pre.iter().filter(|g| g.id != "bypass-disclaimer").cloned().collect();
        enforce_absence_cost(&pre, &mut kept, &mut notes);
        let g = kept
            .iter()
            .find(|g| g.id == "bypass-disclaimer")
            .expect("부재의 비용이 비가역인 관문이 되살아나지 않았다");
        assert_eq!(g.default_index, Some(1), "되살린 관문이 실측 기본 포커스를 잃었다");
        assert!(notes.iter().any(|n| n.contains("되살린다")), "되살린 사유가 조용하다");

        // ⓑ 부재가 가역인 관문(테마)은 되살리지 않는다 — 게이트가 "전부 되살리기"로 퇴화하면
        //    규칙 위반 선언까지 무조건 복구되어 BLOCK-1 이 되돌아온다.
        let mut notes2 = Vec::new();
        let mut kept2: Vec<Gate> = pre.iter().filter(|g| g.id != "theme").cloned().collect();
        enforce_absence_cost(&pre, &mut kept2, &mut notes2);
        assert!(
            kept2.iter().all(|g| g.id != "theme"),
            "부재가 가역인 관문까지 되살렸다 — 게이트가 비용 선언을 읽지 않는다"
        );
        assert!(notes2.iter().any(|n| n.contains("가역")), "제거 사실이 조용하다");

        // ⓒ 봉투는 부재의 비용을 **조일 수만** 있다(완화 거부).
        let loosen = json!({"gates": [{"id": "bypass-disclaimer", "absence_cost": "recoverable"}]});
        let r = resolve_with(Some(&loosen), true);
        let disc = r.gates.iter().find(|g| g.id == "bypass-disclaimer").unwrap();
        assert_eq!(disc.absence_cost, AbsenceCost::Fatal, "봉투 한 줄로 킬체인 보호가 꺼졌다");
        assert!(r.notes.iter().any(|n| n.contains("거부")), "완화 거부가 조용하다");
        let tighten = json!({"gates": [{"id": "theme", "absence_cost": "fatal"}]});
        let r = resolve_with(Some(&tighten), true);
        let th = r.gates.iter().find(|g| g.id == "theme").unwrap();
        assert_eq!(th.absence_cost, AbsenceCost::Fatal, "조이는 방향이 막혔다");
    }

    /// ★P4-9 합성 표본 — **탐지 능력 자체**를 시험한다.
    ///
    /// 트리에 위반이 0이면 탐지기가 고장나도 규칙 검체는 초록이다. 그래서 규칙이 잡아야 하는
    /// 선언을 **지어 넣어** 적색을 확인한다.
    #[test]
    fn question_form_rule_needs_terminal_punctuation_and_the_control_still_bites() {
        // ⓐ 질문형 판정: 물음표가 **끝** 구두점일 때만 참이다.
        assert!(is_question_form("Try the new fullscreen renderer?"));
        assert!(is_question_form("Do you want to proceed?  "));
        for loose in [
            "Browser didn't open? Use the url below to sign in",
            "Opening browser to sign in? no",
            "Do you want to proceed? Press y",
            "?",
            "   ?  ",
            "no question mark at all",
        ] {
            assert!(
                !is_question_form(loose),
                "질문형 판정이 다시 느슨해졌다({loose:?}) — 문장 중간 `?` 하나로 면제 심사가 \
                 통째로 건너뛰어진다(P4-9)"
            );
            // 종전 규칙(`contains('?')`)과의 차분 — 이 표본들이 실제로 규칙을 갈랐음을 못 박는다.
            if loose.contains('?') {
                assert!(
                    !is_question_form(loose),
                    "종전 규칙에서는 통과했을 표본이 새 규칙에서도 통과한다 — 계측 무효"
                );
            }
        }

        // ⓑ 질문형이어도 대조군 통과는 **여전히** 요구된다. 리뷰어가 든 그 문면을 그대로 쓴다:
        //    `"Do you want to proceed?"` = 살아 있는 claude 세션의 권한 프롬프트.
        let planted = Gate {
            id: "planted-permission".to_string(),
            title: "(합성 표본)".to_string(),
            needles: vec!["Do you want to proceed?".to_string()],
            widget: vec!["Enter to confirm".to_string()],
            confirm_echo: vec![],
            passability: Passability::Machine,
            default_index: Some(1),
            action: None,
            human_reason: None,
            absence_cost: AbsenceCost::Recoverable,
            measured_on: MEASURED_ON.to_string(),
            origin: Origin::Added,
        };
        assert!(
            is_question_form(&planted.needles[0]),
            "이 표본이 질문형이 아니면 ⓑ의 서사(질문형만으로는 부족하다)가 성립하지 않는다"
        );
        assert!(
            widget_rule_violations(&planted).is_empty(),
            "이 표본이 위젯 규칙에 걸리면 ⓑ가 시험하는 축이 바뀐다"
        );
        let v = gate_rule_violations(&planted);
        assert!(
            v.iter().any(|m| m.contains("live-permission-prompt")),
            "질문형 needle 이 대조군(살아 있는 권한 프롬프트)에 걸리는데도 규칙이 침묵한다 — \
             P4-9 의 이빨이 없다: {v:?}"
        );

        // ⓒ 그리고 그 표본의 **오탐 경로가 프로덕션 해소에서 실제로 닫힌다**(규칙 → 집행 연결).
        //   ★P4-10 정정: 닫는 수단은 '관문 버리기' 가 아니라 **걸리는 needle 만 제거**다.
        //   버리면 사용자 선언이 조용히 사라지고(성질 ①) 그 관문의 귀결이 주입으로 뒤집힌다(성질 ②).
        //   제거되는 것은 **정상 화면에 걸리는 문면**뿐이라 위험 방향으로 열리지 않는다.
        let env = json!({"gates": [
            {"id": "planted-permission", "needles": ["Do you want to proceed?"],
             "widget": ["Enter to confirm"]}
        ]});
        let r = resolve_with(Some(&env), true);
        assert!(
            r.gates.iter().any(|g| g.id == "planted-permission"),
            "사용자 선언이 통째로 사라졌다 — 규칙 집행이 사용자 주권을 침해한다(P4-10 ①)"
        );
        assert_eq!(
            identify(&r.gates, fixtures::LIVE_PERMISSION_PROMPT),
            None,
            "규칙은 아는데 집행이 안 된다 — 살아 있는 권한 프롬프트가 관문으로 잡힌다(P4-3 재발)"
        );
        assert!(
            r.notes.iter().any(|n| n.contains("planted-permission")),
            "무엇을 왜 고쳤는지가 조용하다: {:?}",
            r.notes
        );
    }

    /// ★리뷰어 권고 채택(2026-08-24) — **새 관문을 들이면 대조군도 들여야 한다.**
    ///
    /// 종전 마찰은 헬스 러너의 `need(len(blocks) == 6)` 하나뿐이라, 손으로 `6 → 7` 만 고치면
    /// **대조군 0으로 통과**했다(리뷰어 표현: "지금 이빨은 자라지 않는다").
    ///
    /// 이 표의 계약은 단순한 존재 요구가 아니다 — 커버리지 화면은 그 관문의 **위젯 서명을
    /// 전부 만족**해야 한다. 그래야 "그 화면에서 관문이 안 잡히는 이유가 오직 needle 축" 이라는
    /// 사실이 실행으로 증명되고, 아무 화면이나 갖다 붙이는 형식적 통과가 막힌다.
    #[test]
    fn every_gate_has_a_control_screen_that_satisfies_its_widget_signature() {
        let gs = builtin();
        let hit = |s: &String, screen: &str| {
            normalize(screen).contains(&normalize(s)) || flatten(screen).contains(&flatten(s))
        };
        for g in &gs {
            let rows: Vec<&(&str, &str)> = GATE_CONTROL_COVERAGE
                .iter()
                .filter(|(gid, _)| *gid == g.id)
                .collect();
            assert!(
                !rows.is_empty(),
                "{}: 대조군 커버리지가 0건이다 — 새 관문을 들일 때 대조군을 함께 들이라는 규율이 \
                 무력화됐다(관문 id 마다 최소 1개)",
                g.id
            );
            for (_, sid) in rows {
                let screen = fixtures::NON_GATE_SCREENS
                    .iter()
                    .find(|(id, _)| id == sid)
                    .map(|(_, s)| *s)
                    .unwrap_or_else(|| {
                        panic!("커버리지 표가 존재하지 않는 대조군 {sid} 를 가리킨다")
                    });
                for w in &g.widget {
                    assert!(
                        hit(w, screen),
                        "{}: 대조군 {sid} 가 위젯 {w:?} 을 담지 않는다 — 위젯 AND 가 애초에 \
                         불만족이라 needle 축이 시험되지 않는 형식적 커버리지다",
                        g.id
                    );
                }
                assert!(
                    !g.matches(screen),
                    "{}: 대조군 {sid} 를 관문으로 식별했다 — 위젯 서명이 전부 만족된 화면에서 \
                     needle 축이 일하지 않는다",
                    g.id
                );
            }
        }
        // 표가 정본에 없는 관문 id 를 가리키면 적색(쓰레기통 금지 — 면제표와 같은 규율).
        for (gid, sid) in GATE_CONTROL_COVERAGE {
            assert!(
                gs.iter().any(|g| g.id == *gid),
                "커버리지 표 항목 ({gid}, {sid}) 이 정본에 없는 관문을 가리킨다"
            );
        }
    }

    /// ★P4-8 — **needle 이 본문으로 실린 화면은 코퍼스 계층에서 닫을 수 없다**(원리 증명).
    ///
    /// 이 검체는 통과를 위해 대조군을 순화하지 않는다. 대신 ⓐ왜 원리상 불가능한지를 소스
    /// 자신으로 증명하고, ⓑ그래서 실제로 잡힌다는 **잔여 위험을 기계로 박제**하며,
    /// ⓒ그것을 닫는 것이 코퍼스가 아니라 **생애 창**이라는 사실을 실행으로 보인다.
    #[test]
    fn body_text_screens_are_unclosable_at_the_corpus_layer() {
        let gs = builtin();

        // ⓐ 자기참조 증명 — 정본 소스는 **모든 needle 과 위젯을 글자 그대로** 담고 있다.
        //    따라서 그 소스를 출력한 화면은 정의상 전량을 포함하고, `NON_GATE_SCREENS` 의
        //    계약("어떤 needle 도 걸리지 않는다")은 그 화면에 대해 **만족 불가능**이다.
        //    needle 을 바꿔도 다음 판의 소스가 그 새 needle 을 다시 담는다.
        let sot = include_str!("first_run_gates.rs");
        for g in &gs {
            for t in g.needles.iter().chain(g.widget.iter()) {
                assert!(
                    sot.contains(t.as_str()),
                    "{}: 정본 소스가 선언 문면 {t:?} 을 담지 않는다 — 자기참조 논거가 무효다",
                    g.id
                );
            }
        }

        // ⓑ 그래서 실제로 잡힌다 — 잔여 위험을 주석이 아니라 검체로 남긴다.
        for &(sid, screen) in fixtures::BODY_TEXT_SCREENS {
            assert!(
                identify(&gs, screen).is_some(),
                "{sid}: 이 화면이 안 잡힌다면 잔여 위험 서사가 틀린 것이다 — doc 을 고쳐라"
            );
        }
        assert_eq!(
            identify(&gs, fixtures::CAT_GATE_CORPUS_SOURCE).map(|g| g.id.clone()),
            Some("theme".to_string()),
            "정본 소스 열람 화면이 theme 로 잡히지 않는다 — 리뷰어 실측과 어긋난다"
        );

        // ⓒ 이것을 닫는 것은 **생애 창**이다(U-14 주입 가드의 각성 래치).
        for &(sid, screen) in fixtures::BODY_TEXT_SCREENS {
            let o = |awakened| crate::inject_guard::Observed {
                screen,
                gates: &gs,
                awakened,
                guard_off: false,
            };
            // 부트 창 안(첫 각성 ack 이전)에서는 막는다 — 그 자리에서는 그것이 옳다.
            assert!(
                crate::inject_guard::decide(&o(Some(false))).blocks(),
                "{sid}: 부트 창에서 관문 축이 침묵했다"
            );
            // 각성 이후·미관측에서는 창이 닫혀 통과한다 — 작업 중 노드가 자기 화면 때문에
            // 영구 차단되지 않는다.
            for awakened in [Some(true), None] {
                assert!(
                    !crate::inject_guard::decide(&o(awakened)).blocks(),
                    "{sid}: 각성 이후에도 주입이 막힌다 — 감사 문서·소스 열람이 그 노드를 영구 \
                     차단한다(U-14 치명위험 ①)"
                );
            }
        }

        // ★대조 — `NON_GATE_SCREENS` 쪽(needle 부재)은 코퍼스 계층에서 **닫힌다**.
        //   즉 '닫을 수 있는 것은 닫았고, 닫을 수 없는 것만 창에 맡겼다'.
        for &(sid, screen) in fixtures::NON_GATE_SCREENS {
            assert_eq!(identify(&gs, screen), None, "{sid}");
        }
    }

    /// 계측 타당성: **구 코드의 needle 은 실제로 그 에코에 걸린다.** 이 대조군이 없으면 위
    /// 불변식이 "원래 안 걸리는 것을 안 걸린다고 확인"하는 공허한 검사일 수 있다.
    #[test]
    fn legacy_needle_would_have_matched_the_echo() {
        let echo = "Yes, I trust this folder ✔";
        assert!(
            flatten(echo).contains("trustthisfolder"),
            "구 needle 이 에코에 안 걸리면 킬체인 서사가 틀린 것 — 코퍼스 근거를 재확인하라"
        );
        // 그리고 이 코퍼스는 그 needle 을 갖지 않는다.
        assert!(
            !builtin()
                .iter()
                .any(|g| g.needles.iter().any(|n| flatten(n) == "trustthisfolder")),
            "정본이 결함 needle 을 다시 들여왔다"
        );
    }

    // ── 실측 화면 식별 ─────────────────────────────────────────────────────

    #[test]
    fn measured_screens_identify_to_their_gate() {
        let gs = builtin();
        let id_of = |s: &str| identify(&gs, s).map(|g| g.id.clone());
        assert_eq!(id_of(fixtures::THEME).as_deref(), Some("theme"));
        assert_eq!(id_of(fixtures::LOGIN_METHOD).as_deref(), Some("login-method"));
        assert_eq!(id_of(fixtures::OAUTH_CODE).as_deref(), Some("oauth-code"));
        assert_eq!(id_of(fixtures::FOLDER_TRUST).as_deref(), Some("folder-trust"));
        assert_eq!(
            id_of(fixtures::FEATURE_FULLSCREEN).as_deref(),
            Some("feature-announce-fullscreen")
        );
        assert_eq!(id_of(fixtures::READY_SHELL), None, "정상 화면 오탐");
    }

    /// ★킬체인 화면: 신뢰 에코가 남아 있어도 **면책 창**으로 읽혀야 한다.
    /// 여기서 folder-trust 로 읽히면 "이미 확인했다"는 오판이 나고, 그 다음 Return 이
    /// `No, exit`(기본 포커스)를 눌러 좌석을 죽인다 — 실사고의 정확한 경로다.
    #[test]
    fn trust_echo_followed_by_disclaimer_reads_as_disclaimer() {
        let gs = builtin();
        let g = identify(&gs, fixtures::TRUST_ECHO_THEN_DISCLAIMER).expect("관문 미식별");
        assert_eq!(g.id, "bypass-disclaimer", "확인 에코가 관문 근거로 쓰였다");
        assert_eq!(g.default_index, Some(1), "면책 기본 포커스 = No, exit(실측)");
        assert_eq!(g.down_presses(), Some(1), "면책 통과 = 아래 1회 + Return");
        assert_eq!(
            g.action.as_ref().unwrap().literal.as_deref(),
            Some("2"),
            "리터럴 등가 입력 선언 소실"
        );
    }

    #[test]
    fn folder_trust_return_is_safe_and_disclaimer_return_is_not() {
        let gs = builtin();
        let trust = gs.iter().find(|g| g.id == "folder-trust").unwrap();
        let disc = gs.iter().find(|g| g.id == "bypass-disclaimer").unwrap();
        // 폴더신뢰: 기본 포커스가 곧 목표 → 아래키 0회(= Return 만으로 안전 통과).
        assert_eq!(trust.down_presses(), Some(0));
        // 면책: 기본 포커스가 목표가 아니다 → Return 만 누르면 rc 1.
        assert_ne!(disc.down_presses(), Some(0), "면책 Return 안전 오판(치명)");
    }

    // ── 버전 핀 ────────────────────────────────────────────────────────────

    #[test]
    fn version_pin_holds_actions_on_drift_and_on_unknown() {
        let gs = builtin();
        let trust = gs.iter().find(|g| g.id == "folder-trust").unwrap();
        assert!(action_policy(trust, Some(MEASURED_ON)).is_allowed());
        assert!(
            matches!(
                action_policy(trust, Some("2.2.0")),
                ActionPolicy::HeldVersionDrift { .. }
            ),
            "벤더 버전이 바뀌었는데 액션이 집행된다 — 관문 증식에 무방비"
        );
        assert!(
            matches!(
                action_policy(trust, None),
                ActionPolicy::HeldVersionUnknown { .. }
            ),
            "측정 불능이 통과로 접혔다"
        );
    }

    #[test]
    fn human_only_gates_never_yield_an_action() {
        let gs = builtin();
        for id in ["login-method", "oauth-code"] {
            let g = gs.iter().find(|g| g.id == id).unwrap();
            for v in [Some(MEASURED_ON), Some("9.9.9"), None] {
                assert!(
                    matches!(action_policy(g, v), ActionPolicy::HumanRequired { .. }),
                    "{id}: 사람 1회 관문에 기계 액션이 났다"
                );
            }
        }
    }

    #[test]
    fn version_parser_reads_measured_banners() {
        assert_eq!(
            parse_cli_version(fixtures::THEME).as_deref(),
            Some("2.1.241"),
            "실측 배너에서 버전을 못 읽는다"
        );
        assert_eq!(
            parse_cli_version("2.1.241 (Claude Code)").as_deref(),
            Some("2.1.241")
        );
        assert_eq!(parse_cli_version("Claude Code v3.0 ready").as_deref(), Some("3.0"));
        // 못 읽으면 추정하지 않는다.
        assert_eq!(parse_cli_version(fixtures::FOLDER_TRUST), None);
        assert_eq!(parse_cli_version("1. Auto (match terminal)"), None);
    }

    // ── override 봉투 ──────────────────────────────────────────────────────

    #[test]
    fn absent_or_malformed_envelope_keeps_the_builtin_corpus() {
        for env in [None, Some(&Value::Null), Some(&json!("nope")), Some(&json!([]))] {
            let r = resolve_with(env, true);
            assert_eq!(r.gates.len(), 6, "봉투 이상이 코퍼스를 지웠다");
        }
    }

    #[test]
    fn override_patches_by_id_and_can_add_new_gates() {
        let env = json!({
            "source": "builtin",
            "measured_on": "9.9.9",
            "gates": [
                {"id": "theme", "needles": ["Pick your colours"], "measured_on": "9.9.9"},
                {"id": "vendor-new-2027", "title": "새 관문",
                 "needles": ["Enable telemetry?"], "widget": ["Enter to confirm"],
                 "default_index": 1, "action": {"select_index": 2, "label": "No", "literal": "2"}}
            ]
        });
        let r = resolve_with(Some(&env), true);
        assert!(matches!(r.source, Source::Merged { overridden: 1, added: 1 }));
        assert_eq!(r.gates.len(), 7);
        let theme = r.gates.iter().find(|g| g.id == "theme").unwrap();
        assert_eq!(theme.needles, vec!["Pick your colours".to_string()]);
        assert_eq!(theme.origin, Origin::Overridden);
        // 버전이 갈리면 액션은 보류된다(같은 코퍼스 안에서도 관문별로 판정한다).
        assert!(matches!(
            action_policy(theme, Some(MEASURED_ON)),
            ActionPolicy::HeldVersionDrift { .. }
        ));
        let neu = r.gates.iter().find(|g| g.id == "vendor-new-2027").unwrap();
        assert_eq!(neu.origin, Origin::Added);
        assert!(neu.matches("Enable telemetry?\nEnter to confirm · Esc to cancel"));
    }

    #[test]
    fn override_cannot_promote_a_human_only_gate_to_machine() {
        let env = json!({"gates": [
            {"id": "login-method", "passability": "machine",
             "action": {"select_index": 1, "label": "just press enter"}}
        ]});
        let r = resolve_with(Some(&env), true);
        let g = r.gates.iter().find(|g| g.id == "login-method").unwrap();
        assert_eq!(g.passability, Passability::HumanOnly, "측정 결과가 선언으로 뒤집혔다");
        assert!(g.action.is_none());
        assert!(r.notes.iter().any(|n| n.contains("거부")), "거부가 조용하다");
    }

    #[test]
    fn override_may_tighten_a_machine_gate_to_human_only() {
        let env = json!({"gates": [{"id": "theme", "passability": "human_only",
                                    "human_reason": "우리 조직은 사람이 고른다"}]});
        let r = resolve_with(Some(&env), true);
        let g = r.gates.iter().find(|g| g.id == "theme").unwrap();
        assert_eq!(g.passability, Passability::HumanOnly);
        assert!(matches!(
            action_policy(g, Some(MEASURED_ON)),
            ActionPolicy::HumanRequired { .. }
        ));
    }

    /// 빌트인 중 **부재의 비용이 비가역**인 관문 id — 코드 정본에서 파생한다(손으로 세지 않는다).
    fn fatal_builtin_ids() -> Vec<String> {
        builtin()
            .into_iter()
            .filter(|g| g.absence_is_fatal())
            .map(|g| g.id)
            .collect()
    }

    /// ★정본 = "선언 1건 + **강제 복원된 Fatal 빌트인 5종**"(N1 정정 · 2026-08-24).
    ///
    /// 【무엇을 고쳐놓았는가】 종전 이 검체는 `assert_eq!(r.gates.len(), 1)` 로
    /// **선언 1건이 곧 코퍼스 전량**임을 박제했다. 그것은 완화가 아니라 **구멍을 초록으로
    /// 고정한 것**이었다 — `source=replace` 한 줄이 Fatal 빌트인을 통째로 없앴고,
    /// 그중 면책 창의 부재는 **주입 Return 한 발 = rc 1 좌석 사망**이다(모듈 doc 비용표).
    /// 지금은 그 5종이 살아남는 것이 정본이고, 사용자 선언은 **그대로 함께** 산다(주권 침해 0).
    #[test]
    fn replace_mode_takes_the_declared_corpus_but_never_an_empty_one() {
        // ★(P4-10) 자기규칙 위반 선언(위젯 AND 가드 0)이라도 사용자 선언은 살아남는다 —
        //   아래 두 번째 블록이 그 유지를 같은 자리에서 대조한다.
        let fatal = fatal_builtin_ids();
        let env = json!({"source": "replace", "gates": [
            {"id": "only", "needles": ["Do you want to continue?"],
             "widget": ["Enter to confirm"]}
        ]});
        let r = resolve_with(Some(&env), true);
        // 출처 회계는 **선언 수**를 말한다(복원된 바닥은 사용자 선언이 아니다).
        assert!(matches!(r.source, Source::Replaced { count: 1 }));
        assert_eq!(
            r.gates.len(),
            1 + fatal.len(),
            "정본은 '선언 1건 + 복원된 Fatal {}종' 이다: {:?}",
            fatal.len(),
            r.gates.iter().map(|g| g.id.as_str()).collect::<Vec<_>>()
        );
        assert!(r.gates.iter().any(|g| g.id == "only"), "사용자 선언이 사라졌다");
        for id in &fatal {
            assert!(
                r.gates.iter().any(|g| &g.id == id),
                "replace 선언 한 줄이 Fatal 관문 {id} 을 없앴다(면책 창 상실 = Return 한 발 rc 1)"
            );
        }

        // ★P4-10 정정 — 자기규칙 위반 선언(위젯 AND 가드 0)이라도 **사용자 선언은 살아남는다**.
        //   종전 판은 이것을 버리고 "비면 정본으로 되돌린다" 폴백으로 벤더 6종을 세웠다. 그것이
        //   사용자 주권 침해였다(같은 형태의 프로덕션 핀: `cys.rs h_deliver_1…` 의 ⑥).
        //   복원할 정본 서명이 없는 신설 관문이므로 선언을 그대로 유지하고 사유만 남긴다.
        let nowidget = json!({"source": "replace", "gates": [
            {"id": "only", "needles": ["Do you want to continue?"]}
        ]});
        let r = resolve_with(Some(&nowidget), true);
        assert_eq!(
            r.gates.len(),
            1 + fatal.len(),
            "사용자 선언이 덮였거나 Fatal 바닥이 사라졌다: {:?}",
            r.gates.iter().map(|g| g.id.as_str()).collect::<Vec<_>>()
        );
        assert_eq!(r.gates[0].id, "only", "사용자 선언이 코드 정본에 덮였다(사용자 주권 침해)");
        assert!(
            r.notes.iter().any(|n| n.contains("유지") && n.contains("only")),
            "유지 사유가 조용하다: {:?}",
            r.notes
        );
        // 그리고 그 선언이 정상 화면을 잡지는 않는다(유지가 오탐 면허가 아니다).
        for &(sid, screen) in fixtures::NON_GATE_SCREENS {
            assert_eq!(identify(&r.gates, screen), None, "{sid} 오탐");
        }

        // ★빈 코퍼스는 '관문 없음'이 아니라 '눈을 감음' — 코드 정본으로 되돌린다.
        let blind = json!({"source": "replace", "gates": []});
        let r = resolve_with(Some(&blind), true);
        assert_eq!(r.gates.len(), 6, "빈 코퍼스를 그대로 받으면 허위 ready 가 열린다");
        assert!(r.notes.iter().any(|n| n.contains("맹목")));
    }

    /// ★N1 — `source=replace` 는 **Fatal 관문을 없앨 권한이 없다**(강제 복원 바닥).
    ///
    /// 【종전 배선의 실체】 [`enforce_absence_cost`] 는 `pre`(해소 직후 코퍼스)와 `kept` 를
    /// 대조하는데, replace 모드에서 `pre` 는 **사용자 목록**이라 빌트인 Fatal 관문이 순회
    /// 대상에 **애초에 없었다** — 되살리는 코드가 한 줄도 실행되지 않았다. 발동 조건이
    /// "사용자가 `source=replace` 를 명시 선언"이라 기본 경로는 아니지만, 귀결은 면책 창
    /// 보호 상실 = **주입 Return 한 발이 rc 1**(좌석 사망)이라 재난 ④ 축이다.
    #[test]
    fn replace_mode_cannot_drop_a_fatal_builtin_gate() {
        let canon = builtin();
        let fatal = fatal_builtin_ids();
        assert!(
            fatal.len() >= 5,
            "Fatal 관문이 5종 미만 — 비용표가 바뀌었다면 이 검체부터 다시 세워라: {fatal:?}"
        );

        let env = json!({"source": "replace", "gates": [
            {"id": "only", "needles": ["Do you want to continue?"],
             "widget": ["Enter to confirm"]}
        ]});
        let r = resolve_with(Some(&env), true);
        for id in &fatal {
            let g = r
                .gates
                .iter()
                .find(|g| &g.id == id)
                .unwrap_or_else(|| panic!("Fatal 관문 {id} 이 replace 선언 한 줄로 사라졌다"));
            let b = canon.iter().find(|b| &b.id == id).expect("코드 정본");
            assert_eq!(g, b, "{id}: 복원본이 코드 정본과 다르다(반쪽 복원은 복원이 아니다)");
        }
        // 면책 창은 이 코퍼스에서 **부재가 가장 비싼** 관문이다 — 실측 사실이 그대로 살아야 한다.
        let disc = r
            .gates
            .iter()
            .find(|g| g.id == "bypass-disclaimer")
            .expect("면책 관문");
        assert_eq!(disc.absence_cost, AbsenceCost::Fatal);
        assert_eq!(
            disc.default_index,
            Some(1),
            "기본 포커스가 `No, exit` 라는 실측이 소실됐다"
        );
        assert_eq!(disc.action.as_ref().map(|a| a.select_index), Some(2));
        // 복원은 조용하지 않다 — 집행이 사용자 코퍼스를 바꿨다는 사실은 반드시 남는다.
        assert!(
            r.notes
                .iter()
                .any(|n| n.contains("bypass-disclaimer") && n.contains("복원")),
            "복원이 조용하다: {:?}",
            r.notes
        );
        // 사용자 선언도 함께 산다(복원이 주권 침해로 뒤집히지 않는다).
        assert!(r.gates.iter().any(|g| g.id == "only"));

        // ★선언이 Fatal 빌트인의 id 를 **가로채도** 부재의 비용은 실측 사실이라 낮아지지 않는다
        //   (merge 경로의 `apply_patch` 가 이미 같은 완화를 거부한다 — 두 경로의 대칭).
        let hijack = json!({"source": "replace", "gates": [
            {"id": "bypass-disclaimer", "needles": ["Do you want to continue?"],
             "widget": ["Enter to confirm"], "absence_cost": "recoverable"}
        ]});
        let r = resolve_with(Some(&hijack), true);
        let g = r
            .gates
            .iter()
            .find(|g| g.id == "bypass-disclaimer")
            .expect("면책 관문");
        assert_eq!(
            g.absence_cost,
            AbsenceCost::Fatal,
            "봉투 한 줄로 킬체인 관문의 부재 비용이 낮아졌다(replace 가 merge 보다 헐거워졌다)"
        );
        for id in fatal.iter().filter(|i| i.as_str() != "bypass-disclaimer") {
            assert!(r.gates.iter().any(|g| &g.id == id), "{id} 소실");
        }
    }

    #[test]
    fn rollback_switch_is_one_pure_predicate() {
        assert!(!override_enabled_from(Some("0")));
        assert!(!override_enabled_from(Some(" OFF ")));
        assert!(!override_enabled_from(Some("false")));
        assert!(override_enabled_from(None), "기본은 override 파싱 활성");
        assert!(override_enabled_from(Some("1")));
        // 스위치가 꺼지면 봉투가 무엇이든 코드 정본이다.
        let env = json!({"source": "replace", "gates": [{"id": "x", "needles": ["y"]}]});
        let r = resolve_with(Some(&env), false);
        assert_eq!(r.source, Source::OverrideDisabled);
        assert_eq!(r.gates.len(), 6);
    }

    // ── S-1 사본 정합(Rust 측 절반) ────────────────────────────────────────

    /// 임베드 `agents.json` 의 `trust-prompt` 선언 문면이 코퍼스 needle 에 실재하는지.
    /// (S-1: 관문 문면 사본 4벌 중 agents.json 벌 — SOT 는 이 파일이고 저쪽은 읽기 소비다.)
    #[test]
    fn agents_json_trust_pattern_is_covered_by_the_corpus() {
        let embedded: Value = crate::pack::PACK_ALL
            .iter()
            .find(|(r, _)| *r == "agents.json")
            .map(|(_, c)| serde_json::from_str(c).expect("임베드 agents.json 파싱"))
            .expect("임베드에 agents.json 존재");
        let pat = embedded["claude"]["approval_patterns"]
            .as_array()
            .expect("approval_patterns 배열")
            .iter()
            .find(|p| p["name"] == json!("trust-prompt"))
            .and_then(|p| p["pattern"].as_str())
            .expect("trust-prompt 선언");
        let gs = builtin();
        let trust = gs.iter().find(|g| g.id == "folder-trust").unwrap();
        assert!(
            trust.needles.iter().any(|n| n == pat),
            "agents.json trust-prompt 문면 {pat:?} 이 코퍼스에 없다 — 사본이 갈렸다"
        );
    }

    /// 봉투가 임베드 `agents.json` 에 실재하고, 코퍼스를 **복사해 두지 않았음**을 못박는다.
    /// (사본이 늘면 S-1 샷건 서저리가 그대로 재발한다.)
    #[test]
    fn embedded_envelope_is_override_only_and_not_a_second_copy() {
        let embedded: Value = crate::pack::PACK_ALL
            .iter()
            .find(|(r, _)| *r == "agents.json")
            .map(|(_, c)| serde_json::from_str(c).expect("임베드 agents.json 파싱"))
            .expect("임베드에 agents.json 존재");
        let env = &embedded["claude"][ADAPTER_KEY];
        assert!(env.is_object(), "임베드 어댑터에 {ADAPTER_KEY} 봉투가 없다 — 배달 경로 미배선");
        assert_eq!(env["source"].as_str(), Some("builtin"));
        assert_eq!(env["measured_on"].as_str(), Some(MEASURED_ON), "봉투 버전 핀 드리프트");
        assert_eq!(
            env["gates"].as_array().map(|a| a.len()),
            Some(0),
            "봉투가 코퍼스 사본을 들고 있다 — 정본은 코드 하나여야 한다"
        );
        // 그리고 그 봉투를 먹인 결과는 코드 정본 그대로여야 한다.
        let r = resolve_with(Some(env), true);
        assert_eq!(r.gates, builtin());
        assert_eq!(r.source, Source::Builtin);
    }
}
