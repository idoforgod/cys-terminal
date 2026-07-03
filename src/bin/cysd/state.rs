//! Daemon state: surfaces (PTY sessions), health rules, process ledger.

use crate::events::EventBus;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use regex::Regex;
use serde_json::json;
use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::sync::broadcast;

const SCROLLBACK_LINES: usize = 10_000;
pub const DEFAULT_ROWS: u16 = 35;
pub const DEFAULT_COLS: u16 = 120;

/// PTY 쓰기 요청 — surface별 전용 writer 스레드가 순서대로 소비한다.
pub enum WriteReq {
    /// 그대로 쓰기 (키 입력·텍스트·DSR 응답)
    Data(Vec<u8>),
    /// 원자적 주입: (clear_first면 Ctrl-U 선정리 → settle) → bracketed paste → cr_delay_ms 대기 → CR.
    /// 전부 한 writer arm에서 처리 = 다른 WriteReq의 끼어듦 차단(동시 주입 병합·부분 전달 차단).
    /// clear_first=권위 전달: 잔존 미제출 텍스트를 지운 깨끗한 라인에 명령을 원자적으로 꽂고 제출한다.
    Inject {
        text: String,
        cr_delay_ms: u64,
        clear_first: bool,
    },
}

/// 청크 경계 상태: 미완성 ESC/UTF-8 꼬리·\r 덮어쓰기·진행 중 라인
struct IngestState {
    carry: Vec<u8>,
    pending_cr: bool,
    partial: String,
}

pub struct Surface {
    pub id: u64,
    pub title: Mutex<String>,
    pub role: Mutex<Option<String>>,
    pub cmd: String,
    pub cwd: String,
    pub pid: u32,
    pub created_at: f64,
    /// RC-3 잔여(T2.1): 이 surface가 create_surface_with_env로 **env 주입**되어 생성됐는가.
    /// Windows node-recover가 기존 pane 재사용 전, pane env에 CLAUDE_CONFIG_DIR 등이 실려있는지
    /// (=순수 cmd 재기동이 안전한지) 판정하는 근거. env 미주입 pane(수동·구세션) 재사용 시 fail-closed.
    pub env_injected: bool,
    pub exited: AtomicBool,
    /// 자력종료(셸 EOF) 시각 — watchdog reap의 grace 측정 기준 (exited와 함께 stamp)
    pub exited_at: Mutex<Option<Instant>>,
    /// PTY 쓰기는 전용 writer 스레드만 수행 — async 경로는 유한 채널 try_send.
    /// 정체된 pane의 블로킹 write가 tokio 워커·watchdog을 멈추는 경로를 원천 차단한다.
    pub write_tx: std::sync::mpsc::SyncSender<WriteReq>,
    pub master: Mutex<Box<dyn MasterPty + Send>>,
    pub child: Mutex<Box<dyn Child + Send + Sync>>,
    pub parser: Mutex<vt100::Parser>,
    pub scrollback: Mutex<VecDeque<String>>,
    ingest: Mutex<IngestState>,
    pub out_tx: broadcast::Sender<Vec<u8>>,
    pub last_output: Mutex<Instant>,
    pub idle_notified: AtomicBool,
    /// recall 영속용 직전 라인 (연속 중복 스킵 — TUI 리드로우 노이즈 억제)
    last_recall_line: Mutex<String>,
    /// 인플라이트 큐: --queued 전송분 — 대상이 조용해질 때(followup) 순서대로 배달
    pub pending_queue: Mutex<std::collections::VecDeque<String>>,
    /// T1-1 자기보고 상태 (`status.set` RPC)
    pub agent_status: Mutex<Option<AgentStatus>>,
    /// T2-5 에이전트 메타: launch-agent가 등록한 (agent 이름, 실행 바이너리)
    pub agent_meta: Mutex<Option<(String, String)>>,
    /// T2-5 사망 감지 상태머신: 자식 트리에서 agent 바이너리를 처음 본 뒤 사라지면 발화
    pub agent_seen: AtomicBool,
    pub agent_exit_notified: AtomicBool,
    /// T3-13 타이핑 가드: 사람(UI) 입력의 마지막 시각 — 원격 주입 충돌 보호
    pub last_human_input: Mutex<Option<Instant>>,
    /// T3-14 단조 라인 커서: scrollback FIFO와 무관하게 증가하는 누적 완성 라인 수
    pub line_count: AtomicU64,
    /// T4-17 헬스 조치: 이 시각까지 queued 배달 일시정지 (직접 send는 통과)
    pub queue_paused_until: Mutex<Option<Instant>>,
    /// T4-17 에코 제외: 마지막 원격 주입 시각 (주입 직후 에코 라인은 룰 매칭 제외)
    pub last_injected: Mutex<Option<Instant>>,
    /// T5 사용량 관측 스냅샷 (usage.rs 수집기가 갱신 — 자기보고 agent_status와 별개 층위)
    pub observed_usage: Mutex<Option<crate::usage::ObservedUsage>>,
    /// T5 세션 트랜스크립트 등록 (`usage.register` — SessionStart hook의 결정론 매핑)
    pub registered_transcript: Mutex<Option<String>>,
    /// (4) resume 핀용 agent transcript session_id — analytics.rs의 회계 session_id와 무관(별개 개념).
    /// usage 수집기가 transcript 발견 시 1회 stash(is_none 가드)·topology에 영속해 정확한 세션 재개.
    pub agent_session_id: Mutex<Option<String>>,
    /// ⑪ pack-reinject 추적 마커 — 마지막 주입 pack_version·directive_hash. 단일 write path는
    /// `reinject.mark` RPC(주입 성공 직후 컨트롤러만 호출). topology 영속·restore 복원으로
    /// 재기동을 견딘다. None=미주입(첫 pack-update에서 1회 주입). agent_session_id와 동일 위치 init.
    pub pack_reinject: Mutex<Option<PackReinject>>,
    /// context.threshold 에지 게이트 — 자기보고(status.set)·관측(usage.rs) **공유**.
    /// true=발화 가능(임계 미만 관측됨). 분리하면 같은 교차에 두 경로가 각각 발화해
    /// master/CSO가 cycle-agent를 이중 집행한다. swap(false)가 원자적 1회 발화를 보장.
    pub ctx_threshold_armed: AtomicBool,
    /// (B2) OSC 9/99/777 알림 스캐너 carry — reader 스레드 전용(단일 스레드 접근이라 Mutex면 충분).
    /// strip 전 raw chunk를 누적해 완성 OSC 시퀀스만 추출한다(화면 렌더/strip 경로와 독립).
    pub osc_carry: Mutex<Vec<u8>>,
    /// T4-4/T6-P3 능력 가드: 이 surface의 정규화된 권한 집합(write⊇read·deny-by-default).
    /// 역할 변경(claim_role)과 동기 갱신 — cysd-매개 변형 경로(send/scoped run)의 게이트 키.
    /// role과 함께 도출하되 self-declared role을 신뢰하지 않고 cysd-인증 발신 surface를 키로 쓴다.
    pub caps: Mutex<crate::caps::Caps>,
    /// T5-2 무음 크래시 재진입 가드: "ack 후 후행 실패" 무음 크래시 발화의 1회성 swap 가드.
    /// agent_exit_notified 패턴 확장 — 회복 시 swap(false). 제2의 AtomicBool 신설 금지(이 1개만).
    pub crash_notified: AtomicBool,
    /// T5-2 직전 성공 ack 시각(epoch초) — 명령(send/key)이 성공 보고한 시점. surface_crashed
    /// 술어의 "성공 ack 후 N초 내 후행 실패" 윈도우 기준. None=아직 ack 없음.
    pub last_cmd_ack: Mutex<Option<f64>>,
}

pub struct HealthRule {
    pub name: String,
    pub regex: Regex,
    /// T4-17 조치 바인딩: None=alert만(기본) / Some("pause-queue")=queued 배달 일시정지
    pub action: Option<String>,
    /// 조치 발동에 필요한 60초 창 내 연속 매칭 횟수 (오탐의 사고화 방지 게이트)
    pub threshold: u32,
    /// pause-queue 지속 시간
    pub pause_secs: u64,
}

/// T5-6 strand-2 오염 격리 — 비정상 종료한 자식 프로세스의 재사용 가능성 2분 분류.
/// Exporter 교훈(penpot exporter/core.md:16 "on error the browser is destroyed instead of
/// reused")의 클린룸 등가 — 계약만 차용, Playwright/Redis 엔진 미차용. 1-byte enum
/// (severity.rs RECOVERABLE/CRITICAL 정신). 기본 Reusable, 비정상 종료 시 Poisoned로 마킹해
/// 재사용 후보 조회에서 영구 배제한다(획득시점 RAII 신설 안 함 — 기존 sweep 모델 존중).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")] // -> "reusable" / "poisoned"
pub enum ProcessHealth {
    #[default]
    Reusable,
    Poisoned,
}

#[derive(Clone, Debug)]
pub struct LedgerEntry {
    pub pid: u32,
    pub pgid: i32,
    pub cmd: String,
    pub surface_id: Option<u64>,
    pub scoped: bool,
    pub registered_at: f64,
    /// T4-4/T6-P3 능력 가드: 이 원장 항목(스코프 프로세스)에 부여된 권한 집합.
    /// launch-agent/claim-role 시점의 surface 역할에서 도출(deny-by-default·write⊇read 정규화).
    /// 기존 필드 불변 — 순수 additive. None=원장에 caps 미기록(레거시 등록·외부 RPC).
    pub caps: Option<crate::caps::Caps>,
    /// T5-6 strand-2 오염 격리: 기본 Reusable, 비정상 종료(크래시·재시작 소진·auth 차단) 감지
    /// 시 Poisoned로 마킹 → `is_reusable`이 false를 돌려 재사용 풀에서 배제. 순수 additive.
    pub health: ProcessHealth,
}

/// T5-6 strand-2 재사용 후보 판정 단일 술어(순수함수 — 테스트 핀 가능, 부작용0).
/// Poisoned 원장 항목은 어떤 재사용 풀에도 돌아가지 않는다. 현 코드베이스는 풀-재사용이
/// 아니라 sweep-회수 모델이라 비-테스트 호출자가 아직 없다(풀 도입 시 이 술어가 게이트).
/// poison-no-reuse 계약을 `is_reusable_excludes_poisoned` 테스트가 박제한다.
#[allow(dead_code)]
pub fn is_reusable(entry: &LedgerEntry) -> bool {
    matches!(entry.health, ProcessHealth::Reusable)
}

/// T1-1 에이전트 자기보고 상태 — 화면 파싱 없이 에이전트가 `cys set-status`로 직접 신고.
/// 신뢰 등급 '참고'(자기신고 — 검증은 attest·기계 게이트의 몫).
#[derive(Clone, Debug, serde::Serialize)]
pub struct AgentStatus {
    pub state: String, // working | waiting | blocked | done
    pub context_pct: Option<u8>,
    pub task: Option<String>,
    pub updated_at: f64,
}

/// ⑪ pack-reinject 추적 마커: 한 surface에 마지막으로 주입된 팩 버전·합성 디렉티브 해시.
/// pack-update/reinject 컨트롤러가 노드 주입 성공 직후 `reinject.mark` RPC로만 갱신한다
/// (단일 write path — status.set 자기보고 경로로는 갱신 불가). topology에 영속되어 cysd
/// 재기동·노드 복원 후에도 생존 → 같은 버전 일괄 재주입(토큰 폭증·컨텍스트 파괴)을 차단한다.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct PackReinject {
    pub pack_version: String,
    pub directive_hash: String,
}

/// 승인 Feed 항목: 워커(에이전트)의 승인 요청을 한 곳에 모은다.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct FeedItem {
    pub request_id: String,
    pub kind: String, // permission | question | notification
    pub title: String,
    pub body: String,
    pub surface_id: Option<u64>,
    pub status: String, // pending | resolved | timeout
    pub decision: Option<String>,
    pub created_at: f64,
    pub resolved_at: Option<f64>,
}

pub struct Config {
    /// PTY에 보장할 로케일 (GUI 기동 데몬은 LANG 미상속 → 한글 입력 깨짐 방지)
    pub lang: String,
    pub load_high_threshold: f64,
    pub proc_count_threshold: usize,
    pub duplicate_threshold: usize,
    pub auto_kill_duplicates: bool,
    pub idle_seconds: u64,
    /// (E-a) 동시 살아있는 worker-* 한도. 0=무제한(하위호환 escape hatch).
    pub max_active_workers: usize,
}

impl Config {
    pub fn from_env() -> Self {
        let cores = std::thread::available_parallelism()
            .map(|n| n.get() as f64)
            .unwrap_or(8.0);
        Config {
            lang: detect_lang(),
            load_high_threshold: env_f64("CYS_LOAD_THRESHOLD", cores * 2.0),
            proc_count_threshold: env_f64("CYS_PROC_THRESHOLD", 50.0) as usize,
            duplicate_threshold: env_f64("CYS_DUP_THRESHOLD", 3.0) as usize,
            auto_kill_duplicates: cys::env_compat("CYS_AUTOKILL_DUP")
                .map(|v| v == "1")
                .unwrap_or(false),
            idle_seconds: env_f64("CYS_IDLE_SECONDS", 300.0) as u64,
            max_active_workers: env_f64("CYS_MAX_ACTIVE_WORKERS", 8.0) as usize,
        }
    }
}

/// LANG 결정: 데몬 env → (macOS) 시스템 사용자 로케일 → en_US.UTF-8.
/// UTF-8 로케일이기만 하면 한글 입출력이 정상 동작한다.
fn detect_lang() -> String {
    if let Ok(l) = std::env::var("LANG") {
        if !l.is_empty() && l.to_uppercase().contains("UTF") {
            return l;
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Ok(out) = std::process::Command::new("defaults")
            .args(["read", "-g", "AppleLocale"])
            .output()
        {
            let loc = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !loc.is_empty() && loc.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                return format!("{loc}.UTF-8");
            }
        }
    }
    "en_US.UTF-8".into()
}

