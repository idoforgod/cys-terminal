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
//!
//! 둘 다 **엄격 비교**다(`== Some("0")` / `== Some("1")`) — 형제 게이트
//! (`CYS_GATE_PENDING_CLOSE`·`CYS_READINESS_V1`)와 같은 규율로, 오타 하나로 안전장치가 조용히
//! 뒤집히는 것을 막는다. env 를 읽는 지점은 축마다 **함수 하나**뿐이고 판정은 순수 코어에 있다.
//! 두 스위치를 다 켜면 이 단위 착지 이전 동작으로 완전 복귀한다.

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
pub fn decide(o: &Observed) -> Decision {
    decide_allowing(o, None)
}

/// 위의 일반형 — `allow_gate_id` 로 지목한 관문 **하나**는 통과 대상으로 본다.
///
/// ★왜 예외 구멍이 필요한가: 폴더신뢰 자동확인(U-15)은 **바로 그 관문을 통과시키려고** 키를
///   보내는 동작이다. 예외가 없으면 자기 자신이 자기를 막아 자동확인 기능이 통째로 죽는다.
///   반대로 예외를 '관문 전체' 로 열면 킬체인이 그대로 돌아온다 — 그래서 구멍은 **id 하나**이고,
///   그 id 가 아닌 관문이 화면에 있으면(= 이미 다음 화면으로 넘어갔으면) 그대로 보류한다.
///   실측 킬체인의 형태가 정확히 그것이다(신뢰 창인 줄 알고 눌렀는데 면책 창이었다).
pub fn decide_allowing(o: &Observed, allow_gate_id: Option<&str>) -> Decision {
    // ★생애 창 상한 — 창이 닫혔거나(각성 완료) 재지 못했으면 스캔 자체를 하지 않는다.
    //   여기서 일찍 반환하는 것이 비용 방어이기도 하다(호출부가 화면 RPC 를 아예 생략한다).
    if o.awakened != Some(false) {
        return Decision::Send;
    }
    let Some(g) = first_run_gates::identify(o.gates, o.screen) else {
        return Decision::Send;
    };
    if allow_gate_id == Some(g.id.as_str()) {
        return Decision::Send;
    }
    let hit = GateHit {
        id: g.id.clone(),
        title: g.title.clone(),
        human_only: g.passability == Passability::HumanOnly,
    };
    if o.guard_off {
        Decision::SendObserved(hit)
    } else {
        Decision::Hold(hit)
    }
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
    /// ★U-14 축: 지금 화면에 폴더신뢰가 **아닌** 관문이 떠 있다.
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

    fn obs<'a>(screen: &'a str, gs: &'a [Gate]) -> Observed<'a> {
        Observed {
            screen,
            gates: gs,
            awakened: Some(false),
            guard_off: false,
        }
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
        // 관문 코퍼스가 비면 축이 통째로 없다 = 종전 동작.
        assert_eq!(decide(&obs(fixtures::FOLDER_TRUST, &[])), Decision::Send);
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

    /// 예외 구멍은 **id 하나**다 — 그 관문만 통과하고 나머지는 그대로 보류한다.
    #[test]
    fn allow_hole_is_exactly_one_gate_and_never_the_disclaimer() {
        let gs = gates();
        assert_eq!(
            decide_allowing(&obs(fixtures::FOLDER_TRUST, &gs), Some(GATE_FOLDER_TRUST)),
            Decision::Send,
            "자동확인 대상 관문까지 막으면 폴더신뢰 자동확인이 통째로 죽는다"
        );
        // ★킬체인 화면: 신뢰 에코가 남아 있어도 면책 창이므로 구멍이 열리지 않는다.
        match decide_allowing(
            &obs(fixtures::TRUST_ECHO_THEN_DISCLAIMER, &gs),
            Some(GATE_FOLDER_TRUST),
        ) {
            Decision::Hold(h) => assert_eq!(h.id, "bypass-disclaimer"),
            other => panic!("면책 창에 예외 구멍이 열렸다(킬 스텝): {other:?}"),
        }
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
        let run = |guard_off: bool, legacy_v1: bool| -> (u32, bool) {
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
                let other_gate = decide_allowing(&o, Some(GATE_FOLDER_TRUST)).blocks();
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
            let (sends, touched) = run(guard_off, legacy_v1);
            assert_eq!(sends, 1, "{why}: Return 이 {sends}발 나갔다(기대 1발)");
            assert!(!touched, "{why}: 면책 창에 Return 이 닿았다 — 좌석이 rc 1 로 죽는 경로");
        }

        // ④ ★계측 타당성 대조군 — 두 축을 **모두** 되돌리면 결함이 재현된다(2발 · 면책 접촉).
        //    재현되지 않으면 이 검체는 '원래 안 나는 일을 안 난다고 확인' 하는 공허한 검사다.
        let (legacy_sends, legacy_touch) = run(true, true);
        assert_eq!(legacy_sends, 2, "구 정책이 2발을 쏘지 않는다 — 킬체인 서사가 틀렸다(계측 무효)");
        assert!(legacy_touch, "구 정책이 면책 창에 닿지 않는다 — 결함 재현 실패(계측 무효)");
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
}
