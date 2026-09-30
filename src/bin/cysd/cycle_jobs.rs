//! ★(0.14.42 · clear 가드 수정 6회차 V42R-1) 비동기 사이클 집행 — `cys cycle-agent --detach`.
//!
//! 【왜】 CSO 는 모든 좌석의 `context.threshold` 를 `cys cycle-agent` 1콜로 집행한다. 그 1콜은 저장·검증·유휴·clear 확인·재주입을
//! 기다리느라 수 분(단일 전체 시한 570초 · 실측 219~231초)이고, CSO 는 능력 게이트가 `run_in_background` 를 거부해 전경으로만 돌린다 —
//! 그래서 한 번에 한 1콜이었다. 다른 좌석의 1콜이 도는 동안 도착한 master 의 가장자리(85%) 통보는 그 1콜이 끝날 때까지 줄에서
//! 기다렸고, 자동 압축을 끈 200K master 는 그동안 차단점(88.5)을 넘어 저장 지시가 거부됐다(재검증 4-2 ROLE V42R-1 · 드릴 v42d-a/b ·
//! 모형 좌석 2 + 1콜 231초 28/40 씨앗 막힘 · 단독 0/40). 순서 규칙만으로는 **이미 진행 중인** 1콜 뒤의 대기를 없앨 수 없다.
//!
//! 【무엇】 CSO(또는 다른 집행자 좌석)의 1콜이 `--detach` 를 달면 CLI 는 이 모듈에 **접수만** 하고 곧바로 돌아온다(rc 89). 데몬은
//! 같은 `cys cycle-agent`(동기판 · 형제 CLI)를 자식으로 띄워 끝까지 붙들고(시한 · 동시 상한 · 좌석당 단일 비행), 끝나면 결과(rc ·
//! 사유 1줄)를 **요청 좌석의 큐로 1회** 넣는다(`[cycle-result] …` · 멱등 = 작업 번호당 적재 성공 1회). 집행의 결정 — 어느 좌석을 ·
//! 어느 통보(`--fire`)로 · 어느 검증자로 · 언제 — 은 여전히 요청 좌석(CSO)의 1콜이 한다. 데몬은 경보를 보고 스스로 사이클을
//! 여는 경로가 없다(이 모듈의 입구는 `surface.cycle_detach` RPC 하나 · clear 집행 거버넌스 무변경 — self-clear 금지 · master↔CSO
//! 상호 집행).
//!   · 신원 위임: 데몬이 띄운 자식 pid 는 **요청 좌석의 신원**으로 해소된다(`Daemon::delegated_callers` · 조상 추적 0홉) — ACL ·
//!     권위 주입의 타이핑 가드 면제 · 점유(`surface.cycle_claim`)·표지(`bind_owner`)의 소유자가 CSO 가 손으로 부른 1콜과 같다.
//!     등록은 자식이 첫 RPC 를 내기 **전**이다(자식은 stdin 의 `go` 한 줄을 받을 때까지 아무것도 보내지 않는다 — 경합 0).
//!   · 동시 상한: 진행 중 ≤ [`MAX_RUNNING`](3) · 그중 master·CEO 밖 좌석은 ≤ [`MAX_RUNNING_GENERAL`](2) — 한 칸은 늘 master·CEO
//!     (라우터 `CLEAR_SIGNAL_ROLES`) 몫이다. 넘치면 데몬 안에서 기다린다(대기열 ≤ [`MAX_PENDING`] · 좌석당 하나) — 순서는 master·CEO
//!     먼저 → 차단점까지 여유(C − 통보 퍼센트)가 작은 좌석 먼저 → 접수 순. 기다림은 CSO 의 턴이 아니다.
//!   · 좌석당 단일 비행: 같은 좌석에 산 점유(`cycle_claim`)나 진행·대기 작업이 있으면 접수하지 않는다(rc 88 — 종전 계약 그대로 ·
//!     그 사이클이 clear 전에 끝나면 데몬 재배달). 통보 뒤 사이클이 이미 끝났으면 rc 87. 같은 요청 좌석이 같은 통보를 이미
//!     detach 로 집행했고 88 밖의 결과였으면 다시 접수하지 않는다(rc 87 · CSO 지침 '내 집행 실패 통보의 재배달은 재집행하지 않는다'의
//!     기계 집행 — 폭주 ① 없음).
//!   · 검증자 교차: 진행 중 작업의 검증자 좌석을 **대상**으로 하는 작업은 그 작업이 끝날 때까지 기다린다(검증 중인 좌석을 비우지 않는다) ·
//!     master·CEO 밖 작업은 검증자가 진행 중 사이클의 대상이면 기다린다(비워지는 좌석으로 검증 요청을 보내지 않는다) — master·CEO 는
//!     기다리지 않는다(그 대기가 V42R-1 자체다 · 사이클 4단계가 검증자 좌석의 유휴를 기다리므로 검증 요청은 clear 전에 처리된다).
//!   · kill-switch: 동결(`system.pause`) 중에는 접수하지 않고(rc 1 · 송신 0건) 대기 작업도 시작하지 않는다(재개 뒤 수집기 틱이 시작).
//!   · 시한: 자식은 자기 단일 전체 시한(570초)으로 끝난다 — 데몬은 [`KILL_AFTER_SECS`](630) 뒤에도 살아 있으면 죽인다(점유·표지는
//!     죽은 pid 로 데몬이 푼다 · 결과는 '시한 초과로 종료').
//!   · 윈도우: 자식은 `ChildLifetime::Attached`(창 숨김만 · 세션·그룹 분리 없음 — 데몬과 함께 죽는 유계 자식)이고 stdin·stderr 파이프,
//!     `Child::try_wait`·`kill` 만 쓴다(OS 분기 0 · 새 분리 코드 0). 자식의 peer pid(Windows named pipe `GetNamedPipeClientProcessId`)
//!     = `Child::id()` 라 위임 해소가 같다.
//! 상태는 휘발이다(데몬 재기동 뒤 진행 중 자식의 결과는 오지 않는다 — 자식은 자기 시한으로 끝난다 · 대기 작업은 사라진다).

