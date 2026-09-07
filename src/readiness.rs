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
//! | `CYS_READINESS_V1` | `1` | 이 파일의 관문 AND 항 + (0.14.31 WP-1) 모달 축·밸브 창 + 주입 가드의 **코퍼스 밖 모달 폴백**(`inject_guard::Observed.readiness_legacy`) — 종전 코퍼스 가드·커서-종료 벨트는 제외 |
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
// 【매칭 공간】 라벨 대조(ⓐ 종료 라벨 · ⓒ 선택지 라벨 · ⓑ 푸터)는 전부 **평탄화 공간**(공백 전부 제거)에서
// 한다 — 코퍼스 식별(`first_run_gates::identify`)과 **같은 공간**이다. 단어 안에서 접힌 렌더(`No, ex⏎it` ·
// `Yes, I tru⏎st this folder`)를 정규화 공간(줄바꿈 → 1칸)으로 보면 `No, ex it` 가 되어 벨트를 지나치는데,
// 코퍼스는 그 화면을 여전히 폴더신뢰로 식별하므로 allow 구멍이 열린다(리뷰 R2 · codex blocking). 커서·
// 번호 경계(`❯` · `N.` 앞 경계)만 **정규화 공간**에서 본다 — 평탄화하면 앞 라벨의 꼬리가 번호에 붙어 경계가
// 사라진다. 두 공간은 `pre[]`(norm→flat)·`flat_to_norm[]` 로 잇는다. CRLF 는 두 공간 모두 흡수한다(ConPTY 안전).
//
// 【생애 창】 부트(`Site::Boot`)에서는 **상수로 열려 있다**(관문 축과 같은 근거 — 미탐 = 관문에 주입).
// 재주입(`Site::Reinject`)에서는 **전경 판정**으로만 닫힌다: 축 ② 모달 문면 전량이 마커보다 앞(역사)
// ∧ 축 ①' 마커의 마지막 출현 **줄**이 빈 대기 프롬프트(마커 뒤 그 줄에 공백만)이고 그 아래 꼬리가
// 입력 상자·상태줄 레이아웃이다([`waiting_prompt_with_harmless_trailer`] · 리뷰 R1). 관문 축
// (`gate_block_left_behind`)의 축 ①은 "마커 뒤 화면 전체가 공백" 인데, 라이브 claude 2.1.261 그리드는
// 입력 상자 **아래**에 상태줄을 그리므로(실측 2026-09-06 10:18) 그 축은 라이브 좌석에서 영영 닫히지
// 않는다 — 관문 축은 needle ∧ 위젯이라 좁아 그대로 두고(P4-11 핀 · 백로그), needle 요구가 없어 훨씬
// 넓은 이 축만 줄 단위로 재정의한다. 마커 미정의(codex)·마커 부재는 **닫지 않는다**(fail-closed).
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

/// ★(리뷰 R1) 재주입 생애 창의 **꼬리 레이아웃 증거** — 대기 프롬프트 줄 아래에 실려도 무해한 상태줄 어휘.
///
/// 실측(2026-09-06 10:18:58 · claude 2.1.261 라이브 좌석 `cys read-screen` · 읽기 전용): 입력 상자는 `❯ ` 줄이
/// 위아래 괘선(`────…`)으로 둘러싸이고, 그 아래 상태줄 `⏵⏵ bypass permissions on (shift+tab to cycle) ·
/// ← for agents` 가 온다(사용자 statusLine 한 줄이 그 위에 더 올 수 있다). 2.1.241 검체(`LIVE_TUI_AT_PROMPT`)
/// 는 `? for shortcuts`. 이 어휘는 **보류 근거가 아니라 창을 닫는 쪽의 양성 증거**이고, 관문 needle 이
/// 아니다(H-READY-13 ⓑ — 상태줄 문면). 어휘 밖의 꼬리(사용자 statusLine 만 있는 경우 등)는 괘선이
/// 첫 줄이면 통과하고, 그것도 없으면 **닫지 않는다**(= 종전과 같이 보류 · 조여지는 방향).
pub const PROMPT_TRAILER_TOKENS: [&str; 3] = ["for shortcuts", "bypass permissions", "shift+tab"];
/// 괘선 줄로 인정하는 박스 문자(U+2500..=U+259F) **연속 길이** 하한 — cys.rs `TUI_FRAME_RUN_MIN`(맨 셸 술어의
/// 프레임 자)과 같은 값이어야 한다(파리티 핀 `prompt_trailer_rule_run_matches_tui_frame_run_min` · cys.rs 테스트).
pub const PROMPT_TRAILER_RULE_MIN_RUN: usize = 8;
/// ★(0.14.31 · 리뷰 R2(R7회차) · codex blocking B2) 꼬리가 빈 분기에서 상태줄을 **이 composer 의 것**으로
/// 인정하는 최대 거리(행). 실측 2.1.241 레이아웃은 2행(`? for shortcuts` → `…43% context left` → `❯ `)이고,
/// 사용자 statusLine 한 줄이 더 낄 수 있어 여유를 둔다. 그보다 멀면 스크롤백의 역사로 본다(조여지는 방향).
pub const PROMPT_STATUS_ABOVE_MAX_ROWS: usize = 4;

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

/// 화면 한 장의 **매칭 좌표계 묶음** — 정규화(norm) · 평탄화(flat) · 원문(raw) 셋과 그 사이의 접두 합.
///
/// ★(0.14.31 · 리뷰 R3) 세 벌을 함수마다 따로 만들면 소비처가 갈리는 순간 "두 판정기가 다른 공간을 본다" 는
///   R2 blocking 이 그대로 재발한다. 한 곳에서 만들어 [`modal_signature`]·[`cursor_resolves_to_label`] 이 공유한다.
///
/// ★(0.14.31 · 리뷰 R4 · codex blocking) 여기에 **원문(raw)** 축이 더해졌다. 정규화 공간은 개행까지 공백 하나로
///   접기 때문에 **물리 행 경계가 없고**, 그래서 "이 커서 뒤에 같은 줄의 글자가 있는가" 를 물을 수 없었다.
///   경쟁 커서 판정이 어휘(라벨 앞머리 n 자)에 의존하던 근본 이유가 그것이다 — 어휘로 재면 어휘 밖으로 잘린
///   렌더(`❯ No` · `❯ 2`)가 전부 새어 나간다. 평탄화(flat) 열은 세 축이 **같은 하나**다(`flatten` = 모든 공백 제거).
struct Frame {
    norm: Vec<char>,
    raw: Vec<char>,
    /// raw 위치 → flat 위치 접두 합.
    raw_pre: Vec<usize>,
    flat: Vec<char>,
}

fn screen_frame(screen: &str) -> Frame {
    let norm: Vec<char> = first_run_gates::normalize(screen).chars().collect();
    let raw: Vec<char> = screen.chars().collect();
    let mut raw_pre: Vec<usize> = Vec::with_capacity(raw.len() + 1);
    raw_pre.push(0);
    for c in &raw {
        raw_pre.push(raw_pre[raw_pre.len() - 1] + usize::from(!c.is_whitespace()));
    }
    let flat: Vec<char> = raw.iter().copied().filter(|c| !c.is_whitespace()).collect();
    Frame { norm, raw, raw_pre, flat }
}

/// 선택 커서(`❯`) 한 행의 좌표.
struct CursorRow {
    /// 커서 문자 자신의 평탄화 위치(= 그 앞의 비공백 문자 수). **위치 비교**의 기준이다.
    cursor_flat: usize,
    /// 선택 번호(`N.`)의 평탄화 끝(엄격 파서 · 뒤가 공백/문말일 때만).
    numbered_flat_end: Option<usize>,
    /// 라벨이 시작하는 평탄화 위치(번호가 있으면 그 뒤 · 없으면 커서 뒤 첫 비공백).
    /// **화면 끝까지 공백뿐이면 `flat.len()`** — 그것이 "라벨이 없다"(빈 입력 프롬프트)의 구조적 표지다.
    label_flat: usize,
    /// ★(0.14.31 · 리뷰 R1(R6회차) · codex blocking) 이 커서 행이 이루는 **선택 행 블록의 끝**(평탄화).
    /// 커서 행부터 시작해 다음 줄이 [`is_choice_tail_boundary`](빈 줄 · 괘선 · 번호 항목 행 · 다른 선택
    /// 커서 행)가 아닌 동안 이어붙인다 — 즉 **줄바꿈 접힘은 흡수하고, 무관한 아래 문면은 자른다**.
    tail_end_flat: usize,
    /// ★(0.14.31 · 리뷰 R2(R7회차) · codex blocking B1) 라벨이 시작하는 **물리 행**의 끝(평탄화).
    /// 잘린 렌더의 판정 재료는 이 한 행이고([`clipped_choice_cursor`] 규칙 ①), 블록의 나머지는
    /// **완결 증거**(접힌 라벨의 이어짐)로만 쓴다 — 그래야 무관한 꼬리 한 줄이 거부를 취소하지 못한다.
    row_end_flat: usize,
}

/// ★(0.14.31 · 리뷰 R1(R6회차) · codex blocking) 선택 행 꼬리가 **여기서 끝난다**는 경계 줄인가.
///
/// 【왜 필요한가】 R5 는 잘린 선택기 판정의 재료를 "커서 뒤 **화면 끝까지**" 로 잡았다. 접힌 확인
/// 에코(`❯ Yes, I trust this fol⏎der ✔`)의 첫 행이 진부분접두가 되어 통과 후 화면을 관문으로 오탐하는
/// **역방향 회귀**(2026-07-29 킬체인)를 막기 위해서였다. 그러나 그 대가로 **꼬리에 무관한 줄이 하나만
/// 이어져도 거부가 통째로 취소된다** — codex 가 든 첫 프레임 `❯ No, exi` + 아래 괘선이 정확히 그것이고,
/// 그 화면에 본문 + Return 이 나가면 커서는 종료 선택지 위다(좌석 사망).
///
/// 【장치】 판정 재료를 **선택 행 블록**으로 좁힌다: 줄바꿈으로 접힌 라벨은 계속 이어 붙이되(에코 보호
/// 유지), 아래 줄이 **다른 시각 요소**이면 거기서 끊는다. 경계는 넷 — 빈 줄 · 괘선(입력 상자 테두리) ·
/// 번호 항목 행 · 다른 선택 커서 행 · 상태줄([`PROMPT_TRAILER_TOKENS`]). 다섯 다 "접힌 라벨의 이어짐"
/// 으로 볼 수 없는 형상이다(상태줄 어휘는 [`scan_composer`] 의 양성 증거와 **같은 코퍼스**다 —
/// 그 줄이 나왔다는 것은 입력 상자 블록이 끝났다는 뜻이다).
fn is_choice_tail_boundary(line: &str) -> bool {
    if line.trim().is_empty() || is_rule_line(line) || is_numbered_item_row(line) || line.contains('❯') {
        return true;
    }
    let norm = first_run_gates::normalize(line).to_lowercase();
    PROMPT_TRAILER_TOKENS.iter().any(|t| norm.contains(t))
}

/// `from` 이 속한 **물리 행의 끝**(개행 앞 · 원문 좌표). `from` 이 원문 끝이면 원문 길이.
fn line_end_of(raw: &[char], from: usize) -> usize {
    let mut j = from;
    while j < raw.len() && raw[j] != '\n' {
        j += 1;
    }
    j
}

/// 커서 행에서 시작하는 선택 행 블록의 **원문 끝 위치**([`is_choice_tail_boundary`] 앞에서 멎는다).
fn choice_tail_end(raw: &[char], cursor: usize) -> usize {
    let line_end = |from: usize| -> usize { line_end_of(raw, from) };
    let mut end = line_end(cursor);
    while end < raw.len() {
        let start = end + 1; // '\n' 다음 줄
        let next_end = line_end(start);
        let line: String = raw[start.min(raw.len())..next_end].iter().collect();
        // ★CRLF: 줄 끝 `\r` 을 벗긴다(`str::lines()` 와 같은 규율). 벗기지 않으면 번호 항목 행
        //   판정(`is_numbered_item_row` — `N.` 뒤가 공백/문말)이 `\r` 때문에 거짓이 되어 Windows·
        //   ConPTY 전사에서만 경계를 놓친다(codex 위임 검체가 실제로 이 결함을 잡았다).
        let line = line.strip_suffix('\r').unwrap_or(&line);
        if is_choice_tail_boundary(line) {
            return end;
        }
        end = next_end;
    }
    raw.len()
}

/// 화면의 선택 커서 행 전량(등장 순). 규칙은 [`modal_signature`] ⓐⓓ 와 **같은 스캐너**다 —
/// 판정 분리 금지(allow 구멍의 양성 증거와 모달 서명이 다른 스캐너를 쓰면 구멍이 생긴다).
///
/// ★스캔은 **원문**에서 한다(리뷰 R4). 커서 뒤 공백 건너뛰기는 개행도 건너뛰므로 접힌 라벨
///   (`❯\n  1. Yes …`)은 종전 정규화 공간과 **같은 결과**를 낸다 — 바뀐 것은 물리 행 경계를
///   함께 얻는다는 것뿐이다.
fn cursor_rows(f: &Frame) -> Vec<CursorRow> {
    let raw = &f.raw;
    let mut rows = Vec::new();
    for (i, &c) in raw.iter().enumerate() {
        if c != '❯' {
            continue;
        }
        let mut j = i + 1;
        while j < raw.len() && raw[j].is_whitespace() {
            j += 1;
        }
        // 선택 번호 `N.`(1~2자리) — 뒤가 공백이거나 문말이어야 번호다(`❯ 1.5 hours` 는 아니다).
        let mut k = j;
        while k < raw.len() && raw[k].is_ascii_digit() && k - j < 2 {
            k += 1;
        }
        let numbered_end = (k > j
            && k < raw.len()
            && raw[k] == '.'
            && (k + 1 == raw.len() || raw[k + 1].is_whitespace()))
        .then_some(k + 1);
        let mut label_start = j;
        if let Some(ne) = numbered_end {
            label_start = ne;
            while label_start < raw.len() && raw[label_start].is_whitespace() {
                label_start += 1;
            }
        }
        rows.push(CursorRow {
            cursor_flat: f.raw_pre[i],
            numbered_flat_end: numbered_end.map(|ne| f.raw_pre[ne]),
            label_flat: f.raw_pre[label_start],
            // 꼬리 경계는 **커서 행**에서 잰다(라벨 시작 행이 아니다 — 라벨 탐색은 개행을 건너뛰므로
            // 경계 너머의 글자를 라벨로 집을 수 있고, 그 경우 `label_flat >= tail_end_flat` 로 걸린다).
            tail_end_flat: f.raw_pre[choice_tail_end(raw, i)],
            // 행 끝은 **라벨이 시작하는 행**에서 잰다(커서 행이 아니다 — `❯⏎  1. Yes …` 처럼 라벨이
            // 다음 줄로 접힌 렌더에서 커서 행 끝을 쓰면 '라벨 한 글자도 없음' 이 되어 규칙 ①이 죽는다).
            row_end_flat: f.raw_pre[line_end_of(raw, label_start)],
        });
    }
    rows
}

/// 이 커서 행이 **선택을 모호하게 만드는가**(해소된 행이 아닌 경쟁 커서).
///
/// 【어휘를 버린 이유(리뷰 R4 · codex blocking)】 R3b 는 "코퍼스 라벨 앞머리 4자로 시작하면 경쟁" 이었다.
/// 그 규칙은 어휘를 못 알아본 렌더를 **면제**한다 — `❯ No`(3자) · `❯ No,` 뒤에 푸터가 붙어 `No,E` 가 되는 렌더 ·
/// 번호만 남은 `❯ 2` 가 전부 새어 나가고, 그때 allow 구멍이 열려 Return 이 종료 선택지를 누른다(좌석 사망).
/// 그래서 판정 재료를 **구조**로 바꾼다:
///   ⓐ 번호 붙은 커서 행(`❯ N.`)·점 뒤 공백 없는 번호 행(`❯ 2.No`)은 그 자체가 선택기의 행이다(위치 무관).
///   ⓑ **선택 블록 안**(질문 문면이 처음 나타나는 자리 이후)의 커서는, 그 뒤에 글자가 하나라도 있으면 전부
///      경쟁이다 — 어휘를 묻지 않는다.
///
/// 【면제는 둘뿐이고 둘 다 구조적이다】
///   · **라벨이 아예 없다**(`label_flat == flat.len()`): 커서 뒤로 화면 끝까지 공백뿐이다 = 꼬리의 빈 입력
///     프롬프트(`❯ ` · p10k `╰─❯ ` · 실측 NBSP 꼬리). 선택할 것이 없으므로 Return 이 선택지를 누를 수 없다.
///     ★(리뷰 R4 · codex) **행 단위 공백**으로 재면 안 된다 — `❯⏎  No`(다음 줄로 접힌 선택지)가 빈 프롬프트로
///     오인돼 구멍이 열린다. 그래서 "그 행의 나머지" 가 아니라 **화면 끝까지**를 본다.
///   · **선택 블록보다 앞이다**: 그 관문의 질문이 나오기 **전**의 커서는 이 관문의 선택지가 아니다(부트 로그·
///     스크롤백 셸 프롬프트 `❯ claude`). 블록 경계 자체를 어떻게 잡는지는 [`choice_block_start`] 참조 —
///     **가장 이른** 질문 일치를 쓰는 것이 이 면제를 좁게 유지하는 장치다.
fn cursor_row_competes(row: &CursorRow, flat: &[char], block_start: usize) -> bool {
    if row.numbered_flat_end.is_some() {
        return true;
    }
    // ⓐ' **점 뒤 공백이 없는** 번호 행(`❯ 2.No, exi`) — 엄격 파서(ⓓ)는 `N.` 뒤에 공백/문말을 요구해 이 렌더를
    //    번호로 세지 않는다(`❯ 1.5 hours` 를 배제하려는 규칙이다). 경쟁 판정에서는 **글자가 이어지는 경우만**
    //    번호로 본다 — `2.No` 는 선택지 행, `1.5` 는 아니다(숫자가 이어진다).
    let compact_numbered = {
        let mut k = row.label_flat;
        while k < flat.len() && flat[k].is_ascii_digit() && k - row.label_flat < 2 {
            k += 1;
        }
        k > row.label_flat && k + 1 < flat.len() && flat[k] == '.' && flat[k + 1].is_alphabetic()
    };
    if compact_numbered {
        return true;
    }
    if row.label_flat >= flat.len() {
        return false; // 라벨 없음 = 빈 입력 프롬프트(선택기가 아니다)
    }
    row.cursor_flat >= block_start
}

