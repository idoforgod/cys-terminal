//! 자원 거버넌스 — 오너 3대 완화책의 1급 구현.
//! 프로세스 원장(ledger) + watchdog(loadavg·자식 수·중복 서버 감지) + idle 감지.
//! 핵심 기능: surface가 낳은 자식 프로세스 트리를 데몬이 직접 추적·강제 종료한다.

use crate::state::{now_epoch, Daemon};
use serde_json::json;
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use sysinfo::{Pid, ProcessesToUpdate, System};

const WATCHDOG_INTERVAL_SECS: u64 = 5;
const LOAD_DEBOUNCE_SECS: f64 = 60.0;

pub fn spawn_watchdog(daemon: Arc<Daemon>) {
    tokio::spawn(async move {
        let mut sys = System::new();
        let mut last_load_alert: f64 = 0.0;
        let mut last_dup_alert: HashMap<String, f64> = HashMap::new();
        let mut last_proc_alert: HashMap<u64, f64> = HashMap::new();
        let mut restart_counts: HashMap<u64, u32> = HashMap::new();
        let mut feed_reminded: HashMap<String, f64> = HashMap::new();
        let mut approval_debounce: HashMap<(u64, String), f64> = HashMap::new();
        let mut queue_depth_alerted: HashMap<u64, f64> = HashMap::new();
        let mut deadman_last_alert: f64 = 0.0;
        let mut alert_fired: HashMap<String, f64> = HashMap::new();
        // (learn gaps C12②) 재시작에도 디바운스 창 유지 — state 파일에서 복원.
        let mut learn_stuck_debounce: HashMap<u64, f64> =
            load_learn_stuck_debounce(&daemon.socket_path);
        let mut zombie_miss: HashMap<u64, u32> = HashMap::new();
        let mut launch_flag_warned: std::collections::HashSet<u64> =
            std::collections::HashSet::new();
        let mut feed_backlog_alerted: bool = false;
        let mut approval_stall_fired: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        let mut tick_no: u64 = 0;
        loop {
            tokio::time::sleep(Duration::from_secs(WATCHDOG_INTERVAL_SECS)).await;
            tick_no += 1;
            // 패닉 격리: 한 틱의 unwrap 패닉이 watchdog 태스크 전체를 죽여
            // 자원 거버넌스가 데몬 수명 내내 조용히 사라지는 것을 막는다.
            let tick = std::panic::AssertUnwindSafe(|| {
                sys.refresh_processes(ProcessesToUpdate::All, true);
                // ★SEAT: 프로세스 표를 갓 refresh 한 이 지점이 좌석 판정의 유일한 write 시점이다.
                // deliver_queued 보다 **먼저** 갱신해야 같은 틱의 배달이 최신 좌석 사실을 본다.
                refresh_seat_cache(&daemon, &sys);
                check_load(&daemon, &mut last_load_alert);
                check_surfaces(&daemon, &sys, &mut last_dup_alert, &mut last_proc_alert);
                check_idle(&daemon);
                // ★C3 틱 내 순서 고정: rehome → expire → deliver.
                // rehome이 먼저여야 WAL 복원분이 **정식 surface에 안착한 뒤** 만기 판정을 받는다
                // (restored_queue에 머무는 동안 만기시키면 주인 없는 항목을 이관하는 오귀속).
                // expire가 deliver보다 먼저여야 이미 만기인 항목을 굳이 배달하지 않는다.
                rehome_restored(&daemon);
                expire_queued(&daemon);
                deliver_queued(&daemon, &mut queue_depth_alerted);
                reap_orphan_ledger(&daemon, &sys);
                reap_exited_surfaces(&daemon);
                reap_zombie_surfaces(&daemon, &sys, &mut zombie_miss);
                check_agent_death(&daemon, &sys, &mut restart_counts);
                check_surface_crash(&daemon);
                check_feed_aging(&daemon, &mut feed_reminded);
                check_feed_backlog(&daemon, &mut feed_backlog_alerted);
                check_approval_stall(&daemon, &mut approval_stall_fired);
                check_master_deadman(&daemon, &mut deadman_last_alert);
                // 저빈도 검사(15초): 파일 stat·화면 렌더 — 5초마다 돌릴 필요 없음
                if tick_no.is_multiple_of(3) {
                    check_todo(&daemon);
                    check_approvals(&daemon, &mut approval_debounce);
                    check_launch_flags(&daemon, &sys, &mut launch_flag_warned);
                }
                // T7 E6 경보(30초): rate·주간예산·반복실패 — analytics SQL 동반이라 저빈도
                if tick_no.is_multiple_of(6) {
                    check_alerts(&daemon, &mut alert_fired);
                    // (RSI 학습 자율추천 i) 막힘 — 읽기전용으로 재시작 카운터를 보고 학습 추천만.
                    check_learn_stuck(&daemon, &restart_counts, &mut learn_stuck_debounce);
                }
                // 24/365 데몬 누수 차단: 위 검사들이 surface_id·cmdline 키로 insert만 하는
                // 태스크-로컬 디바운스/카운터 맵을 살아있는 surface 집합·나이로 솎아낸다.
                let live_surface_ids: std::collections::HashSet<u64> =
                    daemon.surfaces.lock().unwrap().keys().copied().collect();
                prune_watchdog_debounce_maps(
                    &mut last_dup_alert,
                    &mut last_proc_alert,
                    &mut restart_counts,
                    &mut approval_debounce,
                    &live_surface_ids,
                    now_epoch(),
                );
                queue_depth_alerted.retain(|sid, _| live_surface_ids.contains(sid));
                learn_stuck_debounce.retain(|sid, _| live_surface_ids.contains(sid));
                zombie_miss.retain(|sid, _| live_surface_ids.contains(sid));
                launch_flag_warned.retain(|sid| live_surface_ids.contains(sid));
            });
            if std::panic::catch_unwind(tick).is_err() {
                daemon.bus.publish(
                    "watchdog.tick_panic",
                    "watchdog",
                    None,
                    json!({"note": "watchdog tick panicked; continuing next tick"}),
                );
            }
        }
    });
}

fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// T5-2 무음 크래시 윈도우(초): "성공 ack 직후 N초 내 후행 실패 헬스룰" = 크래시.
fn crash_window_secs() -> f64 {
    std::env::var("CYS_CRASH_WINDOW_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10.0)
}

/// T5-2 무음 크래시 술어(순수함수 — 부작용0·테스트 핀 가능, 주입 clock/events).
/// "명령이 성공 ack를 보고했으나(last_ack_ts) 동일 surface에서 매칭 실패 헬스룰이 윈도우
/// `window` 초 내 발화" = 무음 크래시. 프로세스 종료(agent.exited)와 **구분** — 그건
/// check_agent_death가 이미 잡는다(이 술어는 프로세스 생존 여부를 보지 않는다).
///
/// 입력: `recent_health` = `{ts, surface_id, rule, line}` 시퀀스(읽기 전용·병렬 플래그 신설 0),
/// `last_ack`= 직전 성공 ack 시각(없으면 ack 부재 → false), `surface_id`, `window`.
/// 판정: ack 시각 T 직후 (T, T+window] 안에 같은 surface의 헬스 실패 엔트리가 존재하면 true.
fn surface_crashed(
    recent_health: &std::collections::VecDeque<serde_json::Value>,
    last_ack: Option<f64>,
    surface_id: u64,
    window: f64,
) -> bool {
    let Some(ack_ts) = last_ack else {
        return false; // 성공 ack가 없으면 "ack 후 후행 실패" 패턴 성립 불가
    };
    recent_health.iter().any(|h| {
        h["surface_id"].as_u64() == Some(surface_id) && {
            let ts = h["ts"].as_f64().unwrap_or(0.0);
            ts > ack_ts && ts <= ack_ts + window
        }
    })
}

/// T5-2 무음 크래시 알림 핸들러 재진입 가드(전역) — 알림 발화 경로가 자기 자신을 다시
/// 트리거(에러→알림→에러…)하는 무한루프를 차단한다(penpot errors.cljs `@handling-error?`
/// 계약의 클린룸 등가). 알림은 fire-and-forget 비동기(bus.publish는 이미 비동기)라 이 가드는
/// 한 watchdog 틱이 크래시 스캔 도중 재진입하지 않게만 보장한다.
static CRASH_HANDLER_ACTIVE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// T5-2 무음 크래시 감지 watchdog 검사: "ack 후 후행 실패"를 `surface_crashed` 술어로 판정하고,
/// 발화 시 NDJSON 이벤트 tail(~200)을 바이트상한(T4-5A) 적용해 첨부, surface별 swap 가드로
/// 1회만 알림한다. 프로세스 종료(check_agent_death)와 직교 — 생존 프로세스의 후행실패만.
fn check_surface_crash(daemon: &Arc<Daemon>) {
    // 핸들러 재진입 가드 — 이미 처리 중이면 이 틱은 건너뛴다(에러→알림→에러 루프 차단).
    if CRASH_HANDLER_ACTIVE.swap(true, Ordering::Acquire) {
        return;
    }
    let window = crash_window_secs();
    let surfaces: Vec<Arc<crate::state::Surface>> =
        daemon.surfaces.lock().unwrap().values().cloned().collect();
    for s in surfaces {
        // 프로세스가 이미 종료됐으면 check_agent_death 영역 — 무음 크래시 아님.
        if s.exited.load(Ordering::Relaxed) {
            // 회복(또는 종료 회수)된 surface는 재진입 가드 해제 — 다음 라이프사이클에 재발화 가능.
            s.crash_notified.store(false, Ordering::Relaxed);
            continue;
        }
        let last_ack = *s.last_cmd_ack.lock().unwrap();
        let crashed = {
            let recent = daemon.recent_health.lock().unwrap();
            surface_crashed(&recent, last_ack, s.id, window)
        };
        if !crashed {
            // 후행 실패 윈도우를 벗어나 정상화되면 가드 해제(다음 크래시에 재발화).
            s.crash_notified.store(false, Ordering::Relaxed);
            continue;
        }
        if s.crash_notified.swap(true, Ordering::Relaxed) {
            continue; // 이미 통지(1회성)
        }
        // 발화: NDJSON 이벤트 tail 첨부(바이트상한 T4-5A 적용 — 거대 페이로드 폭주 차단).
        let mut timeline = serde_json::Value::Array(daemon.bus.tail(200));
        if let Some(capped) = cys::wire::cap_response(&timeline) {
            timeline = capped; // cap 초과 시 fail-loud sentinel로 대체
        }
        let role = s.role.lock().unwrap().clone();
        // bus.publish는 이미 비동기(fire-and-forget) — 동기 재진입 publish 아님.
        daemon.bus.publish(
            "surface.crashed",
            "surface",
            Some(s.id),
            json!({"surface_ref": cys::surface_ref(s.id), "role": role,
                   "severity": crate::severity::Severity::Recoverable.as_str(),
                   "window_secs": window, "timeline": timeline}),
        );
    }
    CRASH_HANDLER_ACTIVE.store(false, Ordering::Release);
}

/// T4-5B 좀비 하트비트 임계: 연속 N회 ping 미스 시 좀비 surface로 판정·강제정리.
const ZOMBIE_MISS_THRESHOLD: u32 = 3;

/// T4-5B 좀비 판정 단일 술어(순수함수 — 테스트 핀): 연속 미스 카운트가 임계 이상이면 좀비.
fn zombie_over_threshold(missed: u32) -> bool {
    missed >= ZOMBIE_MISS_THRESHOLD
}

/// T4-5B 좀비 surface 정리: per-surface-connection 하트비트를 일반화한다. surface의 자식
/// 프로세스가 사라졌는데 `exited` 플래그가 서지 않은(half-open/좀비) 상태가 watchdog 틱마다
/// 한 번씩 "ping 미스"로 누적되고, 연속 `ZOMBIE_MISS_THRESHOLD`(3)회 미스면 좀비로 확정해
/// 강제 정리(close_surface) + 원장 제거한다. 기존 reap_* sweep 패턴 위에 쌓는다.
/// 한 번이라도 살아있는 신호(자식 생존)가 보이면 미스 카운트 리셋(half-open만 누적).
fn reap_zombie_surfaces(
    daemon: &Arc<Daemon>,
    sys: &System,
    zombie_miss: &mut HashMap<u64, u32>,
) {
    let mut to_cleanup: Vec<u64> = Vec::new();
    {
        let surfaces: Vec<Arc<crate::state::Surface>> =
            daemon.surfaces.lock().unwrap().values().cloned().collect();
        for s in surfaces {
            // 정상 종료(exited)는 reap_exited_surfaces 영역 — 좀비 아님, 카운터 청소.
            if s.exited.load(Ordering::Relaxed) {
                zombie_miss.remove(&s.id);
                continue;
            }
            // 하트비트 = surface의 셸 프로세스(pid) 생존. 살아있으면 미스 리셋.
            let alive = sys.process(Pid::from_u32(s.pid)).is_some();
            if alive {
                zombie_miss.remove(&s.id);
                continue;
            }
            // half-open: 프로세스는 사라졌는데 exited 플래그 미설정 → ping 미스 누적.
            let missed = zombie_miss.entry(s.id).or_insert(0);
            *missed += 1;
            if zombie_over_threshold(*missed) {
                to_cleanup.push(s.id);
            }
        }
    }
    for id in to_cleanup {
        zombie_miss.remove(&id);
        // 강제 정리: close_surface가 surface 등록 해제(이미 죽은 자식엔 kill/wait 무시).
        if close_surface(daemon, id, CloseCause::Reap).is_ok() {
            // 원장 제거: 이 surface가 소유한 스코프 항목을 원장에서 제거(좀비 잔존 차단).
            {
                let mut ledger = daemon.ledger.lock().unwrap();
                ledger.retain(|_, e| e.surface_id != Some(id));
            }
            daemon.bus.publish(
                "surface.zombie_reaped",
                "surface",
                Some(id),
                json!({"surface_ref": cys::surface_ref(id),
                       "reason": "heartbeat_missed", "missed": ZOMBIE_MISS_THRESHOLD}),
            );
        }
    }
}

/// T5-6 strand-2: 한 surface가 소유한 원장 항목(들)을 Poisoned로 마킹 — 비정상 종료한
/// 자식을 재사용 풀에서 영구 배제한다(watchdog 보강). 마킹만 수행(회수는 기존 reaper의
/// 단일 소유 — 같은 pid를 이중 처리하지 않는다). 마킹된 항목이 없으면 무해한 no-op.
fn poison_surface_ledger(daemon: &Arc<Daemon>, surface_id: u64) {
    let mut ledger = daemon.ledger.lock().unwrap();
    for entry in ledger.values_mut() {
        if entry.surface_id == Some(surface_id) {
            entry.health = crate::state::ProcessHealth::Poisoned;
        }
    }
}

/// T2-5 에이전트 사망 감지: 셸은 살았는데 그 위의 에이전트 프로세스만 죽은 상태를
/// 즉시 잡는다 (기존엔 pane.idle 300초가 최초 신호 — '생각 중'과 구분 불가).
/// 판정: 자식 트리에서 agents.json 등록 바이너리가 '한 번 보였다가 사라짐' 전이.
fn check_agent_death(
    daemon: &Arc<Daemon>,
    sys: &System,
    restart_counts: &mut HashMap<u64, u32>,
) {
    let auto_restart = std::env::var("CYS_AGENT_AUTORESTART")
        .map(|v| v == "1")
        .unwrap_or(false);
    let surfaces: Vec<Arc<crate::state::Surface>> =
        daemon.surfaces.lock().unwrap().values().cloned().collect();
    let now = now_epoch();
    for s in surfaces {
        if s.exited.load(Ordering::Relaxed) {
            continue;
        }
        let Some((agent, bin)) = s.agent_meta.lock().unwrap().clone() else {
            continue;
        };
        let bin_base = bin.rsplit(['/', '\\']).next().unwrap_or(&bin).to_string();
        let descendants = collect_descendants(sys, s.pid);
        let alive = descendants
            .iter()
            .any(|(_, cmdline)| cmdline_matches_agent(cmdline, &bin_base));
        if alive {
            s.agent_seen.store(true, Ordering::Relaxed);
            if s.agent_exit_notified.swap(false, Ordering::Relaxed) {
                // 재기동 성공 — 카운터 유지(수명 내 상한 3회), 복귀 이벤트
                daemon.bus.publish(
                    "agent.recovered",
                    "surface",
                    Some(s.id),
                    json!({"agent": agent, "surface_ref": cys::surface_ref(s.id)}),
                );
            }
            continue;
        }
        if !s.agent_seen.load(Ordering::Relaxed) {
            continue; // 아직 기동 전 (launch-agent 진행 중)
        }
        if s.agent_exit_notified.swap(true, Ordering::Relaxed) {
            continue; // 이미 통지
        }
        let role = s.role.lock().unwrap().clone();
        daemon.bus.publish(
            "agent.exited",
            "surface",
            Some(s.id),
            json!({"agent": agent, "role": role, "surface_ref": cys::surface_ref(s.id),
                   "severity": crate::severity::Severity::Recoverable.as_str(),
                   "restart_count": restart_counts.get(&s.id).copied().unwrap_or(0)}),
        );
        if !auto_restart {
            continue;
        }
        // 401·로그인 만료로 죽은 에이전트의 무한 재기동 루프 차단
        let auth_rules = ["not_logged_in", "auth_401", "token_expired", "login_required"];
        let auth_blocked = daemon.recent_health.lock().unwrap().iter().any(|h| {
            h["surface_id"].as_u64() == Some(s.id)
                && auth_rules.contains(&h["rule"].as_str().unwrap_or(""))
                && now - h["ts"].as_f64().unwrap_or(0.0) < 300.0
        });
        if auth_blocked {
            // T5-6 strand-2: auth 차단(401·로그인 만료)으로 죽은 자식은 재기동도 막혔으니
            // 재사용 풀에서도 배제 — 오염 격리.
            poison_surface_ledger(daemon, s.id);
            daemon.bus.publish(
                "agent.restart_blocked",
                "surface",
                Some(s.id),
                json!({"agent": agent, "reason": "recent auth alert (fix login first)"}),
            );
            continue;
        }
        let count = restart_counts.entry(s.id).or_insert(0);
        if *count >= 3 {
            // T5-6 strand-2: 3회 재기동 소진 = 비정상 종료 확정 → Poisoned 마킹(재사용 금지).
            poison_surface_ledger(daemon, s.id);
            daemon.bus.publish(
                "agent.exit_unrecoverable",
                "surface",
                Some(s.id),
                json!({"agent": agent, "role": role,
                       "severity": crate::severity::Severity::Critical.as_str(),
                       "note": "3 auto-restarts exhausted — master 판단 필요"}),
            );
            continue;
        }
        *count += 1;
        let sid = s.id;
        let attempts = *count;
        tokio::spawn(async move {
            use crate::state::HideConsole;
            let cli = crate::state::sibling_cli_path();
            let _ = tokio::time::timeout(
                Duration::from_secs(180),
                tokio::process::Command::new(cli)
                    .arg("node-recover")
                    .arg("--surface")
                    .arg(cys::surface_ref(sid))
                    .hide_console()
                    .output(),
            )
            .await;
            let _ = attempts;
        });
    }
}

/// (RSI 학습 자율추천 i · 순수 판정) 재시작 카운트가 임계 이상이고 디바운스 쿨다운이 지난
/// surface id — '동일 노드 N회 실패 = 막힘' 신호를 결정론으로 추출한다(테스트 핀).
fn learn_stuck_candidates(
    restart_counts: &HashMap<u64, u32>,
    debounce: &HashMap<u64, f64>,
    threshold: u32,
    cooldown: f64,
    now: f64,
) -> Vec<u64> {
    if threshold == 0 {
        return Vec::new();
    }
    let mut out: Vec<u64> = restart_counts
        .iter()
        .filter(|(_, c)| **c >= threshold)
        // 디바운스 기록 부재 = 한 번도 추천 안 됨 = 즉시 적격. 기록 있으면 쿨다운 경과 후만.
        .filter(|(sid, _)| match debounce.get(sid) {
            None => true,
            Some(&last) => now - last >= cooldown,
        })
        .map(|(sid, _)| *sid)
        .collect();
    out.sort_unstable();
    out
}

/// (RSI 학습 자율추천 i·learn gaps C12②) stuck 디바운스 지속화 파일명 — 데몬 state
/// 디렉터리(소켓 동거·부서별 격리) 하위. 직렬화: {"<surface_id>": <last_propose_epoch>}.
const LEARN_STUCK_DEBOUNCE_FILE: &str = "learn_stuck_debounce.json";

/// 디바운스 맵 로드 — 데몬 재시작 시 인메모리 디바운스 소실로 CYS_RSI_STUCK_DEBOUNCE_SECS
/// (기본 3600) 창이 리셋돼 동일 노드 추천이 중복 발화하던 문제 수리: spawn_watchdog가 부트 시
/// 1회 읽어 창을 이어간다. 부재/손상=빈 맵(fail-open — 최악은 추천 1회 중복일 뿐, 차단이 더 해롭다).
fn load_learn_stuck_debounce(socket_path: &std::path::Path) -> HashMap<u64, f64> {
    let path = crate::state::state_dir(socket_path).join(LEARN_STUCK_DEBOUNCE_FILE);
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.as_object().cloned())
        .map(|o| {
            o.iter()
                .filter_map(|(k, v)| Some((k.parse::<u64>().ok()?, v.as_f64()?)))
                .collect()
        })
        .unwrap_or_default()
}

/// 디바운스 맵 저장(원자) — check_learn_stuck가 추천 발화로 타임스탬프를 갱신한 직후 호출.
/// 죽은 surface 항목은 watchdog retain이 인메모리에서 솎아내고 다음 발화 시 파일에도 반영된다.
fn save_learn_stuck_debounce(socket_path: &std::path::Path, debounce: &HashMap<u64, f64>) {
    let obj: serde_json::Map<String, serde_json::Value> = debounce
        .iter()
        .map(|(k, v)| (k.to_string(), json!(v)))
        .collect();
    let dir = crate::state::state_dir(socket_path);
    let _ = write_json_atomic(
        &dir,
        LEARN_STUCK_DEBOUNCE_FILE,
        &serde_json::Value::Object(obj).to_string(),
    );
}

/// (RSI 학습 자율추천 i) 막힘 트리거 — ★읽기 전용: watchdog의 기존 재시작 카운터(동일 노드
/// N회 실패=막힘 신호)만 읽어 학습 추천 feed 항목을 만든다. autopilot(EFEC/AMI) 자율주행
/// 로직은 무손상·자동응답 0 — 추천까지만 자율, 착수는 사람 승인(directive §4). 디바운스로 스팸 차단.
fn check_learn_stuck(
    daemon: &Arc<Daemon>,
    restart_counts: &HashMap<u64, u32>,
    debounce: &mut HashMap<u64, f64>,
) {
    let threshold = env_u64("CYS_RSI_STUCK_RESTARTS", 3) as u32;
    let cooldown = env_u64("CYS_RSI_STUCK_DEBOUNCE_SECS", 3600) as f64;
    let now = now_epoch();
    let cands = learn_stuck_candidates(restart_counts, debounce, threshold, cooldown, now);
    if cands.is_empty() {
        return;
    }
    // role은 읽기 전용으로 조회(surfaces 락을 짧게 잡고 해제) — feed 생성은 락 밖에서.
    let roles: Vec<(u64, String)> = {
        let surfaces = daemon.surfaces.lock().unwrap();
        cands
            .iter()
            .map(|sid| {
                let role = surfaces
                    .get(sid)
                    .and_then(|s| s.role.lock().unwrap().clone())
                    .unwrap_or_else(|| "node".into());
                (*sid, role)
            })
            .collect()
    };
    for (sid, role) in roles {
        debounce.insert(sid, now);
        let body = format!(
            "{{\"event\":\"propose\",\"reason\":\"stuck\",\"topic\":\"{role} 막힘 돌파 방법론\",\"status\":\"awaiting_approval\",\"trigger\":\"watchdog restart>={threshold}\"}}\n\
             동일 노드 {threshold}회+ 재시작(막힘) 감지. 'cys learn \"{role} 막힘 돌파\"'로 학습 착수(사람 승인). directive §4: 추천까지만 자율."
        );
        daemon.push_feed_notification(
            "learn_proposal",
            &format!("[RSI 학습 추천] 막힘 — {role} 재시작 {threshold}회+"),
            &body,
            Some(sid),
        );
    }
    // (learn gaps C12②) 발화 직후 지속화 — 재시작이 디바운스 창을 리셋하지 않게.
    save_learn_stuck_debounce(&daemon.socket_path, debounce);
}

/// T3-12 승인 aging 재알림: pending feed가 무음 적체되지 않게 N분마다 재push.
fn check_feed_aging(daemon: &Arc<Daemon>, reminded: &mut HashMap<String, f64>) {
    let remind_secs = env_u64("CYS_FEED_REMIND_SECS", 300);
    if remind_secs == 0 {
        return;
    }
    let now = now_epoch();
    // (request_id, title, created_at, tier, body) — tier·body는 승인 미러 재조정에 필요(§2.4·O9).
    let pending: Vec<(String, String, f64, Option<String>, String)> = {
        let items = daemon.feed_items.lock().unwrap();
        items
            .iter()
            .filter(|i| i.status == "pending")
            .map(|i| (i.request_id.clone(), i.title.clone(), i.created_at, i.tier.clone(), i.body.clone()))
            .collect()
    }; // ★feed_items 락은 여기서 해제 — 아래 mirror_approval(channels 락)이 lock-order 안전.
    let pending_ids: std::collections::HashSet<&String> =
        pending.iter().map(|(id, _, _, _, _)| id).collect();
    reminded.retain(|id, _| pending_ids.contains(id));
    let total = pending.len();
    for (request_id, title, created_at, tier, body) in &pending {
        let age = now - created_at;
        if age < remind_secs as f64 {
            continue;
        }
        let last = reminded.get(request_id).copied().unwrap_or(*created_at);
        if now - last < remind_secs as f64 {
            continue;
        }
        reminded.insert(request_id.clone(), now);
        daemon.bus.publish(
            "feed.item.aging",
            "feed",
            None,
            json!({"request_id": request_id, "title": title,
                   "age_secs": age as u64, "pending_total": total}),
        );
        // §2.4·§2.6 O9: aging 재알림은 채널측 자체 재발행이 아니라 feed aging에 일원화한다. mirror_approval은
        // 멱등(기존 버튼 있으면 skip)이라 중복 버튼 0을 유지하되, 채널이 push 이후 등록된 경우 늦은 미러를
        // 발행한다. tier≤C·게이트 ON이 아니면 내부에서 fail-closed로 무발행.
        crate::channels::mirror_approval(daemon, request_id, title, body, tier.as_deref());
    }
}

/// T2-8 master dead-man: 조직의 단일 장애점인 master 자신의 사망·장기 무출력 감시.
fn check_master_deadman(daemon: &Arc<Daemon>, last_alert: &mut f64) {
    let secs = env_u64("CYS_MASTER_DEADMAN_SECS", 900);
    if secs == 0 {
        return;
    }
    let Some(sid) = daemon.roles.lock().unwrap().get("master").copied() else {
        return; // master 역할 미등록 — 데몬 단독 가동 등 정상 상황
    };
    let now = now_epoch();
    if now - *last_alert < 300.0 {
        return; // 5분 디바운스
    }
    let problem = match daemon.get_surface(sid) {
        None => Some(json!({"reason": "master surface gone"})),
        Some(s) if s.exited.load(Ordering::Relaxed) => {
            Some(json!({"reason": "master surface exited"}))
        }
        Some(s) => {
            let idle = s.last_output.lock().unwrap().elapsed().as_secs();
            if idle >= secs {
                Some(json!({"reason": "master silent", "idle_secs": idle}))
            } else {
                None
            }
        }
    };
    if let Some(payload) = problem {
        *last_alert = now;
        daemon
            .bus
            .publish("master.deadman", "alert", Some(sid), payload);
    }
}

/// T7 E6 경보: rate 한도·주간 예산·반복실패를 순수 평가기(alerts.rs)로 판정해 **에지 발화**한다.
/// fired 맵에 없는 키만 발행(첫 교차)하고, 해소된 키는 retain으로 제거해 재무장한다(다음 교차 시
/// 재발화). 지속 조건은 30분 디바운스로 재격상(master가 놓치지 않게). ★자동응답 금지 — 이벤트만.
fn check_alerts(daemon: &Arc<Daemon>, fired: &mut HashMap<String, f64>) {
    const REMIND_SECS: f64 = 1800.0;
    let cfg = crate::alerts::AlertConfig::load();
    let now = now_epoch();
    let snap = crate::alerts::snapshot(daemon, now);
    let active = crate::alerts::evaluate(&snap, &cfg);
    let active_keys: std::collections::HashSet<String> =
        active.iter().map(|a| a.key.clone()).collect();
    for a in &active {
        let due = fired.get(&a.key).is_none_or(|t| now - *t >= REMIND_SECS);
        if due {
            fired.insert(a.key.clone(), now);
            // 기존 wire("warn"|"crit") 보존 + 단일 술어 파생 severity_class 추가(additive·외과적).
            let sev = a.severity_enum();
            let mut payload = a.to_value();
            payload["severity_class"] = json!(sev.as_str());
            payload["isolate"] = json!(sev.is_critical());
            daemon
                .bus
                .publish(&format!("alert.{}", a.kind), "alert", None, payload);
        }
    }
    // 해소된 경보 키 재무장(다음 교차 시 즉시 발화) — 태스크-로컬 맵 누수도 차단.
    fired.retain(|k, _| active_keys.contains(k));
}

// CYS_TODO_DIRS 분해·스캔 루트 조립·파일 발견은 전부 **lib 계층 단일 구현**이다
// (`cys::todo_scan`). 여기 재구현을 두면 파리티 하네스가 검증하는 규칙과 데몬이 실제로 쓰는
// 규칙이 갈린다 — 그 갈림이 정확히 S18이었다(정책은 같은데 보는 파일 집합이 달랐다).

// ─────────────────────────────────────────────────────────────────────────────
// ★락 순서 규약 (todo 계열 · 2026-07-26 명문화)
//
//   **`todo_progress` → `todo_verdict`.** 역순 획득 금지.
//
// 근거는 실측이다: `handlers.rs`의 `org.status` 조립이 `todo_progress` 가드를 **잡은 채**
// `todo_verdict`를 획득한다(TP→TV 중첩). 여기 워치독이 TV를 잡은 채 TP를 잡으면 두 스레드가
// 서로의 가드를 기다려 **즉시 데드락**이고, 데드락은 워치독을 죽여 자원 거버넌스를 데몬
// 수명 내내 침묵시킨다 — 아래 poison 내성이 막으려는 것과 정확히 같은 종류의 사고다.
//
// 현행 `check_todo`는 두 맵을 **중첩 없이** 각각 임시 가드로만 잡으므로 규약을 만족한다
// (모든 획득이 한 문장 안에서 끝나 가드가 즉시 소멸한다). 이 파일에 TV 가드를 변수로 묶는
// 코드를 넣게 되면 그 스코프 안에서 TP를 만지지 마라 — 필요하면 TP를 **먼저** 잡아라.
// ─────────────────────────────────────────────────────────────────────────────

