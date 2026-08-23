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
    pub measured_on: String,
    pub origin: Origin,
}

impl Gate {
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
/// ③ `replace` 인데 결과가 비면 코드 정본으로 되돌린다 — **빈 코퍼스는 '관문 없음'이 아니라
/// '눈을 감음'** 이고, 뒤 단위가 '관문 0매칭'을 ready 의 AND 항으로 쓰는 순간 허위 ready 가 된다.
pub fn resolve_with(envelope: Option<&Value>, override_on: bool) -> Resolved {
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

/// 코드 정본에 없는 id 의 신규 관문 선언 → `Gate`. 최소 요건은 id + needle ≥ 1 이다
/// (needle 없는 관문은 아무것도 식별하지 못하므로 받아도 무의미하고, 무의미한 선언을
///  조용히 받아 두면 "선언했는데 왜 안 잡히나"의 진단 비용만 남는다).
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

    pub const FOLDER_TRUST: &str = "Accessing workspace: /Users/me/work\n\
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
        \x20 cwd: /Users/me/work\n\
        ❯ \n";

    /// ★P3-0 픽스처 — **살아 있는** Claude Code TUI. 꼬리가 `❯`(입력 프롬프트 그 자체)이고
    /// 상태줄이 살아 있다. 배너·프레임 전사 근거는 PROBE_RESULTS_WINDOWS.md WIN-2 실화면
    /// (`─ Claude Code ─` · `Welcome back …` · 모델/플랜 줄).
    ///
    /// 이 부류가 **안전 밸브의 시험 대상**이다: 꼬리가 `❯` 라 '끝문자 4종' 술어로는 셸
    /// 프롬프트로 읽히지만, 화면은 명백히 TUI 를 그리고 있다.
    pub const LIVE_TUI_AT_PROMPT: &str = "─ Claude Code ─\n\
        \x20 Welcome back 미래학자!   Opus 5 (1M context) · Claude Max\n\
        \x20 /Users/me/work\n\
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

    /// 관문 문면이 **본문으로** 출력된 화면(감사 문서·소스 열람). 생애 창 상한이 없으면
    /// 작업 중 노드가 자기 화면 때문에 영구 차단된다(U-14 치명위험 ①과 같은 계열).
    pub const AUDIT_LOG_LINE: &str =
        "[launch-agent] ready(신규 출현분에 마커) — 주입 안전\n\
        [boot] worker=claude surface=7 rc=0\n\
        user@mac cys-terminal-rel %\n";

    /// ★관문이 **아닌** 화면 전량(id, 화면). 코퍼스 자기규칙 검체가 이 표를 전수로 돈다.
    pub const NON_GATE_SCREENS: &[(&str, &str)] = &[
        ("ready-shell", READY_SHELL),
        ("healthy-welcome-box", HEALTHY_WELCOME_BOX),
        ("live-tui-at-prompt", LIVE_TUI_AT_PROMPT),
        ("foreign-cli-browser-login", FOREIGN_CLI_BROWSER_LOGIN),
        ("grep-output", GREP_OUTPUT_MENTIONING_OAUTH_ERROR),
        ("audit-log-line", AUDIT_LOG_LINE),
    ];
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
        }
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
            for n in &g.needles {
                if n.contains('?') {
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

    #[test]
    fn replace_mode_takes_the_declared_corpus_but_never_an_empty_one() {
        let env = json!({"source": "replace", "gates": [
            {"id": "only", "needles": ["Do you want to continue?"]}
        ]});
        let r = resolve_with(Some(&env), true);
        assert!(matches!(r.source, Source::Replaced { count: 1 }));
        assert_eq!(r.gates.len(), 1);

        // ★빈 코퍼스는 '관문 없음'이 아니라 '눈을 감음' — 코드 정본으로 되돌린다.
        let blind = json!({"source": "replace", "gates": []});
        let r = resolve_with(Some(&blind), true);
        assert_eq!(r.gates.len(), 6, "빈 코퍼스를 그대로 받으면 허위 ready 가 열린다");
        assert!(r.notes.iter().any(|n| n.contains("맹목")));
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
