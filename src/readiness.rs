//! ready 술어 단일화 (U-13) — "지금 이 pane 에 지침을 넣어도 되는가" 의 **유일한 판정처**.
//!
//! ## 이 단위가 고치는 결함 (실측 2026-08-23 · macOS Claude Code 2.1.241 PTY 캡처 + Windows 실기)
//!
//! 첫기동 관문 6종(테마 → 로그인방식 → OAuth → 폴더신뢰 → **면책** → 새기능안내) **전부에
//! `❯` 가 있다**. 그 문자는 `agents.json` 의 claude `ready_marker` 와 **같은 문자**이고, 관문
//! 화면은 기동 직후 **신규 출력**으로 그려진다. 그래서
//!   ① 델타(커서 이후 신규 출현분) 우선 규칙은 관문 화면을 배제하지 못한다 — 배제되는 것은
//!      **잔존** ❯ 뿐이다(cys.rs 의 종전 주석 "잔존 ❯ 오탐이 원리상 불가능한 유일한 판정" 은
//!      실측으로 **반증**됐다).
//!   ② 게다가 안전 밸브(`agent_alive` 커널 사실)가 마커 분기보다 **먼저** 평가되므로,
//!      마커 축만 고쳐도 판정은 하나도 바뀌지 않는다.
//!   ③ 두 번째 소비처 `adapter_ready` 는 `scrollback_tail.contains(marker)` 한 줄이고 가드가 0이다.
//! 그 결과가 실사고다: 64~118KB 디렉티브가 **테마 선택기에 붙여넣어지고**, 그 붙여넣기의
//! Return 이 면책 창(기본 포커스 `No, exit`)을 눌러 **좌석이 rc 1 로 죽는다**.
//!
//! ## 판정식
//!
//! ```text
//! ready = (입력활성 증거 있음) ∧ (관문 문면 부재)
//! ```
//!
//! 두 항 모두 **호출부가 관측해 넘긴 값**으로만 계산한다. 이 모듈은 파일·시계·전역·env 를
//! 판정 중에 읽지 않는다 — `Observed` 가 **판정 입력의 전량**이다(숨은 입력 금지). 그래서
//! 진리표가 실기 없이 돈다.
//!
//! ## 안전 밸브는 삭제되지 않았다 (치명위험 ④)
//!
//! 밸브(`agent_alive` = 데몬이 커널 프로세스 표에서 관측한 사실)는 "**영구 오부정 불가능성**"
//! 을 보증한다 — 델타 매칭 가정이 어떤 벤더 버전에서 깨져도 살아있는 pane 이 전부 닫히는
//! 방향으로는 가지 않게 하는 장치다. 이 단위는 밸브를 **없애지 않고**, 밸브에 관문 AND 항을
//! 건다. 그리고 그 AND 항은 **밸브 블록 밖**(여기 판정부)에서 계산된다 — cys.rs 의 밸브 배선이
//! 화면 텍스트를 스스로 읽지 않게 하기 위해서다(H-SAFE-2 ①의 '밸브 근거=화면 무의존' 계약).
//!
//! ## 엄격해져서 미충족이 늘면 어떻게 되는가
//!
//! **close 가 아니라 보류다.** readiness 미확정의 귀결은 U-11 이 세운 `BootVerdict::GatePending`
//! (좌석 보존 · close 0 · kill 0 · 주입 0 · 처방 문안)이고, 커널이 **부재를 확정**했을 때만
//! `LaunchFailed`(종전 귀결)로 간다. 즉 이 단위의 엄격화는 오살 방향으로 열리지 않는다.
//! 그 전제가 실재하지 않으면 이 모듈은 착지해서는 안 된다 — `tests::u11_gate_pending_branch_exists`
//! 가 그 순서를 **코드로** 강제한다(사람 규율이 아니라).
//!
//! ## ★관문 축의 생애 창 — `Site` 마다 비용 부호가 다르다 (P4-7 · 2026-08-24)
//!
//! 관문 축은 두 자리에서 같은 코퍼스를 쓰지만 **틀리는 비용이 반대 방향**이다.
//!
//! | 자리 | 오탐 | 미탐 | 창 |
//! |---|---|---|---|
//! | [`Site::Boot`] | 영구 부트 라이브락(사람도 못 푼다) | 관문 창에 디렉티브 주입(면책 창이면 좌석 사망) | **상수로 열림** — 여기서는 관문을 반드시 잡는다 |
//! | [`Site::Reinject`] | **영구 미주입** — pack-update 가 그 노드에 영원히 도달하지 않는다 | 이미 통과한 관문(지금 화면은 역사다) | **닫힘** — 단, 관문이 지나갔음이 화면으로 증명될 때만 |
//!
//! 종전에는 이 축에 창이 아예 없어서, 살아 있는 노드의 scrollback 꼬리에 부트 때 통과한 관문
//! 문면이 남아 있기만 하면 재주입이 **영구 거부**됐다(파괴는 아니지만 영구 미주입도 결함이다).
//! 판정기는 [`gate_axis_window_closed`] 하나이고, 부트 경로는 그 안에서 상수로 열린다.
//!
//! ## ★두 번째 공통 거부 · 밸브 창 (0.14.31 · WP-1 H-1 · 감사 2026-09-06 에러 4)
//!
//! 관문 축은 **코퍼스가 아는 화면**만 잡는다. 잘린 관문(질문 줄 소실)·코퍼스에 없는 새 관문은
//! 식별되지 않고, 그때 `❯` 는 마커 델타에 실리며 밸브는 살아 있다 — 실측 판정 `Ready`(dept-3).
//! 그래서 판정식에 항이 하나 더 붙는다:
//!
//! ```text
//! ready = (입력활성 증거 있음) ∧ (관문 문면 부재) ∧ (모달 어휘 부재)
//! ```
//!
//! `modal_signature` 는 코퍼스와 **독립**인 순수 술어이고(위젯 푸터·선택지 라벨·선택 커서), 형제
//! 축 `inject_guard::decide_allowing` 도 **같은 함수**를 소비한다(판정 분리 금지). 밸브는 이제
//! `time_fallback_reached ∧ idle_quiet==Some(true)` 창 안에서만 열린다 — 아직 그리는 화면에는
//! 열리지 않는다. 두 변경 모두 **보류 방향**이고, 롤백 스위치는 종전 그대로다(아래 표 · 새 노브 0).
//!
//! ## 롤백 스위치 — **마스터 하나 + 축 노브 하나**
//!
//! | 스위치 | 값 | 되돌아가는 범위 |
//! |---|---|---|
//! | **`CYS_BOOT_GATES`** | `0` | ★이 캠페인이 추가한 판정 축 **전부**(readiness·주입 가드·신뢰 정책·보류 귀결) |
//! | `CYS_READINESS_V1` | `1` | 이 파일의 관문 AND 항 하나만 |
//!
//! ★(BLOCK-3 · 2026-08-24) 축 노브 **단독으로는 종전 동작이 돌아오지 않는다** — `V1=1` 로
//! ready 가 나도 부트 사전 가드와 `inject_text` 가드가 다시 잡아 rc 78 · 미주입이 유지된다.
//! 사고 순간에 사람이 쥐는 손잡이는 **마스터 스위치 하나**다(합류 지점은 `crate::gate_axes_from`).
//!
//! env 를 읽는 곳은 [`legacy_v1`] 하나뿐이고 축 판정은 순수 [`legacy_v1_from`] 에 있다. 느슨한
//! truthy 를 받지 않는 것(`== Some("1")`)은 형제 게이트(`CYS_GATE_PENDING_CLOSE`)와 같은
//! 규율이다 — 오타로 안전장치가 조용히 뒤집히는 것을 막는다.

use crate::first_run_gates::{self, Gate, Passability};

/// ★롤백 스위치의 env 이름(1지점).
pub const ENV_V1: &str = "CYS_READINESS_V1";

/// 종전(U-13 이전) 판정으로 되돌릴 것인가. **env 를 읽는 유일한 지점**.
///
/// ★(BLOCK-3/BLOCK-4 · 2026-08-24) 자기 축의 노브와 **상위 접기값**을 OR 한다:
///   · 마스터 스위치(`CYS_BOOT_GATES=0`) — 하나로 전 축 종전 복귀(BLOCK-3).
///   · 보류 장치 꺼짐(`CYS_GATE_PENDING_CLOSE=1` / `CYS_GATE_PENDING=0`) — 보류라는 안전한
///     귀결이 없는데 엄격 판정만 남으면 관문 화면이 곧 `LaunchFailed` → **전 pane close** 다
///     (BLOCK-4 재난④). 엄격화와 보류는 한 몸이므로 여기서 함께 풀린다.
/// 불변식의 소유자는 `crate::gate_axes_from` 하나이고 이 함수는 그 합류값을 소비만 한다.
pub fn legacy_v1() -> bool {
    legacy_v1_from(std::env::var(ENV_V1).ok().as_deref()) || crate::gate_axes_forced_legacy()
}

/// 위 판정의 순수 절반(테스트가 env 를 건드리지 않게 분리).
pub fn legacy_v1_from(raw: Option<&str>) -> bool {
    raw == Some("1")
}

/// 판정을 요청한 자리. 같은 술어를 두 소비처가 쓰되 **관측 재료가 다르다**는 사실을 타입으로
/// 남긴다 — 재료 차이를 `Option` 의 뜻으로 숨기면 다음 감사자가 "이 축은 왜 항상 None 인가"를
/// 다시 발굴해야 한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Site {
    /// `boot_agent_on_surface` 폴링 — 기동 직후. 커널 생존·델타·화면·꼬리 술어를 전부 관측한다.
    Boot,
    /// `adapter_ready`(pack-update 재주입) — 살아있는 노드의 scrollback 꼬리와 idle 회계만 있다.
    Reinject,
}

/// ready 를 선언한 **근거**. 진단 문안과 진리표가 이 값을 읽는다(판정 자체는 bool 이지만,
/// "무엇 때문에 ready 인가"가 사라지면 사고 후 원인 추적이 불가능해진다).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Evidence {
    /// 안전 밸브 — 커널 프로세스 표에 에이전트가 있고 화면이 맨 셸이 아니다(P3-0).
    Valve,
    /// 기동 커서 이후 **신규 출현분**에 마커.
    MarkerDelta,
    /// 델타 미검출이지만 화면에 마커 + 꼬리가 셸 프롬프트 아님 + 시간 폴백 시점 경과.
    MarkerScreen,
    /// 마커 미정의 어댑터(codex 등)의 시간 폴백 — 꼬리가 셸 프롬프트가 아닐 때만.
    TimeFallback,
    /// 재주입 경로: 살아있는 노드의 scrollback 꼬리에 마커.
    MarkerTail,
    /// 재주입 경로: 마커 미정의 어댑터의 idle + quiet 창.
    IdleQuiet,
}

impl Evidence {
    /// 사람이 읽는 한 마디(진단 문안 전용 · 판정 재료 아님).
    pub fn label(&self) -> &'static str {
        match self {
            Evidence::Valve => "안전 밸브(커널 생존 + 화면이 맨 셸 아님)",
            Evidence::MarkerDelta => "신규 출현분에 마커",
            Evidence::MarkerScreen => "화면 마커 + 시간 폴백",
            Evidence::TimeFallback => "시간 폴백(마커 미정의 어댑터)",
            Evidence::MarkerTail => "scrollback 꼬리에 마커",
            Evidence::IdleQuiet => "idle + quiet 창",
        }
    }
}

/// 판정 입력의 **전량**. 이 구조체 밖의 사실은 판정에 쓰이지 않는다.
///
/// ★`tail_is_shell_prompt` 를 bool 로 받는 이유: 그 술어(`screen_tail_is_shell_prompt`)는
///   실행 플랫폼(`cfg!(windows)`)을 축으로 갖는 CLI 소유 함수다. 여기서 다시 구현하면 사본이
///   둘이 되고, 사본은 갈린다. 판정에 필요한 것은 그 결과 한 비트뿐이므로 **관측값으로 받는다**.
///   `None` = 그 축을 관측하지 않은 자리(재주입 경로) — '부재 ≠ 부정' 규약대로 밸브·폴백처럼
///   그 축을 **요구하는** 증거는 발화하지 않고, 요구하지 않는 증거는 종전대로 흐른다.
#[derive(Debug, Clone)]
pub struct Observed<'a> {
    pub site: Site,
    /// 데몬이 커널 프로세스 표에서 관측한 사실. `None` = 판정 불가(구 데몬·조회 실패).
    pub agent_alive: Option<bool>,
    /// 지금 사람이 보는 화면(vt100 그리드 전량). 재주입 경로에서는 scrollback 꼬리.
    pub screen: &'a str,
    /// 기동 커서 이후 신규 출현분. 관측하지 않는 자리는 빈 문자열.
    pub delta: &'a str,
    /// 어댑터 `ready_marker`. 빈 문자열은 **미정의와 동일**하게 다룬다(아래 `marker_of` 참조).
    pub marker: Option<&'a str>,
    /// U-12 관문 코퍼스(해소 완료본). 빈 슬라이스 = 관문 축 없음 = 종전 판정.
    pub gates: &'a [Gate],
    /// 화면 마지막 비공백 줄이 셸 프롬프트인가(호출부가 계산). `None` = 미관측.
    ///
    /// **정밀도 축**이다 — 이 값이 참이면 마커 화면 폴백·시간 폴백을 막는다(ready 를 선언하지
    /// 않는 방향). 밸브는 이 축을 쓰지 않는다(아래 [`Observed::bare_shell`] 참조).
    pub tail_is_shell_prompt: Option<bool>,
    /// ★밸브 전용 축(P3-0) — 이 화면이 **맨 셸**인가(호출부가 계산). `None` = 미관측.
    ///
    /// 【왜 `tail_is_shell_prompt` 와 나뉘어야 하는가 — 축의 비용 부호가 반대다】
    ///
    /// | 축 | 오탐 비용 | 미탐 비용 | 필요 |
    /// |---|---|---|---|
    /// | ready 판정(마커·시간 폴백) | 건강 pane 미기동 | **관문에 주입** | 정밀도 |
    /// | **안전 밸브** | **건강 pane 미기동** | 죽은 셸에 54KB 주입 | 재현율 |
    ///
    /// 밸브의 존재 이유는 "델타 가정이 어떤 벤더 버전에서 깨져도 살아있는 pane 이 전부 닫히는
    /// 방향으로는 가지 않게 하는 것"(영구 오부정 차단)이므로 밸브는 **잘 발화해야** 한다.
    /// 그래서 밸브의 AND 항은 "끝문자 4종"이 아니라 **"화면이 맨 셸인가"**(높은 정밀도의
    /// bare-shell 판별)여야 한다.
    ///
    /// 【무엇이 틀렸었는가 — 2026-08-24 master 확인】 종전 밸브의 AND 항은
    /// `tail_is_shell_prompt == Some(false)` 였고, 그 자리 주석에는 "델타에 `❯` 가 안 실리는
    /// TUI 는 정의상 화면을 그리고 있어 꼬리가 셸 프롬프트가 아니다" 라고 적혀 있었다.
    /// **그 문장은 거짓이다.** `screen_tail_is_shell_prompt_on` 은 셸 프롬프트 탐지기가 아니라
    /// **마지막 비공백 줄의 끝문자가 `%` `$` `#` `❯` 중 하나인지** 보는 검사이고, `❯` 는
    /// 살아있는 Claude Code TUI 의 **입력 프롬프트 그 자체**다. 즉 건강한 pane 의 꼬리가
    /// 일상적으로 `❯` 이고, 그 AND 는 **건강한 pane 에서 밸브를 상시 차단**했다.
    ///
    /// 【새 판별자의 방향】 `bare_shell` 은
    /// `꼬리가 셸 프롬프트 ∧ (꼬리에 사망 문면 ∨ ¬화면에 TUI 렌더 증거)` 로 계산된다
    /// (구현·근거는 CLI 의 `screen_is_bare_shell_on`). 이것은 종전 축(끝문자 4종 단독)보다
    /// **참이 덜 되므로** 밸브의 재현율은 오직 올라간다.
    ///
    /// 【★렌더 증거 축의 실측 정의 — P4-2 · 2026-08-24】 '렌더 증거'는 **박스 문자 1개**가
    /// 아니라 ① 한 줄 안의 **연속 길이 ≥ `TUI_FRAME_RUN_MIN`**(=8 · '프레임 자') 또는
    /// ② 대화형 위젯 문면(`TUI_RENDER_MARKS` — 현재 코퍼스는 `for shortcuts` 하나이며 claude
    /// TUI 의 `? for shortcuts` 줄이 그것이다. ★M6 에서 관문 위젯 서명 `Enter to confirm`·
    /// `Esc to cancel` 두 개를 **뺐다** — 그 둘은 "관문이다"와 "살아있다"를 동시에 뜻해
    /// 코퍼스에 없는 새 관문일수록 주입이 더 잘 나가는 역방향 성질의 출처였다)이다. 종전 정의에서는
    /// p10k 프롬프트(`╭─`/`╰─❯`)·`git log --graph` 괘선·`tree` 잔상 **한 조각**이 렌더 증거로
    /// 세어져 밸브의 AND 항이 영구 무장해제됐다(밸브가 `agent_alive` 단독으로 퇴화).
    /// 축이 좁아졌으므로 `bare_shell` 은 **참이 더 자주** 되고 밸브는 **더 자주 닫힌다** —
    /// 판정이 느슨해지는 방향이 아니라 조여지는 방향이다.
    ///
    /// 【★남는 미탐의 실제 귀결 — 2026-08-24 적대 리뷰어 격리 실행으로 정정(P4-1)】
    /// 이 자리에는 종전에 이렇게 적혀 있었다:
    ///   "남는 미탐(프레임을 그린 뒤 즉사한 경우)의 귀결은 마커 축이 따로 막고, 최악이어도
    ///    U-11 의 보류(좌석 보존)다"
    /// **그 문장은 거짓이다.** [`positive_evidence`] 의 사다리에서 밸브는 **첫 항**이고
    /// `return` 으로 **조기 종료**한다 —
    /// `if o.agent_alive == Some(true) && bare_shell_ok { return Some(Evidence::Valve); }`.
    /// 밸브가 열리는 순간 마커 축은 **한 줄도 평가되지 않으므로**, "마커 축이 따로 막는다" 는
    /// 성립할 수 없다.
    ///
    /// 반례(★P4-2 이후 실측으로 **이사**한 화면): 화면이
    /// `"╭──────────────╮\n│ Claude Code  │\n╰──────────────╯\n bye\nuser@mac ~ %"` 이고
    /// `agent_alive=true` 이면 `bare_shell` 은 **프레임 자**(한 줄 연속 런 16 ≥ 8)를 렌더
    /// 증거로 보고 `false` 를 내므로 **밸브가 열린다**. 그때 이 화면은 마커 축에서 아무 증거도
    /// 못 내는 화면인데도 판정은 `Verdict::NotYet`(보류)이 아니라 **`Verdict::Ready` = 주입**
    /// 이다 — 즉 프레임 자를 그린 뒤 즉사한 셸에 디렉티브가 들어간다. 축소하지 않고 그대로
    /// 적는다: **이 경로의 미탐 비용은 좌석 보존이 아니라 죽은 셸 주입이다.**
    ///
    /// ★이 반례는 **지워진 것이 아니라 옮겨졌다.** 종전 반례는
    /// `"─ Claude Code ─\n bye\nuser@mac ~ %"`(장식 `─` 한 조각)였는데, P4-2 가 렌더 증거 축을
    /// 연속 길이로 좁힌 뒤 그 화면의 `bare_shell` 은 `false` → **`true`** 로 뒤집혔다(실측 ·
    /// 최대 연속 런 1 < 8). 즉 그 화면에서 밸브는 이제 **닫힌다**. 미탐의 폭이 그만큼 좁아졌고,
    /// 새 경계는 검체 ②′ 가 박제한다.
    ///
    /// 이 사실을 사람 주석이 아니라 기계로 박제한 것이
    /// `tests::valve_short_circuits_the_ladder_so_the_marker_axis_is_never_consulted` 다
    /// (다음 감사자가 주석을 믿고 이 경로를 건너뛰지 못하게 한다). 술어 자체
    /// (`screen_is_bare_shell` — CLI 소유)의 수리는 **완료됐다**(P4-2 · `TUI_FRAME_RUN_MIN`
    /// 연속 길이 + `BARE_SHELL_DEATH_TAIL_LINES` 꼬리 사망 문면 OR). 남는 미탐은 **프레임 자를
    /// 그린 뒤 즉사** 한 부류 하나이고, 손으로 박는 이 필드가 CLI 술어와 다시 갈리지 않도록
    /// `tests::hand_stamped_bare_shell_is_rederived_from_the_cli_predicate_source` 가 축 상수를
    /// CLI 소스에서 재유도해 대조한다.
    pub bare_shell: Option<bool>,
    /// 시간 폴백 시점을 지났는가. **벽시계 판정은 호출부가 한다** — 이 모듈은 시계를 읽지 않는다.
    pub time_fallback_reached: bool,
    /// 재주입 경로의 대체 증거(idle ∧ quiet ≥ 임계). 관측하지 않는 자리는 `None`.
    pub idle_quiet: Option<bool>,
    /// 롤백 스위치 값(호출부가 [`legacy_v1`] 로 1회 읽어 넘긴다).
    pub legacy_v1: bool,
}