use crate::state::Daemon;
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::io::{BufRead, Write};
use std::sync::atomic::Ordering;
use std::sync::Arc;

/// 동시 진행 상한(전체).
pub const MAX_RUNNING: usize = 3;
/// 동시 진행 상한 중 master·CEO 밖 좌석 몫 — 한 칸은 늘 master·CEO 몫으로 남는다.
pub const MAX_RUNNING_GENERAL: usize = 2;
/// 대기열 상한(좌석당 하나라 실제로는 좌석 수 이하).
pub const MAX_PENDING: usize = 16;
/// 자식 강제 종료 시한(초) — cycle-agent 단일 전체 시한 570 + 60.
pub const KILL_AFTER_SECS: f64 = 630.0;
/// 자식이 첫 RPC 전에 기다리는 stdin 신호(데몬이 신원 위임을 등록한 뒤에 쓴다).
pub const GO_TOKEN: &str = "go";
/// 결과 큐 항목의 머리표·출처.
pub const RESULT_TAG: &str = "[cycle-result]";
pub const RESULT_FROM: &str = "cycle-agent --detach";
pub const RESULT_ORIGIN: &str = "cycle_result";
/// 결과 적재 상한 — 노드 보고와 같은 활성 큐 상한(`cys send --queued` 100).
const RESULT_QUEUE_CAP: usize = 100;
/// 결과 적재 재시도(동결·큐 가득) 간격·횟수 — 1시간.
const RESULT_RETRY_SECS: u64 = 30;
const RESULT_RETRY_MAX: u32 = 120;
/// 끝난 작업 기록 보관 수(재집행 차단·상태 조회).
const RECENT_KEEP: usize = 64;
/// 재집행 차단을 보는 기간(초) — 통보 시한(1200) + 잠정 보류(7200)보다 길게.
const REPEAT_WINDOW_SECS: f64 = 3.0 * 3600.0;

