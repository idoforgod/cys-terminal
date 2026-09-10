//! 주입·제출 관문 가드 (U-14) + 폴더신뢰 Return 정책 (U-15) — **키를 보내도 되는가**의 판정처.
//!
//! ## 이 단위가 닫는 킬체인 (2026-08-23 실측 · macOS + Windows 실기 · claude 2.1.241)
//!
//! ```text
//!  폴더신뢰 창(기본 포커스 = Yes, I trust this folder)  ← Return 안전
//!      ↓ 통과하면 화면에 확인 에코 `Yes, I trust this folder ✔` 가 남는다
//!  면책 창(기본 포커스 = No, exit)                      ← Return 이 곧 rc 1 종료
//! ```
//!
//! 이 연쇄를 실제로 밟은 방아쇠는 **두 개**였다.
//!
//! **① 디렉티브 제출 Return** — `cys.rs inject_text` 는 bracketed paste 로 본문을 넣고
//!    800ms 뒤 `send_key Return{authoritative:true}` 를 **무조건** 보낸다. ready 가 관문 화면에서
//!    잘못 선언되면(U-13 이 그 판정을 고쳤다) 그 Return 이 면책 창의 `No, exit` 를 누른다.
//!    ★그리고 `inject_text` 를 부르는 경로는 하나가 아니다 — 부트 주입 외에 `[RECOVER]`·
//!    `[DRAIN]`·cycle 재주입·pack-update 재주입·복원 디렉티브·각성 확인 핑까지 **여러 갈래**이고,
//!    그중 대부분은 화면을 전혀 보지 않는다. 그래서 가드를 각 호출부가 아니라 **`inject_text`
//!    안쪽 한 곳**에 둔다: 한 번 걸면 그 갈래 전부가 동시에 덮인다.
//!
//! **② 폴더신뢰 자동확인의 2발째 Return** — 종전 감지 needle 은 하드코딩 `trustthisfolder`
//!    였고, 그것은 통과 직후 화면에 남는 **확인 에코**(`Yes, I trust this folder ✔`)에
//!    **재매칭**된다. 거기에 재전송 상한 2발 + `persisted` 조건이 겹쳐 2발째가 나갔고,
//!    그때 화면은 이미 면책 창이었다. (U-15 가 needle 을 질문형 문면으로 좁히고 전송을 1발로
//!    줄인다.)
//!
//! ## 판정식
//!
//! ```text
//! send = (생애 창이 닫혔다) ∨ (화면에 관문 없음) ∨ (그 관문이 이 키가 겨냥한 바로 그 관문)
//! ```
//!
//! 세 항 모두 **호출부가 관측해 넘긴 값**으로만 계산한다. 이 모듈은 판정 중에 파일·시계·
//! 전역·env·RPC 를 읽지 않는다 — [`Observed`] 가 판정 입력의 전량이다(숨은 입력 금지).
//!
//! ## ★생애 창 상한 — 치명위험 ①(폭주·오탐) 차단
//!
//! 스캔은 **첫 각성 ack(`awakened_at` 래치) 이전에만** 활성이다. 그렇게 하지 않으면 작업 중인
//! 노드가 화면에 관문 문면을 출력하는 순간(예: 이 캠페인의 감사 문서나 `src/first_run_gates.rs`
//! 를 `cat` 할 때) 그 노드로 가는 모든 주입이 영구 거부된다.
//! 관측이 **없을 때**(`None`)의 접기 방향은 '창 닫힘 = 종전대로 보낸다' 이다 —
//!   · 이 가드는 종전에 없던 **추가** 안전망이므로, 관측 불능의 귀결이 '새 차단' 이면
//!     구 데몬·조회 실패라는 무관한 사유로 오늘 되던 일이 안 되게 된다.
//!   · 반대로 접어도 잃는 것은 없다: 창을 못 재는 상황은 정의상 **첫 부트 창 밖**이고
//!     (부트 경로는 자기가 창 안임을 알기 때문에 `Some(false)` 를 상수로 넘긴다),
//!     창 밖의 오탐이야말로 이 축이 막으려는 사고다.
//!
//! ## 걸렸을 때의 귀결은 close 가 아니다
//!
//! 가드에 걸린 주입의 귀결은 **보류**다 — 부트 경로에서는 U-11 의
//! `BootVerdict::GatePending`(좌석 보존 · close 0 · kill 0 · 주입 0 · 처방 문안)으로 흐르고,
//! 그 밖의 경로에서는 주입만 거부되고 좌석은 그대로 남는다. 이 저장소가 가장 비싸게 치른
//! 실수가 "살아 있는 것을 닫은 것" 이고, 그래서 **오살이 오탐보다 훨씬 위험하다**.
//!
//! ## 롤백 스위치 (env 2지점 — 축마다 하나)
//!
//! | env | 값 | 되돌아가는 축 |
//! |---|---|---|
//! | `CYS_INJECT_GATE_GUARD` | `0` | **U-14** 주입 가드를 관측 전용으로 강등(관문을 봐도 종전대로 보낸다) |
//! | `CYS_TRUST_RETURN_V1` | `1` | **U-15** 폴더신뢰 정책을 종전으로(하드코딩 needle 감지 + 재전송 상한 2발) |
//! | `CYS_GATE_VERSION_PIN` | `0` | **H2-B** 확인 경계의 **버전 대조**를 관측 전용으로(= 버전을 보지 않던 종전) |
//!
//! 셋 다 **엄격 비교**다(`== Some("0")` / `== Some("1")`) — 형제 게이트
//! (`CYS_GATE_PENDING_CLOSE`·`CYS_READINESS_V1`)와 같은 규율로, 오타 하나로 안전장치가 조용히
//! 뒤집히는 것을 막는다. env 를 읽는 지점은 축마다 **함수 하나**뿐이고 판정은 순수 코어에 있다.
//! 앞 두 스위치를 다 켜면 이 단위 착지 이전의 **주입 정책**으로 복귀한다(세 번째는 아래 참조).
//!
//! ### ★롤백 예외 — 관문 **확인 허가**([`confirm_allowed`])의 **증거 벨트**는 어느 노브로도 열리지 않는다
//!
//! (0.14.31 · 리뷰 R2·R4 · codex) 위 두 노브와 `CYS_READINESS_V1` 은 전부 **보류를 푸는** 방향의
//! 스위치다(관문을 봐도 보낸다 · 모달 축을 끈다 · 재전송을 되살린다). 그러나 자동확인이 쏘는 것은
//! 본문이 아니라 **키 한 발**이고, 그 한 발의 오판 귀결은 `No, exit` 선택 = 좌석 rc 1 종료(비가역)다.
//! 그래서 "커서가 액션 라벨 위에 있다" 는 **양성 증거**는 조여지는 방향의 벨트이고, 어떤 롤백도 그것을
//! 면제하지 않는다(CONTRACTS B-7). 되돌릴 수 있는 것은 **몇 발 보내는가**(U-15 `CYS_TRUST_RETURN_V1`)
//! 까지이며, **무엇을 보고 보내는가**는 아니다. 이 예외를 문서에 적어 두는 이유는, 노브를 켠 사람이
//! "완전 복귀" 를 기대하다가 자동확인이 안 열리는 것을 결함으로 오독하지 않게 하기 위해서다.
//!
//! ### ★★그 예외의 **경계** — 버전 축은 노브를 갖는다(0.14.31 · 수렴 R2 · reviewer-claude major)
//!
//! 위 예외는 **양성 증거 벨트**(커서가 액션 라벨 위 · 정본 사람 1회 관문 · 코퍼스 식별)에 대한
//! 것이다. 그 벨트들은 **위험이 관측될 때만** 닫히므로 정상 좌석에서는 아무것도 막지 않는다.
//! 버전 축은 성질이 다르다 — 그 보류는 **기본 상태**다: `MEASURED_ON` 을 재실측하지 않은 모든
//! 기계에서 항상 참이고(이 저장소의 개발 기계가 지금 그렇다: 라이브 2.1.263 대 실측본 2.1.241),
//! 그래서 "노브 없는 벨트" 로 두면 **전 좌석이 매 부트마다** 관문에 선다.
//!
//! 그 상태에서 운영자가 쥘 손잡이가 마스터(`CYS_BOOT_GATES=0`) 하나뿐이면 BLOCK-4 형상이
//! 그대로 재현된다: 마스터는 `gate_pending_close` 를 켜므로(보류 → 즉시 close) **보류는 사망이
//! 되는데 버전 축만 엄격하게 남아** Return 이 끝내 나가지 않는다 → readiness 는 legacy 라
//! 관문 화면을 Ready 라 하고 → 주입 가드는 강등돼 디렉티브가 신뢰 모달로 들어가고 → 타임아웃이
//! 좌석을 close 한다. **엄격화와 보류는 한 몸**(BLOCK-4 불변식)이라, 보류가 사망으로 강등된
//! 조합에서 엄격하게 남을 권리는 이 축에도 없다.
//!
//! 그래서 이 축은 `crate::GateAxes::version_pin_legacy` 로 **마스터에 접히고**, 축 단독 노브
//! (`CYS_GATE_VERSION_PIN=0`)도 함께 둔다 — 드리프트가 기본인 기계의 운영자가 마스터(전 축 종전
//! + close 강등)를 누르지 않고 **이 축만** 끌 수 있어야 한다(그것이 사고를 줄이는 방향이다).

use crate::first_run_gates::{self, Gate, Passability};

// ═══════════════════════════════════════════════════════════════════════════
// 롤백 스위치 (env 1지점 × 2축)
// ═══════════════════════════════════════════════════════════════════════════

/// ★U-14 롤백 스위치의 env 이름. `0` → 가드를 **관측 전용**으로 강등한다.
pub const ENV_GUARD_OFF: &str = "CYS_INJECT_GATE_GUARD";

/// ★U-15 롤백 스위치의 env 이름. `1` → 폴더신뢰 Return 정책을 **종전**으로 되돌린다.
pub const ENV_TRUST_V1: &str = "CYS_TRUST_RETURN_V1";

/// 코퍼스에서 폴더신뢰 관문을 가리키는 **식별자**(문면이 아니다 — 문면의 SOT 는
/// `first_run_gates` 하나이고 여기에는 사본을 두지 않는다).
pub const GATE_FOLDER_TRUST: &str = "folder-trust";

/// U-14 가드가 꺼져 있는가. **env 를 읽는 유일한 지점**.
///
/// ★(BLOCK-3/BLOCK-4 · 2026-08-24) 자기 축의 노브와 **상위 접기값**(마스터 스위치 ∨ 보류 장치
/// 꺼짐)을 OR 한다 — 근거 전문은 `crate::gate_axes_from` 의 doc. 특히 보류가 close 로 강등된
/// 상태에서 이 가드만 살아 있으면, 가드의 `Hold` 가 그대로 `LaunchFailed` → `surface.close` 로
/// 흘러 **문서화된 스위치 하나가 전 pane 을 죽인다**(BLOCK-4 재난④의 두 번째 경로).
pub fn guard_off() -> bool {
    guard_off_from(std::env::var(ENV_GUARD_OFF).ok().as_deref()) || crate::gate_axes_forced_legacy()
}

/// 위 판정의 순수 절반(테스트가 env 를 건드리지 않게 분리).
pub fn guard_off_from(raw: Option<&str>) -> bool {
    raw == Some("0")
}

/// ★(0.14.31 · 수렴 R2) 버전 축 롤백 스위치의 env 이름. `0` → 확인 경계의 **버전 대조**를
/// 관측 전용으로 내린다(= 이 축이 태어나기 전과 같이 버전을 보지 않는다).
pub const ENV_VERSION_PIN: &str = "CYS_GATE_VERSION_PIN";

/// U-15 폴더신뢰 정책이 종전(V1)인가. **env 를 읽는 유일한 지점**.
///
/// ★(BLOCK-3 · 2026-08-24) 마스터 스위치·보류 접기값과 OR 한다 — 마스터 하나로 이 캠페인의
/// 축이 **전부** 종전으로 돌아가야 하고, 신뢰 정책도 그 축 중 하나다.
pub fn trust_v1() -> bool {
    trust_v1_from(std::env::var(ENV_TRUST_V1).ok().as_deref()) || crate::gate_axes_forced_legacy()
}

/// 위 판정의 순수 절반.
pub fn trust_v1_from(raw: Option<&str>) -> bool {
    raw == Some("1")
}

/// 확인 경계의 **버전 축**이 종전(= 버전을 보지 않음)인가. **env 를 읽는 유일한 지점**.
///
/// ★(0.14.31 · 수렴 R2 · reviewer-claude major · codex blocking 재기) 마스터·보류 접기값과
/// OR 한다. 근거 전문은 이 모듈 doc 의 「롤백 예외의 경계」와 `crate::gate_axes_from` 의
/// BLOCK-4 불변식 — 보류가 close 로 강등된 조합에서 이 축만 엄격하면 "Return 도 안 나가고
/// 좌석은 닫히는" 상태가 되고, 그것은 이 축이 없던 때보다 나쁘다.
pub fn version_pin_legacy() -> bool {
    version_pin_legacy_from(std::env::var(ENV_VERSION_PIN).ok().as_deref())
        || crate::gate_axes_forced_legacy()
}

/// 위 판정의 순수 절반(형제 노브와 같은 엄격 비교 — `"0"` 만 끈다).
pub fn version_pin_legacy_from(raw: Option<&str>) -> bool {
    raw == Some("0")
}

// ═══════════════════════════════════════════════════════════════════════════
// U-14 · 주입·제출 관문 가드
// ═══════════════════════════════════════════════════════════════════════════

/// 화면에서 걸린 관문의 **요약**(진단·처방 문안용 · 판정 재료 아님).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateHit {
    pub id: String,
    pub title: String,
    /// 사람이 1회 해야 통과한다(로그인·OAuth) — 처방 문안이 갈린다.
    pub human_only: bool,
}

/// 가드 판정 입력의 **전량**. 이 구조체 밖의 사실은 판정에 쓰이지 않는다.
#[derive(Debug, Clone)]
pub struct Observed<'a> {
    /// 지금 사람이 보는 화면(vt100 그리드). **델타가 아니라 화면**이다 — 관문은 떠 있는 동안
    /// 계속 입력을 기다리는 상태이지 한 번 지나가는 출력이 아니다(U-13 과 같은 근거).
    pub screen: &'a str,
    /// U-12 관문 코퍼스. 빈 슬라이스 = 관문 축 없음 = 종전 동작.
    pub gates: &'a [Gate],
    /// 첫 각성 ack 를 이미 받았는가(생애 창). `Some(false)` = 창 열림(스캔) ·
    /// `Some(true)`/`None` = 창 닫힘·미관측(스캔 안 함 — 위 doc 의 접기 방향 참조).
    pub awakened: Option<bool>,
    /// 롤백 스위치 값(호출부가 [`guard_off`] 로 1회 읽어 넘긴다).
    pub guard_off: bool,
    /// ★(0.14.31 · 리뷰 R2) readiness 롤백 노브 값(`CYS_READINESS_V1=1` — 호출부가 [`crate::readiness::legacy_v1`]
    /// 로 읽어 넘긴다). WP-1 이 더한 **코퍼스 밖 모달 폴백**(`unknown-modal` 보류)은 readiness 의 모달 축과
    /// 같은 축이라 같은 노브로 꺼진다 — "종전 판정 복귀" 가 반쪽(부트 폴링은 종전인데 주입 가드는 신판)이 되지
    /// 않게. 종전부터 있던 코퍼스 가드(U-14 축 · [`guard_off`])와 커서-종료 벨트(조여지는 방향만)는 이 값과
    /// 무관하다. 마스터 `CYS_BOOT_GATES=0` 은 두 값 모두 켠다(전 축 종전).
    pub readiness_legacy: bool,
    /// ★(0.14.31 · 독립 재유도 H2-B · 수렴 R2) **이 좌석의 이번 기동에서 관측된 claude 버전
    /// 전량**(누적 래치 · [`latch_seat_versions`]).
    ///
    /// 생산자는 부트 루프(`cys.rs boot_agent_on_surface`)이고, 재료는 그 부트의 누적 델타와
    /// 화면에서 뽑은 배너다([`first_run_gates::banner_versions`]). 화면의 배너는 관문이
    /// 그려지면서 밀려나지만 이 래치에서는 밀려나지 않는다 — 그 **증거 소멸**이 정확히
    /// 드리프트 거부를 다음 틱에 무효로 만드는 경로였다(codex 설계 검토 3).
    ///
    /// ★**하나가 아니라 전량**인 이유(수렴 R2 · codex major): 값을 하나만 들면 "먼저 잡힌
    ///   일치" 가 **나중에 관측한 불일치를 영구히 덮는다** — 첫 틱에 실측본과 같은 배너(이전
    ///   좌석의 잔상이거나 업그레이드 전 출력)를 잡으면, 다음 틱 델타에 진짜 버전이 실려도
    ///   sticky 라 갱신되지 않고 확인 화면에서 배너가 밀려나는 순간 미상으로 열린다. 래치는
    ///   **단조 증가하는 합집합**이어야 시간축의 증거가 보존된다.
    ///
    /// 빈 슬라이스 = 이 부트에서 배너를 아직 못 봤다(= 버전 미상). 미상은 오늘 **통과**한다
    /// ([`first_run_gates::ACTION_POLICY_ENFORCEMENT`] doc — 별도 결정).
    ///
    /// ★래치와 지금 화면의 배너는 **합집합**으로 쓴다(하나라도 실측본과 다르면 보류).
    pub cli_versions: &'a [String],
    /// ★(0.14.31 · 수렴 R2) 버전 축이 **종전(관측 전용)** 인가 — 호출부가
    /// [`version_pin_legacy`] 로 1회 읽어 넘긴다(마스터·보류 접기값 포함).
    ///
    /// 참이면 [`confirm_denied`] 는 버전을 **한 번도 보지 않는다**(이 축이 태어나기 전과 같다).
    /// 왜 이 축만 노브를 갖는지는 모듈 doc 「롤백 예외의 경계」 — 요약하면 이 축의 보류는
    /// **기본 상태**이고, 보류가 close 로 강등된 조합(BLOCK-4)에서 엄격하게 남으면 좌석이
    /// "Return 도 못 받고 close 되는" 상태가 되기 때문이다.
    pub version_pin_legacy: bool,
}