/// 판정 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// 주입해도 된다.
    Ready { evidence: Evidence },
    /// 관문 문면이 화면에 있다 — **준비가 아니다**. 파괴하지 말고 보류한다.
    /// `vetoed` = 관문이 없었다면 ready 를 선언했을 증거(진단용 · 이것이 곧 종전 오탐의 정체).
    GateHeld {
        gate_id: String,
        title: String,
        human_only: bool,
        vetoed: Option<Evidence>,
    },
    /// 아직 증거가 없다 — 계속 관측한다.
    NotYet,
}

impl Verdict {
    pub fn is_ready(&self) -> bool {
        matches!(self, Verdict::Ready { .. })
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 판정부 — ready 술어의 **단일 소유 구간**. 아래 경계 주석까지가 검체 핀의 슬라이스다.
// ═══════════════════════════════════════════════════════════════════════════

/// ready 술어. `ready = 입력활성 증거 ∧ 관문 문면 부재`.
///
/// ★논리곱을 여기 한 곳에 두는 이유: 종전에는 밸브·마커·시간폴백·`adapter_ready` 네 자리가
///   각자 ready 를 선언했고, 그래서 "마커 축만 고쳤는데 아무것도 안 바뀌는" 상태가 됐다.
///   판정이 하나면 고칠 곳도 하나다.
pub fn judge(o: &Observed) -> Verdict {
    let evidence = positive_evidence(o);
    match gate_on_screen(o) {
        Some(g) => Verdict::GateHeld {
            gate_id: g.id.clone(),
            title: g.title.clone(),
            human_only: g.passability == Passability::HumanOnly,
            vetoed: evidence,
        },
        None => {
            // ★(0.14.31 · H-1) **두 번째 공통 거부** — 코퍼스가 식별하지 못한 화면이라도 모달 어휘가
            //   전경에 있으면 준비가 아니다. 양성 증거의 **종류와 무관**하다(밸브·마커 델타·마커
            //   화면·시간 폴백 어느 것이 열려도 여기서 접힌다 — codex P0: 잘린 관문 + 신규 `❯` 가
            //   마커 델타로 Ready 가 되던 경로의 봉인). 귀결은 관문 보류와 같은 `GateHeld` 다.
            if let Some(sig) = modal_on_screen(o) {
                return Verdict::GateHeld {
                    gate_id: MODAL_UNKNOWN_ID.to_string(),
                    title: sig.title(),
                    human_only: false,
                    vetoed: evidence,
                };
            }
            match evidence {
                Some(evidence) => Verdict::Ready { evidence },
                None => Verdict::NotYet,
            }
        }
    }
}

/// 관문 AND 항 — 지금 화면에 관문이 떠 있는가. 문면의 진실원천은 `first_run_gates` 하나다.
///
/// ★왜 델타가 아니라 화면인가: 관문은 **떠 있는 동안 계속** 사람의 입력을 기다리는 상태이지
///   한 번 지나가는 출력이 아니다. 델타로 보면 "이미 그려졌고 새 출력이 없는 틱"에 관문이
///   사라진 것처럼 보인다(허위 ready). 화면으로 보면 반대 방향 오차(관문이 지나갔는데 잔상이
///   남아 보류)만 남고, 그 오차의 귀결은 **보류**다 — 오살이 오탐보다 훨씬 위험하다.
fn gate_on_screen<'a>(o: &Observed<'a>) -> Option<&'a Gate> {
    if o.legacy_v1 {
        // 롤백: U-13 착지 이전 판정 = 관문 축 없음.
        return None;
    }
    let g = first_run_gates::identify(o.gates, o.screen)?;
    if gate_axis_window_closed(o, g) {
        return None;
    }
    Some(g)
}

/// ★관문 축의 **생애 창**(P4-7 · 2026-08-24 적대 리뷰어 격리 실행).
///
/// 종전에는 이 축에 창이 **없었다** — `gate_on_screen` 이 보는 값은 `legacy_v1` 하나뿐이라,
/// 화면에 관문 문면이 있기만 하면 그 좌석이 어느 생애 단계에 있든 영원히 보류였다.
/// 형제 축(`inject_guard::decide`)은 이미 `awakened` 래치로 창을 닫고, 데몬 스캐너
/// (`governance::gate_scan_open`)도 `!awakened ∧ 나이 상한` 으로 닫는데 이 축만 열려 있었다.
///
/// ## 왜 `Site` 마다 다르게 다뤄야 하는가 — **비용 부호가 반대다**
///
/// | 자리 | 오탐(관문이 아닌데 잡음) | 미탐(관문인데 놓침) | 결론 |
/// |---|---|---|---|
/// | [`Site::Boot`] | **영구 부트 라이브락** — 화면에 통과시킬 관문이 실제로는 없으므로 사람도 못 푼다 | **관문 창에 디렉티브 주입** — 면책 창이면 그 Return 이 좌석을 죽인다 | 둘 다 비싸다 → **창을 상수로 연다** |
/// | [`Site::Reinject`] (각성 이후) | **영구 미주입** — pack-update 재주입이 그 노드에 영원히 도달하지 않는다 | **이미 지나간 관문** — 그 관문은 벌써 통과됐고 지금 화면은 역사다 | 부호가 기운다 → **창을 닫는다** |
///
/// 그래서 부트 경로의 판정은 **한 톨도 약해지지 않는다**(아래 `Site::Boot => false`).
/// 창이 닫히는 것은 재주입 경로에서, 그것도 **관문이 이미 지나갔음이 화면으로 증명될 때**뿐이다.
///
/// ★이상적인 축은 형제 축과 같은 `awakened` 래치다. 그것은 [`Observed`] 에 관측 필드를
///   하나 더 요구하고, 그 필드를 채우는 곳은 CLI(`cys.rs`)의 두 호출부다 — **이 단위의 반경
///   밖**이라 배선하지 않았다(인계 위험으로 보고). 여기서는 이미 넘어온 관측값만으로 같은
///   방향의 창을 세운다.
fn gate_axis_window_closed(o: &Observed, g: &Gate) -> bool {
    match o.site {
        // 부트 창은 **상수로 열려 있다**. 이 자리의 미탐은 '관문에 주입' 이고, 그것이 이
        // 캠페인이 막으려는 실사고 그 자체다.
        Site::Boot => false,
        Site::Reinject => gate_block_left_behind(g, o.screen, marker_of(o)),
    }
}

/// 이 관문 블록이 화면에서 **이미 지나갔는가** — 관문 문면이 화면의 **전경이 아님**을 잰다.
///
/// ★근거: 관문이 떠 있는 동안 마커(`❯`)는 그 관문의 **선택 커서**이고, 커서 뒤에는 아직 고르지
///   않은 선택지·확인 줄이 남아 있다. 관문을 통과해 노드가 앞으로 나아가면 그 뒤에 **자기 입력
///   프롬프트**가 다시 그려지고, 그 프롬프트는 화면의 **맨 끝**에 서서 입력을 기다린다.
///   즉 "지나갔다"의 실측 서명은 두 가지가 **동시에** 참인 것이다 —
///     ① 마커 뒤에 아무 문면도 없다(그 프롬프트가 지금 화면의 전경이다), 그리고
///     ② 관문 블록의 끝이 그 마커보다 앞이다(블록은 그 프롬프트 위쪽의 역사다).
///
/// ## ★무엇이 틀렸었는가 — 커서 위치를 '역사'로 오독했다 (P4-11 · 2026-08-24 리뷰어 2인)
///
/// 첫 판(P4-7)은 축 ②만 봤다. 그런데 블록의 끝은 "이 화면에서 관측된 needle·위젯 문면 중 가장
/// 뒤" 이고, `theme` 의 위젯 서명은 선택지 **1·2**(`Auto (match terminal)`·`Dark mode`)뿐이며
/// `login-method` 도 마찬가지다. 그래서 **사람이 커서를 3번째 이후 항목에 두면**(`3. Light mode`
/// · `3. 3rd-party platform`) 마커가 위젯 문면보다 뒤에 오고, 술어는 **떠 있는 관문**을
/// '지나갔다'로 읽었다 → 재주입 창이 열리고 **관문 창에 키가 나간다**(U-13 결함의 부분 재개봉).
///
/// 리뷰어가 권한 대안(블록 끝의 기준을 `needles` 만으로)은 이 축에서 **반대로 움직인다**:
/// needle 은 관문의 질문 줄이라 언제나 선택지보다 **위**에 있고, 그러면 커서가 1번째 항목에만
/// 있어도 마커가 블록 끝보다 뒤가 되어 관문 6화면 전부가 '지나갔다'로 접힌다. 그래서 채택하지
/// 않고, 리뷰어가 함께 제시한 둘째 방향 — **관문 문면이 화면 전경인가** — 을 축 ①로 세운다.
/// 커서가 어디에 있든 그 뒤에 선택지가 남아 있으면 관문은 전경이고, 창은 열리지 않는다.
///
/// 판정은 [`first_run_gates::flatten`] 공간 하나에서만 한다 — 정규화 공간의 매칭은 평탄화
/// 공간의 매칭을 함의하므로(공백만 더 지운다) 평탄화 공간이 상위집합이고, 공간을 둘로 쓰면
/// 인덱스 비교의 의미가 갈린다. 평탄화 공간에서 "마커 뒤가 비었다" 는 **공백만 남았다**와 같은
/// 뜻이다(평탄화가 공백을 전부 지운다) — 프롬프트 뒤의 개행·패딩은 전경 판정을 바꾸지 않는다.
///
/// **fail-closed**: 마커가 미정의(codex 등)거나 화면에 없거나 관문 문면을 평탄화 공간에서
/// 찾지 못하면 창을 **닫지 않는다**(= 종전대로 관문 보류). 판정 불가는 통과가 아니다.
/// 두 축을 AND 로 묶은 것도 같은 방향이다 — 축이 늘수록 창은 덜 열린다.
fn gate_block_left_behind(g: &Gate, screen: &str, marker: Option<&str>) -> bool {
    let Some(m) = marker else {
        return false;
    };
    let fm = first_run_gates::flatten(m);
    if fm.is_empty() {
        return false;
    }
    let fs = first_run_gates::flatten(screen);
    let Some(marker_last) = fs.rfind(fm.as_str()) else {
        return false;
    };
    // ★축 ① — 마커 뒤에 남은 문면이 있으면 그 마커는 **선택 커서**이지 대기 중인 입력
    //   프롬프트가 아니다. 관문은 아직 화면의 전경이므로 창을 닫지 않는다(P4-11).
    if !fs[marker_last + fm.len()..].is_empty() {
        return false;
    }
    // 축 ② — 관문 블록의 끝 = 이 화면에서 관측된 needle·위젯 문면 중 **가장 뒤**의 끝 위치.
    let mut block_end: Option<usize> = None;
    for s in g.needles.iter().chain(g.widget.iter()) {
        let f = first_run_gates::flatten(s);
        if f.is_empty() {
            continue;
        }
        if let Some(i) = fs.rfind(f.as_str()) {
            let end = i + f.len();
            block_end = Some(block_end.map_or(end, |b| b.max(end)));
        }
    }
    block_end.is_some_and(|b| marker_last >= b)
}

/// 어댑터 마커의 정규화. 빈 문자열은 **미정의와 동일**하게 다룬다.
///
/// ★근거: 종전 두 소비처가 이 지점에서 이미 갈려 있었다 — `adapter_ready` 는
///   `Some(m) if !m.is_empty()` 로 걸렀고, 부트 폴링은 `Some("")` 을 그대로 받아
///   `delta.contains("")` == true 로 **즉시 ready** 를 선언했다. 술어를 하나로 합치는 이 단위가
///   그 갈림을 그대로 옮길 이유가 없다. 정규화 방향은 **엄격**(즉시 ready → 시간 폴백 + 꼬리
///   가드)이라 오살 방향으로 열리지 않으며, 현행 어댑터 중 빈 마커를 선언한 것은 없다.
fn marker_of<'a>(o: &Observed<'a>) -> Option<&'a str> {
    o.marker.filter(|m| !m.is_empty())
}