/// 접수된 작업의 내용(요청 좌석의 1콜이 정한 것 그대로).
#[derive(Clone, Debug, PartialEq)]
pub struct Spec {
    pub target: u64,
    pub target_role: Option<String>,
    /// 요청 좌석(신원 위임 · 결과 수신).
    pub requester: u64,
    pub fire: Option<String>,
    pub verifier: Option<String>,
    /// 검증자 좌석(접수 때 역할 해소 · 교차 판정 재료 — 자식은 다시 해소한다).
    pub verifier_sid: Option<u64>,
    pub timeout: u64,
    pub save_files: Vec<String>,
    pub resume_text: Option<String>,
    /// master·CEO 좌석(라우터 `CLEAR_SIGNAL_ROLES` 정확 일치).
    pub priority: bool,
    /// 차단점까지 여유 = C − 통보 퍼센트(작을수록 먼저 · 모르면 최대).
    pub headroom: i32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum JobState {
    Pending,
    /// pid 0 = 띄우는 중(자리 확보).
    Running { pid: u32, since: f64 },
}

#[derive(Clone, Debug)]
pub struct Job {
    pub id: u64,
    pub spec: Spec,
    pub state: JobState,
    pub accepted_at: f64,
}

/// 끝난 작업.
#[derive(Clone, Debug)]
pub struct Done {
    pub id: u64,
    pub target: u64,
    pub requester: u64,
    pub fire: Option<String>,
    /// None = 신호로 종료(시한 초과 강제 종료 포함) · 띄우지 못함.
    pub rc: Option<i32>,
    pub at: f64,
    pub secs: f64,
    pub summary: String,
    /// 결과 큐 적재 성공 여부(발행 재료).
    pub delivered: bool,
}

#[derive(Debug, Default)]
pub struct CycleJobs {
    next_id: u64,
    pub jobs: Vec<Job>,
    pub recent: VecDeque<Done>,
}

/// 접수 판정(순수 — 회귀 핀 대상).
#[derive(Clone, Debug, PartialEq)]
pub enum Admit {
    /// 대기열에 넣었다(곧 [`admit`] 이 시작 여부를 정한다).
    Accepted { job: u64 },
    /// 같은 좌석의 작업이 대기·진행 중이다(88).
    BusyJob { job: u64 },
    /// 같은 요청 좌석이 같은 통보를 이미 detach 로 집행했고 결과가 88 밖이었다(87 · 재집행 금지).
    Repeat { job: u64, rc: Option<i32> },
    /// 대기열이 가득 찼다.
    Full,
}

impl CycleJobs {
    /// 접수(순수) — 좌석당 하나 · 같은 통보의 자기 실패 뒤 재집행 금지 · 대기열 상한.
    pub fn submit(&mut self, spec: Spec, now: f64) -> Admit {
        if let Some(j) = self.jobs.iter().find(|j| j.spec.target == spec.target) {
            return Admit::BusyJob { job: j.id };
        }
        if let Some(f) = spec.fire.as_deref() {
            if let Some(d) = self.recent.iter().rev().find(|d| {
                d.target == spec.target
                    && d.requester == spec.requester
                    && d.fire.as_deref() == Some(f)
                    && now - d.at <= REPEAT_WINDOW_SECS
            }) {
                if d.rc != Some(88) {
                    return Admit::Repeat { job: d.id, rc: d.rc };
                }
            }
        }
        if self.jobs.iter().filter(|j| j.state == JobState::Pending).count() >= MAX_PENDING {
            return Admit::Full;
        }
        self.next_id += 1;
        let id = self.next_id;
        self.jobs.push(Job { id, spec, state: JobState::Pending, accepted_at: now });
        Admit::Accepted { job: id }
    }

    /// 대기 순번(1부터 · 시작 순서와 같은 정렬) — 진행 중이면 0.
    pub fn position(&self, job: u64) -> usize {
        let mut pend: Vec<&Job> = self.jobs.iter().filter(|j| j.state == JobState::Pending).collect();
        pend.sort_by(|a, b| order_key(a).partial_cmp(&order_key(b)).unwrap_or(std::cmp::Ordering::Equal));
        pend.iter().position(|j| j.id == job).map_or(0, |p| p + 1)
    }

    fn finish(&mut self, done: Done) {
        self.jobs.retain(|j| j.id != done.id);
        self.recent.push_back(done);
        while self.recent.len() > RECENT_KEEP {
            self.recent.pop_front();
        }
    }
}

/// 대기 작업 정렬 열쇠 — master·CEO 먼저 · 여유 작은 순 · 접수 순.
fn order_key(j: &Job) -> (u8, i32, f64, u64) {
    (u8::from(!j.spec.priority), j.spec.headroom, j.accepted_at, j.id)
}

/// 지금 시작할 대기 작업(시작 순서대로) — 순수. 동결 중이면 없다.
pub fn admit(jobs: &[Job], paused: bool) -> Vec<u64> {
    if paused {
        return vec![];
    }
    let running: Vec<&Job> = jobs.iter().filter(|j| matches!(j.state, JobState::Running { .. })).collect();
    let mut n = running.len();
    let mut general = running.iter().filter(|j| !j.spec.priority).count();
    let mut targets: Vec<u64> = running.iter().map(|j| j.spec.target).collect();
    let mut verifiers: Vec<u64> = running.iter().filter_map(|j| j.spec.verifier_sid).collect();
    let mut pend: Vec<&Job> = jobs.iter().filter(|j| j.state == JobState::Pending).collect();
    pend.sort_by(|a, b| order_key(a).partial_cmp(&order_key(b)).unwrap_or(std::cmp::Ordering::Equal));
    let mut out = vec![];
    for j in pend {
        if n >= MAX_RUNNING {
            break;
        }
        if !j.spec.priority && general >= MAX_RUNNING_GENERAL {
            continue;
        }
        // 검증 중인 좌석을 비우지 않는다.
        if verifiers.contains(&j.spec.target) {
            continue;
        }
        // master·CEO 밖 작업은 비워지는 좌석에 검증을 맡기지 않는다(master·CEO 는 기다리지 않는다).
        if !j.spec.priority && j.spec.verifier_sid.is_some_and(|v| targets.contains(&v)) {
            continue;
        }
        out.push(j.id);
        n += 1;
        if !j.spec.priority {
            general += 1;
        }
        targets.push(j.spec.target);
        if let Some(v) = j.spec.verifier_sid {
            verifiers.push(v);
        }
    }
    out
}

/// cycle-agent 종료코드의 뜻(결과 문구 · 발행 재료). 판정은 CSO 지침의 표가 한다.
pub fn rc_meaning(rc: Option<i32>) -> &'static str {
    match rc {
        Some(0) => "clear 실효 확인 · 재주입 완료",
        Some(80) => "clear 를 보냈으나 실효 미관측(재주입 0건 · 그 통보는 미해결)",
        Some(81) => "clear 실효 측정 불능(측정 불능은 통과가 아니다)",
        Some(82) => "검증자 충돌(송신 0건 · 다른 검증자로)",
        Some(83) => "검증자 미해소(송신 0건)",
        Some(84) => "대상이 유휴가 되지 않음(clear 송신 0건)",
        Some(85) => "사람 초안 보호(clear 송신 0건)",
        Some(86) => "clear 발효 · 재주입 보류(손으로 다시 clear 하지 마라)",
        Some(87) => "건너뜀 — 그 통보 뒤 사이클이 이미 끝났다(송신 0건)",
        Some(88) => "다른 집행자의 사이클이 진행 중(송신 0건 · 재배달을 기다린다)",
        Some(1) => "실패(저장 검증 실패·검증자 응답 없음 등 — clear 미실행 여부는 사유 참조)",
        Some(_) => "실패(사유 참조)",
        None => "시한 초과·신호로 종료 또는 띄우지 못함(사유 참조)",
    }
}