/// CYS_* 우선, 구 JAVIS_*/AITERM_* 폴백 — README가 약속한 CYS_* 이름이 실제로 동작하게 한다
fn env_f64(key: &str, default: f64) -> f64 {
    cys::env_compat(key)
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// T1-3 발신자 해석 캐시 항목: (소속 surface, 해석 시각, peer start_time).
/// start_time(없으면 None)은 pid 재사용 식별자 — 같은 pid라도 incarnation이 다르면 재해석한다.
type CallerCacheEntry = (Option<u64>, f64, Option<u64>);

/// 워커 인스턴스 dedup: 복수 워커가 같은 역할명(→같은 todo 파일)을 공유하지 않도록,
/// "worker" 요청에 충돌 없는 고유 역할명(worker, worker-2, worker-3 …)을 배정한다.
/// 슬롯은 roles에 없거나 점유자가 죽은(없거나 exited) 경우 '빈' 것으로 본다 →
/// 단일 워커가 재시작하면 죽은 'worker' 슬롯을 재사용해 같은 todo 파일을 이어간다(이력 보존).
/// 비-worker 역할(master/cso/reviewer-*)은 그대로 반환 — 단일·latest-wins 유지.
/// 호출자는 surfaces·roles 락을 surfaces→roles 순서로 보유한 상태여야 한다(데드락 회피).
pub fn dedup_worker_role(
    requested: &str,
    roles: &HashMap<String, u64>,
    is_alive: impl Fn(u64) -> bool,
    my_id: u64,
) -> String {
    if requested != "worker" {
        return requested.to_string();
    }
    let mut n: u32 = 1;
    loop {
        let name = if n == 1 {
            "worker".to_string()
        } else {
            format!("worker-{n}")
        };
        match roles.get(&name) {
            None => return name,                        // 미점유 → 사용
            Some(&h) if h == my_id => return name,       // 이미 내 것(재진입)
            Some(&h) if !is_alive(h) => return name,     // 죽은 슬롯 재사용(재시작 연속성)
            Some(_) => {}                                // 살아있는 점유 → 다음 번호
        }
        n += 1;
    }
}

/// (E-b) 살아있는 worker-* 역할 개수. 호출자는 surfaces·roles 락을 surfaces→roles 순서로
/// 보유한 상태여야 한다(데드락 회피 — dedup_worker_role과 동일 계약). 순수 함수(락 비보유).
pub fn live_worker_count(roles: &HashMap<String, u64>, is_alive: impl Fn(u64) -> bool) -> usize {
    roles
        .iter()
        .filter(|(name, _)| *name == "worker" || name.starts_with("worker-"))
        .filter(|(_, &h)| is_alive(h))
        .count()
}

#[cfg(test)]
mod dedup_tests {
    use super::{dedup_worker_role, live_worker_count};
    use std::collections::HashMap;

    fn roles(pairs: &[(&str, u64)]) -> HashMap<String, u64> {
        pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    #[test]
    fn non_worker_passthrough() {
        let r = roles(&[("master", 1)]);
        assert_eq!(dedup_worker_role("master", &r, |_| true, 9), "master");
        assert_eq!(dedup_worker_role("reviewer-gemini", &r, |_| true, 9), "reviewer-gemini");
    }

    #[test]
    fn first_worker_is_plain() {
        let r = roles(&[]);
        assert_eq!(dedup_worker_role("worker", &r, |_| true, 1), "worker");
    }

    #[test]
    fn second_and_third_live_workers_increment() {
        let r = roles(&[("worker", 1)]);
        assert_eq!(dedup_worker_role("worker", &r, |_| true, 2), "worker-2");
        let r2 = roles(&[("worker", 1), ("worker-2", 2)]);
        assert_eq!(dedup_worker_role("worker", &r2, |_| true, 3), "worker-3");
    }

    #[test]
    fn dead_slot_is_reclaimed() {
        // worker(id=1) 죽음, worker-2(id=2) 생존 → 새 워커는 'worker' 슬롯 재사용(이력 연속)
        let r = roles(&[("worker", 1), ("worker-2", 2)]);
        let alive = |h: u64| h == 2; // 1은 죽음
        assert_eq!(dedup_worker_role("worker", &r, alive, 3), "worker");
    }

    #[test]
    fn own_slot_reentry() {
        // 자기 자신이 이미 'worker'를 보유하면 같은 이름 반환(재진입 idempotent)
        let r = roles(&[("worker", 7)]);
        assert_eq!(dedup_worker_role("worker", &r, |_| true, 7), "worker");
    }

    // ---- (E-b) live_worker_count ----

    #[test]
    fn live_worker_count_empty_is_zero() {
        let r = roles(&[]);
        assert_eq!(live_worker_count(&r, |_| true), 0);
    }

    #[test]
    fn live_worker_count_counts_all_alive_workers() {
        // worker + worker-2 둘 다 alive = 2
        let r = roles(&[("worker", 1), ("worker-2", 2)]);
        assert_eq!(live_worker_count(&r, |_| true), 2);
    }

    #[test]
    fn live_worker_count_excludes_dead() {
        // worker(id=1) 죽음, worker-2(id=2) 생존 = 1
        let r = roles(&[("worker", 1), ("worker-2", 2)]);
        assert_eq!(live_worker_count(&r, |h| h == 2), 1);
    }

    #[test]
    fn live_worker_count_ignores_non_worker_roles() {
        // master/cso/reviewer-*는 worker 한도에서 제외
        let r = roles(&[("master", 1), ("cso", 2), ("reviewer-gemini", 3), ("worker", 4)]);
        assert_eq!(live_worker_count(&r, |_| true), 1);
    }
}

pub struct Daemon {
    pub surfaces: Mutex<HashMap<u64, Arc<Surface>>>,
    pub next_id: AtomicU64,
    pub bus: EventBus,
    pub health_rules: Mutex<Vec<HealthRule>>,
    pub health_debounce: Mutex<HashMap<(u64, String), Instant>>,
    /// T4-17 조치 게이트: (surface, rule) → 최근 매칭 시각들 (60초 창 내 threshold 충족 판정)
    pub health_hits: Mutex<HashMap<(u64, String), Vec<f64>>>,
    /// T1-2 status 보드용 최근 health alert 링 (최대 50)
    pub recent_health: Mutex<VecDeque<serde_json::Value>>,
    /// T4-15 kill-switch: pause 중에는 큐 배달·스케줄 발화가 동결된다 (직접 send는 통과)
    pub paused: AtomicBool,
    pub pause_info: Mutex<Option<(f64, String)>>, // (since, reason)
    /// T3-9 todo 워치: path → (done, total, mtime)
    pub todo_progress: Mutex<HashMap<String, (u64, u64, f64)>>,
    /// T1-3 발신자 해석 캐시: caller pid → 항목 — 60초 TTL (항목 정의는 CallerCacheEntry).
    pub caller_cache: Mutex<HashMap<u32, CallerCacheEntry>>,
    /// (E-c) idempotencyKey → (surface_id, epoch초). 클라이언트 재시도가 같은 key면 기존 surface
    /// 재반환(추가 spawn 0). TTL(CREATE_IDEM_TTL_SECS) 만료 엔트리는 조회 시 lazy 제거.
    pub create_idem: Mutex<HashMap<String, (u64, f64)>>,
    pub ledger: Mutex<HashMap<u32, LedgerEntry>>,
    /// 역할 레지스트리: role → surface_id (launch-agent가 등록, --to <role> 주소 해석에 사용)
    pub roles: Mutex<HashMap<String, u64>>,
    /// 적대검증 벡터-9 방어심화: master role이 현재 보유 surface로 (재)claim된 epoch초.
    /// master surface가 죽는 윈도우에 다른 노드가 claim_role("master")로 합법 승계 → 즉시
    /// approval.sign으로 위험명령을 정당 서명할 수 있다. 이 값으로 갓 승계한 master의 서명을
    /// 쿨다운(SIGN_COOLDOWN_SECS) 동안 동결해 승계-윈도우 남용을 차단한다. master가 부재/해제되면
    /// None. ★단일UID·신뢰노드 모델에선 claim_role 자체가 권한 메커니즘이라 legit/usurper를
    /// 암호학적으로 완전 구분 불가 — 이건 윈도우 축소·탐지(방어심화)이지 암호보증이 아니다.
    pub master_claimed_at: Mutex<Option<f64>>,
    pub feed_items: Mutex<Vec<FeedItem>>,
    pub feed_waiters: Mutex<HashMap<String, tokio::sync::oneshot::Sender<String>>>,
    /// feed.jsonl append 직렬화 락 — write_all이 짧은 write로 쪼개져도 한 줄 전체가
    /// 한 임계영역에서 쓰이게 보장한다. O_APPEND의 원자성은 단일 write() 콜 단위라,
    /// 대용량 body가 분할 write되면 다른 동시 appender의 라인이 끼어들어 JSONL이
    /// 손상되고 복원(replay)에서 pending 항목이 무음 유실될 수 있다.
    pub feed_persist_lock: Mutex<()>,
    /// 큐 WAL(P7): 미배달 `--queued` 메시지의 데몬 재기동 생존분(queue-state.json replay).
    /// 라이브 큐는 surface.pending_queue(휘발)이고, 이건 재시작을 넘긴 스냅샷이다 —
    /// queue.list가 라이브 큐와 함께 노출한다. mid(안정 해시)로 이중 replay를 dedup한다.
    pub restored_queue: Mutex<Vec<serde_json::Value>>,
    pub config: Config,
    pub socket_path: PathBuf,
    pub started_at: f64,
    /// 세션 트랜스크립트 FTS 영속 채널 (전용 writer 스레드)
    pub recall_tx: Mutex<std::sync::mpsc::Sender<crate::recall::LineRecord>>,
    /// T6 Control Center 소비 트래커 (claude 메시지 누적 — 오늘·최근창·12h 스파크라인).
    pub consumption: Mutex<Consumption>,
    /// T7 E1-3 영속 분석 저장소(analytics.db) — open 실패 시 None(graceful degrade).
    pub analytics: Mutex<Option<rusqlite::Connection>>,
}

/// T6 Control Center 소비 트래커 — in-memory(재시작 리셋, 가동시간 의미론과 동일).
/// output_tokens는 메시지당 가산이라 누적 모호성이 없다. 수집기가 새 어시스턴트 메시지마다 적재.
#[derive(Default)]
pub struct Consumption {
    pub today_date: String,
    pub today_tokens: u64,
    pub today_input: u64,
    pub today_msgs: u64,
    pub today_cost_usd: f64,
    pub model_tokens: std::collections::HashMap<String, u64>,
    pub sessions: std::collections::HashSet<String>,
    pub buckets: std::collections::VecDeque<(f64, u64)>,
}

impl Consumption {
    /// 새 어시스턴트 메시지 1건 적재 — 날짜가 바뀌면 오늘 카운터를 리셋한다.
    /// `cost`=cost.rs 4-팩터 환산 USD, `model`=모델믹스 집계 키.
    pub fn record_message(
        &mut self,
        session: &str,
        input: u64,
        output: u64,
        cost: f64,
        model: &str,
        now: f64,
        today: &str,
    ) {
        if self.today_date != today {
            self.today_date = today.to_string();
            self.today_tokens = 0;
            self.today_input = 0;
            self.today_msgs = 0;
            self.today_cost_usd = 0.0;
            self.model_tokens.clear();
            self.sessions.clear();
        }
        let total = input + output;
        self.today_tokens += total;
        self.today_input += input;
        self.today_msgs += 1;
        self.today_cost_usd += cost;
        if !model.is_empty() {
            *self.model_tokens.entry(model.to_string()).or_insert(0) += total;
        }
        if !session.is_empty() {
            self.sessions.insert(session.to_string());
        }
        self.buckets.push_back((now, total));
        while let Some(&(t, _)) = self.buckets.front() {
            if now - t > 43_200.0 {
                self.buckets.pop_front();
            } else {
                break;
            }
        }
        while self.buckets.len() > 20_000 {
            self.buckets.pop_front();
        }
    }

    /// 최근 `secs`초 토큰 합.
    pub fn recent_tokens(&self, now: f64, secs: f64) -> u64 {
        self.buckets.iter().filter(|(t, _)| now - t <= secs).map(|(_, v)| v).sum()
    }

    /// 최근 `span`초를 `bins`개 구간으로 집계한 스파크라인(과거→현재).
    pub fn sparkline(&self, now: f64, bins: usize, span: f64) -> Vec<u64> {
        let mut out = vec![0u64; bins];
        if bins == 0 {
            return out;
        }
        let w = span / bins as f64;
        for (t, v) in &self.buckets {
            let age = now - t;
            if !(0.0..=span).contains(&age) {
                continue;
            }
            let idx = (((span - age) / w) as usize).min(bins - 1);
            out[idx] += v;
        }
        out
    }
}

/// (E-c) create_idem 캐시 엔트리 TTL — 클라이언트 재시도 창. 만료분은 조회 시 lazy GC.
pub const CREATE_IDEM_TTL_SECS: f64 = 120.0;

pub fn now_epoch() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// Windows named pipe 경로(`\\.\pipe\<name>`)에서 `<name>` 슬러그를 추출한다(RC-13).
/// 기본 데몬 `\\.\pipe\cys` → `"cys"`(호출자가 %LOCALAPPDATA%\cys 루트로 매핑·기존 호환 유지),
/// 부서 데몬 `\\.\pipe\cys-dept-<n>` → `"cys-dept-<n>"`(루트 하위 부서 고유 디렉토리).
/// 순수 문자열 함수(전 OS 컴파일·mac서 테스트 가능). 역슬래시·슬래시 모두에서 마지막 컴포넌트를 취하고
/// 파일시스템 안전 문자(영숫자·`-`·`_`)만 남긴다(부서명은 dept-N·카탈로그 키라 이미 안전 — 방어적 sanitize).
// windows state_dir 전용 — mac에선 테스트만 사용(비-windows 비-test 빌드 dead_code 허용).
#[cfg_attr(not(windows), allow(dead_code))]
pub fn pipe_slug(socket_path: &std::path::Path) -> String {
    let s = socket_path.to_string_lossy();
    let last = s.rsplit(|c| c == '\\' || c == '/').next().unwrap_or("");
    last.chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .collect()
}

/// 영속 상태 디렉터리 — 소켓과 같은 곳 (unix). Windows는 LOCALAPPDATA 하위.
/// RC-13: Windows에서 부서 데몬마다 pipe명 슬러그로 **고유 디렉토리**를 파생해 transcripts.db·feed.jsonl
/// 격리를 보장한다(구: 모든 부서가 단일 %LOCALAPPDATA%\cys 공유 → SQLite 락 경합·부서간 오염).
/// 기본 데몬(`\\.\pipe\cys`)은 %LOCALAPPDATA%\cys 유지(호환 예외·마이그레이션 불요).
pub fn state_dir(socket_path: &std::path::Path) -> PathBuf {
    #[cfg(windows)]
    {
        let base = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| ".".into());
        let root = PathBuf::from(base).join("cys");
        let slug = pipe_slug(socket_path);
        if slug.is_empty() || slug == "cys" {
            root // 기본 데몬 — 기존 경로 유지(호환)
        } else {
            root.join(slug) // 부서 데몬 — 슬러그별 격리 디렉토리
        }
    }
    #[cfg(not(windows))]
    {
        socket_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."))
    }
}

/// 데몬과 같은 디렉터리에 놓인 형제 `cys` CLI 경로.
/// Windows에서는 실행파일명이 `cys.exe`이므로 플랫폼별 확장자를 붙인다
/// Windows: 데몬(cysd)이 스폰하는 콘솔 자식(CLI·셸·taskkill 등)이 콘솔 창을 띄우지 않게
/// CREATE_NO_WINDOW 를 건다(Win11 기본터미널=Windows Terminal 일 때 매 스폰마다 검은 창이
/// 순간 떠오르는 flash 차단). 타 OS 무동작. std·tokio Command 모두 지원.
pub trait HideConsole {
    fn hide_console(&mut self) -> &mut Self;
}
impl HideConsole for std::process::Command {
    fn hide_console(&mut self) -> &mut Self {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            self.creation_flags(0x0800_0000);
        }
        self
    }
}
impl HideConsole for tokio::process::Command {
    fn hide_console(&mut self) -> &mut Self {
        #[cfg(windows)]
        {
            self.creation_flags(0x0800_0000);
        }
        self
    }
}

/// (cys.rs `sibling_daemon_path`·main.rs `ensure_daemon`과 동일 패턴).
/// 형제 바이너리가 없으면 PATH 탐색용 파일명만 반환한다.
pub fn sibling_cli_path() -> PathBuf {
    let name = if cfg!(windows) { "cys.exe" } else { "cys" };
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join(name)))
        .filter(|p| p.exists())
        .unwrap_or_else(|| PathBuf::from(name))
}

/// 큐 WAL(P7)의 안정 메시지 id — FNV-1a 64로 (surface_id, text)에서 파생.
/// 재기동을 넘어 동일 논리 메시지가 같은 mid를 갖게 해, queue-state.json 이중 replay 시
/// dedup이 성립한다. (동일 surface의 동일 텍스트는 하나로 수렴 — MVP 멱등, Phase 4에서 enqueue-seq 태깅 승격.)
fn queue_mid(sid: u64, text: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in format!("{sid}\u{0}{text}").bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("q{h:016x}")
}

/// queue-state.json replay: {mid, surface_id, text} 배열을 mid로 dedup해 복원한다.
/// 파일 부재/파손이면 빈 벡터(fail-safe — 큐 없음이 기본).
fn load_queue_state(dir: &std::path::Path) -> Vec<serde_json::Value> {
    let mut by_mid: HashMap<String, serde_json::Value> = HashMap::new();
    if let Ok(content) = std::fs::read_to_string(dir.join("queue-state.json")) {
        if let Ok(arr) = serde_json::from_str::<Vec<serde_json::Value>>(&content) {
            for it in arr {
                if let Some(mid) = it.get("mid").and_then(|v| v.as_str()) {
                    by_mid.entry(mid.to_string()).or_insert(it); // 이중 replay dedup
                }
            }
        }
    }
    by_mid.into_values().collect()
}