// ═══════════════════════════════════════════════════════════════════════════
// ★(0.14.31 · WP-1 H-1) 공통 모달 거부 — 코퍼스 **밖**의 선택 위젯도 "준비"가 아니다
// ═══════════════════════════════════════════════════════════════════════════
//
// 【실측 결함(감사 2026-09-06 에러 4 · dept-3 07:09:15 → +9.1s)】 관문 코퍼스는 **needle(질문형)
// ∧ 위젯 서명** 으로 관문을 식별한다. 관문 화면이 pane 높이에 **잘려**(질문 줄이 위로 밀려 나가고
// 선택지·푸터만 남음) 그려지면 needle 이 없어 식별이 실패하고, 그 순간 선택 커서 `❯` 는 마커
// 델타에 실리고 커널 생존은 참이라 판정은 `Ready` 였다 — 디렉티브가 선택기에 붙여넣어지고 그
// Return 이 `No, exit` 를 누른다. 시뮬레이션: full/wrapped=GateHeld · **clipped/banner=Ready**.
//
// 【수리】 관문 식별과 **독립**인 두 번째 공통 거부를 둔다: 화면에 **모달 어휘**(선택 커서가 종료
// 선택지 위 · 확인/취소 푸터 · 번호 붙은 선택지 행 · 커서가 번호 항목 위)가 있으면 그 화면은 어떤
// 양성 증거(밸브·마커·시간 폴백)가 있어도 `GateHeld{unknown-modal}` 다. 관문 코퍼스에 없는
// **새 관문**(벤더 업그레이드마다 증식)일수록 이 축이 유일한 방어다. 귀결은 **보류**뿐이다 —
// close·kill·키 전송 0(설계 원칙 3 "막는 쪽으로만 틀린다").
//
// 【어휘의 진실원천】 아래 상수는 관문 **needle(질문형)** 이 아니라 **위젯 푸터·선택지 라벨**
// (코퍼스의 `widget`·`confirm_echo` 집합)이다. needle 사본은 H-READY-13 ⓑ 가 금지하고, 이 어휘가
// 코퍼스 집합의 부분집합임은 `tests::modal_vocabulary_is_a_subset_of_the_corpus_widget_and_echo_sets`
// 가 대조한다(두 벌이 갈리면 적색). 코퍼스를 **직접 읽지 않는** 이유: 코퍼스는 `agents.json` 봉투로
// 덮어써질 수 있고(관문 삭제 가능), 이 축은 바로 그 "코퍼스가 모르는 화면" 을 위한 것이다.
//
// 【확인 에코는 모달이 아니다 — 2026-07-29 킬체인의 역방향】 `Yes, I trust this folder ✔` 는 관문을
// **통과한 뒤** 남는 에코다. 규칙 ⓒ는 라벨 앞에 `N.`(선택지 번호)이 있을 때만 걸리므로 에코 한 줄은
// 걸리지 않는다 — 통과 직후 화면을 보류로 접으면 그것이 곧 부트 라이브락이다.
//
// 【매칭 공간】 라벨·커서 규칙은 **공백 정규화 공간**(줄바꿈·들여쓰기 → 1칸)에서 본다 — TUI 폭에
// 따라 라벨이 단어 경계에서 접혀도(`Yes, I trust this\nfolder`) 걸린다. 푸터 규칙은 **평탄화
// 공간**(공백 전부 제거)에서 화면 단위 AND 다 — `Enter to confirm ·\r\n Esc to cancel` 처럼 조각이
// 다른 줄로 갈라져도 걸린다. CRLF 는 `normalize` 가 공백으로 흡수한다(ConPTY 안전).
//
// 【생애 창】 부트(`Site::Boot`)에서는 **상수로 열려 있다**(관문 축과 같은 근거 — 미탐 = 관문에 주입).
// 재주입(`Site::Reinject`)에서는 관문 축과 **같은 전경 판정**(`gate_block_left_behind` 의 축 ①·②)
// 으로만 닫힌다: 마커 뒤에 아무 문면도 없고(그 마커는 대기 중인 입력 프롬프트다) 모달 문면 전량이
// 그 마커보다 앞(역사)일 때. 마커 미정의(codex)·마커 부재는 **닫지 않는다**(fail-closed).
//
// 【받아들인 잔여】 부트 창 안에서 좌석이 이 어휘를 **본문으로** 출력하면(감사표를 cat 하는 등)
// 보류로 접힌다 — 코퍼스 `BODY_TEXT_SCREENS` 와 같은 부류이고 귀결이 파괴가 아니라 보류라 받는다.
// 살아 있는 세션의 **권한 프롬프트**(`Do you want to proceed?` + 푸터)도 이 축에 걸린다 — 그것은
// 오탐이 아니라 정답이다(디렉티브가 권한 선택지에 붙여넣어지면 안 된다).

/// 코퍼스 밖 모달의 보류 id — 관문 코퍼스 id 와 충돌하지 않는 예약어.
pub const MODAL_UNKNOWN_ID: &str = "unknown-modal";

/// Boot Valve 의 출력 정적 임계(초). `surface.read_text` 응답 `quiet_secs`(마지막 PTY 출력 이후
/// 경과 초)가 이 값 **이상**이어야 [`Observed::idle_quiet`] 가 `Some(true)` 다(CONTRACTS B-4).
pub const BOOT_VALVE_QUIET_SECS: f64 = 3.0;

/// 데몬 응답 `quiet_secs` → [`Observed::idle_quiet`]. 필드 부재(구 데몬)·비수치(NaN/∞)는 `None`
/// (미관측 · '부재 ≠ 부정') — 밸브는 그때 **닫힌다**(마커·시간 폴백 경로는 그대로).
pub fn idle_quiet_from(quiet_secs: Option<f64>) -> Option<bool> {
    quiet_secs
        .filter(|q| q.is_finite())
        .map(|q| q >= BOOT_VALVE_QUIET_SECS)
}

/// 확인/취소 푸터 — 두 조각이 **모두** 있어야 한다(AND · 코퍼스 `widget` 의 부분집합).
pub const MODAL_FOOTER: [&str; 2] = ["Enter to confirm", "Esc to cancel"];
/// 종료 선택지 라벨 — 커서가 이 위에 있으면 Return 한 발이 좌석을 죽인다(면책 창의 기본 포커스).
pub const MODAL_EXIT_LABEL: &str = "No, exit";
/// 번호 붙은 선택지 행으로 인정하는 라벨(코퍼스 `confirm_echo` 의 부분집합).
pub const MODAL_CHOICE_LABELS: [&str; 5] = [
    "Yes, I trust this folder",
    "Yes, I accept",
    "No, exit",
    "Yes, try it",
    "Not now",
];

/// 모달 서명의 관측 결과. 판정 재료는 "있는가" 하나이고 나머지는 진단·생애 창 재료다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModalSignature {
    /// 걸린 규칙 라벨(등장 순 · 중복 없음). 사람이 읽는 진단용이며 판정 재료가 아니다.
    pub kinds: Vec<&'static str>,
    /// 평탄화 공간(공백 제거 · **문자 단위**)에서 걸린 문면 조각들의 **가장 뒤** 끝 위치 — 재주입
    /// 생애 창이 "모달 문면 전량이 대기 프롬프트보다 앞(역사)인가" 를 잴 때 쓴다.
    pub flat_end: usize,
    /// 선택 커서(`❯`)가 종료 선택지(`No, exit`) 위에 있다 — `inject_guard` 의 allow 구멍도 이 앞에서는
    /// 닫힌다(그 화면에 Return 을 보내면 좌석이 죽는다 · 2.1.261 폴더신뢰 기본 포커스가 그것이다).
    pub cursor_on_exit: bool,
}

impl ModalSignature {
    /// 보류 제목(진단 문안 전용).
    pub fn title(&self) -> String {
        format!("미등재 모달({})", self.kinds.join("+"))
    }
    fn note(&mut self, kind: &'static str, end: usize) {
        if !self.kinds.contains(&kind) {
            self.kinds.push(kind);
        }
        self.flat_end = self.flat_end.max(end);
    }
}

/// `hay` 안의 `needle` 시작 위치 전량(문자 단위 · 겹침 허용). 화면은 수 KB 라 단순 검색으로 충분하다.
fn find_all_chars(hay: &[char], needle: &[char]) -> Vec<usize> {
    if needle.is_empty() || needle.len() > hay.len() {
        return Vec::new();
    }
    (0..=hay.len() - needle.len())
        .filter(|&i| hay[i..i + needle.len()] == *needle)
        .collect()
}

/// `hay` 안의 `needle` **마지막** 시작 위치(문자 단위).
fn rfind_chars(hay: &[char], needle: &[char]) -> Option<usize> {
    find_all_chars(hay, needle).last().copied()
}

/// 화면에 **모달 어휘**가 있는가 — 코퍼스와 독립인 순수 술어(모듈 머리말 참조).
///
/// 규칙(하나라도 걸리면 `Some`):
///   ⓐ `cursor-on-exit`        — `❯` 뒤(선택 번호 `N.` 이 있어도 좋다)에 `No, exit`.
///   ⓑ `confirm-cancel-footer` — 평탄화 화면에 `Enter to confirm` ∧ `Esc to cancel`.
///   ⓒ `choice-row`            — `N.` 바로 뒤에 선택지 라벨([`MODAL_CHOICE_LABELS`]) · `N` 앞은
///                               경계(문두·공백·`❯`)여야 한다(에코·본문 안의 우연한 `2.` 배제).
///   ⓓ `cursor-on-numbered-item` — `❯ N.`(뒤가 공백/문말) — 선택기의 커서 행 그 자체. 잘린
///                               테마·로그인 화면(질문 줄 소실 · 라벨은 코퍼스 어휘 밖)을 이것이 잡는다.
/// 반환값의 `flat_end`·`cursor_on_exit` 는 소비처(생애 창 · allow 구멍)의 재료다.
pub fn modal_signature(screen: &str) -> Option<ModalSignature> {
    let norm: Vec<char> = first_run_gates::normalize(screen).chars().collect();
    // norm 위치 → 평탄화 위치(그 앞의 비공백 문자 수) 접두 합. `normalize` 의 공백은 ' ' 하나뿐이다.
    let mut pre: Vec<usize> = Vec::with_capacity(norm.len() + 1);
    pre.push(0);
    for c in &norm {
        pre.push(pre[pre.len() - 1] + usize::from(*c != ' '));
    }
    let flat: Vec<char> = norm.iter().copied().filter(|c| *c != ' ').collect();
    let mut sig = ModalSignature {
        kinds: Vec::new(),
        flat_end: 0,
        cursor_on_exit: false,
    };
    let starts_at = |at: usize, label: &[char]| -> bool {
        at + label.len() <= norm.len() && norm[at..at + label.len()] == *label
    };
    let exit: Vec<char> = first_run_gates::normalize(MODAL_EXIT_LABEL).chars().collect();

    // ⓐ·ⓓ — 선택 커서 행.
    for (i, &c) in norm.iter().enumerate() {
        if c != '❯' {
            continue;
        }
        let mut j = i + 1;
        while j < norm.len() && norm[j] == ' ' {
            j += 1;
        }
        // 선택 번호 `N.`(1~2자리) — 뒤가 공백이거나 문말이어야 번호다(`❯ 1.5 hours` 는 아니다).
        let mut k = j;
        while k < norm.len() && norm[k].is_ascii_digit() && k - j < 2 {
            k += 1;
        }
        let numbered_end = (k > j && k < norm.len() && norm[k] == '.'
            && (k + 1 == norm.len() || norm[k + 1] == ' '))
            .then_some(k + 1);
        let mut label_start = j;
        if let Some(ne) = numbered_end {
            sig.note("cursor-on-numbered-item", pre[ne]);
            label_start = ne;
            while label_start < norm.len() && norm[label_start] == ' ' {
                label_start += 1;
            }
        }
        if starts_at(label_start, &exit) {
            sig.cursor_on_exit = true;
            sig.note("cursor-on-exit", pre[label_start + exit.len()]);
        }
    }

    // ⓒ — 번호 붙은 선택지 행(커서 유무 무관 · 접힌 라벨 포함).
    for label in MODAL_CHOICE_LABELS {
        let l: Vec<char> = first_run_gates::normalize(label).chars().collect();
        for p in find_all_chars(&norm, &l) {
            // 라벨 바로 앞: [경계][N]{1,2}[.][ ]? — 경계 = 문두 · 공백 · `❯`.
            let mut q = p;
            if q > 0 && norm[q - 1] == ' ' {
                q -= 1;
            }
            if q == 0 || norm[q - 1] != '.' {
                continue;
            }
            let dot = q - 1;
            let mut d = dot;
            while d > 0 && norm[d - 1].is_ascii_digit() && dot - d < 2 {
                d -= 1;
            }
            if d == dot {
                continue; // 숫자 없음
            }
            let boundary_ok = d == 0 || norm[d - 1] == ' ' || norm[d - 1] == '❯';
            if boundary_ok {
                sig.note("choice-row", pre[p + l.len()]);
            }
        }
    }

    // ⓑ — 확인/취소 푸터(평탄화 공간 · 화면 단위 AND).
    let footer_ends: Vec<usize> = MODAL_FOOTER
        .iter()
        .filter_map(|f| {
            let ff: Vec<char> = first_run_gates::flatten(f).chars().collect();
            rfind_chars(&flat, &ff).map(|i| i + ff.len())
        })
        .collect();
    if footer_ends.len() == MODAL_FOOTER.len() {
        sig.note("confirm-cancel-footer", footer_ends.into_iter().max().unwrap_or(0));
    }

    (!sig.kinds.is_empty()).then_some(sig)
}

/// 모달 거부의 생애 창 — [`gate_axis_window_closed`] 와 **같은 부호**(부트 상수 개방 · 재주입은
/// 전경 판정으로만 닫힘). `true` = 이 모달 문면은 역사다(창 닫힘 · 거부하지 않는다).
fn modal_window_closed(o: &Observed, sig: &ModalSignature) -> bool {
    match o.site {
        Site::Boot => false,
        Site::Reinject => modal_left_behind(sig, o.screen, marker_of(o)),
    }
}

/// 모달 문면이 화면의 **전경이 아님**을 잰다 — [`gate_block_left_behind`] 의 축 ①(마커 뒤 공백만)
/// ∧ 축 ②(문면 전량이 마커보다 앞). **fail-closed**: 마커 미정의·부재·빈 마커는 창을 닫지 않는다.
fn modal_left_behind(sig: &ModalSignature, screen: &str, marker: Option<&str>) -> bool {
    let Some(m) = marker else {
        return false;
    };
    let fm: Vec<char> = first_run_gates::flatten(m).chars().collect();
    if fm.is_empty() {
        return false;
    }
    let fs: Vec<char> = first_run_gates::flatten(screen).chars().collect();
    let Some(marker_last) = rfind_chars(&fs, &fm) else {
        return false;
    };
    // 축 ① — 마커 뒤에 문면이 남아 있으면 그 마커는 **선택 커서**다(대기 프롬프트가 아니다).
    if marker_last + fm.len() < fs.len() {
        return false;
    }
    // 축 ② — 모달 문면 전량이 그 마커보다 앞이다(역사).
    sig.flat_end <= marker_last
}

/// 판정용 합성 — 롤백(`legacy_v1`)이면 축 자체가 없고, 생애 창이 닫혔으면 거부하지 않는다.
fn modal_on_screen(o: &Observed) -> Option<ModalSignature> {
    if o.legacy_v1 {
        return None;
    }
    let sig = modal_signature(o.screen)?;
    (!modal_window_closed(o, &sig)).then_some(sig)
}

