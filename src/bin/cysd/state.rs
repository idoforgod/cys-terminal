//! Daemon state: surfaces (PTY sessions), health rules, process ledger.

use crate::events::EventBus;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use regex::Regex;
use serde_json::json;
use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::sync::broadcast;

const SCROLLBACK_LINES: usize = 10_000;
pub const DEFAULT_ROWS: u16 = 35;
pub const DEFAULT_COLS: u16 = 120;

// ★D3(W5): Windows Job Object — PTY 자식 동반사망(KILL_ON_JOB_CLOSE). unix 는 setsid+killpg/SIGKILL 로 이미
//   동반사망이 성립하지만 Windows 는 자식이 데몬 사후 생존해 잔존/중복 노드가 됐다(P2-9). 데몬 소유 Job 에
//   자식을 편입하면 데몬 프로세스 종료 시 OS 가 Job 핸들을 닫아 편입된 전 자식·손자를 강제 종료한다.
#[cfg(windows)]
pub(crate) mod winjob {
    use std::sync::OnceLock;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, SetInformationJobObject,
        JobObjectExtendedLimitInformation, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE,
    };

    // 데몬 소유 Job(프로세스 수명 = 핸들 수명, 명시 close 없음 → 프로세스 종료 시 OS 가 닫아 KILL 발동).
    //   HANDLE(=*mut c_void)은 !Send 이므로 usize 로 보관한다(핸들 값 자체는 프로세스 전역 유효).
    static JOB: OnceLock<usize> = OnceLock::new();

    fn job() -> HANDLE {
        (*JOB.get_or_init(|| unsafe {
            let h = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if !h.is_null() {
                let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                SetInformationJobObject(
                    h,
                    JobObjectExtendedLimitInformation,
                    (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                );
            }
            h as usize
        })) as HANDLE
    }

    /// PTY 자식(pid)을 데몬 소유 Job(KILL_ON_JOB_CLOSE)에 편입 — 데몬 사후 자식·손자 동반사망(mac SIGKILL 대칭).
    /// ★post-spawn 편입: portable-pty(ConPTY)가 pseudoconsole 핸드셰이크를 위해 자식을 즉시 실행해야 하므로
    /// CREATE_SUSPENDED→resume 은 ConPTY 계약과 충돌한다 — 채택하지 않았다. 편입 이후 자식이 만드는 손자는
    /// Job 을 상속(자동 편입)하고, 편입 직전 sub-ms 창의 손자만 이론적 이탈(에이전트 실무상 무해). best-effort
    /// (실패해도 스폰을 죽이지 않는다 — 잔존 위험은 unix 대비로만 존재, 가용성 우선).
    pub fn assign_child(pid: u32) {
        if pid == 0 {
            return;
        }
        unsafe {
            let j = job();
            if j.is_null() {
                return;
            }
            let proc = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid);
            if !proc.is_null() {
                AssignProcessToJobObject(j, proc);
                CloseHandle(proc);
            }
        }
    }
}

/// ★G1(W2-A): 인플라이트 큐 원소 — 텍스트만 담던 큐(String)를 안정 ID·단조 seq·시각으로 승격.
/// 텍스트만 저장하면 기아 측정·순서 검증·강제배달 지목이 전부 불가능하다(governance의
/// 'anchor 미보존' 주석이 자인한 한계). 병렬 메타맵이 아니라 원소 타입 치환인 이유:
/// 컴파일러가 전 접점 누락을 강제 검출한다. serde 파생은 WAL(queue-state.json) 직렬화 겸용.
///
/// id 조립 = `q{daemon.started_at as u64:x}.{seq}` — boot 식별자(started_at)로 재기동 간
/// 충돌을 차단하고 seq로 boot 내 단조를 보장한다(발급 단일 지점 = Daemon::next_queue_entry).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct QueueEntry {
    /// 안정 항목 ID — enqueue→WAL→rehome→이관→배달 원장·이벤트를 관통하는 조준점.
    pub id: String,
    /// boot 내 단조 시퀀스(Daemon.queue_seq 발급). WAL 복원 시 max(seq)+1로 재시드.
    pub seq: u64,
    /// 배달 본문(주입 바이트) — send-key Return 항목은 빈 문자열.
    pub text: String,
    /// enqueue 시각(epoch초). 레거시 WAL 항목은 **복원 시각**으로 합성(0.0 금지 —
    /// 부트 직후 wait≈수십억 초 오측정으로 stale 백로그가 즉시 최전선 배달되는 병리 차단).
    pub enqueued_at: f64,
    /// 발신자(있으면 surface_ref, 아니면 클라이언트 from 문자열) — 관측·폐기 통지용.
    #[serde(default)]
    pub from: Option<String>,
    /// enqueue 경로 태그: "send" | "send-key" | "governance-approval" (+ WAL 복원 합성값).
    #[serde(default)]
    pub origin: String,
}

// ─── ★G1(W2-B): 큐 이벤트 payload 단일 빌더 3종 ───────────────────────────────
// json! 페이로드는 컴파일러 강제 밖이다 — 발행처마다 손으로 쓰면 나중 수정이 한 곳만
// 고쳐 조용한 계약 파손이 난다(G1·G4 수렴 지적). 발행처는 반드시 이 빌더를 공유하고,
// 스키마는 아래 테스트 핀이 고정한다.
//
// ★명명 계약(성찰 BLOCKER): 큐 항목 id 필드는 단수 `queue_entry_id`·복수 `queue_entry_ids`.
// 기존 `entry_ids`는 **W-id 에코**(배달 원문 text 스캔 결과)로 javis_report_gate
// critical-tier disarm의 유일 조인 키(javis_report_gate.py:1658-1669)다 — 한 이벤트
// 패밀리에 두 id 체계가 같은 키명으로 공존하면 disarm 조인이 오염돼 TTL마다 재enqueue
// (= wakeup 홍수, governance 주석 명시 병리)되므로 `entry_ids` 키명 재사용 절대 금지.

/// queue.dropped payload — 폐기 3발행처(state 자력종료 drain·governance close_surface
/// drain·handlers queue.clear)가 공유한다. 기존 키(reason/count/bytes) 의미 불변,
/// `queue_entry_ids`는 additive(발신자가 자기 항목 유실을 결정론 확인하는 조준점).
/// reason 어휘(현행 3종): "process_exited" | "surface_closed" | "cleared".
pub fn queue_dropped_payload(reason: &str, dropped: &[QueueEntry]) -> Value {
    json!({
        "reason": reason,
        "count": dropped.len(),
        "bytes": dropped.iter().map(|e| e.text.len()).sum::<usize>(),
        "queue_entry_ids": dropped.iter().map(|e| e.id.clone()).collect::<Vec<String>>(),
    })
}

/// queue.enqueued payload — enqueue 3경로(handlers send/send-key·governance
/// enqueue_master_wakeup)가 공유한다. 기존 키(bytes/depth/from · send-key는 key) 의미
/// 불변, `queue_entry_id`/`seq`/`enqueued_at`은 additive — 수락 증거에 항목 조준점을
/// 동봉해 발신자가 이후 배달·폐기 통지를 결정론으로 조인한다.
pub fn queue_enqueued_payload(entry: &QueueEntry, depth: usize, from: Value, key: Option<&str>) -> Value {
    let mut p = json!({
        "bytes": entry.text.len(),
        "depth": depth,
        "from": from,
        "queue_entry_id": entry.id,
        "seq": entry.seq,
        "enqueued_at": entry.enqueued_at,
    });
    if let Some(k) = key {
        p["key"] = json!(k);
    }
    p
}

/// queue.delivered payload — 배달 영수증. 기존 키(bytes/remaining/entry_ids/surface_ref)
/// 의미 완전 불변: `entry_ids`는 W-id 에코(원문 text 기준)이며 큐 항목 자신의 id는 별도 키
/// `queue_entry_id`로만 실린다(위 명명 계약). additive: queue_entry_id/seq/enqueued_at/
/// delivered_at/wait_secs/overdue/forced. wait_secs는 음수 클램프 0 — 시계 스큐·NTP 점프의
/// 역행 방어이며 기아(결함 1) 실측 분포·단계형 임계(G1 2단 롤아웃)의 근거 필드다.
/// ★G1(W2-D): overdue=단계형 완화(제한 배달)로 나간 건, forced=운영자 강제(queue.deliver·
/// W2-E)로 나간 건 — 구분은 이벤트 층에서만 하고 배달 원장(delivery.rs) 스키마는 불변이다.
pub fn queue_delivered_payload(
    entry: &QueueEntry,
    remaining: usize,
    wakeup_ids: &[String],
    surface_ref: &str,
    delivered_at: f64,
    overdue: bool,
    forced: bool,
) -> Value {
    json!({
        "bytes": entry.text.len(),
        "remaining": remaining,
        "entry_ids": wakeup_ids,
        "surface_ref": surface_ref,
        "queue_entry_id": entry.id,
        "seq": entry.seq,
        "enqueued_at": entry.enqueued_at,
        "delivered_at": delivered_at,
        "wait_secs": (delivered_at - entry.enqueued_at).max(0.0) as u64,
        "overdue": overdue,
        "forced": forced,
    })
}

/// queue.starved hint 문구 계약(성찰 BLOCKER) — 이 시스템의 이벤트 실소비자는 LLM
/// 에이전트다: hint 가 강제 배달 명령을 직접 지시하면 '경보 → 반사적 강제 드레인' 폭주
/// 회로가 열린다. 문구는 **운영자(사람) 판단 전제**를 명시하고 자동 반응을 금지해야 하며,
/// 이 상수의 문면은 아래 payload 핀 테스트가 고정한다(임의 수정 = 계약 변경).
pub const QUEUE_STARVED_HINT: &str = "큐 머리가 장기 대기 중(게이트에 막힘) — 운영자(사람) \
     판단 하에 cys queue deliver 로 강제 배달 가능. LLM 에이전트는 이 경보에 자동 반응(강제 \
     배달·드레인) 금지";

/// queue.starved payload — 기아 경보(신규 이벤트·G1 W2-D). depth_high(적체 **양** 경보)와
/// 별도 축: depth 1이라도 머리가 오래 막혀 있으면 기아다. waited_secs 는 uptime 클램프
/// (governance::queue_head_wait_secs) 값 — 부트 전 대기는 세지 않는다. 발행 전용
/// 쿨다운(5분)은 governance 발행처가 관리한다.
pub fn queue_starved_payload(
    surface_ref: &str,
    role: Option<String>,
    head: &QueueEntry,
    waited_secs: u64,
    depth: usize,
    blocked_by: &str,
) -> Value {
    json!({
        "surface_ref": surface_ref,
        "role": role,
        "head_entry_id": head.id,
        "waited_secs": waited_secs,
        "depth": depth,
        "blocked_by": blocked_by,
        "hint": QUEUE_STARVED_HINT,
    })
}

/// queue.rehomed payload — WAL 복원(restored_queue) 항목이 같은 role의 살아있는 surface
/// pending_queue로 (enqueued_at, seq) 정렬 병합될 때 발행(신규 이벤트·G1 W2-C).
/// `reordered=true`는 병합이 복원 항목을 대상 큐 기존 항목 **앞자리**에 넣어 기존 항목이
/// 뒤로 밀렸음을 뜻한다 — 재정렬 발생 지점의 무음 금지(결함 3 순서 역전 봉인).
/// `queue_entry_ids`는 병합 삽입 순서(= (enqueued_at, seq) 오름차순) 그대로다.
pub fn queue_rehomed_payload(role: &str, rehomed: &[QueueEntry], reordered: bool) -> Value {
    json!({
        "count": rehomed.len(),
        "queue_entry_ids": rehomed.iter().map(|e| e.id.clone()).collect::<Vec<String>>(),
        "role": role,
        "reordered": reordered,
    })
}

/// queue.migrated payload — 좌석 승계 시 구 좌석 pending_queue가 신 좌석 큐 **뒤에**
/// append 이관될 때 발행(신규 이벤트·G1 W2-C). 병합 정책은 현행 append 유지 —
/// 대상 큐 기존 항목이 앞서는 재정렬 가능 지점을 이벤트로 명시할 뿐이다(무음 승계 금지).
/// `queue_entry_ids`는 구 좌석 큐 순서(= append 순서) 그대로다.
pub fn queue_migrated_payload(
    from_surface: u64,
    to_surface: u64,
    role: &str,
    migrated: &[QueueEntry],
) -> Value {
    json!({
        "from_surface": from_surface,
        "to_surface": to_surface,
        "queue_entry_ids": migrated.iter().map(|e| e.id.clone()).collect::<Vec<String>>(),
        "role": role,
    })
}

/// queue.reordered payload — 운영자 강제 배달(queue.deliver·G1 W2-E)이 비머리 항목을
/// `allow_reorder`로 머리에 끌어올릴 때 발행(신규 이벤트). 재정렬 발생 지점의 무음 금지
/// (결함 3 순서 역전 봉인)와 짝 — 배달 성패와 무관하게 재정렬 사실 자체를 기록한다.
/// cause 어휘(현행 1종): "force_deliver" (supersede 는 이번 릴리스 제외 — 브리프 확정).
/// 단수 큐 항목 id 키는 명명 계약대로 `queue_entry_id`(`entry_id` 키명 금지 — W-id 에코
/// 계열 `entry_ids`와 한 이벤트 패밀리에서 체계 혼동을 만들지 않는다).
pub fn queue_reordered_payload(
    surface_ref: &str,
    entry: &QueueEntry,
    from_index: usize,
    cause: &str,
) -> Value {
    json!({
        "surface_ref": surface_ref,
        "queue_entry_id": entry.id,
        "seq": entry.seq,
        "from_index": from_index,
        "to_index": 0,
        "cause": cause,
    })
}

/// (enqueued_at, seq) 기준 stable merge 삽입 위치 — 대상 큐에서 새 항목 `(at, seq)`보다
/// **뒤(더 신규)인 첫 인덱스**를 반환한다(순수 판정자·G1 W2-C).
/// - 동률은 기존/선삽입 항목 승(= stable — 같은 키의 복원 항목은 파일·seq 순서를 유지).
/// - enqueued_at 동률·역행(시계 스큐·NTP 점프)은 seq가 타이브레이커(boot 내 단조).
/// - NaN 비교 불능은 기존 항목 승(보수적 — 순서의 1차 진실은 deque 위치).
/// 반환값 == q.len()이면 순수 append(재정렬 없음), < q.len()이면 기존 항목이 뒤로 밀린다.
pub(crate) fn queue_merge_insert_pos(q: &VecDeque<QueueEntry>, at: f64, seq: u64) -> usize {
    for (i, e) in q.iter().enumerate() {
        let existing_is_newer = match e.enqueued_at.partial_cmp(&at) {
            Some(std::cmp::Ordering::Greater) => true,
            Some(std::cmp::Ordering::Equal) => e.seq > seq,
            _ => false, // Less 또는 NaN — 기존 항목이 앞선다(보수적)
        };
        if existing_is_newer {
            return i;
        }
    }
    q.len()
}

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
    /// 인플라이트 큐: --queued 전송분 — 대상이 조용해질 때(followup) 순서대로 배달.
    /// ★G1(W2-A): 원소 = QueueEntry(id·seq·enqueued_at 관통) — String에서 승격.
    pub pending_queue: Mutex<std::collections::VecDeque<QueueEntry>>,
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
    /// ★좌석 점유 캐시(SEAT-1): watchdog 틱이 커널 사실(자손 프로세스 유무)로 갱신하는 단일 SOT.
    /// 0=Unknown(미판정·프로브 실패) 1=Occupied(자손 존재=쓰이는 중) 2=Empty(셸 단독=빈 좌석).
    /// **왜 캐시인가**: 판정 재료(전 프로세스 표)는 watchdog이 이미 매 틱 refresh 한다 — RPC 경로가
    /// 각자 재조회하면 같은 비용을 중복 지불한다. 쓰기 = `governance::refresh_seat_cache` 단독(단일
    /// writer), 읽기 = surface.list·status·deliver_queued. 승계 게이트만은 캐시를 믿지 않고 그 시점
    /// 프로브를 새로 뜬다(드문 경로·판정이 role 재바인딩을 좌우하므로 stale 금지).
    pub seat_cache: AtomicU8,
    /// ★G2(W3-A BLOCK 교정) 좌석 에이전트 엄격 관측 캐시: 이 틱의 신선한 자손 관측에서
    /// **기지(旣知) 에이전트 엄격 매칭**(governance::cmdline_matches_agent_exec — R2 확정
    /// strict 매처)이 잡혔는가. 쓰기 = `governance::refresh_seat_cache` 단독(seat_cache 와
    /// 동일한 단일 writer 규약) · 읽기 = check_role_deadman 의 meta 부재 보조축 arming.
    /// **무meta 좌석 한정 유지**(meta 좌석은 agent_seen 상태머신이 담당 — 판정 이원화 금지).
    /// 원시 Occupied(아무 자손)로 armed 하면 vim/less/빌드 좌석의 프롬프트 복귀가 사망 후보가
    /// 된다(결함 8 동형) — armed 경계는 반드시 이 엄격 관측이다.
    pub seat_agent_cache: AtomicBool,
    /// T5 사용량 관측 스냅샷 (usage.rs 수집기가 갱신 — 자기보고 agent_status와 별개 층위)
    pub observed_usage: Mutex<Option<crate::usage::ObservedUsage>>,
    /// T5 세션 트랜스크립트 등록 (`usage.register` — SessionStart hook의 결정론 매핑)
    pub registered_transcript: Mutex<Option<String>>,
    /// (4) resume 핀용 agent transcript session_id — analytics.rs의 회계 session_id와 무관(별개 개념).
    /// usage 수집기가 transcript 발견 시 1회 stash(is_none 가드)·topology에 영속해 정확한 세션 재개.
    pub agent_session_id: Mutex<Option<String>>,
    /// (W1) 이 pane의 claude 자식이 실제로 받는 CLAUDE_CONFIG_DIR — 생성 시 결정론 해소해 고정한다
    /// (데몬 env의 CYS_ACCOUNT_DIR 또는 $HOME/.cys/claude, cys::resolve_claude_config_dir). topology에
    /// 영속되고 restore가 이 값을 launch 문자열에 인라인 오버라이드해, 데몬 env가 바뀌어도 원 계정 dir로
    /// 정확히 재개한다. discover 스캔은 ~/.cys/claude를 못 보므로 config_dir 권위는 오직 이 결정론 기록이다.
    /// restore로 재생성될 땐 topology 원값을 그대로 주입(재해소 금지 — 데몬 env 변동 시 오염 방지).
    pub claude_config_dir: Mutex<Option<String>>,
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
    /// (W4) 이 surface의 reader 스레드가 vt100 파서 패닉을 격리·재초기화한 누적 횟수.
    /// process_chunk_isolated가 패닉을 잡을 때마다 증가 — status(org.status)에 노출한다.
    pub parser_panics: AtomicU64,
    /// (W4) 마지막 파서 패닉 발생 epoch초(없으면 None) — 상습 트리거 포렌식용 health 신호.
    pub last_parser_panic: Mutex<Option<f64>>,
    /// ★(T-0147-7 W2 · B6) **각성 래치** — 이 surface 가 처음 `status.set`(=cys set-status)을 보낸
    /// epoch초. 단일 write path = status.set 핸들러의 `get_or_insert`(1회성 래치 · 이후 불변).
    ///
    /// **왜 필요한가**: 종전에 부트 체인이 '각성'의 근거로 쓸 수 있는 신호는 `agent_alive`(프로세스
    /// 생존)와 `status.age_secs`(신선도)뿐이었다. 전자는 빈 CLI 도 참이라 **조용한 허위 성공**을
    /// 만들었고(재검증 B6 — self-test 가 그 오답을 박제 중이었다), 후자는 시간이 지나면 부패해
    /// "각성했는데 미기동" 오판으로 넘어갔다. 래치는 **부패하지 않는 사실**이다: "이 노드는 최소
    /// 한 번 디렉티브를 읽고 스스로 신고했다."
    ///
    /// **단방향 계약(금지 방향 ⑦ · 비평2 B-1)**: 값 존재 = awake **확정**. 값 부재는 NOT-awake 가
    /// **아니다** — 이 필드 배포 이전에 각성한 노드는 영원히 부재이므로, 소비자는 부재를 기존
    /// 균형 술어(`agent_alive OR fresh set-status`)로 **강등**만 하고 재주입·재스폰을 유도하지
    /// 못한다. 부재를 부정으로 읽으면 A1 라이브락의 역방향(건강한 전 팀 재스폰)이 신설된다.
    ///
    /// **영속(필수)**: topology.json 에 기록되고 restore 가 `surface.create`의 `awakened_at`
    /// 파라미터로 되돌려 넣는다 — 인메모리 단독이면 데몬 재시작마다 건강한 전 팀이 래치를 잃는다.
    pub awakened_at: Mutex<Option<f64>>,
    /// ★(T-0147-7 W2 · B14/CS-3⑤) 디렉티브 주입 검증 상태 — Some(true)=ack 확인 / Some(false)=창
    /// 만료까지 미확인 / None=미검증(아직 판정 안 함). 단일 write path = `surface.set_meta` 의
    /// 동명 파라미터(launch-agent 가 주입 후 ack 창을 닫고 기록).
    ///
    /// **왜 상태인가**: 종전 검증은 "화면에 지침 머리말이 보이나"였고 실패는 stderr 경고 1줄로
    /// 삼켜졌다(관측 채널 부재 — RC3). 신호의 질을 ack 로 올리되 **치명 격상은 금지**다
    /// (금지 방향 ③ — 위경고 모드 회귀). 그래서 실패를 '상태'로 남겨 부트는 계속시키고,
    /// 대시보드·진단이 그 사실을 읽는다.
    pub directive_verified: Mutex<Option<bool>>,
    /// ★(W4 · D5 관측) 이 pane 이 지금 vt100 **alternate screen**(전체화면 TUI 버퍼)에 있는가.
    /// 단일 write path = reader 스레드(parser 락 임계영역 안에서 `screen().alternate_screen()`
    /// 스냅샷 — 파서 패닉 재초기화 시 fresh 파서의 false 로 자연 정합). 소비 = surface.list ·
    /// org.status **양쪽 동일 키**(`alt_screen` — 동형성 핀 handlers.rs) + launch-agent 의
    /// mac claude fullscreen WARN(D5 env 방어층 우회 관측). additive bool — 구 소비자 무영향.
    pub alt_screen: AtomicBool,
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
    /// 승인 tier(§2.4-3 S8): "a"|"b"|"c"|"d". None=무태그=D 취급(fail-closed) — 채널 미러는
    /// tier≤C(a|b|c)만 허용된다. serde default로 구(舊) 영속 라인(tier 미포함)과 하위호환.
    #[serde(default)]
    pub tier: Option<String>,
    /// 발행자 커널 peer pid(§3.2 표면정책). feed.reply의 caller_pid와 같으면 자기승인이라
    /// 거부한다(요청한 자가 스스로 승인 불가). None=발행 pid 미상(예: 구 영속 라인)이면
    /// 자기승인 판정을 적용하지 않는다(정보 없음 → 차단 근거 없음). serde default로 하위호환.
    #[serde(default)]
    pub publisher_pid: Option<u32>,
    /// 발행자 프로세스 그룹 id(M4 pgid 격상). feed.reply의 caller pgid와 같으면 자기승인으로 본다
    /// — push/reply가 별개 CLI 프로세스라도 같은 노드면 그룹이 같아 pid 단독보다 실효적이다.
    /// None=미상(구 영속 라인·windows·해소 실패)이면 이 경로로는 차단하지 않는다. serde default 하위호환.
    #[serde(default)]
    pub publisher_pgid: Option<u32>,
    /// 발행자 소속 surface(resolve_caller_surface·start-time 검증). feed.reply의 caller surface와
    /// 같으면 pgid가 달라도 자기승인이다(setsid/detached로 새 pid·pgid를 만들어도 surface 귀속은
    /// 유지되므로 pgid 탈출을 fail-closed로 막는다·MED-2 감사). None=미상(구 영속 라인·데몬 발행).
    /// 인메모리 Vec이라 마이그레이션 불요. serde default 하위호환.
    #[serde(default)]
    pub publisher_surface: Option<u64>,
    /// W3.1 서버측 위험 파생 태그("auto"|"high"|"human"). cysd가 title·body에서 파생한다
    /// (발행자 tier/kind 자기신고 무관). None=구 영속 라인·파생 전. serde default 하위호환.
    #[serde(default)]
    pub risk_class: Option<String>,
    /// W3.2 이 항목이 CEO 자동결재 경로로 배달됐는가(flag ON + risk=auto). UI가 CC 전환 유예
    /// 연장(90초) 판단에 쓴다. serde default 하위호환(구 라인·비대상=false).
    #[serde(default)]
    pub auto_route: bool,
}