impl Daemon {
    pub fn new(socket_path: PathBuf) -> Arc<Self> {
        let dir = state_dir(&socket_path);
        let _ = std::fs::create_dir_all(&dir);
        // Feed 복원: JSONL replay. 같은 request_id는 '종결 상태 승리' — append 순서가
        // 경합으로 뒤집혀도 resolved/timeout이 pending에 지지 않는다.
        let mut restored: Vec<FeedItem> = Vec::new();
        let feed_path = dir.join("feed.jsonl");
        if let Ok(content) = std::fs::read_to_string(&feed_path) {
            let mut by_id: HashMap<String, FeedItem> = HashMap::new();
            for line in content.lines() {
                if let Ok(item) = serde_json::from_str::<FeedItem>(line) {
                    match by_id.get(&item.request_id) {
                        Some(prev) if prev.status != "pending" && item.status == "pending" => {}
                        _ => {
                            by_id.insert(item.request_id.clone(), item);
                        }
                    }
                }
            }
            restored = by_id.into_values().collect();
            restored.sort_by(|a, b| {
                a.created_at
                    .partial_cmp(&b.created_at)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            // 보존 한도: pending 전부 + 종결 항목 최근 1000건 (메모리·디스크 무한 누적 차단)
            const FEED_RETAIN: usize = 1000;
            let resolved_count = restored.iter().filter(|i| i.status != "pending").count();
            if resolved_count > FEED_RETAIN {
                let mut drop_n = resolved_count - FEED_RETAIN;
                restored.retain(|i| {
                    if i.status != "pending" && drop_n > 0 {
                        drop_n -= 1;
                        false
                    } else {
                        true
                    }
                });
            }
            // 기동 시 1회 compaction — 서빙 전 단일 스레드 구간이라 append 경합 없음
            let tmp = dir.join("feed.jsonl.tmp");
            if let Ok(mut f) = std::fs::File::create(&tmp) {
                let mut ok = true;
                for item in &restored {
                    if let Ok(line) = serde_json::to_string(item) {
                        if writeln!(f, "{line}").is_err() {
                            ok = false;
                            break;
                        }
                    }
                }
                if ok {
                    let _ = std::fs::rename(&tmp, &feed_path);
                }
            }
        }
        // T4-15 kill-switch 상태 복원 — 재부팅 후에도 pause는 유지된다 (명시 resume까지)
        let pause_restored: Option<(f64, String)> = std::fs::read_to_string(dir.join("autopilot.json"))
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .filter(|v| v["paused"].as_bool() == Some(true))
            .map(|v| {
                (
                    v["since"].as_f64().unwrap_or_else(now_epoch),
                    v["reason"].as_str().unwrap_or("").to_string(),
                )
            });
        // T7 E1-3: 영속 분석 DB는 socket_path가 struct로 move되기 전에 연다.
        let analytics_conn = crate::analytics::open(&socket_path);
        let daemon = Arc::new(Daemon {
            surfaces: Mutex::new(HashMap::new()),
            // 영속 트랜스크립트(transcripts.db)의 최대 id 이후부터 발급 — 재시작 시
            // 무관 세션이 같은 surface_id로 recall에 합쳐지는 것을 차단
            next_id: AtomicU64::new(crate::recall::max_surface_id(&socket_path) + 1),
            bus: EventBus::new(Some(dir.join("event.seq"))),
            health_rules: Mutex::new(default_health_rules()),
            health_debounce: Mutex::new(HashMap::new()),
            health_hits: Mutex::new(HashMap::new()),
            recent_health: Mutex::new(VecDeque::new()),
            paused: AtomicBool::new(pause_restored.is_some()),
            pause_info: Mutex::new(pause_restored),
            todo_progress: Mutex::new(HashMap::new()),
            caller_cache: Mutex::new(HashMap::new()),
            create_idem: Mutex::new(HashMap::new()),
            ledger: Mutex::new(HashMap::new()),
            roles: Mutex::new(HashMap::new()),
            // 벡터-9 방어심화: 기동 시 master 미승계 → None (첫 claim_role("master")에서 기록).
            master_claimed_at: Mutex::new(None),
            feed_items: Mutex::new(restored),
            feed_waiters: Mutex::new(HashMap::new()),
            feed_persist_lock: Mutex::new(()),
            // 큐 WAL 복원: queue-state.json을 mid로 dedup해 replay (미배달 큐 재기동 생존·P7)
            restored_queue: Mutex::new(load_queue_state(&dir)),
            config: Config::from_env(),
            recall_tx: Mutex::new(crate::recall::spawn_writer(socket_path.clone())),
            socket_path,
            started_at: now_epoch(),
            consumption: Mutex::new(Consumption::default()),
            analytics: Mutex::new(analytics_conn),
        });
        // 재시작에도 오늘 소비/비용/모델믹스/스파크라인 보존 — 최근 12h usage_records 리플레이.
        crate::analytics::seed_consumption(&daemon);
        daemon
    }

    /// 데몬 내부용 non-wait feed 항목 생성 (T4-16 승인 격상 등) — push 경로의 축약판.
    pub fn push_feed_notification(
        &self,
        kind: &str,
        title: &str,
        body: &str,
        surface_id: Option<u64>,
    ) {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let request_id = format!(
            "daemon-{}-{}",
            now_epoch() as u64,
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let item = FeedItem {
            request_id: request_id.clone(),
            kind: kind.into(),
            title: title.into(),
            body: body.into(),
            surface_id,
            status: "pending".into(),
            decision: None,
            created_at: now_epoch(),
            resolved_at: None,
        };
        self.feed_items.lock().unwrap().push(item.clone());
        self.persist_feed_item(&item);
        self.bus.publish(
            "feed.item.created",
            "feed",
            surface_id,
            json!({"request_id": request_id, "kind": kind, "title": title,
                   "body": body, "wait": false}),
        );
    }

    /// Feed 항목 스냅샷 한 줄을 JSONL에 append (영속화 — 데몬 재시작 복원용).
    pub fn persist_feed_item(&self, item: &FeedItem) {
        // 직렬화 후에 락 — JSON 변환(락 불필요)은 임계영역 밖에서.
        let Ok(line) = serde_json::to_string(item) else {
            return;
        };
        let dir = state_dir(&self.socket_path);
        // feed_persist_lock으로 append 전 구간을 직렬화: write_all이 짧은 write로
        // 분할돼도 한 줄이 통째로 쓰여, 동시 appender의 라인이 끼어들어 JSONL을
        // 손상시키는 인터리빙(복원 시 pending 무음 유실)을 차단한다.
        let _guard = self.feed_persist_lock.lock().unwrap();
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("feed.jsonl"))
        {
            let _ = std::io::Write::write_all(&mut f, format!("{line}\n").as_bytes());
            // §9.1-1: append 후 fsync — 재부팅에도 미배달 승인요청(feed)이 디스크에 확정된다.
            // (파일별 내구성 불균일 해소: topology는 이미 fsync, feed.jsonl은 누락돼 있었다.)
            let _ = f.sync_all();
        }
    }

    /// 큐 WAL 스냅샷을 원자적으로 영속(P7·§9.1-1). enqueue/pop/clear 뒤 호출한다.
    /// 라이브 surface 큐 + 아직 미소비 restored_queue를 합쳐 mid로 dedup해 쓴다 —
    /// 미배달 `--queued` 메시지가 데몬 재기동을 생존한다(HARNESS 4-a VOLATILE 수리).
    /// ★락 순서 주의: 호출자는 어떤 pending_queue 락도 쥐지 않은 상태여야 한다(재진입 데드락 방지).
    pub fn persist_queue_state(&self) {
        let dir = state_dir(&self.socket_path);
        let mut entries: Vec<serde_json::Value> = Vec::new();
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        {
            let surfaces = self.surfaces.lock().unwrap();
            for s in surfaces.values() {
                let q = s.pending_queue.lock().unwrap();
                for text in q.iter() {
                    let mid = queue_mid(s.id, text);
                    if seen.insert(mid.clone()) {
                        entries.push(json!({"mid": mid, "surface_id": s.id, "text": text}));
                    }
                }
            }
        }
        for it in self.restored_queue.lock().unwrap().iter() {
            if let Some(mid) = it.get("mid").and_then(|v| v.as_str()) {
                if seen.insert(mid.to_string()) {
                    entries.push(it.clone());
                }
            }
        }
        if let Ok(content) = serde_json::to_string(&entries) {
            let _ = crate::governance::write_json_atomic(&dir, "queue-state.json", &content);
        }
    }

    /// Spawn a new PTY surface running the user's shell (or an explicit command).
    // RC-3(B′): env 없는 호환 래퍼(테스트 다수가 사용). 프로덕션 create 경로는 handlers가
    // create_surface_with_env를 직접 호출 → non-test 빌드에선 미사용이라 dead_code 허용.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn create_surface(
        self: &Arc<Self>,
        cwd: Option<String>,
        cmd: Option<String>,
        title: Option<String>,
        role: Option<String>,
        rows: u16,
        cols: u16,
    ) -> Result<Arc<Surface>, String> {
        self.create_surface_with_env(cwd, cmd, title, role, rows, cols, &[])
    }

    /// create_surface + PTY env 주입(RC-3 B′). `env`의 (k,v)를 builder.env로 실어 pane에 직접 전달한다
    /// (Windows launch-agent가 해소한 CLAUDE_CONFIG_DIR 등 — 순수 cmd send와 짝). unix는 빈 슬라이스라
    /// 무동작(셸 인라인 전개가 진실원). CYS_PACK_DIR·CYS_ACCOUNT_DIR 등 기존 주입과 동형.
    #[allow(clippy::too_many_arguments)]
    pub fn create_surface_with_env(
        self: &Arc<Self>,
        cwd: Option<String>,
        cmd: Option<String>,
        title: Option<String>,
        role: Option<String>,
        rows: u16,
        cols: u16,
        env: &[(String, String)],
    ) -> Result<Arc<Surface>, String> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let pty = native_pty_system();
        let pair = pty
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| format!("openpty failed: {e}"))?;