/// '입력이 활성이다' 는 양성 증거의 사다리. **첫 증거에서 멈춘다**.
///
/// ★사다리의 순서가 계약이다: 안전 밸브가 마커 축보다 **먼저** 온다. 종전 폴링 루프에서
///   밸브가 마커 분기 앞에 있었던 이유(마커 판정 실패로 루프가 끝나기 전에 발화해야 한다)를
///   그대로 옮긴 것이고, H-SAFE-2 의 순서 핀이 이 위치를 지킨다.
fn positive_evidence(o: &Observed) -> Option<Evidence> {
    // 화면 꼬리가 **관측되었고** 셸 프롬프트가 아니다. 미관측(None)은 참으로 접지 않는다.
    // ★이 축은 **정밀도**용이다 — 마커 화면 폴백·시간 폴백처럼 "ready 를 선언하지 않는 쪽"
    //   으로만 쓴다. 밸브는 아래 `bare_shell_ok` 를 쓴다(축의 비용 부호가 반대다 — P3-0).
    let tail_ok = matches!(o.tail_is_shell_prompt, Some(false));
    // ★밸브 전용 축(P3-0): 화면이 **맨 셸이 아님**이 관측됐다. 미관측(None)은 참으로 접지
    //   않는다('부재 ≠ 부정' — 밸브는 근거가 있을 때만 연다).
    let bare_shell_ok = matches!(o.bare_shell, Some(false));
    let marker = marker_of(o);
    match o.site {
        Site::Boot => {
            // ★★안전 밸브 — 근거는 **커널 사실 하나**다. 화면·델타 텍스트를 여기서 읽지 않는다
            //   (읽는 순간 "델타 가정이 깨지면 건강 pane 이 전부 닫힌다"는 영구 오부정 방어가
            //   함께 무너진다). 꼬리 술어는 P1-1 이 세운 AND 항으로, 호출부가 계산해 넘긴 값이다.
            //
            // 【P1-1(치명) 근거 전문 — U-5 × U-9 상호작용 · U-13 에서 이 자리로 이사】
            //   종전 조건은 `agent_alive` **단독**이었다. U-5 가 `cmdline_matches_agent` 의 입력을
            //   `name()` 한 토큰에서 **자손 전체 argv** 로 승격시키자, 그 매처의 의도적 넓이
            //   (governance.rs — "false-negative(오살)가 false-positive 보다 훨씬 위험하므로
            //   매칭을 넓힌다")가 비로소 발현됐다:
            //     · Windows 트리 `powershell → cmd.exe(…\claude-2.cmd) → claude.exe` 에서
            //       claude.exe 가 즉사하고 래퍼만 남은 틱 → 래퍼 argv 의 `claude-2.cmd` 토큰이
            //       basename 일치 → `agent_alive=true` → 밸브 발화 → **54KB 디렉티브가 맨
            //       PowerShell 에 제출된다.**
            //     · 유닉스 등가: 좌석 자손의 `sh -c 'claude …'` 래퍼·`vim ~/dev/claude/x.md` 가
            //       에이전트 사망을 그대로 은폐한다.
            //   즉 U-5 는 `alive` 의 참을 늘리는 동시에 **거짓도 늘렸다**. 수리는
            //   `alive ∧ ¬화면이_맨셸` 이다. 밸브가 닫히는 유일한 경우가 "프로세스는 관측되는데
            //   화면은 맨 셸" 이고, 그건 정확히 주입해서는 안 되는 상태이기 때문이다.
            //
            // 【★P3-0 회귀 수리 — 2026-08-24 · 거짓 논거 교체】
            //   이 자리에 이사해 온 종전 논거는 이랬다:
            //     "델타에 `❯` 가 안 실리는 TUI 는 **정의상 화면을 그리고 있어** 꼬리가 셸
            //      프롬프트가 아니다"
            //   **거짓이다.** 그 판정기(`screen_tail_is_shell_prompt_on`)는 셸 프롬프트 탐지기가
            //   아니라 **마지막 비공백 줄의 끝문자가 `%` `$` `#` `❯` 중 하나인지** 보는 검사이고,
            //   `❯` 는 살아있는 Claude Code TUI 의 **입력 프롬프트 그 자체**다. 그래서 그 AND 는
            //   건강한 pane(꼬리 `❯`)에서 밸브를 **상시 차단**했고, 밸브의 존재 이유(영구 오부정
            //   차단)가 통째로 사문화됐다.
            //   수리는 **밸브 전용 술어를 분리**하는 것이다([`Observed::bare_shell`] —
            //   `꼬리가 프롬프트 ∧ (꼬리에 사망 문면 ∨ ¬화면에 TUI 렌더 증거)`). 축의 비용
            //   부호가 반대이기 때문이다: ready 판정은 정밀도가, 밸브는 **재현율**이 필요하다.
            //   새 술어는 종전 축보다 참이 **덜** 되므로 밸브 재현율은 오직 올라가고, 오살
            //   방향으로는 열리지 않는다(맨 셸이면 여전히 닫힌다).
            //   ★(P4-2 · 2026-08-24) 그 '렌더 증거' 는 박스 문자 **1개**가 아니라 한 줄 안의
            //   **연속 길이 ≥ `TUI_FRAME_RUN_MIN`** 또는 위젯 문면이다 — 종전 정의에서는 p10k
            //   프롬프트 장식 한 조각이 밸브의 AND 를 영구 무장해제했다. 축이 좁아진 방향은
            //   '주입 억제' 라 여기 판정을 느슨하게 만들지 않는다.
            //
            // 【★0.14.31 · WP-1 H-1 — 밸브 **창**】 감사 2026-09-06 에러 4: 밸브가 **로딩 배너·잘린
            //   관문 화면**을 ready 로 선언해 시간 폴백보다 **먼저**(+9.1s · inject_delay 10s) 주입을
            //   열었다. 밸브의 존재 이유(영구 오부정 차단)는 "델타 가정이 깨진 **살아 있는 정상**
            //   pane" 인데, 그 pane 은 정의상 ① 준비 예산(시간 폴백)을 다 쓴 뒤에도 ② 출력이 **정적**
            //   (더 그릴 것이 없다)이다. 아직 그리고 있는 화면(스피너·배너 전개)은 그 둘 중 하나가
            //   거짓이므로 밸브가 열릴 이유가 없다. 그래서 `time_fallback_reached ∧ idle_quiet==Some(true)`
            //   를 AND 로 더한다 — 판정 조건이 **조여지는** 방향이고, 근거는 여전히 화면 텍스트가 아닌
            //   벽시계·데몬 회계(`quiet_secs`)다(B4 오탐 방향·화면 무의존 계약 무변).
            //   `idle_quiet==None`(구 데몬 · 미관측)은 '부재 ≠ 부정' — 밸브는 닫히고 마커·시간 폴백
            //   경로만 남는다. 롤백(`legacy_v1`)은 종전 밸브(창 없음)로 그대로 되돌린다(새 노브 0).
            //   비용: 마커 델타 경로(정상 claude 부트)는 무변 · 밸브 단독 경로는 최대 +`quiet` 초 지연.
            let valve_window_ok =
                o.legacy_v1 || (o.time_fallback_reached && o.idle_quiet == Some(true));
            if o.agent_alive == Some(true) && bare_shell_ok && valve_window_ok {
                return Some(Evidence::Valve);
            }
            // ★마커 델타 우선 — 기동 send 직전 커서 이후 **신규 출현분**에서만 본다(B4).
            //   개수 비교(잔존 마커 개수 산술)로 되돌리는 것은 영구 오부정 회귀라 금지다.
            if let Some(m) = marker {
                if o.delta.contains(m) {
                    return Some(Evidence::MarkerDelta);
                }
                // 폴백: TUI 가 개행 없이 그리드만 갱신하는 경우의 구제 경로. 화면 꼬리가 셸
                // 프롬프트면 발화하지 않는다(잔존 마커 상황에서는 꼬리가 곧 셸 프롬프트다).
                if o.time_fallback_reached && o.screen.contains(m) && tail_ok {
                    return Some(Evidence::MarkerScreen);
                }
                return None;
            }
            // 마커 미정의 어댑터(codex 등)의 시간 폴백 — 꼬리가 여전히 셸 프롬프트면 에이전트가
            // 조용히 즉시 종료한 것이므로 주입하면 디렉티브가 맨 셸로 들어간다.
            if o.time_fallback_reached && tail_ok {
                return Some(Evidence::TimeFallback);
            }
            None
        }
        Site::Reinject => {
            // 살아있는 노드의 재주입 판정. 종전 `adapter_ready` 의 두 갈래를 **의미 그대로** 옮겼다:
            // 마커 어댑터는 꼬리에 마커가 있는지만 보고(없으면 idle 폴백으로 흐르지 않는다),
            // 마커 미정의 어댑터만 idle+quiet 로 판정한다.
            // ★여기에 꼬리 술어(비-셸)까지 얹는 것은 이 단위의 위임 범위 밖이다 — 재주입 보류는
            //   안전 방향이지만, 한 단위에서 두 축을 동시에 조이면 회귀 원인이 갈린다(별도 티켓).
            if let Some(m) = marker {
                return o.screen.contains(m).then_some(Evidence::MarkerTail);
            }
            if o.idle_quiet == Some(true) {
                return Some(Evidence::IdleQuiet);
            }
            None
        }
    }
}