/// 결과 큐 항목 본문(요청 좌석이 읽는다 · 한 줄).
pub fn result_text(d: &Done, role: Option<&str>) -> String {
    let rc = d.rc.map_or_else(|| "-".to_string(), |r| r.to_string());
    let why = if d.summary.is_empty() { String::new() } else { format!(" · 사유: {}", d.summary) };
    format!(
        "{RESULT_TAG} surface:{}({}) fire={} rc={rc} — {} · job {} · {}초 · 요청 surface:{}{why}",
        d.target,
        role.unwrap_or("역할 없음"),
        d.fire.as_deref().unwrap_or("-"),
        rc_meaning(d.rc),
        d.id,
        d.secs.round() as u64,
        d.requester,
    )
}

/// 자식 stderr 에서 사유 1줄(마지막 `error:` 줄 · 없으면 마지막 줄) — 300자.
pub fn summarize_stderr(lines: &[String]) -> String {
    let pick = lines
        .iter()
        .rev()
        .find(|l| l.trim_start().starts_with("error:"))
        .or_else(|| lines.iter().rev().find(|l| !l.trim().is_empty()))
        .map(|l| l.trim().trim_start_matches("error:").trim().to_string())
        .unwrap_or_default();
    let mut s: String = pick.chars().take(300).collect();
    if pick.chars().count() > 300 {
        s.push('…');
    }
    s
}

/// 자식 명령줄 인자(순수 — 회귀 핀 대상). 대상은 접수 때 해소한 좌석 번호로 고정한다(역할 이동에도 그 통보의 좌석).
pub fn child_args(spec: &Spec, job: u64) -> Vec<String> {
    let mut a = vec!["cycle-agent".to_string(), "--surface".to_string(), cys::surface_ref(spec.target)];
    if let Some(v) = &spec.verifier {
        a.push("--verifier".into());
        a.push(v.clone());
    }
    a.push("--timeout".into());
    a.push(spec.timeout.to_string());
    if let Some(f) = &spec.fire {
        a.push("--fire".into());
        a.push(f.clone());
    }
    for f in &spec.save_files {
        a.push("--save-file".into());
        a.push(f.clone());
    }
    if let Some(r) = &spec.resume_text {
        a.push("--resume-text".into());
        a.push(r.clone());
    }
    a.push("--job".into());
    a.push(job.to_string());
    a
}