pub struct Config {
    /// PTY에 보장할 로케일 (GUI 기동 데몬은 LANG 미상속 → 한글 입력 깨짐 방지)
    pub lang: String,
    pub load_high_threshold: f64,
    pub proc_count_threshold: usize,
    /// 불투명 명령의 중복 임계 — **한 surface 안**에서 동일 cmdline 이 몇 개면 중복인가(기본 3).
    pub duplicate_threshold: usize,
    /// ★T3-G2 종단점(동일 포트·유닉스 소켓) 중복 임계 — 같은 종단점을 몇이 점유하면 진짜 충돌인가.
    /// 오너 계약 "동일 서버 2개+ 즉시 정리"에 맞춰 기본 2(`CYS_DUP_ENDPOINT_THRESHOLD`). 0=비활성.
    pub duplicate_endpoint_threshold: usize,
    pub auto_kill_duplicates: bool,
    pub idle_seconds: u64,
    /// (E-a) 동시 살아있는 worker-* 한도. 0=무제한(하위호환 escape hatch).
    pub max_active_workers: usize,
    /// W3 CEO 자동결재 라우팅 게이트. 기본 OFF(미설정) — 현행 동작 100% 보존(C-4 부트스트랩
    /// 안전). ON일 때만 risk=auto 항목을 CEO 좌석으로 즉시 배달한다. `CYS_APPROVE_AUTO_ROUTE=1`.
    pub approve_auto_route: bool,
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
            duplicate_endpoint_threshold: env_f64("CYS_DUP_ENDPOINT_THRESHOLD", 2.0) as usize,
            auto_kill_duplicates: cys::env_compat("CYS_AUTOKILL_DUP")
                .map(|v| v == "1")
                .unwrap_or(false),
            idle_seconds: env_f64("CYS_IDLE_SECONDS", 300.0) as u64,
            max_active_workers: env_f64("CYS_MAX_ACTIVE_WORKERS", 8.0) as usize,
            // 미설정=OFF(fail-safe). "1"만 ON — 그 외 값·부재는 현행 동작 보존.
            approve_auto_route: cys::env_compat("CYS_APPROVE_AUTO_ROUTE")
                .map(|v| v == "1")
                .unwrap_or(false),
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
            if !loc.is_empty() {
                return macos_valid_utf8_locale(&loc);
            }
        }
    }
    "en_US.UTF-8".into()
}

