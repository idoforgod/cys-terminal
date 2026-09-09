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
    /// 그 항목의 라벨. ★(0.14.31 · 리뷰 R3·R4) 종전엔 "사람 확인용 · 판정 근거 아님" 이었으나, 지금은
    /// **자동확인의 양성 증거**다 — `inject_guard::confirm_allowed`(관문 확인 허가)와
    /// `decide_allowing`(주입 허가의 allow 구멍)이 둘 다 커서가 이 라벨 전문 위에 있고 **활성 선택 블록에
    /// 경쟁 커서가 없을 때만**(`readiness::cursor_resolves_to_label`) 그 관문의 Return 을 허용한다. 그러므로
    /// 이 값은 화면 실측 문면과 **글자 그대로** 같아야 하고, 틀리면 구멍이 닫히는 쪽(보류)으로 틀린다.
    /// 그 술어의 블록 경계는 이 관문의 [`Gate::needles`] 가 정한다(질문 문면이 SOT · 사본 0).
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
    /// 실측 기본 포커스 — Return 만 눌렀을 때 선택되는 항목. 좌표계는 [`GateAction::select_index`]
    /// 와 **같다**(화면에 보이는 번호). 벤더가 0 부터 세는 메뉴를 그리면 `0` 도 정당한 값이다
    /// (정본 §4 H-2: 2.1.261 폴더신뢰는 `0="No, exit"` · `1="Yes, I trust this folder"`).
    /// `None` = **미측정** → [`Gate::down_presses`] 가 `None` 을 내어 액션이 보류된다(fail-closed).
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
        // ★(0.14.31 · 성찰 R3 · blocking) **빈 문면은 아무것도 식별하지 않는다.** `normalize`/
        //   `flatten` 은 공백뿐인 문자열을 빈 문자열로 만들고, `String::contains("")` 는 **모든**
        //   화면에 참이다. 그래서 needle `"   "` 한 줄이 이 관문을 상시 참으로 만들었다 —
        //   그 귀결은 ① `judge` 가 전 좌석·전 틱 `GateHeld`(디렉티브 영구 미주입 = 노드 0 +
        //   고아 좌석) ② `inject_guard::decide` 가 항상 `Hold` ③ `CYS_GATE_PENDING_CLOSE=1`
        //   기계에서 `boot_verdict_effective` 가 `LaunchFailed` 로 강등 = **모든 pane 사망**이다
        //   (§7 부트체인 재난표). 파서([`str_vec`])가 이미 거르지만 판정부에도 벨트를 둔다 —
        //   봉투를 거치지 않고 조립된 `Gate` 도 이 함수를 지난다.
        let hit = |s: &String| {
            let (n, f) = (normalize(s), flatten(s));
            (!n.is_empty() && norm.contains(&n)) || (!f.is_empty() && flat.contains(&f))
        };
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
        // ★(0.14.31 · 성찰 R3 · blocking) 종전에는 여기서 **면제**(빈 벡터 = 위반 없음)했다.
        //   그런데 공백뿐인 needle 은 `contains("")` 로 **모든 화면**에 걸리는 문면이다 — 면제는
        //   정확히 거꾸로였고, 그래서 `gate_rule_violations` 가 아무 말도 하지 않은 채
        //   `repair_gate` 가 그 항목을 지나쳤다(그리고 `notes` 는 "needle 축은 정상 화면 대조를
        //   이미 통과했다" 는 **정반대**를 찍었다). 전량을 돌려주면 사용자 신설 관문에서는 그
        //   needle 만 제거되고 사유가 남으며(조용한 무력화 0), 빌트인 대응물이 있으면 정본
        //   needle 로 복원된다 — 어느 쪽도 관문을 잃지 않는다(P4-10 보존 계약 무변).
        return fixtures::NON_GATE_SCREENS.iter().map(|&(sid, _)| sid).collect();
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

/// ★[`action_policy`](버전 핀) 판정이 **키 경로에 얼마나 배선돼 있는가** — 세 상태.
///
/// 【왜 bool 이 아닌가 — 0.14.31 · 독립 재유도 H2-B】 종전에는 `ACTION_POLICY_IS_ENFORCED: bool`
/// 하나였고 값은 `false`("어느 지점에도 배선돼 있지 않다")였다. 그런데 확인 경계가
/// **불일치 팔 하나**를 집행하기 시작하면 `false` 도 `true` 도 거짓말이 된다 — `false` 는 실제로
/// 서는 벨트를 없다고 말하고, `true` 는 여전히 통과하는 **미상**과 배선 0 인 `Allowed{down}`
/// 다발 전송을 집행한다고 말한다. 부분 집행을 bool 로 접으면 어느 쪽으로든 산출물이 거짓이 된다
/// (codex 설계 검토 4). 그래서 **상태**로 싣는다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyEnforcement {
    /// 배선 0 — `policy` 는 순수 진단이다.
    Unwired,
    /// ★오늘. [`ActionPolicy::HeldVersionDrift`] **하나만** 확인 경계
    /// ([`crate::inject_guard::confirm_denied`])가 집행한다:
    ///   · **불일치** → 그 관문의 자동확인 보류(키 0) ·
    ///   · **미상**([`ActionPolicy::HeldVersionUnknown`]) → 여전히 통과한다(가용성 · 별도 결정) ·
    ///   · [`ActionPolicy::Allowed`] 의 `down` 다발 전송 → 배선 0(이 조립은 Return 만 보낸다).
    VersionDriftOnly,
    /// 판정 전량이 집행된다(오늘 아니다).
    Full,
}

impl PolicyEnforcement {
    /// 산출물 어휘(하류가 문자열로 읽는다).
    pub fn as_str(self) -> &'static str {
        match self {
            PolicyEnforcement::Unwired => "unwired",
            PolicyEnforcement::VersionDriftOnly => "version_drift_only",
            PolicyEnforcement::Full => "full",
        }
    }
    /// 버전 **불일치**가 키를 막는가.
    pub fn denies_version_drift(self) -> bool {
        matches!(
            self,
            PolicyEnforcement::VersionDriftOnly | PolicyEnforcement::Full
        )
    }
    /// 버전 **미상**이 키를 막는가(오늘 아니다).
    pub fn denies_version_unknown(self) -> bool {
        matches!(self, PolicyEnforcement::Full)
    }
    /// `Allowed{down}` 의 다발 전송이 집행되는가(오늘 아니다).
    pub fn sends_down_bundle(self) -> bool {
        matches!(self, PolicyEnforcement::Full)
    }
}

/// ★지금의 집행 상태. [`report_json`] 이 `policy_enforcement` 로 싣고, 소스 핀
/// (`cys.rs` `action_policy_version_axis_is_wired_only_as_drift_denial_source_pin`)이 이 값과
/// 실제 배선이 어긋나지 않는지 못박는다(상수만 움직이는 거짓 안심이 구조적으로 불가능하다).
///
/// 【무엇이 배선됐고 무엇이 아닌가 — 실측 2026-09-08】 확인 경계가 보는 것은 ⓪ 코드 정본 사람 1회
/// 관문이 화면에 서지 않을 것 ① 코퍼스가 그 id 로 식별할 것 ② 커서가 종료 위가 아닐 것
/// ③ 선언 시퀀스가 Return 한 발일 것(`down_presses()==Some(0)`) ④ 커서가 액션 라벨 전문 위일 것
/// ⑤ **좌석이 밝힌 버전이 이 관문의 실측본과 다르지 않을 것**(이번에 더한 축) — 다섯이다.
/// ⑤의 증거는 좌석 기동에 결속된 래치([`crate::inject_guard::Observed::cli_version`])와 지금
/// 화면의 배너([`banner_versions`])의 **합집합**이고, 그중 하나라도 불일치면 보류다.
/// **미상은 아직 통과한다** — `MEASURED_ON` 이 부분 실측이라(6관문 중 2관문) 미상까지 접으면
/// 오늘 전 좌석이 매 부트마다 사람 1회를 요구한다(정본 §3-3 은 보류를 허용하지만 그 절단은
/// 별도 결정이다). 그 결정이 서면 이 상수가 [`PolicyEnforcement::Full`] 로 간다.
pub const ACTION_POLICY_ENFORCEMENT: PolicyEnforcement = PolicyEnforcement::VersionDriftOnly;

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
///
/// ★두 번째 갈래(앵커 없는 선두 점숫자)는 **`--version` stdout 을 읽을 때의 형태**다. 화면
///   (vt100 그리드)에 그것을 걸면 "첫 글자가 점 있는 숫자면 그게 도는 버전" 이 되어, 벤더가 무엇을
///   그리든 근거가 된다. 그래서 **화면을 재료로 쓰는 자리**는 이 함수가 아니라 [`banner_version`]
///   을 쓴다(0.14.31 · 독립 재유도 H2-B).
pub fn parse_cli_version(text: &str) -> Option<String> {
    banner_version(text).or_else(|| take_dotted(text.trim_start()))
}

/// ★**배너 앵커가 있을 때만** 버전을 뽑는다 — 화면을 재료로 쓰는 자리의 판독기
/// (0.14.31 · 독립 재유도 H2-B).
///
/// 【왜 나누는가】 [`parse_cli_version`] 의 폴백은 앵커가 없으면 텍스트 **선두**에서 점 있는
/// 숫자를 집는다. `claude --version` stdout(`2.1.241 (Claude Code)`)에는 옳지만 화면에는 틀리다 —
/// 관문 화면의 첫 줄이 우연히 `1.5x …` 로 시작하면 그것이 "지금 도는 버전" 이 된다. 확인 경계
/// ([`crate::inject_guard::confirm_denied`])는 좌석이 **스스로 찍은 배너**만 증거로 인정한다.
///
/// 【실패 방향】 못 찾으면 `None` = "이 화면은 버전을 밝히지 않았다" 이고, 그 귀결은 오늘 보류가
/// **아니다**(미상은 아직 통과한다 — [`ACTION_POLICY_ENFORCEMENT`] doc 의 버전 축 표 참조).
/// 앵커를 좁혀서 잘못 놓치는 귀결은 "종전과 같음"이지 새로 열리는 구멍이 아니다.
pub fn banner_version(text: &str) -> Option<String> {
    banner_versions(text).into_iter().next()
}

/// 화면이 밝힌 배너 버전 **전량**(중복 제거 · 최대 [`BANNER_SCAN_MAX`]개).
///
/// 【왜 첫 배너 하나로는 부족한가】 화면은 잔존 출력이 섞인 그리드다. 앞쪽에 옛 배너(또는
/// 에이전트가 출력한 문자열)가 있고 뒤쪽에 지금 좌석의 배너가 있으면, "첫 앵커 하나" 규칙은
/// **앞쪽만 보고** 뒤쪽의 불일치를 놓친다(codex 설계 검토 3). 확인 경계는 이 목록을 전부 대조해
/// **하나라도 불일치면 보류**한다 — 증거가 갈리면 조이는 쪽으로 접는다.
pub fn banner_versions(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    scan_banners(text, BANNER_ANCHOR, &mut out);
    // ★(0.14.31 · 수렴 R2 · reviewer-claude minor) **접힌 배너**도 읽는다. 좁은 pane·ConPTY
    //   에서 배너는 `│ Welcome to Claude` / `│ Code v2.1.263 │` 로 접히고, 그러면 연속 앵커가
    //   깨져 이 축의 **유일한 증거**가 통째로 사라진다(그 귀결은 미상 = 통과). 그래서 공백과
    //   상자 테두리를 지운 사본을 한 번 더 훑는다.
    //   【실패 방향】 이 패스가 만들 수 있는 오탐(앵커 뒤에 무관한 점숫자가 붙는 형상)의 귀결은
    //   **보류**(사람 1회 · 가역)이고, 못 읽는 귀결은 미상 = 종전과 같음이다. 조이는 쪽으로만
    //   틀리는 패스다.
    if out.len() < BANNER_SCAN_MAX {
        let folded = fold_for_banner_scan(text);
        scan_banners(&folded, &flatten(BANNER_ANCHOR), &mut out);
    }
    out
}

/// 배너 앵커 문면(정본 1지점 — 접힌 배너 패스가 같은 문자열의 공백 제거본을 쓴다).
const BANNER_ANCHOR: &str = "Claude Code v";

/// 앵커 뒤의 점숫자를 훑어 `out` 에 **중복 없이** 넣는다(상한 [`BANNER_SCAN_MAX`]).
fn scan_banners(text: &str, anchor: &str, out: &mut Vec<String>) {
    let mut rest = text;
    while let Some(i) = rest.find(anchor) {
        let tail = &rest[i + anchor.len()..];
        if let Some(v) = take_dotted(tail) {
            if !out.contains(&v) {
                out.push(v);
                if out.len() >= BANNER_SCAN_MAX {
                    return;
                }
            }
        }
        rest = tail;
    }
}

/// 접힌 배너 판독용 사본 — 공백과 **상자 테두리**를 지운다.
///
/// 테두리까지 지우는 이유: [`flatten`] 만으로는 `│ Welcome to Claude\n│ Code v2.1.263` 이
/// `…Claude│Codev2.1.263` 이 되어 앵커가 여전히 깨진다(줄머리 테두리가 두 조각 사이에 남는다).
fn fold_for_banner_scan(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_whitespace() && !is_box_border(*c))
        .collect()
}

/// 상자 렌더의 **세로 테두리** 문자(가로줄·모서리는 배너 줄 안에 끼지 않는다).
fn is_box_border(c: char) -> bool {
    matches!(c, '│' | '┃' | '║' | '╎' | '┆' | '┊' | '╏' | '|' | '┇' | '┋')
}

/// 한 화면에서 훑는 배너 상한(병적 입력에서 판정 시간이 화면 길이에 끌려가지 않게).
pub const BANNER_SCAN_MAX: usize = 8;

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
    /// ★(0.14.31 · 리뷰 R1) 어댑터 스펙(`agents.json`)을 **읽지 못해서** 코드 정본으로 되돌아왔다.
    ///
    /// [`Builtin`](Source::Builtin) 과 **다른 사실**이다: `Builtin` 은 "덮을 봉투가 없다"(정상)이고
    /// 이것은 "봉투가 있었을지도 모르는데 도달하지 못했다"(고장)이다. 두 사실을 한 값으로 접으면
    /// 하류(preflight C82 등)가 "봉투가 도달했나" 를 알려면 한국어 산문 note 를 되파싱해야 하고,
    /// 되파싱은 다음 판에 반드시 깨진다. 이 변이는 [`resolve_raw`] 가 만들지 않는다 — 스펙을 읽는
    /// 것은 호출부(`cys.rs resolve_gate_corpus`)이고, 이 모듈은 이미 읽힌 봉투만 받는다.
    SpecUnreadable { reason: String },
}

/// 코퍼스 해소 결과. `notes` 는 사람용 진단이며 **판정 재료가 아니다**(호출부가 원할 때 표출).
#[derive(Debug, Clone)]
pub struct Resolved {
    pub gates: Vec<Gate>,
    pub notes: Vec<String>,
    pub source: Source,
    /// ★(0.14.31 · 독립 재유도 H2-A 확장 · codex 설계 검토 1) 봉투가 **도달했는데 선언이 한 줄도
    /// 코퍼스에 닿지 않았는가**.
    ///
    /// 【왜 `source` 로는 부족한가】 [`Source::Builtin`] 은 "덮을 봉투가 없다"(정상)만 뜻하지
    /// 않는다 — 봉투가 객체가 아니거나, `replace` 선언이 전부 거부됐거나, 병합 선언이 하나도
    /// 적용되지 않으면 [`resolve_raw`] 는 같은 `Builtin` 을 돌려준다. 그 상태에서 보고서가
    /// 되먹임 봉투를 내주면, 운영자가 **쓴 적 있는 선언**(파서가 거부했을 뿐 파일에는 남아 있는
    /// 것)이 안내대로 붙여 넣는 순간 빌트인으로 덮여 사라진다. 출처 하나로는 "원 선언이 애초에
    /// 없었다" 와 "있었는데 반영되지 않았다" 가 구별되지 않는다.
    ///
    /// ★명시 `null` 봉투(= 의도적 비움)와 키 부재는 **거짓**이다 — 지워질 선언이 없다.
    pub envelope_ignored: bool,
    /// ★(0.14.31 · 수렴 R2 · reviewer-claude minor) 봉투에 있었으나 **이 코퍼스에 착지하지
    /// 못한 선언의 수**(id 결손 · 비객체 항목 · needle 결손 신규 선언 · replace 파싱 실패 ·
    /// `gates` 가 배열이 아님).
    ///
    /// 【왜 [`Resolved::envelope_ignored`] 로는 부족한가】 그 축은 **전부/전무**다 — 선언 둘 중
    /// 하나만 착지하면(예: folder-trust 조이기는 착지, 신설 관문은 needle 결손으로 거부)
    /// `overridden=1` 이라 `envelope_ignored=false` 이고 보고서는 `paste_safe=true` 를 낸다.
    /// 그런데 그 봉투를 안내대로 붙여 넣으면 **거부된 선언이 파일에서 사라진다**(오타를 고칠
    /// 원본까지). 효력 있던 조임은 살아남으므로 관문 재개방은 아니지만(그래서 minor),
    /// 운영자는 "몇 건이 빠졌는지" 를 산출물에서 알아야 한다.
    ///
    /// 【이 값은 판정 재료가 아니다】 보고서의 경고 문면에만 쓴다 — 코퍼스 자체는 착지한
    /// 선언으로 이미 결정돼 있다.
    pub declarations_rejected: usize,
}