/// 상태 조회(`org.status` · 좌석 행) — 진행·대기·최근 끝난 작업.
pub fn status_json(daemon: &Daemon) -> Value {
    let j = daemon.cycle_jobs.lock().unwrap_or_else(|e| e.into_inner());
    let row = |x: &Job| {
        json!({"job": x.id, "surface_id": x.spec.target, "role": x.spec.target_role, "requester": x.spec.requester,
               "fire_id": x.spec.fire, "verifier": x.spec.verifier, "priority": x.spec.priority,
               "headroom": (x.spec.headroom != i32::MAX).then_some(x.spec.headroom),
               "state": match x.state { JobState::Pending => "pending", JobState::Running { .. } => "running" },
               "pid": match x.state { JobState::Running { pid, .. } if pid != 0 => Some(pid), _ => None },
               "since": match x.state { JobState::Running { since, .. } => Some(since), _ => None },
               "accepted_at": x.accepted_at})
    };
    json!({
        "max_running": MAX_RUNNING,
        "max_running_general": MAX_RUNNING_GENERAL,
        "running": j.jobs.iter().filter(|x| matches!(x.state, JobState::Running { .. })).map(row).collect::<Vec<_>>(),
        "pending": j.jobs.iter().filter(|x| x.state == JobState::Pending).map(row).collect::<Vec<_>>(),
        "recent": j.recent.iter().rev().take(8).map(|d| json!({"job": d.id, "surface_id": d.target, "requester": d.requester,
            "fire_id": d.fire, "rc": d.rc, "at": d.at, "secs": d.secs.round() as u64, "delivered": d.delivered,
            "summary": d.summary})).collect::<Vec<_>>(),
    })
}

/// 좌석 행(`ctx_guard.job`) — 그 좌석을 대상으로 한 진행·대기 작업.
pub fn seat_job(daemon: &Daemon, sid: u64) -> Value {
    let j = daemon.cycle_jobs.lock().unwrap_or_else(|e| e.into_inner());
    j.jobs.iter().find(|x| x.spec.target == sid).map_or(Value::Null, |x| {
        json!({"job": x.id, "requester": x.spec.requester, "fire_id": x.spec.fire,
               "state": match x.state { JobState::Pending => "pending", JobState::Running { .. } => "running" }})
    })
}

/// 접수(부른 쪽이 ACL·요청 좌석 해소·자기 clear·동결·통보 판정을 마쳤다) → 대기열 → 시작 판정. 반환: (판정, 시작 여부, 대기 순번).
pub fn submit(daemon: &Arc<Daemon>, spec: Spec) -> (Admit, bool, usize) {
    let now = crate::state::now_epoch();
    let a = daemon.cycle_jobs.lock().unwrap_or_else(|e| e.into_inner()).submit(spec, now);
    if let Admit::Accepted { job } = a {
        daemon.bus.publish("cycle.detach_accepted", "usage", None, json!({"job": job}));
        pump(daemon);
        let j = daemon.cycle_jobs.lock().unwrap_or_else(|e| e.into_inner());
        let running = j.jobs.iter().any(|x| x.id == job && matches!(x.state, JobState::Running { .. }));
        let done = !j.jobs.iter().any(|x| x.id == job);
        let pos = j.position(job);
        return (a, running || done, pos);
    }
    (a, false, 0)
}

/// 시작할 수 있는 대기 작업을 띄운다 — 접수 · 작업 끝 · 수집기 틱(2초 · 동결 해제 뒤)이 부른다. 자리는 락 안에서 잡고(pid 0) 띄우기는
/// 락 밖에서 한다(작업 락은 말단).
pub fn pump(daemon: &Arc<Daemon>) {
    let paused = daemon.paused.load(Ordering::Relaxed);
    let starts: Vec<Job> = {
        let mut j = daemon.cycle_jobs.lock().unwrap_or_else(|e| e.into_inner());
        let ids = admit(&j.jobs, paused);
        let now = crate::state::now_epoch();
        let mut v = vec![];
        for id in ids {
            if let Some(x) = j.jobs.iter_mut().find(|x| x.id == id) {
                x.state = JobState::Running { pid: 0, since: now };
                v.push(x.clone());
            }
        }
        v
    };
    for job in starts {
        start(daemon, job);
    }
}

fn start(daemon: &Arc<Daemon>, job: Job) {
    use cys::SpawnPolicy;
    let started = crate::state::now_epoch();
    let spawned = std::process::Command::new(crate::state::sibling_cli_path())
        .args(child_args(&job.spec, job.id))
        .env(cys::ENV_SOCKET, daemon.socket_path.to_string_lossy().as_ref())
        // 자식의 검증자 사전검사(호출자==검증자 · 82)가 요청 좌석을 호출자로 본다(CSO 가 손으로 부른 1콜과 같다).
        .env(cys::ENV_SURFACE_ID, job.spec.requester.to_string())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        // ★유계 자식(데몬이 끝까지 붙든다 · 분리 없음 — 윈도우는 창 숨김만) · 라이벌 데몬 autostart 봉인(폭주 ①).
        .spawn_policy(cys::ChildLifetime::Attached)
        .no_autostart()
        .spawn();
    let mut child = match spawned {
        Ok(c) => c,
        Err(e) => {
            finish(daemon, &job, None, started, format!("cycle-agent 를 띄우지 못했다: {e}"));
            return;
        }
    };
    let pid = child.id();
    // 신원 위임 — 자식이 첫 RPC 를 내기 전에(아직 stdin 신호 전) 등록하고 음성 캐시를 무효화한다.
    daemon.delegated_callers.lock().unwrap_or_else(|e| e.into_inner()).insert(pid, job.spec.requester);
    daemon.caller_gen.fetch_add(1, Ordering::Relaxed);
    {
        let mut j = daemon.cycle_jobs.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(x) = j.jobs.iter_mut().find(|x| x.id == job.id) {
            x.state = JobState::Running { pid, since: started };
        }
    }
    daemon.bus.publish(
        "cycle.detach_started",
        "usage",
        Some(job.spec.target),
        json!({"job": job.id, "pid": pid, "requester": job.spec.requester, "fire_id": job.spec.fire, "priority": job.spec.priority}),
    );
    if let Some(mut stdin) = child.stdin.take() {
        let _ = writeln!(stdin, "{GO_TOKEN}");
    }
    let d = daemon.clone();
    std::thread::spawn(move || monitor(d, job, child, started));
}