/// macOS: `locale -a` 가 보고하는 설치된 로케일 목록(실패 시 빈 Vec → 폴백 경로).
#[cfg(target_os = "macos")]
fn installed_utf8_locales() -> Vec<String> {
    std::process::Command::new("locale")
        .arg("-a")
        .output()
        .ok()
        .map(|out| {
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// AppleLocale → 실제로 설치된 UTF-8 로케일. 설치 목록을 조회해 normalize_locale에 위임한다.
#[cfg(target_os = "macos")]
fn macos_valid_utf8_locale(apple_locale: &str) -> String {
    normalize_locale(apple_locale, &installed_utf8_locales())
}

/// AppleLocale(비표준 스크립트 서브태그·키워드 포함 가능)를 설치된 UTF-8 로케일로 정규화한다.
/// 예: ko_Kore_KR → ko_KR.UTF-8, zh_Hans_CN → zh_CN.UTF-8. 설치 목록을 인자로 받아 순수·테스트 가능.
/// 절대 "C"/"POSIX"/미설치 로케일을 반환하지 않는다 — 실패해도 항상 설치 보장된 en_US.UTF-8.
#[cfg(target_os = "macos")]
fn normalize_locale(apple_locale: &str, installed: &[String]) -> String {
    // '@' 이후 키워드(calendar=gregorian 등) 제거
    let base = apple_locale.split('@').next().unwrap_or("").trim();
    // 소문자화 + '-','_' 제거 → UTF-8==utf8==UTF8 동치 비교
    let norm = |s: &str| s.to_lowercase().replace(['-', '_'], "");
    let is_installed = |cand: &str| installed.iter().any(|i| norm(i) == norm(cand));

    // 1) 직접: ko_KR → ko_KR.UTF-8
    let direct = format!("{base}.UTF-8");
    if is_installed(&direct) {
        return direct;
    }

    // 2) 스크립트/변형 서브태그 제거: 첫 토큰=언어, 마지막=지역, 중간은 버림
    let parts: Vec<&str> = base.split('_').filter(|t| !t.is_empty()).collect();
    if parts.len() >= 3 {
        let cand = format!("{}_{}.UTF-8", parts[0], parts[parts.len() - 1]);
        if is_installed(&cand) {
            return cand;
        }
    }

    // 3) 언어만으로: "{lang}_"로 시작하고 UTF-8인 첫 설치 로케일
    if let Some(lang) = parts.first() {
        let prefix = format!("{lang}_");
        if let Some(hit) = installed
            .iter()
            .find(|i| i.starts_with(&prefix) && norm(i).contains("utf8"))
        {
            return hit.clone();
        }
    }

    // 4) 최종 폴백: macOS에 항상 설치된 en_US.UTF-8 (절대 C/POSIX 아님)
    "en_US.UTF-8".to_string()
}

#[cfg(all(test, target_os = "macos"))]
mod locale_tests {
    use super::normalize_locale;

    // 가짜 설치 목록: 폴백이 "C"/"POSIX"를 잘못 고르지 않음을 증명하려 일부러 포함한다.
    fn installed() -> Vec<String> {
        ["C", "POSIX", "ko_KR.UTF-8", "en_US.UTF-8", "zh_CN.UTF-8", "ja_JP.UTF-8"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    #[test]
    fn direct_match_ko_kr() {
        assert_eq!(normalize_locale("ko_KR", &installed()), "ko_KR.UTF-8");
    }

    #[test]
    fn strips_script_subtag_ko_kore_kr() {
        // 핵심 버그: 비표준 스크립트 서브태그 Kore 제거 → 설치된 ko_KR.UTF-8
        assert_eq!(normalize_locale("ko_Kore_KR", &installed()), "ko_KR.UTF-8");
    }

    #[test]
    fn strips_script_subtag_zh_hans_cn() {
        assert_eq!(normalize_locale("zh_Hans_CN", &installed()), "zh_CN.UTF-8");
    }

    #[test]
    fn strips_keyword_after_at() {
        assert_eq!(
            normalize_locale("ko_KR@calendar=gregorian", &installed()),
            "ko_KR.UTF-8"
        );
    }

    #[test]
    fn language_only_falls_to_region() {
        // ko(언어만) → "ko_"로 시작하는 첫 UTF-8 로케일
        assert_eq!(normalize_locale("ko", &installed()), "ko_KR.UTF-8");
    }

    #[test]
    fn unknown_locale_falls_back_to_en_us() {
        // 완전 미지 → en_US.UTF-8 (절대 C/POSIX 아님)
        assert_eq!(normalize_locale("xx_Yyyy_ZZ", &installed()), "en_US.UTF-8");
    }

    #[test]
    fn empty_installed_still_en_us_never_c() {
        // 설치 목록이 비어도(=locale -a 실패) 절대 C가 아니라 en_US.UTF-8
        assert_eq!(normalize_locale("ko_KR", &[]), "en_US.UTF-8");
    }

    #[test]
    fn script_subtag_region_missing_falls_to_language() {
        // 3-part인데 지역 재구성(zh_HK)이 미설치 → 분기2 미스 → 분기3 언어폴백(zh_ 첫 UTF-8)
        assert_eq!(normalize_locale("zh_Hant_HK", &installed()), "zh_CN.UTF-8");
    }

    #[test]
    fn two_part_unknown_region_falls_to_language() {
        // 2-part(분기2 SKIP)인데 direct(ko_KP.UTF-8) 미설치 → 분기3 언어폴백(ko_ 첫 UTF-8)
        assert_eq!(normalize_locale("ko_KP", &installed()), "ko_KR.UTF-8");
    }
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

/// (W4) PTY 청크를 vt100 파서에 반영하되, 파서 내부 인덱스 패닉을 격리한다.
///
/// vt100 0.15.2는 와이드(CJK·이모지) 문자의 선두 셀이 마지막 열에 놓인 상태에서 그 셀을
/// 지우거나 덮어쓰면 `row.rs:89 clear_wide`가 `cells[col+1]`을 경계 밖 인덱싱해 패닉한다
/// (좁은 pane으로의 resize가 선두 와이드 셀을 마지막 열로 밀어내는 경로 — 한국어 CLI 출력에서
/// 실재, cysd.log 누적 29회). 이 패닉이 reader 스레드를 죽이면 해당 pane의 PTY 배수가 정지해
/// pane 속 CLI가 write 블록으로 동결된다("절대 불사"의 죽음의 경로).
///
/// 패닉 시: 그 청크의 파싱만 포기하고, 오염 가능성 있는 파서를 폐기해 rows/cols만 보존한
/// fresh `vt100::Parser`로 교체한 뒤 `panicked=true`를 반환한다. 호출부(reader 스레드)는
/// 원시 바이트 broadcast·ingest 경로를 계속 태워 PTY 배수를 절대 멈추지 않는다.
///
/// `AssertUnwindSafe` 근거: `parser`(&mut)는 catch_unwind 경계를 넘는 유일한 상태인데, 패닉
/// 발생 시 즉시 fresh Parser로 통째 교체해 불변식이 깨진 상태를 어떤 관찰 경로로도 노출하지
/// 않는다. rows/cols는 process 이전에 포착해 재초기화에 쓰므로(패닉 후 파서 재접근 없음),
/// 이중 패닉 위험도 없다. `set_size`(escape) 등으로 청크 내 크기 변경이 있었다 해도 패닉 시엔
/// 그 청크 전체를 폐기하므로 이전 크기 보존이 정합적이다(다음 resize RPC가 최종 정정).
fn process_chunk_isolated(
    parser: &mut vt100::Parser,
    chunk: &[u8],
    needs_dsr: bool,
) -> (Option<String>, bool) {
    // rows/cols를 process '이전'에 포착 — 패닉 후 파서를 재접근하지 않고 fresh 재초기화에 쓴다.
    let (rows, cols) = parser.screen().size();
    let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        parser.process(chunk);
        needs_dsr.then(|| {
            let (r, c) = parser.screen().cursor_position();
            format!("\x1b[{};{}R", r + 1, c + 1)
        })
    }));
    match res {
        Ok(resp) => (resp, false),
        Err(_) => {
            *parser = vt100::Parser::new(rows, cols, SCROLLBACK_LINES);
            (None, true)
        }
    }
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

#[cfg(test)]
mod panic_isolation_tests {
    use super::{process_chunk_isolated, SCROLLBACK_LINES};

    /// row.rs:89 clear_wide OOB 재현 시퀀스: 와이드(CJK) 문자의 선두 셀을 26열 그리드 끝에 놓고
    /// 25열로 축소하면 선두 와이드 셀이 마지막 열(index 24, len 25)로 밀린다. 그 셀을 덮어쓰면
    /// vt100 0.15.2가 `cells[col+1]`=cells[25]를 경계 밖 인덱싱해 패닉한다(프로덕션 "len 25 index 25").
    /// 좁은 pane으로의 resize + 한국어 CLI 출력이라는 실제 경로를 그대로 박제한다.
    fn drive_row89_panic(parser: &mut vt100::Parser) -> bool {
        process_chunk_isolated(parser, b"\x1b[1;25H", false);
        process_chunk_isolated(parser, "\u{ac00}".as_bytes(), false); // '가'(wide)
        parser.set_size(10, 25); // 축소 → 선두 와이드 셀이 마지막 열로
        let (_, panicked) = process_chunk_isolated(parser, b"\x1b[1;25Ha", false);
        panicked
    }

    #[test]
    fn normal_chunk_does_not_report_panic() {
        let mut p = vt100::Parser::new(10, 26, SCROLLBACK_LINES);
        let (_, panicked) = process_chunk_isolated(&mut p, b"hello world", false);
        assert!(!panicked, "정상 입력은 패닉을 발동하지 않는다");
        assert!(p.screen().contents().contains("hello world"));
    }

    #[test]
    fn row89_sequence_is_contained_not_propagated() {
        // 격리가 없다면 이 시퀀스는 스레드를 죽인다 — catch_unwind가 panicked=true로 흡수해야 한다.
        let mut p = vt100::Parser::new(10, 26, SCROLLBACK_LINES);
        let panicked = drive_row89_panic(&mut p);
        assert!(panicked, "row.rs:89 clear_wide OOB 시퀀스가 격리(패닉 흡수)를 발동해야 한다");
    }

    #[test]
    fn reinit_preserves_rows_cols() {
        let mut p = vt100::Parser::new(10, 26, SCROLLBACK_LINES);
        assert!(drive_row89_panic(&mut p));
        // 패닉 직전 크기(축소 후 10x25)를 fresh 파서가 그대로 보존해야 한다.
        assert_eq!(p.screen().size(), (10, 25), "재초기화가 rows/cols를 보존해야 한다");
    }

    #[test]
    fn parser_survives_and_processes_after_panic() {
        // 격리 후 파서는 계속 동작 — 후속 청크가 정상 반영돼야 한다(reader 배수 지속의 파서측 보증).
        let mut p = vt100::Parser::new(10, 26, SCROLLBACK_LINES);
        assert!(drive_row89_panic(&mut p));
        let (_, panicked) = process_chunk_isolated(&mut p, b"\x1b[2J\x1b[1;1Halive", false);
        assert!(!panicked, "재초기화된 파서는 후속 청크를 패닉 없이 반영해야 한다");
        assert!(
            p.screen().contents().contains("alive"),
            "재초기화 후 새 출력이 화면에 반영돼야 한다"
        );
    }

    #[test]
    fn dsr_response_survives_isolation() {
        // needs_dsr 경로도 격리 헬퍼를 통과 — 정상 시 커서 위치 응답을 반환한다.
        let mut p = vt100::Parser::new(10, 26, SCROLLBACK_LINES);
        let (resp, panicked) = process_chunk_isolated(&mut p, b"\x1b[3;5H", true);
        assert!(!panicked);
        assert_eq!(resp.as_deref(), Some("\x1b[3;5R"));
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
    /// ★T2 자기증폭 차단 관측 카운터 — (rule, 억제사유) → 횟수. 침묵 금지: 억제가 **일어나고
    /// 있다는 사실**은 보이게 두되 트리거 원문은 담지 않는다. 키 공간이 (룰 수 × 사유 4)로
    /// 유계라 무한 성장이 없다(다른 맵과 달리 prune 불필요).
    pub health_suppressed: Mutex<HashMap<(String, &'static str), u64>>,
    /// T4-15 kill-switch: pause 중에는 큐 배달·스케줄 발화가 동결된다 (직접 send는 통과)
    pub paused: AtomicBool,
    pub pause_info: Mutex<Option<(f64, String)>>, // (since, reason)
    /// T3-9 todo 워치: path → (done, total, mtime)
    pub todo_progress: Mutex<HashMap<String, (u64, u64, f64)>>,
    /// C2 선언 판정 캐시(Declared State): path → (mtime, verdict 케밥 문자열, 선언 owner).
    ///
    /// **세 가지 일을 겸한다.** ①`org.status`/`todo.updated`가 실을 구분 플래그의 원천
    /// (`todo_progress` 값 튜플은 건드리지 않는다 — 설계 §5-2가 구조체 변경을 파급 확대로 기각).
    /// ②mtime 기반 재파싱 skip 캐시. ②가 없으면 **배제 판정(retired·foreign-scope) 파일은
    /// `todo_progress`에 등재되지 않으므로 매 워치독 틱마다 다시 읽히고 다시 파싱된다** =
    /// 전 파일 I/O 순증. 그래서 skip 판정의 기준을 진행률 맵이 아니라 이 캐시로 옮겼다.
    /// ③★W14 S16 — **선언 `owner`의 보관처**. `todo.updated` 이벤트에는 이미 실렸는데
    /// `org.status`에는 없어서, HUD 브리지가 스냅샷 경로에서는 여전히 **파일명 정규식**으로
    /// 라벨을 추론했다(D3가 C4에 그대로 생존). 이벤트와 스냅샷이 다른 진실을 말하면 HUD 라벨은
    /// 새로고침 한 번에 뒤집힌다. 센티널 `"?"`(ADR-4 C-3 · 주인 미상)는 `None`으로 저장한다 —
    /// 없는 정보를 있는 것처럼 흘리면 소비자가 `"?"`라는 이름의 노드를 그린다.
    pub todo_verdict: Mutex<HashMap<String, (f64, &'static str, Option<String>)>>,
    /// T1-3 발신자 해석 캐시: caller pid → 항목 — 60초 TTL (항목 정의는 CallerCacheEntry).
    pub caller_cache: Mutex<HashMap<u32, CallerCacheEntry>>,
    /// (E-c) idempotencyKey → (surface_id, epoch초). 클라이언트 재시도가 같은 key면 기존 surface
    /// 재반환(추가 spawn 0). TTL(CREATE_IDEM_TTL_SECS) 만료 엔트리는 조회 시 lazy 제거.
    pub create_idem: Mutex<HashMap<String, (u64, f64)>>,
    /// ★T-0147-4: 생성자 원장 — 새 surface_id → (생성을 요청한 발신 surface_id, epoch초).
    /// `surface.create`가 pane 안에서 호출됐을 때(발신이 surface로 해석될 때)만 기록한다.
    ///
    /// **왜 필요한가**: `surface.close`의 소유 게이트는 "발신 pane은 자기 surface만 닫는다"인데,
    /// `cys launch-agent`는 **자기가 방금 만든** surface의 기동이 실패하면 그것을 되돌려야 한다
    /// (`cys.rs` 롤백 = `surface.close{cause:"reap"}`). pane 안에서 실행되는 모든 경로
    /// (`cys boot`·▶CEO·부트스트랩·master의 노드 재기동)는 발신이 항상 자기 surface로 해석되므로
    /// 롤백이 **구조적으로 close_denied** 였다 → 실패한 surface가 role을 쥔 채 잔존(고아 좌석).
    /// 이 원장이 "생성자 자신의 롤백"만 정확히 열어준다(권한 모델 확장 아님 —
    /// `handlers::rollback_allowed`가 cause=Reap·생성자 일치·TTL 3조건을 모두 요구).
    ///
    /// **영속하지 않는다**: 데몬이 재시작되면 롤백 주체(pane 프로세스)도 함께 죽으므로 원장을
    /// 되살릴 의미가 없고, topology 스키마를 넓히면 조작 표면만 늘어난다. TTL은 create 재시도
    /// 창과 동일한 CREATE_IDEM_TTL_SECS를 재사용하고 만료분은 insert 시 lazy GC 한다.
    pub create_owner: Mutex<HashMap<u64, (u64, f64)>>,
    pub ledger: Mutex<HashMap<u32, LedgerEntry>>,
    /// 역할 레지스트리: role → surface_id (launch-agent가 등록, --to <role> 주소 해석에 사용)
    pub roles: Mutex<HashMap<String, u64>>,
    /// ★불사의 예외(W2a): 의도적으로 닫힌(surface.close 경유) 역할의 묘비 집합.
    /// close_surface가 role 보유 surface를 닫을 때 추가하고, 역할이 명시적으로 재기동
    /// (launch-agent/claim_role로 role 등록)되면 제거한다("살아있는 역할=묘비 아님" 불변식).
    /// topology.json에 영속돼 콜드부트를 넘어 생존하며, auto-restore·phoenix가 이 집합의
    /// 역할을 절대 재스폰하지 않는다(사고사만 부활, 의도삭제는 좀비 차단). 데몬 기동 시
    /// topology.json에서 로드한다(구 topology=필드 부재→빈 집합=기존 동작 하위호환).
    pub tombstones: Mutex<std::collections::HashSet<String>>,
    /// ★BOOTSTRAP_HARDENING WP-3: 부서(dept) 의도-삭제 묘비 — GUI 삭제 클릭 시점에 base 데몬이
    /// 선기록하는 견고 의도 기록(dept_tombstone.set RPC · 단일 writer=이 데몬). 취약한
    /// bash→python teardown 체인(reg_remove)이 무음 실패해도 리바이버(spawn_org_restore·프론트
    /// 복원)가 이 집합을 게이트로 읽어 삭제 부서를 부활시키지 않는다. 부서 재생성(dept_tombstone.set
    /// remove=true)이 유일 해소 경로. topology.json "dept_tombstones" 키로 영속(부재=빈 집합 하위호환).
    pub dept_tombstones: Mutex<std::collections::HashSet<String>>,
    /// ★W2/A-S1: 묘비 변경 단조 카운터(topology.json 의 tombstones_rev). persist_topology 가 묘비 집합이
    /// 직전 영속본과 달라질 때만 +1 한다. phoenix 는 "rev ≥ 마지막으로 본 rev"일 때만 topology 묘비를 desired 에
    /// 그대로 대입(조건부 replace)해, 부분절단·조작으로 묘비만 빈 파일(rev 부재/역행)을 걸러낸다. 기동 시
    /// disk topology 의 tombstones_rev 를 시드해 재시작을 넘어 단조성을 유지한다.
    pub tombstones_rev: std::sync::atomic::AtomicU64,
    /// persist_topology 가 rev 증가 판정에 쓰는 '직전 영속 묘비 집합'(정렬본). 시드=기동 시 disk 묘비.
    pub last_persisted_tombstones: Mutex<Vec<String>>,
    /// 적대검증 벡터-9 방어심화: master role이 현재 보유 surface로 (재)claim된 epoch초.
    /// master surface가 죽는 윈도우에 다른 노드가 claim_role("master")로 합법 승계 → 즉시
    /// approval.sign으로 위험명령을 정당 서명할 수 있다. 이 값으로 갓 승계한 master의 서명을
    /// 쿨다운(SIGN_COOLDOWN_SECS) 동안 동결해 승계-윈도우 남용을 차단한다. master가 부재/해제되면
    /// None. ★단일UID·신뢰노드 모델에선 claim_role 자체가 권한 메커니즘이라 legit/usurper를
    /// 암호학적으로 완전 구분 불가 — 이건 윈도우 축소·탐지(방어심화)이지 암호보증이 아니다.
    pub master_claimed_at: Mutex<Option<f64>>,
    pub feed_items: Mutex<Vec<FeedItem>>,
    pub feed_waiters: Mutex<HashMap<String, tokio::sync::oneshot::Sender<String>>>,
    /// ★GUI 오퍼레이터 승인(오너 2026-07-15): 기동 시 재발급되는 오퍼레이터 토큰 —
    /// state_dir의 `operator.token`(unix 0600) 파일과 동일 값. feed.reply가 이 값과 일치하는
    /// `operator_token` 파라미터를 받으면 §3.2 자기승인 가드를 면제한다(GUI Allow가 pgid 각인·
    /// surface 미귀속 fail-closed에 오탐 차단되던 결함 수리). 첨부 주체는 GUI Tauri 백엔드
    /// 유일 — 공용 cys CLI 무첨부는 워커의 **우발적** 면제만 차단한다(의도적 동일사용자
    /// 프로세스는 토큰 파일을 읽어 raw RPC로 우회 가능 — M11 수준·사고 방지용, 암호학적 방어
    /// 아님). 발급·기록 실패 시 None(면제 경로 비활성=기존 동작) — 부트체인 비치명.
    pub operator_token: Option<String>,
    /// feed.jsonl append 직렬화 락 — write_all이 짧은 write로 쪼개져도 한 줄 전체가
    /// 한 임계영역에서 쓰이게 보장한다. O_APPEND의 원자성은 단일 write() 콜 단위라,
    /// 대용량 body가 분할 write되면 다른 동시 appender의 라인이 끼어들어 JSONL이
    /// 손상되고 복원(replay)에서 pending 항목이 무음 유실될 수 있다.
    pub feed_persist_lock: Mutex<()>,
    /// 큐 WAL(P7): 미배달 `--queued` 메시지의 데몬 재기동 생존분(queue-state.json replay).
    /// 라이브 큐는 surface.pending_queue(휘발)이고, 이건 재시작을 넘긴 스냅샷이다 —
    /// queue.list가 라이브 큐와 함께 노출한다. id 우선(레거시는 mid)으로 이중 replay를 dedup한다.
    ///
    /// ★G1(W2-C) 비타입 경로 수동 감사 4지점: 원소가 serde_json::Value라 QueueEntry 필드
    /// 추가·변경이 **컴파일러 강제 밖**이다 — QueueEntry 스키마를 만질 때 아래 4곳을 손으로
    /// 감사하라(한 곳이라도 빠지면 신 필드가 조용히 결손된다):
    ///   ① 초기화 — load_queue_state(레거시 합성: id/seq/enqueued_at 전 항목 보장)
    ///   ② persist 병합 — persist_queue_state의 restored 잔존분 append(Value 통짜 복제)
    ///   ③ rehome — rehome_restored_queue의 Value→QueueEntry 되살림(필드 관통)
    ///   ④ queue.list restored 노출 — handlers.rs "queue.list"의 restored 행(신규 열 결손 방지)
    pub restored_queue: Mutex<Vec<serde_json::Value>>,
    /// ★G1(W2-A): QueueEntry.seq 발급 카운터 — boot 내 단조. 시드 = WAL(load_queue_state)
    /// 복원 항목들의 max(seq)+1(WAL 부재 시 1). 발급 단일 지점 = next_queue_entry.
    /// EventBus seq와 분리 — 이벤트 발행과 enqueue는 1:1이 아니고, '살아있는 항목 대비 단조'는
    /// WAL max 시드만으로 성립해 별도 영속 파일이 불필요(최소 침습).
    pub queue_seq: AtomicU64,
    /// ★G1(W2-A): persist_queue_state 직렬화 락(feed_persist_lock 관례 동형). watchdog 스레드와
    /// tokio 핸들러가 동시에 호출할 수 있는데 write_json_atomic의 tmp 이름이 고정이라 동시 쓰기가
    /// 파일을 파손할 수 있다 — G1 이후 WAL은 queue_seq 시드·entry id의 근거라 손상 대가가 크다.
    pub queue_persist_lock: Mutex<()>,
    pub config: Config,
    pub socket_path: PathBuf,
    pub started_at: f64,
    /// 세션 트랜스크립트 FTS 영속 채널 (전용 writer 스레드)
    pub recall_tx: Mutex<std::sync::mpsc::Sender<crate::recall::LineRecord>>,
    /// T6 Control Center 소비 트래커 (claude 메시지 누적 — 오늘·최근창·12h 스파크라인).
    pub consumption: Mutex<Consumption>,
    /// T7 E1-3 영속 분석 저장소(analytics.db) — open 실패 시 None(graceful degrade).
    pub analytics: Mutex<Option<rusqlite::Connection>>,
    /// C0 채널 계층 저장소(channels.db) — desired-state·inbox·원장. 무결 필수라 open 실패 시
    /// None(채널 모듈 비활성) — 데몬은 계속 동작한다(순수 추가 계층).
    pub channels: Mutex<Option<rusqlite::Connection>>,
    /// (W4) 전 surface reader 스레드의 vt100 파서 패닉 격리 누적 횟수(데몬 health 신호).
    /// surface별 카운터(Surface::parser_panics)의 데몬 전체 합산 — status(org.status)에 노출한다.
    pub parser_panics_total: AtomicU64,
    /// CC v2 WS-A: 계정 단위 rate limit 집계 상태(뷰·신원 캐시·영속 스로틀) — accounts.rs 전담.
    pub accounts: Mutex<crate::accounts::AccountsState>,
    /// CC v2 WS-C: learn.status assets(기억·스킬·directives fs 스캔) 60s 캐시 — (계산 시각, 값).
    pub learn_assets_cache: Mutex<Option<(f64, serde_json::Value)>>,
    /// CC v2 WS-C: canonical 학습 상태(~/.cys/state/learn) 쓰기 직렬화 — 데몬 단일 writer 불변식.
    pub learn_write: Mutex<()>,
    /// ★T6: auto-restore가 스폰한 phoenix restore 프로세스의 (pid, start_time) 등록부.
    /// authoritative(타이핑 가드 면제) 게이트의 restore-root allowlist — 이 목록에 있는 pid의
    /// **살아있는 자손만, 복원이 도는 동안만** 면제받는다(RestoreRootGuard가 수명 관리). 콜드부트
    /// phoenix 복원이 launch-agent로 부서장을 fresh-fallback 주입할 때 typing_guard에 막혀 부활이
    /// 실패하던 dept-4 결함을 좁게 연다 — surface.create 임의-cmd 자식·HUD bridge는 이 목록에
    /// 오르지 않으므로 면제 대상이 아니다. (pid, start_time)로 pid 재사용을 fail-closed 구분한다.
    pub restore_roots: Mutex<Vec<(u32, u64)>>,
    /// W3.6 형해화 back-pressure: 발행자(surface)별 승인 (요청 수, 거부 수) 누적. 키=발행
    /// surface id(미상 발행자=0). org.status에 노출 + 임계 초과 시 이벤트·경고 플래그. 인메모리
    /// 세션 카운터(재시작 시 리셋 — 저볼륨·근사 신호라 영속 불요).
    pub approval_stats: Mutex<HashMap<u64, (u64, u64)>>,
    /// W3.2 CEO 자동배달 멱등: 의미 키(kind+title+publisher_surface+body sha256) → 마지막
    /// 배달 epoch. 재발행이 매번 새 request_id를 받아도(id 기준 억제 실패) 같은 의미 요청의
    /// CEO 이중 주입을 억제한다. 인메모리(세션 한정 — 저볼륨).
    pub auto_route_seen: Mutex<HashMap<String, f64>>,
}

/// ★T6 RAII: auto-restore가 스폰한 phoenix restore 프로세스를 restore_roots에 등록하고, Drop에서
/// **반드시** 제거한다. 이 수명이 authoritative 면제의 유일한 창 — 정상 종료·early return·panic
/// unwind 모든 경로에서 Drop이 등록 해제를 보장해 복원 종료 후 잔존 자손이 면제받는 것을 막는다.
/// Mutex poison에도 안전하게 제거한다(lock().unwrap_or_else(into_inner)).
pub(crate) struct RestoreRootGuard {
    daemon: Arc<Daemon>,
    pid: u32,
    start_time: u64,
}

impl RestoreRootGuard {
    /// 등록 즉시 push. 호출측은 **Some(start_time)을 얻은 뒤에만** 생성한다(None은 등록 금지).
    pub(crate) fn new(daemon: Arc<Daemon>, pid: u32, start_time: u64) -> Self {
        daemon
            .restore_roots
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((pid, start_time));
        Self {
            daemon,
            pid,
            start_time,
        }
    }
}

impl Drop for RestoreRootGuard {
    fn drop(&mut self) {
        let mut roots = self
            .daemon
            .restore_roots
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        // 자신의 (pid, start_time) 항목 하나만 제거 — 같은 pid 다중 등록에도 정확히 한 개만.
        if let Some(i) = roots
            .iter()
            .position(|&(p, s)| p == self.pid && s == self.start_time)
        {
            roots.remove(i);
        }
    }
}

/// 단일 pid의 현재 start_time(초)만 조회 — pid 재사용 식별(캐시 히트·restore-root 재검증)용
/// 경량 lookup. (T6에서 handlers.rs→state.rs로 이동해 게이트·caller_cache가 단일 구현을 공유한다.)
pub(crate) fn peer_start_time(pid: u32) -> Option<u64> {
    let mut sys = sysinfo::System::new();
    let p = sysinfo::Pid::from_u32(pid);
    sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[p]), true);
    sys.process(p).map(|proc| proc.start_time())
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

/// **데몬 발행 feed 항목의 예약 request_id 접두** — 이 네임스페이스의 단일 정의처다.
///
/// 왜 상수인가(2026-08-17 · 성찰3 설계렌즈 major): 종전에는 `"daemon-"` 리터럴이 생성 1곳 +
/// 판정 5곳 + UI 1곳에 흩어져 있었고 정의는 어디에도 없었다 — '예약 네임스페이스'라고 선언만
/// 하고 네임스페이스의 진리원이 없는 상태였다(같은 저장소가 D5 키에 대해서는
/// `cys::ENV_CLAUDE_NO_ALT_SCREEN` 을 두고 '사본 금지'를 명문화한 것과 어긋난다).
/// 이 접두를 만드는 곳은 `Daemon::push_feed_notification` 하나뿐이고, 읽는 곳은 전부 아래
/// `is_daemon_issued` 를 지난다. 리터럴을 새로 적지 마라 — 늘리려면 여기를 참조하라.
pub const DAEMON_REQ_PREFIX: &str = "daemon-";

/// 이 request_id 가 **데몬이 스스로 발행한** 항목의 것인가(= 외부 caller 가 만들 수 없는 항목).
///
/// 의미: 데몬이 화면 패턴으로 감지해 올린 승인/알림. 이 부류는 ①GUI 에서 Allow/Deny 가 아무
/// 효과를 내지 못하고(응답을 받을 waiter 가 없다) ②surface 의 재발행 코얼레싱 판정에 쓰이며
/// ③governance 의 stalled 스캔 대상이다. 세 소비자가 같은 술어를 봐야 한다.
///
/// 위조 불가의 근거: `handlers.rs` 의 `feed.push` arm 이 클라이언트 지정 request_id 에 이
/// 접두가 있으면 fail-closed 로 거부한다. ∴ 이 술어의 참값은 **서버측 사실**이다.
pub fn is_daemon_issued(request_id: &str) -> bool {
    request_id.starts_with(DAEMON_REQ_PREFIX)
}

pub fn now_epoch() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// §3.2 표면정책 — 자기승인 차단이 켜져 있는가.
/// `~/.cys/policy.json`의 `deny_self_approve`(bool)를 읽는다. 파일이 없거나 파싱 실패하거나
/// 키가 없으면 **기본값 true**(fail-safe — 정책 부재 시 더 안전한 쪽으로 차단). 명시적으로
/// `{"deny_self_approve": false}`로만 끌 수 있다.
///
/// M11 한계 명문화(정직): 이 정책 파일은 **미서명·로컬 파일**이며 데몬과 **동일 신뢰경계** 안의
/// 에이전트(자율 master 포함)가 쓸 수 있다 — 즉 자기승인 차단을 스스로 `false`로 꺼서 우회할 수
/// 있다. 따라서 §3.2의 강제는 "미서명 로컬 정책의 fail-safe 기본값"까지이지, 신뢰경계 내 악의/오작동
/// 에이전트에 대한 암호학적 방어가 아니다. 서명 검증(오너 토큰) 편입은 별도 트랙(크기 큼) — 이번엔
/// 한계만 정직히 고지한다. 정본 반영 = DESIGN-ko.md §3.2.
pub fn deny_self_approve_policy() -> bool {
    let path = cys::home_dir().join(".cys").join("policy.json");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return true; // 파일 없음 → 안전기본 차단 ON
    };
    match serde_json::from_str::<Value>(&text) {
        Ok(v) => v
            .get("deny_self_approve")
            .and_then(|x| x.as_bool())
            .unwrap_or(true), // 키 없음 → 안전기본
        Err(_) => true, // 파싱 실패 → 안전기본(정책 파일 손상이 차단을 끄면 안 됨)
    }
}

/// pid가 속한 프로세스 그룹 id(unix). 자기승인 판정의 pgid 격상(M4)에 쓴다 — `cys feed push`와
/// `reply`가 별개 CLI 프로세스라 pid가 달라도, 같은 노드(워커)에서 나오면 프로세스 그룹이 같다.
/// 존재하지 않는 pid/실패는 None. windows는 프로세스 그룹 개념이 달라 None(pid 단독 폴백).
#[cfg(unix)]
pub fn pgid_of(pid: u32) -> Option<u32> {
    let r = unsafe { libc::getpgid(pid as libc::pid_t) };
    if r < 0 {
        None
    } else {
        Some(r as u32)
    }
}
#[cfg(windows)]
pub fn pgid_of(_pid: u32) -> Option<u32> {
    None
}

/// pid 생존 프로브 — **생존 판정의 단일 정의처**(channels 브리지 자가치유·이중 스폰 게이트와
/// deadman 홀더 회수가 전부 여기로 위임한다). 판정 관용구가 여러 벌 병존하면 결정론 환원
/// 원칙(단일 정의처)이 깨진다 — 프로브 출력과 캐시·원장 기억이 충돌하면 항상 프로브가 이긴다.
/// unix: kill(pid, 0)==0. windows: OpenProcess(PROCESS_SYNCHRONIZE)+WaitForSingleObject(0ms) —
/// 프로세스 핸들이 시그널드(종료 확정)일 때만 dead. 오판 비용이 비대칭이다: 산 프로세스를
/// 죽었다고 보면 재스폰·kill 개입이 나가므로, 확정 못 하면 alive 쪽(개입 금지 방향 fail-closed).
/// 그래서 ERROR_ACCESS_DENIED(존재하나 접근 불가 보호 프로세스)=alive, WAIT_FAILED=alive.
/// channel.status 의 alive·respawn_dead_bridges 의 dead 판정은 전 OS에서 이 실측 하나를
/// 소비한다(payload 형태 불변 — alive 키 의미가 Windows에서도 실측).
#[cfg(unix)]
pub fn pid_alive(pid: u32) -> bool {
    pid != 0 && unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}
#[cfg(windows)]
pub fn pid_alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, ERROR_ACCESS_DENIED, WAIT_OBJECT_0,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE,
    };
    if pid == 0 {
        return false;
    }
    let h = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
    if h.is_null() {
        // 핸들 실패: ACCESS_DENIED 만 '존재 확실'(보호 프로세스) → alive. 그 외
        // (ERROR_INVALID_PARAMETER = pid 부재 등)는 dead.
        return unsafe { GetLastError() } == ERROR_ACCESS_DENIED;
    }
    // 0ms 폴: WAIT_OBJECT_0(시그널드)만 종료 확정 → dead. WAIT_TIMEOUT(실행 중)·WAIT_FAILED
    // (판정 불능)는 alive — 위 doc comment 의 개입 금지 방향.
    let signaled = unsafe { WaitForSingleObject(h, 0) } == WAIT_OBJECT_0;
    unsafe { CloseHandle(h) };
    !signaled
}

/// 자기승인 판정(순수·MED-2 surface 격상) — decision="allow"일 때 아래 중 하나면 자기승인이다:
///  1. pid 동일 OR pgid 동일(M4 기존) — push/reply가 별개 CLI라도 같은 노드면 pgid로 잡는다.
///  2. pub_sid가 Some일 때:
///     - caller가 같은 surface(caller_sid == pub_sid) → pgid가 달라도 자기승인(발행자 surface에서 승인).
///     - caller가 외부 프로세스(caller_pid.is_some())인데 어떤 surface에도 귀속 안 됨(caller_sid.is_none())
///       → **fail-closed 차단**: `setsid cys feed reply`로 새 세션/그룹을 만들어 pid·pgid 매칭을 탈출한
///       경로다. 정당한 승인은 항상 master surface에서 와 귀속되므로 미귀속=탈출로 본다.
/// caller_pid.is_none()(데몬 내부 흐름)은 규칙 2가 caller_pid.is_some()을 명시하므로 걸리지 않는다.
/// master가 워커 feed를 승인하는 정상 흐름은 caller_sid=Some(master)≠pub_sid라 통과한다.
/// 정책 게이트(deny_self_approve_policy)는 호출자가 AND로 결합한다(순수 테스트 가능하게 분리).
pub fn is_self_approval(
    pub_pid: Option<u32>,
    pub_pgid: Option<u32>,
    pub_sid: Option<u64>,
    caller_pid: Option<u32>,
    caller_pgid: Option<u32>,
    caller_sid: Option<u64>,
    decision: &str,
) -> bool {
    if decision != "allow" {
        return false;
    }
    let pid_match = pub_pid.is_some() && pub_pid == caller_pid;
    let pgid_match = pub_pgid.is_some() && pub_pgid == caller_pgid;
    if pid_match || pgid_match {
        return true;
    }
    if pub_sid.is_some() {
        // 같은 surface → 자기승인(pgid 달라도).
        if caller_sid.is_some() && caller_sid == pub_sid {
            return true;
        }
        // 외부 프로세스인데 surface 미귀속 = setsid/detached 탈출 → fail-closed.
        if caller_pid.is_some() && caller_sid.is_none() {
            return true;
        }
    }
    false
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

/// ★GUI 오퍼레이터 승인(오너 2026-07-15): 오퍼레이터 토큰 파일 기록 — unix는 0600(소유자 전용)을
/// 생성·기존 파일 양쪽에 강제한다(mode()는 생성 시에만 적용되므로 set_permissions로 재강제 —
/// 이전 실행이 넓은 권한으로 남긴 파일도 조인다). Windows는 %LOCALAPPDATA%(사용자 프로필 경계)
/// 하위라 별도 ACL 없이 기록 — named pipe owner-only DACL과 동일한 단일-사용자 신뢰경계(M11 수준).
/// 이 토큰은 "데몬 state 디렉토리를 읽을 수 있는 오퍼레이터(사람) 세션" 증명이지 암호학적 방어가
/// 아니다 — 동일 사용자 프로세스는 누구나 읽을 수 있다(정직한 한계 = DESIGN-ko.md §3.2).
fn write_operator_token(path: &std::path::Path, token: &str) -> std::io::Result<()> {
    use std::io::Write;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        f.write_all(token.as_bytes())?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
    }
    #[cfg(not(unix))]
    {
        let mut f = std::fs::File::create(path)?;
        f.write_all(token.as_bytes())
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

/// queue-state.json replay: 엔트리 배열을 **파일 등장순 보존**으로 dedup 복원한다.
///
/// ★G1(W2-A) 재작성: 종전 HashMap.into_values()는 해시-랜덤 순서라 '레거시 seq=파일 등장순
/// 재발급' 합성 규칙과 WAL 라운드트립이 성립 불가였다 — Vec(순서) + HashSet(dedup)으로 교체.
/// dedup 키 = id 우선·부재 시 mid(레거시). 둘 다 없으면 폐기(신원 불능 — fail-safe).
///
/// 레거시(구 WAL: {mid, surface_id, text, role}만) 항목의 신 필드 합성 규칙:
/// - id = mid 재사용 — 레거시 항목도 재기동 간 동일 ID를 갖는다(안정성 유지).
/// - seq = 파일 등장순 재발급(1-기반) — 병합·정렬의 타이브레이커 근거.
/// - enqueued_at = **복원 시각**(0.0 금지 · BLOCKER) — 0.0 합성 시 업그레이드 재기동 직후 전
///   레거시 항목이 wait≈수십억 초로 즉시 overdue 최전선 배달되고 typing 가드가 무방비인
///   부트체인 최취약 창에서 stale 백로그가 폭주한다.
/// from/origin은 합성하지 않는다(없는 정보를 지어내지 않는다 — 소비측 unwrap_or 폴백).
///
/// 파일 부재/파손이면 빈 벡터(fail-safe — 큐 없음이 기본).
///
/// ★비타입 감사 지점 ①(§Daemon::restored_queue) — QueueEntry 스키마 변경 시 여기의
/// 레거시 합성이 전 항목에 신 필드를 보장해야 하류(rehome·queue.list)가 결손 없이 읽는다.
fn load_queue_state(dir: &std::path::Path) -> Vec<serde_json::Value> {
    let mut out: Vec<serde_json::Value> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let restored_at = now_epoch();
    if let Ok(content) = std::fs::read_to_string(dir.join("queue-state.json")) {
        if let Ok(arr) = serde_json::from_str::<Vec<serde_json::Value>>(&content) {
            for (pos, mut it) in arr.into_iter().enumerate() {
                let mid = it.get("mid").and_then(|v| v.as_str()).map(str::to_string);
                let key = it
                    .get("id")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
                    .or_else(|| mid.clone());
                let Some(key) = key else {
                    continue; // id·mid 둘 다 없음 — 신원 불능 항목은 복원하지 않는다
                };
                if !seen.insert(key) {
                    continue; // 이중 replay dedup — 파일 첫 등장 항목 승
                }
                if let Some(obj) = it.as_object_mut() {
                    if !obj.contains_key("id") {
                        if let Some(m) = &mid {
                            obj.insert("id".into(), json!(m)); // 레거시: id=mid 재사용
                        }
                    }
                    if !obj.contains_key("seq") {
                        obj.insert("seq".into(), json!((pos as u64) + 1)); // 파일 등장순 재발급
                    }
                    if !obj.contains_key("enqueued_at") {
                        obj.insert("enqueued_at".into(), json!(restored_at)); // 복원 시각(0.0 금지)
                    }
                }
                out.push(it);
            }
        }
    }
    out
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
        // ★GUI 오퍼레이터 승인(오너 2026-07-15): 오퍼레이터 토큰 발급 — 소켓 listen 전(new 내부)에
        // 기동마다 재발급·덮어쓰기해 파일=메모리 정합을 데몬 재시작(churn)에도 유지한다. GUI(Tauri)가
        // 이 파일을 매 호출 신선 재독해 feed.reply에 첨부. 실패는 비치명(로그만) — 부트체인 차단 금지.
        let operator_token = match crate::channels::random_token_hex() {
            Ok(tok) => match write_operator_token(&dir.join("operator.token"), &tok) {
                Ok(()) => Some(tok),
                Err(e) => {
                    eprintln!("cysd: operator.token 기록 실패(GUI 오퍼레이터 승인 면제 비활성): {e}");
                    None
                }
            },
            Err(e) => {
                eprintln!("cysd: 오퍼레이터 토큰 발급 실패(GUI 오퍼레이터 승인 면제 비활성): {e}");
                None
            }
        };
        // 큐 WAL 복원: queue-state.json을 파일 등장순 보존·id(레거시=mid) dedup으로 replay
        // (미배달 큐 재기동 생존·P7). ★G1(W2-A): queue_seq 시드 계산이 이 복원분을 근거로
        // 하므로 struct init 전에 먼저 로드한다 — 시드 = max(seq)+1(WAL 부재 시 1)로
        // 재기동 후 발급 seq가 살아있는 복원 항목과 절대 겹치지 않는다.
        let restored_qentries = load_queue_state(&dir);
        let queue_seq_seed = restored_qentries
            .iter()
            .filter_map(|it| it.get("seq").and_then(|v| v.as_u64()))
            .max()
            .map(|m| m.saturating_add(1))
            .unwrap_or(1);
        // T7 E1-3: 영속 분석 DB는 socket_path가 struct로 move되기 전에 연다.
        let analytics_conn = crate::analytics::open(&socket_path);
        // C0: 채널 계층 DB(channels.db)도 move 전에 연다. 무결 필수 — open 실패 시 None(모듈 비활성).
        let channels_conn = crate::channels::open(&socket_path);
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
            health_suppressed: Mutex::new(HashMap::new()),
            paused: AtomicBool::new(pause_restored.is_some()),
            pause_info: Mutex::new(pause_restored),
            todo_progress: Mutex::new(HashMap::new()),
            todo_verdict: Mutex::new(HashMap::new()),
            caller_cache: Mutex::new(HashMap::new()),
            create_idem: Mutex::new(HashMap::new()),
            create_owner: Mutex::new(HashMap::new()),
            ledger: Mutex::new(HashMap::new()),
            roles: Mutex::new(HashMap::new()),
            // ★W2a 콜드부트 생존: topology.json에 영속된 묘비를 기동 시 로드(구 topology=빈 집합).
            tombstones: Mutex::new(crate::governance::load_tombstones_from_disk(&socket_path)),
            dept_tombstones: Mutex::new(crate::governance::load_dept_tombstones_from_disk(
                &socket_path,
            )),
            // ★W2/A-S1: rev 를 disk topology 에서 시드(재시작 넘어 단조성 유지)·직전 영속본=시드 묘비.
            tombstones_rev: std::sync::atomic::AtomicU64::new(
                crate::governance::load_tombstones_rev_from_disk(&socket_path),
            ),
            last_persisted_tombstones: Mutex::new({
                let mut v: Vec<String> =
                    crate::governance::load_tombstones_from_disk(&socket_path).into_iter().collect();
                v.sort();
                v
            }),
            // 벡터-9 방어심화: 기동 시 master 미승계 → None (첫 claim_role("master")에서 기록).
            master_claimed_at: Mutex::new(None),
            feed_items: Mutex::new(restored),
            feed_waiters: Mutex::new(HashMap::new()),
            operator_token,
            feed_persist_lock: Mutex::new(()),
            restored_queue: Mutex::new(restored_qentries),
            queue_seq: AtomicU64::new(queue_seq_seed),
            queue_persist_lock: Mutex::new(()),
            config: Config::from_env(),
            recall_tx: Mutex::new(crate::recall::spawn_writer(socket_path.clone())),
            socket_path,
            started_at: now_epoch(),
            consumption: Mutex::new(Consumption::default()),
            analytics: Mutex::new(analytics_conn),
            channels: Mutex::new(channels_conn),
            parser_panics_total: AtomicU64::new(0),
            accounts: Mutex::new(Default::default()),
            learn_assets_cache: Mutex::new(None),
            learn_write: Mutex::new(()),
            restore_roots: Mutex::new(Vec::new()),
            approval_stats: Mutex::new(HashMap::new()),
            auto_route_seen: Mutex::new(HashMap::new()),
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
        // 예약 네임스페이스의 **유일한 생성처**다(접두 정의 = DAEMON_REQ_PREFIX).
        let request_id = format!(
            "{}{}-{}",
            DAEMON_REQ_PREFIX,
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
            tier: None, // 데몬 자동 알림은 무태그(=D·미러 제외) — 채널 스팸 차단.
            publisher_pid: None, // 데몬 발행 — 외부 caller 없음(자기승인 판정 비적용).
            publisher_pgid: None,
            publisher_surface: None,
            // 데몬 자동 알림은 자동결재 대상이 아니다(notification 축약 경로) — 무파생·무라우팅.
            risk_class: None,
            auto_route: false,
        };
        self.feed_items.lock().unwrap().push(item.clone());
        self.persist_feed_item(&item);
        self.bus.publish(
            "feed.item.created",
            "feed",
            surface_id,
            json!({"request_id": request_id, "kind": kind, "title": title,
                   // 데몬 자동 알림은 항상 무태그(=D·미러 제외) — tier 필드 계약 균일성(§2.4-3).
                   "body": body, "wait": false, "tier": "d", "auto_route": false}),
        );
    }

    /// 특정 feed 항목이 아직 pending인가(M8) — channels 모듈이 feed_items 내부를 직접 순회하지 않게
    /// 캡슐화한 헬퍼. verify_interaction(승인 nonce 검증)·register 재조정(승인버튼 복원)이 공유한다.
    pub fn feed_item_pending(&self, request_id: &str) -> bool {
        self.feed_items
            .lock()
            .unwrap()
            .iter()
            .any(|i| i.request_id == request_id && i.status == "pending")
    }

    /// 특정 surface에 데몬 발행(daemon-*) approval 감지 항목이 pending으로 남아 있는가 —
    /// governance 승인 감지의 재발행 억제(코얼레싱) 판정. 같은 프롬프트 에피소드가 살아 있는
    /// 동안 분당 신규 항목이 무한 누적되는 것을 막는다(2026-07-07 feed 189 폭주 재발방지 L3).
    pub fn has_pending_daemon_approval(&self, surface_id: u64) -> bool {
        self.feed_items.lock().unwrap().iter().any(|i| {
            i.status == "pending"
                && i.kind == "approval"
                && i.surface_id == Some(surface_id)
                && is_daemon_issued(&i.request_id)
        })
    }

    /// 특정 surface의 pending 데몬 approval 감지 항목 id 스냅샷 — 화면에서 승인 패턴이
    /// 사라졌을 때 stale 일괄 종결용. 락 해제 후 resolve_feed_item을 개별 호출한다
    /// (데몬 재시작으로 in-memory 추적을 잃은 고아 pending도 이 경로로 청소된다).
    pub fn pending_daemon_approvals(&self, surface_id: u64) -> Vec<String> {
        self.feed_items
            .lock()
            .unwrap()
            .iter()
            .filter(|i| {
                i.status == "pending"
                    && i.kind == "approval"
                    && i.surface_id == Some(surface_id)
                    && is_daemon_issued(&i.request_id)
            })
            .map(|i| i.request_id.clone())
            .collect()
    }

    /// feed 항목을 결정으로 해소한다(pending→resolved) — feed.reply와 채널 승인 미러 interaction이
    /// 공유하는 단일 경로. 성공 시 스냅샷을 영속·대기 pusher wake·feed.item.resolved 발행하고 스냅샷
    /// 반환, pending이 아니거나 없으면 None(멱등 — 중복 해소는 None). ★락 순서: feed_items →
    /// feed_waiters(feed.push와 동일). channels 락을 잡은 채 호출돼도 안전하다(feed_items→channels
    /// 역순 경로 없음 — mirror는 feed_items 해제 후 호출).
    /// 얇은 래퍼(하위호환 — reason·caller 미상 경로: stale-clear·채널 미러). 감사에는
    /// decision만 남고 reason/caller는 null이 된다.
    pub fn resolve_feed_item(&self, request_id: &str, decision: &str) -> Option<FeedItem> {
        self.resolve_feed_item_audited(request_id, decision, None, None)
    }

    /// 단일 해소 경로(M7) + W3.5 감사(producer≠auditor). 모든 결재는 이 코어를 지나며 cysd가
    /// approval_audit.jsonl에 자동 append한다(CEO 자기기록 아님). reason·caller는 feed.reply
    /// 경로에서만 Some.
    pub fn resolve_feed_item_audited(
        &self,
        request_id: &str,
        decision: &str,
        reason: Option<&str>,
        caller_pid: Option<u32>,
    ) -> Option<FeedItem> {
        let snapshot = {
            let mut items = self.feed_items.lock().unwrap();
            let item = items.iter_mut().find(|i| i.request_id == request_id)?;
            if item.status != "pending" {
                return None;
            }
            item.status = "resolved".into();
            item.decision = Some(decision.to_string());
            item.resolved_at = Some(now_epoch());
            item.clone()
        };
        self.persist_feed_item(&snapshot);
        // W3.5 감사 append는 자동결재 기능의 일부 — flag ON일 때만 기록한다(C-4: OFF=현행
        // 100% 동일, audit 파일도 미생성). OFF면 해소만 하고 감사는 건너뛴다.
        if self.config.approve_auto_route {
            self.append_approval_audit(&snapshot, decision, reason, caller_pid);
        }
        if let Some(tx) = self.feed_waiters.lock().unwrap().remove(request_id) {
            let _ = tx.send(decision.to_string());
        }
        self.bus.publish(
            "feed.item.resolved",
            "feed",
            None,
            json!({"request_id": request_id, "decision": decision,
                   // 미러/브리지 tier 필터용(§2.4-3). None(무태그)=D 표기(fail-closed).
                   "tier": snapshot.tier.as_deref().unwrap_or("d")}),
        );
        Some(snapshot)
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

    /// W3.5 승인 감사 append(producer≠auditor): 해소된 항목 스냅샷 + decision·reason·caller를
    /// approval_audit.jsonl에 한 줄 기록한다. CEO가 자기 결재를 기록하는 게 아니라 cysd가 단일
    /// 해소 경로에서 자동 기록한다. ⚠v1 로테이션 부재 명시 수용(승인=사람 페이스 저볼륨) —
    /// size/age 캡은 후속 티켓(§5 리스크 대장). feed_persist_lock 재사용으로 append 직렬화.
    fn append_approval_audit(
        &self,
        item: &FeedItem,
        decision: &str,
        reason: Option<&str>,
        caller_pid: Option<u32>,
    ) {
        let record = json!({
            "ts": now_epoch(),
            "req_id": item.request_id,
            "kind": item.kind,
            "risk": item.risk_class,
            "publisher": item.publisher_surface,
            "caller": caller_pid,
            "decision": decision,
            "reason": reason,
        });
        let Ok(line) = serde_json::to_string(&record) else {
            return;
        };
        let dir = state_dir(&self.socket_path);
        let _guard = self.feed_persist_lock.lock().unwrap();
        // v1 의도적 무제한 append(승인은 사람 페이스 저볼륨) — size/age 캡은 별건 티켓(#7).
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("approval_audit.jsonl"))
        {
            let _ = std::io::Write::write_all(&mut f, format!("{line}\n").as_bytes());
            let _ = f.sync_all();
        }
    }

    /// ★G1(W2-A): QueueEntry 발급 단일 지점 — seq는 boot 내 단조(fetch_add), id는
    /// boot 식별자(started_at)+seq 조합이라 재기동 간에도 충돌하지 않는다.
    pub fn next_queue_entry(&self, text: String, from: Option<String>, origin: &str) -> QueueEntry {
        let seq = self.queue_seq.fetch_add(1, Ordering::SeqCst);
        QueueEntry {
            id: format!("q{:x}.{}", self.started_at as u64, seq),
            seq,
            text,
            enqueued_at: now_epoch(),
            from,
            origin: origin.to_string(),
        }
    }

    /// 큐 WAL 스냅샷을 원자적으로 영속(P7·§9.1-1). enqueue/pop/clear 뒤 호출한다.
    /// 라이브 surface 큐 + 아직 미소비 restored_queue를 합쳐 id(레거시=mid)로 dedup해 쓴다 —
    /// 미배달 `--queued` 메시지가 데몬 재기동을 생존한다(HARNESS 4-a VOLATILE 수리).
    /// ★G1(W2-A) 스키마 확장: {mid(현행 산식 유지), id, seq, surface_id, role, text,
    /// enqueued_at, from, origin}. mid 병기는 구 데몬 롤백 시에도 파일이 읽히게 하는
    /// 하위호환(구 코드는 mid/surface_id/text/role만 읽고 미지 키 무시).
    /// ★락 순서 주의: 호출자는 어떤 pending_queue 락도 쥐지 않은 상태여야 한다(재진입 데드락 방지).
    pub fn persist_queue_state(&self) {
        // ★G1(W2-A): 전용 직렬화 락(feed_persist_lock 관례 동형) — watchdog 스레드·tokio
        // 핸들러 동시 호출 시 고정 tmp명(.queue-state.json.tmp) 공유로 인한 파손 차단.
        // 이 락은 여기서만 잡히므로 pending_queue·surfaces 락과의 역순 획득자가 없다(데드락 무관).
        let _guard = self.queue_persist_lock.lock().unwrap_or_else(|e| e.into_inner());
        let dir = state_dir(&self.socket_path);
        let mut entries: Vec<serde_json::Value> = Vec::new();
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        {
            let surfaces = self.surfaces.lock().unwrap();
            for s in surfaces.values() {
                // ★Phase 5 ①c: 재배달 재타겟 키로 role을 함께 기록한다. surface_id는 재기동 시
                // 소멸하므로(재사용 없음), WAL 생존 메시지를 재기동 후 같은 role의 새 surface로
                // 배달하려면 role 앵커가 필요하다.
                let role = s.role.lock().unwrap().clone();
                let q = s.pending_queue.lock().unwrap();
                for e in q.iter() {
                    if seen.insert(e.id.clone()) {
                        entries.push(json!({
                            "mid": queue_mid(s.id, &e.text), "id": e.id, "seq": e.seq,
                            "surface_id": s.id, "role": role, "text": e.text,
                            "enqueued_at": e.enqueued_at, "from": e.from, "origin": e.origin,
                        }));
                    }
                }
            }
        }
        for it in self.restored_queue.lock().unwrap().iter() {
            // ★비타입 감사 지점 ②(§restored_queue): 잔존 복원분은 Value 통짜 복제라 신 필드가
            // 자동 보존된다 — 필드별 재조립로 바꾸면 결손 위험이 생기니 통짜 복제를 유지하라.
            // dedup 키 = id 우선(load가 전 항목에 합성)·방어적 mid 폴백.
            let key = it
                .get("id")
                .and_then(|v| v.as_str())
                .or_else(|| it.get("mid").and_then(|v| v.as_str()));
            if let Some(k) = key {
                if seen.insert(k.to_string()) {
                    entries.push(it.clone());
                }
            }
        }
        if let Ok(content) = serde_json::to_string(&entries) {
            let _ = crate::governance::write_json_atomic(&dir, "queue-state.json", &content);
        }
    }

    /// ★Phase 5 ①c: 큐 재배달 갭 수리. WAL로 살아난 restored_queue 항목을 **같은 role의 살아있는
    /// surface**의 pending_queue로 옮겨, deliver_queued가 그 surface가 idle일 때 배달하게 한다.
    /// restored_queue는 queue.list에 보이기만 하고 배달 경로(surface.pending_queue)에 없었다(Phase 3 갭).
    /// surface_id는 재기동 시 소멸하므로 role을 앵커로 재타겟한다. role 미기록/무매칭 항목은 보존(정직).
    ///
    /// ★G1(W2-C) 정렬 병합: 종전 무조건 push_back은 재기동 직후 몇 초 사이 enqueue된 신규
    /// 메시지가 재기동 전 구 메시지보다 먼저 배달되는 순서 역전(결함 3의 실경로)을 만들었다 —
    /// 복원 항목을 (enqueued_at, seq) 기준 stable merge로 삽입하고(queue_merge_insert_pos),
    /// 재정렬 발생 여부를 queue.rehomed {count, queue_entry_ids, role, reordered}로 명시 발행한다
    /// (재정렬 지점 무음 금지). 발행은 전 락 해제 후 — publish는 seq 영속 write를 겸한다.
    /// 반환: 재홈된 항목 수(>0이면 호출자가 persist_queue_state로 스냅샷 최신화).
    pub fn rehome_restored_queue(&self) -> usize {
        // (target_sid, role, 병합 삽입 순서의 항목들, reordered) — 락 밖 발행용 수집.
        let mut rehomed_events: Vec<(u64, String, Vec<QueueEntry>, bool)> = Vec::new();
        let mut rehomed = 0usize;
        {
            let mut restored = self.restored_queue.lock().unwrap();
            if restored.is_empty() {
                return 0;
            }
            // role → 살아있는(미exit) surface 매핑
            let mut role_surface: HashMap<String, Arc<Surface>> = HashMap::new();
            for s in self.surfaces.lock().unwrap().values() {
                if s.exited.load(Ordering::Relaxed) {
                    continue;
                }
                if let Some(role) = s.role.lock().unwrap().clone() {
                    role_surface.entry(role).or_insert_with(|| s.clone());
                }
            }
            // 1단: 이관 대상 분리 — role 매칭 항목을 QueueEntry로 되살려 role별 배치로 모은다.
            // ★비타입 감사 지점 ③(§restored_queue): WAL 원값(id/seq/enqueued_at/from/origin)을
            // 보존 승계한다 — id는 load_queue_state가 전 항목에 합성 보장(방어적 mid 폴백).
            // origin 부재(레거시)는 "wal-legacy"로 표기 — 없는 정보를 지어내지 않되
            // 복원 경유 사실은 관측 가능하게 남긴다.
            let mut batches: Vec<(String, Vec<QueueEntry>)> = Vec::new();
            restored.retain(|it| {
                let Some(role) = it.get("role").and_then(|v| v.as_str()) else {
                    return true; // role 미기록 — 보존(정직)
                };
                if !role_surface.contains_key(role) {
                    return true; // role 무매칭 — 보존(재기동 더 기다림)
                }
                let entry = QueueEntry {
                    id: it
                        .get("id")
                        .and_then(|v| v.as_str())
                        .or_else(|| it.get("mid").and_then(|v| v.as_str()))
                        .unwrap_or("")
                        .to_string(),
                    seq: it.get("seq").and_then(|v| v.as_u64()).unwrap_or(0),
                    text: it.get("text").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    enqueued_at: it
                        .get("enqueued_at")
                        .and_then(|v| v.as_f64())
                        .unwrap_or_else(now_epoch),
                    from: it.get("from").and_then(|v| v.as_str()).map(str::to_string),
                    origin: it
                        .get("origin")
                        .and_then(|v| v.as_str())
                        .unwrap_or("wal-legacy")
                        .to_string(),
                };
                match batches.iter_mut().find(|(r, _)| r == role) {
                    Some((_, v)) => v.push(entry),
                    None => batches.push((role.to_string(), vec![entry])),
                }
                rehomed += 1;
                false // restored_queue에서 제거(pending_queue로 이관)
            });
            // 2단: 배치를 (enqueued_at, seq) 오름차순 정렬 후 대상 큐에 stable merge 삽입.
            // 배치 내 정렬은 stable — 동률 키는 WAL 파일 등장순(=push 순서)을 유지한다.
            for (role, mut batch) in batches {
                batch.sort_by(|a, b| {
                    a.enqueued_at
                        .partial_cmp(&b.enqueued_at)
                        .unwrap_or(std::cmp::Ordering::Equal)
                        .then(a.seq.cmp(&b.seq))
                });
                let surf = &role_surface[&role];
                let mut reordered = false;
                {
                    let mut q = surf.pending_queue.lock().unwrap();
                    for entry in &batch {
                        let pos = queue_merge_insert_pos(&q, entry.enqueued_at, entry.seq);
                        if pos < q.len() {
                            reordered = true; // 기존 항목이 복원 항목 뒤로 밀림
                        }
                        q.insert(pos, entry.clone());
                    }
                }
                rehomed_events.push((surf.id, role, batch, reordered));
            }
        }
        // 발행은 restored_queue·pending_queue 락 전부 해제 후(락 밖 I/O 관례 — bus는 leaf지만
        // 256 seq 경계에서 event.seq 파일 write를 겸한다).
        for (sid, role, batch, reordered) in rehomed_events {
            self.bus.publish(
                "queue.rehomed",
                "queue",
                Some(sid),
                queue_rehomed_payload(&role, &batch, reordered),
            );
        }
        rehomed
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
        self.create_surface_with_env(cwd, cmd, title, role, rows, cols, &[], None)
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
        claude_config_dir_override: Option<String>,
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
        // macOS 방어심층: portable-pty는 데몬 env 전체를 자식에 상속한다. GUI/launchd env에
        // LC_ALL/LC_CTYPE(예: C)가 끼어 있으면 우선순위상 LANG을 이겨 한글 입력이 다시 깨진다.
        // 상속된 LC_ALL을 제거하고 LC_CTYPE를 검증된 UTF-8 로케일로 고정해 그 경로를 봉인한다.
        // (Windows 무영향 — cfg로 격리.)
        #[cfg(target_os = "macos")]
        {
            builder.env_remove("LC_ALL");
            builder.env("LC_CTYPE", &self.config.lang);
        }
        // RC-6(T3 발견): Windows 번들 embeddable Python은 open() 기본 인코딩이 cp1252라 UTF-8(한글)
        // 팩 파일 읽기가 UnicodeDecodeError로 크래시. pane에서 도는 python(hooks·javis_*.py)이 UTF-8을
        // 기본으로 쓰게 PYTHONUTF8=1 주입(unix 무영향·이미 UTF-8). cys-dept는 자체 export로 보강.
        builder.env("PYTHONUTF8", "1");
        // 온보딩①: 데몬 옆 동봉 cys CLI + (Windows)동봉 runtime을 pane PATH 선두 주입 —
        // 신규 머신(심링크 없음)에서도 pane 속 AI가 `cys identify`·python3·bash를 즉시 쓴다.
        // RC-5: GUI 직스폰과 공유하는 공용 fn 사용 — 중복 구현 금지.
        // ★T-0147-7 W1a(A17): PATH 단독 주입 → `cys::spawn_env_pairs` 소비로 교체. 종전엔
        //   **HOME backfill 이 schedule.rs 에만 있고 pane 스폰에는 없어서**, HOME 없는 Windows
        //   데몬 env 를 상속한 pane 에서 `${CYS_PACK_DIR:-$HOME/.cys/pack}` 이 `/.cys/pack` 으로
        //   붕괴했다 — 훅(role-bootstrap·session-start)이 팩을 못 찾아 발화가 무산되는 경로다.
        //   unix 는 HOME 이 항상 있어 PATH 쌍만 나오므로 **제로 회귀**(검체 H-WIN-8).
        if let Some(bin_dir) = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        {
            for (k, v) in cys::spawn_env_pairs_from_process(&bin_dir) {
                builder.env(k, v);
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
        // (미설정=기본 계정 fail-safe). CYS_PACK_DIR 전파와 동형.
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
        // ★D3(W5): 스폰 직후 자식을 데몬 소유 Job 에 편입 — 데몬 사후 동반사망(Windows P2-9). unix 는 no-op.
        #[cfg(windows)]
        winjob::assign_child(pid);
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
            // RC-3 잔여(T2.1): env 주입 여부 기록(node-recover·in-seat 재연결의 Windows 안전 판정).
            // ★의미 주의(v0.14 D5 확장 이후 · 적대검증 2R): 이 플래그는 '**무엇이든** env 가
            //   실렸나'이지 '계정격리 키(CLAUDE_CONFIG_DIR)가 실렸나'가 아니다. D5 게이트가
            //   mac 단독에서 넓어진 뒤로는 `cys launch-agent` 가 만드는 surface.create env 맵이
            //   **CLAUDE_CODE_DISABLE_ALTERNATE_SCREEN 한 쌍만으로도 비지 않을 수 있다**
            //   (agent spec 의 env 가 비어 있는 커스텀 구성). 그러면 소비처의 fail-closed 가드
            //   (src/bin/cys.rs — grep `★계정격리 가드(E8)` 와 node-recover 의 `env_injected`
            //   검사)가 격리 키 없이도 열린다.
            // ★그 조합의 정확한 조건(2026-08-17 D5 강등 반영 — 무조건 확장이 아니다):
            //   이 플래그를 **실제로 소비하는 것은 Windows 뿐**이다(실측: in-seat 가드는
            //   `let safe = cfg!(unix) || env_injected;` 라 unix 에선 값과 무관하게 열리고,
            //   node-recover 의 검사는 `#[cfg(windows)]` 로 감싸여 있다).
            //   그리고 Windows 의 D5 는 강등 후 **옵트인**이다(`~/.cys/win-no-alt-screen` ·
            //   `CYS_WIN_NO_ALT_SCREEN=1` — 정본은 lib.rs `d5_gate_for_os` doc). ∴ 플립이
            //   일어나는 조합은 **Windows ∧ 옵트인 ∧ spec env 부재** 3중 조건이다. 기본값
            //   Windows 에서는 D5 가 주입되지 않으므로, spec env 가 비어 있으면 맵도 그대로
            //   비어(=env_injected 거짓) 가드가 닫힌 채다.
            //   (Windows 가 기본 on 으로 승격되면 '∧ 옵트인' 항이 사라져 조건이 넓어진다 —
            //    그 개정 의무는 `d5_gate_for_os` doc 의 승격 절차 **'동반 개정' ④항**에 있다.
            //    거기의 번호는 개정 목록의 번호이지 '앵커 ④' 와 무관하다.)
            // ★그럼에도 이번 라운드에 술어를 좁히지 않은 근거(실측):
            //   ① 동봉 pack 의 claude spec 은 CLAUDE_CONFIG_DIR 을 항상 갖는다(cysjavis-pack/
            //      agents.json) → 기본 구성에서는 D5 이전에도 이미 true 였고 변화가 0 이다.
            //   ② 플립이 일어나는 유일한 조합(spec env 부재)에서는 **애초에 실릴 격리 키가 없다**
            //      — 그 pane 을 순수 cmd 로 재기동하는 것은 새로 launch-agent 하는 것과 동일한
            //      격리 수준이라, 가드가 열려도 잃는 격리가 없다.
            //   ③ `cys new-surface` 로 만든 빈 셸은 env 를 아예 넘기지 않으므로 여전히 false 다
            //      (D5 는 launch-agent 경로에서만 주입된다) → in-seat 가드의 원 목적은 보존된다.
            //   ∴ 지금 고치면 얻는 안전은 0 이고 Windows 부트 경로의 판정만 흔든다. 정본 수리는
            //     '격리 키가 pane 에 실렸는가'를 별도 bool 로 기록하는 것이며, 그때 이 주석과
            //     아래 회귀 핀(create_surface_with_env_records_env_injected_flag)을 함께 고쳐라.
            env_injected: !env.is_empty(),
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
            // ★SEAT-1: 신생 좌석은 Unknown(0)에서 출발한다 — 첫 watchdog 틱이 커널 사실로 확정한다.
            // Unknown의 소비 규약은 "현행 동작 유지"다(§소비처: 큐=배달·승계=거부) — 판정 미도달이
            // 새로운 실패를 만들지 않는다.
            seat_cache: AtomicU8::new(0),
            // 신생 좌석은 미관측(false) — 첫 watchdog 틱의 엄격 관측이 확정한다(fail-closed).
            seat_agent_cache: AtomicBool::new(false),
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
            // (W1) restore가 넘긴 원값이 있으면 그대로 고정(재해소 금지 — 데몬 env 변동 시 오염 방지),
            // 없으면(신규 기동) 이 데몬 프로세스 env로 결정론 해소(pane 셸이 실제 해소할 값과 일치).
            claude_config_dir: Mutex::new(Some(
                claude_config_dir_override
                    .unwrap_or_else(cys::resolve_claude_config_dir),
            )),
            pack_reinject: Mutex::new(None),
            ctx_threshold_armed: AtomicBool::new(true),
            // 능력 가드: 생성 시 역할에서 도출(reviewer-*=read/search, full=worker/master/cso,
            // 그 외 deny-by-default none). claim_role이 역할 전이 시 동기 재도출한다.
            caps: Mutex::new(crate::caps::Caps::for_role(role.as_deref())),
            osc_carry: Mutex::new(Vec::new()),
            parser_panics: AtomicU64::new(0),
            last_parser_panic: Mutex::new(None),
            // ★W2 B6: 래치는 항상 None 으로 시작한다 — 생성 시점엔 아직 어떤 각성 증거도 없다.
            // restore 경로의 하이드레이션은 surface.create 핸들러가 topology 값으로 명시 주입한다
            // (여기서 유추하지 않는다 — 유추는 곧 위양성 래치이고, 그건 재주입 스킵 오판이 된다).
            awakened_at: Mutex::new(None),
            directive_verified: Mutex::new(None),
            // (W4 · D5) 신생 pane 은 primary screen 에서 출발 — 첫 청크 반영 시 reader 가 갱신.
            alt_screen: AtomicBool::new(false),
        });

        // ★W2a: 이 create가 실제 등록한(dedup 후) 역할 — 아래에서 묘비 해제에 쓴다.
        let mut registered_role: Option<String> = None;
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
                roles.insert(final_role.clone(), id);
                registered_role = Some(final_role);
            }
        }
        // ★W2a 해제 불변식: 역할이 명시적으로 (재)기동됐다 = 부활 의도. 묘비에서 제거해
        // 이후 이 역할의 비정상 종료는 다시 정상 부활 대상이 되게 한다("살아있는 역할=묘비 아님").
        // tombstones는 리프 락 — surfaces/roles 락 해제 후 획득(락 순서 무변경).
        if let Some(rr) = registered_role {
            self.tombstones.lock().unwrap().remove(&rr);
            // ★W2/P1-2: master 역할로 (재)기동되면 master_claimed_at 스탬프 — 부활 master 가 approval.sign
            //   동결(master_unstable 거부) 상태로 깨어나 자율주행 게이트가 마비되던 결함 해소. claim_role
            //   경로(handlers.rs)의 승계 스탬프와 동일 의미(새 보유자=쿨다운 시작). tombstones 와 동일 리프 락.
            if rr == "master" {
                *self.master_claimed_at.lock().unwrap() = Some(now_epoch());
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
                        // attach 브로드캐스트 페이로드는 락 '밖'에서 복사한다 — send 자체는 아래
                        // 불변식상 parser 락 안이어야 하지만, chunk 복사(최대 16KB)까지 락 안에서
                        // 하면 대량출력 시 락 보유 시간이 memcpy만큼 늘어 read-screen·status·attach의
                        // .screen() 접근을 불필요하게 블록한다. 복사를 앞당겨 락 임계영역은 send만 남긴다.
                        let attach_payload = chunk.to_vec();
                        // 파서 반영(process)과 attach 브로드캐스트(out_tx.send)를 같은 parser 락
                        // 임계영역에 묶는다 — run_attach가 parser 락 아래에서 구독+스냅샷을 뜨므로,
                        // 이 둘이 분리되면(과거 버그) process 이후·send 이전에 구독한 attach가
                        // 같은 청크를 스냅샷과 live로 중복 수신한다. 락이 process↔send를 직렬화해야
                        // run_attach 주석의 불변식(중복 배달 창 봉쇄)이 실제로 성립한다.
                        // DSR 커서 위치도 같은 락 아래에서 읽어(재진입 락 회피) 일관성을 유지한다.
                        let dsr_resp = {
                            // poison된 락도 복구 — 단일 패닉이 데몬 전체를 마비시키지 않게 한다.
                            let mut parser = surf.parser.lock().unwrap_or_else(|e| e.into_inner());
                            // (W4) vt100 0.15.2(row.rs:89 clear_wide 등) 내부 인덱스 패닉을 격리한다:
                            // 패닉 시 그 청크 파싱만 포기하고 파서를 fresh로 재초기화(rows/cols 보존)한다.
                            // reader 스레드는 죽지 않고, 아래 out_tx.send(원시 바이트 broadcast)와
                            // 후속 ingest 경로는 계속 태워 PTY 배수를 절대 멈추지 않는다.
                            let (resp, panicked) =
                                process_chunk_isolated(&mut parser, chunk, needs_dsr);
                            if panicked {
                                // 재발 관측: surface별·데몬 전체 카운터 + 마지막 발생 시각(status 노출).
                                surf.parser_panics.fetch_add(1, Ordering::Relaxed);
                                *surf.last_parser_panic.lock().unwrap() = Some(now_epoch());
                                daemon.parser_panics_total.fetch_add(1, Ordering::Relaxed);
                                eprintln!(
                                    "[cysd] surface {} vt100 파서 패닉 격리 — 청크 {} 바이트 파싱 포기, \
                                     파서 재초기화(화면 스냅샷 소실). PTY 배수는 계속.",
                                    surf.id,
                                    chunk.len()
                                );
                            }
                            // (W4 · D5) alt_screen 관측 — 파서 락 임계영역 안에서 스냅샷을 떠
                            // 청크 반영과 원자 정합. 패닉 재초기화 경로도 fresh 파서의 false 를
                            // 그대로 반영한다(별도 분기 불요 — 화면 스냅샷 소실과 동일 의미론).
                            surf.alt_screen
                                .store(parser.screen().alternate_screen(), Ordering::Relaxed);
                            // 원시 바이트 broadcast는 파서 반영·패닉 여부와 무관하게 항상 수행한다.
                            // (파서 락 임계영역 내 send — run_attach 구독/스냅샷과의 직렬화 불변식 유지.)
                            let _ = surf.out_tx.send(attach_payload);
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
            // (★G1(W2-B): payload는 폐기 3발행처 공용 빌더 — 스키마 단일 소유).
            let dropped: Vec<QueueEntry> = surf.pending_queue.lock().unwrap().drain(..).collect();
            if !dropped.is_empty() {
                daemon.bus.publish(
                    "queue.dropped",
                    "queue",
                    Some(surf.id),
                    queue_dropped_payload("process_exited", &dropped),
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
        // strip을 carry 슬라이스에서 직접 수행한 뒤 그 구간을 버린다 — 중간 `drained` Vec
        // 할당(청크당 최대 cut바이트)을 제거한다. drain(..cut)은 반환 이터레이터 drop 시
        // 해당 구간을 삭제하므로 collect 없이도 carry가 동일하게 전진한다(산출 불변).
        let stripped = strip_ansi_escapes::strip(&st.carry[..cut]);
        st.carry.drain(..cut);
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
                // ★T2: is_match → find. 매칭 **구간**을 알아야 ⓐ 마스킹·ⓑ 인용/서술 판정이 가능하다.
                // (find는 is_match와 같은 1-pass — `for line × for rule` 핫패스 비용 동등.)
                if let Some(m) = rule.regex.find(line) {
                    // ⓑ 수신 격리 — "경보를 논하는 라인"은 경보가 아니다(자기증폭 차단).
                    // 룰 이름 표식은 `rules`를 직접 훑는다(핫패스 할당 0 — 매칭 시에만 실행).
                    let discourse = alert_discourse_reason(line, m.start(), m.end(), &rules);
                    if let Some(reason) = discourse {
                        // 관측 가능성 유지: 억제 사실만 남기고(원문·트리거 미포함) 발화는 하지 않는다.
                        let mut sup = self.health_suppressed.lock().unwrap();
                        *sup.entry((rule.name.clone(), reason)).or_insert(0) += 1;
                        drop(sup);
                        // ★T3-G2: 억제의 사정거리는 **발신(경보)** 까지다. 여기서 통째로 `continue`
                        // 하면 아래 `recent_health` 인터록 기록까지 함께 사라지는데, 그것은
                        // governance::check_agent_death 의 auth 무한 재기동 차단(auth_blocked)이
                        // 보는 **유일한** 근거다(governance.rs `auth_blocked` 참조). 한국어 문맥이
                        // 붙은 진짜 401 라인이 narration-prose 로 분류되는 순간 차단 장치가 통째로
                        // 죽는다 = 401 상대 무한 재기동. 그래서 클래스를 둘로 가른다:
                        //   · 기계 에코(alert-machinery-token) = **우리 경보의 반사**. 새 정보량 0이고
                        //     인터록 창만 갱신해 자기지속 상태를 만든다 → 완전 폐기(종전대로).
                        //   · 산문 계열(narration-prose·quoted-mention·rule-name-mention) = 진짜일
                        //     **수** 있다(현지화 CLI·구조화 로그·에러코드 문자열이 룰 이름과 동형).
                        //     → 경보는 계속 억제하되 인터록에는 남긴다.
                        // 비대칭의 근거: 놓치면 무한 재기동(시스템 사망), 헛치면 재기동 보류 1건
                        // (master 개입 1회). 안전한 쪽으로 기운다.
                        if is_alert_echo_reason(reason) {
                            continue;
                        }
                    }
                    // ⓐ 발신 봉인 — 이 문자열만 데몬 밖으로 나간다(원문 트리거 유출 0).
                    let safe_line = mask_health_line(line, &rules);
                    let key = (surface_id, rule.name.clone());
                    // status 보드용 최근 alert 링 + ★auth 인터록 원장 (디바운스와 무관하게 기록, cap 50)
                    {
                        let mut recent = self.recent_health.lock().unwrap();
                        if recent.len() >= HEALTH_RING_CAP {
                            // ★T3-G2: 자리가 없으면 **담화(억제) 항목을 먼저** 밀어낸다.
                            // 링이 인터록의 근거 원장이기도 하므로, 경보를 논하는 수다가 진짜 경보
                            // 기록을 창 밖으로 밀어내면 auth 무한 재기동 차단이 근거를 잃는다.
                            // (담화 항목을 남기는 것보다 진짜 항목을 남기는 쪽이 항상 안전하다.)
                            let victim = recent
                                .iter()
                                .position(|e| !e["discourse"].is_null())
                                .unwrap_or(0);
                            recent.remove(victim);
                        }
                        recent.push_back(json!({
                            "ts": now_epoch(), "surface_id": surface_id,
                            "rule": rule.name, "line": safe_line,
                            // 담화로 분류돼 **경보는 억제**된 항목임을 정직하게 표시한다.
                            // status 보드가 "경보"와 "인터록만"을 구분해 보일 수 있게 하는 유일한 필드.
                            "discourse": discourse,
                        }));
                    }
                    if discourse.is_some() {
                        // 발신 억제 유지 — 이벤트·조치 바인딩 없음(자기증폭 경로 원천 차단).
                        continue;
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
                            json!({"rule": rule.name, "line": safe_line, "masked": true}),
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

/// (테스트 가시성) `delivery` 모듈의 race 봉쇄 실증이 이 루프를 직접 구동한다 —
/// "원장 기록이 PTY write 보다 앞선다"는 불변식은 **실제 writer 루프**로만 증명된다.
pub(crate) fn run_writer_loop<W: Write>(
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
/// 동봉 git/python3/uv/node가 /usr/bin CLT-shim을 이기게 한다.
/// ★-lc 확장(2026-07-10): `zsh -lc`(비대화형 로그인)는 .zshrc를 읽지 않아(ZDOTDIR 실측 증명), claude가
/// .zshrc에만 PATH 등록된 소비자 맥에서 명령 pane이 claude를 못 찾는다 → runtime 뒤·"$PATH" 앞에
/// ~/.local/bin을 함께 재선두주입해 대화형 pane(-l·.zshrc 적용)과 우선순위를 일관화한다.
/// cysd 자기 exe_dir(Contents/MacOS) 기준 runtime_bin_dirs와 단일화. runtime 부재(개발)여도 .local/bin은 주입.
#[cfg(target_os = "macos")]
fn mac_runtime_lc_prefix() -> Option<String> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))?;
    let mut dirs = cys::runtime_bin_dirs(&exe_dir);
    dirs.push(cys::home_dir().join(".local").join("bin"));
    mac_lc_path_prefix(&dirs)
}

// ─────────────────────────────────────────────────────────────────────────────
// ★T2 자기증폭 루프 차단 (2026-08-01 윈도우 실사고 · 6분 만에 경보 4→10건, 발생원 0건)
//
// 사고의 구조: `run_health_rules`는 **화면 텍스트(PTY 라인)** 를 정규식으로 매칭한다.
// 그런데 그 매칭 결과(`payload.line` = 원문 그대로)가 다시 **화면에 렌더**된다:
//   ① `cys events --category health --reconnect`(CSO_DIRECTIVE.md:23 상시 구독)는 이벤트
//      JSON 라인을 그대로 `println!`(cys.rs stream_events) → 구독 pane 의 PTY 출력 → 그 pane 에서
//      run_health_rules 재매칭 → 새 health.alert → 모든 구독 pane 으로 다시… (LLM 서술 없이도
//      성립하는 **순수 기계 루프**, 이득 = 구독 pane 수).
//   ② `cys status`/control.dashboard/HUD 가 `recent_health[].line` 을 출력하는 경로.
//   ③ 노드가 경보를 **자연어로 논의**하며 트리거 문구를 화면에 다시 쓰는 경로.
// 발생원이 0건인데 경보만 증식한 이유가 이것이다.
//
// 차단은 두 겹이다(둘 다 필요 — 한쪽만으로는 다른 다리가 남는다):
//   ⓐ **발신 봉인(containment)**: cysd 는 매칭 원문을 다시 내보내지 않는다. 매칭 구간을
//      `‹health-rule:NAME›` 마스크로 치환한 문자열만 이벤트·원장에 싣는다 → ①② 기계 루프가
//      물리적으로 성립 불가(트리거 문자열이 데몬 밖으로 나가지 않는다).
//   ⓑ **수신 격리(quarantine)**: "경보인 라인"과 "경보를 **논하는** 라인"을 가르는 순수 술어로
//      후자를 매칭에서 제외 → ③ 서술 루프 차단.
// (ⓒ 축자인용 금지는 ⓐ에 포함, ⓓ 디바운스는 기존 30초 유지 — 단독으로는 근본이 아니라 완화다.)
// ─────────────────────────────────────────────────────────────────────────────

/// ⓐ 마스크 토큰. 매칭 구간은 전부 이것으로 치환된다.
///
/// ★**룰 이름을 담지 않는다**(이름은 `payload.rule` 필드에 따로 실린다). 담으면
/// `‹health-rule:rate_limited›` 가 `rate.?limit` 에 **스스로 다시 매칭**돼 마스킹이 수렴하지
/// 않는다 — 마스크가 새 트리거를 만드는 자기증폭의 축소판이다.
pub(crate) const HEALTH_MASK: &str = "\u{2039}health-rule\u{203a}";
/// ⓑ 격리 표식으로도 쓰는 마스크 접두(마스크가 찍힌 줄은 cysd 가 만든 경보 표현이다).
pub(crate) const HEALTH_MASK_OPEN: &str = "\u{2039}health-rule";
/// 마스킹 수렴 상한 — 사용자 정의 룰이 마스크 토큰 자체에 매칭하는 병리적 정규식을 등록해도
/// 유한 시간에 끝난다(그 경우 잔여 매칭은 ⓑ 격리가 받는다).
const MASK_PASS_CAP: usize = 8;

/// ⓑ 경보 기계장치 식별자 — **우리가 만든 이름**만 넣는다(자연어 낱말 금지). 제3자 CLI 의
/// 진짜 에러 출력에는 나타날 수 없는 문자열이라 위음성(진짜 고장 은폐) 위험이 없다.
const ALERT_MACHINERY_MARKERS: &[&str] = &[
    HEALTH_MASK_OPEN,
    "health.alert",
    "health.action",
    "health.storm",
    "watchdog.",
    "add-health-rule",
    "health-rules",
    "health.add_rule",
    "health.list_rules",
    "recent_health",
    "run_health_rules",
    "rule=",
    "\"rule\":",
];

/// ⓑ 서술(narration) 판정 임계 — 매칭 구간 **밖**의 한글/CJK 글자 수가 이 값 이상이면 산문으로 본다.
/// 근거: 내장·운영 룰의 패턴은 전부 **영문 토큰**이다. 영문 에러 토큰을 담은 라인에 한글 문장이
/// 붙어 있으면 그것은 제3자 CLI 의 에러 출력이 아니라 **노드가 그 에러를 논한 문장**이다.
/// (한국어로 현지화된 CLI 가 영문 토큰을 함께 뱉는 희귀 사례만 위음성 — 임계를 넉넉히 두고
/// `CYS_HEALTH_NARRATION_CJK_MIN`으로 조정·비활성(0)할 수 있게 한다.)
fn narration_cjk_min() -> usize {
    static CACHE: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *CACHE.get_or_init(|| {
        std::env::var("CYS_HEALTH_NARRATION_CJK_MIN")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(8)
    })
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0xAC00..=0xD7A3      // 한글 음절
        | 0x1100..=0x11FF    // 한글 자모
        | 0x3130..=0x318F    // 호환 자모
        | 0x3040..=0x30FF    // 가나
        | 0x4E00..=0x9FFF    // CJK 통합한자
    )
}

/// ⓑ 매칭 구간이 따옴표로 감싸였는가 — "인용 표기 인식"(경보 문구를 인용한 서술).
///
/// ★T3-G2 정밀화: **JSON 값 자리의 따옴표는 인용이 아니다.** `{"error":"401 Unauthorized"}`
/// 처럼 진짜 도구가 뱉는 구조화 에러 출력은 매칭 구간이 통째로 `"…"` 안에 들어가므로 구
/// 판정에서는 "인용된 서술"로 오분류돼 **경보가 통째로 사라졌다**(진짜 고장 은폐). 여는
/// 따옴표 바로 앞이 JSON 구조 문자(`:` `,` `{` `[` `=`)면 값 자리로 보고 인용에서 제외한다.
/// 산문 인용(`the "rate limit" alarm`)은 여는 따옴표 앞이 공백·문장부호라 판정이 그대로다.
fn match_is_quoted(line: &str, start: usize, end: usize) -> bool {
    let before = line[..start].chars().next_back();
    let after = line[end..].chars().next();
    if before == Some('"') {
        // 여는 따옴표 앞 글자(공백 제외)가 JSON 구조 문자면 값 자리 = 구조화 신호.
        let prev = line[..start - 1].chars().rev().find(|c| !c.is_whitespace());
        if matches!(prev, Some(':') | Some(',') | Some('{') | Some('[') | Some('=')) {
            return false;
        }
    }
    matches!(
        (before, after),
        (Some('"'), Some('"'))
            | (Some('\''), Some('\''))
            | (Some('`'), Some('`'))
            | (Some('\u{201c}'), Some('\u{201d}'))   // “ ”
            | (Some('\u{2018}'), Some('\u{2019}'))   // ‘ ’
            | (Some('\u{300c}'), Some('\u{300d}'))   // 「 」
            | (Some('\u{00ab}'), Some('\u{00bb}'))   // « »
    )
}

/// ⓑ 핵심 순수 술어 — 이 라인이 "경보를 논하는 담화"면 사유를, 진짜 신호면 None 을 반환한다.
///
/// `rules`: 현재 등록된 전 룰(런타임 추가분 포함). 룰 **이름**을 표식으로 쓰되 **식별자 꼴**
/// (`_`·`-`·`.` 포함)만 채택한다 — `relogin` 같은 짧은 일반어 룰 이름을 표식으로 삼으면
/// "please relogin" 같은 진짜 에러가 은폐되기 때문이다(위음성 차단).
pub(crate) fn alert_discourse_reason(
    line: &str,
    start: usize,
    end: usize,
    rules: &[HealthRule],
) -> Option<&'static str> {
    if ALERT_MACHINERY_MARKERS.iter().any(|m| line.contains(m)) {
        return Some("alert-machinery-token");
    }
    if rules
        .iter()
        .any(|r| r.name.contains(['_', '-', '.']) && line.contains(r.name.as_str()))
    {
        return Some("rule-name-mention");
    }
    if match_is_quoted(line, start, end) {
        return Some("quoted-mention");
    }
    let min = narration_cjk_min();
    if min > 0 {
        let outside = line[..start]
            .chars()
            .chain(line[end..].chars())
            .filter(|c| is_cjk(*c))
            .count();
        if outside >= min {
            return Some("narration-prose");
        }
    }
    None
}

/// ★T3-G2: 담화 사유 중 **우리 경보의 기계 에코**인가(= 새 정보량 0이라 인터록에도 남기지
/// 않는 부류인가). `alert-machinery-token` 만 그렇다 — 표식이 전부 우리가 지은 식별자
/// (`health.alert`·`watchdog.`·`"rule":`·마스크 토큰…)라 제3자 CLI 출력에 나타날 수 없다.
///
/// 나머지 셋은 **진짜 신호일 수 있다**:
///   · `narration-prose` — 한국어로 실패를 보고하는 현지화 CLI·복구 스크립트 출력
///   · `quoted-mention` — 값이 따옴표에 담긴 구조화 로그
///   · `rule-name-mention` — 룰 이름이 곧 벤더 에러코드인 경우(`{"error":"token_expired"}`,
///     사용자가 `add-health-rule` 로 에러코드를 그대로 룰 이름에 쓰면 100% 겹친다)
/// 이 셋은 경보만 억제하고 인터록에는 남긴다(fail-safe).
pub(crate) fn is_alert_echo_reason(reason: &str) -> bool {
    reason == "alert-machinery-token"
}

/// governance 의 auth 무한 재기동 차단이 보는 룰 집합 — 단일 등재소.
/// (governance.rs `check_agent_death` 가 문자열 배열을 자기 안에 복제해 갖고 있으면
/// 한쪽만 고쳐질 때 차단이 조용히 새므로 여기 하나로 모은다.)
pub const AUTH_INTERLOCK_RULES: &[&str] =
    &["not_logged_in", "auth_401", "token_expired", "login_required"];

/// auth 인터록 창(초) — 이 시간 안에 auth 계열 신호가 있었으면 자동 재기동을 막는다.
pub const AUTH_INTERLOCK_WINDOW_SECS: f64 = 300.0;

/// `recent_health` 링 상한 — status 보드용이자 auth 인터록의 근거 원장(둘을 겸한다).
pub const HEALTH_RING_CAP: usize = 50;

/// ★T3-G2 — `recent_health` 항목이 **사람에게 보일 경보**인가(= 담화로 억제돼 인터록 전용으로만
/// 남긴 기록이 아닌가).
///
/// 링 하나가 두 소비자를 겸하기 때문에 필요한 술어다:
///   · 안전 인터록(`auth_blocked_by_recent_health`)은 담화 항목도 **센다**(놓치면 무한 재기동).
///   · 사람이 보는 경보 목록·노드 `state=error` 판정은 담화 항목을 **세면 안 된다** —
///     경보를 논한 노드가 화면에서 빨갛게 물들면 그것이 다시 수리 일감이 되어, 우리가 끊으려는
///     자기증폭 루프가 시각 층에서 되살아난다.
pub fn is_alert_record(entry: &serde_json::Value) -> bool {
    entry["discourse"].is_null()
}

/// ★T3-G2 순수 술어 — `recent_health` 원장에서 "이 surface 는 지금 auth 로 막혀 있다"를 읽는다.
/// governance::check_agent_death 가 인라인으로 갖고 있던 판정을 그대로 옮긴 것(동작 동일)으로,
/// 무한 재기동 차단의 **유일한 근거**라 테스트로 직접 핀을 박을 수 있어야 한다.
pub fn auth_blocked_by_recent_health(
    recent: &VecDeque<serde_json::Value>,
    surface_id: u64,
    now: f64,
) -> bool {
    recent.iter().any(|h| {
        h["surface_id"].as_u64() == Some(surface_id)
            && AUTH_INTERLOCK_RULES.contains(&h["rule"].as_str().unwrap_or(""))
            && now - h["ts"].as_f64().unwrap_or(0.0) < AUTH_INTERLOCK_WINDOW_SECS
    })
}

/// ⓐ **전 룰**의 매칭 구간을 `‹health-rule›`로 치환하고 200자로 자른다(문자 경계 안전).
///
/// 불변식: 반환 문자열은 **어떤 헬스룰에도 매칭되지 않는다**. 발화한 룰 하나만 가리면
/// 같은 줄에 있는 다른 룰의 트리거가 원문 그대로 새어 나가고(예: 401 은 가렸는데 같은 줄의
/// `token expired` 는 남는다), 그 문자열이 화면에 다시 찍히는 순간 루프가 부활한다.
/// 반환값만이 데몬 밖으로 나간다 — 원문 트리거는 어떤 이벤트·원장·표면에도 싣지 않는다.
pub(crate) fn mask_health_line(line: &str, rules: &[HealthRule]) -> String {
    let mut out = line.to_string();
    for _ in 0..MASK_PASS_CAP {
        let mut changed = false;
        for r in rules {
            if let Some(m) = r.regex.find(&out) {
                out = format!("{}{}{}", &out[..m.start()], HEALTH_MASK, &out[m.end()..]);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    out.chars().take(200).collect()
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

    // ── pid_alive: 생존 판정 단일 정의처(channels·deadman 위임 대상)의 unix 계약 핀 ──
    // windows arm(OpenProcess+WaitForSingleObject)은 이 호스트에서 컴파일 불가 — 정책 계약은
    // doc comment(ACCESS_DENIED=alive·WAIT_FAILED=alive 개입 금지 방향)이 정본.
    #[test]
    fn pid_alive_self_and_zero() {
        assert!(pid_alive(std::process::id()), "자기 프로세스는 alive");
        assert!(!pid_alive(0), "pid 0 은 프로브 대상이 아님 — 항상 dead");
    }

    #[cfg(unix)]
    #[test]
    fn pid_alive_detects_reaped_child() {
        // kill 후 wait(회수)까지 해야 zombie 가 아니다 — zombie 는 kill(pid,0)==0 이라 alive 로 보인다.
        let mut child = std::process::Command::new("sleep").arg("30").spawn().unwrap();
        let pid = child.id();
        assert!(pid_alive(pid), "스폰 직후 생존");
        let _ = child.kill();
        let _ = child.wait();
        assert!(!pid_alive(pid), "회수된 자식은 dead — false 면 자가치유·재스폰 게이트가 전부 침묵");
    }

    // ★SEAL-1 pane 층 회귀 핀(2026-08-01 실사고): pane 자식(훅·CLI 가 셸 경유로 부르는 python)의
    // 바이트코드 쓰기는 spawn_env_pairs 상속으로만 끈다(직스폰 팩토리 python_command 를 못 타는 경로).
    // ① 단위 핀 — pane 스폰과 같은 모양으로 pairs → CommandBuilder.env 적재 후 get_env 되읽기
    //    (portable-pty 에 env 조회 API 실재 — vendor/portable-pty/src/cmdbuilder.rs get_env).
    // ② 소스 핀 — create_surface_with_env 본문이 spawn_env_pairs_from_process 를 계속 소비한다
    //    (main.rs gui_boot_diagnosis_has_no_prose_matching 소스핀 관례 동형 · 로직 무변경 검증 전용).
    // 실패 시 새는 것: pane 에서 도는 훅/스크립트 python 이 번들 안에 __pycache__/*.pyc 를 써서
    // 코드서명 봉인이 깨진다(다음 실행이 Gatekeeper 에 차단).
    #[test]
    fn pane_children_inherit_no_bytecode_env() {
        let mut b = CommandBuilder::new("true");
        for (k, v) in
            cys::spawn_env_pairs_from_process(std::path::Path::new("/nonexistent-exe-dir-for-pin"))
        {
            b.env(k, v);
        }
        assert_eq!(
            b.get_env(cys::ENV_PY_NO_BYTECODE),
            Some(std::ffi::OsStr::new(cys::PY_NO_BYTECODE_ON)),
            "pane CommandBuilder 에 PYTHONDONTWRITEBYTECODE=1 미적재 — 훅 경유 python 이 번들을 오염시킨다"
        );
        let src = include_str!("state.rs");
        let start = src.find("fn create_surface_with_env").expect("pane 스폰 함수 소실");
        let end = start
            + src[start..]
                .find("\n    fn ingest_output")
                .expect("배선 변형 — 소스핀 앵커 갱신 필요");
        let seg = &src[start..end];
        assert!(
            seg.contains("spawn_env_pairs_from_process"),
            "pane 스폰이 spawn_env_pairs_from_process 소비를 잃었다 — PATH/HOME 과 함께 SEAL-1 상속도 끊긴다"
        );
    }

    // ── M4: 자기승인 pgid 격상 순수 판정 — 같은 pgid(별개 CLI 프로세스)면 차단, 다른 pgid는 허용 ──
    #[test]
    fn is_self_approval_pgid_promotion() {
        // 같은 pid → 차단(allow). (pub_sid·caller_sid None)
        assert!(is_self_approval(Some(100), None, None, Some(100), None, None, "allow"));
        // 다른 pid이지만 같은 pgid(push/reply가 별개 프로세스·같은 노드) → 차단.
        assert!(is_self_approval(Some(100), Some(50), None, Some(200), Some(50), None, "allow"));
        // 다른 pid·다른 pgid(master가 워커 feed 승인)·pub_sid None → 통과.
        assert!(!is_self_approval(Some(100), Some(50), None, Some(200), Some(60), None, "allow"));
        // deny는 항상 통과(자기 요청 취소는 무해).
        assert!(!is_self_approval(Some(100), Some(50), None, Some(100), Some(50), None, "deny"));
        // 발행자 pid·pgid·sid 미상(구 라인) → 차단 근거 없음 → 통과.
        assert!(!is_self_approval(None, None, None, Some(100), Some(50), None, "allow"));
        // pgid만 미상이고 pid 불일치·pub_sid None → 통과(pgid None은 매칭 안 함).
        assert!(!is_self_approval(Some(100), None, None, Some(200), Some(50), None, "allow"));
    }

    // ── MED-2: 자기승인 surface 격상 — 같은 surface·setsid 탈출 fail-closed·master 정상흐름 통과 ──
    #[test]
    fn is_self_approval_surface_promotion() {
        // ① 같은 surface(caller_sid==pub_sid), pgid는 달라도 → 차단.
        assert!(is_self_approval(
            Some(100), Some(50), Some(7), Some(200), Some(60), Some(7), "allow"
        ));
        // ② 다른 surface(master가 워커 feed 승인·caller_sid=master≠pub_sid) → 통과.
        assert!(!is_self_approval(
            Some(100), Some(50), Some(7), Some(200), Some(60), Some(9), "allow"
        ));
        // ③ pub_sid=Some, caller_pid=Some, caller_sid=None(setsid/detached 탈출) → 차단(fail-closed).
        assert!(is_self_approval(
            Some(100), Some(50), Some(7), Some(200), Some(60), None, "allow"
        ));
        // ④ caller_pid=None(데몬 내부 흐름) → 통과(fail-closed 미적용).
        assert!(!is_self_approval(
            Some(100), Some(50), Some(7), None, None, None, "allow"
        ));
        // ⑤ deny는 surface 일치라도 항상 통과.
        assert!(!is_self_approval(
            Some(100), Some(50), Some(7), Some(200), Some(60), Some(7), "deny"
        ));
        // ⑥ 기존 pid/pgid 매칭은 surface 무관하게 유지(pid 동일).
        assert!(is_self_approval(
            Some(100), None, Some(7), Some(100), None, Some(9), "allow"
        ));
    }

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

    // ★-lc 확장 회귀 핀(2026-07-10): -lc 재선두주입에 ~/.local/bin 포함 — zsh -lc가 .zshrc를 안 읽어
    // claude(.zshrc 등록) 미발견이던 소비자 맥 경계 해소. runtime 부재(테스트 바이너리 exe_dir)여도 주입.
    #[cfg(target_os = "macos")]
    #[test]
    fn mac_runtime_lc_prefix_includes_user_local_bin() {
        let p = mac_runtime_lc_prefix().expect("~/.local/bin 추가로 dirs가 비지 않음");
        assert!(p.contains("/.local/bin"), "~/.local/bin 재선두주입: {p}");
        assert!(p.ends_with(":\"$PATH\"; "), "말미 $PATH 확장 보존: {p}");
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
                None,
            )
            .unwrap();
        assert!(s1.env_injected, "env 주입 surface는 env_injected=true여야 node-recover 허용");
        let s2 = daemon
            .create_surface_with_env(
                None, Some("sleep 30".into()), None, Some("worker-2".into()), 24, 80, &[], None,
            )
            .unwrap();
        assert!(!s2.env_injected, "env 미주입 surface는 env_injected=false → Windows node-recover fail-closed");

        // ★D5 한 쌍만 실린 경우 = **의도된 현상**으로 못박는다(2026-08-17 · 성찰3 테스트렌즈 note).
        //  create_surface_with_env 의 주석이 경고하는 조합이다: agent spec 의 env 가 비어 있어도
        //  D5 한 쌍만으로 맵이 비지 않아 env_injected 가 **격리 키 없이 true** 가 된다.
        //  ★그 조합의 조건(강등 반영): 플래그를 소비하는 것은 Windows 뿐이고 Windows 의 D5 는
        //  **옵트인**이므로, 실제 성립 조건은 `Windows ∧ 옵트인 ∧ spec env 부재` 3중이다
        //  (기본값 Windows 는 D5 미주입 → 맵이 비어 가드가 닫힌 채다. 정본은 lib.rs
        //  `d5_gate_for_os` doc). 아래 단언 자체는 OS·옵트인과 **무관한 순수 술어 계약**이라
        //  강등·승격 어느 쪽으로도 흔들리지 않는다 — 흔들리는 것은 이 조건절뿐이다.
        //  지금은 좁히지 않는 것이 옳다고 판단했고(근거 ①②③은 그 주석에 있다), 나중에 술어를
        //  '격리 키가 실렸는가'로 좁히면 이 단언이 **정확히 그 변경 지점을 가리키며** 깨진다 —
        //  그때 주석과 함께 고쳐라.
        let s3 = daemon
            .create_surface_with_env(
                None, Some("sleep 30".into()), None, Some("worker-3".into()), 24, 80,
                &[(cys::ENV_CLAUDE_NO_ALT_SCREEN.to_string(), "1".to_string())],
                None,
            )
            .unwrap();
        assert!(
            s3.env_injected,
            "D5 한 쌍만 실려도 현재 술어(!env.is_empty())는 true 다 — 좁히려면 주석의 정본 수리를 따르라"
        );
    }

    /// ★동봉 pack 의 claude spec 이 계정격리 키를 갖는다 — env_injected 를 좁히지 않기로 한
    /// 판단의 **1번 근거**(create_surface_with_env 의 주석 ①)를 실제 감시선으로 만든다.
    ///
    /// 종전에는 그 근거를 고정하는 테스트가 0건이었다(성찰3 테스트렌즈 note): 이 데이터 파일에서
    /// CLAUDE_CONFIG_DIR 이 빠지면 "기본 구성에서는 D5 이전에도 이미 true 였고 변화가 0" 이라는
    /// 문장이 조용히 거짓이 되고, 그 순간 `env_injected` 는 격리 키 없이 열리는 플래그가 된다.
    /// 값 자체가 아니라 **키의 존재**만 본다(경로 표현은 사용자 환경에 따라 바뀔 수 있다).
    #[test]
    fn packaged_claude_spec_carries_account_isolation_key() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("cysjavis-pack/agents.json");
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("동봉 pack 의 agents.json 을 읽을 수 없다({}): {e}", path.display()));
        let v: serde_json::Value = serde_json::from_str(&raw).expect("agents.json 파싱");
        let env = &v["claude"]["env"];
        assert!(
            env.is_object(),
            "claude spec 에 env 맵이 있어야 한다(없으면 env_injected 근거 ①이 무너진다): {env}"
        );
        assert!(
            env.get("CLAUDE_CONFIG_DIR").is_some(),
            "claude spec env 에 CLAUDE_CONFIG_DIR 이 있어야 한다 — 이것이 사라지면 \
             surface.create 의 env 맵이 D5 한 쌍만 남아 env_injected 가 격리 키 없이 참이 된다: {env}"
        );
    }

    /// ★W2/P1-2: master 역할로 surface 를 (재)기동하면 master_claimed_at 이 스탬프돼 approval.sign 이 즉시
    /// 가능해야 한다(부활 master 동결 해제). 비-master 역할은 master_claimed_at 을 건드리지 않는다.
    #[test]
    fn create_surface_master_stamps_claimed_at() {
        let daemon = Daemon::new(isolated_sock("p1-2-master"));
        assert!(daemon.master_claimed_at.lock().unwrap().is_none(), "기동 직후 None");
        // 비-master → 스탬프 없음
        daemon
            .create_surface_with_env(None, Some("sleep 30".into()), None, Some("worker".into()), 24, 80, &[], None)
            .unwrap();
        assert!(daemon.master_claimed_at.lock().unwrap().is_none(), "worker 생성은 master_claimed_at 무영향");
        // master 부활 → 스탬프(approval.sign 동결 해제)
        daemon
            .create_surface_with_env(None, Some("sleep 30".into()), None, Some("master".into()), 24, 80, &[], None)
            .unwrap();
        assert!(daemon.master_claimed_at.lock().unwrap().is_some(),
                "master 부활 시 master_claimed_at 스탬프돼야 approval.sign 가능(P1-2)");
    }

    /// (W1-6 a·d) 계정 config_dir 영속 라운드트립 + 구 topology 하위호환.
    #[test]
    fn w1_topology_persists_config_dir_and_old_compat() {
        let sock = isolated_sock("w1-topo");
        let daemon = Daemon::new(sock.clone());
        let recorded = "/home/x/acct/.cys/claude";
        // restore 경로 모사: override를 넘기면 재해소 없이 그 원값을 그대로 고정한다.
        let s = daemon
            .create_surface_with_env(
                Some("/home/x/wf".into()),
                Some("sleep 30".into()),
                None,
                Some("worker-1".into()),
                24,
                80,
                &[],
                Some(recorded.to_string()),
            )
            .unwrap();
        assert_eq!(
            s.claude_config_dir.lock().unwrap().clone(),
            Some(recorded.to_string()),
            "restore override는 데몬 env 재해소 없이 원값 고정"
        );
        // 영속 → 재로드 라운드트립: 기록된 config_dir이 topology에 살아 있어야 restore가 인라인할 수 있다.
        crate::governance::persist_topology(&daemon);
        let entries = crate::governance::load_topology(&daemon);
        let found = entries
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["role"].as_str() == Some("worker-1"))
            .expect("worker-1 entry 영속");
        assert_eq!(
            found["claude_config_dir"].as_str(),
            Some(recorded),
            "config_dir 영속·재로드"
        );

        // (d) 구 topology 호환: claude_config_dir 필드 없는 topology.json 직접 기록 → 로드 시 엔트리는
        //     살아있고 config_dir=None(부재) → restore가 override None으로 템플릿 기본에 하위호환.
        let dir = state_dir(&sock);
        let old = r#"{"updated_at":1.0,"entries":[{"role":"worker-9","agent":"claude","cwd":"/x"}]}"#;
        std::fs::write(dir.join("topology.json"), old).unwrap();
        let loaded = crate::governance::load_topology(&daemon);
        let e9 = loaded
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["role"].as_str() == Some("worker-9"))
            .expect("구 topology 엔트리 로드");
        assert!(
            e9.get("claude_config_dir")
                .and_then(|v| v.as_str())
                .is_none(),
            "구 topology엔 필드 부재 → None(restore 템플릿 기본 하위호환)"
        );
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
        let stripped = strip_ansi_escapes::strip(&st.carry[..cut]);
        st.carry.drain(..cut);
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

    // D5 개선 전(pre-refactor) ingest 라인분할을 그대로 재현한 참조 구현 —
    // `drained` 중간 Vec를 collect한 뒤 strip한다. 개선 후 `ingest_step`(carry 슬라이스
    // 직접 strip + drain)과 산출이 바이트 단위로 동일함을 증명하는 데만 쓴다.
    fn ingest_step_pre_refactor(st: &mut IngestState, chunk: &[u8], out: &mut Vec<String>) {
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

    // D5 hard gate: strip 슬라이스 직접화 + drained 할당 제거가 산출을 1비트도 바꾸지
    // 않는다. ANSI 색·커서이동·CRLF·단독CR·TAB·한글 멀티바이트·미완성 ESC/UTF-8 꼬리를
    // 모두 섞은 표본을, 청크 경계를 어긋나게 쪼개 흘려도 개선 전후 라인 목록·carry·partial·
    // pending_cr 상태가 완전히 일치해야 한다.
    #[test]
    fn ingest_refactor_output_bit_identical_to_pre_refactor() {
        let mut sample: Vec<u8> = Vec::new();
        sample.extend_from_slice("\x1b[31mRED\x1b[0m\tTAB\r\n".as_bytes()); // 색+TAB+CRLF
        sample.extend_from_slice("progress 10%\rprogress 100%\n".as_bytes()); // 단독 CR 프레임 연결
        sample.extend_from_slice("\x1b[2J\x1b[H가나다 한글 라인\n".as_bytes()); // 화면소거 CSI + 한글
        sample.extend_from_slice("no-newline-partial".as_bytes()); // 개행 없는 꼬리(partial 보류)
        sample.extend_from_slice("\x1b[1;32m더".as_bytes()); // SGR + 한글
        sample.extend_from_slice(&"가".as_bytes()[..2]); // 미완성 멀티바이트 꼬리(0xea 0xb0)
        let sample: &[u8] = &sample;
        // 여러 청크 크기로 경계를 어긋나게 쪼개 상태기계 인터리빙을 커버
        for split in [1usize, 2, 3, 5, 7, 13, 16, 64, sample.len()] {
            let mut st_new = fresh();
            let mut out_new = Vec::new();
            let mut st_ref = fresh();
            let mut out_ref = Vec::new();
            for piece in sample.chunks(split.max(1)) {
                ingest_step(&mut st_new, piece, &mut out_new);
                ingest_step_pre_refactor(&mut st_ref, piece, &mut out_ref);
            }
            assert_eq!(out_new, out_ref, "split={split}: 완성 라인 목록 불일치");
            assert_eq!(st_new.partial, st_ref.partial, "split={split}: partial 불일치");
            assert_eq!(st_new.carry, st_ref.carry, "split={split}: carry 불일치");
            assert_eq!(
                st_new.pending_cr, st_ref.pending_cr,
                "split={split}: pending_cr 불일치"
            );
        }
    }

    // D5 드레인 처리량 마이크로벤치 — 실 PTY 없이 ingest 라인분할만 직접 구동해 개선 전후
    // 단일스레드 처리 시간을 비교한다(할당 제거 효과 측정). `cargo test -- --nocapture`로
    // 수치 확인. 정확한 비율은 hard gate가 아니므로 assert는 회귀 안전(개선판이 참조판보다
    // 크게 느리지 않음)만 건다.
    #[test]
    fn ingest_drain_throughput_bench() {
        // ~4MB ANSI 혼합 데이터 생성(색코드 + 한글 + 개행)
        let mut data: Vec<u8> = Vec::with_capacity(4 * 1024 * 1024);
        let unit = "\x1b[31m로그\x1b[0m line item with some text 가나다라\n".as_bytes();
        while data.len() < 4 * 1024 * 1024 {
            data.extend_from_slice(unit);
        }
        let run = |f: &dyn Fn(&mut IngestState, &[u8], &mut Vec<String>)| -> (std::time::Duration, usize) {
            let mut st = fresh();
            let mut out = Vec::new();
            let start = std::time::Instant::now();
            for piece in data.chunks(16 * 1024) {
                f(&mut st, piece, &mut out);
                out.clear(); // 다운스트림 소비 흉내(scrollback으로 빠짐) — 메모리 성장 방지
            }
            (start.elapsed(), st.carry.len())
        };
        let (t_ref, _) = run(&ingest_step_pre_refactor);
        let (t_new, _) = run(&ingest_step);
        eprintln!(
            "[D5 bench] {}MB ANSI-mixed | pre-refactor={:?} refactored={:?} (Δ={:.1}%)",
            data.len() / (1024 * 1024),
            t_ref,
            t_new,
            (t_new.as_secs_f64() - t_ref.as_secs_f64()) / t_ref.as_secs_f64() * 100.0
        );
        // 회귀 가드: 개선판이 참조판 대비 크게 느려지면(2배+) 실패 — 노이즈 허용 상한.
        assert!(
            t_new <= t_ref * 2,
            "refactored ingest가 pre-refactor보다 2배+ 느림 (회귀): {t_new:?} vs {t_ref:?}"
        );
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

    // ─────────────────────────────────────────────────────────────────────────
    // ★T2 자기증폭 루프 실증(2026-08-01 윈도우 실사고) — 임시 재현 테스트.
    // 노드가 "경보를 논의하는 산문"을 화면에 출력하면 그 산문이 다시 health.alert를
    // 발화시킨다. 발생원 0건인데 경보만 증식한 실사고의 기계 재현.
    // ─────────────────────────────────────────────────────────────────────────

    /// 실사고 표본 — 노드(master·CSO·리뷰어)가 경보를 **논의하며 화면에 실제로 출력한** 산문.
    const INCIDENT_PROSE: &[&str] = &[
        "[CSO] 경보 요약: rate_limited 룰이 6분 만에 4건 → 10건으로 늘었는데 실제 발생원은 0건입니다.",
        "health.alert rule=rate_limited line=\"api: rate limit reached\" surface=3 — 원인 조사 중",
        "리뷰어 진단: 이 경보를 논의하는 산문이 새 경보를 발화시키는 자기증폭 루프였다 (rate limit 언급 자체가 트리거).",
        "master: token expired 경보도 같은 경로다 — 실제 세션은 정상인데 문장에 'token expired'가 들어가서 걸렸다.",
        "CSO 보고: 노드가 not logged in 상태로 오인 판정됐습니다. 실제로는 로그인 유지 중.",
        "복구 안내를 화면에 남깁니다 — 필요하면 please run /login 을 실행하세요.",
        "watchdog.duplicate_procs 4~5건(powershell.exe·claude.exe) — 401 unauthorized 경보와는 무관합니다.",
    ];

    /// 진짜 고장 신호(양성 대조) — 실제 도구·API가 뱉는 실패 라인.
    const REAL_FAILURE_LINES: &[&str] = &[
        "Error: not logged in",
        "HTTP 429 Too Many Requests",
        "401 Unauthorized",
        "your token has expired",
        "Please run /login to continue",
    ];

    /// ★수리 전(pre-fix) `run_health_rules` 매칭 미러 — 게이트도 마스킹도 없이 `is_match` 만
    /// 하던 구 로직 그대로다(D5 `ingest_step_pre_refactor` 와 동일 관행: 비교 기준을 코드로 박제).
    /// 이 미러가 매칭하는데 프로덕션 경로가 매칭하지 않으면 = 자기증폭 차단이 실제로 작동한 것.
    fn health_matches_pre_fix(line: &str) -> Vec<String> {
        default_health_rules()
            .into_iter()
            .filter(|r| r.regex.is_match(line))
            .map(|r| r.name)
            .collect()
    }

    /// 격리 데몬 + 역할 surface 하나를 만들어 (daemon, surface) 반환. PTY는 `sleep 30`
    /// (기존 governance/handlers 테스트와 동일 관행 — 라이브 원장·라이브 소켓 무접촉).
    fn health_probe_daemon(tag: &str) -> (Arc<Daemon>, Arc<Surface>) {
        let daemon = Daemon::new(isolated_sock(tag));
        let s = daemon
            .create_surface(None, Some("sleep 30".into()), None, Some("master".into()), 24, 80)
            .expect("create surface");
        daemon.surfaces.lock().unwrap().insert(s.id, s.clone());
        (daemon, s)
    }

    /// 라인들을 실제 PTY 배수 경로(ingest_output)로 흘려 넣고 발화한 health.alert를 회수한다.
    fn feed_lines_collect_alerts(
        daemon: &Arc<Daemon>,
        surface: &Arc<Surface>,
        lines: &[&str],
    ) -> Vec<(String, String)> {
        let seq_before = daemon.bus.tail(1).first().and_then(|e| e["seq"].as_u64()).unwrap_or(0);
        for l in lines {
            daemon.ingest_output(surface, format!("{l}\n").as_bytes());
        }
        daemon
            .bus
            .replay_after(seq_before)
            .into_iter()
            .filter(|e| e["name"].as_str() == Some("health.alert"))
            .map(|e| {
                (
                    e["payload"]["rule"].as_str().unwrap_or_default().to_string(),
                    e["payload"]["line"].as_str().unwrap_or_default().to_string(),
                )
            })
            .collect()
    }

    /// ★음성 대조(수용 기준) — 경보를 **논의하는 산문**은 신규 경보 0건이어야 한다.
    /// 델타 증명을 겸한다: 같은 표본을 수리 전 미러에 넣으면 매칭이 나온다(= 사고 재현).
    #[test]
    fn repro_alert_discussion_prose_must_not_fire_alerts() {
        // 수리 전 미러: 실사고 표본 전부가 매칭됐다(전 5룰 발화가 실측된 그 조건).
        let pre_fix: Vec<String> = INCIDENT_PROSE
            .iter()
            .flat_map(|l| health_matches_pre_fix(l))
            .collect();
        assert!(
            pre_fix.len() >= 5,
            "전제 실패: 수리 전 미러가 실사고 표본을 매칭하지 않음 ({pre_fix:?})"
        );

        let (daemon, s) = health_probe_daemon("health-amp-neg");
        let alerts = feed_lines_collect_alerts(&daemon, &s, INCIDENT_PROSE);
        assert!(
            alerts.is_empty(),
            "경보 논의 산문에서 신규 경보 {}건 발화(자기증폭): {:#?}",
            alerts.len(),
            alerts
        );
    }

    /// ★양성 대조: 진짜 고장 신호에는 여전히 경보가 떠야 한다(수리가 탐지를 죽이면 실패).
    #[test]
    fn repro_real_failure_lines_still_fire_alerts() {
        let (daemon, s) = health_probe_daemon("health-amp-pos");
        let alerts = feed_lines_collect_alerts(&daemon, &s, REAL_FAILURE_LINES);
        let rules: std::collections::HashSet<&str> =
            alerts.iter().map(|(r, _)| r.as_str()).collect();
        assert!(
            rules.contains("not_logged_in")
                && rules.contains("rate_limited")
                && rules.contains("auth_401")
                && rules.contains("token_expired")
                && rules.contains("login_required"),
            "진짜 고장 신호에서 경보 누락(탐지 사망): {:#?}",
            alerts
        );
    }

    /// ★기계 루프 차단(ⓐ 발신 봉인) — 사고의 주범 경로. CSO_DIRECTIVE.md:23 이 지시하는
    /// `cys events --category health --reconnect` 는 이벤트 JSON 라인을 그대로 `println!`
    /// (cys.rs stream_events)하므로, **경보 이벤트 자체가 구독 pane 의 화면 텍스트가 된다**.
    /// 그 라인을 다시 넣었을 때 새 경보가 나면 = LLM 서술 없이 성립하는 순수 자기증폭 루프.
    #[test]
    fn health_alert_event_echoed_into_a_pane_must_not_refire() {
        let (daemon, producer) = health_probe_daemon("health-echo-src");
        // ① 진짜 고장 라인 → 경보 발화(발생원 pane)
        let fired = feed_lines_collect_alerts(&daemon, &producer, REAL_FAILURE_LINES);
        assert!(!fired.is_empty(), "전제: 진짜 고장은 경보를 낸다");

        // ② ★봉인 불변식: 발화된 경보의 payload.line 은 마스킹돼 있고, **어떤 룰에도
        //    매칭되지 않는다**(= 그 문자열이 화면에 다시 찍혀도 재발화 불가).
        let all_rules = default_health_rules();
        for (rule, line) in &fired {
            assert!(
                line.contains(HEALTH_MASK_OPEN),
                "경보 payload.line 이 마스킹되지 않음 (rule={rule}): {line}"
            );
            for r in &all_rules {
                assert!(
                    !r.regex.is_match(line),
                    "경보 payload.line 이 룰 {}에 여전히 매칭(봉인 실패): {line}",
                    r.name
                );
            }
        }

        // ③ 그 경보 이벤트들을 `cys events` 가 출력하는 형태(JSON 한 줄)로 직렬화해
        //    구독자 pane(CSO 역할)의 화면 텍스트로 되먹인다.
        let echoed: Vec<String> = daemon
            .bus
            .replay_after(0)
            .into_iter()
            .filter(|e| e["name"].as_str() == Some("health.alert"))
            .map(|e| serde_json::to_string(&e).unwrap())
            .collect();
        assert!(!echoed.is_empty());

        // ④ ★수리 전 참조 미러(ingest_step_pre_refactor 와 같은 관행) — 구 `run_health_rules`는
        //    게이트·마스킹 없이 `is_match` 만 했다. **원문 payload 를 실은 구 이벤트 라인**을
        //    구 로직에 넣으면 새 경보가 나온다 = 루프가 실재했음을 기계로 보인다.
        let legacy_event_line = serde_json::to_string(&json!({
            "name": "health.alert", "category": "health", "surface_id": 3,
            "payload": {"rule": "rate_limited", "line": "api error: rate limit reached, retry later"},
        }))
        .unwrap();
        let pre_fix_hits = health_matches_pre_fix(&legacy_event_line);
        assert!(
            !pre_fix_hits.is_empty(),
            "전제 실패: 수리 전 로직이 경보 에코 라인에 재매칭하지 않음"
        );
        // 같은 라인을 **수리 후** 경로에 넣으면 0건이어야 한다.
        let post_fix = feed_lines_collect_alerts(&daemon, &producer, &[legacy_event_line.as_str()]);
        assert!(
            post_fix.is_empty(),
            "수리 후에도 구형 에코 라인이 재발화: {post_fix:#?}"
        );

        let subscriber = daemon
            .create_surface(None, Some("sleep 30".into()), None, Some("cso".into()), 24, 80)
            .expect("create subscriber surface");
        daemon.surfaces.lock().unwrap().insert(subscriber.id, subscriber.clone());
        let refs: Vec<&str> = echoed.iter().map(|s| s.as_str()).collect();
        let refired = feed_lines_collect_alerts(&daemon, &subscriber, &refs);
        assert!(
            refired.is_empty(),
            "경보 이벤트 에코가 신규 경보 {}건 재발화(기계 자기증폭 루프): {:#?}",
            refired.len(),
            refired
        );
    }

    /// ★`cys status`/control.dashboard 에코 경로 — recent_health 링의 line 도 마스킹되어
    /// 화면에 다시 렌더돼도 재발화하지 않아야 한다.
    #[test]
    fn recent_health_ring_is_masked_and_not_refirable() {
        let (daemon, s) = health_probe_daemon("health-ring-mask");
        feed_lines_collect_alerts(&daemon, &s, REAL_FAILURE_LINES);
        let ring: Vec<String> = daemon
            .recent_health
            .lock()
            .unwrap()
            .iter()
            .map(|e| serde_json::to_string(e).unwrap())
            .collect();
        assert_eq!(ring.len(), REAL_FAILURE_LINES.len(), "전제: 5건 기록");
        for raw in REAL_FAILURE_LINES {
            for entry in &ring {
                assert!(
                    !entry.contains(raw),
                    "recent_health 에 트리거 원문 유출: {entry}"
                );
            }
        }
        let refs: Vec<&str> = ring.iter().map(|s| s.as_str()).collect();
        let refired = feed_lines_collect_alerts(&daemon, &s, &refs);
        assert!(refired.is_empty(), "status 에코가 재발화: {refired:#?}");
    }

    /// ⓑ 격리 술어 단위 핀 — 사유별 판정과 **위음성 금지**(진짜 에러는 통과)를 박제한다.
    #[test]
    fn alert_discourse_reason_classifies_discourse_but_passes_real_errors() {
        // 매칭 구간을 직접 계산해 술어에 넘긴다(run_health_rules 와 동일 입력).
        let judge = |line: &str, rule: &str| -> Option<&'static str> {
            let rules = default_health_rules();
            let r = rules.iter().find(|r| r.name == rule).unwrap();
            let m = r.regex.find(line).unwrap_or_else(|| panic!("no match: {line}"));
            alert_discourse_reason(line, m.start(), m.end(), &rules)
        };
        // ① 기계장치 식별자
        assert_eq!(
            judge("health.alert rule=rate_limited line=\"rate limit\"", "rate_limited"),
            Some("alert-machinery-token")
        );
        // ② 룰 이름 언급(식별자 꼴)
        assert_eq!(
            judge("token_expired 룰 확인 요망 — token expired", "token_expired"),
            Some("rule-name-mention")
        );
        // ③ 인용 표기
        assert_eq!(judge("the \"rate limit\" alarm was noisy", "rate_limited"), Some("quoted-mention"));
        // ④ 한글 산문 서술
        assert_eq!(
            judge("이 경보를 논의하는 산문이 새 경보를 발화시킨다 (rate limit 언급 자체가 트리거)", "rate_limited"),
            Some("narration-prose")
        );
        // ⑤ ★위음성 금지 — 진짜 에러 라인은 전부 통과(None)
        for (line, rule) in [
            ("Error: not logged in", "not_logged_in"),
            ("HTTP 429 Too Many Requests", "rate_limited"),
            ("401 Unauthorized", "auth_401"),
            ("your token has expired", "token_expired"),
            ("Please run /login to continue", "login_required"),
            // 로그 프리픽스·후행 텍스트가 붙은 실제 로그 라인도 통과해야 한다
            ("2026-08-01T10:00:00Z [api] request failed: 401 Unauthorized, retrying", "auth_401"),
            // 짧은 한글이 섞인 현지화 라인은 임계(8) 미만이라 통과
            ("인증 실패: 401 Unauthorized", "auth_401"),
        ] {
            assert_eq!(judge(line, rule), None, "진짜 에러가 억제됨: {line}");
        }
    }

    /// ⓐ 마스킹 함수 핀 — 트리거 구간만 치환·나머지 보존·200자 상한(문자 경계 안전),
    /// 그리고 ★핵심 불변식: **산출물은 어떤 룰에도 매칭되지 않는다**(다중 트리거 한 줄 포함).
    #[test]
    fn mask_health_line_leaves_no_trigger_matchable() {
        let rules = default_health_rules();
        let masked = mask_health_line("Error: not logged in (session 3)", &rules);
        assert_eq!(masked, "Error: \u{2039}health-rule\u{203a} (session 3)");
        assert!(!masked.contains("not logged in"), "트리거 원문 잔존");

        // ★다중 트리거 한 줄 — 발화 룰 하나만 가리면 나머지가 새어 나간다(회귀 핀).
        let multi = "api: 401 Unauthorized and your token has expired, rate limit hit";
        let masked_multi = mask_health_line(multi, &rules);
        for r in &rules {
            assert!(
                !r.regex.is_match(&masked_multi),
                "마스킹 산출물이 룰 {}에 여전히 매칭: {masked_multi}",
                r.name
            );
        }
        // 마스크 토큰 자체가 룰을 재발화시키지 않는다(수렴 보장의 근거).
        for r in &rules {
            assert!(!r.regex.is_match(HEALTH_MASK), "마스크 토큰이 룰 {}에 매칭", r.name);
        }
        // 200자 상한 — 멀티바이트 경계에서 잘라도 패닉 없음
        let long = format!("{}not logged in", "가".repeat(300));
        assert_eq!(mask_health_line(&long, &rules).chars().count(), 200);
    }

    /// ⓑ 억제 관측 카운터 — 억제가 침묵하지 않고 사유별로 집계된다(원문은 담지 않는다).
    #[test]
    fn suppression_is_counted_by_reason_not_silent() {
        let (daemon, s) = health_probe_daemon("health-suppress-count");
        feed_lines_collect_alerts(&daemon, &s, INCIDENT_PROSE);
        let sup = daemon.health_suppressed.lock().unwrap();
        let total: u64 = sup.values().sum();
        assert!(total >= INCIDENT_PROSE.len() as u64 - 2, "억제 집계 누락: {sup:?}");
        assert!(
            sup.keys().any(|(_, reason)| *reason == "narration-prose"),
            "한글 서술 억제 사유 미기록: {sup:?}"
        );
    }

    // ─────────────────────────────────────────────────────────────────────────
    // ★T3-G2 동반 수리(적대 검증 FAIL 봉합) — 억제가 **안전 인터록까지** 삼키면 안 된다.
    //
    // T2 는 담화 판정이 서면 `continue` 로 룰 처리를 통째로 건너뛰었다. 그런데 그 건너뛴
    // 구간에는 경보 발신뿐 아니라 **`recent_health` 인터록 기록**이 함께 들어 있었다.
    // governance::check_agent_death 의 auth 무한 재기동 차단(`auth_blocked`)은 오직
    // `recent_health` 만 보므로, 한국어가 섞인 진짜 auth 실패 라인이 narration-prose 로
    // 분류되는 순간 차단 장치가 통째로 무력화된다(= 401 상대로 무한 재기동).
    // ─────────────────────────────────────────────────────────────────────────

    /// 진짜 auth 실패인데 한국어 문맥이 붙은 라인들(현장 실측형). narration-prose 로
    /// 분류되지만 **신호는 진짜**다 — 인터록에는 반드시 도달해야 한다.
    const KOREAN_REAL_AUTH_FAILURES: &[&str] = &[
        "node-recover: worker-1 재기동 중 401 unauthorized 가 반환되었습니다",
        "에이전트 기동 실패 — 응답 본문에 not logged in 이 담겨 돌아왔습니다 (자동 복구 중단)",
    ];

    #[test]
    fn korean_real_auth_failure_reaches_recent_health_interlock() {
        let (daemon, s) = health_probe_daemon("health-auth-interlock");
        // 전제 ①: 이 라인들은 (수리 전 로직 기준) 확실히 auth 계열 룰에 매칭된다.
        for line in KOREAN_REAL_AUTH_FAILURES {
            let hits = health_matches_pre_fix(line);
            assert!(
                hits.iter().any(|r| AUTH_INTERLOCK_RULES.contains(&r.as_str())),
                "전제 실패: 표본이 auth 룰에 매칭되지 않음 ({line} → {hits:?})"
            );
        }
        // 전제 ②: 이 라인들은 담화(narration-prose)로 분류된다 — 즉 억제 경로를 탄다.
        {
            let rules = default_health_rules();
            for line in KOREAN_REAL_AUTH_FAILURES {
                let r = rules.iter().find(|r| r.regex.is_match(line)).unwrap();
                let m = r.regex.find(line).unwrap();
                assert_eq!(
                    alert_discourse_reason(line, m.start(), m.end(), &rules),
                    Some("narration-prose"),
                    "전제 실패: 표본이 narration-prose 로 분류되지 않음 ({line})"
                );
            }
        }

        let alerts = feed_lines_collect_alerts(&daemon, &s, KOREAN_REAL_AUTH_FAILURES);
        // ★핵심: 인터록(recent_health)에는 도달해야 한다 — 이것이 auth_blocked 의 유일 근거.
        assert!(
            auth_blocked_by_recent_health(&daemon.recent_health.lock().unwrap(), s.id, now_epoch()),
            "진짜 auth 실패가 인터록에 도달하지 못함 → governance auth_blocked 무력화 \
             (recent_health={:?}, alerts={alerts:?})",
            daemon.recent_health.lock().unwrap()
        );
        // 인터록 기록도 마스킹 불변식을 지킨다(원문 유출 0 · 재매칭 0).
        let rules = default_health_rules();
        for e in daemon.recent_health.lock().unwrap().iter() {
            let line = e["line"].as_str().unwrap_or_default();
            for r in &rules {
                assert!(!r.regex.is_match(line), "인터록 기록이 룰 {}에 재매칭: {line}", r.name);
            }
        }
    }

    /// 반대 방향(음성 대조) — 경보를 **논의하는 산문**은 여전히 신규 경보 0건이어야 한다.
    /// (인터록 기록이 살아났다고 해서 발신 억제가 풀리면 자기증폭이 부활한다.)
    #[test]
    fn interlock_record_does_not_reopen_alert_amplification() {
        let (daemon, s) = health_probe_daemon("health-interlock-neg");
        let alerts = feed_lines_collect_alerts(&daemon, &s, INCIDENT_PROSE);
        assert!(alerts.is_empty(), "산문에서 신규 경보 발화(자기증폭 부활): {alerts:#?}");
        let alerts2 = feed_lines_collect_alerts(&daemon, &s, KOREAN_REAL_AUTH_FAILURES);
        assert!(
            alerts2.is_empty(),
            "담화 분류 라인은 인터록에만 남고 경보는 발신하지 않아야 한다: {alerts2:#?}"
        );
    }

    /// ★구조화 신호(JSON 값 자리)는 억제 대상이 아니다 — T2 의 `quoted-mention` 이 진짜 도구의
    /// 구조화 에러 출력을 "인용된 서술"로 오분류해 **경보를 통째로 지웠다**(진짜 고장 은폐).
    /// 산문 인용은 그대로 억제된다(양방향).
    #[test]
    fn structured_json_error_is_not_treated_as_quoted_mention() {
        let rules = default_health_rules();
        let judge = |line: &str| {
            let r = rules.iter().find(|r| r.regex.is_match(line)).unwrap();
            let m = r.regex.find(line).unwrap();
            alert_discourse_reason(line, m.start(), m.end(), &rules)
        };
        // ① 구조화 에러 출력(JSON 값 자리·logfmt 값 자리) = 진짜 신호 → 통과(None)
        assert_eq!(judge(r#"{"error":"rate limit"}"#), None);
        assert_eq!(judge(r#"level=error msg="rate limit" svc=api"#), None);
        // ② 산문 인용 = 여전히 담화(회귀 금지 — T2 판정 그대로 보존)
        assert_eq!(
            judge("the \"rate limit\" alarm was noisy"),
            Some("quoted-mention")
        );
        // ③ 실제 발신 경로에서도 구조화 라인은 경보를 낸다
        let (daemon, s) = health_probe_daemon("health-structured");
        let alerts = feed_lines_collect_alerts(&daemon, &s, &[r#"{"error":"rate limit"}"#]);
        assert!(!alerts.is_empty(), "구조화 실패 신호가 경보를 내지 못함");
    }

    /// ★인터록 원장 보호 — 경보를 논하는 수다(담화 항목)가 링을 채워도 진짜 auth 기록은
    /// 밀려나지 않아야 한다. 밀려나면 auth 무한 재기동 차단이 창 안에서 근거를 잃는다.
    #[test]
    fn discourse_entries_do_not_evict_real_alerts_from_interlock_ring() {
        let (daemon, s) = health_probe_daemon("health-ring-evict");
        // ① 진짜 실패 1건을 먼저 남긴다.
        feed_lines_collect_alerts(&daemon, &s, &["401 Unauthorized"]);
        assert!(auth_blocked_by_recent_health(
            &daemon.recent_health.lock().unwrap(),
            s.id,
            now_epoch()
        ));
        // ② 담화 라인을 링 용량의 두 배 넘게 쏟아붓는다(수다로 밀어내기 시도).
        let chatter: Vec<String> = (0..HEALTH_RING_CAP * 2)
            .map(|i| format!("[CSO] {i}번째 보고: rate limit 경보를 계속 논의하는 중입니다"))
            .collect();
        let refs: Vec<&str> = chatter.iter().map(|s| s.as_str()).collect();
        let alerts = feed_lines_collect_alerts(&daemon, &s, &refs);
        assert!(alerts.is_empty(), "담화가 경보를 발화(자기증폭): {alerts:#?}");
        // ③ 링은 상한을 지키면서도 진짜 항목은 살아남는다 → 인터록 유지.
        let recent = daemon.recent_health.lock().unwrap();
        assert!(recent.len() <= HEALTH_RING_CAP, "링 상한 위반: {}", recent.len());
        assert!(
            auth_blocked_by_recent_health(&recent, s.id, now_epoch()),
            "수다가 진짜 auth 기록을 밀어내 인터록이 무력화됨"
        );
    }

    /// ★두 소비자 분기 핀 — 같은 링을 인터록은 세고, 사람이 보는 경보 목록·`state=error` 판정은
    /// 세지 않는다. 이 비대칭이 깨지면 ①(인터록이 담화를 무시) 무한 재기동이 부활하거나
    /// ②(화면이 담화를 경보로 표시) 노드가 그 빨간불을 수리 일감으로 삼아 루프가 되살아난다.
    #[test]
    fn discourse_records_feed_interlock_but_not_the_human_alert_view() {
        let (daemon, s) = health_probe_daemon("health-two-consumers");
        feed_lines_collect_alerts(&daemon, &s, KOREAN_REAL_AUTH_FAILURES);
        let recent = daemon.recent_health.lock().unwrap();
        assert!(!recent.is_empty(), "전제: 인터록 기록이 남아야 한다");
        // ① 인터록은 센다
        assert!(auth_blocked_by_recent_health(&recent, s.id, now_epoch()));
        // ② 사람이 보는 경보 목록·error 판정에서는 전부 빠진다
        assert!(
            recent.iter().all(|e| !is_alert_record(e)),
            "담화 항목이 사람 경보 뷰에 노출됨: {recent:?}"
        );
        // 진짜 경보는 반대로 둘 다에 잡힌다
        drop(recent);
        feed_lines_collect_alerts(&daemon, &s, &["401 Unauthorized"]);
        let recent = daemon.recent_health.lock().unwrap();
        assert!(recent.iter().any(is_alert_record), "진짜 경보가 사람 뷰에서 누락");
    }

    /// 기계 에코(우리 경보의 반사)는 인터록에도 남기지 않는다 — 정보량 0인데 창만 갱신하면
    /// 인터록이 스스로 살아남는다(자기지속 상태).
    #[test]
    fn alert_machinery_echo_never_reaches_interlock() {
        let (daemon, s) = health_probe_daemon("health-echo-interlock");
        let echo = r#"{"name":"health.alert","payload":{"rule":"auth_401","line":"401 unauthorized"}}"#;
        feed_lines_collect_alerts(&daemon, &s, &[echo]);
        assert!(
            daemon.recent_health.lock().unwrap().is_empty(),
            "경보 기계 에코가 인터록에 기록됨: {:?}",
            daemon.recent_health.lock().unwrap()
        );
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
            tier: None,
            publisher_pid: None,
            publisher_pgid: None,
            publisher_surface: None,
            risk_class: None,
            auto_route: false,
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

    // ─────────────────────────────────────────────────────────────────────────
    // ★G1(W2-A) — 큐 WAL: 파일 등장순 보존·레거시 합성·queue_seq 시드·라운드트립 핀
    // ─────────────────────────────────────────────────────────────────────────

    /// WAL 테스트 전용 격리 dir — 단조 카운터로 같은 초 병렬 실행 간 공유를 차단
    /// (handlers::isolated_daemon 관례 동형).
    fn queue_wal_dir(tag: &str) -> PathBuf {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("cys-qwal-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    /// 레거시(구 WAL: mid/surface_id/text/role만) 합성 핀 — id=mid 재사용·seq=파일 등장순
    /// 재발급·enqueued_at=**복원 시각**(0.0 금지 · BLOCKER: 0.0이면 업그레이드 재기동 직후
    /// 전 항목이 wait≈수십억 초로 즉시 overdue 최전선 배달되는 stale 백로그 폭주). dedup은
    /// 파일 첫 등장 승.
    #[test]
    fn load_queue_state_legacy_synthesizes_id_seq_and_restore_time() {
        let dir = queue_wal_dir("legacy");
        let before = now_epoch();
        std::fs::write(
            dir.join("queue-state.json"),
            r#"[{"mid":"qaaa","surface_id":3,"text":"첫 메시지","role":"master"},
                {"mid":"qbbb","surface_id":3,"text":"둘째","role":"master"},
                {"mid":"qaaa","surface_id":3,"text":"첫 메시지","role":"master"}]"#,
        )
        .unwrap();
        let out = load_queue_state(&dir);
        assert_eq!(out.len(), 2, "mid dedup — 파일 첫 등장 승");
        assert_eq!(out[0]["id"], json!("qaaa"), "id=mid 재사용(재기동 간 안정)");
        assert_eq!(out[1]["id"], json!("qbbb"));
        assert_eq!(out[0]["seq"].as_u64(), Some(1), "seq=파일 등장순 재발급");
        assert_eq!(out[1]["seq"].as_u64(), Some(2));
        for it in &out {
            let ea = it["enqueued_at"].as_f64().expect("enqueued_at 합성 필수");
            assert!(
                ea >= before && ea <= now_epoch() + 1.0,
                "enqueued_at은 복원 시각이어야 한다(0.0 금지 · BLOCKER): {ea}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 순서 보존 회귀 핀 — 종전 HashMap.into_values()는 해시-랜덤 순서라 12건 규모에서
    /// 확률적으로 반드시 뒤섞였다(Vec+HashSet 재작성의 존재 이유). 신-포맷 필드
    /// (id/seq/enqueued_at/from/origin)는 재합성 없이 원값 보존.
    #[test]
    fn load_queue_state_preserves_file_order_and_new_fields() {
        let dir = queue_wal_dir("order");
        let arr: Vec<serde_json::Value> = (0..12)
            .map(|i| {
                json!({
                    "mid": format!("qm{i}"), "id": format!("qid.{i}"), "seq": i + 100,
                    "surface_id": 7, "role": "worker", "text": format!("m{i}"),
                    "enqueued_at": 1000.0 + i as f64, "from": "surface:9", "origin": "send",
                })
            })
            .collect();
        std::fs::write(dir.join("queue-state.json"), serde_json::to_string(&arr).unwrap())
            .unwrap();
        let out = load_queue_state(&dir);
        assert_eq!(out.len(), 12);
        for (i, it) in out.iter().enumerate() {
            assert_eq!(
                it["id"],
                json!(format!("qid.{i}")),
                "파일 등장순 보존(해시-랜덤 순서 회귀 핀)"
            );
            assert_eq!(it["seq"].as_u64(), Some(i as u64 + 100), "seq 원값 보존(재발급 금지)");
            assert_eq!(it["enqueued_at"].as_f64(), Some(1000.0 + i as f64), "원값 보존(재합성 금지)");
            assert_eq!(it["from"], json!("surface:9"));
            assert_eq!(it["origin"], json!("send"));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// id·mid 둘 다 없는 항목은 신원 불능 — 복원하지 않는다(fail-safe · 종전 mid-필수와 동일 방향).
    #[test]
    fn load_queue_state_drops_identityless_entries() {
        let dir = queue_wal_dir("noid");
        std::fs::write(
            dir.join("queue-state.json"),
            r#"[{"surface_id":3,"text":"신원 없음"},{"mid":"qok","surface_id":3,"text":"정상"}]"#,
        )
        .unwrap();
        let out = load_queue_state(&dir);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["id"], json!("qok"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// [WAL 왕복 + queue_seq 시드] persist→load 라운드트립: id/seq/enqueued_at/from/origin·
    /// role·mid(구 데몬 롤백 하위호환 병기) 보존 + 시드 = 복원 항목 max(seq)+1(재기동 후 발급
    /// seq가 살아있는 복원 항목과 절대 불충돌) + id 조립 = boot 식별자(started_at) + seq.
    #[test]
    fn queue_seq_seeds_from_wal_max_and_persist_load_roundtrip() {
        let dir = queue_wal_dir("seed");
        std::fs::write(
            dir.join("queue-state.json"),
            r#"[{"mid":"qzz","id":"qzz","seq":7,"surface_id":3,"role":"ghost-role",
                 "text":"복원 대기","enqueued_at":1234.5,"from":"surface:3","origin":"send"}]"#,
        )
        .unwrap();
        let daemon = Daemon::new(dir.join("cysd.sock"));
        assert_eq!(daemon.queue_seq.load(Ordering::SeqCst), 8, "시드 = max(seq)+1");
        let e = daemon.next_queue_entry("본문".into(), Some("surface:1".into()), "send");
        assert_eq!(e.seq, 8);
        assert_eq!(
            e.id,
            format!("q{:x}.8", daemon.started_at as u64),
            "id = boot 식별자(started_at) + seq — 재기동 간 충돌 차단"
        );
        assert!(e.enqueued_at > 0.0);
        // 라이브 surface 큐 + 미소비 restored 병존 → persist → load 필드 보존
        let s = daemon
            .create_surface(None, Some("sleep 30".into()), None, Some("w2a-role".into()), 24, 80)
            .expect("create surface");
        daemon.surfaces.lock().unwrap().insert(s.id, s.clone());
        s.pending_queue.lock().unwrap().push_back(e.clone());
        daemon.persist_queue_state();
        let out = load_queue_state(&dir);
        assert_eq!(out.len(), 2, "라이브 1 + restored 1");
        let live = out
            .iter()
            .find(|it| it["id"] == json!(e.id))
            .expect("라이브 항목이 WAL에 있어야 한다");
        assert_eq!(live["seq"].as_u64(), Some(8));
        assert_eq!(live["text"], json!("본문"));
        assert_eq!(live["enqueued_at"].as_f64(), Some(e.enqueued_at), "f64 왕복 보존");
        assert_eq!(live["from"], json!("surface:1"));
        assert_eq!(live["origin"], json!("send"));
        assert_eq!(live["role"], json!("w2a-role"));
        assert_eq!(
            live["mid"],
            json!(queue_mid(s.id, "본문")),
            "mid 병기 = 구 데몬 롤백 하위호환(구 코드는 mid/surface_id/text/role만 읽음)"
        );
        let restored = out
            .iter()
            .find(|it| it["id"] == json!("qzz"))
            .expect("미소비 restored 항목 보존");
        assert_eq!(restored["seq"].as_u64(), Some(7));
        assert_eq!(restored["enqueued_at"].as_f64(), Some(1234.5));
        // 정리 — 스폰 자식 회수 + 임시 dir 제거
        {
            let mut child = s.child.lock().unwrap();
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// rehome(기계 이식분) 핀 — WAL 원값(id/seq/enqueued_at/from/origin)이 QueueEntry로
    /// 보존 승계되는지 확인(정렬 병합은 W2-C 별도 티켓 — 여기서는 필드 관통만 고정).
    #[test]
    fn rehome_restores_queue_entry_with_original_metadata() {
        let dir = queue_wal_dir("rehome");
        std::fs::write(
            dir.join("queue-state.json"),
            r#"[{"mid":"qrr","surface_id":3,"role":"w2a-rehome","text":"레거시 복원"}]"#,
        )
        .unwrap();
        let daemon = Daemon::new(dir.join("cysd.sock"));
        let s = daemon
            .create_surface(None, Some("sleep 30".into()), None, Some("w2a-rehome".into()), 24, 80)
            .expect("create surface");
        daemon.surfaces.lock().unwrap().insert(s.id, s.clone());
        assert_eq!(daemon.rehome_restored_queue(), 1);
        let q = s.pending_queue.lock().unwrap();
        let e = q.front().expect("rehome된 항목");
        assert_eq!(e.id, "qrr", "레거시 id=mid 승계(재기동 간 안정 ID)");
        assert_eq!(e.seq, 1, "load가 합성한 파일 등장순 seq 승계");
        assert!(e.enqueued_at > 0.0, "복원 시각 합성 승계(0.0 금지)");
        assert_eq!(e.text, "레거시 복원");
        assert_eq!(e.origin, "wal-legacy", "레거시 origin 표기");
        drop(q);
        {
            let mut child = s.child.lock().unwrap();
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ─── ★G1(W2-C): rehome 정렬 병합 + queue_merge_insert_pos 순수 핀 ─────────

    fn w2c_entry(id: &str, seq: u64, enqueued_at: f64) -> QueueEntry {
        QueueEntry {
            id: id.to_string(),
            seq,
            text: format!("본문 {id}"),
            enqueued_at,
            from: None,
            origin: "send".to_string(),
        }
    }

    /// 순수 판정자 핀 — (enqueued_at, seq) 삽입 위치: 동률은 기존/선삽입 승(stable),
    /// enqueued_at 동률·역행은 seq 타이브레이커, NaN은 기존 승(보수적), 빈 큐는 0(=append).
    #[test]
    fn queue_merge_insert_pos_orders_by_enqueued_at_then_seq() {
        let empty: VecDeque<QueueEntry> = VecDeque::new();
        assert_eq!(queue_merge_insert_pos(&empty, 10.0, 1), 0, "빈 큐 = append 위치 0");
        let q: VecDeque<QueueEntry> =
            vec![w2c_entry("a", 1, 10.0), w2c_entry("b", 2, 20.0)].into();
        assert_eq!(queue_merge_insert_pos(&q, 5.0, 9), 0, "전원보다 과거 → 최전선");
        assert_eq!(queue_merge_insert_pos(&q, 15.0, 9), 1, "사이 시각 → 중간 삽입");
        assert_eq!(queue_merge_insert_pos(&q, 30.0, 9), 2, "전원보다 신규 → append");
        // 동률 enqueued_at: seq 타이브레이커(boot 내 단조 — 시계 스큐 방어).
        assert_eq!(queue_merge_insert_pos(&q, 10.0, 0), 0, "동시각·더 작은 seq → 앞");
        assert_eq!(queue_merge_insert_pos(&q, 10.0, 1), 1, "동시각·동일 seq → 기존 승(stable)");
        assert_eq!(queue_merge_insert_pos(&q, 10.0, 5), 1, "동시각·더 큰 seq → 뒤");
        // NaN(비교 불능): 기존 항목 승 — 순서의 1차 진실은 deque 위치.
        let nan_q: VecDeque<QueueEntry> = vec![w2c_entry("n", 3, f64::NAN)].into();
        assert_eq!(queue_merge_insert_pos(&nan_q, 10.0, 1), 1, "NaN 기존 항목 → append");
    }

    /// ★결함 3(순서 역전) 봉인 핀 — 재기동 전 구 항목(enqueued_at 과거)이 재기동 직후
    /// enqueue된 신규 라이브 항목보다 **앞**에 병합된다. 종전 무조건 push_back이면
    /// [라이브, (파일순) 구2, 구1]이 되어 이 테스트가 실패한다. WAL 파일은 일부러
    /// (enqueued_at, seq) 역순으로 적어 배치 정렬까지 함께 핀한다.
    /// + queue.rehomed 이벤트: queue_entry_ids 병합 순서·reordered=true·role·
    ///   `entry_ids` 키(W-id 에코 계약) 절대 부재.
    #[test]
    fn rehome_sorted_merge_places_restored_before_newer_live_entries() {
        let dir = queue_wal_dir("w2c-merge");
        std::fs::write(
            dir.join("queue-state.json"),
            r#"[{"id":"qold.2","seq":2,"surface_id":9,"role":"w2c-merge","text":"재기동 전 2","enqueued_at":200.0,"origin":"send"},
                {"id":"qold.1","seq":1,"surface_id":9,"role":"w2c-merge","text":"재기동 전 1","enqueued_at":100.0,"origin":"send"}]"#,
        )
        .unwrap();
        let daemon = Daemon::new(dir.join("cysd.sock"));
        let s = daemon
            .create_surface(None, Some("sleep 30".into()), None, Some("w2c-merge".into()), 24, 80)
            .expect("create surface");
        daemon.surfaces.lock().unwrap().insert(s.id, s.clone());
        // 재기동 직후 도착한 신규 라이브 메시지(enqueued_at=now ≫ 200.0).
        let live = daemon.next_queue_entry("재기동 후 신규".into(), None, "send");
        s.pending_queue.lock().unwrap().push_back(live.clone());
        assert_eq!(daemon.rehome_restored_queue(), 2);
        {
            let q = s.pending_queue.lock().unwrap();
            let order: Vec<&str> = q.iter().map(|e| e.id.as_str()).collect();
            assert_eq!(
                order,
                vec!["qold.1", "qold.2", live.id.as_str()],
                "구 항목이 (enqueued_at, seq) 순으로 신규 라이브 앞에 병합돼야 한다(결함 3 봉인)"
            );
        }
        let rehomed: Vec<serde_json::Value> = daemon
            .bus
            .replay_after(0)
            .into_iter()
            .filter(|e| e["name"] == json!("queue.rehomed"))
            .collect();
        assert_eq!(rehomed.len(), 1, "role 배치당 1회 발행");
        let ev = &rehomed[0];
        assert_eq!(ev["category"], json!("queue"));
        assert_eq!(ev["surface_id"], json!(s.id));
        assert_eq!(ev["payload"]["count"], json!(2));
        assert_eq!(
            ev["payload"]["queue_entry_ids"],
            json!(["qold.1", "qold.2"]),
            "queue_entry_ids = 병합 삽입 순서"
        );
        assert_eq!(ev["payload"]["role"], json!("w2c-merge"));
        assert_eq!(ev["payload"]["reordered"], json!(true), "기존 라이브 항목이 뒤로 밀림");
        assert!(
            ev["payload"].get("entry_ids").is_none(),
            "entry_ids 키명은 W-id 에코 전용 — 재사용 금지(성찰 BLOCKER)"
        );
        {
            let mut child = s.child.lock().unwrap();
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 대조군 — 대상 큐가 비어 있으면 순수 append(기존 항목 밀림 없음) → reordered=false.
    /// 배치 정렬(파일 역순 → (enqueued_at, seq) 오름차순)은 여기서도 유지된다.
    #[test]
    fn rehome_into_empty_queue_reports_reordered_false() {
        let dir = queue_wal_dir("w2c-empty");
        std::fs::write(
            dir.join("queue-state.json"),
            r#"[{"id":"qe.2","seq":2,"surface_id":9,"role":"w2c-empty","text":"둘","enqueued_at":200.0,"origin":"send"},
                {"id":"qe.1","seq":1,"surface_id":9,"role":"w2c-empty","text":"하나","enqueued_at":100.0,"origin":"send"}]"#,
        )
        .unwrap();
        let daemon = Daemon::new(dir.join("cysd.sock"));
        let s = daemon
            .create_surface(None, Some("sleep 30".into()), None, Some("w2c-empty".into()), 24, 80)
            .expect("create surface");
        daemon.surfaces.lock().unwrap().insert(s.id, s.clone());
        assert_eq!(daemon.rehome_restored_queue(), 2);
        {
            let q = s.pending_queue.lock().unwrap();
            let order: Vec<&str> = q.iter().map(|e| e.id.as_str()).collect();
            assert_eq!(order, vec!["qe.1", "qe.2"], "빈 큐에도 배치 정렬 순서 유지");
        }
        let ev = daemon
            .bus
            .replay_after(0)
            .into_iter()
            .find(|e| e["name"] == json!("queue.rehomed"))
            .expect("queue.rehomed 발행");
        assert_eq!(ev["payload"]["reordered"], json!(false), "밀린 기존 항목 없음");
        assert_eq!(ev["payload"]["queue_entry_ids"], json!(["qe.1", "qe.2"]));
        {
            let mut child = s.child.lock().unwrap();
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// W2-C 신규 이벤트 payload 빌더 스키마 핀 — queue.rehomed / queue.migrated.
    /// json! payload는 컴파일러 강제 밖 — 키 존재·순서 보존·`entry_ids` 부재를 고정한다.
    #[test]
    fn queue_rehomed_and_migrated_payloads_pin_schema() {
        let batch = vec![w2c_entry("qr.1", 1, 10.0), w2c_entry("qr.2", 2, 20.0)];
        let p = queue_rehomed_payload("worker", &batch, true);
        assert_eq!(p["count"], json!(2));
        assert_eq!(p["queue_entry_ids"], json!(["qr.1", "qr.2"]), "병합 삽입 순서 보존");
        assert_eq!(p["role"], json!("worker"));
        assert_eq!(p["reordered"], json!(true));
        assert!(p.get("entry_ids").is_none(), "entry_ids 키명 재사용 금지(성찰 BLOCKER)");

        let m = queue_migrated_payload(3, 7, "master", &batch);
        assert_eq!(m["from_surface"], json!(3));
        assert_eq!(m["to_surface"], json!(7));
        assert_eq!(m["queue_entry_ids"], json!(["qr.1", "qr.2"]), "append 순서 보존");
        assert_eq!(m["role"], json!("master"));
        assert!(m.get("entry_ids").is_none(), "entry_ids 키명 재사용 금지(성찰 BLOCKER)");
    }

    /// ★G1(W2-E) queue.reordered payload 스키마 핀 — 강제 배달의 비머리 끌어올림 기록.
    /// 단수 키는 명명 계약대로 `queue_entry_id`(`entry_id` 키명 절대 부재 — 성찰 BLOCKER),
    /// to_index 는 항상 0(머리), cause 어휘는 "force_deliver" 하나(supersede 릴리스 제외).
    #[test]
    fn queue_reordered_payload_pins_schema() {
        let e = w2c_entry("qx.4", 4, 40.0);
        let p = queue_reordered_payload("surface:9", &e, 2, "force_deliver");
        assert_eq!(p["surface_ref"], json!("surface:9"));
        assert_eq!(p["queue_entry_id"], json!("qx.4"), "단수 키 = queue_entry_id(명명 계약)");
        assert_eq!(p["seq"], json!(4));
        assert_eq!(p["from_index"], json!(2));
        assert_eq!(p["to_index"], json!(0), "끌어올림 목적지는 항상 머리(0)");
        assert_eq!(p["cause"], json!("force_deliver"));
        assert!(p.get("entry_id").is_none(), "entry_id 키명 금지(W-id 에코 체계와 혼동 차단)");
        assert!(p.get("entry_ids").is_none(), "entry_ids 키명 재사용 금지(성찰 BLOCKER)");
    }

    // ─── ★G1(W2-B): 큐 이벤트 payload 빌더 스키마 핀 ─────────────────────────
    // json! 페이로드는 컴파일러 강제 밖 — 기존 키(소비자 계약)와 additive 키를 여기서 고정.

    fn w2b_entry(id: &str, seq: u64, text: &str, enqueued_at: f64) -> QueueEntry {
        QueueEntry {
            id: id.to_string(),
            seq,
            text: text.to_string(),
            enqueued_at,
            from: Some("surface:1".to_string()),
            origin: "send".to_string(),
        }
    }

    /// 폐기 3발행처 공용 스키마 핀 — reason 어휘 3종 전부에서 기존 키(reason/count/bytes)
    /// 값 불변 + queue_entry_ids 순서 보존. `entry_ids` 키(W-id 에코 계약)는 절대 부재.
    #[test]
    fn queue_dropped_payload_pins_existing_keys_and_adds_queue_entry_ids() {
        let dropped = vec![w2b_entry("qa.1", 1, "첫", 10.0), w2b_entry("qa.2", 2, "둘째", 20.0)];
        for reason in ["process_exited", "surface_closed", "cleared"] {
            let p = queue_dropped_payload(reason, &dropped);
            assert_eq!(p["reason"], json!(reason), "기존 키 reason 불변");
            assert_eq!(p["count"], json!(2), "기존 키 count 불변");
            assert_eq!(
                p["bytes"],
                json!("첫".len() + "둘째".len()),
                "기존 키 bytes = 본문 바이트 합 불변"
            );
            assert_eq!(
                p["queue_entry_ids"],
                json!(["qa.1", "qa.2"]),
                "additive queue_entry_ids — 큐 순서 보존"
            );
            assert!(
                p.get("entry_ids").is_none(),
                "entry_ids 키명은 W-id 에코 전용(javis_report_gate disarm 조인 키) — 재사용 금지"
            );
        }
        // 빈 drain 은 발행처가 발행 자체를 생략하지만, 빌더 자체도 안전해야 한다.
        let empty = queue_dropped_payload("cleared", &[]);
        assert_eq!(empty["count"], json!(0));
        assert_eq!(empty["queue_entry_ids"], json!([] as [&str; 0]));
    }

    /// enqueue 3경로 공용 스키마 핀 — 기존 키(bytes/depth/from · send-key 만 key) 불변 +
    /// queue_entry_id/seq/enqueued_at additive. key 는 send-key 경로에서만 존재한다.
    #[test]
    fn queue_enqueued_payload_pins_existing_keys_and_additive_ids() {
        // send 경로꼴: from = 클라이언트 문자열 or null, key 없음.
        let e = w2b_entry("qb.5", 5, "안녕", 111.5);
        let p = queue_enqueued_payload(&e, 3, json!("w1"), None);
        assert_eq!(p["bytes"], json!("안녕".len()), "기존 키 bytes 불변(UTF-8 바이트)");
        assert_eq!(p["depth"], json!(3), "기존 키 depth 불변");
        assert_eq!(p["from"], json!("w1"), "기존 키 from 불변(호출자 전달값 그대로)");
        assert!(p.get("key").is_none(), "send 경로에 key 키 없음(무회귀)");
        assert_eq!(p["queue_entry_id"], json!("qb.5"), "additive 조준점");
        assert_eq!(p["seq"], json!(5));
        assert_eq!(p["enqueued_at"], json!(111.5));

        // send-key 경로꼴: text="" → bytes 0, key="Return" 유지.
        let ek = w2b_entry("qb.6", 6, "", 112.0);
        let pk = queue_enqueued_payload(&ek, 1, json!(null), Some("Return"));
        assert_eq!(pk["bytes"], json!(0), "send-key 는 bytes 0(무회귀)");
        assert_eq!(pk["key"], json!("Return"), "기존 키 key 불변");
        assert_eq!(pk["from"], json!(null), "from 부재 = null(무회귀)");
        assert_eq!(pk["queue_entry_id"], json!("qb.6"));

        // governance 경로꼴: from = "governance-approval" 고정 문자열.
        let pg = queue_enqueued_payload(&e, 2, json!("governance-approval"), None);
        assert_eq!(pg["from"], json!("governance-approval"));
    }

    /// 배달 영수증 스키마 핀 — 기존 4키(bytes/remaining/entry_ids/surface_ref) 의미 불변:
    /// entry_ids 는 전달된 W-id 에코 그대로(큐 항목 id 와 절대 별개 체계). additive 5키 +
    /// wait_secs 음수 클램프 0(시계 스큐 방어). ★G1(W2-D): overdue/forced additive 추가.
    #[test]
    fn queue_delivered_payload_pins_wid_echo_and_wait_clamp() {
        let e = w2b_entry("qc.9", 9, "[wakeup W-a1b2] 보고", 100.0);
        let wids = vec!["W-a1b2".to_string()];
        let p = queue_delivered_payload(&e, 4, &wids, "surface:12", 107.9, false, false);
        assert_eq!(p["bytes"], json!(e.text.len()), "기존 키 bytes 불변");
        assert_eq!(p["remaining"], json!(4), "기존 키 remaining 불변");
        assert_eq!(
            p["entry_ids"],
            json!(["W-a1b2"]),
            "entry_ids = W-id 에코(원문 기준) 그대로 — critical disarm 조인 키 불변"
        );
        assert_eq!(p["surface_ref"], json!("surface:12"), "기존 키 surface_ref 불변");
        assert_eq!(p["queue_entry_id"], json!("qc.9"), "큐 항목 id 는 별도 키로만");
        assert_ne!(
            p["queue_entry_id"], p["entry_ids"][0],
            "두 id 체계는 같은 값·같은 키로 섞이지 않는다"
        );
        assert_eq!(p["seq"], json!(9));
        assert_eq!(p["enqueued_at"], json!(100.0));
        assert_eq!(p["delivered_at"], json!(107.9));
        assert_eq!(p["wait_secs"], json!(7), "wait = delivered - enqueued 내림(u64)");
        // ★G1(W2-D): 정상(watchdog·비완화) 배달의 additive 기본값 — 둘 다 false.
        assert_eq!(p["overdue"], json!(false), "additive overdue — 정상 배달은 false");
        assert_eq!(p["forced"], json!(false), "additive forced — watchdog 배달은 false");
        // W-id 봉입 없는 일반 배달 = 빈 배열(키는 항상 존재 — 에코 계약).
        let p0 = queue_delivered_payload(&e, 0, &[], "surface:12", 99.0, false, false);
        assert_eq!(p0["entry_ids"], json!([] as [&str; 0]));
        assert_eq!(p0["wait_secs"], json!(0), "시계 역행(delivered < enqueued)은 0 클램프");
        // overdue(단계형 제한 배달)·forced(운영자 강제) 표기는 이벤트 층에서만 구분된다.
        let po = queue_delivered_payload(&e, 0, &[], "surface:12", 108.0, true, false);
        assert_eq!(po["overdue"], json!(true));
        assert_eq!(po["forced"], json!(false));
        let pf = queue_delivered_payload(&e, 0, &[], "surface:12", 108.0, false, true);
        assert_eq!(pf["forced"], json!(true));
        // W-id 에코·기존 키는 overdue/forced 와 무관하게 동일(계약 불변).
        assert_eq!(po["bytes"], p["bytes"]);
        assert_eq!(po["surface_ref"], p["surface_ref"]);
    }

    /// ★G1(W2-D) queue.starved payload 핀 — 스키마 7키 + hint 문구 계약(성찰 BLOCKER):
    /// hint 는 운영자(사람) 판단 전제를 명시하고 LLM 에이전트 자동 반응을 금지해야 한다.
    /// 이벤트 실소비자가 LLM 에이전트인 시스템에서 hint 가 강제 배달을 직접 지시하면
    /// '경보 → 반사적 강제 드레인' 폭주 회로가 열린다 — 문면 자체를 핀으로 고정.
    #[test]
    fn queue_starved_payload_pins_schema_and_operator_only_hint() {
        let head = w2b_entry("qs.3", 3, "오래 기다린 머리", 50.0);
        let p = queue_starved_payload(
            "surface:7",
            Some("worker".into()),
            &head,
            700,
            2,
            "busy(출력 중)",
        );
        assert_eq!(p["surface_ref"], json!("surface:7"));
        assert_eq!(p["role"], json!("worker"));
        assert_eq!(p["head_entry_id"], json!("qs.3"), "머리 항목 조준점 = 큐 entry id");
        assert_eq!(p["waited_secs"], json!(700));
        assert_eq!(p["depth"], json!(2));
        assert_eq!(p["blocked_by"], json!("busy(출력 중)"));
        assert_eq!(p["hint"], json!(QUEUE_STARVED_HINT), "hint 문구 = 상수 계약 그대로");
        // 문구 계약의 핵심 2요소: ①운영자(사람) 판단 명시 ②자동 반응 금지 명시.
        assert!(QUEUE_STARVED_HINT.contains("운영자(사람) 판단"), "사람 판단 전제 명시");
        assert!(QUEUE_STARVED_HINT.contains("자동 반응"), "자동 반응 금지 명시");
        assert!(QUEUE_STARVED_HINT.contains("금지"), "금지 문면 존재");
        // role 없는 맨 셸 = null (depth_high 의 role 직렬화 관례와 동형).
        let p2 = queue_starved_payload("surface:8", None, &head, 700, 1, "queue_paused(헬스 조치)");
        assert_eq!(p2["role"], json!(null));
    }
}