/// ★(0.14.31 · WP-1 H-2 · CONTRACTS §C) **관문 코퍼스 보고서** — `cys gate-corpus --json` 의 봉투.
///
/// 【왜 CLI 가 아니라 여기인가】 코퍼스의 어휘(`passability`·`absence_cost`·`origin`·`source`)를
/// 소유한 것은 이 모듈이다. 문자열을 CLI 에서 다시 지으면 override 봉투가 읽는 어휘
/// ([`parse_passability`]·[`parse_absence_cost`])와 **두 벌**이 되고, 그 순간 보고서는 봉투로
/// 되먹일 수 없는 방언이 된다. 그래서 산출 문자열은 봉투 파서가 **되읽을 수 있는 값**과 같다.
///
/// 【되먹임은 `override_envelope` 로 한다 — 0.14.31 리뷰 R1】 종전 보고서는 `needles`·`widget`·
/// `confirm_echo`·`human_reason` 을 싣지 않았고, 최상위 `source`(출처: `replaced`)가 봉투의
/// **모드** 어휘(`replace`)와 이름만 같고 값이 달랐다. 그래서 보고서를 그대로 `agents.json` 에
/// 붙여 넣으면 ⓐ replace 가 미지 값 → **builtin 병합으로 강등**되고 ⓑ 사용자 신설 관문은 needle
/// 결손으로 [`parse_new_gate`] 가 거부해 **소멸**했다 — 문서화된 탈출구가 운영자의 관문을 지웠다.
///
/// 수리는 어휘를 섞는 쪽이 아니라 **나누는** 쪽이다:
///   · `source`/`source_detail` — **출처**(이 코퍼스가 어떻게 만들어졌나). 봉투에 넣는 값이 아니다.
///   · `override_envelope` — 그대로 붙여 넣으면 **같은 코퍼스를 재현**하는 봉투. 모드(`source`)를
///     스스로 싣고 선언 축을 전량 싣는다. 되먹임의 유일한 정당 경로다.
///   · [`envelope_mode`] 는 보고서 출처 어휘를 모드로 **읽지 않고**(별칭은 새 권한을 연다 —
///     codex 설계 검토 R1) 병합에 착지시킨 뒤 `override_envelope` 를 이름으로 지목한다.
///
/// 【되먹임 재료를 **주지 않는** 경우 — `override_envelope_status`】 이 봉투가 정당한 되먹임
/// 재료이려면 "이 코퍼스가 운영자 선언을 반영했는가" 가 참이어야 한다. 그 답이 거짓인 출처가
/// 둘이다 — [`Source::SpecUnreadable`](스펙에 도달하지 못했다 · 고장)과
/// [`Source::OverrideDisabled`](롤백 스위치로 읽지 않기로 했다). 두 경우 모두
/// `override_envelope` 를 `null` 로 두고 `paste_safe:false` + **서로 다른 사유**를 낸다
/// (0.14.31 · 독립 재유도 H2-A/C). [`Source::Builtin`] 은 "덮을 봉투가 애초에 없다"(정상)라서
/// 여기 들지 않는다 — 셋을 한 값으로 접으면 정상 기계의 되먹임까지 죽는다.
///
/// 무손실의 범위는 **식별 가능한 관문**(needle ≥ 1)의 선언 축 전량이다. 자기규칙 수리로 needle 이
/// 0개가 된 관문은 재파싱에서 거부되는데, 그 관문은 이미 어떤 화면도 식별하지 못하는 불활성
/// 선언이라 사라져도 판정이 한 톨도 바뀌지 않는다. 집행은 검체
/// `gate_corpus_override_envelope_round_trips_builtin_merged_and_replaced_corpora`.
///
/// 【`measured_on` 두 필드의 의미 분리(codex 설계 검토 Q4)】
///   · `measured_on` — **언제나 코드 내장 상수**([`MEASURED_ON`]). 상태에 따라 뜻이 바뀌지 않는
///     안정 앵커이며, preflight `C82.gate-corpus-drift` 가 `claude --version` 과 대조하는 값이다.
///   · `effective_measured_on` — 해소된 코퍼스 **전 관문이 같은 값을 주장할 때만** 그 문자열,
///     봉투가 일부 관문만 덮어 **혼합**이면 `null`. 한 필드가 상황에 따라 '내장' 이었다가
///     '유효' 였다가 하면 하류는 어느 쪽도 믿을 수 없다 — 그래서 필드를 나눈다.
///   · `mixed_versions` — **두 값 이상이 섞였는가**(`versions.len() > 1`). `effective` 가 `null` 인
///     경우가 둘(혼합 · 관문 0개=도달 불가)이라 그 둘을 이 플래그가 가른다 — `null` 하나로는
///     "섞였다" 와 "잴 것이 없다" 가 구별되지 않는다.
///
/// 【`policy` 는 요청했을 때만】 `detected` 가 없으면 필드를 **넣지 않는다**. 넣으면 전 관문이
/// `held_version_unknown` 이 되어 "버전을 재지 못했다" 는 사실처럼 읽히는데, 실제로는 **묻지
/// 않은 것**이다(관측하지 않은 것을 관측 결과로 인쇄하지 않는다).
///
/// 【이 함수는 순수하다】 파일·데몬·서브프로세스 접촉 0. 버전은 **호출부가 재서 넣는다**.
pub fn report_json(
    resolved: &Resolved,
    agent: &str,
    detected: Option<&str>,
    observed_at: Option<&str>,
) -> Value {
    let versions: std::collections::BTreeSet<&str> =
        resolved.gates.iter().map(|g| g.measured_on.as_str()).collect();
    // 유효 버전은 **전 관문이 한 값을 주장할 때만** 잡힌다. 0개(도달 불가 — 해소기가 빈 코퍼스를
    // 돌려주지 않는다)는 '미상'이지 '혼합'이 아니므로 `mixed_versions` 는 `len > 1` 로 따로 센다.
    let effective: Option<&str> = if versions.len() == 1 {
        versions.iter().next().copied()
    } else {
        None
    };
    // 관문 하나의 **선언 축**(= override 봉투가 읽는 것 전부). 보고서 gates[] 와
    // `override_envelope.gates[]` 가 이 한 벌을 공유한다 — 두 벌로 지으면 되먹임이 다음 판에 깨진다.
    let declaration = |g: &Gate| -> serde_json::Map<String, Value> {
        let mut o = serde_json::Map::new();
        o.insert("id".into(), Value::from(g.id.as_str()));
        o.insert("title".into(), Value::from(g.title.as_str()));
        // ★식별 축(0.14.31 · 리뷰 R1) — 이것이 없으면 보고서는 **되먹일 수 없다**:
        //   `parse_new_gate` 가 needle 결손 선언을 거부하므로 사용자 신설 관문이 소멸한다.
        o.insert("needles".into(), Value::from(g.needles.clone()));
        o.insert("widget".into(), Value::from(g.widget.clone()));
        o.insert("confirm_echo".into(), Value::from(g.confirm_echo.clone()));
        o.insert(
            "human_reason".into(),
            g.human_reason.as_deref().map(Value::from).unwrap_or(Value::Null),
        );
        o.insert("passability".into(), Value::from(passability_str(g.passability)));
        o.insert("measured_on".into(), Value::from(g.measured_on.as_str()));
        o.insert(
            "absence_cost".into(),
            Value::from(absence_cost_str(g.absence_cost)),
        );
        o.insert(
            "default_index".into(),
            g.default_index.map(Value::from).unwrap_or(Value::Null),
        );
        o.insert(
            "action".into(),
            match g.action.as_ref() {
                None => Value::Null,
                Some(a) => serde_json::json!({
                    "select_index": a.select_index,
                    "label": a.label,
                    "literal": a.literal,
                }),
            },
        );
        o
    };
    let gates: Vec<Value> = resolved
        .gates
        .iter()
        .map(|g| {
            let mut o = declaration(g);
            // ── 아래는 **파생**이다(선언이 아니다). 봉투는 이 축들을 읽지 않는다.
            o.insert("origin".into(), Value::from(origin_str(g.origin)));
            o.insert(
                "down_presses".into(),
                g.down_presses().map(Value::from).unwrap_or(Value::Null),
            );
            o.insert("absence_is_fatal".into(), Value::from(g.absence_is_fatal()));
            if let Some(v) = detected {
                o.insert("policy".into(), action_policy_json(&action_policy(g, Some(v))));
            }
            Value::Object(o)
        })
        .collect();
    // ★붙여 넣을 수 있는 봉투(0.14.31 · 리뷰 R1). 보고서의 `source` 는 **출처**라 봉투의 **모드**가
    //   아니고, 그 둘을 이름이 같다는 이유로 섞으면 replace 가 병합으로 뒤집힌다([`envelope_mode`]).
    //   그래서 되먹임용 봉투를 따로, 모드를 스스로 싣게 해서 낸다.
    //
    // ★(0.14.31 · 리뷰 R2 · claude 적대) **판독 실패면 봉투를 내지 않는다.** `source=spec_unreadable`
    //   은 "agents.json 을 읽지 못해 코드 정본으로 되돌아왔다" 는 뜻이라, 그 코퍼스는 **운영자의
    //   선언을 반영하지 않는다**. 그런데도 R1 은 빌트인 6관문을 실은 붙여넣기용 봉투를 함께 냈다 —
    //   문서가 시키는 대로 붙여 넣으면 원 선언이 빌트인으로 **덮인다**. 그래서 판독 실패에서는
    //   `override_envelope` 를 `null` 로 두고, 왜 재료를 주지 않는지를 `override_envelope_status`
    //   가 말한다. ★`paste_safe` 는 파서가 집행하는 제약이 아니다(codex 설계 검토 ⑨) — 이 필드는
    //   "이 산출물을 복사해 쓰지 말라" 는 **표식**이지, 붙여 넣어도 안전하다는 보증이 아니다.
    //
    // ★★(0.14.31 · 독립 재유도 H2-A/C · 2026-09-08) **묻는 것은 출처가 아니라 "이 코퍼스가 운영자
    //   선언을 반영했는가" 하나다.** R2 는 그 질문을 `SpecUnreadable` **하나로만** 물었는데,
    //   [`Source::OverrideDisabled`](롤백 스위치로 봉투를 한 줄도 읽지 않은 상태) 역시 같은 답을
    //   갖는다 — 판독 실패가 "봉투에 도달하지 못했다"(고장)라면 이쪽은 "읽지 않기로 했다"(스위치)
    //   이고, **원 선언 미반영**이라는 사실은 똑같다. 실측(격리 CLI e2e · 2026-09-08): 운영자가
    //   folder-trust 를 `human_only` 로 조이고 자기 관문을 신설해 둔 기계에서 스위치를 끄고 뜬
    //   보고서의 봉투를 안내대로 붙여 넣으면 ⓐ 신설 관문이 소멸하고 ⓑ 조여 둔 관문이 `machine`
    //   으로 되돌아간다(= 자동확인 재개방). 스위치를 되켜도 복구되지 않는다 — 원본이 사용자 소유
    //   파일에서 지워졌다. 조이는 방향의 선언이 문서화된 되먹임으로 풀리는 것은 §3-3 역행이다.
    //
    // ★그리고 **셋을 한 값으로 접지 않는다**: `Builtin`(덮을 봉투가 애초에 없다 · 정상)에서는 봉투가
    //   그대로 안전하다. 사유 문면도 두 경우에 서로 다르다(처방이 다르다 — 하나는 판독 실패를
    //   고치는 것이고 하나는 스위치를 되켜는 것이다). 아래 `match` 는 `_` 팔을 두지 않는다:
    //   `Source` 에 변이가 늘면 컴파일러가 "이 새 출처의 코퍼스는 운영자 선언을 반영하는가" 를
    //   다시 묻게 한다(기본값이 조용히 '안전' 으로 접히지 않는다).
    let paste_reason: Option<&str> = match &resolved.source {
        Source::SpecUnreadable { .. } => Some(
            "agents.json 어댑터 스펙을 읽지 못해 코드 정본으로 되돌아온 코퍼스라, 이 보고서는 \
             운영자의 선언을 반영하지 않는다(붙여 넣으면 원 선언이 빌트인으로 덮인다). \
             먼저 판독 실패를 고칠 것",
        ),
        Source::OverrideDisabled => Some(
            "롤백 스위치(CYS_FIRST_RUN_GATES_OVERRIDE=0)로 override 파싱이 꺼져 있어 이 코퍼스는 \
             코드 정본뿐이다 — agents.json 에 어떤 선언이 있든 한 줄도 읽지 않았다. 이 봉투를 \
             붙여 넣으면 스위치를 되켰을 때 원 선언 대신 빌트인이 선다(조여 둔 passability 가 \
             풀리고 신설 관문이 사라진다 · 원본은 파일에서 지워져 복구되지 않는다). \
             먼저 스위치를 되켜고 보고서를 다시 뜰 것",
        ),
        // ★(codex 설계 검토 1) 봉투가 도달했는데 **한 줄도 반영되지 않은** 코퍼스. 출처는
        //   `Builtin` 으로 접히지만 "덮을 봉투가 없다"(정상)와는 다른 사실이다 —
        //   운영자가 쓴 선언이 파일에 남아 있고, 이 봉투를 붙여 넣으면 그것이 사라진다.
        Source::Builtin | Source::Merged { .. } | Source::Replaced { .. }
            if resolved.envelope_ignored =>
        {
            Some(
                "agents.json 의 override 봉투가 도달했지만 선언이 **한 줄도** 이 코퍼스에 \
                 반영되지 않았다(비객체 봉투 · 전부 거부된 선언). 이 보고서를 붙여 넣으면 \
                 파일에 남아 있는 그 선언이 빌트인으로 덮여 사라진다 — 먼저 notes 가 지목한 \
                 거부 사유를 고칠 것",
            )
        }
        // 봉투가 실제로 도달한 코퍼스(또는 덮을 봉투가 애초에 없는 정상) — 되먹임이 항등이다.
        Source::Builtin | Source::Merged { .. } | Source::Replaced { .. } => None,
    };
    let paste_safe = paste_reason.is_none();
    let envelope = if paste_safe {
        let envelope_gates: Vec<Value> = resolved
            .gates
            .iter()
            .map(|g| Value::Object(declaration(g)))
            .collect();
        serde_json::json!({
            "source": if matches!(resolved.source, Source::Replaced { .. }) { "replace" } else { "builtin" },
            "measured_on": MEASURED_ON,
            "gates": envelope_gates,
        })
    } else {
        Value::Null
    };
    serde_json::json!({
        "agent": agent,
        // ★(0.14.31 · 리뷰 R2 · codex minor) **이 보고서를 언제 찍었는가.** `measured_on` 은 벤더
        //   버전(무엇에 대고 실측했는가)이고 이것은 **관측 시각**이다 — 두 축을 한 값으로 접으면
        //   운영자가 `agents.json` 을 고치기 전후로 뜬 두 보고서를 시간으로 가를 수 없다.
        //   호출부가 재서 넣는다(이 함수는 순수하다 · 시계 접촉 0).
        "observed_at": observed_at,
        "source": source_kind(&resolved.source),
        "source_detail": source_detail(&resolved.source),
        "override_envelope": envelope,
        "override_envelope_status": {
            "paste_safe": paste_safe,
            // ★사유는 경우마다 **다른 문면**이다(처방이 다르다) — 하나로 접으면 운영자가
            //   스위치를 되켜야 할 자리에서 파일 권한을 뒤진다.
            "reason": paste_reason.map(Value::from).unwrap_or(Value::Null),
            // ★(0.14.31 · 수렴 R2 · reviewer-claude minor) **부분 착지**의 회계. `paste_safe`
            //   는 전부/전무 축이라 "선언 둘 중 하나만 착지" 를 안전으로 낸다 — 그 봉투를 붙여
            //   넣으면 거부된 선언이 파일에서 사라진다(오타를 고칠 원본까지). 그래서 개수를
            //   싣고, 0 이 아니면 경고를 함께 낸다. `paste_safe` 자체는 뒤집지 않는다: 착지한
            //   조임은 봉투에 그대로 실려 있고(관문 재개방 없음), 이 사실의 처방은 '붙여 넣기
            //   전에 notes 의 거부 사유를 고쳐라' 이지 '이 산출물을 쓰지 마라' 가 아니다.
            "declarations_rejected": resolved.declarations_rejected,
            "warning": if resolved.declarations_rejected > 0 {
                Value::from(format!(
                    "봉투의 선언 {}건이 이 코퍼스에 착지하지 못했다(notes 의 거부 사유 참조). \
                     이 봉투를 agents.json 에 붙여 넣으면 그 {}건이 파일에서 사라진다 — \
                     먼저 거부 사유를 고치고 보고서를 다시 뜰 것",
                    resolved.declarations_rejected, resolved.declarations_rejected
                ))
            } else {
                Value::Null
            },
        },
        // ★(0.14.31 · 리뷰 R1 · 독립 재유도 H2-B 개정) `policy` 열이 **어디까지 집행되는가**를
        //   산출물이 스스로 싣는다. 없으면 운영자는 `held_version_drift` 를 "이 버전에선 키가 안
        //   나간다" 로 읽는데 — 이제 그 읽기는 **불일치에서만** 참이고 **미상에서는 거짓**이다.
        //   그 차이를 축별로 적는다([`ACTION_POLICY_ENFORCEMENT`] doc).
        "policy_enforcement": {
            "state": ACTION_POLICY_ENFORCEMENT.as_str(),
            // 종전 축(하위 호환) — **판정 전량**이 집행되는가. 부분 집행은 여기서 false 다.
            "enforced": ACTION_POLICY_ENFORCEMENT == PolicyEnforcement::Full,
            "scope": "cli-auto-confirm",
            "axes": {
                "version_drift": ACTION_POLICY_ENFORCEMENT.denies_version_drift(),
                "version_unknown": ACTION_POLICY_ENFORCEMENT.denies_version_unknown(),
                "down_bundle": ACTION_POLICY_ENFORCEMENT.sends_down_bundle(),
            },
            // ★집행되는 축의 **증거가 무엇인가**. 이 보고서의 `detected_version` 은 호출부가
            //   손으로 준 값이라 좌석에서 관측된 것이 아니다 — 두 축을 섞어 읽지 않게 적는다.
            "evidence": "seat-boot-latch + screen-banner (inject_guard::Observed)",
            "note": match ACTION_POLICY_ENFORCEMENT {
                PolicyEnforcement::Full =>
                    "action_policy 가 전량 집행된다 — policy 는 집행 판정이다",
                PolicyEnforcement::VersionDriftOnly => concat!(
                    "확인 경계(폴더신뢰 자동확인)가 보는 것은 ⓪ 정본 사람 1회 관문이 화면에 서지 ",
                    "않을 것 ① 코퍼스가 그 id 로 식별할 것 ② 커서가 종료 위가 아닐 것 ③ 선언 ",
                    "시퀀스가 Return 한 발일 것(down_presses==Some(0)) ④ 커서가 액션 라벨 전문 ",
                    "위일 것 ⑤ 좌석이 밝힌 버전이 이 관문 실측본과 **다르지 않을 것** — 다섯이다. ",
                    "즉 held_version_drift 는 이제 '키가 안 나간다' 가 참이지만, ",
                    "held_version_unknown 은 **여전히 통과한다**(부분 실측 코퍼스로 가용성을 ",
                    "끊지 않는다 · 별도 결정). allowed 를 'down 이 집행된다' 로 읽지 말 것 — ",
                    "이 조립은 Return 만 보낸다(0.14.31 독립 재유도 H2-B)"
                ),
                PolicyEnforcement::Unwired => concat!(
                    "action_policy(버전 핀)는 CLI 자동확인 조립 어느 지점에도 배선돼 있지 않다. ",
                    "policy 는 **진단**이며 held_version_* 를 '키가 안 나간다' 로 읽지 말 것"
                ),
            },
        },
        "measured_on": MEASURED_ON,
        "effective_measured_on": effective,
        "mixed_versions": versions.len() > 1,
        "detected_version": detected,
        "notes": resolved.notes,
        "gates": gates,
    })
}

/// 봉투 파서([`parse_passability`])가 **되읽을 수 있는** 문자열.
fn passability_str(p: Passability) -> &'static str {
    match p {
        Passability::Machine => "machine",
        Passability::HumanOnly => "human_only",
    }
}

/// 봉투 파서([`parse_absence_cost`])가 **되읽을 수 있는** 문자열.
fn absence_cost_str(c: AbsenceCost) -> &'static str {
    match c {
        AbsenceCost::Fatal => "fatal",
        AbsenceCost::Recoverable => "recoverable",
    }
}

fn origin_str(o: Origin) -> &'static str {
    match o {
        Origin::Builtin => "builtin",
        Origin::Overridden => "overridden",
        Origin::Added => "added",
    }
}

/// 해소 출처의 **종류**. 회계 수치는 [`source_detail`] 이 따로 낸다 — 한 필드에 종류와 수치를
/// 섞어 넣으면(`"merged(overridden=1,…)"`) 하류가 그 문자열을 되파싱하게 되고, 되파싱은 다음 판에
/// 반드시 깨진다.
fn source_kind(s: &Source) -> &'static str {
    match s {
        Source::Builtin => "builtin",
        Source::Merged { .. } => "merged",
        Source::Replaced { .. } => "replaced",
        Source::OverrideDisabled => "override_disabled",
        Source::SpecUnreadable { .. } => "spec_unreadable",
    }
}

fn source_detail(s: &Source) -> Value {
    match s {
        Source::Builtin | Source::OverrideDisabled => Value::Null,
        Source::Merged { overridden, added } => {
            serde_json::json!({"overridden": overridden, "added": added})
        }
        Source::Replaced { count } => serde_json::json!({"count": count}),
        Source::SpecUnreadable { reason } => serde_json::json!({"reason": reason}),
    }
}