/// 화면에서 **활성 선택 블록이 시작하는** 평탄화 위치 — 이 뒤의 커서는 전부 그 관문의 선택 후보로 본다.
///
/// 여러 anchor(그 관문의 질문형 needle) 중 **가장 이른 곳에서 시작하는** 것을 쓴다.
///
/// ★(리뷰 R4 · codex) 왜 "마지막 일치의 끝" 이 아니라 "가장 이른 일치의 시작" 인가: 재그리기 잔상이 섞인 화면
///   (질문 조각 → `❯ No` → 같은 관문의 다른 질문 문면 → `❯ 1. Yes …`)에서 마지막 anchor 를 쓰면 잔상
///   `❯ No` 가 질문보다 앞이 되어 면제된다. "질문보다 앞" 은 **셸이라는 증거가 아니다** — 가장 이른 질문을
///   경계로 잡아야 그 관문의 문면이 시작된 뒤의 커서가 하나도 새지 않는다. 면제되는 것은 관문 문면이 아직
///   한 글자도 나오지 않은 구간(부트 로그·셸 스크롤백)뿐이다.
///
/// 하나도 못 찾으면 `0` — 블록 경계를 모른다는 뜻이고, 그때는 **화면 전체가 블록**이다(fail-closed).
fn choice_block_start(flat: &[char], anchors: &[&str]) -> usize {
    let mut start: Option<usize> = None;
    for a in anchors {
        let af: Vec<char> = first_run_gates::flatten(a).chars().collect();
        if af.is_empty() {
            continue;
        }
        if let Some(p) = find_all_chars(flat, &af).first().copied() {
            start = Some(start.map_or(p, |s: usize| s.min(p)));
        }
    }
    start.unwrap_or(0)
}

/// ★(0.14.31 · 리뷰 R3 · codex blocking) 이 화면의 선택이 **모호함 없이 그 라벨로 해소되는가**
/// (평탄화 공간 · 완전 일치 접두 · 활성 선택 블록 안의 경쟁 커서 0).
///
/// 【왜 필요한가】 `inject_guard` 의 폴더신뢰 allow 구멍은 종전에 "종료 라벨 위가 **아니면** 연다"(부정 증거)였다.
/// 잘린 렌더(`❯ 2. No, exi` · `❯ 2.No, exi` · 번호 없는 `❯ No, exi`)는 종료 확정이 아니므로 구멍이 열렸고, 그
/// Return 이 종료 선택지를 눌러 좌석을 죽인다(2.1.261 폴더신뢰 기본 포커스 = `No, exit`). 그래서 구멍의 조건을
/// **양성 증거**로 뒤집는다: 커서가 코퍼스가 선언한 **액션 라벨 전문** 위에 있을 때만 연다. 잘림·미관측·모호는
/// 전부 보류(= 좌석 보존 · 키 0 · 사람 처방)로 접힌다.
///
/// 【모호는 양성을 이긴다(리뷰 R3b·R4 · codex)】 액션 라벨 위의 커서 하나로는 부족하다 — 같은 화면의 **질문 뒤**에
/// 해소되지 않은 다른 선택 커서가 있으면 어느 쪽이 실제 선택인지 모른다. 그때는 양성 증거가 있어도 닫는다
/// ([`cursor_row_competes`]). 면제는 **구조적 증거**로만 한다(질문보다 앞 · 뒤가 공백뿐인 커서) — 어휘로 면제하면
/// 어휘 밖으로 잘린 렌더가 그대로 새어 나간다(R4 blocking 의 정확한 자리).
///
/// 【anchors】 그 관문의 질문형 needle 들. 활성 선택 블록의 시작을 여기서 잰다([`choice_block_start`]) —
/// 관문 문면이 **한 글자도 나오기 전**의 커서(부트 로그·스크롤백 셸 프롬프트 `❯ claude` · p10k)만 면제되고,
/// 그래서 자동확인 가용성이 살아 있다. 빈 목록·미발견이면 화면 전체를 블록으로 본다(가장 조이는 쪽).
///
/// 【H-2 와의 결속】 WP-1 H-2 는 액션(아래 방향키 N발) **뒤에** 이 술어를 다시 재고 Return 을 보낸다 — 그때
/// 화면은 `❯ … Yes, I trust this folder` 하나뿐이므로 구멍이 정확히 열린다(액션 경로는 막히지 않는다).
///
/// 【받아들인 잔여】 커서가 화면에 **하나도 없는** 렌더는 이 술어로 판정할 수 없다 → 거짓(= 보류). 그 가용성
/// 대가(`❯` 를 그리지 않는 렌더에서 자동확인이 열리지 않는다)는 좌석 사망보다 싸다(§3-3).
pub fn cursor_resolves_to_label(screen: &str, label: &str, anchors: &[&str]) -> bool {
    let lf: Vec<char> = first_run_gates::flatten(label).chars().collect();
    if lf.is_empty() {
        return false;
    }
    let f = screen_frame(screen);
    let block = choice_block_start(&f.flat, anchors);
    let rows = cursor_rows(&f);
    // 양성 증거도 **블록 안**에서만 인정한다 — 이전 화면의 `❯ … Yes …` 잔상이 새 질문 뒤의 잘린 선택을
    // 승인하는 경로를 막는다(리뷰 R4 · codex).
    let starts_here = |r: &CursorRow| -> bool {
        r.cursor_flat >= block
            && r.label_flat + lf.len() <= f.flat.len()
            && f.flat[r.label_flat..r.label_flat + lf.len()] == *lf
    };
    rows.iter().any(starts_here)
        && !rows
            .iter()
            .any(|r| !starts_here(r) && cursor_row_competes(r, &f.flat, block))
}

/// ★(0.14.31 · 리뷰 R5 · codex blocking) **잘린/번호 없는 선택기 커서 행**인가 —
/// 커서 뒤 꼬리(평탄화 · **화면 끝까지**)로 판정한다. `Some((규칙 라벨, flat 끝))` 이면 모달이다.
///
/// 【무엇이 열려 있었나(R4 잔여 blocking)】 R4 는 **확인(Return) 허가**만 좁혔다. 주입 허가
/// ([`crate::inject_guard::decide_allowing`])와 부트 준비 판정([`judge`])은 그대로였고, 그래서
/// 화면이 `❯ No, exi`(질문·번호·푸터 없음)뿐인 잘린 렌더에서 코퍼스 식별도 모달 서명도 서지 않아
/// **Ready{MarkerDelta} → 디렉티브 붙여넣기 + Return** 이 부분 렌더된 종료 선택지로 나갔다.
/// 자동확인만 막고 정상 주입 경로를 열어 두면 킬체인은 그대로다 — 그래서 거부를 **공용 서명**에 둔다.
///
/// 【판정 재료가 왜 "선택 행 블록의 꼬리" 인가 — R5 의 '화면 끝까지' 를 좁힌다】 물리 행 끝으로 자르면
/// 좁은 pane 에서 접힌 **확인 에코**(`❯ Yes, I trust this fol⏎der ✔`)의 첫 행이 진부분접두가 되어 통과 후
/// 화면을 관문으로 오탐한다(2026-07-29 킬체인의 **역방향** 회귀 · 정본 요구). 그래서 R5 는 꼬리를 화면
/// 끝까지 봤는데, 그러면 **꼬리에 무관한 줄이 하나만 이어져도 거부가 통째로 취소된다** — codex 가 든
/// 첫 프레임 `❯ No, exi` + 아래 괘선이 정확히 그 구멍이다(그 화면에 본문 + Return 이 나가면 커서는
/// 종료 선택지 위다). 지금은 **선택 행 블록**([`choice_tail_end`])까지만 본다: 접힌 라벨은 계속 이어
/// 붙이고(에코 보호 유지), 괘선·빈 줄·번호 항목 행·다른 커서 행에서 끊는다(무관한 꼬리로 취소 불가).
/// 잘려 보이지만 화면 끝까지의 꼬리에서 **라벨이 완결된** 경우(접힌 에코)는 아래 '완결 증거' 가 면제한다.
///
/// 【규칙】 꼬리에서 선택 번호(`N.`/`NN.` — 뒤가 글자이거나 문말일 때만 · `1.5` 배제)를 벗긴 뒤:
///   · 비어 있지 않은 **진부분접두**([`MODAL_CHOICE_LABELS`] 중 하나보다 짧고 그 앞부분과 일치)
///     → `clipped-choice-row`(렌더가 라벨 도중에 멎었다).
///   · **완전 라벨로 시작**(번호가 없어 ⓒ 가 못 잡고, 종료 라벨이 아니어서 ⓐ 도 못 잡는
///     `❯ Yes, try it` · `❯ Not now`) → `cursor-on-choice-label`.
///   · 번호를 벗긴 나머지가 비었거나 꼬리가 **숫자 1~2자뿐**(`❯ 2`) → `clipped-choice-row`.
/// 라벨이 아예 없는 커서(`❯ ` 뒤로 화면 끝까지 공백)는 **빈 composer** 이므로 걸리지 않는다.
///
/// 【어휘를 다시 쓰는 것이 R4 의 결정을 뒤집는가 — 아니다】 R4 가 지운 어휘 규칙은
/// [`cursor_row_competes`] 의 **면제**(fail-open)였다: 어휘를 못 알아본 렌더가 경쟁에서 빠져 구멍이
/// 열렸다. 여기 어휘는 **거부**(fail-closed)다 — 못 알아본 렌더는 종전과 똑같이 흐르고, 알아본
/// 렌더만 보류로 접힌다. 방향이 반대이므로 같은 실패 모드가 아니다.
///
/// 【남는 것(정직 · 노트 §14-3)】 커서만 그려지고 라벨이 **아직 한 글자도 오지 않은** 프레임
/// (`❯ ` 뒤 화면 끝)은 정상 빈 composer 와 **같은 관측**이라 이 술어로 가를 수 없다. 그 창은
/// 부트 폴링의 관문 증거 이월(`cys.rs` 의 `gate_evidence_seen`)이 좁히고, 폴링 틱 사이에만
/// 존재했다 사라진 프레임은 화면·시간만으로는 판정 불가다(codex R5 · 종료 조건 없음).
/// 선택 번호(`N.`/`NN.`)를 벗긴 나머지 — 뒤가 글자이거나 문말일 때만 번호로 본다(`1.5 hours` 배제).
fn strip_choice_number(tail: &[char]) -> &[char] {
    let mut d = 0usize;
    while d < tail.len() && d < 2 && tail[d].is_ascii_digit() {
        d += 1;
    }
    if d > 0 && d < tail.len() && tail[d] == '.' && (d + 1 == tail.len() || tail[d + 1].is_alphabetic()) {
        &tail[d + 1..]
    } else {
        tail
    }
}

