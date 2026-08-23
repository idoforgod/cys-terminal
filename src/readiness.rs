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
    /// 【새 판별자의 방향】 `bare_shell` 은 "꼬리가 셸 프롬프트 **이면서** 화면에 TUI 렌더
    /// 증거가 없다" 로 계산된다(구현·근거는 CLI 의 `screen_is_bare_shell_on`). 이것은 종전
    /// 축보다 **참이 덜 되므로** 밸브의 재현율은 오직 올라간다. 남는 미탐(프레임을 그린 뒤
    /// 즉사한 경우)의 귀결은 마커 축이 따로 막고, 최악이어도 U-11 의 보류(좌석 보존)다.
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
        None => match evidence {
            Some(evidence) => Verdict::Ready { evidence },
            None => Verdict::NotYet,
        },
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
    first_run_gates::identify(o.gates, o.screen)
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
            //   수리는 **밸브 전용 술어를 분리**하는 것이다(`Observed::bare_shell` — "꼬리가
            //   프롬프트 **이면서** 화면에 TUI 렌더 증거가 없다"). 축의 비용 부호가 반대이기
            //   때문이다: ready 판정은 정밀도가, 밸브는 **재현율**이 필요하다. 새 술어는 종전
            //   축보다 참이 **덜** 되므로 밸브 재현율은 오직 올라가고, 오살 방향으로는 열리지
            //   않는다(맨 셸이면 여전히 닫힌다).
            if o.agent_alive == Some(true) && bare_shell_ok {
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
        \x20 Welcome back 미래학자!   Opus 5 (1M context) · Claude Max\n\
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
        // ★P3-0: 이 화면의 꼬리는 `❯` 라 '끝문자 4종' 술어로는 셸 프롬프트로 읽힌다. 그런데
        //   화면은 명백히 TUI 를 그리고 있으므로(`─ Claude Code ─`) 맨 셸이 아니다 →
        //   **밸브가 열려야 한다**. 종전에는 이 자리에서 밸브가 상시 차단돼 마커 델타로만
        //   통과했다(밸브가 사문화된 상태). 두 경로 모두 ready 라는 사실도 함께 박제한다.
        let mut o = obs(HEALTHY_BANNER, HEALTHY_BANNER, &gates);
        o.agent_alive = Some(true);
        o.tail_is_shell_prompt = Some(true);
        o.bare_shell = Some(false);
        assert_eq!(
            judge(&o),
            Verdict::Ready {
                evidence: Evidence::Valve
            },
            "살아있는 TUI(꼬리 `❯`)에서 밸브가 닫혔다 — 영구 오부정 차단 장치가 사문화된다"
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
}