/// [`ActionPolicy`] 를 **타입 그대로** 실은 JSON. 하류가 사유 문자열을 되파싱하지 않게 `kind` 를 둔다.
fn action_policy_json(p: &ActionPolicy) -> Value {
    match p {
        ActionPolicy::Allowed { down, literal, label } => serde_json::json!({
            "kind": "allowed", "down": down, "literal": literal, "label": label,
        }),
        ActionPolicy::HumanRequired { reason } => serde_json::json!({
            "kind": "human_required", "reason": reason,
        }),
        ActionPolicy::HeldNoAction => serde_json::json!({"kind": "held_no_action"}),
        ActionPolicy::HeldVersionDrift { measured_on, detected } => serde_json::json!({
            "kind": "held_version_drift", "measured_on": measured_on, "detected": detected,
        }),
        ActionPolicy::HeldVersionUnknown { measured_on } => serde_json::json!({
            "kind": "held_version_unknown", "measured_on": measured_on,
        }),
    }
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
/// { "source": "builtin" | "replace",     // 보고서 출처 어휘는 모드가 아니다 — envelope_mode
///   "measured_on": "2.1.241",
///   "gates": [ { "id": "...", "needles": [...], ... } ] }
/// ```
/// 규칙 — ① 손상된 선언은 **그 항목만** 버리고 코드 정본을 유지한다(부트를 멈추지 않는다).
/// ② `HumanOnly` 로 실측된 관문은 override 로 **기계 통과로 승격되지 않는다**(로그인은 우회
/// 불가라는 것이 측정 결과이지 정책이 아니다). 반대 방향(Machine→HumanOnly)은 조이는 쪽이라 허용.
/// ★두 경로 모두 막는다(0.14.31 · 리뷰 R1): 병합은 [`apply_patch`], 교체는
/// [`restore_human_only_builtin_floor`] — 종전엔 교체 경로에 이 거부가 없었다.
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
        envelope_ignored,
        declarations_rejected,
    } = resolve_raw(envelope, override_on);
    // ★해소 **직후**에 Fatal 바닥을 세운다(아래 [`restore_fatal_builtin_floor`]). 아래
    //   [`enforce_absence_cost`] 는 '집행 전후 대조' 라 replace 모드에서는 눈이 멀어 있다 —
    //   그 모드의 `pre` 는 사용자 목록이라 빌트인 Fatal 관문이 **순회 대상에 애초에 없다**.
    let gates = restore_fatal_builtin_floor(gates, &mut notes);
    // ★그리고 **사람 1회** 바닥(0.14.31 · 리뷰 R1 · codex 지적). Fatal 바닥과 같은 자리·같은 이유.
    let gates = restore_human_only_builtin_floor(gates, &mut notes);
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
        envelope_ignored,
        declarations_rejected,
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
        // ★(0.14.31 · 리뷰 R2) **id 가 같은 항목 전량**을 본다(종전: `find` = 첫 항목 하나).
        //   replace 봉투는 같은 id 를 몇 번이든 선언할 수 있고, `identify` 는 **화면에 먼저
        //   매칭되는** 항목을 돌려주므로 "첫 항목만 고친다" 는 화면 층위에서 아무것도 보장하지
        //   못한다(같은 결함의 human_only 판이 codex major 로 지목됐다 — 아래 바닥 참조).
        let mut hit = false;
        for g in gates.iter_mut().filter(|g| g.id == b.id) {
            hit = true;
            if g.absence_cost != AbsenceCost::Fatal {
                notes.push(format!(
                    "{}: 선언이 부재의 비용을 낮췄다 — 실측값(fatal)으로 되돌린다(부재의 \
                     귀결은 선언이 아니라 측정이다)",
                    g.id
                ));
                g.absence_cost = AbsenceCost::Fatal;
            }
        }
        if !hit {
            notes.push(format!(
                "{}: ★해소본에 **부재의 비용이 비가역인** 빌트인 관문이 없다 — 코드 정본을 \
                 강제 **복원**했다(선언은 그대로 두고 덧붙이기만 한다). 이 관문의 부재는 \
                 좌석 rc 1 종료·허위 READY 영구화·관측 전제 파괴 중 하나로 귀결한다",
                b.id
            ));
            gates.push(b);
        }
    }
    gates
}

/// ★사람 1회 바닥 — [`Passability::HumanOnly`] 로 **실측된** 빌트인 관문은 어떤 해소 모드에서도
/// 기계 통과로 승격되지 않는다(0.14.31 · 리뷰 R1 · codex 설계 검토 지적).
///
/// 【무엇이 뚫려 있었는가】 병합 경로의 [`apply_patch`] 는 `human_only → machine` 승격을 이미
/// 거부한다. 그런데 **replace 경로**는 [`parse_new_gate`] 를 지나고 그 함수에는 같은 거부가 없다.
/// 그래서 `{"source":"replace","gates":[{"id":"login-method","passability":"machine",
/// "action":{...},"needles":[...]}]}` 한 줄이면 OAuth 화면에 키가 나간다 — 좌석은 브라우저 대기에
/// 갇힌 채 **살아 있고**, 생존만 보는 판정이 그것을 영원히 '준비됨' 으로 읽는다(허위 READY 영구화).
/// 로그인이 사람 1회를 요구하는 것은 **정책이 아니라 측정 결과**이므로(자격증명 경로해시 봉인 ·
/// OAuth 무한 루프) 선언으로 뒤집게 두면 거짓 전제 위에서 키를 쏘게 된다(정본 §8).
///
/// 【무엇을 하지 않는가】 사용자 선언을 지우지 않는다 — id 를 가로챈 선언은 코퍼스에 그대로 남고
/// **통과 가능성 축과 액션만** 실측값으로 되돌린다(Fatal 바닥의 비용 축과 같은 비대칭).
/// 반대 방향(`machine → human_only`)은 조이는 쪽이라 건드리지 않는다.
fn restore_human_only_builtin_floor(mut gates: Vec<Gate>, notes: &mut Vec<String>) -> Vec<Gate> {
    for b in builtin()
        .into_iter()
        .filter(|b| b.passability == Passability::HumanOnly)
    {
        // ★(0.14.31 · 리뷰 R2) 봉인 대상은 **id 하나**가 아니라 "그 화면을 자기 것이라 주장하는
        //   선언 전량" 이다. 아래 두 우회가 R1 의 id 단일 매칭을 그대로 통과했다:
        //     ⓐ 중복 id(codex major) — `{"source":"replace","gates":[{"id":"login-method",
        //        "needles":["Do you want a decoy?"]…},{"id":"login-method", <진짜 로그인 needle>,
        //        "passability":"machine","action":{…}}]}`. `find` 는 **미끼**를 고치고, 로그인
        //        화면에서 `identify` 가 돌려주는 것은 고쳐지지 않은 두 번째 항목이다.
        //     ⓑ 별칭 id(claude 적대) — `{"id":"login-alias","needles":["Select login method"],
        //        "passability":"machine","action":{…}}` 한 줄이면 id 가 안 겹쳐 바닥이 아예 돌지
        //        않고, 그 선언이 복원된 `login-method` **앞**에 서서 첫 매치를 가져간다.
        //   두 경우의 귀결은 같다: 코퍼스가 로그인 화면을 **기계 통과 가능**으로 분류한다.
        for g in gates
            .iter_mut()
            .filter(|g| g.id == b.id || claims_same_screen(g, &b))
        {
            if g.passability == Passability::HumanOnly && g.action.is_none() {
                continue;
            }
            let by_alias = g.id != b.id;
            notes.push(format!(
                "{}: 선언이 **사람 1회**로 실측된 관문{}을 기계 통과로 승격시켰다 — 실측값\
                 (human_only · 액션 없음)으로 되돌린다(기계 통과 불가는 정책이 아니라 측정이다)",
                g.id,
                if by_alias { format!("({} 의 화면을 같은 문면으로 주장한다)", b.id) } else { String::new() }
            ));
            g.passability = Passability::HumanOnly;
            g.action = None;
            if g.human_reason.is_none() {
                g.human_reason = b.human_reason.clone();
            }
        }
    }
    gates
}

/// 이 선언이 저 관문의 **화면을 자기 것이라 주장하는가** — id 가 아니라 식별 문면으로 본다.
///
/// 판정: `g` 의 needle 하나와 `b` 의 needle 하나가 **서로 포함**(정규화본 또는 공백 제거본 ·
/// 어느 방향이든)이면 참. 두 방향 모두 참으로 세는 이유 —
///   · `g ⊆ b`(g 의 문면이 더 짧다): g 는 b 보다 **넓은** 화면 집합에 걸린다 → 로그인 화면에도
///     반드시 걸린다.
///   · `g ⊇ b`(g 의 문면이 더 길다): 좁지만 벤더가 그 긴 문면을 그리는 순간 같은 화면이다.
/// 어느 쪽도 "그 화면에 설 수 있다" 는 사실이라 봉인 대상이다.
///
/// ★위젯 서명은 보지 않는다(의도적 과잉 포섭). [`Gate::matches`] 는 위젯 AND 를 요구하지만
///   감지 경로([`crate::inject_guard::needle_hit`])는 needle 만 보고, 무엇보다 오탐의 귀결이
///   **보류**(사람 1회)라서 넓은 쪽이 안전하다. 좁게 잡았다가 놓친 것의 귀결은 OAuth 화면에
///   키가 나가는 것이다(허위 READY 영구화 · 비가역) — 비대칭이 방향을 정한다(§3-3).
///
/// ★빌트인 코퍼스는 이 술어로 서로 겹치지 않는다(검체
///   `human_only_floor_seals_duplicate_ids_and_alias_declarations` 가 전수 대조).
fn claims_same_screen(g: &Gate, b: &Gate) -> bool {
    g.needles.iter().any(|n| {
        let (nn, nf) = (normalize(n), flatten(n));
        if nn.is_empty() || nf.is_empty() {
            return false;
        }
        b.needles.iter().any(|m| {
            let (mn, mf) = (normalize(m), flatten(m));
            if mn.is_empty() || mf.is_empty() {
                return false;
            }
            nn.contains(&mn) || mn.contains(&nn) || nf.contains(&mf) || mf.contains(&nf)
        })
    })
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

/// 봉투 `source` 필드가 지시하는 **해소 모드**.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EnvelopeMode {
    /// 선언된 것만이 코퍼스다(사용자 주권 · 빈 선언이면 정본 폴백).
    Replace,
    /// 코드 정본 위에 덮는다(기본값 · 미지 값의 안전한 착지점).
    Merge,
}

/// [`report_json`] 이 인쇄하는 **출처 어휘** — 봉투 모드가 아니다([`envelope_mode`] 참조).
const REPORT_SOURCE_WORDS: &[&str] = &["builtin", "merged", "replaced", "override_disabled", "spec_unreadable"];

/// ★(0.14.31 · 리뷰 R1) 봉투 `source` 를 모드로 옮긴다 — 그리고 **보고서 어휘를 모드로 읽지
/// 않는다**(같은 이름의 두 어휘를 구별해 크게 말한다).
///
/// 【무엇이 틀렸었는가】 보고서는 **출처**를 `builtin|merged|replaced|override_disabled` 로
/// 인쇄하고 봉투는 **모드**를 `builtin|replace` 로 읽는다. 이름이 같아서, `cys gate-corpus --json`
/// 산출을 그대로 `agents.json` 에 붙여 넣으면 `"replaced"` 가 미지 값으로 접혀 **replace 가 병합으로
/// 조용히 뒤집혔다**. 게다가 종전 보고서는 needle 을 싣지 않아 사용자 신설 관문이 재파싱에서
/// 거부돼 **소멸**했다 — 문서화된 탈출구가 운영자의 관문을 지우는 경로였다(BLOCK-3 형태).
///
/// 【왜 별칭(`replaced`→replace)으로 고치지 않는가 — codex 설계 검토 R1 채택】 별칭은 **새 권한**을
/// 연다: 같은 입력이 병합에서 교체로 바뀌면 ⓐ 편집 중 빠뜨린 Recoverable 관문이 사라지고,
/// ⓑ [`parse_new_gate`] 는 [`apply_patch`] 와 달리 `human_only → machine` 승격을 거부하지 않으므로
/// 편집된 보고서가 로그인 관문을 기계 통과로 선언할 수 있다. 그래서 모드는 **넓히지 않는다** —
/// 보고서 어휘는 종전대로 [`EnvelopeMode::Merge`](빌트인 6관문이 남는 쪽 = 안전 방향)에 착지하되,
/// 조용히가 아니라 **무엇을 붙여 넣어야 하는지 이름으로 지목**한다: `report_json` 의
/// `override_envelope` 축이 곧 그 자리에 붙여 넣을 봉투다(그 봉투는 모드를 스스로 싣는다).
///
/// ★승격 거부의 빈틈 자체는 [`restore_human_only_builtin_floor`] 가 replace 경로에서도 닫는다.
fn envelope_mode(raw: Option<&str>, notes: &mut Vec<String>) -> EnvelopeMode {
    // ★(0.14.31 · 리뷰 R2 · claude 적대) **공백을 흡수하지 않는다.** R1 은 `raw.map(str::trim)` 으로
    //   `" replace "` 까지 교체로 받았는데, 교체는 **선언되지 않은 관문을 지울 수 있는 유일한 모드**다
    //   (실측: `"source":" replace "` → theme 소실 · base cdba8e8 은 같은 입력에서 병합 + 미지 값 note).
    //   관용을 베푸는 방향이 하필 파괴적인 쪽이면 §3-3("막는 쪽으로만 틀린다")에 역행한다. 그래서
    //   교체는 **정확히 `"replace"`** 하나이고, 공백이 섞인 변형은 병합에 착지하되 조용히가 아니라
    //   "정확히 쓰라" 고 사유를 남긴다(관용의 부재를 운영자가 모르고 지나가지 않는다).
    let raw = raw.unwrap_or("builtin");
    match raw {
        "replace" => EnvelopeMode::Replace,
        "builtin" => EnvelopeMode::Merge,
        other if other.trim() == "replace" || other.trim() == "builtin" => {
            notes.push(format!(
                "{ADAPTER_KEY}.source={other:?} 에 공백이 섞여 있다 — 모드 어휘는 **정확 일치**이므로 \
                 builtin 병합으로 취급했다(교체는 선언되지 않은 관문을 지울 수 있는 유일한 모드라 \
                 공백까지 받아 주지 않는다). 교체를 원하면 정확히 \"replace\" 로 쓸 것"
            ));
            EnvelopeMode::Merge
        }
        other => {
            if REPORT_SOURCE_WORDS.contains(&other) {
                notes.push(format!(
                    "{ADAPTER_KEY}.source={other:?} 는 `cys gate-corpus` 보고서의 **출처** 어휘이지 \
                     봉투의 **모드**(`builtin`|`replace`)가 아니다 — builtin 병합으로 취급했다. \
                     보고서를 되먹이려면 보고서의 `override_envelope` 축을 그대로 붙여 넣어라 \
                     (그 봉투는 모드와 선언 축 전량을 스스로 싣는다)"
                ));
            } else {
                notes.push(format!(
                    "{ADAPTER_KEY}.source={other:?} 는 미지 값 — builtin 병합으로 취급한다"
                ));
            }
            EnvelopeMode::Merge
        }
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
            // 봉투를 **읽지 않았다** — 반영/미반영을 말할 자리가 아니다(출처가 이미 말한다).
            envelope_ignored: false,
            declarations_rejected: 0,
        };
    }
    let mut notes: Vec<String> = Vec::new();
    let Some(env_v) = envelope else {
        return Resolved {
            gates: base,
            notes,
            source: Source::Builtin,
            envelope_ignored: false, // 키가 없다 = 지워질 선언이 없다
            declarations_rejected: 0,
        };
    };
    if env_v.is_null() {
        // 명시 null = "봉투를 의도적으로 비움" → 코드 정본만(사용자 주권 · 코퍼스는 남는다).
        return Resolved {
            gates: base,
            notes,
            source: Source::Builtin,
            envelope_ignored: false, // 의도적 비움도 지워질 선언이 없다
            declarations_rejected: 0,
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
            // ★운영자가 **무언가 써 두었는데** 파서가 통째로 버렸다 — 되먹임이 그것을 덮는다.
            envelope_ignored: true,
            declarations_rejected: 0, // 선언 단위로 셀 수 없다(봉투가 통째로 비객체다)
        };
    };
    let mode = envelope_mode(obj.get("source").and_then(|v| v.as_str()), &mut notes);
    let default_measured = obj
        .get("measured_on")
        .and_then(|v| v.as_str())
        .unwrap_or(MEASURED_ON)
        .to_string();
    // ★(0.14.31 · 수렴 R2 · codex major) `gates` 키의 **형(型)을 구분한다.** 종전은
    //   `as_array()` 실패를 빈 배열로 접었고, 그래서 `{"gates":{…객체 하나…}}` 같은 흔한 오타가
    //   ⓐ 선언 0건으로 접히고 ⓑ `envelope_ignored` 도 서지 않아 ⓒ 보고서가 `paste_safe=true`
    //   와 빌트인 봉투를 냈다 — 안내대로 덮어쓰면 그 객체 선언이 파일에서 사라진다. 원 선언이
    //   이미 파싱되지 않던 상태라 활성 관문의 재개방은 아니지만, "반영 0" 분기의 누락이다.
    //   ★키 부재·명시 null·의도적 빈 배열은 **지울 선언이 없다** — 그 셋과 타입 오류를 가른다.
    let gates_v = obj.get("gates");
    let gates_malformed = matches!(gates_v, Some(v) if !v.is_array() && !v.is_null());
    if gates_malformed {
        notes.push(format!(
            "{ADAPTER_KEY}.gates 가 배열이 아니다({}) — 선언을 한 건도 읽지 못했다. 이 보고서의 \
             봉투를 붙여 넣으면 파일에 남아 있는 그 선언이 사라진다",
            gates_v.map(kind_of).unwrap_or("없음")
        ));
    }
    let decls: Vec<&Value> = gates_v
        .and_then(|v| v.as_array())
        .map(|a| a.iter().collect())
        .unwrap_or_default();
    // 착지하지 못한 선언의 수(보고서 경고용 · 판정 재료 아님). 비배열은 **개수를 셀 수 없다**
    // — 그 사실은 `envelope_ignored` 가 싣는다.
    let mut rejected = 0usize;

    if mode == EnvelopeMode::Replace {
        let mut out: Vec<Gate> = Vec::new();
        for d in &decls {
            match parse_new_gate(d, &default_measured) {
                Ok(g) => out.push(g),
                Err(e) => {
                    rejected += 1;
                    notes.push(format!("replace 선언 무시: {e}"));
                }
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
                // 선언이 있었는데 전부 거부됐으면 되먹임이 그 선언을 지운다(선언 0건이면 지울 것도 없다).
                // ★비배열 `gates` 도 같은 사실이다(선언이 파일에 남아 있는데 한 줄도 반영되지 않았다).
                envelope_ignored: gates_malformed || !decls.is_empty(),
                declarations_rejected: rejected,
            };
        }
        // ★(0.14.31 · 리뷰 R2 · codex major) **중복 id 를 시끄럽게 만든다.** 버리지는 않는다 —
        //   이 모듈의 집행 수단은 '수리이지 제거가 아니다'(`repair_gate` doc · P4-10)이고, 제거는
        //   `보류 → 주입` 으로 귀결을 뒤집는 유일한 방향이다. 대신 두 사실을 함께 둔다:
        //   ⓐ 두 바닥이 이제 **같은 id 전량**을 고치므로 중복으로 바닥을 우회할 수 없고,
        //   ⓑ 그럼에도 중복은 위험 신호다 — id 로 찾는 코드(`apply_patch`)와 화면으로 찾는 코드
        //      (`identify`)가 **다른 항목**을 가리키기 때문이다. 그 어긋남을 운영자가 알아야 한다.
        let mut seen: std::collections::BTreeMap<&str, Vec<usize>> =
            std::collections::BTreeMap::new();
        for (i, g) in out.iter().enumerate() {
            seen.entry(g.id.as_str()).or_default().push(i);
        }
        // 선언 **위치**까지 적는다(codex 설계 검토 ②) — 개수만으로는 어느 줄을 고쳐야 할지 모른다.
        let dups: Vec<String> = seen
            .iter()
            .filter(|(_, at)| at.len() > 1)
            .map(|(id, at)| format!("{id}@gates{at:?}"))
            .collect();
        if !dups.is_empty() {
            notes.push(format!(
                "source=replace 선언에 **중복 id** 가 있다({}) — 선언은 그대로 두지만, id 로 찾는 \
                 경로(패치·바닥)와 화면으로 찾는 경로(identify)가 서로 다른 항목을 가리킬 수 있다. \
                 실측 바닥(fatal·human_only)은 같은 id 전량에 적용되지만, 중복은 의도한 선언이 \
                 아닐 가능성이 높다",
                dups.join(", ")
            ));
        }
        let count = out.len();
        return Resolved {
            gates: out,
            notes,
            source: Source::Replaced { count },
            envelope_ignored: false, // 선언이 코퍼스가 됐다
            declarations_rejected: rejected,
        };
    }
    let mut gates = base;
    let (mut overridden, mut added) = (0usize, 0usize);
    for d in &decls {
        let Some(dm) = d.as_object() else {
            rejected += 1;
            notes.push("gates[] 항목이 객체가 아니다 — 무시".to_string());
            continue;
        };
        let Some(id) = dm.get("id").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) else {
            rejected += 1;
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
                Err(e) => {
                    rejected += 1;
                    notes.push(format!("신규 관문 선언 무시: {e}"));
                }
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
        // ★선언이 있었는데 **하나도** 착지하지 못했다(id 결손·비객체·needle 결손 신규 선언 …).
        //   그 상태의 보고서 봉투를 붙여 넣으면 파일에 남아 있던 그 선언들이 빌트인으로 덮인다.
        //   ★비배열 `gates` 도 같은 사실이다(codex major) — `decls` 가 비어 접히므로 이 항이
        //     없으면 "반영 0" 분기가 통째로 비어 있다.
        envelope_ignored: gates_malformed || (!decls.is_empty() && overridden == 0 && added == 0),
        declarations_rejected: rejected,
    }
}