fn clipped_choice_cursor(
    flat: &[char],
    label_flat: usize,
    row_end: usize,
    tail_end: usize,
) -> Option<(&'static str, usize)> {
    let end = tail_end.min(flat.len());
    if label_flat >= end {
        // 라벨 없음 = 빈 입력 프롬프트 · 또는 라벨이 **블록 경계 너머**다(커서 행 아래의 괘선·다른 요소를
        // 라벨로 집은 경우 — 그것은 이 커서의 선택지가 아니다).
        return None;
    }
    let tail = &flat[label_flat..end];
    // 숫자만 남은 꼬리(`❯ 2`) — 렌더가 번호에서 멎었다.
    if tail.len() <= 2 && tail.iter().all(|c| c.is_ascii_digit()) {
        return Some(("clipped-choice-row", end));
    }
    let body = strip_choice_number(tail);
    if body.is_empty() {
        return Some(("clipped-choice-row", end)); // `❯ 2.` — 번호만 그려졌다
    }
    // ★(0.14.31 · 리뷰 R2(R7회차) · codex blocking B1 + claude major) **부분 라벨은 행에서 재고,
    //   완결 증거는 블록 안에서만 센다.**
    //
    //   R6 은 판정 재료를 선택 행 블록으로 좁히고 면제(완결 증거)를 **화면 끝까지**로 뒀다. 두 좌표계가
    //   어긋나서 두 구멍이 남았다:
    //     ⓐ 블록이 경계 없이 이어지는 꼬리(평범한 다음 줄)가 그대로 `body` 에 붙어 진부분접두가 깨졌다
    //        → `❯ No, exi⏎  이어지는 출력` 이 서명 0(= 본문 + Return 이 종료 선택지로 나간다).
    //     ⓑ 블록 **경계 너머**의 글자가 라벨을 완성해 면제를 발동시켰다
    //        → `❯ No, exi⏎⏎to continue press enter` 가 서명 0.
    //   지금은 **라벨이 시작한 물리 행**(`row`)이 진부분접두인지를 묻고, 면제는 **같은 블록 안에서**
    //   그 라벨이 실제로 완성됐는지로만 준다(접힌 확인 에코 보호는 그대로 — 접힘은 경계가 아니라 블록
    //   안에서 이어진다). 무관한 문면은 어느 쪽에서도 **완결의 증거가 아니다**.
    let row = &flat[label_flat..row_end.clamp(label_flat, end)];
    let row_body = strip_choice_number(row);
    // ★완결 증거는 **라벨 전체 집합에 대해 한 번** 센다(라벨마다 따로 세면 안 된다 — codex 설계 검토
    //   R7 반례: 접힌 에코 `❯ Yes, I⏎accept ✔`의 첫 행 `Yes,I` 는 *다른* 라벨(`Yes, I trust this folder`)의
    //   진부분접두라, 그 라벨의 완결만 보면 면제가 서지 않아 **완결된 에코가 관문으로 오탐**된다 =
    //   2026-07-29 킬체인의 역방향 회귀). 좌표계는 화면 끝이 아니라 **이 블록의 꼬리**다(claude major).
    let completed = MODAL_CHOICE_LABELS.iter().any(|label| {
        let lf: Vec<char> = first_run_gates::flatten(label).chars().collect();
        !lf.is_empty() && body.len() >= lf.len() && body[..lf.len()] == lf[..]
    });
    for label in MODAL_CHOICE_LABELS {
        let lf: Vec<char> = first_run_gates::flatten(label).chars().collect();
        if lf.is_empty() {
            continue;
        }
        if !completed && !row_body.is_empty() && row_body.len() < lf.len() && *row_body == lf[..row_body.len()]
        {
            return Some(("clipped-choice-row", end));
        }
        // 완전 라벨은 **꼬리 전량과 같을 때만** 센다(뒤에 다른 글자가 이어지면 아니다). `starts_with`
        // 로 넓히면 좁은 pane 에서 접힌 **확인 에코**(`❯ Yes, I trust this fol⏎der ✔⏎Welcome back`)의
        // 꼬리가 라벨로 시작해 통과 후 화면이 관문으로 오탐된다 — 2026-07-29 킬체인의 역방향 회귀다
        // (codex R5 반례). 종료 라벨의 '커서가 종료 위' 축(ⓐ)은 종전대로 접두로 본다(용도가 다르다).
        if *body == lf[..] {
            return Some(("cursor-on-choice-label", label_flat + lf.len()));
        }
    }
    None
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
///
/// ★(0.14.31 · 리뷰 R3) 커서 행 스캐너는 [`cursor_rows`] 하나이고 좌표계는 [`screen_frame`] 하나다 —
///   allow 구멍의 양성 증거([`cursor_resolves_to_label`])와 이 서명이 **같은 것을 본다**(판정 분리 금지).
///   좌표계도 하나다([`screen_frame`] — 정규화·평탄화·원문 셋을 한 번에 만든다).
pub fn modal_signature(screen: &str) -> Option<ModalSignature> {
    let f = screen_frame(screen);
    let (norm, flat) = (&f.norm, &f.flat);
    let mut sig = ModalSignature {
        kinds: Vec::new(),
        flat_end: 0,
        cursor_on_exit: false,
    };
    // ★(리뷰 R2 · codex blocking) 라벨 대조는 **평탄화 공간**에서 한다 — 접힌 렌더(`No, ex⏎it` · CRLF 동일)는
    //   정규화 공간에서 `No, ex it` 가 되어 종전 ⓐ 를 지나쳤고, 코퍼스 식별(`identify` · 평탄화)은 그 화면을
    //   여전히 폴더신뢰로 읽어 allow 구멍이 열렸다(자동확인 Return = 종료 선택 = 좌석 사망). 두 판정기가 같은
    //   공간을 봐야 벨트에 구멍이 없다. norm→flat 은 `pre[]`, flat→norm 은 `flat_to_norm[]` 이 잇는다.
    let flat_to_norm: Vec<usize> = norm
        .iter()
        .enumerate()
        .filter(|(_, c)| **c != ' ')
        .map(|(i, _)| i)
        .collect();
    let exit_flat: Vec<char> = first_run_gates::flatten(MODAL_EXIT_LABEL).chars().collect();
    let starts_at_flat = |fl: usize, label: &[char]| -> bool {
        fl + label.len() <= flat.len() && flat[fl..fl + label.len()] == *label
    };

    // ⓐ·ⓓ·ⓔ·ⓕ — 선택 커서 행(스캐너는 `cursor_rows` 하나 · allow 구멍의 양성 증거와 같은 것을 본다).
    for row in cursor_rows(&f) {
        if let Some(ne) = row.numbered_flat_end {
            sig.note("cursor-on-numbered-item", ne);
        }
        // 커서 뒤 첫 비공백 문자부터 평탄화 공간에서 `No,exit` — 단어 안 줄바꿈·CRLF·열 정렬 전부 흡수.
        let fl = row.label_flat;
        if starts_at_flat(fl, &exit_flat) {
            sig.cursor_on_exit = true;
            sig.note("cursor-on-exit", fl + exit_flat.len());
        }
        if let Some((kind, end)) = clipped_choice_cursor(&f.flat, fl, row.row_end_flat, row.tail_end_flat) {
            sig.note(kind, end);
        }
    }

    // ⓒ — 번호 붙은 선택지 행(커서 유무 무관 · 접힌 라벨 포함). 라벨 검색은 평탄화 공간(단어 안 줄바꿈까지
    //   흡수 · 리뷰 R2), 앞 경계(`N.`)는 정규화 공간에서 본다(평탄화하면 앞 라벨 꼬리가 번호에 붙는다).
    for label in MODAL_CHOICE_LABELS {
        let lf: Vec<char> = first_run_gates::flatten(label).chars().collect();
        for fp in find_all_chars(&flat, &lf) {
            let p = flat_to_norm[fp];
            let label_end_flat = fp + lf.len();
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
                sig.note("choice-row", label_end_flat);
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

/// ★(0.14.31 · WP-5 · CONTRACTS B-1 "판정 분리 금지") 큐 배달 게이트(`cysd::governance`)가 **공유**하는
/// 전경 모달 술어 — 화면에 모달 어휘가 있고 그것이 **역사가 아닐 때**(마커 뒤에 있거나 · 마커의 마지막
/// 줄이 빈 대기 프롬프트가 아니거나 · 꼬리에 레이아웃 양성 증거가 없을 때) `Some`. 마커 미정의·부재는
/// **닫지 않는다**(fail-closed = 모달 어휘가 있으면 전경으로 본다). 재주입 창([`modal_window_closed`])과
/// 같은 판정기다 — 두 소비처가 각자 판정하면 벨트에 구멍이 난다.
pub fn modal_foreground(screen: &str, marker: Option<&str>) -> Option<ModalSignature> {
    let sig = modal_signature(screen)?;
    if modal_left_behind(&sig, screen, marker) {
        None
    } else {
        Some(sig)
    }
}

/// ★(0.14.31 · WP-5) 큐 배달 게이트가 공유하는 **번호 선택지 행** 술어([`is_numbered_item_row`]) — 커서행이
/// `N. …` 이면 그 행은 composer 가 아니라 선택기다(codex `› 1. Yes, continue` · claude `❯ 1. Yes`).
pub fn numbered_item_row(line: &str) -> bool {
    is_numbered_item_row(line)
}

/// 모달 거부의 생애 창 — [`gate_axis_window_closed`] 와 **같은 부호**(부트 상수 개방 · 재주입은
/// 전경 판정으로만 닫힘). `true` = 이 모달 문면은 역사다(창 닫힘 · 거부하지 않는다).
fn modal_window_closed(o: &Observed, sig: &ModalSignature) -> bool {
    match o.site {
        Site::Boot => false,
        Site::Reinject => modal_left_behind(sig, o.screen, marker_of(o)),
    }
}

/// 모달 문면이 화면의 **전경이 아님**을 잰다 — 축 ②(문면 전량이 마커보다 앞) ∧ 축 ①'(마커 줄이 빈 대기
/// 프롬프트 ∧ 꼬리가 입력 상자·상태줄 레이아웃 · [`waiting_prompt_with_harmless_trailer`]).
/// **fail-closed**: 마커 미정의·부재·빈 마커는 창을 닫지 않는다.
///
/// ★(리뷰 R1) 종전 축 ①은 [`gate_block_left_behind`] 와 같은 "마커 뒤 **화면 전체**가 공백" 이었다.
/// 라이브 claude 2.1.261 그리드는 입력 상자 아래에 괘선·상태줄을 그리므로 그 조건은 라이브 좌석에서
/// 결코 참이 되지 않았고, 본문에 모달 어휘가 한 번 보이면(감사표 `cat` · 관문 좌석 `read-screen` 출력)
/// 그 문면이 스크롤로 사라질 때까지 pack-update 재주입이 보류됐다(귀결은 보류라 안전했지만 영구 미주입
/// 방향의 결함). 줄 단위 축 ①' 는 선택 커서(`❯ 1. Yes…` — 같은 줄에 라벨)와 대기 프롬프트(`❯ ` 뒤 공백만)를
/// 정확히 가르고, 꼬리에는 **양성 레이아웃 증거**를 요구한다(codex 설계 검토: 빈 `❯` 줄 아래에 모달
/// 본문이 이어지는 형상 — 부분 렌더·접힌 라벨·빈 텍스트 입력 필드 — 은 애매하므로 닫지 않는다).
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
    // 축 ② — 모달 문면 전량이 그 마커보다 앞이다(역사). 마커 뒤에 모달 어휘가 있으면 전경이다.
    if sig.flat_end > marker_last {
        return false;
    }
    // 축 ①' — 마커의 마지막 출현 줄이 빈 대기 프롬프트이고, 그 아래 꼬리가 무해한 레이아웃이다.
    waiting_prompt_with_harmless_trailer(screen, m)
}

/// 한 줄이 **괘선**(입력 상자 테두리)인가 — 박스 문자(U+2500..=U+259F)가 [`PROMPT_TRAILER_RULE_MIN_RUN`]
/// 이상 **연달아** 있고, 그 밖의 비공백 문자가 없다(문장 속 `─` 한두 개는 괘선이 아니다).
fn is_rule_line(line: &str) -> bool {
    let (mut run, mut best) = (0usize, 0usize);
    for c in line.chars() {
        if ('\u{2500}'..='\u{259F}').contains(&c) {
            run += 1;
            best = best.max(run);
        } else if c.is_whitespace() {
            run = 0;
        } else {
            return false;
        }
    }
    best >= PROMPT_TRAILER_RULE_MIN_RUN
}

/// 번호 붙은 선택지 행(`  2. Yes…` · `2.` 문말)인가 — 선택기의 항목 행 그 자체(커서 유무 무관).
/// 규칙 ⓓ와 같은 번호 문법(1~2자리 · `.` · 뒤 공백/문말)이다.
fn is_numbered_item_row(line: &str) -> bool {
    let t = line.trim_start();
    // 앞머리 숫자 길이(바이트 = 문자 · ASCII). 이터레이터 계수 어휘를 쓰지 않는다 — H-PRED-8 은 판정부
    // 슬라이스에서 그 어휘 자체를 금지한다(마커 개수 비교 회귀 차단 핀 · 주석 포함 문자열 검사).
    let digits = t.find(|c: char| !c.is_ascii_digit()).unwrap_or(t.len());
    if digits == 0 || digits > 2 {
        return false;
    }
    let rest = &t[digits..];
    rest.starts_with('.') && rest[1..].chars().next().is_none_or(|c| c == ' ')
}

/// 마커의 마지막 출현 **줄**이 빈 대기 프롬프트(마커 뒤 그 줄에 공백만)이고, 그 아래 꼬리가 **입력
/// 상자·상태줄 레이아웃**인가(리뷰 R1 · 축 ①').
///
/// 꼬리(마커 줄 아래의 비공백 줄들)의 판정:
///   · 비어 있음 → 참(마커가 화면의 마지막 문면 — 종전 축 ①과 같은 화면).
///   · 모달 형상 금지 — 꼬리 어느 줄에도 선택 커서 `❯` 나 번호 항목 행이 없어야 한다(부분 렌더 · 접힌 라벨).
///   · 레이아웃 양성 증거 — 첫 줄이 괘선(입력 상자 아래 테두리 · 2.1.261) **또는** 어느 줄에
///     [`PROMPT_TRAILER_TOKENS`] 상태줄 어휘가 있다(2.1.241 `? for shortcuts` · 2.1.261 `⏵⏵ bypass
///     permissions on (shift+tab to cycle)`). 증거가 없으면 **닫지 않는다** — 빈 `❯` 줄 아래의 정체
///     모를 본문은 모달의 일부일 수 있다(빈 텍스트 입력 필드 · 그리는 중인 프레임).
/// 마커가 화면에 없거나 줄 단위로 찾을 수 없으면(마커가 줄을 넘어 접힘) 참을 주장하지 않는다.
fn waiting_prompt_with_harmless_trailer(screen: &str, marker: &str) -> bool {
    match scan_composer(screen, marker, None) {
        Some(sc) => sc.trailer_empty || sc.strong,
        None => false,
    }
}

/// ★(0.14.31 · 리뷰 R5 · codex blocking / 리뷰 R2(R7회차)) 같은 스캐너의 **엄격판(강한 증거)** — 마커 줄
/// **아래**에 입력 상자 괘선·상태줄이 실제로 있을 때만 참이다(꼬리가 비면 거짓).
///
/// 【왜 두 판이 필요한가 — 묻는 것이 다르다】
///   · 재주입 생애 창의 관대판([`waiting_prompt_with_harmless_trailer`])은 "모달이 **역사**인가" 를
///     묻는다. 거기서 꼬리가 비었다는 것은 **모달 본문이 더 없다**는 뜻이라 창을 닫아도 안전하다.
///   · "지금 이 `❯` 가 composer 인가, 아직 라벨이 안 그려진 선택기인가" 를 묻는 두 소비처는 **엄격판**을
///     쓴다: 부트의 **관문 증거 이월**(`cys.rs::gate_carry_ok`)과 **큐 배달의 alt-screen 자격**
///     (`cysd::governance::prompt_gate_input` 의 `layout_ok`). 꼬리가 비어 있는 `❯ ` 는 **정확히 두 경우가
///     구별되지 않는 프레임**이므로, 거기서 참을 주면 두 장치가 통째로 무의미해진다.
///   ★(0.14.31 · 리뷰 R1(R6회차) · claude 적대) 배달 게이트는 R5 에서 관대판을 쓰고 있었다 — vt100
///     `Grid::write_contents` 가 후행 개행을 잘라 내므로 마커 행이 마지막 비공백 행이면 꼬리는 **항상**
///     빈 벡터이고, 그러면 `layout_ok` 가 무조건 참이 되어 `prompt_gate_verdict` 문서가 약속한
///     "레이아웃 양성 증거" 가 사실상 없었다(BLOCKED_ALT_SCREEN 이 나지 않았다).
/// 스캐너는 하나이고 갈리는 것은 **증거 등급**뿐이다(판정 분리 금지).
///   ★(리뷰 R2(R7회차)) R6 이 여기 넣었던 "마커 **위** 상태줄" 예외는 이 함수에서 **빠졌다** — 그것은
///     [`composer_layout_static_ok`] 의 **약한 증거**이고 출력 정적과 AND 여야 한다(codex R7 D2 반례:
///     역사적 상태줄 · 라벨 미도색 선택기가 같은 문자열을 만든다). 인자 없는 구 이름
///     (`waiting_prompt_layout_positive`)은 **삭제했다** — 남겨 두면 그 이름을 부르는 소비처가
///     '약한 증거까지 포함' 으로 오해할 수 있고, 오용 경로를 지우는 것이 R6 이 관대판에 한 처분과 같다.
///
/// 【왜 필요한가 — 치명위험 ③】 R6 은 이 술어를 두 소비처(부트 관문 증거 이월 · 큐 alt-screen 배달 자격)에
/// 걸었는데, 그 증거 어휘(`PROMPT_TRAILER_TOKENS` · 괘선)는 **claude 문면**이다. codex-cli 0.153.4 의 유휴
/// composer 는 `› Ask Codex to do anything` + 상태줄 `gpt-6-astra medium · ~…` 라서 괘선도 상태줄 어휘도
/// 없고, 게다가 마커 줄 뒤에 **플레이스홀더 문면이 있다** — 그래서 두 축이 **영원히 거짓**이었다
/// (관문을 한 번 본 codex 좌석은 `carry-unproven` 영구 보류 = 디렉티브 미주입 · alt-screen 이면 큐 배달도
/// 영구 `BLOCKED_ALT_SCREEN`). 리뷰어 2인이 각각 같은 자리를 짚었다.
///
/// 【장치】 플레이스홀더는 **비어 있고 포커스된 composer 에만** 그려지는 문면이다(입력이 한 글자라도
/// 들어오면 사라진다 — agents.json codex 어댑터 주석의 실측 근거). 그래서 마커 줄의 나머지가 그
/// 플레이스홀더와 **평탄화 공간에서 완전히 같으면** ⓐ그 줄은 '빈 입력줄' 이고 ⓑ그 자체가 레이아웃
/// 양성 증거다. 잘려 그려진 플레이스홀더(부분 일치)는 **인정하지 않는다**(fail-closed — 부분 일치를 받으면
/// 짧은 접두가 선택지 라벨과 겹칠 수 있다).
/// 꼬리의 모달 형상 배제(`❯`·어댑터 마커·번호 항목 행)는 플레이스홀더가 있어도 그대로 적용한다.
pub fn composer_layout_positive(screen: &str, marker: &str, placeholder: Option<&str>) -> bool {
    matches!(scan_composer(screen, marker, placeholder), Some(sc) if sc.strong)
}

/// ★(0.14.31 · 리뷰 R2(R7회차) · codex blocking B2·D3) **약한 증거는 출력 정적과 AND 다.**
///
/// 【무엇이 남았나】 순수 화면 함수로는 끝내 가를 수 없는 두 프레임이 있다: 정상 유휴 composer
/// (`? for shortcuts⏎ …43% context left⏎❯ `)와, **같은 화면에서 선택지 라벨만 아직 안 그려진**
/// 관문. 문자열이 같으므로 어떤 구조 규칙도 이 둘을 가르지 못한다(codex R7 D2 반례 · 노트 잔여표).
/// 플레이스홀더도 같은 계급이다 — "빈 composer 에만 그려진다" 와 "지금 캡처된 그 셀이 현재 포커스를
/// 증명한다" 는 다른 명제이고, 잔여 셀·부분 재도색에서 후자는 성립하지 않는다(codex R7 D3 반례).
///
/// 【그래서 화면 밖의 축을 AND 한다】 재도색 중 프레임은 **정적일 수 없다** — 라벨이 사라진 창은
/// 청크 하나의 반영 시간이고, 그 사이 출력이 흐른다. 그래서 약한 증거(마커 위 상태줄 · 플레이스홀더)는
/// `idle_quiet`([`BOOT_VALVE_QUIET_SECS`] 이상 출력 없음)과 함께일 때만 참으로 센다. 강한 증거
/// (마커 **아래**의 입력 상자 괘선·상태줄)는 종전대로 단독이다 — 그 레이아웃은 위젯이 다 그려졌다는
/// 사실 자체다. 미관측(`None`)은 참으로 접지 않는다('부재 ≠ 부정').
pub fn composer_layout_static_ok(
    screen: &str,
    marker: &str,
    placeholder: Option<&str>,
    idle_quiet: Option<bool>,
) -> bool {
    match scan_composer(screen, marker, placeholder) {
        Some(sc) if sc.strong => true,
        Some(sc) if sc.weak => idle_quiet == Some(true),
        _ => false,
    }
}

/// 마커 줄 **바로 위**에 있는 상태줄이 이 composer 의 것인가 — 꼬리가 빈 분기의 양성 증거.
///
/// ★(0.14.31 · 리뷰 R2(R7회차) · codex blocking B2) R6 은 "마커 위 **어디든** 상태줄 어휘"였다. 그러면
/// 스크롤백에 남은 **역사적** 상태줄 한 줄이 이월 가드를 통째로 무력화한다 — codex 반례
/// `? for shortcuts⏎────────────────⏎❯ `(라벨이 아직 안 그려진 선택기)가 정확히 그 문면이다.
/// 지금은 **구조적 결속**을 요구한다: 마커 줄 바로 위의 **연속 비공백 블록** 안이어야 하고(빈 줄에서
/// 끊긴다 = 그 위는 다른 위젯·스크롤백), 그 사이에 **괘선이 없어야 하며**(괘선은 입력 상자의 테두리이고,
/// 상자 레이아웃의 상태줄은 상자 **아래**에 온다 — 위쪽 괘선 뒤의 상태줄은 이 composer 의 것이 아니다),
/// 마커로부터 [`PROMPT_STATUS_ABOVE_MAX_ROWS`] 행 안이어야 한다.
/// 실측 2.1.241 레이아웃(`? for shortcuts` / `…43% context left` / `❯ `)은 2행 위라 그대로 통과한다.
fn status_row_bound_to_composer(lines: &[&str], li: usize) -> bool {
    let mut looked = 0usize;
    for k in (0..li).rev() {
        let l = lines[k];
        if l.trim().is_empty() {
            return false; // 연속 블록의 끝 — 그 위는 이 composer 의 위젯이 아니다
        }
        if is_rule_line(l) {
            return false; // 입력 상자 테두리 — 상태줄은 상자 아래에 온다
        }
        let norm = first_run_gates::normalize(l).to_lowercase();
        if PROMPT_TRAILER_TOKENS.iter().any(|t| norm.contains(t)) {
            return true;
        }
        looked += 1;
        if looked >= PROMPT_STATUS_ABOVE_MAX_ROWS {
            return false;
        }
    }
    false
}

/// 한 화면의 composer 레이아웃 관측(판정은 소비처가 한다). `None` = 마커 부재 · 마커 줄에 문면 ·
/// 꼬리에 모달 형상(= 이 화면은 대기 프롬프트가 아니다).
struct ComposerScan {
    /// 마커 줄 아래에 비공백 줄이 하나도 없다(vt100 후행 개행 절단 포함).
    trailer_empty: bool,
    /// **강한 증거** — 마커 아래의 입력 상자 괘선·상태줄(위젯이 다 그려졌다는 사실 자체).
    strong: bool,
    /// **약한 증거** — 마커 위 상태줄(2.1.241) · 어댑터 플레이스홀더. 정적(idle_quiet)과 AND 여야 한다.
    weak: bool,
}

fn scan_composer(screen: &str, marker: &str, placeholder: Option<&str>) -> Option<ComposerScan> {
    let lines: Vec<&str> = screen.lines().collect();
    let li = lines.iter().rposition(|l| l.contains(marker))?;
    let line = lines[li];
    let mi = line.rfind(marker)?;
    let rest = &line[mi + marker.len()..];
    // ★(리뷰 R2(R7회차)) 어댑터 플레이스홀더는 '빈 입력줄' 이고 **약한** 레이아웃 증거다.
    let placeholder_ok = placeholder
        .map(|ph| {
            let a = first_run_gates::flatten(rest);
            let b = first_run_gates::flatten(ph);
            !b.is_empty() && a == b
        })
        .unwrap_or(false);
    if !placeholder_ok && !rest.trim().is_empty() {
        return None; // 같은 줄에 문면 — 선택 커서 행이거나 사람이 치던 초안이다(둘 다 닫지 않는다).
    }
    let has_status_token = |l: &str| -> bool {
        let norm = first_run_gates::normalize(l).to_lowercase();
        PROMPT_TRAILER_TOKENS.iter().any(|t| norm.contains(t))
    };
    let trailer: Vec<&str> = lines[li + 1..]
        .iter()
        .copied()
        .filter(|l| !l.trim().is_empty())
        .collect();
    if trailer.is_empty() {
        // ★(0.14.31 · 리뷰 R1(R6회차) · codex "영구 보류 반례") 엄격판이 **마커 아래**만 보면
        //   2.1.241 레이아웃(`? for shortcuts` 가 프롬프트 **위** · 실측 검체 `LIVE_TUI_AT_PROMPT`)의
        //   정상 composer 가 영구히 거짓이 된다 — 그 좌석은 관문을 한 번 본 뒤 **채택 자체가 불가능**
        //   해지고(부트 이월·재부트 채택 둘 다), 그것이 곧 치명위험 ③(디렉티브 미주입)이다.
        //   그래서 꼬리가 비었을 때는 **마커 위**의 상태줄 어휘를 증거로 받는다.
        // ★(0.14.31 · 리뷰 R2(R7회차) · codex blocking B2) 단 그 증거는 **약한 증거**다: 결속을
        //   구조로 좁혀도([`status_row_bound_to_composer`]) '정상 유휴 composer' 와 '같은 화면에서
        //   라벨만 아직 안 그려진 선택기' 는 **문자열이 같다**. 그래서 소비처가 출력 정적과 AND 한다
        //   ([`composer_layout_static_ok`]) — 재도색 중 프레임은 정적일 수 없다는 것이 화면 밖의
        //   유일한 판별 사실이다. 플레이스홀더도 같은 등급이다(잔여 셀이 포커스를 증명하지 않는다).
        return Some(ComposerScan {
            trailer_empty: true,
            strong: false,
            weak: placeholder_ok || status_row_bound_to_composer(&lines, li),
        });
    }
    // ★(0.14.31 · 리뷰 R1(R6회차) · claude 적대) 선택 커서 배제는 리터럴 `❯` 하나로 고정돼 있었다 —
    //   `prompt_marker` 가 `›`(codex)·`>`(gemini)인 어댑터에서는 꼬리의 선택기 행이 걸러지지 않았다.
    //   **어댑터 마커도 함께** 본다(조여지는 방향 · claude 는 두 값이 같아 거동 불변).
    if trailer
        .iter()
        .any(|l| l.contains('❯') || l.contains(marker) || is_numbered_item_row(l))
    {
        return None;
    }
    // ★(0.14.31 · 리뷰 R2 · codex blocking) **첫 비공백 줄**이 경계여야 강한 증거다. 종전에는
    //   꼬리 **어느 줄이든** 상태줄 어휘가 있으면 강한 증거였다 — 그러면 멀티라인 초안
    //   (`❯ `⏎`  둘째 행 초안`⏎괘선⏎`⏵⏵ bypass permissions on`)이 "빈 대기 composer" 로 읽힌다
    //   (커서를 Home 으로 옮긴 상태의 실측 형상 · codex 반례). 마커 줄 바로 아래가 입력 상자
    //   테두리이거나 상태줄이라는 것은 **편집 영역이 비었다**는 구조적 사실이고, 그 밖의 문면은
    //   초안 이어짐일 수 있으므로 증거로 접지 않는다(조여지는 방향 · 실측 2.1.263 은 괘선이라 불변).
    Some(ComposerScan {
        trailer_empty: false,
        strong: is_rule_line(trailer[0]) || has_status_token(trailer[0]),
        weak: placeholder_ok,
    })
}

/// ★(0.14.31 · 리뷰 R2 · codex blocking) **composer 편집 영역이 비어 있는가** — 커서 한 행이
/// 아니라 화면 구조로 묻는다.
///
/// 【무엇이 틀렸었나】 stale `pending_input_bytes` 리셋은 `PromptObs.line`(커서가 있는 **그 한 행**)
/// 만 보고 "빈 입력줄" 을 판정했다. 그래서 멀티라인 초안의 첫 행(`❯ `)에 커서를 올려 두면
/// (Home·Ctrl-A) 둘째 행의 **실초안**을 못 보고 계수를 0 으로 지웠고, 다음 틱이 큐 본문을 그
/// 초안과 한 줄로 합쳐 제출했다(fail-closed 였던 것이 fail-open 으로 뒤집힘 · §3-3 위반).
///
/// 【규칙】 [`scan_composer`] 를 그대로 쓴다(판정 분리 금지). 참인 경우는 셋뿐이다.
///   ⓐ 마커 줄 아래에 비공백 줄이 없다(`trailer_empty` — vt100 후행 개행 절단 포함)
///   ⓑ 마커 줄 **바로 아래**가 입력 상자 괘선·상태줄이다(`strong` — 편집 영역이 한 줄이라는 구조)
///   ⓒ 마커 줄의 나머지가 어댑터 플레이스홀더와 완전히 같다(`weak` 의 플레이스홀더 갈래 —
///      플레이스홀더는 **비어 있는** composer 에만 그려진다)
/// 마커 줄에 문면이 있거나(초안·선택 커서 행) 마커 아래 첫 비공백 줄이 정체 모를 문면이면 거짓이다.
/// 거짓의 귀결은 **리셋 안 함**(= 종전 0.14.30 의 보류)이고, 참을 잘못 주는 것의 귀결은 **초안 소거**
/// 라 비대칭이 분명하다 — 그래서 미관측·모호는 전부 거짓으로 접는다.
pub fn composer_edit_region_empty(screen: &str, marker: &str, placeholder: Option<&str>) -> bool {
    matches!(scan_composer(screen, marker, placeholder),
             Some(sc) if sc.trailer_empty || sc.strong || sc.weak)
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

    // ═══════════════════════════════════════════════════════════════════════
    // ★(0.14.31 · 리뷰 R5) codex(gpt-6-astra) 위임 산출 — **전행 검토 후 채택**.
    //   수정 1건: `❯ 2.` 의 기대 규칙을 ⓓ(`cursor-on-numbered-item`)로 정정(위 주석 참조).
    //   그 밖은 무수정. 문면 리터럴 사본 0(라벨은 `MODAL_CHOICE_LABELS` 에서 잘라 조립한다).
    // ═══════════════════════════════════════════════════════════════════════

    /// 잘린 선택기와 번호 없는 완전 라벨이 네 양성 증거 모두를 보류하는지 잰다.
    /// 새 규칙의 실제 검출과 거부된 증거 종류를 함께 고정해 다른 규칙에 기대는 공허한 통과를 막는다.
    #[test]
    fn r5_clipped_choices_hold_each_positive_evidence() {
        let gates = first_run_gates::builtin();
        let exit = MODAL_CHOICE_LABELS[2];
        let accept = MODAL_CHOICE_LABELS[1];
        if exit != MODAL_EXIT_LABEL || exit.chars().count() != 8 || accept.chars().count() != 13 {
            panic!("전제 붕괴: 종료/수락 라벨의 위치 또는 절단 길이가 달라졌다");
        }
        let cut: String = exit.chars().take(7).collect();
        let cases = [
            ("exit-cut", format!("❯ {cut}"), "clipped-choice-row", true),
            ("exit-comma", format!("❯ {}", exit.chars().take(3).collect::<String>()), "clipped-choice-row", true),
            ("exit-two-chars", format!("❯ {}", exit.chars().take(2).collect::<String>()), "clipped-choice-row", true),
            ("exit-one-char", format!("❯ {}", exit.chars().take(1).collect::<String>()), "clipped-choice-row", true),
            ("number-only", "❯ 2".to_owned(), "clipped-choice-row", true),
            // `❯ 2.` 는 **종전 규칙 ⓓ**(`cursor-on-numbered-item`)가 이미 잡는다 — 새 규칙이 아니라도
            // 보류 요구는 충족된다(엄격 파서가 번호로 읽어 라벨 시작이 화면 끝이 되기 때문). 검사 대상은
            // "이 화면이 보류되는가" 이므로 기대 규칙을 그 자리로 정정한다(리뷰: 전행 검토).
            ("number-dot-only", "❯ 2.".to_owned(), "cursor-on-numbered-item", true),
            ("number-no-space", format!("❯ 2.{cut}"), "clipped-choice-row", true),
            ("number-space", format!("❯ 2. {cut}"), "clipped-choice-row", false),
            ("try-complete", format!("❯ {}", MODAL_CHOICE_LABELS[3]), "cursor-on-choice-label", true),
            ("defer-complete", format!("❯ {}", MODAL_CHOICE_LABELS[4]), "cursor-on-choice-label", true),
            ("accept-cut", format!("❯ {}", accept.chars().take(11).collect::<String>()), "clipped-choice-row", true),
            ("exit-next-line", format!("❯\n  {cut}"), "clipped-choice-row", true),
        ];
        for (name, base, expected_kind, only_new_rule) in cases {
            // CRLF 판본은 단일 행에도 실제 줄끝을 추가해 LF 검체와 바이트가 다르게 한다.
            for (render, screen) in [("raw", base.clone()), ("crlf", crlf(&format!("{base}\n")))] {
                if first_run_gates::identify(&gates, &screen).is_some() {
                    panic!("전제 붕괴: {name}/{render}를 코퍼스가 식별하여 미등재 모달 축을 잴 수 없다");
                }
                let sig = modal_signature(&screen)
                    .unwrap_or_else(|| panic!("{name}/{render}: 선택기 서명 누락: {screen:?}"));
                assert!(sig.kinds.contains(&expected_kind), "{name}/{render}: 새 규칙 누락: {:?}", sig.kinds);
                if only_new_rule {
                    assert_eq!(sig.kinds, vec![expected_kind], "{name}/{render}: 단독 규칙 계측");
                }
                assert!(!sig.cursor_on_exit, "{name}/{render}: 종료 전문 없는 커서가 종료 축을 오염시켰다");

                let mut valve = boot_all_open(&screen, &gates);
                valve.delta = "";
                valve.marker = None;
                let mut delta = obs(&screen, &screen, &gates);
                delta.agent_alive = Some(false);
                let mut marker_screen = obs(&screen, "", &gates);
                marker_screen.agent_alive = Some(false);
                marker_screen.time_fallback_reached = true;
                let mut time = marker_screen.clone();
                time.marker = None;
                for (o, expected_evidence) in [
                    (valve, Evidence::Valve),
                    (delta, Evidence::MarkerDelta),
                    (marker_screen, Evidence::MarkerScreen),
                    (time, Evidence::TimeFallback),
                ] {
                    if positive_evidence(&o) != Some(expected_evidence) {
                        panic!("전제 붕괴: {name}/{render}: {expected_evidence:?} 증거가 열리지 않았다");
                    }
                    match judge(&o) {
                        Verdict::GateHeld { gate_id, vetoed, .. } => {
                            assert_eq!(gate_id, MODAL_UNKNOWN_ID, "{name}/{render}/{expected_evidence:?}");
                            assert_eq!(vetoed, Some(expected_evidence), "{name}/{render}: 거부된 증거");
                        }
                        other => panic!("{name}/{render}/{expected_evidence:?}: 미등재 모달 보류 대신 {other:?}"),
                    }
                }
            }
        }
    }

    /// 빈 입력창·정상 실측 화면·셸 이력은 선택기 거부로 가용성을 잃으면 안 된다.
    /// 특히 접힌 확인 에코와 숫자 이력은 물리 행이 아니라 화면 끝까지 읽는 경계를 잰다.
    #[test]
    fn r5_clipped_choices_preserve_healthy_screen_availability() {
        let trust = MODAL_CHOICE_LABELS[0];
        if trust.chars().count() != 24 {
            panic!("전제 붕괴: 신뢰 라벨 길이가 달라져 좁은 pane 절단 위치를 재검토해야 한다");
        }
        let head: String = trust.chars().take(21).collect();
        let rest: String = trust.chars().skip(21).collect();
        let echo = format!("{trust} ✔");
        let wrapped_echo = format!("❯ {head}\n{rest} ✔\nWelcome back\n❯ ");
        let live = [
            ("status-below", fixtures::LIVE_TUI_2_1_261_STATUS_BELOW_PROMPT),
            ("at-prompt", fixtures::LIVE_TUI_AT_PROMPT),
            ("nbsp-prompt", fixtures::LIVE_TUI_2_1_261_NBSP_PROMPT),
        ];
        for (name, screen) in live {
            if !screen.lines().any(|line| line.trim() == "❯") {
                panic!("전제 붕괴: {name} 실측 화면에 빈 입력 프롬프트가 없다");
            }
        }
        if !fixtures::LIVE_TUI_2_1_261_NBSP_PROMPT.contains("❯\u{a0}") {
            panic!("전제 붕괴: NBSP 실측 화면의 입력 공백이 달라졌다");
        }
        for (name, screen) in live.into_iter().chain([
            ("empty-composer", "❯ "),
            ("whitespace-to-end", "❯ \n \t\r\n"),
            ("confirmation-echo", echo.as_str()),
            ("wrapped-confirmation-echo", wrapped_echo.as_str()),
            ("numeric-shell-history", "❯ 2\ncommand completed\n❯ "),
            ("numeric-history-without-prompt", "❯ 2\ncommand completed"),
            ("decimal-history", "❯ 1.5 hours later"),
            ("shell-list", "❯ ls -al"),
            ("shell-launch", "❯ claude --dangerously-skip-permissions"),
            ("ready-shell", fixtures::READY_SHELL),
        ]) {
            for (render, candidate) in [("raw", screen.to_owned()), ("crlf", crlf(screen))] {
                assert_eq!(modal_signature(&candidate), None, "{name}/{render}: 정상 화면을 모달로 오탐: {candidate:?}");
            }
        }
    }

    /// 종료 라벨의 모든 진부분접두는 보류하되 종료 커서 비트를 켜지 않아야 한다.
    /// 전문과 접힌 전문은 같은 종료 사실이므로 기존 종료 축을 계속 켜야 한다.
    #[test]
    fn r5_clipped_choices_do_not_widen_cursor_on_exit() {
        let exit = MODAL_CHOICE_LABELS[2];
        if exit != MODAL_EXIT_LABEL || exit.chars().count() < 2 {
            panic!("전제 붕괴: 종료 라벨 상수의 대응 또는 길이가 달라졌다");
        }
        for n in 1..exit.chars().count() {
            let prefix: String = exit.chars().take(n).collect();
            let screen = format!("❯ {prefix}");
            let sig = modal_signature(&screen).unwrap_or_else(|| panic!("길이 {n}: 잘린 종료 서명 누락"));
            assert_eq!(sig.kinds, vec!["clipped-choice-row"], "길이 {n}");
            assert!(!sig.cursor_on_exit, "길이 {n}: 진부분접두가 종료 전문으로 승격됐다");
        }
        let n = exit.chars().count() - 1;
        let head: String = exit.chars().take(n).collect();
        let rest: String = exit.chars().skip(n).collect();
        for screen in [format!("❯ {exit}"), format!("❯ 2. {exit}"), format!("❯ {head}\n  {rest}")] {
            for candidate in [screen.clone(), crlf(&format!("{screen}\n"))] {
                let sig = modal_signature(&candidate).unwrap_or_else(|| panic!("종료 전문 서명 누락: {candidate:?}"));
                assert!(sig.cursor_on_exit, "종료 전문의 기존 경계가 사라졌다: {candidate:?}");
                assert!(sig.kinds.contains(&"cursor-on-exit"));
                assert!(sig.kinds.contains(&"cursor-on-choice-label"));
            }
        }
    }

    /// 순수 술어의 한두 자리 번호 경계와 반환 끝 좌표를 직접 잰다.
    /// 소비 스캐너가 번호를 먼저 벗겨 규칙이 사라지는 경우를 술어 자체의 실패와 구분한다.
    #[test]
    fn r5_clipped_choice_cursor_number_and_empty_tail_boundaries() {
        for tail in ["2", "12", "2.", "12."] {
            let flat: Vec<char> = format!("❯{tail}").chars().collect();
            let n = flat.len();
            assert_eq!(clipped_choice_cursor(&flat, 1, n, n), Some(("clipped-choice-row", n)), "{tail}");
        }
        for tail in ["", "123", "123.", "1.5hourslater", "2commandcompleted"] {
            let flat: Vec<char> = format!("❯{tail}").chars().collect();
            let n = flat.len();
            assert_eq!(clipped_choice_cursor(&flat, 1, n, n), None, "{tail}");
        }
        // ★(0.14.31 · 리뷰 R1(R6회차)) 꼬리 끝(블록 경계)이 술어의 **입력**이다 — 경계 뒤 문면은
        //   판정에 들어오지 않고(무관한 꼬리로 거부를 취소할 수 없다), 경계 앞이 비면 라벨이 없는
        //   것과 같다(빈 composer). 반환 끝 좌표도 그 경계다(생애 창 축 ②의 재료).
        // 좌표계는 **평탄화**다(공백 0) — 라벨도 그 공간에서 자른다.
        let exit_flat = first_run_gates::flatten(MODAL_EXIT_LABEL);
        let exit_cut: String = exit_flat.chars().take(exit_flat.chars().count() - 1).collect();
        let flat: Vec<char> = format!("❯{exit_cut}────────").chars().collect();
        let cut_end = 1 + exit_cut.chars().count();
        assert_eq!(
            clipped_choice_cursor(&flat, 1, cut_end, cut_end),
            Some(("clipped-choice-row", cut_end)),
            "블록 경계 뒤의 괘선이 잘린 선택기 거부를 취소했다"
        );
        // ★(0.14.31 · 리뷰 R2(R7회차) · codex blocking B1) **재핀(조여지는 방향).** R6 은 여기서 `None` 을
        //   못 박아 "블록이 경계 없이 이어지면 거부가 사라진다" 는 잔여를 검체로 승인하고 있었다. 지금은
        //   부분 라벨을 **행**에서 재므로, 블록이 화면 끝까지 이어져도(무관한 꼬리) 거부가 유지된다.
        assert_eq!(
            clipped_choice_cursor(&flat, 1, cut_end, flat.len()),
            Some(("clipped-choice-row", flat.len())),
            "무관한 꼬리가 이어졌다는 이유로 잘린 선택기 거부가 취소됐다(B1 재발)"
        );
        // 완결 증거 면제 — 행은 잘려 보여도 **같은 블록 안에서** 라벨이 완결됐으면 접힌 에코다.
        let trust = first_run_gates::flatten(MODAL_CHOICE_LABELS[0]);
        let full: Vec<char> = format!("❯{trust}✔Welcomeback").chars().collect();
        let head = 1 + trust.chars().count() - 3;
        assert_eq!(
            clipped_choice_cursor(&full, 1, head, full.len()),
            None,
            "접힌 확인 에코가 관문으로 오탐됐다"
        );
        // ★(리뷰 R2(R7회차) · claude major) 완결 증거는 **블록 안에서만** 센다 — 블록이 라벨 도중에
        //   끊겼는데 경계 너머의 글자가 라벨을 완성하는 경우는 면제가 아니다(거부 유지).
        assert_eq!(
            clipped_choice_cursor(&full, 1, head, head),
            Some(("clipped-choice-row", head)),
            "블록 경계 너머의 글자가 완결 증거로 세어져 거부가 취소됐다"
        );
    }

    /// ★(0.14.31 · 리뷰 R7) codex(gpt-6-astra) 위임 산출 — **전행 검토 후 무수정 채택**.
    ///   순수 술어(`clipped_choice_cursor`)와 소비 스캐너(`modal_signature`)를 **함께** 재고,
    ///   라벨 리터럴을 하나도 적지 않는다(문면 SOT = 코퍼스 하나). 계약 셋을 라벨·접기 지점
    ///   **전수**로 돈다: ⓐ완결된 확인 에코는 관문이 아니다(서로 다른 라벨의 **공통 접두**가
    ///   실제로 실행됐는지 `shared_prefix_exercised` 가 자기검사한다) ⓑ무관한 꼬리는 완결 증거가
    ///   아니다 ⓒ블록 경계 너머의 글자도 완결 증거가 아니다. raw·CRLF 두 판본.
    #[test]
    fn r7_clipped_contract_echo_and_partial_rows() {
        let mut shared_prefix_exercised = false;
        for label in MODAL_CHOICE_LABELS {
            for (cut, _) in label.char_indices().skip(1) {
                let (head, rest) = label.split_at(cut);
                let hf = first_run_gates::flatten(head);
                if hf.is_empty() || hf == first_run_gates::flatten(label) {
                    continue;
                }
                let row = format!("❯ {head}");
                let row_end = first_run_gates::flatten(&row).chars().count();
                if label != MODAL_EXIT_LABEL {
                    shared_prefix_exercised |= MODAL_CHOICE_LABELS.iter().any(|other| {
                        let of = first_run_gates::flatten(other);
                        *other != label && of.starts_with(&hf) && hf.chars().count() < of.chars().count()
                    });
                    let echo = format!("{row}\n{rest} ✔\nWelcome back");
                    for (render, screen) in [("raw", echo.clone()), ("CRLF", crlf(&echo))] {
                        let flat: Vec<char> = first_run_gates::flatten(&screen).chars().collect();
                        assert_eq!(clipped_choice_cursor(&flat, 1, row_end, flat.len()), None,
                            "{render}/{label}/{cut}: 라벨 집합 전체의 완결 증거를 놓쳐 공통 접두를 포함한 확인 에코를 잘림으로 오인하면 실패한다.");
                        assert_eq!(modal_signature(&screen), None,
                            "{render}/{label}/{cut}: 소비 스캐너가 접힌 완결 확인 에코를 관문으로 오인하면 실패한다.");
                    }
                }
                for (case, screen, bounded) in [
                    ("무관한 꼬리", format!("{row}\n무관한 출력"), false),
                    ("빈 줄 너머 완성", format!("{row}\n\n{rest} ✔\nWelcome back"), true),
                ] {
                    for (render, screen) in [("raw", screen.clone()), ("CRLF", crlf(&screen))] {
                        let flat: Vec<char> = first_run_gates::flatten(&screen).chars().collect();
                        let end = if bounded { row_end } else { flat.len() };
                        assert_eq!(clipped_choice_cursor(&flat, 1, row_end, end), Some(("clipped-choice-row", end)),
                            "{render}/{label}/{cut}/{case}: 무관한 꼬리나 블록 밖 글자를 완결 증거로 세거나 블록 끝 좌표를 틀리면 실패한다.");
                        assert!(modal_signature(&screen).as_ref().is_some_and(|s| s.kinds.contains(&"clipped-choice-row")),
                            "{render}/{label}/{cut}/{case}: 소비 스캐너가 물리 행의 진부분접두 서명을 잃으면 종료 규칙의 유무와 무관하게 실패한다.");
                    }
                }
            }
        }
        assert!(shared_prefix_exercised,
            "서로 다른 라벨의 공통 진부분접두를 사용하는 확인 에코가 한 번도 실행되지 않으면 실패한다.");
    }

    #[test]
    fn r7_clipped_contract_boundaries_and_exact_tails() {
        for label in MODAL_CHOICE_LABELS {
            let cut = label.char_indices().last().expect("라벨은 비어 있지 않아야 한다").0;
            let (head, rest) = label.split_at(cut);
            let row = format!("❯ {head}");
            let row_end = first_run_gates::flatten(&row).chars().count();
            for boundary in ["", "        ", "────────", "  2. 다른 항목", "  2.",
                "다른 ❯ 출력", "for shortcuts", "bypass permissions", "shift+tab"] {
                let screen = format!("{row}\n{boundary}\n{rest} ✔\nWelcome back");
                for (render, screen) in [("raw", screen.clone()), ("CRLF", crlf(&screen))] {
                    let sig = modal_signature(&screen);
                    assert!(sig.as_ref().is_some_and(|s| s.kinds.contains(&"clipped-choice-row")),
                        "{render}/{label}/{boundary:?}: 빈 줄·괘선·번호 항목·다른 커서·상태줄 경계를 놓쳐 잘린 선택 행의 서명이 사라지면 실패한다.");
                    // 다른 서명은 끝 좌표를 늘릴 수 있으므로 빈 경계에서만 소비부 좌표를 잰다.
                    if boundary.is_empty() && label != MODAL_EXIT_LABEL {
                        assert_eq!(sig.as_ref().map(|s| s.flat_end), Some(row_end),
                            "{render}/{label}: 소비 스캐너가 빈 줄 너머를 선택 행 블록 끝으로 반환하면 실패한다.");
                    }
                }
            }
            let screen = format!("❯ {label}\n");
            for (render, screen) in [("raw", screen.clone()), ("CRLF", crlf(&screen))] {
                let flat: Vec<char> = first_run_gates::flatten(&screen).chars().collect();
                assert_eq!(clipped_choice_cursor(&flat, 1, usize::MAX, usize::MAX), Some(("cursor-on-choice-label", flat.len())),
                    "{render}/{label}: 정확한 라벨 꼬리의 종류·끝 좌표 또는 범위 상한 보정이 깨지면 실패한다.");
            }
        }
        for tail in ["2", "2."] {
            let screen = format!("❯ {tail}\n");
            for (render, screen) in [("raw", screen.clone()), ("CRLF", crlf(&screen))] {
                let flat: Vec<char> = first_run_gates::flatten(&screen).chars().collect();
                assert_eq!(clipped_choice_cursor(&flat, 1, flat.len(), flat.len()), Some(("clipped-choice-row", flat.len())),
                    "{render}/{tail}: 숫자만 또는 번호만 남은 꼬리를 잘린 선택 행으로 반환하지 않으면 실패한다.");
                assert!(modal_signature(&screen).is_some(),
                    "{render}/{tail}: 소비 스캐너가 숫자만 또는 번호만 남은 선택 행의 관문 서명을 잃으면 실패한다.");
            }
        }
    }

    /// ★(0.14.31 · 리뷰 R6) codex(gpt-6-astra) 위임 산출 — **전행 검토 후 채택**.
    ///   꼬리 경계 술어와 끝 좌표를 **직접** 잰다(소비 스캐너를 통하지 않으므로, 규칙이 사라진 것과
    ///   소비부에서 가려진 것을 구분한다). CRLF 판본은 같은 flat 끝을 요구한다.
    /// ★(0.14.31 · 리뷰 R2(R7회차) · claude minor) **생애 창 축 ②의 받아들인 잔여를 기계로 못 박는다.**
    ///
    /// [`ModalSignature::flat_end`] 는 R6 부터 **선택 행 블록의 끝**이다. 그래서 잘린 선택기가 마커의
    /// 마지막 출현보다 **앞**에 있으면 `Site::Reinject`·큐 배달에서 '역사' 로 분류된다. 그 판정은 두
    /// 화면을 **같은 관측**으로 본다:
    ///   ⓐ 무해(스크롤백 잔상 + 아래에 라이브 프롬프트) — 닫아야 한다. 안 닫으면 `❯ Yes` 한 줄에
    ///      큐 배달이 **영구 기아**다(치명위험 ①의 반대편).
    ///   ⓑ 위험(재도색 중 잘린 모달 + 아래에 살아 있는 composer) — 닫으면 안 된다.
    /// 화면 문자열로는 둘이 구별되지 않으므로 **ⓐ를 택했다**(노트 §16-2·§16-3). 이 검체는 그 선택을
    /// 양방향으로 고정한다 — ⓑ가 언젠가 실측되면 여기가 붉어져야 하고, ⓐ를 잃으면 기아가 재발한다.
    /// **`Site::Boot` 는 두 화면 모두 보류다**(창 상수 개방) — 부트 주입은 이 잔여의 영향을 받지 않는다.
    #[test]
    fn r7_lifetime_window_axis_two_residual_is_pinned_in_both_directions() {
        let gates = first_run_gates::builtin();
        let exit_flat = first_run_gates::flatten(MODAL_EXIT_LABEL);
        let cut: String = exit_flat.chars().take(exit_flat.chars().count() - 1).collect();
        let live = fixtures::LIVE_TUI_2_1_261_STATUS_BELOW_PROMPT;
        let screen = format!("❯ {cut}\n────────────────\n{live}");
        // 전제: 잘린 선택기 서명은 **선다**(그것이 이 축의 입력이다).
        let sig = modal_signature(&screen).expect("잘린 선택기 서명이 서지 않는다(전제 붕괴)");
        assert!(sig.kinds.contains(&"clipped-choice-row"));
        // ⓐ·ⓑ 공통 — 재주입·배달 소비처는 '역사' 로 본다(받아들인 잔여 · 기아 회피 방향).
        assert!(
            modal_foreground(&screen, Some("❯")).is_none(),
            "잔여가 바뀌었다: 블록 끝 서명이 라이브 프롬프트 아래에서 전경으로 읽힌다(큐 배달 영구 기아)"
        );
        // 부트는 영향 없음 — 같은 화면이 보류다(주입 0 · 키 0).
        assert!(held_as(&judge(&boot_all_open(&screen, &gates)), MODAL_UNKNOWN_ID));
        // 반대 방향 — 잘린 선택기가 마커 **뒤**(아래)면 전경이다(축 ②가 살아 있다).
        let foreground = format!("{live}❯ {cut}\n");
        assert!(
            modal_foreground(&foreground, Some("❯")).is_some(),
            "마커 뒤의 잘린 선택기까지 역사로 접힌다 — 축 ②가 죽었다"
        );
    }

    /// ★(0.14.31 · 리뷰 R2(R7회차) · codex 설계 검토 D1 반례) **다른 라벨의 공통 접두**가 완결된
    /// 확인 에코를 다시 관문으로 오탐하지 않는다.
    ///
    /// `❯ Yes, I⏎accept ✔⏎Welcome back` 의 첫 행 `Yes,I` 는 `Yes, I accept` 의 접두이자
    /// **`Yes, I trust this folder` 의 진부분접두**이기도 하다. 완결 증거를 '같은 라벨' 로만 세면
    /// 뒤 라벨에서 잘림 판정이 서고, 그 화면은 2026-07-29 킬체인의 **역방향 회귀**(확인 에코를
    /// 관문으로 읽어 다음 Return 이 종료를 누른다)의 입구다. 완결은 **라벨 집합 전체**에 대해 센다.
    #[test]
    fn r7_completed_echo_of_another_label_is_not_a_clipped_choice() {
        let gates = first_run_gates::builtin();
        // ★종료 라벨(`No, exit`)은 **제외**한다 — 그 라벨의 에코는 규칙 ⓐ(`cursor-on-exit`)가
        //   의도적으로 접두로 잡는다(그 화면에 Return 을 보내면 좌석이 죽으므로 에코라도 보류가
        //   옳다 · R6 검체 ⓒ 와 같은 규약). 여기서 재는 것은 **잘림 규칙**의 완결 면제다.
        for label in MODAL_CHOICE_LABELS.iter().filter(|l| **l != MODAL_EXIT_LABEL) {
            // 접힘 지점을 라벨 안에서 옮겨 가며(2..len-1) 전수로 돈다 — 특정 접두 하나에만 맞춘
            // 수리를 배제한다(계측 타당성).
            let n = label.chars().count();
            for cut in 2..n {
                let head: String = label.chars().take(cut).collect();
                let rest: String = label.chars().skip(cut).collect();
                let screen = format!("❯ {head}\n{rest} ✔\nWelcome back\n");
                for (render, screen) in [("raw", screen.clone()), ("crlf", crlf(&screen))] {
                    assert_eq!(
                        modal_signature(&screen),
                        None,
                        "{render}/{label}/{cut}: 완결된 확인 에코가 관문으로 오탐됐다(역방향 회귀)"
                    );
                    let v = judge(&boot_all_open(&screen, &gates));
                    assert!(
                        !held_as(&v, MODAL_UNKNOWN_ID),
                        "{render}/{label}/{cut}: 판정이 미등재 모달로 접혔다: {v:?}"
                    );
                }
            }
        }
    }

    /// ★(0.14.31 · 리뷰 R2(R7회차) · codex blocking B2 + 리뷰어 2인 blocking) composer 레이아웃
    /// 증거의 **등급**과 어댑터 해소.
    ///
    /// ⓐ 강한 증거(마커 **아래**의 괘선·상태줄)는 단독으로 참이다.
    /// ⓑ 약한 증거(마커 **위** 상태줄 · 어댑터 플레이스홀더)는 **출력 정적**과 AND 여야 한다 —
    ///    그 프레임들은 '라벨이 아직 안 그려진 선택기' 와 문자열이 같아 화면만으로 갈리지 않는다.
    /// ⓒ 역사적 상태줄(빈 줄·괘선으로 끊긴 위쪽 · 4행 초과)은 이 composer 의 것이 아니다.
    /// ⓓ 마커가 `ready_marker`(`? for shortcuts`)면 codex·gemini 유휴 화면이 영원히 거짓이다.
    #[test]
    fn r7_composer_layout_evidence_is_graded_and_adapter_resolved() {
        let live261 = fixtures::LIVE_TUI_2_1_261_STATUS_BELOW_PROMPT;
        let live241 = fixtures::LIVE_TUI_AT_PROMPT;
        // ⓐ 강한 증거 — 정적 미관측(None)에서도 참.
        assert!(composer_layout_positive(live261, "❯", None));
        assert!(composer_layout_static_ok(live261, "❯", None, None));
        // ⓑ 2.1.241(상태줄이 프롬프트 위) = 약한 증거 — 정적일 때만 참.
        assert!(!composer_layout_positive(live241, "❯", None), "약한 증거가 강한 증거로 세어졌다");
        assert!(composer_layout_static_ok(live241, "❯", None, Some(true)));
        for q in [None, Some(false)] {
            assert!(!composer_layout_static_ok(live241, "❯", None, q), "재도색 중에도 열렸다: {q:?}");
        }
        // ⓒ 역사적 상태줄 — 괘선·빈 줄·거리로 끊긴다(codex R7 B2 반례 + 변주).
        for (name, screen) in [
            ("괘선이 끼어 있다", "? for shortcuts\n────────────────────────\n❯ \n"),
            ("빈 줄로 끊긴다", "? for shortcuts\n\n❯ \n"),
            ("거리 초과", "? for shortcuts\na\nb\nc\nd\ne\n❯ \n"),
        ] {
            assert!(
                !composer_layout_static_ok(screen, "❯", None, Some(true)),
                "{name}: 역사적 상태줄이 이 composer 의 증거로 세어졌다"
            );
        }
        // ⓓ 어댑터 — codex 유휴 composer 는 `prompt_marker`(›) + 플레이스홀더로만 열린다.
        let codex_idle = "• DIRECTIVE-ACK-11137\n\n────────────────────────\n\n\n› Ask Codex to do anything\n\n  gpt-6-astra medium · ~/dev/cys-t1/src\n";
        let ph = Some("Ask Codex to do anything");
        assert!(composer_layout_static_ok(codex_idle, "›", ph, Some(true)), "codex 유휴가 영구 거짓이다");
        assert!(!composer_layout_static_ok(codex_idle, "›", ph, Some(false)), "플레이스홀더가 정적 없이 열렸다");
        assert!(!composer_layout_static_ok(codex_idle, "›", None, Some(true)), "플레이스홀더 없이 열렸다");
        assert!(
            !composer_layout_static_ok(codex_idle, "? for shortcuts", None, Some(true)),
            "전제 붕괴: ready_marker 로도 참이면 이 검체는 마커 해소를 재지 못한다"
        );
        // 플레이스홀더는 **완전 일치**만 인정한다(잘린 렌더는 fail-closed).
        assert!(
            !composer_layout_static_ok(codex_idle, "›", Some("Ask Codex to do anything now"), Some(true)),
            "플레이스홀더 부분 일치가 열렸다"
        );
        // gemini 유휴(괘선이 아래) = 강한 증거 — `>` 로 열리고 `? for shortcuts` 로는 아니다.
        let gemini_idle = "  각성 확인 완료.\n────────────────────────\n>\n────────────────────────\n? for shortcuts                     Gemini 3.8 Flash · hig\n";
        assert!(composer_layout_positive(gemini_idle, ">", None));
        assert!(
            !composer_layout_positive(gemini_idle, "? for shortcuts", None),
            "전제 붕괴: ready_marker 로도 참이면 이 검체는 마커 해소를 재지 못한다"
        );
        // 꼬리에 선택기 형상이 있으면 어떤 증거로도 열리지 않는다(모달 형상 배제는 그대로).
        let with_selector = format!("{codex_idle}› 1. Yes, continue\n");
        assert!(!composer_layout_static_ok(&with_selector, "›", ph, Some(true)));
    }

    /// ★(0.14.31 · 리뷰 R2 · codex blocking) **강한 증거는 마커 바로 아래 한 줄이다.**
    ///
    /// 종전 `strong` 은 꼬리 **어느 줄이든** 상태줄 어휘가 있으면 참이었다 — 그래서 멀티라인 초안
    /// (`❯ ` / `  둘째 행 초안` / 괘선 / 상태줄)이 "빈 대기 composer" 로 읽혔고, 그 위에서 stale
    /// 리셋이 계수를 0 으로 만들면 다음 배달이 사람 초안과 한 줄로 합쳐진다.
    /// 실측 문면(claude 2.1.263 은 마커 아래가 괘선)은 그대로 통과해야 한다 — 가용성 대조군.
    #[test]
    fn r2_strong_evidence_is_the_first_nonblank_row_below_the_marker() {
        let rule = "─".repeat(PROMPT_TRAILER_RULE_MIN_RUN);
        // ⓐ 실측 유휴(마커 아래 괘선 → 상태줄) — 강한 증거 · 편집 영역 비어 있음.
        let idle = format!("  이전 출력\n{rule}\n❯ \n{rule}\n  ⏵⏵ bypass permissions on\n");
        assert!(composer_layout_positive(&idle, "❯", None), "실측 유휴 그리드가 막혔다(기아)");
        assert!(composer_edit_region_empty(&idle, "❯", None));
        // ⓑ 멀티라인 초안 — 첫 비공백 줄이 초안이므로 증거가 아니고 편집 영역도 비지 않았다.
        let drafted =
            format!("  이전 출력\n{rule}\n❯ \n  둘째 행에 남은 실초안\n{rule}\n  ⏵⏵ bypass permissions on\n");
        assert!(
            !composer_layout_positive(&drafted, "❯", None),
            "초안 아래 상태줄만으로 '빈 composer' 가 됐다"
        );
        assert!(
            !composer_edit_region_empty(&drafted, "❯", None),
            "멀티라인 초안이 '빈 편집 영역' 으로 읽힌다 — stale 리셋이 그것을 지운다"
        );
        // ⓒ 마커 줄에 문면이 있으면(커서 뒤 초안·선택 커서 행) 어느 축도 열리지 않는다.
        let typed = format!("{rule}\n❯ 진행해줘\n{rule}\n");
        assert!(!composer_edit_region_empty(&typed, "❯", None));
        // ⓓ 꼬리가 아예 없으면(vt100 후행 절단) 편집 영역은 비어 있다 — 종전 축 그대로.
        assert!(composer_edit_region_empty("  출력\n❯ ", "❯", None));
        // ⓔ 플레이스홀더는 '비어 있는 composer' 의 증거다(codex 실측 문면).
        let codex_idle = "  출력\n› Ask Codex to do anything\n\n  gpt-6-astra medium · ~\n";
        assert!(composer_edit_region_empty(codex_idle, "›", Some("Ask Codex to do anything")));
        assert!(
            !composer_edit_region_empty(codex_idle, "›", None),
            "플레이스홀더 선언 없이 열리면 이 축은 어떤 문면이든 통과시킨다"
        );
    }

    #[test]
    fn r6_choice_tail_boundary_and_end_are_exact() {
        let trust = MODAL_CHOICE_LABELS[0];
        let split = trust.char_indices().rev().nth(2).expect("접을 라벨은 세 문자 이상이어야 한다").0;
        let (head, rest) = trust.split_at(split);
        let folded_tail = format!("{rest} ✔");
        let partial_label = format!("  {}", trust.rsplit_once(' ').expect("라벨에 단어 경계가 필요하다").0);
        let exit = MODAL_CHOICE_LABELS[2];
        let cut_at = exit.char_indices().last().expect("종료 라벨은 비어 있지 않아야 한다").0;
        let clipped = &exit[..cut_at];
        let rule = "─".repeat(PROMPT_TRAILER_RULE_MIN_RUN);
        let mut boundaries = vec![
            ("빈 줄".to_string(), String::new()),
            ("공백만 있는 줄".to_string(), " \t\r\u{2003}".to_string()),
            ("괘선".to_string(), format!("  {rule}  ")),
            ("번호 항목".to_string(), "  2. 다른 항목".to_string()),
            ("번호만 있는 항목".to_string(), "12.".to_string()),
            ("다른 선택 커서".to_string(), "  다음 ❯ 항목".to_string()),
        ];
        for token in PROMPT_TRAILER_TOKENS {
            boundaries.push((format!("상태줄 {token}"), format!("  ? {}  ", token.to_uppercase())));
            boundaries.push((
                format!("정규화 상태줄 {token}"),
                format!("  ? {}  ", token.to_uppercase().replace(' ', "\t  ")),
            ));
        }
        for (name, line) in &boundaries {
            assert!(is_choice_tail_boundary(line), "{name}: 경계 줄을 놓쳤다: {line:?}");
        }
        for line in [folded_tail.as_str(), partial_label.as_str(), "Welcome back", "  일반 출력 한 줄", "문장 속 ─ 기호"] {
            assert!(!is_choice_tail_boundary(line), "평문 또는 접힌 라벨 조각을 경계로 오인했다: {line:?}");
        }

        // find는 바이트 위치이므로 반드시 문자 수로 변환한다. 한글 머리말과 ❯가 혼동을 드러낸다.
        let check = |name: &str, screen: &str, end_byte: usize| {
            let expected_prefix = &screen[..end_byte];
            let expected_flat = expected_prefix.chars().filter(|c| !c.is_whitespace()).count();
            let mut lf_flat = None;
            for (render, text) in [("LF", screen.to_string()), ("CRLF", screen.replace('\n', "\r\n"))] {
                let raw: Vec<char> = text.chars().collect();
                let cursor = text.find('❯').map(|byte| text[..byte].chars().count()).unwrap_or(raw.len());
                let mut rendered_prefix = if render == "CRLF" {
                    expected_prefix.replace('\n', "\r\n")
                } else {
                    expected_prefix.to_string()
                };
                // 경계 직전 LF는 제외하지만 CRLF의 CR은 커서 블록 마지막 줄에 남는다.
                if render == "CRLF" && screen[end_byte..].starts_with('\n') {
                    rendered_prefix.push('\r');
                }
                let expected_end = rendered_prefix.chars().count();
                let actual_end = choice_tail_end(&raw, cursor);
                assert_eq!(actual_end, expected_end, "{name}/{render}: 원문 끝 인덱스가 틀렸다: {text:?}");
                let actual_flat = raw[..actual_end].iter().filter(|c| !c.is_whitespace()).count();
                assert_eq!(actual_flat, expected_flat, "{name}/{render}: 끝까지의 비공백 문자 수가 틀렸다");
                if let Some(lf) = lf_flat {
                    assert_eq!(actual_flat, lf, "{name}: LF와 CRLF의 flat 끝 위치가 다르다");
                } else {
                    lf_flat = Some(actual_flat);
                }
            }
        };

        for (name, boundary) in &boundaries {
            // 커서 행은 무조건 포함하고 바로 다음 경계는 포함하지 않는다.
            let row = format!("앞선 기록 한 줄\n  ❯ {clipped}");
            let screen = format!("{row}\n{boundary}\n무관한 후속 출력");
            let stop = screen[row.len()..].find('\n').expect("커서 행 다음 구분자가 필요하다") + row.len();
            check(&format!("커서 행에서 종료/{name}"), &screen, stop);

            // 접힌 완성 라벨과 체크 표시를 포함한 뒤에만 경계에서 멎어야 한다.
            let block = format!("앞선 기록 한 줄\n  ❯ {head}\n{folded_tail}");
            let screen = format!("{block}\n{boundary}\n무관한 후속 출력");
            check(&format!("접힌 확인 에코/{name}"), &screen, block.len());
        }

        let screen = format!("기록\n❯ {clipped}\nWelcome back\n  일반 출력 한 줄\n{rule}\n후속 출력");
        let stop = screen.find(&format!("\n{rule}")).expect("평문 뒤 괘선이 필요하다");
        check("여러 평문 줄을 지나 경계 앞 종료", &screen, stop);
        let screen = format!("기록\n❯ {head}\n{folded_tail}");
        check("접힌 라벨이 개행 없이 화면 끝에 도달", &screen, screen.len());
        let screen = format!("기록\n❯ {clipped}");
        check("커서 행이 개행 없이 종료", &screen, screen.len());
        let screen = format!("❯ {clipped}\n");
        check("마지막 개행 다음 빈 줄", &screen, screen.find('\n').expect("마지막 개행이 필요하다"));
        let screen = format!("❯ {rule}\nWelcome back");
        check("커서 행 자체의 경계 형상은 중단하지 않음", &screen, screen.len());
        let screen = "한글 기록\n❯";
        check("커서가 화면 마지막 문자", screen, screen.len());
        check("커서만 있는 화면", "❯", "❯".len());
        check("빈 화면의 cursor=0", "", "".len());

        let raw: Vec<char> = "한글 기록".chars().collect();
        assert_eq!(choice_tail_end(&raw, raw.len()), raw.len(), "cursor가 원문 끝이면 원문 길이를 반환해야 한다");
    }

    /// ★(0.14.31 · 리뷰 R1(R6회차) · codex blocking) **꼬리에 이어지는 무관한 줄이 잘린 선택기 거부를
    /// 취소하지 못한다.** R5 는 판정 재료를 "커서 뒤 화면 끝까지" 로 잡아 접힌 확인 에코 오탐은 막았지만,
    /// 그 대가로 아래에 괘선 한 줄만 그려져도(`❯ No, exi` + `────`) 서명이 통째로 사라졌다 — 그 프레임에서
    /// 준비 판정은 Ready{MarkerDelta} 였고 본문 + Return 이 **부분 렌더된 종료 선택지**로 나갔다.
    /// 지금은 선택 행 블록까지만 보고([`choice_tail_end`]), 완결된 라벨만 면제한다.
    #[test]
    fn r6_trailing_rows_do_not_cancel_the_clipped_choice_veto() {
        let gates = first_run_gates::builtin();
        let exit_cut: String = MODAL_EXIT_LABEL.chars().take(7).collect();
        // ⓐ codex 가 든 첫 프레임과 그 변주 — 경계 넷(괘선·빈 줄·번호 항목 행·다른 커서 행) 전부.
        let below = [
            ("괘선", "────────────────".to_string()),
            ("빈 줄 + 본문", format!("\n  이어지는 출력 한 줄")),
            ("번호 항목 행", "  2. 다른 항목".to_string()),
            ("다른 커서 행", "❯ ".to_string()),
            ("상태줄", " ⏵⏵ bypass permissions on (shift+tab to cycle)".to_string()),
        ];
        for (name, tail) in &below {
            let base = format!("❯ {exit_cut}\n{tail}\n");
            for (render, screen) in [("raw", base.clone()), ("crlf", crlf(&base))] {
                if first_run_gates::identify(&gates, &screen).is_some() {
                    panic!("{name}/{render}: 전제 붕괴 — 코퍼스가 식별하면 미등재 모달 축을 재지 못한다");
                }
                let sig = modal_signature(&screen)
                    .unwrap_or_else(|| panic!("{name}/{render}: 꼬리 한 줄에 거부가 취소됐다: {screen:?}"));
                assert!(
                    sig.kinds.contains(&"clipped-choice-row"),
                    "{name}/{render}: 잘린 선택기 규칙이 아니라 다른 규칙이 잡았다(계측 무효): {:?}",
                    sig.kinds
                );
                // 네 양성 증거 전부에서 보류다(증거 종류와 무관한 공통 거부).
                let v = judge(&boot_all_open(&screen, &gates));
                assert!(held_as(&v, MODAL_UNKNOWN_ID), "{name}/{render}: 보류가 아니다: {v:?}");
                let mut delta = obs(&screen, &screen, &gates);
                delta.agent_alive = Some(false);
                assert!(held_as(&judge(&delta), MODAL_UNKNOWN_ID), "{name}/{render}: 마커 델타 경로");
            }
        }
        // ⓑ 서명의 끝 좌표는 **블록 경계**다 — 화면 끝이 아니다. 스크롤백에 남은 잘린 선택기가
        //    라이브 프롬프트 아래에서 영구 전경으로 읽히면 큐 배달이 상시 기아가 된다(생애 창 축 ②).
        let live = fixtures::LIVE_TUI_2_1_261_STATUS_BELOW_PROMPT;
        let history = format!("❯ {exit_cut}\n────────────────\n{live}");
        let sig = modal_signature(&history).expect("스크롤백 잔상이 서명되지 않았다");
        let flat_len = first_run_gates::flatten(&history).chars().count();
        assert!(sig.flat_end < flat_len, "서명 끝이 화면 끝으로 고정돼 있다(역사 판정 불가)");
        let mut reinject = obs(&history, "", &gates);
        reinject.site = Site::Reinject;
        reinject.marker = Some("❯");
        reinject.tail_is_shell_prompt = None;
        reinject.bare_shell = None;
        reinject.idle_quiet = Some(true);
        assert_eq!(
            judge(&reinject),
            Verdict::Ready { evidence: Evidence::MarkerTail },
            "라이브 프롬프트 아래에 있는 **역사**가 재주입을 영구 보류시켰다(기아 방향 회귀)"
        );
        // 같은 문면이 **부트**에서는 보류다(부트 창은 상수 개방 — 관문 축과 같은 부호).
        assert!(held_as(&judge(&boot_all_open(&history, &gates)), MODAL_UNKNOWN_ID));
        // ⓒ 가용성 — 접힌 확인 에코는 아래에 무엇이 오든 모달이 아니다(2026-07-29 역방향 회귀).
        let trust = MODAL_CHOICE_LABELS[0];
        let head: String = trust.chars().take(21).collect();
        let rest: String = trust.chars().skip(21).collect();
        for tail in ["────────────────", "", "  2. 다른 항목"] {
            let echo = format!("❯ {head}\n{rest} ✔\n{tail}\n");
            assert_eq!(modal_signature(&echo), None, "접힌 확인 에코가 관문으로 오탐됐다: {echo:?}");
        }
        // ⓓ ★(0.14.31 · 리뷰 R2(R7회차) · codex blocking B1) **재핀(조여지는 방향)** — R6 은 여기서
        //    `None` 을 못 박아 "꼬리에 평문이 이어지면 거부가 사라진다" 를 잔여로 승인하고 있었다.
        //    그 프레임에 본문 + Return 이 나가면 커서는 **부분 렌더된 종료 선택지** 위다(좌석 사망).
        //    지금은 부분 라벨을 **행**에서 재고 완결 증거는 **블록 안**에서만 세므로 거부가 유지된다.
        //    ★꼬리가 우연히 라벨을 **완성**하는 경우(`No, exi` + `to continue …` = `No,exito…`)는
        //      종료 라벨 축(ⓐ `cursor-on-exit`)이 잡는다 — 거부는 유지되고 규칙만 다르다.
        for (tail, clipped_rule) in [
            ("  이어지는 출력", true),
            ("  다른 pane 의 로그 한 줄", true),
            ("to continue press enter", false),
        ] {
            let screen = format!("❯ {exit_cut}\n{tail}\n");
            for (render, screen) in [("raw", screen.clone()), ("crlf", crlf(&screen))] {
                let sig = modal_signature(&screen)
                    .unwrap_or_else(|| panic!("{render}/{tail:?}: 평문 꼬리 한 줄에 거부가 취소됐다"));
                if clipped_rule {
                    assert!(sig.kinds.contains(&"clipped-choice-row"), "{render}/{tail:?}: {:?}", sig.kinds);
                }
                assert!(
                    held_as(&judge(&boot_all_open(&screen, &gates)), MODAL_UNKNOWN_ID),
                    "{render}/{tail:?}: 판정이 보류가 아니다"
                );
            }
        }
        // ⓔ ★(리뷰 R2(R7회차) · claude major) 완결 증거의 좌표계도 **블록**이다 — 블록 경계 너머의
        //    글자가 라벨을 완성해도 면제가 아니다(`❯ No, exi⏎⏎to continue press enter`).
        for (name, label) in [("exit", MODAL_EXIT_LABEL), ("trust", MODAL_CHOICE_LABELS[0])] {
            let flat = first_run_gates::flatten(label);
            let n = flat.chars().count();
            let head: String = label.chars().take(label.chars().count() - 3).collect();
            let rest: String = flat.chars().skip(n - 3).collect();
            let screen = format!("❯ {head}\n\n{rest} 이어지는 문장\n");
            let sig = modal_signature(&screen)
                .unwrap_or_else(|| panic!("{name}: 경계 너머 글자가 완결 증거로 세어졌다: {screen:?}"));
            assert!(sig.kinds.contains(&"clipped-choice-row"), "{name}: {:?}", sig.kinds);
        }
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

    /// ★(리뷰 R2 · codex blocking) 접힌 종료 라벨(`No, ex⏎it` · CRLF)도 커서-종료 벨트에 걸린다 — 코퍼스
    /// 식별은 평탄화라 그 화면을 여전히 폴더신뢰로 읽으므로, 벨트가 정규화 공간에 남아 있으면 allow 구멍이
    /// 열려 자동확인 Return 이 종료를 고른다(좌석 사망). 두 판정기의 매칭 공간이 같음을 못 박는다.
    #[test]
    fn wrapped_exit_label_keeps_the_cursor_on_exit_belt() {
        let gs = first_run_gates::builtin();
        let on_exit = with_cursor_on(fixtures::FOLDER_TRUST, 2);
        assert!(on_exit.contains("❯ 2. No, exit"), "전제: 커서가 No, exit 위\n{on_exit}");
        for (label, wrapped) in [
            ("LF 단어 안 접힘", on_exit.replace("No, exit", "No, ex\n   it")),
            ("CRLF 단어 안 접힘", on_exit.replace('\n', "\r\n").replace("No, exit", "No, ex\r\n   it")),
            ("쉼표 뒤 접힘", on_exit.replace("No, exit", "No,\n      exit")),
            ("열 정렬 패딩", on_exit.replace("No, exit", "No,   exit")),
        ] {
            let g = first_run_gates::identify(&gs, &wrapped)
                .unwrap_or_else(|| panic!("{label}: 전제 붕괴 — 코퍼스가 접힌 폴더신뢰를 식별하지 못한다\n{wrapped}"));
            assert_eq!(g.id, "folder-trust", "{label}");
            let sig = modal_signature(&wrapped).unwrap_or_else(|| panic!("{label}: 모달 어휘를 못 봤다"));
            assert!(
                sig.cursor_on_exit,
                "{label}: 접힌 종료 라벨이 벨트를 지나쳤다 — 자동확인 Return 이 종료를 고른다\n{wrapped}"
            );
            assert!(sig.kinds.contains(&"cursor-on-exit"), "{label}: {:?}", sig.kinds);
            assert!(sig.kinds.contains(&"choice-row"), "{label}: 접힌 라벨이 선택지 행으로 안 읽혔다 {:?}", sig.kinds);
        }
        // 단어 안에서 접힌 긍정 라벨도 선택지 행이다(평탄화 검색 · 앞 경계는 정규화 공간).
        let wrapped_yes = fixtures::FOLDER_TRUST.replace("Yes, I trust this folder", "Yes, I tru\n   st this folder");
        assert!(wrapped_yes.contains("tru\n   st"), "전제: 접힘");
        let sig = modal_signature(&wrapped_yes).expect("접힌 긍정 라벨");
        assert!(sig.kinds.contains(&"choice-row"), "{:?}", sig.kinds);
        assert!(!sig.cursor_on_exit, "커서는 Yes 위");
        // 반례: 라벨이 잘려 `No, exi` 로 끝나면 **종료 확정은 아니다**(부정 증거가 없다). 그 화면에서 allow 구멍이
        //   열리지 않는 근거는 이 플래그가 아니라 양성 증거 술어(`cursor_resolves_to_label` · 리뷰 R3)다 —
        //   `inject_guard::allow_hole_needs_the_cursor_on_the_gate_action_label` 이 그 경로를 직접 잰다.
        let cut = on_exit.replace("No, exit", "No, exi");
        let sig = modal_signature(&cut).expect("커서 행");
        assert!(!sig.cursor_on_exit && sig.kinds.contains(&"cursor-on-numbered-item"), "{:?}", sig.kinds);
        // 좌표: 종료 라벨·푸터의 끝은 평탄화 좌표다(생애 창 재료) — 푸터가 마지막 줄이라 화면 끝과 같다.
        let sig = modal_signature(&on_exit.replace("No, exit", "No, ex\r\n   it")).expect("접힌 종료");
        assert_eq!(sig.flat_end, first_run_gates::flatten(&on_exit).chars().count(), "flat_end 가 평탄화 좌표가 아니다");
        // 반례: 평탄화가 본문의 우연한 토큰을 종료 라벨로 오독하지 않는다 — 앞에 `N.` 경계·커서가 없다.
        assert!(modal_signature("$ grep -c No,exit log.txt\n0\n❯ \n").is_none());
        // ★받아들인 잔여(codex 설계 검토): `2.No,exit-code …` 처럼 번호 경계 뒤에 라벨이 붙은 본문은 선택지 행으로
        //   읽혀 **보류**된다 — 비대칭 원칙(오탐의 귀결은 보류)상 허용하며, 보류를 풀려고 뒤 경계를 요구하면 접힌
        //   라벨(`No, ex⏎it`)을 다시 놓친다. 여기 박제해 두어 누가 "완화" 하면 검체가 말하게 한다.
        let sig = modal_signature("nothing to commit\n  2.No,exit-code is not a flag\n").expect("받아들인 잔여");
        assert!(sig.kinds == vec!["choice-row"] && !sig.cursor_on_exit, "{:?}", sig.kinds);
    }

    /// ★(0.14.31 · 리뷰 R4) **두 스캐너의 좌표 파리티** — 렌더 변형 5종(LF · CRLF · NBSP · 단어 안 접힘 ·
    /// 열 정렬 패딩) × 사례 3종(종료 커서 · 액션 커서 · 잘린 종료 커서)에서 `modal_signature` 의
    /// `cursor_on_exit` 와 `cursor_resolves_to_label` 이 **같은 사실**을 낸다. 두 소비처가 갈리면 벨트에
    /// 구멍이 생긴다(R2 blocking 의 그 자리) — 스캐너·좌표계가 하나라는 계약을 렌더 축으로 잰다.
    ///
    /// ★출처: codex gpt-6-astra 위임 산출(`impl/codex/R1-WP1-HF-r4-parity-tests.rs`) · 전행 검토 후 채택
    ///   (프로덕션 무접촉 · stdlib 만 · 도우미 재사용 · 문면 사본 0 · 전제 붕괴를 단언 실패와 구별해 panic).
    #[test]
    fn modal_and_confirm_scanners_agree_across_render_variants() {
        let anchors_owned = trust_anchors();
        let anchors: Vec<&str> = anchors_owned.iter().map(String::as_str).collect();
        let gates = first_run_gates::builtin();
        let exit = MODAL_EXIT_LABEL;
        let action = "Yes, I trust this folder";
        let on_exit = with_cursor_on(fixtures::FOLDER_TRUST, 2);
        let on_action = with_cursor_on(fixtures::FOLDER_TRUST, 1);
        let clipped_exit = exit
            .strip_suffix('t')
            .unwrap_or_else(|| panic!("LF / 잘림 전제 붕괴: 종료 라벨 끝이 t가 아니다\n{on_exit}"));
        let on_clipped_exit = on_exit.replace(exit, clipped_exit);

        for variant in ["LF", "CRLF", "NBSP", "단어 안 접힘", "열 정렬 패딩"] {
            // 커서를 먼저 옮긴다. with_cursor_on의 행 재조립이 CRLF를 지우지 않게 한다.
            // 잘림도 렌더 변형 전에 적용해 접힌 종료 라벨까지 반드시 한 글자 자른다.
            for (case, base, label, expected_exit, expected_resolution) in [
                ("종료 커서", &on_exit, exit, true, true),
                ("액션 커서", &on_action, action, false, true),
                ("잘린 종료 커서", &on_clipped_exit, exit, false, false),
            ] {
                let screen = match variant {
                    "LF" => base.clone(),
                    "CRLF" => base.replace('\n', "\r\n"),
                    "NBSP" => base.replace(". ", ".\u{a0}"),
                    "단어 안 접힘" => base
                        .replace("No, ex", "No, ex\n   ")
                        .replace("Yes, I tru", "Yes, I tru\n   "),
                    "열 정렬 패딩" => base.replace(". ", ".    "),
                    _ => panic!("{variant} / {case}: 알 수 없는 렌더 변형\n{base}"),
                };
                if variant != "LF" {
                    assert_ne!(
                        &screen, base,
                        "{variant} / {case}: 렌더 변형이 적용되지 않았다\n{screen}"
                    );
                }
                let gate = first_run_gates::identify(&gates, &screen).unwrap_or_else(|| {
                    panic!("{variant} / {case}: 전제 붕괴 — 코퍼스 식별 실패\n{screen}")
                });
                if gate.id != "folder-trust" {
                    panic!(
                        "{variant} / {case}: 전제 붕괴 — folder-trust 대신 {} 식별\n{screen}",
                        gate.id
                    );
                }
                let sig = modal_signature(&screen).unwrap_or_else(|| {
                    panic!("{variant} / {case}: 전제 붕괴 — 모달 서명 없음\n{screen}")
                });
                assert_eq!(
                    sig.cursor_on_exit, expected_exit,
                    "{variant} / {case}: modal_signature의 종료 커서 판정 불일치\n{screen}"
                );
                assert_eq!(
                    cursor_resolves_to_label(&screen, label, &anchors),
                    expected_resolution,
                    "{variant} / {case}: cursor_resolves_to_label({label:?}) 판정 불일치\n{screen}"
                );
            }
        }
    }


    /// 폴더신뢰 관문의 질문 문면(코퍼스 소유 · 사본 0) — 검체가 쓰는 선택 블록 경계 재료.
    fn trust_anchors() -> Vec<String> {
        first_run_gates::builtin()
            .into_iter()
            .find(|g| g.id == "folder-trust")
            .expect("코퍼스에 folder-trust")
            .needles
    }

    /// ★(0.14.31 · 리뷰 R3·R4 · codex blocking) 양성 증거 술어 — 커서가 **그 라벨 전문** 위에 있고 **활성 선택
    /// 블록에 경쟁 커서가 없을 때만** 참. 잘린 라벨·번호 없는 잘림·점 뒤 공백 없는 렌더·커서 부재·모호
    /// (양성+미해소 커서 공존)는 전부 거짓이어야 한다(구멍이 닫히는 방향).
    #[test]
    fn cursor_resolution_needs_the_complete_label_and_no_competing_cursor() {
        let yes = "Yes, I trust this folder";
        let exit = MODAL_EXIT_LABEL;
        let anchors_owned = trust_anchors();
        let anchors: Vec<&str> = anchors_owned.iter().map(|s| s.as_str()).collect();
        let resolves = |screen: &str, label: &str| cursor_resolves_to_label(screen, label, &anchors);
        // 양성: 실측 화면 · 접힌 라벨 · CRLF · 열 정렬 패딩 · 번호 없는 커서.
        for (label, screen) in [
            ("실측 폴더신뢰", fixtures::FOLDER_TRUST.to_string()),
            ("접힌 라벨", fixtures::FOLDER_TRUST.replace("Yes, I trust this folder", "Yes, I tru\n   st this folder")),
            ("CRLF", fixtures::FOLDER_TRUST.replace('\n', "\r\n")),
            ("열 정렬 패딩", fixtures::FOLDER_TRUST.replace("Yes, I trust", "Yes,   I  trust")),
            ("번호 없는 커서", fixtures::FOLDER_TRUST.replace("❯ 1. Yes", "❯ Yes")),
        ] {
            assert!(resolves(&screen, yes), "{label}: 액션 라벨 위 커서를 못 봤다\n{screen}");
        }
        // 음성: 잘림 3종 · 커서 부재 · 다른 행 위 커서 · 빈 라벨.
        let on_exit = with_cursor_on(fixtures::FOLDER_TRUST, 2);
        for (label, screen, needle) in [
            ("잘린 번호 종료", on_exit.replace("No, exit", "No, exi"), exit),
            ("점 뒤 공백 없음", on_exit.replace("❯ 2. No, exit", "❯ 2.No, exi"), exit),
            ("번호 없는 잘림", on_exit.replace("❯ 2. No, exit", "❯ No, exi"), exit),
            ("커서 부재", on_exit.replace("❯ 2. No, exit", "  2. No, exit"), exit),
            ("커서는 다른 행", fixtures::FOLDER_TRUST.to_string(), exit),
            ("잘린 액션 라벨", fixtures::FOLDER_TRUST.replace("Yes, I trust this folder", "Yes, I trust this fold"), yes),
        ] {
            assert!(!resolves(&screen, needle), "{label}: 불완전/무관한 라벨이 양성으로 읽혔다\n{screen}");
        }
        assert!(!resolves(fixtures::FOLDER_TRUST, ""), "빈 라벨이 양성이다");
        assert!(!resolves(fixtures::FOLDER_TRUST, "   "), "공백 라벨이 양성이다");
        // 커서 부재 화면(건강한 셸·에코)에서는 어떤 라벨도 양성이 아니다.
        assert!(!resolves(fixtures::READY_SHELL, yes));
        assert!(!resolves("Yes, I trust this folder ✔\n", yes), "확인 에코가 선택으로 읽혔다");
        // 종료 라벨 전문 위 커서는 `modal_signature` 의 `cursor_on_exit` 와 **같은 사실**이다(스캐너 1개).
        assert!(resolves(&on_exit, exit) && modal_signature(&on_exit).unwrap().cursor_on_exit);

        // ── ★(리뷰 R3b·R4 · codex blocking) **모호는 양성을 이긴다** — 액션 라벨 위 커서가 있어도 선택 블록 안에
        //    해소되지 않은 다른 커서가 있으면 어느 쪽이 실제 선택인지 모른다(부분 렌더·잔상). 전부 거짓.
        //    R4 가 더한 것: 어휘로 면제하던 짧은 잘림(`❯ No` · `❯ No,` · `❯ N`)과 **다음 줄로 접힌** 선택지
        //    (`❯⏎  No` — 종전 규칙은 '행이 비었다' 로 읽어 면제했다), 그리고 질문 재출현 잔상.
        let mut ambiguous: Vec<(String, String)> = vec![
            // codex R3b 반례 그대로: 두 행 모두 커서 · 두 번째는 잘려 `cursor_on_exit` 도 못 세운다.
            ("잘린 종료 커서 공존".into(), fixtures::FOLDER_TRUST.replace("\x20 2. No, exit", "❯ 2. No, exi")),
            ("번호 없는 잘린 종료 커서 공존".into(), fixtures::FOLDER_TRUST.replace("\x20 2. No, exit", "❯ No, exi")),
            ("점 뒤 공백 없는 종료 커서 공존".into(), fixtures::FOLDER_TRUST.replace("\x20 2. No, exit", "❯ 2.No, exi")),
            // 코퍼스 어휘 밖의 잘린 선택지라도 **번호 커서**면 경쟁이다(테마·새기능 화면의 부분 렌더).
            ("번호 커서 + 미상 라벨".into(), fixtures::FOLDER_TRUST.replace("\x20 2. No, exit", "❯ 2. Someth")),
            // ★(R4) 어휘 앞머리 4자에 못 미치는 잘림 — 종전 규칙이 전부 면제하던 자리.
            ("두 글자 잘림".into(), fixtures::FOLDER_TRUST.replace("\x20 2. No, exit", "❯ No")),
            ("쉼표까지 잘림(뒤에 푸터)".into(), fixtures::FOLDER_TRUST.replace("\x20 2. No, exit", "❯ No,")),
            ("번호만 남음".into(), fixtures::FOLDER_TRUST.replace("\x20 2. No, exit", "❯ 2")),
            ("한 글자".into(), fixtures::FOLDER_TRUST.replace("\x20 2. No, exit", "❯ N")),
            // ★(R4 · codex) 다음 줄로 접힌 선택지 — 커서 **행**은 비었지만 화면에는 라벨이 이어진다.
            ("다음 줄로 접힌 선택지".into(), fixtures::FOLDER_TRUST.replace("\x20 2. No, exit", "❯\n  No")),
            // ★(R4 · codex) 재그리기 잔상: 질문 조각이 먼저 나오고 그 뒤에 미해소 커서, 다시 질문·해소 커서.
            // ★문면 리터럴 사본 금지(H-READY-13) — 질문 조각은 **코퍼스에서** 읽는다.
            ("질문 재출현 + 앞선 잔상 커서".into(),
             format!("{}\n❯ No\n{}", anchors[0], fixtures::FOLDER_TRUST)),
        ];
        // 종료 라벨이 완전히 보이는 경쟁 행(부정 증거까지 서는 화면)도 당연히 거짓이다.
        ambiguous.push(("종료 커서 공존(전문)".into(), fixtures::FOLDER_TRUST.replace("\x20 2. No, exit", "❯ 2. No, exit")));
        for (label, screen) in &ambiguous {
            assert!(
                !resolves(screen, yes),
                "{label}: 모호한 화면에서 구멍이 열렸다(그 Return 이 종료를 누를 수 있다)\n{screen}"
            );
        }
        // 그러나 **선택 블록 밖·라벨 없음** 커서는 경쟁이 아니다 — 관문 문면이 시작되기 전의 셸 잔상과
        // 화면 꼬리의 빈 입력 프롬프트까지 경쟁으로 세우면 그 사용자들에게 자동확인이 영영 열리지 않는다.
        for (label, screen) in [
            ("질문 앞 셸 프롬프트 잔상", format!("~/work ╰─❯ ls -al\n{}", fixtures::FOLDER_TRUST)),
            ("질문 앞 부트 로그 커서", format!("❯ claude --dangerously-skip-permissions\n{}", fixtures::FOLDER_TRUST)),
            ("빈 입력 상자 꼬리", format!("{}❯ \n", fixtures::FOLDER_TRUST)),
            ("실측 NBSP 꼬리", format!("{}❯\u{a0}\n", fixtures::FOLDER_TRUST)),
        ] {
            assert!(
                resolves(&screen, yes),
                "{label}: 선택기가 아닌 커서를 경쟁으로 세워 자동확인이 닫혔다\n{screen}"
            );
        }
        // ★anchor 를 못 찾으면 화면 전체가 블록이다(fail-closed) — 질문 없는 화면에서는 셸 잔상도 경쟁이다.
        assert!(
            !cursor_resolves_to_label(&format!("~/work ╰─❯ ls -al\n❯ 1. {yes}\n"), yes, &anchors),
            "질문이 없는 화면에서 블록 경계를 임의로 넓혔다(fail-closed 위반)"
        );
        assert!(
            !cursor_resolves_to_label(&format!("~/work ╰─❯ ls -al\n{}", fixtures::FOLDER_TRUST), yes, &[]),
            "anchor 빈 목록에서 블록을 화면 전체로 보지 않았다(면제가 넓어졌다)"
        );
        // 경쟁 판정의 순수 술어 직접 실행 — 번호 커서 · 라벨 유무 · 블록 경계.
        let probe = |line: &str, block: usize| -> bool {
            let f = screen_frame(line);
            let rows = cursor_rows(&f);
            assert_eq!(rows.len(), 1, "전제: 커서 1개\n{line}");
            cursor_row_competes(&rows[0], &f.flat, block)
        };
        assert!(probe("❯ 2. Anything at all", 0), "번호 커서가 경쟁이 아니다");
        assert!(probe("❯ No, exi", 0), "잘린 종료 라벨이 경쟁이 아니다");
        assert!(probe("❯ N", 0), "한 글자 잘림이 경쟁이 아니다");
        assert!(probe("❯ ls -al", 0), "블록 안의 셸 명령 잔상이 경쟁이 아니다(어휘로 면제했다)");
        assert!(!probe("❯ ls -al", 99), "블록보다 앞의 커서가 경쟁이다");
        assert!(!probe("❯ ", 0), "라벨 없는 빈 입력 상자가 경쟁이다");
        assert!(!probe("❯\u{a0}", 0), "NBSP 꼬리가 경쟁이다");
        assert!(probe("❯\n  No", 0), "다음 줄로 접힌 선택지가 경쟁이 아니다");
        // 점 뒤 공백 없는 번호 행은 경쟁(ⓐ') · 소수는 아니다(엄격 파서의 `1.5 hours` 배제와 같은 방향).
        //   ★번호 축은 **위치 무관**이다(블록보다 앞이어도 선택기의 행이다).
        assert!(probe("❯ 2.No, exi", 99), "점 뒤 공백 없는 번호 행이 경쟁이 아니다");
        assert!(probe("❯ 12.Someth", 99), "두 자리 번호 행이 경쟁이 아니다");
        assert!(probe("❯ 2. Dark mode", 99), "번호 커서가 블록 밖이라고 면제됐다");
        assert!(!probe("❯ 1.5 hours", 99), "소수가 번호 행으로 읽혔다");
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
        // ★(0.14.31 · 리뷰 R1) 두 표를 **이어서** 돈다: `NON_GATE_SCREENS`(프로덕션 수리기의 입력 =
        //   그 화면에서 주입이 옳은 화면)와 `MEASURED_NON_CORPUS_MODALS`(검체 전용 = 코퍼스 밖 모달).
        //   표를 가른 뒤에도 **모달 판정의 기대표는 하나**여야 한다 — 갈라 두면 어느 한쪽의 회귀가
        //   조용히 통과한다.
        for &(id, screen) in fixtures::NON_GATE_SCREENS
            .iter()
            .chain(fixtures::MEASURED_NON_CORPUS_MODALS)
        {
            // ★(0.14.31 · WP-1 H-2) 실측 벤더 모달(2.1.261 커스텀 API 키 확인창)은 코퍼스 밖이지만
            //   `Enter to confirm` ∧ `Esc to cancel` 푸터가 전경이라 모달이다.
            let expect_modal = matches!(
                id,
                "audit-log-line" | "live-permission-prompt" | "custom-api-key-modal-2.1.261"
            );
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

    /// ★(0.14.31 · WP-1 H-1 · 리뷰 R1b · W1) Boot Valve 창의 재료 `quiet_secs` 가 **전송층에서 도달 가능한가** —
    /// 유휴 PTY(Windows 러너에서는 ConPTY)가 스스로 바이트를 내지 않는지를 **실행으로** 잰다. 리뷰 지적: 밸브는
    /// `time_fallback ∧ quiet_secs ≥ 3` 을 요구하는데 ConPTY 의 유휴 출력 주기는 측정된 적이 없었다 — ConPTY 가
    /// 유휴에 3s 미만 간격으로 바이트를 내면 밸브 단독 경로 좌석은 영영 GatePending(unidentified) 이다(보류 방향
    /// 이지만 디렉티브 미주입 = 치명위험 ③ 형상). 이 검체는 windows-health.yml 의 **차단** 스텝
    /// (`cargo test --lib readiness::`)에서 windows-latest 실기로 돈다(macOS 에서는 같은 코드가 /bin/sh 로 돌아
    /// 컴파일·의미가 함께 검증된다).
    ///
    /// 측정 설계(codex 설계 검토 반영 · 가짜 정적을 정적으로 읽지 않는다):
    ///   A 정착: 기동 바이트가 있고(펌프 생존 증명) 1.0s 무출력이면 정착 · 상한 12s(계속 그리면 실패).
    ///   B 유휴 창 4.0s(> BOOT_VALVE_QUIET_SECS 3.0 + 여유): 바이트 0 이어야 한다 · EOF/읽기 오류/채널 단절/자식
    ///     종료는 "정적" 이 아니라 실패다.
    ///   C 전송층 생존 재증명: 창이 끝난 뒤 명령을 써 넣고 그 **출력**(입력 에코와 구분되는 토큰 조합)이 8s 안에
    ///     오는지 본다 — ConPTY 가 DSR 응답을 기다리며 펌프를 멈춘 상태(cysd reader 가 답하는 이유)는 조용해
    ///     보이지만 C 에서 드러난다. DSR(`ESC[6n`)은 청크 경계 carry 3바이트로 답한다(cysd 와 같은 규율).
    /// 범위의 정직: 이것은 PTY/ConPTY **전송층**의 유휴 거동이지 ink(claude 렌더러)의 유휴 거동이 아니다. ink 의
    /// 유휴 무재그림은 macOS 라이브 좌석 idle_secs 실측(602~6841s · 431~2595s)이 근거이고, "ink-on-Windows" 는
    /// 릴리스 게이트의 격리 config dir 라이브 부트 1회 계측 항목으로 남는다(노트 참조).
    /// 유휴 계측 창(초) — 밸브 임계 `BOOT_VALVE_QUIET_SECS`(3.0)보다 길어야 한다(전제 단언).
    const WINDOW_SECS: f64 = 4.0;

    #[test]
    fn pty_idle_shell_emits_no_bytes_within_valve_quiet_window() {
        use portable_pty::{native_pty_system, CommandBuilder, PtySize};
        use std::io::{Read, Write};
        use std::sync::mpsc;
        use std::time::{Duration, Instant};

        // ★(0.14.31 · 리뷰 R1(R6회차) · claude 적대) **부하 위양성 1회로 릴리스 게이트를 막지 않는다.**
        //   이 검체는 windows-health.yml 의 차단 스텝(`if:`·continue-on-error 금지)에서 돌고 실 PTY 를
        //   13s 동안 잰다 — 러너 부하가 높으면 정착 직후 지연 청크가 유휴 창에 도착해 적색이 될 수 있고,
        //   같은 트리에서 다른 라이브 검체의 부하 위양성이 이미 실측됐다(노트 §14-8: 부하 126 → 55 fail,
        //   단독 실행 PASS). 그래서 **측정을 최대 2회 시도**하고, 두 번 다 실패했을 때만 적색이다.
        //   완화가 아니라 **측정의 신뢰도**를 올리는 것이다: 판정 기준(유휴 창 바이트 0)은 그대로이고,
        //   실패한 시도의 실측값도 전부 로그로 남긴다(무엇이 왜 실패했는지 밖에서 보인다).
        let probe = |attempt: usize| -> Result<(), String> {
        let pty = native_pty_system();
            let pair = pty
                .openpty(PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 })
                .expect("openpty");
            #[cfg(windows)]
            let mut cmd = CommandBuilder::new("cmd.exe");
            #[cfg(not(windows))]
            let mut cmd = {
                let mut c = CommandBuilder::new("/bin/sh");
                c.arg("-i");
                c.env("PS1", "probe$ ");
                c.env("ENV", ""); // 사용자 rc 파일 미판독(결정론 기동)
                c
            };
            cmd.env("TERM", "xterm-256color");
            let mut child = pair.slave.spawn_command(cmd).expect("spawn shell in pty");
            drop(pair.slave);
            let mut reader = pair.master.try_clone_reader().expect("pty reader");
            let mut writer = pair.master.take_writer().expect("pty writer");
            let (tx, rx) = mpsc::channel::<Result<Vec<u8>, String>>();
            std::thread::spawn(move || {
                let mut buf = [0u8; 4096];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => {
                            let _ = tx.send(Err("EOF".into()));
                            break;
                        }
                        Ok(n) => {
                            if tx.send(Ok(buf[..n].to_vec())).is_err() {
                                break;
                            }
                        }
                        Err(e) => {
                            let _ = tx.send(Err(format!("read error: {e}")));
                            break;
                        }
                    }
                }
            });
            // DSR(`ESC[6n`) 응답 — 청크 경계 carry 3바이트(cysd reader 와 같은 규율). 답하지 않으면 ConPTY 펌프가
            // 멈춰 "조용해 보이는" 가짜 정적이 된다.
            let mut carry: Vec<u8> = Vec::new();
            let mut answer_dsr = |chunk: &[u8], w: &mut Box<dyn Write + Send>| {
                let mut joined = carry.clone();
                joined.extend_from_slice(chunk);
                let mut i = 0usize;
                while i + 4 <= joined.len() {
                    if &joined[i..i + 4] == b"\x1b[6n" {
                        let _ = w.write_all(b"\x1b[1;1R");
                        i += 4;
                    } else {
                        i += 1;
                    }
                }
                let _ = w.flush();
                carry = joined[joined.len().saturating_sub(3)..].to_vec();
            };
            let settle = Duration::from_millis(1000);
            let settle_cap = Duration::from_secs(12);
            let window = Duration::from_millis((WINDOW_SECS * 1000.0) as u64);
            let outcome = (|| -> Result<(usize, usize, usize, Duration), String> {
                // A — 정착.
                let start = Instant::now();
                let mut startup_bytes = 0usize;
                let mut last_byte_at = Instant::now();
                loop {
                    match rx.recv_timeout(Duration::from_millis(100)) {
                        Ok(Ok(chunk)) => {
                            startup_bytes += chunk.len();
                            last_byte_at = Instant::now();
                            answer_dsr(&chunk, &mut writer);
                        }
                        Ok(Err(e)) => return Err(format!("기동 중 전송층 단절: {e}")),
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            if startup_bytes > 0 && last_byte_at.elapsed() >= settle {
                                break;
                            }
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => return Err("reader 스레드 소실".into()),
                    }
                    if start.elapsed() > settle_cap {
                        return Err(format!(
                            "셸이 {settle_cap:?} 안에 정착하지 않았다(기동 바이트 {startup_bytes}) — 유휴에도 계속 그린다"
                        ));
                    }
                }
                // B — 유휴 창.
                let t0 = Instant::now();
                let mut idle_bytes = 0usize;
                let mut idle_chunks = 0usize;
                let mut max_gap = Duration::ZERO;
                let mut last = t0;
                while t0.elapsed() < window {
                    let remain = window.saturating_sub(t0.elapsed()).max(Duration::from_millis(1));
                    match rx.recv_timeout(remain) {
                        Ok(Ok(chunk)) => {
                            idle_bytes += chunk.len();
                            idle_chunks += 1;
                            let now = Instant::now();
                            max_gap = max_gap.max(now - last);
                            last = now;
                            answer_dsr(&chunk, &mut writer);
                        }
                        Ok(Err(e)) => return Err(format!("유휴 창 중 전송층 단절: {e}")),
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(mpsc::RecvTimeoutError::Disconnected) => return Err("reader 스레드 소실".into()),
                    }
                }
                max_gap = max_gap.max(Instant::now() - last);
                if child.try_wait().map_err(|e| format!("try_wait: {e}"))?.is_some() {
                    return Err("셸이 유휴 창 중 종료됐다 — 정적이 아니라 사망이다".into());
                }
                // C — 전송층 생존 재증명(출력 토큰은 입력 에코와 다르게 조합된다).
                #[cfg(windows)]
                writer.write_all(b"echo PROBE_^OK\r\n").map_err(|e| e.to_string())?;
                #[cfg(not(windows))]
                writer.write_all(b"echo PROBE_\"OK\"\n").map_err(|e| e.to_string())?;
                writer.flush().map_err(|e| e.to_string())?;
                let t1 = Instant::now();
                let mut seen: Vec<u8> = Vec::new();
                loop {
                    match rx.recv_timeout(Duration::from_millis(200)) {
                        Ok(Ok(chunk)) => {
                            answer_dsr(&chunk, &mut writer);
                            seen.extend_from_slice(&chunk);
                            if String::from_utf8_lossy(&seen).contains("PROBE_OK") {
                                break;
                            }
                        }
                        Ok(Err(e)) => return Err(format!("응답 대기 중 전송층 단절: {e}")),
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(mpsc::RecvTimeoutError::Disconnected) => return Err("reader 스레드 소실".into()),
                    }
                    if t1.elapsed() > Duration::from_secs(8) {
                        return Err(format!(
                            "유휴 창 뒤 명령 응답이 8s 안에 없다 — 펌프 정지(가짜 정적) 의심 · 수신 {}B",
                            seen.len()
                        ));
                    }
                }
                Ok((startup_bytes, idle_bytes, idle_chunks, max_gap))
            })();
        let _ = child.kill();
        let (startup_bytes, idle_bytes, idle_chunks, max_gap) = outcome?;
        eprintln!(
            "[pty-idle-probe] attempt={attempt} os={} startup_bytes={startup_bytes} idle_window={:.1}s \
             idle_bytes={idle_bytes} idle_chunks={idle_chunks} max_gap={:.2}s valve_quiet={BOOT_VALVE_QUIET_SECS}s",
            std::env::consts::OS,
            window.as_secs_f64(),
            max_gap.as_secs_f64()
        );
        if startup_bytes == 0 {
            return Err("기동 바이트 0 — 펌프 생존이 증명되지 않았다".to_string());
        }
        if idle_bytes != 0 {
            return Err(format!(
                "유휴 PTY 가 {window:?} 동안 {idle_bytes}B({idle_chunks} 청크)를 냈다 — quiet_secs 가 밸브 창 \
                 {BOOT_VALVE_QUIET_SECS}s 에 도달하지 못한다(밸브 단독 경로 영구 보류)"
            ));
        }
        Ok(())
        };
        assert!(WINDOW_SECS > BOOT_VALVE_QUIET_SECS, "검체 전제: 유휴 창이 밸브 임계보다 길다");
        let attempts = 2usize;
        let mut last = String::new();
        for attempt in 1..=attempts {
            match probe(attempt) {
                Ok(()) => return,
                Err(e) => {
                    eprintln!("[pty-idle-probe] attempt={attempt}/{attempts} 실패 — {e}");
                    last = e;
                }
            }
        }
        panic!("PTY 유휴 계측이 {attempts}회 연속 실패 — 마지막 사유: {last}");
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

    /// ★(리뷰 R1) 재주입 생애 창 — **라이브 2.1.261 그리드**(입력 상자 아래 괘선·상태줄 · 실측 2026-09-06
    /// 10:18:58)에서 본문에 남은 모달 어휘는 역사다(창 닫힘 → 재주입). 종전 축 ①("마커 뒤 화면 전체 공백")은
    /// 이 그리드에서 결코 참이 되지 않아 pack-update 재주입이 문면이 스크롤로 사라질 때까지 보류됐다.
    /// 같은 그리드에서 **전경 모달**은 여전히 보류다 — 줄 단위 축 ①' 이 조여지는 방향으로만 갈린다.
    #[test]
    fn reinject_modal_window_closes_behind_a_live_2_1_261_prompt_with_status_line_below() {
        let gates = first_run_gates::builtin();
        let reinject = |screen: &str| -> Verdict {
            let mut o = obs(screen, "", &gates);
            o.site = Site::Reinject;
            o.marker = Some("❯");
            o.tail_is_shell_prompt = None;
            o.bare_shell = None;
            o.idle_quiet = Some(true);
            judge(&o)
        };
        let live = fixtures::LIVE_TUI_2_1_261_STATUS_BELOW_PROMPT;
        // ⓞ 모달 어휘 없는 라이브 그리드 자체 — 재주입 가능(대조군 · 모달 축 무관).
        assert_eq!(reinject(live), Verdict::Ready { evidence: Evidence::MarkerTail });
        // ① 본문에 모달 어휘(마스터가 감사 계획서를 `cat` 한 화면) + 그 아래 라이브 프롬프트·상태줄 → 역사 → 재주입.
        let body_vocab = format!(
            "❯ cat IMPLEMENTATION-PLAN.md\n  H-1: 화면에 `Enter to confirm` ∧ `Esc to cancel` 이면 GateHeld · \
             `Yes, I trust this folder`/`Yes, I accept` 선택지 행\n{live}"
        );
        assert!(modal_signature(&body_vocab).is_some(), "검체 전제: 본문에 모달 어휘가 있어야 한다");
        assert_eq!(
            reinject(&body_vocab),
            Verdict::Ready { evidence: Evidence::MarkerTail },
            "라이브 그리드 아래의 본문 어휘가 창을 영구 보류로 접었다(종전 축 ① 결함 재발)"
        );
        // ①′ 2.1.241 레이아웃(`? for shortcuts` 가 프롬프트 **위**) 뒤에 마커가 마지막인 화면도 종전대로 닫힌다.
        let old_layout = format!("{}\n{}", fixtures::LIVE_PERMISSION_PROMPT, fixtures::LIVE_TUI_AT_PROMPT);
        assert_eq!(reinject(&old_layout), Verdict::Ready { evidence: Evidence::MarkerTail });
        // ② 같은 그리드에서 **전경** 권한 프롬프트(커서 행에 라벨 · 푸터가 마커 뒤) → 보류.
        let foreground = format!("{}\n{}", live.trim_end_matches('\n'), fixtures::LIVE_PERMISSION_PROMPT);
        assert!(held_as(&reinject(&foreground), MODAL_UNKNOWN_ID), "전경 모달이 통과됐다: {:?}", reinject(&foreground));
        // ③ 사람이 치던 초안(`❯ 작업 이어서`) — 마커 줄에 문면 → 닫지 않는다(보수).
        let draft = body_vocab.replace("❯ \n", "❯ 작업 이어서\n");
        assert!(held_as(&reinject(&draft), MODAL_UNKNOWN_ID));
        // ④ 상태줄 **아래**에 모달 푸터가 다시 그려진 화면 — 축 ②(마커 뒤 어휘) → 보류.
        let footer_below = format!("{body_vocab}Enter to confirm · Esc to cancel\n");
        assert!(held_as(&reinject(&footer_below), MODAL_UNKNOWN_ID));
        // ⑤ codex 설계 검토 반례 — 어휘 위 · 빈 `❯` 줄 · 그 아래 **정체 모를 모달 본문**(레이아웃 증거 0) → 보류.
        let ambiguous = "Enter to confirm · Esc to cancel\n❯ \n  additional modal content\n";
        assert!(held_as(&reinject(ambiguous), MODAL_UNKNOWN_ID), "빈 마커 줄만으로 창이 닫혔다(애매한 프레임 통과)");
        // ⑥ 부분 렌더 — 빈 커서 줄 아래에 번호 항목 행이 그려지는 중 → 보류(모달 형상).
        let partial = "Do you want to proceed?\n❯ \n  2. Yes, and don't ask again\nEnter to confirm · Esc to cancel\n";
        assert!(held_as(&reinject(partial), MODAL_UNKNOWN_ID));
        let partial2 = "Enter to confirm · Esc to cancel\n❯ \n  2. Yes, and don't ask again\n────────────────\n";
        assert!(held_as(&reinject(partial2), MODAL_UNKNOWN_ID), "번호 항목 행이 괘선 증거를 이겼어야 한다");
        // ⑦ 접힌 라벨 — 커서 줄이 비고 라벨이 다음 줄로 접힌 형상 + 상태줄 어휘 없음 → 보류.
        let wrapped = "Enter to confirm · Esc to cancel\n❯\n  Yes, I trust this\n  folder\n";
        assert!(held_as(&reinject(wrapped), MODAL_UNKNOWN_ID));
        // ⑧ 확인 에코만 남은 통과 후 화면 + 2.1.261 상태줄 — 모달이 아니다(2026-07-29 역방향) → 재주입.
        let echo_only = format!("  Yes, I trust this folder ✔\n{live}");
        assert!(modal_signature(&echo_only).is_none());
        assert_eq!(reinject(&echo_only), Verdict::Ready { evidence: Evidence::MarkerTail });
        // ⑨ 부트 창은 상수 — 같은 '본문 어휘' 화면도 부트에서는 보류(관문 축과 같은 부호 · 받아들인 잔여).
        assert!(held_as(&judge(&boot_all_open(&body_vocab, &gates)), MODAL_UNKNOWN_ID));
        // ⑩ 롤백 — `legacy_v1` 이면 모달 축 자체가 없다(전경 모달도 종전대로 마커 꼬리로 Ready).
        let mut rolled = obs(&foreground, "", &gates);
        rolled.site = Site::Reinject;
        rolled.marker = Some("❯");
        rolled.legacy_v1 = true;
        assert_eq!(judge(&rolled), Verdict::Ready { evidence: Evidence::MarkerTail });
        // ⑪ ★(리뷰 R1b · 실측 12:13:56) 라이브 그리드의 **실제 바이트** — 대기 프롬프트 줄은 `❯` + U+00A0(NBSP).
        //    ①' 의 "마커 뒤 공백만" 이 NBSP 를 공백으로 읽어야 라이브 좌석에서 창이 닫힌다(안 읽으면 ①' 은 라이브
        //    그리드에서 결코 참이 되지 않아 R-E 결함이 그대로 남는다). 같은 그리드의 전경 모달·초안은 여전히 보류.
        let nbsp = fixtures::LIVE_TUI_2_1_261_NBSP_PROMPT;
        assert!(nbsp.contains("❯\u{a0}\n"), "검체 전제: 실측 NBSP 바이트");
        assert!(waiting_prompt_with_harmless_trailer(nbsp, "❯"), "NBSP 대기 프롬프트를 빈 줄로 읽지 못했다");
        assert_eq!(reinject(nbsp), Verdict::Ready { evidence: Evidence::MarkerTail });
        let nbsp_vocab = body_vocab.replace(live, nbsp);
        assert!(nbsp_vocab.contains(nbsp) && modal_signature(&nbsp_vocab).is_some());
        assert_eq!(reinject(&nbsp_vocab), Verdict::Ready { evidence: Evidence::MarkerTail },
                   "라이브 NBSP 그리드 아래 본문 어휘가 창을 영구 보류로 접었다");
        let nbsp_fore = format!("{}\n{}", nbsp.trim_end_matches('\n'), fixtures::LIVE_PERMISSION_PROMPT);
        assert!(held_as(&reinject(&nbsp_fore), MODAL_UNKNOWN_ID));
        let nbsp_draft = nbsp_vocab.replace("❯\u{a0}\n", "❯\u{a0}작업 이어서\n");
        assert!(held_as(&reinject(&nbsp_draft), MODAL_UNKNOWN_ID));
        // 보조 술어 — 괘선·번호 행 판정의 경계.
        assert!(is_rule_line("────────"), "8연속 괘선");
        assert!(!is_rule_line("───────"), "7연속은 괘선이 아니다(TUI_FRAME_RUN_MIN 파리티)");
        assert!(!is_rule_line("── 제목 ──────────"), "문자가 섞인 줄은 괘선이 아니다");
        assert!(is_rule_line("  ────────────  "));
        assert!(is_numbered_item_row("  2. Yes, and don't ask again"));
        assert!(is_numbered_item_row("12."));
        assert!(!is_numbered_item_row("  2.5 hours"));
        assert!(!is_numbered_item_row("  123. 너무 큰 번호"));
        assert!(!is_numbered_item_row("  Opus 5 · CTX 35% · 5h 20% · 7d 33%"));
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
        // ★(0.14.31 · 리뷰 R1) 두 표를 이어서 돈다 — 위 `modal_signature` 기대표와 같은 이유.
        for &(id, screen) in fixtures::NON_GATE_SCREENS
            .iter()
            .chain(fixtures::MEASURED_NON_CORPUS_MODALS)
        {
            let v = judge(&boot_all_open(screen, &gates));
            match id {
                // ★(0.14.31 · WP-1 H-2) 실측 벤더 모달(2.1.261 커스텀 API 키 확인창)은 코퍼스 밖이지만
                //   `Enter to confirm` ∧ `Esc to cancel` 푸터가 전경이라 H-1 의 모달 축이 보류한다 —
                //   그 보류가 곧 "코퍼스에 없는 새 관문에 Return 이 나가지 않는다" 의 실측 증거다.
                "live-permission-prompt" | "audit-log-line" | "custom-api-key-modal-2.1.261" => assert!(
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