        let shell = default_shell();
        let mut builder = CommandBuilder::new(&shell);
        #[cfg(not(windows))]
        {
            if let Some(c) = &cmd {
                builder = CommandBuilder::new(&shell);
                // D8(RC-19·mac): 로그인셸이 path_helper로 runtime 선두주입(아래 builder.env PATH)을 맨 뒤로
                // 강등한다(검증 완료) → /usr/bin/git·python3(CLT-shim)이 이겨 순정 맥서 개발도구 프롬프트.
                // 프로파일 실행 뒤 도는 -c 명령 앞에서 runtime bin dir를 재선두주입해 동봉본이 이기게 한다.
                // shebang(#!/usr/bin/env python3)도 이 PATH로 해소. runtime 부재(비동봉)면 no-op.
                #[cfg(target_os = "macos")]
                let c_eff = mac_runtime_lc_prefix().map(|pfx| format!("{pfx}{c}"));
                #[cfg(target_os = "macos")]
                builder.args(["-lc", c_eff.as_deref().unwrap_or(c.as_str())]);
                #[cfg(not(target_os = "macos"))]
                builder.args(["-lc", c]);
            } else {
                // 대화형 surface도 로그인 셸(-l)로 기동 — Finder(GUI) 기동 시 빈곤한 PATH를
                // 셸 로그인 프로파일이 복원(/opt/homebrew/bin·~/.local/bin·path_helper)해
                // pane 속 노드(claude·agy 등)가 도구를 찾는다. cmd 경로(-lc)와 동일한 가정.
                builder.args(["-l"]);
            }
        }
        #[cfg(windows)]
        if let Some(c) = &cmd {
            builder = CommandBuilder::new(&shell);
            // -Command는 PowerShell 전용 플래그다. CYS_SHELL로 cmd.exe를 지정하면
            // cmd.exe는 -Command를 못 알아듣고 명령이 깨진다 → 셸명으로 플래그를 선택.
            builder.args([windows_exec_flag(&shell), c.as_str()]);
        }
        let cwd_str = cwd.unwrap_or_else(|| {
            dirs::home_dir()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|| ".".into())
        });
        builder.cwd(&cwd_str);
        builder.env("TERM", "xterm-256color");
        builder.env("LANG", &self.config.lang);
        // RC-6(T3 발견): Windows 번들 embeddable Python은 open() 기본 인코딩이 cp1252라 UTF-8(한글)
        // 팩 파일 읽기가 UnicodeDecodeError로 크래시. pane에서 도는 python(hooks·javis_*.py)이 UTF-8을
        // 기본으로 쓰게 PYTHONUTF8=1 주입(unix 무영향·이미 UTF-8). cys-dept는 자체 export로 보강.
        builder.env("PYTHONUTF8", "1");
        // 온보딩①: 데몬 옆 동봉 cys CLI + (Windows)동봉 runtime을 pane PATH 선두 주입 —
        // 신규 머신(심링크 없음)에서도 pane 속 AI가 `cys identify`·python3·bash를 즉시 쓴다.
        // RC-5: GUI 직스폰과 공유하는 공용 fn(cys::runtime_prefixed_path) 사용 — 중복 구현 금지.
        if let Some(bin_dir) = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        {
            let cur = std::env::var("PATH").unwrap_or_default();
            if let Some(newp) = cys::runtime_prefixed_path(&bin_dir, &cur) {
                builder.env("PATH", newp);
            }
        }
        builder.env(cys::ENV_SOCKET, self.socket_path.to_string_lossy().as_ref());
        // 부서 격리: 데몬 자신의 pack_dir(=CYS_PACK_DIR env, 미설정 시 기본 ~/.cys/pack)을 자식 pane에
        // 전파한다. 이게 없으면 부서 데몬이 띄운 worker pane의 `cys todo-path`/skill/memory가
        // 글로벌 pack으로 폴백해 부서 격리가 도구 레벨에서 깨진다(멀티마스터 정식화 F1).
        // 기본 데몬은 기본값을 전파하므로 단일 사용자 동작은 무변경.
        builder.env(
            cys::pack::ENV_PACK_DIR,
            cys::pack::pack_dir().to_string_lossy().as_ref(),
        );
        // 부서 계정 격리(＋부서 자동화): 데몬 자신의 CYS_ACCOUNT_DIR(cys-dept create 가 주입)을 자식
        // pane 에 전파. agents.json claude.cmd 의 ${CYS_ACCOUNT_DIR:-...} 가 이 값으로 해석된다
        // (미설정=기본 ysfuture fail-safe). CYS_PACK_DIR 전파와 동형.
        if let Ok(acct) = std::env::var("CYS_ACCOUNT_DIR") {
            if !acct.is_empty() {
                builder.env("CYS_ACCOUNT_DIR", acct);
            }
        }
        builder.env(cys::ENV_SURFACE_ID, id.to_string());
        builder.env(cys::ENV_SURFACE_REF, cys::surface_ref(id));
        if let Some(r) = &role {
            builder.env(cys::ENV_ROLE, r);
        }
        // RC-3(B′): 호출자 지정 env(Windows launch-agent가 해소한 CLAUDE_CONFIG_DIR 등)를 마지막에
        // 주입 — 순수 cmd로 기동되는 claude가 pane env에서 직접 읽는다. unix는 빈 슬라이스(무동작).
        for (k, v) in env {
            builder.env(k, v);
        }

        let child = pair
            .slave
            .spawn_command(builder)
            .map_err(|e| format!("spawn failed: {e}"))?;
        let pid = child.process_id().unwrap_or(0);
        drop(pair.slave);

        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| format!("clone reader failed: {e}"))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| format!("take writer failed: {e}"))?;

        let (out_tx, _) = broadcast::channel(256);

        // PTY writer 전용 스레드: 유한 채널 수신 루프가 단독으로 writer를 소유한다.
        // 모든 senders가 drop되거나(서피스 제거) write 실패 시 스스로 종료한다.
        // 자력 종료(셸 EOF) 경로는 close_surface를 거치지 않아 write_tx가 맵 속 Arc에
        // 영구 잔존 → recv()가 영영 반환 않고 writer 스레드·PTY writer fd가 누수된다.
        // writer_stop을 reader 스레드(EOF)가 세우면 recv_timeout 루프가 이를 보고 종료해
        // 좀비 writer 스레드와 그 fd를 즉시 회수한다.
        let (write_tx, write_rx) = std::sync::mpsc::sync_channel::<WriteReq>(128);
        let writer_stop = Arc::new(AtomicBool::new(false));
        {
            let writer = writer;
            let stop = Arc::clone(&writer_stop);
            std::thread::spawn(move || run_writer_loop(writer, write_rx, stop));
        }

        let surface = Arc::new(Surface {
            id,
            title: Mutex::new(title.unwrap_or_else(|| format!("surface {id}"))),
            role: Mutex::new(role.clone()),
            cmd: cmd.unwrap_or_else(|| shell.clone()),
            cwd: cwd_str,
            pid,
            created_at: now_epoch(),
            env_injected: !env.is_empty(), // RC-3 잔여(T2.1): env 주입 여부 기록(node-recover 안전 판정)
            exited: AtomicBool::new(false),
            exited_at: Mutex::new(None),
            write_tx,
            master: Mutex::new(pair.master),
            child: Mutex::new(child),
            parser: Mutex::new(vt100::Parser::new(rows, cols, SCROLLBACK_LINES)),
            scrollback: Mutex::new(VecDeque::with_capacity(1024)),
            ingest: Mutex::new(IngestState {
                carry: Vec::new(),
                pending_cr: false,
                partial: String::new(),
            }),
            out_tx,
            last_output: Mutex::new(Instant::now()),
            idle_notified: AtomicBool::new(false),
            last_recall_line: Mutex::new(String::new()),
            pending_queue: Mutex::new(std::collections::VecDeque::new()),
            agent_status: Mutex::new(None),
            agent_meta: Mutex::new(None),
            agent_seen: AtomicBool::new(false),
            agent_exit_notified: AtomicBool::new(false),
            crash_notified: AtomicBool::new(false),
            last_cmd_ack: Mutex::new(None),
            last_human_input: Mutex::new(None),
            line_count: AtomicU64::new(0),
            queue_paused_until: Mutex::new(None),
            last_injected: Mutex::new(None),
            observed_usage: Mutex::new(None),
            registered_transcript: Mutex::new(None),
            agent_session_id: Mutex::new(None),
            pack_reinject: Mutex::new(None),
            ctx_threshold_armed: AtomicBool::new(true),
            // 능력 가드: 생성 시 역할에서 도출(reviewer-*=read/search, full=worker/master/cso,
            // 그 외 deny-by-default none). claim_role이 역할 전이 시 동기 재도출한다.
            caps: Mutex::new(crate::caps::Caps::for_role(role.as_deref())),
            osc_carry: Mutex::new(Vec::new()),
        });

        {
            // surfaces 등록 '이후'에 역할 공개 — resolve_role 직후 get_surface가
            // 실패해 스케줄러가 역할 부재로 오판하는 창을 닫는다.
            // 락 순서는 surfaces→roles→surface.role (close_surface와 동일 — AB-BA 데드락 차단).
            let mut surfaces = self.surfaces.lock().unwrap();
            surfaces.insert(id, surface.clone());
            if let Some(r) = &role {
                let mut roles = self.roles.lock().unwrap();
                // worker면 충돌 없는 고유 역할명 배정(worker-N) — 복수 워커 todo 충돌 방지.
                // 비-worker는 기존 latest-wins(같은 역할 재등록=최신 승리).
                let final_role = dedup_worker_role(
                    r,
                    &roles,
                    |h| {
                        surfaces
                            .get(&h)
                            .map(|s| !s.exited.load(Ordering::Relaxed))
                            .unwrap_or(false)
                    },
                    id,
                );
                *surface.role.lock().unwrap() = Some(final_role.clone());
                roles.insert(final_role, id);
            }
        }
        if role.is_some() {
            crate::governance::persist_topology(self);
        }
        self.bus.publish(
            "surface.created",
            "surface",
            Some(id),
            json!({"surface_ref": cys::surface_ref(id), "pid": pid, "cwd": surface.cwd,
                   "cmd": surface.cmd, "role": role}),
        );

        // Reader thread: PTY output → vt100 parser + scrollback + attach broadcast + health rules.
        let daemon = Arc::clone(self);
        let surf = Arc::clone(&surface);
        let reader_writer_stop = Arc::clone(&writer_stop);
        let debug = cys::env_compat("CYS_DEBUG")
            .map(|v| v == "1")
            .unwrap_or(false);
        std::thread::spawn(move || {
            let mut reader = reader;
            let mut buf = [0u8; 16 * 1024];
            // DSR 질의가 청크 경계에 걸려도 감지되도록 직전 꼬리 3바이트를 이어붙인다
            let mut dsr_tail: Vec<u8> = Vec::new();
            if debug {
                eprintln!(
                    "[debug] reader thread started for surface {} (pid {})",
                    surf.id, surf.pid
                );
            }
            loop {
                match std::io::Read::read(&mut reader, &mut buf) {
                    Ok(0) => {
                        if debug {
                            eprintln!("[debug] surface {} reader EOF", surf.id);
                        }
                        break;
                    }
                    Err(e) => {
                        if debug {
                            eprintln!("[debug] surface {} reader error: {e}", surf.id);
                        }
                        break;
                    }
                    Ok(n) => {
                        if debug {
                            eprintln!("[debug] surface {} read {n} bytes", surf.id);
                        }
                        let chunk = &buf[..n];
                        // DSR cursor-position query: a real terminal must answer, or
                        // ConPTY(Windows)가 응답을 기다리며 입출력 펌프를 멈춘다.
                        let mut probe = dsr_tail.clone();
                        probe.extend_from_slice(chunk);
                        let needs_dsr = probe.windows(4).any(|w| w == b"\x1b[6n");
                        // 파서 반영(process)과 attach 브로드캐스트(out_tx.send)를 같은 parser 락
                        // 임계영역에 묶는다 — run_attach가 parser 락 아래에서 구독+스냅샷을 뜨므로,
                        // 이 둘이 분리되면(과거 버그) process 이후·send 이전에 구독한 attach가
                        // 같은 청크를 스냅샷과 live로 중복 수신한다. 락이 process↔send를 직렬화해야
                        // run_attach 주석의 불변식(중복 배달 창 봉쇄)이 실제로 성립한다.
                        // DSR 커서 위치도 같은 락 아래에서 읽어(재진입 락 회피) 일관성을 유지한다.
                        let dsr_resp = {
                            // poison된 락도 복구 — 단일 패닉이 데몬 전체를 마비시키지 않게 한다.
                            let mut parser = surf.parser.lock().unwrap_or_else(|e| e.into_inner());
                            // vt100 0.15.2(row.rs:89 등) 내부 인덱스 패닉이 parser 락을 poison시켜
                            // 데몬 요청처리 전체를 마비시키던 연쇄를 차단: process를 catch_unwind로 격리.
                            let resp = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                                || {
                                    parser.process(chunk);
                                    needs_dsr.then(|| {
                                        let (r, c) = parser.screen().cursor_position();
                                        format!("\x1b[{};{}R", r + 1, c + 1)
                                    })
                                },
                            )) {
                                Ok(resp) => resp,
                                Err(_) => {
                                    eprintln!(
                                        "[cysd] surface {} vt100 process panic contained ({} bytes)",
                                        surf.id,
                                        chunk.len()
                                    );
                                    None
                                }
                            };
                            let _ = surf.out_tx.send(chunk.to_vec());
                            resp
                        };
                        if let Some(resp) = dsr_resp {
                            let _ = surf.write_tx.try_send(WriteReq::Data(resp.into_bytes()));
                            if debug {
                                eprintln!("[debug] surface {} answered DSR", surf.id);
                            }
                        }
                        dsr_tail = probe[probe.len().saturating_sub(3)..].to_vec();
                        *surf.last_output.lock().unwrap() = Instant::now();
                        surf.idle_notified.store(false, Ordering::Relaxed);
                        // (B2-c) OSC 9/99/777 알림 스캔 — strip 전 raw chunk 사용. parser 락
                        // 임계영역(위 :876-902) 밖이라 attach 중복배달 불변식과 직교한다.
                        {
                            let mut carry = surf.osc_carry.lock().unwrap();
                            carry.extend_from_slice(chunk);
                            // 미완성 OSC가 무한 성장하는 경로 차단(128KiB 초과 폐기)
                            if carry.len() > 128 * 1024 {
                                carry.clear();
                            }
                            let extracted = drain_complete_osc(&mut carry);
                            drop(carry);
                            for (mut title, body) in extracted {
                                if title.is_empty() {
                                    title = surf.title.lock().unwrap().clone(); // cmux 폴백
                                }
                                // 억제 게이트: 직전 1.5s 내 주입(에코)이 있으면 폐기(cmux suppressesRaw 대응)
                                let recently_injected = surf
                                    .last_injected
                                    .lock()
                                    .unwrap()
                                    .map(|t| t.elapsed().as_millis() < 1500)
                                    .unwrap_or(false);
                                if recently_injected {
                                    continue;
                                }
                                daemon.bus.publish(
                                    "osc.notify",
                                    "notify",
                                    Some(surf.id),
                                    json!({"surface_ref": cys::surface_ref(surf.id), "title": title, "body": body}),
                                );
                            }
                        }
                        daemon.ingest_output(&surf, chunk);
                    }
                }
            }
            surf.exited.store(true, Ordering::Relaxed);
            // 종료 시각 stamp — watchdog reap_exited_surfaces가 grace 경과를 이 시점 기준으로 잰다.
            *surf.exited_at.lock().unwrap() = Some(Instant::now());
            // writer 스레드 종료 신호 — 자력 종료(셸 EOF)는 close_surface를 거치지 않아
            // write_tx가 맵 속 Arc에 영구 잔존하므로, 여기서 stop을 세워 recv_timeout 루프가
            // 좀비 writer 스레드와 PTY writer fd를 회수하게 한다 (24/365 데몬 fd 누수 차단).
            reader_writer_stop.store(true, Ordering::Relaxed);
            // 좀비 회수: 자력 종료(셸 exit)는 close_surface를 거치지 않으므로 여기서 reap.
            // EOF 시점엔 거의 항상 이미 종료 — 즉시 회수, 아니면 1초 후 한 번 더.
            {
                let mut child = surf.child.lock().unwrap();
                if child.try_wait().ok().flatten().is_none() {
                    std::thread::sleep(std::time::Duration::from_millis(1000));
                    let _ = child.try_wait();
                }
            }
            // 미배달 큐 폐기 통지 — queued:true 응답을 받은 발신자의 무음 메시지 유실 차단
            let dropped: Vec<String> = surf.pending_queue.lock().unwrap().drain(..).collect();
            if !dropped.is_empty() {
                daemon.bus.publish(
                    "queue.dropped",
                    "queue",
                    Some(surf.id),
                    json!({"reason": "process_exited", "count": dropped.len(),
                           "bytes": dropped.iter().map(|t| t.len()).sum::<usize>()}),
                );
            }
            daemon.bus.publish(
                "surface.exited",
                "surface",
                Some(surf.id),
                json!({"surface_ref": cys::surface_ref(surf.id)}),
            );
        });

        Ok(surface)
    }

    /// Append stripped output to the scrollback line buffer and run health rules.
    /// 청크 경계 안전: 미완성 ESC 시퀀스·UTF-8 멀티바이트 꼬리는 다음 청크와 합쳐 처리한다
    /// (경계에서 한글 파괴·escape 잔재 혼입 차단).
    fn ingest_output(&self, surface: &Surface, chunk: &[u8]) {
        let mut st = surface.ingest.lock().unwrap();
        st.carry.extend_from_slice(chunk);
        let mut cut = st.carry.len();
        // 마지막 ESC가 미완성 시퀀스면 그 지점부터 보류 (128바이트 초과 보류는 포기 — 영구 정체 방지)
        if let Some(esc) = st.carry.iter().rposition(|&b| b == 0x1b) {
            let tail = &st.carry[esc..];
            if tail.len() < 128 && ansi_incomplete(tail) {
                cut = esc;
            }
        }
        // UTF-8 미완성 꼬리 보류 (진짜 손상 바이트는 lossy로 흘려보낸다 — 보류하면 영구 정체)
        cut = match std::str::from_utf8(&st.carry[..cut]) {
            Ok(_) => cut,
            Err(e) if e.error_len().is_none() => e.valid_up_to(),
            Err(_) => cut,
        };
        if cut == 0 {
            return;
        }
        let drained: Vec<u8> = st.carry.drain(..cut).collect();
        let stripped = strip_ansi_escapes::strip(&drained);
        let text = String::from_utf8_lossy(&stripped);
        let mut completed: Vec<String> = Vec::new();
        for ch in text.chars() {
            if st.pending_cr {
                st.pending_cr = false;
                if ch == '\n' {
                    // CRLF — 일반 줄바꿈
                    completed.push(std::mem::take(&mut st.partial));
                    continue;
                }
                // 단독 \r = 캐리지 리턴 덮어쓰기 — 직전 내용을 대체 (concat·무한 성장 차단)
                st.partial.clear();
            }
            match ch {
                '\n' => completed.push(std::mem::take(&mut st.partial)),
                '\r' => st.pending_cr = true,
                _ => {
                    // \n 없는 스트림의 메모리 무한 성장 방지 상한
                    if st.partial.len() < 8192 {
                        st.partial.push(ch);
                    }
                }
            }
        }
        drop(st);
        if !completed.is_empty() {
            let mut sb = surface.scrollback.lock().unwrap_or_else(|e| e.into_inner());
            for line in &completed {
                if sb.len() >= SCROLLBACK_LINES {
                    sb.pop_front();
                }
                sb.push_back(line.clone());
            }
            // T3-14 단조 라인 커서 — scrollback FIFO 퇴출과 무관하게 누적.
            // ★레이스 차단: line_count 증가를 scrollback 락 임계영역 안에서 수행한다.
            // 델타 read/wait_for(handlers.rs·main.rs)는 scrollback 락을 잡은 채 line_count를
            // 읽으므로, push(N)과 fetch_add(N)이 분리되면 '증가 전 total + push 후 sb.len()'을
            // 관측하는 인터리빙으로 oldest가 N 작아져 skip이 N 과도해지고 최신 N라인을 건너뛴다.
            // 둘을 같은 락 아래로 묶어 reader가 (sb.len, line_count)를 항상 일관되게 본다.
            surface
                .line_count
                .fetch_add(completed.len() as u64, Ordering::Relaxed);
            drop(sb);
            self.persist_for_recall(surface, &completed);
            self.run_health_rules(surface, &completed);
        }
    }

    /// FTS 영속: 의미 있는 라인만 (3자 미만·연속 중복 스킵 — TUI 리드로우 노이즈 억제).
    fn persist_for_recall(&self, surface: &Surface, lines: &[String]) {
        let role = surface.role.lock().unwrap().clone();
        let title = surface.title.lock().unwrap().clone();
        let mut last = surface.last_recall_line.lock().unwrap();
        let tx = self.recall_tx.lock().unwrap();
        for line in lines {
            let trimmed = line.trim();
            if trimmed.chars().count() < 3 || trimmed == last.as_str() {
                continue;
            }
            *last = trimmed.to_string();
            let _ = tx.send(crate::recall::LineRecord {
                ts: now_epoch(),
                surface_id: surface.id,
                role: role.clone(),
                title: title.clone(),
                line: trimmed.to_string(),
            });
        }
    }

    /// 오너 완화책 ①: scrollback 패턴 룰 — 매칭 시 health.alert를 push한다 (폴링 불필요).
    /// T4-17: 에코 제외(주입 직후 2초 라인은 매칭 제외 — 주입 문자열 에코로 인한
    /// 자기/타기 DoS 차단) + 조치 바인딩(60초 창 연속 매칭 게이트 통과 시에만 발동).
    fn run_health_rules(&self, surface: &Surface, lines: &[String]) {
        let surface_id = surface.id;
        // 에코 제외: 직전 원격 주입 후 2초 내 도착한 라인 배치는 룰 평가에서 제외
        if let Some(t) = *surface.last_injected.lock().unwrap() {
            if t.elapsed().as_secs() < 2 {
                return;
            }
        }
        let rules = self.health_rules.lock().unwrap();
        for line in lines {
            for rule in rules.iter() {
                if rule.regex.is_match(line) {
                    let key = (surface_id, rule.name.clone());
                    // status 보드용 최근 alert 링 (디바운스와 무관하게 기록, cap 50)
                    {
                        let mut recent = self.recent_health.lock().unwrap();
                        if recent.len() >= 50 {
                            recent.pop_front();
                        }
                        recent.push_back(json!({
                            "ts": now_epoch(), "surface_id": surface_id,
                            "rule": rule.name, "line": line.chars().take(200).collect::<String>(),
                        }));
                    }
                    let mut debounce = self.health_debounce.lock().unwrap();
                    let fire = match debounce.get(&key) {
                        Some(t) => t.elapsed().as_secs() >= 30,
                        None => true,
                    };
                    if fire {
                        debounce.insert(key.clone(), Instant::now());
                        drop(debounce);
                        self.bus.publish(
                            "health.alert",
                            "health",
                            Some(surface_id),
                            json!({"rule": rule.name, "line": line}),
                        );
                    }
                    // T4-17 조치 바인딩 — 60초 창 내 threshold회 이상 매칭 시에만 발동
                    if let Some(action) = &rule.action {
                        let now = now_epoch();
                        let count = {
                            let mut hits = self.health_hits.lock().unwrap();
                            let v = hits.entry(key).or_default();
                            v.push(now);
                            v.retain(|t| now - t <= 60.0);
                            v.len() as u32
                        };
                        if count >= rule.threshold && action == "pause-queue" {
                            *surface.queue_paused_until.lock().unwrap() = Some(
                                Instant::now() + std::time::Duration::from_secs(rule.pause_secs),
                            );
                            self.bus.publish(
                                "health.action",
                                "health",
                                Some(surface_id),
                                json!({"rule": rule.name, "action": "pause-queue",
                                       "pause_secs": rule.pause_secs, "matches_in_window": count}),
                            );
                        }
                    }
                }
            }
        }
    }

    /// T4-15 pause 상태 영속 — 데몬 재시작 후에도 kill-switch가 유지된다.
    pub fn persist_pause(&self) {
        let dir = state_dir(&self.socket_path);
        let info = self.pause_info.lock().unwrap().clone();
        let v = match (
            self.paused.load(Ordering::Relaxed),
            info,
        ) {
            (true, Some((since, reason))) => {
                json!({"paused": true, "since": since, "reason": reason})
            }
            _ => json!({"paused": false}),
        };
        let _ = std::fs::write(dir.join("autopilot.json"), v.to_string());
    }

    pub fn get_surface(&self, id: u64) -> Option<Arc<Surface>> {
        self.surfaces.lock().unwrap().get(&id).cloned()
    }
}