/// 가드의 결론.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// 보내도 된다.
    Send,
    /// 관문이 떠 있다 — **보내지 않는다**(귀결은 보류이지 close 가 아니다).
    Hold(GateHit),
    /// 롤백 스위치가 켜져 있다 — 관문을 봤지만 종전대로 보낸다(관측·경고만).
    /// ★롤백을 '아무것도 안 하기' 가 아니라 '관측은 남기기' 로 둔 이유: 되돌린 뒤에도
    ///   그 기계에서 가드가 무엇을 봤는지가 로그에 남아야 다음 판단의 재료가 된다.
    SendObserved(GateHit),
}

impl Decision {
    /// 이 판정이 전송을 막는가.
    pub fn blocks(&self) -> bool {
        matches!(self, Decision::Hold(_))
    }
    /// 관문을 관측했는가(막았든 아니든).
    pub fn hit(&self) -> Option<&GateHit> {
        match self {
            Decision::Hold(h) | Decision::SendObserved(h) => Some(h),
            Decision::Send => None,
        }
    }
}

/// 주입·제출 가드. 어떤 관문도 통과 대상이 아니다(= 관문이 보이면 무조건 보류).
///
/// ★(0.14.31 · 성찰 R7 · major) **allow 구멍은 없다.** 종전 `decide_allowing(o, Some(id))` 는 지목한
///   관문 하나를 통과 대상으로 보는 일반형이었고, R4 가 자동확인의 생산자를 [`confirm_denied`] 로
///   옮긴 뒤 프로덕션 호출자가 0 이 됐는데도 `pub` 으로 남아 doc 이 그것을 자동확인 벨트로 지목했다.
///   그 구멍은 확인 경계보다 벨트가 넷 적었다(⓪ 사람 1회 화면 봉인 · ③ Return 한 발 선언 ·
///   ⑤ 버전 드리프트 · 미식별 + `readiness_legacy` 면 곧장 Send). 그 위에 새 자동통과 경로를 세우면
///   미실측 버전 관문에 Return 이 나가고 기본 포커스는 `No, exit`(좌석 rc 1)이다. 그래서 인자를
///   지웠다 — 이 술어는 **확인 벨트가 아니다**(주입 허가는 '모르면 보낸다' 로 접히고, 확인 허가는
///   '모르면 안 보낸다' 로 접힌다 — 두 질문의 방향이 반대라 한 API 에 둘 수 없다). 확인 허가는
///   [`confirm_denied`] 하나가 소유한다. 구 배선의 진리표는 검체의 계측 타당성 대조군
///   (`tests::legacy_decide_allowing`)에만 남아 있다.
pub fn decide(o: &Observed) -> Decision {
    // ★생애 창 상한 — 창이 닫혔거나(각성 완료) 재지 못했으면 스캔 자체를 하지 않는다.
    //   여기서 일찍 반환하는 것이 비용 방어이기도 하다(호출부가 화면 RPC 를 아예 생략한다).
    if o.awakened != Some(false) {
        return Decision::Send;
    }
    // ★(0.14.31 · WP-1 H-1) 모달 어휘는 **readiness 와 같은 함수**로 본다(판정 분리 금지 — 두 판정기가
    //   갈리면 "부트 폴링은 보류인데 주입 가드는 통과" 라는 반쪽 그물이 된다).
    let modal = crate::readiness::modal_signature(o.screen);
    let hit = match first_run_gates::identify(o.gates, o.screen) {
        // 관문이 떠 있다 = 보류. 커서 위치·액션 라벨은 여기서 보지 않는다 — 그 벨트들은 **확인 경계**
        // ([`confirm_denied`] ②·④)의 것이고, 주입 가드에는 통과시킬 관문이 없다(성찰 R7).
        Some(g) => GateHit {
            id: g.id.clone(),
            title: g.title.clone(),
            human_only: g.passability == Passability::HumanOnly,
        },
        // 코퍼스가 모르는 화면 — 모달 어휘가 있으면 `unknown-modal` 로 보류한다(잘린 관문·새 관문).
        // ★(리뷰 R2 · codex minor) 롤백 정합: 이 폴백은 WP-1 이 readiness 에 더한 모달 축의 **주입 가드 쪽 절반**
        //   이다. `CYS_READINESS_V1=1` 이 readiness 의 모달 축(`modal_on_screen`)을 끄면 여기도 함께 종전(Send)이어야
        //   "종전 판정 복귀" 가 반쪽이 아니다. 종전부터 있던 코퍼스 가드(위 Some 분기)는 U-14 축(`guard_off`)이
        //   소유하고, 커서-종료 벨트는 조여지는 방향(보류)만이라 이 노브로 열지 않는다(CONTRACTS B-7 · H-2 재핀까지).
        None => {
            if o.readiness_legacy {
                return Decision::Send;
            }
            match modal {
                Some(m) => GateHit {
                    id: crate::readiness::MODAL_UNKNOWN_ID.to_string(),
                    title: m.title(),
                    human_only: false,
                },
                None => return Decision::Send,
            }
        }
    };
    if o.guard_off {
        Decision::SendObserved(hit)
    } else {
        Decision::Hold(hit)
    }
}

/// 이 관문의 **액션 라벨 위에 선택 커서가 있는가** — 확인 경계([`confirm_denied`] ④)가 요구하는 양성 증거.
///
/// 【왜 라벨 전량이 아니라 이 관문의 액션 라벨인가(codex 리뷰 R3)】 `Not now`·`Yes, I accept` 를 알아본 것은
/// "Return 이 폴더신뢰를 승인한다" 는 증거가 아니다. 확인이 겨냥한 관문의 **통과 동작이 지목한 그 라벨**만
/// 증거로 인정한다(문면 SOT 는 코퍼스 하나 · 사본 0).
///
/// 【액션 선언이 없으면】 `false`(= 확인이 닫힌다). 통과 동작이 선언되지 않은 관문을 기계가 통과시킬 근거는
/// 없고, 접기 방향은 보류다(좌석 보존 · 키 0).
///
/// 【모호하면 닫힌다(리뷰 R3b · codex)】 술어는 액션 라벨 위 커서 **하나**로 만족하지 않는다 — 같은 화면에
/// 해소되지 않은 다른 선택 커서(잘린 종료 행 등)가 있으면 어느 쪽이 실제 선택인지 모르므로 거짓이다
/// (`readiness::cursor_resolves_to_label` doc 의 경쟁 규칙).
fn action_label_selected(gate: &Gate, screen: &str) -> bool {
    // 활성 선택 블록의 시작은 **그 관문의 질문 문면**이 정한다(리뷰 R4) — 질문보다 앞의 커서(스크롤백
    // 셸 프롬프트 `❯ claude` · p10k)는 이 관문의 선택지가 아니므로 모호로 세지 않는다.
    let anchors: Vec<&str> = gate.needles.iter().map(|s| s.as_str()).collect();
    gate.action
        .as_ref()
        .is_some_and(|a| crate::readiness::cursor_resolves_to_label(screen, &a.label, &anchors))
}

/// ★(0.14.31 · 리뷰 R4 · codex blocking) **관문 확인 키**를 지금 화면에 보내도 되는가 —
/// 주입 허가([`decide`])와 **다른 술어**다(성찰 R7 이후 주입 쪽에는 통과 대상 관문이 아예 없다).
///
/// 【왜 나누는가】 자동확인은 "텍스트를 보내도 되는가" 가 아니라 "이 화면의 **선택을 확정**해도 되는가" 를
/// 묻는다. 두 질문의 접기 방향이 반대다:
///   · 주입 허가는 **모르면 보낸다**(관문이 안 보이면 종전대로 — 그러지 않으면 정상 좌석이 영영 못 받는다).
///   · 확인 허가는 **모르면 안 보낸다**(Return 한 발이 `No, exit` 을 눌러 좌석을 죽인다 · 비가역).
/// R3 까지는 자동확인이 `decide_allowing(...).blocks()` 의 **부정**을 썼기 때문에, 코퍼스가 화면을 식별하지
/// 못하고 모달 어휘도 못 본 잘린 렌더(부트 델타에는 질문이 있는데 화면은 `❯ No, exi` 뿐)에서 `Send` 가 나왔고
/// 그 Return 이 부분 렌더된 종료 선택지를 눌렀다(codex R4 blocking). 그래서 확인은 **양성 증거만** 본다.
///
/// 【참이 되는 조건 — 전부 AND】 ⓪ **코드 정본**의 사람 1회 관문(로그인·OAuth)이 지금 화면에 서지
/// **않는다**(코퍼스 선언과 무관한 화면 층위 봉인 · 0.14.31 리뷰 R2) · ① 코퍼스가 지금 화면을 **바로 그
/// id** 로 식별한다(미식별=거짓) · ② 커서가 종료 라벨 위가 **아니다** · ③ 코퍼스에 통과 동작이 선언돼
/// 있고 그 **키 시퀀스가 Return 한 발**이다(`down_presses() == Some(0)` — 이 조립은 Down 을 보내지
/// 않는다 · 0.14.31 리뷰 R2) · ④ 커서가 그 관문의 **액션 라벨 전문** 위에 있고 활성 선택 블록에
/// 경쟁 커서가 없다([`action_label_selected`]).
///
/// ★⓪·③ 은 **조이는 항**이다. ⓪ 은 봉투가 별칭 id·중복 id 로 코퍼스 층위의 바닥을 빠져나가도 화면
///   층위에서 닫고(codex 설계 검토 ①), ③ 은 운영자가 `default_index: null` 로 선언한 무지가 실제로
///   키를 막게 한다(종전엔 보고서만 '보류' 라 인쇄하고 Return 은 그대로 나갔다 — codex major).
///   ★그럼에도 **버전 핀([`first_run_gates::action_policy`])은 여기에 배선돼 있지 않다** — 그 판정은
///   `Allowed{down}` 이 기술하는 **다발 전송**을 전제하는데 이 조립은 Return 1발이고, 좌석이 실제로
///   실행한 바이너리의 버전을 이 자리에서 알 방법이 아직 없다(PATH 조회는 그 바이너리가 아니다).
///   그 배선은 4조건(첫 Down 전 버전 일치 · 탐색 전용 허가 · 전송 후 재관측 · 멱등 래치)과 함께
///   다음 회차로 남아 있다(노트 §4-2).
///
/// 【롤백 노브로 열리지 않는다】 `CYS_READINESS_V1=1`(모달 축)·`CYS_INJECT_GATE_GUARD=0`(U-14 축)은
/// **보류를 푸는** 노브다. 이 술어는 보류를 푸는 것이 아니라 **키를 쏘는 것을 허가**한다 — 조여지는 방향의
/// 벨트이므로 어느 노브로도 열지 않는다(CONTRACTS B-7 · R2 에서 같은 결정을 이미 했다: 커서-종료 벨트는
/// `guard_off` 로 열리지 않는다). 자동확인을 종전 정책으로 되돌리는 노브는 U-15 의 `CYS_TRUST_RETURN_V1`
/// 이고, 그것은 **재전송 횟수**를 되돌릴 뿐 이 벨트를 열지 않는다(`trust_send` doc 의 같은 규율).
///
/// 【실패 방향】 거짓의 귀결은 `cys boot` 의 "관문 보류 · 사람 1회 조치"(가역). 참의 오판 귀결은 좌석 rc 1
/// 종료(비가역). 그래서 모르면 거짓이다.
pub fn confirm_allowed(o: &Observed, gate_id: &str) -> bool {
    confirm_denied(o, gate_id).is_none()
}

/// 확인 허가가 **왜** 닫혔는가 — 진단 전용(판정 재료가 아니다 · 판정은 [`confirm_allowed`] 하나).
///
/// ★(0.14.31 · 리뷰 R5 · claude 적대) 왜 사유가 필요한가: R4 가 생산자를 `!confirm_allowed(..)` 로 바꾸면서
///   **첫 발**(미식별·모호·커서 종료 위)이 흔한 경우가 됐는데 그 분기는 stderr 를 한 줄도 내지 않았다.
///   운영자와 릴리스 게이트 실측자는 그때 "감지 실패(관문을 못 봤다)" 와 "확인 거부(봤지만 안 쏜다)" 를
///   가르지 못한다 — 두 상태의 처방이 다르다(전자는 감지 폭, 후자는 렌더 실측). 사유는 **문안이 아니라
///   타입**으로 낸다(하류가 문자열을 파싱하지 않는다).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmDenied {
    /// 코퍼스가 지금 화면을 **어떤 관문으로도** 식별하지 못한다(잘린 렌더 · 새 관문 · 이미 지나갔다).
    Unidentified,
    /// 다른 관문이 떠 있다 — 구멍은 id 하나다(신뢰창인 줄 알고 눌렀는데 면책창이던 실측 킬체인).
    OtherGate(String),
    /// 커서가 **종료 선택지** 위다 — 그 Return 은 통과가 아니라 종료다(좌석 사망 · 비가역).
    CursorOnExit,
    /// ★(0.14.31 · 리뷰 R2) 지금 화면이 **코드 정본이 사람 1회로 실측한 관문**이다 —
    /// 코퍼스의 선언 순서·id·통과 가능성과 **무관하게** 닫는다(화면 층위 봉인).
    HumanOnlyScreen(String),
    /// 코퍼스에 그 관문의 **통과 동작(action)** 선언이 없다 — 기계가 통과시킬 근거가 없다.
    NoAction,
    /// ★(0.14.31 · 리뷰 R2 · codex major) 코퍼스가 선언한 통과 키 시퀀스가 **Return 한 발이 아니다**.
    /// `None` = 산출 불가(기본 포커스 미상 · 목표가 기본 포커스보다 위) · `Some(n>0)` = 아래키 n회 필요.
    /// 이 조립은 Return 만 보낸다 — 선언이 다른 것을 요구하면 둘은 합의하지 못했다(보류).
    SequenceNotBareReturn { down: Option<u8> },
    /// 커서가 액션 라벨 **전문** 위가 아니거나, 활성 선택 블록에 **경쟁 커서**가 있다(모호).
    LabelUnresolved,
    /// ★(0.14.31 · 독립 재유도 H2-B) 좌석이 밝힌 claude 버전이 이 관문을 **실측한 버전과 다르다**.
    /// 선언된 기본 포커스·항목 순서가 그 버전에서 참이라는 근거가 없으므로 Return 을 보내지 않는다
    /// (정본 §4 WP-1 H-2 "미실측은 보류" · §7 봉인표 ④).
    VersionDrift {
        measured_on: String,
        detected: String,
    },
}

impl ConfirmDenied {
    /// 사람이 읽는 한 마디(stderr 진단 전용).
    pub fn label(&self) -> String {
        match self {
            ConfirmDenied::Unidentified => {
                "지금 화면이 코퍼스의 어떤 관문으로도 식별되지 않는다(잘린 렌더·새 관문·이미 지나감)".into()
            }
            ConfirmDenied::OtherGate(id) => format!("지금 화면은 다른 관문이다(id={id})"),
            ConfirmDenied::CursorOnExit => "선택 커서가 종료 선택지 위다(그 Return 은 좌석 종료)".into(),
            ConfirmDenied::HumanOnlyScreen(id) => format!(
                "지금 화면은 코드 정본이 **사람 1회**로 실측한 관문이다(id={id}) — 코퍼스가 무엇을 \
                 선언했든 기계가 넘지 않는다"
            ),
            ConfirmDenied::NoAction => "코퍼스에 이 관문의 통과 동작(action)이 선언돼 있지 않다".into(),
            ConfirmDenied::SequenceNotBareReturn { down } => match down {
                None => "코퍼스가 이 관문의 아래키 수를 산출하지 못한다(기본 포커스 미상 또는 역방향) \
                         — 이 조립은 Return 만 보내므로 보류한다"
                    .into(),
                Some(n) => format!(
                    "코퍼스가 선언한 통과 시퀀스는 아래키 {n}회 + Return 인데 이 조립은 Return 만 \
                     보낸다 — 합의하지 못한 시퀀스로 키를 쏘지 않는다"
                ),
            },
            ConfirmDenied::LabelUnresolved => {
                "커서가 액션 라벨 전문 위가 아니거나 활성 선택 블록에 경쟁 커서가 있다(모호)".into()
            }
            // ★(0.14.31 · 성찰 R4 · major) 종전 ②는 "실측한 뒤 measured_on 을 갱신하라" 였다.
            //   그런데 `detected` 가 **접힌 배너의 잘린 판독**(`2.1`)일 수 있었고, 그 처방을
            //   그대로 따르면 `measured_on:"2.1"` 이 되어 진짜 2.1.241 좌석 전량이 드리프트로
            //   뒤집힌다. 처방은 화면 판독값을 **베끼라**가 아니라 **실측하라**여야 한다.
            ConfirmDenied::VersionDrift { measured_on, detected } => format!(
                "좌석이 밝힌 claude 버전({detected})이 이 관문을 실측한 버전({measured_on})과 \
                 다르다 — 선언된 통과 액션이 이 버전에서 참이라는 근거가 없다. 탈출구 셋: \
                 ① 사람 1회로 넘긴다 ② 이 값을 **베끼지 말고** 실측하라 — 접힌 배너의 잘린 \
                 판독일 수 있다(`claude --version` · `cys gate-corpus --agent claude \
                 --detected-version <실측값> --json`)—— 그 실측값이 다르면 agents.json 봉투의 \
                 measured_on 을 그 값으로 갱신한다 ③ 이 축만 종전으로 되돌린다({ENV_VERSION_PIN}=0 \
                 — 마스터를 누르지 말 것: 마스터는 보류를 close 로 강등한다)"
            ),
        }
    }
}