fn monitor(daemon: Arc<Daemon>, job: Job, mut child: std::process::Child, started: f64) {
    // stderr 는 끝까지 비운다(파이프가 차서 자식이 멈추지 않게) — 마지막 40줄만 남긴다.
    let reader = child.stderr.take().map(|err| {
        std::thread::spawn(move || {
            let mut tail: VecDeque<String> = VecDeque::new();
            for line in std::io::BufReader::new(err).lines().map_while(Result::ok) {
                tail.push_back(line);
                if tail.len() > 40 {
                    tail.pop_front();
                }
            }
            tail.into_iter().collect::<Vec<_>>()
        })
    });
    let mut killed = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) => {}
            Err(_) => break None,
        }
        if !killed && crate::state::now_epoch() - started > KILL_AFTER_SECS {
            let _ = child.kill();
            killed = true;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    };
    let status = status.or_else(|| child.wait().ok());
    daemon.delegated_callers.lock().unwrap_or_else(|e| e.into_inner()).remove(&child.id());
    let lines = reader.and_then(|h| h.join().ok()).unwrap_or_default();
    let mut summary = summarize_stderr(&lines);
    if killed {
        summary = format!("데몬이 {}초 시한으로 강제 종료했다 — {summary}", KILL_AFTER_SECS as u64);
    }
    finish(&daemon, &job, status.and_then(|s| s.code()), started, summary);
}

/// 작업 끝 — 기록 · 발행 · 다음 대기 작업 시작 · 결과 큐 적재(1회 · 동결·가득이면 재시도).
fn finish(daemon: &Arc<Daemon>, job: &Job, rc: Option<i32>, started: f64, summary: String) {
    let now = crate::state::now_epoch();
    let done = Done {
        id: job.id,
        target: job.spec.target,
        requester: job.spec.requester,
        fire: job.spec.fire.clone(),
        rc,
        at: now,
        secs: (now - started).max(0.0),
        summary,
        delivered: false,
    };
    daemon.cycle_jobs.lock().unwrap_or_else(|e| e.into_inner()).finish(done.clone());
    daemon.bus.publish(
        "cycle.detach_done",
        "usage",
        Some(job.spec.target),
        json!({"job": done.id, "requester": done.requester, "fire_id": done.fire, "rc": done.rc,
               "meaning": rc_meaning(done.rc), "secs": done.secs.round() as u64, "summary": done.summary}),
    );
    pump(daemon);
    let text = result_text(&done, job.spec.target_role.as_deref());
    let d = daemon.clone();
    let (id, requester) = (done.id, done.requester);
    std::thread::spawn(move || deliver(d, id, requester, text));
}