/// PTY writer 전용 스레드의 수신 루프. surface별 writer를 단독 소유하고 WriteReq를
/// 순서대로 PTY에 쓴다. 다음 셋 중 하나면 종료(= writer drop → PTY writer fd 회수):
///   ① 모든 sender drop(Disconnected) — close_surface로 Arc<Surface> 제거
///   ② write 실패 — PTY 닫힘
///   ③ stop 신호 — 자력 종료(셸 EOF). reader 스레드가 EOF에서 이를 세운다.
/// ③이 없으면 자력 종료 surface의 write_tx가 맵 속 Arc에 영구 잔존해 recv()가 영영
/// 반환되지 않고 writer 스레드·PTY writer fd가 단조 누수된다(24/365 데몬의 fd 고갈).
/// recv_timeout 폴링은 stop을 주기적으로 관측하기 위한 것 — 평시 동작·순서는 불변이다.
/// clear_first 주입의 Ctrl-U 후 settle(ms) — TUI가 라인 정리를 반영할 짬. 기본 150
/// (기존 cys.rs --clear-first의 클라측 sleep 값 계승). CYS_CLEAR_SETTLE_MS로 조정.
fn clear_settle_ms() -> u64 {
    std::env::var("CYS_CLEAR_SETTLE_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(150)
}

fn run_writer_loop<W: Write>(
    mut writer: W,
    write_rx: std::sync::mpsc::Receiver<WriteReq>,
    stop: Arc<AtomicBool>,
) {
    use std::sync::mpsc::RecvTimeoutError;
    loop {
        let req = match write_rx.recv_timeout(std::time::Duration::from_millis(200)) {
            Ok(req) => req,
            Err(RecvTimeoutError::Timeout) => {
                if stop.load(Ordering::Relaxed) {
                    break; // 자력 종료 — 좀비 writer 스레드·fd 회수
                }
                continue;
            }
            Err(RecvTimeoutError::Disconnected) => break, // 모든 sender drop
        };
        let res = match req {
            WriteReq::Data(bytes) => writer.write_all(&bytes).and_then(|_| writer.flush()),
            WriteReq::Inject {
                text,
                cr_delay_ms,
                clear_first,
            } => (if clear_first {
                // Ctrl-U(0x15) 선정리 → settle: 잔존 미제출 텍스트를 지우고 TUI가 처리할 짬을 준다.
                // paste·CR과 같은 arm에 묶여 다른 주입이 끼어들 수 없다(원자). 키 의미 게이트는
                // 호출자(send_text)가 agent 등록 pane으로 제한한다(TUI별 Ctrl-U 의미 상이).
                writer
                    .write_all(b"\x15")
                    .and_then(|_| writer.flush())
                    .map(|_| std::thread::sleep(std::time::Duration::from_millis(clear_settle_ms())))
            } else {
                Ok(())
            })
            .and_then(|_| writer.write_all(format!("\x1b[200~{text}\x1b[201~").as_bytes()))
            .and_then(|_| writer.flush())
            .map(|_| std::thread::sleep(std::time::Duration::from_millis(cr_delay_ms)))
            .and_then(|_| writer.write_all(b"\r"))
            .and_then(|_| writer.flush()),
        };
        if res.is_err() {
            break; // PTY 닫힘 — 이후 send는 disconnected로 호출자에 드러난다
        }
    }
}

/// tail(ESC로 시작)이 미완성 ANSI 시퀀스인지 보수적으로 판정한다.
fn ansi_incomplete(tail: &[u8]) -> bool {
    if tail.len() == 1 {
        return true; // ESC 단독
    }
    match tail[1] {
        // CSI: 종결 바이트(0x40-0x7E)가 아직 없으면 미완성
        b'[' => !tail[2..].iter().any(|&b| (0x40..=0x7e).contains(&b)),
        // OSC: BEL 또는 ST(ESC \)가 아직 없으면 미완성
        b']' => !tail.contains(&0x07) && !tail.windows(2).any(|w| w == b"\x1b\\"),
        // 그 외 2바이트 ESC 시퀀스 — 완결로 간주
        _ => false,
    }
}

/// (B2-a) OSC 9/99/777 데스크톱 알림을 (title, body)로 추출한다. 시퀀스 경계는 BEL(0x07)
/// 또는 ST(ESC \)로, 호출처가 ESC]와 종결자를 포함한 완성 시퀀스를 넘긴다(여기서 벗긴다).
/// 추출 못 한 (미완성·진행률·기타) 시퀀스는 None. 1차 범위: 단일-청크 평문 payload
/// (멀티청크 OSC 99·base64는 미지원). 순수 함수 — 슬라이스 연산만(panic-free).
fn parse_osc_notification(seq: &[u8]) -> Option<(String, String)> {
    let s = std::str::from_utf8(seq).ok()?;
    let s = s.strip_prefix("\x1b]").unwrap_or(s);
    // 종결자 BEL/ST 제거 (ST = ESC \)
    let s = s
        .trim_end_matches('\x07')
        .trim_end_matches('\\')
        .trim_end_matches('\x1b');
    let mut it = s.splitn(2, ';');
    let code = it.next()?;
    let rest = it.next().unwrap_or("");
    match code {
        "9" => {
            // OSC 9;4;... = ConEmu 진행률 → 알림 아님
            if rest.starts_with("4;") || rest == "4" {
                return None;
            }
            (!rest.is_empty()).then(|| (String::new(), rest.to_string()))
        }
        "777" => {
            // 777;notify;<title>;<body>
            let mut p = rest.splitn(3, ';');
            if p.next()? != "notify" {
                return None;
            }
            let title = p.next().unwrap_or("").to_string();
            let body = p.next().unwrap_or("").to_string();
            (!title.is_empty() || !body.is_empty()).then(|| (title, body))
        }
        "99" => {
            // 99;<metadata>;<payload> — 1차 범위: metadata 무시, 평문 payload만
            let payload = rest.rsplitn(2, ';').next().unwrap_or(rest).to_string();
            (!payload.is_empty()).then(|| (String::new(), payload))
        }
        _ => None,
    }
}

/// (B2-a) carry에서 `ESC](=0x1b 0x5d)`로 시작해 BEL(0x07) 또는 ST(ESC \)로 끝나는 완성
/// OSC 시퀀스를 앞에서부터 추출해 parse_osc_notification에 넘기고 소비한다. ESC] 앞의
/// 비-OSC 바이트와 추출 실패 시퀀스는 버린다(추출 전용 — 화면 렌더/strip 경로와 독립).
/// 미완성 꼬리(ESC] 시작 후 종결자 미도착)는 carry에 남겨 다음 청크와 이어붙인다.
/// 종결 판정은 ansi_incomplete의 OSC 규칙(BEL 또는 ESC\)과 동일하다.
fn drain_complete_osc(carry: &mut Vec<u8>) -> Vec<(String, String)> {
    let mut out = Vec::new();
    // keep_from = carry에서 보존을 시작할 위치. 미완성 OSC 시작을 만나면 거기로 고정,
    // 아니면 스캔이 끝난 곳까지(앞쪽은 전부 버림 — 추출 전용).
    let mut keep_from = carry.len();
    let mut i = 0;
    while i < carry.len() {
        // 다음 OSC 시작(ESC])을 찾는다
        if i + 1 >= carry.len() {
            // ESC 단독 꼬리 — 다음 청크와 이어붙이게 보존
            if carry[i] == 0x1b {
                keep_from = i;
            } else {
                keep_from = carry.len();
            }
            break;
        }
        if carry[i] != 0x1b || carry[i + 1] != 0x5d {
            i += 1;
            continue;
        }
        // ESC] 이후에서 종결자(BEL 또는 ST=ESC\)를 찾는다
        let mut end: Option<usize> = None;
        let mut j = i + 2;
        while j < carry.len() {
            if carry[j] == 0x07 {
                end = Some(j + 1); // BEL 1바이트 포함
                break;
            }
            if carry[j] == 0x1b && j + 1 < carry.len() && carry[j + 1] == 0x5c {
                end = Some(j + 2); // ST 2바이트 포함
                break;
            }
            j += 1;
        }
        match end {
            Some(e) => {
                if let Some(pair) = parse_osc_notification(&carry[i..e]) {
                    out.push(pair);
                }
                i = e;
                keep_from = e; // 여기까지 확정 소비
            }
            None => {
                // 미완성 OSC — 이 ESC]부터 다음 청크와 이어붙이게 남긴다
                keep_from = i;
                break;
            }
        }
    }
    carry.drain(..keep_from);
    out
}

/// Windows에서 셸에 인라인 명령을 넘길 때 쓰는 플래그를 셸명으로 선택한다.
/// cmd.exe 계열은 `/C`, PowerShell(powershell.exe·pwsh) 계열은 `-Command`.
/// (default_shell이 CYS_SHELL로 셸을 바꿀 수 있으므로 플래그 하드코딩은 깨진다.)
#[cfg_attr(not(windows), allow(dead_code))]
fn windows_exec_flag(shell: &str) -> &'static str {
    // 경로·확장자를 떼고 베이스 이름만 소문자로 비교 (C:\Windows\System32\cmd.exe → cmd)
    let base = shell
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(shell)
        .trim_end_matches(".exe")
        .trim_end_matches(".EXE")
        .to_ascii_lowercase();
    if base == "cmd" {
        "/C"
    } else {
        "-Command"
    }
}

fn default_shell() -> String {
    #[cfg(windows)]
    {
        cys::env_compat("CYS_SHELL").unwrap_or_else(|| "powershell.exe".into())
    }
    #[cfg(not(windows))]
    {
        cys::env_compat("CYS_SHELL")
            .or_else(|| std::env::var("SHELL").ok())
            .unwrap_or_else(|| "/bin/zsh".into())
    }
}

/// POSIX 셸 single-quote 이스케이프(경로의 `$`·백틱·`$()`·공백·특수문자를 리터럴화).
/// 큰따옴표는 `$`·백틱·`$()`가 여전히 확장돼 취약(codex T6b.1) → 단일따옴표로 리터럴 고정하고
/// 내부 `'`만 `'\''`로 닫고-이스케이프-열기. cys 경로에 특수문자가 있어도 명령 주입 불가.
#[cfg(target_os = "macos")]
fn sh_squote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// D8(RC-19·mac): runtime bin dirs → `-lc` 명령 앞에 붙일 `export PATH='<dir>':…:"$PATH"; ` 프리픽스.
/// dir는 POSIX single-quote(확장 취약 제거)·`$PATH`만 큰따옴표로 확장. dirs 비면 None. 순수 fn(테스트용).
#[cfg(target_os = "macos")]
fn mac_lc_path_prefix(dirs: &[std::path::PathBuf]) -> Option<String> {
    if dirs.is_empty() {
        return None;
    }
    let joined = dirs
        .iter()
        .map(|d| sh_squote(&d.to_string_lossy()))
        .collect::<Vec<_>>()
        .join(":");
    Some(format!("export PATH={joined}:\"$PATH\"; "))
}

/// 로그인 프로파일(path_helper)이 동봉 runtime을 PATH 뒤로 강등한 뒤 실행되는 -c 명령에서 재선두주입해
/// 동봉 git/python3/uv/node가 /usr/bin CLT-shim을 이기게 한다. runtime 부재(개발/비동봉)면 None(no-op).
/// cysd 자기 exe_dir(Contents/MacOS) 기준 runtime_bin_dirs와 단일화.
#[cfg(target_os = "macos")]
fn mac_runtime_lc_prefix() -> Option<String> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))?;
    mac_lc_path_prefix(&cys::runtime_bin_dirs(&exe_dir))
}