/// [`confirm_allowed`] 의 사유형. `None` = 허가.
pub fn confirm_denied(o: &Observed, gate_id: &str) -> Option<ConfirmDenied> {
    // ★(0.14.31 · 리뷰 R2 · codex 설계 검토 ①) **화면 층위 봉인이 먼저다.** 코퍼스 층위의 바닥
    //   (`first_run_gates::restore_human_only_builtin_floor`)은 id 와 needle 포함으로 봉하는데,
    //   그 둘 중 어느 것도 "이 화면이 로그인 화면인가" 의 충분조건이 아니다 — 별칭 선언이 정본과
    //   포함관계가 없는 다른 문면(예: 로그인 화면의 `3rd-party platform` 줄)을 needle 로 쓰면
    //   코퍼스 층위를 빠져나간다. 그래서 **코드 정본**(봉투가 손대지 못하는 것)이 지금 화면을
    //   사람 1회 관문으로 식별하면, 코퍼스가 무엇을 선언했든 확인은 열리지 않는다.
    //   실패 방향: 이 항의 오탐 귀결은 보류(사람 1회 · 좌석 보존 · 키 0)다.
    if let Some(b) = first_run_gates::builtin()
        .into_iter()
        .find(|b| b.passability == Passability::HumanOnly && b.matches(o.screen))
    {
        return Some(ConfirmDenied::HumanOnlyScreen(b.id));
    }
    let g = match first_run_gates::identify(o.gates, o.screen) {
        None => return Some(ConfirmDenied::Unidentified),
        Some(g) if g.id != gate_id => return Some(ConfirmDenied::OtherGate(g.id.clone())),
        Some(g) => g,
    };
    if crate::readiness::modal_signature(o.screen).is_some_and(|m| m.cursor_on_exit) {
        return Some(ConfirmDenied::CursorOnExit);
    }
    if g.action.is_none() {
        return Some(ConfirmDenied::NoAction);
    }
    // ★(0.14.31 · 리뷰 R2 · codex major) **선언된 시퀀스가 Return 한 발일 때만** 확인이 열린다.
    //   R1 은 봉투의 `default_index: null`(= "기본 포커스를 모른다")을 보고서에서 '보류' 라고
    //   인쇄하면서 정작 이 경계에서는 아무것도 하지 않았다 — 운영자가 선언한 무지가 키를 막지
    //   못했다. `Some(0)` 을 요구하는 것은 이 조립의 **실제 능력**과 선언을 맞추는 것이다:
    //   여기는 Down 을 보내지 않으므로 `down > 0` 선언과는 애초에 합의가 없다.
    //   ★오늘의 빌트인 folder-trust 는 `default_index:Some(1)` + `action:(1,…)` → `Some(0)` 이라
    //     기본 경로는 한 글자도 바뀌지 않는다(이 항은 코퍼스가 스스로 모호해질 때만 문다).
    //   ★이 항을 "Down 을 보내도 된다" 는 허가로 확대하지 말 것(codex 설계 검토 ⑤) — 전송 후
    //     재관측 없는 다발 전송은 이 벨트가 서 있는 전제(지금 화면의 커서)를 스스로 무너뜨린다.
    match g.down_presses() {
        Some(0) => {}
        down => return Some(ConfirmDenied::SequenceNotBareReturn { down }),
    }
    // ★(0.14.31 · 독립 재유도 H2-B · 정본 §4 WP-1 H-2 · §7 봉인표 ④) **버전 축.**
    //   종전 이 경계는 버전을 한 번도 보지 않았다 — 그래서 보고서가 `held_version_drift` 를
    //   인쇄하면서 같은 화면에 Return 이 나갔다(인쇄된 판정과 실제 키 경로가 달랐다). 관문 선언
    //   (기본 포커스·항목 순서)은 **어떤 버전에 대고 실측한 값**이고, 벤더가 순서를 바꾸면
    //   `Return 한 발 = 통과` 가 곧 `Return 한 발 = No, exit`(좌석 rc 1)이 된다.
    //
    //   ★판정은 코퍼스가 소유한 [`first_run_gates::action_policy`] 를 그대로 소비한다(사본 0 —
    //     여기서 `!=` 를 다시 쓰면 버전 대조가 두 벌이 되고 다음 판에 갈린다). 소비하는 팔은
    //     **불일치 하나**다: `HeldVersionUnknown`(미상)은 오늘 통과한다 — `MEASURED_ON` 이 부분
    //     실측이라 미상까지 접으면 전 좌석이 매 부트마다 사람 1회를 요구한다(별도 결정 ·
    //     [`first_run_gates::ACTION_POLICY_ENFORCEMENT`] doc).
    //
    //   ★증거는 **합집합**이다 — 기동 래치(`o.cli_versions` · 이 부트에서 관측한 버전 전량)
    //     ∪ 지금 화면의 배너 전량. 하나라도 실측본과 다르면 보류한다(증거가 갈리면 조이는 쪽).
    //     화면 배너만 보면 배너가 관문 렌더에 밀려난 틱에서 거부가 풀리고, 래치가 값 하나면
    //     먼저 잡힌 일치가 뒤의 불일치를 영구히 덮는다(수렴 R2 · codex major).
    //
    //   【실패 방향】 오탐(엉뚱한 문자열을 배너로 읽음)의 귀결은 자동확인 보류 = 사람 1회(가역).
    //   미탐의 귀결은 미실측 버전 화면에 Return = 좌석 rc 1(비가역).
    //   ★★(수렴 R2) 그리고 이 축은 **롤백 노브를 가진다**(`o.version_pin_legacy`). 형제 벨트
    //     (커서·라벨·정본 사람 1회)와 달리 이 보류는 재실측 전 기계의 **기본 상태**라, 노브가
    //     없으면 운영자가 쥘 손잡이는 마스터뿐이고 마스터는 보류를 close 로 강등한다 — 그
    //     조합이 정확히 BLOCK-4('엄격 + 즉시 close')다. 종전으로 내린 귀결은 이 축이 태어나기
    //     전과 같다(Return 이 나간다 · 그때의 위험을 그대로 되찾는다 — 그것이 롤백의 뜻이다).
    if !o.version_pin_legacy {
        for v in o
            .cli_versions
            .iter()
            .cloned()
            .chain(first_run_gates::banner_versions(o.screen))
        {
            if let first_run_gates::ActionPolicy::HeldVersionDrift {
                measured_on,
                detected,
            } = first_run_gates::action_policy(g, Some(&v))
            {
                return Some(ConfirmDenied::VersionDrift {
                    measured_on,
                    detected,
                });
            }
        }
    }
    // 술어는 [`action_label_selected`] 하나다(사본 0 · 성찰 R7 이후 이 경계가 유일한 소비처 — 주입
    // 가드의 allow 구멍은 삭제됐다).
    if action_label_selected(g, o.screen) {
        None
    } else {
        Some(ConfirmDenied::LabelUnresolved)
    }
}

/// 한 좌석의 기동 래치가 보존하는 **서로 다른 버전 문자열의 상한**.
///
/// ★상한이 판정을 무디게 하지 않는 이유: 서로 다른 값이 **둘만 되어도** 그중 하나는 반드시
///   실측본과 다르다 = 드리프트가 이미 확정이다. 상한은 병적 입력(배너를 무한히 찍는 화면)에서
///   래치가 무한히 자라는 것만 막는다.
pub const SEAT_VERSION_LATCH_MAX: usize = 4;

/// ★(0.14.31 · 독립 재유도 H2-B · 수렴 R2) [`Observed::cli_versions`] **래치의 갱신 규칙**(순수).
///
/// 부트 루프가 매 틱 부른다: `seat = latch_seat_versions(seat, &delta_text, screen)`.
///
/// 【규칙 셋, 그리고 각각의 이유】
///   · **지우지 않는다**(단조 증가). 이 부트에서 한 번 관측한 버전 증거는 배너가 화면에서
///     밀려나도 남아야 한다 — 안 그러면 드리프트 거부가 한 틱짜리가 되고 다음 틱엔 미상으로
///     열린다(codex 설계 검토 3).
///   · **덮지 않는다**(합집합). R1 판은 값 하나를 sticky 로 들었는데, 그 규칙에서는 **먼저
///     잡힌 일치가 나중의 불일치를 영구히 덮었다**(수렴 R2 · codex major): 첫 틱 델타가 비어
///     있고 화면에 이전 좌석의 잔상 배너(= 실측본과 같은 값)가 있으면 래치가 그 값으로 굳고,
///     그 뒤 델타에 실린 진짜 버전은 다시 읽히지 않는다. 그 좌석은 2.1.263 인데 Return 이
///     나간다(reviewer-claude major · 실측 v1). 합집합은 시간축의 증거를 잃지 않는다.
///   · **판정은 여전히 뒤집히지 않는다.** 집합이 커지는 방향은 언제나 '조이는' 쪽이다(불일치가
///     하나라도 들어오면 보류) — 그래서 R1 이 sticky 로 지키려던 성질(틱마다 판정이 뒤집히지
///     않을 것)은 그대로 산다. 잃는 것은 "느슨해지는 방향의 변동" 뿐이고, 그것은 잃어야 한다.
///
/// 【실패 방향】 못 잡으면 빈 집합 = 미상이고, 미상은 오늘 확인을 막지 않는다(종전과 같음).
/// 잘못 잡으면(잔상·가짜 배너) 불일치로 접혀 **보류**다 — 조이는 쪽이고, 그 보류가 잦은
/// 기계에는 축 노브([`ENV_VERSION_PIN`])가 있다.
pub fn latch_seat_versions(latched: Vec<String>, delta: &str, screen: &str) -> Vec<String> {
    let mut out = latched;
    for v in first_run_gates::banner_versions(delta)
        .into_iter()
        .chain(first_run_gates::banner_versions(screen))
    {
        if out.len() >= SEAT_VERSION_LATCH_MAX {
            break;
        }
        if !out.contains(&v) {
            out.push(v);
        }
    }
    out
}

/// 관문 하나의 **질문형 needle** 만으로 화면을 판별한다(위젯 서명 AND 를 요구하지 않는다).
///
/// ★[`first_run_gates::Gate::matches`](needle ∧ 위젯 서명)와 술어를 나누는 이유 — **쓰임이 둘**이고
///   각 쓰임의 안전 방향이 반대다:
///     · **보류 판정**(`decide`)은 `matches` 를 쓴다. 위젯 AND 는 과잉 보류(오탐)를 줄이고,
///       그래서 놓치는 화면이 생기더라도 U-13 의 ready 판정이 같은 코퍼스로 한 번 더 막는다.
///     · **감지**(자동확인 트리거)는 needle 만 본다. 위젯 줄이 신규 출력에 안 실리면 감지가
///       통째로 죽고, 그 대가는 '노드 0 + 고아 좌석' 이다(종전 감지 폭을 잃지 않는 것이 요구).
///   하나로 합치면 두 쓰임 중 한쪽이 반드시 틀린 폭을 갖는다.
pub fn needle_hit(gate: &Gate, text: &str) -> bool {
    let norm = first_run_gates::normalize(text);
    let flat = first_run_gates::flatten(text);
    gate.needles.iter().any(|n| {
        norm.contains(&first_run_gates::normalize(n)) || flat.contains(&first_run_gates::flatten(n))
    })
}

/// 가드가 막은 전송의 에러 문자열 **머리표**. 호출부가 "이 실패는 보류다(파괴 근거가 아니다)"를
/// 문자열 파싱 없이 구분하게 하는 유일한 계약이다 — 형제 선례 `is_typing_guard_err` 와 같은 형태.
///
/// ★왜 typed error 가 아닌가: `inject_text` 의 `Result<(), String>` 을 typed 로 올리면 호출부
///   11곳이 동시에 바뀌고, 그 커밋은 이 단위(킬체인 차단)의 diff 를 덮어 리뷰가 불가능해진다.
///   대신 **판정 자체는 typed**(`Decision`)로 두고, 문자열 경계에는 머리표 하나만 둔다.
pub const HOLD_TOKEN: &str = "[gate-hold]";

/// 이 에러가 관문 보류인가(= 좌석을 파괴할 근거가 **아니다**).
pub fn is_hold_error(e: &str) -> bool {
    e.starts_with(HOLD_TOKEN)
}

/// 코퍼스에서 폴더신뢰 관문의 **질문형 문면**이 이 텍스트에 있는가.
/// 문면 자체는 코퍼스가 소유한다 — 이 함수는 읽기 소비만 한다(사본 0 · S-1 재발 차단).
pub fn folder_trust_needle_hit(gates: &[Gate], text: &str) -> bool {
    gates
        .iter()
        .any(|g| g.id == GATE_FOLDER_TRUST && needle_hit(g, text))
}

// ═══════════════════════════════════════════════════════════════════════════
// U-15 · 폴더신뢰 자동확인 전송 정책
// ═══════════════════════════════════════════════════════════════════════════

/// 전송 판정 입력의 전량.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrustObserved {
    /// 신규 출현분에서 폴더신뢰 프롬프트를 감지했는가.
    pub hit: bool,
    /// 아직 한 발도 보내지 않았는가.
    pub first: bool,
    /// 전송 뒤에도 프롬프트가 **다시** 새 출력으로 나타났는가(종전 재전송 조건).
    pub persisted: bool,
    /// 지금까지 보낸 횟수.
    pub sends: u32,
    /// 종전 재전송 상한(`BUDGET_TRUST_MAX_SENDS`).
    pub max_sends: u32,
    /// ★U-14 축: 지금 화면이 이 관문의 **확인을 허가하지 않는다**([`confirm_allowed`] 의 부정).
    /// 다른 관문이 떠 있는 경우가 원형이고, 0.14.31(리뷰 R4)부터 **미식별·모호·커서 부재**도 여기 든다.
    pub other_gate: bool,
    /// U-15 롤백 스위치 값.
    pub legacy_v1: bool,
}

/// 지금 Return 을 보내도 되는가(순수 · 진리표 대상).
///
/// ## 재전송 기구를 왜 '상수 1' 이 아니라 '조건' 으로 없앴는가
///
/// 종전 조건은 `first || (sends < BUDGET_TRUST_MAX_SENDS && persisted)` 였다. 여기서
/// `BUDGET_TRUST_MAX_SENDS` 만 2→1 로 내리면 재전송 분기가 **영구 사문화**되어
/// `persisted`·`trust_seen_at`·`BUDGET_TRUST_SETTLE_SECS` 가 전부 죽은 코드가 되고, 다음
/// 감사자는 소스를 읽고 "재전송 기구가 있다" 고 오독한다. 그래서 값이 아니라 **조건**을
/// 줄인다(`first` 단독).
///
/// ## 죽은 코드를 남길지 지울지 — 명시 결정: **남긴다(롤백 분기로 살려 둔다)**
///
/// `persisted`·`sends`·`max_sends` 는 삭제하지 않고 `legacy_v1` 분기의 **실사용 입력**으로
/// 남긴다. 근거 셋:
///   ① 이 단위의 요구는 "롤백 스위치를 반드시 둘 것" 이다. 재전송 기구를 지우면
///      `CYS_TRUST_RETURN_V1=1` 이 되돌릴 대상 자체가 없어져 스위치가 거짓말이 된다.
///   ② `BUDGET_TRUST_MAX_SENDS` 는 `cys.rs` 의 BUDGET 블록 일원이다. 값 변경도, 고아화도
///      예산 파리티 표면을 흔든다(S-5) — **값을 그대로 둔 채 소비를 유지**하는 것이 가장 조용하다.
///   ③ 남은 코드는 죽지 않았다: 스위치를 켠 기계에서 실제로 도는 경로이고, 아래 진리표가
///      두 정책을 **양쪽 다** 박제한다(죽은 코드는 테스트가 없다 — 이 코드는 있다).
///
/// ## `other_gate` 는 롤백으로 열리지 않는다
///
/// "화면에 다른 관문이 있으면 안 보낸다" 는 U-14 축이고, 그 축의 스위치는
/// `CYS_INJECT_GATE_GUARD` 다. `legacy_v1` 로 이 항까지 열면 **하나의 스위치가 두 축을
/// 되돌리게 되어**, 신뢰 감지 폭만 되살리려던 사람이 킬체인까지 함께 되살린다.
///
/// ★(0.14.31 · 리뷰 R4 · codex blocking) 이 항의 **생산자**가 바뀌었다. 종전엔
/// `decide_allowing(..., Some(GATE_FOLDER_TRUST)).blocks()`(= 주입 허가의 부정)였는데, 그 술어는
/// "코퍼스가 화면을 식별하지 못하고 모달 어휘도 없으면" 통과(Send)를 낸다 — 부트 델타에는 질문이
/// 있는데 화면은 `❯ No, exi` 뿐인 잘린 렌더가 정확히 그 자리였고, 거기서 Return 이 부분 렌더된 종료
/// 선택지를 눌렀다. 지금은 [`confirm_allowed`](양성 증거 전용)의 부정이 이 항을 채운다. 그러므로
/// 이름은 `other_gate` 이되 의미는 **"지금 화면이 이 관문의 확인을 허가하지 않는다"** 로 넓어졌다
/// (다른 관문 · 미식별 · 모달 부재 · 모호 · 커서 부재 전부 포함). 이름을 바꾸지 않은 이유는 이
/// 진리표가 U-14/U-15 회귀의 핀이기 때문이다 — 필드명을 바꾸면 핀 전량이 함께 흔들린다.
pub fn trust_send(o: &TrustObserved) -> bool {
    if !o.hit {
        return false;
    }
    // ★킬체인의 실제 방아쇠 차단: 감지는 **누적 델타**에서 하지만 전송 판정은 **지금 화면**을
    //   본다. 신뢰 창을 통과한 뒤에도 델타에는 그 질문이 그대로 남아 있고, 그때 화면은 이미
    //   면책 창이다 — 종전 코드가 2발째를 그 화면에 쏜 경로가 정확히 이것이다.
    if o.other_gate {
        return false;
    }
    if o.legacy_v1 {
        return o.first || (o.sends < o.max_sends && o.persisted);
    }
    o.first
}