/// 판정 캐시 잠금 — 워치독 틱 경로라 poisoning에도 살아남아야 한다(패닉으로 워치독을 죽이면
/// 자원 거버넌스가 데몬 수명 내내 조용히 사라진다 · 틱 패닉 격리와 같은 정신).
fn todo_verdict_map(
    daemon: &Arc<Daemon>,
) -> std::sync::MutexGuard<'_, HashMap<String, (f64, &'static str, Option<String>)>> {
    daemon
        .todo_verdict
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// 진행률 맵 잠금 — `todo_verdict_map`과 **대칭**으로 poison 내성이어야 한다.
///
/// 종전에는 같은 함수 안에서 판정 캐시만 poison 내성이고 진행률 맵은 `.unwrap()`이었다.
/// 주석이 "패닉으로 워치독을 죽이면 자원 거버넌스가 데몬 수명 내내 사라진다"고 적어 놓고
/// 절반만 이행된 상태다 — 다른 스레드가 진행률 맵을 잡은 채 패닉하면 그 순간부터 워치독
/// 틱 전체가 매번 죽는다. 방어의 비대칭은 방어가 아니다.
fn todo_progress_map(
    daemon: &Arc<Daemon>,
) -> std::sync::MutexGuard<'_, HashMap<String, (u64, u64, f64)>> {
    daemon
        .todo_progress
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// 진행률 맵(`todo_progress`)에 등재할 판정인가.
///
/// - `retired`/`foreign-scope` = **등재도 이벤트 발행도 하지 않는다.** 종결된 레인의 유산
///   파일과 남의 팩 파일이 이 경로로 `org.status`·HUD·Control Center까지 흘러들어간 것이
///   07-26 유령 집계 사고(dept-2 306항목 중 301항목이 유령)의 데몬 측 통로였다.
/// - `unclaimed`/`orphan-scope` = **등재한다.** 판정 불능을 '없음'으로 처리하면 죽은 워커의
///   미완 작업이 은폐되고 게이트가 false QUIET에 빠진다(ADR-3 fail-open) — 숨기지 말고
///   구분 플래그를 달아 시끄럽게 보고한다.
fn todo_is_countable(verdict: cys::todo_decl::Verdict) -> bool {
    !matches!(
        verdict,
        cys::todo_decl::Verdict::Retired | cys::todo_decl::Verdict::ForeignScope
    )
}

/// T3-9 todo 파일 워치: 각 surface cwd의 `_round/*_TODO.md` + CYS_TODO_DIRS 추가 루트.
/// 변경 감지 시 todo.updated 이벤트 + org.status 집계 갱신 (push 규약을 기계 보증으로).
///
/// ★C2 선언 기반 판정(Declared State · 설계 §4-5): 어떤 파일을 집계할지는 파일명·경로·mtime이
/// 아니라 **파일 안의 선언 한 줄**이 정한다(ADR-1). 여기 방어가 없어 종결 레인의 유산 todo가
/// `daemon.todo_progress` → `org.status` → HUD까지 유입됐다 — Python 보고기만 고치는 것은
/// 절반만 덮는 것이었다.
fn check_todo(daemon: &Arc<Daemon>) {
    // 팩 정체성 조회는 **틱당 1회**. 파일마다 부르면 워치독 틱에 stat이 순증한다.
    // 판정 입력을 인자로 뽑아 두면 테스트가 라이브 팩(CYS_PACK_DIR)을 건드리지 않고
    // 5분기 전부를 결정론으로 재현할 수 있다.
    //
    // ★S18 교정 — **정본 위치 `pack/round`를 스캔 루트에 넣는다**(같은 이유로 팩 경로도 틱당
    // 1회만 조회한다). 이것이 없어서 데몬은 정본 todo를 한 번도 보지 않았고, 이번 브랜치가
    // 데몬에 배선한 선언 판정·verdict/owner payload·유령 배제가 **정본 파일에는 전혀 적용되지
    // 않았다**. 팩 경로를 인자로 뽑는 이유도 판정 입력과 같다 — 테스트가 라이브 팩을 만지지
    // 않으면서 루트 구성 규칙을 결정론으로 재현할 수 있어야 한다.
    let pack = cys::pack::pack_dir();
    check_todo_with(daemon, &cys::pack::scope_id(), &|s| {
        cys::pack::scope_exists(s)
    }, Some(pack.as_path()))
}

fn check_todo_with(
    daemon: &Arc<Daemon>,
    my_scope: &str,
    scope_exists: &dyn Fn(&str) -> bool,
    pack_dir: Option<&std::path::Path>,
) {
    // 스캔 루트·파일 발견 규칙은 **lib 계층 단일 구현**이다(`cys::todo_scan`) — Python 소비자
    // C1과 같은 집합을 보는지 `parity_todo_scan.py`가 같은 임시 트리로 기계 대조한다.
    let cwds: Vec<String> = daemon
        .surfaces
        .lock()
        .unwrap()
        .values()
        .filter(|s| !s.exited.load(Ordering::Relaxed))
        .map(|s| s.cwd.clone())
        .collect();
    let roots = cys::todo_scan::scan_roots(
        pack_dir,
        &cwds,
        std::env::var("CYS_TODO_DIRS").ok().as_deref(),
    );
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    {
        for path in cys::todo_scan::discover(&roots) {
            let Ok(meta) = std::fs::metadata(&path) else {
                continue;
            };
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs_f64())
                .unwrap_or(0.0);
            let key = path.to_string_lossy().into_owned();
            seen.insert(key.clone());
            // ★성능 계약(§4-5): skip 기준은 진행률 맵이 아니라 **판정 캐시**다. 배제 판정 파일은
            // 진행률 맵에 없으므로, 옛 기준을 그대로 두면 유산 파일이 매 틱 재파싱된다.
            // 캐시 히트(= mtime 무변화)면 파일을 열지 않는다 — 읽기 I/O 순증 0.
            let prev = todo_verdict_map(daemon).get(&key).map(|(m, v, _)| (*m, *v));
            if prev.map(|(m, _)| m) == Some(mtime) {
                continue;
            }
            // 변경됨 — 체크박스 집계 (64KB 상한: 거대 파일이 watchdog 틱을 잡아먹지 않게)
            //
            // ★비UTF-8 정합(2026-07-26): 종전 `read_to_string`은 비UTF-8 바이트 하나에
            // `continue`로 빠져 **등재도 캐시 갱신도 0**이었다 — 그 파일은 매 틱 재파싱되면서
            // 영원히 집계에서 사라진다. 반면 Python 소비자(`javis_report.read_head`·
            // `count_checkboxes`)는 `errors="replace"`로 lossy 디코드해 **집계한다**.
            // 같은 파일에 대해 데몬은 "없음", 팩은 "있음"이라고 말하는 조용한 갈림이었고,
            // 조용한 차이가 가장 나쁘다(2언어 파리티 K1). 여기를 lossy로 맞춘다 —
            // `from_utf8_lossy`의 U+FFFD 치환은 Python의 `errors="replace"`와 동형이며,
            // 체크박스·선언 토큰은 ASCII라 치환이 판정에 영향을 주지 않는다.
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            let content: String = String::from_utf8_lossy(&bytes)
                .chars()
                .take(65536)
                .collect();
            // G3: 선언 파싱 예산은 **원시 바이트** 선두 1 KiB뿐이고, 그 절단은
            // `head_from_bytes`가 유일하게 수행한다(계약 정본).
            //
            // ★W14 S15 교정 — 종전에는 여기서 `content.get(..HEAD_BYTES)`로 **디코드된 문자열**을
            // 잘라 자체 재구현했다. 그 결과 프로덕션 데몬은 계약 정본을 한 번도 통과하지 않았고
            // (`head_from_bytes`의 유일한 호출자가 파리티 테스트 덤퍼였다) **하네스가 검증하는
            // 읽기 경로 ≠ 프로덕션 읽기 경로**였다. 비UTF-8 파일에서 lossy 디코드가 1바이트를
            // 3바이트로 팽창시키므로 두 경로의 절단 지점이 갈리고, 은퇴 선언이 팽창 뒤에 있으면
            // **은퇴한 파일을 데몬만 계속 집계**한다(유령 재발). 재구현하지 말고 여기를 지나라.
            let head = cys::todo_decl::head_from_bytes(&bytes);
            let decl = cys::todo_decl::parse(&head).ok();
            let verdict = cys::todo_decl::classify(decl.as_ref(), my_scope, scope_exists);
            // ★S16 — 선언 owner를 판정 캐시에 함께 보관한다(그래야 `org.status` 조립이 파일명
            // 추론 없이 라벨을 낼 수 있다). 센티널 `"?"`·빈 값은 "모른다"이므로 싣지 않는다.
            let owner = decl
                .as_ref()
                .map(|d| d.owner.as_str())
                .filter(|o| !o.is_empty() && *o != "?")
                .map(|o| o.to_string());
            todo_verdict_map(daemon).insert(key.clone(), (mtime, verdict.as_str(), owner.clone()));
            if !todo_is_countable(verdict) {
                // 은퇴·타 스코프 — 조용히 배제. 직전까지 집계 중이던 파일이 은퇴 선언을 얻은
                // 경우를 위해 기존 등재분도 걷어낸다(유령 잔류 차단).
                todo_progress_map(daemon).remove(&key);
                continue;
            }
            let done = content.matches("- [x]").count() as u64
                + content.matches("- [X]").count() as u64;
            let total = done + content.matches("- [ ]").count() as u64;
            todo_progress_map(daemon).insert(key.clone(), (done, total, mtime));
            if prev.is_some() {
                // 최초 발견은 무음 등록 — 데몬 재시작마다 전 파일 이벤트 폭주 방지
                let mut payload = json!({"path": key, "done": done, "total": total,
                                         "verdict": verdict.as_str()});
                // ★`owner` 동봉(교정 3 · Python 소비자와 정합). 데몬의 집계 **키는 경로 그대로**
                // 유지한다(설계 §5-2가 키 스키마 변경을 파급 확대로 기각). 다만 소비자가 라벨을
                // 파일명에서 추론하지 않아도 되도록 선언의 owner를 실어 보낸다 — Python
                // `javis_report`는 이미 owner를 라벨로 쓰며, 데몬 payload만 파일명 추론에 남으면
                // HUD와 보고기의 라벨이 갈린다. ADR-4 C-3 센티널 `"?"`(주인 미상)는 싣지 않는다.
                // ★S16 — 같은 값이 `org.status`에도 실린다(위 판정 캐시). 이벤트에만 있고
                // 스냅샷에 없으면 HUD 라벨이 새로고침 한 번에 뒤집힌다.
                if let Some(owner) = owner.as_deref() {
                    payload["owner"] = json!(owner);
                }
                daemon.bus.publish("todo.updated", "todo", None, payload);
            }
        }
    }
    // 사라진 파일 정리 — 진행률과 판정 캐시를 **같은 seen 집합**으로 함께 솎는다(캐시 누수 차단).
    // 락 순서 규약(TP→TV) 준수: 두 획득 모두 한 문장 안에서 끝나 가드가 중첩되지 않는다.
    todo_progress_map(daemon).retain(|k, _| seen.contains(k));
    todo_verdict_map(daemon).retain(|k, _| seen.contains(k));
}

/// ★W-B 보완(승인 미감지=워커 hang 방지 · 2026-07-17): agents.json 이 user 소유로 승격되면
/// 사용자 수정본은 영구 보존되지만 **동결**된다 — vendor 가 새 CLI 프롬프트용 approval_patterns 를
/// 추가해도 그 사용자에겐 영영 도달하지 않아 승인 격상이 조용히 멈추고 워커가 hang 한다(우리
/// 지침이 최우선 방지 대상으로 명시한 '큐 적체'의 정확한 기전).
///
/// 해소 = **합집합**: 디스크(사용자) 패턴 + 임베드(vendor) 패턴을 name 기준 dedup 병합하고,
/// 충돌 시 **디스크가 이긴다**(사용자 주권 불변). approval_patterns 는 *감지 전용*(자동 응답
/// 절대 없음 — 판단은 master)이라 추가 패턴은 부작용이 없고 미감지만 위험하다 = 합집합이 안전측.
/// 순수 함수로 분리해 테스트 가능하게 둔다.
fn merged_approval_patterns(
    disk: &serde_json::Value,
    embed: &serde_json::Value,
    agent: &str,
) -> Vec<serde_json::Value> {
    let get = |v: &serde_json::Value| -> Vec<serde_json::Value> {
        v.get(agent)
            .and_then(|a| a.get("approval_patterns"))
            .and_then(|p| p.as_array())
            .cloned()
            .unwrap_or_default()
    };
    let mut out = get(disk);
    let have: std::collections::HashSet<String> = out
        .iter()
        .filter_map(|p| p["name"].as_str().map(String::from))
        .collect();
    for p in get(embed) {
        match p["name"].as_str() {
            Some(n) if !have.contains(n) => out.push(p), // vendor 신규 패턴만 보강
            _ => {}                                      // 동명 = 사용자본 유지(디스크 우선)
        }
    }
    out
}