/// 진단 문면용 JSON 형(型) 이름 — 파서가 되읽는 값이 아니다(사람이 읽는 한 마디).
fn kind_of(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// 봉투의 문자열 배열 — **공백뿐인 항목은 버린다**.
///
/// ★(0.14.31 · 성찰 R3 · blocking) 종전 필터는 `!s.is_empty()` 였다. `"   "`·`"\t"`·NBSP 는
/// 비어 있지 않으므로 통과했고, [`normalize`]/[`flatten`] 이 그것을 빈 문자열로 만든 뒤
/// `contains("")` 가 **모든 화면에 참**이 되어 그 관문이 상시 성립했다(영구 부트 라이브락 →
/// `CYS_GATE_PENDING_CLOSE=1` 이면 모든 pane 사망). `trim()` 은 유니코드 공백(NBSP 포함)을
/// 벗기므로 세 변형이 여기서 함께 사라진다.
fn str_vec(v: Option<&Value>) -> Option<Vec<String>> {
    let arr = v?.as_array()?;
    Some(
        arr.iter()
            .filter_map(|x| x.as_str())
            .filter(|s| !s.trim().is_empty())
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
        // ★0 도 값이다(0.14.31 · 리뷰 R1) — 아래 [`apply_patch`] 의 같은 축 참조.
        default_index: o
            .get("default_index")
            .and_then(|x| x.as_u64())
            .filter(|n| *n <= u8::MAX as u64)
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
    // ★(0.14.31 · 리뷰 R2 · codex 설계 검토 ⑧) 무변화 판정의 기준은 **패치 진입 시점의 값**이다.
    //   패치 도중 값(예: `passability:"human_only"` 가 먼저 지운 action)과 비교하면 **실제로 일어난
    //   변경**까지 무변화로 오인해 조용해진다.
    let entry_default_index = g.default_index;
    let entry_action = g.action.clone();
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
    // ★기본 포커스 축 — **0 도 값이고, 명시 `null` 은 지운다**(0.14.31 · 리뷰 R1).
    //
    // 【무엇이 틀렸었는가】 종전 필터는 `n > 0` 이라 `0` 을 조용히 버렸고, 그러면 빌트인 값이
    // 그대로 남아 봉투는 "고쳤다고 생각하는데 안 고쳐진" 상태가 된다. 그런데 정본 §4 H-2 가
    // 기입하라고 지시한 값이 바로 그것이다 — 2.1.261 폴더신뢰는 `default_index: Some(0)`
    // (`0="No, exit"` · `1="Yes, I trust this folder"`) + `action:(1,…)` → `down = 1`. 즉 **문서화된
    // 탈출구가 정본이 요구하는 정정을 표현하지 못했다.**
    //
    // 【`null` = 지움이 왜 안전한가】 `default_index: None` 이면 [`Gate::down_presses`] 가 `None`
    // 을 내고 그 관문의 액션은 보류된다(fail-closed). 즉 지움은 **조이는 방향**이며, 기본 포커스를
    // 실측하지 못한 운영자가 "모른다" 를 정직하게 선언할 수 있는 유일한 자리다.
    //
    // 【`select_index` 는 왜 0 을 받지 않는가 — 의도된 비대칭】 `default_index` 는 화면을 읽은
    // **관측**이고 `select_index` 는 키를 쏘는 **지시**다(1-based 화면 번호 계약 · `parse_action`).
    // 0-based 메뉴의 0번을 액션으로 지목할 수 없다는 표현 불능이 남지만, 그 불능의 귀결은
    // `action = None` → `HeldNoAction` = **보류**라 실패 방향이 옳다(§3-3).
    //
    // ★(0.14.31 · 리뷰 R2 · claude 적대) **값이 안 바뀌면 사유를 남기지 않는다.** `report_json` 이
    // 권하는 되먹임 봉투는 선언 축을 전량 싣기 때문에 `default_index: null`(원래 None 인 관문) ·
    // `action: null`(human_only 관문)이 매 해소마다 들어온다. 그 무변화 입력에 note 를 남기면
    // `cysd [gate-corpus]` 와 `cys [inject-guard]` 로 **프로세스마다** 오해성 3줄이 나가고,
    // 이 리포 자신의 규율("진짜 신호가 묻힌다")을 스스로 어긴다. 실제 변경·실제 무시는 그대로
    // 시끄럽게 남는다 — 억제되는 것은 **아무 일도 일어나지 않은 경우** 하나뿐이다.
    if d.contains_key("default_index") {
        match d.get("default_index") {
            Some(v) if v.is_null() => {
                g.default_index = None;
                if entry_default_index.is_some() {
                    notes.push(format!(
                        "{}: default_index 를 명시 null 로 지웠다 — 이 관문의 아래키 수는 이제 \
                         산출되지 않는다(보류: 자동확인도 이 관문에서 닫힌다)",
                        g.id
                    ));
                }
            }
            Some(v) => match v.as_u64().filter(|n| *n <= u8::MAX as u64) {
                Some(i) => g.default_index = Some(i as u8),
                None => notes.push(format!(
                    "{}: default_index 선언이 손상({v}) — 종전 기본 포커스 유지",
                    g.id
                )),
            },
            None => {}
        }
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
            // 무변화(선언도 null · **진입 시** 액션도 None)면 조용하다 — 위 note 억제와 같은 규율.
            if !(d.get("action").is_some_and(|v| v.is_null()) && entry_action.is_none()) {
                notes.push(format!("{}: human_only 관문의 action 선언 무시", g.id));
            }
            g.action = None;
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
    /// ★실측(2026-09-06 10:18:58 · claude **2.1.261** · 본부 라이브 좌석 `cys read-screen` · 읽기 전용) —
    /// 유휴 프롬프트의 **입력 상자 아래**에 괘선·상태줄이 그려진다(2.1.241 검체 [`LIVE_TUI_AT_PROMPT`] 는
    /// `? for shortcuts` 를 프롬프트 **위**에 둔다). 재주입 생애 창(readiness 축 ①')의 실측 근거이며,
    /// 2.1.241 검체를 대체하지 않고 **추가**한다(§3-8). 본문·괘선 폭은 줄였고 문면은 실측 그대로다.
    pub const LIVE_TUI_2_1_261_STATUS_BELOW_PROMPT: &str = "\x20 현재 상태: 전 노드 무착수 대기. 이 pane 에 임무 한 줄을 직접 입력하시면 게이트가 열리고 즉시 이어갑니다.\n\
        \n\
        ✻ Churned for 3m 34s · done 오전 7:14\n\
        \x20                                   new task? /clear to save 353.6k tokens\n\
        ────────────────────────────────────────────────────────────\n\
        ❯ \n\
        ────────────────────────────────────────────────────────────\n\
        \x20 Opus 5 · CTX 35% · 5h 20% · 7d 33%                      /rc\n\
        \x20 ⏵⏵ bypass permissions on (shift+tab to cycle) · ← for agents\n";

    /// ★(0.14.31 · 리뷰 R1b · 실측 2026-09-06 12:13:56 · 본부 라이브 좌석 3개 `cys read-screen` 읽기 전용)
    /// 2.1.261 유휴 그리드의 **실제 바이트**: 대기 프롬프트 줄은 `❯` 뒤에 ASCII 공백이 아니라 **U+00A0(NBSP)**
    /// 하나다. 위 `LIVE_TUI_2_1_261_STATUS_BELOW_PROMPT`(10:18:58 실측 · `❯ ` 로 옮겨 적음)와 같은 레이아웃이고,
    /// 이 검체는 그 줄만 관측 바이트 그대로 둔다 — 판정부의 공백 판정(`trim`·`is_whitespace`)이 NBSP 를 공백으로
    /// 읽는지를 실측 바이트로 못 박기 위함(대체 아님 · 추가).
    pub const LIVE_TUI_2_1_261_NBSP_PROMPT: &str = "\n\
        ✻ Churned for 3m 34s · done 오전 7:14\n\
        \x20                                                                   new task? \n\
        ──────────────────────────────────────────────────────────────────────────────\n\
        ❯\u{a0}\n\
        ──────────────────────────────────────────────────────────────────────────────\n\
        \x20 Opus 5 · CTX 35% · 7d 33%                                                  \n\
        \x20 ⏵⏵ bypass permissions on (shift+tab to cycle) · ← for agents\n";

    // ── ★2026-09-08 실측(WP-1 H-2 · claude 2.1.261/2.1.263 · 격리 CLAUDE_CONFIG_DIR) ──────
    //
    // 【무엇을 쟀나】 버려 쓰는 `CLAUDE_CONFIG_DIR` 로 claude 2.1.261 을 격리 데몬의 PTY 좌석에
    // 띄워 화면을 읽었다. 본 관문은 **theme · login-method 둘뿐**이다 — 그 다음은
    // `login-method`(HumanOnly · action 없음)의 벽이라 기계가 넘을 키가 없다. 그래서
    // 폴더신뢰·면책·fullscreen 은 **못 봤고**, [`MEASURED_ON`] 도 folder-trust 선언도
    // 이 회차에서 움직이지 않는다(정본 §4 H-2 "부분 실측이면 핀을 올리지 않는다").
    //
    // 【왜 더하기만 하는가】 2.1.241 픽스처는 그 버전의 **역사적 관측**이다. 덮어쓰면 그때의
    // 증거가 사라지고, 두 버전 사이의 드리프트를 다시는 대조할 수 없다(§3-8).
    //
    // 【★계약 이탈 고지 — 이 검체들은 "키 0" 으로 얻어진 것이 **아니다**】(0.14.31 · 리뷰 R1)
    // 정본 §4 H-2 의 계측 문면은 "관측만 하고 **키는 보내지 않는다**" 이다. 실제로는 theme 관문에서
    // `Return` 을 **3발**(2026-09-08 04:31:43 · 04:33:30 · 04:35:1x · 좌석 3개) 보냈다 — 화면의 기본
    // 포커스(`❯ 2. Dark mode ✔`)를 먼저 읽어 코퍼스 선언(`default_index=Some(2)` · `action=(2,"Dark
    // mode")` → `down_presses()==Some(0)`)과 대조한 뒤, "Return 이 곧 선언된 통과 액션" 인 자리에서만
    // 눌렀다. 그럼에도 **집행 층위에서는 보류였다**: `action_policy(theme, Some("2.1.261"))` 는
    // 코퍼스 실측본이 2.1.241 이라 [`ActionPolicy::HeldVersionDrift`] 다. 즉 "버전 핀을 통과한 키만
    // 보냈다" 고 말할 수 없다. 귀결은 버려 쓰는 config dir 의 테마 1개(가역)이고 라이브 설치본은
    // 무접촉(계측 전후 `~/.claude.json`·`~/.claude` mtime 동일)이었으나, 문면과 행위가 다르다는
    // 사실은 남는다 — 다음 회차 계측 지시는 "키 0" 인지 "선언 액션 1발 허용" 인지 **명문화**할 것.
    // (`impl/R5-WP1-H2-corpus-notes.md` §1-3 이 전량 기록 · 그 밖의 키는 0발.)

    /// ★실측(2026-09-08 04:31 · claude **2.1.261** · 격리 CLAUDE_CONFIG_DIR · 120x40 PTY).
    /// 인사 배너(ASCII 아트 15줄)는 한 줄로 접고, 미리보기 괘선은 폭만 40 으로 줄였다
    /// (H-1 의 `LIVE_TUI_2_1_261_STATUS_BELOW_PROMPT` 와 같은 전사 규약 — 판정 재료가 아닌 줄만).
    /// **2.1.241 픽스처 `THEME` 를 대체하지 않는다**(§3-8 · 반례는 더한다).
    pub const THEME_2_1_261: &str = "\
        Welcome to Claude Code v2.1.261\n\
        \x20…배너…\n\
        \x20Let's get started.\n\
        \n\
        \x20Choose the text style that looks best with your terminal\n\
        \x20To change this later, run /theme\n\
        \n\
        \x20  1. Auto (match terminal)\n\
        \x20❯ 2. Dark mode ✔\n\
        \x20  3. Light mode\n\
        \x20  4. Dark mode (colorblind-friendly)\n\
        \x20  5. Light mode (colorblind-friendly)\n\
        \x20  6. Dark mode (ANSI colors only)\n\
        \x20  7. Light mode (ANSI colors only)\n\
        \n\
        \x20╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌\n\
        \x20 1  function greet() {\n\
        \x20 2 -  console.log(\"Hello, World!\");                                                                               \n\
        \x20 2 +  console.log(\"Hello, Claude!\");                                                                              \n\
        \x20 3  }\n\
        \x20╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌\n\
        \x20 Syntax theme: Monokai Extended (ctrl+t to disable)\n\
        ";

    /// ★실측(2026-09-08 04:32 · claude 2.1.261 · 테마 관문 Return 1발 뒤 화면).
    /// 2.1.241 대비 **선택지 3번이 늘었다**(`3rd-party platform · Amazon Bedrock, …`).
    pub const LOGIN_METHOD_2_1_261: &str = "\
        Welcome to Claude Code v2.1.261\n\
        \x20…배너…\n\
        \n\
        \x20Claude Code can be used with your Claude subscription or billed based on API usage through your Console account.\n\
        \n\
        \x20Select login method:\n\
        \n\
        \x20❯ 1. Claude account with subscription · Pro, Max, Team, or Enterprise\n\
        \x20  2. Anthropic Console account · API usage billing\n\
        \x20  3. 3rd-party platform · Amazon Bedrock, Microsoft Foundry, or Vertex AI\n\
        ";

    /// ★실측(2026-09-08 04:34 · claude 2.1.261) — **코퍼스에 없는** 벤더 모달.
    ///
    /// ★키 자리 마스킹 규약(0.14.31 · 리뷰 R1). 화면에 찍힌 것은 프로브가 넣은 더미 값의 벤더
    ///   절단본이었다. 이 모듈은 `cfg(test)` 가 아니라 **출하 바이너리에 실리는** `pub mod` 이므로
    ///   (`strings target/debug/cys` 에 그대로 잡힌다) 리포 관례대로 `sk-ant-…(마스킹)` 으로 봉한다
    ///   (`scripts/fake_agent.py:193` 과 같은 문면). `<cwd>` 자리표시자와 같은 규약이며 **판정에는
    ///   쓰이지 않는다** — 이 화면의 판정 축은 needle(질문 문면) 부재와 푸터(`Enter to confirm` ∧
    ///   `Esc to cancel`) 둘뿐이고, 어느 축도 이 줄을 보지 않는다.
    pub const CUSTOM_API_KEY_MODAL_2_1_261: &str = "\
        Welcome to Claude Code v2.1.261\n\
        \x20…배너…\n\
        \n\
        \n\
        ────────────────────────────────────────\n\
        \x20 Detected a custom API key in your environment\n\
        \n\
        \x20 ANTHROPIC_API_KEY: sk-ant-…(마스킹)\n\
        \n\
        \x20 Do you want to use this API key?\n\
        \n\
        \x20   Yes\n\
        \x20 ❯ No (recommended)\n\
        \n\
        \x20 Enter to confirm · Esc to cancel\n\
        ";

    /// ★실측(2026-09-08 04:35 · claude **2.1.263** = 이 기계의 현행 설치본).
    pub const THEME_2_1_263: &str = "\
        Welcome to Claude Code v2.1.263\n\
        \x20…배너…\n\
        \x20Let's get started.\n\
        \n\
        \x20Choose the text style that looks best with your terminal\n\
        \x20To change this later, run /theme\n\
        \n\
        \x20  1. Auto (match terminal)\n\
        \x20❯ 2. Dark mode ✔\n\
        \x20  3. Light mode\n\
        \x20  4. Dark mode (colorblind-friendly)\n\
        \x20  5. Light mode (colorblind-friendly)\n\
        \x20  6. Dark mode (ANSI colors only)\n\
        \x20  7. Light mode (ANSI colors only)\n\
        \n\
        \x20╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌\n\
        \x20 1  function greet() {\n\
        \x20 2 -  console.log(\"Hello, World!\");                                                                               \n\
        \x20 2 +  console.log(\"Hello, Claude!\");                                                                              \n\
        \x20 3  }\n\
        \x20╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌\n\
        \x20 Syntax theme: Monokai Extended (ctrl+t to disable)\n\
        ";

    /// ★**관문이 아닌 화면** 표 — 그리고 이것은 테스트 자료가 아니라 **프로덕션 입력**이다.
    ///
    /// 소비 경로: [`super::needle_non_gate_hits`] → [`super::gate_rule_violations`] →
    /// [`super::repair_gate`]. 즉 여기 실린 화면에 걸리는 needle 은 **런타임에 제거되거나 정본으로
    /// 복원된다**. 그래서 이 표에 무엇을 넣는가는 "검체 커버리지" 문제가 아니라 **어떤 문면을
    /// 관문의 근거로 쓸 수 없게 만드는가** 의 문제다.
    ///
    /// ## 등재 기준 (0.14.31 · 리뷰 R1 — codex 지적 채택)
    ///
    /// **그 화면에서 주입이 옳은 화면만 넣는다.** 정상 프롬프트·라이브 세션 화면·문서/로그 본문이
    /// 그것이다(`live-permission-prompt`·`audit-log-line` 은 모달 어휘를 갖지만 **첫기동 관문이
    /// 아니라 라이브 세션의 화면**이라 남는다 — 첫기동 관문 코퍼스가 그 문면으로 성립하면 그것은
    /// 정의상 오탐이고, 귀결은 영구 부트 라이브락이다).
    ///
    /// **첫기동에 뜨는 코퍼스 밖 모달은 넣지 않는다** → [`MEASURED_NON_CORPUS_MODALS`].
    /// 넣으면 그 문면을 needle 로 선언한 **사용자 신설 관문의 needle 이 제거되어**(`repair_gate`
    /// 의 사용자 관문 팔) 관문이 식별 불능이 된다. 그것은 문서화된 탈출구(agents.json 봉투)를
    /// 그 화면에 한해 영구히 닫는 것이고, `CYS_READINESS_V1=1`(모달 폴백 off)에서는 보류가
    /// **주입 허용**으로 뒤집힌다 — 실패 방향의 역전이다(정본 §3-3).
    pub const NON_GATE_SCREENS: &[(&str, &str)] = &[
        ("live-tui-2.1.261-nbsp-prompt", LIVE_TUI_2_1_261_NBSP_PROMPT),
        ("ready-shell", READY_SHELL),
        ("healthy-welcome-box", HEALTHY_WELCOME_BOX),
        ("live-tui-at-prompt", LIVE_TUI_AT_PROMPT),
        ("live-tui-2.1.261-status-below-prompt", LIVE_TUI_2_1_261_STATUS_BELOW_PROMPT),
        ("foreign-cli-browser-login", FOREIGN_CLI_BROWSER_LOGIN),
        ("grep-output", GREP_OUTPUT_MENTIONING_OAUTH_ERROR),
        ("audit-log-line", AUDIT_LOG_LINE),
        ("live-permission-prompt", LIVE_PERMISSION_PROMPT),
        ("config-theme-setting", CONFIG_THEME_SETTING),
        ("account-status-panel", ACCOUNT_STATUS_PANEL),
        ("doc-mentioning-oauth-url", DOC_MENTIONING_OAUTH_URL),
    ];

    /// ★코퍼스 **밖**의 첫기동 벤더 모달(실측) — [`NON_GATE_SCREENS`] 와 **다른 표**다.
    ///
    /// 【두 표가 왜 갈라져 있는가】 위 표는 프로덕션 수리기의 입력이고(그 화면에 걸리는 needle 은
    /// 제거된다), 이 표는 **검체 전용**이다. 여기 실린 화면은 "정상 화면" 이 아니라 **코퍼스가
    /// 아직 모르는 모달**이며, 그 화면에서 옳은 귀결은 주입이 아니라 **보류**다. 그러므로
    ///   ⓐ [`super::needle_non_gate_hits`] 는 이 표를 **보지 않는다**(보면 그 문면으로 관문을
    ///      선언한 운영자의 needle 이 조용히 지워진다 — 리뷰 R1 codex 지적).
    ///   ⓑ 대신 검체가 "어떤 **빌트인** needle 도 이 화면에 단독으로 걸리지 않는다"(오탐 0)와
    ///      "판정은 `unknown-modal` 보류"(미탐 0)를 **직접** 집행한다
    ///      (`builtin_needles_never_hit_an_out_of_corpus_modal` ·
    ///       readiness `non_gate_screens_through_judge_hold_only_true_modals`).
    ///
    /// 두 표의 id 는 겹치지 않는다(`the_two_screen_tables_are_disjoint` 가 집행).
    pub const MEASURED_NON_CORPUS_MODALS: &[(&str, &str)] = &[
        ("custom-api-key-modal-2.1.261", CUSTOM_API_KEY_MODAL_2_1_261),
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

    /// ★두 화면 표는 **겹치지 않는다** — 그리고 코퍼스 밖 모달은 프로덕션 수리기의 입력이 아니다.
    ///
    /// 【무엇을 막는가 — 0.14.31 리뷰 R1 · codex 지적】 [`fixtures::NON_GATE_SCREENS`] 는 검체
    /// 자료가 아니라 [`needle_non_gate_hits`] → [`gate_rule_violations`] → [`repair_gate`] 로
    /// 이어지는 **운영 입력**이다. 첫기동에 뜨는 코퍼스 밖 모달을 거기 등재하면, 그 화면 문면을
    /// needle 로 선언한 운영자의 관문이 런타임에 **needle 을 잃고 식별 불능**이 된다 — 문서화된
    /// 탈출구(agents.json 봉투)가 그 화면에 한해 영구히 닫히고, `CYS_READINESS_V1=1`(모달 폴백
    /// off)에서는 보류가 **주입 허용**으로 뒤집힌다. 실패 방향의 역전이다(정본 §3-3).
    #[test]
    fn out_of_corpus_modals_are_not_production_repair_input() {
        // ⓐ 두 표는 id 가 겹치지 않는다.
        for &(mid, _) in fixtures::MEASURED_NON_CORPUS_MODALS {
            assert!(
                !fixtures::NON_GATE_SCREENS.iter().any(|&(sid, _)| sid == mid),
                "{mid}: 코퍼스 밖 모달이 '관문이 아닌 화면' 표에도 있다 — 그 문면으로 선언한 \
                 사용자 관문의 needle 이 조용히 제거된다"
            );
        }
        // ⓑ 수리기의 판정 핵이 모달 표를 **보지 않는다**(그 문면은 needle 위반이 아니다).
        for &(mid, screen) in fixtures::MEASURED_NON_CORPUS_MODALS {
            for line in screen.lines().map(str::trim).filter(|l| l.ends_with('?')) {
                assert!(
                    needle_non_gate_hits(line).is_empty(),
                    "{mid}: 모달의 질문 문면 {line:?} 이 '정상 화면에 걸린다' 로 판정된다"
                );
            }
        }
        // ⓒ 그래도 **빌트인** needle 은 그 화면에 걸리지 않는다(오탐 0 — 등재의 원래 목적).
        for g in builtin() {
            for n in &g.needles {
                for &(mid, screen) in fixtures::MEASURED_NON_CORPUS_MODALS {
                    let (norm, flat) = (normalize(screen), flatten(screen));
                    assert!(
                        !norm.contains(&normalize(n)) && !flat.contains(&flatten(n)),
                        "{}: needle {n:?} 이 코퍼스 밖 모달 {mid} 에 걸린다",
                        g.id
                    );
                }
            }
        }
        // ⓓ 그리고 코퍼스는 그 화면을 **관문으로 식별하지 않는다**(미탐이 아니라 '모르는 화면').
        for &(mid, screen) in fixtures::MEASURED_NON_CORPUS_MODALS {
            assert!(identify(&builtin(), screen).is_none(), "{mid}: 코퍼스 밖 모달을 오탐했다");
        }
    }

    /// ★반례 — 운영자가 **그 모달을 관문으로 선언하면 그것은 살아남는다**(0.14.31 · 리뷰 R1).
    ///
    /// 위 표 분리가 실제로 무엇을 되살렸는지를 재는 검체다. 분리 전에는 이 선언의 needle 이
    /// `repair_gate` 의 사용자 관문 팔에서 제거돼(남은 needle 0건) 관문이 아무 화면도 식별하지
    /// 못했다 — 선언은 코퍼스에 남는데 이빨이 없는, 가장 나쁜 종류의 조용한 무력화다.
    #[test]
    fn an_operator_can_declare_the_out_of_corpus_modal_as_a_gate() {
        let question = "Do you want to use this API key?";
        let env = serde_json::json!({"gates": [{
            "id": "custom-api-key",
            "title": "커스텀 API 키 확인",
            "needles": [question],
            "widget": ["No (recommended)", "Yes"],
            "passability": "human_only",
            "human_reason": "자격증명 판단은 사람 몫이다",
            "absence_cost": "fatal",
        }]});
        let r = resolve_with(Some(&env), true);
        let g = r.gates.iter().find(|g| g.id == "custom-api-key").expect("선언이 사라졌다");
        assert_eq!(g.needles, vec![question.to_string()], "운영자 needle 이 제거됐다(조용한 무력화)");
        assert_eq!(
            identify(&r.gates, fixtures::CUSTOM_API_KEY_MODAL_2_1_261).map(|g| g.id.as_str()),
            Some("custom-api-key"),
            "선언은 남았는데 실측 화면을 식별하지 못한다 — 이빨 없는 관문이다"
        );
        assert_eq!(g.passability, Passability::HumanOnly);
        assert!(g.action.is_none());
    }

    /// ★(0.14.31 · 성찰 R3 · blocking) **공백뿐인 needle 은 모든 화면을 관문으로 만들 수 없다.**
    ///
    /// 【사슬】 `"   "` needle → [`normalize`]/[`flatten`] 이 빈 문자열 → `contains("")` 가 **모든
    /// 화면에 참** → `identify` 상시 참 → ① `judge` 가 전 좌석·전 틱 `GateHeld`(디렉티브 영구
    /// 미주입 = 노드 0 + 고아 좌석) ② `inject_guard::decide` 가 항상 `Hold`(pack-update 재주입
    /// 영구 미도달) ③ `CYS_GATE_PENDING_CLOSE=1` 기계에서 `boot_verdict_effective` 가
    /// `LaunchFailed` 로 강등 = **모든 pane 사망**(§7 부트체인 재난표 전량).
    /// 그리고 종전 `needle_non_gate_hits` 는 빈 문면을 **면제**했으므로 `notes` 가 정반대를
    /// 찍었다("needle 축은 정상 화면 대조를 이미 통과했다").
    ///
    /// 【재는 것 — 셋 다 AND】 공백·탭·NBSP 3변형이
    ///   ⓐ 파서에서 사라지거나(`str_vec` 의 `trim`), 남더라도
    ///   ⓑ `matches(정상 화면) == false` 이고,
    ///   ⓒ 규칙 위반으로 **말해진다**([`needle_non_gate_hits`] 가 대조군 전량을 돌려준다 →
    ///      `gate_rule_violations` 비지 않음 → `repair_gate` 가 사유를 `notes` 에 남긴다).
    #[test]
    fn blank_needle_cannot_match_every_screen() {
        const BLANKS: [(&str, &str); 3] = [("공백", "   "), ("탭", "\t\t"), ("NBSP", "\u{a0}\u{a0}")];
        for (name, blank) in BLANKS {
            // ⓒ 규칙 축 — 면제가 아니라 **대조군 전량**이 걸린다(수리가 그 항목만 지우는 근거).
            assert_eq!(
                needle_non_gate_hits(blank).len(),
                fixtures::NON_GATE_SCREENS.len(),
                "{name}: 공백 needle 이 규칙 위반으로 세어지지 않는다(종전 면제 회귀)"
            );
            // ⓑ 판정 축 — 손으로 조립한 관문(봉투를 거치지 않는 경로)도 정상 화면을 잡지 못한다.
            let g = Gate {
                needles: vec![blank.to_string()],
                widget: Vec::new(),
                ..builtin().into_iter().next().expect("코드 정본이 비었다")
            };
            for &(sid, screen) in fixtures::NON_GATE_SCREENS {
                assert!(
                    !g.matches(screen),
                    "{name}: 공백 needle 이 정상 화면 {sid} 를 관문으로 만든다(영구 부트 라이브락)"
                );
            }
            // ⓐ 파서 축 — 공백 항목은 사라진다. 그것만 선언하면 신설 선언 자체가 거절된다.
            let env = serde_json::json!({"gates": [
                {"id": "blank-only", "title": "공백 needle 뿐", "needles": [blank]},
                {"id": "blank-mixed", "title": "공백 + 실문면", "needles": [blank, "Do you trust the files in this folder"]},
            ]});
            let r = resolve_with(Some(&env), true);
            assert!(
                r.gates.iter().all(|g| g.id != "blank-only"),
                "{name}: needle 이 공백뿐인 선언이 코퍼스에 들어왔다"
            );
            let mixed = r.gates.iter().find(|g| g.id == "blank-mixed").expect("혼합 선언이 사라졌다");
            assert!(
                mixed.needles.iter().all(|n| !n.trim().is_empty()),
                "{name}: 혼합 선언에서 공백 needle 이 살아남았다: {:?}",
                mixed.needles
            );
            // 그리고 그 관문은 여전히 **정상 화면을 잡지 않는다**(수리가 이빨을 남겼는지).
            for &(sid, screen) in fixtures::NON_GATE_SCREENS {
                assert!(
                    !mixed.matches(screen),
                    "{name}: 혼합 선언이 정상 화면 {sid} 를 관문으로 만든다"
                );
            }
        }
        // ★사유가 **말해진다** — 빌트인 id 로 공백 needle 을 덮으면 정본 needle 로 복원되고 note 가 남는다.
        let victim = builtin().into_iter().next().expect("코드 정본이 비었다");
        let mut notes = Vec::new();
        let repaired = repair_gate(
            Gate { needles: vec!["   ".to_string()], ..victim.clone() },
            &builtin(),
            &mut notes,
        );
        assert_eq!(repaired.needles, victim.needles, "빌트인 대응물의 정본 needle 로 복원되지 않았다");
        assert!(
            notes.iter().any(|n| n.contains(&victim.id)),
            "공백 needle 을 고치고도 사유를 남기지 않았다(조용한 무력화): {notes:?}"
        );
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
                cli_versions: &[],
                version_pin_legacy: false,
                guard_off: false,
                readiness_legacy: false,
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


    // ══════════════════════════════════════════════════════════════════════
    // ★2026-09-08 실측 회차(WP-1 H-2) — 측정 기록과 보고서 봉투
    // ══════════════════════════════════════════════════════════════════════

    /// 실측 화면에서 **기본 포커스 번호**를 뽑는다(손으로 옮겨 적지 않는다 · 화면이 SOT).
    /// `❯` 뒤 첫 정수. 번호 없는 위젯(2.1.261 커스텀 API 키 창)은 `None`.
    fn measured_default_index(screen: &str) -> Option<u8> {
        let line = screen.lines().find(|l| l.trim_start().starts_with('❯'))?;
        let rest = line.trim_start().trim_start_matches('❯').trim_start();
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        digits.parse().ok()
    }

    /// **이 회차의 실측이 선언값을 하나도 움직이지 않았다**는 기계 기록.
    ///
    /// 【무엇을 쟀나】 버려 쓰는 `CLAUDE_CONFIG_DIR` + 임시 소켓 데몬의 PTY 좌석에서 claude
    /// **2.1.261** 을 띄워 `theme` · `login-method` 두 화면을 읽었다(2.1.263 도 같은 두 화면 동형).
    ///
    /// 【무엇을 못 쟀나 · 왜】 `login-method` 는 [`Passability::HumanOnly`] 다 — 액션 선언이
    /// **없으므로** 기계가 넘을 키가 없다. 그 벽 뒤의 `oauth-code`·`folder-trust`·
    /// `bypass-disclaimer`·`feature-announce-fullscreen` 은 **보지 못했다**. 새 config dir 은
    /// 자격증명이 없고(mac Keychain 은 config dir 절대경로로 봉인된다), 살아 있는 계정 dir 을
    /// 빌려 오는 것은 설치본 무접촉 규약 위반이다.
    ///
    /// 【그래서 무엇을 안 했나】 정본 §4 H-2 는 "6관문 전부 실측한 뒤에만" [`MEASURED_ON`] 을
    /// 올리라고 한다. 부분 실측이므로 **올리지 않았고**, CONTRACTS §B-7 이 승인해 둔 folder-trust
    /// 재핀(`down_presses()` `Some(0)` → `Some(1)`)도 **하지 않았다**. 승인은 실측의 대체물이
    /// 아니다 — 그 값이 틀리면 Return 한 발이 `No, exit` 를 눌러 좌석이 rc 1 로 죽는다(§7 ④).
    ///
    /// 다음 회차가 남은 4관문을 실측하면 이 검체를 **의도적으로** 고치게 된다. 조용히 바뀌지
    /// 않는 것이 이 검체의 목적이다.
    #[test]
    fn the_2026_09_08_partial_measurement_moved_no_declared_value() {
        let gs = builtin();
        let g = |id: &str| gs.iter().find(|g| g.id == id).expect(id).clone();

        // ── ① 실측한 두 관문: 화면이 여전히 그 관문으로 식별되고, 기본 포커스가 선언과 같다 ──
        for (name, screen) in [
            ("2.1.261", fixtures::THEME_2_1_261),
            ("2.1.263", fixtures::THEME_2_1_263),
        ] {
            let hit = identify(&gs, screen).unwrap_or_else(|| panic!("{name}: theme 미식별\n{screen}"));
            assert_eq!(hit.id, "theme", "{name}: 다른 관문으로 읽혔다");
            assert_eq!(
                measured_default_index(screen),
                g("theme").default_index,
                "{name}: 실측 기본 포커스가 선언과 다르다 — 선언을 고쳐야 한다"
            );
        }
        // theme 는 기본 포커스가 곧 통과 액션이다(아래키 0회) — 실측이 이것을 **확인**했다.
        assert_eq!(g("theme").down_presses(), Some(0));
        assert_eq!(g("theme").action.as_ref().map(|a| a.select_index), Some(2));

        let login = identify(&gs, fixtures::LOGIN_METHOD_2_1_261).expect("login-method 미식별");
        assert_eq!(login.id, "login-method");
        assert_eq!(
            measured_default_index(fixtures::LOGIN_METHOD_2_1_261),
            login.default_index,
            "로그인 관문 기본 포커스 실측 불일치"
        );
        // ★기계가 넘을 수 없다는 것이 **측정 결과**다 — 이것이 아래 ③의 미실측 사유다.
        assert_eq!(login.passability, Passability::HumanOnly);
        assert!(login.action.is_none(), "HumanOnly 관문에 액션이 생겼다");
        assert!(matches!(
            action_policy(login, Some("2.1.261")),
            ActionPolicy::HumanRequired { .. }
        ));

        // ── ② 코퍼스 밖 벤더 모달(실측) — 어떤 관문으로도 식별되지 않는다 ──────────────
        assert!(
            identify(&gs, fixtures::CUSTOM_API_KEY_MODAL_2_1_261).is_none(),
            "코퍼스가 모르는 벤더 모달을 관문으로 오탐했다"
        );
        assert_eq!(
            measured_default_index(fixtures::CUSTOM_API_KEY_MODAL_2_1_261),
            None,
            "번호 없는 선택 위젯인데 번호가 읽혔다(전사 오류)"
        );

        // ── ③ 못 본 관문의 선언은 종전 그대로다(핀 미갱신) ──────────────────────────
        assert_eq!(MEASURED_ON, "2.1.241", "부분 실측인데 버전 핀이 올라갔다");
        let trust = g("folder-trust");
        assert_eq!(trust.default_index, Some(1), "미실측 관문의 기본 포커스가 바뀌었다");
        assert_eq!(trust.action.as_ref().map(|a| a.select_index), Some(1));
        assert_eq!(trust.down_presses(), Some(0), "CONTRACTS §B-7 재핀은 실측 전에는 하지 않는다");
        assert_eq!(g("bypass-disclaimer").default_index, Some(1));
        assert_eq!(g("feature-announce-fullscreen").default_index, Some(1));
    }

    /// 보고서(`cys gate-corpus --json`)는 **코퍼스의 어휘**로 쓴다 — 봉투 파서가 되읽을 수 있다.
    ///
    /// 방언이면 보고서를 그대로 `agents.json` 봉투에 되먹일 수 없고, 그 순간 진단과 정정이
    /// 다른 언어를 쓰게 된다(운영자가 본 값과 고치는 값이 다르다).
    #[test]
    fn gate_corpus_report_speaks_the_corpus_vocabulary_and_carries_the_contract_keys() {
        let r = resolve_with(None, true);
        let v = report_json(&r, "claude", None, None);

        // CONTRACTS §C 최소 계약: 최상위 measured_on · gates[].{id,passability}
        assert_eq!(v["measured_on"].as_str(), Some(MEASURED_ON));
        let gates = v["gates"].as_array().expect("gates 배열");
        assert_eq!(gates.len(), builtin().len(), "관문 수가 다르다");
        for (i, gv) in gates.iter().enumerate() {
            let id = gv["id"].as_str().expect("id");
            assert!(!id.is_empty());
            // 어휘 왕복 — 봉투 파서가 이 문자열을 되읽는다(사본 0).
            assert_eq!(
                parse_passability(gv.get("passability")),
                Some(builtin()[i].passability),
                "{id}: passability 어휘가 봉투 파서와 갈렸다"
            );
            assert_eq!(
                parse_absence_cost(gv.get("absence_cost")),
                Some(builtin()[i].absence_cost),
                "{id}: absence_cost 어휘가 봉투 파서와 갈렸다"
            );
            assert!(gv.get("policy").is_none(), "{id}: 묻지 않은 버전 정책이 인쇄됐다");
        }
        // 빌트인만이면 전 관문이 같은 버전을 주장한다 → 유효 버전이 잡히고 혼합이 아니다.
        assert_eq!(v["effective_measured_on"].as_str(), Some(MEASURED_ON));
        assert_eq!(v["mixed_versions"].as_bool(), Some(false));
        assert_eq!(v["detected_version"], Value::Null);
        assert_eq!(v["source"].as_str(), Some("builtin"));
        assert_eq!(v["agent"].as_str(), Some("claude"));

        // 실측 대장의 아래키·라벨이 보고서에 그대로 실린다(운영자가 코드를 안 읽어도 된다).
        let theme = gates.iter().find(|g| g["id"] == "theme").expect("theme");
        assert_eq!(theme["down_presses"].as_u64(), Some(0));
        assert_eq!(theme["action"]["label"].as_str(), Some("Dark mode"));
        assert_eq!(theme["default_index"].as_u64(), Some(2));
        let login = gates.iter().find(|g| g["id"] == "login-method").expect("login");
        assert_eq!(login["action"], Value::Null);
        assert_eq!(login["down_presses"], Value::Null, "액션 없는 관문에 아래키가 생겼다");
        assert_eq!(login["absence_is_fatal"].as_bool(), Some(true));
        assert!(login["human_reason"].as_str().is_some_and(|s| s.contains("OAuth")));

        // ★(0.14.31 · 리뷰 R1) **식별 축**이 실린다 — 없으면 보고서를 되먹일 수 없다.
        for (i, gv) in gates.iter().enumerate() {
            let want = &builtin()[i];
            assert_eq!(
                gv["needles"].as_array().map(|a| a.len()),
                Some(want.needles.len()),
                "{}: needles 축이 보고서에 없다(되먹이면 관문이 소멸한다)",
                want.id
            );
            assert_eq!(gv["widget"].as_array().map(|a| a.len()), Some(want.widget.len()));
            assert_eq!(
                gv["confirm_echo"].as_array().map(|a| a.len()),
                Some(want.confirm_echo.len())
            );
        }
        // ★그리고 `policy` 열이 **어디까지 집행되는가**를 보고서 자신이 싣는다.
        //   ★(0.14.31 · 독립 재유도 H2-B 재핀) 종전 핀은 bool 한 축(`enforced`)만 봤다. 확인 경계가
        //     **불일치 팔만** 집행하기 시작했으므로 bool 하나로는 어느 값도 참이 아니다 — 상태와
        //     축 표를 함께 못박는다(축이 늘 때 산출물이 조용히 뒤처지지 않게).
        assert_eq!(
            v["policy_enforcement"]["state"].as_str(),
            Some(ACTION_POLICY_ENFORCEMENT.as_str())
        );
        assert_eq!(
            v["policy_enforcement"]["enforced"].as_bool(),
            Some(ACTION_POLICY_ENFORCEMENT == PolicyEnforcement::Full),
            "부분 집행이 '전량 집행' 으로 인쇄됐다"
        );
        assert_eq!(
            v["policy_enforcement"]["axes"]["version_drift"].as_bool(),
            Some(ACTION_POLICY_ENFORCEMENT.denies_version_drift())
        );
        assert_eq!(
            v["policy_enforcement"]["axes"]["version_unknown"].as_bool(),
            Some(ACTION_POLICY_ENFORCEMENT.denies_version_unknown())
        );
        assert_eq!(
            v["policy_enforcement"]["axes"]["down_bundle"].as_bool(),
            Some(ACTION_POLICY_ENFORCEMENT.sends_down_bundle())
        );
        assert!(v["policy_enforcement"]["evidence"].as_str().is_some_and(|s| !s.is_empty()));
        assert_eq!(v["policy_enforcement"]["scope"].as_str(), Some("cli-auto-confirm"));
        assert!(v["policy_enforcement"]["note"].as_str().is_some_and(|s| !s.is_empty()));
        // ★되먹임 재료는 `source`(출처)가 아니라 `override_envelope`(모드를 스스로 싣는 봉투)다.
        assert_eq!(v["override_envelope"]["source"].as_str(), Some("builtin"));
        assert_eq!(
            v["override_envelope"]["gates"].as_array().map(|a| a.len()),
            Some(builtin().len())
        );
        assert!(
            v["override_envelope"]["gates"][0].get("origin").is_none(),
            "봉투에 파생 축(origin)이 섞였다 — 봉투는 선언 축만 싣는다"
        );
    }

    /// 혼합 버전은 **추측하지 않는다** — 유효 버전은 `null`, 내장 핀은 그대로.
    ///
    /// 봉투가 관문 **일부**만 새 버전으로 덮으면 코퍼스는 두 버전을 동시에 주장한다. 그때
    /// 최상위 한 값을 고르면 그 값은 어느 쪽으로 읽어도 거짓이다(codex 설계 검토 Q4).
    #[test]
    fn gate_corpus_report_reports_mixed_versions_as_null_instead_of_guessing() {
        let env = serde_json::json!({
            "gates": [{"id": "theme", "measured_on": "9.9.9"}]
        });
        let r = resolve_with(Some(&env), true);
        let v = report_json(&r, "claude", None, None);
        assert_eq!(v["measured_on"].as_str(), Some(MEASURED_ON), "내장 핀은 상태에 따라 흔들리지 않는다");
        assert_eq!(v["effective_measured_on"], Value::Null);
        assert_eq!(v["mixed_versions"].as_bool(), Some(true));
        let theme = v["gates"].as_array().unwrap().iter().find(|g| g["id"] == "theme").unwrap().clone();
        assert_eq!(theme["measured_on"].as_str(), Some("9.9.9"));
        assert_eq!(theme["origin"].as_str(), Some("overridden"));
        // 전 관문을 같은 버전으로 덮으면 다시 하나로 접힌다(대조군).
        // ★(0.14.31 · 리뷰 R1) 종전 대조군은 `gates: []` 였다 — 봉투 최상위 `measured_on` 은
        //   **선언된 관문의 기본값**일 뿐이라(`resolve_raw`) 선언이 비면 아무 관문도 덮이지 않는다.
        //   그러면 `mixed_versions == false` 는 "같은 버전으로 덮으면 접힌다" 가 아니라 "아무것도
        //   안 바뀌었다" 를 확인할 뿐이다(공허한 초록). 6관문 **전부**를 선언에 넣어 실제로 덮는다.
        let decls: Vec<Value> = builtin()
            .iter()
            .map(|g| serde_json::json!({"id": g.id}))
            .collect();
        let all = serde_json::json!({"measured_on": "9.9.9", "source": "builtin", "gates": decls});
        let r2 = resolve_with(Some(&all), true);
        let v2 = report_json(&r2, "claude", None, None);
        assert_eq!(v2["mixed_versions"].as_bool(), Some(false));
        assert_eq!(
            v2["effective_measured_on"].as_str(),
            Some("9.9.9"),
            "전 관문을 덮었는데 유효 버전이 접히지 않았다(대조군이 실제로 덮지 못했다)"
        );
        for gv in v2["gates"].as_array().unwrap() {
            assert_eq!(gv["measured_on"].as_str(), Some("9.9.9"), "{}: 덮이지 않았다", gv["id"]);
        }
        assert_eq!(v2["measured_on"].as_str(), Some(MEASURED_ON), "내장 핀이 봉투를 따라갔다");
    }

    /// 버전 정책은 **물었을 때만** 답한다 — 그리고 그 답은 [`ActionPolicy`] 타입 그대로다.
    #[test]
    fn gate_corpus_report_carries_the_version_policy_only_when_a_version_is_supplied() {
        let r = resolve_with(None, true);
        let pick = |v: &Value, id: &str| {
            v["gates"].as_array().unwrap().iter().find(|g| g["id"] == id).unwrap().clone()
        };

        let same = report_json(&r, "claude", Some(MEASURED_ON), None);
        assert_eq!(same["detected_version"].as_str(), Some(MEASURED_ON));
        assert_eq!(pick(&same, "theme")["policy"]["kind"].as_str(), Some("allowed"));
        assert_eq!(pick(&same, "theme")["policy"]["down"].as_u64(), Some(0));
        // HumanOnly 는 버전과 무관하게 사람 몫이다(정책 우선순위 · 회귀 잠금).
        assert_eq!(pick(&same, "login-method")["policy"]["kind"].as_str(), Some("human_required"));

        // ★이 기계의 현행 설치본(2.1.263)에서 코퍼스가 서는 자리 — 액션은 전부 보류다.
        let drift = report_json(&r, "claude", Some("2.1.263"), None);
        let t = pick(&drift, "theme");
        assert_eq!(t["policy"]["kind"].as_str(), Some("held_version_drift"));
        assert_eq!(t["policy"]["measured_on"].as_str(), Some(MEASURED_ON));
        assert_eq!(t["policy"]["detected"].as_str(), Some("2.1.263"));
        for gv in drift["gates"].as_array().unwrap() {
            assert_ne!(
                gv["policy"]["kind"].as_str(),
                Some("allowed"),
                "{}: 버전 드리프트에서 액션이 열렸다(위젯 서명 우회 금지 · §8)",
                gv["id"]
            );
        }
    }

    /// ★(0.14.31 · 리뷰 R1) 보고서의 `override_envelope` 를 그대로 되먹이면 **같은 코퍼스**가 선다 —
    /// 빌트인 · 병합(사용자 신설 관문 포함) · 교체 **세 형상 전부**.
    ///
    /// 【종전 검체가 못 보던 것】 첫 판은 보고서 최상위 `source`(출처 어휘)를 봉투 모드로 그대로
    /// 넣고 `Source::Builtin` 한 형상만 돌렸다. `"builtin"` 은 우연히 두 어휘에 모두 있어 초록이었고,
    /// `"replaced"` 는 미지 값 → 병합 강등, 게다가 보고서가 needle 을 안 실어 사용자 관문이 소멸하는
    /// 경로를 **한 번도 지나지 않았다**. 그리고 base 를 언제나 빌트인 위에 세워서 "무시된 축이 늘 같아
    /// 보이는" 착시가 있었다(needles·widget·confirm_echo·human_reason 축 주장이 공허했다).
    /// 여기서는 ⓐ 되먹임 재료를 `override_envelope` 로 바꾸고 ⓑ base 를 세 형상으로 갈라
    /// **빌트인과 다른 값**(사용자 needle·위젯·에코·사유)을 실제로 통과시킨다.
    #[test]
    fn gate_corpus_override_envelope_round_trips_builtin_merged_and_replaced_corpora() {
        // 사용자 신설 관문 — 모든 선언 축이 빌트인과 **다르다**(무시된 축이 초록으로 보이지 않게).
        let user_gate = || {
            serde_json::json!({
                "id": "only",
                "title": "(운영자 선언 관문)",
                "needles": ["Do you want to hand over the wheel?"],
                "widget": ["Hand over", "Keep driving"],
                "confirm_echo": ["Hand over ✔"],
                "human_reason": "운영자가 적어 둔 사유",
                "passability": "human_only",
                "absence_cost": "fatal",
                "measured_on": "9.9.9",
            })
        };
        let merged_env = serde_json::json!({"source": "builtin", "gates": [user_gate()]});
        let replaced_env = serde_json::json!({"source": "replace", "gates": [user_gate()]});

        for (shape, base) in [
            ("builtin", resolve_with(None, true)),
            ("merged", resolve_with(Some(&merged_env), true)),
            ("replaced", resolve_with(Some(&replaced_env), true)),
        ] {
            let report = report_json(&base, "claude", None, None);
            let env = report
                .get("override_envelope")
                .unwrap_or_else(|| panic!("{shape}: 붙여 넣을 봉투 축이 없다"))
                .clone();
            // 봉투는 **모드를 스스로 싣는다** — 보고서 출처 어휘를 되먹이는 것이 아니다.
            assert_eq!(
                env["source"].as_str(),
                Some(if shape == "replaced" { "replace" } else { "builtin" }),
                "{shape}: 봉투가 자기 모드를 잘못 싣는다"
            );
            let round = resolve_with(Some(&env), true);
            assert_eq!(
                round.gates.iter().map(|g| &g.id).collect::<Vec<_>>(),
                base.gates.iter().map(|g| &g.id).collect::<Vec<_>>(),
                "{shape}: 관문 id 목록이 되먹임 전후 달라졌다(누락·추가·순서 변경)"
            );
            for (actual, expected) in round.gates.iter().zip(&base.gates) {
                macro_rules! same_axis {
                    ($axis:ident) => {
                        assert_eq!(
                            actual.$axis, expected.$axis,
                            "{}: 관문 {} 의 {} 축이 되먹임 전후 달라졌다",
                            shape, expected.id, stringify!($axis)
                        );
                    };
                }
                same_axis!(id);
                same_axis!(title);
                same_axis!(needles);
                same_axis!(widget);
                same_axis!(confirm_echo);
                same_axis!(passability);
                same_axis!(default_index);
                same_axis!(human_reason);
                same_axis!(absence_cost);
                same_axis!(measured_on);
                same_axis!(action);
                // `origin` 은 해소 경로의 기록이라 되먹임에서 바뀐다(선언 축이 아니다) — 제외.
            }
        }

        // ★그리고 **운영자의 관문이 살아남았다** — 이것이 리뷰가 지목한 소멸 경로의 반례다.
        let replaced = resolve_with(Some(&replaced_env), true);
        let report = report_json(&replaced, "claude", None, None);
        let round = resolve_with(Some(&report["override_envelope"]), true);
        let only = round
            .gates
            .iter()
            .find(|g| g.id == "only")
            .expect("되먹임 뒤 운영자 관문이 사라졌다(BLOCK-3 형태)");
        assert_eq!(only.needles, vec!["Do you want to hand over the wheel?".to_string()]);
        assert_eq!(only.passability, Passability::HumanOnly);
        assert_eq!(only.measured_on, "9.9.9");
    }

    /// ★보고서 **출처** 어휘를 봉투 **모드**로 읽지 않는다 — 그리고 조용히 접지도 않는다.
    ///
    /// 종전엔 `"replaced"` 가 미지 값으로 접혀 replace 가 병합으로 뒤집혔다(운영자는 무엇이
    /// 일어났는지 알 길이 없었다). 별칭으로 고치면 **새 권한**이 열린다(codex 설계 검토 R1):
    /// 같은 입력이 교체가 되면 편집 중 빠뜨린 관문이 사라지고, [`parse_new_gate`] 는
    /// [`apply_patch`] 와 달리 `human_only → machine` 승격을 스스로 막지 않는다. 그래서 모드는
    /// 넓히지 않고 **병합에 착지시킨 뒤 붙여 넣을 자리를 이름으로 지목**한다.
    #[test]
    fn report_source_words_land_on_merge_and_name_the_envelope_to_paste() {
        for word in ["replaced", "merged", "override_disabled", "spec_unreadable"] {
            let env = serde_json::json!({
                "source": word,
                "gates": [{"id": "x-gate", "needles": ["Do you want to keep going?"],
                           "widget": ["Keep going", "Stop here"]}],
            });
            let r = resolve_with(Some(&env), true);
            assert!(
                r.gates.iter().any(|g| g.id == "theme"),
                "{word}: 보고서 어휘가 교체로 읽혀 빌트인 관문이 사라졌다"
            );
            assert!(r.gates.iter().any(|g| g.id == "x-gate"), "{word}: 선언이 도달하지 않았다");
            assert!(
                r.notes.iter().any(|n| n.contains("override_envelope")),
                "{word}: 무엇을 붙여 넣어야 하는지 말하지 않는다(조용한 강등): {:?}",
                r.notes
            );
        }
        // 진짜 미지 값은 종전 문면 그대로다(보고서 어휘와 구별한다).
        let unknown = serde_json::json!({"source": "wat", "gates": []});
        let r = resolve_with(Some(&unknown), true);
        assert!(r.notes.iter().any(|n| n.contains("미지 값")), "{:?}", r.notes);
        assert!(!r.notes.iter().any(|n| n.contains("override_envelope")));
    }

    /// ★[`envelope_mode`] 전수 — **교체를 내는 입력은 정확히 하나**이고, **조용한 강등이 없다**.
    ///
    /// (codex gpt-6-astra 위임 산출 · 전행 검토 후 채택 — `impl/codex/R5-WP1-H2-r1-envelope-mode-test.md`.
    ///  중복 단언 2개는 걷어냈다.)
    ///
    /// 위 검체가 재는 것은 "보고서 어휘가 병합에 착지한다" 는 **몇 가지 사례**다. 이것은 그 위의
    /// 성질을 전수로 못박는다 — 새 어휘를 어느 표에 넣든 ⓐ 교체로 새는 입력이 `replace` 말고 하나도
    /// 없고(교체는 선언되지 않은 관문을 지울 수 있는 유일한 모드다) ⓑ 뜻을 정확히 아는 입력
    /// (`replace`·`builtin`·부재)이 아니면 **반드시 사유가 남는다**(운영자가 강등을 모르고 지나가지
    /// 않는다). 그리고 진단은 **덧붙기만** 한다(앞선 사유를 지우면 먼저 난 문제가 묻힌다).
    #[test]
    fn envelope_mode_is_a_total_function_that_only_widens_toward_merge() {
        let mut inputs = vec![
            None,
            Some(""),
            Some("   "),
            Some("replace"),
            // ★(리뷰 R2) 공백 변형은 **교체가 아니다** — 관용은 파괴적인 쪽으로 베풀지 않는다.
            Some(" replace "),
            Some("replace "),
            Some("\treplace"),
            Some(" builtin "),
            Some("Replace"), // 대문자 변형은 교체가 아니다(정확 일치만)
            Some("builtin"),
        ];
        inputs.extend(REPORT_SOURCE_WORDS.iter().copied().map(Some));
        inputs.extend([Some("unknown"), Some("merge")]);

        let mut notes = vec![String::from("기존 진단")];
        for input in inputs {
            // ★모드 어휘는 **정확 일치**다(공백 흡수 없음 · 리뷰 R2). 부재만 기본값 builtin 이다.
            let raw = input.unwrap_or("builtin");
            let is_replace = raw == "replace";
            let is_known = matches!(raw, "replace" | "builtin");
            let is_whitespace_variant =
                !is_known && matches!(raw.trim(), "replace" | "builtin");
            let previous_notes = notes.clone();
            let before = notes.len();
            let mode = envelope_mode(input, &mut notes);

            assert_eq!(
                mode,
                if is_replace { EnvelopeMode::Replace } else { EnvelopeMode::Merge },
                "입력 {input:?}: **정확히** `replace` 만 교체여야 한다 — 그 밖의 입력이 교체로 새면 \
                 선언하지 않은 관문이 사라진다(공백 변형 포함 · 리뷰 R2)"
            );
            assert!(
                notes.starts_with(&previous_notes),
                "입력 {input:?}: 기존 진단을 지우거나 바꾼다 — 먼저 난 문제가 묻힌다"
            );
            let delta = notes.len() - before;
            assert_eq!(
                delta,
                usize::from(!is_known),
                "입력 {input:?}: 진단 증분이 예상과 다르다 — 0 이면 조용한 강등, 2 이상이면 중복 경고"
            );
            if !is_known {
                let note = &notes[before];
                let is_report_word = REPORT_SOURCE_WORDS.contains(&raw);
                assert_eq!(
                    note.contains("공백이 섞여"),
                    is_whitespace_variant,
                    "입력 {input:?}: 공백 변형과 그 밖의 강등이 같은 문면으로 접혔다(처방이 다르다)"
                );
                assert_eq!(
                    note.contains("override_envelope"),
                    is_report_word && !is_whitespace_variant,
                    "입력 {input:?}: 보고서 어휘라면 붙여 넣을 축을 이름으로 지목해야 한다"
                );
                assert_eq!(
                    note.contains("미지 값"),
                    !is_report_word && !is_whitespace_variant,
                    "입력 {input:?}: 보고서 어휘와 진짜 미지 값의 처방이 뒤섞였다"
                );
            }
        }
    }

    /// ★`source=replace` 도 **사람 1회 관문을 기계 통과로 승격시킬 수 없다**(0.14.31 · 리뷰 R1).
    ///
    /// 병합 경로([`apply_patch`])는 이 승격을 거부해 왔지만 교체 경로([`parse_new_gate`])에는 그
    /// 거부가 없었다. 그래서 봉투 한 줄이면 OAuth 화면에 키가 나갔다 — 좌석은 브라우저 대기에
    /// 갇힌 채 **살아 있어서** 생존만 보는 판정이 그것을 영원히 '준비됨' 으로 읽는다(비가역).
    #[test]
    fn replace_mode_cannot_promote_a_human_only_gate_to_machine() {
        let env = serde_json::json!({"source": "replace", "gates": [{
            "id": "login-method",
            "needles": ["Select login method"],
            "widget": ["Claude account with subscription", "Anthropic Console account"],
            "passability": "machine",
            "default_index": 1,
            "action": {"select_index": 1, "label": "Claude account with subscription"},
        }]});
        let r = resolve_with(Some(&env), true);
        let g = r.gates.iter().find(|g| g.id == "login-method").expect("관문이 사라졌다");
        assert_eq!(g.passability, Passability::HumanOnly, "승격이 통과했다(§8 위반 경로)");
        assert!(g.action.is_none(), "사람 1회 관문에 기계 액션이 남았다");
        assert!(g.human_reason.is_some(), "사람에게 보여줄 사유가 비었다");
        assert!(
            matches!(action_policy(g, Some(MEASURED_ON)), ActionPolicy::HumanRequired { .. }),
            "정책이 여전히 기계 통과를 낸다"
        );
        assert!(r.notes.iter().any(|n| n.contains("사람 1회")), "조용히 되돌렸다: {:?}", r.notes);
        // 병합 경로의 종전 거부도 그대로다(두 경로의 대칭).
        let merge = serde_json::json!({"gates": [{"id": "login-method", "passability": "machine"}]});
        let m = resolve_with(Some(&merge), true);
        let mg = m.gates.iter().find(|g| g.id == "login-method").unwrap();
        assert_eq!(mg.passability, Passability::HumanOnly);
    }

    /// ★(0.14.31 · WP-1 H-2 · 리뷰 R2) 권장 봉투의 무변화 되먹임은 조용하고, 실제 변경·거부는 남는다.
    ///
    /// 【무엇이 틀렸었는가】 보고서는 선언 축을 전량 싣는데, R1 은 원래 없는 기본 포커스의
    /// `default_index: null` 과 사람 전용 관문의 `action: null` 에도 매 왕복마다 오해성 3줄을 냈다.
    /// 그래서 권장 경로 그대로 두 번 되먹여 진단 0건과 관문 목록 보존을 값으로 잰다.
    ///
    /// 【왜 양성 대조군도 세는가】 진단을 통째로 없애도 무소음만 재는 검체는 초록이다. 실제 지움·
    /// 액션 무시·승격 거부·손상은 사유가 정확히 1건이어야 한다(누락과 중복 모두 결함).
    /// 특히 기계 관문을 사람 전용으로 바꾸면서 액션을 null 로 지우면, 패치 도중에는 이미 액션이
    /// 없어도 진입 시점에는 있었다. 이 반례가 비교 기준을 패치 진입 시점에 묶는다.
    /// (codex gpt-6-astra 위임 산출 · 전행 검토 후 채택 — `impl/codex/r2-tests/suppression_test.rs`.
    ///  전제 단언 한 줄만 문면을 고쳤다: 실패 메시지가 "빌트인 관문이 없다" 였는데 `expect` 는
    ///  **찾았을 때**의 값을 쓰므로 뜻이 뒤집혀 있었다.)
    #[test]
    fn feeding_the_recommended_envelope_back_is_silent_but_real_changes_are_not() {
        let base = resolve_with(None, true);
        assert_eq!(base.source, Source::Builtin, "전제: 코드 정본에서 출발해야 한다");
        let report = report_json(&base, "claude", None, None);
        assert!(report["override_envelope"].is_object(), "권장 되먹임 봉투가 없다");
        let first = resolve_with(Some(&report["override_envelope"]), true);
        assert!(
            first.notes.is_empty(),
            "무변화 권장 봉투가 오해성 진단을 냈다: {:?}",
            first.notes
        );
        assert_eq!(
            first.source,
            Source::Merged { overridden: base.gates.len(), added: 0 },
            "봉투를 실제로 병합하지 않고 무소음으로 보였다"
        );
        assert_eq!(
            first.gates.iter().map(|g| &g.id).collect::<Vec<_>>(),
            base.gates.iter().map(|g| &g.id).collect::<Vec<_>>(),
            "첫 되먹임이 정본 관문 목록을 바꿨다"
        );

        let second_report = report_json(&first, "claude", None, None);
        assert!(second_report["override_envelope"].is_object(), "두 번째 되먹임 봉투가 없다");
        let second = resolve_with(Some(&second_report["override_envelope"]), true);
        assert!(
            second.notes.is_empty(),
            "두 번째 무변화 되먹임이 다시 진단을 냈다: {:?}",
            second.notes
        );
        assert_eq!(second.source, first.source, "두 번째 되먹임의 병합 출처가 달라졌다");
        assert_eq!(
            second.gates.iter().map(|g| &g.id).collect::<Vec<_>>(),
            first.gates.iter().map(|g| &g.id).collect::<Vec<_>>(),
            "두 번째 되먹임이 관문 id 목록을 바꿨다(누락·추가·순서 변경)"
        );

        // 각 선언은 정본에 따로 적용한다 — 앞 반례의 변경이 뒤 반례의 전제를 지우지 않게 한다.
        for (case, id, patch, reason, passability, clear_default, clear_action) in [
            (
                "실제 기본 포커스 지움", "folder-trust",
                serde_json::json!({"id": "folder-trust", "default_index": null}),
                "명시 null", Passability::Machine, true, false,
            ),
            (
                "사람 전용 관문의 비-null 액션 무시", "login-method",
                serde_json::json!({"id": "login-method", "action": {
                    "select_index": 1, "label": "Claude account with subscription"
                }}),
                "action 선언 무시", Passability::HumanOnly, false, true,
            ),
            (
                "사람 전용 관문의 기계 통과 승격 거부", "oauth-code",
                serde_json::json!({"id": "oauth-code", "passability": "machine"}),
                "승격 선언 거부", Passability::HumanOnly, false, true,
            ),
            (
                "손상된 액션 선언", "folder-trust",
                serde_json::json!({"id": "folder-trust", "action": {"select_index": 0}}),
                "action 선언이 손상", Passability::Machine, false, false,
            ),
            (
                "사람 전용 변경과 액션 지움의 동시 선언", "theme",
                serde_json::json!({"id": "theme", "passability": "human_only", "action": null}),
                "action 선언 무시", Passability::HumanOnly, false, true,
            ),
        ] {
            let before = base.gates.iter().find(|g| g.id == id).expect("전제: 빌트인 관문을 찾지 못했다");
            if id == "folder-trust" || id == "theme" {
                assert_eq!(before.passability, Passability::Machine, "{case}: 전제인 기계 관문이 아니다");
                assert!(before.default_index.is_some(), "{case}: 전제인 기본 포커스 값이 없다");
                assert!(before.action.is_some(), "{case}: 전제인 종전 액션이 없다");
            } else {
                assert_eq!(before.passability, Passability::HumanOnly, "{case}: 전제인 사람 전용 관문이 아니다");
                assert_eq!(before.action, None, "{case}: 사람 전용 관문에 이미 액션이 있다");
            }
            let envelope = serde_json::json!({"gates": [patch]});
            let changed = resolve_with(Some(&envelope), true);
            assert_eq!(
                changed.notes.iter().filter(|note| note.contains(id) && note.contains(reason)).count(),
                1,
                "{case}: 실제 변경·거부 사유가 누락되거나 중복됐다: {:?}",
                changed.notes
            );
            let after = changed.gates.iter().find(|g| g.id == id).expect("선언 뒤 관문이 사라졌다");
            assert_eq!(after.passability, passability, "{case}: 통과 가능성의 적용·거부가 틀렸다");
            assert_eq!(
                after.default_index,
                if clear_default { None } else { before.default_index },
                "{case}: 기본 포커스의 지움·유지가 틀렸다"
            );
            assert_eq!(
                after.action.as_ref(),
                if clear_action { None } else { before.action.as_ref() },
                "{case}: 액션의 제거·유지가 틀렸다"
            );
        }
    }

    /// ★사람 1회 바닥은 **id 하나**가 아니라 "그 화면을 주장하는 선언 전량"을 봉한다(리뷰 R2).
    ///
    /// 【무엇이 뚫려 있었는가】 R1 의 바닥은 `gates.iter_mut().find(|g| g.id == b.id)` — **첫 항목
    /// 하나**만 고쳤다. 그래서 두 우회가 그대로 통과했다:
    ///   ⓐ **중복 id**(codex major): 미끼 선언 #1 이 고쳐지고, 로그인 화면에서 `identify` 가
    ///      돌려주는 것은 고쳐지지 않은 #2(진짜 로그인 needle · machine · action)다.
    ///   ⓑ **별칭 id**(claude 적대): id 가 안 겹치면 바닥이 아예 돌지 않고, 그 선언이 복원된
    ///      `login-method` **앞**에 서서 첫 매치를 가져간다.
    /// 두 경우의 귀결은 같다 — 코퍼스가 로그인 화면을 **기계 통과 가능**으로 분류한다.
    ///
    /// 【이 검체가 재지 **못하는** 것 — 정직】 needle 포함 판정은 화면 동일성의 충분조건도
    /// 필요조건도 아니다(codex 설계 검토 ①). 정본 needle 과 포함관계가 **없는** 다른 문면
    /// (예: 로그인 화면의 `3rd-party platform …` 줄)을 needle 로 쓰면 이 층위를 빠져나간다 —
    /// 아래 마지막 절이 그 사실을 박제하고, 그 구멍은 **화면 층위**에서 닫힌다
    /// (`inject_guard::confirm_denied` 의 `HumanOnlyScreen` · 검체
    ///  `human_only_screen_seals_confirmation_whatever_the_corpus_declares`).
    #[test]
    fn human_only_floor_seals_duplicate_ids_and_alias_declarations() {
        // ── ⓐ 중복 id — 미끼 #1 + 진짜 로그인 #2(codex 재현 그대로)
        let dup = serde_json::json!({"source": "replace", "gates": [
            {"id": "login-method", "needles": ["Do you want a decoy?"],
             "widget": ["Decoy", "Not a decoy"], "passability": "machine",
             "default_index": 1, "action": {"select_index": 1, "label": "Decoy"}},
            {"id": "login-method", "needles": ["Select login method"],
             "widget": ["Claude account with subscription", "Anthropic Console account"],
             "passability": "machine", "default_index": 1,
             "action": {"select_index": 1, "label": "Claude account with subscription"}},
        ]});
        let r = resolve_with(Some(&dup), true);
        let hits: Vec<&Gate> = r.gates.iter().filter(|g| g.id == "login-method").collect();
        assert_eq!(hits.len(), 2, "선언이 버려졌다 — 이 모듈의 집행 수단은 수리이지 제거가 아니다");
        for (i, g) in hits.iter().enumerate() {
            assert_eq!(
                g.passability,
                Passability::HumanOnly,
                "중복 #{i} 가 기계 통과로 남았다(첫 항목만 고치는 바닥의 재발)"
            );
            assert!(g.action.is_none(), "중복 #{i} 에 기계 액션이 남았다");
        }
        // ★화면 층위 — `identify` 가 돌려주는 그 항목이 사람 1회여야 한다(id 회계가 아니라 이것이 축이다).
        let picked = identify(&r.gates, fixtures::LOGIN_METHOD_2_1_261).expect("로그인 화면 미식별");
        assert_eq!(picked.passability, Passability::HumanOnly, "로그인 화면이 기계 통과로 분류된다");
        assert!(
            matches!(action_policy(picked, Some(MEASURED_ON)), ActionPolicy::HumanRequired { .. }),
            "정책이 로그인 화면에 기계 통과를 낸다"
        );
        // 중복은 조용히 넘어가지 않는다 — **위치까지** 적는다(codex 설계 검토 ②).
        assert!(
            r.notes.iter().any(|n| n.contains("중복 id") && n.contains("login-method@gates[0, 1]")),
            "중복 id 진단이 없다: {:?}",
            r.notes
        );
        // ★(codex 설계 검토 ⑫) 무변화 note 억제와 **공존**해도 강등 사유는 남는다.
        assert_eq!(
            r.notes.iter().filter(|n| n.contains("사람 1회")).count(),
            2,
            "강등이 조용해졌다(공격 봉투 + note 억제의 결합): {:?}",
            r.notes
        );

        // ── ⓑ 별칭 id — 정본 needle 과 **포함관계가 있는** 문면은 코퍼스 층위에서 봉해진다.
        let alias = serde_json::json!({"source": "replace", "gates": [{
            "id": "login-alias", "needles": ["Select login method"],
            "widget": ["Claude account with subscription", "Anthropic Console account"],
            "passability": "machine", "default_index": 1,
            "action": {"select_index": 1, "label": "Claude account with subscription"},
        }]});
        let ra = resolve_with(Some(&alias), true);
        let a = ra.gates.iter().find(|g| g.id == "login-alias").expect("별칭 선언이 버려졌다");
        assert_eq!(a.passability, Passability::HumanOnly, "별칭 id 로 바닥을 우회했다");
        assert!(a.action.is_none());
        assert!(
            ra.notes.iter().any(|n| n.contains("화면을 같은 문면으로 주장")),
            "별칭 강등을 이름으로 말하지 않는다: {:?}",
            ra.notes
        );
        let picked = identify(&ra.gates, fixtures::LOGIN_METHOD_2_1_261).expect("로그인 화면 미식별");
        assert_eq!(picked.passability, Passability::HumanOnly);

        // ── ⓒ 빌트인 코퍼스는 이 술어로 서로를 잡아먹지 않는다(과잉 포섭의 상한 확인).
        let bs = builtin();
        for a in &bs {
            for b in bs.iter().filter(|b| b.passability == Passability::HumanOnly) {
                if a.id == b.id {
                    continue;
                }
                assert!(
                    !claims_same_screen(a, b),
                    "빌트인 {} 이 {} 의 화면을 주장한다고 판정된다 — 정본이 스스로 강등된다",
                    a.id,
                    b.id
                );
            }
        }

        // ── ⓓ ★잔여(정직): 포함관계가 **없는** 문면은 이 층위를 빠져나간다.
        let escape = serde_json::json!({"source": "replace", "gates": [{
            "id": "login-escape",
            "needles": ["3rd-party platform · Amazon Bedrock, Microsoft Foundry, or Vertex AI"],
            "widget": ["Claude account with subscription"],
            "passability": "machine", "default_index": 1,
            "action": {"select_index": 1, "label": "Claude account with subscription"},
        }]});
        let re = resolve_with(Some(&escape), true);
        let e = re.gates.iter().find(|g| g.id == "login-escape").unwrap();
        assert_eq!(
            e.passability,
            Passability::Machine,
            "이 검체는 **구멍이 남아 있음**을 박제한다 — 여기가 초록으로 바뀌었다면 코퍼스 층위가 \
             넓어진 것이니 과잉 포섭(정상 관문 몰살)을 함께 재고, 화면 층위 봉인은 그대로 둘 것"
        );
        assert!(
            identify(&re.gates, fixtures::LOGIN_METHOD_2_1_261).is_some_and(|g| g.id == "login-escape"),
            "구멍의 형상이 바뀌었다 — 화면 층위 검체(inject_guard)의 전제를 다시 볼 것"
        );
    }

    /// ★교체 경로도 기본 포커스 `0` 을 **값으로** 받는다(0.14.31 · 리뷰 R2).
    ///
    /// R1 은 `parse_new_gate` 의 필터를 `n <= u8::MAX` 로 넓혔지만 검체는 `source` 미지정 =
    /// **병합**(`apply_patch`) 경로만 탔다 — 필터를 종전 `n > 0` 으로 되돌려도 전 스위트가 초록이었다.
    /// 정본 §4 H-2 가 기입하라는 값(folder-trust `default_index:0` + action(1,…) = down 1)을 다음
    /// 회차가 **replace 봉투로** 쓰면 조용히 `None`(보류)로 접힌다 — 실패 방향은 안전하나 원 결함
    /// (문서화된 탈출구가 못 쓴다) 그대로다.
    #[test]
    fn replace_mode_also_accepts_default_index_zero() {
        let env = serde_json::json!({"source": "replace", "gates": [{
            "id": "folder-trust-2-1-261",
            "title": "작업 폴더 신뢰 확인(2.1.261 실측판)",
            "needles": ["Is this a project you created or one you trust?"],
            "widget": ["Enter to confirm", "Esc to cancel"],
            "passability": "machine",
            // 2.1.261 실측 형상: 0 = "No, exit" 가 기본 포커스, 1 = 신뢰.
            "default_index": 0,
            "action": {"select_index": 1, "label": "Yes, I trust this folder"},
        }]});
        let r = resolve_with(Some(&env), true);
        let g = r.gates.iter().find(|g| g.id == "folder-trust-2-1-261").expect("교체 선언이 사라졌다");
        assert_eq!(g.default_index, Some(0), "교체 경로가 0 을 조용히 버렸다(보류로 접힘)");
        assert_eq!(
            g.down_presses(),
            Some(1),
            "정본 §4 H-2 가 기입하라는 down 이 교체 봉투에서 나오지 않는다"
        );
        // 그리고 그 선언은 **Return 한 발 조립과 합의하지 못한다**(down != 0) — 확인 경계가 닫는다.
        assert!(matches!(action_policy(g, Some(MEASURED_ON)), ActionPolicy::Allowed { down: 1, .. }));
    }

    /// ★기본 포커스 `0` 은 **값이다** — 그리고 명시 `null` 은 지운다(0.14.31 · 리뷰 R1).
    ///
    /// 정본 §4 H-2 는 2.1.261 폴더신뢰를 `default_index: Some(0)` + `action:(1,…)` = **down 1** 로
    /// 기입하라고 지시한다. 종전 봉투 파서는 `n > 0` 필터로 `0` 을 조용히 버려 그 정정을 표현하지
    /// 못했다 — 문서화된 탈출구가 정본이 요구하는 값을 못 쓴 것이다.
    #[test]
    fn envelope_can_express_default_index_zero_and_can_clear_it() {
        let zero = serde_json::json!({"gates": [{"id": "folder-trust", "default_index": 0}]});
        let r = resolve_with(Some(&zero), true);
        let g = r.gates.iter().find(|g| g.id == "folder-trust").unwrap();
        assert_eq!(g.default_index, Some(0), "0 이 조용히 버려졌다(빌트인 값 잔존)");
        assert_eq!(g.down_presses(), Some(1), "정본 §4 H-2 가 기입하라는 down 이 나오지 않는다");

        // 명시 null = "기본 포커스를 모른다" → 아래키 수 산출 불가 = 보류(fail-closed).
        let cleared = serde_json::json!({"gates": [{"id": "folder-trust", "default_index": null}]});
        let r2 = resolve_with(Some(&cleared), true);
        let g2 = r2.gates.iter().find(|g| g.id == "folder-trust").unwrap();
        assert_eq!(g2.default_index, None);
        assert_eq!(g2.down_presses(), None, "미측정이 통과로 접혔다");
        assert!(matches!(action_policy(g2, Some(MEASURED_ON)), ActionPolicy::HeldNoAction));

        // 키를 **쏘는** 축(select_index)은 넓히지 않는다 — 0 은 여전히 손상 선언이다(의도된 비대칭).
        // ★codex 설계 검토 R1 이 지목한 반례: 손상된 action 은 **종전 액션을 유지**하므로
        //   `default_index: 0` 과 함께 오면 유지된 목표로 양수 down 이 산출된다. 유지되는 것이
        //   실측 빌트인 액션이라 값 자체는 옳지만(정본 §4 H-2 의 down=1 과 같다), "손상 선언이
        //   조용히 무시된다" 는 사실은 여기 박제해 둔다 — 다음 사람이 이 조합을 우연으로 만나지
        //   않게 한다. (`down_presses()` 를 소비하는 프로덕션 호출자는 아직 0 이다.)
        let corrupt = serde_json::json!({"gates": [{
            "id": "folder-trust", "default_index": 0, "action": {"select_index": 0},
        }]});
        let r3 = resolve_with(Some(&corrupt), true);
        let g3 = r3.gates.iter().find(|g| g.id == "folder-trust").unwrap();
        assert_eq!(g3.action.as_ref().map(|a| a.select_index), Some(1), "빌트인 액션이 유지되지 않았다");
        assert_eq!(g3.down_presses(), Some(1));
        assert!(r3.notes.iter().any(|n| n.contains("action 선언이 손상")), "{:?}", r3.notes);
    }

    /// ★'봉투 없음' 과 'agents.json 판독 실패' 는 보고서에서 **다른 값**이다(0.14.31 · 리뷰 R1).
    ///
    /// 종전엔 둘 다 `source:"builtin"` · `source_detail:null` 이었고 실패 사실은 한국어 산문
    /// `notes[0]` 에만 있었다. 하류(preflight C82 등)가 "봉투가 도달했나" 를 알려면 산문을
    /// 되파싱해야 했는데, 되파싱은 다음 판에 반드시 깨진다.
    #[test]
    fn report_separates_no_envelope_from_unreadable_adapter_spec() {
        let normal = report_json(&resolve_with(None, true), "claude", None, None);
        assert_eq!(normal["source"].as_str(), Some("builtin"));
        assert_eq!(normal["source_detail"], Value::Null);

        let broken = Resolved {
            gates: builtin(),
            notes: vec!["어댑터 스펙 판독 실패(no such file) — 코드 정본 폴백".to_string()],
            source: Source::SpecUnreadable { reason: "no such file".to_string() },
            envelope_ignored: false, // 스펙에 도달조차 못 했다(출처가 그 사실을 싣는다)
            declarations_rejected: 0,
        };
        let v = report_json(&broken, "claude", None, None);
        assert_eq!(v["source"].as_str(), Some("spec_unreadable"), "고장이 정상과 같은 값으로 접혔다");
        assert_eq!(v["source_detail"]["reason"].as_str(), Some("no such file"));

        // ★(0.14.31 · 리뷰 R2 · claude 적대) 그리고 **되먹임 재료를 내지 않는다.** R1 은 판독 실패
        //   에서도 빌트인 6관문을 실은 붙여넣기용 봉투를 함께 냈는데, 그 봉투는 운영자의 선언을
        //   반영하지 않는다(읽지 못했다) — 문서대로 붙여 넣으면 원 선언이 빌트인으로 덮인다.
        assert_eq!(v["override_envelope"], Value::Null, "판독 실패인데 붙여넣기 재료를 내준다");
        assert_eq!(v["override_envelope_status"]["paste_safe"].as_bool(), Some(false));
        assert!(
            v["override_envelope_status"]["reason"].as_str().is_some_and(|r| r.contains("판독")
                || r.contains("읽지 못")),
            "왜 재료를 주지 않는지 말하지 않는다: {:?}",
            v["override_envelope_status"]
        );
        // 정상 경로는 종전대로 재료를 내고 스스로 안전 표식을 단다.
        assert_eq!(normal["override_envelope"]["source"].as_str(), Some("builtin"));
        assert_eq!(normal["override_envelope_status"]["paste_safe"].as_bool(), Some(true));
        assert_eq!(normal["override_envelope_status"]["reason"], Value::Null);
    }

    /// ★보고서는 **자기가 언제 찍혔는지** 싣는다(0.14.31 · 리뷰 R2 · codex minor).
    ///
    /// `measured_on` 은 **벤더 버전**(무엇에 대고 실측했는가)이라 운영자가 `agents.json` 을 고치기
    /// 전후로 뜬 두 보고서에서 **같은 값**이다. 시각이 없으면 하류(사람·preflight)는 어느 쪽이
    /// 지금의 코퍼스인지 가릴 수 없다 — 계수와 sha 에 측정 시각을 병기하라는 것과 같은 규율이다.
    ///
    /// 시계는 **호출부가** 읽는다(이 함수는 순수하다). 그래서 검체가 고정 시각을 넣어 재현한다.
    #[test]
    fn report_carries_the_observation_time_supplied_by_the_caller() {
        let r = resolve_with(None, true);
        let at = "2026-09-08T06:30:00+0900";
        let v = report_json(&r, "claude", None, Some(at));
        assert_eq!(v["observed_at"].as_str(), Some(at), "관측 시각이 실리지 않는다");
        // 벤더 측정 출처와 **다른 축**이다 — 한 값으로 접히지 않았는지 확인한다.
        assert_eq!(v["measured_on"].as_str(), Some(MEASURED_ON));
        assert_ne!(v["observed_at"], v["measured_on"]);
        // 주지 않으면 **지어내지 않는다**(순수 함수 · 관측하지 않은 것을 인쇄하지 않는다).
        let none = report_json(&r, "claude", None, None);
        assert_eq!(none["observed_at"], Value::Null);
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

    /// ★TRIAGE(R5-WP1-H2-corpus · 독립 재유도 2026-09-08) — **override 파싱이 꺼져 있을 때도**
    /// 보고서가 "붙여 넣어도 안전" 이라며 빌트인 봉투를 내준다(reviewer-claude major ·
    /// reviewer-codex major 는 같은 결함의 두 진술이다).
    ///
    /// 【무엇이 뚫려 있는가】 `paste_safe` 는 `Source::SpecUnreadable` **하나만** 배제한다
    /// (`report_json` :917). 그런데 [`Source::OverrideDisabled`] 도 운영자의 봉투를 **한 줄도
    /// 반영하지 않은** 코드 정본이다 — "덮을 봉투가 없다"(정상)가 아니라 "봉투를 읽지 않기로
    /// 했다"(스위치)이며, 판독 실패와 **똑같이** 원 선언을 반영하지 않는다.
    ///
    /// 【실패 경로】 운영자가 folder-trust 를 `human_only` 로 조이고 자기 관문을 하나 신설해 둔
    /// 상태에서 `CYS_FIRST_RUN_GATES_OVERRIDE=0` 으로 보고서를 뜬 뒤 안내대로
    /// `override_envelope` 를 `agents.json` 에 붙여 넣으면 ⓐ 신설 관문이 사라지고 ⓑ 조여 둔
    /// folder-trust 가 **machine 으로 되돌아가 자동확인이 다시 열린다**(스위치를 켜도 복구되지
    /// 않는다 — 원본이 파일에서 지워졌다). 조이는 방향의 선언이 되먹임으로 풀리는 것은
    /// §3-3("막는 쪽으로만 틀린다") 역행이다.
    #[test]
    fn report_refuses_paste_material_when_override_parsing_is_disabled() {
        // 운영자의 선언: folder-trust 를 사람 1회로 조이고, 자기 관문을 하나 신설한다.
        let operator = serde_json::json!({"gates": [
            {"id": "folder-trust", "passability": "human_only", "human_reason": "우리 조직 규정"},
            {"id": "ops-extra-gate", "needles": ["Approve this workspace policy"], "passability": "human_only"},
        ]});
        let live = resolve_with(Some(&operator), true);
        assert_eq!(
            live.gates.iter().find(|g| g.id == "folder-trust").map(|g| g.passability),
            Some(Passability::HumanOnly),
            "전제가 깨졌다 — 운영자가 조이는 방향으로 덮을 수 없다면 이 검체는 다른 것을 잰다"
        );
        assert!(live.gates.iter().any(|g| g.id == "ops-extra-gate"));

        // 같은 선언 · 스위치만 끈 상태 → 코퍼스는 코드 정본이고 **운영자 선언은 반영되지 않았다**.
        let off = resolve_with(Some(&operator), false);
        assert_eq!(off.source, Source::OverrideDisabled);
        assert_eq!(off.gates, builtin(), "전제: 이 코퍼스에는 운영자 선언이 한 줄도 없다");
        let v = report_json(&off, "claude", None, Some("2026-09-08T09:00:00+0900"));
        assert_eq!(v["source"].as_str(), Some("override_disabled"));

        // ── ① 실제 피해: 지금 나오는 봉투를 안내대로 붙여 넣으면 원 선언이 사라진다.
        if let Some(env) = v.get("override_envelope").filter(|e| !e.is_null()) {
            let pasted = resolve_with(Some(env), true);
            assert_eq!(
                pasted.gates.iter().find(|g| g.id == "folder-trust").map(|g| g.passability),
                Some(Passability::HumanOnly),
                "override 비활성 보고서의 봉투를 붙여 넣자 운영자가 조여 둔 folder-trust 가 \
                 machine 으로 되돌아갔다 — 자동확인이 다시 열린다"
            );
            assert!(
                pasted.gates.iter().any(|g| g.id == "ops-extra-gate"),
                "운영자가 신설한 관문이 되먹임으로 소멸했다"
            );
        }

        // ── ② 표식: 판독 실패와 **같은 이유**로 재료를 주지 않는다(둘 다 원 선언 미반영이다).
        assert_eq!(
            v.get("override_envelope"),
            Some(&Value::Null),
            "override 가 꺼진 코퍼스가 붙여넣기용 봉투를 내준다(필드 삭제도 아니고 빌트인 사본이다)"
        );
        assert_eq!(
            v["override_envelope_status"]["paste_safe"].as_bool(),
            Some(false),
            "원 선언을 반영하지 않은 보고서가 '붙여 넣어도 안전' 이라고 말한다"
        );
        assert!(
            v["override_envelope_status"]["reason"].as_str().is_some_and(|r| !r.is_empty()),
            "왜 재료를 주지 않는지 말하지 않는다"
        );

        // ── ③ 정상 경로는 종전 그대로다(이 수리가 되먹임 자체를 죽이지 않는다).
        let normal = report_json(&resolve_with(Some(&operator), true), "claude", None, None);
        assert_eq!(normal["override_envelope_status"]["paste_safe"].as_bool(), Some(true));
        assert!(normal["override_envelope"]["gates"].as_array().is_some_and(|a| a.len() >= 6));
    }

    /// ★(0.14.31 · 독립 재유도 H2-A 확장 · codex 설계 검토 1) 봉투가 **도달했는데 한 줄도
    /// 반영되지 않은** 코퍼스도 되먹임 재료를 내지 않는다.
    ///
    /// 【무엇이 뚫려 있었나】 [`Source::Builtin`] 은 두 사실을 한 값으로 접는다 — "덮을 봉투가
    /// 애초에 없다"(정상)와 "봉투는 있었는데 선언이 전부 거부됐다"(운영자가 쓴 것이 파일에 남아
    /// 있다). 뒤쪽에서 보고서 봉투를 안내대로 붙여 넣으면 그 선언이 빌트인으로 덮여 사라진다 —
    /// `override_disabled` 와 **같은 형태의 손실**이다(다만 거부된 선언이라 즉시 위험하진 않다).
    #[test]
    fn report_refuses_paste_material_when_the_envelope_reached_but_nothing_was_applied() {
        // 운영자가 쓴 선언 2건이 **전부 거부**되는 형상(needles 결손 · id 결손).
        let env = json!({"gates": [
            {"id": "ops-extra-gate", "passability": "human_only"},
            {"needles": ["Approve this workspace policy"]},
        ]});
        let r = resolve_with(Some(&env), true);
        assert_eq!(r.source, Source::Builtin, "전제: 출처가 '정상' 으로 접힌다");
        assert!(r.envelope_ignored, "봉투가 도달했는데 반영 0 인 사실이 소실됐다");
        let v = report_json(&r, "claude", None, None);
        assert_eq!(v["override_envelope"], Value::Null, "지울 선언이 있는데 붙여넣기 재료를 내준다");
        assert_eq!(v["override_envelope_status"]["paste_safe"].as_bool(), Some(false));
        assert!(v["override_envelope_status"]["reason"]
            .as_str()
            .is_some_and(|s| s.contains("반영")));

        // 비객체 봉투도 같다(운영자가 무언가 써 두었고, 파서가 통째로 버렸다).
        let junk = json!("first_run_gates 를 문자열로 썼다");
        assert!(resolve_with(Some(&junk), true).envelope_ignored);

        // ── 대조군: 지울 선언이 **없는** 두 형상은 종전대로 안전하다(셋을 한 값으로 접지 않는다).
        for (label, envelope) in [
            ("키 부재", None),
            ("명시 null(의도적 비움)", Some(Value::Null)),
        ] {
            let r = resolve_with(envelope.as_ref(), true);
            assert!(!r.envelope_ignored, "{label}: 지울 선언이 없는데 재료를 막았다");
            let v = report_json(&r, "claude", None, None);
            assert_eq!(
                v["override_envelope_status"]["paste_safe"].as_bool(),
                Some(true),
                "{label}: 정상 기계의 되먹임이 죽었다"
            );
        }
        // 그리고 **반영된** 봉투는 그대로 재료를 낸다.
        let ok = json!({"gates": [{"id": "folder-trust", "passability": "human_only"}]});
        let r = resolve_with(Some(&ok), true);
        assert!(!r.envelope_ignored);
        assert_eq!(
            report_json(&r, "claude", None, None)["override_envelope_status"]["paste_safe"].as_bool(),
            Some(true)
        );
    }

    /// ★(0.14.31 · 수렴 R2 · codex major) `gates` 가 **배열이 아닐 때**도 "반영 0" 이다.
    ///
    /// 【무엇이 비어 있었나】 `obj.get("gates").and_then(|v| v.as_array())` 는 타입 오류를 빈
    /// 배열로 접었다. 그래서 `{"gates":{…객체 하나…}}`(흔한 오타)는 `decls=[]` → `Builtin` →
    /// `envelope_ignored=false` → **`paste_safe=true` + 빌트인 봉투**가 됐고, 안내대로 덮어쓰면
    /// 파일의 그 객체 선언이 사라진다. 원 선언은 이미 파싱되지 않던 상태라 활성 관문의 재개방은
    /// 아니지만(그래서 major-not-blocking), 이번 수리가 세운 "반영 0" 분기의 **누락**이다.
    ///
    /// 【가르는 것 셋】 키 부재 · 명시 `null` · **의도적 빈 배열**은 지울 선언이 없다(안전).
    /// 타입 오류만 봉투를 `null` 로 낸다.
    #[test]
    fn non_array_gates_declaration_is_recorded_as_unapplied_and_refuses_paste_material() {
        for (label, bad) in [
            ("객체", json!({"id": "ops-extra-gate", "needles": ["Approve this workspace policy?"],
                            "passability": "human_only"})),
            ("문자열", json!("ops-extra-gate")),
            ("숫자", json!(3)),
            ("불리언", json!(true)),
        ] {
            let env = json!({"gates": bad});
            let r = resolve_with(Some(&env), true);
            assert_eq!(r.source, Source::Builtin, "{label}: 전제(출처는 Builtin 으로 접힌다)");
            assert!(
                r.envelope_ignored,
                "{label}: `gates` 타입 오류가 '반영 0' 으로 서지 않는다 — 보고서가 빌트인 봉투를 \
                 내고, 붙여 넣으면 원 선언이 사라진다"
            );
            assert!(
                r.notes.iter().any(|n| n.contains("배열이 아니다")),
                "{label}: 왜 한 건도 읽지 못했는지 말하지 않는다 → {:?}",
                r.notes
            );
            let v = report_json(&r, "claude", None, None);
            assert_eq!(v["override_envelope"], Value::Null, "{label}: 붙여넣기 재료를 내준다");
            assert_eq!(v["override_envelope_status"]["paste_safe"].as_bool(), Some(false));
        }
        // replace 모드에서도 같다(빈 코퍼스 폴백을 타는 경로).
        let repl_env = json!({"source": "replace", "gates": {"id": "x"}});
        let repl = resolve_with(Some(&repl_env), true);
        assert!(repl.envelope_ignored, "replace 경로의 타입 오류가 '반영 0' 으로 서지 않는다");

        // ── 대조군: **의도적 빈 배열**은 지울 선언이 없다(안전 · 타입 오류와 가른다).
        let empty = resolve_with(Some(&json!({"gates": []})), true);
        assert!(!empty.envelope_ignored, "빈 배열(의도적)이 타입 오류와 같은 값으로 접혔다");
        assert_eq!(
            report_json(&empty, "claude", None, None)["override_envelope_status"]["paste_safe"]
                .as_bool(),
            Some(true)
        );
    }

    /// ★(0.14.31 · 수렴 R2 · reviewer-claude minor) **부분 착지**의 회계 — 거부된 선언 수가
    /// 산출물에 실리고 경고가 붙는다.
    ///
    /// 【형상】 `folder-trust` 조이기(착지) + 신설 관문(needles 결손 · 거부) 두 선언.
    /// `Merged{1,0}` 이라 `envelope_ignored=false` 이고 `paste_safe=true` 다 — 효력 있던 조임은
    /// 봉투에 그대로 실리므로 관문 재개방은 없다(그래서 minor). 그러나 안내대로 붙여 넣으면
    /// **거부된 선언이 파일에서 사라진다**(오타를 고칠 원본까지). 최소 조치는 그 수를 싣는 것.
    #[test]
    fn partially_applied_envelope_reports_how_many_declarations_were_rejected() {
        let env = json!({"gates": [
            {"id": "folder-trust", "passability": "human_only"},
            {"id": "ops-extra-gate", "passability": "human_only"}, // needles 결손 → 거부
        ]});
        let r = resolve_with(Some(&env), true);
        assert!(matches!(r.source, Source::Merged { overridden: 1, added: 0 }), "전제 → {:?}", r.source);
        assert!(!r.envelope_ignored, "전제: 전부/전무 축은 '반영됨' 이다(그래서 이 회계가 필요하다)");
        assert_eq!(r.declarations_rejected, 1, "거부된 선언을 세지 않는다");

        let v = report_json(&r, "claude", None, None);
        assert_eq!(v["override_envelope_status"]["paste_safe"].as_bool(), Some(true));
        assert_eq!(v["override_envelope_status"]["declarations_rejected"].as_u64(), Some(1));
        assert!(
            v["override_envelope_status"]["warning"].as_str().is_some_and(|w| w.contains("1건")),
            "부분 착지인데 경고가 없다 → {:?}",
            v["override_envelope_status"]
        );
        // 그리고 **착지한 조임은 봉투에 살아 있다**(이 사실이 이 항을 minor 로 만든다).
        let landed = v["override_envelope"]["gates"]
            .as_array()
            .expect("봉투")
            .iter()
            .find(|g| g["id"].as_str() == Some("folder-trust"))
            .expect("folder-trust 선언")
            .clone();
        assert_eq!(landed["passability"].as_str(), Some("human_only"));

        // 대조군 — 전부 착지하면 경고가 없다(정상 기계에 소음을 만들지 않는다).
        let ok = resolve_with(Some(&json!({"gates": [{"id": "folder-trust", "passability": "human_only"}]})), true);
        assert_eq!(ok.declarations_rejected, 0);
        let ov = report_json(&ok, "claude", None, None);
        assert_eq!(ov["override_envelope_status"]["declarations_rejected"].as_u64(), Some(0));
        assert_eq!(ov["override_envelope_status"]["warning"], Value::Null);
    }

    /// ★(0.14.31 · 수렴 R2 · reviewer-claude minor) **줄바꿈으로 접힌 배너**에서도 버전을 읽는다.
    ///
    /// 좁은 pane·ConPTY 에서 배너는 상자 안에서 접힌다. 연속 앵커(`Claude Code v`)만 요구하면
    /// 그 화면의 버전 증거는 **통째로 사라지고**(미상 → 통과), 이 축의 유일한 증거가 화면
    /// 문자열이라 그 손실은 곧 드리프트 미탐이다. 실패 방향은 종전과 같지만(새 구멍은 아니다)
    /// 닫을 수 있으면 닫는다.
    ///
    /// ★라이브 좌석에서 배너가 **실제로 어디에 어떤 폭으로** 찍히는지는 아직 미관측이다 —
    ///   릴리스 게이트 항목으로 남는다(커밋 Not-tested).
    #[test]
    fn banner_version_survives_a_line_wrapped_box_render() {
        let wrapped = "│ ✻ Welcome to Claude\n│   Code v2.1.263      │\n│ /help for help       │\n";
        assert_eq!(
            banner_versions(wrapped),
            vec!["2.1.263".to_string()],
            "접힌 배너에서 버전 증거가 사라진다"
        );
        // 종전 형상(한 줄)은 그대로 읽는다 — 그리고 두 패스가 같은 값을 중복으로 싣지 않는다.
        assert_eq!(
            banner_versions("Welcome to Claude Code v2.1.263\n"),
            vec!["2.1.263".to_string()]
        );
        // 접힌 배너 둘이면 합집합이다(불일치가 하나라도 있으면 확인 경계가 문다).
        let two = format!("{wrapped}(중략)\n│ Welcome to Claude\n│ Code v2.1.241 │\n");
        assert_eq!(banner_versions(&two), vec!["2.1.263".to_string(), "2.1.241".to_string()]);
        // 앵커가 없으면 여전히 미상이다(추정 금지 — 접기 패스가 판독기를 넓히지 않는다).
        assert!(banner_versions("2.1.263 (Claude Code)\n").is_empty());
        assert!(banner_versions(fixtures::FOLDER_TRUST).is_empty());
    }

    /// ★(0.14.31 · 독립 재유도 H2-B · codex 설계 검토 3) 화면용 판독기와 `--version` 용 판독기를
    /// **가른다**.
    ///
    /// [`parse_cli_version`] 의 둘째 갈래(앵커 없는 선두 점숫자)는 `claude --version` stdout 을
    /// 읽을 때의 형태다. 화면(vt100 그리드)에 그것을 걸면 "첫 글자가 점 있는 숫자면 그게 도는
    /// 버전" 이 되어 벤더가 무엇을 그리든 좌석의 버전 선언이 된다. 확인 경계는 좌석이 **스스로
    /// 찍은 배너**만 증거로 본다([`banner_versions`]).
    #[test]
    fn banner_reader_takes_every_banner_and_ignores_the_version_stdout_shape() {
        // ① 배너 전량 · 등장 순서 · 중복 제거.
        let screen = "Welcome to Claude Code v2.1.241\n(중략)\nClaude Code v9.9.9 …\n\
                      Welcome to Claude Code v2.1.241\n";
        assert_eq!(banner_versions(screen), vec!["2.1.241".to_string(), "9.9.9".to_string()]);
        assert_eq!(banner_version(screen).as_deref(), Some("2.1.241"));

        // ② `--version` stdout 형상은 **배너가 아니다**(두 판독기가 갈린다).
        let stdout = "2.1.263 (Claude Code)";
        assert_eq!(banner_versions(stdout), Vec::<String>::new());
        assert_eq!(parse_cli_version(stdout).as_deref(), Some("2.1.263"));

        // ③ 실측 픽스처: 관문 화면은 배너를 밝히지 않는다(= 미상) · 테마 화면은 밝힌다.
        assert!(banner_versions(fixtures::FOLDER_TRUST).is_empty());
        assert_eq!(banner_version(fixtures::THEME).as_deref(), Some(MEASURED_ON));
        assert_eq!(banner_version(fixtures::THEME_2_1_263).as_deref(), Some("2.1.263"));

        // ④ 상한 — 병적 입력에서 판정 시간이 화면 길이에 끌려가지 않는다.
        let many: String = (0..50).map(|i| format!("Claude Code v1.{i}\n")).collect();
        assert_eq!(banner_versions(&many).len(), BANNER_SCAN_MAX);
    }
}