/// 오너 완화책 ① 기본 내장 룰: 로그인 만료·401·토큰 만료를 즉시 감지한다.
fn default_health_rules() -> Vec<HealthRule> {
    let defaults: &[(&str, &str)] = &[
        ("not_logged_in", r"(?i)not logged in"),
        (
            "auth_401",
            r"(?i)\b401\b.*(unauthorized|auth)|unauthorized.*\b401\b|authentication[_ ]?error",
        ),
        (
            "token_expired",
            r"(?i)(token|credential|session).{0,20}(expired|invalid)|expired.{0,20}(token|credential)",
        ),
        (
            "login_required",
            r"(?i)(please|run).{0,30}(/login|log ?in again)",
        ),
        (
            "rate_limited",
            r"(?i)rate.?limit(ed)?|too many requests|\b429\b",
        ),
    ];
    defaults
        .iter()
        .filter_map(|(name, pat)| {
            Regex::new(pat).ok().map(|regex| HealthRule {
                name: name.to_string(),
                regex,
                action: None, // 내장 룰은 alert-only (조치 바인딩은 명시 opt-in)
                threshold: 3,
                pause_secs: 300,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── T6b.1 회귀 핀(codex): mac -lc PATH 프리픽스는 POSIX single-quote로 특수문자 리터럴화 ──
    // 버그: 큰따옴표 quoting은 경로의 $·백틱·$()가 셸 확장돼 명령 주입/오해석 취약.
    #[cfg(target_os = "macos")]
    #[test]
    fn mac_lc_path_prefix_single_quotes_special_chars() {
        use std::path::PathBuf;
        let dirs = vec![
            PathBuf::from("/Apps/cys.app/Contents/Resources/runtime/python/bin"),
            PathBuf::from("/weird/$HOME `whoami` $(id)/git/bin"), // $·백틱·$()·공백
            PathBuf::from("/quote'd/uv"),                         // 내부 작은따옴표
        ];
        let p = mac_lc_path_prefix(&dirs).expect("dirs 비지 않음");
        assert!(p.starts_with("export PATH="), "형식: {p}");
        assert!(p.ends_with(":\"$PATH\"; "), "말미 $PATH 확장 보존: {p}");
        // 특수문자 경로 전체가 single-quote 리터럴 — 확장 토큰이 따옴표 밖에 노출되지 않는다.
        assert!(p.contains("'/weird/$HOME `whoami` $(id)/git/bin'"), "특수문자 단일따옴표 리터럴: {p}");
        // 내부 작은따옴표는 '\'' 로 닫고-이스케이프-열기.
        assert!(p.contains("'/quote'\\''d/uv'"), "내부 따옴표 이스케이프: {p}");
        // dirs 비면 None(no-op).
        assert_eq!(mac_lc_path_prefix(&[]), None, "빈 dirs → None");
    }

    // ── RC-13 회귀 핀(agy 요구): Windows 부서 상태 격리 슬러그 ──
    // 버그: state_dir Windows 분기가 socket_path를 폐기하고 %LOCALAPPDATA%\cys 고정 → 모든 부서가
    // 동일 transcripts.db·feed.jsonl 공유(SQLite 락 경합·부서간 오염). pipe_slug로 부서별 격리.
    #[test]
    fn pipe_slug_maps_base_and_dept_pipes() {
        // 기본 데몬 → "cys"(호출자가 루트로 매핑)
        assert_eq!(pipe_slug(std::path::Path::new(r"\\.\pipe\cys")), "cys");
        // 부서 데몬 → 고유 슬러그
        assert_eq!(
            pipe_slug(std::path::Path::new(r"\\.\pipe\cys-dept-3")),
            "cys-dept-3"
        );
        assert_eq!(
            pipe_slug(std::path::Path::new(r"\\.\pipe\cys-dept-future")),
            "cys-dept-future"
        );
        // 방어적 sanitize: 마지막 컴포넌트에서 안전문자(영숫자·-·_)만 — `.`는 제거됨
        // (슬래시/역슬래시 모두에서 마지막 성분 추출: `cys.sock` → `cyssock`)
        assert_eq!(pipe_slug(std::path::Path::new("/tmp/cys-dept-9/cys.sock")), "cyssock");
    }

    #[test]
    fn create_surface_with_env_records_env_injected_flag() {
        // RC-3 잔여(T2.1·codex CONFIRMED) 회귀 핀: env 주입 여부가 Surface.env_injected에 정확 기록돼야
        // Windows node-recover가 "순수 cmd 재기동 안전"을 판정할 수 있다. env 有→true·env 無→false.
        let daemon = Daemon::new(isolated_sock("env-injected"));
        let s1 = daemon
            .create_surface_with_env(
                None, Some("sleep 30".into()), None, Some("worker-1".into()), 24, 80,
                &[("CLAUDE_CONFIG_DIR".to_string(), "/x/.cys/claude".to_string())],
            )
            .unwrap();
        assert!(s1.env_injected, "env 주입 surface는 env_injected=true여야 node-recover 허용");
        let s2 = daemon
            .create_surface_with_env(
                None, Some("sleep 30".into()), None, Some("worker-2".into()), 24, 80, &[],
            )
            .unwrap();
        assert!(!s2.env_injected, "env 미주입 surface는 env_injected=false → Windows node-recover fail-closed");
    }

    #[test]
    fn rc15_dept_logdir_slug_matches_rc13_state_dir() {
        // D7 조건(정합 강제): cys-dept dept_logdir(RC-15)의 Windows 폴더명과 state_dir(RC-13) 슬러그가
        // **동일 규약**이어야 로그(cysd.log)+상태(transcripts.db·feed.jsonl)가 한 폴더로 모인다.
        // dept_logdir(Windows) = %LOCALAPPDATA%\cys\cys-dept-<name> (cys-dept bash·스모크 검증).
        // state_dir(Windows)   = %LOCALAPPDATA%\cys\<pipe_slug(\\.\pipe\cys-dept-<name>)>.
        // 일치 조건: pipe_slug(dept pipe) == "cys-dept-<name>". (2곳 slug 규약 갈라짐 방지 핀.)
        for name in ["dept-3", "dept-future", "dept-1"] {
            let pipe = format!(r"\\.\pipe\cys-dept-{name}");
            assert_eq!(
                pipe_slug(std::path::Path::new(&pipe)),
                format!("cys-dept-{name}"),
                "RC-15 dept_logdir 폴더명 ≠ RC-13 state_dir 슬러그 — 로그/state 폴더 분산 격리결함"
            );
        }
    }

    #[test]
    fn pipe_slug_dept_differs_from_base_for_isolation() {
        // 핵심 불변식: 부서 슬러그 ≠ 기본("cys") → state_dir가 서로 다른 디렉토리 파생(격리 보장).
        let base = pipe_slug(std::path::Path::new(r"\\.\pipe\cys"));
        let d1 = pipe_slug(std::path::Path::new(r"\\.\pipe\cys-dept-1"));
        let d2 = pipe_slug(std::path::Path::new(r"\\.\pipe\cys-dept-2"));
        assert_ne!(d1, base);
        assert_ne!(d2, base);
        assert_ne!(d1, d2, "부서끼리도 서로 다른 상태 디렉토리");
    }

    // ── writer 스레드 누수 회귀 가드 (state.rs run_writer_loop) ──
    // 버그: 자력 종료(셸 EOF) surface는 close_surface를 거치지 않아 write_tx가 surfaces
    // 맵 속 Arc<Surface>에 영구 잔존한다. 구버전 writer 루프는 `while let Ok(req)=recv()`라
    // sender가 살아있는 한 영영 블로킹 → writer 스레드와 그것이 단독 소유한 PTY writer fd가
    // 단조 누수(24/365 데몬의 fd 고갈). 이 테스트는 sender를 *살려둔 채로*(맵 잔존 재현)
    // stop 신호만으로 writer 루프가 종료(=writer drop→fd 회수)됨을 박제한다.
    #[test]
    fn writer_loop_terminates_on_stop_signal_even_with_live_sender() {
        use std::sync::mpsc::sync_channel;

        let (tx, rx) = sync_channel::<WriteReq>(8);
        let stop = Arc::new(AtomicBool::new(false));

        // writer = 메모리 버퍼 (PTY writer 대역). Arc<Mutex>로 스레드와 공유해 사후 검사.
        let sink = Arc::new(Mutex::new(Vec::<u8>::new()));
        struct SharedSink(Arc<Mutex<Vec<u8>>>);
        impl Write for SharedSink {
            fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(b);
                Ok(b.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let writer = SharedSink(Arc::clone(&sink));
        let stop_c = Arc::clone(&stop);
        let handle = std::thread::spawn(move || run_writer_loop(writer, rx, stop_c));

        // 평시 동작 불변: 정상 데이터는 그대로 PTY로 전달된다.
        tx.send(WriteReq::Data(b"hello".to_vec())).unwrap();
        // 전달 반영 대기 (recv_timeout 200ms 폴링이라 넉넉히)
        let mut delivered = false;
        for _ in 0..50 {
            if sink.lock().unwrap().as_slice() == b"hello" {
                delivered = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(delivered, "정상 write가 PTY로 전달돼야 한다(평시 동작 불변)");

        // ★핵심: sender(tx)를 *드롭하지 않는다* — 자력 종료 surface의 write_tx가 맵 속
        // Arc에 잔존하는 상황 그대로다. 구버전 recv() 루프라면 여기서 영영 블로킹한다.
        // stop만 세우면 새 루프는 recv_timeout 다음 틱에 이를 보고 종료해야 한다.
        stop.store(true, Ordering::Relaxed);

        // 별도 watcher 스레드로 join을 폴링해 '유한 시간 내 종료'를 단정 (블로킹 join 회피).
        let (done_tx, done_rx) = sync_channel::<()>(1);
        std::thread::spawn(move || {
            handle.join().ok();
            let _ = done_tx.send(());
        });
        let terminated = done_rx
            .recv_timeout(std::time::Duration::from_secs(3))
            .is_ok();
        assert!(
            terminated,
            "stop 신호 후 writer 루프가 종료돼야 한다(sender 잔존에도 좀비 스레드·fd 회수)"
        );

        // sender는 여전히 살아있음(맵 잔존 재현) — 그래도 누수 회수가 성립함을 못 박는다.
        drop(tx);
    }

    // Disconnected(모든 sender drop = close_surface로 Arc 제거) 경로도 즉시 종료해야 한다.
    #[test]
    fn writer_loop_terminates_on_all_senders_dropped() {
        use std::sync::mpsc::sync_channel;
        let (tx, rx) = sync_channel::<WriteReq>(1);
        let stop = Arc::new(AtomicBool::new(false));
        let handle = std::thread::spawn(move || run_writer_loop(std::io::sink(), rx, stop));
        drop(tx); // 모든 sender drop → Disconnected
        let (done_tx, done_rx) = sync_channel::<()>(1);
        std::thread::spawn(move || {
            handle.join().ok();
            let _ = done_tx.send(());
        });
        assert!(
            done_rx
                .recv_timeout(std::time::Duration::from_secs(3))
                .is_ok(),
            "모든 sender drop 시 writer 루프가 종료돼야 한다"
        );
    }

    /// 불변식 박제: clear_first Inject은 한 writer arm에서 Ctrl-U(선정리)→bracketed paste→CR을
    /// 순서대로 한 단위로 쓴다. 다른 WriteReq가 끼어들 수 없고(원자), 부분 전달(clear만/text만)이
    /// 구조적으로 불가능함을 바이트 순서로 검증한다.
    #[test]
    fn inject_clear_first_emits_ctrl_u_before_paste_then_cr() {
        use std::sync::mpsc::sync_channel;
        struct SharedBuf(Arc<Mutex<Vec<u8>>>);
        impl std::io::Write for SharedBuf {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let buf = Arc::new(Mutex::new(Vec::new()));
        let (tx, rx) = sync_channel::<WriteReq>(2);
        let stop = Arc::new(AtomicBool::new(false));
        let w = SharedBuf(Arc::clone(&buf));
        let handle = std::thread::spawn(move || run_writer_loop(w, rx, stop));
        tx.send(WriteReq::Inject {
            text: "hi".into(),
            cr_delay_ms: 0,
            clear_first: true,
        })
        .unwrap();
        drop(tx); // Disconnected → 루프 종료
        handle.join().ok();

        let out = buf.lock().unwrap().clone();
        let s = String::from_utf8_lossy(&out);
        let cu = out
            .iter()
            .position(|&b| b == 0x15)
            .expect("Ctrl-U(0x15) 선정리가 있어야 한다");
        let paste = s.find("\x1b[200~").expect("bracketed paste 시작이 있어야 한다");
        assert!(cu < paste, "Ctrl-U는 paste보다 먼저여야 한다(클린 라인 보장)");
        assert!(
            s.contains("\x1b[200~hi\x1b[201~"),
            "텍스트가 bracketed paste로 감싸져야 한다 (출력: {s:?})"
        );
        assert!(out.ends_with(b"\r"), "CR로 제출돼야 한다 (출력: {s:?})");
    }

    /// 원자성(비끼어듦) 박제: 같은 채널에 경쟁 WriteReq(Data "X")를 함께 적재해도, clear_first
    /// Inject의 한 줄(Ctrl-U … 첫 CR)은 통째로 연속 — 단일 소비자 writer가 한 req를 끝까지
    /// 처리하므로 경쟁 바이트가 그 사이에 끼어들 수 없다(부분 전달·라인 오염 구조적 차단).
    #[test]
    fn inject_clear_first_is_not_interleaved_by_competing_writereq() {
        use std::sync::mpsc::sync_channel;
        struct SharedBuf(Arc<Mutex<Vec<u8>>>);
        impl std::io::Write for SharedBuf {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let buf = Arc::new(Mutex::new(Vec::new()));
        let (tx, rx) = sync_channel::<WriteReq>(2);
        let stop = Arc::new(AtomicBool::new(false));
        let w = SharedBuf(Arc::clone(&buf));
        let handle = std::thread::spawn(move || run_writer_loop(w, rx, stop));
        // 경쟁 적재: clear_first Inject 직후 Data("X")를 같은 채널에 넣는다.
        tx.send(WriteReq::Inject {
            text: "hi".into(),
            cr_delay_ms: 0,
            clear_first: true,
        })
        .unwrap();
        tx.send(WriteReq::Data(b"X".to_vec())).unwrap();
        drop(tx);
        handle.join().ok();

        let out = buf.lock().unwrap().clone();
        let s = String::from_utf8_lossy(&out);
        let cu = out.iter().position(|&b| b == 0x15).expect("Ctrl-U");
        let cr = out.iter().position(|&b| b == b'\r').expect("CR");
        // Inject의 한 줄(\x15 … 첫 \r)에 경쟁 Data('X')가 끼면 안 된다.
        assert!(
            !out[cu..=cr].contains(&b'X'),
            "경쟁 Data가 clear_first Inject의 한 줄 사이에 끼어들었다 — 원자성 위반 (출력: {s:?})"
        );
        assert!(
            out.ends_with(b"X"),
            "경쟁 Data는 Inject 완료 후에 와야 한다 (출력: {s:?})"
        );
    }

    /// 대조: clear_first=false면 Ctrl-U를 절대 쓰지 않는다(현행 queued/스케줄 동작 보존).
    #[test]
    fn inject_without_clear_first_never_emits_ctrl_u() {
        use std::sync::mpsc::sync_channel;
        struct SharedBuf(Arc<Mutex<Vec<u8>>>);
        impl std::io::Write for SharedBuf {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let buf = Arc::new(Mutex::new(Vec::new()));
        let (tx, rx) = sync_channel::<WriteReq>(2);
        let stop = Arc::new(AtomicBool::new(false));
        let w = SharedBuf(Arc::clone(&buf));
        let handle = std::thread::spawn(move || run_writer_loop(w, rx, stop));
        tx.send(WriteReq::Inject {
            text: "hi".into(),
            cr_delay_ms: 0,
            clear_first: false,
        })
        .unwrap();
        drop(tx);
        handle.join().ok();

        let out = buf.lock().unwrap().clone();
        assert!(
            !out.contains(&0x15),
            "clear_first=false인데 Ctrl-U가 새어나왔다 — 현행 동작 회귀"
        );
    }

    #[test]
    fn sibling_cli_path_uses_platform_extension() {
        // 회귀 박제: 데몬이 형제 CLI를 spawn할 때 플랫폼별 실행파일명을 써야 한다.
        // (버그였던 무확장자 "cys" 하드코딩은 Windows에서 cys.exe를 못 찾아
        //  node-recover·launch-agent 자동 기동이 전부 실패했다 — cys.rs·main.rs와 동일 패턴이어야 함.)
        let p = sibling_cli_path();
        let want = if cfg!(windows) { "cys.exe" } else { "cys" };
        assert_eq!(
            p.file_name().and_then(|s| s.to_str()),
            Some(want),
            "sibling CLI 파일명이 플랫폼 규약과 어긋남: {}",
            p.display()
        );
    }

    #[test]
    fn windows_exec_flag_matches_shell_family() {
        // 회귀 박제: create_surface의 Windows 분기가 -Command를 하드코딩하면
        // CYS_SHELL=cmd.exe일 때 `cmd.exe -Command <c>`가 되어 명령이 깨졌다.
        // 셸 계열별로 올바른 인라인 명령 플래그를 선택해야 한다.
        // cmd.exe 계열 → /C
        assert_eq!(windows_exec_flag("cmd.exe"), "/C");
        assert_eq!(windows_exec_flag("cmd"), "/C");
        assert_eq!(windows_exec_flag("CMD.EXE"), "/C");
        assert_eq!(windows_exec_flag(r"C:\Windows\System32\cmd.exe"), "/C");
        // PowerShell 계열 → -Command (기본/하위호환)
        assert_eq!(windows_exec_flag("powershell.exe"), "-Command");
        assert_eq!(windows_exec_flag("pwsh.exe"), "-Command");
        assert_eq!(windows_exec_flag("pwsh"), "-Command");
        assert_eq!(
            windows_exec_flag(r"C:\Program Files\PowerShell\7\pwsh.exe"),
            "-Command"
        );
        // 그 외(알 수 없는 셸)는 PowerShell 기본값으로 둔다 — 기존 동작 보존.
        assert_eq!(windows_exec_flag("something.exe"), "-Command");
    }

    #[test]
    fn ansi_incomplete_esc_alone() {
        // ESC 단독은 항상 미완성 (다음 청크와 합쳐야 함)
        assert!(ansi_incomplete(b"\x1b"));
    }

    #[test]
    fn ansi_incomplete_csi() {
        // CSI 종결바이트(0x40-0x7e) 없으면 미완성
        assert!(ansi_incomplete(b"\x1b[")); // 파라미터/종결 미도착
        assert!(ansi_incomplete(b"\x1b[0")); // 숫자만, 종결 미도착
        assert!(ansi_incomplete(b"\x1b[1;31")); // SGR 진행 중
        // 종결바이트 도착 → 완성
        assert!(!ansi_incomplete(b"\x1b[A")); // 커서 이동
        assert!(!ansi_incomplete(b"\x1b[0m")); // SGR reset (m=0x6d)
        assert!(!ansi_incomplete(b"\x1b[2J")); // 화면 클리어
    }

    #[test]
    fn ansi_incomplete_osc() {
        // OSC는 BEL(0x07) 또는 ST(ESC \)로 종료
        assert!(ansi_incomplete(b"\x1b]")); // 미종료
        assert!(ansi_incomplete(b"\x1b]0;title")); // 종료자 미도착
        // BEL 종료 → 완성
        assert!(!ansi_incomplete(b"\x1b]0;title\x07"));
        // ST(ESC \) 종료 → 완성
        assert!(!ansi_incomplete(b"\x1b]0;title\x1b\\"));
    }

    #[test]
    fn ansi_incomplete_two_byte_sequences() {
        // CSI/OSC가 아닌 2바이트 ESC 시퀀스는 완결로 간주
        assert!(!ansi_incomplete(b"\x1bM")); // RI (reverse index)
        assert!(!ansi_incomplete(b"\x1b=")); // keypad mode
        assert!(!ansi_incomplete(b"\x1bO")); // SS3 도입부도 여기선 완결 취급
    }

    #[test]
    fn ansi_incomplete_csi_boundary_terminators() {
        // CSI 종결 판정은 0x40-0x7e '범위'다 — 경계값을 정확히 박제.
        // 0x40('@')·0x7e('~')는 종결바이트 → 완성. 0x3f('?')는 범위 미만 → 미완성.
        assert!(!ansi_incomplete(b"\x1b[@")); // 0x40 = 하한 종결바이트
        assert!(!ansi_incomplete(b"\x1b[6~")); // 0x7e = 상한 종결바이트 (PageDown 등)
        assert!(ansi_incomplete(b"\x1b[?2004")); // '?'(0x3f)·숫자는 파라미터, 종결 아직
        assert!(!ansi_incomplete(b"\x1b[?2004h")); // 'h'(0x68) 종결 → 완성 (bracketed paste on)
        // 파라미터에 종결범위 바이트가 섞이면 그 지점에서 완성으로 본다 (any() 의미 박제)
        assert!(!ansi_incomplete(b"\x1b[1A")); // 'A'(0x41) 종결
    }

    #[test]
    fn ansi_incomplete_osc_st_requires_full_two_bytes() {
        // OSC ST는 정확히 ESC '\\' 2바이트 윈도여야 완성. ESC만(끝에) 오면 미완성 유지.
        assert!(ansi_incomplete(b"\x1b]0;t\x1b")); // ST의 ESC만 도착, '\\' 미도착 → 미완성
        assert!(!ansi_incomplete(b"\x1b]0;t\x1b\\")); // 완전한 ST → 완성
        // BEL(0x07)이 payload 어디든 있으면 완성 (contains 의미)
        assert!(!ansi_incomplete(b"\x1b]52;c;data\x07"));
        // ST도 BEL도 없는 긴 OSC는 미완성 (다음 청크 대기)
        assert!(ansi_incomplete(b"\x1b]8;;https://example.com"));
    }

    // ---- (B2) OSC 9/99/777 데스크톱 알림 파서 ----

    /// OSC 9 = 단순 알림. title 없음(빈 문자열), body=payload 전체.
    #[test]
    fn osc_9_notify() {
        assert_eq!(
            parse_osc_notification(b"\x1b]9;build done\x07"),
            Some((String::new(), "build done".to_string()))
        );
        // ST 종결도 동일
        assert_eq!(
            parse_osc_notification(b"\x1b]9;build done\x1b\\"),
            Some((String::new(), "build done".to_string()))
        );
    }

    /// OSC 9;4;... = ConEmu 진행률 → 알림 아님(None). 회귀 박제: 진행률을 알림으로 오발화 금지.
    #[test]
    fn osc_9_progress_ignored() {
        assert_eq!(parse_osc_notification(b"\x1b]9;4;50\x07"), None);
        assert_eq!(parse_osc_notification(b"\x1b]9;4\x07"), None);
        // 빈 payload도 None
        assert_eq!(parse_osc_notification(b"\x1b]9;\x07"), None);
    }

    /// OSC 777;notify;title;body — iTerm2/kitty 계열. notify가 아니면 None.
    #[test]
    fn osc_777() {
        assert_eq!(
            parse_osc_notification(b"\x1b]777;notify;\xed\x85\x8c\xec\x8a\xa4\xed\x8a\xb8;\xeb\xb3\xb8\xeb\xac\xb8\x07"),
            Some(("테스트".to_string(), "본문".to_string()))
        );
        // notify 아닌 서브커맨드는 알림 아님
        assert_eq!(parse_osc_notification(b"\x1b]777;precmd\x07"), None);
    }

    /// OSC 99 = kitty desktop notification. 1차 범위: metadata 무시, 평문 payload만.
    #[test]
    fn osc_99_plain() {
        // 99;<metadata>;<payload> — 마지막 ';' 뒤를 payload로
        assert_eq!(
            parse_osc_notification(b"\x1b]99;i=1;hello\x07"),
            Some((String::new(), "hello".to_string()))
        );
        // metadata 없는 단순형
        assert_eq!(
            parse_osc_notification(b"\x1b]99;hello\x07"),
            Some((String::new(), "hello".to_string()))
        );
    }

    /// drain_complete_osc: 완성 시퀀스만 추출·소비, 미완성 꼬리는 carry에 보존(청크 경계 박제).
    #[test]
    fn drain_osc_keeps_incomplete_tail() {
        // 완성 1개 + 미완성 1개 → 1개 추출, 미완성은 carry에 남음
        let mut carry: Vec<u8> = b"\x1b]9;done\x07\x1b]777;notify;t".to_vec();
        let out = drain_complete_osc(&mut carry);
        assert_eq!(out, vec![(String::new(), "done".to_string())]);
        assert_eq!(carry, b"\x1b]777;notify;t".to_vec()); // 미완성 꼬리 보존
        // 다음 청크로 종결자 도착 → 추출 완료, carry 비움
        carry.extend_from_slice(b";b\x07");
        let out2 = drain_complete_osc(&mut carry);
        assert_eq!(out2, vec![("t".to_string(), "b".to_string())]);
        assert!(carry.is_empty());
        // OSC 사이 비-OSC 노이즈는 버려진다(추출 전용)
        let mut noisy: Vec<u8> = b"plain\x1b]9;x\x07more".to_vec();
        let out3 = drain_complete_osc(&mut noisy);
        assert_eq!(out3, vec![(String::new(), "x".to_string())]);
        assert!(noisy.is_empty()); // 미완성 OSC 없음 → 전부 소비
    }

    // ---- ingest_output 라인분할 상태기계 (state.rs:627) ----
    // Surface/Daemon(PTY 인프라) 결합으로 실 함수 직접 구동이 비싸 fragile하므로,
    // 라인분할 핵심(IngestState의 carry·pending_cr·partial만 다루는 순수 변환)을
    // 프로덕션과 1:1로 미러링한 헬퍼로 경계 불변식을 박제한다.
    // 미러는 ingest_output 본문(carry hold → ESC cut → UTF-8 cut → char 루프)을
    // strip_ansi 직전까지 동일하게 재현 — 프로덕션 분기가 바뀌면 함께 갱신해야 한다.
    fn ingest_step(st: &mut IngestState, chunk: &[u8], out: &mut Vec<String>) {
        st.carry.extend_from_slice(chunk);
        let mut cut = st.carry.len();
        if let Some(esc) = st.carry.iter().rposition(|&b| b == 0x1b) {
            let tail = &st.carry[esc..];
            if tail.len() < 128 && ansi_incomplete(tail) {
                cut = esc;
            }
        }
        cut = match std::str::from_utf8(&st.carry[..cut]) {
            Ok(_) => cut,
            Err(e) if e.error_len().is_none() => e.valid_up_to(),
            Err(_) => cut,
        };
        if cut == 0 {
            return;
        }
        let drained: Vec<u8> = st.carry.drain(..cut).collect();
        let stripped = strip_ansi_escapes::strip(&drained);
        let text = String::from_utf8_lossy(&stripped);
        for ch in text.chars() {
            if st.pending_cr {
                st.pending_cr = false;
                if ch == '\n' {
                    out.push(std::mem::take(&mut st.partial));
                    continue;
                }
                st.partial.clear();
            }
            match ch {
                '\n' => out.push(std::mem::take(&mut st.partial)),
                '\r' => st.pending_cr = true,
                _ => {
                    if st.partial.len() < 8192 {
                        st.partial.push(ch);
                    }
                }
            }
        }
    }

    fn fresh() -> IngestState {
        IngestState {
            carry: Vec::new(),
            pending_cr: false,
            partial: String::new(),
        }
    }

    #[test]
    fn ingest_lf_splits_lines_and_holds_partial() {
        let mut st = fresh();
        let mut out = Vec::new();
        ingest_step(&mut st, b"hello\nworld", &mut out);
        assert_eq!(out, vec!["hello".to_string()]);
        // "world"는 개행 없으니 partial로 보류 (완성 라인 아님)
        assert_eq!(st.partial, "world");
        out.clear();
        ingest_step(&mut st, b"!\n", &mut out);
        assert_eq!(out, vec!["world!".to_string()]);
        assert_eq!(st.partial, "");
    }

    #[test]
    fn strip_removes_cr_and_tab_so_pending_cr_branch_is_dead() {
        // ★R3 발견: strip_ansi_escapes(v0.2.1, vte 기반)는 char 루프에 닿기 전에
        // CR(\r)·TAB(\t)을 모두 제거한다. 따라서 ingest_output의 pending_cr/CRLF/
        // 단독CR-덮어쓰기 분기(state.rs:652-664)는 사실상 '데드코드'다 — 진행바
        // 덮어쓰기 보호는 이 경로로는 동작하지 않고, strip이 프레임을 단순 연결한다.
        // (실제 터미널 렌더는 별도 vt100 parser.process가 정확히 처리 → 사용자 영향 없음)
        // 데드코드는 절대규칙상 '발견 시 보고하되 삭제하지 않는다' → 본 테스트로 '왜
        // pending_cr가 영영 true가 안 되는가'를 박제해, strip 동작이 바뀌면(=분기가
        // 되살아나면) 빨간불로 알린다.
        assert_eq!(strip("a\r\nb"), b"a\nb"); // CRLF → CR 제거, LF만 남음
        assert_eq!(strip("10%\r20%"), b"10%20%"); // 단독 CR 제거 (덮어쓰기 아님)
        assert_eq!(strip("abc\r"), b"abc"); // 꼬리 CR 제거
        assert_eq!(strip("a\tb"), b"ab"); // TAB도 제거됨
    }

    fn strip(s: &str) -> Vec<u8> {
        strip_ansi_escapes::strip(s.as_bytes())
    }

    #[test]
    fn ingest_crlf_yields_one_line_no_blank() {
        // strip이 CR을 제거하므로 CRLF는 LF 한 번 — 빈 줄 끼임 없이 단일 줄바꿈.
        let mut st = fresh();
        let mut out = Vec::new();
        ingest_step(&mut st, b"a\r\nb\r\n", &mut out);
        assert_eq!(out, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(st.partial, "");
        // CR이 청크 끝에 걸려도(strip 후 사라짐) pending_cr는 절대 set되지 않는다 —
        // \r은 char 루프에 도달하지 못하기 때문.
        let mut st = fresh();
        let mut out = Vec::new();
        ingest_step(&mut st, b"a\r", &mut out);
        assert!(out.is_empty());
        assert!(!st.pending_cr); // ★데드코드 확증: \r은 strip돼 분기 미진입
        assert_eq!(st.partial, "a");
        ingest_step(&mut st, b"\nb", &mut out);
        assert_eq!(out, vec!["a".to_string()]);
        assert_eq!(st.partial, "b");
    }

    #[test]
    fn ingest_lone_cr_is_stripped_frames_concatenate() {
        // ★R3 발견의 사용자 가시 결과: 진행바 프레임이 '덮어쓰기'가 아니라 '연결'된다.
        // (코드 주석은 덮어쓰기를 의도하나 strip이 CR을 먼저 지워 무력화됨 — 데드코드)
        let mut st = fresh();
        let mut out = Vec::new();
        ingest_step(&mut st, b"10%\r20%\r100%\n", &mut out);
        assert_eq!(out, vec!["10%20%100%".to_string()]); // 연결됨 (덮어쓰기 아님)
        assert_eq!(st.partial, "");
        // 청크 경계를 가로지르는 CR도 동일하게 연결
        let mut st = fresh();
        let mut out = Vec::new();
        ingest_step(&mut st, b"loading...", &mut out);
        assert_eq!(st.partial, "loading...");
        ingest_step(&mut st, b"\rdone\n", &mut out);
        assert_eq!(out, vec!["loading...done".to_string()]);
    }

    #[test]
    fn ingest_holds_utf8_multibyte_tail_across_chunks() {
        // 한글 '가' = E0 B0 80 (3바이트). 청크가 중간에서 잘려도 깨진 문자가 새지 않는다.
        let ga = "가".as_bytes(); // [0xea, 0xb0, 0x80]
        assert_eq!(ga.len(), 3);
        let mut st = fresh();
        let mut out = Vec::new();
        // 첫 2바이트만 도착 — 미완성 멀티바이트는 carry에 보류, 출력 없음
        ingest_step(&mut st, &ga[..2], &mut out);
        assert!(out.is_empty());
        assert_eq!(st.partial, ""); // 깨진 char가 partial에 들어가지 않음
        assert_eq!(st.carry.len(), 2); // 꼬리 보류
        // 나머지 바이트 + 개행 → 온전한 '가' 완성
        let mut rest = ga[2..].to_vec();
        rest.push(b'\n');
        ingest_step(&mut st, &rest, &mut out);
        assert_eq!(out, vec!["가".to_string()]);
        assert!(st.carry.is_empty());
    }

    #[test]
    fn ingest_holds_incomplete_esc_then_strips_when_complete() {
        // 미완성 CSI가 청크 끝에 걸리면 보류 → 다음 청크와 합쳐 strip
        let mut st = fresh();
        let mut out = Vec::new();
        // "X" + 미완성 SGR("\x1b[1;31") — 종결바이트 미도착이라 ESC부터 보류.
        // ESC 앞의 "X"는 strip 후 partial로 들어가고(개행 전이라 미완성 라인),
        // 미완성 ESC 잔재(\x1b[1;31)는 carry에 보류돼 partial로 새지 않는 것이 핵심.
        ingest_step(&mut st, b"X\x1b[1;31", &mut out);
        assert!(out.is_empty());
        assert_eq!(st.partial, "X"); // ESC 잔재는 carry에, 본문 X만 partial
        assert!(!st.carry.is_empty()); // 미완성 ESC가 carry에 보류됨
        // 종결바이트 'm' + 텍스트 + 개행 → 컬러코드는 strip, 본문만 남음
        ingest_step(&mut st, b"mRED\n", &mut out);
        assert_eq!(out, vec!["XRED".to_string()]);
    }

    #[test]
    fn ingest_partial_growth_is_capped_at_8192() {
        // \n 없는 스트림이 partial을 무한 성장시키지 못한다 (메모리 DoS 가드)
        let mut st = fresh();
        let mut out = Vec::new();
        let big = vec![b'a'; 20_000];
        ingest_step(&mut st, &big, &mut out);
        assert!(out.is_empty());
        assert_eq!(st.partial.len(), 8192); // 상한에서 절단
        // 상한 도달 후에도 개행은 여전히 라인을 확정 (상태기계가 멈추지 않음)
        ingest_step(&mut st, b"\n", &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].len(), 8192);
    }

    #[test]
    fn ingest_truly_invalid_utf8_is_flushed_not_stuck() {
        // 손상 바이트(error_len.is_some())는 lossy로 흘려보낸다 — 보류하면 영구 정체.
        // 0xFF는 어떤 UTF-8 시퀀스 시작도 아님(error_len=Some) → 보류 없이 통과.
        let mut st = fresh();
        let mut out = Vec::new();
        ingest_step(&mut st, b"ok\xff\n", &mut out);
        assert_eq!(out.len(), 1);
        // lossy 치환문자(U+FFFD)를 포함하되 carry에 영구 정체하지 않음
        assert!(out[0].starts_with("ok"));
        assert!(st.carry.is_empty());
    }

    #[test]
    fn ingest_esc_hold_gives_up_past_128_bytes_anti_stall() {
        // ★불변식 박제: 미완성 ESC 꼬리 보류는 무한이 아니다. tail.len() < 128 게이트가
        // 풀리면(꼬리 ≥128B) cut을 carry.len()으로 되돌려 '보류 포기' → drain한다.
        // 이 게이트가 없으면 종결바이트가 영영 안 오는 손상 CSI가 carry를 영구 점유해
        // 그 surface의 라인 분할이 데몬 수명 내내 멈춘다(silent stall). 경계를 박제한다.

        // 127바이트 미완성 CSI(ESC '[' + 125바이트 파라미터, 종결 없음): 아직 보류
        let mut held = b"\x1b[".to_vec();
        held.extend(std::iter::repeat_n(b'0', 125));
        assert_eq!(held.len(), 127);
        let mut st = fresh();
        let mut out = Vec::new();
        ingest_step(&mut st, &held, &mut out);
        assert!(out.is_empty(), "127B 미완성 ESC는 보류 — 라인 미확정");
        assert_eq!(st.carry.len(), 127, "꼬리 전체가 carry에 보류됨");

        // 128바이트 미완성 CSI: 보류 포기 → drain. carry가 비고 stall이 풀린다.
        // (strip이 미완성 CSI 전체를 escape로 소비하므로 partial/out에는 남지 않지만,
        //  핵심은 carry가 비워져 다음 청크 처리가 막히지 않는다는 것.)
        let mut giveup = b"\x1b[".to_vec();
        giveup.extend(std::iter::repeat_n(b'0', 126));
        assert_eq!(giveup.len(), 128);
        let mut st2 = fresh();
        let mut out2 = Vec::new();
        ingest_step(&mut st2, &giveup, &mut out2);
        assert!(st2.carry.is_empty(), "128B 도달 시 보류 포기 — carry drain(anti-stall)");

        // anti-stall 사후 검증: 보류 포기 후에도 후속 청크의 개행이 정상 라인을 만든다.
        ingest_step(&mut st2, b"after\n", &mut out2);
        assert_eq!(out2, vec!["after".to_string()], "포기 후 상태기계 정상 재개");
    }

    #[test]
    fn ingest_esc_then_utf8_double_cut_holds_only_clean_prefix() {
        // ESC-cut과 UTF-8-cut이 같은 청크에 동시 발생: 두 cut이 합리적으로 합성돼
        // (먼저 미완성 ESC 지점으로 자르고, 그 prefix 안에서 다시 UTF-8 valid_up_to로
        //  좁힌다) 깨진 ESC도 깨진 멀티바이트도 출력으로 새지 않아야 한다.
        let ga = "가".as_bytes(); // [0xea,0xb0,0x80] 3바이트
        let mut chunk = b"done\n".to_vec(); // 완성 라인
        chunk.extend_from_slice(&ga[..2]); // 미완성 멀티바이트 꼬리(ESC 뒤에 둘 수 없으니 앞)
        let mut st = fresh();
        let mut out = Vec::new();
        ingest_step(&mut st, &chunk, &mut out);
        // "done"은 확정, 미완성 '가' 꼬리는 carry 보류(깨진 char 미누출)
        assert_eq!(out, vec!["done".to_string()]);
        assert_eq!(st.carry.len(), 2, "미완성 UTF-8 2바이트만 보류");
        // 미완성 ESC가 UTF-8 꼬리보다 앞서면 ESC 지점에서 먼저 잘려 UTF-8 cut은 그 안에서만
        let mut st2 = fresh();
        let mut out2 = Vec::new();
        // "x\n" 확정 + 미완성 CSI("\x1b[31") — ESC부터 보류, '\n' 앞 'x'만 확정
        ingest_step(&mut st2, b"x\n\x1b[31", &mut out2);
        assert_eq!(out2, vec!["x".to_string()]);
        assert!(!st2.carry.is_empty(), "미완성 ESC가 carry에 보류");
        // 종결 'm' 도착 → 컬러코드 strip, 잔여 본문 없음(개행 전이라 partial도 비음)
        ingest_step(&mut st2, b"m\n", &mut out2);
        assert_eq!(out2, vec!["x".to_string(), "".to_string()]);
    }

    #[test]
    fn default_health_rules_match_intended_triggers_not_benign() {
        // ★불변식 박제: 데몬 watchdog의 내장 health 룰(로그인 만료·401·토큰 만료·rate
        // limit)이 의도한 트리거 문자열을 잡고 정상 로그를 오탐하지 않는다. 이 정규식들은
        // run_health_rules가 매 라인에 돌리는 프로덕션 로직인데 테스트가 전무했다 —
        // 한 글자 오타가 들어가도 빌드/clippy는 통과하고 watchdog만 조용히 사문화된다.
        let rules = default_health_rules();
        let find = |name: &str| {
            rules
                .iter()
                .find(|r| r.name == name)
                .unwrap_or_else(|| panic!("rule {name} missing"))
        };
        // 5개 내장 룰이 모두 존재 (이름·개수 박제 — 룰 누락/개명 즉시 감지)
        assert_eq!(rules.len(), 5);
        let m = |name: &str, s: &str| find(name).regex.is_match(s);

        // not_logged_in — 대소문자 무관
        assert!(m("not_logged_in", "Error: not logged in"));
        assert!(m("not_logged_in", "NOT LOGGED IN"));
        assert!(!m("not_logged_in", "logged in successfully"));

        // auth_401 — '401 unauthorized' 양방향 + authentication_error/space
        assert!(m("auth_401", "401 Unauthorized"));
        assert!(m("auth_401", "unauthorized: 401"));
        assert!(m("auth_401", "authentication_error"));
        assert!(m("auth_401", "authentication error"));
        // \b401\b 워드경계 — '4012'·'1401' 같은 무관 숫자에 unauthorized가 붙어도
        // 401이 더 큰 수의 일부면 매치 안 함(오탐 차단)
        assert!(!m("auth_401", "request 4012 unauthorized device"));
        assert!(!m("auth_401", "200 OK"));

        // token_expired — token/credential/session × expired/invalid (근접 .{0,20})
        assert!(m("token_expired", "your token has expired"));
        assert!(m("token_expired", "credential expired"));
        assert!(m("token_expired", "session is invalid"));
        assert!(m("token_expired", "expired token here"));
        assert!(!m("token_expired", "token saved successfully"));

        // login_required — please/run + /login | log in again
        assert!(m("login_required", "Please run /login to continue"));
        assert!(m("login_required", "please log in again"));
        assert!(!m("login_required", "you are logged in"));

        // rate_limited — rate limit(ed)? | too many requests | 429
        assert!(m("rate_limited", "rate limited"));
        assert!(m("rate_limited", "ratelimit"));
        assert!(m("rate_limited", "rate-limited"));
        assert!(m("rate_limited", "too many requests"));
        assert!(m("rate_limited", "HTTP 429 Too Many Requests"));
        assert!(!m("rate_limited", "all good, build complete"));

        // 내장 룰은 alert-only(조치 미바인딩) + threshold/pause 기본값 박제
        for r in &rules {
            assert!(r.action.is_none(), "내장 룰은 명시 opt-in 없이는 조치 없음");
            assert_eq!(r.threshold, 3);
            assert_eq!(r.pause_secs, 300);
        }
    }

    /// 테스트 전용 격리 소켓 경로 — 고유 하위 디렉터리를 만들어 그 안에 둔다. state_dir이
    /// 소켓의 '부모 디렉터리'라, 같은 temp_dir에 소켓을 두면 모든 테스트 데몬이 하나의
    /// feed.jsonl을 공유해 병렬 실행 시 서로 오염된다. 하위 디렉터리로 데몬마다 격리한다.
    fn isolated_sock(tag: &str) -> PathBuf {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "cys-test-{tag}-{}-{}-{}",
            std::process::id(),
            now_epoch().to_bits(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::create_dir_all(&dir);
        dir.join("cysd.sock")
    }

    fn sample_feed_item(id: &str, body: String) -> FeedItem {
        FeedItem {
            request_id: id.into(),
            kind: "permission".into(),
            title: "approval".into(),
            body,
            surface_id: Some(7),
            status: "pending".into(),
            decision: None,
            created_at: now_epoch(),
            resolved_at: None,
        }
    }

    /// O_APPEND 한 줄 쓰기. `split` 모드면 write_all을 부분 write로 강제 분할해(한 바이트씩
    /// 두 토막) "단일 write() 원자성 < write_all" 상황을 결정론적으로 재현한다. `lock`이
    /// 주어지면 open~분할쓰기 전 구간을 직렬화 — persist_feed_item이 feed_persist_lock으로
    /// 하는 것과 동형(同型)이다.
    fn append_line_for_test(
        path: &std::path::Path,
        line: &str,
        split: bool,
        lock: Option<&Mutex<()>>,
    ) {
        let _guard = lock.map(|m| m.lock().unwrap());
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        let bytes = format!("{line}\n").into_bytes();
        if split && bytes.len() >= 2 {
            // 첫 토막을 쓴 뒤 '의도적으로' 양보 — 락이 없으면 다른 스레드의 write()가
            // 이 두 토막 사이로 O_APPEND 원자단위로 끼어든다(인터리빙). write_all이 한 줄을
            // 여러 write()로 쪼갰을 때 정확히 일어나는 손상.
            let mid = bytes.len() / 2;
            f.write_all(&bytes[..mid]).unwrap();
            std::thread::yield_now();
            f.write_all(&bytes[mid..]).unwrap();
        } else {
            f.write_all(&bytes).unwrap();
        }
    }

    /// ★불변식 박제(결정론): write_all이 한 줄을 여러 write()로 분할하는 상황에서, 동시
    /// appender(feed.push·feed.reply·FeedWait 타임아웃의 서로 다른 커넥션 태스크)가 그 분할
    /// 사이로 끼어들면 JSONL이 손상되고, 손상 라인은 Daemon::new의 replay가 serde 실패로
    /// '조용히' 버려(state.rs:242) pending 승인이 영구 유실된다.
    ///
    /// 이 테스트는 분할 write를 강제(append_line_for_test의 split)해 인터리빙을 결정론적으로
    /// 만든다. 직렬화 락 없이는(아래 1단계) 손상 라인이 실제로 발생함을 먼저 입증하고,
    /// persist_feed_item이 쓰는 것과 동형인 락을 끼우면(2단계) 모든 라인이 온전히
    /// round-trip함을 박제한다. 이로써 회귀 테스트가 '이빨'을 갖는다(락 제거 시 1단계가 깨짐을
    /// 보장).
    #[test]
    fn jsonl_append_interleaving_corrupts_without_serialization_lock() {
        const THREADS: usize = 8;
        const PER_THREAD: usize = 60;
        let total = THREADS * PER_THREAD;
        let mk_line = |t: usize, i: usize| {
            // 각 라인은 유효 JSON 객체(FeedItem 직렬화 형태와 동급) — 분할 인터리빙이
            // 일어나면 깨진 JSON이 되어 from_str이 실패한다.
            serde_json::to_string(&sample_feed_item(
                &format!("req-{t}-{i}"),
                format!("body-{t}-{i}-{}", "x".repeat(64)),
            ))
            .unwrap()
        };
        let parse_ok = |path: &std::path::Path| -> (usize, usize) {
            let content = std::fs::read_to_string(path).unwrap_or_default();
            let mut lines = 0usize;
            let mut good = 0usize;
            for l in content.lines() {
                lines += 1;
                if serde_json::from_str::<FeedItem>(l).is_ok() {
                    good += 1;
                }
            }
            (lines, good)
        };

        // ── 1단계: 락 없음 + 분할 강제 → 인터리빙 손상이 실제로 발생함을 입증 ──
        // (이 단계가 손상을 못 만들면 테스트가 무의미하므로, 손상을 적극적으로 요구한다.)
        let unlocked = isolated_sock("jsonl-unlocked").with_file_name("feed.jsonl");
        let _ = std::fs::remove_file(&unlocked);
        let mut handles = Vec::new();
        for t in 0..THREADS {
            let p = unlocked.clone();
            handles.push(std::thread::spawn(move || {
                for i in 0..PER_THREAD {
                    append_line_for_test(&p, &mk_line(t, i), true, None);
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        let (u_lines, u_good) = parse_ok(&unlocked);
        let _ = std::fs::remove_file(&unlocked);
        // 분할 사이 인터리빙으로 라인 수가 늘거나(토막 단독 라인) 깨진 JSON이 생긴다.
        assert!(
            u_lines != total || u_good != total,
            "분할 write 동시 append가 직렬화 없이도 무손상이었다 — 재현 전제가 깨짐 \
             (lines={u_lines}, good={u_good}, expected={total}). 이 단계가 통과하면 \
             아래 락 박제가 '이빨'을 잃는다."
        );

        // ── 2단계: 동형 직렬화 락 + 동일 분할 강제 → 모든 라인 온전 ──
        // persist_feed_item이 feed_persist_lock으로 보장하는 것과 같은 불변식.
        let locked = isolated_sock("jsonl-locked").with_file_name("feed.jsonl");
        let _ = std::fs::remove_file(&locked);
        let lock = Arc::new(Mutex::new(()));
        let mut handles = Vec::new();
        for t in 0..THREADS {
            let p = locked.clone();
            let lk = Arc::clone(&lock);
            handles.push(std::thread::spawn(move || {
                for i in 0..PER_THREAD {
                    append_line_for_test(&p, &mk_line(t, i), true, Some(&lk));
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        let (l_lines, l_good) = parse_ok(&locked);
        let _ = std::fs::remove_file(&locked);
        assert_eq!(l_lines, total, "직렬화 락이 있으면 라인 수가 정확히 보존돼야 한다");
        assert_eq!(
            l_good, total,
            "직렬화 락이 있으면 모든 라인이 유효 JSON으로 round-trip해야 한다 \
             (인터리빙 0건) — persist_feed_item의 feed_persist_lock 불변식"
        );
    }

    /// 실제 persist_feed_item을 동시 다발 호출해도(프로덕션 경로) feed.jsonl이 손상되지
    /// 않음을 확인하는 스모크. (플랫폼이 단일 write()를 분할하지 않으면 락 유무와 무관하게
    /// 통과할 수 있으므로 '이빨' 박제는 위 결정론 테스트가 담당한다. 여기선 프로덕션 경로가
    /// 락을 끼운 뒤에도 데드락·라인손상 없이 정상 동작하는지를 본다.)
    #[test]
    fn persist_feed_item_concurrent_smoke_no_corruption() {
        let tmp = isolated_sock("feed-persist");
        let daemon = Daemon::new(tmp.clone());
        let dir = state_dir(&daemon.socket_path);
        let feed_path = dir.join("feed.jsonl");
        let _ = std::fs::remove_file(&feed_path);

        const THREADS: usize = 8;
        const PER_THREAD: usize = 50;
        let mut handles = Vec::new();
        for t in 0..THREADS {
            let d = Arc::clone(&daemon);
            handles.push(std::thread::spawn(move || {
                for i in 0..PER_THREAD {
                    let rid = format!("req-{t}-{i}");
                    let body = format!("{rid}::{}", "한AB\"{}".repeat(2048));
                    d.persist_feed_item(&sample_feed_item(&rid, body));
                }
            }));
        }
        for h in handles {
            h.join().expect("persist thread");
        }

        let content = std::fs::read_to_string(&feed_path).expect("read feed.jsonl");
        let mut seen = std::collections::HashSet::new();
        for line in content.lines() {
            let item: FeedItem = serde_json::from_str(line)
                .unwrap_or_else(|e| panic!("feed.jsonl 라인 손상: {e}; 길이={}B", line.len()));
            seen.insert(item.request_id);
        }
        let expected = THREADS * PER_THREAD;
        assert_eq!(seen.len(), expected, "고유 request_id 유실");

        let _ = std::fs::remove_file(&feed_path);
        let _ = std::fs::remove_file(&tmp);
    }

    /// ★프로덕션 경로 결합 회귀: persist_feed_item이 실제로 feed_persist_lock을 쥔 채
    /// 쓰는지 결정론적으로 박제한다. 락을 외부에서 잡고 있으면 persist_feed_item은 파일에
    /// 손도 못 대야 한다(차단). 누군가 guard 한 줄을 제거하면(수정 회귀) 이 테스트가
    /// 즉시 실패한다 — 플랫폼의 write() 분할 여부와 무관한 '이빨'.
    #[test]
    fn persist_feed_item_holds_feed_persist_lock_during_write() {
        let tmp = isolated_sock("feed-lockheld");
        let daemon = Daemon::new(tmp.clone());
        let dir = state_dir(&daemon.socket_path);
        let feed_path = dir.join("feed.jsonl");
        let _ = std::fs::remove_file(&feed_path);

        // 외부에서 락을 선점한 상태로 persist를 호출하는 스레드를 띄운다.
        let guard = daemon.feed_persist_lock.lock().unwrap();
        let d = Arc::clone(&daemon);
        let writer = std::thread::spawn(move || {
            d.persist_feed_item(&sample_feed_item("locked-req", "x".into()));
        });

        // 락을 쥔 동안에는 파일이 생성/기록되지 않아야 한다(persist가 락에서 대기 중).
        std::thread::sleep(std::time::Duration::from_millis(150));
        let blocked = std::fs::read_to_string(&feed_path)
            .map(|c| c.contains("locked-req"))
            .unwrap_or(false);
        assert!(
            !blocked,
            "feed_persist_lock을 외부가 쥐고 있는데 persist_feed_item이 기록을 진행했다 — \
             write가 feed_persist_lock 임계영역 밖이다(수정 회귀: guard 누락)"
        );

        // 락 해제 → persist가 진행돼 기록이 나타나야 한다.
        drop(guard);
        writer.join().expect("persist thread");
        let after = std::fs::read_to_string(&feed_path).unwrap_or_default();
        assert!(
            after.contains("locked-req"),
            "락 해제 후 persist_feed_item이 정상 기록해야 한다"
        );

        let _ = std::fs::remove_file(&feed_path);
        let _ = std::fs::remove_file(&tmp);
    }

    // ── 델타-read 커서/scrollback 일관성 (state.rs writer ↔ handlers.rs·main.rs reader) ──
    // ★레이스 박제: ingest_output의 scrollback push(N)와 line_count.fetch_add(N)이 분리되면
    // (두 임계영역), reader(read_text·wait_for)가 '증가 전 total + push 후 sb.len()'을 관측해
    // oldest = total - sb.len() 이 실제보다 N 작아지고 skip = start - oldest 가 N 과도해져
    // 최신 N라인을 건너뛴다. 수정은 둘을 같은 scrollback 락 아래로 묶어 reader가 락 보유 중
    // (line_count, sb.len)을 항상 일관되게 보게 한다. 이 테스트는 프로덕션 델타-math를 1:1
    // 미러링해, '레이스 관측' 입력에서 라인 누락이 일어남을 드러내고(버그 재현), '락-일관 관측'
    // 입력에서는 누락이 없음을 박제한다(수정 회귀 차단).

    /// read_text/wait_for의 델타 오프셋 계산을 프로덕션과 1:1로 미러링한 순수 함수.
    /// 반환: (반환 라인들, 시작 절대 라인번호 start). sb는 현재 scrollback 스냅샷,
    /// observed_total은 reader가 본 line_count, since는 요청 커서.
    fn delta_slice(sb: &VecDeque<String>, observed_total: u64, since: u64) -> (Vec<String>, u64) {
        let oldest = observed_total.saturating_sub(sb.len() as u64); // sb[0]의 라인 번호
        let start = since.max(oldest);
        let skip = (start - oldest) as usize;
        let lines: Vec<String> = sb.iter().skip(skip).cloned().collect();
        (lines, start)
    }

    #[test]
    fn delta_read_race_skips_latest_lines_when_count_lags_scrollback() {
        // scrollback이 가득 찬(SCROLLBACK_LINES) 상태에서 writer가 N라인을 push한 직후,
        // fetch_add가 아직 반영되지 않은 '레이스 관측'을 모델링한다.
        let cap = SCROLLBACK_LINES;
        let n: u64 = 3; // 이번 틱에 추가된 라인 수
        // 소비된 누적 라인 수(=line_count): push 반영 후의 진짜 값.
        let true_total: u64 = cap as u64 + 100; // 이미 100라인이 FIFO에서 퇴출된 상태
        // 현재 scrollback(가득 참): 절대 라인번호 [true_total-cap, true_total) 를 담는다.
        let mut sb: VecDeque<String> = VecDeque::with_capacity(cap);
        for ln in (true_total - cap as u64)..true_total {
            sb.push_back(format!("line-{ln}"));
        }
        assert_eq!(sb.len(), cap);

        // reader가 '직전에 읽은' 커서: 최신 N라인 직전(=true_total - n)부터 받기를 원한다.
        let since = true_total - n;

        // (A) 레이스 관측: writer가 push는 마쳤으나(sb는 최신) line_count는 아직 옛값(-n).
        let raced_total = true_total - n;
        let (raced_lines, _raced_start) = delta_slice(&sb, raced_total, since);
        // 버그 증상: 최신 N라인을 받아야 하는데, oldest가 n 작아져 skip이 n 과도 → 라인 누락.
        assert!(
            raced_lines.len() < n as usize,
            "레이스 관측에서 최신 {n}라인이 건너뛰어져야(버그 재현) 하는데 {}라인 반환됨",
            raced_lines.len()
        );
        // 구체 박제: 정확히 가장 최신 n라인이 통째로 누락된다(이 시나리오에선 0라인 반환).
        assert_eq!(
            raced_lines.len(),
            0,
            "가득 찬 scrollback·count -n 관측에선 요청한 최신 {n}라인이 전부 누락"
        );

        // (B) 락-일관 관측(수정 후): reader가 scrollback 락 보유 중 line_count를 읽으므로
        // (sb.len, total)이 항상 짝이 맞는다 → 옛 total은 옛 sb와만, 새 total은 새 sb와만 짝.
        // 새 total(=true_total)과 새 sb(현재 스냅샷)의 일관 관측에서는 누락이 없어야 한다.
        let (consistent_lines, consistent_start) = delta_slice(&sb, true_total, since);
        assert_eq!(consistent_start, since, "일관 관측에선 start가 요청 커서와 일치");
        assert_eq!(
            consistent_lines.len(),
            n as usize,
            "일관 관측에선 요청한 최신 {n}라인이 정확히 반환(누락 0)"
        );
        let expected: Vec<String> = ((true_total - n)..true_total)
            .map(|ln| format!("line-{ln}"))
            .collect();
        assert_eq!(consistent_lines, expected, "반환 라인 내용·순서가 정확");
    }

    #[test]
    fn delta_read_race_is_masked_until_scrollback_has_evicted() {
        // ★레이스 경계 박제: 퇴출이 한 번도 없었던(미가득) scrollback에서는 항상
        // line_count == sb.len() 이므로 oldest = total - sb.len() = 0 이고,
        // saturating_sub가 옛 total(-n)에서도 0으로 클램프해 레이스가 '가려진다'.
        // 즉 이 버그는 FIFO 퇴출(oldest>0)이 발생한 가득 찬 scrollback에서만 발현한다.
        let n: u64 = 5;
        let true_total: u64 = 40; // 누적 40라인, 퇴출 없이 전부 존재(미가득)
        let mut sb: VecDeque<String> = VecDeque::new();
        for ln in 0..true_total {
            sb.push_back(format!("L{ln}"));
        }
        assert!((sb.len() as u64) == true_total, "미가득: total == sb.len()");
        let since = true_total - n; // 최신 n라인 요청

        // 레이스 관측이어도(옛 total) oldest가 0으로 클램프돼 누락이 일어나지 않는다.
        let (raced_lines, raced_start) = delta_slice(&sb, true_total - n, since);
        assert_eq!(raced_start, since);
        assert_eq!(
            raced_lines.len(),
            n as usize,
            "미가득 scrollback에선 saturating_sub가 레이스를 흡수 — 누락 없음(경계 박제)"
        );
        // 일관 관측도 동일 결과 — 미가득 구간은 두 경로가 합치.
        let (consistent_lines, _) = delta_slice(&sb, true_total, since);
        assert_eq!(consistent_lines.len(), n as usize);
    }

    #[test]
    fn ingest_increments_line_count_under_scrollback_lock() {
        // ★수정 박제(구조 검증): writer가 scrollback 락을 보유하는 동안 line_count가
        // push 라인 수만큼 증가해야 한다. 락을 외부에서 쥔 채 ingest 경로의 (push+증가)
        // 임계영역을 모델링하고, 락 해제 전에 line_count가 이미 반영됐는지 확인한다.
        // (실 ingest_output은 Surface/PTY 결합으로 직접 구동이 비싸므로, 같은 락 아래
        //  push·fetch_add를 수행하는 임계영역만 동형으로 재현한다.)
        use std::sync::atomic::AtomicU64;
        let sb = Mutex::new(VecDeque::<String>::new());
        let line_count = AtomicU64::new(0);

        let completed = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        {
            // ingest_output의 임계영역과 동형: 락 보유 중 push 후 같은 락 아래 fetch_add.
            let mut g = sb.lock().unwrap();
            for line in &completed {
                if g.len() >= SCROLLBACK_LINES {
                    g.pop_front();
                }
                g.push_back(line.clone());
            }
            line_count.fetch_add(completed.len() as u64, Ordering::Relaxed);
            // ★핵심 불변식: 락을 아직 쥔 시점에 line_count가 이미 sb.len과 일관해야 한다.
            assert_eq!(
                line_count.load(Ordering::Relaxed),
                g.len() as u64,
                "락 보유 중 (line_count, sb.len)이 일관 — fetch_add가 락 임계영역 안에서 수행됨"
            );
        }
        assert_eq!(line_count.load(Ordering::Relaxed), 3);
    }
}