/// T4-16 승인 격상 스캔: agents.json의 approval_patterns를 visible screen에 매칭.
/// ★자동 응답 절대 금지 — 감지·격상(이벤트+feed 항목)만. 판단은 master의 몫.
fn check_approvals(daemon: &Arc<Daemon>, debounce: &mut HashMap<(u64, String), f64>) {
    let agents: serde_json::Value =
        match std::fs::read_to_string(cys::pack::pack_dir().join("agents.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
        {
            Some(v) => v,
            None => return,
        };
    // 임베드 vendor 정의(동결 사용자본 보강용 — 파싱 실패 시 빈 객체로 무해 폴백).
    let embed_agents: serde_json::Value = cys::pack::PACK_ALL
        .iter()
        .find(|(r, _)| *r == "agents.json")
        .and_then(|(_, c)| serde_json::from_str(c).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    let now = now_epoch();
    let surfaces: Vec<Arc<crate::state::Surface>> =
        daemon.surfaces.lock().unwrap().values().cloned().collect();
    for s in surfaces {
        if s.exited.load(Ordering::Relaxed) {
            continue;
        }
        let Some((agent, _)) = s.agent_meta.lock().unwrap().clone() else {
            continue;
        };
        let patterns = merged_approval_patterns(&agents, &embed_agents, &agent);
        if patterns.is_empty() {
            continue;
        }
        let patterns = &patterns;
        let screen = s.parser.lock().unwrap_or_else(|e| e.into_inner()).screen().contents();
        let mut any_match = false;
        for p in patterns {
            let (Some(name), Some(pattern)) = (p["name"].as_str(), p["pattern"].as_str()) else {
                continue;
            };
            let Ok(re) = regex::Regex::new(pattern) else {
                continue;
            };
            let Some(m) = re.find(&screen) else { continue };
            any_match = true;
            // L3 코얼레싱(2026-07-07 feed 189 폭주 재발방지): 이 surface의 감지 항목이
            // 아직 pending이면 같은 프롬프트 에피소드 — 이벤트·항목을 재발행하지 않는다.
            // (debounce는 rate-limit일 뿐이라 방치 시 분당 1건 무한 누적되던 구조를 차단.
            //  해소 경로 = reply 또는 아래 stale-clear.)
            if daemon.has_pending_daemon_approval(s.id) {
                continue;
            }
            let key = (s.id, name.to_string());
            if debounce.get(&key).map(|t| now - t < 60.0).unwrap_or(false) {
                continue;
            }
            debounce.insert(key, now);
            let excerpt: String = screen[m.start()..]
                .lines()
                .next()
                .unwrap_or("")
                .chars()
                .take(160)
                .collect();
            let role = s.role.lock().unwrap().clone();
            daemon.bus.publish(
                "approval.request",
                "feed",
                Some(s.id),
                json!({"surface_ref": cys::surface_ref(s.id), "role": role,
                       "agent": agent, "pattern": name, "excerpt": excerpt}),
            );
            daemon.push_feed_notification(
                "approval",
                &format!("{agent} 승인 대기 감지 ({})", cys::surface_ref(s.id)),
                &excerpt,
                Some(s.id),
            );
            // L2 방치 차단(2026-07-07 재발방지): 새 에피소드 1건당 master를 큐로 1회 각성 —
            // '즉각 승인' 산문 계약의 기계 배선(재발행 억제는 위 L3 코얼레싱이 보장).
            // 배달은 deliver_queued의 조용시점·typing-guard 규약을 그대로 탄다.
            enqueue_master_wakeup(
                daemon,
                s.id,
                &format!(
                    "[승인감지] {agent} {}에 승인 프롬프트 대기 — read-screen으로 확인 후 즉시 처리하라: {excerpt}",
                    cys::surface_ref(s.id)
                ),
            );
        }
        // L3 stale-clear: 화면에서 승인 패턴이 전부 사라졌으면 이 surface의 pending 감지
        // 항목은 알림 수명 종료 — 자동 종결한다. 프롬프트가 (사람·master의 pane 응답으로)
        // 해소돼도 feed 항목이 영구 pending으로 남아 배지를 오염시키던 생명주기 부재를
        // 봉인하고, 데몬 재시작 고아 백로그도 같은 경로로 청소된다.
        if !any_match {
            for rid in daemon.pending_daemon_approvals(s.id) {
                daemon.resolve_feed_item(&rid, "stale-cleared");
            }
        }
    }
}

/// L2 방치 차단(2026-07-07 feed 폭주 재발방지): master role surface의 pending_queue에
/// 텍스트 1건을 직접 적재한다 — 승인 감지가 이벤트 bus에만 실려 master stdin에 닿지 않던
/// 갭의 봉인. cap(100)·배달 규약(deliver_queued 조용시점·typing-guard)은 큐 기존 계약을
/// 그대로 따른다. master 부재·종료·큐 포화면 조용히 무시하고, 감지 대상이 master 자신이면
/// 적재하지 않는다(자기 프롬프트에 큐 배달 시 다이얼로그 오입력 위험 — stalled escalation이 커버).
fn enqueue_master_wakeup(daemon: &Arc<Daemon>, detected_sid: u64, text: &str) {
    let Some(master_sid) = daemon.roles.lock().unwrap().get("master").copied() else {
        return;
    };
    if master_sid == detected_sid {
        return;
    }
    let Some(s) = daemon.get_surface(master_sid) else {
        return;
    };
    if s.exited.load(Ordering::Relaxed) {
        return;
    }
    // ★C1: 데몬 내부 발신 — System origin으로 정식 편입(종전 `from:"governance-approval"`
    // 문자열 관행을 타입으로 대체). System은 소프트캡 밖이고 하드캡을 받는다.
    // ★E4 주석 정정: "TTL 밖"은 더 이상 사실이 아니다 — B5 이후 System 은 자기 등급 TTL
    // (기본 4h)을 받는다. 승인 대기 알림이 4h 안에 처리되지 않으면 dead-letter 로 이관되며,
    // 그게 의도다(전문 보존 · master 가 죽은 채 무한 누적되는 편이 더 해롭다).
    let item = crate::state::QueueItem::text(
        text.to_string(),
        crate::state::QueueOrigin::system("governance-approval"),
    );
    // ★D9③: 캡 도달 시 종전엔 **무음 drop**이었다 — 승인 대기 알림이 흔적 없이 사라졌다.
    // 이제 dead-letter에 남긴다(어떤 경로로도 무음 삭제 금지). 원장 기록마저 실패하면
    // health 경고가 발행되므로 침묵은 어느 층에서도 성립하지 않는다.
    if let Err(crate::queue_policy::EnqueueError::HardCap) =
        crate::queue_policy::enqueue_item(daemon, &s, item.clone())
    {
        let role = s.role.lock().unwrap().clone();
        let _ = crate::queue_policy::record_dead_letter(
            daemon,
            master_sid,
            role.as_deref(),
            &item,
            crate::queue_policy::DeadLetterReason::WakeupCap,
        );
    }
}

/// L2 escalation(2026-07-07 재발방지): 데몬 감지(approval) 항목이 stall 임계
/// (CYS_APPROVAL_STALL_SECS, 기본 300s)를 넘겨 pending이면 사람 개입 필요 신호
/// approval.stalled를 항목당 1회 발행한다 — 'master가 처리 못한 승인만 사람에게'
/// (v0.12.27 화면전환 원칙)의 데몬측 짝. resolved는 종결 상태라 재발화 없음. 0=비활성.
fn check_approval_stall(daemon: &Arc<Daemon>, fired: &mut std::collections::HashSet<String>) {
    let stall = env_u64("CYS_APPROVAL_STALL_SECS", 300);
    if stall == 0 {
        return;
    }
    let now = now_epoch();
    let (pending_ids, stalled): (
        std::collections::HashSet<String>,
        Vec<(String, String, f64, Option<u64>)>,
    ) = {
        let items = daemon.feed_items.lock().unwrap();
        let pend: std::collections::HashSet<String> = items
            .iter()
            .filter(|i| {
                i.status == "pending"
                    && i.kind == "approval"
                    && i.request_id.starts_with("daemon-")
            })
            .map(|i| i.request_id.clone())
            .collect();
        let st = items
            .iter()
            .filter(|i| {
                i.status == "pending"
                    && i.kind == "approval"
                    && i.request_id.starts_with("daemon-")
                    && now - i.created_at >= stall as f64
            })
            .map(|i| (i.request_id.clone(), i.title.clone(), now - i.created_at, i.surface_id))
            .collect();
        (pend, st)
    };
    fired.retain(|id| pending_ids.contains(id)); // 해소된 항목 키 회수(맵 누수 차단)
    for (rid, title, age, sid) in stalled {
        if !fired.insert(rid.clone()) {
            continue; // 항목당 1회
        }
        daemon.bus.publish(
            "approval.stalled",
            "watchdog",
            sid,
            json!({"request_id": rid, "title": title, "age_secs": age as u64,
                   "surface_ref": sid.map(cys::surface_ref)}),
        );
    }
}

/// L4 백로그 임계 에지 판정(순수) — 임계 이상으로 '처음' 넘어설 때만 true, 임계 미만으로
/// 내려오면 재무장한다. threshold=0은 비활성.
fn feed_backlog_crossed(total: usize, threshold: u64, alerted: &mut bool) -> bool {
    if threshold == 0 {
        return false;
    }
    if total >= threshold as usize {
        if *alerted {
            return false;
        }
        *alerted = true;
        true
    } else {
        *alerted = false;
        false
    }
}

/// L4 백로그 메타 감시(2026-07-07 feed 189 폭주 재발방지): pending 총량이 임계
/// (CYS_FEED_BACKLOG_ALERT, 기본 25)를 넘으면 에지 1회 경보. 개별 항목 aging 재알림
/// (check_feed_aging)과 달리 '쌓임' 자체를 신호화한다 — 생산 경로가 무엇이든(감지 폭주·
/// 처리 주체 부재) 총량 비정상을 조기에 드러낸다.
fn check_feed_backlog(daemon: &Arc<Daemon>, alerted: &mut bool) {
    let threshold = env_u64("CYS_FEED_BACKLOG_ALERT", 25);
    let total = daemon
        .feed_items
        .lock()
        .unwrap()
        .iter()
        .filter(|i| i.status == "pending")
        .count();
    if feed_backlog_crossed(total, threshold, alerted) {
        daemon.bus.publish(
            "feed.backlog_high",
            "watchdog",
            None,
            json!({"pending_total": total, "threshold": threshold}),
        );
    }
}

/// L1 비정규 기동 감시(2026-07-07 feed 폭주 재발방지): claude 에이전트 노드가
/// --dangerously-skip-permissions 없이 떠 있으면 권한 프롬프트가 발생해 승인 감지·방치
/// 폭주의 씨앗이 된다(오늘 사고의 Why-1). 강제 없이 surface당 1회 경고 이벤트만 발행한다
/// — 수동 기동 자체는 합법이므로, 정규 플래그 복귀를 잊은 상태를 조기에 드러내는 게 목적.
/// 정규 플래그로 복귀가 관측되면 재무장한다(이후 재이탈 시 다시 1회 경고).
fn check_launch_flags(
    daemon: &Arc<Daemon>,
    sys: &System,
    warned: &mut std::collections::HashSet<u64>,
) {
    let surfaces: Vec<Arc<crate::state::Surface>> =
        daemon.surfaces.lock().unwrap().values().cloned().collect();
    for s in surfaces {
        if s.exited.load(Ordering::Relaxed) {
            continue;
        }
        let Some((agent, bin)) = s.agent_meta.lock().unwrap().clone() else {
            continue;
        };
        if agent != "claude" {
            continue;
        }
        let bin_base = bin.rsplit(['/', '\\']).next().unwrap_or(&bin).to_string();
        let Some((_, cmdline)) = collect_descendants(sys, s.pid)
            .into_iter()
            .find(|(_, c)| cmdline_matches_agent(c, &bin_base))
        else {
            continue;
        };
        if cmdline.contains("--dangerously-skip-permissions") {
            warned.remove(&s.id); // 정규 복귀 — 재무장
            continue;
        }
        if !warned.insert(s.id) {
            continue; // 이미 경고함
        }
        let role = s.role.lock().unwrap().clone();
        daemon.bus.publish(
            "node.nonstandard_launch",
            "watchdog",
            Some(s.id),
            json!({"agent": agent, "role": role, "surface_ref": cys::surface_ref(s.id),
                   "note": "claude 노드가 bypass 플래그 없이 구동 — 권한 프롬프트 발생 가능(정규 재기동 권장)"}),
        );
    }
}

/// T2-6 토폴로지 영속: role→agent→cwd 매핑을 디스크에 상시 기록 (cys restore의 진실).
pub fn persist_topology(daemon: &Arc<Daemon>) {
    let mut entries: Vec<serde_json::Value> = daemon
        .surfaces
        .lock()
        .unwrap()
        .values()
        .filter(|s| !s.exited.load(Ordering::Relaxed))
        .filter_map(|s| {
            s.role.lock().unwrap().clone().map(|role| {
                let meta = s.agent_meta.lock().unwrap().clone();
                json!({"role": role, "agent": meta.as_ref().map(|(n, _)| n.clone()),
                       "agent_bin": meta.map(|(_, b)| b),
                       "cwd": s.cwd, "title": s.title.lock().unwrap().clone(),
                       // ★CU-7A(additive): surface 셸의 pid. 교차-데몬 발신자 attest(CU-7B)가
                       // "이 pid가 정말 저 데몬의 이 역할인가"를 대조할 유일한 물증이다. 여기서
                       // 굳이 영속하는 이유: attest는 **남의 데몬**을 소켓으로 물어볼 수 없고
                       // (그 자체가 신뢰 문제) 파일이 유일한 교차 데몬 관측면이기 때문이다.
                       "pid": s.pid,
                       "session_id": s.agent_session_id.lock().unwrap().clone(),
                       // (W1) 원 계정 config_dir 영속 — restore가 이 값을 launch 문자열에 인라인해
                       // 데몬 env 변동에도 원 대화(.jsonl)로 정확히 재개한다. 구 topology(필드 없음)는
                       // 로드 시 None → 기존 동작(템플릿 전개)으로 하위호환.
                       "claude_config_dir": s.claude_config_dir.lock().unwrap().clone(),
                       "pack_reinject": s.pack_reinject.lock().unwrap().clone()})
            })
        })
        .collect();
    // ★CU-7A: pid_start_time 보강은 **surfaces 락을 놓은 뒤**에 한다. peer_start_time은 sysinfo
    // 블로킹 syscall이라 락 안에서 돌리면 그동안 surface 생성·종료·send가 전부 정지한다
    // (surface.list 핸들러가 같은 이유로 pid 수집과 refresh를 분리해 둔 관용구를 따른다).
    // pid만으로는 부족한 이유: pid는 재사용된다 — start_time과 쌍이어야 "같은 프로세스"가
    // 결정론으로 증명된다. 죽은 프로세스·조회 실패는 None(=null)으로 남기고, 소비자는 None을
    // "대조 불가 → 미검증"으로 다루면 된다(fail-closed는 소비자 쪽 책임).
    for e in entries.iter_mut() {
        let st = e["pid"]
            .as_u64()
            .and_then(|p| crate::state::peer_start_time(p as u32));
        e["pid_start_time"] = json!(st);
    }
    // ★W2a 묘비 영속: 의도적으로 닫힌 역할을 topology.json에 함께 써 콜드부트를 넘겨 생존시킨다.
    // auto-restore·phoenix가 이 집합을 desired_roster로 병합해 좀비 부활을 원천 차단한다.
    let tombstones: Vec<String> = {
        let mut v: Vec<String> = daemon.tombstones.lock().unwrap().iter().cloned().collect();
        v.sort();
        v
    };
    // ★W2/A-S1: 묘비 집합이 직전 영속본과 달라졌을 때만 tombstones_rev 를 +1(단조 카운터). phoenix 의
    // 조건부 replace(rev ≥ 마지막으로 본 rev) 게이트 근거 — 부분절단/조작으로 묘비만 빈 파일은 rev 부재/역행으로
    // 걸러진다. rev 관리를 이 단일 지점에 집중(각 mutation 사이트 계장 대신)해 "묘비 변경=rev 증가"를 정확히 반영.
    {
        let mut last = daemon.last_persisted_tombstones.lock().unwrap();
        if *last != tombstones {
            daemon
                .tombstones_rev
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            *last = tombstones.clone();
        }
    }
    let rev = daemon.tombstones_rev.load(std::sync::atomic::Ordering::SeqCst);
    let dir = crate::state::state_dir(&daemon.socket_path);
    let content = serde_json::to_string_pretty(&json!({
        // ★A-S1 스키마 마커 — 이 키 부재=legacy topology(phoenix 는 경고+진행).
        // ★CU-7A 버전 정책(명문화): **키 추가=버전 유지 · 키 의미 변경(또는 제거)=버전 범프**.
        // 근거 — 이 파일의 소비자는 전부 "아는 키만 읽는" 관용 파서다: phoenix restore(python·
        // role/agent/cwd/session_id 등만 읽고 pid는 **읽지 않는다**), 묘비 리더(tombstones 키만),
        // state-generations 스냅샷(파일 통복사). 키 추가는 이들 중 누구도 깨지 않으므로 범프하면
        // 오히려 구 바이너리에 legacy 경고를 유발한다(무해한 변경에 비용만 발생). 반대로 기존 키의
        // 의미가 바뀌면 관용 파서는 조용히 오해석하므로 반드시 범프해야 한다.
        "schema_version": 1,
        "tombstones_rev": rev,        // ★A-S1 단조 카운터
        "updated_at": now_epoch(),
        "entries": entries,
        "tombstones": tombstones,
    }))
    .unwrap_or_default();
    // ★원자 쓰기 — SIGTERM/크래시가 쓰기 도중 끼어도 topology.json은 옛 완본 또는 새 완본만
    // 남는다. 비원자 write면 torn write가 깨진 JSON을 남기고 load_topology가 빈 배열로 폴백해
    // 전 노드 resume 핀(=전 세션 컨텍스트)이 증발한다. 패턴: reference_atomic-sidecar-json-write.
    let _ = write_json_atomic(&dir, "topology.json", &content);
}

/// 손상-안전 원자 JSON 쓰기: 같은 디렉터리 temp에 write + fsync(file) → rename(원자 교체)
/// → fsync(dir). rename 원자성 ≠ 데이터 내구성이므로 fsync(file)로 데이터를, fsync(dir)로
/// rename을 영속한다(dir fsync 없으면 rename이 캐시에만 남아 크래시 시 옛 이름 복귀). 실패 시 temp 정리.
pub(crate) fn write_json_atomic(dir: &std::path::Path, name: &str, content: &str) -> std::io::Result<()> {
    use std::io::Write;
    let target = dir.join(name);
    let tmp = dir.join(format!(".{name}.tmp"));
    let res = (|| -> std::io::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(content.as_bytes())?;
        f.sync_all()?;
        std::fs::rename(&tmp, &target)?;
        Ok(())
    })();
    match res {
        Ok(()) => {
            if let Ok(d) = std::fs::File::open(dir) {
                let _ = d.sync_all();
            }
            Ok(())
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

pub fn load_topology(daemon: &Arc<Daemon>) -> serde_json::Value {
    let dir = crate::state::state_dir(&daemon.socket_path);
    std::fs::read_to_string(dir.join("topology.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .map(|v| v["entries"].clone())
        .unwrap_or_else(|| json!([]))
}

fn _tombs_from_value(v: &serde_json::Value) -> std::collections::HashSet<String> {
    v["tombstones"]
        .as_array()
        .map(|a| a.iter().filter_map(|e| e.as_str().map(String::from)).collect())
        .unwrap_or_default()
}

/// ★W2/P0-3: 세대 스냅샷(~/.cys/state-generations/<gen>/topology.json)의 최신 tombstones 폴백.
/// 손상 topology 복구용 — best-effort(스냅샷 부재/없음=빈 집합).
fn tombstones_from_latest_generation() -> std::collections::HashSet<String> {
    let root = cys::home_dir().join(".cys").join("state-generations");
    let mut gens: Vec<String> = match std::fs::read_dir(&root) {
        Ok(rd) => rd
            .flatten()
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| n.len() >= 16 && n.as_bytes()[8] == b'T')
            .collect(),
        Err(_) => return std::collections::HashSet::new(),
    };
    gens.sort();
    for g in gens.iter().rev() {
        let p = root.join(g).join("topology.json");
        if let Ok(s) = std::fs::read_to_string(&p) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) {
                return _tombs_from_value(&v);
            }
        }
    }
    std::collections::HashSet::new()
}

/// ★W2/P0-3: topology.json에서 묘비 집합을 읽는다(데몬 기동 시 in-메모리 tombstones seed용).
/// **부재=빈 집합(fresh 정상)**. **손상(파싱 실패)=조용한 빈집합 금지** — `.corrupt-<ts>` isolate(파일 보존)
/// + 세대 스냅샷 tombstones 폴백. 손상을 빈집합으로 흘리면 폐역 역할이 부활(P0-3)하므로, 스냅샷으로 복구를
/// 시도하고 원본은 isolate 해 소실을 디스크에 확정하지 않는다(.corrupt prune 상한은 W3).
/// ★WP-3·R9(적대검증 W3): 부서 묘비의 영속은 **전용 사이드카**(dept_tombstones.json — writer는
/// 이 데몬 유일)로 한다. topology.json 공유 키였다면 구(pre-WP-3) 바이너리가 topology를
/// 재작성하는 순간 키가 소실돼(버전 스큐 = 이 시스템의 1급 조건) 삭제 부서가 부활한다 —
/// 구 바이너리가 절대 건드리지 않는 신규 파일이 다운그레이드 면역의 정공법(단일-writer 마커 원칙).
fn dept_tombstones_path(socket_path: &std::path::Path) -> std::path::PathBuf {
    crate::state::state_dir(socket_path).join("dept_tombstones.json")
}

pub fn persist_dept_tombstones(daemon: &Arc<Daemon>) {
    let mut v: Vec<String> = daemon.dept_tombstones.lock().unwrap().iter().cloned().collect();
    v.sort();
    let dir = crate::state::state_dir(&daemon.socket_path);
    let content = serde_json::to_string_pretty(&json!({"dept_tombstones": v})).unwrap_or_default();
    let _ = write_json_atomic(&dir, "dept_tombstones.json", &content);
}

/// 부서 묘비 로더 — 사이드카 우선. 손상=.corrupt-ts 격리+WARN+빈 집합(dept 묘비는 role과 달리
/// 사용자 재삭제로 재기록 가능하라 세대 스냅샷까지는 두지 않는다 — 정직한 한계).
/// 사이드카 부재 시 legacy topology.json "dept_tombstones" 키 폴백(초기 빌드 흔적 흡수) → 빈 집합.
pub fn load_dept_tombstones_from_disk(
    socket_path: &std::path::Path,
) -> std::collections::HashSet<String> {
    let p = dept_tombstones_path(socket_path);
    match std::fs::read_to_string(&p) {
        Ok(s) => match serde_json::from_str::<serde_json::Value>(&s) {
            Ok(v) => v
                .get("dept_tombstones")
                .and_then(|t| t.as_array())
                .map(|arr| arr.iter().filter_map(|x| x.as_str().map(String::from)).collect())
                .unwrap_or_default(),
            Err(e) => {
                let ts = now_epoch() as u64;
                let corrupt = p.with_file_name(format!("dept_tombstones.json.corrupt-{ts}"));
                let _ = std::fs::rename(&p, &corrupt);
                eprintln!(
                    "[cysd] dept_tombstones.json 손상({e}) — {} isolate·빈 집합 폴백(부활 게이트 일시 해제 주의)",
                    corrupt.display()
                );
                std::collections::HashSet::new()
            }
        },
        Err(_) => {
            // legacy 폴백: 초기 빌드가 topology.json 키에 기록했을 수 있다(배포 0·dev 흔적 흡수).
            let tp = crate::state::state_dir(socket_path).join("topology.json");
            std::fs::read_to_string(&tp)
                .ok()
                .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
                .and_then(|v| {
                    v.get("dept_tombstones").and_then(|t| t.as_array()).map(|arr| {
                        arr.iter().filter_map(|x| x.as_str().map(String::from)).collect()
                    })
                })
                .unwrap_or_default()
        }
    }
}

pub fn load_tombstones_from_disk(socket_path: &std::path::Path) -> std::collections::HashSet<String> {
    let dir = crate::state::state_dir(socket_path);
    let p = dir.join("topology.json");
    let s = match std::fs::read_to_string(&p) {
        Ok(s) => s,
        Err(_) => return std::collections::HashSet::new(), // 부재 = fresh install 정상
    };
    match serde_json::from_str::<serde_json::Value>(&s) {
        Ok(v) => _tombs_from_value(&v), // valid(구 topology tombstones 키 부재=빈집합·하위호환)
        Err(e) => {
            // 손상 — isolate + 세대 스냅샷 폴백(조용한 소실 금지).
            let ts = now_epoch() as u64;
            let corrupt = dir.join(format!("topology.json.corrupt-{ts}"));
            let _ = std::fs::rename(&p, &corrupt);
            let recovered = tombstones_from_latest_generation();
            eprintln!(
                "[cysd] ★P0-3 topology.json 손상({e}) — {} isolate + 세대 스냅샷 tombstones 폴백({}개 복구)",
                corrupt.display(),
                recovered.len()
            );
            recovered
        }
    }
}

/// ★W2/A-S1: topology.json 의 tombstones_rev 를 읽어 기동 카운터를 시드(재시작 넘어 단조성 유지).
/// 필드 부재(legacy·fresh install)·부재·손상은 0(phoenix 는 epoch 변경으로 rebase 처리 — gemini R3).
pub fn load_tombstones_rev_from_disk(socket_path: &std::path::Path) -> u64 {
    let dir = crate::state::state_dir(socket_path);
    std::fs::read_to_string(dir.join("topology.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v["tombstones_rev"].as_u64())
        .unwrap_or(0)
}

fn check_load(daemon: &Daemon, last_alert: &mut f64) {
    let load = System::load_average();
    if load.one > daemon.config.load_high_threshold
        && now_epoch() - *last_alert > LOAD_DEBOUNCE_SECS
    {
        *last_alert = now_epoch();
        daemon.bus.publish(
            "watchdog.load_high",
            "watchdog",
            None,
            json!({"load_1m": load.one, "load_5m": load.five, "threshold": daemon.config.load_high_threshold}),
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// ★SEAT: 좌석 점유(seat-occupancy) 판정 — 단일 SOT
//
// 왜 필요한가(2026-07-17 실사고): role=master 를 쥔 채 agent 가 없는 '빈 셸' 좌석이
// ①phoenix 부활 ②▶부서장/▶CEO 버튼 ③디렉티브 재주입을 전부 잠그고, master 앞 큐 메시지를
// zsh 프롬프트에 문자로 타이핑시켰다. 뿌리는 모든 생존 판정이 `exited` 만 보고 **좌석에 실제로
// 누가 앉아 있는지**를 묻지 않은 것이다(cys.rs run_restore `live.contains(role)` ·
// javis_phoenix `_alive()` · surface.create/claim_role 의 holder_live 전부 동형).
//
// 설계 원칙(3중 성찰 반영):
//  - **커널 사실 1차**: "셸 이외의 자손 프로세스가 있는가" = 좌석이 쓰이는 중인가. hook·에이전트
//    종류·config 와 무관하게 커널이 증언한다. 에이전트 계층 부산물(usage transcript 등록 등)을
//    판정에 섞으면 hook 없는 환경에서 '영원히 Occupied → 부활 잠김'이라는 **고치려는 결함과 동형의
//    반대편 결함**이 열린다 — 계층을 섞지 않는다.
//  - **정책 2차는 승계에만**: agent_meta·최근 사람 입력은 좌석을 *지키는* 방향으로만 가산한다
//    (seat_claimable). 큐 배달은 1차만 본다 — agent_meta 가 남은 죽은 노드의 셸에 배달하면 그것도
//    zsh 타이핑이기 때문이다(같은 사고의 다른 얼굴).
//  - **Unknown = 현행 동작 유지**: 프로브 미도달은 새 실패를 만들지 않는다(큐=배달·승계=거부).
//
// 한계(명시): 사용자가 좌석에서 잡을 백그라운드로 돌리면(`sleep 100 &`) 프롬프트가 비어도
// Occupied 로 판정된다 — 보호(fail-closed) 방향이라 오탈취는 없고, 부활은 다음 틱에 재시도된다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeatState {
    /// 판정 미도달(프로브 실패·첫 틱 이전) — 소비처는 **현행 동작**으로 강등한다.
    Unknown = 0,
    /// 자손 프로세스 존재 = 사람이든 에이전트든 이 좌석은 쓰이는 중.
    Occupied = 1,
    /// 셸 단독 = 빈 좌석.
    Empty = 2,
}

impl SeatState {
    pub fn as_u8(self) -> u8 {
        self as u8
    }
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => SeatState::Occupied,
            2 => SeatState::Empty,
            _ => SeatState::Unknown,
        }
    }
    /// status/topology 노출용 — pack·CLI 는 이 문자열만 소비한다(판정 이원화 금지).
    pub fn as_str(self) -> &'static str {
        match self {
            SeatState::Occupied => "occupied",
            SeatState::Empty => "empty",
            SeatState::Unknown => "unknown",
        }
    }
}

/// ★SEAT 1차(커널 사실): 이 좌석에 셸 이외의 자손 프로세스가 있는가.
/// 종료된 surface 는 좌석 개념이 없으므로 Empty(무해 — 승계 게이트는 exited 를 이미 별도 처리).
/// 셸 pid 가 프로세스 표에 아예 없으면(아직 미갱신·프로브 실패) Unknown → 소비처가 현행 동작 유지.
pub fn seat_state(sys: &System, s: &crate::state::Surface) -> SeatState {
    if s.exited.load(Ordering::Relaxed) {
        return SeatState::Empty;
    }
    if sys.process(sysinfo::Pid::from_u32(s.pid)).is_none() {
        return SeatState::Unknown;
    }
    if collect_descendants(sys, s.pid).is_empty() {
        SeatState::Empty
    } else {
        SeatState::Occupied
    }
}

/// ★SEAT 캐시 갱신 — **단일 writer**(watchdog 틱). 판정 재료(전 프로세스 표)를 이미 refresh 한
/// 지점에서 한 번만 계산해 캐시에 싣는다. RPC 읽기 경로(surface.list·status·deliver_queued)는
/// 재조회 없이 이 값을 소비한다(비용 중복 0).
pub fn refresh_seat_cache(daemon: &Arc<Daemon>, sys: &System) {
    let surfaces: Vec<Arc<crate::state::Surface>> =
        daemon.surfaces.lock().unwrap().values().cloned().collect();
    for s in surfaces {
        s.seat_cache
            .store(seat_state(sys, &s).as_u8(), Ordering::Relaxed);
    }
}

/// ★SEAT 2차(승계 정책): 이 좌석의 특권 role 을 다른 surface 가 가져가도 되는가.
/// 커널 사실이 Empty 이고 + agent_meta 부재(죽은 에이전트의 좌석은 node-recover 영역이지 탈취
/// 대상이 아니다) + 최근 사람 입력 없음(사용자가 지금 claude 를 띄우려 타이핑 중일 수 있다)
/// 셋을 **모두** 만족할 때만 true. Unknown 은 false(현행=거부 유지).
pub fn seat_claimable(sys: &System, s: &crate::state::Surface) -> bool {
    if seat_state(sys, s) != SeatState::Empty {
        return false;
    }
    if s.agent_meta.lock().unwrap().is_some() {
        return false;
    }
    let human_recent = s
        .last_human_input
        .lock()
        .unwrap()
        .map(|t| t.elapsed().as_secs() < queue_human_quiet_secs())
        .unwrap_or(false);
    !human_recent
}

/// 승계 게이트 전용 즉시 프로브 — 캐시(watchdog 틱 주기)는 role 재바인딩 판정에 쓰기엔 stale 하다.
/// 드문 경로(부활·부트 선언)라 전 프로세스 refresh 비용을 그 시점에 지불한다.
pub fn seat_claimable_now(s: &crate::state::Surface) -> bool {
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::All, true);
    seat_claimable(&sys, s)
}

/// Walk the process table and collect all descendants of `root`.
/// 에이전트 생존 매칭 — cmdline의 어느 토큰이든 ①basename 정확 일치 ②`.js` 번들 일치
/// (`…/gemini.js`) ③경로 세그먼트 일치(`…/gemini/…` 또는 `…/gemini-cli/…` 패키지 경로)면
/// 생존으로 본다. 구(舊) 규칙(앞 3토큰 제한 + basename 단일 일치)은 npm 래퍼 에이전트
/// (`node --옵션 …/@google/gemini-cli/bundle/gemini.js`)를 놓쳐 agent_alive=false 오판 →
/// orchestra check 상시 FAIL → 멀쩡한 노드 수선·오살(quit·close) 연쇄를 낳았다
/// (2026-06-12 실측). false-negative(오살)가 false-positive보다 훨씬 위험하므로 매칭을
/// 넓힌다 — 검사 범위는 어차피 해당 surface의 자손 프로세스로 한정된다.
pub fn cmdline_matches_agent(cmdline: &str, bin_base: &str) -> bool {
    if bin_base.is_empty() {
        return false;
    }
    // 패키지 세그먼트는 `<bin>-cli`·`<bin>-code` 정확 일치만(실존 npm 패키지명:
    // @google/gemini-cli·@anthropic-ai/claude-code) — `<bin>-` 접두 전체를 열면
    // claude-code-router·grok-1-weights 같은 무관 경로가 생존으로 오판된다(적대 검증 R1:
    // 죽음 은폐 → node-recover 거부의 역결함).
    let pkg_cli = format!("{bin_base}-cli");
    let pkg_code = format!("{bin_base}-code");
    cmdline.split_whitespace().any(|tok| {
        let base = tok.rsplit(['/', '\\']).next().unwrap_or(tok);
        if base == bin_base || base.strip_suffix(".js").is_some_and(|b| b == bin_base) {
            return true;
        }
        // 경로 세그먼트 매칭은 실제 경로 토큰에서만 (단어 인자 오탐 방지)
        tok.contains(['/', '\\'])
            && tok
                .split(['/', '\\'])
                .any(|seg| seg == bin_base || seg == pkg_cli || seg == pkg_code)
    })
}

pub fn collect_descendants(sys: &System, root: u32) -> Vec<(u32, String)> {
    // parent → children index
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    for (pid, proc_) in sys.processes() {
        if let Some(parent) = proc_.parent() {
            children
                .entry(parent.as_u32())
                .or_default()
                .push(pid.as_u32());
        }
    }
    let mut out = Vec::new();
    let mut stack = vec![root];
    // pid 재사용으로 부모 링크에 사이클이 생겨도 무한루프하지 않게 방문 집합 유지
    let mut seen: std::collections::HashSet<u32> = std::collections::HashSet::new();
    seen.insert(root);
    while let Some(p) = stack.pop() {
        if let Some(kids) = children.get(&p) {
            for &kid in kids {
                if !seen.insert(kid) {
                    continue;
                }
                let cmdline = sys
                    .process(Pid::from_u32(kid))
                    .map(|pr| {
                        let parts: Vec<String> = pr
                            .cmd()
                            .iter()
                            .map(|s| s.to_string_lossy().into_owned())
                            .collect();
                        if parts.is_empty() {
                            pr.name().to_string_lossy().into_owned()
                        } else {
                            parts.join(" ")
                        }
                    })
                    .unwrap_or_default();
                out.push((kid, cmdline));
                stack.push(kid);
            }
        }
    }
    out
}

/// 중복 프로세스 kill 정책 — 순수 판정(테스트 핀). check_surfaces가 sys·daemon에서
/// 입력을 미리 수집해 넘기고, 집행(kill_pid·bus.publish)은 호출부에 잔류한다.
///
/// 불변식(★실측 결함 회귀 가드):
///  ① 최古(가장 낮은 pid) 1개는 *항상* 보존 — 정상 서버 1개까지 죽이면 안 된다.
///  ② min_age_secs 미만으로 산 pid는 보존 — 빌드 중 잠깐 뜬 프로세스 오살 방지.
///  ③ 입력이 결정론 정렬(pid asc)되지 않아도 내부에서 정렬 — 죽이는 pid가 호출 순서에
///     의존하면(같은 그룹인데 다른 pid kill) 재현 불가 버그가 된다.
///
/// 입력: ages = (pid, start_time_epoch_secs) 목록(한 cmdline 그룹). now = 현재 에폭.
/// 출력: (kept, killed) — kept=보존된 최古 pid, killed=죽일 pid(pid asc).
fn plan_duplicate_kills(mut ages: Vec<(u32, f64)>, now: f64, min_age_secs: f64) -> (u32, Vec<u32>) {
    ages.sort_by_key(|&(pid, _)| pid); // 불변식 ③: 결정론 정렬
    let kept = ages[0].0; // 불변식 ①: 최古 보존
    let killed: Vec<u32> = ages[1..]
        .iter()
        .filter(|&&(_, start)| now - start >= min_age_secs) // 불변식 ②: 나이 게이트
        .map(|&(pid, _)| pid)
        .collect();
    (kept, killed)
}

/// 완화책 ③: surface별 자식 수 감시 + 동일 cmdline 중복 서버 감지 (예: bun server.ts × 36).
fn check_surfaces(
    daemon: &Daemon,
    sys: &System,
    last_dup_alert: &mut HashMap<String, f64>,
    last_proc_alert: &mut HashMap<u64, f64>,
) {
    let surfaces: Vec<(u64, u32)> = daemon
        .surfaces
        .lock()
        .unwrap()
        .values()
        .map(|s| (s.id, s.pid))
        .collect();

    let mut cmdline_groups: HashMap<String, Vec<u32>> = HashMap::new();
    for (sid, root_pid) in &surfaces {
        let descendants = collect_descendants(sys, *root_pid);
        if descendants.len() > daemon.config.proc_count_threshold {
            // 디바운스 — 임계 초과 상태가 지속돼도 5초마다 영구 발행하지 않는다
            let now = now_epoch();
            let fire = last_proc_alert
                .get(sid)
                .map(|t| now - t > LOAD_DEBOUNCE_SECS)
                .unwrap_or(true);
            if fire {
                last_proc_alert.insert(*sid, now);
                daemon.bus.publish(
                    "watchdog.proc_count_high",
                    "watchdog",
                    Some(*sid),
                    json!({"count": descendants.len(), "threshold": daemon.config.proc_count_threshold}),
                );
            }
        }
        for (pid, cmdline) in descendants {
            if !cmdline.is_empty() {
                cmdline_groups.entry(cmdline).or_default().push(pid);
            }
        }
    }

    for (cmdline, pids) in cmdline_groups {
        if pids.len() >= daemon.config.duplicate_threshold {
            let now = now_epoch();
            let fire = last_dup_alert
                .get(&cmdline)
                .map(|t| now - t > LOAD_DEBOUNCE_SECS)
                .unwrap_or(true);
            if !fire {
                continue;
            }
            last_dup_alert.insert(cmdline.clone(), now);
            daemon.bus.publish(
                "watchdog.duplicate_procs",
                "watchdog",
                None,
                json!({"cmdline": cmdline, "count": pids.len(), "pids": pids,
                       "auto_kill": daemon.config.auto_kill_duplicates}),
            );
            if daemon.config.auto_kill_duplicates {
                // 디렉티브 스펙 "45초+/3개+": 정책 판정은 순수 함수(plan_duplicate_kills)에
                // 위임하고, sys 의존 입력 수집·집행(kill_pid·publish)만 controller에 잔류한다.
                const MIN_AGE_SECS: f64 = 45.0;
                // sys 의존 입력을 순수 경계 밖에서 미리 수집(start_time은 System에서만 조회 가능).
                let ages: Vec<(u32, f64)> = pids
                    .iter()
                    .filter_map(|&pid| {
                        sys.process(Pid::from_u32(pid))
                            .map(|p| (pid, p.start_time() as f64))
                    })
                    .collect();
                if !ages.is_empty() {
                    let (kept, killed) = plan_duplicate_kills(ages, now, MIN_AGE_SECS);
                    if !killed.is_empty() {
                        for &pid in &killed {
                            kill_pid(pid); // 집행 (controller 잔류)
                        }
                        daemon.bus.publish(
                            // 집행 (controller 잔류)
                            "watchdog.duplicates_killed",
                            "watchdog",
                            None,
                            json!({"cmdline": cmdline, "kept": kept, "killed": killed,
                                   "min_age_secs": MIN_AGE_SECS}),
                        );
                    }
                }
            }
        }
    }
}

/// 완화책 ②: 출력이 멎은 지 idle_seconds 지난 surface를 push로 알린다.
/// master가 이 이벤트로 작업 분할·점검 판단을 한다 (read-screen 폴링 불필요).
fn check_idle(daemon: &Daemon) {
    let surfaces: Vec<Arc<crate::state::Surface>> =
        daemon.surfaces.lock().unwrap().values().cloned().collect();
    for s in surfaces {
        if s.exited.load(Ordering::Relaxed) {
            continue;
        }
        let idle_for = s.last_output.lock().unwrap().elapsed().as_secs();
        if idle_for >= daemon.config.idle_seconds && !s.idle_notified.swap(true, Ordering::Relaxed)
        {
            daemon.bus.publish(
                "pane.idle",
                "watchdog",
                Some(s.id),
                json!({"idle_seconds": idle_for, "surface_ref": cys::surface_ref(s.id)}),
            );
        }
    }
}

/// 완화책 ③ 생명주기 강제 종료: scoped 등록 프로세스의 소유 surface가 사라졌거나
/// 프로세스가 이미 죽었으면 원장을 정리하고, 살아있는 고아는 강제 종료한다.
fn reap_orphan_ledger(daemon: &Daemon, sys: &System) {
    let mut to_kill: Vec<(u32, i32)> = Vec::new();
    let mut to_remove: Vec<u32> = Vec::new();
    {
        let surfaces = daemon.surfaces.lock().unwrap();
        let ledger = daemon.ledger.lock().unwrap();
        for entry in ledger.values() {
            let alive = sys.process(Pid::from_u32(entry.pid)).is_some();
            if !alive {
                to_remove.push(entry.pid);
                continue;
            }
            if entry.scoped {
                if let Some(sid) = entry.surface_id {
                    if !surfaces.contains_key(&sid) {
                        to_kill.push((entry.pid, entry.pgid));
                        to_remove.push(entry.pid);
                    }
                }
            }
        }
    }
    for (pid, pgid) in to_kill {
        kill_group_or_pid(pid, pgid);
        daemon.bus.publish(
            "ledger.killed",
            "ledger",
            None,
            json!({"pid": pid, "reason": "owning surface closed"}),
        );
    }
    if !to_remove.is_empty() {
        let mut ledger = daemon.ledger.lock().unwrap();
        for pid in to_remove {
            ledger.remove(&pid);
        }
    }
}

/// reap 기능 on/off — 기본 on, `CYS_REAP_EXITED=0`으로만 비활성(다른 노브 컨벤션과 동일).
fn reap_exited_enabled() -> bool {
    std::env::var("CYS_REAP_EXITED")
        .map(|v| v != "0")
        .unwrap_or(true)
}

/// 종료 후 경과초가 grace 이상이면 회수 대상. grace는 비정상 크래시의 포렌식·노드복구
/// 윈도우 — 역할 노드(worker/cso/reviewer/master)는 길게(기본 60초), 비역할(스크래치·
/// one-shot)은 짧게(기본 10초). 경계값을 박제하기 위해 순수 함수로 분리한다.
fn exited_surface_due(has_role: bool, elapsed_secs: u64) -> bool {
    let grace = if has_role {
        env_u64("CYS_REAP_EXITED_GRACE_SECS", 60)
    } else {
        env_u64("CYS_REAP_EXITED_NONROLE_GRACE_SECS", 10)
    };
    elapsed_secs >= grace
}

/// 자력종료(셸 EOF) surface 회수: `exited=true`인데 close_surface를 거치지 않아
/// (state.rs가 exited만 세움) 레지스트리에 영구 잔존하는 죽은 surface를, 종료 후
/// grace가 지나면 close_surface로 정리한다. grace는 비정상 크래시의 포렌식(마지막 화면)·
/// 노드복구(surface.exited 구독자) 윈도우 — 역할 노드(worker/cso/reviewer/master)는 길게,
/// 비역할(스크래치·one-shot)은 짧게. close_surface는 이미 reap된 자식에도 안전(kill/wait
/// 에러 무시)하므로 신규 종료 로직 없이 '언제 부를지'만 추가한다.
fn reap_exited_surfaces(daemon: &Arc<Daemon>) {
    if !reap_exited_enabled() {
        return;
    }
    // (id, role) 수집은 surfaces Arc 클론으로 — surfaces 락을 짧게 잡고 즉시 놓는다
    // (check_agent_death와 동일 패턴). close_surface는 surfaces 락을 새로 잡으므로
    // 수집과 회수를 분리해 재진입을 피한다.
    let mut to_reap: Vec<(u64, Option<String>)> = Vec::new();
    {
        let surfaces: Vec<Arc<crate::state::Surface>> =
            daemon.surfaces.lock().unwrap().values().cloned().collect();
        for s in surfaces {
            if !s.exited.load(Ordering::Relaxed) {
                continue;
            }
            let Some(exited_at) = *s.exited_at.lock().unwrap() else {
                continue; // exited지만 stamp 직전(찰나) — 다음 틱에
            };
            let role = s.role.lock().unwrap().clone();
            if exited_surface_due(role.is_some(), exited_at.elapsed().as_secs()) {
                to_reap.push((s.id, role));
            }
        }
    }
    for (id, role) in to_reap {
        // 경쟁(이미 닫힘)은 Err — 무시. 성공 시에만 reaped 이벤트.
        if close_surface(daemon, id, CloseCause::Reap).is_ok() {
            daemon.bus.publish(
                "surface.reaped",
                "surface",
                Some(id),
                json!({"surface_ref": cys::surface_ref(id),
                       "reason": "exited_grace_elapsed", "role": role}),
            );
        }
    }
}

pub fn kill_pid(pid: u32) {
    #[cfg(unix)]
    {
        // pid 0(자기 그룹)·음수 래핑(-1=전체 프로세스) 차단 — 심층 방어
        match i32::try_from(pid) {
            Ok(p) if p > 0 => unsafe {
                libc::kill(p, libc::SIGKILL);
            },
            _ => {}
        }
    }
    #[cfg(windows)]
    {
        use crate::state::HideConsole;
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .hide_console()
            .output();
    }
}

pub fn kill_group_or_pid(pid: u32, pgid: i32) {
    #[cfg(unix)]
    {
        if pgid > 0 {
            unsafe {
                libc::killpg(pgid, libc::SIGKILL);
            }
        } else {
            kill_pid(pid);
        }
    }
    #[cfg(windows)]
    {
        let _ = pgid;
        kill_pid(pid);
    }
}

/// 종료 시 회수해야 할 scoped 프로세스 그룹 목록을 원장에서 추린다 (`(pid, pgid)`).
/// 원장은 메모리 전용이라 데몬이 죽으면 아무도 scoped 자식을 회수하지 못한다 —
/// SIGTERM/SIGINT(unix)·Ctrl-C/console-close/shutdown(windows) 핸들러가 모두
/// 이 동일 선별을 거쳐 `kill_group_or_pid`로 그룹을 정리한다. scoped 프로세스는
/// (windows에서) 데몬의 자식이 아니라 cys CLI의 자식이므로 데몬 트리만 죽이는
/// `taskkill /T`로는 닿지 않는다 — 반드시 원장 pid를 직접 회수해야 한다.
pub fn collect_scoped_for_shutdown(
    ledger: &std::collections::HashMap<u32, crate::state::LedgerEntry>,
) -> Vec<(u32, i32)> {
    ledger
        .values()
        .filter(|e| e.scoped)
        .map(|e| (e.pid, e.pgid))
        .collect()
}

/// watchdog 태스크-로컬 디바운스/카운터 맵의 무한 성장을 막는다.
/// 이 4개 맵은 spawn_watchdog 루프 안의 로컬 변수라 close_surface가 접근할 수 없어
/// prune_surface_health_keys(close_surface 지점에서 회수)와 같은 방식을 쓸 수 없다.
/// surface_id는 max_surface_id+1에서 단조 증가해 재시작 너머로도 재사용되지 않으므로,
/// surface가 닫혀도 surface_id-키 엔트리가 영구 잔존한다 → watchdog 틱마다 살아있는
/// surface 집합으로 솎아낸다(prune_surface_health_keys와 동일 철학, 회수 지점만 다름):
///   · last_proc_alert·restart_counts(키=surface_id) → 죽은 surface 키 제거
///   · approval_debounce(키=(surface_id, pattern)) → 죽은 surface 키 제거
/// last_dup_alert(키=cmdline 문자열)는 surface와 무관하고 cmdline이 사실상 무한 변종
/// (temp 경로·PID·타임스탬프)이라 가장 빨리 샌다. cmdline은 살아있는 surface 집합으로
/// 솎을 수 없으므로 나이 기반으로 제거한다: check_surfaces의 fire 판정이 이미
/// `now - t > LOAD_DEBOUNCE_SECS`인 엔트리를 만료(=재발화)로 취급하므로, 그보다 오래된
/// 엔트리를 비우는 것은 디바운스 의미를 정확히 보존한다(비웠다 재삽입 == 잔존한 만료
/// 엔트리, 둘 다 fire). 순수 함수로 분리해 full Daemon 없이 회귀 가드를 박는다.
fn prune_watchdog_debounce_maps(
    last_dup_alert: &mut HashMap<String, f64>,
    last_proc_alert: &mut HashMap<u64, f64>,
    restart_counts: &mut HashMap<u64, u32>,
    approval_debounce: &mut HashMap<(u64, String), f64>,
    live_surface_ids: &std::collections::HashSet<u64>,
    now: f64,
) {
    last_proc_alert.retain(|sid, _| live_surface_ids.contains(sid));
    restart_counts.retain(|sid, _| live_surface_ids.contains(sid));
    approval_debounce.retain(|(sid, _), _| live_surface_ids.contains(sid));
    // cmdline-키 맵: 디바운스 창(LOAD_DEBOUNCE_SECS)을 이미 넘긴 만료 엔트리만 제거.
    last_dup_alert.retain(|_, &mut t| now - t <= LOAD_DEBOUNCE_SECS);
}

/// health_debounce·health_hits에서 닫힌 surface의 (surface_id, rule) 키를 회수한다.
/// 두 맵은 run_health_rules가 (surface_id, rule_name) 키로 insert만 하고 surface 종료
/// 시 어디서도 키를 비우지 않아, surface를 계속 생성·종료하는 24/365 데몬에서 죽은
/// surface별 (룰 수)개의 엔트리가 단조 누적된다(caller_cache와 동일 계열 누수).
/// surface가 맵에서 사라지는 유일 지점(close_surface)에서 두 맵의 해당 키를 솎아내
/// 유한하게 유지한다. 순수 함수로 분리해 full Daemon 없이 회귀 가드를 박는다.
fn prune_surface_health_keys(
    debounce: &mut HashMap<(u64, String), std::time::Instant>,
    hits: &mut HashMap<(u64, String), Vec<f64>>,
    id: u64,
) {
    debounce.retain(|(sid, _), _| *sid != id);
    hits.retain(|(sid, _), _| *sid != id);
}

/// close_surface 호출 사유 — 묘비 삽입 여부를 가른다.
/// 묘비는 "오너가 의도적으로 폐역한 역할"에만 적용돼야 하고(좀비 부활 차단), watchdog가
/// 크래시·EOF·동반사망을 회수하는 경우는 부활 대상이므로 묘비를 남기지 않는다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseCause {
    /// 오너 의도적 닫기(UI 탭 닫기·surface.close RPC) — 역할을 묘비에 올려 auto-restore 좀비 부활 차단.
    OwnerClose,
    /// watchdog 회수(크래시·셸 EOF·데몬 재시작 동반사망·fresh TTL) — 부활 대상이므로 묘비 미삽입.
    Reap,
}

/// Close a surface: kill the entire descendant process tree, then the shell itself.
/// 고아 서버 누적(load 폭주의 원인)을 원천 차단하는 지점.
pub fn close_surface(daemon: &Arc<Daemon>, id: u64, cause: CloseCause) -> Result<(), String> {
    // 멤버십 제거 + 역할 정리를 surfaces 락 아래 한 임계영역에서 —
    // claim_role과 동일한 락 순서(surfaces → roles → surface.role)로 AB-BA 데드락 차단.
    let surface = {
        let mut surfaces = daemon.surfaces.lock().unwrap();
        let surface = surfaces
            .remove(&id)
            .ok_or_else(|| format!("surface {id} not found"))?;
        let mut roles = daemon.roles.lock().unwrap();
        let srole = surface.role.lock().unwrap();
        let mut master_released = false;
        // ★W2a: surface.close = 의도적 닫기. 이 surface가 실제로 보유한 역할(roles 맵이 이 id를
        // 가리킬 때만 — 이미 다른 surface로 재배정된 역할은 그쪽이 살아있으므로 묘비 대상 아님)을
        // 묘비에 올려 auto-restore의 좀비 부활을 차단한다. 실제 삽입은 락 해제 후(tombstones는 리프 락).
        let mut tombstone_role: Option<String> = None;
        if let Some(role) = srole.as_ref() {
            if roles.get(role) == Some(&id) {
                roles.remove(role);
                tombstone_role = Some(role.clone());
                // 벡터-9 방어심화: master 보유 surface가 종료되면 master_claimed_at을 비운다
                // (master 부재 → approval.sign 동결, 다음 정당 승계 시 쿨다운 재시작).
                if role == "master" {
                    master_released = true;
                }
            }
        }
        drop(srole);
        drop(roles);
        // master_claimed_at 갱신은 surfaces·roles 락 해제 후(단일 락만 보유 → 락 순서 무변경).
        if master_released {
            *daemon.master_claimed_at.lock().unwrap() = None;
        }
        // 묘비 삽입만 cause로 게이트 — role-map 정리·master_claimed_at 해제는 위에서 두 사유 모두
        // 이미 수행됐다(reap된 surface도 역할 매핑을 놓아야 신규가 claim 가능). Reap은 부활 대상이라
        // 묘비를 남기지 않는다(phoenix가 desired_roster로 되살린다).
        if let Some(role) = tombstone_role {
            if cause == CloseCause::OwnerClose {
                daemon.tombstones.lock().unwrap().insert(role);
            }
        }
        surface
    };
    // ★D7(BOOTSTRAP_HARDENING WP-3): 묘비를 kill 루프 **이전**에 선영속 — 아래 kill 구간에서
    // 데몬이 SIGKILL/크래시로 죽으면 in-memory 묘비가 디스크에 없어 다음 콜드부트 phoenix가
    // "의도 삭제된 역할"을 부활시켰다. surfaces 락 해제 직후라 persist_topology 재진입 안전
    // (말미 persist는 role-map 후속 정리 반영용으로 유지 — 이중 persist 비용 수용).
    persist_topology(daemon);
    // health 디바운스·조치 게이트 맵에서 이 surface의 (surface_id, rule) 키 회수 —
    // surface가 맵에서 사라지는 유일 지점에서 함께 비워 누수를 차단한다(별도 락).
    prune_surface_health_keys(
        &mut daemon.health_debounce.lock().unwrap(),
        &mut daemon.health_hits.lock().unwrap(),
        id,
    );
    // 미배달 큐 폐기 — queued:true 응답을 받은 발신자의 무음 메시지 유실 차단.
    //
    // ★R1 무손실(record-before-remove): 종전엔 drain 으로 먼저 비우고 **이벤트만** 냈다.
    // 이벤트 버스는 유실 가능한 링이고 원장(dead-letters.jsonl)이 사실의 SOT라는 게 이 설계의
    // 전제이므로, 이 경로는 사실상 무음 인멸이었다 — 오퍼레이터가 탭을 닫는 순간 인플라이트
    // 메시지가 어디에도 남지 않고 사라졌다. 이제 항목별로 원장을 먼저 쓰고 성공분만 제거한다.
    let outcome = crate::queue_policy::record_then_drain(
        daemon,
        &surface,
        crate::queue_policy::DeadLetterReason::SurfaceClosed,
    );
    if outcome.total() > 0 {
        daemon.bus.publish(
            crate::queue_policy::queue_events::DROPPED,
            crate::queue_policy::queue_events::CATEGORY,
            Some(id),
            json!({"reason": "surface_closed", "count": outcome.cleared.len(),
                   "dead_lettered": outcome.cleared.len(),
                   "retained": outcome.retained.len(),
                   "bytes": outcome.cleared_bytes()}),
        );
        // 원장 기록에 실패한 항목은 버리지 않는다. 이 surface 는 이미 surfaces 맵에서 빠졌으므로
        // pending_queue 에 남겨두면 persist_queue_state 가 보지 못한다 — restored_queue 로 옮겨
        // WAL 에 실리게 하고, rehome_restored_queue 가 같은 role 의 새 좌석으로 재배달한다.
        if !outcome.retained.is_empty() {
            daemon.adopt_orphan_queue_items(id, outcome.role.clone(), &outcome.retained);
        }
        // ★D9②(기존 버그 보수): drain 후 WAL을 갱신하지 않아 **유령 항목**이 남았다.
        // 디스크에는 방금 폐기한 메시지가 그대로 있으므로, 다음 재기동에서 restored_queue로
        // 되살아나 같은 role의 새 surface에 오배달된다(사용자가 본 적 없는 유령 배달).
        // drain은 큐 상태 변화이므로 enqueue/pop/clear와 동일하게 즉시 영속해야 한다.
        daemon.persist_queue_state();
    }
    // 시간이 걸리는 sysinfo refresh·프로세스 킬은 락 밖에서 수행
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::All, true);
    let descendants = collect_descendants(&sys, surface.pid);
    for (pid, _) in &descendants {
        kill_pid(*pid);
    }
    {
        let mut child = surface.child.lock().unwrap();
        let _ = child.kill();
        // kill 후 reap — 좀비 잔존 차단 (reader 스레드의 try_wait와는 같은 Mutex로 직렬화)
        let _ = child.wait();
    }
    daemon.bus.publish(
        "surface.closed",
        "surface",
        Some(id),
        json!({"surface_ref": cys::surface_ref(id), "descendants_killed": descendants.len()}),
    );
    persist_topology(daemon);
    Ok(())
}

/// try_send로 writer 채널에 인계한 머리 메시지를 큐에서 제거한다.
/// deliver_queued가 front 읽기·인계·이 호출을 한 락 임계영역으로 묶으므로 호출 시점에
/// 머리는 항상 방금 보낸 항목이다. 그래도 머리 일치를 확인하고 제거하는 belt-and-suspenders
/// 가드 — 무조건 pop_front이 미배달 새 머리를 삼키는 일을 구조적으로 차단한다.
///
/// ★C1: 대조 키가 **텍스트 → seq**로 바뀌었다. 텍스트 비교는 동일 문자열 다건(Return 큐잉·
/// 재시도 wake ×4)을 구별하지 못해, 멱등 교체(C4)가 제자리에 넣은 신 항목을 구 항목으로
/// 오인해 삼킬 수 있었다. seq는 항목 정체성이므로 그 오인이 구조적으로 불가능하다.
fn pop_delivered_head(q: &mut std::collections::VecDeque<crate::state::QueueItem>, delivered_seq: u64) {
    if q.front().map(|i| i.seq) == Some(delivered_seq) {
        q.pop_front();
    }
}

/// queued 배달의 '조용함' 임계(초) — 기본 3초. 출력이 잦은 pane(master 등)에는 큐가
/// 오래 막힐 수 있어 환경별 조정을 허용한다(CYS_QUEUE_QUIET_SECS).
fn queue_quiet_secs() -> u64 {
    std::env::var("CYS_QUEUE_QUIET_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3)
}

/// 큐 적체 경보 임계 — 배달 못 한 채 depth가 이 값 이상이면 `queue.depth_high` 이벤트
/// (기본 5 · CYS_QUEUE_DEPTH_ALERT, 0=비활성). master가 working 중이라 조용해지지 않으면
/// 보고가 무음 적체된다(2026-06-12 실측 depth 9~12) — 침묵 대신 결정론 경보로 드러낸다.
fn queue_depth_alert_threshold() -> usize {
    std::env::var("CYS_QUEUE_DEPTH_ALERT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(5)
}

const QUEUE_ALERT_COOLDOWN_SECS: f64 = 300.0;

/// C5 hard tier·TTL 요약 OOB 통지의 쿨다운(30분). 이벤트 경보(5분)보다 훨씬 길다 —
/// OOB는 상대 stdin을 점유하는 침습적 채널이라 잦으면 그 자체가 소음이 된다.
/// **자동 통지 전용**이다 — 사람이 명시적으로 발행하는 요청에 쓰면 안 된다(아래 참조).
pub(crate) const OOB_HARD_COOLDOWN_SECS: f64 = 1800.0;

/// `queue.request_clear` 전용 쿨다운(60초).
///
/// ★R1 의미론 정직화: 종전에는 사람이 친 `cys queue request-clear` 에 자동 통지용 1800초를
/// 그대로 물렸다. 그 결과 ①오퍼레이터가 재발행해도 30분간 조용히 무시됐고 ②`hint` 는
/// "다음 시도에서 재통지된다"고 안내했지만 **재통지를 돌리는 주기 잡이 없다**(자동 재시도
/// 루프를 도는 depth_high·TTL 통지와 달리 이 경로는 1회성 RPC다) — 문언과 동작이 어긋났다.
/// 60초는 "연타 방지"라는 이 경로의 실제 목적에만 맞춘 값이다.
pub(crate) const OOB_REQUEST_CLEAR_COOLDOWN_SECS: f64 = 60.0;

/// ★E1(0.13.21) 적체 **다이제스트** 쿨다운 기본(초) — 대상 1명이 적체 통지를 받는 최소 간격.
///
/// 종전(B3)에는 같은 값이 키 무관 **전역 레이트캡**(`__global__` 예약 키)이었다. 그 구조는
/// 적체 노드별로 `depth_high_hard:{sid}` 를 따로 주입하는 설계를 전제로, 총량만 사후에 깎았다 —
/// 그래서 적체 노드가 2개 이상이면 전역캡(300s)과 소스 쿨다운(300s)이 동주기로 맞물려 위상이
/// 고정되고, 억제된 통지는 그대로 폐기되므로 **1개 외 전원이 영구 침묵**했다(적대 리뷰 BLOCK-1).
///
/// 지금은 틱마다 hard tier 노드를 먼저 **수집**해 대상별 **1통의 다이제스트**로 주입한다. 통지가
/// 노드 수와 무관하게 대상당 1건이므로 총량 상한이 곧 이 쿨다운이고, 창 안에 새로 적체된 노드는
/// 다음 다이제스트 본문에 실려 나간다 — 영구 침묵이 구조적으로 불가능하다.
///
/// `CYS_OOB_GLOBAL_MIN_SECS` 로 조정한다(이름은 B3 호환 유지). **0 = 쿨다운 없음**이다 —
/// 종전의 "0=전역캡 비활성"과 달리 이제는 틱마다 주입된다는 뜻이므로 진단 목적에만 쓴다.
pub(crate) const OOB_GLOBAL_MIN_SECS_DEFAULT: f64 = 300.0;

fn oob_global_min_secs() -> f64 {
    std::env::var("CYS_OOB_GLOBAL_MIN_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(OOB_GLOBAL_MIN_SECS_DEFAULT)
}

/// 적체 다이제스트의 dedup 키. **surface id 를 넣지 않는다** — 키에 sid 를 박는 순간 통지가
/// 노드별로 쪼개지고, 대상 1명이 맞는 총량을 다시 별도 캡으로 깎아야 하는 B3 구조로 되돌아간다.
/// 계열 추출이 `:` 앞을 자르므로 이 값 자체가 계열이며 B4 사이드카에 (role, 계열)로 영속된다.
pub(crate) const OOB_DEPTH_DIGEST_KEY: &str = "depth_high_digest";

/// dedup_key → 계열(`ttl_expired:12` → `ttl_expired`). 키에 박힌 surface id 는
/// 재기동 시 소멸하므로, 재기동을 넘겨 보존하는 원장(B4)은 계열 단위로만 의미가 있다.
/// sid 를 안 박는 키(적체 다이제스트)는 값 자체가 계열이 된다.
pub(crate) fn oob_key_class(dedup_key: &str) -> &str {
    dedup_key.split(':').next().unwrap_or(dedup_key)
}

/// B4 쿨다운 사이드카. 큐 WAL(v2)에 편승시키지 않은 이유: 큐 WAL 은 enqueue·pop 마다
/// 전량 재직렬화되는 뜨거운 경로라 쿨다운(드문 갱신)을 얹으면 서로의 실패가 전이된다.
pub const OOB_COOLDOWN_FILE: &str = "oob-cooldowns.json";

/// 원장 보존 상한(초). 현행 최장 쿨다운(1800s)의 2배 — 이보다 오래된 스탬프는 어떤 쿨다운도
/// 억제하지 못하므로 보관 자체가 무의미하다(파일·메모리 단조 증가 차단).
const OOB_COOLDOWN_MAX_AGE_SECS: f64 = 3600.0;

/// 사이드카를 읽어 (role, 계열) → 마지막 주입 시각 맵을 만든다. 부재·손상·만료는 조용히
/// 버린다 — 통지를 **막는** 원장이므로 못 읽었을 때의 안전한 방향은 "억제하지 않음"이다.
pub fn load_oob_cooldowns(dir: &std::path::Path) -> HashMap<(String, String), f64> {
    let mut out = HashMap::new();
    let Ok(content) = std::fs::read_to_string(dir.join(OOB_COOLDOWN_FILE)) else {
        return out;
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) else {
        return out;
    };
    let now = now_epoch();
    for e in v["entries"].as_array().into_iter().flatten() {
        let (Some(role), Some(class), Some(ts)) = (
            e["role"].as_str(),
            e["class"].as_str(),
            e["ts"].as_f64(),
        ) else {
            continue;
        };
        // 미래 시각(시계 역행·수기 편집)은 무한 억제가 되므로 버린다.
        if ts > now || now - ts > OOB_COOLDOWN_MAX_AGE_SECS {
            continue;
        }
        out.insert((role.to_string(), class.to_string()), ts);
    }
    out
}

/// 라이브 맵(surface 키)을 role 앵커로 환산해 사이드카에 원자적으로 쓴다.
/// 호출은 **스탬프가 갱신될 때만** — 주입 성공은 쿨다운 간격만큼 드문 사건이라 상시 IO 가 없다.
/// role 없는 surface 의 스탬프는 저장하지 않는다(재기동 후 매칭할 앵커가 없어 무의미하고,
/// role 없는 것끼리 한 바구니에 뭉쳐 과억제를 만든다).
///
/// ★메모리 상의 복원 원장은 **읽기 전용 gap-filler** 다: 라이브 스탬프가 생긴 (role,계열)은
/// 여기서 지운다. 라이브를 정본으로 두지 않으면 "라이브 쿨다운을 되돌렸는데도 복원분이 계속
/// 억제하는" 이중 진실이 생긴다(request_clear 재발행이 조용히 삼켜지던 A4 결함의 재발 경로).
fn persist_oob_cooldowns(daemon: &Arc<Daemon>) {
    let now = now_epoch();
    // ① 라이브 스탬프 스냅샷(락은 짧게) → ② role 해석 → ③ 파일용 병합 뷰 구성.
    let live: Vec<(u64, String, f64)> = {
        let mut cd = daemon.oob_cooldowns.lock().unwrap();
        cd.retain(|_, ts| now - *ts <= OOB_COOLDOWN_MAX_AGE_SECS);
        cd.iter().map(|((sid, k), ts)| (*sid, k.clone(), *ts)).collect()
    };
    let mut superseded: std::collections::HashSet<(String, String)> =
        std::collections::HashSet::new();
    let mut file_ledger = {
        let mut r = daemon.restored_oob_cooldowns.lock().unwrap();
        r.retain(|_, ts| now - *ts <= OOB_COOLDOWN_MAX_AGE_SECS);
        r.clone()
    };
    for (sid, key, ts) in live {
        let Some(role) = daemon.get_surface(sid).and_then(|s| s.role.lock().unwrap().clone()) else {
            continue;
        };
        let k = (role, oob_key_class(&key).to_string());
        superseded.insert(k.clone());
        // 같은 (role, 계열)에 여러 surface 가 걸리면 **가장 최근** 스탬프가 정본이다.
        let e = file_ledger.entry(k).or_insert(ts);
        if ts > *e {
            *e = ts;
        }
    }
    daemon
        .restored_oob_cooldowns
        .lock()
        .unwrap()
        .retain(|k, _| !superseded.contains(k));
    let entries: Vec<serde_json::Value> = file_ledger
        .iter()
        .map(|((role, class), ts)| json!({"role": role, "class": class, "ts": ts}))
        .collect();
    let content = json!({"schema_version": 1, "entries": entries});
    if let Ok(s) = serde_json::to_string(&content) {
        let dir = crate::state::state_dir(&daemon.socket_path);
        let _ = std::fs::create_dir_all(&dir);
        let _ = write_json_atomic(&dir, OOB_COOLDOWN_FILE, &s);
    }
}

/// OOB 주입이 배달되지 않은 **사유**. 종전 `bool` 은 "안 됐다"만 말할 수 있어서, 호출자가
/// 응답에 실을 안내를 추측으로 지어냈다(요청 경로의 잘못된 hint 의 근원). 사유를 값으로
/// 돌려 응답·이벤트가 사실만 말하게 한다.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OobSkip {
    /// 대상 부재·종료·에이전트 미등록·빈 좌석 — 주입하면 셸 명령으로 실행될 수 있는 상태.
    GateBlocked,
    /// 사람 입력 흔적이 아직 식지 않음.
    HumanTyping,
    /// 같은 (surface, dedup_key) 의 쿨다운 창 안.
    Cooldown,
    /// writer 채널 포화 — 쿨다운을 찍지 않으므로 즉시 재발행 가능.
    ChannelFull,
}

impl OobSkip {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            OobSkip::GateBlocked => "gate_blocked",
            OobSkip::HumanTyping => "human_typing",
            OobSkip::Cooldown => "cooldown",
            OobSkip::ChannelFull => "channel_full",
        }
    }
}

/// queued 배달의 '사람 입력 후 정지' 임계(초) — 기본 30초. 사람이 입력하다 3초+ 멈추면
/// quiet(출력 기준)만으로는 배달이 나가 미완성 입력에 이어붙거나(텍스트) 그대로 제출(Return)
/// 한다 — send_text 가드가 명명한 '최악 경로'의 재현(적대 검증 R1). 사람 흔적이 식은 뒤에만
/// 배달한다(CYS_QUEUE_HUMAN_QUIET_SECS로 조정).
fn queue_human_quiet_secs() -> u64 {
    std::env::var("CYS_QUEUE_HUMAN_QUIET_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(30)
}

// ─────────────────────────────────────────────────────────────────────────────
// C5 — OOB(out-of-band) 제어 레인
//
// 혼잡한 큐로 혼잡 경보를 보내면 경보 자체가 큐 뒤에 줄을 선다(순환). 그래서 제어 신호는
// 큐를 우회해 write_tx로 직접 주입한다 — 스케줄러가 이미 쓰는 경로(schedule.rs inject)의
// 일반화이며, 스케줄러와 달리 **human 가드를 존중**한다.
//
// 도달 의미론(D6): OOB 주입은 도달을 보장하지 않는다(수신 TUI가 스트리밍 중이면 다음 턴
// 입력으로 버퍼링된다 — 실측상 도달하나 계약은 아니다). 따라서 "1회 발사 후 기도"가 아니라
// **미해소 상태가 지속되면 쿨다운 경과 후 재통지**하는 자기치유 의미론으로 정의한다.
// ─────────────────────────────────────────────────────────────────────────────

/// OOB 통지 대상이 될 수 있는 지휘 role. **dept-master 포함이 필수**다 — 부서 데몬의
/// roles 맵에는 `master` 키가 없어서, {master,cso}만 조회하면 통지 대상이 공집합이 된다(R3).
const OOB_PRIVILEGED_ROLES: [&str; 3] = ["master", "cso", "dept-master"];

/// 큐를 우회해 대상 stdin에 제어 신호를 직접 주입한다. 배달 시 `Ok(())`,
/// 미배달 시 `Err(OobSkip)` — **사유가 값으로 나온다**(호출자가 안내를 지어내지 않게).
///
/// 게이트 3중:
/// ① **agent-present**(안전 필수): `agent_meta` 보유 ∧ 좌석이 Empty가 아닐 때만. 빈 좌석
///    bare shell에 주입하면 통지 문자열이 **셸 명령으로 실행**된다(Sim3-2 — 스케줄러가
///    schedule.rs에서 이미 지키는 가드와 동일 근거).
/// ② **human 가드**: 사람 입력 흔적이 식기 전이면 skip하고 다음 틱에 재시도한다
///    (미완성 입력에 이어붙이거나 그대로 제출시키는 최악 경로 차단).
/// ③ **쿨다운**: (surface, dedup_key)별. 쿨다운을 **성공했을 때만** 찍으므로, 주입 실패나
///    가드 skip은 다음 틱에 그대로 재시도된다.
/// ④ **재기동 생존 쿨다운(B4)**: ③의 맵은 surface_id 키라 재기동을 넘지 못한다 — role 앵커
///    원장(사이드카)을 같은 창으로 함께 본다.
///
/// ★E1: 종전의 키 무관 **전역 레이트캡**(`__global__`)은 제거됐다. 총량 상한은 이제 통지를
/// 합쳐서 내보내는 쪽(적체 다이제스트)이 구조적으로 갖는다 — 사후에 깎는 캡은 소스 쿨다운과
/// 동주기로 맞물려 위상을 고정시키고, 억제분이 폐기되면 그대로 영구 침묵이 됐다(BLOCK-1).
pub(crate) fn oob_notify(
    daemon: &Arc<Daemon>,
    sid: u64,
    text: &str,
    dedup_key: &str,
    cooldown_secs: f64,
) -> Result<(), OobSkip> {
    let Some(s) = daemon.get_surface(sid) else {
        return Err(OobSkip::GateBlocked);
    };
    if s.exited.load(Ordering::Relaxed) {
        return Err(OobSkip::GateBlocked);
    }
    // ① agent-present
    if s.agent_meta.lock().unwrap().is_none()
        || SeatState::from_u8(s.seat_cache.load(Ordering::Relaxed)) == SeatState::Empty
    {
        return Err(OobSkip::GateBlocked);
    }
    // ② human 가드
    let human_recent = s
        .last_human_input
        .lock()
        .unwrap()
        .map(|t| t.elapsed().as_secs() < queue_human_quiet_secs())
        .unwrap_or(false);
    if human_recent {
        return Err(OobSkip::HumanTyping);
    }
    // ③ 쿨다운
    let now = now_epoch();
    let key = (sid, dedup_key.to_string());
    let class = oob_key_class(dedup_key).to_string();
    let role = s.role.lock().unwrap().clone();
    {
        let cd = daemon.oob_cooldowns.lock().unwrap();
        if let Some(last) = cd.get(&key) {
            if now - last < cooldown_secs {
                return Err(OobSkip::Cooldown);
            }
        }
    }
    // ④ 재기동 생존분(role 앵커) — 라이브 스탬프가 없는 재기동 직후 창에서만 실효한다.
    if let Some(role) = role.as_deref() {
        let led = daemon.restored_oob_cooldowns.lock().unwrap();
        if let Some(last) = led.get(&(role.to_string(), class.clone())) {
            if now - last < cooldown_secs {
                return Err(OobSkip::Cooldown);
            }
        }
    }
    if s.write_tx
        .try_send(crate::state::WriteReq::Inject {
            text: text.to_string(),
            cr_delay_ms: 500,
            clear_first: false,
        })
        .is_err()
    {
        // 채널 포화 — 쿨다운을 찍지 않으므로 즉시/다음 틱에 재시도된다.
        return Err(OobSkip::ChannelFull);
    }
    daemon.oob_cooldowns.lock().unwrap().insert(key, now);
    // ★B4: 스탬프가 갱신됐을 때만 사이드카를 갱신한다(상시 IO 금지 — 주입 자체가 드문 사건).
    persist_oob_cooldowns(daemon);
    // 큐 배달과 마찬가지로 원격 주입이므로 에코 제외 창을 갱신한다.
    *s.last_injected.lock().unwrap() = Some(std::time::Instant::now());
    daemon.bus.publish(
        crate::queue_policy::queue_events::OOB_NOTIFIED,
        crate::queue_policy::queue_events::CATEGORY,
        Some(sid),
        json!({"dedup_key": dedup_key, "bytes": text.len(),
               "surface_ref": cys::surface_ref(sid)}),
    );
    Ok(())
}

/// 통지 수신 후보: 큐 소유 노드 + roles 맵에 **실재하는** 지휘 role. 중복 sid는 제거한다.
/// 전 대상이 게이트를 통과하지 못해도 이벤트·dead-letter는 남으므로 사실은 소실되지 않는다.
fn oob_targets(daemon: &Arc<Daemon>, owner_sid: u64) -> Vec<u64> {
    let mut out = vec![owner_sid];
    let roles = daemon.roles.lock().unwrap();
    for r in OOB_PRIVILEGED_ROLES {
        if let Some(sid) = roles.get(r).copied() {
            if !out.contains(&sid) {
                out.push(sid);
            }
        }
    }
    out
}

/// 한 틱에 hard tier 로 판정된 적체 노드 1건 — 다이제스트(E1)의 원소.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HardBacklog {
    pub sid: u64,
    pub role: Option<String>,
    pub depth: usize,
    pub agent_depth: usize,
    pub oldest_secs: u64,
    pub blocked_by: String,
}

/// 다이제스트 본문(순수) — 적체 노드 **전 목록**을 손실 없이 싣는다.
///
/// 한 줄로 만든다: 주입 텍스트의 개행은 수신 TUI에서 그대로 제출(Return)로 해석될 수 있어,
/// 여러 줄 본문은 첫 줄만 전달되고 나머지가 프롬프트에 흩뿌려진다(기존 통지 문구도 전부 단행).
pub(crate) fn depth_digest_text(hard: &[HardBacklog]) -> String {
    let list: Vec<String> = hard
        .iter()
        .map(|h| {
            format!(
                "{}(role={} depth={} agent={} oldest={}s 사유={})",
                cys::surface_ref(h.sid),
                h.role.as_deref().unwrap_or("-"),
                h.depth,
                h.agent_depth,
                h.oldest_secs,
                h.blocked_by,
            )
        })
        .collect();
    format!(
        "[QUEUE-BACKPRESSURE] 큐 적체 {}건 — {} · 각 노드의 큐를 점검하고 배달을 막는 원인을 \
         해소하라. 철회가 필요하면 `cys queue list --surface <ref>` 후 소유 노드가 \
         `cys queue clear` 집행.",
        hard.len(),
        list.join(" ; "),
    )
}

/// 수집된 hard tier 적체를 **대상별 1통**으로 주입한다(E1).
///
/// 대상 = 적체 노드들의 `oob_targets` 합집합(소유 노드 + master/cso/dept-master). 키가
/// `OOB_DEPTH_DIGEST_KEY` 하나뿐이라 (target,key) 쿨다운이 곧 대상당 총량 상한이고,
/// 창 안에 새로 적체된 노드는 다음 다이제스트 본문에 실린다 — 노드별 통지를 사후 캡으로
/// 깎던 B3 구조의 위상 고정(1개 외 전원 영구 침묵)이 구조적으로 성립하지 않는다.
fn notify_depth_digest(daemon: &Arc<Daemon>, hard: &[HardBacklog]) {
    if hard.is_empty() {
        return;
    }
    let text = depth_digest_text(hard);
    let cooldown = oob_global_min_secs();
    let mut targets: Vec<u64> = Vec::new();
    for h in hard {
        for t in oob_targets(daemon, h.sid) {
            if !targets.contains(&t) {
                targets.push(t);
            }
        }
    }
    for t in targets {
        let _ = oob_notify(daemon, t, &text, OOB_DEPTH_DIGEST_KEY, cooldown);
    }
}

/// 배달이 막힌 surface의 적체 경보 — quiet 미충족·human 흔적·pause 등 모든 '막힘' 분기에서
/// 공통 호출한다(한 분기라도 빠지면 그 사유의 적체가 침묵한다).
///
/// 두 레인이 **분리**돼 있다(E1):
/// - 이벤트 레인(`queue.depth_high`): 소스별 5분 쿨다운(`depth_alerted`).
/// - OOB 레인: 여기서는 **수집만** 하고 주입은 틱 말미 `notify_depth_digest` 가 한다.
///   수집을 이벤트 쿨다운 뒤에 두면 소스별 위상차가 그대로 다이제스트 누락으로 번역돼
///   BLOCK-1(위상 고정 침묵)이 재발하므로, 수집은 쿨다운 **앞**에 둔다.
fn alert_queue_depth_if_high(
    daemon: &Arc<Daemon>,
    s: &Arc<crate::state::Surface>,
    depth_alerted: &mut HashMap<u64, f64>,
    blocked_by: &str,
    hard_backlog: &mut Vec<HardBacklog>,
) {
    let threshold = queue_depth_alert_threshold();
    if threshold == 0 {
        return;
    }
    let depth = s.pending_queue.lock().unwrap().len();
    if depth < threshold {
        return;
    }
    // ★C5 hard tier: depth가 임계의 3배거나 Agent-origin 소프트캡에 닿았다 = 이벤트만으로는
    // 안 풀린다는 뜻이다(이벤트는 구독자가 봐야 의미가 있고, master는 지금 busy다).
    let softcap = crate::queue_policy::agent_softcap();
    let agent_depth = s
        .pending_queue
        .lock()
        .unwrap()
        .iter()
        .filter(|i| i.origin.is_agent())
        .count();
    let hard = depth >= threshold.saturating_mul(3) || (softcap > 0 && agent_depth >= softcap);
    let role = s.role.lock().unwrap().clone();
    if hard && !hard_backlog.iter().any(|h| h.sid == s.id) {
        hard_backlog.push(HardBacklog {
            sid: s.id,
            role: role.clone(),
            depth,
            agent_depth,
            oldest_secs: crate::queue_policy::oldest_age_secs(s).unwrap_or(0.0) as u64,
            blocked_by: blocked_by.to_string(),
        });
    }
    let now = now_epoch();
    let last = depth_alerted.get(&s.id).copied().unwrap_or(0.0);
    if now - last < QUEUE_ALERT_COOLDOWN_SECS {
        return;
    }
    depth_alerted.insert(s.id, now);
    // 손잡이 안내는 막힘 사유별로 — 공용 문구는 엉뚱한 env를 가리킨다(적대 검증 R2).
    let knob = if blocked_by.starts_with("human_typing") {
        "사람 입력이 식을 때까지 보류 중(CYS_QUEUE_HUMAN_QUIET_SECS)"
    } else if blocked_by.starts_with("queue_paused") {
        "헬스 조치(pause-queue) 해제가 대응 — 해당 surface 헬스 상태를 점검하라"
    } else if blocked_by.starts_with("empty_seat") {
        // ★SEAT: 이 사유는 '좌석에 에이전트가 없다'는 뜻이다 — 임계·quiet 노브로는 풀리지 않는다.
        // 조치는 좌석에 에이전트를 앉히는 것(부활·수동 연결)이므로 그 손잡이를 가리킨다.
        "좌석에 에이전트가 없다 — 그 pane 에서 직접 agent 를 실행하거나 `cys restore`(부활)로 \
         좌석을 채우면 보류분이 순서대로 배달된다(메시지는 보존 중·유실 아님)"
    } else {
        "임계 조정은 CYS_QUEUE_QUIET_SECS"
    };
    daemon.bus.publish(
        crate::queue_policy::queue_events::DEPTH_HIGH,
        crate::queue_policy::queue_events::CATEGORY,
        Some(s.id),
        json!({"depth": depth, "threshold": threshold, "blocked_by": blocked_by,
               "role": role, "agent_depth": agent_depth, "tier": if hard { "hard" } else { "soft" },
               "surface_ref": cys::surface_ref(s.id),
               "hint": format!("queued 배달이 막힌 채 적체 중 — read-screen으로 상태 점검, \
                                급한 보고는 직접 send(steer). {knob}")}),
    );
}

/// ★Phase 5 ①c: WAL로 살아난 restored_queue를 같은 role의 살아있는 surface로 재홈한다.
/// (Phase 3에서 restored_queue가 배달 경로에 미배선이라, 재기동 생존 메시지가 idle에도 미배달로
/// 잔존하던 갭을 닫는다 — role 앵커 재타겟.)
/// ★C3에서 deliver_queued 내부에서 **추출**됐다: 틱 순서를 rehome → expire → deliver로 고정해,
/// 복원분이 정식 surface에 안착한 뒤에만 만기 판정을 받게 하기 위함이다(Sim2-3).
fn rehome_restored(daemon: &Arc<Daemon>) {
    if daemon.rehome_restored_queue() > 0 {
        daemon.persist_queue_state();
    }
}

/// 큐 항목 TTL(초) — 기본 3600 · 0=비활성(`CYS_QUEUE_TTL_SECS`).
/// TTL은 **폐기가 아니라 이관** 기한이다: 만기 항목은 dead-letter에 전문이 남는다.
pub(crate) fn queue_ttl_secs() -> f64 {
    std::env::var("CYS_QUEUE_TTL_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3600.0)
}

/// ★B5(0.13.21) System-origin 큐 항목 TTL(초) — 기본 14400(4h) · 0=비활성
/// (`CYS_QUEUE_SYSTEM_TTL_SECS`). Agent(기본 1h)보다 **의도적으로 길다**: System 발신은
/// 스케줄러·거버넌스의 제어 메시지라 성급한 이관이 더 해롭다. 그럼에도 상한을 두는 이유는
/// 종전에 System 이 소프트캡·TTL 양쪽에서 면제라, 배달이 봉쇄된 pane(출력 중·사람 입력 창)
/// 앞에서 **무한 누적**됐기 때문이다(라이브 원장 1h+ 실증).
///
/// `CYS_QUEUE_TTL_SECS=0`(전면 롤백 스위치)이면 이 값과 무관하게 sweep 자체가 돌지 않는다 —
/// "TTL 0 = 아무것도 만기시키지 않는다"는 기존 계약을 System 확장이 깨뜨리면 안 된다.
pub(crate) fn queue_system_ttl_secs() -> f64 {
    std::env::var("CYS_QUEUE_SYSTEM_TTL_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(14400.0)
}

/// ★E4 재수정(0.13.21 최종 재검증) 사람의 GUI 큐잉(`System{label:"gui"}`) 전용 TTL(초) —
/// 기본 86400(24h) · **0=무기한 면제**(`CYS_QUEUE_GUI_TTL_SECS`).
///
/// 종전 이 등급은 무기한 면제였다. 그런데 `gui` 라벨은 클라이언트 자기신고(`human:true`)로
/// 붙으므로, 면제였을 때는 pane 밖 detach 프로세스가 한 줄로 **무기한 TTL 을 위조 획득**했다 —
/// 소프트캡 등급에서 막아둔 자기신고 우회가 TTL 등급에서 다시 열려 있었던 셈이다.
/// 유계 장주기 등급으로 바꾸면 위조의 이득 자체가 사라진다(얻는 것이 24h 뿐이라 위조할
/// 값어치가 없다). 사람 입력 보호는 유지된다: 24h 는 사람이 하루 안에 돌아오는 창을 덮고,
/// 만기는 폐기가 아니라 dead-letter 이관 + OOB 통지다(전문 무손실·소실 아님).
/// 진짜 무기한이 필요하면 `--important`(메시지 단위 의도 선언)나 이 env `0`(운영자 선언)이라는
/// **명시 경로**를 쓴다.
pub(crate) fn queue_gui_ttl_secs() -> f64 {
    std::env::var("CYS_QUEUE_GUI_TTL_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(86400.0)
}

/// 3등급 TTL 을 한 묶음으로 — 판정자(`QueueItem::effective_ttl`)가 등급표를 통째로 받는다.
///
/// 전면 롤백 스위치(`CYS_QUEUE_TTL_SECS=0`)를 **여기서 한 번에** 적용한다: Agent 가 0 이면
/// 세 등급 모두 0 이다. "TTL 을 끄면 아무것도 만기되지 않는다"는 약속을 등급 확장이 깨뜨리면
/// 안 되고, 그 판정이 호출자마다 흩어지면 한 호출자만 빠뜨렸을 때 조용히 뚫린다.
pub(crate) fn queue_ttls() -> crate::state::QueueTtls {
    let agent = queue_ttl_secs();
    if agent <= 0.0 {
        return crate::state::QueueTtls { agent: 0.0, system: 0.0, gui: 0.0 };
    }
    crate::state::QueueTtls {
        agent,
        system: queue_system_ttl_secs(),
        gui: queue_gui_ttl_secs(),
    }
}

/// C3 TTL sweep — 만기 항목을 큐에서 빼 dead-letter로 이관한다.
///
/// 만기 조건(현행 계약 · 판정 정본은 `QueueItem::is_expired`): `now - enqueued_at > 등급별 ttl`
/// ∧ `important` 미보유. 등급은 세 갈래다 —
/// - Agent: `CYS_QUEUE_TTL_SECS`(기본 1h).
/// - System(GUI 제외): `CYS_QUEUE_SYSTEM_TTL_SECS`(기본 4h). 종전 "System 면제"는
///   B5(0.13.21)에서 폐기됐다 — 배달이 봉쇄된 pane 앞에서 제어 메시지가 무한 누적됐기 때문이다.
/// - GUI(`System{label:"gui"}` = 사람의 GUI 큐잉): `CYS_QUEUE_GUI_TTL_SECS`(기본 **24h** ·
///   0=면제). E4 재수정 — 이 등급은 종전 **무기한 면제**였으나, 라벨이 클라이언트
///   자기신고(`human:true`)로 붙는 탓에 detach 프로세스가 무기한 TTL 을 위조 획득했다.
///   유계 장주기 등급으로 바꿔 위조 이득을 없애되, 사람 입력은 24h + 원장 + OOB 통지로 보호한다.
/// - **면제**: `--important` 선언분과 `Human`(WAL 유산 variant · 신규 생성 경로 없음).
/// `--important`는 메시지 단위 의도 선언이다 — role 일괄 면제는 사고 최다 발신자(CSO)를
/// 통째로 무기한화하므로 채택하지 않았다(Sim1-1).
///
/// 락 규율: surface별 pending_queue 락 안에서 **수집·제거만** 하고, dead-letter 기록·이벤트·
/// `persist_queue_state`는 락 해제 후에 한다(persist는 pending_queue 락 비보유가 전제조건).
///
/// ★기록 실패 시 되돌린다: 원장을 못 남긴 항목은 큐 **앞머리로 복원**한다 — 원장 없는 삭제 금지
/// (무손실 우선 fail-open). 실패 자체는 record_dead_letter가 health 경고로 발행한다.
fn expire_queued(daemon: &Arc<Daemon>) {
    let ttls = queue_ttls();
    let ttl = ttls.agent;
    if ttl <= 0.0 {
        return; // 전면 롤백 스위치 — System(B5)·GUI(E4) 등급도 함께 잠긴다.
    }
    let system_ttl = ttls.system;
    let gui_ttl = ttls.gui;
    let now = now_epoch();
    let surfaces: Vec<Arc<crate::state::Surface>> =
        daemon.surfaces.lock().unwrap().values().cloned().collect();
    let mut persist_needed = false;

    for s in surfaces {
        // ── 임계영역: 만기 후보 수집·제거 ──
        let expired: Vec<crate::state::QueueItem> = {
            let mut q = s.pending_queue.lock().unwrap();
            if q.iter().all(|i| !i.is_expired(now, ttls)) {
                continue;
            }
            let (out, keep): (Vec<_>, Vec<_>) = q.drain(..).partition(|i| i.is_expired(now, ttls));
            *q = keep.into();
            out
        };
        if expired.is_empty() {
            continue;
        }

        let role = s.role.lock().unwrap().clone();
        let mut moved: Vec<crate::state::QueueItem> = Vec::new();
        let mut restore: Vec<crate::state::QueueItem> = Vec::new();
        for item in expired {
            match crate::queue_policy::record_dead_letter(
                daemon,
                s.id,
                role.as_deref(),
                &item,
                crate::queue_policy::DeadLetterReason::Ttl,
            ) {
                Ok(_) => moved.push(item),
                Err(_) => restore.push(item), // 원장 없는 삭제 금지 — 큐에 되돌린다.
            }
        }
        if !restore.is_empty() {
            let mut q = s.pending_queue.lock().unwrap();
            for item in restore.into_iter().rev() {
                q.push_front(item);
            }
        }
        if moved.is_empty() {
            continue;
        }
        persist_needed = true;
        let remaining = s.pending_queue.lock().unwrap().len();
        daemon.bus.publish(
            crate::queue_policy::queue_events::EXPIRED,
            crate::queue_policy::queue_events::CATEGORY,
            Some(s.id),
            serde_json::json!({
                "count": moved.len(), "ttl_secs": ttl, "remaining": remaining,
                // ★B5: System 등급 TTL 과 등급별 건수 — 어느 상한이 걸렸는지 이벤트만 보고 안다.
                "system_ttl_secs": system_ttl,
                // ★E4 재수정: GUI 는 이제 면제가 아니라 자기 등급(기본 24h)이라, 이 상한이
                // 걸렸는지도 이벤트에서 읽혀야 한다(면제 시절엔 셀 것 자체가 없었다).
                "gui_ttl_secs": gui_ttl,
                "agent_count": moved.iter().filter(|i| i.origin.is_agent()).count(),
                "system_count": moved.iter().filter(|i| !i.origin.is_agent()).count(),
                "gui_count": moved.iter()
                    .filter(|i| matches!(&i.origin,
                        crate::state::QueueOrigin::System { label }
                            if label == crate::state::SYSTEM_LABEL_GUI))
                    .count(),
                "role": role, "surface_ref": cys::surface_ref(s.id),
                "oldest_age_secs": moved.iter().map(|i| now - i.enqueued_at)
                    .fold(0.0f64, f64::max),
                "bytes": moved.iter().map(|i| i.bytes()).sum::<usize>(),
                "hint": "만기 항목은 폐기가 아니라 dead-letters.jsonl로 이관됐다(전문 무손실). \
                         등급은 3종 — Agent=CYS_QUEUE_TTL_SECS, System=더 긴 \
                         CYS_QUEUE_SYSTEM_TTL_SECS, 사람의 GUI 큐잉(system 라벨 gui)=장주기 \
                         CYS_QUEUE_GUI_TTL_SECS(기본 24h·0=면제). 무기한 면제는 --important \
                         선언분뿐이다.",
            }),
        );
        // ★C5: 만기 요약을 대상 노드·지휘 role에 OOB로 통지한다. 이관은 조용히 일어나면
        // 안 된다 — 발신자는 배달을 기다리고 있고, 원장을 뒤져보라는 신호가 필요하다.
        let oldest = moved.iter().map(|i| now - i.enqueued_at).fold(0.0f64, f64::max) as u64;
        let text = format!(
            "[QUEUE-TTL] {} (role={}) 대기 메시지 {}건이 TTL(agent {}s·system {}s·gui {}s) 만기로 \
             dead-letters.jsonl에 이관됐다(전문 보존·유실 아님). 최고령 {oldest}s. 원장을 확인하고 \
             필요한 건만 다시 처리하라 — 발신자에게 재전송을 요구하지 마라.",
            cys::surface_ref(s.id),
            role.as_deref().unwrap_or("-"),
            moved.len(),
            ttl as u64,
            system_ttl as u64,
            gui_ttl as u64,
        );
        let key = format!("ttl_expired:{}", s.id);
        for target in oob_targets(daemon, s.id) {
            let _ = oob_notify(daemon, target, &text, &key, OOB_HARD_COOLDOWN_SECS);
        }
    }
    if persist_needed {
        daemon.persist_queue_state();
    }
}

/// 인플라이트 큐 배달자: 대상 surface가 quiet 임계(기본 3초) 이상 조용하면 큐에서 한 건 주입.
/// 연속 배달은 다음 틱 — 메시지 사이 자연 간격이 생겨 에이전트가 한 건씩 소화한다.
/// 배달이 막힌 채 적체되면(depth ≥ 임계) `queue.depth_high`를 쿨다운(5분)으로 발행한다.
fn deliver_queued(daemon: &Arc<Daemon>, depth_alerted: &mut HashMap<u64, f64>) {
    // T4-15 kill-switch: pause 중에는 큐 배달 동결 (메시지는 보존 — resume 시 재개)
    if daemon.paused.load(Ordering::Relaxed) {
        return;
    }
    let surfaces: Vec<Arc<crate::state::Surface>> =
        daemon.surfaces.lock().unwrap().values().cloned().collect();
    // ★E1: 이 틱의 hard tier 적체를 모아 **말미에 대상별 1통**으로 통지한다(다이제스트).
    let mut hard_backlog: Vec<HardBacklog> = Vec::new();
    for s in surfaces {
        if s.exited.load(Ordering::Relaxed) {
            continue;
        }
        // T4-17 헬스 조치: pause-queue 발동 중인 surface는 배달 보류 — 적체는 침묵 금지
        if s.queue_paused_until
            .lock()
            .unwrap()
            .map(|t| t > std::time::Instant::now())
            .unwrap_or(false)
        {
            alert_queue_depth_if_high(
                daemon,
                &s,
                depth_alerted,
                "queue_paused(헬스 조치)",
                &mut hard_backlog,
            );
            continue;
        }
        // 아직 바쁨(출력 중) — steer는 즉시 전송이 담당, 큐는 기다린다.
        let quiet_for = s.last_output.lock().unwrap().elapsed().as_secs();
        if quiet_for < queue_quiet_secs() {
            alert_queue_depth_if_high(daemon, &s, depth_alerted, "busy(출력 중)", &mut hard_backlog);
            continue;
        }
        // 사람 입력 흔적이 식기 전 배달 금지 — 미완성 입력에 이어붙기/제출 차단(R1 MED-2).
        let human_recent = s
            .last_human_input
            .lock()
            .unwrap()
            .map(|t| t.elapsed().as_secs() < queue_human_quiet_secs())
            .unwrap_or(false);
        if human_recent {
            alert_queue_depth_if_high(
                daemon,
                &s,
                depth_alerted,
                "human_typing(사람 입력 직후)",
                &mut hard_backlog,
            );
            continue;
        }
        // ★SEAT 게이트(2026-07-17 실사고 수리): **role 좌석**인데 좌석이 비었으면(에이전트 없음)
        // 배달을 보류한다. 종전엔 quiet 이기만 하면 배달해, 빈 셸이 role 을 쥔 동안 리뷰어 verdict·
        // 워커 보고가 zsh 프롬프트에 문자로 타이핑돼 **보고가 증발**했다(surface:112 실측).
        //
        // 판정 기준을 'role 유무'로 둔 이유: role 좌석은 정의상 에이전트 자리이므로 'role 있는
        // surface'가 role-앵커 메시지의 실질 대상이다. role 없는 맨 셸의 `--queued` 자동화는
        // 종전 그대로 통과한다(무회귀).
        //
        // ★주석 정정(A7): C1 이후 `QueueItem` 은 origin(발신 분류·발신 surface·role)을 **보유한다** —
        // "pending_queue 는 텍스트만 담아 항목별 앵커를 구분할 수 없다"는 종전 설명은 더 이상
        // 사실이 아니다. 그럼에도 판정을 surface 단위 role 로 유지하는 것은 의도적 선택이다:
        // 좌석이 비었으면 그 좌석 앞 큐는 **항목 출처와 무관하게** 배달할 곳이 없다(주입하면
        // 셸 명령이 된다). origin 은 혼잡 정책(누가 보냈나)의 축이고, 이 게이트는 배달 안전
        // (어디로 가나)의 축이다 — 두 축을 섞지 않는다.
        //
        // Unknown(프로브 미도달)은 **배달**한다 — 현행 동작 유지(판정 실패가 전 큐를 멈추는
        // 새 장애를 만들지 않는다). 보류는 유실이 아니라 지연이며, 좌석에 에이전트가 앉으면
        // 순서대로 배달된다. 적체는 아래 기존 알림이 사유와 함께 가시화한다(침묵 적체 금지).
        if s.role.lock().unwrap().is_some()
            && SeatState::from_u8(s.seat_cache.load(Ordering::Relaxed)) == SeatState::Empty
        {
            alert_queue_depth_if_high(
                daemon,
                &s,
                depth_alerted,
                "empty_seat(좌석에 에이전트 미연결)",
                &mut hard_backlog,
            );
            continue;
        }
        // pop은 writer 채널 인계 성공 후에만 — 실패 시 메시지를 보존해 다음 틱에 재시도.
        // 블로킹 write·sleep은 surface 전용 writer 스레드가 수행하므로 watchdog은 멈추지 않는다.
        //
        // TOCTOU 차단: front 읽기·writer 인계·pop_front를 pending_queue 락 한 임계영역으로
        // 묶는다. queue.clear(handlers.rs)·close_surface는 같은 락으로 drain하므로, '읽고서
        // 인계하는' 사이에 끼어들 수 없다 — 사용자가 clear한 메시지가 그래도 PTY에 주입되는
        // 경합 창이 사라진다. try_send는 논블로킹(블로킹 write는 writer 스레드)이라 락 보유는
        // 순간이고 watchdog은 멈추지 않는다.
        let delivered = {
            let mut q = s.pending_queue.lock().unwrap();
            let Some(item) = q.front().cloned() else {
                continue;
            };
            // ReturnKey 항목의 text는 빈 문자열 — 종전 '빈 문자열 큐잉'과 동일한 빈 Inject + CR.
            let req = crate::state::WriteReq::Inject {
                text: item.text.clone(),
                cr_delay_ms: 400,
                clear_first: false, // queued 배달은 quiet 대기 후라 선정리 불필요(현행 동작 보존)
            };
            if s.write_tx.try_send(req).is_err() {
                continue; // 인계 실패 — 메시지 보존, 다음 틱 재시도
            }
            pop_delivered_head(&mut q, item.seq);
            Some((item, q.len()))
        };
        if let Some((item, remaining)) = delivered {
            // T4-17 에코 제외 창 — 큐 배달도 원격 주입이다
            *s.last_injected.lock().unwrap() = Some(std::time::Instant::now());
            daemon.bus.publish(
                crate::queue_policy::queue_events::DELIVERED,
                crate::queue_policy::queue_events::CATEGORY,
                Some(s.id),
                serde_json::json!({"bytes": item.bytes(), "remaining": remaining,
                                   "seq": item.seq, "kind": item.kind.as_str(),
                                   "origin_class": item.origin.class()}),
            );
            // P7 큐 WAL: 배달로 줄어든 큐를 디스크에 반영(스냅샷 최신화).
            daemon.persist_queue_state();
        }
    }
    // ★E1: 틱 말미 1회 — 적체 노드가 몇 개든 대상 1명에게 가는 통지는 1통이다.
    notify_depth_digest(daemon, &hard_backlog);
}

#[cfg(test)]
mod tests {
    use super::{learn_stuck_candidates, merged_approval_patterns, plan_duplicate_kills};

    /// ★W-B 보완 핀: agents.json user 동결이 vendor 신규 approval_patterns 를 못 받아 승인
    /// 미감지→워커 hang 으로 가는 경로를 차단한다. 규칙 = 합집합(디스크 ∪ 임베드), 동명은 디스크 승.
    #[test]
    fn approval_patterns_union_disk_wins_vendor_fills() {
        let disk = serde_json::json!({
            "claude": { "approval_patterns": [
                { "name": "tool-permission", "pattern": "MY-CUSTOM-REGEX" }
            ]}
        });
        let embed = serde_json::json!({
            "claude": { "approval_patterns": [
                { "name": "tool-permission", "pattern": "VENDOR-OLD" },
                { "name": "new-vendor-prompt", "pattern": "VENDOR-NEW" }
            ]},
            "codex": { "approval_patterns": [{ "name": "codex-approve", "pattern": "CX" }]}
        });
        let merged = merged_approval_patterns(&disk, &embed, "claude");
        assert_eq!(merged.len(), 2, "동명 dedup + vendor 신규 1건 보강: {merged:?}");
        let mine = merged.iter().find(|p| p["name"] == "tool-permission").unwrap();
        assert_eq!(mine["pattern"], "MY-CUSTOM-REGEX", "동명 충돌은 디스크(사용자) 승");
        assert!(merged.iter().any(|p| p["name"] == "new-vendor-prompt"), "vendor 신규 패턴 도달(hang 방지)");
        // 디스크에 아예 없는 어댑터 → 임베드 전량 폴백(신규 CLI 지원 즉시 유효).
        let cx = merged_approval_patterns(&disk, &embed, "codex");
        assert_eq!(cx.len(), 1, "디스크 결손 어댑터는 임베드로 채움");
        // 양쪽 모두 없음 → 빈 벡터(무해 — 호출측이 continue).
        assert!(merged_approval_patterns(&disk, &embed, "nosuch").is_empty());
    }

    /// (learn gaps C12②) stuck 디바운스 지속화 — 저장→로드 왕복 + 부재/손상 fail-open 핀.
    /// 데몬 재시작 후에도 CYS_RSI_STUCK_DEBOUNCE_SECS 창이 유지되는 토대(소실=추천 중복 발화).
    #[test]
    fn learn_stuck_debounce_persistence_roundtrip() {
        let dir = std::env::temp_dir().join(format!("cys_learn_debounce_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        let sock = dir.join("cysd.sock");
        // 실제 저장 위치는 state_dir 파생(unix=소켓 부모·Windows=LOCALAPPDATA 슬러그) —
        // 플랫폼 중립으로 state_dir 경유로 정리·손상 주입한다.
        let sfile = crate::state::state_dir(&sock).join(super::LEARN_STUCK_DEBOUNCE_FILE);
        let _ = std::fs::create_dir_all(sfile.parent().unwrap());
        let _ = std::fs::remove_file(&sfile);
        // 부재 = 빈 맵(fail-open)
        assert!(super::load_learn_stuck_debounce(&sock).is_empty());
        let mut m = std::collections::HashMap::new();
        m.insert(7u64, 1_700_000_000.5f64);
        m.insert(12u64, 1_700_000_100.0f64);
        super::save_learn_stuck_debounce(&sock, &m);
        assert_eq!(super::load_learn_stuck_debounce(&sock), m, "저장→로드 왕복 보존");
        // 손상 = 빈 맵(fail-open — 조용한 차단보다 추천 재발화가 안전측)
        std::fs::write(&sfile, "{corrupt").unwrap();
        assert!(super::load_learn_stuck_debounce(&sock).is_empty());
        let _ = std::fs::remove_file(&sfile);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// L3 재발방지 핀(2026-07-07 feed 189 폭주): 데몬 감지 항목의 surface 단위
    /// pending 판정·stale 스냅샷·멱등 해소 계약을 박제한다.
    #[test]
    fn daemon_approval_dedup_helpers_and_stale_clear() {
        let dir = std::env::temp_dir().join(format!("cys_feed_dedup_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let daemon = crate::state::Daemon::new(dir.join("cysd.sock"));

        assert!(!daemon.has_pending_daemon_approval(7));
        daemon.push_feed_notification(
            "approval",
            "claude 승인 대기 감지 (surface:7)",
            "Do you want to proceed?",
            Some(7),
        );
        assert!(daemon.has_pending_daemon_approval(7), "감지 직후 pending");
        assert!(!daemon.has_pending_daemon_approval(8), "타 surface 독립");

        let ids = daemon.pending_daemon_approvals(7);
        assert_eq!(ids.len(), 1);
        assert!(daemon.resolve_feed_item(&ids[0], "stale-cleared").is_some());
        assert!(!daemon.has_pending_daemon_approval(7), "해소 후 pending 소거");
        assert!(
            daemon.resolve_feed_item(&ids[0], "stale-cleared").is_none(),
            "중복 해소=None(멱등)"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// L2 escalation 핀: stall 임계 초과 pending 감지 항목은 approval.stalled를 항목당
    /// 정확히 1회 발행하고, 해소된 항목은 fired 집합에서 회수된다.
    #[test]
    fn approval_stall_fires_once_per_item() {
        let dir = std::env::temp_dir().join(format!("cys_stall_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let daemon = crate::state::Daemon::new(dir.join("cysd.sock"));
        let mut rx = daemon.bus.subscribe();
        daemon.push_feed_notification("approval", "claude 승인 대기 감지 (surface:7)", "b", Some(7));
        // 인위 노화: created_at을 임계(기본 300s) 밖으로 이동
        {
            let mut items = daemon.feed_items.lock().unwrap();
            items.last_mut().unwrap().created_at -= 400.0;
        }
        let mut fired = std::collections::HashSet::new();
        super::check_approval_stall(&daemon, &mut fired);
        super::check_approval_stall(&daemon, &mut fired); // 2회 호출해도
        let mut stalled_events = 0;
        while let Ok(ev) = rx.try_recv() {
            if ev["name"].as_str() == Some("approval.stalled") {
                stalled_events += 1;
                assert_eq!(ev["payload"]["surface_ref"].as_str(), Some("surface:7"));
            }
        }
        assert_eq!(stalled_events, 1, "항목당 1회만 발화");
        // 해소 후 fired 집합 회수
        let rid = daemon.pending_daemon_approvals(7).pop().unwrap();
        daemon.resolve_feed_item(&rid, "allow");
        super::check_approval_stall(&daemon, &mut fired);
        assert!(fired.is_empty(), "해소 항목 키 회수");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// L4 백로그 에지 판정 핀: 임계 교차 1회 발화·지속 무재발화·하강 재무장·0=비활성.
    #[test]
    fn feed_backlog_crossed_edge_fire_and_rearm() {
        use super::feed_backlog_crossed;
        let mut alerted = false;
        assert!(!feed_backlog_crossed(24, 25, &mut alerted));
        assert!(feed_backlog_crossed(25, 25, &mut alerted), "임계 도달 첫 교차 발화");
        assert!(!feed_backlog_crossed(180, 25, &mut alerted), "지속 중 재발화 없음");
        assert!(!feed_backlog_crossed(3, 25, &mut alerted), "하강 — 재무장(무발화)");
        assert!(feed_backlog_crossed(30, 25, &mut alerted), "재교차 재발화");
        let mut off = false;
        assert!(!feed_backlog_crossed(999, 0, &mut off), "threshold=0 비활성");
    }

    /// (RSI 학습 자율추천 i) 막힘 판정 순수 함수 — 임계·디바운스·비활성(threshold=0)을 박제한다.
    #[test]
    fn learn_stuck_candidates_threshold_and_debounce() {
        let mut counts: HashMap<u64, u32> = HashMap::new();
        counts.insert(10, 3); // 임계 도달
        counts.insert(11, 2); // 임계 미달
        counts.insert(12, 5); // 임계 초과지만 디바운스 쿨다운 내
        let mut deb: HashMap<u64, f64> = HashMap::new();
        deb.insert(12, 1000.0); // 최근 추천 → 쿨다운(3600) 내
        let now = 2000.0;
        // threshold=3, cooldown=3600: 10만 후보(11=미달, 12=쿨다운 내)
        assert_eq!(learn_stuck_candidates(&counts, &deb, 3, 3600.0, now), vec![10]);
        // 쿨다운 경과 후엔 12도 포함(정렬)
        assert_eq!(learn_stuck_candidates(&counts, &deb, 3, 3600.0, 5000.0), vec![10, 12]);
        // threshold=0 = 비활성(보수적 옵트아웃)
        assert!(learn_stuck_candidates(&counts, &deb, 0, 3600.0, now).is_empty());
    }

    /// ★불변식 박제: 45초/3개 중복-kill 정책의 최古보존·나이게이트·결정론정렬을 핀한다.
    /// (check_surfaces에서 순수화 — sys 부재 시 mock 불가 회귀를 단위로 잡는다)
    #[test]
    fn plan_duplicate_kills_age_gate_and_keeps_oldest() {
        let now = 1000.0;
        // 입력을 일부러 pid 역순으로 — 내부 정렬이 깨지면 다른 pid를 죽인다(불변식 ③).
        let ages = vec![(30, 900.0), (10, 800.0), (20, 950.0)];
        // min_age=45: 10(나이200)·30(나이100) kill 적격, 20(나이50)도 적격, 최古 10 보존.
        let (kept, killed) = plan_duplicate_kills(ages, now, 45.0);
        assert_eq!(kept, 10, "최古(가장 낮은 pid) 1개는 항상 보존");
        assert_eq!(killed, vec![20, 30], "나머지 중 45초+ 산 것만, pid asc 결정론");
    }

    #[test]
    fn plan_duplicate_kills_spares_young_processes() {
        let now = 1000.0;
        // 20은 now-980=20s < 45 → 빌드 중 잠깐 뜬 정상 프로세스로 보존(불변식 ②).
        let ages = vec![(10, 800.0), (20, 980.0), (30, 940.0)];
        let (kept, killed) = plan_duplicate_kills(ages, now, 45.0);
        assert_eq!(kept, 10);
        assert_eq!(killed, vec![30], "20은 45초 미만이라 보존, 30(나이60)만 kill");
    }

    #[test]
    fn plan_duplicate_kills_boundary_exactly_min_age() {
        let now = 1000.0;
        // 경계: now-start == min_age(45)는 `>=`이므로 kill 적격(alerts.rs `>=` 경계와 정합).
        let ages = vec![(10, 500.0), (20, 955.0)];
        let (kept, killed) = plan_duplicate_kills(ages, now, 45.0);
        assert_eq!(kept, 10);
        assert_eq!(killed, vec![20], "정확히 45초는 kill 적격(>=)");
    }

    /// ★불변식 박제(2026-06-12 실측 결함): npm 래퍼 에이전트의 모든 실행 형태가 생존으로
    /// 매칭돼야 한다 — 놓치면 agent_alive=false 오판 → orchestra check FAIL → 멀쩡한
    /// 노드를 수선·오살(quit·close-surface)하는 연쇄가 재발한다.
    #[test]
    fn cmdline_matches_agent_covers_npm_wrapper_forms() {
        use super::cmdline_matches_agent as m;
        // gemini의 실존 3형태: bin 심링크 직접 / node 옵션 끼움 + .js 번들 / 패키지 경로 실행
        assert!(m("node /Users/user/.npm-global/bin/gemini", "gemini"));
        assert!(m(
            "node --no-warnings /Users/user/.npm-global/lib/node_modules/@google/gemini-cli/bundle/gemini.js",
            "gemini"
        ));
        assert!(m(
            "node /usr/local/lib/node_modules/@google/gemini-cli/dist/index.js --model x",
            "gemini"
        ));
        // 단일 실행파일 에이전트 (기존 동작 회귀 없음)
        assert!(m("claude --dangerously-skip-permissions", "claude"));
        assert!(m("codex --dangerously-bypass-approvals-and-sandbox", "codex"));
        // 비매치: 무관 프로세스 / 단어 인자(비경로)는 패키지 접두 오탐 금지
        assert!(!m("vim notes.md", "gemini"));
        assert!(!m("python3 train.py gemini-style-arg", "gemini"));
        assert!(!m("zsh -il", "claude"));
        assert!(!m("", "gemini"));
        assert!(!m("node /x/y.js", ""));
        // 유사명 패키지·디렉터리는 생존 아님 — `<bin>-cli`·`<bin>-code` 정확 일치만
        // 패키지 세그먼트로 인정(죽음 은폐 → node-recover 거부 역결함 차단, 적대 검증 R1·R2)
        assert!(!m("node /opt/claude-code-router/index.js", "claude"));
        assert!(!m("/a/grok-1-weights/loader.js", "grok"));
        assert!(!m("tail -f logs/claude-archive/x.log", "claude"));
        assert!(m("node /n/m/@google/gemini-cli/bundle/x.js", "gemini"));
        assert!(m("node /n/m/@anthropic-ai/claude-code/cli.js", "claude"));
        // 옵션이 3토큰을 넘겨도(구 규칙의 사각) 잡는다
        assert!(m(
            "node --max-old-space-size=4096 --enable-source-maps --no-deprecation /n/m/@google/gemini-cli/bundle/gemini.js",
            "gemini"
        ));
    }

    use super::{
        collect_scoped_for_shutdown, pop_delivered_head,
        prune_surface_health_keys, prune_watchdog_debounce_maps, LOAD_DEBOUNCE_SECS,
    };
    use crate::state::LedgerEntry;
    use std::collections::{HashMap, HashSet, VecDeque};
    #[cfg(windows)]
    use std::path::PathBuf;
    use std::time::Instant;

    fn entry(pid: u32, pgid: i32, scoped: bool) -> LedgerEntry {
        LedgerEntry {
            pid,
            pgid,
            cmd: "x".into(),
            surface_id: Some(1),
            scoped,
            registered_at: 0.0,
            caps: None,
            health: crate::state::ProcessHealth::Reusable,
        }
    }

    // ── T5-2: 무음 크래시 술어 (ack 후 N초 내 후행 실패 헬스룰 = crash) ──
    // 주입 clock/events로 결정론 핀(실제 sleep·라이브 데몬 없음). 부작용0 순수함수.
    #[test]
    fn surface_crashed_predicate_window_semantics() {
        use super::surface_crashed;
        use serde_json::json;
        let mk = |sid: u64, ts: f64| json!({"surface_id": sid, "ts": ts, "rule": "panic", "line": "x"});
        let window = 10.0;

        // (1) ack(t=100) 후 윈도우 내(t=105) 실패 = crash.
        let mut rh = VecDeque::new();
        rh.push_back(mk(7, 105.0));
        assert!(surface_crashed(&rh, Some(100.0), 7, window), "ack 후 윈도우 내 실패 → crash");

        // (2) ack만 있고 실패 헬스룰 없음 = false.
        let empty: VecDeque<serde_json::Value> = VecDeque::new();
        assert!(!surface_crashed(&empty, Some(100.0), 7, window), "ack만 → not crash");

        // (3) 실패만 있고 ack 없음(last_ack=None) = false.
        let mut rh3 = VecDeque::new();
        rh3.push_back(mk(7, 105.0));
        assert!(!surface_crashed(&rh3, None, 7, window), "ack 부재 → not crash");

        // (4) 윈도우 초과(t=120 > 100+10) = false.
        let mut rh4 = VecDeque::new();
        rh4.push_back(mk(7, 120.0));
        assert!(!surface_crashed(&rh4, Some(100.0), 7, window), "윈도우 초과 → not crash");

        // (5) ack 이전(t=95 <= ack) 실패는 후행 아님 = false.
        let mut rh5 = VecDeque::new();
        rh5.push_back(mk(7, 95.0));
        assert!(!surface_crashed(&rh5, Some(100.0), 7, window), "ack 이전 실패 → not crash");

        // (6) 타 surface(sid=8) 실패는 본 surface(7) 크래시 아님 = false.
        let mut rh6 = VecDeque::new();
        rh6.push_back(mk(8, 105.0));
        assert!(!surface_crashed(&rh6, Some(100.0), 7, window), "타 surface 실패 → not crash");
    }

    // ── T4-5B: 좀비 하트비트 — 연속 3회 ping 미스 시 좀비 정리 ──
    // 순수 술어 + 카운터 누적 의미(주입 카운트, 실제 sleep·라이브 데몬 없음).
    #[test]
    fn zombie_threshold_fires_on_third_miss() {
        use super::zombie_over_threshold;
        // 술어: 1·2회 미스는 좀비 아님, 3회째부터 좀비.
        assert!(!zombie_over_threshold(0));
        assert!(!zombie_over_threshold(1));
        assert!(!zombie_over_threshold(2));
        assert!(zombie_over_threshold(3), "3회 미스 = 좀비");
        assert!(zombie_over_threshold(4));

        // 카운터 누적 의미: half-open(자식 사망·exited 미설정)이 3틱 연속 누적되면 cleanup 후보.
        // reap_zombie_surfaces의 카운팅 본문과 동일한 누적·임계 판정을 순수하게 핀.
        let mut zombie_miss: HashMap<u64, u32> = HashMap::new();
        let mut cleanup_at: Option<u32> = None;
        for tick in 1..=3 {
            let missed = zombie_miss.entry(42).or_insert(0);
            *missed += 1; // half-open 미스 누적(살아있으면 remove로 리셋되는 경로)
            if zombie_over_threshold(*missed) && cleanup_at.is_none() {
                cleanup_at = Some(tick);
            }
        }
        assert_eq!(cleanup_at, Some(3), "정확히 3번째 미스에서 정리 트리거");

        // 살아있는 신호가 한 번이라도 오면 리셋 — half-open만 누적됨을 핀.
        zombie_miss.insert(99, 2);
        zombie_miss.remove(&99); // alive 분기의 reset
        assert!(!zombie_miss.contains_key(&99));
    }

    // ── T5-6 strand-2: 오염(Poisoned) 자식 풀 반환 금지 (재사용 후보 배제) ──
    // 비정상 종료 ledger 엔트리가 Poisoned로 마킹되면 is_reusable이 false를 돌려
    // 재사용 풀에서 배제된다. 기본(Reusable)은 재사용 가능. 순수함수 테스트 핀.
    #[test]
    fn poisoned_entry_is_excluded_from_reuse() {
        use crate::state::{is_reusable, ProcessHealth};
        let mut healthy = entry(100, 100, true);
        assert_eq!(healthy.health, ProcessHealth::Reusable);
        assert!(is_reusable(&healthy), "기본 Reusable 항목은 재사용 가능");
        healthy.health = ProcessHealth::Poisoned;
        assert!(!is_reusable(&healthy), "Poisoned 항목은 재사용 후보에서 배제");
    }

    // poison_surface_ledger가 해당 surface의 항목만 Poisoned로 마킹하고 타 surface는 불변.
    #[test]
    fn poison_marks_only_owning_surface_entries() {
        use crate::state::{is_reusable, LedgerEntry, ProcessHealth};
        let mk = |pid: u32, sid: u64| LedgerEntry {
            pid,
            pgid: pid as i32,
            cmd: "x".into(),
            surface_id: Some(sid),
            scoped: true,
            registered_at: 0.0,
            caps: None,
            health: ProcessHealth::Reusable,
        };
        let mut ledger: HashMap<u32, LedgerEntry> = HashMap::new();
        ledger.insert(100, mk(100, 1));
        ledger.insert(200, mk(200, 1));
        ledger.insert(300, mk(300, 2));
        // poison_surface_ledger의 본문과 동일한 순수 마킹(daemon 락 없이 핀).
        for entry in ledger.values_mut() {
            if entry.surface_id == Some(1) {
                entry.health = ProcessHealth::Poisoned;
            }
        }
        assert!(!is_reusable(&ledger[&100]));
        assert!(!is_reusable(&ledger[&200]));
        assert!(is_reusable(&ledger[&300]), "타 surface 항목은 불변");
    }

    // ── 종료 시 회수 대상 선별 회귀 가드 (크로스플랫폼 대칭 핵심) ──
    // unix SIGTERM/SIGINT 핸들러와 windows console-event 핸들러가 *동일하게* 이
    // 선별을 거쳐 scoped 그룹만 죽인다. 비-scoped(데몬이 생명주기를 책임지지 않는
    // 외부 프로세스)는 절대 회수 대상이 아니다. 이 선별이 windows에서 누락되면
    // (과거 버그: 핸들러 자체가 #[cfg(unix)]뿐) Ctrl-C·콘솔닫힘·셧다운 시 scoped
    // 자식 트리가 전부 고아로 남아 거버넌스 철학(고아 누적 차단)이 깨진다.
    #[test]
    fn collect_scoped_for_shutdown_picks_only_scoped_groups() {
        let mut ledger: HashMap<u32, LedgerEntry> = HashMap::new();
        ledger.insert(100, entry(100, 100, true)); // scoped → 회수
        ledger.insert(200, entry(200, 200, false)); // 비-scoped → 보존
        ledger.insert(300, entry(300, 300, true)); // scoped → 회수
        let mut picked = collect_scoped_for_shutdown(&ledger);
        picked.sort_unstable();
        assert_eq!(
            picked,
            vec![(100, 100), (300, 300)],
            "scoped만 (pid,pgid)로 회수 대상이 되고 비-scoped는 제외돼야 한다"
        );
    }

    // ── health 맵 무한 성장 회귀 가드 (state.rs run_health_rules가 insert) ──
    // 발견(medium): health_debounce·health_hits는 (surface_id, rule) 키로 insert만 되고
    // surface 종료 시 어디서도 회수되지 않아, surface를 계속 생성·종료하는 24/365 데몬에서
    // 죽은 surface별 (룰 수)개 엔트리가 단조 누적된다(caller_cache와 동일 계열 누수).
    // 이 테스트는 close_surface가 호출하는 회수 헬퍼가 ①닫힌 surface의 모든 rule 키를
    // 두 맵에서 제거하고 ②살아있는 다른 surface의 키는 한 건도 건드리지 않음을 박제한다.
    #[test]
    fn prune_surface_health_keys_evicts_only_closed_surface() {
        let mut debounce: HashMap<(u64, String), Instant> = HashMap::new();
        let mut hits: HashMap<(u64, String), Vec<f64>> = HashMap::new();
        // surface 1 (닫힐 대상): 두 룰에 매칭된 이력
        debounce.insert((1, "rate_limited".into()), Instant::now());
        debounce.insert((1, "auth_401".into()), Instant::now());
        hits.insert((1, "rate_limited".into()), vec![0.0, 1.0]);
        // surface 2 (생존): 보존돼야 한다
        debounce.insert((2, "rate_limited".into()), Instant::now());
        hits.insert((2, "auth_401".into()), vec![5.0]);

        prune_surface_health_keys(&mut debounce, &mut hits, 1);

        assert!(
            !debounce.keys().any(|(sid, _)| *sid == 1),
            "닫힌 surface 1의 debounce 키가 전부 회수돼야 한다(누수 차단)"
        );
        assert!(
            !hits.keys().any(|(sid, _)| *sid == 1),
            "닫힌 surface 1의 hits 키가 전부 회수돼야 한다(누수 차단)"
        );
        assert!(
            debounce.contains_key(&(2, "rate_limited".into())),
            "살아있는 surface 2의 debounce 키는 보존돼야 한다(오회수 금지)"
        );
        assert_eq!(
            hits.get(&(2, "auth_401".into())),
            Some(&vec![5.0]),
            "살아있는 surface 2의 hits 값은 그대로 보존돼야 한다(오회수 금지)"
        );
    }

    // 회수 대상이 없으면(닫힌 surface가 한 번도 health 룰에 매칭된 적 없음) no-op.
    #[test]
    fn prune_surface_health_keys_noop_when_surface_absent() {
        let mut debounce: HashMap<(u64, String), Instant> = HashMap::new();
        let mut hits: HashMap<(u64, String), Vec<f64>> = HashMap::new();
        debounce.insert((2, "rate_limited".into()), Instant::now());
        hits.insert((2, "rate_limited".into()), vec![1.0]);
        prune_surface_health_keys(&mut debounce, &mut hits, 99);
        assert_eq!(debounce.len(), 1, "무관 surface 회수는 다른 키를 건드리면 안 된다");
        assert_eq!(hits.len(), 1, "무관 surface 회수는 다른 키를 건드리면 안 된다");
    }

    // ── watchdog 태스크-로컬 디바운스/카운터 맵 무한 성장 회귀 가드 ──
    // 발견(medium): spawn_watchdog 루프의 4개 로컬 맵(last_dup_alert·last_proc_alert·
    // restart_counts·approval_debounce)이 insert만 하고 retain/remove가 없어, surface를
    // 계속 생성·종료하는(surface_id 단조 증가, 재사용 없음) 24/365 데몬에서 죽은 surface별
    // 엔트리와 무한 변종 cmdline 엔트리가 단조 누적된다(feed_reminded·todo_progress는 이미
    // retain 정리가 있는데 이들만 빠졌다). 이 테스트는 prune이 ①죽은 surface의 surface_id
    // 키를 세 맵에서 전부 제거하고 ②살아있는 surface 키는 한 건도 건드리지 않으며 ③cmdline
    // 키 맵은 디바운스 창을 넘긴 만료 엔트리만 비우고 창 안 엔트리는 보존함을 박제한다.
    #[test]
    fn prune_watchdog_maps_evicts_dead_surfaces_and_stale_cmdlines() {
        let now = 1_000_000.0_f64;
        let mut last_dup_alert: HashMap<String, f64> = HashMap::new();
        let mut last_proc_alert: HashMap<u64, f64> = HashMap::new();
        let mut restart_counts: HashMap<u64, u32> = HashMap::new();
        let mut approval_debounce: HashMap<(u64, String), f64> = HashMap::new();

        // surface 1 = 살아있음, surface 2 = 닫힘(live 집합에 없음)
        last_proc_alert.insert(1, now - 5.0);
        last_proc_alert.insert(2, now - 5.0);
        restart_counts.insert(1, 2);
        restart_counts.insert(2, 3);
        approval_debounce.insert((1, "allow".into()), now - 5.0);
        approval_debounce.insert((2, "allow".into()), now - 5.0);
        approval_debounce.insert((2, "yes".into()), now - 5.0);

        // cmdline 키: 만료(창 초과) vs 신선(창 안)
        last_dup_alert.insert("bun /tmp/aaa/server.ts".into(), now - LOAD_DEBOUNCE_SECS - 1.0);
        last_dup_alert.insert("bun /tmp/bbb/server.ts".into(), now - 1.0);

        let live: HashSet<u64> = [1u64].into_iter().collect();
        prune_watchdog_debounce_maps(
            &mut last_dup_alert,
            &mut last_proc_alert,
            &mut restart_counts,
            &mut approval_debounce,
            &live,
            now,
        );

        // 죽은 surface 2의 모든 키가 사라졌다.
        assert_eq!(last_proc_alert.get(&2), None, "죽은 surface proc_alert 회수");
        assert_eq!(restart_counts.get(&2), None, "죽은 surface restart_count 회수");
        assert!(
            !approval_debounce.keys().any(|(sid, _)| *sid == 2),
            "죽은 surface의 approval_debounce 키 전부 회수"
        );
        // 살아있는 surface 1의 키·값은 그대로다(오회수 금지).
        assert_eq!(last_proc_alert.get(&1), Some(&(now - 5.0)));
        assert_eq!(restart_counts.get(&1), Some(&2), "live surface 카운터 보존");
        assert_eq!(
            approval_debounce.get(&(1, "allow".into())),
            Some(&(now - 5.0)),
            "live surface approval_debounce 보존"
        );
        // 만료 cmdline은 비우고, 창 안 cmdline은 보존(디바운스 의미 보존).
        assert!(
            !last_dup_alert.contains_key("bun /tmp/aaa/server.ts"),
            "디바운스 창을 넘긴 cmdline 엔트리는 제거돼야 한다(누수 차단)"
        );
        assert!(
            last_dup_alert.contains_key("bun /tmp/bbb/server.ts"),
            "디바운스 창 안 cmdline 엔트리는 보존돼야 한다(잘못된 재발화 금지)"
        );
    }

    // 경계: 정확히 LOAD_DEBOUNCE_SECS 나이의 엔트리는 보존(fire 판정 `> 창`과 대칭 —
    // `<= 창`은 아직 디바운스 중이므로 비우면 안 된다).
    #[test]
    fn prune_watchdog_maps_keeps_cmdline_at_exact_debounce_boundary() {
        let now = 2_000_000.0_f64;
        let mut last_dup_alert: HashMap<String, f64> = HashMap::new();
        last_dup_alert.insert("svc".into(), now - LOAD_DEBOUNCE_SECS);
        let mut a: HashMap<u64, f64> = HashMap::new();
        let mut b: HashMap<u64, u32> = HashMap::new();
        let mut c: HashMap<(u64, String), f64> = HashMap::new();
        prune_watchdog_debounce_maps(
            &mut last_dup_alert,
            &mut a,
            &mut b,
            &mut c,
            &HashSet::new(),
            now,
        );
        assert!(
            last_dup_alert.contains_key("svc"),
            "정확히 창 경계 나이의 엔트리는 아직 디바운스 중이라 보존돼야 한다"
        );
    }

    #[test]
    fn collect_scoped_for_shutdown_empty_when_no_scoped() {
        let mut ledger: HashMap<u32, LedgerEntry> = HashMap::new();
        ledger.insert(1, entry(1, 1, false));
        assert!(
            collect_scoped_for_shutdown(&ledger).is_empty(),
            "scoped가 없으면 회수 대상도 없어야 한다 (외부 프로세스 오인 킬 금지)"
        );
        assert!(collect_scoped_for_shutdown(&HashMap::new()).is_empty());
    }

    /// ★C1: 큐가 String → QueueItem으로 승격되면서 테스트 픽스처도 항목을 만든다.
    fn qi(text: &str) -> crate::state::QueueItem {
        crate::state::QueueItem::text(
            text.to_string(),
            crate::state::QueueOrigin::system("test"),
        )
    }
    fn q(items: &[&str]) -> VecDeque<crate::state::QueueItem> {
        items.iter().map(|s| qi(s)).collect()
    }
    /// 단언 가독성용 — 큐의 텍스트만 뽑는다(seq는 발급마다 달라 값 비교 대상이 아니다).
    fn qtexts(deque: &VecDeque<crate::state::QueueItem>) -> Vec<String> {
        deque.iter().map(|i| i.text.clone()).collect()
    }

    // ── CYS_TODO_DIRS 파싱 회귀 가드 ──
    // ★W14 S18: 구현이 `cys::todo_scan::parse_todo_dirs`(lib 단일 구현)로 이관됐다.
    // 빈 항목 처리 회귀는 그 모듈의 단위 테스트가 갖고, 여기서는 **데몬이 그 구현을 쓰는지**만
    // 확인한다(재구현이 부활하면 이 두 테스트가 lib 구현을 검증하지 않게 되므로 함께 옮겼다).

    // Windows 드라이브 문자 콜론(`C:\…`)을 구분자로 오인하지 않아야 한다.
    // 구버전 `extra.split(':')`는 `C:\Users\x\_round`를 `C` + `\Users\x\_round`로
    // 쪼개 둘 다 존재하지 않는 경로로 만들어 워치를 무력화했다 — 이 테스트는
    // Windows 타깃에서만 의미가 있으므로 cfg(windows)로 가둔다.
    #[cfg(windows)]
    #[test]
    fn parse_todo_dirs_keeps_windows_drive_paths_intact() {
        let dirs = cys::todo_scan::parse_todo_dirs(r"C:\Users\x\_round;D:\proj\_round");
        assert_eq!(
            dirs,
            vec![
                PathBuf::from(r"C:\Users\x\_round"),
                PathBuf::from(r"D:\proj\_round"),
            ],
            "드라이브 문자 콜론을 구분자로 잘못 쪼개면 안 된다"
        );
    }

    #[test]
    fn pop_delivered_head_removes_matching_head() {
        // 정상 경로: 보낸 항목이 여전히 머리 → 제거. 뒤 메시지는 보존.
        let mut deque = q(&["msg1", "msg2"]);
        let head_seq = deque[0].seq;
        pop_delivered_head(&mut deque, head_seq);
        assert_eq!(qtexts(&deque), vec!["msg2".to_string()]);
    }

    #[test]
    fn pop_delivered_head_noop_on_empty_after_clear() {
        // lost-clear 시나리오: front 읽은 뒤 락이 풀린 창에서 queue.clear가 drain →
        // 빈 큐. 핵심은 '빈 큐를 건드리지 않고' 손상 없이 빠져나오는 것.
        // (이미 PTY로 간 메시지는 회수 불가 — 아키텍처 한계)
        let mut deque = q(&[]);
        pop_delivered_head(&mut deque, qi("msg1").seq);
        assert!(deque.is_empty());
    }

    #[test]
    fn pop_delivered_head_preserves_new_message_after_clear_and_enqueue() {
        // 유해 변종(이 수정의 핵심 회귀 가드): front(msgA) 읽고 락 해제 →
        // 그 창에서 clear가 drain([]) 후 새 메시지 "msgB" enqueue → 큐=["msgB"].
        // 무조건 pop_front이면 미배달 "msgB"를 삼켜 조용히 유실시킨다.
        // 머리가 보낸 항목이 아니므로 제거하지 않아야 한다 — "msgB"는 다음 틱에 배달.
        let delivered = qi("msgA");
        let mut deque = q(&["msgB"]);
        pop_delivered_head(&mut deque, delivered.seq);
        assert_eq!(
            qtexts(&deque),
            vec!["msgB".to_string()],
            "미배달 새 메시지가 유실되면 안 된다"
        );
    }

    #[test]
    fn pop_delivered_head_preserves_replacement_head() {
        // clear→enqueue가 여러 건이어도 머리 불일치면 한 건도 삼키지 않는다.
        let delivered = qi("msgA");
        let mut deque = q(&["msgB", "msgC"]);
        pop_delivered_head(&mut deque, delivered.seq);
        assert_eq!(qtexts(&deque), vec!["msgB".to_string(), "msgC".to_string()]);
    }

    /// ★C1 신설(T1): **동일 텍스트 다건**을 seq가 구별한다.
    /// 종전 텍스트 대조는 "같은 문자열이면 같은 항목"으로 취급해, 배달 직전 교체(C4 멱등)나
    /// 연속 Return 큐잉처럼 텍스트가 겹치는 상황에서 **미배달 항목을 삼킬 수 있었다**.
    /// 머리와 배달분의 텍스트가 완전히 같아도 seq가 다르면 제거하지 않아야 한다.
    #[test]
    fn pop_delivered_head_distinguishes_identical_text_by_seq() {
        let mut deque = q(&["같은 텍스트", "같은 텍스트"]);
        let stale_seq = qi("같은 텍스트").seq; // 큐에 없는 제3의 항목(텍스트만 동일)
        pop_delivered_head(&mut deque, stale_seq);
        assert_eq!(
            deque.len(),
            2,
            "텍스트가 같아도 seq가 다르면 한 건도 제거되면 안 된다"
        );
        let head_seq = deque[0].seq;
        pop_delivered_head(&mut deque, head_seq);
        assert_eq!(deque.len(), 1, "정확히 머리 항목만 제거돼야 한다");
    }

    // ── TOCTOU 회귀 가드: read-handoff-pop 단일 임계영역 ──
    // deliver_queued의 핵심 불변식을 production과 동일한 락 규율로 재현한다:
    // front 읽기·writer 인계·pop을 pending_queue 락 한 임계영역으로 묶으면,
    // 같은 락으로 drain하는 queue.clear/close_surface는 '읽고서 인계하는' 사이에
    // 끼어들 수 없다. 따라서 '주입된 메시지는 반드시 큐에서도 제거된 것'이고,
    // clear가 비운 메시지는 결코 writer로 가지 않는다.
    use std::sync::mpsc::sync_channel;
    use std::sync::{Arc, Mutex};

    // production deliver_queued의 임계영역과 동일한 순서:
    // 락 획득 → front().cloned() → try_send(writer) → pop_delivered_head → 락 해제.
    fn deliver_one_atomic(
        queue: &Mutex<VecDeque<crate::state::QueueItem>>,
        writer: &std::sync::mpsc::SyncSender<String>,
    ) -> Option<String> {
        let mut q = queue.lock().unwrap();
        let item = q.front().cloned()?;
        // 논블로킹 인계. 실패 시 메시지 보존(pop 안 함).
        if writer.try_send(item.text.clone()).is_err() {
            return None;
        }
        pop_delivered_head(&mut q, item.seq);
        Some(item.text)
    }

    #[test]
    fn deliver_is_atomic_against_concurrent_clear() {
        // clear(drain)와 deliver를 수천 회 경합시켜도, writer로 인계된 모든 메시지는
        // 큐에서 함께 제거된 것이어야 한다(주입=제거가 한 트랜잭션). 인계된 적 없는데
        // 사라진(clear가 비운) 메시지가 writer로 새는 일은 없어야 한다.
        for _round in 0..2000 {
            let queue = Arc::new(Mutex::new(q(&["only"])));
            // 용량 1 채널 — 인계 성공 = writer가 '주입할' 메시지를 받았다는 뜻.
            let (tx, rx) = sync_channel::<String>(1);

            let qc = Arc::clone(&queue);
            let clearer = std::thread::spawn(move || {
                // queue.clear / close_surface의 drain과 동일.
                let _: Vec<crate::state::QueueItem> = qc.lock().unwrap().drain(..).collect();
            });

            let delivered = deliver_one_atomic(&queue, &tx);
            clearer.join().unwrap();
            drop(tx);

            let injected: Vec<String> = rx.into_iter().collect();
            match delivered {
                // 인계 성공: 정확히 그 메시지가 writer로 갔고, 큐에는 남지 않았다.
                Some(text) => {
                    assert_eq!(injected, vec![text.clone()]);
                    assert!(
                        queue.lock().unwrap().is_empty(),
                        "주입된 메시지는 큐에서도 제거돼야 한다"
                    );
                }
                // clear가 먼저 이겨 큐가 비었으면 writer로 아무것도 가지 않았다 —
                // '사용자가 비운 메시지가 그래도 주입되는' 경합 창이 없다.
                None => assert!(
                    injected.is_empty(),
                    "clear가 비운 메시지가 writer로 새면 안 된다(TOCTOU)"
                ),
            }
        }
    }

    /// reap 경계: exited 후 grace 미만이면 보존(포렌식·복구 윈도우), 이상이면 회수.
    /// 역할 노드는 60초, 비역할은 10초로 더 빨리 정리 — 자력종료 surface 누수 차단의 핵심 불변식.
    #[test]
    fn exited_surface_due_respects_role_grace() {
        use super::exited_surface_due;
        // 역할 노드: 기본 60초 grace — 경계 직전 보존, 경계에서 회수
        assert!(!exited_surface_due(true, 59), "역할 노드는 grace 내(59s)에 보존돼야");
        assert!(exited_surface_due(true, 60), "역할 노드는 grace 경계(60s)에서 회수돼야");
        // 비역할(스크래치·one-shot): 기본 10초 grace — 더 빨리 정리
        assert!(!exited_surface_due(false, 9), "비역할은 grace 내(9s)에 보존돼야");
        assert!(exited_surface_due(false, 10), "비역할은 grace 경계(10s)에서 회수돼야");
    }

    // ─────────── ★묘비 게이트: reap≠묘비, owner-close=묘비 (부활 불변식) ───────────

    use super::{
        close_surface, load_tombstones_from_disk, load_tombstones_rev_from_disk, load_topology,
        now_epoch, persist_topology, reap_exited_surfaces, CloseCause, Daemon,
    };
    use serde_json::json;
    use std::sync::atomic::Ordering as AtomicOrdering;

    /// reap 계열 테스트는 CYS_REAP_EXITED* env를 만지므로 직렬화(다른 env-터치 테스트와 충돌 방지).
    static REAP_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// CYS_REAP_EXITED* env를 테스트 종료 시(패닉 포함) 이전 값으로 원복하는 가드 —
    /// 없던 값은 remove, 있던 값은 원복. 프로세스 전역 env 누수 차단.
    struct ReapEnvGuard {
        prev: Vec<(&'static str, Option<String>)>,
    }
    impl ReapEnvGuard {
        fn set(vars: &[(&'static str, &str)]) -> Self {
            let prev = vars
                .iter()
                .map(|(k, v)| {
                    let old = std::env::var(k).ok();
                    std::env::set_var(k, v);
                    (*k, old)
                })
                .collect();
            ReapEnvGuard { prev }
        }
    }
    impl Drop for ReapEnvGuard {
        fn drop(&mut self) {
            for (k, old) in &self.prev {
                match old {
                    Some(v) => std::env::set_var(k, v),
                    None => std::env::remove_var(k),
                }
            }
        }
    }

    /// 격리 데몬 — temp 소켓 디렉터리(개인 경로 하드코딩 금지).
    fn drill_daemon(tag: &str) -> Arc<Daemon> {
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = SEQ.fetch_add(1, AtomicOrdering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "cys-govdrill-{}-{}-{}-{}",
            tag,
            std::process::id(),
            now_epoch() as u64,
            n
        ));
        let _ = std::fs::create_dir_all(&dir);
        Daemon::new(dir.join("cysd.sock"))
    }

    /// 역할 보유 surface(live pid) 하나를 만들어 roles·surfaces에 등록하고 id 반환.
    fn spawn_role_surface(daemon: &Arc<Daemon>, role: &str) -> u64 {
        let s = daemon
            .create_surface(None, Some("sleep 30".into()), None, Some(role.into()), 24, 80)
            .expect("create surface");
        daemon.roles.lock().unwrap().insert(role.into(), s.id);
        daemon.surfaces.lock().unwrap().insert(s.id, s.clone());
        s.id
    }

    /// watchdog가 자력종료(exited) surface를 회수해도 역할을 묘비에 올리지 않는다 —
    /// phoenix가 desired_roster로 되살려야 하므로. 역할 매핑 정리는 여전히 일어나야 한다.
    #[test]
    fn reap_exited_does_not_tombstone_role() {
        let _g = REAP_ENV_LOCK.lock().unwrap();
        let daemon = drill_daemon("reap-exited");
        let id = spawn_role_surface(&daemon, "worker");
        // exited 마킹 + stamp(과거로 둘 필요 없음 — grace 0으로 즉시 회수 대상).
        let s = daemon.surfaces.lock().unwrap().get(&id).cloned().unwrap();
        s.exited.store(true, AtomicOrdering::Relaxed);
        *s.exited_at.lock().unwrap() = Some(std::time::Instant::now());
        let _env = ReapEnvGuard::set(&[
            ("CYS_REAP_EXITED", "1"),
            ("CYS_REAP_EXITED_GRACE_SECS", "0"),
        ]);
        reap_exited_surfaces(&daemon);

        assert!(
            !daemon.tombstones.lock().unwrap().contains("worker"),
            "reap된 역할이 묘비에 올랐다 — phoenix 부활이 영구 차단된다"
        );
        assert!(
            daemon.roles.lock().unwrap().get("worker").is_none(),
            "reap 후 역할 매핑이 남아 신규 claim을 막는다(정리 누락)"
        );
        // 디스크 라운드트립: topology.json에도 묘비가 없어야 phoenix가 되살린다.
        persist_topology(&daemon);
        assert!(
            !load_tombstones_from_disk(&daemon.socket_path).contains("worker"),
            "reap 묘비가 topology.json에 영속돼 재부팅 후 부활이 막힌다"
        );
    }

    /// ★W2/A-S1: tombstones_rev 는 묘비 집합이 실제 바뀔 때만 +1(단조), 무변경 persist 는 불변.
    /// topology.json 에 schema_version:1 + tombstones_rev 영속, disk 시드 라운드트립.
    #[test]
    fn tombstones_rev_increments_only_on_change() {
        use std::sync::atomic::Ordering;
        let daemon = drill_daemon("rev");
        let rev0 = daemon.tombstones_rev.load(Ordering::SeqCst);
        // 묘비 무변경 persist 2회 → rev 불변
        persist_topology(&daemon);
        persist_topology(&daemon);
        assert_eq!(daemon.tombstones_rev.load(Ordering::SeqCst), rev0, "무변경 persist 는 rev 불변");
        // 오너 close(묘비 삽입) → persist side-effect → rev +1
        let id = spawn_role_surface(&daemon, "worker");
        close_surface(&daemon, id, CloseCause::OwnerClose).expect("close");
        let rev1 = daemon.tombstones_rev.load(Ordering::SeqCst);
        assert_eq!(rev1, rev0 + 1, "묘비 삽입 시 rev +1");
        // 재persist(무변경) → rev 불변
        persist_topology(&daemon);
        assert_eq!(daemon.tombstones_rev.load(Ordering::SeqCst), rev1, "재persist 무변경 rev 불변");
        // topology.json 에 schema_version + tombstones_rev 영속
        let content = std::fs::read_to_string(
            crate::state::state_dir(&daemon.socket_path).join("topology.json"),
        )
        .unwrap();
        let v: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(v["schema_version"], 1);
        assert_eq!(v["tombstones_rev"].as_u64(), Some(rev1));
        // disk 시드 라운드트립
        assert_eq!(load_tombstones_rev_from_disk(&daemon.socket_path), rev1);
    }

    /// ★CU-7A 라운드트립: persist_topology 가 쓴 entries 에 `pid`·`pid_start_time` 이 실리고,
    /// 기존 키는 하나도 잃지 않는다(additive·INV-5). `schema_version` 은 **불변**이어야 한다 —
    /// 키 추가는 관용 파서 소비자(phoenix·묘비 리더·세대 스냅샷)를 깨지 않으므로 범프 대상이 아니다.
    #[test]
    fn topology_entries_carry_pid_additively() {
        let daemon = drill_daemon("cu7a-roundtrip");
        let id = spawn_role_surface(&daemon, "worker");
        let live_pid = daemon.surfaces.lock().unwrap().get(&id).cloned().unwrap().pid;
        persist_topology(&daemon);

        // ① 디스크 원본(직접 파싱)
        let content = std::fs::read_to_string(
            crate::state::state_dir(&daemon.socket_path).join("topology.json"),
        )
        .unwrap();
        let v: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(v["schema_version"], 1, "키 추가는 schema_version 을 범프하지 않는다");
        let e = v["entries"]
            .as_array()
            .and_then(|a| a.iter().find(|e| e["role"] == "worker"))
            .cloned()
            .expect("worker 엔트리가 topology 에 없다");
        assert_eq!(e["pid"].as_u64(), Some(live_pid as u64), "엔트리 pid 가 실제 셸 pid 와 다르다");
        assert!(
            e.get("pid_start_time").is_some(),
            "pid_start_time 키 자체가 없다(attest 대조 불가): {e}"
        );
        // 살아있으면 Some·죽었으면 None — 둘 다 같은 시점 재조회와 일치해야 한다(비플래키 등가 단정).
        assert_eq!(
            e["pid_start_time"].as_u64(),
            crate::state::peer_start_time(live_pid),
            "pid_start_time 이 실제 프로세스 start_time 과 불일치: {e}"
        );
        // 기존 키 무손실(additive 계약) — 하나라도 빠지면 restore 가 조용히 열화된다.
        for k in [
            "role",
            "agent",
            "agent_bin",
            "cwd",
            "title",
            "session_id",
            "claude_config_dir",
            "pack_reinject",
        ] {
            assert!(e.get(k).is_some(), "기존 키 {k} 가 사라졌다(additive 위반): {e}");
        }

        // ② 기존 읽기 경로(load_topology) 라운드트립 — 소비자가 실제로 쓰는 경로에서도 무손실.
        let loaded = load_topology(&daemon);
        let le = loaded
            .as_array()
            .and_then(|a| a.iter().find(|e| e["role"] == "worker"))
            .cloned()
            .expect("load_topology 가 worker 엔트리를 잃었다");
        assert_eq!(le["pid"].as_u64(), Some(live_pid as u64));
        assert_eq!(le["cwd"], e["cwd"], "load_topology 라운드트립에서 cwd 가 변형됐다");
    }

    /// ★CU-7A 하위호환: 구 topology.json(=pid·pid_start_time 키 없음)을 **기존 읽기 경로**로 읽어도
    /// 깨지지 않는다 — 부재는 null 로 보이고 소비자는 현행 동작으로 폴백한다(버전 스큐 안전·INV-5).
    #[test]
    fn legacy_topology_without_pid_loads_as_null() {
        let daemon = drill_daemon("cu7a-legacy");
        let dir = crate::state::state_dir(&daemon.socket_path);
        let legacy = json!({
            "schema_version": 1,
            "updated_at": 0,
            "entries": [{"role": "worker", "agent": "claude", "agent_bin": "claude",
                         "cwd": "/tmp", "title": "worker", "session_id": null,
                         "claude_config_dir": null, "pack_reinject": null}],
            "tombstones": [],
        });
        std::fs::write(dir.join("topology.json"), legacy.to_string()).unwrap();
        let loaded = load_topology(&daemon);
        let e = loaded
            .as_array()
            .and_then(|a| a.iter().find(|e| e["role"] == "worker"))
            .cloned()
            .expect("구 topology 의 worker 엔트리를 읽지 못했다");
        assert!(e["pid"].is_null(), "구 topology 의 pid 부재가 null 이 아니다: {e}");
        assert!(e["pid_start_time"].is_null(), "구 topology 의 pid_start_time 부재가 null 이 아니다: {e}");
        assert_eq!(e["cwd"], json!("/tmp"), "구 엔트리의 기존 키가 손상됐다");
    }

    /// ★W2/C3(데몬측 원자화): close 의 엔트리 제거 + 묘비 삽입이 **단일 persist_topology** 로 원자화된다
    /// (중간 persist 없음). 디스크 topology 한 파일에 entry 부재 + 묘비 존재가 함께 나타나야 한다(TOCTOU 차단).
    #[test]
    fn close_persists_entry_removal_and_tombstone_atomically() {
        let daemon = drill_daemon("c3-atomic");
        let id = spawn_role_surface(&daemon, "worker");
        close_surface(&daemon, id, CloseCause::OwnerClose).expect("close");
        let content = std::fs::read_to_string(
            crate::state::state_dir(&daemon.socket_path).join("topology.json"),
        )
        .unwrap();
        let v: serde_json::Value = serde_json::from_str(&content).unwrap();
        let has_worker = v["entries"]
            .as_array()
            .map(|a| a.iter().any(|e| e["role"] == "worker"))
            .unwrap_or(false);
        let tombs: Vec<String> = v["tombstones"]
            .as_array()
            .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
            .unwrap_or_default();
        assert!(!has_worker, "close 후 topology entries 에 worker 잔존(원자화 실패)");
        assert!(tombs.contains(&"worker".to_string()), "close 후 topology tombstones 에 worker 부재(원자화 실패)");
    }

    /// ★W2/P0-3: 손상 topology.json 은 조용한 빈집합이 아니라 `.corrupt-<ts>` isolate(원본 보존) — 폐역 역할
    /// 소실을 디스크에 확정하지 않는다. 격리 dir(스냅샷 없음)에선 빈 폴백이되 원본은 isolate.
    #[test]
    fn corrupt_topology_isolated_not_silently_empty() {
        let daemon = drill_daemon("p0-3");
        let dir = crate::state::state_dir(&daemon.socket_path);
        std::fs::write(dir.join("topology.json"), "{ corrupt ]]] not json").unwrap();
        let tombs = load_tombstones_from_disk(&daemon.socket_path);
        assert!(tombs.is_empty(), "격리 dir 스냅샷 없음 → 빈 폴백");
        let corrupt_isolated = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .any(|e| e.file_name().to_string_lossy().starts_with("topology.json.corrupt-"));
        assert!(corrupt_isolated, "손상 topology 가 .corrupt-* 로 isolate 되지 않음(조용한 소실)");
        assert!(!dir.join("topology.json").exists(), "손상 원본이 isolate 안 되고 그대로 남음");
    }

    /// ★W2/P0-3: 부재(fresh install)는 손상과 구분 — isolate 없이 빈집합(정상 부팅).
    #[test]
    fn missing_topology_is_empty_not_corrupt() {
        let daemon = drill_daemon("p0-3-missing");
        let dir = crate::state::state_dir(&daemon.socket_path);
        let _ = std::fs::remove_file(dir.join("topology.json"));
        let tombs = load_tombstones_from_disk(&daemon.socket_path);
        assert!(tombs.is_empty());
        let has_corrupt = std::fs::read_dir(&dir)
            .map(|rd| rd.flatten().any(|e| e.file_name().to_string_lossy().contains(".corrupt-")))
            .unwrap_or(false);
        assert!(!has_corrupt, "부재(fresh)를 손상으로 오판해 isolate 하면 안 된다");
    }

    /// 오너 의도적 닫기는 여전히 묘비를 남기고 영속한다(좀비 부활 차단 불변식 보존).
    #[test]
    fn owner_close_still_tombstones() {
        let daemon = drill_daemon("owner-close");
        let id = spawn_role_surface(&daemon, "worker");
        close_surface(&daemon, id, CloseCause::OwnerClose).expect("close");
        assert!(
            daemon.tombstones.lock().unwrap().contains("worker"),
            "오너 close가 묘비를 남기지 않았다 — auto-restore 좀비 부활 위험"
        );
        // 수동 persist 없이 디스크를 읽어 close_surface 자체의 persist_topology side effect를 실검증.
        assert!(
            load_tombstones_from_disk(&daemon.socket_path).contains("worker"),
            "오너 close 묘비가 topology.json에 영속되지 않았다"
        );
    }

    /// 데몬 재시작 동반사망 재현: 4역할 노드를 모두 reap로 회수하면 묘비가 하나도 안 남아
    /// phoenix가 4역할을 전부 자동부활할 수 있다(결정론 단위 재현).
    #[test]
    fn fleet_reap_leaves_roster_revivable() {
        let daemon = drill_daemon("fleet-reap");
        for role in ["cso", "worker", "reviewer-gemini", "reviewer-codex"] {
            let id = spawn_role_surface(&daemon, role);
            close_surface(&daemon, id, CloseCause::Reap).expect("reap close");
        }
        assert!(
            daemon.tombstones.lock().unwrap().is_empty(),
            "reap된 4역할 중 묘비가 남았다 — 함대 자동부활이 부분 차단된다"
        );
        // 4역할 매핑이 roles map에서 모두 제거돼야 phoenix가 desired_roster로 재claim 가능
        // (worker 단일 케이스와 동일 불변식 확장).
        {
            let roles = daemon.roles.lock().unwrap();
            for role in ["cso", "worker", "reviewer-gemini", "reviewer-codex"] {
                assert!(
                    roles.get(role).is_none(),
                    "reap 후 역할 매핑이 남았다({role}) — 신규 claim을 막아 부활이 차단된다"
                );
            }
        }
        // 수동 persist 없이 디스크를 읽어 close_surface 자체의 persist_topology side effect를 실검증.
        assert!(
            load_tombstones_from_disk(&daemon.socket_path).is_empty(),
            "topology.json에 reap 묘비가 영속돼 재부팅 후 4역할 부활이 막힌다"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// T3 (C3) — TTL 이관 · origin/important 면제 · rehome→expire 순서 · dead-letter 기록 ·
//           기록 실패 시 수용(보류) · 로테이션
// ─────────────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod queue_ttl_tests {
    use super::{expire_queued, queue_gui_ttl_secs, queue_ttls, rehome_restored};
    use crate::queue_policy;
    use crate::state::{Daemon, QueueItem, QueueOrigin, QueueWalEntry};
    use serde_json::Value;
    use std::sync::Arc;

    /// ★A7: TTL env 도 `CYS_QUEUE_*` 가족이다 — 모듈 사설 락이 아니라 **공유 락**을 잡는다.
    /// (queue_policy·handlers 테스트와 같은 락이어야 동시 set_var 경합이 실제로 막힌다.)
    use crate::queue_policy::QUEUE_ENV_LOCK as TTL_ENV_LOCK;

    struct TtlEnv;
    impl TtlEnv {
        fn set(secs: &str) -> Self {
            std::env::set_var("CYS_QUEUE_TTL_SECS", secs);
            TtlEnv
        }
    }
    impl Drop for TtlEnv {
        fn drop(&mut self) {
            std::env::remove_var("CYS_QUEUE_TTL_SECS");
            // ★B5: System 등급 TTL 도 같은 가족이다 — 패닉으로 빠져나가도 반드시 원복한다.
            std::env::remove_var("CYS_QUEUE_SYSTEM_TTL_SECS");
            // ★E4 재수정: GUI 등급도 같은 가족(면제 철회 후 신설).
            std::env::remove_var("CYS_QUEUE_GUI_TTL_SECS");
        }
    }

    fn daemon(tag: &str) -> (Arc<Daemon>, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "cys-qttl-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        (Daemon::new(dir.join("cysd.sock")), dir)
    }

    fn surface(d: &Arc<Daemon>, role: Option<&str>) -> Arc<crate::state::Surface> {
        let s = d
            .create_surface(None, Some("sleep 30".into()), None, role.map(|r| r.into()), 24, 80)
            .expect("create surface");
        d.surfaces.lock().unwrap().insert(s.id, s.clone());
        s
    }

    /// `age_secs` 만큼 과거에 적재된 항목.
    fn aged(text: &str, origin: QueueOrigin, age_secs: f64) -> QueueItem {
        let mut i = QueueItem::text(text.into(), origin);
        i.enqueued_at = crate::state::now_epoch() - age_secs;
        i
    }

    fn agent_origin() -> QueueOrigin {
        QueueOrigin::Agent { surface: 42, role: Some("worker-1".into()) }
    }

    fn dl_path(dir: &std::path::Path) -> std::path::PathBuf {
        crate::state::state_dir(&dir.join("cysd.sock")).join(queue_policy::DEAD_LETTER_FILE)
    }

    fn dead_letters(dir: &std::path::Path) -> Vec<Value> {
        std::fs::read_to_string(dl_path(dir))
            .unwrap_or_default()
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).expect("유효 JSON"))
            .collect()
    }

    fn texts(s: &Arc<crate::state::Surface>) -> Vec<String> {
        s.pending_queue.lock().unwrap().iter().map(|i| i.text.clone()).collect()
    }

    /// 만기 조건: 등급별 TTL 경과 ∧ !important ∧ 면제 origin 아님.
    ///
    /// ★R-3(적대 리뷰) 재작성: 종전 이 테스트는 Agent TTL 만 60s 로 걸고 System 등급은
    /// **기본값 14400** 에 맡겼다 — 120s 항목이 보존된 건 "등급이 분리돼서"가 아니라 "System
    /// 등급 상한이 4시간이라 아직 안 됐을 뿐"이라, 등급 분리가 통째로 사라져도 초록으로 남는
    /// 공허한 통과였다. 두 등급을 **모두 명시**하고 각 등급의 경계 양쪽을 함께 핀한다.
    #[test]
    fn ttl_expires_by_origin_tier_with_both_grades_pinned() {
        let _g = TTL_ENV_LOCK.lock().unwrap();
        let _e = TtlEnv::set("60"); // Agent 등급
        std::env::set_var("CYS_QUEUE_SYSTEM_TTL_SECS", "600"); // System 등급(명시)
        let (d, dir) = daemon("selectivity");
        let s = surface(&d, Some("master"));
        {
            let mut q = s.pending_queue.lock().unwrap();
            q.push_back(aged("만기 agent", agent_origin(), 120.0));
            q.push_back(aged("신선 agent", agent_origin(), 10.0));
            // 120s = Agent 등급(60)은 넘었고 System 등급(600)은 안 넘었다 — 등급이 실제로
            // 분리돼 있어야만 보존된다(등급 통합 회귀 시 이 줄이 먼저 깨진다).
            q.push_back(aged("등급차 보존 system", QueueOrigin::system("boot"), 120.0));
            q.push_back(aged("만기 system", QueueOrigin::system("boot"), 900.0));
            q.push_back(aged("만기 human", QueueOrigin::Human, 120.0));
            let mut imp = aged("만기 important", agent_origin(), 120.0);
            imp.important = true;
            q.push_back(imp);
        }
        expire_queued(&d);

        assert_eq!(
            texts(&s),
            vec![
                "신선 agent".to_string(),
                "등급차 보존 system".to_string(),
                "만기 human".to_string(),
                "만기 important".to_string(),
            ],
            "각 등급의 만기분만 이관되고 순서는 보존돼야 한다"
        );
        let dl = dead_letters(&dir);
        assert_eq!(dl.len(), 2, "이관 2건 = 원장 2줄");
        assert_eq!(dl[0]["text"], serde_json::json!("만기 agent"), "전문 무손실");
        assert_eq!(dl[1]["text"], serde_json::json!("만기 system"));
        assert!(dl.iter().all(|e| e["reason"] == serde_json::json!("ttl")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★E4(적대 리뷰 REVISE-5/6) + **재수정(최종 재검증)**: 사람의 GUI 큐잉
    /// (`System{label:"gui"}`)은 **자기 전용 장주기 등급**이다 — 무기한 면제가 **아니다**.
    ///
    /// D2 이후 실사람 입력은 `Human` variant 가 아니라 이 라벨로 들어온다. B5 가 System 전체에
    /// 4h 상한을 걸면서 정작 보호 대상이던 사람 입력이 제어 메시지와 같은 등급을 받았으므로
    /// 라벨로 갈라 자기 등급을 준다(여기까지가 E4).
    ///
    /// 재수정의 이유: 그 라벨은 **클라이언트 자기신고(`human:true`)** 로 붙는다. 등급이
    /// 무기한 면제였을 때는 pane 밖 detach 프로세스가 한 줄로 무기한 TTL 을 위조 획득할 수
    /// 있었다. 유계 장주기(기본 24h · `CYS_QUEUE_GUI_TTL_SECS`)로 바꾸면 위조 이득이 사라진다.
    ///
    /// 이 테스트가 핀하는 3사실: ①GUI 는 System 등급과 **분리**돼 더 길다 ②그래도 **상한이
    /// 있다**(자기 등급 경과 시 이관) ③`anonymous`(데몬 자식 자동 발신)는 GUI 등급을 못 받는다.
    #[test]
    fn gui_labeled_system_items_get_their_own_long_bounded_tier() {
        let _g = TTL_ENV_LOCK.lock().unwrap();
        let _e = TtlEnv::set("60"); // Agent 등급
        std::env::set_var("CYS_QUEUE_SYSTEM_TTL_SECS", "600"); // System 등급
        std::env::set_var("CYS_QUEUE_GUI_TTL_SECS", "6000"); // GUI 등급(명시 — 셋 다 다르게)
        let (d, dir) = daemon("gui-tier");
        let s = surface(&d, Some("master"));
        {
            let mut q = s.pending_queue.lock().unwrap();
            // System 등급(600)은 넘겼지만 GUI 등급(6000) 안 — 등급이 분리돼야만 보존된다.
            q.push_back(aged(
                "사람이 친 GUI 입력(등급차 보존)",
                QueueOrigin::system(crate::state::SYSTEM_LABEL_GUI),
                900.0,
            ));
            // GUI 등급도 넘기면 이관된다 — 면제가 아니라 상한이다(위조 이득 제거의 핵심).
            q.push_back(aged(
                "묵은 GUI 입력(등급 초과)",
                QueueOrigin::system(crate::state::SYSTEM_LABEL_GUI),
                9_000.0,
            ));
            q.push_back(aged("익명 자동 발신", QueueOrigin::system("anonymous"), 900.0));
        }
        expire_queued(&d);

        assert_eq!(
            texts(&s),
            vec!["사람이 친 GUI 입력(등급차 보존)".to_string()],
            "GUI 는 System 보다 긴 자기 등급을 받되(보존), 그 등급을 넘으면 이관된다(면제 아님)"
        );
        let dl = dead_letters(&dir);
        assert_eq!(dl.len(), 2, "이관 2건 = 원장 2줄(전문 무손실)");
        assert_eq!(dl[0]["text"], serde_json::json!("묵은 GUI 입력(등급 초과)"));
        assert_eq!(dl[0]["origin"]["label"], serde_json::json!("gui"));
        assert_eq!(dl[1]["text"], serde_json::json!("익명 자동 발신"));
        assert!(dl.iter().all(|e| e["reason"] == serde_json::json!("ttl")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★E4 재수정 — GUI 등급의 **기본값은 24h** 이고, `0` 은 운영자의 **명시 면제 선언**이다.
    ///
    /// 기본값 핀이 없으면 "유계로 바꿨다"는 계약이 env 를 거는 테스트에서만 참이 되어,
    /// 라이브 기본이 조용히 면제로 돌아가도 초록으로 남는다(E4 원 결함과 같은 형태의 공허 통과).
    #[test]
    fn gui_tier_defaults_to_24h_and_zero_means_exempt() {
        let _g = TTL_ENV_LOCK.lock().unwrap();
        let _e = TtlEnv::set("60");
        // GUI env 미설정 = 라이브 기본(86400).
        std::env::remove_var("CYS_QUEUE_GUI_TTL_SECS");
        assert_eq!(queue_gui_ttl_secs(), 86_400.0, "GUI 기본 등급은 24h 다");
        assert_eq!(queue_ttls().gui, 86_400.0);

        let (d, dir) = daemon("gui-default");
        let s = surface(&d, Some("master"));
        {
            let mut q = s.pending_queue.lock().unwrap();
            q.push_back(aged("23h GUI", QueueOrigin::system(crate::state::SYSTEM_LABEL_GUI), 82_800.0));
            q.push_back(aged("25h GUI", QueueOrigin::system(crate::state::SYSTEM_LABEL_GUI), 90_000.0));
        }
        expire_queued(&d);
        assert_eq!(texts(&s), vec!["23h GUI".to_string()], "기본 24h 안은 보존 · 넘으면 이관");
        assert_eq!(dead_letters(&dir).len(), 1);

        // 운영자가 0 을 명시하면 종전 무기한 면제로 되돌아간다(롤백 경로 보존).
        std::env::set_var("CYS_QUEUE_GUI_TTL_SECS", "0");
        let (d2, dir2) = daemon("gui-exempt-optin");
        let s2 = surface(&d2, Some("master"));
        s2.pending_queue.lock().unwrap().push_back(aged(
            "아주 묵은 GUI",
            QueueOrigin::system(crate::state::SYSTEM_LABEL_GUI),
            999_999.0,
        ));
        expire_queued(&d2);
        assert_eq!(texts(&s2).len(), 1, "CYS_QUEUE_GUI_TTL_SECS=0 은 명시 면제 선언이다");
        assert!(dead_letters(&dir2).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&dir2);
    }

    /// ★E4: 승인 대기 wakeup(`governance-approval`)은 **만기가 의도**다.
    /// 종전 주석은 이 항목을 "TTL 밖"이라 서술했지만 B5 이후 사실이 아니고, 그래야 master 가
    /// 죽은 채로 승인 알림이 무한 누적되는 것을 막는다. 만기는 폐기가 아니라 이관이므로
    /// 전문은 dead-letter 에 남는다(무손실) — 그 두 사실을 함께 핀한다.
    #[test]
    fn governance_approval_wakeup_expires_into_dead_letter() {
        let _g = TTL_ENV_LOCK.lock().unwrap();
        let _e = TtlEnv::set("3600");
        std::env::set_var("CYS_QUEUE_SYSTEM_TTL_SECS", "14400"); // 기본 4h 명시
        let (d, dir) = daemon("approval-ttl");
        let s = surface(&d, Some("master"));
        {
            let mut q = s.pending_queue.lock().unwrap();
            // 4h 직전 = 보존, 4h 경과 = 이관.
            q.push_back(aged(
                "승인 대기(3h)",
                QueueOrigin::system("governance-approval"),
                10_800.0,
            ));
            q.push_back(aged(
                "승인 대기(5h)",
                QueueOrigin::system("governance-approval"),
                18_000.0,
            ));
        }
        expire_queued(&d);

        assert_eq!(texts(&s), vec!["승인 대기(3h)".to_string()], "4h 안은 보존");
        let dl = dead_letters(&dir);
        assert_eq!(dl.len(), 1, "만기분은 무음 삭제가 아니라 원장 이관이다");
        assert_eq!(dl[0]["text"], serde_json::json!("승인 대기(5h)"), "전문 무손실");
        assert_eq!(dl[0]["origin"]["label"], serde_json::json!("governance-approval"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★B5: System origin 도 상한을 받는다 — 단 **Agent 보다 긴 별도 등급**이다.
    /// 종전엔 System 이 TTL 면제라, 배달이 봉쇄된 pane 앞에서 제어 메시지가 무한 누적됐다.
    /// 이관 방식은 Agent 와 동일하다(폐기 아님·dead-letter 전문 보존).
    #[test]
    fn system_origin_expires_on_its_own_longer_ttl() {
        let _g = TTL_ENV_LOCK.lock().unwrap();
        let _e = TtlEnv::set("60"); // Agent 등급
        std::env::set_var("CYS_QUEUE_SYSTEM_TTL_SECS", "600"); // System 등급(더 길다)
        let (d, dir) = daemon("system-ttl");
        let s = surface(&d, Some("master"));
        {
            let mut q = s.pending_queue.lock().unwrap();
            // Agent TTL(60)은 넘겼지만 System 등급(600) 안 — 등급이 다르므로 보존돼야 한다.
            q.push_back(aged("system 신선", QueueOrigin::system("schedule"), 300.0));
            q.push_back(aged("system 만기", QueueOrigin::system("schedule"), 900.0));
            let mut imp = aged("system important", QueueOrigin::system("schedule"), 900.0);
            imp.important = true;
            q.push_back(imp);
            q.push_back(aged("human 무기한", QueueOrigin::Human, 999_999.0));
        }
        expire_queued(&d);

        assert_eq!(
            texts(&s),
            vec![
                "system 신선".to_string(),
                "system important".to_string(),
                "human 무기한".to_string(),
            ],
            "System 은 자기 등급 TTL 로만 만기되고 important·Human 은 면제여야 한다"
        );
        let dl = dead_letters(&dir);
        assert_eq!(dl.len(), 1, "이관 1건 = 원장 1줄(무손실)");
        assert_eq!(dl[0]["text"], serde_json::json!("system 만기"), "전문 무손실");
        assert_eq!(dl[0]["reason"], serde_json::json!("ttl"));
        assert_eq!(dl[0]["origin"]["class"], serde_json::json!("system"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★B5 롤백 계약: `CYS_QUEUE_TTL_SECS=0` 은 **전면** 비활성이다 — System·GUI 등급이 살아
    /// 있어도 sweep 자체가 돌지 않는다(운영자가 TTL 을 끄면 아무것도 만기되지 않는다는 약속).
    /// ★E4 재수정: 등급이 셋으로 늘었으므로 GUI 도 함께 잠기는지 핀한다 — 등급을 추가할 때마다
    /// 이 스위치를 빠뜨리면 "껐는데 뭔가 만기된다"는 가장 나쁜 배신이 생긴다.
    #[test]
    fn ttl_zero_also_disables_system_and_gui_tiers() {
        let _g = TTL_ENV_LOCK.lock().unwrap();
        let _e = TtlEnv::set("0");
        std::env::set_var("CYS_QUEUE_SYSTEM_TTL_SECS", "600");
        std::env::set_var("CYS_QUEUE_GUI_TTL_SECS", "600");
        let t = queue_ttls();
        assert_eq!(
            (t.agent, t.system, t.gui),
            (0.0, 0.0, 0.0),
            "전면 롤백 스위치는 등급표 전체를 0 으로 만든다(호출자별 재판정 금지)"
        );
        let (d, dir) = daemon("system-ttl-off");
        let s = surface(&d, Some("master"));
        {
            let mut q = s.pending_queue.lock().unwrap();
            q.push_back(aged("아주 묵은 system", QueueOrigin::system("schedule"), 99_999.0));
            q.push_back(aged(
                "아주 묵은 gui",
                QueueOrigin::system(crate::state::SYSTEM_LABEL_GUI),
                99_999.0,
            ));
        }
        expire_queued(&d);
        assert_eq!(texts(&s).len(), 2, "전면 롤백 스위치가 등급 확장으로 뚫리면 안 된다");
        assert!(dead_letters(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// TTL=0은 비활성 — 아무리 묵어도 손대지 않는다(런타임 롤백 스위치·§10).
    #[test]
    fn ttl_zero_disables_sweep() {
        let _g = TTL_ENV_LOCK.lock().unwrap();
        let _e = TtlEnv::set("0");
        let (d, dir) = daemon("disabled");
        let s = surface(&d, Some("master"));
        s.pending_queue
            .lock()
            .unwrap()
            .push_back(aged("아주 묵은 항목", agent_origin(), 999_999.0));
        expire_queued(&d);
        assert_eq!(texts(&s).len(), 1, "TTL=0이면 sweep이 돌면 안 된다");
        assert!(!dl_path(&dir).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★Sim2-3 순서 불변식: WAL 복원분은 **rehome 후에만** 만기 판정을 받는다.
    /// expire를 먼저 돌리면 restored_queue에 있는 항목은 손대지 않는다(주인 없는 오귀속 차단).
    #[test]
    fn expire_does_not_touch_restored_queue_before_rehome() {
        let _g = TTL_ENV_LOCK.lock().unwrap();
        let _e = TtlEnv::set("60");
        let (d, dir) = daemon("order");
        let s = surface(&d, Some("worker-1"));
        let item = aged("복원 후 만기", agent_origin(), 120.0);
        d.restored_queue.lock().unwrap().push(QueueWalEntry {
            mid: "qtest".into(),
            surface_id: 9999, // 재기동으로 소멸한 구 id — role이 실질 앵커다.
            role: Some("worker-1".into()),
            item,
        });

        // ① rehome 없이 expire만: restored_queue는 만기 대상이 아니다.
        expire_queued(&d);
        assert_eq!(d.restored_queue.lock().unwrap().len(), 1, "복원 대기분을 만기시키면 안 된다");
        assert!(dead_letters(&dir).is_empty());

        // ② 정식 순서(rehome → expire): 안착 후 만기 판정 → 이관.
        rehome_restored(&d);
        assert!(d.restored_queue.lock().unwrap().is_empty(), "role 앵커로 재홈돼야 한다");
        assert_eq!(texts(&s), vec!["복원 후 만기".to_string()]);
        expire_queued(&d);
        assert!(texts(&s).is_empty(), "안착 후에는 만기 이관 대상이다");
        let dl = dead_letters(&dir);
        assert_eq!(dl.len(), 1);
        assert_eq!(dl[0]["reason"], serde_json::json!("ttl"));
        // WAL이 enqueued_at을 보존하므로 복원분은 나이를 유지한다(재홈이 시계를 리셋하지 않는다).
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★Sim3-4 (무손실 우선 fail-open): dead-letter 기록이 실패하면 항목을 **제거하지 않는다**.
    /// 원장 없는 삭제는 금지 — 실패는 health 경고로 가시화하고 항목은 큐에 남는다.
    /// 주입 방법: 원장 경로에 **디렉터리**를 만들어 append open을 실패시킨다.
    #[test]
    fn dead_letter_write_failure_keeps_item_in_queue() {
        let _g = TTL_ENV_LOCK.lock().unwrap();
        let _e = TtlEnv::set("60");
        let (d, dir) = daemon("failopen");
        let s = surface(&d, Some("master"));
        let sdir = crate::state::state_dir(&dir.join("cysd.sock"));
        std::fs::create_dir_all(&sdir).unwrap();
        std::fs::create_dir_all(sdir.join(queue_policy::DEAD_LETTER_FILE)).unwrap();

        {
            let mut q = s.pending_queue.lock().unwrap();
            q.push_back(aged("원장 실패분", agent_origin(), 120.0));
            q.push_back(aged("신선분", agent_origin(), 1.0));
        }
        expire_queued(&d);
        assert_eq!(
            texts(&s),
            vec!["원장 실패분".to_string(), "신선분".to_string()],
            "원장을 못 남긴 항목은 큐 앞머리로 복원돼 순서까지 보존돼야 한다"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 로테이션: 원장이 10MB 임계에 닿으면 `dead-letters.<ts>.jsonl`로 회전하고
    /// 새 파일에 이어 쓴다(무한 성장 차단). 크기는 sparse set_len으로 만든다.
    #[test]
    fn dead_letter_rotates_at_threshold() {
        let _g = TTL_ENV_LOCK.lock().unwrap();
        let _e = TtlEnv::set("60");
        let (d, dir) = daemon("rotate");
        let s = surface(&d, Some("master"));
        let sdir = crate::state::state_dir(&dir.join("cysd.sock"));
        std::fs::create_dir_all(&sdir).unwrap();
        let f = std::fs::File::create(dl_path(&dir)).unwrap();
        f.set_len(queue_policy::DEAD_LETTER_MAX_BYTES).unwrap();
        drop(f);

        s.pending_queue
            .lock()
            .unwrap()
            .push_back(aged("회전 후 첫 줄", agent_origin(), 120.0));
        expire_queued(&d);

        let dl = dead_letters(&dir);
        assert_eq!(dl.len(), 1, "회전 후 새 파일에는 이번 줄만 있어야 한다");
        assert_eq!(dl[0]["text"], serde_json::json!("회전 후 첫 줄"));
        let rotated: Vec<_> = std::fs::read_dir(&sdir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                let n = e.file_name().to_string_lossy().to_string();
                n.starts_with("dead-letters.") && n.ends_with(".jsonl")
                    && n != queue_policy::DEAD_LETTER_FILE
            })
            .collect();
        assert_eq!(rotated.len(), 1, "회전본이 정확히 1개 남아야 한다");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// C2 (Declared State) — 유령 todo 배제 · fail-open 등재 · mtime 판정 캐시
//
// 이 스위트가 지키는 것: 07-11~07-20에 종결된 레인의 유산 todo 4파일이 07-26 편대의 집계에
// 유입돼 dept-2 306항목 중 301항목(98%)이 유령이 된 사고의 **데몬 측 통로**를 다시 열지 않는 것.
// Python 보고기(C1)만 고치면 절반만 덮는다 — 데몬은 같은 파일들을 같은 방식으로 스캔해
// `daemon.todo_progress` → `org.status` → HUD·Control Center까지 오염시키고 있었다.
// ─────────────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod todo_decl_tests {
    use super::check_todo_with;
    use crate::state::Daemon;
    use serde_json::Value;
    use std::sync::Arc;

    /// CYS_TODO_DIRS는 프로세스 전역 env라 스캔 대상 지정 창을 직렬화한다(같은 테스트 바이너리의
    /// 다른 todo 테스트와 충돌 방지). ★`my_scope`·`scope_exists`는 env가 아니라 **인자**로
    /// 주입하므로 라이브 팩(CYS_PACK_DIR)은 이 스위트에서 아예 건드리지 않는다.
    static TODO_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    const MY: &str = "pack-dept-dept-2";

    /// 이 스위트의 팩 실재 판정 — dept-1은 실재, dept-9는 부재(개명·teardown 흔적).
    fn packs(scope: &str) -> bool {
        matches!(scope, "pack" | "pack-dept-dept-1" | "pack-dept-dept-2")
    }

    fn decl(scope: &str, status: &str) -> String {
        format!("<!-- javis:todo v1 owner=worker-2 scope={scope} status={status} -->\n")
    }

    /// (done 1, total 2)짜리 본문 — 판정과 무관하게 항상 "집계할 거리가 있는" 파일을 만든다.
    /// 유령이 배제되지 않으면 반드시 수치로 드러나게 하는 장치다.
    fn body() -> &'static str {
        "\n# TODO\n- [x] 완료\n- [ ] 미완\n"
    }

    struct Fixture {
        daemon: Arc<Daemon>,
        round: std::path::PathBuf,
        dir: std::path::PathBuf,
    }

    impl Fixture {
        fn new(tag: &str) -> Fixture {
            let dir = std::env::temp_dir().join(format!(
                "cys-todo-decl-{tag}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            let round = dir.join("_round");
            std::fs::create_dir_all(&round).expect("픽스처 디렉터리");
            std::env::set_var("CYS_TODO_DIRS", &round);
            Fixture {
                daemon: Daemon::new(dir.join("cysd.sock")),
                round,
                dir,
            }
        }

        fn write(&self, name: &str, content: &str) -> std::path::PathBuf {
            let p = self.round.join(name);
            std::fs::write(&p, content).expect("픽스처 파일");
            p
        }

        fn tick(&self) {
            // ★S18 이후 `check_todo_with`는 팩 경로를 인자로 받는다. 이 스위트는 라이브 팩을
            // 만지지 않으므로 `None`을 넘긴다(정본 루트 추가 규칙 자체는
            // `cys::todo_scan::scan_roots` 단위 테스트와 `parity_todo_scan.py`가 지킨다).
            check_todo_with(&self.daemon, MY, &packs, None);
        }

        /// 정본 위치(`pack/round`)를 스캔 루트로 넣은 틱 — S18 회귀용.
        fn tick_with_pack(&self, pack: &std::path::Path) {
            check_todo_with(&self.daemon, MY, &packs, Some(pack));
        }

        /// 등재 키는 **정규경로**다(Python 소비자 `os.path.realpath`와 같은 규칙).
        fn key(&self, name: &str) -> String {
            let p = self.round.join(name);
            std::fs::canonicalize(&p)
                .unwrap_or(p)
                .to_string_lossy()
                .into_owned()
        }

        /// 등재된 경로의 파일명 집합 — 절대경로 비교는 임시디렉터리 이름에 묶여 읽기 어렵다.
        fn registered(&self) -> std::collections::BTreeSet<String> {
            self.daemon
                .todo_progress
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .keys()
                .filter_map(|k| {
                    std::path::Path::new(k)
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                })
                .collect()
        }

        fn progress(&self, name: &str) -> Option<(u64, u64)> {
            let key = self.key(name);
            self.daemon
                .todo_progress
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&key)
                .map(|(d, t, _)| (*d, *t))
        }

        fn verdict(&self, name: &str) -> Option<&'static str> {
            let key = self.key(name);
            self.daemon
                .todo_verdict
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&key)
                .map(|(_, v, _)| *v)
        }

        /// 판정 캐시에 보관된 선언 owner(= `org.status`가 싣는 값).
        fn owner(&self, name: &str) -> Option<String> {
            let key = self.key(name);
            self.daemon
                .todo_verdict
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&key)
                .and_then(|(_, _, o)| o.clone())
        }

        /// `after_seq` 이후 발행된 todo.updated 이벤트의 (파일명, verdict) 목록.
        fn todo_events(&self, after_seq: u64) -> Vec<(String, String)> {
            self.daemon
                .bus
                .replay_after(after_seq)
                .into_iter()
                .filter(|e| e["name"] == Value::from("todo.updated"))
                .map(|e| {
                    let p = e["payload"]["path"].as_str().unwrap_or_default().to_string();
                    let name = std::path::Path::new(&p)
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    let v = e["payload"]["verdict"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string();
                    (name, v)
                })
                .collect()
        }

        fn seq(&self) -> u64 {
            self.daemon.bus.latest_seq()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            std::env::remove_var("CYS_TODO_DIRS");
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// ★핵심 회귀 핀 — 유령(은퇴·타 스코프)은 `todo_progress`에 **등재되지 않는다**.
    /// 그리고 판정 불능(미선언·고아)은 fail-open으로 **등재하되 구분 플래그를 단다**(ADR-3):
    /// 판정 못 한다고 숨기면 죽은 워커의 미완 작업이 은폐돼 게이트가 false QUIET에 빠진다.
    #[test]
    fn ghost_todos_are_excluded_and_unclaimed_is_flagged() {
        let _g = TODO_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let f = Fixture::new("exclude");
        // 유령 3종 — 전부 체크박스를 갖고 있어, 배제 실패 시 집계 수치로 즉시 드러난다.
        f.write("MASTER_TODO.md", &format!("{}{}", decl(MY, "retired"), body()));
        f.write("CSO_TODO.md", &format!("{}{}", decl("pack-dept-dept-1", "active"), body()));
        f.write("LEGACY_TODO.md", &format!("<!-- ★ STALE 무효화 -->\n{}", body()));
        // 살아있는 내 파일 + 판정 불능 2종.
        f.write("WORKER_TODO.md", &format!("{}{}", decl(MY, "active"), body()));
        f.write("PLAIN_TODO.md", &format!("# 손으로 쓴 todo{}", body()));
        f.write("ORPHAN_TODO.md", &format!("{}{}", decl("pack-dept-dept-9", "active"), body()));

        let before = f.seq();
        f.tick();

        assert_eq!(
            f.registered(),
            ["ORPHAN_TODO.md", "PLAIN_TODO.md", "WORKER_TODO.md"]
                .iter()
                .map(|s| s.to_string())
                .collect::<std::collections::BTreeSet<_>>(),
            "은퇴·타 스코프 파일이 진행률 집계에 남아 있다 — 유령 유입 경로가 다시 열렸다"
        );
        // 판정 캐시는 배제분까지 **전부** 보유해야 한다(다음 틱 재파싱 방지의 전제).
        assert_eq!(f.verdict("MASTER_TODO.md"), Some("retired"));
        assert_eq!(f.verdict("CSO_TODO.md"), Some("foreign-scope"));
        assert_eq!(f.verdict("LEGACY_TODO.md"), Some("retired"));
        assert_eq!(f.verdict("WORKER_TODO.md"), Some("counted"));
        assert_eq!(f.verdict("PLAIN_TODO.md"), Some("unclaimed"));
        assert_eq!(f.verdict("ORPHAN_TODO.md"), Some("orphan-scope"));
        // 온보딩 방어(§6-2): 미선언 파일의 진행률을 사용자에게서 빼앗지 않는다.
        assert_eq!(f.progress("PLAIN_TODO.md"), Some((1, 2)));
        assert_eq!(f.progress("WORKER_TODO.md"), Some((1, 2)));
        // 최초 발견은 무음 등록 — 데몬 재시작마다 전 파일 이벤트가 폭주하지 않는다(기존 계약).
        assert!(
            f.todo_events(before).is_empty(),
            "최초 스캔은 무음이어야 한다: {:?}",
            f.todo_events(before)
        );
    }

    /// 은퇴·타 스코프는 **이벤트도 발행하지 않는다**. 등재 배제만 하고 이벤트를 흘리면
    /// HUD·구독자가 유령의 갱신을 계속 그린다.
    #[test]
    fn excluded_todos_publish_no_events_on_change() {
        let _g = TODO_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let f = Fixture::new("events");
        let retired = f.write("MASTER_TODO.md", &format!("{}{}", decl(MY, "retired"), body()));
        let alive = f.write("WORKER_TODO.md", &format!("{}{}", decl(MY, "active"), body()));
        f.tick(); // 최초 무음 등록

        let before = f.seq();
        std::fs::write(&retired, format!("{}{}- [ ] 추가\n", decl(MY, "retired"), body())).unwrap();
        std::fs::write(&alive, format!("{}{}- [ ] 추가\n", decl(MY, "active"), body())).unwrap();
        f.tick();

        assert_eq!(
            f.todo_events(before),
            vec![("WORKER_TODO.md".to_string(), "counted".to_string())],
            "은퇴 파일의 갱신이 이벤트로 새어나갔다"
        );
        assert_eq!(f.progress("WORKER_TODO.md"), Some((1, 3)));
    }

    /// 이벤트 payload의 `verdict`는 신설 **선택 필드**다 — 미선언·고아를 HUD가 구분 표시하는
    /// 유일한 근거이며, 불리언 하나로는 두 상태를 나를 수 없어 판정 문자열을 그대로 싣는다.
    #[test]
    fn update_event_carries_verdict_for_unclaimed_and_orphan() {
        let _g = TODO_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let f = Fixture::new("verdict-payload");
        let plain = f.write("PLAIN_TODO.md", body());
        let orphan = f.write(
            "ORPHAN_TODO.md",
            &format!("{}{}", decl("pack-dept-dept-9", "active"), body()),
        );
        f.tick();

        let before = f.seq();
        std::fs::write(&plain, format!("{}- [ ] 추가\n", body())).unwrap();
        std::fs::write(
            &orphan,
            format!("{}{}- [ ] 추가\n", decl("pack-dept-dept-9", "active"), body()),
        )
        .unwrap();
        f.tick();

        let mut got = f.todo_events(before);
        got.sort();
        assert_eq!(
            got,
            vec![
                ("ORPHAN_TODO.md".to_string(), "orphan-scope".to_string()),
                ("PLAIN_TODO.md".to_string(), "unclaimed".to_string()),
            ]
        );
    }

    /// 레인 종결(= 살아있던 파일에 `status=retired`를 기록)이 **이미 등재된 유령을 걷어낸다**.
    /// 배제를 신규 파일에만 적용하면 종결 시점에 집계돼 있던 항목이 영구 잔류한다.
    #[test]
    fn retiring_a_counted_todo_removes_it_from_progress() {
        let _g = TODO_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let f = Fixture::new("retire-transition");
        let p = f.write("WORKER_TODO.md", &format!("{}{}", decl(MY, "active"), body()));
        f.tick();
        assert_eq!(f.progress("WORKER_TODO.md"), Some((1, 2)));

        let before = f.seq();
        std::fs::write(&p, format!("{}{}", decl(MY, "retired"), body())).unwrap();
        f.tick();

        assert!(
            f.progress("WORKER_TODO.md").is_none(),
            "은퇴 선언을 얻은 파일이 집계에 잔류했다"
        );
        assert_eq!(f.verdict("WORKER_TODO.md"), Some("retired"));
        assert!(f.todo_events(before).is_empty(), "은퇴 전이는 조용해야 한다");
    }

    /// ★성능 계약(설계 §4-5 · R2 발견) — mtime이 그대로면 **파일을 다시 읽지 않는다**.
    ///
    /// 검증 방법: 내용을 바꾸되 mtime을 원래 값으로 되돌린 뒤 틱을 돌린다. 재파싱했다면 새 내용
    /// (counted)이 반영돼 집계에 등재됐을 것이다. 등재되지 않았다는 것이 곧 "읽지 않았다"는 증거다.
    /// 이 계약이 없으면 배제 판정 파일은 진행률 맵에 없다는 이유로 **매 워치독 틱마다** 다시
    /// 읽히고 다시 파싱된다 = 전 파일 I/O 순증.
    #[test]
    fn unchanged_mtime_skips_reparse_even_for_excluded_files() {
        let _g = TODO_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let f = Fixture::new("mtime-cache");
        let p = f.write("MASTER_TODO.md", &format!("{}{}", decl(MY, "retired"), body()));
        f.tick();
        assert_eq!(f.verdict("MASTER_TODO.md"), Some("retired"));

        let times = std::fs::metadata(&p).unwrap();
        let stamp = std::fs::FileTimes::new()
            .set_accessed(times.accessed().unwrap())
            .set_modified(times.modified().unwrap());
        std::fs::write(&p, format!("{}{}", decl(MY, "active"), body())).unwrap();
        std::fs::File::options()
            .write(true)
            .open(&p)
            .unwrap()
            .set_times(stamp)
            .unwrap();
        f.tick();

        assert_eq!(
            f.verdict("MASTER_TODO.md"),
            Some("retired"),
            "mtime 무변화인데 재파싱했다 — 워치독 틱에 전 파일 I/O가 순증한다"
        );
        assert!(f.progress("MASTER_TODO.md").is_none());

        // 반대 방향: mtime이 실제로 바뀌면 즉시 반영된다(캐시가 갱신을 막지 않는다).
        std::fs::write(&p, format!("{}{}", decl(MY, "active"), body())).unwrap();
        f.tick();
        assert_eq!(f.verdict("MASTER_TODO.md"), Some("counted"));
        assert_eq!(f.progress("MASTER_TODO.md"), Some((1, 2)));
    }

    /// 사라진 파일은 진행률과 **판정 캐시 양쪽에서** 함께 정리된다(24/365 데몬의 맵 누수 차단).
    #[test]
    fn vanished_files_are_pruned_from_both_maps() {
        let _g = TODO_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let f = Fixture::new("prune");
        let alive = f.write("WORKER_TODO.md", &format!("{}{}", decl(MY, "active"), body()));
        let ghost = f.write("MASTER_TODO.md", &format!("{}{}", decl(MY, "retired"), body()));
        f.tick();
        assert_eq!(f.daemon.todo_verdict.lock().unwrap().len(), 2);

        std::fs::remove_file(&alive).unwrap();
        std::fs::remove_file(&ghost).unwrap();
        f.tick();

        assert!(f.daemon.todo_progress.lock().unwrap().is_empty());
        assert!(
            f.daemon.todo_verdict.lock().unwrap().is_empty(),
            "판정 캐시가 사라진 파일을 붙들고 있다(단조 누적 누수)"
        );
    }

    /// 선언 파싱 예산(G3)은 선두 1 KiB다. 체크박스 집계용 64KB 읽기를 재사용하되 **선두만**
    /// 넘긴다 — 1 KiB 밖의 은퇴 선언은 보이지 않아야 예산이 계약으로 성립한다.
    /// (예산이 없으면 거대 파일이 워치독 틱을 잡아먹는다.)
    #[test]
    fn declaration_beyond_head_budget_is_not_honored() {
        let _g = TODO_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let f = Fixture::new("budget");
        let pad = "x".repeat(cys::todo_decl::HEAD_BYTES);
        f.write(
            "WORKER_TODO.md",
            &format!("{pad}\n{}{}", decl(MY, "retired"), body()),
        );
        f.tick();
        assert_eq!(
            f.verdict("WORKER_TODO.md"),
            Some("unclaimed"),
            "예산 밖 선언이 인정되면 임의 파일 말미의 문구가 집계를 조작할 수 있다"
        );
        assert_eq!(f.progress("WORKER_TODO.md"), Some((1, 2)), "fail-open 등재는 유지");
    }

    /// ★비UTF-8 정합(2026-07-26 교정 6) — 데몬과 Python 소비자가 갈리지 않는다.
    ///
    /// 종전 `read_to_string`은 비UTF-8 바이트 하나에 `continue`로 빠져 **등재 0·캐시 갱신 0**
    /// 이었다(캐시가 비니 매 틱 재파싱까지 겹친다). Python `javis_report`는 같은 파일을
    /// `errors="replace"`로 lossy 디코드해 **집계한다** — 같은 파일에 대해 데몬은 "없음",
    /// 팩은 "있음"이라고 말하는 조용한 갈림이었다. 조용한 차이가 최악이므로 여기로 수렴시킨다.
    #[test]
    fn non_utf8_todo_is_lossy_decoded_like_python() {
        let _g = TODO_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let f = Fixture::new("non-utf8");
        let mut bytes = decl(MY, "active").into_bytes();
        bytes.extend_from_slice(b"\n# \xff\xfe\x80 (\xeb\x81\xa8\xec\xa7\x84 UTF-8)\n");
        bytes.extend_from_slice(b"- [x] \xff\xfe\n- [ ] \x80\n");
        std::fs::write(f.round.join("WORKER_TODO.md"), &bytes).expect("픽스처 파일");
        f.tick();

        assert_eq!(
            f.verdict("WORKER_TODO.md"),
            Some("counted"),
            "비UTF-8 바이트 하나로 판정 캐시가 통째로 비면 매 틱 재파싱된다"
        );
        assert_eq!(
            f.progress("WORKER_TODO.md"),
            Some((1, 2)),
            "데몬이 집계하지 않는 파일을 Python 소비자는 집계한다 = 2언어 조용한 갈림"
        );
    }

    /// ★`owner` 동봉(교정 3) — 소비자가 라벨을 파일명에서 추론하지 않아도 되게 한다.
    /// 집계 **키는 경로 그대로**다(설계 §5-2: 키 스키마 변경은 파급 확대로 기각).
    /// 센티널 `"?"`(ADR-4 C-3 · 레거시 은퇴 = 주인 미상)는 싣지 않는다 — 없는 정보를
    /// 있는 것처럼 흘리면 소비자가 `"?"`라는 라벨의 노드를 그린다.
    #[test]
    fn update_event_carries_owner_but_not_sentinel() {
        let _g = TODO_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let f = Fixture::new("owner-payload");
        // 파일명(WORKER)과 owner(worker-2)가 다른 상태 — 파일명 추론이 틀리는 정확한 조건.
        let named = f.write("WORKER_TODO.md", &format!("{}{}", decl(MY, "active"), body()));
        let plain = f.write("PLAIN_TODO.md", body());
        f.tick();

        let before = f.seq();
        std::fs::write(&named, format!("{}{}- [ ] 추가\n", decl(MY, "active"), body())).unwrap();
        std::fs::write(&plain, format!("{}- [ ] 추가\n", body())).unwrap();
        f.tick();

        let owners: std::collections::BTreeMap<String, Option<String>> = f
            .daemon
            .bus
            .replay_after(before)
            .into_iter()
            .filter(|e| e["name"] == Value::from("todo.updated"))
            .map(|e| {
                let p = e["payload"]["path"].as_str().unwrap_or_default().to_string();
                let name = std::path::Path::new(&p)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                (
                    name,
                    e["payload"]["owner"].as_str().map(|s| s.to_string()),
                )
            })
            .collect();

        assert_eq!(owners.get("WORKER_TODO.md"), Some(&Some("worker-2".into())));
        // 미선언 파일은 owner를 알 수 없다 — 필드 자체가 없어야 한다(빈 문자열도 아니다).
        assert_eq!(owners.get("PLAIN_TODO.md"), Some(&None));
    }

    /// ★락 순서 규약(TP→TV) 회귀 — poison된 맵에서도 워치독 틱은 살아남는다(교정 5).
    ///
    /// 종전에는 같은 함수 안에서 판정 캐시만 poison 내성이고 진행률 맵은 `.unwrap()`이라
    /// 다른 스레드의 패닉 한 번이 워치독 틱을 데몬 수명 내내 죽였다 — 주석은 그 위험을
    /// 정확히 적어 놓고 절반만 이행돼 있었다. 방어의 비대칭은 방어가 아니다.
    #[test]
    fn watchdog_tick_survives_both_poisoned_todo_locks() {
        let _g = TODO_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let f = Fixture::new("poison");
        f.write("WORKER_TODO.md", &format!("{}{}", decl(MY, "active"), body()));

        // 두 맵을 각각 poison 시킨다(패닉 스레드는 join으로 회수 — 테스트 러너는 죽지 않는다).
        for which in 0..2 {
            let d = Arc::clone(&f.daemon);
            let h = std::thread::spawn(move || {
                if which == 0 {
                    let _g = d.todo_progress.lock().unwrap();
                    panic!("의도된 패닉 — todo_progress poison");
                } else {
                    let _g = d.todo_verdict.lock().unwrap();
                    panic!("의도된 패닉 — todo_verdict poison");
                }
            });
            assert!(h.join().is_err(), "패닉 스레드가 패닉하지 않았다");
        }
        assert!(f.daemon.todo_progress.is_poisoned());
        assert!(f.daemon.todo_verdict.is_poisoned());

        f.tick(); // 패닉하면 여기서 테스트가 죽는다 = 회귀

        assert_eq!(f.progress("WORKER_TODO.md"), Some((1, 2)));
        assert_eq!(f.verdict("WORKER_TODO.md"), Some("counted"));
    }

    /// ★**W14 S18 회귀 핀 — 데몬이 정본 위치(`pack/round`)를 본다.**
    ///
    /// 종전 스캔 루트는 surface `cwd/_round` + `CYS_TODO_DIRS`뿐이었고, `CYS_TODO_DIRS`를
    /// 자동 주입하는 지점은 저장소 전수 grep 0건이었다. 그런데 이 조직의 **정본 todo 위치는
    /// `${CYS_PACK_DIR}/round/`** 다(위임 티켓·`cys todo-path`·Python 보고기가 전부 그곳을 쓴다).
    /// 즉 이번 브랜치가 데몬에 배선한 선언 판정·유령 배제·verdict/owner payload가 **정본
    /// todo에는 한 번도 적용되지 않았다** = 데몬 작업 대부분이 실질 무효였다.
    #[test]
    fn canonical_pack_round_is_scanned() {
        let _g = TODO_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let f = Fixture::new("pack-round");
        // 팩은 픽스처 디렉터리 안에 만든다 — 라이브 `~/.cys/pack` 무접촉.
        let pack = f.dir.join("pack");
        let pack_round = pack.join("round");
        std::fs::create_dir_all(&pack_round).expect("팩 round");
        let canonical = pack_round.join("WORKER_TODO.md");
        std::fs::write(&canonical, format!("{}{}", decl(MY, "active"), body())).unwrap();
        // 정본 위치의 유령도 같은 정책으로 배제돼야 한다(정책은 이미 같았고 시야만 없었다).
        std::fs::write(
            pack_round.join("MASTER_TODO.md"),
            format!("{}{}", decl(MY, "retired"), body()),
        )
        .unwrap();

        // ① 팩 경로를 안 주면 정본 파일은 **보이지 않는다**(종전 동작 = 결함 재현).
        f.tick();
        assert!(
            f.registered().is_empty(),
            "팩 루트 없이 정본 파일이 보였다 — 이 테스트의 전제가 무너졌다: {:?}",
            f.registered()
        );

        // ② 팩 경로를 주면 보인다. 키는 정규경로다(Python 소비자와 같은 규칙).
        f.tick_with_pack(&pack);
        assert_eq!(
            f.registered(),
            ["WORKER_TODO.md".to_string()]
                .into_iter()
                .collect::<std::collections::BTreeSet<_>>(),
            "정본 위치의 살아있는 todo가 집계에 없다(S18 재발) / 유령이 섞였다"
        );
        let key = std::fs::canonicalize(&canonical)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert_eq!(
            f.daemon
                .todo_progress
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&key)
                .map(|(d, t, _)| (*d, *t)),
            Some((1, 2))
        );
    }

    /// ★**W14 — 소비자 테스트의 자기 반사 차단(reviewer3 자기 신고 2번).**
    ///
    /// 이 스위트의 나머지 케이스는 기대값을 `cys::todo_decl`(파서)에서 **유도**한다. 즉
    /// 파서와 소비자가 **함께 틀리면 초록**이다 — Python 쪽에는 `expected.json`이라는 외부
    /// SOT가 있는데 Rust 소비자에는 대응물이 없었다(그래서 "가장 의심스러운 남은 자리"였다).
    ///
    /// 여기서는 골든 픽스처 파일을 **그대로** 스캔 디렉터리에 넣고, 기대값을 오직
    /// `expected.json`에서 읽어 대조한다. 파서를 호출해 기대값을 만들지 않는다 —
    /// 그것이 자기 반사를 끊는다는 말의 실제 내용이다.
    ///
    /// 대조 2축: ①판정 캐시의 verdict = 대장의 `classify` ②등재 여부 = "조용히 빼도 되는
    /// 것은 `retired`·`foreign-scope` 둘뿐"이라는 정책(ADR-3)이 대장 값으로부터 재현되는가.
    #[test]
    fn golden_fixtures_drive_daemon_verdicts_from_external_sot() {
        let _g = TODO_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("cysjavis-pack/bin/tests/fixtures/todo-decl");
        let raw = std::fs::read_to_string(dir.join("expected.json")).unwrap_or_else(|e| {
            panic!("골든 대장을 읽을 수 없다({}): {e} — SOT 부재는 skip이 아니라 실패다",
                   dir.display())
        });
        let spec: Value = serde_json::from_str(&raw).expect("expected.json 파싱");
        let my_scope = spec["my_scope"].as_str().expect("my_scope").to_string();
        let existing: Vec<String> = spec["existing_scopes"]
            .as_array()
            .expect("existing_scopes")
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect();
        let scope_exists = move |s: &str| existing.iter().any(|e| e == s);

        let f = Fixture::new("golden-sot");
        let cases = spec["cases"].as_object().expect("cases");
        assert!(cases.len() >= 15, "픽스처 케이스가 15종 미만이다: {}", cases.len());
        // 픽스처 이름을 todo 파일명 규칙(`*_TODO.md`)에 맞춰 복사한다 — 내용은 한 바이트도
        // 바꾸지 않는다(바이너리 케이스가 있으므로 텍스트 경유 금지).
        let mut want: std::collections::BTreeMap<String, String> =
            std::collections::BTreeMap::new();
        for (name, exp) in cases {
            let bytes = std::fs::read(dir.join(name))
                .unwrap_or_else(|e| panic!("픽스처 {name} 읽기 실패: {e}"));
            let stem = name.trim_end_matches(".md").replace('.', "_");
            let todo_name = format!("{stem}_TODO.md");
            std::fs::write(f.round.join(&todo_name), &bytes).expect("픽스처 복사");
            want.insert(
                todo_name,
                exp["classify"].as_str().expect("classify").to_string(),
            );
        }

        check_todo_with(&f.daemon, &my_scope, &scope_exists, None);

        let got_verdicts = f
            .daemon
            .todo_verdict
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter_map(|(k, (_, v, _))| {
                std::path::Path::new(k)
                    .file_name()
                    .map(|n| (n.to_string_lossy().into_owned(), v.to_string()))
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(got_verdicts, want, "데몬 판정이 골든 대장(외부 SOT)과 갈렸다");

        // 등재 정책도 대장 값에서 유도한다(파서에 묻지 않는다).
        let want_registered: std::collections::BTreeSet<String> = want
            .iter()
            .filter(|(_, v)| v.as_str() != "retired" && v.as_str() != "foreign-scope")
            .map(|(k, _)| k.clone())
            .collect();
        assert_eq!(
            f.registered(),
            want_registered,
            "등재 집합이 대장에서 유도한 정책과 갈렸다(조용한 배제는 retired·foreign-scope 둘뿐)"
        );
    }

    /// ★W14 S16 — 판정 캐시가 선언 `owner`를 보관한다(= `org.status`가 싣는 값의 원천).
    /// 이벤트에만 owner가 있고 스냅샷에 없으면 HUD 라벨이 새로고침 한 번에 뒤집힌다.
    #[test]
    fn verdict_cache_keeps_declared_owner_for_status_snapshot() {
        let _g = TODO_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let f = Fixture::new("owner-cache");
        // 파일명(WORKER)과 owner(worker-2)가 다른 상태 — 파일명 추론이 틀리는 정확한 조건.
        f.write("WORKER_TODO.md", &format!("{}{}", decl(MY, "active"), body()));
        f.write("PLAIN_TODO.md", body());
        f.write("LEGACY_TODO.md", &format!("<!-- ★ STALE 무효화 -->\n{}", body()));
        f.tick();

        assert_eq!(f.owner("WORKER_TODO.md").as_deref(), Some("worker-2"));
        assert_eq!(f.owner("PLAIN_TODO.md"), None, "미선언은 주인을 모른다");
        // ADR-4 C-3 센티널 `"?"`는 저장하지 않는다 — 소비자가 `"?"` 노드를 그리면 안 된다.
        assert_eq!(f.owner("LEGACY_TODO.md"), None, "센티널이 owner로 새어나갔다");
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// D9② 회귀 가드 — drain(폐기) 후 WAL 갱신 누락 = 유령 항목
// ─────────────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod queue_drain_persist_tests {
    use crate::state::{Daemon, QueueItem, QueueOrigin};
    use std::sync::Arc;

    fn daemon(tag: &str) -> (Arc<Daemon>, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "cys-qdrain-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        (Daemon::new(dir.join("cysd.sock")), dir)
    }

    /// WAL v2 파일의 entries 수 — 디스크 사실만 본다(인메모리 상태를 믿지 않는다).
    fn wal_len(dir: &std::path::Path) -> usize {
        let p = crate::state::state_dir(&dir.join("cysd.sock")).join(crate::state::QUEUE_WAL_V2);
        let Ok(s) = std::fs::read_to_string(p) else {
            return 0;
        };
        serde_json::from_str::<serde_json::Value>(&s)
            .ok()
            .and_then(|v| v["entries"].as_array().map(|a| a.len()))
            .unwrap_or(0)
    }

    /// ★D9②: close_surface의 큐 drain 후 WAL을 갱신하지 않으면 디스크에 폐기 메시지가 남아
    /// 다음 재기동에서 같은 role의 새 surface로 되살아난다(유령 배달). drain = 큐 상태 변화이므로
    /// enqueue/pop/clear와 동일하게 즉시 영속돼야 한다.
    #[test]
    fn close_surface_drain_updates_wal() {
        let (d, dir) = daemon("close");
        let s = d
            .create_surface(None, Some("sleep 30".into()), None, Some("worker-1".into()), 24, 80)
            .expect("create surface");
        d.surfaces.lock().unwrap().insert(s.id, s.clone());
        s.pending_queue.lock().unwrap().push_back(QueueItem::text(
            "유령 후보".into(),
            QueueOrigin::system("t"),
        ));
        d.persist_queue_state();
        assert_eq!(wal_len(&dir), 1, "전제: WAL에 1건이 실려 있다");

        super::close_surface(&d, s.id, super::CloseCause::OwnerClose).expect("close");
        assert_eq!(
            wal_len(&dir),
            0,
            "drain 후 WAL에 유령 항목이 남으면 재기동 시 되살아나 오배달된다"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★D9②(짝): 자력 종료(셸 EOF)는 close_surface를 거치지 않는 별도 경로다 —
    /// 같은 누락이 그쪽에도 있었다. 짧게 사는 셸이 EOF를 내면 reader 스레드가 drain하고
    /// WAL을 갱신해야 한다.
    #[test]
    fn self_exit_drain_updates_wal() {
        let (d, dir) = daemon("eof");
        let s = d
            .create_surface(None, Some("sleep 1".into()), None, Some("worker-2".into()), 24, 80)
            .expect("create surface");
        d.surfaces.lock().unwrap().insert(s.id, s.clone());
        s.pending_queue.lock().unwrap().push_back(QueueItem::text(
            "유령 후보".into(),
            QueueOrigin::system("t"),
        ));
        d.persist_queue_state();
        assert_eq!(wal_len(&dir), 1, "전제: WAL에 1건이 실려 있다");

        // EOF는 reader 스레드에서 비동기로 처리된다 — 최대 15초 폴링.
        let t0 = std::time::Instant::now();
        while t0.elapsed() < std::time::Duration::from_secs(15) {
            if s.exited.load(std::sync::atomic::Ordering::Relaxed) && wal_len(&dir) == 0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        assert!(
            s.exited.load(std::sync::atomic::Ordering::Relaxed),
            "전제: 셸이 자력 종료해 EOF 경로가 돌아야 한다"
        );
        assert_eq!(wal_len(&dir), 0, "자력 종료 drain 후에도 WAL이 갱신돼야 한다");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn dead_letters(dir: &std::path::Path) -> Vec<serde_json::Value> {
        let p = crate::state::state_dir(&dir.join("cysd.sock"))
            .join(crate::queue_policy::DEAD_LETTER_FILE);
        std::fs::read_to_string(p)
            .unwrap_or_default()
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).expect("원장 라인은 유효 JSON"))
            .collect()
    }

    /// ★A2(REVISE 수리): surface 종료는 **무음 인멸 경로**였다. drain 후 이벤트만 냈는데
    /// 이벤트 버스는 유실 가능한 링이고 원장이 SOT라는 게 이 설계의 전제다 —
    /// 오퍼레이터가 탭을 닫는 순간 인플라이트 메시지가 어디에도 남지 않고 사라졌다.
    #[test]
    fn close_surface_records_dead_letter_before_drain() {
        let (d, dir) = daemon("close-dl");
        let s = d
            .create_surface(None, Some("sleep 30".into()), None, Some("worker-1".into()), 24, 80)
            .expect("create surface");
        d.surfaces.lock().unwrap().insert(s.id, s.clone());
        s.pending_queue.lock().unwrap().push_back(QueueItem::text(
            "닫힘과 함께 사라지던 전문".into(),
            QueueOrigin::system("t"),
        ));

        super::close_surface(&d, s.id, super::CloseCause::OwnerClose).expect("close");

        let dl = dead_letters(&dir);
        assert_eq!(dl.len(), 1, "surface 종료가 무음 인멸됐다");
        assert_eq!(dl[0]["reason"], serde_json::json!("surface_closed"));
        assert_eq!(dl[0]["text"], serde_json::json!("닫힘과 함께 사라지던 전문"));
        assert_eq!(dl[0]["role"], serde_json::json!("worker-1"), "role 앵커도 남아야 한다");
        assert_eq!(wal_len(&dir), 0, "기록 성공분은 WAL 에서도 빠져야 한다");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★A2 무손실: 원장을 못 남기면 항목을 버리지 않는다. close_surface 는 surface 를 맵에서
    /// 이미 뺐으므로 pending_queue 잔류만으로는 persist 가 보지 못한다 — restored_queue 로
    /// 입양해 WAL 에 싣고 rehome 대상으로 만들어야 한다(그게 '보존'의 실제 조건).
    #[cfg(unix)]
    #[test]
    fn close_surface_retains_items_when_ledger_write_fails() {
        use std::os::unix::fs::PermissionsExt;
        let (d, dir) = daemon("close-retain");
        let s = d
            .create_surface(None, Some("sleep 30".into()), None, Some("worker-9".into()), 24, 80)
            .expect("create surface");
        d.surfaces.lock().unwrap().insert(s.id, s.clone());
        s.pending_queue.lock().unwrap().push_back(QueueItem::text(
            "버려지면 안 되는 전문".into(),
            QueueOrigin::system("t"),
        ));

        let sdir = crate::state::state_dir(&dir.join("cysd.sock"));
        std::fs::create_dir_all(&sdir).unwrap();
        let _ = std::fs::remove_file(sdir.join(crate::queue_policy::DEAD_LETTER_FILE));
        let orig = std::fs::metadata(&sdir).unwrap().permissions();
        std::fs::set_permissions(&sdir, std::fs::Permissions::from_mode(0o555)).unwrap();

        let closed = super::close_surface(&d, s.id, super::CloseCause::OwnerClose);
        std::fs::set_permissions(&sdir, orig).unwrap();
        closed.expect("close");

        assert!(dead_letters(&dir).is_empty(), "전제: 원장 기록이 실패해야 한다");
        assert_eq!(
            d.restored_queue.lock().unwrap().len(),
            1,
            "★원장 없는 삭제가 발생했다 — 잔류 항목이 WAL(rehome) 로 보존되지 않았다"
        );
        // 권한 복구 후 다시 영속하면 잔류분이 실제로 WAL 에 실린다.
        d.persist_queue_state();
        assert_eq!(wal_len(&dir), 1, "잔류 항목이 디스크에 보존되지 않았다");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★A2(짝): 자력 종료(reader EOF)도 같은 규율 — 원장 기록 후 drain.
    #[test]
    fn self_exit_records_dead_letter_before_drain() {
        let (d, dir) = daemon("eof-dl");
        let s = d
            .create_surface(None, Some("sleep 1".into()), None, Some("worker-2".into()), 24, 80)
            .expect("create surface");
        d.surfaces.lock().unwrap().insert(s.id, s.clone());
        s.pending_queue.lock().unwrap().push_back(QueueItem::text(
            "자력 종료와 함께 사라지던 전문".into(),
            QueueOrigin::system("t"),
        ));

        let t0 = std::time::Instant::now();
        while t0.elapsed() < std::time::Duration::from_secs(15) {
            if s.exited.load(std::sync::atomic::Ordering::Relaxed) && !dead_letters(&dir).is_empty()
            {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        let dl = dead_letters(&dir);
        assert_eq!(dl.len(), 1, "자력 종료가 무음 인멸됐다");
        assert_eq!(dl[0]["reason"], serde_json::json!("process_exited"));
        assert_eq!(dl[0]["text"], serde_json::json!("자력 종료와 함께 사라지던 전문"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// T5 (C5) — oob_notify: agent-present 게이트 · human 가드 · 쿨다운/재통지 · privileged set
// ─────────────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod oob_notify_tests {
    use super::{
        alert_queue_depth_if_high, depth_digest_text, notify_depth_digest, oob_notify, oob_targets,
        HardBacklog, OobSkip, SeatState, OOB_COOLDOWN_FILE, OOB_DEPTH_DIGEST_KEY,
        OOB_HARD_COOLDOWN_SECS, OOB_REQUEST_CLEAR_COOLDOWN_SECS,
    };
    use crate::state::Daemon;
    use std::collections::HashMap;
    use std::sync::atomic::Ordering;
    use std::sync::Arc;

    fn daemon(tag: &str) -> (Arc<Daemon>, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "cys-oob-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        (Daemon::new(dir.join("cysd.sock")), dir)
    }

    fn surface(d: &Arc<Daemon>, role: Option<&str>) -> Arc<crate::state::Surface> {
        let s = d
            .create_surface(None, Some("sleep 30".into()), None, role.map(|r| r.into()), 24, 80)
            .expect("create surface");
        d.surfaces.lock().unwrap().insert(s.id, s.clone());
        if let Some(r) = role {
            d.roles.lock().unwrap().insert(r.to_string(), s.id);
        }
        s
    }

    /// 에이전트가 앉아 있는 좌석으로 만든다(주입 안전 조건 충족).
    fn seat_agent(s: &Arc<crate::state::Surface>) {
        *s.agent_meta.lock().unwrap() = Some(("claude".into(), "claude".into()));
        s.seat_cache.store(SeatState::Occupied.as_u8(), Ordering::Relaxed);
    }

    /// ★Sim3-2(안전 필수): 빈 좌석 bare shell에 주입하면 통지 문자열이 **셸 명령으로 실행**된다.
    /// agent_meta 부재 또는 seat=Empty면 주입하지 않는다(스케줄러가 이미 지키는 가드와 동일 근거).
    #[test]
    fn agent_present_gate_blocks_bare_shell_injection() {
        let (d, dir) = daemon("agent-gate");
        let s = surface(&d, Some("worker-1"));

        // ① agent_meta 없음(맨 셸) — seat가 Occupied여도 주입 금지.
        s.seat_cache.store(SeatState::Occupied.as_u8(), Ordering::Relaxed);
        assert_eq!(
            oob_notify(&d, s.id, "통지", "k", 1.0),
            Err(OobSkip::GateBlocked),
            "맨 셸에 주입하면 안 된다"
        );

        // ② agent_meta 있으나 좌석이 비었음(에이전트 죽음) — 주입 금지.
        *s.agent_meta.lock().unwrap() = Some(("claude".into(), "claude".into()));
        s.seat_cache.store(SeatState::Empty.as_u8(), Ordering::Relaxed);
        assert_eq!(
            oob_notify(&d, s.id, "통지", "k", 1.0),
            Err(OobSkip::GateBlocked),
            "빈 좌석에 주입하면 안 된다"
        );

        // ③ 에이전트 착석 — 통과.
        s.seat_cache.store(SeatState::Occupied.as_u8(), Ordering::Relaxed);
        assert_eq!(
            oob_notify(&d, s.id, "통지", "k", 1.0),
            Ok(()),
            "정상 좌석에는 주입돼야 한다"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// human 가드: 사람 입력 흔적이 식기 전에는 주입하지 않는다. 스케줄러 경로와 달리
    /// OOB는 이 가드를 **존중한다** — 미완성 입력에 이어붙이거나 그대로 제출시키지 않기 위함.
    #[test]
    fn human_guard_defers_injection() {
        let (d, dir) = daemon("human");
        let s = surface(&d, Some("worker-1"));
        seat_agent(&s);
        *s.last_human_input.lock().unwrap() = Some(std::time::Instant::now());
        assert_eq!(
            oob_notify(&d, s.id, "통지", "k", 1.0),
            Err(OobSkip::HumanTyping),
            "사람 입력 직후엔 보류해야 한다 — 사유도 정확해야 한다"
        );
        // 가드로 skip된 시도는 쿨다운을 찍지 않으므로 다음 틱에 그대로 재시도된다.
        assert!(d.oob_cooldowns.lock().unwrap().is_empty(), "skip이 쿨다운을 소모하면 안 된다");
        *s.last_human_input.lock().unwrap() = None;
        assert_eq!(oob_notify(&d, s.id, "통지", "k", 1.0), Ok(()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 쿨다운 + **재통지 의미론**(D6): OOB 도달은 보장되지 않으므로 1회 발사 후 기도하지 않는다.
    /// 쿨다운 안에는 억제하고, 경과하면 (문제가 미해소인 한) 다시 통지한다.
    #[test]
    fn cooldown_suppresses_then_renotifies() {
        let (d, dir) = daemon("cooldown");
        let s = surface(&d, Some("worker-1"));
        seat_agent(&s);

        assert_eq!(oob_notify(&d, s.id, "1차", "depth_high", OOB_HARD_COOLDOWN_SECS), Ok(()));
        assert_eq!(
            oob_notify(&d, s.id, "2차", "depth_high", OOB_HARD_COOLDOWN_SECS),
            Err(OobSkip::Cooldown),
            "쿨다운 내 재발화는 억제돼야 한다"
        );
        // 다른 키는 독립 쿨다운(★E1 이후 전역캡이 사라져 별도 seam 없이 그대로 성립한다).
        assert_eq!(
            oob_notify(&d, s.id, "다른 주제", "ttl_expired", OOB_HARD_COOLDOWN_SECS),
            Ok(())
        );

        // 쿨다운 경과를 모사 → 재통지(자기치유).
        d.oob_cooldowns
            .lock()
            .unwrap()
            .insert((s.id, "depth_high".into()), crate::state::now_epoch() - 3600.0);
        assert_eq!(
            oob_notify(&d, s.id, "3차", "depth_high", OOB_HARD_COOLDOWN_SECS),
            Ok(()),
            "미해소가 지속되면 쿨다운 후 재통지돼야 한다"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 실제로 **주입된** OOB 를 이벤트 원장으로 관측한다 — 주입 텍스트는 writer 스레드가
    /// 소비해 사후 확인이 불가하므로, 유일한 발행자(`oob_notify`)의 이벤트가 사실의 대리자다.
    /// `bytes` 가 본문 길이라 "어떤 다이제스트가 갔는지"까지 바이트 단위로 대조할 수 있다.
    fn oob_events(d: &Arc<Daemon>, sid: u64, key: &str) -> Vec<serde_json::Value> {
        d.bus
            .replay_after(0)
            .into_iter()
            .filter(|e| {
                e["name"] == crate::queue_policy::queue_events::OOB_NOTIFIED
                    && e["surface_id"].as_u64() == Some(sid)
                    && e["payload"]["dedup_key"] == key
            })
            .collect()
    }

    fn digest_bytes(d: &Arc<Daemon>, sid: u64) -> Vec<usize> {
        oob_events(d, sid, OOB_DEPTH_DIGEST_KEY)
            .iter()
            .map(|e| e["payload"]["bytes"].as_u64().unwrap_or(0) as usize)
            .collect()
    }

    /// 쿨다운 스탬프를 창 밖으로 밀어 "창 경과"를 모사한다(테스트 전용 seam).
    fn rewind_cooldown(d: &Arc<Daemon>, sid: u64, key: &str, secs: f64) {
        let mut cd = d.oob_cooldowns.lock().unwrap();
        let k = (sid, key.to_string());
        if let Some(ts) = cd.get(&k).copied() {
            cd.insert(k, ts - secs);
        }
    }

    fn hard(sid: u64, role: &str) -> HardBacklog {
        HardBacklog {
            sid,
            role: Some(role.to_string()),
            depth: 15,
            agent_depth: 15,
            oldest_secs: 120,
            blocked_by: "busy(출력 중)".into(),
        }
    }

    /// ★E1 (BLOCK-1 회귀 핀 · 핵심): **적체 노드 2개가 서로 다른 시각에 발생해도 둘 다 도달**한다.
    ///
    /// 종전 구조(노드별 `depth_high_hard:{sid}` + 300s 전역캡)에서는 소스 쿨다운(300s)과
    /// 전역캡(300s)이 동주기라 위상이 고정됐고, 억제된 통지는 폐기(`let _ =`)돼 **먼저 적체된
    /// 1개 외 전원이 영구 침묵**했다. 다이제스트는 통지를 노드 수와 무관하게 1통으로 합치므로
    /// 나중에 적체된 노드도 다음 창의 본문에 실린다.
    #[test]
    fn depth_digest_reaches_every_backlogged_node_and_caps_at_one_per_window() {
        let (d, dir) = daemon("digest");
        let master = surface(&d, Some("master"));
        seat_agent(&master);

        let solo = hard(11, "worker-1");
        let both = [hard(11, "worker-1"), hard(22, "cso")];

        // ── t0: 노드 11만 적체 → 다이제스트 1통 ──
        notify_depth_digest(&d, std::slice::from_ref(&solo));
        assert_eq!(
            digest_bytes(&d, master.id),
            vec![depth_digest_text(std::slice::from_ref(&solo)).len()],
            "다이제스트는 대상당 1통이다"
        );

        // ── 창 안: 노드 22가 뒤늦게 적체(위상차) → 총량 상한 1 이므로 지금은 억제 ──
        notify_depth_digest(&d, &both);
        assert_eq!(
            digest_bytes(&d, master.id).len(),
            1,
            "창 안에서 추가 주입이 나가면 총량 상한이 없는 것과 같다"
        );

        // ── 창 경과: 억제됐던 22가 **다음 다이제스트 본문에 실려** 도달한다 ──
        rewind_cooldown(&d, master.id, OOB_DEPTH_DIGEST_KEY, 86_400.0);
        notify_depth_digest(&d, &both);
        let sent = digest_bytes(&d, master.id);
        assert_eq!(sent.len(), 2, "창이 지나면 재통지된다(자기치유)");
        assert_eq!(
            sent[1],
            depth_digest_text(&both).len(),
            "2차 통지 본문이 2노드 다이제스트가 아니면 뒤늦은 노드는 영구 침묵한다"
        );
        assert_ne!(sent[0], sent[1], "전제: 1노드·2노드 본문은 서로 다르다");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★E1: 다이제스트 대상 = 적체 노드들의 소유 노드 **합집합** + 지휘 role. 노드가 몇 개든
    /// 대상 1명이 받는 통지는 1건이다(침습 채널 총량 상한이 곧 이 구조).
    #[test]
    fn depth_digest_targets_every_owner_union_once() {
        let (d, dir) = daemon("digest-targets");
        let master = surface(&d, Some("master"));
        let w1 = surface(&d, Some("worker-1"));
        let w2 = surface(&d, Some("worker-2"));
        for s in [&master, &w1, &w2] {
            seat_agent(s);
        }
        let backlog = [hard(w1.id, "worker-1"), hard(w2.id, "worker-2")];
        let want = depth_digest_text(&backlog);
        notify_depth_digest(&d, &backlog);
        for s in [&master, &w1, &w2] {
            assert_eq!(
                digest_bytes(&d, s.id),
                vec![want.len()],
                "대상 {}: 노드 2개여도 통지는 1통이고 본문은 전 목록이다",
                s.id
            );
        }
        // 무손실: 본문에 적체 노드가 모두 들어 있다(위 bytes 대조의 대상 문자열).
        assert!(want.contains(&cys::surface_ref(w1.id)) && want.contains(&cys::surface_ref(w2.id)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★E1: 다이제스트 본문은 **단행**이어야 한다 — 주입 텍스트의 개행은 수신 TUI 에서 제출
    /// (Return)로 해석돼 두 번째 줄부터 프롬프트에 흩뿌려진다.
    #[test]
    fn depth_digest_text_is_single_line_and_lossless() {
        let t = depth_digest_text(&[hard(11, "worker-1"), hard(22, "cso")]);
        assert!(!t.contains('\n'), "다이제스트 본문에 개행이 있으면 안 된다: {t}");
        assert!(t.contains("2건") && t.contains("surface:11") && t.contains("surface:22"));
        assert!(t.contains("oldest=120s"), "진단 수치가 빠지면 수신측이 판단할 재료가 없다");
        assert!(depth_digest_text(&[]).contains("0건"), "빈 목록도 패닉 없이 다뤄야 한다");
    }

    /// ★E1: 수집 레인은 **이벤트 쿨다운과 독립**이다. 종전 구조에서는 hard 판정 자체가 소스별
    /// 5분 쿨다운 뒤에 있어서, 이벤트가 억제된 틱에는 OOB 도 통째로 사라졌다 — 소스별 위상차가
    /// 그대로 통지 누락으로 번역되는 경로(BLOCK-1의 절반)다.
    #[test]
    fn hard_backlog_collection_is_not_gated_by_event_cooldown() {
        let (d, dir) = daemon("collect");
        let s = surface(&d, Some("worker-1"));
        {
            // 기본 임계 5 → hard 는 depth ≥ 15(env 무의존).
            let mut q = s.pending_queue.lock().unwrap();
            for i in 0..15 {
                q.push_back(crate::state::QueueItem::text(
                    format!("m{i}"),
                    crate::state::QueueOrigin::Agent { surface: 7, role: None },
                ));
            }
        }
        let mut alerted: HashMap<u64, f64> = HashMap::new();
        let mut h1 = Vec::new();
        alert_queue_depth_if_high(&d, &s, &mut alerted, "busy(출력 중)", &mut h1);
        assert_eq!(h1.len(), 1, "hard tier 적체가 수집돼야 한다");
        assert_eq!(h1[0].sid, s.id);
        assert_eq!(h1[0].depth, 15);

        // 같은 틱 직후(이벤트는 5분 쿨다운으로 억제되는 구간) — 수집은 계속돼야 한다.
        let mut h2 = Vec::new();
        alert_queue_depth_if_high(&d, &s, &mut alerted, "busy(출력 중)", &mut h2);
        assert_eq!(
            h2.len(),
            1,
            "이벤트 쿨다운이 다이제스트 수집까지 막으면 위상 고정 침묵이 재발한다"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★A4 보존(전역캡 제거 후): `request_clear` 는 사람이 명시 발행하는 요청 레인이라
    /// 자동 통지와 **예산을 공유하지 않는다**. 종전에는 전역캡이 이 결합을 만들어 면제 상수로
    /// 도려내야 했는데, 캡 자체가 사라져 (surface,key) 쿨다운만으로 독립이 성립한다.
    /// 새 전역 상한을 다시 들이면 이 테스트가 먼저 깨진다.
    #[test]
    fn request_clear_lane_keeps_independent_budget() {
        let (d, dir) = daemon("req-clear-lane");
        let s = surface(&d, Some("master"));
        seat_agent(&s);

        // ① 자동 통지(다이제스트) 직후에도 사람의 요청은 즉시 배달된다.
        notify_depth_digest(&d, &[hard(11, "worker-1")]);
        assert_eq!(
            oob_notify(&d, s.id, "clear 요청", "request_clear", OOB_REQUEST_CLEAR_COOLDOWN_SECS),
            Ok(()),
            "자동 통지가 사람 요청을 삼키면 전용 60초 쿨다운(A4)의 의미가 죽는다"
        );
        // ② 반대 방향: 사람 요청이 자동 통지 예산을 소모하지 않는다.
        let (d2, dir2) = daemon("req-clear-lane-rev");
        let s2 = surface(&d2, Some("cso"));
        seat_agent(&s2);
        assert_eq!(
            oob_notify(&d2, s2.id, "clear 요청", "request_clear", OOB_REQUEST_CLEAR_COOLDOWN_SECS),
            Ok(())
        );
        notify_depth_digest(&d2, &[hard(7, "worker-1")]);
        assert_eq!(
            digest_bytes(&d2, s2.id).len(),
            1,
            "사람 요청 직후에도 자동 경보는 나가야 한다"
        );
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&dir2);
    }

    /// ★B4: 쿨다운이 **데몬 재기동을 생존**한다. 큐는 WAL 로 즉시 복원돼 depth 가 그대로
    /// 살아나는데 쿨다운만 빈 맵이라, 재기동마다 미해소 조건 전체가 재통지 버스트로 터졌다(R4).
    /// 앵커는 큐 WAL 과 같은 **role** 이다 — surface_id 는 재기동 시 소멸한다(재사용 없음).
    #[test]
    fn cooldowns_survive_daemon_restart_via_sidecar() {
        let dir = std::env::temp_dir().join(format!(
            "cys-oob-persist-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("cysd.sock");
        let sidecar = crate::state::state_dir(&sock).join(OOB_COOLDOWN_FILE);

        // ── 1세대 데몬: 통지 1회로 스탬프를 찍는다 ──
        let d1 = Daemon::new(sock.clone());
        let s1 = surface(&d1, Some("master"));
        seat_agent(&s1);
        assert_eq!(
            oob_notify(
                &d1,
                s1.id,
                "적체",
                &format!("depth_high_hard:{}", s1.id),
                OOB_HARD_COOLDOWN_SECS
            ),
            Ok(())
        );
        assert!(sidecar.exists(), "스탬프 갱신 시 사이드카가 기록돼야 한다");

        // ── 2세대 데몬(재기동 모사): 라이브 맵은 비었지만 원장이 억제한다 ──
        let d2 = Daemon::new(sock.clone());
        assert!(
            d2.oob_cooldowns.lock().unwrap().is_empty(),
            "전제: surface 키 라이브 맵은 재기동을 넘지 못한다"
        );
        let mut restored_keys: Vec<(String, String)> =
            d2.restored_oob_cooldowns.lock().unwrap().keys().cloned().collect();
        restored_keys.sort();
        assert_eq!(
            restored_keys,
            // ★E1: 전역캡(`__global__`) 예산 항목은 사라졌다 — 총량 상한이 다이제스트 구조로
            // 옮겨갔으므로 영속 대상은 실제 dedup 키 계열뿐이다.
            vec![("master".to_string(), "depth_high_hard".to_string())],
            "role 앵커 + dedup 키 계열로 복원돼야 한다"
        );
        let _filler = surface(&d2, None); // surface id 를 어긋나게 해 sid 무관성을 드러낸다
        let s2 = surface(&d2, Some("master"));
        seat_agent(&s2);
        assert_ne!(s1.id, s2.id, "전제: 재기동 후 같은 role 이 다른 surface id 를 갖는다");
        assert_eq!(
            oob_notify(
                &d2,
                s2.id,
                "적체",
                &format!("depth_high_hard:{}", s2.id),
                OOB_HARD_COOLDOWN_SECS
            ),
            Err(OobSkip::Cooldown),
            "재기동 직후 미해소 조건이 그대로 재통지되면 R4 버스트가 재발한다"
        );

        // ── 손상 폴백: 못 읽으면 빈 맵(fail-open) — 통지를 **막는** 원장이라 억제하지 않는다 ──
        std::fs::write(&sidecar, "{ 손상된 json").unwrap();
        let d3 = Daemon::new(sock);
        assert!(
            d3.restored_oob_cooldowns.lock().unwrap().is_empty(),
            "손상본은 빈 맵 폴백이어야 한다(버스트 1회 감수 · 통지 봉쇄 금지)"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★R3: 통지 대상 집합에 **dept-master**가 포함돼야 한다. 부서 데몬의 roles 맵에는
    /// `master` 키가 없어서 {master,cso}만 조회하면 통지가 공집합이 된다(부서 조직 무통지).
    #[test]
    fn privileged_targets_include_dept_master_and_dedupe() {
        let (d, dir) = daemon("targets");
        let owner = surface(&d, Some("worker-1"));
        let cso = surface(&d, Some("cso"));
        let deptm = surface(&d, Some("dept-master"));
        let _noise = surface(&d, Some("reviewer-codex"));

        let t = oob_targets(&d, owner.id);
        assert!(t.contains(&owner.id), "큐 소유 노드는 항상 대상이다");
        assert!(t.contains(&cso.id));
        assert!(t.contains(&deptm.id), "부서 데몬에서 통지가 공집합이 되면 안 된다");
        assert!(!t.contains(&_noise.id), "지휘 role이 아닌 노드는 대상이 아니다");

        // 소유 노드가 지휘 role 자신이면 중복 없이 1건.
        let t2 = oob_targets(&d, cso.id);
        assert_eq!(t2.iter().filter(|x| **x == cso.id).count(), 1, "중복 통지 금지");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