fn deliver(daemon: Arc<Daemon>, id: u64, requester: u64, text: String) {
    use crate::alert_route::{enqueue_into_seat, EnqueueErr, FreezeGuard};
    for attempt in 0..=RESULT_RETRY_MAX {
        match enqueue_into_seat(
            &daemon,
            requester,
            text.clone(),
            Some(RESULT_FROM.to_string()),
            RESULT_ORIGIN,
            RESULT_QUEUE_CAP,
            None,
            FreezeGuard::Daemon,
            None,
        ) {
            Ok(_) => {
                let mut j = daemon.cycle_jobs.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(d) = j.recent.iter_mut().find(|d| d.id == id) {
                    d.delivered = true;
                }
                return;
            }
            Err(EnqueueErr::Frozen) | Err(EnqueueErr::QueueFull) if attempt < RESULT_RETRY_MAX => {
                std::thread::sleep(std::time::Duration::from_secs(RESULT_RETRY_SECS));
            }
            Err(e) => {
                daemon.bus.publish(
                    "cycle.detach_result_undelivered",
                    "usage",
                    Some(requester),
                    json!({"job": id, "reason": format!("{e:?}"), "text": text}),
                );
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(target: u64, priority: bool, headroom: i32, verifier_sid: Option<u64>) -> Spec {
        Spec {
            target,
            target_role: Some(if priority { "master".into() } else { format!("worker-{target}") }),
            requester: 99,
            fire: Some(format!("1:{target}:1")),
            verifier: verifier_sid.map(|_| "worker".to_string()),
            verifier_sid,
            timeout: 120,
            save_files: vec![],
            resume_text: None,
            priority,
            headroom,
        }
    }

    fn running(jobs: &mut CycleJobs, ids: &[u64]) {
        for x in jobs.jobs.iter_mut().filter(|x| ids.contains(&x.id)) {
            x.state = JobState::Running { pid: 1000 + x.id as u32, since: 0.0 };
        }
    }

    /// V42R-1 ①(좌석 간 격리): 다른 좌석 1콜이 도는 동안 master 의 통보는 곧바로 시작한다 — 일반 좌석이 둘 진행 중이어도(상한 3 중
    /// 한 칸은 master·CEO 몫). 일반 좌석 셋째는 기다린다(일반 몫 2). 동결 중에는 아무것도 시작하지 않는다.
    #[test]
    fn master_is_never_queued_behind_other_seats_cycles() {
        let mut j = CycleJobs::default();
        let w1 = match j.submit(spec(11, false, 5, None), 0.0) { Admit::Accepted { job } => job, a => panic!("{a:?}") };
        let w2 = match j.submit(spec(12, false, 6, None), 1.0) { Admit::Accepted { job } => job, a => panic!("{a:?}") };
        assert_eq!(admit(&j.jobs, false), vec![w1, w2]);
        running(&mut j, &[w1, w2]);
        let w3 = match j.submit(spec(13, false, 1, None), 2.0) { Admit::Accepted { job } => job, a => panic!("{a:?}") };
        assert!(admit(&j.jobs, false).is_empty(), "일반 좌석 셋째가 master·CEO 몫을 썼다");
        let m = match j.submit(spec(10, true, 3, Some(20)), 3.0) { Admit::Accepted { job } => job, a => panic!("{a:?}") };
        assert_eq!(admit(&j.jobs, false), vec![m], "master 가 다른 좌석 1콜 뒤에서 기다린다(V42R-1)");
        assert!(admit(&j.jobs, true).is_empty(), "동결 중 시작");
        running(&mut j, &[m]);
        assert!(admit(&j.jobs, false).is_empty(), "상한 3 초과");
        // 끝나면 대기(w3) 가 시작된다.
        j.finish(Done { id: w1, target: 11, requester: 99, fire: None, rc: Some(0), at: 10.0, secs: 10.0, summary: String::new(), delivered: false });
        assert_eq!(admit(&j.jobs, false), vec![w3]);
    }

    /// 대기 순서: master·CEO 먼저 → 차단점까지 여유 작은 순 → 접수 순.
    #[test]
    fn pending_order_is_priority_then_headroom_then_arrival() {
        let mut j = CycleJobs::default();
        let a = match j.submit(spec(21, false, 3, None), 0.0) { Admit::Accepted { job } => job, x => panic!("{x:?}") };
        let b = match j.submit(spec(22, false, 1, None), 1.0) { Admit::Accepted { job } => job, x => panic!("{x:?}") };
        let c = match j.submit(spec(23, false, 1, None), 2.0) { Admit::Accepted { job } => job, x => panic!("{x:?}") };
        let m = match j.submit(spec(24, true, 9, None), 3.0) { Admit::Accepted { job } => job, x => panic!("{x:?}") };
        assert_eq!([j.position(m), j.position(b), j.position(c), j.position(a)], [1, 2, 3, 4]);
        assert_eq!(admit(&j.jobs, false), vec![m, b, c], "상한 3 · 일반 2");
    }

    /// 좌석당 단일 비행 · 자기 실패 뒤 같은 통보 재집행 금지(88 만 다시 받는다) · 대기열 상한.
    #[test]
    fn single_flight_repeat_guard_and_pending_cap() {
        let mut j = CycleJobs::default();
        let first = match j.submit(spec(31, false, 1, None), 0.0) { Admit::Accepted { job } => job, x => panic!("{x:?}") };
        assert_eq!(j.submit(spec(31, false, 1, None), 1.0), Admit::BusyJob { job: first });
        for (rc, blocked) in [(Some(1), true), (Some(84), true), (None, true), (Some(0), true), (Some(88), false)] {
            let mut k = CycleJobs::default();
            let id = match k.submit(spec(32, false, 1, None), 0.0) { Admit::Accepted { job } => job, x => panic!("{x:?}") };
            k.finish(Done { id, target: 32, requester: 99, fire: Some("1:32:1".into()), rc, at: 10.0, secs: 10.0, summary: String::new(), delivered: true });
            let again = k.submit(spec(32, false, 1, None), 20.0);
            assert_eq!(matches!(again, Admit::Repeat { .. }), blocked, "rc {rc:?}: {again:?}");
            // 다른 요청 좌석(다른 집행자)은 막지 않는다 · 다른 통보도 막지 않는다.
            let mut other = spec(32, false, 1, None);
            other.requester = 98;
            if blocked {
                assert!(matches!(k.submit(other, 21.0), Admit::Accepted { .. } | Admit::BusyJob { .. }));
            }
        }
        let mut k = CycleJobs::default();
        for t in 0..MAX_PENDING as u64 {
            assert!(matches!(k.submit(spec(100 + t, false, 1, None), 0.0), Admit::Accepted { .. }));
        }
        assert_eq!(k.submit(spec(999, false, 1, None), 0.0), Admit::Full);
    }

    /// 검증자 교차: 검증 중인 좌석을 대상으로 하는 작업은 기다린다 · 일반 작업은 비워지는 좌석에 검증을 맡기지 않는다 · master 는
    /// 기다리지 않는다.
    #[test]
    fn verifier_seat_is_not_cleared_mid_handshake() {
        let mut j = CycleJobs::default();
        let m = match j.submit(spec(40, true, 3, Some(41)), 0.0) { Admit::Accepted { job } => job, x => panic!("{x:?}") };
        running(&mut j, &[m]);
        let w = match j.submit(spec(41, false, 1, None), 1.0) { Admit::Accepted { job } => job, x => panic!("{x:?}") };
        assert!(admit(&j.jobs, false).is_empty(), "master 의 검증자 좌석을 검증 도중 비운다");
        j.finish(Done { id: m, target: 40, requester: 99, fire: None, rc: Some(0), at: 5.0, secs: 5.0, summary: String::new(), delivered: false });
        assert_eq!(admit(&j.jobs, false), vec![w]);
        running(&mut j, &[w]);
        // 검증자(41)가 비워지는 중 — 일반 작업은 기다리고 master 는 시작한다.
        let g = match j.submit(spec(42, false, 1, Some(41)), 6.0) { Admit::Accepted { job } => job, x => panic!("{x:?}") };
        let m2 = match j.submit(spec(40, true, 2, Some(41)), 7.0) { Admit::Accepted { job } => job, x => panic!("{x:?}") };
        assert_eq!(admit(&j.jobs, false), vec![m2], "일반 작업 {g} 이 비워지는 검증자에 기댔거나 master 가 기다렸다");
    }

    /// 자식 인자 — 대상 좌석 번호 고정 · 검증자 · 시한 · 통보 · 저장 파일 · 재개 문안 · 작업 번호. 검증 생략·임의 clear 명령은 싣지
    /// 않는다(`--force-no-verify`·`--clear-cmd` 는 detach 경로에 없다).
    #[test]
    fn child_args_pin() {
        let mut s = spec(7, true, 1, Some(8));
        s.save_files = vec!["/w/_round/SESSION_STATE.md".into()];
        s.resume_text = Some("재개".into());
        let a = child_args(&s, 5);
        assert_eq!(
            a,
            ["cycle-agent", "--surface", "surface:7", "--verifier", "worker", "--timeout", "120", "--fire", "1:7:1", "--save-file",
             "/w/_round/SESSION_STATE.md", "--resume-text", "재개", "--job", "5"]
                .map(String::from)
                .to_vec()
        );
        assert!(!a.iter().any(|x| x == "--force-no-verify" || x == "--clear-cmd" || x == "--detach"));
    }

    #[test]
    fn result_text_and_summary() {
        let d = Done { id: 3, target: 7, requester: 9, fire: Some("1:7:2".into()), rc: Some(1), at: 0.0, secs: 121.4, summary: summarize_stderr(&[
            "[cycle 1/7] 저장 지시 주입".into(), "error: 저장 검증 실패 — 60s 내 파일 갱신 없음. cycle 중단 (clear 미실행)".into()]), delivered: false };
        let t = result_text(&d, Some("master"));
        assert!(t.starts_with("[cycle-result] surface:7(master) fire=1:7:2 rc=1 — "), "{t}");
        assert!(t.contains("job 3 · 121초 · 요청 surface:9 · 사유: 저장 검증 실패"), "{t}");
        assert_eq!(GO_TOKEN, "go", "CLI(cys.rs cycle_job_go_ok)와 같은 값");
        assert_eq!(rc_meaning(Some(88)), "다른 집행자의 사이클이 진행 중(송신 0건 · 재배달을 기다린다)");
    }
}