// ── 판정부 끝(핀 슬라이스 경계) ──────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::first_run_gates::fixtures;

    /// 실측 관문 6화면(등장 순서) — 문면은 U-12 정본의 픽스처를 **참조**한다(사본 0).
    const GATE_SCREENS: &[(&str, &str)] = &[
        ("theme", fixtures::THEME),
        ("login-method", fixtures::LOGIN_METHOD),
        ("oauth-code", fixtures::OAUTH_CODE),
        ("folder-trust", fixtures::FOLDER_TRUST),
        ("bypass-disclaimer", fixtures::TRUST_ECHO_THEN_DISCLAIMER),
        ("feature-announce-fullscreen", fixtures::FEATURE_FULLSCREEN),
    ];

    /// 실측 **정상**(온보딩 완료) 화면 — Windows 실기 캡처 전사(PROBE_RESULTS_WINDOWS.md WIN-2).
    /// 이것이 오탐 대조군이다: 관문 코퍼스가 이 화면에 걸리면 **건강한 부트 전량이 보류**로 접힌다.
    const HEALTHY_BANNER: &str = "PS C:\\WINDOWS\\system32> claude --dangerously-skip-permissions\n\
        ─ Claude Code ─\n\
        \x20 Welcome back user!   Opus 5 (1M context) · Claude Max\n\
        \x20 C:\\WINDOWS\\system32\n\
        ❯ \n";

    fn obs<'a>(screen: &'a str, delta: &'a str, gates: &'a [Gate]) -> Observed<'a> {
        Observed {
            site: Site::Boot,
            agent_alive: None,
            screen,
            delta,
            marker: Some("❯"),
            gates,
            tail_is_shell_prompt: Some(false),
            bare_shell: Some(false),
            time_fallback_reached: false,
            idle_quiet: None,
            legacy_v1: false,
        }
    }

    // ── ① 진리표: 관문 6화면 × 밸브 참/거짓 × 델타 유/무 ────────────────────
    #[test]
    fn truth_table_gate_screens_never_ready() {
        let gates = first_run_gates::builtin();
        for &(id, screen) in GATE_SCREENS {
            for alive in [Some(true), Some(false), None] {
                for tail in [Some(false), Some(true), None] {
                    // ★P3-0: 밸브 축(`bare_shell`)도 전수로 돈다 — 밸브가 열리는 조합에서도
                    //   관문 화면은 여전히 보류여야 한다(축을 나눈 것이 관문 AND 를 약화시키지
                    //   않았음의 증명).
                    for bare in [Some(false), Some(true), None] {
                        for delta in [screen, ""] {
                            for fallback in [false, true] {
                                let mut o = obs(screen, delta, &gates);
                                o.agent_alive = alive;
                                o.tail_is_shell_prompt = tail;
                                o.bare_shell = bare;
                                o.time_fallback_reached = fallback;
                                match judge(&o) {
                                    Verdict::GateHeld { ref gate_id, .. } => assert_eq!(
                                        gate_id.as_str(),
                                        id,
                                        "관문 식별이 어긋났다: 기대 {id} · 화면=\n{screen}"
                                    ),
                                    other => panic!(
                                        "관문 화면이 보류로 접히지 않았다({id} · alive={alive:?} \
                                         tail={tail:?} bare={bare:?} delta={} fallback={fallback}): \
                                         {other:?}",
                                        !delta.is_empty()
                                    ),
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// ★계측 타당성(이 진리표가 결함을 실제로 잡는가): 같은 입력이 **종전 판정에서는 ready**였다.
    /// 롤백 스위치로 종전 판정을 되살려 그 사실을 박제한다 — 관문 6화면 전부가 오탐이었다.
    #[test]
    fn legacy_v1_reproduces_the_defect_on_every_gate_screen() {
        let gates = first_run_gates::builtin();
        for &(id, screen) in GATE_SCREENS {
            let mut o = obs(screen, screen, &gates);
            o.legacy_v1 = true;
            o.agent_alive = Some(true);
            assert!(
                judge(&o).is_ready(),
                "계측 타당성 실패: 종전 판정이 관문 화면 {id} 를 ready 로 선언하지 않았다면 \
                 이 단위가 고칠 결함이 없다는 뜻이다"
            );
            // 밸브를 끄고 델타만 남겨도 종전엔 마커 축이 통과시켰다(마커만 고쳐서는 못 막는 이유).
            o.agent_alive = Some(false);
            let by_marker = judge(&o).is_ready();
            assert_eq!(
                by_marker,
                screen.contains('❯'),
                "종전 마커 축의 거동이 재현되지 않았다({id})"
            );
        }
    }

    // ── ② 오탐 대조군: 건강한 화면은 종전과 똑같이 ready 여야 한다 ──────────
    #[test]
    fn healthy_screen_stays_ready_and_matches_no_gate() {
        let gates = first_run_gates::builtin();
        assert!(
            first_run_gates::identify(&gates, HEALTHY_BANNER).is_none(),
            "관문 코퍼스가 **정상 화면**에 걸린다 — 이 상태로 AND 항을 켜면 건강한 부트가 전부 \
             보류로 접힌다(코퍼스 needle 이 질문형이 아닌 것이 원인일 수 있다: 정본 소유는 U-12)"
        );
        // ★★P4-2 핀 이사(2026-08-24 · master 지시 밖에서 워커가 실측으로 발견) —
        //   **이 자리의 밸브 주장은 더 이상 이 화면에서 성립하지 않는다.**
        //
        //   종전 서사(P3-0): "이 화면의 꼬리는 `❯` 라 끝문자 4종 술어로는 셸 프롬프트로
        //   읽힌다. 그런데 화면은 명백히 TUI 를 그리고 있으므로(`─ Claude Code ─`) 맨 셸이
        //   아니다 → **밸브가 열려야 한다**."
        //
        //   **뒷문장이 거짓이 됐다.** P4-2 가 렌더 증거를 '박스 문자 1개' 에서 **한 줄 안의
        //   연속 길이 ≥ `TUI_FRAME_RUN_MIN`(=8)** 로 좁혔는데, 이 실측 배너의 `─ Claude Code ─`
        //   는 연속 런이 **1** 이다(프레임 자가 아니라 장식이다). 위젯 문면(★M6 이후 코퍼스는
        //   `for shortcuts` 하나뿐이다)도 이 화면에는 없다. 그래서 CLI `screen_is_bare_shell_on` 의
        //   새 산출은 이 화면에서 **`true`** 다(격리 실행 실측 · unix·windows 두 축 동일).
        //
        //   → 픽스처 문자열은 **Windows 실기 캡처 전사본이라 한 글자도 바꾸지 않는다**(고쳐
        //     맞추면 실측이 아니라 창작이다). 손으로 박던 값만 **실제 산출**로 정정하고,
        //     밸브 주장은 삭제가 아니라 `live_tui_whose_tail_is_the_input_caret_still_opens_the_valve`
        //     로 **이사**했다 — 그 검체의 픽스처(`fixtures::LIVE_TUI_AT_PROMPT`)는 `? for
        //     shortcuts` 위젯 문면을 들고 있어 새 축에서도 렌더 증거가 남기 때문이다.
        //     (재유도 대조는 `hand_stamped_bare_shell_is_rederived_from_the_cli_predicate_source`.)
        let mut o = obs(HEALTHY_BANNER, HEALTHY_BANNER, &gates);
        o.agent_alive = Some(true);
        o.tail_is_shell_prompt = Some(true);
        o.bare_shell = Some(true); // ★CLI 새 술어의 실제 산출 — 손으로 고른 값이 아니다
        assert_eq!(
            judge(&o),
            Verdict::Ready {
                evidence: Evidence::MarkerDelta
            },
            "건강한 부트가 ready 를 잃었다 — 밸브가 닫혀도 마커 델타 경로는 남아야 한다"
        );
        // ★그리고 이것이 P4-2 가 **실제로 치른 비용**이다: 이 실측 배너에서 밸브는 닫힌다.
        //   마커 축을 끄면 통과 경로가 하나도 남지 않는다 — 밸브가 닫혔다는 사실의 in-band
        //   증명이자, 이 화면에서 밸브가 다시 열리면(잔상 무장해제 복귀) 적색이 나는 경계다.
        let mut valve_only = o.clone();
        valve_only.marker = None;
        valve_only.time_fallback_reached = true;
        assert_eq!(
            judge(&valve_only),
            Verdict::NotYet,
            "P4-2 의 비용 경계가 바뀌었다 — 이 배너에서 밸브가 다시 열린다면 장식 한 조각으로 \
             밸브가 무장해제되던 상태로 되돌아간 것이다"
        );
        // 커널 사실이 없을 때의 종전 통과 경로(마커 델타)는 그대로다.
        o.agent_alive = Some(false);
        assert_eq!(
            judge(&o),
            Verdict::Ready {
                evidence: Evidence::MarkerDelta
            }
        );
    }

    /// ★P3-0 회귀 박제 — **살아 있는 Claude Code TUI 가 밸브를 통과한다.**
    ///
    /// 이 부류(꼬리 `❯` · 상태줄 살아 있음)는 종전 진리표의 `live_tui` 픽스처가 시험하지
    /// **못했다** — 그 픽스처의 꼬리는 `Enter to confirm · Esc to cancel` 이라 애초에 셸
    /// 프롬프트 술어에 걸리지 않았고, 그래서 "꼬리가 `❯` 인 건강한 pane" 이라는 실제 상시
    /// 상태가 무검체로 남았다. 픽스처는 실측 캡처 기반본(U-12 정본)을 참조한다(사본 0).
    #[test]
    fn live_tui_whose_tail_is_the_input_caret_still_opens_the_valve() {
        let gates = first_run_gates::builtin();
        // 마커·시간 폴백을 모두 꺼서 **밸브만이 유일한 통과 경로**가 되게 한다.
        let mut o = obs(fixtures::LIVE_TUI_AT_PROMPT, "", &gates);
        o.marker = None;
        o.agent_alive = Some(true);
        // 꼬리는 `❯` 다 = 종전 축(끝문자 4종)에서는 '셸 프롬프트'로 읽힌다.
        o.tail_is_shell_prompt = Some(true);
        o.bare_shell = Some(false);
        // ★(0.14.31 · H-1 입력 보정) 밸브 창 재료 — 준비 예산 소진 + 출력 정적. 이 검체의 축은
        //   P3-0(맨 셸 판별 vs 꼬리 술어)이므로 창은 열어 두고 잰다.
        o.time_fallback_reached = true;
        o.idle_quiet = Some(true);
        assert_eq!(
            judge(&o),
            Verdict::Ready { evidence: Evidence::Valve },
            "살아있는 TUI 에서 밸브가 닫혔다 — 건강 pane 미기동(P3-0 회귀)"
        );

        // ★계측 타당성(in-band): 같은 입력이 **종전 AND 항**(꼬리 술어)에서는 밸브를 닫았다.
        //   그 조건을 그대로 재현하면 통과 경로가 사라진다 = 이 검체가 고치는 결함이 실재한다.
        let legacy_valve_would_fire = o.agent_alive == Some(true)
            && matches!(o.tail_is_shell_prompt, Some(false));
        assert!(
            !legacy_valve_would_fire,
            "계측 무효: 종전 AND 항이 이 화면에서 밸브를 닫지 않았다면 P3-0 은 결함이 아니다"
        );

        // 반대 방향은 그대로다 — 화면이 **맨 셸**이면 밸브는 여전히 닫힌다(오살 방지 축 무변).
        o.bare_shell = Some(true);
        assert_eq!(judge(&o), Verdict::NotYet, "맨 셸에서 밸브가 열렸다 — 죽은 셸에 주입");
        // 미관측도 열지 않는다('부재 ≠ 부정').
        o.bare_shell = None;
        assert_eq!(judge(&o), Verdict::NotYet);
    }

    /// ★P4-1 회귀 박제(2026-08-24 적대 리뷰어 격리 실행) — **밸브가 열리면 마커 축은 평가되지
    /// 않는다.** 그러므로 "남는 미탐은 마커 축이 따로 막고 최악이어도 보류다" 라는 종전 주석은
    /// 성립할 수 없고, 실제 귀결은 **`Ready` = 주입**이다.
    ///
    /// 이 검체가 없으면 다음 감사자는 주석을 믿고 이 경로를 건너뛴다 — 그것이 이 항목의 본질이다.
    ///
    /// ★★이 검체는 **바람직한 동작이 아니라 현행 결함을 있는 그대로 특성화**한다
    /// (characterization pin).
    ///
    /// 【★이사 완료 — P4-2 · 2026-08-24】 이 자리에는 종전에 이렇게 적혀 있었다:
    ///   "밸브 술어 자체(`screen_is_bare_shell` — CLI 소유)를 고치는 **다음 웨이브**에서는
    ///    ②의 기대값이 `Ready` 에서 보류로 바뀌어야 하며, 그때 이 핀은 삭제가 아니라 이사다"
    /// 그 웨이브가 **왔다.** 렌더 증거 축이 '박스 문자 1개' 에서 한 줄 안의 **연속 길이 ≥
    /// `TUI_FRAME_RUN_MIN`(=8)** 으로 좁혀졌고, 꼬리 사망 문면 OR 축
    /// (`BARE_SHELL_DEATH_TAIL_LINES`)이 더해졌다. 그래서 **핀을 지우지 않고 옮겼다**:
    ///   · ②의 화면이 잔상 한 조각(`─ Claude Code ─` · 연속 런 1)에서 **프레임 자**
    ///     (`╭───…╮` · 연속 런 16)로 이사했다. 기대값 `Ready{Valve}` 는 **그대로 참**이다.
    ///   · 이사 전 화면은 버리지 않고 **②′** 로 남겨, 같은 화면이 이제 **닫힌다**는 사실
    ///     (`bare_shell` 이 `false` → `true` 로 뒤집혔다 · 실측)을 박제한다.
    ///
    /// 미탐 자체는 남는다 — **프레임 자를 그린 뒤 즉사**한 화면은 여전히 밸브를 연다. 그러나
    /// 폭이 좁아졌고(프롬프트 장식·`tree` 괘선·`git log --graph` 잔상 한 조각으로는 더 이상
    /// 열리지 않는다), ②′ 가 그 새 경계를 정확히 박제한다. 사다리 순서 계약 ③④는 무변이다.
    #[test]
    fn valve_short_circuits_the_ladder_so_the_marker_axis_is_never_consulted() {
        let gates: Vec<Gate> = Vec::new();
        // 리뷰어 격리 실행 화면(★P4-2 이사): TUI **프레임 자**를 그린 **뒤 즉사**했다.
        //  · P4-2 이후 잔상 한 조각(`─ Claude Code ─`)은 더 이상 렌더 증거가 아니다
        //    (`TUI_FRAME_RUN_MIN` = 한 줄 안 연속 길이 하한). 밸브가 **여전히 열리는** 화면
        //    = 프레임 자가 남은 화면으로 이사한다(연속 런 16 ≥ 8).
        //  · `screen_is_bare_shell_on`(CLI 소유)은 이 화면에서 `false` 를 낸다 = 맨 셸이 아니다
        //    → 밸브의 AND 항이 열린다. 꼬리 사망 문면 축도 비어 있다(` bye` 는 사망 문면이
        //    아니다 — `command not found` 류만 그 축에 걸린다).
        //  · 마커 축은 이 화면에서 **아무 증거도 못 낸다**(델타에도 화면에도 `❯` 가 없다).
        const FRAME_THEN_DEAD: &str =
            "╭──────────────╮\n│ Claude Code  │\n╰──────────────╯\n bye\nuser@mac ~ %";
        let mut o = obs(FRAME_THEN_DEAD, "", &gates);
        o.agent_alive = Some(true);
        o.bare_shell = Some(false); // ★CLI 새 술어의 실제 산출(실측) — 손으로 고른 값이 아니다
        o.tail_is_shell_prompt = Some(true);
        o.time_fallback_reached = true;
        // ★(0.14.31 · H-1 입력 보정) 밸브 창 재료(출력 정적). 이 검체의 축은 **사다리 순서**이므로
        //   창은 열어 두고 잰다 — 창 자체는 `boot_valve_requires_time_fallback_and_quiet_output` 이 본다.
        o.idle_quiet = Some(true);

        // 전제 확인 — 마커는 정의돼 있는데 델타·화면 어디에도 없다(마커 축의 결론은 '미충족').
        let m = o.marker.expect("마커 정의");
        assert!(
            !o.delta.contains(m) && !o.screen.contains(m),
            "드릴 전제 붕괴: 이 화면에 마커가 있으면 '마커 축이 아무 증거도 못 낸다'가 거짓이다"
        );

        // ① 밸브만 끄면 드러나는 마커 축 단독의 결론 = **미충족**(보류).
        let mut marker_only = o.clone();
        marker_only.agent_alive = Some(false);
        assert_eq!(
            judge(&marker_only),
            Verdict::NotYet,
            "마커 축 단독이 이 화면에서 증거를 낸다면 이 검체의 전제가 틀렸다"
        );

        // ② 그런데 밸브가 열리면 판정은 **보류가 아니라 Ready = 주입**이다.
        //    ★이것이 종전 주석이 거짓인 지점이다: 마커 축은 '따로 막는' 위치에 있지 않다.
        assert_eq!(
            judge(&o),
            Verdict::Ready {
                evidence: Evidence::Valve
            },
            "프레임 자를 그린 뒤 즉사한 화면의 귀결은 U-11 보류가 아니라 **주입**이다(P4-1)"
        );

        // ②′ ★P4-2 이사 박제 — **잔상 한 조각으로 밸브가 열리던 화면은 이제 닫힌다.**
        //    이 화면(`─ Claude Code ─` 한 조각)이 ②의 원래 픽스처였다. 삭제하지 않고 여기
        //    남겨, "핀이 사라졌다" 가 아니라 "핀이 옮겨졌고 경계가 좁아졌다" 를 기계로 남긴다.
        //    `bare_shell` 은 CLI 새 술어의 **실제 산출**이다: 최대 연속 프레임 런 1 < 8 이고
        //    위젯 문면도 없으므로 렌더 증거 부재 → 꼬리 `%` 와 AND 하여 맨 셸 = `true`.
        let mut residue = obs("─ Claude Code ─\n bye\nuser@mac ~ %", "", &gates);
        residue.agent_alive = Some(true);
        residue.bare_shell = Some(true); // ★`false` → `true` 로 뒤집힌 축(P4-2)
        residue.tail_is_shell_prompt = Some(true);
        residue.time_fallback_reached = true;
        residue.idle_quiet = Some(true); // (H-1 입력 보정) 창은 열려 있어도 맨 셸이면 닫힌다
        assert_eq!(
            judge(&residue),
            Verdict::NotYet,
            "잔상 프레임이 여전히 밸브를 연다(P4-2 회귀) — 장식 한 조각으로 밸브가 \
             agent_alive 단독으로 퇴화하던 상태가 되돌아왔다"
        );

        // ③ 마커 축이 **다른 근거를 낼 수 있는** 화면에서도 밸브가 이긴다(첫 항 계약).
        let mut both = obs("x\n", "❯", &gates);
        both.agent_alive = Some(true);
        both.bare_shell = Some(false);
        both.time_fallback_reached = true; // (H-1 입력 보정) 밸브 창
        both.idle_quiet = Some(true);
        assert_eq!(
            judge(&both),
            Verdict::Ready {
                evidence: Evidence::Valve
            },
            "마커 델타가 있는데도 근거가 밸브가 아니면 사다리 순서가 뒤집힌 것이다"
        );

        // ④ 구조 핀 — 소스에서 **밸브가 마커 축보다 먼저이고 `return` 으로 끊는다**는 사실을
        //    박제한다. 순서를 바꾸는 리팩터가 조용히 들어오면 여기서 적색이 난다.
        let src = include_str!("readiness.rs");
        let ladder = &src[src.find("fn positive_evidence(").expect("사다리 함수")..];
        let valve = ladder
            .find("return Some(Evidence::Valve);")
            .expect("밸브 조기 반환");
        let marker = ladder.find("Evidence::MarkerDelta").expect("마커 축");
        assert!(
            valve < marker,
            "밸브가 마커 축보다 뒤로 갔다 — 이 검체의 서사(조기 반환)가 무효가 된다"
        );
    }

    /// ★판별력 보강(P4-2 · 2026-08-24) — **손으로 박은 `bare_shell` 을 CLI 술어의 소스에서
    /// 재유도해 대조한다.**
    ///
    /// 【고치는 약점】 이 모듈의 검체는 `bare_shell` 을 손으로 박는다([`Observed`] 가 판정
    /// 입력의 전량이라는 계약상 그래야 한다). 그래서 CLI 쪽 술어가 바뀌어도 여기 진리표는
    /// **기계적으로 여전히 초록**이다 — 이번 웨이브가 정확히 그 방식으로 낡았다: 잔상 화면의
    /// `bare_shell` 이 실제로는 `false` → `true` 로 뒤집힌 뒤에도 검체·주석은 초록인 채
    /// 옛 산출을 서술하고 있었다(이 저장소가 반복해 당한 '낡은 사본' 클래스).
    ///
    /// 【왜 술어를 직접 부르지 않는가】 `screen_is_bare_shell_on` 은 **바이너리 크레이트**
    /// (`src/bin/cys.rs`)의 비공개 함수라 lib 검체에서 링크할 수 없다(bin → lib 는 되지만
    /// 반대는 안 된다). 여기서 다시 구현하면 사본이 둘이 되고, 사본은 갈린다.
    ///
    /// 【그래서 무엇을 하는가】 **판정 축의 상수·코퍼스를 CLI 소스에서 읽어**(값을 여기 복사
    /// 하지 않는다) 픽스처를 그 축으로 직접 잰다. 재는 것은 *픽스처의 성질*(한 줄 안 최대
    /// 연속 프레임 런 · 위젯 문면 유무)이지 *판정*이 아니므로 술어의 사본이 아니다.
    /// CLI 가 축을 다시 넓히면(예: 하한을 1 로 되돌리면) 이 핀이 먼저 적색이 난다.
    ///
    /// 【★M6 판별력 보강 — 2026-08-24】 M6 이 `TUI_RENDER_MARKS` 를 `["for shortcuts"]` 로
    /// 줄이자 이 표에서 **축 ②(위젯 문면)를 태우는 픽스처가 `LIVE_TUI_AT_PROMPT` 하나**만
    /// 남았고, 그것은 남은 코퍼스 항(`for shortcuts`)을 쓴다. 즉 누군가 관문 위젯 서명
    /// (`Enter to confirm`·`Esc to cancel`)을 코퍼스에 **되돌려도 이 검체는 적색이 나지 않았다**
    /// — M6 의 회귀 방어가 CLI 쪽 검체 한 곳에만 걸린 단일 방어선이었다. 그래서 "꼬리 `❯` +
    /// 관문 위젯 푸터" 행을 더하고, 그 행이 실제로 적색을 낼 수 있는지를 루프 뒤에서 **직접
    /// 측정**한다(주장 아님). 있다고 믿는 방어가 없는 것은 없는 것보다 위험하다.
    ///
    /// ★대상 한정: 아래 픽스처는 전부 **꼬리가 셸 프롬프트이고 꼬리에 사망 문면이 없다**
    /// (그 두 전제도 아래에서 함께 잰다). 그 구간에서 `bare_shell` = `¬렌더 증거` 로 환원되고,
    /// 재유도가 성립하는 것도 그 구간뿐이다 — 전제 밖 화면을 여기서 판정하지 않는다.
    #[test]
    fn hand_stamped_bare_shell_is_rederived_from_the_cli_predicate_source() {
        const CLI: &str = include_str!("bin/cys.rs");

        // ── 축 ①: 프레임 **연속 길이** 하한을 CLI 소스에서 읽는다.
        let anchor = "const TUI_FRAME_RUN_MIN: usize = ";
        let at = CLI
            .find(anchor)
            .expect("CLI 의 프레임 연속 길이 상수가 없다 — 축이 사라졌거나 이름이 바뀌었다")
            + anchor.len();
        let run_min: usize = CLI[at..]
            .split(';')
            .next()
            .and_then(|s| s.trim().parse().ok())
            .expect("TUI_FRAME_RUN_MIN 을 수로 읽지 못했다");

        // ── 축 ②: 대화형 위젯 문면 코퍼스도 CLI 소스에서 읽는다.
        let anchor = "const TUI_RENDER_MARKS: &[&str] = &[";
        let at = CLI.find(anchor).expect("CLI 의 위젯 문면 코퍼스가 사라졌다") + anchor.len();
        let body = &CLI[at..at + CLI[at..].find("];").expect("코퍼스 끝을 찾지 못했다")];
        let marks: Vec<&str> = body.split('"').skip(1).step_by(2).collect();

        // 다리가 조용히 끊기지 않도록 술어 자체의 실재도 함께 못 박는다.
        assert!(
            run_min >= 2 && !marks.is_empty(),
            "축 재유도 실패: run_min={run_min} marks={marks:?}"
        );
        assert!(
            CLI.contains("fn screen_is_bare_shell_on(")
                && CLI.contains("fn screen_has_frame_rule(")
                && CLI.contains("fn screen_has_tui_render_evidence("),
            "밸브 전용 술어가 사라졌다 — 이 핀이 재유도할 대상이 없다"
        );

        // 한 줄 안 **최대** 연속 프레임 문자 수. 판정이 아니라 픽스처의 성질을 잰다.
        fn max_frame_run(text: &str) -> usize {
            text.lines()
                .map(|line| {
                    let (mut run, mut max) = (0usize, 0usize);
                    for c in line.chars() {
                        // Box Drawing `U+2500..=U+257F` · Block Elements `U+2580..=U+259F`.
                        if matches!(c as u32, 0x2500..=0x259F) {
                            run += 1;
                            max = max.max(run);
                        } else {
                            run = 0;
                        }
                    }
                    max
                })
                .max()
                .unwrap_or(0)
        }

        // ★M6 회귀 방어 픽스처 — **관문 화면인데 꼬리가 입력 캐럿**인 부류.
        //   실기 관문 화면(문면 SOT 소유)에 꼬리 `❯` 만 이어 붙인다. 관문 needle 을 여기에
        //   문자열로 복사하면 그 순간 문면의 진실원천이 둘이 된다(H-READY-13 ⓑ 적색).
        let gate_screen_at_caret = format!("{}❯ ", fixtures::FEATURE_FULLSCREEN);

        // 【진리표】 이 모듈이 손으로 박는 값 ↔ CLI 축에서 재유도한 값.
        for (label, screen, stamped) in [
            ("잔상 한 조각(P4-2 이사원 · ②′)", "─ Claude Code ─\n bye\nuser@mac ~ %", true),
            (
                "프레임 자 + 즉사(P4-2 이사처 · ②)",
                "╭──────────────╮\n│ Claude Code  │\n╰──────────────╯\n bye\nuser@mac ~ %",
                false,
            ),
            ("실측 정상 배너(Windows WIN-2)", HEALTHY_BANNER, true),
            ("살아있는 TUI(위젯 문면)", fixtures::LIVE_TUI_AT_PROMPT, false),
            // ★M6 회귀 방어 행(2026-08-24) — **관문 위젯 푸터는 렌더 증거가 아니다.**
            //   M6 이 `TUI_RENDER_MARKS` 를 `["for shortcuts"]` 로 줄이면서 이 표에서 축 ②를
            //   태우는 픽스처가 `LIVE_TUI_AT_PROMPT`(=`? for shortcuts`) 하나만 남았고,
            //   그 결과 **누가 관문 위젯 서명을 코퍼스에 되돌려도 이 검체는 적색이 나지 않았다**
            //   (M6 의 회귀 방어가 CLI 쪽 `bare_shell_predicate_separates_a_live_tui_from_a_dead_shell`
            //   의 관문 푸터 항 하나에만 걸린 **단일 방어선**이었다).
            //   ★화면은 문면 SOT 픽스처에 꼬리 캐럿만 이어 붙여 만든다 — 관문 needle 을 이
            //   파일에 **사본으로 박지 않는다**(그 사본 금지는 H-READY-13 ⓑ 가 집행한다).
            (
                "관문 화면 + 꼬리 `❯`(M6 회귀 방어)",
                gate_screen_at_caret.as_str(),
                true,
            ),
        ] {
            // 전제 ⓐ — 꼬리가 셸 프롬프트다(아니면 `bare_shell` 은 무조건 false 라 잴 것이 없다).
            let tail = screen
                .lines()
                .rev()
                .find(|l| !l.trim().is_empty())
                .map(|l| l.trim_end())
                .unwrap_or("");
            assert!(
                tail.ends_with(['%', '$', '#', '❯']),
                "{label}: 재유도 전제 붕괴 — 꼬리가 셸 프롬프트가 아니다({tail:?})"
            );
            // 전제 ⓑ — 사망 문면이 없다(있으면 OR 축이 먼저 참을 낸다). CLI 는 꼬리
            //   `BARE_SHELL_DEATH_TAIL_LINES` 줄만 보지만 여기서는 **화면 전량**을 본다 —
            //   상위 집합이라 "전량에 없으면 꼬리에도 없다" 가 항상 성립한다(안전 방향).
            let flat = first_run_gates::flatten(screen);
            for phrase in [
                "commandnotfound",
                "notfoundinPATH",
                "Nosuchfileordirectory",
                "isnotrecognizedasthenameofacmdlet",
                "isnotrecognizedasaninternalorexternalcommand",
            ] {
                assert!(
                    !flat.contains(phrase),
                    "{label}: 재유도 전제 붕괴 — 사망 문면 축({phrase})이 개입한다"
                );
            }

            let run = max_frame_run(screen);
            let marked = marks
                .iter()
                .any(|m| flat.contains(&first_run_gates::flatten(m)));
            let rederived = !(run >= run_min || marked);
            assert_eq!(
                rederived, stamped,
                "{label}: 검체가 손으로 박은 bare_shell={stamped} 인데 CLI 축에서 재유도하면 \
                 {rederived} 다(최대 연속 런 {run} vs 하한 {run_min} · 위젯 문면 {marked}) — \
                 검체가 낡았거나 CLI 술어의 축이 바뀌었다"
            );
        }

        // ── ★계측 타당성(in-band) — M6 회귀 방어 행이 **실제로 적색을 낼 수 있는가** ──
        //   위 표의 마지막 행은 "코퍼스에 관문 위젯 서명이 되돌아오면 적색"일 때만 방어선이다.
        //   그 경계가 살아 있다는 것을 여기서 직접 잰다(주장이 아니라 측정):
        //     ⓐ 그 행의 판정을 가르는 축이 **위젯 문면 하나뿐**이다(프레임 축은 개입하지 않는다).
        //     ⓑ 지금 코퍼스로는 어떤 마크도 걸리지 않는다 → 재유도 `true` = 손으로 박은 값.
        //     ⓒ 되돌림 후보 두 문면은 그 화면에 **실재한다** → 코퍼스에 하나라도 되돌아오면
        //        `marked=true` → 재유도 `false` → 위 `assert_eq!` 가 즉시 적색.
        //   ⓒ 가 없으면 "되돌려도 안 걸리는" 무력한 핀을 세운 것이고, 그것이 곧 이 캠페인이
        //   반복해서 밟은 함정(있다고 믿는 방어가 없는 것보다 위험하다)이다.
        let gate_flat = first_run_gates::flatten(&gate_screen_at_caret);
        assert!(
            max_frame_run(&gate_screen_at_caret) < run_min,
            "M6 회귀 방어 행이 프레임 축으로 먼저 걸린다 — 위젯 문면 축을 재지 못한다(핀 무효)"
        );
        assert!(
            !marks
                .iter()
                .any(|m| gate_flat.contains(&first_run_gates::flatten(m))),
            "관문 위젯 서명이 이미 코퍼스에 있다 — M6 이 되돌아간 상태다(marks={marks:?})"
        );
        for restored in ["Enter to confirm", "Esc to cancel"] {
            assert!(
                gate_flat.contains(&first_run_gates::flatten(restored)),
                "계측 무효: 되돌림 후보({restored})가 이 화면에 없다 — 코퍼스가 되돌아와도 \
                 이 핀은 적색을 내지 못한다"
            );
        }
    }

    /// ★P4-7 — 관문 축의 **생애 창**. 재주입 경로에서 *이미 지나간* 관문이 pack-update 재주입을
    /// 영구 거부하지 못하게 한다. **부트 경로의 관문 판정은 한 톨도 약해지지 않는다.**
    #[test]
    fn reinject_gate_axis_has_a_lifetime_window_while_boot_stays_constant_open() {
        let gates = first_run_gates::builtin();
        // 살아 있는 노드의 scrollback 꼬리: 부트 때 **통과한** 신기능 안내가 역사로 남아 있고,
        // 그 뒤에 작업 로그와 **현재 입력 프롬프트**가 있다.
        let tail = format!(
            "{}[boot] worker=claude surface=7 rc=0\n작업 로그\n❯ \n",
            fixtures::FEATURE_FULLSCREEN
        );

        // ── 재주입: 창이 닫힌다 → 종전의 영구 미주입이 풀린다.
        let mut r = obs(&tail, "", &gates);
        r.site = Site::Reinject;
        r.tail_is_shell_prompt = None;
        r.bare_shell = None;
        assert_eq!(
            judge(&r),
            Verdict::Ready {
                evidence: Evidence::MarkerTail
            },
            "이미 지나간 관문이 재주입을 영구 거부한다(P4-7)"
        );

        // ── 부트: 같은 화면이라도 **반드시 잡는다**(창이 상수로 열려 있다).
        let mut b = obs(&tail, "", &gates);
        b.agent_alive = Some(true);
        match judge(&b) {
            Verdict::GateHeld { ref gate_id, .. } => {
                assert_eq!(gate_id.as_str(), "feature-announce-fullscreen")
            }
            other => panic!("부트 경로의 관문 판정이 약해졌다(P4-7 수리가 반경을 넘었다): {other:?}"),
        }

        // ── 재주입이라도 **떠 있는** 관문은 여전히 잡는다(창은 증명될 때만 닫힌다).
        let mut live = obs(fixtures::FEATURE_FULLSCREEN, "", &gates);
        live.site = Site::Reinject;
        live.tail_is_shell_prompt = None;
        live.bare_shell = None;
        assert!(
            !judge(&live).is_ready(),
            "떠 있는 관문에서 창이 닫혔다 — 관문 창에 재주입하는 U-13 결함이 되돌아온다"
        );
    }

    /// 실측 관문 화면에서 **커서만** 옮긴 화면을 만든다 — 문면은 한 글자도 바꾸지 않는다.
    ///
    /// ★새 픽스처를 손으로 지어내지 않는 이유: 지어낸 화면은 "그 화면이 실재하는가" 를 다시
    ///   증명해야 한다. 여기서 시험하려는 사실은 **커서 위치 하나**이므로, 실측 전사본에서
    ///   선택 커서(`❯`)만 옮기면 그 축만 정확히 갈린다(나머지는 전부 동일).
    fn with_cursor_on(screen: &str, item: u8) -> String {
        let want = format!("{item}.");
        let mut out: String = screen
            .lines()
            .map(|l| {
                let bare = l.trim_start_matches(['❯', ' ']);
                if bare.starts_with(&want) {
                    format!("❯ {bare}")
                } else if l.starts_with('❯') {
                    format!("  {bare}")
                } else {
                    l.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        out.push('\n');
        out
    }

    /// ★★P4-11 — 생애 창이 **커서 위치**를 '관문이 지나갔다' 로 오독하지 않는다.
    ///
    /// 첫 판(P4-7)의 축은 "마커가 관문 블록 문면의 끝보다 뒤인가" 하나였다. `theme` 의 위젯
    /// 서명은 선택지 **1·2** 뿐이고 `login-method` 도 그렇다 — 그래서 사람이 커서를 3번째
    /// 항목에 두면 마커가 위젯 문면 뒤에 오고, **떠 있는 관문**이 '지나갔다'로 읽혀 재주입 창이
    /// 열렸다(U-13 결함의 부분 재개봉 · 관문 창에 키가 나간다).
    ///
    /// ★`Site::Boot` 는 이 검체에서도 **상수로 열려 있어야** 한다(창은 재주입 경로에만 있다).
    #[test]
    fn cursor_on_a_later_item_is_not_read_as_a_passed_gate() {
        let gates = first_run_gates::builtin();
        // 실측형 관문 화면 — 선택지가 3개 이상이고 위젯 서명은 1·2 번째 항목뿐인 둘.
        for (gid, base, item) in [
            ("theme", fixtures::THEME, 3u8),
            ("login-method", fixtures::LOGIN_METHOD, 3u8),
        ] {
            let screen = with_cursor_on(base, item);
            // ⓐ 커서만 옮겼을 뿐 **같은 관문**이다(계측 타당성 — 화면이 달라졌으면 서사가 무효).
            let g = first_run_gates::identify(&gates, &screen)
                .unwrap_or_else(|| panic!("{gid}: 커서를 옮겼더니 관문 식별이 깨졌다 — 이 검체는 \
                                           커서 축만 갈라야 한다:\n{screen}"));
            assert_eq!(g.id, gid, "{gid}: 커서 이동이 다른 관문으로 읽혔다");
            assert!(
                screen.contains(&format!("❯ {item}.")),
                "{gid}: 커서가 {item}번째 항목으로 옮겨지지 않았다(도우미 파손):\n{screen}"
            );

            // ⓑ ★창이 닫히지 않는다 — 관문은 지금 **떠 있다**.
            assert!(
                !gate_block_left_behind(g, &screen, Some("❯")),
                "{gid}: 커서가 {item}번째 항목에 있다는 이유로 '관문이 지나갔다'로 읽혔다 — \
                 떠 있는 관문에 재주입이 열린다(U-13 부분 재개봉):\n{screen}"
            );

            // ⓒ 그래서 재주입 경로에서도 ready 가 아니다(판정 경로 전체로 확인).
            let mut r = obs(&screen, "", &gates);
            r.site = Site::Reinject;
            r.tail_is_shell_prompt = None;
            r.bare_shell = None;
            assert!(
                !judge(&r).is_ready(),
                "{gid}: 떠 있는 관문 화면이 재주입 ready 로 읽혔다"
            );

            // ⓓ 부트 경로는 한 톨도 약해지지 않았다(창이 상수로 열려 있다).
            let mut b = obs(&screen, "", &gates);
            b.agent_alive = Some(true);
            match judge(&b) {
                Verdict::GateHeld { ref gate_id, .. } => assert_eq!(gate_id.as_str(), gid),
                other => panic!("{gid}: 부트 경로의 관문 판정이 약해졌다: {other:?}"),
            }
        }

        // ★대조군 — 같은 화면을 **실제로 통과한** 뒤(뒤에 로그 + 대기 프롬프트)에는 창이 닫힌다.
        //   이 대조가 없으면 위 초록은 "창이 아예 안 열린다" 는 퇴화로도 설명된다.
        let passed = format!("{}\n작업 로그\n❯ \n", with_cursor_on(fixtures::THEME, 3));
        let g = gates.iter().find(|g| g.id == "theme").unwrap();
        assert!(
            gate_block_left_behind(g, &passed, Some("❯")),
            "지나간 관문에서도 창이 닫히지 않는다 — 생애 창이 통째로 죽었다(영구 미주입 복귀)"
        );
    }

    /// 생애 창 술어 자체의 진리표 — **판정 불가는 창을 닫지 않는다**(fail-closed).
    #[test]
    fn gate_lifetime_window_is_fail_closed_on_every_unmeasurable_axis() {
        let gates = first_run_gates::builtin();
        let g = gates
            .iter()
            .find(|g| g.id == "feature-announce-fullscreen")
            .unwrap();
        let passed = format!("{}\n작업 로그\n❯ \n", fixtures::FEATURE_FULLSCREEN);

        // 마커가 관문 블록 **뒤**에 다시 나온다 = 지나갔다.
        assert!(gate_block_left_behind(g, &passed, Some("❯")));
        // 관문이 **떠 있는** 화면(마커는 블록 안의 선택 커서다) = 지나가지 않았다.
        assert!(!gate_block_left_behind(
            g,
            fixtures::FEATURE_FULLSCREEN,
            Some("❯")
        ));
        // 마커 미정의(codex 등) · 빈 마커 · 화면에 마커 없음 — 전부 창을 닫지 않는다.
        assert!(!gate_block_left_behind(g, &passed, None));
        assert!(!gate_block_left_behind(g, &passed, Some("")));
        assert!(!gate_block_left_behind(
            g,
            fixtures::FEATURE_FULLSCREEN,
            Some("§없는마커§")
        ));
        // 실측 관문 6화면 전부에서 창은 닫히지 않는다(재주입 보호가 통째로 사라지지 않았다).
        for &(id, screen) in GATE_SCREENS {
            let gate = first_run_gates::identify(&gates, screen).expect("관문 식별");
            assert!(
                !gate_block_left_behind(gate, screen, Some("❯")),
                "{id}: 떠 있는 관문에서 생애 창이 닫혔다"
            );
        }
    }

    #[test]
    fn dead_shell_is_not_ready() {
        let gates = first_run_gates::builtin();
        // 에이전트가 조용히 즉시 종료 — 화면에 남은 것은 셸 프롬프트뿐이고 델타에 마커가 없다.
        let mut o = obs("user@mac ~ %\n", "", &gates);
        o.agent_alive = Some(false);
        o.tail_is_shell_prompt = Some(true);
        o.bare_shell = Some(true);
        o.time_fallback_reached = true;
        assert_eq!(judge(&o), Verdict::NotYet);
        // 마커 미정의 어댑터도 같다 — 시간이 지났다는 사실만으로는 통과하지 못한다.
        o.marker = None;
        assert_eq!(judge(&o), Verdict::NotYet);
    }

    // ── ③ 증거 사다리 ──────────────────────────────────────────────────────
    #[test]
    fn valve_needs_kernel_fact_and_tail_and_precedes_marker() {
        let gates: Vec<Gate> = Vec::new();
        let mut o = obs("아무 화면\n", "", &gates);
        o.agent_alive = Some(true);
        o.time_fallback_reached = true; // (0.14.31 · H-1 입력 보정) 밸브 창 — 축은 커널 사실·맨 셸
        o.idle_quiet = Some(true);
        assert_eq!(judge(&o), Verdict::Ready { evidence: Evidence::Valve });
        // ★핀 이사(P3-0): 밸브의 AND 항은 '꼬리 술어'가 아니라 **맨 셸 판별**이다.
        //   화면이 맨 셸이면 밸브는 발화하지 않는다(래퍼만 살아있는 사망 은폐 — P1-1).
        o.bare_shell = Some(true);
        assert_eq!(judge(&o), Verdict::NotYet);
        // 미관측도 밸브를 열지 않는다(부재 ≠ '맨 셸 아님').
        o.bare_shell = None;
        assert_eq!(judge(&o), Verdict::NotYet);
        // ★그리고 **꼬리 술어 단독은 더 이상 밸브를 닫지 않는다**(P3-0 수리의 본체):
        //   살아있는 TUI 의 입력 프롬프트가 곧 `❯` 라 꼬리가 일상적으로 셸 프롬프트로 읽힌다.
        o.bare_shell = Some(false);
        o.tail_is_shell_prompt = Some(true);
        assert_eq!(
            judge(&o),
            Verdict::Ready { evidence: Evidence::Valve },
            "꼬리 술어가 다시 밸브의 AND 항이 됐다 — 건강 pane 에서 밸브 상시 차단(P3-0 회귀)"
        );
        o.tail_is_shell_prompt = None;
        assert_eq!(judge(&o), Verdict::Ready { evidence: Evidence::Valve });
        // 커널 사실이 판정 불가면 밸브는 열리지 않는다(그 보류의 귀결은 U-11 의 GatePending 이다).
        o.agent_alive = None;
        o.tail_is_shell_prompt = Some(false);
        assert_eq!(judge(&o), Verdict::NotYet);
        // 밸브가 마커보다 앞이다 — 둘 다 성립하면 근거는 밸브로 보고된다.
        let mut o2 = obs("x\n", "❯", &gates);
        o2.agent_alive = Some(true);
        o2.time_fallback_reached = true; // (H-1 입력 보정) 밸브 창
        o2.idle_quiet = Some(true);
        assert_eq!(judge(&o2), Verdict::Ready { evidence: Evidence::Valve });
    }

    #[test]
    fn marker_screen_fallback_requires_time_and_tail() {
        let gates: Vec<Gate> = Vec::new();
        let mut o = obs("...\n❯ ready\n", "", &gates);
        o.agent_alive = Some(false); // 밸브 차단 — 마커 축만 본다
        assert_eq!(judge(&o), Verdict::NotYet, "시간 폴백 전에는 화면 폴백이 없다");
        o.time_fallback_reached = true;
        assert_eq!(judge(&o), Verdict::Ready { evidence: Evidence::MarkerScreen });
        o.tail_is_shell_prompt = Some(true);
        assert_eq!(judge(&o), Verdict::NotYet, "꼬리가 셸 프롬프트면 화면 폴백 금지");
    }

    #[test]
    fn empty_marker_is_treated_as_undefined() {
        let gates: Vec<Gate> = Vec::new();
        let mut o = obs("아무 화면\n", "", &gates);
        o.agent_alive = Some(false);
        o.marker = Some("");
        assert_eq!(judge(&o), Verdict::NotYet, "빈 마커가 즉시 ready 를 만들면 안 된다");
        o.time_fallback_reached = true;
        assert_eq!(judge(&o), Verdict::Ready { evidence: Evidence::TimeFallback });
    }

    // ── ④ 두 번째 소비처(adapter_ready)도 같은 술어를 경유한다 ──────────────
    #[test]
    fn reinject_site_keeps_legacy_semantics_but_gains_gate_and() {
        let gates = first_run_gates::builtin();
        let mut o = obs("작업 로그\n❯ \n", "", &gates);
        o.site = Site::Reinject;
        o.tail_is_shell_prompt = None; // 재주입 경로는 이 축을 관측하지 않는다
        o.bare_shell = None;
        assert_eq!(judge(&o), Verdict::Ready { evidence: Evidence::MarkerTail });

        // 마커 어댑터인데 꼬리에 마커가 없으면 idle 폴백으로 흐르지 않는다(종전 의미 보존).
        let mut o2 = obs("작업 로그\n", "", &gates);
        o2.site = Site::Reinject;
        o2.tail_is_shell_prompt = None;
        o2.bare_shell = None;
        o2.idle_quiet = Some(true);
        assert_eq!(judge(&o2), Verdict::NotYet);

        // 마커 미정의 어댑터는 idle+quiet 로 통과.
        o2.marker = None;
        assert_eq!(judge(&o2), Verdict::Ready { evidence: Evidence::IdleQuiet });
        o2.idle_quiet = Some(false);
        assert_eq!(judge(&o2), Verdict::NotYet);

        // ★핵심: 관문 화면에 앉은 노드에는 재주입도 하지 않는다(두 번째 소비처가 눈멀지 않는다).
        for &(id, screen) in GATE_SCREENS {
            let mut g = obs(screen, "", &gates);
            g.site = Site::Reinject;
            g.tail_is_shell_prompt = None;
            g.bare_shell = None;
            g.idle_quiet = Some(true);
            assert!(
                !judge(&g).is_ready(),
                "재주입 경로가 관문 화면 {id} 를 ready 로 봤다 — adapter_ready 가 눈먼 채 남았다"
            );
        }
    }

    // ── ⑤ 롤백 스위치 진리표 ────────────────────────────────────────────────
    #[test]
    fn rollback_switch_is_strict_and_single_axis() {
        assert!(legacy_v1_from(Some("1")));
        for raw in [None, Some(""), Some("0"), Some("true"), Some("yes"), Some("on"), Some(" 1")] {
            assert!(
                !legacy_v1_from(raw),
                "느슨한 truthy({raw:?})가 안전장치를 뒤집었다 — 형제 게이트와 같은 엄격 비교여야 한다"
            );
        }
        // 스위치가 끄는 것은 **관문 AND 항 하나**다(양성 증거 사다리는 그대로).
        let gates = first_run_gates::builtin();
        let mut o = obs(fixtures::FOLDER_TRUST, fixtures::FOLDER_TRUST, &gates);
        o.agent_alive = Some(true);
        assert!(matches!(judge(&o), Verdict::GateHeld { .. }));
        o.legacy_v1 = true;
        assert_eq!(judge(&o), Verdict::Ready { evidence: Evidence::Valve });
    }

    // ── ⑥ 관문이 있어도 '왜 ready 로 보였는지'가 진단에 남는다 ──────────────
    #[test]
    fn gate_held_reports_the_vetoed_evidence_and_human_only() {
        let gates = first_run_gates::builtin();
        let mut o = obs(fixtures::LOGIN_METHOD, fixtures::LOGIN_METHOD, &gates);
        o.agent_alive = Some(true);
        o.time_fallback_reached = true; // (0.14.31 · H-1 입력 보정) 밸브 창 — vetoed 가 밸브여야 한다
        o.idle_quiet = Some(true);
        match judge(&o) {
            Verdict::GateHeld { gate_id, human_only, vetoed, .. } => {
                assert_eq!(gate_id, "login-method");
                assert!(human_only, "로그인 관문은 사람만 통과시킬 수 있다(실측)");
                assert_eq!(vetoed, Some(Evidence::Valve));
            }
            other => panic!("{other:?}"),
        }
    }

    // ── ⑦ ★순서 핀: U-11 의 GatePending 분기가 실재해야 이 단위가 성립한다 ──
    /// 이 단위는 판정을 **엄격하게** 만든다. 그 엄격화가 안전한 것은 오직 미충족의 귀결이
    /// `close` 가 아니라 `GatePending`(좌석 보존)일 때뿐이다. U-11 이 아직 착지하지 않았는데
    /// 이 파일만 들어오면 **엄격해진 판정이 곧 좌석 파괴**가 된다 — 그 순서를 사람 규율이 아니라
    /// 코드가 강제한다.
    #[test]
    fn u11_gate_pending_branch_exists() {
        let cli = include_str!("bin/cys.rs");
        for anchor in [
            "enum BootVerdict",
            "GatePending { gate: String, tail: String }",
            "fn readiness_timeout_verdict(",
            "fn boot_verdict_effective(",
            "fn mark_gate_pending(",
        ] {
            assert!(
                cli.contains(anchor),
                "U-11(보류 귀결)이 착지하지 않았다 — 앵커 부재: {anchor}. 이 단위(엄격화)를 \
                 먼저 넣으면 미충족이 그대로 close 로 흘러 살아있는 좌석을 죽인다"
            );
        }
        // 보류가 실제로 '닫지 않는' 귀결인지도 본다: close 는 LaunchFailed 아크에만 있어야 한다.
        let i = cli
            .find("Ok(BootVerdict::GatePending { gate, tail }) => {")
            .expect("launch 호출부의 보류 분기");
        // 문자 경계 안전 슬라이스 — 본문이 한글이라 바이트 인덱스로 자르면 패닉한다.
        let arm: String = cli[i..].chars().take(400).collect();
        assert!(
            !arm.contains("surface.close"),
            "보류 분기가 좌석을 닫는다 — 치명위험 ④(전 pane 사망) 방향"
        );
    }

    // ═══════════════════════════════════════════════════════════════════════
    // ★(0.14.31 · WP-1 H-1) 공통 모달 거부 · 밸브 창 — 반례 배터리
    //
    //   픽스처는 **U-12 정본을 변형해** 만든다(잘림·접힘·CRLF·패딩). 관문 needle 을 여기에
    //   문자열로 적으면 문면의 진실원천이 둘이 된다(H-READY-13 ⓑ 적색) — 그래서 변형 도우미만 둔다.
    // ═══════════════════════════════════════════════════════════════════════

    /// 화면의 **아래** n 줄만 남긴다 — 잘린 관문(pane 높이에 질문 줄이 위로 밀려 나간 렌더).
    fn clip_tail(screen: &str, n: usize) -> String {
        let lines: Vec<&str> = screen.lines().collect();
        let start = lines.len().saturating_sub(n);
        let mut out = lines[start..].join("\n");
        out.push('\n');
        out
    }

    fn drop_lines_containing(screen: &str, needle: &str) -> String {
        let mut out = screen
            .lines()
            .filter(|l| !l.contains(needle))
            .collect::<Vec<_>>()
            .join("\n");
        out.push('\n');
        out
    }

    /// ConPTY 가 그리드를 전사할 때의 두 형상 — CRLF 줄끝 · 콘솔 폭까지 우측 공백 패딩.
    fn crlf(s: &str) -> String {
        s.replace('\n', "\r\n")
    }
    fn pad_cols(s: &str, w: usize) -> String {
        let mut out = String::new();
        for l in s.lines() {
            let n = l.chars().count();
            out.push_str(l);
            out.push_str(&" ".repeat(w.saturating_sub(n)));
            out.push('\n');
        }
        out
    }

    /// 부트 관측 — **모든 양성 증거가 열린** 상태(커널 생존 · 맨 셸 아님 · 예산 소진 · 출력 정적 ·
    /// 델타 = 화면). 이 위에서 보류가 나면 그것은 오직 관문/모달 축의 일이다.
    fn boot_all_open<'a>(screen: &'a str, gates: &'a [Gate]) -> Observed<'a> {
        let mut o = obs(screen, screen, gates);
        o.agent_alive = Some(true);
        o.bare_shell = Some(false);
        o.tail_is_shell_prompt = Some(false);
        o.time_fallback_reached = true;
        o.idle_quiet = Some(true);
        o
    }

    fn held_as(v: &Verdict, id: &str) -> bool {
        matches!(v, Verdict::GateHeld { gate_id, .. } if gate_id == id)
    }

    #[test]
    fn modal_signature_recognizes_gate_widgets_including_clipped_and_wrapped_renders() {
        // ① 실측 관문 화면 — OAuth 코드 창(텍스트 입력 프롬프트 · 선택 위젯 없음)만 어휘 밖이다.
        //    그 창은 코퍼스(needle ∧ 인가 URL)가 잡고, URL 까지 잘린 경우는 받아들인 잔여다(HumanOnly ·
        //    Return 의 귀결은 재시도 루프이지 좌석 사망이 아니다).
        for &(id, screen) in GATE_SCREENS {
            let sig = modal_signature(screen);
            if id == "oauth-code" {
                assert!(sig.is_none(), "{id}: 텍스트 입력 창이 모달 어휘로 읽혔다");
            } else {
                assert!(sig.is_some(), "{id}: 관문 화면에서 모달 어휘를 못 봤다");
            }
        }
        // ② 잘린 폴더신뢰 — 질문 줄 소실 · 선택지 두 줄 + 푸터 / 푸터만 / 커서 행만.
        let two = clip_tail(fixtures::FOLDER_TRUST, 3);
        let sig = modal_signature(&two).expect("선택지+푸터");
        assert!(sig.kinds.contains(&"choice-row") && sig.kinds.contains(&"confirm-cancel-footer"));
        assert!(!sig.cursor_on_exit, "커서는 Yes 위인데 exit 위로 읽혔다");
        assert!(modal_signature(&clip_tail(fixtures::FOLDER_TRUST, 1)).is_some(), "푸터 한 줄");
        let cursor_row = fixtures::FOLDER_TRUST
            .lines()
            .find(|l| l.starts_with('❯'))
            .expect("커서 행");
        let sig = modal_signature(cursor_row).expect("커서 행 단독");
        assert!(sig.kinds.contains(&"cursor-on-numbered-item") && sig.kinds.contains(&"choice-row"));
        // ③ 잘린 테마·로그인 — 라벨은 어휘 밖이지만 **커서가 번호 항목 위**라는 사실이 잡는다.
        //    ★받아들인 잔여: 커서 행까지 잘려 번호 행만 남은 화면(`2. …` `3. …`)은 어휘 밖이다 —
        //    번호 목록은 정상 출력에도 흔해 그것을 모달로 읽으면 부트 라이브락 방향이다. pane 이
        //    2줄인 렌더는 실측에 없다(질문 줄이 밀려도 커서 행은 남는다 — dept-3 07:09 형태).
        for (id, base, keep) in [("theme", fixtures::THEME, 2usize), ("login-method", fixtures::LOGIN_METHOD, 3usize)] {
            let clipped = clip_tail(base, keep);
            assert!(
                first_run_gates::identify(&first_run_gates::builtin(), &clipped).is_none(),
                "{id}: 전제 붕괴 — 잘린 화면을 코퍼스가 식별한다면 이 검체는 모달 축을 재지 못한다"
            );
            let sig = modal_signature(&clipped)
                .unwrap_or_else(|| panic!("{id}: 잘린 선택기(커서+번호)를 놓쳤다:\n{clipped}"));
            assert!(sig.kinds.contains(&"cursor-on-numbered-item"), "{id}: {:?}", sig.kinds);
        }
        // ④ 커서가 종료 선택지 위 — 면책 창(실측) · 폴더신뢰에서 커서만 옮긴 화면(2.1.261 기본 포커스형).
        assert!(modal_signature(fixtures::TRUST_ECHO_THEN_DISCLAIMER).unwrap().cursor_on_exit);
        assert!(modal_signature(&with_cursor_on(fixtures::FOLDER_TRUST, 2)).unwrap().cursor_on_exit);
        // ⑤ 접힌 라벨(단어 경계 줄바꿈) + 푸터 부재 — 정규화 공간이 잡는다.
        let wrapped = drop_lines_containing(fixtures::FOLDER_TRUST, "Enter to confirm")
            .replace("Yes, I trust this folder", "Yes, I trust this\n   folder");
        assert!(
            wrapped.contains("this\n   folder"),
            "전제 붕괴: 라벨이 접히지 않았다"
        );
        let sig = modal_signature(&wrapped).expect("접힌 라벨");
        assert!(sig.kinds.contains(&"choice-row"), "{:?}", sig.kinds);
        // ⑥ 소수점은 선택 번호가 아니다.
        assert!(modal_signature("❯ 1.5 hours left\n").is_none());
    }

    #[test]
    fn modal_signature_ignores_confirmation_echo_and_healthy_screens() {
        // ★2026-07-29 킬체인 역방향 — 통과 직후의 확인 에코는 모달이 아니다(보류하면 부트 라이브락).
        let echo = "Yes, I trust this folder ✔\n";
        assert!(modal_signature(echo).is_none(), "확인 에코 한 줄이 모달로 읽혔다");
        let echo_then_welcome = format!("{echo}{}", fixtures::HEALTHY_WELCOME_BOX);
        assert!(modal_signature(&echo_then_welcome).is_none(), "에코 + 환영 배너가 모달로 읽혔다");
        let echo_then_live = format!("{echo}{}", fixtures::LIVE_TUI_AT_PROMPT);
        assert!(modal_signature(&echo_then_live).is_none());
        // 건강한 화면 전량 — 관문이 아닌 화면 표에서 **진짜 모달·본문 표**를 뺀 나머지.
        for &(id, screen) in fixtures::NON_GATE_SCREENS {
            let expect_modal = matches!(id, "audit-log-line" | "live-permission-prompt");
            assert_eq!(
                modal_signature(screen).is_some(),
                expect_modal,
                "{id}: 모달 판정이 기대와 다르다\n{screen}"
            );
        }
        assert!(modal_signature(HEALTHY_BANNER).is_none(), "Windows 실측 정상 배너가 모달로 읽혔다");
        assert!(modal_signature("").is_none());
    }

    /// ★codex P0 봉인 — 잘린 관문 + 신규 `❯` 가 **마커 델타**로 Ready 가 되던 경로. 그리고 밸브·
    /// 마커 화면·시간 폴백 어느 증거가 열려도 같은 화면은 보류다(증거 종류와 무관).
    #[test]
    fn unknown_modal_is_held_regardless_of_evidence_kind() {
        let gates = first_run_gates::builtin();
        let clipped = clip_tail(fixtures::FOLDER_TRUST, 3);
        assert!(
            first_run_gates::identify(&gates, &clipped).is_none(),
            "전제 붕괴: 코퍼스가 잘린 관문을 식별한다면 이 검체는 두 번째 거부를 재지 못한다"
        );
        // 마커 델타 — 잘린 관문의 선택 커서가 신규 출현분에 실렸다(실측 결함의 정확한 형태).
        let mut o = obs(&clipped, &clipped, &gates);
        o.agent_alive = Some(false);
        match judge(&o) {
            Verdict::GateHeld { gate_id, vetoed, human_only, .. } => {
                assert_eq!(gate_id, MODAL_UNKNOWN_ID);
                assert_eq!(vetoed, Some(Evidence::MarkerDelta), "종전 판정의 정체가 진단에 남아야 한다");
                assert!(!human_only);
            }
            other => panic!("잘린 관문 + 신규 ❯ 가 보류되지 않았다(codex P0 경로 재개봉): {other:?}"),
        }
        // 밸브 — 커널 생존 · 맨 셸 아님 · 예산 소진 · 정적.
        let mut v = boot_all_open(&clipped, &gates);
        v.delta = "";
        v.marker = None;
        assert!(matches!(judge(&v), Verdict::GateHeld { vetoed: Some(Evidence::Valve), .. }), "{:?}", judge(&v));
        // 마커 화면 폴백.
        let mut m = obs(&clipped, "", &gates);
        m.agent_alive = Some(false);
        m.time_fallback_reached = true;
        assert!(matches!(judge(&m), Verdict::GateHeld { vetoed: Some(Evidence::MarkerScreen), .. }), "{:?}", judge(&m));
        // 시간 폴백(마커 미정의 어댑터).
        let mut t = obs(&clipped, "", &gates);
        t.agent_alive = Some(false);
        t.marker = None;
        t.time_fallback_reached = true;
        assert!(matches!(judge(&t), Verdict::GateHeld { vetoed: Some(Evidence::TimeFallback), .. }), "{:?}", judge(&t));
        // ★계측 타당성 — 롤백(종전 판정)에서는 같은 입력이 ready 다(고칠 결함이 실재한다).
        let mut legacy = boot_all_open(&clipped, &gates);
        legacy.legacy_v1 = true;
        assert!(judge(&legacy).is_ready(), "종전 판정이 잘린 관문을 ready 로 내지 않았다면 결함이 없다는 뜻");
        // 재주입 경로도 같은 함수를 지난다 — 떠 있는 잘린 관문에 재주입하지 않는다.
        let mut r = obs(&clipped, "", &gates);
        r.site = Site::Reinject;
        r.tail_is_shell_prompt = None;
        r.bare_shell = None;
        assert!(held_as(&judge(&r), MODAL_UNKNOWN_ID), "{:?}", judge(&r));
    }

    /// ★밸브 창 — 감사 에러 4 의 실제 지점(+9.1s < inject_delay 10s 에 밸브가 열렸다).
    #[test]
    fn boot_valve_requires_time_fallback_and_quiet_output() {
        let gates: Vec<Gate> = Vec::new();
        let mut o = obs("살아있는 TUI 를 그리는 중\n", "", &gates);
        o.marker = None; // 마커 축 없음
        o.tail_is_shell_prompt = Some(true); // 시간 폴백(마커 미정의 어댑터)도 막는다 → 밸브만이 유일한 통과 경로
        o.agent_alive = Some(true);
        o.bare_shell = Some(false);
        // 시간 폴백 **전** — 정적이어도 열리지 않는다(dept-3 실측 지점).
        o.time_fallback_reached = false;
        o.idle_quiet = Some(true);
        assert_eq!(judge(&o), Verdict::NotYet, "시간 폴백 전에 밸브가 열렸다(감사 에러 4 재현)");
        // 폴백 도달 · 미관측(구 데몬) — '부재 ≠ 부정'.
        o.time_fallback_reached = true;
        o.idle_quiet = None;
        assert_eq!(judge(&o), Verdict::NotYet, "quiet 미관측인데 밸브가 열렸다");
        // 폴백 도달 · 아직 출력 중.
        o.idle_quiet = Some(false);
        assert_eq!(judge(&o), Verdict::NotYet, "출력이 흐르는 화면에 밸브가 열렸다");
        // 폴백 도달 · 정적 — 열린다(밸브의 존재 이유 · 영구 오부정 차단).
        o.idle_quiet = Some(true);
        assert_eq!(judge(&o), Verdict::Ready { evidence: Evidence::Valve });
        // 롤백 — 종전 밸브(창 없음)로 그대로 돌아간다(새 노브 0).
        o.time_fallback_reached = false;
        o.idle_quiet = None;
        o.legacy_v1 = true;
        assert_eq!(judge(&o), Verdict::Ready { evidence: Evidence::Valve }, "롤백이 종전 밸브를 되살리지 않는다");
    }

    /// 정적 로딩 배너(아직 `❯` 없음) — 예산 전·출력 중에는 보류. 예산 소진 + 정적이면 밸브가 연다:
    /// 그것이 밸브가 지키는 부류(델타 가정이 깨진 살아있는 pane)이고, 여기서 열리지 않으면 영구
    /// 오부정이다. **받아들인 잔여**를 기대값으로 명시한다(codex 설계 검토 Q3).
    #[test]
    fn static_loading_banner_is_not_ready_before_fallback_or_while_output_flows() {
        let gates = first_run_gates::builtin();
        let banner = "─ Claude Code ─\n Welcome back user!   Opus 5 (1M context) · Claude Max\n Loading…\n";
        assert!(first_run_gates::identify(&gates, banner).is_none() && modal_signature(banner).is_none());
        let mut o = obs(banner, banner, &gates);
        o.agent_alive = Some(true);
        o.bare_shell = Some(false);
        o.time_fallback_reached = false;
        o.idle_quiet = Some(true);
        assert_eq!(judge(&o), Verdict::NotYet, "배너 전개 중(예산 전)에 ready");
        o.time_fallback_reached = true;
        o.idle_quiet = Some(false);
        assert_eq!(judge(&o), Verdict::NotYet, "배너가 아직 그려지는데 ready");
        o.idle_quiet = None;
        assert_eq!(judge(&o), Verdict::NotYet, "구 데몬(quiet 미관측)에서 밸브가 열렸다");
        o.idle_quiet = Some(true);
        assert_eq!(judge(&o), Verdict::Ready { evidence: Evidence::Valve }, "정적·예산 소진 배너는 밸브의 대상이다");
    }

    /// 관문 보류 **재관측**의 관측 재료(델타 없음 · 예산 소진 · 같은 응답의 quiet) —
    /// 떠 있는 관문/잘린 모달은 보류, 사람이 통과시킨 프롬프트는 채택.
    #[test]
    fn gate_pending_reobservation_materials_hold_modals_and_adopt_prompts() {
        let gates = first_run_gates::builtin();
        let re = |screen: &str, quiet: Option<bool>| -> Verdict {
            let mut o = obs(screen, "", &gates);
            o.agent_alive = Some(true);
            o.bare_shell = Some(false);
            o.tail_is_shell_prompt = Some(screen.trim_end().ends_with('❯'));
            o.time_fallback_reached = true;
            o.idle_quiet = quiet;
            judge(&o)
        };
        assert!(held_as(&re(fixtures::TRUST_ECHO_THEN_DISCLAIMER, Some(true)), "bypass-disclaimer"));
        let clipped_disclaimer = clip_tail(fixtures::TRUST_ECHO_THEN_DISCLAIMER, 3);
        assert!(
            first_run_gates::identify(&gates, &clipped_disclaimer).is_none(),
            "전제 붕괴: 잘린 면책 창을 코퍼스가 식별한다"
        );
        assert!(
            held_as(&re(&clipped_disclaimer, Some(true)), MODAL_UNKNOWN_ID),
            "잘린 면책 창(커서=No, exit)이 재관측에서 채택됐다 — 그 주입 Return 이 좌석을 죽인다"
        );
        // 사람이 통과시킨 뒤 — 프롬프트 화면. quiet 가 있어야 밸브가 열린다(없으면 보류 유지 · 파괴 0).
        assert_eq!(re(fixtures::LIVE_TUI_AT_PROMPT, Some(true)), Verdict::Ready { evidence: Evidence::Valve });
        assert_eq!(re(fixtures::LIVE_TUI_AT_PROMPT, None), Verdict::NotYet);
        assert_eq!(re(fixtures::LIVE_TUI_AT_PROMPT, Some(false)), Verdict::NotYet);
    }

    /// 정상 프롬프트·확인 에코 화면은 **종전과 똑같이** ready 다(오탐 대조군).
    #[test]
    fn normal_prompt_and_confirmation_echo_stay_ready() {
        let gates = first_run_gates::builtin();
        let echo_then_welcome = format!("Yes, I trust this folder ✔\n{}", fixtures::HEALTHY_WELCOME_BOX);
        for screen in [
            fixtures::LIVE_TUI_AT_PROMPT,
            fixtures::HEALTHY_WELCOME_BOX,
            echo_then_welcome.as_str(),
        ] {
            // 마커 델타(정상 claude 부트의 통상 경로) — 커널 사실 유무와 무관.
            for alive in [Some(true), Some(false), None] {
                let mut o = obs(screen, screen, &gates);
                o.agent_alive = alive;
                assert_eq!(judge(&o), Verdict::Ready { evidence: Evidence::MarkerDelta }, "{screen}");
            }
            // 밸브 경로(델타 없음).
            let mut v = boot_all_open(screen, &gates);
            v.delta = "";
            v.marker = None;
            assert_eq!(judge(&v), Verdict::Ready { evidence: Evidence::Valve }, "{screen}");
        }
    }

    /// ★H-WIN 검체 5종 — ConPTY 전사 형상으로 **파생**한 모달 화면(주장된 캡처가 아니라 변형이다:
    /// CRLF 줄끝 · 콘솔 폭 우측 패딩 · 푸터 조각 분리 · 라벨 접힘+푸터 소실 · PowerShell 프롬프트
    /// 잔상 + 잘린 테마 하단). Windows 실측 근거: `HEALTHY_BANNER`(WIN-2) 의 PS 프롬프트 줄 ·
    /// `docs/plans/2026-07-29-win-two-defects-plan.md:243`(Windows 신뢰창 푸터 두 갈래).
    /// 이 검체는 windows-health.yml 의 `cargo test --lib readiness::` 로 Windows 실기에서 돈다(B-8).
    #[test]
    fn conpty_rendered_modal_variants_are_held() {
        let gates = first_run_gates::builtin();
        let ps_line = HEALTHY_BANNER.lines().next().expect("WIN-2 PS 프롬프트 줄");
        let specimens: Vec<(&str, String)> = vec![
            ("CRLF 폴더신뢰", crlf(fixtures::FOLDER_TRUST)),
            ("우측 패딩 면책 창(120열)", pad_cols(fixtures::TRUST_ECHO_THEN_DISCLAIMER, 120)),
            (
                "푸터 조각 분리(Enter to confirm ·\\r\\n Esc to cancel)",
                crlf(&fixtures::FOLDER_TRUST.replace(" · ", " ·\n ")),
            ),
            (
                "라벨 접힘 + 푸터 소실(CRLF)",
                crlf(&drop_lines_containing(fixtures::FOLDER_TRUST, "Enter to confirm")
                    .replace("Yes, I trust this folder", "Yes, I trust this\n   folder")),
            ),
            (
                "PS 프롬프트 잔상 + 잘린 테마 하단",
                format!("{ps_line}\r\n{}", crlf(&clip_tail(fixtures::THEME, 2))),
            ),
        ];
        assert_eq!(specimens.len(), 5, "H-WIN 검체는 5종이다(CONTRACTS B-8)");
        for (label, screen) in &specimens {
            assert!(modal_signature(screen).is_some(), "{label}: 모달 어휘를 못 봤다:\n{screen:?}");
            let v = judge(&boot_all_open(screen, &gates));
            assert!(
                matches!(v, Verdict::GateHeld { .. }),
                "{label}: 모든 양성 증거가 열린 부트 관측에서 보류가 아니다: {v:?}"
            );
        }
        // 대조군 — 같은 변형을 건강한 화면에 걸면 ready 는 그대로다(변형 자체가 보류를 만들지 않는다).
        for screen in [crlf(fixtures::LIVE_TUI_AT_PROMPT), pad_cols(fixtures::HEALTHY_WELCOME_BOX, 120)] {
            assert!(judge(&boot_all_open(&screen, &gates)).is_ready(), "{screen:?}");
        }
    }

    /// 재주입 생애 창 — 모달 문면은 **대기 프롬프트 뒤의 역사**일 때만 닫힌다(fail-closed).
    #[test]
    fn reinject_modal_window_closes_only_behind_a_waiting_prompt() {
        let gates = first_run_gates::builtin();
        let reinject = |screen: &str, marker: Option<&str>| -> Verdict {
            let mut o = obs(screen, "", &gates);
            o.site = Site::Reinject;
            o.marker = marker;
            o.tail_is_shell_prompt = None;
            o.bare_shell = None;
            o.idle_quiet = Some(true);
            judge(&o)
        };
        // 답한 권한 프롬프트 + 작업 로그 + 대기 프롬프트 — 역사다 → 재주입 가능(영구 미주입 방지).
        let answered = format!("{}\n✓ Bash 완료\n❯ \n", fixtures::LIVE_PERMISSION_PROMPT);
        assert_eq!(reinject(&answered, Some("❯")), Verdict::Ready { evidence: Evidence::MarkerTail });
        // 떠 있는 권한 프롬프트 — 전경이다 → 보류(디렉티브가 권한 선택지에 붙여넣어지면 안 된다).
        assert!(held_as(&reinject(fixtures::LIVE_PERMISSION_PROMPT, Some("❯")), MODAL_UNKNOWN_ID));
        // 부분 렌더 — 푸터 뒤에 선택 커서 행이 다시 그려지는 중(`❯ 2.`) → 마커 뒤에 문면 → 열림 안 함.
        let partial = format!("{}❯ 2.", fixtures::LIVE_PERMISSION_PROMPT);
        assert!(held_as(&reinject(&partial, Some("❯")), MODAL_UNKNOWN_ID));
        // 마커 미정의(codex 등) — 창을 닫을 근거가 없다 → 보류(fail-closed · 받아들인 잔여).
        assert!(held_as(&reinject(&answered, None), MODAL_UNKNOWN_ID));
        assert!(held_as(&reinject(&answered, Some("")), MODAL_UNKNOWN_ID));
        // 부트는 상수로 열려 있다 — 같은 '답한' 화면도 부트 창에서는 보류다(관문 축과 같은 부호).
        assert!(held_as(&judge(&boot_all_open(&answered, &gates)), MODAL_UNKNOWN_ID));
        // 그리고 관문 축의 생애 창 검체 화면(지나간 신기능 안내 + 프롬프트)은 여전히 재주입된다 —
        // 모달 축이 관문 축의 P4-7 수리를 되돌리지 않았다.
        let passed_gate = format!("{}[boot] worker=claude surface=7 rc=0\n작업 로그\n❯ \n", fixtures::FEATURE_FULLSCREEN);
        assert_eq!(reinject(&passed_gate, Some("❯")), Verdict::Ready { evidence: Evidence::MarkerTail });
    }

    /// 어휘 파리티 — 모달 어휘는 코퍼스 `widget`·`confirm_echo` 집합의 **부분집합**이다(두 벌 드리프트 차단).
    /// 관문 needle(질문형)은 어휘에 없다 — 그 사본 금지는 H-READY-13 ⓑ 가 별도로 집행한다.
    #[test]
    fn modal_vocabulary_is_a_subset_of_the_corpus_widget_and_echo_sets() {
        let gates = first_run_gates::builtin();
        let widgets: Vec<&str> = gates.iter().flat_map(|g| g.widget.iter().map(String::as_str)).collect();
        let echoes: Vec<&str> = gates.iter().flat_map(|g| g.confirm_echo.iter().map(String::as_str)).collect();
        let needles: Vec<&str> = gates.iter().flat_map(|g| g.needles.iter().map(String::as_str)).collect();
        for f in MODAL_FOOTER {
            assert!(widgets.contains(&f), "푸터 어휘 {f:?} 가 코퍼스 widget 에 없다(드리프트)");
        }
        for l in MODAL_CHOICE_LABELS {
            assert!(echoes.contains(&l), "선택지 어휘 {l:?} 가 코퍼스 confirm_echo 에 없다(드리프트)");
            assert!(!needles.contains(&l), "선택지 어휘 {l:?} 가 needle 이다 — 에코/라벨은 needle 이 아니어야 한다");
        }
        assert!(echoes.contains(&MODAL_EXIT_LABEL));
        assert!(MODAL_CHOICE_LABELS.contains(&MODAL_EXIT_LABEL));
    }

    #[test]
    fn idle_quiet_from_folds_missing_and_non_finite_to_unobserved() {
        assert_eq!(idle_quiet_from(None), None);
        assert_eq!(idle_quiet_from(Some(f64::NAN)), None);
        assert_eq!(idle_quiet_from(Some(f64::INFINITY)), None);
        assert_eq!(idle_quiet_from(Some(BOOT_VALVE_QUIET_SECS - 0.001)), Some(false));
        assert_eq!(idle_quiet_from(Some(BOOT_VALVE_QUIET_SECS)), Some(true));
        assert_eq!(idle_quiet_from(Some(0.0)), Some(false));
        assert_eq!(idle_quiet_from(Some(120.0)), Some(true));
    }

    /// 관문이 아닌 화면 표를 **판정 전체**로 관통한다 — 코퍼스 층 검체만으로는 새 `judge` 의 거부를
    /// 재지 못한다(codex 설계 검토 Q5). 진짜 모달(권한 프롬프트)과 본문 표(감사 문서)만 보류다.
    #[test]
    fn non_gate_screens_through_judge_hold_only_true_modals() {
        let gates = first_run_gates::builtin();
        for &(id, screen) in fixtures::NON_GATE_SCREENS {
            let v = judge(&boot_all_open(screen, &gates));
            match id {
                "live-permission-prompt" | "audit-log-line" => assert!(
                    held_as(&v, MODAL_UNKNOWN_ID),
                    "{id}: 모달 어휘가 전경인데 보류가 아니다: {v:?}"
                ),
                _ => assert!(v.is_ready(), "{id}: 건강한 화면이 보류로 접혔다(부트 라이브락 방향): {v:?}"),
            }
        }
    }

    /// 롤백은 두 변경(모달 거부 · 밸브 창)을 **함께** 종전으로 되돌린다 — 반쪽 롤백 없음.
    #[test]
    fn legacy_v1_disables_modal_rejection_and_valve_window_together() {
        let gates = first_run_gates::builtin();
        let clipped = clip_tail(fixtures::FOLDER_TRUST, 3);
        let mut o = obs(&clipped, "", &gates);
        o.agent_alive = Some(true);
        o.bare_shell = Some(false);
        o.marker = None;
        o.time_fallback_reached = false;
        o.idle_quiet = None;
        assert!(held_as(&judge(&o), MODAL_UNKNOWN_ID), "신동작: 잘린 관문은 모달 보류다: {:?}", judge(&o));
        // 모달이 아닌 화면에서는 창 전이라 미충족(밸브 창) — 두 축 모두 신동작.
        let mut plain = o.clone();
        plain.screen = "살아있는 TUI 를 그리는 중\n";
        plain.tail_is_shell_prompt = Some(true);
        assert_eq!(judge(&plain), Verdict::NotYet, "신동작: 창 전에는 밸브가 닫힌다");
        plain.legacy_v1 = true;
        assert_eq!(judge(&plain), Verdict::Ready { evidence: Evidence::Valve }, "롤백이 종전 밸브를 되살리지 않는다");
        o.legacy_v1 = true;
        assert_eq!(judge(&o), Verdict::Ready { evidence: Evidence::Valve }, "롤백이 종전 판정을 되살리지 않는다");
        assert_eq!(ModalSignature { kinds: vec!["a", "b"], flat_end: 0, cursor_on_exit: false }.title(), "미등재 모달(a+b)");
    }
}