// ── 판정부 끝(핀 슬라이스 경계 · U-14/U-15) ──────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::first_run_gates::fixtures;

    fn gates() -> Vec<Gate> {
        first_run_gates::builtin()
    }

    /// 폴더신뢰 관문의 **질문 문면**(코퍼스 소유 · 사본 0 — H-KILLCHAIN-1 ⓑ 가 이 파일의 리터럴을 금지한다).
    fn trust_needle(gs: &[Gate]) -> String {
        gs.iter()
            .find(|g| g.id == GATE_FOLDER_TRUST)
            .expect("코퍼스에 folder-trust")
            .needles[0]
            .clone()
    }

    fn obs<'a>(screen: &'a str, gs: &'a [Gate]) -> Observed<'a> {
        Observed {
            screen,
            gates: gs,
            awakened: Some(false),
            guard_off: false,
            readiness_legacy: false,
            // 기동 래치 없음 = 이 검체의 버전 증거는 **화면 배너뿐**이다(H2-B).
            cli_versions: &[],
            // 버전 축은 켜져 있다(기본) — 노브를 재는 검체는 이 값을 스스로 뒤집는다.
            version_pin_legacy: false,
        }
    }

    /// ★(0.14.31 · 성찰 R7) **구 배선 재현** — 삭제된 `decide_allowing(o, allow_gate_id)` 의 진리표(R3 벨트
    /// 포함)를 검체 안에서만 되살린다. 계측 타당성 대조군 전용이다(프로덕션 코드에서 부르면 이 함수가 아니라
    /// [`decide`] 를 써야 한다 — 그것이 R7 이 인자를 지운 이유다).
    fn legacy_decide_allowing(o: &Observed, allow_gate_id: Option<&str>) -> Decision {
        if o.awakened != Some(false) {
            return Decision::Send;
        }
        let modal = crate::readiness::modal_signature(o.screen);
        let hit = match first_run_gates::identify(o.gates, o.screen) {
            Some(g) => {
                let cursor_on_exit = modal.as_ref().is_some_and(|m| m.cursor_on_exit);
                if allow_gate_id == Some(g.id.as_str()) && !cursor_on_exit && action_label_selected(g, o.screen) {
                    return Decision::Send;
                }
                GateHit { id: g.id.clone(), title: g.title.clone(), human_only: g.passability == Passability::HumanOnly }
            }
            None => {
                if o.readiness_legacy {
                    return Decision::Send;
                }
                match modal {
                    Some(m) => GateHit { id: crate::readiness::MODAL_UNKNOWN_ID.to_string(), title: m.title(), human_only: false },
                    None => return Decision::Send,
                }
            }
        };
        if o.guard_off { Decision::SendObserved(hit) } else { Decision::Hold(hit) }
    }

    /// 실측 관문 6화면 — 문면은 U-12 정본의 픽스처를 **참조**한다(사본 0).
    const GATE_SCREENS: &[(&str, &str)] = &[
        ("theme", fixtures::THEME),
        ("login-method", fixtures::LOGIN_METHOD),
        ("oauth-code", fixtures::OAUTH_CODE),
        ("folder-trust", fixtures::FOLDER_TRUST),
        ("bypass-disclaimer", fixtures::TRUST_ECHO_THEN_DISCLAIMER),
        ("feature-announce-fullscreen", fixtures::FEATURE_FULLSCREEN),
    ];

    // ── U-14 진리표 ────────────────────────────────────────────────────────

    #[test]
    fn gate_screens_hold_every_injection_in_the_first_boot_window() {
        let gs = gates();
        for &(id, screen) in GATE_SCREENS {
            match decide(&obs(screen, &gs)) {
                Decision::Hold(h) => assert_eq!(h.id, id, "관문 식별이 어긋났다: 기대 {id}"),
                other => panic!("{id}: 관문 화면인데 주입이 허용됐다 — {other:?}"),
            }
        }
    }

    #[test]
    fn healthy_screen_never_holds() {
        let gs = gates();
        assert_eq!(decide(&obs(fixtures::READY_SHELL, &gs)), Decision::Send);
        // 관문 코퍼스가 비어도 **건강한 화면**은 종전대로 통과한다.
        assert_eq!(decide(&obs(fixtures::READY_SHELL, &[])), Decision::Send);
        assert_eq!(decide(&obs(fixtures::LIVE_TUI_AT_PROMPT, &[])), Decision::Send);
        // ★(0.14.31 · WP-1 H-1 재핀 · 정본 §4 H-1 · 오너 위임 승인 2026-09-06) 종전 핀은
        //   "코퍼스가 비면 축이 통째로 없다 = 관문 화면도 Send" 였다. 모달 거부는 **코퍼스와 독립**인
        //   두 번째 축이므로(코퍼스는 agents.json 봉투로 비워질 수 있고, 그 순간 관문 창에 Return 이
        //   나가던 것이 정확히 이 축이 막는 결함이다) 빈 코퍼스라도 모달 어휘가 전경이면 보류다.
        //   종전 동작으로 되돌리는 손잡이는 코퍼스 비우기가 아니라 롤백 스위치(가드 노브·마스터)다.
        match decide(&obs(fixtures::FOLDER_TRUST, &[])) {
            Decision::Hold(h) => assert_eq!(h.id, crate::readiness::MODAL_UNKNOWN_ID),
            other => panic!("빈 코퍼스에서 관문 화면에 주입이 허용됐다(코퍼스 비우기 = 그물 해제): {other:?}"),
        }
    }

    /// ★생애 창 상한 — 치명위험 ①. 각성한 노드가 관문 문면을 **본문으로** 출력해도
    /// (이 저장소의 감사 문서·`first_run_gates.rs` 를 `cat` 하는 순간이 그렇다) 주입이 막히면 안 된다.
    #[test]
    fn after_awakening_ack_the_scan_is_off_even_on_gate_text() {
        let gs = gates();
        for &(_, screen) in GATE_SCREENS {
            for awakened in [Some(true), None] {
                let mut o = obs(screen, &gs);
                o.awakened = awakened;
                assert_eq!(
                    decide(&o),
                    Decision::Send,
                    "생애 창이 닫혔는데(awakened={awakened:?}) 관문 문면으로 주입이 막혔다 — \
                     작업 중 노드가 영구 차단된다"
                );
            }
        }
    }

    #[test]
    fn rollback_switch_downgrades_to_observation_only() {
        let gs = gates();
        let mut o = obs(fixtures::TRUST_ECHO_THEN_DISCLAIMER, &gs);
        o.guard_off = true;
        match decide(&o) {
            Decision::SendObserved(h) => assert_eq!(h.id, "bypass-disclaimer"),
            other => panic!("롤백이 관측 전용 강등이 아니다: {other:?}"),
        }
        assert!(!decide(&o).blocks(), "롤백인데 전송이 막힌다");
    }

    #[test]
    fn rollback_switches_are_strict_and_default_to_the_new_behavior() {
        // 기본(미설정) = 신동작.
        assert!(!guard_off_from(None), "가드 기본값이 꺼짐이다");
        assert!(!trust_v1_from(None), "신뢰 정책 기본값이 종전이다");
        // 느슨한 truthy 불허 — 오타로 안전장치가 조용히 뒤집히면 안 된다.
        for raw in ["", "1", "off", "false", "no", "0 ", "OFF"] {
            assert!(!guard_off_from(Some(raw)), "가드 스위치가 {raw:?} 를 받아들였다");
        }
        assert!(guard_off_from(Some("0")));
        for raw in ["", "0", "on", "true", "yes", " 1", "TRUE"] {
            assert!(!trust_v1_from(Some(raw)), "신뢰 스위치가 {raw:?} 를 받아들였다");
        }
        assert!(trust_v1_from(Some("1")));
    }

    /// ★(0.14.31 · 성찰 R7) 주입 가드에는 **allow 구멍이 없다** — 폴더신뢰 화면에서도 [`decide`] 는 보류이고,
    /// 자동확인의 허가는 별개 술어([`confirm_denied`])가 **그 id 하나**에만 연다(면책 창에서는 닫힌다).
    /// 종전 구멍(`decide_allowing(.., Some(id))`)의 진리표는 대조군 `legacy_decide_allowing` 이 그대로 재현한다.
    #[test]
    fn injection_guard_has_no_allow_hole_and_confirmation_is_a_separate_belt() {
        let gs = gates();
        match decide(&obs(fixtures::FOLDER_TRUST, &gs)) {
            Decision::Hold(h) => assert_eq!(h.id, GATE_FOLDER_TRUST),
            other => panic!("주입 가드가 폴더신뢰 관문을 통과시켰다(구멍 부활): {other:?}"),
        }
        assert_eq!(
            confirm_denied(&obs(fixtures::FOLDER_TRUST, &gs), GATE_FOLDER_TRUST),
            None,
            "자동확인 대상 관문까지 막으면 폴더신뢰 자동확인이 통째로 죽는다"
        );
        // ★킬체인 화면: 신뢰 에코가 남아 있어도 면책 창이므로 확인은 열리지 않는다.
        assert_eq!(
            confirm_denied(&obs(fixtures::TRUST_ECHO_THEN_DISCLAIMER, &gs), GATE_FOLDER_TRUST),
            Some(ConfirmDenied::OtherGate("bypass-disclaimer".into())),
            "면책 창에 확인이 열렸다(킬 스텝)"
        );
        // 계측 타당성 — 구 구멍은 같은 두 화면에서 Send / Hold(bypass-disclaimer) 였다.
        assert_eq!(legacy_decide_allowing(&obs(fixtures::FOLDER_TRUST, &gs), Some(GATE_FOLDER_TRUST)), Decision::Send);
        assert!(matches!(
            legacy_decide_allowing(&obs(fixtures::TRUST_ECHO_THEN_DISCLAIMER, &gs), Some(GATE_FOLDER_TRUST)),
            Decision::Hold(h) if h.id == "bypass-disclaimer"
        ));
        // 소스 핀 — 라이브러리 본문(테스트 모듈 밖)에 allow 인자가 부활하지 않았다(컴파일러가 재는 것을
        // 문자열로도 못박는다 — 같은 이름의 새 API 가 조용히 생기는 것까지 막는다).
        let src = include_str!("inject_guard.rs");
        let body = &src[..src.find("#[cfg(test)]").expect("테스트 모듈 경계")];
        assert!(!body.contains("allow_gate_id"), "주입 가드에 allow 구멍 인자가 부활했다");
        assert!(!body.contains("pub fn decide_allowing"), "삭제된 allow API 가 부활했다");
    }

    // ── needle 감지(U-15) ──────────────────────────────────────────────────

    /// ★확인 에코는 감지 근거가 아니다 — 2026-07-29 킬체인의 형태를 직접 반증한다.
    #[test]
    fn confirm_echo_is_not_a_trust_detection() {
        let gs = gates();
        assert!(
            folder_trust_needle_hit(&gs, fixtures::FOLDER_TRUST),
            "질문형 문면을 감지하지 못한다 — 자동확인이 통째로 죽는다"
        );
        let echo = "Yes, I trust this folder ✔\n";
        assert!(
            !folder_trust_needle_hit(&gs, echo),
            "확인 에코가 신뢰 프롬프트로 읽힌다 — 2발째 Return 이 면책 창을 누른다(킬체인)"
        );
        // 계측 타당성: 구 하드코딩 needle 은 실제로 그 에코에 걸린다(대조군).
        assert!(
            first_run_gates::flatten(echo).contains("trustthisfolder"),
            "구 needle 이 에코에 안 걸리면 킬체인 서사가 틀린 것 — 근거를 재확인하라"
        );
    }

    /// 두 술어(감지용 `needle_hit` · 보류용 `Gate::matches`)의 **폭이 다름**을 박제한다.
    /// ★문면은 픽스처에서 **잘라 쓴다** — 여기에 문장을 손으로 적으면 그것이 사본 4벌째다(S-1).
    #[test]
    fn needle_hit_ignores_widget_signature_but_matches_wrapped_lines() {
        let gs = gates();
        let g = gs.iter().find(|g| g.id == GATE_FOLDER_TRUST).unwrap();
        // 위젯 서명 줄('Enter to confirm …')을 뺀 화면.
        let without_widget: String = fixtures::FOLDER_TRUST
            .lines()
            .filter(|l| !g.widget.iter().any(|w| l.contains(w.as_str())))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            needle_hit(g, &without_widget),
            "위젯 줄이 신규 출력에 안 실리면 감지가 통째로 죽는다(종전 감지 폭 상실)"
        );
        assert!(
            !g.matches(&without_widget),
            "보류 판정이 위젯 AND 를 잃었다 — 두 술어의 폭이 같아지면 한쪽이 반드시 틀린다"
        );
        // TUI 폭에 따라 접힌 프롬프트도 감지된다(공백 정규화·제거 양쪽 매칭).
        let folded = without_widget.replace(' ', "\n  ");
        assert!(needle_hit(g, &folded), "접힌 프롬프트를 놓친다 — 자동확인 불발");
    }

    // ── U-15 전송 정책 진리표 ──────────────────────────────────────────────

    fn t(hit: bool, first: bool, persisted: bool, sends: u32) -> TrustObserved {
        TrustObserved {
            hit,
            first,
            persisted,
            sends,
            max_sends: 2,
            other_gate: false,
            legacy_v1: false,
        }
    }

    #[test]
    fn trust_send_is_exactly_one_shot_by_default() {
        assert!(trust_send(&t(true, true, false, 0)), "첫 감지에서 보내지 않는다");
        // 재전송 조건이 아무리 갖춰져도 2발째는 없다.
        for persisted in [true, false] {
            for sends in [1u32, 2, 3] {
                assert!(
                    !trust_send(&t(true, false, persisted, sends)),
                    "2발째가 나갔다(persisted={persisted} sends={sends}) — 그때 화면은 면책 창이다"
                );
            }
        }
        assert!(!trust_send(&t(false, true, false, 0)), "미감지인데 보냈다");
    }

    #[test]
    fn other_gate_on_screen_blocks_the_trust_return_even_on_the_first_shot() {
        let mut o = t(true, true, false, 0);
        o.other_gate = true;
        assert!(!trust_send(&o), "다른 관문(면책)이 떠 있는데 신뢰 Return 이 나갔다 — 킬 스텝");
        // ★롤백으로도 이 항은 열리지 않는다(축이 다르다).
        o.legacy_v1 = true;
        assert!(!trust_send(&o), "신뢰 롤백 스위치가 U-14 축까지 되돌렸다(스위치 오염)");
    }

    #[test]
    fn legacy_v1_restores_the_previous_resend_policy() {
        let mut o = t(true, false, true, 1);
        assert!(!trust_send(&o), "기본 정책에서 재전송이 살아 있다");
        o.legacy_v1 = true;
        assert!(trust_send(&o), "롤백이 종전 재전송 정책을 되살리지 못한다");
        // 종전 상한은 그대로 집행된다(값 무변경 — 예산 표면 무접촉).
        o.sends = o.max_sends;
        assert!(!trust_send(&o), "롤백 경로가 종전 상한을 넘겼다");
        // persisted 미충족이면 종전에도 재전송하지 않았다.
        o.sends = 1;
        o.persisted = false;
        assert!(!trust_send(&o));
    }

    // ── ★킬체인 e2e ────────────────────────────────────────────────────────

    /// 신뢰 → 면책 연쇄를 **화면 시퀀스로** 모사해 Return 총 발수와 면책 창 접촉을 센다.
    ///
    /// 실측 순서: ①신뢰 창 → ②(Return 1발) → ③확인 에코 + 면책 창.
    /// 종전 코드는 ③에서 `trustthisfolder` 재매칭 + `persisted` 로 2발째를 쐈고, 그 Return 이
    /// 면책 창의 기본 포커스 `No, exit` 를 눌러 좌석을 rc 1 로 죽였다.
    /// ★두 축(U-14 화면 재확인 · U-15 1발 래치)이 **각각 단독으로** 킬체인을 닫는지 2×2 로 본다.
    ///   한 축만 검사하면 다른 축을 되돌려도 검체가 초록이라 회귀를 못 본다(실제로 이 검체의
    ///   첫 판이 그랬다 — 돌연변이 시험에서 U-15 를 종전으로 되돌렸는데 e2e 가 통과했다).
    #[test]
    fn killchain_trust_then_disclaimer_sends_exactly_one_return_and_never_touches_the_disclaimer() {
        let gs = gates();
        let screens = [fixtures::FOLDER_TRUST, fixtures::TRUST_ECHO_THEN_DISCLAIMER];

        // (guard_off, legacy_v1) → (Return 발수, 면책 창 접촉)
        let run = |guard_off: bool, legacy_v1: bool, legacy_producer: bool| -> (u32, bool) {
            // 누적 델타 — 화면이 넘어가도 질문이 그대로 남는다(since_line 이후 전량).
            let (mut delta, mut sends, mut seen_at) = (String::new(), 0u32, None::<u32>);
            let mut touched = false;
            for (tick, screen) in screens.iter().enumerate() {
                delta.push_str(screen);
                let cursor = tick as u32 + 1;
                // 감지 축: 기본은 코퍼스의 질문형 needle · 롤백이면 구 하드코딩 needle 이 더해진다.
                let hit = folder_trust_needle_hit(&gs, &delta)
                    || (legacy_v1 && first_run_gates::flatten(&delta).contains("trustthisfolder"));
                // 화면 재확인 축(U-14): 지금 떠 있는 관문이 folder-trust 가 아니면 막힌다.
                let mut o = obs(screen, &gs);
                o.guard_off = guard_off;
                // ★(리뷰 R4) 프로덕션과 **같은 생산자**를 쓴다 — 확인 허가는 주입 허가의 부정이 아니다.
                //   `legacy_producer` 는 구 배선(주입 허가의 부정)을 그대로 재현하는 계측 타당성 대조군용.
                let other_gate = if legacy_producer {
                    legacy_decide_allowing(&o, Some(GATE_FOLDER_TRUST)).blocks()
                } else {
                    !confirm_allowed(&o, GATE_FOLDER_TRUST)
                };
                let send = trust_send(&TrustObserved {
                    hit,
                    first: sends == 0,
                    persisted: seen_at.map(|c| cursor > c).unwrap_or(false),
                    sends,
                    max_sends: 2,
                    other_gate,
                    legacy_v1,
                });
                if send {
                    sends += 1;
                    seen_at = Some(cursor);
                    if tick == 1 {
                        touched = true;
                    }
                }
            }
            (sends, touched)
        };

        // ① 기본(두 축 모두 신동작) · ② U-14 만 되돌림 · ③ U-15 만 되돌림 — 셋 다 1발·미접촉.
        for (guard_off, legacy_v1, why) in [
            (false, false, "기본(두 축 신동작)"),
            (true, false, "U-14 롤백 — U-15 의 1발 래치가 단독으로 막아야 한다"),
            (false, true, "U-15 롤백 — U-14 의 화면 재확인이 단독으로 막아야 한다"),
        ] {
            let (sends, touched) = run(guard_off, legacy_v1, false);
            assert_eq!(sends, 1, "{why}: Return 이 {sends}발 나갔다(기대 1발)");
            assert!(!touched, "{why}: 면책 창에 Return 이 닿았다 — 좌석이 rc 1 로 죽는 경로");
        }

        // ④ ★계측 타당성 대조군 — **구 배선**(주입 허가의 부정 = R3 까지의 생산자) + 두 축 롤백이면
        //    결함이 재현된다(2발 · 면책 접촉). 재현되지 않으면 이 검체는 '원래 안 나는 일을 안 난다고
        //    확인' 하는 공허한 검사다.
        //    ★(리뷰 R4) 신 생산자(`confirm_allowed`)에서는 두 노브를 다 켜도 2발이 나지 않는다 —
        //    확인 벨트는 롤백으로 열리지 않기 때문이다(모듈 doc '롤백 예외'). 그 사실 자체를 아래 ⑤가 잰다.
        let (legacy_sends, legacy_touch) = run(true, true, true);
        assert_eq!(legacy_sends, 2, "구 정책이 2발을 쏘지 않는다 — 킬체인 서사가 틀렸다(계측 무효)");
        assert!(legacy_touch, "구 정책이 면책 창에 닿지 않는다 — 결함 재현 실패(계측 무효)");
        // ⑤ ★(리뷰 R4 · codex) 확인 벨트는 롤백으로 열리지 않는다 — 두 노브를 다 켜도 1발·면책 미접촉.
        let (both_knobs, both_touch) = run(true, true, false);
        assert_eq!(both_knobs, 1, "롤백 두 개로 확인 벨트가 열려 {both_knobs}발이 나갔다");
        assert!(!both_touch, "롤백 두 개로 면책 창에 Return 이 닿았다");
    }

    /// 정상 경로 회귀 0 — 관문이 없는 화면에서는 종전과 똑같이 주입·제출된다.
    #[test]
    fn normal_path_is_unchanged() {
        let gs = gates();
        for screen in [fixtures::READY_SHELL, "", "worker idle\n❯ \n"] {
            for awakened in [Some(false), Some(true), None] {
                let mut o = obs(screen, &gs);
                o.awakened = awakened;
                assert_eq!(decide(&o), Decision::Send, "정상 화면에서 주입이 막혔다: {screen:?}");
            }
        }
    }

    #[test]
    fn hold_error_marker_is_recognizable_and_does_not_swallow_ordinary_errors() {
        let msg = format!("{HOLD_TOKEN} 관문 보류(gate=bypass-disclaimer)");
        assert!(is_hold_error(&msg));
        for other in [
            "typing_guard: human is typing",
            "method_not_found",
            "관문 보류(gate=x)", // 머리표 없는 우연한 문면
            "",
        ] {
            assert!(!is_hold_error(other), "일반 실패를 보류로 오분류했다: {other:?}");
        }
    }

    /// 코퍼스 id 계약 — 이 모듈이 이름으로 지목하는 관문이 정본에 실재해야 한다.
    #[test]
    fn folder_trust_id_exists_in_the_corpus() {
        assert!(
            gates().iter().any(|g| g.id == GATE_FOLDER_TRUST),
            "코퍼스에 {GATE_FOLDER_TRUST} 가 없다 — 자동확인 예외 구멍이 아무 관문도 가리키지 않는다"
        );
    }

    // ═══════════════════════════════════════════════════════════════════════
    // ★(0.14.31 · WP-1 H-1) 공통 모달 거부 — 가드도 **같은 함수**(`readiness::modal_signature`)를 소비한다
    // ═══════════════════════════════════════════════════════════════════════

    /// 화면의 아래 n 줄(잘린 관문 — 질문 줄 소실). 관문 needle 사본을 만들지 않기 위한 변형 도우미.
    fn clip_tail(screen: &str, n: usize) -> String {
        let lines: Vec<&str> = screen.lines().collect();
        let mut out = lines[lines.len().saturating_sub(n)..].join("\n");
        out.push('\n');
        out
    }

    /// 실측 관문 화면에서 **커서만** 옮긴 화면(readiness 검체의 도우미와 같은 형태 · 문면 무변).
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

    /// ★본문 전송 후 Return 직전에 관문이 **잘린 채** 뜨는 경우 — 코퍼스는 식별 못 하지만 가드는 막는다.
    #[test]
    fn clipped_modal_between_paste_and_return_is_held_as_unknown_modal() {
        let gs = gates();
        let clipped = clip_tail(fixtures::TRUST_ECHO_THEN_DISCLAIMER, 3); // `❯ 1. No, exit` · `2. Yes, I accept` · 푸터
        assert!(
            first_run_gates::identify(&gs, &clipped).is_none(),
            "전제 붕괴: 코퍼스가 잘린 면책 창을 식별한다면 이 검체는 모달 축을 재지 못한다"
        );
        match decide(&obs(&clipped, &gs)) {
            Decision::Hold(h) => {
                assert_eq!(h.id, crate::readiness::MODAL_UNKNOWN_ID);
                assert!(!h.human_only);
                assert!(h.title.contains("미등재 모달"), "{}", h.title);
            }
            other => panic!("잘린 면책 창(커서=No, exit)에 제출 Return 이 나간다(킬 스텝): {other:?}"),
        }
        // 롤백(가드 노브)은 관측 전용으로 강등한다 — 모달 폴백도 같은 스위치를 따른다(새 노브 0).
        let mut o = obs(&clipped, &gs);
        o.guard_off = true;
        assert!(matches!(decide(&o), Decision::SendObserved(h) if h.id == crate::readiness::MODAL_UNKNOWN_ID));
        // 생애 창이 닫힌 뒤에는 스캔하지 않는다(치명위험 ① — 각성한 노드가 모달 어휘를 본문으로 출력해도 무관).
        for awakened in [Some(true), None] {
            let mut o = obs(&clipped, &gs);
            o.awakened = awakened;
            assert_eq!(decide(&o), Decision::Send);
        }
    }

    /// ★(리뷰 R1) 라이브 2.1.261 그리드(입력 상자 아래 상태줄) — 주입 가드는 **부트 창 안**에서 readiness 와
    /// 같은 함수로 본문 어휘를 보류하고(창 상수 · 재주입 생애 창의 완화는 readiness 축 ①' 몫), 창이 닫힌
    /// 좌석에서는 스캔하지 않는다. 두 판정기의 어휘 함수가 하나임을 라이브 형상으로도 못 박는다.
    #[test]
    fn live_2_1_261_grid_shares_the_modal_vocabulary_with_readiness() {
        let gs = gates();
        let live = fixtures::LIVE_TUI_2_1_261_STATUS_BELOW_PROMPT;
        // 어휘 없는 라이브 그리드 — 부트 창 안에서도 Send(건강한 프롬프트 · 라이브락 방향 회귀 없음).
        assert_eq!(decide(&obs(live, &gs)), Decision::Send);
        assert!(crate::readiness::modal_signature(live).is_none());
        // 본문에 모달 어휘 + 라이브 프롬프트 — 부트 창 안(awakened=Some(false))에서는 보류(관문 축과 같은 상수 창).
        let body_vocab = format!("  H-1: `Enter to confirm` ∧ `Esc to cancel`\n{live}");
        assert!(matches!(decide(&obs(&body_vocab, &gs)), Decision::Hold(h) if h.id == crate::readiness::MODAL_UNKNOWN_ID));
        // 각성한 좌석(창 닫힘)에서는 스캔 0 — 재주입 경로의 판정은 readiness(Site::Reinject)가 맡는다.
        let mut o = obs(&body_vocab, &gs);
        o.awakened = Some(true);
        assert_eq!(decide(&o), Decision::Send);
        // 전경 권한 프롬프트 + 상태줄 — 보류(readiness 검체 ②와 같은 판정).
        let foreground = format!("{}\n{}", live.trim_end_matches('\n'), fixtures::LIVE_PERMISSION_PROMPT);
        assert!(matches!(decide(&obs(&foreground, &gs)), Decision::Hold(h) if h.id == crate::readiness::MODAL_UNKNOWN_ID));
        // ★(리뷰 R1b) 실측 바이트 그대로의 그리드(`❯` + U+00A0) — 같은 세 판정(건강=Send · 본문 어휘=Hold ·
        //   전경 모달=Hold). 두 판정기가 NBSP 를 공백으로 읽지 못하면 여기서 갈린다.
        let nbsp = fixtures::LIVE_TUI_2_1_261_NBSP_PROMPT;
        assert!(nbsp.contains("❯\u{a0}\n"), "검체 전제: 실측 NBSP 바이트");
        assert_eq!(decide(&obs(nbsp, &gs)), Decision::Send);
        assert!(crate::readiness::modal_signature(nbsp).is_none());
        let nbsp_vocab = format!("  H-1: `Enter to confirm` ∧ `Esc to cancel`\n{nbsp}");
        assert!(matches!(decide(&obs(&nbsp_vocab, &gs)), Decision::Hold(h) if h.id == crate::readiness::MODAL_UNKNOWN_ID));
        let nbsp_fore = format!("{}\n{}", nbsp.trim_end_matches('\n'), fixtures::LIVE_PERMISSION_PROMPT);
        assert!(matches!(decide(&obs(&nbsp_fore, &gs)), Decision::Hold(h) if h.id == crate::readiness::MODAL_UNKNOWN_ID));
    }

    /// 확인 벨트는 **커서가 종료 선택지 위**에 있으면 닫힌다 — 2.1.261 폴더신뢰(기본 포커스 `No, exit`)에
    /// 종전 자동확인 Return 이 나가면 좌석이 죽는다(CONTRACTS B-7 · 재핀은 H-2 의 몫 · 이 벨트는 그 앞을 막는다).
    /// ★(성찰 R7) 생산자는 [`confirm_denied`] 다 — 구 구멍(`legacy_decide_allowing`)도 같은 화면에서 닫혔음을
    /// 대조군으로 함께 잰다(R3 벨트의 보존 · 삭제가 거동을 되돌리지 않았다).
    #[test]
    fn confirmation_closes_when_the_cursor_sits_on_the_exit_option() {
        let gs = gates();
        // 2.1.241 형(커서=Yes) — 확인은 열린다(자동확인 기능 보존).
        assert_eq!(confirm_denied(&obs(fixtures::FOLDER_TRUST, &gs), GATE_FOLDER_TRUST), None);
        // 커서만 `No, exit` 로 옮긴 같은 관문 — 같은 id 인데 Return 은 통과가 아니라 종료다 → 거부.
        let on_exit = with_cursor_on(fixtures::FOLDER_TRUST, 2);
        let g = first_run_gates::identify(&gs, &on_exit).expect("커서 이동은 관문 식별을 바꾸지 않는다");
        assert_eq!(g.id, GATE_FOLDER_TRUST);
        assert_eq!(
            confirm_denied(&obs(&on_exit, &gs), GATE_FOLDER_TRUST),
            Some(ConfirmDenied::CursorOnExit),
            "커서가 No, exit 위인데 자동확인 Return 이 허용됐다(좌석 사망 경로)"
        );
        // 계측 타당성 — 구 구멍도 커서=Yes 면 열리고 커서=No, exit 면 닫혔다(R3 벨트).
        assert_eq!(legacy_decide_allowing(&obs(fixtures::FOLDER_TRUST, &gs), Some(GATE_FOLDER_TRUST)), Decision::Send);
        assert!(matches!(
            legacy_decide_allowing(&obs(&on_exit, &gs), Some(GATE_FOLDER_TRUST)),
            Decision::Hold(h) if h.id == GATE_FOLDER_TRUST
        ));
        // 전송 정책 조립도 같은 결론 — `other_gate`(=확인 거부) 가 참이면 `trust_send` 는 쏘지 않는다.
        let blocked = confirm_denied(&obs(&on_exit, &gs), GATE_FOLDER_TRUST).is_some();
        assert!(!trust_send(&TrustObserved {
            hit: true, first: true, persisted: false, sends: 0, max_sends: 2,
            other_gate: blocked, legacy_v1: false,
        }));
    }

    /// ★★(0.14.31 · 리뷰 R4 · codex blocking) **미식별 잘린 관문**에서 확인 키가 나가면 안 된다.
    ///
    /// 【재현한 결함】 누적 델타에는 폴더신뢰 질문이 있는데 **지금 화면**은 `❯ No, exi` 한 줄뿐이다(부분 렌더·
    /// 스크롤). 코퍼스는 그 화면을 식별하지 못하고(`identify=None`) 모달 서명도 서지 않는다(완전한 종료 라벨
    /// 없음 · 번호 행 없음 · 푸터 없음) → 종전 생산자(`decide_allowing(...).blocks()`)는 **Send**(=막지 않음)를
    /// 냈고, 그 Return 이 부분 렌더된 `No, exit` 을 눌러 좌석이 rc 1 로 죽는다.
    ///
    /// 【무엇을 잰다】 ① 코퍼스가 그 화면을 **식별하지 못한다**(전제가 살아 있어야 이 검체가 미식별 축을
    /// 잰다 — R3 당시 생산자가 여기서 Send 를 낸 이유가 바로 그 미식별이었고, 지금은 R5 의 모달 서명이
    /// 같은 화면을 별도로 잡는다) · ② `confirm_allowed` 는 거짓이다 · ③ 조립(`trust_send`)이 **0발**이다 ·
    /// ④ 세 롤백 노브 조합 어디에서도 0발이다(확인 벨트는 롤백으로 열리지 않는다).
    #[test]
    fn unidentified_clipped_screen_never_confirms_even_though_injection_is_permitted() {
        let gs = gates();
        // 잘린 화면 3종: 종료 라벨만 · 빈 화면 · 커서만.
        // ★문면 리터럴 사본 금지(H-KILLCHAIN-1) — 질문 화면은 **코퍼스에서** 읽어 만든다.
        let question_only = format!("{}\n", trust_needle(&gs));
        for (label, screen) in [
            ("종료 라벨 잘림", "❯ No, exi\n"),
            ("빈 화면", ""),
            ("커서만", "❯ \n"),
            ("선택지 없이 질문만", question_only.as_str()),
        ] {
            assert!(
                first_run_gates::identify(&gs, screen).is_none(),
                "{label}: 전제 붕괴 — 코퍼스가 이 화면을 식별한다면 이 검체는 미식별 축을 재지 못한다\n{screen}"
            );
            for (guard_off, readiness_legacy) in [(false, false), (true, false), (false, true), (true, true)] {
                let mut o = obs(screen, &gs);
                o.guard_off = guard_off;
                o.readiness_legacy = readiness_legacy;
                assert!(
                    !confirm_allowed(&o, GATE_FOLDER_TRUST),
                    "{label}: 미식별 화면에서 확인 허가가 났다(guard_off={guard_off} legacy={readiness_legacy})\n{screen}"
                );
                for legacy_v1 in [false, true] {
                    assert!(
                        !trust_send(&TrustObserved {
                            hit: true, first: true, persisted: false, sends: 0, max_sends: 2,
                            other_gate: !confirm_allowed(&o, GATE_FOLDER_TRUST), legacy_v1,
                        }),
                        "{label}: 조립이 미식별 화면에 Return 을 쐈다(좌석 사망 경로)"
                    );
                }
            }
        }
        // ★(0.14.31 · 리뷰 R5 · codex blocking) **잘린 선택기는 이제 주입도 막는다.** R4 는 확인만
        //   좁혔고 그때 이 자리에는 `Decision::Send` 가 핀돼 있었다 — 그 Send 가 디렉티브 붙여넣기와
        //   Return 을 부분 렌더된 종료 선택지로 내보내던 잔여 킬체인이다(확인 격리만으로는 안 닫힌다).
        let clipped = "❯ No, exi\n";
        // 계측 타당성 ①: 이 화면을 잡는 것은 **새 규칙 하나뿐**이다(ⓐ 종료 라벨 전문·ⓑ 푸터·ⓒⓓ 번호
        //   어느 것도 서지 않는다). 다른 규칙이 이미 잡고 있었다면 이 검체는 공허한 검사다.
        let sig = crate::readiness::modal_signature(clipped).expect("잘린 선택기가 모달로 안 잡힌다");
        assert_eq!(sig.kinds, vec!["clipped-choice-row"], "다른 규칙이 이미 이 화면을 잡고 있었다(계측 무효)");
        assert!(!sig.cursor_on_exit, "전제: 종료 라벨 **전문** 위가 아니다(부정 증거만으로는 안 닫힌다)");
        // 계측 타당성 ②: 롤백(`readiness_legacy`)으로 새 축을 끄면 **구 판정(Send)** 이 그대로 재현된다
        //   — 결함 서사가 사실이었다는 증거이자, 롤백 경로가 반쪽이 아니라는 증거다.
        let mut legacy = obs(clipped, &gs);
        legacy.readiness_legacy = true;
        assert_eq!(decide(&legacy), Decision::Send, "구 판정이 재현되지 않는다(계측 무효 · 롤백 반쪽)");
        // 킬체인 전량: 주입 가드 · 확인 경계 · 준비 판정 셋이 **모두** 닫힌다(성찰 R7: allow 구멍은 삭제).
        assert!(decide(&obs(clipped, &gs)).blocks(), "잘린 선택기 화면에 본문이 주입된다");
        assert!(
            confirm_denied(&obs(clipped, &gs), GATE_FOLDER_TRUST).is_some(),
            "확인 경계가 미식별 잘린 선택기에서 열렸다"
        );
        // 정상 좌석 회귀 0 — 건강한 화면·확인 에코·빈 composer 는 그대로 통과한다.
        assert_eq!(decide(&obs(fixtures::READY_SHELL, &gs)), Decision::Send);
        assert_eq!(decide(&obs("❯ \n", &gs)), Decision::Send, "빈 composer 가 막혔다(가용성 붕괴)");
        assert_eq!(
            decide(&obs(fixtures::LIVE_TUI_2_1_261_STATUS_BELOW_PROMPT, &gs)),
            Decision::Send,
            "라이브 2.1.261 프롬프트가 막혔다(부트 영구 보류)"
        );
        assert_eq!(
            decide(&obs("Yes, I trust this folder ✔\n", &gs)),
            Decision::Send,
            "확인 에코가 모달로 잡혔다(2026-07-29 킬체인 역방향 회귀)"
        );
        // 그러나 식별되는 정상 관문 화면에서는 확인이 정확히 열린다(자동확인 가용성 보존).
        assert!(confirm_allowed(&obs(fixtures::FOLDER_TRUST, &gs), GATE_FOLDER_TRUST));
        // 다른 관문 id 로는 열리지 않는다(구멍은 id 하나).
        assert!(!confirm_allowed(&obs(fixtures::FOLDER_TRUST, &gs), "bypass-disclaimer"));
        assert!(!confirm_allowed(&obs(fixtures::TRUST_ECHO_THEN_DISCLAIMER, &gs), GATE_FOLDER_TRUST));
        // 각성 래치는 확인 술어를 **열지 않는다**(주입 허가의 조기 반환을 복사하지 않았다).
        for awakened in [Some(true), None, Some(false)] {
            let mut o = obs(clipped, &gs);
            o.awakened = awakened;
            assert!(!confirm_allowed(&o, GATE_FOLDER_TRUST), "awakened={awakened:?} 에서 확인이 열렸다");
        }
        // ★(0.14.31 · 리뷰 R1(R6회차) · codex blocking) **꼬리에 무관한 줄이 이어져도** 킬체인은 닫힌다.
        //   R5 는 판정 재료를 화면 끝까지로 잡아, 아래 괘선 한 줄이 서명을 통째로 지웠다(그 프레임에서
        //   주입 가드도 allow 구멍도 다시 열렸다 — 확인만 격리해서는 안 닫힌다는 R5 의 교훈 그대로).
        let trailing = "❯ No, exi\n────────────────\n";
        let sig = crate::readiness::modal_signature(trailing)
            .expect("괘선이 이어진 잘린 선택기가 모달로 잡히지 않는다(꼬리 경계 회귀)");
        assert_eq!(sig.kinds, vec!["clipped-choice-row"], "다른 규칙이 이미 잡고 있었다(계측 무효)");
        assert!(!sig.cursor_on_exit, "전제: 종료 라벨 전문 위가 아니다");
        assert!(decide(&obs(trailing, &gs)).blocks(), "꼬리가 이어진 잘린 선택기 화면에 본문이 주입된다");
        assert!(
            confirm_denied(&obs(trailing, &gs), GATE_FOLDER_TRUST).is_some(),
            "확인 경계가 꼬리 이어진 잘린 선택기에서 열렸다"
        );
        assert!(!confirm_allowed(&obs(trailing, &gs), GATE_FOLDER_TRUST));
        let mut trailing_legacy = obs(trailing, &gs);
        trailing_legacy.readiness_legacy = true;
        assert_eq!(decide(&trailing_legacy), Decision::Send, "롤백이 구 판정을 재현하지 않는다(반쪽 롤백)");
    }

    /// ★(0.14.31 · 리뷰 R4 · codex blocking) 짧게 잘린 경쟁 커서·다음 줄로 접힌 선택지도 구멍을 연다 —
    /// 어휘(라벨 앞머리)로 면제하던 자리 전량을 구조 규칙이 닫는지 관문 화면에서 직접 잰다.
    #[test]
    fn allow_hole_closes_for_every_short_or_wrapped_competing_cursor() {
        let gs = gates();
        for (label, replacement) in [
            ("두 글자 잘림", "❯ No"),
            ("쉼표까지 잘림", "❯ No,"),
            ("한 글자", "❯ N"),
            ("번호만", "❯ 2"),
            ("다음 줄로 접힌 선택지", "❯\n  No"),
        ] {
            let screen = fixtures::FOLDER_TRUST.replace("\x20 2. No, exit", replacement);
            assert!(
                first_run_gates::identify(&gs, &screen).is_some_and(|g| g.id == GATE_FOLDER_TRUST),
                "{label}: 전제 붕괴 — 코퍼스가 이 화면을 폴더신뢰로 읽지 않는다\n{screen}"
            );
            assert!(
                !crate::readiness::modal_signature(&screen).is_some_and(|m| m.cursor_on_exit),
                "{label}: 전제 붕괴 — 부정 증거만으로 이미 닫혔다면 이 반례는 아무것도 재지 못한다"
            );
            assert!(
                !confirm_allowed(&obs(&screen, &gs), GATE_FOLDER_TRUST),
                "{label}: 잘린 경쟁 커서 화면에서 확인이 열렸다(그 Return 이 종료를 누른다)\n{screen}"
            );
            assert!(
                decide(&obs(&screen, &gs)).blocks(),
                "{label}: 주입 가드가 잘린 경쟁 커서 화면에 본문을 넣는다\n{screen}"
            );
        }
        // 질문 재출현 잔상(codex R4 반례 ②) — 앞선 미해소 커서도 블록 안이다.
        let ghost = format!("{}\n❯ No\n{}", trust_needle(&gs), fixtures::FOLDER_TRUST);
        assert!(!confirm_allowed(&obs(&ghost, &gs), GATE_FOLDER_TRUST), "질문 잔상 앞 커서가 면제됐다\n{ghost}");
        // 가용성 대조군 — 관문 문면 **이전**의 셸 잔상·꼬리 빈 프롬프트는 확인을 막지 않는다.
        for (label, screen) in [
            ("질문 앞 p10k 프롬프트", format!("~/work ╰─❯ ls -al\n{}", fixtures::FOLDER_TRUST)),
            ("질문 앞 부트 명령줄", format!("❯ claude --dangerously-skip-permissions\n{}", fixtures::FOLDER_TRUST)),
            ("꼬리 빈 입력 상자", format!("{}❯ \n", fixtures::FOLDER_TRUST)),
        ] {
            assert!(
                confirm_allowed(&obs(&screen, &gs), GATE_FOLDER_TRUST),
                "{label}: 자동확인이 닫혔다(가용성 붕괴)\n{screen}"
            );
        }
    }

    /// ★(리뷰 R2 · codex blocking) 접힌 종료 라벨(`No, ex⏎it` · CRLF)도 확인 경계를 닫는다 — 코퍼스 식별
    /// (평탄화)은 그 화면을 폴더신뢰로 읽으므로, 벨트가 접힘을 못 읽으면 `trust_send` 가 종료 위에 Return 을 쏜다.
    /// (성찰 R7: 생산자는 [`confirm_denied`] — 삭제된 allow 구멍이 아니다.)
    #[test]
    fn confirmation_stays_closed_when_the_exit_label_is_wrapped() {
        let gs = gates();
        let on_exit = with_cursor_on(fixtures::FOLDER_TRUST, 2);
        for wrapped in [
            on_exit.replace("No, exit", "No, ex\n   it"),
            on_exit.replace('\n', "\r\n").replace("No, exit", "No, ex\r\n   it"),
        ] {
            let g = first_run_gates::identify(&gs, &wrapped).expect("전제: 코퍼스가 접힌 폴더신뢰를 식별한다");
            assert_eq!(g.id, GATE_FOLDER_TRUST);
            assert_eq!(
                confirm_denied(&obs(&wrapped, &gs), GATE_FOLDER_TRUST),
                Some(ConfirmDenied::CursorOnExit),
                "접힌 `No, exit` 위 커서에 자동확인 Return 이 허용됐다(좌석 사망 경로)\n{wrapped}"
            );
            let blocked = confirm_denied(&obs(&wrapped, &gs), GATE_FOLDER_TRUST).is_some();
            assert!(!trust_send(&TrustObserved {
                hit: true, first: true, persisted: false, sends: 0, max_sends: 2,
                other_gate: blocked, legacy_v1: false,
            }));
        }
        // 대조군: 커서가 Yes 위인 접힌 화면은 확인이 열린다(자동확인 기능 보존 · 라이브락 방향 회귀 없음).
        let yes_wrapped = fixtures::FOLDER_TRUST.replace("Yes, I trust this folder", "Yes, I tru\n   st this folder");
        assert_eq!(confirm_denied(&obs(&yes_wrapped, &gs), GATE_FOLDER_TRUST), None);
    }

    /// ★(0.14.31 · 리뷰 R3 · codex blocking) 확인 경계는 **양성 증거**로만 열린다 — 커서가 코퍼스 액션 라벨
    /// (`Yes, I trust this folder`) 전문 위에 있을 때만. R2 의 부정 증거(`!cursor_on_exit`)는 **잘린 종료 라벨**을
    /// 통과시켰다: 화면이 여전히 폴더신뢰로 식별되므로 구멍이 열리고, 그 Return 이 종료 선택지를 눌러 좌석이
    /// 죽는다(2.1.261 기본 포커스 = `No, exit`). 여기 반례 넷 전부 **보류**여야 한다.
    #[test]
    fn confirmation_needs_the_cursor_on_the_gate_action_label() {
        let gs = gates();
        let on_exit = with_cursor_on(fixtures::FOLDER_TRUST, 2);
        // ── 반례: 종료 라벨이 잘려 `cursor_on_exit=false` 인 렌더들(전부 코퍼스는 폴더신뢰로 읽는다) ──
        let clipped_numbered = on_exit.replace("No, exit", "No, exi");
        let compact = on_exit.replace("❯ 2. No, exit", "❯ 2.No, exi"); // 점 뒤 공백 없음 — ⓓ 도 안 걸린다
        let unnumbered = on_exit.replace("❯ 2. No, exit", "❯ No, exi"); // 번호 소실 + 잘림
        let no_cursor = on_exit.replace("❯ 2. No, exit", "  2. No, exit"); // 커서 행 자체가 소실
        for (label, screen) in [
            ("잘린 번호 종료 라벨", &clipped_numbered),
            ("점 뒤 공백 없는 잘린 종료 라벨", &compact),
            ("번호 없는 잘린 종료 라벨", &unnumbered),
            ("커서 소실", &no_cursor),
        ] {
            let g = first_run_gates::identify(&gs, screen)
                .unwrap_or_else(|| panic!("{label}: 전제 붕괴 — 코퍼스가 이 화면을 폴더신뢰로 읽지 않는다\n{screen}"));
            assert_eq!(g.id, GATE_FOLDER_TRUST, "{label}");
            assert!(
                !crate::readiness::modal_signature(screen).is_some_and(|m| m.cursor_on_exit),
                "{label}: 전제 붕괴 — 종전 부정 증거만으로 이미 닫혔다면 이 반례는 아무것도 재지 못한다"
            );
            let denied = confirm_denied(&obs(screen, &gs), GATE_FOLDER_TRUST);
            assert!(
                matches!(denied, Some(ConfirmDenied::LabelUnresolved)),
                "{label}: 잘린/미관측 선택 증거에 자동확인 Return 이 허용됐다(좌석 사망 경로): {denied:?}\n{screen}"
            );
            // 조립도 같은 결론 — `other_gate`(=확인 거부) 가 참이면 `trust_send` 는 쏘지 않는다.
            let blocked = denied.is_some();
            assert!(!trust_send(&TrustObserved {
                hit: true, first: true, persisted: false, sends: 0, max_sends: 2,
                other_gate: blocked, legacy_v1: false,
            }), "{label}: 조립이 Return 을 쐈다");
        }
        // ── ★(리뷰 R3b · codex blocking) 모호한 화면: 액션 라벨 위 커서가 **있어도** 해소되지 않은 다른
        //    선택 커서가 함께 있으면 닫는다(잔상·부분 렌더 — 실제 선택이 종료 쪽일 수 있다) ──
        let both_cursors = fixtures::FOLDER_TRUST.replace("\x20 2. No, exit", "❯ 2. No, exi");
        assert!(
            first_run_gates::identify(&gs, &both_cursors).is_some_and(|g| g.id == GATE_FOLDER_TRUST),
            "전제 붕괴 — 코퍼스가 이 화면을 폴더신뢰로 읽지 않는다\n{both_cursors}"
        );
        assert!(
            !crate::readiness::modal_signature(&both_cursors).is_some_and(|m| m.cursor_on_exit),
            "전제 붕괴 — 종전 부정 증거만으로 이미 닫혔다면 이 반례는 아무것도 재지 못한다"
        );
        assert!(
            confirm_denied(&obs(&both_cursors, &gs), GATE_FOLDER_TRUST).is_some(),
            "모호한 두 커서 화면에서 자동확인 Return 이 허용됐다(좌석 사망 경로)\n{both_cursors}"
        );
        // ── 양성 대조군: 액션 라벨 전문 위 커서면 확인은 열린다(자동확인 기능 보존 · 라이브락 회귀 0) ──
        assert_eq!(confirm_denied(&obs(fixtures::FOLDER_TRUST, &gs), GATE_FOLDER_TRUST), None);
        // 접힌 액션 라벨(단어 안 줄바꿈)도 평탄화 공간에서 전문으로 읽힌다.
        let wrapped_yes = fixtures::FOLDER_TRUST.replace("Yes, I trust this folder", "Yes, I tru\n   st this folder");
        assert_eq!(confirm_denied(&obs(&wrapped_yes, &gs), GATE_FOLDER_TRUST), None);
        // ★H-2 결속: 2.1.261 형(기본 포커스 = 종료 · 액션 = 아래 1발) — 액션 **전**은 보류, 액션 **후**는 통과.
        //   벨트가 액션 뒤 화면까지 막으면 H-2 의 통과 경로가 통째로 죽는다(그 회귀를 여기서 잡는다).
        let v261_before = fixtures::FOLDER_TRUST
            .replace("❯ 1. Yes, I trust this folder", "  2. Yes, I trust this folder")
            .replace("\x20 2. No, exit", "❯ 1. No, exit")
            .replace(" 2. No, exit", "❯ 1. No, exit");
        assert!(v261_before.contains("❯ 1. No, exit"), "전제: 2.1.261 기본 포커스 재현\n{v261_before}");
        assert!(confirm_denied(&obs(&v261_before, &gs), GATE_FOLDER_TRUST).is_some(), "2.1.261 기본 포커스에서 확인이 열렸다");
        let v261_after = v261_before
            .replace("❯ 1. No, exit", "  1. No, exit")
            .replace("  2. Yes, I trust this folder", "❯ 2. Yes, I trust this folder");
        assert_eq!(
            confirm_denied(&obs(&v261_after, &gs), GATE_FOLDER_TRUST),
            None,
            "액션(아래 1발) 뒤 화면에서도 확인이 닫혀 있다 — WP-1 H-2 의 통과 경로가 죽는다\n{v261_after}"
        );
        // 액션 선언이 없는 관문은 확인이 열리지 않는다(순수 술어 직접 실행).
        let mut no_action = gs.iter().find(|g| g.id == GATE_FOLDER_TRUST).cloned().expect("폴더신뢰");
        assert!(action_label_selected(&no_action, fixtures::FOLDER_TRUST), "전제: 선언이 있으면 양성");
        no_action.action = None;
        assert!(!action_label_selected(&no_action, fixtures::FOLDER_TRUST), "액션 선언 부재인데 확인이 열렸다");
    }

    /// ★(리뷰 R2 · codex minor) 롤백 노브는 **자기 축만** 끈다 — `CYS_READINESS_V1=1`(readiness_legacy)은 WP-1 의
    /// 코퍼스 밖 모달 폴백만 종전(Send)으로 되돌리고, 종전부터 있던 코퍼스 가드는 U-14 노브(guard_off)가, 둘 다는
    /// 마스터가 되돌린다. 커서-종료 벨트는 readiness 노브에 열리지 않는다(조여지는 방향만).
    #[test]
    fn readiness_v1_switches_off_only_the_unidentified_modal_fallback() {
        let gs = gates();
        let clipped = clip_tail(fixtures::TRUST_ECHO_THEN_DISCLAIMER, 3);
        assert!(first_run_gates::identify(&gs, &clipped).is_none(), "전제: 코퍼스 밖 모달");
        let corpus_gate = fixtures::OAUTH_CODE;
        let corpus_id = first_run_gates::identify(&gs, corpus_gate).expect("전제: 코퍼스 관문").id.clone();
        let mk = |screen: &str, v1: bool, off: bool| -> Decision {
            decide(&Observed { screen, gates: &gs, awakened: Some(false), guard_off: off, readiness_legacy: v1, cli_versions: &[], version_pin_legacy: false })
        };
        let unknown = crate::readiness::MODAL_UNKNOWN_ID;
        // 기본(두 노브 0): 둘 다 보류.
        assert!(matches!(mk(&clipped, false, false), Decision::Hold(h) if h.id == unknown));
        assert!(matches!(mk(corpus_gate, false, false), Decision::Hold(h) if h.id == corpus_id));
        // V1 단독: 모달 폴백만 종전(Send · 관측도 없다 = 축 자체가 없던 판정) · 코퍼스 가드는 그대로 보류.
        assert_eq!(mk(&clipped, true, false), Decision::Send, "V1 이 WP-1 모달 폴백을 되돌리지 못한다(반쪽 롤백)");
        assert!(matches!(mk(corpus_gate, true, false), Decision::Hold(h) if h.id == corpus_id),
                "V1 이 종전 코퍼스 가드까지 껐다 — 리뷰어 4칸 진리표(lib.rs BLOCK-3 핀)와 모순");
        // guard_off 단독: 둘 다 관측 강등(SendObserved) — U-14 축의 종전 규약.
        assert!(matches!(mk(&clipped, false, true), Decision::SendObserved(h) if h.id == unknown));
        assert!(matches!(mk(corpus_gate, false, true), Decision::SendObserved(h) if h.id == corpus_id));
        // 마스터(둘 다): 모달 폴백은 Send(축 없음) · 코퍼스 가드는 SendObserved(종전 롤백 형태 그대로).
        assert_eq!(mk(&clipped, true, true), Decision::Send);
        assert!(matches!(mk(corpus_gate, true, true), Decision::SendObserved(_)));
        // 벨트: 확인 경계는 **어느 노브로도** 열리지 않는다(성찰 R7: 주입 가드의 allow 구멍은 삭제 — 커서-종료
        //   벨트는 [`confirm_denied`] 소유이고, 종전 구멍이 `guard_off` 로 관측 강등되던 진리표는 대조군
        //   `legacy_decide_allowing` 에만 남는다).
        let on_exit = with_cursor_on(fixtures::FOLDER_TRUST, 2);
        let belt = |v1: bool, off: bool| confirm_denied(
            &Observed { screen: &on_exit, gates: &gs, awakened: Some(false), guard_off: off, readiness_legacy: v1, cli_versions: &[], version_pin_legacy: false },
            GATE_FOLDER_TRUST,
        );
        for (v1, off) in [(false, false), (true, false), (false, true), (true, true)] {
            assert_eq!(
                belt(v1, off),
                Some(ConfirmDenied::CursorOnExit),
                "노브(v1={v1} · off={off})가 커서-종료 벨트를 열었다(2.1.261 좌석 사망 경로)"
            );
        }
        let legacy_belt = |v1: bool, off: bool| legacy_decide_allowing(
            &Observed { screen: &on_exit, gates: &gs, awakened: Some(false), guard_off: off, readiness_legacy: v1, cli_versions: &[], version_pin_legacy: false },
            Some(GATE_FOLDER_TRUST),
        );
        assert!(legacy_belt(true, false).blocks(), "계측: 구 구멍도 readiness 롤백에는 닫혀 있었다");
        assert!(matches!(legacy_belt(false, true), Decision::SendObserved(_)), "계측: 구 구멍은 마스터 롤백에 관측 강등됐다");
    }

    /// 확인 에코·정상 프롬프트는 모달 폴백에도 걸리지 않는다(2026-07-29 킬체인 역방향 · 부트 창 안에서도).
    #[test]
    fn confirmation_echo_and_healthy_prompts_pass_the_modal_fallback() {
        let gs = gates();
        let echo_then_welcome = format!("Yes, I trust this folder ✔\n{}", fixtures::HEALTHY_WELCOME_BOX);
        for screen in [
            "Yes, I trust this folder ✔\n",
            echo_then_welcome.as_str(),
            fixtures::LIVE_TUI_AT_PROMPT,
            fixtures::HEALTHY_WELCOME_BOX,
            fixtures::READY_SHELL,
            fixtures::CONFIG_THEME_SETTING,
            fixtures::ACCOUNT_STATUS_PANEL,
        ] {
            assert_eq!(decide(&obs(screen, &gs)), Decision::Send, "정상 화면에서 주입이 막혔다: {screen:?}");
        }
        // 살아 있는 권한 프롬프트는 부트 창 안에서 **막힌다** — 디렉티브가 권한 선택지에 붙여넣어지면 안 된다.
        assert!(decide(&obs(fixtures::LIVE_PERMISSION_PROMPT, &gs)).blocks());
        // 그리고 그 판정은 readiness 와 **같은 함수**의 결과다(판정 분리 금지의 in-band 확인).
        assert!(crate::readiness::modal_signature(fixtures::LIVE_PERMISSION_PROMPT).is_some());
        assert!(crate::readiness::modal_signature("Yes, I trust this folder ✔\n").is_none());
    }

    /// ★확인은 **선언된 키 시퀀스가 Return 한 발일 때만** 열린다(0.14.31 · 리뷰 R2 · codex major).
    ///
    /// 【무엇이 뚫려 있었는가】 R1 은 봉투의 `default_index: null`(= 운영자가 "기본 포커스를 모른다"
    /// 고 선언한 자리)을 보고서에 `down_presses: null` · `held_no_action` = **보류**라고 인쇄하면서,
    /// 정작 이 경계에서는 아무것도 하지 않았다 — 커서만 라벨 위면 Return 이 그대로 나갔다.
    /// 인쇄한 판정과 실제 키 경로가 다르면 그 인쇄는 운영자를 속인다.
    ///
    /// 【왜 `Some(0)` 인가】 이 조립(`cys.rs` 폴더신뢰 자동확인)은 **Down 을 보내지 않는다**.
    /// 그러므로 `down > 0` 선언과는 애초에 합의가 없고, 산출 불가(`None` — 미상 또는 목표가 기본
    /// 포커스보다 **위**)와도 합의가 없다. 이 항을 "Down 을 보내도 된다" 는 허가로 확대하면
    /// 이 벨트가 서 있는 전제(지금 화면의 커서)를 스스로 무너뜨린다(codex 설계 검토 ⑤).
    ///
    /// 【기본 경로 무영향】 빌트인 folder-trust 는 `default_index:Some(1)` + `action:(1,…)` →
    /// `Some(0)` 이다. 이 항은 **코퍼스가 스스로 모호해질 때만** 문다.
    #[test]
    fn confirm_needs_the_declared_sequence_to_be_a_bare_return() {
        let screen = fixtures::FOLDER_TRUST;
        // ① 정본(down=0) — 종전과 한 글자도 다르지 않다.
        let base = gates();
        assert_eq!(
            base.iter().find(|g| g.id == GATE_FOLDER_TRUST).unwrap().down_presses(),
            Some(0),
            "빌트인 전제가 바뀌었다 — 이 검체의 '기본 경로 무영향' 주장이 함께 무너진다"
        );
        assert!(confirm_allowed(&obs(screen, &base), GATE_FOLDER_TRUST));

        // ② 운영자가 "모른다"를 선언한다(`default_index: null`) → 보류. 그리고 **전송도 0**이다.
        let cleared = first_run_gates::resolve_with(
            Some(&serde_json::json!({"gates": [{"id": GATE_FOLDER_TRUST, "default_index": null}]})),
            true,
        )
        .gates;
        assert_eq!(
            confirm_denied(&obs(screen, &cleared), GATE_FOLDER_TRUST),
            Some(ConfirmDenied::SequenceNotBareReturn { down: None }),
            "선언된 무지가 확인을 막지 못한다(보고서만 '보류' 라고 인쇄하던 그 상태)"
        );

        // ③ 아래키가 필요한 선언(2.1.261 실측 형상 · default 0 · 목표 1) → 보류.
        let down1 = first_run_gates::resolve_with(
            Some(&serde_json::json!({"gates": [{"id": GATE_FOLDER_TRUST, "default_index": 0}]})),
            true,
        )
        .gates;
        assert_eq!(
            confirm_denied(&obs(screen, &down1), GATE_FOLDER_TRUST),
            Some(ConfirmDenied::SequenceNotBareReturn { down: Some(1) }),
            "Return 만 보내는 조립이 아래키 1회 선언과 합의했다고 판정한다"
        );

        // ④ 역방향(목표가 기본 포커스보다 위) → `checked_sub` 가 None → 같은 보류.
        let upward = first_run_gates::resolve_with(
            Some(&serde_json::json!({"gates": [
                {"id": GATE_FOLDER_TRUST, "default_index": 9},
            ]})),
            true,
        )
        .gates;
        assert_eq!(
            confirm_denied(&obs(screen, &upward), GATE_FOLDER_TRUST),
            Some(ConfirmDenied::SequenceNotBareReturn { down: None })
        );

        // ⑤ ★전송 관측 — 판정이 아니라 **조립의 산출**로 확인한다(진리표 소비자까지 닫혔는가).
        for (label, gs) in [("cleared", &cleared), ("down1", &down1), ("upward", &upward)] {
            let other_gate = !confirm_allowed(&obs(screen, gs), GATE_FOLDER_TRUST);
            for legacy_v1 in [false, true] {
                assert!(
                    !trust_send(&TrustObserved {
                        hit: true,
                        first: true,
                        persisted: false,
                        sends: 0,
                        max_sends: 2,
                        other_gate,
                        legacy_v1,
                    }),
                    "{label}: 합의하지 못한 시퀀스에 Return 이 나갔다(legacy_v1={legacy_v1})"
                );
            }
        }
    }

    /// ★화면이 **코드 정본의 사람 1회 관문**이면 코퍼스가 무엇을 선언했든 확인이 열리지 않는다.
    ///
    /// (0.14.31 · 리뷰 R2 — claude 적대 minor + codex 설계 검토 ①)
    ///
    /// 코퍼스 층위의 바닥(`first_run_gates::restore_human_only_builtin_floor`)은 id 와 needle 포함
    /// 으로 봉하지만, 그 둘 중 어느 것도 "이 화면이 로그인 화면인가" 의 충분조건이 아니다 —
    /// 정본 needle 과 포함관계가 **없는** 다른 문면(로그인 화면의 `3rd-party platform …` 줄)을
    /// needle 로 쓰면 코퍼스 층위를 빠져나간다(그 사실은 `first_run_gates` 쪽 검체가 박제한다).
    /// 그래서 마지막 그물은 **봉투가 손대지 못하는 코드 정본**으로 화면을 다시 보는 것이다.
    #[test]
    fn human_only_screen_seals_confirmation_whatever_the_corpus_declares() {
        let screen = fixtures::LOGIN_METHOD_2_1_261;
        // 코퍼스 층위를 빠져나가는 별칭 — 기계 통과 · 액션 라벨이 **화면 커서 위**(`❯ 1. Claude …`).
        let escape = serde_json::json!({"source": "replace", "gates": [{
            "id": "login-escape",
            "needles": ["3rd-party platform · Amazon Bedrock, Microsoft Foundry, or Vertex AI"],
            "widget": ["Claude account with subscription"],
            "passability": "machine", "default_index": 1,
            "action": {"select_index": 1, "label": "Claude account with subscription"},
        }]});
        let gs = first_run_gates::resolve_with(Some(&escape), true).gates;
        let hijacker = first_run_gates::identify(&gs, screen).expect("전제: 별칭이 화면을 가져간다");
        assert_eq!(hijacker.id, "login-escape", "전제가 깨졌다 — 코퍼스 층위 검체를 함께 볼 것");
        assert_eq!(hijacker.passability, Passability::Machine, "전제: 코퍼스 층위는 이것을 못 막았다");

        // 화면 층위 봉인이 그 자리를 닫는다 — **어느 id 를 물어도**.
        for asked in ["login-escape", GATE_FOLDER_TRUST, "login-method"] {
            assert_eq!(
                confirm_denied(&obs(screen, &gs), asked),
                Some(ConfirmDenied::HumanOnlyScreen("login-method".to_string())),
                "로그인 화면에서 확인이 열렸다(asked={asked})"
            );
        }
        // 롤백 노브로도 열리지 않는다(조이는 벨트는 어느 노브로도 열지 않는다).
        for (guard_off, readiness_legacy) in [(true, false), (false, true), (true, true)] {
            let o = Observed { screen, gates: &gs, awakened: Some(false), guard_off, readiness_legacy, cli_versions: &[], version_pin_legacy: false };
            assert!(
                !confirm_allowed(&o, "login-escape"),
                "노브({guard_off},{readiness_legacy})가 사람 1회 화면의 확인을 열었다"
            );
        }
        // 그리고 조립은 **0발**이다.
        assert!(!trust_send(&TrustObserved {
            hit: true, first: true, persisted: false, sends: 0, max_sends: 2,
            other_gate: !confirm_allowed(&obs(screen, &gs), GATE_FOLDER_TRUST), legacy_v1: false,
        }));
        // ★정상 관문(폴더신뢰)은 종전대로 열린다 — 봉인이 자동확인 자체를 죽이지 않았다.
        let base = gates();
        assert!(confirm_allowed(&obs(fixtures::FOLDER_TRUST, &base), GATE_FOLDER_TRUST));
    }

    /// ★(0.14.31 · WP-1 H-2 · **실측**) 2026-09-08 격리 계측에서 claude 2.1.261 이 실제로 그린
    /// **코퍼스 밖 모달**(커스텀 API 키 확인창 · 기본 포커스가 `No (recommended)` · 번호 없음)이
    /// 주입 가드에서도 보류된다.
    ///
    /// 【왜 이 검체가 필요한가】 H-1 의 모달 축은 그때까지 **파생 검체**로만 증명돼 있었다
    /// (관문 화면을 잘라 만든 것). 이 화면은 지어낸 것이 아니라 벤더가 실제로 그린 것이고,
    /// 코퍼스의 needle 도 위젯 라벨도 하나도 걸리지 않는데 **푸터 어휘만으로** 보류된다 —
    /// "코퍼스에 없는 새 관문에 Return 이 나가지 않는다" 의 실측 증거다.
    ///
    /// 【확인 허가도 닫힌다】 자동확인은 지목 관문으로 **식별**돼야 열리는데 이 화면은 미식별이다
    /// (`ConfirmDenied::Unidentified`). 번호 없는 선택 위젯이라 종전 `❯ N.` 축으로는 잡히지 않는
    /// 형상이기도 하다 — 그래서 푸터 축(ⓑ)이 유일한 그물이다.
    #[test]
    fn measured_2_1_261_vendor_modal_outside_the_corpus_is_held_and_never_confirmed() {
        let gs = gates();
        let screen = fixtures::CUSTOM_API_KEY_MODAL_2_1_261;
        assert!(first_run_gates::identify(&gs, screen).is_none(), "코퍼스가 이 화면을 관문으로 오탐했다");
        let sig = crate::readiness::modal_signature(screen).expect("실측 모달이 모달로 안 읽혔다");
        assert!(sig.kinds.contains(&"confirm-cancel-footer"), "푸터 축이 아니라 다른 축이 잡았다: {:?}", sig.kinds);
        assert!(!sig.cursor_on_exit, "`No (recommended)` 는 종료 라벨 전문이 아니다(어휘 과확장 금지)");
        match decide(&obs(screen, &gs)) {
            Decision::Hold(h) => assert_eq!(h.id, crate::readiness::MODAL_UNKNOWN_ID),
            other => panic!("실측 벤더 모달에 주입이 허용됐다: {other:?}"),
        }
        assert_eq!(
            confirm_denied(&obs(screen, &gs), GATE_FOLDER_TRUST),
            Some(ConfirmDenied::Unidentified),
            "미식별 화면에서 확인이 열렸다"
        );
    }

    /// ★TRIAGE(R5-WP1-H2-corpus · 독립 재유도 2026-09-08 · reviewer-codex blocking) —
    /// **좌석이 스스로 밝힌 미실측 버전에서도 확인이 열린다.**
    ///
    /// 【정본】 §4 WP-1 H-2: "미실측은 `MEASURED_ON` 미일치 상태로 남겨 **보류되게 한다**"
    /// · §7 봉인표 ④(전 pane 사망 / 코퍼스 액션 오측정)의 봉인이 "미실측 관문은 핀 미갱신
    /// (**보류 유지**)" 이다. 그런데 확인 경계는 버전을 판정 재료로 **한 번도** 보지 않는다
    /// (`confirm_denied` :373~ · [`first_run_gates::ACTION_POLICY_IS_ENFORCED`] = false).
    /// 그래서 인쇄된 `held_version_drift` 와 실제 키 경로가 동시에 성립한다.
    ///
    /// 【왜 이 형상인가 — 버전을 받을 API 가 없다는 반론에 대해】 이 검체는 새 인자를 요구하지
    /// 않는다. 좌석은 **자기 화면에** 배너로 버전을 찍고, 코퍼스는 그것을 뽑는 순수 함수를 이미
    /// 소유한다([`first_run_gates::parse_cli_version`]). 즉 "지금 이 좌석이 도는 바이너리" 의
    /// 증거가 판정 입력(`Observed::screen`) 안에 이미 들어와 있는데도 쓰이지 않는다.
    ///
    /// 【실측 근거】 이 기계의 라이브 claude 는 2.1.263 이고 코퍼스 실측본은 2.1.241 이다.
    /// 정본 §4 H-2 는 2.1.261 폴더신뢰의 기본 포커스가 `0="No, exit"` 라고 적는다 — 즉 이
    /// 코퍼스가 선언한 `default_index: Some(1)`(=Return 한 발이 안전)은 그 버전에서 **거짓**이다.
    /// 오늘 좌석을 지키는 것은 버전 핀이 아니라 커서 벨트 하나뿐이다.
    #[test]
    fn confirm_is_denied_when_the_screen_declares_a_version_the_corpus_never_measured() {
        let gs = gates();
        let g = gs.iter().find(|g| g.id == GATE_FOLDER_TRUST).expect("코퍼스에 folder-trust");

        // 좌석이 자기 배너로 밝힌 버전 — 코퍼스 실측본과 다르다(라이브 실측값 2.1.263).
        let drifted = format!("Welcome to Claude Code v2.1.263\n{}", fixtures::FOLDER_TRUST);
        assert_eq!(
            first_run_gates::parse_cli_version(&drifted).as_deref(),
            Some("2.1.263"),
            "전제: 화면에서 버전을 뽑을 수 있다(이 증거는 이미 판정 입력 안에 있다)"
        );
        assert_ne!("2.1.263", g.measured_on.as_str(), "전제: 실측본과 다른 버전이다");
        assert!(
            !first_run_gates::action_policy(g, Some("2.1.263")).is_allowed(),
            "전제: 버전 핀은 이 조합을 **보류**로 판정한다(보고서가 그렇게 인쇄한다)"
        );

        // 그런데 확인 경계는 그 보류를 집행하지 않는다 — Return 이 나간다.
        assert!(
            confirm_denied(&obs(&drifted, &gs), GATE_FOLDER_TRUST).is_some(),
            "화면이 미실측 버전을 스스로 밝히는데 확인이 열렸다 — 인쇄된 held_version_drift 와 \
             실제 키 경로가 다르다(정본 §4 H-2 '미실측은 보류' · §7 봉인표 ④)"
        );

        // 그리고 조립의 산출까지 0발이어야 한다(판정만 고치고 소비자를 두면 반쪽이다).
        let other_gate = !confirm_allowed(&obs(&drifted, &gs), GATE_FOLDER_TRUST);
        for legacy_v1 in [false, true] {
            assert!(
                !trust_send(&TrustObserved {
                    hit: true,
                    first: true,
                    persisted: false,
                    sends: 0,
                    max_sends: 2,
                    other_gate,
                    legacy_v1,
                }),
                "미실측 버전 화면에 Return 이 나갔다(legacy_v1={legacy_v1})"
            );
        }
    }

    /// ★(0.14.31 · 독립 재유도 H2-B · codex 설계 검토 3) 버전 축의 **증거 규칙**.
    ///
    /// 위 검체(`confirm_is_denied_when_the_screen_declares_a_version_the_corpus_never_measured`)는
    /// "불일치 배너가 화면에 있을 때" 하나를 잰다. 여기서는 그 축이 **어떤 증거로 서는지**를 잰다:
    ///   ① 미상(배너 없음) → **오늘은 통과한다**(부분 실측 코퍼스로 가용성을 끊지 않는다 ·
    ///      별도 결정 · [`first_run_gates::ACTION_POLICY_ENFORCEMENT`] doc) ·
    ///   ② 기동 래치만 불일치(배너가 화면에서 밀려난 뒤) → **보류**. 이것이 없으면 드리프트 거부는
    ///      다음 틱에 저절로 풀린다 — 배너는 관문이 그려지면 화면 밖으로 나간다 ·
    ///   ③ 래치는 일치인데 화면 뒤쪽에 불일치 배너 → **보류**(증거는 합집합이다) ·
    ///   ④ 둘 다 일치 → 통과(이 수리가 자동확인 기능 자체를 죽이지 않는다) ·
    ///   ⑤ 앵커 없는 `--version` 형상은 이 경계에서 **증거가 아니다**(좌석이 스스로 찍은 배너만
    ///      증거다). 그래서 ⑤는 미상으로 접혀 통과한다 — 이 결정을 바꾸려면 이 줄을 먼저 고쳐라.
    ///   ⑥ 롤백 노브 셋 중 어느 것으로도 열리지 않는다(조이는 벨트의 규율 · CONTRACTS B-7).
    #[test]
    fn version_axis_holds_on_any_drifting_evidence_but_unknown_still_passes() {
        let gs = gates();
        let measured = gs
            .iter()
            .find(|g| g.id == GATE_FOLDER_TRUST)
            .expect("코퍼스에 folder-trust")
            .measured_on
            .clone();
        let drift = "9.9.9";
        assert_ne!(drift, measured, "전제: 실측본과 다른 버전");
        let denied = |screen: &str, latch: &[String]| -> Option<ConfirmDenied> {
            confirm_denied(
                &Observed {
                    screen,
                    gates: &gs,
                    awakened: Some(false),
                    guard_off: false,
                    readiness_legacy: false,
                    cli_versions: latch,
                    version_pin_legacy: false,
                },
                GATE_FOLDER_TRUST,
            )
        };
        let latch = |v: &str| vec![v.to_string()];
        let banner = |v: &str| format!("Welcome to Claude Code v{v}\n{}", fixtures::FOLDER_TRUST);

        // ① 미상 — 배너가 없으면 종전대로 열린다(오늘의 결정).
        assert_eq!(denied(fixtures::FOLDER_TRUST, &[]), None, "미상에서 확인이 닫혔다(가용성 절단)");

        // ② 래치만 불일치 — **화면에는 배너가 없다**(밀려난 상태). 그래도 닫힌다.
        assert_eq!(
            denied(fixtures::FOLDER_TRUST, &latch(drift)),
            Some(ConfirmDenied::VersionDrift {
                measured_on: measured.clone(),
                detected: drift.to_string(),
            }),
            "배너가 화면에서 밀려나자 드리프트 거부가 풀렸다 — 거부가 한 틱짜리면 없는 것과 같다"
        );

        // ③ 래치는 일치인데 화면 뒤쪽에 불일치 배너 — 합집합이므로 닫힌다.
        let two = format!("Welcome to Claude Code v{measured}\n(중략)\n{}", banner(drift));
        assert!(
            denied(&two, &latch(&measured)).is_some(),
            "앞선 일치 배너가 뒤의 불일치를 덮었다 — 증거는 합집합이어야 한다"
        );

        // ③' ★(수렴 R2 · codex major) **래치 안에서도** 합집합이다 — 일치 하나를 먼저 잡은 뒤
        //     불일치를 관측했고 그 뒤 화면에서 배너가 사라져도 거부는 유지된다.
        assert!(
            denied(
                fixtures::FOLDER_TRUST,
                &[measured.clone(), drift.to_string()]
            )
            .is_some(),
            "먼저 잡힌 일치가 나중에 관측한 불일치를 덮었다 — 시간축의 증거가 소멸한다"
        );

        // ④ 둘 다 일치 — 자동확인은 그대로 산다.
        assert_eq!(denied(&banner(&measured), &latch(&measured)), None, "일치인데 확인이 닫혔다");

        // ⑤ `--version` stdout 형상(앵커 없음)은 이 경계의 증거가 아니다 → 미상 → 통과.
        //    ★좁힌 판독기의 대가를 정직하게 박제한다: 이 줄이 초록인 동안 "화면 첫 점숫자" 는
        //      좌석의 버전 선언으로 쓰이지 않는다([`first_run_gates::banner_versions`] doc).
        let stdout_shape = format!("{drift} (Claude Code)\n{}", fixtures::FOLDER_TRUST);
        assert_eq!(denied(&stdout_shape, &[]), None);

        // ⑥ **보류를 푸는 형제 노브**로는 열리지 않는다(조이는 벨트의 규율 · CONTRACTS B-7).
        for (guard_off, readiness_legacy) in [(true, false), (false, true), (true, true)] {
            let o = Observed {
                screen: &banner(drift),
                gates: &gs,
                awakened: Some(false),
                guard_off,
                readiness_legacy,
                cli_versions: &[],
                version_pin_legacy: false,
            };
            assert!(
                !confirm_allowed(&o, GATE_FOLDER_TRUST),
                "노브({guard_off},{readiness_legacy})가 미실측 버전 화면의 확인을 열었다"
            );
        }

        // ⑦ ★(수렴 R2) 그러나 **이 축 자신의 노브**(`version_pin_legacy`)로는 열린다 —
        //    그리고 그것이 이 축이 마스터에 접히는 방식이다. 근거: 이 보류는 재실측 전 기계의
        //    **기본 상태**라, 노브가 없으면 운영자가 쥘 손잡이는 마스터뿐이고 마스터는 보류를
        //    close 로 강등한다(= BLOCK-4 '엄격 + 즉시 close').
        let rolled_back = Observed {
            screen: &banner(drift),
            gates: &gs,
            awakened: Some(false),
            guard_off: false,
            readiness_legacy: false,
            cli_versions: &latch(drift),
            version_pin_legacy: true,
        };
        assert_eq!(
            confirm_denied(&rolled_back, GATE_FOLDER_TRUST),
            None,
            "버전 축 롤백이 듣지 않는다 — 드리프트가 기본인 기계에서 운영자에게 남는 손잡이는 \
             마스터뿐이고, 마스터는 이 보류를 close 로 바꾼다"
        );
        // 그리고 그 롤백 상태에서 조립은 실제로 **1발을 낸다**(종전 복귀가 반쪽이 아니다).
        assert!(
            trust_send(&TrustObserved {
                hit: true,
                first: true,
                persisted: false,
                sends: 0,
                max_sends: 2,
                other_gate: !confirm_allowed(&rolled_back, GATE_FOLDER_TRUST),
                legacy_v1: false,
            }),
            "버전 축을 되돌렸는데 Return 이 여전히 0발이다 — 좌석은 관문에 서고 보류는 close 로 \
             강등되는 조합(reviewer-claude major)이 그대로 남는다"
        );
        // ⑧ 축 노브의 **판독 규약**(형제 게이트와 같은 엄격 비교 — 오타로 벨트가 조용히 열리지 않는다).
        assert!(version_pin_legacy_from(Some("0")));
        for loose in [None, Some(""), Some("1"), Some("true"), Some("off"), Some(" 0")] {
            assert!(!version_pin_legacy_from(loose), "느슨한 값 {loose:?} 이 버전 축을 껐다");
        }
        assert_eq!(ENV_VERSION_PIN, "CYS_GATE_VERSION_PIN");
    }

    /// ★(0.14.31 · 독립 재유도 H2-B · 수렴 R2) 기동 래치의 **갱신 규칙** — 이 규칙이 없으면
    /// 드리프트 거부가 한 틱짜리가 된다(배너가 관문 렌더에 밀려나는 순간 미상으로 열린다).
    ///
    /// 【R1 판이 무엇을 못박았고 왜 고쳤나】 R1 은 값 **하나**를 sticky 로 들었다("한 번 잡으면
    /// 바꾸지 않는다"). 두 리뷰어가 같은 구멍을 서로 다른 형상으로 재현했다:
    ///   · reviewer-claude major — 틱1 델타는 비어 있고(alt-screen 기본) 화면에는 **이전 좌석의
    ///     잔상 배너**(실측본과 같은 값)가 있다 → 래치가 그 값으로 굳는다 → 틱N 델타에 실린
    ///     진짜 버전은 sticky 라 무시된다 → 확인 시점엔 배너가 밀려나 증거 합집합={실측본}
    ///     → **Return 이 나간다**(좌석은 미실측 버전인데).
    ///   · codex major — 일치 래치 → 이후 틱 델타에 불일치 관측 → 다음 틱 화면에서 배너 소멸.
    ///     같은 귀결이다(관측한 불일치가 시간축에서 소멸한다).
    /// 그래서 래치는 **단조 증가하는 합집합**이 됐다. sticky 가 지키려던 성질(판정이 틱마다
    /// 뒤집히지 않을 것)은 그대로다 — 집합은 커지기만 하고, 커지는 방향은 언제나 '조이는' 쪽이다.
    #[test]
    fn seat_version_latch_accumulates_every_observed_version_and_never_forgets() {
        let banner = "Welcome to Claude Code v2.1.263\n";
        // ① 처음엔 델타에서 잡는다.
        assert_eq!(
            latch_seat_versions(vec![], banner, fixtures::FOLDER_TRUST),
            vec!["2.1.263".to_string()]
        );
        // ② 델타에 없으면 화면에서도 잡는다(alt-screen 좌석의 유일한 증거일 수 있다).
        assert_eq!(latch_seat_versions(vec![], "", banner), vec!["2.1.263".to_string()]);
        // ③ 둘 다 배너가 없으면 미상 그대로(추정 금지).
        assert!(latch_seat_versions(vec![], "", fixtures::FOLDER_TRUST).is_empty());
        // ④ ★한 번 들어온 값은 사라지지 않는다(단조) — 배너가 화면에서 밀려나도.
        let held = vec!["2.1.241".to_string()];
        assert_eq!(
            latch_seat_versions(held.clone(), "", fixtures::FOLDER_TRUST),
            held,
            "래치가 흔들리면 같은 좌석의 판정이 틱마다 뒤집힌다"
        );
        // ⑤ ★그리고 **덮지 않는다** — 나중에 관측한 다른 버전이 합류한다(수렴 R2 의 핵심).
        assert_eq!(
            latch_seat_versions(held.clone(), banner, banner),
            vec!["2.1.241".to_string(), "2.1.263".to_string()],
            "먼저 잡힌 값이 나중의 관측을 덮으면 드리프트 거부가 영구히 무력해진다"
        );
        // ⑥ 상한이 있다(병적 입력에서 무한 성장 금지) — 그리고 상한에 닿기 전에 이미 보류다.
        let many: String = (0..20).map(|i| format!("Claude Code v9.9.{i}\n")).collect();
        assert_eq!(latch_seat_versions(vec![], &many, "").len(), SEAT_VERSION_LATCH_MAX);

        let gs = gates();
        // ⑦ 그리고 그 래치는 확인 경계에서 **실제로 문다**(배너가 화면에서 사라진 뒤에도).
        let latched = latch_seat_versions(vec![], banner, "");
        let o = Observed {
            screen: fixtures::FOLDER_TRUST, // 화면에는 배너가 없다
            gates: &gs,
            awakened: Some(false),
            guard_off: false,
            readiness_legacy: false,
            cli_versions: &latched,
            version_pin_legacy: false,
        };
        assert!(
            !confirm_allowed(&o, GATE_FOLDER_TRUST),
            "래치가 물지 않으면 이 규칙은 장식이다"
        );
    }

    /// ★(0.14.31 · 수렴 R2 · reviewer-claude major 실측 v1) **틱 시퀀스 재현** — 부트 루프가
    /// 실제로 부르는 순서 그대로 잰다(잔상 화면 → 진짜 배너 → 관문 화면).
    ///
    /// 【발동 조건】 같은 pane 을 재사용하며 그 사이 claude 를 업그레이드한 첫 부트:
    /// 잔상 버전 == `MEASURED_ON` · 실행 버전 != `MEASURED_ON` · 기동 send 직전에 `since_line` 을
    /// 잡으므로 첫 틱의 델타는 비어 있는 것이 **정상**이다(claude 는 alt-screen 기본).
    #[test]
    fn stale_screen_residue_cannot_shadow_the_real_banner_seen_later_in_the_delta() {
        let gs = gates();
        let measured = gs
            .iter()
            .find(|g| g.id == GATE_FOLDER_TRUST)
            .expect("코퍼스에 folder-trust")
            .measured_on
            .clone();
        let live = "2.1.263";
        assert_ne!(live, measured, "전제: 라이브 버전이 실측본과 다르다");

        // 틱1 — 델타는 비어 있고 화면은 **이전 좌석의 잔상**(실측본과 같은 배너).
        let residue = format!("Welcome to Claude Code v{measured}\n❯ ");
        let mut seat = latch_seat_versions(vec![], "", &residue);
        assert_eq!(seat, vec![measured.clone()], "전제: 잔상이 래치에 든다(그 자체는 정상)");

        // 틱N — 누적 델타에 **진짜 배너**가 실린다.
        let real = format!("Welcome to Claude Code v{live}\n");
        seat = latch_seat_versions(seat, &real, fixtures::FOLDER_TRUST);

        // 확인 시점 — 화면은 관문이고 배너는 밀려났다(증거는 래치뿐).
        let o = Observed {
            screen: fixtures::FOLDER_TRUST,
            gates: &gs,
            awakened: Some(false),
            guard_off: false,
            readiness_legacy: false,
            cli_versions: &seat,
            version_pin_legacy: false,
        };
        assert_eq!(
            confirm_denied(&o, GATE_FOLDER_TRUST),
            Some(ConfirmDenied::VersionDrift {
                measured_on: measured.clone(),
                detected: live.to_string(),
            }),
            "잔상 배너가 진짜 버전을 가렸다 — 좌석은 {live} 인데 Return 이 나간다(H2-B 확정 결함 재현)"
        );
        // 조립까지 0발이어야 한다(판정만 고치고 소비자를 두면 반쪽이다).
        assert!(
            !trust_send(&TrustObserved {
                hit: true,
                first: true,
                persisted: false,
                sends: 0,
                max_sends: 2,
                other_gate: !confirm_allowed(&o, GATE_FOLDER_TRUST),
                legacy_v1: false,
            }),
            "미실측 버전 좌석에 Return 이 나갔다"
        );
    }
}
