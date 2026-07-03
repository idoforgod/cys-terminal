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
        let mut learn_stuck_debounce: HashMap<u64, f64> = HashMap::new();
        let mut zombie_miss: HashMap<u64, u32> = HashMap::new();
        let mut tick_no: u64 = 0;
        loop {
            tokio::time::sleep(Duration::from_secs(WATCHDOG_INTERVAL_SECS)).await;
            tick_no += 1;
            // 패닉 격리: 한 틱의 unwrap 패닉이 watchdog 태스크 전체를 죽여
            // 자원 거버넌스가 데몬 수명 내내 조용히 사라지는 것을 막는다.
            let tick = std::panic::AssertUnwindSafe(|| {
                sys.refresh_processes(ProcessesToUpdate::All, true);
                check_load(&daemon, &mut last_load_alert);
                check_surfaces(&daemon, &sys, &mut last_dup_alert, &mut last_proc_alert);
                check_idle(&daemon);
                deliver_queued(&daemon, &mut queue_depth_alerted);
                reap_orphan_ledger(&daemon, &sys);
                reap_exited_surfaces(&daemon);
                reap_zombie_surfaces(&daemon, &sys, &mut zombie_miss);
                check_agent_death(&daemon, &sys, &mut restart_counts);
                check_surface_crash(&daemon);
                check_feed_aging(&daemon, &mut feed_reminded);
                check_master_deadman(&daemon, &mut deadman_last_alert);
                // 저빈도 검사(15초): 파일 stat·화면 렌더 — 5초마다 돌릴 필요 없음
                if tick_no.is_multiple_of(3) {
                    check_todo(&daemon);
                    check_approvals(&daemon, &mut approval_debounce);
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
        if close_surface(daemon, id).is_ok() {
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
}

/// T3-12 승인 aging 재알림: pending feed가 무음 적체되지 않게 N분마다 재push.
fn check_feed_aging(daemon: &Arc<Daemon>, reminded: &mut HashMap<String, f64>) {
    let remind_secs = env_u64("CYS_FEED_REMIND_SECS", 300);
    if remind_secs == 0 {
        return;
    }
    let now = now_epoch();
    let pending: Vec<(String, String, f64)> = {
        let items = daemon.feed_items.lock().unwrap();
        items
            .iter()
            .filter(|i| i.status == "pending")
            .map(|i| (i.request_id.clone(), i.title.clone(), i.created_at))
            .collect()
    };
    let pending_ids: std::collections::HashSet<&String> =
        pending.iter().map(|(id, _, _)| id).collect();
    reminded.retain(|id, _| pending_ids.contains(id));
    let total = pending.len();
    for (request_id, title, created_at) in &pending {
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

/// CYS_TODO_DIRS(PATH류 목록)를 플랫폼 규약대로 분해한다.
/// `std::env::split_paths`는 Unix에서 ':' · Windows에서 ';'로 가르며,
/// Windows 드라이브 문자 콜론(`C:\…`)을 구분자로 오인하지 않는다 — ':' 하드코딩이
/// Windows 절대경로를 `C`와 `\…`로 쪼개 워치를 무력화하던 버그를 차단한다.
/// 빈 항목은 기존 동작과 동일하게 버린다.
fn parse_todo_dirs(raw: &str) -> Vec<std::path::PathBuf> {
    std::env::split_paths(raw)
        .filter(|p| !p.as_os_str().is_empty())
        .collect()
}

/// T3-9 todo 파일 워치: 각 surface cwd의 `_round/*_TODO.md` + CYS_TODO_DIRS 추가 루트.
/// 변경 감지 시 todo.updated 이벤트 + org.status 집계 갱신 (push 규약을 기계 보증으로).
fn check_todo(daemon: &Arc<Daemon>) {
    let mut dirs: std::collections::HashSet<std::path::PathBuf> =
        std::collections::HashSet::new();
    for s in daemon.surfaces.lock().unwrap().values() {
        if !s.exited.load(Ordering::Relaxed) {
            dirs.insert(std::path::PathBuf::from(&s.cwd).join("_round"));
        }
    }
    if let Ok(extra) = std::env::var("CYS_TODO_DIRS") {
        dirs.extend(parse_todo_dirs(&extra));
    }
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.ends_with("_TODO.md") {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs_f64())
                .unwrap_or(0.0);
            let key = path.to_string_lossy().into_owned();
            seen.insert(key.clone());
            let prev_mtime = daemon
                .todo_progress
                .lock()
                .unwrap()
                .get(&key)
                .map(|(_, _, m)| *m);
            if prev_mtime == Some(mtime) {
                continue;
            }
            // 변경됨 — 체크박스 집계 (64KB 상한: 거대 파일이 watchdog 틱을 잡아먹지 않게)
            let Ok(content) = std::fs::read_to_string(&path) else {
                continue;
            };
            let content: String = content.chars().take(65536).collect();
            let done = content.matches("- [x]").count() as u64
                + content.matches("- [X]").count() as u64;
            let total = done + content.matches("- [ ]").count() as u64;
            daemon
                .todo_progress
                .lock()
                .unwrap()
                .insert(key.clone(), (done, total, mtime));
            if prev_mtime.is_some() {
                // 최초 발견은 무음 등록 — 데몬 재시작마다 전 파일 이벤트 폭주 방지
                daemon.bus.publish(
                    "todo.updated",
                    "todo",
                    None,
                    json!({"path": key, "done": done, "total": total}),
                );
            }
        }
    }
    // 사라진 파일 정리
    daemon
        .todo_progress
        .lock()
        .unwrap()
        .retain(|k, _| seen.contains(k));
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
        let Some(patterns) = agents[&agent]["approval_patterns"].as_array() else {
            continue;
        };
        if patterns.is_empty() {
            continue;
        }
        let screen = s.parser.lock().unwrap_or_else(|e| e.into_inner()).screen().contents();
        for p in patterns {
            let (Some(name), Some(pattern)) = (p["name"].as_str(), p["pattern"].as_str()) else {
                continue;
            };
            let Ok(re) = regex::Regex::new(pattern) else {
                continue;
            };
            let Some(m) = re.find(&screen) else { continue };
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
        }
    }
}

/// T2-6 토폴로지 영속: role→agent→cwd 매핑을 디스크에 상시 기록 (cys restore의 진실).
pub fn persist_topology(daemon: &Arc<Daemon>) {
    let entries: Vec<serde_json::Value> = daemon
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
                       "session_id": s.agent_session_id.lock().unwrap().clone(),
                       "pack_reinject": s.pack_reinject.lock().unwrap().clone()})
            })
        })
        .collect();
    let dir = crate::state::state_dir(&daemon.socket_path);
    let content = serde_json::to_string_pretty(&json!({"updated_at": now_epoch(), "entries": entries}))
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
        if close_surface(daemon, id).is_ok() {
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

/// Close a surface: kill the entire descendant process tree, then the shell itself.
/// 고아 서버 누적(load 폭주의 원인)을 원천 차단하는 지점.
pub fn close_surface(daemon: &Arc<Daemon>, id: u64) -> Result<(), String> {
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
        if let Some(role) = srole.as_ref() {
            if roles.get(role) == Some(&id) {
                roles.remove(role);
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
        surface
    };
    // health 디바운스·조치 게이트 맵에서 이 surface의 (surface_id, rule) 키 회수 —
    // surface가 맵에서 사라지는 유일 지점에서 함께 비워 누수를 차단한다(별도 락).
    prune_surface_health_keys(
        &mut daemon.health_debounce.lock().unwrap(),
        &mut daemon.health_hits.lock().unwrap(),
        id,
    );
    // 미배달 큐 폐기 통지 — queued:true 응답을 받은 발신자의 무음 메시지 유실 차단
    let dropped: Vec<String> = surface.pending_queue.lock().unwrap().drain(..).collect();
    if !dropped.is_empty() {
        daemon.bus.publish(
            "queue.dropped",
            "queue",
            Some(id),
            json!({"reason": "surface_closed", "count": dropped.len(),
                   "bytes": dropped.iter().map(|t| t.len()).sum::<usize>()}),
        );
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
/// 머리는 항상 방금 보낸 text다. 그래도 머리 일치를 확인하고 제거하는 belt-and-suspenders
/// 가드 — 무조건 pop_front이 미배달 새 머리를 삼키는 일을 구조적으로 차단한다.
fn pop_delivered_head(q: &mut std::collections::VecDeque<String>, delivered: &str) {
    if q.front().map(String::as_str) == Some(delivered) {
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

/// 배달이 막힌 surface의 적체 경보(쿨다운 5분) — quiet 미충족·human 흔적·pause 등
/// 모든 '막힘' 분기에서 공통 호출한다(한 분기라도 빠지면 그 사유의 적체가 침묵한다).
fn alert_queue_depth_if_high(
    daemon: &Arc<Daemon>,
    s: &Arc<crate::state::Surface>,
    depth_alerted: &mut HashMap<u64, f64>,
    blocked_by: &str,
) {
    let threshold = queue_depth_alert_threshold();
    if threshold == 0 {
        return;
    }
    let depth = s.pending_queue.lock().unwrap().len();
    if depth < threshold {
        return;
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
    } else {
        "임계 조정은 CYS_QUEUE_QUIET_SECS"
    };
    daemon.bus.publish(
        "queue.depth_high",
        "queue",
        Some(s.id),
        json!({"depth": depth, "threshold": threshold, "blocked_by": blocked_by,
               "role": s.role.lock().unwrap().clone(),
               "surface_ref": cys::surface_ref(s.id),
               "hint": format!("queued 배달이 막힌 채 적체 중 — read-screen으로 상태 점검, \
                                급한 보고는 직접 send(steer). {knob}")}),
    );
}

/// 인플라이트 큐 배달자: 대상 surface가 quiet 임계(기본 3초) 이상 조용하면 큐에서 한 건 주입.
/// 연속 배달은 다음 틱 — 메시지 사이 자연 간격이 생겨 에이전트가 한 건씩 소화한다.
/// 배달이 막힌 채 적체되면(depth ≥ 임계) `queue.depth_high`를 쿨다운(5분)으로 발행한다.
fn deliver_queued(daemon: &Arc<Daemon>, depth_alerted: &mut HashMap<u64, f64>) {
    // T4-15 kill-switch: pause 중에는 큐 배달 동결 (메시지는 보존 — resume 시 재개)
    if daemon.paused.load(Ordering::Relaxed) {
        return;
    }
    // ★Phase 5 ①c: WAL로 살아난 restored_queue를 같은 role의 살아있는 surface로 재홈한 뒤 배달.
    // (Phase 3에서 restored_queue가 배달 경로에 미배선이라, 재기동 생존 메시지가 idle에도 미배달로
    // 잔존하던 갭을 닫는다 — role 앵커 재타겟.)
    if daemon.rehome_restored_queue() > 0 {
        daemon.persist_queue_state();
    }
    let surfaces: Vec<Arc<crate::state::Surface>> =
        daemon.surfaces.lock().unwrap().values().cloned().collect();
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
            alert_queue_depth_if_high(daemon, &s, depth_alerted, "queue_paused(헬스 조치)");
            continue;
        }
        // 아직 바쁨(출력 중) — steer는 즉시 전송이 담당, 큐는 기다린다.
        let quiet_for = s.last_output.lock().unwrap().elapsed().as_secs();
        if quiet_for < queue_quiet_secs() {
            alert_queue_depth_if_high(daemon, &s, depth_alerted, "busy(출력 중)");
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
            alert_queue_depth_if_high(daemon, &s, depth_alerted, "human_typing(사람 입력 직후)");
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
            let Some(text) = q.front().cloned() else {
                continue;
            };
            let req = crate::state::WriteReq::Inject {
                text: text.clone(),
                cr_delay_ms: 400,
                clear_first: false, // queued 배달은 quiet 대기 후라 선정리 불필요(현행 동작 보존)
            };
            if s.write_tx.try_send(req).is_err() {
                continue; // 인계 실패 — 메시지 보존, 다음 틱 재시도
            }
            pop_delivered_head(&mut q, &text);
            Some((text, q.len()))
        };
        if let Some((text, remaining)) = delivered {
            // T4-17 에코 제외 창 — 큐 배달도 원격 주입이다
            *s.last_injected.lock().unwrap() = Some(std::time::Instant::now());
            daemon.bus.publish(
                "queue.delivered",
                "queue",
                Some(s.id),
                serde_json::json!({"bytes": text.len(), "remaining": remaining}),
            );
            // P7 큐 WAL: 배달로 줄어든 큐를 디스크에 반영(스냅샷 최신화).
            daemon.persist_queue_state();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{learn_stuck_candidates, plan_duplicate_kills};

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
        collect_scoped_for_shutdown, parse_todo_dirs, pop_delivered_head,
        prune_surface_health_keys, prune_watchdog_debounce_maps, LOAD_DEBOUNCE_SECS,
    };
    use crate::state::LedgerEntry;
    use std::collections::{HashMap, HashSet, VecDeque};
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

    fn q(items: &[&str]) -> VecDeque<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    // ── CYS_TODO_DIRS 파싱 회귀 가드 ──
    // 빈 항목은 버린다(기존 동작 보존). split_paths가 Unix에서 ':'로 가른다.
    #[test]
    fn parse_todo_dirs_drops_empty_entries() {
        // 구버전 split(':').filter(!is_empty)와 동치임을 확인.
        let dirs = parse_todo_dirs("/a/b::/c/d");
        assert_eq!(dirs, vec![PathBuf::from("/a/b"), PathBuf::from("/c/d")]);
        assert!(parse_todo_dirs("").is_empty());
    }

    // Windows 드라이브 문자 콜론(`C:\…`)을 구분자로 오인하지 않아야 한다.
    // 구버전 `extra.split(':')`는 `C:\Users\x\_round`를 `C` + `\Users\x\_round`로
    // 쪼개 둘 다 존재하지 않는 경로로 만들어 워치를 무력화했다 — 이 테스트는
    // Windows 타깃에서만 의미가 있으므로 cfg(windows)로 가둔다.
    #[cfg(windows)]
    #[test]
    fn parse_todo_dirs_keeps_windows_drive_paths_intact() {
        let dirs = parse_todo_dirs(r"C:\Users\x\_round;D:\proj\_round");
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
        // 정상 경로: 보낸 메시지가 여전히 머리 → 제거. 뒤 메시지는 보존.
        let mut deque = q(&["msg1", "msg2"]);
        pop_delivered_head(&mut deque, "msg1");
        assert_eq!(deque, q(&["msg2"]));
    }

    #[test]
    fn pop_delivered_head_noop_on_empty_after_clear() {
        // lost-clear 시나리오: front 읽은 뒤 락이 풀린 창에서 queue.clear가 drain →
        // 빈 큐. 핵심은 '빈 큐를 건드리지 않고' 손상 없이 빠져나오는 것.
        // (이미 PTY로 간 메시지는 회수 불가 — 아키텍처 한계)
        let mut deque = q(&[]);
        pop_delivered_head(&mut deque, "msg1");
        assert!(deque.is_empty());
    }

    #[test]
    fn pop_delivered_head_preserves_new_message_after_clear_and_enqueue() {
        // 유해 변종(이 수정의 핵심 회귀 가드): front("msgA") 읽고 락 해제 →
        // 그 창에서 clear가 drain([]) 후 새 메시지 "msgB" enqueue → 큐=["msgB"].
        // 무조건 pop_front이면 미배달 "msgB"를 삼켜 조용히 유실시킨다.
        // 머리가 보낸 "msgA"가 아니므로 제거하지 않아야 한다 — "msgB"는 다음 틱에 배달.
        let mut deque = q(&["msgB"]);
        pop_delivered_head(&mut deque, "msgA");
        assert_eq!(deque, q(&["msgB"]), "미배달 새 메시지가 유실되면 안 된다");
    }

    #[test]
    fn pop_delivered_head_preserves_replacement_head() {
        // clear→enqueue가 여러 건이어도 머리 불일치면 한 건도 삼키지 않는다.
        let mut deque = q(&["msgB", "msgC"]);
        pop_delivered_head(&mut deque, "msgA");
        assert_eq!(deque, q(&["msgB", "msgC"]));
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
        queue: &Mutex<VecDeque<String>>,
        writer: &std::sync::mpsc::SyncSender<String>,
    ) -> Option<String> {
        let mut q = queue.lock().unwrap();
        let text = q.front().cloned()?;
        // 논블로킹 인계. 실패 시 메시지 보존(pop 안 함).
        if writer.try_send(text.clone()).is_err() {
            return None;
        }
        pop_delivered_head(&mut q, &text);
        Some(text)
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
                let _: Vec<String> = qc.lock().unwrap().drain(..).collect();
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
}
